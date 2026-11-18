//! The quality the person asked for, and the ladder that steps down under a
//! slow link.
//!
//! Owned by **MB3b**. Two ideas, and the note pins both
//! (`docs/agents/desktop.md`, *Streams*; `docs/agents/mobile.md`, *MB3b*):
//!
//! - **The setting** is one of five: `Auto`, Opus 320, Opus 128, Opus 64, FLAC.
//!   `Auto` is the default, and it is the ONLY one that moves.
//! - **The ladder** starts at 320 and steps down per measured stream
//!   throughput — an EWMA over the last three streams: **≥ 600 kB/s → 320,
//!   ≥ 200 → 128, below → 64** — and a metered network caps it at
//!   [`AudioSettings::metered_cap`] (the phone pays for the bytes).
//!
//! The measurement is the bytes a stream actually delivered over the time it
//! took, taken when the body ends ([`crate::player::source::Metered`]). A
//! stream cut short by a skip still counts if it delivered at least
//! [`MIN_SAMPLE_BYTES`]: a person who skips every 15 s would otherwise never
//! teach the ladder anything, and the ladder is exactly for them.
//!
//! Why an EWMA and not "the last stream": one stream on a lift or in a tunnel
//! is not the link. Why three and not ten: a track is two to four minutes, so
//! three streams is already several minutes of evidence, and a link that
//! recovers must be allowed to step back UP within a couple of tracks (the
//! ladder is symmetric — 320 is restored as soon as the window says so).

use serde::{Deserialize, Serialize};

/// The quality the person asked for. `Auto` follows the measured ladder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Quality {
    #[default]
    Auto,
    Opus320,
    Opus128,
    Opus64,
    Flac,
}

impl Quality {
    /// What the server is asked for, before the ladder: `Auto` starts at 320.
    pub fn start(self) -> nz_audio::Quality {
        match self {
            Quality::Auto | Quality::Opus320 => nz_audio::Quality::Opus320,
            Quality::Opus128 => nz_audio::Quality::Opus128,
            Quality::Opus64 => nz_audio::Quality::Opus64,
            Quality::Flac => nz_audio::Quality::Flac,
        }
    }

    /// The badge's own text (`"auto"`, `"320"`…), which is also what the kv
    /// keeps.
    pub fn as_str(self) -> &'static str {
        match self {
            Quality::Auto => "auto",
            Quality::Opus320 => "320",
            Quality::Opus128 => "128",
            Quality::Opus64 => "64",
            Quality::Flac => "flac",
        }
    }

    pub fn parse(value: &str) -> Quality {
        match value.trim().to_ascii_lowercase().as_str() {
            "320" | "opus_320" => Quality::Opus320,
            "128" | "opus_128" => Quality::Opus128,
            "64" | "opus_64" => Quality::Opus64,
            "flac" | "lossless" => Quality::Flac,
            _ => Quality::Auto,
        }
    }

    /// Where this rung sits, so a ceiling can be applied without a match per
    /// pair: 0 is the smallest body (64), 3 the largest (FLAC).
    fn rung(self) -> u8 {
        match self {
            Quality::Opus64 => 0,
            Quality::Opus128 => 1,
            Quality::Opus320 => 2,
            Quality::Flac => 3,
            // `Auto` has no rung of its own: it is resolved first.
            Quality::Auto => 2,
        }
    }

    fn from_rung(rung: u8) -> Quality {
        match rung {
            0 => Quality::Opus64,
            1 => Quality::Opus128,
            3 => Quality::Flac,
            _ => Quality::Opus320,
        }
    }

    /// The lower of the two, so a metered ceiling can only ever lower the ask.
    pub fn lower(self, ceiling: Quality) -> Quality {
        if ceiling == Quality::Auto {
            return self;
        }
        Self::from_rung(self.rung().min(ceiling.rung()))
    }
}

/// How loudness is normalised (ReplayGain). The four levels are
/// `nz_audio::dsp`'s, which are `graph.js`'s own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Normalization {
    #[default]
    Off,
    Low,
    Medium,
    High,
}

