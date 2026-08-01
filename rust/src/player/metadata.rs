/// Embedded cover image (raw encoded bytes, typically JPEG/PNG).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CoverArt {
    pub mime_type: Option<String>,
    pub data: Vec<u8>,
}

/// Where lyrics content came from (independent of transport / UI).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LyricsSource {
    /// Embedded in the audio file (ID3 USLT, Vorbis UNSYNCEDLYRICS, etc.).
    Embedded,
    // /// Sidecar lyrics file (e.g. `.lrc` / `.txt`). Not filled this phase.
    // ExternalFile,
    // /// Fetched from a remote lyrics provider. Not filled this phase.
    // Network,
}

// /// One timed lyric line.
// Not filled this phase (no timed lyrics yet).
// #[derive(Debug, Clone, PartialEq, Eq)]
// pub(crate) struct LyricsLine {
//     pub start_ms: u64,
//     pub end_ms: Option<u64>,
//     pub text: String,
// }

/// Lyrics body: plain text or timed lines (shared by embedded / file / network).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LyricsContent {
    Plain { text: String },
    // Timed { lines: Vec<LyricsLine> },
}

/// One lyrics track (language / source / content shape).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Lyrics {
    pub source: LyricsSource,
    pub language: Option<String>,
    pub description: Option<String>,
    pub content: LyricsContent,
}

/// Tags, cover, and lyrics for the currently open track.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct TrackMetadata {
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
    pub cover: Option<CoverArt>,
    pub lyrics: Vec<Lyrics>,
}

impl TrackMetadata {
    /// Keep the first non-empty assignment for a single-valued text field.
    pub(crate) fn set_text_if_empty(slot: &mut Option<String>, value: String) {
        if slot.is_none() && !value.is_empty() {
            *slot = Some(value);
        }
    }

    /// Append a value to a joined multi-value field (e.g. multiple artists).
    pub(crate) fn append_joined(slot: &mut Option<String>, value: String) {
        if value.is_empty() {
            return;
        }
        match slot {
            Some(existing) => {
                if !existing.is_empty() {
                    existing.push_str("; ");
                }
                existing.push_str(&value);
            }
            None => *slot = Some(value),
        }
    }

    /// Keep the first year assignment.
    pub(crate) fn set_year_if_empty(&mut self, year: u16) {
        if self.year.is_none() {
            self.year = Some(year);
        }
    }

    /// Keep the first assignment for a numeric field.
    pub(crate) fn set_u64_if_empty(slot: &mut Option<u64>, value: u64) {
        if slot.is_none() {
            *slot = Some(value);
        }
    }

    /// Keep the first cover art assignment.
    pub(crate) fn set_cover_if_empty(&mut self, cover: CoverArt) {
        if self.cover.is_none() {
            self.cover = Some(cover);
        }
    }

    /// Append an embedded plain-text lyrics entry.
    pub(crate) fn push_embedded_plain_lyrics(
        &mut self,
        text: String,
        language: Option<String>,
        description: Option<String>,
    ) {
        if text.is_empty() {
            return;
        }
        self.lyrics.push(Lyrics {
            source: LyricsSource::Embedded,
            language,
            description,
            content: LyricsContent::Plain { text },
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_joined_merges_artists() {
        let mut artist = None;
        TrackMetadata::append_joined(&mut artist, "A".into());
        TrackMetadata::append_joined(&mut artist, "B".into());
        assert_eq!(artist.as_deref(), Some("A; B"));
    }

    #[test]
    fn set_text_if_empty_keeps_first() {
        let mut title = None;
        TrackMetadata::set_text_if_empty(&mut title, "One".into());
        TrackMetadata::set_text_if_empty(&mut title, "Two".into());
        assert_eq!(title.as_deref(), Some("One"));
    }

    #[test]
    fn push_embedded_plain_lyrics_skips_empty() {
        let mut meta = TrackMetadata::default();
        meta.push_embedded_plain_lyrics(String::new(), None, None);
        assert!(meta.lyrics.is_empty());
        meta.push_embedded_plain_lyrics("hello".into(), Some("eng".into()), None);
        assert_eq!(meta.lyrics.len(), 1);
        assert_eq!(meta.lyrics[0].source, LyricsSource::Embedded);
        assert_eq!(meta.lyrics[0].language.as_deref(), Some("eng"));
        assert!(matches!(
            &meta.lyrics[0].content,
            LyricsContent::Plain { text } if text == "hello"
        ));
    }
}
