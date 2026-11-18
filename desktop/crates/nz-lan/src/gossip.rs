//! The trust a peer can teach us: revocation lists are gossiped between any two
//! authenticated devices, rosters only between devices of the same subject
//! (docs/agents/lan-nodes.md, *Lifetime and revocation*).
//!
//! **LN7 filled `offers` and `accept`.**
//!
//! The rule this enforces is the one that bounds a revocation's residual window: a
//! device revoked on the server is refused by every peer that has since met ANY
//! device which saw the server. Serial numbers only grow per `kid`, a forged or
//! lower-serial list is ignored, and a roster that arrives from a subject that is
//! not mine is dropped — it names other people, and only the server may sign it.
//!
//! Three properties the implementation keeps, each measured in `tests/enroll.rs`:
//!
//! - **Nothing unverified is stored.** A blob is base64-decoded, verified against
//!   the server keys this device holds, and only then written — and the base64
//!   TEXT is capped *before* it is decoded, so a peer cannot make this device
//!   allocate a body it will refuse anyway.
//! - **A lower serial is silence, not an error.** Gossip converges: every node
//!   sends what it knows, and the receiver keeps the maximum. A refused blob
//!   answers `Ok(false)`; only a *malformed* one answers a `CredError`.
//! - **A roster only ever comes from one's own subject**, decided by the
//!   connection's authentication (`same_subject`), never by the frame's content.
//!   A peer that claims another subject's roster cannot even be asked to.

use std::collections::HashMap;

use crate::cred::{CredError, Revocations, Roster, TrustKeys, base64_decode, base64_encode};
use crate::proto;
use crate::store::Store;
use crate::transport::{SharedTrust, TrustState};
use crate::types::{Kid, Subject};

/// The `trust` row one revocation list is kept under (`revocations:<kid>`,
/// hex — the note's spelling).
pub(crate) fn revocations_key(kid: &Kid) -> String {
    format!("revocations:{}", crate::cred::hex_encode(&kid.0))
}

/// The `trust` row the roster is kept under.
pub(crate) const ROSTER_KEY: &str = "roster";

/// The most base64 text one gossip frame may carry before it is decoded.
///
/// A revocation list at its cap (4 096 keys) is 131 169 signed bytes, i.e.
/// 174 892 base64 characters; a roster's body is capped at 4 MiB by `cred.rs`
/// and cannot exceed the 1 MiB frame anyway. Capping at the frame's own limit
/// keeps the invariant honest: **never allocate a decoded body this device would
/// refuse**, and never trust a length the sender chose.
pub(crate) const MAX_BLOB_TEXT: usize = 1 << 20;

/// What to send a peer that just authenticated, so it learns anything newer this
/// device knows.
///
/// Answers at most one [`proto::Control::Revocations`] per `kid` where this device
/// has the higher serial, and at most one [`proto::Control::Roster`] when the peer
/// is the same subject and this device's roster serial is higher.
///
/// The frames are ordered by `kid` so two devices that know the same lists offer
/// them in the same order — nothing depends on hash iteration order.
pub fn offers(
    trust: &TrustState,
    peer_revocation_serial: u64,
    same_subject: bool,
    peer_roster_serial: Option<u64>,
) -> Vec<proto::Control> {
    let mut frames = Vec::new();
    let mut kids: Vec<&Kid> = trust.revocations.keys().collect();
    kids.sort_by_key(|kid| kid.0);
    for kid in kids {
        let Some(list) = trust.revocations.get(kid) else {
            continue;
        };
        // A list this device never received (`Revocations::empty`) has no bytes
        // to offer: an empty blob is not a list the sender could verify.
        if list.raw().is_empty() || list.serial <= peer_revocation_serial {
            continue;
        }
        frames.push(proto::Control::Revocations {
            blob: base64_encode(list.raw()),
        });
    }
    if same_subject
        && let Some(roster) = trust.roster.as_ref()
        && peer_roster_serial.is_none_or(|serial| roster.serial > serial)
    {
        frames.push(proto::Control::Roster {
            blob: base64_encode(&roster.raw_signed()),
        });
    }
    frames
}

/// Takes one control frame from a peer: verify it, keep it when it is newer, and
/// answer whether anything was stored.
///
/// `same_subject` is decided by the connection's authentication, never by the
/// frame: a roster is only ever accepted from one's own devices.
pub fn accept(
    trust: &SharedTrust,
    store: &Store,
    c: &proto::Control,
    same_subject: bool,
) -> Result<bool, CredError> {
    match c {
        proto::Control::Revocations { blob } => accept_revocations(trust, store, blob),
        proto::Control::Roster { blob } => {
            if !same_subject {
                // Not an error: a peer of another subject simply has nothing to
                // teach us, and saying so would tell it what we are.
                return Ok(false);
            }
            accept_roster(trust, store, blob)
        }
        // Every other frame is the transport's business, not trust's.
        _ => Ok(false),
    }
}

fn accept_revocations(trust: &SharedTrust, store: &Store, blob: &str) -> Result<bool, CredError> {
    let raw = decode_blob(blob)?;
    // The keys first (verification needs them), then the serial actually held for
    // this `kid` — never a clone of every list the device has (a list at its cap
    // is 131 KB).
    let keys = read(trust).keys.clone();
    let list = Revocations::decode_verify(&raw, &keys)?;
    let held = read(trust)
        .revocations
        .get(&list.kid)
        .map(|list| list.serial);
    if held.is_some_and(|serial| serial >= list.serial) {
        // Equal or older: the peer is behind us, which is the normal case after
        // both sides have synced. Silence, not a refusal to report.
        return Ok(false);
    }
    store
        .put_trust(&revocations_key(&list.kid), list.raw())
        .map_err(|error| CredError::Json(error.to_string()))?;
    let mut state = write(trust);
    // A concurrent gossip may have stored the same (or a newer) list between the
    // check and the write: keep the maximum, never the last arrival.
    if state
        .revocations
        .get(&list.kid)
        .is_none_or(|held| list.serial > held.serial)
    {
        state.revocations.insert(list.kid, list);
    }
    Ok(true)
}

