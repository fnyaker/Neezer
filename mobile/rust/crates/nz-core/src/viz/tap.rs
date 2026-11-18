//! The audio tap, as the visualiser sees it: a SEAM, not an engine.
//!
//! The sound the bars and the scope draw is the play engine's own tap, taken
//! after the DSP and before the mono/balance fold (`nz-audio/src/tap.rs`, the
//! SPA's `analysisBus`). The phone makes that tap **multi-reader** in MB12: one
//! ring per subscriber, a slow reader losing only its own oldest frames, and
//! zero copies while nobody subscribes (`nz-core::player::Player::subscribe_tap`).
//!
//! This module exists so that MB21 can be written, tested and measured against
//! the SPA's behaviour BEFORE that API is merged, and so that the visualiser
//! never borrows the tap exclusively: it opens ONE subscription when a view is
//! visible, drops it when the last view leaves, and the `Engine::tap()` slot the
//! desktop uses stays free for whoever else wants a read.
//!
//! [`TapSource`] mirrors MB12's published contract one method at a time
//! (`read`, `frames_available`, `dropped`, `clear`) and is deliberately as small
//! as that contract, so the production adapter is a three-line `impl` and
//! nothing of this module reaches the audio thread.

use std::sync::Arc;

/// One reader on the analysis tap — the shape of MB12's
/// `nz_audio::TapSubscription`, so `impl TapSource for TapSubscription` is all
/// the wiring the merge needs.
///
/// Every method is the tap's own: `read` fills from the start of `out` and
/// answers the number of FRAMES written (stereo interleaved `f32`); a reader
/// that falls behind loses only its own oldest frames (`dropped` counts them);
/// `clear` throws the queue away after a pause or a seek, because the analyser
/// wants the newest audio and not a stale second of it.
pub trait TapSource: Send {
    /// Copy what is available into `out` (interleaved stereo) and return the
    /// number of frames written.
    fn read(&mut self, out: &mut [f32]) -> usize;
    /// Frames waiting to be read.
    fn frames_available(&self) -> usize;
    /// Frames the callback dropped because this reader was behind.
    fn dropped(&self) -> u64;
    /// Throw away what is queued.
    fn clear(&mut self);
}

/// How the visualiser opens its subscription. `None` = the engine has no tap
/// (nothing has ever played, or the sink is an offline one in a test).
///
/// A closure rather than a `Player` handle on purpose: `nz-core::viz` never
/// holds the player, so a screen that never opens a visualiser subscribes to
/// nothing and the core keeps no reference that could keep an engine alive.
pub type TapOpener = Arc<dyn Fn() -> Option<Box<dyn TapSource>> + Send + Sync>;

/// An opener for a process with no tap at all: every read answers nothing and
/// the analysis thread stays parked (the host tests of the screens use it).
pub fn no_tap() -> TapOpener {
    Arc::new(|| None)
}

/// **MB12's own subscription, as this seam sees it.** The four methods are the
/// whole of the published contract (`docs/agents/mobile.md`, *The tap contract*),
/// and Drop is the unsubscription: MB12's `TapSubscription` frees its slot on the
/// mixer's next buffer with no lock and no join, so the visualiser releasing one
/// costs the music nothing.
impl TapSource for nz_audio::TapSubscription {
    fn read(&mut self, out: &mut [f32]) -> usize {
        nz_audio::TapSubscription::read(self, out)
    }

    fn frames_available(&self) -> usize {
        nz_audio::TapSubscription::frames_available(self)
    }

    fn dropped(&self) -> u64 {
        nz_audio::TapSubscription::dropped(self)
    }

    fn clear(&mut self) {
        nz_audio::TapSubscription::clear(self)
    }
}

/// What one read of a [`SyntheticTap`] costs, in frames: 10 ms at 48 kHz.
pub const SYNTH_BLOCK: usize = 480;

