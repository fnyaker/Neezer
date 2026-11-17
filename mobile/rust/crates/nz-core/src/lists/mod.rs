//! The open lists' registry and the windowed list handles every long screen
//! draws (rule 4). Owned by **MB2**.
//!
//! A 4 000-row queue or an 8 000-track library never crosses the boundary
//! whole: Rust keeps the rows, the screen keeps a `ListHandle`, and
//! `window(start, count)` hands it the pages it is about to paint. Three things
//! follow from that, and each is measured in this file's tests:
//!
//! - **Filtering and sorting are `appcore`'s**, not a second spelling of them:
//!   every row's searchable text is folded ONCE (`appcore::fold`, the web app's
//!   own table, so "beyonce" finds "Beyoncé" and É sorts with the E's), and a
//!   keystroke is one pass over folded bytes.
//! - **A change reaches every visible list in the same frame.** `Registry`
//!   holds WEAK references to the open lists (a list the screen dropped must
//!   not be kept alive by the registry), and `update_rows(key, f)` patches the
//!   row in every list that carries it and bumps each list's `version`, so a
//!   like/dislike (MB10) or a finished download (MB15) redraws without a
//!   re-fetch.
//! - **Memory is counted, in bytes.** The rows, their strings and the folded
//!   text are the list's own (`List::memory_bytes`), which is what
//!   `NzCore::memory()` reports — never the RSS of a shared test process.

use std::sync::{Mutex, RwLock, Weak};

use appcore::fold::{contains, fold_into};

/// One row of any track list, shaped the way the row is DRAWN: title, subtitle,
/// duration, cover key, flags. Not the server's JSON — the feature lots convert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// `<source>:<id>` for a track, or a queue position the server has no id
    /// for. `nz-ffi`'s `TrackRow::key` is this value verbatim.
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
    /// The provider cannot serve it right now — shown, not hidden.
    pub unavailable: bool,
    pub downloaded: bool,
    /// `deezer`, `youtube`, `upload`, `local` — what the source chip says.
    pub source: String,
    /// Position in the underlying list, so a drag can name a target index.
    pub position: u32,
    /// When it was last played, in milliseconds, for `SortKey::Played`. Not
    /// drawn, and 0 for a row that never was.
    pub played_at: u64,
    /// When it was added, in milliseconds, for `SortKey::Added`.
    pub added_at: u64,
}

impl Row {
    /// What a search matches, folded once: the title, the credited names and
    /// the album, separated exactly as `appcore`'s track index separates them
    /// (U+001F between fields) so a query never runs two fields together.
    fn haystack(&self) -> Vec<u8> {
        let mut folded =
            Vec::with_capacity(self.title.len() + self.artists.len() + self.album.len() + 2);
        for (index, field) in [&self.title, &self.artists, &self.album]
            .into_iter()
            .enumerate()
        {
            if index > 0 {
                folded.push(0x1F);
            }
            fold_into(field.as_bytes(), &mut folded);
        }
        folded
    }

    /// A row a test or a bench builds without the server: the shape only.
    pub fn test_row(number: usize, title: &str) -> Row {
        Row {
            key: format!("test:{number}"),
            id: number.to_string(),
            title: title.to_owned(),
            artists: format!("Artiste {number}"),
            album: format!("Album {number}"),
            album_id: Some(format!("album-{number}")),
            duration_s: 180 + (number as u32 % 120),
            cover: None,
            explicit: false,
            liked: false,
            disliked: false,
            unavailable: false,
            downloaded: false,
            source: "deezer".to_owned(),
            position: number as u32,
            played_at: 0,
            added_at: number as u64,
        }
    }

    /// The bytes this row owns, counted (strings by capacity, the row by size).
    fn memory_bytes(&self) -> usize {
        std::mem::size_of::<Row>()
            + self.key.capacity()
            + self.id.capacity()
            + self.title.capacity()
            + self.artists.capacity()
            + self.album.capacity()
            + self.album_id.as_ref().map_or(0, String::capacity)
            + self.cover.as_ref().map_or(0, String::capacity)
            + self.source.capacity()
    }
}

/// The order a list is shown in. `appcore`'s five sorts, plus the list's own
/// order and the last-played order the history screen needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Title,
    Artist,
    Album,
    Duration,
    Added,
    Position,
    Played,
}

