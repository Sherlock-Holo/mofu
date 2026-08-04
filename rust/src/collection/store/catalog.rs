//! `catalog.db`: playlists, songs (tags only), playlist membership.

use std::path::Path;

use turso::{Connection, Value, params};

use super::super::error::CollectionError;
use super::super::model::{
    CoverExt, PlaybackSession, PlaylistId, PlaylistKind, PlaylistRow, SongId, SongRow, SongTags,
    StoredPlaybackMode, StoredPlaylistKind,
};
use super::reconnect::ReconnectableDb;

const SCHEMA_VERSION: &str = "1";
const PLAYBACK_MODE_KEY: &str = "playback_mode";
const VOLUME_KEY: &str = "volume";
const PLAYBACK_SESSION_KEY: &str = "playback_session";
const DEFAULT_VOLUME: f64 = 1.0;

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS playlists (
    id INTEGER PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    directory_path TEXT
);
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
);
CREATE TABLE IF NOT EXISTS playlist_songs (
    playlist_id INTEGER NOT NULL,
    song_id INTEGER NOT NULL,
    position INTEGER NOT NULL,
    PRIMARY KEY (playlist_id, song_id),
    UNIQUE (playlist_id, position)
);
"#;

/// Catalog database store.
pub(crate) struct CatalogStore {
    db: ReconnectableDb,
}

impl CatalogStore {
    pub(crate) async fn open(path: &Path) -> Result<Self, CollectionError> {
        let db = ReconnectableDb::open(path).await?;
        let store = Self { db };
        store.migrate().await?;
        Ok(store)
    }

    pub(crate) async fn migrate(&self) -> Result<(), CollectionError> {
        self.db
            .run(async move |conn| {
                conn.execute_batch(SCHEMA_SQL).await?;
                conn.execute(
                    "INSERT OR IGNORE INTO meta (key, value) VALUES (?1, ?2)",
                    params!["schema_version", SCHEMA_VERSION],
                )
                .await?;
                Ok(())
            })
            .await
    }

    pub(crate) async fn next_playlist_id(&self) -> Result<PlaylistId, CollectionError> {
        self.db
            .run(async move |conn| {
                let mut rows = conn
                    .query("SELECT COALESCE(MAX(id), 0) FROM playlists", ())
                    .await?;
                let row = rows
                    .next()
                    .await?
                    .ok_or_else(|| CollectionError::Database(turso::Error::QueryReturnedNoRows))?;
                let max_id = row.get::<i64>(0)? as PlaylistId;
                Ok(max_id.saturating_add(1).max(1))
            })
            .await
    }

    pub(crate) async fn list_playlist_rows(&self) -> Result<Vec<PlaylistRow>, CollectionError> {
        self.db
            .run(async move |conn| Self::list_playlist_rows_on(conn).await)
            .await
    }

    async fn list_playlist_rows_on(conn: &Connection) -> Result<Vec<PlaylistRow>, CollectionError> {
        let mut rows = conn
            .query(
                "SELECT id, name, kind, directory_path FROM playlists ORDER BY id",
                (),
            )
            .await?;
        let mut playlists = Vec::new();
        while let Some(row) = rows.next().await? {
            let id = row.get::<i64>(0)? as PlaylistId;
            let name = row.get::<String>(1)?;
            let kind_raw = row.get::<String>(2)?;
            let directory_path = match row.get_value(3)? {
                Value::Null => None,
                Value::Text(text) => Some(text),
                other => {
                    return Err(CollectionError::Database(turso::Error::ConversionFailure(
                        format!("unexpected directory_path value: {other:?}"),
                    )));
                }
            };
            let kind = match StoredPlaylistKind::parse(&kind_raw) {
                Some(StoredPlaylistKind::Directory) => match directory_path {
                    Some(path) => PlaylistKind::Directory { path },
                    None => {
                        return Err(CollectionError::Database(turso::Error::ConversionFailure(
                            "directory playlist missing directory_path".to_owned(),
                        )));
                    }
                },
                Some(StoredPlaylistKind::Virtual) => PlaylistKind::Virtual,
                None => {
                    return Err(CollectionError::Database(turso::Error::ConversionFailure(
                        format!("unknown playlist kind: {kind_raw}"),
                    )));
                }
            };
            playlists.push(PlaylistRow { id, name, kind });
        }
        Ok(playlists)
    }

