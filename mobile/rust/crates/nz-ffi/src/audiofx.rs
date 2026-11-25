//! Audio effects: the equalizer, the bass lift, mono and balance, the output's
//! verdict, the volume warning — and the meter that feeds it (lot MB12).
//!
//! Owned by **MB12** (docs/agents/mobile.md, "Module and file ownership"): the
//! lot fills this file — its own `#[uniffi::export] impl` blocks, its own
//! records, its own listener — and edits no other file of the crate.
//!
//! # Why the methods live on `PlayerApi`
//!
//! Every control here changes what the ONE player produces: the chain is the
//! engine's, the meter is the engine's, the route is the engine's. UniFFI merges
//! several `#[uniffi::export] impl PlayerApi` blocks written in different files
//! into the one Kotlin class (the mechanism that already lets each lot add its
//! own `impl Screens`), so the screen holds this object anyway and no new
//! accessor had to be invented. `PlayerApi::player()` and `core_ref()` are the
//! crate-visible seams that make it possible — two lines, declared in the PR.
//!
//! # One meter per process
//!
//! The meter's thread belongs to the equalizer CARD: `fx_meter_subscribe` starts
//! it (25 Hz, its own analysis subscription), `fx_meter_unsubscribe` stops AND
//! JOINS it, and `fx_meter_stats()` answers how many readings it took — the
//! number "0 analysis CPU quand la carte disparaît" is held to. Subscribing while
//! one runs replaces it: the card is one screen, and two meters would be two
//! threads for one reading.
//!
//! # Nothing here is a request
//!
//! No method of this file touches the network: the settings are this device's
//! (`kv`, through `AudioSettings`), the presets too, and the meter is the engine's
//! own sound. A card that opens costs **0 requests and 0 bytes**, cold or warm —
//! what it costs is two indexed `SELECT`s and one spawn.

use std::sync::{Arc, Mutex};

use nz_core::audiofx::{
    self, AudioFx, MeterFrame, MeterSink, Mode, Path, Preset, Tone, Why, describe_balance,
};

use crate::player::PlayerApi;

/// Kotlin receives a frame on Rust's own thread and hands it to a Flow at once.
#[uniffi::export(callback_interface)]
pub trait AudioFxListener: Send + Sync {
    /// One reading, at 25 Hz while the card is on screen, and never after it
    /// leaves.
    fn on_meter(&self, frame: FxMeterFrame);
}

/// One reading, as the card draws it (`saturation.js`'s snapshot).
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct FxMeterFrame {
    /// False when there is nothing to measure: `reason` says why.
    pub available: bool,
    pub measured: bool,
    /// The pre-limiter sample peak of the last buffer, dBFS, floored at −96.
    pub peak_db: f64,
    pub headroom_db: f64,
    /// The limiter's gain reduction, dB, `>= 0`.
    pub reduction_db: f64,
    /// `ok` | `warn` | `clip`.
    pub risk: String,
    /// The ten bands, 0..1 over −60..0 dBFS.
    pub bands: Vec<f32>,
    /// The same in dBFS.
    pub bands_db: Vec<f32>,
    /// The banner, if an episode is open.
    pub warning: Option<FxWarning>,
    /// `idle` or `direct` when `available` is false, empty otherwise.
    pub reason: String,
}

/// The warning banner: what the equalizer is doing to the sound, and what the
/// card offers to do about it.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct FxWarning {
    /// `warn` | `clip`.
    pub tone: String,
    /// The reduction above the episode's baseline, dB.
    pub reduction_db: f64,
    /// What "Réduire le volume" should ask for, dB.
    pub cut_db: f64,
    /// Measured since the person's own change, not since the track started.
    pub from_change: bool,
}

/// The meter's counters, for the acceptance: a card that leaves takes no more
/// readings.
#[derive(uniffi::Record, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FxMeterStats {
    pub ticks: u64,
    pub frames: u64,
    pub running: bool,
}

/// One preset chip: a shipped curve or one of the person's own.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct FxPreset {
    pub id: String,
    pub name: String,
    /// True for a shipped curve: never writable, never deletable.
    pub builtin: bool,
    /// The heights of the curve at a glance (the chip's sparkline, 0..1).
    pub spark: Vec<f32>,
}

/// A verdict still on record, for the card's "Réessayer" block.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct FxVerdict {
    pub key: String,
    pub kind: String,
    pub name: String,
    pub at_ms: f64,
    pub retry_at_ms: f64,
    pub probing: bool,
    pub count: u32,
}

