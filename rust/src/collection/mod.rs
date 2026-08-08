//! Persistent local media collection and user-managed playlists.

mod cover_store;
mod error;
mod model;
mod scan;
mod store;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use cover_store::CoverStore;
use directories::ProjectDirs;
pub use error::CollectionError;
pub(crate) use model::{CoverAnimationInBackground, StoredPlaybackMode};
pub use model::{PlaybackSession, Playlist, PlaylistId, PlaylistKind, Song};
use model::{SongId, SongTags};
use scan::CollectionScanner;
use store::{Catalog, CatalogStore};

use crate::player::metadata::{CoverArt, TrackMetadata};
use crate::player::{MediaInspector, SymphoniaMediaInspector};

const CATALOG_FILE_NAME: &str = "catalog.db";
const COVERS_DIR_NAME: &str = "covers";

/// Persistent collection service backed by Turso catalog + cover cache.
///
/// Lyrics are not stored in the collection DB; playback loads them from the media
/// file / sidecar at decode time. Any leftover `lyrics.db` on disk is unused.
pub(crate) struct Collection<C: Catalog = CatalogStore> {
    catalog: C,
    covers: CoverStore,
    inspector: Box<dyn MediaInspector>,
}

impl Collection<CatalogStore> {
    /// Open the collection using platform data + cache directories.
    pub(crate) async fn open_default() -> Result<Self, CollectionError> {
        let dirs = ProjectDirs::from("dev", "mofu", "mofu")
            .ok_or(CollectionError::DataDirectoryUnavailable)?;
        Self::open_at_dirs(
            dirs.data_dir().to_path_buf(),
            dirs.cache_dir().join(COVERS_DIR_NAME),
        )
        .await
    }

    /// Open under an explicit data directory; covers use the platform cache.
    pub(crate) async fn open_at(data_dir: PathBuf) -> Result<Self, CollectionError> {
        let cache_dir = ProjectDirs::from("dev", "mofu", "mofu")
            .map(|dirs| dirs.cache_dir().join(COVERS_DIR_NAME))
            .ok_or(CollectionError::CacheDirectoryUnavailable)?;
        Self::open_at_dirs(data_dir, cache_dir).await
    }

    /// Open with explicit data and cover-cache directories (tests).
    pub(crate) async fn open_at_dirs(
        data_dir: PathBuf,
        cache_dir: PathBuf,
    ) -> Result<Self, CollectionError> {
        Self::open_with_inspector(data_dir, cache_dir, Box::new(SymphoniaMediaInspector)).await
    }

    async fn open_with_inspector(
        data_dir: PathBuf,
        cache_dir: PathBuf,
        inspector: Box<dyn MediaInspector>,
    ) -> Result<Self, CollectionError> {
        fs::create_dir_all(&data_dir)?;
        let catalog = CatalogStore::open(&data_dir.join(CATALOG_FILE_NAME)).await?;
        let covers = CoverStore::open(cache_dir)?;
        Ok(Self {
            catalog,
            covers,
            inspector,
        })
    }
}

