//! LAN nodes: this desktop app as one of the person's devices.
//!
//! `docs/agents/lan-nodes.md` is the contract; this file is the app's half of it
//! (LN12) and nothing else is: the node itself is `nz-lan`'s [`Lan`], the
//! peer-first seams are LN11's [`PeerFirstOpener`] and [`PeerFirstDownloader`],
//! and the screens read [`snapshot`] without ever touching SQLite or the
//! network.
//!
//! Four rules shape what lives here.
//!
//! - **An older server hides the feature, and nothing starts.** [`attach`] never
//!   creates a directory, a socket or a thread for a server that answers `404`
//!   on `/api/lan/status`: `Lan::start` probes first and answers
//!   `Err(Unsupported)`, and the answer is remembered for that server so a
//!   screen opened later costs no second probe.
//! - **The login stays instant.** The probe is a network round trip and the
//!   index a SQLite read; both run on their own thread ([`start_in_background`]),
//!   never on the GPUI one. The wrappers below are transparent until the node
//!   exists, so a tap in the first hundred milliseconds of a session goes
//!   straight to the server — 0 ms added, which is what rule 1 asks of a
//!   subsystem that is not ready yet.
//! - **The tap never waits for the LAN.** A connected peer is a `decide` in
//!   memory (1.3 µs measured by LN11); a node that is not up is a `None` and one
//!   branch. Nothing here retries, sleeps or asks the network to find out where
//!   a track should come from.
//! - **The quit joins the node LAST.** The engine's threads and `nz-bulk` drive
//!   `nz-net`; `nz-lan` drives its own runtime, whose tasks are the peers'
//!   requests. So `NetRuntime::drop` calls `offline::shutdown_process`, then
//!   `engine::shutdown_process`, then [`shutdown_process`] here.
//!
//! The one thing this file adds to the plan's surface is [`source_line`]: LN11's
//! correction says there is no `LanEvent::Served`, so the app remembers which
//! peer served which track — in [`record_served`], called by the opener wrapper
//! with the *decision* it was about to take and the quality the body really
//! carried — and « En lecture » prints it.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use nz_api::{Api, Quality};
use nz_audio::{ByteStream, Lane, SourceError, StreamOpener, StreamReady};
use nz_cache::Cache;
use nz_lan::downloader::PeerFirstDownloader;
use nz_lan::opener::PeerFirstOpener;
use nz_lan::service::{Activity, DiscoveryKind, Lan, LanConfig};
use nz_lan::sets::{PeerSetKey, download_peer_sets};
use nz_lan::{DevicePub, LanError, Source, Urgency};
use nz_model::TrackId;
use nz_offline::{ApiDownloader, Downloader, Fetched, Outcome, Progress, Stop};
use tracing::{debug, warn};

pub use nz_lan::service::{
    Direction, LanSettings, LanSnapshot, LanState, PeerSet, PeerView, TransferView,
};
pub use nz_lan::{Relation, Tolerance};

// --- the process's node ------------------------------------------------------

/// One started node, with the session it belongs to.
struct Node {
    /// `"<server>|<owner>"`: a login on another server (or as another account)
    /// is another node, and the previous one is shut down first.
    session: String,
    /// How a wrapper tells "the node I built my adapter for" from "a new one".
    generation: u64,
    lan: Lan,
}

fn node_cell() -> &'static Mutex<Option<Node>> {
    static NODE: OnceLock<Mutex<Option<Node>>> = OnceLock::new();
    NODE.get_or_init(|| Mutex::new(None))
}

fn lock<T>(cell: &'static OnceLock<Mutex<T>>, make: impl FnOnce() -> T) -> &'static Mutex<T> {
    cell.get_or_init(|| Mutex::new(make()))
}

/// The servers this process already asked, and the answer: `404` means every
/// later screen and every later login costs no second probe (`nz-lan`'s
/// `Lan::start` probes on its own, once per start).
fn answered_cell() -> &'static Mutex<Vec<String>> {
    lock(&ANSWERED, Vec::new)
}

static ANSWERED: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

/// The session currently being started, so a login that runs both the engine's
/// and the worker's hook spawns ONE start thread.
static STARTING: OnceLock<Mutex<Option<String>>> = OnceLock::new();

/// The process is closing: a start that lands now must not publish a node.
static CLOSING: AtomicBool = AtomicBool::new(false);

/// Bumped for every node this process starts.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// The `"<server>|<owner>"` of a session: two logins of the same account on the
/// same server share a node, anything else does not.
fn session_key(api: &Api) -> String {
    format!(
        "{}|{}",
        api.base_url(),
        api.owner().unwrap_or_else(|| "-".to_owned())
    )
}

/// The node this process holds, if any.
pub fn current() -> Option<Lan> {
    node_cell()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
        .map(|node| node.lan.clone())
}

/// The generation of the node in force: a screen or a wrapper uses it to know
/// whether the snapshot it painted belongs to the node there is now.
pub fn generation() -> u64 {
    node_cell()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
        .map(|node| node.generation)
        .unwrap_or(0)
}

/// The node's published state, or `None` when this app is not a node.
///
/// One short lock and counted replicas inside `nz-lan`: never SQLite, never the
/// network (LN10 measured 482–974 ns per call).
pub fn snapshot() -> Option<LanSnapshot> {
    current().map(|lan| lan.snapshot())
}

/// The change signal's current value: `0` when no node runs. A screen compares
/// it to what it painted and re-reads [`snapshot`] only when it moved.
pub fn version() -> u64 {
    match current() {
        Some(lan) => *lan.events().borrow(),
        None => 0,
    }
}

/// The node's settings as the device holds them, node or not (the same `kv`
/// rows either way: the screen shows what the person chose even while the node
/// is off).
pub fn settings(cache: Option<&Cache>) -> LanSettings {
    match cache {
        Some(cache) => nz_lan::settings::load(cache),
        None => LanSettings::default(),
    }
}

