//! The LAN exchange at the FFI boundary: one object (`LanApi`) and one listener,
//! screen-shaped (docs/agents/lan-nodes.md, *Mobile FFI*).
//!
//! Owned by **LN13**: LN0 declared the whole surface, so the Kotlin side compiles
//! against it and LN13 only fills the bodies. Two mobile.md rules decide its shape:
//!
//! - **Reads are synchronous** when they are a clone of memory: `snapshot()`,
//!   `settings()`, `peer_sets()` and `advertised_port()` answer in one frame, so a
//!   screen never waits for the LAN to paint.
//! - **Events fire on CHANGE only.** `subscribe` takes a [`LanListener`], and the
//!   core calls `changed(version)` when something moved — never once per second,
//!   and never while holding a lock. Kotlin hands it to a `Flow` and returns.
//!
//! Every failure is an [`NzError`], whose French sentence `nz_error_text` already
//! knows, so `LoadFailed` can name the cause (rule 3).
//!
//! The `lan()` method lives in ITS OWN `impl NzCore` block, in this file, which is
//! what lets UniFFI merge it into the one Kotlin `NzCore` class without any lot
//! editing `core.rs` — the same rule every feature lot follows.

use std::sync::Arc;

use crate::core::NzCore;
use crate::errors::NzError;

/// The state of the node, in one word a `when` can switch on.
#[derive(uniffi::Enum, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LanStateFfi {
    /// Switched off in Réglages.
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
    /// The UDP port actually bound.
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
    pub user_id: u32,
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
/// **LN13** puts `nz_core::lan::LanCore` behind it; LN0's object has no field so
/// that the Kotlin build is green before the core exists.
#[derive(uniffi::Object)]
pub struct LanApi;

/// What the node tells Kotlin, on change only.
///
/// `callback_interface` and not `export(with_foreign)`: this crate's three other
/// listeners (`SessionListener`, `OfflineListener`, `PlayerListener`) are
/// declared the same way, and `with_foreign` on a TRAIT does not give
/// `Box<dyn LanListener>` a `TypeId` in uniffi 0.32.2 — measured on this lot,
/// 2 × E0277. One spelling for one pattern.
#[uniffi::export(callback_interface)]
pub trait LanListener: Send + Sync {
    /// A version counter that only grows: a screen redraws on a new value and
    /// does nothing on the same one.
    fn changed(&self, version: u64);
}

#[uniffi::export(async_runtime = "tokio")]
impl LanApi {
    /// The whole state, cloned from memory: synchronous, so the screen paints in
    /// one frame.
    pub fn snapshot(&self) -> LanSnapshotFfi {
        LanSnapshotFfi::default()
    }

    /// Registers the one listener that hears about changes.
    pub fn subscribe(&self, l: Box<dyn LanListener>) {
        let _ = l;
    }

    /// The platform's NSD resolved a node at `host`:`port`.
    pub fn found(&self, host: String, port: u16) {
        let _ = (host, port);
    }

    /// The platform lost a node it had resolved.
    pub fn lost(&self, host: String, port: u16) {
        let _ = (host, port);
    }

    /// What the app is doing, for the battery rules: browsing and announcing only
    /// run while one of the three is true.
    pub fn set_active(&self, foreground: bool, playing: bool, downloading: bool) {
        let _ = (foreground, playing, downloading);
    }

    /// The settings as they are now.
    pub fn settings(&self) -> LanSettingsFfi {
        LanSettingsFfi::default()
    }

    /// Writes them; applied at once, persisted by the core.
    pub fn set_settings(&self, s: LanSettingsFfi) {
        let _ = s;
    }

    /// Every set the connected peers expose.
    pub fn peer_sets(&self) -> Vec<PeerSetFfi> {
        Vec::new()
    }

    /// Queues the sets named `"<peer>:<kind>:<ident>"` as ordinary downloads.
    pub fn download_sets(&self, keys: Vec<String>) -> Result<(), NzError> {
        let _ = keys;
        Err(unsupported("download_sets"))
    }

    /// The account's enrolled devices (a `User` request: the screen is open).
    pub async fn devices(&self) -> Result<Vec<LanDeviceFfi>, NzError> {
        Err(unsupported("devices"))
    }

    /// Revokes one device on the server; every peer refuses it at its next sync.
    pub async fn revoke(&self, id: String) -> Result<(), NzError> {
        let _ = id;
        Err(unsupported("revoke"))
    }

    /// Renames one device, 1–64 characters.
    pub async fn rename(&self, id: String, name: String) -> Result<(), NzError> {
        let _ = (id, name);
        Err(unsupported("rename"))
    }

    /// The two directions of trust.
    pub async fn friends(&self) -> Result<LanFriendsFfi, NzError> {
        Err(unsupported("friends"))
    }

    /// Adds a trusted friend by exact account name.
    pub async fn add_friend(&self, name: String) -> Result<(), NzError> {
        let _ = name;
        Err(unsupported("add_friend"))
    }

    /// What one friend may fetch: everything, or the listed playlists.
    pub async fn set_friend_scope(
        &self,
        user_id: u32,
        all: bool,
        playlists: Vec<String>,
    ) -> Result<(), NzError> {
        let _ = (user_id, all, playlists);
        Err(unsupported("set_friend_scope"))
    }

    /// Stops sharing with one friend; their devices stop at their next roster sync.
    pub async fn remove_friend(&self, user_id: u32) -> Result<(), NzError> {
        let _ = user_id;
        Err(unsupported("remove_friend"))
    }

    /// The port Kotlin registers with `NsdManager` — the one really bound, which is
    /// the ephemeral fallback when 45910 was taken.
    pub fn advertised_port(&self) -> u16 {
        0
    }
}

/// The refusal every stub answers, naming the method so the log says which.
fn unsupported(method: &str) -> NzError {
    NzError::Unsupported {
        feature: format!("ln13:{method}"),
    }
}

/// `NzCore::lan()` lives here, not in `core.rs`: one `impl NzCore` block per file
/// is how UniFFI accepts a new method without a shared file being edited.
#[uniffi::export]
impl NzCore {
    /// The LAN node of this process.
    pub fn lan(&self) -> Arc<LanApi> {
        Arc::new(LanApi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stub object answers the empty values and refuses every write, with the
    /// sentence the UI already knows how to show. Nothing invents a peer.
    #[test]
    fn the_stub_refuses_the_writes_and_answers_nothing() {
        let api = LanApi;
        let snapshot = api.snapshot();
        assert_eq!(snapshot.state, LanStateFfi::Off);
        assert!(snapshot.peers.is_empty());
        assert!(snapshot.transfers.is_empty());
        assert!(snapshot.failures.is_empty());
        assert_eq!(snapshot.port, 0);
        assert!(!snapshot.port_fallback);
        assert!(api.peer_sets().is_empty());
        assert_eq!(api.advertised_port(), 0);

        match api.download_sets(vec!["aa:playlist:42".into()]) {
            Err(NzError::Unsupported { feature }) => assert_eq!(feature, "ln13:download_sets"),
            other => panic!("unexpected answer: {other:?}"),
        }
        // The French sentence is the existing one, so `LoadFailed` needs nothing
        // new to draw it.
        let text = crate::errors::nz_error_text(&unsupported("devices"));
        assert!(text.contains("ln13:devices"), "{text}");
    }

    /// A listener is a plain trait object: the Kotlin side registers one and the
    /// core pushes a version number, never a record (records are pulled).
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
        listener.changed(7);
        assert_eq!(seen.load(Ordering::SeqCst), 7);
    }
}