impl<C: Catalog> Collection<C> {
    pub(crate) async fn playlists(&self) -> Result<Vec<Playlist>, CollectionError> {
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

    pub(crate) async fn create_virtual(&self, name: String) -> Result<Playlist, CollectionError> {
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

    pub(crate) async fn add_directory(&self, path: PathBuf) -> Result<Playlist, CollectionError> {
        let canonical = fs::canonicalize(path)?;
        let path_string = canonical.to_string_lossy().into_owned();
        let songs = CollectionScanner::new(self.inspector.as_ref()).scan_directory(&canonical);
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
    ) -> Result<(), CollectionError> {
        let name = Self::validated_name(name)?;
        let row = self.catalog.playlist_row(id).await?;
        if row.kind != PlaylistKind::Virtual {
            return Err(CollectionError::NotVirtualPlaylist);
        }
        self.catalog.rename_playlist(id, &name).await
    }

    pub(crate) async fn delete_virtual(&self, id: PlaylistId) -> Result<(), CollectionError> {
        let row = self.catalog.playlist_row(id).await?;
        if row.kind != PlaylistKind::Virtual {
            return Err(CollectionError::NotVirtualPlaylist);
        }
        self.catalog.delete_playlist(id).await?;
        self.garbage_collect().await
    }

    pub(crate) async fn add_files(
        &self,
        id: PlaylistId,
        paths: Vec<PathBuf>,
    ) -> Result<usize, CollectionError> {
        let row = self.catalog.playlist_row(id).await?;
        if row.kind != PlaylistKind::Virtual {
            return Err(CollectionError::NotVirtualPlaylist);
        }
        let inspected = CollectionScanner::new(self.inspector.as_ref()).inspect_paths(paths);
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
    ) -> Result<usize, CollectionError> {
        let target = self.catalog.playlist_row(target_id).await?;
        if target.kind != PlaylistKind::Virtual {
            return Err(CollectionError::NotVirtualPlaylist);
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

    pub(crate) async fn refresh(&self, id: PlaylistId) -> Result<Playlist, CollectionError> {
        let row = self.catalog.playlist_row(id).await?;
        let songs = match &row.kind {
            PlaylistKind::Directory { path } => {
                CollectionScanner::new(self.inspector.as_ref()).scan_directory(Path::new(path))
            }
            PlaylistKind::Virtual => {
                let paths = self
                    .catalog
                    .playlist_song_paths(id)
                    .await?
                    .into_iter()
                    .map(PathBuf::from)
                    .collect::<Vec<_>>();
                CollectionScanner::new(self.inspector.as_ref()).inspect_paths(paths)
            }
        };
        let song_ids = self.persist_songs(&songs).await?;
        self.catalog.replace_playlist_songs(id, &song_ids).await?;
        self.garbage_collect().await?;
        self.playlist_by_id(id).await
    }

    async fn playlist_by_id(&self, id: PlaylistId) -> Result<Playlist, CollectionError> {
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
    ) -> Result<Vec<(String, CoverArt)>, CollectionError> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let rows = self.catalog.song_rows_for_paths(&paths).await?;
        let mut covers = Vec::new();
        for row in rows {
            let Some(file_type) = row.tags.cover_file_type else {
                continue;
            };
            let Some(cover) = self.covers.read(row.id, file_type, row.tags.cover_mime) else {
                continue;
            };
            covers.push((row.path, cover));
        }
        Ok(covers)
    }

    async fn load_songs_for_playlist(
        &self,
        playlist_id: PlaylistId,
    ) -> Result<Vec<Song>, CollectionError> {
        let rows = self.catalog.songs_for_playlist(playlist_id).await?;
        let mut songs = Vec::with_capacity(rows.len());
        for row in rows {
            // 首包只带文字标签；封面经 covers_for_paths 异步补全，歌词由播放路径提供。
            songs.push(Song::from(row));
        }
        Ok(songs)
    }

    async fn persist_songs(&self, songs: &[Song]) -> Result<Vec<SongId>, CollectionError> {
        let mut ids = Vec::with_capacity(songs.len());
        for song in songs {
            ids.push(self.persist_song(song).await?);
        }
        Ok(ids)
    }

    async fn persist_song(&self, song: &Song) -> Result<SongId, CollectionError> {
        let tags = SongTags::from_metadata(&song.metadata, None);
        let song_id = self
            .catalog
            .upsert_song(&song.path, song.duration_ms, &tags)
            .await?;
        let cover_file_type = if let Some(cover) = song.metadata.cover.as_ref() {
            Some(self.covers.write(song_id, cover)?)
        } else {
            self.covers.remove_all(song_id);
            None
        };
        let tags = SongTags::from_metadata(&song.metadata, cover_file_type);
        self.catalog
            .upsert_song(&song.path, song.duration_ms, &tags)
            .await
    }

    async fn garbage_collect(&self) -> Result<(), CollectionError> {
        let orphans = self.catalog.garbage_collect_orphan_songs().await?;
        for (song_id, cover_file_type) in orphans {
            if let Some(file_type) = cover_file_type {
                self.covers.remove(song_id, file_type);
            } else {
                self.covers.remove_all(song_id);
            }
        }
        Ok(())
    }

    pub(crate) async fn playback_mode(&self) -> Result<StoredPlaybackMode, CollectionError> {
        self.catalog.playback_mode().await
    }

    pub(crate) async fn set_playback_mode(
        &self,
        mode: StoredPlaybackMode,
    ) -> Result<(), CollectionError> {
        self.catalog.set_playback_mode(mode).await
    }

    pub(crate) async fn cover_animation_in_background(
        &self,
    ) -> Result<CoverAnimationInBackground, CollectionError> {
        self.catalog.cover_animation_in_background().await
    }

    pub(crate) async fn set_cover_animation_in_background(
        &self,
        value: CoverAnimationInBackground,
    ) -> Result<(), CollectionError> {
        self.catalog.set_cover_animation_in_background(value).await
    }

    pub(crate) async fn volume(&self) -> Result<f64, CollectionError> {
        self.catalog.volume().await
    }

    pub(crate) async fn set_volume(&self, volume: f64) -> Result<(), CollectionError> {
        self.catalog.set_volume(volume).await
    }

    pub(crate) async fn playback_session(
        &self,
    ) -> Result<Option<PlaybackSession>, CollectionError> {
        self.catalog.playback_session().await
    }

    pub(crate) async fn set_playback_session(
        &self,
        session: PlaybackSession,
    ) -> Result<(), CollectionError> {
        self.catalog.set_playback_session(&session).await
    }

    pub(crate) async fn clear_playback_session(&self) -> Result<(), CollectionError> {
        self.catalog.clear_playback_session().await
    }

    /// Validate a persisted session against the current catalog.
    ///
    /// Returns `None` (and clears the stored session) when the playlist or path is gone.
    pub(crate) async fn resolve_playback_session(
        &self,
        session: PlaybackSession,
    ) -> Result<Option<SessionRestorePlan>, CollectionError> {
        let row = match self.catalog.playlist_row(session.playlist_id).await {
            Ok(row) => row,
            Err(CollectionError::PlaylistNotFound) => {
                self.clear_playback_session().await?;
                return Ok(None);
            }
            Err(err) => return Err(err),
        };
        let paths = self
            .catalog
            .playlist_song_paths(session.playlist_id)
            .await?;
        let Some(start_index) = paths.iter().position(|path| path == &session.path) else {
            self.clear_playback_session().await?;
            return Ok(None);
        };
        Ok(Some(SessionRestorePlan {
            playlist_id: row.id,
            paths,
            start_index,
            position_ms: session.position_ms,
        }))
    }

    /// Build a library snapshot with lite songs, then attach covers in batches.
    pub(crate) async fn library_snapshot_lite(&self) -> Result<Vec<Playlist>, CollectionError> {
        self.playlists().await
    }

    /// Load covers for every song missing artwork and return playlists with covers merged.
    pub(crate) async fn library_snapshot_with_covers(
        &self,
    ) -> Result<Vec<Playlist>, CollectionError> {
        let mut playlists = self.playlists().await?;
        let paths = playlists
            .iter()
            .flat_map(|playlist| playlist.songs.iter())
            .filter(|song| song.metadata.cover.is_none())
            .map(|song| song.path.clone())
            .collect::<Vec<_>>();
        if paths.is_empty() {
            return Ok(playlists);
        }

        let mut by_path = HashMap::new();
        for chunk in paths.chunks(COVER_HYDRATE_CHUNK_SIZE) {
            for (path, cover) in self.covers_for_paths(chunk.to_vec()).await? {
                by_path.insert(path, cover);
            }
        }
        if by_path.is_empty() {
            return Ok(playlists);
        }

        for playlist in &mut playlists {
            for song in &mut playlist.songs {
                if let Some(cover) = by_path.remove(&song.path) {
                    song.metadata.cover = Some(cover);
                }
            }
        }
        Ok(playlists)
    }

    /// Pick a virtual playlist for file import, creating one when needed.
    pub(crate) async fn ensure_virtual_import_target(
        &self,
        selected_id: Option<PlaylistId>,
    ) -> Result<Playlist, CollectionError> {
        if let Some(id) = selected_id {
            match self.catalog.playlist_row(id).await {
                Ok(row) if row.kind == PlaylistKind::Virtual => {
                    return self.playlist_by_id(id).await;
                }
                Ok(_) | Err(CollectionError::PlaylistNotFound) => {}
                Err(err) => return Err(err),
            }
        }
        self.create_virtual(DEFAULT_IMPORT_PLAYLIST_NAME.to_owned())
            .await
    }

    fn validated_name(name: String) -> Result<String, CollectionError> {
        let name = name.trim().to_owned();
        if name.is_empty() {
            Err(CollectionError::EmptyPlaylistName)
        } else {
            Ok(name)
        }
    }
}

impl Playlist {
    /// First non-empty cover in playlist order.
    pub(crate) fn cover(&self) -> Option<&CoverArt> {
        self.songs.iter().find_map(|song| {
            song.metadata
                .cover
                .as_ref()
                .filter(|cover| !cover.data.is_empty())
        })
    }
}

impl Song {
    /// Metadata title with the path basename as a fallback.
    pub(crate) fn display_title(&self) -> String {
        TrackMetadata::display_title(&self.path, self.metadata.title.as_deref())
    }
}

/// Validated resume plan ready for the player queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionRestorePlan {
    /// Source playlist containing the restored track.
    pub playlist_id: PlaylistId,
    /// Queue paths in persisted playlist order.
    pub paths: Vec<String>,
    /// Selected queue index for the persisted path.
    pub start_index: usize,
    /// Resume position within the selected track.
    pub position_ms: u64,
}

const COVER_HYDRATE_CHUNK_SIZE: usize = 32;
const DEFAULT_IMPORT_PLAYLIST_NAME: &str = "导入的歌曲";

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::store::{Catalog, CatalogStore};
    use super::*;
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
            let base = std::env::temp_dir()
                .join(format!("mofu-collection-{}-{nonce}", std::process::id()));
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

    async fn open_test_collection(dirs: &TestDirs) -> Collection {
        Collection::open_with_inspector(
            dirs.data.clone(),
            dirs.cache.clone(),
            Box::new(FakeInspector),
        )
        .await
        .expect("open collection")
    }

    #[test]
    fn virtual_playlist_persists_and_deduplicates_files() {
        async_global_executor::block_on(async {
            let dirs = TestDirs::new();
            let media = dirs.data.join("song.mp3");
            fs::write(&media, b"fake").expect("write media");
            {
                let collection = open_test_collection(&dirs).await;
                let playlist = collection
                    .create_virtual("Favorites".into())
                    .await
                    .expect("create");
                assert_eq!(
                    collection
                        .add_files(playlist.id, vec![media.clone(), media.clone()])
                        .await
                        .expect("add"),
                    1
                );
            }
            let reopened = open_test_collection(&dirs).await;
            let playlists = reopened.playlists().await.expect("playlists");
            assert_eq!(playlists.len(), 1);
            assert_eq!(playlists[0].songs.len(), 1);
        });
    }

    #[test]
    fn refresh_virtual_removes_missing_paths() {
        async_global_executor::block_on(async {
            let dirs = TestDirs::new();
            let media = dirs.data.join("song.flac");
            fs::write(&media, b"fake").expect("write media");
            let collection = open_test_collection(&dirs).await;
            let playlist = collection
                .create_virtual("List".into())
                .await
                .expect("create");
            collection
                .add_files(playlist.id, vec![media.clone()])
                .await
                .expect("add");
            fs::remove_file(media).expect("remove media");
            let refreshed = collection.refresh(playlist.id).await.expect("refresh");
            assert!(refreshed.songs.is_empty());
        });
    }

    #[test]
    fn directory_playlist_scans_recursively_and_ignores_other_files() {
        async_global_executor::block_on(async {
            let dirs = TestDirs::new();
            let nested = dirs.data.join("album");
            fs::create_dir_all(&nested).expect("create nested directory");
            fs::write(dirs.data.join("first.mp3"), b"fake").expect("write first media");
            fs::write(nested.join("second.flac"), b"fake").expect("write second media");
            fs::write(nested.join("notes.txt"), b"not media").expect("write text");
            let collection = open_test_collection(&dirs).await;
            let playlist = collection
                .add_directory(dirs.data.clone())
                .await
                .expect("add directory");
            assert_eq!(playlist.songs.len(), 2);
            assert!(matches!(playlist.kind, PlaylistKind::Directory { .. }));
        });
    }

    #[test]
    fn cover_cache_missing_file_yields_no_cover() {
        async_global_executor::block_on(async {
            let dirs = TestDirs::new();
            let collection = open_test_collection(&dirs).await;
            let playlist = collection
                .create_virtual("Covers".into())
                .await
                .expect("create");
            let media = dirs.data.join("art.mp3");
            fs::write(&media, b"fake").expect("write");
            collection
                .add_files(playlist.id, vec![media])
                .await
                .expect("add");
            let playlists = collection.playlists().await.expect("list");
            assert!(playlists[0].songs[0].metadata.cover.is_none());
        });
    }

    #[test]
    fn playback_mode_defaults_and_persists_across_reopen() {
        async_global_executor::block_on(async {
            let dirs = TestDirs::new();
            {
                let collection = open_test_collection(&dirs).await;
                assert_eq!(
                    collection.playback_mode().await.expect("default"),
                    StoredPlaybackMode::Sequential
                );
                collection
                    .set_playback_mode(StoredPlaybackMode::Shuffle)
                    .await
                    .expect("set");
                assert_eq!(
                    collection.playback_mode().await.expect("read"),
                    StoredPlaybackMode::Shuffle
                );
            }
            let reopened = open_test_collection(&dirs).await;
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
    fn cover_animation_defaults_and_persists_across_reopen() {
        async_global_executor::block_on(async {
            let dirs = TestDirs::new();
            {
                let collection = open_test_collection(&dirs).await;
                assert_eq!(
                    collection
                        .cover_animation_in_background()
                        .await
                        .expect("default"),
                    CoverAnimationInBackground::Pause
                );
                collection
                    .set_cover_animation_in_background(CoverAnimationInBackground::Continue)
                    .await
                    .expect("set");
                assert_eq!(
                    collection
                        .cover_animation_in_background()
                        .await
                        .expect("read"),
                    CoverAnimationInBackground::Continue
                );
            }
            let reopened = open_test_collection(&dirs).await;
            assert_eq!(
                reopened
                    .cover_animation_in_background()
                    .await
                    .expect("reopen"),
                CoverAnimationInBackground::Continue
            );
            reopened
                .set_cover_animation_in_background(CoverAnimationInBackground::Pause)
                .await
                .expect("set pause");
            assert_eq!(
                reopened
                    .cover_animation_in_background()
                    .await
                    .expect("pause"),
                CoverAnimationInBackground::Pause
            );
        });
    }

    #[test]
    fn corrupt_cover_animation_falls_back_to_pause() {
        async_global_executor::block_on(async {
            let dirs = TestDirs::new();
            {
                let catalog = CatalogStore::open(&dirs.data.join(CATALOG_FILE_NAME))
                    .await
                    .expect("open catalog");
                catalog
                    .upsert_meta_raw("cover_animation_in_background", "spin-forever")
                    .await
                    .expect("plant corrupt value");
            }
            let collection = open_test_collection(&dirs).await;
            assert_eq!(
                collection
                    .cover_animation_in_background()
                    .await
                    .expect("corrupt cleared"),
                CoverAnimationInBackground::Pause
            );
            assert_eq!(
                collection
                    .cover_animation_in_background()
                    .await
                    .expect("stays default"),
                CoverAnimationInBackground::Pause
            );
        });
    }

    #[test]
    fn concurrent_meta_reads_do_not_fail() {
        async_global_executor::block_on(async {
            let dirs = TestDirs::new();
            let collection = open_test_collection(&dirs).await;
            collection
                .set_playback_mode(StoredPlaybackMode::RepeatAll)
                .await
                .expect("set");
            let (mode, volume, session) = futures_util::join!(
                collection.playback_mode(),
                collection.volume(),
                collection.playback_session(),
            );
            assert_eq!(mode.expect("mode"), StoredPlaybackMode::RepeatAll);
            assert_eq!(volume.expect("volume"), 1.0);
            assert!(session.expect("session").is_none());
        });
    }

    #[test]
    fn concurrent_meta_read_and_write_do_not_fail() {
        async_global_executor::block_on(async {
            let dirs = TestDirs::new();
            let collection = open_test_collection(&dirs).await;
            collection.set_volume(0.25).await.expect("seed volume");
            let (set_mode, volume, set_volume) = futures_util::join!(
                collection.set_playback_mode(StoredPlaybackMode::RepeatOne),
                collection.volume(),
                collection.set_volume(0.75),
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
                collection.playback_mode().await.expect("mode"),
                StoredPlaybackMode::RepeatOne
            );
            assert!((collection.volume().await.expect("final volume") - 0.75).abs() < f64::EPSILON);
        });
    }

    #[test]
    fn volume_and_session_default_when_missing() {
        async_global_executor::block_on(async {
            let dirs = TestDirs::new();
            let collection = open_test_collection(&dirs).await;
            assert_eq!(collection.volume().await.expect("default volume"), 1.0);
            assert_eq!(
                collection
                    .playback_session()
                    .await
                    .expect("default session"),
                None
            );
        });
    }

    #[test]
    fn volume_and_session_persist_across_reopen() {
        async_global_executor::block_on(async {
            let dirs = TestDirs::new();
            let session = PlaybackSession {
                playlist_id: 7,
                path: "/music/track.flac".into(),
                position_ms: 12_345,
            };
            {
                let collection = open_test_collection(&dirs).await;
                collection.set_volume(0.42).await.expect("set volume");
                collection
                    .set_playback_session(session.clone())
                    .await
                    .expect("set session");
                assert!(
                    (collection.volume().await.expect("read volume") - 0.42).abs() < f64::EPSILON
                );
                assert_eq!(
                    collection.playback_session().await.expect("read session"),
                    Some(session.clone())
                );
            }
            let reopened = open_test_collection(&dirs).await;
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
        async_global_executor::block_on(async {
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
            let collection = open_test_collection(&dirs).await;
            assert_eq!(
                collection
                    .playback_session()
                    .await
                    .expect("corrupt cleared"),
                None
            );
            assert_eq!(
                collection.playback_session().await.expect("stays cleared"),
                None
            );
        });
    }

    #[test]
    fn open_creates_schema_when_db_file_exists_without_tables() {
        async_global_executor::block_on(async {
            let dirs = TestDirs::new();
            let path = dirs.data.join(CATALOG_FILE_NAME);
            fs::write(&path, []).expect("create empty db file");
            let catalog = CatalogStore::open(&path)
                .await
                .expect("open empty existing file");
            catalog
                .set_volume(0.5)
                .await
                .expect("write after schema bootstrap");
            let reopened = CatalogStore::open(&path).await.expect("reopen");
            assert_eq!(reopened.volume().await.expect("volume"), 0.5);
        });
    }

    #[test]
    fn open_recreates_missing_tables_after_partial_schema() {
        async_global_executor::block_on(async {
            let dirs = TestDirs::new();
            let path = dirs.data.join(CATALOG_FILE_NAME);
            {
                let catalog = CatalogStore::open(&path).await.expect("bootstrap");
                catalog
                    .set_volume(0.25)
                    .await
                    .expect("seed volume via meta");
                catalog
                    .exec_raw_statement("DROP TABLE songs")
                    .await
                    .expect("drop songs to simulate partial schema");
            }
            let catalog = CatalogStore::open(&path)
                .await
                .expect("reopen repairs missing songs table");
            assert_eq!(catalog.volume().await.expect("meta intact"), 0.25);
            catalog
                .upsert_song("/tmp/repaired.mp3", 1, &SongTags::default())
                .await
                .expect("songs table usable after repair");
        });
    }

    #[test]
    fn invalid_numeric_fields_are_rejected() {
        async_global_executor::block_on(async {
            let dirs = TestDirs::new();
            let catalog = CatalogStore::open(&dirs.data.join(CATALOG_FILE_NAME))
                .await
                .expect("open catalog");
            catalog
                .insert_song_raw("/tmp/bad-year.mp3", 1, Some(-1))
                .await
                .expect("plant negative year");
            let err = catalog
                .song_rows_for_paths(&["/tmp/bad-year.mp3".to_owned()])
                .await
                .expect_err("negative year must fail");
            assert!(matches!(
                err,
                CollectionError::InvalidData(super::error::CatalogDataError::InvalidNumericField(
                    super::error::CatalogNumericField::Year
                ))
            ));

            catalog
                .insert_song_raw("/tmp/bad-duration.mp3", -5, None)
                .await
                .expect("plant negative duration");
            let err = catalog
                .song_rows_for_paths(&["/tmp/bad-duration.mp3".to_owned()])
                .await
                .expect_err("negative duration must fail");
            assert!(matches!(
                err,
                CollectionError::InvalidData(super::error::CatalogDataError::InvalidNumericField(
                    super::error::CatalogNumericField::DurationMs
                ))
            ));
        });
    }

    #[test]
    fn garbage_collect_removes_orphan_songs_and_covers() {
        async_global_executor::block_on(async {
            let dirs = TestDirs::new();
            let collection = open_test_collection(&dirs).await;
            let playlist = collection
                .create_virtual("temp".into())
                .await
                .expect("create playlist");
            let audio = dirs.data.join("orphan.mp3");
            fs::write(&audio, b"fake").expect("write audio stub");
            collection
                .add_files(playlist.id, vec![audio.clone()])
                .await
                .expect("add file");

            let catalog = CatalogStore::open(&dirs.data.join(CATALOG_FILE_NAME))
                .await
                .expect("open catalog");
            let rows = catalog
                .song_rows_for_paths(&[audio.to_string_lossy().into_owned()])
                .await
                .expect("lookup song");
            let song = rows.into_iter().next().expect("song row");
            let cover_path = dirs.cache.join(format!("{}.jpg", song.id));
            fs::write(&cover_path, b"cover").expect("plant cover file");
            catalog
                .exec_raw_statement(&format!(
                    "UPDATE songs SET cover_ext = '.jpg' WHERE id = {}",
                    song.id
                ))
                .await
                .expect("mark cover ext");

            collection
                .delete_virtual(playlist.id)
                .await
                .expect("delete playlist triggers GC");
            assert!(
                !cover_path.exists(),
                "orphan cover file should be removed after GC"
            );
            let leftover = catalog
                .song_rows_for_paths(&[audio.to_string_lossy().into_owned()])
                .await
                .expect("lookup after GC");
            assert!(leftover.is_empty(), "orphan song row should be deleted");
        });
    }

    #[test]
    fn resolve_playback_session_validates_path_and_clears_invalid() {
        async_global_executor::block_on(async {
            let dirs = TestDirs::new();
            let media = dirs.data.join("song.mp3");
            fs::write(&media, b"fake").expect("write media");
            let collection = open_test_collection(&dirs).await;
            let playlist = collection
                .create_virtual("Resume".into())
                .await
                .expect("create");
            collection
                .add_files(playlist.id, vec![media.clone()])
                .await
                .expect("add");
            let path = media.to_string_lossy().into_owned();
            let plan = collection
                .resolve_playback_session(PlaybackSession {
                    playlist_id: playlist.id,
                    path: path.clone(),
                    position_ms: 1234,
                })
                .await
                .expect("resolve")
                .expect("valid session");
            assert_eq!(plan.playlist_id, playlist.id);
            assert_eq!(plan.start_index, 0);
            assert_eq!(plan.position_ms, 1234);

            let cleared = collection
                .resolve_playback_session(PlaybackSession {
                    playlist_id: playlist.id,
                    path: "/missing.mp3".into(),
                    position_ms: 1,
                })
                .await
                .expect("resolve missing");
            assert!(cleared.is_none());
            assert!(
                collection
                    .playback_session()
                    .await
                    .expect("session after clear")
                    .is_none()
            );
        });
    }

    #[test]
    fn ensure_virtual_import_target_reuses_or_creates() {
        async_global_executor::block_on(async {
            let dirs = TestDirs::new();
            let collection = open_test_collection(&dirs).await;
            let created = collection
                .create_virtual("目标".into())
                .await
                .expect("create");
            let reused = collection
                .ensure_virtual_import_target(Some(created.id))
                .await
                .expect("reuse");
            assert_eq!(reused.id, created.id);

            let directory = collection
                .add_directory(dirs.data.clone())
                .await
                .expect("directory playlist");
            let fallback = collection
                .ensure_virtual_import_target(Some(directory.id))
                .await
                .expect("create import playlist");
            assert_eq!(fallback.name, "导入的歌曲");
            assert_eq!(fallback.kind, PlaylistKind::Virtual);
        });
    }

    #[test]
    fn playlist_cover_picks_first_non_empty() {
        let playlist = Playlist {
            id: 1,
            name: "x".into(),
            kind: PlaylistKind::Virtual,
            songs: vec![
                Song {
                    path: "/a.mp3".into(),
                    duration_ms: 1,
                    metadata: TrackMetadata::default(),
                },
                Song {
                    path: "/b.mp3".into(),
                    duration_ms: 1,
                    metadata: TrackMetadata {
                        cover: Some(CoverArt {
                            mime_type: Some("image/jpeg".into()),
                            data: vec![1, 2, 3],
                        }),
                        ..Default::default()
                    },
                },
            ],
        };
        let cover = playlist.cover().expect("cover");
        assert_eq!(cover.data, vec![1, 2, 3]);
    }
}
