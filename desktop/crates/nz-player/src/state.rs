//! The queue and the transport: one struct, one `apply`, and the rules the web
//! player already taught us.
//!
//! Every rule below is `webapp/src/lib/stores.js`'s `player` store, ported with
//! the same names and the same edge cases: a queue chosen one title at a time
//! (the Flow) tops itself up, `prev` restarts the track after 3 s, a shuffle
//! keeps the playing row first and gives the original order back, removing a row
//! fixes the index instead of shifting the wrong track, and a hand-made reorder
//! never restarts the music. The one deliberate difference is written down where
//! it happens (`removeAt` keeps the un-shuffle order in step, so a removed row
//! cannot come back).

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::command::{
    Command, Context, IndexArgs, MAX_ADD, MAX_QUEUE, MoveArgs, QueueArgs, ShuffleArgs,
};
use crate::effect::Effect;
use crate::listen::ListenLedger;
use crate::shuffle::{Rng, shuffled_positions};

/// One queue row: what the lists and the player draw, and nothing else.
///
/// This is `webapp/src/lib/remote/commands.js#slimTrack`, key by key, because
/// that is what crosses when a queue is handed to this device: the playing
/// title's art, its artist line, its length, its gain and the flags the lists
/// grey a row with. Everything else (credits, lyrics, the album's notes) is
/// fetched by the screen that shows it.
///
/// `id` is the queue KEY, not necessarily a row's UUID: a catalogue result that
/// the library does not hold yet carries the uuid5 its row will have (see
/// [`crate::track_key`]), which is what lets a search result and the row it
/// becomes be the same queue entry.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    #[serde(default, deserialize_with = "loose_string")]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deezer_id: Option<String>,
    #[serde(default, deserialize_with = "loose_string")]
    pub title: String,
    /// Seconds, as the server counts them.
    #[serde(default)]
    pub duration: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explicit: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub podcast: Option<bool>,
    /// ReplayGain in dB, when the server has measured it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    /// Relative to the server (`/api/cover/<id>`), never an absolute URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_artist: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<EntryArtist>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artists: Vec<EntryArtist>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<EntryAlbum>,
}

/// `{"id", "deezer_id", "name"}`: a row's performer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryArtist {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deezer_id: Option<String>,
    #[serde(default, deserialize_with = "loose_string")]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

/// `{"id", "deezer_id", "title", "cover"}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryAlbum {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deezer_id: Option<String>,
    #[serde(default, deserialize_with = "loose_string")]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
}

/// A `null` where a string is expected reads as absent, never as a parse error:
/// a queue carrying a row the server could not name must not make the whole
/// command unreadable.
fn loose_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

impl Entry {
    /// A row with only its key, for a test or a placeholder.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            ..Self::default()
        }
    }

    /// The queue key: what de-duplication, un-shuffle and a remote `jump` use.
    pub fn key(&self) -> &str {
        &self.id
    }

    /// What a request names this track by (`trackkey.js#trackRef`): its row id,
    /// else the Deezer id.
    pub fn reference(&self) -> Option<&str> {
        crate::track_ref(Some(&self.id), self.deezer_id.as_deref())
    }

    /// Whether the row can stand in a URL path segment: the server drops any
    /// other (`commands.js#playableId`). One unplayable row must not make a
    /// whole queue unreadable, it is filtered out.
    pub fn is_playable(&self) -> bool {
        let id = self.id.as_bytes();
        !id.is_empty()
            && id.len() <= 64
            && id
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
    }

    pub fn is_explicit(&self) -> bool {
        self.explicit.unwrap_or(false)
    }

    pub fn is_local(&self) -> bool {
        self.local.unwrap_or(false)
    }

    pub fn is_podcast(&self) -> bool {
        self.podcast.unwrap_or(false)
    }

    pub fn is_unavailable(&self) -> bool {
        self.unavailable.unwrap_or(false)
    }

    pub fn is_archived(&self) -> bool {
        self.archived.unwrap_or(false)
    }

    /// The one line a list shows for the performers.
    pub fn artist_name(&self) -> &str {
        self.display_artist
            .as_deref()
            .filter(|s| !s.is_empty())
            .or_else(|| self.artist.as_ref().map(|a| a.name.as_str()))
            .unwrap_or("")
    }
}

/// How the queue repeats.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

