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

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::{Context, SharedString};
use nz_api::{Api, ApiError, SessionJar};
use nz_audio::{EngineEvent, TrackMeta};
use nz_cache::{Cache, QueueSnapshot};
use nz_model::{ListenContext, ListenPayload, Me, Quality as ModelQuality, Track};
use nz_mpris::{MprisCommand, MprisHandle};
use nz_player::{
    Command, Context as PlayerContext, Effect, Entry, EntryAlbum, EntryArtist, PlayerState, Repeat,
    Snapshot,
};
use tracing::{debug, warn};

use crate::nav::{Nav, Route, SettingsSection};
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
    /// MPRIS `Quit`.
    Quit,
    /// The single-instance name was claimed, refused, or the bus was missing.
    Instance(crate::wiring::instance::Claim),
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
    pub viz: Option<VizService>,
    pub side_panel: Option<SidePanel>,
    pub quality: PlaybackQuality,
    pub reduced_motion: bool,
    pub zoom: f32,
    pub pending_load: Option<PendingLoad>,
    pub pending_preload: Option<Option<Entry>>,
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
            viz: None,
            side_panel: None,
            quality,
            reduced_motion,
            zoom: 1.0,
            pending_load: None,
            pending_preload: None,
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
        let effects = self.player.apply(command, self.now_ms());
        self.apply_effects(effects);
        self.sync_transport();
        self.sync_mpris();
        cx.notify();
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
        // session is normalised like the next one (`nz-audio`, D6).
        audio.engine.set_track_meta(
            &reference,
            TrackMeta {
                loudness: entry.gain.map(|gain| gain as f32),
                edges: None,
                duration_s: Some(entry.duration as f64),
                podcast: entry.is_podcast(),
            },
        );
        let quality = self.quality.audio();
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
        self.update_bulk_cap();
    }

    fn preload(&mut self, next: Option<Entry>) {
        let Some(audio) = self.audio.clone() else {
            self.pending_preload = Some(next);
            return;
        };
        match next.and_then(|entry| entry.reference().map(str::to_owned)) {
            Some(reference) => audio.engine.preload_next(&reference, self.quality.audio()),
            None => {
                // Nothing after the playing row: the second voice starts empty.
                // `nz-audio` has no explicit "clear" (a preload is replaced by
                // the next one), so a silent id is not invented here.
            }
        }
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
                self.sync_mpris();
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
                self.auth = Auth::LoggedIn { me };
                self.session_cause = None;
                self.nav.reset(Route::Home);
                self.start_audio();
                self.warm_owner();
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
                self.login.error = None;
                self.session_cause = None;
                self.auth = Auth::LoggedIn { me };
                self.nav.reset(Route::Home);
                self.start_audio();
                self.warm_owner();
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
                position_s,
                duration_s,
                ..
            } => {
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
        if let Some(audio) = &self.audio
            && self.player.playing()
        {
            let position = audio.position();
            if position.is_finite() && (position - self.player.current_time()).abs() > 0.05 {
                self.player.set_position(position);
                self.sync_mpris();
                changed = true;
            }
        }
        if self.toasts.prune(Instant::now()) {
            changed = true;
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

    #[test]
    fn the_servers_quality_and_the_engines_agree_for_the_picker() {
        assert_eq!(model_quality(PlaybackQuality::Flac), ModelQuality::Flac);
        assert_eq!(model_quality(PlaybackQuality::Auto), ModelQuality::Opus320);
        assert_eq!(ModelQuality::Opus320.wire(), Some("OPUS_320"));
    }
}
