//! The LAN exchange between the person's devices: `Lan`, the adapters that make
//! playback and downloads ask a peer first, and the discovery the platform feeds
//! (docs/agents/lan-nodes.md, lot LN13).
//!
//! The phone becomes a node exactly like the desktop does, with two differences
//! that decide this module's shape:
//!
//! - **Discovery is the platform's.** Android's `NsdManager` browses and
//!   registers ([`LanCore::found`] / [`LanCore::lost`]), so the node is started
//!   with `DiscoveryKind::External` and this crate never multicasts.
//! - **The player and the downloads worker outlive or precede the node.** The
//!   player is opened from the first screen, the downloads worker from the first
//!   queue, and the node only exists once the person has a session (it
//!   authenticates with the session's own client). [`PlayerSource`] and
//!   `crate::offline`'s fetcher therefore look the node up when they are USED,
//!   not when they are built.
//!
//! # The rules this module keeps
//!
//! - **Rule 1, every byte has its urgency.** Nothing here opens a transfer
//!   itself: `nz-lan`'s `PeerFirstOpener` turns the engine's `Lane` into an
//!   `Urgency`, and `PeerFirstDownloader` takes the app's ONE `Gates` on `Bulk`.
//!   The one thing this module adds is that a queued playlist is `Bulk` work on a
//!   thread of its own, never on the caller's.
//! - **Rule 2, long work says so.** `download_sets` answers in one frame; the
//!   jobs it queues live in `nz-offline`, whose rows carry the progress and the
//!   cause of each failure.
//! - **Rule 3, a failure is a cause.** Every [`nz_lan::LanError`] becomes a
//!   [`CoreError`] with a French sentence, so `LoadFailed` prints it.
//! - **Rule 4, a read is one clone.** [`LanCore::snapshot`] is a short lock and
//!   the counted replicas: no SQLite, no await, nothing on Kotlin's main thread.
//! - **Rule 7, an older server changes nothing.** A server without
//!   `/api/lan/status` answers [`LanError::Unsupported`]; no node starts, no
//!   thread exists, no directory is created, and the screens hide the section.
//! - **Worker rule 6, every thread is joined.** The `nz-lan` runtime and its
//!   threads belong to [`nz_lan::Lan`] and are joined by [`LanCore::shutdown`];
//!   the listeners' watcher and the queued set downloads are this module's own
//!   and are joined by the same call, BEFORE it.
//!
//! # Why a process-wide slot
//!
//! `NzCore::lan()` is declared by LN0 as `Arc<LanApi>` over a unit struct, so it
//! cannot carry the core the way `PlayerApi::open(core)` does. The process's ONE
//! node therefore lives in [`SERVICE`], exactly like `crate::offline`'s worker,
//! and Kotlin reaches it through the FFI. [`LanCore::start`] stays public: a test
//! owns its own node, and two nodes in one process is what the two-device
//! acceptance needs.

#[cfg(test)]
mod harness;
mod source;
#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;

use nz_lan::settings as lan_settings;
use nz_lan::{
    Activity, DevicePub, DiscoveryKind, Lan, LanConfig, LanError, LanSettings, LanSnapshot,
    LanState, PeerSet, Relation, Tolerance,
};
use tracing::{debug, info, warn};

use crate::core::{Core, CoreError};

pub use source::PlayerSource;

/// What a screen is told when the server has no such feature (rule 7): the same
/// name `nz-api`'s own feature probe carries.
pub const FEATURE: &str = "L'échange entre appareils proches";

/// The platform name the node announces inside an authenticated connection, and
/// what the server stores in its device row.
pub const PLATFORM: &str = "android";

/// How many `"<peer>:<kind>:<ident>"` keys one call may queue: « Sur vos
/// appareils » multi-selects, and a caller must not make the device hold an
/// unbounded list of threads.
pub const MAX_SET_KEYS: usize = 64;

/// The state of the node, as a `when` in Kotlin switches on it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NodeState {
    /// Switched off in Réglages, or never started.
    #[default]
    Off,
    /// The server has no such feature: no screen and no setting mentions it.
    Unsupported,
    /// No credential yet: enrolment failed, or the server was never reached.
    NotEnrolled,
    /// Enrolled, looking for peers.
    Searching,
    /// Enrolled, with this many authenticated peers.
    Connected(u32),
}

/// One authenticated peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodePeer {
    /// The device's public key, hex: an opaque identifier, never a name.
    pub device_pub: String,
    pub name: String,
    /// `true` for one of my own devices, `false` for a trusted friend's.
    pub own: bool,
    pub platform: String,
    pub rtt_ms: u32,
    pub tracks: u32,
    pub sets: u32,
}

