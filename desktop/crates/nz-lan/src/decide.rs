//! Where a track will be taken from: the local disk, a peer, or the server
//! (docs/agents/lan-nodes.md, *Choosing the source*).
//!
//! **LN0 declares `decide`; LN9 fills it.** It is a pure function over data that is
//! already in memory — the replicated inventories, the local index, the tolerance
//! and the server's reachability — so the answer costs no request and under a
//! millisecond. That is what keeps rule 3 true on a bad link: a tap never waits on
//! the LAN to learn the LAN has nothing.
//!
//! ```text
//! 1. a complete local file (download, then play cache)      -> Local
//! 2. candidates = peers having the track, each with its qualities
//! 3. one has W, or a quality in T                           -> Peer(best)
//!    best = W if present, else the highest quality in T;
//!    ties: own device before friend, then complete for the set being
//!    downloaded, then lowest RTT
//! 4. else the server is reachable                           -> Server(W)
//! 5. else any candidate at all                              -> Peer(highest it has)
//! 6. else                                                   -> Server(W) (fails
//!    fast offline: rule 3)
//! ```
//!
//! Row 5 is the travel case: with no server, a worse copy beats silence, which is
//! why the tolerance only ever applies while the server could answer.

use std::path::PathBuf;

use crate::peers::Candidate;
use crate::types::{DevicePub, Source, Tolerance};

/// The decision, from data already in memory.
///
/// `local` is a complete local file if `nz-cache` has one, `prefer` the peer a
/// playlist download is already committed to (so its 50 tracks come from the same
/// device instead of scattering across the LAN).
pub fn decide(
    local: Option<PathBuf>,
    candidates: &[Candidate],
    wanted: nz_api::Quality,
    tolerated: &Tolerance,
    server_reachable: bool,
    prefer: Option<DevicePub>,
) -> Source {
    let _ = (candidates, tolerated, server_reachable, prefer);
    match local {
        Some(path) => Source::Local(path),
        // LN9 replaces this with the table above. Until then the answer is the
        // one that changes nothing: the server, at the quality asked.
        None => Source::Server(wanted),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// LN0's stub only proves the shape of the answer, and that a local file wins
    /// outright: `decide` must never send the person to the network for a file
    /// they already have.
    #[test]
    fn a_local_file_wins_and_nothing_else_is_decided_yet() {
        let local = PathBuf::from("/data/offline/stream-01.flac");
        assert_eq!(
            decide(
                Some(local.clone()),
                &[],
                nz_api::Quality::Opus320,
                &Tolerance::default_set(),
                true,
                None
            ),
            Source::Local(local)
        );
        assert_eq!(
            decide(
                None,
                &[],
                nz_api::Quality::Opus320,
                &Tolerance::default_set(),
                true,
                None
            ),
            Source::Server(nz_api::Quality::Opus320)
        );
    }
}
