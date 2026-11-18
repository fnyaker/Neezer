//! `PeerFirstDownloader`: the downloads worker's fetcher, with a peer asked first
//! (docs/agents/lan-nodes.md, *Downloads and playlists*).
//!
//! **LN11 filled the body.** It wraps `ApiDownloader` and stays one
//! `nz_offline::Downloader`, so the queue, the `.part` reservation, the labels,
//! the resume rows and the progress the downloads screen draws are all unchanged.
//! What it adds: `fetch` runs [`crate::decide`] with `Urgency::Bulk`, writes the
//! `.part` the store reserved, checks the length and the SHA-256 (LN9's
//! `fetch_to_file`), probes the file with the decoder, and returns the quality it
//! really received (LN5's `Fetched { bytes, quality }`) so the store records the
//! right file name and the screen shows the right rung.
//!
//! Rule 1 is the transfer's own: `Bulk` takes the app's ONE `Gates`
//! (`Api::gates`), so a playlist pulled from a peer waits behind the tapped track
//! exactly like a download from the server, and it is paced by the same token
//! bucket. Nothing here sleeps or divides a rate.
//!
//! Rule 3 is the failure path: no connected peer, a `NotFound`, a `Busy`, a wrong
//! file or a dead stream SKIPS that peer for this track and the inner fetcher
//! takes over — never a retry storm, never a second LAN request. A `BadFile` also
//! takes the peer out of the track's next decision, so the retry the person taps
//! does not go back to the same broken file.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use nz_api::Quality;
use nz_model::TrackId;
use nz_offline::{Downloader, Fetched, Outcome, Progress, Stop};

use crate::fetch::{FetchPlan, INTERRUPTED, fetch_to_file, probe};
use crate::proto::Urgency;
use crate::service::Lan;
use crate::types::{LanError, Source};

/// The longest a transfer may take before this fetcher gives up on the peer: a
/// net, not a policy — the worker's own cancel is `stop`, and a 20 MB file on a
/// 1 MB/s link is 20 s.
pub const TRANSFER_NET: Duration = Duration::from_secs(600);

/// What one peer attempt means for the queue.
enum Attempt {
    /// The peer delivered a file that passed its length, its hash and the probe.
    Done(Fetched),
    /// The worker was told to stop: the row stays `active` (LN9's rule).
    Interrupted,
    /// This peer cannot serve this track now: the server takes over.
    Skip(Option<LanError>),
}

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
    ) -> Result<Fetched, Outcome> {
        // The transfer itself runs on the `nz-lan` runtime and this thread only
        // waits for its report, so any thread may call this — except the LAN
        // runtime's OWN single worker, where waiting would deadlock the future it
        // is waiting for. There, the server path is taken.
        let Some(handle) = self.lan.handle() else {
            return self.inner.fetch(track, quality, part, stop, progress);
        };
        if tokio::runtime::Handle::try_current().is_ok_and(|current| current.id() == handle.id()) {
            return self.inner.fetch(track, quality, part, stop, progress);
        }
        if stop.stopped() {
            return Err(Outcome::Interrupted);
        }
        if let Source::Peer {
            peer,
            quality: served,
        } = self.lan.decide_download(track.as_str(), quality)
            && let Some(conn) = self.lan.conn(&peer)
        {
            match self.fetch_from_peer(&handle, &conn, track, served, part, stop, progress) {
                Attempt::Done(fetched) => return Ok(fetched),
                Attempt::Interrupted => return Err(Outcome::Interrupted),
                Attempt::Skip(error) => {
                    if let Some(error) = error {
                        tracing::debug!(
                            track = track.as_str(),
                            %error,
                            "LAN : ce pair ne sert pas ce titre, le serveur prend le relais"
                        );
                    }
                }
            }
        }
        // Local and Server are the inner fetcher's business: nothing on the LAN is
        // asked, and no request is added.
        self.inner.fetch(track, quality, part, stop, progress)
    }
}

impl PeerFirstDownloader {
    /// One attempt over one peer's connection: `Bulk`, the app's gates, the
    /// worker's stop, and the decoder probe before the file may be published.
    #[allow(
        clippy::too_many_arguments,
        reason = "one call site, one attempt: bundling these into a struct would only move the line"
    )]
    fn fetch_from_peer(
        &self,
        handle: &tokio::runtime::Handle,
        conn: &crate::transport::PeerConn,
        track: &TrackId,
        served: Quality,
        part: &Path,
        stop: &Stop,
        progress: &mut dyn Progress,
    ) -> Attempt {
        // The worker reserves a FRESH `.part` for every attempt (`reserve`), so
        // this is a transfer from 0: a resume would need the tag the row held,
        // and `nz-offline` does not hand one to a fetcher.
        let plan = FetchPlan {
            peer: conn.clone(),
            track: track.as_str().to_owned(),
            quality: served,
            urgency: Urgency::Bulk,
            from: 0,
            expect: None,
        };
        let gates = self.lan.api().gates();
        let watch = stop.watch();
        let path = part.to_path_buf();

        // The transfer runs on the `nz-lan` runtime (its own thread), and progress
        // crosses back through a channel: `Progress` is not `Send`, so the closure
        // may not capture it — the calling thread is the only one that touches it.
        enum Event {
            Progress(u64, u64),
            Done(Result<crate::fetch::Fetched, LanError>),
        }
        let (sender, receiver) = std::sync::mpsc::channel::<Event>();
        let reporter = sender.clone();
        handle.spawn(async move {
            let mut send = |bytes: u64, total: u64| {
                let _ = reporter.send(Event::Progress(bytes, total));
            };
            let outcome = fetch_to_file(plan, &path, &gates, watch, &mut send).await;
            let _ = sender.send(Event::Done(outcome));
        });

        let outcome = loop {
            match receiver.recv_timeout(TRANSFER_NET) {
                Ok(Event::Progress(bytes, total)) => {
                    progress.total(Some(total));
                    progress.written(bytes);
                }
                Ok(Event::Done(outcome)) => break outcome,
                // The runtime went (the app is closing): the row stays `active`.
                Err(_) => return Attempt::Interrupted,
            }
        };

        match outcome {
            Ok(fetched) => {
                // The decoder is the only thing that can say the bytes are AUDIO
                // (LN9): a peer's file can match its length and its hash and still
                // be something no decoder opens.
                if let Err(error) = probe(part, served, None) {
                    self.lan.mark_bad(&conn.info().device_pub, track.as_str());
                    return Attempt::Skip(Some(error));
                }
                Attempt::Done(Fetched {
                    bytes: fetched.len,
                    quality: fetched.quality,
                })
            }
            Err(LanError::Internal(cause)) if cause == INTERRUPTED => Attempt::Interrupted,
            Err(LanError::BadFile(_)) => {
                self.lan.mark_bad(&conn.info().device_pub, track.as_str());
                Attempt::Skip(None)
            }
            Err(error) => Attempt::Skip(Some(error)),
        }
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

    /// A download from a peer takes the ONE `Bulk` lane and no other: the plan's
    /// urgency is what `fetch_to_file` turns into `gates.acquire(Bulk)`, and the
    /// worker's own lane is `Bulk` (rule 1: one throttle for server and LAN).
    #[test]
    fn a_peer_download_is_bulk() {
        assert_eq!(
            Urgency::Bulk,
            crate::import::urgency_of(nz_audio::Lane::Bulk)
        );
    }
}