/// One transfer in flight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeTransfer {
    pub track: String,
    pub title: String,
    /// The peer's display name (sent inside the authenticated connection).
    pub peer: String,
    /// `true` when this device sends it to a peer.
    pub sending: bool,
    pub bytes: u64,
    pub total: u64,
    /// `"play"`, `"prefetch"` or `"bulk"` (rule 1's urgency).
    pub urgency: String,
}

/// One failure, with its cause (rule 2: never just a count).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeFailure {
    pub what: String,
    pub cause: String,
}

/// What the screens read in one clone of the node's state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeSnapshot {
    pub state: NodeState,
    pub key_until: Option<i64>,
    pub key_error: Option<String>,
    pub peers: Vec<NodePeer>,
    pub transfers: Vec<NodeTransfer>,
    pub failures: Vec<NodeFailure>,
    pub port: u16,
    pub port_fallback: bool,
}

/// The per-device settings Réglages edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSettings {
    /// « Échanger avec mes appareils à proximité ».
    pub enabled: bool,
    /// « Partager mes sons ».
    pub serve: bool,
    /// « Qualités tolérées », in the server's own spelling: `"FLAC"`,
    /// `"OPUS_320"`, `"OPUS_128"`, `"OPUS_64"`.
    pub tolerated: Vec<String>,
    /// The UDP port, `0` = any.
    pub port: u16,
}

impl Default for NodeSettings {
    fn default() -> Self {
        NodeSettings::from(LanSettings::default())
    }
}

/// One set a connected peer exposes, for « Sur vos appareils ».
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSet {
    /// The peer's public key, hex: the first half of `"<peer>:<kind>:<ident>"`.
    pub peer: String,
    pub peer_name: String,
    pub own: bool,
    pub kind: String,
    pub ident: String,
    pub title: String,
    pub cover: Option<String>,
    pub tracks: u32,
    pub local: u32,
    pub complete: bool,
    pub bytes: u64,
}

/// One enrolled device of the account.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeDevice {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub created: Option<String>,
    pub last_seen: Option<String>,
    pub revoked: Option<String>,
    /// `true` for the device the app is running on.
    pub current: bool,
}

/// One playlist a friend may take.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodePlaylist {
    pub id: String,
    pub title: String,
}

/// One trusted friend and what they may see.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeFriend {
    /// The ACCOUNT's id, a UUID string: never the LAN's opaque subject.
    pub user_id: String,
    pub name: String,
    pub all: bool,
    pub playlists: Vec<NodePlaylist>,
}

/// The two directions of trust.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeFriends {
    pub shares_to: Vec<NodeFriend>,
    pub trusted_by: Vec<NodeFriend>,
}

/// Who hears about a change: `nz-ffi` implements this over `LanListener`.
///
/// Called from the watcher thread with no lock held, and only when the node's
/// version really moved — never on a timer (mobile.md: a 4 Hz tick into a
/// sleeping UI is what the old app was).
pub trait ChangeSink: Send + Sync {
    fn changed(&self, version: u64);
}

/// The fan-out to the listeners, shared by the node and its watcher thread: the
/// watcher must not need the node itself (this crate forbids `unsafe`, so no
/// borrowed `&LanCore` crosses a thread boundary).
struct Fanout {
    sinks: Mutex<Vec<Arc<dyn ChangeSink>>>,
    version: AtomicU64,
    /// `true` once the node is shutting down: the watcher is blocked on the
    /// node's own `watch`, whose sender only dies with `Lan`, so a join without
    /// this would wait for ever (measured: the first version hung the shutdown
    /// test's 40 s deadline).
    stop: tokio::sync::watch::Sender<bool>,
}

impl Fanout {
    fn new() -> Fanout {
        let (stop, _) = tokio::sync::watch::channel(false);
        Fanout {
            sinks: Mutex::new(Vec::new()),
            version: AtomicU64::new(0),
            stop,
        }
    }

    fn push(&self, sink: Arc<dyn ChangeSink>) {
        self.sinks
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(sink);
    }

    /// One version to every listener, holding no lock while they run, and only
    /// when it moves: a repeated publish is not an event (LN10's rule).
    fn publish(&self, version: u64) {
        let sinks: Vec<Arc<dyn ChangeSink>> = self
            .sinks
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if sinks.is_empty() || self.version.swap(version, Ordering::SeqCst) == version {
            return;
        }
        for sink in sinks {
            sink.changed(version);
        }
    }

    fn clear(&self) {
        self.sinks
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
    }
}

/// The node, as the FFI and the adapters hold it: one per process, cloned freely.
pub struct LanCore {
    core: Arc<Core>,
    lan: Lan,
    fanout: Arc<Fanout>,
    /// The thread that watches `Lan::events()` and fans it out.
    watcher: Mutex<Option<JoinHandle<()>>>,
    /// The playlist downloads this module queued: joined before the node's
    /// runtime stops (worker rule 6).
    queued: Mutex<Vec<JoinHandle<()>>>,
}

