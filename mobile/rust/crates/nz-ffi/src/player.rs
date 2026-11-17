//! The player's surface: the queue, the engine and every playback rule, behind
//! `PlayerApi` (docs/agents/mobile.md, "MB3b"). Owned by **MB3b**.
//!
//! The types below are the ones the note pins as SHARED — MB3c builds its
//! `NzPlayer` facade from [`PlayerSnapshot`], MB11 reads
//! [`PlayerApi::position`] from the progress bar, MB12 sends
//! [`AudioSettings`] and reads [`OutputStats`]. MB0 declared them in their final
//! shape and every body answers an empty value: the calls are infallible by
//! design (a transport command never blocks or fails its caller — it enqueues),
//! so there is no `NzError` to return here.

use std::sync::Arc;

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

/// How loudness is normalised (ReplayGain). The variants are MB3b's to confirm
/// against `webapp/src/lib/gaincache.js` — the note pins the FIELD, not its
/// values.
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Normalization {
    Off,
    ReplayGain,
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
    /// Minutes left, or `0` for "at the end of this track".
    pub minutes: u32,
    pub at_end_of_track: bool,
}

/// A track the caller names in a queue edit: the key the row carries and the id
/// the server knows. One FFI call carries up to 4 000 of these (≈ 300 KB).
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct TrackRef {
    pub key: String,
    pub id: String,
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
    /// `auto`, `320`, `128`, `64`, `flac` — what the badge shows.
    pub quality: String,
    /// `deezer`, `youtube`, `upload`, `local`, `cache`, `download`.
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
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct AudioRoute {
    /// `bluetooth`, `usb`, `wired`, `hdmi`, `speaker`.
    pub kind: String,
    pub name: String,
    pub conn: String,
}

/// What the audio path did — the numbers Aide and the journal show (android.md
/// bridge 12: underruns and rebuffers need opposite fixes).
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq, Default)]
pub struct OutputStats {
    /// The mixer's starvations plus the device's xruns (one number: both are
    /// glitches the person heard).
    pub underruns: u64,
    pub rebuffers: u64,
    /// The buffer the device really opened.
    pub buffer_ms: u32,
    pub sample_rate: u32,
    pub latency_ms: u32,
    /// What AAudio actually granted (`PowerSaving`, or what it fell back to).
    pub performance_mode: String,
    /// Kotlin's tap instant and the first rendered frame above −60 dBFS, in
    /// nanoseconds: the difference is "tap → sound", read from events.
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
    /// `album`, `playlist`, `artist`, `favorites`, `flow`…
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
    /// The queue changed (an edit, a shuffle, an advance).
    fn on_queue_changed(&self, version: u64);
    /// A title could not be played; `cause` is a French sentence. The list goes
    /// on: a title that cannot be played costs that title, not the list.
    fn on_fault(&self, cause: String);
}

/// The player. MB3b fills it; every body below answers an empty value, which is
/// what "nothing plays yet" honestly is.
#[derive(uniffi::Object)]
pub struct PlayerApi;

// No `async_runtime` here: every one of these is synchronous by design (a
// command enqueues, a read is an atomic). UniFFI refuses the attribute on an
// impl block with no async method — measured.
#[uniffi::export]
impl PlayerApi {
    /// Enqueues a command and returns: the caller (a Compose click handler)
    /// never waits on the network, a lock or the engine.
    pub fn command(&self, cmd: PlayerCommand, tap_ns: u64) {
        tracing::debug!(?cmd, tap_ns, "player command (MB0 skeleton: dropped)");
    }

    /// The whole picture, cheap to copy.
    pub fn snapshot(&self) -> PlayerSnapshot {
        PlayerSnapshot {
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
            error: None,
            fault: None,
            version: 0,
        }
    }

    /// The hot read: atomics only. The progress bar calls this each visible
    /// frame and NOTHING while the screen is off.
    pub fn position(&self) -> PositionInfo {
        PositionInfo {
            position_s: 0.0,
            duration_s: 0.0,
            buffered_s: 0.0,
            playing: false,
        }
    }

    /// The queue as a windowed handle (4 000 entries never cross whole).
    pub fn queue(&self) -> Arc<ListHandle> {
        crate::lists::open(Vec::new())
    }

    pub fn subscribe(&self, listener: Box<dyn PlayerListener>) {
        drop(listener);
    }

    pub fn audio(&self) -> AudioSettings {
        AudioSettings {
            quality: Quality::Auto,
            crossfade_s: 0.0,
            trim_silence: false,
            normalization: Normalization::Off,
            eq_db: vec![0.0; 10],
            eq_preset: None,
            bass_db: 0.0,
            mono: false,
            balance: 0.0,
            speed: 1.0,
            metered_cap: Quality::Opus128,
        }
    }

    pub fn set_audio(&self, s: AudioSettings) {
        tracing::debug!(quantum = s.eq_db.len(), "set_audio (MB0 skeleton)");
    }

    pub fn output_stats(&self) -> OutputStats {
        OutputStats::default()
    }

    /// Screen off raises the read-ahead to 32 MB (the radio sleeps between
    /// bursts); screen on brings it back to 2 MB (rule 1).
    pub fn set_screen_on(&self, on: bool) {
        tracing::debug!(on, "set_screen_on (MB0 skeleton)");
    }

    pub fn set_route(&self, route: AudioRoute) {
        tracing::debug!(kind = %route.kind, "set_route (MB0 skeleton)");
    }

    pub fn set_sleep(&self, s: Option<SleepTimer>) {
        tracing::debug!(?s, "set_sleep (MB0 skeleton)");
    }

    /// Audio focus said "can duck": the ramp goes to 0.2 and back.
    pub fn set_ducked(&self, ducked: bool) {
        tracing::debug!(ducked, "set_ducked (MB0 skeleton)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nothing plays, and asking changes nothing: the skeleton's promises are
    /// empty values, not panics.
    #[test]
    fn an_empty_player_answers_empty_values() {
        let api = PlayerApi;
        let snap = api.snapshot();
        assert_eq!(snap.state, PlayState::Stopped);
        assert!(snap.current.is_none());
        assert_eq!(snap.version, 0);
        assert_eq!(api.position().position_s, 0.0);
        assert_eq!(api.output_stats().underruns, 0);
        assert_eq!(api.audio().eq_db.len(), 10);
        assert_eq!(api.queue().len(), 0);

        api.command(PlayerCommand::Play, 0);
        api.set_screen_on(false);
        api.set_ducked(true);
        api.set_sleep(Some(SleepTimer::EndOfTrack));
    }
}
