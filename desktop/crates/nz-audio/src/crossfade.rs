//! When two tracks overlap, and how their levels ride through it.
//!
//! The rules are the web player's (`Player.svelte#maybeCrossfade`,
//! `#armCrossfade`, `docs/agents/playback.md`, *Crossfade + silence trimming*),
//! and they fit in five sentences:
//!
//! - **The fade covers the END of the outgoing track**, from
//!   `end - seconds` to `end`, where `end` is the trim end when there is one
//!   ([`crate::trim`]) — otherwise the file's own end.
//! - **The handover happens at the START of the fade**, not its end: the
//!   incoming track becomes the current one (and the queue advances, and the
//!   clock and the media session name it) the moment the two start to blend.
//!   That is why a bar or a lock screen never shows the wrong title.
//! - **Equal power, not equal amplitude.** Two uncorrelated tracks crossfaded
//!   with complementary linear ramps dip by 3 dB in the middle. The pair here is
//!   `cos`/`sin` over a quarter circle, which sums to 1 in POWER at every point
//!   of the fade — the same curve the web and the Android chain sample
//!   (`nscore::dsp`, `equalPowerCurve`).
//! - **No crossfade into itself.** `repeat = one` restarts the track; it does
//!   not blend it with its own beginning (`Player.svelte#repeatsItself`).
//! - **A manual skip is not an overlap.** It gets a short [`SKIP_CUT_SECONDS`]
//!   ramp out of the old track and into the new one: long enough to kill the
//!   click of a cut mid-waveform, short enough to be inaudible as a delay. With
//!   `fade.onSkip` the skip instead blends for `fade.skipSeconds`.
//!
//! `fade.seconds` is clamped to [`MAX_SECONDS`] (the web's `XFADE_MAX`: past it
//! it stops being a crossfade and becomes a mash-up), and `skipSeconds` to
//! [`MAX_SKIP_SECONDS`], exactly as the player's own sliders are.

use core::f64::consts::FRAC_PI_2;

/// Past this a crossfade stops being a crossfade (`Player.svelte#XFADE_MAX`).
pub const MAX_SECONDS: f64 = 12.0;

/// Réglages → Lecture's default, and the lot's contract.
pub const DEFAULT_SECONDS: f64 = 6.0;

/// The SPA's `crossfadeSkipSeconds` is capped at 5 (`skipFadeSeconds`).
pub const MAX_SKIP_SECONDS: f64 = 5.0;

/// Réglages → Lecture's default for a skipped overlap.
pub const DEFAULT_SKIP_SECONDS: f64 = 1.5;

/// A plain skip's ramp: `Player.svelte#SKIP_FADE`, 60 ms.
pub const SKIP_CUT_SECONDS: f64 = 0.06;

/// Réglages → Lecture's crossfade block (`fade.*` in `stores.js`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FadeSettings {
    /// The person turned crossfading on. Off by default, like the Web Audio
    /// graph's effects (`graph.js#effectsOn`).
    pub enabled: bool,
    /// How long the overlap lasts, in seconds.
    pub seconds: f64,
    /// A manual skip blends too, instead of a 60 ms cut.
    pub on_skip: bool,
    /// How long a skipped blend lasts.
    pub skip_seconds: f64,
}

impl Default for FadeSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            seconds: DEFAULT_SECONDS,
            on_skip: false,
            skip_seconds: DEFAULT_SKIP_SECONDS,
        }
    }
}

impl FadeSettings {
    /// The same settings with hostile or stale values made harmless: a NaN, a
    /// negative, or a number a slider could never produce plays as "no fade".
    pub fn sanitized(self) -> Self {
        Self {
            enabled: self.enabled,
            seconds: clamp_seconds(self.seconds, 0.0, MAX_SECONDS),
            on_skip: self.on_skip,
            skip_seconds: clamp_seconds(self.skip_seconds, 0.0, MAX_SKIP_SECONDS),
        }
    }

    /// The length of the overlap a track change gets right now: the full fade
    /// at the end of a track, `skip_seconds` (or the 60 ms cut) for a skip.
    pub fn overlap_seconds(&self, skip: bool) -> f64 {
        let fade = self.sanitized();
        if skip {
            if fade.enabled && fade.on_skip && fade.skip_seconds > 0.0 {
                fade.skip_seconds
            } else {
                SKIP_CUT_SECONDS
            }
        } else if fade.enabled {
            fade.seconds
        } else {
            0.0
        }
    }

    /// Whether an end-of-track overlap is wanted at all (both tracks being
    /// eligible: an episode is never blended into what follows it).
    pub fn overlaps_ends(&self) -> bool {
        let fade = self.sanitized();
        fade.enabled && fade.seconds > 0.0
    }
}

fn clamp_seconds(value: f64, min: f64, max: f64) -> f64 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        min
    }
}

