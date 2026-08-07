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
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use command::CommandReply;
pub use command::PlayerCommand;
pub use error::PlayerError;
use flume::{RecvTimeoutError, SendTimeoutError, Sender};
pub use inspector::{MediaInspection, MediaInspector, SymphoniaMediaInspector};
use media_session::MediaSession;
use metadata::TrackMetadata;
pub use queue::{PlaybackMode, QueuePosition, QueueSnapshot};
pub use state::{PlaybackState, PlayerSnapshot, PositionSnapshot};
use state::{ProgressNotify, SharedProgress};
use worker::DecodeWorker;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// Handle for controlling background decode/playback.
///
/// Dropping the player sends [`PlayerCommand::Shutdown`] and joins the worker.
/// FFI exposes a separate opaque mirror in `crate::api::player`.
pub(crate) struct Player {
    cmd_tx: Mutex<Option<Sender<PlayerCommand>>>,
    progress: Arc<SharedProgress>,
    progress_notify: ProgressNotify,
    worker: Mutex<Option<JoinHandle<()>>>,
    media_session: Mutex<Option<MediaSession>>,
}

impl Player {
    /// Create a player and start its decode worker thread.
    pub fn new() -> Self {
        let (cmd_tx, cmd_rx) = flume::bounded::<PlayerCommand>(32);
        let (progress, progress_notify) = SharedProgress::new();
        let progress = Arc::new(progress);
        let worker = DecodeWorker::spawn(cmd_rx, progress.clone());
        let media_session = MediaSession::try_attach(cmd_tx.clone(), progress.clone());
        Self {
            cmd_tx: Mutex::new(Some(cmd_tx)),
            progress,
            progress_notify,
            worker: Mutex::new(Some(worker)),
            media_session: Mutex::new(media_session),
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

    /// Wait until a semantic player change is signaled, or `timeout` elapses.
    pub(crate) fn wait_for_progress(&self, timeout: Duration) {
        self.progress_notify.recv_timeout(timeout);
    }

    /// Wake the application watch loop without changing player state.
    pub(crate) fn wake_progress_watch(&self) {
        self.progress.notify_change();
    }

    /// Stop media-session and decode workers. Safe to call more than once.
    pub(crate) fn shutdown(&self) {
        self.media_session
            .lock()
            .expect("media session mutex")
            .take();
        if let Some(cmd_tx) = self.cmd_tx.lock().expect("command sender mutex").take() {
            let _ = cmd_tx.try_send(PlayerCommand::Shutdown);
        }
        if let Some(worker) = self.worker.lock().expect("worker mutex").take() {
            let _ = worker.join();
        }
    }

    /// Send a command and block until the worker replies on a bounded(1) channel.
    fn request(
        &self,
        command: impl FnOnce(CommandReply) -> PlayerCommand,
    ) -> Result<(), PlayerError> {
        let (reply_tx, reply_rx) = flume::bounded(1);
        let cmd_tx = self
            .cmd_tx
            .lock()
            .expect("command sender mutex")
            .clone()
            .ok_or(PlayerError::CommandChannelClosed)?;
        match cmd_tx.send_timeout(command(reply_tx), COMMAND_TIMEOUT) {
            Ok(()) => {}
            Err(SendTimeoutError::Disconnected(_)) => {
                return Err(PlayerError::CommandChannelClosed);
            }
            Err(SendTimeoutError::Timeout(_)) => return Err(PlayerError::CommandTimedOut),
        }
        match reply_rx.recv_timeout(COMMAND_TIMEOUT) {
            Ok(result) => result,
            Err(RecvTimeoutError::Disconnected) => Err(PlayerError::CommandChannelClosed),
            Err(RecvTimeoutError::Timeout) => Err(PlayerError::CommandTimedOut),
        }
    }
}

impl Default for Player {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_is_idempotent_and_closes_commands() {
        let player = Player::new();

        player.shutdown();
        player.shutdown();

        assert!(matches!(
            player.play(),
            Err(PlayerError::CommandChannelClosed)
        ));
    }
}