impl std::fmt::Debug for LanCore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never a credential, a key or a roster.
        let snapshot = self.lan.snapshot();
        formatter
            .debug_struct("LanCore")
            .field("state", &snapshot.state)
            .field("port", &snapshot.port)
            .finish()
    }
}

/// What the process holds: nothing yet, a running node, or a server that said no.
enum Slot {
    Empty,
    /// A caller is building the node: a second one is told so instead of starting
    /// a second runtime over the same `lan.sqlite`.
    Starting,
    Running(Arc<LanCore>),
    /// The server has no such feature (an older server, or an admin who switched
    /// it off). Nothing is retried until a new session ([`reset`]): probing again
    /// on every screen would be a request storm for a feature that cannot work.
    Refused,
}

static SERVICE: OnceLock<Mutex<Slot>> = OnceLock::new();

fn slot() -> MutexGuard<'static, Slot> {
    SERVICE
        .get_or_init(|| Mutex::new(Slot::Empty))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

/// Starts the process's node, or answers the one already running.
///
/// **Call this from a runtime context** (every FFI `async fn` is): `Lan::start`
/// probes the server's feature first when it runs outside a runtime, and a
/// blocking request on Kotlin's main thread is exactly what mobile.md forbids.
/// Inside a runtime the probe is the driver's own first act instead.
///
/// `Err(NotEnrolled)` before the first successful login: the node authenticates
/// with the session's own client, and a device with no session has nothing to
/// enrol. Nothing is created in that case — no directory, no socket, no thread.
pub async fn attach(core: &Arc<Core>) -> Result<Arc<LanCore>, LanError> {
    // The slot's guard is taken and released inside this block: a
    // `std::sync::MutexGuard` may not be alive across an await (the FFI's
    // futures must stay `Send`), so the state machine marks `Starting` instead of
    // holding the lock while the node builds.
    {
        let mut held = slot();
        match &*held {
            Slot::Running(node) => return Ok(Arc::clone(node)),
            Slot::Refused => return Err(LanError::Unsupported),
            // A second caller while the first is starting: the shell asks once,
            // and answering "it is starting" is a cause a screen can print.
            Slot::Starting => {
                return Err(LanError::Internal("le nœud LAN démarre".to_owned()));
            }
            Slot::Empty => {}
        }
        *held = Slot::Starting;
    }
    if core.me().is_none() {
        *slot() = Slot::Empty;
        return Err(LanError::NotEnrolled);
    }
    match LanCore::start(Arc::clone(core)).await {
        Ok(node) => {
            *slot() = Slot::Running(Arc::clone(&node));
            Ok(node)
        }
        Err(LanError::Unsupported | LanError::Disabled) => {
            // Rule 7: an older server (or an admin who switched it off) is asked
            // once, never on every screen.
            *slot() = Slot::Refused;
            Err(LanError::Unsupported)
        }
        Err(error) => {
            *slot() = Slot::Empty;
            Err(error)
        }
    }
}

/// The node, when [`attach`] has run.
pub fn get() -> Option<Arc<LanCore>> {
    match &*slot() {
        Slot::Running(node) => Some(Arc::clone(node)),
        Slot::Empty | Slot::Starting | Slot::Refused => None,
    }
}

/// Forgets a refusal, so the next [`attach`] asks the server again: what a new
/// login does (the server may have been upgraded, or the admin may have switched
/// the feature back on).
pub fn reset() {
    let mut held = slot();
    if matches!(*held, Slot::Refused) {
        *held = Slot::Empty;
    }
}

/// Whether the server refused the feature (rule 7): what the FFI reads before a
/// node exists, so the section is hidden rather than drawn empty.
pub fn unsupported() -> bool {
    matches!(*slot(), Slot::Refused)
}

/// How many listeners may be waiting for a node that does not exist yet. The app
/// registers one per screen at most; the cap is what keeps a caller that loops
/// from making the core hold an unbounded list.
const MAX_PENDING: usize = 8;

/// Listeners registered before the node exists (`LanListener` is registered in
/// `onViewCreated`, the node starts when the session does).
static PENDING: Mutex<Vec<Arc<dyn ChangeSink>>> = Mutex::new(Vec::new());

/// Registers a listener on the node, or keeps it for the node that is about to
/// start. Never blocks and never starts a node itself.
pub fn subscribe(sink: Arc<dyn ChangeSink>) {
    if let Some(node) = get() {
        node.subscribe(sink);
        return;
    }
    let mut pending = PENDING.lock().unwrap_or_else(|error| error.into_inner());
    if pending.len() < MAX_PENDING && !pending.iter().any(|kept| Arc::ptr_eq(kept, &sink)) {
        pending.push(sink);
    }
}

/// The settings the device stored, for a screen that reads Réglages before the
/// node exists (a device that never enrolled, an offline trip).
pub fn stored_settings(core: &Core) -> NodeSettings {
    NodeSettings::from(lan_settings::load(core.cache()))
}

