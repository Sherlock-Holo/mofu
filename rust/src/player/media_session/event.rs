//! Pure DTOs for OS media-session events and published state.

use std::path::Path;
use std::time::Duration;

use super::super::state::{PlaybackState, PlayerSnapshot};

/// Direction for relative seek requests from the OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekDirection {
    Forward,
    Backward,
}

impl SeekDirection {
    /// Signed millisecond delta for a relative seek of `amount`.
    pub fn delta_ms(self, amount: Duration) -> i64 {
        let ms = amount.as_millis().min(i64::MAX as u128) as i64;
        match self {
            Self::Forward => ms,
            Self::Backward => -ms,
        }
    }
}

/// Control event delivered by the OS media session.
#[derive(Debug, Clone, PartialEq)]
pub enum MediaSessionEvent {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Stop,
    Seek(SeekDirection),
    SeekBy(SeekDirection, Duration),
    SetPosition(Duration),
    SetVolume(f64),
    OpenUri(String),
    Raise,
    Quit,
}

/// Metadata published to the OS media session.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub cover_url: Option<String>,
    pub duration: Option<Duration>,
}

impl SessionMetadata {
    /// Build OS metadata from a player snapshot and optional `file://` cover URL.
    pub fn from_snapshot(snapshot: &PlayerSnapshot, cover_url: Option<String>) -> Self {
        let metadata = snapshot.metadata.as_ref();
        let title = metadata
            .and_then(|meta| meta.title.clone())
            .or_else(|| Self::fallback_title(snapshot));
        Self {
            title,
            artist: metadata.and_then(|meta| meta.artist.clone()),
            album: metadata.and_then(|meta| meta.album.clone()),
            cover_url,
            duration: (snapshot.duration > Duration::ZERO).then_some(snapshot.duration),
        }
    }

    fn fallback_title(snapshot: &PlayerSnapshot) -> Option<String> {
        let path = snapshot.queue.current_path()?;
        Path::new(path)
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
    }
}

/// Playback status published to the OS media session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPlayback {
    Stopped,
    Paused { progress: Option<Duration> },
    Playing { progress: Option<Duration> },
}

impl SessionPlayback {
    /// Map domain playback state onto an OS session status.
    pub fn from_player_state(state: PlaybackState, position: Duration) -> Self {
        let progress = Some(position);
        match state {
            PlaybackState::Playing => Self::Playing { progress },
            PlaybackState::Paused | PlaybackState::Ready => Self::Paused { progress },
            PlaybackState::Idle | PlaybackState::Ended => Self::Stopped,
        }
    }
}

/// Fire-and-forget command derived from a media-session event.
#[derive(Debug, Clone, PartialEq)]
pub enum MappedCommand {
    Play,
    Pause,
    Stop,
    Next,
    Previous,
    SeekRelative { delta_ms: i64 },
    Seek { position: Duration },
    SetVolume { volume: f32 },
    Ignore,
}

/// Default relative seek step for OS "seek" without an explicit amount.
pub const DEFAULT_SEEK_STEP_MS: i64 = 10_000;

impl MappedCommand {
    /// Map an OS media-session event onto a player command.
    ///
    /// `playing` is used for [`MediaSessionEvent::Toggle`].
    pub fn from_event(event: MediaSessionEvent, playing: bool) -> Self {
        match event {
            MediaSessionEvent::Play => Self::Play,
            MediaSessionEvent::Pause => Self::Pause,
            MediaSessionEvent::Toggle => {
                if playing {
                    Self::Pause
                } else {
                    Self::Play
                }
            }
            MediaSessionEvent::Next => Self::Next,
            MediaSessionEvent::Previous => Self::Previous,
            MediaSessionEvent::Stop => Self::Stop,
            MediaSessionEvent::Seek(direction) => Self::SeekRelative {
                delta_ms: direction.delta_ms(Duration::from_millis(DEFAULT_SEEK_STEP_MS as u64)),
            },
            MediaSessionEvent::SeekBy(direction, amount) => Self::SeekRelative {
                delta_ms: direction.delta_ms(amount),
            },
            MediaSessionEvent::SetPosition(position) => Self::Seek { position },
            MediaSessionEvent::SetVolume(volume) => Self::SetVolume {
                volume: volume.clamp(0.0, 1.0) as f32,
            },
            MediaSessionEvent::OpenUri(_) | MediaSessionEvent::Raise | MediaSessionEvent::Quit => {
                Self::Ignore
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_depends_on_playing_flag() {
        assert_eq!(
            MappedCommand::from_event(MediaSessionEvent::Toggle, true),
            MappedCommand::Pause
        );
        assert_eq!(
            MappedCommand::from_event(MediaSessionEvent::Toggle, false),
            MappedCommand::Play
        );
    }

    #[test]
    fn seek_uses_default_step() {
        assert_eq!(
            MappedCommand::from_event(MediaSessionEvent::Seek(SeekDirection::Forward), false),
            MappedCommand::SeekRelative {
                delta_ms: DEFAULT_SEEK_STEP_MS
            }
        );
        assert_eq!(
            MappedCommand::from_event(MediaSessionEvent::Seek(SeekDirection::Backward), false),
            MappedCommand::SeekRelative {
                delta_ms: -DEFAULT_SEEK_STEP_MS
            }
        );
    }

    #[test]
    fn seek_by_preserves_amount_and_sign() {
        let amount = Duration::from_secs(3);
        assert_eq!(
            MappedCommand::from_event(
                MediaSessionEvent::SeekBy(SeekDirection::Backward, amount),
                false
            ),
            MappedCommand::SeekRelative { delta_ms: -3_000 }
        );
    }

    #[test]
    fn volume_is_clamped() {
        assert_eq!(
            MappedCommand::from_event(MediaSessionEvent::SetVolume(1.5), false),
            MappedCommand::SetVolume { volume: 1.0 }
        );
        assert_eq!(
            MappedCommand::from_event(MediaSessionEvent::SetVolume(-0.2), false),
            MappedCommand::SetVolume { volume: 0.0 }
        );
    }

    #[test]
    fn ignored_events() {
        assert_eq!(
            MappedCommand::from_event(MediaSessionEvent::Raise, false),
            MappedCommand::Ignore
        );
        assert_eq!(
            MappedCommand::from_event(MediaSessionEvent::Quit, false),
            MappedCommand::Ignore
        );
        assert_eq!(
            MappedCommand::from_event(MediaSessionEvent::OpenUri("x".into()), false),
            MappedCommand::Ignore
        );
    }

    #[test]
    fn playback_maps_player_states() {
        let pos = Duration::from_secs(12);
        assert_eq!(
            SessionPlayback::from_player_state(PlaybackState::Playing, pos),
            SessionPlayback::Playing {
                progress: Some(pos)
            }
        );
        assert_eq!(
            SessionPlayback::from_player_state(PlaybackState::Ready, pos),
            SessionPlayback::Paused {
                progress: Some(pos)
            }
        );
        assert_eq!(
            SessionPlayback::from_player_state(PlaybackState::Idle, pos),
            SessionPlayback::Stopped
        );
    }
}
