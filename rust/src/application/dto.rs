//! Pure application DTOs shared across the orchestration layer.

use std::path::PathBuf;
use std::sync::Arc;

use crate::collection::{CoverAnimationInBackground, Playlist, PlaylistId, PlaylistKind, Song};
use crate::player::metadata::{CoverArt, TrackMetadata};
use crate::player::{
    PlaybackMode, PlaybackState, PlayerSnapshot as DomainPlayerSnapshot, QueuePosition,
};

/// Lightweight library view for Flutter (may omit covers until hydrated).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LibrarySnapshot {
    pub playlists: Vec<PlaylistView>,
    pub covers_hydrated: bool,
}

/// Playlist projection with display helpers.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlaylistView {
    pub id: PlaylistId,
    pub name: String,
    pub kind: PlaylistKind,
    pub songs: Vec<SongView>,
    pub cover: Option<CoverArt>,
}

/// Song projection with a resolved display title.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SongView {
    pub path: String,
    pub duration_ms: u64,
    pub display_title: String,
    pub metadata: TrackMetadata,
}

/// Whether a published player view is authoritative or a progress cursor only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum PlayerViewKind {
    /// Queue / metadata / cover / lyrics are complete and authoritative.
    #[default]
    Full,
    /// Only playback cursor fields changed; UI must retain heavy fields from the
    /// last [`Self::Full`] for the same `current_path`.
    Progress,
}

/// Unified player view pushed to Flutter.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlayerView {
    pub kind: PlayerViewKind,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub state: PlaybackState,
    pub volume: f64,
    pub playback_mode: PlaybackMode,
    pub current_index: Option<u64>,
    pub current_from_up_next: bool,
    pub current_path: Option<String>,
    /// Valid main-queue path used for persistence while an up-next track is active.
    pub session_path: Option<String>,
    /// Position paired with `session_path`; an up-next fallback resumes at zero.
    pub session_position_ms: u64,
    pub display_title: Option<String>,
    pub active_lyrics_line: Option<u64>,
    pub source_playlist_id: Option<PlaylistId>,
    pub queue: Vec<QueueItemView>,
    pub up_next: Vec<QueueItemView>,
    pub metadata: Option<Arc<TrackMetadata>>,
}

impl PlayerView {
    /// Whether only playback cursor fields differ (safe to publish as [`PlayerViewKind::Progress`]).
    pub(crate) fn is_progress_only_update(&self, previous: &Self) -> bool {
        self.duration_ms == previous.duration_ms
            && self.state == previous.state
            && self.volume == previous.volume
            && self.playback_mode == previous.playback_mode
            && self.current_index == previous.current_index
            && self.current_from_up_next == previous.current_from_up_next
            && self.current_path == previous.current_path
            && self.session_path == previous.session_path
            && self.display_title == previous.display_title
            && self.source_playlist_id == previous.source_playlist_id
            && self.queue == previous.queue
            && self.up_next == previous.up_next
            && self.metadata_ptr_eq(previous)
            && (self.position_ms != previous.position_ms
                || self.active_lyrics_line != previous.active_lyrics_line
                || self.session_position_ms != previous.session_position_ms)
    }

    fn metadata_ptr_eq(&self, other: &Self) -> bool {
        match (&self.metadata, &other.metadata) {
            (None, None) => true,
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }

    /// Build a [`PlayerViewKind::Progress`] fan-out: omit cover, lyrics, and queues.
    pub(crate) fn as_progress_tick(&self) -> Self {
        Self {
            kind: PlayerViewKind::Progress,
            position_ms: self.position_ms,
            duration_ms: self.duration_ms,
            state: self.state,
            volume: self.volume,
            playback_mode: self.playback_mode,
            current_index: self.current_index,
            current_from_up_next: self.current_from_up_next,
            current_path: self.current_path.clone(),
            session_path: self.session_path.clone(),
            session_position_ms: self.session_position_ms,
            display_title: self.display_title.clone(),
            active_lyrics_line: self.active_lyrics_line,
            source_playlist_id: self.source_playlist_id,
            queue: Vec::new(),
            up_next: Vec::new(),
            metadata: self
                .metadata
                .as_ref()
                .map(|meta| Arc::new(meta.without_heavy_fields())),
        }
    }

    pub(crate) fn from_domain(
        snapshot: DomainPlayerSnapshot,
        source_playlist_id: Option<PlaylistId>,
    ) -> Self {
        let current_index = match &snapshot.queue.current {
            Some(QueuePosition::Main(index)) => Some(*index as u64),
            _ => None,
        };
        let current_from_up_next =
            matches!(&snapshot.queue.current, Some(QueuePosition::Manual { .. }));
        let current_path = snapshot
            .queue
            .current_path()
            .map(|path| path.to_string_lossy().into_owned());
        let position_ms = snapshot.position.as_millis() as u64;
        let (session_path, session_position_ms) = match &snapshot.queue.current {
            Some(QueuePosition::Main(index)) => (
                snapshot
                    .queue
                    .main_paths
                    .get(*index)
                    .map(|path| path.to_string_lossy().into_owned()),
                position_ms,
            ),
            Some(QueuePosition::Manual {
                return_index: Some(index),
                ..
            }) => (
                snapshot
                    .queue
                    .main_paths
                    .get(*index)
                    .map(|path| path.to_string_lossy().into_owned()),
                0,
            ),
            Some(QueuePosition::Manual {
                return_index: None, ..
            })
            | None => (None, 0),
        };
        let display_title = current_path.as_ref().map(|path| {
            TrackMetadata::display_title(
                path,
                snapshot
                    .metadata
                    .as_ref()
                    .and_then(|meta| meta.title.as_deref()),
            )
        });
        let active_lyrics_line = snapshot
            .metadata
            .as_ref()
            .and_then(|meta| meta.active_lyrics_line_index(position_ms))
            .map(|index| index as u64);

        Self {
            kind: PlayerViewKind::Full,
            position_ms,
            duration_ms: snapshot.duration.as_millis() as u64,
            state: snapshot.state,
            volume: f64::from(snapshot.volume),
            playback_mode: snapshot.queue.mode,
            current_index,
            current_from_up_next,
            current_path,
            session_path,
            session_position_ms,
            display_title,
            active_lyrics_line,
            source_playlist_id,
            queue: QueueItemView::from_paths(snapshot.queue.main_paths),
            up_next: QueueItemView::from_paths(snapshot.queue.up_next_paths),
            metadata: snapshot.metadata,
        }
    }
}

/// Queue item projection with a path-derived display title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QueueItemView {
    pub index: u64,
    pub path: String,
    pub display_title: String,
}

