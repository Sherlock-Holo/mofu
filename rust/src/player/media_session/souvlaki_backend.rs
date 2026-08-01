//! souvlaki-backed OS media session (MPRIS / SMTC / Now Playing).

use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection as SouvlakiSeekDirection,
};

use super::backend::{MediaSessionBackend, MediaSessionError};
use super::event::{MediaSessionEvent, SeekDirection, SessionMetadata, SessionPlayback};
#[cfg(windows)]
use super::windows::DummyWindow;

/// Concrete [`MediaSessionBackend`] using souvlaki.
pub struct SouvlakiBackend {
    controls: MediaControls,
    #[cfg(windows)]
    _dummy_window: DummyWindow,
}

impl SouvlakiBackend {
    pub fn new() -> Result<Self, MediaSessionError> {
        #[cfg(windows)]
        let dummy_window = DummyWindow::new().map_err(|err| {
            MediaSessionError::Backend(format!("create media session window: {err}"))
        })?;

        #[cfg(windows)]
        let hwnd = Some(dummy_window.hwnd());
        #[cfg(not(windows))]
        let hwnd = None;

        let config = PlatformConfig {
            dbus_name: "mofu",
            display_name: "Mofu",
            hwnd,
        };
        let controls = MediaControls::new(config)
            .map_err(|err| MediaSessionError::Backend(err.to_string()))?;

        Ok(Self {
            controls,
            #[cfg(windows)]
            _dummy_window: dummy_window,
        })
    }

    fn map_event(event: MediaControlEvent) -> MediaSessionEvent {
        match event {
            MediaControlEvent::Play => MediaSessionEvent::Play,
            MediaControlEvent::Pause => MediaSessionEvent::Pause,
            MediaControlEvent::Toggle => MediaSessionEvent::Toggle,
            MediaControlEvent::Next => MediaSessionEvent::Next,
            MediaControlEvent::Previous => MediaSessionEvent::Previous,
            MediaControlEvent::Stop => MediaSessionEvent::Stop,
            MediaControlEvent::Seek(direction) => {
                MediaSessionEvent::Seek(Self::map_seek_direction(direction))
            }
            MediaControlEvent::SeekBy(direction, amount) => {
                MediaSessionEvent::SeekBy(Self::map_seek_direction(direction), amount)
            }
            MediaControlEvent::SetPosition(MediaPosition(position)) => {
                MediaSessionEvent::SetPosition(position)
            }
            MediaControlEvent::SetVolume(volume) => MediaSessionEvent::SetVolume(volume),
            MediaControlEvent::OpenUri(uri) => MediaSessionEvent::OpenUri(uri),
            MediaControlEvent::Raise => MediaSessionEvent::Raise,
            MediaControlEvent::Quit => MediaSessionEvent::Quit,
        }
    }

    fn map_seek_direction(direction: SouvlakiSeekDirection) -> SeekDirection {
        match direction {
            SouvlakiSeekDirection::Forward => SeekDirection::Forward,
            SouvlakiSeekDirection::Backward => SeekDirection::Backward,
        }
    }
}

impl MediaSessionBackend for SouvlakiBackend {
    fn attach(
        &mut self,
        handler: Box<dyn Fn(MediaSessionEvent) + Send + 'static>,
    ) -> Result<(), MediaSessionError> {
        self.controls
            .attach(move |event| handler(Self::map_event(event)))
            .map_err(|err| MediaSessionError::Backend(err.to_string()))
    }

    fn set_metadata(&mut self, metadata: &SessionMetadata) -> Result<(), MediaSessionError> {
        self.controls
            .set_metadata(MediaMetadata {
                title: metadata.title.as_deref(),
                artist: metadata.artist.as_deref(),
                album: metadata.album.as_deref(),
                cover_url: metadata.cover_url.as_deref(),
                duration: metadata.duration,
            })
            .map_err(|err| MediaSessionError::Backend(err.to_string()))
    }

    fn set_playback(&mut self, playback: SessionPlayback) -> Result<(), MediaSessionError> {
        let playback = match playback {
            SessionPlayback::Stopped => MediaPlayback::Stopped,
            SessionPlayback::Paused { progress } => MediaPlayback::Paused {
                progress: progress.map(MediaPosition),
            },
            SessionPlayback::Playing { progress } => MediaPlayback::Playing {
                progress: progress.map(MediaPosition),
            },
        };
        self.controls
            .set_playback(playback)
            .map_err(|err| MediaSessionError::Backend(err.to_string()))
    }

    fn set_volume(&mut self, volume: f64) -> Result<(), MediaSessionError> {
        // set_volume exists only on the Linux MPRIS backends.
        #[cfg(all(
            unix,
            not(any(target_os = "macos", target_os = "ios", target_os = "android"))
        ))]
        {
            self.controls
                .set_volume(volume)
                .map_err(|err| MediaSessionError::Backend(err.to_string()))
        }
        #[cfg(not(all(
            unix,
            not(any(target_os = "macos", target_os = "ios", target_os = "android"))
        )))]
        {
            let _ = volume;
            Ok(())
        }
    }
}
