//! Backend trait isolating OS media-session implementations.

use thiserror::Error;

use super::event::{MediaSessionEvent, SessionMetadata, SessionPlayback};

/// Failures from the OS media-session backend.
#[derive(Debug, Error)]
pub enum MediaSessionError {
    #[error("media session backend: {0}")]
    Backend(String),
}

/// OS media session surface (MPRIS / SMTC / Now Playing).
pub trait MediaSessionBackend: Send {
    /// Register the event handler that receives OS media-control events.
    fn attach(
        &mut self,
        handler: Box<dyn Fn(MediaSessionEvent) + Send + 'static>,
    ) -> Result<(), MediaSessionError>;

    /// Publish track metadata to the OS.
    fn set_metadata(&mut self, metadata: &SessionMetadata) -> Result<(), MediaSessionError>;

    /// Publish playback status / position to the OS.
    fn set_playback(&mut self, playback: SessionPlayback) -> Result<(), MediaSessionError>;

    /// Publish volume to the OS (needed for MPRIS after handling SetVolume).
    fn set_volume(&mut self, volume: f64) -> Result<(), MediaSessionError>;
}
