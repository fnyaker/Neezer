//! `Lan` — the one handle the apps hold (docs/agents/lan-nodes.md,
//! *Architecture*).
//!
//! **LN0 declared the whole surface; LN10 filled it.** What every later lot reads
//! from here, and what the apps rely on:
//!
//! - [`Lan::start`] returns `Err(LanError::Unsupported)` on a server without the
//!   feature, and then NOTHING runs: no runtime, no thread, no socket, and the
//!   data directory was not even created (rule 7, D17's "an older server hides the
//!   feature"). The probe is the first thing it does.
//! - The crate owns its own tokio runtime (one worker, threads named `nz-lan`,
//!   niced +5), and [`Lan::shutdown`] joins it — along with the `nz-lan-hash`
//!   worker and the discovery daemon. The app's shutdown order is engine →
//!   offline worker → `Lan` → `nz-net`, because each borrows the previous one's
//!   runtime.
//! - [`Lan::snapshot`] is a CLONE taken from one short mutex and the counted
//!   replicas. It never touches SQLite and never waits on the runtime: it is read
//!   on the UI thread.
//! - [`Lan::events`] is a `watch` of a version counter: readers wake when
//!   something changed, and a screen that was not open costs nothing.
//! - [`Lan::memory`] is counted bytes (rule: never the RSS of a shared process),
//!   and [`Lan::wire`] is what quinn really put on the socket — the battery
//!   measure: once the node is asleep neither number moves.
//! - [`Lan::decide`] answers from memory, without the network: it is what a tap
//!   calls, and it costs no request.
//!
//! Activity ([`Activity`]) is what keeps the battery: browsing, announcing and
//! connections run only while the app is in the foreground, playing, or has
//! downloads to do, and stop [`SLEEP_GRACE`] after the last of those.
//! `set_active(…, false)` closes the connections, so an idle phone sends nothing
//! at all.
//!
//! # How the pieces are wired
//!
//! ```text
//! nz-lan runtime (threads `nz-lan`, nice +5)
//!   driver    ── identity, endpoint, discovery, timers, the dial table
//!   enrol     ── a child task: `ensure_enrolled` + the conditional roster sync
//!   accept    ── its own task, so an inbound handshake is never cancelled
//!   per peer  ── one serve task (`Server::handle`) + one control task (gossip,
//!                `InventoryChanged`, `sync_inventory`)
//! nz-lan-hash ── LN4's worker: local SHA-256, gated and niced +10
//! ```
//!
//! Every child is a task of THAT runtime: the driver closes the endpoint (which
//! joins quinn's only task and closes every connection), stops the discovery
//! (which joins its daemon), then aborts and drains its children — so nothing
//! borrows a runtime that is gone by the time it is dropped.

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;
use tracing::{debug, warn};

use crate::auth;
use crate::decide::decide as decide_source;
use crate::discovery::{Discovery, DiscoveryEvent, ExternalDiscovery};
use crate::enroll::{self, Enrolled};
use crate::gossip;
use crate::hashing::Hashing;
use crate::index::Index;
use crate::peers::Peers;
use crate::proto;
use crate::serve::{ServeLimits, Server};
use crate::settings;
use crate::store::{Store, StoredPeer};
use crate::transport::{CLOSE_DUPLICATE, Endpoint, Identity, PeerConn, SharedTrust, TrustState};
use crate::types::{DevicePub, LanError, Relation, Source, Tolerance, Urgency};

/// What the app gives `Lan` once, at login.
#[derive(Debug, Clone)]
pub struct LanConfig {
    /// `$data/neezer/lan/` on the device: `device.key` and `lan.sqlite` live
    /// there, and nothing else does.
    pub data_dir: PathBuf,
    /// The device's display name, as the person set it (1–64 characters). It only
    /// ever travels inside an authenticated connection.
    pub device_name: String,
    /// `"desktop"` or `"android"` — what the peer list shows and what the server
    /// stores.
    pub platform: &'static str,
    /// The app's version, exchanged in `Hello` so a peer with a newer protocol can
    /// say « mettez à jour l'app sur <appareil> ».
    pub app_version: String,
    /// Where discovery comes from: this crate's own mDNS, or the platform's feed.
    pub discovery: DiscoveryKind,
    /// The UDP port to bind, `0` = any. Default [`DEFAULT_PORT`].
    pub port: u16,
}

/// Where discovery comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveryKind {
    /// `mdns-sd` browses and registers on this device (the desktop, feature
    /// `mdns`).
    Mdns,
    /// The platform browses and calls [`Lan::found`] / [`Lan::lost`]: Android's
    /// `NsdManager` today, iOS's `NWBrowser` later.
    External,
}

/// The default QUIC port: what the firewall line in Réglages names
/// (`sudo ufw allow 45910/udp`).
pub const DEFAULT_PORT: u16 = 45910;

/// How long discovery, announcement and the connections survive after the app
/// stopped being in the foreground, playing or downloading.
///
/// The note's 60 s (*Discovery* → *When*): long enough that a track transition or
/// a screen switch never tears the LAN down, short enough that a phone left on a
/// table stops talking. Nothing schedules a wake-up for the LAN.
pub const SLEEP_GRACE: Duration = Duration::from_secs(60);

/// How often the local inventory is re-read while the node is active.
///
/// `nz-cache` has no "something changed" event, so this is a timer; five seconds
/// costs 0.5 % of one core for an 8 000-track library (LN4 measured a refresh at
/// 23.8 ms release) and a download shows up on the other devices within five
/// seconds.
pub const REFRESH_EVERY: Duration = Duration::from_secs(5);

/// The daily server round trip: renewal and the conditional roster sync.
pub const ENROL_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// How long a node that refused us, or that could not be reached, is left alone.
///
/// Per announcement instance (the random name), never per address: a flapping
/// Wi-Fi must not become a dial storm (rule 3).
pub const DIAL_BACKOFF: Duration = Duration::from_secs(600);

/// At most this many announcement instances are remembered, so a flooded segment
/// costs a fixed amount of memory (every count from the wire is capped).
pub const MAX_DIAL_ENTRIES: usize = 64;

/// The shortest gap between two dials of the SAME instance.
///
/// A peer that accepts and closes at once (a bug, or a hostile node) would
/// otherwise be redialled on every close — a handshake loop, exactly what rule 3
/// forbids. Two seconds is one `IDLE_TIMEOUT`'s worth of slack and a rounding
/// error beside the ten-minute silence a refusal gets.
pub const MIN_REDIAL: Duration = Duration::from_secs(2);

/// Recent failures the snapshot carries, in French (rule 2: a count alone is a
/// bug, a cause is what a person can act on).
pub const MAX_FAILURES: usize = 8;

/// The niceness of the `nz-lan` threads: serving and syncing are background work
/// for this device (rule 1). The hashing worker is niced further, +10.
pub const LAN_NICE: i32 = 5;

/// How often quinn's own counters are read into [`WireCounts`] while a peer is
/// connected (never when none is: an idle node has no timer at all).
const SAMPLE_EVERY: Duration = Duration::from_secs(1);

/// How many track ids « Sur vos appareils » may weigh per call before a set's
/// size is left at `0`: a screen refresh must not walk a library. 50 000 ids is
/// a hundred 50-track playlists, each answered by one binary search per peer.
const SET_SIZE_BUDGET: usize = 50_000;

/// The longest an enrolment (or a renewal) may take on the server before the
/// driver gives up on the try: a LAN node must not hold a task for minutes.
const ENROL_DEADLINE: Duration = Duration::from_secs(30);

/// The most transfer rows the snapshot carries at once (a peer can open 64
/// streams; a registry that grew with it would be a way to spend this device's
/// memory from the wire).
const MAX_TRANSFERS: usize = 32;

/// How many stored peer addresses a wake-up dials, at most.
const MAX_KNOWN_ADDRS: usize = 16;

/// What the app tells `Lan` about itself, so the battery rules can hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    /// The app is in the foreground: screens are open and can show peers.
    Foreground,
    /// Something is playing: discovery and the connection stay up even with the
    /// screen off (that is what a phone in a pocket needs).
    Playing,
    /// The downloads worker has work: a playlist is being taken from a peer.
    Downloading,
}

impl Activity {
    /// The slot of this activity in the counted flags.
    fn slot(self) -> usize {
        match self {
            Activity::Foreground => 0,
            Activity::Playing => 1,
            Activity::Downloading => 2,
        }
    }
}

/// Whether a transfer is going out of this device or coming in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// This device serves it to a peer.
    Send,
    /// This device takes it from a peer.
    Recv,
}

/// The state a screen shows, in one word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanState {
    /// The person switched it off (`lan.enabled`), or the admin did.
    Off,
    /// This server has no such feature: no thread runs and no screen mentions it.
    Unsupported,
    /// No credential yet (the enrolment failed, or the server is unreachable on a
    /// device that never enrolled).
    NotEnrolled,
    /// Enrolled, looking for peers.
    Searching,
    /// Enrolled, with this many authenticated peers.
    Connected(u32),
}

/// One authenticated peer, as the peer list draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerView {
    pub device_pub: DevicePub,
    pub name: String,
    pub relation: Relation,
    pub platform: String,
    /// Smoothed round trip: the "latency" the list shows beside « connecté ».
    pub rtt_ms: u32,
    /// How many tracks its inventory holds, in scope for us.
    pub tracks: u32,
    /// How many download sets it exposes.
    pub sets: u32,
}

/// One transfer in flight, as the peer list shows "what is being exchanged".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferView {
    pub track: String,
    /// The title, when the metadata is already known — the row never blocks on a
    /// request to be drawn.
    pub title: String,
    /// The peer's display name.
    pub peer: String,
    pub direction: Direction,
    pub bytes: u64,
    pub total: u64,
    pub urgency: Urgency,
}

/// One set a peer exposes (« Sur vos appareils »), with its size on that peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerSet {
    pub peer: DevicePub,
    pub peer_name: String,
    pub relation: Relation,
    pub set: proto::SetInfo,
    /// The bytes these tracks take on the peer, when it announced them.
    pub bytes: u64,
}

/// What the screens read in one clone (rule: a snapshot, never a lock held across
/// a read of the database).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanSnapshot {
    pub state: LanState,
    /// When the credential ends (unix seconds), for « Clé valable jusqu'au … ».
    pub key_until: Option<i64>,
    /// Why the last renewal failed, in French, when it did.
    pub key_error: Option<String>,
    pub peers: Vec<PeerView>,
    pub transfers: Vec<TransferView>,
    /// Recent failures `(what, why)` in French — a count alone is a bug (rule 2).
    pub failures: Vec<(String, String)>,
    /// The UDP port actually bound.
    pub port: u16,
    /// Whether the preferred port was taken and an ephemeral one is in use: the
    /// person must know, because the firewall line then does not match.
    pub port_fallback: bool,
}

impl Default for LanSnapshot {
    fn default() -> Self {
        LanSnapshot {
            state: LanState::Off,
            key_until: None,
            key_error: None,
            peers: Vec::new(),
            transfers: Vec::new(),
            failures: Vec::new(),
            port: DEFAULT_PORT,
            port_fallback: false,
        }
    }
}

