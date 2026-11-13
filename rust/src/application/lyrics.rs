//! In-memory lyrics search session, generation guards, and effective-lyrics resolution.
//!
//! Confirmed persistence lives in [`super::lyrics_store`]. Production search uses
//! [`super::lyrics_providers::RemoteLyricsSource`] (LRCLIB then Megalobiz) over reqwest.
//! Tests may still inject fixtures or mock HTTP. Confirm never issues another HTTP request.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::dto::PlayerView;
use super::lyrics_providers::RemoteLyricsSource;
use tracing::debug;

use crate::player::metadata::{LyricsContent, TrackMetadata};

/// Identifies one active (or just-replaced) lyrics search session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct LyricsSessionId(u64);

impl LyricsSessionId {
    pub(crate) fn from_raw(id: u64) -> Self {
        Self(id)
    }

    pub(crate) fn get(self) -> u64 {
        self.0
    }
}

/// Identifies one candidate inside a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct LyricsCandidateId(u64);

impl LyricsCandidateId {
    pub(crate) fn from_raw(id: u64) -> Self {
        Self(id)
    }

    pub(crate) fn get(self) -> u64 {
        self.0
    }
}

/// Monotonic search generation used to drop stale provider results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct LyricsSearchGeneration(u64);

impl LyricsSearchGeneration {
    pub(crate) fn get(self) -> u64 {
        self.0
    }
}

/// Where a candidate was produced. Fixture stands in until HTTP providers land.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LyricsProviderKind {
    Fixture,
    Lrclib,
    Megalobiz,
}

impl LyricsProviderKind {
    const ALL: [Self; 3] = [Self::Fixture, Self::Lrclib, Self::Megalobiz];

    pub(crate) fn as_str(self) -> &'static str {
        const LABELS: [&str; 3] = ["fixture", "lrclib", "megalobiz"];
        let index = Self::ALL
            .iter()
            .position(|kind| *kind == self)
            .expect("closed lyrics provider set");
        LABELS[index]
    }
}

/// Visible search/session phase for the UI snapshot.
#[flutter_rust_bridge::frb(ignore)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LyricsSearchStatus {
    Searching,
    Ready,
    NoResults,
    Failed,
}

/// Failures from session commands (not network errors, which become [`LyricsSearchStatus::Failed`]).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum LyricsSessionError {
    #[error("lyrics session is not active")]
    SessionNotFound,
    #[error("lyrics candidate is not in this session")]
    CandidateNotFound,
    #[error("lyrics session is still searching")]
    StillSearching,
    #[error("play-and-preview path does not match the session target")]
    PlayPathMismatch,
    #[error("lyrics confirm was abandoned")]
    ConfirmAbandoned,
}

/// Query handed to a search source. Tags may be incomplete when the path is unknown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LyricsSearchQuery {
    pub path: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
}

/// One in-memory candidate, including the body used for preview (never persisted here).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LyricsCandidate {
    pub id: LyricsCandidateId,
    pub provider: LyricsProviderKind,
    pub title: String,
    pub artist: String,
    pub duration_ms: Option<u64>,
    pub synced: bool,
    pub instrumental: Option<bool>,
    pub body: LyricsContent,
    /// Provider-specific identifier, persisted as provenance (not the lyric body).
    pub source_id: Option<String>,
    /// Original LRC text when the provider supplied it; otherwise [`LyricsContent::to_lrc_text`].
    pub raw_text: Option<String>,
}

impl LyricsCandidate {
    pub(crate) fn text_for_save(&self) -> String {
        self.raw_text
            .clone()
            .unwrap_or_else(|| self.body.to_lrc_text())
    }
}

/// UI-facing candidate row. The lyric body stays in the session, not this view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LyricsCandidateView {
    pub id: LyricsCandidateId,
    pub provider: LyricsProviderKind,
    pub provider_label: &'static str,
    pub title: String,
    pub artist: String,
    pub duration_ms: Option<u64>,
    pub synced: bool,
    pub instrumental: Option<bool>,
}

