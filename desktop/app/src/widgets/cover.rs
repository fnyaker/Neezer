//! Sleeves: fetch one, keep it on the device, paint it — or its placeholder.
//!
//! A cover is a file `nz-cache` holds (`covers.put` sniffs its format and writes
//! `<FNV-1a-64(url)>.<ext>`), and this widget is the only place that reads one.
//! The rules it follows are the web app's `lib/coverqueue.js` and rule 1:
//!
//! - **Visible first, bounded.** A screen asks for the sleeves of the rows it is
//!   ACTUALLY showing (`Covers::want` from the list's visible-range callback),
//!   at most [`MAX_IN_FLIGHT`] at a time; a hidden row costs nothing.
//! - **A screen never waits on a sleeve.** The placeholder paints in the same
//!   frame, the image replaces it when it lands, and the *data* request is what
//!   the screen waits for.
//! - **What was fetched once is kept**: the second visit finds the file and
//!   makes **0 requests** (measured in the D8a tests).
//! - **A sleeve is decoded at the size it is SEEN, and freed when it is not
//!   seen.** GPUI uploads a painted image into one atlas tile at the image's
//!   NATURAL size, keyed by the image's id, and releases that tile only when the
//!   window goes away (`Window::paint_image` / `drop_image`); the decoded pixels
//!   live in the `Arc<RenderImage>` (GPUI's own asset cache never evicts either).
//!   So this module owns both: [`Sleeves`] decodes the 128 px copy off the paint
//!   thread, keeps the `Arc` while a screen shows it, paints it through
//!   `ImageSource::Render`, and drops the tile AND the `Arc` when the screen
//!   leaves or when the byte budget forces the LRU. Measured on the 4 000-row
//!   favourites list (docs/agents/desktop.md, *Memory*): a 1 000 px sleeve in a
//!   36 px row is 4 MB of heap + 4 MB of atlas; the 128 px copy is 64 KB.
//!
//! The bytes come from `Api::bytes` on the `User` lane — a sleeve for what is on
//! screen is part of the action just taken, not background work — and the cache
//! write happens on the network thread, never on GPUI's.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use gpui_kit::{
    App, Asset as _, Global, ImageSource, ImgResourceLoader, IntoElement, ObjectFit,
    ParentElement, Pixels, RenderImage, Resource, SharedString, Styled, StyledImage as _, Window,
    img, px,
};
use gpui_omarchy::{IconName, icon};
use nz_api::Lane;

use crate::state::AppState;
use crate::theme;

/// How many sleeves one screen asks for at once. Six is `Lane::User`'s own slot
/// count, so two more are queued and start as the first ones land: enough for a
/// list's first screenful without ever being a fan-out (rule 3).
pub const MAX_IN_FLIGHT: usize = 8;

/// How many bytes of DECODED sleeves the app keeps at once.
///
/// The web app's own cap (`lib/coverqueue.js`): 64 MB. A row's sleeve is a
/// 128 px copy, so 64 KB decoded — the budget holds a thousand of them, which
/// is more than a scroll pass shows and far less than a 4 000-row list would
/// take (256 MB). What a screen is showing RIGHT NOW is never evicted, however
/// full the cache is.
pub const DECODED_BUDGET: usize = 64 * 1024 * 1024;

/// How long a sleeve nobody shows is kept before its tile and its pixels go.
///
/// A row that leaves the window is freed when it has been out for this long —
/// long enough that scrolling back and forth over a boundary costs nothing,
/// short enough that "I looked at a screen once" is not a permanent 64 MB. A
/// sleeve a screen still shows is never freed.
pub const IDLE_FOR: Duration = Duration::from_secs(2);

/// The size class a screen draws its sleeves at.
///
/// The FILE the app paints is a copy at this size (`nz-cache`'s
/// `Covers::variant`), because GPUI uploads a painted image into an atlas tile
/// at its NATURAL size and never drops that tile on its own: a 1 000 px sleeve
/// in a 36 px row is 4 MB of decoded heap and 4 MB of atlas for 1 296 useful
/// pixels. Measured on the 4 000-row favourites list at 2560×1600 (see
/// docs/agents/desktop.md, *Memory*).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Class {
    /// A row's thumbnail (2.25 rem ≈ 36 px), the hot path of a long list.
    Row,
    /// A page's cards and shelves (≤ 220 px).
    #[default]
    Card,
    /// A page's own sleeve (the artist/album header), ≤ 340 px.
    Page,
}

