//! Local music player built on Symphonia decoding and CPAL output.
//!
//! Modules share pure DTOs (`AudioFormat`, track metadata, commands, domain errors). `worker` is
//! the composition root that wires concrete decoder/output types; backends map into domain
//! errors at their boundaries instead of leaking through shared traits. FFI mirrors live in
//! `crate::api::player` so Freezed / layout compromises stay out of the domain model.

mod command;
mod decoder;
mod error;
mod format;
mod inspector;
mod media_session;
pub mod metadata;
mod output;
mod queue;
mod state;
mod worker;

use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use command::CommandReply;
pub use command::PlayerCommand;
use crossbeam_channel::Sender;
pub use error::PlayerError;
pub use inspector::{MediaInspection, MediaInspector, SymphoniaMediaInspector};
use media_session::MediaSession;
use metadata::TrackMetadata;
pub use queue::{PlaybackMode, QueuePosition, QueueSnapshot};
use state::SharedProgress;
pub use state::{PlaybackState, PlayerSnapshot, PositionSnapshot};
use worker::DecodeWorker;

/// Handle for controlling background decode/playback.
///
/// Dropping the player sends [`PlayerCommand::Shutdown`] and joins the worker.
/// FFI exposes a separate opaque mirror in `crate::api::player`.
pub(crate) struct Player {
    cmd_tx: Sender<PlayerCommand>,
    progress: Arc<SharedProgress>,
    worker: Option<JoinHandle<()>>,
    media_session: Option<MediaSession>,
}

impl Player {
    /// Create a player and start its decode worker thread.
    pub fn new() -> Self {
        let (cmd_tx, cmd_rx) = crossbeam_channel::bounded::<PlayerCommand>(32);
        let progress = Arc::new(SharedProgress::new());
        let worker = DecodeWorker::spawn(cmd_rx, progress.clone());
        let media_session = MediaSession::try_attach(cmd_tx.clone(), progress.clone());
        Self {
            cmd_tx,
            progress,
            worker: Some(worker),
            media_session,
        }
    }

    /// Open a local media file. Playback stays stopped until [`Self::play`].
    pub fn open(&self, path: PathBuf) -> Result<(), PlayerError> {
        self.request(|reply| PlayerCommand::Open { path, reply })
    }

    /// Start or resume playback.
    pub fn play(&self) -> Result<(), PlayerError> {
        self.request(|reply| PlayerCommand::Play { reply })
    }

    /// Pause playback while keeping the current media open.
    pub fn pause(&self) -> Result<(), PlayerError> {
        self.request(|reply| PlayerCommand::Pause { reply })
    }

    /// Stop playback and close the current media.
    pub fn stop(&self) -> Result<(), PlayerError> {
        self.request(|reply| PlayerCommand::Stop { reply })
    }

    /// Seek to an absolute position from the start of the track.
    pub fn seek(&self, position: Duration) -> Result<(), PlayerError> {
        self.request(|reply| PlayerCommand::Seek { position, reply })
    }

    /// Seek relative to the current position.
    pub fn seek_relative(&self, delta_ms: i64) -> Result<(), PlayerError> {
        self.request(|reply| PlayerCommand::SeekRelative { delta_ms, reply })
    }

    /// Replace the queue and optionally prepare/start one entry.
    pub fn set_queue(
        &self,
        paths: Vec<PathBuf>,
        start_index: Option<usize>,
        autoplay: bool,
    ) -> Result<(), PlayerError> {
        self.request(|reply| PlayerCommand::SetQueue {
            paths,
            start_index,
            autoplay,
            reply,
        })
    }

    /// Select and start one queue entry.
    pub fn play_index(&self, index: usize) -> Result<(), PlayerError> {
        self.request(|reply| PlayerCommand::PlayIndex { index, reply })
    }

    /// Append a track to the up-next queue; it plays after the current track.
    pub fn enqueue_next(&self, path: PathBuf) -> Result<(), PlayerError> {
        self.request(|reply| PlayerCommand::EnqueueNext { path, reply })
    }

    /// Start the next queue entry when available.
    pub fn next(&self) -> Result<(), PlayerError> {
        self.request(|reply| PlayerCommand::Next { reply })
    }

    /// Start the previous queue entry when available.
    pub fn previous(&self) -> Result<(), PlayerError> {
        self.request(|reply| PlayerCommand::Previous { reply })
    }

    /// Set automatic queue navigation behavior.
    pub fn set_playback_mode(&self, mode: PlaybackMode) -> Result<(), PlayerError> {
        self.request(|reply| PlayerCommand::SetPlaybackMode { mode, reply })
    }

    /// Set output volume in the range 0–1. Out-of-range values are clamped.
    pub fn set_volume(&self, volume: f32) -> Result<(), PlayerError> {
        self.request(|reply| PlayerCommand::SetVolume { volume, reply })
    }

    /// Current playback snapshot (position, duration, state).
    pub fn snapshot(&self) -> PositionSnapshot {
        self.progress.snapshot()
    }

    /// Complete playback, queue, volume, and metadata snapshot.
    pub fn full_snapshot(&self) -> PlayerSnapshot {
        self.progress.full_snapshot()
    }

    /// Current position.
    pub fn position(&self) -> Duration {
        self.snapshot().position
    }

    /// Track duration when known.
    pub fn duration(&self) -> Duration {
        self.snapshot().duration
    }

    /// Current high-level playback state.
    pub fn state(&self) -> PlaybackState {
        self.snapshot().state
    }

    /// Tags / cover / lyrics for the open track, if any.
    pub(crate) fn metadata(&self) -> Option<Arc<TrackMetadata>> {
        self.progress.metadata()
    }

    /// Send a command and block until the worker replies on a bounded(1) channel.
    fn request(
        &self,
        command: impl FnOnce(CommandReply) -> PlayerCommand,
    ) -> Result<(), PlayerError> {
        let (reply_tx, reply_rx) = crossbeam_channel::bounded(1);
        self.cmd_tx
            .send(command(reply_tx))
            .map_err(|_| PlayerError::CommandChannelClosed)?;
        reply_rx
            .recv()
            .map_err(|_| PlayerError::CommandChannelClosed)?
    }
}

impl Default for Player {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // Tear down the OS session before shutting down the worker so late
        // media-key events do not race a closed command channel.
        self.media_session.take();
        let _ = self.cmd_tx.send(PlayerCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