/// Write the node's settings (`lan.enabled`, `lan.serve`, `lan.tolerated`,
/// `lan.port`): persisted on this device in `nz-cache`'s `kv`, and applied at
/// once by the driver when a node runs.
///
/// The two halves are one function on purpose: a click that lands while the node
/// is still starting (or after it stopped, with the feature switched off) must
/// still be remembered — otherwise a switch would seem to forget itself on the
/// next launch.
pub fn save_settings(cache: Option<&Cache>, settings: LanSettings) {
    if let Some(cache) = cache {
        nz_lan::settings::save(cache, &settings);
    }
    if let Some(lan) = current() {
        lan.set_settings(settings);
    }
}

/// The UDP port a firewall rule has to open: the bound one, so a node that fell
/// back to an ephemeral port does not print a line that opens nothing.
pub fn firewall_line(snapshot: &LanSnapshot) -> String {
    format!("sudo ufw allow {}/udp", snapshot.port)
}

/// Start the node for `api`'s session, blocking until it exists or is refused.
///
/// **Not for the UI thread**: it probes the server (`OPTIONS /api/lan/status`)
/// and opens `lan.sqlite`. [`start_in_background`] is what the app calls; this
/// is the deterministic half the tests and the start thread use.
///
/// `Err(Unsupported)` from `nz-lan` means "this server has no such feature": no
/// directory was created, no socket bound, no thread spawned — the app then
/// shows no section at all (D17's *an older server hides its features*).
pub fn attach(api: &Arc<Api>, cache: &Cache) -> Option<Lan> {
    let session = session_key(api);
    if let Some(existing) = current() {
        let matches = node_cell()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .is_some_and(|node| node.session == session);
        if matches {
            return Some(existing);
        }
    }
    if answered_cell()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .contains(&session)
    {
        // This server already answered "no such feature": nothing to try.
        return None;
    }
    // Another session's node goes before this one starts: two nodes in one
    // process would both bind a port and announce themselves.
    shutdown_process();
    CLOSING.store(false, Ordering::Release);
    let configured = nz_lan::settings::load(cache);
    let config = LanConfig {
        data_dir: data_dir(cache),
        device_name: device_name(),
        platform: "desktop",
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        discovery: DiscoveryKind::Mdns,
        // The person's own port (`Réglages` → « Appareils à proximité »), not the
        // default: 45910 unless they changed it, and `0` = let the system pick
        // (the snapshot then prints the port really bound, which is what the
        // firewall line must name).
        port: configured.port,
    };
    match Lan::start(config, Arc::clone(api), cache.clone()) {
        Ok(lan) => {
            // The desktop is mains-powered and the app is open: the note's
            // *When* keeps discovery running for as long as the app is (the
            // phone's battery rules are the phone's). `Lan::start` already
            // starts in the foreground; saying it once keeps the intent here.
            lan.set_active(Activity::Foreground, true);
            let generation = GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
            if CLOSING.load(Ordering::Acquire) {
                // A quit landed while this start was in flight: the node must
                // not outlive it.
                lan.shutdown();
                return None;
            }
            *node_cell()
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(Node {
                session,
                generation,
                lan: lan.clone(),
            });
            debug!(
                port = lan.snapshot().port,
                "LAN : le nœud de cet appareil est démarré"
            );
            Some(lan)
        }
        Err(LanError::Unsupported) => {
            let mut answered = answered_cell()
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if !answered.contains(&session) {
                answered.push(session);
            }
            debug!("LAN : ce serveur n'a pas la fonction, rien ne démarre");
            None
        }
        Err(error) => {
            // A failure to start is not a verdict about the feature: the next
            // login tries again. The cause is in the log, never a screen that
            // cannot act on it.
            warn!(cause = %error.cause_fr(), "LAN : le nœud n'a pas démarré");
            None
        }
    }
}

/// Start the node off the UI thread, once per session.
///
/// The ONE call site is `wiring::engine::spawn_audio`, i.e. the login and the
/// restore (`AppState::start_audio`): the probe and the index read happen on a
/// `nz-lan-start` thread, never on the GPUI one, and the two peripherals that
/// wrap `nz-lan`'s seams pick the node up when it lands (see [`LanOpener`] and
/// [`LanDownloader`]) — so the audio and download paths never wait for it and
/// never have to be rebuilt.
pub fn start_in_background(api: Arc<Api>, cache: Cache) {
    let session = session_key(&api);
    if node_cell()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
        .is_some_and(|node| node.session == session)
    {
        return;
    }
    if answered_cell()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .contains(&session)
    {
        return;
    }
    {
        let mut starting = STARTING
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if starting.as_deref() == Some(session.as_str()) {
            return;
        }
        *starting = Some(session.clone());
    }
    let spawned = std::thread::Builder::new()
        .name("nz-lan-start".to_owned())
        .spawn(move || {
            let _ = attach(&api, &cache);
            let mut starting = STARTING
                .get_or_init(|| Mutex::new(None))
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if starting.as_deref() == Some(session.as_str()) {
                *starting = None;
            }
        });
    if let Err(error) = spawned {
        warn!(%error, "LAN : le fil de démarrage n'a pas pu être créé");
        *STARTING
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
    }
}

/// Where this device keeps `device.key` and `lan.sqlite`: `$data/neezer/lan/`.
fn data_dir(cache: &Cache) -> PathBuf {
    cache.data_dir().join("lan")
}

/// The name this device travels under (inside an authenticated connection and to
/// the server): the machine's own name, because that is what the person
/// recognises in « Mes appareils ».
///
/// It is deliberately NOT what mDNS announces — an announcement carries a random
/// instance name and `v=1` only (the note's *Discovery privacy*) — and it never
/// reaches a stranger: [`nz_lan::LanConfig::device_name`] is only ever read
/// inside a connection that proved who it is, or by the person's own server.
pub fn device_name() -> String {
    let raw = std::fs::read_to_string("/etc/hostname")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_default();
    let cleaned: String = raw
        .trim()
        .trim_end_matches(".local")
        .chars()
        .filter(|character| !character.is_control())
        .take(64)
        .collect();
    if cleaned.is_empty() {
        "Ordinateur".to_owned()
    } else {
        cleaned
    }
}

/// Stop the node and JOIN it: its runtime, its hashing thread and its discovery
/// daemon. Called from `NetRuntime::drop` after the engine and the downloads
/// worker, and before `nz-net` goes.
///
/// Idempotent, and a process that never started a node pays one atomic store.
pub fn shutdown_process() {
    CLOSING.store(true, Ordering::Release);
    let node = node_cell()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take();
    if let Some(node) = node {
        node.lan.shutdown();
    }
}

