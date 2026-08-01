use std::io;

use thiserror::Error;

/// Errors from library persistence and playlist operations.
#[derive(Debug, Error)]
pub enum LibraryError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("invalid library manifest: {0}")]
    Manifest(#[from] serde_json::Error),
    #[error("application data directory is unavailable")]
    DataDirectoryUnavailable,
    #[error("playlist was not found")]
    PlaylistNotFound,
    #[error("operation is only valid for virtual playlists")]
    NotVirtualPlaylist,
    #[error("playlist name cannot be empty")]
    EmptyPlaylistName,
}
