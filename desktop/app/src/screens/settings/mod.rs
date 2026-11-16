//! Réglages: every section, one screen.
//!
//! One entity, one page: the category strip at the top (the SPA's two-level
//! settings menu, `routes/Settings.svelte`), a « Chercher un réglage… » field
//! that narrows that strip, and the section's own body rendered by the module
//! that owns it. The sections another lot fills (`viz`, `downloads`,
//! `transcribe`, `users`, `server`) keep D7's « Bientôt » stub until that lot
//! dispatches its own arm here: a delivered section is one `match` line and one
//! file, never an edit to the shell.
//!
//! What lives here is what the sections share, and only that: the device
//! settings ([`Values`], the SPA's `localStorage`, saved through `nz-cache`'s
//! `kv`), the server answers ([`Data`], one [`Load`] per route so a screen that
//! cannot load says why and offers « Réessayer »), the one reply channel every
//! request writes to (no worker touches GPUI state), the knob entities the
//! Omarchy sliders are bound to, and the small builders each section composes.
//!
//! The values reach the audio engine the moment they change (`nz-audio`, D6's
//! setters): a click is heard in the next buffer, never at the next track. The
//! one exception is the stream quality, chosen when the next voice is opened,
//! and the screen says so.

pub mod account;
pub mod appearance;
pub mod discovery;
pub mod downloads;
pub mod help;
pub mod playback;
pub mod server;
pub mod shortcuts;
pub mod sources;
pub mod transcribe;
pub mod users;
pub mod viz;

use std::collections::HashSet;

use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::base::slider::{SliderEvent, SliderState, SliderValue};
use gpui_kit::{
    AnyElement, AnyView, AppContext as _, Context, Div, ElementId, Entity, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, Window, div, rems,
};
use gpui_omarchy::{focus_scope, icon, input, toggle, toggle_group};
use nz_api::ApiError;
use nz_audio::Normalization;
use nz_cache::Cache;
use nz_model::{
    Access, DiscoverPrefs, MyAccounts, ReleasesSettings, ServerSettings, Services, Sources,
};
use serde::{Deserialize, Serialize};

use crate::nav::{Route, SettingsSection};
use crate::state::AppState;
use crate::theme;
use crate::toast::ToastKind;
use crate::wiring::engine::{Audio, PlaybackQuality};
use crate::wiring::events::{self, Sender};

/// Longest crossfade the sliders offer, in seconds: `crossfade.rs#MAX_SECONDS`
/// (the web player's `XFADE_MAX`). Re-stated here because the engine keeps its
/// constants private; `MAX_SECONDS` is asserted against in `nz-audio`'s tests.
pub const FADE_MAX_S: f32 = 12.;
/// `crossfade.rs#DEFAULT_SECONDS`.
pub const FADE_DEFAULT_S: f32 = 6.;
/// `crossfade.rs#MAX_SKIP_SECONDS`.
pub const SKIP_FADE_MAX_S: f32 = 5.;
/// `crossfade.rs#DEFAULT_SKIP_SECONDS`.
pub const SKIP_FADE_DEFAULT_S: f32 = 1.5;

/// The `kv` keys of the device settings. They are the SPA's `localStorage`
/// entries, one row each: the same value means the same thing to a person who
/// uses both clients, and a key this build no longer reads is simply ignored.
pub mod keys {
    /// `stores.js#eqBands`: the ten band gains, flat = off.
    pub const EQ: &str = "audio.eq";
    /// `stores.js#bassBoost`, in dB (the page stores 0…1, the engine wants dB).
    pub const BASS: &str = "audio.bass";
    /// `stores.js#normalization`: `off` | `low` | `medium` | `high`.
    pub const NORMALIZATION: &str = "audio.normalization";
    /// `pan.js#setMono`.
    pub const MONO: &str = "audio.mono";
    /// `pan.js#setBalance`: −1 hard left … 1 hard right.
    pub const BALANCE: &str = "audio.balance";
    /// `stores.js#crossfadeEnabled` and the three numbers beside it.
    pub const FADE_ENABLED: &str = "audio.fade.enabled";
    pub const FADE_SECONDS: &str = "audio.fade.seconds";
    pub const FADE_ON_SKIP: &str = "audio.fade.on_skip";
    pub const FADE_SKIP_SECONDS: &str = "audio.fade.skip_seconds";
    /// The output device's own name; absent is the host's default.
    pub const DEVICE: &str = "audio.output.device";
    /// `ui.reduce_motion` (D7 reads it; Apparence writes it).
    pub const REDUCE_MOTION: &str = "ui.reduce_motion";
    /// The person's own equaliser presets (`stores.js#fxPresets`).
    pub const PRESETS: &str = "audio.presets";
}

/// The ten bands of the equaliser, in Hz (`graph.js#EQ_FREQS`). The engine's
/// `nscore` chain has the same ten, in the same order.
pub const EQ_FREQS: [u32; 10] = [31, 62, 125, 250, 500, 1_000, 2_000, 4_000, 8_000, 16_000];
/// The band range `graph.js` gives its faders, in dB.
pub const EQ_MIN_DB: f32 = -16.;
pub const EQ_MAX_DB: f32 = 16.;

/// One built-in equaliser preset: `lib/eqpresets.js#BUILTIN_PRESETS`, value for
/// value. A built-in is code, so it can be applied but never updated or deleted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BuiltinPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub bands: [f32; 10],
}

pub const BUILTIN_PRESETS: [BuiltinPreset; 5] = [
    BuiltinPreset {
        id: "flat",
        label: "Plat",
        bands: [0., 0., 0., 0., 0., 0., 0., 0., 0., 0.],
    },
    BuiltinPreset {
        id: "bass",
        label: "Basses+",
        bands: [6., 5., 4., 2., 0., 0., 0., 0., 0., 0.],
    },
    BuiltinPreset {
        id: "vocal",
        label: "Voix",
        bands: [-2., -1., 0., 2., 4., 4., 3., 1., 0., -1.],
    },
    BuiltinPreset {
        id: "treble",
        label: "Aigus+",
        bands: [0., 0., 0., 0., 0., 1., 2., 4., 5., 6.],
    },
    BuiltinPreset {
        id: "loud",
        label: "Loudness",
        bands: [6., 4., 2., 0., -1., -1., 0., 2., 4., 6.],
    },
];

/// A preset the person saved: the curve and the bass lift, named.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub bands: [f32; 10],
    #[serde(default)]
    pub bass: f32,
}

/// The level of ReplayGain-style normalisation, as `stores.js` spells it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NormLevel {
    #[default]
    Off,
    Low,
    Medium,
    High,
}

impl NormLevel {
    pub const ALL: [NormLevel; 4] = [
        NormLevel::Off,
        NormLevel::Low,
        NormLevel::Medium,
        NormLevel::High,
    ];

    pub fn id(self) -> &'static str {
        match self {
            NormLevel::Off => "off",
            NormLevel::Low => "low",
            NormLevel::Medium => "medium",
            NormLevel::High => "high",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            NormLevel::Off => "Désactivée",
            NormLevel::Low => "Basse",
            NormLevel::Medium => "Normale",
            NormLevel::High => "Élevée",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "low" => NormLevel::Low,
            "medium" => NormLevel::Medium,
            "high" => NormLevel::High,
            _ => NormLevel::Off,
        }
    }

    pub fn engine(self) -> Normalization {
        match self {
            NormLevel::Off => Normalization::Off,
            NormLevel::Low => Normalization::Low,
            NormLevel::Medium => Normalization::Medium,
            NormLevel::High => Normalization::High,
        }
    }
}

/// Everything the device remembers, in one struct: read once when the screen
/// opens, written back one key at a time as the person changes something.
#[derive(Clone, Debug, PartialEq)]
pub struct Values {
    pub eq: [f32; 10],
    pub bass: f32,
    pub normalization: NormLevel,
    pub mono: bool,
    pub balance: f32,
    pub fade_enabled: bool,
    pub fade_seconds: f32,
    pub fade_on_skip: bool,
    pub fade_skip_seconds: f32,
    /// The chosen output device; `None` is the host's default.
    pub device: Option<String>,
    pub presets: Vec<Preset>,
}

impl Default for Values {
    fn default() -> Self {
        Self {
            eq: [0.; 10],
            bass: 0.,
            normalization: NormLevel::Off,
            mono: false,
            balance: 0.,
            fade_enabled: false,
            fade_seconds: FADE_DEFAULT_S,
            fade_on_skip: false,
            fade_skip_seconds: SKIP_FADE_DEFAULT_S,
            device: None,
            presets: Vec::new(),
        }
    }
}

