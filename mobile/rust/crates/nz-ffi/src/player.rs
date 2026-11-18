//! The player's surface: the queue, the engine and every playback rule, behind
//! `PlayerApi` (docs/agents/mobile.md, "MB3b"). Owned by **MB3b**.
//!
//! The types below are the ones the note pins as SHARED — MB3c builds its
//! `NzPlayer` facade from [`PlayerSnapshot`], MB11 reads [`PlayerApi::position`]
//! from the progress bar, MB12 sends [`AudioSettings`] and reads
//! [`OutputStats`]. Every body here converts to `nz_core::player` and back; no
//! rule lives in this file.
//!
//! # Two extensions MB3b makes to MB0's stubs
//!
//! - **[`TrackRef`] carries what the row DRAWS** (title, artists, album, cover,
//!   duration, episode, liked), not only `{key, id}`. A queue row with no title
//!   would be a phantom row on screen, and the phone has no route that turns an
//!   id back into a track — the caller (a list screen, the queue editor) always
//!   has the row it just drew. A bare `{key, id}` stays valid: the row then
//!   shows its id as its title. One `PlayQueue` of 4 000 rows with their titles
//!   is still ONE call (≈ 900 KB), never 4 000.
//! - **[`Normalization`] has the four levels** the chain and Réglages actually
//!   have (`off`/`low`/`medium`/`high`, `graph.js`'s own), not the single
//!   `ReplayGain` the note left open.
//!
//! # What is NOT here: the platform's output
//!
//! On Android the sound leaves through `nz-droid-audio`'s `DroidOutput` (AAudio,
//! or the API-26 `AudioTrack`), which consumes the pull sink
//! `nz_core::player::Player::take_sink` hands out once. That call is MB3a's
//! signature and MB3c's wiring, so this file names neither: a wave-2 build has
//! the queue, the engine and every rule, and no device output — which is what
//! `nz-droid-audio`'s own stub answers today (`Unavailable`).

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

// MB3c: the platform's output (below). `nz-droid-audio` is already a dependency
// of this crate (it keeps the API-26 JNI entry points alive, MB3a); MB3c is what
// finally STARTS it, so the sink `Player::take_sink` hands out has a consumer.
use nz_droid_audio::Output as PlatformOutput;
use nz_droid_audio::{DroidOutput, OutputPrefs, PullSession, output_for};

use nz_core::player as core;
use nz_core::player::{Player, PlayerError, Sink};

use crate::errors::NzError;
use crate::lists::ListHandle;

/// What the transport is doing.
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayState {
    Stopped,
    Buffering,
    Playing,
    Paused,
    Ended,
}

/// Shuffle is a flag; this is where the repeat cycle landed.
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repeat {
    Off,
    One,
    All,
}

/// The quality the person asked for. `Auto` follows the measured ladder.
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    Auto,
    Opus320,
    Opus128,
    Opus64,
    Flac,
}

/// How loudness is normalised (ReplayGain). The four levels are the chain's and
/// Réglages' own (`graph.js`).
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Normalization {
    Off,
    Low,
    Medium,
    High,
}

/// `minutes` or the end of the current track (`webapp/src/lib/sleep.js`).
#[derive(uniffi::Enum, Debug, Clone, PartialEq, Eq)]
pub enum SleepTimer {
    Minutes { minutes: u32 },
    EndOfTrack,
}

/// What the sleep timer has left, for the button's label.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct SleepState {
    /// Minutes left, rounded up; `0` for "at the end of this track".
    pub minutes: u32,
    pub at_end_of_track: bool,
}

/// A track the caller names in a queue edit: the key the row carries, the id the
/// server knows, and what the row draws.
///
/// One FFI call carries up to 4 000 of these (≈ 300 KB bare, ≈ 900 KB with every
/// title): the queue crosses ONCE, never row by row.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq, Default)]
pub struct TrackRef {
    pub key: String,
    pub id: String,
    pub title: String,
    /// Already joined for display ("A, B").
    pub artists: String,
    pub album: String,
    pub album_id: Option<String>,
    pub artist_id: Option<String>,
    pub cover: Option<String>,
    pub duration_s: u32,
    pub episode: bool,
    pub liked: bool,
}

/// What the queue was started from, so the Flow can refill and the history can
/// name it.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct PlayContext {
    /// `album`, `playlist`, `artist`, `favorites`, `flow`, `radio`, `podcast`…
    pub kind: String,
    pub id: String,
}

/// The current track, as the notification, the sheet and the lock screen draw it.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct NowPlaying {
    pub key: String,
    pub id: String,
    pub title: String,
    pub artists: String,
    pub album: String,
    pub album_id: Option<String>,
    pub artist_id: Option<String>,
    pub cover: Option<String>,
    pub duration_s: f64,
    pub is_episode: bool,
    /// `320`, `128`, `64`, `flac` — the quality actually being fetched.
    pub quality: String,
    /// `deezer` or `local`.
    pub source: String,
    pub liked: bool,
}

/// The whole player, pushed on every CHANGE and never on a timer.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct PlayerSnapshot {
    pub state: PlayState,
    pub current: Option<NowPlaying>,
    pub index: Option<u32>,
    pub len: u32,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub volume: f32,
    pub muted: bool,
    /// A network source is being read: the Wi-Fi lock is held, the Bulk lane
    /// yields.
    pub streaming: bool,
    pub sleep: Option<SleepState>,
    pub error: Option<String>,
    /// The last skippable fault, in French; the list goes on.
    pub fault: Option<String>,
    /// Bumps on every change, so the UI can coalesce two states.
    pub version: u64,
}

/// The HOT read: atomics only, called by the progress bar each visible frame.
#[derive(uniffi::Record, Debug, Clone, Copy, PartialEq)]
pub struct PositionInfo {
    pub position_s: f64,
    pub duration_s: f64,
    pub buffered_s: f64,
    pub playing: bool,
}

