//! The oscilloscope: the per-channel tap, the window the trigger hunts
//! through, and the two traces in the shape a renderer wants.
//!
//! THE ARITHMETIC IS NOT HERE. The trigger, the reduction of each channel's
//! window to one MIN/MAX pair per column, and the auto-range are
//! `rhythm::viz_scope::Scope` — the same code the page runs
//! (`webapp/rhythm/src/viz_scope.rs`). This module owns only what the page's
//! `ScopeCore` and its scene own on the browser side: the raw samples the
//! AnalyserNode would have kept, and the copy of the trace out of the core.
//!
//! WHY A COPY OF THE WINDOW. The page's AnalyserNode owns the last `fftSize`
//! samples and `getFloatTimeDomainData` reads them at the instant of the frame.
//! A native engine has no AnalyserNode, so this keeps the ring itself
//! ([`Wave`]) and takes a snapshot when an analysis frame is produced; the
//! snapshot is what the trigger reads, once, at the cadence the analyser runs
//! at (94 Hz at 48 kHz), never at the display rate. See `nz_viz::VizState`
//! for why that cadence is the web's and why the paint rate cannot change the
//! picture.

use std::slice;

pub use rhythm::viz_scope::{MAX_COLS, ScopeCfg};

/// The tap's window, in samples: `graph.js#requestScope`'s default and the
/// `buffer` of the web's low and medium tiers (an AnalyserNode's `fftSize` is
/// a power of two in [1024, 32768]). At 48 kHz that is 85 ms — the 42 ms
/// timebase plus the 40 ms the trigger hunts back through, with room to spare.
///
/// It is a CONSTANT and not the device rate times a duration, on purpose: the
/// browser's window is in samples too, so a 96 kHz device gets the same number
/// of samples on both sides and the same (degraded, still correct) search
/// range. `Scope::update` clamps the timebase to the window it is given.
pub const WINDOW: usize = 4096;

/// The most recent [`WINDOW`] samples of each channel, oldest first.
///
/// One writer (the analysis thread's tap) and one reader (the snapshot taken
/// when an analysis frame lands), so it needs no lock: `nz-viz` is pure and
/// single-threaded, and the app owns whatever synchronisation its threads need.
pub struct Wave {
    left: Vec<f32>,
    right: Vec<f32>,
    /// The next slot to write: the OLDEST sample once the ring is full.
    pos: usize,
}

impl Wave {
    pub fn new() -> Wave {
        Wave {
            left: vec![0.0; WINDOW],
            right: vec![0.0; WINDOW],
            pos: 0,
        }
    }

    /// Push interleaved stereo frames (`[l, r, l, r, …]`); a trailing odd
    /// sample is ignored.
    pub fn push_interleaved(&mut self, frames: &[f32]) {
        let (pairs, _) = frames.as_chunks::<2>();
        for pair in pairs {
            self.left[self.pos] = pair[0];
            self.right[self.pos] = pair[1];
            self.pos += 1;
            if self.pos == WINDOW {
                self.pos = 0;
            }
        }
    }

    /// The window, oldest first, into `out` (left then right, `2 * WINDOW`
    /// floats): what an AnalyserNode's `getFloatTimeDomainData` answered at the
    /// instant of the frame. Slots never written are zeros, exactly like a
    /// fresh analyser's buffer.
    pub fn copy_into(&self, out: &mut [f32]) {
        let (left, right) = out.split_at_mut(WINDOW);
        let (l0, l1) = self.left.split_at(self.pos);
        let (r0, r1) = self.right.split_at(self.pos);
        left[..WINDOW - self.pos].copy_from_slice(l1);
        left[WINDOW - self.pos..].copy_from_slice(l0);
        right[..WINDOW - self.pos].copy_from_slice(r1);
        right[WINDOW - self.pos..].copy_from_slice(r0);
    }
}

impl Default for Wave {
    fn default() -> Self {
        Self::new()
    }
}

/// What the scene reads to draw the instrument's face: the auto-range's gain,
/// how solidly the trigger is locked (0..1), and whether there is a signal at
/// all (`ScopeCore#state`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScopeState {
    pub gain: f32,
    pub locked: f32,
    pub signal: bool,
}

