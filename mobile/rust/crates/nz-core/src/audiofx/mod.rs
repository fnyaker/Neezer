//! Audio effects: the equalizer, the bass lift, mono and balance, the output's
//! verdict and the volume warning — the SPA's `AudioEffects.svelte` and the rules
//! behind it, on the phone's own player.
//!
//! Owned by **MB12** (docs/agents/mobile.md, "Module and file ownership").
//!
//! # What is where
//!
//! - The SOUND is `nz-audio`'s chain — the same `nscore::dsp::Chain` the desktop
//!   uses (`docs/agents/playback.md`, D6). Nothing here filters a sample: this
//!   module owns the SETTINGS (what the person chose, persisted), the attribution
//!   (whether the equalizer is the cause of a warning), and the meter's session.
//! - The SETTINGS the player already persists live in `player.audio()`
//!   (`AudioSettings`, MB3b) and are written through it, never around it: one
//!   copy, applied by the player's worker, so a fader reaches the sound without
//!   the screen waiting on the mixer.
//! - What `AudioSettings` does not carry — the equalizer's switch, the curve
//!   while it is off, the crossfade's length while it is off, the skip fade, the
//!   output mode and the verdicts — lives in `kv` under the SPA's own key names,
//!   so a person who migrated from the web app finds their setup where they left
//!   it (rule 7).
//!
//! # The order of a change
//!
//! 1. the new value goes to `kv` (one write, the SPA's key);
//! 2. `player.audio()` is patched and handed to `Player::set_audio`, which applies
//!    it to the engine's atomics at once and persists it on its own worker — the
//!    mixer reads those atomics at the top of the next buffer, which is what
//!    makes "a fader changes the sound within one buffer" true;
//! 3. the screen is answered with the state it should draw, computed from what
//!    was written.

pub mod meter;
pub mod output;
pub mod pan;
pub mod presets;
pub mod risk;

use std::sync::Arc;

use crate::core::Core;
use crate::player::{AudioSettings, Player, TRIM_DB};

pub use meter::{
    BAND_CEIL_DB, BAND_FLOOR_DB, BANDS, CHANGE_HOLD_MS, EQ_FREQS, MeterFrame, MeterSession,
    MeterSink, TICK_MS, Warning,
};
pub use output::{
    Glitch, Mode, Path, Plan, Route, Verdict, Verdicts, Why, kind_label, normalize_route,
};
pub use pan::{describe_balance, describe_pan};
pub use presets::{
    AppliedChain, Builtin, Chain, EQ_BAND_COUNT, EQ_MAX_DB, EQ_MIN_DB, Preset, active_preset_id,
    apply_preset, is_builtin, preset_items, preset_needs_update, save_preset, updated_preset,
};
pub use risk::{Shown, Tone, classify_risk, db_to_gain, eq_risk, fmt_db, volume_cut_db};

/// The device's own keys, the SPA's spelling (`stores.js`, `pan.js`,
/// `output.js`). A key outside this list is never written.
pub const KEY_EQ_ENABLED: &str = "fx.eq.enabled";
pub const KEY_EQ_BANDS: &str = "fx.eq.bands";
pub const KEY_BASS: &str = "fx.bass";
pub const KEY_NORMALIZE: &str = "fx.normalize";
pub const KEY_MONO: &str = "audio.mono";
pub const KEY_BALANCE: &str = "audio.balance";
pub const KEY_FADE_ENABLED: &str = "fade.enabled";
pub const KEY_FADE_SECONDS: &str = "fade.seconds";
pub const KEY_FADE_ON_SKIP: &str = "fade.onSkip";
pub const KEY_FADE_SKIP_SECONDS: &str = "fade.skipSeconds";
pub const KEY_TRIM: &str = "fade.trim";
pub const KEY_OUTPUT: &str = "audio.output";
pub const KEY_PRESETS: &str = "fx.presets";
pub const KEY_VERDICTS: &str = "audio.glitchRoutes";

/// The bass lift's ceiling: the SPA stores 0..1 and the graph asks for
/// `bass * 12` dB of shelf (`graph.js#bassDb`).
pub const BASS_MAX_DB: f32 = 12.0;
/// The crossfade's range in seconds (`AudioEffects.svelte`'s slider: 0..12).
pub const CROSSFADE_MAX_S: f32 = 12.0;
/// …and the skip fade's: 0..5 (`crossfadeSkipSeconds`).
pub const SKIP_FADE_MAX_S: f32 = 5.0;

