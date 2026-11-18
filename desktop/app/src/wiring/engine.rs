//! The audio engine, and the only bridge between it and the network.
//!
//! `nz-audio` is deliberately synchronous (`StreamOpener::open` returns a
//! blocking `Read`), while `nz-api` is async: this module is where the two meet
//! and nowhere else. A request is issued on `nz-net` and awaited from the
//! engine's own loader/decode thread with `Handle::block_on`; the body is
//! pulled chunk by chunk, so a stream that stalls cannot hold a slot on the
//! User lane (D2 releases it once the headers arrive) and the decoder's
//! read-ahead cap is what bounds memory (rule 1).
//!
//! Every one of those waits races a `watch`: [`shutdown_process`] — run by the
//! app's `NetRuntime` in its `Drop`, before its runtime goes — stops the engine,
//! the engine's own `shutdown` joins its threads, and none of them can be inside
//! the runtime afterwards. Tokio PANICS, it does not return an error, when a
//! timer is polled on a runtime that is shutting down (`Handle::block_on`'s own
//! documentation: « timers will panic »), so this is a lifetime, not an error
//! (`docs/agents/desktop.md`, D13 *Pitfalls*).
//!
//! The engine starts on a named thread so the first painted frame never waits
//! for PipeWire; its events are forwarded through [`crate::wiring::events`] and
//! the spool of a completed stream is published into the play cache here, which
//! is the only place that knows both file names.
//!
//! Every wait this module drives is raced against a `watch`: [`shutdown_process`]
//! — called by the app's `NetRuntime` in its `Drop`, before the runtime goes —
//! stops the engine, the engine joins its own threads, and none of them can be
//! inside this runtime afterwards. Tokio PANICS, it does not return an error,
//! when a timer is polled on a runtime that is shutting down (`Handle::block_on`'s
//! own documentation: « timers will panic »), so this is a lifetime, not an
//! error (`docs/agents/desktop.md`, D13 *Pitfalls*).

use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use nz_api::{Api, ApiError, StreamResponse};
use nz_audio::{
    ByteStream, Engine, EngineConfig, EngineHandle, SourceError, StreamOpener, StreamReady,
};
use nz_cache::Cache;
use tokio::sync::watch;
use tracing::{debug, warn};

use crate::state::AppEvent;
use crate::wiring::events::Sender;

/// The engine plus the rate it settled on.
#[derive(Clone)]
pub struct Audio {
    pub engine: Engine,
    pub sample_rate: f32,
}

impl Audio {
    /// Where the bar reads the playhead from: the mixer's own clock, not a
    /// timer in the UI.
    pub fn position(&self) -> f64 {
        self.engine.position()
    }
}

/// The quality the person chose. `Auto` is the plan's ladder starting point;
/// the measured step-down (>= 600 kB/s → 320, …) lands with the player screen
/// that shows it, and until then Auto is exactly Opus 320.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlaybackQuality {
    #[default]
    Auto,
    Flac,
    Opus320,
    Opus128,
    Opus64,
}

impl PlaybackQuality {
    pub fn key(self) -> &'static str {
        match self {
            PlaybackQuality::Auto => "auto",
            PlaybackQuality::Flac => "flac",
            PlaybackQuality::Opus320 => "opus_320",
            PlaybackQuality::Opus128 => "opus_128",
            PlaybackQuality::Opus64 => "opus_64",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "flac" => PlaybackQuality::Flac,
            "opus_320" | "320" => PlaybackQuality::Opus320,
            "opus_128" | "128" => PlaybackQuality::Opus128,
            "opus_64" | "64" => PlaybackQuality::Opus64,
            _ => PlaybackQuality::Auto,
        }
    }

    /// What the engine asks the server for.
    pub fn audio(self) -> nz_audio::Quality {
        match self {
            PlaybackQuality::Auto | PlaybackQuality::Opus320 => nz_audio::Quality::Opus320,
            PlaybackQuality::Flac => nz_audio::Quality::Flac,
            PlaybackQuality::Opus128 => nz_audio::Quality::Opus128,
            PlaybackQuality::Opus64 => nz_audio::Quality::Opus64,
        }
    }
}

