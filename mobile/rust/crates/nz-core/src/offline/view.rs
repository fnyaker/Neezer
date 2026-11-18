//! What a screen draws of the downloads: one set's progress, the rows of what
//! the device holds, and the storage report (lot MB15).
//!
//! These are the core's own types — `nz-ffi` converts them into the records the
//! Kotlin side sees, and no rule lives there.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A title kept beside its download, so the list opens with real names in
/// airplane mode (`lib/offline.js#slimTrack`: the SPA stores the same fields in
/// its `meta` store for exactly this reason).
///
/// It is written under the SET that asked for it (`offline.set.<kind>:<id>`):
/// one blob per set, so adding a 4 000-track playlist is one `kv` statement,
/// not four thousand (`nz-cache`'s `kv` stores one row per write).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackLabel {
    /// The track's universal id (`/api/stream/<id>`'s).
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    /// The one display line the row shows ("A feat. B"), already joined.
    #[serde(default)]
    pub artist: String,
    /// The archived cover's path, or the CDN URL — what `CoverImage` renders.
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(default)]
    pub duration_s: u32,
    #[serde(default)]
    pub explicit: bool,
    /// Milliseconds since the epoch: the list's order (newest first).
    #[serde(default)]
    pub added_at: i64,
}

/// One queued set as `kv` holds it: its own title (the row reads offline) and
/// the labels of every title it wants. One blob per set, so adding a
/// 4 000-track playlist is ONE statement, and removing the set is one delete.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedSet {
    /// The set's own title ("Nuit", an album's name, a show's).
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub tracks: Vec<TrackLabel>,
}

/// One download set — a playlist, an album, a show, or a single track — as the
/// `DownloadButton` and the settings list draw it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetRow {
    /// `playlist:42`, `album:7`, `show:3`, `track:3135556` — the context the
    /// button was pressed on, and the key the FFI speaks.
    pub context: String,
    pub kind: String,
    pub id: String,
    /// The set's own title, kept locally: the row reads offline.
    pub title: String,
    /// Published and playable.
    pub done: u32,
    /// What the queue ASKED for: a failure is a wanted title that is not there,
    /// so a finished run with six causes reads `194 / 200`, never `200 / 200`.
    pub total: u32,
    /// Gave up, each with its cause in [`SetRow::failures`].
    pub failed: u32,
    /// Still to come: queued or being written.
    pub running: bool,
    /// The person asked for a pause.
    pub paused: bool,
    /// One French sentence per failed title (« titre — cause »), never a count.
    pub failures: Vec<String>,
}

impl SetRow {
    /// `"12 / 200"` — what the button says while it works.
    pub fn done_label(&self) -> String {
        format!("{} / {}", self.done, self.total)
    }

    /// `true` when every title the set asked for is on the device.
    pub fn complete(&self) -> bool {
        self.total > 0 && self.done >= self.total && self.failed == 0
    }

    /// `true` when nothing of it is left to fetch.
    pub fn settled(&self) -> bool {
        !self.running && self.done + self.failed >= self.total
    }
}

/// One row of « Titres téléchargés » (`Settings.svelte#downloads`): the title,
/// its artist, the quality it really is and the bytes it costs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DownloadRow {
    pub key: String,
    pub title: String,
    pub artist: String,
    pub cover: Option<String>,
    pub duration_s: u32,
    pub explicit: bool,
    /// The rung the FILE is, not the one the row asked for: a LAN peer serves
    /// the copy it holds (`docs/agents/lan-nodes.md`).
    pub quality: String,
    pub bytes: u64,
    pub added_at: i64,
}

/// What the device holds, in counted bytes (rule 3's *Measures*: never an RSS).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StorageView {
    /// `filesDir`: the downloads, never evicted.
    pub downloads_bytes: u64,
    /// `cacheDir`: the play cache, LRU.
    pub cache_bytes: u64,
    /// `cacheDir`: the covers, LRU.
    pub covers_bytes: u64,
    /// How many titles are published on the device.
    pub tracks: u32,
    /// The disk's free space. **The platform fills this one**: Rust would need
    /// `statvfs` (an `unsafe` binding this crate forbids) for a figure Android
    /// answers anyway through `StatFs`, so the core answers 0 and the settings
    /// section prints the platform's number.
    pub free_bytes: u64,
}

