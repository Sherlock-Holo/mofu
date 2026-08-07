use std::io;

use thiserror::Error;

/// Errors produced by the music player.
///
/// Domain-only: backend crates map into these variants at module boundaries.
#[derive(Debug, Error)]
pub enum PlayerError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error(transparent)]
    Unsupported(#[from] UnsupportedMedia),
    #[error(transparent)]
    Seek(#[from] SeekError),
    #[error(transparent)]
    Output(#[from] OutputError),
    #[error("player command channel closed")]
    CommandChannelClosed,
    #[error("player command timed out")]
    CommandTimedOut,
    #[error("invalid seek position")]
    InvalidSeek,
    #[error("queue index is out of range")]
    InvalidQueueIndex,
}

/// Demux/decode failures that are not plain I/O or media-open rejection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DecodeError {
    #[error("malformed stream")]
    Malformed,
    #[error("unsupported feature")]
    UnsupportedFeature,
    #[error("limit reached")]
    Limit,
    #[error("decoder needs to be reset")]
    ResetRequired,
    /// Reserved for decoder-backend variants added in future releases.
    #[error("unknown decode failure")]
    Unknown,
}

/// Media that cannot be opened with the current decoder pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum UnsupportedMedia {
    #[error("no audio track")]
    NoAudioTrack,
    #[error("selected track has no audio codec parameters")]
    MissingAudioCodecParams,
    #[error("audio track missing sample rate")]
    MissingSampleRate,
}

/// Seek failures reported by the demuxer (distinct from [`PlayerError::InvalidSeek`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SeekError {
    #[error("stream is not seekable")]
    Unseekable,
    #[error("stream can only be seeked forward")]
    ForwardOnly,
    #[error("requested seek timestamp is out-of-range for stream")]
    OutOfRange,
    #[error("invalid track id")]
    InvalidTrack,
    /// Reserved for demuxer seek kinds added in future releases.
    #[error("unknown seek failure")]
    Unknown,
}

/// Audio device / resampling failures (mapped from the output backend).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum OutputError {
    #[error("no default output device")]
    NoDefaultDevice,
    #[error("audio device is busy")]
    DeviceBusy,
    #[error("audio device route changed")]
    DeviceChanged,
    #[error("audio device is not available")]
    DeviceNotAvailable,
    #[error("audio host is unavailable")]
    HostUnavailable,
    #[error("invalid audio device input")]
    InvalidInput,
    #[error("audio device permission denied")]
    PermissionDenied,
    #[error("audio realtime scheduling denied")]
    RealtimeDenied,
    #[error("audio device resource exhausted")]
    ResourceExhausted,
    #[error("audio stream configuration invalidated")]
    StreamInvalidated,
    #[error("unsupported audio stream configuration")]
    UnsupportedConfig,
    #[error("unsupported audio device operation")]
    UnsupportedOperation,
    #[error("audio buffer underrun or overrun")]
    Xrun,
    #[error("audio backend error")]
    Backend,
    #[error("audio device error")]
    DeviceOther,
    /// Reserved for device-backend kinds added in future releases.
    #[error("unknown audio device failure")]
    DeviceUnknown,
    #[error("unsupported output sample format")]
    UnsupportedSampleFormat,
    #[error("resampler initialization failed")]
    ResamplerInit,
    #[error("interleaved sample count is not a multiple of channel count")]
    InterleavedLengthMismatch,
    #[error("resampler input layout is invalid")]
    ResamplerInput,
    #[error("resample failed")]
    Resample,
}
