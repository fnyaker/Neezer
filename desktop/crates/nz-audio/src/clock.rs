//! The playback clock: what the person is hearing right now.
//!
//! Three numbers, all atomics written by the output callback and read by the
//! UI: the position in the current track where the voice started (0, a seek
//! target, or the server's `X-NS-Start` for a `?start=` slice), the frames the
//! mixer has handed to the device since, and the device's own latency. The
//! position is `base + frames / rate - latency`, because the samples still in
//! the device's buffer have not reached the speakers yet.
//!
//! The callback never locks: every field is an `AtomicU64`, and the only
//! inconsistency a reader can see is a position one output period stale, which
//! is below the seek tolerance the player already allows
//! (`webapp/src/lib/audio/seekplan.js#SEEK_TOLERANCE` is 1.5 s).

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Position of the sound now leaving the speakers, in seconds.
#[derive(Debug)]
pub struct Clock {
    sample_rate: u32,
    base_bits: AtomicU64,
    frames: AtomicU64,
    latency_frames: AtomicU64,
    /// Frames handed to the device since the stream opened. Unlike `frames` it is
    /// never reset by a voice: it is the OUTPUT clock's own timeline, which is
    /// what a listen-party guest schedules on (see [`Clock::output_anchor`]).
    stream_frames: AtomicU64,
    /// The pair (device instant, stream position heard at that instant), written
    /// together by the output callback and read as one value (a seqlock: the odd
    /// generation says a write is in progress).
    anchor_gen: AtomicU64,
    anchor_at_bits: AtomicU64,
    anchor_pos_bits: AtomicU64,
    /// The mapping this device reports (`at − pos`), the last one seen (f64
    /// bits, `0.0` while none), and how many times it STEPPED. Chromium's page
    /// steps its audio mapping by a render burst a few seconds in; a sound card
    /// does the same when its reported delay settles, and every reading a fit
    /// holds is then wrong by that step — so the fit has to start over, which is
    /// what this counter tells it to do (`lib/audio/latency.js`'s
    /// `outputClockGeneration`).
    map_bits: AtomicU64,
    map_generation: AtomicU64,
}

impl Clock {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate: sample_rate.max(1),
            base_bits: AtomicU64::new(0.0f64.to_bits()),
            frames: AtomicU64::new(0),
            latency_frames: AtomicU64::new(0),
            stream_frames: AtomicU64::new(0),
            anchor_gen: AtomicU64::new(0),
            anchor_at_bits: AtomicU64::new(0.0f64.to_bits()),
            anchor_pos_bits: AtomicU64::new(0.0f64.to_bits()),
            map_bits: AtomicU64::new(0.0f64.to_bits()),
            map_generation: AtomicU64::new(0),
        }
    }

    /// The output callback consumed `frames` more frames: the stream timeline
    /// moves even while the player is paused or a party owns the sound.
    pub fn advance_stream(&self, frames: u64) -> u64 {
        self.stream_frames.fetch_add(frames, Ordering::AcqRel) + frames
    }

    /// Seconds of the output stream the device has been handed (no latency
    /// correction: frame N of the stream is heard at `anchor_at + N/rate`).
    pub fn stream_position(&self) -> f64 {
        self.stream_frames.load(Ordering::Acquire) as f64 / f64::from(self.sample_rate)
    }

    /// The device says the frame at stream position `pos_s` will be heard at
    /// `at_s` — cpal's own `playback` timestamp, in seconds of the clock that
    /// backend uses. Written as one pair, so a reader never mixes two callbacks.
    pub fn note_output(&self, at_s: f64, pos_s: f64) {
        if !at_s.is_finite() || !pos_s.is_finite() {
            return;
        }
        // A move of more than a render burst is the device's mapping stepping,
        // not the noise of one reading: the line a fit holds is then off by it.
        let mapping = at_s - pos_s;
        let previous = f64::from_bits(self.map_bits.load(Ordering::Acquire));
        if previous != 0.0 && (mapping - previous).abs() > 0.004 {
            self.map_generation.fetch_add(1, Ordering::AcqRel);
        }
        self.map_bits.store(mapping.to_bits(), Ordering::Release);
        let generation = self.anchor_gen.load(Ordering::Relaxed);
        self.anchor_gen
            .store(generation.wrapping_add(1), Ordering::Release);
        self.anchor_at_bits.store(at_s.to_bits(), Ordering::Release);
        self.anchor_pos_bits
            .store(pos_s.to_bits(), Ordering::Release);
        self.anchor_gen
            .store(generation.wrapping_add(2), Ordering::Release);
    }

    /// How many times the device's mapping has stepped since the stream opened.
    pub fn output_generation(&self) -> u64 {
        self.map_generation.load(Ordering::Acquire)
    }

    /// `(at_s, pos_s, latency_s)`: the instant, on the device's clock, at which
    /// the frame at stream position `pos_s` is heard, and how much of what the
    /// mixer wrote the device still holds. With no callback behind it (an
    /// offline sink) the stream is its own clock: `at_s == pos_s`, no latency.
    pub fn output_anchor(&self) -> (f64, f64, f64) {
        let latency_s = self.latency_frames() as f64 / f64::from(self.sample_rate);
        for _ in 0..8 {
            let before = self.anchor_gen.load(Ordering::Acquire);
            if !before.is_multiple_of(2) {
                std::hint::spin_loop();
                continue;
            }
            let at_s = f64::from_bits(self.anchor_at_bits.load(Ordering::Acquire));
            let pos_s = f64::from_bits(self.anchor_pos_bits.load(Ordering::Acquire));
            if self.anchor_gen.load(Ordering::Acquire) == before && before > 0 {
                return (at_s, pos_s, latency_s);
            }
        }
        let position = self.stream_position();
        (position, position, latency_s)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// A voice starts being heard at `start_s` seconds into its track.
    pub fn start_at(&self, start_s: f64) {
        self.base_bits
            .store(start_s.max(0.0).to_bits(), Ordering::Release);
        self.frames.store(0, Ordering::Release);
    }

    /// The output callback consumed `frames` more frames of the current voice.
    pub fn advance(&self, frames: u64) {
        self.frames.fetch_add(frames, Ordering::Release);
    }

    /// The device's buffer is `frames` long: that much of what the mixer wrote
    /// has not been heard yet.
    pub fn set_latency_frames(&self, frames: u64) {
        self.latency_frames.store(frames, Ordering::Release);
    }

    pub fn latency_frames(&self) -> u64 {
        self.latency_frames.load(Ordering::Acquire)
    }

    /// Seconds of the current track that have left the mixer.
    pub fn position(&self) -> f64 {
        let base = f64::from_bits(self.base_bits.load(Ordering::Acquire));
        let frames = self.frames.load(Ordering::Acquire);
        let mixed = base + frames as f64 / f64::from(self.sample_rate);
        let latency = self.latency_frames() as f64 / f64::from(self.sample_rate);
        (mixed - latency).max(0.0)
    }

    /// Where the current voice started, without the latency correction.
    pub fn base(&self) -> f64 {
        f64::from_bits(self.base_bits.load(Ordering::Acquire))
    }
}