impl LyricsCandidateView {
    fn from_candidate(candidate: &LyricsCandidate) -> Self {
        Self {
            id: candidate.id,
            provider: candidate.provider,
            provider_label: candidate.provider.as_str(),
            title: candidate.title.clone(),
            artist: candidate.artist.clone(),
            duration_ms: candidate.duration_ms,
            synced: candidate.synced,
            instrumental: candidate.instrumental,
        }
    }
}

/// Published session state. Cheap to retain across progress ticks via [`Arc`] identity.
#[derive(Debug, Clone, PartialEq, Eq)]
#[flutter_rust_bridge::frb(ignore)]
pub(crate) struct LyricsSessionSnapshot {
    pub session_id: LyricsSessionId,
    pub generation: LyricsSearchGeneration,
    pub target_path: String,
    pub status: LyricsSearchStatus,
    pub candidates: Vec<LyricsCandidateView>,
    pub preview_id: Option<LyricsCandidateId>,
    pub error_message: Option<String>,
}

/// Lyrics the UI should render instead of official `metadata.lyrics`.
///
/// `None` on [`PlayerView::effective_lyrics`] means "use official tracks".
#[derive(Debug, Clone, PartialEq, Eq)]
#[flutter_rust_bridge::frb(ignore)]
pub(crate) struct EffectiveLyrics {
    pub preview_id: LyricsCandidateId,
    pub content: LyricsContent,
}

/// Token returned by [`LyricsSessionController::begin_search`] to complete or drop a fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LyricsSearchHandle {
    pub session_id: LyricsSessionId,
    pub generation: LyricsSearchGeneration,
}

/// Outcome of a provider fetch, applied only if the handle is still current.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LyricsSearchOutcome {
    Ready(Vec<LyricsCandidate>),
    Failed(String),
}

impl LyricsSearchOutcome {
    pub(crate) fn from_result(result: Result<Vec<LyricsCandidate>, String>) -> Self {
        match result {
            Ok(candidates) => Self::Ready(candidates),
            Err(message) => Self::Failed(message),
        }
    }
}

/// Owns the single active lyrics session and generation counter.
#[derive(Debug, Default)]
pub(crate) struct LyricsSessionController {
    next_session_id: u64,
    next_generation: u64,
    next_candidate_id: u64,
    session: Option<LyricsSession>,
    last_observed_path: Option<String>,
    published: Option<Arc<LyricsSessionSnapshot>>,
    preview_lyrics: Option<Arc<EffectiveLyrics>>,
    /// Set while a confirm persist is in flight. Cancel and a new search abort it;
    /// a normal track-change clear does not.
    confirm_abort: Option<InFlightConfirm>,
}

#[derive(Clone)]
struct InFlightConfirm {
    session_id: LyricsSessionId,
    abort: Arc<AtomicBool>,
}

impl std::fmt::Debug for InFlightConfirm {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InFlightConfirm")
            .field("session_id", &self.session_id.get())
            .field("aborted", &self.abort.load(Ordering::Relaxed))
            .finish()
    }
}

#[derive(Debug)]
struct LyricsSession {
    id: LyricsSessionId,
    generation: LyricsSearchGeneration,
    target_path: String,
    status: LyricsSearchStatus,
    candidates: Vec<LyricsCandidate>,
    preview_id: Option<LyricsCandidateId>,
    error_message: Option<String>,
    /// When set, a change of `current_path` to this value is play-and-preview, not a cancel.
    pending_play_path: Option<String>,
}

impl LyricsSessionController {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Replace any existing session and start a new search generation.
    pub(crate) fn begin_search(&mut self, query: LyricsSearchQuery) -> LyricsSearchHandle {
        self.abort_inflight_confirm();
        self.next_session_id += 1;
        self.next_generation += 1;
        let session_id = LyricsSessionId(self.next_session_id);
        let generation = LyricsSearchGeneration(self.next_generation);
        self.session = Some(LyricsSession {
            id: session_id,
            generation,
            target_path: query.path,
            status: LyricsSearchStatus::Searching,
            candidates: Vec::new(),
            preview_id: None,
            error_message: None,
            pending_play_path: None,
        });
        self.preview_lyrics = None;
        self.republish();
        LyricsSearchHandle {
            session_id,
            generation,
        }
    }

