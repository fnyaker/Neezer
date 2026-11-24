//! Where the bytes come from, and how they become a seekable `MediaSource`.
//!
//! The engine never talks HTTP itself: the app implements [`StreamOpener`] on
//! `nz-api` + `nz-cache` (`docs/agents/desktop.md`, *Streams*), and the tests
//! implement it on files and on a local HTTP server. Two source kinds come out
//! of that:
//!
//! - a finished file — the app's play cache ([`StreamOpener::local`]) or a
//!   completed spool — is handed to symphonia as a plain [`std::fs::File`];
//! - a network body is spooled to disk by [`HttpSource`] while it arrives, and
//!   `read` blocks until the bytes are there. This is what makes a first play
//!   ("generated as it is sent", `Accept-Ranges: none`, no length) seekable at
//!   all later, and what lets the decoder run at up to a second ahead of the
//!   network instead of stalling on every socket read.
//!
//! `HttpSource` also answers symphonia's `Seek`: on a body whose length is
//! known (a finished file, `Accept-Ranges: bytes`) it restarts the request at
//! the byte the container asked for — one `Range` request — and keeps reading.
//! A generated body (`is_seekable == false`) is never seeked by symphonia; the
//! engine reopens it with `?start=<seconds>` instead, which is the server's own
//! contract for a seek (`docs/agents/playback.md`, *A seek is answered by the
//! server, or explicitly deferred*).

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::fs::{File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use symphonia::core::io::MediaSource;

use crate::events::{EngineEvent, EventBus};

/// The server's `deezer_id` field: a Deezer number as a string, or a UUID for
/// uploads, YouTube rows and podcast episodes. Never parsed as an integer
/// (`docs/agents/desktop.md`, D1).
pub type TrackId = String;

/// The stream qualities the server serves. `FLAC` is the archived master;
/// Opus is transcoded from it on demand. MP3 is never requested: the server
/// never sends it.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Quality {
    Flac,
    Opus320,
    Opus128,
    Opus64,
}

impl Quality {
    /// The `q=` value of `GET /api/stream/<id>`, or `None` for FLAC (the server
    /// treats an absent `q` as FLAC).
    pub fn as_query(self) -> Option<&'static str> {
        match self {
            Quality::Flac => None,
            Quality::Opus320 => Some("OPUS_320"),
            Quality::Opus128 => Some("OPUS_128"),
            Quality::Opus64 => Some("OPUS_64"),
        }
    }

    /// A stable key for spool file names and cache rows.
    pub fn as_str(self) -> &'static str {
        match self {
            Quality::Flac => "FLAC",
            Quality::Opus320 => "OPUS_320",
            Quality::Opus128 => "OPUS_128",
            Quality::Opus64 => "OPUS_64",
        }
    }

    /// The quality a settings value names, accepting the server's spelling.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_uppercase().as_str() {
            "FLAC" | "LOSSLESS" => Some(Quality::Flac),
            "OPUS_320" | "320" | "OPUS320" => Some(Quality::Opus320),
            "OPUS_128" | "128" | "OPUS128" => Some(Quality::Opus128),
            "OPUS_64" | "64" | "OPUS64" => Some(Quality::Opus64),
            _ => None,
        }
    }
}

/// The request lanes of rule 1, defined here so the engine can pass one to the
/// opener; the app maps it onto `nz-api`'s identical enum. Nothing in this
/// crate waits on, throttles or reorders a lane — that is `nz-api`'s job — but a
/// prefetched next track asks with `Prefetch`, and a download with `Bulk`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Lane {
    User,
    Prefetch,
    Bulk,
}

/// What `GET /api/stream/<id>/ready?q=` answers (`neezer/webui/__init__.py`,
/// `stream_ready`), field for field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamReady {
    /// A finished FILE will be served: length, byte ranges, a seek lands anywhere.
    pub ready: bool,
    /// `"bytes"` when the next answer is that file, else `None`.
    pub ranges: Option<String>,
    /// Size of the archive on disk, when there is one.
    pub bytes: Option<u64>,
    /// The Opus transcode for this quality is already cached.
    pub cached: bool,
    /// `?start=<seconds>` is honoured now: a finished file on disk can be
    /// encoded from that time in one request.
    pub offset: bool,
    /// Nothing is published yet and an archive is being written right now, so
    /// the wait is real and it ends.
    pub streaming: bool,
}

impl StreamReady {
    /// How long to wait before asking again, following `seekplan.js#readiness`:
    /// 1 s normally, 1.5 s while an archive is being written. The engine polls
    /// the route itself (it is cheap by contract) instead of spinning.
    pub fn retry_delay(&self) -> Duration {
        if self.streaming {
            Duration::from_millis(1_500)
        } else {
            Duration::from_millis(1_000)
        }
    }
}

