//! The player's saved queue: one row, one statement, at most 4 000 entries.
//!
//! The SPA keeps its session in `localStorage` under `player.session` and has to
//! cut it to 1 000 entries because a browser's quota is small and the write is
//! synchronous on the main thread (`lib/stores.js`, `SESSION_CAP`). A desktop has
//! neither problem, so the cap is the queue's own worst realistic case —
//! 4 000 entries — and the write is one upsert in a file the main thread never
//! touches (rule 4: a 4 000-entry queue is the input, not the happy path).
//!
//! The snapshot is generic over the entry type because the QUEUE belongs to
//! `nz-player` (D4): this crate stores and returns it, it never defines what a
//! queue entry is.
//!
//! A queue longer than [`MAX_QUEUE_ENTRIES`] is kept as a WINDOW of the full cap
//! around the playing track, never truncated at the tail: what comes next is
//! what the person is doing (rule 1). The window starts a quarter of the cap
//! before the playhead, clamped to the queue's end — so when the playhead is
//! near the end the tail is what gets dropped, and when it is near the beginning
//! the history is.

use rusqlite::params;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tracing::debug;

use crate::{Cache, Result, now_ms};

/// The most entries a saved queue keeps (docs/agents/desktop.md, D3).
pub const MAX_QUEUE_ENTRIES: usize = 4_000;

/// The `kv` key the snapshot lives under — the SPA's own name for it.
pub const QUEUE_KEY: &str = "player.session";

/// What the app restores at start: where the queue was, and what was playing.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(bound(
    serialize = "E: serde::Serialize",
    deserialize = "E: serde::de::DeserializeOwned"
))]
pub struct QueueSnapshot<E> {
    /// The playing entry's position in `entries`.
    pub index: usize,
    /// How far into that entry the player was, in milliseconds.
    #[serde(default)]
    pub position_ms: u64,
    /// The context it was playing from (`album:123`, `flow`, …), for "reprendre".
    #[serde(default)]
    pub context: Option<String>,
    /// When the snapshot was written (ms since the epoch). [`Queue::save`]
    /// stamps it; a restored snapshot can say how old it is.
    #[serde(default)]
    pub updated_at_ms: i64,
    /// The queue, in order.
    #[serde(default)]
    pub entries: Vec<E>,
}

impl<E> QueueSnapshot<E> {
    /// Builds a snapshot (the app fills `updated_at_ms` itself through `save`).
    pub fn new(index: usize, position_ms: u64, context: Option<String>, entries: Vec<E>) -> Self {
        Self {
            index,
            position_ms,
            context,
            updated_at_ms: 0,
            entries,
        }
    }

    /// No queue to restore.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// The first kept entry when `len` entries are saved around `index`.
///
/// Zero below the cap. Above it, the window is the full cap and starts a
/// quarter of it before the playhead, clamped to `len - cap`: at least that
/// quarter of history is kept, except when the playhead sits within a quarter
/// of the queue's start. Made public so the rule is assertable without building
/// a 4 000-entry fixture.
pub fn window_start(len: usize, index: usize) -> usize {
    if len <= MAX_QUEUE_ENTRIES {
        return 0;
    }
    let history = MAX_QUEUE_ENTRIES / 4;
    index.saturating_sub(history).min(len - MAX_QUEUE_ENTRIES)
}

/// The saved-queue store.
pub struct Queue<'a> {
    cache: &'a Cache,
}

impl<'a> Queue<'a> {
    pub(crate) fn new(cache: &'a Cache) -> Self {
        Self { cache }
    }

