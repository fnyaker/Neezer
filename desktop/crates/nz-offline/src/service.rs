//! The downloads worker, and the snapshot a screen reads.
//!
//! This is lot D13's worker, moved out of `desktop/app/src/wiring/offline.rs`
//! (SC4) so the desktop app and the mobile core run the SAME one. It owns
//! exactly four things, and nothing else:
//!
//! - **The `nz-bulk` worker.** One thread, `nice 10`, that drains the
//!   `nz-cache` download queue through `Lane::Bulk`. Rule 1 is not re-invented
//!   here: a download declares `Lane::Bulk`, so `nz-api`'s gates make it wait a
//!   second of quiet, serialise it against itself, send `X-NS-Background: 1`,
//!   and pace its body with a token bucket at 25 % of the playing stream's
//!   measured throughput (never under 64 kB/s). The meter that feeds that cap is
//!   [`Offline::note_stream`], called from the player's own reader.
//! - **The published state a screen reads.** A [`crate::state::Snapshot`] is a
//!   small, cloned value, republished off the UI thread and read with a mutex
//!   and a clone — never a SQLite statement on the GPUI thread (`nz-cache`'s own
//!   rule).
//! - **The seam the engine uses.** [`Offline::local`] answers where a downloaded
//!   track is, at the quality it was fetched at, so a track chosen for offline
//!   play still plays with the network cut; the engine's opener asks it before
//!   it reaches the network.
//! - **The fetcher seam.** [`Downloader`] is what asks for a track's bytes —
//!   [`ApiDownloader`] on `nz-api` by default — so a platform with its own HTTP
//!   stack supplies one and keeps this queue, this store, these causes.
//!
//! The store itself is D3's (`nz_cache::Downloads`): a set and its tracks are
//! one transaction, a track in three sets is downloaded once, a file is
//! published only once complete (`.part` + rename + length read back), and
//! nothing deletes a download but the person. There is deliberately no second
//! queue, no in-memory membership and no path arithmetic here.
//!
//! A failure is never a verdict about a track (CLAUDE.md): a dead link or a
//! server that answered `503 Retry-After` keeps the row PENDING and the worker
//! comes back, exactly like the SPA's `netyield`/`offline.js`. Only the server's
//! own `X-NS-Error` classification, a `4xx`, or a full disk marks a row failed,
//! and then the CAUSE is stored and shown (rule 2), with « Réessayer ».

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use nz_api::{Api, Quality, TrackId};
use nz_cache::{Cache, DownloadState, DownloadTrack};
use tokio::sync::watch;
use tracing::{debug, info, warn};

use crate::download::{ApiDownloader, Downloader, Fetched, Outcome, Progress, Stop};
use crate::settings::{COVERS_CAP_BYTES, LABELS_KEY, Settings, bytes_label, quality_label};
use crate::state::{Choice, Failure, Labels, Running, Snapshot, WorkerState};

#[derive(Debug)]
enum Job {
    Choose {
        kind: String,
        ident: String,
        title: String,
        tracks: Vec<Choice>,
    },
    DropSet {
        kind: String,
        ident: String,
    },
    DropTrack {
        track: String,
    },
    RetryFailures,
    SaveSettings,
    Sweep,
}

struct Meter {
    /// Bytes counted since `at`.
    bytes: u64,
    at: Option<Instant>,
    /// Bytes per second, EWMA over 250 ms windows (the plan's "EWMA over the
    /// last three streams"; a window is cheaper and reacts in a second).
    rate: f64,
    /// When a stream last delivered bytes, so a finished stream releases the cap.
    fed_at: Option<Instant>,
}

impl Default for Meter {
    fn default() -> Self {
        Self {
            bytes: 0,
            at: None,
            rate: 0.0,
            fed_at: None,
        }
    }
}

impl Meter {
    /// Forget the measured rate: a stopped stream has no throughput to yield to.
    fn clear(&mut self) {
        self.rate = 0.0;
        self.bytes = 0;
        self.at = None;
        self.fed_at = None;
    }
}

pub(crate) struct Shared {
    cache: Option<Cache>,
    api: Mutex<Option<Arc<Api>>>,
    /// What turns a track into bytes: `ApiDownloader` for a session, or
    /// whatever a platform supplied (`Offline::configure_with`). It holds the
    /// network runtime it drives, so the worker does not keep a second copy.
    downloader: Mutex<Option<Arc<dyn Downloader>>>,
    settings: Mutex<Settings>,
    snapshot: Mutex<Snapshot>,
    labels: Mutex<HashMap<String, Labels>>,
    paused: AtomicBool,
    /// Set by `Offline::shutdown` (the app closing, a test's "the process ends
    /// here"): the loop's fast check, between two chunks and at its top, and the
    /// signal every call into the network runtime is raced against, so a stop
    /// abandons a request in flight instead of waiting for the next chunk.
    stop: Stop,
    jobs: Mutex<VecDeque<Job>>,
    /// The condvar's own lock: the worker sleeps on it and every request wakes
    /// it, so a click never waits for the next poll.
    gate: Mutex<()>,
    wake: Condvar,
    meter: Mutex<Meter>,
    version: AtomicU64,
    /// The same publications as a signal, for a consumer that would otherwise
    /// poll [`Offline::snapshot`] (the phone's listener). The value is the new
    /// `version`; `watch` keeps only the latest, which is what "read it again"
    /// needs.
    events: watch::Sender<u64>,
}

