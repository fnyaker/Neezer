//! The player: the queue, the engine and every playback rule of the SPA and of
//! the old native player, in Rust, behind one object.
//!
//! Owned by **MB3b** (`docs/agents/mobile.md`, "MB3b"). The rules it ports, and
//! where each one comes from:
//!
//! | Behaviour | Its source |
//! |---|---|
//! | the queue, shuffle, repeat, the listen ledger, the effects | `nz-player` (desktop D4) — the SAME state machine the desktop uses |
//! | gapless, crossfade, trim, speed, EQ, the clock | `nz-audio` (desktop D5/D6) through an [`ExternalSink`] |
//! | the source order download → play cache → network, the lanes | [`source`] (`docs/agents/mobile.md`, *Rule 1*) |
//! | the Auto quality ladder | [`quality`] (`docs/agents/desktop.md`, *Streams*) |
//! | the fault policy (2/5/10 s, 4xx skips, three at most) | [`fault`], ported from `NativeRouting.fault` |
//! | listens and an episode's position | [`listen`] (`Player.svelte#flushListen`, `podcastProgress.js`) |
//! | the Flow, the radios, `PlayEntity` | [`flow`] (`lib/flow.js`, `lib/api.js#radioNext`) |
//! | the sleep timer | [`sleep`] (`lib/sleep.js`) |
//! | the queue restored PAUSED at its position | [`persist`] (`nz-cache`'s `Queue`) |
//!
//! # The shape: one lock for a command, one thread for everything else
//!
//! [`Player::command`] takes the machine's lock **exactly once**, applies the
//! command to `nz-player`'s state machine, hands the resulting effects to the
//! worker thread and returns. It never waits on the engine, the network or a
//! listener — that is what "a transport command never blocks its caller" means,
//! and what the acceptance counts (`Player::locks_taken`).
//!
//! Everything that could wait happens on `nz-player`'s own thread, which
//! [`Player::shutdown`] joins: opening a stream, priming the gains and the
//! silence bounds, POSTing a listen, asking the Flow for the next title, writing
//! the queue to SQLite. The engine's events come back through the same channel,
//! so the queue advances on ONE thread and never races a command.
//!
//! | thread | owns | joined by |
//! |---|---|---|
//! | `nz-player` | the effects, the reports, the priming | [`Player::shutdown`] |
//! | `nz-player-events` | the engine's event receiver | [`Player::shutdown`] |
//! | `nz-player-net` (tokio, 2 workers) | every request the engine and the worker make | its `Runtime` is dropped only after both above are joined |
//! | the engine's own (`nz-load-*`, `nz-decode-*`, `nz-audio-spool`) | decode and spool | `Engine::shutdown`, called by [`Player::shutdown`] |
//!
//! Tokio panics — it does not return an error — when a timer is polled on a
//! runtime that is shutting down, so that order is a lifetime, not a detail.
//!
//! # Hot reads
//!
//! [`Player::position`] reads atomics only (`nz-audio`'s clock, the duration the
//! last `Started` carried, the buffered mark). The full-screen player's progress
//! bar calls it every visible frame and NOTHING while the screen is off: Rust
//! pushes [`Snapshot`]s on CHANGE, never on a timer (the old app's 4 Hz tick into
//! a sleeping page queued 115 000 states; there is no page here, and still no
//! tick).

mod fault;
mod flow;
mod listen;
mod persist;
mod quality;
mod sleep;
mod source;

#[cfg(test)]
mod tests;

pub use fault::{FAULT_SKIPS_MAX, Faults};
pub use flow::{Fill, Filler};
pub use listen::{EpisodeProgress, Reports};
pub use persist::Store;
pub use quality::{
    AudioSettings, BANDS, Ladder, MAX_CROSSFADE_S, Normalization, Quality, RUNG_128_BYTES_PER_S,
    RUNG_320_BYTES_PER_S, rung_of,
};
pub use sleep::{SLEEP_FADE_MS, Sleep, SleepState, SleepTimer};

pub use source::{Failures, ServerOpener};

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use nz_audio::{Engine, EngineConfig, EngineEvent, ExternalSink, TrackMeta};
// The queue's own vocabulary, re-exported so `nz-ffi` names ONE crate: a
// `Context` or a `Repeat` crossing the boundary is the same type the machine
// applies, never a copy that could drift.
pub use nz_player::{Context, Effect, Entry, PlayerState, Repeat};
use tokio::sync::watch;

use crate::core::{Core, CoreError};

/// The trim threshold the server measures at, which is the SPA's own default
/// (`stores.js#trimThresholdDb`, `edges.py#DEFAULT_THRESHOLD`).
pub const TRIM_DB: f64 = -45.0;
/// The read-ahead a network source may spool while somebody is looking (rule 1:
/// a prefetched next track stops instead of downloading itself whole).
pub const READ_AHEAD_ON_BYTES: u64 = 2 * 1024 * 1024;
/// ...and behind a locked screen, so the radio sleeps between bursts instead of
/// trickling for four minutes (`docs/agents/mobile.md`, *Rule 1*).
pub const READ_AHEAD_OFF_BYTES: u64 = 32 * 1024 * 1024;
/// How many titles ahead the gains and the silence bounds are primed
/// (`Player.svelte`'s `GAIN_WINDOW`).
pub const PRIME_WINDOW: usize = 8;
/// The gain a ducked player drops to (audio focus said "can duck").
pub const DUCK_GAIN: f32 = 0.2;
/// The housekeeping tick, when nothing else is due.
const TICK: Duration = Duration::from_millis(1_000);
/// The tick while the sleep timer fades: 15 s in 150 steps, none audible.
const FADE_TICK: Duration = Duration::from_millis(100);
/// The engine's event thread is woken this often to notice a close.
const EVENT_TICK: Duration = Duration::from_millis(200);
/// The queue is written at most this often while it moves.
const PERSIST_EVERY_MS: u64 = 5_000;

// Locks taken by the CURRENT thread, so `command()`'s count is exact even with
// a busy worker taking its own.
thread_local! {
    static TAKEN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// What the transport is doing. `Buffering` is also the state of a fault the
/// policy is retrying: silence WITH a cause, never a lie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayState {
    Stopped,
    Buffering,
    Playing,
    Paused,
    Ended,
}

/// Where the sound goes (MB12's verdict is per connection).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AudioRoute {
    /// `bluetooth`, `usb`, `wired`, `hdmi`, `speaker`.
    pub kind: String,
    pub name: String,
    /// Changes whenever the chosen device does, so a policy is per connection.
    pub conn: String,
}

/// The queue's identity as the caller names it: the key the row carries, the id
/// the server knows, and — when the caller has them — the fields the row DRAWS.
///
/// The display fields are the extension MB3b makes to MB0's two-field stub: a
/// queue row with no title would be a phantom row, and the phone has no route
/// that turns an id back into a track. A bare `{key, id}` is still valid (a
/// queue arriving from another device, a remote command); the row then shows
/// its id as its title, which is honest and visible.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrackRef {
    pub key: String,
    pub id: String,
    pub title: String,
    pub artists: String,
    pub album: String,
    pub album_id: Option<String>,
    pub artist_id: Option<String>,
    pub cover: Option<String>,
    pub duration_s: u32,
    /// An episode: no crossfade into or out of it, and the podcast speed.
    pub episode: bool,
    pub liked: bool,
}

impl TrackRef {
    /// The id a request uses: the row's id when it is one the server accepts in
    /// a path, else the key.
    fn reference(&self) -> Option<String> {
        if playable(&self.id) {
            Some(self.id.clone())
        } else if playable(&self.key) {
            Some(self.key.clone())
        } else {
            None
        }
    }

    fn entry(&self) -> Option<Entry> {
        let reference = self.reference()?;
        Some(Entry {
            id: reference.clone(),
            deezer_id: (self.id != reference).then(|| self.id.clone()),
            title: if self.title.is_empty() {
                reference
            } else {
                self.title.clone()
            },
            duration: self.duration_s,
            explicit: None,
            local: None,
            podcast: self.episode.then_some(true),
            gain: None,
            unavailable: None,
            archived: None,
            published: None,
            channel_id: None,
            cover: self.cover.clone(),
            display_artist: None,
            artist: self.artist_id.as_ref().map(|id| nz_player::EntryArtist {
                id: Some(id.clone()),
                deezer_id: None,
                name: self.artists.clone(),
                role: None,
            }),
            artists: if self.artists.is_empty() {
                Vec::new()
            } else {
                vec![nz_player::EntryArtist {
                    id: None,
                    deezer_id: None,
                    name: self.artists.clone(),
                    role: None,
                }]
            },
            album: (!self.album.is_empty() || self.album_id.is_some()).then(|| {
                nz_player::EntryAlbum {
                    id: self.album_id.clone(),
                    deezer_id: None,
                    title: self.album.clone(),
                    cover: self.cover.clone(),
                }
            }),
        })
    }
}

/// Whether a string can stand in a URL path segment (`commands.js#playableId`).
fn playable(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// The current track, as the notification, the sheet and the lock screen draw it.
#[derive(Debug, Clone, PartialEq, Default)]
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
    /// `deezer` or `local` — what the source chip says.
    pub source: String,
    pub liked: bool,
}

impl NowPlaying {
    fn from_entry(entry: &Entry, quality: &str, liked: bool) -> NowPlaying {
        let artists = entry
            .artists
            .iter()
            .map(|artist| artist.name.as_str())
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        NowPlaying {
            key: entry.key().to_owned(),
            id: entry.reference().unwrap_or_default().to_owned(),
            title: entry.title.clone(),
            artists: if artists.is_empty() {
                entry.artist_name().to_owned()
            } else {
                artists
            },
            album: entry
                .album
                .as_ref()
                .map(|album| album.title.clone())
                .unwrap_or_default(),
            album_id: entry
                .album
                .as_ref()
                .and_then(|album| album.id.clone().or_else(|| album.deezer_id.clone())),
            artist_id: entry
                .artist
                .as_ref()
                .and_then(|artist| artist.id.clone().or_else(|| artist.deezer_id.clone())),
            cover: entry.cover.clone(),
            duration_s: f64::from(entry.duration),
            is_episode: entry.is_podcast(),
            quality: quality.to_owned(),
            source: if entry.is_local() {
                "local".to_owned()
            } else {
                "deezer".to_owned()
            },
            liked,
        }
    }
}

