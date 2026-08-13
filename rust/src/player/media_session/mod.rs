//! OS media session orchestration: event → player commands, snapshot → OS metadata.

mod backend;
mod cover_file;
mod event;
mod souvlaki_backend;

#[cfg(windows)]
mod windows;

use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use backend::MediaSessionBackend;
use cover_file::CoverFileCache;
use event::{MappedCommand, MediaSessionEvent, SessionMetadata, SessionPlayback};
use flume::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use souvlaki_backend::SouvlakiBackend;
use tracing::warn;

use super::command::{CommandReply, PlayerCommand};
use super::state::SharedProgress;

const SYNC_INTERVAL: Duration = Duration::from_millis(250);
const POSITION_PUBLISH_INTERVAL: Duration = Duration::from_secs(1);

/// Owns the OS media-session sync thread; soft-fails when the backend cannot start.
pub(crate) struct MediaSession {
    shutdown_tx: Sender<()>,
    sync_thread: Option<JoinHandle<()>>,
}

impl MediaSession {
    /// Attach to the OS media session. Returns `None` when the backend cannot start.
    pub(crate) fn try_attach(
        cmd_tx: Sender<PlayerCommand>,
        progress: Arc<SharedProgress>,
    ) -> Option<Self> {
        let mut backend = match SouvlakiBackend::new() {
            Ok(backend) => backend,
            Err(err) => {
                warn!(%err, "media session backend unavailable");
                return None;
            }
        };

        let event_tx = cmd_tx.clone();
        let event_progress = progress.clone();
        if let Err(err) = backend.attach(Box::new(move |event| {
            Self::handle_event(&event_tx, &event_progress, event);
        })) {
            warn!(%err, "media session attach failed");
            return None;
        }

        let (shutdown_tx, shutdown_rx) = flume::bounded(1);
        let sync_thread = thread::Builder::new()
            .name("mofu-media-session".into())
            .spawn(move || {
                Self::run_sync_loop(backend, progress, shutdown_rx);
            })
            .ok()?;

        Some(Self {
            shutdown_tx,
            sync_thread: Some(sync_thread),
        })
    }

    fn handle_event(
        cmd_tx: &Sender<PlayerCommand>,
        progress: &SharedProgress,
        event: MediaSessionEvent,
    ) {
        Self::dispatch_command(
            cmd_tx,
            MappedCommand::from_event(event, progress.state().is_playing()),
        );
    }

    fn dispatch_command(cmd_tx: &Sender<PlayerCommand>, command: MappedCommand) {
        match command {
            MappedCommand::Play => Self::fire(cmd_tx, |reply| PlayerCommand::Play { reply }),
            MappedCommand::Pause => Self::fire(cmd_tx, |reply| PlayerCommand::Pause { reply }),
            MappedCommand::Stop => Self::fire(cmd_tx, |reply| PlayerCommand::Stop { reply }),
            MappedCommand::Next => Self::fire(cmd_tx, |reply| PlayerCommand::Next { reply }),
            MappedCommand::Previous => {
                Self::fire(cmd_tx, |reply| PlayerCommand::Previous { reply })
            }
            MappedCommand::SeekRelative { delta_ms } => Self::fire(cmd_tx, |reply| {
                PlayerCommand::SeekRelative { delta_ms, reply }
            }),
            MappedCommand::Seek { position } => {
                Self::fire(cmd_tx, |reply| PlayerCommand::Seek { position, reply })
            }
            MappedCommand::SetVolume { volume } => {
                Self::fire(cmd_tx, |reply| PlayerCommand::SetVolume { volume, reply })
            }
            MappedCommand::Ignore => {}
        }
    }

    fn fire(cmd_tx: &Sender<PlayerCommand>, command: impl FnOnce(CommandReply) -> PlayerCommand) {
        let (reply_tx, _reply_rx) = flume::bounded(1);
        let _ = cmd_tx.try_send(command(reply_tx));
    }

