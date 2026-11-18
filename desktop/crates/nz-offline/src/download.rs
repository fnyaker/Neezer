//! The seam a download goes through: what asks for a track's bytes, and what a
//! failure means for the queue.
//!
//! `nz-offline` never writes a body itself. It reserves a `.part` through
//! `nz-cache`, hands it to a [`Downloader`] with the stop it races, and takes
//! back the byte count; `complete` is what renames the file into place, so a
//! fetcher cannot make a half file look finished.
//!
//! [`ApiDownloader`] is the default one — `nz-api` on `Lane::Bulk`, which is
//! what the desktop app and the Android core use, and therefore the only
//! throttle a download has (rule 1: 1 s of quiet, one in flight,
//! `X-NS-Background: 1`, the stream-derived token bucket; there is no second
//! one here). A platform whose HTTP stack is not `reqwest` supplies its own
//! (`URLSession` on iOS, which is why this trait exists) and keeps the queue,
//! the store, the retry rules and the causes unchanged.
//!
//! Extracted from `desktop/app/src/wiring/offline.rs` (SC4).

use std::fs::File;
use std::future::Future;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use nz_api::{Api, ApiError, Lane, Quality, TrackId};
use tokio::sync::watch;

/// How often a running body republishes its byte count.
pub(crate) const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// What a download's failure means for the QUEUE: a dead link or a busy server
/// is "later", a provider verdict or a refused request is the track's cause.
#[derive(Debug)]
pub enum Outcome {
    /// Try again in that long, for that reason.
    Later(Duration, String),
    Offline,
    /// The worker was told to stop: the row stays `active` for the next start.
    Interrupted,
    /// A cause the person can read, stored with the row.
    Failed(String),
}

/// The stop of a worker, in the two forms a fetcher needs: a flag it checks
/// between two chunks, and a signal an `await` can be given up on.
///
/// One per worker ([`crate::Offline`]); cloning is cheap and every clone sees
/// the same stop. It replaces the bare flag + channel pair the worker used to
/// carry, because a fetcher outside this crate needs both halves and must not
/// reach into the worker's state to get them.
#[derive(Clone)]
pub struct Stop {
    flag: Arc<AtomicBool>,
    signal: watch::Sender<bool>,
}

impl Stop {
    /// A stop nothing has set yet.
    pub fn new() -> Stop {
        Stop {
            flag: Arc::new(AtomicBool::new(false)),
            signal: watch::channel(false).0,
        }
    }

    /// `true` once the worker was told to stop. A fast check, cheap enough for
    /// the top of a chunk loop.
    pub fn stopped(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }

    /// Set the stop, for every clone and every waiter. Idempotent.
    pub fn set(&self) {
        self.flag.store(true, Ordering::Release);
        // `send_replace` updates the value even with no receiver listening,
        // which `send` does not: a stop set before anything waited must still
        // be seen by the first `watch` (a worker stopped before its first
        // download).
        self.signal.send_replace(true);
    }

    /// Wait for the stop. `wait_for` answers at once when it is already set.
    pub fn watch(&self) -> watch::Receiver<bool> {
        self.signal.subscribe()
    }

    /// Run `future` on `handle`, giving it up the moment the worker is told to
    /// stop.
    ///
    /// `None` means « the app is closing »: the future was dropped before it
    /// finished (a request waiting in a lane, a body in flight), and the caller
    /// turns that into [`Outcome::Interrupted`] rather than a verdict about the
    /// track.
    ///
    /// There is deliberately NO `catch_unwind` here. The worker must never reach
    /// a runtime that is shutting down — tokio PANICS on
    /// `TimerEntry::poll_elapsed` — and joining the thread before the runtime
    /// goes is what guarantees it (`crate::service::Worker::stop`). A panic
    /// swallowed here would turn a real bug into « le réseau est coupé » and
    /// still print the message.
    pub fn drive<T>(
        &self,
        handle: &tokio::runtime::Handle,
        future: impl Future<Output = T>,
    ) -> Option<T> {
        let mut stop = self.watch();
        handle.block_on(async {
            tokio::select! {
                biased;
                // `Err` means the sender is gone: the service that could still
                // ask us to stop went with it, so stop.
                _ = stop.wait_for(|gone| *gone) => None,
                value = future => Some(value),
            }
        })
    }
}

impl Default for Stop {
    fn default() -> Self {
        Stop::new()
    }
}

/// What a fetcher reports while it writes, so the screen shows the bar move
/// without the fetcher knowing what a screen is.
pub trait Progress {
    /// The size the server announced, when it announced one (absent for a body
    /// generated as it is sent).
    fn total(&mut self, total: Option<u64>);
    /// Bytes written into the `.part` so far.
    fn written(&mut self, bytes: u64);
}

/// What a fetcher delivered: the bytes it wrote, and the quality they really
/// are.
///
/// The quality is not always the one the row asked for. `ApiDownloader` answers
/// the rung it requested — the server transcodes to order — but a peer on the
/// LAN serves the copy it already holds, so a track reserved at Opus 320 can
/// come back as FLAC. The worker records this one, and the file is then named,
/// labelled and cached for what it is (`docs/agents/lan-nodes.md`,
/// *Shared-crate changes*, LN5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fetched {
    /// Bytes written into the `.part` the store reserved.
    pub bytes: u64,
    /// The quality of those bytes.
    pub quality: Quality,
}

