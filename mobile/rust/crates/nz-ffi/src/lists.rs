//! Every long list, windowed (rule 4). Owned by **MB2**.
//!
//! A 4 000-row list never crosses the boundary whole: the screen keeps a
//! `ListHandle` and asks `window(start, count)` as rows near the viewport, so a
//! scroll costs pages of at most 128 rows instead of one 4 000-record copy.

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

/// A windowed handle over one list. MB2 fills it; MB0 answers empty.
#[derive(uniffi::Object)]
pub struct ListHandle {
    /// The rows' keys, in the list's current order. MB0 keeps them so `len()`
    /// is honest about what was opened before MB2 exists.
    pub(crate) keys: Vec<String>,
}

/// Opens a handle over `keys` (MB2: over the typed rows the caller names).
pub(crate) fn open(keys: Vec<String>) -> std::sync::Arc<ListHandle> {
    std::sync::Arc::new(ListHandle { keys })
}

// `len` is the SCREEN's row count (the contract every list compiles against),
// not a collection's length: the Kotlin side asks `len() == 0` and never
// `isEmpty()`, so the usual sibling method would be a second spelling of it.
#[allow(clippy::len_without_is_empty)]
#[uniffi::export]
impl ListHandle {
    /// The number of rows the list has NOW (after a filter or a sort).
    pub fn len(&self) -> u32 {
        // MB0: the rows themselves arrive with MB2's typed store; the handle is
        // honestly empty until then, so no screen draws a phantom row.
        let _ = &self.keys;
        0
    }

    /// `count` is at most 128; the UI pages 64 at a time as rows near the
    /// viewport, so a fling asks a handful of windows, not thousands of rows.
    pub fn window(&self, start: u32, count: u32) -> Vec<TrackRow> {
        let _ = (start, count);
        Vec::new()
    }

    /// The row carrying `key`, or `None` when the filter hid it.
    pub fn index_of(&self, key: String) -> Option<u32> {
        let _ = key;
        None
    }

    /// Folds the list with `appcore::fold` (accents, case, artist splits) and
    /// returns the new length.
    pub fn filter(&self, query: String) -> u32 {
        let _ = query;
        0
    }

    pub fn sort(&self, by: SortKey, descending: bool) -> u32 {
        let _ = (by, descending);
        0
    }

    /// Bumps on every change; the UI re-windows when it moves. A like/dislike
    /// (MB10) or a finished download (MB15) patches every open row carrying
    /// that key in the same frame, without a re-fetch.
    pub fn version(&self) -> u64 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The handle never hands out more than it has, and MB0's is empty.
    #[test]
    fn an_empty_handle_answers_empty() {
        let h = open(vec!["a".into()]);
        assert_eq!(h.len(), 0);
        assert!(h.window(0, 128).is_empty());
        assert_eq!(h.filter("a".to_string()), 0);
        assert!(h.index_of("a".into()).is_none());
        assert_eq!(h.version(), 0);
    }
}