/// The equal-power pair at `progress` of a fade: `(outgoing, incoming)`.
///
/// At 0 the outgoing track is at unity and the incoming silent; at 1 the
/// reverse; in between `out² + in² == 1` exactly, so two uncorrelated tracks
/// keep a constant summed power instead of dipping 3 dB in the middle.
pub fn equal_power(progress: f64) -> (f64, f64) {
    let angle = progress.clamp(0.0, 1.0) * FRAC_PI_2;
    (angle.cos(), angle.sin())
}

/// Whether the fade into the next voice must begin now.
///
/// `remaining_s` is the MEDIA time left in the current track (its trim end or
/// its file's end, minus where it has played to). At a playback speed `speed`
/// the fade's `fade_s` real seconds consume `fade_s · speed` media seconds, so
/// that is the window it has to fit in.
pub fn due(remaining_s: f64, fade_s: f64, speed: f64) -> bool {
    fade_s > 0.0 && remaining_s <= fade_s * speed.max(0.01)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_lot_contract() {
        let fade = FadeSettings::default();
        assert!(
            !fade.enabled,
            "off until the person asks, like the graph's effects"
        );
        assert_eq!(fade.seconds, 6.0);
        assert!(!fade.on_skip);
        assert_eq!(fade.skip_seconds, 1.5);
    }

    /// The sliders' bounds: 12 s is the longest overlap the player will do, and
    /// a skipped blend is capped at 5 s.
    #[test]
    fn hostile_or_stale_lengths_are_clamped() {
        let fade = FadeSettings {
            enabled: true,
            seconds: 60.0,
            on_skip: true,
            skip_seconds: 30.0,
        };
        let fade = fade.sanitized();
        assert_eq!(fade.seconds, MAX_SECONDS);
        assert_eq!(fade.skip_seconds, MAX_SKIP_SECONDS);

        let nan = FadeSettings {
            seconds: f64::NAN,
            skip_seconds: -2.0,
            ..FadeSettings::default()
        }
        .sanitized();
        assert_eq!(nan.seconds, 0.0);
        assert_eq!(nan.skip_seconds, 0.0);
        assert!(!nan.overlaps_ends(), "a zero-length fade is no fade");
        assert_eq!(nan.overlap_seconds(true), SKIP_CUT_SECONDS);
    }

    /// An equal-power fade keeps the POWER constant, not the amplitude: over
    /// 4 800 sampled points of a 6 s fade the summed power is 1.0 to 6e-16, and
    /// the amplitude pair is 0.707 at the middle (`cos` and `sin` of 45°).
    #[test]
    fn the_pair_holds_the_power_constant() {
        let mut worst = 0.0f64;
        for step in 0..=4_800 {
            let (out, inc) = equal_power(step as f64 / 4_800.0);
            worst = worst.max((out * out + inc * inc - 1.0).abs());
        }
        assert!(worst < 1e-12, "worst power error {worst:e}");
        let (out, inc) = equal_power(0.5);
        assert!((out - core::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12);
        assert!((inc - core::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12);
        let ends = equal_power(0.0);
        assert!((ends.0 - 1.0).abs() < 1e-15 && ends.1 == 0.0);
        let ends = equal_power(1.0);
        assert!(ends.0.abs() < 1e-15 && (ends.1 - 1.0).abs() < 1e-15);
        // Out of range input is clamped, never extrapolated.
        assert!((equal_power(-1.0).0 - 1.0).abs() < 1e-15);
        assert!(equal_power(2.0).0.abs() < 1e-15);
    }

    /// The fade starts when the track has `fade · speed` media seconds left: at
    /// 1× a 6 s fade begins at 6 s, at 2× the same six real seconds are twelve
    /// media seconds.
    #[test]
    fn the_window_follows_the_playback_speed() {
        assert!(!due(6.1, 6.0, 1.0));
        assert!(due(6.0, 6.0, 1.0));
        assert!(due(5.9, 6.0, 1.0));
        assert!(!due(12.1, 6.0, 2.0));
        assert!(due(12.0, 6.0, 2.0));
        assert!(!due(5.0, 0.0, 1.0), "no fade configured");
    }

    #[test]
    fn a_skip_is_a_short_cut_unless_the_person_asked_for_an_overlap() {
        let mut fade = FadeSettings::default();
        assert_eq!(fade.overlap_seconds(true), SKIP_CUT_SECONDS);
        fade.on_skip = true;
        assert_eq!(
            fade.overlap_seconds(true),
            SKIP_CUT_SECONDS,
            "on_skip alone does not crossfade: the feature is off"
        );
        fade.enabled = true;
        assert_eq!(fade.overlap_seconds(true), 1.5);
        assert_eq!(fade.overlap_seconds(false), 6.0);
    }
}