impl Repeat {
    /// off → all → one → off, the cycle the `r` key walks (`stores.js`).
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }
}

/// What survives a restart: the queue, where it was, and the transport's
/// settings. `orig` is the un-shuffle order as POSITIONS in `queue`, so a 4 000
/// row queue stays a few kilobytes instead of doubling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub queue: Vec<Entry>,
    pub index: i64,
    #[serde(default)]
    pub position: f64,
    #[serde(default)]
    pub shuffle: bool,
    #[serde(default)]
    pub repeat: Repeat,
    #[serde(default = "autoplay_on")]
    pub autoplay: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<Context>,
    #[serde(default = "full_volume")]
    pub volume: f64,
    #[serde(default)]
    pub muted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orig: Option<Vec<u32>>,
}

fn autoplay_on() -> bool {
    true
}

fn full_volume() -> f64 {
    1.0
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            queue: Vec::new(),
            index: -1,
            position: 0.0,
            shuffle: false,
            repeat: Repeat::Off,
            autoplay: true,
            context: None,
            volume: 1.0,
            muted: false,
            orig: None,
        }
    }
}

/// The state of one apply before it, so the effects can say what changed.
struct Before {
    current: Option<Arc<Entry>>,
    index: i64,
    playing: bool,
    position: f64,
    seq: u64,
    version: u64,
    volume: f64,
    muted: bool,
    shuffle: bool,
    repeat: Repeat,
    context: Option<Context>,
    autoplay: bool,
}

/// The queue, the transport and the rules of the web player.
///
/// Rows are `Arc<Entry>` so a shuffle, an un-shuffle or a `move` reorders
/// pointers instead of copying a 4 000-row library: `toggleShuffle` on 4 000
/// rows has to answer inside a frame (measured in `tests/perf.rs`).
pub struct PlayerState {
    queue: Vec<Arc<Entry>>,
    /// The playing row, `-1` when there is none (an empty queue).
    index: i64,
    playing: bool,
    current_time: f64,
    duration: f64,
    seq: u64,
    volume: f64,
    muted: bool,
    shuffle: bool,
    repeat: Repeat,
    autoplay: bool,
    context: Option<Context>,
    orig: Option<Vec<Arc<Entry>>>,
    /// Bumped by every change of the queue itself.
    version: u64,
    ledger: ListenLedger,
    /// The key of the row announced by the last `PreloadNext`.
    preloaded: Option<String>,
    /// The key of the row a `NeedMore` was already asked for.
    asked_more: Option<String>,
    rng: Rng,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self::new()
    }
}

impl PlayerState {
    pub fn new() -> Self {
        Self {
            queue: Vec::new(),
            index: -1,
            playing: false,
            current_time: 0.0,
            duration: 0.0,
            seq: 0,
            volume: 1.0,
            muted: false,
            shuffle: false,
            repeat: Repeat::Off,
            autoplay: true,
            context: None,
            orig: None,
            version: 0,
            ledger: ListenLedger::default(),
            preloaded: None,
            asked_more: None,
            rng: Rng::from_entropy(),
        }
    }

    /// Rebuilds a saved session. The transport comes back PAUSED — audio never
    /// starts without a click (`stores.js`: `playing: false`) — and the
    /// [`Effect::Load`] says where the playhead was.
    pub fn restore(snapshot: Snapshot) -> (Self, Vec<Effect>) {
        let entries: Vec<Arc<Entry>> = snapshot.queue.into_iter().map(Arc::new).collect();
        let orig = snapshot.orig.as_ref().and_then(|positions| {
            let mut out = Vec::with_capacity(positions.len());
            for position in positions {
                out.push(entries.get(*position as usize)?.clone());
            }
            Some(out)
        });
        let index = if entries.is_empty() {
            -1
        } else {
            snapshot.index.clamp(0, entries.len() as i64 - 1)
        };
        let duration = entries
            .get(index.max(0) as usize)
            .map(|e| e.duration as f64)
            .unwrap_or(0.0);
        let mut state = Self {
            queue: entries,
            index,
            playing: false,
            current_time: snapshot.position.max(0.0),
            duration,
            seq: 0,
            volume: snapshot.volume,
            muted: snapshot.muted,
            shuffle: snapshot.shuffle,
            repeat: snapshot.repeat,
            autoplay: snapshot.autoplay,
            context: snapshot.context,
            orig,
            version: 0,
            ledger: ListenLedger::default(),
            preloaded: None,
            asked_more: None,
            rng: Rng::from_entropy(),
        };
        let key = state.current_arc().map(|e| e.key().to_owned());
        state.ledger.start(key, false, 0);

        let mut effects = Vec::new();
        if let Some(entry) = state.current_arc() {
            effects.push(Effect::Load {
                entry: (**entry).clone(),
                autoplay: false,
                start_s: state.current_time,
            });
        }
        let next = state.next_entry().cloned();
        state.preloaded = next.as_ref().map(|e| e.key().to_owned());
        effects.push(Effect::PreloadNext(next));
        if state.current_arc().is_some() {
            effects.push(Effect::Changed);
        }
        (state, effects)
    }

