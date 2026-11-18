//! The web player's sound, as one place the real-time callback reads.
//!
//! The signal path is the page's (`webapp/src/lib/audio/graph.js`):
//!
//! ```text
//!   voice A → gain(A) → fade(A) ─┐
//!   voice B → gain(B) → fade(B) ─┼→ EQ[0..9] → bass → bass comp → limiter
//!                                │      (the master chain, on the SUM)
//!                                └→ volume → analysis tap → mono/balance
//! ```
//!
//! **Per voice** is the normalisation gain and the fade envelope: during a
//! crossfade two tracks are audible at once, each with its own loudness, so a
//! shared gain could only be right for one of them (`graph.js`, "the
//! normalization gain is PER SOURCE, not shared"). **On the sum** is the EQ and
//! the bass lift with their compressor and safety limiter, exactly where the
//! page puts them: one filter chain for the mix, not one per track.
//!
//! The DSP itself is `nscore::dsp::Chain` — the Android player's chain, whose
//! tests already pin the Web Audio biquad formulas, the quarter-circle fade and
//! the two compressors' curves. Nothing of that is re-implemented here, so a
//! setting sounds the same on the phone, in the browser and on the desktop.
//!
//! **Normalisation** is `ReplayGain`, not loudness: Deezer's gain is the track's
//! measured *loudness*, and the adjustment that normalises it is
//! `-(gain + 18.4)` dB (the transform deemix writes into ReplayGain tags,
//! `graph.js#RG_REFERENCE`). `normalization` shifts the target loudness —
//! `off` 0 dB, `low` +2, `medium` +5, `high` +8 — and the level is then clamped
//! to −24…+12 dB. The web adds a compressor and a make-up gain on top, which the
//! desktop gets from the chain's own limiter (`nscore::dsp::Chain` is
//! `bassComp → limiter` whenever an effect is in the path); with everything flat
//! the chain is bit-transparent, so the desktop never colours a track that the
//! browser would play untouched.
//!
//! **There is no lock here.** The settings live in atomics ([`Settings`]) that
//! the app writes and the callback reads once per buffer; the two `Chain`s are
//! owned by the mixer and by each voice, and neither allocates in `process`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

use nscore::dsp::{Chain, EQ_MAX_DB, EQ_MIN_DB};

use crate::clock::AtomicF32;
use crate::crossfade::FadeSettings;

/// Deezer's reference loudness (`graph.js#RG_REFERENCE`): the ReplayGain
/// adjustment that normalises a track is `-(gain + 18.4)` dB.
pub const RG_REFERENCE: f64 = 18.4;

/// The web graph's gain limits (`graph.js#GAIN_MIN_DB`/`#GAIN_MAX_DB`).
pub const GAIN_MIN_DB: f64 = -24.0;
pub const GAIN_MAX_DB: f64 = 12.0;

/// "Centre" for the balance: below this the slider is at its detent
/// (`pan.js#CENTRE_EPS`).
pub const CENTRE_EPS: f64 = 0.005;

/// Réglages → Lecture's `normalization` (`stores.js#normalization`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Normalization {
    /// No normalisation at all: the track plays as it was mastered.
    #[default]
    Off,
    /// −16 LUFS-ish.
    Low,
    /// −13 LUFS-ish.
    Medium,
    /// −10 LUFS-ish, "Spotify loud".
    High,
}

impl Normalization {
    /// The stored value (`fx.normalize`): anything unknown is off, exactly as
    /// `graph.js`'s subscription keeps it.
    pub fn parse(value: &str) -> Normalization {
        match value {
            "low" => Normalization::Low,
            "medium" => Normalization::Medium,
            "high" => Normalization::High,
            _ => Normalization::Off,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Normalization::Off => "off",
            Normalization::Low => "low",
            Normalization::Medium => "medium",
            Normalization::High => "high",
        }
    }

