//! One analysed frame, in the shape a Compose `Canvas` draws — and the one
//! place that turns the analyser's output into it.
//!
//! The arithmetic is NOT here. `nz-viz` feeds `webapp/rhythm`'s own `viz_bars`
//! and `viz_scope` the inputs the page feeds them, in the same order, so a bar
//! does not sit at a different height depending on where the sound came from
//! (`desktop/crates/nz-viz`, measured against the shipped `rhythm.wasm` to 1e-3).
//! What this module owns is what the browser's scene owns: which number goes
//! where, and how many of them.
//!
//! # The cadence, and why it is the analyser's
//!
//! A frame is built once per ANALYSIS frame (~94 Hz at 48 kHz), never per
//! paint: the levelling's envelopes and the scope's auto-range follow seconds
//! (`nz-viz`'s own rule), so they are spent with the time since the previous
//! frame, and the surface may paint the newest frame at 30, 60 or 120 Hz
//! without moving the picture by one pixel. That is also what keeps the whole
//! per-paint cost on the phone to one FFI call and one draw.
//!
//! # What a frame carries, and what it does not
//!
//! - `bars` and `peaks`: the SPA's `rb.smooth` and `rb.peaks` — a level per
//!   capsule and the afterglow's height, 0..1.
//! - `scope_a`/`scope_b`: one `[lo, hi]` pair per column per channel, the
//!   `ScopeCore#trace` a real oscilloscope draws (channel A is the left).
//! - the palette: the base hue/saturation/lightness/spread AND the seven stops
//!   of the sleeve's walk, in HSL, which is the colour language the SPA's scene
//!   paints with (`palette.js`). RGB is computed on the surface, where the
//!   pixels are.
//! - the scope's face: the auto-range's gain and how solidly the trigger is
//!   locked (`ScopeState`), which is what tells "still because it is triggered"
//!   from "sliding because nothing triggers".
//! - `level`: the frame's own loudness, 0..1.
//!
//! `beat`/`bpm` are carried and always `false`/`0` here: they are the RHYTHM
//! ladder's, which MB22's worlds need and the bars and the scope do not
//! (`modes.js#levelFor` gives both the spectrum depth). The record keeps them
//! so that a later lot fills them instead of changing the contract.

use nz_viz::VizState;

use super::palette::{Colours, Music, PaletteSource, PaletteState, ZONE_COUNT, Zone};

/// How many capsules the bars may hold at once: the SPA's `preset.bars`
/// ceiling at its richest tier is 96 (`quality.js#PRESETS`), and the phone asks
/// for what its width supports (`bars.js#resize`: one bead per 16 CSS px in the
/// strip, one per 20.5 full screen).
pub const MAX_BARS: usize = 96;
/// The fewest bars worth drawing: under this a capsule is a hairline.
pub const MIN_BARS: usize = 8;
/// The most columns the scope is reduced to. The SPA clamps to the tier's
/// `points` (512 at the medium tier, which is what `nz-viz` configures), and no
/// phone lane is wider in dp.
pub const MAX_COLS: usize = 512;
/// The fewest columns a trace can be read at.
pub const MIN_COLS: usize = 16;

/// What the surface is shaped like: how many capsules, how many columns. Sent
/// by the view on a size or mode change — never per frame, because changing
/// either resets an envelope (`rhythm::viz_bars::Bars::set_count`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shape {
    pub bars: usize,
    pub cols: usize,
}

impl Default for Shape {
    fn default() -> Self {
        Shape {
            bars: 32,
            cols: 256,
        }
    }
}

impl Shape {
    /// Clamped to what the drawing and the analyser can actually do.
    pub fn settled(self) -> Shape {
        Shape {
            bars: self.bars.clamp(MIN_BARS, MAX_BARS),
            cols: self.cols.clamp(MIN_COLS, MAX_COLS),
        }
    }
}

/// One analysed frame, ready to draw.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Frame {
    /// A level per capsule, 0..1, left (bass) to right (treble).
    pub bars: Vec<f32>,
    /// The slow-falling peak per capsule, same shape — the afterglow.
    pub peaks: Vec<f32>,
    /// Channel A (left): one `[lo, hi]` pair per column.
    pub scope_a: Vec<f32>,
    /// Channel B (right), the same.
    pub scope_b: Vec<f32>,
    /// The palette's base colour.
    pub colours: Colours,
    /// The sleeve's walk, seven HSL stops (`palette.js#ZONE_COUNT`).
    pub zones: [Zone; ZONE_COUNT],
    /// Whether the walk is live (the source is the cover AND there is artwork).
    pub has_zones: bool,
    /// The palette source the frame was coloured with — the name MB0 pinned in
    /// the record, carried here so no paint reads the settings again.
    pub source: PaletteSource,
    /// The frame's loudness, 0..1.
    pub level: f32,
    /// The scope's auto-range, for the graticule.
    pub gain: f32,
    /// How solidly the trigger is locked, 0..1, for the trigger marker.
    pub locked: f32,
    /// Whether there is a signal on the probe at all.
    pub signal: bool,
    /// A beat landed on this frame — the rhythm ladder's (MB22).
    pub beat: bool,
    /// The tempo estimate, 0 when unknown — MB22's too.
    pub bpm: f32,
}