/// A `f32` in an `AtomicU32`, for the volume the callback reads.
#[derive(Debug)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub fn new(value: f32) -> Self {
        Self(AtomicU32::new(value.to_bits()))
    }

    pub fn load(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Acquire))
    }

    pub fn store(&self, value: f32) {
        self.0.store(value.to_bits(), Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_is_base_plus_mixed_minus_latency() {
        let clock = Clock::new(48_000);
        clock.start_at(6.0);
        clock.advance(4_800); // 100 ms
        assert!((clock.position() - 6.1).abs() < 1e-9);
        // The device still holds 480 frames (10 ms) that nobody has heard.
        clock.set_latency_frames(480);
        assert!((clock.position() - 6.09).abs() < 1e-9);
    }

    #[test]
    fn position_never_goes_negative() {
        let clock = Clock::new(48_000);
        clock.set_latency_frames(480);
        assert_eq!(clock.position(), 0.0);
    }

    #[test]
    fn starting_a_voice_resets_the_frame_count() {
        let clock = Clock::new(48_000);
        clock.start_at(0.0);
        clock.advance(48_000);
        clock.start_at(12.5);
        assert_eq!(clock.position(), 12.5);
        assert_eq!(clock.base(), 12.5);
    }

    #[test]
    fn the_stream_timeline_outlives_every_voice() {
        let clock = Clock::new(48_000);
        clock.start_at(0.0);
        clock.advance(48_000);
        clock.advance_stream(48_000);
        // A new track resets the media position, never the output timeline: a
        // party guest schedules on the stream, not on whatever track plays.
        clock.start_at(200.0);
        assert_eq!(clock.position(), 200.0);
        assert!((clock.stream_position() - 1.0).abs() < 1e-9);
        clock.advance_stream(24_000);
        assert!((clock.stream_position() - 1.5).abs() < 1e-9);
    }

    #[test]
    fn the_output_anchor_pairs_an_instant_with_the_frame_heard_then() {
        let clock = Clock::new(48_000);
        clock.set_latency_frames(480);
        // The frame at 2.000 s is heard at device instant 91.000 s.
        clock.note_output(91.0, 2.0);
        let (at_s, pos_s, latency_s) = clock.output_anchor();
        assert!((at_s - 91.0).abs() < 1e-12);
        assert!((pos_s - 2.0).abs() < 1e-12);
        assert!((latency_s - 0.01).abs() < 1e-12);
    }

    #[test]
    fn a_mapping_that_steps_is_counted_and_the_noise_is_not() {
        let clock = Clock::new(48_000);
        clock.note_output(91.000, 2.000);
        assert_eq!(clock.output_generation(), 0);
        // A reading a burst's worth off (the device's delay settling).
        clock.note_output(91.020, 2.000);
        assert_eq!(clock.output_generation(), 1);
        // Two readings 0.2 ms apart are noise, not a step.
        clock.note_output(91.0202, 2.000);
        assert_eq!(clock.output_generation(), 1);
    }

    #[test]
    fn with_no_device_the_stream_is_its_own_clock() {
        let clock = Clock::new(48_000);
        clock.advance_stream(96_000);
        let (at_s, pos_s, _) = clock.output_anchor();
        assert!((at_s - 2.0).abs() < 1e-9);
        assert!((pos_s - 2.0).abs() < 1e-9);
    }
}