fn api_quality(quality: nz_audio::Quality) -> nz_api::Quality {
    match quality {
        nz_audio::Quality::Flac => nz_api::Quality::Flac,
        nz_audio::Quality::Opus320 => nz_api::Quality::Opus320,
        nz_audio::Quality::Opus128 => nz_api::Quality::Opus128,
        nz_audio::Quality::Opus64 => nz_api::Quality::Opus64,
    }
}

fn source_error(error: ApiError) -> SourceError {
    match error {
        ApiError::Offline => SourceError::Offline,
        ApiError::Busy { retry_after } | ApiError::NotReady { retry_after } => {
            SourceError::NotReady { retry_after }
        }
        ApiError::Provider {
            status, message, ..
        } => SourceError::Provider { status, message },
        other => SourceError::Message(other.cause_fr()),
    }
}

fn engine_lane(lane: nz_audio::Lane) -> nz_api::Lane {
    match lane {
        nz_audio::Lane::User => nz_api::Lane::User,
        nz_audio::Lane::Prefetch => nz_api::Lane::Prefetch,
        nz_audio::Lane::Bulk => nz_api::Lane::Bulk,
    }
}

/// `nz-audio`'s window of the server's `/stream` routes, over `nz-api`.
///
/// Every call into the network runs on an engine thread (a loader, a decode
/// thread, a spooler) with `Handle::block_on` on the process's tokio runtime.
/// That is why this type owns a STOP too: tokio panics — it returns no error —
/// when a timer is polled on a runtime that is shutting down
/// (`Handle::block_on`: « timers will panic »), so the app's `NetRuntime` global
/// stops this opener and joins the engine's threads before it drops the
/// runtime (`shutdown_process`, called from `NetRuntime::drop`).
pub struct NetOpener {
    api: Arc<Api>,
    runtime: tokio::runtime::Handle,
    cache: Cache,
    /// `true` means « the app is closing »: every wait races this, so a close
    /// abandons the request in flight instead of waiting for the next chunk.
    gone: watch::Sender<bool>,
}

impl NetOpener {
    pub fn new(api: Arc<Api>, runtime: tokio::runtime::Handle, cache: Cache) -> Self {
        Self {
            api,
            runtime,
            cache,
            gone: watch::channel(false).0,
        }
    }

    /// Abandon every request in flight: the caller is closing, and no answer is
    /// wanted any more.
    pub fn shutdown(&self) {
        let _ = self.gone.send(true);
    }
}

impl StreamOpener for NetOpener {
    fn open(
        &self,
        id: &str,
        quality: nz_audio::Quality,
        start: Option<f64>,
        from_byte: Option<u64>,
        lane: nz_audio::Lane,
    ) -> Result<Box<dyn ByteStream>, SourceError> {
        let id = nz_api::TrackId::from(id);
        let mut gone = self.gone.subscribe();
        let response = block_on(
            &self.runtime,
            &mut gone,
            self.api.stream(
                engine_lane(lane),
                &id,
                api_quality(quality),
                start,
                from_byte,
            ),
        )
        .ok_or_else(closed)?
        .map_err(source_error)?;
        Ok(Box::new(ApiByteStream {
            response,
            runtime: self.runtime.clone(),
            gone,
            pending: Vec::new(),
            done: false,
        }))
    }

    fn ready(&self, id: &str, quality: nz_audio::Quality) -> Result<StreamReady, SourceError> {
        let id = nz_api::TrackId::from(id);
        let mut gone = self.gone.subscribe();
        let ready = block_on(
            &self.runtime,
            &mut gone,
            self.api.ready(&id, api_quality(quality)),
        )
        .ok_or_else(closed)?
        .map_err(source_error)?;
        Ok(StreamReady {
            ready: ready.ready,
            ranges: ready.ranges,
            bytes: ready.bytes,
            cached: ready.cached,
            offset: ready.offset,
            streaming: ready.streaming,
        })
    }

    /// A complete local file for a track: the play cache first, then the
    /// person's own DOWNLOAD (lot D13).
    ///
    /// A download is permanent and lives in its own directory, at the quality it
    /// was fetched at — so it is found whatever the bar's quality says. That is
    /// what makes a track chosen for offline play with the link cut: without
    /// this line the engine asks the network for a file that is already on disk.
    fn local(&self, id: &str, quality: nz_audio::Quality) -> Option<PathBuf> {
        let track = nz_model::TrackId::new(id);
        if let Some(path) = self
            .cache
            .play_cache()
            .lookup(&track, api_quality(quality))
            .ok()
            .flatten()
        {
            return Some(path);
        }
        let owner = self.api.owner()?;
        self.cache
            .downloads()
            .local(&owner, &track)
            .ok()
            .flatten()
            .map(|(path, _quality)| path)
    }

