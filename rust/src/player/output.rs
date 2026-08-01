use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, ChannelCount, Device, Error as CpalError, ErrorKind, FromSample, SampleFormat,
    SizedSample, Stream, StreamConfig, SupportedStreamConfig,
};
use rtrb::chunks::ChunkError;
use rtrb::{Consumer, Producer, RingBuffer};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};
use tracing::error;

use super::error::{OutputError, PlayerError};
use super::format::AudioFormat;
use super::state::SharedVolume;

/// Outcome of a non-blocking write into the audio ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteStatus {
    /// All queued samples are in the ring.
    Drained,
    /// Ring was full; remaining samples stay buffered for a later flush.
    Pending,
}

/// Coordinates draining stale samples after a seek without blocking the realtime callback for long.
struct FlushGate {
    requested: AtomicU64,
    acknowledged: AtomicU64,
}

impl FlushGate {
    fn new() -> Self {
        Self {
            requested: AtomicU64::new(0),
            acknowledged: AtomicU64::new(0),
        }
    }

    fn request_flush(&self) -> u64 {
        self.requested.fetch_add(1, Ordering::AcqRel) + 1
    }

    fn requested(&self) -> u64 {
        self.requested.load(Ordering::Acquire)
    }

    fn acknowledge(&self, token: u64) {
        self.acknowledged.store(token, Ordering::Release);
    }

    fn wait_acknowledged(&self, token: u64) {
        // Audio callbacks typically run every few ms; bound the wait so seek never hangs forever.
        for _ in 0..200 {
            if self.acknowledged.load(Ordering::Acquire) >= token {
                return;
            }
            thread::sleep(Duration::from_millis(1));
        }
    }
}

struct RateConverter {
    resampler: Fft<f32>,
    pending: Vec<f32>,
    channels: usize,
}

impl RateConverter {
    fn try_new(input_rate: u32, output_rate: u32, channels: usize) -> Result<Self, PlayerError> {
        let chunk_size = 1024;
        let resampler = Fft::<f32>::new(
            input_rate as usize,
            output_rate as usize,
            chunk_size,
            channels,
            FixedSync::Input,
        )
        .map_err(|_| OutputError::ResamplerInit)?;

        Ok(Self {
            resampler,
            pending: Vec::new(),
            channels,
        })
    }

    fn reset(&mut self) {
        self.resampler.reset();
        self.pending.clear();
    }

    fn push_interleaved(&mut self, samples: &[f32], out: &mut Vec<f32>) -> Result<(), PlayerError> {
        if samples.is_empty() {
            return Ok(());
        }
        if !samples.len().is_multiple_of(self.channels) {
            return Err(OutputError::InterleavedLengthMismatch.into());
        }

        self.pending.extend_from_slice(samples);

        loop {
            let frames_needed = self.resampler.input_frames_next();
            let samples_needed = frames_needed * self.channels;
            if self.pending.len() < samples_needed {
                break;
            }

            let chunk: Vec<f32> = self.pending.drain(..samples_needed).collect();
            let frames = chunk.len() / self.channels;
            let input = InterleavedSlice::new(&chunk, self.channels, frames)
                .map_err(|_| OutputError::ResamplerInput)?;
            let produced = self
                .resampler
                .process(&input, None)
                .map_err(|_| OutputError::Resample)?;
            out.extend_from_slice(&produced.take_data());
        }

        Ok(())
    }
}

/// CPAL-backed output: decode thread writes into a lock-free ring; the stream callback only reads.
pub(crate) struct CpalOutput {
    producer: Producer<f32>,
    flush: Arc<FlushGate>,
    _stream: Stream,
    converter: Option<RateConverter>,
    /// Resampled (or passthrough) samples waiting for free ring slots.
    ring_pending: Vec<f32>,
    scratch: Vec<f32>,
}

impl CpalOutput {
    /// Open the default output device, preferring the source sample rate when possible.
    pub(crate) fn try_open(
        format: AudioFormat,
        volume: Arc<SharedVolume>,
    ) -> Result<Self, PlayerError> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or(OutputError::NoDefaultDevice)?;

        let default_config = device.default_output_config().map_err(map_cpal_error)?;

        let channels = format.channels.max(1);
        let source_rate = format.sample_rate;
        let device_rate = default_config.sample_rate();

        // Prefer playing at the source rate to avoid resampling when the host allows it.
        let (config, output_rate, needs_resample) =
            match try_source_rate_config(&device, &default_config, channels as u16, source_rate) {
                Some(config) => (config, source_rate, false),
                None => {
                    let mut config: StreamConfig = default_config.config();
                    config.channels = channels as ChannelCount;
                    (config, device_rate, source_rate != device_rate)
                }
            };

        let ring_capacity = ((200 * output_rate as usize) / 1000)
            .saturating_mul(channels)
            .max(channels * 32);
        let (producer, consumer) = RingBuffer::<f32>::new(ring_capacity);
        let flush = Arc::new(FlushGate::new());
        let callback_flush = flush.clone();

        let stream = match default_config.sample_format() {
            SampleFormat::F32 => {
                build_stream::<f32>(&device, &config, consumer, callback_flush, volume)?
            }
            SampleFormat::I16 => {
                build_stream::<i16>(&device, &config, consumer, callback_flush, volume)?
            }
            SampleFormat::U16 => {
                build_stream::<u16>(&device, &config, consumer, callback_flush, volume)?
            }
            _ => return Err(OutputError::UnsupportedSampleFormat.into()),
        };