/// The whole player, pushed on every CHANGE and never on a timer.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub state: PlayState,
    pub current: Option<NowPlaying>,
    pub index: Option<u32>,
    pub len: u32,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub volume: f32,
    pub muted: bool,
    pub streaming: bool,
    pub sleep: Option<SleepState>,
    pub error: Option<String>,
    pub fault: Option<String>,
    pub version: u64,
}

/// The HOT read: atomics only.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Position {
    pub position_s: f64,
    pub duration_s: f64,
    pub buffered_s: f64,
    pub playing: bool,
}

/// A transport or queue command. Writes are optimistic: the state changes at
/// once, the snapshot is emitted, and the worker does the rest.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
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
        context: Option<Context>,
    },
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

impl Command {
    /// Whether the command can change the queue's COMPOSITION (what a screen
    /// rebuilds its windowed handle for). `Play`/`Pause`/`Seek`/volume do not.
    fn touches_queue(&self) -> bool {
        !matches!(
            self,
            Command::Play
                | Command::Pause
                | Command::Toggle
                | Command::Seek { .. }
                | Command::SetVolume { .. }
                | Command::ToggleMute
        )
    }
}

/// Who hears about the player. Implemented by `nz-ffi`'s listener adapter: it
/// runs on the worker thread, never on Kotlin's main thread, and the player
/// holds no lock while calling it.
pub trait Sink: Send + Sync {
    fn on_snapshot(&self, snapshot: Snapshot);
    fn on_queue_changed(&self, version: u64);
    fn on_fault(&self, cause: String);
}

/// Why the player could not start.
#[derive(Debug)]
pub enum PlayerError {
    /// The runtime or a thread could not be created.
    Start(String),
}

impl std::fmt::Display for PlayerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlayerError::Start(cause) => f.write_str(cause),
        }
    }
}

impl std::error::Error for PlayerError {}

/// The player, once per process.
pub struct Player {
    core: Arc<Core>,
    /// The ONE lock [`Player::command`] takes.
    machine: Mutex<Machine>,
    locks: AtomicU64,
    /// How many locks the LAST `command()` took: the acceptance's "1 lock",
    /// measured on the call itself so a busy worker cannot blur the count.
    last_command_locks: AtomicU64,
    jobs: Sender<Job>,
    worker: Mutex<Option<JoinHandle<()>>>,
    events: Mutex<Option<JoinHandle<()>>>,
    /// Dropped LAST: every thread that borrows it is joined first.
    runtime: Mutex<Option<Arc<tokio::runtime::Runtime>>>,
    engine: Engine,
    /// The platform's pull sink, taken once by Android's `DroidOutput` or by a
    /// host test.
    sink: Mutex<Option<ExternalSink>>,
    opener: Arc<ServerOpener>,
    listeners: Mutex<Vec<Arc<dyn Sink>>>,
    /// One delivery at a time, the snapshot taken INSIDE it: the worker (after a
    /// skip) and the platform's callbacks (focus, route, sleep) all push, and a
    /// snapshot taken earlier but delivered later would be the screen's last
    /// word until the next change.
    delivery: Mutex<()>,
    stopped: AtomicBool,
    /// The measured rate of the last streams, in bits of an `f64` (0 = none):
    /// what the Auto ladder reads without a lock.
    rate: Arc<AtomicU64>,
    streaming: Arc<AtomicBool>,
    tap_ns: AtomicU64,
    first_audible_ns: AtomicU64,
    /// The highest voice generation a replaced queue/row dropped. A `Started`
    /// or `Ended` at or below is a voice the stop killed after its event was
    /// already queued, and it must not advance the new queue.
    stop_generation: AtomicU64,
    duration: AtomicU64,
    buffered: AtomicU64,
    playing: AtomicBool,
    rebuffers: AtomicU64,
    /// The last route the platform named (MB12's verdict reads it).
    route: Mutex<AudioRoute>,
    start: Instant,
}

/// The locked state: the machine, its settings and what the snapshot carries.
struct Machine {
    state: PlayerState,
    audio: AudioSettings,
    faults: Faults,
    sleep: Option<Sleep>,
    version: u64,
    error: Option<String>,
    fault: Option<String>,
    /// The queue's composition version (what a windowed handle is rebuilt for).
    queue_version: u64,
    /// Whether the screen is on: it decides the read-ahead, not the sound.
    screen_on: bool,
    /// Audio focus allows a duck (a notification, a navigation prompt).
    ducked: bool,
    /// The row a `load` was just issued for, so the engine's `Started` can tell
    /// a first start (or a `repeat=one` restart) from a gapless handover.
    pending_load: Option<String>,
    /// Where the pending load starts, so a second load of the SAME row at the
    /// SAME position collapses into it instead of stacking a second voice.
    pending_start: f64,
    /// A fault the policy is retrying: the transport says "reprise", not
    /// "paused", and a STALL (the retry budget spent) goes back to Paused with
    /// its cause kept.
    retrying: bool,
}

impl Machine {
    fn new(state: PlayerState, audio: AudioSettings) -> Machine {
        Machine {
            state,
            audio,
            faults: Faults::new(),
            sleep: None,
            version: 1,
            error: None,
            fault: None,
            queue_version: 1,
            screen_on: true,
            ducked: false,
            pending_load: None,
            pending_start: 0.0,
            retrying: false,
        }
    }

    /// The gain the person's volume is multiplied by: the duck ramp and the
    /// sleep fade, exactly as `Player.svelte` multiplies `sleepFade` into the
    /// element's volume.
    fn volume(&self, now_ms: u64) -> f32 {
        let base = self.state.volume() as f32;
        let ducked = if self.ducked { DUCK_GAIN } else { 1.0 };
        let sleep = self.sleep.map(|sleep| sleep.fade(now_ms)).unwrap_or(1.0);
        (base * ducked * sleep).clamp(0.0, 1.0)
    }

    fn play_state(&self) -> PlayState {
        if self.state.current().is_none() {
            return PlayState::Stopped;
        }
        if self.state.playing() {
            PlayState::Playing
        } else if self.retrying {
            // A retry in flight says "reprise", not a pause over silence.
            PlayState::Buffering
        } else {
            PlayState::Paused
        }
    }

    fn bump(&mut self) {
        self.version += 1;
    }

    /// Applies one command to the state machine, under the lock the caller
    /// already holds.
    fn apply_command(&mut self, cmd: Command, now: u64) -> Action {
        let touch = cmd.touches_queue();
        let action = match cmd {
            Command::Play => self.effects(nz_player::Command::play(), now),
            // A pause is where an episode's position is written (a pause is
            // often the last thing before the app is frozen), and the write is
            // I/O: the worker does it, the lock is released first.
            Command::Pause => match self.effects(nz_player::Command::pause(), now) {
                Action::Effects(effects) => Action::Pause(effects),
                other => other,
            },
            Command::Toggle => self.effects(nz_player::Command::toggle(), now),
            Command::Next => self.effects(nz_player::Command::next(), now),
            Command::Prev => self.effects(nz_player::Command::prev(), now),
            Command::Seek { s } => self.effects(nz_player::Command::seek(s), now),
            Command::Jump { index, key } => {
                let index = self.resolve_index(index, &key);
                self.effects(nz_player::Command::jump(index), now)
            }
            Command::Remove { index, key } => {
                let index = self.resolve_index(index, &key);
                self.effects(nz_player::Command::remove_at(index), now)
            }
            Command::Move { from, to } => {
                self.effects(nz_player::Command::move_to(from as usize, to as usize), now)
            }
            Command::ClearUpcoming => self.effects(nz_player::Command::clear_upcoming(), now),
            Command::SetShuffle { on } => {
                if self.state.shuffle() == on {
                    Action::Effects(Vec::new())
                } else {
                    self.effects(nz_player::Command::toggle_shuffle(), now)
                }
            }
            Command::CycleRepeat => self.effects(nz_player::Command::cycle_repeat(), now),
            Command::SetVolume { v } => {
                self.effects(nz_player::Command::set_volume(f64::from(v)), now)
            }
            Command::ToggleMute => self.effects(nz_player::Command::toggle_mute(), now),
            Command::PlayQueue {
                tracks,
                start,
                context,
            } => {
                let entries = entries_of(tracks);
                if entries.is_empty() {
                    return Action::Effects(Vec::new());
                }
                self.push_progress(now);
                self.effects(
                    nz_player::Command::play_queue(entries, start as usize, context),
                    now,
                )
            }
            Command::AddToQueue { tracks } => {
                let entries = entries_of(tracks);
                self.effects(nz_player::Command::add_to_queue(entries), now)
            }
            Command::PlayNext { tracks } => {
                let entries = entries_of(tracks);
                self.effects(nz_player::Command::play_next(entries), now)
            }
            Command::PlayEntity { kind, id, shuffle } => {
                return Action::PlayEntity { kind, id, shuffle };
            }
            Command::StartRadio { kind, id } => return Action::Radio { kind, id },
            Command::StartFlow { style } => return Action::Flow { style },
        };
        if touch {
            self.queue_version += 1;
        }
        action
    }

    /// The index a row-named edit really means: the given one when it still
    /// holds that row, else the row carrying the key, else the given index
    /// (`commands.js#resolveIndex`: a wrong jump is worse than none, but an
    /// edit that does nothing at all is worse still).
    fn resolve_index(&self, index: u32, key: &str) -> usize {
        if key.is_empty() {
            return index as usize;
        }
        let at = index as usize;
        if let Some(entry) = self.state.queue().get(at)
            && entry.key() == key
        {
            return at;
        }
        self.state
            .queue()
            .iter()
            .position(|entry| entry.key() == key || entry.reference() == Some(key))
            .unwrap_or(at)
    }

    fn effects(&mut self, cmd: nz_player::Command, now: u64) -> Action {
        let effects = self.state.apply(cmd, now);
        if !effects.is_empty() {
            self.bump();
        }
        Action::Effects(effects)
    }

    /// The listen ledger is closed by the next command anyway; a pause is also
    /// where an episode's position must be written (`podcastProgress.js`: a
    /// pause is often the last thing before the app is frozen).
    fn push_progress(&mut self, now: u64) {
        let _ = now;
    }
}