/// Why a stream could not be opened. The app's `ApiError` maps onto it; the
/// engine only needs to name the cause in an [`EngineEvent::Error`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceError {
    /// The server could not be reached at all.
    Offline,
    /// `503` + `Retry-After: 30` for a prefetch whose track is not archived yet:
    /// means "try later", not "failed".
    NotReady { retry_after: u32 },
    /// `X-NS-Error` or any other classified refusal.
    Provider { status: u16, message: String },
    /// Anything else, already worded for the person.
    Message(String),
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceError::Offline => f.write_str("serveur injoignable"),
            SourceError::NotReady { retry_after } => {
                write!(f, "pas encore archivé (réessayer dans {retry_after} s)")
            }
            SourceError::Provider { status, message } => write!(f, "{message} ({status})"),
            SourceError::Message(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for SourceError {}

/// A blocking body of bytes. The app implements this over a reqwest response
/// (`reqwest::blocking::Response` or a channel-fed adapter); the tests implement
/// it over a file and over a local HTTP socket.
pub trait ByteStream: Read + Send {
    /// The total length the server stated, when it did. `None` for a body
    /// generated as it is sent — that is what makes a seek defer instead of
    /// jumping.
    fn len_hint(&self) -> Option<u64> {
        None
    }

    /// Where this body starts in the track, in seconds: the server's
    /// `X-NS-Start` on a `?start=` slice, `0.0` otherwise. The engine adds it
    /// to its position so the bar shows the track's time, not the slice's.
    fn start_offset_s(&self) -> f64 {
        0.0
    }

    /// The quality this body REALLY delivers, when it knows better than the
    /// request did.
    ///
    /// `None` — every existing body — means « the quality that was asked for »,
    /// which is what `GET /api/stream/<id>?q=` always answers. A body that
    /// serves a copy it already holds says so instead: a peer on the LAN serves
    /// the file it has, so an Opus 320 request can be answered in FLAC
    /// (`docs/agents/lan-nodes.md`, *Shared-crate changes*). The engine carries
    /// it in [`EngineEvent::StreamComplete`] so the app publishes the finished
    /// spool into the play cache under the quality the file really holds.
    fn quality(&self) -> Option<Quality> {
        None
    }
}

/// Everything the engine needs from the app's network and cache layers.
pub trait StreamOpener: Send + Sync {
    /// Open the body of `GET /api/stream/<id>?q=`, optionally from `start`
    /// seconds (`?start=`, honoured only when `/ready` says `offset`) or from a
    /// byte offset (`Range: bytes=<n>-`, for a finished file).
    fn open(
        &self,
        id: &str,
        quality: Quality,
        start: Option<f64>,
        from_byte: Option<u64>,
        lane: Lane,
    ) -> Result<Box<dyn ByteStream>, SourceError>;

    /// `GET /api/stream/<id>/ready?q=`: cheap by contract, never starts work.
    fn ready(&self, id: &str, quality: Quality) -> Result<StreamReady, SourceError>;

    /// A complete file for this (track, quality), if the play cache has one.
    fn local(&self, id: &str, quality: Quality) -> Option<PathBuf>;

    /// The engine is closing: abandon every wait in flight instead of letting
    /// it finish.
    ///
    /// The app's opener drives the network with `Handle::block_on` on the
    /// process's tokio runtime, and tokio PANICS — it does not return an error —
    /// when a timer is polled on a runtime that is shutting down
    /// (`Handle::block_on`'s own documentation: « timers will panic »). The
    /// engine's threads therefore cannot outlive that runtime: [`Engine::shutdown`]
    /// calls this first, so each `block_on` it is racing gives up at once, and
    /// then JOINS every thread it started. An opener that has no in-flight wait
    /// of its own keeps this no-op — the test openers are files, and a file read
    /// always returns.
    ///
    /// [`Engine::shutdown`]: crate::Engine::shutdown
    fn shutdown(&self) {}
}

/// The spool file name: a hash of (track, quality), never a path from the
/// server (`docs/agents/desktop.md`, *Caches*).
pub(crate) fn spool_path(dir: &Path, id: &str, quality: Quality) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    id.hash(&mut hasher);
    quality.as_str().hash(&mut hasher);
    dir.join(format!("stream-{:016x}", hasher.finish()))
}

/// What a restart needs: who to ask, for what, and how far ahead it may run.
struct Restart {
    opener: Arc<dyn StreamOpener>,
    id: String,
    quality: Quality,
    lane: Lane,
}

