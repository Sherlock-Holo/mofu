use std::fs::File;
use std::path::Path;
use std::time::Duration;

use symphonia::core::audio::Channels;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::errors::SeekErrorKind;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, Track, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{
    MetadataContainer, MetadataOptions, MetadataRevision, RawValue, StandardTag, StandardVisualKey,
    Tag, Visual,
};
use symphonia::core::units::{Time, TimeBase, Timestamp};
use symphonia::default;

use super::error::{DecodeError, PlayerError, SeekError, UnsupportedMedia};
use super::format::AudioFormat;
use super::metadata::{CoverArt, TrackMetadata};

/// Owns the open demuxer/decoder for a single track and applies seek trimming.
pub(crate) struct DecoderSession {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    time_base: Option<TimeBase>,
    pcm_format: AudioFormat,
    discard_until: Option<Timestamp>,
    duration_ms: u64,
    metadata: TrackMetadata,
    interleaved: Vec<f32>,
}

/// One decode step result exposed to the worker as a pure data DTO.
pub(crate) enum DecodePacket {
    Samples {
        interleaved: Vec<f32>,
        position_ms: u64,
    },
    Ended,
}

impl DecoderSession {
    pub(crate) fn open(path: &Path) -> Result<Self, PlayerError> {
        let file = File::open(path)?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }

        let mut format = default::get_probe().probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )?;

        let track = format
            .default_track(TrackType::Audio)
            .ok_or(UnsupportedMedia::NoAudioTrack)?;
        let track_id = track.id;
        let time_base = track.time_base;
        let duration_ms = track_duration_ms(track);

        let codec_params = match track.codec_params.as_ref() {
            Some(CodecParameters::Audio(audio)) => audio.clone(),
            _ => return Err(UnsupportedMedia::MissingAudioCodecParams.into()),
        };

        let sample_rate = codec_params
            .sample_rate
            .ok_or(UnsupportedMedia::MissingSampleRate)?;
        let channels = codec_params
            .channels
            .as_ref()
            .map(Channels::count)
            .unwrap_or(1)
            .max(1);

        let decoder = default::get_codecs()
            .make_audio_decoder(&codec_params, &AudioDecoderOptions::default())?;

        let mut metadata = match format.metadata().skip_to_latest() {
            Some(revision) => RevisionMetadata::new(revision, track_id).to_track_metadata(),
            None => TrackMetadata::default(),
        };
        metadata.load_sidecar_lyrics(path);

        Ok(Self {
            format,
            decoder,
            track_id,
            time_base,
            pcm_format: AudioFormat {
                sample_rate,
                channels,
            },
            discard_until: None,
            duration_ms,
            metadata,
            interleaved: Vec::new(),
        })
    }

    pub(crate) fn pcm_format(&self) -> AudioFormat {
        self.pcm_format
    }

    pub(crate) fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    pub(crate) fn metadata(&self) -> &TrackMetadata {
        &self.metadata
    }

    pub(crate) fn seek(&mut self, position: Duration) -> Result<u64, PlayerError> {
        let target_ms = position.as_millis() as u64;
        let clamped_ms = if self.duration_ms == 0 {
            target_ms
        } else {
            target_ms.min(self.duration_ms)
        };

        let time =
            Time::try_from_secs_f64(clamped_ms as f64 / 1000.0).ok_or(PlayerError::InvalidSeek)?;

        let seeked = self
            .format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(self.track_id),
                },
            )
            .map_err(PlayerError::from)?;

        self.decoder.reset();
        self.discard_until = Some(seeked.required_ts);
        Ok(clamped_ms)
    }

    /// Decode one packet worth of interleaved f32 samples, applying seek trim when needed.
    pub(crate) fn decode_next_packet(&mut self) -> Result<DecodePacket, PlayerError> {
        loop {
            let packet = match self.format.next_packet()? {
                Some(packet) => packet,
                None => return Ok(DecodePacket::Ended),
            };

            if packet.track_id != self.track_id {
                continue;
            }

            let decoded = match self.decoder.decode(&packet) {
                Ok(decoded) => decoded,
                Err(SymphoniaError::DecodeError(_)) => continue,
                Err(err) => return Err(err.into()),
            };

            if decoded.frames() == 0 {
                continue;
            }

            let decoded_frames = decoded.frames();
            let channels = decoded.spec().channels().count().max(1);
            let packet_pts = packet.pts;

            // Prefer the layout observed from decoded frames once available.
            self.pcm_format = AudioFormat {
                sample_rate: decoded.spec().rate(),
                channels,
            };

            self.interleaved.clear();
            decoded.copy_to_vec_interleaved(&mut self.interleaved);

            let trim_frames = self.trim_frames_for_packet(packet_pts, decoded_frames);
            if trim_frames >= decoded_frames {
                continue;
            }

            let skip_samples = trim_frames.saturating_mul(channels);
            if skip_samples > 0 {
                self.interleaved
                    .drain(..skip_samples.min(self.interleaved.len()));
            }

            let position_ms = timestamp_to_ms(packet_pts, self.time_base).unwrap_or(0);
            let trim_ms = frames_to_ms(trim_frames, self.pcm_format.sample_rate);
            return Ok(DecodePacket::Samples {
                interleaved: self.interleaved.clone(),
                position_ms: position_ms.saturating_add(trim_ms),
            });
        }
    }

    fn trim_frames_for_packet(&mut self, packet_pts: Timestamp, decoded_frames: usize) -> usize {
        let Some(target) = self.discard_until else {
            return 0;
        };
        if packet_pts >= target {
            self.discard_until = None;
            return 0;
        }

        let skip = timestamp_delta_frames(
            packet_pts,
            target,
            self.time_base,
            self.pcm_format.sample_rate,
        )
        .min(decoded_frames);

        // Once this packet reaches/passes the target after trim, stop discarding.
        if skip < decoded_frames {
            self.discard_until = None;
        }
        skip
    }
}