/// Persists settings for the node that starts later.
pub fn save_settings(core: &Core, settings: &NodeSettings) {
    lan_settings::save(core.cache(), &settings.to_lan());
}

/// Stops the process's node and joins every thread it owns (worker rule 6). Safe
/// to call twice, and safe when nothing was ever started.
pub fn shutdown() {
    let node = {
        let mut held = slot();
        match std::mem::replace(&mut *held, Slot::Empty) {
            Slot::Running(node) => Some(node),
            Slot::Empty | Slot::Starting | Slot::Refused => None,
        }
    };
    if let Some(node) = node {
        node.shutdown();
    }
}

impl LanCore {
    /// Builds the node over `core`'s client, caches and data directory.
    ///
    /// `Err(Unsupported)` when the server has no such feature, and then nothing
    /// was started: `nz-lan` probes before it creates a directory or a thread
    /// (rule 7).
    pub async fn start(core: Arc<Core>) -> Result<Arc<LanCore>, LanError> {
        let api = core.api().map_err(account_error)?;
        // The same probe `attach` makes, so a test and the FFI agree; `Lan::start`
        // skips its own inside a runtime (it would block the driver).
        if api
            .has_route(nz_api::features::Feature::Lan.probe())
            .await
            .is_some_and(|present| !present)
        {
            return Err(LanError::Unsupported);
        }
        let port = lan_settings::load(core.cache()).port;
        let config = LanConfig {
            // `$data/neezer/lan/`: `device.key` (0600) and `lan.sqlite`, in the
            // app's private storage and excluded from Android's backup.
            data_dir: core.data_path("lan"),
            device_name: device_name(&core),
            platform: PLATFORM,
            app_version: core.config().app_version.clone(),
            // `NsdManager` browses and registers; this crate never multicasts.
            discovery: DiscoveryKind::External,
            port,
        };
        let lan = Lan::start(config, api, core.cache().clone())?;
        let node = Arc::new(LanCore {
            core,
            lan,
            fanout: Arc::new(Fanout::new()),
            watcher: Mutex::new(None),
            queued: Mutex::new(Vec::new()),
        });
        info!(
            port = node.lan.snapshot().port,
            "nœud LAN démarré (découverte externe)"
        );
        // A listener registered before the node existed hears it from now on. It
        // is NOT told "changed" here: a screen reads `snapshot()` once, then
        // redraws on a real change (mobile.md's "on change only").
        {
            let mut pending = PENDING.lock().unwrap_or_else(|error| error.into_inner());
            for sink in pending.drain(..) {
                node.fanout.push(sink);
            }
        }
        // The downloads worker wraps its fetcher the moment a node exists; the
        // player's opener looks the node up on every load and needs nothing.
        crate::offline::reconfigure();
        Ok(node)
    }

    /// The node itself, for `nz-lan`'s adapters (a `Lan` clones cheaply).
    pub fn lan(&self) -> Lan {
        self.lan.clone()
    }

    /// The core this node was built over.
    pub fn core(&self) -> &Arc<Core> {
        &self.core
    }

    /// What the screens draw, cloned from memory: one short lock and the counted
    /// replicas, no SQLite and no await (rule 4).
    pub fn snapshot(&self) -> NodeSnapshot {
        NodeSnapshot::from(self.lan.snapshot())
    }

    /// The version channel, for a caller that waits on a change itself (the
    /// watcher does; a test may).
    pub fn events(&self) -> tokio::sync::watch::Receiver<u64> {
        self.lan.events()
    }

    /// Registers one listener and starts the watcher thread on first use.
    ///
    /// Nothing is pushed here: a screen reads [`LanCore::snapshot`] once and then
    /// redraws on each `changed`, so an idle node costs **0 events**.
    pub fn subscribe(&self, sink: Arc<dyn ChangeSink>) {
        self.fanout.push(sink);
        let mut watcher = self
            .watcher
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if watcher.is_some() {
            return;
        }
        let mut events = self.lan.events();
        let fanout = Arc::clone(&self.fanout);
        let stop = fanout.stop.subscribe();
        let spawned = std::thread::Builder::new()
            .name("nz-lan-watch".to_owned())
            .spawn(move || watch(fanout, &mut events, stop));
        match spawned {
            Ok(thread) => *watcher = Some(thread),
            Err(error) => warn!(%error, "nz-lan : le guetteur n'a pas démarré"),
        }
    }

    /// The platform resolved a node at `host`:`port` (`NsdManager`'s callback).
    ///
    /// A host that is not an IP literal is dropped: discovery only ever carries
    /// an address, and nothing from the platform becomes a name to resolve.
    pub fn found(&self, host: &str, port: u16) {
        match host.trim_matches(['[', ']']).parse() {
            Ok(ip) => self.lan.found(ip, port),
            Err(_) => debug!("nz-lan : adresse annoncée illisible, ignorée"),
        }
    }