/// Everything the audio chain is asked for (desktop D6's set, on the phone).
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct AudioSettings {
    pub quality: Quality,
    pub crossfade_s: f32,
    pub trim_silence: bool,
    pub normalization: Normalization,
    /// The ten peaking bands, in dB (webapp's `AudioEffects`).
    pub eq_db: Vec<f32>,
    pub eq_preset: Option<String>,
    pub bass_db: f32,
    pub mono: bool,
    pub balance: f32,
    pub speed: f32,
    /// The quality ceiling on a metered network.
    pub metered_cap: Quality,
}

/// Where the sound goes, so a direct/processed verdict is per connection
/// (MB12). `conn` changes whenever the chosen device does.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq, Default)]
pub struct AudioRoute {
    /// `bluetooth`, `usb`, `wired`, `hdmi`, `speaker`.
    pub kind: String,
    pub name: String,
    pub conn: String,
}

/// What the audio path did — the numbers Aide and the journal show.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq, Default)]
pub struct OutputStats {
    /// The mixer's starvations plus the device's xruns: glitches the person
    /// heard.
    pub underruns: u64,
    /// The SOURCE starved (a deferred seek, a link that went away) — it needs
    /// the opposite fix from an underrun (android.md, bridge 12).
    pub rebuffers: u64,
    pub buffer_ms: u32,
    pub sample_rate: u32,
    pub latency_ms: u32,
    /// What the platform actually granted (`PowerSaving`, or what it fell back
    /// to); empty until `DroidOutput` is wired (MB3a).
    pub performance_mode: String,
    /// Kotlin's tap instant and the first rendered frame, in nanoseconds: the
    /// difference is "tap → sound", read from events.
    pub last_tap_ns: u64,
    pub last_first_audible_ns: u64,
}

/// A transport or queue command. Writes are optimistic: the core updates its
/// state at once, emits, then sends.
#[derive(uniffi::Enum, Debug, Clone, PartialEq)]
pub enum PlayerCommand {
    Play,
    Pause,
    Toggle,
    Next,
    Prev,
    Seek {
        s: f64,
    },
    Jump {
        index: u32,
        key: String,
    },
    PlayQueue {
        tracks: Vec<TrackRef>,
        start: u32,
        context: Option<PlayContext>,
    },
    /// `album`, `playlist`, `artist`, `favorites`, `flow`, `radio`, `podcast`…
    PlayEntity {
        kind: String,
        id: String,
        shuffle: bool,
    },
    AddToQueue {
        tracks: Vec<TrackRef>,
    },
    PlayNext {
        tracks: Vec<TrackRef>,
    },
    Remove {
        index: u32,
        key: String,
    },
    Move {
        from: u32,
        to: u32,
    },
    ClearUpcoming,
    SetShuffle {
        on: bool,
    },
    CycleRepeat,
    SetVolume {
        v: f32,
    },
    ToggleMute,
    StartRadio {
        kind: String,
        id: String,
    },
    StartFlow {
        style: Option<String>,
    },
}

/// Pushed on a Rust thread: Kotlin hands it to a Flow and returns at once.
#[uniffi::export(callback_interface)]
pub trait PlayerListener: Send + Sync {
    fn on_snapshot(&self, snapshot: PlayerSnapshot);
    /// The queue changed (an edit, a shuffle, an advance): re-window the handle.
    fn on_queue_changed(&self, version: u64);
    /// A title could not be played; `cause` is a French sentence. The list goes
    /// on: a title that cannot be played costs that title, not the list.
    fn on_fault(&self, cause: String);
}

/// The player. Built once, by `NzCore::player()`, over the core that owns the
/// client, the cache and the lists.
#[derive(uniffi::Object)]
pub struct PlayerApi {
    /// `Err` when the runtime or a thread could not be created: every call then
    /// answers an empty value with `error` set, never a panic on a click.
    inner: Result<Arc<Player>, String>,
    core: Arc<nz_core::core::Core>,
    /// The one listener list, so `subscribe` replaces rather than doubles.
    listeners: OnceLock<()>,
    /// The device output, which lives as long as the PROCESS: the engine's sink
    /// is handed out once, and Android recreates a media service whenever it
    /// likes (a swiped task, a low-memory kill of the service only). A second
    /// `start_output` must adopt the running output, not fail — measured: the
    /// recreated service answered "la sortie audio a déjà été prise" and the
    /// engine's clock stayed at 0 while the state said PLAYING.
    output: Mutex<Option<Arc<OutputHandle>>>,
}

impl PlayerApi {
    /// Builds the player over `core`. Called once by `NzCore::player()`.
    #[uniffi::constructor]
    pub fn open(core: Arc<nz_core::core::Core>) -> Arc<PlayerApi> {
        let inner = Player::open(Arc::clone(&core)).map_err(|error: PlayerError| error.to_string());
        Arc::new(PlayerApi {
            inner,
            core,
            listeners: OnceLock::new(),
            output: Mutex::new(None),
        })
    }

    fn player(&self) -> Option<&Arc<Player>> {
        self.inner.as_ref().ok()
    }
}

#[uniffi::export]
impl PlayerApi {
    /// Enqueues a command and returns: the caller (a Compose click handler)
    /// never waits on the network, a lock or the engine.
    pub fn command(&self, cmd: PlayerCommand, tap_ns: u64) {
        let Some(player) = self.player() else {
            return;
        };
        player.command(core_command(cmd), tap_ns);
    }

    /// The whole picture, cheap to copy.
    pub fn snapshot(&self) -> PlayerSnapshot {
        match self.player() {
            Some(player) => snapshot_of(player.snapshot()),
            None => PlayerSnapshot {
                state: PlayState::Stopped,
                current: None,
                index: None,
                len: 0,
                shuffle: false,
                repeat: Repeat::Off,
                volume: 1.0,
                muted: false,
                streaming: false,
                sleep: None,
                error: self.inner.as_ref().err().cloned(),
                fault: None,
                version: 0,
            },
        }
    }

