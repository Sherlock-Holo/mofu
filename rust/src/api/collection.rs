use std::path::PathBuf;

use crate::api::player::CoverArt;
use crate::api::player::PlaybackMode;
use crate::api::player::TrackMetadata;
use crate::collection::{
    Collection as DomainCollection, PlaybackSession as DomainPlaybackSession,
    Playlist as DomainPlaylist, PlaylistKind as DomainPlaylistKind, Song as DomainSong,
    StoredPlaybackMode,
};

/// FFI handle for the persistent local music collection.
#[flutter_rust_bridge::frb(opaque)]
pub struct Collection {
    inner: DomainCollection,
}

impl Collection {
    /// Open the collection in the platform application data directory.
    pub async fn new() -> Result<Self, String> {
        Ok(Self {
            inner: DomainCollection::open_default()
                .await
                .map_err(|err| err.to_string())?,
        })
    }

    /// Open the collection under an explicit data directory.
    ///
    /// Cover art still uses the platform XDG cache directory.
    pub async fn new_at(data_directory: String) -> Result<Self, String> {
        Ok(Self {
            inner: DomainCollection::open_at(PathBuf::from(data_directory))
                .await
                .map_err(|err| err.to_string())?,
        })
    }

    /// Return every directory and virtual playlist.
    ///
    /// Song rows omit cover bytes and lyrics so the first paint stays light; use
    /// [`Self::covers_for_paths`] to hydrate artwork afterward.
    pub async fn playlists(&self) -> Result<Vec<Playlist>, String> {
        self.inner
            .playlists()
            .await
            .map(|playlists| playlists.into_iter().map(Playlist::from).collect())
            .map_err(|err| err.to_string())
    }

    /// Load cover art for the given song paths; unknown / cover-less paths are skipped.
    ///
    /// Prefer chunking large path lists on the Dart side so each FFI payload stays bounded.
    pub async fn covers_for_paths(&self, paths: Vec<String>) -> Result<Vec<SongCover>, String> {
        self.inner
            .covers_for_paths(paths)
            .await
            .map(|covers| {
                covers
                    .into_iter()
                    .map(|(path, cover)| SongCover {
                        path,
                        cover: CoverArt::from(cover),
                    })
                    .collect()
            })
            .map_err(|err| err.to_string())
    }

    /// Create an empty user-managed playlist.
    pub async fn create_virtual(&self, name: String) -> Result<Playlist, String> {
        self.inner
            .create_virtual(name)
            .await
            .map(Playlist::from)
            .map_err(|err| err.to_string())
    }

    /// Recursively scan and persist a music directory.
    pub async fn add_directory(&self, path: String) -> Result<Playlist, String> {
        self.inner
            .add_directory(path.into())
            .await
            .map(Playlist::from)
            .map_err(|err| err.to_string())
    }

    /// Rename a virtual playlist.
    pub async fn rename_virtual(&self, id: u64, name: String) -> Result<(), String> {
        self.inner
            .rename_virtual(id, name)
            .await
            .map_err(|err| err.to_string())
    }

    /// Delete a virtual playlist.
    pub async fn delete_virtual(&self, id: u64) -> Result<(), String> {
        self.inner
            .delete_virtual(id)
            .await
            .map_err(|err| err.to_string())
    }

    /// Inspect and add multiple files to a virtual playlist.
    pub async fn add_files(&self, id: u64, paths: Vec<String>) -> Result<u64, String> {
        self.inner
            .add_files(id, paths.into_iter().map(PathBuf::from).collect())
            .await
            .map(|count| count as u64)
            .map_err(|err| err.to_string())
    }

    /// Import selected paths from one playlist into a virtual playlist.
    ///
    /// An empty path list imports every song in the source playlist.
    pub async fn import_from_playlist(
        &self,
        target_id: u64,
        source_id: u64,
        paths: Vec<String>,
    ) -> Result<u64, String> {
        self.inner
            .import_from_playlist(target_id, source_id, paths)
            .await
            .map(|count| count as u64)
            .map_err(|err| err.to_string())
    }

    /// Rescan a directory playlist or drop invalid virtual-list paths.
    pub async fn refresh(&self, id: u64) -> Result<Playlist, String> {
        self.inner
            .refresh(id)
            .await
            .map(Playlist::from)
            .map_err(|err| err.to_string())
    }

    /// Load the persisted playback mode (defaults to sequential).
    pub async fn playback_mode(&self) -> Result<PlaybackMode, String> {
        self.inner
            .playback_mode()
            .await
            .map(PlaybackMode::from)
            .map_err(|err| err.to_string())
    }

