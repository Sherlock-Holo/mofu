use std::path::PathBuf;
use std::time::Duration;

use crate::collection::{
    Collection, CollectionError, CoverAnimationInBackground, PlaybackSession, Playlist, PlaylistId,
    SessionRestorePlan, StoredPlaybackMode,
};
use crate::player::{PlaybackMode, Player, PlayerError, PlayerSnapshot};

/// Production collection adapter kept behind the application port.
#[flutter_rust_bridge::frb(ignore)]
pub(crate) struct CollectionAdapter {
    inner: Collection,
}

impl CollectionAdapter {
    pub(crate) async fn open_default() -> Result<Self, CollectionError> {
        Ok(Self {
            inner: Collection::open_default().await?,
        })
    }
}

/// Collection operations required by application orchestration.
#[flutter_rust_bridge::frb(ignore)]
pub(crate) trait CollectionPort: Send + Sync + 'static {
    fn volume(&self) -> impl Future<Output = Result<f64, CollectionError>> + Send;
    fn set_volume(&self, volume: f64) -> impl Future<Output = Result<(), CollectionError>> + Send;
    fn playback_mode(
        &self,
    ) -> impl Future<Output = Result<StoredPlaybackMode, CollectionError>> + Send;
    fn set_playback_mode(
        &self,
        mode: StoredPlaybackMode,
    ) -> impl Future<Output = Result<(), CollectionError>> + Send;
    fn cover_animation_in_background(
        &self,
    ) -> impl Future<Output = Result<CoverAnimationInBackground, CollectionError>> + Send;
    fn set_cover_animation_in_background(
        &self,
        value: CoverAnimationInBackground,
    ) -> impl Future<Output = Result<(), CollectionError>> + Send;
    fn playback_session(
        &self,
    ) -> impl Future<Output = Result<Option<PlaybackSession>, CollectionError>> + Send;
    fn set_playback_session(
        &self,
        session: PlaybackSession,
    ) -> impl Future<Output = Result<(), CollectionError>> + Send;
    fn clear_playback_session(&self) -> impl Future<Output = Result<(), CollectionError>> + Send;
    fn resolve_playback_session(
        &self,
        session: PlaybackSession,
    ) -> impl Future<Output = Result<Option<SessionRestorePlan>, CollectionError>> + Send;
    fn library_snapshot_lite(
        &self,
    ) -> impl Future<Output = Result<Vec<Playlist>, CollectionError>> + Send;
    fn library_snapshot_with_covers(
        &self,
    ) -> impl Future<Output = Result<Vec<Playlist>, CollectionError>> + Send;
    fn playlists(&self) -> impl Future<Output = Result<Vec<Playlist>, CollectionError>> + Send;
    fn create_virtual(
        &self,
        name: String,
    ) -> impl Future<Output = Result<Playlist, CollectionError>> + Send;
    fn rename_virtual(
        &self,
        id: PlaylistId,
        name: String,
    ) -> impl Future<Output = Result<(), CollectionError>> + Send;
    fn delete_virtual(
        &self,
        id: PlaylistId,
    ) -> impl Future<Output = Result<(), CollectionError>> + Send;
    fn refresh(
        &self,
        id: PlaylistId,
    ) -> impl Future<Output = Result<Playlist, CollectionError>> + Send;
    fn add_directory(
        &self,
        path: PathBuf,
    ) -> impl Future<Output = Result<Playlist, CollectionError>> + Send;
    fn ensure_virtual_import_target(
        &self,
        selected_id: Option<PlaylistId>,
    ) -> impl Future<Output = Result<Playlist, CollectionError>> + Send;
    fn add_files(
        &self,
        id: PlaylistId,
        paths: Vec<PathBuf>,
    ) -> impl Future<Output = Result<usize, CollectionError>> + Send;
    fn import_from_playlist(
        &self,
        target_id: PlaylistId,
        source_id: PlaylistId,
        paths: Vec<String>,
    ) -> impl Future<Output = Result<usize, CollectionError>> + Send;
}

impl CollectionPort for CollectionAdapter {
    async fn volume(&self) -> Result<f64, CollectionError> {
        self.inner.volume().await
    }

    async fn set_volume(&self, volume: f64) -> Result<(), CollectionError> {
        self.inner.set_volume(volume).await
    }

    async fn playback_mode(&self) -> Result<StoredPlaybackMode, CollectionError> {
        self.inner.playback_mode().await
    }

    async fn set_playback_mode(&self, mode: StoredPlaybackMode) -> Result<(), CollectionError> {
        self.inner.set_playback_mode(mode).await
    }

    async fn cover_animation_in_background(
        &self,
    ) -> Result<CoverAnimationInBackground, CollectionError> {
        self.inner.cover_animation_in_background().await
    }

    async fn set_cover_animation_in_background(
        &self,
        value: CoverAnimationInBackground,
    ) -> Result<(), CollectionError> {
        self.inner.set_cover_animation_in_background(value).await
    }

    async fn playback_session(&self) -> Result<Option<PlaybackSession>, CollectionError> {
        self.inner.playback_session().await
    }

    async fn set_playback_session(&self, session: PlaybackSession) -> Result<(), CollectionError> {
        self.inner.set_playback_session(session).await
    }

    async fn clear_playback_session(&self) -> Result<(), CollectionError> {
        self.inner.clear_playback_session().await
    }

    async fn resolve_playback_session(
        &self,
        session: PlaybackSession,
    ) -> Result<Option<SessionRestorePlan>, CollectionError> {
        self.inner.resolve_playback_session(session).await
    }