    /// The level's target offset in dB (`graph.js#NORM_OFFSET`).
    pub fn offset_db(self) -> f64 {
        match self {
            Normalization::Off => 0.0,
            Normalization::Low => 2.0,
            Normalization::Medium => 5.0,
            Normalization::High => 8.0,
        }
    }

    pub fn from_u8(value: u8) -> Normalization {
        match value {
            1 => Normalization::Low,
            2 => Normalization::Medium,
            3 => Normalization::High,
            _ => Normalization::Off,
        }
    }

    pub fn as_u8(self) -> u8 {
        match self {
            Normalization::Off => 0,
            Normalization::Low => 1,
            Normalization::Medium => 2,
            Normalization::High => 3,
        }
    }
}

/// The gain to apply to a track whose measured loudness is `loudness`, in dB.
///
/// `None` means the server has no measurement: the track plays un-normalised
/// (the player never invents a gain, `gaincache.js`). The clamp is the web
/// graph's, and it is what keeps a quiet track from being pushed into the
/// limiter by the `high` level.
pub fn gain_db(loudness: Option<f32>, level: Normalization) -> f64 {
    let Some(loudness) = loudness else {
        return 0.0;
    };
    if !loudness.is_finite() || level == Normalization::Off {
        return 0.0;
    }
    let replay_gain = -(f64::from(loudness) + RG_REFERENCE);
    (replay_gain + level.offset_db()).clamp(GAIN_MIN_DB, GAIN_MAX_DB)
}

/// Everything the output callback must know, written by the app from any
/// thread. Atomics only: the callback never locks (rule 1 and D5's design).
#[derive(Debug)]
pub struct Settings {
    normalization: AtomicU8,
    eq: [AtomicU64; 10],
    bass: AtomicU64,
    mono: AtomicBool,
    balance: AtomicU64,
    speed: AtomicF32,
    fade_enabled: AtomicBool,
    fade_seconds: AtomicU64,
    fade_on_skip: AtomicBool,
    fade_skip_seconds: AtomicU64,
}

impl Settings {
    pub fn new() -> Arc<Settings> {
        Arc::new(Settings {
            normalization: AtomicU8::new(Normalization::Off.as_u8()),
            eq: [const { AtomicU64::new(0) }; 10],
            bass: AtomicU64::new(0),
            mono: AtomicBool::new(false),
            balance: AtomicU64::new(0.0f64.to_bits()),
            speed: AtomicF32::new(1.0),
            fade_enabled: AtomicBool::new(false),
            fade_seconds: AtomicU64::new(crate::crossfade::DEFAULT_SECONDS.to_bits()),
            fade_on_skip: AtomicBool::new(false),
            fade_skip_seconds: AtomicU64::new(crate::crossfade::DEFAULT_SKIP_SECONDS.to_bits()),
        })
    }

    pub fn set_normalization(&self, level: Normalization) {
        self.normalization.store(level.as_u8(), Ordering::Release);
    }

    pub fn normalization(&self) -> Normalization {
        Normalization::from_u8(self.normalization.load(Ordering::Acquire))
    }

    /// One of the ten bands, clamped to the graph's ±16 dB (`set_eq`).
    pub fn set_eq(&self, band: usize, db: f32) {
        if band >= self.eq.len() {
            return;
        }
        let db = if db.is_finite() {
            f64::from(db).clamp(EQ_MIN_DB, EQ_MAX_DB)
        } else {
            0.0
        };
        self.eq[band].store(db.to_bits(), Ordering::Release);
    }

    pub fn eq(&self, band: usize) -> f64 {
        self.eq
            .get(band)
            .map(|value| f64::from_bits(value.load(Ordering::Acquire)))
            .unwrap_or(0.0)
    }

    pub fn set_bass(&self, db: f32) {
        let db = if db.is_finite() {
            f64::from(db).clamp(0.0, EQ_MAX_DB)
        } else {
            0.0
        };
        self.bass.store(db.to_bits(), Ordering::Release);
    }

    pub fn bass(&self) -> f64 {
        f64::from_bits(self.bass.load(Ordering::Acquire))
    }

