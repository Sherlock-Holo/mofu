//! Persistent local media library and user-managed playlists.

mod error;
mod model;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use directories::ProjectDirs;
pub use error::LibraryError;
use model::LibraryManifest;
pub use model::{Playlist, PlaylistId, PlaylistKind, Song};
use walkdir::WalkDir;

use crate::player::{MediaInspection, MediaInspector, SymphoniaMediaInspector};

const MANIFEST_FILE_NAME: &str = "library.json";

/// Thread-safe persistent library service.
pub(crate) struct Library {
    manifest_path: PathBuf,
    inspector: Box<dyn MediaInspector>,
    manifest: Mutex<LibraryManifest>,
}

impl Library {
    /// Open the library in the platform application data directory.
    pub(crate) fn open_default() -> Result<Self, LibraryError> {
        let dirs = ProjectDirs::from("dev", "mofu", "mofu")
            .ok_or(LibraryError::DataDirectoryUnavailable)?;
        Self::open_at(dirs.data_dir().join(MANIFEST_FILE_NAME))
    }

    /// Open a manifest at an explicit location.
    pub(crate) fn open_at(manifest_path: PathBuf) -> Result<Self, LibraryError> {
        Self::open_with_inspector(manifest_path, Box::new(SymphoniaMediaInspector))
    }

    fn open_with_inspector(
        manifest_path: PathBuf,
        inspector: Box<dyn MediaInspector>,
    ) -> Result<Self, LibraryError> {
        let manifest = if manifest_path.exists() {
            serde_json::from_slice(&fs::read(&manifest_path)?)?
        } else {
            LibraryManifest::default()
        };
        Ok(Self {
            manifest_path,
            inspector,
            manifest: Mutex::new(manifest),
        })
    }

    pub(crate) fn playlists(&self) -> Vec<Playlist> {
        self.manifest
            .lock()
            .expect("library manifest mutex poisoned")
            .playlists
            .clone()
    }

    pub(crate) fn create_virtual(&self, name: String) -> Result<Playlist, LibraryError> {
        let name = Self::validated_name(name)?;
        let mut manifest = self
            .manifest
            .lock()
            .expect("library manifest mutex poisoned");
        let playlist = Playlist {
            id: manifest.allocate_id(),
            name,
            kind: PlaylistKind::Virtual,
            songs: Vec::new(),
        };
        manifest.playlists.push(playlist.clone());
        self.save(&manifest)?;
        Ok(playlist)
    }

    pub(crate) fn add_directory(&self, path: PathBuf) -> Result<Playlist, LibraryError> {
        let canonical = fs::canonicalize(path)?;
        let path_string = canonical.to_string_lossy().into_owned();
        let songs = self.scan_directory(&canonical);
        let name = canonical
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or(&path_string)
            .to_owned();
        let mut manifest = self
            .manifest
            .lock()
            .expect("library manifest mutex poisoned");
        if let Some(existing) = manifest.playlists.iter_mut().find(|playlist| {
            matches!(&playlist.kind, PlaylistKind::Directory { path } if path == &path_string)
        }) {
            existing.songs = songs;
            let result = existing.clone();
            self.save(&manifest)?;
            return Ok(result);
        }
        let playlist = Playlist {
            id: manifest.allocate_id(),
            name,
            kind: PlaylistKind::Directory { path: path_string },
            songs,
        };
        manifest.playlists.push(playlist.clone());
        self.save(&manifest)?;
        Ok(playlist)
    }

    pub(crate) fn rename_virtual(&self, id: PlaylistId, name: String) -> Result<(), LibraryError> {
        let name = Self::validated_name(name)?;
        let mut manifest = self
            .manifest
            .lock()
            .expect("library manifest mutex poisoned");
        let playlist = manifest
            .playlists
            .iter_mut()
            .find(|playlist| playlist.id == id)
            .ok_or(LibraryError::PlaylistNotFound)?;
        if playlist.kind != PlaylistKind::Virtual {
            return Err(LibraryError::NotVirtualPlaylist);
        }
        playlist.name = name;
        self.save(&manifest)
    }