    /// The engine is closing: abandon every wait in flight, on every thread it
    /// owns. The engine calls this before it joins those threads, and the
    /// runtime dies only once they are gone (`shutdown_process`).
    fn shutdown(&self) {
        NetOpener::shutdown(self);
    }
}

/// A streaming body read synchronously by the decoder.
struct ApiByteStream {
    response: StreamResponse,
    runtime: tokio::runtime::Handle,
    /// The app's stop, raced against every chunk read.
    gone: watch::Receiver<bool>,
    pending: Vec<u8>,
    done: bool,
}

impl ByteStream for ApiByteStream {
    fn len_hint(&self) -> Option<u64> {
        self.response.content_length
    }

    fn start_offset_s(&self) -> f64 {
        self.response.start_s.unwrap_or(0.0)
    }
}

impl Read for ApiByteStream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.pending.is_empty() {
            if self.done {
                return Ok(0);
            }
            let Self {
                response,
                runtime,
                gone,
                pending,
                done,
            } = self;
            match block_on(runtime, gone, response.chunk()) {
                Some(Ok(Some(chunk))) => *pending = chunk,
                Some(Ok(None)) => {
                    *done = true;
                    return Ok(0);
                }
                Some(Err(error)) => {
                    return Err(io::Error::other(error.cause_fr()));
                }
                // The app is closing. This is an ERROR and not an end of body:
                // the spool must never rename a half body into the play cache
                // as a complete track.
                None => return Err(io::Error::other("application fermée")),
            }
        }
        let take = out.len().min(self.pending.len());
        out[..take].copy_from_slice(&self.pending[..take]);
        self.pending.drain(..take);
        Ok(take)
    }
}

/// Why an `open`/`ready` gave up: the app is closing ([`NetOpener::shutdown`]).
fn closed() -> SourceError {
    SourceError::Message("application fermée".into())
}

/// Run one future on the app's network runtime, and give it up the moment the
/// app closes.
///
/// `None` means « the app is closing »: the future was dropped before it
/// finished, and the caller names that cause. There is deliberately NO
/// `catch_unwind` here — a panicking worker is a bug — and none is needed: the
/// engine stops and joins every thread that calls this before the runtime is
/// dropped (`nz_audio::Engine::shutdown`), which is the lifetime the panic came
/// from. The `select!` is what makes that join immediate: a close abandons a
/// request in flight instead of waiting for its answer.
fn block_on<T>(
    runtime: &tokio::runtime::Handle,
    gone: &mut watch::Receiver<bool>,
    future: impl std::future::Future<Output = T>,
) -> Option<T> {
    // A stop that landed between two chunks must not even enter the runtime: a
    // `block_on` there is exactly what panics.
    if *gone.borrow() {
        return None;
    }
    runtime.block_on(async {
        tokio::select! {
            biased;
            // `Err` means the sender is gone: the owner that could still ask
            // us to stop went with it, so stop.
            _ = gone.wait_for(|stop| *stop) => None,
            value = future => Some(value),
        }
    })
}

/// **Every engine this process started, so the close path can stop and JOIN
/// them.**
///
/// The engine's threads borrow the network runtime with `Handle::block_on`
/// inside [`NetOpener`], and tokio panics when a timer is polled on a runtime
/// that is shutting down. The engine owns its threads and joins them
/// (`nz_audio::Engine::shutdown`); this registry is how the runtime's owner —
/// the `NetRuntime` global in `main` — reaches the engine, whose only other
/// owner is the shell's state (which gpui drops first). Weak handles: a
/// registry entry never keeps a dead engine alive.
static ENGINES: OnceLock<Mutex<Vec<EngineHandle>>> = OnceLock::new();

fn engines() -> &'static Mutex<Vec<EngineHandle>> {
    ENGINES.get_or_init(|| Mutex::new(Vec::new()))
}