impl Class {
    /// The longest side of the file this class paints, in physical pixels:
    /// about 3.5× the logical box, so a HiDPI panel still has real pixels to
    /// sample (a 36 px row reads a 128 px file).
    pub fn px(self) -> u32 {
        match self {
            Class::Row => 128,
            Class::Card => 384,
            Class::Page => 512,
        }
    }
}

/// What one screen's sleeves are, and which of them are in flight.
///
/// Held by the screen (not global) so a closed screen makes no more requests,
/// and so two screens never fight over the same in-flight map. The DECODED
/// images are process-wide ([`Sleeves`]): one sleeve shown in two places is
/// decoded once, and the screen that shows it says so every paint
/// ([`Covers::show`]) so it is never evicted from under it. Two caches on one
/// screen (a page's banner at `Class::Card`, its rows at `Class::Row`) are two
/// holders, each with its own shown set.
pub struct Covers {
    /// Relative path → the file the device has, at this screen's [`Class`].
    files: HashMap<String, PathBuf>,
    /// The size this screen draws its sleeves at.
    class: Class,
    /// Asked and refused: no second request this visit (a dead sleeve is not a
    /// retry storm), and the row keeps its placeholder.
    refused: HashSet<String>,
    inflight: HashSet<String>,
    /// This cache's own name in the process-wide decoded cache, so two caches
    /// never share one "what this screen shows" set.
    holder: u64,
}

/// The next cache's holder: a process-wide counter, never reused (a released
/// screen's set is dropped with its number).
static HOLDERS: AtomicU64 = AtomicU64::new(1);

impl Default for Covers {
    fn default() -> Self {
        Self::new()
    }
}

impl Covers {
    /// A screen whose sleeves are cards or a page header.
    pub fn new() -> Self {
        Self::with_class(Class::Card)
    }

    /// A screen that draws its sleeves in LIST ROWS: the file it paints is the
    /// small one (`Class::Row`), which is what keeps a 4 000-row list's decoded
    /// images and atlas tiles at a row's size instead of a sleeve's.
    pub fn rows() -> Self {
        Self::with_class(Class::Row)
    }

    fn with_class(class: Class) -> Self {
        Self {
            files: HashMap::new(),
            class,
            refused: HashSet::new(),
            inflight: HashSet::new(),
            holder: HOLDERS.fetch_add(1, Ordering::SeqCst),
        }
    }

    /// The size class this screen paints at.
    pub fn class(&self) -> Class {
        self.class
    }

    /// The file the device already holds for `url`, if this screen knows it.
    ///
    /// A pure map read: no SQLite, no network, no allocation on the paint path.
    pub fn file(&self, url: &str) -> Option<&PathBuf> {
        self.files.get(url)
    }

    /// How many sleeves this screen is still waiting for.
    pub fn in_flight(&self) -> usize {
        self.inflight.len()
    }

