//! The background SHA-256 of the local files, so a peer can be told exactly which
//! bytes it will receive (docs/agents/lan-nodes.md, *Integrity, resume*).
//!
//! **LN4 filled it.** One thread, named `nz-lan-hash`, reniced **+10** (rule 1:
//! the person's own playback and the serving paths come first), which hashes what
//! the index lists and stops hashing while a `User` request is in flight. It is
//! JOINED by [`Hashing::stop`], and by `Lan::shutdown` after it: the thread owns
//! the tiny tokio runtime it uses to wait on `nz-api`'s gate, so nothing it
//! borrows outlives it.
//!
//! A hash is recomputed only when `(len, mtime)` changed — the store keeps both,
//! and `needs_hash` is the whole rule — so a restart does not re-read a
//! 2 000-track library to learn what it already knew.
//!
//! # Why the gate is taken per tile
//!
//! `nz_api::lanes::Gates` decides *admission*, not pre-emption: a `Bulk` slot is
//! granted once no `User` request has been in flight for a second, and it is held
//! until the guard drops. A file hashed under one guard would therefore keep a
//! core for its whole length (a 30 MB FLAC is ~60 ms — enough to matter while
//! somebody waits). So the worker takes the slot, hashes one
//! [`TILE_BYTES`] tile, gives it back, and takes it again: with no user traffic
//! that costs a mutex and a `Notify` per MiB, and as soon as the person taps a
//! track the next tile waits for them to be served. Measured on this machine: a
//! 512 MB file hashes at 1.4 GB/s, i.e. 1.4 ms per tile.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, UNIX_EPOCH};

use aws_lc_rs::digest::{Context, SHA256};
use tokio::sync::{Notify, watch};
use tracing::debug;

use crate::index::Index;
use crate::store::{self, Store};

/// How much of a file one tile reads between two yields to the person's work.
pub const TILE_BYTES: usize = 1024 * 1024;

/// The niceness the hashing thread runs at (`nice +10`), which the acceptance
/// reads back from `/proc/self/task/<tid>/stat`.
pub const HASH_NICE: i32 = 10;

/// How long a gated tile waits before looking at the stop flag again, so a
/// shutdown never hangs behind the person's own traffic.
const STOP_POLL: Duration = Duration::from_millis(250);

/// What one file's attempt produced.
enum Outcome {
    /// The tag is stored; the next sweep will not see it again.
    Hashed,
    /// The file could not be read or its tag could not be written, at this exact
    /// `(len, mtime)`. It is not retried until either moves: a file this process
    /// may not read would otherwise be re-attempted in a tight loop, which is the
    /// one way a background worker can eat a core (rule 1).
    Failed { len: u64, mtime: i64 },
    /// The worker was asked to stop mid-file.
    Stopped,
}

/// What the hashing worker is doing, so a screen can say it and a test can wait
/// on it instead of sleeping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashState {
    /// Nothing to hash: waiting for the inventory to change.
    Idle,
    /// A file is listed and the worker is waiting for the person's own work
    /// (`Lane::Bulk` yields to it).
    Waiting,
    /// A file is being hashed right now.
    Hashing,
}

/// The hashing worker.
///
/// A node has at most one: it is started by `Lan::start` and stopped before the
/// runtime it borrows stops.
#[derive(Debug)]
pub struct Hashing {
    stop: Arc<Stop>,
    state: watch::Receiver<HashState>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Debug)]
struct Stop {
    stopped: AtomicBool,
    /// Set by the worker itself as it leaves, whatever brought it there: the one
    /// fact a test (or a shutdown) needs from "the thread ended", with no clock
    /// and no `/proc` race.
    finished: AtomicBool,
    wake: Notify,
}

impl Stop {
    fn new() -> Arc<Stop> {
        Arc::new(Stop {
            stopped: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            wake: Notify::new(),
        })
    }

    fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }
}

/// Marks the worker as finished on EVERY way out of `run` (a return, an early
/// return, a panic), so `stop`'s join and the flag always agree.
struct Running(Arc<Stop>);

impl Drop for Running {
    fn drop(&mut self) {
        self.0.finished.store(true, Ordering::Release);
    }
}

impl Hashing {
    /// Starts hashing `index`'s files, persisting the tags in `store`.
    ///
    /// Without the app's lanes: the worker still runs at `nice +10` (the OS
    /// scheduler's own yield), but it does not wait for a playing stream. The
    /// service uses [`Hashing::start_with`], which does.
    pub fn start(index: Index, store: Store) -> Hashing {
        Hashing::start_inner(index, store, None)
    }

