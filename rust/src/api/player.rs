use std::path::PathBuf;
use std::time::Duration;

use crate::player::metadata::{
    CoverArt as DomainCoverArt, Lyrics as DomainLyrics, LyricsContent as DomainLyricsContent,
    LyricsLine as DomainLyricsLine, LyricsSource as DomainLyricsSource,
    TrackMetadata as DomainTrackMetadata,
};
use crate::player::{
    PlaybackMode as DomainPlaybackMode, PlaybackState, Player as DomainPlayer,
    PlayerSnapshot as DomainPlayerSnapshot, QueuePosition as DomainQueuePosition,
};

/// FFI handle for a local music player instance.
#[flutter_rust_bridge::frb(opaque)]
pub struct Player {
    inner: DomainPlayer,
}

impl Player {
    /// Create a player and start its decode worker thread.
    #[flutter_rust_bridge::frb(sync)]
    pub fn new() -> Self {
        Self {
            inner: DomainPlayer::new(),
        }
    }

    /// Open a local audio file. Does not start playback.
    #[flutter_rust_bridge::frb(sync)]
    pub fn open(&self, path: String) -> Result<(), String> {
        self.inner.open(path.into()).map_err(|err| err.to_string())
    }

    /// Start or resume playback.
    #[flutter_rust_bridge::frb(sync)]
    pub fn play(&self) -> Result<(), String> {
        self.inner.play().map_err(|err| err.to_string())
    }

    /// Pause playback.
    #[flutter_rust_bridge::frb(sync)]
    pub fn pause(&self) -> Result<(), String> {
        self.inner.pause().map_err(|err| err.to_string())
    }

    /// Stop playback and close the current media.
    #[flutter_rust_bridge::frb(sync)]
    pub fn stop(&self) -> Result<(), String> {
        self.inner.stop().map_err(|err| err.to_string())
    }

    /// Seek to an absolute position in milliseconds.
    #[flutter_rust_bridge::frb(sync)]
    pub fn seek(&self, position_ms: u64) -> Result<(), String> {
        self.inner
            .seek(Duration::from_millis(position_ms))
            .map_err(|err| err.to_string())
    }

    /// Seek backward or forward from the current position.
    #[flutter_rust_bridge::frb(sync)]
    pub fn seek_relative(&self, delta_ms: i64) -> Result<(), String> {
        self.inner
            .seek_relative(delta_ms)
            .map_err(|err| err.to_string())
    }

    /// Replace the playback queue and optionally prepare/start one item.
    #[flutter_rust_bridge::frb(sync)]
    pub fn set_queue(
        &self,
        paths: Vec<String>,
        start_index: Option<u64>,
        autoplay: bool,
    ) -> Result<(), String> {
        let start_index = start_index
            .map(usize::try_from)
            .transpose()
            .map_err(|err| err.to_string())?;
        self.inner
            .set_queue(
                paths.into_iter().map(Into::into).collect(),
                start_index,
                autoplay,
            )
            .map_err(|err| err.to_string())
    }

    /// Select and start one queue item.
    #[flutter_rust_bridge::frb(sync)]
    pub fn play_index(&self, index: u64) -> Result<(), String> {
        let index = usize::try_from(index).map_err(|err| err.to_string())?;
        self.inner.play_index(index).map_err(|err| err.to_string())
    }

    /// Queue a track to play after the current one (up-next queue).
    #[flutter_rust_bridge::frb(sync)]
    pub fn enqueue_next(&self, path: String) -> Result<(), String> {
        self.inner
            .enqueue_next(path.into())
            .map_err(|err| err.to_string())
    }

    /// Start the next queue item when available.
    #[flutter_rust_bridge::frb(sync)]
    pub fn next(&self) -> Result<(), String> {
        self.inner.next().map_err(|err| err.to_string())
    }

    /// Start the previous queue item when available.
    #[flutter_rust_bridge::frb(sync)]
    pub fn previous(&self) -> Result<(), String> {
        self.inner.previous().map_err(|err| err.to_string())
    }