    pub fn snapshot(&self) -> Snapshot {
        let orig = self.orig.as_ref().and_then(|orig| {
            let mut positions = Vec::with_capacity(orig.len());
            for entry in orig {
                let at = self.queue.iter().position(|e| Arc::ptr_eq(e, entry))?;
                positions.push(at as u32);
            }
            Some(positions)
        });
        Snapshot {
            queue: self.queue.iter().map(|e| (**e).clone()).collect(),
            index: self.index,
            position: self.current_time,
            shuffle: self.shuffle,
            repeat: self.repeat,
            autoplay: self.autoplay,
            context: self.context.clone(),
            volume: self.volume,
            muted: self.muted,
            orig,
        }
    }

    /// Seeds the shuffle, so a bench is reproducible. Production seeds itself.
    pub fn set_shuffle_seed(&mut self, seed: u64) {
        self.rng = Rng::new(seed);
    }

    pub fn queue(&self) -> &[Arc<Entry>] {
        &self.queue
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn index(&self) -> i64 {
        self.index
    }

    pub fn current(&self) -> Option<&Entry> {
        self.current_arc().map(|e| &**e)
    }

    /// What follows the playing row, if the queue has one.
    pub fn next_entry(&self) -> Option<&Entry> {
        if self.index < 0 {
            return None;
        }
        self.queue.get(self.index as usize + 1).map(|e| &**e)
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    pub fn current_time(&self) -> f64 {
        self.current_time
    }

    pub fn duration(&self) -> f64 {
        self.duration
    }

    pub fn seq(&self) -> u64 {
        self.seq
    }

    pub fn volume(&self) -> f64 {
        self.volume
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn repeat(&self) -> Repeat {
        self.repeat
    }

    pub fn autoplay(&self) -> bool {
        self.autoplay
    }

    pub fn context(&self) -> Option<&Context> {
        self.context.as_ref()
    }

    /// The engine's own position, at the 4 Hz the bar polls: it drives `prev`'s
    /// "restart after 3 s" and the seek clamp, and it never changes the state
    /// machine's own bookkeeping.
    pub fn set_position(&mut self, at_s: f64) {
        self.current_time = if at_s.is_nan() { 0.0 } else { at_s.max(0.0) };
    }

    /// The length the engine measured, when the element disagrees with the row.
    pub fn set_duration(&mut self, duration_s: f64) {
        if duration_s.is_finite() && duration_s > 0.0 {
            self.duration = duration_s;
        }
    }

    /// Does one thing to the transport and returns what the app must do next.
    pub fn apply(&mut self, cmd: Command, now_ms: u64) -> Vec<Effect> {
        self.ledger.close(now_ms);
        let before = self.before();
        let mut explicit = None;
        match cmd {
            Command::Play { .. } => {
                if self.index >= 0 {
                    self.playing = true;
                }
            }
            Command::Pause { .. } => self.playing = false,
            Command::Toggle { .. } => {
                if self.index >= 0 {
                    self.playing = !self.playing;
                }
            }
            Command::Next { .. } => explicit = self.advance(),
            Command::Prev { .. } => explicit = self.back(),
            Command::Seek { args } => explicit = self.seek(args.t),
            Command::Jump { args } => explicit = self.jump(&args),
            Command::RemoveAt { args } => self.remove_at(&args),
            Command::Move { args } => self.move_to(&args),
            Command::ClearUpcoming { .. } => self.clear_upcoming(),
            Command::SetVolume { args } => self.set_volume(args.v),
            Command::ToggleMute { .. } => self.muted = !self.muted,
            Command::ToggleShuffle { .. } => self.toggle_shuffle(),
            Command::CycleRepeat { .. } => self.repeat = self.repeat.next(),
            Command::AddToQueue { args } => self.add_to_queue(args.tracks),
            Command::PlayNext { args } => self.play_next(args.tracks),
            Command::PlayQueue { args } => self.play_queue(args),
            Command::ShufflePlay { args } => self.shuffle_play(args),
        }
        self.finish(before, explicit, now_ms)
    }

    /// A track ended by itself (the engine's `ended`): repeat one restarts it,
    /// the queue advances, repeat all wraps, a Flow/radio asks for more, and
    /// anything else stops — reporting the final listen, exactly as
    /// `Player.svelte#onEnded` does.
    pub fn track_ended(&mut self, now_ms: u64) -> Vec<Effect> {
        self.ledger.close(now_ms);
        let before = self.before();
        let Some(current) = self.current_arc().cloned() else {
            return Vec::new();
        };
        let mut stopped = false;
        let explicit = if self.repeat == Repeat::One {
            self.current_time = 0.0;
            self.seq += 1;
            Some(((*current).clone(), 0.0))
        } else if self.index + 1 < self.queue.len() as i64 {
            self.index += 1;
            self.playing = true;
            self.seq += 1;
            None
        } else if self.repeat == Repeat::All {
            if self.index == 0 {
                // A one-row queue, or already at the top: start it again.
                self.current_time = 0.0;
                self.seq += 1;
                Some(((*current).clone(), 0.0))
            } else {
                self.index = 0;
                self.playing = true;
                self.seq += 1;
                None
            }
        } else {
            self.playing = false;
            self.asked_more = None;
            stopped = true;
            None
        };

        let mut effects = Vec::new();
        if stopped {
            if let Some((id, listened_s)) = self.ledger.take(now_ms) {
                effects.push(Effect::ReportListen {
                    id,
                    listened_s,
                    next_id: None,
                    context: self.context.clone(),
                    shuffle: self.shuffle,
                });
            }
            let key = self.current_arc().map(|e| e.key().to_owned());
            self.ledger.start(key, self.playing, now_ms);
        }
        effects.extend(self.finish(before, explicit, now_ms));
        effects
    }

    /// Reports what the current row heard and starts counting again — the quit
    /// path (`Player.svelte#onDestroy` → `flushListen(null)`).
    pub fn flush_listen(&mut self, now_ms: u64) -> Vec<Effect> {
        self.ledger.close(now_ms);
        let mut effects = Vec::new();
        if let Some((id, listened_s)) = self.ledger.take(now_ms) {
            effects.push(Effect::ReportListen {
                id,
                listened_s,
                next_id: None,
                context: self.context.clone(),
                shuffle: self.shuffle,
            });
        }
        let key = self.current_arc().map(|e| e.key().to_owned());
        self.ledger.start(key, self.playing, now_ms);
        effects
    }

    /// Appends rows, de-duplicated by key, without moving the playhead
    /// (`stores.js#extend`). Returns how many actually landed, so a caller can
    /// tell a top-up that did something from one that was all duplicates — a
    /// radio that answers with the same rows must not spin.
    pub fn extend(&mut self, tracks: Vec<Entry>, now_ms: u64) -> (usize, Vec<Effect>) {
        self.ledger.close(now_ms);
        let extra = Self::clean(tracks, MAX_ADD);
        let have: Vec<&str> = self.queue.iter().map(|e| e.key()).collect();
        let add: Vec<Arc<Entry>> = extra
            .into_iter()
            .filter(|e| !have.contains(&e.key()))
            .map(Arc::new)
            .collect();
        if add.is_empty() {
            return (0, Vec::new());
        }
        let before = self.before();
        let added = add.len();
        self.queue.extend(add.iter().cloned());
        if let Some(orig) = self.orig.as_mut() {
            orig.extend(add);
        }
        self.version += 1;
        (added, self.finish(before, None, now_ms))
    }

    /// Swaps everything AFTER the playing row for `tracks` (the Flow retuned to
    /// a new discovery level): what was heard stays, and the music does not
    /// restart (`stores.js#replaceUpcoming`).
    pub fn replace_upcoming(&mut self, tracks: Vec<Entry>, now_ms: u64) -> Vec<Effect> {
        if self.index < 0 {
            return Vec::new();
        }
        self.ledger.close(now_ms);
        let before = self.before();
        let kept = ((self.index + 1) as usize).min(self.queue.len());
        let have: Vec<String> = self.queue[..kept]
            .iter()
            .map(|e| e.key().to_owned())
            .collect();
        let add: Vec<Arc<Entry>> = Self::clean(tracks, MAX_ADD)
            .into_iter()
            .filter(|e| !have.iter().any(|k| k == e.key()))
            .map(Arc::new)
            .collect();
        self.queue.truncate(kept);
        self.queue.extend(add);
        if self.orig.is_some() {
            self.orig = Some(self.queue.clone());
        }
        self.version += 1;
        self.finish(before, None, now_ms)
    }

    /// Appends and advances at once: what a Flow or radio top-up answers
    /// [`Effect::NeedMore`] with (`stores.js#autoExtend`). The rows play even
    /// when the queue had run out.
    pub fn auto_extend(&mut self, tracks: Vec<Entry>, now_ms: u64) -> Vec<Effect> {
        let extra: Vec<Arc<Entry>> = Self::clean(tracks, MAX_ADD)
            .into_iter()
            .map(Arc::new)
            .collect();
        if extra.is_empty() {
            return Vec::new();
        }
        self.ledger.close(now_ms);
        let before = self.before();
        self.queue.extend(extra.iter().cloned());
        if let Some(orig) = self.orig.as_mut() {
            orig.extend(extra);
        }
        self.index = (self.index + 1).min(self.queue.len() as i64 - 1);
        self.playing = true;
        self.seq += 1;
        self.version += 1;
        self.asked_more = None;
        self.finish(before, None, now_ms)
    }

    /// Re-labels what is playing without touching the queue.
    pub fn set_context(&mut self, context: Option<Context>, now_ms: u64) -> Vec<Effect> {
        if self.context == context {
            return Vec::new();
        }
        self.ledger.close(now_ms);
        let before = self.before();
        self.context = context;
        self.finish(before, None, now_ms)
    }

    // -- the rules ---------------------------------------------------------

    /// `player.next()`: forward, wrap on repeat-all, else the end of the queue.
    fn advance(&mut self) -> Option<(Entry, f64)> {
        let current = self.current_arc()?.clone();
        let was = self.index;
        if self.index + 1 < self.queue.len() as i64 {
            self.index += 1;
            self.playing = true;
            self.seq += 1;
        } else if self.repeat == Repeat::All && !self.queue.is_empty() {
            self.index = 0;
            self.playing = true;
            self.seq += 1;
        } else {
            // The end: a Flow or a radio can name what comes next, so ask — the
            // app answers with `auto_extend`. Anything else stops here.
            self.asked_more = None;
            self.playing = false;
            return None;
        }
        // A one-row queue wrapping onto itself is a restart, not a no-op.
        if self.index == was {
            Some(((*current).clone(), 0.0))
        } else {
            None
        }
    }

    /// `player.prev()`: restart the track past 3 s, else step back, else restart.
    fn back(&mut self) -> Option<(Entry, f64)> {
        let current = self.current_arc()?.clone();
        if self.current_time > 3.0 {
            self.current_time = 0.0;
            self.seq += 1;
            return Some(((*current).clone(), 0.0));
        }
        if self.index > 0 {
            self.index -= 1;
            self.playing = true;
            self.seq += 1;
            return None;
        }
        self.current_time = 0.0;
        self.seq += 1;
        Some(((*current).clone(), 0.0))
    }

    /// `player.jump(i)`, named by index and id when it comes from a controller.
    fn jump(&mut self, args: &IndexArgs) -> Option<(Entry, f64)> {
        let target = self.resolve(args.i, args.id.as_deref())?;
        let current = self.current_arc()?.clone();
        self.playing = true;
        self.seq += 1;
        if target as i64 == self.index {
            return Some(((*current).clone(), 0.0));
        }
        self.index = target as i64;
        None
    }

    fn remove_at(&mut self, args: &IndexArgs) {
        let Some(at) = self.resolve(args.i, args.id.as_deref()) else {
            return;
        };
        let removed = self.queue.remove(at);
        // The un-shuffle order follows the queue: a removed row must not come
        // back when the person un-shuffles. (The SPA leaves `_orig` alone and
        // does exactly that.)
        if let Some(orig) = self.orig.as_mut() {
            orig.retain(|e| !Arc::ptr_eq(e, &removed));
        }
        self.version += 1;
        if self.queue.is_empty() {
            self.index = -1;
            self.playing = false;
            self.orig = None;
        } else if (at as i64) < self.index {
            self.index -= 1;
        } else if at as i64 == self.index {
            // The playing row went away: the next one takes its place.
            self.index = self.index.min(self.queue.len() as i64 - 1);
        }
    }

    /// `player.move(from, to)`: the playing row keeps playing, only its index
    /// follows, and the audio is not restarted (`seq` untouched).
    fn move_to(&mut self, args: &MoveArgs) {
        let Some(from) = self.resolve(args.i, args.id.as_deref()) else {
            return;
        };
        let n = self.queue.len();
        if n < 2 {
            return;
        }
        // `to` is an index in the CALLER's view of the queue: carry it over by
        // however far the row itself moved (`commands.js#runCommand`).
        let to = (args.to + (from as i64 - args.i)).clamp(0, n as i64 - 1) as usize;
        if to == from {
            return;
        }
        let entry = self.queue.remove(from);
        self.queue.insert(to, entry);
        if from as i64 == self.index {
            self.index = to as i64;
        } else if (from as i64) < self.index && to as i64 >= self.index {
            self.index -= 1;
        } else if (from as i64) > self.index && to as i64 <= self.index {
            self.index += 1;
        }
        self.version += 1;
    }

    /// `player.clearUpcoming()`: drop everything after the playing row, and end
    /// the endless continuation with it — refilling a queue the person just
    /// cleared would be the opposite of what was asked.
    fn clear_upcoming(&mut self) {
        let keep = if self.index < 0 {
            0
        } else {
            (self.index + 1) as usize
        };
        if keep < self.queue.len() {
            self.queue.truncate(keep);
            self.version += 1;
        }
        self.orig = None;
        self.autoplay = false;
    }

    fn set_volume(&mut self, volume: f64) {
        self.volume = if volume.is_nan() {
            0.0
        } else {
            volume.clamp(0.0, 1.0)
        };
        // Turning the volume up is how a person unmutes.
        self.muted = false;
    }

    fn seek(&mut self, at_s: f64) -> Option<(Entry, f64)> {
        let current = self.current_arc()?.clone();
        let at = self.clamp_seek(at_s);
        self.current_time = at;
        self.seq += 1;
        // The engine's own seek path (a file seek, a byte range, `?start=`) is
        // what a Load at an offset means: never a restart from zero.
        Some(((*current).clone(), at))
    }

    fn clamp_seek(&self, at_s: f64) -> f64 {
        let at = if at_s.is_nan() { 0.0 } else { at_s.max(0.0) };
        if self.duration > 0.0 {
            at.min((self.duration - 0.25).max(0.0))
        } else {
            at
        }
    }

    fn play_queue(&mut self, args: QueueArgs) {
        let QueueArgs {
            tracks,
            start,
            context,
        } = args;
        if tracks.is_empty() {
            return;
        }
        // The tapped row is resolved in the ORIGINAL list first: dropping an
        // unplayable row above it must not start the wrong track.
        let wanted = start.min(tracks.len() - 1);
        let cleaned: Vec<(usize, Entry)> = tracks
            .into_iter()
            .enumerate()
            .filter(|(_, entry)| entry.is_playable())
            .take(MAX_QUEUE)
            .collect();
        let Some(at) = cleaned
            .iter()
            .position(|(original, _)| *original == wanted)
            .or_else(|| (!cleaned.is_empty()).then_some(0))
        else {
            return;
        };
        let entries: Vec<Arc<Entry>> = cleaned.into_iter().map(|(_, e)| Arc::new(e)).collect();
        self.orig = None;
        if self.shuffle {
            // The tapped row first, the rest shuffled. Positions are drawn over
            // the list WITHOUT it and mapped back, so nothing is copied twice:
            // a 4 000-row shuffle is 4 000 pointer copies, not 12 000.
            let order = shuffled_positions(entries.len() - 1, &mut self.rng);
            let mut queue = Vec::with_capacity(entries.len());
            queue.push(entries[at].clone());
            for p in order {
                let p = if p >= at { p + 1 } else { p };
                queue.push(entries[p].clone());
            }
            // The order the list came in is what un-shuffling goes back to.
            self.orig = Some(entries);
            self.queue = queue;
            self.index = 0;
        } else {
            self.queue = entries;
            self.index = at as i64;
        }
        self.playing = true;
        self.autoplay = true;
        self.context = context;
        self.version += 1;
        self.seq += 1;
        self.asked_more = None;
    }

    fn shuffle_play(&mut self, args: ShuffleArgs) {
        let entries: Vec<Arc<Entry>> = Self::clean(args.tracks, MAX_QUEUE)
            .into_iter()
            .map(Arc::new)
            .collect();
        if entries.is_empty() {
            return;
        }
        let order = shuffled_positions(entries.len(), &mut self.rng);
        self.queue = order.into_iter().map(|p| entries[p].clone()).collect();
        self.orig = Some(entries);
        self.shuffle = true;
        self.index = 0;
        self.playing = true;
        self.autoplay = true;
        self.context = args.context;
        self.version += 1;
        self.seq += 1;
        self.asked_more = None;
    }

    fn add_to_queue(&mut self, tracks: Vec<Entry>) {
        let extra: Vec<Arc<Entry>> = Self::clean(tracks, MAX_ADD)
            .into_iter()
            .map(Arc::new)
            .collect();
        if extra.is_empty() {
            return;
        }
        if self.index < 0 {
            self.index = 0;
            self.playing = true;
        }
        self.queue.extend(extra.iter().cloned());
        if let Some(orig) = self.orig.as_mut() {
            orig.extend(extra);
        }
        self.version += 1;
    }

    fn play_next(&mut self, tracks: Vec<Entry>) {
        let extra: Vec<Arc<Entry>> = Self::clean(tracks, MAX_ADD)
            .into_iter()
            .map(Arc::new)
            .collect();
        if extra.is_empty() {
            return;
        }
        let at = if self.index < 0 {
            0
        } else {
            (self.index + 1) as usize
        };
        if self.index < 0 {
            self.index = 0;
            self.playing = true;
        }
        self.queue.splice(at..at, extra.iter().cloned());
        // The un-shuffle order cannot say where a hand-inserted row belongs, so
        // it goes to its end: it is queued, and it survives an un-shuffle.
        if let Some(orig) = self.orig.as_mut() {
            orig.extend(extra);
        }
        self.version += 1;
    }

    /// `player.toggleShuffle()`: keep the playing row first, remember the order
    /// it came from, and give that order back — the playing row found by
    /// identity first, because a queue may hold the same track twice.
    fn toggle_shuffle(&mut self) {
        self.shuffle = !self.shuffle;
        if self.queue.is_empty() {
            return;
        }
        self.version += 1;
        if !self.shuffle {
            let current = self.current_arc().cloned();
            let base = self.orig.take().unwrap_or_else(|| self.queue.clone());
            let at = current
                .as_ref()
                .and_then(|cur| {
                    base.iter()
                        .position(|e| Arc::ptr_eq(e, cur))
                        .or_else(|| base.iter().position(|e| e.key() == cur.key()))
                })
                .unwrap_or(0);
            self.queue = base;
            self.index = at as i64;
            return;
        }
        if self.index < 0 {
            // Nothing playing: shuffle the whole queue and keep it unplayed.
            let order = shuffled_positions(self.queue.len(), &mut self.rng);
            let base = self.queue.clone();
            self.queue = order.into_iter().map(|p| base[p].clone()).collect();
            self.orig = Some(base);
            return;
        }
        let at = self.index as usize;
        // The queue itself becomes the un-shuffle order (no copy of 4 000
        // rows), and the shuffled queue is built from it: one pointer copy per
        // row, which is what keeps a 4 000-row shuffle inside a frame.
        let base = std::mem::take(&mut self.queue);
        let order = shuffled_positions(base.len().saturating_sub(1), &mut self.rng);
        let mut queue = Vec::with_capacity(base.len());
        queue.push(base[at].clone());
        for p in order {
            let p = if p >= at { p + 1 } else { p };
            queue.push(base[p].clone());
        }
        self.queue = queue;
        self.orig = Some(base);
        self.index = 0;
    }

    // -- plumbing ----------------------------------------------------------

    fn current_arc(&self) -> Option<&Arc<Entry>> {
        if self.index < 0 {
            None
        } else {
            self.queue.get(self.index as usize)
        }
    }

    fn before(&self) -> Before {
        Before {
            current: self.current_arc().cloned(),
            index: self.index,
            playing: self.playing,
            position: self.current_time,
            seq: self.seq,
            version: self.version,
            volume: self.volume,
            muted: self.muted,
            shuffle: self.shuffle,
            repeat: self.repeat,
            context: self.context.clone(),
            autoplay: self.autoplay,
        }
    }

    fn clean(tracks: Vec<Entry>, cap: usize) -> Vec<Entry> {
        tracks
            .into_iter()
            .filter(Entry::is_playable)
            .take(cap)
            .collect()
    }

    /// Where a row named by index AND id is now (`commands.js#resolveIndex`): the
    /// index when it still holds that id, else the nearest row carrying that id,
    /// else nothing — a jump to the wrong track is worse than no jump.
    fn resolve(&self, i: i64, id: Option<&str>) -> Option<usize> {
        let n = self.queue.len();
        if n == 0 {
            return None;
        }
        let Some(id) = id.filter(|id| !id.is_empty()) else {
            return (i >= 0 && (i as usize) < n).then_some(i as usize);
        };
        if i >= 0 && (i as usize) < n && self.queue[i as usize].key() == id {
            return Some(i as usize);
        }
        let mut best: Option<usize> = None;
        for (k, entry) in self.queue.iter().enumerate() {
            if entry.key() != id {
                continue;
            }
            let closer = best.is_none_or(|b| (k as i64 - i).abs() < (b as i64 - i).abs());
            if closer {
                best = Some(k);
            }
        }
        best
    }

    fn should_ask_more(&self) -> bool {
        self.autoplay
            && self.index >= 0
            && self.index + 1 == self.queue.len() as i64
            && self.context.as_ref().is_some_and(Context::wants_more)
            && self.asked_more.as_deref() != self.current_arc().map(|e| e.key())
    }

    fn finish(
        &mut self,
        before: Before,
        explicit: Option<(Entry, f64)>,
        now_ms: u64,
    ) -> Vec<Effect> {
        let current = self.current_arc().cloned();
        let key_now = current.as_ref().map(|e| e.key().to_owned());
        let changed = match (&before.current, &current) {
            (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
            (None, None) => false,
            _ => true,
        };
        let mut effects = Vec::new();

        if changed {
            if let Some((id, listened_s)) = self.ledger.take(now_ms) {
                effects.push(Effect::ReportListen {
                    id,
                    listened_s,
                    next_id: key_now.clone(),
                    context: self.context.clone(),
                    shuffle: self.shuffle,
                });
            }
            self.ledger.start(key_now.clone(), self.playing, now_ms);
            self.duration = current.as_ref().map(|e| e.duration as f64).unwrap_or(0.0);
            self.current_time = 0.0;
            self.seq += 1;
            self.asked_more = None;
        } else {
            self.ledger.open(self.playing, now_ms);
        }

        let load = match explicit {
            Some((entry, start_s)) => Some((entry, start_s)),
            None if changed => current.as_ref().map(|e| ((**e).clone(), 0.0)),
            None => None,
        };
        if let Some((entry, start_s)) = load {
            self.current_time = start_s;
            effects.push(Effect::Load {
                entry,
                autoplay: self.playing,
                start_s,
            });
        }

        let next = self.next_entry().cloned();
        let next_key = next.as_ref().map(|e| e.key().to_owned());
        if next_key != self.preloaded {
            self.preloaded = next_key;
            effects.push(Effect::PreloadNext(next));
        }

        if self.should_ask_more()
            && let Some(context) = self.context.clone()
        {
            effects.push(Effect::NeedMore { context });
            self.asked_more = key_now;
        }

        let dirty = before.version != self.version
            || before.index != self.index
            || before.playing != self.playing
            || before.position != self.current_time
            || before.seq != self.seq
            || before.volume != self.volume
            || before.muted != self.muted
            || before.shuffle != self.shuffle
            || before.repeat != self.repeat
            || before.context != self.context
            || before.autoplay != self.autoplay;
        if dirty {
            effects.push(Effect::Persist);
            effects.push(Effect::Changed);
        }
        effects
    }
}
