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

use std::sync::{Arc, OnceLock};

use nz_core::player as core;
use nz_core::player::{Player, PlayerError, Sink};

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

    pub fn output_stats(&self) -> OutputStats {
        match self.player() {
            Some(player) => {
                let stats = player.output_stats();
                OutputStats {
                    underruns: stats.underruns,
                    rebuffers: stats.rebuffers,
                    buffer_ms: stats.buffer_ms,
                    sample_rate: stats.sample_rate,
                    latency_ms: stats.latency_ms,
                    performance_mode: stats.performance_mode,
                    last_tap_ns: stats.last_tap_ns,
                    last_first_audible_ns: stats.last_first_audible_ns,
                }
            }
            None => OutputStats::default(),
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