/// What fetches one track's bytes into the `.part` the store reserved.
///
/// `fetch` owns the body: it opens `part`, writes it, and answers what it
/// delivered — the byte count and the quality those bytes really are. It
/// publishes nothing and marks no row — the worker owns those, and a `0` is the
/// worker's « corps vide » cause, not a success.
pub trait Downloader: Send + Sync {
    fn fetch(
        &self,
        track: &TrackId,
        quality: Quality,
        part: &Path,
        stop: &Stop,
        progress: &mut dyn Progress,
    ) -> Result<Fetched, Outcome>;

    /// Whether this fetcher has a SOURCE for the queue right now.
    ///
    /// The worker asks this before it declares itself offline (`service::run`).
    /// The server is only one possible source: a fetcher that has another one — a
    /// peer on the local network (`docs/agents/lan-nodes.md`, LN11's addendum) —
    /// must be able to say so, or a trip without data could never take a playlist
    /// from a peer.
    ///
    /// The default is `true`: a fetcher that does not name its source is assumed
    /// to have one, which is what a platform's own downloader is, and what keeps
    /// every existing custom fetcher's behaviour. [`ApiDownloader`] answers the
    /// server's own verdict, because that is the only source it has.
    ///
    /// It must be LOCAL and cheap: the worker calls it on every idle poll (2 s)
    /// and it must never touch the network — rule 1, no request from a background
    /// loop.
    fn reachable(&self) -> bool {
        true
    }
}

/// The default fetcher: `nz-api` on `Lane::Bulk`.
///
/// The lane IS the throttle (CLAUDE.md rule 1): its gates wait for a second of
/// quiet, keep one Bulk request in flight, send `X-NS-Background: 1` and pace
/// the body at 25 % of the playing stream's measured throughput. Nothing here
/// sleeps or divides a rate, so a second throttle cannot disagree with the
/// first.
pub struct ApiDownloader {
    api: Arc<Api>,
    runtime: tokio::runtime::Handle,
}

impl ApiDownloader {
    /// A fetcher for this session: the client and the runtime the worker
    /// borrows (`Offline::configure` builds one on every login).
    pub fn new(api: Arc<Api>, runtime: tokio::runtime::Handle) -> ApiDownloader {
        ApiDownloader { api, runtime }
    }
}

impl Downloader for ApiDownloader {
    /// The server is this fetcher's only source: its own reachability verdict is
    /// the answer, exactly what the worker used to ask itself.
    fn reachable(&self) -> bool {
        self.api.is_online()
    }

    fn fetch(
        &self,
        track: &TrackId,
        quality: Quality,
        part: &Path,
        stop: &Stop,
        progress: &mut dyn Progress,
    ) -> Result<Fetched, Outcome> {
        let file =
            File::create(part).map_err(|error| Outcome::Failed(format!("disque : {error}")))?;
        let mut out = BufWriter::with_capacity(64 * 1024, file);

        let mut response = match stop.drive(
            &self.runtime,
            self.api.stream(Lane::Bulk, track, quality, None, None),
        ) {
            Some(Ok(response)) => response,
            Some(Err(error)) => return Err(classify(error)),
            // The app closed while the request was in a lane: nothing was read,
            // so the empty `.part` this opened goes, and the row stays `active`
            // for the next launch.
            None => {
                drop(out);
                let _ = std::fs::remove_file(part);
                return Err(Outcome::Interrupted);
            }
        };
        progress.total(response.content_length);

        let mut bytes = 0u64;
        let mut published = Instant::now();
        loop {
            if stop.stopped() {
                let _ = out.flush();
                return Err(Outcome::Interrupted);
            }
            match stop.drive(&self.runtime, response.chunk()) {
                Some(Ok(Some(chunk))) => {
                    if let Err(error) = out.write_all(&chunk) {
                        return Err(Outcome::Failed(format!("écriture : {error}")));
                    }
                    bytes += chunk.len() as u64;
                    if published.elapsed() >= PROGRESS_EVERY {
                        published = Instant::now();
                        progress.written(bytes);
                    }
                }
                Some(Ok(None)) => break,
                Some(Err(error)) => {
                    let _ = out.flush();
                    return Err(classify(error));
                }
                // The app closed mid-body: keep the `.part`, write nothing, and
                // let the row stay `active` — the next launch fetches it again
                // rather than trusting a truncated file.
                None => {
                    let _ = out.flush();
                    return Err(Outcome::Interrupted);
                }
            }
        }
        if let Err(error) = out.flush() {
            return Err(Outcome::Failed(format!("écriture : {error}")));
        }
        drop(out);
        if bytes == 0 {
            return Err(Outcome::Failed(
                "le serveur a répondu un corps vide".to_owned(),
            ));
        }
        // The server transcodes to order: what was asked for is what arrived.
        Ok(Fetched { bytes, quality })
    }
}

/// What a request's failure means for the QUEUE: a dead link or a busy server is
/// "later", a provider verdict or a refused request is the track's cause.
fn classify(error: ApiError) -> Outcome {
    match error {
        ApiError::Offline => Outcome::Offline,
        ApiError::Busy { retry_after } | ApiError::NotReady { retry_after } => Outcome::Later(
            Duration::from_secs(retry_after.max(1) as u64),
            format!("le serveur prépare ce titre (réessai dans {retry_after} s)"),
        ),
        ApiError::Unauthorized => Outcome::Later(
            Duration::from_secs(5),
            "session expirée : reconnectez-vous".to_owned(),
        ),
        other => Outcome::Failed(other.cause_fr()),
    }
}
