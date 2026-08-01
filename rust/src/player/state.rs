use std::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::metadata::TrackMetadata;
use super::queue::{PlaybackMode, QueueSnapshot};

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

/// Complete player state returned as one coherent FFI-friendly snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerSnapshot {
    pub position: Duration,
    pub duration: Duration,
    pub state: PlaybackState,
    pub volume: f32,
    pub queue: QueueSnapshot,
    pub metadata: Option<TrackMetadata>,
}

/// Atomic gain read directly by the realtime audio callback.
pub(crate) struct SharedVolume {
    bits: AtomicU32,
}

impl SharedVolume {
    pub(crate) fn new() -> Self {
        Self {
            bits: AtomicU32::new(1.0_f32.to_bits()),
        }
    }

    pub(crate) fn set(&self, volume: f32) {
        let volume = if volume.is_finite() {
            volume.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.bits.store(volume.to_bits(), Ordering::Release);
    }

    pub(crate) fn get(&self) -> f32 {
        f32::from_bits(self.bits.load(Ordering::Acquire))
    }
}

/// Shared progress/state readable from any thread without blocking the audio callback.
pub(crate) struct SharedProgress {
    position_ms: AtomicU64,
    duration_ms: AtomicU64,
    state: AtomicU8,
    metadata: Mutex<Option<Arc<TrackMetadata>>>,
    queue: Mutex<QueueSnapshot>,
    volume: Arc<SharedVolume>,
}

impl SharedProgress {
    pub(crate) fn new() -> Self {
        Self {
            position_ms: AtomicU64::new(0),
            duration_ms: AtomicU64::new(0),
            state: AtomicU8::new(PlaybackState::Idle as u8),
            metadata: Mutex::new(None),
            queue: Mutex::new(QueueSnapshot {
                paths: Vec::new(),
                current_index: None,
                mode: PlaybackMode::Sequential,
            }),
            volume: Arc::new(SharedVolume::new()),
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

    pub(crate) fn set_queue(&self, queue: QueueSnapshot) {
        *self.queue.lock().expect("queue mutex poisoned") = queue;
    }

    pub(crate) fn queue(&self) -> QueueSnapshot {
        self.queue.lock().expect("queue mutex poisoned").clone()
    }

    pub(crate) fn set_volume(&self, volume: f32) {
        self.volume.set(volume);
    }

    pub(crate) fn volume(&self) -> f32 {
        self.volume.get()
    }

    pub(crate) fn shared_volume(&self) -> Arc<SharedVolume> {
        self.volume.clone()
    }

    pub(crate) fn snapshot(&self) -> PositionSnapshot {
        PositionSnapshot {
            position: Duration::from_millis(self.position_ms.load(Ordering::Acquire)),
            duration: Duration::from_millis(self.duration_ms.load(Ordering::Acquire)),
            state: PlaybackState::from_u8(self.state.load(Ordering::Acquire)),
        }
    }

    pub(crate) fn full_snapshot(&self) -> PlayerSnapshot {
        let position = self.snapshot();
        PlayerSnapshot {
            position: position.position,
            duration: position.duration,
            state: position.state,
            volume: self.volume(),
            queue: self.queue(),
            metadata: self.metadata().map(|metadata| metadata.as_ref().clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_is_clamped() {
        let volume = SharedVolume::new();
        volume.set(-0.25);
        assert_eq!(volume.get(), 0.0);
        volume.set(1.25);
        assert_eq!(volume.get(), 1.0);
        volume.set(0.4);
        assert!((volume.get() - 0.4).abs() < f32::EPSILON);
        volume.set(f32::NAN);
        assert_eq!(volume.get(), 0.0);
    }
}