/// Local newtype so Symphonia revision → domain DTO mapping stays on a dedicated type.
struct RevisionMetadata<'a> {
    revision: &'a MetadataRevision,
    track_id: u32,
}

impl<'a> RevisionMetadata<'a> {
    fn new(revision: &'a MetadataRevision, track_id: u32) -> Self {
        Self { revision, track_id }
    }

    fn to_track_metadata(&self) -> TrackMetadata {
        let mut meta = TrackMetadata::default();
        self.absorb_container(&mut meta, &self.revision.media);
        for per_track in &self.revision.per_track {
            if per_track.track_id == u64::from(self.track_id) {
                self.absorb_container(&mut meta, &per_track.metadata);
            }
        }
        meta
    }

    fn absorb_container(&self, meta: &mut TrackMetadata, container: &MetadataContainer) {
        for tag in &container.tags {
            self.absorb_tag(meta, tag);
        }
        if let Some(cover) = Self::pick_cover(&container.visuals) {
            meta.set_cover_if_empty(cover);
        }
    }

    fn absorb_tag(&self, meta: &mut TrackMetadata, tag: &Tag) {
        let Some(std) = tag.std.as_ref() else {
            return;
        };
        match std {
            StandardTag::TrackTitle(value) => {
                TrackMetadata::set_text_if_empty(&mut meta.title, value.as_ref().clone());
            }
            StandardTag::Artist(value) => {
                TrackMetadata::append_joined(&mut meta.artist, value.as_ref().clone());
            }
            StandardTag::Album(value) => {
                TrackMetadata::set_text_if_empty(&mut meta.album, value.as_ref().clone());
            }
            StandardTag::AlbumArtist(value) => {
                TrackMetadata::set_text_if_empty(&mut meta.album_artist, value.as_ref().clone());
            }
            StandardTag::Composer(value) => {
                TrackMetadata::set_text_if_empty(&mut meta.composer, value.as_ref().clone());
            }
            StandardTag::Lyricist(value) => {
                TrackMetadata::set_text_if_empty(&mut meta.lyricist, value.as_ref().clone());
            }
            StandardTag::Performer(value) => {
                TrackMetadata::set_text_if_empty(&mut meta.performer, value.as_ref().clone());
            }
            StandardTag::Genre(value) => {
                TrackMetadata::set_text_if_empty(&mut meta.genre, value.as_ref().clone());
            }
            StandardTag::Comment(value) => {
                TrackMetadata::set_text_if_empty(&mut meta.comment, value.as_ref().clone());
            }
            StandardTag::ReleaseYear(year)
            | StandardTag::RecordingYear(year)
            | StandardTag::OriginalReleaseYear(year) => {
                meta.set_year_if_empty(*year);
            }
            StandardTag::ReleaseDate(value) | StandardTag::RecordingDate(value) => {
                TrackMetadata::set_text_if_empty(&mut meta.date, value.as_ref().clone());
            }
            StandardTag::TrackNumber(value) => {
                TrackMetadata::set_u64_if_empty(&mut meta.track_number, *value);
            }
            StandardTag::TrackTotal(value) => {
                TrackMetadata::set_u64_if_empty(&mut meta.track_total, *value);
            }
            StandardTag::DiscNumber(value) => {
                TrackMetadata::set_u64_if_empty(&mut meta.disc_number, *value);
            }
            StandardTag::DiscTotal(value) => {
                TrackMetadata::set_u64_if_empty(&mut meta.disc_total, *value);
            }
            StandardTag::Lyrics(value) => {
                let (language, description) = Self::lyrics_sub_fields(tag);
                meta.push_embedded_plain_lyrics(value.as_ref().clone(), language, description);
            }
            _ => {}
        }
    }