impl SortKey {
    /// The folded field a real sort compares, or `None` for a sort that reads a
    /// number (`Duration`, `Added`, `Played`) or the list's own order.
    fn folded_field(self) -> Option<fn(&Prepared) -> &[u8]> {
        match self {
            SortKey::Title => Some(|row: &Prepared| row.title.as_slice()),
            SortKey::Artist => Some(|row: &Prepared| row.artists.as_slice()),
            SortKey::Album => Some(|row: &Prepared| row.album.as_slice()),
            SortKey::Duration | SortKey::Added | SortKey::Position | SortKey::Played => None,
        }
    }

    fn number(self, row: &Row) -> u64 {
        match self {
            SortKey::Duration => u64::from(row.duration_s),
            SortKey::Added => row.added_at,
            SortKey::Played => row.played_at,
            _ => 0,
        }
    }
}

/// One row as sorting needs it: its folded keys and its position in the list.
#[derive(Debug, Clone)]
struct Prepared {
    /// The row's position in `List::rows`.
    at: usize,
    hay: Vec<u8>,
    title: Vec<u8>,
    artists: Vec<u8>,
    album: Vec<u8>,
}

/// What a list holds, behind one lock so a patch is one frame.
#[derive(Debug)]
struct State {
    rows: Vec<Row>,
    /// One entry per row, in the list's own order.
    prepared: Vec<Prepared>,
    /// The query as it was last folded, empty when nothing is filtered.
    folded_query: Vec<u8>,
    /// The row positions the screen may draw, in display order.
    visible: Vec<u32>,
    sort: SortKey,
    descending: bool,
    version: u64,
}

impl State {
    /// Recomputes the folded text (after a row changed).
    fn prepare(&mut self) {
        self.prepared = self
            .rows
            .iter()
            .enumerate()
            .map(|(at, row)| Prepared {
                at,
                hay: row.haystack(),
                title: fold(&row.title),
                artists: fold(&row.artists),
                album: fold(&row.album),
            })
            .collect();
    }

    /// Recomputes the display order from the query and the sort. Stable, and
    /// ties keep the list's own order in BOTH directions — `appcore`'s rule, so
    /// a reversed title sort is not also a reversed playlist.
    fn reorder(&mut self) {
        let mut visible: Vec<u32> = (0..self.rows.len() as u32).collect();
        if !self.folded_query.is_empty() {
            visible.retain(|at| contains(&self.prepared[*at as usize].hay, &self.folded_query));
        }
        if self.sort != SortKey::Position {
            let descending = self.descending;
            let field = self.sort.folded_field();
            let prepared = &self.prepared;
            visible.sort_by(|left, right| {
                let left = &prepared[*left as usize];
                let right = &prepared[*right as usize];
                let order = match field {
                    Some(field) => {
                        let (a, b) = (field(left), field(right));
                        if descending { b.cmp(a) } else { a.cmp(b) }
                    }
                    None => {
                        let (a, b) = (
                            self.sort.number(&self.rows[left.at]),
                            self.sort.number(&self.rows[right.at]),
                        );
                        if descending { b.cmp(&a) } else { a.cmp(&b) }
                    }
                };
                // Ties keep the list's order, in both directions.
                order.then(left.at.cmp(&right.at))
            });
        } else if self.descending {
            // The list's own order, reversed — the SPA's `SORT_DEFAULT` in the
            // other direction.
            visible.reverse();
        }
        self.visible = visible;
        self.version += 1;
    }

    fn memory_bytes(&self) -> usize {
        self.rows.iter().map(Row::memory_bytes).sum::<usize>()
            + self.rows.capacity() * std::mem::size_of::<Row>()
            + self
                .prepared
                .iter()
                .map(|row| {
                    std::mem::size_of::<Prepared>()
                        + row.hay.capacity()
                        + row.title.capacity()
                        + row.artists.capacity()
                        + row.album.capacity()
                })
                .sum::<usize>()
            + self.folded_query.capacity()
            + self.visible.capacity() * std::mem::size_of::<u32>()
    }
}

