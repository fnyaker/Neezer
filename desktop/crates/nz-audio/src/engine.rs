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

use crate::crossfade::FadeSettings;
use crate::decode::{self, DecodeError};
use crate::dsp::{Normalization, Settings};
use crate::events::{EngineEvent, EventBus, VoiceInfo, VoiceInfoEntry};
use crate::mixer::{MixerCmd, MixerShared};
use crate::output::{self, OfflineSink, Output, OutputChoice, OutputInfo};
use crate::source::{Lane, Quality, SourceError, Spoolers, StreamOpener, spool_path};
use crate::tap::{TAP_FRAMES, Tap, TapReader};
use crate::trim::{self, Edges, Trim};
use crate::voice::{VoiceConsumer, VoiceMeta, VoiceStatus};

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
}

/// Only the voice THIS engine installed last as current. The mixer owns the
/// real current (it switches on its own at a gapless handover, and the engine
/// reads `current_generation` for that); this is only what to stop when a new
/// load replaces it.
struct State {
    current: Option<Slot>,
}

/// What the app knows about a track before it is played: the server's
/// measurements and the kind of track it is.
///
/// Every field is optional because the app asks the server for them ahead of
/// time (`lib/gaincache.js#primeGains`, `Player.svelte#primeEdgesAround`) and a
/// track whose answer has not arrived yet simply plays un-normalised and
/// untrimmed — never late, never as a verdict about the track (rule 3).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TrackMeta {
    /// The server's ReplayGain LOUDNESS (`GET /api/gain/<id>`, or one entry of
    /// `POST /api/gains`), NOT the gain to apply: `dsp::gain_db` turns it into
    /// one with the level the person chose.
    pub loudness: Option<f32>,
    /// Where the audio really starts and stops (`GET /api/audio/edges/<id>?db=`).
    pub edges: Option<Edges>,
    /// The track's own length from the metadata, in seconds: it is what tells a
    /// re-archive's bounds from this file's.
    pub duration_s: Option<f64>,
    /// An episode: no crossfade into or out of it, and the podcast speed
    /// applies (`Player.svelte#edgesFor`, `speed.js`).
    pub podcast: bool,
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
    /// The DSP, the speed, the crossfade and the trim: written by the app from
    /// any thread, read by the output callback and the decode threads.
    settings: Arc<Settings>,
    /// The analysis tap, taken once by the app's analysis thread.
    tap: Mutex<Option<TapReader>>,
    /// What the app told us about the tracks around the queue position.
    meta: Mutex<Vec<(String, TrackMeta)>>,
    output: Mutex<Output>,
    out_info: Mutex<OutputInfo>,
    state: Mutex<State>,
    /// Bumped by every `load`/`seek`: a loader whose mark is stale installs
    /// nothing.
    load_gen: AtomicU64,
    gen_seq: AtomicU64,
    playing: AtomicBool,
    stopped: Arc<AtomicBool>,
    /// Everything asked to stop once, when the engine closes ([`Inner::shutdown`]).
    shut: AtomicBool,
    /// The loader threads this engine started (a load, a preload, a seek). The
    /// engine JOINS them when it closes: each one borrows the app's network
    /// runtime inside `StreamOpener`, and that runtime may not be dropped under
    /// a thread still inside it — tokio panics on a timer polled on a runtime
    /// that is shutting down (`docs/agents/desktop.md`, D13 *Pitfalls*).
    loaders: Mutex<Vec<JoinHandle<()>>>,
    /// The decode threads, with the flag that stops each one. Kept here, not only
    /// in the mixer, so a shutdown can stop every voice — the current one, the
    /// preloaded next one and one in a crossfade — and join them.
    voices: Mutex<Vec<(Arc<VoiceStatus>, JoinHandle<()>)>>,
    /// The spooler threads the network sources started.
    spoolers: Spoolers,
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
        let settings = Settings::new();
        let (tap, reader) = Tap::pair(TAP_FRAMES);
        let (output, shared, mixer, out_info) = output::start(
            &cfg.device,
            cfg.sample_rate,
            cfg.volume,
            cfg.period_frames,
            Arc::clone(&events),
            notes,
            Arc::clone(&settings),
            tap,
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
                settings,
                tap: Mutex::new(Some(reader)),
                meta: Mutex::new(Vec::new()),
                output: Mutex::new(output),
                out_info: Mutex::new(out_info),
                state: Mutex::new(State { current: None }),
                load_gen: AtomicU64::new(0),
                gen_seq: AtomicU64::new(0),
                playing: AtomicBool::new(false),
                stopped: Arc::new(AtomicBool::new(false)),
                shut: AtomicBool::new(false),
                loaders: Mutex::new(Vec::new()),
                voices: Mutex::new(Vec::new()),
                spoolers: Spoolers::default(),
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
        let settings = Settings::new();
        let (tap, reader) = Tap::pair(TAP_FRAMES);
        let (output, sink, shared, mixer) = output::start_offline(
            cfg.sample_rate,
            cfg.volume,
            notes,
            Arc::clone(&settings),
            tap,
        );
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
                settings,
                tap: Mutex::new(Some(reader)),
                meta: Mutex::new(Vec::new()),
                output: Mutex::new(output),
                out_info: Mutex::new(out_info),
                state: Mutex::new(State { current: None }),
                load_gen: AtomicU64::new(0),
                gen_seq: AtomicU64::new(0),
                playing: AtomicBool::new(false),
                stopped: Arc::new(AtomicBool::new(false)),
                shut: AtomicBool::new(false),
                loaders: Mutex::new(Vec::new()),
                voices: Mutex::new(Vec::new()),
                spoolers: Spoolers::default(),
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
        let loading = Arc::clone(&inner);
        inner.spawn_loader(format!("nz-load-{mark}"), move || {
            match build_at(&loading, &id, quality, start, Lane::User, true) {
                Ok(built) => loading.install_current(mark, &id, quality, built),
                Err(e) => loading.fail(Some(&id), e),
            }
        });
    }

    /// Queue the track that follows: decoded up to 0.5 s ahead, then idle, so
    /// the handover is a ring the mixer already has.
    pub fn preload_next(&self, id: &str, quality: Quality) {
        let inner = Arc::clone(&self.inner);
        let mark = inner.load_gen.load(Ordering::SeqCst);
        let id = id.to_string();
        let loading = Arc::clone(&inner);
        inner.spawn_loader(format!("nz-preload-{id}"), move || {
            if loading.stopped.load(Ordering::Relaxed)
                || mark != loading.load_gen.load(Ordering::SeqCst)
            {
                return;
            }
            match build_at(&loading, &id, quality, None, Lane::Prefetch, false) {
                Ok(built) => loading.install_next(mark, &id, quality, built),
                // A prefetch that fails is not a verdict about the track: the
                // app can try again when it becomes current (rule 3, *A failure
                // to reach a provider is never a verdict*). It is silent only
                // when the engine itself is closing.
                Err(e) => loading.fail(Some(&id), e),
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
        let seeking = Arc::clone(&inner);
        inner.spawn_loader(format!("nz-seek-{mark}"), move || {
            seeking.run_seek(mark, &id, quality, target)
        });
    }

    pub fn set_volume(&self, volume: f32) {
        self.inner.shared.set_volume(volume);
    }

    pub fn set_mute(&self, muted: bool) {
        self.inner.shared.set_muted(muted);
    }

    // -- D6: the web player's sound -------------------------------------------
    //
    // Every setter is a plain atomic store (rule 4: a click answers in the same
    // frame); the callback picks the value up at the top of its next buffer.

    /// Réglages → Lecture's crossfade block (`fade.enabled`, `fade.seconds`,
    /// `fade.onSkip`, `fade.skipSeconds`). The app turns it off while an episode
    /// plays: a podcast is never blended into what follows it.
    pub fn set_fade(&self, fade: FadeSettings) {
        self.inner.settings.set_fade(fade);
    }

    pub fn fade(&self) -> FadeSettings {
        self.inner.settings.fade()
    }

    /// `off` / `low` / `medium` / `high` (`stores.js#normalization`).
    pub fn set_normalization(&self, level: Normalization) {
        self.inner.settings.set_normalization(level);
    }

    pub fn normalization(&self) -> Normalization {
        self.inner.settings.normalization()
    }

    /// One of the ten bands of Réglages → Égaliseur, in dB (±16).
    pub fn set_eq(&self, band: usize, db: f32) {
        self.inner.settings.set_eq(band, db);
    }

    pub fn eq(&self, band: usize) -> f64 {
        self.inner.settings.eq(band)
    }

    /// The bass lift, 0…+16 dB (its compressor follows it, `graph.js`).
    pub fn set_bass(&self, db: f32) {
        self.inner.settings.set_bass(db)
    }

    pub fn bass(&self) -> f64 {
        self.inner.settings.bass()
    }

    /// The mono fold, for the ears only: the analysis tap stays stereo.
    pub fn set_mono(&self, mono: bool) {
        self.inner.settings.set_mono(mono);
    }

    pub fn mono(&self) -> bool {
        self.inner.settings.mono()
    }

    /// −1 hard left … 0 centre … 1 hard right.
    pub fn set_balance(&self, balance: f32) {
        self.inner.settings.set_balance(balance);
    }

    pub fn balance(&self) -> f64 {
        self.inner.settings.balance()
    }

    /// The podcast speed, 0.5–3×, pitch kept. It applies to an episode only
    /// ([`TrackMeta::podcast`]); music always plays at 1× whatever this says.
    pub fn set_speed(&self, speed: f32) {
        self.inner.settings.set_speed(speed);
    }

    pub fn speed(&self) -> f32 {
        self.inner.settings.speed()
    }

    /// What the app knows about a track: its loudness, its measured bounds, and
    /// whether it is an episode. Called before `load`/`preload_next`, and again
    /// when an answer arrives late — the track playing is then trimmed from
    /// where its audio really ends (`Player.svelte#audioEndsAt` reads live too).
    pub fn set_track_meta(&self, id: &str, meta: TrackMeta) {
        self.inner.store_meta(id, meta);
        if let Some((current, _)) = self.inner.now_playing()
            && current == id
        {
            let end = trim::trim_for(meta.edges, None, meta.duration_s).end;
            self.inner.send(MixerCmd::SetCurrentEnd(end));
        }
    }

    pub fn track_meta(&self, id: &str) -> TrackMeta {
        self.inner.meta_of(id)
    }

    /// The analysis tap: post-DSP stereo frames, dropped when the reader is
    /// behind. One reader per engine — the app's analysis thread takes it once.
    pub fn tap(&self) -> Option<TapReader> {
        self.inner.tap.lock().expect("tap poisoned").take()
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

    /// Stop every thread this engine started, and WAIT for them.
    ///
    /// The app must call this — through [`EngineHandle::shutdown`] or straight
    /// here — before it drops the tokio runtime its [`StreamOpener`] borrows
    /// with `Handle::block_on`: tokio panics when a timer is polled on a runtime
    /// that is shutting down. Idempotent, and safe to call from any thread; no
    /// audio is heard afterwards.
    pub fn shutdown(&self) {
        self.inner.shutdown();
    }

    /// A handle that survives the app dropping the [`Engine`] itself: the close
    /// path keeps one so it can still stop and join the engine's threads.
    pub fn handle(&self) -> EngineHandle {
        EngineHandle(Arc::downgrade(&self.inner))
    }
}

/// A weak reference to a running engine, for the app's close path.
///
/// It is deliberately WEAK: the app holds it in a process-wide registry (the
/// same place `NetRuntime` lives), and a strong reference there would keep the
/// engine — and its threads — alive for the life of the process.
#[derive(Clone)]
pub struct EngineHandle(std::sync::Weak<Inner>);

impl EngineHandle {
    /// Is the engine still alive? A handle whose engine went away has nothing
    /// left to join.
    pub fn is_alive(&self) -> bool {
        self.0.strong_count() > 0
    }

    /// Stop and join the engine's threads; a no-op if it is already gone.
    pub fn shutdown(&self) {
        if let Some(inner) = self.0.upgrade() {
            inner.shutdown();
        }
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

    /// The app's last word on a track. Bounded like the voice table: a long
    /// session must not grow it forever, and the entry for a track three hours
    /// ago is not one any voice will be built from again.
    fn store_meta(&self, id: &str, meta: TrackMeta) {
        let mut entries = self.meta.lock().expect("track meta poisoned");
        if let Some((_, slot)) = entries.iter_mut().find(|(key, _)| key == id) {
            *slot = meta;
            return;
        }
        entries.push((id.to_string(), meta));
        if entries.len() > 512 {
            let drop_to = entries.len() - 256;
            entries.drain(..drop_to);
        }
    }

    fn meta_of(&self, id: &str) -> TrackMeta {
        self.meta
            .lock()
            .expect("track meta poisoned")
            .iter()
            .rev()
            .find(|(key, _)| key == id)
            .map(|(_, meta)| *meta)
            .unwrap_or_default()
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
        // A thread that returns BECAUSE the engine is closing has nothing to
        // report: the app is going away, and "application fermée" is not a
        // verdict about the track (`docs/agents/desktop.md`, rule 3).
        if self.shut.load(Ordering::SeqCst) {
            return;
        }
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
                    if !self.poll_wait(ready.retry_delay()) {
                        return;
                    }
                }
                Err(e) => {
                    self.fail(Some(id), e.into());
                    return;
                }
            }
        }
    }
    /// The seek policy's own wait: a deferred seek polls `/ready` every 1 s
    /// (1.5 s while an archive is written), but a shutdown must never wait out
    /// that interval — the engine JOINS this thread before the network runtime
    /// it borrows goes away. So the wait is cut into slices that re-read the
    /// stop, and the total stays exactly what `retry_delay` asked for.
    fn poll_wait(&self, total: Duration) -> bool {
        let deadline = Instant::now() + total;
        loop {
            if self.stopped.load(Ordering::Relaxed) {
                return false;
            }
            let now = Instant::now();
            if now >= deadline {
                return true;
            }
            thread::sleep((deadline - now).min(Duration::from_millis(20)));
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Stop the engine's threads and JOIN them.
///
/// This is the rule `docs/agents/desktop.md` (D13, *Pitfalls*) states for every
/// thread that borrows the app's network runtime: the app's `StreamOpener`
/// drives `nz-api` with `Handle::block_on`, and tokio PANICS — it returns no
/// error — when a timer is polled on a runtime that is shutting down. So the
/// engine owns the threads it starts, and the runtime's owner joins them here
/// before it drops the runtime (the app calls [`Engine::shutdown`] from the
/// same place it stops the download worker).
///
/// The order is the contract:
/// 1. `stopped`, so no new loader, spooler or source starts anything;
/// 2. `opener.shutdown()`, so every wait already in flight is abandoned at
///    once instead of finishing a request nobody wants;
/// 3. every voice stopped (the current one, the preloaded next one, a
///    crossfading one), so their decode threads return;
/// 4. the loaders, the decode threads and the spoolers joined — once this
///    returns, no thread of this engine can touch the runtime again.
///
/// Idempotent, and a no-op the second time: `Inner::drop` calls it, and so does
/// the app's close path.
impl Inner {
    fn shutdown(&self) {
        if self.shut.swap(true, Ordering::SeqCst) {
            return;
        }
        self.stopped.store(true, Ordering::SeqCst);
        self.opener.shutdown();
        {
            let mut state = self.state.lock().expect("engine state poisoned");
            if let Some(slot) = state.current.take() {
                slot.status.stop();
            }
        }
        for (status, _) in self.voices.lock().expect("engine voices poisoned").iter() {
            status.stop();
        }
        // The mixer drops its voices (and with them the preloaded next one) on
        // this command; `try` push, because a shutdown may be late enough for
        // the output to have stopped draining and it must not wait 500 ms.
        self.send_now(MixerCmd::Shutdown);

        let loaders: Vec<JoinHandle<()>> = {
            let mut loaders = self.loaders.lock().expect("engine loaders poisoned");
            std::mem::take(&mut *loaders)
        };
        for loader in loaders {
            Inner::join(loader);
        }
        let voices: Vec<(Arc<VoiceStatus>, JoinHandle<()>)> = {
            let mut voices = self.voices.lock().expect("engine voices poisoned");
            std::mem::take(&mut *voices)
        };
        for (status, voice) in voices {
            status.stop();
            Inner::join(voice);
        }
        self.spoolers.join_all();
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

    /// Join a thread this engine started — unless the caller IS that thread.
    ///
    /// A thread that drops the last `Arc<Inner>` (a loader whose engine the app
    /// let go of, with no close path) runs `Inner::drop` on ITSELF, and
    /// `JoinHandle::join` on one's own thread fails with `EDEADLK` — a panic in
    /// a worker, which is exactly what this lot exists to stop. The handle is
    /// dropped instead; that thread is on its way out anyway.
    fn join(handle: JoinHandle<()>) {
        if handle.thread().id() == thread::current().id() {
            return;
        }
        let _ = handle.join();
    }

    /// Start a loader thread and keep its handle: the engine joins it when it
    /// closes, so that no thread is left inside the network runtime
    /// ([`Inner::shutdown`]).
    fn spawn_loader(&self, name: String, task: impl FnOnce() + Send + 'static) {
        let spawned = thread::Builder::new()
            .name(name)
            .stack_size(512 * 1024)
            .spawn(task);
        if let Ok(handle) = spawned {
            let mut loaders = self.loaders.lock().expect("engine loaders poisoned");
            loaders.retain(|loader| !loader.is_finished());
            loaders.push(handle);
        }
    }

    /// Keep a decode thread's handle and stop flag, so [`Inner::shutdown`] can
    /// stop and join it whatever the mixer is doing.
    fn register_voice(&self, status: Arc<VoiceStatus>, handle: JoinHandle<()>) {
        let mut voices = self.voices.lock().expect("engine voices poisoned");
        voices.retain(|(_, voice)| !voice.is_finished());
        voices.push((status, handle));
    }

    /// Queue a mixer command without waiting for room. Only the shutdown uses
    /// it: waiting up to 500 ms for an output that is going away would make a
    /// close slower than the frame it happened in.
    fn send_now(&self, command: MixerCmd) {
        if let Ok(mut mixer) = self.mixer.lock() {
            let _ = mixer.push(command);
        }
    }
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
    // The trim start is the audio's own beginning when the person is not
    // seeking past it (`Player.svelte#loadTrack`: `e.start > 0.25 ? e.start : 0`).
    let meta = inner.meta_of(id);
    let trim = trim::Trim {
        start: meta
            .edges
            .and_then(|edges| edges.accepted(meta.duration_s))
            .map(Edges::start_offset)
            .unwrap_or(0.0),
        end: None,
    };
    let requested = start.unwrap_or(0.0).max(0.0);
    let begin = if requested <= trim::MIN_TRIM_START_S {
        requested.max(trim.start)
    } else {
        requested
    };
    let target = (begin > 0.001).then_some(begin);
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
                if !inner.poll_wait(ready.retry_delay()) {
                    return Err(EngineError::Decode("moteur arrêté".into()));
                }
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
                stop: Arc::clone(&inner.stopped),
                spoolers: inner.spoolers.clone(),
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
    let meta = inner.meta_of(id);
    // The bounds this voice may skip: a trim end only when the file plays past
    // its audio, and never when the measured file is not this track's.
    let trim: Trim = trim::trim_for(meta.edges, duration_s, meta.duration_s);
    let generation = inner.next_gen();
    let (mut consumer, handle) = decode::spawn(
        prepared,
        inner.cfg.sample_rate,
        inner.cfg.ring_seconds,
        generation,
        position_s,
        Arc::clone(&inner.events),
        id.to_string(),
        Arc::clone(&inner.settings),
        meta.podcast,
    )?;
    consumer.set_meta(VoiceMeta {
        end_s: trim.end,
        // The track's own length first: a `?start=` slice is shorter than the
        // track, and the trim's window is measured in the track's media time.
        duration_s: meta.duration_s.or(duration_s),
        loudness: meta.loudness,
        allows_fade: !meta.podcast,
        allows_speed: meta.podcast,
    });
    let status = consumer.status();
    // The engine owns the decode thread: it stops and joins it when it closes,
    // before the runtime the opener borrows goes away.
    inner.register_voice(Arc::clone(&status), handle);
    Ok(Built {
        consumer,
        status,
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