impl Normalization {
    pub fn as_str(self) -> &'static str {
        match self {
            Normalization::Off => "off",
            Normalization::Low => "low",
            Normalization::Medium => "medium",
            Normalization::High => "high",
        }
    }

    pub fn parse(value: &str) -> Normalization {
        match value {
            "low" => Normalization::Low,
            "medium" => Normalization::Medium,
            "high" => Normalization::High,
            _ => Normalization::Off,
        }
    }

    pub fn audio(self) -> nz_audio::Normalization {
        match self {
            Normalization::Off => nz_audio::Normalization::Off,
            Normalization::Low => nz_audio::Normalization::Low,
            Normalization::Medium => nz_audio::Normalization::Medium,
            Normalization::High => nz_audio::Normalization::High,
        }
    }
}

/// Everything the audio chain is asked for (desktop D6's set, on the phone).
/// Persisted whole in the settings store under [`AudioSettings::KEY`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioSettings {
    pub quality: Quality,
    /// Seconds of overlap between two tracks. 0 is gapless, not "no handover".
    pub crossfade_s: f32,
    pub trim_silence: bool,
    pub normalization: Normalization,
    /// The ten peaking bands, in dB (webapp's `AudioEffects`).
    pub eq_db: Vec<f32>,
    pub eq_preset: Option<String>,
    pub bass_db: f32,
    pub mono: bool,
    pub balance: f32,
    pub speed: f32,
    /// The quality ceiling on a metered network.
    pub metered_cap: Quality,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            quality: Quality::Auto,
            crossfade_s: 0.0,
            trim_silence: false,
            normalization: Normalization::Off,
            eq_db: vec![0.0; BANDS],
            eq_preset: None,
            bass_db: 0.0,
            mono: false,
            balance: 0.0,
            speed: 1.0,
            metered_cap: Quality::Opus128,
        }
    }
}

/// The ten bands `AudioEffects` draws and `nz_audio`'s chain has.
pub const BANDS: usize = 10;

/// The slowest link that still earns 320: 600 kB/s, the note's number.
pub const RUNG_320_BYTES_PER_S: f64 = 600_000.0;
/// Below this, 64: 200 kB/s.
pub const RUNG_128_BYTES_PER_S: f64 = 200_000.0;
/// A stream shorter than this is not a measurement (a skip, a failed open).
pub const MIN_SAMPLE_BYTES: u64 = 64 * 1024;
/// The window the EWMA weights: the note's "last 3 streams".
pub const WINDOW: usize = 3;

impl AudioSettings {
    /// The key this whole value lives under in `nz-cache`'s `kv`.
    pub const KEY: &'static str = "player.audio";

    /// The quality the engine is asked for: the setting, stepped down by the
    /// ladder when it is `Auto`, and never above the metered ceiling.
    pub fn effective(&self, rate: Option<f64>, metered: bool) -> nz_audio::Quality {
        let asked = if self.quality == Quality::Auto {
            rung_of(rate)
        } else {
            self.quality
        };
        let capped = if metered {
            asked.lower(self.metered_cap)
        } else {
            asked
        };
        capped.start()
    }

    /// Clamps what a caller sent: the chain has ten bands, a speed between 0.5
    /// and 3, a balance in −1…1, a crossfade that cannot outlast a track.
    /// A value out of range is CLAMPED rather than refused — the caller is a
    /// slider, and the sound must not stop because one went past its end.
    pub fn clamped(mut self) -> AudioSettings {
        self.eq_db.resize(BANDS, 0.0);
        for band in &mut self.eq_db {
            // ±16, which is what the chain can do and what the card draws:
            // `graph.js#EQ_MIN_DB/EQ_MAX_DB` and `nscore::dsp::EQ_MIN_DB/
            // EQ_MAX_DB` both say 16 (MB12 found this clamp discarding a +16
            // fader down to +12 while the DSP would have taken all of it).
            *band = band.clamp(-16.0, 16.0);
        }
        self.bass_db = self.bass_db.clamp(0.0, 15.0);
        self.balance = self.balance.clamp(-1.0, 1.0);
        self.crossfade_s = self.crossfade_s.clamp(0.0, MAX_CROSSFADE_S);
        self.speed = nz_audio::clamp_speed(self.speed);
        self
    }
}