    /// The platform lost a node it had resolved.
    pub fn lost(&self, host: &str, port: u16) {
        if let Ok(ip) = host.trim_matches(['[', ']']).parse() {
            self.lan.lost(ip, port);
        }
    }

    /// The network moved (`ConnectivityManager`'s callback): re-announce and try
    /// the last-known addresses of the peers this device had, which is the
    /// reconnect path when multicast is filtered but unicast passes.
    pub fn network_changed(&self) {
        self.lan.network_changed();
    }

    /// Tells the node what the app is doing, for the battery rules: browsing and
    /// announcing run only while one of the three is true (and 60 s after).
    pub fn set_active(&self, foreground: bool, playing: bool, downloading: bool) {
        self.lan.set_active(Activity::Foreground, foreground);
        self.lan.set_active(Activity::Playing, playing);
        self.lan.set_active(Activity::Downloading, downloading);
    }

    /// How long the node keeps its connections after the app stopped being
    /// active. A test cannot pause the clock of a runtime it does not own, so it
    /// shortens the grace instead of waiting the real minute.
    pub fn set_sleep_grace(&self, grace: Duration) {
        self.lan.set_sleep_grace(grace);
    }

    /// The settings as they are now.
    pub fn settings(&self) -> NodeSettings {
        NodeSettings::from(self.lan.settings())
    }

    /// Writes them: applied at once for the session and persisted by `nz-lan`.
    pub fn set_settings(&self, settings: &NodeSettings) {
        self.lan.set_settings(settings.to_lan());
    }

    /// Enrols (or renews) now instead of waiting for the daily run.
    pub fn enroll_now(&self) {
        self.lan.enroll_now();
    }

    /// Every set the connected peers expose, from the replica: 0 requests.
    pub fn peer_sets(&self) -> Vec<NodeSet> {
        self.lan.sets().into_iter().map(NodeSet::from).collect()
    }

    /// The port Kotlin registers with `NsdManager`: the one really bound, which
    /// is the ephemeral fallback when the preferred port was taken.
    pub fn advertised_port(&self) -> u16 {
        self.lan.snapshot().port
    }

    /// The bytes this node's state costs, counted (never an RSS).
    pub fn memory(&self) -> usize {
        self.lan.memory()
            + self
                .fanout
                .sinks
                .lock()
                .map(|sinks| sinks.len() * std::mem::size_of::<Arc<dyn ChangeSink>>())
                .unwrap_or(0)
    }

    /// What this node put on the socket, counted (`nz-lan`'s own quinn counters):
    /// the battery measure of "nothing is sent afterwards".
    pub fn wire(&self) -> nz_lan::service::WireCounts {
        self.lan.wire()
    }

    /// Where `track` will come from, from memory: no request, no waiting.
    pub fn decide(&self, track: &str, wanted: nz_api::Quality) -> nz_lan::Source {
        self.lan.decide(track, wanted, nz_lan::Urgency::Play)
    }

    /// Queues the sets named `"<peer>:<kind>:<ident>"` as ordinary downloads.
    ///
    /// Answers in one frame (rule 4): what the replica already knows is checked
    /// HERE (a set no peer has complete is a `NotFound` the screen can act on),
    /// then the queueing — one `Sets` pass at worst, plus the metadata and the
    /// sleeves — runs on a thread of its own, which is `Bulk` work for `nz-lan`
    /// (rule 1) and an ordinary `nz-offline` job for the person (rule 2).
    pub fn download_sets(&self, keys: &[String]) -> Result<usize, LanError> {
        if keys.is_empty() {
            return Ok(0);
        }
        if keys.len() > MAX_SET_KEYS {
            return Err(LanError::Internal(format!(
                "{MAX_SET_KEYS} ensembles au plus par demande"
            )));
        }
        let known = self.lan.sets();
        let peers: Vec<DevicePub> = self
            .lan
            .snapshot()
            .peers
            .into_iter()
            .map(|peer| peer.device_pub)
            .collect();
        let mut parsed = Vec::with_capacity(keys.len());
        for raw in keys {
            let key = parse_set_key(raw)?;
            // A peer that is not connected cannot serve a set: said now, not on a
            // worker thread minutes later.
            if !peers.contains(&key.peer) {
                return Err(LanError::PeerGone);
            }
            if let Some(set) = known.iter().find(|row| {
                row.peer == key.peer && row.set.kind == key.kind && row.set.ident == key.ident
            }) && (!set.set.complete || set.set.tracks.is_empty())
            {
                // Not "complete" on that device any more: the screen offers it
                // again rather than queueing a set that would half-fail.
                return Err(LanError::NotFound);
            }
            parsed.push(key);
        }
        let Some(offline) = crate::offline::get() else {
            return Err(LanError::Internal(
                "le service hors ligne n'est pas démarré".to_owned(),
            ));
        };
        let queued = parsed.len();
        // `download_peer_sets` speaks to peers through `nz-lan`'s runtime and
        // hands the jobs to `nz-offline`; neither may run on the caller's thread
        // (it is Kotlin's), nor on the LAN runtime's own single worker.
        let node = self.lan.clone();
        let spawned = std::thread::Builder::new()
            .name("nz-lan-sets".to_owned())
            .spawn(move || {
                match nz_lan::sets::download_peer_sets(&node, offline.worker(), &parsed) {
                    Ok(created) => debug!(created, "ensembles d'un pair mis en file"),
                    // The set moved between the screen and the tap, or the peer
                    // went: the downloads screen shows nothing, and the next
                    // refresh offers the set again. Logged with its cause.
                    Err(error) => warn!(cause = %error, "téléchargement d'un ensemble refusé"),
                }
            });
        match spawned {
            Ok(thread) => {
                let mut threads = self
                    .queued
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                // A finished thread is joined and forgotten here, so a long
                // session does not accumulate handles.
                threads.retain(|handle| !handle.is_finished());
                threads.push(thread);
            }
            Err(error) => {
                return Err(LanError::Internal(format!(
                    "le téléchargement n'a pas pu démarrer : {error}"
                )));
            }
        }
        Ok(queued)
    }