    /// Ask for `url` unless it is here, in flight, or was refused. `done` runs on
    /// the screen's own event channel when the answer lands (file or refusal).
    ///
    /// Nothing is asked when the screen is offline: the placeholder is the right
    /// answer, and a doomed request per row is what rule 3 forbids.
    pub fn want<F>(&mut self, state: &AppState, url: &str, done: F)
    where
        F: FnOnce(Option<PathBuf>) + Send + 'static,
    {
        if url.is_empty()
            || self.files.contains_key(url)
            || self.refused.contains(url)
            || self.inflight.contains(url)
            || self.inflight.len() >= MAX_IN_FLIGHT
            || !state.online
        {
            return;
        }
        let (Some(api), Some(cache)) = (state.api.clone(), state.cache.clone()) else {
            return;
        };
        self.inflight.insert(url.to_owned());
        let path = format!("/cover/{}", url.trim_start_matches("/api/cover/"));
        let key = url.to_owned();
        let px = self.class.px();
        state.runtime.spawn(async move {
            // The file may already be there from an earlier visit (`nz-cache`
            // keeps sleeves): one SQLite read on the network thread, no request.
            let cached = cache.covers().file(&key);
            let answer = match cached {
                Some(file) => Some(file),
                None => match api.bytes(Lane::User, &path).await {
                    Ok(bytes) => match cache.covers().put(&key, &bytes) {
                        Ok(file) => Some(file),
                        Err(error) => {
                            tracing::debug!(%error, "la vignette n'a pas pu être écrite");
                            None
                        }
                    },
                    Err(error) => {
                        // A failure to reach a provider is never a verdict:
                        // the row keeps its placeholder for this visit only.
                        tracing::debug!(%error, url = %key, "vignette indisponible");
                        None
                    }
                },
            };
            // Decode and resize ONCE, at the size this screen draws (`Class`):
            // a row reads a 128 px file instead of the archived 1 000 px one.
            let answer = answer.and_then(|_| cache.covers().variant(&key, px));
            done(answer);
        });
    }

    /// The answer to an earlier `want`, so the next paint draws the image.
    ///
    /// `None` marks the sleeve refused: the placeholder stays and no further
    /// request is made for it this visit.
    pub fn resolved(&mut self, url: &str, file: Option<PathBuf>) {
        self.inflight.remove(url);
        match file {
            Some(file) => {
                self.files.insert(url.to_owned(), file);
            }
            None => {
                self.refused.insert(url.to_owned());
            }
        }
    }

    /// Forget what was asked, so a `Réessayer` asks again — a screen that came
    /// back online must not keep the placeholders a dead link left.
    pub fn retry_failed(&mut self) {
        self.refused.clear();
    }

    /// The frame is starting: settle what the LAST frame showed.
    ///
    /// One screen's paint can run its list closure more than once (a virtual
    /// list measures, then builds), so what a screen shows is the UNION of the
    /// frame's calls (`show`), and this is where the previous union is retired:
    /// the sleeves that left the window are forgotten, the pixels and atlas
    /// tiles of the ones nothing shows any more are freed, and the cache is
    /// brought back inside [`DECODED_BUDGET`].
    pub fn begin_frame(&mut self, window: &mut Window, cx: &mut App) {
        let holder = self.holder;
        // A sleeve whose decode never landed leaves GPUI's own asset entry
        // behind (`img(path)` fills it while this cache has nothing yet), and
        // that cache never evicts: it is cancelled here instead of being found
        // again in a minute's RSS.
        for path in cx.default_global::<Sleeves>().retire(holder) {
            if cx.default_global::<Sleeves>().cancel(&path) {
                cx.remove_asset::<ImgResourceLoader>(&Resource::Path(path.into()));
            }
        }
        let freed = cx.default_global::<Sleeves>().evict();
        if std::env::var_os("NZ_COVER_DEBUG").is_some() {
            let resident = cx.default_global::<Sleeves>().resident();
            eprintln!("DBG begin_frame holder={holder} freed={} resident={resident:?}", freed.len());
        }
        for (path, image) in freed {
            // `Some(window)` because during a paint the window is taken OUT of
            // `App::windows`: without it the tile of the window being drawn is
            // the one left behind.
            cx.drop_image(image, Some(window));
            cx.remove_asset::<ImgResourceLoader>(&Resource::Path(path.into()));
        }
    }