/// What this node really put on the socket, counted (the battery measure).
///
/// The numbers come from quinn's own per-connection counters, sampled by the
/// driver while a connection lives (never on the UI thread: `Connection::stats`
/// locks the connection's state). LN10's acceptance reads them for "0 packets are
/// sent afterwards"; a Réglages line may print them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WireCounts {
    /// QUIC datagrams this device sent, every peer together.
    pub packets_out: u64,
    /// QUIC datagrams it received.
    pub packets_in: u64,
    /// UDP payload bytes sent.
    pub bytes_out: u64,
    /// UDP payload bytes received.
    pub bytes_in: u64,
    /// Dials this node started: what a refused node must not make grow.
    pub dials: u64,
}

/// The per-device settings (`Réglages` → « Appareils à proximité »).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanSettings {
    /// « Échanger avec mes appareils à proximité ».
    pub enabled: bool,
    /// « Partager mes sons » : whether this device serves what it has.
    pub serve: bool,
    /// « Qualités tolérées » : what may be taken from a peer while the server is
    /// reachable. Never empty in the UI, and Opus 64 is deliberately not a default.
    pub tolerated: Tolerance,
    /// The UDP port, `0` = any.
    pub port: u16,
}

impl Default for LanSettings {
    fn default() -> Self {
        LanSettings {
            enabled: true,
            serve: true,
            tolerated: Tolerance::default_set(),
            port: DEFAULT_PORT,
        }
    }
}

/// The node, as the app holds it: one per process, cloned freely.
#[derive(Clone)]
pub struct Lan {
    inner: Arc<Shared>,
}

impl std::fmt::Debug for Lan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never a credential, a key or a roster: the counted state only.
        formatter
            .debug_struct("Lan")
            .field("state", &self.snapshot_state())
            .field("peers", &self.inner.peers.len())
            .finish()
    }
}

// --- the node's pieces -------------------------------------------------------

/// The whole counted and published state, shared by the UI thread, the driver and
/// the per-connection tasks.
struct Shared {
    dir: PathBuf,
    name: String,
    platform: &'static str,
    app_version: String,
    api: Arc<nz_api::Api>,
    cache: nz_cache::Cache,
    store: Store,
    index: Index,
    peers: Peers,
    /// The peers' download sets, replicated by `sets.rs` (LN11). « Sur vos
    /// appareils » and [`Lan::sets`] read it; nothing on the UI thread asks a
    /// peer for it.
    sets: crate::sets::Replica,
    /// The `nz-lan` runtime's handle, so the adapters (`opener`, `downloader`,
    /// `sets`, `import`) can run a peer request from their own thread — the
    /// engine's decoder thread, the offline worker's. `None` before the runtime
    /// exists and `None` again once `shutdown` joined it: a `block_on` on a dead
    /// runtime panics, and an adapter must fall back, not crash.
    handle: Mutex<Option<tokio::runtime::Handle>>,
    server: Server,
    trust: SharedTrust,
    /// The live connections, so a sleep (or a shutdown) can close them and the
    /// wire sampler can read quinn's counters. Bounded by `MAX_CONNECTIONS`.
    conns: Mutex<HashMap<DevicePub, PeerConn>>,
    /// What the screens draw: written by the driver, read by `snapshot`.
    view: Mutex<View>,
    /// The version counter, and its integer.
    version: AtomicU64,
    events: watch::Sender<u64>,
    /// Counted wire traffic (atomics: read from the UI thread).
    wire: Wire,
    /// The last counters read per connection, so the total only ever grows.
    wire_seen: Mutex<HashMap<usize, WireCounts>>,
    /// `lan.enabled`, `lan.serve`, `lan.tolerated`, `lan.port`.
    settings: Mutex<LanSettings>,
    /// Foreground / Playing / Downloading, as the app set them.
    activity: [AtomicBool; 3],
    activity_version: AtomicU64,
    activity_events: watch::Sender<u64>,
    /// How long the node stays up after the app stopped being active.
    sleep_grace_ms: AtomicU64,
    /// The announcement instances this node must not dial (yet).
    dials: Mutex<DialTable>,
    /// The transfers in flight (what the peer list shows being exchanged).
    transfers: Mutex<Transfers>,
    /// The discovery, so `found`/`lost` reach it without the driver.
    feed: RwLock<Option<Arc<Feed>>>,
    /// The hashing worker (its own thread and runtime; joined by `shutdown`).
    hashing: Mutex<Option<Hashing>>,
    /// The runtime's stop signal and thread.
    engine: Mutex<Option<Engine>>,
    /// The commands the sync API posts to the driver.
    commands: mpsc::UnboundedSender<Command>,
    /// What a child task reports to the driver. The driver keeps the receiving
    /// half, so a child holds a sender and never the receiver.
    internal: mpsc::UnboundedSender<Internal>,
    /// Shutdown runs once, whichever clone ends up last.
    stopped: AtomicBool,
}

/// The published, short-lock state a screen reads.
#[derive(Debug, Clone, PartialEq, Eq)]
struct View {
    state: LanState,
    key_until: Option<i64>,
    key_error: Option<String>,
    failures: VecDeque<(String, String)>,
    port: u16,
    port_fallback: bool,
}

impl Default for View {
    fn default() -> Self {
        View {
            state: LanState::Searching,
            key_until: None,
            key_error: None,
            failures: VecDeque::new(),
            port: DEFAULT_PORT,
            port_fallback: false,
        }
    }
}

#[derive(Debug, Default)]
struct Wire {
    packets_out: AtomicU64,
    packets_in: AtomicU64,
    bytes_out: AtomicU64,
    bytes_in: AtomicU64,
    dials: AtomicU64,
}

impl Wire {
    fn read(&self) -> WireCounts {
        WireCounts {
            packets_out: self.packets_out.load(Ordering::Relaxed),
            packets_in: self.packets_in.load(Ordering::Relaxed),
            bytes_out: self.bytes_out.load(Ordering::Relaxed),
            bytes_in: self.bytes_in.load(Ordering::Relaxed),
            dials: self.dials.load(Ordering::Relaxed),
        }
    }
}

/// One announcement instance, and what this node decided about it.
#[derive(Debug)]
struct DialEntry {
    addr: SocketAddr,
    /// When a backoff ends, or `None` when the instance may be dialled.
    retry_at: Option<tokio::time::Instant>,
    /// When this node last dialled it, for [`MIN_REDIAL`].
    dialled_at: Option<tokio::time::Instant>,
}

/// The instances this node knows, bounded (a hostile segment floods).
#[derive(Debug, Default)]
struct DialTable {
    entries: HashMap<String, DialEntry>,
    /// Insertion order, so the oldest entry is the one forgotten.
    order: VecDeque<String>,
}

impl DialTable {
    /// Whether `instance` may be dialled now, remembering it when it is new.
    fn allow(&mut self, instance: &str, addr: SocketAddr, now: tokio::time::Instant) -> bool {
        if let Some(entry) = self.entries.get_mut(instance) {
            entry.addr = addr;
            let backing_off = entry.retry_at.is_some_and(|at| at > now);
            let too_soon = entry
                .dialled_at
                .is_some_and(|at| now.duration_since(at) < MIN_REDIAL);
            return !backing_off && !too_soon;
        }
        if self.entries.len() >= MAX_DIAL_ENTRIES
            && let Some(oldest) = self.order.pop_front()
        {
            self.entries.remove(&oldest);
        }
        self.entries.insert(
            instance.to_owned(),
            DialEntry {
                addr,
                retry_at: None,
                dialled_at: None,
            },
        );
        self.order.push_back(instance.to_owned());
        true
    }

    /// The instance answered: a later announcement may re-dial it.
    fn up(&mut self, instance: &str, addr: SocketAddr, now: tokio::time::Instant) {
        self.allow(instance, addr, now);
        if let Some(entry) = self.entries.get_mut(instance) {
            entry.retry_at = None;
        }
    }

    /// The instance refused us, or did not answer: silence for [`DIAL_BACKOFF`].
    ///
    /// `now` is the caller's instant, never read here: a test drives the boundary
    /// with the same clock the decision reads, instead of guessing at microseconds.
    fn back_off(&mut self, instance: &str, addr: SocketAddr, now: tokio::time::Instant) {
        self.allow(instance, addr, now);
        if let Some(entry) = self.entries.get_mut(instance) {
            entry.retry_at = Some(now + DIAL_BACKOFF);
        }
    }

    fn counted(&self) -> usize {
        self.entries
            .keys()
            .map(|instance| instance.len() + std::mem::size_of::<DialEntry>())
            .sum()
    }
}

/// The transfers in flight, bounded.
#[derive(Debug, Default)]
struct Transfers {
    rows: Vec<(u64, TransferView)>,
    next: u64,
}

impl Transfers {
    fn begin(&mut self, row: TransferView) -> u64 {
        self.next += 1;
        let id = self.next;
        if self.rows.len() < MAX_TRANSFERS {
            self.rows.push((id, row));
        }
        id
    }

    fn end(&mut self, id: u64, bytes: u64) {
        // The row leaves when the transfer does: the peer list says "en cours",
        // and the count of bytes written is the serve's own answer (the responder
        // never blocks its loop to count 64 KiB chunks).
        let _ = bytes;
        self.rows.retain(|(row, _)| *row != id);
    }

    fn rows(&self) -> Vec<TransferView> {
        self.rows.iter().map(|(_, row)| row.clone()).collect()
    }

    fn counted(&self) -> usize {
        self.rows
            .iter()
            .map(|(_, row)| {
                row.track.len()
                    + row.title.len()
                    + row.peer.len()
                    + std::mem::size_of::<TransferView>()
            })
            .sum()
    }
}

/// The runtime's thread, owned so that the runtime it borrows cannot go first.
struct Engine {
    stop: watch::Sender<bool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Engine {
    /// Signals the driver and JOINS its thread.
    fn stop(&mut self) {
        let _ = self.stop.send(true);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// What the sync API asks the driver to do.
#[derive(Debug)]
enum Command {
    EnrollNow,
    Forget,
    NetworkChanged,
    Settings(Box<LanSettings>),
}

/// What a child task tells the driver.
#[derive(Debug)]
enum Internal {
    Enrolled(Box<Enrolled>),
    EnrolFailed(Box<LanError>),
    Accepted(Box<PeerConn>),
    AcceptFailed(Box<LanError>),
    Dialed {
        instance: String,
        addr: SocketAddr,
        result: Box<Result<PeerConn, LanError>>,
    },
    Closed {
        peer: DevicePub,
        connection: usize,
        /// Whether the transport closed it because it COLLAPSED a pair (both
        /// devices dialled each other): the same device is still here, on the
        /// survivor, so its replicas must not be forgotten. LN15 measured the cost
        /// of forgetting them: two of twenty netns runs lost the inventory for one
        /// `Bulk` window, and a download decided inside it fell back to a server
        /// that was not there.
        duplicate: bool,
    },
    Note(String, Box<LanError>),
}

// --- the discovery, whichever implementation ---------------------------------

/// The discovery this node runs.
///
/// One enum rather than a `dyn Discovery` plus a downcast: `found`/`lost` only
/// exist on the platform feed, and the call must be a typed one.
enum Feed {
    External(Arc<ExternalDiscovery>),
    #[cfg(feature = "mdns")]
    Mdns(Arc<crate::discovery_mdns::MdnsDiscovery>),
}

impl std::fmt::Debug for Feed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Feed::External(_) => "Feed::External",
            #[cfg(feature = "mdns")]
            Feed::Mdns(_) => "Feed::Mdns",
        })
    }
}

