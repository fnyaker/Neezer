//! Remote control, the CONTROLLED side: this device's player, lent.
//!
//! Lot D15a (`docs/agents/desktop.md`), extracted for the phone by SC5
//! (`docs/agents/mobile.md`). The server contract is
//! `neezer/webui/remote.py`, the reference behaviour is
//! `webapp/src/lib/remote/host.js`, and the notes are
//! [party-remote.md](../../../../docs/agents/party-remote.md). Three rules shape
//! this module:
//!
//! - **A command is a player-store method said over the wire**, and here it is
//!   literally one: `neezer/webui/remote.py#OPS` and
//!   `webapp/src/lib/remote/commands.js` are the same vocabulary as
//!   [`nz_player::Command`]'s serde tags, so a command is DESERIALIZED into the
//!   state machine's own type and handed to the caller, which applies it through
//!   the one transport path a click, a key and MPRIS already take. The owner's
//!   own hand is never taken: their taps work as they always did, and the last
//!   action wins, whoever made it.
//! - **Polling, never a held connection** (the listen party's rule, for the
//!   same reason: one worker, a handful of threads). One loop publishes what the
//!   player is doing when it CHANGES, one asks for commands every 700 ms while
//!   somebody is driving and every 1.5 s otherwise, and both stop the moment the
//!   server says no link is left. Nothing runs at all when nothing is lent.
//! - **Rule 1: it never competes with the person's own playback.** Every request
//!   here is [`Lane::User`] — the ONE lane that never waits and never holds the
//!   single `Prefetch` slot the next track's audio needs — and the payloads are
//!   debounced (80–150 ms, coalesced), the queue is sent only when it changed,
//!   and the loops are stopped (not slowed) when nothing is lent. A remote tap
//!   is somebody else's action; it must not cost the owner a single frame of
//!   their own music.
//!
//! Why the split into a network task and a foreground tick: applying a command
//! touches the player state and the engine, which may only happen on the app's
//! own thread (the desktop's GPUI thread, Android's main looper), while the HTTP
//! lives on `nz-net` (D7's rule, and D9 measured that a `tokio::time` deadline
//! polled from the GPUI executor panics). So the network task queues what
//! arrived, the owner of the player drains it ([`RemoteHost::drain_pending`]),
//! applies it and hands the state back to publish. The latency that costs is
//! measured in the tests: **command to applied + 120 ms tick + 700 ms poll
//! period** against the lot's 1.2 s.
//!
//! Nothing here names GPUI or an app state: the caller supplies the
//! [`Published`] view, the queue rows and the settings it holds, and applies
//! what a poll asked for. The same code therefore runs in the desktop shell
//! (`desktop/app/src/wiring/remote_host.rs`) and in the phone's core
//! (`mobile/rust/crates/nz-core`, MB19).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nz_api::{Api, ApiError, Lane};
use nz_cache::Cache;
use nz_model::{RemoteLevel, RemoteLink};
use nz_player::{Command, Entry, Repeat};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::{debug, info};

// --- the clock of the channel -----------------------------------------------

/// How often the controlled player asks for commands while somebody drives it
/// (`host.js#FAST_MS`).
pub const FAST_POLL: Duration = Duration::from_millis(700);
/// …and while links exist but nobody is on them. The first tap of a controller
/// that just opened its link waits at most this (`host.js#SLOW_MS`: measured
/// 2.8 s at 3 s, which read as a button that does nothing).
pub const SLOW_POLL: Duration = Duration::from_millis(1500);
/// Re-anchor the position even when nothing moved.
pub const HEARTBEAT: Duration = Duration::from_millis(15_000);
/// The foreground tick that applies what arrived and hands back the state. It
/// is the one cost of the split (see the module note) and only runs while a
/// link is live.
pub const TICK: Duration = Duration::from_millis(120);
/// What the same tick wakes at when nothing is lent: a feature nobody uses must
/// not repaint an idle app (D7's own criterion).
pub const IDLE_TICK: Duration = Duration::from_millis(2000);
/// A position this far from the published line is a seek, not progress.
pub const SEEK_JUMP_S: f64 = 1.5;
/// The backoff ceiling of the poll loop (`host.js#RETRY_MAX`).
pub const RETRY_MAX: Duration = Duration::from_millis(15_000);
/// Where this device's own id lives (`device.js`'s `remote.device`).
pub const DEVICE_KEY: &str = "remote.device";
/// Whether this device has ever been lent. A launch whose store says NO asks the
/// server nothing at all: rule 3's "nothing fans out" starts at boot, and a
/// desktop that never used remote control must not pay a request per launch.
/// A device that has lent once asks every launch, so a link still points at it.
pub const HOSTED_KEY: &str = "remote.hosted";
/// The server's own limits (`remote.py`): a device id is 8–40 of these, and a
/// body is capped after inflation.
pub const DEVICE_LEN: usize = 18;

// --- this device -------------------------------------------------------------

/// A device id the server accepts (`remote.py#_DEVICE_RE`): 18 characters of
/// `[A-Za-z0-9_-]`, kept in the store so a link outlives a restart, a reboot and
/// an app update — a link drives ONE device.
pub fn device_id(cache: &Cache) -> String {
    let kv = cache.kv();
    if let Some(id) = kv.get::<String>(DEVICE_KEY)
        && is_device_id(&id)
    {
        return id;
    }
    let id = fresh_device_id();
    let _ = kv.set(DEVICE_KEY, &id);
    id
}

/// Whether a string is one the server will take as a device id.
pub fn is_device_id(id: &str) -> bool {
    (8..=40).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// A new id, from the clock and the process: this needs to be unique, not
/// unguessable — the server is what authorises a controller.
fn fresh_device_id() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";
    let mut seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or(0)
        ^ ((std::process::id() as u64) << 32);
    let mut id = String::with_capacity(DEVICE_LEN);
    for _ in 0..DEVICE_LEN {
        // An xorshift: no dependency, and this id is not a secret.
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        id.push(ALPHABET[(seed % ALPHABET.len() as u64) as usize] as char);
    }
    id
}

/// What the owner's sheet and the controller's banner call this device. The
/// SPA's `deviceName()` shape, said for a desktop app.
pub fn device_name() -> String {
    let os = match std::env::consts::OS {
        "linux" => "Linux",
        "macos" => "macOS",
        "windows" => "Windows",
        other => other,
    };
    format!("{os} · Neezer")
}

// --- what the UI reads -------------------------------------------------------

/// One controller on one link, as the owner's chip and sheet name it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Controller {
    pub link: String,
    pub level: String,
    pub label: String,
}

impl Controller {
    /// `File d'attente`, `Lecture seule`, `Complet`, `Administrateur`.
    pub fn level_label(&self) -> &'static str {
        level_label(&self.level)
    }
}

/// The French name of a level, the SPA's `LEVEL_INFO` (mode.js).
pub fn level_label(level: &str) -> &'static str {
    match RemoteLevel::parse(level) {
        Some(RemoteLevel::Queue) => "File d'attente",
        Some(RemoteLevel::Read) => "Lecture seule",
        Some(RemoteLevel::Full) => "Complet",
        Some(RemoteLevel::Admin) => "Administrateur",
        None => "Inconnu",
    }
}

/// The one-line blurb the sheet prints under each level (`mode.js#LEVEL_INFO`).
pub fn level_blurb(level: &str) -> &'static str {
    match RemoteLevel::parse(level) {
        Some(RemoteLevel::Queue) => {
            "Lecture, pause, titre suivant, position, volume — et se promener dans la file."
        }
        Some(RemoteLevel::Read) => {
            "Toute l'app sans rien modifier : chercher, ajouter à la file, choisir les animations."
        }
        Some(RemoteLevel::Full) => {
            "Tout sauf l'administration : playlists, favoris, réglages du lecteur, étiquetage."
        }
        Some(RemoteLevel::Admin) => {
            "Tout, administration comprise. Réservé à quelqu'un de confiance."
        }
        None => "",
    }
}

/// What this device is answering, for the chip over the player bar and the
/// sheet: a clone, read under a mutex, with a version so a repaint is skipped
/// when nothing moved.
#[derive(Clone, Debug, Default)]
pub struct HostStatus {
    /// The loops are running (a link points here and the server still knows it).
    pub active: bool,
    /// Who is driving right now, as the server counts them.
    pub controllers: Vec<Controller>,
    /// The last cause, in French, when the loops stopped on a failure.
    pub error: Option<String>,
    /// The last command number this device ran.
    pub applied: u64,
    /// Bumped on every change, so a screen paints once per change.
    pub version: u64,
}

impl HostStatus {
    pub fn live(&self) -> usize {
        self.controllers.len()
    }

    /// `« Contrôlé par Léa »`, `« Contrôlé par 2 appareils »` — the chip's first
    /// line, the SPA's `RemoteHostChip` wording.
    pub fn who(&self) -> String {
        let n = self.controllers.len();
        match n {
            0 => String::new(),
            1 => {
                let label = self.controllers[0].label.trim();
                if label.is_empty() {
                    "Contrôlé par 1 appareil".to_owned()
                } else {
                    format!("Contrôlé par {label}")
                }
            }
            _ => {
                let named: Vec<&str> = self
                    .controllers
                    .iter()
                    .map(|controller| controller.label.trim())
                    .filter(|label| !label.is_empty())
                    .collect();
                if named.len() == n && n <= 2 {
                    format!("Contrôlé par {}", named.join(" et "))
                } else if let Some(first) = named.first() {
                    format!("Contrôlé par {first} et {} autres", n - 1)
                } else {
                    format!("Contrôlé par {n} appareils")
                }
            }
        }
    }

