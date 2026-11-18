//! What the equalizer can do to the sound, as the card says it.
//!
//! The rules are `webapp/src/lib/audio/saturation.js` (`classifyRisk`, its four
//! thresholds) and `webapp/src/lib/eqpresets.js` (`eqRisk`, `createRiskHold`,
//! `fmtDb`, `volumeCutDb`, `dbToGain`) — ported, not reinvented. The numbers and
//! the reasons for them live in `docs/agents/playback.md` (*The warning is about
//! the SETTING, not the recording* and *The banner's number is a measurement with
//! a baseline*); the assertions beside the tests below carry the measurements
//! this port reproduced.

use std::fmt;

/// The three states a reading can be in (`RISK_TONES`). Anything else — a bug in
/// a reading, an unknown wire value — is `Ok`, never a warning: a broken number
/// must not paint the screen red.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Warn,
    Clip,
}

impl Tone {
    pub fn as_str(self) -> &'static str {
        match self {
            Tone::Ok => "ok",
            Tone::Warn => "warn",
            Tone::Clip => "clip",
        }
    }

    /// An unknown value is `Ok` (`riskTone`): a reading this build does not know
    /// is not evidence of anything.
    pub fn parse(value: &str) -> Tone {
        match value {
            "warn" => Tone::Warn,
            "clip" => Tone::Clip,
            _ => Tone::Ok,
        }
    }
}

impl fmt::Display for Tone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Warn at +2 dBFS pre-limiter or 2 dB of limiter reduction (`WARN_*_DB`).
pub const WARN_PEAK_DB: f64 = 2.0;
/// Clip at +6 dBFS or 6 dB of reduction (`CLIP_*_DB`).
pub const CLIP_PEAK_DB: f64 = 6.0;
/// The reduction that makes the limiter the cause, not the master's level.
pub const WARN_REDUCTION_DB: f64 = 2.0;
/// …and the one that says the chain is squashing the music.
pub const CLIP_REDUCTION_DB: f64 = 6.0;

/// A deliberate change keeps its baseline for this long: a slider drag is ONE
/// gesture, and a reading taken mid-drag already carries the earlier steps
/// (`createRiskHold`'s `gestureMs`).
pub const GESTURE_MS: f64 = 900.0;

/// The silence floor a reading reports instead of `-inf` (`PEAK_FLOOR_DB`).
pub const PEAK_FLOOR_DB: f64 = -96.0;

/// Classify a pre-limiter peak and the limiter's reduction. Pure.
pub fn classify_risk(peak_db: f64, reduction_db: f64) -> Tone {
    if peak_db >= CLIP_PEAK_DB || reduction_db >= CLIP_REDUCTION_DB {
        return Tone::Clip;
    }
    if peak_db >= WARN_PEAK_DB || reduction_db >= WARN_REDUCTION_DB {
        return Tone::Warn;
    }
    Tone::Ok
}

/// The risk a SETTING can cause, not the one a loud recording shows: a bypassed
/// equalizer, a flat curve and a curve made only of cuts add no energy, so the
/// limiter working on a loud master is the recording's level and the card says
/// nothing. Only a band above 0 dB can make the equalizer the cause (`eqRisk`).
pub fn eq_risk(risk: Tone, enabled: bool, bands: &[f32]) -> Tone {
    if !enabled {
        return Tone::Ok;
    }
    let boost = bands.iter().fold(0.0f32, |acc, v| {
        let v = if v.is_finite() { *v } else { 0.0 };
        if v > acc { v } else { acc }
    });
    if boost > 0.0 { risk } else { Tone::Ok }
}

/// One warning episode: it starts at the first warning and STAYS until the person
/// changes the setting — a peak that falls back 40 ms later must not take the
/// banner away under the reader's eyes. While it lasts, its numbers keep the
/// highest value seen.
#[derive(Debug, Clone, Default)]
pub struct RiskHold {
    tone: Option<Tone>,
    reduction: f64,
    shown: Option<Shown>,
    base: Option<Baseline>,
    last_at: f64,
    last_key: Option<String>,
    started: bool,
}

