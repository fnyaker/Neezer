//! Mono and left/right balance — for the AUDIBLE output only (`pan.js`).
//!
//! The maths are the W3C Web Audio spec's, which is what `pan.js` ports: the
//! stereo→mono down-mix `0.5·(L+R)` (§4.5) and the equal-power pair
//! `cos/sin((balance+1)·π/4)` a `StereoPannerNode` applies to a MONO input
//! (§6.3.3) — `gL² + gR² = 1` everywhere, so a signal keeps its power while it
//! moves between the speakers instead of dipping 3 dB in the middle.
//!
//! The device's own chain applies the stage itself (`android/nscore/src/dsp.rs`,
//! `Pan`, after the tap so the analysers keep the original stereo); this module
//! owns the settings' vocabulary — the stored values, their clamps, and the
//! French sentence under the slider — so the screen and the DSP can never
//! disagree about what "centre" means.

/// The spec's stereo→mono coefficient: `mono = MONO_GAIN · (L + R)`.
pub const MONO_GAIN: f64 = 0.5;

/// Half a percent is "centre": below that the slider is at its detent.
pub const CENTRE_EPS: f64 = 0.005;

/// -1 … 1, with anything unreadable landing at the centre (`clampBalance`).
pub fn clamp_balance(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    value.clamp(-1.0, 1.0)
}

/// The spec's equal-power pair for a MONO source panned by `balance`.
pub fn pan_gains(balance: f64) -> (f64, f64) {
    let x = (clamp_balance(balance) + 1.0) * core::f64::consts::PI / 4.0;
    (x.cos(), x.sin())
}

/// The spec's stereo→mono down-mix, one sample: `0.5 · (L + R)`.
pub fn mono_downmix(left: f64, right: f64) -> f64 {
    MONO_GAIN * (left + right)
}

/// A stored value → the mono switch. Unknown means off (`parseMono`).
pub fn parse_mono(value: Option<&str>) -> bool {
    match value {
        None => false,
        Some(raw) => {
            let s = raw.trim().trim_matches('"').to_ascii_lowercase();
            matches!(s.as_str(), "true" | "1" | "on" | "yes")
        }
    }
}

/// A stored value → a clamped balance; malformed or missing means centre.
pub fn parse_balance(value: Option<&str>) -> f64 {
    match value {
        None => 0.0,
        Some(raw) if raw.trim().is_empty() => 0.0,
        Some(raw) => clamp_balance(raw.trim().replace(',', ".").parse::<f64>().unwrap_or(0.0)),
    }
}

/// "Centre", "30 % à gauche", "À droite" — the readout under the slider
/// (`describeBalance`).
pub fn describe_balance(balance: f64) -> String {
    let b = clamp_balance(balance);
    if b.abs() < CENTRE_EPS {
        return "Centre".to_owned();
    }
    if b <= -0.995 {
        return "À gauche".to_owned();
    }
    if b >= 0.995 {
        return "À droite".to_owned();
    }
    let pct = (b.abs() * 100.0).round() as i32;
    format!("{pct} % à {}", if b < 0.0 { "gauche" } else { "droite" })
}

/// The whole stage in one line, for a status sentence (`describePan`).
pub fn describe_pan(mono: bool, balance: f64) -> String {
    format!(
        "{} · {}",
        if mono { "Mono" } else { "Stéréo" },
        describe_balance(balance)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The equal-power law: `gL² + gR² = 1` at every position, 1/√2 per side at
    /// the centre, and hard pans that keep both channels of a STEREO signal
    /// audible (the DSP's stereo branch is not this function's).
    #[test]
    fn the_pan_law_keeps_the_power_constant() {
        for step in -100..=100 {
            let b = f64::from(step) / 100.0;
            let (l, r) = pan_gains(b);
            assert!((l * l + r * r - 1.0).abs() < 1e-12, "power at {b}");
        }
        let (l, r) = pan_gains(0.0);
        assert!((l - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12);
        assert!((r - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12);
        assert!((pan_gains(-1.0).0 - 1.0).abs() < 1e-12, "hard left");
        assert!(pan_gains(-1.0).1.abs() < 1e-12);
        assert!((pan_gains(1.0).1 - 1.0).abs() < 1e-12, "hard right");
    }

    /// The spec's down-mix: a centred signal keeps its amplitude, and an
    /// out-of-phase pair cancels — the honest result of a mono fold.
    #[test]
    fn the_mono_fold_is_the_specs_down_mix() {
        assert!((mono_downmix(0.5, 0.5) - 0.5).abs() < 1e-12);
        assert!(mono_downmix(0.5, -0.5).abs() < 1e-12);
        assert!((mono_downmix(0.4, 0.1) - 0.25).abs() < 1e-12);
    }

    /// Stored values, hostile ones included: an unreadable balance is the
    /// centre, a French comma is understood, and only a real "on" is mono.
    #[test]
    fn stored_values_are_read_defensively() {
        assert_eq!(parse_balance(None), 0.0);
        assert_eq!(parse_balance(Some("")), 0.0);
        assert_eq!(parse_balance(Some("0,35")), 0.35);
        assert_eq!(parse_balance(Some("-2")), -1.0);
        assert_eq!(parse_balance(Some("9")), 1.0);
        assert_eq!(parse_balance(Some("gauche")), 0.0);
        assert!(parse_mono(Some("true")));
        assert!(parse_mono(Some("\"ON\"")));
        assert!(!parse_mono(None));
        assert!(!parse_mono(Some("off")));
        assert!(!parse_mono(Some("no")));
    }

    /// The readout, exactly `pan.js#describeBalance`.
    #[test]
    fn the_readout_names_the_position_in_french() {
        assert_eq!(describe_balance(0.0), "Centre");
        assert_eq!(describe_balance(0.004), "Centre");
        assert_eq!(describe_balance(-1.0), "À gauche");
        assert_eq!(describe_balance(1.0), "À droite");
        assert_eq!(describe_balance(-0.3), "30 % à gauche");
        assert_eq!(describe_balance(0.66), "66 % à droite");
        assert_eq!(describe_pan(false, 0.0), "Stéréo · Centre");
        assert_eq!(describe_pan(true, -0.5), "Mono · 50 % à gauche");
    }
}