    /// Set output volume. Values outside 0–1 are clamped.
    #[flutter_rust_bridge::frb(sync)]
    pub fn set_volume(&self, volume: f64) -> Result<(), String> {
        self.inner
            .set_volume(volume as f32)
            .map_err(|err| err.to_string())
    }

    /// Set queue navigation behavior.
    #[flutter_rust_bridge::frb(sync)]
    pub fn set_playback_mode(&self, mode: PlaybackMode) -> Result<(), String> {
        self.inner
            .set_playback_mode(mode.into())
            .map_err(|err| err.to_string())
    }

    /// Unified playback, queue, volume, and metadata snapshot.
    #[flutter_rust_bridge::frb(sync)]
    pub fn snapshot(&self) -> PlayerSnapshot {
        self.inner.full_snapshot().into()
    }

    /// Current playback position in milliseconds.
    #[flutter_rust_bridge::frb(sync)]
    pub fn position_ms(&self) -> u64 {
        self.inner.position().as_millis() as u64
    }

    /// Track duration in milliseconds (0 if unknown).
    #[flutter_rust_bridge::frb(sync)]
    pub fn duration_ms(&self) -> u64 {
        self.inner.duration().as_millis() as u64
    }

    /// Current playback state.
    #[flutter_rust_bridge::frb(sync)]
    pub fn state(&self) -> PlayerState {
        self.inner.state().into()
    }

    /// Tags, cover, and lyrics for the open track (`None` if idle / not ready yet).
    #[flutter_rust_bridge::frb(sync)]
    pub fn metadata(&self) -> Option<TrackMetadata> {
        self.inner
            .metadata()
            .map(|meta| TrackMetadata::from(meta.as_ref()))
    }
}

/// Playback state for Flutter FFI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerState {
    Idle,
    Ready,
    Playing,
    Paused,
    Ended,
}

/// Queue navigation behavior for Flutter FFI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackMode {
    Sequential,
    Shuffle,
    RepeatOne,
    RepeatAll,
}

/// Whether a snapshot is a full authoritative payload or a progress cursor only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerSnapshotKind {
    /// Queue / metadata / cover / lyrics are complete and authoritative.
    Full,
    /// Only playback cursor fields changed; UI must retain heavy fields from the
    /// last Full snapshot for the same `current_path`.
    Progress,
}

impl From<PlaybackMode> for DomainPlaybackMode {
    fn from(value: PlaybackMode) -> Self {
        match value {
            PlaybackMode::Sequential => Self::Sequential,
            PlaybackMode::Shuffle => Self::Shuffle,
            PlaybackMode::RepeatOne => Self::RepeatOne,
            PlaybackMode::RepeatAll => Self::RepeatAll,
        }
    }
}

impl From<DomainPlaybackMode> for PlaybackMode {
    fn from(value: DomainPlaybackMode) -> Self {
        match value {
            DomainPlaybackMode::Sequential => Self::Sequential,
            DomainPlaybackMode::Shuffle => Self::Shuffle,
            DomainPlaybackMode::RepeatOne => Self::RepeatOne,
            DomainPlaybackMode::RepeatAll => Self::RepeatAll,
        }
    }
}

impl From<PlaybackState> for PlayerState {
    fn from(value: PlaybackState) -> Self {
        match value {
            PlaybackState::Idle => Self::Idle,
            PlaybackState::Ready => Self::Ready,
            PlaybackState::Playing => Self::Playing,
            PlaybackState::Paused => Self::Paused,
            PlaybackState::Ended => Self::Ended,
        }
    }
}

/// Cover art bytes for Flutter FFI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverArt {
    pub mime_type: Option<String>,
    pub data: Vec<u8>,
}

/// Lyrics source for Flutter FFI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LyricsSource {
    Embedded,
    ExternalFile,
}

/// One timed lyric line for Flutter FFI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricsLine {
    pub start_ms: u64,
    pub end_ms: Option<u64>,
    pub text: String,
}

/// Lyrics body shape for Flutter FFI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LyricsContentKind {
    Plain,
    Timed,
}