/// Register a freshly built engine, so [`shutdown_process`] can join it.
pub(crate) fn attach(engine: &Engine) {
    let mut registered = engines().lock().unwrap_or_else(|error| error.into_inner());
    // A previous session's engine, or one the shell dropped on its own, is not
    // this process's live engine any more.
    registered.retain(EngineHandle::is_alive);
    registered.push(engine.handle());
}

/// Stop every engine this process started, and WAIT for them.
///
/// This is what the app's `NetRuntime` global calls from its `Drop`: the
/// engine's loader, decode and spooler threads borrow that very runtime, so they
/// must be joined BEFORE its `Arc<Runtime>` field goes (Rust drops the fields
/// after the `Drop` body). Idempotent, and a no-op when no engine ever started.
pub fn shutdown_process() {
    let mut registered = engines().lock().unwrap_or_else(|error| error.into_inner());
    let handles: Vec<EngineHandle> = std::mem::take(&mut *registered);
    drop(registered);
    for handle in handles {
        handle.shutdown();
    }
}

/// Start the engine on its own thread and announce it. Returns immediately: the
/// window paints first, the sound arrives when PipeWire is ready.
pub fn spawn_audio(
    api: Arc<Api>,
    cache: Cache,
    runtime: tokio::runtime::Handle,
    events: Sender<AppEvent>,
) {
    // One engine per process: a new session's engine replaces the previous one,
    // and the previous one's threads borrow the SAME runtime. Stop and join it
    // first, or a sign-in would leave a player running on the old session (and
    // its threads inside the runtime when the app closes).
    shutdown_process();
    let failure = events.clone();
    let spawned = std::thread::Builder::new()
        .name("nz-audio-start".to_owned())
        .spawn(move || {
            // The LAN node starts before this opener is wrapped: the probe and
            // the index read are this thread's job (never the GPUI one), and the
            // wrapper is transparent until the node lands (`wiring::lan`).
            crate::wiring::lan::start_in_background(Arc::clone(&api), cache.clone());
            let opener =
                crate::wiring::lan::opener(Arc::new(NetOpener::new(api, runtime, cache.clone())));
            // A 48 kHz preference: the device's own rate wins when it cannot do
            // this one, and the decode threads resample.
            let config = EngineConfig {
                spool_dir: cache.audio_dir(),
                sample_rate: 48_000,
                ..EngineConfig::default()
            };
            let engine = Engine::new(config, opener);
            attach(&engine);
            let sample_rate = engine.output_info().sample_rate as f32;
            let receiver = engine.events();
            let tx = events.clone();
            let _ = std::thread::Builder::new()
                .name("nz-audio-events".to_owned())
                .spawn(move || {
                    while let Ok(event) = receiver.recv() {
                        tx.send(AppEvent::Engine(event));
                    }
                });
            events.send(AppEvent::AudioReady(Ok(Box::new(Audio {
                engine,
                sample_rate,
            }))));
        });
    if let Err(error) = spawned {
        failure.send(AppEvent::AudioReady(Err(format!(
            "le moteur audio n'a pas pu démarrer : {error}"
        ))));
    }
}

/// Publish the engine's completed spool into the play cache: the spool is the
/// track's bytes, and the cache is what lets the next play come off disk.
///
/// A `?start=` slice is never announced by the engine (`StreamComplete` is only
/// emitted for a whole body), so a window can never become a "complete" entry.
pub fn publish_spool(
    cache: &Cache,
    id: &str,
    quality: nz_audio::Quality,
    path: &PathBuf,
    bytes: u64,
) {
    let track = nz_model::TrackId::new(id);
    let quality = api_quality(quality);
    let play_cache = cache.play_cache();
    let reserved = match play_cache.reserve(&track, quality) {
        Ok(path) => path,
        Err(error) => {
            warn!(track = id, %error, "réservation du cache de lecture impossible");
            return;
        }
    };
    if reserved != *path {
        // Same directory in the default install (the engine spools into
        // `$XDG_CACHE_HOME/neezer/audio`), so this is a rename; a custom spool
        // directory falls back to a copy rather than losing the entry.
        if fs::rename(path, &reserved).is_err()
            && let Err(error) = fs::copy(path, &reserved)
        {
            debug!(track = id, %error, "copie du spool dans le cache de lecture impossible");
            let _ = fs::remove_file(&reserved);
            return;
        }
    }
    if let Err(error) = play_cache.complete(&track, quality, bytes) {
        warn!(track = id, %error, "publication du cache de lecture impossible");
    }
    // The engine reuses its spool until the cache published it; once renamed it
    // is already gone, and a copied one is removed here.
    let _ = fs::remove_file(path);
}