/// A crossfade longer than this is not a crossfade, it is two songs at once.
pub const MAX_CROSSFADE_S: f32 = 12.0;

/// The rung a measured rate earns — the one function the ladder and the player's
/// snapshot share, so the two can never disagree.
pub fn rung_of(bytes_per_s: Option<f64>) -> Quality {
    match bytes_per_s {
        Some(rate) if rate >= RUNG_320_BYTES_PER_S => Quality::Opus320,
        Some(rate) if rate >= RUNG_128_BYTES_PER_S => Quality::Opus128,
        Some(_) => Quality::Opus64,
        None => Quality::Opus320,
    }
}

/// The measured throughput of the last few streams, and the rung it earns.
#[derive(Debug, Default)]
pub struct Ladder {
    samples: [Option<f64>; WINDOW],
    at: usize,
}

impl Ladder {
    pub fn new() -> Ladder {
        Ladder::default()
    }

    /// One finished stream: `bytes` delivered in `seconds`.
    ///
    /// A sample under [`MIN_SAMPLE_BYTES`] is ignored (a skip is not a link),
    /// and so is a nonsensical duration. Answers the rung the window now says,
    /// so a caller can notice a step in the same call.
    pub fn observe(&mut self, bytes: u64, seconds: f64) -> Quality {
        if bytes >= MIN_SAMPLE_BYTES && seconds > 0.0 && seconds.is_finite() {
            self.samples[self.at] = Some(bytes as f64 / seconds);
            self.at = (self.at + 1) % WINDOW;
        }
        self.rung()
    }

    /// The rate the window says, in bytes per second, or `None` before the
    /// first measurement — which is what makes Auto start at 320 instead of
    /// guessing 64.
    pub fn bytes_per_s(&self) -> Option<f64> {
        let mut total = 0.0;
        let mut count = 0usize;
        for sample in self.samples.iter().flatten() {
            total += sample;
            count += 1;
        }
        (count > 0).then(|| total / count as f64)
    }

    /// The rung the window earns. No measurement yet is 320, as the note says.
    pub fn rung(&self) -> Quality {
        rung_of(self.bytes_per_s())
    }