impl Shared {
    pub(crate) fn lock<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
        // A panic in another task must not turn every later call into a panic:
        // the state behind these locks is plain data.
        lock.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn snapshot(&self) -> MutexGuard<'_, Snapshot> {
        Self::lock(&self.snapshot)
    }

    /// Change the published snapshot, bump its version, and tell whoever waits
    /// on [`Offline::events`].
    fn publish(&self, change: impl FnOnce(&mut Snapshot)) {
        let mut snapshot = self.snapshot();
        change(&mut snapshot);
        snapshot.version = self.version.fetch_add(1, Ordering::AcqRel) + 1;
        let version = snapshot.version;
        drop(snapshot);
        self.events.send_replace(version);
    }

    fn nudge(&self) {
        let _gate = Self::lock(&self.gate);
        self.wake.notify_all();
    }

    /// Sleep until something is asked for, or `timeout`.
    fn wait(&self, timeout: Duration) {
        let gate = Self::lock(&self.gate);
        let _ = self
            .wake
            .wait_timeout(gate, timeout)
            .unwrap_or_else(|error| error.into_inner());
    }

    fn settings(&self) -> Settings {
        *Self::lock(&self.settings)
    }
}

/// The offline service: a worker and the state it publishes.
///
/// Cheap to clone (`Arc` inside); the app has exactly one per process
/// ([`Offline::get`]), and a test makes its own with [`Offline::start`] so two
/// tests never share a worker.
#[derive(Clone)]
pub struct Offline {
    shared: Arc<Shared>,
    /// The thread itself, whose life is tied to the runtime it borrows: see
    /// [`Worker`]. Kept here so the LAST clone to drop joins it.
    worker: Arc<Worker>,
}

/// The worker's thread, owned so that the runtime it borrows cannot go first.
///
/// `nz-bulk` drives every request with `Handle::block_on` on the process's
/// tokio runtime, and tokio **panics** — it does not return an error — when a
/// timer is polled on a runtime that is shutting down
/// (`runtime::time::entry::TimerEntry::poll_elapsed`: « A Tokio 1.x context was
/// found, but it is being shutdown. »). Measured in CI (run 43964917017): the
/// bench's runtime was dropped under a worker that still had 200 tracks queued,
/// every test passed, and the thread died with that message.
///
/// So the worker is JOINED before the runtime it borrows is dropped, and the two
/// owners do it: the app's `NetRuntime` global calls [`shutdown_process`] from
/// its `Drop` (before its own `Arc<Runtime>` goes), and a test's last `Offline`
/// clone joins here. Dropping that runtime while an `Offline` lives is the one
/// thing nothing may do.
struct Worker {
    shared: Arc<Shared>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl Worker {
    /// Stop the loop and wait for the thread: once this returns, nothing can
    /// reach the network runtime from `nz-bulk` any more. Idempotent, and the
    /// second caller waits for the first one's join (the mutex is held across
    /// it) instead of returning while the thread still runs.
    fn stop(&self) {
        self.shared.stop.set();
        self.shared.nudge();
        let mut thread = Shared::lock(&self.thread);
        if let Some(thread) = thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
    }
}

static SERVICE: OnceLock<Offline> = OnceLock::new();

impl Offline {
    /// Start a worker for `store`. The thread is named `nz-bulk` and runs
    /// `nice 10` (rule 1: downloads are background work, never the person's).
    pub fn start(store: Cache) -> Offline {
        let settings = Settings::read(&store);
        let labels = store
            .kv()
            .get::<HashMap<String, Labels>>(LABELS_KEY)
            .unwrap_or_default();
        let shared = Arc::new(Shared {
            cache: Some(store),
            api: Mutex::new(None),
            downloader: Mutex::new(None),
            settings: Mutex::new(settings),
            snapshot: Mutex::new(Snapshot {
                settings,
                ..Snapshot::default()
            }),
            labels: Mutex::new(labels),
            paused: AtomicBool::new(false),
            stop: Stop::new(),
            jobs: Mutex::new(VecDeque::new()),
            gate: Mutex::new(()),
            wake: Condvar::new(),
            meter: Mutex::new(Meter::default()),
            version: AtomicU64::new(0),
            events: watch::channel(0).0,
        });
        let worker = Arc::new(Worker {
            shared: Arc::clone(&shared),
            thread: Mutex::new(None),
        });
        let looped = Arc::clone(&shared);
        let spawned = std::thread::Builder::new()
            .name("nz-bulk".to_owned())
            .spawn(move || {
                lower_priority();
                run(looped);
            });
        match spawned {
            Ok(thread) => *Shared::lock(&worker.thread) = Some(thread),
            Err(error) => {
                Shared::lock(&shared.snapshot).state =
                    WorkerState::Blocked(format!("le worker hors ligne n'a pas démarré : {error}"));
            }
        }
        Offline { shared, worker }
    }

    /// The process's service, configured (or reconfigured) from the session.
    ///
    /// Called at every login and restore: the `Api` is rebuilt per server and
    /// the owner changes with the account, while the store and the worker live
    /// for the process's life — so a download started before a restart is picked
    /// up again (rule 2).
    pub fn ensure(api: Arc<Api>, store: Cache, runtime: tokio::runtime::Handle) -> Offline {
        let service = SERVICE.get_or_init(|| Offline::start(store));
        service.configure(api, runtime);
        service.clone()
    }

    /// The process's service, when one was started.
    pub fn get() -> Option<Offline> {
        SERVICE.get().cloned()
    }

    /// Point the worker at a session, with the default fetcher (`nz-api` on
    /// `Lane::Bulk`).
    pub fn configure(&self, api: Arc<Api>, runtime: tokio::runtime::Handle) {
        let downloader = Arc::new(ApiDownloader::new(Arc::clone(&api), runtime));
        self.configure_with(api, downloader);
    }

    /// Point the worker at a session and at the fetcher that will serve it.
    ///
    /// A platform whose HTTP stack is not `reqwest` passes its own
    /// [`Downloader`]: the queue, the store, the retry rules and the causes are
    /// this crate's either way, and the runtime such a fetcher needs is its own
    /// (the desktop's goes in [`ApiDownloader::new`]).
    pub fn configure_with(&self, api: Arc<Api>, downloader: Arc<dyn Downloader>) {
        *Shared::lock(&self.shared.api) = Some(api);
        *Shared::lock(&self.shared.downloader) = Some(downloader);
        self.shared.publish(|snapshot| {
            if matches!(snapshot.state, WorkerState::Unavailable) {
                snapshot.state = WorkerState::Idle;
            }
        });
        self.shared.nudge();
    }

