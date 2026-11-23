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
//! The bytes come from the address the API gave: this server's own route
//! (`Api::bytes`, the session cookie goes with it) or the provider's CDN
//! (`Api::bytes_url`, no cookie — see [`source`]), always on the `User` lane: a
//! sleeve for what is on screen is part of the action just taken, not background
//! work. The cache write happens on the network thread, never on GPUI's.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use gpui_kit::{
    App, Asset as _, Global, ImageSource, ImgResourceLoader, IntoElement, ObjectFit, ParentElement,
    Pixels, RenderImage, Resource, SharedString, Styled, StyledImage as _, Window, img, px,
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

/// How long a refused sleeve waits before a paint may ask again.
///
/// A sleeve that failed (the provider's CDN blipped, the archive was not ready)
/// keeps its placeholder for this long, then one paint retries it — never once
/// per paint, which is a request storm (rule 3). A refused URL is also cleared
/// at once by [`Covers::retry_failed`], which is what a screen's « Réessayer »
/// calls.
pub const RETRY_AFTER: Duration = Duration::from_secs(20);

/// How many sleeves may WAIT for a slot. A screen asks in paint order (what it
/// shows first is asked first); the queue drains as slots free, so a page's
/// sleeves load without a re-render, and the oldest wait is dropped when the
/// queue is full — a screen re-asks every paint, so every URL reaches a slot in
/// turn instead of the first 64 freezing the rest out.
const MAX_WAITING: usize = 64;

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
    /// The address the API gave → the file the device has, at this screen's
    /// [`Class`].
    files: HashMap<String, PathBuf>,
    /// The size this screen draws its sleeves at.
    class: Class,
    /// Asked and refused, and when: a dead sleeve keeps its placeholder for
    /// [`RETRY_AFTER`] and is then retried once per window, never per paint.
    refused: HashMap<String, Instant>,
    inflight: HashSet<String>,
    /// Asked for while every slot was busy, oldest first.
    waiting: VecDeque<Waiting>,
    /// The retry window (`RETRY_AFTER`; a test asks for a shorter one).
    retry: Duration,
    /// This cache's own name in the process-wide decoded cache, so two caches
    /// never share one "what this screen shows" set.
    holder: u64,
}

/// One sleeve waiting for an in-flight slot: everything its request needs, and
/// the screen's callback, so it can start later (on the screen's own channel).
struct Waiting {
    url: String,
    px: u32,
    /// The link at the moment the paint asked: an offline ask reads the disk
    /// and nothing else (a sleeve already fetched still paints, a miss keeps
    /// its placeholder without a doomed request).
    online: bool,
    api: Arc<nz_api::Api>,
    cache: nz_cache::Cache,
    runtime: tokio::runtime::Handle,
    done: Box<dyn FnOnce(Option<PathBuf>) + Send + 'static>,
}

