//! Application orchestration: bootstrap, persistence, and state fan-out.

mod dto;
mod lyrics;
mod lyrics_http;
mod lyrics_providers;
mod lyrics_store;
mod ports;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use async_global_executor::Task;
use directories::ProjectDirs;
pub(crate) use dto::{
    BootstrapSnapshot, LibrarySnapshot, PlayerView, PlayerViewKind, PlaylistView, SongView,
};
use flume::{Receiver, Sender, TrySendError};
pub(crate) use lyrics::{
    EffectiveLyrics, LyricsCandidateId, LyricsCandidateView, LyricsSearchSource,
    LyricsSearchStatus, LyricsSessionController, LyricsSessionId, LyricsSessionSnapshot,
};
pub(crate) use lyrics_store::{
    LyricsConfirmAction, LyricsConfirmOutcome, LyricsSaveLocation, LyricsStore,
};
use ports::{CollectionAdapter, CollectionPort, PlayerAdapter, PlayerPort};
use tracing::{debug, warn};

use crate::collection::{
    CollectionError, CoverAnimationInBackground, PlaybackSession, PlaylistId, SessionRestorePlan,
    StoredPlaybackMode,
};
use crate::file_dialog::{
    DialogSelection, DirectoryDialogRequest, FileDialog, MusicFilesDialogRequest, RfdFileDialog,
};
use crate::player::{PlaybackMode, PlaybackState, PlayerError};

const PLAYER_PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const VOLUME_DEBOUNCE: Duration = Duration::from_millis(400);
const FLUSH_TIMEOUT: Duration = Duration::from_secs(5);

