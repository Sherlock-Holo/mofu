use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::metadata::TrackMetadata;

/// High-level playback state exposed to callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PlaybackState {
    Idle = 0,
    Ready = 1,
    Playing = 2,
    Paused = 3,
    Ended = 4,
}

impl PlaybackState {
    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Ready,
            2 => Self::Playing,
            3 => Self::Paused,
            4 => Self::Ended,
            _ => Self::Idle,
        }
    }

    /// Whether the worker should actively decode and feed the output.
    pub(crate) fn is_playing(self) -> bool {
        matches!(self, Self::Playing)
    }
}

/// Snapshot of playback progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PositionSnapshot {
    pub position: Duration,
    pub duration: Duration,
    pub state: PlaybackState,
}

/// Shared progress/state readable from any thread without blocking the audio callback.
pub(crate) struct SharedProgress {
    position_ms: AtomicU64,
    duration_ms: AtomicU64,
    state: AtomicU8,
    metadata: Mutex<Option<Arc<TrackMetadata>>>,
}

impl SharedProgress {
    pub(crate) fn new() -> Self {
        Self {
            position_ms: AtomicU64::new(0),
            duration_ms: AtomicU64::new(0),
            state: AtomicU8::new(PlaybackState::Idle as u8),
            metadata: Mutex::new(None),
        }
    }

    pub(crate) fn set_state(&self, state: PlaybackState) {
        self.state.store(state as u8, Ordering::Release);
    }

    pub(crate) fn state(&self) -> PlaybackState {
        PlaybackState::from_u8(self.state.load(Ordering::Acquire))
    }

    pub(crate) fn set_duration_ms(&self, duration_ms: u64) {
        self.duration_ms.store(duration_ms, Ordering::Release);
    }

    pub(crate) fn set_position_ms(&self, position_ms: u64) {
        self.position_ms.store(position_ms, Ordering::Release);
    }

    pub(crate) fn reset_timing(&self) {
        self.position_ms.store(0, Ordering::Release);
        self.duration_ms.store(0, Ordering::Release);
    }

    pub(crate) fn set_metadata(&self, metadata: TrackMetadata) {
        *self.metadata.lock().expect("metadata mutex poisoned") = Some(Arc::new(metadata));
    }

    pub(crate) fn clear_metadata(&self) {
        *self.metadata.lock().expect("metadata mutex poisoned") = None;
    }

    pub(crate) fn metadata(&self) -> Option<Arc<TrackMetadata>> {
        self.metadata
            .lock()
            .expect("metadata mutex poisoned")
            .clone()
    }

    pub(crate) fn snapshot(&self) -> PositionSnapshot {
        PositionSnapshot {
            position: Duration::from_millis(self.position_ms.load(Ordering::Acquire)),
            duration: Duration::from_millis(self.duration_ms.load(Ordering::Acquire)),
            state: PlaybackState::from_u8(self.state.load(Ordering::Acquire)),
        }
    }
}
