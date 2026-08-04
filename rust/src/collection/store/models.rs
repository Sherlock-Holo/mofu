//! Toasty models for `catalog.db` tables.

use toasty::Deferred;

/// Key/value settings stored in `meta`.
#[derive(Debug, toasty::Model)]
#[table = "meta"]
pub(crate) struct Meta {
    #[key]
    pub key: String,
    pub value: String,
}

/// Playlist kind column values persisted in `playlists.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, toasty::Embed)]
#[column(type = text)]
pub(crate) enum StoredPlaylistKind {
    #[column(variant = "directory")]
    Directory,
    #[column(variant = "virtual")]
    Virtual,
}

/// Cover file type column values persisted in `songs.cover_ext`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, toasty::Embed)]
#[column(type = text)]
pub(crate) enum StoredCoverFileType {
    #[column(variant = ".jpg")]
    Jpg,
    #[column(variant = ".png")]
    Png,
    #[column(variant = ".webp")]
    Webp,
    #[column(variant = ".gif")]
    Gif,
    #[column(variant = ".bin")]
    Bin,
}

/// Playlist header row in `playlists`.
#[derive(Debug, toasty::Model)]
#[table = "playlists"]
pub(crate) struct Playlist {
    #[key]
    pub id: i64,
    pub name: String,
    pub kind: StoredPlaylistKind,
    pub directory_path: Option<String>,

    #[has_many]
    pub memberships: Deferred<Vec<PlaylistSong>>,
}

/// Song row in `songs` (tags only; no cover bytes).
#[derive(Debug, toasty::Model)]
#[table = "songs"]
pub(crate) struct Song {
    #[key]
    #[auto]
    pub id: i64,

    #[unique]
    #[column("path")]
    pub file_path: String,

    pub duration_ms: i64,

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
    pub year: Option<i64>,
    pub track_number: Option<i64>,
    pub track_total: Option<i64>,
    pub disc_number: Option<i64>,
    pub disc_total: Option<i64>,
    pub cover_mime: Option<String>,
    #[column("cover_ext")]
    pub cover_file_type: Option<StoredCoverFileType>,

    #[has_many]
    pub memberships: Deferred<Vec<PlaylistSong>>,
}

/// Playlist membership join row in `playlist_songs`.
#[derive(Debug, toasty::Model)]
#[table = "playlist_songs"]
#[key(playlist_id, song_id)]
#[unique(playlist_id, position)]
pub(crate) struct PlaylistSong {
    #[index]
    pub playlist_id: i64,

    #[belongs_to(key = playlist_id, references = id)]
    pub playlist: Deferred<Playlist>,

    #[index]
    pub song_id: i64,

    #[belongs_to(key = song_id, references = id)]
    pub song: Deferred<Song>,

    pub position: i64,
}