    pub(crate) fn delete_virtual(&self, id: PlaylistId) -> Result<(), LibraryError> {
        let mut manifest = self
            .manifest
            .lock()
            .expect("library manifest mutex poisoned");
        let index = manifest
            .playlists
            .iter()
            .position(|playlist| playlist.id == id)
            .ok_or(LibraryError::PlaylistNotFound)?;
        if manifest.playlists[index].kind != PlaylistKind::Virtual {
            return Err(LibraryError::NotVirtualPlaylist);
        }
        manifest.playlists.remove(index);
        self.save(&manifest)
    }

    pub(crate) fn add_files(
        &self,
        id: PlaylistId,
        paths: Vec<PathBuf>,
    ) -> Result<usize, LibraryError> {
        let inspected = paths
            .into_iter()
            .filter_map(|path| self.inspect_path(&path))
            .collect::<Vec<_>>();
        let mut manifest = self
            .manifest
            .lock()
            .expect("library manifest mutex poisoned");
        let playlist = manifest
            .playlists
            .iter_mut()
            .find(|playlist| playlist.id == id)
            .ok_or(LibraryError::PlaylistNotFound)?;
        if playlist.kind != PlaylistKind::Virtual {
            return Err(LibraryError::NotVirtualPlaylist);
        }
        let mut existing = playlist
            .songs
            .iter()
            .map(|song| song.path.clone())
            .collect::<HashSet<_>>();
        let before = playlist.songs.len();
        for song in inspected {
            if existing.insert(song.path.clone()) {
                playlist.songs.push(song);
            }
        }
        let added = playlist.songs.len() - before;
        self.save(&manifest)?;
        Ok(added)
    }

    pub(crate) fn import_from_playlist(
        &self,
        target_id: PlaylistId,
        source_id: PlaylistId,
        paths: Vec<String>,
    ) -> Result<usize, LibraryError> {
        let selected = paths.into_iter().collect::<HashSet<_>>();
        let mut manifest = self
            .manifest
            .lock()
            .expect("library manifest mutex poisoned");
        let source = manifest
            .playlists
            .iter()
            .find(|playlist| playlist.id == source_id)
            .ok_or(LibraryError::PlaylistNotFound)?;
        let songs = source
            .songs
            .iter()
            .filter(|song| selected.is_empty() || selected.contains(&song.path))
            .cloned()
            .collect::<Vec<_>>();
        let target = manifest
            .playlists
            .iter_mut()
            .find(|playlist| playlist.id == target_id)
            .ok_or(LibraryError::PlaylistNotFound)?;
        if target.kind != PlaylistKind::Virtual {
            return Err(LibraryError::NotVirtualPlaylist);
        }
        let mut existing = target
            .songs
            .iter()
            .map(|song| song.path.clone())
            .collect::<HashSet<_>>();
        let before = target.songs.len();
        target.songs.extend(
            songs
                .into_iter()
                .filter(|song| existing.insert(song.path.clone())),
        );
        let added = target.songs.len() - before;
        self.save(&manifest)?;
        Ok(added)
    }

    pub(crate) fn refresh(&self, id: PlaylistId) -> Result<Playlist, LibraryError> {
        let kind = {
            let manifest = self
                .manifest
                .lock()
                .expect("library manifest mutex poisoned");
            manifest
                .playlists
                .iter()
                .find(|playlist| playlist.id == id)
                .map(|playlist| playlist.kind.clone())
                .ok_or(LibraryError::PlaylistNotFound)?
        };
        let songs = match kind {
            PlaylistKind::Directory { ref path } => self.scan_directory(Path::new(path)),
            PlaylistKind::Virtual => {
                let paths = {
                    let manifest = self
                        .manifest
                        .lock()
                        .expect("library manifest mutex poisoned");
                    manifest
                        .playlists
                        .iter()
                        .find(|playlist| playlist.id == id)
                        .expect("playlist checked above")
                        .songs
                        .iter()
                        .map(|song| PathBuf::from(&song.path))
                        .collect::<Vec<_>>()
                };
                paths
                    .into_iter()
                    .filter_map(|path| self.inspect_path(&path))
                    .collect()
            }
        };
        let mut manifest = self
            .manifest
            .lock()
            .expect("library manifest mutex poisoned");
        let playlist = manifest
            .playlists
            .iter_mut()
            .find(|playlist| playlist.id == id)
            .ok_or(LibraryError::PlaylistNotFound)?;
        playlist.songs = songs;
        let result = playlist.clone();
        self.save(&manifest)?;
        Ok(result)
    }

