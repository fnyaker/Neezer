//! The Flow and the radios: the service that keeps a station alive, and the
//! panel that steers it.
//!
//! `lib/flow.js` (the Flow), `Player.svelte#ensureUpcoming` (the radios) and
//! `FlowPicker.svelte`/`FlowControls.svelte` (the controls) are the reference.
//! Two rules travel with them:
//!
//! - **A station never runs dry.** The player's own `Effect::NeedMore` fires
//!   when the queue holds its LAST row, which is already a gap: this service
//!   tops the queue up while [`UPCOMING_MIN`] titles still follow the playing
//!   one, so the engine always has a second voice to preload and the handover
//!   is the gapless one `nz-audio` already does.
//! - **The top-up asks once, and says why when it cannot.** One attempt in
//!   flight, [`MIN_EXTEND_MS`] between two, a `(source, queue length)` key that
//!   is never asked twice when its answer added nothing (the SPA's `exhausted`,
//!   which is what stopped a permanent request storm there), and a bounded
//!   retry on failure with the cause shown (rule 3: never a silent storm, never
//!   a silent stop).
//!
//! The service lives for the process (a GPUI global with one task on the app's
//! own entity), so a Flow started on Nouveautés keeps choosing while the person
//! reads their history — the SPA's `tick` has the same independence from the
//! screen. Its beat reads the player once a second while a station plays and
//! once every [`IDLE_BEAT_MS`] otherwise, and it never repaints on its own.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, BorrowAppContext as _, Context, Entity, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement as _, Styled,
    WeakEntity, div, px, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon, toggle, toggle_group};
use nz_api::{ApiError, RadioSeed};
use nz_cache::Cache;
use nz_model::{FlowStyle, Track};
use nz_player::{Command, Context as PlayContext};
use serde::{Deserialize, Serialize};

use crate::state::{AppState, entry_from_track};
use crate::theme;
use crate::toast::{Retry, ToastKind};
use crate::widgets::cover::{self, Covers, Kind};
use crate::wiring::events::{self, Sender};

// -- the numbers ------------------------------------------------------------

/// Titles kept AFTER the playing one. Two is what `nz-audio` needs (the next
/// voice to preload) and what makes a skip at the queue's end impossible to
/// expose — the SPA chooses 20 s before the end of the track for the same
/// reason.
pub const UPCOMING_MIN: usize = 2;
/// The floor between two top-up attempts (`Player.svelte#MIN_EXTEND_GAP`).
pub const MIN_EXTEND_MS: u128 = 5_000;
/// How many refs the exclude list carries (the server clamps at 400).
pub const EXCLUDE_MAX: usize = 400;
/// How many heard refs a session keeps (`lib/flow.js#HEARD_MAX`).
const HEARD_MAX: usize = 40;
/// How many "plus / moins comme ça" presses a session keeps.
const PRESSES_MAX: usize = 20;
/// A play under this many seconds is a skip (`Player.svelte#SKIP_UNDER`).
const SKIP_UNDER: f64 = 30.;
/// How many genres a Flow may aim at (`discover.STYLES_MAX`).
pub const STYLES_MAX: usize = 6;
/// How many titles a radio refill asks for. `POST /api/radio/next` takes up to
/// 40; eight is about 25 minutes of audio — enough that a refill is rare, small
/// enough that the next one still follows what the session heard.
pub const RADIO_BATCH: u32 = 8;
/// How many titles one Flow request asks for. The server clamps `n` to 2.
const FLOW_PICK: u8 = 1;
/// The wake period while a station plays (one cheap read a second).
pub const BEAT_MS: u64 = 1_000;
/// The wake period while nothing plays: a radio started from another screen is
/// noticed within this delay, and an idle app is left alone.
pub const IDLE_BEAT_MS: u64 = 20_000;
/// How long each retry of a failing choice waits (5 s, 15 s, 45 s): the music
/// keeps playing meanwhile, and the count is what makes it bounded.
pub const RETRY_DELAYS_MS: [u64; 3] = [5_000, 15_000, 45_000];

/// `lib/flow.js#DIVERSITY_STEPS`: the six notches of the same discovery level.
pub const DIVERSITY_STEPS: [(&str, f64, &str); 6] = [
    (
        "Prudent",
        0.,
        "Presque uniquement ce que vous écoutez déjà.",
    ),
    (
        "Proche",
        0.2,
        "Votre bibliothèque et ses voisins immédiats.",
    ),
    ("Voisin", 0.4, "Des artistes proches de vos favoris."),
    (
        "Exploration",
        0.6,
        "Des genres voisins, des artistes à deux pas.",
    ),
    ("Aventure", 0.8, "Loin de vos habitudes."),
    ("Large", 1., "Le plus d'inconnus possible, sans boussole."),
];

/// `energy.py#STEPS` as `lib/flow.js` names them: the notch a Flow starts at.
pub const ENERGY_STEPS: [(&str, Option<f64>, &str); 6] = [
    ("Libre", None, "Le Flow suit votre écoute"),
    ("Très calme", Some(0.1), "Doux, posé"),
    ("Calme", Some(0.3), "Tranquille"),
    ("Modéré", Some(0.5), "Entre les deux"),
    ("Énergique", Some(0.7), "Ça bouge"),
    ("Intense", Some(0.9), "À fond"),
];

/// The notch a level falls on: the NEAREST stop, ties upward — the rule a
/// slider's own value reads back with (`lib/flow.js#diversityIndex`).
pub fn diversity_index(level: f64) -> usize {
    let level = if level.is_finite() {
        level.clamp(0., 1.)
    } else {
        0.
    };
    let mut best = 0;
    for (index, (_, value, _)) in DIVERSITY_STEPS.iter().enumerate().skip(1) {
        if (value - level).abs() <= (DIVERSITY_STEPS[best].1 - level).abs() {
            best = index;
        }
    }
    best
}

/// The notch itself: its name, its level and its one-line hint.
pub fn diversity_step(level: f64) -> (&'static str, f64, &'static str) {
    DIVERSITY_STEPS[diversity_index(level)]
}

/// "Techno, House +1 · Voisin": what the Flow will aim at, in the width of a
/// chip (`lib/flow.js#flowAimSummary`). Two names at most, and "Tous les
/// styles" rather than the word "genres" when nothing is chosen.
pub fn aim_summary(styles: &[String], level: f64) -> String {
    let names: Vec<&str> = styles
        .iter()
        .map(String::as_str)
        .filter(|name| !name.is_empty())
        .take(STYLES_MAX)
        .collect();
    let label = match names.len() {
        0 => "Tous les styles".to_owned(),
        1 | 2 => names.join(", "),
        count => format!("{}, {} +{}", names[0], names[1], count - 2),
    };
    format!("{label} · {}", diversity_step(level).0)
}

/// The name of an energy notch, from the value the session holds
/// (`lib/flow.js#energyName`).
pub fn energy_name(value: Option<f64>) -> &'static str {
    let Some(value) = value.filter(|value| value.is_finite()) else {
        return ENERGY_STEPS[0].0;
    };
    let mut best = 1;
    for (index, (_, notch, _)) in ENERGY_STEPS.iter().enumerate().skip(1) {
        let left = notch.unwrap_or(0.5);
        let right = ENERGY_STEPS[best].1.unwrap_or(0.5);
        if (left - value).abs() < (right - value).abs() {
            best = index;
        }
    }
    ENERGY_STEPS[best].0
}

/// The device's copy of the level and of the Flow's aim (`lib/flow.js`'s
/// `localStorage` keys, in `nz-cache`'s `kv`).
pub const LEVEL_KEY: &str = "discover.level";
pub const ENERGY_KEY: &str = "flow.energy";
pub const STYLES_KEY: &str = "flow.styles";
pub const SESSION_KEY: &str = "flow.session";
/// `lib/discovery.js#DEFAULT_LEVEL`.
pub const DEFAULT_LEVEL: f64 = 0.35;

// -- the session ------------------------------------------------------------

/// One title the Flow heard, with the seconds actually played.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Heard {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub s: u32,
}

