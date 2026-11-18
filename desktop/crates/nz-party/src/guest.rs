//! Écoute partagée — the GUEST side.
//!
//! The contract is `neezer/webui/party.py`, the reference behaviour is
//! `webapp/src/lib/party/` (`guest.js`, `engine.js`, `timeline.js`, `clock.js`),
//! and the notes are [party-remote.md](../../../../docs/agents/party-remote.md).
//! A guest has no account and no session: everything it touches is under
//! `/api/party/<id>/`, which only ever opens what the host is playing.
//!
//! Five rules shape this file, each one the SPA's, each one measured here:
//!
//! - **The host is the reference and the guest follows a LINE, never a stream.**
//!   The server publishes `{anchor: {t, p}}` — the media position `p` at server
//!   instant `t` — and everyone derives "where we should be" from it
//!   ([`crate::timeline::position_at`]). Continuous playback therefore survives a
//!   missed poll: nothing is extrapolated twice, nothing drifts with the poll
//!   cadence.
//! - **Audio as 6 s chunks, scheduled on the DEVICE'S OWN clock.** A chunk is
//!   `[k·6 s, (k+1)·6 s + 50 ms)` of the track, cut by the server, decoded here,
//!   and handed to [`nz_audio::PartyChannel`] with the exact OUTPUT-stream
//!   position it must be heard at. That is the one number that makes a room
//!   sound like one room, and it comes from the device's `playback` timestamp
//!   plus the NTP offset ([`crate::timeline::to_stream`]), never from a timer.
//! - **Corrections follow ONE rule** (`timeline.js#correctionFor`): ≤ 3 ms
//!   nothing, ≤ 30 ms at the next chunk seam, behind by more skip forward now,
//!   ahead by more fall silent and resume the same audio — never repeat what the
//!   room just heard. Past 2.5 s ahead it is not a stall but the host going BACK,
//!   and waiting it out was thirty seconds of silence after a rewind.
//! - **Rule 1: the sync traffic never costs the music a frame.** The clock
//!   probes, the state polls and the chunk fetches are all `Lane::User` — the
//!   lane that never waits — coalesced to one request in flight per use, the
//!   chunks fetched four ahead at most, and every call goes through `nz-api`'s
//!   gates rather than around them.
//! - **Rule 3: a flaky link fails fast and says so.** A poll that fails keeps
//!   the timeline it has (the music carries on, extrapolated), a chunk that is
//!   not on disk yet is retried at the delay the server asked for, and after six
//!   seconds without an answer the view says « Connexion perdue — la musique
//!   continue » instead of pretending.
//!
//! The estimators are [`appcore::sync`]'s — the same Rust the page runs
//! (`webapp/appcore/src/sync.rs`): `ClockEstimator` for the NTP offset and
//! `Bridge` for the map from that clock onto the device's output clock.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use appcore::sync::Bridge;
use nz_api::{Api, ApiError, Lane};
use nz_audio::Segment;
use nz_model::{PartyState, PartyTrack};
use serde_json::Value;
use tracing::{debug, info};

use crate::clock::{ServerClock, local_ms};
use crate::planner::{LOOKAHEAD, Planner, Situation};
use crate::timeline::{
    CHUNK_S, Correction, Timeline, chunk_index, last_chunk, norm_gain, position_at, server_time_at,
    timeline_of, track_id,
};

/// A guest asks where the party is about once a second…
pub const POLL_MS: u64 = 1_000;
/// …and four times faster near a predicted handover, or while the host loads a
/// track (`guest.js#POLL_FAST_MS`): that is where a poll of delay costs audio.
pub const POLL_FAST_MS: u64 = 350;
/// How close to the announced handover the fast cadence starts.
pub const HANDOVER_SOON_MS: f64 = 4_000.0;
/// No answer for this long: the view says the link is lost (`OFFLINE_AFTER`).
pub const OFFLINE_AFTER_MS: u64 = 6_000;
/// The scheduler's own beat: it re-reads the output clock and places what is
/// due. 100 ms is a sixth of a chunk and far below the correction window.
pub const TICK: Duration = Duration::from_millis(100);
/// How often the output-clock pair is read for the bridge: 50 ms, the SPA's
/// cadence, because a mapping that steps has to be SEEN fast (`engine.js`).
pub const SAMPLE: Duration = Duration::from_millis(50);
/// At most two chunk requests in flight (`engine.js#MAX_INFLIGHT`).
pub const MAX_INFLIGHT: usize = 2;
/// The store keys the SPA keeps for a listener (`guest.js`).
pub const NAME_KEY: &str = "party.name";
pub const VOLUME_KEY: &str = "party.volume";
pub const LATENCY_KEY: &str = "party.latency";

/// Where a guest stands, for the screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    Joining,
    Live,
    /// The host ended it (or the server forgot it).
    Ended,
    /// The listener left.
    Left,
}