    /// The account's enrolled devices (`User` lane: the screen is open).
    pub async fn devices(&self) -> Result<Vec<NodeDevice>, CoreError> {
        let api = self.core.api()?;
        let rows = api.lan_devices().await.map_err(CoreError::from)?;
        Ok(rows.into_iter().map(NodeDevice::from).collect())
    }

    /// Revokes one device on the server; every peer refuses it at its next sync.
    pub async fn revoke(&self, id: &str) -> Result<(), CoreError> {
        let api = self.core.api()?;
        api.lan_revoke(id).await.map_err(CoreError::from)
    }

    /// Renames one device, 1–64 characters.
    pub async fn rename(&self, id: &str, name: &str) -> Result<(), CoreError> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 64 {
            return Err(CoreError::Invalid {
                detail: "Le nom doit faire entre 1 et 64 caractères.".to_owned(),
            });
        }
        let api = self.core.api()?;
        api.lan_rename(id, name)
            .await
            .map(|_| ())
            .map_err(CoreError::from)
    }

    /// The two directions of trust.
    pub async fn friends(&self) -> Result<NodeFriends, CoreError> {
        let api = self.core.api()?;
        let friends = api.lan_friends().await.map_err(CoreError::from)?;
        Ok(NodeFriends::from(friends))
    }

    /// Adds a trusted friend by exact account name.
    pub async fn add_friend(&self, name: &str) -> Result<(), CoreError> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 64 {
            return Err(CoreError::Invalid {
                detail: "Le nom du compte doit faire entre 1 et 64 caractères.".to_owned(),
            });
        }
        let api = self.core.api()?;
        api.lan_add_friend(name)
            .await
            .map(|_| ())
            .map_err(CoreError::from)
    }

    /// What one friend may fetch: everything, or the listed playlists.
    pub async fn set_friend_scope(
        &self,
        user_id: &str,
        all: bool,
        playlists: &[String],
    ) -> Result<(), CoreError> {
        let api = self.core.api()?;
        let scope = nz_model::lan::LanScope {
            all,
            playlists: playlists.to_vec(),
        };
        let id = nz_model::UserId::new(user_id);
        api.lan_set_friend(&id, &scope)
            .await
            .map(|_| ())
            .map_err(CoreError::from)
    }

    /// Stops sharing with one friend; their devices stop at their next roster
    /// sync.
    pub async fn remove_friend(&self, user_id: &str) -> Result<(), CoreError> {
        let api = self.core.api()?;
        let id = nz_model::UserId::new(user_id);
        api.lan_remove_friend(&id).await.map_err(CoreError::from)
    }

    /// Logs out: wipes the key, the credential and the roster.
    pub fn forget(&self) {
        self.lan.forget();
    }

    /// The failure the FFI turns into an `NzError` with a French sentence.
    pub fn shutdown_error_text(error: &LanError) -> String {
        match error {
            LanError::Unsupported | LanError::Disabled => FEATURE.to_owned(),
            other => other.cause_fr(),
        }
    }

    /// Stops everything and JOINS: the watcher, the queued downloads, then the
    /// `nz-lan` runtime with its own threads. Idempotent.
    pub fn shutdown(&self) {
        let _ = self.fanout.stop.send(true);
        {
            let mut watcher = self
                .watcher
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(thread) = watcher.take() {
                let _ = thread.join();
            }
        }
        {
            let mut queued = self
                .queued
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let handles: Vec<JoinHandle<()>> = queued.drain(..).collect();
            for handle in handles {
                let _ = handle.join();
            }
        }
        self.fanout.clear();
        self.lan.shutdown();
        debug!("nœud LAN arrêté");
    }
}