    /// Starts hashing the inventory, yielding to the person: the SAME
    /// `nz-api` gates the app's own requests and the responder's `Bulk` serves
    /// take, so one Bulk slot covers server, LAN and hashing.
    ///
    /// Additive on purpose: LN0 declared `start(index, store)`, which has no room
    /// for the gate, and a gate it never sees is a worker that does not yield.
    pub fn start_with(index: Index, store: Store, gates: Arc<nz_api::lanes::Gates>) -> Hashing {
        Hashing::start_inner(index, store, Some(gates))
    }

    fn start_inner(
        index: Index,
        store: Store,
        gates: Option<Arc<nz_api::lanes::Gates>>,
    ) -> Hashing {
        let stop = Stop::new();
        let (states, state) = watch::channel(HashState::Idle);
        let thread = {
            let stop = Arc::clone(&stop);
            thread::Builder::new()
                .name("nz-lan-hash".to_owned())
                .spawn(move || run(index, store, gates, stop, states))
                .ok()
        };
        if thread.is_none() {
            debug!("nz-lan-hash could not start: no file is hashed");
        }
        Hashing {
            stop,
            state,
            thread,
        }
    }

    /// What the worker is doing (a screen line, and the event a test waits on).
    pub fn state(&self) -> watch::Receiver<HashState> {
        self.state.clone()
    }

    /// Stops the worker and JOINS its thread.
    pub fn stop(mut self) {
        self.join();
    }

    fn join(&mut self) {
        self.stop.stopped.store(true, Ordering::Relaxed);
        // `notify_one`, not `notify_waiters`: a permit is REMEMBERED for a waiter
        // that has not registered yet, which closes the window between the worker
        // reading `stopped` and awaiting the wake (a lost wake there is a `stop`
        // that never returns).
        self.stop.wake.notify_one();
        if let Some(thread) = self.thread.take() {
            // The thread owns its runtime: joining it before the process (and
            // before `Lan`'s runtime) is what keeps `nz-lan`'s shutdown order.
            let _ = thread.join();
        }
    }
}

impl Drop for Hashing {
    fn drop(&mut self) {
        self.join();
    }
}

/// The worker's body: hash what is missing, wait for the inventory to move.
fn run(
    index: Index,
    store: Store,
    gates: Option<Arc<nz_api::lanes::Gates>>,
    stop: Arc<Stop>,
    states: watch::Sender<HashState>,
) {
    lower_priority();
    // The worker's OWN runtime, owned by this thread: `Gates::acquire` awaits a
    // `Notify` and a timer, and a runtime that is born and dropped inside the
    // thread cannot outlive it.
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            debug!(%error, "nz-lan-hash: no runtime, nothing is hashed");
            return;
        }
    };
    let mut events = index.events();
    let _running = Running(Arc::clone(&stop));
    // A file this thread could not hash at a given `(len, mtime)`: skipped until
    // one of the two moves, so a permission error costs one attempt, not a spin.
    let mut failed: HashMap<PathBuf, (u64, i64)> = HashMap::new();

    while !stop.stopped() {
        // "I am about to read the inventory": a refresh that lands between this
        // line and the read below still wakes the wait that follows.
        let pending = {
            events.mark_unchanged();
            let tags = store.tags();
            pending_paths(&index, &tags, &failed)
        };
        if pending.is_empty() {
            let _ = states.send(HashState::Idle);
            let stopped = runtime.block_on(async {
                tokio::select! {
                    _ = events.changed() => false,
                    _ = stop.wake.notified() => true,
                }
            });
            if stopped || stop.stopped() {
                return;
            }
            continue;
        }

        for path in pending {
            if stop.stopped() {
                return;
            }
            match hash_one(
                &index,
                &store,
                gates.as_ref(),
                &runtime,
                &stop,
                &states,
                &path,
            ) {
                Outcome::Hashed => {}
                Outcome::Failed { len, mtime } => {
                    failed.insert(path, (len, mtime));
                }
                Outcome::Stopped => return,
            }
        }
    }
}

/// The files whose stored `(len, mtime)` no longer match the one on disk — one
/// `stat` per file, no read.
fn pending_paths(
    index: &Index,
    tags: &HashMap<String, store::StoredTag>,
    failed: &HashMap<PathBuf, (u64, i64)>,
) -> Vec<PathBuf> {
    index
        .local_paths()
        .into_iter()
        .filter(|path| {
            if let Some((len, mtime)) = failed.get(path)
                && let Ok(meta) = fs::metadata(path)
                && meta.len() == *len
                && file_mtime(&meta) == *mtime
            {
                // Already refused at exactly these bytes: a re-download or a
                // re-encode changes one of the two and lets it back in.
                return false;
            }
            needs_hash(path, tags)
        })
        .collect()
}

