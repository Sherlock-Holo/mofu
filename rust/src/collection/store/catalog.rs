//! `catalog.db`: playlists, songs (tags only), playlist membership.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::Duration;

use async_compat::CompatExt;
use futures_timer::Delay;
use toasty::Db;
use toasty::db::Executor;
use toasty_driver_turso::Turso;
use tracing::{error, warn};

use super::Catalog;
use super::models::{Meta, Playlist, PlaylistSong, Song, StoredCoverFileType, StoredPlaylistKind};
use crate::collection::error::{CatalogDataError, CatalogNumericField, CollectionError};
use crate::collection::model::{
    CoverAnimationInBackground, CoverFileType, PlaybackSession, PlaylistId, PlaylistKind,
    PlaylistRow, SongId, SongRow, SongTags, StoredPlaybackMode,
};

const SCHEMA_VERSION: &str = "1";
const PLAYBACK_MODE_KEY: &str = "playback_mode";
const COVER_ANIMATION_IN_BACKGROUND_KEY: &str = "cover_animation_in_background";
const VOLUME_KEY: &str = "volume";
const PLAYBACK_SESSION_KEY: &str = "playback_session";
const DEFAULT_VOLUME: f64 = 1.0;
/// Same-op retries after a serialization / MVCC conflict.
const MAX_SERIALIZATION_RETRIES: usize = 20;
const SERIALIZATION_RETRY_DELAY: Duration = Duration::from_millis(10);
/// Keep `IN` lists under typical SQLite/Turso variable limits.
const IN_LIST_BATCH: usize = 200;

/// `CREATE TABLE/INDEX IF NOT EXISTS` so partial schemas and open races are safe.
/// Kept in lockstep with [`super::models`].
const SCHEMA_STATEMENTS: &[&str] = &[
    r#"
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
)
"#,
    r#"
CREATE TABLE IF NOT EXISTS playlists (
    id INTEGER PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    directory_path TEXT
)
"#,
    r#"
CREATE TABLE IF NOT EXISTS songs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL UNIQUE,
    duration_ms INTEGER NOT NULL,
    title TEXT,
    artist TEXT,
    album TEXT,
    album_artist TEXT,
    composer TEXT,
    lyricist TEXT,
    performer TEXT,
    genre TEXT,
    comment TEXT,
    date TEXT,
    year INTEGER,
    track_number INTEGER,
    track_total INTEGER,
    disc_number INTEGER,
    disc_total INTEGER,
    cover_mime TEXT,
    cover_ext TEXT
)
"#,
    r#"
CREATE TABLE IF NOT EXISTS playlist_songs (
    playlist_id INTEGER NOT NULL,
    song_id INTEGER NOT NULL,
    position INTEGER NOT NULL,
    PRIMARY KEY (playlist_id, song_id),
    UNIQUE (playlist_id, position)
)
"#,
    "CREATE INDEX IF NOT EXISTS idx_playlist_songs_playlist_id ON playlist_songs (playlist_id)",
    "CREATE INDEX IF NOT EXISTS idx_playlist_songs_song_id ON playlist_songs (song_id)",
];

/// Catalog database store backed by Toasty + Turso.
pub(crate) struct CatalogStore {
    db: Db,
}