    /// A sleeve this screen is showing in the frame being built: the file of a
    /// row or a card it just built.
    ///
    /// Costs nothing for a file already decoded; the first time it starts the
    /// background decode of the 128 px copy (a JPEG is ~0.2 ms on the
    /// background executor, one per file per visit — never on the paint thread).
    /// The image is never evicted while a screen shows it.
    pub fn show(&mut self, shown: impl IntoIterator<Item = PathBuf>, cx: &mut App) {
        let holder = self.holder;
        let shown: HashSet<PathBuf> = shown.into_iter().collect();
        cx.default_global::<Sleeves>().show(holder, shown);
        for path in cx.default_global::<Sleeves>().claim(holder) {
            let decode = ImgResourceLoader::load(Resource::Path(path.clone().into()), cx);
            cx.spawn(async move |cx| match decode.await {
                Ok(image) => cx.update(|cx| {
                    // Kept only if a screen still shows it: a sleeve scrolled
                    // away before its decode landed is dropped here, never
                    // remembered.
                    if cx.default_global::<Sleeves>().land(&path, image) {
                        cx.refresh_windows();
                    }
                }),
                Err(error) => {
                    tracing::debug!(
                        %error,
                        path = %path.display(),
                        "la pochette n'a pas pu être décodée"
                    );
                    cx.update(|cx| cx.default_global::<Sleeves>().fail(&path));
                }
            })
            .detach();
        }
    }

    /// The screen is going away: free every image it was the last to show, and
    /// drop their atlas tiles.
    pub fn release(&mut self, cx: &mut App) {
        for (path, image) in cx.default_global::<Sleeves>().release(self.holder) {
            cx.drop_image(image, None);
            cx.remove_asset::<ImgResourceLoader>(&Resource::Path(path.into()));
        }
    }
}

/// What to paint for one sleeve file, this frame.
#[derive(Clone)]
pub enum Look {
    /// Decoded: the tile is in the atlas, paint it.
    Ready(Arc<RenderImage>),
    /// A screen asked for it and the decode is running: the placeholder holds
    /// the box for a frame or two.
    Loading,
    /// The file is not a picture: the placeholder stays, nothing is asked again.
    Failed,
    /// No screen runs this path's decoder (a card shelf, the player bar): GPUI's
    /// own loader paints the file, as it always has.
    Unknown,
}

/// One screen's shown sleeves: the last frame's union and the one being built.
#[derive(Default, Clone)]
struct Shows {
    previous: HashSet<PathBuf>,
    current: HashSet<PathBuf>,
}

impl Shows {
    fn holds(&self, path: &Path) -> bool {
        self.previous.contains(path) || self.current.contains(path)
    }
}

struct Painted {
    image: Arc<RenderImage>,
    /// What the decoded pixels hold: every frame's buffer, BGRA.
    bytes: usize,
    /// The last paint that showed it: what the LRU orders on.
    used: Instant,
}

#[derive(Default)]
struct Inner {
    painted: HashMap<PathBuf, Painted>,
    /// What each cache showed: the frame just painted (`previous`) and the one
    /// being built (`current`). An image in either is never evicted.
    showing: HashMap<u64, Shows>,
    /// Decodes on the background executor, so one paint never asks twice.
    decoding: HashSet<PathBuf>,
    /// Files this process could not decode: never retried, the placeholder is
    /// the answer (a corrupt cover must not be a decode loop).
    broken: HashSet<PathBuf>,
    bytes: usize,
}

/// The decoded sleeves, process-wide, bounded in BYTES.
pub struct Sleeves {
    inner: RefCell<Inner>,
    budget: usize,
    idle: Duration,
}

impl Global for Sleeves {}

impl Default for Sleeves {
    fn default() -> Self {
        Self::new()
    }
}

impl Sleeves {
    pub fn new() -> Self {
        Self::with_rules(DECODED_BUDGET, IDLE_FOR)
    }

    /// The same cache with other rules: a test that must SEE the LRU run uses a
    /// few kilobytes, and one that must see the idle sweep uses zero seconds.
    pub fn with_rules(budget: usize, idle: Duration) -> Self {
        Self {
            inner: RefCell::new(Inner::default()),
            budget,
            idle,
        }
    }

    /// What to paint for `path` now, bumping its recency. A pure map read: the
    /// paint path never decodes, allocates or touches SQLite.
    pub fn lookup(&self, path: &Path) -> Look {
        let mut inner = self.inner.borrow_mut();
        let now = Instant::now();
        if let Some(item) = inner.painted.get_mut(path) {
            item.used = now;
            return Look::Ready(item.image.clone());
        }
        if inner.decoding.contains(path) {
            return Look::Loading;
        }
        if inner.broken.contains(path) {
            return Look::Failed;
        }
        Look::Unknown
    }