/// The spooler threads the engine owns, so it can JOIN them before the network
/// runtime they borrow goes away.
///
/// A spooler (`nz-audio-spool`) pulls a body chunk by chunk; on the app's opener
/// each pull is a `Handle::block_on`, and tokio panics — it returns no error —
/// when a timer is polled on a runtime that is shutting down. The handle cannot
/// stay inside the [`Spool`] it fills: that struct is itself owned by the
/// thread's own closure, so it could never be joined. The engine keeps them here
/// instead, prunes the ones that finished, and joins the rest in
/// [`Spoolers::join_all`].
///
/// A spooler a SEEK superseded is `abandon`ed, not joined: it may be inside a
/// blocking socket read of bytes nobody wants any more, and only the opener's
/// own stop (the app closing) can end that. Measured: joining one made
/// `seek_in_a_finished_network_file_uses_a_byte_range` hang for ever, and a
/// close that waited for such a read would blow the 50 ms budget on a dead link.
/// It cannot touch the runtime after the engine stops: every pull of its body
/// checks the stop first (`ApiByteStream::read`).
#[derive(Clone, Default)]
pub(crate) struct Spoolers(Arc<Mutex<HashMap<u64, JoinHandle<()>>>>);

impl Spoolers {
    /// Register a spooler thread and answer the ticket that names it.
    fn register(&self, handle: JoinHandle<()>) -> u64 {
        let mut threads = self.0.lock().unwrap_or_else(|error| error.into_inner());
        // A track change leaves its spooler behind: drop the handles of the
        // ones that are done instead of keeping one per track of a session.
        threads.retain(|_, thread| !thread.is_finished());
        let ticket = threads.keys().max().copied().unwrap_or(0) + 1;
        threads.insert(ticket, handle);
        ticket
    }

    /// Let go of a spooler without waiting for it (the handle is detached, as a
    /// dropped `JoinHandle` always is).
    fn abandon(&self, ticket: u64) {
        let handle = self
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&ticket);
        drop(handle);
    }

    /// Join every CURRENT spooler, and wait for it. Only the engine's shutdown
    /// calls this: by then `stop` is set on every spool and the opener's own stop
    /// has abandoned the chunk in flight, so no spooler waits for the network.
    pub(crate) fn join_all(&self) {
        let handles: Vec<JoinHandle<()>> = {
            let mut threads = self.0.lock().unwrap_or_else(|error| error.into_inner());
            std::mem::take(&mut *threads).into_values().collect()
        };
        let me = thread::current().id();
        for handle in handles {
            // A spooler can be the thread that drops the last source, and joining
            // yourself fails with `EDEADLK`; it is leaving anyway.
            if handle.thread().id() == me {
                continue;
            }
            let _ = handle.join();
        }
    }
}

/// One download attempt, writing sequentially into its own file.
struct Spool {
    path: PathBuf,
    writer: File,
    written: u64,
    /// Logical byte of this file's offset 0 (a `Range` restart starts mid-track).
    base: u64,
    eof: bool,
    error: Option<String>,
    stop: Arc<AtomicBool>,
}

/// Everything `HttpSource::new` needs about the track it is spooling, kept in
/// one struct so the constructor stays readable at the call site.
pub(crate) struct SourceSpec<'a> {
    pub dir: &'a Path,
    pub id: &'a str,
    pub quality: Quality,
    pub restart: Option<(Arc<dyn StreamOpener>, Lane)>,
    pub bus: Option<Arc<EventBus>>,
    /// Bytes the spooler may run ahead of the decoder (rule 1).
    pub ahead: u64,
    /// Only a whole track may be renamed into the cached name.
    pub publish: bool,
    /// The engine's own stop: set once, when the engine shuts down. A reader
    /// waiting for bytes returns at once instead of holding its decode thread,
    /// and no half body can be published as a complete one.
    pub stop: Arc<AtomicBool>,
    /// Where the spooler thread's handle goes: the engine joins it.
    pub spoolers: Spoolers,
}

/// A network body spooled to disk, read as a blocking `Read + Seek`.
pub(crate) struct HttpSource {
    /// Logical position in the body, in bytes.
    pos: u64,
    /// Total body length when the server stated one.
    len: Option<u64>,
    /// The decoder's position, shared with the spooler (stay `ahead` in front).
    read_pos: Arc<AtomicU64>,
    /// How many bytes the spooler may run ahead of `read_pos`.
    ahead: u64,
    /// `None` for a `?start=` slice: its bytes are not a seekable track image.
    restart: Option<Restart>,
    spool: Arc<Mutex<Spool>>,
    ready: Arc<Condvar>,
    /// Set when the source is dropped: the spooler stops.
    closed: Arc<AtomicBool>,
    /// The engine's stop, shared with every source it builds (`SourceSpec::stop`).
    stop: Arc<AtomicBool>,
    /// Where the spooler threads' handles go (the engine joins them).
    spoolers: Spoolers,
    /// The ticket of the CURRENT spooler: a seek abandons it instead of joining
    /// it (it may be inside a socket read).
    spooler: Option<u64>,
    /// True while symphonia is probing the container. `is_seekable` answers
    /// false then, so the probe does not scan the TRAILING metadata: that scan
    /// seeks to the end of the body, which over the network is a download of
    /// the band's last kilobyte for a tag nobody asked for.
    probing: Arc<AtomicBool>,
    /// The track this body belongs to, for the completion event.
    id: String,
    /// The quality this body really delivers: [`ByteStream::quality`] when the
    /// body knows better than the request, the requested rung otherwise. It is
    /// what [`EngineEvent::StreamComplete`] carries, so the app caches the file
    /// under the quality it holds.
    delivered: Quality,
    bus: Option<Arc<EventBus>>,
    /// The completed spool file is announced once.
    announced: bool,
    /// Only a body that is the WHOLE track may be renamed into the spool's
    /// final name: a `?start=` slice or a byte resume is a window, and caching
    /// it as the track would replay 3.5 s as if it were 0.
    publish: bool,
    final_path: PathBuf,
    /// Files left behind by a restart, removed when the source goes away.
    temps: Vec<PathBuf>,
}