/// What the screen paints: a clone, under a mutex, with a version so a repaint
/// happens once per change.
#[derive(Clone, Debug, Default)]
pub struct GuestView {
    pub phase: Phase,
    pub host: String,
    pub track: Option<PartyTrack>,
    pub next: Option<PartyTrack>,
    pub listeners: usize,
    pub live: bool,
    pub playing: bool,
    pub loading: bool,
    /// The party's own playhead, seconds.
    pub position_s: f64,
    pub duration_s: f64,
    pub status: Status,
    pub clock: Option<(f64, f64)>,
    pub offline: bool,
    pub volume: f64,
    pub trim_ms: f64,
    pub os_latency_ms: Option<i64>,
    pub error: Option<String>,
    pub version: u64,
}

/// The green/amber line under the playhead (`Party.svelte#statusLine`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Idle,
    Synced,
    Loading,
    Paused,
}

impl GuestView {
    /// Which line the screen prints, and the tone it prints it in.
    pub fn status_line(&self) -> (Tone, String) {
        if let Some(error) = &self.error {
            return (Tone::Error, error.clone());
        }
        if self.offline {
            return (
                Tone::Warn,
                "Connexion perdue — la musique continue".to_owned(),
            );
        }
        if !self.live {
            return (
                Tone::Warn,
                if self.host.is_empty() {
                    "L'hôte s'est absenté".to_owned()
                } else {
                    format!("{} s'est absenté", self.host)
                },
            );
        }
        if self.track.is_none() {
            return (
                Tone::Wait,
                if self.host.is_empty() {
                    "En attente de l'hôte".to_owned()
                } else {
                    format!("En attente de {}", self.host)
                },
            );
        }
        if !self.playing {
            return (
                Tone::Wait,
                if self.loading {
                    format!("Chargement chez {}…", self.host)
                } else {
                    format!("En pause chez {}", self.host)
                },
            );
        }
        match self.clock {
            None => (Tone::Wait, "Synchronisation de l'horloge…".to_owned()),
            Some((spread, _)) => {
                if self.status == Status::Loading {
                    (Tone::Wait, "Chargement…".to_owned())
                } else if self.status == Status::Synced {
                    (
                        Tone::Ok,
                        format!("Calé sur {} · ±{:.1} ms", self.host, spread.max(0.1)),
                    )
                } else {
                    (Tone::Wait, "Mise en place…".to_owned())
                }
            }
        }
    }
}

/// The colour of a status line, the screen's own vocabulary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Wait,
    Ok,
    Warn,
    Error,
}

struct Shared {
    view: Mutex<GuestView>,
    api: Mutex<Option<Arc<Api>>>,
    runtime: Mutex<Option<tokio::runtime::Handle>>,
    clock: Mutex<Option<Arc<ServerClock>>>,
    /// `/party/<pid>` and the party's id.
    base: Mutex<String>,
    pid: Mutex<String>,
    /// The listener's own seat, and the name the host sees.
    lid: Mutex<Option<String>>,
    name: Mutex<String>,
    timeline: Mutex<Option<Timeline>>,
    state: Mutex<Option<PartyState>>,
    /// When the last poll ANSWERED (the offline verdict counts from here).
    answered_at: Mutex<Option<Instant>>,
    /// The decoded chunks of the current track, by index.
    chunks: Mutex<HashMap<u64, Arc<[f32]>>>,
    /// Every buffer handed to the mixer lately: the callback drops its own
    /// copy, and this keeps the allocation alive until the guest's thread lets
    /// it go — the callback never frees (see `nz-audio`'s `party.rs`).
    keep: Mutex<std::collections::VecDeque<Arc<[f32]>>>,
    /// The track the cache belongs to: a new track starts empty.
    chunks_of: Mutex<String>,
    /// A chunk the server said it is preparing, and when to ask again.
    retry_at: Mutex<HashMap<u64, Instant>>,
    inflight: AtomicUsize,
    /// The output clock's own rate, and the party's volume and trim.
    rate: AtomicU64,
    volume: Mutex<f64>,
    trim_ms: Mutex<f64>,
    engine: Mutex<Option<nz_audio::PartyChannel>>,
    /// The scheduler thread: OWNED and joined by [`Guest::stop`], like every
    /// other thread this app starts.
    scheduler: Mutex<Option<std::thread::JoinHandle<()>>>,
    stop: Arc<AtomicBool>,
    version: AtomicU64,
}

static SERVICE: std::sync::OnceLock<Guest> = std::sync::OnceLock::new();

/// The guest session the process is running, if one was started. Cheap to clone.
#[derive(Clone)]
pub struct Guest {
    shared: Arc<Shared>,
}

impl Guest {
    pub fn get() -> Option<Guest> {
        SERVICE.get().cloned()
    }

    /// The party this device is listening to, when it is listening to one.
    pub fn pid(&self) -> String {
        self.shared.pid.lock().expect("party poisoned").clone()
    }

    pub fn view(&self) -> GuestView {
        self.shared.view.lock().expect("party poisoned").clone()
    }