/// The downloads worker's fetcher, wrapped by `nz-lan`'s peer-first one when a
/// node exists (`crate::offline`'s `configure` calls this on every path).
///
/// `None` means "no node yet": the worker keeps the plain `ApiDownloader`, which
/// is rule 3's fail-fast server path and never a stub of a LAN fetch.
pub fn peer_downloader(
    inner: Arc<dyn nz_offline::Downloader>,
) -> Option<Arc<dyn nz_offline::Downloader>> {
    let node = get()?;
    Some(Arc::new(nz_lan::downloader::PeerFirstDownloader::new(
        inner,
        node.lan(),
    )))
}

/// The watcher thread's body: `Lan::events()` (a `watch` channel) fanned out to
/// the listeners on change only.
///
/// This thread owns its current-thread runtime, so it borrows nothing from the
/// `nz-lan` runtime, and [`LanCore::shutdown`] joins it before that runtime stops
/// (worker rule 6). No timer: an idle node wakes nothing.
fn watch(
    fanout: Arc<Fanout>,
    events: &mut tokio::sync::watch::Receiver<u64>,
    mut stop: tokio::sync::watch::Receiver<bool>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            warn!(%error, "nz-lan : le guetteur n'a pas de runtime");
            return;
        }
    };
    loop {
        // Two things end the wait: a new version, or the node shutting down (its
        // own `watch` sender only dies with `Lan`, which is joined after this
        // thread — hence the explicit stop).
        let alive = runtime.block_on(async {
            tokio::select! {
                changed = events.changed() => changed.is_ok(),
                _ = stop.changed() => false,
            }
        });
        if !alive {
            break;
        }
        let version = *events.borrow_and_update();
        fanout.publish(version);
    }
}

/// One `"<peer>:<kind>:<ident>"` key, parsed before anything is asked of a peer.
///
/// Nothing a screen sends becomes a path: the peer is a hex key this node already
/// holds, the kind is one of three literals (`nz-lan` refuses the rest), and the
/// ident is the server's own id, length-checked here and again by `fetch_set`.
fn parse_set_key(raw: &str) -> Result<nz_lan::sets::PeerSetKey, LanError> {
    let mut parts = raw.splitn(3, ':');
    let (Some(peer), Some(kind), Some(ident)) = (parts.next(), parts.next(), parts.next()) else {
        return Err(LanError::Internal(
            "clé d'ensemble illisible : forme attendue <appareil>:<genre>:<ident>".to_owned(),
        ));
    };
    let key: [u8; 32] = hex_decode(peer)
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .ok_or_else(|| LanError::Internal("clé d'appareil illisible".to_owned()))?;
    if ident.is_empty() || ident.len() > 64 {
        return Err(LanError::Internal(
            "identifiant d'ensemble invalide".to_owned(),
        ));
    }
    Ok(nz_lan::sets::PeerSetKey {
        peer: key,
        kind: kind.to_owned(),
        ident: ident.to_owned(),
    })
}

/// Lowercase hex of a byte string, without a dependency: a device key is the only
/// thing this module ever encodes.
pub fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        out.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
    }
    out
}