/// The whole card's state, in one record: one read, no request.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct FxState {
    pub eq_enabled: bool,
    /// The ten band gains in dB, kept while the switch is off.
    pub bands: Vec<f32>,
    /// 0..1.
    pub bass: f32,
    /// `off` | `low` | `medium` | `high`.
    pub normalization: String,
    pub mono: bool,
    /// −1 … 1.
    pub balance: f32,
    /// « Stéréo · Centre » (`pan.js#describePan`).
    pub pan_text: String,
    pub crossfade_enabled: bool,
    pub crossfade_s: f32,
    pub crossfade_on_skip: bool,
    pub crossfade_skip_s: f32,
    pub trim: bool,
    /// The threshold the SERVER measures at, dBFS: the switch is this card's, the
    /// threshold is not a device setting.
    pub trim_db: i32,
    /// `auto` | `direct` | `graph`.
    pub output: String,
    /// The output playing now, in words (« Bluetooth · KMM-BT309 »), or empty.
    pub route_text: String,
    /// « Lecture traitée » / « Lecture directe » / « Lecteur Android ».
    pub path_text: String,
    /// Why it is that path (« choisie ici », « le son se hachait… »), or the
    /// route's own words when there is no other reason.
    pub why_text: String,
    /// When the processed path is due for another try, ms since the epoch.
    pub retry_at_ms: Option<f64>,
    /// The presets in the card's order: the five built-ins, then the person's.
    pub presets: Vec<FxPreset>,
    /// The preset the chain matches, if any.
    pub active_preset: Option<String>,
    /// The verdicts still on record, newest first.
    pub verdicts: Vec<FxVerdict>,
}

/// The meter's slot: one session per process, replaced rather than doubled.
/// There is one equalizer card in the app and one player behind it, so a second
/// subscription (a new card instance, a test's own core) replaces the first —
/// two threads for one reading would be a bug, not a feature.
struct MeterSlot {
    session: nz_core::audiofx::MeterSession,
}

static METER: Mutex<Option<MeterSlot>> = Mutex::new(None);

/// The listener adapter: a Kotlin object behind the core's sink trait.
struct ListenerSink(Arc<dyn AudioFxListener>);

impl MeterSink for ListenerSink {
    fn on_frame(&self, frame: MeterFrame) {
        self.0.on_meter(FxMeterFrame::from(frame));
    }
}

impl From<MeterFrame> for FxMeterFrame {
    fn from(frame: MeterFrame) -> FxMeterFrame {
        FxMeterFrame {
            available: frame.available,
            measured: frame.measured,
            peak_db: frame.peak_db,
            headroom_db: frame.headroom_db,
            reduction_db: frame.reduction_db,
            risk: frame.risk.as_str().to_owned(),
            bands: frame.bands,
            bands_db: frame.bands_db,
            warning: frame.warning.map(FxWarning::from),
            reason: frame.reason,
        }
    }
}

impl From<audiofx::Warning> for FxWarning {
    fn from(warning: audiofx::Warning) -> FxWarning {
        FxWarning {
            tone: warning.tone.as_str().to_owned(),
            reduction_db: warning.reduction_db,
            cut_db: warning.cut_db,
            from_change: warning.from_change,
        }
    }
}

impl FxPreset {
    fn of(preset: &Preset) -> FxPreset {
        // The chip's sparkline: the same 10..90 % mapping the SPA's `sparkHeight`
        // gives, so two curves named alike are told apart before applying them.
        let spark = preset
            .bands
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(|db| {
                let v = f64::from(*db).clamp(-16.0, 16.0);
                (10.0 + ((v + 16.0) / 32.0) * 80.0) as f32 / 100.0
            })
            .collect();
        FxPreset {
            id: preset.id.clone(),
            name: preset.name.clone(),
            builtin: audiofx::is_builtin(preset),
            spark,
        }
    }
}