/// The counted bytes this wiring holds, over the node's own count.
///
/// The rule is bytes counted, never RSS: `Lan::memory()` is the node's own
/// figure (replicas, index, connections), and this file adds exactly one bounded
/// ring ([`SERVED_LIMIT`] rows). Measured in the tests.
pub fn memory() -> usize {
    let served = served_cell()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .iter()
        .map(|entry| std::mem::size_of::<Served>() + entry.track.capacity() + entry.name.capacity())
        .sum::<usize>();
    current().map(|lan| lan.memory()).unwrap_or(0) + served
}

// --- the opener: peer first --------------------------------------------------

/// The inner opener behind `dyn`, so a `PeerFirstOpener` can own it and the
/// app's own [`crate::wiring::engine::NetOpener`] keeps its shutdown.
struct SharedOpener(Arc<dyn StreamOpener>);

impl StreamOpener for SharedOpener {
    fn open(
        &self,
        id: &str,
        quality: nz_audio::Quality,
        start: Option<f64>,
        from_byte: Option<u64>,
        lane: Lane,
    ) -> Result<Box<dyn ByteStream>, SourceError> {
        self.0.open(id, quality, start, from_byte, lane)
    }

    fn ready(&self, id: &str, quality: nz_audio::Quality) -> Result<StreamReady, SourceError> {
        self.0.ready(id, quality)
    }

    fn local(&self, id: &str, quality: nz_audio::Quality) -> Option<PathBuf> {
        self.0.local(id, quality)
    }

    fn shutdown(&self) {
        self.0.shutdown();
    }
}

/// The app's opener, with LN11's [`PeerFirstOpener`] in front of it once a node
/// exists.
///
/// It is deliberately tolerant of a node that lands late: the engine is built
/// before `nz-lan` has finished probing (a login must not wait for a network
/// round trip), so `open`/`ready` ask [`current`] each time and build the adapter
/// on first use — one `Arc` clone, no lock held across the call. Before that,
/// every method is the inner opener's: **0 ms added**, which is the note's « No
/// peer has it » row.
pub struct LanOpener {
    inner: Arc<dyn StreamOpener>,
    adapter: Mutex<Option<(u64, Arc<PeerFirstOpener<SharedOpener>>)>>,
}

impl LanOpener {
    pub fn new(inner: Arc<dyn StreamOpener>) -> Self {
        Self {
            inner,
            adapter: Mutex::new(None),
        }
    }

    /// The peer-first adapter for the node in force, built once per node.
    fn adapter(&self) -> Option<(Arc<PeerFirstOpener<SharedOpener>>, Lan)> {
        let lan = current()?;
        let generation = generation();
        let mut slot = self
            .adapter
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some((built, adapter)) = slot.as_ref()
            && *built == generation
        {
            return Some((Arc::clone(adapter), lan));
        }
        let adapter = Arc::new(PeerFirstOpener::new(
            SharedOpener(Arc::clone(&self.inner)),
            lan.clone(),
        ));
        *slot = Some((generation, Arc::clone(&adapter)));
        Some((adapter, lan))
    }
}

impl StreamOpener for LanOpener {
    fn open(
        &self,
        id: &str,
        quality: nz_audio::Quality,
        start: Option<f64>,
        from_byte: Option<u64>,
        lane: Lane,
    ) -> Result<Box<dyn ByteStream>, SourceError> {
        let Some((adapter, lan)) = self.adapter() else {
            return self.inner.open(id, quality, start, from_byte, lane);
        };
        // The decision is what the opener is about to do, read in memory before
        // the body exists: it is what « En lecture » prints (LN11's correction:
        // there is no `Served` event to listen to).
        let decided = lan.decide(id, api_quality(quality), urgency(lane));
        let opened = adapter.open(id, quality, start, from_byte, lane);
        if let Ok(body) = &opened
            && let Source::Peer {
                peer,
                quality: wanted,
            } = decided
        {
            // The quality the BODY carries wins: a peer answers the asked rung
            // when it has it and its own copy otherwise (`ByteStream::quality`,
            // LN5), and the line must say what was really played.
            let served = body.quality().map(api_quality).unwrap_or(wanted);
            record_served(id, peer, served);
        }
        opened
    }

    fn ready(&self, id: &str, quality: nz_audio::Quality) -> Result<StreamReady, SourceError> {
        match self.adapter() {
            Some((adapter, _)) => adapter.ready(id, quality),
            None => self.inner.ready(id, quality),
        }
    }

    fn local(&self, id: &str, quality: nz_audio::Quality) -> Option<PathBuf> {
        match self.adapter() {
            Some((adapter, _)) => adapter.local(id, quality),
            None => self.inner.local(id, quality),
        }
    }

    fn shutdown(&self) {
        // The engine is closing: abandon what this opener waits for. The NODE is
        // stopped later, by `shutdown_process`, after `nz-bulk` and the engine
        // have joined their own threads.
        self.inner.shutdown();
    }
}

/// The app's downloader, with LN11's [`PeerFirstDownloader`] in front of it once
/// a node exists.
///
/// The worker reads its fetcher once per job (`nz-offline`'s loop), so a node
/// that lands after the login is picked up by the next track without a
/// reconfiguration. Before that, every byte is `nz-api`'s: the downloads keep
/// working exactly as they did.
pub struct LanDownloader {
    inner: Arc<dyn Downloader>,
    adapter: Mutex<Option<(u64, Arc<PeerFirstDownloader>)>>,
}

impl LanDownloader {
    pub fn new(inner: Arc<dyn Downloader>) -> Self {
        Self {
            inner,
            adapter: Mutex::new(None),
        }
    }

    fn adapter(&self) -> Option<Arc<PeerFirstDownloader>> {
        let lan = current()?;
        let generation = generation();
        let mut slot = self
            .adapter
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some((built, adapter)) = slot.as_ref()
            && *built == generation
        {
            return Some(Arc::clone(adapter));
        }
        let adapter = Arc::new(PeerFirstDownloader::new(Arc::clone(&self.inner), lan));
        *slot = Some((generation, Arc::clone(&adapter)));
        Some(adapter)
    }
}