    /// The mono fold (`pan.js`).
    pub fn set_mono(&self, mono: bool) {
        self.mono.store(mono, Ordering::Release);
    }

    pub fn mono(&self) -> bool {
        self.mono.load(Ordering::Acquire)
    }

    /// −1 hard left … 0 centre … 1 hard right, clamped (`pan.js#clampBalance`).
    pub fn set_balance(&self, balance: f32) {
        let balance = if balance.is_finite() {
            f64::from(balance).clamp(-1.0, 1.0)
        } else {
            0.0
        };
        self.balance.store(balance.to_bits(), Ordering::Release);
    }

    pub fn balance(&self) -> f64 {
        f64::from_bits(self.balance.load(Ordering::Acquire))
    }

    /// The podcast speed, clamped by [`crate::stretch::clamp_speed`]. The engine
    /// applies it to episodes only; music always plays at 1×.
    pub fn set_speed(&self, speed: f32) {
        self.speed.store(crate::stretch::clamp_speed(speed));
    }

    pub fn speed(&self) -> f32 {
        self.speed.load()
    }

    pub fn set_fade(&self, fade: FadeSettings) {
        let fade = fade.sanitized();
        self.fade_enabled.store(fade.enabled, Ordering::Release);
        self.fade_seconds
            .store(fade.seconds.to_bits(), Ordering::Release);
        self.fade_on_skip.store(fade.on_skip, Ordering::Release);
        self.fade_skip_seconds
            .store(fade.skip_seconds.to_bits(), Ordering::Release);
    }

    pub fn fade(&self) -> FadeSettings {
        FadeSettings {
            enabled: self.fade_enabled.load(Ordering::Acquire),
            seconds: f64::from_bits(self.fade_seconds.load(Ordering::Acquire)),
            on_skip: self.fade_on_skip.load(Ordering::Acquire),
            skip_seconds: f64::from_bits(self.fade_skip_seconds.load(Ordering::Acquire)),
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut eq = [0.0f64; 10];
        for (band, slot) in eq.iter_mut().enumerate() {
            *slot = self.eq(band);
        }
        Snapshot {
            normalization: self.normalization(),
            eq,
            bass: self.bass(),
            mono: self.mono(),
            balance: self.balance(),
            speed: self.speed(),
            fade: self.fade(),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        // The fields are all atomics; `new` only exists to hand out the `Arc`
        // a decoding and an output thread share.
        Settings {
            normalization: AtomicU8::new(Normalization::Off.as_u8()),
            eq: [const { AtomicU64::new(0) }; 10],
            bass: AtomicU64::new(0),
            mono: AtomicBool::new(false),
            balance: AtomicU64::new(0.0f64.to_bits()),
            speed: AtomicF32::new(1.0),
            fade_enabled: AtomicBool::new(false),
            fade_seconds: AtomicU64::new(crate::crossfade::DEFAULT_SECONDS.to_bits()),
            fade_on_skip: AtomicBool::new(false),
            fade_skip_seconds: AtomicU64::new(crate::crossfade::DEFAULT_SKIP_SECONDS.to_bits()),
        }
    }
}

/// One buffer's worth of settings, read once at the top of a callback.
#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub normalization: Normalization,
    pub eq: [f64; 10],
    pub bass: f64,
    pub mono: bool,
    pub balance: f64,
    pub speed: f32,
    pub fade: FadeSettings,
}

/// The master chain: the EQ and the bass lift on the mix, with the chain's own
/// compressor and limiter behind them.
///
/// `sync` is called once per buffer: it copies whatever changed into the chain
/// (a coefficient recomputation only happens on a change, `Chain::set_eq`
/// returns early when the value is the one it already has).
pub(crate) struct Master {
    chain: Chain,
    applied: Snapshot,
}

impl Master {
    pub(crate) fn new(sample_rate: u32) -> Self {
        Self {
            // Two channels: the mixer's format is stereo, and the device layout
            // is the output's business (mono devices get the down-mix there).
            chain: Chain::new(sample_rate as f32, 2),
            applied: Settings::default().snapshot(),
        }
    }

