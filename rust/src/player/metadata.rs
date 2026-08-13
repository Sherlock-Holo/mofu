use std::fs;
use std::path::Path;
use std::sync::Arc;

/// Embedded cover image (raw encoded bytes, typically JPEG/PNG).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverArt {
    pub mime_type: Option<String>,
    pub data: Vec<u8>,
}

/// Where lyrics content came from (independent of transport / UI).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LyricsSource {
    /// Embedded in the audio file (ID3 USLT, Vorbis UNSYNCEDLYRICS, etc.).
    Embedded,
    /// Loaded from the same-name `.lrc` next to the audio file.
    ExternalFile,
}

/// One timed lyric line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricsLine {
    pub start_ms: u64,
    pub end_ms: Option<u64>,
    pub text: String,
}

/// Lyrics body: plain text or timed lines (shared by embedded / file / network).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LyricsContent {
    Plain { text: String },
    Timed { lines: Vec<LyricsLine> },
}

/// One lyrics track (language / source / content shape).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lyrics {
    pub source: LyricsSource,
    pub language: Option<String>,
    pub description: Option<String>,
    pub content: LyricsContent,
}

/// Tags, cover, and lyrics for the currently open track.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrackMetadata {
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
    pub cover: Option<Arc<CoverArt>>,
    pub lyrics: Vec<Lyrics>,
}

impl TrackMetadata {
    /// Display title with path basename fallback when the tag is missing/blank.
    pub fn display_title(path: &str, title: Option<&str>) -> String {
        if let Some(title) = title {
            let trimmed = title.trim();
            if !trimmed.is_empty() {
                return trimmed.to_owned();
            }
        }
        Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("未知曲目")
            .to_owned()
    }

    /// Index of the active timed lyric line at `position_ms`, if any.
    pub fn active_lyrics_line_index(&self, position_ms: u64) -> Option<usize> {
        let lines = self
            .lyrics
            .iter()
            .find_map(|lyrics| match &lyrics.content {
                LyricsContent::Timed { lines } if !lines.is_empty() => Some(lines.as_slice()),
                _ => None,
            })?;
        let mut active = 0usize;
        for (index, line) in lines.iter().enumerate() {
            if line.start_ms <= position_ms {
                active = index;
            } else {
                break;
            }
        }
        Some(active)
    }

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
            self.cover = Some(Arc::new(cover));
        }
    }

    /// Tags only: drop cover bytes and lyric bodies for progress-tick fan-out.
    pub(crate) fn without_heavy_fields(&self) -> Self {
        Self {
            title: self.title.clone(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            album_artist: self.album_artist.clone(),
            composer: self.composer.clone(),
            lyricist: self.lyricist.clone(),
            performer: self.performer.clone(),
            genre: self.genre.clone(),
            comment: self.comment.clone(),
            date: self.date.clone(),
            year: self.year,
            track_number: self.track_number,
            track_total: self.track_total,
            disc_number: self.disc_number,
            disc_total: self.disc_total,
            cover: None,
            lyrics: Vec::new(),
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
        let content = LrcDocument::parse(&text)
            .map(|lines| LyricsContent::Timed { lines })
            .unwrap_or(LyricsContent::Plain { text });
        self.lyrics.push(Lyrics {
            source: LyricsSource::Embedded,
            language,
            description,
            content,
        });
    }

    /// Load a same-name `.lrc` when embedded metadata has no timed lyrics.
    pub(crate) fn load_sidecar_lyrics(&mut self, media_path: &Path) {
        if self
            .lyrics
            .iter()
            .any(|lyrics| matches!(lyrics.content, LyricsContent::Timed { .. }))
        {
            return;
        }
        let sidecar = media_path.with_extension("lrc");
        let Ok(bytes) = fs::read(&sidecar) else {
            return;
        };
        let text = String::from_utf8_lossy(&bytes)
            .trim_start_matches('\u{feff}')
            .to_owned();
        if text.trim().is_empty() {
            return;
        }
        let content = LrcDocument::parse(&text)
            .map(|lines| LyricsContent::Timed { lines })
            .unwrap_or(LyricsContent::Plain { text });
        self.lyrics.push(Lyrics {
            source: LyricsSource::ExternalFile,
            language: None,
            description: sidecar
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned),
            content,
        });
    }
}

/// Parser for conventional LRC timestamp lines.
struct LrcDocument;

impl LrcDocument {
    fn parse(text: &str) -> Option<Vec<LyricsLine>> {
        let mut lines = Vec::new();
        for raw_line in text.lines() {
            let mut rest = raw_line.trim();
            let mut timestamps = Vec::new();
            while let Some(after_open) = rest.strip_prefix('[') {
                let Some(close) = after_open.find(']') else {
                    break;
                };
                let token = &after_open[..close];
                let Some(timestamp) = Self::parse_timestamp(token) else {
                    break;
                };
                timestamps.push(timestamp);
                rest = &after_open[close + 1..];
            }
            for start_ms in timestamps {
                lines.push(LyricsLine {
                    start_ms,
                    end_ms: None,
                    text: rest.trim().to_owned(),
                });
            }
        }
        if lines.is_empty() {
            return None;
        }
        lines.sort_by_key(|line| line.start_ms);
        for index in 0..lines.len().saturating_sub(1) {
            let next_start = lines[index + 1].start_ms;
            lines[index].end_ms = Some(next_start);
        }
        Some(lines)
    }