    /// For a test's diagnosis: the painted files and each holder's two sets.
    pub fn debug_state(&self) -> (Vec<PathBuf>, Vec<(u64, Vec<PathBuf>, Vec<PathBuf>)>) {
        let inner = self.inner.borrow();
        (
            inner.painted.keys().cloned().collect(),
            inner
                .showing
                .iter()
                .map(|(holder, shows)| {
                    (
                        *holder,
                        shows.previous.iter().cloned().collect(),
                        shows.current.iter().cloned().collect(),
                    )
                })
                .collect(),
        )
    }

    /// The decoded bytes held, and how many images (the number a test asserts).
    pub fn resident(&self) -> (usize, usize) {
        let inner = self.inner.borrow();
        (inner.bytes, inner.painted.len())
    }

    /// Retire the frame that was just painted and return what it showed that no
    /// other screen does: the caller cancels the decodes of those still in
    /// flight, then `evict` frees what nothing shows.
    fn retire(&self, holder: u64) -> Vec<PathBuf> {
        let mut inner = self.inner.borrow_mut();
        let done = {
            let shows = inner.showing.entry(holder).or_default();
            std::mem::take(&mut shows.current)
        };
        let left: Vec<PathBuf> = done
            .iter()
            .filter(|path| {
                !inner
                    .showing
                    .iter()
                    .any(|(other, shows)| *other != holder && shows.holds(path))
            })
            .cloned()
            .collect();
        if let Some(shows) = inner.showing.get_mut(&holder) {
            shows.previous = done;
        }
        left
    }

    /// Union in what a paint call built this frame.
    fn show(&self, holder: u64, shown: HashSet<PathBuf>) {
        let mut inner = self.inner.borrow_mut();
        let shows = inner.showing.entry(holder).or_default();
        shows.current.extend(shown);
    }

    fn claim(&self, holder: u64) -> Vec<PathBuf> {
        let mut inner = self.inner.borrow_mut();
        let Some(shows) = inner.showing.get(&holder).cloned() else {
            return Vec::new();
        };
        let shown: HashSet<PathBuf> = shows.previous.into_iter().chain(shows.current).collect();
        let Inner {
            painted,
            decoding,
            broken,
            ..
        } = &mut *inner;
        let mut out = Vec::new();
        for path in shown {
            if painted.contains_key(&path) || decoding.contains(&path) || broken.contains(&path) {
                continue;
            }
            decoding.insert(path.clone());
            out.push(path);
        }
        // Sorted, so two runs of the same screen ask in the same order (a
        // `HashSet`'s own order is not a contract, and the bench compares runs).
        out.sort();
        out
    }

    /// A decode that landed. `false` means no screen shows it any more: the
    /// caller lets the `Arc` go.
    fn land(&self, path: &Path, image: Arc<RenderImage>) -> bool {
        let mut inner = self.inner.borrow_mut();
        inner.decoding.remove(path);
        if !inner.showing.values().any(|shows| shows.holds(path)) {
            return false;
        }
        let now = Instant::now();
        let bytes = decoded_bytes(&image);
        if let Some(previous) = inner.painted.remove(path) {
            inner.bytes = inner.bytes.saturating_sub(previous.bytes);
        }
        inner.bytes += bytes;
        inner.painted.insert(
            path.to_path_buf(),
            Painted {
                image,
                bytes,
                used: now,
            },
        );
        true
    }

    fn fail(&self, path: &Path) {
        let mut inner = self.inner.borrow_mut();
        inner.decoding.remove(path);
        inner.broken.insert(path.to_path_buf());
    }

    /// Stop waiting for `path`. `true` when a decode was still running, so its
    /// GPUI asset entry must go with it.
    fn cancel(&self, path: &Path) -> bool {
        let mut inner = self.inner.borrow_mut();
        inner.decoding.remove(path)
    }

