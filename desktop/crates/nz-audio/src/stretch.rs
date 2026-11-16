//! Podcast speed, pitch kept.
//!
//! The web player sets `element.playbackRate` (the SPA's `speed.js` offers
//! 0.75–3×), which every browser implements as a time-stretch that keeps the
//! pitch; the desktop must do the same or a 1.5× episode sounds like a chipmunk.
//! The algorithm is Signalsmith Stretch (`signalsmith-stretch`, MIT, no system
//! library), a phase-vocoder/STFT hybrid built for exactly this: hand it `n`
//! input frames and ask for `n / rate` output frames, and it re-times without
//! transposing.
//!
//! Three rules this file enforces:
//!
//! - **Music never goes through it.** The stage is applied only to a voice the
//!   app marked as an episode ([`crate::engine::TrackMeta::podcast`]), and at
//!   exactly 1× it is bypassed entirely — a phase vocoder is not
//!   bit-transparent, and a song must sound like the file.
//! - **It runs on the DECODE thread**, in front of the voice ring, never in the
//!   output callback: an FFT per 21 ms of audio is cheap, but the callback's
//!   budget belongs to the mix (D6's p99 criterion).
//! - **The length follows the rate.** A 2.000 s episode at 2× must come out at
//!   1.000 s, so the timeline, the seek bar and the trim end stay in media
//!   seconds and the clock scales by the same rate. A phase vocoder cannot be
//!   sample-exact at the head — it needs a window of history before its first
//!   output, so an episode opens on one latency's worth of silence (60 ms at
//!   48 kHz, measured) — and the stage flushes that same delay's worth of REAL
//!   audio at the tail, then trims to `fed / speed`: the length is exact and
//!   the last word is not lost.

use signalsmith_stretch::Stretch;

/// The SPA's `sane()` accepts 0.5–4 (`speed.js`), its list stops at 3×
/// (`SPEEDS`), and the lot's contract is 0.5–3×.
pub const MIN_SPEED: f32 = 0.5;
pub const MAX_SPEED: f32 = 3.0;

/// The size of one `process` call: 21 ms at 48 kHz, the scale of the preset's
/// own interval (0.03 s) and short enough that a speed change lands promptly.
const CHUNK_FRAMES: usize = 1_024;

/// A finite rate inside the app's range, or exactly 1× (which bypasses the
/// stage): a NaN or a stale 4× from another client is never played.
pub fn clamp_speed(speed: f32) -> f32 {
    if !speed.is_finite() {
        return 1.0;
    }
    let speed = speed.clamp(MIN_SPEED, MAX_SPEED);
    if (speed - 1.0).abs() < 1e-3 {
        1.0
    } else {
        speed
    }
}

/// The speed stage, fed interleaved stereo at the device rate.
pub(crate) struct StretchStage {
    stretch: Stretch,
    /// Input frames accumulated for the next call, interleaved.
    input: Vec<f32>,
    /// Where the next input frame goes (`input` is `CHUNK_FRAMES` long).
    filled: usize,
    /// The output of one call, interleaved. Sized for the slowest rate.
    output: Vec<f32>,
    /// Frames handed in, and frames emitted: the tail is trimmed to their
    /// ratio, so a 2.000 s episode at 2× is 1.000 s.
    fed: u64,
    emitted: u64,
    /// How far behind the input the algorithm's output is, in frames.
    latency: usize,
    /// Has the analysis window been primed with pre-roll silence?
    primed: bool,
    /// The rate the current chunk is being processed at.
    speed: f32,
}

impl StretchStage {
    pub(crate) fn new(sample_rate: u32) -> Self {
        let mut stretch = Stretch::preset_default(2, sample_rate.max(8_000));
        // Time stretch only: the pitch is the recording's.
        stretch.set_transpose_factor(1.0, None);
        let latency = stretch.output_latency();
        // The output buffer holds one chunk at the slowest rate, and the flush's
        // `2 · latency` frames (what `SignalsmithStretch::flush` requires).
        let output_frames = (CHUNK_FRAMES * 2).max(latency * 2 + 2);
        Self {
            stretch,
            input: vec![0.0; CHUNK_FRAMES * 2],
            filled: 0,
            output: vec![0.0; output_frames * 2],
            fed: 0,
            emitted: 0,
            latency,
            primed: false,
            speed: 1.0,
        }
    }