    /// The hot read: atomics only. The progress bar calls this each visible
    /// frame and NOTHING while the screen is off.
    pub fn position(&self) -> PositionInfo {
        match self.player() {
            Some(player) => {
                let position = player.position();
                PositionInfo {
                    position_s: position.position_s,
                    duration_s: position.duration_s,
                    buffered_s: position.buffered_s,
                    playing: position.playing,
                }
            }
            None => PositionInfo {
                position_s: 0.0,
                duration_s: 0.0,
                buffered_s: 0.0,
                playing: false,
            },
        }
    }

    /// The queue as a windowed handle (4 000 entries never cross whole). A
    /// fresh handle per call: the UI re-asks when `on_queue_changed` fires.
    pub fn queue(&self) -> Arc<ListHandle> {
        match self.player() {
            Some(player) => crate::lists::open_rows(&self.core, player.queue_rows()),
            // No player at all (its runtime could not start): the empty handle
            // MB0's stub answered, so a screen draws nothing instead of crashing.
            None => crate::lists::open(Vec::new()),
        }
    }

    /// Where a screen's `LazyColumn` starts when the queue is *already* what it
    /// drew: a cheap read of the same rows.
    pub fn queue_rows(&self) -> Vec<TrackRef> {
        match self.player() {
            Some(player) => player
                .queue_rows()
                .into_iter()
                .map(|row| TrackRef {
                    key: row.key,
                    id: row.id,
                    title: row.title,
                    artists: row.artists,
                    album: row.album,
                    album_id: row.album_id,
                    artist_id: None,
                    cover: row.cover,
                    duration_s: row.duration_s,
                    episode: false,
                    liked: row.liked,
                })
                .collect(),
            None => Vec::new(),
        }
    }

    /// Registers a listener. Rust pushes on CHANGE only; the Kotlin side hands
    /// each callback to a Flow and returns at once.
    pub fn subscribe(&self, listener: Box<dyn PlayerListener>) {
        let Some(player) = self.player() else {
            return;
        };
        let listener: Arc<dyn PlayerListener> = Arc::from(listener);
        player.subscribe(Arc::new(ListenerSink { listener }));
        let _ = self.listeners.set(());
    }

    pub fn audio(&self) -> AudioSettings {
        match self.player() {
            Some(player) => audio_of(player.audio()),
            None => audio_of(core::AudioSettings::default()),
        }
    }

    /// Persists (in the settings store) and applies at once.
    pub fn set_audio(&self, settings: AudioSettings) {
        if let Some(player) = self.player() {
            player.set_audio(core_audio(settings));
        }
    }

    /// The numbers Aide and the journal show. `nz-core` deliberately answers
    /// `buffer_ms: 0` and an empty `performance_mode` ("leaves those two to
    /// nz-ffi, which owns the output on the device", MB3b), and MB3c is what
    /// fills them: they are the HAL's own reads, never the request — AAudio may
    /// grant less than the 500 ms asked for, and `PowerSaving` may be refused.
    pub fn output_stats(&self) -> OutputStats {
        let Some(player) = self.player() else {
            return OutputStats::default();
        };
        let stats = player.output_stats();
        let device = self
            .output
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let (buffer_ms, sample_rate, performance_mode) = match device {
            Some(handle) => (
                handle.buffer_ms(),
                handle.sample_rate(),
                handle.performance_mode(),
            ),
            None => (stats.buffer_ms, stats.sample_rate, stats.performance_mode),
        };
        OutputStats {
            underruns: stats.underruns,
            rebuffers: stats.rebuffers,
            buffer_ms,
            sample_rate,
            latency_ms: stats.latency_ms,
            performance_mode,
            last_tap_ns: stats.last_tap_ns,
            last_first_audible_ns: stats.last_first_audible_ns,
        }
    }

    /// Screen off raises the read-ahead to 32 MB (the radio sleeps between
    /// bursts); screen on brings it back to 2 MB (rule 1).
    pub fn set_screen_on(&self, on: bool) {
        if let Some(player) = self.player() {
            player.set_screen_on(on);
        }
    }

    pub fn set_route(&self, route: AudioRoute) {
        if let Some(player) = self.player() {
            player.set_route(core::AudioRoute {
                kind: route.kind,
                name: route.name,
                conn: route.conn,
            });
        }
    }

    pub fn set_sleep(&self, timer: Option<SleepTimer>) {
        if let Some(player) = self.player() {
            player.set_sleep(timer.map(core_sleep));
        }
    }

    /// The audio focus said "can duck": the ramp goes to 0.2 and back.
    pub fn set_ducked(&self, ducked: bool) {
        if let Some(player) = self.player() {
            player.set_ducked(ducked);
        }
    }

    /// Counted bytes of what the player holds in memory (worker rule 4): a
    /// 4 000-row queue is held to 4 MB, counted, never by the process's RSS.
    pub fn memory_bytes(&self) -> u64 {
        self.player()
            .map(|player| player.memory_bytes())
            .unwrap_or(0)
    }

    /// Stops and joins every thread the player started. Android kills the
    /// process instead, so nothing correct depends on this running.
    pub fn shutdown(&self) {
        if let Some(player) = self.player() {
            player.shutdown();
        }
    }
}

// --- MB3c: the platform's output ---------------------------------------
//
// `nz-core::player::Player` builds its engine with `Engine::new_external`: the
// clock only moves when SOMETHING renders into the sink, and `Player::take_sink`
// hands that sink out exactly once. On Android the renderer is
// `nz-droid-audio` — AAudio on API >= 27, MB3a's Kotlin `AudioTrackPull` on 26 —
// and until MB3c nothing called it, so a "play" moved the machine's flag and not
// the engine's clock. `start_output` is that wiring, and it is the ONLY thing
// this block does: no rule, no state, no queue.
//
// It answers a `Result` rather than an `Option` so a refused device reaches the
// service with the French cause `nz-droid-audio` wrote (a silent `null` would
// leave the person with a player that says "Lecture" over no sound).

