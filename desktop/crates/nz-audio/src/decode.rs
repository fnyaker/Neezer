//! From a container to interleaved stereo f32 at the device's rate.
//!
//! Two things happen here that must not happen in the output callback, and
//! therefore happen on the decode thread (`docs/agents/desktop.md`, D5
//! *Pitfalls*):
//!
//! - **Resampling.** Opus is always 48 kHz; a FLAC may be 44.1, 48, 88.2 or
//!   96 kHz. The device runs at one rate, so every voice is resampled to it with
//!   rubato's FFT resampler, chunk by chunk, with the resampler's startup delay
//!   dropped so a 1.000 s 44.1 kHz file is 1.000 s at 48 kHz.
//! - **Channel mapping.** Everything becomes stereo: mono is doubled, stereo is
//!   copied, and a multichannel file keeps its front pair (the server archives
//!   music, not films).
//!
//! Ogg Opus gapless comes for free and is worth naming: symphonia's Ogg reader
//! puts the granule position's trim into `Packet::trim_start`/`trim_end`, and
//! `symphonia-adapter-libopus` applies both plus the `OpusHead` pre-skip. Without
//! that, every Opus track would decode 6.5 ms of encoder delay at its head and
//! the seam between two tracks would be audible.

use std::io::ErrorKind;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};
use symphonia::core::codecs::audio::AudioDecoder;
use symphonia::core::codecs::audio::well_known::CODEC_ID_OPUS;
use symphonia::core::codecs::audio::{AudioDecoderOptions, CODEC_ID_NULL_AUDIO};
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::{Hint, Probe};
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::Time;

use crate::dsp::Settings;
use crate::events::{EngineEvent, EventBus};
use crate::stretch::StretchStage;
use crate::voice::{VoiceConsumer, VoiceProducer, VoiceStatus, voice_ring};

/// A voice that could not be prepared. The message is shown to the person
/// (`docs/agents/desktop.md`, rule 3: a failure says why).
#[derive(Debug)]
pub(crate) enum DecodeError {
    /// The container or codec is not one of those the app ships.
    Unsupported(String),
    /// Reading the source failed (a dropped connection, a bad file).
    Io(String),
    /// The decoder rejected the stream itself.
    Decode(String),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::Unsupported(m) | DecodeError::Io(m) | DecodeError::Decode(m) => {
                f.write_str(m)
            }
        }
    }
}

impl std::error::Error for DecodeError {}

/// Everything the decode thread needs, prepared on the loader thread: the
/// container is probed once, and a seek to the requested position has already
/// been performed on the reader (so the first decoded packet is at or before
/// `start_s`).
pub(crate) struct Prepared {
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    /// The container's own rate (48000 for Opus, anything for FLAC).
    source_rate: u32,
    channels: usize,
    /// The track's length as the container states it, when it does.
    pub(crate) duration_s: Option<f64>,
    /// How many source frames the decoder may still emit before `start_s`: the
    /// accurate seek lands on a packet boundary, and the difference is dropped.
    skip_frames: u64,
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

/// A probe that knows every container this app must play: Ogg (Opus), FLAC and
/// WAV, plus MP3/MP4 for podcast fallbacks. Built once per voice; the probe is
/// cheap (a few hundred bytes scanned), the decoder probe is not repeated.
fn probe() -> Probe {
    let mut probe = Probe::new();
    symphonia::default::register_enabled_formats(&mut probe);
    probe
}

/// Probe the source and, when `start_s` is set, seek the container to it.
/// Blocking: opening a source and reading its first bytes is I/O, so this runs
/// on a loader thread, never on the UI thread or in the callback.
pub(crate) fn prepare(
    source: Box<dyn MediaSource>,
    start_s: f64,
    target_rate: u32,
    probing: Option<Arc<std::sync::atomic::AtomicBool>>,
) -> Result<Prepared, DecodeError> {
    let stream = MediaSourceStream::new(source, MediaSourceStreamOptions::default());
    let probed = probe().probe(
        &Hint::new(),
        stream,
        FormatOptions::default(),
        MetadataOptions::default(),
    );
    // Head only, then the source may seek for real (a finished file's seek, or
    // a FLAC index search) — see `HttpSource::is_seekable`.
    if let Some(probing) = &probing {
        probing.store(false, std::sync::atomic::Ordering::Release);
    }
    let format = probed.map_err(|e| DecodeError::Unsupported(classify(e)))?;

    let track = format
        .default_track(TrackType::Audio)
        .or_else(|| format.first_track(TrackType::Audio))
        .ok_or_else(|| DecodeError::Unsupported("aucune piste audio".into()))?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| DecodeError::Unsupported("codec audio inconnu".into()))?
        .clone();
    if params.codec == CODEC_ID_NULL_AUDIO {
        return Err(DecodeError::Unsupported("codec audio inconnu".into()));
    }
    let source_rate = params
        .sample_rate
        .ok_or_else(|| DecodeError::Unsupported("fréquence d'échantillonnage absente".into()))?;
    let channels = params
        .channels
        .as_ref()
        .map(|channels| channels.count())
        .unwrap_or(2)
        .max(1);
    let duration_s = track
        .duration
        .and_then(|duration| track.time_base?.calc_duration(duration))
        .map(|time| time.as_secs_f64())
        .filter(|seconds| *seconds > 0.0);