fn accept_roster(trust: &SharedTrust, store: &Store, blob: &str) -> Result<bool, CredError> {
    let raw = decode_blob(blob)?;
    let (keys, subject, current_serial) = {
        let state = read(trust);
        (
            state.keys.clone(),
            state.subject,
            state.roster.as_ref().map(|r| r.serial),
        )
    };
    let roster = Roster::decode_verify(&raw, &keys)?;
    if roster.subject != subject {
        // Signed, verified, and still not mine: only my own devices hold a
        // roster about me, and this one is about somebody else.
        return Ok(false);
    }
    if current_serial.is_some_and(|serial| roster.serial <= serial) {
        return Ok(false);
    }
    store
        .put_trust(ROSTER_KEY, &roster.raw_signed())
        .map_err(|error| CredError::Json(error.to_string()))?;
    // The `roster.etag` row is left as it was on purpose: it describes the
    // SERVER's last answer, and the next sync sending a stale validator simply
    // gets one full answer back (never a wrong roster), then stores the fresh
    // one. `Store` has no per-key delete and this lot does not own that file.
    let mut state = write(trust);
    state.roster = Some(roster);
    Ok(true)
}

/// Base64 of one gossip blob, refused above [`MAX_BLOB_TEXT`] **before** any
/// allocation of the decoded size.
fn decode_blob(blob: &str) -> Result<Vec<u8>, CredError> {
    if blob.len() > MAX_BLOB_TEXT {
        return Err(CredError::Caps);
    }
    base64_decode(blob).ok_or_else(|| CredError::Json("blob base64 invalide".to_owned()))
}

/// The trust state, read. A poisoned lock is recovered rather than propagated:
/// `TrustState` is plain data and a panic elsewhere must not hide the trust this
/// device holds.
fn read(trust: &SharedTrust) -> std::sync::RwLockReadGuard<'_, TrustState> {
    trust
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write(trust: &SharedTrust) -> std::sync::RwLockWriteGuard<'_, TrustState> {
    trust
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The trust state a device with no server contact starts from: its subject, no
/// keys, no lists. Tests build it here so the gossip rules are exercised on the
/// same shape the service will hold.
pub fn empty_trust(subject: Subject) -> TrustState {
    TrustState {
        subject,
        keys: TrustKeys::default(),
        revocations: HashMap::new(),
        roster: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cred::base64_encode;
    use std::sync::{Arc, RwLock};

    fn trust() -> (SharedTrust, TrustState) {
        let state = TrustState {
            subject: Subject([1; 16]),
            keys: TrustKeys::default(),
            revocations: HashMap::from([(Kid([0; 8]), Revocations::empty(Kid([0; 8])))]),
            roster: None,
        };
        (Arc::new(RwLock::new(state.clone())), state)
    }

    /// With nothing newer to say, a device offers nothing: the gossip is not a
    /// heartbeat, and every frame it sends is a frame the other side verifies.
    #[test]
    fn a_device_that_knows_nothing_newer_offers_nothing() {
        let (shared, state) = trust();
        assert!(offers(&state, 0, true, None).is_empty());
        let stored = accept(&shared, &Store, &proto::Control::Bye, true)
            .expect("a refused frame is not an error");
        assert!(!stored);
    }

    /// An empty list (`Revocations::empty`, the "never heard of one" placeholder)
    /// has no bytes and is not offered: an empty blob would be a frame the other
    /// side must decode and refuse, for nothing.
    #[test]
    fn an_empty_placeholder_list_is_not_offered() {
        let (_, state) = trust();
        assert!(offers(&state, 0, true, None).is_empty());
    }

    /// A blob over the cap is refused as `Caps` without being decoded: the
    /// sender's length is never trusted to size an allocation.
    #[test]
    fn a_blob_over_the_cap_is_refused_before_decoding() {
        assert_eq!(
            decode_blob(&"A".repeat(MAX_BLOB_TEXT + 4)).err(),
            Some(CredError::Caps)
        );
        // The control: one byte under the cap still decodes.
        let under = base64_encode(&[0u8; 16]);
        assert_eq!(decode_blob(&under).expect("a small blob").len(), 16);
    }

    /// A roster is dropped without a word when the connection's subject is not
    /// ours — before it is even decoded, so a stranger's frame costs nothing.
    #[test]
    fn a_foreign_roster_is_ignored_before_it_is_decoded() {
        let (shared, _) = trust();
        let stored = accept(
            &shared,
            &Store,
            &proto::Control::Roster {
                blob: "not base64 at all !!".to_owned(),
            },
            false,
        )
        .expect("ignored, not refused");
        assert!(!stored);
    }

    /// A malformed blob that IS addressed to us is refused, and the trust state
    /// is untouched: a bug or a hostile peer must not move what we verify with.
    #[test]
    fn a_malformed_revocations_blob_is_refused_and_changes_nothing() {
        let (shared, _) = trust();
        let before = read(&shared).revocations.clone();
        let error = accept(
            &shared,
            &Store,
            &proto::Control::Revocations {
                blob: "!!!!".to_owned(),
            },
            true,
        );
        assert!(error.is_err());
        assert_eq!(read(&shared).revocations, before);
    }
}
