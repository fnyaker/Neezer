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
}

impl Clock {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate: sample_rate.max(1),
            base_bits: AtomicU64::new(0.0f64.to_bits()),
            frames: AtomicU64::new(0),
            latency_frames: AtomicU64::new(0),
        }
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
}
