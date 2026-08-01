use std::path::Path;

use super::decoder::DecoderSession;
use super::error::PlayerError;
use super::metadata::TrackMetadata;

/// Metadata-only result shared with the library scanner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaInspection {
    pub duration_ms: u64,
    pub metadata: TrackMetadata,
}

/// Media probe boundary used by the library without playback state.
pub trait MediaInspector: Send + Sync {
    fn inspect(&self, path: &Path) -> Result<MediaInspection, PlayerError>;
}

/// Symphonia-backed media inspector.
pub struct SymphoniaMediaInspector;

impl MediaInspector for SymphoniaMediaInspector {
    fn inspect(&self, path: &Path) -> Result<MediaInspection, PlayerError> {
        let session = DecoderSession::open(path)?;
        Ok(MediaInspection {
            duration_ms: session.duration_ms(),
            metadata: session.metadata().clone(),
        })
    }
}
