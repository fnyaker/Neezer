//! The server's trust chain as a device reads it: the credential that names this
//! device, the revocation list that can cut it, the roster that says who shares
//! with whom, and the exporter-bound proof that binds a device key to ONE QUIC
//! connection (docs/agents/lan-nodes.md, *Trust chain*).
//!
//! **LN0 declares every type and the three verifiers; LN2 fills them.** The byte
//! layouts are the note's tables, and they are reproduced here so that the two
//! lots (the Python encoder and the Rust decoder) cannot drift:
//!
//! - credential, 154 bytes, big-endian: `NZLC` · version 1 · `kid` 8 · `subject`
//!   16 · `device_id` 12 · `device_pub` 32 · `not_before` i64 · `not_after` i64 ·
//!   `flags` 1 · `sig` 64;
//! - revocation list: `NZLR` · version 1 · `kid` 8 · `serial` u64 · `issued` i64 ·
//!   `count` u32 · `count × 32` sorted unique device keys · `sig` 64;
//! - roster: a canonical JSON body, base64, and its Ed25519 signature.
//!
//! Every signature is over a DOMAIN-SEPARATED message (`b"neezer-lan/1
//! credential\0" ‖ bytes`, `b"neezer-lan/1 revocations\0" ‖ body`,
//! `b"neezer-lan/1 roster\0" ‖ body`) and, for a device, over the TLS exporter of
//! the connection it travels on — so a recorded proof is useless on any other
//! connection (T5) and a relayed one fails (T4).
//!
//! **The rule LN2 must keep**: a caller only ever learns
//! [`CredError`]'s precise variant locally, for the log and the tests. What a
//! PEER learns is one closed connection and no bytes, whatever the cause was.

use std::collections::HashMap;

use crate::types::{DevicePub, Kid, Subject};

/// The length of an encoded credential, in bytes.
pub const CREDENTIAL_LEN: usize = 154;

/// This device's credential: who it is, until when, and what it may do.
///
/// `raw` is the exact 154 bytes that were received. Signatures are verified over
/// those bytes (never over a re-serialisation of the fields), so they are kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    /// The server LAN key that signed it.
    pub kid: Kid,
    /// The person it belongs to.
    pub subject: Subject,
    /// The server's random 12-byte row id for this device.
    pub device_id: [u8; 12],
    /// The device's Ed25519 public key — the identity every peer checks.
    pub device_pub: DevicePub,
    /// Validity start, unix seconds.
    pub not_before: i64,
    /// Validity end, unix seconds.
    pub not_after: i64,
    /// Bit 0: may serve. Bit 1: may fetch. Every other bit must be 0.
    pub flags: u8,
    raw: [u8; CREDENTIAL_LEN],
}

impl Credential {
    /// Decodes a credential without verifying it (see [`Credential::verify`]).
    ///
    /// **LN2**: exact length, magic, version, reserved flag bits — everything
    /// that can be refused before any signature work.
    pub fn decode(bytes: &[u8]) -> Result<Credential, CredError> {
        let _ = bytes;
        Err(CredError::Length)
    }

    /// Verifies a decoded credential against the known server keys and the
    /// revocation list, at `now` (unix seconds, ±10 min of skew allowed).
    pub fn verify(
        &self,
        keys: &TrustKeys,
        revoked: &Revocations,
        now: i64,
    ) -> Result<(), CredError> {
        let _ = (keys, revoked, now);
        Err(CredError::Signature)
    }

    /// The exact bytes received, which is what any signature over a credential
    /// travels with.
    pub fn raw(&self) -> &[u8; CREDENTIAL_LEN] {
        &self.raw
    }
}

/// The server LAN keys this device knows: `kid` → Ed25519 public key.
///
/// Filled from `GET /api/lan/status` (the current key, and the previous one while
/// credentials it signed are still valid). A key that is not here is
/// [`CredError::UnknownKid`], and a credential signed by one is refused.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustKeys {
    keys: HashMap<Kid, [u8; 32]>,
}

impl TrustKeys {
    /// The known keys, in the order the server listed them.
    pub fn from_status(keys: &[(Kid, [u8; 32])]) -> Self {
        TrustKeys {
            keys: keys.iter().copied().collect(),
        }
    }

    /// The public key of one `kid`, when this device knows it.
    pub fn get(&self, kid: &Kid) -> Option<&[u8; 32]> {
        self.keys.get(kid)
    }