    fn run_sync_loop(
        mut backend: SouvlakiBackend,
        progress: Arc<SharedProgress>,
        shutdown_rx: Receiver<()>,
    ) {
        let mut cover_cache = CoverFileCache::new();
        let mut meta_cursor = MetadataPublishCursor::new();
        let mut last_playback_kind: Option<PlaybackKind> = None;
        let mut last_volume: Option<f32> = None;
        let mut last_position_publish = Instant::now()
            .checked_sub(POSITION_PUBLISH_INTERVAL)
            .unwrap_or_else(Instant::now);

        loop {
            match shutdown_rx.try_recv() {
                Ok(()) | Err(TryRecvError::Disconnected) => break,
                Err(TryRecvError::Empty) => {}
            }

            #[cfg(windows)]
            windows::DummyWindow::pump_event_queue();

            let snapshot = progress.full_snapshot();
            let meta_arc = progress.metadata();
            let meta_ptr = meta_arc.as_ref().map(|meta| Arc::as_ptr(meta) as usize);
            let has_metadata = meta_arc.is_some();
            let metadata_changed = meta_cursor.source_changed(meta_ptr, has_metadata);

            let cover_url = if metadata_changed {
                match cover_cache.update(meta_arc.as_ref().and_then(|meta| meta.cover.as_deref())) {
                    Ok(url) => url,
                    Err(err) => {
                        warn!(%err, "media session cover cache update failed");
                        None
                    }
                }
            } else {
                meta_cursor.last_published().cover_url.clone()
            };

            let published = SessionMetadata::from_snapshot(&snapshot, cover_url);
            if published != *meta_cursor.last_published() {
                match backend.set_metadata(&published) {
                    Ok(()) => meta_cursor.note_publish_ok(published, meta_ptr, has_metadata),
                    Err(err) => {
                        // Keep source cursor unchanged so the next tick rewrites cover and retries.
                        warn!(%err, "media session set_metadata failed");
                    }
                }
            } else if metadata_changed {
                meta_cursor.note_source_seen(meta_ptr, has_metadata);
            }

            let playback = SessionPlayback::from_player_state(snapshot.state, snapshot.position);
            let kind = PlaybackKind::from(playback);
            let position_due = last_position_publish.elapsed() >= POSITION_PUBLISH_INTERVAL;
            let kind_changed = last_playback_kind != Some(kind);
            let should_publish_position = position_due
                && matches!(
                    playback,
                    SessionPlayback::Playing { .. } | SessionPlayback::Paused { .. }
                );
            if kind_changed || should_publish_position {
                if let Err(err) = backend.set_playback(playback) {
                    warn!(%err, "media session set_playback failed");
                } else {
                    last_playback_kind = Some(kind);
                    last_position_publish = Instant::now();
                }
            }

            if last_volume != Some(snapshot.volume) {
                if let Err(err) = backend.set_volume(f64::from(snapshot.volume)) {
                    warn!(%err, "media session set_volume failed");
                } else {
                    last_volume = Some(snapshot.volume);
                }
            }

            match shutdown_rx.recv_timeout(SYNC_INTERVAL) {
                Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {}
            }
        }

        let _ = backend.set_playback(SessionPlayback::Stopped);
        let _ = cover_cache.clear();
    }
}

impl Drop for MediaSession {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(());
        if let Some(thread) = self.sync_thread.take() {
            let _ = thread.join();
        }
    }
}

/// Tracks which metadata Arc was last observed / successfully published to the OS.
#[derive(Debug, Default)]
struct MetadataPublishCursor {
    last_meta_ptr: Option<usize>,
    last_had_metadata: bool,
    last_published: SessionMetadata,
}

impl MetadataPublishCursor {
    fn new() -> Self {
        Self::default()
    }

    fn last_published(&self) -> &SessionMetadata {
        &self.last_published
    }

    fn source_changed(&self, meta_ptr: Option<usize>, has_metadata: bool) -> bool {
        meta_ptr != self.last_meta_ptr || has_metadata != self.last_had_metadata
    }

    /// Advance after a successful OS publish.
    fn note_publish_ok(
        &mut self,
        published: SessionMetadata,
        meta_ptr: Option<usize>,
        has_metadata: bool,
    ) {
        self.last_published = published;
        self.note_source_seen(meta_ptr, has_metadata);
    }

    /// Advance the source cursor when content already matches the last publish.
    fn note_source_seen(&mut self, meta_ptr: Option<usize>, has_metadata: bool) {
        self.last_meta_ptr = meta_ptr;
        self.last_had_metadata = has_metadata;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlaybackKind {
    Stopped,
    Paused,
    Playing,
}

impl From<SessionPlayback> for PlaybackKind {
    fn from(playback: SessionPlayback) -> Self {
        match playback {
            SessionPlayback::Stopped => Self::Stopped,
            SessionPlayback::Paused { .. } => Self::Paused,
            SessionPlayback::Playing { .. } => Self::Playing,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::command::PlayerCommand;
    use super::super::state::SharedProgress;
    use super::*;

    #[test]
    fn try_attach_does_not_panic() {
        let (cmd_tx, _cmd_rx) = flume::bounded::<PlayerCommand>(8);
        let (progress, _notify) = SharedProgress::new();
        let progress = Arc::new(progress);
        // Soft-fail is fine (e.g. no session bus); must not panic.
        let _session = MediaSession::try_attach(cmd_tx, progress);
    }

    #[test]
    fn failed_publish_keeps_source_cursor_for_retry() {
        let mut cursor = MetadataPublishCursor::new();
        let meta_ptr = Some(0x_dead_beef);
        assert!(cursor.source_changed(meta_ptr, true));

        // Simulate failed publish: do not call note_publish_ok.
        assert!(cursor.source_changed(meta_ptr, true));

        cursor.note_publish_ok(
            SessionMetadata {
                title: Some("t".into()),
                ..SessionMetadata::default()
            },
            meta_ptr,
            true,
        );
        assert!(!cursor.source_changed(meta_ptr, true));
        assert_eq!(cursor.last_published().title.as_deref(), Some("t"));
    }

    #[test]
    fn identical_publish_still_advances_source_cursor() {
        let mut cursor = MetadataPublishCursor::new();
        let meta_ptr = Some(1);
        assert!(cursor.source_changed(meta_ptr, true));
        cursor.note_source_seen(meta_ptr, true);
        assert!(!cursor.source_changed(meta_ptr, true));
    }
}