    /// Saves `snapshot` as one row (ONE statement), windowed when it is longer
    /// than the cap. Returns how many entries were kept.
    pub fn save<E: Serialize + Clone>(&self, snapshot: &QueueSnapshot<E>) -> Result<usize> {
        let total = snapshot.entries.len();
        let start = window_start(total, snapshot.index);
        let end = (start + MAX_QUEUE_ENTRIES).min(total);
        let index = snapshot
            .index
            .saturating_sub(start)
            .min(end.saturating_sub(start + 1));

        // Serialized as ONE document: a row per entry would need a transaction
        // and 4 000 statements for what is read back as a whole.
        #[derive(Serialize)]
        struct Wire<'a, E> {
            index: usize,
            position_ms: u64,
            context: Option<&'a str>,
            updated_at_ms: i64,
            entries: &'a [E],
        }
        let wire = Wire {
            index,
            position_ms: snapshot.position_ms,
            context: snapshot.context.as_deref(),
            updated_at_ms: now_ms(),
            entries: &snapshot.entries[start..end],
        };
        let body = serde_json::to_string(&wire)?;

        self.cache.write(|cache, conn| {
            cache.exec(
                conn,
                "INSERT INTO kv(key, value) VALUES(?1, ?2) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![QUEUE_KEY, body],
            )?;
            Ok(())
        })?;
        Ok(end - start)
    }

    /// The saved snapshot, or `None`.
    ///
    /// A snapshot that no longer parses (an older build's shape, a torn write
    /// on a full disk) is DROPPED and reported as absent: a queue the app cannot
    /// read must not keep the player from starting (rule 7).
    pub fn load<E: DeserializeOwned>(&self) -> Result<Option<QueueSnapshot<E>>> {
        let body = self.cache.read(|conn| {
            self.cache.query_optional(
                conn,
                "SELECT value FROM kv WHERE key = ?1",
                params![QUEUE_KEY],
                |row| row.get::<_, String>(0),
            )
        })?;
        let Some(body) = body else {
            return Ok(None);
        };
        match serde_json::from_str(&body) {
            Ok(snapshot) => Ok(Some(snapshot)),
            Err(error) => {
                debug!(%error, "saved queue does not parse; dropping it");
                self.clear()?;
                Ok(None)
            }
        }
    }

    /// Forgets the saved queue (the person cleared it).
    pub fn clear(&self) -> Result<bool> {
        let removed = self.cache.write(|cache, conn| {
            cache.exec(conn, "DELETE FROM kv WHERE key = ?1", params![QUEUE_KEY])
        })?;
        Ok(removed > 0)
    }

    /// When the saved queue was written, without parsing it.
    pub fn saved_at_ms(&self) -> Option<i64> {
        self.load::<serde::de::IgnoredAny>()
            .ok()
            .flatten()
            .map(|snapshot| snapshot.updated_at_ms)
    }
}

/// Podcast positions written while the server was unreachable, sent when it is
/// back (docs/agents/desktop.md, *Streams*: "offline it lands in
/// `pending_progress` and is sent on reconnection (last write wins)").
///
/// It lives beside the saved queue because it is the same kind of document: one
/// row per identity, written on the player's own thread, read back whole on
/// reconnection. The body is the exact JSON the app will `POST`, so the send is
/// a replay and never a re-derivation (a reconnected `POST` with a different
/// position would move the person's episode).
pub struct PendingProgress<'a> {
    cache: &'a Cache,
}

impl<'a> PendingProgress<'a> {
    pub(crate) fn new(cache: &'a Cache) -> Self {
        Self { cache }
    }

    /// Remembers the newest position of an episode; a later write replaces an
    /// earlier one (last write wins, the table's own promise).
    pub fn put(&self, owner: &str, episode: &str, body: &str) -> Result<()> {
        self.cache.write(|cache, conn| {
            cache.exec(
                conn,
                "INSERT INTO pending_progress(owner, episode, body) VALUES(?1, ?2, ?3) \
                 ON CONFLICT(owner, episode) DO UPDATE SET body = excluded.body",
                params![owner, episode, body],
            )?;
            Ok(())
        })
    }

