//! What survives a restart: the queue, the transport settings, and the audio
//! chain's own settings.
//!
//! Owned by **MB3b**. The note is explicit about the two stores and why they are
//! two (`docs/agents/mobile.md`, *Caches*):
//!
//! - **The queue lives in `nz-cache`'s `Queue` table** (SQLite, WAL): the rows,
//!   the index, the position in milliseconds and the context. It is the SPA's
//!   `player.session` snapshot, and `Queue::save` already windows a very long
//!   queue around the playhead — a 4 000-row queue is not copied whole into one
//!   row of a database the phone keeps on flash.
//! - **The transport settings** (`shuffle`, `repeat`, `volume`, `muted`) are
//!   small scalars in `kv`, because they must survive even when the queue was
//!   cleared.
//! - **The audio chain's settings** are one JSON value under
//!   [`AudioSettings::KEY`]: what Réglages shows, and what the engine is given at
//!   start.
//!
//! **A restored queue comes back PAUSED, at the same position** — the person
//! presses play, the phone does not start talking in their pocket. That is the
//! `autoplay: false` the state machine already understands (`Snapshot::autoplay`),
//! and the position arrives as a `Load { start_s }` so a restored episode
//! reopens where it was instead of at 0.
//!
//! Writing happens on the player's own worker thread, never on a caller's: a
//! command applies in memory at once and asks for [`Effect::Persist`], which is
//! the only place a database write happens.

use nz_cache::QueueSnapshot;
use nz_player::{Context, Entry, PlayerState, Repeat, Snapshot};
use serde::{Deserialize, Serialize};

use super::quality::AudioSettings;

/// Where the transport's own scalars live (`kv`).
pub const KEY_TRANSPORT: &str = "player.transport";

/// The scalars `nz-cache`'s `Queue` does not keep.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Transport {
    shuffle: bool,
    repeat: Repeat,
    volume: f64,
    muted: bool,
    autoplay: bool,
}

impl Default for Transport {
    fn default() -> Self {
        Self {
            shuffle: false,
            repeat: Repeat::Off,
            volume: 1.0,
            muted: false,
            autoplay: true,
        }
    }
}

/// The persistence seam: the queue, the transport and the audio settings.
pub struct Store<'a> {
    cache: &'a nz_cache::Cache,
}