/// The audio-effects settings, resolved: every field is a real value, the SPA's
/// default when the person never chose one.
#[derive(Debug, Clone, PartialEq)]
pub struct FxSettings {
    /// The equalizer's switch (`fx.eq.enabled`, default off).
    pub eq_enabled: bool,
    /// The ten band gains in dB, ±16 (`fx.eq.bands`), kept while the switch is
    /// off — that is what makes the switch reversible.
    pub bands: Vec<f32>,
    /// The bass lift, 0..1 (`fx.bass`).
    pub bass: f32,
    /// `off` | `low` | `medium` | `high` (`fx.normalize`).
    pub normalization: String,
    /// The mono fold (`audio.mono`).
    pub mono: bool,
    /// −1 … 1 (`audio.balance`).
    pub balance: f32,
    /// `fade.enabled`.
    pub crossfade_enabled: bool,
    /// The overlap in seconds, remembered while the switch is off
    /// (`fade.seconds`, default 6).
    pub crossfade_s: f32,
    /// `fade.onSkip`.
    pub crossfade_on_skip: bool,
    /// The manual skip's overlap, 0..5 (`fade.skipSeconds`, default 1.5).
    pub crossfade_skip_s: f32,
    /// `fade.trim` (the SPA's default is ON).
    pub trim: bool,
    /// `auto` | `direct` | `graph` (`audio.output`).
    pub output: String,
}

impl Default for FxSettings {
    fn default() -> Self {
        FxSettings {
            eq_enabled: false,
            bands: vec![0.0; BANDS],
            bass: 0.0,
            normalization: "off".to_owned(),
            mono: false,
            balance: 0.0,
            crossfade_enabled: false,
            crossfade_s: 6.0,
            crossfade_on_skip: true,
            crossfade_skip_s: 1.5,
            trim: true,
            output: "auto".to_owned(),
        }
    }
}

impl FxSettings {
    /// Every value in range, whatever a stored row or a caller said.
    pub fn clamped(mut self) -> FxSettings {
        if self.bands.len() != BANDS {
            self.bands = vec![0.0; BANDS];
        }
        for band in self.bands.iter_mut() {
            *band = if band.is_finite() {
                band.clamp(EQ_MIN_DB, EQ_MAX_DB)
            } else {
                0.0
            };
        }
        self.bass = if self.bass.is_finite() {
            self.bass.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.balance = pan::clamp_balance(f64::from(self.balance)) as f32;
        self.crossfade_s = if self.crossfade_s.is_finite() {
            self.crossfade_s.clamp(0.0, CROSSFADE_MAX_S)
        } else {
            0.0
        };
        self.crossfade_skip_s = if self.crossfade_skip_s.is_finite() {
            self.crossfade_skip_s.clamp(0.0, SKIP_FADE_MAX_S)
        } else {
            0.0
        };
        if !matches!(
            self.normalization.as_str(),
            "off" | "low" | "medium" | "high"
        ) {
            self.normalization = "off".to_owned();
        }
        if !matches!(self.output.as_str(), "auto" | "direct" | "graph") {
            self.output = "auto".to_owned();
        }
        self
    }

    /// The chain a preset is compared to (`eqpresets.js#chain`).
    pub fn chain(&self) -> Chain<'_> {
        Chain {
            eq: self.eq_enabled,
            bands: &self.bands,
            bass: self.bass,
            norm: &self.normalization,
        }
    }

    /// The ten bands the ENGINE is given: the curve while the equalizer is on,
    /// silence (a flat, bypassed chain) while it is off.
    pub fn effective_bands(&self) -> Vec<f32> {
        if self.eq_enabled {
            self.bands.clone()
        } else {
            vec![0.0; BANDS]
        }
    }

    /// Patch what the player persists from these settings, keeping the fields
    /// this card does not own (the quality, the podcast speed, the metered cap).
    pub fn to_audio(&self, current: &AudioSettings) -> AudioSettings {
        AudioSettings {
            eq_db: self.effective_bands(),
            eq_preset: None,
            bass_db: self.bass * BASS_MAX_DB,
            mono: self.mono,
            balance: self.balance,
            normalization: current_normalization(&self.normalization),
            crossfade_s: if self.crossfade_enabled {
                self.crossfade_s
            } else {
                0.0
            },
            trim_silence: self.trim,
            ..current.clone()
        }
    }
}