impl CatalogStore {
    pub(crate) async fn open(path: &Path) -> Result<Self, CollectionError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let path = path.to_path_buf();
        async move {
            let driver = Turso::file(&path).concurrent_writes();
            let mut db = Db::builder()
                .models(toasty::models!(Meta, Playlist, Song, PlaylistSong))
                .build(driver)
                .await?;
            Self::ensure_schema(&mut db).await?;
            // Seed schema version without overwriting an existing value.
            Meta::upsert_by_key("schema_version")
                .value(SCHEMA_VERSION)
                .or_ignore()
                .exec(&mut db)
                .await?;
            Ok(Self { db })
        }
        .compat()
        .await
    }

    /// Ensure catalog tables/indexes exist (`IF NOT EXISTS`, race-safe).
    async fn ensure_schema(db: &mut Db) -> Result<(), CollectionError> {
        for sql in SCHEMA_STATEMENTS {
            toasty::sql::statement(*sql).exec(db).await?;
        }
        Ok(())
    }

    /// Run `op` under async-compat Tokio context, retrying serialization failures.
    ///
    /// Each attempt receives an owned [`Db`] clone so the future does not borrow
    /// across the retry loop. `op` is an [`AsyncFnMut`] whose captures can be
    /// reused on retry.
    async fn run<T, F>(&self, mut op: F) -> Result<T, CollectionError>
    where
        F: AsyncFnMut(Db) -> Result<T, CollectionError>,
    {
        async {
            for attempt in 0..MAX_SERIALIZATION_RETRIES {
                match op(self.db.clone()).await {
                    Ok(value) => return Ok(value),
                    Err(err) if err.is_serialization_failure() => {
                        warn!(
                            %err,
                            attempt,
                            "catalog serialization failure; retrying"
                        );
                        Delay::new(SERIALIZATION_RETRY_DELAY).await;
                    }
                    Err(err) => return Err(err),
                }
            }
            op(self.db.clone()).await
        }
        .compat()
        .await
    }

    #[cfg(test)]
    pub(crate) async fn upsert_meta_raw(
        &self,
        key: &str,
        value: &str,
    ) -> Result<(), CollectionError> {
        let key = key.to_owned();
        let value = value.to_owned();
        self.run(async move |mut db| {
            let key = key.clone();
            let value = value.clone();
            Meta::upsert_by_key(key).value(value).exec(&mut db).await?;
            Ok(())
        })
        .await
    }

    #[cfg(test)]
    pub(crate) async fn exec_raw_statement(&self, sql: &str) -> Result<(), CollectionError> {
        let sql = sql.to_owned();
        self.run(async move |mut db| {
            let sql = sql.clone();
            toasty::sql::statement(sql).exec(&mut db).await?;
            Ok(())
        })
        .await
    }

    #[cfg(test)]
    pub(crate) async fn insert_song_raw(
        &self,
        path: &str,
        duration_ms: i64,
        year: Option<i64>,
    ) -> Result<(), CollectionError> {
        let path = path.to_owned();
        self.run(async move |mut db| {
            let path = path.clone();
            let mut statement = toasty::sql::statement(
                "INSERT INTO songs (path, duration_ms, year) VALUES (?1, ?2, ?3)",
            )
            .bind(path)
            .bind(duration_ms);
            statement = match year {
                Some(year) => statement.bind(year),
                None => statement.bind_typed(
                    toasty::stmt::Value::Null,
                    toasty::schema::db::Type::Integer(8),
                ),
            };
            statement.exec(&mut db).await?;
            Ok(())
        })
        .await
    }

    async fn songs_by_ids(
        executor: &mut dyn Executor,
        song_ids: &[SongId],
    ) -> Result<HashMap<SongId, Song>, CollectionError> {
        let mut songs_by_id = HashMap::new();
        if song_ids.is_empty() {
            return Ok(songs_by_id);
        }
        for chunk in song_ids.chunks(IN_LIST_BATCH) {
            let rows = Song::filter(Song::fields().id().in_list(chunk.to_vec()))
                .exec(executor)
                .await?;
            for song in rows {
                songs_by_id.insert(song.id, song);
            }
        }
        Ok(songs_by_id)
    }
}

impl TryFrom<Playlist> for PlaylistRow {
    type Error = CollectionError;

    fn try_from(row: Playlist) -> Result<Self, Self::Error> {
        let kind = match row.kind {
            StoredPlaylistKind::Directory => match row.directory_path {
                Some(path) => PlaylistKind::Directory { path },
                None => return Err(CatalogDataError::DirectoryPlaylistMissingPath.into()),
            },
            StoredPlaylistKind::Virtual => PlaylistKind::Virtual,
        };
        Ok(PlaylistRow {
            id: row.id as PlaylistId,
            name: row.name,
            kind,
        })
    }
}

impl TryFrom<&Song> for SongRow {
    type Error = CollectionError;