    /// Apply a provider result. Returns `false` when the handle is stale.
    pub(crate) fn apply_search_outcome(
        &mut self,
        handle: LyricsSearchHandle,
        outcome: LyricsSearchOutcome,
    ) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        if session.id != handle.session_id || session.generation != handle.generation {
            debug!(
                stale = handle.generation.get(),
                current = session.generation.get(),
                "dropping stale lyrics search result"
            );
            return false;
        }
        match outcome {
            LyricsSearchOutcome::Ready(mut candidates) => {
                for candidate in &mut candidates {
                    if candidate.id.0 == 0 {
                        self.next_candidate_id += 1;
                        candidate.id = LyricsCandidateId(self.next_candidate_id);
                    } else {
                        self.next_candidate_id = self.next_candidate_id.max(candidate.id.0);
                    }
                }
                session.status = if candidates.is_empty() {
                    LyricsSearchStatus::NoResults
                } else {
                    LyricsSearchStatus::Ready
                };
                session.candidates = candidates;
                session.preview_id = None;
                session.error_message = None;
            }
            LyricsSearchOutcome::Failed(message) => {
                session.status = LyricsSearchStatus::Failed;
                session.candidates.clear();
                session.preview_id = None;
                session.error_message = Some(message);
            }
        }
        self.preview_lyrics = None;
        self.republish();
        true
    }

    pub(crate) fn preview(
        &mut self,
        session_id: LyricsSessionId,
        candidate_id: LyricsCandidateId,
    ) -> Result<(), LyricsSessionError> {
        let session = self
            .session
            .as_mut()
            .filter(|session| session.id == session_id)
            .ok_or(LyricsSessionError::SessionNotFound)?;
        if session.status == LyricsSearchStatus::Searching {
            return Err(LyricsSessionError::StillSearching);
        }
        let candidate = session
            .candidates
            .iter()
            .find(|candidate| candidate.id == candidate_id)
            .ok_or(LyricsSessionError::CandidateNotFound)?;
        session.preview_id = Some(candidate_id);
        self.preview_lyrics = Some(Arc::new(EffectiveLyrics {
            preview_id: candidate.id,
            content: candidate.body.clone(),
        }));
        self.republish();
        Ok(())
    }

    pub(crate) fn cancel(&mut self, session_id: LyricsSessionId) -> Result<(), LyricsSessionError> {
        let Some(session) = self.session.as_ref() else {
            return Err(LyricsSessionError::SessionNotFound);
        };
        if session.id != session_id {
            return Err(LyricsSessionError::SessionNotFound);
        }
        self.abort_inflight_confirm();
        self.clear_session();
        Ok(())
    }

    /// End the session after a successful persist. Missing/stale sessions are ignored.
    pub(crate) fn complete_confirm(&mut self, session_id: LyricsSessionId) {
        if self
            .confirm_abort
            .as_ref()
            .is_some_and(|inflight| inflight.session_id == session_id)
        {
            self.confirm_abort = None;
        }
        if self
            .session
            .as_ref()
            .is_some_and(|session| session.id == session_id)
        {
            self.clear_session();
        }
    }

    /// Mark a confirm persist as in flight so later cancel/new-search can abort it.
    pub(crate) fn arm_confirm(&mut self, session_id: LyricsSessionId) -> Arc<AtomicBool> {
        self.abort_inflight_confirm();
        let abort = Arc::new(AtomicBool::new(false));
        self.confirm_abort = Some(InFlightConfirm {
            session_id,
            abort: abort.clone(),
        });
        abort
    }

    /// Whether this confirm should still complete the session after a successful write.
    pub(crate) fn confirm_still_active(
        &self,
        session_id: LyricsSessionId,
        abort: &AtomicBool,
    ) -> bool {
        !abort.load(Ordering::Acquire)
            && self
                .session
                .as_ref()
                .is_some_and(|session| session.id == session_id)
    }

    fn abort_inflight_confirm(&mut self) {
        if let Some(inflight) = self.confirm_abort.take() {
            inflight.abort.store(true, Ordering::Release);
        }
    }

    /// Arm the next current-path change to `path` so it does not cancel the session.
    pub(crate) fn arm_play_and_preview(
        &mut self,
        session_id: LyricsSessionId,
        path: &str,
    ) -> Result<(), LyricsSessionError> {
        let session = self
            .session
            .as_mut()
            .filter(|session| session.id == session_id)
            .ok_or(LyricsSessionError::SessionNotFound)?;
        if session.target_path != path {
            return Err(LyricsSessionError::PlayPathMismatch);
        }
        session.pending_play_path = Some(path.to_owned());
        self.republish();
        Ok(())
    }

    /// React to the player current path. Normal track changes cancel the session.
    pub(crate) fn sync_current_path(&mut self, current_path: Option<&str>) {
        let current = current_path.map(str::to_owned);
        if self.last_observed_path == current {
            return;
        }
        let previous = self.last_observed_path.clone();
        self.last_observed_path = current.clone();
        let Some(session) = self.session.as_mut() else {
            return;
        };
        if session.target_path.as_str() == current_path.unwrap_or("") {
            session.pending_play_path = None;
            self.republish();
            return;
        }
        if current_path.is_some() && session.pending_play_path.as_deref() == current_path {
            session.target_path = current.clone().expect("checked is_some");
            session.pending_play_path = None;
            self.republish();
            return;
        }
        // First observation only records the playing path. Searching a
        // non-current track must not be treated as a canceling track change.
        if previous.is_none() {
            return;
        }
        self.clear_session();
    }

    /// Validated in-memory candidate chosen for persist. Does not write files or
    /// mutate official metadata, and does not end the session.
    pub(crate) fn candidate_for_confirm(
        &self,
        session_id: LyricsSessionId,
        candidate_id: LyricsCandidateId,
    ) -> Result<(&str, &LyricsCandidate), LyricsSessionError> {
        let session = self
            .session
            .as_ref()
            .filter(|session| session.id == session_id)
            .ok_or(LyricsSessionError::SessionNotFound)?;
        if session.status == LyricsSearchStatus::Searching {
            return Err(LyricsSessionError::StillSearching);
        }
        let candidate = session
            .candidates
            .iter()
            .find(|candidate| candidate.id == candidate_id)
            .ok_or(LyricsSessionError::CandidateNotFound)?;
        Ok((session.target_path.as_str(), candidate))
    }

    pub(crate) fn snapshot(&self) -> Option<Arc<LyricsSessionSnapshot>> {
        self.published.clone()
    }

    /// Attach session / effective lyrics onto a player view without touching official metadata.
    pub(crate) fn apply_to_view(&self, view: &mut PlayerView) {
        let official_line = view.active_lyrics_line;
        view.lyrics_session = self.snapshot();
        let preview_applies = self.preview_applies_to(view.current_path.as_deref());
        if preview_applies {
            view.effective_lyrics = self.preview_lyrics.clone();
            view.active_lyrics_line = self
                .preview_lyrics
                .as_ref()
                .and_then(|lyrics| lyrics.content.active_line_index(view.position_ms))
                .map(|index| index as u64);
        } else {
            view.effective_lyrics = None;
            view.active_lyrics_line = official_line;
        }
    }

    fn preview_applies_to(&self, current_path: Option<&str>) -> bool {
        let Some(session) = self.session.as_ref() else {
            return false;
        };
        session.preview_id.is_some() && current_path == Some(session.target_path.as_str())
    }

    fn clear_session(&mut self) {
        self.session = None;
        self.published = None;
        self.preview_lyrics = None;
    }

    fn republish(&mut self) {
        self.published = self.session.as_ref().map(|session| {
            Arc::new(LyricsSessionSnapshot {
                session_id: session.id,
                generation: session.generation,
                target_path: session.target_path.clone(),
                status: session.status,
                candidates: session
                    .candidates
                    .iter()
                    .map(LyricsCandidateView::from_candidate)
                    .collect(),
                preview_id: session.preview_id,
                error_message: session.error_message.clone(),
            })
        });
    }
}