/// `fx.normalize` → the player's own level.
fn current_normalization(id: &str) -> crate::player::Normalization {
    match id {
        "low" => crate::player::Normalization::Low,
        "medium" => crate::player::Normalization::Medium,
        "high" => crate::player::Normalization::High,
        _ => crate::player::Normalization::Off,
    }
}

fn normalization_id(level: crate::player::Normalization) -> &'static str {
    match level {
        crate::player::Normalization::Low => "low",
        crate::player::Normalization::Medium => "medium",
        crate::player::Normalization::High => "high",
        crate::player::Normalization::Off => "off",
    }
}

/// The state the card draws, all in one record: what it costs one read.
#[derive(Debug, Clone, PartialEq)]
pub struct FxState {
    pub settings: FxSettings,
    /// The presets in the card's order (the five built-ins, then the person's).
    pub presets: Vec<Preset>,
    /// The preset the chain matches, if any.
    pub active_preset: Option<String>,
    /// The route playing now, and what the app decided to do with it.
    pub route: Route,
    pub plan: Plan,
    /// The stored verdicts still on record, newest first.
    pub verdicts: Vec<Verdict>,
    /// The trim threshold the SERVER measures at, dBFS (`TRIM_DB`): the switch
    /// above is this card's, the threshold is the server's measurement.
    pub trim_db: i32,
}

/// The audio-effects service, over the core and the ONE player.
///
/// Built per call (it holds two `Arc`s and no state); the meter's thread lives in
/// [`AudioFx::start_meter`]'s session, which the caller keeps — the app keeps one
/// per process.
pub struct AudioFx {
    core: Arc<Core>,
    player: Arc<Player>,
}

impl AudioFx {
    pub fn new(core: Arc<Core>, player: Arc<Player>) -> AudioFx {
        AudioFx { core, player }
    }

    pub fn core(&self) -> &Arc<Core> {
        &self.core
    }

    pub fn player(&self) -> &Arc<Player> {
        &self.player
    }

    /// This device's settings: `kv` first, then the fields the player persists
    /// (the quality, the speed) where the card does not own them.
    pub fn settings(&self) -> FxSettings {
        let kv = self.core.cache().kv();
        let audio = self.player.audio();
        let bands: Vec<f32> = kv
            .get::<Vec<f32>>(KEY_EQ_BANDS)
            .unwrap_or_else(|| vec![0.0; BANDS]);
        let stored_enabled = kv.get::<bool>(KEY_EQ_ENABLED).unwrap_or(false);
        FxSettings {
            eq_enabled: stored_enabled,
            bands,
            bass: kv.get::<f64>(KEY_BASS).unwrap_or(0.0) as f32,
            normalization: kv
                .get::<String>(KEY_NORMALIZE)
                .unwrap_or_else(|| normalization_id(audio.normalization).to_owned()),
            mono: kv.get::<bool>(KEY_MONO).unwrap_or(audio.mono),
            balance: kv.get::<f64>(KEY_BALANCE).unwrap_or(audio.balance as f64) as f32,
            crossfade_enabled: kv
                .get::<bool>(KEY_FADE_ENABLED)
                .unwrap_or(audio.crossfade_s > 0.0),
            crossfade_s: kv.get::<f64>(KEY_FADE_SECONDS).map(|v| v as f32).unwrap_or(
                if audio.crossfade_s > 0.0 {
                    audio.crossfade_s
                } else {
                    6.0
                },
            ),
            crossfade_on_skip: kv.get::<bool>(KEY_FADE_ON_SKIP).unwrap_or(true),
            crossfade_skip_s: kv.get::<f64>(KEY_FADE_SKIP_SECONDS).unwrap_or(1.5) as f32,
            // The SPA's own default is ON (`stores.js#trimSilence`), so a device
            // that never chose reads ON, not the player's own quieter default.
            trim: kv.get::<bool>(KEY_TRIM).unwrap_or(true),
            output: kv
                .get::<String>(KEY_OUTPUT)
                .unwrap_or_else(|| "auto".to_owned()),
        }
        .clamped()
    }

