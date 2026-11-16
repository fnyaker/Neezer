//! The engine handle: what the app calls, and the threads it hides.
//!
//! Every method returns at once. Anything that can block — opening a stream,
//! probing a container, asking `/stream/<id>/ready` until a seek can land — runs
//! on a loader thread, because rule 4 says a click answers in the same frame and
//! a network open can take a second. The decode threads and the output then do
//! the rest without ever calling back into here.
//!
//! The seek policy is the browser's (`docs/agents/playback.md`, *A seek is
//! answered by the server, or explicitly deferred*): a finished file is seeked
//! in place (locally or with a `Range` request), a source that has to be encoded
//! is reopened with `?start=<seconds>`, and a source that is still being written
//! is awaited by polling the cheap `/ready` route — 1 s, 1.5 s while `streaming`
//! — instead of restarting from 0.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use rtrb::{Producer, PushError};

use crate::decode::{self, DecodeError};
use crate::events::{EngineEvent, EventBus, VoiceInfo, VoiceInfoEntry};
use crate::mixer::{MixerCmd, MixerShared};
use crate::output::{self, OfflineSink, Output, OutputChoice, OutputInfo};
use crate::source::{Lane, Quality, SourceError, StreamOpener, spool_path};
use crate::voice::{VoiceConsumer, VoiceStatus};

/// Everything the engine needs to start. `sample_rate` is a preference: the
/// device's own rate wins when it cannot do this one, and the decode threads
/// resample to whatever the engine settles on.
#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub sample_rate: u32,
    /// 10 ms at 48 kHz: the PipeWire buffer the callback sees.
    pub period_frames: u32,
    /// The decoded audio a voice may run ahead of the output, in seconds.
    pub ring_seconds: f32,
    pub volume: f32,
    pub device: OutputChoice,
    /// Where a network body is spooled while it arrives. Defaults to
    /// `$XDG_CACHE_HOME/neezer/audio`.
    pub spool_dir: PathBuf,
    /// How long a deferred seek keeps polling `/ready` before it gives up.
    pub seek_poll_limit: Duration,
    /// How many bytes a network source may spool ahead of the decoder. Rule 1
    /// in one number: a prefetched next track stops at this distance instead of
    /// downloading itself whole while the track being heard competes for the
    /// link.
    pub read_ahead_bytes: u64,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            period_frames: 480,
            ring_seconds: 0.5,
            volume: 1.0,
            device: OutputChoice::Default,
            spool_dir: default_spool_dir(),
            seek_poll_limit: Duration::from_secs(30),
            read_ahead_bytes: 2 * 1024 * 1024,
        }
    }
}

fn default_spool_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_CACHE_HOME")
        && !dir.is_empty()
    {
        return PathBuf::from(dir).join("neezer").join("audio");
    }
    if let Ok(home) = std::env::var("HOME")
        && !home.is_empty()
    {
        return PathBuf::from(home)
            .join(".cache")
            .join("neezer")
            .join("audio");
    }
    std::env::temp_dir().join("neezer-audio")
}

/// Why the engine could not do what was asked. Every variant has a sentence the
/// person can read (`docs/agents/desktop.md`, rule 3).
#[derive(Debug)]
pub enum EngineError {
    Source(SourceError),
    Decode(String),
    Output(String),
    /// No track is loaded, so there is nothing to seek in.
    NoTrack,
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::Source(e) => write!(f, "{e}"),
            EngineError::Decode(m) | EngineError::Output(m) => f.write_str(m),
            EngineError::NoTrack => f.write_str("aucune piste en lecture"),
        }
    }
}

impl std::error::Error for EngineError {}

impl From<SourceError> for EngineError {
    fn from(value: SourceError) -> Self {
        EngineError::Source(value)
    }
}

impl From<DecodeError> for EngineError {
    fn from(value: DecodeError) -> Self {
        EngineError::Decode(value.to_string())
    }
}

/// A loaded voice, before it is handed to the mixer.
struct Built {
    consumer: VoiceConsumer,
    status: Arc<VoiceStatus>,
    handle: JoinHandle<()>,
    generation: u64,
    position_s: f64,
    duration_s: Option<f64>,
}

