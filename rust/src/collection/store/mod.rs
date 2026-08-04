//! Catalog persistence (`catalog.db`) behind the [`Catalog`] trait.

mod catalog;
mod models;

pub(crate) use catalog::CatalogStore;

use crate::collection::error::CollectionError;
use crate::collection::model::{
    CoverFileType, PlaybackSession, PlaylistId, PlaylistKind, PlaylistRow, SongId, SongRow,
    SongTags, StoredPlaybackMode,
};

/// Persistence boundary for playlists, songs, and playback meta in `catalog.db`.
pub(crate) trait Catalog: Send + Sync {
    fn next_playlist_id(&self) -> impl Future<Output = Result<PlaylistId, CollectionError>> + Send;

    fn list_playlist_rows(
        &self,
    ) -> impl Future<Output = Result<Vec<PlaylistRow>, CollectionError>> + Send;

    fn playlist_row(
        &self,
        id: PlaylistId,
    ) -> impl Future<Output = Result<PlaylistRow, CollectionError>> + Send;

    fn songs_for_playlist(
        &self,
        playlist_id: PlaylistId,
    ) -> impl Future<Output = Result<Vec<SongRow>, CollectionError>> + Send;

    fn song_rows_for_paths(
        &self,
        paths: &[String],
    ) -> impl Future<Output = Result<Vec<SongRow>, CollectionError>> + Send;

    fn insert_playlist(
        &self,
        id: PlaylistId,
        name: &str,
        kind: &PlaylistKind,
    ) -> impl Future<Output = Result<(), CollectionError>> + Send;

    fn find_directory_playlist_id(
        &self,
        directory_path: &str,
    ) -> impl Future<Output = Result<Option<PlaylistId>, CollectionError>> + Send;

    fn rename_playlist(
        &self,
        id: PlaylistId,
        name: &str,
    ) -> impl Future<Output = Result<(), CollectionError>> + Send;

    fn delete_playlist(
        &self,
        id: PlaylistId,
    ) -> impl Future<Output = Result<(), CollectionError>> + Send;

    fn upsert_song(
        &self,
        path: &str,
        duration_ms: u64,
        tags: &SongTags,
    ) -> impl Future<Output = Result<SongId, CollectionError>> + Send;

    fn attach_song(
        &self,
        playlist_id: PlaylistId,
        song_id: SongId,
        position: i64,
    ) -> impl Future<Output = Result<(), CollectionError>> + Send;

    fn replace_playlist_songs(
        &self,
        playlist_id: PlaylistId,
        song_ids: &[SongId],
    ) -> impl Future<Output = Result<(), CollectionError>> + Send;

    fn playlist_song_paths(
        &self,
        playlist_id: PlaylistId,
    ) -> impl Future<Output = Result<Vec<String>, CollectionError>> + Send;

    fn garbage_collect_orphan_songs(
        &self,
    ) -> impl Future<Output = Result<Vec<(SongId, Option<CoverFileType>)>, CollectionError>> + Send;

    fn max_position(
        &self,
        playlist_id: PlaylistId,
    ) -> impl Future<Output = Result<i64, CollectionError>> + Send;

    fn playback_mode(
        &self,
    ) -> impl Future<Output = Result<StoredPlaybackMode, CollectionError>> + Send;

    fn set_playback_mode(
        &self,
        mode: StoredPlaybackMode,
    ) -> impl Future<Output = Result<(), CollectionError>> + Send;

    fn volume(&self) -> impl Future<Output = Result<f64, CollectionError>> + Send;

    fn set_volume(&self, volume: f64) -> impl Future<Output = Result<(), CollectionError>> + Send;

    fn playback_session(
        &self,
    ) -> impl Future<Output = Result<Option<PlaybackSession>, CollectionError>> + Send;

    fn set_playback_session(
        &self,
        session: &PlaybackSession,
    ) -> impl Future<Output = Result<(), CollectionError>> + Send;

    fn clear_playback_session(&self) -> impl Future<Output = Result<(), CollectionError>> + Send;
}
