//! Decoding an episode to 16 kHz mono in this process (symphonia, the libopus
//! adapter, rubato, and hound for WAV) — owned by lot T2.
//!
//! The desktop has no ffmpeg dependency: the server shells out to `ffmpeg` for a
//! chunk (`engine.py#extract_chunk`), this crate decodes in-process instead. What
//! that buys is a pipeline with no external binary to find, version or install
//! (rule 7: an AppImage or an Arch package cannot assume `ffmpeg`), and a decode
//! that costs one pass instead of a subprocess per chunk.
//!
//! Three facts the rest of the crate (and T3's supervisor) build on:
//!
//! - A `--pcm` file is RAW little-endian `f32`, one channel, 16 kHz, no header
//!   ([`write_pcm`]/[`read_pcm`]). Its duration is therefore a multiplication
//!   ([`pcm_duration_ms`]), which is how the supervisor knows how far a chunk
//!   got without parsing audio.
//! - Everything is mono: the model was trained on one channel, and the server's
//!   `ffmpeg -ac 1` does the same average (`(l + r) / 2`), so a stereo episode
//!   transcribes the way it does on the server. A multichannel file keeps its
//!   front pair's average — the server archives podcasts and music, not films.
//! - Everything is 16 kHz ([`SAMPLE_RATE`]): anything else is resampled with
//!   rubato's FFT resampler, whose own start-up delay is dropped (a 1.000 s file
//!   stays 1.000 s) and whose last partial block is trimmed to the length the
//!   ratio promises — the two traps `nz-audio` measured (54 954 frames for a
//!   second; 246 frames cut off a second's tail).
//!
//! WAV is decoded by `hound`, not symphonia: the workspace builds symphonia with
//! `ogg`, `flac`, `mp3`, `isomp4` and `aac` only (the formats an episode and a
//! music file arrive in), and `jfk.wav` — the acceptance fixture — is a WAV.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};
use symphonia::core::audio::GenericAudioBufferRef;
use symphonia::core::codecs::audio::well_known::CODEC_ID_OPUS;
use symphonia::core::codecs::audio::{AudioDecoderOptions, CODEC_ID_NULL_AUDIO};
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::{Hint, Probe};
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;

use crate::error::Error;
use crate::segments::SAMPLE_RATE;

/// Bytes one sample of a `--pcm` file takes: an `f32`.
pub const SAMPLE_BYTES: u64 = 4;

/// Bytes of a `--pcm` file per millisecond: 4 bytes × 16 000 samples/s ÷ 1 000.
pub const BYTES_PER_MS: u64 = SAMPLE_BYTES * SAMPLE_RATE as u64 / 1000;

/// Frames of a `--pcm` file per millisecond: [`SAMPLE_RATE`] ÷ 1 000.
pub const FRAMES_PER_MS: usize = SAMPLE_RATE as usize / 1000;

/// Duration of a raw `--pcm` file in whole milliseconds, rounded down.
///
/// The caller that resumes a job compares this to the chunk it asked for, so a
/// short read (a killed child, a truncated file) is visible instead of silent.
pub fn pcm_duration_ms(bytes: u64) -> u64 {
    bytes / BYTES_PER_MS
}

/// Duration of decoded 16 kHz mono samples in whole milliseconds, rounded down.
pub fn samples_duration_ms(samples: usize) -> u64 {
    (samples / FRAMES_PER_MS) as u64
}

/// Decode a whole audio file to 16 kHz mono `f32` samples.
///
/// `Err(Error::NoAudio)` when the container, the codec or the file cannot be
/// read; the detail is logged, never shown raw (the person reads the crate's
/// French sentence, `error.rs`).
pub fn decode_file(path: &Path) -> Result<Vec<f32>, Error> {
    decode_range(path, 0, i64::MAX)
}

