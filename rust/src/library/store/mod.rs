mod catalog;
mod lyrics;
mod migrate_json;
mod reconnect;

pub(crate) use catalog::CatalogStore;
pub(crate) use lyrics::LyricsStore;
pub(crate) use migrate_json::JsonMigrator;

use super::cover_store::CoverStore;
use super::error::LibraryError;
use super::model::{CoverExt, PlaylistId, PlaylistKind, PlaylistRow, SongId, SongTags};
use crate::player::metadata::{CoverArt, Lyrics};

/// Catalog operations needed by library orchestration and JSON migration.
pub(crate) trait CatalogStoreApi {
    fn playlist_count(&self) -> impl Future<Output = Result<usize, LibraryError>> + Send;
    fn list_playlist_rows(
        &self,
    ) -> impl Future<Output = Result<Vec<PlaylistRow>, LibraryError>> + Send;
    fn delete_playlist(
        &self,
        id: PlaylistId,
    ) -> impl Future<Output = Result<(), LibraryError>> + Send;
    fn garbage_collect_orphan_songs(
        &self,
    ) -> impl Future<Output = Result<Vec<(SongId, Option<CoverExt>)>, LibraryError>> + Send;
    fn insert_playlist(
        &self,
        id: PlaylistId,
        name: &str,
        kind: &PlaylistKind,
    ) -> impl Future<Output = Result<(), LibraryError>> + Send;
    fn upsert_song(
        &self,
        path: &str,
        duration_ms: u64,
        tags: &SongTags,
    ) -> impl Future<Output = Result<SongId, LibraryError>> + Send;
    fn replace_playlist_songs(
        &self,
        playlist_id: PlaylistId,
        song_ids: &[SongId],
    ) -> impl Future<Output = Result<(), LibraryError>> + Send;
}

impl CatalogStoreApi for CatalogStore {
    async fn playlist_count(&self) -> Result<usize, LibraryError> {
        CatalogStore::playlist_count(self).await
    }

    async fn list_playlist_rows(&self) -> Result<Vec<PlaylistRow>, LibraryError> {
        CatalogStore::list_playlist_rows(self).await
    }

    async fn delete_playlist(&self, id: PlaylistId) -> Result<(), LibraryError> {
        CatalogStore::delete_playlist(self, id).await
    }

    async fn garbage_collect_orphan_songs(
        &self,
    ) -> Result<Vec<(SongId, Option<CoverExt>)>, LibraryError> {
        CatalogStore::garbage_collect_orphan_songs(self).await
    }

    async fn insert_playlist(
        &self,
        id: PlaylistId,
        name: &str,
        kind: &PlaylistKind,
    ) -> Result<(), LibraryError> {
        CatalogStore::insert_playlist(self, id, name, kind).await
    }

    async fn upsert_song(
        &self,
        path: &str,
        duration_ms: u64,
        tags: &SongTags,
    ) -> Result<SongId, LibraryError> {
        CatalogStore::upsert_song(self, path, duration_ms, tags).await
    }

    async fn replace_playlist_songs(
        &self,
        playlist_id: PlaylistId,
        song_ids: &[SongId],
    ) -> Result<(), LibraryError> {
        CatalogStore::replace_playlist_songs(self, playlist_id, song_ids).await
    }
}

/// Lyrics store operations needed by library orchestration and JSON migration.
pub(crate) trait LyricsStoreApi {
    fn replace_for_song(
        &self,
        song_id: SongId,
        lyrics: &[Lyrics],
    ) -> impl Future<Output = Result<(), LibraryError>> + Send;
    fn delete_for_songs(
        &self,
        song_ids: &[SongId],
    ) -> impl Future<Output = Result<(), LibraryError>> + Send;
}

impl LyricsStoreApi for LyricsStore {
    async fn replace_for_song(
        &self,
        song_id: SongId,
        lyrics: &[Lyrics],
    ) -> Result<(), LibraryError> {
        LyricsStore::replace_for_song(self, song_id, lyrics).await
    }

    async fn delete_for_songs(&self, song_ids: &[SongId]) -> Result<(), LibraryError> {
        LyricsStore::delete_for_songs(self, song_ids).await
    }
}

/// Cover cache operations needed by library orchestration and JSON migration.
pub(crate) trait CoverStoreApi {
    fn write(&self, song_id: SongId, cover: &CoverArt) -> Result<CoverExt, LibraryError>;
    fn remove(&self, song_id: SongId, ext: CoverExt);
    fn remove_all(&self, song_id: SongId);
}

impl CoverStoreApi for CoverStore {
    fn write(&self, song_id: SongId, cover: &CoverArt) -> Result<CoverExt, LibraryError> {
        CoverStore::write(self, song_id, cover)
    }

    fn remove(&self, song_id: SongId, ext: CoverExt) {
        CoverStore::remove(self, song_id, ext);
    }

    fn remove_all(&self, song_id: SongId) {
        CoverStore::remove_all(self, song_id);
    }
}