    /// Where the room is RIGHT NOW, seconds into the track: the line the room
    /// follows, read against this device's own clock estimate. This is the
    /// playhead the screen paints, and it moves between two polls — the poll is
    /// only what re-anchors the line.
    pub fn position_now(&self) -> f64 {
        let timeline = *self.shared.timeline.lock().expect("party poisoned");
        let Some(timeline) = timeline else {
            return 0.0;
        };
        let offset = self
            .shared
            .clock
            .lock()
            .expect("party poisoned")
            .as_ref()
            .and_then(|clock| clock.offset_ms());
        let now = offset.map(|offset| local_ms() + offset);
        now.map(|now| position_at(timeline, now).max(0.0))
            .unwrap_or(timeline.p)
    }

    /// The clock probes' own count, for the panel and the tests.
    pub fn probes(&self) -> u64 {
        self.shared
            .clock
            .lock()
            .expect("party poisoned")
            .as_ref()
            .map(|clock| clock.probes())
            .unwrap_or(0)
    }

    /// `GET /api/party/<pid>` without joining: the landing card's peek. `None`
    /// when the party does not exist (ended, or never did) — not an error.
    pub async fn peek(api: &Api, pid: &str) -> Result<Option<PartyState>, ApiError> {
        match api.party_state(pid, None, None, None).await {
            Ok(state) => Ok(Some(state)),
            Err(ApiError::Http { status: 404, .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// `POST /api/party/<pid>/join` — take a seat, unless this device already
    /// holds one. Returns the listener id.
    pub async fn join(api: &Api, pid: &str, name: &str) -> Result<String, ApiError> {
        let body = serde_json::json!({ "name": name });
        let answer: Option<Value> = api
            .send(
                Lane::User,
                reqwest::Method::POST,
                &format!("/party/{}/join", path_segment(pid)),
                Some(&body),
            )
            .await?;
        Ok(answer
            .and_then(|value| value.get("lid").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_default())
    }

    /// Start (or restart) the session: the clock probes, the poll loop and the
    /// scheduler thread. `engine` is `None` on a machine with no output, which is
    /// the "no sound, but say why" case instead of a panic.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        api: Arc<Api>,
        runtime: tokio::runtime::Handle,
        pid: String,
        name: String,
        lid: Option<String>,
        engine: Option<nz_audio::PartyChannel>,
        rate: u32,
        volume: f64,
        trim_ms: f64,
    ) -> Guest {
        let service = SERVICE.get_or_init(|| Guest {
            shared: Arc::new(Shared {
                view: Mutex::new(GuestView::default()),
                api: Mutex::new(None),
                runtime: Mutex::new(None),
                clock: Mutex::new(None),
                base: Mutex::new(String::new()),
                pid: Mutex::new(String::new()),
                lid: Mutex::new(None),
                name: Mutex::new(String::new()),
                timeline: Mutex::new(None),
                state: Mutex::new(None),
                answered_at: Mutex::new(None),
                chunks: Mutex::new(HashMap::new()),
                keep: Mutex::new(std::collections::VecDeque::new()),
                chunks_of: Mutex::new(String::new()),
                retry_at: Mutex::new(HashMap::new()),
                inflight: AtomicUsize::new(0),
                rate: AtomicU64::new(48_000),
                volume: Mutex::new(1.0),
                trim_ms: Mutex::new(0.0),
                engine: Mutex::new(None),
                scheduler: Mutex::new(None),
                stop: Arc::new(AtomicBool::new(false)),
                version: AtomicU64::new(1),
            }),
        });
        // A second join (another party, or the same one after leaving): the old
        // loops stop first, so two never publish into one device.
        service.stop_threads();
        let base = format!("/party/{}", path_segment(&pid));
        *service.shared.api.lock().expect("party poisoned") = Some(Arc::clone(&api));
        *service.shared.runtime.lock().expect("party poisoned") = Some(runtime.clone());
        *service.shared.base.lock().expect("party poisoned") = base.clone();
        *service.shared.pid.lock().expect("party poisoned") = pid.clone();
        *service.shared.name.lock().expect("party poisoned") = name.clone();
        *service.shared.lid.lock().expect("party poisoned") = lid.clone();
        *service.shared.engine.lock().expect("party poisoned") = engine;
        *service.shared.volume.lock().expect("party poisoned") = volume.clamp(0.0, 1.0);
        *service.shared.trim_ms.lock().expect("party poisoned") = trim_ms;
        service
            .shared
            .rate
            .store(u64::from(rate.max(1)), Ordering::Release);
        service.shared.inflight.store(0, Ordering::Release);
        service
            .shared
            .chunks
            .lock()
            .expect("party poisoned")
            .clear();
        *service.shared.chunks_of.lock().expect("party poisoned") = String::new();
        service
            .shared
            .retry_at
            .lock()
            .expect("party poisoned")
            .clear();
        *service.shared.timeline.lock().expect("party poisoned") = None;
        *service.shared.state.lock().expect("party poisoned") = None;
        *service.shared.answered_at.lock().expect("party poisoned") = None;
        service.shared.stop.store(false, Ordering::Release);
        {
            let mut view = service.shared.view.lock().expect("party poisoned");
            *view = GuestView {
                phase: Phase::Joining,
                volume: volume.clamp(0.0, 1.0),
                trim_ms,
                ..GuestView::default()
            };
        }
        if let Some(engine) = service
            .shared
            .engine
            .lock()
            .expect("party poisoned")
            .clone()
        {
            engine.take_over(true);
            engine.set_gain((volume * volume).clamp(0.0, 1.0) as f32);
        }
        let clock = ServerClock::open(Arc::clone(&api), runtime.clone(), base.clone());
        if let Some(old) = service
            .shared
            .clock
            .lock()
            .expect("party poisoned")
            .replace(clock)
        {
            old.stop();
        }
        // The clock probes and the poll loop are tasks (the runtime owns them);
        // the scheduler is a thread, because it reads the output clock every
        // 100 ms and owns the bridge's mutable state.
        {
            let this = service.clone();
            let poll_runtime = runtime.clone();
            runtime.spawn(async move { this.poll_loop(api, poll_runtime).await });
        }
        let this = service.clone();
        let handle = std::thread::Builder::new()
            .name("nz-party".into())
            .stack_size(256 * 1024)
            .spawn(move || this.scheduler_loop());
        if let Ok(handle) = handle {
            *service.shared.scheduler.lock().expect("party poisoned") = Some(handle);
        }
        info!(pid = %service.pid(), "écoute partagée : invité");
        service.clone()
    }

    /// Stop listening: tell the host, cut the sound, and join the scheduler.
    pub fn leave(&self) {
        let api = self.shared.api.lock().expect("party poisoned").clone();
        let runtime = self.shared.runtime.lock().expect("party poisoned").clone();
        let lid = self.shared.lid.lock().expect("party poisoned").clone();
        let pid = self.pid();
        if let (Some(api), Some(runtime), Some(lid), false) = (api, runtime, lid, pid.is_empty()) {
            let base = self.shared.base.lock().expect("party poisoned").clone();
            runtime.spawn(async move {
                let body = serde_json::json!({ "lid": lid });
                let _ = api
                    .send::<Value, Value>(
                        Lane::User,
                        reqwest::Method::POST,
                        &format!("{base}/leave"),
                        Some(&body),
                    )
                    .await;
            });
        }
        self.stop_threads();
        self.set_phase(Phase::Left);
    }

    /// Give the output back and stop every loop, without a request (the host
    /// ended the party, or the app is closing).
    pub fn stop(&self) {
        self.stop_threads();
        self.set_phase(Phase::Ended);
    }

    pub fn set_volume(&self, volume: f64) {
        let volume = volume.clamp(0.0, 1.0);
        *self.shared.volume.lock().expect("party poisoned") = volume;
        if let Some(engine) = self.shared.engine.lock().expect("party poisoned").clone() {
            engine.set_gain((volume * volume) as f32);
        }
        self.touch(|view| view.volume = volume);
    }

    /// This device's own output delay (a Bluetooth speaker that does not report
    /// it): positive plays earlier, so the room lines up.
    pub fn set_trim(&self, ms: f64) {
        let ms = ms.clamp(-500.0, 1000.0).round();
        *self.shared.trim_ms.lock().expect("party poisoned") = ms;
        self.touch(|view| view.trim_ms = ms);
    }

    pub fn trim(&self) -> f64 {
        *self.shared.trim_ms.lock().expect("party poisoned")
    }

    pub fn volume(&self) -> f64 {
        *self.shared.volume.lock().expect("party poisoned")
    }

    fn stop_threads(&self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(clock) = self
            .shared
            .clock
            .lock()
            .expect("party poisoned")
            .as_ref()
            .cloned()
        {
            clock.stop();
        }
        if let Some(engine) = self.shared.engine.lock().expect("party poisoned").clone() {
            engine.close();
        }
        if let Some(handle) = self.shared.scheduler.lock().expect("party poisoned").take() {
            let _ = handle.join();
        }
    }

    fn set_phase(&self, phase: Phase) {
        self.touch(|view| {
            view.phase = phase;
            view.status = Status::Idle;
        });
    }

    /// Mutate the view and bump its version, so the screen repaints once.
    fn touch(&self, change: impl FnOnce(&mut GuestView)) {
        let mut view = self.shared.view.lock().expect("party poisoned");
        change(&mut view);
        self.shared.version.fetch_add(1, Ordering::AcqRel);
        view.version = self.shared.version.load(Ordering::Acquire);
    }

    fn stopped(&self) -> bool {
        self.shared.stop.load(Ordering::Acquire)
    }

    // --- the network side ----------------------------------------------------

    /// Ask where the party is (about once a second, four times faster near a
    /// handover), feed the planner, fetch the chunks that are coming.
    async fn poll_loop(&self, api: Arc<Api>, runtime: tokio::runtime::Handle) {
        let mut failures = 0_u32;
        while !self.stopped() {
            let started = Instant::now();
            let pid = self.pid();
            let lid = self.shared.lid.lock().expect("party poisoned").clone();
            let quality = self
                .shared
                .clock
                .lock()
                .expect("party poisoned")
                .as_ref()
                .and_then(|clock| clock.quality())
                .map(|(spread, _)| spread.max(0.0).round() as u32);
            let status = view_status(&self.shared.view);
            let mut next = POLL_MS;
            match api
                .party_state(&pid, lid.as_deref(), quality, Some(status))
                .await
            {
                Ok(state) => {
                    failures = 0;
                    *self.shared.answered_at.lock().expect("party poisoned") = Some(Instant::now());
                    // A seat the server dropped (a device that went quiet) is
                    // taken again instead of listening on unseen.
                    if state.me == Some(false) {
                        self.rejoin(&api).await;
                    }
                    self.adopt(&state);
                    if state.buf {
                        next = POLL_FAST_MS;
                    } else if let Some(handover) = handover_in_ms(&state, &self.shared)
                        && (-2_000.0..HANDOVER_SOON_MS).contains(&handover)
                    {
                        next = POLL_FAST_MS;
                    }
                    self.prefetch(&api, &runtime, &state);
                }
                Err(ApiError::Http { status: 404, .. }) => {
                    // Over (or never there): let the music stop as it would at
                    // home, and say which of the two it was.
                    self.stop_threads();
                    self.touch(|view| {
                        view.phase = Phase::Ended;
                        view.status = Status::Idle;
                    });
                    return;
                }
                Err(error) => {
                    failures += 1;
                    let offline = self
                        .shared
                        .answered_at
                        .lock()
                        .expect("party poisoned")
                        .is_none_or(|at| at.elapsed() >= Duration::from_millis(OFFLINE_AFTER_MS));
                    if offline {
                        self.touch(|view| view.offline = true);
                    }
                    debug!(%error, "interrogation de l'écoute partagée");
                    // Fail fast, then back off: a poll storm on a flaky link is
                    // rule 3's own prohibition, and the timeline in hand keeps
                    // the music going meanwhile.
                    tokio::time::sleep(backoff(failures)).await;
                    continue;
                }
            }
            let elapsed = started.elapsed();
            tokio::time::sleep(Duration::from_millis(next).saturating_sub(elapsed)).await;
        }
    }

    /// Take a seat again, after the server forgot this one.
    async fn rejoin(&self, api: &Arc<Api>) {
        let pid = self.pid();
        let name = self.shared.name.lock().expect("party poisoned").clone();
        if let Ok(lid) = Guest::join(api, &pid, &name).await
            && !lid.is_empty()
        {
            *self.shared.lid.lock().expect("party poisoned") = Some(lid);
        }
    }

    /// Adopt one poll's answer: the timeline for the planner, the rest for the
    /// screen.
    fn adopt(&self, state: &PartyState) {
        let timeline = timeline_of(state);
        *self.shared.timeline.lock().expect("party poisoned") = timeline;
        let track_changed = {
            let previous = self.shared.state.lock().expect("party poisoned").clone();
            previous.as_ref().and_then(track_id) != track_id(state)
        };
        if track_changed {
            // A new track: the chunks of the old one are not what anyone plays.
            self.shared.chunks.lock().expect("party poisoned").clear();
            self.shared.retry_at.lock().expect("party poisoned").clear();
            *self.shared.chunks_of.lock().expect("party poisoned") =
                track_id(state).unwrap_or_default().to_owned();
        }
        *self.shared.state.lock().expect("party poisoned") = Some(state.clone());
        let clock = self
            .shared
            .clock
            .lock()
            .expect("party poisoned")
            .as_ref()
            .and_then(|clock| clock.quality());
        let listeners = state.listeners.len();
        let position_s = match (timeline_of(state), clock) {
            (Some(timeline), _) => {
                let now = self
                    .shared
                    .clock
                    .lock()
                    .expect("party poisoned")
                    .as_ref()
                    .and_then(|clock| clock.offset_ms())
                    .map(|offset| local_ms() + offset)
                    .unwrap_or(state.now);
                position_at(timeline, now)
            }
            _ => 0.0,
        };
        let duration = state
            .track
            .as_ref()
            .map(|track| track.duration)
            .unwrap_or(0.0);
        self.touch(|view| {
            view.phase = if view.phase == Phase::Joining {
                Phase::Live
            } else {
                view.phase
            };
            view.host = state.host.clone();
            view.track = state.track.clone();
            view.next = state.next.as_ref().and_then(|next| next.track.clone());
            view.listeners = listeners;
            view.live = state.live;
            view.playing = state.playing;
            view.loading = state.buf;
            view.position_s =
                position_s.clamp(0.0, if duration > 0.0 { duration } else { f64::MAX });
            view.duration_s = duration;
            view.clock = clock;
            view.offline = false;
            view.error = None;
        });
    }

    /// Fetch the chunks the planner is about to need: four ahead, two in
    /// flight, and nothing the server said it is still cutting.
    fn prefetch(&self, api: &Arc<Api>, runtime: &tokio::runtime::Handle, state: &PartyState) {
        let Some(track) = state.track.as_ref() else {
            return;
        };
        let Some(timeline) = timeline_of(state) else {
            return;
        };
        let now = self
            .shared
            .clock
            .lock()
            .expect("party poisoned")
            .as_ref()
            .and_then(|clock| clock.offset_ms())
            .map(|offset| local_ms() + offset)
            .unwrap_or(state.now);
        let position = position_at(timeline, now);
        let first = chunk_index(position.max(0.0), CHUNK_S);
        let last = match last_chunk(track.duration, CHUNK_S) {
            Some(last) => last,
            None => first + LOOKAHEAD,
        };
        let mut started = 0;
        for chunk in first..=last.min(first + LOOKAHEAD) {
            if self.shared.inflight.load(Ordering::Acquire) + started >= MAX_INFLIGHT {
                break;
            }
            if self
                .shared
                .chunks
                .lock()
                .expect("party poisoned")
                .contains_key(&chunk)
            {
                continue;
            }
            let retry = self
                .shared
                .retry_at
                .lock()
                .expect("party poisoned")
                .get(&chunk)
                .copied();
            if retry.is_some_and(|at| at > Instant::now()) {
                continue;
            }
            let path = chunk_path(&self.pid(), &track.id, chunk);
            let api = Arc::clone(api);
            let this = self.clone();
            let rate = self.shared.rate.load(Ordering::Acquire) as u32;
            self.shared.inflight.fetch_add(1, Ordering::AcqRel);
            started += 1;
            runtime.spawn(async move {
                let answer = api.bytes(Lane::User, &path).await;
                let this2 = this.clone();
                let decoded = match answer {
                    Ok(bytes) => tokio::task::spawn_blocking(move || decode_chunk(&bytes, rate))
                        .await
                        .unwrap_or_else(|error| Err(format!("décodage interrompu : {error}"))),
                    Err(ApiError::Http { status: 404, .. }) => {
                        // Past the end, or no longer in the party: nothing will
                        // ever bring this chunk back.
                        Err("fin de la piste".to_owned())
                    }
                    Err(error) => Err(error.cause_fr()),
                };
                this2.finish_chunk(chunk, decoded);
            });
        }
    }

    fn finish_chunk(&self, chunk: u64, decoded: Result<Arc<[f32]>, String>) {
        self.shared.inflight.fetch_sub(1, Ordering::AcqRel);
        match decoded {
            Ok(pcm) if !pcm.is_empty() => {
                self.shared
                    .chunks
                    .lock()
                    .expect("party poisoned")
                    .insert(chunk, pcm);
            }
            Ok(_) => {}
            Err(cause) => {
                debug!(chunk, cause, "chunk d'écoute partagée indisponible");
                // Two seconds, the server's own `Retry-After` for a chunk it is
                // still cutting; the planner asks again when it is due.
                self.shared
                    .retry_at
                    .lock()
                    .expect("party poisoned")
                    .insert(chunk, Instant::now() + Duration::from_secs(2));
            }
        }
    }

    // --- the scheduler -------------------------------------------------------

    /// The output's own beat: read the device clock into the bridge, ask the
    /// planner, hand the chunks to the mixer. 100 ms, and it never touches GPUI.
    fn scheduler_loop(&self) {
        let mut bridge = Bridge::new();
        let rate = self.shared.rate.load(Ordering::Acquire) as u32;
        let mut planner = Planner::new(rate);
        let mut last_track = String::new();
        while !self.stopped() {
            std::thread::sleep(TICK);
            if self.stopped() {
                break;
            }
            let Some(engine) = self.shared.engine.lock().expect("party poisoned").clone() else {
                continue;
            };
            let clock = engine.output_clock();
            let local = local_ms();
            bridge.offer(clock.at_s - local / 1000.0, clock.at_s, local);
            let bridge_s = if bridge.ready() {
                let median = bridge.median();
                if median.is_finite() {
                    Some(median)
                } else {
                    None
                }
            } else {
                None
            };
            let (timeline, state) = {
                let timeline = *self.shared.timeline.lock().expect("party poisoned");
                let state = self.shared.state.lock().expect("party poisoned").clone();
                (timeline, state)
            };
            let track_id = state
                .as_ref()
                .and_then(track_id)
                .unwrap_or_default()
                .to_owned();
            if track_id != last_track {
                planner.forget();
                last_track = track_id;
            }
            let offset = self
                .shared
                .clock
                .lock()
                .expect("party poisoned")
                .as_ref()
                .and_then(|clock| clock.offset_ms());
            let trim = *self.shared.trim_ms.lock().expect("party poisoned");
            let duration = state
                .as_ref()
                .and_then(|state| state.track.as_ref())
                .map(|track| track.duration)
                .unwrap_or(0.0);
            // Where the DEVICE is now: the anchor, extrapolated by the bridge's
            // own median rather than the instantaneous reading (a stepped
            // mapping must not move the extrapolation).
            let stream_now = match bridge_s {
                Some(bridge_s) => clock.position_s + ((local / 1000.0 + bridge_s) - clock.at_s),
                None => clock.position_s,
            };
            let ready = |chunk: u64| {
                self.shared
                    .chunks
                    .lock()
                    .expect("party poisoned")
                    .contains_key(&chunk)
            };
            let answered = *self.shared.answered_at.lock().expect("party poisoned");
            let situation = Situation {
                timeline,
                offset_ms: offset,
                bridge_s,
                stream_now,
                rate,
                trim_ms: trim,
                since_poll_ms: answered
                    .map(|at| at.elapsed().as_millis() as u64)
                    .unwrap_or(0),
                duration,
                ready: &ready,
            };
            let plan = planner.plan(&situation);
            if plan.cut {
                engine.clear();
            } else if plan.clear_future {
                engine.clear_future();
            }
            let gain = state
                .as_ref()
                .map(|state| {
                    norm_gain(
                        state.track.as_ref().and_then(|track| track.gain),
                        &state.norm,
                    )
                })
                .unwrap_or(1.0);
            for placement in &plan.place {
                let pcm = self
                    .shared
                    .chunks
                    .lock()
                    .expect("party poisoned")
                    .get(&placement.chunk)
                    .cloned();
                let Some(pcm) = pcm else { continue };
                let skip = (placement.skip_frames as usize).min(pcm.len() / 2);
                let data: Arc<[f32]> = pcm[skip * 2..].to_vec().into();
                if data.is_empty() {
                    continue;
                }
                let mut segment = Segment::at(placement.from_s, f64::from(rate), Arc::clone(&data));
                segment.gain = gain;
                engine.schedule(segment);
                // The caller keeps the buffer alive: the callback's drop is a
                // refcount decrement, the free happens here (party-remote.md's
                // rule for `nz-audio`'s callback).
                self.shared
                    .keep
                    .lock()
                    .expect("party poisoned")
                    .push_back(data);
                while self.shared.keep.lock().expect("party poisoned").len() > 32 {
                    self.shared.keep.lock().expect("party poisoned").pop_front();
                }
            }
            let status = match plan.correction {
                _ if plan.wanted_s.is_none() => Status::Idle,
                _ if plan.heard_s.is_none() => Status::Loading,
                Some(Correction::Keep) => Status::Synced,
                Some(Correction::Seam) => Status::Synced,
                Some(_) => Status::Loading,
                None if !timeline.map(|tl| tl.playing).unwrap_or(false) => Status::Paused,
                None => Status::Loading,
            };
            let changed = {
                let mut view = self.shared.view.lock().expect("party poisoned");
                let changed =
                    view.status != status || view.os_latency_ms != Some(latency_ms(clock));
                view.status = status;
                view.os_latency_ms = Some(latency_ms(clock));
                changed
            };
            if changed {
                self.shared.version.fetch_add(1, Ordering::AcqRel);
            }
        }
    }
}

fn view_status(view: &Mutex<GuestView>) -> &'static str {
    match view.lock().expect("party poisoned").status {
        Status::Synced => "sync",
        Status::Loading => "load",
        Status::Paused => "pause",
        Status::Idle => "idle",
    }
}

fn latency_ms(clock: nz_audio::OutputClock) -> i64 {
    (clock.latency_s * 1000.0).round() as i64
}

/// Milliseconds until the announced handover, when the party announced one.
fn handover_in_ms(state: &PartyState, shared: &Shared) -> Option<f64> {
    let timeline = timeline_of(state)?;
    let next = state.next.as_ref()?;
    let clock = shared.clock.lock().expect("party poisoned").clone()?;
    let now = local_ms() + clock.offset_ms()?;
    let handover = server_time_at(timeline, next.at) + next.gap * 1000.0;
    Some(handover - now)
}

/// A poll's backoff: 1, 2, 4, 8 s, capped at 8 (`rule 3`: fail fast, no storm).
pub fn backoff(failures: u32) -> Duration {
    Duration::from_millis(1_000 * (1_u64 << failures.min(3)))
}

/// The route one chunk is fetched from, exactly as the SPA builds it
/// (`engine.js#fetchChunk`: `${base}/chunk/${id}/${k}?f=…&sr=…`) and as
/// `neezer/webui/party.py#party_chunk` routes it
/// (`/party/<pid>/chunk/<mid>/<int:k>`).
///
/// **The index is a PATH segment and the options are the QUERY.** The first
/// version appended `?f=opus&sr=48000` and then `/{chunk}`, i.e.
/// `/chunk/<mid>?f=opus&sr=48000/3`: no route matches that, and the first real
/// client (MB20's phone, against `webapp/test/party/server.py`) answered 404 to
/// EVERY chunk while the screen sat on « Chargement… ». The desktop never
/// noticed because its benches serve chunks from a fixture, not from the route.
pub fn chunk_path(pid: &str, track: &str, chunk: u64) -> String {
    format!(
        "/party/{}/chunk/{}/{}?f=opus&sr=48000",
        path_segment(pid),
        path_segment(track),
        chunk
    )
}

/// One path segment, percent-encoded the way the SPA's `encodeURIComponent`
/// does for the characters an id can carry.
fn path_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

// --- decoding a chunk ---------------------------------------------------------

/// Decode one Ogg Opus chunk the server cut into interleaved stereo f32 at
/// `rate` (the device's own), dropping the resampler's own delay so the chunk
/// keeps its length — the mixer places it by stream position, and a chunk one
/// millisecond short would open a hole at every seam.
///
/// Blocking, on the blocking pool: six seconds of Opus is a few milliseconds of
/// work, and it must never run on the UI thread or in the output callback.
pub fn decode_chunk(bytes: &[u8], rate: u32) -> Result<Arc<[f32]>, String> {
    use symphonia::core::codecs::audio::well_known::CODEC_ID_OPUS;
    use symphonia::core::codecs::audio::{AudioDecoderOptions, CODEC_ID_NULL_AUDIO};
    use symphonia::core::codecs::registry::CodecRegistry;
    use symphonia::core::formats::probe::{Hint, Probe};
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
    use symphonia::core::meta::MetadataOptions;

    struct Bytes<'a>(&'a [u8], usize);
    impl std::io::Read for Bytes<'_> {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let take = out.len().min(self.0.len() - self.1);
            out[..take].copy_from_slice(&self.0[self.1..self.1 + take]);
            self.1 += take;
            Ok(take)
        }
    }
    impl std::io::Seek for Bytes<'_> {
        fn seek(&mut self, from: std::io::SeekFrom) -> std::io::Result<u64> {
            let base = match from {
                std::io::SeekFrom::Start(0) => 0,
                std::io::SeekFrom::Current(0) => self.1,
                std::io::SeekFrom::End(0) => self.0.len(),
                _ => return Err(std::io::Error::other("chunk non déplaçable")),
            };
            self.1 = base;
            Ok(base as u64)
        }
    }
    impl symphonia::core::io::MediaSource for Bytes<'_> {
        fn is_seekable(&self) -> bool {
            false
        }
        fn byte_len(&self) -> Option<u64> {
            Some(self.0.len() as u64)
        }
    }

    let mut registry = CodecRegistry::new();
    symphonia::default::register_enabled_codecs(&mut registry);
    if registry.get_audio_decoder(CODEC_ID_OPUS).is_none() {
        registry.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
    }
    let mut probe = Probe::new();
    symphonia::default::register_enabled_formats(&mut probe);
    let stream = MediaSourceStream::new(
        Box::new(Bytes(bytes, 0)),
        MediaSourceStreamOptions::default(),
    );
    let format = probe
        .probe(
            &Hint::new(),
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|error| format!("conteneur illisible : {error}"))?;
    let track = format
        .default_track(TrackType::Audio)
        .or_else(|| format.first_track(TrackType::Audio))
        .ok_or_else(|| "aucune piste audio".to_owned())?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|params| params.audio())
        .ok_or_else(|| "codec inconnu".to_owned())?
        .clone();
    if params.codec == CODEC_ID_NULL_AUDIO {
        return Err("codec inconnu".to_owned());
    }
    let source_rate = params.sample_rate.unwrap_or(48_000);
    let mut decoder = registry
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|error| format!("décodeur indisponible : {error}"))?;
    let mut format = format;
    let mut pcm: Vec<f32> = Vec::with_capacity(source_rate as usize * 2 * 7);
    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            // The chunk ended on a packet boundary: 6.05 s of audio, cut by the
            // server, is not a broken stream.
            Ok(None) => break,
            Err(_) => break,
        };
        if packet.track_id != track_id {
            continue;
        }
        let audio = match decoder.decode(&packet) {
            Ok(audio) => audio,
            // One undecodable packet costs that packet, like the engine's own
            // reader (`decode.rs`).
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(symphonia::core::errors::Error::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(error) => return Err(format!("décodage impossible : {error}")),
        };
        let frames = audio.frames();
        if frames == 0 {
            continue;
        }
        let spec = audio.spec().clone();
        let channels = spec.channels().count();
        let mut block = vec![0.0f32; frames * channels];
        audio.copy_to_slice_interleaved::<f32, _>(&mut block);
        match channels {
            1 => pcm.extend(block.iter().flat_map(|sample| [*sample, *sample])),
            _ => {
                for frame in 0..frames {
                    pcm.push(block[frame * channels]);
                    pcm.push(block[frame * channels + 1]);
                }
            }
        }
    }
    // The device's rate: the party's chunks are 48 kHz, an ALSA card may be
    // 44.1. Resample the whole chunk at once — it is 6 s of audio, not a stream.
    let block: Arc<[f32]> = if source_rate == rate.max(1) || pcm.is_empty() {
        pcm.into()
    } else {
        resample(&pcm, source_rate, rate.max(1))?.into()
    };
    Ok(block)
}

/// 48 kHz chunk → a 44.1 kHz device, with rubato's FFT resampler; `process_all`
/// trims the resampler's startup delay itself, so the chunk keeps its length (a
/// chunk a few frames short would open a hole at every seam).
fn resample(interleaved: &[f32], from: u32, to: u32) -> Result<Vec<f32>, String> {
    use rubato::Resampler;
    if interleaved.is_empty() {
        return Ok(Vec::new());
    }
    let frames = interleaved.len() / 2;
    let input = rubato::audioadapter_buffers::direct::InterleavedSlice::new(interleaved, 2, frames)
        .map_err(|error| format!("rééchantillonnage : {error}"))?;
    let mut fft = rubato::Fft::<f32>::new(
        from as usize,
        to as usize,
        1024,
        2,
        rubato::FixedSync::Input,
    )
    .map_err(|error| format!("rééchantillonnage : {error}"))?;
    let output = fft
        .process_all(&input, frames, None)
        .map_err(|error| format!("rééchantillonnage : {error}"))?;
    Ok(output.take_data())
}
