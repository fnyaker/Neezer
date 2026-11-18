//! The LAN side of the server's JSON: the feature probe, the enrolment, the
//! roster, my enrolled devices, my trusted friends and their scope, and the two
//! admin settings (docs/agents/lan-nodes.md, LN1's routes).
//!
//! Tolerant like every other file of this crate (rule 7): an unknown field is
//! ignored and a missing one falls back to its default, so an older server never
//! fails a screen and a newer one never fails a parse.
//!
//! The crypto blobs (`credential`, `roster`, `revocations`) travel as the
//! base64/hex strings the server signs. This crate does NOT decode them:
//! verifying a credential, a roster or a revocation list is `nz-lan`'s job
//! (`cred.rs`), against the exact bytes received, and a second decoder here
//! would be a second thing to keep right.

use serde::{Deserialize, Serialize};

/// `GET /api/lan/status` — what the feature probe reads, and the first request
/// of an enrolment (it carries the caller's subject and the server's keys).
///
/// `enabled = false` is the server's own switch (rule 6): every client hides the
/// feature then, exactly as it does for a server too old to have the route.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LanStatus {
    #[serde(default)]
    pub enabled: bool,
    /// The caller's opaque 16-byte subject, hex — never a name.
    #[serde(default)]
    pub subject: String,
    /// The LAN key that signs credentials now, then the previous one while
    /// credentials it signed are still valid.
    #[serde(default)]
    pub keys: Vec<LanKey>,
    /// The lifetime of a NEW credential, in days (7–90, default 30).
    #[serde(default)]
    pub credential_days: u32,
    /// The highest revocation serial the server has issued.
    #[serde(default)]
    pub revocation_serial: u64,
}

/// One Ed25519 LAN key of the server.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LanKey {
    /// `SHA-256(public key)[0..8]`, hex.
    #[serde(default)]
    pub kid: String,
    /// The public key, hex (the server's JSON calls it `pub`).
    #[serde(default, rename = "pub")]
    pub pub_key: String,
    /// Until when this key still validates a credential. `None` on the current
    /// key: it has no end date until it is rotated.
    #[serde(default)]
    pub until: Option<i64>,
}

/// `POST /api/lan/enroll` — the possession proof: the device signs its own
/// public key, its subject and the instant it asks at.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LanEnrollRequest {
    /// The device's Ed25519 public key, hex (64 characters).
    #[serde(default)]
    pub pubkey: String,
    /// The device's display name, 1–64 characters.
    #[serde(default)]
    pub name: String,
    /// `"desktop"`, `"android"` or `"ios"`.
    #[serde(default)]
    pub platform: String,
    /// The device's own clock, unix seconds; the server allows ±300 s.
    #[serde(default)]
    pub at: i64,
    /// `Ed25519(device key, "neezer-lan/1 enroll\0" ‖ subject_hex ‖ pubkey_hex ‖ at)`,
    /// base64.
    #[serde(default)]
    pub proof: String,
    /// The device this key REPLACES, when the app already had one (a renewal
    /// rotates the key): the server then keeps the old one only until it
    /// expires.
    #[serde(default)]
    pub device_id: Option<String>,
}

/// What an enrolment answers: everything the device needs to be a node, so a
/// login costs no second round trip.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LanEnrollAnswer {
    #[serde(default)]
    pub device_id: String,
    /// The 154-byte credential, base64 (`nz-lan`'s `cred::Credential`).
    #[serde(default)]
    pub credential: String,
    #[serde(default)]
    pub roster: LanSigned,
    /// The signed revocation list, base64.
    #[serde(default)]
    pub revocations: String,
}

/// `GET /api/lan/roster` — the roster and the revocation list, or `None` when
/// the caller's `ETag` still matches (`304`, no body).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LanRosterAnswer {
    #[serde(default)]
    pub roster: LanSigned,
    #[serde(default)]
    pub revocations: String,
    /// The `ETag` to send back as `If-None-Match`.
    #[serde(default)]
    pub etag: String,
}