/// The person's offline settings, as the section draws them (`stores.js`'s own
/// keys, held by `nz-offline` in `nz-cache`'s `kv`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OfflineSettingsView {
    /// `offline.quality`: the rung a NEW download takes (`OPUS_320` by default).
    pub quality: String,
    /// `offline.onlyDownloaded`: when the link is down, queue only what is here.
    pub offline_only: bool,
    /// `cache.limit`: the play cache's cap, in bytes.
    pub play_cache_cap: i64,
}

/// The whole offline state, as one value: what was published, and the version a
/// screen compares to know whether to repaint (`nz-offline`'s own counter).
#[derive(Clone, Debug, Default)]
pub struct OfflineState {
    pub version: u64,
    /// The worker's own sentence (« Téléchargement en cours », « Hors ligne :
    /// reprise automatique », or the cause that stopped it).
    pub state: String,
    pub paused: bool,
    pub sets: Vec<SetRow>,
    pub settings: OfflineSettingsView,
    pub storage: StorageView,
    /// The last thing the run did, in one sentence (rule 2).
    pub note: Option<String>,
    /// Every published id, for a row's « téléchargé » flag and the
    /// "only downloaded" filter: a set, so a 4 000-row list is O(1) per row.
    pub downloaded: std::sync::Arc<std::collections::HashSet<String>>,
}

/// The labels of every download, by track id. Rebuilt from the per-set blobs at
/// start and kept in memory: the list, the failures' titles and the single-track
/// rows all read it without a statement.
#[derive(Clone, Debug, Default)]
pub struct Labels {
    by_track: HashMap<String, TrackLabel>,
}

impl Labels {
    pub fn new(by_track: HashMap<String, TrackLabel>) -> Labels {
        Labels { by_track }
    }

    pub fn get(&self, track: &str) -> Option<&TrackLabel> {
        self.by_track.get(track)
    }

    /// The title of a downloaded track, or its id when it was never labelled.
    pub fn title_of(&self, track: &str) -> String {
        self.by_track
            .get(track)
            .map(|label| label.title.clone())
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| track.to_owned())
    }

    pub fn len(&self) -> usize {
        self.by_track.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_track.is_empty()
    }

    /// Counted bytes: the ids, the titles and the artists, never an estimate
    /// from a row count (worker rule 4).
    pub fn memory_bytes(&self) -> usize {
        self.by_track
            .iter()
            .map(|(id, label)| {
                id.len()
                    + label.id.len()
                    + label.title.len()
                    + label.artist.len()
                    + label.cover.as_ref().map(String::len).unwrap_or(0)
                    + std::mem::size_of::<TrackLabel>()
                    + 2 * std::mem::size_of::<String>()
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A set's labels say what a row DRAWS, and the counts a button prints are
    /// the ones a person reads: a failure is a wanted title.
    #[test]
    fn the_labels_are_counted_and_a_failure_is_still_wanted() {
        let mut by_track = HashMap::new();
        by_track.insert(
            "3135556".to_owned(),
            TrackLabel {
                id: "3135556".to_owned(),
                title: "Sinnerman".to_owned(),
                artist: "Nina Simone".to_owned(),
                cover: Some("/api/cover/1".to_owned()),
                duration_s: 620,
                explicit: false,
                added_at: 1,
            },
        );
        let labels = Labels::new(by_track);
        assert_eq!(labels.title_of("3135556"), "Sinnerman");
        assert_eq!(
            labels.title_of("404"),
            "404",
            "an id stands in for no label"
        );
        // 4 strings + the struct (+ the two the map adds per entry): the point
        // is that it is COUNTED, and that a label costs what it holds.
        assert!(labels.memory_bytes() > "Sinnerman".len() + "Nina Simone".len());

        let set = SetRow {
            context: "playlist:42".to_owned(),
            kind: "playlist".to_owned(),
            id: "42".to_owned(),
            title: "Nuit".to_owned(),
            done: 194,
            total: 200,
            failed: 6,
            running: false,
            paused: false,
            failures: vec!["Titre — 404".to_owned()],
        };
        assert_eq!(set.done_label(), "194 / 200");
        assert!(!set.complete(), "six wanted titles are not on the device");
        assert!(set.settled(), "nothing is left to fetch");
    }
}