    /// What a view draws. A mutex and a clone: never a statement on the UI
    /// thread. The same value [`Offline::add_set`] and its neighbours describe,
    /// so one word per thing across the two apps.
    pub fn snapshot(&self) -> Snapshot {
        Shared::lock(&self.shared.snapshot).clone()
    }

    /// The published state as a signal, for a consumer that would otherwise
    /// poll: every value is one of [`Offline::snapshot`]'s `version`s, so a
    /// waiter reads the snapshot once per change and never on a timer.
    ///
    /// `watch` keeps only the latest value, which is what "read it again"
    /// needs; a burst of publications costs one wake-up.
    pub fn events(&self) -> watch::Receiver<u64> {
        self.shared.events.subscribe()
    }

    /// Queue a playlist, an album, a show or a single track for offline — the
    /// mobile core's name for [`Offline::choose`].
    pub fn add_set(&self, kind: &str, ident: &str, title: &str, tracks: Vec<Choice>) {
        self.choose(kind, ident, title, tracks);
    }

    /// Take a set back, files and all — the mobile core's name for
    /// [`Offline::drop_set`].
    pub fn remove_set(&self, kind: &str, ident: &str) {
        self.drop_set(kind, ident);
    }

    /// Give every failure another chance — the mobile core's name for
    /// [`Offline::retry_failures`].
    pub fn retry(&self) {
        self.retry_failures();
    }

    /// Publish a state a test wants to draw, without a session or a disk.
    ///
    /// Public, not `#[cfg(test)]`: the desktop screens' layout benches live in
    /// the app crate, which compiles this one without `cfg(test)` (SC4). It is
    /// the same `publish` the worker uses, so a bench cannot paint something the
    /// worker could not.
    pub fn publish_for_test(&self, change: impl FnOnce(&mut Snapshot)) {
        self.shared.publish(change);
    }

    /// The Bulk cap the meter currently derives from the playing stream, in
    /// bytes per second. A test reads the MECHANISM here instead of inferring it
    /// from a timing, which a loaded machine makes noisy.
    #[cfg(test)]
    pub fn bulk_cap_for_test(&self) -> Option<u64> {
        let meter = Shared::lock(&self.shared.meter);
        (meter.rate > 0.0)
            .then(|| nz_api::bulk_cap_from_stream(Some(meter.rate as u64)).unwrap_or(0))
    }

    /// Choose a playlist, an album, a show or a single track for offline.
    ///
    /// `kind` is the store's own word (`playlist`, `album`, `show`, `track`); a
    /// single track is a set of one, which is what makes it appear in the same
    /// list and be removed the same way.
    pub fn choose(&self, kind: &str, ident: &str, title: &str, tracks: Vec<Choice>) {
        if tracks.is_empty() {
            self.shared.publish(|snapshot| {
                snapshot.note = Some(format!("« {title} » ne contient aucun titre à télécharger"));
            });
            return;
        }
        // Optimistic: the row exists in the UI before the transaction lands, and
        // the worker corrects it one frame later (rule 4).
        self.shared.publish(|snapshot| {
            snapshot.note = Some(format!("« {title} » ajouté aux téléchargements"));
        });
        Shared::lock(&self.shared.jobs).push_back(Job::Choose {
            kind: kind.to_owned(),
            ident: ident.to_owned(),
            title: title.to_owned(),
            tracks,
        });
        self.shared.nudge();
    }

    /// Choose one track (« télécharger ce titre » in a row's menu).
    pub fn choose_track(&self, choice: Choice) {
        let title = choice.title.clone();
        let track = choice.track.clone();
        self.choose("track", &track, &title, vec![choice]);
    }

    /// Take a set back, files and all. Returns immediately; the worker does the
    /// deleting so the click never waits on the disk.
    pub fn drop_set(&self, kind: &str, ident: &str) {
        Shared::lock(&self.shared.jobs).push_back(Job::DropSet {
            kind: kind.to_owned(),
            ident: ident.to_owned(),
        });
        self.shared.nudge();
    }

    pub fn drop_track(&self, track: &str) {
        Shared::lock(&self.shared.jobs).push_back(Job::DropTrack {
            track: track.to_owned(),
        });
        self.shared.nudge();
    }

    /// Give the failures another chance: every `failed` row goes back to the
    /// queue, with its cause cleared.
    pub fn retry_failures(&self) {
        Shared::lock(&self.shared.jobs).push_back(Job::RetryFailures);
        self.shared.nudge();
    }

    /// Re-run the eviction sweep now (a settings change, or « Vider »).
    pub fn sweep_now(&self) {
        Shared::lock(&self.shared.jobs).push_back(Job::Sweep);
        self.shared.nudge();
    }

    pub fn pause(&self) {
        self.shared.paused.store(true, Ordering::Release);
        self.shared.publish(|snapshot| {
            if snapshot.state == WorkerState::Running {
                snapshot.state = WorkerState::Pausing;
            }
        });
        self.shared.nudge();
    }

    pub fn resume(&self) {
        self.shared.paused.store(false, Ordering::Release);
        self.shared.nudge();
    }

    pub fn paused(&self) -> bool {
        self.shared.paused.load(Ordering::Acquire)
    }

    pub fn settings(&self) -> Settings {
        self.shared.settings()
    }

    /// Change one setting: published at once (the switch answers the click), then
    /// persisted and applied by the worker.
    pub fn set_quality(&self, quality: Quality) {
        Shared::lock(&self.shared.settings).quality = quality;
        self.shared.publish(|snapshot| {
            snapshot.settings.quality = quality;
            snapshot.note = Some(format!("Qualité : {}", quality_label(quality)));
        });
        Shared::lock(&self.shared.jobs).push_back(Job::SaveSettings);
        self.shared.nudge();
    }

    pub fn set_offline_only(&self, offline_only: bool) {
        Shared::lock(&self.shared.settings).offline_only = offline_only;
        self.shared
            .publish(|snapshot| snapshot.settings.offline_only = offline_only);
        Shared::lock(&self.shared.jobs).push_back(Job::SaveSettings);
        self.shared.nudge();
    }

