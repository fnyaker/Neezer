//! The trust a peer can teach us: revocation lists are gossiped between any two
//! authenticated devices, rosters only between devices of the same subject
//! (docs/agents/lan-nodes.md, *Lifetime and revocation*).
//!
//! **LN0 declares `offers` and `accept`; LN7 fills them.**
//!
//! The rule this enforces is the one that bounds a revocation's residual window: a
//! device revoked on the server is refused by every peer that has since met ANY
//! device which saw the server. Serial numbers only grow per `kid`, a forged or
//! lower-serial list is ignored, and a roster that arrives from a subject that is
//! not mine is dropped — it names other people, and only the server may sign it.

use crate::cred::CredError;
use crate::proto;
use crate::store::Store;
use crate::transport::{SharedTrust, TrustState};

/// What to send a peer that just authenticated, so it learns anything newer this
/// device knows.
///
/// Answers at most one [`proto::Control::Revocations`] per `kid` where this device
/// has the higher serial, and at most one [`proto::Control::Roster`] when the peer
/// is the same subject and this device's roster serial is higher.
pub fn offers(
    trust: &TrustState,
    peer_revocation_serial: u64,
    same_subject: bool,
    peer_roster_serial: Option<u64>,
) -> Vec<proto::Control> {
    let _ = (
        trust,
        peer_revocation_serial,
        same_subject,
        peer_roster_serial,
    );
    Vec::new()
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
    let _ = (trust, store, c, same_subject);
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cred::{Revocations, TrustKeys};
    use crate::types::{Kid, Subject};
    use std::collections::HashMap;
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
}
