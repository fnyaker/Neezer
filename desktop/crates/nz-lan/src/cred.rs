//! The server's trust chain as a device reads it: the credential that names this
//! device, the revocation list that can cut it, the roster that says who shares
//! with whom, and the exporter-bound proof that binds a device key to ONE QUIC
//! connection (docs/agents/lan-nodes.md, *Trust chain*).
//!
//! **LN2 fills the three verifiers.** The byte layouts are the note's tables, and
//! they are reproduced here so that the two implementations (the Python encoder
//! of `neezer/lan.py` and this decoder) cannot drift:
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
//! # What LN2 decided, and why
//!
//! - **The signature is always checked over the bytes that were received.** The
//!   roster is JSON, so a decoder that re-serialised its fields would refuse a
//!   valid roster whose writer ordered its keys differently (or added a space);
//!   `the_exact_received_bytes_are_signed_not_a_re_serialisation` holds that.
//! - **The length, the magic, the version and the reserved flag bits are checked
//!   before any work**, and a blow-up on length is the answer to a 1 MiB blob:
//!   `decode` allocates nothing at all for an input that is not 154 bytes (counted
//!   with a counting allocator in `tests/cred.rs`).
//! - **A roster's `kid` is read with a minimal parse first**, because the key that
//!   verifies the signature is named INSIDE the body that is being verified: the
//!   order is therefore parse-`kid` → verify the exact bytes → parse the rest →
//!   check the caps, never "parse everything, then verify".
//! - **A revocation list must be canonical** (strictly ascending keys). The server
//!   sorts and dedups; a verifier that sorted them itself would accept two
//!   encodings of one list, and the second one would be an attacker's. Such a list
//!   is refused as [`CredError::Signature`], whose doc says it also covers a body
//!   that cannot be the server's canonical one.
//!
//! **The rule LN2 must keep**: a caller only ever learns [`CredError`]'s precise
//! variant locally, for the log and the tests. What a PEER learns is one closed
//! connection and no bytes, whatever the cause was.

use std::collections::HashMap;

use aws_lc_rs::signature::{ED25519, UnparsedPublicKey};
use serde::Deserialize;

use crate::types::{DevicePub, Kid, Subject};

/// The length of an encoded credential, in bytes.
pub const CREDENTIAL_LEN: usize = 154;

/// The most device keys one revocation list may name (the note's cap).
pub const REVOCATIONS_MAX: usize = 4096;

/// The most friends one roster may name, on either side (`shares_to` as much as
/// `trusted_by`).
pub const ROSTER_MAX_FRIENDS: usize = 64;

/// The most track ids one roster may carry, all shared playlists together.
pub const ROSTER_MAX_TRACKS: usize = 20_000;

/// The most bytes of roster envelope this decoder will look at.
///
/// The note's cap is 20 000 track ids (a 4 000-track playlist is ≤ 200 KB of
/// roster); 4 MiB leaves room for a maximal roster with its titles and still caps
/// what a compromised server can make a device parse. Above it: [`CredError::Caps`],
/// before any allocation proportional to the input.
pub const ROSTER_MAX_BYTES: usize = 4 << 20;

/// The domain the credential's signature is computed over.
const CREDENTIAL_DOMAIN: &[u8] = b"neezer-lan/1 credential\x00";
/// The domain the revocation list's signature is computed over.
const REVOCATIONS_DOMAIN: &[u8] = b"neezer-lan/1 revocations\x00";
/// The domain the roster's signature is computed over.
const ROSTER_DOMAIN: &[u8] = b"neezer-lan/1 roster\x00";
/// The domain a device key signs an exporter with (see [`auth_message`]).
const AUTH_DOMAIN: &[u8] = b"neezer-lan/1 auth\x00";

/// The credential's magic.
const CREDENTIAL_MAGIC: &[u8; 4] = b"NZLC";
/// The revocation list's magic.
const REVOCATIONS_MAGIC: &[u8; 4] = b"NZLR";

/// The version both binary layouts speak.
const VERSION: u8 = 1;
/// The flag bits this version defines: bit 0 may serve, bit 1 may fetch.
const FLAGS_KNOWN: u8 = 0b11;