fn state_record(state: audiofx::FxState) -> FxState {
    let settings = &state.settings;
    let plan = state.plan;
    let route_text = state.route.describe();
    let path_text = match plan.why {
        Why::Native => "Lecteur Android".to_owned(),
        _ if plan.path == Path::Direct => "Lecture directe".to_owned(),
        _ => "Lecture traitée".to_owned(),
    };
    // `AudioEffects.svelte`'s own `whyText` table, minus the two branches the
    // native app cannot be in.
    let why_text = match plan.why {
        Why::Setting => "choisie ici".to_owned(),
        Why::Glitch => "le son se hachait en lecture traitée sur cette sortie".to_owned(),
        Why::Probing => "nouvel essai après un son haché — le lecteur écoute".to_owned(),
        Why::Native | Why::Default => route_text.clone(),
    };
    FxState {
        eq_enabled: settings.eq_enabled,
        bands: settings.bands.clone(),
        bass: settings.bass,
        normalization: settings.normalization.clone(),
        mono: settings.mono,
        balance: settings.balance,
        pan_text: describe_balance(f64::from(settings.balance)),
        crossfade_enabled: settings.crossfade_enabled,
        crossfade_s: settings.crossfade_s,
        crossfade_on_skip: settings.crossfade_on_skip,
        crossfade_skip_s: settings.crossfade_skip_s,
        trim: settings.trim,
        trim_db: state.trim_db,
        output: settings.output.clone(),
        route_text,
        path_text,
        why_text,
        retry_at_ms: plan.retry_at,
        presets: state.presets.iter().map(FxPreset::of).collect(),
        active_preset: state.active_preset,
        verdicts: state
            .verdicts
            .into_iter()
            .map(|v| FxVerdict {
                key: v.conn.clone(),
                kind: v.kind,
                name: v.name,
                at_ms: v.at,
                retry_at_ms: v.retry_at,
                probing: v.probing,
                count: v.count,
            })
            .collect(),
    }
}

impl PlayerApi {
    /// The audio-effects service over this player, or `None` when the runtime
    /// never started (every call then answers what it would have answered with
    /// defaults, and the card still draws).
    fn fx(&self) -> Option<AudioFx> {
        let player = self.player()?;
        Some(AudioFx::new(
            Arc::clone(self.core_ref()),
            Arc::clone(player),
        ))
    }

    /// One setting, written and applied, answered with the state to draw. Each
    /// of the exported setters is a call to this: the value reaches the engine's
    /// atomics at once, so the change is audible in the next buffer.
    fn fx_with(&self, change: impl FnOnce(&mut audiofx::FxSettings)) -> FxState {
        let Some(fx) = self.fx() else {
            return self.fx_state();
        };
        let mut settings = fx.settings();
        change(&mut settings);
        fx.save(settings);
        state_record(fx.state())
    }
}

#[uniffi::export]
impl PlayerApi {
    /// The whole card's state — this device's settings, the presets, the output's
    /// verdict. **Synchronous and request-free**: `kv` reads only, which is what
    /// lets the card paint on the frame it opens.
    pub fn fx_state(&self) -> FxState {
        match self.fx() {
            Some(fx) => state_record(fx.state()),
            None => state_record(audiofx::FxState {
                settings: audiofx::FxSettings::default().clamped(),
                presets: audiofx::preset_items(&[]),
                active_preset: None,
                route: audiofx::Route::default(),
                plan: audiofx::Plan {
                    path: Path::Processed,
                    why: Why::Native,
                    retry_at: None,
                },
                verdicts: Vec::new(),
                trim_db: nz_core::player::TRIM_DB as i32,
            }),
        }
    }

    pub fn fx_set_eq_enabled(&self, enabled: bool) -> FxState {
        self.fx_with(|s| s.eq_enabled = enabled)
    }

    /// One band's gain in dB, ±16: the fader the person is dragging.
    pub fn fx_set_band(&self, index: u32, db: f32) -> FxState {
        self.fx_with(|s| {
            if let Some(band) = s.bands.get_mut(index as usize) {
                *band = if db.is_finite() {
                    db.clamp(-16.0, 16.0)
                } else {
                    0.0
                };
            }
        })
    }

    /// The bass lift, 0..1 (`fx.bass`): the chain gets `× 12` dB.
    pub fn fx_set_bass(&self, lift: f32) -> FxState {
        self.fx_with(|s| s.bass = lift)
    }

    pub fn fx_set_normalization(&self, level: String) -> FxState {
        self.fx_with(|s| s.normalization = level)
    }

    pub fn fx_set_mono(&self, mono: bool) -> FxState {
        self.fx_with(|s| s.mono = mono)
    }

    /// −1 (hard left) … 0 (centre) … 1 (hard right).
    pub fn fx_set_balance(&self, balance: f32) -> FxState {
        self.fx_with(|s| s.balance = balance)
    }

    /// The "Centre" chip.
    pub fn fx_reset_balance(&self) -> FxState {
        self.fx_with(|s| s.balance = 0.0)
    }

    pub fn fx_set_crossfade(&self, enabled: bool, seconds: f32) -> FxState {
        self.fx_with(|s| {
            s.crossfade_enabled = enabled;
            s.crossfade_s = seconds;
        })
    }