    fn parse_timestamp(token: &str) -> Option<u64> {
        let parts: Vec<&str> = token.split(':').collect();
        let (hours, minutes, seconds) = match parts.as_slice() {
            [minutes, seconds] => (0, minutes.parse::<u64>().ok()?, *seconds),
            [hours, minutes, seconds] => (
                hours.parse::<u64>().ok()?,
                minutes.parse::<u64>().ok()?,
                *seconds,
            ),
            _ => return None,
        };
        let (whole_seconds, fraction_ms) = match seconds.split_once('.') {
            Some((whole, fraction)) => {
                (whole.parse::<u64>().ok()?, Self::fraction_to_ms(fraction)?)
            }
            None => (seconds.parse::<u64>().ok()?, 0),
        };
        if minutes >= 60 || whole_seconds >= 60 {
            return None;
        }
        Some(
            hours
                .saturating_mul(3_600_000)
                .saturating_add(minutes.saturating_mul(60_000))
                .saturating_add(whole_seconds.saturating_mul(1_000))
                .saturating_add(fraction_ms),
        )
    }

    fn fraction_to_ms(fraction: &str) -> Option<u64> {
        if fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let mut digits = fraction.bytes().take(3).collect::<Vec<_>>();
        while digits.len() < 3 {
            digits.push(b'0');
        }
        str::from_utf8(&digits).ok()?.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

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

    #[test]
    fn lrc_parses_multiple_tags_and_assigns_end_times() {
        let lines =
            LrcDocument::parse("[00:01.20][00:02.345]hello\n[01:02]world\n[ar:ignored]\nnot timed")
                .expect("timed lyrics");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].start_ms, 1_200);
        assert_eq!(lines[0].end_ms, Some(2_345));
        assert_eq!(lines[1].start_ms, 2_345);
        assert_eq!(lines[2].start_ms, 62_000);
        assert_eq!(lines[2].text, "world");
    }

    #[test]
    fn invalid_lrc_has_no_timed_lines() {
        assert!(LrcDocument::parse("[ar:artist]\nplain lyrics").is_none());
        assert!(LrcDocument::parse("[00:99]invalid").is_none());
    }

    #[test]
    fn sidecar_is_loaded_unless_embedded_timed_lyrics_exist() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("mofu-lrc-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&directory).expect("create test directory");
        let media = directory.join("song.mp3");
        fs::write(media.with_extension("lrc"), "[00:01.00]sidecar").expect("write sidecar");

        let mut metadata = TrackMetadata::default();
        metadata.push_embedded_plain_lyrics("plain".into(), None, None);
        metadata.load_sidecar_lyrics(&media);
        assert_eq!(metadata.lyrics.len(), 2);
        assert_eq!(metadata.lyrics[1].source, LyricsSource::ExternalFile);
        assert!(matches!(
            metadata.lyrics[1].content,
            LyricsContent::Timed { .. }
        ));

        let mut timed = TrackMetadata::default();
        timed.push_embedded_plain_lyrics("[00:02]embedded".into(), None, None);
        timed.load_sidecar_lyrics(&media);
        assert_eq!(timed.lyrics.len(), 1);
        assert_eq!(timed.lyrics[0].source, LyricsSource::Embedded);
        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn display_title_prefers_tag_then_basename() {
        assert_eq!(
            TrackMetadata::display_title("/music/a.mp3", Some("  标题  ")),
            "标题"
        );
        assert_eq!(
            TrackMetadata::display_title("/music/a.mp3", Some("   ")),
            "a.mp3"
        );
        assert_eq!(TrackMetadata::display_title("/music/a.mp3", None), "a.mp3");
    }

    #[test]
    fn active_lyrics_line_tracks_position() {
        let metadata = TrackMetadata {
            lyrics: vec![Lyrics {
                source: LyricsSource::Embedded,
                language: None,
                description: None,
                content: LyricsContent::Timed {
                    lines: vec![
                        LyricsLine {
                            start_ms: 0,
                            end_ms: Some(1000),
                            text: "a".into(),
                        },
                        LyricsLine {
                            start_ms: 1000,
                            end_ms: Some(2000),
                            text: "b".into(),
                        },
                    ],
                },
            }],
            ..Default::default()
        };
        assert_eq!(metadata.active_lyrics_line_index(0), Some(0));
        assert_eq!(metadata.active_lyrics_line_index(1500), Some(1));
        assert_eq!(TrackMetadata::default().active_lyrics_line_index(0), None);
    }
}
