//! `PeerFirstDownloader`: the downloads worker's fetcher, with a peer asked first
//! (docs/agents/lan-nodes.md, *Downloads and playlists*).
//!
//! **LN0 declares the wrapper; LN11 fills the behaviour.**
//!
//! It wraps `ApiDownloader` and stays one `nz_offline::Downloader`, so the queue,
//! the `.part` reservation, the labels, the resume rows and the progress the
//! downloads screen draws are all unchanged. What LN11 adds: `fetch` runs
//! [`crate::decide`] with `Urgency::Bulk`, writes the `.part` the store reserved,
//! checks the length and the SHA-256, probes the file, and returns the quality it
//! really received (LN5's `Fetched { bytes, quality }`) so the store records the
//! right file name and the screen shows the right rung.
//!
//! Until LN11 every call goes to the inner fetcher: the wrapper is in the graph and
//! changes nothing.

use std::path::Path;
use std::sync::Arc;

use nz_api::Quality;
use nz_model::TrackId;
use nz_offline::{Downloader, Outcome, Progress, Stop};

use crate::service::Lan;

/// The downloads worker's fetcher with a peer asked first.
pub struct PeerFirstDownloader {
    inner: Arc<dyn Downloader>,
    lan: Lan,
}

impl PeerFirstDownloader {
    /// Wraps `inner`, asking `lan` before it for every track.
    pub fn new(inner: Arc<dyn Downloader>, lan: Lan) -> Self {
        PeerFirstDownloader { inner, lan }
    }

    /// The node behind this fetcher (LN11's playlist preference and the tests read
    /// it).
    pub fn lan(&self) -> &Lan {
        &self.lan
    }
}

impl Downloader for PeerFirstDownloader {
    /// Fetches one track into `part`, resuming when the row already holds bytes,
    /// and reports progress through the worker's own [`Progress`].
    ///
    /// One retry per peer (a `NotFound`/`Busy`/failure skips that peer for this
    /// track), then the inner fetcher — never a storm (rule 3).
    fn fetch(
        &self,
        track: &TrackId,
        quality: Quality,
        part: &Path,
        stop: &Stop,
        progress: &mut dyn Progress,
    ) -> Result<u64, Outcome> {
        // LN0: delegate, so wrapping changes nothing yet.
        self.inner.fetch(track, quality, part, stop, progress)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The worker calls a fetcher from its own thread and holds it behind an
    /// `Arc<dyn Downloader>`: the wrapper must be `Send + Sync` and one
    /// `Downloader`, or the queue cannot hold it. LN11 proves the behaviour with
    /// two real nodes.
    #[test]
    fn the_wrapper_is_one_send_sync_downloader() {
        fn assert_downloader<D: Downloader + Send + Sync>() {}
        assert_downloader::<PeerFirstDownloader>();
    }
}