/// The scope's state: the core, the window it was last given, and the two
/// traces copied out for the renderer.
pub struct ScopeViz {
    scope: rhythm::viz_scope::Scope,
    sr: f64,
    cols: usize,
    /// Channel A then channel B, each `[lo, hi]` per column.
    a: Vec<f32>,
    b: Vec<f32>,
    /// Time since the last analysis frame, accumulated across draws — this is
    /// what keeps the auto-range and the lock envelopes following SECONDS and
    /// not the paint rate (see `nz_viz::VizState`).
    dt: f64,
    /// A new window (or a new column count) is waiting for the next draw.
    dirty: bool,
    idle: bool,
}

impl ScopeViz {
    pub fn new(sr: f32) -> ScopeViz {
        let sr = if sr > 8000.0 { sr as f64 } else { 48000.0 };
        ScopeViz {
            scope: {
                let mut scope = rhythm::viz_scope::Scope::new();
                // THE PRECISION OF A DESKTOP, not of a phone. The page's tiers
                // grade the scope because a phone cannot afford the min/max of
                // every column (`quality.js#SCOPE`: `exact`/`fine` only at high
                // and ultra, one value per column below). A trace on a 1548 px
                // lane reduced to 512 single values is visibly polygonal, so the
                // desktop always takes the exact pair and the sub-sample
                // trigger, at the medium tier's 4096-sample buffer (the 42 ms
                // timebase needs 2016, and a 40 ms hunt fits in what is left).
                scope.cfg = ScopeCfg {
                    search_ms: 40.0,
                    exact: true,
                    fine: true,
                    interp: false,
                };
                scope
            },
            sr,
            cols: 0,
            a: vec![0.0; MAX_COLS * 2],
            b: vec![0.0; MAX_COLS * 2],
            dt: 0.0,
            dirty: false,
            idle: false,
        }
    }

    /// The precision knobs the web's quality tiers set
    /// (`quality.js#SCOPE`: `search`, `exact`, `fine`, `interp`). The default
    /// is the medium tier's, which is the closest thing the page has to "the
    /// canvas fallback": 40 ms of search, one minimum and one maximum per
    /// column, no sub-sample trigger.
    pub fn set_config(&mut self, cfg: ScopeCfg) {
        self.scope.cfg = cfg;
        self.dirty = true;
    }

    /// A new window to draw at the next [`ScopeViz::trace`]. It is written
    /// straight into the core's own input buffer: one copy, and the draw then
    /// reads a window that cannot have moved.
    pub fn frame(&mut self, wave: &Wave) {
        let p = self.scope.wave_ptr(WINDOW);
        // SAFETY: `wave_ptr(WINDOW)` points at the core's own buffer of
        // `WINDOW * 2` floats (left then right), grown to exactly that size by
        // the call above; nothing else holds a view on it between here and the
        // next `wave_ptr` call.
        let buf = unsafe { slice::from_raw_parts_mut(p, WINDOW * 2) };
        wave.copy_into(buf);
        self.dirty = true;
        self.idle = false;
    }

    /// No probe on the signal (paused, nothing playing): a flat line, which is
    /// the truth, and the lock let go — `ScopeCore#idle`.
    pub fn go_idle(&mut self) {
        self.idle = true;
        self.dirty = true;
    }

    /// The two traces, `[lo, hi]` per column, for this draw. `dt` is the time
    /// since the previous call; it is only spent when a new window arrives.
    pub fn trace(&mut self, cols: usize, dt: f64) -> (&[f32], &[f32]) {
        let cols = cols.min(MAX_COLS);
        if cols != self.cols {
            self.cols = cols;
            self.scope.set_cols(cols);
            self.dirty = true;
        }
        self.dt += dt;
        if self.dirty {
            let dt = core::mem::take(&mut self.dt);
            if self.idle {
                self.scope.idle(dt);
            } else {
                self.scope.update(WINDOW, self.sr, dt);
            }
            self.dirty = false;
            self.copy_trace();
        }
        (&self.a[..cols * 2], &self.b[..cols * 2])
    }