/// Whether a file has to be hashed: it has no tag, or its `(len, mtime)` moved.
///
/// The rule the acceptance asks for, in one function, so it is tested without a
/// thread: a re-encode, a re-download and a truncation all change it; a restart,
/// a re-read or a touch that keeps both does not.
pub fn needs_hash(path: &Path, tags: &HashMap<String, store::StoredTag>) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        // A file that vanished between the inventory and here is not hashed: the
        // next refresh takes it out of the index.
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    match tags.get(&store::key_of(path)) {
        Some(tag) => tag.len != meta.len() || tag.mtime != file_mtime(&meta),
        None => true,
    }
}

fn file_mtime(meta: &fs::Metadata) -> i64 {
    // Nanoseconds, not seconds: a file rewritten within the same second with the
    // same length must still be rehashed, and the pair is the whole check.
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|since| since.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

/// Hashes one file, one gated tile at a time, and stores its tag.
fn hash_one(
    index: &Index,
    store: &Store,
    gates: Option<&Arc<nz_api::lanes::Gates>>,
    runtime: &tokio::runtime::Runtime,
    stop: &Stop,
    states: &watch::Sender<HashState>,
    path: &Path,
) -> Outcome {
    // "A file is being worked on" starts with the open, not with the first tile: a
    // file this process may not read is still an attempt, and the state says so.
    let _ = states.send(HashState::Hashing);
    let Ok(meta) = fs::metadata(path) else {
        // Gone between the inventory and here: nothing to hash, and nothing to
        // remember (the next refresh takes it out of the index).
        return Outcome::Hashed;
    };
    if !meta.is_file() {
        return Outcome::Hashed;
    }
    let len = meta.len();
    let mtime = file_mtime(&meta);
    let Ok(mut file) = fs::File::open(path) else {
        debug!("nz-lan-hash: a local file could not be opened");
        return Outcome::Failed { len, mtime };
    };
    let mut context = Context::new(&SHA256);
    let mut buffer = vec![0u8; TILE_BYTES];

    loop {
        if stop.stopped() {
            return Outcome::Stopped;
        }
        // Yield BEFORE reading the tile: while the person is being served, this
        // is where the worker waits.
        let _slot = match gates {
            Some(gates) => {
                let _ = states.send(HashState::Waiting);
                let Some(slot) = wait_for_gate(gates, runtime, stop) else {
                    return Outcome::Stopped;
                };
                let _ = states.send(HashState::Hashing);
                Some(slot)
            }
            None => {
                let _ = states.send(HashState::Hashing);
                None
            }
        };
        let read = match file.read(&mut buffer) {
            Ok(read) => read,
            Err(error) => {
                debug!(%error, "nz-lan-hash: a local file could not be read");
                return Outcome::Failed { len, mtime };
            }
        };
        if read == 0 {
            break;
        }
        context.update(&buffer[..read]);
    }

    let digest = hex(context.finish().as_ref());
    if let Err(error) = store.put_tag(path, len, mtime, &digest) {
        debug!(%error, "nz-lan-hash: the tag was not stored");
        return Outcome::Failed { len, mtime };
    }
    index.note_hashed();
    debug!(bytes = len, "nz-lan-hash: one local file hashed");
    Outcome::Hashed
}

/// Takes one `Bulk` slot, looking at the stop flag every [`STOP_POLL`] so a
/// shutdown is never stuck behind the person's own traffic.
fn wait_for_gate<'a>(
    gates: &'a Arc<nz_api::lanes::Gates>,
    runtime: &tokio::runtime::Runtime,
    stop: &Stop,
) -> Option<nz_api::lanes::LaneGuard<'a>> {
    loop {
        if stop.stopped() {
            return None;
        }
        // The timeout future is created INSIDE `block_on`: building it outside
        // would ask for the current runtime's timer and panic (there is none on
        // this thread until `block_on` runs).
        match runtime.block_on(async {
            tokio::time::timeout(STOP_POLL, gates.acquire(nz_api::lanes::Lane::Bulk)).await
        }) {
            Ok(guard) => return Some(guard),
            // Still queued behind a User request: check the flag and wait again.
            Err(_) => continue,
        }
    }
}

/// Lowercase hex of a SHA-256, which is what the wire carries.
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

