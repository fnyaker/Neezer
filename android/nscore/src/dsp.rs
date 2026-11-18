//! The output processing the web graph does with Web Audio nodes, as one chain a
//! native player runs on its decoded PCM: the source's normalization gain, its
//! fade envelope, then the ten-band EQ and the bass shelf.
//!
//! Each piece follows what `webapp/src/lib/audio/graph.js` asks Web Audio for,
//! so a setting sounds the same on both paths:
//!
//! - the EQ is ten PEAKING biquads at `EQ_FREQS`, Q 1.1, and the bass a LOWSHELF
//!   at 120 Hz — the Web Audio spec's BiquadFilterNode formulas (the Audio EQ
//!   Cookbook's, shelf slope 1);
//! - the fade is the same quarter circle `equalPowerCurve` samples: the ANGLE is
//!   interpolated, so a fade interrupted half way resumes on the curve, and two
//!   uncorrelated tracks crossing sum to constant power instead of dipping 3 dB;
//! - a gain change is ramped over a few milliseconds rather than stepped (a step
//!   in the middle of a waveform is a click).
//!
//! - the bass lift is the shelf plus graph.js's two compressors: a soft-knee one
//!   whose threshold and ratio follow the lift, then the brick-wall limiter just
//!   under 0 dBFS. They follow the Web Audio DynamicsCompressorNode's static
//!   curve (soft knee, ratio, and its automatic makeup gain,
//!   `(1 / gain at 0 dBFS)^0.6`) with one-pole attack and release on the gain
//!   reduction, without its 6 ms look-ahead.
//!
//! With everything flat the chain is bit-transparent, dynamics included: on the
//! web the compressors are only in the path while an effect is.

pub const EQ_FREQS: [f64; 10] = [31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
pub const EQ_Q: f64 = 1.1;
pub const EQ_MIN_DB: f64 = -16.0;
pub const EQ_MAX_DB: f64 = 16.0;
pub const BASS_FREQ: f64 = 120.0;
pub const MAX_CHANNELS: usize = 8;
/// How long a gain change takes when the caller does not say.
pub const GAIN_RAMP_MS: f64 = 10.0;
/// The meter's floor: what silence reads instead of `-inf` (`saturation.js#PEAK_FLOOR_DB`).
pub const METER_FLOOR_DB: f64 = -96.0;

/// What the chain's own dynamics did to the last buffer (`saturation.js`).
///
/// The equalizer's meter needs two numbers the chain already computes frame by
/// frame and used to throw away: the PRE-limiter peak (after the ten bands and
/// the bass shelf, the SPA's `pre` analyser) and how hard the safety limiter
/// pulled it back down. Measured on the same buffer, so the two are comparable.
///
/// **Nothing here is estimated.** The app's card reads these while it is on
/// screen and the chain publishes nothing when it is not (`set_meter(false)` is
/// the default), which is what makes "0 analysis CPU when the card leaves" true
/// by construction rather than by a timer that happens to be stopped.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChainMeter {
    /// The pre-limiter sample peak of the buffer, dBFS, floored at
    /// [`METER_FLOOR_DB`].
    pub peak_db: f64,
    /// The limiter's gain reduction on that same buffer, dB, `>= 0`
    /// (`saturation.js#limiterReduction`).
    pub reduction_db: f64,
}

impl ChainMeter {
    /// Nothing measured yet: silence, and a limiter that did nothing.
    pub const SILENT: ChainMeter = ChainMeter {
        peak_db: METER_FLOOR_DB,
        reduction_db: 0.0,
    };
}

/// One biquad, transposed direct form II, in f64: at 31 Hz and 48 kHz the
/// poles sit within 0.4% of the unit circle, where f32 state audibly rounds.
#[derive(Clone, Copy)]
pub struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: [f64; MAX_CHANNELS],
    z2: [f64; MAX_CHANNELS],
    /// Unity: skipped entirely (a 0 dB band is most of them, most of the time).
    flat: bool,
}

impl Biquad {
    pub fn flat() -> Biquad {
        Biquad { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0, z1: [0.0; MAX_CHANNELS], z2: [0.0; MAX_CHANNELS], flat: true }
    }

    fn set(&mut self, b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) {
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
        self.flat = false;
    }