/// One Flow session: what was heard, what the person pushed away or asked more
/// of, and the aim it was started with (`lib/flow.js#flowSession`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Session {
    #[serde(default)]
    pub sid: String,
    #[serde(default)]
    pub energy: Option<f64>,
    #[serde(default)]
    pub styles: Vec<String>,
    #[serde(default)]
    pub heard: Vec<Heard>,
    #[serde(default)]
    pub more: Vec<String>,
    #[serde(default)]
    pub less: Vec<String>,
}

/// What a radio is anchored on (`Player.svelte#radioSeed`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seed {
    /// One title: the radio's own seed, kept across refills so it never wanders
    /// off a neighbour of a neighbour.
    Track(String),
    Artist(String),
    /// A genre station refills from its STYLE, never from the title playing.
    Genre(String),
}

impl Seed {
    fn key(&self) -> String {
        match self {
            Seed::Track(id) => format!("track:{id}"),
            Seed::Artist(id) => format!("artist:{id}"),
            Seed::Genre(style) => format!("genre:{style}"),
        }
    }
}

/// One request the service asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Job {
    /// A Flow's first titles: the first one plays, the rest is queued.
    FlowStart,
    /// More of a Flow already playing.
    FlowMore,
    /// The Flow was retuned (a notch or a feedback press): what follows the
    /// playing row is replaced, not appended.
    FlowRetune,
    /// More of a radio already playing.
    RadioMore,
    /// The genres a Flow can aim at, for the picker.
    Styles,
}

/// What the network thread answers.
enum Reply {
    Tracks {
        job: Job,
        key: String,
        answer: Result<Vec<Track>, ApiError>,
    },
    Styles(Result<Vec<FlowStyle>, ApiError>),
}

/// The whole service, a GPUI global: the session, the radio's seed, the one
/// attempt in flight and the picker's own little state.
#[derive(Default)]
struct Service {
    session: Option<Session>,
    /// The radio playing now, and the seed its refills use.
    seed: Option<(String, Seed)>,
    /// `recent` / `skipped` for the radio body, per context (`sessionFor`).
    recent: Vec<String>,
    skipped: Vec<String>,
    /// A request is in flight: one at a time, whatever the beat says.
    busy: bool,
    /// `<source>|<queue length>` already asked and answered with nothing new.
    exhausted: Option<String>,
    last_attempt: Option<Instant>,
    /// Attempts made for the current row, and when the next one may run.
    attempts: u8,
    retry_at: Option<Instant>,
    /// The playing row and the position it had at the last beat, so a change is
    /// what closes its listen.
    last: Option<(String, f64)>,
    /// The picker: fetched once, open or not.
    styles_open: bool,
    styles: Option<Vec<FlowStyle>>,
    styles_failed: Option<ApiError>,
    tx: Option<Sender<Reply>>,
    installed: bool,
    beating: bool,
}

struct Owner(RefCell<Service>);

impl gpui_kit::Global for Owner {}

fn with<R>(cx: &mut App, f: impl FnOnce(&mut Service) -> R) -> R {
    if !cx.has_global::<Owner>() {
        cx.set_global(Owner(RefCell::new(Service::default())));
    }
    cx.update_global::<Owner, _>(|owner, _| f(&mut owner.0.borrow_mut()))
}

fn read<R>(cx: &App, f: impl FnOnce(&Service) -> R) -> R {
    match cx.try_global::<Owner>() {
        Some(owner) => f(&owner.0.borrow()),
        None => f(&Service::default()),
    }
}

/// Is a Flow session the one this queue is playing?
pub fn flow_live(state: &AppState, cx: &App) -> bool {
    let Some(context) = state.player.context() else {
        return false;
    };
    if context.kind != "flow" {
        return false;
    }
    let session = read(cx, |service| service.session.clone());
    match session {
        Some(session) => context.id.as_deref() == Some(session.sid.as_str()),
        None => false,
    }
}

/// The Flow's own state, for the panel and for a test.
pub fn session(cx: &App) -> Option<Session> {
    read(cx, |service| service.session.clone())
}

/// Whether the queue playing right now is a radio (`radio`, `artist-radio`,
/// `genre-radio`), and which.
pub fn radio(state: &AppState) -> Option<String> {
    let context = state.player.context()?;
    is_radio(&context.kind).then(|| context.kind.clone())
}

fn is_radio(kind: &str) -> bool {
    matches!(kind, "radio" | "artist-radio" | "genre-radio")
}

// -- the beat ---------------------------------------------------------------