    /// How many keys are known (a test and a log line read this; nothing else).
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Whether no key is known at all — a device that never enrolled.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// One revocation list: a monotonic serial and the device keys it cuts.
///
/// Only keys whose credential has not expired yet are listed, so the list stays
/// bounded by the devices revoked in the last credential lifetime (max 4 096
/// entries). A device keeps the highest serial it has seen per `kid`, from the
/// server or from any authenticated peer (the gossip).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revocations {
    pub kid: Kid,
    pub serial: u64,
    pub issued: i64,
    /// Sorted ascending and unique, as the wire requires.
    keys: Vec<DevicePub>,
    /// The exact signed bytes, kept so the blob can be gossiped unchanged.
    raw: Vec<u8>,
}

impl Revocations {
    /// Decodes and verifies a signed revocation list. A list whose entries are
    /// unsorted or duplicated is refused: a verifier that sorted them itself
    /// would accept two different encodings of one list.
    pub fn decode_verify(bytes: &[u8], keys: &TrustKeys) -> Result<Self, CredError> {
        let _ = (bytes, keys);
        Err(CredError::Length)
    }

    /// Whether this device key is revoked.
    pub fn contains(&self, k: &DevicePub) -> bool {
        self.keys.contains(k)
    }

    /// An empty list for one `kid`: what a device holds before it ever saw the
    /// server.
    pub fn empty(kid: Kid) -> Self {
        Revocations {
            kid,
            serial: 0,
            issued: 0,
            keys: Vec::new(),
            raw: Vec::new(),
        }
    }

    /// The exact signed bytes, ready to gossip as [`crate::proto::Control::Revocations`].
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// How many keys are revoked (bounded by the list's cap).
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Whether nothing is revoked at all — the state before the first server sync.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// The per-person view the server signs: to whom I share what, and who shares
/// with me.
///
/// Fetched by each of the person's devices, verified over the exact bytes
/// received, then parsed. `serial` only grows; a roster is gossiped ONLY between
/// devices of the same subject, because it names other people.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roster {
    pub kid: Kid,
    pub subject: Subject,
    pub serial: u64,
    pub issued: i64,
    pub expires: i64,
    /// The people I share with, and exactly what.
    pub shares_to: Vec<Share>,
    /// The people who share with me, and the names they are shown under.
    pub trusted_by: Vec<Truster>,
    /// The exact signed JSON body, kept so the blob can be gossiped unchanged.
    raw: Vec<u8>,
}

impl Roster {
    /// Decodes and verifies a signed roster (`{"body","sig"}`, base64, as
    /// `GET /api/lan/roster` and an enrolment answer carry it).
    ///
    /// Caps are part of verification: 64 friends, 20 000 track ids in total,
    /// beyond which [`CredError::Caps`] is the answer — a roster is signed by the
    /// server, but a device that trusted it blindly would still allocate what a
    /// compromised one asked for.
    pub fn decode_verify(json: &[u8], keys: &TrustKeys) -> Result<Self, CredError> {
        let _ = (json, keys);
        Err(CredError::Length)
    }

    /// The exact signed body, ready to gossip as [`crate::proto::Control::Roster`].
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// Where this subject sits in my roster, if anywhere.
    pub fn share_to(&self, subject: &Subject) -> Option<&Share> {
        self.shares_to
            .iter()
            .find(|share| &share.subject == subject)
    }

    /// Who named me as a friend, if anyone: the people whose devices I may ask.
    pub fn truster(&self, subject: &Subject) -> Option<&Truster> {
        self.trusted_by
            .iter()
            .find(|truster| &truster.subject == subject)
    }
}

/// What one friend may fetch from me.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Share {
    pub subject: Subject,
    /// Everything I have locally.
    pub all: bool,
    /// Otherwise: the union of these playlists' tracks, as the server saw them
    /// when it issued the roster.
    pub playlists: Vec<SharedPlaylist>,
}

/// One playlist of a share, with its tracks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedPlaylist {
    pub id: String,
    pub title: String,
    pub tracks: Vec<String>,
}

/// One person who shares with me.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Truster {
    pub subject: Subject,
    /// The display name the server put in MY roster: names never travel from a
    /// peer, they come from the server.
    pub name: String,
}

/// Which side of a connection a proof is for.
///
/// The role is part of the signed message, so a responder's proof cannot be
/// replayed as an initiator's on another connection (reflection, T5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Initiator,
    Responder,
}

