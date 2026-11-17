//! Every long list, windowed (rule 4). Owned by **MB2**.
//!
//! A 4 000-row list never crosses the boundary whole: the screen keeps a
//! `ListHandle` and asks `window(start, count)` as rows near the viewport, so a
//! scroll costs pages of at most 128 rows instead of one 4 000-record copy. The
//! rows themselves, their folded search text and their order live in
//! `nz_core::lists`, where a like/dislike (MB10) or a finished download (MB15)
//! patches every open list in the same frame.

use std::sync::Arc;

use nz_core::core::Core;
use nz_core::lists::{List, Registry, Row};

/// One row of any track list, shaped the way the row is DRAWN: title, subtitle,
/// duration, cover key, flags. Not the server's JSON — `nz-core` converts.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct TrackRow {
    /// The stable key the screen uses for `LazyColumn`'s `key` and for
    /// `update_rows`: `<source>:<id>` for a track, the queue position for an
    /// entry the server has no id for.
    pub key: String,
    pub id: String,
    pub title: String,
    /// Already joined for display ("A, B"), never a list of names to rebuild.
    pub artists: String,
    pub album: String,
    pub album_id: Option<String>,
    pub duration_s: u32,
    /// A cover URL or a cached file's path; `None` draws the Sillon grooves.
    pub cover: Option<String>,
    pub explicit: bool,
    pub liked: bool,
    pub disliked: bool,
    /// The provider cannot serve it right now — shown, not hidden (CLAUDE.md:
    /// a failure to reach a provider is never a verdict).
    pub unavailable: bool,
    pub downloaded: bool,
    /// `deezer`, `youtube`, `upload`, `local` — what the source chip says.
    pub source: String,
    /// Position in the underlying list, so a drag can name a target index.
    pub position: u32,
}

impl From<Row> for TrackRow {
    fn from(row: Row) -> Self {
        TrackRow {
            key: row.key,
            id: row.id,
            title: row.title,
            artists: row.artists,
            album: row.album,
            album_id: row.album_id,
            duration_s: row.duration_s,
            cover: row.cover,
            explicit: row.explicit,
            liked: row.liked,
            disliked: row.disliked,
            unavailable: row.unavailable,
            downloaded: row.downloaded,
            source: row.source,
            position: row.position,
        }
    }
}

impl From<TrackRow> for Row {
    fn from(row: TrackRow) -> Self {
        Row {
            key: row.key,
            id: row.id,
            title: row.title,
            artists: row.artists,
            album: row.album,
            album_id: row.album_id,
            duration_s: row.duration_s,
            cover: row.cover,
            explicit: row.explicit,
            liked: row.liked,
            disliked: row.disliked,
            unavailable: row.unavailable,
            downloaded: row.downloaded,
            source: row.source,
            position: row.position,
            // The two sort keys a screen does not draw: a lot that has them
            // (history's last play, a playlist's added date) sets them on the
            // core side, where they belong.
            played_at: 0,
            added_at: 0,
        }
    }
}

impl TrackRow {
    /// A row a test builds without the server: the shape only, with the flags a
    /// fresh row carries.
    pub fn test_row(number: usize, title: &str) -> TrackRow {
        TrackRow::from(Row::test_row(number, title))
    }
}

/// The sort keys the track lists offer. `appcore` provides the fold and the
/// comparisons; the handle only counts.
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Title,
    Artist,
    Album,
    Duration,
    Added,
    Position,
    Played,
}

impl From<SortKey> for nz_core::lists::SortKey {
    fn from(key: SortKey) -> Self {
        match key {
            SortKey::Title => nz_core::lists::SortKey::Title,
            SortKey::Artist => nz_core::lists::SortKey::Artist,
            SortKey::Album => nz_core::lists::SortKey::Album,
            SortKey::Duration => nz_core::lists::SortKey::Duration,
            SortKey::Added => nz_core::lists::SortKey::Added,
            SortKey::Position => nz_core::lists::SortKey::Position,
            SortKey::Played => nz_core::lists::SortKey::Played,
        }
    }
}

