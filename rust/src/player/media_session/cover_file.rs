//! Persist in-memory cover art to a temp file for `file://` URLs.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::super::metadata::CoverArt;

/// Writes cover bytes under a fixed temp directory and exposes a `file://` URL.
#[derive(Debug, Default)]
pub struct CoverFileCache {
    path: Option<PathBuf>,
}

impl CoverFileCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the cached cover file. Returns a `file://` URL when cover data is present.
    pub fn update(&mut self, cover: Option<&CoverArt>) -> io::Result<Option<String>> {
        let Some(cover) = cover else {
            self.clear()?;
            return Ok(None);
        };

        let dir = Self::cache_dir();
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!(
            "cover{}",
            Self::extension_for(cover.mime_type.as_deref())
        ));
        fs::write(&path, &cover.data)?;
        // Drop the previous file when the extension changed.
        if let Some(previous) = self.path.take()
            && previous != path
        {
            let _ = fs::remove_file(previous);
        }
        self.path = Some(path.clone());
        Ok(Some(Self::path_to_file_url(&path)))
    }

    /// Remove the cached cover file if any.
    pub fn clear(&mut self) -> io::Result<()> {
        if let Some(path) = self.path.take() {
            match fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(err) => Err(err),
            }
        } else {
            Ok(())
        }
    }

    fn cache_dir() -> PathBuf {
        env::temp_dir().join("mofu_media_session")
    }

    fn extension_for(mime: Option<&str>) -> &'static str {
        match mime {
            Some("image/png") => ".png",
            Some("image/webp") => ".webp",
            Some("image/gif") => ".gif",
            Some("image/bmp") => ".bmp",
            _ => ".jpg",
        }
    }

    fn path_to_file_url(path: &Path) -> String {
        let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let mut url = String::from("file://");
        #[cfg(windows)]
        {
            url.push('/');
            for ch in abs.to_string_lossy().chars() {
                match ch {
                    '\\' => url.push('/'),
                    ' ' => url.push_str("%20"),
                    c if c.is_ascii_alphanumeric() || matches!(c, '/' | ':' | '.' | '-' | '_') => {
                        url.push(c)
                    }
                    c => {
                        for byte in c.to_string().into_bytes() {
                            url.push_str(&format!("%{byte:02X}"));
                        }
                    }
                }
            }
        }
        #[cfg(not(windows))]
        {
            for ch in abs.to_string_lossy().chars() {
                match ch {
                    ' ' => url.push_str("%20"),
                    c if c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '-' | '_') => {
                        url.push(c)
                    }
                    c => {
                        for byte in c.to_string().into_bytes() {
                            url.push_str(&format!("%{byte:02X}"));
                        }
                    }
                }
            }
        }
        url
    }
}

impl Drop for CoverFileCache {
    fn drop(&mut self) {
        let _ = self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_writes_file_and_clear_removes_it() {
        let mut cache = CoverFileCache::new();
        let cover = CoverArt {
            mime_type: Some("image/png".into()),
            data: vec![0x89, 0x50, 0x4E, 0x47],
        };
        let url = cache
            .update(Some(&cover))
            .expect("write cover")
            .expect("url");
        assert!(url.starts_with("file://"));
        let path = cache.path.clone().expect("path retained");
        assert!(path.exists());
        assert!(path.extension().is_some_and(|ext| ext == "png"));
        cache.clear().expect("clear");
        assert!(!path.exists());
        assert!(cache.path.is_none());
    }

    #[test]
    fn clear_without_file_is_ok() {
        let mut cache = CoverFileCache::new();
        cache.clear().expect("noop clear");
    }
}
