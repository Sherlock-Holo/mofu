//! One-shot import from legacy `library.json` into Turso + cover cache.

use std::fs;
use std::path::Path;

use tracing::info;

use super::super::error::LibraryError;
use super::super::model::{LibraryManifest, SongTags};
use super::{CatalogStoreApi, CoverStoreApi, LyricsStoreApi};

const MANIFEST_FILE_NAME: &str = "library.json";
const MIGRATED_FILE_NAME: &str = "library.json.migrated";

/// Runs the legacy JSON → Turso migration when `library.json` is still present.
pub(crate) struct JsonMigrator;

impl JsonMigrator {
    /// Import legacy JSON when the manifest file still exists.
    ///
    /// Presence of `library.json` means migration is not finished (including after a
    /// partial import that left catalog rows behind). Wipe any existing catalog
    /// content, re-import, then rename the manifest to mark completion.
    pub(crate) async fn run(
        data_dir: &Path,
        catalog: &impl CatalogStoreApi,
        lyrics: &impl LyricsStoreApi,
        covers: &impl CoverStoreApi,
    ) -> Result<(), LibraryError> {
        let manifest_path = data_dir.join(MANIFEST_FILE_NAME);
        if !manifest_path.exists() {
            return Ok(());
        }

        Self::wipe_partial_state(catalog, lyrics, covers).await?;

        let manifest: LibraryManifest = serde_json::from_slice(&fs::read(&manifest_path)?)?;
        info!(
            playlists = manifest.playlists.len(),
            "migrating legacy library.json into turso stores"
        );

        for playlist in &manifest.playlists {
            catalog
                .insert_playlist(playlist.id, &playlist.name, &playlist.kind)
                .await?;
            let mut song_ids = Vec::with_capacity(playlist.songs.len());
            for song in &playlist.songs {
                let mut tags = SongTags::from_metadata(&song.metadata, None);
                let song_id = catalog
                    .upsert_song(&song.path, song.duration_ms, &tags)
                    .await?;
                if let Some(cover) = &song.metadata.cover {
                    let ext = covers.write(song_id, cover)?;
                    tags.cover_ext = Some(ext);
                    tags.cover_mime = cover.mime_type.clone();
                    catalog
                        .upsert_song(&song.path, song.duration_ms, &tags)
                        .await?;
                }
                lyrics
                    .replace_for_song(song_id, &song.metadata.lyrics)
                    .await?;
                song_ids.push(song_id);
            }
            catalog
                .replace_playlist_songs(playlist.id, &song_ids)
                .await?;
        }

        let migrated = data_dir.join(MIGRATED_FILE_NAME);
        fs::rename(&manifest_path, &migrated)?;
        Ok(())
    }

    /// Clear playlists then orphan songs/lyrics/covers left by a failed prior attempt.
    async fn wipe_partial_state(
        catalog: &impl CatalogStoreApi,
        lyrics: &impl LyricsStoreApi,
        covers: &impl CoverStoreApi,
    ) -> Result<(), LibraryError> {
        if catalog.playlist_count().await? > 0 {
            let rows = catalog.list_playlist_rows().await?;
            for row in rows {
                catalog.delete_playlist(row.id).await?;
            }
        }
        // Always GC: a partial import may leave songs with no playlist rows.
        let orphans = catalog.garbage_collect_orphan_songs().await?;
        if orphans.is_empty() {
            return Ok(());
        }
        let ids = orphans.iter().map(|(id, _)| *id).collect::<Vec<_>>();
        lyrics.delete_for_songs(&ids).await?;
        for (song_id, cover_ext) in orphans {
            if let Some(ext) = cover_ext {
                covers.remove(song_id, ext);
            } else {
                covers.remove_all(song_id);
            }
        }
        Ok(())
    }
}
