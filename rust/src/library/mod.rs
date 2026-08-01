//! Persistent local media library and user-managed playlists.

mod cover_store;
mod error;
mod model;
mod scan;
mod store;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use cover_store::CoverStore;
use directories::ProjectDirs;
pub use error::LibraryError;
pub(crate) use model::StoredPlaybackMode;
pub use model::{PlaybackSession, Playlist, PlaylistId, PlaylistKind, Song};
use model::{SongId, SongRow, SongTags};
use scan::LibraryScanner;
use store::{CatalogStore, JsonMigrator};

use crate::player::metadata::{CoverArt, TrackMetadata};
use crate::player::{MediaInspector, SymphoniaMediaInspector};

const CATALOG_FILE_NAME: &str = "catalog.db";
const COVERS_DIR_NAME: &str = "covers";

/// Persistent library service backed by Turso catalog + cover cache.
///
/// Lyrics are not stored in the library DB; playback loads them from the media
/// file / sidecar at decode time. Any leftover `lyrics.db` on disk is unused.
pub(crate) struct Library {
    catalog: CatalogStore,
    covers: CoverStore,
    inspector: Box<dyn MediaInspector>,
}

impl Library {
    /// Open the library using platform data + cache directories.
    pub(crate) async fn open_default() -> Result<Self, LibraryError> {
        let dirs = ProjectDirs::from("dev", "mofu", "mofu")
            .ok_or(LibraryError::DataDirectoryUnavailable)?;
        Self::open_at_dirs(
            dirs.data_dir().to_path_buf(),
            dirs.cache_dir().join(COVERS_DIR_NAME),
        )
        .await
    }

    /// Open under an explicit data directory; covers use the platform cache.
    pub(crate) async fn open_at(data_dir: PathBuf) -> Result<Self, LibraryError> {
        let cache_dir = ProjectDirs::from("dev", "mofu", "mofu")
            .map(|dirs| dirs.cache_dir().join(COVERS_DIR_NAME))
            .ok_or(LibraryError::CacheDirectoryUnavailable)?;
        Self::open_at_dirs(data_dir, cache_dir).await
    }

    /// Open with explicit data and cover-cache directories (tests).
    pub(crate) async fn open_at_dirs(
        data_dir: PathBuf,
        cache_dir: PathBuf,
    ) -> Result<Self, LibraryError> {
        Self::open_with_inspector(data_dir, cache_dir, Box::new(SymphoniaMediaInspector)).await
    }

    async fn open_with_inspector(
        data_dir: PathBuf,
        cache_dir: PathBuf,
        inspector: Box<dyn MediaInspector>,
    ) -> Result<Self, LibraryError> {
        fs::create_dir_all(&data_dir)?;
        let catalog = CatalogStore::open(&data_dir.join(CATALOG_FILE_NAME)).await?;
        let covers = CoverStore::open(cache_dir)?;
        JsonMigrator::run(&data_dir, &catalog, &covers).await?;
        Ok(Self {
            catalog,
            covers,
            inspector,
        })
    }

    pub(crate) async fn playlists(&self) -> Result<Vec<Playlist>, LibraryError> {
        let rows = self.catalog.list_playlist_rows().await?;
        let mut playlists = Vec::with_capacity(rows.len());
        for row in rows {
            let songs = self.load_songs_for_playlist(row.id).await?;
            playlists.push(Playlist {
                id: row.id,
                name: row.name,
                kind: row.kind,
                songs,
            });
        }
        Ok(playlists)
    }

    pub(crate) async fn create_virtual(&self, name: String) -> Result<Playlist, LibraryError> {
        let name = Self::validated_name(name)?;
        let id = self.catalog.next_playlist_id().await?;
        self.catalog
            .insert_playlist(id, &name, &PlaylistKind::Virtual)
            .await?;
        Ok(Playlist {
            id,
            name,
            kind: PlaylistKind::Virtual,
            songs: Vec::new(),
        })
    }

    pub(crate) async fn add_directory(&self, path: PathBuf) -> Result<Playlist, LibraryError> {
        let canonical = fs::canonicalize(path)?;
        let path_string = canonical.to_string_lossy().into_owned();
        let songs = LibraryScanner::new(self.inspector.as_ref()).scan_directory(&canonical);
        let name = canonical
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or(&path_string)
            .to_owned();

        let playlist_id = if let Some(existing) = self
            .catalog
            .find_directory_playlist_id(&path_string)
            .await?
        {
            existing
        } else {
            let id = self.catalog.next_playlist_id().await?;
            self.catalog
                .insert_playlist(
                    id,
                    &name,
                    &PlaylistKind::Directory {
                        path: path_string.clone(),
                    },
                )
                .await?;
            id
        };

        let song_ids = self.persist_songs(&songs).await?;
        self.catalog
            .replace_playlist_songs(playlist_id, &song_ids)
            .await?;
        self.garbage_collect().await?;
        self.playlist_by_id(playlist_id).await
    }

