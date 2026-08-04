//! Directory walking and media inspection helpers for the collection.

use std::fs;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use walkdir::WalkDir;

use super::model::Song;
use crate::player::{MediaInspection, MediaInspector};

/// Scans audio files and builds [`Song`] values via a [`MediaInspector`].
pub(crate) struct CollectionScanner<'a> {
    inspector: &'a dyn MediaInspector,
}

impl<'a> CollectionScanner<'a> {
    pub(crate) fn new(inspector: &'a dyn MediaInspector) -> Self {
        Self { inspector }
    }

    pub(crate) fn scan_directory(&self, root: &Path) -> Vec<Song> {
        // Collect paths first so WalkDir stays single-threaded; inspect in parallel.
        let paths = WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| entry.into_path())
            .filter(|path| Self::is_audio_path(path))
            .collect::<Vec<_>>();
        let mut songs = paths
            .into_par_iter()
            .filter_map(|path| self.inspect_path(&path))
            .collect::<Vec<_>>();
        songs.sort_by(|left, right| left.path.cmp(&right.path));
        songs
    }

    pub(crate) fn inspect_path(&self, path: &Path) -> Option<Song> {
        let canonical = fs::canonicalize(path).ok()?;
        let MediaInspection {
            duration_ms,
            metadata,
        } = self.inspector.inspect(&canonical).ok()?;
        Some(Song {
            path: canonical.to_string_lossy().into_owned(),
            duration_ms,
            metadata,
        })
    }

    pub(crate) fn is_audio_path(path: &Path) -> bool {
        let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
            return false;
        };
        matches!(
            extension.to_ascii_lowercase().as_str(),
            "aac"
                | "aiff"
                | "alac"
                | "flac"
                | "m4a"
                | "mp3"
                | "mp4"
                | "ogg"
                | "opus"
                | "wav"
                | "webm"
                | "wma"
        )
    }

    pub(crate) fn inspect_paths(&self, paths: impl IntoIterator<Item = PathBuf>) -> Vec<Song> {
        paths
            .into_iter()
            .filter_map(|path| self.inspect_path(&path))
            .collect()
    }
}