/// Decode the `[start_ms, start_ms + duration_ms)` window of a file to 16 kHz
/// mono.
///
/// The window is applied while decoding, so an hour-long episode costs the window
/// and one packet, never the whole file: that is what lets the supervisor give
/// the engine one 600 s chunk without holding a three-hour episode's samples.
/// `duration_ms` is clamped to what the file has.
pub fn decode_range(path: &Path, start_ms: i64, duration_ms: i64) -> Result<Vec<f32>, Error> {
    let start = start_ms.max(0) as usize;
    let limit = if duration_ms < 0 {
        usize::MAX
    } else {
        (duration_ms as usize).saturating_mul(FRAMES_PER_MS)
    };
    if is_wav(path) {
        decode_wav(path, start, limit)
    } else {
        let file = File::open(path).map_err(|error| {
            tracing::warn!("audio {} : {error}", path.display());
            Error::NoAudio
        })?;
        decode_stream(Box::new(file), start, limit)
    }
}

/// Decode any container symphonia knows (Ogg Opus, FLAC, MP3, MP4/AAC) to
/// 16 kHz mono, keeping the `skip`-th to `skip + limit`-th output sample.
fn decode_stream(
    source: Box<dyn MediaSource>,
    skip: usize,
    limit: usize,
) -> Result<Vec<f32>, Error> {
    let stream = MediaSourceStream::new(source, MediaSourceStreamOptions::default());
    let probed = probe().probe(
        &Hint::new(),
        stream,
        FormatOptions::default(),
        MetadataOptions::default(),
    );
    let format = probed.map_err(|error| {
        tracing::warn!("format : {}", classify(&error));
        Error::NoAudio
    })?;
    let track = format
        .default_track(TrackType::Audio)
        .or_else(|| format.first_track(TrackType::Audio))
        .ok_or(Error::NoAudio)?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|params| params.audio())
        .ok_or(Error::NoAudio)?
        .clone();
    if params.codec == CODEC_ID_NULL_AUDIO {
        return Err(Error::NoAudio);
    }
    let source_rate = params.sample_rate.ok_or(Error::NoAudio)?;
    let channels = params
        .channels
        .as_ref()
        .map(|channels| channels.count())
        .unwrap_or(1)
        .max(1);

    let mut reader: Box<dyn FormatReader> = format;
    let mut decoder = registry()
        .make_audio_decoder(
            &params,
            // A transcription decodes once: verifying a FLAC would decode it
            // twice for a checksum nobody asked for.
            &AudioDecoderOptions::default(),
        )
        .map_err(|error| {
            tracing::warn!("codec : {}", classify(&error));
            Error::NoAudio
        })?;

    let mut collector = Collector::new(source_rate, skip, limit);
    let mut raw: Vec<f32> = Vec::new();
    loop {
        let packet = match reader.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(SymphoniaError::IoError(error)) if error.kind() == io::ErrorKind::Interrupted => {
                continue;
            }
            Err(SymphoniaError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(error) => {
                // A truncated tail costs the frames it held, not the episode: a
                // podcast that ends mid-packet still transcribes its speech.
                tracing::warn!("lecture : {}", classify(&error));
                break;
            }
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(reason)) => {
                tracing::warn!("paquet ignoré : {reason}");
                continue;
            }
            Err(SymphoniaError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(error) => {
                tracing::warn!("décodage : {}", classify(&error));
                break;
            }
        };
        let frames = decoded.frames();
        if frames == 0 {
            continue;
        }
        raw.resize(frames * channels, 0.0);
        copy_frames(&decoded, &mut raw[..frames * channels]);
        collector.push(&raw, frames, channels);
        if !collector.wants_more() {
            break;
        }
    }
    Ok(collector.finish())
}