fn fold(text: &str) -> Vec<u8> {
    let mut folded = Vec::with_capacity(text.len());
    fold_into(text.as_bytes(), &mut folded);
    folded
}

/// A windowed handle over one list. All methods are cheap and thread-safe: the
/// UI calls `window` from its own thread while a network answer patches rows on
/// the runtime's.
#[derive(Debug)]
pub struct List {
    state: RwLock<State>,
}

impl List {
    /// A list over `rows`, in the order given.
    pub fn new(rows: Vec<Row>) -> List {
        let mut state = State {
            rows,
            prepared: Vec::new(),
            folded_query: Vec::new(),
            visible: Vec::new(),
            sort: SortKey::Position,
            descending: false,
            version: 1,
        };
        state.prepare();
        state.visible = (0..state.rows.len() as u32).collect();
        List {
            state: RwLock::new(state),
        }
    }

    /// The number of rows the list has NOW (after a filter).
    pub fn len(&self) -> usize {
        self.state
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .visible
            .len()
    }

    /// Whether nothing is shown. A sibling of `len`, not a second spelling of
    /// it: the FFI asks `len() == 0`.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `count` rows from `start` in display order, at most the list's length.
    pub fn window(&self, start: u32, count: u32) -> Vec<Row> {
        let state = self.state.read().unwrap_or_else(|err| err.into_inner());
        let start = start as usize;
        if start >= state.visible.len() || count == 0 {
            return Vec::new();
        }
        let end = (start + count as usize).min(state.visible.len());
        state.visible[start..end]
            .iter()
            .map(|at| state.rows[*at as usize].clone())
            .collect()
    }

    /// The display position of the row carrying `key`, or `None` when the
    /// filter hid it (or the list never had it).
    pub fn index_of(&self, key: &str) -> Option<u32> {
        let state = self.state.read().unwrap_or_else(|err| err.into_inner());
        state
            .visible
            .iter()
            .position(|at| state.rows[*at as usize].key == key)
            .map(|position| position as u32)
    }

    /// Folds the list with `appcore::fold` and returns the new length. An empty
    /// query clears the filter.
    pub fn filter(&self, query: &str) -> usize {
        let mut state = self.state.write().unwrap_or_else(|err| err.into_inner());
        state.folded_query = fold(query.trim());
        state.reorder();
        state.visible.len()
    }

    /// Sorts by `by`, keeping the current filter, and returns the new length.
    pub fn sort(&self, by: SortKey, descending: bool) -> usize {
        let mut state = self.state.write().unwrap_or_else(|err| err.into_inner());
        state.sort = by;
        state.descending = descending;
        state.reorder();
        state.visible.len()
    }

    /// Changes on every filter, sort or patch; the UI re-windows when it moves.
    pub fn version(&self) -> u64 {
        self.state
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .version
    }

    /// The counted bytes this list holds (`NzCore::memory`'s `lists_bytes`).
    pub fn memory_bytes(&self) -> usize {
        self.state
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .memory_bytes()
    }

    /// Applies `f` to every row carrying `key`, recomputes the folded text when
    /// one changed, and bumps the version. Answers how many rows changed.
    ///
    /// The sorted order is kept: patching `liked` must not make a list jump.
    pub fn patch(&self, key: &str, f: &mut dyn FnMut(&mut Row)) -> usize {
        let mut state = self.state.write().unwrap_or_else(|err| err.into_inner());
        let targets: Vec<usize> = state
            .prepared
            .iter()
            .filter(|prepared| state.rows[prepared.at].key == key)
            .map(|prepared| prepared.at)
            .collect();
        for at in &targets {
            f(&mut state.rows[*at]);
        }
        if !targets.is_empty() {
            state.prepare();
            if !state.folded_query.is_empty() {
                state.reorder();
            } else {
                state.version += 1;
            }
        }
        targets.len()
    }
}

/// Every open list, by WEAK reference: the registry never keeps a list a screen
/// dropped alive, and a patch reaches the ones still on a screen.
#[derive(Debug, Default)]
pub struct Registry {
    lists: Mutex<Vec<Weak<List>>>,
}

impl Registry {
    pub fn new() -> Registry {
        Registry::default()
    }