    /// Feed interleaved stereo media frames; `sink` gets the real-time frames
    /// this produces (a chunk at a time) and returns `false` when the voice was
    /// stopped. Returns `false` then too.
    pub(crate) fn push(
        &mut self,
        stereo: &[f32],
        speed: f32,
        sink: &mut dyn FnMut(&[f32]) -> bool,
    ) -> bool {
        let speed = clamp_speed(speed);
        self.speed = speed;
        if !self.primed {
            self.prime(speed);
        }
        let mut frames = stereo.len() / 2;
        let mut offset = 0usize;
        self.fed += frames as u64;
        while frames > 0 {
            let take = (CHUNK_FRAMES - self.filled).min(frames);
            self.input[self.filled * 2..(self.filled + take) * 2]
                .copy_from_slice(&stereo[offset * 2..(offset + take) * 2]);
            self.filled += take;
            offset += take;
            frames -= take;
            if self.filled == CHUNK_FRAMES {
                self.filled = 0;
                if !self.emit(speed, None, sink) {
                    return false;
                }
            }
        }
        true
    }

    /// The container ended: process the partial chunk (padded with silence,
    /// which is trimmed off), then flush the audio the algorithm still holds so
    /// the last word is not lost.
    pub(crate) fn finish(&mut self, sink: &mut dyn FnMut(&[f32]) -> bool) -> bool {
        if self.filled > 0 {
            self.filled = 0;
            let frames = self.target_frames();
            if !self.emit(self.speed, Some(frames), sink) {
                return false;
            }
        }
        if self.latency == 0 {
            return true;
        }
        let len = (self.latency * 2).min(self.output.len() / 2) * 2;
        let target = self.target_frames();
        self.flush_block(len, target, sink)
    }

    /// Real-time frames the fed input is worth: the timeline is media time, so
    /// `fed / speed` frames must come out, no more.
    fn target_frames(&self) -> usize {
        (self.fed as f64 / f64::from(self.speed)).round() as usize
    }

    /// Prime the analysis window with pre-roll silence: without it the first
    /// audible block reconstructs the silence before the track.
    fn prime(&mut self, speed: f32) {
        self.primed = true;
        // As much history as the algorithm can look back on: its own analysis
        // latency plus a chunk. `seek` pads to a whole window by itself.
        let frames = self.stretch.input_latency() * 2 + CHUNK_FRAMES;
        let preroll = vec![0.0f32; frames * 2];
        self.stretch.seek(preroll, f64::from(speed));
    }

    /// One `process` call: `CHUNK_FRAMES` of input, `round(chunk / speed)` of
    /// output, handed to `sink`.
    ///
    /// `limit` is the total number of output frames the track may still have
    /// (the container's end); `None` means "as much as the rate says".
    fn emit(
        &mut self,
        speed: f32,
        limit: Option<usize>,
        sink: &mut dyn FnMut(&[f32]) -> bool,
    ) -> bool {
        let out_frames = (CHUNK_FRAMES as f64 / f64::from(speed)).round() as usize;
        let out_frames = out_frames.clamp(1, self.output.len() / 2);
        {
            let output = &mut self.output[..out_frames * 2];
            self.stretch.process(&self.input, output);
        }
        let keep = match limit {
            Some(limit) => {
                (limit as i64 - self.emitted as i64).clamp(0, out_frames as i64) as usize
            }
            None => out_frames,
        };
        self.emit_front(keep, sink)
    }

    /// Flush the algorithm's tail: the frames it still holds after the last real
    /// input.
    fn flush_block(
        &mut self,
        len: usize,
        target: usize,
        sink: &mut dyn FnMut(&[f32]) -> bool,
    ) -> bool {
        {
            let output = &mut self.output[..len];
            for value in output.iter_mut() {
                *value = 0.0;
            }
            self.stretch.flush(output);
        }
        let frames = len / 2;
        let keep = (target as i64 - self.emitted as i64).clamp(0, frames as i64) as usize;
        self.emit_front(keep, sink)
    }

