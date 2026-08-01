use serde::{Deserialize, Serialize};

use crate::player::metadata::TrackMetadata;

/// Stable playlist identifier persisted in the library catalog.
pub type PlaylistId = u64;

/// Database-assigned song row identifier.
pub type SongId = i64;

/// File extension used for a cached cover image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CoverExt {
    Jpg,
    Png,
    Webp,
    Gif,
    Bin,
}

impl CoverExt {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Jpg => ".jpg",
            Self::Png => ".png",
            Self::Webp => ".webp",
            Self::Gif => ".gif",
            Self::Bin => ".bin",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            ".jpg" => Some(Self::Jpg),
            ".png" => Some(Self::Png),
            ".webp" => Some(Self::Webp),
            ".gif" => Some(Self::Gif),
            ".bin" => Some(Self::Bin),
            _ => None,
        }
    }

    pub(crate) fn from_mime(mime: Option<&str>) -> Self {
        match mime {
            Some("image/jpeg" | "image/jpg") => Self::Jpg,
            Some("image/png") => Self::Png,
            Some("image/webp") => Self::Webp,
            Some("image/gif") => Self::Gif,
            _ => Self::Bin,
        }
    }

    pub(crate) const ALL: [Self; 5] = [Self::Jpg, Self::Png, Self::Webp, Self::Gif, Self::Bin];
}

/// A library song with parsed media metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Song {
    pub path: String,
    pub duration_ms: u64,
    pub metadata: TrackMetadata,
}

/// Distinguishes scanned folders from user-managed virtual playlists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaylistKind {
    Directory { path: String },
    Virtual,
}

/// A persisted playlist and its current songs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Playlist {
    pub id: PlaylistId,
    pub name: String,
    pub kind: PlaylistKind,
    pub songs: Vec<Song>,
}

/// Legacy JSON manifest used only for one-shot migration into Turso.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct LibraryManifest {
    pub(crate) next_id: PlaylistId,
    pub(crate) playlists: Vec<Playlist>,
}

/// How playback mode is stored in `catalog.db` meta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum StoredPlaybackMode {
    #[default]
    Sequential,
    Shuffle,
    RepeatOne,
    RepeatAll,
}

impl StoredPlaybackMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Sequential => "sequential",
            Self::Shuffle => "shuffle",
            Self::RepeatOne => "repeat_one",
            Self::RepeatAll => "repeat_all",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "sequential" => Some(Self::Sequential),
            "shuffle" => Some(Self::Shuffle),
            "repeat_one" => Some(Self::RepeatOne),
            "repeat_all" => Some(Self::RepeatAll),
            _ => None,
        }
    }
}

/// How a playlist kind is stored in `catalog.db`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StoredPlaylistKind {
    Directory,
    Virtual,
}

impl StoredPlaylistKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Directory => "directory",
            Self::Virtual => "virtual",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "directory" => Some(Self::Directory),
            "virtual" => Some(Self::Virtual),
            _ => None,
        }
    }
}

/// Song tags persisted in `catalog.db` (no cover bytes / lyrics).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct SongTags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub composer: Option<String>,
    pub lyricist: Option<String>,
    pub performer: Option<String>,
    pub genre: Option<String>,
    pub comment: Option<String>,
    pub date: Option<String>,
    pub year: Option<u16>,
    pub track_number: Option<u64>,
    pub track_total: Option<u64>,
    pub disc_number: Option<u64>,
    pub disc_total: Option<u64>,
    pub cover_mime: Option<String>,
    pub cover_ext: Option<CoverExt>,
}

impl SongTags {
    pub(crate) fn from_metadata(metadata: &TrackMetadata, cover_ext: Option<CoverExt>) -> Self {
        Self {
            title: metadata.title.clone(),
            artist: metadata.artist.clone(),
            album: metadata.album.clone(),
            album_artist: metadata.album_artist.clone(),
            composer: metadata.composer.clone(),
            lyricist: metadata.lyricist.clone(),
            performer: metadata.performer.clone(),
            genre: metadata.genre.clone(),
            comment: metadata.comment.clone(),
            date: metadata.date.clone(),
            year: metadata.year,
            track_number: metadata.track_number,
            track_total: metadata.track_total,
            disc_number: metadata.disc_number,
            disc_total: metadata.disc_total,
            cover_mime: metadata
                .cover
                .as_ref()
                .and_then(|cover| cover.mime_type.clone()),
            cover_ext,
        }
    }
}

/// Catalog row for a song without assembling cover/lyrics yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SongRow {
    pub id: SongId,
    pub path: String,
    pub duration_ms: u64,
    pub tags: SongTags,
}

/// Playlist header without songs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlaylistRow {
    pub id: PlaylistId,
    pub name: String,
    pub kind: PlaylistKind,
}

/// Persisted playback resume point (playlist + track path + position).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaybackSession {
    pub playlist_id: PlaylistId,
    pub path: String,
    pub position_ms: u64,
}

impl PlaybackSession {
    pub(crate) fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub(crate) fn from_json(value: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(value)
    }
}
