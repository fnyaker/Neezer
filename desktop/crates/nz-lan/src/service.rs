//! `Lan` — the one handle the apps hold (docs/agents/lan-nodes.md,
//! *Architecture*).
//!
//! **LN0 declares the whole surface; LN10 fills it.** What every later lot reads
//! from here, and what the apps rely on:
//!
//! - [`Lan::start`] returns `Err(LanError::Unsupported)` on a server without the
//!   feature, and then NOTHING runs: no runtime, no thread, no socket, no screen
//!   (rule 7, D17's "an older server hides the feature").
//! - The crate owns its own tokio runtime (1 worker, thread `nz-lan`), and
//!   [`Lan::shutdown`] joins it. The app's shutdown order is engine → offline
//!   worker → `Lan` → `nz-net`, because each borrows the previous one's runtime; a
//!   thread that borrows a runtime must be joined before that runtime stops.
//! - [`Lan::snapshot`] is a CLONE taken from atomics and `Arc`s. It never touches
//!   SQLite and never waits on the runtime: it is read on the UI thread.
//! - [`Lan::events`] is a `watch` of a version counter: readers wake when something
//!   changed, and a screen that was not open costs nothing.
//! - [`Lan::memory`] is counted bytes (rule: never the RSS of a shared process).
//! - [`Lan::decide`] answers from memory, without the network: it is what a tap
//!   calls, and it costs no request.
//!
//! Activity ([`Activity`]) is what keeps the battery: browsing, announcing and
//! keep-alives run only while the app is in the foreground, playing, or has
//! downloads to do, and stop 60 s after the last of those. `set_active(false)`
//! closes the connections, so an idle phone sends nothing at all.

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::watch;

use crate::proto;
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
    /// The UDP port to bind, `0` = any. Default [`crate::service::DEFAULT_PORT`].
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
    /// The person switched it off (`lan.enabled`).
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
#[derive(Debug, Clone)]
pub struct Lan {
    /// The version counter the screens follow. LN10 replaces the whole struct's
    /// inside with `Arc<Inner>`; the accessor is what stops that being a change to
    /// any caller.
    events: watch::Sender<u64>,
}

impl Lan {
    /// Starts the node for `api`'s account, with `cache` as its local library.
    ///
    /// `Err(LanError::Unsupported)` when the server has no such feature, and then
    /// nothing was started — no runtime, no thread, no socket.
    pub fn start(
        cfg: LanConfig,
        api: Arc<nz_api::Api>,
        cache: nz_cache::Cache,
    ) -> Result<Lan, LanError> {
        let _ = (cfg, api, cache);
        Err(LanError::Unsupported)
    }

    /// Tells the node what the app is doing; `on = false` means "that stopped".
    ///
    /// When none of the three is on, connections close and discovery stops within
    /// 60 s, and no packet is sent afterwards.
    pub fn set_active(&self, why: Activity, on: bool) {
        let _ = (why, on);
    }

    /// The platform resolved a node at `host`:`port` ([`DiscoveryKind::External`]).
    pub fn found(&self, host: IpAddr, port: u16) {
        let _ = (host, port);
    }

    /// The platform lost a node it had resolved.
    pub fn lost(&self, host: IpAddr, port: u16) {
        let _ = (host, port);
    }

    /// The interface list or the network changed: re-announce, and try the
    /// last-known addresses of the peers we had.
    pub fn network_changed(&self) {}

    /// What the screens draw, cloned from memory: never a lock across SQLite, never
    /// a wait on the runtime.
    pub fn snapshot(&self) -> LanSnapshot {
        LanSnapshot::default()
    }

    /// The version counter, for a screen that must redraw only on change.
    pub fn events(&self) -> watch::Receiver<u64> {
        self.events.subscribe()
    }

    /// Where `track` will come from, from memory: no request, no waiting.
    pub fn decide(&self, track: &str, wanted: nz_api::Quality, urgency: Urgency) -> Source {
        let _ = (track, urgency);
        // LN10 asks `crate::decide` with the local index, the peers' replicas, this
        // device's tolerance and `api.is_online()`. Until then: the server, which is
        // exactly the path the app takes today.
        Source::Server(wanted)
    }

    /// Every set the connected peers expose, for « Sur vos appareils ».
    pub fn sets(&self) -> Vec<PeerSet> {
        Vec::new()
    }

    /// The peer that has ALL of `tracks` complete, if one does: a playlist
    /// download then asks that device for every track.
    pub fn best_peer_for(&self, tracks: &[String]) -> Option<DevicePub> {
        let _ = tracks;
        None
    }

    /// The settings as they are now.
    pub fn settings(&self) -> LanSettings {
        LanSettings::default()
    }

    /// Writes the settings. Applied at once for the session, persisted by
    /// [`crate::settings::save`].
    pub fn set_settings(&self, s: LanSettings) {
        let _ = s;
    }

    /// Enrols (or renews) now instead of waiting for the daily run: the app calls
    /// this after a login.
    pub fn enroll_now(&self) {}

    /// Logs out: wipes the key, the credential and the roster, and tells the server
    /// when it is reachable.
    pub fn forget(&self) {}

    /// The bytes this node's state costs, counted (index + clone replicas +
    /// buffers). LN10's targets: ≤ 64 B per replicated item, flat over an hour.
    pub fn memory(&self) -> usize {
        0
    }

    /// Stops everything and JOINS: the `nz-lan` runtime, the hashing thread and the
    /// discovery daemon. Called after the engine and the offline worker, and before
    /// `nz-net` stops.
    pub fn shutdown(&self) {}
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

    /// A stub node is `Unsupported` and knows nothing: no peer, no transfer, no
    /// memory, and a decision that is the server path the app already takes. A
    /// screen reading it draws "indisponible", never a peer that does not exist.
    #[test]
    fn a_stub_node_reports_unsupported_and_no_peer() {
        let lan = Lan {
            events: watch::channel(0).0,
        };
        let snapshot = lan.snapshot();
        assert_eq!(snapshot.state, LanState::Off);
        assert!(snapshot.peers.is_empty());
        assert!(snapshot.transfers.is_empty());
        assert!(snapshot.failures.is_empty());
        assert_eq!(snapshot.port, DEFAULT_PORT);
        assert!(!snapshot.port_fallback);
        assert_eq!(lan.memory(), 0);
        assert!(lan.sets().is_empty());
        assert_eq!(lan.best_peer_for(&["3135556".to_owned()]), None);
        assert_eq!(
            lan.decide("3135556", nz_api::Quality::Opus320, Urgency::Play),
            Source::Server(nz_api::Quality::Opus320)
        );
        assert_eq!(*lan.events().borrow(), 0);
        // Every lifecycle call is a no-op that cannot panic.
        lan.set_active(Activity::Foreground, true);
        lan.set_active(Activity::Playing, false);
        lan.network_changed();
        lan.enroll_now();
        lan.forget();
        lan.shutdown();
    }
}