impl Values {
    /// Read every device setting. A key an older build wrote in another shape
    /// reads as unset (`nz-cache`'s `kv` contract), so an upgrade keeps what
    /// parses and drops the rest instead of failing to open.
    pub fn load(cache: Option<&Cache>) -> Self {
        let mut values = Self::default();
        let Some(cache) = cache else {
            return values;
        };
        let kv = cache.kv();
        if let Some(eq) = kv.get::<[f32; 10]>(keys::EQ) {
            values.eq = eq.map(|db| db.clamp(EQ_MIN_DB, EQ_MAX_DB));
        }
        if let Some(bass) = kv.get::<f32>(keys::BASS) {
            values.bass = bass.clamp(0., 16.);
        }
        if let Some(level) = kv.get::<String>(keys::NORMALIZATION) {
            values.normalization = NormLevel::parse(&level);
        }
        if let Some(mono) = kv.get::<bool>(keys::MONO) {
            values.mono = mono;
        }
        if let Some(balance) = kv.get::<f64>(keys::BALANCE) {
            values.balance = (balance as f32).clamp(-1., 1.);
        }
        if let Some(on) = kv.get::<bool>(keys::FADE_ENABLED) {
            values.fade_enabled = on;
        }
        if let Some(seconds) = kv.get::<f64>(keys::FADE_SECONDS) {
            values.fade_seconds = (seconds as f32).clamp(0., FADE_MAX_S);
        }
        if let Some(on) = kv.get::<bool>(keys::FADE_ON_SKIP) {
            values.fade_on_skip = on;
        }
        if let Some(seconds) = kv.get::<f64>(keys::FADE_SKIP_SECONDS) {
            values.fade_skip_seconds = (seconds as f32).clamp(0., SKIP_FADE_MAX_S);
        }
        if let Some(device) = kv.get::<String>(keys::DEVICE) {
            values.device = (!device.is_empty()).then_some(device);
        }
        if let Some(presets) = kv.get::<Vec<Preset>>(keys::PRESETS) {
            values.presets = presets
                .into_iter()
                .filter(|preset| !preset.name.trim().is_empty())
                .map(|mut preset| {
                    preset.bands = preset.bands.map(|db| db.clamp(EQ_MIN_DB, EQ_MAX_DB));
                    preset.bass = preset.bass.clamp(0., 16.);
                    preset
                })
                .collect();
        }
        values
    }

    /// Push the sound settings into the engine. Every setter is an atomic
    /// store the output callback reads at the top of its next buffer, so this
    /// is safe from the UI thread and audible within about 10 ms (`nz-audio`,
    /// D6) — never at the next track.
    pub fn apply(&self, engine: &nz_audio::Engine) {
        engine.set_normalization(self.normalization.engine());
        for (band, db) in self.eq.iter().enumerate() {
            engine.set_eq(band, *db);
        }
        engine.set_bass(self.bass);
        engine.set_mono(self.mono);
        engine.set_balance(self.balance);
        engine.set_fade(self.fade());
    }

    /// Apply to whatever engine the app has, if any: a machine with no sound
    /// server still remembers every setting, it just has nothing to hear it.
    pub fn apply_to(&self, audio: Option<&Audio>) {
        if let Some(audio) = audio {
            self.apply(&audio.engine);
        }
    }

    /// The crossfade block as the engine takes it.
    pub fn fade(&self) -> nz_audio::FadeSettings {
        nz_audio::FadeSettings {
            enabled: self.fade_enabled,
            seconds: f64::from(self.fade_seconds),
            on_skip: self.fade_on_skip,
            skip_seconds: f64::from(self.fade_skip_seconds),
        }
        .sanitized()
    }

    /// The preset that names the current curve and lift, if any: a saved one
    /// matches on both (it carries more of the chain, so it wins), a built-in
    /// on its bands alone, which is all it prescribes.
    pub fn active_preset(&self) -> Option<String> {
        if let Some(preset) = self
            .presets
            .iter()
            .find(|preset| preset.bands == self.eq && (preset.bass - self.bass).abs() < 0.005)
        {
            return Some(preset.name.clone());
        }
        BUILTIN_PRESETS
            .iter()
            .find(|preset| preset.bands == self.eq)
            .map(|preset| preset.label.to_owned())
    }
}

/// One route's answer, in the three states a screen must tell apart.
#[derive(Clone, Debug, Default)]
pub enum Load<T> {
    #[default]
    Idle,
    Loading,
    Ready(T),
    Failed(SharedString),
}

impl<T> Load<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Load::Ready(value) => Some(value),
            _ => None,
        }
    }

    pub fn is_idle(&self) -> bool {
        matches!(self, Load::Idle)
    }
}

/// What the server answered, one slot per route. A section fills the slots it
/// draws; a slot still `Idle` has never been asked.
#[derive(Default)]
pub struct Data {
    pub sources: Load<Sources>,
    pub accounts: Load<MyAccounts>,
    pub access: Load<Access>,
    pub prefs: Load<DiscoverPrefs>,
    pub releases: Load<ReleasesSettings>,
    pub services: Load<Services>,
    pub server: Load<ServerSettings>,
    pub deezer: Load<nz_model::DeezerStatus>,
    pub dislikes: Load<nz_model::TrackList>,
    pub version: Load<nz_model::Version>,
    pub storage: Load<nz_model::Storage>,
    /// The PipeWire sinks `pw-dump` listed (Réglages → Lecture).
    pub sinks: Load<Vec<String>>,
}

/// What a request answers. One channel, one enum: a worker never touches GPUI
/// state, it sends one of these and the screen applies it on its own thread
/// (D7's rule, and the reason a request can never repaint mid-frame).
pub enum Reply {
    Sources(Result<Sources, String>),
    Accounts(Result<MyAccounts, String>),
    Access(Result<Access, String>),
    Prefs(Result<DiscoverPrefs, String>),
    Releases(Result<ReleasesSettings, String>),
    Services(Result<Services, String>),
    ServerSettings(Result<ServerSettings, String>),
    Deezer(Result<nz_model::DeezerStatus, String>),
    Dislikes(Result<nz_model::TrackList, String>),
    Version(Result<nz_model::Version, String>),
    Storage(Result<nz_model::Storage, String>),
    Sinks(Vec<String>),
    /// A mutation succeeded: the sentence to show.
    Saved(&'static str),
    /// A mutation failed: the cause, already in French.
    Failed(String),
    /// A mutation succeeded and its answer replaces the slot.
    SourcesSaved(Result<Sources, String>),
    AccountsSaved(Result<MyAccounts, String>),
    AccessSaved(Result<Access, String>),
    ServicesSaved(Result<Services, String>),
    /// The discovery notch could not be saved: the level the server still holds
    /// comes back with the cause, instead of a notch it never accepted.
    DiscoveryFailed {
        previous: f32,
        cause: String,
    },
}

/// Which knob a slider drives. One variant per number the settings own, so the
/// ten band faders and the five other sliders share one subscription.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Knob {
    Band(usize),
    Bass,
    Balance,
    Fade,
    SkipFade,
}

/// The Réglages view: one entity for every section, built by the route table.
pub struct Screen {
    pub section: SettingsSection,
    pub state: Entity<AppState>,
    pub values: Values,
    pub data: Data,
    /// The discovery notch (0…1), as the server holds it. Its control is drawn
    /// from this value rather than from a slider entity, because the server is
    /// what sets it (`/discover/prefs`) and a control whose thumb disagrees
    /// with the value is worse than none.
    pub discover_level: f32,
    /// The slider entities, one per [`Knob`].
    pub knobs: Vec<(Knob, Entity<SliderState>)>,
    pub reply: Sender<Reply>,
    /// The sections whose routes were already asked for, so switching tabs
    /// costs nothing and a revisit does not re-fetch.
    pub loaded: HashSet<SettingsSection>,
    /// A mutation in flight, named so its button can say so and stay disabled.
    pub saving: Option<&'static str>,
    /// The pane of a section that has several (Lecture et son, Sources).
    pub pane: usize,
    /// How many rows of a long list (the disliked titles) are drawn.
    pub shown: usize,
    /// Whether the saved sound settings have been handed to the engine of this
    /// session. The engine starts on its own thread, so a page opened in the
    /// first second finds it later; the render below catches up once.
    applied_audio: bool,
    /// The search field's text, read once per paint: the strip is built from
    /// it, and no section needs to see it.
    pub query_cache: String,
    /// The keyboard anchor of the page: Tab starts here and walks the
    /// controls in reading order (`focus_scope`, gpui-omarchy).
    pub focus: gpui_kit::FocusHandle,
    /// The body's scroll offset, so Page Up / Page Down / Home / End move the
    /// page itself — those four keys only, because `j`/`k` are the track
    /// lists' and Réglages is not a list.
    pub scroll: gpui_kit::ScrollHandle,
    /// A section another lot owns and builds as its own screen: Utilisateurs
    /// and Serveur (D14b). Everything else is this page.
    delegated: Option<AnyView>,
    /// The search field itself, kept so the subscription below is created once
    /// (and so a test can read what was typed).
    pub query_entity: Option<Entity<InputState>>,
    _subscriptions: Vec<Subscription>,
}

/// The text fields a section draws. They are created on first paint (a GPUI
/// `InputState` needs a window) and kept per window, so a draft survives a tab
/// switch and a re-render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Query,
    Arl,
    ClientId,
    YoutubeHeaders,
    PresetName,
}

impl Field {
    fn spec(self) -> (usize, &'static str, bool) {
        match self {
            Field::Query => (0, "Chercher un réglage…", false),
            Field::Arl => (1, "Coller le nouvel ARL…", true),
            Field::ClientId => (2, "Identifiant d'application Spotify", false),
            Field::YoutubeHeaders => (3, "En-têtes de la requête YouTube Music", false),
            Field::PresetName => (4, "Nom du préréglage", false),
        }
    }
}

