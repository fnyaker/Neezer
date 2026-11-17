//! The application state: one `Entity` that owns the session, the player, the
//! queue, the cache handles and every background service's handle.
//!
//! The shape the plan asks for is simple on purpose — screens get `&AppState`,
//! emit a [`nz_player::Command`] (or dispatch an action that becomes one), and
//! the effects the pure state machine returns are carried out here: a `Load`
//! opens the engine, a `ReportListen` becomes a request, a `Persist` writes the
//! queue off the UI thread, a `Changed` redraws. Nothing invents a second
//! transport path, so a click, a key, MPRIS and (later) a remote controller all
//! pass through `PlayerState::apply`.
//!
//! Background subsystems announce themselves through [`AppEvent`]: the audio
//! engine starts on its own thread, MPRIS waits for the bus, a restore is one
//! request on `nz-net`, and the network chip follows `Api::online()`. The shell
//! drains that channel in a foreground task (`crate::app`), so no worker ever
//! touches GPUI state.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::{Context, SharedString};
pub use nz_api::features::Feature;
use nz_api::{Api, ApiError, Lane, SessionJar};
use nz_audio::{Edges, EngineEvent, TrackMeta};
use nz_cache::{Cache, QueueSnapshot};
use nz_model::{ListenContext, ListenPayload, Me, Quality as ModelQuality, Track};
use nz_mpris::{MprisCommand, MprisHandle};
use nz_player::{
    Command, Context as PlayerContext, Effect, Entry, EntryAlbum, EntryArtist, PlayerState, Repeat,
    Snapshot,
};
use tracing::{debug, warn};

use crate::nav::{Nav, Route, SettingsSection};
use crate::screens::settings::keys as settings_keys;
use crate::toast::{Retry, ToastKind, Toasts};
use crate::wiring::engine::{self, Audio, PlaybackQuality};
use crate::wiring::events::{self, Sender};
use crate::wiring::instance::Instance;
use crate::wiring::{mpris as mpris_wiring, net, viz::VizService};

/// How the app is launched. `Default` is the real app; `for_test` points every
/// path at a temp directory and starts no subsystem that needs a bus or a sound
/// card.
#[derive(Clone, Debug)]
pub struct AppConfig {
    /// A server override for a development run; `None` reads the stored one.
    pub server: Option<String>,
    /// Data directory override (tests only; the app uses the XDG one).
    pub data_dir: Option<PathBuf>,
    /// Cache directory override (tests only).
    pub cache_dir: Option<PathBuf>,
    /// Session file override (tests only).
    pub session_file: Option<PathBuf>,
    /// The screen to open once the session is restored (`--route`), instead of
    /// the landing screen. The journey bench opens every screen this way: no
    /// blind click, the same screen on every run (D17).
    pub start_route: Option<Route>,
    pub start_audio: bool,
    pub start_mpris: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: None,
            data_dir: None,
            cache_dir: None,
            session_file: None,
            start_route: None,
            start_audio: true,
            start_mpris: true,
        }
    }
}

impl AppConfig {
    /// A test's app: temp directories, no audio device, no D-Bus.
    pub fn for_test(dir: impl AsRef<Path>) -> Self {
        let dir = dir.as_ref().to_path_buf();
        Self {
            server: None,
            data_dir: Some(dir.join("data")),
            cache_dir: Some(dir.join("cache")),
            session_file: Some(dir.join("session.json")),
            start_route: None,
            start_audio: false,
            start_mpris: false,
        }
    }
}

/// Where the session stands. `Failed` is the shell's `LoadFailed`: a stored
/// session that `/api/me` could not confirm because the server is unreachable.
#[derive(Clone, Debug, PartialEq)]
pub enum Auth {
    /// No stored cookie: the login screen.
    LoggedOut,
    /// A stored cookie is being confirmed (or a login is in flight).
    Restoring,
    LoggedIn {
        me: Box<Me>,
    },
    /// A stored session exists but could not be confirmed. The cause is in
    /// [`AppState::session_cause`].
    Failed,
}

impl Auth {
    pub fn user(&self) -> Option<&Me> {
        match self {
            Auth::LoggedIn { me } => Some(me.as_ref()),
            _ => None,
        }
    }

    pub fn is_logged_in(&self) -> bool {
        matches!(self, Auth::LoggedIn { .. })
    }
}

/// What the login screen shows besides its fields.
#[derive(Clone, Debug, Default)]
pub struct LoginUi {
    pub pending: bool,
    pub error: Option<SharedString>,
}

/// Which side panel is open. The queue is the only one D7 can show; the party
/// and remote sheets add their own variant with the lots that own them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidePanel {
    Queue,
}

/// A `Load` that arrived before the engine existed (startup, restore, a click
/// during the first second). Kept so playback is not silently lost.
#[derive(Clone, Debug)]
pub struct PendingLoad {
    pub entry: Entry,
    pub autoplay: bool,
    pub start_s: f64,
}

/// What a background thread tells the shell.
pub enum AppEvent {
    AudioReady(Result<Box<Audio>, String>),
    MprisReady(MprisHandle),
    Mpris(MprisCommand),
    Engine(EngineEvent),
    /// Reachability changed (`Api::online`).
    Online(bool),
    /// Our own probe answered.
    Probed(bool),
    /// A stored session's `/api/me`.
    Restored(Result<Box<Me>, ApiError>),
    /// A login's answer (login already also fetched `/api/me`).
    LoggedIn(Result<Box<Me>, ApiError>),
    /// A second launch asked for the window.
    Raise,
    /// Which optional features the server does not have (one `OPTIONS` each).
    Features(Vec<Feature>),
    /// MPRIS `Quit`.
    Quit,
    /// The single-instance name was claimed, refused, or the bus was missing.
    Instance(crate::wiring::instance::Claim),
    /// The server's answer for one track's measured bounds.
    ///
    /// `Ok(None)` is the server saying "not archived, or not measured" — a fact
    /// to remember for this session. `Err` is a failure to reach it, which is
    /// never a verdict about the track: nothing is remembered, and the next play
    /// asks once more (rule 3).
    Edges {
        id: String,
        answer: Result<Option<Edges>, ApiError>,
    },
}

/// How many tracks' measured bounds one session keeps. The server measures a
/// file once and caches the answer, so a re-ask is cheap, but a 4 000-track
/// queue must not grow the app's memory for tracks nobody plays: the oldest
/// answers go first, as in `Player.svelte`'s `edges.js`.
const EDGES_MAX: usize = 512;

/// The bounds this session knows, and the threshold they were measured at.
///
/// A threshold is part of the measurement (`neezer/webui/edges.py`), so changing
/// it invalidates every answer: the cache is emptied rather than keyed twice.
#[derive(Clone, Debug, Default)]
struct EdgesCache {
    seen: HashMap<String, Option<Edges>>,
    order: VecDeque<String>,
}

impl EdgesCache {
    fn get(&self, id: &str) -> Option<Option<Edges>> {
        self.seen.get(id).copied()
    }

    fn has(&self, id: &str) -> bool {
        self.seen.contains_key(id)
    }

    fn remember(&mut self, id: &str, edges: Option<Edges>) {
        if !self.seen.contains_key(id) {
            self.order.push_back(id.to_owned());
            while self.order.len() > EDGES_MAX {
                if let Some(oldest) = self.order.pop_front() {
                    self.seen.remove(&oldest);
                }
            }
        }
        self.seen.insert(id.to_owned(), edges);
    }
}

/// How long a persisted queue may sit before another write is allowed: a
/// shuffle of a 4 000-row queue produces a burst of `Persist`s, and one write
/// per burst is the point (D3: 8.8 ms, one statement).
const PERSIST_MIN_INTERVAL: Duration = Duration::from_millis(750);

/// The whole application, as screens see it.
pub struct AppState {
    pub config: AppConfig,
    pub cache: Option<Cache>,
    pub storage_error: Option<SharedString>,
    pub session: Arc<SessionJar>,
    pub api: Option<Arc<Api>>,
    /// The address in use (or to use). Normalised: no trailing slash.
    pub server: String,
    /// The cause shown by the shell's `LoadFailed` when a stored session could
    /// not be confirmed.
    pub session_cause: Option<SharedString>,
    pub auth: Auth,
    pub login: LoginUi,
    pub player: PlayerState,
    pub nav: Nav,
    pub toasts: Toasts,
    pub online: bool,
    pub probing: bool,
    pub audio: Option<Audio>,
    pub audio_error: Option<SharedString>,
    pub mpris: Option<MprisHandle>,
    pub mpris_track: Option<String>,
    /// The 4 Hz position publication, on its own thread (D17): the shell's
    /// tick stops when the window is hidden, and MPRIS froze with it.
    position_clock: Option<mpris_wiring::PositionClock>,
    /// What this server does not have (D17, `nz_api::features`): one `OPTIONS`
    /// per feature at login, and the entries it is missing are not shown.
    pub features: Features,
    pub viz: Option<VizService>,
    pub side_panel: Option<SidePanel>,
    pub quality: PlaybackQuality,
    pub reduced_motion: bool,
    pub zoom: f32,
    pub pending_load: Option<PendingLoad>,
    pub pending_preload: Option<Option<Entry>>,
    /// The next track whose voice is waiting for its measured bounds: its head
    /// is trimmed when the voice is BUILT, so the answer has to be in hand first.
    preload_wait: Option<Entry>,
    /// Where this session's tracks really start and stop, asked once each.
    edges: EdgesCache,
    /// « Tronquer les silences » and the threshold it is measured at (Réglages →
    /// Lecture writes both, through [`AppState::set_trim`]).
    trim: bool,
    trim_db: f64,
    pub runtime: tokio::runtime::Handle,
    pub events: Sender<AppEvent>,
    /// Holds the `fr.neezer.Desktop` D-Bus name. Dropping it releases it.
    pub instance: Option<Instance>,
    pub started: Instant,
    last_persist: Option<Instant>,
    /// A one-shot for a test that wants to know when the engine boundary was
    /// crossed: the effect a command produced.
    pub last_effect: Option<String>,
    /// A second launch asked for the window: the shell activates it on the next
    /// paint and clears this.
    pub raise_requested: bool,
}