impl Feed {
    fn new(kind: DiscoveryKind) -> Result<Feed, LanError> {
        match kind {
            DiscoveryKind::External => Ok(Feed::External(Arc::new(ExternalDiscovery::new()))),
            #[cfg(feature = "mdns")]
            DiscoveryKind::Mdns => Ok(Feed::Mdns(Arc::new(
                crate::discovery_mdns::MdnsDiscovery::new(false)?,
            ))),
            #[cfg(not(feature = "mdns"))]
            DiscoveryKind::Mdns => Err(LanError::Internal(
                "cette compilation n'a pas la découverte mDNS".to_owned(),
            )),
        }
    }

    fn as_discovery(&self) -> &dyn Discovery {
        match self {
            Feed::External(feed) => feed.as_ref(),
            #[cfg(feature = "mdns")]
            Feed::Mdns(feed) => feed.as_ref(),
        }
    }

    /// The platform resolved a node: only the platform's feed has this door.
    fn found(&self, ip: IpAddr, port: u16) {
        match self {
            Feed::External(feed) => feed.found(ip, port),
            #[cfg(feature = "mdns")]
            Feed::Mdns(_) => {}
        }
    }

    fn lost(&self, ip: IpAddr, port: u16) {
        match self {
            Feed::External(feed) => feed.lost(ip, port),
            #[cfg(feature = "mdns")]
            Feed::Mdns(_) => {}
        }
    }
}

// --- the handle --------------------------------------------------------------