    pub fn set_play_cache_cap(&self, bytes: i64) {
        if bytes <= 0 {
            return;
        }
        Shared::lock(&self.shared.settings).play_cache_cap = bytes;
        self.shared
            .publish(|snapshot| snapshot.settings.play_cache_cap = bytes);
        Shared::lock(&self.shared.jobs).push_back(Job::SaveSettings);
        Shared::lock(&self.shared.jobs).push_back(Job::Sweep);
        self.shared.nudge();
    }

    /// Drop the whole playback cache. Downloads are untouched: they are not a
    /// cache, and nothing but the person deletes one.
    pub fn clear_play_cache(&self) {
        let Some(store) = self.shared.cache.as_ref() else {
            return;
        };
        match store.play_cache().clear() {
            Ok(freed) => {
                info!(freed, "cache de lecture vidé");
                self.shared.publish(|snapshot| {
                    snapshot.play_cache_bytes = 0;
                    snapshot.note = Some("Cache de lecture vidé".to_owned());
                });
            }
            Err(error) => {
                let cause = format!("cache de lecture : {error}");
                warn!(%cause);
                self.shared.publish(|snapshot| snapshot.note = Some(cause));
            }
        }
    }

    /// Where a downloaded track is, at the quality it was fetched at.
    ///
    /// This is the engine's offline seam ([`local_path`] is its global form): a
    /// track chosen for offline plays with the link cut, whatever the quality on
    /// the bar says — the bytes on disk are the only ones there are.
    pub fn local(&self, track: &str) -> Option<(PathBuf, Quality)> {
        let store = self.shared.cache.as_ref()?;
        let owner = Shared::lock(&self.shared.api)
            .as_ref()
            .and_then(|api| api.owner())?;
        store
            .downloads()
            .local(&owner, &TrackId::new(track))
            .ok()
            .flatten()
    }

    /// `true` when `track` is downloaded and playable.
    pub fn is_downloaded(&self, track: &str) -> bool {
        self.local(track).is_some()
    }

    /// End the worker: what a process ending does (and what a restart test
    /// needs). The row being written stays `active` in the store, which is
    /// exactly what the next start finds.
    ///
    /// It WAITS for the thread ([`Worker::stop`]). The app's network runtime is
    /// borrowed by that thread with `Handle::block_on`, and tokio panics when a
    /// timer is polled on a shutting-down runtime, so a caller that owns the
    /// runtime must see this return before dropping it: `NetRuntime`'s `Drop`
    /// does exactly that ([`shutdown_process`]).
    pub fn shutdown(&self) {
        self.worker.stop();
    }