/// What the banner shows: the tone, and the reduction ABOVE the episode's
/// baseline (never the raw one, which blames a band for the master's level).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shown {
    pub tone: Tone,
    pub reduction_db: f64,
    /// Measured since the person's own change, not since the track started.
    pub from_change: bool,
}

/// The measurement a change starts from (`createRiskHold`'s `begin`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Baseline {
    pub reduction_db: f64,
    pub peak_db: Option<f64>,
}

/// What `net` answers: the reduction above the baseline, and whether there is one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Net {
    pub reduction_db: f64,
    pub peak_db: f64,
    pub from_change: bool,
}

impl RiskHold {
    pub fn new() -> RiskHold {
        RiskHold::default()
    }

    /// The episode on screen, if any.
    pub fn current(&self) -> Option<Shown> {
        self.shown
    }

    /// The measurement this episode is measured against.
    pub fn baseline(&self) -> Option<Baseline> {
        self.base
    }

    /// A deliberate change starts a new episode. `reference` is the reading that
    /// was on screen (so the baseline is the level the change started from);
    /// `now_ms` and `key` keep one gesture's baseline across a drag's steps.
    pub fn begin(&mut self, reference: Option<(f64, f64)>, now_ms: f64, key: Option<&str>) {
        let same = self.base.is_some()
            && key.is_some()
            && key == self.last_key.as_deref()
            && now_ms - self.last_at <= GESTURE_MS;
        if !same {
            self.base = reference.map(|(reduction, peak)| Baseline {
                reduction_db: reduction.max(0.0),
                peak_db: if peak.is_finite() { Some(peak) } else { None },
            });
        }
        self.last_at = now_ms;
        self.last_key = key.map(str::to_owned);
        self.tone = None;
        self.reduction = 0.0;
        self.shown = None;
        self.started = false;
    }

    /// The reduction above the episode's baseline. With no baseline (nothing was
    /// measured when the person changed the setting), the raw reduction is the
    /// only honest value.
    pub fn net(&self, raw_reduction_db: f64, raw_peak_db: f64) -> Net {
        let reduction = raw_reduction_db.max(0.0);
        let peak = if raw_peak_db.is_finite() {
            raw_peak_db
        } else {
            0.0
        };
        match self.base {
            None => Net {
                reduction_db: reduction,
                peak_db: peak,
                from_change: false,
            },
            Some(base) => Net {
                reduction_db: (reduction - base.reduction_db).max(0.0),
                peak_db: match base.peak_db {
                    None => 0.0,
                    Some(baseline_peak) => (peak - baseline_peak).max(0.0),
                },
                from_change: true,
            },
        }
    }

    /// Fold one reading into the episode. `Ok` never ends it: only `begin` (or
    /// `reset`) does. The reduction keeps the highest value seen.
    pub fn fold(&mut self, tone: Tone, net_reduction_db: f64, from_change: bool) -> Option<Shown> {
        if tone == Tone::Ok {
            return self.shown;
        }
        let tone = match (tone, self.tone) {
            (Tone::Clip, _) | (_, Some(Tone::Clip)) => Tone::Clip,
            _ => Tone::Warn,
        };
        self.tone = Some(tone);
        self.reduction = self.reduction.max(if net_reduction_db.is_finite() {
            net_reduction_db.max(0.0)
        } else {
            0.0
        });
        self.shown = Some(Shown {
            tone,
            reduction_db: self.reduction,
            from_change,
        });
        self.started = true;
        self.shown
    }

    /// Forget everything (the card left, the equalizer went off).
    pub fn reset(&mut self) {
        self.tone = None;
        self.reduction = 0.0;
        self.shown = None;
        self.started = false;
    }
}