/// What the machine asked for, once a command has been applied.
enum Action {
    Effects(Vec<Effect>),
    /// A pause: the effects, and the episode's position asked for RIGHT NOW.
    Pause(Vec<Effect>),
    PlayEntity {
        kind: String,
        id: String,
        shuffle: bool,
    },
    Radio {
        kind: String,
        id: String,
    },
    Flow {
        style: Option<String>,
    },
}

/// One piece of work for the player's own thread.
enum Job {
    Act(Action, u64),
    Engine(EngineEvent),
    Audio,
    Screen(bool),
    Shutdown,
}

/// Everything the worker owns: created on its own thread and gone with it.
struct Work {
    jobs: Receiver<Job>,
    reports: Reports,
    /// The tracks whose gains and bounds are known, so a repeat play asks
    /// nothing.
    meta: HashMap<String, TrackMeta>,
    /// The loudness the server answered (`None` = measured as absent, which is
    /// not the same as "not asked").
    loudness: HashMap<String, Option<f32>>,
    edges: HashMap<String, nz_audio::Edges>,
    edges_pending: std::collections::HashSet<String>,
    /// When the current row may be retried, in milliseconds.
    retry_at: Option<u64>,
    /// The row a fault is retrying.
    retrying: Option<String>,
    progress_at: u64,
    persist_at: u64,
    /// What the session heard, for the Flow's own session report.
    heard: Vec<(String, u32)>,
    last_queue_version: u64,
}