    let mut reader: Box<dyn FormatReader> = format;
    let mut decoder = registry()
        .make_audio_decoder(
            &params,
            // A voice is decoded once; verifying a FLAC would decode it twice
            // for a checksum no listener asked for.
            &AudioDecoderOptions::default(),
        )
        .map_err(|e| DecodeError::Unsupported(classify(e)))?;

    let mut skip_frames = 0u64;
    if start_s > 0.001 {
        let time = Time::try_from_secs_f64(start_s)
            .ok_or_else(|| DecodeError::Decode("position de lecture invalide".into()))?;
        let seeked = reader
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(track_id),
                },
            )
            .map_err(|e| {
                DecodeError::Decode(format!("déplacement impossible : {}", classify(e)))
            })?;
        decoder.reset();
        // The container lands on a packet boundary at or before the target; the
        // remaining distance is dropped sample-exactly below. `target_rate` is
        // the DEVICE rate, but the frames are still at `source_rate` here.
        let actual_s = reader
            .tracks()
            .iter()
            .find(|track| track.id == seeked.track_id)
            .and_then(|track| track.time_base?.calc_time(seeked.actual_ts))
            .map(|time| time.as_secs_f64())
            .unwrap_or(0.0);
        skip_frames = ((start_s - actual_s).max(0.0) * f64::from(source_rate)).round() as u64;
    }
    let _ = target_rate;

    Ok(Prepared {
        reader,
        decoder,
        track_id,
        source_rate,
        channels,
        duration_s,
        skip_frames,
    })
}

fn classify(error: SymphoniaError) -> String {
    match error {
        SymphoniaError::IoError(e) => format!("lecture impossible : {e}"),
        SymphoniaError::DecodeError(e) => format!("décodage impossible : {e}"),
        SymphoniaError::Unsupported(e) => format!("format non pris en charge : {e}"),
        SymphoniaError::ResetRequired => "flux réinitialisé".into(),
        other => other.to_string(),
    }
}