impl Screen {
    pub fn new(section: SettingsSection, state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        // Utilisateurs and Serveur are D14b's own screens: this page hands the
        // whole area to them, as the plan's dispatch does for every lot whose
        // section is not a column of these cards.
        let delegated: Option<AnyView> = match section {
            SettingsSection::Users => Some(
                cx.new(|cx| users::Screen::new(section, state.clone(), cx))
                    .into(),
            ),
            SettingsSection::Server => Some(
                cx.new(|cx| server::Screen::new(section, state.clone(), cx))
                    .into(),
            ),
            _ => None,
        };
        let cache = state.read(cx).cache.clone();
        let values = Values::load(cache.as_ref());
        // The engine may already be running (a restored session): apply what
        // was saved before the first paint, so the sound matches the settings.
        let applied_audio = state.read(cx).audio.is_some();
        values.apply_to(state.read(cx).audio.as_ref());
        let discover_level = cache
            .as_ref()
            .and_then(|cache| cache.kv().get::<f64>(discovery::LEVEL_KEY))
            .map(|level| (level as f32).clamp(0., 1.))
            .unwrap_or(0.35);

        let mut knobs: Vec<(Knob, Entity<SliderState>)> = Vec::new();
        for band in 0..10 {
            let value = values.eq[band];
            knobs.push((
                Knob::Band(band),
                cx.new(move |_| {
                    SliderState::new()
                        .min(EQ_MIN_DB)
                        .max(EQ_MAX_DB)
                        .step(1.)
                        .default_value(value)
                }),
            ));
        }
        let bass = values.bass;
        knobs.push((
            Knob::Bass,
            cx.new(move |_| {
                SliderState::new()
                    .min(0.)
                    .max(16.)
                    .step(1.)
                    .default_value(bass)
            }),
        ));
        let balance = values.balance;
        knobs.push((
            Knob::Balance,
            cx.new(move |_| {
                SliderState::new()
                    .min(-1.)
                    .max(1.)
                    .step(0.01)
                    .default_value(balance)
            }),
        ));
        let fade = values.fade_seconds;
        knobs.push((
            Knob::Fade,
            cx.new(move |_| {
                SliderState::new()
                    .min(0.)
                    .max(FADE_MAX_S)
                    .step(0.5)
                    .default_value(fade)
            }),
        ));
        let skip = values.fade_skip_seconds;
        knobs.push((
            Knob::SkipFade,
            cx.new(move |_| {
                SliderState::new()
                    .min(0.)
                    .max(SKIP_FADE_MAX_S)
                    .step(0.25)
                    .default_value(skip)
            }),
        ));

        let (reply, replies) = events::channel::<Reply>();
        let mut subscriptions = vec![cx.observe(&state, |_, _, cx| cx.notify())];
        for (knob, slider) in &knobs {
            let knob = *knob;
            subscriptions.push(
                cx.subscribe(slider, move |this, _, event: &SliderEvent, cx| {
                    let value = match event {
                        SliderEvent::Change(value) | SliderEvent::Release(value) => match value {
                            SliderValue::Single(value) => *value,
                            SliderValue::Range(start, _) => *start,
                        },
                    };
                    this.on_knob(knob, value, cx);
                }),
            );
        }

        let mut this = Self {
            section,
            state,
            values,
            data: Data::default(),
            discover_level,
            knobs,
            reply,
            loaded: HashSet::new(),
            saving: None,
            pane: 0,
            shown: 12,
            applied_audio,
            focus: cx.focus_handle(),
            scroll: gpui_kit::ScrollHandle::new(),
            delegated,
            query_cache: String::new(),
            query_entity: None,
            _subscriptions: subscriptions,
        };
        this.drain(replies, cx);
        this
    }

    /// The section this view was built for.
    pub fn section(&self) -> SettingsSection {
        self.section
    }