/// Failures from application orchestration boundaries.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ApplicationError {
    #[error(transparent)]
    Player(#[from] PlayerError),
    #[error(transparent)]
    Collection(#[from] CollectionError),
    #[error("selected track is no longer in the playlist")]
    TrackNotFound,
    #[error(transparent)]
    Lyrics(#[from] lyrics::LyricsSessionError),
    #[error(transparent)]
    LyricsStore(#[from] lyrics_store::LyricsStoreError),
}

/// Coordinates player, collection, persistence, and UI snapshots.
pub(crate) struct Application<C = CollectionAdapter, P = PlayerAdapter, D = RfdFileDialog>
where
    C: CollectionPort,
    P: PlayerPort,
    D: FileDialog + Send + Sync + 'static,
{
    player: P,
    collection: C,
    dialog: D,
    source_playlist_id: Mutex<Option<PlaylistId>>,
    volume_state: Mutex<VolumePersistState>,
    cover_animation_in_background: Mutex<CoverAnimationInBackground>,
    player_events: Mutex<SnapshotBus<PlayerView>>,
    library_events: Mutex<SnapshotBus<LibrarySnapshot>>,
    persist_tx: Sender<PersistCommand>,
    shutdown: Arc<AtomicBool>,
    cover_generation: AtomicU64,
    lyrics: Mutex<LyricsSessionController>,
    lyrics_source: LyricsSearchSource,
    lyrics_store: LyricsStore,
    watch_thread: Mutex<Option<JoinHandle<()>>>,
    persist_task: Mutex<Option<Task<()>>>,
    cover_task: Mutex<Option<Task<()>>>,
    lyrics_task: Mutex<Option<Task<()>>>,
    lyrics_hydrate_task: Mutex<Option<Task<()>>>,
    lyrics_hydrate_inflight: Mutex<Option<String>>,
}

#[derive(Debug, Default)]
struct VolumePersistState {
    pending: Option<f64>,
    due_at: Option<Instant>,
}

#[derive(Debug)]
enum PersistCommand {
    Session(Option<PlaybackSession>),
    Mode(PlaybackMode),
    CoverAnimation(CoverAnimationInBackground),
    Volume(f64),
    Flush {
        volume: f64,
        mode: PlaybackMode,
        cover_animation_in_background: CoverAnimationInBackground,
        session: Option<PlaybackSession>,
        reply: Sender<()>,
    },
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionWatchKey {
    path: Option<String>,
    index: Option<u64>,
    state: PlaybackState,
}

impl SessionWatchKey {
    fn from_view(view: &PlayerView) -> Self {
        Self {
            path: view.current_path.clone(),
            index: view.current_index,
            state: view.state,
        }
    }
}

/// Atomically registers subscribers and publishes the latest snapshot.
#[derive(Debug)]
struct SnapshotBus<T> {
    latest: Option<T>,
    subscribers: Vec<SnapshotSubscriber<T>>,
}

#[derive(Debug)]
struct SnapshotSubscriber<T> {
    latest: Arc<Mutex<Option<T>>>,
    wake_tx: Sender<()>,
}

/// Coalesced latest-value subscription used by FFI bridge threads.
pub(crate) struct SnapshotSubscription<T> {
    latest: Arc<Mutex<Option<T>>>,
    wake_rx: Receiver<()>,
}

impl<T> SnapshotSubscription<T> {
    pub(crate) fn recv(&self) -> Result<T, flume::RecvError> {
        loop {
            self.wake_rx.recv()?;
            if let Some(snapshot) = self.latest.lock().expect("snapshot slot mutex").take() {
                return Ok(snapshot);
            }
        }
    }
}

impl<T> Default for SnapshotBus<T> {
    fn default() -> Self {
        Self {
            latest: None,
            subscribers: Vec::new(),
        }
    }
}

impl<T: Clone> SnapshotBus<T> {
    fn publish(&mut self, snapshot: T) {
        self.latest = Some(snapshot.clone());
        self.subscribers.retain(|subscriber| {
            *subscriber.latest.lock().expect("snapshot slot mutex") = Some(snapshot.clone());
            !matches!(
                subscriber.wake_tx.try_send(()),
                Err(TrySendError::Disconnected(_))
            )
        });
    }

    fn subscribe(&mut self) -> SnapshotSubscription<T> {
        let latest = Arc::new(Mutex::new(self.latest.clone()));
        let (wake_tx, wake_rx) = flume::bounded(1);
        if self.latest.is_some() {
            let _ = wake_tx.send(());
        }
        self.subscribers.push(SnapshotSubscriber {
            latest: latest.clone(),
            wake_tx,
        });
        SnapshotSubscription { latest, wake_rx }
    }
}

impl Application<CollectionAdapter, PlayerAdapter, RfdFileDialog> {
    /// Open collection + player, restore session, and start background watchers.
    pub(crate) async fn bootstrap() -> Result<(Arc<Self>, BootstrapSnapshot), ApplicationError> {
        static EXECUTOR_INIT: Once = Once::new();
        EXECUTOR_INIT.call_once(|| {
            async_global_executor::init_with_config(
                async_global_executor::GlobalExecutorConfig::default()
                    .with_env_var("MOFU_ASYNC_THREADS")
                    .with_min_threads(1)
                    .with_max_threads(2),
            );
        });

        let collection = CollectionAdapter::open_default().await?;
        let lyrics_store = LyricsStore::open(
            ProjectDirs::from("dev", "mofu", "mofu")
                .ok_or(lyrics_store::LyricsStoreError::DataDirectoryUnavailable)?
                .data_dir()
                .join("lyrics"),
        )?;
        let player = PlayerAdapter::new();
        // Persistence traffic is already coalesced/debounced and must never be dropped.
        let (persist_tx, persist_rx) = flume::unbounded();
        let shutdown = Arc::new(AtomicBool::new(false));

        let cover_animation_in_background =
            CollectionPort::cover_animation_in_background(&collection).await?;

        let app = Arc::new(Self {
            player,
            collection,
            dialog: RfdFileDialog::default(),
            source_playlist_id: Mutex::new(None),
            volume_state: Mutex::new(VolumePersistState::default()),
            cover_animation_in_background: Mutex::new(cover_animation_in_background),
            player_events: Mutex::new(SnapshotBus::default()),
            library_events: Mutex::new(SnapshotBus::default()),
            persist_tx,
            shutdown,
            cover_generation: AtomicU64::new(0),
            lyrics: Mutex::new(LyricsSessionController::new()),
            lyrics_source: LyricsSearchSource::remote_default(),
            lyrics_store,
            watch_thread: Mutex::new(None),
            persist_task: Mutex::new(None),
            cover_task: Mutex::new(None),
            lyrics_task: Mutex::new(None),
            lyrics_hydrate_task: Mutex::new(None),
            lyrics_hydrate_inflight: Mutex::new(None),
        });

        let volume = CollectionPort::volume(&app.collection).await?;
        PlayerPort::set_volume(&app.player, volume as f32)?;

        let mode = CollectionPort::playback_mode(&app.collection).await?;
        PlayerPort::set_playback_mode(&app.player, PlaybackMode::from(mode))?;

        let playlists = CollectionPort::library_snapshot_lite(&app.collection).await?;
        let mut selected_playlist_id = playlists.first().map(|playlist| playlist.id);

        if let Some(session) = CollectionPort::playback_session(&app.collection).await? {
            match CollectionPort::resolve_playback_session(&app.collection, session).await? {
                Some(plan) => {
                    if app.restore_playback_session(plan).await {
                        selected_playlist_id = *app
                            .source_playlist_id
                            .lock()
                            .expect("source playlist mutex");
                    }
                }
                None => {
                    debug!("bootstrap cleared invalid playback session");
                }
            }
        }

        let library = LibrarySnapshot::from_playlists(playlists, false);
        if let Some(path) = app
            .player
            .full_snapshot()
            .queue
            .current_path()
            .map(|path| path.to_string_lossy().into_owned())
        {
            app.lyrics_store.hydrate_official(&path);
        }
        let player_view = app.player_view();
        app.library_events
            .lock()
            .expect("library events mutex")
            .publish(library.clone());
        app.player_events
            .lock()
            .expect("player events mutex")
            .publish(player_view.clone());
        let bootstrap = BootstrapSnapshot {
            library: library.clone(),
            player: player_view,
            selected_playlist_id,
            cover_animation_in_background,
        };

        app.spawn_persist_worker(persist_rx);
        app.spawn_player_watch();
        app.spawn_cover_hydrate();

        Ok((app, bootstrap))
    }
}

impl<C, P, D> Application<C, P, D>
where
    C: CollectionPort,
    P: PlayerPort,
    D: FileDialog + Send + Sync + 'static,
{
    async fn restore_playback_session(&self, plan: SessionRestorePlan) -> bool {
        let playlist_id = plan.playlist_id;
        *self
            .source_playlist_id
            .lock()
            .expect("source playlist mutex") = Some(playlist_id);
        let queue_result = self.player.set_queue(
            plan.paths.into_iter().map(PathBuf::from).collect(),
            Some(plan.start_index),
            false,
        );
        let restore_result = match queue_result {
            Ok(()) if plan.position_ms > 0 => {
                self.player.seek(Duration::from_millis(plan.position_ms))
            }
            result => result,
        };
        if let Err(err) = restore_result {
            warn!(%err, requested_position_ms = plan.position_ms, "playback session restore failed");
            *self
                .source_playlist_id
                .lock()
                .expect("source playlist mutex") = None;
            let _ = self.player.set_queue(Vec::new(), None, false);
            if let Err(err) = self.collection.clear_playback_session().await {
                warn!(%err, "clear unusable playback session failed");
            }
            return false;
        }
        true
    }

    fn spawn_persist_worker(self: &Arc<Self>, rx: Receiver<PersistCommand>) {
        let app = self.clone();
        let task = async_global_executor::spawn(async move {
            app.run_persist_loop(rx).await;
        });
        *self.persist_task.lock().expect("persist task mutex") = Some(task);
    }

    async fn run_persist_loop(self: Arc<Self>, rx: Receiver<PersistCommand>) {
        while let Ok(command) = rx.recv_async().await {
            match command {
                PersistCommand::Shutdown => break,
                PersistCommand::Session(session) => {
                    self.persist_session(session).await;
                }
                PersistCommand::Mode(mode) => {
                    if let Err(err) = self
                        .collection
                        .set_playback_mode(StoredPlaybackMode::from(mode))
                        .await
                    {
                        warn!(%err, "persist playback mode failed");
                    }
                }
                PersistCommand::CoverAnimation(value) => {
                    if let Err(err) = self
                        .collection
                        .set_cover_animation_in_background(value)
                        .await
                    {
                        warn!(%err, "persist cover animation preference failed");
                    }
                }
                PersistCommand::Volume(volume) => {
                    if let Err(err) = self.collection.set_volume(volume).await {
                        warn!(%err, "persist volume failed");
                    }
                }
                PersistCommand::Flush {
                    volume,
                    mode,
                    cover_animation_in_background,
                    session,
                    reply,
                } => {
                    if let Err(err) = self.collection.set_volume(volume).await {
                        warn!(%err, "flush volume failed");
                    }
                    if let Err(err) = self
                        .collection
                        .set_playback_mode(StoredPlaybackMode::from(mode))
                        .await
                    {
                        warn!(%err, "flush playback mode failed");
                    }
                    if let Err(err) = self
                        .collection
                        .set_cover_animation_in_background(cover_animation_in_background)
                        .await
                    {
                        warn!(%err, "flush cover animation preference failed");
                    }
                    self.persist_session(session).await;
                    let _ = reply.send(());
                }
            }
        }
    }

    async fn persist_session(&self, session: Option<PlaybackSession>) {
        let result = match session {
            Some(session) => self.collection.set_playback_session(session).await,
            None => self.collection.clear_playback_session().await,
        };
        if let Err(err) = result {
            warn!(%err, "persist session failed");
        }
    }

    fn spawn_player_watch(self: &Arc<Self>) {
        let app = self.clone();
        let handle = std::thread::Builder::new()
            .name("mofu-player-watch".into())
            .spawn(move || app.run_player_watch())
            .expect("spawn player watch");
        *self.watch_thread.lock().expect("watch thread mutex") = Some(handle);
    }

    fn spawn_cover_hydrate(self: &Arc<Self>) {
        let generation = self.cover_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let app = self.clone();
        let task = async_global_executor::spawn(async move {
            match app.collection.library_snapshot_with_covers().await {
                Ok(playlists)
                    if !app.shutdown.load(Ordering::Acquire)
                        && app.cover_generation.load(Ordering::Acquire) == generation =>
                {
                    let snapshot = LibrarySnapshot::from_playlists(playlists, true);
                    app.publish_library_snapshot(snapshot);
                }
                Ok(_) => {}
                Err(err) => warn!(%err, "cover hydrate failed"),
            }
        });
        let mut slot = self.cover_task.lock().expect("cover task mutex");
        // The generation guard prevents a canceled task's final poll from publishing stale data.
        *slot = Some(task);
    }

    fn run_player_watch(self: Arc<Self>) {
        let (mut last_view, needed) = self.player_view_and_hydrate();
        if let Some(path) = needed {
            self.schedule_official_hydrate(path);
        }
        let mut last_key = SessionWatchKey::from_view(&last_view);
        loop {
            if self.shutdown.load(Ordering::Acquire) {
                break;
            }
            self.player.wait_for_progress(PLAYER_PROGRESS_INTERVAL);
            self.tick_volume_debounce();
            self.sync_lyrics_session();

            let (view, needed) = self.player_view_and_hydrate();
            if let Some(path) = needed {
                self.schedule_official_hydrate(path);
            }
            let key = SessionWatchKey::from_view(&view);
            if key != last_key {
                let paused_from_playing =
                    last_key.state == PlaybackState::Playing && key.state == PlaybackState::Paused;
                let track_changed = last_key.path != key.path || last_key.index != key.index;
                let stopped =
                    key.state == PlaybackState::Idle && last_key.state != PlaybackState::Idle;
                if track_changed || paused_from_playing || stopped {
                    self.queue_persist_session_from_view(&view);
                }
                last_key = key;
            }

            if view != last_view {
                let published = if view.is_progress_only_update(&last_view) {
                    view.as_progress_tick()
                } else {
                    view.clone()
                };
                last_view = view;
                self.fanout_player(published);
            }
        }
    }

    fn player_view(&self) -> PlayerView {
        self.player_view_and_hydrate().0
    }

    /// Build the current view from memory only. The optional path should be
    /// hydrated on a background task so the watch loop never reads lyric files.
    fn player_view_and_hydrate(&self) -> (PlayerView, Option<String>) {
        let source = *self
            .source_playlist_id
            .lock()
            .expect("source playlist mutex");
        let mut view = PlayerView::from_domain(self.player.full_snapshot(), source);
        let needed = self.lyrics_store.apply_official_to_view(&mut view);
        self.lyrics
            .lock()
            .expect("lyrics session mutex")
            .apply_to_view(&mut view);
        (view, needed)
    }

    fn schedule_official_hydrate(self: &Arc<Self>, path: String) {
        if self.shutdown.load(Ordering::Acquire) {
            return;
        }
        {
            let mut inflight = self
                .lyrics_hydrate_inflight
                .lock()
                .expect("lyrics hydrate inflight mutex");
            if inflight.as_deref() == Some(path.as_str()) {
                return;
            }
            *inflight = Some(path.clone());
        }
        let app = self.clone();
        let task = async_global_executor::spawn(async move {
            app.lyrics_store.hydrate_official(&path);
            let mut inflight = app
                .lyrics_hydrate_inflight
                .lock()
                .expect("lyrics hydrate inflight mutex");
            if inflight.as_deref() == Some(path.as_str()) {
                *inflight = None;
            }
            drop(inflight);
            if !app.shutdown.load(Ordering::Acquire) {
                app.publish_current_player_view();
            }
        });
        *self
            .lyrics_hydrate_task
            .lock()
            .expect("lyrics hydrate task mutex") = Some(task);
    }

    fn sync_lyrics_session(&self) {
        let current_path = self
            .player
            .full_snapshot()
            .queue
            .current_path()
            .map(|path| path.to_string_lossy().into_owned());
        self.lyrics
            .lock()
            .expect("lyrics session mutex")
            .sync_current_path(current_path.as_deref());
    }

    fn publish_current_player_view(&self) {
        self.fanout_player(self.player_view());
    }

    fn fanout_player(&self, view: PlayerView) {
        self.player_events
            .lock()
            .expect("player events mutex")
            .publish(view);
    }

    fn publish_library_snapshot(&self, snapshot: LibrarySnapshot) {
        self.library_events
            .lock()
            .expect("library events mutex")
            .publish(snapshot);
    }

    fn queue_persist_session_from_view(&self, view: &PlayerView) {
        let session = match (
            view.state,
            view.source_playlist_id,
            view.session_path.as_ref(),
        ) {
            (PlaybackState::Idle, _, _) => None,
            (_, Some(playlist_id), Some(path)) if !view.queue.is_empty() => Some(PlaybackSession {
                playlist_id,
                path: path.clone(),
                position_ms: view.session_position_ms,
            }),
            _ => None,
        };
        if self
            .persist_tx
            .send(PersistCommand::Session(session))
            .is_err()
        {
            warn!("persist session channel closed");
        }
    }

    fn tick_volume_debounce(&self) {
        let mut state = self.volume_state.lock().expect("volume state mutex");
        let Some(due_at) = state.due_at else {
            return;
        };
        if Instant::now() < due_at {
            return;
        }
        let Some(volume) = state.pending.take() else {
            state.due_at = None;
            return;
        };
        state.due_at = None;
        drop(state);
        if self
            .persist_tx
            .send(PersistCommand::Volume(volume))
            .is_err()
        {
            warn!("persist volume channel closed");
        }
    }

    /// Register for player updates; immediately receives the current snapshot.
    pub(crate) fn subscribe_player(&self) -> SnapshotSubscription<PlayerView> {
        self.player_events
            .lock()
            .expect("player events mutex")
            .subscribe()
    }

    /// Register for library updates; immediately receives the latest snapshot when present.
    pub(crate) fn subscribe_library(&self) -> SnapshotSubscription<LibrarySnapshot> {
        self.library_events
            .lock()
            .expect("library events mutex")
            .subscribe()
    }

    pub(crate) fn current_player_view(&self) -> PlayerView {
        self.player_view()
    }

    async fn reload_library_and_hydrate(self: &Arc<Self>) -> Result<(), ApplicationError> {
        let playlists = self.collection.library_snapshot_lite().await?;
        let snapshot = LibrarySnapshot::from_playlists(playlists, false);
        self.publish_library_snapshot(snapshot);
        self.spawn_cover_hydrate();
        Ok(())
    }

    pub(crate) fn play(&self) -> Result<(), ApplicationError> {
        Ok(self.player.play()?)
    }

    pub(crate) fn pause(&self) -> Result<(), ApplicationError> {
        Ok(self.player.pause()?)
    }

    pub(crate) fn stop(&self) -> Result<(), ApplicationError> {
        self.player.stop()?;
        self.sync_lyrics_session();
        Ok(())
    }

    pub(crate) fn next(&self) -> Result<(), ApplicationError> {
        self.player.next()?;
        self.sync_lyrics_session();
        Ok(())
    }

    pub(crate) fn previous(&self) -> Result<(), ApplicationError> {
        self.player.previous()?;
        self.sync_lyrics_session();
        Ok(())
    }

    pub(crate) fn seek(&self, position_ms: u64) -> Result<(), ApplicationError> {
        Ok(self.player.seek(Duration::from_millis(position_ms))?)
    }

    pub(crate) fn seek_relative(&self, delta_ms: i64) -> Result<(), ApplicationError> {
        Ok(self.player.seek_relative(delta_ms)?)
    }

    pub(crate) fn set_volume(&self, volume: f64) -> Result<(), ApplicationError> {
        let volume = if volume.is_finite() {
            volume.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.player.set_volume(volume as f32)?;
        let mut state = self.volume_state.lock().expect("volume state mutex");
        state.pending = Some(volume);
        state.due_at = Some(Instant::now() + VOLUME_DEBOUNCE);
        Ok(())
    }

    pub(crate) fn set_playback_mode(&self, mode: PlaybackMode) -> Result<(), ApplicationError> {
        self.player.set_playback_mode(mode)?;
        if self.persist_tx.send(PersistCommand::Mode(mode)).is_err() {
            warn!("persist playback mode channel closed");
        }
        Ok(())
    }

    pub(crate) fn cover_animation_in_background(&self) -> CoverAnimationInBackground {
        *self
            .cover_animation_in_background
            .lock()
            .expect("cover animation mutex")
    }

    pub(crate) fn set_cover_animation_in_background(
        &self,
        value: CoverAnimationInBackground,
    ) -> Result<(), ApplicationError> {
        *self
            .cover_animation_in_background
            .lock()
            .expect("cover animation mutex") = value;
        if self
            .persist_tx
            .send(PersistCommand::CoverAnimation(value))
            .is_err()
        {
            warn!("persist cover animation channel closed");
        }
        Ok(())
    }

    pub(crate) fn enqueue_next(&self, path: String) -> Result<(), ApplicationError> {
        Ok(self.player.enqueue_next(PathBuf::from(path))?)
    }

    pub(crate) async fn play_playlist(
        self: &Arc<Self>,
        playlist_id: PlaylistId,
        start_path: Option<String>,
        autoplay: bool,
    ) -> Result<(), ApplicationError> {
        let playlists = self.collection.playlists().await?;
        let Some(playlist) = playlists.into_iter().find(|item| item.id == playlist_id) else {
            return Err(ApplicationError::Collection(
                CollectionError::PlaylistNotFound,
            ));
        };
        let paths = playlist
            .songs
            .into_iter()
            .map(|song| PathBuf::from(song.path))
            .collect::<Vec<_>>();
        if paths.is_empty() {
            return Ok(());
        }
        let start = match start_path {
            Some(path) => {
                let path = PathBuf::from(path);
                paths
                    .iter()
                    .position(|candidate| candidate == &path)
                    .ok_or(ApplicationError::TrackNotFound)?
            }
            None => 0,
        };
        *self
            .source_playlist_id
            .lock()
            .expect("source playlist mutex") = Some(playlist_id);
        self.player.set_queue(paths, Some(start), autoplay)?;
        self.sync_lyrics_session();
        Ok(())
    }

    pub(crate) async fn create_virtual(
        self: &Arc<Self>,
        name: String,
    ) -> Result<PlaylistId, ApplicationError> {
        let created = self.collection.create_virtual(name).await?;
        self.reload_library_and_hydrate().await?;
        Ok(created.id)
    }

    pub(crate) async fn rename_virtual(
        self: &Arc<Self>,
        id: PlaylistId,
        name: String,
    ) -> Result<(), ApplicationError> {
        self.collection.rename_virtual(id, name).await?;
        self.reload_library_and_hydrate().await
    }

    pub(crate) async fn delete_virtual(
        self: &Arc<Self>,
        id: PlaylistId,
    ) -> Result<(), ApplicationError> {
        self.collection.delete_virtual(id).await?;
        self.reload_library_and_hydrate().await
    }

    pub(crate) async fn refresh(self: &Arc<Self>, id: PlaylistId) -> Result<(), ApplicationError> {
        self.collection.refresh(id).await?;
        self.reload_library_and_hydrate().await
    }

    pub(crate) async fn pick_and_add_directory(
        self: &Arc<Self>,
    ) -> Result<Option<PlaylistId>, ApplicationError> {
        match self
            .dialog
            .pick_directory(DirectoryDialogRequest::new("选择音乐目录"))
            .await
        {
            DialogSelection::Cancelled => Ok(None),
            DialogSelection::Selected(path) => {
                let created = self.collection.add_directory(path).await?;
                self.reload_library_and_hydrate().await?;
                Ok(Some(created.id))
            }
        }
    }

    pub(crate) async fn pick_and_add_files(
        self: &Arc<Self>,
        selected_playlist_id: Option<PlaylistId>,
    ) -> Result<Option<PlaylistId>, ApplicationError> {
        match self
            .dialog
            .pick_music_files(MusicFilesDialogRequest::new("添加音乐文件"))
            .await
        {
            DialogSelection::Cancelled => Ok(None),
            DialogSelection::Selected(paths) => {
                let target = self
                    .collection
                    .ensure_virtual_import_target(selected_playlist_id)
                    .await?;
                self.collection.add_files(target.id, paths).await?;
                self.reload_library_and_hydrate().await?;
                Ok(Some(target.id))
            }
        }
    }

    pub(crate) async fn import_from_playlist(
        self: &Arc<Self>,
        target_id: PlaylistId,
        source_id: PlaylistId,
        paths: Vec<String>,
    ) -> Result<(), ApplicationError> {
        self.collection
            .import_from_playlist(target_id, source_id, paths)
            .await?;
        self.reload_library_and_hydrate().await
    }

    pub(crate) async fn lyrics_search(
        self: &Arc<Self>,
        path: String,
    ) -> Result<LyricsSessionId, ApplicationError> {
        let query = self.lyrics_query_for_path(&path).await;
        let handle = self
            .lyrics
            .lock()
            .expect("lyrics session mutex")
            .begin_search(query.clone());
        self.spawn_lyrics_search(handle, query);
        self.publish_current_player_view();
        Ok(handle.session_id)
    }

    pub(crate) fn lyrics_preview(
        &self,
        session_id: LyricsSessionId,
        candidate_id: LyricsCandidateId,
    ) -> Result<(), ApplicationError> {
        self.lyrics
            .lock()
            .expect("lyrics session mutex")
            .preview(session_id, candidate_id)?;
        self.publish_current_player_view();
        Ok(())
    }

    pub(crate) fn lyrics_cancel(
        &self,
        session_id: LyricsSessionId,
    ) -> Result<(), ApplicationError> {
        self.lyrics
            .lock()
            .expect("lyrics session mutex")
            .cancel(session_id)?;
        self.publish_current_player_view();
        Ok(())
    }

    /// Confirm the chosen in-memory candidate and persist it when the action is allowed.
    ///
    /// Candidate lookup stays in memory; the file write runs on async_global_executor
    /// so the FFI / UI thread and the player-watch loop are not blocked. This is
    /// not the CPAL callback or decode worker; those threads never call the
    /// lyrics store or providers.
    pub(crate) async fn lyrics_confirm(
        &self,
        session_id: LyricsSessionId,
        candidate_id: LyricsCandidateId,
        action: LyricsConfirmAction,
    ) -> Result<LyricsConfirmOutcome, ApplicationError> {
        let (request, abort) = {
            let mut lyrics = self.lyrics.lock().expect("lyrics session mutex");
            let (path, candidate) = lyrics.candidate_for_confirm(session_id, candidate_id)?;
            let request = lyrics_store::LyricsConfirmRequest {
                audio_path: path.to_owned(),
                text: candidate.text_for_save(),
                provider: candidate.provider.as_str().to_owned(),
                source_id: candidate.source_id.clone(),
                title: Some(candidate.title.clone()),
                artist: Some(candidate.artist.clone()),
                action,
            };
            let abort = lyrics.arm_confirm(session_id);
            (request, abort)
        };
        debug!(
            session = session_id.get(),
            candidate = candidate_id.get(),
            path = request.audio_path.as_str(),
            "persisting confirmed lyrics candidate"
        );
        let store = self.lyrics_store.clone();
        let outcome = async_global_executor::spawn({
            let abort = abort.clone();
            async move { store.confirm_with_abort(request, Some(abort.as_ref())) }
        })
        .await?;
        let Some(outcome) = outcome else {
            return Err(lyrics::LyricsSessionError::ConfirmAbandoned.into());
        };
        if matches!(outcome, LyricsConfirmOutcome::Saved(_)) {
            let mut lyrics = self.lyrics.lock().expect("lyrics session mutex");
            if lyrics.confirm_still_active(session_id, abort.as_ref()) {
                lyrics.complete_confirm(session_id);
                drop(lyrics);
                self.publish_current_player_view();
            }
        }
        Ok(outcome)
    }

    /// Switch playback to the session target without cancelling the preview session.
    pub(crate) fn lyrics_play_and_preview(
        &self,
        session_id: LyricsSessionId,
        path: String,
    ) -> Result<(), ApplicationError> {
        self.lyrics
            .lock()
            .expect("lyrics session mutex")
            .arm_play_and_preview(session_id, &path)?;
        let snapshot = self.player.full_snapshot();
        if let Some(index) = snapshot
            .queue
            .main_paths
            .iter()
            .position(|candidate| candidate.as_os_str() == std::ffi::OsStr::new(&path))
        {
            self.player.play_index(index)?;
        } else {
            self.player
                .set_queue(vec![PathBuf::from(&path)], Some(0), true)?;
        }
        self.sync_lyrics_session();
        self.publish_current_player_view();
        Ok(())
    }

    async fn lyrics_query_for_path(&self, path: &str) -> lyrics::LyricsSearchQuery {
        let snapshot = self.player.full_snapshot();
        if snapshot
            .queue
            .current_path()
            .is_some_and(|current| current.as_os_str() == std::ffi::OsStr::new(path))
        {
            return lyrics::lyrics_query_from_metadata(
                path,
                snapshot.metadata.as_deref(),
                Some(snapshot.duration.as_millis() as u64),
            );
        }
        if let Ok(playlists) = self.collection.playlists().await {
            for playlist in playlists {
                if let Some(song) = playlist.songs.iter().find(|song| song.path == path) {
                    return lyrics::lyrics_query_from_metadata(
                        path,
                        Some(&song.metadata),
                        Some(song.duration_ms),
                    );
                }
            }
        }
        lyrics::lyrics_query_from_metadata(path, None, None)
    }

    fn spawn_lyrics_search(
        self: &Arc<Self>,
        handle: lyrics::LyricsSearchHandle,
        query: lyrics::LyricsSearchQuery,
    ) {
        let app = self.clone();
        let source = self.lyrics_source.clone();
        // Search (including HTTP) runs on async_global_executor, not the audio or decode threads.
        let task = async_global_executor::spawn(async move {
            if app.shutdown.load(Ordering::Acquire) {
                return;
            }
            let outcome = source.search(query).await;
            if app.shutdown.load(Ordering::Acquire) {
                return;
            }
            let applied = app
                .lyrics
                .lock()
                .expect("lyrics session mutex")
                .apply_search_outcome(handle, outcome);
            if applied {
                app.publish_current_player_view();
            }
        });
        *self.lyrics_task.lock().expect("lyrics task mutex") = Some(task);
    }

    pub(crate) fn flush(&self) -> bool {
        let mut state = self.volume_state.lock().expect("volume state mutex");
        state.pending = None;
        state.due_at = None;
        drop(state);

        let view = self.player_view();
        let volume = view.volume;
        let mode = view.playback_mode;
        let cover_animation_in_background = self.cover_animation_in_background();
        let session = match (view.state, view.source_playlist_id, view.session_path) {
            (PlaybackState::Idle, _, _) => None,
            (_, Some(playlist_id), Some(path)) if !view.queue.is_empty() => Some(PlaybackSession {
                playlist_id,
                path,
                position_ms: view.session_position_ms,
            }),
            _ => None,
        };
        let (reply_tx, reply_rx) = flume::bounded(1);
        if self
            .persist_tx
            .send(PersistCommand::Flush {
                volume,
                mode,
                cover_animation_in_background,
                session,
                reply: reply_tx,
            })
            .is_err()
        {
            return false;
        }
        if reply_rx.recv_timeout(FLUSH_TIMEOUT).is_err() {
            warn!("persist flush timed out");
            return false;
        }
        true
    }

    #[cfg(test)]
    fn new_for_test(
        collection: C,
        player: P,
        dialog: D,
        lyrics_source: LyricsSearchSource,
    ) -> Arc<Self> {
        Self::new_for_test_with_store(
            collection,
            player,
            dialog,
            lyrics_source,
            LyricsStore::open_ephemeral().expect("temp lyrics store"),
        )
    }

    #[cfg(test)]
    fn new_for_test_with_store(
        collection: C,
        player: P,
        dialog: D,
        lyrics_source: LyricsSearchSource,
        lyrics_store: LyricsStore,
    ) -> Arc<Self> {
        let (persist_tx, _persist_rx) = flume::unbounded();
        Arc::new(Self {
            player,
            collection,
            dialog,
            source_playlist_id: Mutex::new(None),
            volume_state: Mutex::new(VolumePersistState::default()),
            cover_animation_in_background: Mutex::new(CoverAnimationInBackground::default()),
            player_events: Mutex::new(SnapshotBus::default()),
            library_events: Mutex::new(SnapshotBus::default()),
            persist_tx,
            shutdown: Arc::new(AtomicBool::new(false)),
            cover_generation: AtomicU64::new(0),
            lyrics: Mutex::new(LyricsSessionController::new()),
            lyrics_source,
            lyrics_store,
            watch_thread: Mutex::new(None),
            persist_task: Mutex::new(None),
            cover_task: Mutex::new(None),
            lyrics_task: Mutex::new(None),
            lyrics_hydrate_task: Mutex::new(None),
            lyrics_hydrate_inflight: Mutex::new(None),
        })
    }

    #[cfg(test)]
    fn lyrics_store(&self) -> &LyricsStore {
        &self.lyrics_store
    }

    #[cfg(test)]
    fn sync_lyrics_session_for_test(&self) {
        self.sync_lyrics_session();
    }

    /// Stop watchers and wait for background tasks to exit.
    pub(crate) fn shutdown_and_join(&self) {
        if self.shutdown.swap(true, Ordering::AcqRel) {
            return;
        }
        let flushed = self.flush();

        self.cover_generation.fetch_add(1, Ordering::AcqRel);
        self.player.wake_progress_watch();

        if let Some(handle) = self.watch_thread.lock().expect("watch thread mutex").take() {
            let _ = handle.join();
        }

        let _ = self.persist_tx.send(PersistCommand::Shutdown);
        if let Some(task) = self.persist_task.lock().expect("persist task mutex").take() {
            if flushed {
                async_global_executor::block_on(task);
            } else {
                let _ = async_global_executor::block_on(task.cancel());
            }
        }
        if let Some(task) = self.cover_task.lock().expect("cover task mutex").take() {
            let _ = async_global_executor::block_on(task.cancel());
        }
        if let Some(task) = self.lyrics_task.lock().expect("lyrics task mutex").take() {
            let _ = async_global_executor::block_on(task.cancel());
        }
        if let Some(task) = self
            .lyrics_hydrate_task
            .lock()
            .expect("lyrics hydrate task mutex")
            .take()
        {
            let _ = async_global_executor::block_on(task.cancel());
        }
        self.player.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn snapshot_subscription_cannot_miss_handoff_update() {
        let mut bus = SnapshotBus::default();
        bus.publish(1_u8);
        let rx = bus.subscribe();
        bus.publish(2);

        assert_eq!(rx.recv().expect("latest snapshot"), 2);
        assert!(matches!(
            rx.wake_rx.try_recv(),
            Err(flume::TryRecvError::Empty)
        ));
    }

    #[test]
    fn snapshot_publish_removes_disconnected_subscribers() {
        let mut bus = SnapshotBus::default();
        let rx = bus.subscribe();
        drop(rx);

        bus.publish(1_u8);

        assert!(bus.subscribers.is_empty());
    }

    #[test]
    fn application_wires_fake_search_preview_and_cancel() {
        use crate::player::metadata::{LyricsContent, LyricsLine, TrackMetadata};
        use lyrics::{
            FixtureLyricsSource, LyricsCandidate, LyricsProviderKind, LyricsSearchSource,
            LyricsSearchStatus,
        };

        let path = "/music/rubia.flac";
        let metadata = TrackMetadata {
            title: Some("Rubia".into()),
            artist: Some("周深".into()),
            lyrics: Vec::new(),
            ..TrackMetadata::default()
        };
        let player = FakePlayer::new(path, metadata.clone());
        let collection = FakeCollection::with_song(path, 195_000, metadata);
        let source =
            LyricsSearchSource::Fixture(FixtureLyricsSource::ready(vec![LyricsCandidate::new(
                LyricsProviderKind::Lrclib,
                "Rubia",
                "周深",
                Some(195_000),
                LyricsContent::Timed {
                    lines: vec![LyricsLine {
                        start_ms: 0,
                        end_ms: None,
                        text: "预览句".into(),
                    }],
                },
            )]));
        let app = Application::new_for_test(collection, player, StubDialog, source);
        let session_id =
            async_global_executor::block_on(app.lyrics_search(path.into())).expect("search starts");
        let snapshot = wait_for_lyrics_status(&app, LyricsSearchStatus::Ready);
        assert_eq!(snapshot.session_id, session_id);
        assert_eq!(snapshot.session_id.get(), session_id.get());
        assert_eq!(snapshot.candidates.len(), 1);

        let candidate_id = snapshot.candidates[0].id;
        assert!(candidate_id.get() > 0);
        app.lyrics_preview(session_id, candidate_id)
            .expect("preview");
        let view = app.current_player_view();
        assert!(view.effective_lyrics.is_some());
        assert_eq!(view.active_lyrics_line, Some(0));

        app.lyrics_cancel(session_id).expect("cancel");
        let view = app.current_player_view();
        assert!(view.effective_lyrics.is_none());
        assert!(view.lyrics_session.is_none());
    }

    #[test]
    fn application_play_and_preview_keeps_session_across_track_switch() {
        use crate::player::metadata::{LyricsContent, LyricsLine, TrackMetadata};
        use lyrics::{
            FixtureLyricsSource, LyricsCandidate, LyricsProviderKind, LyricsSearchSource,
            LyricsSearchStatus,
        };

        let playing = "/music/a.flac";
        let target = "/music/b.flac";
        let metadata = TrackMetadata {
            title: Some("A".into()),
            ..TrackMetadata::default()
        };
        let player = FakePlayer::with_queue(&[playing, target], 0, metadata.clone());
        let mut collection_songs = FakeCollection::with_song(playing, 1000, metadata.clone());
        collection_songs.push_song(target, 2000, TrackMetadata::default());
        let source =
            LyricsSearchSource::Fixture(FixtureLyricsSource::ready(vec![LyricsCandidate::new(
                LyricsProviderKind::Fixture,
                "B",
                "Artist",
                Some(2_000),
                LyricsContent::Timed {
                    lines: vec![LyricsLine {
                        start_ms: 0,
                        end_ms: None,
                        text: "B preview".into(),
                    }],
                },
            )]));
        let app = Application::new_for_test(collection_songs, player, StubDialog, source);
        let session_id =
            async_global_executor::block_on(app.lyrics_search(target.into())).expect("search");
        let snapshot = wait_for_lyrics_status(&app, LyricsSearchStatus::Ready);
        let candidate_id = snapshot.candidates[0].id;
        app.lyrics_preview(session_id, candidate_id)
            .expect("preview");
        let view = app.current_player_view();
        assert_eq!(view.current_path.as_deref(), Some(playing));
        assert!(view.effective_lyrics.is_none());

        app.lyrics_play_and_preview(session_id, target.into())
            .expect("play and preview");
        let view = app.current_player_view();
        assert_eq!(view.current_path.as_deref(), Some(target));
        assert!(view.lyrics_session.is_some());
        assert!(view.effective_lyrics.is_some());
    }

    fn unique_test_dir(prefix: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).expect("test dir");
        path
    }

    fn timed_candidate(line: &str) -> lyrics::LyricsCandidate {
        use crate::player::metadata::LyricsContent;
        use lyrics::{LyricsCandidate, LyricsProviderKind};

        let raw = format!("[00:00.000]{line}\n");
        LyricsCandidate::new(
            LyricsProviderKind::Lrclib,
            "Rubia",
            "周深",
            Some(195_000),
            LyricsContent::from_text(raw.clone()),
        )
        .with_raw_text(raw)
        .with_source_id("cand-1")
    }

    fn official_embedded(line: &str) -> crate::player::metadata::TrackMetadata {
        use crate::player::metadata::{
            Lyrics, LyricsContent, LyricsLine, LyricsSource, TrackMetadata,
        };
        TrackMetadata {
            title: Some("Official".into()),
            lyrics: vec![Lyrics {
                source: LyricsSource::Embedded,
                language: None,
                description: None,
                content: LyricsContent::Timed {
                    lines: vec![LyricsLine {
                        start_ms: 0,
                        end_ms: None,
                        text: line.into(),
                    }],
                },
            }],
            ..TrackMetadata::default()
        }
    }

    fn preferred_line(view: &PlayerView) -> Option<String> {
        use crate::player::metadata::{Lyrics, LyricsContent};
        view.metadata.as_ref().and_then(|meta| {
            Lyrics::preferred(&meta.lyrics).map(|lyrics| match &lyrics.content {
                LyricsContent::Timed { lines } => lines[0].text.clone(),
                LyricsContent::Plain { text } => text.clone(),
            })
        })
    }

    #[test]
    fn confirm_saves_current_track_and_switches_official_immediately() {
        use lyrics::{FixtureLyricsSource, LyricsSearchSource, LyricsSearchStatus};

        let music = unique_test_dir("mofu-lyrics-app-music");
        let store_root = unique_test_dir("mofu-lyrics-app-store");
        let audio = music.join("rubia.flac");
        std::fs::write(&audio, b"fake").expect("audio");
        let path = audio.to_str().expect("utf-8").to_owned();
        let metadata = official_embedded("正式旧词");
        let player = FakePlayer::new(&path, metadata.clone());
        let collection = FakeCollection::with_song(&path, 195_000, metadata);
        let store = LyricsStore::open(store_root.clone()).expect("store");
        let app = Application::new_for_test_with_store(
            collection,
            player,
            StubDialog,
            LyricsSearchSource::Fixture(FixtureLyricsSource::ready(vec![timed_candidate(
                "确认新词",
            )])),
            store,
        );
        let session_id =
            async_global_executor::block_on(app.lyrics_search(path.clone())).expect("search");
        let snapshot = wait_for_lyrics_status(&app, LyricsSearchStatus::Ready);
        let candidate_id = snapshot.candidates[0].id;
        app.lyrics_preview(session_id, candidate_id)
            .expect("preview");
        assert!(app.current_player_view().effective_lyrics.is_some());

        let outcome = async_global_executor::block_on(app.lyrics_confirm(
            session_id,
            candidate_id,
            LyricsConfirmAction::Save,
        ))
        .expect("confirm");
        match outcome {
            LyricsConfirmOutcome::Saved(info) => {
                assert_eq!(info.location, LyricsSaveLocation::Sidecar);
                assert_eq!(info.saved_path, audio.with_extension("lrc"));
            }
            other => panic!("expected saved, got {other:?}"),
        }
        let view = app.current_player_view();
        assert!(view.lyrics_session.is_none());
        assert!(view.effective_lyrics.is_none());
        assert_eq!(preferred_line(&view).as_deref(), Some("确认新词"));
        assert_eq!(
            std::fs::read_to_string(audio.with_extension("lrc")).expect("lrc"),
            "[00:00.000]确认新词\n"
        );
        let index = std::fs::read_to_string(store_root.join("overrides.json")).expect("index");
        assert!(!index.contains("确认新词"));
        let reopened = LyricsStore::open(store_root.clone()).expect("reopen");
        assert!(reopened.load_official(&path).is_some());
        let _ = std::fs::remove_dir_all(music);
        let _ = std::fs::remove_dir_all(store_root);
    }

    #[test]
    fn confirm_non_current_track_does_not_change_current_lyrics() {
        use lyrics::{FixtureLyricsSource, LyricsSearchSource, LyricsSearchStatus};

        let music = unique_test_dir("mofu-lyrics-app-other");
        let store_root = unique_test_dir("mofu-lyrics-app-other-store");
        let playing = music.join("playing.flac");
        let other = music.join("other.flac");
        std::fs::write(&playing, b"a").unwrap();
        std::fs::write(&other, b"b").unwrap();
        let playing_path = playing.to_str().unwrap().to_owned();
        let other_path = other.to_str().unwrap().to_owned();
        let playing_meta = official_embedded("正在播的正式词");
        let player = FakePlayer::with_queue(
            &[playing_path.as_str(), other_path.as_str()],
            0,
            playing_meta.clone(),
        );
        let mut collection = FakeCollection::with_song(&playing_path, 1000, playing_meta);
        collection.push_song(&other_path, 2000, official_embedded("other official"));
        let app = Application::new_for_test_with_store(
            collection,
            player,
            StubDialog,
            LyricsSearchSource::Fixture(FixtureLyricsSource::ready(vec![timed_candidate(
                "另一首新词",
            )])),
            LyricsStore::open(store_root.clone()).expect("store"),
        );
        let session_id =
            async_global_executor::block_on(app.lyrics_search(other_path.clone())).expect("search");
        let snapshot = wait_for_lyrics_status(&app, LyricsSearchStatus::Ready);
        let candidate_id = snapshot.candidates[0].id;
        app.lyrics_preview(session_id, candidate_id)
            .expect("preview");
        let before = preferred_line(&app.current_player_view());
        assert_eq!(before.as_deref(), Some("正在播的正式词"));
        assert!(app.current_player_view().effective_lyrics.is_none());

        let outcome = async_global_executor::block_on(app.lyrics_confirm(
            session_id,
            candidate_id,
            LyricsConfirmAction::Save,
        ))
        .expect("confirm");
        assert!(matches!(outcome, LyricsConfirmOutcome::Saved(_)));
        let view = app.current_player_view();
        assert_eq!(view.current_path.as_deref(), Some(playing_path.as_str()));
        assert!(view.lyrics_session.is_none());
        assert!(view.effective_lyrics.is_none());
        assert_eq!(preferred_line(&view), before);
        assert!(other.with_extension("lrc").exists());
        assert!(!playing.with_extension("lrc").exists());
        let _ = std::fs::remove_dir_all(music);
        let _ = std::fs::remove_dir_all(store_root);
    }

    #[test]
    fn cancel_does_not_persist_unconfirmed_candidates() {
        use lyrics::{FixtureLyricsSource, LyricsSearchSource, LyricsSearchStatus};

        let music = unique_test_dir("mofu-lyrics-app-cancel");
        let store_root = unique_test_dir("mofu-lyrics-app-cancel-store");
        let audio = music.join("rubia.flac");
        std::fs::write(&audio, b"fake").unwrap();
        let path = audio.to_str().unwrap().to_owned();
        let metadata = official_embedded("正式词");
        let app = Application::new_for_test_with_store(
            FakeCollection::with_song(&path, 195_000, metadata.clone()),
            FakePlayer::new(&path, metadata),
            StubDialog,
            LyricsSearchSource::Fixture(FixtureLyricsSource::ready(vec![timed_candidate(
                "未确认词",
            )])),
            LyricsStore::open(store_root.clone()).expect("store"),
        );
        let session_id =
            async_global_executor::block_on(app.lyrics_search(path.clone())).expect("search");
        let snapshot = wait_for_lyrics_status(&app, LyricsSearchStatus::Ready);
        app.lyrics_preview(session_id, snapshot.candidates[0].id)
            .expect("preview");
        app.lyrics_cancel(session_id).expect("cancel");
        let view = app.current_player_view();
        assert!(view.lyrics_session.is_none());
        assert!(view.effective_lyrics.is_none());
        assert_eq!(preferred_line(&view).as_deref(), Some("正式词"));
        assert!(!audio.with_extension("lrc").exists());
        assert!(!app.lyrics_store().has_override(&path));
        assert!(!store_root.join("overrides.json").exists());
        let _ = std::fs::remove_dir_all(music);
        let _ = std::fs::remove_dir_all(store_root);
    }

    struct ReadyConfirm {
        app: std::sync::Arc<Application<FakeCollection, FakePlayer, StubDialog>>,
        music: PathBuf,
        store_root: PathBuf,
        audio: PathBuf,
        path: String,
        session_id: lyrics::LyricsSessionId,
        candidate_id: lyrics::LyricsCandidateId,
    }

    fn ready_to_confirm(prefix: &str, line: &str) -> ReadyConfirm {
        use lyrics::{FixtureLyricsSource, LyricsSearchSource, LyricsSearchStatus};

        let music = unique_test_dir(&format!("{prefix}-music"));
        let store_root = unique_test_dir(&format!("{prefix}-store"));
        let audio = music.join("rubia.flac");
        std::fs::write(&audio, b"fake").expect("audio");
        let path = audio.to_str().expect("utf-8").to_owned();
        let metadata = official_embedded("正式旧词");
        let store = LyricsStore::open(store_root.clone()).expect("store");
        let app = Application::new_for_test_with_store(
            FakeCollection::with_song(&path, 195_000, metadata.clone()),
            FakePlayer::new(&path, metadata),
            StubDialog,
            LyricsSearchSource::Fixture(FixtureLyricsSource::ready(vec![timed_candidate(line)])),
            store,
        );
        let session_id =
            async_global_executor::block_on(app.lyrics_search(path.clone())).expect("search");
        let snapshot = wait_for_lyrics_status(&app, LyricsSearchStatus::Ready);
        let candidate_id = snapshot.candidates[0].id;
        app.lyrics_preview(session_id, candidate_id)
            .expect("preview");
        ReadyConfirm {
            app,
            music,
            store_root,
            audio,
            path,
            session_id,
            candidate_id,
        }
    }

    #[test]
    fn cancel_during_inflight_confirm_does_not_persist() {
        let ReadyConfirm {
            app,
            music,
            store_root,
            audio,
            path,
            session_id,
            candidate_id,
        } = ready_to_confirm("mofu-lyrics-app-cancel-inflight", "竞态词");
        app.lyrics_store().arm_confirm_hold();
        let confirm = {
            let app = app.clone();
            std::thread::spawn(move || {
                async_global_executor::block_on(app.lyrics_confirm(
                    session_id,
                    candidate_id,
                    LyricsConfirmAction::Save,
                ))
            })
        };
        app.lyrics_store().wait_confirm_hold_entered();
        app.lyrics_cancel(session_id).expect("cancel");
        app.lyrics_store().release_confirm_hold();
        let result = confirm.join().expect("confirm thread");
        assert!(
            matches!(
                result,
                Err(ApplicationError::Lyrics(
                    lyrics::LyricsSessionError::ConfirmAbandoned
                ))
            ),
            "unexpected confirm result: {result:?}"
        );
        assert!(!audio.with_extension("lrc").exists());
        assert!(!app.lyrics_store().has_override(&path));
        assert!(app.current_player_view().lyrics_session.is_none());
        let _ = std::fs::remove_dir_all(music);
        let _ = std::fs::remove_dir_all(store_root);
    }

    #[test]
    fn new_search_during_inflight_confirm_does_not_persist() {
        use lyrics::LyricsSearchStatus;

        let ReadyConfirm {
            app,
            music,
            store_root,
            audio,
            path,
            session_id,
            candidate_id,
        } = ready_to_confirm("mofu-lyrics-app-search-inflight", "竞态词");
        app.lyrics_store().arm_confirm_hold();
        let confirm = {
            let app = app.clone();
            std::thread::spawn(move || {
                async_global_executor::block_on(app.lyrics_confirm(
                    session_id,
                    candidate_id,
                    LyricsConfirmAction::Save,
                ))
            })
        };
        app.lyrics_store().wait_confirm_hold_entered();
        let new_session =
            async_global_executor::block_on(app.lyrics_search(path.clone())).expect("new search");
        app.lyrics_store().release_confirm_hold();
        let result = confirm.join().expect("confirm thread");
        assert!(
            matches!(
                result,
                Err(ApplicationError::Lyrics(
                    lyrics::LyricsSessionError::ConfirmAbandoned
                ))
            ),
            "unexpected confirm result: {result:?}"
        );
        assert!(!audio.with_extension("lrc").exists());
        assert!(!app.lyrics_store().has_override(&path));
        let session = wait_for_lyrics_status(&app, LyricsSearchStatus::Ready);
        assert_eq!(session.session_id, new_session);
        assert_ne!(new_session, session_id);
        let _ = std::fs::remove_dir_all(music);
        let _ = std::fs::remove_dir_all(store_root);
    }

    #[test]
    fn track_change_during_inflight_confirm_still_persists() {
        use lyrics::{FixtureLyricsSource, LyricsSearchSource, LyricsSearchStatus};

        let music = unique_test_dir("mofu-lyrics-app-change-inflight-music");
        let store_root = unique_test_dir("mofu-lyrics-app-change-inflight-store");
        let first = music.join("first.flac");
        let second = music.join("second.flac");
        std::fs::write(&first, b"a").unwrap();
        std::fs::write(&second, b"b").unwrap();
        let first_path = first.to_str().unwrap().to_owned();
        let second_path = second.to_str().unwrap().to_owned();
        let metadata = official_embedded("正式旧词");
        let player = FakePlayer::with_queue(
            &[first_path.as_str(), second_path.as_str()],
            0,
            metadata.clone(),
        );
        let mut collection = FakeCollection::with_song(&first_path, 195_000, metadata.clone());
        collection.push_song(&second_path, 180_000, official_embedded("第二首"));
        let app = Application::new_for_test_with_store(
            collection,
            player,
            StubDialog,
            LyricsSearchSource::Fixture(FixtureLyricsSource::ready(vec![timed_candidate(
                "切歌仍保存",
            )])),
            LyricsStore::open(store_root.clone()).expect("store"),
        );
        let session_id =
            async_global_executor::block_on(app.lyrics_search(first_path.clone())).expect("search");
        let snapshot = wait_for_lyrics_status(&app, LyricsSearchStatus::Ready);
        let candidate_id = snapshot.candidates[0].id;
        app.lyrics_preview(session_id, candidate_id)
            .expect("preview");
        // Record the first playing path so a later next() is a real track change,
        // not the "first observation" that leaves a non-current search alone.
        app.sync_lyrics_session_for_test();
        app.lyrics_store().arm_confirm_hold();
        let confirm = {
            let app = app.clone();
            std::thread::spawn(move || {
                async_global_executor::block_on(app.lyrics_confirm(
                    session_id,
                    candidate_id,
                    LyricsConfirmAction::Save,
                ))
            })
        };
        app.lyrics_store().wait_confirm_hold_entered();
        app.next().expect("change track");
        assert_eq!(
            app.current_player_view().current_path.as_deref(),
            Some(second_path.as_str())
        );
        assert!(app.current_player_view().lyrics_session.is_none());
        app.lyrics_store().release_confirm_hold();
        let outcome = confirm.join().expect("confirm thread").expect("saved");
        assert!(matches!(outcome, LyricsConfirmOutcome::Saved(_)));
        assert!(first.with_extension("lrc").exists());
        assert!(app.lyrics_store().has_override(&first_path));
        assert!(!second.with_extension("lrc").exists());
        let _ = std::fs::remove_dir_all(music);
        let _ = std::fs::remove_dir_all(store_root);
    }

    #[test]
    fn seek_and_pause_during_inflight_confirm_still_complete() {
        let ReadyConfirm {
            app,
            music,
            store_root,
            audio,
            path,
            session_id,
            candidate_id,
        } = ready_to_confirm("mofu-lyrics-app-seek-inflight", "边播边存");
        app.lyrics_store().arm_confirm_hold();
        let confirm = {
            let app = app.clone();
            std::thread::spawn(move || {
                async_global_executor::block_on(app.lyrics_confirm(
                    session_id,
                    candidate_id,
                    LyricsConfirmAction::Save,
                ))
            })
        };
        app.lyrics_store().wait_confirm_hold_entered();
        app.seek(12_345).expect("seek");
        app.pause().expect("pause");
        assert_eq!(app.current_player_view().position_ms, 12_345);
        app.lyrics_store().release_confirm_hold();
        let outcome = confirm.join().expect("confirm thread").expect("saved");
        assert!(matches!(outcome, LyricsConfirmOutcome::Saved(_)));
        assert!(audio.with_extension("lrc").exists());
        assert!(app.lyrics_store().has_override(&path));
        assert!(app.current_player_view().lyrics_session.is_none());
        let _ = std::fs::remove_dir_all(music);
        let _ = std::fs::remove_dir_all(store_root);
    }

    fn wait_for_lyrics_status<C, P, D>(
        app: &Application<C, P, D>,
        status: lyrics::LyricsSearchStatus,
    ) -> std::sync::Arc<lyrics::LyricsSessionSnapshot>
    where
        C: CollectionPort,
        P: PlayerPort,
        D: crate::file_dialog::FileDialog + Send + Sync + 'static,
    {
        for _ in 0..100 {
            if let Some(session) = app.current_player_view().lyrics_session.clone()
                && session.status == status
            {
                return session;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("timed out waiting for lyrics status {status:?}");
    }

    #[derive(Clone)]
    struct StubDialog;

    impl crate::file_dialog::FileDialog for StubDialog {
        async fn pick_directory(
            &self,
            _request: crate::file_dialog::DirectoryDialogRequest,
        ) -> crate::file_dialog::DialogSelection<PathBuf> {
            crate::file_dialog::DialogSelection::Cancelled
        }

        async fn pick_music_files(
            &self,
            _request: crate::file_dialog::MusicFilesDialogRequest,
        ) -> crate::file_dialog::DialogSelection<Vec<PathBuf>> {
            crate::file_dialog::DialogSelection::Cancelled
        }
    }

    struct FakePlayer {
        snapshot: Mutex<crate::player::PlayerSnapshot>,
    }

    impl FakePlayer {
        fn new(path: &str, metadata: crate::player::metadata::TrackMetadata) -> Self {
            Self::with_queue(&[path], 0, metadata)
        }

        fn with_queue(
            paths: &[&str],
            start: usize,
            metadata: crate::player::metadata::TrackMetadata,
        ) -> Self {
            use std::time::Duration;

            use crate::player::{
                PlaybackMode, PlaybackState, PlayerSnapshot, QueuePosition, QueueSnapshot,
            };
            Self {
                snapshot: Mutex::new(PlayerSnapshot {
                    position: Duration::ZERO,
                    duration: Duration::from_secs(3),
                    state: PlaybackState::Playing,
                    volume: 1.0,
                    queue: QueueSnapshot {
                        main_paths: paths.iter().map(PathBuf::from).collect(),
                        up_next_paths: Vec::new(),
                        current: Some(QueuePosition::Main(start)),
                        mode: PlaybackMode::Sequential,
                    },
                    metadata: Some(std::sync::Arc::new(metadata)),
                }),
            }
        }
    }

    impl PlayerPort for FakePlayer {
        fn set_volume(&self, volume: f32) -> Result<(), crate::player::PlayerError> {
            self.snapshot.lock().expect("player").volume = volume;
            Ok(())
        }
        fn set_playback_mode(
            &self,
            mode: crate::player::PlaybackMode,
        ) -> Result<(), crate::player::PlayerError> {
            self.snapshot.lock().expect("player").queue.mode = mode;
            Ok(())
        }
        fn set_queue(
            &self,
            paths: Vec<PathBuf>,
            start_index: Option<usize>,
            autoplay: bool,
        ) -> Result<(), crate::player::PlayerError> {
            let mut snapshot = self.snapshot.lock().expect("player");
            snapshot.queue.main_paths = paths;
            snapshot.queue.current = start_index.map(crate::player::QueuePosition::Main);
            if autoplay {
                snapshot.state = crate::player::PlaybackState::Playing;
            }
            Ok(())
        }
        fn play_index(&self, index: usize) -> Result<(), crate::player::PlayerError> {
            let mut snapshot = self.snapshot.lock().expect("player");
            if index >= snapshot.queue.main_paths.len() {
                return Err(crate::player::PlayerError::InvalidQueueIndex);
            }
            snapshot.queue.current = Some(crate::player::QueuePosition::Main(index));
            snapshot.state = crate::player::PlaybackState::Playing;
            Ok(())
        }
        fn play(&self) -> Result<(), crate::player::PlayerError> {
            self.snapshot.lock().expect("player").state = crate::player::PlaybackState::Playing;
            Ok(())
        }
        fn pause(&self) -> Result<(), crate::player::PlayerError> {
            self.snapshot.lock().expect("player").state = crate::player::PlaybackState::Paused;
            Ok(())
        }
        fn stop(&self) -> Result<(), crate::player::PlayerError> {
            let mut snapshot = self.snapshot.lock().expect("player");
            snapshot.state = crate::player::PlaybackState::Idle;
            snapshot.queue.current = None;
            Ok(())
        }
        fn next(&self) -> Result<(), crate::player::PlayerError> {
            let mut snapshot = self.snapshot.lock().expect("player");
            let Some(crate::player::QueuePosition::Main(index)) = snapshot.queue.current else {
                return Ok(());
            };
            let next = index + 1;
            if next < snapshot.queue.main_paths.len() {
                snapshot.queue.current = Some(crate::player::QueuePosition::Main(next));
            }
            Ok(())
        }
        fn previous(&self) -> Result<(), crate::player::PlayerError> {
            Ok(())
        }
        fn seek(&self, position: std::time::Duration) -> Result<(), crate::player::PlayerError> {
            self.snapshot.lock().expect("player").position = position;
            Ok(())
        }
        fn seek_relative(&self, _delta_ms: i64) -> Result<(), crate::player::PlayerError> {
            Ok(())
        }
        fn enqueue_next(&self, _path: PathBuf) -> Result<(), crate::player::PlayerError> {
            Ok(())
        }
        fn full_snapshot(&self) -> crate::player::PlayerSnapshot {
            self.snapshot.lock().expect("player").clone()
        }
        fn wait_for_progress(&self, _timeout: std::time::Duration) {}
        fn wake_progress_watch(&self) {}
        fn shutdown(&self) {}
    }

    struct FakeCollection {
        playlists: Vec<crate::collection::Playlist>,
    }

    impl FakeCollection {
        fn with_song(
            path: &str,
            duration_ms: u64,
            metadata: crate::player::metadata::TrackMetadata,
        ) -> Self {
            use crate::collection::{Playlist, PlaylistKind, Song};
            Self {
                playlists: vec![Playlist {
                    id: 1,
                    name: "library".into(),
                    kind: PlaylistKind::Virtual,
                    songs: vec![Song {
                        path: path.into(),
                        duration_ms,
                        metadata,
                    }],
                }],
            }
        }

        fn push_song(
            &mut self,
            path: &str,
            duration_ms: u64,
            metadata: crate::player::metadata::TrackMetadata,
        ) {
            self.playlists[0].songs.push(crate::collection::Song {
                path: path.into(),
                duration_ms,
                metadata,
            });
        }
    }

    impl CollectionPort for FakeCollection {
        async fn volume(&self) -> Result<f64, crate::collection::CollectionError> {
            Ok(1.0)
        }
        async fn set_volume(&self, _volume: f64) -> Result<(), crate::collection::CollectionError> {
            Ok(())
        }
        async fn playback_mode(
            &self,
        ) -> Result<crate::collection::StoredPlaybackMode, crate::collection::CollectionError>
        {
            Ok(crate::collection::StoredPlaybackMode::default())
        }
        async fn set_playback_mode(
            &self,
            _mode: crate::collection::StoredPlaybackMode,
        ) -> Result<(), crate::collection::CollectionError> {
            Ok(())
        }
        async fn cover_animation_in_background(
            &self,
        ) -> Result<crate::collection::CoverAnimationInBackground, crate::collection::CollectionError>
        {
            Ok(crate::collection::CoverAnimationInBackground::default())
        }
        async fn set_cover_animation_in_background(
            &self,
            _value: crate::collection::CoverAnimationInBackground,
        ) -> Result<(), crate::collection::CollectionError> {
            Ok(())
        }
        async fn playback_session(
            &self,
        ) -> Result<Option<crate::collection::PlaybackSession>, crate::collection::CollectionError>
        {
            Ok(None)
        }
        async fn set_playback_session(
            &self,
            _session: crate::collection::PlaybackSession,
        ) -> Result<(), crate::collection::CollectionError> {
            Ok(())
        }
        async fn clear_playback_session(&self) -> Result<(), crate::collection::CollectionError> {
            Ok(())
        }
        async fn resolve_playback_session(
            &self,
            _session: crate::collection::PlaybackSession,
        ) -> Result<Option<crate::collection::SessionRestorePlan>, crate::collection::CollectionError>
        {
            Ok(None)
        }
        async fn library_snapshot_lite(
            &self,
        ) -> Result<Vec<crate::collection::Playlist>, crate::collection::CollectionError> {
            Ok(self.playlists.clone())
        }
        async fn library_snapshot_with_covers(
            &self,
        ) -> Result<Vec<crate::collection::Playlist>, crate::collection::CollectionError> {
            Ok(self.playlists.clone())
        }
        async fn playlists(
            &self,
        ) -> Result<Vec<crate::collection::Playlist>, crate::collection::CollectionError> {
            Ok(self.playlists.clone())
        }
        async fn create_virtual(
            &self,
            _name: String,
        ) -> Result<crate::collection::Playlist, crate::collection::CollectionError> {
            Err(crate::collection::CollectionError::PlaylistNotFound)
        }
        async fn rename_virtual(
            &self,
            _id: crate::collection::PlaylistId,
            _name: String,
        ) -> Result<(), crate::collection::CollectionError> {
            Err(crate::collection::CollectionError::PlaylistNotFound)
        }
        async fn delete_virtual(
            &self,
            _id: crate::collection::PlaylistId,
        ) -> Result<(), crate::collection::CollectionError> {
            Err(crate::collection::CollectionError::PlaylistNotFound)
        }
        async fn refresh(
            &self,
            _id: crate::collection::PlaylistId,
        ) -> Result<crate::collection::Playlist, crate::collection::CollectionError> {
            Err(crate::collection::CollectionError::PlaylistNotFound)
        }
        async fn add_directory(
            &self,
            _path: PathBuf,
        ) -> Result<crate::collection::Playlist, crate::collection::CollectionError> {
            Err(crate::collection::CollectionError::PlaylistNotFound)
        }
        async fn ensure_virtual_import_target(
            &self,
            _selected_id: Option<crate::collection::PlaylistId>,
        ) -> Result<crate::collection::Playlist, crate::collection::CollectionError> {
            Err(crate::collection::CollectionError::PlaylistNotFound)
        }
        async fn add_files(
            &self,
            _id: crate::collection::PlaylistId,
            _paths: Vec<PathBuf>,
        ) -> Result<usize, crate::collection::CollectionError> {
            Ok(0)
        }
        async fn import_from_playlist(
            &self,
            _target_id: crate::collection::PlaylistId,
            _source_id: crate::collection::PlaylistId,
            _paths: Vec<String>,
        ) -> Result<usize, crate::collection::CollectionError> {
            Ok(0)
        }
    }
}