/// A WAV file: hound rather than symphonia (see the module doc).
fn decode_wav(path: &Path, skip: usize, limit: usize) -> Result<Vec<f32>, Error> {
    let mut reader = hound::WavReader::open(path).map_err(|error| {
        tracing::warn!("wav {} : {error}", path.display());
        Error::NoAudio
    })?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;
    let mut collector = Collector::new(spec.sample_rate, skip, limit);
    let mut raw: Vec<f32> = Vec::new();
    match spec.sample_format {
        hound::SampleFormat::Float => {
            let mut frames = 0usize;
            for sample in reader.samples::<f32>() {
                let sample = sample.map_err(|error| {
                    tracing::warn!("wav : {error}");
                    Error::NoAudio
                })?;
                raw.push(sample);
                frames += 1;
                if frames == channels {
                    collector.push(&raw, 1, channels);
                    raw.clear();
                    frames = 0;
                    if !collector.wants_more() {
                        break;
                    }
                }
            }
        }
        hound::SampleFormat::Int => {
            let mut frames = 0usize;
            // 8- and 16-bit samples are signed shorts in hound; 17 to 32 bit
            // arrive as `i32`. Both are scaled by their own full range, the way
            // `ffmpeg` converts to float.
            if spec.bits_per_sample <= 16 {
                for sample in reader.samples::<i16>() {
                    let sample = sample.map_err(|error| {
                        tracing::warn!("wav : {error}");
                        Error::NoAudio
                    })?;
                    raw.push(f32::from(sample) / 32_768.0);
                    frames += 1;
                    if frames == channels {
                        collector.push(&raw, 1, channels);
                        raw.clear();
                        frames = 0;
                        if !collector.wants_more() {
                            break;
                        }
                    }
                }
            } else {
                for sample in reader.samples::<i32>() {
                    let sample = sample.map_err(|error| {
                        tracing::warn!("wav : {error}");
                        Error::NoAudio
                    })?;
                    raw.push(sample as f32 / 2_147_483_648.0);
                    frames += 1;
                    if frames == channels {
                        collector.push(&raw, 1, channels);
                        raw.clear();
                        frames = 0;
                        if !collector.wants_more() {
                            break;
                        }
                    }
                }
            }
        }
    }
    Ok(collector.finish())
}

/// Whether `path` starts with a RIFF/WAVE header, read from 12 bytes (never the
/// whole file).
fn is_wav(path: &Path) -> bool {
    let mut header = [0u8; 12];
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    if file.read_exact(&mut header).is_err() {
        return false;
    }
    &header[..4] == b"RIFF" && &header[8..12] == b"WAVE"
}

/// Every sample format symphonia produced, as interleaved `f32` (`i16` becomes
/// `x / 32768`, which is `convert_integer_to_float_audio`'s own scale).
fn copy_frames(decoded: &GenericAudioBufferRef<'_>, out: &mut [f32]) {
    decoded.copy_to_slice_interleaved::<f32, _>(out);
}

/// The codec registry with the one decoder symphonia lacks: Opus, from the
/// bundled libopus adapter.
fn registry() -> CodecRegistry {
    let mut registry = CodecRegistry::new();
    symphonia::default::register_enabled_codecs(&mut registry);
    if registry.get_audio_decoder(CODEC_ID_OPUS).is_none() {
        registry.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
    }
    registry
}

/// The probe with every container this crate can read: Ogg (Opus), FLAC, MP3,
/// MP4/AAC.
fn probe() -> Probe {
    let mut probe = Probe::new();
    symphonia::default::register_enabled_formats(&mut probe);
    probe
}

/// A decode failure, in the words the log wants (the person reads the crate's
/// French sentence instead).
fn classify(error: &SymphoniaError) -> String {
    match error {
        SymphoniaError::IoError(error) => format!("lecture impossible : {error}"),
        SymphoniaError::DecodeError(reason) => format!("décodage impossible : {reason}"),
        SymphoniaError::Unsupported(reason) => format!("format non pris en charge : {reason}"),
        SymphoniaError::ResetRequired => "flux réinitialisé".into(),
        other => other.to_string(),
    }
}

/// Interleaved `channels` → mono (`(l + r) / 2`, the server's `-ac 1`) → 16 kHz,
/// keeping one window of the result.
struct Collector {
    resampler: Option<Fft<f32>>,
    /// Interleaved mono input of `input_frames_next()` frames.
    input: Vec<f32>,
    /// Mono output, `output_frames_next()` frames.
    output: Vec<f32>,
    /// Frames already copied into `input`.
    filled: usize,
    /// Output samples of the resampler's own delay, still to drop.
    delay: usize,
    /// Source frames handed to the resampler, and output samples emitted after
    /// its delay: their ratio is the audio's length, which the last chunk must
    /// respect (a partial chunk still produces a full FFT block).
    fed: usize,
    emitted: usize,
    /// Target samples still to drop before the window starts.
    drop: usize,
    /// Target samples the window may still take.
    keep: usize,
    out: Vec<f32>,
}