    pub(crate) fn note_stream(&self, bytes: u64) {
        if bytes == 0 {
            return;
        }
        let cap = {
            let mut meter = Shared::lock(&self.shared.meter);
            let now = Instant::now();
            meter.bytes += bytes;
            meter.fed_at = Some(now);
            match meter.at {
                None => {
                    meter.at = Some(now);
                    return;
                }
                Some(at) => {
                    let elapsed = now.duration_since(at).as_secs_f64();
                    if elapsed < 0.25 {
                        return;
                    }
                    let instant = meter.bytes as f64 / elapsed;
                    meter.rate = if meter.rate <= 0.0 {
                        instant
                    } else {
                        // A quarter of a second is short enough that one stalled
                        // chunk does not collapse the cap for the next track.
                        0.6 * meter.rate + 0.4 * instant
                    };
                    meter.bytes = 0;
                    meter.at = Some(now);
                }
            }
            meter.rate
        };
        let api = Shared::lock(&self.shared.api).clone();
        if let Some(api) = api
            && cap > 0.0
        {
            api.set_bulk_cap_from_stream(nz_api::bulk_cap_from_stream(Some(cap as u64)));
        }
    }
}

/// Release the Bulk cap once no stream has delivered a byte for 4 s.
fn release_stale_cap(shared: &Shared) {
    let stale = {
        let mut meter = Shared::lock(&shared.meter);
        match meter.fed_at {
            Some(at) if at.elapsed() < Duration::from_secs(4) => false,
            Some(_) => {
                meter.clear();
                true
            }
            None => false,
        }
    };
    if stale && let Some(api) = Shared::lock(&shared.api).clone() {
        api.set_bulk_cap_from_stream(None);
    }
}

/// Stop the process's worker, and wait for it.
///
/// This is what the app's `NetRuntime` global calls from its `Drop`: the worker
/// borrows that very runtime, so it must be joined BEFORE its `Arc<Runtime>`
/// field goes (tokio asserts on a timer polled on a shutdown runtime, and the
/// field drops after the `Drop` body — the order Rust guarantees). Idempotent,
/// and a no-op when no session ever started the worker.
pub fn shutdown_process() {
    if let Some(service) = SERVICE.get() {
        service.shutdown();
    }
}

/// The process's worker for a session is started and configured by the caller
/// that owns the session (the desktop's `attach`, the phone's core): this crate
/// keeps the singleton, the caller knows where an `Api` and a `Cache` come
/// from.
///
/// The engine's offline seam: the file of a downloaded track, whatever quality
/// it was fetched at.
///
/// `nz-audio` asks its `StreamOpener::local` before it reaches the network, so a
/// downloaded track plays with the link cut. A no-op when the service was never
/// started (a test of another area, the login screen).
pub fn local_path(track: &str) -> Option<PathBuf> {
    Offline::get()?.local(track).map(|(path, _)| path)
}

/// Tell the worker how fast the playing stream is reading, so `Lane::Bulk` can
/// be paced at 25 % of it (rule 1). Called by the player's own stream reader;
/// a no-op when the service was never started.
pub fn note_stream_bytes(bytes: u64) {
    if let Some(service) = Offline::get() {
        service.note_stream(bytes);
    }
}

/// `nice 10` for the worker thread. `SCHED_IDLE` and the idle I/O class need
/// bindings this workspace does not carry yet; the lane's own yield is the load
/// that matters, and a higher nice keeps a 200-track download off the decoder's
/// cores when the machine is busy.
fn lower_priority() {
    // SAFETY: `nice` takes an int and returns the new priority; no memory is
    // involved and a refusal (EPERM on a restricted system) is a no-op.
    unsafe {
        nice(10);
    }
}

unsafe extern "C" {
    fn nice(increment: i32) -> i32;
}

// --- the worker -------------------------------------------------------------

/// How often an idle worker re-reads its queue. Two seconds is a cost nobody
/// sees and a download that starts promptly after a click (which also nudges).
const IDLE_POLL: Duration = Duration::from_secs(2);
/// A pause is checked often enough that a resume feels immediate.
const PAUSE_POLL: Duration = Duration::from_millis(400);

fn run(shared: Arc<Shared>) {
    loop {
        if shared.stop.stopped() {
            return;
        }
        drain(&shared);

        let (Some(store), Some(api), Some(downloader)) = (
            shared.cache.clone(),
            Shared::lock(&shared.api).clone(),
            Shared::lock(&shared.downloader).clone(),
        ) else {
            set_state(&shared, WorkerState::Unavailable);
            shared.wait(IDLE_POLL);
            continue;
        };

        if shared.paused.load(Ordering::Acquire) {
            let pausing = shared.snapshot().state == WorkerState::Running;
            set_state(
                &shared,
                if pausing {
                    WorkerState::Pausing
                } else {
                    WorkerState::Paused
                },
            );
            shared.wait(PAUSE_POLL);
            continue;
        }

        if !api.is_online() {
            set_state(&shared, WorkerState::Offline);
            shared.wait(IDLE_POLL);
            continue;
        }

        release_stale_cap(&shared);

        let Some(owner) = api.owner() else {
            set_state(&shared, WorkerState::Unavailable);
            shared.wait(IDLE_POLL);
            continue;
        };

        match next_job(&store, &owner) {
            Ok(Some(job)) => {
                step(&shared, &store, &owner, &job, &downloader);
            }
            Ok(None) => {
                finish(&shared, &store, &owner);
                shared.wait(IDLE_POLL);
            }
            Err(cause) => {
                let cause = format!("file de téléchargement : {cause}");
                warn!(%cause);
                shared.publish(|snapshot| snapshot.state = WorkerState::Blocked(cause));
                shared.wait(IDLE_POLL);
            }
        }
    }
}

fn next_job(store: &Cache, owner: &str) -> nz_cache::Result<Option<nz_cache::DownloadJob>> {
    Ok(store.downloads().pending(owner, 1)?.into_iter().next())
}

/// Set the worker's state, publishing only when it actually moved: an idle app
/// polling its queue every two seconds must not repaint every two seconds.
fn set_state(shared: &Shared, state: WorkerState) {
    if shared.snapshot().state == state {
        return;
    }
    shared.publish(|snapshot| snapshot.state = state);
}

/// The queue is empty: sweep the two evictable caches, then say what the run did.
fn finish(shared: &Shared, store: &Cache, owner: &str) {
    sweep(shared, store);
    let failures = store
        .downloads()
        .failures(owner)
        .map(|rows| rows.len())
        .unwrap_or(0);
    let progress = store.downloads().progress(owner).unwrap_or_default();
    let mut note = if progress.done == 0 {
        None
    } else {
        Some(format!(
            "{} titre{} disponible{} hors ligne ({})",
            progress.done,
            if progress.done > 1 { "s" } else { "" },
            if progress.done > 1 { "s" } else { "" },
            bytes_label(progress.bytes.max(0) as u64)
        ))
    };
    if failures > 0 {
        // Never just a count: every cause is in `failures`, one row each.
        note = Some(match note {
            Some(note) => format!(
                "{note} · {failures} échec{} (voir la liste)",
                if failures > 1 { "s" } else { "" }
            ),
            None => format!(
                "{failures} titre{} n'a pas pu être téléchargé (causes ci-dessous)",
                if failures > 1 { "s" } else { "" }
            ),
        });
    }
    let changed = {
        let snapshot = shared.snapshot();
        let idle = !matches!(
            snapshot.state,
            WorkerState::Paused | WorkerState::Pausing | WorkerState::Blocked(_)
        );
        (idle && snapshot.state != WorkerState::Idle)
            || snapshot.running.is_some()
            || snapshot.note != note
    };
    if changed {
        shared.publish(|snapshot| {
            if !matches!(
                snapshot.state,
                WorkerState::Paused | WorkerState::Pausing | WorkerState::Blocked(_)
            ) {
                snapshot.state = WorkerState::Idle;
            }
            snapshot.running = None;
            if note.is_some() {
                snapshot.note = note;
            }
        });
    }
    refresh(shared, store, owner);
}

/// Re-read the whole workload and the caches' sizes into the snapshot.
fn refresh(shared: &Shared, store: &Cache, owner: &str) {
    let progress = store.downloads().progress(owner).unwrap_or_default();
    let sets = store.downloads().sets(owner).unwrap_or_default();
    // The whole map, for the failure rows' titles; the snapshot itself keeps
    // only what ITS rows can print (a playlist's row carries its own title).
    let all_labels = Shared::lock(&shared.labels).clone();
    let labels: HashMap<String, Labels> = sets
        .iter()
        .filter(|set| set.kind == "track")
        .filter_map(|set| {
            all_labels
                .get(&set.ident)
                .map(|labels| (set.ident.clone(), labels.clone()))
        })
        .collect();
    let failures: Vec<Failure> = store
        .downloads()
        .failures(owner)
        .unwrap_or_default()
        .into_iter()
        .map(|(track, cause)| Failure {
            title: all_labels
                .get(track.as_str())
                .map(|labels| labels.title.clone())
                .filter(|title| !title.is_empty())
                .unwrap_or_else(|| track.as_str().to_owned()),
            track: track.as_str().to_owned(),
            cause,
        })
        .collect();
    let downloaded: Arc<HashSet<String>> = Arc::new(
        store
            .downloads()
            .published(owner)
            .unwrap_or_default()
            .into_iter()
            .collect(),
    );
    let play_cache_bytes = store.play_cache().total_bytes().unwrap_or(0);
    let covers_bytes = store.covers().total_bytes().unwrap_or(0);
    let same = {
        let snapshot = shared.snapshot();
        snapshot.progress == progress
            && snapshot.sets == sets
            && snapshot.failures == failures
            && snapshot.labels == labels
            && *snapshot.downloaded == *downloaded
            && snapshot.play_cache_bytes == play_cache_bytes
            && snapshot.covers_bytes == covers_bytes
    };
    if same {
        return;
    }
    shared.publish(|snapshot| {
        snapshot.progress = progress;
        snapshot.sets = sets;
        snapshot.failures = failures;
        snapshot.labels = labels;
        snapshot.downloaded = downloaded;
        snapshot.play_cache_bytes = play_cache_bytes;
        snapshot.covers_bytes = covers_bytes;
    });
}

/// Evict the two caches back under their caps. A download is never touched: it
/// lives in its own directory and its own table (D3's rule).
fn sweep(shared: &Shared, store: &Cache) {
    let cap = shared.settings().play_cache_cap;
    if let Err(error) = store.play_cache().evict_to(cap) {
        debug!(%error, "éviction du cache de lecture");
    }
    if let Err(error) = store.covers().evict_to(COVERS_CAP_BYTES) {
        debug!(%error, "éviction des pochettes");
    }
}

/// Run the UI's requests. Every SQLite call happens here, on `nz-bulk` — never
/// on the GPUI thread (`nz-cache`'s access rule).
fn drain(shared: &Shared) {
    loop {
        // A request that arrives before the session is known is KEPT, not
        // dropped: the person asked for it, and `/api/me` lands a moment later.
        let (Some(store), Some(api)) = (shared.cache.clone(), Shared::lock(&shared.api).clone())
        else {
            return;
        };
        let Some(owner) = api.owner() else {
            if !Shared::lock(&shared.jobs).is_empty() {
                shared.publish(|snapshot| {
                    snapshot.state = WorkerState::Unavailable;
                    snapshot.note =
                        Some("aucune session : la demande partira à la connexion".to_owned());
                });
            }
            return;
        };
        let Some(job) = Shared::lock(&shared.jobs).pop_front() else {
            return;
        };
        apply(shared, &store, &owner, job);
        refresh(shared, &store, &owner);
    }
}

fn apply(shared: &Shared, store: &Cache, owner: &str, job: Job) {
    match job {
        Job::Choose {
            kind,
            ident,
            title,
            tracks,
        } => {
            let quality = shared.settings().quality;
            let downloads: Vec<DownloadTrack> = tracks
                .iter()
                .map(|choice| DownloadTrack {
                    track: TrackId::new(choice.track.clone()),
                    quality: choice.quality.unwrap_or(quality),
                })
                .collect();
            let labels: Vec<(String, Labels)> = tracks
                .iter()
                .map(|choice| {
                    (
                        choice.track.clone(),
                        Labels {
                            title: choice.title.clone(),
                            artist: choice.artist.clone(),
                        },
                    )
                })
                .collect();
            match store
                .downloads()
                .add_set(owner, &kind, &ident, &title, &downloads)
            {
                Ok(linked) => {
                    {
                        let mut kept = Shared::lock(&shared.labels);
                        for (track, labels) in labels {
                            kept.insert(track, labels);
                        }
                    }
                    save_labels(shared, store);
                    info!(kind, ident, linked, "téléchargement demandé");
                    shared.publish(|snapshot| {
                        snapshot.note = Some(format!(
                            "« {title} » : {} titre{} à télécharger",
                            downloads.len(),
                            if downloads.len() > 1 { "s" } else { "" }
                        ));
                    });
                }
                Err(error) => {
                    let cause = format!("« {title} » n'a pas pu être mis en file : {error}");
                    warn!(%cause);
                    shared.publish(|snapshot| snapshot.note = Some(cause));
                }
            }
        }
        Job::DropSet { kind, ident } => {
            let removed = store.downloads().remove_set(owner, &kind, &ident);
            match removed {
                Ok(count) => {
                    prune_labels(shared, store, owner);
                    shared.publish(|snapshot| {
                        snapshot.note = Some(format!(
                            "{count} titre{} retiré{} des téléchargements",
                            if count > 1 { "s" } else { "" },
                            if count > 1 { "s" } else { "" }
                        ));
                    });
                }
                Err(error) => {
                    let cause = format!("retrait impossible : {error}");
                    warn!(%cause);
                    shared.publish(|snapshot| snapshot.note = Some(cause));
                }
            }
        }
        Job::DropTrack { track } => {
            let id = TrackId::new(track.clone());
            match store.downloads().remove(owner, &id) {
                Ok(removed) => {
                    Shared::lock(&shared.labels).remove(&track);
                    save_labels(shared, store);
                    shared.publish(|snapshot| {
                        snapshot.note = Some(if removed {
                            "Titre retiré des téléchargements".to_owned()
                        } else {
                            "Ce titre n'était pas téléchargé".to_owned()
                        });
                    });
                }
                Err(error) => {
                    let cause = format!("retrait impossible : {error}");
                    warn!(%cause);
                    shared.publish(|snapshot| snapshot.note = Some(cause));
                }
            }
        }
        Job::RetryFailures => {
            let failures = store.downloads().failures(owner).unwrap_or_default();
            let mut retried = 0usize;
            for (track, _cause) in &failures {
                if store
                    .downloads()
                    .mark(owner, track, DownloadState::Pending, None)
                    .unwrap_or(false)
                {
                    retried += 1;
                }
            }
            shared.publish(|snapshot| {
                snapshot.note = Some(if retried == 0 {
                    "Aucun échec à réessayer".to_owned()
                } else {
                    format!(
                        "{retried} téléchargement{} remis en file",
                        if retried > 1 { "s" } else { "" }
                    )
                });
            });
        }
        Job::SaveSettings => {
            let settings = shared.settings();
            settings.write(store);
            shared.publish(|snapshot| snapshot.settings = settings);
        }
        Job::Sweep => sweep(shared, store),
    }
}

fn save_labels(shared: &Shared, store: &Cache) {
    let labels = Shared::lock(&shared.labels).clone();
    if let Err(error) = store.kv().set(LABELS_KEY, &labels) {
        warn!(%error, "libellés des téléchargements non enregistrés");
    }
}

/// Drop the labels of tracks nothing wants any more (a removed set, a failure
/// that was retried and removed): the map is the only thing that would grow
/// without a bound.
fn prune_labels(shared: &Shared, store: &Cache, owner: &str) {
    let wanted: HashMap<String, ()> = store
        .downloads()
        .sets(owner)
        .unwrap_or_default()
        .into_iter()
        // A set's own row carries its title; the per-track labels are for the
        // rows of `kind == "track"` and for the failures.
        .map(|set| (set.ident, ()))
        .collect();
    let failed: HashMap<String, ()> = store
        .downloads()
        .failures(owner)
        .unwrap_or_default()
        .into_iter()
        .map(|(track, _)| (track.as_str().to_owned(), ()))
        .collect();
    {
        let mut labels = Shared::lock(&shared.labels);
        labels.retain(|track, _| wanted.contains_key(track) || failed.contains_key(track));
    }
    save_labels(shared, store);
}

/// One track of the queue, through `Lane::Bulk`.
fn step(
    shared: &Shared,
    store: &Cache,
    owner: &str,
    job: &nz_cache::DownloadJob,
    downloader: &Arc<dyn Downloader>,
) {
    let title = Shared::lock(&shared.labels)
        .get(job.track.as_str())
        .map(|labels| labels.title.clone())
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| job.track.as_str().to_owned());
    let quality = job.quality;
    shared.publish(|snapshot| {
        snapshot.state = WorkerState::Running;
        snapshot.running = Some(Running {
            track: job.track.as_str().to_owned(),
            title: title.clone(),
            quality,
            bytes: 0,
            total: None,
        });
    });