    fn try_from(row: &Song) -> Result<Self, Self::Error> {
        Ok(SongRow {
            id: row.id,
            path: row.file_path.clone(),
            duration_ms: CatalogNumericField::DurationMs.u64_from_i64(row.duration_ms)?,
            tags: SongTags {
                title: row.title.clone(),
                artist: row.artist.clone(),
                album: row.album.clone(),
                album_artist: row.album_artist.clone(),
                composer: row.composer.clone(),
                lyricist: row.lyricist.clone(),
                performer: row.performer.clone(),
                genre: row.genre.clone(),
                comment: row.comment.clone(),
                date: row.date.clone(),
                year: CatalogNumericField::Year.opt_u16_from_i64(row.year)?,
                track_number: CatalogNumericField::TrackNumber
                    .opt_u64_from_i64(row.track_number)?,
                track_total: CatalogNumericField::TrackTotal.opt_u64_from_i64(row.track_total)?,
                disc_number: CatalogNumericField::DiscNumber.opt_u64_from_i64(row.disc_number)?,
                disc_total: CatalogNumericField::DiscTotal.opt_u64_from_i64(row.disc_total)?,
                cover_mime: row.cover_mime.clone(),
                cover_file_type: row.cover_file_type.map(CoverFileType::from),
            },
        })
    }
}

impl Catalog for CatalogStore {
    async fn next_playlist_id(&self) -> Result<PlaylistId, CollectionError> {
        self.run(async |mut db| {
            let max = Playlist::all()
                .order_by(Playlist::fields().id().desc())
                .first()
                .exec(&mut db)
                .await?;
            let max_id = max.map(|playlist| playlist.id as PlaylistId).unwrap_or(0);
            Ok(max_id.saturating_add(1).max(1))
        })
        .await
    }

    async fn list_playlist_rows(&self) -> Result<Vec<PlaylistRow>, CollectionError> {
        self.run(async |mut db| {
            let rows = Playlist::all()
                .order_by(Playlist::fields().id().asc())
                .exec(&mut db)
                .await?;
            rows.into_iter()
                .map(PlaylistRow::try_from)
                .collect::<Result<Vec<_>, _>>()
        })
        .await
    }

    async fn playlist_row(&self, id: PlaylistId) -> Result<PlaylistRow, CollectionError> {
        self.run(
            async move |mut db| match Playlist::get_by_id(&mut db, id as i64).await {
                Ok(row) => PlaylistRow::try_from(row),
                Err(err) if err.is_record_not_found() => Err(CollectionError::PlaylistNotFound),
                Err(err) => Err(err.into()),
            },
        )
        .await
    }

    async fn songs_for_playlist(
        &self,
        playlist_id: PlaylistId,
    ) -> Result<Vec<SongRow>, CollectionError> {
        self.run(async move |mut db| {
            let mut tx = db.transaction().await?;
            let memberships = PlaylistSong::filter_by_playlist_id(playlist_id as i64)
                .order_by(PlaylistSong::fields().position().asc())
                .exec(&mut tx)
                .await?;
            let song_ids = memberships
                .iter()
                .map(|membership| membership.song_id)
                .collect::<Vec<_>>();
            let songs_by_id = Self::songs_by_ids(&mut tx, &song_ids).await?;
            let mut songs = Vec::with_capacity(memberships.len());
            for membership in memberships {
                let song_id = membership.song_id;
                let Some(song) = songs_by_id.get(&song_id) else {
                    error!(
                        playlist_id,
                        song_id, "playlist membership references missing song"
                    );
                    continue;
                };
                songs.push(SongRow::try_from(song)?);
            }
            tx.commit().await?;
            Ok(songs)
        })
        .await
    }