    /// The least recently used images no screen shows, until the budget holds.
    /// Everything, when `holder`'s screen is going away.
    fn release(&self, holder: u64) -> Vec<(PathBuf, Arc<RenderImage>)> {
        let mut inner = self.inner.borrow_mut();
        inner.showing.remove(&holder);
        // Its in-flight decodes are not this cache's to wait for: they land,
        // find no screen showing them, and are dropped by `land`.
        let mut freed = Vec::new();
        let idle: Vec<PathBuf> = inner
            .painted
            .keys()
            .filter(|path| !inner.showing.values().any(|shows| shows.holds(path)))
            .cloned()
            .collect();
        for path in idle {
            if let Some(item) = inner.painted.remove(&path) {
                inner.bytes = inner.bytes.saturating_sub(item.bytes);
                freed.push((path, item.image));
            }
        }
        freed
    }

    fn evict(&self) -> Vec<(PathBuf, Arc<RenderImage>)> {
        let mut inner = self.inner.borrow_mut();
        let mut freed = Vec::new();
        let now = Instant::now();
        // A sleeve no screen shows any more, and has not for `idle`: out.
        let idle: Vec<PathBuf> = inner
            .painted
            .iter()
            .filter(|(path, item)| {
                !inner.showing.values().any(|shows| shows.holds(path))
                    && now.saturating_duration_since(item.used) >= self.idle
            })
            .map(|(path, _)| path.clone())
            .collect();
        for path in idle {
            if let Some(item) = inner.painted.remove(&path) {
                inner.bytes = inner.bytes.saturating_sub(item.bytes);
                freed.push((path, item.image));
            }
        }
        // Then the budget, least recently used first.
        while inner.bytes > self.budget {
            let victim = inner
                .painted
                .iter()
                .filter(|(path, _)| !inner.showing.values().any(|shows| shows.holds(path)))
                .min_by_key(|(_, item)| item.used)
                .map(|(path, _)| path.clone());
            // Everything left is on screen: a visible row wins over the budget
            // (it is twenty-five images, not the list).
            let Some(victim) = victim else { break };
            if let Some(item) = inner.painted.remove(&victim) {
                inner.bytes = inner.bytes.saturating_sub(item.bytes);
                freed.push((victim, item.image));
            }
        }
        freed
    }
}

/// The bytes one decoded image holds, every frame included.
fn decoded_bytes(image: &RenderImage) -> usize {
    (0..image.frame_count())
        .filter_map(|frame| {
            let size = image.size(frame);
            Some(size.width.0.max(0) as usize * size.height.0.max(0) as usize * 4)
        })
        .sum()
}

/// The file the device holds for `url`, as an owned path.
///
/// A screen holds its sleeves behind an `Rc<RefCell<..>>` (the fetch task and
/// the paint share it), so a bare `borrow()` in the middle of a `div()` chain
/// would hand out a reference into a temporary: one function, one clone, and the
/// call sites stay one line.
pub fn file_in(covers: &std::rc::Rc<std::cell::RefCell<Covers>>, url: &str) -> Option<PathBuf> {
    covers.borrow().file(url).cloned()
}

/// Which glyph a placeholder shows, and whether the tile is round.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Album,
    Artist,
    Playlist,
    Podcast,
}

impl Kind {
    fn icon(self) -> IconName {
        match self {
            Kind::Album => IconName::Disc3,
            Kind::Artist => IconName::Music,
            Kind::Playlist => IconName::ListMusic,
            Kind::Podcast => IconName::Mic,
        }
    }
}