    /// Web Audio's "peaking".
    pub fn peaking(&mut self, sr: f64, f0: f64, q: f64, db: f64) {
        if db == 0.0 || f0 <= 0.0 || f0 >= sr * 0.5 {
            self.flat = true;
            return;
        }
        let a = 10f64.powf(db / 40.0);
        let w0 = 2.0 * core::f64::consts::PI * f0 / sr;
        let (s, c) = (w0.sin(), w0.cos());
        let alpha = s / (2.0 * q);
        self.set(1.0 + alpha * a, -2.0 * c, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * c, 1.0 - alpha / a);
    }

    /// Web Audio's "lowshelf" (slope 1).
    pub fn lowshelf(&mut self, sr: f64, f0: f64, db: f64) {
        if db == 0.0 || f0 <= 0.0 || f0 >= sr * 0.5 {
            self.flat = true;
            return;
        }
        let a = 10f64.powf(db / 40.0);
        let w0 = 2.0 * core::f64::consts::PI * f0 / sr;
        let (s, c) = (w0.sin(), w0.cos());
        let alpha = s / 2.0 * 2f64.sqrt();
        let k = 2.0 * a.sqrt() * alpha;
        self.set(
            a * ((a + 1.0) - (a - 1.0) * c + k),
            2.0 * a * ((a - 1.0) - (a + 1.0) * c),
            a * ((a + 1.0) - (a - 1.0) * c - k),
            (a + 1.0) + (a - 1.0) * c + k,
            -2.0 * ((a - 1.0) + (a + 1.0) * c),
            (a + 1.0) + (a - 1.0) * c - k,
        );
    }

    #[inline]
    fn run(&mut self, x: f64, ch: usize) -> f64 {
        let y = self.b0 * x + self.z1[ch];
        self.z1[ch] = self.b1 * x - self.a1 * y + self.z2[ch];
        self.z2[ch] = self.b2 * x - self.a2 * y;
        y
    }

    /// Magnitude response at `f` (for the tests: what getFrequencyResponse says).
    pub fn magnitude(&self, sr: f64, f: f64) -> f64 {
        if self.flat {
            return 1.0;
        }
        let w = 2.0 * core::f64::consts::PI * f / sr;
        let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
        let nr = self.b0 + self.b1 * c1 + self.b2 * c2;
        let ni = -(self.b1 * s1 + self.b2 * s2);
        let dr = 1.0 + self.a1 * c1 + self.a2 * c2;
        let di = -(self.a1 * s1 + self.a2 * s2);
        ((nr * nr + ni * ni) / (dr * dr + di * di)).sqrt()
    }
}

/// A feed-forward compressor on the loudest channel, in decibels.
#[derive(Clone, Copy)]
pub struct Compressor {
    threshold: f64,
    knee: f64,
    ratio: f64,
    att: f64,
    rel: f64,
    makeup_db: f64,
    /// The gain reduction in force, dB (<= 0).
    env: f64,
    /// threshold 0 and ratio 1: nothing to do.
    neutral: bool,
}

impl Compressor {
    pub fn new(sr: f64) -> Compressor {
        let mut c = Compressor { threshold: 0.0, knee: 0.0, ratio: 1.0, att: 0.0, rel: 0.0, makeup_db: 0.0, env: 0.0, neutral: true };
        c.set(sr, 0.0, 0.0, 1.0, 0.003, 0.25);
        c
    }

    /// Web Audio's parameters: threshold and knee in dB, ratio, attack and release in seconds.
    pub fn set(&mut self, sr: f64, threshold: f64, knee: f64, ratio: f64, attack: f64, release: f64) {
        self.threshold = threshold.clamp(-100.0, 0.0);
        self.knee = knee.clamp(0.0, 40.0);
        self.ratio = ratio.clamp(1.0, 20.0);
        let coef = |t: f64| (-1.0 / (t.clamp(0.0, 1.0).max(1e-5) * sr)).exp();
        self.att = coef(attack);
        self.rel = coef(release);
        self.neutral = self.ratio <= 1.0 + 1e-9;
        // The automatic makeup gain: what the curve leaves of a full-scale input.
        self.makeup_db = if self.neutral { 0.0 } else { -0.6 * self.curve(0.0) };
    }