impl<'a> Store<'a> {
    pub fn new(cache: &'a nz_cache::Cache) -> Store<'a> {
        Store { cache }
    }

    /// Writes the queue and the transport. Answers the number of entries the
    /// store kept (a 4 000-row queue is windowed around the playhead).
    pub fn save_queue(&self, machine: &PlayerState) -> usize {
        let snapshot = machine.snapshot();
        let entries: Vec<Entry> = snapshot.queue.clone();
        let context = snapshot
            .context
            .as_ref()
            .and_then(|context| serde_json::to_string(context).ok());
        let index = snapshot.index.max(0) as usize;
        let position_ms = (snapshot.position.max(0.0) * 1000.0) as u64;
        let kept = self
            .cache
            .queue()
            .save(&QueueSnapshot::new(index, position_ms, context, entries))
            .unwrap_or(0);
        self.save_transport(&Transport {
            shuffle: snapshot.shuffle,
            repeat: snapshot.repeat,
            volume: snapshot.volume,
            muted: snapshot.muted,
            autoplay: snapshot.autoplay,
        });
        kept
    }

    /// The queue to restore, or `None` when there is none (a fresh install, or a
    /// queue the person cleared).
    ///
    /// `autoplay` is forced to FALSE: a restored session waits for a click
    /// (`Player.svelte`'s `preload="none"` and the note's own rule).
    pub fn load_queue(&self) -> Option<Snapshot> {
        let stored: QueueSnapshot<Entry> = self.cache.queue().load().ok().flatten()?;
        if stored.entries.is_empty() {
            return None;
        }
        let transport = self.load_transport().unwrap_or_default();
        let index = (stored.index.min(stored.entries.len() - 1)) as i64;
        Some(Snapshot {
            queue: stored.entries,
            index,
            position: stored.position_ms as f64 / 1000.0,
            shuffle: transport.shuffle,
            repeat: transport.repeat,
            autoplay: false,
            context: stored
                .context
                .as_deref()
                .and_then(|text| serde_json::from_str::<Context>(text).ok()),
            volume: transport.volume,
            muted: transport.muted,
            // The un-shuffle order is NOT restored: `PlayerState::restore`
            // rebuilds it from the queue as it stands, which is the order a
            // person sees. Restoring a stale `orig` would make a later
            // un-shuffle resurrect rows the person removed (`stores.js`'s own
            // trap, already fixed in `nz-player`).
            orig: None,
        })
    }

    /// Forgets the queue (the person cleared it, or the last row ended and the
    /// list is over).
    pub fn clear_queue(&self) {
        let _ = self.cache.queue().clear();
    }

    pub fn save_audio(&self, settings: &AudioSettings) {
        if let Err(error) = self.cache.kv().set(AudioSettings::KEY, settings) {
            tracing::debug!(%error, "audio settings not written");
        }
    }

    /// The stored settings, clamped: a value written by an older build (or by a
    /// slider that went past its end) never reaches the chain out of range.
    pub fn load_audio(&self) -> AudioSettings {
        self.cache
            .kv()
            .get::<AudioSettings>(AudioSettings::KEY)
            .unwrap_or_default()
            .clamped()
    }

    fn save_transport(&self, transport: &Transport) {
        if let Err(error) = self.cache.kv().set(KEY_TRANSPORT, transport) {
            tracing::debug!(%error, "transport settings not written");
        }
    }

    fn load_transport(&self) -> Option<Transport> {
        self.cache.kv().get(KEY_TRANSPORT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nz_player::Command;

    fn state_with_queue(len: usize) -> PlayerState {
        let mut machine = PlayerState::new();
        let tracks: Vec<Entry> = (0..len)
            .map(|n| Entry {
                title: format!("Titre {n}"),
                duration: 180,
                ..Entry::new(format!("key-{n}"))
            })
            .collect();
        machine.apply(Command::play_queue(tracks, 3, None), 1_000);
        machine
    }

    /// A queue comes back where it was — same rows, same index, same position —
    /// and PAUSED: the phone does not start talking in a pocket.
    #[test]
    fn a_queue_comes_back_paused_at_the_same_position() {
        let dir = crate::player::tests::TempDir::new("persist-queue");
        let core = dir.core();
        let mut machine = state_with_queue(40);
        machine.set_position(42.5);
        machine.apply(Command::set_volume(0.4), 2_000);
        machine.apply(Command::cycle_repeat(), 3_000);

        let kept = Store::new(core.cache()).save_queue(&machine);
        assert_eq!(kept, 40, "a short queue is kept whole");

        let restored = Store::new(core.cache())
            .load_queue()
            .expect("a stored queue");
        assert_eq!(restored.queue.len(), 40);
        assert_eq!(restored.index, 3);
        assert!(
            (restored.position - 42.5).abs() < 0.001,
            "{}",
            restored.position
        );
        assert!(!restored.autoplay, "a restored queue waits for a click");
        assert_eq!(restored.volume, 0.4);
        assert_eq!(restored.repeat, Repeat::All);

        let (machine, effects) = PlayerState::restore(restored);
        assert!(!machine.playing());
        let load = effects
            .iter()
            .find_map(|effect| match effect {
                nz_player::Effect::Load {
                    entry,
                    autoplay,
                    start_s,
                } => Some((entry.key().to_owned(), *autoplay, *start_s)),
                _ => None,
            })
            .expect("a restored queue loads its row, paused");
        assert_eq!(load.0, "key-3");
        assert!(!load.1);
        assert!((load.2 - 42.5).abs() < 0.001);
    }

    /// A cleared queue restores nothing at all — not an empty queue that a
    /// screen would draw as "0 sur 0".
    #[test]
    fn a_cleared_queue_restores_nothing() {
        let dir = crate::player::tests::TempDir::new("persist-clear");
        let core = dir.core();
        let machine = state_with_queue(5);
        let store = Store::new(core.cache());
        store.save_queue(&machine);
        store.clear_queue();
        assert!(store.load_queue().is_none());
    }

    /// The audio settings round-trip, and a hand-edited value is clamped rather
    /// than handed to the chain.
    #[test]
    fn the_audio_settings_round_trip_and_are_clamped() {
        let dir = crate::player::tests::TempDir::new("persist-audio");
        let core = dir.core();
        let store = Store::new(core.cache());
        assert_eq!(store.load_audio(), AudioSettings::default());

        let settings = AudioSettings {
            quality: super::super::quality::Quality::Opus128,
            speed: 1.4,
            ..AudioSettings::default()
        };
        store.save_audio(&settings);
        assert_eq!(store.load_audio(), settings);

        // A row an older build (or a slider gone past its end) could have
        // written: built from the real value, then pushed out of range.
        let mut hand_edited = serde_json::to_value(AudioSettings {
            quality: super::super::quality::Quality::Flac,
            ..AudioSettings::default()
        })
        .expect("serialises");
        hand_edited["speed"] = serde_json::json!(9.0);
        hand_edited["crossfade_s"] = serde_json::json!(99.0);
        hand_edited["bass_db"] = serde_json::json!(40.0);
        hand_edited["eq_db"] = serde_json::json!([]);
        core.cache()
            .kv()
            .set(AudioSettings::KEY, &hand_edited)
            .expect("a hand-edited row");
        let clamped = store.load_audio();
        assert_eq!(clamped.quality, super::super::quality::Quality::Flac);
        assert!(clamped.speed <= nz_audio::MAX_SPEED);
        assert_eq!(clamped.crossfade_s, super::super::quality::MAX_CROSSFADE_S);
        assert_eq!(clamped.eq_db.len(), super::super::quality::BANDS);
    }
}