impl Role {
    /// The bytes this role signs and verifies.
    pub fn as_bytes(self) -> &'static [u8] {
        match self {
            Role::Initiator => b"initiator",
            Role::Responder => b"responder",
        }
    }
}

/// `b"neezer-lan/1 auth\0" ‖ role ‖ exporter` — what a device signs with its key.
///
/// The exporter is the TLS exporter of THIS connection (RFC 9266 channel
/// binding): a proof is only valid on the connection whose exporter it names, so
/// a recorded proof is useless anywhere else and a MITM that terminates two
/// sessions cannot relay one.
///
/// **LN2 implements this** (and its test vector); LN6 calls it on both sides.
pub fn auth_message(role: Role, exporter: &[u8; 32]) -> Vec<u8> {
    let _ = (role, exporter);
    Vec::new()
}

/// Why a credential, a revocation list or a roster was refused.
///
/// Every variant is for THIS device's log and its tests only: the peer is closed
/// with one code and no reason, whatever happened (`LanError::Refused`).
#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum CredError {
    /// Not the fixed length the layout requires.
    #[error("longueur inattendue")]
    Length,
    /// Not `NZLC`/`NZLR`.
    #[error("magie inconnue")]
    Magic,
    /// A version this build does not speak.
    #[error("version inconnue")]
    Version,
    /// A reserved flag bit is set.
    #[error("drapeaux réservés")]
    Flags,
    /// Signed by a server key this device does not know.
    #[error("clé du serveur inconnue")]
    UnknownKid,
    /// The signature does not verify over the exact bytes.
    #[error("signature invalide")]
    Signature,
    /// `not_before` is further in the future than the allowed skew.
    #[error("attestation pas encore valide")]
    NotYetValid,
    /// `not_after` is past.
    #[error("attestation expirée")]
    Expired,
    /// The device key is in the revocation list.
    #[error("appareil révoqué")]
    Revoked,
    /// A body that is not the JSON the layout describes.
    #[error("JSON invalide : {0}")]
    Json(String),
    /// A count over its cap (4 096 revoked keys, 64 friends, 20 000 track ids).
    #[error("au-delà du plafond")]
    Caps,
}

impl CredError {
    /// The cause in the words this device's log and its tests use.
    pub fn cause_fr(&self) -> String {
        self.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The layout lengths are the note's table, and the domain strings are
    /// `b"neezer-lan/1 …\0"`: LN2 asserts the three test vectors against them, so
    /// a typo here is caught by a different lot's tests than the one that wrote it.
    #[test]
    fn the_credential_is_154_bytes_and_its_parts_add_up() {
        assert_eq!(CREDENTIAL_LEN, 154);
        assert_eq!(4 + 1 + 8 + 16 + 12 + 32 + 8 + 8 + 1 + 64, CREDENTIAL_LEN);
    }

    /// An empty trust state is what a device that never enrolled holds: no key,
    /// no revocation, so everything is refused — and `get` says so instead of
    /// panicking.
    #[test]
    fn an_empty_trust_state_knows_no_key() {
        let keys = TrustKeys::from_status(&[]);
        assert!(keys.is_empty());
        assert_eq!(keys.get(&Kid([0; 8])), None);

        let revoked = Revocations::empty(Kid([1; 8]));
        assert_eq!(revoked.serial, 0);
        assert!(revoked.raw().is_empty());
        assert_eq!(revoked.len(), 0);
        assert!(!revoked.contains(&[2; 32]));
    }

    /// The role is part of the signed message, and the two roles are different
    /// bytes: reflection cannot work by construction.
    #[test]
    fn the_two_roles_sign_different_bytes() {
        assert_eq!(Role::Initiator.as_bytes(), b"initiator");
        assert_eq!(Role::Responder.as_bytes(), b"responder");
        assert_ne!(Role::Initiator.as_bytes(), Role::Responder.as_bytes());
    }

    /// A stub answers "refused", never "accepted": the shape of the type is
    /// final, the behaviour is LN2's.
    #[test]
    fn the_unimplemented_verifiers_refuse() {
        assert_eq!(
            Credential::decode(&[0; CREDENTIAL_LEN]).err(),
            Some(CredError::Length)
        );
        let keys = TrustKeys::default();
        assert_eq!(
            Revocations::decode_verify(&[], &keys).err(),
            Some(CredError::Length)
        );
        assert_eq!(
            Roster::decode_verify(b"{}", &keys).err(),
            Some(CredError::Length)
        );
        assert!(auth_message(Role::Initiator, &[0; 32]).is_empty());
    }
}