impl HttpSource {
    /// Start spooling `body`. `final_path` is where the file is renamed when the
    /// body ended at byte 0 and is whole (so a later play reuses it as a local
    /// file); a restart from a later byte writes a `.part` and is dropped.
    pub(crate) fn new(body: Box<dyn ByteStream>, spec: SourceSpec<'_>) -> io::Result<Self> {
        let SourceSpec {
            dir,
            id,
            quality,
            restart,
            bus,
            ahead,
            publish,
            stop,
            spoolers,
        } = spec;
        let len = body.len_hint();
        // What the body really serves; a body that says nothing is the server,
        // which serves exactly the rung that was asked for.
        let delivered = body.quality().unwrap_or(quality);
        let final_path = spool_path(dir, id, quality);
        let path = final_path.with_extension("part");
        let mut options = OpenOptions::new();
        options.create(true).write(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let writer = options.open(&path)?;
        let read_pos = Arc::new(AtomicU64::new(0));
        let spool = Arc::new(Mutex::new(Spool {
            path,
            writer,
            written: 0,
            base: 0,
            eof: false,
            error: None,
            stop: Arc::new(AtomicBool::new(false)),
        }));
        let ready = Arc::new(Condvar::new());
        let restart_cfg = restart.map(|(opener, lane)| Restart {
            opener,
            id: id.to_string(),
            quality,
            lane,
        });
        let closed = Arc::new(AtomicBool::new(false));
        let mut source = Self {
            pos: 0,
            len,
            read_pos: Arc::clone(&read_pos),
            ahead,
            restart: restart_cfg,
            spool: Arc::clone(&spool),
            ready: Arc::clone(&ready),
            closed: Arc::clone(&closed),
            stop,
            spoolers,
            spooler: None,
            probing: Arc::new(AtomicBool::new(true)),
            id: id.to_string(),
            delivered,
            bus,
            announced: false,
            publish,
            final_path,
            temps: Vec::new(),
        };
        source.spawn_downloader(body, Arc::clone(&closed), read_pos);
        Ok(source)
    }

    fn spawn_downloader(
        &mut self,
        body: Box<dyn ByteStream>,
        closed: Arc<AtomicBool>,
        read_pos: Arc<AtomicU64>,
    ) {
        let ahead = self.ahead;
        let spool = Arc::clone(&self.spool);
        let ready = Arc::clone(&self.ready);
        let engine_stop = Arc::clone(&self.stop);
        let body = Arc::new(Mutex::new(body));
        let handle = thread::Builder::new()
            .name("nz-audio-spool".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    let stop = {
                        let guard = spool.lock().expect("spool poisoned");
                        guard.stop.load(Ordering::Relaxed)
                    };
                    if stop || closed.load(Ordering::Relaxed) {
                        break;
                    }
                    // The engine is closing: its network runtime is about to go,
                    // and this thread may not touch it (`nz-audio`'s sources
                    // race every pull against this flag).
                    if engine_stop.load(Ordering::Acquire) {
                        let mut guard = spool.lock().expect("spool poisoned");
                        guard.error = Some("moteur arrêté".into());
                        guard.eof = true;
                        drop(guard);
                        ready.notify_all();
                        break;
                    }
                    // Rule 1: stay `ahead` bytes in front of the decoder, never
                    // a whole track in front of it. A prefetched next track
                    // stops downloading once its 2 MiB are there.
                    loop {
                        let (base, written) = {
                            let guard = spool.lock().expect("spool poisoned");
                            (guard.base, guard.written)
                        };
                        let decoder = read_pos.load(Ordering::Acquire);
                        if base + written < decoder + ahead {
                            break;
                        }
                        if stop || closed.load(Ordering::Relaxed) {
                            return;
                        }
                        // 10 ms: the read-ahead cap is checked against a 0.5 s
                        // ring, so this is a frame of slack for half the wakeups.
                        thread::sleep(Duration::from_millis(10));
                    }
                    let read = {
                        let mut body = body.lock().expect("body poisoned");
                        body.read(&mut buf)
                    };
                    match read {
                        Ok(0) => {
                            let mut guard = spool.lock().expect("spool poisoned");
                            guard.eof = true;
                            drop(guard);
                            ready.notify_all();
                            break;
                        }
                        Ok(n) => {
                            let mut guard = spool.lock().expect("spool poisoned");
                            if let Err(e) = guard.writer.write_all(&buf[..n]) {
                                guard.error = Some(format!("écriture du tampon : {e}"));
                                guard.eof = true;
                                drop(guard);
                                ready.notify_all();
                                break;
                            }
                            guard.written += n as u64;
                            let base = guard.base;
                            let written = guard.written;
                            drop(guard);
                            // Publish the new high-water mark before waking the
                            // reader, so `read` never sees the wake-up first.
                            let _ = (base, written);
                            ready.notify_all();
                        }
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(e) => {
                            let mut guard = spool.lock().expect("spool poisoned");
                            guard.error = Some(format!("flux interrompu : {e}"));
                            guard.eof = true;
                            drop(guard);
                            ready.notify_all();
                            break;
                        }
                    }
                }
            })
            .expect("spawn the spooler thread");
        self.spooler = Some(self.spoolers.register(handle));
    }

    /// Stop the current download and start a new one at logical byte `target`.
    fn restart_at(&mut self, target: u64) -> io::Result<()> {
        let Some(cfg) = self.restart.as_ref() else {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "ce flux ne peut pas être rembobiné",
            ));
        };
        let body = cfg
            .opener
            .open(&cfg.id, cfg.quality, None, Some(target), cfg.lane)
            .map_err(|e| io::Error::other(e.to_string()))?;
        // The previous attempt keeps its own file and stops on its own; nothing
        // here waits for a blocked socket read. Its thread is ABANDONED, not
        // joined (`Spoolers::abandon`): it may be inside a read of bytes nobody
        // wants, and joining it hung a seek test for ever.
        {
            let guard = self.spool.lock().expect("spool poisoned");
            guard.stop.store(true, Ordering::Relaxed);
        }
        self.ready.notify_all();
        if let Some(ticket) = self.spooler.take() {
            self.spoolers.abandon(ticket);
        }

        let path = self.final_path.with_extension(format!("part.{target}"));
        self.temps.push(path.clone());
        let mut options = OpenOptions::new();
        options.create(true).write(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let writer = options.open(&path)?;
        let spool = Arc::new(Mutex::new(Spool {
            path: path.clone(),
            writer,
            written: 0,
            base: target,
            eof: false,
            error: None,
            stop: Arc::new(AtomicBool::new(false)),
        }));
        self.spool = Arc::clone(&spool);
        self.pos = target;
        self.read_pos.store(target, Ordering::Release);
        self.len = body.len_hint().map(|n| target + n);
        let read_pos = Arc::clone(&self.read_pos);
        self.spawn_downloader(body, Arc::clone(&self.closed), read_pos);
        Ok(())
    }

    fn logical_written(&self) -> (u64, bool, Option<String>) {
        let guard = self.spool.lock().expect("spool poisoned");
        (guard.base + guard.written, guard.eof, guard.error.clone())
    }

    /// Rename the whole spool into place and tell the app once.
    ///
    /// **A body that ended short of its `Content-Length`, or empty, is not the
    /// track**: renaming it would make the spool a local file the next play
    /// reads as a whole title, and a 45 s file ending after 12 s (or instantly)
    /// is the "same line reloaded" loop. The same rule applies to a 0-byte
    /// body, which a server can answer for a slice past the end.
    fn announce_if_complete(&mut self) {
        if self.announced || !self.publish {
            return;
        }
        let (written, eof, error) = self.logical_written();
        let base = self.spool.lock().expect("spool poisoned").base;
        if !eof || error.is_some() || base != 0 {
            return;
        }
        if written == 0 || self.len.is_some_and(|len| written < len) {
            tracing::debug!(
                id = %self.id,
                written,
                declared = ?self.len,
                "a short body is not published as a track"
            );
            return;
        }
        self.announced = true;
        let bytes = self.spool.lock().expect("spool poisoned").written;
        let part = self.spool.lock().expect("spool poisoned").path.clone();
        // Rename while holding no lock on the writer: it has finished.
        let final_path = self.final_path.clone();
        if std::fs::rename(&part, &final_path).is_ok()
            && let Some(bus) = &self.bus
        {
            bus.emit(EngineEvent::StreamComplete {
                id: self.id.clone(),
                quality: self.delivered,
                path: final_path,
                bytes,
            });
        }
    }
}

