use serde::{Deserialize, Serialize};

use crate::player::metadata::TrackMetadata;

/// Stable playlist identifier persisted in the library manifest.
pub type PlaylistId = u64;

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

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct LibraryManifest {
    pub(crate) next_id: PlaylistId,
    pub(crate) playlists: Vec<Playlist>,
}

impl LibraryManifest {
    pub(crate) fn allocate_id(&mut self) -> PlaylistId {
        self.next_id = self.next_id.max(1);
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }
}
