//! The responder: answering a peer's `Inventory`, `Sets`, `Meta`, `Cover` and
//! `Fetch`, within that peer's [`Scope`] and within this device's own serve limits
//! (docs/agents/lan-nodes.md, *Rule 1 on the LAN*).
//!
//! **LN0 declares `Server` and `ServeLimits`; LN4 fills them.**
//!
//! Serving a peer is BACKGROUND work for this device (rule 1): at most
//! [`ServeLimits::default`]'s two `Play`, one `Prefetch` and one `Bulk` transfer at
//! once across all peers, a `Bulk` serve waits on this node's own
//! [`nz_api::lanes::Gates`] so it never slows what the person is listening to, and
//! a request beyond those limits is answered `Busy { retry_in_ms }` rather than
//! queued for ever. Reads go in 64 KiB chunks on a reniced `nz-lan` thread.
//!
//! Every handler takes `&Scope`: there is no handler without it, and out of scope
//! is answered by the same function that answers an absent file.

use std::sync::Arc;

use crate::index::Index;
use crate::scope::Scope;
use crate::transport::Incoming;
use crate::types::LanError;

/// How many transfers one device serves at once, per urgency.
///
/// Two `Play` (two of the person's own devices can tap a track at the same moment)
/// and one each of the rest: a third simultaneous `Play` gets `Busy`, which the
/// requester honours exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServeLimits {
    pub play: usize,
    pub prefetch: usize,
    pub bulk: usize,
}

impl Default for ServeLimits {
    fn default() -> Self {
        ServeLimits {
            play: 2,
            prefetch: 1,
            bulk: 1,
        }
    }
}

/// The responder: the local index, the gates it yields to, and the serve counts.
#[derive(Debug, Clone)]
pub struct Server {
    limits: ServeLimits,
}

impl Server {
    /// A responder over `index`, sharing this device's `gates` (the SAME
    /// `nz-api` gates the app's own requests take, so one Bulk slot covers server
    /// and LAN), with `limits` as its own serve budget.
    pub fn new(index: Index, gates: Arc<nz_api::lanes::Gates>, limits: ServeLimits) -> Server {
        let _ = (index, gates);
        Server { limits }
    }

    /// Answers one request, and answers the number of body bytes written.
    ///
    /// A request that hits a limit answers `Busy` and writes no body; one this
    /// peer may not see answers the same `NotFound` an absent file does.
    pub async fn handle(&self, scope: &Scope, incoming: Incoming) -> Result<u64, LanError> {
        let _ = (scope, incoming);
        Err(LanError::Unsupported)
    }

    /// How many transfers are being served right now: `(play, prefetch, bulk)`.
    /// The screens show it and the tests count it; nothing else reads it.
    pub fn counts(&self) -> (usize, usize, usize) {
        let _ = self.limits;
        (0, 0, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The note's serve budget: 2 Play, 1 Prefetch, 1 Bulk. A bigger number here
    /// is a device that serves four peers and stutters its own playback.
    #[test]
    fn the_default_serve_limits_are_two_one_one() {
        let limits = ServeLimits::default();
        assert_eq!((limits.play, limits.prefetch, limits.bulk), (2, 1, 1));
    }

    /// A stub serves nothing and counts nothing — never a non-zero count that a
    /// screen would draw as a transfer in progress.
    #[test]
    fn a_stub_server_counts_nothing() {
        let dir = std::env::temp_dir().join(format!("nz-lan-serve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let cache = nz_cache::Cache::open_with_cache_dir(&dir, dir.join("cache")).expect("cache");
        let server = Server::new(
            Index::new(cache, "1".to_owned(), crate::store::Store),
            Arc::new(nz_api::lanes::Gates::new()),
            ServeLimits::default(),
        );
        assert_eq!(server.counts(), (0, 0, 0));
        std::fs::remove_dir_all(&dir).ok();
    }
}