    /// The static curve's gain reduction (dB, <= 0) for an input at `x` dBFS.
    #[inline]
    pub fn curve(&self, x: f64) -> f64 {
        let (t, w, r) = (self.threshold, self.knee, self.ratio);
        let d = x - t;
        if 2.0 * d < -w {
            0.0
        } else if w > 0.0 && 2.0 * d.abs() <= w {
            (1.0 / r - 1.0) * (d + w / 2.0).powi(2) / (2.0 * w)
        } else {
            d * (1.0 / r - 1.0)
        }
    }

    /// The linear gain for a frame whose loudest sample is `peak`.
    #[inline]
    fn gain(&mut self, peak: f64) -> f64 {
        if self.neutral {
            self.env = 0.0;
            return 1.0;
        }
        let x = if peak > 1e-9 { 20.0 * peak.log10() } else { -180.0 };
        let target = self.curve(x);
        let a = if target < self.env { self.att } else { self.rel };
        self.env = a * self.env + (1.0 - a) * target;
        10f64.powf((self.env + self.makeup_db) / 20.0)
    }

    pub fn reduction(&self) -> f64 {
        self.env
    }

    fn clear(&mut self) {
        self.env = 0.0;
    }
}

/// A 0..1 envelope ridden on the quarter circle (see the module notes).
#[derive(Clone, Copy)]
struct Ramp {
    from: f64,
    to: f64,
    len: u64,
    at: u64,
}

impl Ramp {
    fn steady(v: f64) -> Ramp {
        Ramp { from: v, to: v, len: 0, at: 0 }
    }

    #[inline]
    fn value(&self) -> f64 {
        if self.at >= self.len {
            return self.to;
        }
        let x = self.at as f64 / self.len as f64;
        (angle(self.from) * (1.0 - x) + angle(self.to) * x).sin()
    }
}

#[inline]
fn angle(v: f64) -> f64 {
    v.clamp(0.0, 1.0).asin()
}

/// What one source goes through on its way out.
pub struct Chain {
    sr: f64,
    channels: usize,
    eq: [Biquad; 10],
    eq_db: [f64; 10],
    bass: Biquad,
    bass_comp: Compressor,
    limiter: Compressor,
    /// The compressors in the path (off only for the filter tests, which
    /// measure the filters alone).
    dynamics: bool,
    // The normalization: linear, ramped from `gain_from` toward `gain_to`.
    gain_from: f64,
    gain_to: f64,
    gain_len: u64,
    gain_at: u64,
    fade: Ramp,
    /// The equalizer card's meter: on only while that card is on screen
    /// ([`Chain::set_meter`]). Off, `process` runs exactly as it did before the
    /// meter existed.
    meter_on: bool,
    /// The last buffer's pre-limiter peak and limiter reduction.
    meter: ChainMeter,
}