/// Install the service and its beat, once per process. Called by the screens
/// that show the Flow; a no-op afterwards.
pub fn ensure(state: &Entity<AppState>, cx: &mut App) {
    with(cx, |_| {});
    if read(cx, |service| service.installed) {
        return;
    }
    let (tx, rx) = events::channel::<Reply>();
    with(cx, |service| {
        service.installed = true;
        service.tx = Some(tx);
    });
    // The answers are applied on a task owned by the app's own entity: the
    // service must outlive every screen, or a Flow would stop the moment the
    // person navigates away.
    state.update(cx, move |_, cx: &mut Context<AppState>| {
        cx.spawn(async move |state: WeakEntity<AppState>, cx| {
            while let Some(reply) = rx.next().await {
                if state
                    .update(cx, |state, cx| apply(reply, state, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    });
    start_beat(state, cx);
}

/// The beat: one cheap read a second while a station plays. It never repaints
/// on its own and holds nothing but the player's own state.
fn start_beat(state: &Entity<AppState>, cx: &mut App) {
    if read(cx, |service| service.beating) {
        return;
    }
    with(cx, |service| service.beating = true);
    state.update(cx, |_, cx: &mut Context<AppState>| {
        cx.spawn(async move |state: WeakEntity<AppState>, cx| {
            loop {
                let period = match state.update(cx, |state, _| beat_period(state)) {
                    Ok(period) => period,
                    Err(_) => break,
                };
                cx.background_executor().timer(period).await;
                let alive = state.update(cx, pump).is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();
    });
}

/// The period the next read may wait: what a test asserts and what keeps an
/// idle app asleep.
pub fn beat_period(state: &AppState) -> Duration {
    let station = state
        .player
        .context()
        .is_some_and(|context| context.kind == "flow" || is_radio(&context.kind));
    if station && state.player.playing() {
        Duration::from_millis(BEAT_MS)
    } else {
        Duration::from_millis(IDLE_BEAT_MS)
    }
}

// -- starting and steering --------------------------------------------------

/// Start a Flow: one request, its first title plays, the rest is queued.
///
/// `energy` is the notch the panel shows; the level and the genres are the
/// device's copies (`widgets/discovery_slider.rs` writes the level, and the
/// server holds the same value so the phone and the desktop agree).
pub fn start(state: &Entity<AppState>, energy: Option<f64>, cx: &mut App) {
    let Some(api) = state.read(cx).api.clone() else {
        return;
    };
    let runtime = state.read(cx).runtime.clone();
    let Some(tx) = read(cx, |service| service.tx.clone()) else {
        return;
    };
    let level = level(state.read(cx), cx);
    let styles = styles_of_device(state.read(cx), cx);
    let session = Session {
        sid: new_sid(),
        energy,
        styles: styles.clone(),
        heard: Vec::new(),
        more: Vec::new(),
        less: Vec::new(),
    };
    let body = flow_body(&session, level, &styles, FLOW_PICK, &[]);
    let cache = state.read(cx).cache.clone();
    with(cx, |service| {
        service.session = Some(session.clone());
        service.seed = None;
        service.busy = true;
        service.last_attempt = Some(Instant::now());
        service.attempts = 0;
        service.retry_at = None;
        service.exhausted = None;
        service.last = None;
        service.recent.clear();
        service.skipped.clear();
    });
    persist_session_of(cache.as_ref(), &session);
    let key = format!("flow-start:{}", session.sid);
    runtime.spawn(async move {
        let answer = api.flow_pick(&body).await.map(|list| list.tracks);
        tx.send(Reply::Tracks {
            job: Job::FlowStart,
            key,
            answer,
        });
    });
}

/// "Plus / moins comme ça" (`lib/flow.js#moreLikeThis`/`#lessLikeThis`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feedback {
    More,
    Less,
}

/// Press "plus" or "moins": the session remembers the title, and what follows
/// the playing row is chosen again — the playing one is never cut.
pub fn feedback(state: &Entity<AppState>, which: Feedback, cx: &mut App) {
    let Some(reference) = state
        .read(cx)
        .player
        .current()
        .and_then(|entry| entry.reference().map(str::to_owned))
    else {
        return;
    };
    let cache = state.read(cx).cache.clone();
    let changed = with(cx, |service| {
        let Some(session) = service.session.as_mut() else {
            return false;
        };
        match which {
            Feedback::More => session.more.push(reference.clone()),
            Feedback::Less => session.less.push(reference.clone()),
        }
        session.more.truncate(PRESSES_MAX);
        session.less.truncate(PRESSES_MAX);
        let session = session.clone();
        persist_session_of(cache.as_ref(), &session);
        true
    });
    if !changed {
        return;
    }
    retune(state, cx);
    let message = match which {
        Feedback::More => "Plus de titres comme celui-ci",
        Feedback::Less => "Moins de titres comme celui-ci",
    };
    state.update(cx, |state, cx| {
        state.toasts.push(ToastKind::Info, message);
        cx.notify();
    });
}

/// Choose again what follows the playing row — a notch moved, a feedback press,
/// the breadth changed in Réglages (`lib/flow.js#feedbackChanged`).
pub fn retune(state: &Entity<AppState>, cx: &mut App) {
    if !flow_live(state.read(cx), cx) {
        return;
    }
    with(cx, |service| {
        service.exhausted = None;
        service.attempts = 0;
        service.retry_at = None;
    });
    state.update(cx, |state, cx| ask(state, cx, Job::FlowRetune));
}

/// Stop the Flow: the session is forgotten, and the queue keeps its rows.
pub fn stop(cx: &mut App) {
    with(cx, |service| {
        service.session = None;
        service.seed = None;
        service.exhausted = None;
        service.attempts = 0;
        service.retry_at = None;
        service.last = None;
    });
}

/// "Ne plus proposer cet artiste" (`POST /api/discover/mute`): the artist of the
/// playing title is never proposed again, and the Flow's next title is chosen
/// without them.
pub fn mute_current_artist(state: &Entity<AppState>, cx: &mut App) {
    let Some(api) = state.read(cx).api.clone() else {
        return;
    };
    let runtime = state.read(cx).runtime.clone();
    let target = state.read(cx).player.current().map(|entry| {
        let provider = if entry.is_local() { "local" } else { "deezer" };
        let id = entry
            .artist
            .as_ref()
            .and_then(|artist| artist.deezer_id.clone())
            .unwrap_or_default();
        (provider.to_owned(), id, entry.artist_name().to_owned())
    });
    let Some((provider, id, name)) = target.filter(|(_, id, _)| !id.is_empty()) else {
        state.update(cx, |state, cx| {
            state
                .toasts
                .push(ToastKind::Info, "Cet artiste n'a pas de fiche à écarter.");
            cx.notify();
        });
        return;
    };
    runtime.spawn(async move {
        if let Err(error) = api.mute_artist(&provider, &id).await {
            // The artist was the person's gesture: a refusal is worth a line,
            // not a toast that would fight the confirmation one.
            tracing::debug!(%error, "l'artiste n'a pas pu être écarté");
        }
    });
    state.update(cx, |state, cx| {
        state
            .toasts
            .push(ToastKind::Info, format!("{name} ne sera plus proposé."));
        cx.notify();
    });
    retune(state, cx);
}

// -- the pump ---------------------------------------------------------------

/// One beat: close the listen of a row that just left, then top the queue up
/// when fewer than [`UPCOMING_MIN`] titles follow the playing one.
fn pump(state: &mut AppState, cx: &mut Context<AppState>) {
    let context = state.player.context().cloned();
    let station = context
        .as_ref()
        .is_some_and(|context| context.kind == "flow" || is_radio(&context.kind));
    if !station {
        // Nothing to keep alive: forget the radio's seed, so the next station is
        // asked with its own.
        with(cx, |service| {
            service.seed = None;
            service.last = None;
            service.busy = false;
        });
        return;
    }
    let Some(context) = context else {
        return;
    };
    let flow = context.kind == "flow";
    // A Flow queue whose id nobody holds (restored from an older build) is not
    // ours to choose for: adopting it would overwrite the person's session.
    if flow && !flow_live(state, cx) {
        return;
    }
    track_change(state, cx);

    let index = state.player.index();
    if index < 0 || !state.player.autoplay() {
        return;
    }
    let len = state.player.len();
    let upcoming = (len as i64 - index - 1).max(0) as usize;
    if !flow {
        remember_seed(state, cx, &context);
    }
    if upcoming >= UPCOMING_MIN {
        return;
    }

    let (busy, exhausted, retry_at, last_attempt) = read(cx, |service| {
        (
            service.busy,
            service.exhausted.clone(),
            service.retry_at,
            service.last_attempt,
        )
    });
    if busy {
        return;
    }
    if let Some(at) = retry_at
        && Instant::now() < at
    {
        return;
    }
    let key = format!("{}|{len}", context.kind);
    if exhausted.as_deref() == Some(key.as_str()) {
        return;
    }
    if let Some(last) = last_attempt
        && last.elapsed().as_millis() < MIN_EXTEND_MS
    {
        return;
    }
    ask(state, cx, if flow { Job::FlowMore } else { Job::RadioMore });
}

/// Close the listen of the row that just left and keep the radio's session
/// (`Player.svelte#noteSession`). The position read at the previous beat is what
/// the row was played for.
fn track_change(state: &mut AppState, cx: &mut Context<AppState>) {
    let Some(entry) = state.player.current() else {
        return;
    };
    let key = entry.key().to_owned();
    let reference = entry.reference().map(str::to_owned).unwrap_or_default();
    let position = state.player.current_time();
    let previous = read(cx, |service| service.last.clone());
    let Some((previous_key, played)) = previous else {
        with(cx, |service| service.last = Some((key, position)));
        return;
    };
    if previous_key == key {
        with(cx, |service| service.last = Some((key, position)));
        return;
    }
    let cache = state.cache.clone();
    let skip = played < SKIP_UNDER;
    with(cx, |service| {
        if skip {
            service.skipped.push(reference);
            service.skipped.truncate(30);
        } else {
            service.recent.push(reference);
            service.recent.truncate(20);
        }
        if let Some(session) = service.session.as_mut() {
            session.heard.push(Heard {
                id: previous_key,
                s: played.round().max(0.) as u32,
            });
            session.heard.truncate(HEARD_MAX);
            let session = session.clone();
            persist_session_of(cache.as_ref(), &session);
        }
        service.attempts = 0;
        service.retry_at = None;
        service.exhausted = None;
        service.last = Some((key, position));
    });
}

fn remember_seed(state: &AppState, cx: &mut App, context: &PlayContext) {
    let id = context.id.clone().unwrap_or_default();
    let seed = match context.kind.as_str() {
        "radio" if !id.is_empty() => Some(Seed::Track(id)),
        "artist-radio" if !id.is_empty() => Some(Seed::Artist(id)),
        "genre-radio" if !id.is_empty() => Some(Seed::Genre(id)),
        _ => None,
    };
    let Some(seed) = seed else {
        // A radio without an id: the title playing when the queue first ran dry
        // becomes the seed, and stays (`Player.svelte#radioSeed`).
        if read(cx, |service| service.seed.is_some()) {
            return;
        }
        if let Some(id) = state
            .player
            .current()
            .and_then(|entry| entry.reference().map(str::to_owned))
        {
            with(cx, |service| {
                service.seed = Some((String::new(), Seed::Track(id)))
            });
        }
        return;
    };
    let key = format!("{}:{}", context.kind, seed.key());
    let known = read(cx, |service| {
        service
            .seed
            .as_ref()
            .is_some_and(|(known, _)| known == &key)
    });
    if !known {
        with(cx, |service| {
            service.seed = Some((key, seed));
            service.recent.clear();
            service.skipped.clear();
            service.exhausted = None;
            service.attempts = 0;
            service.retry_at = None;
        });
    }
}

/// Ask for what the station lacks, on the network thread.
fn ask(state: &mut AppState, cx: &mut Context<AppState>, job: Job) {
    if read(cx, |service| service.busy) {
        return;
    }
    let api = state.api.clone();
    let online = state.online;
    let runtime = state.runtime.clone();
    let Some(api) = api else {
        return;
    };
    if !online {
        return;
    }
    let Some(tx) = read(cx, |service| service.tx.clone()) else {
        return;
    };
    if job == Job::Styles {
        with(cx, |service| {
            service.busy = true;
            service.last_attempt = Some(Instant::now());
        });
        runtime.spawn(async move {
            let answer = api.flow_styles().await.map(|styles| styles.styles);
            tx.send(Reply::Styles(answer));
        });
        return;
    }
    let exclude = exclude_refs(state);
    if job == Job::RadioMore {
        let Some((source, seed)) = read(cx, |service| service.seed.clone()) else {
            return;
        };
        let recent = read(cx, |service| service.recent.clone());
        let skipped = read(cx, |service| service.skipped.clone());
        with(cx, |service| {
            service.busy = true;
            service.last_attempt = Some(Instant::now());
        });
        runtime.spawn(async move {
            let answer = match &seed {
                Seed::Genre(style) => api
                    .genre_radio_next(style, &recent, &skipped, &exclude)
                    .await
                    .map(|list| list.tracks),
                // The seed is an OBJECT (`{"kind", "id"}`), which is what the
                // route reads and what `Api::radio_next` now takes: this used
                // to hand-build the body through `Api::send`, because the typed
                // function sent a bare string and the server answered
                // `400 invalid seed`.
                Seed::Artist(id) => api
                    .radio_next(
                        &RadioSeed::artist(id.as_str()),
                        Some(RADIO_BATCH),
                        &recent,
                        &skipped,
                        &exclude,
                    )
                    .await
                    .map(|list| list.tracks),
                Seed::Track(id) => api
                    .radio_next(
                        &RadioSeed::track(id.as_str()),
                        Some(RADIO_BATCH),
                        &recent,
                        &skipped,
                        &exclude,
                    )
                    .await
                    .map(|list| list.tracks),
            };
            tx.send(Reply::Tracks {
                job: Job::RadioMore,
                key: source,
                answer,
            });
        });
        return;
    }
    let level = level(state, cx);
    let playing = state
        .player
        .current()
        .and_then(|entry| entry.reference().map(str::to_owned))
        .unwrap_or_default();
    let Some((mut session, adopt)) = session_of(state, cx) else {
        return;
    };
    if adopt {
        session.sid = new_sid();
        let adopted = session.clone();
        with(cx, |service| service.session = Some(adopted));
    }
    let styles = session.styles.clone();
    let body = flow_body(&session, level, &styles, FLOW_PICK, &exclude);
    let key = format!("flow:{}:{playing}", session.sid);
    with(cx, |service| {
        service.busy = true;
        service.last_attempt = Some(Instant::now());
    });
    runtime.spawn(async move {
        let answer = api.flow_pick(&body).await.map(|list| list.tracks);
        tx.send(Reply::Tracks { job, key, answer });
    });
}

/// The session this queue belongs to, and whether it needs adopting: a Flow
/// queue saved without an id becomes a session of its own, as the SPA does
/// (`flow.js#session`), and a queue whose id nobody holds is not ours to choose
/// for.
fn session_of(state: &AppState, cx: &App) -> Option<(Session, bool)> {
    let context = state.player.context()?;
    let session = read(cx, |service| service.session.clone())?;
    if context.id.as_deref() == Some(session.sid.as_str()) {
        return Some((session, false));
    }
    context.id.is_none().then_some((session, true))
}

/// `lib/flow.js#body`: the session's vocabulary, sent as built. The server
/// validates it (`neezer/webui/discover.py#flow_next`).
fn flow_body(
    session: &Session,
    level: f64,
    styles: &[String],
    n: u8,
    exclude: &[String],
) -> serde_json::Value {
    serde_json::json!({
        "sid": session.sid,
        "d": (level.clamp(0., 1.) * 100.).round() / 100.,
        "energy": session.energy,
        "styles": styles,
        "heard": session.heard,
        "more": session.more,
        "less": session.less,
        "exclude": exclude,
        "n": n,
    })
}

/// The last [`EXCLUDE_MAX`] refs of the queue, oldest first
/// (`s.queue.slice(-400).map(trackRef)`).
fn exclude_refs(state: &AppState) -> Vec<String> {
    let queue = state.player.queue();
    let start = queue.len().saturating_sub(EXCLUDE_MAX);
    queue[start..]
        .iter()
        .filter_map(|entry| entry.reference().map(str::to_owned))
        .collect()
}

fn persist_session_of(cache: Option<&Cache>, session: &Session) {
    let Some(cache) = cache else {
        return;
    };
    if let Err(error) = cache.kv().set(SESSION_KEY, session) {
        tracing::debug!(%error, "la session du Flow n'a pas pu être enregistrée");
    }
}

// -- the answers ------------------------------------------------------------

fn apply(reply: Reply, state: &mut AppState, cx: &mut Context<AppState>) {
    match reply {
        Reply::Styles(answer) => {
            with(cx, |service| {
                service.busy = false;
                match answer {
                    Ok(styles) => {
                        service.styles = Some(styles);
                        service.styles_failed = None;
                    }
                    Err(error) => service.styles_failed = Some(error),
                }
            });
            cx.notify();
        }
        Reply::Tracks { job, key, answer } => {
            with(cx, |service| service.busy = false);
            match answer {
                Ok(rows) if !rows.is_empty() => {
                    with(cx, |service| {
                        service.attempts = 0;
                        service.retry_at = None;
                        service.exhausted = None;
                    });
                    match job {
                        Job::FlowStart => start_rows(state, rows, cx),
                        Job::FlowRetune => replace_rows(state, rows),
                        Job::FlowMore | Job::RadioMore => {
                            let added = extend_rows(state, rows);
                            if added == 0 {
                                with(cx, |service| service.exhausted = Some(key));
                            }
                        }
                        Job::Styles => {}
                    }
                }
                Ok(_) => {
                    // Nothing new at this queue length: the source will answer
                    // the same again, so it is not asked twice (the SPA's
                    // `exhausted`, which is what stopped a request storm there).
                    with(cx, |service| service.exhausted = Some(key));
                    if job == Job::FlowStart {
                        state
                            .toasts
                            .push(ToastKind::Info, "Rien à proposer pour l'instant.");
                    }
                }
                Err(error) => failed(job, key, error, state, cx),
            }
            cx.notify();
        }
    }
}

fn start_rows(state: &mut AppState, rows: Vec<Track>, cx: &mut Context<AppState>) {
    let Some(session) = read(cx, |service| service.session.clone()) else {
        return;
    };
    let mut entries: Vec<_> = rows.iter().map(entry_from_track).collect();
    let rest: Vec<_> = if entries.len() > 1 {
        entries.split_off(1)
    } else {
        Vec::new()
    };
    let context = PlayContext {
        kind: "flow".to_owned(),
        id: Some(session.sid),
        title: Some("Flow".to_owned()),
    };
    state.command(Command::play_queue(entries, 0, Some(context)), cx);
    if !rest.is_empty() {
        let now = state.now_ms();
        let (_, effects) = state.player.extend(rest, now);
        state.apply_effects(effects);
    }
    with(cx, |service| {
        service.exhausted = None;
        service.last = None;
    });
}

fn extend_rows(state: &mut AppState, rows: Vec<Track>) -> usize {
    if rows.is_empty() {
        return 0;
    }
    let entries: Vec<_> = rows.iter().map(entry_from_track).collect();
    let now = state.now_ms();
    let (added, effects) = state.player.extend(entries, now);
    state.apply_effects(effects);
    if added > 0 {
        state.sync_mpris();
    }
    added
}

fn replace_rows(state: &mut AppState, rows: Vec<Track>) {
    let entries: Vec<_> = rows.iter().map(entry_from_track).collect();
    let now = state.now_ms();
    let effects = state.player.replace_upcoming(entries, now);
    state.apply_effects(effects);
}

/// A choice that failed: retried a bounded number of times for the same row,
/// then given up on with the cause on screen (rule 3: no storm, no silence).
fn failed(
    job: Job,
    key: String,
    error: ApiError,
    state: &mut AppState,
    cx: &mut Context<AppState>,
) {
    let cause = error.cause_fr();
    let more = with(cx, |service| {
        service.attempts = service.attempts.saturating_add(1);
        let slot = RETRY_DELAYS_MS
            .get(usize::from(service.attempts).saturating_sub(1))
            .copied();
        service.retry_at = slot.map(|delay| Instant::now() + Duration::from_millis(delay));
        slot.is_some()
    });
    if more {
        return;
    }
    with(cx, |service| service.exhausted = Some(key));
    let what = match job {
        Job::FlowStart | Job::FlowMore | Job::FlowRetune => "Le Flow ne répond pas",
        Job::Styles => "Les genres du Flow n'ont pas pu être lus",
        Job::RadioMore => "La radio ne répond pas",
    };
    state.toasts.push_with(
        ToastKind::Error,
        format!("{what} : {cause}"),
        Some(Retry::Probe),
    );
}

// -- the device's copies ----------------------------------------------------

/// The discovery level: the device's copy, so the control paints before any
/// answer (`lib/discovery.js#readLocal`).
pub fn level(state: &AppState, cx: &App) -> f64 {
    read(cx, |_| ());
    state
        .cache
        .as_ref()
        .and_then(|cache| cache.kv().get::<f64>(LEVEL_KEY))
        .map(|level| level.clamp(0., 1.))
        .unwrap_or(DEFAULT_LEVEL)
}

/// The genres the device last chose (`flow.styles`).
pub fn styles_of_device(state: &AppState, cx: &App) -> Vec<String> {
    read(cx, |_| ());
    state
        .cache
        .as_ref()
        .and_then(|cache| cache.kv().get::<Vec<String>>(STYLES_KEY))
        .map(|styles| {
            styles
                .into_iter()
                .filter(|name| !name.trim().is_empty())
                .take(STYLES_MAX)
                .collect()
        })
        .unwrap_or_default()
}

/// The energy notch the device starts from (`flow.energy`).
pub fn energy_of_device(state: &AppState, cx: &App) -> Option<f64> {
    read(cx, |_| ());
    state
        .cache
        .as_ref()
        .and_then(|cache| cache.kv().get::<Option<f64>>(ENERGY_KEY))
        .flatten()
        .filter(|value| value.is_finite())
}

/// Remember the Flow's starting notch and tell the server (one PUT: the server
/// holds the value, so the phone and the desktop agree).
pub fn set_energy(state: &Entity<AppState>, energy: Option<f64>, cx: &mut App) {
    let Some(api) = state.read(cx).api.clone() else {
        return;
    };
    let runtime = state.read(cx).runtime.clone();
    let cache = state.read(cx).cache.clone();
    if let Some(cache) = &cache
        && let Err(error) = cache.kv().set(ENERGY_KEY, &energy)
    {
        tracing::debug!(%error, "l'énergie du Flow n'a pas pu être enregistrée");
    }
    let changed = with(cx, |service| {
        let Some(session) = service.session.as_mut() else {
            return false;
        };
        session.energy = energy;
        let session = session.clone();
        persist_session_of(cache.as_ref(), &session);
        true
    });
    runtime.spawn(async move {
        if let Err(error) = api.set_flow_energy(energy).await {
            tracing::debug!(%error, "l'énergie du Flow n'a pas été enregistrée sur le serveur");
        }
    });
    if changed {
        retune(state, cx);
    }
    state.update(cx, |_, cx| cx.notify());
}

/// Choose the genres the Flow aims at (`lib/flow.js#setStartStyles`): kept on
/// the device — a mood of the moment, not a setting of the account.
pub fn set_styles(state: &Entity<AppState>, styles: Vec<String>, cx: &mut App) {
    let styles: Vec<String> = styles.into_iter().take(STYLES_MAX).collect();
    let cache = state.read(cx).cache.clone();
    if let Some(cache) = &cache
        && let Err(error) = cache.kv().set(STYLES_KEY, &styles)
    {
        tracing::debug!(%error, "les genres du Flow n'ont pas pu être enregistrés");
    }
    let changed = with(cx, |service| {
        let Some(session) = service.session.as_mut() else {
            return false;
        };
        session.styles = styles.clone();
        let session = session.clone();
        persist_session_of(cache.as_ref(), &session);
        true
    });
    if changed {
        retune(state, cx);
    }
    state.update(cx, |_, cx| cx.notify());
}

/// Open or close the genre picker; its first open asks `/api/flow/styles` once.
pub fn toggle_styles(state: &Entity<AppState>, cx: &mut App) {
    let (open, known) = with(cx, |service| {
        service.styles_open = !service.styles_open;
        (service.styles_open, service.styles.is_some())
    });
    if open && !known {
        state.update(cx, |state, cx| ask(state, cx, Job::Styles));
    }
    state.update(cx, |_, cx| cx.notify());
}

/// The genres the picker can offer, when they were read.
pub fn styles(cx: &App) -> Option<Vec<FlowStyle>> {
    read(cx, |service| service.styles.clone())
}

/// Why the picker's list could not be read.
pub fn styles_failure(cx: &App) -> Option<ApiError> {
    read(cx, |service| service.styles_failed.clone())
}

/// Whether the genre picker is open.
pub fn styles_open(cx: &App) -> bool {
    read(cx, |service| service.styles_open)
}

/// Ask the picker's list again (its own "Réessayer").
pub fn retry_styles(state: &Entity<AppState>, cx: &mut App) {
    with(cx, |service| service.styles_failed = None);
    state.update(cx, |state, cx| ask(state, cx, Job::Styles));
}

/// A fresh session id: hexadecimal characters, the shape `flow.js#newSid` makes
/// and the only alphabet the server keeps (`"".join(ch for ch in ... if
/// ch.isalnum())`).
fn new_sid() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_nanos() as u64)
        .unwrap_or(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:016x}{count:04x}")
}

// -- the panel --------------------------------------------------------------

/// The Flow's card: the aim, the two notch rows, the genre picker and — while it
/// plays — what follows and the three steering buttons.
pub fn panel<V: Render>(
    state: &Entity<AppState>,
    covers: &Rc<RefCell<Covers>>,
    cx: &mut Context<V>,
) -> AnyElement {
    let (live, playing, upcoming, energy) = {
        let snapshot = state.read(cx);
        let live = flow_live(snapshot, cx);
        let energy = read(cx, |service| {
            service.session.as_ref().and_then(|s| s.energy)
        })
        .or_else(|| energy_of_device(snapshot, cx));
        (
            live,
            snapshot
                .player
                .context()
                .is_some_and(|context| context.kind == "flow"),
            snapshot.player.next_entry().cloned(),
            energy,
        )
    };
    let level = level(state.read(cx), cx);
    let styles = styles_of_device(state.read(cx), cx);
    let aim = aim_summary(&styles, level);
    let (name, _, hint) = diversity_step(level);

    let mut card = div()
        .id("flow-panel")
        .flex()
        .flex_col()
        .gap(rems(0.75))
        .p(rems(1.))
        .border_1()
        .border_color(theme::border(cx))
        .bg(theme::chrome(cx))
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.625))
                .child(
                    icon(IconName::Sparkles)
                        .size(rems(1.125))
                        .text_color(theme::accent(cx)),
                )
                .child(
                    div()
                        .text_size(rems(0.9375))
                        .font_weight(FontWeight::BOLD)
                        .child("Flow"),
                )
                .child(
                    div()
                        .text_size(rems(0.75))
                        .font_family(theme::mono(cx))
                        .text_color(theme::secondary(cx))
                        .child(aim),
                )
                .child(div().flex_1())
                .when(live, |row| {
                    row.child(
                        div()
                            .flex()
                            .items_center()
                            .px(rems(0.5))
                            .py(rems(0.125))
                            .bg(theme::accent(cx))
                            .text_size(rems(0.5625))
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme::shell(cx))
                            .child("EN ÉCOUTE"),
                    )
                })
                .when(!live, |row| {
                    row.child(
                        button("flow-start", "Lancer le Flow", ButtonVariant::Primary, cx)
                            .on_click(cx.listener({
                                let state = state.clone();
                                move |_, _, _, cx| start(&state, energy, cx)
                            })),
                    )
                }),
        )
        .child(
            div()
                .text_size(rems(0.75))
                .text_color(theme::secondary(cx))
                .child(if playing && !live {
                    "Un autre Flow joue : lancer celui-ci remplacera la file."
                } else {
                    hint
                }),
        );

    // The two notch rows: the discovery breadth first (the level is the
    // person's, on the server), then the energy the Flow starts at.
    card = card.child(
        div()
            .flex()
            .flex_col()
            .gap(rems(0.25))
            .child(
                div()
                    .text_size(rems(0.6875))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::secondary(cx))
                    .child(format!("Découverte · {name}")),
            )
            .child(crate::widgets::discovery_slider::notches(state, level, cx)),
    );

    let mut energies = toggle_group("flow-energy", cx);
    for (index, (label, value, _)) in ENERGY_STEPS.iter().enumerate() {
        let selected = energy_name(energy) == *label;
        let value = *value;
        energies = energies.child(
            toggle(("flow-energy-notch", index), *label, selected, cx)
                .debug_selector(move || format!("flow-energy-{index}"))
                .on_click(cx.listener({
                    let state = state.clone();
                    move |_, _, _, cx| set_energy(&state, value, cx)
                })),
        );
    }
    card = card.child(
        div()
            .flex()
            .flex_col()
            .gap(rems(0.25))
            .child(
                div()
                    .text_size(rems(0.6875))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::secondary(cx))
                    .child(format!("Énergie de départ · {}", energy_name(energy))),
            )
            .child(energies),
    );

    // The genre picker: one chip to open it, the server's own tree inside.
    let open = styles_open(cx);
    card = card.child(
        div()
            .flex()
            .items_center()
            .gap(rems(0.5))
            .child(
                div()
                    .id("flow-styles-toggle")
                    .flex()
                    .items_center()
                    .gap(rems(0.25))
                    .h(rems(1.75))
                    .px(rems(0.5))
                    .border_1()
                    .border_color(theme::border(cx))
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .cursor_pointer()
                    .hover(|style| style.text_color(theme::text(cx)))
                    .on_click(cx.listener({
                        let state = state.clone();
                        move |_, _, _, cx| toggle_styles(&state, cx)
                    }))
                    .child(icon(IconName::ChevronDown).size(rems(0.75)))
                    .child(if styles.is_empty() {
                        "Choisir des genres".to_owned()
                    } else {
                        format!("Genres · {}", styles.join(", "))
                    }),
            )
            .when(!styles.is_empty(), |row| {
                row.child(
                    div()
                        .id("flow-styles-clear")
                        .h(rems(1.75))
                        .px(rems(0.5))
                        .flex()
                        .items_center()
                        .text_size(rems(0.6875))
                        .text_color(theme::secondary(cx))
                        .cursor_pointer()
                        .hover(|style| style.text_color(theme::text(cx)))
                        .on_click(cx.listener({
                            let state = state.clone();
                            move |_, _, _, cx| set_styles(&state, Vec::new(), cx)
                        }))
                        .child("Tous les styles"),
                )
            }),
    );
    if open {
        card = card.child(genre_picker(state, &styles, cx));
    }

    if live {
        let next_label = upcoming
            .as_ref()
            .map(|entry| format!("{} — {}", entry.title, entry.artist_name()))
            .unwrap_or_else(|| "Choix du titre suivant…".to_owned());
        let cover = upcoming
            .as_ref()
            .and_then(|entry| entry.cover.as_deref())
            .and_then(|url| cover::file_in(covers, url));
        card = card.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .pt(rems(0.25))
                .border_t_1()
                .border_color(theme::border(cx))
                .child(cover::view(
                    cover.as_ref(),
                    upcoming.as_ref().and_then(|entry| entry.cover.as_deref()),
                    upcoming
                        .as_ref()
                        .map(|entry| entry.title.as_str())
                        .unwrap_or(""),
                    Kind::Album,
                    px(32.),
                    cx,
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.))
                        .child(
                            div()
                                .text_size(rems(0.5625))
                                .font_family(theme::mono(cx))
                                .text_color(theme::secondary(cx))
                                .child("À SUIVRE"),
                        )
                        .child(div().truncate().text_size(rems(0.75)).child(next_label)),
                ),
        );
        card = card.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.375))
                .child(
                    button("flow-more", "Plus comme ça", ButtonVariant::Outline, cx).on_click(
                        cx.listener({
                            let state = state.clone();
                            move |_, _, _, cx| feedback(&state, Feedback::More, cx)
                        }),
                    ),
                )
                .child(
                    button("flow-less", "Moins comme ça", ButtonVariant::Outline, cx).on_click(
                        cx.listener({
                            let state = state.clone();
                            move |_, _, _, cx| feedback(&state, Feedback::Less, cx)
                        }),
                    ),
                )
                .child(
                    button(
                        "flow-mute",
                        "Écarter cet artiste",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .on_click(cx.listener({
                        let state = state.clone();
                        move |_, _, _, cx| mute_current_artist(&state, cx)
                    })),
                ),
        );
    }

    card.into_any_element()
}