impl Built {
    fn stop(&self) {
        self.status.stop();
    }
}

/// What the engine remembers about a voice so it can stop or replace it.
struct Slot {
    status: Arc<VoiceStatus>,
    #[allow(dead_code)]
    handle: JoinHandle<()>,
}

/// Only the voice THIS engine installed last as current. The mixer owns the
/// real current (it switches on its own at a gapless handover, and the engine
/// reads `current_generation` for that); this is only what to stop when a new
/// load replaces it.
struct State {
    current: Option<Slot>,
}

/// The engine, cheap to clone into the UI.
#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

struct Inner {
    cfg: EngineConfig,
    opener: Arc<dyn StreamOpener>,
    shared: Arc<MixerShared>,
    mixer: Mutex<Producer<MixerCmd>>,
    events: Arc<EventBus>,
    info: Arc<VoiceInfo>,
    output: Mutex<Output>,
    out_info: Mutex<OutputInfo>,
    state: Mutex<State>,
    /// Bumped by every `load`/`seek`: a loader whose mark is stale installs
    /// nothing.
    load_gen: AtomicU64,
    gen_seq: AtomicU64,
    playing: AtomicBool,
    stopped: AtomicBool,
    event_stop: Arc<AtomicBool>,
    event_thread: Mutex<Option<JoinHandle<()>>>,
}

impl Engine {
    /// Start on the real output (PipeWire, then ALSA, then a paced silence
    /// pump when the machine has no sound server).
    pub fn new(cfg: EngineConfig, opener: Arc<dyn StreamOpener>) -> Self {
        Self::build(cfg, opener)
    }

    /// Start with a pull-driven sink the caller renders: what the acceptance
    /// tests use, so a gapless seam does not race a sound card.
    pub fn new_offline(cfg: EngineConfig, opener: Arc<dyn StreamOpener>) -> (Self, OfflineSink) {
        Self::build_offline(cfg, opener)
    }

    fn build(cfg: EngineConfig, opener: Arc<dyn StreamOpener>) -> Self {
        let _ = std::fs::create_dir_all(&cfg.spool_dir);
        let events = EventBus::new();
        let notes = events.take_note_producer();
        let (output, shared, mixer, out_info) = output::start(
            &cfg.device,
            cfg.sample_rate,
            cfg.volume,
            cfg.period_frames,
            Arc::clone(&events),
            notes,
        );
        let info = Arc::new(VoiceInfo::default());
        let event_stop = Arc::new(AtomicBool::new(false));
        let event_thread = events.spawn_bridge(
            Arc::clone(&event_stop),
            Arc::clone(&info),
            Arc::clone(&shared),
        );
        let engine = Engine {
            inner: Arc::new(Inner {
                cfg,
                opener,
                shared,
                mixer: Mutex::new(mixer),
                events,
                info,
                output: Mutex::new(output),
                out_info: Mutex::new(out_info),
                state: Mutex::new(State { current: None }),
                load_gen: AtomicU64::new(0),
                gen_seq: AtomicU64::new(0),
                playing: AtomicBool::new(false),
                stopped: AtomicBool::new(false),
                event_stop,
                event_thread: Mutex::new(Some(event_thread)),
            }),
        };
        engine.inner.emit_output();
        engine
    }

    fn build_offline(cfg: EngineConfig, opener: Arc<dyn StreamOpener>) -> (Self, OfflineSink) {
        let _ = std::fs::create_dir_all(&cfg.spool_dir);
        let events = EventBus::new();
        let notes = events.take_note_producer();
        let (output, sink, shared, mixer) =
            output::start_offline(cfg.sample_rate, cfg.volume, notes);
        let out_info = OutputInfo {
            device: "sortie de test".into(),
            sample_rate: cfg.sample_rate,
            channels: 2,
            latency_frames: 0,
        };
        let info = Arc::new(VoiceInfo::default());
        let event_stop = Arc::new(AtomicBool::new(false));
        let event_thread = events.spawn_bridge(
            Arc::clone(&event_stop),
            Arc::clone(&info),
            Arc::clone(&shared),
        );
        let engine = Engine {
            inner: Arc::new(Inner {
                cfg,
                opener,
                shared,
                mixer: Mutex::new(mixer),
                events,
                info,
                output: Mutex::new(output),
                out_info: Mutex::new(out_info),
                state: Mutex::new(State { current: None }),
                load_gen: AtomicU64::new(0),
                gen_seq: AtomicU64::new(0),
                playing: AtomicBool::new(false),
                stopped: AtomicBool::new(false),
                event_stop,
                event_thread: Mutex::new(Some(event_thread)),
            }),
        };
        (engine, sink)
    }