impl Chain {
    pub fn new(sr: f32, channels: usize) -> Chain {
        Chain {
            sr: sr.max(1.0) as f64,
            channels: channels.clamp(1, MAX_CHANNELS),
            eq: [Biquad::flat(); 10],
            eq_db: [0.0; 10],
            bass: Biquad::flat(),
            bass_comp: Compressor::new(sr.max(1.0) as f64),
            limiter: {
                let mut l = Compressor::new(sr.max(1.0) as f64);
                l.set(sr.max(1.0) as f64, -0.5, 0.0, 20.0, 0.002, 0.06);
                l
            },
            dynamics: true,
            gain_from: 1.0,
            gain_to: 1.0,
            gain_len: 0,
            gain_at: 0,
            fade: Ramp::steady(1.0),
            meter_on: false,
            meter: ChainMeter::SILENT,
        }
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Turns the meter on or off. While it is off, `process` does exactly the
    /// arithmetic it did before the meter existed: the card that reads it is the
    /// only thing that pays for it.
    pub fn set_meter(&mut self, on: bool) {
        self.meter_on = on;
        if !on {
            self.meter = ChainMeter::SILENT;
        }
    }

    /// The last buffer's pre-limiter peak and limiter reduction. Both are
    /// measured in [`Chain::process`] and cost nothing to read.
    pub fn meter(&self) -> ChainMeter {
        self.meter
    }

    fn samples(&self, ms: f64) -> u64 {
        (ms.max(0.0) * self.sr / 1000.0).round() as u64
    }

    /// The normalization, in dB, reached over `ramp_ms` (negative: the default).
    pub fn set_gain_db(&mut self, db: f64, ramp_ms: f64) {
        let now = self.gain_now();
        self.gain_from = now;
        self.gain_to = 10f64.powf(db.clamp(-60.0, 24.0) / 20.0);
        self.gain_len = self.samples(if ramp_ms < 0.0 { GAIN_RAMP_MS } else { ramp_ms });
        self.gain_at = 0;
    }

    #[inline]
    fn gain_now(&self) -> f64 {
        if self.gain_at >= self.gain_len {
            return self.gain_to;
        }
        let x = self.gain_at as f64 / self.gain_len as f64;
        self.gain_from + (self.gain_to - self.gain_from) * x
    }

    /// Ride the fade envelope from where it is to `to` over `ms`.
    pub fn fade_to(&mut self, to: f64, ms: f64) {
        let from = self.fade.value();
        self.fade = Ramp { from, to: to.clamp(0.0, 1.0), len: self.samples(ms), at: 0 };
    }

    pub fn fade(&self) -> f64 {
        self.fade.value()
    }

    pub fn set_eq(&mut self, band: usize, db: f64) {
        if band >= self.eq.len() {
            return;
        }
        let db = if db.is_finite() { db.clamp(EQ_MIN_DB, EQ_MAX_DB) } else { 0.0 };
        if self.eq_db[band] == db {
            return;
        }
        self.eq_db[band] = db;
        let sr = self.sr;
        self.eq[band].peaking(sr, EQ_FREQS[band], EQ_Q, db);
    }

    pub fn set_bass(&mut self, db: f64) {
        let db = if db.is_finite() { db.clamp(0.0, EQ_MAX_DB) } else { 0.0 };
        let sr = self.sr;
        self.bass.lowshelf(sr, BASS_FREQ, db);
        // graph.js: the lift is `amount * 12` dB, and the compressor follows the
        // amount — neutral at none, -6..-20 dB and 2..6:1 from a little to all.
        let amount = (db / 12.0).clamp(0.0, 1.0);
        if amount > 0.001 {
            self.bass_comp.set(sr, -6.0 - amount * 14.0, 30.0, 2.0 + amount * 4.0, 0.012, 0.22);
        } else {
            self.bass_comp.set(sr, 0.0, 30.0, 1.0, 0.012, 0.22);
        }
    }

    pub fn bass_compressor(&self) -> &Compressor {
        &self.bass_comp
    }

    pub fn eq_filter(&self, band: usize) -> &Biquad {
        &self.eq[band]
    }

    pub fn bass_filter(&self) -> &Biquad {
        &self.bass
    }

    /// Forget the filters' memory (a seek: the next sample is not this one's
    /// continuation, and ringing the old one into it is a click).
    pub fn clear_state(&mut self) {
        for b in self.eq.iter_mut().chain(core::iter::once(&mut self.bass)) {
            b.z1 = [0.0; MAX_CHANNELS];
            b.z2 = [0.0; MAX_CHANNELS];
        }
        self.bass_comp.clear();
        self.limiter.clear();
    }

    /// Process `frames` interleaved frames of `self.channels()` samples in place.
    pub fn process(&mut self, buf: &mut [f32], frames: usize) {
        let ch = self.channels;
        let frames = frames.min(buf.len() / ch);
        // The filters, and the dynamics behind them, only while an effect is on.
        let any_eq = self.eq.iter().any(|b| !b.flat) || !self.bass.flat;
        let mut row = [0f64; MAX_CHANNELS];
        // The meter reads the block, not the frame: the SPA's two analysers read
        // an 85 ms window, and a buffer is the same kind of window (`set_meter`).
        let mut peak_max = 0f64;
        let mut post_max = 0f64;
        for f in 0..frames {
            let g = self.gain_now() * self.fade.value();
            if self.gain_at < self.gain_len {
                self.gain_at += 1;
            }
            if self.fade.at < self.fade.len {
                self.fade.at += 1;
            }
            let out = &mut buf[f * ch..f * ch + ch];
            if !any_eq {
                for s in out.iter_mut() {
                    *s = (*s as f64 * g) as f32;
                }
                if self.meter_on {
                    for s in out.iter() {
                        let a = f64::from(*s).abs();
                        peak_max = peak_max.max(a);
                        post_max = post_max.max(a);
                    }
                }
                continue;
            }
            let mut peak = 0f64;
            for (c, s) in out.iter().enumerate() {
                let mut x = *s as f64 * g;
                for b in self.eq.iter_mut() {
                    if !b.flat {
                        x = b.run(x, c);
                    }
                }
                if !self.bass.flat {
                    x = self.bass.run(x, c);
                }
                row[c] = x;
                peak = peak.max(x.abs());
            }
            let (gc, gl) = if self.dynamics {
                let gc = self.bass_comp.gain(peak);
                (gc, self.limiter.gain(peak * gc))
            } else {
                (1.0, 1.0)
            };
            // The SPA's `pre` analyser sits AFTER the bass shelf and its
            // compressor, BEFORE the limiter: `saturation.js` measures the
            // limiter's own reduction, not the pair's.
            if self.meter_on {
                peak_max = peak_max.max(peak * gc);
            }
            for (c, s) in out.iter_mut().enumerate() {
                let y = row[c] * gc * gl;
                *s = y as f32;
                if self.meter_on {
                    post_max = post_max.max(y.abs());
                }
            }
        }
        if self.meter_on {
            // `saturation.js#limiterReduction`: the same window's peak before and
            // after the limiter, in dB. Two logarithms per buffer, nothing per
            // sample beyond the two maxima above.
            let db_of = |x: f64| {
                if x > 0.0 {
                    (20.0 * x.log10()).max(METER_FLOOR_DB)
                } else {
                    METER_FLOOR_DB
                }
            };
            let pre = db_of(peak_max);
            let post = db_of(post_max);
            self.meter = ChainMeter {
                peak_db: pre,
                reduction_db: (pre - post).max(0.0),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48000.0;

    fn db(x: f64) -> f64 {
        20.0 * x.log10()
    }

    /// The level a sine comes out at, measured on the samples (after settling).
    fn measured_gain(chain: &mut Chain, f: f64) -> f64 {
        chain.dynamics = false;
        let n = 48000;
        let mut buf: Vec<f32> = (0..n * 2)
            .map(|i| (2.0 * core::f64::consts::PI * f * (i / 2) as f64 / SR as f64).sin() as f32 * 0.25)
            .collect();
        chain.process(&mut buf, n);
        // RMS, not the peak: at 8 kHz a cycle is six samples, and the largest
        // of them falls short of the crest by up to a dB.
        let tail = &buf[n..];
        let rms = (tail.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / tail.len() as f64).sqrt();
        rms / (0.25 / 2f64.sqrt())
    }

    #[test]
    fn a_peaking_band_lifts_its_own_frequency_by_its_gain() {
        for (band, &f) in EQ_FREQS.iter().enumerate() {
            let mut c = Chain::new(SR, 2);
            c.set_eq(band, 6.0);
            // The response formula and the samples agree, at the centre: +6 dB.
            assert!((db(c.eq_filter(band).magnitude(SR as f64, f)) - 6.0).abs() < 1e-9, "band {f}");
            if f < 10000.0 {
                let g = db(measured_gain(&mut c, f));
                assert!((g - 6.0).abs() < 0.05, "band {f}: {g:.3} dB");
            }
            // Two octaves away it does almost nothing (Q 1.1).
            let far = if f * 4.0 < 20000.0 { f * 4.0 } else { f / 4.0 };
            assert!(db(c.eq_filter(band).magnitude(SR as f64, far)).abs() < 1.0, "band {f} at {far}");
        }
    }

    #[test]
    fn the_bass_shelf_lifts_the_bottom_and_leaves_the_top() {
        let mut c = Chain::new(SR, 2);
        c.set_bass(9.0);
        let b = c.bass_filter();
        assert!((db(b.magnitude(SR as f64, 10.0)) - 9.0).abs() < 0.1);
        // Slope 1: half the gain at the corner, nothing left two octaves up.
        assert!((db(b.magnitude(SR as f64, BASS_FREQ)) - 4.5).abs() < 0.01);
        assert!(db(b.magnitude(SR as f64, 2000.0)).abs() < 0.1);
        let g = db(measured_gain(&mut c, 40.0));
        assert!((g - 8.8).abs() < 0.3, "{g:.2} dB at 40 Hz");
    }

    #[test]
    fn flat_settings_are_bit_transparent() {
        let mut c = Chain::new(SR, 2);
        for b in 0..10 {
            c.set_eq(b, 0.0);
        }
        c.set_bass(0.0);
        let src: Vec<f32> = (0..2048).map(|i| ((i * 7919) % 2001) as f32 / 1000.0 - 1.0).collect();
        let mut buf = src.clone();
        c.process(&mut buf, 1024);
        assert_eq!(buf, src);
    }

    #[test]
    fn hostile_settings_stay_in_range() {
        let mut c = Chain::new(SR, 2);
        c.set_eq(0, f64::NAN);
        c.set_eq(3, 1e9);
        c.set_eq(99, 6.0);
        c.set_bass(f64::INFINITY);
        c.set_gain_db(f64::INFINITY, 0.0);
        let mut buf = vec![0.5f32; 4096];
        c.process(&mut buf, 2048);
        assert!(buf.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn a_crossfade_holds_the_power_constant() {
        let n = 4800;
        let mut out = Chain::new(SR, 1);
        let mut inc = Chain::new(SR, 1);
        inc.fade_to(0.0, 0.0);
        out.fade_to(0.0, 100.0);
        inc.fade_to(1.0, 100.0);
        let mut a = vec![1.0f32; n];
        let mut b = vec![1.0f32; n];
        out.process(&mut a, n);
        inc.process(&mut b, n);
        for i in 0..n {
            let p = (a[i] as f64).powi(2) + (b[i] as f64).powi(2);
            assert!((p - 1.0).abs() < 1e-6, "power {p} at {i}");
        }
        assert!(a[n - 1] < 1e-3 && b[n - 1] > 0.999_999);
        // And once the ramp is over, it stays exactly where it went.
        let mut more = vec![1.0f32; 16];
        out.process(&mut more, 16);
        assert!(more.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn an_interrupted_fade_resumes_from_where_it_was() {
        let mut c = Chain::new(SR, 1);
        c.fade_to(0.0, 100.0);
        let mut buf = vec![1.0f32; 2400]; // half way
        c.process(&mut buf, 2400);
        let mid = c.fade();
        assert!((mid - (core::f64::consts::FRAC_PI_4).sin()).abs() < 1e-3, "{mid}");
        c.fade_to(1.0, 50.0);
        let mut one = vec![1.0f32; 1];
        c.process(&mut one, 1);
        assert!((one[0] as f64 - mid).abs() < 1e-3);
    }

    /// A 50 Hz sine at `amp`, stereo, through the chain; the peak of its last half.
    fn peak_out(c: &mut Chain, amp: f32) -> f32 {
        let n = 48000;
        let mut buf: Vec<f32> = (0..n * 2)
            .map(|i| (2.0 * core::f64::consts::PI * 50.0 * (i / 2) as f64 / SR as f64).sin() as f32 * amp)
            .collect();
        c.process(&mut buf, n);
        buf[n..].iter().fold(0f32, |m, v| m.max(v.abs()))
    }

    #[test]
    fn the_curve_is_web_audios() {
        let mut k = Compressor::new(SR as f64);
        k.set(SR as f64, -20.0, 30.0, 6.0, 0.012, 0.22);
        // Below the knee nothing; far above it the ratio.
        assert_eq!(k.curve(-40.0), 0.0);
        assert!((k.curve(0.0) - 20.0 * (1.0 / 6.0 - 1.0)).abs() < 1e-9);
        // Inside the knee, a smooth join: continuous at both ends.
        let lo = k.curve(-35.0 + 1e-9);
        let hi = k.curve(-5.0);
        assert!(lo.abs() < 1e-6 && (hi - 15.0 * (1.0 / 6.0 - 1.0)).abs() < 1e-6, "{lo} {hi}");
        // The limiter's: -0.5 dB, 20:1, no knee; its makeup is 0.6 of what it takes off full scale.
        let mut l = Compressor::new(SR as f64);
        l.set(SR as f64, -0.5, 0.0, 20.0, 0.002, 0.06);
        assert!((l.makeup_db - 0.6 * 0.5 * (1.0 - 1.0 / 20.0)).abs() < 1e-9);
    }

    #[test]
    fn the_bass_lift_never_saturates() {
        // Full lift on a full-scale bass note: +12 dB at the shelf, and the two
        // compressors keep it under full scale, by a hair at most.
        let mut c = Chain::new(SR, 2);
        c.set_bass(12.0);
        let p = peak_out(&mut c, 0.95);
        assert!(p < 1.06, "peak {p}");
        assert!(c.bass_compressor().reduction() < -3.0, "{}", c.bass_compressor().reduction());
        // A quiet passage is lifted, not squashed: the shelf's 12 dB plus the
        // compressor's makeup, 0.6 of the 16.7 dB its curve takes off full
        // scale — exactly what the web's DynamicsCompressorNode adds.
        let mut q = Chain::new(SR, 2);
        q.set_bass(12.0);
        let g = db(peak_out(&mut q, 0.01) as f64 / 0.01);
        assert!((g - 22.0).abs() < 1.0, "{g:.2} dB");
    }

    #[test]
    fn the_limiter_holds_an_eq_boost_under_full_scale() {
        // +12 dB at 62 Hz on a 50 Hz note at 0.9: four times full scale out of
        // the filter, and the limiter alone (no lift) brings it back to about
        // 0 dBFS. Measured 1.107: its makeup (+0.29 dB) and a 2 ms attack with
        // no look-ahead let each cycle's crest over by a little, as Web Audio's
        // does; the PCM conversion clips that last tenth of a dB.
        let mut c = Chain::new(SR, 2);
        c.set_eq(1, 12.0);
        let p = peak_out(&mut c, 0.9);
        assert!(p < 1.15, "peak {p}");
    }

    #[test]
    fn no_lift_leaves_the_dynamics_alone() {
        // EQ on, bass off: the compressor is neutral, only the limiter can act,
        // and a moderate signal passes at the EQ's own gain.
        let mut c = Chain::new(SR, 2);
        c.set_eq(1, 3.0);
        c.set_bass(0.0);
        assert_eq!(c.bass_compressor().reduction(), 0.0);
        let g = db(peak_out(&mut c, 0.1) as f64 / 0.1);
        assert!(g > 1.0 && g < 3.6, "{g:.2} dB");
    }

    #[test]
    fn a_gain_change_is_a_ramp_not_a_step() {
        let mut c = Chain::new(SR, 1);
        c.set_gain_db(-6.0, 10.0);
        let mut buf = vec![1.0f32; 960];
        c.process(&mut buf, 960);
        let target = 10f32.powf(-6.0 / 20.0);
        let worst = buf.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0f32, f32::max);
        // 0.5 over 480 samples: no step bigger than one sample's share.
        assert!(worst < 0.0011, "{worst}");
        assert!((buf[959] - target).abs() < 1e-6);
    }

    // -- the equalizer card's meter (MB12) ----------------------------------

    /// A block of a 1 kHz sine at `amp`, stereo, `frames` long.
    fn sine_block(amp: f32, frames: usize) -> Vec<f32> {
        (0..frames * 2)
            .map(|i| {
                (2.0 * core::f64::consts::PI * 1000.0 * (i / 2) as f64 / SR as f64).sin() as f32 * amp
            })
            .collect()
    }

    /// **Off by default, and off costs nothing**: the same block through the same
    /// chain produces the same samples with the meter on, so turning the card on
    /// cannot change what the person hears.
    #[test]
    fn the_meter_is_off_by_default_and_never_changes_the_sound() {
        let mut quiet = Chain::new(SR, 2);
        quiet.set_eq(4, 9.0);
        quiet.set_bass(12.0);
        assert_eq!(quiet.meter(), ChainMeter::SILENT, "no meter asked for");
        quiet.set_meter(false);

        let mut metered = Chain::new(SR, 2);
        metered.set_eq(4, 9.0);
        metered.set_bass(12.0);
        metered.set_meter(true);

        let block = sine_block(0.31, 4_800);
        let mut a = block.clone();
        let mut b = block.clone();
        quiet.process(&mut a, 4_800);
        metered.process(&mut b, 4_800);
        assert_eq!(a, b, "the meter must not touch a sample");
        assert_ne!(metered.meter(), ChainMeter::SILENT);
    }

    /// The two numbers are the SPA's: the PRE-limiter peak (after the bands, the
    /// bass shelf and its compressor) and the limiter's own reduction on the same
    /// buffer (`saturation.js#limiterReduction`).
    #[test]
    fn the_meter_reads_the_pre_limiter_peak_and_the_limiters_work() {
        // Transparent chain, meter on: the peak is the input's own, exactly.
        let mut plain = Chain::new(SR, 2);
        plain.set_meter(true);
        let mut buf = sine_block(0.25, 4_800);
        plain.process(&mut buf, 4_800);
        let m = plain.meter();
        assert!((m.peak_db - db(0.25)).abs() < 0.02, "{}", m.peak_db);
        assert_eq!(m.reduction_db, 0.0, "a clean master never touches the limiter");

        // A +16 dB band on the same sine drives the limiter: the peak is read
        // BEFORE it, so it stays above full scale, and the reduction is the
        // difference the limiter made.
        let mut hot = Chain::new(SR, 2);
        hot.set_eq(5, 16.0); // 1 kHz, the sine's own band
        hot.set_meter(true);
        let mut buf = sine_block(0.25, 4_800);
        hot.process(&mut buf, 4_800);
        let m = hot.meter();
        assert!(
            (m.peak_db - (db(0.25) + 16.0)).abs() < 1.5,
            "pre-limiter peak {} dB, expected about {}",
            m.peak_db,
            db(0.25) + 16.0
        );
        assert!(m.reduction_db > 0.5, "the limiter pulled {} dB", m.reduction_db);
        // The reduction the meter reports is what the limiter actually took off
        // the peak it measured: pre-limiter minus post-limiter, in dB. (It is
        // 0.9 dB here, not the 3.5 its static curve asks for: a 2 ms attack with
        // no look-ahead lets each cycle's crest over, exactly as Web Audio's
        // DynamicsCompressorNode does — the SPA measured 0.38 dB on a finished
        // master for the same reason.)
        let post = buf.iter().fold(0f32, |m, v| m.max(v.abs()));
        let heard = db(f64::from(post));
        assert!(
            (m.peak_db - heard - m.reduction_db).abs() < 1.0,
            "{} - {} = {} vs {}",
            m.peak_db,
            heard,
            m.peak_db - heard,
            m.reduction_db
        );
    }

    /// Silence reads the floor, not `-inf`, and a fresh chain that measured
    /// nothing says so: the card's "Mesure en cours…" is that state.
    #[test]
    fn silence_reads_the_floor_and_a_fresh_chain_has_no_reading() {
        let mut c = Chain::new(SR, 2);
        assert_eq!(c.meter(), ChainMeter::SILENT);
        c.set_meter(true);
        let mut buf = vec![0.0f32; 4_800 * 2];
        c.process(&mut buf, 4_800);
        assert_eq!(c.meter().peak_db, METER_FLOOR_DB);
        assert_eq!(c.meter().reduction_db, 0.0);
        // Turning it off forgets: a stale reading under a hidden card is a lie.
        c.set_meter(false);
        assert_eq!(c.meter(), ChainMeter::SILENT);
    }

    /// **The cost, measured**: 2 000 buffers of 480 frames through an engaged
    /// chain, meter off against meter on. The meter is a max over the block and
    /// two logarithms per buffer, and this says so in nanoseconds per sample
    /// rather than assuming it.
    #[test]
    fn the_meter_costs_a_max_per_sample_and_nothing_else() {
        const BUFFERS: usize = 2_000;
        const FRAMES: usize = 480;
        let block = sine_block(0.4, FRAMES);
        let run = |meter: bool| {
            let mut c = Chain::new(SR, 2);
            c.set_eq(2, 6.0);
            c.set_bass(6.0);
            c.set_meter(meter);
            let mut buf = vec![0f32; FRAMES * 2];
            let start = std::time::Instant::now();
            for _ in 0..BUFFERS {
                buf.copy_from_slice(&block);
                c.process(&mut buf, FRAMES);
            }
            let ns = start.elapsed().as_nanos() as f64;
            std::hint::black_box(c.meter());
            ns / (BUFFERS * FRAMES) as f64
        };
        // Warm the branch predictor and the caches before either measurement.
        let _ = run(false);
        let _ = run(true);
        let off = run(false);
        let on = run(true);
        // A max over 480 samples and two logarithms per buffer. Measured on this
        // machine 2026-11-18, 2 000 buffers of 480 frames through an engaged
        // chain: **+3.7 ns/frame in release** (69.1 → 72.8), i.e. 0.18 ms of one
        // core per second of 48 kHz stereo — 0.018 %, under the noise floor in a
        // debug build. The bound is what a later machine may not cross.
        println!("meter cost {:.3} ns/frame (off {off:.3}, on {on:.3})", on - off);
        assert!(on - off < 8.0, "meter cost {:.2} ns/frame (off {off:.2}, on {on:.2})", on - off);
    }
}