    async fn library_snapshot_lite(&self) -> Result<Vec<Playlist>, CollectionError> {
        self.inner.library_snapshot_lite().await
    }

    async fn library_snapshot_with_covers(&self) -> Result<Vec<Playlist>, CollectionError> {
        self.inner.library_snapshot_with_covers().await
    }

    async fn playlists(&self) -> Result<Vec<Playlist>, CollectionError> {
        self.inner.playlists().await
    }

    async fn create_virtual(&self, name: String) -> Result<Playlist, CollectionError> {
        self.inner.create_virtual(name).await
    }

    async fn rename_virtual(&self, id: PlaylistId, name: String) -> Result<(), CollectionError> {
        self.inner.rename_virtual(id, name).await
    }

    async fn delete_virtual(&self, id: PlaylistId) -> Result<(), CollectionError> {
        self.inner.delete_virtual(id).await
    }

    async fn refresh(&self, id: PlaylistId) -> Result<Playlist, CollectionError> {
        self.inner.refresh(id).await
    }

    async fn add_directory(&self, path: PathBuf) -> Result<Playlist, CollectionError> {
        self.inner.add_directory(path).await
    }

    async fn ensure_virtual_import_target(
        &self,
        selected_id: Option<PlaylistId>,
    ) -> Result<Playlist, CollectionError> {
        self.inner.ensure_virtual_import_target(selected_id).await
    }

    async fn add_files(
        &self,
        id: PlaylistId,
        paths: Vec<PathBuf>,
    ) -> Result<usize, CollectionError> {
        self.inner.add_files(id, paths).await
    }

    async fn import_from_playlist(
        &self,
        target_id: PlaylistId,
        source_id: PlaylistId,
        paths: Vec<String>,
    ) -> Result<usize, CollectionError> {
        self.inner
            .import_from_playlist(target_id, source_id, paths)
            .await
    }
}

/// Production player adapter kept behind the application port.
#[flutter_rust_bridge::frb(ignore)]
pub(crate) struct PlayerAdapter {
    inner: Player,
}

impl PlayerAdapter {
    pub(crate) fn new() -> Self {
        Self {
            inner: Player::new(),
        }
    }
}

/// Player operations required by application orchestration.
#[flutter_rust_bridge::frb(ignore)]
pub(crate) trait PlayerPort: Send + Sync + 'static {
    fn set_volume(&self, volume: f32) -> Result<(), PlayerError>;
    fn set_playback_mode(&self, mode: PlaybackMode) -> Result<(), PlayerError>;
    fn set_queue(
        &self,
        paths: Vec<PathBuf>,
        start_index: Option<usize>,
        autoplay: bool,
    ) -> Result<(), PlayerError>;
    fn play_index(&self, index: usize) -> Result<(), PlayerError>;
    fn play(&self) -> Result<(), PlayerError>;
    fn pause(&self) -> Result<(), PlayerError>;
    fn stop(&self) -> Result<(), PlayerError>;
    fn next(&self) -> Result<(), PlayerError>;
    fn previous(&self) -> Result<(), PlayerError>;
    fn seek(&self, position: Duration) -> Result<(), PlayerError>;
    fn seek_relative(&self, delta_ms: i64) -> Result<(), PlayerError>;
    fn enqueue_next(&self, path: PathBuf) -> Result<(), PlayerError>;
    fn full_snapshot(&self) -> PlayerSnapshot;
    fn wait_for_progress(&self, timeout: Duration);
    fn wake_progress_watch(&self);
    fn shutdown(&self);
}

impl PlayerPort for PlayerAdapter {
    fn set_volume(&self, volume: f32) -> Result<(), PlayerError> {
        self.inner.set_volume(volume)
    }

    fn set_playback_mode(&self, mode: PlaybackMode) -> Result<(), PlayerError> {
        self.inner.set_playback_mode(mode)
    }

    fn set_queue(
        &self,
        paths: Vec<PathBuf>,
        start_index: Option<usize>,
        autoplay: bool,
    ) -> Result<(), PlayerError> {
        self.inner.set_queue(paths, start_index, autoplay)
    }

    fn play_index(&self, index: usize) -> Result<(), PlayerError> {
        self.inner.play_index(index)
    }

    fn play(&self) -> Result<(), PlayerError> {
        self.inner.play()
    }

    fn pause(&self) -> Result<(), PlayerError> {
        self.inner.pause()
    }

    fn stop(&self) -> Result<(), PlayerError> {
        self.inner.stop()
    }

    fn next(&self) -> Result<(), PlayerError> {
        self.inner.next()
    }

    fn previous(&self) -> Result<(), PlayerError> {
        self.inner.previous()
    }

    fn seek(&self, position: Duration) -> Result<(), PlayerError> {
        self.inner.seek(position)
    }

    fn seek_relative(&self, delta_ms: i64) -> Result<(), PlayerError> {
        self.inner.seek_relative(delta_ms)
    }

    fn enqueue_next(&self, path: PathBuf) -> Result<(), PlayerError> {
        self.inner.enqueue_next(path)
    }

    fn full_snapshot(&self) -> PlayerSnapshot {
        self.inner.full_snapshot()
    }

    fn wait_for_progress(&self, timeout: Duration) {
        self.inner.wait_for_progress(timeout);
    }

    fn wake_progress_watch(&self) {
        self.inner.wake_progress_watch();
    }

    fn shutdown(&self) {
        self.inner.shutdown();
    }
}