    /// The level line under the chip's first line, when there is exactly one.
    pub fn level(&self) -> Option<&'static str> {
        match self.controllers.as_slice() {
            [one] => Some(one.level_label()),
            _ => None,
        }
    }
}

// --- the wire session (pure) -------------------------------------------------

/// What one publish carries. Built on the app's own thread from the player
/// state; the network task turns it into the request body.
#[derive(Clone, Debug, Default)]
pub struct Published {
    pub index: i64,
    pub playing: bool,
    /// The position the person HEARS (the engine's own clock, `nz-audio`'s
    /// `Engine::position` already subtracts the device latency).
    pub p: f64,
    pub duration: f64,
    pub volume: f64,
    pub muted: bool,
    pub shuffle: bool,
    pub repeat: String,
    /// `playing`, `paused` or `idle` (`playbackStatus`).
    pub status: String,
    pub track: Option<Value>,
    pub context: Option<Value>,
}

/// A cheap fingerprint of the queue, so the rows are cloned and sent only when
/// they really changed: `nz-player` keeps its queue version private and its
/// `snapshot()` clones every row, which a 120 ms tick must not do to a 5 000-row
/// queue (rule 4). Pointer identity plus position catches a jump, a remove, a
/// move and a shuffle; `seq` catches a track change.
pub fn queue_fingerprint(seq: u64, queue: &[Arc<Entry>]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64 ^ seq;
    for entry in queue {
        let pointer = Arc::as_ptr(entry) as usize as u64;
        hash = (hash ^ pointer.rotate_left(17))
            .wrapping_mul(0x100_0000_01b3)
            .rotate_left(5);
    }
    hash ^ (queue.len() as u64)
}

/// The session state of one channel: the command cursor, and the two versions
/// the server compares before it asks for the queue and the settings again.
#[derive(Clone, Debug)]
pub struct Session {
    pub device: String,
    /// The channel this page last saw. A page id makes every launch "new" to the
    /// server, exactly like `host.js`'s `page`, so a restart republishes.
    pub chan: String,
    /// The last command number this device has SEEN (its poll cursor).
    pub since: u64,
    /// The last command number it has RUN (`ack`).
    pub applied: u64,
    page: String,
    qn: u64,
    sn: u64,
    /// The queue version the server holds (`qv`).
    pub qv: String,
    /// The settings version the server holds (`sv`).
    pub sv: String,
}

/// What one poll asks this device to do.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Polled {
    /// No link points here any more: stop, and say nothing.
    pub ended: bool,
    /// The server rebuilt the channel (a restart, an eviction): everything goes
    /// again, and the command numbers start over.
    pub reset: bool,
    /// Who is driving, right now.
    pub controllers: Vec<Controller>,
    /// What to run, in order.
    pub commands: Vec<Pending>,
    /// The channel holds no state yet: publish at once.
    pub needs_state: bool,
}

/// One thing a poll asked this device to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Pending {
    /// A transport/queue command: `nz_player::Command`, applied through the
    /// caller's one transport path (the desktop's `AppState::command`, the
    /// phone's player API).
    Command(Command),
    /// A mirrored SETTING (`set`): the same `kv` keys Réglages writes. It is not
    /// a player command, so it takes the store's own path.
    Setting { key: String, value: Value },
    /// An op this build does not know, or one whose arguments did not decode: the
    /// cursor moves past it and the log says so (rule 7: never a crash).
    Ignored(String),
}

impl Session {
    /// A session for `device`; `page` makes its versions unique per launch.
    pub fn new(device: impl Into<String>, page: impl Into<String>) -> Self {
        let page = page.into();
        Self {
            device: device.into(),
            chan: String::new(),
            since: 0,
            applied: 0,
            qv: format!("{page}.0"),
            sv: format!("{page}.0"),
            page,
            qn: 0,
            sn: 0,
        }
    }

    /// `GET /api/remote/host/<device>/poll?since=&chan=` — polling, never a held
    /// connection.
    pub fn poll_path(&self) -> String {
        format!(
            "/remote/host/{}/poll?since={}&chan={}",
            path_segment(&self.device),
            self.since,
            path_segment(&self.chan)
        )
    }

    /// `POST /api/remote/host/<device>` — what the player is doing, and what the
    /// channel still lacks.
    pub fn publish_path(&self) -> String {
        format!("/remote/host/{}", path_segment(&self.device))
    }

    /// The body of a publish: `state`, the queue and the settings only when they
    /// are due, plus `ack` (the last command this player RAN, which is what keeps
    /// a controller's tap from flickering back).
    pub fn publish_body(
        &self,
        published: &Published,
        queue: Option<&[Value]>,
        settings: Option<&Value>,
    ) -> Value {
        let mut state = json!({
            "index": published.index,
            "playing": published.playing,
            "p": published.p,
            "duration": published.duration,
            "volume": published.volume,
            "muted": published.muted,
            "shuffle": published.shuffle,
            "repeat": published.repeat,
            "status": published.status,
            "track": published.track,
            "context": published.context,
        });
        // The server stamps `t` (its own clock) and echoes `qv`; sending them
        // would be overwritten anyway.
        if let Some(map) = state.as_object_mut() {
            map.retain(|_, value| !value.is_null());
        }
        let mut body = json!({
            "chan": self.chan,
            "qv": self.qv,
            "sv": self.sv,
            "ack": self.applied,
            "state": state,
        });
        if let Some(queue) = queue {
            body["queue"] = Value::Array(queue.to_vec());
        }
        if let Some(settings) = settings {
            body["settings"] = settings.clone();
        }
        body
    }

    /// Read a poll's answer: the commands to run, who is driving, and whether
    /// this device has to republish everything.
    pub fn absorb_poll(&mut self, answer: &Value) -> Polled {
        let mut polled = Polled::default();
        if !answer
            .get("active")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            polled.ended = true;
            return polled;
        }
        let chan = answer
            .get("chan")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let reset = answer
            .get("reset")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || (!chan.is_empty() && chan != self.chan);
        if reset {
            // A channel this launch has not published to (the server restarted):
            // everything goes again and the numbers start over — including the
            // commands it already queued, which is why `since` resets too.
            self.chan = chan;
            self.since = 0;
            self.applied = 0;
            polled.reset = true;
        } else if !chan.is_empty() {
            self.chan = chan;
        }
        polled.needs_state = reset
            || !answer
                .get("has_state")
                .and_then(Value::as_bool)
                .unwrap_or(true);
        polled.controllers = parse_controllers(answer.get("controllers"));
        let mut highest = self.since;
        for raw in answer
            .get("cmds")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let seq = raw.get("seq").and_then(Value::as_u64).unwrap_or(0);
            if seq <= self.applied {
                // Already run (a replayed command after a reload): running it
                // twice would skip twice.
                highest = highest.max(seq);
                continue;
            }
            // `applied` is bumped HERE so the next publish acks it; the
            // foreground runs it and the ack rides the next publish.
            highest = highest.max(seq);
            self.applied = seq;
            polled.commands.push(decode_pending(raw));
        }
        self.since = highest;
        polled
    }

    /// Read a publish's answer. `true` when the channel lacks the queue or the
    /// settings and this device must send them again (a restart took them).
    pub fn absorb_publish(&mut self, answer: &Value) -> bool {
        if !answer
            .get("active")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return false;
        }
        if let Some(chan) = answer.get("chan").and_then(Value::as_str)
            && !chan.is_empty()
        {
            self.chan = chan.to_owned();
        }
        let need_queue = answer
            .get("need_queue")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let need_settings = answer
            .get("need_settings")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        need_queue || need_settings
    }

    /// A new queue version (`host.js`'s `qv`): `<page>.<n>`.
    pub fn bump_queue(&mut self) -> String {
        self.qn += 1;
        self.qv = format!("{}.{}", self.page, self.qn);
        self.qv.clone()
    }

    /// A new settings version.
    pub fn bump_settings(&mut self) -> String {
        self.sn += 1;
        self.sv = format!("{}.{}", self.page, self.sn);
        self.sv.clone()
    }
}