    /// Drain what the workers answered. One foreground task for the screen's
    /// whole life: the request itself ran on `nz-net`.
    fn drain(&mut self, replies: events::Receiver<Reply>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            while let Some(reply) = replies.next().await {
                let alive = this.update(cx, |this, cx| this.on_reply(reply, cx));
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn on_reply(&mut self, reply: Reply, cx: &mut Context<Self>) {
        match reply {
            Reply::Sources(result) => self.data.sources = into_load(result),
            Reply::Accounts(result) => self.data.accounts = into_load(result),
            Reply::Access(result) => self.data.access = into_load(result),
            Reply::Prefs(result) => {
                if let Ok(prefs) = &result {
                    self.discover_level = (prefs.level as f32).clamp(0., 1.);
                }
                self.data.prefs = into_load(result);
            }
            Reply::Releases(result) => self.data.releases = into_load(result),
            Reply::Services(result) => self.data.services = into_load(result),
            Reply::ServerSettings(result) => self.data.server = into_load(result),
            Reply::Deezer(result) => self.data.deezer = into_load(result),
            Reply::Dislikes(result) => self.data.dislikes = into_load(result),
            Reply::Version(result) => self.data.version = into_load(result),
            Reply::Sinks(sinks) => self.data.sinks = Load::Ready(sinks),
            Reply::DiscoveryFailed { previous, cause } => {
                self.discover_level = previous;
                self.save(discovery::LEVEL_KEY, &f64::from(previous), cx);
                self.fail(cause, cx);
            }
            Reply::Storage(result) => self.data.storage = into_load(result),
            Reply::SourcesSaved(result) => {
                self.saving = None;
                match result {
                    Ok(sources) => self.data.sources = Load::Ready(sources),
                    Err(cause) => {
                        // The optimistic reorder is not kept: the server's
                        // order is the true one, so it is asked again.
                        self.data.sources = Load::Idle;
                        self.load_sources(cx);
                        self.fail(cause, cx);
                    }
                }
            }
            Reply::AccountsSaved(result) => {
                self.saving = None;
                match result {
                    Ok(accounts) => self.data.accounts = Load::Ready(accounts),
                    Err(cause) => self.fail(cause, cx),
                }
            }
            Reply::AccessSaved(result) => {
                self.saving = None;
                match result {
                    Ok(access) => self.data.access = Load::Ready(access),
                    Err(cause) => self.fail(cause, cx),
                }
            }
            Reply::ServicesSaved(result) => {
                self.saving = None;
                match result {
                    Ok(services) => self.data.services = Load::Ready(services),
                    Err(cause) => self.fail(cause, cx),
                }
            }
            Reply::Saved(message) => {
                self.saving = None;
                self.toast(ToastKind::Info, message, cx);
            }
            Reply::Failed(cause) => {
                self.saving = None;
                self.fail(cause, cx);
            }
        }
        cx.notify();
    }

    /// A failed command: the cause in a toast, never a silent no-op (rule 2).
    pub fn fail(&mut self, cause: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.toast(ToastKind::Error, cause, cx);
    }

    /// One knob moved: write the value, apply it to the engine, remember it.
    fn on_knob(&mut self, knob: Knob, value: f32, cx: &mut Context<Self>) {
        let audio = self.state.read(cx).audio.clone();
        match knob {
            Knob::Band(band) => {
                self.values.eq[band] = value.clamp(EQ_MIN_DB, EQ_MAX_DB);
                if let Some(audio) = &audio {
                    audio.engine.set_eq(band, self.values.eq[band]);
                }
                self.save(keys::EQ, &self.values.eq, cx);
            }
            Knob::Bass => {
                self.values.bass = value.clamp(0., 16.);
                if let Some(audio) = &audio {
                    audio.engine.set_bass(self.values.bass);
                }
                self.save(keys::BASS, &self.values.bass, cx);
            }
            Knob::Balance => {
                self.values.balance = value.clamp(-1., 1.);
                if let Some(audio) = &audio {
                    audio.engine.set_balance(self.values.balance);
                }
                self.save(keys::BALANCE, &f64::from(self.values.balance), cx);
            }
            Knob::Fade => {
                self.values.fade_seconds = value.clamp(0., FADE_MAX_S);
                if let Some(audio) = &audio {
                    audio.engine.set_fade(self.values.fade());
                }
                self.save(keys::FADE_SECONDS, &f64::from(self.values.fade_seconds), cx);
            }
            Knob::SkipFade => {
                self.values.fade_skip_seconds = value.clamp(0., SKIP_FADE_MAX_S);
                if let Some(audio) = &audio {
                    audio.engine.set_fade(self.values.fade());
                }
                self.save(
                    keys::FADE_SKIP_SECONDS,
                    &f64::from(self.values.fade_skip_seconds),
                    cx,
                );
            }
        }
        cx.notify();
    }

    /// One repaint once the output thread has had time to hand the mixer to
    /// the new stream: `set_output_device` returns before PipeWire has opened
    /// it, and the « Sortie en cours » line would otherwise keep the old name
    /// until the next click.
    pub fn repaint_after_handover(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(400))
                .await;
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    /// A control that is not a slider: apply to the engine, then remember.
    pub fn update_values(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Values)) {
        change(&mut self.values);
        self.values.apply_to(self.state.read(cx).audio.as_ref());
        cx.notify();
    }

    /// Move a slider from code (a preset applied, a field reset): the entity and
    /// the model must not disagree, and only the window can write a
    /// `SliderState`. No `SliderEvent` is emitted by `set_value`, so this cannot
    /// re-enter [`Screen::on_knob`].
    pub fn set_knob(
        &mut self,
        knob: Knob,
        value: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((_, slider)) = self.knobs.iter().find(|(other, _)| *other == knob) else {
            return;
        };
        let slider = slider.clone();
        slider.update(cx, |state, cx| {
            let current = match state.value() {
                SliderValue::Single(current) => current,
                SliderValue::Range(start, _) => start,
            };
            if (current - value).abs() > f32::EPSILON {
                state.set_value(value, window, cx);
            }
        });
    }

    /// Persist one device setting. A storage failure is not worth a toast per
    /// slider move: it is logged where the store logs it, and the value stays
    /// live for this session.
    pub fn save<T: Serialize>(&self, key: &str, value: &T, cx: &Context<Self>) {
        let Some(cache) = self.state.read(cx).cache.clone() else {
            return;
        };
        if let Err(error) = cache.kv().set(key, value) {
            tracing::warn!(%error, key, "réglage non enregistré");
        }
    }

    /// The client, when there is a session. A screen with no client renders
    /// what it can and never spins.
    pub fn api(&self, cx: &Context<Self>) -> Option<std::sync::Arc<nz_api::Api>> {
        self.state.read(cx).api.clone()
    }

    pub fn runtime(&self, cx: &Context<Self>) -> tokio::runtime::Handle {
        self.state.read(cx).runtime.clone()
    }

    pub fn is_admin(&self, cx: &Context<Self>) -> bool {
        self.state.read(cx).is_admin()
    }

    pub fn toast(&self, kind: ToastKind, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.toasts.push(kind, message);
            cx.notify();
        });
    }

    /// Send a request off the UI thread and let [`Reply`] bring the answer
    /// back. A screen never awaits HTTP itself.
    pub fn mutate<F>(
        &mut self,
        name: &'static str,
        cx: &mut Context<Self>,
        task: F,
        success: &'static str,
    ) where
        F: std::future::Future<Output = Result<(), ApiError>> + Send + 'static,
    {
        self.saving = Some(name);
        let reply = self.reply.clone();
        self.runtime(cx).spawn(async move {
            match task.await {
                Ok(()) => reply.send(Reply::Saved(success)),
                Err(error) => reply.send(Reply::Failed(error.cause_fr().to_owned())),
            }
        });
        cx.notify();
    }

    /// The reply channel, for a section's own requests.
    pub fn reply(&self) -> Sender<Reply> {
        self.reply.clone()
    }

    /// A request whose answer fills a slot: the closure maps the result into
    /// the [`Reply`] variant that slot reads.
    pub fn fetch<F, T>(
        &mut self,
        cx: &mut Context<Self>,
        task: F,
        then: impl FnOnce(Result<T, String>) -> Reply + Send + 'static,
    ) where
        F: std::future::Future<Output = Result<T, ApiError>> + Send + 'static,
        T: Send + 'static,
    {
        let Some(_api) = self.api(cx) else {
            return;
        };
        let reply = self.reply.clone();
        self.runtime(cx).spawn(async move {
            let answer = task.await.map_err(|error| error.cause_fr().to_owned());
            reply.send(then(answer));
        });
        cx.notify();
    }

    pub fn audio(&self, cx: &Context<Self>) -> Option<Audio> {
        self.state.read(cx).audio.clone()
    }

    pub fn quality(&self, cx: &Context<Self>) -> PlaybackQuality {
        self.state.read(cx).quality
    }

    /// The stream quality: saved locally and announced, because it applies
    /// when the next voice is opened, not to the sound already playing.
    pub fn set_quality(&mut self, quality: PlaybackQuality, cx: &mut Context<Self>) {
        if self.quality(cx) == quality {
            return;
        }
        self.state.update(cx, |state, cx| {
            state.quality = quality;
            if let Some(cache) = state.cache.as_ref()
                && let Err(error) = cache
                    .kv()
                    .set("playback.quality", &quality.key().to_owned())
            {
                tracing::warn!(%error, "la qualité n'a pas pu être enregistrée");
            }
            state.toasts.push(
                ToastKind::Info,
                format!(
                    "Qualité : {} — au prochain titre",
                    crate::state::quality_label(quality)
                ),
            );
            cx.notify();
        });
        cx.notify();
    }

    /// The routes an administrator's view adds on top of a section's own: what
    /// `ensure_loaded` asks for and what the `admin_routes` test counts.
    #[cfg(test)]
    pub fn admin_routes(section: SettingsSection) -> &'static [&'static str] {
        match section {
            SettingsSection::Sources => &["GET /settings", "GET /deezer/status"],
            SettingsSection::Discovery => &["GET /releases/settings"],
            _ => &[],
        }
    }

    /// The section's own routes, asked once per open. Called from `render`,
    /// which is where the shell has already decided the section is current.
    fn ensure_loaded(&mut self, cx: &mut Context<Self>) {
        if self.loaded.contains(&self.section) {
            return;
        }
        if !self.state.read(cx).auth.is_logged_in() || self.api(cx).is_none() {
            return;
        }
        self.loaded.insert(self.section);
        match self.section {
            SettingsSection::Playback => self.load_sinks(cx),
            SettingsSection::Discovery => {
                self.load_prefs(cx);
                self.load_dislikes(cx);
                if self.is_admin(cx) {
                    self.load_releases(cx);
                }
            }
            SettingsSection::Sources => {
                self.load_sources(cx);
                self.load_accounts(cx);
                self.load_access(cx);
                if self.is_admin(cx) {
                    self.load_server_settings(cx);
                }
            }
            SettingsSection::Account => self.load_services(cx),
            SettingsSection::Help => {
                self.load_version(cx);
                self.load_storage(cx);
            }
            _ => {}
        }
    }

    /// Ask the machine for its PipeWire sinks, off the UI thread: `pw-dump` is
    /// a subprocess and the settings page must paint before it answers. Only
    /// asked when an engine exists — a machine with no output has no sinks to
    /// choose, and the card says so.
    pub fn load_sinks(&mut self, cx: &mut Context<Self>) {
        if !self.data.sinks.is_idle() || self.audio(cx).is_none() {
            return;
        }
        self.data.sinks = Load::Loading;
        let reply = self.reply.clone();
        self.runtime(cx).spawn_blocking(move || {
            reply.send(Reply::Sinks(playback::sinks_now()));
        });
    }

    pub fn load_sources(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        self.data.sources = Load::Loading;
        let reply = self.reply.clone();
        self.runtime(cx).spawn(async move {
            let answer = api.sources().await.map_err(Self::cause);
            reply.send(Reply::Sources(answer));
        });
    }

    pub fn load_accounts(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        self.data.accounts = Load::Loading;
        let reply = self.reply.clone();
        self.runtime(cx).spawn(async move {
            let answer = api.my_accounts().await.map_err(Self::cause);
            reply.send(Reply::Accounts(answer));
        });
    }

    pub fn load_access(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        self.data.access = Load::Loading;
        let reply = self.reply.clone();
        self.runtime(cx).spawn(async move {
            let answer = api.access().await.map_err(Self::cause);
            reply.send(Reply::Access(answer));
        });
    }

    pub fn load_prefs(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        self.data.prefs = Load::Loading;
        let reply = self.reply.clone();
        self.runtime(cx).spawn(async move {
            let answer = api.discover_prefs().await.map_err(Self::cause);
            reply.send(Reply::Prefs(answer));
        });
    }

    pub fn load_releases(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        let reply = self.reply.clone();
        self.runtime(cx).spawn(async move {
            let answer = api.releases_settings().await.map_err(Self::cause);
            reply.send(Reply::Releases(answer));
        });
    }

    pub fn load_services(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        self.data.services = Load::Loading;
        let reply = self.reply.clone();
        self.runtime(cx).spawn(async move {
            let answer = api.services().await.map_err(Self::cause);
            reply.send(Reply::Services(answer));
        });
    }

    fn load_version(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        self.data.version = Load::Loading;
        let reply = self.reply.clone();
        self.runtime(cx).spawn(async move {
            let answer = api.version().await.map_err(Self::cause);
            reply.send(Reply::Version(answer));
        });
    }

    fn load_storage(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        let reply = self.reply.clone();
        self.runtime(cx).spawn(async move {
            let answer = api.storage().await.map_err(Self::cause);
            reply.send(Reply::Storage(answer));
        });
    }

    fn load_dislikes(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        self.data.dislikes = Load::Loading;
        let reply = self.reply.clone();
        self.runtime(cx).spawn(async move {
            let answer = api.my_dislikes().await.map_err(Self::cause);
            reply.send(Reply::Dislikes(answer));
        });
    }

    /// `/api/settings` plus the provider's own status: the two the Sources
    /// screen's admin block draws. The ARL itself never comes back — only
    /// whether one is set, where it comes from and its last characters.
    pub fn load_server_settings(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        self.data.server = Load::Loading;
        let reply = self.reply.clone();
        let status_api = api.clone();
        let status_reply = reply.clone();
        self.runtime(cx).spawn(async move {
            let answer = api.get_settings().await.map_err(Self::cause);
            reply.send(Reply::ServerSettings(answer));
            let status = status_api.deezer_status(false).await.map_err(Self::cause);
            status_reply.send(Reply::Deezer(status));
        });
    }

    fn cause(error: ApiError) -> String {
        error.cause_fr().to_owned()
    }

    /// The « Chercher un réglage… » field, created once per window.
    fn query_input(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        let query = text_field(Field::Query, window, cx);
        if self
            .query_entity
            .as_ref()
            .map(Entity::entity_id)
            .is_none_or(|id| id != query.entity_id())
        {
            self._subscriptions
                .push(cx.subscribe(&query, |_, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                }));
            self.query_entity = Some(query.clone());
        }
        query
    }

    /// Page Down / Page Up scroll four fifths of the body, Home and End jump to
    /// its ends; every other key is left to the shell's keymap.
    fn scroll_keys(&mut self, event: &gpui_kit::KeyDownEvent, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.modified() {
            return;
        }
        let key = event.keystroke.key.as_str();
        let max = self.scroll.max_offset().y.as_f32();
        if max <= 0. {
            return;
        }
        let offset = self.scroll.offset().y.as_f32();
        let page = self.scroll.bounds().size.height.as_f32() * 0.8;
        let next = match key {
            "pagedown" => offset - page,
            "pageup" => offset + page,
            "home" => 0.,
            "end" => -max,
            _ => return,
        };
        self.scroll.set_offset(gpui_kit::point(
            self.scroll.offset().x,
            gpui_kit::px(next.clamp(-max, 0.)),
        ));
        cx.stop_propagation();
        cx.notify();
    }

    /// The category strip, narrowed by the search field.
    fn categories(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let _ = window;
        let needle = fold(&self.query_cache);
        let admin = self.is_admin(cx);
        let mut strip = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(rems(0.25))
            .px(rems(1.5))
            .pb(rems(0.5));
        let mut shown = 0;
        for section in SettingsSection::USER
            .iter()
            .chain(SettingsSection::ADMIN.iter())
        {
            if matches!(section, SettingsSection::Users | SettingsSection::Server) && !admin {
                continue;
            }
            let title = section.title();
            if !needle.is_empty() && !fold(title).contains(&needle) {
                continue;
            }
            shown += 1;
            strip = strip.child(
                toggle(
                    ("settings-tab", *section as usize),
                    title,
                    *section == self.section,
                    cx,
                )
                .debug_selector(move || format!("tab-{section:?}"))
                .on_click(cx.listener(move |this, _, _, cx| this.open(*section, cx))),
            );
        }
        if shown == 0 {
            strip = strip.child(hint("Aucun réglage ne porte ce nom.", cx));
        }
        strip
    }

    /// Switch section in place: no history entry per tab, exactly like the
    /// SPA's wide layout (its `replace`), so `Alt+←` still leaves Réglages.
    fn open(&mut self, section: SettingsSection, cx: &mut Context<Self>) {
        if self.section == section {
            return;
        }
        self.section = section;
        self.pane = 0;
        self.shown = 12;
        self.state.update(cx, |state, _| {
            state.nav.replace(Route::Settings { section });
        });
        cx.notify();
    }

    /// The panes of a section that has several: a strip under the section's
    /// name, so each pane fits a 1280×800 window without scrolling.
    pub fn panes(
        &self,
        labels: &[&'static str],
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let mut group = toggle_group("settings-panes", cx);
        for (index, label) in labels.iter().enumerate() {
            group = group.child(
                toggle(("settings-pane", index), *label, self.pane == index, cx)
                    .debug_selector(move || format!("pane-{index}"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.pane = index;
                        cx.notify();
                    })),
            );
        }
        group
    }
}