impl Frame {
    /// The counted bytes this frame holds (worker rule 4): the five float
    /// vectors and the struct itself, never an RSS.
    pub fn bytes(&self) -> u64 {
        let floats = (self.bars.capacity()
            + self.peaks.capacity()
            + self.scope_a.capacity()
            + self.scope_b.capacity()) as u64;
        core::mem::size_of::<Frame>() as u64 + floats * core::mem::size_of::<f32>() as u64
    }
}

/// Builds this analysis frame's picture.
///
/// `dt` is the time since the previous ANALYSIS frame (the caller accumulates
/// it), which is what keeps every envelope following seconds and not the paint
/// rate. `needs_wave` comes from the mode (`modes.js#needsWave`): in bars mode
/// the scope is not computed at all, because the page does not compute it
/// either.
pub fn sample(
    state: &mut VizState,
    shape: Shape,
    palette: &mut PaletteState,
    dt: f64,
    needs_wave: bool,
) -> Frame {
    let shape = shape.settled();
    // The bars first: `levels` hands back levels followed by peaks, contiguous
    // (`nz-viz/src/bars.rs`), and the borrow ends with the copy.
    let (bars, peaks) = {
        let all = state.bars(shape.bars, dt);
        let split = (all.len() / 2).min(shape.bars);
        (all[..split].to_vec(), all[split..].to_vec())
    };
    let level = if bars.is_empty() {
        0.0
    } else {
        bars.iter().sum::<f32>() / bars.len() as f32
    };
    // The register balance the palette leans on: the bass third against the
    // treble third of the strip. The SPA reads the six energy bands; the bars
    // are the same spectrum, levelled per zone (see `palette.rs`).
    let third = (bars.len() / 3).max(1);
    let low_share = share(&bars[..third]);
    let high_share = share(&bars[bars.len().saturating_sub(third)..]);
    palette.update(Music {
        level: level as f64,
        low_share,
        high_share,
        dt,
    });

    let (scope_a, scope_b, gain, locked, signal) = if needs_wave {
        // The trace first (it borrows the state), then its face: two calls
        // because `scope` hands back a slice and `scope_state` needs the state
        // mutably — the same order the desktop's `VizState` users take.
        let (a, b) = {
            let (a, b) = state.scope(shape.cols, dt);
            (a.to_vec(), b.to_vec())
        };
        let face = state.scope_state();
        (a, b, face.gain, face.locked, face.signal)
    } else {
        (Vec::new(), Vec::new(), 0.0, 0.0, false)
    };

    Frame {
        bars,
        peaks,
        scope_a,
        scope_b,
        colours: palette.colours(),
        zones: *palette.zones(),
        has_zones: palette.has_zones(),
        source: palette.source(),
        level,
        gain,
        locked,
        signal,
        beat: false,
        bpm: 0.0,
    }
}

/// The mean of a slice, 0 for an empty one.
fn share(slice: &[f32]) -> f64 {
    if slice.is_empty() {
        0.0
    } else {
        slice.iter().map(|&v| v as f64).sum::<f64>() / slice.len() as f64
    }
}