    /// Write one setting and apply it. The screen answers from what was written.
    pub fn save(&self, settings: FxSettings) -> FxSettings {
        let settings = settings.clamped();
        let kv = self.core.cache().kv();
        // One write per key, the SPA's own names: a failure here is the device's
        // storage, and the value still applies for this session (the SPA's own
        // rule for a private-mode localStorage).
        let _ = kv.set(KEY_EQ_ENABLED, &settings.eq_enabled);
        let _ = kv.set(KEY_EQ_BANDS, &settings.bands);
        let _ = kv.set(KEY_BASS, &f64::from(settings.bass));
        let _ = kv.set(KEY_NORMALIZE, &settings.normalization);
        let _ = kv.set(KEY_MONO, &settings.mono);
        let _ = kv.set(KEY_BALANCE, &f64::from(settings.balance));
        let _ = kv.set(KEY_FADE_ENABLED, &settings.crossfade_enabled);
        let _ = kv.set(KEY_FADE_SECONDS, &f64::from(settings.crossfade_s));
        let _ = kv.set(KEY_FADE_ON_SKIP, &settings.crossfade_on_skip);
        let _ = kv.set(KEY_FADE_SKIP_SECONDS, &f64::from(settings.crossfade_skip_s));
        let _ = kv.set(KEY_TRIM, &settings.trim);
        let _ = kv.set(KEY_OUTPUT, &settings.output);
        // The player applies it to the engine's atomics at once and persists it
        // on its own worker: the next buffer carries the new curve.
        let audio = settings.to_audio(&self.player.audio());
        self.player.set_audio(audio);
        settings
    }

    /// The whole card's state, including the presets and the output's verdict.
    pub fn state(&self) -> FxState {
        let settings = self.settings();
        let custom = self.custom_presets();
        let items = preset_items(&custom);
        let active = active_preset_id(&items, settings.chain());
        let route = self.route();
        let verdicts = self.verdicts();
        let plan = output::decide_output(
            true,
            Mode::parse(&settings.output),
            &route,
            &verdicts,
            now_ms(),
        );
        FxState {
            settings,
            presets: items,
            active_preset: active,
            route,
            plan,
            verdicts: verdicts
                .active(now_ms())
                .into_iter()
                .map(|(_, v)| v)
                .collect(),
            trim_db: TRIM_DB as i32,
        }
    }

    /// This device's own presets, in the card's order.
    pub fn custom_presets(&self) -> Vec<Preset> {
        let stored = self.core.cache().kv().get::<String>(KEY_PRESETS);
        presets::parse_presets(stored.as_deref())
    }

    pub fn write_presets(&self, custom: &[Preset]) {
        let _ = self
            .core
            .cache()
            .kv()
            .set(KEY_PRESETS, &presets::presets_json(custom));
    }