/// One-shot bootstrap payload before streams take over.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BootstrapSnapshot {
    pub library: LibrarySnapshot,
    pub player: PlayerView,
    pub selected_playlist_id: Option<PlaylistId>,
    pub cover_animation_in_background: CoverAnimationInBackground,
}

impl LibrarySnapshot {
    pub(crate) fn from_playlists(playlists: Vec<Playlist>, covers_hydrated: bool) -> Self {
        Self {
            playlists: playlists.into_iter().map(PlaylistView::from).collect(),
            covers_hydrated,
        }
    }
}

impl From<Playlist> for PlaylistView {
    fn from(playlist: Playlist) -> Self {
        let cover = playlist.cover().cloned();
        Self {
            id: playlist.id,
            name: playlist.name,
            kind: playlist.kind,
            songs: playlist.songs.into_iter().map(SongView::from).collect(),
            cover,
        }
    }
}

impl From<Song> for SongView {
    fn from(song: Song) -> Self {
        let display_title = song.display_title();
        Self {
            path: song.path,
            duration_ms: song.duration_ms,
            display_title,
            metadata: song.metadata,
        }
    }
}

impl QueueItemView {
    fn from_paths(paths: Vec<PathBuf>) -> Vec<Self> {
        paths
            .into_iter()
            .enumerate()
            .map(|(index, path)| {
                let path = path.to_string_lossy().into_owned();
                let display_title = TrackMetadata::display_title(&path, None);
                Self {
                    index: index as u64,
                    path,
                    display_title,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::player::{PlayerSnapshot, QueueSnapshot};

    #[test]
    fn manual_track_persists_main_queue_anchor() {
        let snapshot = PlayerSnapshot {
            position: Duration::from_secs(12),
            duration: Duration::from_secs(30),
            state: PlaybackState::Playing,
            volume: 1.0,
            queue: QueueSnapshot {
                main_paths: vec![PathBuf::from("/music/main.mp3")],
                up_next_paths: Vec::new(),
                current: Some(QueuePosition::Manual {
                    path: PathBuf::from("/other/manual.mp3"),
                    return_index: Some(0),
                }),
                mode: PlaybackMode::Sequential,
            },
            metadata: None,
        };

        let view = PlayerView::from_domain(snapshot, Some(7));

        assert_eq!(view.kind, PlayerViewKind::Full);
        assert_eq!(view.current_path.as_deref(), Some("/other/manual.mp3"));
        assert_eq!(view.session_path.as_deref(), Some("/music/main.mp3"));
        assert_eq!(view.session_position_ms, 0);
    }

    #[test]
    fn progress_tick_marks_kind_and_strips_heavy_fields() {
        let meta = Arc::new(TrackMetadata {
            title: Some("t".into()),
            cover: Some(CoverArt {
                mime_type: Some("image/jpeg".into()),
                data: vec![1, 2, 3, 4],
            }),
            lyrics: vec![],
            ..TrackMetadata::default()
        });
        let previous = PlayerView {
            kind: PlayerViewKind::Full,
            position_ms: 1000,
            duration_ms: 10_000,
            state: PlaybackState::Playing,
            volume: 1.0,
            playback_mode: PlaybackMode::Sequential,
            current_index: Some(0),
            current_from_up_next: false,
            current_path: Some("/a.mp3".into()),
            session_path: Some("/a.mp3".into()),
            session_position_ms: 1000,
            display_title: Some("t".into()),
            active_lyrics_line: None,
            source_playlist_id: None,
            queue: vec![QueueItemView {
                index: 0,
                path: "/a.mp3".into(),
                display_title: "a.mp3".into(),
            }],
            up_next: Vec::new(),
            metadata: Some(meta.clone()),
        };
        let mut next = previous.clone();
        next.position_ms = 1100;
        next.session_position_ms = 1100;

        assert!(next.is_progress_only_update(&previous));
        let lite = next.as_progress_tick();
        assert_eq!(lite.kind, PlayerViewKind::Progress);
        let lite_meta = lite.metadata.as_ref().expect("metadata");
        assert!(lite_meta.cover.is_none());
        assert!(lite_meta.lyrics.is_empty());
        assert_eq!(lite_meta.title.as_deref(), Some("t"));
        assert_eq!(lite.position_ms, 1100);
        assert!(lite.queue.is_empty());
        assert!(lite.up_next.is_empty());
    }
}
