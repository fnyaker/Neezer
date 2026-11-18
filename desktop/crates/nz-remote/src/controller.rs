//! Remote control, the CONTROLLING side: this app drives somebody else's
//! player.
//!
//! Lot D15a (`docs/agents/desktop.md`), extracted for the phone by SC5
//! (`docs/agents/mobile.md`); the reference is
//! `webapp/src/lib/remote/controller.js`, the server contract
//! `neezer/webui/remote.py`, the notes
//! [party-remote.md](../../../../docs/agents/party-remote.md).
//!
//! Four rules shape this module:
//!
//! - **A tap answers at once, then the network.** A control applies its command
//!   to the local player state immediately (the SPA's `rebindPlayer` + `pending`
//!   pair) and posts it; the owner's own state replaces the local view only once
//!   the owner's player has RUN it (`ack`), so a tap never flickers back for the
//!   half second it takes to land. [`ACK_WAIT`] bounds the optimism, and a failed
//!   post drops it at once.
//! - **The position runs on the owner's own line.** The controlled player
//!   publishes a position at a SERVER instant (`{p, t}`); the controller maps its
//!   clock to the server's (`skew`, the round trip's midpoint — the SPA's own
//!   rule) and extrapolates between two reads, so the playhead moves as smoothly
//!   as it does over local audio.
//! - **It resumes on its own.** A dropped link, a server restart, the owner's
//!   player going away for a minute: the loop keeps asking, less and less often up
//!   to [`RETRY_MAX`], and reads at once after a tap. Commands are NEVER queued
//!   across an outage — a pause pressed a minute ago is not what anybody wants
//!   when the link returns — so a tap while offline says so (a transport error
//!   empties the outbox, [`Controller::flush`]).
//! - **Rule 1**: a controller has no engine and no playback of its own, so its
//!   requests are [`Lane::User`], the lane that never waits. Its whole traffic is
//!   one small GET a second plus one POST per tap, and both stop when the grant
//!   ends.
//!
//! A loan keeps NOTHING of the owner's here (`mode.js`): no queue is stored, no
//! listen is reported, no sleeve is cached (the caller turns caching off), and
//! the effects the state machine produces are dropped — [`adopt_effects`] is the
//! one place that says so.
//!
//! Nothing here names GPUI or an app state: the caller hands its own
//! `nz_player::PlayerState`, its clock and its store, and gets back what changed
//! so its screen can repaint.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nz_api::{Api, ApiError, Lane};
use nz_model::{Me, RemoteGrant, RemoteLevel};
use nz_player::{Command, Context as PlayerContext, Effect, Entry, PlayerState, Repeat, Snapshot};
use serde_json::Value;
use tracing::debug;

/// How often the controller reads the owner's state (`controller.js#POLL_MS`).
pub const POLL: Duration = Duration::from_millis(1000);
/// …and right after a tap, so its effect shows promptly.
pub const POLL_FAST: Duration = Duration::from_millis(300);
/// The fast window lasts this long after a command.
pub const FAST_WINDOW: Duration = Duration::from_millis(3000);
/// How long a local, optimistic view is held while its command lands.
pub const ACK_WAIT: Duration = Duration::from_millis(2500);
/// The ceiling of the reconnect backoff (`controller.js#RETRY_MAX_MS`).
pub const RETRY_MAX: Duration = Duration::from_millis(8000);
/// The foreground tick: applies what the owner says and moves the playhead along
/// the published line between two reads. The SPA's own `TICK_MS` is 250 ms; the
/// shell already repaints at 4 Hz, so 120 ms keeps the bar and the status fresh
/// without a wake per frame.
pub const TICK: Duration = Duration::from_millis(120);

// --- the grant ---------------------------------------------------------------

/// What a loan allows, and whose player it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    pub level: RemoteLevel,
    pub owner: String,
    pub device: String,
    pub label: String,
}

impl Default for Grant {
    fn default() -> Self {
        Self {
            level: RemoteLevel::Queue,
            owner: String::new(),
            device: String::new(),
            label: String::new(),
        }
    }
}

impl Grant {
    /// The loan `/api/me` reports, if this session is one. `None` on one's own
    /// app — which is every ordinary login.
    pub fn from_me(me: &Me) -> Option<Self> {
        let grant: &RemoteGrant = me.remote.as_ref()?;
        Some(Self {
            level: RemoteLevel::parse(&grant.level).unwrap_or(RemoteLevel::Queue),
            owner: grant.owner.clone(),
            device: grant.device.clone(),
            label: grant.label.clone(),
        })
    }

    /// Whether this loan may send `op` with `args` — the SPA's `opLevel`, which
    /// only decides what the UI HIDES; `remote.py#POLICY` is the enforcement.
    pub fn allows(&self, op: &str, args: Option<&Value>) -> bool {
        let Some(need) = op_level(op, args) else {
            return false;
        };
        self.level >= need
    }

    /// `File d'attente`, `Lecture seule`, … (`mode.js#LEVEL_INFO`).
    pub fn level_label(&self) -> &'static str {
        crate::host::level_label(self.level.wire())
    }
}