/// A windowed handle over one list.
#[derive(uniffi::Object)]
pub struct ListHandle {
    list: Arc<List>,
}

impl ListHandle {
    /// The list behind this handle, for a lot that patches it (MB10's
    /// like/dislike, MB15's "downloaded" flag). Nothing in MB2 calls it outside
    /// its tests — the accessor exists FOR those lots, so it is not dead code.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the seam MB10/MB15 patch lists through")
    )]
    pub(crate) fn list(&self) -> &Arc<List> {
        &self.list
    }
}

/// An EMPTY handle with no registry behind it — what a caller that has no core
/// yet (MB0's player stub) gets. A real screen asks [`open_rows`].
pub(crate) fn open(keys: Vec<String>) -> Arc<ListHandle> {
    tracing::debug!(keys = keys.len(), "open: no core, an empty handle");
    Arc::new(ListHandle {
        list: Arc::new(List::new(Vec::new())),
    })
}

/// Opens an EMPTY list, registered so a patch still reaches it.
///
/// MB0's `open_list(keys)` had no way to carry the rows themselves, and a row
/// with a key and an empty title would be a PHANTOM row a screen draws. So the
/// keys are accepted (a caller may already have them, and a later lot can
/// resolve them) but nothing is invented: a caller that has rows asks
/// [`open_rows`] — through `NzCore::open_track_list`.
pub(crate) fn open_empty(core: &Arc<Core>, keys: Vec<String>) -> Arc<ListHandle> {
    tracing::debug!(keys = keys.len(), "open_list: no rows yet, an empty handle");
    open_rows(core, Vec::new())
}

/// Opens a list over `rows`, registered so a later lot's patch reaches it.
pub(crate) fn open_rows(core: &Arc<Core>, rows: Vec<Row>) -> Arc<ListHandle> {
    let registry: &Registry = core.lists();
    let list = registry.open(rows);
    Arc::new(ListHandle { list })
}

/// The maximum a single `window` call hands out. The UI pages 64 at a time; 128
/// is the ceiling the contract names, so a caller cannot ask for the whole list.
pub const MAX_WINDOW: u32 = 128;

// `len` is the SCREEN's row count (the contract every list compiles against),
// not a collection's length: the Kotlin side asks `len() == 0` and never
// `isEmpty()`, so the usual sibling method would be a second spelling of it.
#[allow(clippy::len_without_is_empty)]
#[uniffi::export]
impl ListHandle {
    /// The number of rows the list has NOW (after a filter or a sort).
    pub fn len(&self) -> u32 {
        self.list.len().min(u32::MAX as usize) as u32
    }

    /// `count` is at most [`MAX_WINDOW`]; the UI pages 64 at a time as rows near
    /// the viewport, so a fling asks a handful of windows, not thousands of
    /// rows. A larger `count` is clamped rather than refused: the screen is
    /// drawing what it can hold.
    pub fn window(&self, start: u32, count: u32) -> Vec<TrackRow> {
        self.list
            .window(start, count.min(MAX_WINDOW))
            .into_iter()
            .map(TrackRow::from)
            .collect()
    }

    /// The row carrying `key`, or `None` when the filter hid it.
    pub fn index_of(&self, key: String) -> Option<u32> {
        self.list.index_of(&key)
    }

    /// Folds the list with `appcore::fold` (accents, case, artist splits) and
    /// returns the new length.
    pub fn filter(&self, query: String) -> u32 {
        self.list.filter(&query).min(u32::MAX as usize) as u32
    }

    pub fn sort(&self, by: SortKey, descending: bool) -> u32 {
        self.list.sort(by.into(), descending).min(u32::MAX as usize) as u32
    }