    let started = Instant::now();
    match fetch(shared, store, owner, job, downloader) {
        Ok(fetched) => {
            info!(
                track = job.track.as_str(),
                bytes = fetched.bytes,
                quality = fetched.quality.wire().unwrap_or("FLAC"),
                ms = started.elapsed().as_millis() as u64,
                "téléchargement terminé"
            );
            shared.publish(|snapshot| {
                snapshot.running = None;
                snapshot.state = WorkerState::Running;
            });
            refresh(shared, store, owner);
        }
        Err(Outcome::Later(delay, cause)) => {
            debug!(track = job.track.as_str(), %cause, "téléchargement reporté");
            // Not a verdict about the track: the row goes back to the queue and
            // the worker comes back to it (a 503 + Retry-After, a cut link).
            let _ = store
                .downloads()
                .mark(owner, &job.track, DownloadState::Pending, None);
            shared.publish(|snapshot| {
                snapshot.running = None;
                snapshot.note = Some(format!("« {title} » : {cause}"));
            });
            shared.wait(delay.min(Duration::from_secs(30)));
        }
        Err(Outcome::Offline) => {
            let _ = store
                .downloads()
                .mark(owner, &job.track, DownloadState::Pending, None);
            shared.publish(|snapshot| {
                snapshot.running = None;
                snapshot.state = WorkerState::Offline;
                snapshot.note = Some(format!("« {title} » attend le réseau"));
            });
            shared.wait(IDLE_POLL);
        }
        Err(Outcome::Interrupted) => {
            // Nothing is written: the row stays `active`, the `.part` is a stray
            // file the next `reserve` overwrites, and the next start resumes.
            debug!(track = job.track.as_str(), "téléchargement interrompu");
        }
        Err(Outcome::Failed(cause)) => {
            warn!(track = job.track.as_str(), %cause, "téléchargement abandonné");
            let _ = store
                .downloads()
                .mark(owner, &job.track, DownloadState::Failed, Some(&cause));
            shared.publish(|snapshot| {
                snapshot.running = None;
                snapshot.note = Some(format!("« {title} » : {cause}"));
            });
            refresh(shared, store, owner);
        }
    }
}