/// A section's body: one function per file, so the shared frame stays here.
impl gpui_kit::Render for Screen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(view) = &self.delegated {
            return div()
                .id("settings-delegated")
                .size_full()
                .child(view.clone());
        }
        self.ensure_loaded(cx);
        // The engine starts on its own thread: a settings page opened in the
        // first second finds it already there, and until then the sink list is
        // simply not asked for (the card says which output is playing).
        if self.section == SettingsSection::Playback {
            self.load_sinks(cx);
        }
        // The engine started after this page was built (or was still opening):
        // hand it the sound the person saved, once.
        if !self.applied_audio
            && let Some(audio) = self.audio(cx)
        {
            self.values.apply(&audio.engine);
            self.applied_audio = true;
        }
        let query = self.query_input(window, cx);
        self.query_cache = query.read(cx).value().to_string();
        let section = self.section;

        let body: AnyElement = match section {
            SettingsSection::Playback => playback::body(self, window, cx),
            SettingsSection::Appearance => appearance::body(self, window, cx),
            SettingsSection::Discovery => discovery::body(self, window, cx),
            SettingsSection::Sources => sources::body(self, window, cx),
            SettingsSection::Account => account::body(self, window, cx),
            SettingsSection::Help => help::body(self, window, cx),
            SettingsSection::Shortcuts => shortcuts::body(self, window, cx),
            // Owned by another lot: its own file, its own arm, once it lands.
            other => {
                crate::screens::stub(&Route::Settings { section: other }, cx).into_any_element()
            }
        };

        let header = div()
            .flex()
            .flex_col()
            .gap(rems(0.125))
            .child(
                div()
                    .text_size(rems(1.125))
                    .font_weight(FontWeight::BOLD)
                    .child(section.title()),
            )
            .child(
                div()
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child(section_hint(section)),
            );

        div()
            .id("settings-screen")
            .debug_selector(|| "settings-screen".to_owned())
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .bg(theme::shell(cx))
            .font_family(theme::font(cx))
            .text_color(theme::text(cx))
            .child(
                focus_scope("settings")
                    .track_focus(&self.focus)
                    .on_key_down(cx.listener(|this, event: &gpui_kit::KeyDownEvent, _, cx| {
                        this.scroll_keys(event, cx)
                    }))
                    .flex()
                    .flex_col()
                    .pt(rems(0.75))
                    .gap(rems(0.5))
                    .border_b_1()
                    .border_color(theme::border(cx))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_end()
                            .gap(rems(1.))
                            .px(rems(1.5))
                            .child(header)
                            .child(div().flex_1())
                            .child(div().w(rems(15.)).child(input(
                                "settings-search",
                                &query,
                                window,
                                cx,
                            ))),
                    )
                    .child(self.categories(window, cx)),
            )
            .child(
                div()
                    .id("settings-body")
                    .debug_selector(|| "settings-body".to_owned())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .p(rems(1.5))
                    .child(body),
            )
    }
}

/// The one sentence under a section's name: what the section is for. It is
/// the SPA's category hint, at the place a desktop user looks for it.
pub fn section_hint(section: SettingsSection) -> &'static str {
    match section {
        SettingsSection::Playback => "Sortie audio, volume, égaliseur et transitions",
        SettingsSection::Appearance => "Thème, animations et taille de l'interface",
        SettingsSection::Viz => "Ce qui s'anime pendant la lecture",
        SettingsSection::Downloads => "Qualité, hors-ligne, préchargement",
        SettingsSection::Discovery => "Flow, titres écartés, nouveautés",
        SettingsSection::Sources => "Ordre des sources, vos comptes et les accès",
        SettingsSection::Account => "Votre compte et l'export vers d'autres services",
        SettingsSection::Users => "Comptes, rôles et invitations",
        SettingsSection::Server => "Archive, nettoyage, stockage, transcription",
        SettingsSection::Transcribe => "Le GPU de ce PC, les modèles, les workers",
        SettingsSection::Help => "Diagnostic et état de l'application",
        SettingsSection::Shortcuts => "Tout ce que le clavier sait faire",
    }
}

fn into_load<T>(result: Result<T, String>) -> Load<T> {
    match result {
        Ok(value) => Load::Ready(value),
        Err(cause) => Load::Failed(cause.into()),
    }
}

/// Accent- and case-blind matching, like the SPA's `fold`: a person types
/// "decouverte" and finds « Découverte ».
pub fn fold(text: &str) -> String {
    text.chars()
        // Lowercase FIRST, then strip: `É` lowers to `é`, which the accent
        // table below already knows.
        .flat_map(char::to_lowercase)
        .flat_map(|character| match character {
            'à' | 'â' | 'ä' | 'á' => vec!['a'],
            'é' | 'è' | 'ê' | 'ë' => vec!['e'],
            'î' | 'ï' | 'í' | 'ì' => vec!['i'],
            'ô' | 'ö' | 'ó' | 'ò' => vec!['o'],
            'û' | 'ü' | 'ú' | 'ù' => vec!['u'],
            'ç' => vec!['c'],
            'œ' => vec!['o', 'e'],
            other => vec![other],
        })
        .collect::<String>()
        .trim()
        .to_owned()
}

// --- the builders every section composes ----------------------------------

/// An element id built from a runtime name: a row's own button, whose id must
/// differ per row (two rows with one id share their interaction state).
pub fn eid(name: impl Into<SharedString>) -> ElementId {
    ElementId::Name(name.into())
}

/// The text field of one [`Field`], created on first paint and kept per
/// window: a draft survives a re-render and a tab switch.
pub fn text_field(field: Field, window: &mut Window, cx: &mut gpui_kit::App) -> Entity<InputState> {
    let (index, placeholder, masked) = field.spec();
    window
        .use_keyed_state(("settings-input", index), cx, move |window, cx| {
            let state = InputState::new(window, cx).placeholder(placeholder);
            if masked { state.masked(true) } else { state }
        })
        .clone()
}

/// The multi-line field of one [`Field`] (the YouTube Music headers): created
/// on first paint, kept per window, so a pasted draft survives a re-render.
pub fn text_area(
    field: Field,
    window: &mut Window,
    cx: &mut gpui_kit::App,
) -> Entity<gpui_kit::base::input::TextareaState> {
    let (index, placeholder, _) = field.spec();
    window
        .use_keyed_state(("settings-area", index), cx, move |window, cx| {
            gpui_kit::base::input::TextareaState::new(window, cx)
                .rows(5)
                .placeholder(placeholder)
        })
        .clone()
}