/// `nice +10` for the hashing thread.
///
/// `setpriority(PRIO_PROCESS, 0, …)` on Linux and Android names the CALLING
/// thread, which is exactly what this is: raising niceness needs no privilege, and
/// a refusal (a restricted system) is a no-op rather than a failed start.
fn lower_priority() {
    // SAFETY: `setpriority` takes three integers and touches no memory of ours.
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, HASH_NICE);
    }
}

#[cfg(test)]
mod tests {
    use crate::index::Index;

    use super::*;

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(tag: &str) -> TempDir {
            let path =
                std::env::temp_dir().join(format!("nz-lan-hash-{}-{tag}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("temp dir");
            TempDir { path }
        }

        fn cache(&self) -> nz_cache::Cache {
            nz_cache::Cache::open_with_cache_dir(self.path.join("data"), self.path.join("cache"))
                .expect("cache")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    /// The rule, on its own: a tag is reused while `(len, mtime)` hold, and any
    /// change to either — a re-encode, a re-download, a truncation — asks for a
    /// new hash.
    #[test]
    fn a_file_is_hashed_again_only_when_its_length_or_mtime_changed() {
        let dir = TempDir::new("needs");
        let path = dir.path.join("a.FLAC");
        fs::write(&path, vec![1u8; 32]).expect("write");
        let meta = fs::metadata(&path).expect("meta");
        let tag = store::StoredTag {
            len: meta.len(),
            mtime: file_mtime(&meta),
            sha256: "00".repeat(32),
        };
        let mut tags = HashMap::new();
        tags.insert(store::key_of(&path), tag.clone());
        assert!(!needs_hash(&path, &tags), "nothing changed: reuse the hash");

        let mut truncated = tags.clone();
        truncated.insert(
            store::key_of(&path),
            store::StoredTag {
                len: tag.len - 1,
                ..tag.clone()
            },
        );
        assert!(
            needs_hash(&path, &truncated),
            "a shorter file is another file"
        );

        let mut touched = tags.clone();
        touched.insert(
            store::key_of(&path),
            store::StoredTag {
                mtime: tag.mtime + 1,
                ..tag
            },
        );
        assert!(needs_hash(&path, &touched), "a new mtime is another file");

        assert!(needs_hash(&path, &HashMap::new()), "no tag at all");
        assert!(
            !needs_hash(&dir.path.join("absent.FLAC"), &HashMap::new()),
            "a file that is gone is not hashed"
        );
    }

    /// The worker: a real play-cache file is hashed, its tag lands in the store
    /// with the file's own length and digest, the thread is `nice +10`, and
    /// `stop` joins it.
    #[test]
    fn the_worker_hashes_the_inventory_at_nice_ten_and_joins_on_stop() {
        let dir = TempDir::new("worker");
        let cache = dir.cache();
        let track = nz_model::TrackId::new("3135556");
        let partial = cache
            .play_cache()
            .reserve(&track, nz_model::Quality::Opus320)
            .expect("reserve");
        let bytes = vec![7u8; 8 * 1024];
        fs::write(&partial, &bytes).expect("bytes");
        let published = cache
            .play_cache()
            .complete(&track, nz_model::Quality::Opus320, bytes.len() as u64)
            .expect("complete");

        let store = Store::open(&dir.path.join("lan")).expect("store");
        let index = Index::new(cache, "1".to_owned(), store.clone());
        index.refresh();
        assert_eq!(index.tracks(), 1, "the play-cache file is indexed");

        let base_nice = thread_nice(thread_id()).expect("our own niceness");
        let worker = Hashing::start(index.clone(), store.clone());
        let mut hashed = index.hashed();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        let hashed_now = runtime
            .block_on(hashed.wait_for(|count| *count > 0))
            .map(|count| *count)
            .expect("the worker published its first hash");
        assert_eq!(hashed_now, 1, "the worker hashed exactly one file");

        let tag = store.tag(&published).expect("the tag was stored");
        assert_eq!(tag.len, bytes.len() as u64);
        assert_eq!(
            tag.sha256,
            hex(aws_lc_rs::digest::digest(&SHA256, &bytes).as_ref()),
            "the digest is the file's own bytes"
        );
        // And the length/mtime pair it stored is the pair the file has NOW, which
        // is what makes `needs_hash` answer `false` on the next boot.
        assert!(!needs_hash(&published, &store.tags()));

        // The niceness, read back from the kernel for the thread by name.
        let tid = thread_by_name("nz-lan-hash").expect("the hashing thread is running");
        assert_eq!(
            thread_nice(tid).expect("the thread's own nice value"),
            (base_nice + HASH_NICE).min(19),
            "the hashing worker must yield to the person's own work"
        );

        let stop = Arc::clone(&worker.stop);
        assert!(
            !stop.finished.load(Ordering::Acquire),
            "the worker is still waiting for work"
        );
        worker.stop();
        // The worker set its own exit flag before `stop`'s join returned: the
        // thread ended, and it cannot have ended before the join. An event, never
        // a clock.
        assert!(
            stop.finished.load(Ordering::Acquire),
            "stop joins the thread"
        );
    }

    /// A file the process may not read is attempted ONCE: the worker goes back to
    /// `Idle` instead of re-listing it for ever (the spin that eats a core).
    ///
    /// `chmod 000` is a no-op for root, so the test says so and returns rather than
    /// asserting a permission error it cannot produce.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_file_is_not_retried_in_a_loop() {
        use std::os::unix::fs::PermissionsExt;

        // SAFETY: `geteuid` reads the calling process's effective uid.
        if unsafe { libc::geteuid() } == 0 {
            eprintln!("root reads anything: the permission case cannot be tested here");
            return;
        }
        let dir = TempDir::new("unreadable");
        let cache = dir.cache();
        let track = nz_model::TrackId::new("forbidden");
        let partial = cache
            .play_cache()
            .reserve(&track, nz_model::Quality::Flac)
            .expect("reserve");
        fs::write(&partial, vec![1u8; 4096]).expect("bytes");
        let published = cache
            .play_cache()
            .complete(&track, nz_model::Quality::Flac, 4096)
            .expect("complete");
        fs::set_permissions(&published, fs::Permissions::from_mode(0o000)).expect("chmod");

        let store = Store::open(&dir.path.join("lan")).expect("store");
        let index = Index::new(cache, "1".to_owned(), store.clone());
        index.refresh();
        assert_eq!(index.tracks(), 1, "the file is in the inventory");

        let worker = Hashing::start(index.clone(), store.clone());
        let mut state = worker.state();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        // It announces the attempt, then gives up on it and goes idle: two events,
        // no clock read (the watchdogs only fire on a worker that spins).
        for wanted in [HashState::Hashing, HashState::Idle] {
            match runtime.block_on(async {
                tokio::time::timeout(
                    Duration::from_secs(10),
                    state.wait_for(|state| *state == wanted),
                )
                .await
            }) {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => panic!("the worker's state channel closed: {error}"),
                Err(_) => panic!("the worker never reached {wanted:?}: it is spinning"),
            }
        }
        worker.stop();
        assert!(
            store.tag(&published).is_none(),
            "a file that cannot be read has no tag"
        );

        // A readable file at the same path (the person fixed the permission) is
        // hashed by the next worker: the failure is remembered per (len, mtime),
        // not for ever.
        fs::set_permissions(&published, fs::Permissions::from_mode(0o600)).expect("chmod back");
        let worker = Hashing::start(index.clone(), store.clone());
        let mut hashed = index.hashed();
        runtime
            .block_on(hashed.wait_for(|count| *count >= 1))
            .expect("the worker hashed the file once it could read it");
        worker.stop();
        assert!(store.tag(&published).is_some());
    }

    /// The thread id of the caller (`gettid`), which is not the process id.
    fn thread_id() -> i64 {
        // SAFETY: `gettid` reads the calling thread's kernel id and touches no
        // memory of ours.
        unsafe { libc::gettid() as i64 }
    }

    /// Field 19 of `/proc/self/task/<tid>/stat`: the nice value of ONE thread.
    fn thread_nice(tid: i64) -> Option<i32> {
        let stat = fs::read_to_string(format!("/proc/self/task/{tid}/stat")).ok()?;
        // The command name is in parentheses and may itself contain a ')': the
        // fields start after the LAST one.
        let rest = stat.rsplit_once(')')?.1;
        let fields: Vec<&str> = rest.split_whitespace().collect();
        // state is field 3, so nice (field 19) is this index.
        fields.get(16)?.parse().ok()
    }

    /// The tid of the thread carrying `name`, 15 characters at most in `/proc`
    /// (`nz-lan-hash` is 11).
    fn thread_by_name(name: &str) -> Option<i64> {
        for entry in fs::read_dir("/proc/self/task").ok()? {
            let entry = entry.ok()?;
            let comm = fs::read_to_string(entry.path().join("comm")).ok()?;
            if comm.trim_end() == name {
                return entry.file_name().to_string_lossy().parse().ok();
            }
        }
        None
    }
}