/// A DETERMINISTIC audio source, for the tests and the benches.
///
/// It is the fixture `nz-viz` already holds its arithmetic to (`lib.rs#recording`):
/// a 220 Hz square wave — a spectrum with real harmonics, so the bands have a
/// shape a bar can be at — plus a noise floor on both channels, generated with
/// integers and `f32` arithmetic only. Two runs of the same number of reads are
/// the same samples, so a level, a bar height or a scope column can be asserted
/// as a NUMBER without recording anything.
///
/// `read` also counts what it handed over, which is how the acceptance's "0
/// analysis tick while no view is visible" is measured on an event (the count)
/// rather than on a clock.
pub struct SyntheticTap {
    /// The next sample index: the generator's clock, not wall time.
    index: u64,
    /// The LCG's state, as `nz-viz`'s fixture.
    noise: u32,
    /// How many reads found a caller (the event the hidden test waits on).
    reads: u64,
    /// How many frames were handed over.
    frames: u64,
    /// A gain on the tone: 0 gives a silent tap (a paused engine).
    gain: f32,
}

impl SyntheticTap {
    pub fn new() -> SyntheticTap {
        SyntheticTap {
            index: 0,
            noise: 0x1234_5678,
            reads: 0,
            frames: 0,
            gain: 1.0,
        }
    }

    /// The same source, at half the level (the "quiet passage" a bar must
    /// follow rather than renormalise).
    pub fn with_gain(gain: f32) -> SyntheticTap {
        SyntheticTap {
            gain,
            ..SyntheticTap::new()
        }
    }

    /// How many times a reader asked for frames — the hidden/parked event.
    pub fn reads(&self) -> u64 {
        self.reads
    }

    /// How many frames it produced.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    fn next_noise(&mut self) -> f32 {
        self.noise = self
            .noise
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        ((self.noise >> 8) as f32 / 8_388_608.0) - 1.0
    }
}

impl Default for SyntheticTap {
    fn default() -> Self {
        SyntheticTap::new()
    }
}

impl TapSource for SyntheticTap {
    fn read(&mut self, out: &mut [f32]) -> usize {
        self.reads += 1;
        let frames = out.len() / 2;
        for slot in 0..frames {
            let i = self.index;
            let phase = ((i * 220) / 48_000) % 2;
            let square = if phase == 0 { 0.6 } else { -0.6 } * self.gain;
            let left = square * 0.8 + self.next_noise() * 0.05;
            let right = square * 0.8 + self.next_noise() * 0.05;
            out[slot * 2] = left;
            out[slot * 2 + 1] = right;
            self.index += 1;
            self.frames += 1;
        }
        frames
    }

    fn frames_available(&self) -> usize {
        SYNTH_BLOCK
    }

    fn dropped(&self) -> u64 {
        0
    }

    fn clear(&mut self) {
        // A generated source has no queue to throw away: the next read is the
        // next instant either way.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_synthetic_source_is_the_same_samples_twice() {
        let mut a = SyntheticTap::new();
        let mut b = SyntheticTap::new();
        let mut ba = vec![0.0f32; SYNTH_BLOCK * 2];
        let mut bb = vec![0.0f32; SYNTH_BLOCK * 2];
        a.read(&mut ba);
        b.read(&mut bb);
        assert_eq!(ba, bb, "the generator carries no clock of its own");
        assert!(ba.iter().any(|v| v.abs() > 0.1), "and it is not silence");
    }

    #[test]
    fn a_read_reports_the_frames_it_filled() {
        let mut tap = SyntheticTap::new();
        let mut out = vec![0.0f32; 100 * 2];
        assert_eq!(tap.read(&mut out), 100);
        assert_eq!(tap.frames(), 100);
        assert_eq!(tap.reads(), 1);
        // An odd buffer still fills whole stereo frames only.
        let mut odd = vec![0.0f32; 7];
        assert_eq!(tap.read(&mut odd), 3);
    }

    #[test]
    fn a_silent_source_is_allowed_and_stays_silent() {
        let mut tap = SyntheticTap::with_gain(0.0);
        let mut out = vec![1.0f32; 8];
        tap.read(&mut out);
        // The noise floor remains (it is the fixture's own), but no tone: the
        // point is that a gain of 0 is not a special case anywhere.
        assert!(out.iter().all(|v| v.abs() < 0.06), "{out:?}");
    }
}