/// What is really rendering, once `PlayerApi::start_output` has run.
enum OutputDevice {
    /// API >= 27: AAudio owns the real-time thread and re-opens it by itself.
    Aa(Box<DroidOutput>),
    /// API 26: `AudioTrackPull.kt` pulls with `handle()`; the device's own
    /// numbers arrive through its `nativeUnderruns` (MB3a).
    Track,
}

/// The device output the media service holds for as long as it plays.
///
/// Started once per process (the sink is taken once), stopped when the service
/// goes away. Every getter is a cheap read of `nz-droid-audio`'s published
/// state: the service calls `underruns()` and `opens()` from its state loop and
/// never blocks on a lock the real-time thread could hold.
#[derive(uniffi::Object)]
pub struct OutputHandle {
    device: Mutex<Option<OutputDevice>>,
    prefs: OutputPrefs,
    /// The `Box<PullSession>` the API-26 path turned into a `long`; `0` on the
    /// AAudio path.
    handle: AtomicI64,
    track: bool,
}

impl OutputHandle {
    fn new(device: OutputDevice, prefs: OutputPrefs, handle: i64, track: bool) -> Self {
        Self {
            device: Mutex::new(Some(device)),
            prefs,
            handle: AtomicI64::new(handle),
            track,
        }
    }

    /// The AAudio output, when this handle has one.
    fn with_aa<T>(&self, read: impl FnOnce(&DroidOutput) -> T) -> Option<T> {
        let device = self
            .device
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match device.as_ref() {
            Some(OutputDevice::Aa(output)) => Some(read(output)),
            _ => None,
        }
    }
}

#[uniffi::export]
impl OutputHandle {
    /// `true` on API 26: the caller builds an `AudioTrackPull` with `handle()`
    /// and drives the device itself (AAudio can crash after close on Android
    /// 8.0, Oboe #40).
    pub fn needs_audio_track(&self) -> bool {
        self.track
    }

    /// The `Box<PullSession>` for `AudioTrackPull`, `0` on the AAudio path or
    /// after `stop()`. **The pull OWNS it**: its `close()` calls `nativeClose`,
    /// which is the only free — this handle never frees it a second time.
    pub fn handle(&self) -> i64 {
        self.handle.load(Ordering::Acquire)
    }

    /// The rate the device really opened at (the voices are decoded at the
    /// engine's rate, so this is `EngineConfig::sample_rate` on both paths).
    pub fn sample_rate(&self) -> u32 {
        self.with_aa(|output| output.info().sample_rate)
            .unwrap_or(self.prefs.sample_rate)
    }

    /// The buffer the device really has, in milliseconds — never the request
    /// (AAudio's capacity may cap it, MB3a).
    pub fn buffer_ms(&self) -> u32 {
        self.with_aa(DroidOutput::buffer_ms)
            .unwrap_or(self.prefs.buffer_ms_min)
    }

    /// What the platform actually granted: `power-saving`, `low-latency`,
    /// `none` (refused), `audio-track` on the API-26 path. Réglages shows this
    /// number, so it is read back and never assumed.
    pub fn performance_mode(&self) -> String {
        self.with_aa(|output| output.performance_mode().to_string())
            .unwrap_or_else(|| "audio-track".to_string())
    }

    /// The DEVICE's own xruns (AAudio's `getXRunCount` total; the API-26 pull
    /// reports its total straight into the engine instead). The person's number
    /// is `PlayerApi::output_stats().underruns`, which adds the mixer's own
    /// starvations.
    pub fn underruns(&self) -> u64 {
        self.with_aa(DroidOutput::underruns).unwrap_or(0)
    }

    /// How many streams this output has opened: `1` after `start`, `2` after a
    /// route change. The one count a reopen is proven by.
    pub fn opens(&self) -> u64 {
        self.with_aa(DroidOutput::opens).unwrap_or(0)
    }

    /// How many re-opens failed, and why — the cause a failed route change
    /// leaves behind.
    pub fn failures(&self) -> u64 {
        self.with_aa(DroidOutput::failures).unwrap_or(0)
    }

    pub fn last_failure(&self) -> Option<String> {
        self.with_aa(|output| output.last_failure()).flatten()
    }

    /// Close the stream and open a new one on the device that is there now.
    /// **Returns once the new stream plays** (or stopped failing), so a route
    /// change is an event, never a guess. `false` on the API-26 path, where the
    /// pull's own `AudioDeviceCallback` re-opens its `AudioTrack`.
    pub fn reopen(&self) -> bool {
        self.with_aa(|output| output.reopen().is_ok())
            .unwrap_or(false)
    }

    /// Close the device and JOIN its thread (AAudio's reopen thread borrows the
    /// engine, worker rule 5). Idempotent. On the API-26 path the pull's
    /// `close()` is the owner, so this only forgets the handle.
    pub fn stop(&self) {
        let device = self
            .device
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(OutputDevice::Aa(output)) = device {
            output.stop();
        }
        self.handle.store(0, Ordering::Release);
    }
}