/// Lyrics body for Flutter FFI.
///
/// Flattened here so Dart generation does not require Freezed; the domain model keeps a real enum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricsContent {
    pub kind: LyricsContentKind,
    /// Set when [`LyricsContentKind::Plain`].
    pub text: Option<String>,
    /// Set when [`LyricsContentKind::Timed`].
    pub lines: Option<Vec<LyricsLine>>,
}

/// One lyrics track for Flutter FFI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lyrics {
    pub source: LyricsSource,
    pub language: Option<String>,
    pub description: Option<String>,
    pub content: LyricsContent,
}

/// Track tags / cover / lyrics for Flutter FFI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub composer: Option<String>,
    pub lyricist: Option<String>,
    pub performer: Option<String>,
    pub genre: Option<String>,
    pub comment: Option<String>,
    pub date: Option<String>,
    pub year: Option<u16>,
    pub track_number: Option<u64>,
    pub track_total: Option<u64>,
    pub disc_number: Option<u64>,
    pub disc_total: Option<u64>,
    pub cover: Option<CoverArt>,
    pub lyrics: Vec<Lyrics>,
}

/// One queue item in a unified player snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueItem {
    pub index: u64,
    pub path: String,
}

impl QueueItem {
    fn list_from(paths: Vec<PathBuf>) -> Vec<Self> {
        paths
            .into_iter()
            .enumerate()
            .map(|(index, path)| Self {
                index: index as u64,
                path: path.to_string_lossy().into_owned(),
            })
            .collect()
    }
}

/// Unified state for polling from Flutter.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerSnapshot {
    /// Full vs progress-only; Progress means UI retains heavy fields for the same path.
    pub kind: PlayerSnapshotKind,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub state: PlayerState,
    pub volume: f64,
    pub playback_mode: PlaybackMode,
    /// Index into `queue`; `None` when the current track comes from `up_next`.
    pub current_index: Option<u64>,
    /// Whether the current track comes from the up-next queue.
    pub current_from_up_next: bool,
    /// Path of the current track; `None` when idle.
    pub current_path: Option<String>,
    /// Resolved title for the current track (tag or basename).
    pub display_title: Option<String>,
    /// Active timed lyric line index for `position_ms`, when available.
    pub active_lyrics_line: Option<u64>,
    /// Active lyrics search/preview session. `None` when no session is open.
    pub lyrics_session: Option<LyricsSessionSnapshot>,
    /// Preview body to render when the session target is the current track.
    /// `None` means the UI should keep using official `metadata.lyrics`.
    pub effective_lyrics: Option<EffectiveLyrics>,
    /// Playlist that sourced the current main queue, when known.
    pub source_playlist_id: Option<u64>,
    /// Main queue in play order (shuffled when `playback_mode` is shuffle).
    pub queue: Vec<QueueItem>,
    /// Manually queued tracks, in play order.
    pub up_next: Vec<QueueItem>,
    pub metadata: Option<TrackMetadata>,
}

impl From<DomainCoverArt> for CoverArt {
    fn from(value: DomainCoverArt) -> Self {
        Self {
            mime_type: value.mime_type,
            data: value.data,
        }
    }
}

impl From<&DomainCoverArt> for CoverArt {
    fn from(value: &DomainCoverArt) -> Self {
        Self {
            mime_type: value.mime_type.clone(),
            data: value.data.clone(),
        }
    }
}

impl From<DomainLyricsSource> for LyricsSource {
    fn from(value: DomainLyricsSource) -> Self {
        match value {
            DomainLyricsSource::Embedded => Self::Embedded,
            DomainLyricsSource::ExternalFile => Self::ExternalFile,
        }
    }
}

impl From<&DomainLyricsContent> for LyricsContent {
    fn from(value: &DomainLyricsContent) -> Self {
        match value {
            DomainLyricsContent::Plain { text } => Self {
                kind: LyricsContentKind::Plain,
                text: Some(text.clone()),
                lines: None,
            },
            DomainLyricsContent::Timed { lines } => Self {
                kind: LyricsContentKind::Timed,
                text: None,
                lines: Some(lines.iter().map(LyricsLine::from).collect()),
            },
        }
    }
}

