use std::io;

use deadpool::managed::{BuildError, HookError, PoolError, TimeoutType};
use thiserror::Error;

/// Failure modes of the Turso connection pool itself (not backend/create errors).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum PoolFailure {
    #[error("timed out waiting for a pooled connection")]
    WaitTimeout,
    #[error("timed out creating a pooled connection")]
    CreateTimeout,
    #[error("timed out recycling a pooled connection")]
    RecycleTimeout,
    #[error("connection pool has been closed")]
    Closed,
    #[error("connection pool runtime was not specified")]
    NoRuntimeSpecified,
    #[error("connection pool post_create hook failed")]
    PostCreateHook,
}

/// Errors from collection persistence and playlist operations.
#[derive(Debug, Error)]
pub enum CollectionError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("database error: {0}")]
    Database(#[from] turso::Error),
    #[error("connection pool error: {0}")]
    Pool(PoolFailure),
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

impl From<PoolError<CollectionError>> for CollectionError {
    fn from(err: PoolError<CollectionError>) -> Self {
        match err {
            PoolError::Backend(err) => err,
            PoolError::Timeout(TimeoutType::Wait) => Self::Pool(PoolFailure::WaitTimeout),
            PoolError::Timeout(TimeoutType::Create) => Self::Pool(PoolFailure::CreateTimeout),
            PoolError::Timeout(TimeoutType::Recycle) => Self::Pool(PoolFailure::RecycleTimeout),
            PoolError::Closed => Self::Pool(PoolFailure::Closed),
            PoolError::NoRuntimeSpecified => Self::Pool(PoolFailure::NoRuntimeSpecified),
            PoolError::PostCreateHook(HookError::Backend(err)) => err,
            PoolError::PostCreateHook(HookError::Message(_)) => {
                Self::Pool(PoolFailure::PostCreateHook)
            }
        }
    }
}

impl From<BuildError> for CollectionError {
    fn from(err: BuildError) -> Self {
        match err {
            BuildError::NoRuntimeSpecified => Self::Pool(PoolFailure::NoRuntimeSpecified),
        }
    }
}

impl CollectionError {
    /// Whether the pooled connection is broken and should be discarded.
    ///
    /// Busy/BusySnapshot are lock contention, not a bad connection — those are
    /// retried on the same checkout. Only misuse / I/O trigger discard.
    pub(crate) fn should_discard_pooled_connection(&self) -> bool {
        match self {
            Self::Database(turso::Error::IoError(..)) => true,
            Self::Database(turso::Error::Misuse(message))
                if message.contains("inner connection must be set")
                    || message.contains("concurrent use forbidden") =>
            {
                true
            }
            _ => false,
        }
    }

    /// Writer–writer contention (`database is locked` / snapshot busy).
    pub(crate) fn is_database_busy(&self) -> bool {
        matches!(
            self,
            Self::Database(turso::Error::Busy(_)) | Self::Database(turso::Error::BusySnapshot(_))
        )
    }
}