/// Builds a search query from already-known tags. Missing fields stay `None`.
pub(crate) fn lyrics_query_from_metadata(
    path: impl Into<String>,
    metadata: Option<&TrackMetadata>,
    duration_ms: Option<u64>,
) -> LyricsSearchQuery {
    let path = path.into();
    LyricsSearchQuery {
        title: metadata.and_then(|meta| meta.title.clone()),
        artist: metadata.and_then(|meta| meta.artist.clone()),
        album: metadata.and_then(|meta| meta.album.clone()),
        duration_ms,
        path,
    }
}

impl LyricsCandidate {
    #[cfg(test)]
    pub(crate) fn new(
        provider: LyricsProviderKind,
        title: impl Into<String>,
        artist: impl Into<String>,
        duration_ms: Option<u64>,
        body: LyricsContent,
    ) -> Self {
        let synced = matches!(body, LyricsContent::Timed { .. });
        Self {
            id: LyricsCandidateId(0),
            provider,
            title: title.into(),
            artist: artist.into(),
            duration_ms,
            synced,
            instrumental: None,
            body,
            source_id: None,
            raw_text: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_source_id(mut self, source_id: impl Into<String>) -> Self {
        self.source_id = Some(source_id.into());
        self
    }

    #[cfg(test)]
    pub(crate) fn with_raw_text(mut self, raw_text: impl Into<String>) -> Self {
        self.raw_text = Some(raw_text.into());
        self
    }
}

/// In-memory search source used by unit tests.
#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct FixtureLyricsSource {
    outcome: LyricsSearchOutcome,
}

#[cfg(test)]
impl FixtureLyricsSource {
    pub(crate) fn ready(candidates: Vec<LyricsCandidate>) -> Self {
        Self {
            outcome: LyricsSearchOutcome::Ready(candidates),
        }
    }