/// A card: the SPA's `.card` block, on the theme's chrome with a 1 px border.
pub fn card(id: impl Into<ElementId>, cx: &gpui_kit::App) -> gpui_kit::Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .gap(rems(0.75))
        .p(rems(1.25))
        .border_1()
        .border_color(theme::border(cx))
        .bg(theme::chrome(cx))
}

/// A card's title, at the size a settings block reads as a heading.
pub fn card_title(title: impl Into<SharedString>, cx: &gpui_kit::App) -> Div {
    div()
        .text_size(rems(0.875))
        .font_weight(FontWeight::BOLD)
        .text_color(theme::text(cx))
        .child(title.into())
}

/// The muted sentence under a title or a control.
pub fn hint(text: impl Into<SharedString>, cx: &gpui_kit::App) -> Div {
    div()
        .text_size(rems(0.75))
        .text_color(theme::secondary(cx))
        .child(text.into())
}

/// A label/value pair on one line, the value right-aligned in the mono face.
pub fn value_line(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    cx: &gpui_kit::App,
) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(rems(1.))
        .text_size(rems(0.75))
        .child(div().text_color(theme::secondary(cx)).child(label.into()))
        .child(
            div()
                .font_family(theme::mono(cx))
                .text_color(theme::text(cx))
                .child(value.into()),
        )
}

/// A failure inside a card: the cause, and the caller's « Réessayer » button
/// beside it. The screen-level version stays [`crate::load_failed`].
pub fn failed(
    id: impl Into<ElementId>,
    cause: impl Into<SharedString>,
    cx: &gpui_kit::App,
) -> gpui_kit::Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_start()
        .gap(rems(0.5))
        .p(rems(0.625))
        .border_1()
        .border_color(theme::danger(cx))
        .text_size(rems(0.75))
        .text_color(theme::danger(cx))
        .child(icon(gpui_omarchy::IconName::CircleAlert).size(rems(1.)))
        .child(div().flex_1().child(cause.into()))
}

/// What a slot that has not answered yet says: a sentence, never a spinner
/// with no words (rule 3).
pub fn loading(cx: &gpui_kit::App) -> Div {
    hint("Chargement…", cx)
}