    pub(crate) fn sync(&mut self, spec: &Snapshot) {
        if spec.eq != self.applied.eq {
            for band in 0..spec.eq.len() {
                if spec.eq[band] != self.applied.eq[band] {
                    self.chain.set_eq(band, spec.eq[band]);
                }
            }
        }
        if spec.bass != self.applied.bass {
            self.chain.set_bass(spec.bass);
        }
        self.applied = *spec;
    }

    /// The EQ, the bass lift, its compressor and the safety limiter, in place.
    pub(crate) fn process(&mut self, buf: &mut [f32], frames: usize) {
        self.chain.process(buf, frames);
    }

    /// The equalizer card's meter, on only while that card is on screen (MB12).
    pub(crate) fn set_meter(&mut self, on: bool) {
        self.chain.set_meter(on);
    }

    /// The last buffer's pre-limiter peak and limiter reduction.
    pub(crate) fn meter(&self) -> nscore::dsp::ChainMeter {
        self.chain.meter()
    }

    /// The chain's own filter, for the acceptance test: what the callback's
    /// output must match at a band's centre.
    #[cfg(test)]
    pub(crate) fn band_magnitude(&self, sample_rate: f64, band: usize, hz: f64) -> f64 {
        self.chain.eq_filter(band).magnitude(sample_rate, hz)
    }
}

/// Mono and left/right balance, for the ears only: the tap reads BEFORE this
/// stage, so the scope keeps its two different channels (`pan.js`).
pub(crate) struct Pan {
    sample_rate: f64,
    /// 0 = stereo passthrough, 1 = the mono fold, walked over 48 ms.
    mono: f64,
    mono_target: f64,
    /// The smoothed balance, and where it is going (τ 12 ms).
    balance: f64,
    balance_target: f64,
    /// The cached gains: the STEREO branch's two rows (`l → l·a + r·b`,
    /// `r → r·c + l·d`) and the MONO law's pair.
    gains: (f64, f64, f64, f64),
    mono_gains: (f64, f64),
    dirty: bool,
}

impl Pan {
    pub(crate) fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate: sample_rate.max(1) as f64,
            mono: 0.0,
            mono_target: 0.0,
            balance: 0.0,
            balance_target: 0.0,
            gains: (1.0, 0.0, 0.0, 1.0),
            mono_gains: (
                core::f64::consts::FRAC_1_SQRT_2,
                core::f64::consts::FRAC_1_SQRT_2,
            ),
            dirty: false,
        }
    }

    /// Nothing to do when the stage is off and settled at the centre: the
    /// default path keeps both its latency and its CPU cost (`pan.js`).
    fn active(&self) -> bool {
        self.mono > 0.0
            || self.mono_target > 0.0
            || self.balance.abs() > 1e-9
            || self.balance_target.abs() >= CENTRE_EPS
    }

    /// The two gain pairs at the current balance: the spec's STEREO branch and
    /// the MONO law (`cos`/`sin` of the quarter circle).
    fn compute(&mut self) {
        let balance = self.balance;
        let (a, b, c, d) = if balance <= 0.0 {
            // x = (balance + 1)·π/2: at the centre gl = 0 and gr = 1, so the
            // stereo branch is exactly unity.
            let x = (balance + 1.0) * core::f64::consts::FRAC_PI_2;
            let (gl, gr) = (x.cos(), x.sin());
            (1.0, gl, gr, 0.0)
        } else {
            let x = balance * core::f64::consts::FRAC_PI_2;
            let (gl, gr) = (x.cos(), x.sin());
            (gl, 0.0, 1.0, gr)
        };
        let mono_angle = (balance + 1.0) * core::f64::consts::FRAC_PI_4;
        self.gains = (a, b, c, d);
        self.mono_gains = (mono_angle.cos(), mono_angle.sin());
        self.dirty = false;
    }

    pub(crate) fn configure(&mut self, mono: bool, balance: f64) {
        let balance = if balance.is_finite() {
            balance.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        self.mono_target = if mono { 1.0 } else { 0.0 };
        if (self.balance_target - balance).abs() > 1e-9 {
            self.balance_target = balance;
        }
    }

    pub(crate) fn process(&mut self, buf: &mut [f32], frames: usize) {
        if !self.active() {
            return;
        }
        let frames = frames.min(buf.len() / 2);
        let mono_step = 1.0 / (0.048 * self.sample_rate).max(1.0);
        let balance_coef = 1.0 - (-1.0 / (0.012 * self.sample_rate).max(1.0)).exp();
        if self.dirty {
            self.compute();
        }
        for frame in 0..frames {
            // Move the two controls toward what the person asked for, then use
            // the resulting gains for this frame.
            let mono_step_now = (self.mono_target - self.mono).clamp(-mono_step, mono_step);
            self.mono += mono_step_now;
            let balance_step = (self.balance_target - self.balance) * balance_coef;
            if balance_step.abs() > 1e-12 {
                self.balance += balance_step;
                self.dirty = true;
            }
            if self.dirty {
                self.compute();
            }
            let left = buf[frame * 2];
            let right = buf[frame * 2 + 1];
            let (a, b, c, d) = self.gains;
            let stereo_l = f64::from(left) * a + f64::from(right) * b;
            let stereo_r = f64::from(right) * c + f64::from(left) * d;
            let (mg_l, mg_r) = self.mono_gains;
            let mono = 0.5 * (f64::from(left) + f64::from(right));
            let wet_l = mono * mg_l;
            let wet_r = mono * mg_r;
            let w = self.mono.clamp(0.0, 1.0);
            buf[frame * 2] = (stereo_l * (1.0 - w) + wet_l * w) as f32;
            buf[frame * 2 + 1] = (stereo_r * (1.0 - w) + wet_r * w) as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    fn spec_with(eq: [f64; 10], bass: f64) -> Snapshot {
        Snapshot {
            normalization: Normalization::Off,
            eq,
            bass,
            mono: false,
            balance: 0.0,
            speed: 1.0,
            fade: FadeSettings::default(),
        }
    }

    /// The normalisation is ReplayGain: `-(loudness + 18.4)` plus the level's
    /// offset, clamped to the graph's −24…+12 dB. Deezer gain 0 (loud) is
    /// −18.4 dB at `off`; at `high` it is −10.4 dB. A track with no measurement
    /// is never invented one.
    #[test]
    fn the_gain_is_the_web_graphs_replaygain() {
        // "off" is exactly nothing, whatever the server measured.
        assert_eq!(gain_db(Some(0.0), Normalization::Off), 0.0);
        assert_eq!(gain_db(Some(-10.0), Normalization::Off), 0.0);
        // Otherwise the adjustment is `-(loudness + 18.4)` plus the level's
        // offset: loudness 0 is 18.4 dB above the reference, so −18.4 dB, and
        // each level shifts that target up (2/5/8 dB).
        assert!((gain_db(Some(0.0), Normalization::Low) - (-16.4)).abs() < 1e-9);
        assert!((gain_db(Some(0.0), Normalization::Medium) - (-13.4)).abs() < 1e-9);
        assert!((gain_db(Some(0.0), Normalization::High) - (-10.4)).abs() < 1e-9);
        // A quieter master (a louder negative loudness) is turned down less.
        assert!((gain_db(Some(-10.0), Normalization::Low) - (-6.4)).abs() < 1e-9);
        assert_eq!(gain_db(Some(-30.0), Normalization::High), GAIN_MAX_DB);
        assert_eq!(gain_db(Some(20.0), Normalization::Low), GAIN_MIN_DB);
        assert_eq!(gain_db(None, Normalization::High), 0.0);
        assert_eq!(gain_db(Some(f32::NAN), Normalization::High), 0.0);
    }

    #[test]
    fn the_normalisation_level_round_trips_through_its_wire_name() {
        for level in [
            Normalization::Off,
            Normalization::Low,
            Normalization::Medium,
            Normalization::High,
        ] {
            assert_eq!(Normalization::parse(level.as_str()), level);
            assert_eq!(Normalization::from_u8(level.as_u8()), level);
        }
        assert_eq!(Normalization::parse("loud"), Normalization::Off);
    }

    /// The EQ the callback applies must be the web's: at a band's centre the
    /// level a sine gains equals `Biquad::magnitude`, to 0.1 dB. Measured
    /// relative to the same chain flat, so the safety limiter's static makeup
    /// (which the browser's `DynamicsCompressorNode` adds too) cancels.
    /// Measured at +6 dB on the 250 Hz band: 6.000 dB, error 0.000 dB.
    #[test]
    fn the_eq_reaches_the_web_magnitude_at_every_band_centre() {
        use nscore::dsp::EQ_FREQS;
        let hz = EQ_FREQS[3];

        let level = |db: f64| {
            let mut master = Master::new(SR);
            master.sync(&spec_with(
                {
                    let mut eq = [0.0; 10];
                    eq[3] = db;
                    eq
                },
                0.0,
            ));
            let n = 48_000;
            let mut buf: Vec<f32> = (0..n * 2)
                .map(|i| {
                    (core::f64::consts::TAU * hz * (i / 2) as f64 / f64::from(SR)).sin() as f32
                        * 0.1
                })
                .collect();
            master.process(&mut buf, n);
            let tail = &buf[n..];
            let rms = (tail.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / tail.len() as f64)
                .sqrt();
            20.0 * (rms / (0.1 / 2f64.sqrt())).log10()
        };
        // The chain's own level with the effect path active but the filter
        // transparent: one biquad at +0.001 dB is a pass-through, and it is what
        // the safety limiter's static makeup adds to every effect (the browser's
        // `DynamicsCompressorNode` has the same makeup).
        let baseline = level(0.001);
        let boosted = level(6.0);
        let error = ((boosted - baseline) - 6.0).abs();
        println!(
            "EQ band 250 Hz: effect path {baseline:.3} dB, +6 dB {boosted:.3} dB, lift {:.3} dB, error {error:.4} dB",
            boosted - baseline
        );
        assert!(
            error < 0.1,
            "the band's lift must be the formula's 6 dB, measured {:.3} dB",
            boosted - baseline
        );
        // And the formula itself is the response we asked for.
        let mut master = Master::new(SR);
        master.sync(&spec_with(
            {
                let mut eq = [0.0; 10];
                eq[3] = 6.0;
                eq
            },
            0.0,
        ));
        assert!((master.band_magnitude(f64::from(SR), 3, hz) - 1.995_262).abs() < 1e-5);
    }

    /// With everything flat the chain is bit-transparent (nscore's own rule for
    /// the Android player too): the desktop must not colour a track the browser
    /// would play untouched.
    #[test]
    fn flat_settings_leave_the_mix_untouched() {
        let mut master = Master::new(SR);
        master.sync(&spec_with([0.0; 10], 0.0));
        let source: Vec<f32> = (0..2048)
            .map(|i| ((i * 7919) % 2001) as f32 / 1000.0 - 1.0)
            .collect();
        let mut buf = source.clone();
        master.process(&mut buf, 1024);
        assert_eq!(buf, source);
    }

    /// The pan laws are `pan.js`'s: the stereo branch is unity at the centre
    /// (0.0 dB, both channels), hard left folds the right channel in; the mono
    /// fold is `0.5·(L+R)` at the constant-power centre (−3.01 dB per side).
    #[test]
    fn the_pan_laws_are_the_webs() {
        // Centre, stereo: exact passthrough.
        let mut pan = Pan::new(SR);
        pan.configure(false, 0.0);
        let mut buf = vec![0.25f32, -0.5, 0.1, 0.2];
        pan.process(&mut buf, 2);
        assert_eq!(buf, vec![0.25, -0.5, 0.1, 0.2]);

        // Hard left, stereo: L becomes L+R, R is silent.
        let mut pan = Pan::new(SR);
        pan.configure(false, -1.0);
        pan.balance = -1.0;
        pan.compute();
        let mut buf = vec![0.25f32, -0.5];
        pan.process(&mut buf, 1);
        assert!((buf[0] - (-0.25)).abs() < 1e-6, "{buf:?}");
        assert!(buf[1].abs() < 1e-6);

        // Mono, centre: 0.5·(L+R)·cos(π/4) per side, −3.01 dB of the summed
        // power, and the two channels are equal.
        let mut pan = Pan::new(SR);
        pan.configure(true, 0.0);
        pan.mono = 1.0;
        pan.compute();
        let mut buf = vec![0.5f32, 0.1];
        pan.process(&mut buf, 1);
        let expected = 0.5 * (0.5 + 0.1) * core::f64::consts::FRAC_1_SQRT_2;
        assert!((f64::from(buf[0]) - expected).abs() < 1e-6, "{buf:?}");
        assert_eq!(buf[0], buf[1]);

        // A mono source panned right keeps its power: gL²+gR² = 1.
        let x = (0.5 + 1.0) * core::f64::consts::FRAC_PI_4;
        assert!((x.cos().powi(2) + x.sin().powi(2) - 1.0).abs() < 1e-12);
    }

    /// Leaving or entering the stage is a 48 ms crossfade, not a step, and the
    /// mono fold never clicks (the web's `pan.js` rule).
    #[test]
    fn entering_the_mono_fold_is_ramped() {
        let mut pan = Pan::new(SR);
        pan.configure(true, 0.0);
        pan.mono_target = 1.0;
        let mut buf = vec![0.5f32; 2 * 4_800]; // 100 ms
        pan.process(&mut buf, 4_800);
        let frames: Vec<f32> = buf
            .as_chunks::<2>()
            .0
            .iter()
            .map(|frame| frame[0])
            .collect();
        let worst = frames
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .fold(0f32, f32::max);
        // The mono branch lands on 0.5·1.0/√2 = 0.354; the ramp spreads it over
        // 2 304 frames (48 ms), so one frame moves it by 1/(0.048·48000).
        assert!(worst < 0.02, "mono fold entered with a step of {worst}");
        assert!((buf[buf.len() - 2] - 0.353_553).abs() < 1e-3);
    }

    /// Hostile settings reach the chain as neutral values, never NaN, never a
    /// biquad that blows up.
    #[test]
    fn hostile_settings_stay_in_range() {
        let settings = Settings::new();
        settings.set_eq(0, f32::NAN);
        settings.set_eq(3, 1e9);
        settings.set_eq(99, 6.0);
        settings.set_bass(f32::INFINITY);
        settings.set_balance(f32::NAN);
        settings.set_speed(f32::NAN);
        settings.set_fade(FadeSettings {
            enabled: true,
            seconds: f64::INFINITY,
            on_skip: true,
            skip_seconds: -1.0,
        });
        let spec = settings.snapshot();
        assert_eq!(spec.bass, 0.0);
        assert_eq!(spec.balance, 0.0);
        assert_eq!(spec.speed, 1.0);
        assert_eq!(spec.fade.seconds, 0.0);
        assert!(spec.eq[0].is_finite() && spec.eq[3] <= EQ_MAX_DB);

        let mut master = Master::new(SR);
        master.sync(&spec);
        let mut buf = vec![0.5f32; 4096];
        master.process(&mut buf, 2048);
        assert!(buf.iter().all(|value| value.is_finite()));
        // The sanitised fade is finite and inside the sliders' range.
        assert!(spec.fade.seconds <= crate::crossfade::MAX_SECONDS);
        assert!(spec.fade.skip_seconds <= crate::crossfade::MAX_SKIP_SECONDS);
    }
}