/// Start the decode thread. Returns the voice's two ends; the caller installs
/// the consumer in the mixer and keeps the join handle (dropping it only
/// detaches, the status flags are the real stop signal).
///
/// `allow_speed` is the episode flag: only then does the output go through the
/// time-stretch stage, and only then does it read the shared speed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn(
    prepared: Prepared,
    target_rate: u32,
    ring_seconds: f32,
    generation: u64,
    position_s: f64,
    bus: Arc<EventBus>,
    id: String,
    settings: Arc<Settings>,
    allow_speed: bool,
) -> Result<(VoiceConsumer, JoinHandle<()>), DecodeError> {
    let stage = if prepared.source_rate == target_rate {
        None
    } else {
        Some(
            ResamplerStage::new(prepared.source_rate, target_rate)
                .map_err(|e| DecodeError::Unsupported(format!("rééchantillonnage : {e}")))?,
        )
    };
    let (mut producer, consumer, status) =
        voice_ring(target_rate, ring_seconds, generation, position_s);
    let handle = thread::Builder::new()
        .name(format!("nz-decode-{generation}"))
        .stack_size(512 * 1024)
        .spawn(move || {
            run(
                prepared,
                stage,
                &mut producer,
                &status,
                &bus,
                &id,
                settings,
                allow_speed,
                target_rate,
            );
        })
        .map_err(|e| DecodeError::Io(format!("thread de décodage : {e}")))?;
    Ok((consumer, handle))
}

/// Where the decode thread's frames go: the stretch stage for an episode, the
/// voice ring otherwise. One place, so the resampler does not have to know
/// whether a speed is in the path.
struct Sink<'a> {
    stretch: Option<StretchStage>,
    producer: &'a mut VoiceProducer,
    status: &'a VoiceStatus,
    settings: Arc<Settings>,
    allow_speed: bool,
}