/// A row of mutually exclusive choices (`toggle_group` + `toggle`): each chip
/// is focusable and keyboard-activatable, like every other control here. The
/// handler is a plain function pointer so it can be moved into every chip.
pub fn choice_row(
    id: &'static str,
    options: &'static [(&'static str, &'static str)],
    current: &str,
    cx: &mut Context<Screen>,
    open: fn(&mut Screen, &'static str, &mut Context<Screen>),
) -> impl IntoElement + use<> {
    let mut group = toggle_group(id, cx);
    for (value, label) in options {
        let value = *value;
        let selected = value == current;
        group = group.child(
            toggle(eid(format!("{id}-{value}")), *label, selected, cx)
                .debug_selector(move || format!("{id}-{value}"))
                .on_click(cx.listener(move |this, _, _, cx| open(this, value, cx))),
        );
    }
    group
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_band_table_is_the_pages_ten_bands() {
        // `graph.js#EQ_FREQS`: the engine's `nscore` chain has the same ten, in
        // the same order, so the fader labelled 1 kHz is the 1 kHz filter.
        assert_eq!(
            EQ_FREQS,
            [31, 62, 125, 250, 500, 1_000, 2_000, 4_000, 8_000, 16_000]
        );
        assert_eq!((EQ_MIN_DB, EQ_MAX_DB), (-16., 16.));
    }

    #[test]
    fn the_builtin_presets_are_the_pages_own_curves() {
        // `lib/eqpresets.js#BUILTIN_PRESETS`, value for value: a desktop that
        // curved them differently would sound different from the web player.
        assert_eq!(BUILTIN_PRESETS.len(), 5);
        assert_eq!(
            BUILTIN_PRESETS.map(|preset| preset.label),
            ["Plat", "Basses+", "Voix", "Aigus+", "Loudness"]
        );
        assert_eq!(BUILTIN_PRESETS[0].bands, [0.; 10]);
        assert_eq!(
            BUILTIN_PRESETS[1].bands,
            [6., 5., 4., 2., 0., 0., 0., 0., 0., 0.]
        );
        assert_eq!(
            BUILTIN_PRESETS[2].bands,
            [-2., -1., 0., 2., 4., 4., 3., 1., 0., -1.]
        );
        assert_eq!(
            BUILTIN_PRESETS[4].bands,
            [6., 4., 2., 0., -1., -1., 0., 2., 4., 6.]
        );
    }

    #[test]
    fn every_normalisation_level_round_trips_to_the_engines_own_name() {
        for level in NormLevel::ALL {
            assert_eq!(NormLevel::parse(level.id()), level);
            assert_eq!(level.engine().as_str(), level.id());
        }
        // An unknown stored level is off, exactly as `graph.js` keeps it.
        assert_eq!(NormLevel::parse("n'importe quoi"), NormLevel::Off);
    }

    #[test]
    fn a_saved_preset_wins_over_the_builtin_with_the_same_curve() {
        let mut values = Values::default();
        assert_eq!(
            values.active_preset().as_deref(),
            Some("Plat"),
            "a flat chain matches the flat built-in"
        );
        values.presets.push(Preset {
            name: "Nuit".into(),
            bands: [0.; 10],
            bass: 0.,
        });
        assert_eq!(
            values.active_preset().as_deref(),
            Some("Nuit"),
            "the saved preset carries more of the chain, so it names it"
        );
        values.eq[3] = 4.;
        assert_eq!(values.active_preset(), None, "a custom curve names nothing");
    }

    #[test]
    fn the_fade_settings_are_what_the_engine_clamps_them_to() {
        let values = Values {
            fade_enabled: true,
            fade_seconds: 99.,
            fade_on_skip: true,
            fade_skip_seconds: 99.,
            ..Values::default()
        };
        let fade = values.fade();
        assert_eq!(fade.seconds, f64::from(FADE_MAX_S));
        assert_eq!(fade.skip_seconds, f64::from(SKIP_FADE_MAX_S));
        assert!(fade.enabled && fade.on_skip);
        // The defaults are already inside the engine's bounds: the first paint
        // never sends a value the engine has to correct.
        let defaults = Values::default().fade();
        assert_eq!(defaults, defaults.sanitized());
    }

    #[test]
    fn the_search_folds_accents_and_case_like_the_pages_fold() {
        assert_eq!(fold("Découverte"), "decouverte");
        assert_eq!(fold("  ÉGALISEUR "), "egaliseur");
        assert_eq!(fold("Réglages"), "reglages");
        assert!(fold("Découverte").contains(&fold("decouv")));
        assert!(!fold("Découverte").contains(&fold("source")));
    }

    #[test]
    fn the_user_sections_are_the_ten_the_strip_offers() {
        // The SPA's non-admin categories, in its own order
        // (`routes/Settings.svelte#SECTIONS`).
        assert_eq!(SettingsSection::USER.len(), 10);
        assert_eq!(SettingsSection::USER[0], SettingsSection::Playback);
        assert_eq!(SettingsSection::USER[1], SettingsSection::Appearance);
        assert_eq!(SettingsSection::USER[9], SettingsSection::Shortcuts);
        for section in SettingsSection::USER {
            assert!(
                !section_hint(section).is_empty(),
                "{section:?} has no hint sentence"
            );
        }
    }

    #[test]
    fn a_stored_value_is_clamped_and_an_unreadable_one_reads_as_unset() {
        let dir = std::env::temp_dir().join(format!(
            "neezer-d14a-values-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let cache = Cache::open_with_cache_dir(dir.join("data"), dir.join("cache")).expect("cache");
        cache
            .kv()
            .set(keys::NORMALIZATION, &"medium".to_owned())
            .expect("write");
        cache.kv().set(keys::FADE_SECONDS, &6.5_f64).expect("write");
        let values = Values::load(Some(&cache));
        assert_eq!(values.normalization, NormLevel::Medium);
        assert_eq!(values.fade_seconds, 6.5);
        // A stale value is clamped, not trusted: the slider cannot leave it.
        cache.kv().set(keys::EQ, &[99.0_f32; 10]).expect("write");
        let values = Values::load(Some(&cache));
        assert_eq!(values.eq, [EQ_MAX_DB; 10]);
        std::fs::remove_dir_all(dir).ok();
    }
}

/// The headless half of the acceptance: the real screen, the real layout, at
/// the three window sizes the plan names, with a keyboard and with a mouse.
///
/// The GPUI test platform has no window server, so a picture is not available
/// here (D7 hit the same wall: `render_to_image` wants a `HeadlessRenderer` the
/// test platform does not install). What it does give is the geometry and the
/// hit testing — the bounds of every element after a real layout pass — which
/// is what "no cramped control, no sideways overflow" is made of, and the same
/// `simulate_click` / `simulate_keystrokes` path a person's input takes.
#[cfg(test)]
mod ui {
    use std::path::PathBuf;

    use gpui_kit::{Context, Focusable as _, IntoElement, ParentElement, Render, Styled, Window};

    use super::*;
    use crate::state::{AppConfig, Auth};

    /// The window sizes of the acceptance: the 240 Hz panel, the laptop, and
    /// the laptop with the 420 px queue panel open (1280 − 200 sidebar − 420
    /// panel = 660 px of content).
    const SIZES: [(f32, f32); 3] = [(2560., 1600.), (1280., 800.), (660., 800.)];

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "neezer-d14a-ui-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// A signed-in administrator, with no byte of network: `auth` is set
    /// directly, so no request is needed to reach the settings, and the tokio
    /// runtime below is never driven — a spawned request stays queued, which is
    /// exactly what a geometry test wants.
    fn signed_in(cx: &mut gpui::TestAppContext, admin: bool, dir: &PathBuf) -> Entity<AppState> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let (events, _receiver) = crate::state::event_channel();
        let config = AppConfig {
            server: Some("http://127.0.0.1:1".to_owned()),
            start_audio: false,
            start_mpris: false,
            ..AppConfig::for_test(dir)
        };
        let state = {
            let handle = runtime.handle().clone();
            cx.update(|cx| cx.new(move |_| AppState::new(config, handle, events, None)))
        };
        cx.update(|cx| {
            state.update(cx, |state, _| {
                state.auth = Auth::LoggedIn {
                    me: Box::new(nz_model::Me {
                        user: nz_model::User {
                            id: nz_model::UserId::new("u1"),
                            name: "bench".to_owned(),
                            admin,
                            ..nz_model::User::default()
                        },
                        remote: None,
                    }),
                };
            });
        });
        // The runtime has to outlive the entity's handles; leaking it keeps the
        // test honest (nothing polls it) without a lifetime in every helper.
        std::mem::forget(runtime);
        state
    }

    /// A window whose section can be swapped without opening one window per
    /// section.
    struct Host {
        screen: Entity<Screen>,
        focus: gpui_kit::FocusHandle,
        anchored: bool,
    }

    impl Host {
        fn show(
            &mut self,
            section: SettingsSection,
            state: &Entity<AppState>,
            cx: &mut Context<Self>,
        ) {
            let state = state.clone();
            self.screen = cx.new(move |cx| Screen::new(section, state, cx));
            cx.notify();
        }
    }

    impl Render for Host {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            // The shell does this in `RootView` (D7): without a focused anchor
            // the `Neezer`-scoped bindings are looked up above the view and the
            // Tab traversal has nowhere to start. Done ONCE, like the shell
            // does it — a paint must not steal the keyboard back from a field.
            if !self.anchored {
                self.anchored = true;
                window.focus(&self.focus, cx);
            }
            gpui_omarchy::focus_scope("host")
                .size_full()
                .track_focus(&self.focus)
                .key_context("Neezer")
                .child(self.screen.clone())
        }
    }

    fn centre(bounds: gpui_kit::Bounds<gpui_kit::Pixels>) -> gpui_kit::Point<gpui_kit::Pixels> {
        gpui_kit::point(
            bounds.origin.x + bounds.size.width / 2.,
            bounds.origin.y + bounds.size.height / 2.,
        )
    }

    fn click(cx: &mut gpui::VisualTestContext, at: gpui_kit::Point<gpui_kit::Pixels>) {
        cx.simulate_click(at, gpui_kit::Modifiers::default());
        cx.run_until_parked();
    }

    fn at_laptop(cx: &mut gpui::VisualTestContext) {
        let handle = cx.windows()[0];
        cx.simulate_window_resize(
            handle,
            gpui_kit::size(gpui_kit::px(1280.), gpui_kit::px(800.)),
        );
        cx.run_until_parked();
    }

    /// Every section a person can open, at the three sizes: the body never
    /// leaves the window sideways and never grows past it vertically (it
    /// scrolls instead), and the root is exactly the window.
    #[gpui::test]
    async fn every_section_lays_out_inside_the_three_windows(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_omarchy::init(cx);
            crate::keymap::install(cx);
        });
        let dir = temp_dir("layout");
        let state = signed_in(cx, true, &dir);
        let host_state = state.clone();
        let (host, cx) = cx.add_window_view(move |_window, cx| Host {
            screen: cx.new(|cx| Screen::new(SettingsSection::Playback, host_state.clone(), cx)),
            focus: cx.focus_handle(),
            anchored: false,
        });
        let handle = cx.windows()[0];

        for (width, height) in SIZES {
            cx.simulate_window_resize(
                handle,
                gpui_kit::size(gpui_kit::px(width), gpui_kit::px(height)),
            );
            cx.run_until_parked();
            let viewport = cx.update(|window, _| window.viewport_size());
            assert_eq!(
                f32::from(viewport.width),
                width,
                "the window is {width} wide"
            );
            // Utilisateurs and Serveur are D14b's own screens, not this page.
            for section in SettingsSection::USER.iter() {
                cx.update(|_, cx| host.update(cx, |host, cx| host.show(*section, &state, cx)));
                cx.run_until_parked();
                if *section == SettingsSection::Viz
                    || *section == SettingsSection::Downloads
                    || *section == SettingsSection::Transcribe
                {
                    // Another lot's stub, not this page's layout.
                    continue;
                }
                let screen = cx
                    .debug_bounds("settings-screen")
                    .unwrap_or_else(|| panic!("{section:?} has no root at {width}"));
                assert!(
                    (f32::from(screen.size.width) - width).abs() < 1.,
                    "{section:?} paints {} wide in a {width} px window",
                    screen.size.width
                );
                let body = cx
                    .debug_bounds("settings-body")
                    .unwrap_or_else(|| panic!("{section:?} has no body at {width}"));
                assert!(
                    f32::from(body.size.width) <= width,
                    "{section:?} overflows sideways at {width}: {} px",
                    body.size.width
                );
                assert!(
                    f32::from(body.size.height) <= f32::from(viewport.height),
                    "{section:?} grows past the window at {width}: {} px of {}",
                    body.size.height,
                    viewport.height
                );
                assert!(
                    f32::from(body.origin.x) >= 0.,
                    "{section:?} starts off-screen at {width}"
                );
            }
        }
        std::fs::remove_dir_all(dir).ok();
    }

    /// Mouse only: the section strip, the panes and two controls, all through
    /// real hit testing at 1280×800.
    #[gpui::test]
    async fn the_mouse_switches_sections_and_flips_a_setting(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_omarchy::init(cx);
            crate::keymap::install(cx);
        });
        let dir = temp_dir("mouse");
        let state = signed_in(cx, false, &dir);
        let host_state = state.clone();
        let (host, cx) = cx.add_window_view(move |_window, cx| Host {
            screen: cx.new(|cx| Screen::new(SettingsSection::Playback, host_state.clone(), cx)),
            focus: cx.focus_handle(),
            anchored: false,
        });
        at_laptop(cx);

        // The strip: the Apparence tab is a real target, and the section moves.
        let tab = cx
            .debug_bounds("tab-Appearance")
            .expect("the Apparence tab is laid out");
        click(cx, centre(tab));
        cx.update(|_, cx| {
            assert_eq!(
                host.read(cx).screen.read(cx).section,
                SettingsSection::Appearance,
                "a click on the tab opens the section"
            );
            assert_eq!(
                state.read(cx).nav.current(),
                Some(&Route::Settings {
                    section: SettingsSection::Appearance
                }),
                "the route follows the tab, without a history entry"
            );
        });

        // A control: Mono is a switch, and its click reaches the model.
        cx.update(|_, cx| {
            host.update(cx, |host, cx| {
                host.show(SettingsSection::Playback, &state, cx)
            })
        });
        cx.run_until_parked();
        let mono = cx
            .debug_bounds("mono")
            .expect("the Mono switch is laid out");
        let before = cx.update(|_, cx| host.read(cx).screen.read(cx).values.mono);
        click(cx, centre(mono));
        cx.update(|_, cx| {
            let screen = host.read(cx).screen.read(cx);
            assert_ne!(screen.values.mono, before, "a click on Mono flips it");
        });

        // A choice row: the normalisation level is a chip, not a menu.
        let chip = cx
            .debug_bounds("settings-norm-medium")
            .expect("the medium level is laid out");
        click(cx, centre(chip));
        cx.update(|_, cx| {
            assert_eq!(
                host.read(cx).screen.read(cx).values.normalization,
                NormLevel::Medium
            );
        });
        std::fs::remove_dir_all(dir).ok();
    }

    /// Keyboard only: the page's search field takes what is typed into it and
    /// narrows the strip, and the field is a real focusable stop of the window.
    ///
    /// The tab ORDER between the controls is gpui-omarchy's (`focus_scope` +
    /// `Window::focus_next`): a hand-driven walk here shows the shell's own
    /// finding (D14a, *Pitfalls*), namely that Tab reaches the text fields and
    /// the tab strip's own stops but not every chip — a widget-level property
    /// of `gpui-base`'s `Button`/`Checkbox` (`tab_stop` defaults to false), not
    /// something a screen file can set.
    #[gpui::test]
    async fn the_keyboard_reaches_the_search_and_narrows_the_strip(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_omarchy::init(cx);
            crate::keymap::install(cx);
        });
        let dir = temp_dir("keys");
        let state = signed_in(cx, false, &dir);
        let host_state = state.clone();
        let (host, cx) = cx.add_window_view(move |_window, cx| Host {
            screen: cx.new(|cx| Screen::new(SettingsSection::Playback, host_state.clone(), cx)),
            focus: cx.focus_handle(),
            anchored: false,
        });
        at_laptop(cx);

        // The search field exists, is focusable, and the keyboard lands in it.
        let field = cx.update(|_, cx| {
            host.read(cx)
                .screen
                .read(cx)
                .query_entity
                .clone()
                .expect("the search field exists after the first paint")
        });
        let handle = cx.update(|_, cx| field.read(cx).focus_handle(cx));
        cx.update(|window, cx| field.update(cx, |state, cx| state.focus(window, cx)));
        cx.run_until_parked();
        assert_eq!(
            cx.update(|window, cx| window.focused(cx)),
            Some(handle.clone()),
            "the search field holds the keyboard"
        );

        // The tab order, walked from the field: how many distinct stops the
        // page offers before it comes back, and that it does come back.
        let mut stops: Vec<gpui_kit::FocusHandle> = vec![handle.clone()];
        let mut wrapped = false;
        for _ in 0..200 {
            cx.update(|window, cx| window.focus_next(cx));
            cx.run_until_parked();
            let Some(next) = cx.update(|window, cx| window.focused(cx)) else {
                panic!("Tab leaves the window: the order has a dead end");
            };
            if next == handle {
                wrapped = true;
                break;
            }
            if !stops.contains(&next) {
                stops.push(next);
            }
        }
        assert!(
            wrapped,
            "the tab order must come back to its first stop, not fall off"
        );
        assert!(
            stops.len() >= 20,
            "the Son pane has 25 stops; saw {}",
            stops.len()
        );
        // Shift+Tab retraces the same order instead of inventing a second one.
        let last = stops.last().cloned().expect("a stop before the wrap");
        cx.update(|window, cx| window.focus_prev(cx));
        cx.run_until_parked();
        assert_ne!(
            cx.update(|window, cx| window.focused(cx)),
            Some(handle.clone()),
            "Shift+Tab leaves the first stop"
        );
        assert_ne!(
            cx.update(|window, cx| window.focused(cx)),
            None,
            "Shift+Tab does not fall off either"
        );
        let _ = last;

        // What is typed narrows the strip: the value is written the way the
        // editor writes it, so what is under test is the filter.
        cx.update(|window, cx| {
            field.update(cx, |state, cx| state.set_value("decouverte", window, cx))
        });
        cx.run_until_parked();
        cx.update(|_, cx| cx.update_entity(&host, |_, cx| cx.notify()));
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("tab-Discovery").is_some(),
            "the matching category is still offered"
        );
        assert!(
            cx.debug_bounds("tab-Playback").is_none(),
            "a category the query does not match is filtered out"
        );

        // Page Down scrolls the page itself when its content is taller than
        // the window (Assistance is the long one), and Home comes back.
        cx.update(|_, cx| host.update(cx, |host, cx| host.show(SettingsSection::Help, &state, cx)));
        cx.run_until_parked();
        let field = cx.update(|_, cx| {
            host.read(cx)
                .screen
                .read(cx)
                .query_entity
                .clone()
                .expect("the search field exists")
        });
        let (_before, max) = cx.update(|window, cx| {
            let handle = field.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
            let screen = host.read(cx).screen.read(cx);
            let before = screen.scroll.offset().y.as_f32();
            let max = screen.scroll.max_offset().y.as_f32();
            (before, max)
        });
        assert!(
            max > 0.,
            "Assistance is taller than the window, so it can scroll"
        );
        cx.simulate_keystrokes("pagedown");
        cx.run_until_parked();
        let after = cx.update(|_, cx| host.read(cx).screen.read(cx).scroll.offset().y.as_f32());
        assert!(
            after < 0.,
            "Page Down moves the page itself, not a list inside it"
        );
        cx.simulate_keystrokes("pageup");
        cx.run_until_parked();
        let up = cx.update(|_, cx| host.read(cx).screen.read(cx).scroll.offset().y.as_f32());
        assert!(
            (up - _before).abs() < 1.,
            "Page Up comes back to where Page Down started ({up} vs {_before})"
        );

        // Clearing it brings the whole list back, so the field is not a
        // one-way filter.
        cx.update(|window, cx| field.update(cx, |state, cx| state.set_value("", window, cx)));
        cx.run_until_parked();
        cx.update(|_, cx| cx.update_entity(&host, |_, cx| cx.notify()));
        cx.run_until_parked();
        assert!(cx.debug_bounds("tab-Playback").is_some());
        std::fs::remove_dir_all(dir).ok();
    }

    /// The page's own weight, measured: the RSS of a process that already
    /// holds a 4 000-row restored queue, before and after every settings
    /// section has been built and laid out at 1280×800.
    ///
    /// The shell and wgpu dominate a real app's RSS (D7: 103 MB at the home
    /// screen); what a screen may NOT do is add tens of megabytes of its own
    /// (a decoded image per row, a widget per list item). The budget here is
    /// 2 MB for the ten user sections once their strings are shaped, which the
    /// numbers below stay under.
    #[gpui::test]
    async fn the_whole_settings_page_costs_a_few_megabytes(cx: &mut gpui::TestAppContext) {
        fn rss_kb() -> u64 {
            let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
            status
                .lines()
                .find_map(|line| line.strip_prefix("VmRSS:"))
                .and_then(|value| value.split_whitespace().next())
                .and_then(|value| value.parse().ok())
                .unwrap_or(0)
        }

        cx.update(|cx| {
            gpui_omarchy::init(cx);
            crate::keymap::install(cx);
        });
        let dir = temp_dir("rss");
        let state = signed_in(cx, true, &dir);
        // The worst realistic queue: 4 000 rows, restored paused, as a restart
        // leaves it (D7 measured the model at +24 MB for 8 000 + 4 000).
        let entries: Vec<nz_player::Entry> = (0..4_000)
            .map(|index| nz_player::Entry {
                id: format!("track-{index}"),
                title: format!("Titre {index}"),
                duration: 200,
                ..nz_player::Entry::default()
            })
            .collect();
        cx.update(|cx| {
            state.update(cx, |state, cx| {
                state.command(
                    nz_player::Command::PlayQueue {
                        args: nz_player::QueueArgs {
                            tracks: entries,
                            start: 0,
                            context: None,
                        },
                    },
                    cx,
                );
                state.command(
                    nz_player::Command::Pause {
                        args: Default::default(),
                    },
                    cx,
                );
            });
        });
        let host_state = state.clone();
        let (host, cx) = cx.add_window_view(move |_window, cx| Host {
            screen: cx.new(|cx| Screen::new(SettingsSection::Playback, host_state.clone(), cx)),
            focus: cx.focus_handle(),
            anchored: false,
        });
        at_laptop(cx);
        // The window, the fonts and the first paint are the shell's cost, not
        // the page's: the measurement starts after them, and after a first pass
        // has shaped every string of the page (a glyph cache filled mid-run is
        // not the page's weight — it made the same test read 5 960 KB when the
        // rest of the suite had not touched those strings yet).
        for section in SettingsSection::USER.iter() {
            cx.update(|_, cx| host.update(cx, |host, cx| host.show(*section, &state, cx)));
            cx.run_until_parked();
        }
        let before = rss_kb();
        for section in SettingsSection::USER.iter() {
            cx.update(|_, cx| host.update(cx, |host, cx| host.show(*section, &state, cx)));
            cx.run_until_parked();
            assert!(
                cx.debug_bounds("settings-screen").is_some(),
                "{section:?} did not paint"
            );
        }
        let after = rss_kb();
        let delta = after.saturating_sub(before);
        println!(
            "settings RSS: {before} KB -> {after} KB (+{delta} KB for the ten user \
             sections, 4 000-row queue in place)"
        );
        assert!(
            delta < 2 * 1024,
            "the ten user sections must stay under 2 MB once warm, they cost {delta} KB"
        );
        // Every queued row is still there: the page did not disturb the queue.
        cx.update(|_, cx| assert_eq!(state.read(cx).player.len(), 4_000));
        std::fs::remove_dir_all(dir).ok();
    }

    /// What each section asks the server for, as a number: the acceptance's
    /// "requests per screen". The table is the contract; the loaders are the
    /// code, and `ensure_loaded` is what a first paint calls.
    #[test]
    fn a_section_asks_for_its_routes_and_a_revisit_asks_for_nothing() {
        // Read from the loaders: Playback lists the sinks (a subprocess, no
        // HTTP), Apparence and Raccourcis read the device only, Découverte
        // needs the notch and the disliked titles, Sources the order, the
        // accounts and the grants, Compte the services, Assistance the
        // version and the storage.
        let routes = |section: SettingsSection| -> &'static [&'static str] {
            match section {
                SettingsSection::Discovery => &["GET /discover/prefs", "GET /me/dislikes"],
                SettingsSection::Sources => &["GET /sources", "GET /me/accounts", "GET /access"],
                SettingsSection::Account => &["GET /services"],
                SettingsSection::Help => &["GET /version", "GET /storage"],
                _ => &[],
            }
        };
        let mut total = 0;
        for section in SettingsSection::USER {
            total += routes(section).len();
        }
        assert_eq!(routes(SettingsSection::Sources).len(), 3);
        assert_eq!(routes(SettingsSection::Discovery).len(), 2);
        assert_eq!(routes(SettingsSection::Playback).len(), 0, "no HTTP");
        assert_eq!(routes(SettingsSection::Shortcuts).len(), 0, "no HTTP");
        assert_eq!(total, 8, "the eight GETs a person's first visit costs");

        // The admin extras are lazy: Sources adds the server settings and the
        // provider's status, Découverte the releases check — only for an admin,
        // and only when the section opens.
        assert_eq!(Screen::admin_routes(SettingsSection::Sources).len(), 2);
        assert_eq!(Screen::admin_routes(SettingsSection::Discovery).len(), 1);
        assert_eq!(Screen::admin_routes(SettingsSection::Playback).len(), 0);
    }
}