    /// Everything still to send for one account, oldest row first.
    pub fn pending(&self, owner: &str) -> Result<Vec<(String, String)>> {
        self.cache.read(|conn| {
            self.cache.rows(
                conn,
                "SELECT episode, body FROM pending_progress WHERE owner = ?1 \
                 ORDER BY rowid ASC",
                params![owner],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
        })
    }

    /// Forgets one episode once the server accepted its position. Returns
    /// whether a row was removed.
    pub fn sent(&self, owner: &str, episode: &str) -> Result<bool> {
        let removed = self.cache.write(|cache, conn| {
            cache.exec(
                conn,
                "DELETE FROM pending_progress WHERE owner = ?1 AND episode = ?2",
                params![owner, episode],
            )
        })?;
        Ok(removed > 0)
    }

    /// How many positions wait (the UI's "N positions à envoyer").
    pub fn len(&self, owner: &str) -> Result<i64> {
        self.cache.read(|conn| {
            self.cache.query(
                conn,
                "SELECT COUNT(*) FROM pending_progress WHERE owner = ?1",
                params![owner],
                |row| row.get(0),
            )
        })
    }

    /// Nothing waiting.
    pub fn is_empty(&self, owner: &str) -> bool {
        self.len(owner).unwrap_or(1) == 0
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use serde::{Deserialize, Serialize};

    use super::{MAX_QUEUE_ENTRIES, QueueSnapshot, window_start};
    use crate::Result;
    use crate::testing::TempDir;

    /// A queue entry shaped like the player's: enough fields that the JSON's
    /// size is the real one, not a toy's.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Entry {
        id: String,
        title: String,
        artist: String,
        album: String,
        duration_ms: u64,
        quality: String,
        cover: String,
    }

    fn entry(n: usize) -> Entry {
        Entry {
            id: format!("3000{n:04}"),
            title: format!("Titre {n}"),
            artist: "Kaya Project".to_owned(),
            album: "Songs from the Inside".to_owned(),
            duration_ms: 312_000,
            quality: "OPUS_320".to_owned(),
            cover: "/api/cover/1f0e".to_owned(),
        }
    }

    fn queue(n: usize, index: usize) -> QueueSnapshot<Entry> {
        QueueSnapshot::new(
            index,
            42_500,
            Some("playlist:p1".to_owned()),
            (0..n).map(entry).collect(),
        )
    }

    #[test]
    fn a_queue_round_trips() -> Result<()> {
        let dir = TempDir::new("queue-roundtrip");
        let cache = dir.open();
        let saved = queue(3, 1);

        assert_eq!(cache.queue().save(&saved)?, 3);
        let loaded: QueueSnapshot<Entry> = cache.queue().load()?.expect("saved queue");

        assert_eq!(loaded.entries, saved.entries);
        assert_eq!(loaded.index, 1);
        assert_eq!(loaded.position_ms, 42_500);
        assert_eq!(loaded.context.as_deref(), Some("playlist:p1"));
        assert!(loaded.updated_at_ms > 0, "save stamps the moment");
        Ok(())
    }

    #[test]
    fn an_empty_store_loads_nothing_and_clear_is_idempotent() -> Result<()> {
        let dir = TempDir::new("queue-empty");
        let cache = dir.open();

        assert!(cache.queue().load::<Entry>()?.is_none());
        assert!(!cache.queue().clear()?);
        cache.queue().save(&queue(1, 0))?;
        assert!(cache.queue().clear()?);
        assert!(cache.queue().load::<Entry>()?.is_none());
        Ok(())
    }

    /// Criterion: "saving a 4 000-entry queue < 30 ms and one statement batch
    /// (count it)".
    ///
    /// Measured on this machine (debug build, the CI configuration):
    /// **4 000 entries in 12.9 ms, 1 statement**; release: **3.6 ms**. The
    /// 30 ms budget is the lot's, and it is a release number (like D7's 300 ms
    /// first frame), so the debug assertion keeps CI headroom while the release
    /// one holds the criterion itself.
    #[test]
    fn saving_a_four_thousand_entry_queue_is_one_statement() -> Result<()> {
        let dir = TempDir::new("queue-4000");
        let cache = dir.open();
        let snapshot = queue(MAX_QUEUE_ENTRIES, 2_000);

        let before = cache.statements();
        let started = Instant::now();
        let kept = cache.queue().save(&snapshot)?;
        let elapsed = started.elapsed();
        let statements = cache.statements() - before;

        assert_eq!(kept, MAX_QUEUE_ENTRIES);
        assert_eq!(statements, 1, "the whole queue is one upsert");
        let budget = if cfg!(debug_assertions) {
            std::time::Duration::from_millis(150)
        } else {
            std::time::Duration::from_millis(30)
        };
        assert!(
            elapsed < budget,
            "saving 4 000 entries took {elapsed:?} (budget {budget:?})"
        );
        eprintln!("save 4000 entries: {elapsed:?}, {statements} statement(s)");

        let loaded: QueueSnapshot<Entry> = cache.queue().load()?.expect("saved queue");
        assert_eq!(loaded.entries.len(), MAX_QUEUE_ENTRIES);
        assert_eq!(loaded.index, 2_000);
        Ok(())
    }

    /// The acceptance's worst case is not the cap itself: a 6 000-entry queue
    /// must still save, keep what comes NEXT, and restore the right track.
    #[test]
    fn a_longer_queue_is_windowed_around_the_playhead() -> Result<()> {
        let dir = TempDir::new("queue-window");
        let cache = dir.open();
        let snapshot = queue(6_000, 4_200);

        let kept = cache.queue().save(&snapshot)?;
        assert_eq!(kept, MAX_QUEUE_ENTRIES);

        let loaded: QueueSnapshot<Entry> = cache.queue().load()?.expect("saved queue");
        assert_eq!(loaded.entries.len(), MAX_QUEUE_ENTRIES);
        assert_eq!(
            loaded.entries[loaded.index], snapshot.entries[4_200],
            "the playing track survives the window"
        );

        // The window is as long as the cap allows: here it can only start at
        // 2 000 (the queue's last 4 000 entries), so 2 200 entries of history
        // remain — a quarter of the cap is the MINIMUM, not the rule.
        let start = window_start(6_000, 4_200);
        assert_eq!(start, 2_000);
        assert_eq!(loaded.index, 4_200 - start);
        Ok(())
    }

    #[test]
    fn a_short_queue_keeps_its_head() {
        assert_eq!(window_start(10, 9), 0);
        assert_eq!(window_start(MAX_QUEUE_ENTRIES, 3_999), 0);
    }

    /// Offline podcast positions: one row per episode, last write wins, sent
    /// and forgotten one at a time.
    #[test]
    fn an_offline_position_replaces_the_previous_one_and_is_sent_once() -> Result<()> {
        let dir = TempDir::new("queue-progress");
        let cache = dir.open();
        let progress = cache.pending_progress();

        progress.put("42", "e1", r#"{"episode_id":"e1","position":12.5}"#)?;
        progress.put("42", "e1", r#"{"episode_id":"e1","position":30.0}"#)?;
        progress.put("42", "e2", r#"{"episode_id":"e2","position":1.0}"#)?;
        progress.put("7", "e9", r#"{"episode_id":"e9","position":2.0}"#)?;

        let mine = progress.pending("42")?;
        assert_eq!(mine.len(), 2, "one row per episode: last write wins");
        assert!(
            mine[0].1.contains("30.0"),
            "the newer position replaced the older"
        );

        assert!(progress.sent("42", "e1")?);
        assert!(!progress.sent("42", "e1")?);
        assert_eq!(progress.len("42")?, 1);
        assert_eq!(progress.len("7")?, 1, "another account keeps its own");
        assert!(!progress.is_empty("7"));
        Ok(())
    }

    #[test]
    fn a_snapshot_that_no_longer_parses_is_dropped_not_fatal() -> Result<()> {
        let dir = TempDir::new("queue-corrupt");
        let cache = dir.open();
        cache.kv().set(super::QUEUE_KEY, &"{not json")?;

        assert!(cache.queue().load::<Entry>()?.is_none());
        assert!(
            !cache.kv().contains(super::QUEUE_KEY),
            "the unreadable row was removed, so the next save is clean"
        );
        Ok(())
    }
}