impl Lan {
    /// Starts the node for `api`'s account, with `cache` as its local library.
    ///
    /// `Err(LanError::Unsupported)` when the server has no such feature, and then
    /// nothing was started — no runtime, no thread, no socket, and the data
    /// directory was not even created.
    pub fn start(
        cfg: LanConfig,
        api: Arc<nz_api::Api>,
        cache: nz_cache::Cache,
    ) -> Result<Lan, LanError> {
        // The feature probe comes FIRST: rule 7's "an older server hides the
        // feature" has to be true before a directory or a thread exists.
        if probe_feature(&api) == Some(false) {
            debug!("LAN : ce serveur n'a pas la route /api/lan/status, rien ne démarre");
            return Err(LanError::Unsupported);
        }
        // `cfg.port` is the port to bind, `0` = any. The app normally passes
        // `settings().port` (what Réglages shows and persists); a caller that names
        // another one overrides it for this session, without rewriting the setting.
        let mut settings = settings::load(&cache);
        if cfg.port != settings.port {
            settings.port = cfg.port;
        }
        let store = Store::open(&cfg.data_dir)?;
        let enabled = settings.enabled;
        let port = settings.port;
        let owner = api.owner().unwrap_or_default();
        let index = Index::new(cache.clone(), owner, store.clone());
        let peers = Peers::new();
        let server = Server::new(index.clone(), api.gates(), ServeLimits::default());
        let trust: SharedTrust = Arc::new(RwLock::new(TrustState::default()));
        let (events, _) = watch::channel(0);
        let (commands, command_rx) = mpsc::unbounded_channel();
        let (internal, internal_rx) = mpsc::unbounded_channel();
        let (activity_events, activity_rx) = watch::channel(0);
        // The feed is created NOW and started only once the node is enrolled: a
        // platform callback that arrives before that is dropped, not queued.
        let feed = Arc::new(Feed::new(cfg.discovery)?);
        let discovery_events = feed.as_discovery().events();
        let view = View {
            state: if enabled {
                LanState::NotEnrolled
            } else {
                LanState::Off
            },
            port,
            ..View::default()
        };
        let shared = Arc::new(Shared {
            dir: cfg.data_dir,
            name: cfg.device_name,
            platform: cfg.platform,
            app_version: cfg.app_version,
            api,
            cache,
            store,
            index,
            peers,
            server,
            trust,
            sets: crate::sets::Replica::new(),
            handle: Mutex::new(None),
            conns: Mutex::new(HashMap::new()),
            view: Mutex::new(view),
            version: AtomicU64::new(0),
            events,
            wire: Wire::default(),
            wire_seen: Mutex::new(HashMap::new()),
            settings: Mutex::new(settings),
            // The app calls `start` at login, with its window open: the node is in
            // the foreground until the app says otherwise (one `set_active`).
            activity: [
                AtomicBool::new(true),
                AtomicBool::new(false),
                AtomicBool::new(false),
            ],
            activity_version: AtomicU64::new(1),
            activity_events,
            sleep_grace_ms: AtomicU64::new(SLEEP_GRACE.as_millis() as u64),
            dials: Mutex::new(DialTable::default()),
            transfers: Mutex::new(Transfers::default()),
            feed: RwLock::new(Some(Arc::clone(&feed))),
            hashing: Mutex::new(None),
            engine: Mutex::new(None),
            commands,
            internal,
            stopped: AtomicBool::new(false),
        });

        let (stop, stop_rx) = watch::channel(false);
        let thread = {
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("nz-lan".to_owned())
                .spawn(move || {
                    lower_priority(LAN_NICE);
                    let runtime = match tokio::runtime::Builder::new_multi_thread()
                        .worker_threads(1)
                        .thread_name("nz-lan")
                        .on_thread_start(|| lower_priority(LAN_NICE))
                        .enable_all()
                        .build()
                    {
                        Ok(runtime) => runtime,
                        Err(error) => {
                            warn!(%error, "nz-lan : le runtime n'a pas démarré");
                            return;
                        }
                    };
                    // The driver joins every child before it returns, so this
                    // runtime borrows nothing when it is dropped.
                    // LN11's adapters run a peer request from the engine's and the
                    // offline worker's OWN threads: the handle is published before
                    // the driver starts, and cleared before the runtime stops (see
                    // `Shared::stop`), so a `block_on` on it is never a dead one.
                    *shared
                        .handle
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) =
                        Some(runtime.handle().clone());
                    runtime.block_on(driver(
                        shared,
                        stop_rx,
                        command_rx,
                        activity_rx,
                        discovery_events,
                        internal_rx,
                        feed,
                    ));
                    drop(runtime);
                    debug!("nz-lan : le runtime est arrêté");
                })
                .map_err(|error| LanError::Internal(format!("thread nz-lan : {error}")))?
        };
        *shared
            .engine
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(Engine {
            stop,
            thread: Some(thread),
        });
        Ok(Lan { inner: shared })
    }

    /// Tells the node what the app is doing; `on = false` means "that stopped".
    ///
    /// When none of the three is on, connections close and discovery stops after
    /// [`SLEEP_GRACE`], and no packet is sent afterwards.
    pub fn set_active(&self, why: Activity, on: bool) {
        let slot = why.slot();
        let previous = self.inner.activity[slot].swap(on, Ordering::AcqRel);
        if previous != on {
            self.inner.nudge_activity();
        }
    }

    /// The platform resolved a node at `host`:`port` ([`DiscoveryKind::External`]).
    ///
    /// A callback that lands while the node is asleep is dropped by the feed, which
    /// is what keeps the battery rule true (nothing dials with the screen off).
    pub fn found(&self, host: IpAddr, port: u16) {
        if let Some(feed) = self.inner.feed_now() {
            feed.found(host, port);
        }
    }

    /// The platform lost a node it had resolved.
    pub fn lost(&self, host: IpAddr, port: u16) {
        if let Some(feed) = self.inner.feed_now() {
            feed.lost(host, port);
        }
    }

    /// The interface list or the network changed: re-announce, and try the
    /// last-known addresses of the peers we had.
    pub fn network_changed(&self) {
        self.inner.send(Command::NetworkChanged);
    }

    /// What the screens draw, cloned from memory: never a lock across SQLite, never
    /// a wait on the runtime.
    pub fn snapshot(&self) -> LanSnapshot {
        // One short lock and the counted replicas. No database, no runtime, no
        // await: this runs on the UI thread (the source scan in `tests/service.rs`
        // keeps it that way).
        let view = self
            .inner
            .view
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // Only the devices this node HOLDS a connection to. A replica outlives a
        // close on purpose (a duplicate collapse keeps it for the survivor, and a
        // reconnect syncs it as a delta instead of ~480 KB of full pages), so the
        // peer list reads the connection map and not the replica's keys: a screen
        // never draws a device the node is not talking to (LN15).
        let peers: Vec<PeerView> = {
            let conns = self
                .inner
                .conns
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            self.inner
                .peers
                .views()
                .into_iter()
                .filter(|peer| conns.contains_key(&peer.device_pub))
                .collect()
        };
        let state = match view.state {
            LanState::Off | LanState::Unsupported | LanState::NotEnrolled => view.state,
            _ => {
                let connected = u32::try_from(peers.len()).unwrap_or(u32::MAX);
                if connected == 0 {
                    LanState::Searching
                } else {
                    LanState::Connected(connected)
                }
            }
        };
        LanSnapshot {
            state,
            key_until: view.key_until,
            key_error: view.key_error.clone(),
            peers,
            transfers: self
                .inner
                .transfers
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .rows(),
            failures: view.failures.iter().cloned().collect(),
            port: view.port,
            port_fallback: view.port_fallback,
        }
    }

    /// The version counter, for a screen that must redraw only on change.
    pub fn events(&self) -> watch::Receiver<u64> {
        self.inner.events.subscribe()
    }

    /// What this node really put on the socket, counted (the battery measure).
    pub fn wire(&self) -> WireCounts {
        self.inner.wire.read()
    }

    /// Where `track` will come from, from memory: no request, no waiting.
    ///
    /// LN9's six-row table over the local index, the replicated inventories, this
    /// device's tolerance and `nz-api`'s own reachability verdict.
    pub fn decide(&self, track: &str, wanted: nz_api::Quality, urgency: Urgency) -> Source {
        // The urgency belongs to the caller (it becomes the request's `Urgency`
        // when the transfer starts, LN11); the decision itself does not depend on
        // it — the same peer serves a tap and a download.
        let _ = urgency;
        let settings = self.settings();
        let local = self.inner.index.best_local(track).map(|file| file.path);
        let candidates = live_candidates(&self.inner.peers, track);
        decide_source(
            local,
            &candidates,
            wanted,
            &settings.tolerated,
            self.inner.api.is_online(),
            None,
        )
    }

    /// The same decision, with the peer a playlist download is COMMITTED to
    /// (`download_peer_sets`): its tracks all come from that one device instead
    /// of scattering across the LAN (`docs/agents/lan-nodes.md`, *Choosing the
    /// source*, row 3's `prefer`).
    ///
    /// Additive to LN0's surface: the downloader is the only caller, and the
    /// decision itself stays in [`decide_source`] — there is no second table.
    pub(crate) fn decide_download(&self, track: &str, wanted: nz_api::Quality) -> Source {
        let settings = self.settings();
        let local = self.inner.index.best_local(track).map(|file| file.path);
        let candidates = live_candidates(&self.inner.peers, track);
        decide_source(
            local,
            &candidates,
            wanted,
            &settings.tolerated,
            self.inner.api.is_online(),
            self.inner.sets.committed(track),
        )
    }

    // --- the doors the adapters use (LN11, additive and `pub(crate)` only) -----
    //
    // The four adapters live in sibling modules of this one, and `Shared`'s
    // fields are private to `service`: without these, `PeerFirstOpener` and
    // `PeerFirstDownloader` could not open a single transfer. They are reads of
    // state this module already owns, except `mark_bad`, which is the peer
    // table's own verdict on a file that failed its SHA-256 or its decoder probe.

    /// The `nz-lan` runtime, for a request started on the engine's or the offline
    /// worker's thread. `None` before the runtime exists and after `shutdown`
    /// joined it.
    pub(crate) fn handle(&self) -> Option<tokio::runtime::Handle> {
        self.inner
            .handle
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// The live connection to `peer`, if there is one. `None` is not an error:
    /// the caller takes the server, which is what rule 3 asks of a failed LAN
    /// path.
    pub(crate) fn conn(&self, peer: &DevicePub) -> Option<PeerConn> {
        self.inner
            .conns
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(peer)
            .cloned()
    }

    /// The client this node shares with its app: the ONE `Gates` a `Bulk` transfer
    /// takes (server and LAN), and `is_online` for the decision.
    pub(crate) fn api(&self) -> Arc<nz_api::Api> {
        Arc::clone(&self.inner.api)
    }

    /// This device's caches, for `import.rs` (covers pinned, metadata read).
    pub(crate) fn cache(&self) -> &nz_cache::Cache {
        &self.inner.cache
    }

    /// The length a peer announced for one copy, from the replica: what
    /// `StreamOpener::ready` answers **without a request**.
    pub(crate) fn peer_len(&self, track: &str, quality: nz_api::Quality) -> Option<u64> {
        self.inner
            .peers
            .candidates(track)
            .into_iter()
            .find(|candidate| candidate.has(quality))
            .and_then(|candidate| {
                candidate
                    .copies
                    .iter()
                    .find(|(offered, _)| *offered == quality)
                    .map(|(_, len)| *len)
            })
    }

    /// A peer answered a wrong file (a bad SHA-256, a decoder probe that refused
    /// it): out of this track's next decisions, for this session.
    pub(crate) fn mark_bad(&self, peer: &DevicePub, track: &str) {
        self.inner.peers.mark_bad(peer, track);
        self.inner.touch();
    }

    /// One set of one peer, from the sets replica: what « Télécharger » re-checks
    /// against, with **0** requests.
    pub(crate) fn peer_set(
        &self,
        peer: &DevicePub,
        kind: &str,
        ident: &str,
    ) -> Option<proto::SetInfo> {
        self.inner.sets.find(peer, kind, ident)
    }

    /// The tracks of a set are being taken from `peer`: every later decision for
    /// them prefers that device (`decide_download`).
    pub(crate) fn commit_peer_tracks(&self, peer: &DevicePub, tracks: &[String]) {
        self.inner.sets.commit(peer, tracks);
    }

    /// Every set the connected peers expose, for « Sur vos appareils ».
    ///
    /// The replica is `sets.rs`'s, filled by the control loop on every connection
    /// (and on every `SetsChanged`), so this call is a short lock and no request.
    /// The sizes come from the inventory replica, within a fixed budget: a
    /// library with hundreds of sets must not make a screen walk millions of
    /// tracks.
    pub fn sets(&self) -> Vec<PeerSet> {
        let mut rows = self.inner.sets.rows();
        let mut budget: usize = SET_SIZE_BUDGET;
        for row in &mut rows {
            if row.set.tracks.len() > budget {
                continue;
            }
            budget -= row.set.tracks.len();
            row.bytes = self.peer_bytes(&row.peer, &row.set.tracks);
        }
        rows
    }

    /// What `peer`'s copies of `tracks` weigh, read from the replicated inventory:
    /// the best copy of each, summed.
    fn peer_bytes(&self, peer: &DevicePub, tracks: &[String]) -> u64 {
        let mut total = 0u64;
        for track in tracks {
            let candidates = self.inner.peers.candidates(track);
            let Some(candidate) = candidates.iter().find(|candidate| candidate.peer == *peer)
            else {
                continue;
            };
            if let Some((_, len)) = candidate.copies.iter().max_by_key(|(quality, _)| *quality) {
                total += len;
            }
        }
        total
    }

    /// The peer that has ALL of `tracks` complete, if one does: a playlist
    /// download then asks that device for every track.
    pub fn best_peer_for(&self, tracks: &[String]) -> Option<DevicePub> {
        self.inner.peers.complete_for(tracks)
    }

    /// The settings as they are now.
    pub fn settings(&self) -> LanSettings {
        self.inner.settings_now()
    }

    /// Writes the settings. Applied at once for the session, persisted by
    /// [`crate::settings::save`].
    pub fn set_settings(&self, s: LanSettings) {
        self.inner.send(Command::Settings(Box::new(s)));
    }

    /// How long the node keeps discovery and its connections after the app stopped
    /// being active.
    ///
    /// Additive to LN0's surface, and the reason is a test's: a test cannot pause
    /// the clock of a runtime it does not own, and waiting the real minute would
    /// make the suite pay a minute per run. The app keeps [`SLEEP_GRACE`].
    pub fn set_sleep_grace(&self, grace: Duration) {
        let millis = grace.as_millis().min(u128::from(u64::MAX)) as u64;
        self.inner.sleep_grace_ms.store(millis, Ordering::Relaxed);
        self.inner.nudge_activity();
    }

    /// Enrols (or renews) now instead of waiting for the daily run: the app calls
    /// this after a login.
    pub fn enroll_now(&self) {
        self.inner.send(Command::EnrollNow);
    }

    /// Logs out: wipes the key, the credential and the roster, and tells the server
    /// when it is reachable.
    pub fn forget(&self) {
        self.inner.send(Command::Forget);
    }

    /// The bytes this node's state costs, counted (index + replicas + the counted
    /// buffers). LN10's targets: ≤ 64 B per replicated item, flat over ten thousand
    /// inventory deltas.
    pub fn memory(&self) -> usize {
        self.inner.index.memory()
            + self.inner.peers.memory()
            + self.inner.sets.memory()
            + counted_lock(&self.inner.conns, |conns| conns.len() * 64)
            + counted_lock(&self.inner.dials, DialTable::counted)
            + counted_lock(&self.inner.transfers, Transfers::counted)
            + self.inner.view_counted()
            + self.inner.trust_counted()
    }

    /// Stops everything and JOINS: the `nz-lan` runtime, the hashing thread and the
    /// discovery daemon. Called after the engine and the offline worker, and before
    /// `nz-net` stops.
    ///
    /// Idempotent and safe to call twice. Never call it from a task of the `nz-lan`
    /// runtime: it joins that runtime's thread.
    pub fn shutdown(&self) {
        self.inner.stop();
    }

    fn snapshot_state(&self) -> LanState {
        self.inner
            .view
            .lock()
            .map(|view| view.state)
            .unwrap_or(LanState::Off)
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Shared {
    fn feed_now(&self) -> Option<Arc<Feed>> {
        self.feed
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// Posts one command; an unbounded send never blocks a caller, so
    /// `set_settings`, `enroll_now` and `network_changed` are safe on the UI thread.
    fn send(&self, command: Command) {
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        if self.commands.send(command).is_err() {
            debug!("nz-lan : le pilote n'écoute plus");
        }
    }

    fn nudge_activity(&self) {
        let version = self.activity_version.fetch_add(1, Ordering::AcqRel) + 1;
        self.activity_events.send_replace(version);
    }

    fn active(&self) -> bool {
        self.activity
            .iter()
            .any(|flag| flag.load(Ordering::Relaxed))
    }

    fn sleep_grace(&self) -> Duration {
        Duration::from_millis(self.sleep_grace_ms.load(Ordering::Relaxed))
    }

    /// Publishes a change: mutate the view, bump the version, wake the screens —
    /// and nothing at all when nothing changed.
    fn publish(&self, change: impl FnOnce(&mut View)) {
        let mut view = self.view.lock().unwrap_or_else(|error| error.into_inner());
        let before = view.clone();
        change(&mut view);
        if *view == before {
            return;
        }
        let version = self.version.fetch_add(1, Ordering::AcqRel) + 1;
        drop(view);
        // `send_replace` stores AND wakes: `send` loses the value when no screen is
        // subscribed (LN4's lesson), and the next screen to open would then see a
        // stale counter.
        self.events.send_replace(version);
    }

    /// Tells the screens that the snapshot may have changed without changing the
    /// view itself: a renewal (the same second twice) and a peer's replica that
    /// moved are both real events a list redraws for.
    fn touch(&self) {
        let version = self.version.fetch_add(1, Ordering::AcqRel) + 1;
        self.events.send_replace(version);
    }

    /// One French failure line for the snapshot (bounded, newest first).
    fn fail(&self, what: &str, error: &LanError) {
        let what = what.to_owned();
        let why = error.cause_fr();
        self.publish(|view| {
            view.failures.retain(|(kept, _)| kept != &what);
            view.failures.push_front((what, why));
            while view.failures.len() > MAX_FAILURES {
                view.failures.pop_back();
            }
        });
    }

    fn view_counted(&self) -> usize {
        let view = self.view.lock().unwrap_or_else(|error| error.into_inner());
        view.failures
            .iter()
            .map(|(what, why)| what.len() + why.len() + 32)
            .sum::<usize>()
            + view.key_error.as_ref().map(String::len).unwrap_or(0)
    }

    /// The counted trust this device holds: the signed blobs as received (what
    /// gossip relays) plus the server keys.
    fn trust_counted(&self) -> usize {
        let trust = self.trust.read().unwrap_or_else(|error| error.into_inner());
        trust
            .roster
            .as_ref()
            .map(|roster| roster.raw_signed().len())
            .unwrap_or(0)
            + trust
                .revocations
                .values()
                .map(|list| list.raw().len())
                .sum::<usize>()
            + trust.keys.len() * 40
    }

    /// Starts the hashing worker if none runs. The lock lives in here, so no
    /// guard of this crate is ever held across an await point.
    fn ensure_hashing(&self) {
        let mut hashing = self
            .hashing
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if hashing.is_none() {
            *hashing = Some(Hashing::start_with(
                self.index.clone(),
                self.store.clone(),
                self.api.gates(),
            ));
        }
    }

    fn settings_now(&self) -> LanSettings {
        self.settings
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    fn hello(&self) -> proto::Hello {
        proto::Hello {
            name: self.name.clone(),
            platform: self.platform.to_owned(),
            app_version: self.app_version.clone(),
            inventory_version: self.index.version(),
            sets_version: self.index.sets_version(),
        }
    }

    /// The stop sequence: the hashing worker, then the runtime (whose driver closes
    /// the endpoint, stops the discovery and joins its children), then the state a
    /// screen reads afterwards.
    fn stop(&self) {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return;
        }
        // The hashing worker owns its own runtime and thread: join it first, so the
        // `Index`/`Store` it borrowed outlive it.
        let hashing = self
            .hashing
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(hashing) = hashing {
            hashing.stop();
        }
        let engine = self
            .engine
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(mut engine) = engine {
            // The adapters must stop `block_on`ing what is about to be joined:
            // the handle goes BEFORE the runtime's thread, never after (a caller
            // that still holds one falls back to the inner opener instead of
            // panicking inside tokio).
            *self
                .handle
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = None;
            engine.stop();
        }
        self.publish(|view| view.state = LanState::Off);
    }
}

/// One counted field behind a mutex, or `0` when a panic poisoned it (a screen's
/// memory line must never panic).
fn counted_lock<T>(lock: &Mutex<T>, count: impl FnOnce(&T) -> usize) -> usize {
    lock.lock().map(|value| count(&value)).unwrap_or(0)
}

// --- the driver --------------------------------------------------------------

/// The driver's own state: what the runtime's select reads and writes.
struct Driver {
    shared: Arc<Shared>,
    feed: Arc<Feed>,
    endpoint: Option<Arc<Endpoint>>,
    identity: Option<Arc<Identity>>,
    /// Whether the discovery is announcing and browsing right now.
    running: bool,
    /// One enrolment at a time, whatever the app asks.
    enrolment_running: bool,
}

async fn driver(
    shared: Arc<Shared>,
    mut stop: watch::Receiver<bool>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    mut activity: watch::Receiver<u64>,
    mut discovery_events: mpsc::Receiver<DiscoveryEvent>,
    mut internal_rx: mpsc::UnboundedReceiver<Internal>,
    feed: Arc<Feed>,
) {
    let internal = shared.commands_internal();
    let mut tasks: JoinSet<()> = JoinSet::new();
    let mut driver = Driver {
        shared,
        feed,
        endpoint: None,
        identity: None,
        running: false,
        enrolment_running: false,
    };
    // The local inventory first: a peer that connects during the enrolment gets a
    // complete answer, not an empty one.
    driver.shared.index.refresh();
    // A cached credential answers with no request at all, which is the offline
    // start of a trip (rule 3).
    driver.start_or_restore(&mut tasks, &internal).await;

    let mut sleep_at = if driver.active() {
        None
    } else {
        Some(tokio::time::Instant::now() + driver.shared.sleep_grace())
    };
    let mut refresh = ticker(REFRESH_EVERY);
    let mut daily = ticker(ENROL_EVERY);
    let mut sample = ticker(SAMPLE_EVERY);

    loop {
        // Reap finished children outside the `select!`: a future that borrows the
        // `JoinSet` and a handler that spawns into it cannot coexist.
        while let Some(joined) = tasks.try_join_next() {
            if let Err(error) = joined
                && !error.is_cancelled()
            {
                debug!(%error, "nz-lan : une tâche est tombée");
            }
        }
        let awake = driver.active();
        let enrolled = driver.identity.is_some();
        let connected = !driver
            .shared
            .conns
            .lock()
            .map(|conns| conns.is_empty())
            .unwrap_or(true);
        let sleeping = async {
            match sleep_at {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending().await,
            }
        };
        let discovery_event = async {
            match discovery_events.recv().await {
                Some(event) => event,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            biased;
            changed = stop.changed() => {
                let _ = changed;
                if *stop.borrow() {
                    break;
                }
            }
            Some(command) = commands.recv() => {
                driver.command(command, &mut tasks, &internal).await;
                sleep_at = driver.recompute_sleep(sleep_at);
            }
            Some(message) = internal_rx.recv() => {
                driver.handle(message, &mut tasks, &internal).await;
                sleep_at = driver.recompute_sleep(sleep_at);
            }
            changed = activity.changed() => {
                let _ = changed;
                driver.on_activity(&mut tasks, &internal).await;
                sleep_at = driver.recompute_sleep(sleep_at);
            }
            event = discovery_event => {
                driver.on_discovery(event, &mut tasks).await;
            }
            _ = sleeping, if sleep_at.is_some() => {
                sleep_at = None;
                driver.sleep_now().await;
            }
            _ = refresh.tick(), if awake && enrolled => {
                driver.refresh_index().await;
            }
            _ = daily.tick(), if awake && enrolled => {
                driver.enrol(&mut tasks, &internal);
            }
            _ = sample.tick(), if connected => {
                driver.sample_wire();
            }
        }
    }

    driver.finish(&mut tasks).await;
}

impl Driver {
    fn active(&self) -> bool {
        self.shared.active()
    }

    /// When the node must fall asleep, as the `select!`'s deadline.
    fn recompute_sleep(
        &self,
        current: Option<tokio::time::Instant>,
    ) -> Option<tokio::time::Instant> {
        if self.active() {
            return None;
        }
        current.or(Some(
            tokio::time::Instant::now() + self.shared.sleep_grace(),
        ))
    }

    /// The enrolment of a start: a cached credential answers with no request, so a
    /// trip without a server still brings the node up.
    async fn start_or_restore(
        &mut self,
        tasks: &mut JoinSet<()>,
        internal: &mpsc::UnboundedSender<Internal>,
    ) {
        if !self.shared.settings_now().enabled {
            self.shared.publish(|view| view.state = LanState::Off);
            return;
        }
        let now = auth::now_seconds();
        if let Some(enrolled) = enroll::load_cached(&self.shared.dir, &self.shared.store, now) {
            debug!(
                until = enrolled.until,
                "LAN : inscription en cache, aucun appel serveur"
            );
            self.apply_enrolled(enrolled, tasks, internal).await;
            return;
        }
        self.shared
            .publish(|view| view.state = LanState::NotEnrolled);
        if self.active() {
            self.enrol(tasks, internal);
        }
    }

    /// Asks the server for (or renews) the credential, on a child task: an HTTP
    /// round trip must never hold the driver.
    fn enrol(&mut self, tasks: &mut JoinSet<()>, internal: &mpsc::UnboundedSender<Internal>) {
        if self.enrolment_running {
            return;
        }
        self.enrolment_running = true;
        let shared = Arc::clone(&self.shared);
        let internal = internal.clone();
        tasks.spawn(async move {
            let now = auth::now_seconds();
            // A device that already holds a credential syncs first: the roster and
            // the revocation list move between renewals, and one conditional request
            // that answers `304` costs 0 body bytes (the note's "one request,
            // `304`, 0 bytes" per day). `ensure_enrolled` then reads whatever moved.
            if let Some(cached) = enroll::load_cached(&shared.dir, &shared.store, now) {
                {
                    let mut trust = shared
                        .trust
                        .write()
                        .unwrap_or_else(|error| error.into_inner());
                    *trust = cached.trust;
                }
                if let Err(error) =
                    enroll::sync_trust(&shared.api, &shared.store, &shared.trust).await
                {
                    debug!(%error, "LAN : synchronisation de confiance impossible");
                }
            }
            let answer = tokio::time::timeout(
                ENROL_DEADLINE,
                enroll::ensure_enrolled(
                    &shared.api,
                    &shared.dir,
                    &shared.name,
                    shared.platform,
                    now,
                ),
            )
            .await;
            let message = match answer {
                Ok(Ok(enrolled)) => Internal::Enrolled(Box::new(enrolled)),
                Ok(Err(error)) => Internal::EnrolFailed(Box::new(error)),
                Err(_) => Internal::EnrolFailed(Box::new(LanError::Server(
                    "le serveur n'a pas répondu dans les temps".to_owned(),
                ))),
            };
            let _ = internal.send(message);
        });
    }

    /// An identity, its trust and its hashes: what a node needs before it can open
    /// a socket.
    async fn apply_enrolled(
        &mut self,
        enrolled: Enrolled,
        tasks: &mut JoinSet<()>,
        internal: &mpsc::UnboundedSender<Internal>,
    ) {
        let rotated = self
            .identity
            .as_ref()
            .map(|identity| identity.key.public() != enrolled.identity.key.public())
            .unwrap_or(false);
        let until = enrolled.until;
        self.identity = Some(Arc::clone(&enrolled.identity));
        {
            let mut trust = self
                .shared
                .trust
                .write()
                .unwrap_or_else(|error| error.into_inner());
            *trust = enrolled.trust;
        }
        // The hashing worker: the files this device shares need their SHA-256, and
        // it yields to the person (LN4's `start_with` takes the app's gates).
        self.shared.ensure_hashing();
        // The endpoint gets the new identity FIRST, and the state says so after:
        // nothing may read a credential this node does not serve yet.
        if self.active() {
            self.ensure_endpoint(rotated, tasks, internal).await;
        }
        self.shared.publish(|view| {
            view.key_until = Some(until);
            view.key_error = None;
            // Enrolled: the state a screen shows is "looking for peers" (the count
            // is derived from the replica, so nothing else is stored here).
            if matches!(
                view.state,
                LanState::NotEnrolled | LanState::Unsupported | LanState::Off
            ) {
                view.state = LanState::Searching;
            }
        });
        // A renewal can land on the same second as the credential it replaced: the
        // screens still have to hear about it (and a test waits on it).
        self.shared.touch();
    }

    /// Binds the QUIC endpoint once, starts the accept task and the discovery.
    ///
    /// A renewal keeps the endpoint and swaps its identity: a live connection keeps
    /// the identity it authenticated with, and a new one uses the new key.
    async fn ensure_endpoint(
        &mut self,
        rotated: bool,
        tasks: &mut JoinSet<()>,
        internal: &mpsc::UnboundedSender<Internal>,
    ) {
        if let Some(endpoint) = self.endpoint.as_ref() {
            if rotated && let Some(identity) = self.identity.as_ref() {
                endpoint.set_identity(Arc::clone(identity));
            }
            self.start_feed();
            return;
        }
        let Some(identity) = self.identity.clone() else {
            return;
        };
        let hello = self.shared.hello();
        let bound = {
            let handle = tokio::runtime::Handle::current();
            Endpoint::bind(
                &handle,
                self.shared.settings_now().port,
                identity,
                Arc::clone(&self.shared.trust),
                hello,
            )
        };
        match bound {
            Ok(endpoint) => {
                let endpoint = Arc::new(endpoint);
                let port = endpoint.port();
                let fallback = endpoint.port_fallback();
                if port == 0 {
                    self.shared.fail(
                        "Écoute sur le réseau local",
                        &LanError::Internal("aucun port UDP n'a pu être lié".to_owned()),
                    );
                    return;
                }
                self.shared.publish(|view| {
                    view.port = port;
                    view.port_fallback = fallback;
                });
                self.endpoint = Some(Arc::clone(&endpoint));
                // Its own task: an inbound handshake must never be cancelled by
                // another event of the select (quinn's `Incoming` is dropped and the
                // peer sees a timeout).
                let internal = internal.clone();
                tasks.spawn(async move {
                    accept_loop(endpoint, internal).await;
                });
                self.start_feed();
                self.dial_known(tasks);
            }
            Err(error) => self.shared.fail("Écoute sur le réseau local", &error),
        }
    }

    /// Announces and browses, if the node is awake.
    fn start_feed(&mut self) {
        if self.running || !self.active() {
            return;
        }
        let port = self
            .endpoint
            .as_ref()
            .map(|endpoint| endpoint.port())
            .unwrap_or(0);
        match self.feed.as_discovery().start(port) {
            Ok(()) => {
                self.running = true;
                debug!(port, "LAN : découverte démarrée");
            }
            Err(error) => self.shared.fail("Découverte sur le réseau local", &error),
        }
    }

    /// The last-known addresses: the reconnect path when multicast is filtered but
    /// unicast passes (*Discovery*, "last-known addresses").
    fn dial_known(&mut self, tasks: &mut JoinSet<()>) {
        if !self.active() || self.endpoint.is_none() {
            return;
        }
        let stored = self.shared.store.peer_addrs();
        for peer in stored.into_iter().take(MAX_KNOWN_ADDRS) {
            let Ok(ip) = peer.ip.parse::<IpAddr>() else {
                continue;
            };
            let addr = SocketAddr::new(ip, peer.port);
            self.dial(addr, stored_instance(&peer), tasks);
        }
    }

    /// Dials one address, once, unless the instance is backing off.
    fn dial(&mut self, addr: SocketAddr, instance: String, tasks: &mut JoinSet<()>) {
        let Some(endpoint) = self.endpoint.clone() else {
            return;
        };
        {
            let now = tokio::time::Instant::now();
            let mut dials = self
                .shared
                .dials
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if !dials.allow(&instance, addr, now) {
                return;
            }
            if let Some(entry) = dials.entries.get_mut(&instance) {
                entry.dialled_at = Some(now);
            }
        }
        self.shared.wire.dials.fetch_add(1, Ordering::Relaxed);
        let internal = self.shared.commands_internal();
        tasks.spawn(async move {
            let result = endpoint.connect(addr).await;
            let _ = internal.send(Internal::Dialed {
                instance,
                addr,
                result: Box::new(result),
            });
        });
    }

    /// Records one authenticated connection and starts its two loops.
    fn admit(&mut self, conn: PeerConn, tasks: &mut JoinSet<()>) {
        let peer = conn.info().device_pub;
        let connection = conn.connection().stable_id();
        self.shared
            .conns
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(peer, conn.clone());
        self.shared.peers.insert(conn.clone());
        self.sample_connection(&conn);
        self.remember(&conn);
        self.shared.touch();
        let internal = self.shared.commands_internal();
        let shared = Arc::clone(&self.shared);
        let serving = conn.clone();
        let answering = internal.clone();
        tasks.spawn(async move {
            serve_loop(shared, serving, answering).await;
        });
        let shared = Arc::clone(&self.shared);
        tasks.spawn(async move {
            control_loop(shared, conn, internal).await;
        });
        let _ = connection;
    }

    /// Saves the address, for the reconnect after a network change or a sleep.
    fn remember(&self, conn: &PeerConn) {
        let info = conn.info();
        let stored = StoredPeer {
            device_pub: crate::cred::hex_encode(&info.device_pub),
            subject: crate::cred::hex_encode(&info.subject.0),
            name: info.hello.as_ref().map(|hello| hello.name.clone()),
            ip: info.addr.ip().to_string(),
            port: info.addr.port(),
            seen_at: auth::now_seconds(),
        };
        if let Err(error) = self.shared.store.put_peer_addr(&stored) {
            debug!(%error, "LAN : adresse d'appareil non enregistrée");
        }
        let mut dials = self
            .shared
            .dials
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        dials.up(
            &stored_instance(&stored),
            info.addr,
            tokio::time::Instant::now(),
        );
    }

    /// What a command from the app does.
    async fn command(
        &mut self,
        command: Command,
        tasks: &mut JoinSet<()>,
        internal: &mpsc::UnboundedSender<Internal>,
    ) {
        match command {
            Command::EnrollNow => {
                // The app's explicit call (after a login, or from Réglages): it is
                // one HTTP round trip on the Bulk lane, never a LAN packet, so it
                // does not need the app to be in the foreground.
                if self.shared.settings_now().enabled {
                    self.enrol(tasks, internal);
                }
            }
            Command::Forget => self.forget(tasks).await,
            Command::NetworkChanged => {
                self.feed.as_discovery().network_changed();
                if self.active() {
                    self.dial_known(tasks);
                }
            }
            Command::Settings(settings) => self.apply_settings(*settings, tasks, internal).await,
        }
    }

    async fn apply_settings(
        &mut self,
        settings: LanSettings,
        tasks: &mut JoinSet<()>,
        internal: &mpsc::UnboundedSender<Internal>,
    ) {
        let previous = self.shared.settings_now();
        settings::save(&self.shared.cache, &settings);
        *self
            .shared
            .settings
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = settings.clone();
        self.shared.touch();
        if !settings.enabled {
            // « Échanger avec mes appareils à proximité » is off: no socket, no
            // announcement, no hash, nothing.
            self.sleep_now().await;
            self.close_endpoint().await;
            self.stop_hashing();
            self.identity = None;
            self.shared.publish(|view| view.state = LanState::Off);
            return;
        }
        if !previous.enabled && settings.enabled {
            // Back on: the cached credential answers without a request, and the
            // addresses this device already knows are dialled again (it closed
            // everything on the way off, and nothing announces itself to it).
            self.start_or_restore(tasks, internal).await;
            self.dial_known(tasks);
            return;
        }
        if previous.port != settings.port {
            self.close_endpoint().await;
            self.ensure_endpoint(false, tasks, internal).await;
        }
    }

    /// Logout: the key, the credential and the roster go, and so does every peer.
    async fn forget(&mut self, tasks: &mut JoinSet<()>) {
        self.sleep_now().await;
        self.stop_hashing();
        enroll::forget(
            Some(self.shared.api.as_ref()),
            &self.shared.dir,
            &self.shared.store,
        )
        .await;
        let _ = self.shared.store.forget_peers();
        self.shared.sets.clear();
        *self
            .shared
            .trust
            .write()
            .unwrap_or_else(|error| error.into_inner()) = TrustState::default();
        self.identity = None;
        self.close_endpoint().await;
        {
            let mut dials = self
                .shared
                .dials
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            dials.entries.clear();
            dials.order.clear();
        }
        let _ = tasks;
        self.shared.publish(|view| {
            view.state = LanState::NotEnrolled;
            view.key_until = None;
            view.key_error = None;
        });
    }

    fn stop_hashing(&mut self) {
        let hashing = self
            .shared
            .hashing
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(hashing) = hashing {
            hashing.stop();
        }
    }

    /// The activity flags moved: wake up (endpoint, discovery, dials), or leave the
    /// sleep countdown to the `select!`.
    async fn on_activity(
        &mut self,
        tasks: &mut JoinSet<()>,
        internal: &mpsc::UnboundedSender<Internal>,
    ) {
        if !self.active() || !self.shared.settings_now().enabled {
            return;
        }
        if self.identity.is_none() {
            self.start_or_restore(tasks, internal).await;
        }
        self.ensure_endpoint(false, tasks, internal).await;
        self.dial_known(tasks);
    }

    /// Asleep: the connections close, the discovery stops, and nothing is sent
    /// until the app comes back.
    async fn sleep_now(&mut self) {
        let drained: Vec<PeerConn> = {
            let mut conns = self
                .shared
                .conns
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            conns.drain().map(|(_, conn)| conn).collect()
        };
        for conn in drained {
            // Code 0 is "going away", never `CLOSE_REFUSED`: the peer must not back
            // off for ten minutes because the phone's screen went off.
            conn.close(0);
        }
        if self.running {
            self.feed.as_discovery().stop();
            self.running = false;
            debug!("LAN : découverte arrêtée");
        }
        // The peers leave as their loops end; the version is what tells a screen
        // (the peer list is derived from the replica, so the view itself does not
        // move — without this `touch`, a list would keep a device that is gone).
        self.shared.touch();
    }

    async fn close_endpoint(&mut self) {
        if let Some(endpoint) = self.endpoint.take() {
            // Closes every connection with code 0 and joins quinn's only task.
            endpoint.close().await;
        }
        if self.running {
            self.feed.as_discovery().stop();
            self.running = false;
        }
        self.shared
            .conns
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
        self.shared.touch();
    }

    async fn on_discovery(&mut self, event: DiscoveryEvent, tasks: &mut JoinSet<()>) {
        if !self.active() || !self.shared.settings_now().enabled {
            // Belt and braces: the feed is stopped when the node sleeps, so this
            // only happens in the instant of the transition.
            return;
        }
        match event {
            DiscoveryEvent::Found { addr, instance } => self.dial(addr, instance, tasks),
            DiscoveryEvent::Lost { instance } => {
                let mut dials = self
                    .shared
                    .dials
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                dials.entries.remove(&instance);
                dials.order.retain(|kept| kept != &instance);
            }
        }
    }

    /// The local inventory moved: every peer is told, once, with its version.
    async fn refresh_index(&mut self) {
        let before = (
            self.shared.index.version(),
            self.shared.index.sets_version(),
        );
        self.shared.index.refresh();
        let after = (
            self.shared.index.version(),
            self.shared.index.sets_version(),
        );
        if before == after {
            return;
        }
        self.shared.touch();
        let conns: Vec<PeerConn> = self
            .shared
            .conns
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .cloned()
            .collect();
        for conn in conns {
            let frame = if after.0 != before.0 {
                proto::Control::InventoryChanged { version: after.0 }
            } else {
                proto::Control::SetsChanged { version: after.1 }
            };
            if let Err(error) = conn.send_control(frame).await {
                debug!(%error, "LAN : un pair n'a pas reçu son changement d'inventaire");
            }
        }
    }

    /// What a child task reported.
    async fn handle(
        &mut self,
        message: Internal,
        tasks: &mut JoinSet<()>,
        internal: &mpsc::UnboundedSender<Internal>,
    ) {
        match message {
            Internal::Enrolled(enrolled) => {
                self.enrolment_running = false;
                self.apply_enrolled(*enrolled, tasks, internal).await;
            }
            Internal::EnrolFailed(error) => {
                self.enrolment_running = false;
                match *error {
                    // Rule 7: the server has no such feature (or its admin turned
                    // it off): nothing runs, no screen mentions it.
                    LanError::Unsupported => {
                        self.close_endpoint().await;
                        self.stop_hashing();
                        self.identity = None;
                        self.shared.publish(|view| {
                            view.state = LanState::Unsupported;
                            view.key_error = None;
                        });
                    }
                    LanError::Disabled => {
                        self.close_endpoint().await;
                        self.stop_hashing();
                        self.identity = None;
                        self.shared.publish(|view| view.state = LanState::Off);
                    }
                    other => {
                        // Keep whatever the device already holds (a trip without a
                        // server works on the cached credential) and say why.
                        let cached = enroll::load_cached(
                            &self.shared.dir,
                            &self.shared.store,
                            auth::now_seconds(),
                        );
                        match cached {
                            Some(enrolled) => {
                                self.apply_enrolled(enrolled, tasks, internal).await;
                                self.shared.publish(|view| {
                                    view.key_error = Some(other.cause_fr());
                                });
                            }
                            None => self.shared.publish(|view| {
                                view.state = LanState::NotEnrolled;
                                view.key_error = Some(other.cause_fr());
                            }),
                        }
                        self.shared.fail("Renouvellement de la clé LAN", &other);
                    }
                }
            }
            Internal::Accepted(conn) => self.admit(*conn, tasks),
            Internal::AcceptFailed(error) => {
                if !matches!(*error, LanError::PeerGone) {
                    self.shared.fail("Connexion d'un appareil", &error);
                }
                // An attempted connection is an event a screen re-reads for, even
                // when the sentence is the one already shown (one line per cause).
                self.shared.touch();
            }
            Internal::Dialed {
                instance,
                addr,
                result,
            } => match *result {
                Ok(conn) => {
                    {
                        let mut dials = self
                            .shared
                            .dials
                            .lock()
                            .unwrap_or_else(|error| error.into_inner());
                        dials.up(&instance, addr, tokio::time::Instant::now());
                    }
                    self.admit(conn, tasks);
                }
                Err(error) => {
                    {
                        let mut dials = self
                            .shared
                            .dials
                            .lock()
                            .unwrap_or_else(|error| error.into_inner());
                        dials.back_off(&instance, addr, tokio::time::Instant::now());
                    }
                    if !matches!(error, LanError::PeerGone) {
                        self.shared.fail("Appareil à proximité", &error);
                    }
                    self.shared.touch();
                }
            },
            Internal::Closed {
                peer,
                connection,
                duplicate,
            } => {
                let current = {
                    let conns = self
                        .shared
                        .conns
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    conns.get(&peer).map(|conn| conn.connection().stable_id())
                };
                // The connection leaves the map when it IS the one the map holds (or
                // the map is already empty, a sleep having drained it). The
                // REPLICAS are a separate question: a duplicate loser's device is
                // still here on the survivor, so forgetting them would cost a full
                // re-sync (one `Bulk` quiet window, ≥ 1 s) during which a download
                // decided for that peer falls back to the server — and to nothing at
                // all on a trip (measured 2 of 20 netns runs, LN15).
                if current == Some(connection) || current.is_none() {
                    let drained = self
                        .shared
                        .conns
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .remove(&peer);
                    if let Some(conn) = drained {
                        // The close's own datagrams belong to this connection: read
                        // them once more, so the total is what really left.
                        self.sample_connection(&conn);
                    }
                }
                if close_forgets_replica(duplicate, current, connection) {
                    self.shared.peers.remove(&peer);
                    self.shared.sets.remove(&peer);
                }
                self.shared.touch();
                // An idle connection dies after `IDLE_TIMEOUT` (LN6 keeps no
                // keep-alive: a phone with the screen off must not ping). While the
                // app is active the node re-establishes it — ONE handshake, and only
                // when the device has no connection left at all: a duplicate that
                // collapsed to the survivor is not a loss, and the dial table's
                // `MIN_REDIAL` keeps a peer that hangs up at once from being
                // redialled in a loop.
                let gone = !self
                    .shared
                    .conns
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .contains_key(&peer);
                if gone && !duplicate && self.active() && self.shared.settings_now().enabled {
                    self.dial_known(tasks);
                }
            }
            Internal::Note(what, error) => self.shared.fail(&what, &error),
        }
    }

    /// Reads quinn's own counters of ONE connection into the counted wire (never on
    /// the UI thread: `Connection::stats` locks the connection's state).
    ///
    /// It is cumulative per connection and this adds the DELTA since the previous
    /// read, so a reconnection that restarts at zero never moves the total
    /// backwards.
    fn sample_connection(&self, conn: &PeerConn) {
        let stats = conn.connection().stats();
        let sample = WireCounts {
            packets_out: stats.udp_tx.datagrams,
            packets_in: stats.udp_rx.datagrams,
            bytes_out: stats.udp_tx.bytes,
            bytes_in: stats.udp_rx.bytes,
            dials: 0,
        };
        let mut seen = self
            .shared
            .wire_seen
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let previous = seen
            .insert(conn.connection().stable_id(), sample)
            .unwrap_or_default();
        let wire = &self.shared.wire;
        wire.packets_out.fetch_add(
            sample.packets_out.saturating_sub(previous.packets_out),
            Ordering::Relaxed,
        );
        wire.packets_in.fetch_add(
            sample.packets_in.saturating_sub(previous.packets_in),
            Ordering::Relaxed,
        );
        wire.bytes_out.fetch_add(
            sample.bytes_out.saturating_sub(previous.bytes_out),
            Ordering::Relaxed,
        );
        wire.bytes_in.fetch_add(
            sample.bytes_in.saturating_sub(previous.bytes_in),
            Ordering::Relaxed,
        );
    }

    /// Reads every live connection's counters (the 1 s tick while a peer is
    /// connected), and forgets the ones that are gone — their last sample is
    /// already in the total.
    fn sample_wire(&mut self) {
        let conns: Vec<PeerConn> = self
            .shared
            .conns
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .cloned()
            .collect();
        for conn in &conns {
            self.sample_connection(conn);
        }
        let live: Vec<usize> = conns
            .iter()
            .map(|conn| conn.connection().stable_id())
            .collect();
        let mut seen = self
            .shared
            .wire_seen
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        seen.retain(|id, _| live.contains(id));
    }

    /// Stop, in the note's order: the endpoint (which closes the connections and
    /// joins quinn's task), the discovery (which joins its daemon), then every child
    /// task — before the runtime goes.
    async fn finish(&mut self, tasks: &mut JoinSet<()>) {
        let endpoint = self.endpoint.take();
        if let Some(endpoint) = endpoint {
            endpoint.close().await;
        }
        if self.running {
            self.feed.as_discovery().stop();
            self.running = false;
        }
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        let drained: Vec<DevicePub> = {
            let mut conns = self
                .shared
                .conns
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            conns.drain().map(|(peer, _)| peer).collect()
        };
        for peer in drained {
            self.shared.peers.remove(&peer);
        }
        debug!("nz-lan : tout est arrêté");
    }
}

/// The accept loop: its own task, so nothing cancels a handshake in flight.
async fn accept_loop(endpoint: Arc<Endpoint>, internal: mpsc::UnboundedSender<Internal>) {
    loop {
        match endpoint.accept().await {
            Some(Ok(conn)) => {
                if internal.send(Internal::Accepted(Box::new(conn))).is_err() {
                    return;
                }
            }
            Some(Err(error)) => {
                if internal
                    .send(Internal::AcceptFailed(Box::new(error)))
                    .is_err()
                {
                    return;
                }
            }
            None => return,
        }
    }
}

/// Answers a peer's requests, within the scope authentication gave it.
async fn serve_loop(
    shared: Arc<Shared>,
    conn: PeerConn,
    internal: mpsc::UnboundedSender<Internal>,
) {
    let peer = conn.info().device_pub;
    let connection = conn.connection().stable_id();
    let scope = conn.info().scope.clone();
    let peer_name = conn
        .info()
        .hello
        .as_ref()
        .map(|hello| hello.name.clone())
        .unwrap_or_default();
    while let Some(incoming) = conn.next_incoming().await {
        if !shared.settings_now().serve {
            // « Partager mes sons » is off: this device shares nothing, and it says
            // so with the answers of a node that HAS nothing — never a refusal that
            // would tell a friend more than an absent file does.
            let head = match &incoming.request {
                proto::Request::Ping => proto::Response::Pong,
                proto::Request::Inventory { page, .. } => proto::Response::InventoryFull {
                    version: 0,
                    page: *page,
                    last: true,
                    items: Vec::new(),
                },
                proto::Request::Sets { page } => proto::Response::Sets {
                    version: 0,
                    page: *page,
                    last: true,
                    sets: Vec::new(),
                },
                proto::Request::Meta { .. } => proto::Response::Meta { rows: Vec::new() },
                _ => proto::Response::NotFound,
            };
            if let Err(error) = incoming.respond(head, None).await {
                debug!(%error, "LAN : une réponse vide n'a pas pu être écrite");
            }
            continue;
        }
        let transfer = match &incoming.request {
            proto::Request::Fetch { track, urgency, .. } => {
                Some(shared.begin_transfer(TransferView {
                    track: track.clone(),
                    title: String::new(),
                    peer: peer_name.clone(),
                    direction: Direction::Send,
                    bytes: 0,
                    total: 0,
                    urgency: *urgency,
                }))
            }
            _ => None,
        };
        match shared.server.handle(&scope, incoming).await {
            Ok(bytes) => {
                if let Some(id) = transfer {
                    shared.end_transfer(id, bytes);
                }
            }
            Err(error) => {
                if let Some(id) = transfer {
                    shared.end_transfer(id, 0);
                }
                debug!(%error, "LAN : un service a échoué");
            }
        }
    }
    let _ = internal.send(Internal::Closed {
        peer,
        connection,
        duplicate: closed_as_duplicate(&conn),
    });
}

/// Whether this connection ended because the transport COLLAPSED a duplicate pair.
///
/// The loser of the pair is closed by [`crate::transport::ConnTable`] with
/// [`CLOSE_DUPLICATE`], and its `close_reason` carries that code on both ends. The
/// caller uses it to keep the device's replicas: the survivor is the same device,
/// one handshake away (LN15's measurement).
fn closed_as_duplicate(conn: &PeerConn) -> bool {
    match conn.connection().close_reason() {
        Some(quinn::ConnectionError::ApplicationClosed(close)) => {
            close.error_code == quinn::VarInt::from_u32(CLOSE_DUPLICATE)
        }
        _ => false,
    }
}

/// Whether a connection's close must FORGET the device's replicas (its inventory
/// and its sets).
///
/// - the closed connection is still the one the node holds, or the node holds none
///   (a sleep drained it): the device is gone, so its replicas go with it;
/// - a duplicate loser does NOT forget them: the transport collapsed a pair, and the
///   same device is one handshake away on the survivor. Forgetting them costs a full
///   re-sync (one `Bulk` quiet window, ≥ 1 s) during which a download decided for
///   that peer falls back to the server — and to nothing at all on a trip; measured
///   2 of 20 netns runs before this rule (LN15);
/// - a NEWER connection of the same device is current: leave it alone (LN10).
///
/// A peer that closes with `0x12` without having been collapsed keeps its last
/// inventory in memory until its next connection or a `forget`; it cannot be served
/// from (the connection is gone, a fetch fails and marks it bad for that track), the
/// snapshot lists no device without a live connection, and its replica is replaced on
/// reconnect.
fn close_forgets_replica(duplicate: bool, current: Option<usize>, connection: usize) -> bool {
    if duplicate {
        return false;
    }
    match current {
        Some(id) => id == connection,
        None => true,
    }
}

/// The candidates a decision may use, which are the peers whose replica holds the
/// track AND whose connection is still open.
///
/// A replica outlives a duplicate collapse until the survivor is admitted (LN15), so
/// a decision taken in that window would offer a connection that is already closed:
/// the opener then opens nothing and falls back to the server — and on a trip there
/// is no server to fall back to. Measured as a failed fetch in 8 of 20 netns runs
/// before this filter. `close_reason()` is `None` while the connection lives, so the
/// check costs one lock per candidate on the tap path and never a request.
fn live_candidates(peers: &Peers, track: &str) -> Vec<crate::peers::Candidate> {
    peers
        .candidates(track)
        .into_iter()
        .filter(|candidate| {
            peers
                .get(&candidate.peer)
                .is_some_and(|conn| conn.connection().close_reason().is_none())
        })
        .collect()
}

/// Reads the peer's control stream: gossip on connect, and the two version notices
/// that make a replica catch up.
async fn control_loop(
    shared: Arc<Shared>,
    conn: PeerConn,
    internal: mpsc::UnboundedSender<Internal>,
) {
    let peer = conn.info().device_pub;
    let connection = conn.connection().stable_id();
    // Authentication's own answer, never a comparison this file makes up.
    let same_subject = conn.info().relation == Relation::Own;
    // What this device knows that the peer may not. `PeerInfo` does not carry the
    // peer's revocation serial (LN6's shape), so the offers are built for an
    // unknown peer: the peer keeps the maximum and silently ignores what is not
    // newer, which costs at most one list and one roster per connection.
    let offers = {
        let trust = shared
            .trust
            .read()
            .unwrap_or_else(|error| error.into_inner());
        gossip::offers(&trust, 0, same_subject, None)
    };
    for frame in offers {
        if let Err(error) = conn.send_control(frame).await {
            debug!(%error, "LAN : la confiance n'a pas pu être envoyée");
            break;
        }
    }
    let _ = conn
        .send_control(proto::Control::InventoryChanged {
            version: shared.index.version(),
        })
        .await;
    let _ = conn
        .send_control(proto::Control::SetsChanged {
            version: shared.index.sets_version(),
        })
        .await;
    // The connect burst: the peer's download sets, then its inventory, inside ONE
    // `Bulk` window. One window because both are the same background work (rule 1:
    // a 480 KB inventory and a sets page yield to what the person is doing), and
    // because a second `acquire` would make the second sync wait the quiet window
    // again — a peer's files would take twice as long to become usable.
    //
    // The sets go FIRST on purpose: a caller that waits for the inventory to arrive
    // (a tap, and LN10's own measurement of "a tap sends no request") then already
    // has the sets, so no request can land in the window it starts measuring once
    // the inventory is visible.
    {
        let gates = shared.api.gates();
        let _bulk = gates.acquire(nz_api::Lane::Bulk).await;
        replicate_sets(&shared, &internal, &conn).await;
        replicate_inventory(&shared, &internal, &peer).await;
    }
    while let Some(frame) = conn.next_control().await {
        match frame {
            proto::Control::Revocations { .. } | proto::Control::Roster { .. } => {
                match gossip::accept(&shared.trust, &shared.store, &frame, same_subject) {
                    Ok(true) => shared.touch(),
                    Ok(false) => {}
                    Err(error) => debug!(%error, "LAN : une confiance reçue a été refusée"),
                }
            }
            proto::Control::InventoryChanged { .. } => {
                let gates = shared.api.gates();
                let _bulk = gates.acquire(nz_api::Lane::Bulk).await;
                replicate_inventory(&shared, &internal, &peer).await;
            }
            proto::Control::SetsChanged { .. } => {
                let gates = shared.api.gates();
                let _bulk = gates.acquire(nz_api::Lane::Bulk).await;
                replicate_sets(&shared, &internal, &conn).await;
            }
            proto::Control::Bye => {
                conn.close(0);
                break;
            }
            _ => {}
        }
    }
    let _ = internal.send(Internal::Closed {
        peer,
        connection,
        duplicate: closed_as_duplicate(&conn),
    });
}

/// Replicates one peer's inventory. The caller holds the `Bulk` lane: this is
/// background work for this device (rule 1) and yields to what the person does.
async fn replicate_inventory(
    shared: &Arc<Shared>,
    internal: &mpsc::UnboundedSender<Internal>,
    peer: &DevicePub,
) {
    match shared.peers.sync_inventory(peer).await {
        Ok(()) => shared.touch(),
        Err(LanError::PeerGone) => {}
        Err(error) => {
            let _ = internal.send(Internal::Note(
                "Inventaire d'un appareil".to_owned(),
                Box::new(error),
            ));
        }
    }
}

/// Replicates one peer's download sets. The caller holds the `Bulk` lane, for the
/// same reason as the inventory.
///
/// A failure is a line for the screen and never a verdict about the peer (the
/// replica keeps what it had). A peer that answers `NotFound` shares nothing (an
/// older node, or one whose « Partager mes sons » never reached this frame): its
/// sets are empty, not an error.
async fn replicate_sets(
    shared: &Arc<Shared>,
    internal: &mpsc::UnboundedSender<Internal>,
    conn: &PeerConn,
) {
    match shared.sets.sync(conn).await {
        Ok(()) => shared.touch(),
        Err(LanError::PeerGone) => {}
        Err(error) => {
            let _ = internal.send(Internal::Note(
                "Ensembles d'un appareil".to_owned(),
                Box::new(error),
            ));
        }
    }
}

impl Shared {
    fn begin_transfer(&self, row: TransferView) -> u64 {
        self.transfers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .begin(row)
    }

    fn end_transfer(&self, id: u64, bytes: u64) {
        self.transfers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .end(id, bytes);
        self.touch();
    }

    /// The sender of the driver's internal channel.
    ///
    /// It lives beside the commands' sender: the driver creates the channel and
    /// hands this clone to every child, so a child never holds the receiver.
    fn commands_internal(&self) -> mpsc::UnboundedSender<Internal> {
        self.internal.clone()
    }
}

/// A `tokio` ticker that fires every `period`, the first time one period from now
/// (never immediately: a start must not spawn work before the first frame).
fn ticker(period: Duration) -> tokio::time::Interval {
    tokio::time::interval_at(tokio::time::Instant::now() + period, period)
}

/// The instance handle of a stored address: it must not collide with an mDNS
/// announcement, so it is namespaced.
fn stored_instance(peer: &StoredPeer) -> String {
    format!("stored-{}", peer.device_pub)
}

/// Whether the server has the LAN feature at all.
///
/// `None` when the question cannot be asked HERE (we are inside a runtime: blocking
/// on a probe would panic, and `has_route`'s own rule is that a feature is never
/// hidden on a doubt) — the driver then asks as its first act, and a `404` there
/// answers [`LanState::Unsupported`] and stops everything.
fn probe_feature(api: &nz_api::Api) -> Option<bool> {
    if tokio::runtime::Handle::try_current().is_ok() {
        return None;
    }
    // A current-thread runtime owns NO thread: dropping it leaves nothing behind,
    // which is what "an older server starts nothing" needs.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    runtime.block_on(api.has_route(nz_api::features::Feature::Lan.probe()))
}

/// Lowers this thread's priority (`nice` semantics; per thread on Linux).
fn lower_priority(nice: i32) {
    // SAFETY: `setpriority(PRIO_PROCESS, 0, …)` only changes the calling thread's
    // scheduling weight; it touches no memory, and a refusal (EPERM) is a no-op.
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, nice);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defaults Réglages shows on a fresh device: the feature on, sharing on,
    /// the three tolerated rungs, and the port the firewall line names.
    #[test]
    fn the_default_settings_are_the_notes() {
        let settings = LanSettings::default();
        assert!(settings.enabled);
        assert!(settings.serve);
        assert_eq!(settings.port, DEFAULT_PORT);
        assert_eq!(DEFAULT_PORT, 45910);
        assert_eq!(settings.tolerated, Tolerance::default_set());
    }

    /// The note's numbers are the code's: 60 s of grace, a five-second inventory
    /// refresh, a daily server round trip and ten minutes of silence for a node
    /// that refused us. Written out HERE as well, so a mutant that moves a constant
    /// cannot move the test's boundary with it.
    #[test]
    fn the_service_numbers_are_the_notes() {
        assert_eq!(SLEEP_GRACE, Duration::from_secs(60));
        assert_eq!(REFRESH_EVERY, Duration::from_secs(5));
        assert_eq!(ENROL_EVERY, Duration::from_secs(86_400));
        assert_eq!(DIAL_BACKOFF, Duration::from_secs(600));
        assert_eq!(LAN_NICE, 5);
        assert_eq!(MAX_DIAL_ENTRIES, 64);
        assert_eq!(MAX_FAILURES, 8);
    }

    /// A refused node is left alone for ten minutes, and the table never grows past
    /// its cap: a flooded segment costs a bounded amount of memory.
    #[test]
    fn a_refused_node_is_not_dialled_again_for_ten_minutes() {
        let mut table = DialTable::default();
        let now = tokio::time::Instant::now();
        let addr: SocketAddr = "127.0.0.1:45910".parse().expect("adresse");
        assert!(table.allow("nz-aaaaaaaaaa", addr, now));
        table.back_off("nz-aaaaaaaaaa", addr, now);
        // Five seconds later, and one second before the backoff ends: silence.
        assert!(!table.allow("nz-aaaaaaaaaa", addr, now + Duration::from_secs(5)));
        assert!(!table.allow(
            "nz-aaaaaaaaaa",
            addr,
            now + DIAL_BACKOFF - Duration::from_secs(1)
        ));
        // At the boundary, and after it: the instance may be dialled again.
        assert!(table.allow("nz-aaaaaaaaaa", addr, now + DIAL_BACKOFF));
        assert!(table.allow(
            "nz-aaaaaaaaaa",
            addr,
            now + DIAL_BACKOFF + Duration::from_secs(1)
        ));

        for index in 0..(MAX_DIAL_ENTRIES * 4) {
            table.allow(&format!("nz-{index:010x}"), addr, now);
        }
        assert_eq!(table.entries.len(), MAX_DIAL_ENTRIES, "the table is capped");
        assert_eq!(table.order.len(), MAX_DIAL_ENTRIES);
    }

    /// The failure lines are bounded and newest first: a screen shows the cause,
    /// and a peer that fails in a loop cannot grow the snapshot.
    #[test]
    fn the_failure_lines_are_bounded_and_newest_first() {
        let mut view = View::default();
        for index in 0..(MAX_FAILURES * 3) {
            view.failures
                .push_front((format!("cas {index}"), "une cause".to_owned()));
            while view.failures.len() > MAX_FAILURES {
                view.failures.pop_back();
            }
        }
        assert_eq!(view.failures.len(), MAX_FAILURES);
        assert_eq!(view.failures.front().expect("la plus récente").0, "cas 23");
    }

    /// The transfers registry is bounded too: sixty-four streams from one hostile
    /// peer cannot turn into sixty-four rows this device keeps.
    #[test]
    fn the_transfer_registry_is_bounded() {
        let mut transfers = Transfers::default();
        for index in 0..(MAX_TRANSFERS * 4) {
            transfers.begin(TransferView {
                track: format!("3{index:07}"),
                title: String::new(),
                peer: "PC".to_owned(),
                direction: Direction::Send,
                bytes: 0,
                total: 0,
                urgency: Urgency::Bulk,
            });
        }
        assert_eq!(transfers.rows().len(), MAX_TRANSFERS);
        let first = transfers.rows()[0].track.clone();
        let id = transfers.rows.first().expect("une ligne").0;
        transfers.end(id, 42);
        assert!(!transfers.rows().iter().any(|row| row.track == first));
    }
    /// LN15's rule: the loser of a duplicate pair does NOT cost the device its
    /// replicas. The same device is one handshake away on the survivor, and
    /// forgetting the inventory costs a full re-sync (one `Bulk` quiet window)
    /// during which a download decided for that peer falls back to the server — and
    /// to nothing at all on a trip. Measured 2 of 20 netns runs before this rule.
    ///
    /// Every other close keeps its own behaviour: the device is gone, so its
    /// replicas go with it, and a NEWER connection (a reconnect that already landed)
    /// is never disturbed.
    #[test]
    fn a_duplicate_close_keeps_the_replicas_and_a_real_one_forgets_them() {
        // A duplicate loser, in each of the three states the map can be in.
        assert!(
            !close_forgets_replica(true, Some(7), 7),
            "le doublon courant garde la réplique"
        );
        assert!(
            !close_forgets_replica(true, Some(8), 7),
            "un doublon dépassé ne touche pas la connexion survivante"
        );
        assert!(
            !close_forgets_replica(true, None, 7),
            "un doublon après un sommeil garde la réplique du survivant"
        );
        // A plain close of the current connection: the device is gone.
        assert!(close_forgets_replica(false, Some(7), 7));
        // …and of a connection the map no longer holds (a sleep drained it).
        assert!(close_forgets_replica(false, None, 7));
        // A newer connection is live: leave it alone (LN10's rule).
        assert!(!close_forgets_replica(false, Some(8), 7));
    }

    /// The code the duplicate rule is written against is the transport's own, and it
    /// is not one of the two refusal codes: a duplicate is "the pair is already
    /// connected", never "do not dial me again for ten minutes".
    #[test]
    fn the_duplicate_close_code_is_the_transports_and_not_a_refusal() {
        assert_eq!(CLOSE_DUPLICATE, 0x12);
        assert_ne!(CLOSE_DUPLICATE, crate::auth::CLOSE_REFUSED);
        assert_ne!(CLOSE_DUPLICATE, crate::auth::CLOSE_WRONG_MAJOR);
    }
}