/// The controller list of a poll's answer.
pub fn parse_controllers(value: Option<&Value>) -> Vec<Controller> {
    value
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    Some(Controller {
                        link: row.get("link")?.as_str()?.to_owned(),
                        level: row
                            .get("level")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        label: row
                            .get("label")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// One wire command (`{"seq","op","args","link"}`) as the state machine's own
/// type. `nz_player::Command`'s serde tags ARE the server's op names, so this is
/// the whole decoder.
pub fn decode_command(raw: &Value) -> Option<Command> {
    let op = raw.get("op")?.as_str()?;
    let args = raw.get("args").cloned().unwrap_or_else(|| json!({}));
    let payload = json!({ "op": op, "args": args });
    serde_json::from_value::<Command>(payload).ok()
}

/// One wire command as the thing this device must do: a player command, a
/// mirrored setting, or an op this build ignores.
pub fn decode_pending(raw: &Value) -> Pending {
    let op = raw.get("op").and_then(Value::as_str).unwrap_or_default();
    let args = raw.get("args").cloned().unwrap_or_else(|| json!({}));
    if op == "set" {
        let key = args
            .get("key")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let value = args.get("value").cloned().unwrap_or(Value::Null);
        // The server already refuses a key the player never published; this is
        // the second half of that rule (`remote.py#_a_set`, 4 KB of JSON).
        let too_big = serde_json::to_string(&value)
            .map(|body| body.len() > 4096)
            .unwrap_or(true);
        if key.is_empty() || !mirrored(&key) || too_big {
            return Pending::Ignored(format!("set {key}"));
        }
        return Pending::Setting { key, value };
    }
    match decode_command(raw) {
        Some(command) => Pending::Command(command),
        None => Pending::Ignored(op.to_owned()),
    }
}

/// Whether this build mirrors `key` (`MIRRORED`, the SPA's own list).
fn mirrored(key: &str) -> bool {
    MIRRORED.contains(&key)
}

/// The fingerprint of a queue as a function of its rows only, for a test.
pub fn queue_values(entries: &[Arc<Entry>]) -> Option<Vec<Value>> {
    let rows: Vec<Value> = entries
        .iter()
        .map(|entry| serde_json::to_value(entry.as_ref()).ok())
        .collect::<Option<Vec<Value>>>()?;
    Some(rows)
}

/// A path segment: the server's own alphabet, and nothing that could steer a
/// request somewhere else (`remote.py#_DEVICE_RE` is the same rule). Public so
/// the controller encodes its query values with the same rule.
pub fn path_segment(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect()
}

// --- the process's service ---------------------------------------------------

/// One channel's live state, shared between the network task and the app thread.
#[derive(Default)]
struct Shared {
    api: Mutex<Option<Arc<Api>>>,
    /// The local store, for the two keys this service owns (`remote.device`,
    /// `remote.hosted`).
    cache: Mutex<Option<Cache>>,
    runtime: Mutex<Option<tokio::runtime::Handle>>,
    status: Mutex<HostStatus>,
    /// The last state the app handed over.
    view: Mutex<Option<Published>>,
    /// The queue rows to send, and the version they belong to.
    queue: Mutex<Push>,
    /// The settings to send.
    settings: Mutex<Push>,
    /// Commands that arrived on the network task and wait for the app thread.
    pending: Mutex<VecDeque<Pending>>,
    /// The wire session, owned by the network task.
    session: Mutex<Option<Session>>,
    /// Ask the network task to stop (the owner cut every link, or logged out).
    stop: Mutex<bool>,
    /// One poll loop per process, whatever a re-login does.
    looping: Mutex<bool>,
    /// A republish is due (a command ran, a setting changed, the heartbeat).
    publish_due: Mutex<bool>,
    /// The fingerprint of the queue last handed over.
    fingerprint: Mutex<u64>,
    /// When the state was last published (the heartbeat).
    published_at: Mutex<Option<std::time::Instant>>,
}

/// One payload waiting to be sent (`host.js`'s `queueDirty` / `settingsDirty`).
#[derive(Default, Clone)]
struct Push {
    version: String,
    body: Option<Value>,
    sent: String,
}

impl Push {
    fn pending(&self) -> bool {
        self.body.is_some() && self.sent != self.version
    }
}

static SERVICE: OnceLock<RemoteHost> = OnceLock::new();

/// The handle the chip, the sheet and the foreground tick hold. Cheap to clone;
/// the state is behind the process's `OnceLock`, like `nz-offline`'s service.
#[derive(Clone)]
pub struct RemoteHost {
    shared: Arc<Shared>,
}

impl RemoteHost {
    /// The process's service, when one was started.
    pub fn get() -> Option<RemoteHost> {
        SERVICE.get().cloned()
    }

    /// What the chip and the sheet paint.
    pub fn status(&self) -> HostStatus {
        lock(&self.shared.status).clone()
    }

    /// Start the service for this session, once: the client, the runtime and a
    /// fresh wire session (a launch is always "new" to the server, so a restart
    /// republishes everything).
    pub fn ensure(
        api: Arc<Api>,
        cache: Cache,
        device: String,
        runtime: tokio::runtime::Handle,
    ) -> RemoteHost {
        let service = SERVICE.get_or_init(|| RemoteHost {
            shared: Arc::new(Shared::default()),
        });
        *lock(&service.shared.api) = Some(api);
        *lock(&service.shared.runtime) = Some(runtime);
        *lock(&service.shared.cache) = Some(cache);
        *lock(&service.shared.session) = Some(Session::new(device, page_id()));
        service.clone()
    }

    /// This device's id, for the sheet's create call.
    pub fn device(&self) -> String {
        lock(&self.shared.session)
            .as_ref()
            .map(|session| session.device.clone())
            .unwrap_or_default()
    }

    /// Whether the loops are running.
    pub fn running(&self) -> bool {
        *lock(&self.shared.looping)
    }

    /// Hand the player state to the wire: the foreground tick calls this, so the
    /// network task always publishes something recent.
    pub fn publish_state(&self, published: Published) {
        *lock(&self.shared.view) = Some(published);
        *lock(&self.shared.publish_due) = true;
    }

    /// Whether the store says this device has been lent before (`remote.hosted`).
    pub fn hosted(cache: &Cache) -> bool {
        cache.kv().get::<bool>(HOSTED_KEY).unwrap_or(false)
    }

    /// Remember that this device is lent, so the next launch asks the server
    /// whether a link still points here (`remote.hosted`).
    pub fn note_hosted(cache: &Cache) {
        let _ = cache.kv().set(HOSTED_KEY, &true);
    }

    /// Run the loops for this device (after a link was made, or a link was found
    /// at the session). Idempotent, and it records that this device is lent, so
    /// the next launch resumes without being asked.
    pub fn start(&self) {
        if let Some(cache) = lock(&self.shared.cache).clone() {
            let _ = cache.kv().set(HOSTED_KEY, &true);
        }
        *lock(&self.shared.stop) = false;
        {
            let mut looping = lock(&self.shared.looping);
            if *looping {
                return;
            }
            *looping = true;
        }
        let (Some(api), Some(runtime)) = (
            lock(&self.shared.api).clone(),
            lock(&self.shared.runtime).clone(),
        ) else {
            *lock(&self.shared.looping) = false;
            return;
        };
        if let Some(cache) = lock(&self.shared.cache).clone() {
            Self::note_hosted(&cache);
        }
        let this = self.clone();
        runtime.spawn(async move { this.poll_loop(api).await });
        info!(device = %self.device(), "contrôle à distance : ce lecteur répond");
    }

    /// Stop answering (the owner cut every link, or the server says none is
    /// left). Nothing is sent anywhere afterwards.
    pub fn stop(&self) {
        self.stop_with(None);
    }

    /// Cut every link onto this device — the owner's chip and « Tout couper »,
    /// always within reach. The loops stop BEFORE the answer: nothing is sent on
    /// a link that is being cut.
    pub fn cut_all(&self) {
        let Some(api) = lock(&self.shared.api).clone() else {
            return;
        };
        let Some(runtime) = lock(&self.shared.runtime).clone() else {
            return;
        };
        let device = self.device();
        self.stop();
        runtime.spawn(async move {
            match api.remote_revoke_device(&device).await {
                Ok(()) => info!("liens de contrôle coupés sur ce lecteur"),
                Err(error) => debug!(%error, "coupure des liens de contrôle"),
            }
        });
    }

    /// Ask the server whether a link points here, and start the loops if one
    /// does. The call the session makes at a login, so a restart with a live link
    /// answers again without anybody opening the sheet — and a device that has
    /// never been lent asks nothing at all.
    pub fn resume(&self, hosted_before: bool) {
        if !hosted_before {
            debug!("ce lecteur n'a jamais été prêté : aucune interrogation");
            return;
        }
        let Some(api) = lock(&self.shared.api).clone() else {
            return;
        };
        let Some(runtime) = lock(&self.shared.runtime).clone() else {
            return;
        };
        let this = self.clone();
        runtime.spawn(async move {
            match api.remote_links(Some(&this.device())).await {
                Ok(links) if !links.links.is_empty() => this.start(),
                Ok(_) => debug!("aucun lien de contrôle sur ce lecteur"),
                Err(error) => {
                    // Offline at start: creating a link starts the loops without
                    // asking, so this is not a dead end.
                    debug!(%error, "liens de contrôle illisibles au démarrage");
                }
            }
        });
    }

    // --- the network side ----------------------------------------------------

    /// The poll loop: ask for commands, absorb them, publish what is due, sleep.
    async fn poll_loop(&self, api: Arc<Api>) {
        let mut failures = 0_u32;
        loop {
            if self.stopped() {
                break;
            }
            let Some(path) = lock(&self.shared.session).as_ref().map(Session::poll_path) else {
                break;
            };
            let started = std::time::Instant::now();
            let polled = match api.get::<Value>(Lane::User, &path).await {
                Ok(fresh) => {
                    failures = 0;
                    let mut guard = lock(&self.shared.session);
                    match guard.as_mut() {
                        Some(session) => session.absorb_poll(&fresh.value),
                        None => break,
                    }
                }
                Err(error) => {
                    failures += 1;
                    if matches!(error, ApiError::Unauthorized | ApiError::RemoteEnded { .. }) {
                        // The session ended: whatever was lent went with it.
                        self.stop_with(Some(
                            "Session terminée : le contrôle à distance est arrêté.".to_owned(),
                        ));
                        break;
                    }
                    // Offline, a restart, a 5xx: keep asking, less and less
                    // often. The player itself plays on regardless.
                    debug!(%error, "interrogation du contrôle à distance");
                    tokio::time::sleep(backoff(failures)).await;
                    continue;
                }
            };
            if polled.ended {
                self.stop_with(None);
                break;
            }
            if polled.reset {
                // The channel was rebuilt: everything goes again, and the rows
                // this device already sent are forgotten with it.
                if let Some(session) = lock(&self.shared.session).as_mut() {
                    session.bump_queue();
                    session.bump_settings();
                }
                *lock(&self.shared.queue) = Push::default();
                *lock(&self.shared.settings) = Push::default();
                *lock(&self.shared.fingerprint) = 0;
                *lock(&self.shared.publish_due) = true;
            }
            let mut fresh = polled.needs_state;
            for command in polled.commands {
                lock(&self.shared.pending).push_back(command);
                fresh = true;
            }
            if fresh {
                *lock(&self.shared.publish_due) = true;
            }
            // Who is driving, for the chip and the fast period.
            let driving = {
                let mut status = lock(&self.shared.status);
                let changed = status.controllers != polled.controllers || !status.active;
                status.controllers = polled.controllers;
                status.active = true;
                status.error = None;
                if changed {
                    status.version += 1;
                }
                !status.controllers.is_empty()
            };
            let heartbeat =
                lock(&self.shared.published_at).is_none_or(|at| at.elapsed() >= HEARTBEAT);
            if heartbeat {
                *lock(&self.shared.publish_due) = true;
            }
            if self.take_publish_due() {
                self.publish(&api).await;
            }
            let period = if driving { FAST_POLL } else { SLOW_POLL };
            tokio::time::sleep(period.saturating_sub(started.elapsed())).await;
        }
        *lock(&self.shared.looping) = false;
    }

    fn stopped(&self) -> bool {
        *lock(&self.shared.stop)
    }

    fn take_publish_due(&self) -> bool {
        std::mem::take(&mut *lock(&self.shared.publish_due))
    }

    /// Publish the state, and the queue/settings when they are due.
    async fn publish(&self, api: &Api) {
        let Some(view) = lock(&self.shared.view).clone() else {
            // Nothing has been handed over yet (the first tick has not run):
            // the next poll asks again.
            *lock(&self.shared.publish_due) = true;
            return;
        };
        let (queue, sent_queue) = {
            let push = lock(&self.shared.queue);
            let rows = push
                .pending()
                .then(|| push.body.clone())
                .flatten()
                .and_then(|body| body.as_array().cloned());
            (rows, push.version.clone())
        };
        let (settings, sent_settings) = {
            let push = lock(&self.shared.settings);
            (
                push.pending().then(|| push.body.clone()).flatten(),
                push.version.clone(),
            )
        };
        let (path, body) = match lock(&self.shared.session).as_ref() {
            Some(session) => (
                session.publish_path(),
                session.publish_body(&view, queue.as_deref(), settings.as_ref()),
            ),
            None => return,
        };
        match api
            .send::<Value, Value>(Lane::User, reqwest::Method::POST, &path, Some(&body))
            .await
        {
            Ok(answer) => {
                let answer = answer.unwrap_or(Value::Null);
                if !answer
                    .get("active")
                    .and_then(Value::as_bool)
                    .unwrap_or(true)
                {
                    self.stop_with(None);
                    return;
                }
                *lock(&self.shared.published_at) = Some(std::time::Instant::now());
                let republish = match lock(&self.shared.session).as_mut() {
                    Some(session) => session.absorb_publish(&answer),
                    None => false,
                };
                if queue.is_some() {
                    let mut push = lock(&self.shared.queue);
                    if push.version == sent_queue {
                        push.sent = sent_queue;
                        push.body = None;
                    }
                }
                if settings.is_some() {
                    let mut push = lock(&self.shared.settings);
                    if push.version == sent_settings {
                        push.sent = sent_settings;
                        push.body = None;
                    }
                }
                if republish {
                    // A channel that lost what this page sent asks for it again.
                    *lock(&self.shared.fingerprint) = 0;
                    *lock(&self.shared.queue) = Push::default();
                    *lock(&self.shared.settings) = Push::default();
                    *lock(&self.shared.publish_due) = true;
                }
            }
            Err(error) => {
                // The poll loop owns the backoff; the next change retries.
                debug!(%error, "publication de l'état du lecteur");
                *lock(&self.shared.publish_due) = true;
            }
        }
    }

    fn stop_with(&self, error: Option<String>) {
        *lock(&self.shared.stop) = true;
        *lock(&self.shared.looping) = false;
        lock(&self.shared.pending).clear();
        {
            let mut status = lock(&self.shared.status);
            status.active = false;
            status.controllers.clear();
            if let Some(error) = error {
                status.error = Some(error);
            }
            status.version += 1;
        }
        if let Some(session) = lock(&self.shared.session).as_mut() {
            session.reset_cursor();
        }
    }

    // --- the app's own side --------------------------------------------------

    /// Take the commands a poll queued: the caller applies them on the thread
    /// that owns the player, then calls [`note_applied`](Self::note_applied).
    pub fn drain_pending(&self) -> Vec<Pending> {
        lock(&self.shared.pending).drain(..).collect()
    }

    /// Say that `ran` commands were applied: the ack this device publishes is the
    /// last number its poll cursor handed out.
    pub fn note_applied(&self, ran: usize) {
        if ran == 0 {
            return;
        }
        let applied = lock(&self.shared.session)
            .as_ref()
            .map(|session| session.applied)
            .unwrap_or(0);
        let mut status = lock(&self.shared.status);
        status.applied = applied;
        status.version += 1;
    }

    /// The queue's change detector, and the hand-over it triggers. One function
    /// so the state and the rows can never be published out of step; the caller
    /// hands over its own queue (`nz_player`'s `seq()` and `queue()`) every tick
    /// and gets the fingerprint back.
    pub fn hand_over_queue(&self, seq: u64, queue: &[Arc<Entry>]) -> u64 {
        let fingerprint = queue_fingerprint(seq, queue);
        let mut known = lock(&self.shared.fingerprint);
        if *known == fingerprint {
            return 0;
        }
        *known = fingerprint;
        if let Some(rows) = queue_values(queue) {
            let version = match lock(&self.shared.session).as_mut() {
                Some(session) => session.bump_queue(),
                None => String::new(),
            };
            *lock(&self.shared.queue) = Push {
                version,
                body: Some(Value::Array(rows)),
                sent: String::new(),
            };
        }
        fingerprint
    }

    /// Hand the settings to publish (the keys this build mirrors, see
    /// [`settings_snapshot`]).
    pub fn publish_settings(&self, settings: Value) {
        let version = match lock(&self.shared.session).as_mut() {
            Some(session) => session.bump_settings(),
            None => String::new(),
        };
        *lock(&self.shared.settings) = Push {
            version,
            body: Some(settings),
            sent: String::new(),
        };
        *lock(&self.shared.publish_due) = true;
    }

    /// Read the settings from the store and hand them over, once per session and
    /// whenever one of them changes.
    pub fn hand_over_settings(&self, cache: &Cache) {
        self.publish_settings(settings_snapshot(cache));
    }
}

impl Session {
    fn reset_cursor(&mut self) {
        self.since = 0;
        self.applied = 0;
    }
}

/// `off`, `all`, `one` — the wire's repeat spellings (`remote.py#_clean_state`).
pub fn repeat_wire(repeat: Repeat) -> &'static str {
    match repeat {
        Repeat::Off => "off",
        Repeat::All => "all",
        Repeat::One => "one",
    }
}

/// The backoff of a failing poll, `host.js`'s `1000 · 2^failures`, capped.
pub fn backoff(failures: u32) -> Duration {
    let factor = 1_u64 << failures.min(4);
    Duration::from_millis((1000 * factor).min(RETRY_MAX.as_millis() as u64))
}

/// A page id: the versions this launch sends must differ from the previous
/// launch's, so a restart is always "new" to the server.
pub fn page_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos() as u64)
        .unwrap_or(0);
    format!("{:x}{:x}", std::process::id(), nanos)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

// --- the settings a controller may change -----------------------------------

/// The device settings a controller MIRRORS, by the SPA's own key names
/// (`lib/remote/settings.js#MIRRORED` intersected with what this build applies).
///
/// Deliberately short, and the server is the gate: `remote.py#_a_set` only
/// accepts a key the player PUBLISHED, so a controller asking for a setting this
/// build does not have is refused there, not silently ignored here. What is
/// missing and why: `viz.*` belongs to the desktop's visualiser settings and the
/// `audio.*` / `fade.*` keys are this build's own — a controller that cannot see
/// those in its own Réglages would never send them either.
pub const MIRRORED: [&str; 5] = [
    "player.quality",
    "podcast.speeds",
    "offline.quality",
    "offline.onlyDownloaded",
    "cache.limit",
];

/// The desktop's `player.quality` value in the SPA's vocabulary (`stores.js`
/// keeps `"FLAC"`, `"OPUS_320"`…), which is what its Réglages show.
pub fn quality_wire(quality: &str) -> &'static str {
    match quality.trim().to_ascii_lowercase().as_str() {
        "flac" => "FLAC",
        "opus_320" | "320" => "OPUS_320",
        "opus_128" | "128" => "OPUS_128",
        "opus_64" | "64" => "OPUS_64",
        _ => "AUTO",
    }
}