impl AppState {
    /// Build the state. Opens the local store, reads the server and the saved
    /// queue, and starts what can start without waiting (network watch, engine
    /// on its own thread, MPRIS on its own thread). Returns in a few
    /// milliseconds; the first frame does not wait for any of it.
    pub fn new(
        config: AppConfig,
        runtime: tokio::runtime::Handle,
        events: Sender<AppEvent>,
        instance: Option<Instance>,
    ) -> Self {
        let (cache, storage_error) = match open_cache(&config) {
            Ok((cache, error)) => (Some(cache), error),
            Err(error) => (None, Some(error.into())),
        };
        let session = Arc::new(match &config.session_file {
            Some(path) => SessionJar::at(path.clone()),
            None => SessionJar::default_jar(),
        });
        let stored_server = cache
            .as_ref()
            .and_then(|cache| cache.kv().get::<String>(net::SERVER_KEY));
        let server = config
            .server
            .clone()
            .or(stored_server)
            .unwrap_or_else(|| net::DEFAULT_SERVER.to_owned());

        let quality = cache
            .as_ref()
            .and_then(|cache| cache.kv().get::<String>("playback.quality"))
            .map(|raw| PlaybackQuality::parse(&raw))
            .unwrap_or_default();
        let reduced_motion = cache
            .as_ref()
            .and_then(|cache| cache.kv().get::<bool>("ui.reduce_motion"))
            .unwrap_or(false);
        // Silence trimming is ON by default, exactly like the web player
        // (`stores.js#trimSilence`): a master's beat of digital silence is not
        // something to play. The threshold is the SPA's (-45 dBFS), bounded like
        // the server bounds it (`edges.py` MIN/MAX_THRESHOLD).
        let trim = cache
            .as_ref()
            .and_then(|cache| cache.kv().get::<bool>(settings_keys::TRIM))
            .unwrap_or(true);
        let trim_db = cache
            .as_ref()
            .and_then(|cache| cache.kv().get::<f64>(settings_keys::TRIM_DB))
            .unwrap_or(crate::screens::settings::TRIM_DB_DEFAULT)
            .clamp(-90., -20.);

        let mut state = Self {
            config,
            cache,
            storage_error,
            session,
            api: None,
            server: server.clone(),
            session_cause: None,
            auth: Auth::LoggedOut,
            login: LoginUi::default(),
            player: PlayerState::new(),
            nav: Nav::new(Route::Login),
            toasts: Toasts::default(),
            online: true,
            probing: false,
            audio: None,
            audio_error: None,
            mpris: None,
            mpris_track: None,
            position_clock: None,
            features: Features::default(),
            viz: None,
            side_panel: None,
            quality,
            reduced_motion,
            zoom: 1.0,
            pending_load: None,
            pending_preload: None,
            preload_wait: None,
            edges: EdgesCache::default(),
            trim,
            trim_db,
            runtime,
            events,
            instance,
            started: Instant::now(),
            last_persist: None,
            last_effect: None,
            raise_requested: false,
        };
        state.restore_queue();
        state.install_api(&server, /* persist */ false);
        if state.api.as_ref().is_some_and(|api| api.has_session()) {
            state.auth = Auth::Restoring;
            let api = state.api.clone().expect("an api was just installed");
            net::spawn_restore(api, &state.runtime, state.events.clone());
        }
        if state.config.start_mpris {
            mpris_wiring::spawn(state.runtime.clone(), state.events.clone());
        }
        if state.api.is_some() {
            state.start_audio();
        }
        state
    }

    pub fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    pub fn api(&self) -> Option<Arc<Api>> {
        self.api.clone()
    }

    /// Rebuild the client for `server` and start following its reachability.
    /// The cookie jar is shared, so the same account is found again after a
    /// restart; the cache is keyed by owner, so two accounts never mix.
    pub fn install_api(&mut self, server: &str, persist: bool) {
        let Some(cache) = self.cache.as_ref() else {
            self.storage_error = Some(
                "Le stockage local est indisponible : la session ne sera pas conservée.".into(),
            );
            return;
        };
        let server = match net::normalise(server) {
            Ok(server) => server,
            Err(error) => {
                self.login.error = Some(error.into());
                return;
            }
        };
        match net::build_api(&server, cache, Arc::clone(&self.session)) {
            Ok(api) => {
                let api = Arc::new(api);
                net::spawn_online_watch(&api, &self.runtime, self.events.clone());
                self.online = api.is_online();
                self.api = Some(api);
                self.server = server.clone();
                if persist && let Err(error) = cache.kv().set(net::SERVER_KEY, &server) {
                    warn!(%error, "l'adresse du serveur n'a pas pu être enregistrée");
                }
            }
            Err(error) => {
                self.login.error = Some(error.cause_fr().into());
            }
        }
    }

    /// Start the offline worker for this session (lot D13).
    ///
    /// One call, here, because the worker is what makes a download queued before
    /// a restart pick itself up again: the queue lives in `nz-cache`, and the
    /// thread that drains it has to exist before the person opens Réglages →
    /// Téléchargements. `nice 10` and `Lane::Bulk` are the worker's own.
    pub fn start_offline(&mut self) -> Option<crate::wiring::offline::Offline> {
        let offline = crate::wiring::offline::attach(self);
        if let Some(service) = &offline {
            let snapshot = service.snapshot();
            debug!(
                pending = snapshot.progress.pending,
                done = snapshot.progress.done,
                "téléchargements hors ligne"
            );
        }
        offline
    }

    /// Start the engine once, on its own thread.
    pub fn start_audio(&mut self) {
        if self.audio.is_some() || !self.config.start_audio {
            return;
        }
        let (Some(api), Some(cache)) = (self.api.clone(), self.cache.clone()) else {
            return;
        };
        engine::spawn_audio(api, cache, self.runtime.clone(), self.events.clone());
    }

    /// The saved queue, paused at its last position. Restoring is one read
    /// (`nz-cache`, one statement), and the `Load` it produces waits for the
    /// engine like any other.
    fn restore_queue(&mut self) {
        let Some(cache) = self.cache.as_ref() else {
            return;
        };
        let Ok(Some(snapshot)) = cache.queue().load::<Entry>() else {
            return;
        };
        if snapshot.is_empty() {
            return;
        }
        let snapshot = Snapshot {
            queue: snapshot.entries,
            index: snapshot.index as i64,
            position: snapshot.position_ms as f64 / 1000.0,
            ..Snapshot::default()
        };
        let (player, effects) = PlayerState::restore(snapshot);
        self.player = player;
        self.apply_effects(effects);
    }

    /// Send a transport command through the one state machine.
    pub fn command(&mut self, command: Command, cx: &mut Context<Self>) {
        // The listen ledger's `now` is the state machine's, so the playhead it
        // counts from must be the ENGINE's, not the last frame's: with the
        // window hidden the 4 Hz shell tick does not run, and a `next` there
        // used to under-count the seconds just heard. One read on the mixer's
        // own clock, before the command, and the ledger is right whatever the
        // window is doing.
        self.sync_playhead_from_engine();
        let effects = self.player.apply(command, self.now_ms());
        self.apply_effects(effects);
        self.sync_transport();
        self.sync_mpris();
        cx.notify();
    }

    /// `player.current_time` (and therefore the listen ledger) follows the
    /// engine's mixer clock. Called before every command and by the 4 Hz tick;
    /// a paused player has nothing to sync (the clock does not move).
    fn sync_playhead_from_engine(&mut self) {
        if let Some(audio) = &self.audio
            && self.player.playing()
        {
            let position = audio.position();
            if position.is_finite() && (position - self.player.current_time()).abs() > 0.05 {
                self.player.set_position(position);
            }
        }
    }

    /// Ask the server which optional features it has, once per session.
    ///
    /// An older server answers 404 for a route it never had, and the app must
    /// not offer a screen that cannot work (D17). One `OPTIONS` per feature, on
    /// the Prefetch lane, never on a screen's own path; a feature is hidden
    /// only on a definite `404`, never on a doubt (offline, a 5xx).
    fn probe_features(&self) {
        let Some(api) = self.api.clone() else {
            return;
        };
        let events = self.events.clone();
        self.runtime.spawn(async move {
            let mut missing = Vec::new();
            for feature in Feature::ALL {
                if api.has_route(feature.probe()).await == Some(false) {
                    missing.push(feature);
                }
            }
            events.send(AppEvent::Features(missing));
        });
    }

    /// Start the position clock once both halves exist: the engine (the clock)
    /// and the D-Bus server (the sink). Called from either event, so the order
    /// they arrive in does not matter, and replaced when the engine is.
    fn ensure_position_clock(&mut self) {
        if self.position_clock.is_some() {
            return;
        }
        let (Some(audio), Some(ticker)) = (
            self.audio.clone(),
            self.mpris.as_ref().and_then(|mpris| mpris.ticker()),
        ) else {
            return;
        };
        self.position_clock = Some(mpris_wiring::PositionClock::start(ticker, move || {
            audio.position()
        }));
    }

    /// The transport side the state machine does not carry: a play/pause, a
    /// volume or a mute is a `Changed` effect (the state machine has no I/O),
    /// and the engine is told here. A `Load` has already opened the right
    /// track, so this only ever moves the existing one.
    fn sync_transport(&self) {
        let Some(audio) = &self.audio else {
            return;
        };
        if self.player.playing() {
            audio.engine.play();
        } else {
            audio.engine.pause();
        }
        audio
            .engine
            .set_volume(self.player.volume().clamp(0.0, 1.0) as f32);
        audio.engine.set_mute(self.player.muted());
    }

    /// A click on the progress bar: the fraction of the track to land on.
    pub fn seek_fraction(&mut self, fraction: f64, cx: &mut Context<Self>) {
        let duration = self.player.duration();
        if duration <= 0.0 || !fraction.is_finite() {
            return;
        }
        let at = (fraction.clamp(0.0, 1.0) * duration).clamp(0.0, duration);
        self.command(
            Command::Seek {
                args: nz_player::SeekArgs { t: at },
            },
            cx,
        );
    }