#[uniffi::export]
impl PlayerApi {
    /// Starts the platform's output, ONCE: the media service calls this when
    /// the session is created and `stop()`s it when the service goes away.
    ///
    /// `sdk_int` is `Build.VERSION.SDK_INT`; `nz-droid-audio::output_for` is the
    /// only place the AAudio/AudioTrack split is decided, so a caller cannot
    /// pick AAudio on Android 8.0 by accident. A host build has no device at
    /// all: it answers an error on the AAudio path (the tests use that), while
    /// the API-26 path only builds the pull session and needs no device.
    ///
    /// The error is `Internal`: it is the device's own French cause, not
    /// something the person did.
    pub fn start_output(&self, sdk_int: u32) -> Result<Arc<OutputHandle>, NzError> {
        let mut started = self
            .output
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // The output belongs to the process, not to the service that first asked
        // for it: Android recreates a media service at will, and the second one
        // must get the SAME stream (and its Kotlin pull) instead of a refusal.
        if let Some(handle) = started.as_ref() {
            return Ok(Arc::clone(handle));
        }
        let player = self.player().ok_or_else(|| NzError::Internal {
            detail: "le lecteur n'a pas démarré : aucune sortie audio".into(),
        })?;
        // `take_sink` is the one-shot hand-over (MB3b): a second call is a bug,
        // and it says so instead of rendering silence for ever.
        let (engine, sink) = player.take_sink().ok_or_else(|| NzError::Internal {
            detail: "la sortie audio a déjà été prise".into(),
        })?;
        let prefs = OutputPrefs::default();
        match output_for(sdk_int) {
            PlatformOutput::Aa => {
                let output =
                    DroidOutput::start(&engine, sink, prefs, sdk_int).map_err(|error| {
                        NzError::Internal {
                            detail: error.to_string(),
                        }
                    })?;
                let handle = Arc::new(OutputHandle::new(
                    OutputDevice::Aa(Box::new(output)),
                    prefs,
                    0,
                    false,
                ));
                *started = Some(Arc::clone(&handle));
                Ok(handle)
            }
            PlatformOutput::Track => {
                let session = PullSession::new(&engine, sink, prefs).into_handle();
                let handle = Arc::new(OutputHandle::new(OutputDevice::Track, prefs, session, true));
                *started = Some(Arc::clone(&handle));
                Ok(handle)
            }
            PlatformOutput::Unsupported => Err(NzError::Internal {
                detail: format!("l'API {sdk_int} n'a pas de sortie audio (minSdk 26)"),
            }),
        }
    }
}

/// The bridge from `nz-core`'s sink trait to UniFFI's callback interface.
struct ListenerSink {
    listener: Arc<dyn PlayerListener>,
}

impl Sink for ListenerSink {
    fn on_snapshot(&self, snapshot: core::Snapshot) {
        self.listener.on_snapshot(snapshot_of(snapshot));
    }

    fn on_queue_changed(&self, version: u64) {
        self.listener.on_queue_changed(version);
    }

    fn on_fault(&self, cause: String) {
        self.listener.on_fault(cause);
    }
}

// --- the conversions, in both directions -------------------------------

fn core_command(cmd: PlayerCommand) -> core::Command {
    match cmd {
        PlayerCommand::Play => core::Command::Play,
        PlayerCommand::Pause => core::Command::Pause,
        PlayerCommand::Toggle => core::Command::Toggle,
        PlayerCommand::Next => core::Command::Next,
        PlayerCommand::Prev => core::Command::Prev,
        PlayerCommand::Seek { s } => core::Command::Seek { s },
        PlayerCommand::Jump { index, key } => core::Command::Jump { index, key },
        PlayerCommand::PlayQueue {
            tracks,
            start,
            context,
        } => core::Command::PlayQueue {
            tracks: tracks.into_iter().map(core_track).collect(),
            start,
            context: context.map(|context| core::Context {
                kind: context.kind,
                id: Some(context.id).filter(|id| !id.is_empty()),
                title: None,
            }),
        },
        PlayerCommand::PlayEntity { kind, id, shuffle } => {
            core::Command::PlayEntity { kind, id, shuffle }
        }
        PlayerCommand::AddToQueue { tracks } => core::Command::AddToQueue {
            tracks: tracks.into_iter().map(core_track).collect(),
        },
        PlayerCommand::PlayNext { tracks } => core::Command::PlayNext {
            tracks: tracks.into_iter().map(core_track).collect(),
        },
        PlayerCommand::Remove { index, key } => core::Command::Remove { index, key },
        PlayerCommand::Move { from, to } => core::Command::Move { from, to },
        PlayerCommand::ClearUpcoming => core::Command::ClearUpcoming,
        PlayerCommand::SetShuffle { on } => core::Command::SetShuffle { on },
        PlayerCommand::CycleRepeat => core::Command::CycleRepeat,
        PlayerCommand::SetVolume { v } => core::Command::SetVolume { v },
        PlayerCommand::ToggleMute => core::Command::ToggleMute,
        PlayerCommand::StartRadio { kind, id } => core::Command::StartRadio { kind, id },
        PlayerCommand::StartFlow { style } => core::Command::StartFlow { style },
    }
}

fn core_track(track: TrackRef) -> core::TrackRef {
    core::TrackRef {
        key: track.key,
        id: track.id,
        title: track.title,
        artists: track.artists,
        album: track.album,
        album_id: track.album_id,
        artist_id: track.artist_id,
        cover: track.cover,
        duration_s: track.duration_s,
        episode: track.episode,
        liked: track.liked,
    }
}

#[cfg(test)]
fn track_ref(track: core::TrackRef) -> TrackRef {
    TrackRef {
        key: track.key,
        id: track.id,
        title: track.title,
        artists: track.artists,
        album: track.album,
        album_id: track.album_id,
        artist_id: track.artist_id,
        cover: track.cover,
        duration_s: track.duration_s,
        episode: track.episode,
        liked: track.liked,
    }
}

fn core_sleep(timer: SleepTimer) -> core::SleepTimer {
    match timer {
        SleepTimer::Minutes { minutes } => core::SleepTimer::Minutes { minutes },
        SleepTimer::EndOfTrack => core::SleepTimer::EndOfTrack,
    }
}