impl Collector {
    fn new(source_rate: u32, skip: usize, limit: usize) -> Self {
        let (resampler, input, output, delay) = if source_rate == SAMPLE_RATE {
            (None, Vec::new(), Vec::new(), 0)
        } else {
            match Fft::<f32>::new(
                source_rate as usize,
                SAMPLE_RATE as usize,
                1024,
                1,
                FixedSync::Input,
            ) {
                Ok(mut fft) => {
                    let input = fft.input_frames_next();
                    let output = fft.output_frames_max();
                    let delay = fft.output_delay();
                    fft.reset();
                    (Some(fft), vec![0.0; input], vec![0.0; output], delay)
                }
                Err(error) => {
                    // A rate rubato refuses (an absurd one from a broken
                    // container): the samples are then kept as they are, and the
                    // duration check downstream says so.
                    tracing::warn!("rééchantillonnage {source_rate} → {SAMPLE_RATE} : {error}");
                    (None, Vec::new(), Vec::new(), 0)
                }
            }
        };
        Self {
            resampler,
            input,
            output,
            filled: 0,
            delay,
            fed: 0,
            emitted: 0,
            drop: skip,
            keep: limit,
            out: Vec::new(),
        }
    }

    /// Whether the window is complete: no more decoding needed.
    fn wants_more(&self) -> bool {
        self.keep > 0
    }

    /// Feed `frames` interleaved frames of `channels` channels.
    fn push(&mut self, raw: &[f32], frames: usize, channels: usize) {
        if self.keep == 0 || frames == 0 {
            return;
        }
        // One mono buffer per packet: decoding is a background job, so this is
        // a few hundred kB per second of audio and nothing else allocates.
        let mut mono: Vec<f32> = Vec::with_capacity(frames);
        match channels {
            1 => mono.extend_from_slice(&raw[..frames]),
            2 => {
                let (pairs, _) = raw[..frames * 2].as_chunks::<2>();
                for pair in pairs {
                    mono.push((pair[0] + pair[1]) / 2.0);
                }
            }
            _ => {
                for frame in raw[..frames * channels].chunks_exact(channels) {
                    mono.push((frame[0] + frame[1]) / 2.0);
                }
            }
        }
        if self.resampler.is_none() {
            self.push_target(&mono);
            return;
        }
        self.fed += mono.len();
        let chunk = self.input.len();
        let mut offset = 0;
        while offset < mono.len() {
            let take = (chunk - self.filled).min(mono.len() - offset);
            self.input[self.filled..self.filled + take]
                .copy_from_slice(&mono[offset..offset + take]);
            self.filled += take;
            offset += take;
            if self.filled == chunk {
                self.filled = 0;
                self.emit(None);
            }
        }
    }

    /// The audio ended: process the partial chunk and trim the resampler's tail
    /// to the length the ratio promises.
    fn finish(mut self) -> Vec<f32> {
        if self.resampler.is_some() && self.filled > 0 {
            let filled = self.filled;
            self.filled = 0;
            self.emit(Some(filled));
        }
        self.out
    }

    /// One resampler call. `partial` is the number of real frames left in the
    /// buffer when the audio ended.
    fn emit(&mut self, partial: Option<usize>) {
        let Some(fft) = self.resampler.as_mut() else {
            return;
        };
        let chunk = self.input.len();
        let out_frames = self.output.len();
        // `output_frames_max` is the worst case over every call; a given call
        // writes `output_frames_next`, and emitting the maximum added a tail of
        // stale samples to every chunk (54 954 frames for one second, measured
        // by `nz-audio`).
        let produced = fft.output_frames_next().min(out_frames);
        let valid = match partial {
            Some(_) => {
                let target = (self.fed as f64 * fft.resample_ratio()).round() as usize;
                produced.min(target.saturating_sub(self.emitted))
            }
            None => produced,
        };
        let indexing = rubato::Indexing {
            input_offset: 0,
            output_offset: 0,
            partial_len: partial,
            active_channels_mask: None,
        };
        let input = match InterleavedSlice::new(&self.input, 1, chunk) {
            Ok(input) => input,
            Err(error) => {
                tracing::warn!("tampon d'entrée du rééchantillonneur : {error}");
                self.keep = 0;
                return;
            }
        };
        let mut output = match InterleavedSlice::new_mut(&mut self.output, 1, out_frames) {
            Ok(output) => output,
            Err(error) => {
                tracing::warn!("tampon de sortie du rééchantillonneur : {error}");
                self.keep = 0;
                return;
            }
        };
        if let Err(error) = fft.process_into_buffer(&input, &mut output, Some(&indexing)) {
            tracing::warn!("rééchantillonnage : {error}");
            self.keep = 0;
            return;
        }
        let mut frames = valid;
        let delay = self.delay.min(frames);
        self.delay -= delay;
        frames -= delay;
        self.emitted += frames;
        let start = delay;
        let copy = self.output[start..start + frames].to_vec();
        self.push_target(&copy);
    }