    /// Carry out what the state machine asked for. Called from a command, from
    /// an engine event (`Ended`) and from a restore; one path only.
    pub fn apply_effects(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Load {
                    entry,
                    autoplay,
                    start_s,
                } => {
                    self.last_effect = Some(format!(
                        "load:{}:{}:{}",
                        entry.reference().unwrap_or(""),
                        autoplay,
                        start_s
                    ));
                    self.load(entry, autoplay, start_s);
                }
                Effect::PreloadNext(next) => {
                    self.last_effect = Some(match &next {
                        Some(entry) => format!("preload:{}", entry.reference().unwrap_or("")),
                        None => "preload:".to_owned(),
                    });
                    self.preload(next);
                }
                Effect::ReportListen {
                    id,
                    listened_s,
                    next_id,
                    context,
                    shuffle,
                } => self.report_listen(id, listened_s, next_id, context, shuffle),
                Effect::NeedMore { context } => {
                    // D12 owns the Flow/radio top-up; until then a queue ends
                    // where it ends, which is what the SPA does for a context
                    // that cannot be extended.
                    debug!(kind = %context.kind, "file épuisée");
                }
                Effect::Persist => self.persist(),
                Effect::Changed => {}
            }
        }
    }

    fn load(&mut self, entry: Entry, autoplay: bool, start_s: f64) {
        let Some(audio) = self.audio.clone() else {
            self.pending_load = Some(PendingLoad {
                entry,
                autoplay,
                start_s,
            });
            return;
        };
        let reference = match entry.reference() {
            Some(reference) => reference.to_owned(),
            None => return,
        };
        // The metadata goes in BEFORE the voice is built: the first track of a
        // session is normalised like the next one, and a track whose bounds a
        // previous play taught us starts where its music does (`nz-audio`, D6).
        audio
            .engine
            .set_track_meta(&reference, self.meta_for(&entry));
        let quality = self.quality.audio();
        crate::bench::load(&reference);
        audio
            .engine
            .load(&reference, quality, (start_s > 0.0).then_some(start_s));
        if autoplay {
            audio.engine.play();
        } else {
            audio.engine.pause();
        }
        self.player.set_position(start_s);
        if entry.duration > 0 {
            self.player.set_duration(entry.duration as f64);
        }
        // The playing track's own bounds, asked at user priority (rule 1: the
        // wait on this track's answer must never be behind the next one's). Its
        // head is already past by the time an answer arrives, so what this
        // changes is the END — live — and the next play of the same track.
        if self.trim && !self.edges.has(&reference) {
            self.ask_edges(reference, Lane::User);
        }
        self.update_bulk_cap();
    }

    fn preload(&mut self, next: Option<Entry>) {
        // Whatever was waiting before is no longer the next track: this call is
        // the app's last word on what comes after the playing row.
        self.preload_wait = None;
        let Some(audio) = self.audio.clone() else {
            self.pending_preload = Some(next);
            return;
        };
        let Some(entry) = next else {
            // Nothing after the playing row: the second voice starts empty.
            // `nz-audio` has no explicit "clear" (a preload is replaced by the
            // next one), so a silent id is not invented here.
            return;
        };
        let Some(reference) = entry.reference().map(str::to_owned) else {
            return;
        };
        // The bounds must be in hand BEFORE the voice exists: the head trim is a
        // decision taken when the voice is built, and a voice built at 0 plays
        // the silence the trim was for. A prefetch is the only thing that waits
        // for this answer, and it has the whole current track to do it in — it
        // would have yielded to the User lane anyway.
        if self.trim && !self.edges.has(&reference) {
            self.preload_wait = Some(entry);
            self.ask_edges(reference, Lane::Prefetch);
            return;
        }
        audio
            .engine
            .set_track_meta(&reference, self.meta_for(&entry));
        audio.engine.preload_next(&reference, self.quality.audio());
    }

    /// What the engine must know about a queue row before its voice is built.
    fn meta_for(&self, entry: &Entry) -> TrackMeta {
        TrackMeta {
            loudness: entry.gain.map(|gain| gain as f32),
            edges: self.edges_for(entry),
            duration_s: Some(entry.duration as f64),
            podcast: entry.is_podcast(),
        }
    }

    /// The bounds of this row, when trimming is on and they are known.
    fn edges_for(&self, entry: &Entry) -> Option<Edges> {
        if !self.trim {
            return None;
        }
        entry
            .reference()
            .and_then(|id| self.edges.get(id).flatten())
    }

    /// Ask the server where this track's audio starts and stops.
    ///
    /// Fire-and-forget on `nz-net`, once per track and threshold: the answer
    /// comes back as [`AppEvent::Edges`], and a failure is logged and forgotten
    /// (nothing is remembered, so the next play asks once more — a dead link is
    /// never a verdict about a track, rule 3).
    fn ask_edges(&self, id: String, lane: Lane) {
        let Some(api) = self.api.clone() else {
            return;
        };
        let db = self.trim_db;
        let events = self.events.clone();
        self.runtime.spawn(async move {
            let answer = match api.audio_edges_on(lane, &id, db).await {
                Ok(answer) if answer.ready => {
                    answer.start.zip(answer.end).zip(answer.duration).map(
                        |((start, end), duration)| Edges {
                            start,
                            end,
                            duration,
                        },
                    )
                }
                // `ready: false`: not on disk, or not measured yet. No trimming
                // for this one, and the session remembers that.
                Ok(_) => None,
                Err(error) => {
                    debug!(%error, track = %id, "bornes de silence non mesurées");
                    events.send(AppEvent::Edges {
                        id,
                        answer: Err(error),
                    });
                    return;
                }
            };
            events.send(AppEvent::Edges {
                id,
                answer: Ok(answer),
            });
        });
    }

    /// The trim switch and its threshold, as Réglages → Lecture sets them.
    ///
    /// The sound changes NOW, not at the next launch: turning trimming off drops
    /// the bounds of what is playing (the voice's stop is lifted), turning it on
    /// asks for them, and a new threshold throws away answers measured at the old
    /// one because a threshold is part of the measurement.
    pub fn set_trim(&mut self, enabled: bool, db: f64) {
        let db = db.clamp(-90., -20.);
        let db_changed = (db - self.trim_db).abs() > 0.001;
        self.trim = enabled;
        self.trim_db = db;
        if db_changed || !enabled {
            self.edges = EdgesCache::default();
        }
        let Some(entry) = self.player.current().cloned() else {
            return;
        };
        let Some(reference) = entry.reference().map(str::to_owned) else {
            return;
        };
        if let Some(audio) = self.audio.clone() {
            audio
                .engine
                .set_track_meta(&reference, self.meta_for(&entry));
        }
        if enabled && !self.edges.has(&reference) {
            self.ask_edges(reference, Lane::User);
        }
    }

    /// Is silence trimming on, and at what threshold? For Réglages' own view of
    /// the setting (the switch reflects the stored value, not a default).
    pub fn trim(&self) -> (bool, f64) {
        (self.trim, self.trim_db)
    }

    fn report_listen(
        &mut self,
        id: String,
        listened_s: u32,
        next_id: Option<String>,
        context: Option<PlayerContext>,
        shuffle: bool,
    ) {
        if listened_s == 0 {
            return;
        }
        let Some(api) = self.api.clone() else {
            return;
        };
        let payload = ListenPayload {
            id: nz_model::TrackId::new(id),
            listened: listened_s,
            next_id: next_id.map(nz_model::TrackId::new),
            context: context.map(|context| ListenContext {
                kind: context.kind,
                id: context.id,
                title: context.title,
                artist: None,
            }),
            shuffle,
        };
        self.runtime.spawn(async move {
            if let Err(error) = api.report_listen(&payload).await {
                // A listen is accounting, not the person's action: it never
                // produces a toast, and offline it is skipped by the endpoint.
                debug!(%error, "écoute non enregistrée");
            }
        });
    }

    fn persist(&mut self) {
        let now = Instant::now();
        if let Some(last) = self.last_persist
            && now.duration_since(last) < PERSIST_MIN_INTERVAL
        {
            return;
        }
        self.last_persist = Some(now);
        let Some(cache) = self.cache.clone() else {
            return;
        };
        let snapshot = self.player.snapshot();
        let index = snapshot.index.max(0) as usize;
        let position_ms = (snapshot.position * 1000.0).max(0.0) as u64;
        let context = snapshot.context.map(|context| context.kind);
        let queue = QueueSnapshot::new(index, position_ms, context, snapshot.queue);
        self.runtime.spawn_blocking(move || {
            if let Err(error) = cache.queue().save(&queue) {
                warn!(%error, "la file n'a pas pu être enregistrée");
            }
        });
    }

    /// End the session: the server forgets it, the local copy stops showing the
    /// previous person's queue, and the login screen is the next screen.
    pub fn logout(&mut self, cx: &mut Context<Self>) {
        if let Some(api) = self.api.clone() {
            self.runtime.spawn(async move {
                let _ = api.logout().await;
            });
        }
        self.auth = Auth::LoggedOut;
        self.login = LoginUi::default();
        self.session_cause = None;
        self.player = PlayerState::new();
        self.pending_load = None;
        self.pending_preload = None;
        self.preload_wait = None;
        // Another person's session keeps nothing of this one: the measured
        // bounds are re-asked on the next play instead of being trusted.
        self.edges = EdgesCache::default();
        self.mpris_track = None;
        if let Some(handle) = &self.mpris {
            handle.set_status(nz_mpris::PlaybackStatus::Stopped);
        }
        self.audio = None;
        self.viz = None;
        if let Some(cache) = &self.cache {
            let _ = cache.queue().clear();
        }
        self.nav.reset(Route::Login);
        cx.notify();
    }

    /// The quality chip's click: the next rung of the ladder, saved locally
    /// (the SPA keeps this one on the device, not on the server), announced so
    /// a change during a track does not look like it did nothing.
    pub fn cycle_quality(&mut self, cx: &mut Context<Self>) {
        self.quality = match self.quality {
            PlaybackQuality::Auto => PlaybackQuality::Opus320,
            PlaybackQuality::Opus320 => PlaybackQuality::Opus128,
            PlaybackQuality::Opus128 => PlaybackQuality::Opus64,
            PlaybackQuality::Opus64 => PlaybackQuality::Flac,
            PlaybackQuality::Flac => PlaybackQuality::Auto,
        };
        if let Some(cache) = &self.cache
            && let Err(error) = cache
                .kv()
                .set("playback.quality", &self.quality.key().to_owned())
        {
            warn!(%error, "la qualité n'a pas pu être enregistrée");
        }
        self.toasts.push(
            ToastKind::Info,
            format!(
                "Qualité : {} — au prochain titre",
                quality_label(self.quality)
            ),
        );
        cx.notify();
    }

    /// A nice-to-have the player bar shows: the playing track's cover, when the
    /// cache already has it.
    pub fn cover_path(&self, entry: &Entry) -> Option<PathBuf> {
        let cover = entry.cover.as_ref()?;
        self.cache.as_ref()?.covers().file(cover)
    }

    /// Log in to `server` with a password. The answer sets the cookie; the
    /// screen that called this stays on `Auth::Restoring` until it lands.
    pub fn login(&mut self, server: String, username: String, password: String) {
        if username.trim().is_empty() {
            self.login.error = Some("indiquez votre identifiant".into());
            return;
        }
        let normalised = match net::normalise(&server) {
            Ok(server) => server,
            Err(error) => {
                self.login.error = Some(error.into());
                return;
            }
        };
        if self.server != normalised || self.api.is_none() {
            self.install_api(&normalised, true);
        }
        let Some(api) = self.api.clone() else {
            return;
        };
        self.login.pending = true;
        self.login.error = None;
        self.auth = Auth::Restoring;
        self.session_cause = None;
        let events = self.events.clone();
        self.runtime.spawn(async move {
            let result = api.login(&username, &password).await.map(Box::new);
            events.send(AppEvent::LoggedIn(result));
        });
    }

    /// Ask for `/api/me` again: the `Retry` of a `LoadFailed` and of the toast a
    /// failed restore leaves.
    pub fn retry_session(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else {
            self.install_api(&self.server.clone(), false);
            self.retry_session(cx);
            return;
        };
        self.auth = Auth::Restoring;
        self.session_cause = None;
        net::spawn_restore(api, &self.runtime, self.events.clone());
        cx.notify();
    }

    /// Probe the server once. `/api/version` is the cheapest route and the one
    /// that also answers before a session is restored.
    pub fn probe(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else {
            return;
        };
        if self.probing {
            return;
        }
        self.probing = true;
        net::spawn_probe(api, &self.runtime, self.events.clone());
        cx.notify();
    }

    pub fn retry(&mut self, cx: &mut Context<Self>) {
        match self.auth {
            Auth::Failed => self.retry_session(cx),
            _ => match self.toasts.top_retry() {
                Some(Retry::Probe) | None => self.probe(cx),
                Some(Retry::Restore) => self.retry_session(cx),
                Some(Retry::Play) => {
                    if let Some(entry) = self.player.current().cloned() {
                        let start = self.player.current_time();
                        self.apply_effects(vec![Effect::Load {
                            entry,
                            autoplay: true,
                            start_s: start,
                        }]);
                    }
                }
            },
        }
    }

    /// Apply what a background thread reported. Every arm is small; the work
    /// itself already happened elsewhere.
    pub fn on_event(&mut self, event: AppEvent, cx: &mut Context<Self>) {
        match event {
            AppEvent::AudioReady(Ok(audio)) => {
                self.audio_error = None;
                self.viz = VizService::start(&audio.engine, audio.sample_rate);
                self.audio = Some(*audio);
                // A new engine is a new clock: the old thread read the old one.
                self.position_clock = None;
                self.ensure_position_clock();
                // The saved sound, applied when the ENGINE arrives — not when
                // the person opens Réglages (D14a's seam): a session that never
                // opens the page must sound like the settings say, and a cold
                // start must not run on the engine's defaults until someone
                // looks. `Values::load` is the same read the page makes, so the
                // two cannot disagree.
                let values = crate::screens::settings::Values::load(self.cache.as_ref());
                values.apply_to(self.audio.as_ref());
                if let Some(device) = values.device.as_deref()
                    && let Some(audio) = self.audio.as_ref()
                    && let Err(error) = audio.engine.set_output_device(Some(device))
                {
                    // A machine with no switchable output says so once, quietly:
                    // the device may still be the only one, and a toast on every
                    // launch would be noise. A real device that fails to open
                    // reports itself through `EngineEvent::Error`.
                    debug!(%error, device, "sortie enregistrée indisponible");
                }
                if let Some(preload) = self.pending_preload.take() {
                    self.preload(preload);
                }
                if let Some(pending) = self.pending_load.take() {
                    self.load(pending.entry, pending.autoplay, pending.start_s);
                    self.sync_transport();
                    self.sync_mpris();
                }
                cx.notify();
            }
            AppEvent::AudioReady(Err(message)) => {
                self.audio_error = Some(message.clone().into());
                self.toasts
                    .push_with(ToastKind::Error, message, Some(Retry::Play));
                cx.notify();
            }
            AppEvent::MprisReady(handle) => {
                self.mpris = Some(handle);
                self.ensure_position_clock();
                self.sync_mpris();
                cx.notify();
            }
            AppEvent::Features(missing) => {
                self.features = Features::from_missing(missing);
                cx.notify();
            }
            AppEvent::Mpris(command) => self.on_mpris(command, cx),
            AppEvent::Engine(event) => self.on_engine(event, cx),
            AppEvent::Online(online) => {
                self.online = online;
                cx.notify();
            }
            AppEvent::Probed(reachable) => {
                self.probing = false;
                self.online = reachable;
                if !reachable {
                    self.toasts.push_with(
                        ToastKind::Error,
                        "le serveur ne répond pas (réseau)",
                        Some(Retry::Probe),
                    );
                }
                cx.notify();
            }
            AppEvent::Restored(result) => self.on_restored(result, cx),
            AppEvent::LoggedIn(result) => self.on_logged_in(result, cx),
            AppEvent::Edges { id, answer } => {
                if let Ok(edges) = &answer
                    && self.trim
                {
                    self.edges.remember(&id, *edges);
                }
                // The next track's voice was waiting for exactly this answer, an
                // answer that failed included: a measurement that never arrived
                // must not hold a track back (rule 3). Building it now means the
                // head trim is applied, which is the whole reason to wait.
                let waiting = self
                    .preload_wait
                    .take_if(|entry| entry.reference() == Some(id.as_str()));
                if let Some(entry) = waiting {
                    self.preload(Some(entry));
                }
                // The playing track's end follows its bounds immediately
                // (`Player.svelte#audioEndsAt` reads the cache live too); its
                // head is already past.
                if let Some(entry) = self.player.current().cloned()
                    && entry.reference() == Some(id.as_str())
                    && let Some(audio) = self.audio.clone()
                {
                    audio.engine.set_track_meta(&id, self.meta_for(&entry));
                }
                cx.notify();
            }
            AppEvent::Raise => {
                self.raise_requested = true;
                cx.notify();
            }
            AppEvent::Quit => cx.quit(),
            AppEvent::Instance(claim) => match claim {
                crate::wiring::instance::Claim::Primary(instance) => {
                    self.instance = Some(instance);
                }
                crate::wiring::instance::Claim::Secondary => {
                    // The running instance was asked to raise; this one goes.
                    cx.quit();
                }
                crate::wiring::instance::Claim::Unavailable(reason) => {
                    debug!(%reason, "instance unique indisponible");
                }
            },
        }
    }

    fn on_restored(&mut self, result: Result<Box<Me>, ApiError>, cx: &mut Context<Self>) {
        match result {
            Ok(me) => {
                let landing = self
                    .config
                    .start_route
                    .clone()
                    .unwrap_or_else(|| landing_route(&me));
                self.auth = Auth::LoggedIn { me };
                self.session_cause = None;
                self.nav.reset(landing);
                self.start_audio();
                self.start_offline();
                crate::bench::mark("logged_in");
                // Remote control (lot D15a): the CONTROLLER when this session is
                // a loan, the HOST otherwise — one call, so a link that was live
                // before a restart answers again without opening a screen.
                crate::wiring::remote_host::start(self, cx);
                // Écoute partagée (lot D15b): a party this account hosts is
                // picked back up, so a relaunched app does not strand the guests
                // on a link nobody publishes to (the SPA's `maybeResumeHosting`).
                // One read, and only when this store ever hosted one.
                crate::wiring::party_host::resume_at_launch(self, cx);
                self.warm_owner();
                self.probe_features();
                self.sync_mpris();
            }
            Err(ApiError::Unauthorized | ApiError::RemoteEnded { .. }) => {
                self.auth = Auth::LoggedOut;
                self.nav.reset(Route::Login);
            }
            Err(error) => {
                let cause = error.cause_fr();
                self.auth = Auth::Failed;
                self.session_cause = Some(cause.clone().into());
                self.toasts.push_with(
                    ToastKind::Error,
                    format!("Session non confirmée : {cause}"),
                    Some(Retry::Restore),
                );
            }
        }
        cx.notify();
    }

    fn on_logged_in(&mut self, result: Result<Box<Me>, ApiError>, cx: &mut Context<Self>) {
        self.login.pending = false;
        match result {
            Ok(me) => {
                let landing = self
                    .config
                    .start_route
                    .clone()
                    .unwrap_or_else(|| landing_route(&me));
                self.login.error = None;
                self.session_cause = None;
                self.auth = Auth::LoggedIn { me };
                self.nav.reset(landing);
                self.start_audio();
                self.start_offline();
                crate::bench::mark("logged_in");
                // Remote control (lot D15a): the CONTROLLER when this session is
                // a loan, the HOST otherwise — one call, so a link that was live
                // before a restart answers again without opening a screen.
                crate::wiring::remote_host::start(self, cx);
                // Écoute partagée (lot D15b): a party this account hosts is
                // picked back up, so a relaunched app does not strand the guests
                // on a link nobody publishes to (the SPA's `maybeResumeHosting`).
                // One read, and only when this store ever hosted one.
                crate::wiring::party_host::resume_at_launch(self, cx);
                self.warm_owner();
                self.probe_features();
                self.sync_mpris();
            }
            Err(error) => {
                self.auth = Auth::LoggedOut;
                self.login.error = Some(error.cause_fr().into());
            }
        }
        cx.notify();
    }

    /// The cache is keyed by owner, so the first thing after a login is to let
    /// `Api` know whose answers those are (it does it itself on `/me`) and log
    /// which account the store holds.
    fn warm_owner(&self) {
        if let Some(api) = &self.api {
            debug!(owner = ?api.owner(), "session ouverte");
        }
    }

    fn on_engine(&mut self, event: EngineEvent, cx: &mut Context<Self>) {
        match event {
            EngineEvent::Started {
                id,
                position_s,
                duration_s,
            } => {
                crate::bench::started(&id);
                self.player.set_position(position_s);
                if let Some(duration) = duration_s {
                    self.player.set_duration(duration);
                }
                self.sync_mpris();
                cx.notify();
            }
            EngineEvent::Ended { .. } => {
                let effects = self.player.track_ended(self.now_ms());
                self.apply_effects(effects);
                self.sync_mpris();
                cx.notify();
            }
            EngineEvent::Underrun { count } => {
                crate::bench::underrun(count);
                warn!(count, "sous-alimentation du tampon audio");
            }
            EngineEvent::SeekDeferred { target_s } => {
                debug!(
                    target_s,
                    "déplacement différé : le serveur prépare le fichier"
                );
            }
            EngineEvent::Seeked { position_s } => {
                self.player.set_position(position_s);
                self.sync_mpris();
                cx.notify();
            }
            EngineEvent::Error { id, message } => {
                // A failure about a track that is no longer the one playing is
                // STALE, not news: the engine drops cancelled voices without a
                // word (`decode::fail`), and what is left here is an answer that
                // arrived after the person moved on — the next track's prefetch,
                // or a stream the queue left behind. Nothing to offer: the bar
                // already names what plays now.
                let current = self
                    .player
                    .current()
                    .and_then(Entry::reference)
                    .map(str::to_owned);
                if let Some(id) = &id
                    && current.as_deref() != Some(id.as_str())
                {
                    debug!(track = %id, %message, "erreur de lecture périmée");
                    cx.notify();
                    return;
                }
                let cause = match &id {
                    Some(id) => format!("Lecture impossible ({id}) : {message}"),
                    None => format!("Sortie audio : {message}"),
                };
                self.toasts
                    .push_with(ToastKind::Error, cause, Some(Retry::Play));
                cx.notify();
            }
            EngineEvent::OutputChanged {
                device,
                sample_rate,
            } => {
                debug!(%device, sample_rate, "sortie audio");
            }
            EngineEvent::StreamComplete {
                id,
                quality,
                path,
                bytes,
            } => {
                crate::bench::event("stream_complete", &id, Some(bytes as u128));
                if let Some(cache) = self.cache.clone() {
                    let path = path.clone();
                    self.runtime.spawn_blocking(move || {
                        engine::publish_spool(&cache, &id, quality, &path, bytes);
                    });
                }
            }
        }
    }

    fn on_mpris(&mut self, command: MprisCommand, cx: &mut Context<Self>) {
        match command {
            MprisCommand::PlayPause => self.command(
                Command::Toggle {
                    args: Default::default(),
                },
                cx,
            ),
            MprisCommand::Play => self.command(
                Command::Play {
                    args: Default::default(),
                },
                cx,
            ),
            MprisCommand::Pause => self.command(
                Command::Pause {
                    args: Default::default(),
                },
                cx,
            ),
            MprisCommand::Next => self.command(
                Command::Next {
                    args: Default::default(),
                },
                cx,
            ),
            MprisCommand::Previous => self.command(
                Command::Prev {
                    args: Default::default(),
                },
                cx,
            ),
            MprisCommand::Seek(offset_us) => {
                let at = (self.player.current_time() + offset_us as f64 / 1_000_000.0).max(0.0);
                self.command(
                    Command::Seek {
                        args: nz_player::SeekArgs { t: at },
                    },
                    cx,
                );
            }
            MprisCommand::SetPosition(id, position_us) => {
                let current = self
                    .player
                    .current()
                    .and_then(|entry| entry.reference().map(str::to_owned));
                if current.as_deref() == Some(id.as_str()) {
                    let at = position_us as f64 / 1_000_000.0;
                    self.command(
                        Command::Seek {
                            args: nz_player::SeekArgs { t: at },
                        },
                        cx,
                    );
                }
            }
            MprisCommand::Volume(volume) => {
                self.command(
                    Command::SetVolume {
                        args: nz_player::VolumeArgs { v: volume },
                    },
                    cx,
                );
            }
            MprisCommand::Shuffle(on) => {
                if self.player.shuffle() != on {
                    self.command(
                        Command::ToggleShuffle {
                            args: Default::default(),
                        },
                        cx,
                    );
                }
            }
            MprisCommand::Loop(loop_status) => {
                use nz_mpris::LoopStatus;
                let target = match loop_status {
                    LoopStatus::None => Repeat::Off,
                    LoopStatus::Playlist => Repeat::All,
                    LoopStatus::Track => Repeat::One,
                };
                // The state machine exposes repeat as a cycle (the SPA's key);
                // cycling at most twice reaches any of the three states.
                for _ in 0..3 {
                    if self.player.repeat() == target {
                        break;
                    }
                    self.command(
                        Command::CycleRepeat {
                            args: Default::default(),
                        },
                        cx,
                    );
                }
            }
            MprisCommand::Raise => {
                self.toasts.push(ToastKind::Info, "Neezer est déjà ouvert");
                cx.notify();
            }
            MprisCommand::Quit => cx.quit(),
        }
    }

    /// Publish what the bar shows to MPRIS.
    pub fn sync_mpris(&mut self) {
        let Some(handle) = &self.mpris else {
            return;
        };
        let duration = self.player.duration();
        let now = mpris_wiring::NowPlaying {
            id: self
                .player
                .current()
                .and_then(|entry| entry.reference().map(str::to_owned))
                .unwrap_or_default(),
            title: self
                .player
                .current()
                .map(|entry| entry.title.clone())
                .unwrap_or_default(),
            artist: self
                .player
                .current()
                .map(|entry| entry.artist_name().to_owned())
                .unwrap_or_default(),
            album: self
                .player
                .current()
                .and_then(|entry| entry.album.as_ref())
                .map(|album| album.title.clone())
                .unwrap_or_default(),
            duration,
            playing: self.player.playing(),
            position: self.player.current_time(),
            volume: self.player.volume(),
            shuffle: self.player.shuffle(),
            repeat: self.player.repeat(),
        };
        mpris_wiring::publish(handle, &now, &mut self.mpris_track);
    }

    /// The 4 Hz refresh: the playhead, MPRIS, and the toast clock. One task,
    /// started by the shell, so a paused app wakes for nothing else.
    pub fn tick(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        let before = self.player.current_time();
        self.sync_playhead_from_engine();
        if self.player.current_time() != before {
            // MPRIS is already told by its own clock (D17); the bar and the
            // queue draw from `player.current_time`, so this is the redraw.
            changed = true;
        }
        if self.toasts.prune(Instant::now()) {
            changed = true;
        }
        if crate::bench::active()
            && let Some(audio) = &self.audio
        {
            let (sleeves_bytes, _) = cx
                .default_global::<crate::widgets::cover::Sleeves>()
                .resident();
            crate::bench::status(
                audio.engine.underruns(),
                self.player.current_time(),
                self.player.len(),
                sleeves_bytes,
            );
        }
        if changed {
            cx.notify();
        }
    }

    /// The playing stream's measured throughput caps Bulk work (rule 1);
    /// until a throughput meter exists, the cap is removed rather than guessed.
    fn update_bulk_cap(&self) {
        if let Some(api) = &self.api {
            api.set_bulk_cap_from_stream(None);
        }
    }

    // --- presentation helpers ---------------------------------------------

    pub fn user_name(&self) -> Option<&str> {
        self.auth.user().map(|me| me.user.name.as_str())
    }

    pub fn is_admin(&self) -> bool {
        self.auth.user().is_some_and(|me| me.user.admin)
    }

    pub fn is_offline(&self) -> bool {
        !self.online
    }

    pub fn current_route(&self) -> Option<&Route> {
        self.nav.current()
    }

    pub fn settings_section(&self) -> SettingsSection {
        match self.nav.current() {
            Some(Route::Settings { section }) => *section,
            _ => SettingsSection::Playback,
        }
    }
}