    /// "Enregistrer comme…": the whole chain, under a name.
    pub fn save_preset(&self, name: &str) -> Option<Preset> {
        let mut custom = self.custom_presets();
        let settings = self.settings();
        let id = format!("p{}", now_ms() as u64);
        let saved = {
            let chain = settings.chain();
            presets::save_preset(&mut custom, name, chain, &id)
        }?;
        self.write_presets(&custom);
        // Selecting the stored row: a re-save keeps the stored id, not the one
        // that was just proposed.
        Some(
            custom
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case(&saved.name))
                .cloned()
                .unwrap_or(saved),
        )
    }

    /// "Mettre à jour": the curve and the lift into the person's own preset.
    pub fn update_preset(&self, id: &str) -> Option<Preset> {
        let mut custom = self.custom_presets();
        let current = self.settings();
        let chain = current.chain();
        let stored = custom.iter().find(|p| p.id == id)?.clone();
        let next = updated_preset(&stored, chain)?;
        let index = custom.iter().position(|p| p.id == id)?;
        custom[index] = next.clone();
        self.write_presets(&custom);
        Some(next)
    }

    /// "Supprimer": forget one of the person's own.
    pub fn delete_preset(&self, id: &str) -> bool {
        let mut custom = self.custom_presets();
        let before = custom.len();
        custom.retain(|p| p.id != id);
        let removed = custom.len() != before;
        if removed {
            self.write_presets(&custom);
        }
        removed
    }

    /// Apply a preset: its curve, its switch, its lift and its level — then the
    /// settings are written like any other change.
    pub fn apply_preset(&self, id: &str) -> Option<FxSettings> {
        let custom = self.custom_presets();
        let items = preset_items(&custom);
        let preset = items.iter().find(|p| p.id == id)?.clone();
        let mut settings = self.settings();
        let applied: AppliedChain = apply_preset(&preset, &settings.bands);
        settings.eq_enabled = applied.eq;
        settings.bands = applied.bands;
        settings.bass = applied.bass;
        settings.normalization = applied.norm;
        Some(self.save(settings))
    }

    /// The route the platform reported (`Player::set_route` wrote it).
    pub fn route(&self) -> Route {
        let route = self.player.route();
        let mut normalized = normalize_route(&route.kind, &route.name, &route.conn, "native");
        if route.kind.is_empty() {
            normalized = Route::default();
        }
        normalized
    }

    /// The output's mode, as stored.
    pub fn mode(&self) -> Mode {
        Mode::parse(&self.settings().output)
    }

    pub fn verdicts(&self) -> Verdicts {
        let stored = self.core.cache().kv().get::<String>(KEY_VERDICTS);
        output::parse_verdicts(stored.as_deref())
    }

    pub fn write_verdicts(&self, verdicts: &Verdicts) {
        let _ = self
            .core
            .cache()
            .kv()
            .set(KEY_VERDICTS, &output::verdicts_json(verdicts));
    }

    /// A fault measured on the output playing now. Answers whether that changes
    /// what plays (only in `auto`).
    pub fn report_glitch(&self, why: String, ratio: Option<f64>) -> bool {
        let mut verdicts = self.verdicts();
        let route = self.route();
        let changed = output::report_glitch(
            &mut verdicts,
            &route,
            Glitch { why, ratio },
            now_ms(),
            self.mode(),
        );
        self.write_verdicts(&verdicts);
        changed
    }

    /// At a track change: retry the processed path if the verdict is due.
    pub fn retry_if_due(&self) -> bool {
        let mut verdicts = self.verdicts();
        let route = self.route();
        let retried = output::retry_if_due(&mut verdicts, &route, now_ms(), self.mode());
        if retried {
            self.write_verdicts(&verdicts);
        }
        retried
    }

    /// "Réessayer maintenant" on the card.
    pub fn retry_now(&self) {
        let mut verdicts = self.verdicts();
        let route = self.route();
        output::retry_now(&mut verdicts, &route.key, now_ms());
        self.write_verdicts(&verdicts);
    }

    /// Forget one output's verdict.
    pub fn forget_glitch(&self, key: &str) {
        let mut verdicts = self.verdicts();
        output::forget(&mut verdicts, key, now_ms());
        self.write_verdicts(&verdicts);
    }

    /// Start the meter: the card's own tap subscription, its own thread. The
    /// caller keeps the session and stops it when the card leaves.
    pub fn start_meter(&self, sink: Arc<dyn MeterSink>) -> Option<MeterSession> {
        let tap = self.player.subscribe_tap()?;
        self.player.set_meter(true);
        let player = Arc::clone(&self.player);
        let chain: meter::ChainReading = Arc::new(move || {
            let meter = player.chain_meter();
            (meter.peak_db, meter.reduction_db)
        });
        let holder = SettingsSnapshot::new(Arc::clone(&self.core));
        let curve: meter::Curve = Arc::new(move || holder.eq_curve());
        Some(MeterSession::start(tap, chain, curve, sink))
    }
}

/// The card's equalizer switch and curve, read on the meter's thread without
/// touching the player's lock: one `kv` read per frame, and the curve from the
/// same store the card writes.
struct SettingsSnapshot {
    core: Arc<Core>,
}

impl SettingsSnapshot {
    fn new(core: Arc<Core>) -> SettingsSnapshot {
        SettingsSnapshot { core }
    }

    fn eq_curve(&self) -> (bool, Vec<f32>) {
        let kv = self.core.cache().kv();
        (
            kv.get::<bool>(KEY_EQ_ENABLED).unwrap_or(false),
            kv.get::<Vec<f32>>(KEY_EQ_BANDS)
                .unwrap_or_else(|| vec![0.0; BANDS]),
        )
    }
}

/// Milliseconds since the epoch: the same clock the SPA's `Date.now()` uses, so a
/// verdict stored by the web app and one stored here compare.
pub fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests;