impl Sink<'_> {
    fn feed(&mut self, frames: &[f32]) -> bool {
        let Self {
            stretch,
            producer,
            status,
            settings,
            allow_speed,
        } = self;
        match stretch.as_mut() {
            Some(stage) => {
                let speed = if *allow_speed { settings.speed() } else { 1.0 };
                let mut push = |frames: &[f32]| push_all(producer, frames, status);
                stage.push(frames, speed, &mut push)
            }
            None => push_all(producer, frames, status),
        }
    }

    /// The container ended: the stretch stage still holds audio to flush.
    fn finish(&mut self) -> bool {
        let Self {
            stretch,
            producer,
            status,
            ..
        } = self;
        match stretch.as_mut() {
            Some(stage) => {
                let mut push = |frames: &[f32]| push_all(producer, frames, status);
                stage.finish(&mut push)
            }
            None => true,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run(
    prepared: Prepared,
    mut stage: Option<ResamplerStage>,
    producer: &mut VoiceProducer,
    status: &VoiceStatus,
    bus: &EventBus,
    id: &str,
    settings: Arc<Settings>,
    allow_speed: bool,
    target_rate: u32,
) {
    let Prepared {
        mut reader,
        mut decoder,
        track_id,
        channels,
        mut skip_frames,
        ..
    } = prepared;
    let mut raw: Vec<f32> = Vec::new();
    let mut stereo: Vec<f32> = Vec::new();
    let mut sink = Sink {
        stretch: allow_speed.then(|| StretchStage::new(target_rate)),
        producer,
        status,
        settings,
        allow_speed,
    };
    loop {
        if status.is_stopped() || sink.producer.is_stopped() {
            return;
        }
        let packet = match reader.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(SymphoniaError::IoError(e)) if e.kind() == ErrorKind::Interrupted => continue,
            Err(SymphoniaError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(e) => {
                fail(bus, id, &classify(e), status);
                return;
            }
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => {
                // One undecodable packet costs that packet, not the track
                // (`docs/agents/desktop.md`, the Android player's rule as well).
                continue;
            }
            Err(SymphoniaError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(e) => {
                fail(bus, id, &classify(e), status);
                return;
            }
        };
        let frames = decoded.frames();
        if frames == 0 {
            continue;
        }
        let samples = frames * channels;
        if raw.len() != samples {
            raw.resize(samples, 0.0);
        }
        decoded.copy_to_slice_interleaved::<f32, _>(&mut raw[..samples]);
        let skipped = (skip_frames as usize).min(frames);
        skip_frames -= skipped as u64;
        if skipped == frames {
            continue;
        }
        to_stereo(&raw, skipped, frames, channels, &mut stereo);
        if let Some(stage) = stage.as_mut() {
            if let Err(message) = stage.push(&stereo, &mut sink) {
                fail(bus, id, &message, status);
                return;
            }
        } else if !sink.feed(&stereo) {
            return;
        }
    }
    if let Some(stage) = stage.as_mut()
        && let Err(message) = stage.finish(&mut sink)
    {
        fail(bus, id, &message, status);
        return;
    }
    if !sink.finish() {
        return;
    }
    status.finish();
}

fn fail(bus: &EventBus, id: &str, message: &str, status: &VoiceStatus) {
    // **A cancelled decode is not a failure.** The engine stops a voice the
    // moment the person changes track, seeks or quits; the read it was in the
    // middle of then fails (`corps interrompu`, a closed body, `Interrupted`),
    // and that failure is the click the person just made. Reporting it put an
    // error toast on every fast track change (D10b's report). A voice that was
    // stopped has nothing to tell anybody: the mixer has already forgotten it.
    if status.is_stopped() {
        return;
    }
    status.fail();
    bus.emit(EngineEvent::Error {
        id: Some(id.to_string()),
        message: message.to_string(),
    });
}

/// Interleaved `channels` → interleaved stereo.
fn to_stereo(raw: &[f32], from: usize, frames: usize, channels: usize, out: &mut Vec<f32>) {
    out.clear();
    out.reserve((frames - from) * 2);
    match channels {
        1 => {
            for sample in &raw[from..frames] {
                out.push(*sample);
                out.push(*sample);
            }
        }
        2 => {
            out.extend_from_slice(&raw[from * 2..frames * 2]);
        }
        _ => {
            for frame in from..frames {
                let base = frame * channels;
                out.push(raw[base]);
                out.push(raw[base + 1]);
            }
        }
    }
}

/// Push the whole slice, sleeping while the 0.5 s ring is full: that is the
/// decode thread's back-pressure, and it is what keeps the engine's memory flat
/// (`docs/agents/desktop.md`, D5 *Design*). Returns false when the voice was
/// stopped while waiting.
fn push_all(producer: &mut VoiceProducer, samples: &[f32], status: &VoiceStatus) -> bool {
    let mut offset = 0;
    while offset < samples.len() {
        if status.is_stopped() || producer.is_stopped() {
            return false;
        }
        let pushed = producer.push(&samples[offset..]);
        offset += pushed;
        if offset < samples.len() {
            // 10 ms: the ring is 0.5 s, so a frame of granularity never reaches
            // the output; 2 ms was 500 wakeups/s on the decoder thread.
            thread::sleep(Duration::from_millis(10));
        }
    }
    true
}

/// 44.1 kHz FLAC → 48 kHz device, on the decode thread.
struct ResamplerStage {
    fft: Fft<f32>,
    /// Interleaved stereo input of `input_frames_next()` frames.
    input: Vec<f32>,
    /// Interleaved stereo output, `output_frames_max()` frames.
    output: Vec<f32>,
    /// Frames already copied into `input`.
    filled: usize,
    /// Output frames of the resampler's own delay, still to drop.
    skip: usize,
    /// Source frames handed to the resampler, and output frames emitted after
    /// the delay: their ratio is the track's length, which the last chunk must
    /// respect (a partial chunk still produces a full FFT block, and emitting
    /// all of it added 960 frames to a one-second track — measured).
    fed: usize,
    emitted: usize,
    /// Output frames of the current chunk that are real audio.
    valid_out: usize,
}

impl ResamplerStage {
    fn new(source_rate: u32, target_rate: u32) -> Result<Self, String> {
        // 1024 frames (21 ms at 48 kHz) is rubato's recommended starting point:
        // short enough for a low delay, long enough to keep the FFT cheap.
        let mut fft = Fft::<f32>::new(
            source_rate as usize,
            target_rate as usize,
            1024,
            2,
            FixedSync::Input,
        )
        .map_err(|e| e.to_string())?;
        let input_frames = fft.input_frames_next();
        let output_frames = fft.output_frames_max();
        let skip = fft.output_delay();
        fft.reset();
        Ok(Self {
            fft,
            input: vec![0.0; input_frames * 2],
            output: vec![0.0; output_frames * 2],
            filled: 0,
            skip,
            fed: 0,
            emitted: 0,
            valid_out: output_frames,
        })
    }

    /// Feed interleaved stereo; emits whatever the resampler produced.
    fn push(&mut self, stereo: &[f32], sink: &mut Sink) -> Result<(), String> {
        let chunk_frames = self.input.len() / 2;
        self.fed += stereo.len() / 2;
        let mut offset = 0;
        while offset < stereo.len() / 2 {
            let take = (chunk_frames - self.filled).min(stereo.len() / 2 - offset);
            self.input[self.filled * 2..(self.filled + take) * 2]
                .copy_from_slice(&stereo[offset * 2..(offset + take) * 2]);
            self.filled += take;
            offset += take;
            if self.filled == chunk_frames {
                self.filled = 0;
                self.emit(sink, None)?;
            }
        }
        Ok(())
    }

    /// The container ended: process the partial chunk, then trim to the length
    /// the ratio promises so a 1.000 s source stays 1.000 s. The tail of the
    /// filter's output is real audio, not padding: dropping it cut 246 frames
    /// off a second at 44.1 → 48 kHz.
    fn finish(&mut self, sink: &mut Sink) -> Result<(), String> {
        if self.filled > 0 {
            let filled = self.filled;
            self.filled = 0;
            self.emit(sink, Some(filled))?;
        }
        Ok(())
    }

    /// One resampler call. `partial` is the number of real frames still in the
    /// buffer when the container ended.
    fn emit(&mut self, sink: &mut Sink, partial: Option<usize>) -> Result<(), String> {
        let chunk_frames = self.input.len() / 2;
        let out_frames = self.output.len() / 2;
        // `output_frames_max` is the worst case over every call; a given call
        // writes `output_frames_next`, and emitting the maximum would add a
        // tail of stale samples to every chunk (measured: 54 954 frames for one
        // second instead of 48 000).
        let produced = self.fft.output_frames_next().min(out_frames);
        self.valid_out = match partial {
            Some(_) => {
                // The container ended: the target length is the ratio applied to
                // every real frame, and the FFT's overshoot beyond it is the
                // filter's tail, not silence.
                let target = (self.fed as f64 * self.fft.resample_ratio()).round() as usize;
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
        let input = InterleavedSlice::new(&self.input, 2, chunk_frames)
            .map_err(|e| format!("tampon d'entrée du rééchantillonneur : {e}"))?;
        let mut output = InterleavedSlice::new_mut(&mut self.output, 2, out_frames)
            .map_err(|e| format!("tampon de sortie du rééchantillonneur : {e}"))?;
        self.fft
            .process_into_buffer(&input, &mut output, Some(&indexing))
            .map_err(|e| format!("rééchantillonnage : {e}"))?;
        let mut frames = self.valid_out;
        let skip = self.skip.min(frames);
        self.skip -= skip;
        frames -= skip;
        self.emitted += frames;
        if frames == 0 {
            return Ok(());
        }
        let start = skip * 2;
        if !sink.feed(&self.output[start..start + frames * 2]) {
            return Err("lecture interrompue".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The FFT resampler's startup delay must be dropped, not left as leading
    /// silence, and the length must follow the ratio: 1.000 s of 44.1 kHz is
    /// 1.000 s at 48 kHz (± one frame).
    #[test]
    fn resampling_keeps_the_duration_and_the_leading_edge() {
        let source_rate = 44_100u32;
        let target_rate = 48_000u32;
        let frames = source_rate as usize; // exactly one second
        let mut stereo = Vec::with_capacity(frames * 2);
        // A cosine starts at 1.0, so leading silence is impossible to miss.
        for i in 0..frames {
            let value = (std::f64::consts::TAU * 1_000.0 * i as f64 / f64::from(source_rate)).cos();
            stereo.push(value as f32);
            stereo.push(value as f32);
        }
        let (mut producer, mut consumer, status) = voice_ring(target_rate, 4.0, 1, 0.0);
        let mut stage = ResamplerStage::new(source_rate, target_rate).expect("resampler");
        let mut sink = Sink {
            stretch: None,
            producer: &mut producer,
            status: &status,
            settings: Settings::new(),
            allow_speed: false,
        };
        stage.push(&stereo, &mut sink).expect("resample");
        stage.finish(&mut sink).expect("flush");

        let mut out = Vec::new();
        while let Some((left, _right)) = consumer.frame() {
            out.push(left);
        }
        assert!(
            (out.len() as i64 - target_rate as i64).abs() <= 2,
            "expected about {} frames at 48 kHz, got {}",
            target_rate,
            out.len()
        );
        assert!(
            out[0] > 0.9,
            "the resampler's delay must be dropped, first sample was {}",
            out[0]
        );
        // 1 kHz survives the rate change: count zero crossings over 0.5 s.
        let crossings = out[..out.len() / 2]
            .windows(2)
            .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
            .count();
        assert!(
            (crossings as i64 - 1_000).abs() <= 2,
            "expected 1 000 crossings in 0.5 s, got {crossings}"
        );
    }

    #[test]
    fn mono_is_doubled_and_multichannel_keeps_the_front_pair() {
        let mut out = Vec::new();
        to_stereo(&[0.1, 0.2, 0.3], 0, 3, 1, &mut out);
        assert_eq!(out, vec![0.1, 0.1, 0.2, 0.2, 0.3, 0.3]);

        let mut out = Vec::new();
        let raw = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]; // two 4-channel frames
        to_stereo(&raw, 0, 2, 4, &mut out);
        assert_eq!(out, vec![1.0, 2.0, 5.0, 6.0]);

        let mut out = Vec::new();
        to_stereo(&[1.0, 2.0, 3.0, 4.0], 1, 2, 2, &mut out);
        assert_eq!(out, vec![3.0, 4.0], "a seek skip drops whole source frames");
    }

    /// **A cancelled decode says nothing.** A stopped voice's read fails where it
    /// stood (`corps interrompu`, a body closed by the server, a file that went
    /// away), and that failure is the person's own track change: it must not
    /// become an `EngineEvent::Error` — which the app turns into an error toast
    /// ("Lecture impossible"). A voice that was NOT stopped still reports, so a
    /// real decode failure keeps its cause.
    #[test]
    fn a_stopped_voice_reports_nothing_and_a_live_one_reports_its_cause() {
        let bus = EventBus::new();
        let events = bus.subscribe();

        let cancelled = VoiceStatus::default();
        cancelled.stop();
        fail(
            &bus,
            "track-a",
            "lecture impossible : corps interrompu",
            &cancelled,
        );
        assert!(
            events.try_recv().is_err(),
            "a cancelled voice must not raise an error"
        );

        let live = VoiceStatus::default();
        fail(&bus, "track-b", "décodage impossible : bitstream", &live);
        let event = events.try_recv().expect("a real failure is reported");
        match event {
            EngineEvent::Error { id, message } => {
                assert_eq!(id.as_deref(), Some("track-b"));
                assert!(message.contains("décodage impossible"));
            }
            other => panic!("expected an Error, got {other:?}"),
        }
        assert!(live.is_stopped() || live.is_ended(), "the voice is over");
    }

    #[test]
    fn the_codec_registry_knows_opus_through_the_bundled_libopus() {
        // `symphonia::default` registers FLAC/WAV/MP3 but never Opus: without
        // this registration an Opus track fails with "unsupported codec".
        assert!(registry().get_audio_decoder(CODEC_ID_OPUS).is_some());
    }
}