    pub(crate) async fn playlist_row(
        &self,
        id: PlaylistId,
    ) -> Result<PlaylistRow, CollectionError> {
        self.db
            .run(async move |conn| {
                let mut rows = conn
                    .query(
                        "SELECT id, name, kind, directory_path FROM playlists WHERE id = ?1",
                        params![id as i64],
                    )
                    .await?;
                let row = rows
                    .next()
                    .await?
                    .ok_or(CollectionError::PlaylistNotFound)?;
                let name = row.get::<String>(1)?;
                let kind_raw = row.get::<String>(2)?;
                let directory_path = match row.get_value(3)? {
                    Value::Null => None,
                    Value::Text(text) => Some(text),
                    other => {
                        return Err(CollectionError::Database(turso::Error::ConversionFailure(
                            format!("unexpected directory_path value: {other:?}"),
                        )));
                    }
                };
                let kind = match StoredPlaylistKind::parse(&kind_raw) {
                    Some(StoredPlaylistKind::Directory) => match directory_path {
                        Some(path) => PlaylistKind::Directory { path },
                        None => {
                            return Err(CollectionError::Database(
                                turso::Error::ConversionFailure(
                                    "directory playlist missing directory_path".to_owned(),
                                ),
                            ));
                        }
                    },
                    Some(StoredPlaylistKind::Virtual) => PlaylistKind::Virtual,
                    None => {
                        return Err(CollectionError::Database(turso::Error::ConversionFailure(
                            format!("unknown playlist kind: {kind_raw}"),
                        )));
                    }
                };
                Ok(PlaylistRow { id, name, kind })
            })
            .await
    }

    pub(crate) async fn songs_for_playlist(
        &self,
        playlist_id: PlaylistId,
    ) -> Result<Vec<SongRow>, CollectionError> {
        self.db
            .run(async move |conn| Self::songs_for_playlist_on(conn, playlist_id).await)
            .await
    }

