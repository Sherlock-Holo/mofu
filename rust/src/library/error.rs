use std::io;

use thiserror::Error;

/// Errors from library persistence and playlist operations.
#[derive(Debug, Error)]
pub enum LibraryError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("invalid library manifest: {0}")]
    Manifest(#[from] serde_json::Error),
    #[error("database error: {0}")]
    Database(#[from] turso::Error),
    #[error("application data directory is unavailable")]
    DataDirectoryUnavailable,
    #[error("application cache directory is unavailable")]
    CacheDirectoryUnavailable,
    #[error("playlist was not found")]
    PlaylistNotFound,
    #[error("operation is only valid for virtual playlists")]
    NotVirtualPlaylist,
    #[error("playlist name cannot be empty")]
    EmptyPlaylistName,
}

impl LibraryError {
    /// Whether the failure looks like a dead connection worth reconnecting once.
    pub(crate) fn is_connection_failure(&self) -> bool {
        match self {
            Self::Database(turso::Error::IoError(..)) => true,
            Self::Database(turso::Error::Misuse(message))
                if message.contains("inner connection must be set") =>
            {
                true
            }
            _ => false,
        }
    }
}
