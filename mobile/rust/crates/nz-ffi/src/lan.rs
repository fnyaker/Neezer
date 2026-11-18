//! The LAN exchange at the FFI boundary: one object (`LanApi`) and one listener,
//! screen-shaped (docs/agents/lan-nodes.md, *Mobile FFI*).
//!
//! Owned by **LN13**: LN0 declared the whole surface, so the Kotlin side compiles
//! against it, and this lot fills the bodies over `nz_core::lan::LanCore`.
//!
//! Three mobile.md rules decide the shape:
//!
//! - **Reads that are a clone of memory are synchronous**: `snapshot()`,
//!   `settings()`, `peer_sets()` and `advertised_port()` answer in one frame, so
//!   a screen never waits for the LAN to paint.
//! - **Events fire on CHANGE only.** `subscribe` takes a [`LanListener`], and the
//!   core calls `changed(version)` when something moved — never once per second,
//!   and never while holding a lock. Kotlin hands it to a `Flow` and returns.
//! - **Every failure has a French sentence** (rule 3): a LAN refusal crosses as
//!   [`NzError::Unsupported`] when the server has no such feature (rule 7: the
//!   screens then hide the section) and as [`NzError::Invalid`] carrying the
//!   core's own sentence otherwise, which `nz_error_text` prints verbatim.
//!
//! # Two additions this lot had to make, and why
//!
//! 1. **`LanApi::start()`** (and `stop()`): the node needs the session's client,
//!    and `Lan::start` probes the server when it runs outside a runtime — a
//!    blocking request on Kotlin's main thread is what mobile.md forbids. An
//!    `async fn` puts it on the runtime the FFI already owns. LN14 calls `start()`
//!    when its NSD setup runs; every async method of this file also starts the
//!    node on first use, so a screen that only reads `devices()` or `friends()`
//!    still gets a node.
//! 2. **`memory_bytes()`**: the counted bytes of the node's state (worker rule 4
//!    — never an RSS), for Aide's diagnostic and for this lot's own measure.
//!
//! The object stays stateless: `NzCore::lan()` builds a fresh `LanApi` per call
//! (LN0's signature), so everything lives in `nz_core::lan`'s ONE process node,
//! exactly like `offline()`'s worker.
//!
//! `lan()` lives in ITS OWN `impl NzCore` block, in this file, which is what lets
//! UniFFI merge it into the one Kotlin `NzCore` class without any lot editing
//! `core.rs` — the same rule every feature lot follows.

use std::sync::Arc;

use nz_core::core::Core;
use nz_core::lan::{self, ChangeSink, NodeSnapshot, NodeState};

use crate::core::NzCore;
use crate::errors::NzError;

/// The state of the node, in one word a `when` can switch on.
#[derive(uniffi::Enum, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LanStateFfi {
    /// Switched off in Réglages, or never started (no session yet).
    #[default]
    Off,
    /// The server has no such feature: no screen and no setting mentions it.
    Unsupported,
    /// No credential yet.
    NotEnrolled,
    /// Enrolled, looking for peers.
    Searching,
    /// Enrolled, with this many authenticated peers.
    Connected { peers: u32 },
}

/// Whether a transfer leaves this device or comes in.
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanDirectionFfi {
    Send,
    Recv,
}

/// Rule 1's priority, as the transfer row shows it.
#[derive(uniffi::Enum, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LanUrgencyFfi {
    Play,
    #[default]
    Prefetch,
    Bulk,
}

/// One authenticated peer, as « Appareils à proximité » lists it.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct LanPeerFfi {
    /// The device's public key, hex — an opaque identifier, never a name.
    pub device_pub: String,
    /// The name the peer sent in `Hello`, inside the authenticated connection.
    pub name: String,
    /// `true` for one of my own devices, `false` for a trusted friend's.
    pub own: bool,
    pub platform: String,
    pub rtt_ms: u32,
    pub tracks: u32,
    pub sets: u32,
}