fn snapshot_of(snapshot: core::Snapshot) -> PlayerSnapshot {
    PlayerSnapshot {
        state: state_of(snapshot.state),
        current: snapshot.current.map(|now| NowPlaying {
            key: now.key,
            id: now.id,
            title: now.title,
            artists: now.artists,
            album: now.album,
            album_id: now.album_id,
            artist_id: now.artist_id,
            cover: now.cover,
            duration_s: now.duration_s,
            is_episode: now.is_episode,
            quality: now.quality,
            source: now.source,
            liked: now.liked,
        }),
        index: snapshot.index,
        len: snapshot.len,
        shuffle: snapshot.shuffle,
        repeat: repeat_of(snapshot.repeat),
        volume: snapshot.volume,
        muted: snapshot.muted,
        streaming: snapshot.streaming,
        sleep: snapshot.sleep.map(|sleep| SleepState {
            minutes: sleep.minutes,
            at_end_of_track: sleep.at_end_of_track,
        }),
        error: snapshot.error,
        fault: snapshot.fault,
        version: snapshot.version,
    }
}

fn state_of(state: core::PlayState) -> PlayState {
    match state {
        core::PlayState::Stopped => PlayState::Stopped,
        core::PlayState::Buffering => PlayState::Buffering,
        core::PlayState::Playing => PlayState::Playing,
        core::PlayState::Paused => PlayState::Paused,
        core::PlayState::Ended => PlayState::Ended,
    }
}

fn repeat_of(repeat: core::Repeat) -> Repeat {
    match repeat {
        core::Repeat::Off => Repeat::Off,
        core::Repeat::One => Repeat::One,
        core::Repeat::All => Repeat::All,
    }
}

fn quality_of(quality: core::Quality) -> Quality {
    match quality {
        core::Quality::Auto => Quality::Auto,
        core::Quality::Opus320 => Quality::Opus320,
        core::Quality::Opus128 => Quality::Opus128,
        core::Quality::Opus64 => Quality::Opus64,
        core::Quality::Flac => Quality::Flac,
    }
}

fn core_quality(quality: Quality) -> core::Quality {
    match quality {
        Quality::Auto => core::Quality::Auto,
        Quality::Opus320 => core::Quality::Opus320,
        Quality::Opus128 => core::Quality::Opus128,
        Quality::Opus64 => core::Quality::Opus64,
        Quality::Flac => core::Quality::Flac,
    }
}

fn normalization_of(level: core::Normalization) -> Normalization {
    match level {
        core::Normalization::Off => Normalization::Off,
        core::Normalization::Low => Normalization::Low,
        core::Normalization::Medium => Normalization::Medium,
        core::Normalization::High => Normalization::High,
    }
}

fn core_normalization(level: Normalization) -> core::Normalization {
    match level {
        Normalization::Off => core::Normalization::Off,
        Normalization::Low => core::Normalization::Low,
        Normalization::Medium => core::Normalization::Medium,
        Normalization::High => core::Normalization::High,
    }
}

fn audio_of(audio: core::AudioSettings) -> AudioSettings {
    AudioSettings {
        quality: quality_of(audio.quality),
        crossfade_s: audio.crossfade_s,
        trim_silence: audio.trim_silence,
        normalization: normalization_of(audio.normalization),
        eq_db: audio.eq_db,
        eq_preset: audio.eq_preset,
        bass_db: audio.bass_db,
        mono: audio.mono,
        balance: audio.balance,
        speed: audio.speed,
        metered_cap: quality_of(audio.metered_cap),
    }
}