    /// Count what is kept and hand it over.
    fn emit_front(&mut self, frames: usize, sink: &mut dyn FnMut(&[f32]) -> bool) -> bool {
        if frames == 0 {
            return true;
        }
        self.emitted += frames as u64;
        sink(&self.output[..frames * 2])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 440 Hz cosine at unit amplitude, `seconds` long, stereo. A cosine
    /// starts at +1, so a dropped or extra frame at the head is visible.
    fn tone(seconds: f64, sample_rate: u32) -> Vec<f32> {
        let frames = (seconds * f64::from(sample_rate)) as usize;
        (0..frames)
            .flat_map(|i| {
                let value = (core::f64::consts::TAU * 440.0 * i as f64 / f64::from(sample_rate))
                    .cos() as f32;
                [value, value]
            })
            .collect()
    }

    fn run(input: &[f32], speed: f32, sample_rate: u32) -> Vec<f32> {
        let mut stage = StretchStage::new(sample_rate);
        let mut out = Vec::new();
        let mut sink = |frames: &[f32]| {
            out.extend_from_slice(frames);
            true
        };
        assert!(stage.push(input, speed, &mut sink));
        assert!(stage.finish(&mut sink));
        out
    }

    /// 2.000 s at 2× is 1.000 s: the timeline is media time, so the clock and
    /// the seek bar keep working.
    #[test]
    fn a_double_speed_episode_is_half_as_long() {
        let out = run(&tone(2.0, 48_000), 2.0, 48_000);
        let frames = out.len() / 2;
        println!(
            "2.000 s at 2x → {frames} frames ({:.4} s)",
            frames as f64 / 48_000.0
        );
        assert!(
            (frames as i64 - 48_000).abs() < 480,
            "expected 48 000 frames, got {frames}"
        );
    }

    /// The pitch is KEPT: a 440 Hz tone stretched by 2× still crosses zero 880
    /// times per second (a resampling player would give 880 Hz).
    #[test]
    fn the_pitch_survives_the_stretch() {
        let out = run(&tone(2.0, 48_000), 2.0, 48_000);
        let left: Vec<f32> = out
            .as_chunks::<2>()
            .0
            .iter()
            .map(|frame| frame[0])
            .collect();
        // Skip the algorithm's onset, count crossings over 0.5 s.
        let start = left.len() / 4;
        let window = &left[start..start + 24_000];
        let crossings = window
            .windows(2)
            .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
            .count();
        let hz = crossings as f64 / 2.0 / 0.5;
        println!("440 Hz at 2x: {hz:.1} Hz measured");
        assert!((hz - 440.0).abs() < 15.0, "pitch moved to {hz:.1} Hz");
    }

    /// 0.75× is longer, and the length follows the rate the same way.
    #[test]
    fn a_slower_speed_stretches_the_timeline() {
        let out = run(&tone(1.0, 48_000), 0.75, 48_000);
        let frames = out.len() / 2;
        let expected = (48_000.0f64 / 0.75).round() as i64;
        println!("1.000 s at 0.75x → {frames} frames (expected {expected})");
        assert!((frames as i64 - expected).abs() < 960, "got {frames}");
    }

    /// The stage costs the algorithm's own latency at the head and nothing
    /// more: a phase vocoder needs a window of history before its first output,
    /// so an episode opens on `input_latency + output_latency` frames (5 760 =
    /// 120 ms at 48 kHz), and full level is reached inside it. The tail keeps
    /// the real audio, which the flush is for.
    ///
    /// Measured on speech-level material (0.1): loud from frame 4 780 (99.6 ms),
    /// steady level 0.100, last 10 ms 0.100. At FULL scale the same onset sums
    /// to +4.7 dB for one 10 ms window (the STFT's windows are not yet fully
    /// overlapping); a podcast sits 10–20 dB below that, and the master limiter
    /// is behind this stage for anyone who brings a hot file.
    #[test]
    fn the_opening_costs_only_the_algorithms_latency() {
        let mut input = tone(2.0, 48_000);
        for sample in input.iter_mut() {
            *sample *= 0.1;
        }
        let stage = StretchStage::new(48_000);
        let latency = stage.stretch.input_latency() + stage.stretch.output_latency();
        let mut out = Vec::new();
        let mut sink = |frames: &[f32]| {
            out.extend_from_slice(frames);
            true
        };
        let mut stage = stage;
        assert!(stage.push(&input, 1.5, &mut sink));
        assert!(stage.finish(&mut sink));
        let left: Vec<f32> = out
            .as_chunks::<2>()
            .0
            .iter()
            .map(|frame| frame[0])
            .collect();
        let first_loud = left
            .iter()
            .position(|sample| sample.abs() > 0.05)
            .expect("the tone starts");
        let peak = left.iter().fold(0f32, |m, v| m.max(v.abs()));
        let steady = left[left.len() / 2..left.len() / 2 + 4_800]
            .iter()
            .fold(0f32, |m, v| m.max(v.abs()));
        let tail = left[left.len() - 480..]
            .iter()
            .fold(0f32, |m, v| m.max(v.abs()));
        println!(
            "latency {latency} frames; loud from frame {first_loud} ({:.1} ms); peak {peak:.4}; steady {steady:.4}; last 10 ms {tail:.4}",
            first_loud as f64 / 48.0
        );
        assert!(
            first_loud <= latency,
            "the opening's silence must be the stage's latency ({latency} frames), saw {first_loud}"
        );
        assert!(
            (steady - 0.1).abs() < 0.01,
            "the steady level must be the source's 0.1, got {steady}"
        );
        assert!(peak < 0.25, "the onset spikes to {peak} for a 0.1 source");
        assert!(
            tail > 0.05,
            "the tail's real audio must survive, got {tail}"
        );
    }

    #[test]
    fn the_rate_is_clamped_and_one_bypasses_the_stage() {
        assert_eq!(clamp_speed(f32::NAN), 1.0);
        assert_eq!(clamp_speed(0.1), MIN_SPEED);
        assert_eq!(clamp_speed(9.0), MAX_SPEED);
        assert_eq!(clamp_speed(1.0004), 1.0);
        assert_eq!(clamp_speed(1.5), 1.5);
    }
}