/// The genres `/api/flow/styles` lists, as chips (at most [`STYLES_MAX`] chosen
/// at a time); a click adds or removes one.
fn genre_picker<V: Render>(
    state: &Entity<AppState>,
    chosen: &[String],
    cx: &mut Context<V>,
) -> AnyElement {
    if let Some(error) = styles_failure(cx) {
        return crate::widgets::card::inline_error(
            format!("Les genres n'ont pas pu être lus : {}", error.cause_fr()),
            cx,
            {
                let state = state.clone();
                move |_, _, cx| retry_styles(&state, cx)
            },
        )
        .into_any_element();
    }
    let Some(rows) = styles(cx) else {
        return div()
            .text_size(rems(0.6875))
            .text_color(theme::secondary(cx))
            .child("Lecture des genres…")
            .into_any_element();
    };
    if rows.is_empty() {
        return div()
            .text_size(rems(0.6875))
            .text_color(theme::secondary(cx))
            .child("Aucun genre mesuré pour l'instant : le Flow suit toute votre écoute.")
            .into_any_element();
    }
    let mut chips = div().flex().flex_wrap().gap(rems(0.25));
    for row in rows.iter().take(48) {
        let active = chosen.iter().any(|name| name == &row.name);
        let name = row.name.clone();
        let count = row.branch.max(row.tracks);
        chips = chips.child(
            div()
                .id(SharedString::from(format!("flow-style-{}", row.name)))
                .flex()
                .items_center()
                .gap(rems(0.25))
                .h(rems(1.75))
                .px(rems(0.5))
                .border_1()
                .border_color(if active {
                    theme::accent(cx)
                } else {
                    theme::border(cx)
                })
                .bg(if active {
                    theme::selected(cx)
                } else {
                    theme::chrome(cx)
                })
                .text_size(rems(0.6875))
                .text_color(if active {
                    theme::text(cx)
                } else {
                    theme::secondary(cx)
                })
                .cursor_pointer()
                .hover(|style| style.text_color(theme::text(cx)))
                .on_click(cx.listener({
                    let state = state.clone();
                    let chosen = chosen.to_vec();
                    move |_, _, _, cx| {
                        let mut next = chosen.clone();
                        if let Some(at) = next.iter().position(|known| known == &name) {
                            next.remove(at);
                        } else if next.len() < STYLES_MAX {
                            next.push(name.clone());
                        }
                        set_styles(&state, next, cx);
                    }
                }))
                .child(row.name.clone())
                .child(
                    div()
                        .font_family(theme::mono(cx))
                        .text_size(rems(0.5625))
                        .text_color(theme::secondary(cx))
                        .child(count.to_string()),
                ),
        );
    }
    chips.into_any_element()
}