impl HttpSource {
    /// Cleared by `decode::prepare` once the container is probed.
    pub(crate) fn probe_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.probing)
    }
}

impl Read for HttpSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut reader: Option<File> = None;
        loop {
            let (avail_end, eof, error, base, path) = {
                let guard = self.spool.lock().expect("spool poisoned");
                (
                    guard.base + guard.written,
                    guard.eof,
                    guard.error.clone(),
                    guard.base,
                    guard.path.clone(),
                )
            };
            if self.pos < avail_end {
                // Reading past `buf` would read bytes the spooler has not
                // written; the high-water mark above is the boundary.
                let want = ((avail_end - self.pos) as usize).min(buf.len());
                if reader.is_none() {
                    reader = Some(File::open(&path)?);
                }
                let file = reader.as_mut().expect("opened above");
                file.seek(SeekFrom::Start(self.pos - base))?;
                let n = file.read(&mut buf[..want])?;
                self.pos += n as u64;
                self.read_pos.store(self.pos, Ordering::Release);
                return Ok(n);
            }
            if let Some(message) = error {
                return Err(io::Error::other(message));
            }
            if eof {
                self.announce_if_complete();
                return Ok(0);
            }
            if self.closed.load(Ordering::Relaxed) {
                return Ok(0);
            }
            // The engine is closing: stop waiting, and never let the `eof` above
            // rename a half spool into the play cache as a complete track.
            if self.stop.load(Ordering::Acquire) {
                return Ok(0);
            }
            let guard = self.spool.lock().expect("spool poisoned");
            let _ = self
                .ready
                .wait_timeout(guard, Duration::from_millis(50))
                .expect("spool poisoned");
        }
    }
}

