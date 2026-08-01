//! `lyrics.db`: lyrics tracks keyed by catalog song id.

use std::path::Path;

use turso::{Connection, Value, params};

use super::super::error::LibraryError;
use super::super::model::SongId;
use super::reconnect::ReconnectableDb;
use crate::player::metadata::{Lyrics, LyricsContent, LyricsLine, LyricsSource};

const SCHEMA_VERSION: &str = "1";

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS lyrics (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    song_id INTEGER NOT NULL,
    source TEXT NOT NULL,
    language TEXT,
    description TEXT,
    content_kind TEXT NOT NULL,
    plain_text TEXT,
    timed_json TEXT
);
CREATE INDEX IF NOT EXISTS idx_lyrics_song_id ON lyrics(song_id);
"#;

/// How lyrics content kind is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StoredContentKind {
    Plain,
    Timed,
}

impl StoredContentKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Timed => "timed",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "plain" => Some(Self::Plain),
            "timed" => Some(Self::Timed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StoredSource {
    Embedded,
    ExternalFile,
}

impl StoredSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Embedded => "embedded",
            Self::ExternalFile => "external_file",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "embedded" => Some(Self::Embedded),
            "external_file" => Some(Self::ExternalFile),
            _ => None,
        }
    }

    fn from_domain(source: LyricsSource) -> Self {
        match source {
            LyricsSource::Embedded => Self::Embedded,
            LyricsSource::ExternalFile => Self::ExternalFile,
        }
    }

    fn to_domain(self) -> LyricsSource {
        match self {
            Self::Embedded => LyricsSource::Embedded,
            Self::ExternalFile => LyricsSource::ExternalFile,
        }
    }
}

/// Lyrics database store.
pub(crate) struct LyricsStore {
    db: ReconnectableDb,
}

impl LyricsStore {
    pub(crate) async fn open(path: &Path) -> Result<Self, LibraryError> {
        let db = ReconnectableDb::open(path).await?;
        let store = Self { db };
        store.migrate().await?;
        Ok(store)
    }

    pub(crate) async fn migrate(&self) -> Result<(), LibraryError> {
        self.db
            .run(|conn| async move {
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

    pub(crate) async fn replace_for_song(
        &self,
        song_id: SongId,
        lyrics: &[Lyrics],
    ) -> Result<(), LibraryError> {
        let lyrics = lyrics.to_vec();
        self.db
            .run(|conn| {
                let lyrics = lyrics.clone();
                async move { Self::replace_for_song_on(&conn, song_id, &lyrics).await }
            })
            .await
    }

    async fn replace_for_song_on(
        conn: &Connection,
        song_id: SongId,
        lyrics: &[Lyrics],
    ) -> Result<(), LibraryError> {
        conn.execute("DELETE FROM lyrics WHERE song_id = ?1", params![song_id])
            .await?;
        for entry in lyrics {
            let source = StoredSource::from_domain(entry.source);
            let (content_kind, plain_text, timed_json) = match &entry.content {
                LyricsContent::Plain { text } => {
                    (StoredContentKind::Plain, Some(text.clone()), None)
                }
                LyricsContent::Timed { lines } => (
                    StoredContentKind::Timed,
                    None,
                    Some(serde_json::to_string(lines)?),
                ),
            };
            conn.execute(
                r#"
                INSERT INTO lyrics (
                    song_id, source, language, description, content_kind, plain_text, timed_json
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                "#,
                params![
                    song_id,
                    source.as_str(),
                    entry.language.clone(),
                    entry.description.clone(),
                    content_kind.as_str(),
                    plain_text,
                    timed_json,
                ],
            )
            .await?;
        }
        Ok(())
    }

    pub(crate) async fn load_for_song(&self, song_id: SongId) -> Result<Vec<Lyrics>, LibraryError> {
        self.db
            .run(|conn| async move { Self::load_for_song_on(&conn, song_id).await })
            .await
    }

    async fn load_for_song_on(
        conn: &Connection,
        song_id: SongId,
    ) -> Result<Vec<Lyrics>, LibraryError> {
        let mut rows = conn
            .query(
                r#"
                SELECT source, language, description, content_kind, plain_text, timed_json
                FROM lyrics WHERE song_id = ?1 ORDER BY id
                "#,
                params![song_id],
            )
            .await?;
        let mut lyrics = Vec::new();
        while let Some(row) = rows.next().await? {
            let source_raw = row.get::<String>(0)?;
            let source = StoredSource::parse(&source_raw)
                .ok_or_else(|| {
                    LibraryError::Database(turso::Error::ConversionFailure(format!(
                        "unknown lyrics source: {source_raw}"
                    )))
                })?
                .to_domain();
            let language = Self::opt_text(&row, 1)?;
            let description = Self::opt_text(&row, 2)?;
            let kind_raw = row.get::<String>(3)?;
            let content = match StoredContentKind::parse(&kind_raw) {
                Some(StoredContentKind::Plain) => LyricsContent::Plain {
                    text: Self::opt_text(&row, 4)?.unwrap_or_default(),
                },
                Some(StoredContentKind::Timed) => {
                    let timed = Self::opt_text(&row, 5)?.unwrap_or_else(|| "[]".to_owned());
                    let lines: Vec<LyricsLine> = serde_json::from_str(&timed)?;
                    LyricsContent::Timed { lines }
                }
                None => {
                    return Err(LibraryError::Database(turso::Error::ConversionFailure(
                        format!("unknown lyrics content kind: {kind_raw}"),
                    )));
                }
            };
            lyrics.push(Lyrics {
                source,
                language,
                description,
                content,
            });
        }
        Ok(lyrics)
    }

    pub(crate) async fn delete_for_songs(&self, song_ids: &[SongId]) -> Result<(), LibraryError> {
        if song_ids.is_empty() {
            return Ok(());
        }
        let song_ids = song_ids.to_vec();
        self.db
            .run(|conn| {
                let song_ids = song_ids.clone();
                async move {
                    for song_id in song_ids {
                        conn.execute("DELETE FROM lyrics WHERE song_id = ?1", params![song_id])
                            .await?;
                    }
                    Ok(())
                }
            })
            .await
    }

    fn opt_text(row: &turso::Row, idx: usize) -> Result<Option<String>, LibraryError> {
        match row.get_value(idx)? {
            Value::Null => Ok(None),
            Value::Text(text) => Ok(Some(text)),
            other => Err(LibraryError::Database(turso::Error::ConversionFailure(
                format!("expected text/null at {idx}, got {other:?}"),
            ))),
        }
    }
}