    pub(crate) async fn rename_virtual(
        &self,
        id: PlaylistId,
        name: String,
    ) -> Result<(), LibraryError> {
        let name = Self::validated_name(name)?;
        let row = self.catalog.playlist_row(id).await?;
        if row.kind != PlaylistKind::Virtual {
            return Err(LibraryError::NotVirtualPlaylist);
        }
        self.catalog.rename_playlist(id, &name).await
    }

    pub(crate) async fn delete_virtual(&self, id: PlaylistId) -> Result<(), LibraryError> {
        let row = self.catalog.playlist_row(id).await?;
        if row.kind != PlaylistKind::Virtual {
            return Err(LibraryError::NotVirtualPlaylist);
        }
        self.catalog.delete_playlist(id).await?;
        self.garbage_collect().await
    }

    pub(crate) async fn add_files(
        &self,
        id: PlaylistId,
        paths: Vec<PathBuf>,
    ) -> Result<usize, LibraryError> {
        let row = self.catalog.playlist_row(id).await?;
        if row.kind != PlaylistKind::Virtual {
            return Err(LibraryError::NotVirtualPlaylist);
        }
        let inspected = LibraryScanner::new(self.inspector.as_ref()).inspect_paths(paths);
        let mut existing = self
            .catalog
            .songs_for_playlist(id)
            .await?
            .into_iter()
            .map(|song| song.path)
            .collect::<HashSet<_>>();
        let mut added = 0usize;
        let mut position = self.catalog.max_position(id).await? + 1;
        for song in inspected {
            if !existing.insert(song.path.clone()) {
                continue;
            }
            let song_id = self.persist_song(&song).await?;
            self.catalog.attach_song(id, song_id, position).await?;
            position += 1;
            added += 1;
        }
        Ok(added)
    }

    pub(crate) async fn import_from_playlist(
        &self,
        target_id: PlaylistId,
        source_id: PlaylistId,
        paths: Vec<String>,
    ) -> Result<usize, LibraryError> {
        let target = self.catalog.playlist_row(target_id).await?;
        if target.kind != PlaylistKind::Virtual {
            return Err(LibraryError::NotVirtualPlaylist);
        }
        let _source = self.catalog.playlist_row(source_id).await?;
        let selected = paths.into_iter().collect::<HashSet<_>>();
        let source_songs = self.catalog.songs_for_playlist(source_id).await?;
        let mut existing = self
            .catalog
            .songs_for_playlist(target_id)
            .await?
            .into_iter()
            .map(|song| song.path)
            .collect::<HashSet<_>>();
        let mut added = 0usize;
        let mut position = self.catalog.max_position(target_id).await? + 1;
        for song in source_songs {
            if !selected.is_empty() && !selected.contains(&song.path) {
                continue;
            }
            if !existing.insert(song.path.clone()) {
                continue;
            }
            self.catalog
                .attach_song(target_id, song.id, position)
                .await?;
            position += 1;
            added += 1;
        }
        Ok(added)
    }

    pub(crate) async fn refresh(&self, id: PlaylistId) -> Result<Playlist, LibraryError> {
        let row = self.catalog.playlist_row(id).await?;
        let songs = match &row.kind {
            PlaylistKind::Directory { path } => {
                LibraryScanner::new(self.inspector.as_ref()).scan_directory(Path::new(path))
            }
            PlaylistKind::Virtual => {
                let paths = self
                    .catalog
                    .playlist_song_paths(id)
                    .await?
                    .into_iter()
                    .map(PathBuf::from)
                    .collect::<Vec<_>>();
                LibraryScanner::new(self.inspector.as_ref()).inspect_paths(paths)
            }
        };
        let song_ids = self.persist_songs(&songs).await?;
        self.catalog.replace_playlist_songs(id, &song_ids).await?;
        self.garbage_collect().await?;
        self.playlist_by_id(id).await
    }

    async fn playlist_by_id(&self, id: PlaylistId) -> Result<Playlist, LibraryError> {
        let row = self.catalog.playlist_row(id).await?;
        let songs = self.load_songs_for_playlist(id).await?;
        Ok(Playlist {
            id: row.id,
            name: row.name,
            kind: row.kind,
            songs,
        })
    }