impl From<&DomainLyricsLine> for LyricsLine {
    fn from(value: &DomainLyricsLine) -> Self {
        Self {
            start_ms: value.start_ms,
            end_ms: value.end_ms,
            text: value.text.clone(),
        }
    }
}

impl From<&DomainLyrics> for Lyrics {
    fn from(value: &DomainLyrics) -> Self {
        Self {
            source: value.source.into(),
            language: value.language.clone(),
            description: value.description.clone(),
            content: (&value.content).into(),
        }
    }
}

impl From<&DomainTrackMetadata> for TrackMetadata {
    fn from(value: &DomainTrackMetadata) -> Self {
        Self {
            title: value.title.clone(),
            artist: value.artist.clone(),
            album: value.album.clone(),
            album_artist: value.album_artist.clone(),
            composer: value.composer.clone(),
            lyricist: value.lyricist.clone(),
            performer: value.performer.clone(),
            genre: value.genre.clone(),
            comment: value.comment.clone(),
            date: value.date.clone(),
            year: value.year,
            track_number: value.track_number,
            track_total: value.track_total,
            disc_number: value.disc_number,
            disc_total: value.disc_total,
            cover: value.cover.as_deref().map(CoverArt::from),
            lyrics: value.lyrics.iter().map(Lyrics::from).collect(),
        }
    }
}

impl From<DomainPlayerSnapshot> for PlayerSnapshot {
    fn from(value: DomainPlayerSnapshot) -> Self {
        let queue = value.queue;
        let current_index = match &queue.current {
            Some(DomainQueuePosition::Main(index)) => Some(*index as u64),
            _ => None,
        };
        let current_from_up_next =
            matches!(&queue.current, Some(DomainQueuePosition::Manual { .. }));
        let current_path = queue
            .current_path()
            .map(|path| path.to_string_lossy().into_owned());
        let position_ms = value.position.as_millis() as u64;
        let display_title = current_path.as_ref().map(|path| {
            DomainTrackMetadata::display_title(
                path,
                value
                    .metadata
                    .as_ref()
                    .and_then(|meta| meta.title.as_deref()),
            )
        });
        let active_lyrics_line = value
            .metadata
            .as_ref()
            .and_then(|meta| meta.active_lyrics_line_index(position_ms))
            .map(|index| index as u64);
        Self {
            kind: PlayerSnapshotKind::Full,
            position_ms,
            duration_ms: value.duration.as_millis() as u64,
            state: value.state.into(),
            volume: f64::from(value.volume),
            playback_mode: queue.mode.into(),
            current_index,
            current_from_up_next,
            current_path,
            display_title,
            active_lyrics_line,
            lyrics_session: None,
            effective_lyrics: None,
            source_playlist_id: None,
            queue: QueueItem::list_from(queue.main_paths),
            up_next: QueueItem::list_from(queue.up_next_paths),
            metadata: value
                .metadata
                .as_ref()
                .map(|meta| TrackMetadata::from(meta.as_ref())),
        }
    }
}

/// Session status for a lyrics search shown in the candidate panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LyricsSearchStatus {
    Searching,
    Ready,
    NoResults,
    Failed,
}

/// One lyrics candidate row. The lyric body stays in the Rust session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricsCandidateSnapshot {
    pub id: u64,
    pub provider: String,
    pub provider_label: String,
    pub title: String,
    pub artist: String,
    pub duration_ms: Option<u64>,
    pub synced: bool,
    pub instrumental: Option<bool>,
}

/// Active lyrics search/preview session for Flutter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricsSessionSnapshot {
    pub session_id: u64,
    pub target_path: String,
    pub status: LyricsSearchStatus,
    pub candidates: Vec<LyricsCandidateSnapshot>,
    pub preview_id: Option<u64>,
    pub error_message: Option<String>,
}

/// Preview lyrics the UI should render instead of official `metadata.lyrics`.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveLyrics {
    pub preview_id: u64,
    pub content: LyricsContent,
}