        stream.play().map_err(map_cpal_error)?;

        let converter = if needs_resample {
            Some(RateConverter::try_new(source_rate, output_rate, channels)?)
        } else {
            None
        };

        Ok(Self {
            producer,
            flush,
            _stream: stream,
            converter,
            ring_pending: Vec::new(),
            scratch: Vec::new(),
        })
    }

    /// Enqueue samples and push as many as possible into the ring without blocking.
    pub(crate) fn write_interleaved(
        &mut self,
        samples: &[f32],
    ) -> Result<WriteStatus, PlayerError> {
        if samples.is_empty() {
            return Ok(self.flush_ring_pending());
        }

        if let Some(converter) = self.converter.as_mut() {
            self.scratch.clear();
            converter.push_interleaved(samples, &mut self.scratch)?;
            self.ring_pending.extend_from_slice(&self.scratch);
        } else {
            self.ring_pending.extend_from_slice(samples);
        }

        Ok(self.flush_ring_pending())
    }

    /// Retry pushing any buffered samples that did not fit earlier.
    pub(crate) fn try_flush(&mut self) -> WriteStatus {
        self.flush_ring_pending()
    }

    /// Whether samples are still waiting to enter the ring.
    pub(crate) fn has_pending(&self) -> bool {
        !self.ring_pending.is_empty()
    }

    pub(crate) fn clear_buffered(&mut self) {
        if let Some(converter) = self.converter.as_mut() {
            converter.reset();
        }
        self.scratch.clear();
        self.ring_pending.clear();
        let token = self.flush.request_flush();
        self.flush.wait_acknowledged(token);
    }

    /// Push as many `ring_pending` samples as currently fit; never blocks.
    fn flush_ring_pending(&mut self) -> WriteStatus {
        while !self.ring_pending.is_empty() {
            let want = self.ring_pending.len();
            let chunk = match self.producer.write_chunk_uninit(want) {
                Ok(chunk) => chunk,
                Err(ChunkError::TooFewSlots(0)) => return WriteStatus::Pending,
                Err(ChunkError::TooFewSlots(available)) => {
                    match self.producer.write_chunk_uninit(available) {
                        Ok(chunk) => chunk,
                        Err(_) => return WriteStatus::Pending,
                    }
                }
            };

            let n = chunk.len();
            let written = chunk.fill_from_iter(self.ring_pending.drain(..n));
            debug_assert_eq!(written, n);
        }
        WriteStatus::Drained
    }
}

fn map_cpal_error(err: CpalError) -> OutputError {
    match err.kind() {
        ErrorKind::DeviceBusy => OutputError::DeviceBusy,
        ErrorKind::DeviceChanged => OutputError::DeviceChanged,
        ErrorKind::DeviceNotAvailable => OutputError::DeviceNotAvailable,
        ErrorKind::HostUnavailable => OutputError::HostUnavailable,
        ErrorKind::InvalidInput => OutputError::InvalidInput,
        ErrorKind::PermissionDenied => OutputError::PermissionDenied,
        ErrorKind::RealtimeDenied => OutputError::RealtimeDenied,
        ErrorKind::ResourceExhausted => OutputError::ResourceExhausted,
        ErrorKind::StreamInvalidated => OutputError::StreamInvalidated,
        ErrorKind::UnsupportedConfig => OutputError::UnsupportedConfig,
        ErrorKind::UnsupportedOperation => OutputError::UnsupportedOperation,
        ErrorKind::Xrun => OutputError::Xrun,
        ErrorKind::BackendError => OutputError::Backend,
        ErrorKind::Other => OutputError::DeviceOther,
        _ => OutputError::DeviceUnknown,
    }
}

fn try_source_rate_config(
    device: &Device,
    default_config: &SupportedStreamConfig,
    channels: u16,
    source_rate: u32,
) -> Option<StreamConfig> {
    if source_rate == default_config.sample_rate() {
        let mut config: StreamConfig = default_config.config();
        config.channels = channels;
        return Some(config);
    }

    let range = device.supported_output_configs().ok()?;
    for supported in range {
        if supported.channels() == channels
            && supported.min_sample_rate() <= source_rate
            && supported.max_sample_rate() >= source_rate
            && supported.sample_format() == default_config.sample_format()
        {
            return Some(StreamConfig {
                channels,
                sample_rate: source_rate,
                buffer_size: BufferSize::Default,
            });
        }
    }
    None
}

fn build_stream<T>(
    device: &Device,
    config: &StreamConfig,
    mut consumer: Consumer<f32>,
    flush: Arc<FlushGate>,
    volume: Arc<SharedVolume>,
) -> Result<Stream, PlayerError>
where
    T: SizedSample + FromSample<f32>,
{
    let mut local_flush = 0_u64;
    let err_fn = |err| error!(%err, "cpal stream error");

    device
        .build_output_stream(
            *config,
            move |data: &mut [T], _| {
                let requested = flush.requested();
                if requested != local_flush {
                    while consumer.pop().is_ok() {}
                    local_flush = requested;
                    flush.acknowledge(requested);
                }

                let gain = volume.get();
                for sample in data.iter_mut() {
                    let value = consumer.pop().unwrap_or(0.0) * gain;
                    *sample = T::from_sample(value);
                }
            },
            err_fn,
            None,
        )
        .map_err(|err| map_cpal_error(err).into())
}