    /// Look up catalog rows for the given absolute paths (order not preserved).
    pub(crate) async fn song_rows_for_paths(
        &self,
        paths: &[String],
    ) -> Result<Vec<SongRow>, CollectionError> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let paths = paths.to_vec();
        self.db
            .run(async move |conn| Self::song_rows_for_paths_on(conn, &paths).await)
            .await
    }

    async fn song_rows_for_paths_on(
        conn: &Connection,
        paths: &[String],
    ) -> Result<Vec<SongRow>, CollectionError> {
        // SQLite 变量数有上限；按批查询避免超限。
        const BATCH: usize = 200;
        let mut songs = Vec::new();
        for chunk in paths.chunks(BATCH) {
            let placeholders = (1..=chunk.len())
                .map(|index| format!("?{index}"))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                r#"
                SELECT id, path, duration_ms,
                       title, artist, album, album_artist, composer, lyricist,
                       performer, genre, comment, date, year,
                       track_number, track_total, disc_number, disc_total,
                       cover_mime, cover_ext
                FROM songs
                WHERE path IN ({placeholders})
                "#
            );
            let params: Vec<Value> = chunk.iter().map(|path| Value::Text(path.clone())).collect();
            let mut rows = conn.query(&sql, params).await?;
            while let Some(row) = rows.next().await? {
                songs.push(Self::song_row_from_query(row)?);
            }
        }
        Ok(songs)
    }

    async fn songs_for_playlist_on(
        conn: &Connection,
        playlist_id: PlaylistId,
    ) -> Result<Vec<SongRow>, CollectionError> {
        let mut rows = conn
            .query(
                r#"
                SELECT s.id, s.path, s.duration_ms,
                       s.title, s.artist, s.album, s.album_artist, s.composer, s.lyricist,
                       s.performer, s.genre, s.comment, s.date, s.year,
                       s.track_number, s.track_total, s.disc_number, s.disc_total,
                       s.cover_mime, s.cover_ext
                FROM playlist_songs ps
                JOIN songs s ON s.id = ps.song_id
                WHERE ps.playlist_id = ?1
                ORDER BY ps.position
                "#,
                params![playlist_id as i64],
            )
            .await?;
        let mut songs = Vec::new();
        while let Some(row) = rows.next().await? {
            songs.push(Self::song_row_from_query(row)?);
        }
        Ok(songs)
    }

    fn song_row_from_query(row: turso::Row) -> Result<SongRow, CollectionError> {
        Ok(SongRow {
            id: row.get::<i64>(0)?,
            path: row.get::<String>(1)?,
            duration_ms: row.get::<i64>(2)? as u64,
            tags: SongTags {
                title: Self::opt_text(&row, 3)?,
                artist: Self::opt_text(&row, 4)?,
                album: Self::opt_text(&row, 5)?,
                album_artist: Self::opt_text(&row, 6)?,
                composer: Self::opt_text(&row, 7)?,
                lyricist: Self::opt_text(&row, 8)?,
                performer: Self::opt_text(&row, 9)?,
                genre: Self::opt_text(&row, 10)?,
                comment: Self::opt_text(&row, 11)?,
                date: Self::opt_text(&row, 12)?,
                year: Self::opt_u16(&row, 13)?,
                track_number: Self::opt_u64(&row, 14)?,
                track_total: Self::opt_u64(&row, 15)?,
                disc_number: Self::opt_u64(&row, 16)?,
                disc_total: Self::opt_u64(&row, 17)?,
                cover_mime: Self::opt_text(&row, 18)?,
                cover_ext: Self::opt_cover_ext(&row, 19)?,
            },
        })
    }

    fn opt_cover_ext(row: &turso::Row, idx: usize) -> Result<Option<CoverExt>, CollectionError> {
        match Self::opt_text(row, idx)? {
            None => Ok(None),
            Some(text) => match CoverExt::parse(&text) {
                Some(ext) => Ok(Some(ext)),
                None => Err(CollectionError::Database(turso::Error::ConversionFailure(
                    format!("unknown cover_ext: {text}"),
                ))),
            },
        }
    }

    fn opt_text(row: &turso::Row, idx: usize) -> Result<Option<String>, CollectionError> {
        match row.get_value(idx)? {
            Value::Null => Ok(None),
            Value::Text(text) => Ok(Some(text)),
            other => Err(CollectionError::Database(turso::Error::ConversionFailure(
                format!("expected text/null at {idx}, got {other:?}"),
            ))),
        }
    }

    fn opt_u16(row: &turso::Row, idx: usize) -> Result<Option<u16>, CollectionError> {
        match row.get_value(idx)? {
            Value::Null => Ok(None),
            Value::Integer(value) => Ok(Some(value as u16)),
            other => Err(CollectionError::Database(turso::Error::ConversionFailure(
                format!("expected int/null at {idx}, got {other:?}"),
            ))),
        }
    }

    fn opt_u64(row: &turso::Row, idx: usize) -> Result<Option<u64>, CollectionError> {
        match row.get_value(idx)? {
            Value::Null => Ok(None),
            Value::Integer(value) => Ok(Some(value as u64)),
            other => Err(CollectionError::Database(turso::Error::ConversionFailure(
                format!("expected int/null at {idx}, got {other:?}"),
            ))),
        }
    }

    pub(crate) async fn insert_playlist(
        &self,
        id: PlaylistId,
        name: &str,
        kind: &PlaylistKind,
    ) -> Result<(), CollectionError> {
        let (stored_kind, directory_path) = match kind {
            PlaylistKind::Directory { path } => {
                (StoredPlaylistKind::Directory, Some(path.as_str()))
            }
            PlaylistKind::Virtual => (StoredPlaylistKind::Virtual, None),
        };
        let name = name.to_owned();
        let directory_path = directory_path.map(str::to_owned);
        self.db
            .run(async move |conn| {
                conn.execute(
                    "INSERT INTO playlists (id, name, kind, directory_path) VALUES (?1, ?2, ?3, ?4)",
                    params![
                        id as i64,
                        name.as_str(),
                        stored_kind.as_str(),
                        directory_path.clone(),
                    ],
                )
                    .await?;
                Ok(())
            })
            .await
    }

    pub(crate) async fn find_directory_playlist_id(
        &self,
        directory_path: &str,
    ) -> Result<Option<PlaylistId>, CollectionError> {
        let directory_path = directory_path.to_owned();
        self.db
            .run(async move |conn| {
                let mut rows = conn
                    .query(
                        "SELECT id FROM playlists WHERE kind = ?1 AND directory_path = ?2",
                        params![
                            StoredPlaylistKind::Directory.as_str(),
                            directory_path.as_str()
                        ],
                    )
                    .await?;
                Ok(match rows.next().await? {
                    Some(row) => Some(row.get::<i64>(0)? as PlaylistId),
                    None => None,
                })
            })
            .await
    }

    pub(crate) async fn rename_playlist(
        &self,
        id: PlaylistId,
        name: &str,
    ) -> Result<(), CollectionError> {
        let name = name.to_owned();
        self.db
            .run(async move |conn| {
                let changed = conn
                    .execute(
                        "UPDATE playlists SET name = ?1 WHERE id = ?2",
                        params![name.as_str(), id as i64],
                    )
                    .await?;
                if changed == 0 {
                    return Err(CollectionError::PlaylistNotFound);
                }
                Ok(())
            })
            .await
    }

    pub(crate) async fn delete_playlist(&self, id: PlaylistId) -> Result<(), CollectionError> {
        self.db
            .run(async move |conn| {
                let tx = conn.transaction().await?;
                tx.execute(
                    "DELETE FROM playlist_songs WHERE playlist_id = ?1",
                    params![id as i64],
                )
                .await?;
                let changed = tx
                    .execute("DELETE FROM playlists WHERE id = ?1", params![id as i64])
                    .await?;
                if changed == 0 {
                    return Err(CollectionError::PlaylistNotFound);
                }
                tx.commit().await?;
                Ok(())
            })
            .await
    }

    /// Upsert song tags by path; returns song id.
    pub(crate) async fn upsert_song(
        &self,
        path: &str,
        duration_ms: u64,
        tags: &SongTags,
    ) -> Result<SongId, CollectionError> {
        let path = path.to_owned();
        let tags = tags.clone();
        self.db
            .run(async move |conn| Self::upsert_song_on(conn, &path, duration_ms, &tags).await)
            .await
    }

    async fn upsert_song_on(
        conn: &Connection,
        path: &str,
        duration_ms: u64,
        tags: &SongTags,
    ) -> Result<SongId, CollectionError> {
        conn.execute(
            r#"
            INSERT INTO songs (
                path, duration_ms, title, artist, album, album_artist, composer, lyricist,
                performer, genre, comment, date, year, track_number, track_total,
                disc_number, disc_total, cover_mime, cover_ext
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19
            )
            ON CONFLICT(path) DO UPDATE SET
                duration_ms = excluded.duration_ms,
                title = excluded.title,
                artist = excluded.artist,
                album = excluded.album,
                album_artist = excluded.album_artist,
                composer = excluded.composer,
                lyricist = excluded.lyricist,
                performer = excluded.performer,
                genre = excluded.genre,
                comment = excluded.comment,
                date = excluded.date,
                year = excluded.year,
                track_number = excluded.track_number,
                track_total = excluded.track_total,
                disc_number = excluded.disc_number,
                disc_total = excluded.disc_total,
                cover_mime = excluded.cover_mime,
                cover_ext = excluded.cover_ext
            "#,
            params![
                path,
                duration_ms as i64,
                tags.title.clone(),
                tags.artist.clone(),
                tags.album.clone(),
                tags.album_artist.clone(),
                tags.composer.clone(),
                tags.lyricist.clone(),
                tags.performer.clone(),
                tags.genre.clone(),
                tags.comment.clone(),
                tags.date.clone(),
                tags.year.map(|year| year as i64),
                tags.track_number.map(|value| value as i64),
                tags.track_total.map(|value| value as i64),
                tags.disc_number.map(|value| value as i64),
                tags.disc_total.map(|value| value as i64),
                tags.cover_mime.clone(),
                tags.cover_ext.map(CoverExt::as_str),
            ],
        )
        .await?;

        let mut rows = conn
            .query("SELECT id FROM songs WHERE path = ?1", params![path])
            .await?;
        let row = rows
            .next()
            .await?
            .ok_or_else(|| CollectionError::Database(turso::Error::QueryReturnedNoRows))?;
        Ok(row.get::<i64>(0)?)
    }

    pub(crate) async fn attach_song(
        &self,
        playlist_id: PlaylistId,
        song_id: SongId,
        position: i64,
    ) -> Result<(), CollectionError> {
        self.db
            .run(async move |conn| {
                conn.execute(
                    r#"
                    INSERT OR IGNORE INTO playlist_songs (playlist_id, song_id, position)
                    VALUES (?1, ?2, ?3)
                    "#,
                    params![playlist_id as i64, song_id, position],
                )
                .await?;
                Ok(())
            })
            .await
    }

    pub(crate) async fn replace_playlist_songs(
        &self,
        playlist_id: PlaylistId,
        song_ids: &[SongId],
    ) -> Result<(), CollectionError> {
        let song_ids = song_ids.to_vec();
        self.db
            .run(async move |conn| {
                // DELETE then INSERT must be atomic: Busy mid-way would leave an empty playlist.
                let tx = conn.transaction().await?;
                tx.execute(
                    "DELETE FROM playlist_songs WHERE playlist_id = ?1",
                    params![playlist_id as i64],
                )
                    .await?;
                for (position, song_id) in song_ids.iter().copied().enumerate() {
                    tx.execute(
                        "INSERT INTO playlist_songs (playlist_id, song_id, position) VALUES (?1, ?2, ?3)",
                        params![playlist_id as i64, song_id, position as i64],
                    )
                        .await?;
                }
                tx.commit().await?;
                Ok(())
            })
            .await
    }

    pub(crate) async fn playlist_song_paths(
        &self,
        playlist_id: PlaylistId,
    ) -> Result<Vec<String>, CollectionError> {
        self.db
            .run(async move |conn| {
                let mut rows = conn
                    .query(
                        r#"
                        SELECT s.path
                        FROM playlist_songs ps
                        JOIN songs s ON s.id = ps.song_id
                        WHERE ps.playlist_id = ?1
                        ORDER BY ps.position
                        "#,
                        params![playlist_id as i64],
                    )
                    .await?;
                let mut paths = Vec::new();
                while let Some(row) = rows.next().await? {
                    paths.push(row.get::<String>(0)?);
                }
                Ok(paths)
            })
            .await
    }

    /// Delete songs not referenced by any playlist; returns removed song ids and cover_ext.
    pub(crate) async fn garbage_collect_orphan_songs(
        &self,
    ) -> Result<Vec<(SongId, Option<CoverExt>)>, CollectionError> {
        self.db
            .run(async move |conn| {
                let mut rows = conn
                    .query(
                        r#"
                        SELECT id, cover_ext FROM songs
                        WHERE id NOT IN (SELECT DISTINCT song_id FROM playlist_songs)
                        "#,
                        (),
                    )
                    .await?;
                let mut orphans = Vec::new();
                while let Some(row) = rows.next().await? {
                    let id = row.get::<i64>(0)?;
                    let cover_ext = Self::opt_cover_ext(&row, 1)?;
                    orphans.push((id, cover_ext));
                }
                if !orphans.is_empty() {
                    conn.execute(
                        "DELETE FROM songs WHERE id NOT IN (SELECT DISTINCT song_id FROM playlist_songs)",
                        (),
                    )
                        .await?;
                }
                Ok(orphans)
            })
            .await
    }

    pub(crate) async fn max_position(
        &self,
        playlist_id: PlaylistId,
    ) -> Result<i64, CollectionError> {
        self.db
            .run(async move |conn| {
                let mut rows = conn
                    .query(
                        "SELECT COALESCE(MAX(position), -1) FROM playlist_songs WHERE playlist_id = ?1",
                        params![playlist_id as i64],
                    )
                    .await?;
                let row = rows
                    .next()
                    .await?
                    .ok_or_else(|| CollectionError::Database(turso::Error::QueryReturnedNoRows))?;
                Ok(row.get::<i64>(0)?)
            })
            .await
    }

    /// Load persisted playback mode; missing key defaults to sequential.
    pub(crate) async fn playback_mode(&self) -> Result<StoredPlaybackMode, CollectionError> {
        self.db
            .run(async move |conn| {
                let mut rows = conn
                    .query(
                        "SELECT value FROM meta WHERE key = ?1",
                        params![PLAYBACK_MODE_KEY],
                    )
                    .await?;
                let Some(row) = rows.next().await? else {
                    return Ok(StoredPlaybackMode::Sequential);
                };
                let value = row.get::<String>(0)?;
                StoredPlaybackMode::parse(&value).ok_or_else(|| {
                    CollectionError::Database(turso::Error::ConversionFailure(format!(
                        "unknown playback_mode: {value}"
                    )))
                })
            })
            .await
    }

    pub(crate) async fn set_playback_mode(
        &self,
        mode: StoredPlaybackMode,
    ) -> Result<(), CollectionError> {
        self.db
            .run(async move |conn| {
                conn.execute(
                    "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![PLAYBACK_MODE_KEY, mode.as_str()],
                )
                    .await?;
                Ok(())
            })
            .await
    }

    /// Load persisted volume; missing key defaults to `1.0`.
    pub(crate) async fn volume(&self) -> Result<f64, CollectionError> {
        self.db
            .run(async move |conn| {
                let mut rows = conn
                    .query("SELECT value FROM meta WHERE key = ?1", params![VOLUME_KEY])
                    .await?;
                let Some(row) = rows.next().await? else {
                    return Ok(DEFAULT_VOLUME);
                };
                let value = row.get::<String>(0)?;
                value.parse::<f64>().map_err(|err| {
                    CollectionError::Database(turso::Error::ConversionFailure(format!(
                        "invalid volume `{value}`: {err}"
                    )))
                })
            })
            .await
    }

    /// Persist volume clamped to `[0, 1]`.
    pub(crate) async fn set_volume(&self, volume: f64) -> Result<(), CollectionError> {
        let volume = volume.clamp(0.0, 1.0);
        let value = volume.to_string();
        self.db
            .run(async move |conn| {
                conn.execute(
                    "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![VOLUME_KEY, value.as_str()],
                )
                    .await?;
                Ok(())
            })
            .await
    }

    /// Load persisted playback session; missing key is `None`.
    ///
    /// Corrupt JSON is cleared and treated as no session (not silently defaulted).
    pub(crate) async fn playback_session(
        &self,
    ) -> Result<Option<PlaybackSession>, CollectionError> {
        self.db
            .run(async move |conn| {
                let mut rows = conn
                    .query(
                        "SELECT value FROM meta WHERE key = ?1",
                        params![PLAYBACK_SESSION_KEY],
                    )
                    .await?;
                let Some(row) = rows.next().await? else {
                    return Ok(None);
                };
                let value = row.get::<String>(0)?;
                match PlaybackSession::from_json(&value) {
                    Ok(session) => Ok(Some(session)),
                    Err(_) => {
                        conn.execute(
                            "DELETE FROM meta WHERE key = ?1",
                            params![PLAYBACK_SESSION_KEY],
                        )
                        .await?;
                        Ok(None)
                    }
                }
            })
            .await
    }

    pub(crate) async fn set_playback_session(
        &self,
        session: &PlaybackSession,
    ) -> Result<(), CollectionError> {
        let value = session.to_json()?;
        self.db
            .run(async move |conn| {
                conn.execute(
                    "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![PLAYBACK_SESSION_KEY, value.as_str()],
                )
                    .await?;
                Ok(())
            })
            .await
    }

    pub(crate) async fn clear_playback_session(&self) -> Result<(), CollectionError> {
        self.db
            .run(async move |conn| {
                conn.execute(
                    "DELETE FROM meta WHERE key = ?1",
                    params![PLAYBACK_SESSION_KEY],
                )
                .await?;
                Ok(())
            })
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
        self.db
            .run(async move |conn| {
                conn.execute(
                    "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![key.as_str(), value.as_str()],
                )
                    .await?;
                Ok(())
            })
            .await
    }
}