/// The player's periodic refresh: position and MPRIS, at 4 Hz. Kept here so the
/// shell's task and `nz-audio`'s clock cannot disagree about what "now" is.
pub const TICK: Duration = Duration::from_millis(250);

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Instant;

    use nz_audio::{Engine, EngineConfig, EngineEvent, Quality};

    use super::*;
    use crate::wiring::offline::tests::{
        Link, Panics, Payload, Scratch, TestServer, api_for, in_its_own_process, pump_until,
        runtime, tone_flac, wait_for,
    };

    #[test]
    fn the_quality_ladder_names_the_wire_and_the_engine_the_same() {
        for (setting, audio, api) in [
            (
                PlaybackQuality::Auto,
                nz_audio::Quality::Opus320,
                nz_api::Quality::Opus320,
            ),
            (
                PlaybackQuality::Opus320,
                nz_audio::Quality::Opus320,
                nz_api::Quality::Opus320,
            ),
            (
                PlaybackQuality::Opus128,
                nz_audio::Quality::Opus128,
                nz_api::Quality::Opus128,
            ),
            (
                PlaybackQuality::Opus64,
                nz_audio::Quality::Opus64,
                nz_api::Quality::Opus64,
            ),
            (PlaybackQuality::Flac, Quality::Flac, nz_api::Quality::Flac),
        ] {
            assert_eq!(setting.audio(), audio);
            assert_eq!(api_quality(audio), api);
        }
        for setting in [
            PlaybackQuality::Auto,
            PlaybackQuality::Flac,
            PlaybackQuality::Opus320,
            PlaybackQuality::Opus128,
            PlaybackQuality::Opus64,
        ] {
            assert_eq!(PlaybackQuality::parse(setting.key()), setting);
        }
    }

    #[test]
    fn an_api_failure_becomes_the_cause_the_bar_can_print() {
        assert_eq!(source_error(ApiError::Offline), SourceError::Offline);
        assert_eq!(
            source_error(ApiError::Busy { retry_after: 30 }),
            SourceError::NotReady { retry_after: 30 }
        );
        match source_error(ApiError::Provider {
            status: 502,
            code: "deezer-unreachable".into(),
            message: "Deezer ne répond pas".into(),
            retry_in: 12,
        }) {
            SourceError::Provider { status, message } => {
                assert_eq!(status, 502);
                assert_eq!(message, "Deezer ne répond pas");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    /// **LN5, the app's half** (`docs/agents/lan-nodes.md`, *Shared-crate
    /// changes*): the play cache is published under the quality the ENGINE
    /// reported, not the one the bar asked for.
    ///
    /// A peer that holds only the FLAC answers an Opus 320 request with FLAC —
    /// `nz-audio`'s `StreamComplete` then carries `Flac` instead of `Opus320`
    /// (`the_completion_event_carries_the_quality_the_body_really_served`), and
    /// this is the one function that turns that event into a cache row. Caching
    /// those bytes under `OPUS_320` would make the engine believe it holds a
    /// transcode it never had, and `local()` would answer for a rung whose file
    /// does not exist.
    ///
    /// Measured: a 4 096-byte spool announced as `Flac` is in the play cache
    /// under `FLAC` and nowhere under `OPUS_320`, and the spool file was moved.
    #[test]
    fn a_spool_announced_as_flac_is_cached_under_flac() {
        let scratch = Scratch::new("publish-spool");
        let cache = scratch.store();
        let spool = scratch.dir().join("stream-spool");
        std::fs::write(&spool, vec![3u8; 4096]).expect("the spool");

        publish_spool(&cache, "3135556", Quality::Flac, &spool, 4096);

        let track = nz_model::TrackId::new("3135556");
        let cached = cache
            .play_cache()
            .lookup(&track, nz_api::Quality::Flac)
            .expect("the play cache");
        println!("cache de lecture : {cached:?}");
        assert!(cached.is_some(), "the bytes really were FLAC");
        assert_eq!(
            cache
                .play_cache()
                .lookup(&track, nz_api::Quality::Opus320)
                .expect("the play cache"),
            None,
            "no entry may claim a transcode that never existed"
        );
        assert!(!spool.exists(), "the spool was renamed into the cache");
    }

    /// The engine's own threads, by the names `nz-audio` gives them: every one
    /// of them may borrow the network runtime, so the close must leave none.
    fn engine_threads() -> Vec<String> {
        let mut names = Vec::new();
        let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
            return names;
        };
        for task in tasks.flatten() {
            let Ok(comm) = std::fs::read_to_string(task.path().join("comm")) else {
                continue;
            };
            let name = comm.trim().to_owned();
            if [
                "nz-load-",
                "nz-preload-",
                "nz-seek-",
                "nz-decode-",
                "nz-audio-",
            ]
            .iter()
            .any(|prefix| name.starts_with(prefix))
            {
                names.push(name);
            }
        }
        names.sort();
        names
    }

    /// **Closing the app during a track: the engine stops, nothing panics.**
    ///
    /// This is the second owner `docs/agents/desktop.md` (D13 *Pitfalls*) names:
    /// the engine's loader, decode and spooler threads borrow the process's tokio
    /// runtime with `Handle::block_on` (through [`NetOpener`]), and tokio PANICS —
    /// « A Tokio 1.x context was found, but it is being shutdown. » — when a
    /// timer is polled on a runtime that is shutting down. The suite would still
    /// report `ok`, exactly as `nz-bulk`'s panic did in run 43964917017.
    ///
    /// It drops what the app drops, in the app's order: the shell's state (which
    /// owns the engine) first, then the `NetRuntime` global (whose `Drop` is what
    /// a quit runs), and only then the runtime. The engine is closed in the
    /// middle of three things at once — a track loading/streaming, that track
    /// playing, and the next one preloading — and the test proves 0 panics, a
    /// close under 50 ms, and no engine thread left.
    #[test]
    fn closing_the_app_during_a_track_stops_the_engine_without_a_panic() {
        in_its_own_process("wiring::engine::tests::the_close_bench_in_its_own_process");
    }

    #[test]
    #[ignore = "run by its parent, a process of its own (a panic hook is process-wide)"]
    fn the_close_bench_in_its_own_process() {
        let panics = Panics::install();
        let scratch = Scratch::new("engine-close");
        let store = scratch.store();

        // Real audio (a real decoder, real resampling), because "the track
        // plays" is the state the app closes in. Without ffmpeg there is nothing
        // to play: the test says so and passes (CI installs it).
        let flac = scratch.dir().join("tone.flac");
        if !tone_flac(&flac) {
            println!("ffmpeg absent : le critère de fermeture n'a pas été mesuré");
            return;
        }
        let audio = std::fs::read(&flac).expect("the generated FLAC");
        let mut bodies: HashMap<String, Payload> = HashMap::new();
        bodies.insert("current".to_owned(), Payload::Bytes(audio.clone()));
        bodies.insert("next".to_owned(), Payload::Bytes(audio));
        // A link slow enough that BOTH bodies are still arriving when the window
        // closes: a finished download would leave nothing in flight to stop.
        let server = TestServer::start(Link::new(Some(120_000.0)), bodies);
        let runtime = Arc::new(runtime());
        let api = api_for(&server, &store, &scratch);

        let opener = Arc::new(NetOpener::new(api, runtime.handle().clone(), store.clone()));
        let config = EngineConfig {
            sample_rate: 48_000,
            // A quarter-second ring: the audio a test needs, and half the memory
            // — `cargo test` runs the screens' RSS assertions in this process.
            ring_seconds: 0.25,
            spool_dir: store.audio_dir(),
            ..EngineConfig::default()
        };
        let (engine, mut sink) = Engine::new_offline(config, opener);
        // What `spawn_audio` does: the runtime's owner must be able to reach the
        // engine, because gpui drops the shell's state before the globals.
        attach(&engine);
        let events = engine.events();

        // The track loads, and the next one is ASKED for in the same breath — the
        // way a queue does it: the prefetch parks behind the tap's own User
        // request, on the Prefetch lane's quiet period, which is a tokio TIMER
        // (`Gates::acquire`: `timeout_at`). That timer is the panic the CI log
        // carried once the runtime goes.
        engine.load("current", Quality::Flac, None);
        engine.preload_next("next", Quality::Flac);
        // ... and the track plays.
        assert!(
            pump_until(
                &mut sink,
                &events,
                Duration::from_secs(15),
                |event| matches!(event, EngineEvent::Started { .. })
            ),
            "the track started"
        );
        engine.play();
        // Rendered, not slept: the mixer only needs buffers, and the close must
        // land while the prefetch is still in its quiet period.
        let mut buffer = vec![0f32; 480 * 2];
        let mut heard = 0.0f32;
        for _ in 0..20 {
            sink.render(&mut buffer);
            heard = heard.max(buffer.iter().fold(0.0f32, |top, s| top.max(s.abs())));
        }
        assert!(heard > 0.001, "the playing track was audible ({heard:.4})");
        assert!(
            server.delivered("current") > 0,
            "the playhead's body really came over the link"
        );
        assert!(
            wait_for(Duration::from_secs(10), || {
                engine_threads()
                    .iter()
                    .any(|name| name.starts_with("nz-preload-"))
            }),
            "the prefetch loader is running"
        );
        // The three states the close must survive, asserted rather than assumed:
        // a decode thread runs, a body is spooling, and a loader is inside the
        // runtime (it holds the engine alive, which is why the registry matters).
        let before = engine_threads();
        println!("fils du moteur avant la fermeture : {before:?}");
        assert!(
            before.iter().any(|name| name.starts_with("nz-preload-")),
            "the prefetch loader is in flight: {before:?}"
        );
        assert!(
            before.iter().any(|name| name.starts_with("nz-decode-")),
            "a track is being decoded: {before:?}"
        );
        assert!(
            before.iter().any(|name| name == "nz-audio-spool"),
            "a body is being spooled: {before:?}"
        );

        // The window closes. The shell's state goes first, the network runtime's
        // global second (its `Drop` is what a quit runs): the loader still holds
        // the engine here, so only the registry can reach it.
        let closing = Instant::now();
        drop(engine);
        let net = crate::NetRuntime(Arc::clone(&runtime));
        drop(net);
        let closed = closing.elapsed();
        // The rule, asserted where it is decided: when the close RETURNS, the
        // engine's threads are gone — joined, not left to a runtime that is
        // about to shut down under them. This is the assertion the pre-fix code
        // cannot pass: it returned with the loader, the voice and the spooler
        // still running.
        let left_at_close = engine_threads();
        // The kernel's task entry can outlive `pthread_join` by a moment (a
        // 2-core CI runner still listed `nz-audio-events` right after the close),
        // so this one is bounded and event-based — while the pre-fix code does
        // not come near it: its loader and spooler only end when the runtime
        // dies, which happens after this check.
        let joined = left_at_close.is_empty()
            || wait_for(Duration::from_millis(50), || engine_threads().is_empty());
        // Only now may the runtime go: nothing of the engine can be inside it.
        drop(runtime);
        // A thread that survived the close would hit a shutdown runtime HERE,
        // where a timer polled on it panics.
        std::thread::sleep(Duration::from_millis(300));

        let after = engine_threads();
        let recorded = panics.take();
        println!(
            "fermeture pendant lecture + préchargement : rendue en {closed:?}, \
             {} panique(s), fils restants {after:?}",
            recorded.len()
        );
        assert!(
            recorded.is_empty(),
            "no thread may panic while the app closes: {recorded:?}"
        );
        assert!(
            joined,
            "the close JOINS the engine's threads before it returns: {left_at_close:?}"
        );
        assert!(
            closed < Duration::from_millis(50),
            "the close answered in {closed:?}, not behind the body in flight"
        );
        assert!(
            after.is_empty(),
            "the engine's threads are joined before the runtime goes: {after:?}"
        );
        // The truncated spool is not a track: nothing was published into the play
        // cache, and the next play fetches it again (rule 3: an interruption is
        // not a verdict, and a half file is not a file).
        assert!(
            store
                .play_cache()
                .lookup(&nz_model::TrackId::new("current"), nz_api::Quality::Flac)
                .expect("the play cache")
                .is_none(),
            "a half body must never become a complete cached track"
        );
        // And the same for the prefetched one.
        assert!(
            store
                .play_cache()
                .lookup(&nz_model::TrackId::new("next"), nz_api::Quality::Flac)
                .expect("the play cache")
                .is_none(),
            "a prefetched half body must not be published either"
        );
    }
}