    pub fn fx_set_skip_fade(&self, enabled: bool, seconds: f32) -> FxState {
        self.fx_with(|s| {
            s.crossfade_on_skip = enabled;
            s.crossfade_skip_s = seconds;
        })
    }

    pub fn fx_set_trim(&self, on: bool) -> FxState {
        self.fx_with(|s| s.trim = on)
    }

    /// `auto` | `direct` | `graph` — the SPA's own three modes. The app's player
    /// always processes, so the mode is what the verdicts are judged against and
    /// what the card explains, not a second audio path.
    pub fn fx_set_output(&self, mode: String) -> FxState {
        self.fx_with(|s| s.output = mode)
    }

    /// "Enregistrer comme…": the whole chain under a name (≤ 40 characters).
    /// Re-saving an existing name keeps that row's id.
    pub fn fx_save_preset(&self, name: String) -> Option<FxPreset> {
        self.fx()?.save_preset(&name).as_ref().map(FxPreset::of)
    }

    /// "Mettre à jour": the curve and the lift into the person's own preset.
    pub fn fx_update_preset(&self, id: String) -> Option<FxPreset> {
        self.fx()?.update_preset(&id).as_ref().map(FxPreset::of)
    }

    pub fn fx_delete_preset(&self, id: String) -> bool {
        self.fx().map(|fx| fx.delete_preset(&id)).unwrap_or(false)
    }

    /// Apply a preset: its curve, its switch, its lift and its level.
    pub fn fx_apply_preset(&self, id: String) -> Option<FxState> {
        let fx = self.fx()?;
        fx.apply_preset(&id)?;
        Some(state_record(fx.state()))
    }

    // -- the meter ------------------------------------------------------------

    /// Starts the meter for the card on screen. Called from the card's
    /// `onStart`/`onDispose` equivalents — never on a timer: the thread exists
    /// only while the card does, so the analysis really stops when it leaves.
    pub fn fx_meter_subscribe(&self, listener: Box<dyn AudioFxListener>) {
        let Some(fx) = self.fx() else {
            return;
        };
        let mut slot = METER.lock().unwrap_or_else(|e| e.into_inner());
        // A card that opens twice (a recomposition, a new instance) replaces the
        // running meter instead of stacking a second thread behind it.
        if let Some(mut previous) = slot.take() {
            previous.session.stop_and_join();
        }
        let sink = Arc::new(ListenerSink(Arc::from(listener)));
        if let Some(session) = fx.start_meter(sink) {
            *slot = Some(MeterSlot { session });
        }
    }

    /// Stops AND joins the meter: after this returns, no reading can arrive.
    pub fn fx_meter_unsubscribe(&self) {
        let mut slot = METER.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(mut session) = slot.take() {
            // Joined, not merely flagged: the caller may then assert that the
            // reading count no longer moves (the acceptance's 0 CPU).
            session.session.stop_and_join();
        }
    }

    /// A deliberate change (a band, the switch, a preset, the bass, the level):
    /// the warning's next episode starts from the reading on screen.
    pub fn fx_meter_note_change(&self, key: String) {
        let slot = METER.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(slot) = slot.as_ref() {
            slot.session.note_change(&key);
        }
    }

    /// How many readings the meter has taken: the counter a bench compares
    /// before and after the card leaves.
    pub fn fx_meter_stats(&self) -> FxMeterStats {
        let slot = METER.lock().unwrap_or_else(|e| e.into_inner());
        match slot.as_ref() {
            Some(slot) => FxMeterStats {
                ticks: slot.session.ticks(),
                frames: slot.session.frames(),
                running: true,
            },
            None => FxMeterStats::default(),
        }
    }

    // -- the output's verdict -------------------------------------------------

    /// The output's own reading, without the rest of the card.
    pub fn fx_output_state(&self) -> FxState {
        self.fx_state()
    }

    /// A fault measured on the output playing now (the engine's own starvations,
    /// seen by the service). Answers whether that changes what plays.
    pub fn fx_report_glitch(&self, why: String, ratio: Option<f64>) -> bool {
        self.fx()
            .map(|fx| fx.report_glitch(why, ratio))
            .unwrap_or(false)
    }

    /// At a track change: retry the processed path if the verdict is due.
    pub fn fx_retry_if_due(&self) -> bool {
        self.fx().map(|fx| fx.retry_if_due()).unwrap_or(false)
    }