    /// Load cover bytes for known paths; skips missing songs / missing cover files.
    pub(crate) async fn covers_for_paths(
        &self,
        paths: Vec<String>,
    ) -> Result<Vec<(String, CoverArt)>, LibraryError> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let rows = self.catalog.song_rows_for_paths(&paths).await?;
        let mut covers = Vec::new();
        for row in rows {
            let Some(ext) = row.tags.cover_ext else {
                continue;
            };
            let Some(cover) = self.covers.read(row.id, ext, row.tags.cover_mime) else {
                continue;
            };
            covers.push((row.path, cover));
        }
        Ok(covers)
    }

    async fn load_songs_for_playlist(
        &self,
        playlist_id: PlaylistId,
    ) -> Result<Vec<Song>, LibraryError> {
        let rows = self.catalog.songs_for_playlist(playlist_id).await?;
        let mut songs = Vec::with_capacity(rows.len());
        for row in rows {
            // 首包只带文字标签；封面经 covers_for_paths 异步补全，歌词由播放路径提供。
            songs.push(Self::assemble_song_lite(row));
        }
        Ok(songs)
    }

    fn assemble_song_lite(row: SongRow) -> Song {
        Song {
            path: row.path,
            duration_ms: row.duration_ms,
            metadata: TrackMetadata {
                title: row.tags.title,
                artist: row.tags.artist,
                album: row.tags.album,
                album_artist: row.tags.album_artist,
                composer: row.tags.composer,
                lyricist: row.tags.lyricist,
                performer: row.tags.performer,
                genre: row.tags.genre,
                comment: row.tags.comment,
                date: row.tags.date,
                year: row.tags.year,
                track_number: row.tags.track_number,
                track_total: row.tags.track_total,
                disc_number: row.tags.disc_number,
                disc_total: row.tags.disc_total,
                cover: None,
                lyrics: Vec::new(),
            },
        }
    }

    async fn persist_songs(&self, songs: &[Song]) -> Result<Vec<SongId>, LibraryError> {
        let mut ids = Vec::with_capacity(songs.len());
        for song in songs {
            ids.push(self.persist_song(song).await?);
        }
        Ok(ids)
    }

    async fn persist_song(&self, song: &Song) -> Result<SongId, LibraryError> {
        let tags = SongTags::from_metadata(&song.metadata, None);
        let song_id = self
            .catalog
            .upsert_song(&song.path, song.duration_ms, &tags)
            .await?;
        let cover_ext = if let Some(cover) = song.metadata.cover.as_ref() {
            Some(self.covers.write(song_id, cover)?)
        } else {
            self.covers.remove_all(song_id);
            None
        };
        let tags = SongTags::from_metadata(&song.metadata, cover_ext);
        self.catalog
            .upsert_song(&song.path, song.duration_ms, &tags)
            .await
    }

    async fn garbage_collect(&self) -> Result<(), LibraryError> {
        let orphans = self.catalog.garbage_collect_orphan_songs().await?;
        for (song_id, cover_ext) in orphans {
            if let Some(ext) = cover_ext {
                self.covers.remove(song_id, ext);
            } else {
                self.covers.remove_all(song_id);
            }
        }
        Ok(())
    }

    pub(crate) async fn playback_mode(&self) -> Result<StoredPlaybackMode, LibraryError> {
        self.catalog.playback_mode().await
    }

    pub(crate) async fn set_playback_mode(
        &self,
        mode: StoredPlaybackMode,
    ) -> Result<(), LibraryError> {
        self.catalog.set_playback_mode(mode).await
    }

    pub(crate) async fn volume(&self) -> Result<f64, LibraryError> {
        self.catalog.volume().await
    }

    pub(crate) async fn set_volume(&self, volume: f64) -> Result<(), LibraryError> {
        self.catalog.set_volume(volume).await
    }

    pub(crate) async fn playback_session(&self) -> Result<Option<PlaybackSession>, LibraryError> {
        self.catalog.playback_session().await
    }

    pub(crate) async fn set_playback_session(
        &self,
        session: PlaybackSession,
    ) -> Result<(), LibraryError> {
        self.catalog.set_playback_session(&session).await
    }

    pub(crate) async fn clear_playback_session(&self) -> Result<(), LibraryError> {
        self.catalog.clear_playback_session().await
    }

    fn validated_name(name: String) -> Result<String, LibraryError> {
        let name = name.trim().to_owned();
        if name.is_empty() {
            Err(LibraryError::EmptyPlaylistName)
        } else {
            Ok(name)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::model::LibraryManifest;
    use super::store::CatalogStore;
    use super::*;
    use crate::player::metadata::CoverArt;
    use crate::player::{MediaInspection, PlayerError};

    struct FakeInspector;

    impl MediaInspector for FakeInspector {
        fn inspect(&self, path: &Path) -> Result<MediaInspection, PlayerError> {
            Ok(MediaInspection {
                duration_ms: 42,
                metadata: TrackMetadata {
                    title: path
                        .file_stem()
                        .map(|name| name.to_string_lossy().into_owned()),
                    ..Default::default()
                },
            })
        }
    }

    struct TestDirs {
        data: PathBuf,
        cache: PathBuf,
    }

    impl TestDirs {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos();
            let base =
                std::env::temp_dir().join(format!("mofu-library-{}-{nonce}", std::process::id()));
            let data = base.join("data");
            let cache = base.join("cache");
            fs::create_dir_all(&data).expect("create data dir");
            fs::create_dir_all(&cache).expect("create cache dir");
            Self { data, cache }
        }
    }

    impl Drop for TestDirs {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(self.data.parent().unwrap_or(&self.data));
        }
    }

    async fn open_test_library(dirs: &TestDirs) -> Library {
        Library::open_with_inspector(
            dirs.data.clone(),
            dirs.cache.clone(),
            Box::new(FakeInspector),
        )
        .await
        .expect("open library")
    }

    #[test]
    fn virtual_playlist_persists_and_deduplicates_files() {
        futures_executor::block_on(async {
            let dirs = TestDirs::new();
            let media = dirs.data.join("song.mp3");
            fs::write(&media, b"fake").expect("write media");
            {
                let library = open_test_library(&dirs).await;
                let playlist = library
                    .create_virtual("Favorites".into())
                    .await
                    .expect("create");
                assert_eq!(
                    library
                        .add_files(playlist.id, vec![media.clone(), media.clone()])
                        .await
                        .expect("add"),
                    1
                );
            }
            let reopened = open_test_library(&dirs).await;
            let playlists = reopened.playlists().await.expect("playlists");
            assert_eq!(playlists.len(), 1);
            assert_eq!(playlists[0].songs.len(), 1);
        });
    }

    #[test]
    fn refresh_virtual_removes_missing_paths() {
        futures_executor::block_on(async {
            let dirs = TestDirs::new();
            let media = dirs.data.join("song.flac");
            fs::write(&media, b"fake").expect("write media");
            let library = open_test_library(&dirs).await;
            let playlist = library.create_virtual("List".into()).await.expect("create");
            library
                .add_files(playlist.id, vec![media.clone()])
                .await
                .expect("add");
            fs::remove_file(media).expect("remove media");
            let refreshed = library.refresh(playlist.id).await.expect("refresh");
            assert!(refreshed.songs.is_empty());
        });
    }

    #[test]
    fn directory_playlist_scans_recursively_and_ignores_other_files() {
        futures_executor::block_on(async {
            let dirs = TestDirs::new();
            let nested = dirs.data.join("album");
            fs::create_dir_all(&nested).expect("create nested directory");
            fs::write(dirs.data.join("first.mp3"), b"fake").expect("write first media");
            fs::write(nested.join("second.flac"), b"fake").expect("write second media");
            fs::write(nested.join("notes.txt"), b"not media").expect("write text");
            let library = open_test_library(&dirs).await;
            let playlist = library
                .add_directory(dirs.data.clone())
                .await
                .expect("add directory");
            assert_eq!(playlist.songs.len(), 2);
            assert!(matches!(playlist.kind, PlaylistKind::Directory { .. }));
        });
    }

    #[test]
    fn json_manifest_migrates_into_turso() {
        futures_executor::block_on(async {
            let dirs = TestDirs::new();
            let manifest = LibraryManifest {
                next_id: 2,
                playlists: vec![Playlist {
                    id: 1,
                    name: "Legacy".into(),
                    kind: PlaylistKind::Virtual,
                    songs: vec![Song {
                        path: "/tmp/legacy.mp3".into(),
                        duration_ms: 10,
                        metadata: TrackMetadata {
                            title: Some("Legacy Song".into()),
                            cover: Some(CoverArt {
                                mime_type: Some("image/png".into()),
                                data: vec![1, 2, 3, 4],
                            }),
                            ..Default::default()
                        },
                    }],
                }],
            };
            fs::write(
                dirs.data.join("library.json"),
                serde_json::to_vec_pretty(&manifest).expect("serialize"),
            )
            .expect("write manifest");
            let library = open_test_library(&dirs).await;
            let playlists = library.playlists().await.expect("playlists");
            assert_eq!(playlists.len(), 1);
            assert_eq!(playlists[0].name, "Legacy");
            assert_eq!(playlists[0].songs.len(), 1);
            assert_eq!(
                playlists[0].songs[0].metadata.title.as_deref(),
                Some("Legacy Song")
            );
            assert!(playlists[0].songs[0].metadata.cover.is_none());
            let covers = library
                .covers_for_paths(vec!["/tmp/legacy.mp3".into()])
                .await
                .expect("covers");
            assert_eq!(covers.len(), 1);
            assert_eq!(covers[0].0, "/tmp/legacy.mp3");
            assert_eq!(covers[0].1.data, vec![1, 2, 3, 4]);
            assert!(!dirs.data.join("library.json").exists());
            assert!(dirs.data.join("library.json.migrated").exists());
        });
    }

    #[test]
    fn partial_json_migration_wipes_and_reimports() {
        futures_executor::block_on(async {
            let dirs = TestDirs::new();
            let manifest = LibraryManifest {
                next_id: 3,
                playlists: vec![
                    Playlist {
                        id: 1,
                        name: "First".into(),
                        kind: PlaylistKind::Virtual,
                        songs: vec![Song {
                            path: "/tmp/first.mp3".into(),
                            duration_ms: 10,
                            metadata: TrackMetadata {
                                title: Some("Song One".into()),
                                ..Default::default()
                            },
                        }],
                    },
                    Playlist {
                        id: 2,
                        name: "Second".into(),
                        kind: PlaylistKind::Virtual,
                        songs: vec![Song {
                            path: "/tmp/second.mp3".into(),
                            duration_ms: 20,
                            metadata: TrackMetadata {
                                title: Some("Song Two".into()),
                                ..Default::default()
                            },
                        }],
                    },
                ],
            };
            fs::write(
                dirs.data.join("library.json"),
                serde_json::to_vec_pretty(&manifest).expect("serialize"),
            )
            .expect("write manifest");

            // Simulate a partial migration: only the first playlist header was written.
            {
                let catalog = CatalogStore::open(&dirs.data.join(CATALOG_FILE_NAME))
                    .await
                    .expect("open catalog");
                catalog
                    .insert_playlist(1, "Stale Partial", &PlaylistKind::Virtual)
                    .await
                    .expect("insert partial playlist");
            }

            let library = open_test_library(&dirs).await;
            let playlists = library.playlists().await.expect("playlists");
            assert_eq!(playlists.len(), 2);
            assert_eq!(playlists[0].name, "First");
            assert_eq!(playlists[0].songs.len(), 1);
            assert_eq!(
                playlists[0].songs[0].metadata.title.as_deref(),
                Some("Song One")
            );
            assert_eq!(playlists[1].name, "Second");
            assert_eq!(playlists[1].songs.len(), 1);
            assert_eq!(
                playlists[1].songs[0].metadata.title.as_deref(),
                Some("Song Two")
            );
            assert!(!dirs.data.join("library.json").exists());
            assert!(dirs.data.join("library.json.migrated").exists());
        });
    }

    #[test]
    fn cover_cache_missing_file_yields_no_cover() {
        futures_executor::block_on(async {
            let dirs = TestDirs::new();
            let library = open_test_library(&dirs).await;
            let playlist = library
                .create_virtual("Covers".into())
                .await
                .expect("create");
            let media = dirs.data.join("art.mp3");
            fs::write(&media, b"fake").expect("write");
            library
                .add_files(playlist.id, vec![media])
                .await
                .expect("add");
            let playlists = library.playlists().await.expect("list");
            assert!(playlists[0].songs[0].metadata.cover.is_none());
        });
    }

    #[test]
    fn playback_mode_defaults_and_persists_across_reopen() {
        futures_executor::block_on(async {
            let dirs = TestDirs::new();
            {
                let library = open_test_library(&dirs).await;
                assert_eq!(
                    library.playback_mode().await.expect("default"),
                    StoredPlaybackMode::Sequential
                );
                library
                    .set_playback_mode(StoredPlaybackMode::Shuffle)
                    .await
                    .expect("set");
                assert_eq!(
                    library.playback_mode().await.expect("read"),
                    StoredPlaybackMode::Shuffle
                );
            }
            let reopened = open_test_library(&dirs).await;
            assert_eq!(
                reopened.playback_mode().await.expect("reopen"),
                StoredPlaybackMode::Shuffle
            );
            reopened
                .set_playback_mode(StoredPlaybackMode::RepeatOne)
                .await
                .expect("set repeat one");
            assert_eq!(
                reopened.playback_mode().await.expect("repeat one"),
                StoredPlaybackMode::RepeatOne
            );
        });
    }

    #[test]
    fn concurrent_meta_reads_do_not_fail() {
        futures_executor::block_on(async {
            let dirs = TestDirs::new();
            let library = open_test_library(&dirs).await;
            library
                .set_playback_mode(StoredPlaybackMode::RepeatAll)
                .await
                .expect("set");
            let (mode, volume, session) = futures_util::join!(
                library.playback_mode(),
                library.volume(),
                library.playback_session(),
            );
            assert_eq!(mode.expect("mode"), StoredPlaybackMode::RepeatAll);
            assert_eq!(volume.expect("volume"), 1.0);
            assert!(session.expect("session").is_none());
        });
    }

    #[test]
    fn concurrent_meta_read_and_write_do_not_fail() {
        futures_executor::block_on(async {
            let dirs = TestDirs::new();
            let library = open_test_library(&dirs).await;
            library.set_volume(0.25).await.expect("seed volume");
            let (set_mode, volume, set_volume) = futures_util::join!(
                library.set_playback_mode(StoredPlaybackMode::RepeatOne),
                library.volume(),
                library.set_volume(0.75),
            );
            set_mode.expect("set mode");
            let mid_volume = volume.expect("volume");
            assert!(
                (mid_volume - 0.25).abs() < f64::EPSILON
                    || (mid_volume - 0.75).abs() < f64::EPSILON,
                "concurrent volume read saw unexpected value {mid_volume}"
            );
            set_volume.expect("set volume");
            assert_eq!(
                library.playback_mode().await.expect("mode"),
                StoredPlaybackMode::RepeatOne
            );
            assert!((library.volume().await.expect("final volume") - 0.75).abs() < f64::EPSILON);
        });
    }

    #[test]
    fn volume_and_session_default_when_missing() {
        futures_executor::block_on(async {
            let dirs = TestDirs::new();
            let library = open_test_library(&dirs).await;
            assert_eq!(library.volume().await.expect("default volume"), 1.0);
            assert_eq!(
                library.playback_session().await.expect("default session"),
                None
            );
        });
    }

    #[test]
    fn volume_and_session_persist_across_reopen() {
        futures_executor::block_on(async {
            let dirs = TestDirs::new();
            let session = PlaybackSession {
                playlist_id: 7,
                path: "/music/track.flac".into(),
                position_ms: 12_345,
            };
            {
                let library = open_test_library(&dirs).await;
                library.set_volume(0.42).await.expect("set volume");
                library
                    .set_playback_session(session.clone())
                    .await
                    .expect("set session");
                assert!((library.volume().await.expect("read volume") - 0.42).abs() < f64::EPSILON);
                assert_eq!(
                    library.playback_session().await.expect("read session"),
                    Some(session.clone())
                );
            }
            let reopened = open_test_library(&dirs).await;
            assert!((reopened.volume().await.expect("reopen volume") - 0.42).abs() < f64::EPSILON);
            assert_eq!(
                reopened.playback_session().await.expect("reopen session"),
                Some(session)
            );
            reopened
                .set_volume(1.5)
                .await
                .expect("clamp volume above one");
            assert_eq!(reopened.volume().await.expect("clamped"), 1.0);
            reopened
                .clear_playback_session()
                .await
                .expect("clear session");
            assert_eq!(reopened.playback_session().await.expect("cleared"), None);
        });
    }

    #[test]
    fn corrupt_playback_session_is_cleared() {
        futures_executor::block_on(async {
            let dirs = TestDirs::new();
            {
                let catalog = CatalogStore::open(&dirs.data.join(CATALOG_FILE_NAME))
                    .await
                    .expect("open catalog");
                catalog
                    .upsert_meta_raw("playback_session", "{not-json")
                    .await
                    .expect("plant corrupt session");
            }
            let library = open_test_library(&dirs).await;
            assert_eq!(
                library.playback_session().await.expect("corrupt cleared"),
                None
            );
            assert_eq!(
                library.playback_session().await.expect("stays cleared"),
                None
            );
        });
    }
}