impl Downloader for LanDownloader {
    /// The worker's gate asks the FETCHER whether a download can run (#352):
    /// this answers the peer-first adapter's own verdict — « the server is
    /// reachable **or** a connected peer holds files » — and falls back to the
    /// plain fetcher's while no node exists. Without this line the wrapper would
    /// keep the trait's `true` and a queue with no source at all would never say
    /// it is offline.
    fn reachable(&self) -> bool {
        match self.adapter() {
            Some(adapter) => adapter.reachable(),
            None => self.inner.reachable(),
        }
    }

    fn fetch(
        &self,
        track: &TrackId,
        quality: Quality,
        part: &std::path::Path,
        stop: &Stop,
        progress: &mut dyn Progress,
    ) -> Result<Fetched, Outcome> {
        match self.adapter() {
            Some(adapter) => adapter.fetch(track, quality, part, stop, progress),
            None => self.inner.fetch(track, quality, part, stop, progress),
        }
    }
}

/// The engine's opener, with the LAN in front of it (`wiring::engine`'s one
/// call).
pub fn opener(inner: Arc<dyn StreamOpener>) -> Arc<dyn StreamOpener> {
    Arc::new(LanOpener::new(inner))
}

/// The downloads worker's fetcher, with the LAN in front of it
/// (`wiring::offline`'s one call).
pub fn downloader(api: Arc<Api>, runtime: tokio::runtime::Handle) -> Arc<dyn Downloader> {
    Arc::new(LanDownloader::new(Arc::new(ApiDownloader::new(
        api, runtime,
    ))))
}

/// Which session's fetcher the worker already holds.
struct Installed {
    server: String,
    owner: String,
}

/// Point the worker at the peer-first fetcher, **once per session**.
///
/// `wiring::offline::attach` runs at every login and from every screen that
/// watches the worker, so this compares the session without allocating and
/// installs nothing when it is the one already in place: the alternative —
/// re-wrapping on every attach — cost `app::mouse`'s bench **+1.6 MB per 2 000
/// screen builds** (measured), i.e. a leak of a fetcher per navigation.
pub fn install_downloader(
    offline: &nz_offline::Offline,
    api: Arc<Api>,
    runtime: tokio::runtime::Handle,
) {
    static INSTALLED: OnceLock<Mutex<Option<Installed>>> = OnceLock::new();
    let server = api.base_url();
    let owner = api.owner().unwrap_or_default();
    {
        let cell = INSTALLED.get_or_init(|| Mutex::new(None));
        let mut installed = cell.lock().unwrap_or_else(|error| error.into_inner());
        if installed
            .as_ref()
            .is_some_and(|current| current.server == server && current.owner == owner)
        {
            return;
        }
        *installed = Some(Installed {
            server: server.to_owned(),
            owner,
        });
    }
    offline.configure_with(
        Arc::clone(&api),
        Arc::new(LanDownloader::new(Arc::new(ApiDownloader::new(
            api, runtime,
        )))),
    );
}

fn api_quality(quality: nz_audio::Quality) -> Quality {
    match quality {
        nz_audio::Quality::Flac => Quality::Flac,
        nz_audio::Quality::Opus320 => Quality::Opus320,
        nz_audio::Quality::Opus128 => Quality::Opus128,
        nz_audio::Quality::Opus64 => Quality::Opus64,
    }
}

#[cfg(test)]
fn audio_quality(quality: Quality) -> nz_audio::Quality {
    match quality {
        Quality::Opus64 => nz_audio::Quality::Opus64,
        Quality::Opus128 => nz_audio::Quality::Opus128,
        Quality::Opus320 | Quality::Opus256 | Quality::Opus192 => nz_audio::Quality::Opus320,
        Quality::Flac => nz_audio::Quality::Flac,
    }
}

/// Rule 1's ladder, in the LAN's words: the tapped track is `Play`, the next one
/// `Prefetch`, everything a screen or the worker asks for `Bulk`.
fn urgency(lane: Lane) -> Urgency {
    match lane {
        Lane::User => Urgency::Play,
        Lane::Prefetch => Urgency::Prefetch,
        Lane::Bulk => Urgency::Bulk,
    }
}

// --- « Lu depuis … » ---------------------------------------------------------

/// How many served tracks the ring remembers: a source line only matters for
/// what is on screen and the next few, and the map must never grow with the
/// queue (rule: memory counted, bounded).
pub const SERVED_LIMIT: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Served {
    track: String,
    /// The peer's display name at the time it served: the connection may be gone
    /// by the time a screen paints, and the line is about the bytes already
    /// playing.
    name: String,
    quality: Quality,
}

fn served_cell() -> &'static Mutex<VecDeque<Served>> {
    lock(&SERVED, VecDeque::new)
}

static SERVED: OnceLock<Mutex<VecDeque<Served>>> = OnceLock::new();

/// Remember that `peer` served `track` at `quality`.
///
/// Called by [`LanOpener::open`] on the engine's loader thread, before the first
/// byte: a `Mutex`, a `VecDeque` push and at most one name lookup in the
/// snapshot. The name is resolved HERE (and not when the screen paints) because
/// a peer may disconnect while its track keeps playing.
pub fn record_served(track: &str, peer: DevicePub, quality: Quality) {
    let name = snapshot()
        .and_then(|snapshot| {
            snapshot
                .peers
                .into_iter()
                .find(|view| view.device_pub == peer)
                .map(|view| view.name)
        })
        .unwrap_or_else(|| "un appareil".to_owned());
    push_served(track, name, quality);
}

/// The ring's one writer: the name is resolved by the caller (a peer's name, or
/// a test's), so the sentence's plumbing is testable without a node.
fn push_served(track: &str, name: String, quality: Quality) {
    let mut ring = served_cell()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    ring.retain(|entry| entry.track != track);
    if ring.len() == SERVED_LIMIT {
        ring.pop_front();
    }
    ring.push_back(Served {
        track: track.to_owned(),
        name,
        quality,
    });
}

/// A test's way in: the same ring, with the peer's name given.
#[cfg(test)]
fn record_served_as(track: &str, name: &str, quality: Quality) {
    push_served(track, name.to_owned(), quality);
}