fn core_audio(audio: AudioSettings) -> core::AudioSettings {
    core::AudioSettings {
        quality: core_quality(audio.quality),
        crossfade_s: audio.crossfade_s,
        trim_silence: audio.trim_silence,
        normalization: core_normalization(audio.normalization),
        eq_db: audio.eq_db,
        eq_preset: audio.eq_preset,
        bass_db: audio.bass_db,
        mono: audio.mono,
        balance: audio.balance,
        speed: audio.speed,
        metered_cap: core_quality(audio.metered_cap),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- the bench: a tone over loopback, and the sink's own samples --------

    /// The tone this bench plays: MB3a's own FLAC fixture (440 Hz), because
    /// symphonia is built WITHOUT the `wav` feature (`desktop/Cargo.toml`: ogg,
    /// flac, mp3, isomp4, aac) — a WAV body fails the probe with "no suitable
    /// format reader found", which is what the first version of this test
    /// measured.
    fn tone_flac() -> Vec<u8> {
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../nz-droid-audio/tests/assets/sine.flac"
        ))
        .expect("the FLAC fixture")
    }

    /// The routes the player's opener uses, and nothing else.
    struct ToneServer {
        base: String,
        stop: Arc<std::sync::atomic::AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl ToneServer {
        fn start(flac: Vec<u8>) -> Self {
            use std::io::Write as _;
            use std::net::TcpListener;
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind the bench");
            let base = format!(
                "http://{}",
                listener.local_addr().expect("the bench's address")
            );
            let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let stopping = Arc::clone(&stop);
            let thread = std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if stopping.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(mut stream) = stream else { continue };
                    let flac = flac.clone();
                    std::thread::spawn(move || {
                        use std::io::{BufRead, BufReader, Read};
                        let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                        let mut request = String::new();
                        let _ = reader.read_line(&mut request);
                        loop {
                            let mut line = String::new();
                            if reader.read_line(&mut line).unwrap_or(0) == 0
                                || line.trim().is_empty()
                            {
                                break;
                            }
                        }
                        let target = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
                        let (code, kind, body) = if target.starts_with("/api/version") {
                            (200, "application/json", VERSION_BODY.as_bytes().to_vec())
                        } else if target.contains("/ready") {
                            (200, "application/json", b"{\"ready\":true}".to_vec())
                        } else if target.starts_with("/api/stream/") {
                            (200, "audio/flac", flac)
                        } else {
                            (200, "application/json", b"{}".to_vec())
                        };
                        let head = format!(
                            "HTTP/1.1 {code} X\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(head.as_bytes());
                        let _ = stream.write_all(&body);
                        let _ = stream.flush();
                        let _ = reader.read(&mut [0u8; 0]);
                    });
                }
            });
            Self {
                base,
                stop,
                thread: Some(thread),
            }
        }
    }

    impl Drop for ToneServer {
        fn drop(&mut self) {
            use std::net::TcpStream;
            self.stop.store(true, Ordering::SeqCst);
            let _ = TcpStream::connect(self.base.trim_start_matches("http://"));
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    const VERSION_BODY: &str = r#"{"version":"9.9.9","build":"bench"}"#;

    /// A real core over throwaway directories: the player only exists over one,
    /// and `start_output` needs its cache for the engine's spool.
    fn test_core(tag: &str) -> Arc<crate::core::NzCore> {
        let path = std::env::temp_dir().join(format!(
            "nz-ffi-output-{}-{tag}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("data")).expect("data");
        std::fs::create_dir_all(path.join("cache")).expect("cache");
        crate::core::NzCore::new(crate::core::CoreConfig {
            data_dir: path.join("data").display().to_string(),
            cache_dir: path.join("cache").display().to_string(),
            app_version: "0.0.0".into(),
            version_code: 2_000_001,
            sdk_int: 35,
            device_name: "test".into(),
        })
        .expect("the core")
    }

    /// A host has no AAudio at all, the failure names the driver, and the sink
    /// is gone after a failed attempt (nothing was started to adopt).
    #[test]
    fn a_host_has_no_aaudio_and_a_failed_start_keeps_nothing() {
        let core = test_core("aa");
        let player = core.player();
        let error = player
            .start_output(35)
            .err()
            .expect("AAudio does not exist on a host");
        assert!(
            error.to_string().contains("AAudio"),
            "the driver's own cause: {error}"
        );
        let again = player
            .start_output(35)
            .err()
            .expect("the sink is gone for good");
        assert!(
            again.to_string().contains("déjà été prise"),
            "a failed open consumed the sink: {again}"
        );
        core.shutdown();
    }

    /// The API-26 path needs NO device: it builds the pull session the Kotlin
    /// thread drives, at the engine's rate and the 500 ms buffer MB3a measured,
    /// and the handle is what `AudioTrackPull` renders into.
    #[test]
    fn the_api_26_pull_session_crosses_as_a_handle() {
        let core = test_core("track");
        let player = core.player();
        let output = player.start_output(26).expect("the pull session");
        assert!(output.needs_audio_track());
        let handle = output.handle();
        assert_ne!(handle, 0, "a live Box<PullSession>");
        assert_eq!(output.sample_rate(), 48_000);
        assert_eq!(output.buffer_ms(), 500);
        assert_eq!(output.performance_mode(), "audio-track");
        assert_eq!(output.opens(), 0, "no AAudio stream on this path");
        assert!(!output.reopen(), "the pull re-opens its own AudioTrack");
        // The Kotlin pull's `close()` is the free (`nativeClose`, MB3a); the
        // test plays that role, so the session does not outlive it.
        // SAFETY: the handle came from `into_handle` and is closed once.
        unsafe { nz_droid_audio::close_handle(handle) };
        output.stop();
        assert_eq!(output.handle(), 0);
        output.stop(); // idempotent
        // The output belongs to the PROCESS: a recreated media service adopts it
        // instead of being refused (which is what the device measured before
        // this: a service recreated by Android got "déjà été prise" and the
        // engine's clock stayed at 0 while the state said PLAYING).
        let adopted = player.start_output(26).expect("the same output");
        assert!(
            Arc::ptr_eq(&adopted, &output),
            "the SAME stream, not a new one"
        );
        core.shutdown();
    }

    /// **What the phone renders is not silence.** The same player, the same tone
    /// the emulator's bench serves, drained through the very sink the AAudio
    /// callback writes: the RMS of the rendered frames, in dBFS.
    ///
    /// Why here and not on the device: on Android the samples go straight from
    /// `ExternalSink::render` into AAudio's real-time callback (API >= 27) or into
    /// `AudioTrack` (26), and nothing hands them back to Kotlin — `AudioTrackPull`
    /// owns its own loop and its natives are private. The DEVICE proves the other
    /// half in `NzMediaServiceTest`: the stream opened at 48 kHz with a 500 ms
    /// buffer, its clock advances at realtime, and it reports 0 underruns. A mixer
    /// that rendered silence would still clock, which is why this number has to
    /// come from the buffer itself.
    #[test]
    fn the_sink_renders_a_signal_and_not_silence() {
        let server = ToneServer::start(tone_flac());
        let core = test_core("rms");
        let api = core.player();
        let runtime = tokio::runtime::Runtime::new().expect("a runtime for the probe");
        runtime
            .block_on(core.set_server(server.base.clone()))
            .expect("the bench answers /api/version");

        let inner = api.player().expect("the player");
        // The sink before the play: the engine renders only when someone drains
        // it, exactly as the platform's callback does.
        let (_engine, mut sink) = inner.take_sink().expect("the sink");
        api.command(
            PlayerCommand::PlayQueue {
                tracks: vec![TrackRef {
                    key: "tone3-1".into(),
                    id: "tone3-1".into(),
                    title: "Titre".into(),
                    duration_s: 3,
                    ..Default::default()
                }],
                start: 0,
                context: None,
            },
            0,
        );
        api.command(PlayerCommand::Play, 0);

        let mut buffer = vec![0f32; 960]; // 10 ms at 48 kHz
        // Only the AUDIBLE part: the first buffers are the silence before the
        // voice is installed, and averaging them in would dilute the number.
        let mut body: Vec<f32> = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while body.len() < 48_000 && std::time::Instant::now() < deadline {
            sink.render(&mut buffer);
            if body.is_empty() {
                if buffer.iter().any(|sample| sample.abs() > 0.01) {
                    body.extend_from_slice(&buffer);
                }
            } else {
                body.extend_from_slice(&buffer);
            }
            // The decoder threads fill the ring in the background: a tight loop
            // would drain it faster than it fills and measure starvations.
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let snap = api.snapshot();
        assert!(
            !body.is_empty(),
            "the sink rendered nothing at all (state {:?}, error {:?}, len {})",
            snap.state,
            snap.error,
            snap.len
        );
        let rms = (body
            .iter()
            .map(|sample| f64::from(*sample) * f64::from(*sample))
            .sum::<f64>()
            / body.len() as f64)
            .sqrt();
        let dbfs = 20.0 * rms.log10();
        println!(
            "MB3C-RMS samples={} rms={rms:.4} dbfs={dbfs:.2}",
            body.len()
        );
        assert!(
            rms > 0.2,
            "the sink rendered {rms:.4} RMS: silence, or the wrong signal"
        );
        assert!(
            dbfs > -20.0,
            "the rendered signal is {dbfs:.2} dBFS, far below a half-scale tone"
        );
        api.shutdown();
        core.shutdown();
    }

    /// Every transport command crosses the boundary unchanged: a slide of
    /// controls that reaches a different variant is the bug this catches.
    #[test]
    fn every_command_survives_the_crossing() {
        let commands = vec![
            PlayerCommand::Play,
            PlayerCommand::Pause,
            PlayerCommand::Toggle,
            PlayerCommand::Next,
            PlayerCommand::Prev,
            PlayerCommand::Seek { s: 42.5 },
            PlayerCommand::Jump {
                index: 3,
                key: "k".into(),
            },
            PlayerCommand::PlayQueue {
                tracks: vec![TrackRef {
                    key: "k".into(),
                    id: "id".into(),
                    title: "T".into(),
                    ..Default::default()
                }],
                start: 0,
                context: Some(PlayContext {
                    kind: "album".into(),
                    id: "a1".into(),
                }),
            },
            PlayerCommand::PlayEntity {
                kind: "album".into(),
                id: "a1".into(),
                shuffle: false,
            },
            PlayerCommand::AddToQueue { tracks: vec![] },
            PlayerCommand::PlayNext { tracks: vec![] },
            PlayerCommand::Remove {
                index: 1,
                key: "k".into(),
            },
            PlayerCommand::Move { from: 0, to: 2 },
            PlayerCommand::ClearUpcoming,
            PlayerCommand::SetShuffle { on: true },
            PlayerCommand::CycleRepeat,
            PlayerCommand::SetVolume { v: 0.3 },
            PlayerCommand::ToggleMute,
            PlayerCommand::StartRadio {
                kind: "track".into(),
                id: "r".into(),
            },
            PlayerCommand::StartFlow {
                style: Some("Rap".into()),
            },
        ];
        for command in commands {
            let core = core_command(command.clone());
            let back = match core {
                core::Command::Play => PlayerCommand::Play,
                core::Command::Pause => PlayerCommand::Pause,
                core::Command::Toggle => PlayerCommand::Toggle,
                core::Command::Next => PlayerCommand::Next,
                core::Command::Prev => PlayerCommand::Prev,
                core::Command::Seek { s } => PlayerCommand::Seek { s },
                core::Command::Jump { index, key } => PlayerCommand::Jump { index, key },
                core::Command::PlayEntity { kind, id, shuffle } => {
                    PlayerCommand::PlayEntity { kind, id, shuffle }
                }

                core::Command::AddToQueue { tracks } => PlayerCommand::AddToQueue {
                    tracks: tracks.into_iter().map(track_ref).collect(),
                },
                core::Command::PlayNext { tracks } => PlayerCommand::PlayNext {
                    tracks: tracks.into_iter().map(track_ref).collect(),
                },
                core::Command::Remove { index, key } => PlayerCommand::Remove { index, key },
                core::Command::Move { from, to } => PlayerCommand::Move { from, to },
                core::Command::ClearUpcoming => PlayerCommand::ClearUpcoming,
                core::Command::SetShuffle { on } => PlayerCommand::SetShuffle { on },
                core::Command::StartRadio { kind, id } => PlayerCommand::StartRadio { kind, id },
                core::Command::StartFlow { style } => PlayerCommand::StartFlow { style },
                core::Command::CycleRepeat => PlayerCommand::CycleRepeat,
                core::Command::SetVolume { v } => PlayerCommand::SetVolume { v },
                core::Command::ToggleMute => PlayerCommand::ToggleMute,
                core::Command::PlayQueue {
                    tracks,
                    start,
                    context,
                } => PlayerCommand::PlayQueue {
                    tracks: tracks.into_iter().map(track_ref).collect(),
                    start,
                    context: context.map(|context| PlayContext {
                        kind: context.kind,
                        id: context.id.unwrap_or_default(),
                    }),
                },
            };
            assert_eq!(back, command);
        }
    }

    /// The audio settings round-trip, all four normalisation levels included.
    #[test]
    fn the_audio_settings_round_trip() {
        for level in [
            Normalization::Off,
            Normalization::Low,
            Normalization::Medium,
            Normalization::High,
        ] {
            let settings = AudioSettings {
                quality: Quality::Auto,
                crossfade_s: 3.0,
                trim_silence: true,
                normalization: level,
                eq_db: vec![0.0; 10],
                eq_preset: Some("Plat".into()),
                bass_db: 2.0,
                mono: false,
                balance: -0.5,
                speed: 1.2,
                metered_cap: Quality::Opus64,
            };
            assert_eq!(audio_of(core_audio(settings.clone())), settings);
        }
    }
}