    async fn song_rows_for_paths(&self, paths: &[String]) -> Result<Vec<SongRow>, CollectionError> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let paths = paths.to_vec();
        self.run(async move |mut db| {
            let paths = paths.clone();
            let mut songs = Vec::new();
            for chunk in paths.chunks(IN_LIST_BATCH) {
                let rows = Song::filter(Song::fields().file_path().in_list(chunk.to_vec()))
                    .exec(&mut db)
                    .await?;
                for row in rows {
                    songs.push(SongRow::try_from(&row)?);
                }
            }
            Ok(songs)
        })
        .await
    }

    async fn insert_playlist(
        &self,
        id: PlaylistId,
        name: &str,
        kind: &PlaylistKind,
    ) -> Result<(), CollectionError> {
        let (stored_kind, directory_path) = kind.into();
        let name = name.to_owned();
        self.run(async move |mut db| {
            let name = name.clone();
            let directory_path = directory_path.clone();
            toasty::create!(Playlist {
                id: id as i64,
                name,
                kind: stored_kind,
                directory_path,
            })
            .exec(&mut db)
            .await?;
            Ok(())
        })
        .await
    }

    async fn find_directory_playlist_id(
        &self,
        directory_path: &str,
    ) -> Result<Option<PlaylistId>, CollectionError> {
        let directory_path = directory_path.to_owned();
        self.run(async move |mut db| {
            let directory_path = directory_path.clone();
            let row = Playlist::filter(
                Playlist::fields()
                    .kind()
                    .eq(StoredPlaylistKind::Directory)
                    .and(Playlist::fields().directory_path().eq(&directory_path)),
            )
            .first()
            .exec(&mut db)
            .await?;
            Ok(row.map(|playlist| playlist.id as PlaylistId))
        })
        .await
    }

    async fn rename_playlist(&self, id: PlaylistId, name: &str) -> Result<(), CollectionError> {
        let name = name.to_owned();
        self.run(async move |mut db| {
            let name = name.clone();
            match Playlist::get_by_id(&mut db, id as i64).await {
                Ok(mut playlist) => {
                    playlist.update().name(name).exec(&mut db).await?;
                    Ok(())
                }
                Err(err) if err.is_record_not_found() => Err(CollectionError::PlaylistNotFound),
                Err(err) => Err(err.into()),
            }
        })
        .await
    }

    async fn delete_playlist(&self, id: PlaylistId) -> Result<(), CollectionError> {
        self.run(async move |mut db| {
            let mut tx = db.transaction().await?;
            PlaylistSong::filter_by_playlist_id(id as i64)
                .delete()
                .exec(&mut tx)
                .await?;
            match Playlist::get_by_id(&mut tx, id as i64).await {
                Ok(playlist) => {
                    playlist.delete().exec(&mut tx).await?;
                }
                Err(err) if err.is_record_not_found() => {
                    return Err(CollectionError::PlaylistNotFound);
                }
                Err(err) => return Err(err.into()),
            }
            tx.commit().await?;
            Ok(())
        })
        .await
    }

    async fn upsert_song(
        &self,
        path: &str,
        duration_ms: u64,
        tags: &SongTags,
    ) -> Result<SongId, CollectionError> {
        let path = path.to_owned();
        let tags = tags.clone();
        self.run(async move |mut db| {
            let path = path.clone();
            let tags = tags.clone();
            let song = Song::upsert_by_file_path(&path)
                .duration_ms(duration_ms as i64)
                .title(tags.title.clone())
                .artist(tags.artist.clone())
                .album(tags.album.clone())
                .album_artist(tags.album_artist.clone())
                .composer(tags.composer.clone())
                .lyricist(tags.lyricist.clone())
                .performer(tags.performer.clone())
                .genre(tags.genre.clone())
                .comment(tags.comment.clone())
                .date(tags.date.clone())
                .year(tags.year.map(i64::from))
                .track_number(tags.track_number.map(|value| value as i64))
                .track_total(tags.track_total.map(|value| value as i64))
                .disc_number(tags.disc_number.map(|value| value as i64))
                .disc_total(tags.disc_total.map(|value| value as i64))
                .cover_mime(tags.cover_mime.clone())
                .cover_file_type(tags.cover_file_type.map(StoredCoverFileType::from))
                .exec(&mut db)
                .await?;
            Ok(song.id)
        })
        .await
    }

    async fn attach_song(
        &self,
        playlist_id: PlaylistId,
        song_id: SongId,
        position: i64,
    ) -> Result<(), CollectionError> {
        self.run(async move |mut db| {
            PlaylistSong::upsert_by_playlist_id_and_song_id(playlist_id as i64, song_id)
                .position(position)
                .or_ignore()
                .exec(&mut db)
                .await?;
            Ok(())
        })
        .await
    }

    async fn replace_playlist_songs(
        &self,
        playlist_id: PlaylistId,
        song_ids: &[SongId],
    ) -> Result<(), CollectionError> {
        let song_ids = song_ids.to_vec();
        self.run(async move |mut db| {
            let song_ids = song_ids.clone();
            // DELETE then INSERT must be atomic so a mid-way failure cannot leave
            // an empty playlist.
            let mut tx = db.transaction().await?;
            PlaylistSong::filter_by_playlist_id(playlist_id as i64)
                .delete()
                .exec(&mut tx)
                .await?;
            for (position, song_id) in song_ids.iter().copied().enumerate() {
                PlaylistSong::create()
                    .playlist_id(playlist_id as i64)
                    .song_id(song_id)
                    .position(position as i64)
                    .exec(&mut tx)
                    .await?;
            }
            tx.commit().await?;
            Ok(())
        })
        .await
    }

    async fn playlist_song_paths(
        &self,
        playlist_id: PlaylistId,
    ) -> Result<Vec<String>, CollectionError> {
        self.run(async move |mut db| {
            let mut tx = db.transaction().await?;
            let memberships = PlaylistSong::filter_by_playlist_id(playlist_id as i64)
                .order_by(PlaylistSong::fields().position().asc())
                .exec(&mut tx)
                .await?;
            let song_ids = memberships
                .iter()
                .map(|membership| membership.song_id)
                .collect::<Vec<_>>();
            let songs_by_id = Self::songs_by_ids(&mut tx, &song_ids).await?;
            let mut paths = Vec::with_capacity(memberships.len());
            for membership in memberships {
                let song_id = membership.song_id;
                let Some(song) = songs_by_id.get(&song_id) else {
                    error!(
                        playlist_id,
                        song_id, "playlist membership references missing song"
                    );
                    continue;
                };
                paths.push(song.file_path.clone());
            }
            tx.commit().await?;
            Ok(paths)
        })
        .await
    }

    async fn garbage_collect_orphan_songs(
        &self,
    ) -> Result<Vec<(SongId, Option<CoverFileType>)>, CollectionError> {
        self.run(async |mut db| {
            // Avoid relation-subquery filters: they can kill the Turso connection
            // task under async-compat (RecvError). Diff memberships in Rust instead.
            // All deletes commit together so a mid-way failure cannot orphan covers
            // that Collection would never learn about.
            let mut tx = db.transaction().await?;
            let songs = Song::all().exec(&mut tx).await?;
            let memberships = PlaylistSong::all().exec(&mut tx).await?;
            let referenced = memberships
                .into_iter()
                .map(|membership| membership.song_id)
                .collect::<HashSet<_>>();
            let mut removed = Vec::new();
            for song in songs {
                if referenced.contains(&song.id) {
                    continue;
                }
                let cover_file_type = song.cover_file_type.map(CoverFileType::from);
                let song_id = song.id;
                song.delete().exec(&mut tx).await?;
                removed.push((song_id, cover_file_type));
            }
            tx.commit().await?;
            Ok(removed)
        })
        .await
    }

    async fn max_position(&self, playlist_id: PlaylistId) -> Result<i64, CollectionError> {
        self.run(async move |mut db| {
            let max = PlaylistSong::filter_by_playlist_id(playlist_id as i64)
                .order_by(PlaylistSong::fields().position().desc())
                .first()
                .exec(&mut db)
                .await?;
            Ok(max.map(|membership| membership.position).unwrap_or(-1))
        })
        .await
    }

    async fn playback_mode(&self) -> Result<StoredPlaybackMode, CollectionError> {
        self.run(
            async |mut db| match Meta::get_by_key(&mut db, PLAYBACK_MODE_KEY).await {
                Ok(meta) => StoredPlaybackMode::parse(&meta.value)
                    .ok_or(CatalogDataError::UnknownPlaybackMode.into()),
                Err(err) if err.is_record_not_found() => Ok(StoredPlaybackMode::Sequential),
                Err(err) => Err(err.into()),
            },
        )
        .await
    }

    async fn set_playback_mode(&self, mode: StoredPlaybackMode) -> Result<(), CollectionError> {
        self.run(async move |mut db| {
            Meta::upsert_by_key(PLAYBACK_MODE_KEY)
                .value(mode.as_str())
                .exec(&mut db)
                .await?;
            Ok(())
        })
        .await
    }

    async fn cover_animation_in_background(
        &self,
    ) -> Result<CoverAnimationInBackground, CollectionError> {
        self.run(async |mut db| {
            match Meta::get_by_key(&mut db, COVER_ANIMATION_IN_BACKGROUND_KEY).await {
                Ok(meta) => match CoverAnimationInBackground::parse(&meta.value) {
                    Some(value) => Ok(value),
                    None => {
                        meta.delete().exec(&mut db).await?;
                        Ok(CoverAnimationInBackground::Pause)
                    }
                },
                Err(err) if err.is_record_not_found() => Ok(CoverAnimationInBackground::Pause),
                Err(err) => Err(err.into()),
            }
        })
        .await
    }

    async fn set_cover_animation_in_background(
        &self,
        value: CoverAnimationInBackground,
    ) -> Result<(), CollectionError> {
        self.run(async move |mut db| {
            Meta::upsert_by_key(COVER_ANIMATION_IN_BACKGROUND_KEY)
                .value(value.as_str())
                .exec(&mut db)
                .await?;
            Ok(())
        })
        .await
    }

    async fn volume(&self) -> Result<f64, CollectionError> {
        self.run(
            async |mut db| match Meta::get_by_key(&mut db, VOLUME_KEY).await {
                Ok(meta) => meta
                    .value
                    .parse::<f64>()
                    .map_err(|_| CatalogDataError::InvalidVolume.into()),
                Err(err) if err.is_record_not_found() => Ok(DEFAULT_VOLUME),
                Err(err) => Err(err.into()),
            },
        )
        .await
    }

    async fn set_volume(&self, volume: f64) -> Result<(), CollectionError> {
        let volume = volume.clamp(0.0, 1.0);
        let value = volume.to_string();
        self.run(async move |mut db| {
            let value = value.clone();
            Meta::upsert_by_key(VOLUME_KEY)
                .value(value)
                .exec(&mut db)
                .await?;
            Ok(())
        })
        .await
    }

    async fn playback_session(&self) -> Result<Option<PlaybackSession>, CollectionError> {
        self.run(
            async |mut db| match Meta::get_by_key(&mut db, PLAYBACK_SESSION_KEY).await {
                Ok(meta) => match PlaybackSession::from_json(&meta.value) {
                    Ok(session) => Ok(Some(session)),
                    Err(_) => {
                        meta.delete().exec(&mut db).await?;
                        Ok(None)
                    }
                },
                Err(err) if err.is_record_not_found() => Ok(None),
                Err(err) => Err(err.into()),
            },
        )
        .await
    }

    async fn set_playback_session(&self, session: &PlaybackSession) -> Result<(), CollectionError> {
        let value = session.to_json()?;
        self.run(async move |mut db| {
            let value = value.clone();
            Meta::upsert_by_key(PLAYBACK_SESSION_KEY)
                .value(value)
                .exec(&mut db)
                .await?;
            Ok(())
        })
        .await
    }

    async fn clear_playback_session(&self) -> Result<(), CollectionError> {
        self.run(
            async |mut db| match Meta::get_by_key(&mut db, PLAYBACK_SESSION_KEY).await {
                Ok(meta) => {
                    meta.delete().exec(&mut db).await?;
                    Ok(())
                }
                Err(err) if err.is_record_not_found() => Ok(()),
                Err(err) => Err(err.into()),
            },
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirrors [`CatalogStore::run`]'s retry loop without a live [`Db`].
    async fn with_serialization_retries<T, F>(mut op: F) -> Result<T, CollectionError>
    where
        F: AsyncFnMut() -> Result<T, CollectionError>,
    {
        for _attempt in 0..MAX_SERIALIZATION_RETRIES {
            match op().await {
                Ok(value) => return Ok(value),
                Err(err) if err.is_serialization_failure() => {
                    Delay::new(SERIALIZATION_RETRY_DELAY).await;
                }
                Err(err) => return Err(err),
            }
        }
        op().await
    }

    #[test]
    fn serialization_retries_until_success() {
        async_global_executor::block_on(async {
            let mut attempts = 0_usize;
            let result = with_serialization_retries(async || {
                attempts += 1;
                if attempts < 3 {
                    Err(CollectionError::Database(
                        toasty::Error::serialization_failure("test conflict"),
                    ))
                } else {
                    Ok(7_i32)
                }
            })
            .await;
            assert_eq!(result.expect("eventually ok"), 7);
            assert_eq!(attempts, 3);
        });
    }

    #[test]
    fn serialization_retries_exhaust_and_return_error() {
        async_global_executor::block_on(async {
            let mut attempts = 0_usize;
            let result = with_serialization_retries(async || {
                attempts += 1;
                Err::<(), _>(CollectionError::Database(
                    toasty::Error::serialization_failure("still conflict"),
                ))
            })
            .await;
            assert!(
                result
                    .expect_err("retries exhausted")
                    .is_serialization_failure(),
                "final error should still be serialization failure"
            );
            // MAX_SERIALIZATION_RETRIES loop attempts + one final attempt.
            assert_eq!(attempts, MAX_SERIALIZATION_RETRIES + 1);
        });
    }
}