/// A blob and the signature over it: the roster's `{"body","sig"}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LanSigned {
    /// The canonical JSON body, base64.
    #[serde(default)]
    pub body: String,
    /// `Ed25519(LAN key, "neezer-lan/1 roster\0" ‖ body)`, base64.
    #[serde(default)]
    pub sig: String,
}

/// One device of the person's, as `GET /api/lan/devices` lists it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LanDevice {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub last_seen: Option<String>,
    /// Set once the device is revoked; its credentials then stop being renewed.
    #[serde(default)]
    pub revoked: Option<String>,
    /// `true` for the device that made the request.
    #[serde(default)]
    pub current: bool,
}

/// `GET /api/lan/friends` — the two directions of one person's trust.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LanFriends {
    /// The people I share with, and exactly what they may fetch.
    #[serde(default)]
    pub shares_to: Vec<LanFriend>,
    /// The people who share with me: whom my devices may ask, and the name they
    /// are shown under.
    #[serde(default)]
    pub trusted_by: Vec<LanFriend>,
}

/// One trusted friend and the scope of the share.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LanFriend {
    #[serde(default)]
    pub user_id: u32,
    #[serde(default)]
    pub name: String,
    /// Everything I have locally, rather than the playlists below.
    #[serde(default)]
    pub all: bool,
    /// The playlists the friend may take, when `all` is `false`.
    #[serde(default)]
    pub playlists: Vec<LanPlaylistRef>,
}

/// One playlist of a share, as the friends screen names it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LanPlaylistRef {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
}

/// The scope a `PATCH /api/lan/friends/<id>` sets.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LanScope {
    #[serde(default)]
    pub all: bool,
    /// Playlist ids, owned by the caller.
    #[serde(default)]
    pub playlists: Vec<String>,
}

/// `GET`/`PUT /api/lan/settings` — the server's two LAN settings (rule 6).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LanServerSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub credential_days: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rule 7 in miniature: an older server omits what it does not know, and a
    /// newer one adds fields this client never heard of. Neither fails a screen.
    #[test]
    fn an_older_and_a_newer_server_both_parse() {
        let status: LanStatus = serde_json::from_str("{}").expect("an empty status");
        assert!(!status.enabled);
        assert!(status.keys.is_empty());

        let status: LanStatus = serde_json::from_str(
            r#"{"enabled":true,"subject":"00112233445566778899aabbccddeeff",
                "keys":[{"kid":"56475aa75463474c",
                         "pub":"03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8"}],
                "credential_days":30,"revocation_serial":7,"future":{"nested":[1,2]}}"#,
        )
        .expect("the server's own answer");
        assert!(status.enabled);
        assert_eq!(status.keys.len(), 1);
        assert_eq!(status.keys[0].kid, "56475aa75463474c");
        // The server's field is `pub`; the Rust name must not be.
        assert!(status.keys[0].pub_key.starts_with("03a107"));
        assert_eq!(status.keys[0].until, None);
        assert_eq!(status.revocation_serial, 7);
    }

    /// The friends payload carries both directions, and a `trusted_by` entry
    /// has no scope at all — that is what the defaults are for.
    #[test]
    fn the_two_directions_of_trust_parse() {
        let friends: LanFriends = serde_json::from_str(
            r#"{"shares_to":[{"user_id":2,"name":"Alice","all":false,
                              "playlists":[{"id":"p1","title":"Route"}]}],
                "trusted_by":[{"user_id":3,"name":"Bob"}]}"#,
        )
        .expect("the friends answer");
        assert_eq!(friends.shares_to.len(), 1);
        assert_eq!(friends.shares_to[0].playlists[0].title, "Route");
        assert!(!friends.shares_to[0].all);
        assert_eq!(friends.trusted_by[0].user_id, 3);
        assert!(friends.trusted_by[0].playlists.is_empty());
        assert!(!friends.trusted_by[0].all);
    }
}