fn open_cache(config: &AppConfig) -> Result<(Cache, Option<SharedString>), String> {
    let (data_dir, cache_dir) = match (&config.data_dir, &config.cache_dir) {
        (Some(data), Some(cache)) => (data.clone(), cache.clone()),
        _ => {
            let dirs = directories::ProjectDirs::from("fr", "neezer", "neezer")
                .ok_or_else(|| "répertoire de données introuvable".to_owned())?;
            (
                config
                    .data_dir
                    .clone()
                    .unwrap_or_else(|| dirs.data_dir().to_path_buf()),
                config
                    .cache_dir
                    .clone()
                    .unwrap_or_else(|| dirs.cache_dir().to_path_buf()),
            )
        }
    };
    Cache::open_with_cache_dir(data_dir, cache_dir)
        .map(|cache| (cache, None))
        .map_err(|error| error.to_string())
}

/// What the quality chip prints.
pub fn quality_label(quality: PlaybackQuality) -> &'static str {
    match quality {
        PlaybackQuality::Auto => "Auto",
        PlaybackQuality::Flac => "FLAC",
        PlaybackQuality::Opus320 => "Opus 320",
        PlaybackQuality::Opus128 => "Opus 128",
        PlaybackQuality::Opus64 => "Opus 64",
    }
}

