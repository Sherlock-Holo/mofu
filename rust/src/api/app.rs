//! Single FFI application facade: bootstrap, commands, and state streams.

use std::sync::Arc;
use std::thread;

use crate::api::collection::{Playlist, PlaylistKind, Song};
use crate::api::player::{
    CoverArt, EffectiveLyrics, LyricsCandidateSnapshot, LyricsContent, LyricsSearchStatus,
    LyricsSessionSnapshot, PlaybackMode, PlayerSnapshot, PlayerSnapshotKind, PlayerState,
    QueueItem, TrackMetadata,
};
use crate::application::{
    Application, BootstrapSnapshot as DomainBootstrap, EffectiveLyrics as DomainEffectiveLyrics,
    LibrarySnapshot as DomainLibrary, LyricsCandidateId, LyricsCandidateView,
    LyricsConfirmAction as DomainLyricsConfirmAction,
    LyricsConfirmOutcome as DomainLyricsConfirmOutcome, LyricsSaveLocation,
    LyricsSearchStatus as DomainLyricsSearchStatus, LyricsSessionId,
    LyricsSessionSnapshot as DomainLyricsSessionSnapshot, PlayerView, PlayerViewKind, PlaylistView,
    SongView,
};
use crate::collection::CoverAnimationInBackground as DomainCoverAnimation;
use crate::collection::PlaylistKind as DomainPlaylistKind;
use crate::frb_generated::StreamSink;

/// Opaque application handle owned by Flutter.
#[flutter_rust_bridge::frb(opaque)]
pub struct App {
    inner: Arc<Application>,
}

impl App {
    /// Open catalog + player, restore session, and start background watchers.
    pub async fn bootstrap() -> Result<(Self, BootstrapSnapshot), String> {
        let (inner, snapshot) = Application::bootstrap()
            .await
            .map_err(|err| err.to_string())?;
        Ok((Self { inner }, snapshot.into()))
    }