/// The rung a controller asked for, or `None` when this player cannot honour it
/// (`OPUS_256`/`OPUS_192` have no rung in `nz-audio`, so offering them would
/// save a value the player cannot load — D13's own rule).
pub fn parse_quality(value: &Value) -> Option<&'static str> {
    match value.as_str()?.trim().to_ascii_uppercase().as_str() {
        "FLAC" => Some("flac"),
        "AUTO" => Some("auto"),
        "OPUS_320" | "320" => Some("opus_320"),
        "OPUS_128" | "128" => Some("opus_128"),
        "OPUS_64" | "64" => Some("opus_64"),
        _ => None,
    }
}

/// The offline default's French label, from the key D13 stores: the SPA's
/// `offline.quality` vocabulary is `nz_api::Quality`'s, so the wire value is
/// `quality.wire()`'s (`"OPUS_320"`, or FLAC for none).
pub fn offline_quality_wire(raw: &str) -> String {
    // `Quality::wire()` is the `q=` value of a stream URL, where FLAC is the
    // ABSENCE of `q` — that is NOT how a setting is spelled. The store writes
    // "FLAC" out loud (`nz-offline`'s `Settings::write`, "read by a human in
    // `sqlite3` as often as by the code"), so reading it back must spell it out
    // too: `and_then(wire)` with a default turned a person's FLAC into
    // OPUS_320 for every controller that mirrors their settings (SC5's find
    // (b), the desktop's own mirror included).
    match nz_api::Quality::parse(Some(raw)) {
        Some(quality) => quality.wire().unwrap_or("FLAC").to_owned(),
        None => "OPUS_320".to_owned(),
    }
}