/// A track as the queue's own row (`commands.js#slimTrack`): the row's key, the
/// Deezer id only when there is no row yet, and what the lists and the player
/// draw — never a podcast's description or an album's track count.
pub fn entry_from_track(track: &Track) -> Entry {
    let row_id = track.id.as_ref().map(|id| id.as_str());
    let deezer_id = Some(track.deezer_id.as_str());
    Entry {
        id: nz_player::track_key(row_id, deezer_id).unwrap_or_default(),
        deezer_id: track
            .id
            .is_none()
            .then(|| track.deezer_id.as_str().to_owned()),
        title: track.title.clone(),
        duration: track.duration,
        explicit: Some(track.explicit),
        local: Some(track.local),
        podcast: None,
        gain: track.gain,
        unavailable: Some(track.unavailable),
        archived: Some(track.is_archived()),
        published: None,
        channel_id: None,
        cover: track.album.cover.clone(),
        display_artist: track.display_artist.clone(),
        artist: Some(EntryArtist {
            id: None,
            deezer_id: (!track.artist.deezer_id.is_empty())
                .then(|| track.artist.deezer_id.as_str().to_owned()),
            name: track.artist.name.clone(),
            role: None,
        }),
        artists: track
            .artists
            .iter()
            .map(|artist| EntryArtist {
                id: None,
                deezer_id: (!artist.deezer_id.is_empty())
                    .then(|| artist.deezer_id.as_str().to_owned()),
                name: artist.name.clone(),
                role: (!artist.role.is_empty()).then(|| artist.role.clone()),
            })
            .collect(),
        album: (!track.album.title.is_empty()).then(|| EntryAlbum {
            id: None,
            deezer_id: (!track.album.deezer_id.is_empty())
                .then(|| track.album.deezer_id.as_str().to_owned()),
            title: track.album.title.clone(),
            cover: track.album.cover.clone(),
        }),
    }
}

/// Where a session lands once the server has confirmed it.
///
/// The SPA's own rule (`webapp/src/App.svelte`: `onboardingRequired = !REMOTE &&
/// stepFor($user.onboarding) !== "done"`, `lib/onboarding.js#stepFor`), and
/// nothing else leads there: a NEW account — invited, or just created — has
/// tastes to pick before the library means anything, and the shell used to open
/// Home and leave `Route::Onboarding` reachable by no key at all. A
/// remote-control loan never onboards: it drives somebody else's session.
/// Which optional features this server answered it does not have.
///
/// The shell reads this to hide a sidebar entry and to show why a screen that
/// was opened by a link (a remote-control URL, a folder saved in a store) says
/// the server does not know it — never an endless spinner (rule 3).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Features {
    missing: std::collections::BTreeSet<Feature>,
}

