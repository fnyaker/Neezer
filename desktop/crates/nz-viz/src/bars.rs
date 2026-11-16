//! The spectrum bars: the levelling `rhythm::viz_bars::Bars` computes, handed
//! out in the shape a renderer wants, at whatever paint rate the display runs.
//!
//! THE ARITHMETIC IS NOT HERE. The three-zone auto-gain, the gamma, the
//! fast-attack/slow-release envelope and the falling peaks are the page's own
//! Rust (`webapp/rhythm/src/viz_bars.rs`, through `lib/viz/core.js#BarsCore`),
//! which is why the desktop cannot drift from what the browser draws. What is
//! here is the frame bridge: the bands of one analysis frame go in, and the
//! levels followed by the peaks come out contiguous — `[level × n, peak × n]` —
//! where the core keeps them `MAX_BARS` apart so a bar count change never
//! moves a view.
//!
//! WHY THE TIME IS ACCUMULATED. The page updates the levelling once per
//! analysis frame (~94 Hz at 48 kHz) and passes that frame's own interval; a
//! desktop paints at the display rate (240 Hz here). Calling `update` on every
//! paint would spend a fraction of the interval 240 times — the envelopes
//! would follow the paint rate instead of seconds. Feeding it once per
//! analysis frame with the time since the previous frame is exactly what the
//! page does, and it is what makes the picture independent of the frame rate
//! (asserted in `lib.rs`).

use std::slice;

pub use rhythm::viz_bars::MAX_BARS;

/// The most bands the analyser can hand over (its layout says 120 today, but
/// the core takes up to this and the layout is the authority for the count).
const MAX_BANDS: usize = rhythm::viz_bars::MAX_BANDS;

/// The bars' state: the core, the newest frame's bands, and the compacted
/// levels-and-peaks the renderer reads.
pub struct BarsViz {
    bars: rhythm::viz_bars::Bars,
    /// How many bars are live; `MAX_BARS` at most, 0 allowed.
    n: usize,
    /// How many bands the newest frame carried.
    bands: usize,
    level: f64,
    /// Seconds since the previous analysis frame, accumulated across paints.
    dt: f64,
    /// A frame of bands is waiting for the next draw.
    dirty: bool,
    /// `[level × n, peak × n]`.
    out: Vec<f32>,
}

impl BarsViz {
    pub fn new() -> BarsViz {
        BarsViz {
            bars: rhythm::viz_bars::Bars::new(),
            n: 0,
            bands: 0,
            level: 0.0,
            dt: 0.0,
            dirty: false,
            out: vec![0.0; MAX_BARS * 2],
        }
    }

    /// One analysis frame: this frame's bands (the analyser's `bands` field)
    /// and the overall level it read (`level`).
    pub fn frame(&mut self, bands: &[f32], level: f32) {
        let n = bands.len().min(MAX_BANDS);
        let p = self.bars.input_ptr();
        // SAFETY: `input_ptr` points at the core's own `MAX_BANDS` buffer,
        // which never moves (it is allocated once in `Bars::new`), and `n` is
        // clamped to its length.
        let input = unsafe { slice::from_raw_parts_mut(p, MAX_BANDS) };
        input[..n].copy_from_slice(&bands[..n]);
        self.bands = n;
        self.level = level as f64;
        self.dirty = true;
    }

    /// The levels then the peaks, `n` of each, for this draw. `dt` is the time
    /// since the previous call; it is only spent when a new frame arrives.
    pub fn levels(&mut self, n: usize, dt: f64) -> &[f32] {
        let n = n.min(MAX_BARS);
        if n != self.n {
            self.bars.set_count(n);
            self.n = n;
        }
        self.dt += dt;
        if self.dirty {
            self.bars
                .update(self.bands, self.level, core::mem::take(&mut self.dt));
            self.dirty = false;
        }
        if n == 0 {
            return &self.out[..0];
        }
        let all = unsafe {
            // SAFETY: the core's output is `MAX_BARS` smooth values followed by
            // `MAX_BARS` peaks, in a buffer of `2 * MAX_BARS` floats allocated
            // once in `Bars::new` and never resized.
            slice::from_raw_parts(self.bars.output_ptr(), MAX_BARS * 2)
        };
        self.out[..n].copy_from_slice(&all[..n]);
        self.out[n..n * 2].copy_from_slice(&all[MAX_BARS..MAX_BARS + n]);
        &self.out[..n * 2]
    }

    /// The smoothed overall level the core keeps (`Bars::level`), for a scene
    /// that breathes with it. 0 until the first frame.
    pub fn level(&self) -> f32 {
        self.bars.level() as f32
    }

    /// How many bars are live.
    pub fn count(&self) -> usize {
        self.n
    }
}

impl Default for BarsViz {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bands(low: f32, high: f32) -> Vec<f32> {
        // 120 bands, bass at the bottom and treble at the top.
        (0..120)
            .map(|i| low + (high - low) * (i as f32 / 119.0))
            .collect()
    }

    #[test]
    fn no_frame_yet_is_a_flat_run_of_bars() {
        let mut v = BarsViz::new();
        let out = v.levels(64, 0.004);
        assert_eq!(out.len(), 128);
        assert!(out.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn levels_then_peaks_are_contiguous_for_the_live_bars() {
        let mut v = BarsViz::new();
        for _ in 0..8 {
            v.frame(&bands(0.1, 0.9), 0.5);
            let out = v.levels(64, 512.0 / 48000.0);
            assert_eq!(out.len(), 128, "64 levels then 64 peaks");
            // A sloping spectrum is a sloping row of bars, and the peaks are
            // above (or on) their bar: the bass end low, the treble end high.
            assert!(
                out[63] > out[0],
                "the row follows the bands: {} -> {}",
                out[0],
                out[63]
            );
            assert!(out[64..].iter().zip(&out[..64]).all(|(p, l)| p >= l));
        }
        assert_eq!(v.count(), 64);
        assert!(v.level() > 0.0);
    }

    #[test]
    fn the_bar_count_is_clamped_and_zero_is_allowed() {
        let mut v = BarsViz::new();
        v.frame(&bands(0.2, 0.6), 0.4);
        assert_eq!(v.levels(MAX_BARS + 40, 0.01).len(), MAX_BARS * 2);
        assert_eq!(v.levels(0, 0.01).len(), 0);
    }
}