/// What « En lecture » prints for `track`: « Lu depuis PC (FLAC) », or `None`
/// when this device, the play cache or the server served it.
pub fn source_line(track: &str) -> Option<String> {
    let ring = served_cell()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let entry = ring.iter().find(|entry| entry.track == track)?;
    Some(served_line(&entry.name, entry.quality))
}

/// The one spelling of the source line, so a test can hold the copy and a screen
/// cannot invent a second one.
pub fn served_line(name: &str, quality: Quality) -> String {
    format!("Lu depuis {name} ({})", nz_offline::quality_label(quality))
}

/// Forget what was served (a logout, or a test): the ring is this device's, and
/// another account must not inherit a line.
pub fn forget_served() {
    served_cell()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clear();
}

// --- « Sur vos appareils » and the playlist header ---------------------------

/// The sets the connected peers expose, cloned from memory (0 requests).
pub fn peer_sets() -> Vec<PeerSet> {
    current().map(|lan| lan.sets()).unwrap_or_default()
}

/// Queue every selected set into the downloads worker (one job per set).
///
/// The peer's completeness is re-checked by `nz-lan` against its live replica:
/// a set that lost a file since the screen drew it answers `NotFound`, and the
/// screen offers it again instead of half-failing (rule 3).
pub fn download_sets(keys: &[PeerSetKey]) -> Result<usize, LanError> {
    let Some(lan) = current() else {
        return Err(LanError::NotEnrolled);
    };
    let Some(offline) = nz_offline::Offline::get() else {
        return Err(LanError::Internal(
            "le travailleur hors ligne n'est pas démarré".to_owned(),
        ));
    };
    download_peer_sets(&lan, &offline, keys)
}

/// The name of the peer that has ALL of `tracks` complete, if one does — the
/// playlist header's « Disponible complète sur PC ».
///
/// One in-memory pass per call, never a request: call it when the page's tracks
/// change, not per frame.
pub fn best_peer_name(tracks: &[String]) -> Option<String> {
    let lan = current()?;
    let peer = lan.best_peer_for(tracks)?;
    let snapshot = lan.snapshot();
    snapshot
        .peers
        .into_iter()
        .find(|view| view.device_pub == peer)
        .map(|view| view.name)
}

/// The peer list row of `peer`, for a screen that has a `DevicePub`.
pub fn peer_name(snapshot: &LanSnapshot, peer: DevicePub) -> Option<String> {
    snapshot
        .peers
        .iter()
        .find(|view| view.device_pub == peer)
        .map(|view| view.name.clone())
}

/// Whether a peer with that display name is connected right now — the playlist
/// header's cheap "is the device that had it still here" check, without cloning
/// the peer list into a screen.
pub fn peer_named(name: &str) -> bool {
    snapshot().is_some_and(|snapshot| snapshot.peers.iter().any(|view| view.name == name))
}

/// « Clé valable jusqu'au 18 décembre 2026 », or the failure's cause.
///
/// One function for the sentence, so Réglages and a test cannot disagree about
/// it (rule 2: a state says what it is, in French, and a failure its cause).
pub fn key_line(snapshot: &LanSnapshot) -> String {
    if let Some(cause) = snapshot.key_error.as_deref() {
        return format!("Clé LAN : {cause}");
    }
    match snapshot.key_until {
        Some(until) => format!("Clé valable jusqu'au {}", french_date(until)),
        None => match snapshot.state {
            LanState::Off => "Clé LAN : l'échange est désactivé sur cet appareil".to_owned(),
            LanState::Unsupported => "Ce serveur ne connaît pas les appareils proches".to_owned(),
            _ => "Aucune clé LAN : l'enregistrement auprès du serveur n'a pas encore abouti"
                .to_owned(),
        },
    }
}

/// A unix timestamp as a French day: « 18 décembre 2026 ».
///
/// Hand-rolled on purpose: the app has no date crate, and the one thing the LAN
/// needs is a day a person reads (no locale, no time-of-day, no time zone
/// surprise — the credential's midnight is the server's, and the day is the same
/// either side of it). The civil-from-days algorithm is Howard Hinnant's
/// (`chrono`'s own derivation from `days_from_civil`).
pub fn french_date(unix: i64) -> String {
    const MONTHS: [&str; 12] = [
        "janvier",
        "février",
        "mars",
        "avril",
        "mai",
        "juin",
        "juillet",
        "août",
        "septembre",
        "octobre",
        "novembre",
        "décembre",
    ];
    let days = unix.div_euclid(86_400) + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{day} {}", MONTHS[(month - 1) as usize]) + &format!(" {year}")
}

/// The tolerated set with `quality` under `wanted`, refusing to leave it empty:
/// the note says « four checkboxes, at least one », and an empty set would mean
/// "never take anything from a peer" — a different feature from the one the
/// person turned on.
pub fn toggle_tolerated(tolerance: &Tolerance, quality: Quality, wanted: bool) -> Tolerance {
    let mut set = tolerance.0.clone();
    if wanted {
        set.insert(quality);
    } else {
        set.remove(&quality);
    }
    if set.is_empty() {
        set.insert(quality);
    }
    Tolerance(set)
}

/// What a peer list row says about the device beside its name: « mon appareil »
/// or the friend's name. One spelling for the peer list and for « Sur vos
/// appareils », so the same relation cannot read two ways.
pub fn relation_label(relation: &Relation) -> String {
    match relation {
        Relation::Own => "mon appareil".to_owned(),
        Relation::Friend { name } => format!("ami · {name}"),
    }
}

/// The two lines a transfer row prints: what is exchanged, and with whom.
pub fn transfer_line(view: &TransferView) -> String {
    let direction = match view.direction {
        Direction::Send => "envoyé à",
        Direction::Recv => "reçu de",
    };
    let title = if view.title.is_empty() {
        view.track.as_str()
    } else {
        view.title.as_str()
    };
    format!("{title} — {direction} {}", view.peer)
}