    /// Load a track. Returns at once; `Started` says when it is audible.
    pub fn load(&self, id: &str, quality: Quality, start: Option<f64>) {
        let inner = Arc::clone(&self.inner);
        let mark = inner.load_gen.fetch_add(1, Ordering::SeqCst) + 1;
        let id = id.to_string();
        spawn_loader(format!("nz-load-{mark}"), move || {
            match build_at(&inner, &id, quality, start, Lane::User, true) {
                Ok(built) => inner.install_current(mark, &id, quality, built),
                Err(e) => inner.fail(Some(&id), e),
            }
        });
    }

    /// Queue the track that follows: decoded up to 0.5 s ahead, then idle, so
    /// the handover is a ring the mixer already has.
    pub fn preload_next(&self, id: &str, quality: Quality) {
        let inner = Arc::clone(&self.inner);
        let mark = inner.load_gen.load(Ordering::SeqCst);
        let id = id.to_string();
        spawn_loader(format!("nz-preload-{id}"), move || {
            if inner.stopped.load(Ordering::Relaxed)
                || mark != inner.load_gen.load(Ordering::SeqCst)
            {
                return;
            }
            match build_at(&inner, &id, quality, None, Lane::Prefetch, false) {
                Ok(built) => inner.install_next(mark, &id, quality, built),
                // A prefetch that fails is not a verdict about the track: the
                // app can try again when it becomes current (rule 3, *A failure
                // to reach a provider is never a verdict*).
                Err(e) => inner.events.emit(EngineEvent::Error {
                    id: Some(id),
                    message: e.to_string(),
                }),
            }
        });
    }

    pub fn play(&self) {
        self.inner.playing.store(true, Ordering::SeqCst);
        self.inner.shared.set_paused(false);
    }

    pub fn pause(&self) {
        self.inner.playing.store(false, Ordering::SeqCst);
        self.inner.shared.set_paused(true);
    }

    pub fn is_playing(&self) -> bool {
        self.inner.playing.load(Ordering::SeqCst) && !self.inner.shared.paused()
    }

    /// Seek in the current track. Applies at once (`Seeked`), or after the
    /// server can answer (`SeekDeferred`, then `Seeked`).
    pub fn seek(&self, position_s: f64) {
        let target = position_s.max(0.0);
        let inner = Arc::clone(&self.inner);
        let mark = inner.load_gen.fetch_add(1, Ordering::SeqCst) + 1;
        let Some((id, quality)) = inner.now_playing() else {
            inner.events.emit(EngineEvent::Error {
                id: None,
                message: EngineError::NoTrack.to_string(),
            });
            return;
        };
        spawn_loader(format!("nz-seek-{mark}"), move || {
            inner.run_seek(mark, &id, quality, target)
        });
    }

    pub fn set_volume(&self, volume: f32) {
        self.inner.shared.set_volume(volume);
    }

    pub fn set_mute(&self, muted: bool) {
        self.inner.shared.set_muted(muted);
    }

    /// Switch the output device without stopping the music: the mixer is handed
    /// to the new stream. `None` is the host's default.
    pub fn set_output_device(&self, device: Option<&str>) -> Result<(), EngineError> {
        let choice = match device {
            Some(name) => OutputChoice::named(name),
            None => OutputChoice::Default,
        };
        self.inner
            .output
            .lock()
            .expect("output poisoned")
            .set_device(choice)
            .map_err(|e| EngineError::Output(e.to_string()))
    }

