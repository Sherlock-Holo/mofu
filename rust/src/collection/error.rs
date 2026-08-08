use std::io;

use thiserror::Error;
use toasty::Error as ToastyError;

/// Numeric song fields that can fail range conversion from the DB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum CatalogNumericField {
    #[error("duration_ms")]
    DurationMs,
    #[error("year")]
    Year,
    #[error("track_number")]
    TrackNumber,
    #[error("track_total")]
    TrackTotal,
    #[error("disc_number")]
    DiscNumber,
    #[error("disc_total")]
    DiscTotal,
}

impl CatalogNumericField {
    pub(crate) fn u64_from_i64(self, value: i64) -> Result<u64, CollectionError> {
        u64::try_from(value).map_err(|_| CatalogDataError::InvalidNumericField(self).into())
    }

    pub(crate) fn opt_u16_from_i64(
        self,
        value: Option<i64>,
    ) -> Result<Option<u16>, CollectionError> {
        match value {
            None => Ok(None),
            Some(raw) => u16::try_from(raw)
                .map(Some)
                .map_err(|_| CatalogDataError::InvalidNumericField(self).into()),
        }
    }

    pub(crate) fn opt_u64_from_i64(
        self,
        value: Option<i64>,
    ) -> Result<Option<u64>, CollectionError> {
        match value {
            None => Ok(None),
            Some(raw) => u64::try_from(raw)
                .map(Some)
                .map_err(|_| CatalogDataError::InvalidNumericField(self).into()),
        }
    }
}

/// Corrupt or unexpected values read from `catalog.db`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum CatalogDataError {
    #[error("unknown playback_mode value")]
    UnknownPlaybackMode,
    #[error("invalid volume value")]
    InvalidVolume,
    #[error("directory playlist is missing directory_path")]
    DirectoryPlaylistMissingPath,
    #[error("invalid {0} field")]
    InvalidNumericField(CatalogNumericField),
}

/// Errors from collection persistence and playlist operations.
#[derive(Debug, Error)]
pub enum CollectionError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("database error: {0}")]
    Database(#[from] ToastyError),
    #[error("invalid catalog data: {0}")]
    InvalidData(#[from] CatalogDataError),
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

impl CollectionError {
    /// Writer–writer / MVCC conflict that should be retried.
    pub(crate) fn is_serialization_failure(&self) -> bool {
        matches!(self, Self::Database(err) if err.is_serialization_failure())
    }
}
