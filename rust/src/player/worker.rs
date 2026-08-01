use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError, TryRecvError};
use tracing::{error, warn};

use super::command::PlayerCommand;
use super::decoder::{DecodePacket, DecoderSession};
use super::error::PlayerError;
use super::output::{CpalOutput, WriteStatus};
use super::state::{PlaybackState, SharedProgress};

/// Orchestrates commands, decoding, and output.
///
/// Composition root for the decode thread: owns concrete session/output types and
/// connects them through shared DTOs (`AudioFormat`, `WriteStatus`, `PlayerCommand`).
pub(crate) struct DecodeWorker {
    cmd_rx: Receiver<PlayerCommand>,
    progress: Arc<SharedProgress>,
    session: Option<DecoderSession>,
    output: Option<CpalOutput>,
}

enum RecvCommand {
    None,
    Command(PlayerCommand),
    Disconnected,
}

impl DecodeWorker {
    pub(crate) fn spawn(
        cmd_rx: Receiver<PlayerCommand>,
        progress: Arc<SharedProgress>,
    ) -> thread::JoinHandle<()> {
        thread::Builder::new()
            .name("mofu-decode".into())
            .spawn(move || {
                Self {
                    cmd_rx,
                    progress,
                    session: None,
                    output: None,
                }
                .run();
            })
            .expect("failed to spawn decode worker")
    }

    fn run(mut self) {
        loop {
            match self.recv_command() {
                RecvCommand::Disconnected => return,
                RecvCommand::None => {}
                RecvCommand::Command(PlayerCommand::Shutdown) => return,
                RecvCommand::Command(cmd) => self.handle_command(cmd),
            }

            if !self.progress.state().is_playing() {
                continue;
            }

            // Drain leftover samples before decoding more so seek stays low-latency.
            if self.flush_pending_output() {
                continue;
            }

            self.decode_step();
        }
    }

    /// Wait/poll for the next command.
    ///
    /// Idle/paused: block until a command arrives. Playing with a full ring: short wait so
    /// seek/pause stay responsive. Playing with free ring capacity: non-blocking poll.
    fn recv_command(&self) -> RecvCommand {
        let playing = self.progress.state().is_playing();
        let ring_backlog = self.output.as_ref().is_some_and(CpalOutput::has_pending);

        if !playing {
            return match self.cmd_rx.recv() {
                Ok(cmd) => RecvCommand::Command(cmd),
                Err(_) => RecvCommand::Disconnected,
            };
        }

        if ring_backlog {
            return match self.cmd_rx.recv_timeout(Duration::from_millis(5)) {
                Ok(cmd) => RecvCommand::Command(cmd),
                Err(RecvTimeoutError::Timeout) => RecvCommand::None,
                Err(RecvTimeoutError::Disconnected) => RecvCommand::Disconnected,
            };
        }

        match self.cmd_rx.try_recv() {
            Ok(cmd) => RecvCommand::Command(cmd),
            Err(TryRecvError::Empty) => RecvCommand::None,
            Err(TryRecvError::Disconnected) => RecvCommand::Disconnected,
        }
    }

    fn handle_command(&mut self, cmd: PlayerCommand) {
        match cmd {
            PlayerCommand::Shutdown => unreachable!("shutdown is handled by the run loop"),
            PlayerCommand::Open { path, reply } => {
                let _ = reply.send(self.open_media(path));
            }
            PlayerCommand::Play { reply } => {
                if self.session.is_some() {
                    self.progress.set_state(PlaybackState::Playing);
                }
                let _ = reply.send(Ok(()));
            }
            PlayerCommand::Pause { reply } => {
                if self.session.is_some() && self.progress.state().is_playing() {
                    self.progress.set_state(PlaybackState::Paused);
                }
                let _ = reply.send(Ok(()));
            }
            PlayerCommand::Stop { reply } => {
                self.clear_media(PlaybackState::Idle);
                let _ = reply.send(Ok(()));
            }
            PlayerCommand::Seek { position, reply } => {
                let _ = reply.send(self.seek(position));
            }
        }
    }

    fn open_media(&mut self, open_path: PathBuf) -> Result<(), PlayerError> {
        self.output = None;
        self.session = None;
        self.progress.reset_timing();
        self.progress.clear_metadata();
        match DecoderSession::open(&open_path) {
            Ok(opened) => {
                self.progress.set_duration_ms(opened.duration_ms());
                self.progress.set_position_ms(0);
                self.progress.set_metadata(opened.metadata().clone());
                self.progress.set_state(PlaybackState::Ready);
                self.session = Some(opened);
                Ok(())
            }
            Err(err) => {
                error!(%err, "player open failed");
                self.progress.set_state(PlaybackState::Idle);
                Err(err)
            }
        }
    }

    fn clear_media(&mut self, state: PlaybackState) {
        self.output = None;
        self.session = None;
        self.progress.reset_timing();
        self.progress.clear_metadata();
        self.progress.set_state(state);
    }

    fn seek(&mut self, position: Duration) -> Result<(), PlayerError> {
        let Some(active) = self.session.as_mut() else {
            return Ok(());
        };
        if let Some(out) = self.output.as_mut() {
            out.clear_buffered();
        }
        match active.seek(position) {
            Ok(position_ms) => {
                self.progress.set_position_ms(position_ms);
                // Seeking out of Ended returns to a paused media, not idle.
                if self.progress.state() == PlaybackState::Ended {
                    self.progress.set_state(PlaybackState::Paused);
                }
                Ok(())
            }
            Err(err) => {
                warn!(%err, "player seek failed");
                Err(err)
            }
        }
    }

    /// Returns `true` when the ring still has pending samples and decode should wait.
    fn flush_pending_output(&mut self) -> bool {
        let Some(out) = self.output.as_mut() else {
            return false;
        };
        out.has_pending() && out.try_flush() == WriteStatus::Pending
    }

    fn decode_step(&mut self) {
        let Some(active) = self.session.as_mut() else {
            self.progress.set_state(PlaybackState::Idle);
            return;
        };

        let packet = match active.decode_next_packet() {
            Ok(packet) => packet,
            Err(err) => {
                error!(%err, "decode failed");
                self.progress.set_state(PlaybackState::Paused);
                return;
            }
        };

        match packet {
            DecodePacket::Samples {
                interleaved,
                position_ms,
            } => self.write_decoded_samples(interleaved, position_ms),
            DecodePacket::Ended => self.on_track_ended(),
        }
    }

    fn write_decoded_samples(&mut self, interleaved: Vec<f32>, position_ms: u64) {
        if self.output.is_none() {
            let format = self
                .session
                .as_ref()
                .expect("session present while writing samples")
                .pcm_format();
            match CpalOutput::try_open(format) {
                Ok(opened) => self.output = Some(opened),
                Err(err) => {
                    error!(%err, "audio output open failed");
                    self.progress.set_state(PlaybackState::Paused);
                    return;
                }
            }
        }

        match self
            .output
            .as_mut()
            .expect("output opened above")
            .write_interleaved(&interleaved)
        {
            Ok(WriteStatus::Drained | WriteStatus::Pending) => {
                self.progress.set_position_ms(position_ms);
            }
            Err(err) => {
                error!(%err, "audio write failed");
                self.progress.set_state(PlaybackState::Paused);
            }
        }
    }

    fn on_track_ended(&mut self) {
        if let Some(out) = self.output.as_mut() {
            out.clear_buffered();
        }
        if let Some(active) = self.session.as_ref()
            && active.duration_ms() > 0
        {
            self.progress.set_position_ms(active.duration_ms());
        }
        self.progress.set_state(PlaybackState::Ended);
    }
}