    /// Seconds of the current track that have reached the speakers.
    pub fn position(&self) -> f64 {
        self.inner.shared.clock.position()
    }

    pub fn events(&self) -> Receiver<EngineEvent> {
        self.inner.events.subscribe()
    }

    pub fn underruns(&self) -> u64 {
        self.inner.shared.underruns()
    }

    pub fn output_info(&self) -> OutputInfo {
        self.inner
            .output
            .lock()
            .expect("output poisoned")
            .info()
            .unwrap_or_else(|| {
                self.inner
                    .out_info
                    .lock()
                    .expect("output info poisoned")
                    .clone()
            })
    }

    /// The id of the track the mixer is playing (or about to), if any. Read
    /// from the mixer's own current voice, so it is right after a gapless
    /// handover too.
    pub fn current_id(&self) -> Option<String> {
        self.inner.now_playing().map(|(id, _)| id)
    }
}

impl Inner {
    fn emit_output(&self) {
        let info = self.out_info.lock().expect("output info poisoned").clone();
        self.events.emit(EngineEvent::OutputChanged {
            device: info.device,
            sample_rate: info.sample_rate,
        });
    }

    /// The track the mixer is actually playing: its generation, then the table
    /// that names it. After a gapless switch this is the NEXT track, which the
    /// engine's installed-current bookkeeping no longer represents.
    fn now_playing(&self) -> Option<(String, Quality)> {
        let generation = self.shared.current_generation();
        if generation == 0 {
            return None;
        }
        self.info
            .get(generation)
            .map(|entry| (entry.id, entry.quality))
    }