    pub(crate) fn failed(message: impl Into<String>) -> Self {
        Self {
            outcome: LyricsSearchOutcome::Failed(message.into()),
        }
    }

    pub(crate) fn empty() -> Self {
        Self::ready(Vec::new())
    }

    pub(crate) fn outcome(&self) -> LyricsSearchOutcome {
        self.outcome.clone()
    }
}

/// Search backend held by [`super::Application`].
#[derive(Debug, Clone)]
pub(crate) enum LyricsSearchSource {
    Remote(Arc<RemoteLyricsSource>),
    #[cfg(test)]
    Fixture(FixtureLyricsSource),
}

impl LyricsSearchSource {
    pub(crate) fn remote_default() -> Self {
        Self::Remote(super::lyrics_providers::remote_search_source())
    }

    pub(crate) async fn search(&self, query: LyricsSearchQuery) -> LyricsSearchOutcome {
        match self {
            Self::Remote(source) => source.search(query).await,
            #[cfg(test)]
            Self::Fixture(source) => source.outcome(),
        }
    }
}

impl Default for LyricsSearchSource {
    fn default() -> Self {
        Self::remote_default()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use crate::player::metadata::{Lyrics, LyricsLine, LyricsSource, TrackMetadata};
    use crate::player::{
        PlaybackMode, PlaybackState, PlayerSnapshot, QueuePosition, QueueSnapshot,
    };

    use super::super::dto::{PlayerView, PlayerViewKind};

    fn timed(lines: &[(&str, u64)]) -> LyricsContent {
        LyricsContent::Timed {
            lines: lines
                .iter()
                .map(|(text, start)| LyricsLine {
                    start_ms: *start,
                    end_ms: None,
                    text: (*text).to_owned(),
                })
                .collect(),
        }
    }

    fn official_meta(lines: &[(&str, u64)]) -> Arc<TrackMetadata> {
        Arc::new(TrackMetadata {
            title: Some("Official".into()),
            lyrics: vec![Lyrics {
                source: LyricsSource::Embedded,
                language: None,
                description: None,
                content: timed(lines),
            }],
            ..TrackMetadata::default()
        })
    }

    fn view_for(path: &str, position_ms: u64, metadata: Option<Arc<TrackMetadata>>) -> PlayerView {
        let snapshot = PlayerSnapshot {
            position: Duration::from_millis(position_ms),
            duration: Duration::from_secs(200),
            state: PlaybackState::Playing,
            volume: 1.0,
            queue: QueueSnapshot {
                main_paths: vec![path.into()],
                up_next_paths: Vec::new(),
                current: Some(QueuePosition::Main(0)),
                mode: PlaybackMode::Sequential,
            },
            metadata,
        };
        PlayerView::from_domain(snapshot, None)
    }

    fn two_candidates() -> Vec<LyricsCandidate> {
        vec![
            LyricsCandidate::new(
                LyricsProviderKind::Lrclib,
                "Rubia",
                "周深",
                Some(195_000),
                timed(&[("第一句预览", 0), ("第二句预览", 1_000)]),
            ),
            LyricsCandidate::new(
                LyricsProviderKind::Megalobiz,
                "Rubia",
                "Zhou Shen",
                Some(195_000),
                timed(&[("alt line", 0)]),
            ),
        ]
    }

    fn start_ready(path: &str) -> (LyricsSessionController, LyricsSearchHandle) {
        let mut controller = LyricsSessionController::new();
        let handle = controller.begin_search(LyricsSearchQuery {
            path: path.into(),
            title: Some("Rubia".into()),
            artist: Some("周深".into()),
            album: None,
            duration_ms: Some(195_000),
        });
        assert!(
            controller.apply_search_outcome(handle, LyricsSearchOutcome::Ready(two_candidates()))
        );
        (controller, handle)
    }

    #[test]
    fn cancel_restores_official_lyrics_and_line() {
        let official = official_meta(&[("正式第一句", 0), ("正式第二句", 1_500)]);
        let mut view = view_for("/music/rubia.flac", 1_600, Some(official));
        let official_line = view.active_lyrics_line;
        assert_eq!(official_line, Some(1));

        let (mut controller, handle) = start_ready("/music/rubia.flac");
        controller
            .preview(handle.session_id, LyricsCandidateId(1))
            .expect("preview");
        controller.apply_to_view(&mut view);
        assert!(view.effective_lyrics.is_some());
        assert_eq!(view.active_lyrics_line, Some(1));
        assert_eq!(
            view.effective_lyrics
                .as_ref()
                .and_then(|lyrics| lyrics.content.active_line_index(1_600)),
            Some(1)
        );

        controller.cancel(handle.session_id).expect("cancel");
        let mut restored = view_for("/music/rubia.flac", 1_600, view.metadata.clone());
        controller.apply_to_view(&mut restored);
        assert!(restored.effective_lyrics.is_none());
        assert!(restored.lyrics_session.is_none());
        assert_eq!(restored.active_lyrics_line, official_line);
    }

    #[test]
    fn normal_track_change_cancels_session() {
        let (mut controller, handle) = start_ready("/music/a.flac");
        controller
            .preview(handle.session_id, LyricsCandidateId(1))
            .unwrap();
        controller.sync_current_path(Some("/music/a.flac"));
        controller.sync_current_path(Some("/music/b.flac"));
        assert!(controller.snapshot().is_none());
    }

    #[test]
    fn play_and_preview_does_not_self_cancel() {
        let official_a = official_meta(&[("A官方", 0)]);
        let mut view = view_for("/music/a.flac", 0, Some(official_a));
        let (mut controller, handle) = start_ready("/music/b.flac");
        controller
            .preview(handle.session_id, LyricsCandidateId(1))
            .unwrap();
        controller.sync_current_path(Some("/music/a.flac"));
        controller.apply_to_view(&mut view);
        // Searching a non-current track must not replace the current official lyrics.
        assert!(view.effective_lyrics.is_none());
        assert_eq!(view.active_lyrics_line, Some(0));
        assert_eq!(
            view.lyrics_session.as_ref().map(|s| s.target_path.as_str()),
            Some("/music/b.flac")
        );

        controller
            .arm_play_and_preview(handle.session_id, "/music/b.flac")
            .unwrap();
        controller.sync_current_path(Some("/music/b.flac"));
        let mut playing_b = view_for("/music/b.flac", 1_200, None);
        controller.apply_to_view(&mut playing_b);
        assert!(controller.snapshot().is_some());
        assert_eq!(playing_b.active_lyrics_line, Some(1));
        assert_eq!(
            playing_b
                .effective_lyrics
                .as_ref()
                .map(|lyrics| lyrics.preview_id),
            Some(LyricsCandidateId(1))
        );
    }

    #[test]
    fn stale_generation_is_discarded() {
        let mut controller = LyricsSessionController::new();
        let first = controller.begin_search(LyricsSearchQuery {
            path: "/music/a.flac".into(),
            title: None,
            artist: None,
            album: None,
            duration_ms: None,
        });
        let second = controller.begin_search(LyricsSearchQuery {
            path: "/music/b.flac".into(),
            title: None,
            artist: None,
            album: None,
            duration_ms: None,
        });
        assert_ne!(first.generation.get(), second.generation.get());
        assert_ne!(first.session_id.get(), second.session_id.get());
        assert!(
            !controller.apply_search_outcome(first, LyricsSearchOutcome::Ready(two_candidates()))
        );
        assert_eq!(
            controller.snapshot().map(|s| s.target_path.clone()),
            Some("/music/b.flac".into())
        );
        assert_eq!(
            controller.snapshot().map(|s| s.status),
            Some(LyricsSearchStatus::Searching)
        );
        assert!(
            controller.apply_search_outcome(second, LyricsSearchOutcome::Ready(two_candidates()))
        );
        assert_eq!(
            controller.snapshot().map(|s| s.status),
            Some(LyricsSearchStatus::Ready)
        );
        assert_eq!(
            controller
                .snapshot()
                .map(|s| s.candidates.len())
                .unwrap_or(0),
            2
        );
    }

    #[test]
    fn progress_tick_keeps_session_and_preview_identity() {
        let official = official_meta(&[("正式", 0), ("正式二", 2_000)]);
        let mut first = view_for("/music/rubia.flac", 200, Some(official.clone()));
        let (mut controller, handle) = start_ready("/music/rubia.flac");
        controller
            .preview(handle.session_id, LyricsCandidateId(1))
            .unwrap();
        controller.apply_to_view(&mut first);

        let mut second = view_for("/music/rubia.flac", 1_200, Some(official));
        controller.apply_to_view(&mut second);
        assert!(second.is_progress_only_update(&first));
        assert!(Arc::ptr_eq(
            first.lyrics_session.as_ref().unwrap(),
            second.lyrics_session.as_ref().unwrap()
        ));
        assert!(Arc::ptr_eq(
            first.effective_lyrics.as_ref().unwrap(),
            second.effective_lyrics.as_ref().unwrap()
        ));
        let tick = second.as_progress_tick();
        assert_eq!(tick.kind, PlayerViewKind::Progress);
        assert!(tick.effective_lyrics.is_some());
        assert!(tick.lyrics_session.is_some());
        assert_eq!(tick.active_lyrics_line, Some(1));
    }

    #[test]
    fn confirming_a_non_current_track_does_not_change_current_snapshot() {
        let official = official_meta(&[("正在播的正式词", 0)]);
        let mut view = view_for("/music/playing.flac", 0, Some(official));
        let (mut controller, handle) = start_ready("/music/other.flac");
        controller
            .preview(handle.session_id, LyricsCandidateId(1))
            .unwrap();
        controller.sync_current_path(Some("/music/playing.flac"));
        controller.apply_to_view(&mut view);
        let before = view.clone();
        let (path, candidate) = controller
            .candidate_for_confirm(handle.session_id, LyricsCandidateId(1))
            .expect("confirm intent");
        assert_eq!(path, "/music/other.flac");
        assert_eq!(candidate.title, "Rubia");
        controller.apply_to_view(&mut view);
        assert_eq!(view, before);
        assert!(view.effective_lyrics.is_none());
        assert_eq!(view.current_path.as_deref(), Some("/music/playing.flac"));
    }

    #[test]
    fn empty_search_is_no_results_and_single_candidate_is_not_auto_previewed() {
        let mut controller = LyricsSessionController::new();
        let handle = controller.begin_search(LyricsSearchQuery {
            path: "/music/x.flac".into(),
            title: None,
            artist: None,
            album: None,
            duration_ms: None,
        });
        assert!(controller.apply_search_outcome(handle, LyricsSearchOutcome::Ready(Vec::new())));
        assert_eq!(
            controller.snapshot().map(|s| s.status),
            Some(LyricsSearchStatus::NoResults)
        );

        let handle = controller.begin_search(LyricsSearchQuery {
            path: "/music/x.flac".into(),
            title: None,
            artist: None,
            album: None,
            duration_ms: None,
        });
        let mut only = two_candidates();
        only.truncate(1);
        assert!(controller.apply_search_outcome(handle, LyricsSearchOutcome::Ready(only)));
        let snapshot = controller.snapshot().expect("session");
        assert_eq!(snapshot.status, LyricsSearchStatus::Ready);
        assert_eq!(snapshot.candidates.len(), 1);
        assert!(snapshot.preview_id.is_none());
        assert!(controller.snapshot().unwrap().preview_id.is_none());
    }

    #[test]
    fn failed_and_empty_fixture_outcomes_are_applied() {
        let empty = FixtureLyricsSource::empty();
        let failed = FixtureLyricsSource::failed("provider down");
        let mut controller = LyricsSessionController::new();
        let handle = controller.begin_search(LyricsSearchQuery {
            path: "/music/x.flac".into(),
            title: None,
            artist: None,
            album: None,
            duration_ms: None,
        });
        assert!(controller.apply_search_outcome(handle, empty.outcome()));
        assert_eq!(
            controller.snapshot().map(|s| s.status),
            Some(LyricsSearchStatus::NoResults)
        );

        let handle = controller.begin_search(LyricsSearchQuery {
            path: "/music/x.flac".into(),
            title: None,
            artist: None,
            album: None,
            duration_ms: None,
        });
        assert!(controller.apply_search_outcome(handle, failed.outcome()));
        assert_eq!(
            controller.snapshot().map(|s| s.status),
            Some(LyricsSearchStatus::Failed)
        );
        assert_eq!(
            controller
                .snapshot()
                .and_then(|s| s.error_message.clone())
                .as_deref(),
            Some("provider down")
        );
        let parsed = LyricsContent::from_text("[00:01.00]hello".into());
        assert!(matches!(parsed, LyricsContent::Timed { .. }));
        assert_eq!(parsed.active_line_index(0), Some(0));
        assert_eq!(parsed.active_line_index(2_000), Some(0));
    }

    #[test]
    fn cancel_and_new_search_abort_inflight_confirm_but_track_change_does_not() {
        let (mut controller, handle) = start_ready("/music/a.flac");
        let abort = controller.arm_confirm(handle.session_id);
        assert!(!abort.load(std::sync::atomic::Ordering::Acquire));

        // A later track change must not abort a confirm the user already approved.
        controller.sync_current_path(Some("/music/a.flac"));
        controller.sync_current_path(Some("/music/b.flac"));
        assert!(controller.snapshot().is_none());
        assert!(
            !abort.load(std::sync::atomic::Ordering::Acquire),
            "track change must not abort an in-flight persist"
        );
        assert!(!controller.confirm_still_active(handle.session_id, &abort));

        let (mut controller, handle) = start_ready("/music/a.flac");
        let abort = controller.arm_confirm(handle.session_id);
        controller
            .cancel(handle.session_id)
            .expect("cancel current session");
        assert!(abort.load(std::sync::atomic::Ordering::Acquire));
        assert!(!controller.confirm_still_active(handle.session_id, &abort));

        let (mut controller, first) = start_ready("/music/a.flac");
        let abort = controller.arm_confirm(first.session_id);
        let _second = controller.begin_search(LyricsSearchQuery {
            path: "/music/c.flac".into(),
            title: None,
            artist: None,
            album: None,
            duration_ms: None,
        });
        assert!(abort.load(std::sync::atomic::Ordering::Acquire));
        assert!(!controller.confirm_still_active(first.session_id, &abort));
    }
}