/// French decimals with a real minus sign and a comma, one decimal, as the meter
/// publishes them. `-0,0` is not a reading: anything under half a tenth is zero.
pub fn fmt_db(value: f64, plus: bool) -> String {
    let n = if value.is_finite() { value } else { 0.0 };
    let shown = if n.abs() < 0.05 { 0.0 } else { n };
    let body = format!("{:.1}", shown.abs()).replace('.', ",");
    let sign = if shown < 0.0 {
        "\u{2212}"
    } else if plus && shown > 0.0 {
        "+"
    } else {
        ""
    };
    format!("{sign}{body} dB")
}

/// How much to pull the level back to release the limiter: the measured
/// reduction, rounded to half a decibel (a whole number of clicks on a phone),
/// never below 0.5 so the button always does something (`volumeCutDb`).
pub fn volume_cut_db(reduction_db: f64) -> f64 {
    let m = if reduction_db.is_finite() {
        reduction_db.max(0.0)
    } else {
        0.0
    };
    (m * 2.0).round().max(1.0) / 2.0
}

/// dB of attenuation as a linear gain (0.5 for -6 dB, 1 for 0) — `dbToGain`.
pub fn db_to_gain(db: f64) -> f64 {
    let db = if db.is_finite() { db.max(0.0) } else { 0.0 };
    10f64.powf(-db / 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four thresholds are the SPA's, at the values playback.md records: a
    /// finished master at 0 dBFS with 0.38 dB of reduction is normal and silent.
    #[test]
    fn the_thresholds_are_the_measured_ones() {
        assert_eq!(
            classify_risk(0.0, 0.38),
            Tone::Ok,
            "a modern master is not a fault"
        );
        assert_eq!(classify_risk(2.0, 0.0), Tone::Warn);
        assert_eq!(classify_risk(1.9, 1.9), Tone::Ok);
        assert_eq!(classify_risk(0.0, 2.0), Tone::Warn);
        assert_eq!(classify_risk(6.0, 0.0), Tone::Clip);
        assert_eq!(classify_risk(0.0, 6.0), Tone::Clip);
        assert_eq!(classify_risk(5.9, 5.9), Tone::Warn);
    }

    /// A loud recording with a flat, cut-only or bypassed curve never warns
    /// (`eqRisk`): the equalizer is not the cause, so the card says nothing.
    #[test]
    fn only_a_band_above_zero_makes_the_equalizer_the_cause() {
        let flat = [0.0f32; 10];
        let cut = [-3.0, -1.0, 0.0, -6.0, 0.0, 0.0, 0.0, 0.0, 0.0, -2.0];
        let boost = [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        assert_eq!(eq_risk(Tone::Clip, true, &flat), Tone::Ok);
        assert_eq!(eq_risk(Tone::Clip, true, &cut), Tone::Ok);
        assert_eq!(eq_risk(Tone::Warn, true, &boost), Tone::Warn);
        assert_eq!(eq_risk(Tone::Clip, false, &boost), Tone::Ok, "bypassed");
        // A hostile stored curve (NaN, infinities) is not a boost.
        assert_eq!(eq_risk(Tone::Warn, true, &[f32::NAN; 10]), Tone::Ok);
    }

    /// An episode survives readings that fall back: the banner is about the
    /// setting, and it keeps the highest reduction it saw until the person
    /// changes something. `playback.md`: « a peak that falls back 40 ms later
    /// must not take the banner away ».
    #[test]
    fn an_episode_stays_and_keeps_its_worst_reading() {
        let mut hold = RiskHold::new();
        assert_eq!(hold.current(), None);
        // The change starts from the level that was playing: 6.4 dB of reduction
        // on a flat curve (the bench's number for a hot master).
        hold.begin(Some((6.4, -0.1)), 1_000.0, Some("band:1"));
        let net = hold.net(7.6, 0.4);
        assert!(
            (net.reduction_db - 1.2).abs() < 1e-9,
            "{}",
            net.reduction_db
        );
        assert!(net.from_change);

        let first = hold.fold(Tone::Warn, net.reduction_db, net.from_change);
        assert!(first.is_some(), "the first warning opens the episode");
        // A quiet reading 40 ms later: `Ok` does not close it, and the numbers
        // stay the worst seen.
        let still = hold.fold(Tone::Ok, 0.0, false).expect("still shown");
        assert_eq!(still.tone, Tone::Warn);
        assert!((still.reduction_db - 1.2).abs() < 1e-9);
        // A worse reading raises the tone and the number for good.
        let worse = hold
            .fold(Tone::Clip, 6.0, true)
            .expect("a clip supersedes a warn");
        assert_eq!(worse.tone, Tone::Clip);
        assert!((worse.reduction_db - 6.0).abs() < 1e-9);
        // …and a later `Warn` cannot lower it back.
        let after = hold.fold(Tone::Warn, 0.5, true).expect("still the clip");
        assert_eq!(after.tone, Tone::Clip);
        assert!((after.reduction_db - 6.0).abs() < 1e-9);

        // The person's next change is a new episode with a new baseline.
        hold.begin(Some((12.0, 1.0)), 2_000.0, Some("band:1"));
        assert_eq!(hold.current(), None, "a change takes the old banner away");
        let net = hold.net(12.0, 1.0);
        assert_eq!(net.reduction_db, 0.0, "nothing added since the change");
    }

    /// A slider drag is ONE gesture: every step of it keeps the baseline the
    /// drag started from (900 ms), because a reading taken mid-drag already
    /// carries the earlier steps. A pause longer than that is a new gesture.
    #[test]
    fn a_drag_keeps_its_first_baseline_and_a_later_moves_starts_over() {
        let mut hold = RiskHold::new();
        hold.begin(Some((1.0, 0.0)), 0.0, Some("band:3"));
        hold.begin(Some((9.0, 5.0)), 400.0, Some("band:3"));
        let base = hold.baseline().expect("a baseline");
        assert_eq!(
            base.reduction_db, 1.0,
            "the drag's first step, not its middle"
        );
        hold.begin(Some((3.0, 0.0)), 1_500.0, Some("band:3"));
        assert_eq!(
            hold.baseline().expect("a new baseline").reduction_db,
            3.0,
            "past the gesture window, the latest reading is the baseline"
        );
        // A different control is always a new gesture.
        hold.begin(Some((7.0, 0.0)), 1_600.0, Some("bass"));
        assert_eq!(hold.baseline().expect("a baseline").reduction_db, 7.0);
    }

    /// The button's number: half a decibel, never below 0.5, so a press always
    /// does something; and the gain it asks the player for.
    #[test]
    fn the_recommended_cut_and_its_gain() {
        assert_eq!(volume_cut_db(0.0), 0.5);
        assert_eq!(volume_cut_db(0.9), 1.0);
        assert_eq!(volume_cut_db(6.0), 6.0);
        assert_eq!(volume_cut_db(6.2), 6.0);
        assert_eq!(volume_cut_db(13.2), 13.0);
        assert!((db_to_gain(0.0) - 1.0).abs() < 1e-12);
        assert!((db_to_gain(6.0) - 0.501_187).abs() < 1e-6);
        // A negative "cut" is not an amplification.
        assert!((db_to_gain(-3.0) - 1.0).abs() < 1e-12);
    }

    /// The readout is French: a real minus sign, a comma, one decimal, and no
    /// `−0,0`.
    #[test]
    fn the_readout_is_french_and_never_says_minus_zero() {
        assert_eq!(fmt_db(-12.04, false), "\u{2212}12,0 dB");
        assert_eq!(fmt_db(0.0, true), "0,0 dB");
        assert_eq!(fmt_db(6.0, true), "+6,0 dB");
        assert_eq!(fmt_db(-0.02, true), "0,0 dB");
        assert_eq!(fmt_db(-96.0, false), "\u{2212}96,0 dB");
    }

    /// An unknown tone is `Ok`: a bug in a reading must not paint the screen red.
    #[test]
    fn an_unknown_tone_is_safe() {
        assert_eq!(Tone::parse("warn"), Tone::Warn);
        assert_eq!(Tone::parse("clip"), Tone::Clip);
        assert_eq!(Tone::parse("explode"), Tone::Ok);
        assert_eq!(Tone::parse(""), Tone::Ok);
    }
}