/// A frame that draws nothing yet — what `frame()` answers before the first
/// analysis frame lands, and what a surface with no view sees: the bars at
/// rest, the scope flat. Never `None`, so a screen has no empty state to draw.
pub fn rest(shape: Shape) -> Frame {
    let shape = shape.settled();
    Frame {
        bars: vec![0.0; shape.bars],
        peaks: vec![0.0; shape.bars],
        colours: PaletteState::new(super::palette::PaletteSource::Cover).colours(),
        ..Frame::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viz::palette::PaletteSource;
    use crate::viz::tap::TapSource;

    fn state() -> VizState {
        VizState::new(48_000.0)
    }

    fn feed(state: &mut VizState, reads: usize) -> Vec<f32> {
        // The `nz-viz` fixture's generator, through the synthetic tap's own
        // shape: 480 stereo frames a read, so one read is ~10 ms.
        let mut tap = crate::viz::tap::SyntheticTap::new();
        let mut block = vec![0.0f32; crate::viz::tap::SYNTH_BLOCK * 2];
        let mut last = Vec::new();
        for _ in 0..reads {
            let got = tap.read(&mut block);
            state.push_frames(&block[..got * 2]);
            last = block[..got * 2].to_vec();
        }
        last
    }

    #[test]
    fn a_frame_has_one_level_and_one_peak_per_capsule() {
        let mut s = state();
        let mut p = PaletteState::new(PaletteSource::Cover);
        feed(&mut s, 40);
        let frame = sample(
            &mut s,
            Shape { bars: 24, cols: 64 },
            &mut p,
            480.0 / 48_000.0,
            false,
        );
        assert_eq!(frame.bars.len(), 24);
        assert_eq!(frame.peaks.len(), 24);
        assert!(
            frame.bars.iter().any(|&v| v > 0.02),
            "the tone lights some capsules: {:?}",
            &frame.bars[..4]
        );
        assert!(frame.bars.iter().all(|&v| (0.0..=1.0).contains(&v)));
        assert!(frame.peaks.iter().all(|&v| (0.0..=1.0).contains(&v)));
    }

    #[test]
    fn bars_mode_computes_no_scope_and_scope_mode_does() {
        let mut s = state();
        let mut p = PaletteState::new(PaletteSource::Neon);
        feed(&mut s, 40);
        let bars = sample(
            &mut s,
            Shape {
                bars: 16,
                cols: 128,
            },
            &mut p,
            480.0 / 48_000.0,
            false,
        );
        assert!(
            bars.scope_a.is_empty(),
            "modes.js#needsWave is false for bars"
        );
        assert!(bars.scope_b.is_empty());
        assert!(!bars.signal);

        let scope = sample(
            &mut s,
            Shape {
                bars: 16,
                cols: 128,
            },
            &mut p,
            480.0 / 48_000.0,
            true,
        );
        assert_eq!(scope.scope_a.len(), 128 * 2, "one lo/hi pair per column");
        assert_eq!(scope.scope_b.len(), 128 * 2);
        assert!(scope.scope_a.iter().all(|v| v.is_finite()));
        assert!(scope.gain > 0.0, "the auto-range settled");
        assert!(scope.signal, "the probe sees the tone");
    }

    #[test]
    fn the_two_channels_are_the_same_tone_and_not_one_buffer_twice() {
        let mut s = state();
        let mut p = PaletteState::new(PaletteSource::Cover);
        feed(&mut s, 40);
        let frame = sample(
            &mut s,
            Shape { bars: 16, cols: 64 },
            &mut p,
            480.0 / 48_000.0,
            true,
        );
        assert_ne!(
            frame.scope_a, frame.scope_b,
            "the synthetic tap's two channels carry different noise"
        );
    }

    #[test]
    fn the_shape_is_clamped_to_what_can_be_drawn() {
        let mut s = state();
        let mut p = PaletteState::new(PaletteSource::Cover);
        feed(&mut s, 8);
        let too_many = sample(
            &mut s,
            Shape {
                bars: 10_000,
                cols: 10_000,
            },
            &mut p,
            0.01,
            true,
        );
        assert_eq!(too_many.bars.len(), MAX_BARS);
        assert_eq!(too_many.scope_a.len(), MAX_COLS * 2);
        let too_few = sample(&mut s, Shape { bars: 0, cols: 0 }, &mut p, 0.01, false);
        assert_eq!(too_few.bars.len(), MIN_BARS);
    }

    #[test]
    fn a_frame_counts_its_own_bytes() {
        let mut s = state();
        let mut p = PaletteState::new(PaletteSource::Cover);
        feed(&mut s, 8);
        let frame = sample(
            &mut s,
            Shape {
                bars: 32,
                cols: 256,
            },
            &mut p,
            0.01,
            true,
        );
        // (32 + 32 + 512 + 512) floats = 1 088 × 4 B = 4 352 B, plus the struct.
        let floats = (32 + 32 + 256 * 2 + 256 * 2) as u64 * 4;
        assert!(
            frame.bytes() >= floats,
            "{} bytes for {floats} of floats",
            frame.bytes()
        );
        assert!(frame.bytes() < 16 * 1024);
    }

    #[test]
    fn the_rest_frame_is_flat_and_not_empty() {
        let frame = rest(Shape { bars: 12, cols: 32 });
        assert_eq!(frame.bars.len(), 12);
        assert!(frame.bars.iter().all(|&v| v == 0.0));
        assert!(frame.scope_a.is_empty());
        assert!(!frame.beat);
        assert_eq!(frame.bpm, 0.0);
    }
}