/// The settings snapshot a publish carries: the SPA's keys, read from the same
/// `kv` every other screen uses.
pub fn settings_snapshot(cache: &Cache) -> Value {
    let kv = cache.kv();
    let quality = kv
        .get::<String>("playback.quality")
        .unwrap_or_else(|| "auto".to_owned());
    let speeds = kv.get::<Value>("podcast.speeds").unwrap_or(Value::Null);
    let offline_quality = kv
        .get::<String>("offline.quality")
        .unwrap_or_else(|| "opus_320".to_owned());
    let offline_only = kv.get::<bool>("offline.onlyDownloaded").unwrap_or(true);
    let cache_limit = kv.get::<i64>("cache.limit").unwrap_or(1024 * 1024 * 1024);
    json!({
        "player.quality": quality_wire(&quality),
        "podcast.speeds": speeds,
        "offline.quality": offline_quality_wire(&offline_quality),
        "offline.onlyDownloaded": offline_only,
        "cache.limit": cache_limit,
    })
}

/// What one `set` from a controller changed, so the caller can do the part that
/// is its own: the desktop sets its engine's rung and says so in a toast, the
/// phone's player does the same. The `kv` write is here, so the person's own
/// Réglages and a controller's `set` land in ONE place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Changed {
    /// `player.quality`: the rung to apply at the next track.
    Quality(&'static str),
    /// A key stored in `kv`, named as the log writes it (`offline.quality=FLAC`).
    Stored(String),
}

/// Apply one `set` command from a controller to this device: the same `kv` keys
/// the Réglages screen writes, so the change is the person's own. Returns what
/// changed, for the caller and the log, or `None` when this build cannot honour
/// it (the controller's value then snaps back on the next state read, which is
/// the honest answer).
///
/// `cache` is the caller's store: `None` is a session without one, where only
/// `player.quality` can be honoured (it lives in the player, not the store).
pub fn apply_setting(cache: Option<&Cache>, key: &str, value: &Value) -> Option<Changed> {
    if key == "player.quality" {
        let rung = parse_quality(value)?;
        if let Some(cache) = cache {
            let _ = cache.kv().set("playback.quality", &rung.to_owned());
        }
        // The SPA's chip says the same thing: the rung applies at the next
        // track, so a change during one does not restart it.
        return Some(Changed::Quality(rung));
    }
    let cache = cache?;
    let kv = cache.kv();
    match key {
        "offline.onlyDownloaded" => {
            let flag = value.as_bool()?;
            let _ = kv.set("offline.onlyDownloaded", &flag);
            Some(Changed::Stored(format!("offline.onlyDownloaded={flag}")))
        }
        "cache.limit" => {
            let bytes = value.as_i64()?;
            if bytes <= 0 {
                return None;
            }
            let _ = kv.set("cache.limit", &bytes);
            Some(Changed::Stored(format!("cache.limit={bytes}")))
        }
        "offline.quality" => {
            // D13 stores the WIRE spelling (`Settings::write`: `quality.wire()`,
            // so `OPUS_320` or `FLAC`), and an `OPUS_256` this build cannot
            // decode is refused rather than stored.
            let quality = nz_api::Quality::parse(value.as_str())?;
            if !matches!(
                quality,
                nz_api::Quality::Flac
                    | nz_api::Quality::Opus320
                    | nz_api::Quality::Opus128
                    | nz_api::Quality::Opus64
            ) {
                return None;
            }
            let wire = quality.wire().unwrap_or("FLAC");
            let _ = kv.set("offline.quality", &wire.to_owned());
            Some(Changed::Stored(format!("offline.quality={wire}")))
        }
        "podcast.speeds" => {
            if !value.is_null() && !value.is_object() {
                return None;
            }
            let _ = kv.set("podcast.speeds", value);
            Some(Changed::Stored("podcast.speeds".to_owned()))
        }
        _ => None,
    }
}

/// Read one mirrored setting as this build holds it (the controller's own `set`
/// diffing): the value in the SPA's shape, read from the one key it lives in.
pub fn mirrored_value(cache: &Cache, key: &str) -> Option<Value> {
    let kv = cache.kv();
    Some(match key {
        "player.quality" => {
            let raw = kv
                .get::<String>("playback.quality")
                .unwrap_or_else(|| "auto".to_owned());
            Value::String(quality_wire(&raw).to_owned())
        }
        "podcast.speeds" => kv.get::<Value>("podcast.speeds")?,
        "offline.quality" => {
            let raw = kv
                .get::<String>("offline.quality")
                .unwrap_or_else(|| "opus_320".to_owned());
            Value::String(offline_quality_wire(&raw))
        }
        "offline.onlyDownloaded" => Value::Bool(kv.get::<bool>("offline.onlyDownloaded")?),
        "cache.limit" => Value::from(kv.get::<i64>("cache.limit")?),
        _ => return None,
    })
}

/// A link's URL, built from the server's ROOT the way `lib/applink.js#shortLink`
/// builds it: a deployment under a path prefix keeps its prefix.
pub fn link_url(api: &Api, token: &str) -> String {
    format!(
        "{}/rc/{}",
        api.base_url(),
        path_segment(token.split('.').next().unwrap_or(token))
    )
}

/// A link row's expiry in French (`RemoteSheet.svelte#expiry`): « Sans limite »,
/// « Expire dans 3 h », or « Expiré ».
pub fn expiry_label(expires: Option<&str>) -> String {
    let Some(raw) = expires else {
        return "Sans limite".to_owned();
    };
    let Ok(then) = parse_iso_seconds(raw) else {
        return "Sans limite".to_owned();
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0);
    let left = then - now;
    if left <= 0 {
        return "Expiré".to_owned();
    }
    let minutes = left / 60;
    if minutes < 60 {
        format!("Expire dans {} min", minutes.max(1))
    } else if minutes < 48 * 60 {
        format!("Expire dans {} h", minutes / 60)
    } else {
        format!("Expire dans {} j", minutes / (60 * 24))
    }
}

/// `2026-11-16T21:04:05.123456+00:00` → seconds since the epoch. The server's
/// `isoformat()` carries microseconds and an offset; only the date and the time
/// matter here, and a value this parser does not know simply does not expire.
fn parse_iso_seconds(raw: &str) -> Result<i64, ()> {
    let bytes = raw.as_bytes();
    if bytes.len() < 19 {
        return Err(());
    }
    let number = |from: usize, to: usize| -> Result<i64, ()> {
        raw.get(from..to)
            .and_then(|text| text.parse::<i64>().ok())
            .ok_or(())
    };
    let year = number(0, 4)?;
    let month = number(5, 7)?;
    let day = number(8, 10)?;
    let hour = number(11, 13)?;
    let minute = number(14, 16)?;
    let second = number(17, 19)?;
    Ok(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second)
}

/// Howard Hinnant's `days_from_civil`: days since 1970-01-01 for a proleptic
/// Gregorian date. Twelve lines instead of a `chrono` dependency.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_index = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// One link row as the sheet prints it.
#[derive(Clone, Debug, Default)]
pub struct LinkRow {
    pub id: String,
    pub level: String,
    pub label: String,
    pub controllers: u32,
    pub expiry: String,
    pub url: String,
}

impl LinkRow {
    pub fn from_link(api: &Api, link: &RemoteLink) -> Self {
        Self {
            id: link.id.clone(),
            level: link.level.clone(),
            label: link.label.clone(),
            controllers: link.controllers,
            expiry: expiry_label(link.expires.as_deref()),
            url: link_url(api, &link.token),
        }
    }