impl Features {
    pub fn from_missing(missing: impl IntoIterator<Item = Feature>) -> Self {
        Self {
            missing: missing.into_iter().collect(),
        }
    }

    /// Whether the server said it does not have this feature.
    pub fn hidden(&self, feature: Feature) -> bool {
        self.missing.contains(&feature)
    }

    /// What to show: the feature's own name when the server lacks it.
    pub fn absent(&self, route: &Route) -> Option<&'static str> {
        route
            .feature()
            .filter(|feature| self.hidden(*feature))
            .map(|feature| feature.absent_fr())
    }

    /// How many features are missing (the diagnostic log prints this).
    pub fn missing(&self) -> usize {
        self.missing.len()
    }
}

pub fn landing_route(me: &Me) -> Route {
    if me.remote.is_none() && onboarding_pending(me.user.onboarding.as_deref()) {
        Route::Onboarding
    } else {
        Route::Home
    }
}

/// `stepFor`'s two unfinished steps: `"artists"` and `"account"`. Anything else
/// (including no value at all) is a finished account — an older server that
/// never grew the field must land on Home, not on an onboarding it cannot
/// complete.
pub fn onboarding_pending(step: Option<&str>) -> bool {
    matches!(step, Some("artists" | "account"))
}

/// The server's Quality for an `Entry`'s archived state: what a screen asks
/// when it queues a list and wants the prefetch to use the right quality.
pub fn model_quality(quality: PlaybackQuality) -> ModelQuality {
    match quality.audio() {
        nz_audio::Quality::Flac => ModelQuality::Flac,
        nz_audio::Quality::Opus320 => ModelQuality::Opus320,
        nz_audio::Quality::Opus128 => ModelQuality::Opus128,
        nz_audio::Quality::Opus64 => ModelQuality::Opus64,
    }
}

/// The queue row an `Effect::Load` names, for a test that only wants the id.
pub fn effect_reference(effect: &Effect) -> Option<&str> {
    effect.loaded().and_then(Entry::reference)
}