impl Seek for HttpSource {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(byte) => byte,
            SeekFrom::Current(delta) => (self.pos as i64 + delta).max(0) as u64,
            SeekFrom::End(delta) => {
                let len = self.len.ok_or_else(|| {
                    io::Error::new(io::ErrorKind::Unsupported, "longueur inconnue")
                })?;
                (len as i64 + delta).max(0) as u64
            }
        };
        let (avail_end, base) = {
            let guard = self.spool.lock().expect("spool poisoned");
            (guard.base + guard.written, guard.base)
        };
        if target >= base && target <= avail_end {
            self.pos = target;
            self.read_pos.store(target, Ordering::Release);
            return Ok(target);
        }
        if self.restart.is_none() || self.len.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "ce flux ne peut pas être rembobiné",
            ));
        }
        self.restart_at(target)?;
        Ok(target)
    }
}

impl MediaSource for HttpSource {
    fn is_seekable(&self) -> bool {
        // The note's contract: seekable only when the length is known. A
        // generated body has neither a length nor a byte range. During the
        // probe, a network source says no for a second reason: a seekable
        // source makes symphonia read the trailing metadata, i.e. the END of
        // the body, before a single sample is played.
        !self.probing.load(Ordering::Acquire) && self.len.is_some() && self.restart.is_some()
    }

    fn byte_len(&self) -> Option<u64> {
        self.len
    }
}

impl Drop for HttpSource {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        if let Ok(guard) = self.spool.lock() {
            guard.stop.store(true, Ordering::Release);
            // A partial spool is worth nothing; a completed one was renamed.
            let _ = std::fs::remove_file(&guard.path);
        }
        for path in &self.temps {
            let _ = std::fs::remove_file(path);
        }
        self.ready.notify_all();
        // The spooler thread itself is NOT joined here: this runs on a decode
        // thread, which must never wait for a socket (a skip on a stalled link
        // would then block for the stream's 15 s timeout). It is the engine's
        // thread registry that joins them (`Spoolers::join_all`), before the
        // runtime they borrow goes away.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A body that counts what the spooler actually read from it.
    struct CountingBody {
        data: Vec<u8>,
        pos: usize,
        read: Arc<AtomicU64>,
        done: Arc<AtomicBool>,
    }