/// Where a sleeve's bytes come from.
///
/// The API's `cover` fields are TWO kinds of address, and both are in real
/// answers: this server's same-origin route (`/api/cover/<id>`, the session
/// cookie goes with it) and a provider CDN URL (`https://….dzcdn.net/…`, the
/// address the SPA puts in an `<img src>`). Treating the second as the first —
/// asking the server for `/api/cover/https://…` — is a 404 on every sleeve of
/// « Nouveautés », `/search` and `/moods`, which is why they stayed empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source<'a> {
    /// This server's own route, below `/api`.
    Api(&'a str),
    /// The provider's own address: no session cookie is sent.
    Provider(&'a str),
    /// Neither: nothing is asked, the placeholder is the answer.
    None,
}

/// What `url` is, without touching the network: one function so the plan is
/// testable and the request path never guesses.
pub fn source(url: &str) -> Source<'_> {
    if let Some(path) = url.strip_prefix("/api")
        && path.starts_with('/')
    {
        return Source::Api(path);
    }
    if url.starts_with("https://") || url.starts_with("http://") {
        return Source::Provider(url);
    }
    Source::None
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

    /// A screen whose sleeve IS the page: the now-playing stage draws one at up
    /// to 340 px, so it paints the 512 px copy (`Class::Page`) rather than the
    /// archived 1 000 px file — the same rule as a row, one size class up.
    pub fn pages() -> Self {
        Self::with_class(Class::Page)
    }

    fn with_class(class: Class) -> Self {
        Self {
            files: HashMap::new(),
            class,
            refused: HashMap::new(),
            inflight: HashSet::new(),
            waiting: VecDeque::new(),
            retry: RETRY_AFTER,
            holder: HOLDERS.fetch_add(1, Ordering::SeqCst),
        }
    }

    /// The same cache with another retry window: a test that must SEE the
    /// window expire does not wait twenty seconds.
    #[cfg(test)]
    fn with_retry(mut self, retry: Duration) -> Self {
        self.retry = retry;
        self
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

    /// Ask for `url` unless it is here, in flight, waiting, or was refused less
    /// than [`RETRY_AFTER`] ago. `done` runs on the screen's own event channel
    /// when the answer lands (file or refusal).
    ///
    /// **Visible first, bounded, and nothing lost.** The caller asks in paint
    /// order, so the sleeves the screen shows first take the first slots; the
    /// rest wait in that order and start as slots free, without a re-render.
    /// Nothing is asked when the screen is offline: the placeholder is the right
    /// answer, and a doomed request per row is what rule 3 forbids.
    pub fn want<F>(&mut self, state: &AppState, url: &str, done: F)
    where
        F: FnOnce(Option<PathBuf>) + Send + 'static,
    {
        if url.is_empty() || self.files.contains_key(url) || self.inflight.contains(url) {
            return;
        }
        if let Some(at) = self.refused.get(url)
            && at.elapsed() < self.retry
        {
            return;
        }
        if self.waiting.iter().any(|job| job.url == url) {
            return;
        }
        let (Some(api), Some(cache)) = (state.api.clone(), state.cache.clone()) else {
            return;
        };
        self.refused.remove(url);
        let job = Waiting {
            url: url.to_owned(),
            px: self.class.px(),
            online: state.online,
            api,
            cache,
            runtime: state.runtime.clone(),
            done: Box::new(done),
        };
        if self.inflight.len() < MAX_IN_FLIGHT {
            self.start(job);
        } else {
            if self.waiting.len() >= MAX_WAITING {
                self.waiting.pop_front();
            }
            self.waiting.push_back(job);
        }
    }

    /// Start one sleeve's request now, holding its slot until `resolved`.
    fn start(&mut self, job: Waiting) {
        self.inflight.insert(job.url.clone());
        let Waiting {
            url,
            px,
            online,
            api,
            cache,
            runtime,
            done,
        } = job;
        runtime.spawn(async move {
            let answer = fetch(&api, &cache, &url, px, online).await;
            done(answer);
        });
    }

    /// The answer to an earlier `want`, so the next paint draws the image.
    ///
    /// `None` marks the sleeve refused for [`RETRY_AFTER`]. A freed slot starts
    /// the first waiting sleeve, in the order the screen asked — that is what
    /// keeps a page's remaining sleeves loading without another paint.
    pub fn resolved(&mut self, url: &str, file: Option<PathBuf>) {
        self.inflight.remove(url);
        match file {
            Some(file) => {
                self.files.insert(url.to_owned(), file);
                self.refused.remove(url);
            }
            None => {
                self.refused.insert(url.to_owned(), Instant::now());
            }
        }
        while self.inflight.len() < MAX_IN_FLIGHT {
            let Some(job) = self.waiting.pop_front() else {
                break;
            };
            if self.files.contains_key(&job.url)
                || self.inflight.contains(&job.url)
                || self.refused.contains_key(&job.url)
            {
                continue;
            }
            self.start(job);
        }
    }

    /// How many sleeves are waiting for a slot (a test holds the queue to
    /// `MAX_IN_FLIGHT` requests and no more).
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }

    /// Forget what was asked, so a `Réessayer` asks again — a screen that came
    /// back online must not keep the placeholders a dead link left.
    pub fn retry_failed(&mut self) {
        self.refused.clear();
    }

    /// What this screen shows, declared by the paint that builds it: the files
    /// of the rows, cards or page header it is about to draw.
    ///
    /// A virtual list calls its closure more than once per paint — a measuring
    /// pass with ONE row, then the viewport — and those extra rows are really
    /// painted, so "what was painted" is not "what is on screen". The screen's
    /// own last range is: the declaration REPLACES this cache's previous one
    /// and is published by [`Covers::begin_frame`] on the next render, which is
    /// why the measuring pass cannot keep a sleeve alive.
    ///
    /// Costs nothing for a file already decoded; the first time it starts the
    /// background decode of the 128 px copy (a JPEG is ~0.2 ms on the executor,
    /// never on the paint thread).
    pub fn show(&mut self, shown: impl IntoIterator<Item = PathBuf>, cx: &mut App) {
        let holder = self.holder;
        cx.default_global::<Sleeves>()
            .show(holder, shown.into_iter().collect());
        self.decode(cx);
    }

    /// The frame is starting: publish what the last paint declared, decode what
    /// is new, and free the pixels and atlas tiles of every sleeve no screen
    /// shows any more.
    pub fn begin_frame(&mut self, window: &mut Window, cx: &mut App) {
        cx.default_global::<Sleeves>().publish(self.holder);
        self.decode(cx);
        for (path, image) in cx.default_global::<Sleeves>().evict() {
            // `Some(window)` because during a paint the window is taken OUT of
            // `App::windows`: without it the tile of the window being drawn is
            // the one left behind.
            cx.drop_image(image, Some(window));
            cx.remove_asset::<ImgResourceLoader>(&Resource::Path(path.into()));
        }
    }

    /// Start one background decode per file this cache declared and does not
    /// hold yet.
    fn decode(&mut self, cx: &mut App) {
        let holder = self.holder;
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

/// A sleeve's bytes, from the disk first and from wherever [`source`] points
/// second, then the file this screen draws (`Class`), never hidden by a
/// downscale that failed.
async fn fetch(
    api: &nz_api::Api,
    cache: &nz_cache::Cache,
    url: &str,
    px: u32,
    online: bool,
) -> Option<PathBuf> {
    // The file may already be there from an earlier visit (`nz-cache` keeps
    // sleeves): one SQLite read on the network thread, no request — and it is
    // the ONLY thing an offline ask does (rule 3: what was fetched once is
    // kept, and a screen with no link paints its covers).
    if let Some(original) = cache.covers().file(url) {
        return ready(cache, url, px, original);
    }
    if !online {
        return None;
    }
    let bytes = match source(url) {
        Source::Api(path) => api.bytes(Lane::User, path).await,
        Source::Provider(absolute) => api.bytes_url(Lane::User, absolute).await,
        Source::None => {
            tracing::debug!(url, "vignette : adresse ni locale ni du fournisseur");
            return None;
        }
    };
    let original = match bytes {
        Ok(bytes) => match cache.covers().put(url, &bytes) {
            Ok(file) => file,
            Err(error) => {
                tracing::debug!(%error, "la vignette n'a pas pu être écrite");
                return None;
            }
        },
        Err(error) => {
            // A failure to reach a provider is never a verdict: the row keeps
            // its placeholder and the window in `Covers::refused` decides the
            // retry, not this paint.
            tracing::debug!(%error, url, "vignette indisponible");
            return None;
        }
    };
    ready(cache, url, px, original)
}

/// The file `url` paints at this screen's size: the resized copy, or the
/// original when the resize refused it (`nz-cache` keeps sleeves, so the bytes
/// DID arrive — a wrong size is a right picture, a gap is not).
fn ready(cache: &nz_cache::Cache, url: &str, px: u32, original: PathBuf) -> Option<PathBuf> {
    cache.covers().variant(url, px).or(Some(original))
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

/// One decoded sleeve.
struct Painted {
    image: Arc<RenderImage>,
    /// What the decoded pixels hold: every frame's buffer, BGRA.
    bytes: usize,
    /// When a screen last declared it: what the idle sweep orders on.
    used: Instant,
}

#[derive(Default)]
struct Inner {
    painted: HashMap<PathBuf, Painted>,
    /// What each cache declared it shows, as of its last published frame. An
    /// image in one of these is never evicted.
    showing: HashMap<u64, HashSet<PathBuf>>,
    /// What the paint being built declared: the NEXT frame's `showing`. A
    /// virtual list overwrites it once per pass, so the viewport — declared
    /// last — is what survives.
    pending: HashMap<u64, HashSet<PathBuf>>,
    /// Decodes on the background executor, so one paint never asks twice.
    decoding: HashSet<PathBuf>,
    /// Files this process could not decode: never retried, the placeholder is
    /// the answer (a corrupt cover must not be a decode loop).
    broken: HashSet<PathBuf>,
    /// When the last sweep ran, so the LRU does not walk the map every frame.
    swept: Option<Instant>,
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

    /// What to paint for `path` now. A pure map read: the paint path never
    /// decodes, allocates or touches SQLite, and it does NOT decide what stays
    /// in memory — the screen's own declaration does (`Covers::show`), because
    /// a virtual list paints one measuring row the viewport never shows.
    pub fn lookup(&self, path: &Path) -> Look {
        let inner = self.inner.borrow();
        if let Some(item) = inner.painted.get(path) {
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

    /// Whether this cache still holds `path`'s decoded pixels. The question a
    /// screen's test asks about a row that left the window.
    pub fn holds(&self, path: &Path) -> bool {
        self.inner.borrow().painted.contains_key(path)
    }

    /// The decoded bytes held, and how many images (the number a test asserts).
    pub fn resident(&self) -> (usize, usize) {
        let inner = self.inner.borrow();
        (inner.bytes, inner.painted.len())
    }

    /// Declare what this cache shows, replacing the declaration of the paint
    /// being built.
    fn show(&self, holder: u64, shown: HashSet<PathBuf>) {
        self.inner.borrow_mut().pending.insert(holder, shown);
    }

    /// The frame boundary: this screen's declaration becomes what it shows, and
    /// every image in it is as fresh as now.
    fn publish(&self, holder: u64) {
        let mut inner = self.inner.borrow_mut();
        let Some(pending) = inner.pending.get(&holder).cloned() else {
            return;
        };
        inner.pending.insert(holder, HashSet::new());
        let now = Instant::now();
        for path in &pending {
            if let Some(item) = inner.painted.get_mut(path) {
                item.used = now;
            }
        }
        inner.showing.insert(holder, pending);
    }

    /// The files this cache declared — this frame's or the published one — that
    /// are not decoded yet: one decode each.
    fn claim(&self, holder: u64) -> Vec<PathBuf> {
        let mut inner = self.inner.borrow_mut();
        let mut shown: HashSet<PathBuf> = HashSet::new();
        if let Some(published) = inner.showing.get(&holder) {
            shown.extend(published.iter().cloned());
        }
        if let Some(pending) = inner.pending.get(&holder) {
            shown.extend(pending.iter().cloned());
        }
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
        if !inner
            .showing
            .values()
            .chain(inner.pending.values())
            .any(|shown| shown.contains(path))
        {
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

    /// The least recently declared images no screen shows, until the budget
    /// holds. Everything, when `holder`'s screen is going away.
    fn release(&self, holder: u64) -> Vec<(PathBuf, Arc<RenderImage>)> {
        let mut inner = self.inner.borrow_mut();
        inner.showing.remove(&holder);
        inner.pending.remove(&holder);
        // Its in-flight decodes are not this cache's to wait for: they land,
        // find no screen showing them, and are dropped by `land`.
        let mut freed = Vec::new();
        let idle: Vec<PathBuf> = inner
            .painted
            .keys()
            .filter(|path| {
                !inner
                    .showing
                    .values()
                    .chain(inner.pending.values())
                    .any(|shown| shown.contains(*path))
            })
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
        // Once per `idle`, or at once when the budget is over: walking a
        // thousand images on every frame of a 240 Hz scroll is work nobody
        // asked for, and the rule it enforces is timed in seconds anyway.
        if inner.bytes <= self.budget
            && inner
                .swept
                .is_some_and(|swept| now.saturating_duration_since(swept) < self.idle)
        {
            return freed;
        }
        inner.swept = Some(now);
        // A sleeve no screen has declared for `idle`: out. The screen's own
        // render is what keeps an image alive, so a row inside the window
        // refreshes it every frame and a row that left stops — the virtual
        // list's measuring pass (which paints one row) cannot keep a sleeve
        // alive, and neither can a transient frame that declares none.
        let idle: Vec<PathBuf> = inner
            .painted
            .iter()
            .filter(|(path, item)| {
                !inner
                    .showing
                    .values()
                    .chain(inner.pending.values())
                    .any(|shown| shown.contains(*path))
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
        // Then the budget, least recently declared first. What a screen shows
        // wins over the budget: it is a screenful, not the list.
        while inner.bytes > self.budget {
            let victim = inner
                .painted
                .iter()
                .filter(|(path, _)| {
                    !inner
                        .showing
                        .values()
                        .chain(inner.pending.values())
                        .any(|shown| shown.contains(*path))
                })
                .min_by_key(|(_, item)| item.used)
                .map(|(path, _)| path.clone());
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
        .map(|frame| {
            let size = image.size(frame);
            size.width.0.max(0) as usize * size.height.0.max(0) as usize * 4
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
    if let Some(file) = file {
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
            let mut picture = img(source).object_fit(ObjectFit::Cover).w(size).h(size);
            if round {
                picture = picture.rounded_full();
            }
            return tile.child(picture);
        }
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
    use std::sync::Mutex;

    use super::*;
    use crate::widgets::card::stub::{self, Stub};

    /// The answers a test's callbacks collect, keyed to their URL.
    type Answers = Arc<Mutex<Vec<(String, Option<PathBuf>)>>>;

    /// The login answer the stub gives (`/api/login` and `/api/me`).
    const ME: &str =
        r#"{"user":{"id":"u1","name":"bench","admin":false,"access":{},"sources":[]}}"#;

    /// A real 128 px JPEG, the bytes a sleeve route answers.
    fn sleeve_jpeg() -> Vec<u8> {
        let picture = image::RgbImage::from_fn(128, 128, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8])
        });
        let mut bytes = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 82);
        encoder.encode_image(&picture).expect("encode");
        bytes
    }

    /// What the app answers its screens: the two kinds of `cover` address in
    /// one place, so a provider URL can never be sent to the server's own route
    /// again (`/api/cover/https://…` is a 404 on every sleeve of Nouveautés).
    #[test]
    fn a_cover_address_is_read_as_the_route_or_the_provider_it_is() {
        assert_eq!(source("/api/cover/9"), Source::Api("/cover/9"));
        assert_eq!(
            source("https://e-cdns-images.dzcdn.net/images/cover/ab/1000x1000.jpg"),
            Source::Provider("https://e-cdns-images.dzcdn.net/images/cover/ab/1000x1000.jpg")
        );
        assert_eq!(
            source("http://127.0.0.1:8080/art.jpg"),
            Source::Provider("http://127.0.0.1:8080/art.jpg")
        );
        assert_eq!(source(""), Source::None);
        assert_eq!(source("/elsewhere.jpg"), Source::None, "no route to ask");
        assert_eq!(source("ftp://host/x.jpg"), Source::None, "never a file URL");
    }

    /// One pump of the network runtime and the UI, the way the screen tests do.
    fn pump(runtime: &tokio::runtime::Runtime, cx: &mut gpui::TestAppContext) {
        runtime.block_on(async {
            tokio::time::sleep(Duration::from_millis(2)).await;
        });
        cx.run_until_parked();
    }

    /// Hand every answer the callbacks collected to `Covers`, as the screen's
    /// own channel would.
    fn settle(covers: &mut Covers, answers: &Mutex<Vec<(String, Option<PathBuf>)>>) {
        for (url, file) in answers.lock().expect("answers").drain(..) {
            covers.resolved(&url, file);
        }
    }

    /// Ask one screen's cache for `url`, collecting the answer the way its
    /// event channel does.
    fn ask(
        cx: &mut gpui::TestAppContext,
        state: &gpui_kit::Entity<AppState>,
        covers: &mut Covers,
        answers: &Answers,
        url: &str,
    ) {
        let answer = Arc::clone(answers);
        let key = url.to_owned();
        cx.update(|cx| {
            let state = state.read(cx);
            covers.want(state, url, move |file| {
                answer.lock().expect("answers").push((key, file));
            });
        });
    }

    fn wait_for(
        runtime: &tokio::runtime::Runtime,
        cx: &mut gpui::TestAppContext,
        covers: &mut Covers,
        answers: &Mutex<Vec<(String, Option<PathBuf>)>>,
        what: &str,
        mut done: impl FnMut(&Covers) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            pump(runtime, cx);
            settle(covers, answers);
            if done(covers) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
        }
    }

    /// **The rule the person's empty shelves taught.** A `cover` field may be
    /// the provider's own CDN address: the request goes there, with the
    /// external client (no session cookie), and the bytes land in the device's
    /// cover store. Measured here on one page's four sleeves — one local, three
    /// provider addresses — **4 files** in the store (a 100 % display rate),
    /// one request each, and **0** requests to the server's own cover route;
    /// the old path resolved **1 of 4** (25 %) and spent three 404s on the
    /// `User` lane per provider address.
    #[gpui::test]
    async fn a_provider_sleeve_is_fetched_from_its_own_host_and_kept(
        cx: &mut gpui::TestAppContext,
    ) {
        let jpeg = sleeve_jpeg();
        let stub = Stub::start_bytes(&[
            ("/api/login", ME.as_bytes().to_vec()),
            ("/api/me", ME.as_bytes().to_vec()),
            ("/api/cover/9", jpeg.clone()),
            ("/art/7.jpg", jpeg.clone()),
            ("/art/8.jpg", jpeg.clone()),
            ("/art/9.jpg", jpeg.clone()),
        ]);
        let (state, runtime) = stub::logged_in(cx, &stub);
        let base = stub.base();
        let urls = [
            "/api/cover/9".to_owned(),
            format!("{base}/art/7.jpg"),
            format!("{base}/art/8.jpg"),
            format!("{base}/art/9.jpg"),
        ];
        let answers: Answers = Arc::new(Mutex::new(Vec::new()));
        let mut covers = Covers::new();
        for url in &urls {
            ask(cx, &state, &mut covers, &answers, url);
        }
        assert_eq!(covers.in_flight(), 4, "four sleeves are on their way");

        wait_for(
            &runtime,
            cx,
            &mut covers,
            &answers,
            "four sleeves",
            |covers| urls.iter().all(|url| covers.file(url).is_some()),
        );
        assert_eq!(stub.count("/api/cover/9"), 1, "the server's own route");
        for art in ["/art/7.jpg", "/art/8.jpg", "/art/9.jpg"] {
            assert_eq!(stub.count(art), 1, "the provider's address");
        }
        assert!(
            stub.hits()
                .iter()
                .all(|hit| !hit.path.starts_with("/api/cover/http")),
            "the provider URL was never sent to the server's cover route: {:?}",
            stub.hits()
                .iter()
                .map(|hit| hit.path.clone())
                .collect::<Vec<_>>()
        );
        for url in &urls {
            let file = covers.file(url).expect("a file");
            assert!(file.is_file(), "{url} → {}", file.display());
        }

        // What was fetched once is kept: a second visit makes NO request.
        let before = stub.total();
        let mut again = Covers::new();
        let answers: Answers = Arc::new(Mutex::new(Vec::new()));
        for url in &urls {
            ask(cx, &state, &mut again, &answers, url);
        }
        wait_for(&runtime, cx, &mut again, &answers, "four files", |again| {
            urls.iter().all(|url| again.file(url).is_some())
        });
        assert_eq!(stub.total(), before, "the disk was enough, no request");

        runtime.shutdown_background();
        stub.stop();
    }

    /// **Eight at a time, in the order asked, and nothing dropped.** A page
    /// asks for its sleeves in paint order (what it shows first first); when
    /// every slot is busy the rest WAIT and start as answers land, so a shelf
    /// of covers loads without another paint and the request count never fans
    /// out. Measured: 20 asked with no slot free → **8 in flight, 12 waiting**;
    /// after pumping, all 20 settled.
    #[gpui::test]
    async fn eight_covers_are_in_flight_and_the_rest_wait_in_the_order_asked(
        cx: &mut gpui::TestAppContext,
    ) {
        let jpeg = sleeve_jpeg();
        let mut routes: Vec<(&'static str, Vec<u8>)> = vec![
            ("/api/login", ME.as_bytes().to_vec()),
            ("/api/me", ME.as_bytes().to_vec()),
        ];
        for index in 0..20 {
            let path: &'static str = Box::leak(format!("/api/cover/c{index}").into_boxed_str());
            routes.push((path, jpeg.clone()));
        }
        let stub = Stub::start_bytes(&routes);
        let (state, runtime) = stub::logged_in(cx, &stub);
        let answers: Answers = Arc::new(Mutex::new(Vec::new()));
        let mut covers = Covers::new();
        for index in 0..20 {
            ask(
                cx,
                &state,
                &mut covers,
                &answers,
                &format!("/api/cover/c{index}"),
            );
        }
        assert_eq!(
            covers.in_flight(),
            MAX_IN_FLIGHT,
            "the whole fan-out is bounded"
        );
        assert_eq!(covers.waiting(), 12, "the rest wait, they are not dropped");

        wait_for(
            &runtime,
            cx,
            &mut covers,
            &answers,
            "twenty sleeves",
            |covers| covers.waiting() == 0 && covers.in_flight() == 0,
        );
        let asked: usize = (0..20)
            .map(|index| {
                assert!(
                    covers.file(&format!("/api/cover/c{index}")).is_some(),
                    "sleeve c{index} settled"
                );
                stub.count(&format!("/api/cover/c{index}"))
            })
            .sum();
        assert_eq!(asked, 20, "one request per sleeve, no fan-out");

        runtime.shutdown_background();
        stub.stop();
    }

    /// **The rule, offline still shows what it kept.** A sleeve already on the
    /// device is painted with the link down, and the disk read happens BEFORE
    /// the online check, so an offline screen makes **0** requests and still
    /// draws its covers (rule 3: what was fetched once is kept).
    #[gpui::test]
    async fn an_offline_screen_still_paints_the_sleeves_it_kept(cx: &mut gpui::TestAppContext) {
        let jpeg = sleeve_jpeg();
        let stub = Stub::start_bytes(&[
            ("/api/login", ME.as_bytes().to_vec()),
            ("/api/me", ME.as_bytes().to_vec()),
        ]);
        let (state, runtime) = stub::logged_in(cx, &stub);
        let url = "/api/cover/9";
        // What an earlier visit left on the device.
        cx.update(|cx| {
            let cache = state.read(cx).cache.clone().expect("a cache");
            cache.covers().put(url, &jpeg).expect("stored");
        });
        cx.update(|cx| state.update(cx, |state, _| state.online = false));

        let answers: Answers = Arc::new(Mutex::new(Vec::new()));
        let mut covers = Covers::new();
        ask(cx, &state, &mut covers, &answers, url);
        wait_for(
            &runtime,
            cx,
            &mut covers,
            &answers,
            "the kept sleeve",
            |covers| covers.in_flight() == 0,
        );
        let file = covers.file(url).expect("the kept sleeve paints");
        assert!(file.is_file(), "{}", file.display());
        assert_eq!(stub.count(url), 0, "offline: the disk, never the network");

        runtime.shutdown_background();
        stub.stop();
    }

    /// **A refusal is not a storm, and not a life sentence.** A sleeve that
    /// failed keeps its placeholder for `RETRY_AFTER`: repainting it asks
    /// nothing (measured: **1** request for two paints), and once the window has
    /// passed exactly one paint asks again (**2**). A test asks for a 200 ms
    /// window; the shipped one is 20 s.
    #[gpui::test]
    async fn a_refused_sleeve_waits_before_asking_again(cx: &mut gpui::TestAppContext) {
        let stub = Stub::start_bytes(&[
            ("/api/login", ME.as_bytes().to_vec()),
            ("/api/me", ME.as_bytes().to_vec()),
        ]);
        let (state, runtime) = stub::logged_in(cx, &stub);
        let answers: Answers = Arc::new(Mutex::new(Vec::new()));
        let mut covers = Covers::new().with_retry(Duration::from_millis(200));
        let url = "/api/cover/missing";
        ask(cx, &state, &mut covers, &answers, url);
        wait_for(
            &runtime,
            cx,
            &mut covers,
            &answers,
            "the refusal",
            |covers| covers.in_flight() == 0,
        );
        assert_eq!(stub.count(url), 1, "asked once");
        assert!(covers.file(url).is_none(), "the placeholder stays");

        // The next paint: no second request inside the window.
        ask(cx, &state, &mut covers, &answers, url);
        pump(&runtime, cx);
        assert_eq!(stub.count(url), 1, "no storm");

        std::thread::sleep(Duration::from_millis(260));
        ask(cx, &state, &mut covers, &answers, url);
        wait_for(&runtime, cx, &mut covers, &answers, "the retry", |covers| {
            covers.in_flight() == 0
        });
        assert_eq!(stub.count(url), 2, "one retry after the window");

        runtime.shutdown_background();
        stub.stop();
    }

    fn image(px: u32) -> Arc<RenderImage> {
        let frame = image::Frame::new(image::RgbaImage::new(px, px));
        Arc::new(RenderImage::new(vec![frame]))
    }

    fn path(name: &str) -> PathBuf {
        PathBuf::from(format!("/tmp/{name}.jpg"))
    }

    /// One render: the paint declares what it shows, then the frame after
    /// publishes it — exactly what `Covers::show` and `Covers::begin_frame` do.
    fn frame(sleeves: &Sleeves, holder: u64, shown: &[&PathBuf]) {
        sleeves.show(holder, shown.iter().map(|path| (*path).clone()).collect());
        sleeves.publish(holder);
    }

    /// The rule the person's 1 000 scrolls taught: however far a list is
    /// scrolled, the pixels held are ONE SCREENFUL — counted bytes, not RSS.
    /// A thousand declarations over four thousand rows, each frame sweeping,
    /// never keeps more than the viewport.
    #[test]
    fn a_thousand_renders_over_four_thousand_rows_keep_one_screenful() {
        let screen = 1u64;
        let sleeves = Sleeves::with_rules(DECODED_BUDGET, Duration::ZERO);
        let rows: Vec<PathBuf> = (0..4_000).map(|row| path(&format!("row{row}"))).collect();
        let mut most = 0;
        for start in 0..1_000usize {
            let visible: Vec<&PathBuf> = rows[start..start + 19].iter().collect();
            frame(&sleeves, screen, &visible);
            for row in &visible {
                sleeves.land(row, image(128));
            }
            sleeves.evict();
            let (bytes, images) = sleeves.resident();
            assert_eq!(bytes, images * 65_536, "a row's image is 128 × 128 × 4 B");
            most = most.max(images);
        }
        assert!(
            most <= 19 + 1,
            "a thousand frames over four thousand rows kept {most} images"
        );
    }

    /// The rule the measuring pass taught: a virtual list declares ONE row to
    /// measure its height, then the viewport it really paints. The viewport is
    /// what the frame published, so a sleeve only the measuring pass named is
    /// freed like any other row that left — it is NOT "on screen".
    #[test]
    fn a_measuring_pass_row_is_not_what_the_screen_shows() {
        let screen = 1u64;
        let sleeves = Sleeves::with_rules(DECODED_BUDGET, Duration::ZERO);
        let (measured, a, b) = (path("measured"), path("a"), path("b"));

        // One frame: the measuring pass, then the viewport.
        frame(&sleeves, screen, &[&measured]);
        assert!(sleeves.land(&measured, image(128)));
        sleeves.show(screen, HashSet::from([a.clone(), b.clone()]));
        assert!(sleeves.land(&a, image(128)));
        assert!(sleeves.land(&b, image(128)));
        sleeves.publish(screen);

        let freed = sleeves.evict();
        assert_eq!(freed.len(), 1);
        assert_eq!(
            freed[0].0, measured,
            "the row the measuring pass named, not the viewport's"
        );
        assert_eq!(sleeves.resident(), (2 * 65_536, 2));
        assert!(matches!(sleeves.lookup(&a), Look::Ready(_)));
        assert!(matches!(sleeves.lookup(&b), Look::Ready(_)));
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
        // 1_000² × 4 = 4 000 000 B decoded against 128² × 4 = 65 536 B: the
        // factor is 61, and it is the same factor on the atlas tile.
        assert_eq!(1_000u64 * 1_000 * 4 / (128 * 128 * 4), 61);
    }

    /// The three constructors name the three sizes a screen can need; a screen
    /// that picks the wrong one pays 4 MB where it needed 1 MB.
    #[test]
    fn each_constructor_names_its_size_class() {
        assert_eq!(Covers::rows().class(), Class::Row);
        assert_eq!(Covers::new().class(), Class::Card);
        assert_eq!(Covers::pages().class(), Class::Page);
    }
}