/// Reserve the `.part`, let the fetcher fill it, then publish it — the store's
/// own rule: a file is renamed into place only once complete.
///
/// The quality published is the one the FETCHER reports, not the one the row
/// asked for: a peer on the LAN serves the copy it holds, so a row reserved at
/// Opus 320 can hold FLAC. The store then names, records and answers that
/// quality, and every screen prints the rung the file really is
/// (`docs/agents/lan-nodes.md`, *Shared-crate changes*).
///
/// An [`Outcome::Interrupted`] KEEPS the `.part`: the row stays `active` for the
/// next start, which fetches the track again rather than trusting a truncated
/// file (`a_restart_mid_download_resumes_the_set`). Every other failure throws
/// the partial away: a cause the person can read is a verdict, and a half file
/// nothing will finish is litter.
fn fetch(
    shared: &Shared,
    store: &Cache,
    owner: &str,
    job: &nz_cache::DownloadJob,
    downloader: &Arc<dyn Downloader>,
) -> Result<Fetched, Outcome> {
    let _ = store
        .downloads()
        .mark(owner, &job.track, DownloadState::Active, None)
        .map_err(|error| Outcome::Failed(format!("file locale : {error}")))?;
    let partial = store
        .downloads()
        .reserve(owner, &job.track, job.quality)
        .map_err(|error| Outcome::Failed(format!("disque : {error}")))?;

    let mut progress = PublishedProgress { shared };
    let outcome = downloader.fetch(
        &job.track,
        job.quality,
        &partial,
        &shared.stop,
        &mut progress,
    );
    let fetched = match outcome {
        Ok(fetched) => fetched,
        Err(Outcome::Interrupted) => return Err(Outcome::Interrupted),
        Err(other) => {
            let _ = std::fs::remove_file(&partial);
            return Err(other);
        }
    };

    store
        .downloads()
        .complete_as(owner, &job.track, fetched.bytes, fetched.quality)
        .map_err(|error| Outcome::Failed(format!("publication : {error}")))?;
    Ok(fetched)
}