/// The inverse of [`hex_encode`]; `None` on an odd length, a non-hex digit or a
/// string longer than any key this protocol has, so hostile input is refused
/// before anything is allocated.
pub fn hex_decode(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || text.len() > 2 * 64 {
        return None;
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks(2) {
        let high = char::from(pair[0]).to_digit(16)?;
        let low = char::from(pair[1]).to_digit(16)?;
        out.push((high as u8) << 4 | low as u8);
    }
    Some(out)
}

/// Why no node can be started yet, in `nz-lan`'s own words.
fn account_error(error: CoreError) -> LanError {
    match error {
        // No link at all: the node is not a failure, it is simply not reachable.
        CoreError::Offline => LanError::Server("Hors ligne".to_owned()),
        // No server configured, or no session: nothing to enrol with.
        CoreError::Invalid { .. } => LanError::NotEnrolled,
        other => LanError::Internal(other.to_string()),
    }
}

/// The name a peer sees for this device: what the phone is called, trimmed to the
/// 64 characters the credential allows. It only travels inside an authenticated
/// connection.
fn device_name(core: &Core) -> String {
    let name = core.config().device_name.trim();
    if name.is_empty() {
        "Téléphone".to_owned()
    } else {
        name.chars().take(64).collect()
    }
}

// --- conversions: nz-lan's values are the node's, this module's are the app's ---

impl From<LanState> for NodeState {
    fn from(state: LanState) -> Self {
        match state {
            LanState::Off => NodeState::Off,
            LanState::Unsupported => NodeState::Unsupported,
            LanState::NotEnrolled => NodeState::NotEnrolled,
            LanState::Searching => NodeState::Searching,
            LanState::Connected(peers) => NodeState::Connected(peers),
        }
    }
}

impl From<LanSnapshot> for NodeSnapshot {
    fn from(snapshot: LanSnapshot) -> Self {
        NodeSnapshot {
            state: snapshot.state.into(),
            key_until: snapshot.key_until,
            key_error: snapshot.key_error,
            peers: snapshot.peers.into_iter().map(NodePeer::from).collect(),
            transfers: snapshot
                .transfers
                .into_iter()
                .map(NodeTransfer::from)
                .collect(),
            failures: snapshot
                .failures
                .into_iter()
                .map(|(what, cause)| NodeFailure { what, cause })
                .collect(),
            port: snapshot.port,
            port_fallback: snapshot.port_fallback,
        }
    }
}

impl From<nz_lan::PeerView> for NodePeer {
    fn from(peer: nz_lan::PeerView) -> Self {
        NodePeer {
            device_pub: hex_encode(&peer.device_pub),
            name: peer.name,
            own: matches!(peer.relation, Relation::Own),
            platform: peer.platform,
            rtt_ms: peer.rtt_ms,
            tracks: peer.tracks,
            sets: peer.sets,
        }
    }
}

impl From<nz_lan::TransferView> for NodeTransfer {
    fn from(transfer: nz_lan::TransferView) -> Self {
        NodeTransfer {
            track: transfer.track,
            title: transfer.title,
            peer: transfer.peer,
            sending: matches!(transfer.direction, nz_lan::Direction::Send),
            bytes: transfer.bytes,
            total: transfer.total,
            urgency: match transfer.urgency {
                nz_lan::Urgency::Play => "play",
                nz_lan::Urgency::Prefetch => "prefetch",
                nz_lan::Urgency::Bulk => "bulk",
            }
            .to_owned(),
        }
    }
}

impl From<PeerSet> for NodeSet {
    fn from(row: PeerSet) -> Self {
        NodeSet {
            peer: hex_encode(&row.peer),
            peer_name: row.peer_name,
            own: matches!(row.relation, Relation::Own),
            kind: row.set.kind,
            ident: row.set.ident,
            title: row.set.title,
            cover: row.set.cover,
            tracks: row.set.tracks.len() as u32,
            local: row.set.local,
            complete: row.set.complete,
            bytes: row.bytes,
        }
    }
}

impl From<LanSettings> for NodeSettings {
    fn from(settings: LanSettings) -> Self {
        NodeSettings {
            enabled: settings.enabled,
            serve: settings.serve,
            tolerated: settings
                .tolerated
                .0
                .iter()
                .map(|quality| quality.wire().unwrap_or("FLAC").to_owned())
                .collect(),
            port: settings.port,
        }
    }
}

impl NodeSettings {
    /// Back to `nz-lan`'s own value. A list nothing parses falls back to the
    /// default three: an empty set would take every peer copy while the server is
    /// reachable, which is the regression the note names.
    fn to_lan(&self) -> LanSettings {
        let mut tolerated = Tolerance(std::collections::BTreeSet::new());
        for spelling in &self.tolerated {
            if let Some(quality) = nz_api::Quality::parse(Some(spelling)) {
                tolerated.0.insert(quality);
            }
        }
        if tolerated.0.is_empty() {
            tolerated = Tolerance::default_set();
        }
        LanSettings {
            enabled: self.enabled,
            serve: self.serve,
            tolerated,
            port: self.port,
        }
    }
}

impl From<nz_model::lan::LanDevice> for NodeDevice {
    fn from(device: nz_model::lan::LanDevice) -> Self {
        NodeDevice {
            id: device.id,
            name: device.name,
            platform: device.platform,
            created: device.created,
            last_seen: device.last_seen,
            revoked: device.revoked,
            current: device.current,
        }
    }
}

impl From<nz_model::lan::LanFriend> for NodeFriend {
    fn from(friend: nz_model::lan::LanFriend) -> Self {
        NodeFriend {
            user_id: friend.user_id.into_inner(),
            name: friend.name,
            all: friend.all,
            playlists: friend
                .playlists
                .into_iter()
                .map(|row| NodePlaylist {
                    id: row.id,
                    title: row.title,
                })
                .collect(),
        }
    }
}

impl From<nz_model::lan::LanFriends> for NodeFriends {
    fn from(friends: nz_model::lan::LanFriends) -> Self {
        NodeFriends {
            shares_to: friends
                .shares_to
                .into_iter()
                .map(NodeFriend::from)
                .collect(),
            trusted_by: friends
                .trusted_by
                .into_iter()
                .map(NodeFriend::from)
                .collect(),
        }
    }
}
