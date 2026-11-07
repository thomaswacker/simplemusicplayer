//! Opus support. rodio's built-in symphonia decoder has no Opus codec, so Ogg/Opus
//! files are decoded here with symphonia's Ogg demuxer plus libopus.

use rodio::{ChannelCount, SampleRate, Source};
use std::num::NonZero;
use std::path::Path;
use std::time::Duration;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_OPUS, CodecRegistry, Decoder, DecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia_adapter_libopus::OpusDecoder;

pub struct OpusSource {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    buffer: Option<SampleBuffer<f32>>,
    pos: usize,
    channels: ChannelCount,
    sample_rate: SampleRate,
    total_duration: Option<Duration>,
}

impl OpusSource {
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("Datei nicht lesbar: {e}"))?;
        let mss = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());
        let mut hint = Hint::new();
        hint.with_extension("ogg");
        let probed = symphonia::default::get_probe()
            .format(
                &hint,
                mss,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .map_err(|e| format!("Format nicht unterstützt: {e}"))?;
        let format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec == CODEC_TYPE_OPUS)
            .ok_or("Keine Opus-Spur gefunden")?;

        let mut registry = CodecRegistry::new();
        registry.register_all::<OpusDecoder>();
        let decoder = registry
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(|e| format!("Opus-Decoder: {e}"))?;

        let params = &track.codec_params;
        let channels = params
            .channels
            .and_then(|c| NonZero::new(c.count() as u16))
            .ok_or("Unbekannte Kanalzahl")?;
        let sample_rate =
            NonZero::new(params.sample_rate.unwrap_or(48_000)).ok_or("Ungültige Abtastrate")?;
        let total_duration = params
            .time_base
            .zip(params.n_frames)
            .map(|(tb, n)| tb.calc_time(n).into())
            .filter(|d: &Duration| !d.is_zero());

        Ok(Self {
            track_id: track.id,
            format,
            decoder,
            buffer: None,
            pos: 0,
            channels,
            sample_rate,
            total_duration,
        })
    }

    /// Decodes the next packet into `buffer`. Returns false at end of stream.
    fn refill(&mut self) -> bool {
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                // End of file, or anything unrecoverable: end the track.
                Err(_) => return false,
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    let needed = decoded.capacity() as u64;
                    let spec = *decoded.spec();
                    if spec.channels.count() != self.channels.get() as usize {
                        // Channel layout changes mid-stream are not supported by our span model.
                        return false;
                    }
                    let buf = match &mut self.buffer {
                        Some(b) if b.capacity() >= needed as usize * spec.channels.count() => b,
                        _ => self.buffer.insert(SampleBuffer::new(needed, spec)),
                    };
                    buf.copy_interleaved_ref(decoded);
                    self.pos = 0;
                    if !buf.samples().is_empty() {
                        return true;
                    }
                }
                // Corrupt packet: skip it and keep going.
                Err(Error::DecodeError(_)) => continue,
                Err(_) => return false,
            }
        }
    }
}

impl Iterator for OpusSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        loop {
            if let Some(buf) = &self.buffer
                && let Some(&s) = buf.samples().get(self.pos)
            {
                self.pos += 1;
                return Some(s);
            }
            if !self.refill() {
                return None;
            }
        }
    }
}

impl Source for OpusSource {
    fn current_span_len(&self) -> Option<usize> {
        // Channel count and sample rate are constant for the whole stream.
        None
    }

    fn channels(&self) -> ChannelCount {
        self.channels
    }

    fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        self.total_duration
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        let target = self.total_duration.map_or(pos, |d| pos.min(d));
        self.format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time: target.into(),
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|e| rodio::source::SeekError::Other(std::sync::Arc::new(e)))?;
        self.decoder.reset();
        self.buffer = None;
        self.pos = 0;
        Ok(())
    }
}
