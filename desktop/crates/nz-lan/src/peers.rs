//! The connected peers, and an in-memory replica of each one's inventory
//! (docs/agents/lan-nodes.md, *Choosing the source*).
//!
//! **LN0 declares `Peers` and `Candidate`; LN9 fills them.**
//!
//! The replica is what makes the source decision instant: it is already in memory,
//! so a tap asks no peer anything (`0` control bytes and ≤ 1 ms — the note's
//! measure). It costs counted bytes ([`Peers::memory`], ≤ 64 B per item) and it is
//! filled once per peer and version, then by deltas.
//!
//! A replica can be STALE — a peer can evict a play-cache file between the
//! inventory and the fetch — so a `NotFound` from a peer is not an error about the
//! track: the decision is re-run with the remaining candidates, and then the
//! server answers.

use crate::service::PeerView;
use crate::transport::PeerConn;
use crate::types::{DevicePub, LanError, Relation};

/// One peer that has the track asked about, with everything needed to rank it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub peer: DevicePub,
    pub relation: Relation,
    /// The copies that peer announced, per quality and length.
    pub copies: Vec<(nz_api::Quality, u64)>,
    /// The connection's smoothed round trip, for the tie-break.
    pub rtt_ms: u32,
}

/// The peers this node is connected to.
///
/// Cheap to clone once LN9 is written (an `Arc` inside); the connection table and
/// the replicated inventories live behind it.
#[derive(Debug, Clone, Default)]
pub struct Peers;

impl Peers {
    /// No peer.
    pub fn new() -> Peers {
        Peers
    }

    /// Records an authenticated connection.
    pub fn insert(&self, conn: PeerConn) {
        let _ = conn;
    }

    /// Forgets a peer, its replica included.
    pub fn remove(&self, peer: &DevicePub) {
        let _ = peer;
    }

    /// The live connection to one peer, if there is one.
    pub fn get(&self, peer: &DevicePub) -> Option<PeerConn> {
        let _ = peer;
        None
    }

    /// Pulls a peer's inventory — full the first time, a delta on every version
    /// after — paginating `Inventory` and applying the answer. An inventory can be
    /// stale afterwards; the fetch is the truth.
    pub async fn sync_inventory(&self, peer: &DevicePub) -> Result<(), LanError> {
        let _ = peer;
        Err(LanError::Unsupported)
    }

    /// Every peer that has `track`, ranked by [`crate::decide`] (never by this
    /// call: it only lists them).
    pub fn candidates(&self, track: &str) -> Vec<Candidate> {
        let _ = track;
        Vec::new()
    }

    /// The one peer that has ALL of `tracks` complete, if any: "download the
    /// playlist from the device that has it" (`Lan::best_peer_for`).
    pub fn complete_for(&self, tracks: &[String]) -> Option<DevicePub> {
        let _ = tracks;
        None
    }

    /// Marks a peer's copy of a track unusable for this session: a wrong hash, a
    /// probe that failed, one bad file. The next decision skips it.
    pub fn mark_bad(&self, peer: &DevicePub, track: &str) {
        let _ = (peer, track);
    }

    /// What the peer list draws: one row per connection.
    pub fn views(&self) -> Vec<PeerView> {
        Vec::new()
    }

    /// The counted bytes of every replica (rule: never the RSS of a shared
    /// process). LN9's target is ≤ 64 B per item, so four peers with 8 000 tracks
    /// each ≈ 2 MB.
    pub fn memory(&self) -> usize {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A node with no peer has no candidate, no complete set and no memory to
    /// report: the decision that follows from this is "the server", instant and
    /// with no request (rule 3).
    #[test]
    fn no_peer_means_no_candidate() {
        let peers = Peers::new();
        assert!(peers.candidates("3135556").is_empty());
        assert_eq!(peers.complete_for(&["3135556".to_owned()]), None);
        assert!(peers.get(&[0; 32]).is_none());
        assert!(peers.views().is_empty());
        assert_eq!(peers.memory(), 0);
        peers.mark_bad(&[0; 32], "3135556");
        peers.remove(&[0; 32]);
    }
}