    /// Registers a list the caller keeps (the FFI handle owns the strong `Arc`).
    pub fn open(&self, rows: Vec<Row>) -> std::sync::Arc<List> {
        let list = std::sync::Arc::new(List::new(rows));
        self.adopt(&list);
        list
    }

    /// Registers a list somebody else built.
    pub fn adopt(&self, list: &std::sync::Arc<List>) {
        let mut lists = self.lists.lock().unwrap_or_else(|err| err.into_inner());
        lists.retain(|weak| weak.strong_count() > 0);
        lists.push(std::sync::Arc::downgrade(list));
    }

    /// Applies `f` to every open row carrying `key` — what MB10's like/dislike
    /// and MB15's "downloaded" flag call, so every visible list changes in the
    /// same frame without a re-fetch. Answers how many rows changed in total.
    pub fn update_rows(&self, key: &str, mut f: impl FnMut(&mut Row)) -> usize {
        let live: Vec<std::sync::Arc<List>> = {
            let lists = self.lists.lock().unwrap_or_else(|err| err.into_inner());
            lists.iter().filter_map(Weak::upgrade).collect()
        };
        let mut changed = 0;
        for list in live {
            changed += list.patch(key, &mut f);
        }
        changed
    }

    /// The number of lists a screen still holds.
    pub fn live(&self) -> usize {
        let mut lists = self.lists.lock().unwrap_or_else(|err| err.into_inner());
        lists.retain(|weak| weak.strong_count() > 0);
        lists.len()
    }