    /// Push player snapshots (throttled progress + immediate semantic changes).
    pub fn watch_player(&self, sink: StreamSink<PlayerSnapshot>) {
        let rx = self.inner.subscribe_player();
        thread::Builder::new()
            .name("mofu-frb-player".into())
            .spawn(move || {
                while let Ok(view) = rx.recv() {
                    if sink.add(PlayerSnapshot::from(view)).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn player stream bridge");
    }

    /// Push library snapshots (lite first, then cover-hydrated).
    pub fn watch_library(&self, sink: StreamSink<LibrarySnapshot>) {
        let rx = self.inner.subscribe_library();
        thread::Builder::new()
            .name("mofu-frb-library".into())
            .spawn(move || {
                while let Ok(snapshot) = rx.recv() {
                    if sink.add(LibrarySnapshot::from(snapshot)).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn library stream bridge");
    }

    /// Current player snapshot without waiting for the next push.
    #[flutter_rust_bridge::frb(sync)]
    pub fn player_snapshot(&self) -> PlayerSnapshot {
        self.inner.current_player_view().into()
    }

    #[flutter_rust_bridge::frb(sync)]
    pub fn play(&self) -> Result<(), String> {
        self.inner.play().map_err(|err| err.to_string())
    }

    #[flutter_rust_bridge::frb(sync)]
    pub fn pause(&self) -> Result<(), String> {
        self.inner.pause().map_err(|err| err.to_string())
    }

    #[flutter_rust_bridge::frb(sync)]
    pub fn stop(&self) -> Result<(), String> {
        self.inner.stop().map_err(|err| err.to_string())
    }

    #[flutter_rust_bridge::frb(sync)]
    pub fn next(&self) -> Result<(), String> {
        self.inner.next().map_err(|err| err.to_string())
    }

    #[flutter_rust_bridge::frb(sync)]
    pub fn previous(&self) -> Result<(), String> {
        self.inner.previous().map_err(|err| err.to_string())
    }

    #[flutter_rust_bridge::frb(sync)]
    pub fn seek(&self, position_ms: u64) -> Result<(), String> {
        self.inner.seek(position_ms).map_err(|err| err.to_string())
    }

    #[flutter_rust_bridge::frb(sync)]
    pub fn seek_relative(&self, delta_ms: i64) -> Result<(), String> {
        self.inner
            .seek_relative(delta_ms)
            .map_err(|err| err.to_string())
    }

    #[flutter_rust_bridge::frb(sync)]
    pub fn set_volume(&self, volume: f64) -> Result<(), String> {
        self.inner.set_volume(volume).map_err(|err| err.to_string())
    }

    #[flutter_rust_bridge::frb(sync)]
    pub fn set_playback_mode(&self, mode: PlaybackMode) -> Result<(), String> {
        self.inner
            .set_playback_mode(mode.into())
            .map_err(|err| err.to_string())
    }

    #[flutter_rust_bridge::frb(sync)]
    pub fn cover_animation_in_background(&self) -> CoverAnimationInBackground {
        self.inner.cover_animation_in_background().into()
    }

    #[flutter_rust_bridge::frb(sync)]
    pub fn set_cover_animation_in_background(
        &self,
        value: CoverAnimationInBackground,
    ) -> Result<(), String> {
        self.inner
            .set_cover_animation_in_background(value.into())
            .map_err(|err| err.to_string())
    }

    #[flutter_rust_bridge::frb(sync)]
    pub fn enqueue_next(&self, path: String) -> Result<(), String> {
        self.inner.enqueue_next(path).map_err(|err| err.to_string())
    }

    /// Start a lyrics search session.
    pub async fn lyrics_search(&self, path: String) -> Result<u64, String> {
        self.inner
            .lyrics_search(path)
            .await
            .map(|id| id.get())
            .map_err(|err| err.to_string())
    }

    /// Select a candidate as the unsaved preview.
    #[flutter_rust_bridge::frb(sync)]
    pub fn lyrics_preview(&self, session_id: u64, candidate_id: u64) -> Result<(), String> {
        self.inner
            .lyrics_preview(
                LyricsSessionId::from_raw(session_id),
                LyricsCandidateId::from_raw(candidate_id),
            )
            .map_err(|err| err.to_string())
    }

    /// Discard the session and restore official lyrics.
    #[flutter_rust_bridge::frb(sync)]
    pub fn lyrics_cancel(&self, session_id: u64) -> Result<(), String> {
        self.inner
            .lyrics_cancel(LyricsSessionId::from_raw(session_id))
            .map_err(|err| err.to_string())
    }

    /// Play the session target without cancelling preview.
    #[flutter_rust_bridge::frb(sync)]
    pub fn lyrics_play_and_preview(&self, session_id: u64, path: String) -> Result<(), String> {
        self.inner
            .lyrics_play_and_preview(LyricsSessionId::from_raw(session_id), path)
            .map_err(|err| err.to_string())
    }

    /// Confirm the selected in-memory candidate. May return a conflict that needs a follow-up action.
    ///
    /// File IO runs off the FFI thread so playback controls stay responsive.
    pub async fn lyrics_confirm(
        &self,
        session_id: u64,
        candidate_id: u64,
        action: LyricsConfirmAction,
    ) -> Result<LyricsConfirmResult, String> {
        self.inner
            .lyrics_confirm(
                LyricsSessionId::from_raw(session_id),
                LyricsCandidateId::from_raw(candidate_id),
                DomainLyricsConfirmAction::from(action),
            )
            .await
            .map(LyricsConfirmResult::from)
            .map_err(|err| err.to_string())
    }

    pub async fn play_playlist(
        &self,
        playlist_id: u64,
        start_path: Option<String>,
        autoplay: bool,
    ) -> Result<(), String> {
        self.inner
            .play_playlist(playlist_id, start_path, autoplay)
            .await
            .map_err(|err| err.to_string())
    }

    pub async fn create_virtual(&self, name: String) -> Result<u64, String> {
        self.inner
            .create_virtual(name)
            .await
            .map_err(|err| err.to_string())
    }

    pub async fn rename_virtual(&self, id: u64, name: String) -> Result<(), String> {
        self.inner
            .rename_virtual(id, name)
            .await
            .map_err(|err| err.to_string())
    }

    pub async fn delete_virtual(&self, id: u64) -> Result<(), String> {
        self.inner
            .delete_virtual(id)
            .await
            .map_err(|err| err.to_string())
    }

    pub async fn refresh(&self, id: u64) -> Result<(), String> {
        self.inner.refresh(id).await.map_err(|err| err.to_string())
    }

    pub async fn pick_and_add_directory(&self) -> Result<Option<u64>, String> {
        self.inner
            .pick_and_add_directory()
            .await
            .map_err(|err| err.to_string())
    }

    pub async fn pick_and_add_files(
        &self,
        selected_playlist_id: Option<u64>,
    ) -> Result<Option<u64>, String> {
        self.inner
            .pick_and_add_files(selected_playlist_id)
            .await
            .map_err(|err| err.to_string())
    }

    pub async fn import_from_playlist(
        &self,
        target_id: u64,
        source_id: u64,
        paths: Vec<String>,
    ) -> Result<(), String> {
        self.inner
            .import_from_playlist(target_id, source_id, paths)
            .await
            .map_err(|err| err.to_string())
    }

    /// Persist pending volume/session immediately (app pause / dispose).
    #[flutter_rust_bridge::frb(sync)]
    pub fn flush(&self) {
        self.inner.flush();
    }

    /// Stop background workers before the handle is dropped.
    #[flutter_rust_bridge::frb(sync)]
    pub fn shutdown(&self) {
        self.inner.shutdown_and_join();
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.inner.shutdown_and_join();
    }
}

/// Bootstrap payload returned once at startup.
#[derive(Debug, Clone, PartialEq)]
pub struct BootstrapSnapshot {
    pub library: LibrarySnapshot,
    pub player: PlayerSnapshot,
    pub selected_playlist_id: Option<u64>,
    pub cover_animation_in_background: CoverAnimationInBackground,
}

/// Whether the now-playing cover keeps spinning while the app is backgrounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverAnimationInBackground {
    Pause,
    Continue,
}

/// Library state pushed to Flutter.
#[derive(Debug, Clone, PartialEq)]
pub struct LibrarySnapshot {
    pub playlists: Vec<Playlist>,
    pub covers_hydrated: bool,
}

impl From<DomainBootstrap> for BootstrapSnapshot {
    fn from(value: DomainBootstrap) -> Self {
        Self {
            library: value.library.into(),
            player: value.player.into(),
            selected_playlist_id: value.selected_playlist_id,
            cover_animation_in_background: value.cover_animation_in_background.into(),
        }
    }
}

impl From<DomainCoverAnimation> for CoverAnimationInBackground {
    fn from(value: DomainCoverAnimation) -> Self {
        match value {
            DomainCoverAnimation::Pause => Self::Pause,
            DomainCoverAnimation::Continue => Self::Continue,
        }
    }
}

impl From<CoverAnimationInBackground> for DomainCoverAnimation {
    fn from(value: CoverAnimationInBackground) -> Self {
        match value {
            CoverAnimationInBackground::Pause => Self::Pause,
            CoverAnimationInBackground::Continue => Self::Continue,
        }
    }
}

impl From<DomainLibrary> for LibrarySnapshot {
    fn from(value: DomainLibrary) -> Self {
        Self {
            playlists: value.playlists.into_iter().map(Playlist::from).collect(),
            covers_hydrated: value.covers_hydrated,
        }
    }
}

impl From<PlaylistView> for Playlist {
    fn from(value: PlaylistView) -> Self {
        let cover = value.cover.map(CoverArt::from);
        let (kind, directory_path) = match value.kind {
            DomainPlaylistKind::Directory { path } => (PlaylistKind::Directory, Some(path)),
            DomainPlaylistKind::Virtual => (PlaylistKind::Virtual, None),
        };
        Self {
            id: value.id,
            name: value.name,
            kind,
            directory_path,
            cover,
            songs: value.songs.into_iter().map(Song::from).collect(),
        }
    }
}

impl From<SongView> for Song {
    fn from(value: SongView) -> Self {
        Self {
            path: value.path,
            duration_ms: value.duration_ms,
            display_title: value.display_title,
            metadata: TrackMetadata::from(&value.metadata),
        }
    }
}

impl From<PlayerViewKind> for PlayerSnapshotKind {
    fn from(value: PlayerViewKind) -> Self {
        match value {
            PlayerViewKind::Full => Self::Full,
            PlayerViewKind::Progress => Self::Progress,
        }
    }
}

impl From<PlayerView> for PlayerSnapshot {
    fn from(value: PlayerView) -> Self {
        Self {
            kind: value.kind.into(),
            position_ms: value.position_ms,
            duration_ms: value.duration_ms,
            state: PlayerState::from(value.state),
            volume: value.volume,
            playback_mode: value.playback_mode.into(),
            current_index: value.current_index,
            current_from_up_next: value.current_from_up_next,
            current_path: value.current_path,
            display_title: value.display_title,
            active_lyrics_line: value.active_lyrics_line,
            lyrics_session: value
                .lyrics_session
                .as_deref()
                .map(LyricsSessionSnapshot::from),
            effective_lyrics: value.effective_lyrics.as_deref().map(EffectiveLyrics::from),
            source_playlist_id: value.source_playlist_id,
            queue: value
                .queue
                .into_iter()
                .map(|item| QueueItem {
                    index: item.index,
                    path: item.path,
                })
                .collect(),
            up_next: value
                .up_next
                .into_iter()
                .map(|item| QueueItem {
                    index: item.index,
                    path: item.path,
                })
                .collect(),
            metadata: value
                .metadata
                .as_ref()
                .map(|meta| TrackMetadata::from(meta.as_ref())),
        }
    }
}

/// Authorized persist action for a confirmed lyrics candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LyricsConfirmAction {
    Save,
    ReplaceExistingSidecar,
    PreferDataDir,
}

impl From<LyricsConfirmAction> for DomainLyricsConfirmAction {
    fn from(value: LyricsConfirmAction) -> Self {
        match value {
            LyricsConfirmAction::Save => Self::Save,
            LyricsConfirmAction::ReplaceExistingSidecar => Self::ReplaceExistingSidecar,
            LyricsConfirmAction::PreferDataDir => Self::PreferDataDir,
        }
    }
}

/// Flattened confirm result so Dart generation does not require Freezed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LyricsConfirmResultKind {
    Saved,
    NeedsReplace,
    NeedsStemCollisionAck,
}

/// Structured confirm result, including conflict state and the authorized follow-up action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricsConfirmResult {
    pub kind: LyricsConfirmResultKind,
    pub audio_path: String,
    pub saved_path: Option<String>,
    pub stored_in_data_dir: Option<bool>,
    pub replaced_existing: Option<bool>,
    pub sidecar_path: Option<String>,
    pub conflicting_audio: Vec<String>,
}

impl From<DomainLyricsConfirmOutcome> for LyricsConfirmResult {
    fn from(value: DomainLyricsConfirmOutcome) -> Self {
        match value {
            DomainLyricsConfirmOutcome::Saved(info) => Self {
                kind: LyricsConfirmResultKind::Saved,
                audio_path: info.audio_path,
                saved_path: Some(info.saved_path.to_string_lossy().into_owned()),
                stored_in_data_dir: Some(info.location == LyricsSaveLocation::DataDir),
                replaced_existing: Some(info.replaced_existing),
                sidecar_path: None,
                conflicting_audio: Vec::new(),
            },
            DomainLyricsConfirmOutcome::NeedsReplace {
                audio_path,
                sidecar_path,
            } => Self {
                kind: LyricsConfirmResultKind::NeedsReplace,
                audio_path,
                saved_path: None,
                stored_in_data_dir: None,
                replaced_existing: None,
                sidecar_path: Some(sidecar_path.to_string_lossy().into_owned()),
                conflicting_audio: Vec::new(),
            },
            DomainLyricsConfirmOutcome::NeedsStemCollisionAck {
                audio_path,
                sidecar_path,
                conflicting_audio,
            } => Self {
                kind: LyricsConfirmResultKind::NeedsStemCollisionAck,
                audio_path,
                saved_path: None,
                stored_in_data_dir: None,
                replaced_existing: None,
                sidecar_path: Some(sidecar_path.to_string_lossy().into_owned()),
                conflicting_audio: conflicting_audio
                    .into_iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect(),
            },
        }
    }
}

impl From<DomainLyricsSearchStatus> for LyricsSearchStatus {
    fn from(value: DomainLyricsSearchStatus) -> Self {
        match value {
            DomainLyricsSearchStatus::Searching => Self::Searching,
            DomainLyricsSearchStatus::Ready => Self::Ready,
            DomainLyricsSearchStatus::NoResults => Self::NoResults,
            DomainLyricsSearchStatus::Failed => Self::Failed,
        }
    }
}

impl From<&LyricsCandidateView> for LyricsCandidateSnapshot {
    fn from(value: &LyricsCandidateView) -> Self {
        Self {
            id: value.id.get(),
            provider: value.provider.as_str().to_owned(),
            provider_label: value.provider_label.to_owned(),
            title: value.title.clone(),
            artist: value.artist.clone(),
            duration_ms: value.duration_ms,
            synced: value.synced,
            instrumental: value.instrumental,
        }
    }
}

impl From<&DomainLyricsSessionSnapshot> for LyricsSessionSnapshot {
    fn from(value: &DomainLyricsSessionSnapshot) -> Self {
        Self {
            session_id: value.session_id.get(),
            target_path: value.target_path.clone(),
            status: value.status.into(),
            candidates: value
                .candidates
                .iter()
                .map(LyricsCandidateSnapshot::from)
                .collect(),
            preview_id: value.preview_id.map(|id| id.get()),
            error_message: value.error_message.clone(),
        }
    }
}

impl From<&DomainEffectiveLyrics> for EffectiveLyrics {
    fn from(value: &DomainEffectiveLyrics) -> Self {
        Self {
            preview_id: value.preview_id.get(),
            content: LyricsContent::from(&value.content),
        }
    }
}