    impl Read for CountingBody {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let take = (self.data.len() - self.pos).min(buf.len()).min(8 * 1024);
            if take == 0 {
                self.done.store(true, Ordering::Release);
                return Ok(0);
            }
            buf[..take].copy_from_slice(&self.data[self.pos..self.pos + take]);
            self.pos += take;
            self.read.fetch_add(take as u64, Ordering::Relaxed);
            Ok(take)
        }
    }

    impl ByteStream for CountingBody {
        fn len_hint(&self) -> Option<u64> {
            Some(self.data.len() as u64)
        }
    }

    /// A body that knows which quality it really serves: a peer that holds only
    /// the FLAC, asked for Opus 320 (`ByteStream::quality`).
    struct DeclaredBody {
        data: Vec<u8>,
        pos: usize,
        quality: Option<Quality>,
    }

    impl Read for DeclaredBody {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let take = (self.data.len() - self.pos).min(buf.len()).min(8 * 1024);
            if take == 0 {
                return Ok(0);
            }
            buf[..take].copy_from_slice(&self.data[self.pos..self.pos + take]);
            self.pos += take;
            Ok(take)
        }
    }

    impl ByteStream for DeclaredBody {
        fn len_hint(&self) -> Option<u64> {
            Some(self.data.len() as u64)
        }

        fn quality(&self) -> Option<Quality> {
            self.quality
        }
    }

    /// Spool a whole 4 096-byte body through a real [`HttpSource`] and answer
    /// every event it emitted, plus the directory it wrote in.
    fn spool_a_body(
        served: Option<Quality>,
        requested: Quality,
        name: &str,
    ) -> (Vec<EngineEvent>, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("nz-audio-quality-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("the spool directory");
        let bus = EventBus::new();
        let events = bus.subscribe();
        let spoolers = Spoolers::default();
        let mut source = HttpSource::new(
            Box::new(DeclaredBody {
                data: vec![0x42; 4096],
                pos: 0,
                quality: served,
            }),
            SourceSpec {
                dir: &dir,
                id: name,
                quality: requested,
                restart: None,
                bus: Some(Arc::clone(&bus)),
                ahead: 1024 * 1024,
                publish: true,
                stop: Arc::new(AtomicBool::new(false)),
                spoolers: spoolers.clone(),
            },
        )
        .expect("a source");
        let mut bytes = Vec::new();
        source.read_to_end(&mut bytes).expect("the whole body");
        // The publication happens on the reading thread at EOF, so the event is
        // already queued; joining the spooler keeps the directory quiet.
        drop(source);
        spoolers.join_all();
        (events.try_iter().collect(), dir)
    }

    /// A body that KNOWS it is shorter than the length it declared: a connection
    /// that died after a chunk, which is not a track.
    struct ShortBody {
        data: Vec<u8>,
        pos: usize,
        declared: u64,
    }

    impl Read for ShortBody {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let take = (self.data.len() - self.pos).min(buf.len());
            if take == 0 {
                return Ok(0);
            }
            buf[..take].copy_from_slice(&self.data[self.pos..self.pos + take]);
            self.pos += take;
            Ok(take)
        }
    }

    impl ByteStream for ShortBody {
        fn len_hint(&self) -> Option<u64> {
            Some(self.declared)
        }
    }

    /// **A short body is not a track.** A connection that ends before its
    /// `Content-Length` (or a 0-byte answer, which a slice past the end
    /// produces) must not be renamed into the spool a later play reads as a
    /// whole title: that is the "same line reloaded at 43.5 s" loop, a 45 s file
    /// ending after 12 s — or instantly — forever.
    #[test]
    fn a_short_or_empty_body_is_never_published() {
        for (name, data, declared) in [
            ("short", vec![0x42; 1_000], 4_096u64),
            ("empty", vec![], 0u64),
        ] {
            let dir =
                std::env::temp_dir().join(format!("nz-audio-short-{name}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("the spool directory");
            let bus = EventBus::new();
            let events = bus.subscribe();
            let spoolers = Spoolers::default();
            let mut source = HttpSource::new(
                Box::new(ShortBody {
                    data,
                    pos: 0,
                    declared,
                }),
                SourceSpec {
                    dir: &dir,
                    id: name,
                    quality: Quality::Opus320,
                    restart: None,
                    bus: Some(Arc::clone(&bus)),
                    ahead: 1024 * 1024,
                    publish: true,
                    stop: Arc::new(AtomicBool::new(false)),
                    spoolers: spoolers.clone(),
                },
            )
            .expect("a source");
            let mut bytes = Vec::new();
            source
                .read_to_end(&mut bytes)
                .expect("the body is readable");
            drop(source);
            spoolers.join_all();
            let events: Vec<EngineEvent> = events.try_iter().collect();
            let final_path = spool_path(&dir, name, Quality::Opus320);
            println!(
                "{name}: {declared} declared, {bytes_len} read, published={}, events={}",
                final_path.exists(),
                events.len(),
                bytes_len = bytes.len()
            );
            assert!(
                !final_path.exists(),
                "a {name} body became a local track ({declared} declared, {} read)",
                bytes.len()
            );
            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, EngineEvent::StreamComplete { .. })),
                "a {name} body was announced as a complete track"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// **LN5, the engine's half** (`docs/agents/lan-nodes.md`, *Shared-crate
    /// changes*): the completion event names the quality the BODY served, so the
    /// app caches the file under it — a peer that only holds the FLAC answers an
    /// Opus 320 request with FLAC, and caching that under `OPUS_320` would make
    /// the engine believe it holds a transcode it never had.
    ///
    /// Measured: the whole 4 096-byte body is announced as `Flac`, with the file
    /// renamed into the spool exactly once.
    #[test]
    fn the_completion_event_carries_the_quality_the_body_really_served() {
        let (events, dir) = spool_a_body(Some(Quality::Flac), Quality::Opus320, "served-flac");
        let (quality, path, bytes) = events
            .iter()
            .find_map(|event| match event {
                EngineEvent::StreamComplete {
                    quality,
                    path,
                    bytes,
                    ..
                } => Some((*quality, path.clone(), *bytes)),
                _ => None,
            })
            .expect("one StreamComplete for the whole body");
        println!(
            "corps annoncé {quality:?} (demandé OPUS_320), {bytes} octets, {}",
            path.display()
        );
        assert_eq!(
            quality,
            Quality::Flac,
            "the FLAC the body served, not the OPUS_320 it was asked for"
        );
        assert_eq!(bytes, 4096);
        assert!(path.is_file(), "the spool was renamed into place");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The other half of the same rule: a body that says nothing is the server,
    /// which serves exactly the rung that was asked for — so every existing
    /// opener keeps its behaviour (`tests/resume.rs`, the app's `ApiByteStream`).
    #[test]
    fn a_body_that_says_nothing_keeps_the_requested_quality() {
        let (events, dir) = spool_a_body(None, Quality::Opus128, "served-asked");
        let quality = events
            .iter()
            .find_map(|event| match event {
                EngineEvent::StreamComplete { quality, .. } => Some(*quality),
                _ => None,
            })
            .expect("one StreamComplete for the whole body");
        assert_eq!(quality, Quality::Opus128);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Rule 1 on the device: a body is spooled at most `ahead` bytes in front
    /// of the decoder, never whole. The measurement is exact — the body counts
    /// its own reads — where a socket would only show kernel buffering.
    ///
    /// Measured: with a 64 KiB cap and a 4 MiB body, the spooler read
    /// **8 192 B** while the decoder read nothing, then stopped.
    #[test]
    fn the_spooler_stops_at_its_read_ahead_cap() {
        let dir = std::env::temp_dir().join(format!("nz-audio-source-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let read = Arc::new(AtomicU64::new(0));
        let done = Arc::new(AtomicBool::new(false));
        let body = CountingBody {
            data: vec![0x5a; 4 * 1024 * 1024],
            pos: 0,
            read: Arc::clone(&read),
            done: Arc::clone(&done),
        };
        let spoolers = Spoolers::default();
        let source = HttpSource::new(
            Box::new(body),
            SourceSpec {
                dir: &dir,
                id: "cap-test",
                quality: Quality::Flac,
                restart: None,
                bus: None,
                ahead: 64 * 1024,
                publish: true,
                stop: Arc::new(AtomicBool::new(false)),
                spoolers: spoolers.clone(),
            },
        )
        .expect("source");
        std::thread::sleep(std::time::Duration::from_millis(100));
        let consumed = read.load(Ordering::Relaxed);
        println!("spooled with a 64 KiB cap: {consumed} B");
        assert!(
            consumed <= 64 * 1024 + 8 * 1024,
            "the spooler read {consumed} B, past its 64 KiB cap"
        );
        assert!(!done.load(Ordering::Relaxed), "the whole body was read");
        drop(source);
        // The engine owns the spooler thread and joins it: a dropped source only
        // asks it to stop.
        let closing = std::time::Instant::now();
        spoolers.join_all();
        println!("spooler joint en {:?}", closing.elapsed());
        assert!(
            closing.elapsed() < std::time::Duration::from_millis(50),
            "the spooler was joined in {:?}",
            closing.elapsed()
        );
    }

    #[test]
    fn quality_round_trips_the_servers_spelling() {
        assert_eq!(Quality::Opus320.as_query(), Some("OPUS_320"));
        assert_eq!(Quality::Flac.as_query(), None);
        for quality in [
            Quality::Flac,
            Quality::Opus320,
            Quality::Opus128,
            Quality::Opus64,
        ] {
            assert_eq!(Quality::parse(quality.as_str()), Some(quality));
        }
        assert_eq!(Quality::parse("opus_128"), Some(Quality::Opus128));
    }

    #[test]
    fn a_spool_name_is_a_hash_not_a_path() {
        let dir = Path::new("/tmp/neezer-test");
        let name = spool_path(dir, "../../etc/passwd", Quality::Flac);
        assert_eq!(name.parent(), Some(dir));
        let file = name.file_name().unwrap().to_string_lossy().to_string();
        assert!(file.starts_with("stream-"));
        assert!(!file.contains('/'));
        assert!(!file.contains(".."));
    }

    #[test]
    fn readiness_waits_longer_while_an_archive_is_being_written() {
        let idle = StreamReady::default();
        let writing = StreamReady {
            streaming: true,
            ..StreamReady::default()
        };
        assert_eq!(idle.retry_delay(), Duration::from_millis(1_000));
        assert_eq!(writing.retry_delay(), Duration::from_millis(1_500));
    }
}
