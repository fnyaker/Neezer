//! Taking a whole playlist (or album, or show) from a peer that has it complete
//! (docs/agents/lan-nodes.md, *Downloads and playlists*).
//!
//! **LN0 declares `PeerSetKey` and `download_peer_sets`; LN11 fills them.**
//!
//! A playlist download is an ordinary `nz-offline` job — it shows its progress and
//! each failure its cause (rule 2) — but when a peer has every track, the job asks
//! that ONE device for all of them ([`crate::service::Lan::best_peer_for`]): the
//! LAN is 20–80 MB/s where the server is 1–5, and a peer does not split a set
//! across four connections.
//!
//! The track list itself comes from the server when it is reachable, and from a
//! peer's `Sets` when it is not: a trip without data still lets the phone take a
//! playlist the PC already has.

use crate::service::Lan;
use crate::types::{DevicePub, LanError};

/// One set to take from one peer: « Sur vos appareils » multi-selects these.
///
/// The `"<peer>:<kind>:<ident>"` string the FFI passes around is this triple, and
/// it is parsed into it before anything is asked of a peer — a key never becomes a
/// path and never reaches the network as one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerSetKey {
    pub peer: DevicePub,
    /// `"playlist"`, `"album"` or `"show"`.
    pub kind: String,
    /// The server's id for the set.
    pub ident: String,
}

/// Adds every set in `keys` to the downloads worker, and answers how many jobs were
/// created.
///
/// The peer's completeness is re-checked against its live inventory here: a set can
/// lose a file between the screen drawing it and the person tapping « Télécharger »,
/// and a job that half-fails is worse than a set that is offered again.
pub fn download_peer_sets(
    lan: &Lan,
    offline: &nz_offline::Offline,
    keys: &[PeerSetKey],
) -> Result<usize, LanError> {
    let _ = (lan, offline, keys);
    Err(LanError::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stub creates no job: the downloads screen must not gain an empty row for
    /// a feature that does not work yet.
    #[test]
    fn a_stub_key_is_the_triple_the_ffi_parses() {
        let key = PeerSetKey {
            peer: [7; 32],
            kind: "playlist".to_owned(),
            ident: "42".to_owned(),
        };
        assert_eq!(key.kind, "playlist");
        assert_eq!(key.ident, "42");
        assert_eq!(key.peer[0], 7);
    }
}
