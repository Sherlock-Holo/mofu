//! Application orchestration: bootstrap, persistence, and state fan-out.

mod dto;
mod ports;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use async_global_executor::Task;
pub(crate) use dto::{
    BootstrapSnapshot, LibrarySnapshot, PlayerView, PlayerViewKind, PlaylistView, SongView,
};
use flume::{Receiver, Sender, TrySendError};
use ports::{CollectionAdapter, CollectionPort, PlayerAdapter, PlayerPort};
use tracing::{debug, warn};

use crate::collection::{
    CollectionError, PlaybackSession, PlaylistId, SessionRestorePlan, StoredPlaybackMode,
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
    player_events: Mutex<SnapshotBus<PlayerView>>,
    library_events: Mutex<SnapshotBus<LibrarySnapshot>>,
    persist_tx: Sender<PersistCommand>,
    shutdown: Arc<AtomicBool>,
    cover_generation: AtomicU64,
    watch_thread: Mutex<Option<JoinHandle<()>>>,
    persist_task: Mutex<Option<Task<()>>>,
    cover_task: Mutex<Option<Task<()>>>,
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
    Volume(f64),
    Flush {
        volume: f64,
        mode: PlaybackMode,
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
        let player = PlayerAdapter::new();
        // Persistence traffic is already coalesced/debounced and must never be dropped.
        let (persist_tx, persist_rx) = flume::unbounded();
        let shutdown = Arc::new(AtomicBool::new(false));

        let app = Arc::new(Self {
            player,
            collection,
            dialog: RfdFileDialog::default(),
            source_playlist_id: Mutex::new(None),
            volume_state: Mutex::new(VolumePersistState::default()),
            player_events: Mutex::new(SnapshotBus::default()),
            library_events: Mutex::new(SnapshotBus::default()),
            persist_tx,
            shutdown,
            cover_generation: AtomicU64::new(0),
            watch_thread: Mutex::new(None),
            persist_task: Mutex::new(None),
            cover_task: Mutex::new(None),
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
            warn!(%err, "playback session restore failed");
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
                PersistCommand::Volume(volume) => {
                    if let Err(err) = self.collection.set_volume(volume).await {
                        warn!(%err, "persist volume failed");
                    }
                }
                PersistCommand::Flush {
                    volume,
                    mode,
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
        let mut last_key = SessionWatchKey::from_view(&self.player_view());
        let mut last_view = self.player_view();
        loop {
            if self.shutdown.load(Ordering::Acquire) {
                break;
            }
            self.player.wait_for_progress(PLAYER_PROGRESS_INTERVAL);
            self.tick_volume_debounce();

            let view = self.player_view();
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
        let source = *self
            .source_playlist_id
            .lock()
            .expect("source playlist mutex");
        PlayerView::from_domain(self.player.full_snapshot(), source)
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
        Ok(self.player.stop()?)
    }

    pub(crate) fn next(&self) -> Result<(), ApplicationError> {
        Ok(self.player.next()?)
    }

    pub(crate) fn previous(&self) -> Result<(), ApplicationError> {
        Ok(self.player.previous()?)
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

    pub(crate) fn flush(&self) -> bool {
        let mut state = self.volume_state.lock().expect("volume state mutex");
        state.pending = None;
        state.due_at = None;
        drop(state);

        let view = self.player_view();
        let volume = view.volume;
        let mode = view.playback_mode;
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
        self.player.shutdown();
    }
}

impl From<StoredPlaybackMode> for PlaybackMode {
    fn from(mode: StoredPlaybackMode) -> Self {
        match mode {
            StoredPlaybackMode::Sequential => Self::Sequential,
            StoredPlaybackMode::Shuffle => Self::Shuffle,
            StoredPlaybackMode::RepeatOne => Self::RepeatOne,
            StoredPlaybackMode::RepeatAll => Self::RepeatAll,
        }
    }
}

impl From<PlaybackMode> for StoredPlaybackMode {
    fn from(mode: PlaybackMode) -> Self {
        match mode {
            PlaybackMode::Sequential => Self::Sequential,
            PlaybackMode::Shuffle => Self::Shuffle,
            PlaybackMode::RepeatOne => Self::RepeatOne,
            PlaybackMode::RepeatAll => Self::RepeatAll,
        }
    }
}

#[cfg(test)]
mod tests {
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
}