    /// Persist the playback mode for the next launch.
    pub async fn set_playback_mode(&self, mode: PlaybackMode) -> Result<(), String> {
        self.inner
            .set_playback_mode(StoredPlaybackMode::from(mode))
            .await
            .map_err(|err| err.to_string())
    }

    /// Load the persisted volume (defaults to `1.0`).
    pub async fn volume(&self) -> Result<f64, String> {
        self.inner.volume().await.map_err(|err| err.to_string())
    }

    /// Persist volume clamped to `[0, 1]`.
    pub async fn set_volume(&self, volume: f64) -> Result<(), String> {
        self.inner
            .set_volume(volume)
            .await
            .map_err(|err| err.to_string())
    }

    /// Load the persisted playback session, if any.
    pub async fn playback_session(&self) -> Result<Option<PlaybackSession>, String> {
        self.inner
            .playback_session()
            .await
            .map(|session| session.map(PlaybackSession::from))
            .map_err(|err| err.to_string())
    }

    /// Persist the current playback session for the next launch.
    pub async fn set_playback_session(&self, session: PlaybackSession) -> Result<(), String> {
        self.inner
            .set_playback_session(session.into())
            .await
            .map_err(|err| err.to_string())
    }

    /// Clear any persisted playback session.
    pub async fn clear_playback_session(&self) -> Result<(), String> {
        self.inner
            .clear_playback_session()
            .await
            .map_err(|err| err.to_string())
    }
}

/// Playlist category for Flutter FFI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistKind {
    Directory,
    Virtual,
}

impl From<PlaybackMode> for StoredPlaybackMode {
    fn from(mode: PlaybackMode) -> Self {
        match mode {
            PlaybackMode::Sequential => Self::Sequential,
            PlaybackMode::Shuffle => Self::Shuffle,
            PlaybackMode::RepeatOne => Self::RepeatOne,
            PlaybackMode::RepeatAll => Self::RepeatAll,
        }
    }
}

impl From<StoredPlaybackMode> for PlaybackMode {
    fn from(mode: StoredPlaybackMode) -> Self {
        match mode {
            StoredPlaybackMode::Sequential => Self::Sequential,
            StoredPlaybackMode::Shuffle => Self::Shuffle,
            StoredPlaybackMode::RepeatOne => Self::RepeatOne,
            StoredPlaybackMode::RepeatAll => Self::RepeatAll,
        }
    }
}

/// One parsed collection song.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Song {
    pub path: String,
    pub duration_ms: u64,
    /// Title tag with path basename fallback.
    pub display_title: String,
    pub metadata: TrackMetadata,
}

/// Cover art for a single collection path (async hydrate after lite playlists).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SongCover {
    pub path: String,
    pub cover: CoverArt,
}

/// Directory-backed or user-managed playlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playlist {
    pub id: u64,
    pub name: String,
    pub kind: PlaylistKind,
    pub directory_path: Option<String>,
    /// First non-empty song cover in list order.
    pub cover: Option<CoverArt>,
    pub songs: Vec<Song>,
}

/// Persisted playback resume point for Flutter FFI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackSession {
    pub playlist_id: u64,
    pub path: String,
    pub position_ms: u64,
}

impl From<DomainSong> for Song {
    fn from(value: DomainSong) -> Self {
        let display_title = value.display_title();
        Self {
            path: value.path,
            duration_ms: value.duration_ms,
            display_title,
            metadata: (&value.metadata).into(),
        }
    }
}

impl From<DomainPlaylist> for Playlist {
    fn from(value: DomainPlaylist) -> Self {
        let cover = value.cover().cloned().map(CoverArt::from);
        let (kind, directory_path) = match value.kind {
            DomainPlaylistKind::Directory { path } => (PlaylistKind::Directory, Some(path)),
            DomainPlaylistKind::Virtual => (PlaylistKind::Virtual, None),
        };
        Self {
            id: value.id,
            name: value.name,
            kind,
            directory_path,
            cover,
            songs: value.songs.into_iter().map(Song::from).collect(),
        }
    }
}

impl From<DomainPlaybackSession> for PlaybackSession {
    fn from(value: DomainPlaybackSession) -> Self {
        Self {
            playlist_id: value.playlist_id,
            path: value.path,
            position_ms: value.position_ms,
        }
    }
}

impl From<PlaybackSession> for DomainPlaybackSession {
    fn from(value: PlaybackSession) -> Self {
        Self {
            playlist_id: value.playlist_id,
            path: value.path,
            position_ms: value.position_ms,
        }
    }
}