    fn next_gen(&self) -> u64 {
        self.gen_seq.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn send(&self, command: MixerCmd) {
        let mut guard = self.mixer.lock().expect("mixer poisoned");
        let mut pending = command;
        for _ in 0..500 {
            match guard.push(pending) {
                Ok(()) => return,
                Err(PushError::Full(returned)) => {
                    pending = returned;
                    thread::sleep(Duration::from_millis(1));
                }
            }
        }
        // The output stopped draining: the engine is going away, and dropping a
        // mixer command then is harmless.
    }

    fn fail(&self, id: Option<&str>, error: EngineError) {
        self.events.emit(EngineEvent::Error {
            id: id.map(str::to_string),
            message: error.to_string(),
        });
    }

    fn install_current(&self, mark: u64, id: &str, quality: Quality, built: Built) {
        if self.stopped.load(Ordering::Relaxed) || mark != self.load_gen.load(Ordering::SeqCst) {
            built.stop();
            return;
        }
        self.info.insert(
            built.generation,
            VoiceInfoEntry {
                id: id.to_string(),
                quality,
                start_s: built.position_s,
                duration_s: built.duration_s,
            },
        );
        let mut state = self.state.lock().expect("engine state poisoned");
        if let Some(old) = state.current.replace(Slot {
            status: Arc::clone(&built.status),
            handle: built.handle,
        }) {
            old.status.stop();
        }
        drop(state);
        // Whatever was preloaded belongs to the old queue position: playing it
        // now would jump the queue.
        self.send(MixerCmd::InstallNext(None));
        self.send(MixerCmd::InstallCurrent(Some(built.consumer)));
        if self.playing.load(Ordering::SeqCst) {
            self.shared.set_paused(false);
        }
    }

    fn install_next(&self, mark: u64, id: &str, quality: Quality, built: Built) {
        if self.stopped.load(Ordering::Relaxed) || mark != self.load_gen.load(Ordering::SeqCst) {
            built.stop();
            return;
        }
        self.info.insert(
            built.generation,
            VoiceInfoEntry {
                id: id.to_string(),
                quality,
                start_s: built.position_s,
                duration_s: built.duration_s,
            },
        );
        // The mixer replaces (and stops) any previously queued next voice on
        // its own; the engine keeps no second copy that could go stale at a
        // gapless handover.
        self.send(MixerCmd::InstallNext(Some(built.consumer)));
        // The handle must outlive this call: dropping a JoinHandle only detaches,
        // which is what we want, and the status stops the thread.
        drop(built.handle);
        drop(built.status);
    }

    /// Install the voice a seek produced, then announce it only once the clock
    /// carries the new position: the bar must never move before the sound does
    /// (`docs/agents/playback.md`, *A transport action always wins*).
    fn land_seek(&self, mark: u64, id: &str, quality: Quality, built: Built) {
        let position_s = built.position_s;
        self.install_current(mark, id, quality, built);
        let deadline = Instant::now() + Duration::from_millis(500);
        while (self.shared.clock.base() - position_s).abs() > 0.001 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        self.events.emit(EngineEvent::Seeked { position_s });
    }

    /// The seek policy, on its own thread.
    fn run_seek(&self, mark: u64, id: &str, quality: Quality, target: f64) {
        let cancelled =
            || self.stopped.load(Ordering::Relaxed) || mark != self.load_gen.load(Ordering::SeqCst);
        // A complete local file (the play cache, or a finished spool) is seeked
        // in place: no request at all.
        if local_path(self, id, quality).is_some() {
            match build_at(self, id, quality, Some(target), Lane::User, false) {
                Ok(built) => self.land_seek(mark, id, quality, built),
                Err(e) => self.fail(Some(id), e),
            }
            return;
        }
        let started = Instant::now();
        let mut deferred = false;
        loop {
            if cancelled() {
                return;
            }
            match self.opener.ready(id, quality) {
                Ok(ready) if ready.ready || ready.offset => {
                    match build_at(self, id, quality, Some(target), Lane::User, false) {
                        Ok(built) => self.land_seek(mark, id, quality, built),
                        Err(e) => self.fail(Some(id), e),
                    }
                    return;
                }
                Ok(ready) => {
                    if !deferred {
                        deferred = true;
                        self.events
                            .emit(EngineEvent::SeekDeferred { target_s: target });
                    }
                    if started.elapsed() > self.cfg.seek_poll_limit {
                        self.fail(
                            Some(id),
                            EngineError::Decode(
                                "le serveur ne peut pas encore répondre à ce déplacement".into(),
                            ),
                        );
                        return;
                    }
                    thread::sleep(ready.retry_delay());
                }
                Err(e) => {
                    self.fail(Some(id), e.into());
                    return;
                }
            }
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        {
            let mut state = self.state.lock().expect("engine state poisoned");
            if let Some(slot) = state.current.take() {
                slot.status.stop();
            }
        }
        self.send(MixerCmd::Shutdown);
        self.event_stop.store(true, Ordering::Release);
        if let Some(handle) = self
            .event_thread
            .lock()
            .expect("event thread poisoned")
            .take()
        {
            let _ = handle.join();
        }
    }
}

fn spawn_loader(name: String, task: impl FnOnce() + Send + 'static) {
    let _ = thread::Builder::new()
        .name(name)
        .stack_size(512 * 1024)
        .spawn(task);
}

fn local_path(inner: &Inner, id: &str, quality: Quality) -> Option<PathBuf> {
    if let Some(path) = inner.opener.local(id, quality)
        && path.is_file()
    {
        return Some(path);
    }
    let spool = spool_path(&inner.cfg.spool_dir, id, quality);
    if spool.is_file() { Some(spool) } else { None }
}

/// Open the best source for this request and prepare the decoder. `wait` makes
/// a source that cannot answer a seek yet be awaited (the seek policy); a
/// prefetch never waits.
fn build_at(
    inner: &Inner,
    id: &str,
    quality: Quality,
    start: Option<f64>,
    lane: Lane,
    wait: bool,
) -> Result<Built, EngineError> {
    let target = start.filter(|seconds| *seconds > 0.001);
    // 1. A finished local file: no request, and a seek is a file seek.
    if let Some(path) = local_path(inner, id, quality) {
        let file = std::fs::File::open(&path)
            .map_err(|e| EngineError::Decode(format!("{} : {e}", path.display())))?;
        let prepared = decode::prepare(
            Box::new(file),
            target.unwrap_or(0.0),
            inner.cfg.sample_rate,
            None,
        )?;
        return finish(inner, prepared, id, target.unwrap_or(0.0));
    }

    // 2. A network body. What the server says decides the mechanism.
    let deadline = Instant::now() + inner.cfg.seek_poll_limit;
    let mut announced_deferred = false;
    loop {
        if inner.stopped.load(Ordering::Relaxed) {
            return Err(EngineError::Decode("moteur arrêté".into()));
        }
        let ready = if target.is_some() {
            let ready = inner.opener.ready(id, quality)?;
            if !ready.ready && !ready.offset {
                if !wait {
                    return Err(EngineError::Source(SourceError::NotReady {
                        retry_after: 2,
                    }));
                }
                if !announced_deferred {
                    announced_deferred = true;
                    inner.events.emit(EngineEvent::SeekDeferred {
                        target_s: target.unwrap_or(0.0),
                    });
                }
                if Instant::now() > deadline {
                    return Err(EngineError::Decode(
                        "le serveur ne peut pas encore répondre à ce déplacement".into(),
                    ));
                }
                thread::sleep(ready.retry_delay());
                continue;
            }
            Some(ready)
        } else {
            None
        };

        // `?start=` is the server slicing a finished file at a time; byte ranges
        // are the file itself, seeked by the container.
        let slice = ready.as_ref().map(|ready| ready.offset).unwrap_or(false);
        let body = match (target, slice) {
            (Some(seconds), true) => inner.opener.open(id, quality, Some(seconds), None, lane)?,
            _ => inner.opener.open(id, quality, None, None, lane)?,
        };
        let position_s = if slice {
            body.start_offset_s()
        } else {
            target.unwrap_or(0.0)
        };
        let spool_dir = inner.cfg.spool_dir.clone();
        let restart = match (target, slice) {
            // A slice is a window into the track, not the track's bytes: it must
            // never be seeked as if it started at 0.
            (_, true) => None,
            _ => Some((Arc::clone(&inner.opener), lane)),
        };
        // A prefetched next track may run far less far ahead than the one being
        // heard: 512 KiB of FLAC is several seconds, and the rest of rule 1 is
        // `nz-api`'s lane.
        let ahead = if lane == Lane::Prefetch {
            inner.cfg.read_ahead_bytes.min(512 * 1024)
        } else {
            inner.cfg.read_ahead_bytes
        };
        let source = crate::source::HttpSource::new(
            body,
            crate::source::SourceSpec {
                dir: &spool_dir,
                id,
                quality,
                restart,
                bus: Some(Arc::clone(&inner.events)),
                ahead,
                // A body opened at 0 without `?start=` is the whole track;
                // anything else is a window and must never become the cached
                // track.
                publish: !slice && target.is_none(),
            },
        )
        .map_err(|e| EngineError::Decode(format!("tampon disque : {e}")))?;
        let probing = source.probe_handle();
        let seek_s = if slice { 0.0 } else { target.unwrap_or(0.0) };
        let prepared = decode::prepare(
            Box::new(source),
            seek_s,
            inner.cfg.sample_rate,
            Some(probing),
        )?;
        return finish(inner, prepared, id, position_s);
    }
}

fn finish(
    inner: &Inner,
    prepared: decode::Prepared,
    id: &str,
    position_s: f64,
) -> Result<Built, EngineError> {
    let duration_s = prepared.duration_s;
    let generation = inner.next_gen();
    let (consumer, handle) = decode::spawn(
        prepared,
        inner.cfg.sample_rate,
        inner.cfg.ring_seconds,
        generation,
        position_s,
        Arc::clone(&inner.events),
        id.to_string(),
    )?;
    let status = consumer.status();
    Ok(Built {
        consumer,
        status,
        handle,
        generation,
        position_s,
        duration_s,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_spool_directory_is_under_the_cache_home() {
        // Not an environment assertion (CI sets XDG_CACHE_HOME): the path must
        // simply be absolute and end in neezer/audio.
        let dir = default_spool_dir();
        assert!(dir.is_absolute(), "{}", dir.display());
        assert!(dir.ends_with("neezer/audio"), "{}", dir.display());
    }

    #[test]
    fn quality_round_trips_through_the_engine_reexport() {
        assert_eq!(Quality::parse("OPUS_64"), Some(Quality::Opus64));
    }
}