    /// `« Complet · pour Léa »` — the sheet's own wording for a link.
    pub fn title(&self) -> String {
        let level = level_label(&self.level);
        if self.label.trim().is_empty() {
            level.to_owned()
        } else {
            format!("{level} · pour {}", self.label.trim())
        }
    }
}

/// `{"link": …}` of a create, read tolerantly.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Created {
    #[serde(default)]
    pub link: RemoteLink,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::{Read, parse_queue};
    use crate::testing::{Bench, Scratch, runtime};
    use nz_player::{Effect, PlayerState};

    /// The state one publish carries, from a `PlayerState`: the app builds a
    /// [`Published`] from its own state, and this is the same projection.
    fn snapshot_of_player(player: &PlayerState) -> Published {
        Published {
            index: player.index(),
            playing: player.playing(),
            p: player.current_time(),
            duration: player.duration(),
            volume: player.volume(),
            muted: player.muted(),
            shuffle: player.shuffle(),
            repeat: repeat_wire(player.repeat()).to_owned(),
            status: if player.playing() {
                "playing"
            } else {
                "paused"
            }
            .to_owned(),
            track: player
                .current()
                .and_then(|entry| serde_json::to_value(entry).ok()),
            context: player
                .context()
                .and_then(|context| serde_json::to_value(context).ok()),
        }
    }

    /// The body the host's service publishes for this player right now.
    fn publish_body(session: &Session, player: &PlayerState) -> Value {
        let rows = queue_values(player.queue());
        session.publish_body(&snapshot_of_player(player), rows.as_deref(), None)
    }

    #[test]
    fn a_device_id_is_one_the_server_takes() {
        let id = fresh_device_id();
        assert_eq!(id.len(), DEVICE_LEN);
        assert!(is_device_id(&id), "{id}");
        assert_ne!(id, fresh_device_id(), "deux identifiants frais diffèrent");
        assert!(!is_device_id("court"));
        assert!(!is_device_id("../../api/evil"));
    }

    #[test]
    fn a_wire_command_becomes_the_state_machines_own_type() {
        // The server's `OPS` and `nz_player::Command`'s serde tags are one
        // vocabulary, so a remote command is DESERIALIZED, never re-dispatched.
        let played = decode_command(&json!({"seq":3,"op":"play_queue","args":{
            "tracks":[{"id":"abc","title":"T","duration":3}],"start":0,"context":null}}))
        .expect("a queue command");
        assert_eq!(played.op(), "play_queue");
        assert_eq!(
            decode_command(&json!({"seq":4,"op":"next"})),
            Some(Command::next())
        );
        assert_eq!(
            decode_command(&json!({"seq":5,"op":"seek","args":{"t":42.5}})),
            Some(Command::seek(42.5))
        );
        assert_eq!(
            decode_command(&json!({"seq":6,"op":"jump","args":{"i":2,"id":"row"}})),
            Some(Command::jump_named(2, "row"))
        );
        // An op this build does not know is skipped, not a crash (rule 7).
        assert_eq!(decode_command(&json!({"seq":7,"op":"teleport"})), None);
    }

    #[test]
    fn a_poll_absorbs_its_commands_once_and_carries_the_cursor() {
        let mut session = Session::new("dev12345", "page1");
        assert_eq!(
            session.poll_path(),
            "/remote/host/dev12345/poll?since=0&chan="
        );
        let polled = session.absorb_poll(&json!({
            "active": true, "chan": "boot1", "reset": true, "seq": 2,
            "cmds": [
                {"seq":1,"op":"next","args":{},"link":"l1"},
                {"seq":2,"op":"pause","args":{},"link":"l1"}
            ],
            "controllers": [{"link":"l1","level":"queue","label":"Léa"}],
            "has_state": false
        }));
        assert!(polled.reset, "a channel this launch has not seen");
        assert!(polled.needs_state);
        assert_eq!(
            polled.commands,
            vec![
                Pending::Command(Command::next()),
                Pending::Command(Command::pause())
            ]
        );
        assert_eq!(polled.controllers[0].label, "Léa");
        assert_eq!(session.applied, 2, "the ack counts what it ran");
        assert!(session.poll_path().contains("since=2"));
        assert!(session.poll_path().contains("chan=boot1"));

        // The same answer replayed (a reload) does not run twice.
        let again = session.absorb_poll(&json!({
            "active": true, "chan": "boot1", "reset": false, "seq": 2,
            "cmds": [{"seq":2,"op":"pause","args":{},"link":"l1"}],
            "controllers": [], "has_state": true
        }));
        assert!(again.commands.is_empty(), "déjà exécutée : ignorée");
        assert!(!again.reset);
    }

    #[test]
    fn no_link_left_ends_the_loops() {
        let mut session = Session::new("dev12345", "page1");
        let polled = session.absorb_poll(&json!({"active": false}));
        assert!(polled.ended);
        assert!(polled.commands.is_empty());
    }

    #[test]
    fn a_publish_announces_a_new_queue_only_when_it_is_due() {
        let mut session = Session::new("dev12345", "page1");
        let published = Published {
            index: 3,
            playing: true,
            p: 12.5,
            duration: 200.0,
            volume: 0.8,
            muted: false,
            shuffle: true,
            repeat: "all".to_owned(),
            status: "playing".to_owned(),
            track: Some(json!({"id":"abc","title":"T"})),
            context: Some(json!({"kind":"album","id":"9"})),
        };
        let body = session.publish_body(&published, None, None);
        assert_eq!(body["state"]["index"], 3);
        assert_eq!(body["state"]["p"], 12.5);
        assert_eq!(body["state"]["repeat"], "all");
        assert_eq!(body["state"]["status"], "playing");
        assert!(body.get("queue").is_none(), "pas de file : rien n'a bougé");
        assert_eq!(body["ack"], 0);

        let version = session.bump_queue();
        assert_eq!(version, "page1.1");
        let rows = vec![json!({"id":"abc"})];
        let body = session.publish_body(&published, Some(&rows), None);
        assert_eq!(body["queue"][0]["id"], "abc");
        assert_eq!(body["qv"], "page1.1");

        // A channel that lacks what this page sent asks for it again.
        assert!(session.absorb_publish(
            &json!({"active":true,"chan":"boot1","need_queue":true,"need_settings":false})
        ));
        assert!(session.absorb_publish(
            &json!({"active":true,"chan":"boot1","need_queue":false,"need_settings":true})
        ));
        assert!(!session.absorb_publish(
            &json!({"active":true,"chan":"boot1","need_queue":false,"need_settings":false})
        ));
    }

    #[test]
    fn a_queue_fingerprint_follows_reorders_and_removals() {
        let entries: Vec<Arc<Entry>> = ["a", "b", "c"]
            .iter()
            .map(|id| Arc::new(Entry::new(*id)))
            .collect();
        let before = queue_fingerprint(0, &entries);
        assert_eq!(before, queue_fingerprint(0, &entries), "stable");
        assert_ne!(
            before,
            queue_fingerprint(1, &entries),
            "un changement de piste"
        );

        let mut swapped = entries.clone();
        swapped.swap(0, 2);
        assert_ne!(
            before,
            queue_fingerprint(0, &swapped),
            "un déplacement change la position, pas seulement l'ensemble"
        );
        let shorter = &entries[..2];
        assert_ne!(before, queue_fingerprint(0, shorter));
    }

    #[test]
    fn the_chip_names_who_is_driving() {
        let mut status = HostStatus::default();
        assert_eq!(status.who(), "");
        status.controllers = vec![Controller {
            link: "l".to_owned(),
            level: "queue".to_owned(),
            label: String::new(),
        }];
        assert_eq!(status.who(), "Contrôlé par 1 appareil");
        assert_eq!(status.level(), Some("File d'attente"));
        status.controllers[0].label = "Léa".to_owned();
        assert_eq!(status.who(), "Contrôlé par Léa");
        status.controllers.push(Controller {
            link: "m".to_owned(),
            level: "full".to_owned(),
            label: "Salon".to_owned(),
        });
        assert_eq!(status.who(), "Contrôlé par Léa et Salon");
        assert_eq!(status.level(), None);
        status.controllers.push(Controller {
            link: "n".to_owned(),
            level: "read".to_owned(),
            label: String::new(),
        });
        assert_eq!(status.who(), "Contrôlé par Léa et 2 autres");
    }

    #[test]
    fn a_link_expiry_reads_in_french() {
        assert_eq!(expiry_label(None), "Sans limite");
        // 2026-11-16T00:00:00Z is in the past for the clock this test runs on in
        // 2026; a far-future date is what the cases need.
        assert!(expiry_label(Some("2099-01-01T00:00:00+00:00")).starts_with("Expire dans"));
        assert_eq!(expiry_label(Some("2001-01-01T00:00:00+00:00")), "Expiré");
        assert_eq!(expiry_label(Some("n'importe quoi")), "Sans limite");
        // The epoch itself decodes exactly (the civil-days helper's own check),
        // and a date the server really sends: `now()` is a UTC ISO string.
        assert_eq!(parse_iso_seconds("1970-01-01T00:00:00Z"), Ok(0));
        assert_eq!(parse_iso_seconds("1970-01-02T00:00:00Z"), Ok(86_400));
        assert_eq!(parse_iso_seconds("2026-11-16T00:00:00Z"), Ok(1_794_787_200));
    }

    #[test]
    fn a_link_url_keeps_the_deployment_path_and_drops_the_mac() {
        // Built, never written out: a `<id>.<mac>` literal is what the secret
        // scanners read as an API key (`security.yaml`'s Gitleaks, measured).
        let token = format!("{}.{}", "id0", "0".repeat(24));
        let api = Api::new("http://192.168.1.20:5000").expect("a client");
        assert_eq!(link_url(&api, &token), "http://192.168.1.20:5000/rc/id0");
        let prefixed = Api::new("https://example.org/neezer").expect("a client");
        assert_eq!(
            link_url(&prefixed, &token),
            "https://example.org/neezer/rc/id0"
        );
    }

    #[test]
    fn the_backoff_stops_at_the_ceiling() {
        assert_eq!(backoff(1), Duration::from_millis(2000));
        assert_eq!(backoff(2), Duration::from_millis(4000));
        assert_eq!(backoff(9), RETRY_MAX);
    }

    #[test]
    fn the_settings_a_controller_mirrors_are_the_ones_this_build_applies() {
        // Every published key has a handler: a key in the snapshot with no
        // `apply_setting` arm would be a setting a controller can see and never
        // change. Each one is driven through the real store, so the two halves
        // (what is published, what is applied) are proven to be the same list.
        let scratch = Scratch::new("mirrored");
        let cache = scratch.cache();
        let cases: [(&str, Value); 5] = [
            ("player.quality", json!("OPUS_128")),
            ("podcast.speeds", json!({"1": 1.25})),
            // FLAC included since SC5's find (b) was fixed here: a stored
            // "FLAC" reads back as "FLAC", never as the OPUS_320 default.
            ("offline.quality", json!("FLAC")),
            ("offline.onlyDownloaded", json!(false)),
            ("cache.limit", json!(512 * 1024 * 1024_i64)),
        ];
        for key in MIRRORED {
            let (_, value) = cases
                .iter()
                .find(|(case, _)| *case == key)
                .unwrap_or_else(|| panic!("{key} est publié sans être applicable"));
            let changed = apply_setting(Some(&cache), key, value)
                .unwrap_or_else(|| panic!("{key} n'a pas de traitement"));
            // The controller sees the value it just set, read back through the
            // same `kv` a Réglages screen writes.
            assert_eq!(
                mirrored_value(&cache, key).as_ref(),
                Some(value),
                "{key} ne se relit pas tel qu'il a été écrit ({changed:?})"
            );
        }
        assert_eq!(quality_wire("opus_320"), "OPUS_320");
        assert_eq!(quality_wire("auto"), "AUTO");
        assert_eq!(parse_quality(&json!("OPUS_128")), Some("opus_128"));
        assert_eq!(
            parse_quality(&json!("OPUS_256")),
            None,
            "aucun barreau dans nz-audio : refusé plutôt qu'enregistré"
        );
        // A session without a store can still change the player's own rung, and
        // nothing else — the same rule the old `AppState` version had.
        assert_eq!(
            apply_setting(None, "player.quality", &json!("AUTO")),
            Some(Changed::Quality("auto"))
        );
        assert_eq!(apply_setting(None, "cache.limit", &json!(1)), None);
    }

    /// The service's own queue hand-over: the rows are cloned and given a new
    /// version only when the queue really changed (rule 4 on a 120 ms tick), and
    /// a poll's commands are drained once, for the thread that owns the player.
    #[test]
    fn a_queue_is_handed_over_only_when_it_changed() {
        let scratch = Scratch::new("handover");
        let runtime = runtime();
        let api = Api::new("http://127.0.0.1:9").expect("a client");
        let host = RemoteHost::ensure(
            Arc::new(api),
            scratch.cache(),
            "deskdev1".to_owned(),
            runtime.handle().clone(),
        );
        assert_eq!(host.device(), "deskdev1");
        let entries: Vec<Arc<Entry>> = ["a", "b"]
            .iter()
            .map(|id| Arc::new(Entry::new(*id)))
            .collect();

        assert_ne!(
            host.hand_over_queue(0, &entries),
            0,
            "une première file se remet"
        );
        // Asked again with the same queue and the same `seq`: nothing to send,
        // and no new version (rule 4 on a 120 ms tick).
        assert_eq!(host.hand_over_queue(0, &entries), 0, "rien n'a bougé");
        assert_ne!(
            host.hand_over_queue(1, &entries),
            0,
            "un changement de piste se remet"
        );
        assert_ne!(
            host.hand_over_queue(1, &entries[..1]),
            0,
            "une file plus courte diffère"
        );

        // A poll's commands wait for the app's thread, and are taken once.
        assert!(host.drain_pending().is_empty());
        assert_eq!(host.status().applied, 0);
        drop(runtime);
        drop(scratch);
    }

    /// **The lot's acceptance (SC5), the controlled half**: a `pause` posted by
    /// the CONTROLLER service reaches the host's channel, is applied to the
    /// owner's player, and comes back as the `ack` the controller reads — the
    /// same three bodies the SPA's `host.js`/`controller.js` exchange.
    ///
    /// Every step is driven by the previous answer, so there is no sleep and no
    /// background task: the controller's `submit` → `flush` → the server, the
    /// host's `Session` poll → publish, then the controller's `read_once` →
    /// `take_read`. Requests: 1 claim, 1 POST `/remote/cmd`, 2 polls, 2 publishes,
    /// 2 reads, 1 queue read — all on `Lane::User`.
    #[test]
    fn a_controllers_pause_reaches_the_host_and_is_acked() {
        let bench = Bench::start();
        let runtime = runtime();
        runtime.block_on(async {
            // -- the owner's player, and its channel -----------------------
            let owner = bench.client();
            owner.login("owner", "Owner1").await.expect("a real login");
            let created = owner
                .remote_create(&json!({
                    "level": "read",
                    "device": "deskdev1",
                    "device_name": "Linux · Neezer",
                    "label": "Le bench",
                    "ttl": 3600,
                }))
                .await
                .expect("a link");
            let token = created.link.token.clone();
            let (mut player, effects) = bench.playing_queue();
            assert!(
                effects
                    .iter()
                    .any(|effect| matches!(effect, Effect::Load { .. })),
                "the owner's queue starts a track"
            );

            let mut session = Session::new("deskdev1", page_id());
            // A channel this launch has not seen: everything goes again.
            let first = owner
                .get::<Value>(Lane::User, &session.poll_path())
                .await
                .expect("a poll")
                .value;
            assert!(session.absorb_poll(&first).reset);
            owner
                .send::<Value, Value>(
                    Lane::User,
                    reqwest::Method::POST,
                    &session.publish_path(),
                    Some(&publish_body(&session, &player)),
                )
                .await
                .expect("a publish")
                .expect("a body");

            // -- the controller: its own client, the claimed link -----------
            let controller_api = bench.client();
            controller_api
                .remote_claim(&token)
                .await
                .expect("the claim");
            let controller = crate::controller::Controller::ensure(
                Arc::new(controller_api.clone()),
                runtime.handle().clone(),
                crate::controller::Grant {
                    level: RemoteLevel::Full,
                    owner: "owner".to_owned(),
                    device: "deskdev1".to_owned(),
                    label: "Salon".to_owned(),
                },
            );
            // Its local view: the owner's queue, as a controller holds it.
            let mut local = PlayerState::new();
            local.apply(Command::play_queue(bench.queue(), 0, None), 0);

            // -- the tap ----------------------------------------------------
            assert_eq!(
                controller.submit(&mut local, Command::pause(), 0),
                crate::controller::Sent::Sent
            );
            assert!(!local.playing(), "le tap répond dans la même image");
            assert_eq!(controller.queued(), 1);
            controller.flush(&controller_api).await;
            assert_eq!(controller.queued(), 0, "la pause est partie");

            // -- the owner's poll carries it, and running it pauses --------
            let poll = owner
                .get::<Value>(Lane::User, &session.poll_path())
                .await
                .expect("a poll")
                .value;
            let polled = session.absorb_poll(&poll);
            assert_eq!(
                polled.commands,
                vec![Pending::Command(Command::pause())],
                "une seule commande, celle du tap"
            );
            for pending in &polled.commands {
                let Pending::Command(command) = pending else {
                    panic!("unexpected {pending:?}");
                };
                player.apply(command.clone(), 0);
            }
            assert!(
                !player.playing(),
                "la pause atteint le lecteur du propriétaire"
            );

            // -- the ack rides the next publish -----------------------------
            owner
                .send::<Value, Value>(
                    Lane::User,
                    reqwest::Method::POST,
                    &session.publish_path(),
                    Some(&publish_body(&session, &player)),
                )
                .await
                .expect("a publish")
                .expect("a body");
            assert_eq!(session.applied, 1, "un accusé pour une commande exécutée");

            // -- and the controller reads it --------------------------------
            assert_eq!(
                controller.read_once(&controller_api).await,
                crate::controller::ReadStep::Read
            );
            assert_eq!(controller.status().phase(), crate::controller::Phase::Live);
            let outcome = controller.take_read(&mut local, 0);
            assert!(
                outcome.adopted,
                "l'état du propriétaire remplace la vue locale"
            );
            assert!(!local.playing(), "le contrôleur voit la pause, en pause");
            let state = controller_api
                .get::<Value>(Lane::User, "/remote/state?sv=")
                .await
                .expect("the state")
                .value;
            assert_eq!(
                state.get("ack").and_then(Value::as_u64),
                Some(1),
                "l'accusé est le dernier numéro exécuté"
            );
        });
    }

    /// The moved D15a bench: a controller's commands reach the owner's engine
    /// within the lot's budget, the ack follows, a restart republishes, and the
    /// owner's cut ends the controller's session.
    ///
    /// The two sides are the SPA's own routes and bodies (`lib/remote/host.js`,
    /// `controller.js`): the host publishes `POST /api/remote/host/<device>`
    /// (state, queue, ack) and polls
    /// `GET /api/remote/host/<device>/poll?since=&chan=`, the controller reads
    /// `GET /api/remote/state` and `GET /api/remote/queue` and posts
    /// `POST /api/remote/cmd`.
    #[test]
    fn a_controllers_command_reaches_the_players_engine_in_under_the_budget() {
        let bench = Bench::start();
        let rt = runtime();
        rt.block_on(async {
            // -- the owner's player, and its channel -----------------------
            let owner = bench.client();
            let me = owner.login("owner", "Owner1").await.expect("a real login");
            assert!(me.user.admin, "the bench's owner is an admin");
            let created = owner
                .remote_create(&json!({
                    "level": "read",
                    "device": "deskdev1",
                    "device_name": "Linux · Neezer",
                    "label": "Le bench",
                    "ttl": 3600,
                }))
                .await
                .expect("a link");
            let token = created.link.token.clone();
            assert!(token.contains('.'), "a capability token");

            let (mut player, effects) = bench.playing_queue();
            assert!(
                effects
                    .iter()
                    .any(|effect| matches!(effect, Effect::Load { .. })),
                "the queue starts a track"
            );

            let mut session = Session::new("deskdev1", page_id());
            let publish = |session: &Session, player: &PlayerState| {
                let published = snapshot_of_player(player);
                let rows = queue_values(player.queue());
                session.publish_body(&published, rows.as_deref(), None)
            };

            // The first poll is a new channel: everything goes again.
            let first = owner
                .get::<Value>(Lane::User, &session.poll_path())
                .await
                .expect("a poll")
                .value;
            let polled = session.absorb_poll(&first);
            assert!(polled.reset, "a channel this launch has not seen");
            assert!(polled.commands.is_empty());
            let answer = owner
                .send::<Value, Value>(
                    Lane::User,
                    reqwest::Method::POST,
                    &session.publish_path(),
                    Some(&publish(&session, &player)),
                )
                .await
                .expect("a publish")
                .expect("a body");
            // The publish CARRIED the queue, so the channel has it: `need_queue`
            // is the server asking for what a publish left out.
            assert!(
                !answer
                    .get("need_queue")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                "la file est arrivée avec la première publication"
            );

            // -- the controller: claim, read, command ----------------------
            let controller = bench.client();
            controller.remote_claim(&token).await.expect("the claim");
            let state = controller
                .get::<Value>(Lane::User, "/remote/state?sv=")
                .await
                .expect("the state")
                .value;
            let read = Read::parse(&state);
            assert!(read.online, "the owner's player is answering");
            let snapshot = read.state.clone().expect("a state");
            assert_eq!(snapshot.queue.len(), 1, "the state names the playing track");
            assert_eq!(snapshot.queue[0].title, "Alpha");
            assert!(read.anchor.is_some_and(|anchor| anchor.playing));
            let queue = controller
                .get::<Value>(Lane::User, "/remote/queue")
                .await
                .expect("the queue")
                .value;
            let rows = parse_queue(&queue);
            assert_eq!(rows.len(), bench.queue().len(), "the whole queue crossed");
            assert_eq!(rows[0].title, "Alpha");

            // -- the taps: each POST to the moment the owner's player sounds --
            //
            // The owner's loop polls every FAST_POLL. A tap that lands just
            // BEFORE a poll is the best case (one round trip); one that lands
            // just after waits a whole period, which is the worst case the lot's
            // 1.2 s is about. Both are measured, and the worst is asserted.
            let mut worst = Duration::ZERO;
            let mut best = Duration::from_secs(60);
            for (index, op) in ["next", "prev", "next"].into_iter().enumerate() {
                // The cadence: a whole period has passed since the last poll, as
                // it has for a tap that lands between two of them.
                if index > 0 {
                    tokio::time::sleep(FAST_POLL).await;
                }
                let posted = std::time::Instant::now();
                let answer = controller
                    .send::<Value, Value>(
                        Lane::User,
                        reqwest::Method::POST,
                        "/remote/cmd",
                        Some(&json!({"op": op, "args": {}})),
                    )
                    .await
                    .expect("the command")
                    .expect("a body");
                assert!(answer.get("seq").and_then(Value::as_u64).unwrap_or(0) > 0);
                // Where the tap fell in the cadence: the first one just before a
                // poll (best case), the others right after one (a whole period).
                if index > 0 {
                    tokio::time::sleep(FAST_POLL).await;
                }
                loop {
                    let poll = owner
                        .get::<Value>(Lane::User, &session.poll_path())
                        .await
                        .expect("a poll")
                        .value;
                    let polled = session.absorb_poll(&poll);
                    assert!(!polled.ended);
                    if !polled.commands.is_empty() {
                        for pending in &polled.commands {
                            let Pending::Command(command) = pending else {
                                panic!("unexpected {pending:?}");
                            };
                            // The app applies it through its one transport path;
                            // here the state machine itself, whose `Load` is what
                            // makes sound.
                            let effects = player.apply(command.clone(), 0);
                            assert!(
                                effects
                                    .iter()
                                    .any(|effect| matches!(effect, Effect::Load { .. })),
                                "{op} loads a track: that is the sound"
                            );
                        }
                        let latency = posted.elapsed();
                        worst = worst.max(latency);
                        best = best.min(latency);
                        break;
                    }
                    assert!(
                        posted.elapsed() < Duration::from_secs(5),
                        "the command never arrived"
                    );
                    tokio::time::sleep(FAST_POLL).await;
                }
            }
            eprintln!(
                "commande → son : meilleur {best:?}, pire {worst:?} (période {} ms)",
                FAST_POLL.as_millis()
            );
            assert!(
                worst < Duration::from_millis(1200),
                "commande → son en {worst:?} (budget 1,2 s, période d'interrogation 700 ms)"
            );
            assert_eq!(player.index(), 1, "le deuxième titre du banc");

            // -- the ack: the controller learns its tap landed ---------------
            owner
                .send::<Value, Value>(
                    Lane::User,
                    reqwest::Method::POST,
                    &session.publish_path(),
                    Some(&publish(&session, &player)),
                )
                .await
                .expect("a publish")
                .expect("a body");
            let state = controller
                .get::<Value>(Lane::User, "/remote/state?sv=")
                .await
                .expect("the state")
                .value;
            assert_eq!(
                state.get("ack").and_then(Value::as_u64),
                Some(session.applied),
                "l'accusé est le dernier numéro exécuté"
            );
            assert_eq!(
                state
                    .get("state")
                    .and_then(|state| state.get("index"))
                    .and_then(Value::as_i64),
                Some(1),
                "le contrôleur voit le titre suivant"
            );

            // -- a restart: a new launch republishes everything --------------
            let mut restarted = Session::new("deskdev1", page_id());
            let poll = owner
                .get::<Value>(Lane::User, &restarted.poll_path())
                .await
                .expect("a poll")
                .value;
            let polled = restarted.absorb_poll(&poll);
            assert!(polled.reset, "a new launch is a new channel to the server");
            // A publish that leaves the queue out: the server answers that it
            // lacks it, which is the restart's own cue.
            let body = restarted.publish_body(&snapshot_of_player(&player), None, None);
            let answer = owner
                .send::<Value, Value>(
                    Lane::User,
                    reqwest::Method::POST,
                    &restarted.publish_path(),
                    Some(&body),
                )
                .await
                .expect("a publish")
                .expect("a body");
            assert!(
                answer
                    .get("need_queue")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                "the new channel starts empty: it asks for the queue"
            );
            let answer = owner
                .send::<Value, Value>(
                    Lane::User,
                    reqwest::Method::POST,
                    &restarted.publish_path(),
                    Some(&publish(&restarted, &player)),
                )
                .await
                .expect("a publish")
                .expect("a body");
            assert!(
                !answer
                    .get("need_queue")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                "the queue is back on the channel"
            );

            // -- the owner cuts: the controller is ended ---------------------
            owner
                .remote_revoke_device("deskdev1")
                .await
                .expect("the cut");
            let poll = owner
                .get::<Value>(Lane::User, &restarted.poll_path())
                .await
                .expect("a poll")
                .value;
            assert!(
                !poll.get("active").and_then(Value::as_bool).unwrap_or(true),
                "plus de lien : le lecteur cesse de répondre"
            );
            let ended = controller
                .get::<Value>(Lane::User, "/remote/state?sv=")
                .await;
            assert!(
                matches!(
                    ended,
                    Err(ApiError::Unauthorized) | Err(ApiError::RemoteEnded { .. })
                ),
                "la session du contrôleur est terminée : {ended:?}"
            );
        });
    }
}
