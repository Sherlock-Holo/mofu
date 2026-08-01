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
pub(crate) mod metadata;
mod output;
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
use metadata::TrackMetadata;
use state::SharedProgress;
pub use state::{PlaybackState, PositionSnapshot};
use worker::DecodeWorker;

/// Handle for controlling background decode/playback.
///
/// Dropping the player sends [`PlayerCommand::Shutdown`] and joins the worker.
/// FFI exposes a separate opaque mirror in `crate::api::player`.
pub(crate) struct Player {
    cmd_tx: Sender<PlayerCommand>,
    progress: Arc<SharedProgress>,
    worker: Option<JoinHandle<()>>,
}

impl Player {
    /// Create a player and start its decode worker thread.
    pub fn new() -> Self {
        let (cmd_tx, cmd_rx) = crossbeam_channel::bounded::<PlayerCommand>(32);
        let progress = Arc::new(SharedProgress::new());
        let worker = DecodeWorker::spawn(cmd_rx, progress.clone());
        Self {
            cmd_tx,
            progress,
            worker: Some(worker),
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

    /// Current playback snapshot (position, duration, state).
    pub fn snapshot(&self) -> PositionSnapshot {
        self.progress.snapshot()
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
        let _ = self.cmd_tx.send(PlayerCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