/// The accepted clock skew, on both ends of the validity window.
const SKEW_SECONDS: i64 = 600;

/// An Ed25519 signature, in bytes.
const SIGNATURE_LEN: usize = 64;

/// The credential's 11 field offsets, in the note's table order.
const OFF_MAGIC: usize = 0;
const OFF_VERSION: usize = 4;
const OFF_KID: usize = 5;
const OFF_SUBJECT: usize = 13;
const OFF_DEVICE_ID: usize = 29;
const OFF_DEVICE_PUB: usize = 41;
const OFF_NOT_BEFORE: usize = 73;
const OFF_NOT_AFTER: usize = 81;
const OFF_FLAGS: usize = 89;
const OFF_SIGNATURE: usize = 90;

/// The revocation list's header: magic, version, kid, serial, issued, count.
const REVOCATIONS_HEADER_LEN: usize = 4 + 1 + 8 + 8 + 8 + 4;

/// A device key, in bytes.
const DEVICE_PUB_LEN: usize = 32;

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
    /// Everything that can be refused before any signature work is: the exact
    /// length, the magic, the version, and the reserved flag bits. Nothing here
    /// depends on a count from the wire, so a 1 MiB blob costs one comparison and
    /// no allocation (measured: `decode_allocates_nothing_for_a_bad_length`).
    pub fn decode(bytes: &[u8]) -> Result<Credential, CredError> {
        if bytes.len() != CREDENTIAL_LEN {
            return Err(CredError::Length);
        }
        if bytes.get(OFF_MAGIC..OFF_VERSION) != Some(CREDENTIAL_MAGIC.as_slice()) {
            return Err(CredError::Magic);
        }
        if bytes.get(OFF_VERSION).copied() != Some(VERSION) {
            return Err(CredError::Version);
        }
        let flags = bytes.get(OFF_FLAGS).copied().ok_or(CredError::Length)?;
        if flags & !FLAGS_KNOWN != 0 {
            return Err(CredError::Flags);
        }

        let mut raw = [0u8; CREDENTIAL_LEN];
        raw.copy_from_slice(bytes);
        Ok(Credential {
            kid: Kid(array(&raw, OFF_KID).ok_or(CredError::Length)?),
            subject: Subject(array(&raw, OFF_SUBJECT).ok_or(CredError::Length)?),
            device_id: array(&raw, OFF_DEVICE_ID).ok_or(CredError::Length)?,
            device_pub: array(&raw, OFF_DEVICE_PUB).ok_or(CredError::Length)?,
            not_before: i64::from_be_bytes(array(&raw, OFF_NOT_BEFORE).ok_or(CredError::Length)?),
            not_after: i64::from_be_bytes(array(&raw, OFF_NOT_AFTER).ok_or(CredError::Length)?),
            flags,
            raw,
        })
    }

    /// Verifies a decoded credential against the known server keys and the
    /// revocation list, at `now` (unix seconds, ±10 min of skew allowed).
    ///
    /// The order is the cheap-to-expensive one: the key must be known, the
    /// signature must verify over the exact bytes, the window must cover `now`,
    /// and the device key must not be revoked. The dates are compared with
    /// saturating arithmetic, so a credential whose `not_before` is `i64::MIN`
    /// refuses instead of overflowing.
    ///
    /// A device key named by the list is refused whatever `kid` the list was
    /// signed under: a key is revoked once and for all, and a caller that passes
    /// the list of another server key must not thereby accept it.
    pub fn verify(
        &self,
        keys: &TrustKeys,
        revoked: &Revocations,
        now: i64,
    ) -> Result<(), CredError> {
        let key = keys.get(&self.kid).ok_or(CredError::UnknownKid)?;
        verify_signature(
            key,
            CREDENTIAL_DOMAIN,
            &self.raw[..OFF_SIGNATURE],
            &self.raw[OFF_SIGNATURE..],
        )?;
        if now < self.not_before.saturating_sub(SKEW_SECONDS) {
            return Err(CredError::NotYetValid);
        }
        if now > self.not_after.saturating_add(SKEW_SECONDS) {
            return Err(CredError::Expired);
        }
        if revoked.contains(&self.device_pub) {
            return Err(CredError::Revoked);
        }
        Ok(())
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
    ///
    /// Every count is checked against [`REVOCATIONS_MAX`] **before** the length
    /// that would follow from it is even computed, and the exact length is checked
    /// before the bytes are copied: a 1 MiB blob that is not a list of this shape
    /// allocates nothing.
    pub fn decode_verify(bytes: &[u8], keys: &TrustKeys) -> Result<Self, CredError> {
        let header = bytes
            .get(..REVOCATIONS_HEADER_LEN)
            .ok_or(CredError::Length)?;
        if header.get(..4) != Some(REVOCATIONS_MAGIC.as_slice()) {
            return Err(CredError::Magic);
        }
        if header.get(4).copied() != Some(VERSION) {
            return Err(CredError::Version);
        }
        let kid = Kid(array::<8>(header, 5).ok_or(CredError::Length)?);
        let serial = u64::from_be_bytes(array::<8>(header, 13).ok_or(CredError::Length)?);
        let issued = i64::from_be_bytes(array::<8>(header, 21).ok_or(CredError::Length)?);
        let count = u32::from_be_bytes(array::<4>(header, 29).ok_or(CredError::Length)?) as usize;
        if count > REVOCATIONS_MAX {
            return Err(CredError::Caps);
        }
        let body_len = REVOCATIONS_HEADER_LEN + count * DEVICE_PUB_LEN;
        if bytes.len() != body_len + SIGNATURE_LEN {
            return Err(CredError::Length);
        }

        let key = keys.get(&kid).ok_or(CredError::UnknownKid)?;
        verify_signature(
            key,
            REVOCATIONS_DOMAIN,
            &bytes[..body_len],
            &bytes[body_len..],
        )?;

        // Canonical, or not the server's: strictly ascending, so neither a
        // duplicate nor a shuffle can name the same set of keys twice.
        let mut revoked: Vec<DevicePub> = Vec::with_capacity(count);
        let (entries, rest) = bytes[REVOCATIONS_HEADER_LEN..body_len].as_chunks::<DEVICE_PUB_LEN>();
        debug_assert!(rest.is_empty(), "le corps est un multiple de 32 octets");
        for entry in entries {
            let pubkey: DevicePub = *entry;
            if revoked.last().is_some_and(|last| pubkey <= *last) {
                return Err(CredError::Signature);
            }
            revoked.push(pubkey);
        }

        Ok(Revocations {
            kid,
            serial,
            issued,
            keys: revoked,
            raw: bytes.to_vec(),
        })
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
    /// The server's signature over `raw`, kept so the envelope can be rebuilt.
    signature: [u8; SIGNATURE_LEN],
}

impl Roster {
    /// Decodes and verifies a signed roster (`{"body","sig"}`, base64, as
    /// `GET /api/lan/roster` and an enrolment answer carry it).
    ///
    /// Caps are part of verification: 64 friends, 20 000 track ids in total,
    /// beyond which [`CredError::Caps`] is the answer — a roster is signed by the
    /// server, but a device that trusted it blindly would still allocate what a
    /// compromised one asked for. The envelope itself is capped at
    /// [`ROSTER_MAX_BYTES`] before it is parsed at all.
    ///
    /// The `kid` names the key that verifies the signature and lives INSIDE the
    /// body, so it is read with a minimal parse first; the signature is then
    /// checked over the exact received bytes, and only then is the whole body
    /// parsed. A roster written by a different JSON writer (other key order, other
    /// spacing) still verifies, because its bytes are what is signed.
    pub fn decode_verify(json: &[u8], keys: &TrustKeys) -> Result<Self, CredError> {
        if json.len() > ROSTER_MAX_BYTES {
            return Err(CredError::Caps);
        }
        let envelope: Envelope =
            serde_json::from_slice(json).map_err(|err| CredError::Json(err.to_string()))?;
        let body = base64_decode(&envelope.body)
            .ok_or_else(|| CredError::Json("corps signé : base64 invalide".to_owned()))?;
        let mut signature = [0u8; SIGNATURE_LEN];
        let decoded = base64_decode(&envelope.sig)
            .ok_or_else(|| CredError::Json("signature : base64 invalide".to_owned()))?;
        if decoded.len() != SIGNATURE_LEN {
            return Err(CredError::Length);
        }
        signature.copy_from_slice(&decoded);

        // 1. Which key signed it: the only thing read before verifying.
        let named: NamedKid =
            serde_json::from_slice(&body).map_err(|err| CredError::Json(err.to_string()))?;
        let kid = Kid(hex_decode::<8>(&named.kid, "kid")?);
        let key = keys.get(&kid).ok_or(CredError::UnknownKid)?;

        // 2. The signature, over exactly what was received.
        verify_signature(key, ROSTER_DOMAIN, &body, &signature)?;

        // 3. The body, now that it is the server's.
        let parsed: Body =
            serde_json::from_slice(&body).map_err(|err| CredError::Json(err.to_string()))?;
        if parsed.v != 1 {
            return Err(CredError::Version);
        }
        if parsed.shares_to.len() > ROSTER_MAX_FRIENDS
            || parsed.trusted_by.len() > ROSTER_MAX_FRIENDS
        {
            return Err(CredError::Caps);
        }
        let mut tracks = 0usize;
        for share in &parsed.shares_to {
            for playlist in &share.playlists {
                tracks = tracks.saturating_add(playlist.tracks.len());
                if tracks > ROSTER_MAX_TRACKS {
                    return Err(CredError::Caps);
                }
            }
        }

        let mut shares_to = Vec::with_capacity(parsed.shares_to.len());
        for share in parsed.shares_to {
            let subject = hex_decode::<16>(&share.subject, "sujet partagé")?;
            let mut playlists = Vec::with_capacity(share.playlists.len());
            for playlist in share.playlists {
                playlists.push(SharedPlaylist {
                    id: playlist.id,
                    title: playlist.title,
                    tracks: playlist.tracks,
                });
            }
            shares_to.push(Share {
                subject: Subject(subject),
                all: share.all,
                playlists,
            });
        }
        let mut trusted_by = Vec::with_capacity(parsed.trusted_by.len());
        for truster in parsed.trusted_by {
            let subject = hex_decode::<16>(&truster.subject, "sujet de confiance")?;
            trusted_by.push(Truster {
                subject: Subject(subject),
                name: truster.name,
            });
        }

        Ok(Roster {
            kid,
            subject: Subject(hex_decode::<16>(&parsed.subject, "sujet")?),
            serial: parsed.serial,
            issued: parsed.issued,
            expires: parsed.expires,
            shares_to,
            trusted_by,
            raw: body,
            signature,
        })
    }

    /// The exact signed body, ready to gossip as [`crate::proto::Control::Roster`].
    ///
    /// **This is the body alone**, which is the unit the signature covers. What
    /// travels on the gossip is the `{"body","sig"}` envelope: [`Roster::raw_signed`].
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// The `{"body","sig"}` envelope this roster arrived in, rebuilt from the exact
    /// body bytes and the exact signature.
    ///
    /// The signed unit is the body, so rebuilding the envelope is safe (it is not
    /// signed) and a peer verifies the same bytes this device did. It is what
    /// `proto::Control::Roster` carries, base64-encoded.
    pub fn raw_signed(&self) -> Vec<u8> {
        let envelope = EnvelopeOut {
            body: base64_encode(&self.raw),
            sig: base64_encode(&self.signature),
        };
        serde_json::to_vec(&envelope).unwrap_or_default()
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
/// 58 bytes: 17 of domain, 9 of role, 32 of exporter. The note's third test vector
/// is this message with the `initiator` role and an all-`0xAA` exporter.
pub fn auth_message(role: Role, exporter: &[u8; 32]) -> Vec<u8> {
    let mut message =
        Vec::with_capacity(AUTH_DOMAIN.len() + role.as_bytes().len() + exporter.len());
    message.extend_from_slice(AUTH_DOMAIN);
    message.extend_from_slice(role.as_bytes());
    message.extend_from_slice(exporter);
    message
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
    /// The signature does not verify over the exact bytes — or the bytes are not
    /// the canonical encoding the signer emits (a revocation list whose keys are
    /// not strictly ascending), which no verifier may normalise away.
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

/// Verifies `signature` over `domain ‖ message` with an Ed25519 public key.
///
/// `aws-lc-rs`'s own verification — no hand-rolled comparison anywhere, and the
/// bytes are the caller's exact ones.
fn verify_signature(
    public: &[u8; 32],
    domain: &[u8],
    message: &[u8],
    signature: &[u8],
) -> Result<(), CredError> {
    let mut signed = Vec::with_capacity(domain.len() + message.len());
    signed.extend_from_slice(domain);
    signed.extend_from_slice(message);
    UnparsedPublicKey::new(&ED25519, public)
        .verify(&signed, signature)
        .map_err(|_| CredError::Signature)
}

/// Copies `N` bytes at `at`, or `None` when the slice is too short.
///
/// The layouts are length-checked before this is called; answering `None` instead
/// of panicking keeps a hostile input from crashing a node even if a later edit
/// gets that order wrong.
fn array<const N: usize>(bytes: &[u8], at: usize) -> Option<[u8; N]> {
    let mut out = [0u8; N];
    out.copy_from_slice(bytes.get(at..at + N)?);
    Some(out)
}

/// Decodes exactly `N` bytes of lowercase-or-uppercase hex, or a precise cause.
fn hex_decode<const N: usize>(hex: &str, what: &str) -> Result<[u8; N], CredError> {
    let bytes = hex.as_bytes();
    if bytes.len() != N * 2 {
        return Err(CredError::Json(format!(
            "{what} : {} caractères hexadécimaux attendus",
            N * 2
        )));
    }
    let mut out = [0u8; N];
    for (index, pair) in bytes.as_chunks::<2>().0.iter().enumerate() {
        let high = hex_value(pair[0])
            .ok_or_else(|| CredError::Json(format!("{what} : chiffre hexadécimal invalide")))?;
        let low = hex_value(pair[1])
            .ok_or_else(|| CredError::Json(format!("{what} : chiffre hexadécimal invalide")))?;
        out[index] = (high << 4) | low;
    }
    Ok(out)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Lowercase hex, the spelling the server's JSON uses for every id.
pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Standard base64 with padding, decoded strictly.
///
/// `None` for any character outside the alphabet, for padding in the middle, for
/// a length that cannot be a base64 body, and for non-zero trailing bits: the
/// server's encoder emits one spelling of one body, and a decoder that accepted
/// several would give an attacker a second envelope for the same signature.
pub(crate) fn base64_decode(text: &str) -> Option<Vec<u8>> {
    const INVALID: u8 = 0xff;
    fn value(byte: u8) -> u8 {
        match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => INVALID,
        }
    }

    let raw = text.as_bytes();
    let padding = raw.iter().rev().take_while(|byte| **byte == b'=').count();
    if padding > 2 {
        return None;
    }
    let body = raw.get(..raw.len() - padding)?;
    if padding > 0 && !raw.len().is_multiple_of(4) {
        return None;
    }
    if body.len() % 4 == 1 {
        return None;
    }

    let mut out = Vec::with_capacity(body.len() / 4 * 3 + 3);
    let mut accumulator = 0u32;
    let mut bits = 0u32;
    for byte in body {
        let value = value(*byte);
        if value == INVALID {
            return None;
        }
        accumulator = (accumulator << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((accumulator >> bits) as u8);
            accumulator &= (1 << bits) - 1;
        }
    }
    if accumulator != 0 {
        return None;
    }
    Some(out)
}

/// Standard base64 with padding, the spelling Python's `b64encode` emits.
pub(crate) fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = u32::from(chunk[0]);
        let second = chunk.get(1).map_or(0, |byte| u32::from(*byte));
        let third = chunk.get(2).map_or(0, |byte| u32::from(*byte));
        let triple = (first << 16) | (second << 8) | third;
        out.push(ALPHABET[(triple >> 18) as usize & 0x3f] as char);
        out.push(ALPHABET[(triple >> 12) as usize & 0x3f] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(triple >> 6) as usize & 0x3f] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[triple as usize & 0x3f] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// `{"body": "<base64>", "sig": "<base64>"}`, as the server and a peer send it.
#[derive(Deserialize)]
struct Envelope {
    #[serde(default)]
    body: String,
    #[serde(default)]
    sig: String,
}

#[derive(serde::Serialize)]
struct EnvelopeOut {
    body: String,
    sig: String,
}

/// The one field read before the signature is verified.
#[derive(Deserialize)]
struct NamedKid {
    #[serde(default)]
    kid: String,
}

#[derive(Deserialize)]
struct Body {
    #[serde(default)]
    v: u32,
    // `kid` is read by `NamedKid` before the signature; the full parse ignores it.
    #[serde(default)]
    subject: String,
    #[serde(default)]
    serial: u64,
    #[serde(default)]
    issued: i64,
    #[serde(default)]
    expires: i64,
    #[serde(default)]
    shares_to: Vec<ShareJson>,
    #[serde(default)]
    trusted_by: Vec<TrusterJson>,
}

#[derive(Deserialize)]
struct ShareJson {
    #[serde(default)]
    subject: String,
    #[serde(default)]
    all: bool,
    #[serde(default)]
    playlists: Vec<PlaylistJson>,
}

#[derive(Deserialize)]
struct PlaylistJson {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    tracks: Vec<String>,
}

#[derive(Deserialize)]
struct TrusterJson {
    #[serde(default)]
    subject: String,
    #[serde(default)]
    name: String,
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
        assert_eq!(OFF_SIGNATURE, 90);
        assert_eq!(REVOCATIONS_HEADER_LEN, 33);
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

    /// The proof message is the note's three parts, in that order, and the domain
    /// is NUL-terminated so no role can be glued onto another domain's words.
    #[test]
    fn the_auth_message_is_the_domain_then_the_role_then_the_exporter() {
        let exporter = [0xAA; 32];
        let message = auth_message(Role::Initiator, &exporter);
        assert_eq!(message.len(), 18 + 9 + 32);
        assert_eq!(&message[..18], b"neezer-lan/1 auth\0");
        assert_eq!(&message[18..27], b"initiator");
        assert_eq!(&message[27..], &exporter);
        assert_ne!(message, auth_message(Role::Responder, &exporter));
    }

    /// Base64 is one spelling of one body: the round trip is exact, and a body
    /// that is not canonical base64 is refused rather than repaired.
    #[test]
    fn base64_round_trips_and_refuses_non_canonical_forms() {
        for bytes in [
            &b""[..],
            &b"f"[..],
            &b"fo"[..],
            &b"foo"[..],
            &b"foob"[..],
            &b"fooba"[..],
            &b"foobar"[..],
            &[0u8, 255, 128, 1][..],
        ] {
            let text = base64_encode(bytes);
            assert_eq!(base64_decode(&text).as_deref(), Some(bytes), "« {text} »");
        }
        // Padding only in a padded length, never in the middle.
        assert_eq!(base64_decode("AAA"), Some(vec![0, 0]));
        assert_eq!(base64_decode("AA=="), Some(vec![0]));
        assert_eq!(base64_decode("AAA="), Some(vec![0, 0]));
        assert_eq!(base64_decode("A==="), None);
        assert_eq!(base64_decode("AAAA="), None);
        assert_eq!(base64_decode("A A="), None);
        assert_eq!(base64_decode("AA=A"), None);
        // Non-zero trailing bits: two spellings of one byte would otherwise exist.
        assert_eq!(base64_decode("AB=="), None);
    }

    /// Hex is the server's spelling, either case, and nothing else.
    #[test]
    fn hex_decodes_exactly_and_encodes_lowercase() {
        assert_eq!(hex_decode::<2>("0aFF", "test"), Ok([0x0a, 0xff]));
        assert!(hex_decode::<4>("0a", "test").is_err());
        assert!(hex_decode::<2>("0a0", "test").is_err());
        assert!(hex_decode::<2>("0g00", "test").is_err());
        assert_eq!(hex_encode(&[0x00, 0x0a, 0xff]), "000aff");
    }

    /// The cap constants are the note's numbers, so a changed one is a changed
    /// contract and no test can silently agree with it.
    #[test]
    fn the_caps_are_the_note_numbers() {
        assert_eq!(REVOCATIONS_MAX, 4096);
        assert_eq!(ROSTER_MAX_FRIENDS, 64);
        assert_eq!(ROSTER_MAX_TRACKS, 20_000);
        assert_eq!(ROSTER_MAX_BYTES, 4 << 20);
    }
}