    /// Bumps on every change; the UI re-windows when it moves. A like/dislike
    /// (MB10) or a finished download (MB15) patches every open row carrying
    /// that key in the same frame, without a re-fetch.
    pub fn version(&self) -> u64 {
        self.list.version()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nz_core::core::CoreConfig;

    fn core(tag: &str) -> (std::path::PathBuf, Arc<Core>) {
        let path = std::env::temp_dir().join(format!("nz-ffi-lists-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let core = Arc::new(
            Core::new(CoreConfig {
                data_dir: path.join("data"),
                cache_dir: path.join("cache"),
                app_version: "0.0.0".into(),
                version_code: 2_000_001,
                sdk_int: 35,
                device_name: "test".into(),
            })
            .expect("a core"),
        );
        (path, core)
    }

    /// The acceptance: 8 000 rows, `window(0, 64)` gives 64, a filter answers
    /// the count the SPA's own fold answers, and the window never exceeds 128.
    #[test]
    fn eight_thousand_rows_window_and_filter_through_the_boundary() {
        let (path, core) = core("window");
        let rows: Vec<TrackRow> = (0..8_000)
            .map(|number| TrackRow::test_row(number, &format!("Titre {number}")))
            .collect();
        let handle = open_rows(&core, rows.into_iter().map(Row::from).collect());

        assert_eq!(handle.len(), 8_000);
        // The list behind the handle is what a later lot patches directly.
        assert_eq!(handle.list().len(), 8_000);
        assert_eq!(handle.window(0, 64).len(), 64);
        // A caller asking for everything gets a page, not the list.
        assert_eq!(handle.window(0, 10_000).len(), MAX_WINDOW as usize);

        // The expected count, computed the way the fold makes it: ASCII digits
        // and lowercase letters fold to themselves, so "titre 1" matches every
        // title that spells it (and no artist or album does).
        let expected = (0..8_000)
            .filter(|number| format!("Titre {number}").to_lowercase().contains("titre 1"))
            .count();
        assert_eq!(handle.filter("titre 1".into()), expected as u32);
        assert!(expected > 0);

        // The registry knows the list, so MB10's patch will reach it.
        assert_eq!(core.lists().live(), 1);
        let before = handle.version();
        core.lists().update_rows("test:3", |row| row.liked = true);
        assert!(handle.version() > before, "a patch bumps the version");
        let _ = std::fs::remove_dir_all(path);
    }

    /// `index_of` reads the DISPLAY order, which is what a queue reorder needs
    /// (the row's own `position` is the list's order, not the filter's).
    #[test]
    fn index_of_reads_the_display_order() {
        let (path, core) = core("index");
        let rows: Vec<TrackRow> = ["z", "a", "m"]
            .iter()
            .enumerate()
            .map(|(number, title)| TrackRow::test_row(number, title))
            .collect();
        let handle = open_rows(&core, rows.into_iter().map(Row::from).collect());
        assert_eq!(handle.index_of("test:0".into()), Some(0));
        handle.sort(SortKey::Title, false);
        assert_eq!(handle.index_of("test:0".into()), Some(2), "z is last now");
        assert_eq!(handle.index_of("inconnu".into()), None);
        let _ = std::fs::remove_dir_all(path);
    }

    /// An empty list is honest about being empty (the skeleton's own promise,
    /// kept for `open_list`).
    #[test]
    fn an_empty_handle_answers_empty() {
        let (path, core) = core("empty");
        // `open_list(keys)` invents no rows: a key without its row is a phantom
        // row a screen would draw.
        let handle = open_empty(&core, vec!["a".into(), "b".into()]);
        assert_eq!(handle.len(), 0);
        assert!(handle.window(0, 128).is_empty());
        let empty = open_rows(&core, Vec::new());
        assert_eq!(empty.len(), 0);
        assert!(empty.window(0, 64).is_empty());
        assert_eq!(empty.filter("a".into()), 0);
        assert!(empty.version() >= 1, "a handle is versioned from birth");
        let _ = std::fs::remove_dir_all(path);
    }
}