    /// The trigger lock and the auto-range, for the marker and the graticule.
    pub fn state(&mut self) -> ScopeState {
        let p = self.scope.state_ptr();
        // SAFETY: `state_ptr` points at the core's fixed `STATE_LEN` record and
        // is refreshed by the call itself; `gain`, `locked` and `signal` are
        // fields 0, 1 and 4 of it (`viz_scope.rs#state_ptr`).
        let s = unsafe { slice::from_raw_parts(p, rhythm::viz_scope::STATE_LEN) };
        ScopeState {
            gain: s[0],
            locked: s[1],
            signal: s[4] > 0.0,
        }
    }

    fn copy_trace(&mut self) {
        let cols = self.cols;
        if cols == 0 {
            return;
        }
        let all = unsafe {
            // SAFETY: the core's trace is `TRACE_LEN` floats for good (it is a
            // fixed-size buffer, `viz_scope.rs#new`), channel A's `[lo, hi]`
            // pairs first, then channel B's.
            slice::from_raw_parts(self.scope.trace_ptr(), rhythm::viz_scope::TRACE_LEN)
        };
        let lane = MAX_COLS * 2;
        self.a[..cols * 2].copy_from_slice(&all[..cols * 2]);
        self.b[..cols * 2].copy_from_slice(&all[lane..lane + cols * 2]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_is_the_newest_samples_oldest_first() {
        let mut w = Wave::new();
        // More than one window's worth, so the ring wraps at least once.
        let n = WINDOW + 100;
        let mut stereo = Vec::with_capacity(n * 2);
        for i in 0..n {
            stereo.push(i as f32);
            stereo.push(-(i as f32));
        }
        w.push_interleaved(&stereo);
        let mut out = vec![0.0; WINDOW * 2];
        w.copy_into(&mut out);
        let (l, r) = out.split_at(WINDOW);
        // The newest `WINDOW` samples, in time order: from n - WINDOW to n - 1.
        assert_eq!(l[0], (n - WINDOW) as f32);
        assert_eq!(l[WINDOW - 1], (n - 1) as f32);
        assert_eq!(r[0], -((n - WINDOW) as f32));
        assert_eq!(r[WINDOW - 1], -((n - 1) as f32));
    }

    #[test]
    fn a_fresh_tap_is_silent_not_uninitialised() {
        let w = Wave::new();
        let mut out = vec![7.0; WINDOW * 2];
        w.copy_into(&mut out);
        assert!(out.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn a_trace_has_one_lo_hi_pair_per_column_per_channel() {
        let mut v = ScopeViz::new(48000.0);
        let mut w = Wave::new();
        let tone: Vec<f32> = (0..WINDOW)
            .map(|i| (i as f32 * 0.05).sin() * 0.5)
            .flat_map(|s| [s, -s])
            .collect();
        w.push_interleaved(&tone);
        for _ in 0..8 {
            v.frame(&w);
            let (a, b) = v.trace(512, 512.0 / 48000.0);
            assert_eq!(a.len(), 512 * 2);
            assert_eq!(b.len(), 512 * 2);
            assert!(a.iter().all(|x| x.is_finite()));
            // The two channels are the same tone with opposite signs: the trace
            // mirrors, which is the whole reason the tap is per channel.
            let (pa, _) = a.as_chunks::<2>();
            let (pb, _) = b.as_chunks::<2>();
            assert!(
                pa.iter()
                    .zip(pb.iter())
                    .all(|(x, y)| (x[0] + y[1]).abs() < 0.3)
            );
        }
        let st = v.state();
        assert!(st.gain > 0.0);
    }

    #[test]
    fn idle_draws_a_flat_line_and_lets_the_lock_go() {
        let mut v = ScopeViz::new(48000.0);
        assert!(!v.state().signal);
        v.go_idle();
        let (a, b) = v.trace(64, 0.01);
        assert!(a.iter().all(|&x| x == 0.0), "a flat line");
        assert!(b.iter().all(|&x| x == 0.0));
        assert_eq!(v.state().locked, 0.0);
    }
}
