//! On-disk cover art cache under the XDG cache directory.

use std::fs;
use std::path::{Path, PathBuf};

use super::error::CollectionError;
use super::model::{CoverExt, SongId};
use crate::player::metadata::CoverArt;

/// Reads and writes cover files as `{song_id}{ext}` under a cache root.
pub(crate) struct CoverStore {
    root: PathBuf,
}

impl CoverStore {
    pub(crate) fn open(root: PathBuf) -> Result<Self, CollectionError> {
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// Persist cover bytes; returns the extension used on disk.
    pub(crate) fn write(
        &self,
        song_id: SongId,
        cover: &CoverArt,
    ) -> Result<CoverExt, CollectionError> {
        let ext = CoverExt::from_mime(cover.mime_type.as_deref());
        let path = self.path_for(song_id, ext);
        // Drop previous extension variants before writing the new temp file.
        Self::remove_all_for_id(&self.root, song_id);
        let temporary = self.root.join(format!("{song_id}.tmp"));
        fs::write(&temporary, &cover.data)?;
        fs::rename(&temporary, &path)?;
        Ok(ext)
    }

    pub(crate) fn read(
        &self,
        song_id: SongId,
        ext: CoverExt,
        mime_type: Option<String>,
    ) -> Option<CoverArt> {
        let path = self.path_for(song_id, ext);
        let data = fs::read(path).ok()?;
        if data.is_empty() {
            return None;
        }
        Some(CoverArt { mime_type, data })
    }

    pub(crate) fn remove(&self, song_id: SongId, ext: CoverExt) {
        let path = self.path_for(song_id, ext);
        let _ = fs::remove_file(path);
    }

    pub(crate) fn remove_all(&self, song_id: SongId) {
        Self::remove_all_for_id(&self.root, song_id);
    }

    fn path_for(&self, song_id: SongId, ext: CoverExt) -> PathBuf {
        self.root.join(format!("{song_id}{}", ext.as_str()))
    }

    fn remove_all_for_id(root: &Path, song_id: SongId) {
        for ext in CoverExt::ALL {
            let _ = fs::remove_file(root.join(format!("{song_id}{}", ext.as_str())));
        }
        let _ = fs::remove_file(root.join(format!("{song_id}.tmp")));
    }
}