    fn lyrics_sub_fields(tag: &Tag) -> (Option<String>, Option<String>) {
        let Some(sub_fields) = tag.raw.sub_fields.as_ref() else {
            return (None, None);
        };
        let mut language = None;
        let mut description = None;
        for field in sub_fields.iter() {
            let Some(text) = Self::raw_value_text(&field.value) else {
                continue;
            };
            match field.field.as_str() {
                "LANGUAGE" if language.is_none() => language = Some(text),
                "DESCRIPTION" if description.is_none() => description = Some(text),
                _ => {}
            }
        }
        (language, description)
    }

    fn raw_value_text(value: &RawValue) -> Option<String> {
        match value {
            RawValue::String(text) => Some(text.as_ref().clone()),
            RawValue::StringList(list) => {
                let joined = list.join("\n");
                if joined.is_empty() {
                    None
                } else {
                    Some(joined)
                }
            }
            _ => None,
        }
    }

    fn pick_cover(visuals: &[Visual]) -> Option<CoverArt> {
        let preferred = visuals
            .iter()
            .find(|visual| visual.usage == Some(StandardVisualKey::FrontCover))
            .or_else(|| visuals.first())?;
        Some(CoverArt {
            mime_type: preferred.media_type.clone(),
            data: preferred.data.to_vec(),
        })
    }
}

fn track_duration_ms(track: &Track) -> u64 {
    let Some(time_base) = track.time_base else {
        return 0;
    };
    let Some(duration) = track.duration else {
        return 0;
    };
    let time = time_base.calc_time_saturating(Timestamp::new(duration.get() as i64));
    (time.as_secs_f64() * 1000.0).round().max(0.0) as u64
}

fn timestamp_to_ms(ts: Timestamp, time_base: Option<TimeBase>) -> Option<u64> {
    let time_base = time_base?;
    let time = time_base.calc_time(ts)?;
    Some((time.as_secs_f64() * 1000.0).round().max(0.0) as u64)
}

fn timestamp_delta_frames(
    from: Timestamp,
    to: Timestamp,
    time_base: Option<TimeBase>,
    sample_rate: u32,
) -> usize {
    if to <= from {
        return 0;
    }

    if let Some(tb) = time_base {
        let from_t = tb.calc_time_saturating(from);
        let to_t = tb.calc_time_saturating(to);
        let delta_secs = (to_t.as_secs_f64() - from_t.as_secs_f64()).max(0.0);
        return (delta_secs * f64::from(sample_rate)).round().max(0.0) as usize;
    }

    (to.get() - from.get()).max(0) as usize
}

fn frames_to_ms(frames: usize, sample_rate: u32) -> u64 {
    if sample_rate == 0 {
        return 0;
    }
    ((frames as u64) * 1000) / u64::from(sample_rate)
}

impl From<SymphoniaError> for PlayerError {
    fn from(err: SymphoniaError) -> Self {
        match err {
            SymphoniaError::IoError(err) => Self::Io(err),
            SymphoniaError::SeekError(err) => Self::Seek(map_seek_error(err)),
            SymphoniaError::DecodeError(_) => Self::Decode(DecodeError::Malformed),
            SymphoniaError::Unsupported(_) => Self::Decode(DecodeError::UnsupportedFeature),
            SymphoniaError::LimitError(_) => Self::Decode(DecodeError::Limit),
            SymphoniaError::ResetRequired => Self::Decode(DecodeError::ResetRequired),
            _ => Self::Decode(DecodeError::Unknown),
        }
    }
}

fn map_seek_error(err: SeekErrorKind) -> SeekError {
    match err {
        SeekErrorKind::Unseekable => SeekError::Unseekable,
        SeekErrorKind::ForwardOnly => SeekError::ForwardOnly,
        SeekErrorKind::OutOfRange => SeekError::OutOfRange,
        SeekErrorKind::InvalidTrack => SeekError::InvalidTrack,
        _ => SeekError::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::*;

    #[test]
    fn frames_to_ms_basic() {
        assert_eq!(frames_to_ms(44_100, 44_100), 1000);
        assert_eq!(frames_to_ms(0, 44_100), 0);
    }

    #[test]
    fn open_and_seek_wav_fixture() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tone.wav");
        let mut session = DecoderSession::open(&path).expect("open wav");
        assert!(session.duration_ms() >= 1900);
        let pos = session.seek(Duration::from_millis(500)).expect("seek");
        assert_eq!(pos, 500);
        match session.decode_next_packet().expect("decode after seek") {
            DecodePacket::Samples {
                interleaved,
                position_ms,
            } => {
                assert!(!interleaved.is_empty());
                assert!((450..=600).contains(&position_ms));
            }
            DecodePacket::Ended => panic!("unexpected end after seek"),
        }
    }
}