/// The app's event channel, made by the shell and handed to `AppState` and
/// every worker. A convenience so `main` and the tests create it the same way.
pub fn event_channel() -> (Sender<AppEvent>, events::Receiver<AppEvent>) {
    events::channel()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "neezer-d7-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn a_slim_queue_row_is_the_java_scripts_shape() {
        // `commands.js#slimTrack`: a catalogue row (no `id`) carries its Deezer
        // id and gets the uuid5 key; a library row keeps its UUID and DROPS the
        // Deezer id, because the same row after a remote round trip must parse
        // back into the same type.
        let catalogue: Track = serde_json::from_str(
            r#"{"deezer_id":"3000001","title":"Balancing the Dark","duration":312,
                "artist":{"deezer_id":"42","name":"Kaya Project"},
                "album":{"deezer_id":"9","title":"Songs from the Inside","cover":"/api/cover/9"}}"#,
        )
        .expect("catalogue track");
        let entry = entry_from_track(&catalogue);
        assert_eq!(entry.id, "21408a08-d175-53a4-a02d-733a7bc83812");
        assert_eq!(entry.deezer_id.as_deref(), Some("3000001"));
        assert_eq!(entry.title, "Balancing the Dark");
        assert_eq!(entry.artist.as_ref().unwrap().name, "Kaya Project");
        assert_eq!(
            entry.cover.as_deref(),
            Some("/api/cover/9"),
            "the cover stays relative to the server"
        );

        let library: Track = serde_json::from_str(
            r#"{"id":"3f7c1d2e-9a4b-5c6d-8e0f-1a2b3c4d5e6f",
                "deezer_id":"3000001","title":"Balancing the Dark","duration":312,
                "artist":{"deezer_id":"42","name":"Kaya Project"},
                "album":{"deezer_id":"9","title":"Songs from the Inside"}}"#,
        )
        .expect("library track");
        let entry = entry_from_track(&library);
        assert_eq!(entry.id, "3f7c1d2e-9a4b-5c6d-8e0f-1a2b3c4d5e6f");
        assert_eq!(
            entry.deezer_id, None,
            "a row id wins and deezer_id is dropped"
        );
        assert_eq!(
            entry.reference(),
            Some("3f7c1d2e-9a4b-5c6d-8e0f-1a2b3c4d5e6f")
        );
    }

    #[test]
    fn a_queue_row_round_trips_through_its_own_json() {
        let entry = Entry {
            id: "abc".into(),
            deezer_id: Some("42".into()),
            title: "T".into(),
            duration: 3,
            explicit: Some(false),
            ..Entry::default()
        };
        let json = serde_json::to_string(&entry).expect("serialize");
        let back: Entry = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, entry);
        assert_eq!(entry.title, back.title);
    }

    #[test]
    fn the_app_starts_without_a_server_and_offers_the_default_address() {
        let dir = temp_dir("start");
        let (tx, _rx) = event_channel();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let state = AppState::new(
            AppConfig {
                server: Some(net::DEFAULT_SERVER.to_owned()),
                start_audio: false,
                start_mpris: false,
                ..AppConfig::for_test(&dir)
            },
            runtime.handle().clone(),
            tx,
            None,
        );
        assert_eq!(state.server, net::DEFAULT_SERVER);
        assert_eq!(state.auth, Auth::LoggedOut);
        assert!(state.api.is_some());
        std::fs::remove_dir_all(dir).ok();
    }

    /// D17: a server that answered 404 for a feature's probe has it hidden —
    /// the sidebar entry is not drawn and a screen reached by a link says why.
    #[test]
    fn a_feature_the_server_lacks_is_hidden_and_named() {
        let features = Features::from_missing([Feature::Party]);
        assert!(features.hidden(Feature::Party));
        assert!(!features.hidden(Feature::Genres));
        assert_eq!(features.absent(&Route::Party), Some("L'écoute partagée"));
        assert_eq!(features.absent(&Route::Home), None);
        assert_eq!(features.missing(), 1);
        // Nothing missing is the shipped case for a current server: every
        // optional screen stays reachable.
        let none = Features::default();
        for route in [
            Route::Party,
            Route::Remote,
            Route::Genres,
            Route::Settings {
                section: SettingsSection::Transcribe,
            },
        ] {
            assert_eq!(none.absent(&route), None, "{route:?}");
        }
    }

    #[test]
    fn a_saved_queue_comes_back_paused_at_its_position() {
        let dir = temp_dir("queue");
        let cache = Cache::open_with_cache_dir(dir.join("data"), dir.join("cache")).expect("cache");
        let entries: Vec<Entry> = (0..3)
            .map(|index| Entry {
                id: format!("track-{index}"),
                title: format!("Titre {index}"),
                duration: 100 + index,
                ..Entry::default()
            })
            .collect();
        cache
            .queue()
            .save(&QueueSnapshot::new(1, 42_500, None, entries))
            .expect("save");
        drop(cache);

        let (tx, _rx) = event_channel();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let state = AppState::new(
            AppConfig {
                server: Some(net::DEFAULT_SERVER.to_owned()),
                start_audio: false,
                start_mpris: false,
                ..AppConfig::for_test(&dir)
            },
            runtime.handle().clone(),
            tx,
            None,
        );
        assert_eq!(state.player.len(), 3);
        assert_eq!(state.player.index(), 1);
        assert_eq!(state.player.current_time(), 42.5);
        assert!(
            !state.player.playing(),
            "a restored queue waits for a click"
        );
        let pending = state.pending_load.expect("the load waits for the engine");
        assert!(!pending.autoplay);
        assert_eq!(pending.start_s, 42.5);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_login_screen_refuses_an_empty_user_before_any_request() {
        let dir = temp_dir("login");
        let (tx, _rx) = event_channel();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let mut state = AppState::new(
            AppConfig {
                server: Some(net::DEFAULT_SERVER.to_owned()),
                start_audio: false,
                start_mpris: false,
                ..AppConfig::for_test(&dir)
            },
            runtime.handle().clone(),
            tx,
            None,
        );
        state.login(
            net::DEFAULT_SERVER.to_owned(),
            "  ".to_owned(),
            "secret".to_owned(),
        );
        assert_eq!(state.auth, Auth::LoggedOut);
        assert!(state.login.error.is_some());
        std::fs::remove_dir_all(dir).ok();
    }

    fn rss_kb() -> i64 {
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        status
            .lines()
            .find_map(|line| line.strip_prefix("VmRSS:"))
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse().ok())
            .unwrap_or(0)
    }

    /// The worst realistic input in one process: an 8 000-row library page and
    /// a 4 000-row queue. The rows are shared pointers in the queue and a plain
    /// `Vec<Entry>` for the list, so this measures the data model D8a will
    /// virtualise — the widget count never grows with either.
    #[test]
    fn an_eight_thousand_row_list_and_a_four_thousand_row_queue_stay_small() {
        let before = rss_kb();
        let library: Vec<Entry> = (0..8_000)
            .map(|index| Entry {
                id: format!("track-{index}"),
                title: format!("Titre {index}"),
                duration: 200,
                artist: Some(EntryArtist {
                    name: format!("Artiste {index}"),
                    ..EntryArtist::default()
                }),
                ..Entry::default()
            })
            .collect();
        let queue: Vec<Entry> = (0..4_000)
            .map(|index| Entry {
                id: format!("queue-{index}"),
                title: format!("File {index}"),
                duration: 180,
                ..Entry::default()
            })
            .collect();
        let mut player = PlayerState::new();
        let _ = player.apply(
            Command::PlayQueue {
                args: nz_player::QueueArgs {
                    tracks: queue,
                    start: 0,
                    context: None,
                },
            },
            0,
        );
        let after = rss_kb();
        println!(
            "RSS with an 8 000-row list and a 4 000-row queue: {before} KB -> {after} KB (+{} KB)",
            after - before
        );
        assert_eq!(player.len(), 4_000);
        assert!(library.len() == 8_000);
        assert!(
            after - before < 40 * 1024,
            "the two worst-case lists cost {} KB of RSS",
            after - before
        );
    }

    /// `ffmpeg` writes a FLAC whose first `head` seconds are digital silence,
    /// then a tone, then `tail` seconds of silence: the same fixture
    /// `nz-audio`'s trim tests use. The desktop CI installs ffmpeg for the audio
    /// crate's own real-audio tests, so this is the same tool.
    fn silence_tone_silence(path: &Path, head: f64, seconds: f64, tail: f64) {
        let expression = format!(
            "if(lt(t\\,{head})\\,0\\,if(lt(t\\,{})\\,0.25*cos(2*PI*440*t)\\,0))",
            head + seconds
        );
        let source = format!(
            "aevalsrc={expression}:s=48000:d={}:c=stereo",
            head + seconds + tail
        );
        let status = std::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
            .arg("-i")
            .arg(&source)
            .args(["-c:a", "flac"])
            .arg(path)
            .status()
            .expect("ffmpeg is installed (the desktop CI installs it)");
        assert!(status.success(), "ffmpeg failed for {source}");
    }

    /// A `StreamOpener` over files on disk: the engine reads them directly (the
    /// play cache's answer), so a test needs no server for the audio itself.
    struct Files(std::collections::HashMap<String, PathBuf>);

    impl nz_audio::StreamOpener for Files {
        fn open(
            &self,
            id: &str,
            _quality: nz_audio::Quality,
            _start: Option<f64>,
            _from_byte: Option<u64>,
            _lane: nz_audio::Lane,
        ) -> Result<Box<dyn nz_audio::ByteStream>, nz_audio::SourceError> {
            Err(nz_audio::SourceError::Message(format!(
                "{id} est un fichier local"
            )))
        }

        fn ready(
            &self,
            _id: &str,
            _quality: nz_audio::Quality,
        ) -> Result<nz_audio::StreamReady, nz_audio::SourceError> {
            Ok(nz_audio::StreamReady::default())
        }

        fn local(&self, id: &str, _quality: nz_audio::Quality) -> Option<PathBuf> {
            self.0.get(id).cloned()
        }
    }

    /// **The trim, end to end**: the app asks the server where a track's audio
    /// starts and stops, hands the answer to the engine BEFORE the voice is
    /// built, and the file really plays from its music's first sample to its
    /// last — proven on a real audio file.
    ///
    /// The chain: a two-row queue (the tapped track, then the next one) →
    /// `AppState::load` asks the playing track's bounds at USER priority while
    /// `AppState::preload` asks the next one's at PREFETCH priority →
    /// `AppEvent::Edges` lands → the next track's `TrackMeta::edges` is set →
    /// the voice is built at 0.5 s, not at 0.
    ///
    /// Measured on this machine (Rust 1.99.1): the two requests leave in that
    /// order, one ask each, both at `db=-45`; the engine's meta for the next
    /// track holds `{0.5, 2.5, 5.5}`; the rendered window peaks at **0.2500**
    /// (the tone, from the first buffer) and the clock starts at **0.500 s**,
    /// never at 0. Turning the switch off afterwards drops that meta at once.
    #[gpui::test]
    async fn the_servers_bounds_reach_the_engine_before_the_next_voice(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::widgets::card::stub;
        use gpui_kit::AppContext as _;
        use nz_player::QueueArgs;

        let dir = temp_dir("trim-chain");
        let plain = dir.join("plain.flac");
        let trimmed = dir.join("trimmed.flac");
        // A plain second of tone, and the fixture: 0.5 s of silence, 2 s of tone,
        // 3 s of silence.
        silence_tone_silence(&plain, 0.0, 1.0, 0.0);
        silence_tone_silence(&trimmed, 0.5, 2.0, 3.0);

        // What the server answers for the trimmed track (`edges.py`'s payload).
        let answer = r#"{"ready":true,"start":0.5,"end":2.5,"duration":5.5,"threshold":-45.0}"#;
        let server = stub::Stub::start(&[
            ("/api/login", r#"{"user":{"id":"u1","name":"bench"}}"#),
            ("/api/me", r#"{"user":{"id":"u1","name":"bench"}}"#),
            (
                "/api/audio/edges/plain",
                r#"{"ready":false,"duration":1.0}"#,
            ),
            ("/api/audio/edges/trimmed", answer),
        ]);

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let (tx, rx) = event_channel();
        let config = AppConfig {
            server: Some(server.base()),
            start_audio: false,
            start_mpris: false,
            ..AppConfig::for_test(&dir)
        };
        let state = {
            let handle = runtime.handle().clone();
            cx.update(move |cx| cx.new(move |_| AppState::new(config, handle, tx, None)))
        };
        // The shell's own drain, so an `AppEvent::Edges` is handled exactly as
        // the app handles it.
        {
            let state = state.clone();
            cx.update(move |cx| {
                cx.spawn(async move |cx| {
                    while let Some(event) = rx.next().await {
                        state.update(cx, |state, cx| state.on_event(event, cx));
                    }
                })
                .detach();
            });
        }

        // The engine, over the two files, with the sink the test drives.
        let cfg = nz_audio::EngineConfig {
            sample_rate: 48_000,
            spool_dir: dir.join("spool"),
            ..Default::default()
        };
        let files = Files(std::collections::HashMap::from([
            ("plain".to_owned(), plain.clone()),
            ("trimmed".to_owned(), trimmed.clone()),
        ]));
        let (engine, mut sink) = nz_audio::Engine::new_offline(cfg, std::sync::Arc::new(files));
        let engine_for_state = engine.clone();
        cx.update(|cx| {
            state.update(cx, |state, _| {
                state.audio = Some(Audio {
                    engine: engine_for_state,
                    sample_rate: 48_000.,
                });
            });
        });

        // The tapped track, then the next one: the state machine's own effects.
        cx.update(|cx| {
            state.update(cx, |state, cx| {
                state.command(
                    Command::PlayQueue {
                        args: QueueArgs {
                            tracks: vec![
                                Entry {
                                    id: "plain".into(),
                                    title: "Plain".into(),
                                    duration: 1,
                                    ..Entry::default()
                                },
                                Entry {
                                    id: "trimmed".into(),
                                    title: "Trimmed".into(),
                                    duration: 6,
                                    ..Entry::default()
                                },
                            ],
                            start: 0,
                            context: None,
                        },
                    },
                    cx,
                );
            });
        });

        // Pump the network, the UI and the sink until the engine holds the next
        // track's bounds.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut buffer = vec![0.0f32; 960];
        loop {
            runtime.block_on(async {
                tokio::time::sleep(Duration::from_millis(1)).await;
            });
            cx.run_until_parked();
            sink.render(&mut buffer);
            if engine.track_meta("trimmed").edges.is_some() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the bounds never reached the engine (edges asked: {:?})",
                server
                    .hits()
                    .iter()
                    .map(|hit| hit.path.clone())
                    .collect::<Vec<_>>()
            );
        }

        // The wire contract: the playing track first (user priority), the next
        // one second, both at the configured threshold, one ask each.
        let hits: Vec<String> = server
            .hits()
            .iter()
            .filter(|hit| hit.path.starts_with("/api/audio/edges/"))
            .map(|hit| format!("{}?{}", hit.path, hit.query))
            .collect();
        println!("bornes demandées : {hits:?}");
        assert_eq!(
            hits,
            vec![
                "/api/audio/edges/plain?db=-45".to_owned(),
                "/api/audio/edges/trimmed?db=-45".to_owned(),
            ],
            "the playing track first, then the next one — one ask each"
        );

        let meta = engine.track_meta("trimmed");
        assert_eq!(
            meta.edges,
            Some(Edges {
                start: 0.5,
                end: 2.5,
                duration: 5.5
            }),
            "the answer became the voice's bounds before the voice existed"
        );

        // And the sound: the voice starts where the music does. (The engine's own
        // half is `nz-audio`'s
        // `a_measured_head_and_tail_of_silence_are_both_skipped`; this plays the
        // very meta the app built.)
        let events = engine.events();
        engine.load("trimmed", nz_audio::Quality::Flac, None);
        // The mixer consumes its commands only while it renders, so the sink is
        // pumped here too: a paused device would leave the install command
        // queued and the voice would never be built.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut started = false;
        while !started {
            while let Ok(event) = events.try_recv() {
                started |= matches!(event, EngineEvent::Started { id, .. } if id == "trimmed");
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the trimmed voice was never built"
            );
            sink.render(&mut buffer);
        }
        // 0.200 s of audio from wherever the voice starts, counted by the
        // engine's own clock rather than by the wall clock. The decode thread
        // fills its ring in the background, so the sink is pumped until the
        // clock has covered the window (a buffer rendered before the first
        // decoded frame is silence, and correctly does not advance it).
        engine.play();
        let start_position = engine.position();
        let mut peak = 0.0f32;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut played = start_position;
        while played - start_position < 0.2 {
            sink.render(&mut buffer);
            peak = buffer
                .iter()
                .fold(peak, |peak, sample| peak.max(sample.abs()));
            played = engine.position();
            assert!(
                std::time::Instant::now() < deadline,
                "the trimmed voice stopped advancing at {played:.3} s"
            );
        }
        engine.pause();
        println!("départ {start_position:.3} s, crête {peak:.4}, position {played:.3} s");
        assert!(
            (0.49..0.75).contains(&start_position),
            "the voice starts at the audio's own beginning (0.5 s in), got {start_position:.3} s"
        );
        assert!(
            peak > 0.2,
            "the music is there from the first buffer — no 0.5 s of head silence (crest {peak:.4})"
        );
        assert!(
            played - start_position < 0.35,
            "the clock covered the window in one go, got {:.3} s",
            played - start_position
        );

        // The switch takes effect on what is playing. Press next first, so the
        // state machine's own current row is the trimmed track — the app never
        // guesses what the engine plays, it follows the queue.
        cx.update(|cx| {
            state.update(cx, |state, cx| {
                state.command(
                    Command::Next {
                        args: Default::default(),
                    },
                    cx,
                );
            });
        });
        assert_eq!(
            state.read_with(cx, |state, _| state
                .player
                .current()
                .and_then(Entry::reference)
                .map(str::to_owned)),
            Some("trimmed".to_owned()),
            "the queue advanced to the trimmed track"
        );
        cx.update(|cx| {
            state.update(cx, |state, _| state.set_trim(false, -45.0));
        });
        assert_eq!(
            engine.track_meta("trimmed").edges,
            None,
            "off means the playing track's measured bounds go too"
        );
        server.stop();
    }

    /// **The saved sound is applied when the engine arrives**, not when Réglages
    /// is opened: the equaliser, the normalisation level, the bass lift, mono,
    /// the balance and the crossfade all reach the engine at
    /// `AppEvent::AudioReady`, so a session that never opens the page sounds like
    /// the settings say. Before this, the engine ran on its defaults until a
    /// person visited Réglages → Lecture.
    ///
    /// The saved OUTPUT device is attempted too; on a machine whose output is the
    /// silence pump (CI, a box with no card) that is an honest `Err`, and the
    /// engine says so through `EngineEvent::Error` for a real device that fails.
    ///
    /// Measured: the engine starts flat (`eq(0) = 0`, normalisation `Off`) and
    /// reads **6.0 dB on the 31 Hz band, 9.0 dB of bass, `Medium`, mono, −0.30**
    /// after the one event.
    #[gpui::test]
    async fn the_saved_sound_is_applied_when_the_engine_arrives(cx: &mut gpui::TestAppContext) {
        use gpui_kit::AppContext as _;

        let dir = temp_dir("audio-ready");
        // What Réglages wrote in an earlier session.
        {
            let cache =
                Cache::open_with_cache_dir(dir.join("data"), dir.join("cache")).expect("cache");
            let kv = cache.kv();
            kv.set("audio.eq", &[6., 5., 4., 2., 0., 0., 0., 0., 0., 0.])
                .expect("eq");
            kv.set("audio.normalization", &"medium".to_owned())
                .expect("norm");
            kv.set("audio.bass", &9.0_f32).expect("bass");
            kv.set("audio.mono", &true).expect("mono");
            kv.set("audio.balance", &-0.3_f64).expect("balance");
            kv.set("audio.fade.enabled", &true).expect("fade");
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let (tx, _rx) = event_channel();
        let config = AppConfig {
            server: Some(net::DEFAULT_SERVER.to_owned()),
            start_audio: false,
            start_mpris: false,
            ..AppConfig::for_test(&dir)
        };
        let state = {
            let handle = runtime.handle().clone();
            cx.update(move |cx| cx.new(move |_| AppState::new(config, handle, tx, None)))
        };

        let cfg = nz_audio::EngineConfig {
            sample_rate: 48_000,
            spool_dir: dir.join("spool"),
            ..Default::default()
        };
        let (engine, _sink) = nz_audio::Engine::new_offline(
            cfg,
            std::sync::Arc::new(Files(std::collections::HashMap::new())),
        );
        assert!(engine.eq(0).abs() < 0.01, "the engine starts flat");
        assert_eq!(engine.normalization(), nz_audio::Normalization::Off);

        // The one event a cold start produces.
        let audio = Audio {
            engine: engine.clone(),
            sample_rate: 48_000.,
        };
        cx.update(|cx| {
            state.update(cx, |state, cx| {
                state.on_event(AppEvent::AudioReady(Ok(Box::new(audio))), cx);
            });
        });

        assert!(
            (engine.eq(0) - 6.0).abs() < 0.01,
            "the saved 31 Hz band reached the engine, got {}",
            engine.eq(0)
        );
        assert!((engine.bass() - 9.0).abs() < 0.01, "bass {}", engine.bass());
        assert_eq!(engine.normalization(), nz_audio::Normalization::Medium);
        assert!(engine.mono());
        assert!(
            (engine.balance() + 0.3).abs() < 0.01,
            "balance {}",
            engine.balance()
        );
        assert!(engine.fade().enabled, "the saved crossfade is on");
        std::fs::remove_dir_all(dir).ok();
    }

    /// **An error about a track that is not playing is not a toast.**
    ///
    /// The engine stops a voice the instant the person changes track, and the
    /// read it was in the middle of then fails — D10b saw an error toast on
    /// every fast next/next. The engine no longer raises those
    /// (`nz-audio`'s `a_stopped_voice_reports_nothing_and_a_live_one_reports_its_cause`),
    /// and this is the app's own guard for an answer that arrives late: it is
    /// logged, and the bar still names what plays now. A failure of the track
    /// that IS playing keeps its cause and its « Réessayer ».
    #[gpui::test]
    async fn a_late_error_for_another_track_is_not_a_toast(cx: &mut gpui::TestAppContext) {
        use gpui_kit::AppContext as _;

        let dir = temp_dir("stale-error");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let (tx, _rx) = event_channel();
        let config = AppConfig {
            server: Some(net::DEFAULT_SERVER.to_owned()),
            start_audio: false,
            start_mpris: false,
            ..AppConfig::for_test(&dir)
        };
        let state = {
            let handle = runtime.handle().clone();
            cx.update(move |cx| cx.new(move |_| AppState::new(config, handle, tx, None)))
        };
        // A queue of two, the first one current — what a track change leaves.
        let (tx, _rx2) = event_channel();
        cx.update(|cx| {
            state.update(cx, |state, _| state.events = tx);
        });
        cx.update(|cx| {
            state.update(cx, |state, cx| {
                state.pending_load = Some(PendingLoad {
                    entry: Entry {
                        id: "a".into(),
                        title: "A".into(),
                        duration: 100,
                        ..Entry::default()
                    },
                    autoplay: false,
                    start_s: 0.0,
                });
                state.on_event(
                    AppEvent::AudioReady(Ok(Box::new(Audio {
                        engine: nz_audio::Engine::new_offline(
                            nz_audio::EngineConfig::default(),
                            std::sync::Arc::new(Files(std::collections::HashMap::new())),
                        )
                        .0,
                        sample_rate: 48_000.,
                    }))),
                    cx,
                );
            });
        });
        // The state machine's own current is what the guard reads.
        cx.update(|cx| {
            state.update(cx, |state, cx| {
                state.command(
                    Command::PlayQueue {
                        args: nz_player::QueueArgs {
                            tracks: vec![
                                Entry {
                                    id: "a".into(),
                                    title: "A".into(),
                                    duration: 100,
                                    ..Entry::default()
                                },
                                Entry {
                                    id: "b".into(),
                                    title: "B".into(),
                                    duration: 100,
                                    ..Entry::default()
                                },
                            ],
                            start: 0,
                            context: None,
                        },
                    },
                    cx,
                );
            });
        });
        let toasts = |cx: &mut gpui::TestAppContext| {
            state.read_with(cx, |state, _| {
                state
                    .toasts
                    .items()
                    .iter()
                    .map(|toast| (toast.kind, toast.message.clone()))
                    .collect::<Vec<_>>()
            })
        };
        let before = toasts(cx).len();

        // What a superseded voice used to raise: a read that died with the track.
        cx.update(|cx| {
            state.update(cx, |state, cx| {
                state.on_event(
                    AppEvent::Engine(EngineEvent::Error {
                        id: Some("b".to_owned()),
                        message: "lecture impossible : corps interrompu".to_owned(),
                    }),
                    cx,
                );
            });
        });
        assert_eq!(
            toasts(cx).len(),
            before,
            "an error about the track that is NOT playing raises nothing"
        );

        // A failure of the track that IS playing keeps its cause and its retry.
        cx.update(|cx| {
            state.update(cx, |state, cx| {
                state.on_event(
                    AppEvent::Engine(EngineEvent::Error {
                        id: Some("a".to_owned()),
                        message: "décodage impossible : bitstream".to_owned(),
                    }),
                    cx,
                );
            });
        });
        let after = toasts(cx);
        assert_eq!(
            after.len(),
            before + 1,
            "the playing track's failure is news"
        );
        assert!(matches!(
            after.last().map(|(kind, _)| kind),
            Some(ToastKind::Error)
        ));
        assert!(
            after
                .last()
                .is_some_and(|(_, message)| message.contains("décodage impossible")),
            "the cause is kept, never a bare « Lecture impossible »"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    /// **A new account lands on the onboarding**, from the SPA's own rule
    /// (`webapp/src/App.svelte#onboardingRequired`, `lib/onboarding.js#stepFor`):
    /// `"artists"` and `"account"` are unfinished, anything else is done. Before
    /// this, `Route::Onboarding` (D12's « Bienvenue » screen) existed and no key,
    /// click or login ever reached it.
    ///
    /// A remote-control loan never onboards — it drives somebody else's session,
    /// and an account whose field is absent (an older server) must land on Home,
    /// not on a step it cannot complete.
    #[test]
    fn a_new_account_lands_on_the_onboarding_and_a_finished_one_on_home() {
        fn me(onboarding: Option<&str>, remote: bool) -> Me {
            Me {
                user: nz_model::User {
                    id: nz_model::UserId::new("u1"),
                    name: "bench".to_owned(),
                    onboarding: onboarding.map(str::to_owned),
                    ..nz_model::User::default()
                },
                remote: remote.then(nz_model::RemoteGrant::default),
            }
        }
        for step in ["artists", "account"] {
            assert_eq!(
                landing_route(&me(Some(step), false)),
                Route::Onboarding,
                "{step} is an unfinished onboarding"
            );
            assert_eq!(
                landing_route(&me(Some(step), true)),
                Route::Home,
                "a remote-control loan never onboards"
            );
        }
        for step in [Some("done"), None] {
            assert_eq!(landing_route(&me(step, false)), Route::Home, "{step:?}");
        }
        assert!(onboarding_pending(Some("artists")));
        assert!(onboarding_pending(Some("account")));
        assert!(!onboarding_pending(Some("done")));
        assert!(!onboarding_pending(None));
        assert!(!onboarding_pending(Some("n'importe quoi")));
    }

    /// The trim setting's own value: on by default like the web player, written
    /// by Réglages → Lecture, bounded like the server bounds it — a value from a
    /// build that allowed a lower threshold is clamped, never refused.
    #[test]
    fn the_trim_setting_is_stored_and_bounded() {
        let dir = temp_dir("trim-off");
        let (tx, _rx) = event_channel();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let mut state = AppState::new(
            AppConfig {
                server: Some(net::DEFAULT_SERVER.to_owned()),
                start_audio: false,
                start_mpris: false,
                ..AppConfig::for_test(&dir)
            },
            runtime.handle().clone(),
            tx,
            None,
        );
        assert!(state.trim().0, "trimming is on by default, like the SPA");
        assert_eq!(state.trim().1, -45.0);
        state.set_trim(false, -45.0);
        assert!(!state.trim().0);
        state.set_trim(true, -30.0);
        assert_eq!(state.trim(), (true, -30.0));
        state.set_trim(true, -120.0);
        assert_eq!(
            state.trim().1,
            -90.0,
            "the server's own floor (`edges.py` MIN_THRESHOLD)"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_servers_quality_and_the_engines_agree_for_the_picker() {
        assert_eq!(model_quality(PlaybackQuality::Flac), ModelQuality::Flac);
        assert_eq!(model_quality(PlaybackQuality::Auto), ModelQuality::Opus320);
        assert_eq!(ModelQuality::Opus320.wire(), Some("OPUS_320"));
    }
}