/// One transfer in flight, with its byte count.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct LanTransferFfi {
    pub track: String,
    pub title: String,
    pub peer: String,
    pub direction: LanDirectionFfi,
    pub bytes: u64,
    pub total: u64,
    pub urgency: LanUrgencyFfi,
}

/// One failure the screen must show with its cause (rule 2: never just a count).
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct LanFailureFfi {
    /// What failed, in French (« Télécharger Route »).
    pub what: String,
    /// Why, in French, already worded by the core.
    pub cause: String,
}

/// What the screens read in one call.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct LanSnapshotFfi {
    pub state: LanStateFfi,
    /// When the credential ends (unix seconds), for « Clé valable jusqu'au … ».
    pub key_until: Option<i64>,
    /// Why the last renewal failed, in French.
    pub key_error: Option<String>,
    pub peers: Vec<LanPeerFfi>,
    pub transfers: Vec<LanTransferFfi>,
    pub failures: Vec<LanFailureFfi>,
    /// The UDP port actually bound (`0` before the node starts).
    pub port: u16,
    /// Whether the preferred port was taken (the firewall line then does not match).
    pub port_fallback: bool,
}

/// The per-device settings Réglages edits.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct LanSettingsFfi {
    /// « Échanger avec mes appareils à proximité ».
    pub enabled: bool,
    /// « Partager mes sons ».
    pub serve: bool,
    /// « Qualités tolérées », in the server's own spelling (`"FLAC"`, `"OPUS_320"`…).
    pub tolerated: Vec<String>,
    /// The UDP port, `0` = any.
    pub port: u16,
}

/// One enrolled device of the account.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct LanDeviceFfi {
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
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct LanPlaylistFfi {
    pub id: String,
    pub title: String,
}

/// One trusted friend and what they may see.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct LanFriendFfi {
    /// The ACCOUNT's id: a UUID string, like every other account id the app
    /// holds. The LAN's own opaque `subject` is a separate field, never this.
    pub user_id: String,
    pub name: String,
    /// Everything I have locally.
    pub all: bool,
    pub playlists: Vec<LanPlaylistFfi>,
}

/// The two directions of trust.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct LanFriendsFfi {
    pub shares_to: Vec<LanFriendFfi>,
    pub trusted_by: Vec<LanFriendFfi>,
}

/// One set a peer exposes, for « Sur vos appareils ».
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct PeerSetFfi {
    /// The peer's public key, hex: the first half of the `"<peer>:<kind>:<ident>"`
    /// key `download_sets` takes.
    pub peer: String,
    pub peer_name: String,
    /// `true` for one of my own devices.
    pub own: bool,
    pub kind: String,
    pub ident: String,
    pub title: String,
    pub cover: Option<String>,
    pub tracks: u32,
    pub local: u32,
    /// Every member is on that peer: the whole set can come from it.
    pub complete: bool,
    pub bytes: u64,
}

/// The node, as Kotlin holds it.
///
/// It carries the core (not a node): `NzCore::lan()` builds a fresh object per
/// call, and the process's ONE node lives in `nz_core::lan`'s slot, so every
/// object of this type talks to the same node and a listener registered through
/// one is heard by all.
#[derive(uniffi::Object)]
pub struct LanApi {
    core: Arc<Core>,
}

impl LanApi {
    pub(crate) fn open(core: Arc<Core>) -> Arc<LanApi> {
        Arc::new(LanApi { core })
    }

    /// The node, starting it if this is the first call and the person has a
    /// session. Callers that only read memory (`snapshot`, `settings`) do not use
    /// this: a screen paints before the node exists.
    async fn node(&self) -> Result<Arc<lan::LanCore>, NzError> {
        if let Some(node) = lan::get() {
            return Ok(node);
        }
        lan::attach(&self.core).await.map_err(lan_error)
    }
}