/// One sleeve at `size` square pixels.
///
/// `file` is what `Covers::file` answered; `label` only names the placeholder
/// (an artist's initial, an album's title's first letter). A screen that runs
/// [`Covers::show`] paints the image this cache holds (`ImageSource::Render`,
/// so the tile is the one `show` can drop); any other screen keeps GPUI's own
/// loader, which paints the file as it always did.
pub fn view(
    file: Option<&PathBuf>,
    url: Option<&str>,
    label: &str,
    kind: Kind,
    size: Pixels,
    cx: &App,
) -> impl IntoElement + use<> {
    let round = kind == Kind::Artist;
    let mut tile = gpui_kit::div()
        .flex_shrink_0()
        .w(size)
        .h(size)
        .overflow_hidden()
        .bg(theme::chrome(cx))
        .flex()
        .items_center()
        .justify_center();
    tile = if round {
        tile.rounded_full()
    } else {
        tile.rounded_none()
    };
    match file {
        Some(file) => {
            let look = cx
                .try_global::<Sleeves>()
                .map(|sleeves| sleeves.lookup(file))
                .unwrap_or(Look::Unknown);
            let source = match look {
                Look::Ready(image) => Some(ImageSource::Render(image)),
                Look::Unknown => Some(ImageSource::from(file.clone())),
                // The decode is in flight: the placeholder box holds the frame
                // rather than asking GPUI's loader for a second decode of the
                // same file (two tiles, twice the bytes, for one row).
                Look::Loading | Look::Failed => None,
            };
            if let Some(source) = source {
                // The radius goes on the image too: a rounded parent clips its
                // background but not necessarily a child's own drawing (measured
                // on an artist tile: the sleeve stayed a square).
                let mut picture = img(source)
                    .object_fit(ObjectFit::Cover)
                    .w(size)
                    .h(size);
                if round {
                    picture = picture.rounded_full();
                }
                return tile.child(picture);
            }
        }
        None => {}
    }
    let initial = label
        .trim()
        .chars()
        .next()
        .map(|first| first.to_uppercase().collect::<String>())
        .unwrap_or_default();
    let glyph = if url.is_some() && !initial.is_empty() && round {
        gpui_kit::div()
            .text_size(glyph_size(size))
            .text_color(theme::secondary(cx))
            .child(SharedString::from(initial))
    } else {
        gpui_kit::div().child(
            icon(kind.icon())
                .size(glyph_size(size))
                .text_color(theme::secondary(cx).opacity(0.65)),
        )
    };
    tile.child(glyph)
}