    /// The counted bytes of every live list.
    pub fn memory_bytes(&self) -> usize {
        let live: Vec<std::sync::Arc<List>> = {
            let lists = self.lists.lock().unwrap_or_else(|err| err.into_inner());
            lists.iter().filter_map(Weak::upgrade).collect()
        };
        live.iter().map(|list| list.memory_bytes()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn rows(count: usize) -> Vec<Row> {
        (0..count)
            .map(|n| Row::test_row(n, &format!("Titre {n}")))
            .collect()
    }

    /// The acceptance: 8 000 rows, one window of 64, and the count `appcore`'s
    /// own fold answers for the same query.
    #[test]
    fn eight_thousand_rows_window_in_pages_and_filter_like_appcore() {
        let list = List::new(rows(8_000));
        assert_eq!(list.len(), 8_000);
        let window = list.window(0, 64);
        assert_eq!(window.len(), 64, "a page is what the UI asked for");
        assert_eq!(window[0].position, 0);
        assert_eq!(window[63].position, 63);
        // Past the end: what is left, never a panic.
        assert_eq!(list.window(7_990, 64).len(), 10);
        assert!(list.window(8_000, 64).is_empty());
        assert!(list.window(0, 0).is_empty());

        // The expected count, computed the way appcore computes it.
        let query = "titre 1";
        let mut folded = Vec::new();
        fold_into(query.as_bytes(), &mut folded);
        let expected = rows(8_000)
            .iter()
            .filter(|row| contains(&row.haystack(), &folded))
            .count();
        assert_eq!(list.filter(query), expected);
        assert_eq!(list.len(), expected);
        assert!(expected > 0);
        // An empty query clears the filter.
        assert_eq!(list.filter(""), 8_000);
    }

    /// The accent rules are `appcore`'s own (`fold.rs`), which is the whole
    /// reason the phone uses that crate instead of `toLowerCase`.
    #[test]
    fn a_search_without_accents_finds_them_and_ligatures_are_spelled_out() {
        let mut rows = rows(0);
        rows.push(Row {
            title: "Beyoncé".to_owned(),
            artists: "Beyoncé Jay-Z".to_owned(),
            album: "Lemonade".to_owned(),
            ..Row::test_row(0, "Beyoncé")
        });
        rows.push(Row {
            title: "Été".to_owned(),
            artists: "Œil".to_owned(),
            album: "Album".to_owned(),
            ..Row::test_row(1, "Été")
        });
        let list = List::new(rows);
        assert_eq!(list.filter("beyonce"), 1);
        assert_eq!(list.index_of("test:0"), Some(0));
        assert_eq!(list.filter("OEIL"), 1, "the ligature is spelled out");
        assert_eq!(list.index_of("test:1"), Some(0), "the filter's own order");
        assert_eq!(list.filter("jay-z"), 1, "a featured name is findable");
        assert_eq!(list.filter("calbum"), 0, "fields never run together");
    }

    /// Sorting is folded-byte order, ties keep the list's order in BOTH
    /// directions, and the filter survives a sort.
    #[test]
    fn sort_uses_folded_keys_and_keeps_ties_in_list_order() {
        let mut rows = rows(0);
        for (title, position) in [
            ("Zazie", 0u32),
            ("Éric", 1),
            ("abc", 2),
            ("Été", 3),
            ("abc", 4),
        ] {
            rows.push(Row {
                title: title.to_owned(),
                artists: String::new(),
                album: String::new(),
                position,
                ..Row::test_row(position as usize, title)
            });
        }
        let list = List::new(rows);
        assert_eq!(list.len(), 5);
        let titles = |list: &List| -> Vec<String> {
            list.window(0, 16)
                .into_iter()
                .map(|row| row.title)
                .collect()
        };
        assert_eq!(list.sort(SortKey::Title, false), 5);
        assert_eq!(titles(&list), ["abc", "abc", "Éric", "Été", "Zazie"]);
        assert_eq!(
            list.sort(SortKey::Title, true),
            5,
            "the length does not change"
        );
        assert_eq!(titles(&list), ["Zazie", "Été", "Éric", "abc", "abc"]);

        // A filter, then a sort: the filter is kept.
        assert_eq!(list.filter("e"), 3, "Éric, Été and Zazie are the e's");
        assert_eq!(list.sort(SortKey::Title, false), 3);
        assert_eq!(titles(&list), ["Éric", "Été", "Zazie"]);
        assert_eq!(list.sort(SortKey::Position, false), 3);
        // The list's own order for those three rows is Zazie (0), Éric (1),
        // Été (3) — a sort back to "position" restores it.
        assert_eq!(
            titles(&list),
            ["Zazie", "Éric", "Été"],
            "the list's own order"
        );
    }

    /// A patch reaches every open list in the same frame and bumps the version,
    /// without a re-fetch and without the registry keeping a dropped list alive.
    #[test]
    fn a_patch_reaches_every_open_list_and_no_dropped_one() {
        let registry = Registry::new();
        let favourites = registry.open(rows(3));
        let queue = registry.open(rows(3));
        assert_eq!(registry.live(), 2);

        let before = favourites.version();
        let changed = registry.update_rows("test:1", |row| row.liked = true);
        assert_eq!(changed, 2, "one row in each list");
        assert!(favourites.version() > before, "the UI re-windows");
        assert!(favourites.window(0, 3)[1].liked);
        assert!(queue.window(0, 3)[1].liked);

        // A key nobody shows changes nothing.
        let quiet = registry.update_rows("test:99", |row| row.liked = false);
        assert_eq!(quiet, 0);

        // A dropped list is forgotten, and the registry does not keep it alive.
        drop(queue);
        assert_eq!(registry.live(), 1);
        let changed = registry.update_rows("test:1", |row| row.downloaded = true);
        assert_eq!(changed, 1, "only the screen that still shows it");
    }

    /// Memory is counted from the rows themselves, and a dropped list counts 0.
    #[test]
    fn memory_is_the_lists_own_counted_bytes() {
        let registry = Registry::new();
        let list = registry.open(rows(1_000));
        let counted = list.memory_bytes();
        assert!(
            counted > 1_000 * std::mem::size_of::<Row>(),
            "1 000 rows count at least their own size: {counted}"
        );
        assert_eq!(registry.memory_bytes(), counted);
        drop(list);
        assert_eq!(registry.memory_bytes(), 0);
    }

    /// `window` hands out copies: a later patch does not rewrite a row a screen
    /// is drawing, so Compose's `TrackRow` never changes under its own key.
    #[test]
    fn a_window_is_a_copy_not_a_view() {
        let registry = Registry::new();
        let list: Arc<List> = registry.open(rows(2));
        let page = list.window(0, 2);
        registry.update_rows("test:0", |row| row.title = "Changé".to_owned());
        assert_eq!(page[0].title, "Titre 0");
        assert_eq!(list.window(0, 1)[0].title, "Changé");
    }
}