/// The level a command needs (`remote.py#OPS` + `op_level`): `set` on a picture
/// is readable, every other setting needs `full`.
pub fn op_level(op: &str, args: Option<&Value>) -> Option<RemoteLevel> {
    let level = match op {
        "play" | "pause" | "toggle" | "next" | "prev" | "seek" | "jump" | "volume" | "mute" => {
            RemoteLevel::Queue
        }
        "shuffle" | "repeat" | "remove" | "move" | "clear" | "add" | "play_next" | "play_queue"
        | "shuffle_play" => RemoteLevel::Read,
        "set" => {
            let key = args
                .and_then(|args| args.get("key"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            if key.starts_with("viz.") {
                RemoteLevel::Read
            } else {
                RemoteLevel::Full
            }
        }
        _ => return None,
    };
    Some(level)
}

// --- what the UI reads -------------------------------------------------------

/// Where the loan stands, as the banner and the screen paint it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// The first read has not landed yet.
    Connecting,
    /// The owner's player answers.
    Live,
    /// A read failed; the loop is asking again.
    Reconnecting,
    /// The grant ended (cut, expired, or the owner logged out).
    Ended,
}

impl Phase {
    /// The one-line state the banner prints (`RemoteBanner.svelte`).
    pub fn label(self, online: bool) -> &'static str {
        match self {
            Phase::Connecting => "Connexion…",
            Phase::Live if online => "En direct",
            Phase::Live => "Lecteur hors ligne",
            Phase::Reconnecting => "Reconnexion…",
            Phase::Ended => "Terminé",
        }
    }
}

/// The controller's published state, cloned by the screen.
#[derive(Clone, Debug, Default)]
pub struct Status {
    phase: Option<Phase>,
    /// Whether the OWNED player is reachable right now.
    pub online: bool,
    pub grant: Option<Grant>,
    /// The cause, in French, when the loop stopped on a failure.
    pub error: Option<String>,
    /// Bumped on every change.
    pub version: u64,
}

impl Status {
    pub fn phase(&self) -> Phase {
        self.phase.unwrap_or(Phase::Connecting)
    }

    /// `Connexion…`, `En direct · Complet`, `Reconnexion…` — the banner's line.
    pub fn line(&self) -> String {
        let state = self.phase().label(self.online);
        match &self.grant {
            Some(grant) => format!("{state} · {}", grant.level_label()),
            None => state.to_owned(),
        }
    }
}

// --- the published line ------------------------------------------------------

/// The owner's position line: a position at a SERVER instant, extrapolated.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Anchor {
    /// Seconds into the track at `t`.
    pub p: f64,
    /// The server's clock, in milliseconds.
    pub t: i64,
    pub playing: bool,
    pub duration: f64,
}

impl Anchor {
    /// Where the track is now, given the server's clock (`now_server_ms`).
    pub fn position(&self, now_server_ms: i64) -> f64 {
        let elapsed = if self.playing {
            ((now_server_ms - self.t) as f64 / 1000.0).max(0.0)
        } else {
            0.0
        };
        let at = self.p + elapsed;
        if self.duration > 0.0 {
            at.clamp(0.0, self.duration)
        } else {
            at.max(0.0)
        }
    }
}

/// The server's clock minus this one, in milliseconds: `now - (sent + received)/2`
/// (the SPA's own estimate, good to well under the tenth of a second a seek bar
/// can show).
pub fn skew_from(server_now_ms: i64, sent_ms: i64, received_ms: i64) -> i64 {
    server_now_ms - (sent_ms + received_ms) / 2
}

/// What a read changes in the local view: everything but the position, which the
/// anchor drives. Rebuilding a 5 000-row queue only when this moved is the "same
/// result, cheaper" rule on a 1 Hz loop.
#[derive(Clone, Debug, Default, PartialEq)]
struct Shape {
    qv: String,
    index: i64,
    playing: bool,
    volume: u64,
    muted: bool,
    shuffle: bool,
    repeat: String,
    track: String,
}

/// One read of `/api/remote/state`, decoded.
#[derive(Clone, Debug, Default)]
pub struct Read {
    pub online: bool,
    /// The owner's player: a one-row queue holding the playing track (the server
    /// publishes the TRACK, not the queue), plus the transport. The rows
    /// themselves come from `/api/remote/queue`.
    pub state: Option<Snapshot>,
    pub anchor: Option<Anchor>,
    pub qv: String,
    pub sv: String,
    pub ack: u64,
    pub seq: u64,
    /// The server's clock when it answered.
    pub now: i64,
    pub settings: Option<Value>,
    pub level: Option<String>,
    pub label: Option<String>,
    pub device: Option<String>,
}

impl Read {
    /// Decode the answer, tolerantly (`nz-model`'s rule: a key this build does
    /// not know is ignored, a missing one is not a failure).
    pub fn parse(answer: &Value) -> Self {
        let mut read = Read {
            online: answer
                .get("online")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            qv: text(answer, "qv"),
            sv: text(answer, "sv"),
            ack: answer.get("ack").and_then(Value::as_u64).unwrap_or(0),
            seq: answer.get("seq").and_then(Value::as_u64).unwrap_or(0),
            now: answer.get("now").and_then(Value::as_i64).unwrap_or(0),
            settings: answer
                .get("settings")
                .cloned()
                .filter(|value| !value.is_null()),
            ..Read::default()
        };
        if let Some(remote) = answer.get("remote") {
            read.level = Some(text(remote, "level")).filter(|level| !level.is_empty());
            read.label = Some(text(remote, "label"));
            read.device = Some(text(remote, "device"));
        }
        let Some(state) = answer.get("state").filter(|state| !state.is_null()) else {
            return read;
        };
        let p = number(state.get("p"));
        let duration = number(state.get("duration"));
        let playing = state
            .get("playing")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let track: Option<Entry> = state
            .get("track")
            .filter(|track| !track.is_null())
            .and_then(|track| serde_json::from_value(track.clone()).ok());
        let index = state.get("index").and_then(Value::as_i64).unwrap_or(-1);
        let mut snapshot = Snapshot {
            queue: track.into_iter().collect(),
            index: if index < 0 { -1 } else { 0 },
            position: p,
            shuffle: state
                .get("shuffle")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            repeat: repeat_from(state.get("repeat")),
            volume: number(state.get("volume")),
            muted: state.get("muted").and_then(Value::as_bool).unwrap_or(false),
            context: state.get("context").and_then(context_from),
            ..Snapshot::default()
        };
        if snapshot.queue.is_empty() {
            snapshot.index = -1;
        }
        read.anchor = Some(Anchor {
            p,
            t: state.get("t").and_then(Value::as_i64).unwrap_or(read.now),
            playing,
            duration,
        });
        read.state = Some(snapshot);
        read
    }