    /// "Réessayer maintenant" on the card.
    pub fn fx_retry_now(&self) {
        if let Some(fx) = self.fx() {
            fx.retry_now();
        }
    }

    /// Forget one output's verdict.
    pub fn fx_forget_glitch(&self, key: String) {
        if let Some(fx) = self.fx() {
            fx.forget_glitch(&key);
        }
    }
}

/// The card's own tone vocabulary, for a Kotlin `when` that never guesses.
#[uniffi::export]
pub fn fx_risk_tone(value: String) -> String {
    Tone::parse(&value).as_str().to_owned()
}

/// The mode's stored spelling, so the screen and the file cannot disagree.
#[uniffi::export]
pub fn fx_mode_id(mode: String) -> String {
    Mode::parse(&mode).as_str().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn test_core(tag: &str) -> Arc<crate::core::NzCore> {
        let path = std::env::temp_dir().join(format!(
            "nz-ffi-fx-{}-{tag}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("data")).expect("data");
        std::fs::create_dir_all(path.join("cache")).expect("cache");
        crate::core::NzCore::new(crate::core::CoreConfig {
            data_dir: path.join("data").display().to_string(),
            cache_dir: path.join("cache").display().to_string(),
            app_version: "0.0.0".into(),
            version_code: 2_000_001,
            sdk_int: 35,
            device_name: "test".into(),
        })
        .expect("the core")
    }

    /// The card's first frame: the SPA's defaults, the five built-ins, and the
    /// app's own player as the reason (it always processes) — **0 requests**.
    #[test]
    fn a_fresh_card_is_the_spas_defaults_and_costs_no_request() {
        let core = test_core("defaults");
        let player = core.player();
        let state = player.fx_state();
        assert!(!state.eq_enabled);
        assert_eq!(state.bands, vec![0.0; 10]);
        assert_eq!(state.bass, 0.0);
        assert_eq!(state.normalization, "off");
        assert!(!state.mono);
        assert_eq!(state.balance, 0.0);
        assert_eq!(state.pan_text, "Centre");
        assert!(!state.crossfade_enabled);
        assert_eq!(state.crossfade_s, 6.0);
        assert!(state.crossfade_on_skip);
        assert_eq!(state.crossfade_skip_s, 1.5);
        assert!(state.trim);
        assert_eq!(state.trim_db, -45);
        assert_eq!(state.output, "auto");
        assert_eq!(
            state
                .presets
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            ["Plat", "Basses+", "Voix", "Aigus+", "Loudness"]
        );
        assert!(state.presets.iter().all(|p| p.builtin));
        assert_eq!(state.presets[1].spark.len(), 10);
        // The app's own player: processed, and the card says which player.
        assert_eq!(state.path_text, "Lecteur Android");
        assert_eq!(core.stats().iter().map(|s| s.requests).sum::<u64>(), 0);
        core.shutdown();
    }

    /// A control writes, the player applies, and the state answers what was
    /// written — including the fields this card does not own.
    #[test]
    fn a_control_writes_and_the_state_reads_back() {
        let core = test_core("write");
        let player = core.player();
        let state = player.fx_set_eq_enabled(true);
        assert!(state.eq_enabled);
        let state = player.fx_set_band(4, 12.0);
        assert_eq!(state.bands[4], 12.0);
        // Out of range and hostile values are clamped, never stored.
        let state = player.fx_set_band(4, 99.0);
        assert_eq!(state.bands[4], 16.0);
        let state = player.fx_set_band(99, 5.0);
        assert_eq!(state.bands.len(), 10);
        let state = player.fx_set_band(0, f32::NAN);
        assert_eq!(state.bands[0], 0.0);
        // The engine's chain really got it: `AudioSettings` is the player's own.
        let audio = player.audio();
        assert_eq!(audio.eq_db[4], 16.0);
        assert_eq!(audio.eq_db[0], 0.0);
        let state = player.fx_set_bass(0.5);
        assert_eq!(player.audio().bass_db, 6.0, "the chain's +6 dB");
        assert_eq!(state.bass, 0.5);
        let state = player.fx_set_mono(true);
        assert!(state.mono);
        assert_eq!(state.pan_text, "Centre");
        let state = player.fx_set_balance(-1.0);
        assert_eq!(state.pan_text, "À gauche");
        assert!(player.audio().mono);
        // The crossfade's length is remembered while its switch is off.
        player.fx_set_crossfade(true, 4.0);
        let state = player.fx_set_crossfade(false, 4.0);
        assert_eq!(state.crossfade_s, 4.0);
        assert_eq!(player.audio().crossfade_s, 0.0);
        core.shutdown();
    }

    /// **The screen's write reaches the LIVE engine, not only the string.**
    ///
    /// The other tests (and the JVM ones) stop at `"high"` coming back: this one
    /// reads the player's own `AudioSettings`, the value `apply_audio` hands the
    /// engine's atomics, which is the link that was missing between the card and
    /// the sound (`docs/agents/mobile.md`, T6).
    #[test]
    fn fx_set_normalization_reaches_the_live_player() {
        let core = test_core("normalization-live");
        let api = core.player();
        let live = api.player().expect("the live player");
        assert_eq!(
            live.audio().normalization,
            nz_core::player::Normalization::Off
        );
        let state = api.fx_set_normalization("high".to_owned());
        assert_eq!(state.normalization, "high", "the card draws what it wrote");
        // The FFI's own record, which is what the screen reads…
        assert_eq!(
            api.audio().normalization,
            crate::player::Normalization::High
        );
        // …and the CORE's settings, which `apply_audio` hands the engine.
        assert_eq!(
            live.audio().normalization,
            nz_core::player::Normalization::High,
            "the engine's own setting got the level"
        );
        // The unknown value is off, exactly as the chain's `parse` keeps it.
        let state = api.fx_set_normalization("maximum".to_owned());
        assert_eq!(state.normalization, "off");
        assert_eq!(
            live.audio().normalization,
            nz_core::player::Normalization::Off
        );
        core.shutdown();
    }

    /// A preset survives a restart of the core (it is in `kv`), a re-save keeps
    /// its id, and a built-in refuses both update and delete.
    #[test]
    fn presets_round_trip_through_the_devices_store() {
        let core = test_core("presets");
        let player = core.player();
        player.fx_set_eq_enabled(true);
        player.fx_set_band(1, 6.0);
        let saved = player.fx_save_preset("Casque".to_owned()).expect("saved");
        assert_eq!(saved.name, "Casque");
        assert!(!saved.builtin);
        let state = player.fx_state();
        assert_eq!(state.presets.len(), 6, "five built-ins and one of mine");
        // The chain it captured IS the chain it applies: the card highlights it.
        assert_eq!(state.active_preset.as_deref(), Some(saved.id.as_str()));
        // Re-saving the same name keeps the row's id.
        let again = player.fx_save_preset("casque".to_owned()).expect("saved");
        assert_eq!(again.id, saved.id);
        // A shipped curve is nobody's to rewrite or delete.
        assert!(player.fx_update_preset("bass".to_owned()).is_none());
        assert!(!player.fx_delete_preset("bass".to_owned()));
        assert!(player.fx_update_preset(saved.id.clone()).is_some());
        // Applying it after a change restores it.
        player.fx_set_band(1, -6.0);
        let state = player.fx_apply_preset(saved.id.clone()).expect("applied");
        assert_eq!(state.bands[1], 6.0);
        assert!(player.fx_delete_preset(saved.id.clone()));
        assert_eq!(player.fx_state().presets.len(), 5);
        core.shutdown();
    }

    struct Counter(Arc<AtomicU64>);

    impl AudioFxListener for Counter {
        fn on_meter(&self, _frame: FxMeterFrame) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// **The meter runs while the card is subscribed and stops dead when it
    /// leaves**: the counters are the acceptance's own numbers.
    #[test]
    fn the_meter_stops_when_the_card_leaves() {
        let core = test_core("meter");
        let player = core.player();
        let frames = Arc::new(AtomicU64::new(0));
        player.fx_meter_subscribe(Box::new(Counter(Arc::clone(&frames))));
        // Wait on the meter's own readings, never on the clock.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while player.fx_meter_stats().ticks == 0 && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        let running = player.fx_meter_stats();
        assert!(running.running);
        assert!(running.ticks > 0, "the meter took a reading");
        player.fx_meter_unsubscribe();
        let stopped = player.fx_meter_stats();
        assert!(!stopped.running);
        assert_eq!(stopped.ticks, 0, "an unsubscribed card has no meter");
        let at_stop = frames.load(Ordering::Relaxed);
        assert!(at_stop > 0, "the listener saw at least one frame");
        // Nothing arrives afterwards: the thread is joined, not flagged.
        let wait = std::time::Instant::now() + std::time::Duration::from_millis(200);
        while std::time::Instant::now() < wait {
            std::thread::yield_now();
        }
        assert_eq!(frames.load(Ordering::Relaxed), at_stop);
        core.shutdown();
    }
}