/// What the node tells Kotlin, on change only.
///
/// `callback_interface` and not `export(with_foreign)`: this crate's three other
/// listeners (`SessionListener`, `OfflineListener`, `PlayerListener`) are
/// declared the same way, and `with_foreign` on a TRAIT does not give
/// `Box<dyn LanListener>` a `TypeId` in uniffi 0.32.2 — measured on this crate,
/// 2 × E0277. One spelling for one pattern.
#[uniffi::export(callback_interface)]
pub trait LanListener: Send + Sync {
    /// A version counter that only grows: a screen redraws on a new value and
    /// does nothing on the same one.
    fn changed(&self, version: u64);
}

/// The listener, behind the core's own `ChangeSink`: the core holds no lock while
/// it calls this, and the Kotlin side hands the value to a `Flow` and returns.
struct ListenerAdapter(Arc<dyn LanListener>);

impl ChangeSink for ListenerAdapter {
    fn changed(&self, version: u64) {
        self.0.changed(version);
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl LanApi {
    /// Starts the node for this session: enrols in the background, binds the UDP
    /// port and lets the platform's discovery feed find peers.
    ///
    /// Idempotent, and answers at once (`Lan::start` probes the server from its
    /// own driver). LN14 calls this when the app comes to the foreground; the
    /// async methods below start the node on first use too.
    pub async fn start(&self) -> Result<(), NzError> {
        self.node().await.map(|_| ())
    }

    /// Stops the node and JOINS its threads, the listener's watcher and every
    /// queued download (worker rule 6). Idempotent; Android kills the process
    /// otherwise, so nothing correct depends on it running.
    pub fn stop(&self) {
        lan::shutdown();
    }

    /// The whole state, cloned from memory: synchronous, so the screen paints in
    /// one frame.
    pub fn snapshot(&self) -> LanSnapshotFfi {
        match lan::get() {
            Some(node) => node.snapshot().into(),
            None => LanSnapshotFfi {
                // A server without the feature says so before any node exists, so
                // the section is never drawn for it (rule 7).
                state: if lan::unsupported() {
                    LanStateFfi::Unsupported
                } else {
                    LanStateFfi::Off
                },
                ..LanSnapshotFfi::default()
            },
        }
    }

    /// Registers the one listener that hears about changes. A listener registered
    /// before the node exists is kept and gets every change from the start, so a
    /// screen can subscribe once, in `onViewCreated`.
    pub fn subscribe(&self, l: Box<dyn LanListener>) {
        lan::subscribe(Arc::new(ListenerAdapter(Arc::from(l))));
    }

    /// The platform's NSD resolved a node at `host`:`port`.
    pub fn found(&self, host: String, port: u16) {
        if let Some(node) = lan::get() {
            node.found(&host, port);
        }
    }

    /// The platform lost a node it had resolved.
    pub fn lost(&self, host: String, port: u16) {
        if let Some(node) = lan::get() {
            node.lost(&host, port);
        }
    }

    /// The network moved (`ConnectivityManager`): re-announce and try the
    /// last-known addresses, which is the reconnect path when multicast is
    /// filtered but unicast passes.
    pub fn network_changed(&self) {
        if let Some(node) = lan::get() {
            node.network_changed();
        }
    }

    /// What the app is doing, for the battery rules: browsing and announcing only
    /// run while one of the three is true (and 60 s after).
    pub fn set_active(&self, foreground: bool, playing: bool, downloading: bool) {
        if let Some(node) = lan::get() {
            node.set_active(foreground, playing, downloading);
        }
    }

    /// The settings as they are now — the STORED ones before the node exists, so
    /// Réglages works on a device that never enrolled.
    pub fn settings(&self) -> LanSettingsFfi {
        match lan::get() {
            Some(node) => node.settings().into(),
            None => lan::stored_settings(&self.core).into(),
        }
    }

    /// Writes them; applied at once for the session and persisted by the core.
    pub fn set_settings(&self, s: LanSettingsFfi) {
        match lan::get() {
            Some(node) => node.set_settings(&s.into()),
            None => lan::save_settings(&self.core, &s.into()),
        }
    }

    /// Every set the connected peers expose.
    pub fn peer_sets(&self) -> Vec<PeerSetFfi> {
        lan::get()
            .map(|node| node.peer_sets().into_iter().map(PeerSetFfi::from).collect())
            .unwrap_or_default()
    }

    /// Queues the sets named `"<peer>:<kind>:<ident>"` as ordinary downloads.
    ///
    /// It answers in one frame: the keys are validated here and the transfer runs
    /// in the background, so a tap never waits on a peer (rule 4). What the
    /// replica already knows is refused HERE, with its cause.
    pub fn download_sets(&self, keys: Vec<String>) -> Result<(), NzError> {
        let node = lan::get().ok_or_else(|| lan_error(nz_lan::LanError::NotEnrolled))?;
        node.download_sets(&keys).map(|_| ()).map_err(lan_error)
    }

    /// The account's enrolled devices (a `User` request: the screen is open).
    pub async fn devices(&self) -> Result<Vec<LanDeviceFfi>, NzError> {
        let node = self.node().await?;
        Ok(node
            .devices()
            .await?
            .into_iter()
            .map(LanDeviceFfi::from)
            .collect())
    }

    /// Revokes one device on the server; every peer refuses it at its next sync.
    pub async fn revoke(&self, id: String) -> Result<(), NzError> {
        let node = self.node().await?;
        node.revoke(&id).await.map_err(NzError::from)
    }

    /// Renames one device, 1–64 characters.
    pub async fn rename(&self, id: String, name: String) -> Result<(), NzError> {
        let node = self.node().await?;
        node.rename(&id, &name).await.map_err(NzError::from)
    }

    /// The two directions of trust.
    pub async fn friends(&self) -> Result<LanFriendsFfi, NzError> {
        let node = self.node().await?;
        Ok(node.friends().await?.into())
    }

    /// Adds a trusted friend by exact account name.
    pub async fn add_friend(&self, name: String) -> Result<(), NzError> {
        let node = self.node().await?;
        node.add_friend(&name).await.map_err(NzError::from)
    }

    /// What one friend may fetch: everything, or the listed playlists.
    pub async fn set_friend_scope(
        &self,
        user_id: String,
        all: bool,
        playlists: Vec<String>,
    ) -> Result<(), NzError> {
        let node = self.node().await?;
        node.set_friend_scope(&user_id, all, &playlists)
            .await
            .map_err(NzError::from)
    }

    /// Stops sharing with one friend; their devices stop at their next roster sync.
    pub async fn remove_friend(&self, user_id: String) -> Result<(), NzError> {
        let node = self.node().await?;
        node.remove_friend(&user_id).await.map_err(NzError::from)
    }

    /// The port Kotlin registers with `NsdManager` — the one really bound, which is
    /// the ephemeral fallback when 45910 was taken.
    pub fn advertised_port(&self) -> u16 {
        lan::get()
            .map(|node| node.advertised_port())
            .unwrap_or_default()
    }

    /// The counted bytes of the node's state (worker rule 4: never an RSS).
    pub fn memory_bytes(&self) -> u64 {
        lan::get().map(|node| node.memory() as u64).unwrap_or(0)
    }
}

/// A LAN failure as the one error the screens know: the feature's name when the
/// server cannot do it (rule 7), the core's own French sentence otherwise.
fn lan_error(error: nz_lan::LanError) -> NzError {
    match error {
        nz_lan::LanError::Unsupported | nz_lan::LanError::Disabled => NzError::Unsupported {
            feature: lan::FEATURE.to_owned(),
        },
        other => NzError::Invalid {
            detail: other.cause_fr(),
        },
    }
}

// --- the conversions: the core's values, the records Kotlin sees -------------

impl From<NodeSnapshot> for LanSnapshotFfi {
    fn from(snapshot: NodeSnapshot) -> Self {
        LanSnapshotFfi {
            state: snapshot.state.into(),
            key_until: snapshot.key_until,
            key_error: snapshot.key_error,
            peers: snapshot.peers.into_iter().map(LanPeerFfi::from).collect(),
            transfers: snapshot
                .transfers
                .into_iter()
                .map(LanTransferFfi::from)
                .collect(),
            failures: snapshot
                .failures
                .into_iter()
                .map(|failure| LanFailureFfi {
                    what: failure.what,
                    cause: failure.cause,
                })
                .collect(),
            port: snapshot.port,
            port_fallback: snapshot.port_fallback,
        }
    }
}

impl From<NodeState> for LanStateFfi {
    fn from(state: NodeState) -> Self {
        match state {
            NodeState::Off => LanStateFfi::Off,
            NodeState::Unsupported => LanStateFfi::Unsupported,
            NodeState::NotEnrolled => LanStateFfi::NotEnrolled,
            NodeState::Searching => LanStateFfi::Searching,
            NodeState::Connected(peers) => LanStateFfi::Connected { peers },
        }
    }
}

impl From<lan::NodePeer> for LanPeerFfi {
    fn from(peer: lan::NodePeer) -> Self {
        LanPeerFfi {
            device_pub: peer.device_pub,
            name: peer.name,
            own: peer.own,
            platform: peer.platform,
            rtt_ms: peer.rtt_ms,
            tracks: peer.tracks,
            sets: peer.sets,
        }
    }
}

impl From<lan::NodeTransfer> for LanTransferFfi {
    fn from(transfer: lan::NodeTransfer) -> Self {
        LanTransferFfi {
            track: transfer.track,
            title: transfer.title,
            peer: transfer.peer,
            direction: if transfer.sending {
                LanDirectionFfi::Send
            } else {
                LanDirectionFfi::Recv
            },
            bytes: transfer.bytes,
            total: transfer.total,
            urgency: match transfer.urgency.as_str() {
                "play" => LanUrgencyFfi::Play,
                "bulk" => LanUrgencyFfi::Bulk,
                _ => LanUrgencyFfi::Prefetch,
            },
        }
    }
}

impl From<LanSettingsFfi> for lan::NodeSettings {
    fn from(settings: LanSettingsFfi) -> Self {
        lan::NodeSettings {
            enabled: settings.enabled,
            serve: settings.serve,
            tolerated: settings.tolerated,
            port: settings.port,
        }
    }
}

impl From<lan::NodeSettings> for LanSettingsFfi {
    fn from(settings: lan::NodeSettings) -> Self {
        LanSettingsFfi {
            enabled: settings.enabled,
            serve: settings.serve,
            tolerated: settings.tolerated,
            port: settings.port,
        }
    }
}

impl From<lan::NodeDevice> for LanDeviceFfi {
    fn from(device: lan::NodeDevice) -> Self {
        LanDeviceFfi {
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

impl From<lan::NodeFriend> for LanFriendFfi {
    fn from(friend: lan::NodeFriend) -> Self {
        LanFriendFfi {
            user_id: friend.user_id,
            name: friend.name,
            all: friend.all,
            playlists: friend
                .playlists
                .into_iter()
                .map(|playlist| LanPlaylistFfi {
                    id: playlist.id,
                    title: playlist.title,
                })
                .collect(),
        }
    }
}

impl From<lan::NodeFriends> for LanFriendsFfi {
    fn from(friends: lan::NodeFriends) -> Self {
        LanFriendsFfi {
            shares_to: friends
                .shares_to
                .into_iter()
                .map(LanFriendFfi::from)
                .collect(),
            trusted_by: friends
                .trusted_by
                .into_iter()
                .map(LanFriendFfi::from)
                .collect(),
        }
    }
}

impl From<lan::NodeSet> for PeerSetFfi {
    fn from(row: lan::NodeSet) -> Self {
        PeerSetFfi {
            peer: row.peer,
            peer_name: row.peer_name,
            own: row.own,
            kind: row.kind,
            ident: row.ident,
            title: row.title,
            cover: row.cover,
            tracks: row.tracks,
            local: row.local,
            complete: row.complete,
            bytes: row.bytes,
        }
    }
}

/// `NzCore::lan()` lives here, not in `core.rs`: one `impl NzCore` block per file
/// is how UniFFI accepts a new method without a shared file being edited.
///
/// The object carries the core only — the process's ONE node is `nz_core::lan`'s
/// slot — so two calls answer two handles onto the same node and the same
/// listener list, which is what makes `lan()` safe to call from any screen.
#[uniffi::export]
impl NzCore {
    /// The LAN node of this process.
    pub fn lan(&self) -> Arc<LanApi> {
        LanApi::open(Arc::clone(self.core()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::nz_error_text;

    /// The stub's promise, kept now that the bodies exist: before a session and a
    /// node, every read answers the empty truth in ONE frame and every write that
    /// needs a node names the cause instead of panicking. Nothing here opens a
    /// socket, a thread or a directory.
    #[test]
    fn before_a_node_every_read_is_empty_and_every_write_has_a_cause() {
        let dir = std::env::temp_dir().join(format!("nz-ffi-lan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("data")).expect("le dossier");
        std::fs::create_dir_all(dir.join("cache")).expect("le cache");
        let core = Arc::new(
            Core::new(nz_core::core::CoreConfig {
                data_dir: dir.join("data"),
                cache_dir: dir.join("cache"),
                app_version: "0.0.0-test".to_owned(),
                version_code: 1,
                sdk_int: 35,
                device_name: "Téléphone".to_owned(),
            })
            .expect("un noyau"),
        );
        let api = LanApi::open(Arc::clone(&core));

        let snapshot = api.snapshot();
        assert_eq!(snapshot.state, LanStateFfi::Off);
        assert!(snapshot.peers.is_empty());
        assert_eq!(snapshot.port, 0);
        assert!(api.peer_sets().is_empty());
        assert_eq!(api.advertised_port(), 0);
        assert_eq!(api.memory_bytes(), 0);

        // The stored settings, not a made-up default: Réglages works before the
        // node exists.
        let stored = api.settings();
        assert!(stored.enabled, "l'échange est actif par défaut");
        assert_eq!(stored.tolerated.len(), 3);

        // A write reaches the store even without a node…
        let mut wanted = stored;
        wanted.serve = false;
        api.set_settings(wanted);
        assert!(!api.settings().serve);

        // …and a transfer refuses with a French cause, never a panic.
        match api.download_sets(vec!["aa:playlist:42".into()]) {
            Err(error) => {
                let text = nz_error_text(&error);
                assert!(
                    text.contains("inscrit") || text.contains("proches"),
                    "{text}"
                );
            }
            Ok(()) => panic!("aucun nœud ne peut mettre un ensemble en file"),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A listener is a plain trait object behind the core's `ChangeSink`: the
    /// Kotlin side registers one and the core pushes a version, never a record
    /// (records are pulled).
    #[test]
    fn a_listener_hears_a_version() {
        use std::sync::atomic::{AtomicU64, Ordering};

        struct Counter(Arc<AtomicU64>);
        impl LanListener for Counter {
            fn changed(&self, version: u64) {
                self.0.store(version, Ordering::SeqCst);
            }
        }

        let seen = Arc::new(AtomicU64::new(0));
        let listener: Box<dyn LanListener> = Box::new(Counter(Arc::clone(&seen)));
        let sink = ListenerAdapter(Arc::from(listener));
        sink.changed(7);
        assert_eq!(seen.load(Ordering::SeqCst), 7);
    }

    /// Rule 7's two refusals carry different sentences, and neither is a code.
    #[test]
    fn a_refusal_is_a_sentence() {
        let unsupported = lan_error(nz_lan::LanError::Unsupported);
        assert!(matches!(unsupported, NzError::Unsupported { .. }));
        assert!(nz_error_text(&unsupported).contains("proches"));
        let gone = lan_error(nz_lan::LanError::PeerGone);
        assert_eq!(nz_error_text(&gone), "L'appareil ne répond plus.");
    }
}