/// A glyph or an initial takes about a third of the tile, clamped so a 20 px
/// row thumbnail and a 200 px page sleeve both read.
fn glyph_size(size: Pixels) -> Pixels {
    px((f32::from(size) * 0.34).clamp(10., 64.))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(px: u32) -> Arc<RenderImage> {
        let frame = image::Frame::new(image::RgbaImage::new(px, px));
        Arc::new(RenderImage::new(vec![frame]))
    }

    fn path(name: &str) -> PathBuf {
        PathBuf::from(format!("/tmp/{name}.jpg"))
    }

    /// One paint: what the screen shows, then the frame boundary that retires
    /// it (`Covers::begin_frame`'s two halves, which the tests drive by hand).
    fn frame(sleeves: &Sleeves, holder: u64, shown: &[&PathBuf]) {
        sleeves.show(holder, shown.iter().map(|path| (*path).clone()).collect());
        sleeves.retire(holder);
    }

    /// The rule: the decoded cache is bounded in BYTES, least recently used
    /// first, and it never drops what a screen is showing.
    #[test]
    fn the_decoded_cache_is_bounded_in_bytes_and_evicts_the_least_recent() {
        let screen = 1u64;
        // Three 128 px images are 65 536 B each; a 100 000 B budget holds one.
        // The idle sweep is out of the way here (`IDLE_FOR`), so this test is
        // the budget's own.
        let sleeves = Sleeves::with_rules(100_000, IDLE_FOR);
        assert_eq!(decoded_bytes(&image(128)), 65_536);

        let (a, b, c) = (path("a"), path("b"), path("c"));
        frame(&sleeves, screen, &[&a, &b, &c]);
        for path in [&a, &b, &c] {
            std::thread::sleep(Duration::from_millis(1));
            assert!(sleeves.land(path, image(128)), "kept");
        }
        // Nothing goes while it is shown, even three times over budget.
        assert_eq!(sleeves.resident(), (3 * 65_536, 3));
        assert!(sleeves.evict().is_empty());

        // The next frame shows only `c`: `a` was decoded first and is the least
        // recently used, so it goes, then `b`.
        frame(&sleeves, screen, &[&c]);
        let freed = sleeves.evict();
        assert_eq!(freed.len(), 2);
        assert_eq!(freed[0].0, a, "the least recently used unshown sleeve");
        assert_eq!(freed[1].0, b, "then the next one, still unshown");
        assert_eq!(sleeves.resident(), (65_536, 1));
        assert!(matches!(sleeves.lookup(&c), Look::Ready(_)));
        assert!(matches!(sleeves.lookup(&a), Look::Unknown));
    }

    /// The rule: a sleeve a screen stopped showing is freed after `idle` — its
    /// atlas tile with it — and one another screen still shows is not.
    #[test]
    fn a_sleeve_nobody_shows_is_freed_once_it_has_left_the_window() {
        let screen = 1u64;
        let other = 2u64;
        let sleeves = Sleeves::with_rules(DECODED_BUDGET, Duration::ZERO);
        let (a, b) = (path("a"), path("b"));

        frame(&sleeves, screen, &[&a, &b]);
        frame(&sleeves, other, &[&b]);
        assert!(sleeves.land(&a, image(128)));
        assert!(sleeves.land(&b, image(128)));
        assert!(sleeves.evict().is_empty(), "both are still on a screen");

        // `a` left the window; the other screen keeps `b`.
        frame(&sleeves, screen, &[]);
        let freed = sleeves.evict();
        assert_eq!(freed.len(), 1);
        assert_eq!(freed[0].0, a);
        assert_eq!(sleeves.resident(), (65_536, 1));

        // The other screen goes too: `b`'s tile goes with it.
        let freed = sleeves.release(other);
        assert_eq!(freed.len(), 1);
        assert_eq!(freed[0].0, b);
        assert_eq!(sleeves.resident(), (0, 0));
    }

    /// A decode that lands after its row left the window is dropped, not kept:
    /// the cache only pays for what is on screen.
    #[test]
    fn a_decode_that_lands_after_its_row_left_is_dropped() {
        let screen = 1u64;
        let sleeves = Sleeves::with_rules(DECODED_BUDGET, IDLE_FOR);
        let (a, b) = (path("a"), path("b"));
        frame(&sleeves, screen, &[&a, &b]);
        let claimed = sleeves.claim(screen);
        assert_eq!(claimed.len(), 2);

        // `b`'s row scrolled away before its decode answered.
        frame(&sleeves, screen, &[&a]);
        assert!(sleeves.land(&a, image(128)));
        assert!(!sleeves.land(&b, image(128)));
        assert_eq!(sleeves.resident(), (65_536, 1));
        assert!(
            !sleeves.inner.borrow().decoding.contains(&b),
            "the in-flight mark is cleared, a later visit decodes it again"
        );
    }

    /// A file that will not decode is remembered as such: a corrupt cover must
    /// not be a decode loop every paint.
    #[test]
    fn a_file_that_does_not_decode_is_not_retried() {
        let screen = 1u64;
        let sleeves = Sleeves::with_rules(DECODED_BUDGET, IDLE_FOR);
        let a = path("broken");
        frame(&sleeves, screen, &[&a]);
        assert_eq!(sleeves.claim(screen), vec![a.clone()]);
        sleeves.fail(&a);
        assert_eq!(sleeves.claim(screen), Vec::<PathBuf>::new());
        assert!(matches!(sleeves.lookup(&a), Look::Failed));
    }

    /// What a row paints is the 128 px file, so the tile GPUI uploads is a
    /// row's tile: the arithmetic the class exists for.
    #[test]
    fn a_row_sleeve_is_a_128_px_file() {
        assert_eq!(Class::Row.px(), 128);
        assert_eq!(Class::Card.px(), 384);
        assert_eq!(Class::Page.px(), 512);
        // 1 000² × 4 = 4 000 000 B decoded against 128² × 4 = 65 536 B: the
        // factor is 61, and it is the same factor on the atlas tile.
        assert_eq!(1_000u64 * 1_000 * 4 / (128 * 128 * 4), 61);
    }
}