#[cfg(test)]
mod tests {
    //! The app's half of LN12, measured: the node starts off the UI thread at a
    //! login (or not at all), an older server starts nothing and shows nothing,
    //! the wrappers never cost a tap anything, the source line's copy is held
    //! here, and the quit joins `nz-lan` after the other two.
    //!
    //! **What these tests deliberately do NOT do**: enrol against a real server.
    //! A credential is signed by the server's LAN key, and the fake that signs
    //! one lives in `nz-lan/tests/service.rs` (LN10's, where enrolment, renewal
    //! and gossip are measured). Here the fake server answers the feature probe
    //! and refuses the enrolment, which is what proves the app's own behaviour:
    //! the node starts anyway (from cache: the trip works with the server
    //! unreachable), the failure reaches `key_error` in French, and the section
    //! prints it. The valid-key sentence is held by [`lan::key_line`]'s test.

    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use nz_audio::{ByteStream, Lane, SourceError, StreamOpener, StreamReady};

    use super::*;
    use crate::wiring::offline::tests::{Link, Scratch, TestServer, api_for, runtime, wait_for};

    /// The process-wide node and the thread census are one resource: these tests
    /// run in threads of one process (LN10's lesson), so they take turns.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static SERIAL: Mutex<()> = Mutex::new(());
        let guard = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
        shutdown_process();
        answered_cell()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
        forget_served();
        guard
    }

    /// Every thread of this process whose name `nz-lan` gives its own (`LAN`'s
    /// runtime and its worker, the hashing worker, the mDNS daemon).
    fn lan_threads() -> Vec<String> {
        let mut names = Vec::new();
        let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
            return names;
        };
        for task in tasks.flatten() {
            // A thread that exits between the read_dir and this read must not
            // hide the whole census (LN10's bug).
            let Ok(comm) = std::fs::read_to_string(task.path().join("comm")) else {
                continue;
            };
            let name = comm.trim().to_owned();
            if name.starts_with("nz-lan") {
                names.push(name);
            }
        }
        names.sort();
        names
    }

    /// A server that has the feature: `GET /api/lan/status` answers the payload
    /// LN1's route answers (a key it never signs with, an opaque subject), and
    /// `POST /api/lan/enroll` **fails with a cause** — a `5xx`, because a `404`
    /// there means "no such feature" to the node (it then stops, which is
    /// LN10's own test). See the module comment.
    const LAN_STATUS: &str = r#"{"enabled":true,"subject":"00112233445566778899aabbccddeeff",
        "keys":[{"kid":"56475aa75463474c",
                 "pub":"03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8"}],
        "credential_days":30,"revocation_serial":1}"#;

    fn a_lan_server(scratch: &Scratch) -> (TestServer, Arc<Api>) {
        let store = scratch.store();
        // `lan.port = 0`: an ephemeral UDP port, so a test never takes 45910 from
        // the app the person is running (and two runs never collide).
        store.kv().set("lan.port", &0u16).expect("the port setting");
        let server = TestServer::start(Link::new(None), HashMap::new());
        for _ in 0..8 {
            server.script(
                "status",
                crate::wiring::offline::tests::Script {
                    status: 200,
                    headers: Vec::new(),
                    body: LAN_STATUS.to_owned(),
                },
            );
        }
        for _ in 0..4 {
            server.script(
                "enroll",
                crate::wiring::offline::tests::Script {
                    status: 500,
                    headers: Vec::new(),
                    body: r#"{"error":"le serveur n'a pas pu signer","code":"internal"}"#
                        .to_owned(),
                },
            );
        }
        let api = api_for(&server, &store, scratch);
        (server, api)
    }

    // --- older server -------------------------------------------------------

    /// **LN12's first acceptance: an older server starts nothing.**
    ///
    /// A server without `/api/lan/status` answers `404`; `Lan::start` refuses
    /// before it creates a directory, binds a socket or spawns a thread. Measured
    /// here: `attach` answers `None`, the data directory does not exist, and the
    /// thread census names **0** `nz-lan` threads. `Réglages` then hides the
    /// section (`settings`' own test walks the strip).
    #[test]
    fn a_server_without_the_feature_starts_nothing_at_all() {
        let _guard = serial();
        let scratch = Scratch::new("lan-absent");
        let store = scratch.store();
        let server = TestServer::start(Link::new(None), HashMap::new());
        let api = api_for(&server, &store, &scratch);

        assert!(
            attach(&api, &store).is_none(),
            "no node for a server without the feature"
        );
        assert!(current().is_none());
        assert_eq!(lan_threads(), Vec::<String>::new(), "0 thread named nz-lan");
        assert!(
            std::fs::metadata(store.data_dir().join("lan")).is_err(),
            "nothing was created on disk: {:?}",
            store.data_dir().join("lan")
        );
        // ... and a second screen (or a second login) costs no second probe:
        // the answer is remembered for this session.
        let before = server.hits("status");
        assert!(attach(&api, &store).is_none());
        assert_eq!(
            server.hits("status"),
            before,
            "the refusal is remembered: 0 requests on a revisit"
        );
    }

    // --- a LAN-enabled server: the node starts, off the UI thread ------------

    /// **Login on a LAN-enabled server: the node runs, and the app can name its
    /// port.**
    ///
    /// The enrolment is refused (the fake signs nothing): the node keeps the
    /// cached identity — none — and says the cause in French, which is exactly
    /// what « Réglages » prints and what a trip with the server unreachable
    /// looks like. Measured: one `nz-lan` thread, an ephemeral port bound, and
    /// `key_error` set with a sentence.
    #[test]
    fn a_lan_server_gets_a_node_that_says_where_its_key_stands() {
        let _guard = serial();
        let scratch = Scratch::new("lan-present");
        let store = scratch.store();
        let (server, api) = a_lan_server(&scratch);

        let lan = attach(&api, &store).expect("the node started");
        assert!(
            wait_for(Duration::from_secs(20), || { !lan_threads().is_empty() }),
            "the nz-lan thread is running"
        );
        let snapshot = lan.snapshot();
        println!(
            "nœud : {:?}, port {}, clé {:?}, requêtes status={} enroll={}",
            snapshot.state,
            snapshot.port,
            snapshot.key_error,
            server.hits("status"),
            server.hits("enroll")
        );
        // The port the snapshot names is the SETTING until an endpoint exists:
        // this node never enrolled (the fake signs nothing), so it says 0 here —
        // "the system picks one" — and the app's default is 45910.
        assert_eq!(snapshot.port, 0, "an ephemeral port was asked for");
        assert_eq!(
            settings(Some(&store)).port,
            0,
            "and it is the device's setting"
        );
        // The enrolment fails against this fake: the driver retries on the next
        // daily tick, and the CAUSE is on the snapshot (rule 2).
        assert!(
            wait_for(Duration::from_secs(20), || {
                lan.snapshot().key_error.is_some()
            }),
            "the enrolment's failure is reported: {:?} (status={} enroll={})",
            lan.snapshot().key_error,
            server.hits("status"),
            server.hits("enroll")
        );
        let cause = lan.snapshot().key_error.expect("a cause");
        assert!(!cause.is_empty(), "the cause is a sentence: {cause}");
        // The settings are the device's own, reachable without a node:
        assert!(settings(Some(&store)).enabled);
        shutdown_process();
        assert!(
            wait_for(Duration::from_secs(10), || lan_threads().is_empty()),
            "shutdown joined every nz-lan thread: {:?}",
            lan_threads()
        );
    }

    // --- the quit's order ---------------------------------------------------

    /// **The app's quit joins the node LAST** (D13's shutdown order extended).
    ///
    /// `NetRuntime::drop` calls, in this order, `offline::shutdown_process`,
    /// `engine::shutdown_process`, then `lan::shutdown_process`. This test walks
    /// the same three calls with a real node and asserts what each one leaves
    /// behind: the node is still running when the other two return, and **0**
    /// `nz-lan` (and 0 `nz-lan-hash`) threads are left when `lan::shutdown_process`
    /// returns — the join, not a timer.
    #[test]
    fn the_quit_joins_the_node_after_the_worker_and_the_engine() {
        let _guard = serial();
        let scratch = Scratch::new("lan-close");
        let store = scratch.store();
        let (_server, api) = a_lan_server(&scratch);
        let _lan = attach(&api, &store).expect("the node started");
        assert!(
            wait_for(Duration::from_secs(20), || !lan_threads().is_empty()),
            "the node runs: {:?}",
            lan_threads()
        );
        let running = lan_threads();
        println!("fils du nœud avant la fermeture : {running:?}");

        // What a quit does first: the downloads worker, then the engine.
        crate::wiring::offline::shutdown_process();
        crate::wiring::engine::shutdown_process();
        assert!(
            !lan_threads().is_empty(),
            "the node is still up while the two others close: {:?}",
            lan_threads()
        );
        // ... and then the node, which is what this lot adds to that list.
        shutdown_process();
        let left = lan_threads();
        assert!(
            left.is_empty(),
            "0 nz-lan threads when the close returns: {left:?}"
        );
        assert!(current().is_none());
        // Idempotent: a second close (the tests' `Node` drop, a panic) is free.
        shutdown_process();
        assert!(lan_threads().is_empty());
    }

    // --- the wrappers cost nothing without a node ---------------------------

    /// An inner opener that counts what it was asked, so a test can say "the
    /// body came from here" without a network.
    struct CountingOpener(Arc<std::sync::atomic::AtomicUsize>);

    impl StreamOpener for CountingOpener {
        fn open(
            &self,
            _id: &str,
            _quality: nz_audio::Quality,
            _start: Option<f64>,
            _from_byte: Option<u64>,
            _lane: Lane,
        ) -> Result<Box<dyn ByteStream>, SourceError> {
            self.0.fetch_add(1, Ordering::AcqRel);
            Err(SourceError::Message("inner opener".to_owned()))
        }

        fn ready(
            &self,
            _id: &str,
            _quality: nz_audio::Quality,
        ) -> Result<StreamReady, SourceError> {
            Ok(StreamReady {
                ready: false,
                ranges: None,
                bytes: None,
                cached: false,
                offset: false,
                streaming: true,
            })
        }

        fn local(&self, _id: &str, _quality: nz_audio::Quality) -> Option<PathBuf> {
            None
        }
    }

    /// **No node: the opener is the inner one, 0 ms added** (the note's « No peer
    /// has it » row, app side). One `open` is one inner `open`, and nothing is
    /// recorded as served.
    #[test]
    fn without_a_node_the_opener_is_the_inner_one() {
        let _guard = serial();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let opener = opener(Arc::new(CountingOpener(Arc::clone(&calls))));
        let opened = opener.open(
            "3135556",
            nz_audio::Quality::Opus320,
            None,
            None,
            Lane::User,
        );
        assert!(opened.is_err(), "the inner opener's answer came back");
        assert_eq!(calls.load(Ordering::Acquire), 1, "exactly one inner open");
        assert_eq!(source_line("3135556"), None, "nothing was served by a peer");
        // `ready` is the inner one's too, so the engine's seek plan is unchanged.
        let ready = opener
            .ready("3135556", nz_audio::Quality::Opus320)
            .expect("ready");
        assert!(!ready.ready);
    }

    // --- « Lu depuis PC (FLAC) » --------------------------------------------

    /// **The source line's copy, and the ring that feeds it.**
    ///
    /// The exact sentence « Lu depuis PC (FLAC) » the lot's acceptance names, from
    /// the one function that builds it; the ring keeps the last 64 tracks and
    /// forgets older ones (a queue must not grow the UI's memory).
    #[test]
    fn the_source_line_names_the_peer_and_the_quality() {
        let _guard = serial();
        record_served_as("3135556", "PC", Quality::Flac);
        assert_eq!(
            source_line("3135556").as_deref(),
            Some("Lu depuis PC (FLAC)")
        );
        record_served_as("7000", "Téléphone", Quality::Opus320);
        assert_eq!(
            source_line("7000").as_deref(),
            Some("Lu depuis Téléphone (Opus 320)")
        );
        // The ring is bounded: 64 rows, oldest out first.
        for index in 0..(SERVED_LIMIT + 8) {
            record_served_as(&format!("t{index}"), "PC", Quality::Flac);
        }
        let len = served_cell()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .len();
        assert_eq!(
            len, SERVED_LIMIT,
            "the ring holds at most {SERVED_LIMIT} rows"
        );
        // The oldest entries went; the newest is there.
        assert_eq!(source_line("3135556"), None);
        assert!(source_line(&format!("t{}", SERVED_LIMIT + 7)).is_some());
        println!(
            "ligne de source : {:?} ; anneau : {len} entrées, {} octets comptés",
            source_line("t71"),
            memory() - current().map(|lan| lan.memory()).unwrap_or(0)
        );
        forget_served();
        assert_eq!(source_line("t71"), None);
    }

    // --- the settings and the sentences -------------------------------------

    #[test]
    fn the_firewall_line_names_the_port_really_bound() {
        let mut snapshot = LanSnapshot {
            port: 45_910,
            ..LanSnapshot::default()
        };
        assert_eq!(
            firewall_line(&snapshot),
            "sudo ufw allow 45910/udp",
            "the exact line the lot's acceptance asks the screen to copy"
        );
        // A node that fell back to an ephemeral port must not print a line that
        // opens nothing.
        snapshot.port = 38_462;
        snapshot.port_fallback = true;
        assert_eq!(firewall_line(&snapshot), "sudo ufw allow 38462/udp");
    }

    #[test]
    fn the_key_line_is_a_date_or_the_cause_never_an_empty_state() {
        // No node yet: `nz-lan` reports `NotEnrolled`, and the sentence says so.
        let mut snapshot = LanSnapshot {
            state: LanState::NotEnrolled,
            ..LanSnapshot::default()
        };
        assert_eq!(
            key_line(&snapshot),
            "Aucune clé LAN : l'enregistrement auprès du serveur n'a pas encore abouti"
        );
        snapshot.key_until = Some(1_800_000_000);
        assert_eq!(
            key_line(&snapshot),
            "Clé valable jusqu'au 15 janvier 2027",
            "the note's own sentence, with the day a person reads"
        );
        snapshot.key_error =
            Some("Impossible de renouveler la clé LAN : serveur injoignable".to_owned());
        assert_eq!(
            key_line(&snapshot),
            "Clé LAN : Impossible de renouveler la clé LAN : serveur injoignable",
            "a failure prints its CAUSE, not the old date"
        );
    }

    #[test]
    fn a_french_date_is_the_credential_window_s_day() {
        // 1 800 000 000 = 2027-01-15T08:00:00Z, and 1 801 353 600 = the note's
        // `not_after` (2027-01-31T00:00:00Z).
        assert_eq!(french_date(1_800_000_000), "15 janvier 2027");
        assert_eq!(french_date(1_801_353_600), "31 janvier 2027");
        // A credential's midnight in UTC is the right day in Paris too (UTC+1),
        // which is the point of not doing the time zone here.
        assert_eq!(french_date(1_797_552_000), "18 décembre 2026");
        // Leap day, to hold the civil-from-days arithmetic.
        assert_eq!(french_date(1_709_164_800), "29 février 2024");
    }

    #[test]
    fn the_tolerated_set_never_empties() {
        let tolerated = Tolerance::default_set();
        // Turning the last one off is refused by the screen; the helper keeps the
        // rule in one place either way.
        let one = Tolerance([Quality::Opus128].into_iter().collect());
        let off = toggle_tolerated(&one, Quality::Opus128, false);
        assert!(off.contains(Quality::Opus128), "at least one stays");
        let on = toggle_tolerated(&tolerated, Quality::Opus64, true);
        assert!(on.contains(Quality::Opus64));
        assert!(on.contains(Quality::Flac));
    }

    /// **The node's memory is counted bytes, and this file adds a bounded ring.**
    ///
    /// `Lan::memory()` is the node's own figure (LN10: 48 B per replicated item);
    /// the whole wiring adds the served ring only, at most
    /// [`SERVED_LIMIT`] × (a track id + a peer name), which is what is measured
    /// here — never the RSS of the process.
    #[test]
    fn the_wiring_memory_is_the_node_plus_one_bounded_ring() {
        let _guard = serial();
        let base = memory();
        for index in 0..SERVED_LIMIT {
            record_served_as(
                &format!("31355{index:02}"),
                "Ordinateur du salon",
                Quality::Flac,
            );
        }
        let full = memory();
        let ring = full - base;
        println!(
            "mémoire du câblage : {base} o sans anneau, {full} o avec {} lignes ({ring} o)",
            SERVED_LIMIT
        );
        assert_eq!(base, 0, "no node and an empty ring count nothing");
        // Measured: 82 B per row = 56 B of `Served` (two `String`s and a rung)
        // + 7 B of track id + 19 B of peer name, times the 64 the ring holds.
        assert_eq!(
            ring,
            SERVED_LIMIT
                * (std::mem::size_of::<Served>() + "3135500".len() + "Ordinateur du salon".len()),
            "the ring is exactly its rows, no growth with the queue"
        );
        forget_served();
        assert_eq!(memory(), 0);
    }

    /// **The worker's gate asks the fetcher (#352), so the wrapper must answer.**
    ///
    /// Without a node the answer is `ApiDownloader`'s own verdict — the server's
    /// link — so a fleet with no LAN behaves exactly as it did; with a node it is
    /// `PeerFirstDownloader`'s (« the server OR a connected peer »), which is what
    /// lets a trip queue a playlist from a peer.
    #[test]
    fn the_download_fetcher_answers_the_worker_s_reachability_question() {
        let _guard = serial();
        let scratch = Scratch::new("lan-reachable");
        let (server, api) = a_lan_server(&scratch);
        let runtime = runtime();
        let fetcher = downloader(Arc::clone(&api), runtime.handle().clone());
        assert!(fetcher.reachable(), "the stub server is up");
        api.set_online(false);
        assert!(
            !fetcher.reachable(),
            "no node and a dead link: the worker says Offline, as before"
        );
        drop(server);
        drop(runtime);
    }

    #[test]
    fn a_quality_travels_both_ways_across_the_engine_s_ladder() {
        for engine in [
            nz_audio::Quality::Flac,
            nz_audio::Quality::Opus320,
            nz_audio::Quality::Opus128,
            nz_audio::Quality::Opus64,
        ] {
            assert_eq!(audio_quality(api_quality(engine)), engine);
        }
        // The server's two rungs the desktop does not offer land on the nearest.
        assert_eq!(audio_quality(Quality::Opus256), nz_audio::Quality::Opus320);
    }
}