impl Player {
    /// Builds the whole graph: the runtime, the engine and its sink, the opener,
    /// the worker and the engine's event forwarder. Cheap on the person's
    /// screen: it opens no stream and touches no network.
    pub fn open(core: Arc<Core>) -> Result<Arc<Player>, PlayerError> {
        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("nz-player-net")
                .enable_all()
                .build()
                .map_err(|error| PlayerError::Start(error.to_string()))?,
        );
        let ladder = Arc::new(Mutex::new(Ladder::new()));
        let failures = Arc::new(Failures::new());
        let streaming = Arc::new(AtomicBool::new(false));
        let rate = Arc::new(AtomicU64::new(0));
        let (gone_tx, gone_rx) = watch::channel(false);
        let opener = Arc::new(ServerOpener::new(
            Arc::clone(&core),
            runtime.handle().clone(),
            Arc::clone(&ladder),
            Arc::clone(&rate),
            Arc::clone(&failures),
            Arc::clone(&streaming),
            gone_tx,
        ));
        // LN13: the engine takes its opener ONCE, and the phone builds its player
        // from the first screen — before the LAN node can exist (a node enrols
        // against the session's client). `PlayerSource` is that stable opener: the
        // server's own until a node exists, a peer asked first from then on
        // (docs/agents/lan-nodes.md, *Mobile FFI*).
        let source =
            crate::lan::PlayerSource::new(Arc::clone(&opener) as Arc<dyn nz_audio::StreamOpener>);
        let config = EngineConfig {
            sample_rate: 48_000,
            volume: 1.0,
            spool_dir: core.cache().audio_dir(),
            read_ahead_bytes: READ_AHEAD_ON_BYTES,
            ..EngineConfig::default()
        };
        let (engine, sink) = Engine::new_external(config, source);
        let store = Store::new(core.cache());
        let audio = store.load_audio();
        let (mut state, restore_effects) = match store.load_queue() {
            Some(snapshot) => PlayerState::restore(snapshot),
            None => (PlayerState::new(), Vec::new()),
        };
        state.set_shuffle_seed(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_nanos() as u64)
                .unwrap_or(0x5DEE_CE66_D1CE_1234),
        );
        let queue_version = 1;
        let machine = Machine::new(state, audio);
        let (jobs, jobs_rx) = std::sync::mpsc::channel();

        let player = Arc::new(Player {
            core: Arc::clone(&core),
            machine: Mutex::new(machine),
            locks: AtomicU64::new(0),
            last_command_locks: AtomicU64::new(0),
            jobs,
            worker: Mutex::new(None),
            events: Mutex::new(None),
            runtime: Mutex::new(Some(Arc::clone(&runtime))),
            engine,
            sink: Mutex::new(Some(sink)),
            opener,
            listeners: Mutex::new(Vec::new()),
            delivery: Mutex::new(()),
            stopped: AtomicBool::new(false),
            rate,
            streaming,
            tap_ns: AtomicU64::new(0),
            first_audible_ns: AtomicU64::new(0),
            stop_generation: AtomicU64::new(0),
            duration: AtomicU64::new(0),
            buffered: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            rebuffers: AtomicU64::new(0),
            route: Mutex::new(AudioRoute::default()),
            start: Instant::now(),
        });

        // The settings the engine starts with, applied once so a restored
        // session sounds like the one that was closed.
        player.apply_audio();

        let worker = {
            let player = Arc::clone(&player);
            let reports = Reports::new(Arc::clone(&core), runtime.handle().clone(), gone_rx);
            let work = Work {
                jobs: jobs_rx,
                reports,
                meta: HashMap::new(),
                loudness: HashMap::new(),
                edges: HashMap::new(),
                edges_pending: std::collections::HashSet::new(),
                retry_at: None,
                retrying: None,
                progress_at: 0,
                persist_at: 0,
                heard: Vec::new(),
                last_queue_version: queue_version,
            };
            std::thread::Builder::new()
                .name("nz-player".to_owned())
                .spawn(move || player.run(work))
                .map_err(|error| PlayerError::Start(error.to_string()))?
        };
        *player.worker.lock().unwrap_or_else(|err| err.into_inner()) = Some(worker);

        // The engine's events, on their own thread: the bus blocks, and the
        // worker must never block on a channel only a close would drain.
        let events = {
            let player = Arc::clone(&player);
            let bus = player.engine.events();
            let jobs = player.jobs.clone();
            std::thread::Builder::new()
                .name("nz-player-events".to_owned())
                .spawn(move || {
                    while !player.stopped.load(Ordering::SeqCst) {
                        match bus.recv_timeout(EVENT_TICK) {
                            Ok(event) => {
                                if jobs.send(Job::Engine(event)).is_err() {
                                    break;
                                }
                            }
                            Err(RecvTimeoutError::Timeout) => continue,
                            Err(RecvTimeoutError::Disconnected) => break,
                        }
                    }
                })
                .map_err(|error| PlayerError::Start(error.to_string()))?
        };
        *player.events.lock().unwrap_or_else(|err| err.into_inner()) = Some(events);

        // A restored queue comes back PAUSED at the position it had: the effects
        // `restore` produced are the same ones a command produces.
        if !restore_effects.is_empty() {
            let now = player.now_ms();
            let _ = player
                .jobs
                .send(Job::Act(Action::Effects(restore_effects), now));
        }
        Ok(player)
    }

    /// Enqueues a command and returns. **One lock**, and nothing that can wait:
    /// the caller is a Compose click handler.
    ///
    /// The machine's transport reaches the ENGINE here too, and not only through
    /// a `Load`: a tap on pause must silence the mixer NOW even when the worker
    /// is inside a report, and a tap on play must wake a player the restore left
    /// paused. Both are one atomic store and a conditional notify in `nz-audio`.
    pub fn command(&self, cmd: Command, tap_ns: u64) {
        let now = self.now_ms();
        self.tap_ns.store(tap_ns, Ordering::SeqCst);
        TAKEN.with(|taken| taken.set(0));
        // A command that REPLACES the queue, or jumps to ANOTHER row, drops the
        // voices that belonged to the old one before the transport resumes them:
        // a restored voice of the same id, resumed for a few milliseconds, ends
        // and its `Ended` would be read against the new queue and eat its first
        // rows. A jump to the SAME row keeps its pending voice: a seek that is
        // still opening at the same position is the one the jump wants, and
        // dropping it left no voice at all (`a_jump_after_a_pause_and_a_seek_renders`).
        let was_queue = matches!(&cmd, Command::PlayQueue { .. });
        let was_jump = matches!(&cmd, Command::Jump { .. });
        let was_step = matches!(&cmd, Command::Next | Command::Prev);
        let engine_before = self.engine.current_id();
        let preloaded = self.engine.preloaded_id();
        let (action, playing, replacing) = {
            let mut machine = self.lock();
            let action = machine.apply_command(cmd, now);
            let new_row = machine
                .state
                .current()
                .and_then(|entry| entry.reference())
                .map(str::to_owned);
            // A skip onto the row the engine already decoded is a promotion in
            // the mixer (no gap, no reopen). A skip onto any other row opens
            // it, and the voice left behind must not go on playing under the
            // new title and cover for as long as the stream takes to open: the
            // screen names the row the person chose, so that is the one the
            // speaker serves or the silence of "loading".
            let steps_away = was_step
                && engine_before.is_some()
                && engine_before != new_row
                && preloaded != new_row;
            let replacing = was_queue
                || (was_jump && engine_before.is_some() && engine_before != new_row)
                || steps_away;
            if replacing {
                // The pending load named a voice this command just dropped.
                machine.pending_load = None;
                machine.pending_start = 0.0;
            }
            (action, machine.state.playing(), replacing)
        };
        if replacing {
            self.stop_generation
                .store(self.engine.max_generation(), Ordering::SeqCst);
            self.engine.stop_current();
        }
        self.last_command_locks
            .store(TAKEN.with(|taken| taken.get()), Ordering::SeqCst);
        let _ = self.jobs.send(Job::Act(action, now));
        self.apply_transport(playing);
    }

    /// How many locks the last `command()` took. The acceptance holds it to 1:
    /// the caller of a transport command waits for nothing else.
    pub fn last_command_locks(&self) -> u64 {
        self.last_command_locks.load(Ordering::SeqCst)
    }

    /// The whole picture, cheap to copy.
    pub fn snapshot(&self) -> Snapshot {
        let machine = self.lock();
        let quality = self.quality_locked(&machine);
        let now = self.now_ms();
        Snapshot {
            state: machine.play_state(),
            current: machine
                .state
                .current()
                .map(|entry| NowPlaying::from_entry(entry, quality.as_str(), false)),
            index: (machine.state.index() >= 0).then(|| machine.state.index() as u32),
            len: machine.state.len() as u32,
            shuffle: machine.state.shuffle(),
            repeat: machine.state.repeat(),
            volume: machine.state.volume() as f32,
            muted: machine.state.muted(),
            streaming: self.streaming.load(Ordering::SeqCst),
            sleep: machine.sleep.map(|sleep| sleep.state(now)),
            error: machine.error.clone(),
            fault: machine.fault.clone(),
            version: machine.version,
        }
    }

    /// The HOT read — atomics and the engine's own atomic clock, no lock.
    pub fn position(&self) -> Position {
        Position {
            position_s: self.engine.position(),
            duration_s: f64::from_bits(self.duration.load(Ordering::SeqCst)),
            buffered_s: f64::from_bits(self.buffered.load(Ordering::SeqCst)),
            playing: self.playing.load(Ordering::SeqCst),
        }
    }

    /// The queue, as the rows a windowed list draws. Keyed by the track's own
    /// id: unique in a queue, and what a like/dislike patch names.
    pub fn queue_rows(&self) -> Vec<crate::lists::Row> {
        let machine = self.lock();
        machine
            .state
            .queue()
            .iter()
            .enumerate()
            .map(|(position, entry)| row_of(entry, position))
            .collect()
    }

    /// The version of the queue's composition: a screen re-windows its handle
    /// when it changes (`PlayerListener::on_queue_changed`).
    pub fn queue_version(&self) -> u64 {
        self.lock().queue_version
    }

    pub fn subscribe(&self, sink: Arc<dyn Sink>) {
        self.listeners
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .push(sink);
    }

    pub fn audio(&self) -> AudioSettings {
        self.lock().audio.clone()
    }

    /// Persists and applies. The engine hears it through the worker, so a slider
    /// never waits on the mixer.
    pub fn set_audio(&self, settings: AudioSettings) {
        let settings = settings.clamped();
        {
            let mut machine = self.lock();
            if machine.audio == settings {
                return;
            }
            machine.audio = settings.clone();
            machine.bump();
        }
        Store::new(self.core.cache()).save_audio(&settings);
        self.apply_audio();
        let _ = self.jobs.send(Job::Audio);
        self.emit_change();
    }

    /// The numbers Aide and the journal show. `performance_mode` and
    /// `buffer_ms` are the PLATFORM's (Android's `DroidOutput`); this core
    /// reports what `nz-audio` knows and leaves those two to `nz-ffi`, which
    /// owns the output on the device.
    pub fn output_stats(&self) -> OutputStats {
        let info = self.engine.output_info();
        let rate = info.sample_rate.max(1);
        OutputStats {
            underruns: self.engine.underruns(),
            rebuffers: self.rebuffers.load(Ordering::SeqCst),
            buffer_ms: 0,
            sample_rate: info.sample_rate,
            latency_ms: (info.latency_frames.saturating_mul(1000) / u64::from(rate)) as u32,
            performance_mode: String::new(),
            last_tap_ns: self.tap_ns.load(Ordering::SeqCst),
            last_first_audible_ns: self.first_audible_ns.load(Ordering::SeqCst),
        }
    }

    /// Screen off raises the read-ahead to 32 MB (the radio sleeps between
    /// bursts); screen on brings it back to 2 MB (rule 1: a prefetch must not
    /// eat the link the person is using).
    pub fn set_screen_on(&self, on: bool) {
        {
            let mut machine = self.lock();
            if machine.screen_on == on {
                return;
            }
            machine.screen_on = on;
        }
        self.set_read_ahead(on);
        let _ = self.jobs.send(Job::Screen(on));
    }

    /// The audio route the platform sees (MB12's verdict is per connection).
    pub fn set_route(&self, route: AudioRoute) {
        *self.lock_route() = route;
    }

    /// A counted subscription to the engine's analysis tap (MB12): the
    /// visualiser and the equalizer card each read their own ring, so both can
    /// be on screen at once and neither takes the other's frames. The frames are
    /// post-DSP and pre-pan (`nz_audio::tap`).
    pub fn subscribe_tap(&self) -> Option<nz_audio::TapSubscription> {
        self.engine.subscribe_tap()
    }

    /// The equalizer card's meter, on while that card is on screen. Off, the
    /// output callback does exactly the arithmetic it did before (MB12).
    pub fn set_meter(&self, on: bool) {
        self.engine.set_meter(on);
    }

    /// The last buffer's pre-limiter peak and limiter reduction: the two numbers
    /// `saturation.js` shows, measured by the chain that makes them.
    pub fn chain_meter(&self) -> nz_audio::ChainMeter {
        self.engine.chain_meter()
    }

    pub fn route(&self) -> AudioRoute {
        self.lock_route().clone()
    }

    /// The audio focus said "pause, or duck": a duck ramps to [`DUCK_GAIN`],
    /// never to silence (`set_ducked(false)` restores the level).
    pub fn set_ducked(&self, ducked: bool) {
        {
            let mut machine = self.lock();
            if machine.ducked == ducked {
                return;
            }
            machine.ducked = ducked;
            machine.bump();
        }
        self.apply_volume();
        self.emit_change();
    }

    /// A sleep timer: minutes, or the end of the track.
    pub fn set_sleep(&self, timer: Option<SleepTimer>) {
        let now = self.now_ms();
        {
            let mut machine = self.lock();
            machine.sleep = timer.map(|timer| Sleep::new(timer, now));
            machine.bump();
        }
        self.emit_change();
    }

    /// The engine's scheduled-PCM channel: what a listen-party GUEST plays
    /// (MB20, `nz_audio::PartyChannel`).
    ///
    /// It is the one path into the mixer's party voice, and it is deliberately
    /// the PLAYER's engine: a guest schedules its six-second chunks on the
    /// output clock this engine reports, the sound goes through the person's own
    /// DSP chain and output, and [`nz_audio::PartyChannel::take_over`] leaves
    /// their queue in place while somebody else's music plays. A second engine
    /// would be a second output, which is what "the player is the singleton"
    /// forbids (CLAUDE.md).
    pub fn party(&self) -> nz_audio::PartyChannel {
        self.engine.party()
    }

    /// The platform's pull sink and the engine that feeds it, taken ONCE: on
    /// Android by `DroidOutput` (AAudio, or the API-26 `AudioTrack`), on the host
    /// by the tests, which render by hand.
    pub fn take_sink(&self) -> Option<(Engine, ExternalSink)> {
        let sink = self
            .sink
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .take()?;
        Some((self.engine.clone(), sink))
    }

    /// The counted bytes of what the player holds in memory (worker rule 4):
    /// the queue's rows, their strings, and the list a screen reads.
    pub fn memory_bytes(&self) -> u64 {
        let machine = self.lock();
        machine
            .state
            .queue()
            .iter()
            .map(|entry| entry_bytes(entry))
            .sum()
    }

    /// How many times the machine's lock was taken — the acceptance's "1 lock".
    pub fn locks_taken(&self) -> u64 {
        self.locks.load(Ordering::SeqCst)
    }

    /// Stops and JOINS every thread this player started, then drops the runtime.
    pub fn shutdown(&self) {
        if self.stopped.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = self.jobs.send(Job::Shutdown);
        // Every in-flight wait of the opener gives up at once, so the engine's
        // threads leave the runtime instead of waiting for their next chunk.
        self.opener.shutdown();
        // The quit path: report what was heard, and the episode's position.
        let now = self.now_ms();
        let effects = {
            let mut machine = self.lock();
            machine.state.flush_listen(now)
        };
        let mut work = Work {
            jobs: std::sync::mpsc::channel().1,
            reports: Reports::new(
                Arc::clone(&self.core),
                self.runtime_handle(),
                self.opener.gone(),
            ),
            meta: HashMap::new(),
            loudness: HashMap::new(),
            edges: HashMap::new(),
            edges_pending: std::collections::HashSet::new(),
            retry_at: None,
            retrying: None,
            progress_at: 0,
            persist_at: 0,
            heard: Vec::new(),
            last_queue_version: 0,
        };
        self.report_progress(now, &mut work, true);
        self.perform(effects, now, Some(&mut work));
        self.persist(&mut work);
        self.engine.shutdown();
        if let Some(thread) = self
            .worker
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .take()
        {
            let _ = thread.join();
        }
        if let Some(thread) = self
            .events
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .take()
        {
            let _ = thread.join();
        }
        // Last: no thread of ours is inside it any more.
        let runtime = self
            .runtime
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .take();
        drop(runtime);
    }

    // -- the worker ---------------------------------------------------------

    fn run(self: Arc<Self>, mut work: Work) {
        while !self.stopped.load(Ordering::SeqCst) {
            let timeout = if self.fading() { FADE_TICK } else { TICK };
            match work.jobs.recv_timeout(timeout) {
                Ok(Job::Shutdown) => break,
                Ok(job) => self.handle(job, &mut work),
                Err(RecvTimeoutError::Timeout) => {
                    let now = self.now_ms();
                    self.tick(now, &mut work);
                    // The sleep timer and a stall pause through the tick, and a
                    // tick has no effect list to carry that to the mixer.
                    self.transport_now();
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    fn handle(&self, job: Job, work: &mut Work) {
        match job {
            Job::Shutdown => {}
            Job::Act(Action::Effects(effects), now) => {
                self.perform(effects, now, Some(work));
                self.offer_work(work);
            }
            Job::Act(Action::Pause(effects), now) => {
                self.perform(effects, now, Some(work));
                self.report_progress(now, work, true);
                self.offer_work(work);
            }
            Job::Act(Action::PlayEntity { kind, id, shuffle }, now) => {
                self.resolve_entity(&kind, &id, shuffle, now, work);
                self.offer_work(work);
            }
            Job::Act(Action::Radio { kind, id }, now) => {
                self.start_radio(&kind, &id, now, work);
                self.offer_work(work);
            }
            Job::Act(Action::Flow { style }, now) => {
                self.start_flow(&style, now, work);
                self.offer_work(work);
            }
            Job::Engine(event) => {
                self.engine_event(event, work);
                self.offer_work(work);
            }
            Job::Audio => {
                self.apply_audio();
                self.audio_changed(work);
            }
            Job::Screen(on) => self.set_read_ahead(on),
        }
        // Every job that can move the transport (an advance, an end, a stall, a
        // sleep deadline) ends here, so the engine hears it in the same turn.
        self.transport_now();
    }

    /// Applies the machine's transport to the ENGINE. A `Load` is not the only
    /// way the sound starts or stops: a pause, a resume, an end of queue, a
    /// stall and a sleep deadline all change the machine without one, and the
    /// mixer used to keep rendering through them (PAUSED over sound, and a
    /// restored queue that answered PLAY with silence).
    fn transport_now(&self) {
        let playing = {
            let machine = self.lock();
            machine.state.playing()
        };
        self.apply_transport(playing);
    }

    fn apply_transport(&self, playing: bool) {
        if playing {
            self.engine.play();
        } else {
            self.engine.pause();
        }
        self.playing.store(playing, Ordering::SeqCst);
    }

    /// Tells the screens what changed, after a job: the snapshot on every
    /// change, the queue's version when its composition moved.
    fn offer_work(&self, work: &mut Work) {
        // The core's total, so `NzCore::memory()` is the app's own number
        // (worker rule 4: counted bytes, never a process's RSS).
        self.core.set_queue_bytes(self.memory_bytes());
        let version = self.queue_version();
        if version != work.last_queue_version {
            work.last_queue_version = version;
            self.emit_queue(version);
        }
        self.emit_change();
    }

    /// Carries out the effects `nz-player` produced, in their own order
    /// (`ReportListen`, `Load`, `PreloadNext`, `NeedMore`, `Persist`,
    /// `Changed`).
    fn perform(&self, effects: Vec<Effect>, now: u64, mut work: Option<&mut Work>) {
        for effect in effects {
            match effect {
                Effect::ReportListen {
                    id,
                    listened_s,
                    next_id,
                    context,
                    shuffle,
                } => {
                    // A row skipped before it played reports 0 s: the server
                    // would count a play for a title nobody heard.
                    if listened_s == 0 {
                        continue;
                    }
                    let payload = nz_model::ListenPayload {
                        id: nz_model::TrackId::new(id.clone()),
                        listened: listened_s,
                        next_id: next_id.map(nz_model::TrackId::new),
                        context: context.map(|context| nz_model::ListenContext {
                            kind: context.kind,
                            id: context.id,
                            title: context.title,
                            artist: None,
                        }),
                        shuffle,
                    };
                    if let Some(work) = work.as_deref_mut() {
                        work.heard.push((id, listened_s));
                        work.reports.listen(&payload);
                    }
                }
                Effect::Load {
                    entry,
                    autoplay,
                    start_s,
                } => self.load(&entry, autoplay, start_s, now, work.as_deref_mut()),
                Effect::PreloadNext(next) => {
                    if let Some(work) = work.as_deref_mut() {
                        self.preload(next.as_ref(), work);
                    }
                }
                Effect::NeedMore { context } => {
                    if let Some(work) = work.as_deref_mut() {
                        self.need_more(context, now, work);
                    }
                }
                Effect::Persist => {
                    if let Some(work) = work.as_deref_mut() {
                        self.persist(work);
                    }
                }
                Effect::Changed => {}
            }
        }
    }

    /// Opens a row on the engine, with the quality the ladder earns, the gains
    /// and bounds the server already answered, and the offset it was asked for.
    fn load(
        &self,
        entry: &Entry,
        autoplay: bool,
        start_s: f64,
        now: u64,
        mut work: Option<&mut Work>,
    ) {
        let Some(reference) = entry.reference().map(str::to_owned) else {
            tracing::warn!(title = %entry.title, "a row with no request id cannot be played");
            return;
        };
        // A seek inside the row that is already playing is the ENGINE's plan (a
        // byte range on a finished file, `?start=` on a generated one, or an
        // explicit defer — `lib/audio/seekplan.js`), never a restart at 0.
        let same = self.engine.current_id().as_deref() == Some(reference.as_str());
        let position = if entry.is_podcast() && start_s <= 0.0 {
            work.as_deref()
                .and_then(|work| {
                    work.reports
                        .resume_point(&reference, f64::from(entry.duration))
                })
                .unwrap_or(0.0)
        } else {
            start_s
        };

        // Two loads of the SAME row at the SAME position while the first is
        // still opening (a seek then a jump — `NowPlayingDeviceTest`'s 20 taps
        // on two titles) must be ONE voice: two `Started`s of that id make the
        // second look like a `repeat=one` restart and advance the queue (the
        // device test's queue stalled on "trial 1 never rendered").
        let collapses = {
            let machine = self.lock();
            machine.pending_load.as_deref() == Some(reference.as_str())
                && (machine.pending_start - position).abs() < 0.001
        };
        if collapses {
            tracing::debug!(id = %reference, position, "load collapsed into the pending one");
        } else if same && position > 0.0 {
            // A seek inside the row that is already playing. The engine rebuilds
            // the voice at `position` and announces it with a `Started` exactly
            // like a load does, so this has to be recorded as the pending load:
            // without it `on_started` reads that `Started` as a `repeat=one`
            // restart and advances the queue. Measured: a seek to the middle of
            // a title skipped to the next one, and a fault RETRY (which seeks at
            // the position it stopped at) turned every transient error into a
            // skip — the person's "it jumps by itself at half the song".
            {
                let mut machine = self.lock();
                machine.pending_load = Some(reference.clone());
                machine.pending_start = position;
            }
            self.engine.seek(position);
        } else {
            let quality = self.effective_quality();
            {
                let mut machine = self.lock();
                machine.pending_load = Some(reference.clone());
                machine.pending_start = position;
            }
            if let Some(work) = work.as_deref_mut() {
                self.prime(entry, work, true);
                let meta = work.meta.get(&reference).copied().unwrap_or_default();
                self.engine.set_track_meta(&reference, meta);
            }
            let audio_quality = to_audio_quality(quality);
            // The row was ALREADY decoded by `preload_next`: a manual skip (the
            // next button, a headset, MediaSession, the car) promotes that voice
            // in the mixer instead of reopening the source it just paid for.
            // `false` (another id or quality, or a natural handover already
            // consumed it) falls back to a real load.
            let promoted = position <= 0.001
                && self.engine.preloaded_id().as_deref() == Some(reference.as_str())
                && self.engine.skip_to_preloaded(&reference, audio_quality);
            if !promoted {
                self.engine.load(
                    &reference,
                    audio_quality,
                    Some(position).filter(|at| *at > 0.0),
                );
            }
        }
        if autoplay {
            self.engine.play();
            self.playing.store(true, Ordering::SeqCst);
        } else {
            self.engine.pause();
            self.playing.store(false, Ordering::SeqCst);
        }
        self.duration
            .store(f64::from(entry.duration).to_bits(), Ordering::SeqCst);
        self.buffered.store(0, Ordering::SeqCst);
        if let Some(work) = work {
            work.retry_at = None;
            work.retrying = None;
        }
        tracing::debug!(id = %reference, autoplay, position, at = now, "load");
    }

    /// Fills the engine's second voice, once its bounds are known (a voice built
    /// before they land plays the silence the trim exists to remove — D6's own
    /// measured rule).
    fn preload(&self, next: Option<&Entry>, work: &mut Work) {
        let Some(entry) = next else {
            return;
        };
        let Some(reference) = entry.reference().map(str::to_owned) else {
            return;
        };
        self.prime(entry, work, false);
        let meta = work.meta.get(&reference).copied().unwrap_or_default();
        self.engine.set_track_meta(&reference, meta);
        self.engine
            .preload_next(&reference, to_audio_quality(self.effective_quality()));
    }

    /// Asks the server for what it knows about a row before it plays.
    ///
    /// **Only what the settings need**, and on the lane the row deserves (rule 1):
    ///
    /// - the row being LOADED (the action just taken) asks on `User`, and only
    ///   for what changes the sound: its silence bounds when the trim is on, its
    ///   loudness when normalisation is on. With both off — the defaults — the
    ///   first play makes NO extra request and starts at once;
    /// - the NEXT row (a prefetch) asks the same two things on `Prefetch`, which
    ///   `nz-api` holds back until no `User` request has been in flight for
    ///   250 ms, so its own bounds never queue in front of the track being heard.
    ///
    /// The loudness is asked once for the whole window (`lib/gaincache.js`'s
    /// batching: one request for the playing row and the seven after it), so a
    /// queue that plays on asks nothing per track.
    fn prime(&self, entry: &Entry, work: &mut Work, current: bool) {
        let Some(reference) = entry.reference().map(str::to_owned) else {
            return;
        };
        work.meta.entry(reference.clone()).or_default().duration_s =
            Some(f64::from(entry.duration));
        work.meta.entry(reference.clone()).or_default().podcast = entry.is_podcast();

        let Ok(api) = self.core.api() else {
            return;
        };
        let (normalization, trim) = {
            let machine = self.lock();
            (machine.audio.normalization, machine.audio.trim_silence)
        };
        let lane = if current {
            nz_api::Lane::User
        } else {
            nz_api::Lane::Prefetch
        };

        // The loudness: the playing row and the window after it, one request.
        if normalization != Normalization::Off && !work.loudness.contains_key(&reference) {
            let ids: Vec<String> = {
                let machine = self.lock();
                let index = machine.state.index().max(0) as usize;
                machine
                    .state
                    .queue()
                    .iter()
                    .skip(index)
                    .take(PRIME_WINDOW)
                    .filter_map(|row| row.reference().map(str::to_owned))
                    .collect()
            };
            for id in &ids {
                work.loudness.entry(id.clone()).or_insert(None);
            }
            if !ids.is_empty() {
                let mut gone = self.opener.gone();
                let answer =
                    source::block_on(&self.runtime_handle(), &mut gone, api.track_gains(ids));
                if let Some(Ok(gains)) = answer {
                    for (id, gain) in gains.gains {
                        work.loudness
                            .insert(id.clone(), gain.map(|value| value as f32));
                    }
                }
            }
        }
        let loudness = work.loudness.get(&reference).copied().flatten();
        work.meta.entry(reference.clone()).or_default().loudness = loudness;

        // The silence bounds, once each: a voice built before they land plays
        // the silence the trim exists to remove (D6's own measured rule).
        if trim
            && !work.edges.contains_key(&reference)
            && work.edges_pending.insert(reference.clone())
        {
            let mut gone = self.opener.gone();
            let answer = source::block_on(
                &self.runtime_handle(),
                &mut gone,
                api.audio_edges_on(lane, &reference, TRIM_DB),
            );
            work.edges_pending.remove(&reference);
            if let Some(Ok(edges)) = answer
                && edges.ready
                && let (Some(start), Some(end), Some(duration)) =
                    (edges.start, edges.end, edges.duration)
            {
                work.edges.insert(
                    reference.clone(),
                    nz_audio::Edges {
                        start,
                        end,
                        duration,
                    },
                );
            }
        }
        let edges = work
            .edges
            .get(&reference)
            .copied()
            .and_then(|edges| edges.accepted(Some(f64::from(entry.duration))));
        work.meta.entry(reference).or_default().edges = edges;
    }

    /// A Flow or a radio queue ran out: ask for the next title, once.
    fn need_more(&self, context: Context, now: u64, work: &mut Work) {
        let Ok(api) = self.core.api() else {
            return;
        };
        let exclude: Vec<String> = {
            let machine = self.lock();
            machine
                .state
                .queue()
                .iter()
                .filter_map(|entry| entry.reference().map(str::to_owned))
                .collect()
        };
        let heard = work.heard.clone();
        let mut gone = self.opener.gone();
        let runtime = self.runtime_handle();
        let answer = {
            let mut filler = Filler::new(api, &runtime, &mut gone);
            for (id, seconds) in &heard {
                filler.heard(id, *seconds);
            }
            filler.exclude(exclude);
            match context.kind.as_str() {
                "flow" => filler.flow(&None, flow::FLOW_REFILL_N),
                "radio" => {
                    let (kind, id) = split_context(&context);
                    filler.radio(&kind, &id)
                }
                _ => Ok(Vec::new()),
            }
        };
        match answer {
            Ok(rows) if !rows.is_empty() => {
                let effects = {
                    let mut machine = self.lock();
                    machine.queue_version += 1;
                    machine.state.auto_extend(rows, now)
                };
                self.perform(effects, now, Some(work));
            }
            // A top-up that answered nothing is a station that ran out: the
            // queue stops instead of asking again on every row.
            Ok(_) => tracing::debug!("no more rows for this context"),
            Err(cause) => {
                tracing::debug!(%cause, "the top-up failed");
                self.publish_fault(cause);
            }
        }
    }

    fn resolve_entity(&self, kind: &str, id: &str, shuffle: bool, now: u64, work: &mut Work) {
        let Ok(api) = self.core.api() else {
            self.publish_fault("Hors ligne : impossible de charger cette liste".to_owned());
            return;
        };
        let mut gone = self.opener.gone();
        let runtime = self.runtime_handle();
        let answer = {
            let mut filler = Filler::new(api, &runtime, &mut gone);
            filler.entity(kind, id, shuffle)
        };
        match answer {
            Ok(Fill::Rows(rows)) if !rows.is_empty() => {
                let context = Context {
                    kind: kind.to_owned(),
                    id: Some(id.to_owned()),
                    title: None,
                };
                self.play_rows(rows, 0, Some(context), now, work);
            }
            Ok(Fill::Rows(_)) => self.publish_fault("Cette liste est vide".to_owned()),
            Ok(Fill::Flow { style }) => self.start_flow(&style, now, work),
            Ok(Fill::Radio { kind, id }) => self.start_radio(&kind, &id, now, work),
            Err(cause) => self.publish_fault(cause),
        }
    }

    fn start_radio(&self, kind: &str, id: &str, now: u64, work: &mut Work) {
        let Ok(api) = self.core.api() else {
            self.publish_fault("Hors ligne : la radio ne peut pas démarrer".to_owned());
            return;
        };
        let mut gone = self.opener.gone();
        let runtime = self.runtime_handle();
        let answer = {
            let mut filler = Filler::new(api, &runtime, &mut gone);
            filler.radio(kind, id)
        };
        match answer {
            Ok(rows) if !rows.is_empty() => {
                let context = Context {
                    kind: "radio".to_owned(),
                    id: Some(format!("{kind}:{id}")),
                    title: None,
                };
                self.play_rows(rows, 0, Some(context), now, work);
            }
            Ok(_) => self.publish_fault("Rien à proposer pour cette radio".to_owned()),
            Err(cause) => self.publish_fault(cause),
        }
    }

    fn start_flow(&self, style: &Option<String>, now: u64, work: &mut Work) {
        let Ok(api) = self.core.api() else {
            self.publish_fault("Hors ligne : le Flow ne peut pas démarrer".to_owned());
            return;
        };
        let mut gone = self.opener.gone();
        let runtime = self.runtime_handle();
        let answer = {
            let mut filler = Filler::new(api, &runtime, &mut gone);
            filler.flow(style, flow::FLOW_START_N)
        };
        match answer {
            Ok(rows) if !rows.is_empty() => {
                let context = Context {
                    kind: "flow".to_owned(),
                    id: style.clone(),
                    title: None,
                };
                self.play_rows(rows, 0, Some(context), now, work);
            }
            Ok(_) => self.publish_fault("Rien à proposer pour l'instant".to_owned()),
            Err(cause) => self.publish_fault(cause),
        }
    }

    fn play_rows(
        &self,
        rows: Vec<Entry>,
        start: u32,
        context: Option<Context>,
        now: u64,
        work: &mut Work,
    ) {
        let effects = {
            let mut machine = self.lock();
            machine.queue_version += 1;
            machine.bump();
            machine.state.apply(
                nz_player::Command::play_queue(rows, start as usize, context),
                now,
            )
        };
        // The queue just changed under whatever voice was playing: drop it
        // BEFORE the new rows open, or its end/its preloaded neighbour start is
        // read against the new queue and advances it (measured: five rows eaten
        // in 0.2 s when a restored queue was replaced by a playlist).
        self.stop_generation
            .store(self.engine.max_generation(), Ordering::SeqCst);
        self.engine.stop_current();
        self.perform(effects, now, Some(work));
    }

    /// One of the engine's own events: the gapless handover, a seek landing, a
    /// failure and a finished body are read here.
    fn engine_event(&self, event: EngineEvent, work: &mut Work) {
        let now = self.now_ms();
        match event {
            EngineEvent::Started {
                id,
                position_s,
                duration_s,
                generation,
            } => {
                if generation <= self.stop_generation.load(Ordering::SeqCst) {
                    tracing::debug!(
                        id = %id,
                        generation,
                        floor = self.stop_generation.load(Ordering::SeqCst),
                        "a start from a voice a stop already dropped"
                    );
                    return;
                }
                if let Some(duration) = duration_s {
                    self.duration.store(duration.to_bits(), Ordering::SeqCst);
                }
                self.first_audible_ns
                    .store(now.saturating_mul(1_000_000), Ordering::SeqCst);
                self.playing.store(true, Ordering::SeqCst);
                self.on_started(&id, position_s, now, work);
            }
            EngineEvent::Ended { id, generation } => {
                if generation <= self.stop_generation.load(Ordering::SeqCst) {
                    tracing::debug!(
                        id = %id,
                        generation,
                        floor = self.stop_generation.load(Ordering::SeqCst),
                        "an end from a voice a stop already dropped"
                    );
                    return;
                }
                // The end belongs to the row the MACHINE names. A voice from a
                // queue the person just replaced (or one a seek superseded) can
                // finish after it: advancing on it skipped the new queue's first
                // titles (measured: five rows consumed in 0.2 s on a fresh
                // `PlayQueue`, `listened=0` on each).
                let current = {
                    let machine = self.lock();
                    machine
                        .state
                        .current()
                        .and_then(|entry| entry.reference())
                        .map(str::to_owned)
                };
                if current.as_deref() != Some(id.as_str()) {
                    tracing::debug!(id = %id, "an end for a row the machine no longer names");
                    return;
                }
                self.playing.store(false, Ordering::SeqCst);
                self.report_progress(now, work, true);
                let effects = {
                    let mut machine = self.lock();
                    machine.state.track_ended(now)
                };
                self.perform(effects, now, Some(work));
            }
            EngineEvent::Underrun { .. } => {}
            EngineEvent::SeekDeferred { .. } => {
                // The plan is doing its job (the server cannot answer yet): this
                // is a REBUFFER, the counter bridge 12 exists to tell apart from
                // an underrun — the source starved, the mixer did not. Published
                // on the core too, so `Assistance`'s report carries it without a
                // player handle.
                self.rebuffers.fetch_add(1, Ordering::SeqCst);
                self.core.note_rebuffer();
            }
            EngineEvent::Seeked { position_s } => {
                let mut machine = self.lock();
                machine.state.set_position(position_s);
                machine.bump();
            }
            EngineEvent::Error { id, message } => {
                let cause = id
                    .as_deref()
                    .and_then(|id| self.opener.failures().take(id))
                    .or(Some(nz_audio::SourceError::Message(message)));
                self.on_error(id, cause, now, work);
            }
            EngineEvent::OutputChanged { .. } => {}
            EngineEvent::StreamComplete {
                id,
                quality,
                path,
                bytes,
            } => {
                // A finished body is a complete track: publish it into the play
                // cache so a replay reads it from disk (`lib/playcache.js`).
                let track = nz_model::TrackId::new(id);
                if let Err(error) =
                    self.core
                        .cache()
                        .play_cache()
                        .complete(&track, api_quality(quality), bytes)
                {
                    tracing::debug!(%error, "the finished body was not published");
                }
                self.buffered.store(
                    f64::from_bits(self.duration.load(Ordering::SeqCst)).to_bits(),
                    Ordering::SeqCst,
                );
                tracing::debug!(path = %path.display(), bytes, "track cached");
            }
        }
    }

    /// A voice began: either the row the machine already names (the load that
    /// was asked for, or a `repeat=one` restart), or the NEXT row the engine took
    /// over from gaplessly — which advances the machine at the START of the
    /// handover, so the bar and the notification name the track being heard
    /// (`Player.svelte#adoptIncoming`).
    fn on_started(&self, id: &str, position_s: f64, now: u64, work: &mut Work) {
        let (current, pending, next) = {
            let machine = self.lock();
            let current = machine
                .state
                .current()
                .and_then(|entry| entry.reference())
                .map(str::to_owned);
            // NOT taken yet: a start that belongs to no row of this queue must
            // leave the pending load alone, or the row it named later looks like
            // a `repeat=one` restart and the queue advances by itself (measured:
            // the first title of a fresh `PlayQueue` ended 7 ms after its load).
            let pending = machine.pending_load.clone();
            let next = machine
                .state
                .next_entry()
                .and_then(|entry| entry.reference())
                .map(str::to_owned);
            (current, pending, next)
        };
        // A voice may start for a row that is neither the one the machine names,
        // the one a load just asked for, nor the next one a gapless handover
        // owes: it is a voice of a queue that no longer exists, and letting it
        // advance skipped the new queue (the same measured cascade as `Ended`).
        // The next row is only a handover when the ENGINE's current voice IS it:
        // a stale seek's voice for the next row starts after a jump, and the
        // engine is already on the jumped row (measured: trial 2 of
        // `a_jump_after_a_pause_and_a_seek_renders` skipped row 0).
        let engine_current = self.engine.current_id();
        // A handover happens while NO load is outstanding: with one pending the
        // machine is waiting for that row's own `Started`, and a start of the
        // next row is the late announcement of a voice a skip promoted before a
        // Previous moved the queue back (measured: Next then Prev at once ended
        // on the row after, the speaker on the one before).
        let next_handover = pending.is_none()
            && next.as_deref() == Some(id)
            && engine_current.as_deref() == Some(id);
        if current.as_deref() != Some(id) && pending.as_deref() != Some(id) && !next_handover {
            tracing::debug!(
                id = %id,
                next = ?next,
                engine = ?engine_current,
                "a start for a row outside the queue"
            );
            return;
        }
        let pending = {
            let mut machine = self.lock();
            if current.as_deref() == Some(id) {
                machine.state.set_position(position_s);
                machine.faults.started();
                machine.error = None;
                machine.fault = None;
                machine.retrying = false;
                machine.bump();
            }
            machine.pending_load.take()
        };
        if current.as_deref() == Some(id) {
            if pending.as_deref() != Some(id) {
                // The same row started again without us asking: `repeat=one`.
                let effects = {
                    let mut machine = self.lock();
                    machine.state.track_ended(now)
                };
                self.perform_advance(effects, id, now, work);
            }
            self.emit_change();
            return;
        }
        let effects = {
            let mut machine = self.lock();
            machine.state.track_ended(now)
        };
        self.perform_advance(effects, id, now, work);
    }

    /// Carries out an advance the ENGINE made: the `Load` for the row it is
    /// already playing is skipped (a reload would restart it), every other
    /// effect happens as usual.
    fn perform_advance(&self, effects: Vec<Effect>, started: &str, now: u64, work: &mut Work) {
        let mut rest = Vec::new();
        for effect in effects {
            match effect {
                Effect::Load { entry, .. } if entry.reference() == Some(started) => {
                    let mut machine = self.lock();
                    machine.state.set_position(0.0);
                    machine.faults.started();
                    machine.error = None;
                    machine.retrying = false;
                    machine.bump();
                }
                other => rest.push(other),
            }
        }
        self.perform(rest, now, Some(work));
    }

    fn on_error(
        &self,
        id: Option<String>,
        cause: Option<nz_audio::SourceError>,
        now: u64,
        work: &mut Work,
    ) {
        let Some(id) = id else {
            // A failure with no row (the output device): a cause, never a skip.
            self.publish_fault(Faults::cause_text(cause.as_ref()));
            return;
        };
        // Is this row still the one the MACHINE names? The engine's own
        // `current_id` is empty for a row that never opened (a 404, a dead
        // link), which is exactly the failure the policy must act on.
        let still_current = {
            let machine = self.lock();
            machine
                .state
                .current()
                .and_then(|entry| entry.reference())
                .map(|reference| reference == id)
                .unwrap_or(false)
        };
        if !still_current {
            // A late failure for a row that is no longer current: the person's
            // own click, not a verdict (`decode::fail`'s rule).
            tracing::debug!(id = %id, "a failure for a row that is no longer playing");
            return;
        }
        let resolver =
            cause.unwrap_or_else(|| nz_audio::SourceError::Message("Titre illisible".to_owned()));
        self.opener.failures().record(&id, resolver.clone());
        let (has_next, repeat_one) = {
            let machine = self.lock();
            let index = machine.state.index();
            (
                index + 1 < machine.state.len() as i64,
                machine.state.repeat() == Repeat::One,
            )
        };
        {
            // The row's voice never opened: nothing is pending for the Started
            // any more, or a retry would collapse into a dead load.
            let mut machine = self.lock();
            if machine.pending_load.as_deref() == Some(id.as_str()) {
                machine.pending_load = None;
            }
        }
        let action = {
            let mut machine = self.lock();
            machine.faults.failed(Some(&resolver), has_next, repeat_one)
        };
        let text = Faults::cause_text(Some(&resolver));
        match action {
            fault::Action::Retry(delay) => {
                {
                    let mut machine = self.lock();
                    machine.error = Some(text);
                    machine.bump();
                }
                work.retry_at = Some(now.saturating_add(delay.as_millis() as u64));
                work.retrying = Some(id);
                self.lock().retrying = true;
                self.emit_change();
            }
            fault::Action::Skip => {
                self.publish_fault(text.clone());
                let effects = {
                    let mut machine = self.lock();
                    machine.fault = Some(text);
                    machine.queue_version += 1;
                    machine.state.apply(nz_player::Command::next(), now)
                };
                self.perform_advance(effects, &id, now, work);
            }
            fault::Action::Stall => {
                self.publish_fault(text.clone());
                let effects = {
                    let mut machine = self.lock();
                    machine.error = Some(text.clone());
                    machine.fault = Some(text);
                    machine.state.apply(nz_player::Command::pause(), now)
                };
                self.perform(effects, now, Some(work));
            }
        }
    }

    /// The housekeeping tick: the sleep timer, an episode's position, a link
    /// that came back, a promised retry, the queue's own write.
    fn tick(&self, now: u64, work: &mut Work) {
        let expired = {
            let mut machine = self.lock();
            match machine.sleep {
                Some(sleep) if sleep.expired(now) => {
                    machine.sleep = None;
                    machine.bump();
                    true
                }
                _ => false,
            }
        };
        if expired {
            let effects = {
                let mut machine = self.lock();
                machine.state.apply(nz_player::Command::pause(), now)
            };
            self.perform(effects, now, Some(work));
            self.apply_volume();
            self.emit_change();
        }
        if let Some(at) = work.retry_at
            && now >= at
        {
            work.retry_at = None;
            self.retry_current(now, work);
        }
        // An episode's position while it plays (a pause and a quit write it
        // through their own paths).
        self.report_progress(now, work, false);
        // A link that came back: the positions the phone could not send.
        if self.core.network().is_online() && !work.reports.mirror().is_empty() {
            work.reports.flush_pending();
        }
        if now.saturating_sub(work.persist_at) >= PERSIST_EVERY_MS {
            self.persist(work);
        }
    }

    fn retry_current(&self, now: u64, work: &mut Work) {
        let entry = {
            let machine = self.lock();
            machine.state.current().cloned()
        };
        if let Some(entry) = entry {
            let position = self.engine.position();
            self.load(&entry, true, position, now, Some(work));
        }
    }

    /// An episode's position: every 30 s while it plays, on a pause, at a track
    /// change and at quit (`savePodcastProgress`).
    fn report_progress(&self, now: u64, work: &mut Work, force: bool) {
        let entry = {
            let machine = self.lock();
            machine.state.current().cloned()
        };
        let Some(entry) = entry else {
            return;
        };
        if !entry.is_podcast() {
            return;
        }
        let Some(reference) = entry.reference().map(str::to_owned) else {
            return;
        };
        let playing = self.playing.load(Ordering::SeqCst);
        if !force && (!playing || now.saturating_sub(work.progress_at) < listen::SYNC_EVERY_MS) {
            return;
        }
        work.progress_at = now;
        let position = self.engine.position();
        let duration = f64::from(entry.duration);
        // A clip shorter than the done tail would be "finished" at its first
        // second: only a real episode length can be near its end.
        if duration > listen::DONE_TAIL_S && position >= duration - listen::DONE_TAIL_S {
            work.reports.podcast_finished(&reference, duration, now);
        } else {
            work.reports
                .podcast_position(&reference, position, duration, now);
        }
    }

    fn persist(&self, work: &mut Work) {
        let position = self.engine.position();
        // The engine's clock belongs to the row IT names. At a handover the
        // machine can already name the next row while the outgoing voice still
        // reports its end (44.5 s of a 45 s title), and saving that pair made the
        // next launch resume the NEXT row at the PREVIOUS row's end — a phone
        // that reopened on "Reprendre" skipped 44 s of a title. A position from
        // another row is no position: it is 0.
        let engine_row = self.engine.current_id();
        let kept = {
            let mut machine = self.lock();
            if let Some(current) = machine.state.current() {
                let machine_row = current.reference().map(str::to_owned);
                let at = if engine_row.is_some() && engine_row == machine_row {
                    position
                } else {
                    0.0
                };
                machine.state.set_position(at);
            }
            Store::new(self.core.cache()).save_queue(&machine.state)
        };
        work.persist_at = self.now_ms();
        tracing::debug!(kept, "queue persisted");
    }

    /// A failure the person must be told about, on the worker thread and
    /// holding no lock.
    fn publish_fault(&self, cause: String) {
        for sink in self.listeners_snapshot() {
            sink.on_fault(cause.clone());
        }
    }

    /// Pushes the snapshot to every listener, in order. **The machine's lock is
    /// not held while calling them**; the delivery lock is, so a listener must
    /// not push (`set_audio`, `set_ducked`...) from inside its own callback —
    /// the FFI boundary's rule already says a listener writes its state and
    /// returns.
    fn emit_change(&self) {
        // Held across the take AND the calls, so two pushes reach every
        // listener in the order their snapshots were taken. The machine's lock
        // is NOT held (`snapshot` takes and releases it), and a listener only
        // writes its state and returns (the FFI boundary's rule).
        let _delivery = self.delivery.lock().unwrap_or_else(|err| err.into_inner());
        let snapshot = self.snapshot();
        for sink in self.listeners_snapshot() {
            sink.on_snapshot(snapshot.clone());
        }
    }

    fn emit_queue(&self, version: u64) {
        for sink in self.listeners_snapshot() {
            sink.on_queue_changed(version);
        }
    }

    fn listeners_snapshot(&self) -> Vec<Arc<dyn Sink>> {
        self.listeners
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    /// A settings change that can move the sound of what is PLAYING.
    ///
    /// Normalisation off at the moment a row opens asks the server NOTHING (rule
    /// 1: no extra request on the play path), so a voice built then carries no
    /// loudness and the engine's gain stays at 0 dB. A person changing the level
    /// while the music plays is the one mid-track change that may move the level
    /// — the SPA's own rule (`Player.svelte#onNormalizationChange`, "the one
    /// mid-track change the [static-gain] rule doesn't apply to: the user asked
    /// for it") — so the loudness is asked for HERE, on the `User` lane, and the
    /// voice already playing takes it.
    ///
    /// Only the row playing and the preloaded next: everything else is primed by
    /// [`Player::prime`] when it plays, so one switch never becomes a queue of
    /// requests. A level change alone — the loudness already known — costs
    /// nothing: the engine's gain follows the live level on its own.
    fn audio_changed(&self, work: &mut Work) {
        let (normalization, index, queue) = {
            let machine = self.lock();
            (
                machine.audio.normalization,
                usize::try_from(machine.state.index()).ok(),
                machine.state.queue().to_vec(),
            )
        };
        let Some(index) = index else {
            // Nothing has ever played: the row that does is primed by its load.
            return;
        };
        if normalization == Normalization::Off {
            return;
        }
        let Some(current) = queue.get(index).cloned() else {
            return;
        };
        let Some(reference) = current.reference().map(str::to_owned) else {
            return;
        };
        // Already asked (this row's own load, or an earlier change): nothing was
        // skipped, the voice carries it and the live level is enough.
        if work.loudness.contains_key(&reference) {
            return;
        }
        self.prime(&current, work, true);
        let meta = work.meta.get(&reference).copied().unwrap_or_default();
        self.engine.set_track_meta(&reference, meta);
        self.engine.set_current_loudness(&reference, meta.loudness);
        // The preloaded next row was built before this answer existed: rebuild it
        // so the handover is normalised too (it is not playing, so this is free).
        if let Some(next) = queue.get(index + 1).cloned() {
            self.preload(Some(&next), work);
        }
    }

    /// Applies the machine's audio settings to the engine. Idempotent, and cheap
    /// enough to call on every change.
    fn apply_audio(&self) {
        let (audio, volume, muted) = {
            let now = self.now_ms();
            let machine = self.lock();
            (
                machine.audio.clone(),
                machine.volume(now),
                machine.state.muted(),
            )
        };
        self.engine.set_normalization(audio.normalization.audio());
        for (band, db) in audio.eq_db.iter().enumerate() {
            self.engine.set_eq(band, *db);
        }
        self.engine.set_bass(audio.bass_db);
        self.engine.set_mono(audio.mono);
        self.engine.set_balance(audio.balance);
        self.engine.set_speed(audio.speed);
        self.engine.set_fade(nz_audio::FadeSettings {
            enabled: audio.crossfade_s > 0.0,
            seconds: f64::from(audio.crossfade_s),
            ..Default::default()
        });
        self.engine.set_volume(volume);
        self.engine.set_mute(muted);
    }

    fn apply_volume(&self) {
        let (volume, muted) = {
            let now = self.now_ms();
            let machine = self.lock();
            (machine.volume(now), machine.state.muted())
        };
        self.engine.set_volume(volume);
        self.engine.set_mute(muted);
    }

    fn set_read_ahead(&self, screen_on: bool) {
        let bytes = if screen_on {
            READ_AHEAD_ON_BYTES
        } else {
            READ_AHEAD_OFF_BYTES
        };
        self.engine.set_read_ahead_bytes(bytes);
    }

    fn fading(&self) -> bool {
        let now = self.now_ms();
        self.lock()
            .sleep
            .map(|sleep| sleep.fading(now))
            .unwrap_or(false)
    }

    fn effective_quality(&self) -> Quality {
        let machine = self.lock();
        self.quality_locked(&machine)
    }

    fn quality_locked(&self, machine: &Machine) -> Quality {
        let bits = self.rate.load(Ordering::SeqCst);
        let rate = (bits != 0).then(|| f64::from_bits(bits));
        let asked = if machine.audio.quality == Quality::Auto {
            rung_of(rate)
        } else {
            machine.audio.quality
        };
        let metered = self.core.network().metered();
        if metered {
            asked.lower(machine.audio.metered_cap)
        } else {
            asked
        }
    }

    fn now_ms(&self) -> u64 {
        // The player's own clock, in milliseconds since it started: what a
        // listen ledger and a sleep deadline need, never a wall-clock instant a
        // device time change could jump.
        self.start.elapsed().as_millis() as u64
    }

    fn runtime_handle(&self) -> tokio::runtime::Handle {
        let runtime = self
            .runtime
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        match runtime {
            Some(runtime) => runtime.handle().clone(),
            // Only reachable before `open` finishes or after `shutdown`, and no
            // caller asks for work then.
            None => tokio::runtime::Handle::current(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Machine> {
        self.locks.fetch_add(1, Ordering::Relaxed);
        TAKEN.with(|taken| taken.set(taken.get() + 1));
        self.machine.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn lock_route(&self) -> MutexGuard<'_, AudioRoute> {
        self.route.lock().unwrap_or_else(|err| err.into_inner())
    }
}

/// The numbers Aide and the journal show. `performance_mode` and `buffer_ms` are
/// the PLATFORM's (`DroidOutput`); a host build leaves them empty.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OutputStats {
    pub underruns: u64,
    pub rebuffers: u64,
    pub buffer_ms: u32,
    pub sample_rate: u32,
    pub latency_ms: u32,
    pub performance_mode: String,
    pub last_tap_ns: u64,
    pub last_first_audible_ns: u64,
}

/// The caller's rows, as the queue's own entries. A row with no reference the
/// server accepts in a path is dropped (one unplayable row must not make a whole
/// queue unreadable — `nz-player`'s `clean` does the same).
fn entries_of(tracks: Vec<TrackRef>) -> Vec<Entry> {
    tracks
        .into_iter()
        .filter_map(|track| track.entry())
        .collect()
}

fn to_audio_quality(quality: Quality) -> nz_audio::Quality {
    quality.start()
}

fn api_quality(quality: nz_audio::Quality) -> nz_api::Quality {
    match quality {
        nz_audio::Quality::Flac => nz_api::Quality::Flac,
        nz_audio::Quality::Opus320 => nz_api::Quality::Opus320,
        nz_audio::Quality::Opus128 => nz_api::Quality::Opus128,
        nz_audio::Quality::Opus64 => nz_api::Quality::Opus64,
    }
}

/// Turns a queued `Entry` into the row a `ListHandle` draws.
fn row_of(entry: &Entry, position: usize) -> crate::lists::Row {
    crate::lists::Row {
        key: entry.key().to_owned(),
        id: entry.reference().unwrap_or_default().to_owned(),
        title: entry.title.clone(),
        artists: {
            let names = entry
                .artists
                .iter()
                .map(|artist| artist.name.as_str())
                .filter(|name| !name.is_empty())
                .collect::<Vec<_>>()
                .join(", ");
            if names.is_empty() {
                entry.artist_name().to_owned()
            } else {
                names
            }
        },
        album: entry
            .album
            .as_ref()
            .map(|album| album.title.clone())
            .unwrap_or_default(),
        album_id: entry
            .album
            .as_ref()
            .and_then(|album| album.id.clone().or_else(|| album.deezer_id.clone())),
        duration_s: entry.duration,
        cover: entry.cover.clone(),
        explicit: entry.is_explicit(),
        liked: false,
        disliked: false,
        unavailable: entry.is_unavailable(),
        downloaded: entry.is_local(),
        source: if entry.is_local() {
            "local".to_owned()
        } else {
            "deezer".to_owned()
        },
        position: position as u32,
        played_at: 0,
        added_at: 0,
    }
}

/// The counted bytes of one row: its own size plus the bytes of every string it
/// owns — what "a 4 000-row queue costs ≤ 4 MB" measures (worker rule 4: counted
/// bytes, never a process's RSS).
fn entry_bytes(entry: &Entry) -> u64 {
    let mut total = std::mem::size_of::<Entry>() as u64;
    total += entry.id.len() as u64 + entry.title.len() as u64;
    if let Some(display) = &entry.display_artist {
        total += display.len() as u64;
    }
    if let Some(deezer) = &entry.deezer_id {
        total += deezer.len() as u64;
    }
    if let Some(cover) = &entry.cover {
        total += cover.len() as u64;
    }
    for artist in &entry.artists {
        total += std::mem::size_of::<nz_player::EntryArtist>() as u64;
        total += artist.name.len() as u64;
        if let Some(id) = &artist.id {
            total += id.len() as u64;
        }
        if let Some(id) = &artist.deezer_id {
            total += id.len() as u64;
        }
    }
    if let Some(album) = &entry.album {
        total += std::mem::size_of::<nz_player::EntryAlbum>() as u64;
        total += album.title.len() as u64;
        if let Some(cover) = &album.cover {
            total += cover.len() as u64;
        }
    }
    total
}

/// `radio:track:1234` → (`track`, `1234`): the shape a radio's context keeps,
/// so a refill asks the seed it started from and not the last title it played.
pub fn split_context(context: &Context) -> (String, String) {
    let id = context.id.clone().unwrap_or_default();
    match id.split_once(':') {
        Some((kind, rest)) => (kind.to_owned(), rest.to_owned()),
        None => ("track".to_owned(), id),
    }
}

/// The core's error, as the sentence the person reads: a server that is not
/// configured is a cause, not a code.
pub fn core_cause(error: &CoreError) -> String {
    match error {
        CoreError::Offline => "Hors ligne".to_owned(),
        CoreError::Timeout => "Le serveur ne répond pas".to_owned(),
        CoreError::Session => "Session expirée".to_owned(),
        CoreError::RemoteEnded { .. } => "Télécommande terminée".to_owned(),
        CoreError::NotReady { retry_after_s } => {
            format!("Pas encore prêt (réessayer dans {retry_after_s} s)")
        }
        CoreError::Conflict { detail }
        | CoreError::Provider { detail, .. }
        | CoreError::Server { detail, .. }
        | CoreError::Invalid { detail }
        | CoreError::Internal { detail } => detail.clone(),
        CoreError::Certificate { changed, .. } => {
            if *changed {
                "Le certificat du serveur a changé".to_owned()
            } else {
                "Certificat du serveur refusé".to_owned()
            }
        }
        CoreError::Unsupported { feature } => {
            format!("Fonction absente de ce serveur : {feature}")
        }
    }
}