/// What a fetcher's byte count becomes: the snapshot a screen draws (rule 2 —
/// a running download says where it is).
struct PublishedProgress<'a> {
    shared: &'a Shared,
}

impl Progress for PublishedProgress<'_> {
    fn total(&mut self, total: Option<u64>) {
        self.shared.publish(|snapshot| {
            if let Some(running) = snapshot.running.as_mut() {
                running.total = total;
            }
        });
    }

    fn written(&mut self, bytes: u64) {
        self.shared.publish(|snapshot| {
            if let Some(running) = snapshot.running.as_mut() {
                running.bytes = bytes;
            }
        });
    }
}

/// The offline-only filter: what a queue builder keeps when « ne lire que les
/// titres téléchargés » is on and the link is down (`lib/offline.js`). An entry
/// is kept when its reference is a downloaded track.
pub fn keep_downloaded<T, F>(items: impl IntoIterator<Item = T>, reference: F) -> Vec<T>
where
    F: Fn(&T) -> Option<String>,
{
    keep_downloaded_with(Offline::get().as_ref(), items, reference)
}

/// The same filter against a given service (a test, or a caller that holds one).
/// `None` keeps everything: with no service nothing is known to be local, and
/// dropping the queue would be the worse failure.
pub fn keep_downloaded_with<T, F>(
    service: Option<&Offline>,
    items: impl IntoIterator<Item = T>,
    reference: F,
) -> Vec<T>
where
    F: Fn(&T) -> Option<String>,
{
    let Some(service) = service else {
        return items.into_iter().collect();
    };
    items
        .into_iter()
        .filter(|item| {
            reference(item)
                .map(|track| service.is_downloaded(&track))
                .unwrap_or(false)
        })
        .collect()
}

/// The settings, from the service when it runs (a screen that only wants the
/// caps before a session exists).
pub fn settings() -> Settings {
    Offline::get()
        .map(|service| service.settings())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;

    use nz_api::{Quality, TrackId};

    use super::*;
    use crate::download::{Downloader, Fetched};
    use crate::settings::quality_label;
    use crate::state::Choice;
    use crate::testing::{Link, Scratch, TestServer, api_for, wait_for};

    /// A fetcher that writes `bytes` itself and says they are FLAC: the peer a
    /// LAN node is (`docs/agents/lan-nodes.md`, *Shared-crate changes*). The
    /// server is never asked — only the trait's report changes.
    struct DeliversFlac {
        bytes: usize,
    }

    impl Downloader for DeliversFlac {
        fn fetch(
            &self,
            _track: &TrackId,
            _quality: Quality,
            part: &Path,
            _stop: &Stop,
            progress: &mut dyn Progress,
        ) -> Result<Fetched, Outcome> {
            std::fs::write(part, vec![0x11u8; self.bytes])
                .map_err(|error| Outcome::Failed(format!("écriture : {error}")))?;
            progress.total(Some(self.bytes as u64));
            progress.written(self.bytes as u64);
            Ok(Fetched {
                bytes: self.bytes as u64,
                quality: Quality::Flac,
            })
        }
    }

    /// **LN5's acceptance for the downloads worker**: a row reserved at Opus 320
    /// whose fetcher delivered FLAC is published, NAMED and reported as FLAC.
    ///
    /// `Downloads::local` answers `Flac` — which is what the engine's offline
    /// seam and the downloads screen's label read — the file carries the FLAC
    /// digest, no `.part` survives, and the run reports no failure.
    ///
    /// Measured: 1 published download of 4 096 bytes in **1** file named
    /// `<digest(FLAC)>.FLAC`, **0** `.part`, `quality_label` = « FLAC ».
    #[test]
    fn a_download_delivered_in_flac_is_stored_and_labelled_flac() {
        let scratch = Scratch::new("delivered-flac");
        let store = scratch.store();
        // The client needs a base URL; this fetcher never reaches it.
        let server = TestServer::start(Link::new(None), HashMap::new());
        let api = api_for(&server, &store, &scratch);
        let owner = api.owner().expect("a session owner").to_owned();
        let service = Offline::start(store.clone());
        service.configure_with(api, Arc::new(DeliversFlac { bytes: 4096 }));
        service.choose_track(Choice::new("3135556", "Titre").with_quality(Quality::Opus320));

        assert!(
            wait_for(Duration::from_secs(20), || service.snapshot().progress.done
                == 1),
            "the track was published: {:?}",
            service.snapshot().progress
        );
        let snapshot = service.snapshot();
        assert_eq!(snapshot.progress.failed, 0, "a delivery is not a failure");
        assert_eq!(snapshot.progress.bytes, 4096);

        let track = TrackId::new("3135556");
        let (path, quality) = store
            .downloads()
            .local(&owner, &track)
            .expect("the store")
            .expect("the download is published");
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .expect("a file name");
        println!(
            "publié : {name}, qualité {quality:?}, libellé « {} »",
            quality_label(quality)
        );
        assert_eq!(
            quality,
            Quality::Flac,
            "the FLAC the fetcher delivered, not the OPUS_320 the row asked for"
        );
        assert_eq!(quality_label(quality), "FLAC", "what the screen prints");
        assert!(name.ends_with(".FLAC"), "the file name holds the quality");
        let leftovers: Vec<String> = std::fs::read_dir(store.offline_dir())
            .expect("the offline directory")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.ends_with(".part"))
            .collect();
        assert_eq!(leftovers, Vec::<String>::new(), "no `.part` survives");

        service.shutdown();
        drop(service);
    }
}