    /// How many measurements the window holds (a test reads it).
    pub fn len(&self) -> usize {
        self.samples
            .iter()
            .filter(|sample| sample.is_some())
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The note's three thresholds, at their own numbers and one byte either
    /// side of each: 600 000 B/s earns 320, 599 999 does not; 200 000 earns
    /// 128, 199 999 falls to 64.
    #[test]
    fn the_ladder_steps_at_the_notes_own_thresholds() {
        let mut ladder = Ladder::new();
        assert_eq!(
            ladder.rung(),
            Quality::Opus320,
            "no measurement starts at 320"
        );

        let mut feed = |rate: f64| {
            ladder = Ladder::new();
            for _ in 0..WINDOW {
                ladder.observe((rate * 2.0) as u64, 2.0);
            }
            ladder.rung()
        };
        assert_eq!(feed(600_000.0), Quality::Opus320);
        assert_eq!(feed(599_999.0), Quality::Opus128);
        assert_eq!(feed(200_000.0), Quality::Opus128);
        assert_eq!(feed(199_999.0), Quality::Opus64);
        // 40 kB/s over two seconds is 80 kB: past the 64 KiB a measurement must
        // deliver, so the ladder really sees it.
        assert_eq!(feed(40_000.0), Quality::Opus64);
    }

    /// The window is the last THREE streams: two fast ones and a slow one do
    /// not hold 320, and a recovered link steps back up in the same window —
    /// the ladder is symmetric on purpose.
    #[test]
    fn the_window_is_the_last_three_streams_and_moves_both_ways() {
        let mut ladder = Ladder::new();
        for _ in 0..WINDOW {
            ladder.observe(2_000_000, 2.0); // 1 000 000 B/s
        }
        assert_eq!(ladder.rung(), Quality::Opus320);
        assert_eq!(ladder.len(), WINDOW);

        ladder.observe(200_000, 2.0); // 100 000 B/s
        // 1 000 000 + 1 000 000 + 100 000 over three = 700 kB/s: still 320.
        assert_eq!(ladder.rung(), Quality::Opus320);
        ladder.observe(200_000, 2.0);
        ladder.observe(200_000, 2.0);
        assert_eq!(ladder.rung(), Quality::Opus64, "three slow streams");

        ladder.observe(2_000_000, 2.0);
        ladder.observe(2_000_000, 2.0);
        ladder.observe(2_000_000, 2.0);
        assert_eq!(
            ladder.rung(),
            Quality::Opus320,
            "a recovered link comes back"
        );
    }

    /// A skip is not a link: a sample under 64 KiB and a nonsensical duration
    /// change nothing.
    #[test]
    fn a_skipped_stream_is_not_a_measurement() {
        let mut ladder = Ladder::new();
        ladder.observe(1_000, 0.2);
        ladder.observe(MIN_SAMPLE_BYTES - 1, 3.0);
        ladder.observe(5_000_000, 0.0);
        ladder.observe(5_000_000, f64::NAN);
        assert!(ladder.is_empty());
        assert_eq!(ladder.rung(), Quality::Opus320);
    }

    /// Auto is the ladder, a fixed quality never moves, and the metered ceiling
    /// can only LOWER the ask (a Wi-Fi ceiling never raises a 64).
    #[test]
    fn auto_follows_the_ladder_and_a_ceiling_only_lowers() {
        let mut settings = AudioSettings::default();
        let mut ladder = Ladder::new();
        assert_eq!(
            settings.effective(ladder.bytes_per_s(), false),
            nz_audio::Quality::Opus320
        );
        for _ in 0..WINDOW {
            ladder.observe(100_000, 2.0);
        }
        assert_eq!(
            settings.effective(ladder.bytes_per_s(), false),
            nz_audio::Quality::Opus64
        );
        assert_eq!(
            settings.effective(ladder.bytes_per_s(), true),
            nz_audio::Quality::Opus64,
            "the ceiling never raises"
        );

        settings.quality = Quality::Flac;
        settings.metered_cap = Quality::Opus128;
        assert_eq!(
            settings.effective(ladder.bytes_per_s(), false),
            nz_audio::Quality::Flac
        );
        assert_eq!(
            settings.effective(ladder.bytes_per_s(), true),
            nz_audio::Quality::Opus128
        );

        settings.quality = Quality::Opus64;
        assert_eq!(
            settings.effective(ladder.bytes_per_s(), true),
            nz_audio::Quality::Opus64
        );
    }

    /// A slider that goes past its end is clamped, never refused: the sound
    /// must not stop because a value is out of range.
    #[test]
    fn settings_are_clamped_to_what_the_chain_can_do() {
        let wild = AudioSettings {
            eq_db: vec![40.0; 4],
            bass_db: 99.0,
            balance: -9.0,
            crossfade_s: 60.0,
            speed: 12.0,
            ..AudioSettings::default()
        }
        .clamped();
        assert_eq!(wild.eq_db.len(), BANDS, "ten bands, whatever came in");
        assert!(
            wild.eq_db[..4].iter().all(|band| *band == 16.0),
            "the four that came in are clamped to the chain's ceiling, ±16 dB \
             (`nscore::dsp::EQ_MAX_DB`, and what the card draws)"
        );
        assert!(
            wild.eq_db[4..].iter().all(|band| *band == 0.0),
            "the six the caller did not send are flat, not invented"
        );
        assert_eq!(wild.bass_db, 15.0);
        assert_eq!(wild.balance, -1.0);
        assert_eq!(wild.crossfade_s, MAX_CROSSFADE_S);
        assert!(wild.speed <= nz_audio::MAX_SPEED);
    }

    /// The badge's text round-trips through the settings store.
    #[test]
    fn the_settings_text_round_trips() {
        for quality in [
            Quality::Auto,
            Quality::Opus320,
            Quality::Opus128,
            Quality::Opus64,
            Quality::Flac,
        ] {
            assert_eq!(Quality::parse(quality.as_str()), quality);
        }
        for level in [
            Normalization::Off,
            Normalization::Low,
            Normalization::Medium,
            Normalization::High,
        ] {
            assert_eq!(Normalization::parse(level.as_str()), level);
        }
        assert_eq!(Quality::parse("nonsense"), Quality::Auto);
    }
}