    fn validated_name(name: String) -> Result<String, LibraryError> {
        let name = name.trim().to_owned();
        if name.is_empty() {
            Err(LibraryError::EmptyPlaylistName)
        } else {
            Ok(name)
        }
    }

    fn scan_directory(&self, root: &Path) -> Vec<Song> {
        let mut songs = WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| entry.into_path())
            .filter(|path| Self::is_audio_path(path))
            .filter_map(|path| self.inspect_path(&path))
            .collect::<Vec<_>>();
        songs.sort_by(|left, right| left.path.cmp(&right.path));
        songs
    }

    fn inspect_path(&self, path: &Path) -> Option<Song> {
        let canonical = fs::canonicalize(path).ok()?;
        let MediaInspection {
            duration_ms,
            metadata,
        } = self.inspector.inspect(&canonical).ok()?;
        Some(Song {
            path: canonical.to_string_lossy().into_owned(),
            duration_ms,
            metadata,
        })
    }

    fn is_audio_path(path: &Path) -> bool {
        let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
            return false;
        };
        matches!(
            extension.to_ascii_lowercase().as_str(),
            "aac"
                | "aiff"
                | "alac"
                | "flac"
                | "m4a"
                | "mp3"
                | "mp4"
                | "ogg"
                | "opus"
                | "wav"
                | "webm"
                | "wma"
        )
    }

    fn save(&self, manifest: &LibraryManifest) -> Result<(), LibraryError> {
        if let Some(parent) = self.manifest_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = self.manifest_path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(manifest)?)?;
        fs::rename(temporary, &self.manifest_path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::player::{MediaInspection, PlayerError};

    struct FakeInspector;

    impl MediaInspector for FakeInspector {
        fn inspect(&self, path: &Path) -> Result<MediaInspection, PlayerError> {
            Ok(MediaInspection {
                duration_ms: 42,
                metadata: crate::player::metadata::TrackMetadata {
                    title: path
                        .file_stem()
                        .map(|name| name.to_string_lossy().into_owned()),
                    ..Default::default()
                },
            })
        }
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("mofu-library-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&path).expect("create test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn virtual_playlist_persists_and_deduplicates_files() {
        let dir = TestDirectory::new();
        let manifest = dir.0.join("library.json");
        let media = dir.0.join("song.mp3");
        fs::write(&media, b"fake").expect("write media");
        {
            let library = Library::open_with_inspector(manifest.clone(), Box::new(FakeInspector))
                .expect("open library");
            let playlist = library.create_virtual("Favorites".into()).expect("create");
            assert_eq!(
                library
                    .add_files(playlist.id, vec![media.clone(), media.clone()])
                    .expect("add"),
                1
            );
        }
        let reopened = Library::open_with_inspector(manifest, Box::new(FakeInspector))
            .expect("reopen library");
        assert_eq!(reopened.playlists().len(), 1);
        assert_eq!(reopened.playlists()[0].songs.len(), 1);
    }

    #[test]
    fn refresh_virtual_removes_missing_paths() {
        let dir = TestDirectory::new();
        let manifest = dir.0.join("library.json");
        let media = dir.0.join("song.flac");
        fs::write(&media, b"fake").expect("write media");
        let library =
            Library::open_with_inspector(manifest, Box::new(FakeInspector)).expect("open library");
        let playlist = library.create_virtual("List".into()).expect("create");
        library
            .add_files(playlist.id, vec![media.clone()])
            .expect("add");
        fs::remove_file(media).expect("remove media");
        let refreshed = library.refresh(playlist.id).expect("refresh");
        assert!(refreshed.songs.is_empty());
    }

    #[test]
    fn directory_playlist_scans_recursively_and_ignores_other_files() {
        let dir = TestDirectory::new();
        let nested = dir.0.join("album");
        fs::create_dir_all(&nested).expect("create nested directory");
        fs::write(dir.0.join("first.mp3"), b"fake").expect("write first media");
        fs::write(nested.join("second.flac"), b"fake").expect("write second media");
        fs::write(nested.join("notes.txt"), b"not media").expect("write text");
        let library =
            Library::open_with_inspector(dir.0.join("library.json"), Box::new(FakeInspector))
                .expect("open library");
        let playlist = library.add_directory(dir.0.clone()).expect("add directory");
        assert_eq!(playlist.songs.len(), 2);
        assert!(matches!(playlist.kind, PlaylistKind::Directory { .. }));
    }
}