    /// What this read would change, for the adopt-or-skip decision.
    fn shape(&self) -> Shape {
        let snapshot = self.state.clone().unwrap_or_default();
        Shape {
            qv: self.qv.clone(),
            index: snapshot.index,
            playing: self.anchor.map(|anchor| anchor.playing).unwrap_or(false),
            volume: (snapshot.volume * 1000.0).round() as u64,
            muted: snapshot.muted,
            shuffle: snapshot.shuffle,
            repeat: crate::host::repeat_wire(snapshot.repeat).to_owned(),
            track: snapshot
                .queue
                .first()
                .map(|entry| entry.key().to_owned())
                .unwrap_or_default(),
        }
    }
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn number(value: Option<&Value>) -> f64 {
    value
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
        .unwrap_or(0.0)
}

fn repeat_from(value: Option<&Value>) -> Repeat {
    match value.and_then(Value::as_str) {
        Some("all") => Repeat::All,
        Some("one") => Repeat::One,
        _ => Repeat::Off,
    }
}

fn context_from(value: &Value) -> Option<PlayerContext> {
    Some(PlayerContext {
        kind: value.get("kind").and_then(Value::as_str)?.to_owned(),
        id: value.get("id").and_then(Value::as_str).map(str::to_owned),
        title: value
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

/// The queue `/api/remote/queue` answers, as the rows the lists draw.
pub fn parse_queue(answer: &Value) -> Vec<Entry> {
    answer
        .get("queue")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| serde_json::from_value::<Entry>(row.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

// --- the process's service ---------------------------------------------------

#[derive(Default)]
struct Shared {
    api: Mutex<Option<Arc<Api>>>,
    runtime: Mutex<Option<tokio::runtime::Handle>>,
    status: Mutex<Status>,
    /// The last read, waiting for the app thread.
    read: Mutex<Option<Read>>,
    /// The owner's queue rows, once they arrived.
    queue: Mutex<Vec<Entry>>,
    /// The `qv` those rows belong to (a new one is read once).
    held_qv: Mutex<String>,
    /// The `sv` this device already holds, sent back so the server leaves its
    /// settings out of the answer.
    held_sv: Mutex<String>,
    /// The owner's position line.
    anchor: Mutex<Option<Anchor>>,
    /// The shape last adopted, so a read that changes nothing rebuilds nothing.
    shape: Mutex<Shape>,
    /// Commands the person made here, waiting for their POST.
    outbox: Mutex<VecDeque<Command>>,
    /// The last sequence the server gave one of this device's commands, and when
    /// (`0` = nothing in flight).
    pending_seq: Mutex<(u64, Option<Instant>)>,
    /// The server's clock minus this one.
    skew: AtomicI64,
    /// Ask the loop to stop (the leave, or a grant that ended).
    stop: Mutex<bool>,
    /// Read fast until then (a tap just happened).
    fast_until: Mutex<Option<Instant>>,
    /// One loop per process, whatever a re-login does.
    looping: AtomicBool,
    /// A claim arrived while that one loop was on its way out. Its exit starts
    /// the next loop: a `spawn` swallowed in that instant ran nothing at all.
    wanted: AtomicBool,
}

static SERVICE: OnceLock<Controller> = OnceLock::new();

/// The handle the screen holds. Cheap to clone; the state is behind the
/// process's `OnceLock`, like `nz-offline`'s service.
#[derive(Clone)]
pub struct Controller {
    shared: Arc<Shared>,
}

impl Controller {
    /// The process's controller, when one was started.
    pub fn get() -> Option<Controller> {
        SERVICE.get().cloned()
    }

    /// What the screen paints.
    /// Whether the read loop is running. `spawn` takes this guard and the loop
    /// releases it on every way out (`Looping`), so a stuck `true` means the
    /// controller is silently dead.
    pub fn looping(&self) -> bool {
        self.shared.looping.load(Ordering::Acquire)
    }

    pub fn status(&self) -> Status {
        lock(&self.shared.status).clone()
    }

    /// The owner's queue, as last read.
    pub fn queue(&self) -> Vec<Entry> {
        lock(&self.shared.queue).clone()
    }

    /// The version of the queue those rows belong to: the screen clones 5 000
    /// rows only when this moved (`host.js`'s `qv`).
    pub fn queue_version(&self) -> String {
        lock(&self.shared.held_qv).clone()
    }

    /// The grant, when this session is a loan.
    pub fn grant(&self) -> Option<Grant> {
        lock(&self.shared.status).grant.clone()
    }

    /// Whether `op` may be sent on this loan (the UI hides the rest).
    pub fn allows(&self, op: &str, args: Option<&Value>) -> bool {
        self.grant()
            .map(|grant| grant.allows(op, args))
            .unwrap_or(false)
    }

    /// The length of the owner's track, as its line published it.
    pub fn duration(&self) -> f64 {
        (*lock(&self.shared.anchor)).unwrap_or_default().duration
    }

    /// Where the owner's track is right now, on its published line.
    pub fn position(&self) -> f64 {
        let anchor = (*lock(&self.shared.anchor)).unwrap_or_default();
        anchor.position(server_now(self.shared.skew.load(Ordering::Relaxed)))
    }

    /// How many commands wait for their POST. A tap pressed while the link is
    /// down leaves this at `0` after the next [`flush`](Self::flush): nothing is
    /// queued across an outage.
    pub fn queued(&self) -> usize {
        lock(&self.shared.outbox).len()
    }

    /// Take one command and show its effect at once.
    ///
    /// The local application is the SPA's `rebindPlayer`: the button answers in
    /// the same frame, and the owner's own state replaces it once the owner's
    /// player has RUN it. A command this loan may not send is refused BEFORE it
    /// touches the local state, so a `queue`-level link cannot make this screen
    /// lie about the owner's player; the caller shows the returned sentence.
    pub fn submit(&self, player: &mut PlayerState, command: Command, now_ms: u64) -> Sent {
        let args = serde_json::to_value(&command)
            .ok()
            .and_then(|value| value.get("args").cloned());
        if !self.allows(command.op(), args.as_ref()) {
            let refusal = match self.grant().map(|grant| grant.level) {
                Some(RemoteLevel::Queue) => "Ce lien ne permet que la lecture et la file d'attente",
                _ => "Ce lien est en lecture seule",
            };
            return Sent::Refused(refusal);
        }
        let effects = player.apply(command.clone(), now_ms);
        adopt_effects(effects);
        // The local view was changed by us: re-anchor it from where the tap put
        // it, so the playhead runs from there instead of from the owner's line.
        if let Some(anchor) = lock(&self.shared.anchor).as_mut() {
            anchor.p = player.current_time();
            anchor.t = server_now(self.shared.skew.load(Ordering::Relaxed));
            anchor.playing = player.playing();
            anchor.duration = player.duration();
        }
        lock(&self.shared.outbox).push_back(command);
        *lock(&self.shared.pending_seq) = (u64::MAX, Some(Instant::now()));
        *lock(&self.shared.fast_until) = Some(Instant::now() + FAST_WINDOW);
        Sent::Sent
    }

    /// Ask for a fast read now (the screen's « Réessayer », a tap).
    pub fn poke(&self) {
        *lock(&self.shared.fast_until) = Some(Instant::now() + FAST_WINDOW);
    }

    /// Take what the owner said, and apply it to the local player: the app
    /// thread calls this, then [`follow_line`](Self::follow_line), then repaints
    /// if either changed something. The settings come back for the caller to
    /// apply, because the store and the player's own rung are its own.
    pub fn take_read(&self, player: &mut PlayerState, now_ms: u64) -> ReadOutcome {
        let mut outcome = ReadOutcome::default();
        let read = lock(&self.shared.read).take();
        if let Some(read) = read {
            // Hold the local view while our own tap is still on its way.
            let (pending_seq, at) = *lock(&self.shared.pending_seq);
            let holding = pending_seq > read.ack
                && at.is_some_and(|at| Instant::now().duration_since(at) < ACK_WAIT);
            if holding {
                debug!(ack = read.ack, "vue locale conservée : commande en vol");
            } else {
                *lock(&self.shared.pending_seq) = (0, None);
                let shape = read.shape();
                let stale = shape != *lock(&self.shared.shape);
                if stale && let Some(state_snapshot) = read.state.clone() {
                    *lock(&self.shared.shape) = shape;
                    let mut snapshot = state_snapshot;
                    let queue = lock(&self.shared.queue);
                    if !queue.is_empty() && read.qv == *lock(&self.shared.held_qv) {
                        // The rows came from `/remote/queue`: the playing row is
                        // one of them, named by the owner's own index.
                        let index = snapshot.index;
                        snapshot.queue = queue.clone();
                        snapshot.index = index.clamp(0, snapshot.queue.len() as i64 - 1);
                    }
                    let playing = read.anchor.map(|anchor| anchor.playing).unwrap_or(false);
                    adopt(player, snapshot, playing, now_ms);
                    outcome.adopted = true;
                }
            }
            outcome.settings = read.settings.clone();
        }
        outcome
    }

    /// Between two reads the playhead follows the published line, at the shell's
    /// own rate (`controller.js#tick`). `true` when it moved the player.
    pub fn follow_line(&self, player: &mut PlayerState) -> bool {
        if player.playing() {
            let anchor = (*lock(&self.shared.anchor)).unwrap_or_default();
            if anchor.playing {
                let at = anchor.position(server_now(self.shared.skew.load(Ordering::Relaxed)));
                if (at - player.current_time()).abs() > 0.05 {
                    player.set_position(at);
                    return true;
                }
            }
        }
        false
    }

    // --- the network side ----------------------------------------------------

    /// Wire this session's client, runtime and grant into the process's
    /// controller. Idempotent: a re-login reuses the one service.
    pub fn ensure(api: Arc<Api>, runtime: tokio::runtime::Handle, grant: Grant) -> Controller {
        let controller = SERVICE
            .get_or_init(|| Controller {
                shared: Arc::new(Shared::default()),
            })
            .clone();
        *lock(&controller.shared.api) = Some(api);
        *lock(&controller.shared.runtime) = Some(runtime);
        *lock(&controller.shared.stop) = false;
        {
            let mut status = lock(&controller.shared.status);
            if status.grant.as_ref() != Some(&grant) || status.phase() == Phase::Ended {
                status.grant = Some(grant);
                status.phase = Some(Phase::Connecting);
                status.version += 1;
            }
        }
        controller
    }

    /// Run the network loop, once per process whatever a re-login does.
    pub fn spawn(&self, api: Arc<Api>) {
        if self.shared.looping.swap(true, Ordering::AcqRel) {
            // A loop is on its way out: remember the claim, so its exit starts
            // the next one (see `Looping`).
            self.shared.wanted.store(true, Ordering::Release);
            return;
        }
        let Some(runtime) = lock(&self.shared.runtime).clone() else {
            self.shared.looping.store(false, Ordering::Release);
            return;
        };
        let task = self.clone();
        runtime.spawn(async move { task.run(api).await });
    }

    /// The whole traffic in one loop: send what the person asked, read what the
    /// owner is doing. Runs on `nz-net` until the grant ends.
    async fn run(&self, api: Arc<Api>) {
        {
            // The guard lives in a value, so EVERY way out of the traffic loop
            // releases it — the `return` a loan that ended on an error takes
            // used to leave it set, and then a re-claim's `spawn` was swallowed:
            // the screen said « Connecté » and nothing read until the app
            // restarted (SC5's find (a)).
            let _looping = Looping(&self.shared.looping);
            self.traffic(&api).await;
        }
        if self.shared.wanted.swap(false, Ordering::AcqRel)
            && let Some(api) = lock(&self.shared.api).clone()
        {
            // A claim arrived while the loop above was exiting, with the
            // session's CURRENT client (a re-login replaces it).
            self.spawn(api);
        }
    }

    /// The loop itself: send what the person asked, read what the owner is
    /// doing, until a leave or a grant that ended.
    async fn traffic(&self, api: &Api) {
        let mut failures = 0_u32;
        loop {
            if *lock(&self.shared.stop) {
                break;
            }
            self.flush(api).await;
            match self.read_once(api).await {
                ReadStep::Read => {
                    failures = 0;
                }
                ReadStep::Failed => {
                    failures += 1;
                    tokio::time::sleep(retry(failures)).await;
                    continue;
                }
                ReadStep::Ended(error) => {
                    self.end(error.cause_fr().to_owned());
                    return;
                }
            }
            let fast = lock(&self.shared.fast_until).is_some_and(|until| Instant::now() < until);
            tokio::time::sleep(if fast { POLL_FAST } else { POLL }).await;
        }
    }

    /// One read of `/api/remote/state`, applied to the shared view. Public so a
    /// test can drive the loop one step at a time, on its own events.
    pub async fn read_once(&self, api: &Api) -> ReadStep {
        let sv = lock(&self.shared.held_sv).clone();
        let path = format!("/remote/state?sv={}", crate::host::path_segment(&sv));
        let sent = now_ms();
        let answer = api.get::<Value>(Lane::User, &path).await;
        let received = now_ms();
        match answer {
            Ok(fresh) => {
                let mut read = Read::parse(&fresh.value);
                if read.now == 0 {
                    read.now = received;
                }
                self.shared
                    .skew
                    .store(skew_from(read.now, sent, received), Ordering::Relaxed);
                if !read.qv.is_empty() && *lock(&self.shared.held_qv) != read.qv {
                    // The queue moved: read it once per version, and only once
                    // (the SPA's `loadQueue`).
                    match api.get::<Value>(Lane::User, "/remote/queue").await {
                        Ok(queue) => {
                            *lock(&self.shared.queue) = parse_queue(&queue.value);
                            *lock(&self.shared.held_qv) = text(&queue.value, "qv");
                        }
                        Err(error) => debug!(%error, "file du lecteur piloté"),
                    }
                }
                if let Some(anchor) = read.anchor {
                    *lock(&self.shared.anchor) = Some(anchor);
                }
                if !read.sv.is_empty() {
                    *lock(&self.shared.held_sv) = read.sv.clone();
                }
                {
                    let mut status = lock(&self.shared.status);
                    let phase = if status.phase() == Phase::Ended {
                        Phase::Ended
                    } else {
                        Phase::Live
                    };
                    status.phase = Some(phase);
                    status.online = read.online;
                    status.error = None;
                    if let Some(grant) = status.grant.as_mut()
                        && let Some(level) = read.level.as_deref().and_then(RemoteLevel::parse)
                        && grant.level != level
                    {
                        grant.level = level;
                    }
                    status.version += 1;
                }
                *lock(&self.shared.read) = Some(read);
                ReadStep::Read
            }
            Err(error) => {
                if matches!(error, ApiError::Unauthorized | ApiError::RemoteEnded { .. }) {
                    return ReadStep::Ended(error);
                }
                debug!(%error, "lecture du lecteur piloté");
                {
                    let mut status = lock(&self.shared.status);
                    status.phase = Some(Phase::Reconnecting);
                    status.error = Some(error.cause_fr());
                    status.version += 1;
                }
                ReadStep::Failed
            }
        }
    }

    /// POST every command the person made, in order. A refusal is the server's
    /// and is reported, never retried (a mutation is never replayed) — and a
    /// command that cannot be sent is DROPPED, never carried over an outage.
    pub async fn flush(&self, api: &Api) {
        let commands: Vec<Command> = lock(&self.shared.outbox).drain(..).collect();
        for command in commands {
            let body = serde_json::to_value(&command).unwrap_or(Value::Null);
            match api
                .send::<Value, Value>(
                    Lane::User,
                    reqwest::Method::POST,
                    "/remote/cmd",
                    Some(&body),
                )
                .await
            {
                Ok(answer) => {
                    if let Some(seq) =
                        answer.and_then(|value| value.get("seq").and_then(Value::as_u64))
                    {
                        *lock(&self.shared.pending_seq) = (seq, Some(Instant::now()));
                    }
                }
                Err(error) => {
                    *lock(&self.shared.pending_seq) = (0, None);
                    {
                        let mut status = lock(&self.shared.status);
                        status.error = Some(error.cause_fr());
                        status.version += 1;
                    }
                    debug!(%error, op = command.op(), "commande de contrôle non envoyée");
                }
            }
        }
    }

    /// End the loop: the loan is over (a cut, an expiry, a logout).
    fn end(&self, cause: String) {
        {
            let mut status = lock(&self.shared.status);
            status.phase = Some(Phase::Ended);
            status.online = false;
            status.error = Some(cause);
            status.version += 1;
        }
        *lock(&self.shared.stop) = true;
        lock(&self.shared.outbox).clear();
    }

    /// A leave: nothing is sent anywhere afterwards.
    pub fn stop(&self) {
        self.end("Contrôle terminé".to_owned());
    }
}

/// What one [`Controller::submit`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sent {
    /// The local player changed and the command waits for its POST.
    Sent,
    /// The loan refuses this op: nothing changed, and the sentence is the
    /// screen's to show.
    Refused(&'static str),
}

/// What one [`Controller::take_read`] changed.
#[derive(Clone, Debug, Default)]
pub struct ReadOutcome {
    /// The owner's state replaced the local player.
    pub adopted: bool,
    /// The owner's settings, for the caller to apply (it holds the store and its
    /// player's own rung).
    pub settings: Option<Value>,
}

/// One step of the read loop, so a failure and a finished grant are told apart.
#[derive(Clone, Debug, PartialEq)]
pub enum ReadStep {
    /// The owner answered; the view is updated.
    Read,
    /// The read failed (offline, a restart, a 5xx): ask again after a backoff.
    Failed,
    /// The grant ended (a cut, an expiry, the owner logged out).
    Ended(ApiError),
}

/// Adopt the owner's state: the local player becomes a view of it, and its
/// effects are DROPPED — a controller sounds nothing, reports nothing and stores
/// nothing of the owner's (`mode.js`). [`adopt_effects`] is that rule.
pub fn adopt(player: &mut PlayerState, snapshot: Snapshot, playing: bool, now_ms: u64) {
    let (mut restored, _) = PlayerState::restore(snapshot);
    let effects = if playing {
        // `restore` comes back paused by contract ("audio never starts without a
        // click"); a controller's click happened on the other device.
        restored.apply(Command::play(), now_ms)
    } else {
        Vec::new()
    };
    *player = restored;
    adopt_effects(effects);
}

/// Carry out nothing: a controller's effects are dropped on purpose. Kept as one
/// named function so the two call sites cannot drift into "just this once".
fn adopt_effects(effects: Vec<Effect>) {
    if !effects.is_empty() {
        debug!(
            count = effects.len(),
            "effets ignorés : ce poste pilote un autre lecteur"
        );
    }
}

/// The server's clock from this one.
pub fn server_now(skew: i64) -> i64 {
    now_ms() + skew
}

/// Milliseconds since the epoch (the SPA's `Date.now()`).
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// The reconnect backoff, `controller.js`'s `500 · 2^failures`, capped.
pub fn retry(failures: u32) -> Duration {
    let factor = 1_u64 << failures.min(4);
    Duration::from_millis((500 * factor).min(RETRY_MAX.as_millis() as u64))
}

/// The token in a link the person pasted: `…/rc/<id>.<mac>` (or the bare token).
/// The server checks the mac; this only refuses to send it something that is not
/// even shaped like one.
pub fn token_from_input(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let tail = trimmed
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or(trimmed);
    let tail = tail.split(['?', '#']).next().unwrap_or(tail);
    let mut parts = tail.split('.');
    let id = parts.next()?;
    let mac = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let shaped = |text: &str| {
        text.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    };
    if (16..=24).contains(&id.len()) && shaped(id) && mac.len() == 24 && shaped(mac) {
        Some(format!("{id}.{mac}"))
    } else {
        None
    }
}

/// `POST /remote/claim` — this device becomes the controller. The caller reads
/// the session again afterwards (`/api/me` then reports the loan).
pub async fn claim(api: &Api, token: &str) -> Result<Value, ApiError> {
    api.remote_claim(token).await
}

/// Releases the "one loop per process" guard however the loop returns.
///
/// A plain `store(false)` at the end of the loop body was not enough: the arm
/// that ends a loan on an error (`ReadStep::Ended`) returns from the middle, so
/// the guard stayed set and every later claim in that process called `spawn`
/// only to be turned away.
struct Looping<'a>(&'a AtomicBool);

impl Drop for Looping<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{Session, page_id};
    use crate::testing::{Bench, runtime};
    use serde_json::json;

    fn grant(level: RemoteLevel) -> Grant {
        Grant {
            level,
            owner: "owner".to_owned(),
            device: "Salon".to_owned(),
            label: String::new(),
        }
    }

    #[test]
    fn the_owner_position_line_extrapolates_and_clamps() {
        let anchor = Anchor {
            p: 10.0,
            t: 1_000,
            playing: true,
            duration: 200.0,
        };
        assert_eq!(anchor.position(1_000), 10.0);
        assert_eq!(anchor.position(1_500), 10.5);
        assert_eq!(anchor.position(0), 10.0, "jamais en arrière");
        let paused = Anchor {
            playing: false,
            ..anchor
        };
        assert_eq!(
            paused.position(9_999),
            10.0,
            "en pause, la position ne bouge pas"
        );
        let short = Anchor {
            p: 199.5,
            t: 0,
            playing: true,
            duration: 200.0,
        };
        assert_eq!(short.position(5_000), 200.0, "bornée par la durée");
    }

    #[test]
    fn the_clock_skew_is_the_round_trips_midpoint() {
        // Sent at 1000 local, answered at 1300 local, the server says 1410: the
        // round trip's midpoint is 1150, so the server's clock is 260 ms ahead.
        assert_eq!(skew_from(1_410, 1_000, 1_300), 260);
    }

    #[test]
    fn a_read_decodes_the_owner_state_and_its_line() {
        let read = Read::parse(&json!({
            "remote": {"level":"read","label":"Léa","device":"Salon"},
            "online": true,
            "state": {
                "index": 2, "playing": true, "p": 12.25, "duration": 200.0,
                "volume": 0.6, "muted": false, "shuffle": true, "repeat": "all",
                "status": "playing", "t": 1_700_000_000_000_i64,
                "track": {"id":"abc","title":"Titre","duration":200},
                "context": {"kind":"album","id":"9","title":"Album"}
            },
            "qv": "p.1", "sv": "p.2", "ack": 3, "seq": 4,
            "now": 1_700_000_000_100_i64
        }));
        assert!(read.online);
        assert_eq!(read.level.as_deref(), Some("read"));
        assert_eq!(read.ack, 3);
        assert_eq!(read.now, 1_700_000_000_100);
        let snapshot = read.state.clone().expect("un état");
        assert_eq!(
            snapshot.queue.len(),
            1,
            "le serveur nomme la piste, pas la file"
        );
        assert_eq!(snapshot.queue[0].title, "Titre");
        assert!(snapshot.shuffle);
        assert_eq!(snapshot.repeat, Repeat::All);
        assert_eq!(snapshot.volume, 0.6);
        assert_eq!(
            snapshot.context.map(|context| context.kind),
            Some("album".to_owned())
        );
        let anchor = read.anchor.expect("une ligne");
        // 600 ms after the published instant, the track is 0.6 s further on.
        assert!((anchor.position(1_700_000_000_600) - 12.85).abs() < 1e-9);
        // The shape carries what a rebuild depends on, and not the position.
        let shape = read.shape();
        assert_eq!(shape.track, "abc");
        assert!(shape.playing);
        let mut moved = read.clone();
        moved.anchor = Some(Anchor { p: 99.0, ..anchor });
        assert_eq!(moved.shape(), shape, "la position ne reconstruit rien");
        let mut paused = read;
        paused.anchor = Some(Anchor {
            playing: false,
            ..anchor
        });
        assert_ne!(paused.shape(), shape, "une pause, si");
    }

    #[test]
    fn an_empty_channel_reads_as_no_queue() {
        let read = Read::parse(&json!({
            "remote": {"level":"queue"}, "online": false, "state": null,
            "qv": "", "sv": "", "ack": 0, "seq": 0, "now": 0
        }));
        assert!(!read.online);
        assert!(read.state.is_none());
        assert!(read.anchor.is_none());
    }

    #[test]
    fn a_level_hides_what_the_server_would_refuse() {
        let queue = grant(RemoteLevel::Queue);
        assert!(queue.allows("next", None));
        assert!(queue.allows("seek", None));
        assert!(!queue.allows("shuffle", None), "la file demande read");
        assert!(!queue.allows("add", None));
        assert!(!queue.allows("set", Some(&json!({"key":"player.quality"}))));

        let read = grant(RemoteLevel::Read);
        assert!(read.allows("add", None));
        assert!(read.allows("set", Some(&json!({"key":"viz.mode"}))));
        assert!(!read.allows("set", Some(&json!({"key":"player.quality"}))));

        let full = grant(RemoteLevel::Full);
        assert!(full.allows("set", Some(&json!({"key":"player.quality"}))));
        assert!(!full.allows("teleport", None), "inconnu = refusé");
    }

    #[test]
    fn a_pasted_link_yields_its_token_and_nothing_else() {
        // A SHAPE, not a key: 18-character id, 24-character mac, a dot (the
        // server's `_TOKEN_RE`). `gitleaks:allow` because a high-entropy
        // fixture is what the generic-api-key rule looks for, and this one is
        // generated by the test, never a capability.
        let token = format!("{}.{}", "test".repeat(4) + "ab", "mac".repeat(8)); // gitleaks:allow
        assert_eq!(token_from_input(&token), Some(token.clone()));
        assert_eq!(
            token_from_input(&format!("http://192.168.1.2:5000/rc/{token}")),
            Some(token.clone())
        );
        assert_eq!(
            token_from_input(&format!("  https://x/neezer/rc/{token}  ")),
            Some(token.clone())
        );
        // Hostile or malformed input is refused before it ever leaves the app.
        assert_eq!(token_from_input(""), None);
        assert_eq!(token_from_input("http://evil/rc/../api/me"), None);
        assert_eq!(token_from_input("trop.court"), None);
        assert_eq!(token_from_input(&format!("{token}EXTRA")), None);
    }

    #[test]
    fn the_backoff_stops_at_the_ceiling() {
        assert_eq!(retry(1), Duration::from_millis(1000));
        assert_eq!(retry(2), Duration::from_millis(2000));
        assert_eq!(retry(9), RETRY_MAX);
    }

    #[test]
    fn a_grant_comes_from_the_sessions_own_me() {
        let me: Me = serde_json::from_value(json!({
            "user": {"id":"u1","name":"friend","admin":false},
            "remote": {"level":"full","owner":"owner","device":"Salon","label":"Salon"}
        }))
        .expect("a loan");
        let grant = Grant::from_me(&me).expect("un prêt");
        assert_eq!(grant.level, RemoteLevel::Full);
        assert_eq!(grant.owner, "owner");
        assert_eq!(grant.level_label(), "Complet");

        let own: Me = serde_json::from_value(json!({
            "user": {"id":"u1","name":"friend","admin":false}
        }))
        .expect("an ordinary login");
        assert!(Grant::from_me(&own).is_none());
    }

    #[test]
    fn a_status_says_where_the_loan_stands() {
        let mut status = Status::default();
        assert_eq!(status.phase(), Phase::Connecting);
        assert_eq!(status.line(), "Connexion…");
        status.phase = Some(Phase::Live);
        status.online = true;
        status.grant = Some(grant(RemoteLevel::Read));
        assert_eq!(status.line(), "En direct · Lecture seule");
        status.online = false;
        assert_eq!(status.line(), "Lecteur hors ligne · Lecture seule");
        status.phase = Some(Phase::Reconnecting);
        assert_eq!(status.line(), "Reconnexion… · Lecture seule");
    }

    /// A controller of its own, NOT the process's service: a test must not race
    /// another test on the one `OnceLock`, and the loops are never spawned here.
    fn detached(level: RemoteLevel) -> Controller {
        let controller = Controller {
            shared: Arc::new(Shared::default()),
        };
        *lock(&controller.shared.status) = Status {
            phase: Some(Phase::Connecting),
            grant: Some(grant(level)),
            ..Status::default()
        };
        controller
    }

    /// **A loan that ends on an error releases the loop.** SC5's find (a): the
    /// `Ended` arm returns from the middle of the traffic loop, so a plain
    /// `store(false)` at its end never ran, the "one loop per process" guard
    /// stayed set, and every later claim's `spawn` was turned away — the screen
    /// showed « Connecté » and nothing read until the app restarted.
    ///
    /// A client that never claimed gets the server's 401, which is the same
    /// `Ended` path a cut, an expiry and a revoked token take.
    #[test]
    fn a_loan_that_ends_on_an_error_releases_the_loop_for_the_next_claim() {
        let bench = Bench::start();
        let runtime = runtime();
        let api = Arc::new(bench.client());
        let controller = detached(RemoteLevel::Full);
        assert!(!controller.looping(), "a fresh controller runs no loop");

        runtime.block_on(controller.run(Arc::clone(&api)));
        assert_eq!(controller.status().phase(), Phase::Ended);
        assert!(
            !controller.looping(),
            "the guard `spawn` takes must be released when the loan ends, or a \
             re-claim in this process never reads again"
        );

        // The next claim runs a loop of its own — the behaviour the stuck guard
        // took away — and that loop releases the guard when it ends too.
        *lock(&controller.shared.runtime) = Some(runtime.handle().clone());
        *lock(&controller.shared.api) = Some(Arc::clone(&api));
        *lock(&controller.shared.stop) = false;
        controller.spawn(Arc::clone(&api));
        assert!(
            controller.looping(),
            "a claim after the first loan starts a loop"
        );
        runtime.block_on(async { tokio::time::sleep(Duration::from_millis(200)).await });
        assert_eq!(
            controller.status().phase(),
            Phase::Ended,
            "the new loop read, and met the same verdict"
        );
        assert!(!controller.looping(), "and it released the guard in turn");
    }

    /// A loan's `queue` level cannot send a `full`-only op, and the refusal
    /// changes NOTHING locally: the screen must not lie about the owner's player.
    #[test]
    fn a_refused_command_leaves_the_local_player_alone() {
        let controller = detached(RemoteLevel::Queue);
        let mut player = PlayerState::new();
        assert_eq!(
            controller.submit(&mut player, Command::remove_at(0), 0),
            Sent::Refused("Ce lien ne permet que la lecture et la file d'attente")
        );
        assert_eq!(controller.queued(), 0, "rien n'est parti, rien n'attend");
        assert_eq!(player.len(), 0, "l'état local n'a pas menti");
    }

    /// **The lot's acceptance (SC5), the controller half**: a `pause` pressed
    /// while the link is down is NOT carried over the outage — nothing is queued
    /// for the moment the link returns (party-remote.md's rule).
    ///
    /// The outage is a port nothing listens on (a refused connection, the way
    /// airplane mode answers), and the link coming back is the bench's own
    /// server: after the failed POST, a second flush sends NOTHING and the
    /// owner's channel sees no command at all.
    #[test]
    fn a_pause_pressed_while_the_link_is_down_is_not_sent_later() {
        let bench = Bench::start();
        let runtime = runtime();
        runtime.block_on(async {
            let dead = {
                let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
                let port = listener.local_addr().expect("its address").port();
                drop(listener);
                format!("http://127.0.0.1:{port}")
            };
            let offline = nz_api::Api::builder(dead)
                .session(Arc::new(nz_api::SessionJar::in_memory()))
                .build()
                .expect("a client");

            let owner = bench.client();
            owner.login("owner", "Owner1").await.expect("a real login");
            let created = owner
                .remote_create(&serde_json::json!({
                    "level": "full",
                    "device": "deskdev1",
                    "device_name": "Linux · Neezer",
                    "label": "Le bench",
                    "ttl": 3600,
                }))
                .await
                .expect("a link");
            let live = bench.client();
            live.remote_claim(&created.link.token)
                .await
                .expect("the claim");

            // A controller of its own, with the grant the claim gives it: the
            // process's one service is not shared with another test.
            let controller = detached(RemoteLevel::Full);
            let mut local = PlayerState::new();
            local.apply(Command::play_queue(bench.queue(), 0, None), 0);

            // The tap: it answers at once, and waits for its POST.
            assert_eq!(
                controller.submit(&mut local, Command::pause(), 0),
                Sent::Sent
            );
            assert!(!local.playing(), "le tap répond quand même");
            assert_eq!(controller.queued(), 1);

            // The link is down: the POST fails, and the tap is DROPPED — not
            // parked until the link comes back.
            controller.flush(&offline).await;
            assert_eq!(controller.queued(), 0, "rien n'attend un retour du lien");
            assert!(
                controller.status().error.is_some(),
                "l'échec est dit (le bandeau a sa cause)"
            );

            // The link comes back and the loop runs again: the old pause must
            // not land now. The owner's poll is where a command would show.
            controller.flush(&live).await;
            assert_eq!(controller.queued(), 0);
            let mut session = Session::new("deskdev1", page_id());
            owner
                .get::<Value>(Lane::User, &session.poll_path())
                .await
                .expect("a poll");
            let poll = owner
                .get::<Value>(Lane::User, &session.poll_path())
                .await
                .expect("a poll")
                .value;
            let polled = session.absorb_poll(&poll);
            assert!(
                polled.commands.is_empty(),
                "la pause d'avant la coupure n'est pas rejouée : {:?}",
                polled.commands
            );
            assert_eq!(session.applied, 0, "aucune commande accusée");
        });
    }
}