    /// Keep the part of `samples` that falls inside the requested window.
    fn push_target(&mut self, samples: &[f32]) {
        let mut samples = samples;
        if self.drop > 0 {
            let take = self.drop.min(samples.len());
            self.drop -= take;
            samples = &samples[take..];
        }
        let take = self.keep.min(samples.len());
        self.out.extend_from_slice(&samples[..take]);
        self.keep -= take;
    }
}

/// Write 16 kHz mono samples as a raw `--pcm` file (little-endian `f32`).
pub fn write_pcm(path: &Path, samples: &[f32]) -> io::Result<()> {
    let mut file = File::create(path)?;
    let mut bytes = Vec::with_capacity(samples.len() * 4);
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    file.write_all(&bytes)
}

/// Read a raw `--pcm` file back. A length that is not a multiple of four is a
/// truncated file, not audio.
pub fn read_pcm(path: &Path) -> Result<Vec<f32>, Error> {
    let bytes = std::fs::read(path).map_err(|error| {
        tracing::warn!("pcm {} : {error}", path.display());
        Error::NoAudio
    })?;
    if bytes.len() % 4 != 0 {
        tracing::warn!(
            "pcm {} : {} octets, pas un multiple de 4",
            path.display(),
            bytes.len()
        );
        return Err(Error::NoAudio);
    }
    let (words, _) = bytes.as_chunks::<4>();
    Ok(words.iter().map(|word| f32::from_le_bytes(*word)).collect())
}

#[cfg(test)]
mod tests {
    use super::{
        BYTES_PER_MS, FRAMES_PER_MS, SAMPLE_RATE, pcm_duration_ms, read_pcm, samples_duration_ms,
        write_pcm,
    };
    use crate::segments::CHUNK_SECONDS;

    /// A chunk of ten minutes is exactly `CHUNK_SECONDS` of this file, and the
    /// conversion is a division, not a decode.
    #[test]
    fn a_chunk_of_pcm_is_a_multiplication() {
        assert_eq!(SAMPLE_RATE, 16_000);
        assert_eq!(BYTES_PER_MS, 64);
        assert_eq!(FRAMES_PER_MS, 16);
        assert_eq!(pcm_duration_ms(64), 1);
        assert_eq!(pcm_duration_ms(0), 0);
        assert_eq!(
            pcm_duration_ms(BYTES_PER_MS * CHUNK_SECONDS as u64 * 1000),
            (CHUNK_SECONDS * 1000) as u64
        );
        // Half a millisecond of samples is not a millisecond.
        assert_eq!(pcm_duration_ms(95), 1);
        assert_eq!(samples_duration_ms(16_000), 1000);
        assert_eq!(samples_duration_ms(15), 0);
    }

    /// A `--pcm` file is exactly the samples it holds: same values, same order,
    /// and an alignment check that refuses a truncated write.
    #[test]
    fn a_pcm_file_round_trips_and_refuses_a_truncated_one() {
        let dir = std::env::temp_dir().join(format!(
            "nz-audio-pcm-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("scratch");
        let samples: Vec<f32> = (0..1600).map(|i| (i as f32 / 1600.0) - 0.5).collect();
        let path = dir.join("chunk.pcm");
        write_pcm(&path, &samples).expect("write");
        assert_eq!(
            pcm_duration_ms(std::fs::metadata(&path).unwrap().len()),
            100
        );
        assert_eq!(read_pcm(&path), Ok(samples));

        let truncated = dir.join("truncated.pcm");
        std::fs::write(&truncated, [0u8; 6]).expect("write");
        assert_eq!(read_pcm(&truncated), Err(crate::error::Error::NoAudio));
        assert_eq!(
            read_pcm(&dir.join("absent.pcm")),
            Err(crate::error::Error::NoAudio)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