#[cfg(test)]
mod playing {
    use super::*;
    use crate::widgets::card::stub;
    use gpui_kit::Window;

    const ME: &str =
        r#"{"user":{"id":"u1","name":"bench","admin":false,"access":{},"sources":[]}}"#;
    const PREFS: &str = r#"{"level":0.4,"step":"Voisin","steps":[],"energy":0.3,
        "energy_step":"Calme","energy_steps":[]}"#;

    /// Two titles: the server clamps `/flow/next` to `n` ≤ 2.
    fn flow_json() -> String {
        let mut tracks = Vec::new();
        for id in ["3001", "3002"] {
            tracks.push(format!(
                r#"{{"deezer_id":"{id}","title":"Titre {id}","duration":200,
                    "artist":{{"deezer_id":"42","name":"Kaya Project"}},
                    "album":{{"deezer_id":"9","title":"Album","cover":null}}}}"#
            ));
        }
        format!(r#"{{"tracks":[{}],"energy":0.3}}"#, tracks.join(","))
    }

    fn track(id: &str) -> Track {
        serde_json::from_str(&format!(
            r#"{{"deezer_id":"{id}","title":"Titre {id}","duration":200,
                "artist":{{"deezer_id":"42","name":"Kaya Project"}},
                "album":{{"deezer_id":"9","title":"Album","cover":null}}}}"#
        ))
        .expect("a track")
    }

    struct Blank;

    impl Render for Blank {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    /// What one beat does, without the 5 s floor (the floor has its own
    /// assertion below): the pump, on the app's own state.
    fn beat(cx: &mut gpui::VisualTestContext, state: &Entity<AppState>) {
        cx.update(|_, cx| {
            // The 5 s floor is asserted on its own below: here the clock is the
            // test's, so a beat is allowed to be the next one.
            with(cx, |service| {
                service.last_attempt = None;
                service.retry_at = None;
            });
            state.update(cx, pump);
        });
    }

    /// Pump the tokio runtime the requests run on: the count of arrivals on the
    /// stub is what the numbers below are made of.
    fn pump_network(runtime: &tokio::runtime::Runtime) {
        runtime.block_on(async {
            tokio::time::sleep(Duration::from_millis(2)).await;
        });
    }

    /// What the network answer does when it lands.
    fn answer(cx: &mut gpui::VisualTestContext, state: &Entity<AppState>, rows: &[&str], job: Job) {
        let tracks: Vec<Track> = rows.iter().map(|id| track(id)).collect();
        cx.update(|_, cx| {
            state.update(cx, |state, cx| {
                apply(
                    Reply::Tracks {
                        job,
                        key: "flow:test".into(),
                        answer: Ok(tracks),
                    },
                    state,
                    cx,
                );
            });
        });
    }

    /// An `ApiError` clone for a test that hands the same failure again.
    fn clone_error(error: &ApiError) -> ApiError {
        match error {
            ApiError::Offline => ApiError::Offline,
            ApiError::Http {
                status,
                message,
                code,
            } => ApiError::Http {
                status: *status,
                message: message.clone(),
                code: code.clone(),
            },
            other => other.clone(),
        }
    }

    fn source_of(text: &str) -> Option<usize> {
        text.find("serveur").or_else(|| text.find("répond"))
    }

    fn upcoming(state: &AppState) -> usize {
        (state.player.len() as i64 - state.player.index() - 1).max(0) as usize
    }

    /// The acceptance, in numbers: a Flow keeps TWO titles after the playing one,
    /// and every top-up is asked while they are still queued — the queue is
    /// never one title from its end.
    #[gpui::test]
    async fn a_flow_keeps_two_titles_ahead_and_asks_before_it_runs_dry(
        cx: &mut gpui::TestAppContext,
    ) {
        let body = flow_json();
        let stub = stub::Stub::start(&[
            ("/api/login", ME),
            ("/api/me", ME),
            ("/api/discover/prefs", PREFS),
            ("/api/flow/next", body.as_str()),
        ]);
        let (state, runtime) = stub::logged_in(cx, &stub);
        let (_view, cx) = cx.add_window_view(|_, _| Blank);

        cx.update(|_, cx| {
            ensure(&state, cx);
            start(&state, Some(0.3), cx);
        });
        stub::until(&runtime, cx, "the Flow's first title", |cx| {
            state.read(cx).player.len() >= 2
        });
        cx.update(|_, cx| {
            let player = &state.read(cx).player;
            assert_eq!(player.len(), 2, "the start queues the title and its next");
            assert_eq!(player.index(), 0);
            assert_eq!(
                player.context().map(|context| context.kind.as_str()),
                Some("flow")
            );
            assert!(session(cx).is_some(), "a session was opened");
            assert_eq!(beat_period(state.read(cx)), Duration::from_millis(BEAT_MS));
            assert_eq!(upcoming(state.read(cx)), 1, "one title still queued");
        });
        assert_eq!(stub.count("/api/flow/next"), 1, "one request to start");

        // One upcoming is not enough: the next beat asks once, and the answer
        // brings the queue back to two ahead.
        beat(cx, &state);
        pump_network(&runtime);
        stub::until(&runtime, cx, "the first top-up", |cx| {
            let _ = cx;
            stub.count("/api/flow/next") == 2
        });
        answer(cx, &state, &["3003"], Job::FlowMore);
        cx.update(|_, cx| {
            assert_eq!(state.read(cx).player.len(), 3);
            assert_eq!(upcoming(state.read(cx)), UPCOMING_MIN);
        });

        // Four tracks played: one request each, always with two queued.
        for step in 1..=4 {
            cx.update(|_, cx| {
                state.update(cx, |state, _| {
                    let effects = state.player.track_ended(state.now_ms());
                    state.apply_effects(effects);
                });
            });
            let before = stub.count("/api/flow/next");
            beat(cx, &state);
            pump_network(&runtime);
            stub::until(&runtime, cx, "the top-up", |cx| {
                let _ = cx;
                stub.count("/api/flow/next") == before + 1
            });
            assert!(
                cx.update(|_, cx| upcoming(state.read(cx))) >= 1,
                "step {step}: the top-up fired while a title was still queued"
            );
            answer(cx, &state, &[&format!("40{:02}", step)], Job::FlowMore);
            cx.update(|_, cx| {
                assert_eq!(upcoming(state.read(cx)), UPCOMING_MIN, "step {step}");
            });
        }
        // Five titles consumed, five picks asked: the start is the first.
        assert_eq!(stub.count("/api/flow/next"), 6);
        assert_eq!(
            stub.count("/api/discover/prefs"),
            0,
            "the level is the device's copy; only the Releases screen asks the server"
        );

        runtime.shutdown_background();
        stub.stop();
    }

    /// The 5 s floor is what makes a top-up a decision instead of a loop: a
    /// second beat right after an ask does nothing.
    #[gpui::test]
    async fn two_beats_never_ask_twice_in_a_row(cx: &mut gpui::TestAppContext) {
        let body = flow_json();
        let stub = stub::Stub::start(&[
            ("/api/login", ME),
            ("/api/me", ME),
            ("/api/discover/prefs", PREFS),
            ("/api/flow/next", body.as_str()),
        ]);
        let (state, runtime) = stub::logged_in(cx, &stub);
        let (_view, cx) = cx.add_window_view(|_, _| Blank);
        cx.update(|_, cx| {
            ensure(&state, cx);
            start(&state, None, cx);
        });
        stub::until(&runtime, cx, "the start", |cx| {
            state.read(cx).player.len() == 2
        });
        let before = stub.count("/api/flow/next");
        // The 5 s floor: a beat right after the start asks nothing.
        cx.update(|_, cx| state.update(cx, pump));
        cx.update(|_, cx| state.update(cx, pump));
        pump_network(&runtime);
        assert_eq!(
            stub.count("/api/flow/next"),
            before,
            "the floor between two top-ups holds"
        );
        // The floor passed (the test moves the clock instead of sleeping): one
        // beat asks once, and the next one, with an attempt in flight, is
        // silent.
        beat(cx, &state);
        pump_network(&runtime);
        stub::until(&runtime, cx, "the attempt", |cx| {
            let _ = cx;
            stub.count("/api/flow/next") == before + 1
        });
        cx.update(|_, cx| state.update(cx, pump));
        pump_network(&runtime);
        assert_eq!(
            stub.count("/api/flow/next"),
            before + 1,
            "one attempt in flight at a time"
        );

        runtime.shutdown_background();
        stub.stop();
    }

    /// A source that answers only queued titles is never asked again at that
    /// length (the SPA's `exhausted`), and a duplicate is not added twice.
    #[gpui::test]
    async fn a_top_up_that_adds_nothing_is_not_asked_again(cx: &mut gpui::TestAppContext) {
        let body = flow_json();
        let stub = stub::Stub::start(&[
            ("/api/login", ME),
            ("/api/me", ME),
            ("/api/discover/prefs", PREFS),
            ("/api/flow/next", body.as_str()),
        ]);
        let (state, runtime) = stub::logged_in(cx, &stub);
        let (_view, cx) = cx.add_window_view(|_, _| Blank);
        cx.update(|_, cx| {
            ensure(&state, cx);
            start(&state, None, cx);
        });
        stub::until(&runtime, cx, "the start", |cx| {
            state.read(cx).player.len() == 2
        });
        let before = stub.count("/api/flow/next");
        beat(cx, &state);
        pump_network(&runtime);
        stub::until(&runtime, cx, "the top-up", |cx| {
            let _ = cx;
            stub.count("/api/flow/next") == before + 1
        });
        // The same two titles: nothing lands, and the source is marked.
        answer(cx, &state, &["3001", "3002"], Job::FlowMore);
        cx.update(|_, cx| {
            assert_eq!(state.read(cx).player.len(), 2, "a duplicate is not queued");
            assert!(
                read(cx, |service| service.exhausted.is_some()),
                "the source is exhausted at this queue length"
            );
        });
        for _ in 0..4 {
            beat(cx, &state);
        }
        assert_eq!(
            stub.count("/api/flow/next"),
            before + 1,
            "an exhausted source is not asked again"
        );

        runtime.shutdown_background();
        stub.stop();
    }

    /// A failed choice is retried for the SAME row a bounded number of times,
    /// then given up on with the cause shown — never a silent stop, never a
    /// storm. The attempts are the ones `ask` makes after `failed` scheduled
    /// them: three waits, then the toast.
    #[gpui::test]
    async fn a_failing_choice_is_retried_a_bounded_number_of_times(cx: &mut gpui::TestAppContext) {
        let stub = stub::Stub::start(&[("/api/login", ME), ("/api/me", ME)]);
        let (state, runtime) = stub::logged_in(cx, &stub);
        let (_view, cx) = cx.add_window_view(|_, _| Blank);
        cx.update(|_, cx| ensure(&state, cx));
        let cause = ApiError::Http {
            status: 503,
            message: "Service Unavailable".into(),
            code: None,
        };
        for attempt in 1..=RETRY_DELAYS_MS.len() + 1 {
            cx.update(|_, cx| {
                state.update(cx, |state, cx| {
                    failed(
                        Job::FlowMore,
                        "flow:row".into(),
                        clone_error(&cause),
                        state,
                        cx,
                    );
                });
            });
            cx.update(|_, cx| {
                let (attempts, armed) =
                    read(cx, |service| (service.attempts, service.retry_at.is_some()));
                assert_eq!(attempts as usize, attempt);
                assert_eq!(
                    armed,
                    attempt <= RETRY_DELAYS_MS.len(),
                    "attempt {attempt}: a retry is armed for the first three only"
                );
            });
        }
        cx.update(|_, cx| {
            let toast = state
                .read(cx)
                .toasts
                .items()
                .last()
                .map(|toast| toast.message.clone())
                .unwrap_or_default();
            assert!(
                toast.contains("La radio ne répond pas") || toast.contains("Le Flow ne répond"),
                "the failure names itself: {toast:?}"
            );
            assert!(
                source_of(&toast).is_some(),
                "the cause a person can read is on the toast: {toast:?}"
            );
        });
        assert_eq!(
            stub.count("/api/flow/next"),
            0,
            "no request is made here: the retries are the beats' job"
        );
        runtime.shutdown_background();
        stub.stop();
    }

    /// The beat is what keeps an idle app idle: one read a second while a
    /// station plays, one every 20 s otherwise.
    #[gpui::test]
    async fn the_beat_slows_down_when_nothing_plays(cx: &mut gpui::TestAppContext) {
        let stub = stub::Stub::start(&[("/api/login", ME), ("/api/me", ME)]);
        let (state, runtime) = stub::logged_in(cx, &stub);
        let (_view, cx) = cx.add_window_view(|_, _| Blank);
        cx.update(|_, cx| {
            assert_eq!(
                beat_period(state.read(cx)),
                Duration::from_millis(IDLE_BEAT_MS)
            );
            state.update(cx, |state, cx| {
                state.command(
                    Command::play_queue(
                        vec![nz_player::Entry {
                            id: "a".into(),
                            title: "A".into(),
                            duration: 200,
                            ..Default::default()
                        }],
                        0,
                        Some(PlayContext {
                            kind: "flow".into(),
                            id: Some("sid".into()),
                            title: None,
                        }),
                    ),
                    cx,
                );
            });
            assert_eq!(beat_period(state.read(cx)), Duration::from_millis(BEAT_MS));
        });
        runtime.shutdown_background();
        stub.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_level_reads_back_on_the_nearest_notch() {
        // The SPA's rule, ties upward: an older build's 0.35 is "Voisin".
        assert_eq!(diversity_index(0.0), 0);
        assert_eq!(diversity_index(0.35), 2);
        assert_eq!(diversity_index(0.5), 3, "a tie goes up: Exploration");
        assert_eq!(diversity_index(0.49), 2);
        assert_eq!(diversity_index(1.4), 5, "clamped, never a panic");
        assert_eq!(diversity_index(f64::NAN), 0);
        assert_eq!(diversity_step(0.4).0, "Voisin");
    }

    #[test]
    fn the_aim_summary_names_two_styles_at_most() {
        assert_eq!(aim_summary(&[], 0.4), "Tous les styles · Voisin");
        assert_eq!(
            aim_summary(&["Techno".into(), "House".into()], 0.6),
            "Techno, House · Exploration"
        );
        assert_eq!(
            aim_summary(
                &[
                    "Techno".into(),
                    "House".into(),
                    "Disco".into(),
                    "Jazz".into()
                ],
                0.8
            ),
            "Techno, House +2 · Aventure"
        );
        // More than six are never sent: the server refuses the overflow.
        let many: Vec<String> = (0..9).map(|index| format!("S{index}")).collect();
        assert_eq!(aim_summary(&many, 1.0), "S0, S1 +4 · Large");
    }

    #[test]
    fn an_energy_value_reads_back_on_its_notch() {
        assert_eq!(energy_name(None), "Libre");
        assert_eq!(energy_name(Some(0.1)), "Très calme");
        assert_eq!(energy_name(Some(0.28)), "Calme");
        assert_eq!(energy_name(Some(0.9)), "Intense");
    }

    #[test]
    fn a_radio_kind_is_the_three_the_app_starts() {
        assert!(is_radio("radio"));
        assert!(is_radio("artist-radio"));
        assert!(is_radio("genre-radio"));
        assert!(
            !is_radio("flow"),
            "the Flow chooses with its own vocabulary"
        );
        assert!(!is_radio("playlist"));
    }

    /// The body is the SPA's own object: a field renamed here is a Flow that
    /// silently loses its session.
    #[test]
    fn the_flow_body_carries_the_sessions_vocabulary() {
        let session = Session {
            sid: "abc".into(),
            energy: Some(0.5),
            styles: vec!["Techno".into()],
            heard: vec![Heard {
                id: "3000".into(),
                s: 42,
            }],
            more: vec!["1".into()],
            less: vec!["2".into()],
        };
        let body = flow_body(&session, 0.4567, &session.styles, 1, &["9".into()]);
        assert_eq!(body["sid"], "abc");
        assert_eq!(body["d"], 0.46, "two decimals, like `toFixed(2)`");
        assert_eq!(body["energy"], 0.5);
        assert_eq!(body["heard"][0]["s"], 42);
        assert_eq!(body["exclude"][0], "9");
        assert_eq!(body["n"], 1);
    }

    /// The seed the radio refill sends is the OBJECT the server reads
    /// (`neezer/webui#radio_next`: `seed.kind` then `seed.id`), never the bare
    /// string `nz-api` used to send, which the route refused with
    /// `400 invalid seed`. Pinned on the serialised body, since that is what
    /// crosses the wire.
    #[test]
    fn the_radio_seed_is_the_object_the_server_reads() {
        let track = serde_json::to_value(RadioSeed::track("3000")).expect("json");
        assert_eq!(track["kind"], "track");
        assert_eq!(track["id"], "3000");
        assert!(
            track.get("artist").is_none(),
            "an absent artist is not sent: the server reads `seed.artist` only when it has one"
        );
        let artist = serde_json::to_value(RadioSeed::artist("42")).expect("json");
        assert_eq!(artist["kind"], "artist");
        assert_eq!(artist["id"], "42");
    }
}
