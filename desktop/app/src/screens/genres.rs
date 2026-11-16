//! Le studio de genres : étiqueter, mesurer, entraîner — l'écran de l'administrateur.
//!
//! `routes/Genres.svelte` and its components are the reference, and the plan's
//! inventory names the same features: the genre studio (`/genre/*`), the genre
//! tree, the Explorer's parent styles (`/explorer*`) and client-side analysis
//! and training on this PC.
//!
//! Five tabs, the page's own order with the Explorer added (the SPA puts it on
//! the Search page; D16 owns `genre_explorer.rs` and no `search.rs`, so the
//! studio carries it — a placement difference, not a feature one):
//!
//! - **Étiquetage** — the confirming-not-typing flow: one candidate, the nine
//!   genres most likely for it on `1`…`9`, `Entrée` for the first, `z` to undo,
//!   `j`/`k` to walk the queue, `/` to search the vocabulary. One tag per track.
//! - **Bibliothèque** — what is out of date and at which step, where the
//!   measuring happens (the server or this PC), the two backfills with their
//!   progress and failure CAUSES, the nightly window, the Explorer's switches
//!   and the frozen extractor's card.
//! - **Arbre** — the genre DAG, with multi-parent links, create/rename/delete,
//!   cut, undo and reset.
//! - **Modèle** — the labelled set, the trainer (`webapp/trainer` linked
//!   natively) with its health verdict, the head sent to the server, the
//!   re-decision's progress, and the share file's export/import.
//! - **Explorer** — the parent styles this person listens to, their discovery at
//!   one of five breadths, and the stations.
//!
//! **It does not take the keyboard.** D8a's rule: the shell's `RootView` owns the
//! focus anchor, and the app-level list actions reach this screen through
//! `track_list::own`. The three keys the page adds (`1`…`9`, `z`) are bound at
//! startup by this file's own [`keys`] module and delivered through a weak
//! handle, exactly like the list keys — a screen that focused a handle of its
//! own would take every other binding with it when it is replaced.
//!
//! **Everything long is a job with a cause.** The two server backfills are
//! polled; the local measure runs on `nz-analysis` with the Bulk lane and parks
//! while a track is starting; the trainer runs on its own thread. Each one shows
//! its stage, its counters, and the counted reasons it failed (rule 2).

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement as _, Styled,
    Subscription, Window, div, px, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon, input};
use nz_api::{Api, ApiError, Lane, Method};
use nz_model::{
    AnalysisBackfill, AnalysisPlan, ExtractorInfo, ExtractorTest, GenreCandidates, GenreEmbed,
    GenreRelabel, GenreShareStatus, GenreStatus, GenreTree as TreeReply, Track, TrackList,
};
use serde::Deserialize;
use serde_json::Value;

use crate::nav::Route;
use crate::state::{AppState, entry_from_track};
use crate::theme;
use crate::toast::ToastKind;
use crate::widgets::card::{self, Phase};
use crate::widgets::cover::{self, Covers, Kind};
use crate::widgets::genre_bulk_sheet::{self, Scope, ScopeKind};
use crate::widgets::genre_explorer::{self, Explorer};
use crate::widgets::genre_extractor::{self, Card};
use crate::widgets::genre_label::{self, Tag};
use crate::widgets::genre_tag_sheet::{self, Sheet};
use crate::widgets::genre_tree::{self, Tree};
use crate::widgets::track_list;
use crate::wiring::analysis::training::{self, Prepared};
use crate::wiring::analysis::{self, Control, Runner};
use crate::wiring::events::{self, Sender};

/// The tabs, in the order the page lists them (Explorer appended, see above).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Tagging,
    Library,
    Tree,
    Model,
    Explorer,
}

impl Tab {
    const ALL: [Tab; 5] = [
        Tab::Tagging,
        Tab::Library,
        Tab::Tree,
        Tab::Model,
        Tab::Explorer,
    ];

    fn label(self) -> &'static str {
        match self {
            Tab::Tagging => "Étiquetage",
            Tab::Library => "Bibliothèque",
            Tab::Tree => "Arbre",
            Tab::Model => "Modèle",
            Tab::Explorer => "Explorer",
        }
    }

    fn glyph(self) -> IconName {
        match self {
            Tab::Tagging => IconName::Tag,
            Tab::Library => IconName::Database,
            Tab::Tree => IconName::GitBranch,
            Tab::Model => IconName::Activity,
            Tab::Explorer => IconName::Compass,
        }
    }
}

/// Which order the candidates come in: the page's two, neither wrong.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Sort {
    #[default]
    Plays,
    Active,
}

impl Sort {
    fn wire(self) -> Option<&'static str> {
        match self {
            Sort::Plays => None,
            Sort::Active => Some("active"),
        }
    }
}

/// One candidate row, as `/genre/candidates` sends it (`_track_json` + scores).
#[derive(Clone, Debug, Default, Deserialize)]
struct Candidate {
    #[serde(default)]
    id: String,
    /// A Deezer number, or the row's UUID for an upload; `Value` because the
    /// server sends a number here and a string there.
    #[serde(default)]
    deezer_id: Value,
    #[serde(default)]
    title: String,
    #[serde(default)]
    artist: String,
    #[serde(default)]
    album: String,
    #[serde(default)]
    duration: u32,
    #[serde(default)]
    play_count: u32,
    #[serde(default)]
    deezer_bpm: Option<f64>,
    #[serde(default)]
    predicted: Option<String>,
    #[serde(default)]
    confidence: Option<f64>,
    #[serde(default)]
    uncertainty: Option<f64>,
    #[serde(default)]
    scores: Vec<Value>,
}

impl Candidate {
    /// The ref every route takes: the universal `deezer_id` when it exists (a
    /// number or a UUID), else the row's UUID.
    fn reference(&self) -> String {
        match &self.deezer_id {
            Value::String(text) if !text.is_empty() => text.clone(),
            Value::Number(number) => number.to_string(),
            _ => self.id.clone(),
        }
    }

    fn cover_path(&self) -> String {
        format!("/api/cover/{}", self.reference())
    }

    fn scores(&self) -> Vec<genre_label::ScoreRow> {
        genre_tag_sheet::read_scores(Some(&Value::Array(self.scores.clone())))
    }
}

/// The last label, for one-key undo: a slip of the finger must cost one key.
#[derive(Clone, Debug)]
struct Last {
    track: String,
    title: String,
    applied: String,
    previous: Option<String>,
    /// The candidate row, so `z` puts it back where it was: a slip of the
    /// finger costs one key, not a lost track.
    candidate: Option<Candidate>,
}

/// The client-side analysis run, as the card reads it.
#[derive(Default)]
struct Run {
    control: Option<Arc<Control>>,
    runner: Option<Runner>,
    running: bool,
    done: u32,
    failed: u32,
    total: u32,
    current: String,
    causes: BTreeMap<String, u32>,
    stopped: Option<String>,
}

/// The training run, in the Model tab.
#[derive(Clone, Debug, Default)]
struct Training {
    mode: training::Mode,
    running: bool,
    stage: String,
    fraction: f64,
    head: Option<training::Outcome>,
    error: Option<SharedString>,
    /// The health verdict refused to send on its own (`poor`).
    sending: bool,
    sent: bool,
    relabel: Option<GenreRelabel>,
    self_train: bool,
}

/// The share card's own state (`/genre/share/*`).
#[derive(Clone, Debug, Default)]
struct Share {
    path: String,
    exporting: bool,
    importing: bool,
    preview: Option<Value>,
    head: bool,
    tags: bool,
    job: Option<GenreShareStatus>,
    error: Option<SharedString>,
    note: Option<SharedString>,
}

/// The bulk form: which scope, and which reference it names.
#[derive(Clone, Debug)]
struct BulkForm {
    kind: ScopeKind,
    reference: String,
    error: Option<SharedString>,
    busy: bool,
}

impl Default for BulkForm {
    fn default() -> Self {
        Self {
            kind: ScopeKind::Album,
            reference: String::new(),
            error: None,
            busy: false,
        }
    }
}

/// The matrix and the self-training pool a run starts with.
type PreparedTraining = (Prepared, Option<Vec<f32>>);

/// What a background answer brings back.
enum Msg {
    Status(Box<Result<GenreStatus, ApiError>>),
    Candidates(Box<Result<GenreCandidates, ApiError>>),
    Labelled(Box<Result<Value, ApiError>>),
    Label(Box<Result<Value, ApiError>>, Box<Last>, bool),
    Sheet(Box<Result<Value, ApiError>>),
    SheetCleared(Box<Result<Value, ApiError>>),
    BulkPreview(Box<Result<Value, ApiError>>),
    BulkApply(Box<Result<Value, ApiError>>),
    Tree(Box<Result<TreeReply, ApiError>>),
    TreeWritten(Box<Result<Value, ApiError>>),
    TagCreated(Box<Result<Value, ApiError>>, String, Option<String>),
    Plan(Box<Result<AnalysisPlan, ApiError>>),
    Backfill(Box<Result<AnalysisBackfill, ApiError>>),
    EmbedStatus(Box<Result<GenreEmbed, ApiError>>),
    ExplorerAdmin(Box<Result<nz_model::Explorer, ApiError>>),
    Where(Box<Result<(), ApiError>>),
    Analysis(analysis::Event),
    TrainStage(&'static str, f64),
    TrainPrepared(Box<Result<PreparedTraining, String>>),
    TrainEvent(training::Event),
    ModelPut(Box<Result<Value, ApiError>>),
    ModelDeleted(Box<Result<(), ApiError>>),
    Relabel(Box<Result<GenreRelabel, ApiError>>),
    ExtractorTest(Box<Result<ExtractorTest, ApiError>>),
    ExtractorUpload(Box<Result<Value, String>>),
    ExtractorDeleted(Box<Result<(), ApiError>>),
    ShareExported(Box<Result<Value, String>>),
    SharePreview(Box<Result<Value, ApiError>>),
    ShareImported(Box<Result<Value, ApiError>>),
    ShareJob(Box<Result<GenreShareStatus, ApiError>>),
    Album(Box<Result<nz_model::AlbumPage, ApiError>>),
    Playlist(Box<Result<nz_model::PlaylistPage, ApiError>>),
    Styles(Box<Result<Value, ApiError>>),
    Discover(Box<Result<Value, ApiError>>),
    Station(Box<Result<TrackList, ApiError>>),
    Cover(String, Option<PathBuf>),
}

/// The text fields one visit keeps (a draft survives a repaint and a tab
/// switch: `Window::use_keyed_state` is per window).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Field {
    Query,
    TreeDraft,
    SharePath,
    ExtractorPath,
    BulkRef,
}

pub struct Screen {
    route: Route,
    state: Entity<AppState>,
    tab: Tab,
    status: Phase<GenreStatus>,
    tags: Vec<Tag>,
    counts: BTreeMap<String, u32>,
    labelled: u32,
    extractor: ExtractorInfo,
    model: Option<Value>,
    candidates: Phase<Vec<Candidate>>,
    /// The candidate page was asked at least once (a cached status must still
    /// load it: `first` is false when the device painted the vocabulary).
    candidates_asked: bool,
    cursor: usize,
    sort: Sort,
    last: Option<Last>,
    query: String,
    show_all: bool,
    error: Option<SharedString>,
    note: Option<SharedString>,
    covers: Rc<RefCell<Covers>>,
    // -- Bibliothèque
    plan: Phase<AnalysisPlan>,
    backfill: Phase<AnalysisBackfill>,
    embed: Phase<GenreEmbed>,
    explorer_admin: Phase<nz_model::Explorer>,
    library_loaded: bool,
    where_choice: String,
    force: bool,
    workers: u32,
    run: Run,
    // -- Arbre
    tree: Tree,
    tree_loaded: bool,
    /// The parent a create editor will attach the new genre under (`None`: a
    /// root).
    tree_parent: Option<String>,
    // -- Modèle
    training: Training,
    share: Share,
    // -- Explorer
    explorer: Explorer,
    explorer_loaded: bool,
    // -- sheets and forms
    sheet: Sheet,
    bulk: genre_bulk_sheet::Sheet,
    bulk_form: Option<BulkForm>,
    /// The bulk form is behind a button: the tagging tab is one candidate at a
    /// time, and a permanent form under it is clutter on a laptop.
    bulk_open: bool,
    extractor_card: Card,
    /// The genre the bulk form applies, once one is picked.
    bulk_tag: Option<Tag>,
    // -- plumbing
    fields: HashMap<Field, Entity<InputState>>,
    events: Sender<Msg>,
    _subscriptions: Vec<Subscription>,
}

impl Screen {
    pub fn new(route: Route, state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let (tx, rx) = events::channel::<Msg>();
        let is_admin = state.read(cx).is_admin();
        let mut screen = Self {
            route,
            state: state.clone(),
            tab: Tab::Tagging,
            status: Phase::Loading,
            tags: Vec::new(),
            counts: BTreeMap::new(),
            labelled: 0,
            extractor: ExtractorInfo::default(),
            model: None,
            candidates: Phase::Loading,
            candidates_asked: false,
            cursor: 0,
            sort: Sort::Plays,
            last: None,
            query: String::new(),
            show_all: false,
            error: None,
            note: None,
            covers: Rc::new(RefCell::new(Covers::new())),
            plan: Phase::Loading,
            backfill: Phase::Loading,
            embed: Phase::Loading,
            explorer_admin: Phase::Loading,
            library_loaded: false,
            where_choice: "server".to_owned(),
            force: false,
            workers: 0,
            run: Run::default(),
            tree: Tree::default(),
            tree_loaded: false,
            tree_parent: None,
            training: Training {
                self_train: true,
                ..Training::default()
            },
            share: Share::default(),
            explorer: Explorer::new(),
            explorer_loaded: false,
            sheet: Sheet::default(),
            bulk: genre_bulk_sheet::Sheet::default(),
            bulk_form: None,
            bulk_open: false,
            extractor_card: Card::default(),
            bulk_tag: None,
            fields: HashMap::new(),
            events: tx,
            _subscriptions: vec![cx.observe(&state, |this, state, cx| {
                // Rule 1: while a track is starting, the local analysis parks.
                let playing = {
                    let state = state.read(cx);
                    state.player.playing() && state.player.current_time() < 8.0
                };
                if let Some(control) = &this.run.control {
                    control.set_yield(playing);
                }
                cx.notify();
            })],
        };
        // The device's copy paints first (the SPA's `swr` first half), then one
        // request corrects it: a revisit shows the vocabulary at once.
        if let Some(cached) = card::cached::<GenreStatus>(state.read(cx), "/genre/status") {
            screen.apply_status(cached);
        }
        screen.drain(rx, cx);
        screen.load_status(cx);
        if is_admin && !matches!(screen.status, Phase::Ready(_)) {
            // The status request will trigger the candidates; nothing else is
            // asked before the page knows its vocabulary.
        }
        keys::install(cx);
        keys::own(cx, |this: &mut Self, key, window, cx| {
            this.on_key(key, window, cx);
        });
        track_list::own(cx, |this: &mut Self, intent, _window, cx| {
            this.on_list_intent(intent, cx);
        });
        screen
    }

    pub fn route(&self) -> &Route {
        &self.route
    }

    // -- plumbing ---------------------------------------------------------

    fn drain(&mut self, rx: events::Receiver<Msg>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            while let Some(msg) = rx.next().await {
                if this.update(cx, |this, cx| this.on_msg(msg, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn api(&self, cx: &Context<Self>) -> Option<Arc<Api>> {
        self.state.read(cx).api.clone()
    }

    fn runtime(&self, cx: &Context<Self>) -> tokio::runtime::Handle {
        self.state.read(cx).runtime.clone()
    }

    fn toast(&mut self, kind: ToastKind, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.state
            .update(cx, |state, _| state.toasts.push(kind, message.into()));
    }

    fn is_admin(&self, cx: &Context<Self>) -> bool {
        self.state.read(cx).is_admin()
    }
}

impl Render for Screen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Centred and bound: on a 2560 px panel a full-bleed form would put a
        // three-word label a metre from its value. The page keeps a readable
        // measure and the extra width stays margin (the shell's rails already
        // own the edges).
        let page = div()
            .id("genre-studio")
            .key_context("Neezer")
            .relative()
            .flex()
            .flex_col()
            .w_full()
            .max_w(rems(100.))
            .mx_auto()
            .min_h_0()
            .gap(rems(1.))
            .p(rems(1.25))
            .bg(theme::shell(cx))
            .text_color(theme::text(cx))
            .font_family(theme::font(cx))
            .child(self.header(cx))
            .child(self.tabs(cx));
        let body = match self.tab {
            Tab::Tagging => self.tagging(window, cx).into_any_element(),
            Tab::Library => self.library(window, cx).into_any_element(),
            Tab::Tree => self.tree_tab(window, cx).into_any_element(),
            Tab::Model => self.model_tab(window, cx).into_any_element(),
            Tab::Explorer => self.explorer_tab(window, cx).into_any_element(),
        };
        let page = page.child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .child(body),
        );
        let column = div()
            .id("genre-scroll")
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .child(page);
        let mut root = div().relative().size_full().child(column);
        if self.sheet.open {
            root = root.child(genre_tag_sheet::view(
                &self.sheet,
                &self.tags,
                &self.counts,
                cx,
                |this: &mut Self, action, cx| this.on_sheet_action(action, cx),
            ));
        }
        if self.bulk.open {
            root = root.child(genre_bulk_sheet::view(
                &self.bulk,
                cx,
                |this: &mut Self, action, cx| this.on_bulk_action(action, cx),
            ));
        }
        root
    }
}

// -- the keys the page adds ---------------------------------------------------
//
// `1`…`9` and `z` are the page's three gestures (`1`…`9` pick a genre, `z`
// undoes). They are bound once, globally, with the shell's own context, and
// delivered to the LIVE screen through a weak handle — the mechanism
// `track_list::own` uses, and for the same reason: an app-level listener runs
// outside the window's borrow, so re-entering it inside a dispatch fails
// silently (D8a measured it).
mod keys {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui_kit::{App, BorrowAppContext as _, Context, KeyBinding, Render, Window};

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum Key {
        Pick(u8),
        Undo,
    }

    gpui_kit::actions!(neezer_genre, [GenreUndo]);

    #[derive(Clone, Copy, Debug, PartialEq, gpui_kit::Action)]
    #[action(namespace = neezer_genre, no_json)]
    pub struct GenrePick(pub u8);

    type Handler = Rc<dyn Fn(Key, &mut Window, &mut App)>;

    pub struct Owner(pub RefCell<Option<Handler>>);

    impl gpui_kit::Global for Owner {}

    pub fn install<V: Render>(cx: &mut Context<V>) {
        if cx.has_global::<Owner>() {
            return;
        }
        cx.set_global(Owner(RefCell::new(None)));
        cx.bind_keys(bindings());
        App::on_action(&mut *cx, |action: &GenrePick, cx: &mut App| {
            deliver(Key::Pick(action.0), cx)
        });
        App::on_action(&mut *cx, |_: &GenreUndo, cx: &mut App| {
            deliver(Key::Undo, cx)
        });
    }

    fn bindings() -> Vec<KeyBinding> {
        // GLOBAL on purpose: `1`…`9` and `z` are unbound everywhere else, and a
        // context-scoped binding is only found along the FOCUSED node's path
        // (D0's pitfall: with nothing focused the lookup starts above the view,
        // so a screen that does not own the anchor would never see its own
        // keys). The handler is the live screen's weak handle, so the keys do
        // nothing once this screen is gone.
        let context = None;
        let mut bindings = Vec::new();
        for number in 1..=9u8 {
            let stroke = number.to_string();
            bindings.push(KeyBinding::new(stroke.as_str(), GenrePick(number), context));
        }
        bindings.push(KeyBinding::new("z", GenreUndo, context));
        bindings
    }

    pub fn own<V: Render>(
        cx: &mut Context<V>,
        on: impl Fn(&mut V, Key, &mut Window, &mut Context<V>) + 'static,
    ) {
        let weak = cx.weak_entity();
        let handler: Handler = Rc::new(move |key, window, cx| {
            let _ = weak.update(cx, |this, cx| on(this, key, window, cx));
        });
        cx.update_global::<Owner, _>(|owner, _| {
            *owner.0.borrow_mut() = Some(handler);
        });
    }

    fn deliver(key: Key, cx: &mut App) {
        cx.defer(move |cx| {
            let handler = cx
                .try_global::<Owner>()
                .and_then(|owner| owner.0.borrow().clone());
            let Some(handler) = handler else {
                return;
            };
            let Some(window) = cx
                .active_window()
                .or_else(|| cx.windows().into_iter().next())
            else {
                return;
            };
            let _ = window.update(cx, |_, window, cx| handler(key, window, cx));
        });
    }
}

impl Screen {
    fn on_key(&mut self, key: keys::Key, _window: &mut Window, cx: &mut Context<Self>) {
        match key {
            keys::Key::Pick(number) => self.pick(number as usize, cx),
            keys::Key::Undo => self.undo(cx),
        }
    }

    fn on_list_intent(&mut self, intent: track_list::Intent, cx: &mut Context<Self>) {
        match self.tab {
            Tab::Tagging => match intent {
                track_list::Intent::Move(delta) => {
                    if delta > 0 {
                        self.skip(cx);
                    } else {
                        self.back(cx);
                    }
                }
                track_list::Intent::PlayFrom(_) => self.confirm(cx),
                track_list::Intent::First => {
                    self.cursor = 0;
                    cx.notify();
                }
                track_list::Intent::Last => {
                    if let Phase::Ready(list) = &self.candidates {
                        self.cursor = list.len().saturating_sub(1);
                    }
                    cx.notify();
                }
                track_list::Intent::Page(delta) => {
                    self.cursor = self.cursor.saturating_add_signed(delta as isize * 5);
                    self.clamp_cursor();
                    cx.notify();
                }
                _ => {}
            },
            Tab::Tree => match intent {
                track_list::Intent::Move(delta) => {
                    self.tree.move_by(delta);
                    cx.notify();
                }
                track_list::Intent::First => {
                    self.tree.select(0);
                    cx.notify();
                }
                track_list::Intent::Last => {
                    self.tree.select(usize::MAX);
                    cx.notify();
                }
                track_list::Intent::Page(delta) => {
                    self.tree.move_by(delta * 10);
                    cx.notify();
                }
                track_list::Intent::PlayFrom(_) => {
                    self.tree.toggle_selected();
                    cx.notify();
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn clamp_cursor(&mut self) {
        if let Phase::Ready(list) = &self.candidates {
            self.cursor = self.cursor.min(list.len().saturating_sub(1));
        } else {
            self.cursor = 0;
        }
    }

    fn current(&self) -> Option<&Candidate> {
        match &self.candidates {
            Phase::Ready(list) => list.get(self.cursor),
            _ => None,
        }
    }

    // -- loading ----------------------------------------------------------

    fn load_status(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            self.status = Phase::Failed(ApiError::Offline);
            return;
        };
        let tx = self.events.clone();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            tx.send(Msg::Status(Box::new(api.genre_status().await)));
        });
    }

    fn load_candidates(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        self.candidates = Phase::Loading;
        self.candidates_asked = true;
        self.cursor = 0;
        let tx = self.events.clone();
        let runtime = self.runtime(cx);
        let sort = self.sort;
        let last = self.tags.first().map(|tag| tag.name.clone());
        runtime.spawn(async move {
            let reply = api.genre_candidates(40, sort.wire(), last.as_deref()).await;
            tx.send(Msg::Candidates(Box::new(reply)));
        });
    }

    fn apply_status(&mut self, status: GenreStatus) {
        self.tags = status.tags.iter().map(Tag::from_node).collect();
        self.tags.sort_by(|a, b| a.name.cmp(&b.name));
        self.counts = status.counts.clone();
        self.labelled = status.labelled;
        self.extractor = status.extractor.clone();
        self.model = status.model.clone();
        self.status = Phase::Ready(status);
    }

    fn load_tree(&mut self, cx: &mut Context<Self>) {
        if self.tree_loaded {
            return;
        }
        let Some(api) = self.api(cx) else {
            return;
        };
        self.tree_loaded = true;
        let tx = self.events.clone();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            tx.send(Msg::Tree(Box::new(api.genre_tree().await)));
        });
    }

    fn load_library(&mut self, cx: &mut Context<Self>) {
        if self.library_loaded {
            return;
        }
        let Some(api) = self.api(cx) else {
            return;
        };
        self.library_loaded = true;
        let tx = self.events.clone();
        let runtime = self.runtime(cx);
        {
            let api = Arc::clone(&api);
            let tx = tx.clone();
            runtime.spawn(async move {
                let reply = api.analysis_plan().await;
                tx.send(Msg::Plan(Box::new(reply)));
            });
        }
        {
            let api = Arc::clone(&api);
            let tx = tx.clone();
            runtime.spawn(async move {
                let reply = api.analysis_backfill_status().await;
                tx.send(Msg::Backfill(Box::new(reply)));
            });
        }
        {
            let api = Arc::clone(&api);
            let tx = tx.clone();
            runtime.spawn(async move {
                let reply = api.genre_embed_status().await;
                tx.send(Msg::EmbedStatus(Box::new(reply)));
            });
        }
        {
            let api = Arc::clone(&api);
            let tx = tx.clone();
            runtime.spawn(async move {
                let reply = api.explorer(None).await;
                tx.send(Msg::ExplorerAdmin(Box::new(reply)));
            });
        }
    }

    fn load_explorer(&mut self, cx: &mut Context<Self>) {
        if self.explorer_loaded {
            return;
        }
        let Some(api) = self.api(cx) else {
            return;
        };
        self.explorer_loaded = true;
        self.explorer.loading = true;
        let tx = self.events.clone();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            let reply = api.get::<Value>(Lane::User, "/explorer/styles").await;
            tx.send(Msg::Styles(Box::new(reply.map(|fresh| fresh.value))));
        });
    }

    // -- the tagging flow -------------------------------------------------

    fn pick(&mut self, number: usize, cx: &mut Context<Self>) {
        if self.tab != Tab::Tagging || number == 0 {
            return;
        }
        let Some(candidate) = self.current().cloned() else {
            return;
        };
        let ranked = genre_label::rank(&self.tags, &candidate.scores(), &self.counts);
        let Some(row) = ranked.get(number - 1).cloned() else {
            return;
        };
        self.label(candidate, row.tag, cx);
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        if self.tab != Tab::Tagging {
            return;
        }
        let Some(candidate) = self.current().cloned() else {
            return;
        };
        let ranked = genre_label::rank(&self.tags, &candidate.scores(), &self.counts);
        let Some(row) = ranked.first().cloned() else {
            return;
        };
        self.label(candidate, row.tag, cx);
    }

    fn label(&mut self, candidate: Candidate, tag: Tag, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        let last = Last {
            track: candidate.reference(),
            title: candidate.title.clone(),
            applied: tag.name.clone(),
            // The candidate already wears nothing (the endpoint lists untagged
            // rows), so undo clears rather than restores. A sheet opened on a
            // tagged track carries its current tag and restores it.
            previous: self
                .sheet
                .current
                .as_ref()
                .filter(|_| self.sheet.track == candidate.id)
                .map(|tag| tag.name.clone()),
            candidate: Some(candidate.clone()),
        };
        self.last = Some(last.clone());
        // Optimistic: the row is gone from the queue at once (rule 4), and the
        // counters move with it. A refusal rolls the queue back.
        *self.counts.entry(tag.name.clone()).or_insert(0) += 1;
        self.labelled += 1;
        self.note = Some(format!("{} · {}", candidate.title, tag.name).into());
        self.advance(cx);
        let tx = self.events.clone();
        let runtime = self.runtime(cx);
        let track = candidate.reference();
        let wire = tag.id.to_string();
        runtime.spawn(async move {
            let reply = api.genre_label(&track, &wire).await;
            tx.send(Msg::Label(Box::new(reply), Box::new(last), false));
        });
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        let Some(last) = self.last.clone() else {
            return;
        };
        let Some(api) = self.api(cx) else {
            return;
        };
        // Optimistic: the genre comes back off the counter.
        if let Some(count) = self.counts.get_mut(&last.applied) {
            *count = count.saturating_sub(1);
        }
        self.labelled = self.labelled.saturating_sub(1);
        self.last = None;
        if let Some(candidate) = last.candidate.clone()
            && let Phase::Ready(list) = &mut self.candidates
            && !list.iter().any(|row| row.id == candidate.id)
        {
            let at = self.cursor.min(list.len());
            list.insert(at, candidate);
            self.cursor = at;
        }
        self.note = Some(format!("Annulé : {} · {}", last.title, last.applied).into());
        let tx = self.events.clone();
        let runtime = self.runtime(cx);
        let track = last.track.clone();
        let previous = last.previous.clone();
        runtime.spawn(async move {
            let reply = match &previous {
                Some(name) => api.genre_label(&track, name).await,
                None => api.genre_label(&track, "").await,
            };
            tx.send(Msg::Label(Box::new(reply), Box::new(last), true));
        });
    }

    fn advance(&mut self, cx: &mut Context<Self>) {
        if let Phase::Ready(list) = &mut self.candidates {
            if !list.is_empty() {
                list.remove(self.cursor.min(list.len() - 1));
            }
            if list.is_empty() {
                self.cursor = 0;
            } else {
                self.cursor = self.cursor.min(list.len() - 1);
            }
        }
        if matches!(&self.candidates, Phase::Ready(list) if list.is_empty()) {
            // The queue ran out: ask for more at once, so the studio never sits
            // on an empty page a person has to reload by hand (rule 2).
            self.load_candidates(cx);
        }
    }

    fn skip(&mut self, cx: &mut Context<Self>) {
        if let Phase::Ready(list) = &self.candidates
            && self.cursor + 1 >= list.len()
        {
            self.load_candidates(cx);
            return;
        }
        self.cursor += 1;
        self.clamp_cursor();
        cx.notify();
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        self.cursor = self.cursor.saturating_sub(1);
        cx.notify();
    }

    fn open_sheet(&mut self, candidate: &Candidate, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        self.sheet = Sheet::open(candidate.id.clone(), candidate.title.clone());
        let tx = self.events.clone();
        let runtime = self.runtime(cx);
        let path = self.sheet.path();
        runtime.spawn(async move {
            let reply = api.get::<Value>(Lane::User, &path).await;
            tx.send(Msg::Sheet(Box::new(reply.map(|fresh| fresh.value))));
        });
    }

    fn on_sheet_action(&mut self, action: genre_tag_sheet::Action, cx: &mut Context<Self>) {
        match action {
            genre_tag_sheet::Action::Close => {
                self.sheet.close();
                cx.notify();
            }
            genre_tag_sheet::Action::Pick(tag) => {
                let Some(api) = self.api(cx) else {
                    return;
                };
                let track = self.sheet.track.clone();
                let previous = self.sheet.current.as_ref().map(|tag| tag.name.clone());
                self.sheet.picked(&tag);
                let last = Last {
                    track: track.clone(),
                    title: self.sheet.title.clone(),
                    applied: tag.name.clone(),
                    previous,
                    candidate: None,
                };
                self.last = Some(last.clone());
                let tx = self.events.clone();
                let runtime = self.runtime(cx);
                runtime.spawn(async move {
                    let reply = api.genre_label(&track, &tag.id.to_string()).await;
                    tx.send(Msg::Label(Box::new(reply), Box::new(last), false));
                });
                cx.notify();
            }
            genre_tag_sheet::Action::Clear => {
                let Some(api) = self.api(cx) else {
                    return;
                };
                let track = self.sheet.track.clone();
                self.sheet.saving = true;
                self.sheet.current = None;
                let tx = self.events.clone();
                let runtime = self.runtime(cx);
                runtime.spawn(async move {
                    let reply = api.genre_label(&track, "").await;
                    tx.send(Msg::SheetCleared(Box::new(reply)));
                });
            }
        }
    }

    // -- the bulk sheet ---------------------------------------------------

    fn open_bulk(&mut self, form: BulkForm, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        let reference = form.reference.trim().to_owned();
        if reference.is_empty() {
            self.bulk_form = Some(BulkForm {
                error: Some(
                    "Indiquez la référence de l'album, de la playlist ou de l'artiste.".into(),
                ),
                ..form
            });
            return;
        }
        let tx = self.events.clone();
        let runtime = self.runtime(cx);
        let kind = form.kind;
        match kind {
            ScopeKind::Artist => {
                // An artist scope is resolved by the SERVER (every library row
                // of that artist): the preview names the artist, not a list.
                self.preview_bulk(
                    Scope {
                        kind: ScopeKind::Artist,
                        title: reference.clone(),
                        tracks: Vec::new(),
                        artist: Some(reference.clone()),
                    },
                    cx,
                );
                return;
            }
            ScopeKind::Album => {
                let api = Arc::clone(&api);
                let tx = tx.clone();
                runtime.spawn(async move {
                    tx.send(Msg::Album(Box::new(api.album(&reference).await)));
                });
            }
            ScopeKind::Playlist => {
                runtime.spawn(async move {
                    tx.send(Msg::Playlist(Box::new(api.playlist(&reference).await)));
                });
            }
        }
        self.bulk_form = Some(BulkForm {
            error: None,
            busy: true,
            ..form
        });
    }
}

// -- answers ----------------------------------------------------------------

impl Screen {
    fn on_msg(&mut self, msg: Msg, cx: &mut Context<Self>) {
        match msg {
            Msg::Status(reply) => match *reply {
                Ok(status) => {
                    self.apply_status(status);
                    if !self.candidates_asked {
                        self.load_candidates(cx);
                    }
                    cx.notify();
                }
                Err(error) => {
                    self.status = Phase::Failed(error);
                    cx.notify();
                }
            },
            Msg::Candidates(reply) => {
                match *reply {
                    Ok(answer) => {
                        let mut rows = Vec::new();
                        for value in answer.candidates {
                            match serde_json::from_value::<Candidate>(value) {
                                Ok(row) => rows.push(row),
                                Err(error) => {
                                    tracing::debug!(%error, "candidat illisible");
                                }
                            }
                        }
                        self.labelled = answer.labelled;
                        self.candidates = Phase::Ready(rows);
                        self.clamp_cursor();
                    }
                    Err(error) => self.candidates = Phase::Failed(error),
                }
                cx.notify();
            }
            Msg::Label(reply, last, undo) => {
                let last = *last;
                if let Err(error) = *reply {
                    // The optimistic move rolls back, and the cause is said.
                    if undo {
                        *self.counts.entry(last.applied.clone()).or_insert(0) += 1;
                        self.labelled += 1;
                    } else if let Some(count) = self.counts.get_mut(&last.applied) {
                        *count = count.saturating_sub(1);
                        self.labelled = self.labelled.saturating_sub(1);
                    }
                    self.toast(
                        ToastKind::Error,
                        format!("Étiquette non enregistrée : {}", error.cause_fr()),
                        cx,
                    );
                }
                if self.sheet.open {
                    self.sheet.saving = false;
                }
                cx.notify();
            }
            Msg::Sheet(reply) => {
                match *reply {
                    Ok(body) => {
                        let track = self.sheet.track.clone();
                        let title = self.sheet.title.clone();
                        self.sheet.answered(&body);
                        self.sheet.track = track;
                        self.sheet.title = title;
                    }
                    Err(error) => self.sheet.failed(error.cause_fr()),
                }
                cx.notify();
            }
            Msg::SheetCleared(reply) => {
                self.sheet.saving = false;
                if let Err(error) = *reply {
                    self.toast(
                        ToastKind::Error,
                        format!("Étiquette non retirée : {}", error.cause_fr()),
                        cx,
                    );
                }
                cx.notify();
            }
            Msg::Tree(reply) => {
                match *reply {
                    Ok(tree) => {
                        self.tree.set(&tree);
                        self.tree.cursor = self
                            .tree
                            .cursor
                            .min(self.tree.rows().len().saturating_sub(1));
                    }
                    Err(error) => self.tree.error = Some(error.cause_fr().into()),
                }
                cx.notify();
            }
            Msg::TreeWritten(reply) => {
                self.tree.busy = false;
                match *reply {
                    Ok(body) => {
                        // Every write answers the whole tree (`_tree_answer`), so
                        // the screen paints the server's version, never its own
                        // guess at what the DAG became.
                        if let Ok(tree) = serde_json::from_value::<TreeReply>(body) {
                            self.tree.set(&tree);
                        } else {
                            // A tag edit/delete answers the tag, not the tree:
                            // ask the tree rather than pretend nothing moved.
                            self.refresh_tree(cx);
                        }
                        self.tree.editor = None;
                        self.tree_parent = None;
                        self.tree.draft.clear();
                        self.tree.linking = None;
                        self.refresh_status(cx);
                    }
                    Err(error) => {
                        self.tree.error = Some(match error {
                            ApiError::Http { status: 409, .. } => {
                                "Ce lien ferait d'un genre son propre ancêtre.".into()
                            }
                            other => other.cause_fr().into(),
                        });
                    }
                }
                cx.notify();
            }
            Msg::TagCreated(reply, name, parent) => {
                self.tree.busy = false;
                match *reply {
                    Ok(body) => {
                        self.tree.draft.clear();
                        self.tree.editor = None;
                        self.tree_parent = None;
                        if let Some(parent) = parent {
                            let id = body
                                .get("id")
                                .and_then(|id| id.as_i64())
                                .map(|id| id.to_string())
                                .unwrap_or_else(|| name.clone());
                            let Some(api) = self.api(cx) else {
                                return;
                            };
                            let tx = self.events.clone();
                            let runtime = self.runtime(cx);
                            runtime.spawn(async move {
                                let reply =
                                    api.genre_tree_edge(&id, &parent, 1.0, Method::POST).await;
                                tx.send(Msg::TreeWritten(Box::new(reply)));
                            });
                        } else {
                            self.refresh_tree(cx);
                        }
                        self.refresh_status(cx);
                        self.note = Some(format!("Genre « {name} » créé").into());
                    }
                    Err(error) => {
                        self.tree.error = Some(error.cause_fr().into());
                    }
                }
                cx.notify();
            }
            Msg::Plan(reply) => {
                match *reply {
                    Ok(plan) => {
                        if !self.run.running {
                            self.where_choice = plan.r#where.clone();
                        }
                        self.plan = Phase::Ready(plan);
                    }
                    Err(error) => self.plan = Phase::Failed(error),
                }
                cx.notify();
            }
            Msg::Backfill(reply) => {
                let was_running = matches!(&self.backfill, Phase::Ready(job) if job.job.running);
                match *reply {
                    Ok(job) => {
                        let running = job.job.running;
                        self.backfill = Phase::Ready(job);
                        if running {
                            self.watch_backfill(cx);
                        }
                        if was_running && !running {
                            self.toast(ToastKind::Info, "Mesure de la bibliothèque terminée", cx);
                            self.refresh_status(cx);
                            self.load_candidates(cx);
                        }
                    }
                    Err(error) => self.backfill = Phase::Failed(error),
                }
                cx.notify();
            }
            Msg::EmbedStatus(reply) => {
                let was_running = matches!(&self.embed, Phase::Ready(job) if job.job.running);
                match *reply {
                    Ok(job) => {
                        let running = job.job.running;
                        self.embed = Phase::Ready(job);
                        if running {
                            self.watch_embed(cx);
                        }
                        if was_running && !running {
                            self.toast(ToastKind::Info, "Calcul des empreintes terminé", cx);
                            self.refresh_status(cx);
                        }
                    }
                    Err(error) => self.embed = Phase::Failed(error),
                }
                cx.notify();
            }
            Msg::ExplorerAdmin(reply) => {
                match *reply {
                    Ok(explorer) => self.explorer_admin = Phase::Ready(explorer),
                    Err(error) => self.explorer_admin = Phase::Failed(error),
                }
                cx.notify();
            }
            Msg::Where(reply) => match *reply {
                Ok(()) => {
                    self.toast(
                        ToastKind::Info,
                        if self.where_choice == "client" {
                            "La bibliothèque sera mesurée sur cet ordinateur"
                        } else {
                            "La bibliothèque sera mesurée sur le serveur"
                        },
                        cx,
                    );
                }
                Err(error) => {
                    self.where_choice = if self.where_choice == "client" {
                        "server".to_owned()
                    } else {
                        "client".to_owned()
                    };
                    self.toast(
                        ToastKind::Error,
                        format!("Choix non enregistré : {}", error.cause_fr()),
                        cx,
                    );
                }
            },
            Msg::Analysis(event) => {
                match event {
                    analysis::Event::Started { total } => {
                        self.run.running = true;
                        self.run.total = total;
                        self.run.current = String::new();
                    }
                    analysis::Event::Track {
                        done,
                        failed,
                        total,
                        title,
                        cause,
                    } => {
                        self.run.done = done;
                        self.run.failed = failed;
                        self.run.total = total;
                        self.run.current = title.clone();
                        if let Some(cause) = cause {
                            *self.run.causes.entry(cause).or_insert(0) += 1;
                        }
                    }
                    analysis::Event::Finished {
                        done,
                        failed,
                        causes,
                        stopped,
                    } => {
                        self.run.running = false;
                        self.run.done = done;
                        self.run.failed = failed;
                        self.run.causes = causes;
                        self.run.stopped = stopped.clone();
                        self.run.control = None;
                        self.run.runner = None;
                        self.toast(
                            if failed > 0 {
                                ToastKind::Error
                            } else {
                                ToastKind::Info
                            },
                            match &stopped {
                                Some(cause) => format!("Mesure locale arrêtée : {cause}"),
                                None => format!(
                                    "Mesure locale terminée : {done} titre(s), {failed} échec(s)"
                                ),
                            },
                            cx,
                        );
                        self.refresh_status(cx);
                        self.load_candidates(cx);
                    }
                }
                cx.notify();
            }
            Msg::TrainStage(stage, fraction) => {
                self.training.stage = stage.to_owned();
                self.training.fraction = fraction;
                cx.notify();
            }
            Msg::TrainPrepared(reply) => match *reply {
                Ok((prepared, pool)) => {
                    self.training.stage = "entraînement".to_owned();
                    self.training.fraction = 0.55;
                    let control = Arc::new(Control::default());
                    let (handle, rx) = training::start(prepared, self.training.mode, pool, control);
                    let _ = handle;
                    let tx = self.events.clone();
                    let runtime = self.runtime(cx);
                    runtime.spawn(async move {
                        while let Some(event) = rx.next().await {
                            tx.send(Msg::TrainEvent(event));
                        }
                    });
                }
                Err(error) => {
                    self.training.running = false;
                    self.training.error = Some(error.into());
                }
            },
            Msg::TrainEvent(event) => match event {
                training::Event::Progress { stage, fraction } => {
                    self.training.stage = stage.to_owned();
                    self.training.fraction = 0.55 + 0.45 * fraction;
                }
                training::Event::Done(outcome) => {
                    self.training.running = false;
                    self.training.fraction = 1.0;
                    self.training.stage = "terminé".to_owned();
                    if outcome.health.level != "poor" {
                        self.send_head(&outcome, cx);
                    }
                    self.training.head = Some(*outcome);
                }
                training::Event::Failed(error) => {
                    self.training.running = false;
                    self.training.error = Some(error.into());
                }
            },
            Msg::ModelPut(reply) => {
                self.training.sending = false;
                match *reply {
                    Ok(body) => {
                        self.training.sent = true;
                        if let Some(relabel) = body.get("relabel")
                            && let Ok(relabel) =
                                serde_json::from_value::<GenreRelabel>(relabel.clone())
                        {
                            self.training.relabel = Some(relabel);
                        }
                        self.toast(
                            ToastKind::Info,
                            "Modèle envoyé — il s'applique à toute la bibliothèque",
                            cx,
                        );
                        self.watch_relabel(cx);
                        self.refresh_status(cx);
                    }
                    Err(error) => {
                        self.toast(
                            ToastKind::Error,
                            format!("Modèle refusé : {}", error.cause_fr()),
                            cx,
                        );
                    }
                }
                cx.notify();
            }
            Msg::ModelDeleted(reply) => match *reply {
                Ok(()) => {
                    self.training.head = None;
                    self.training.sent = false;
                    self.toast(ToastKind::Info, "Modèle désactivé", cx);
                    self.refresh_status(cx);
                    self.watch_relabel(cx);
                }
                Err(error) => self.toast(
                    ToastKind::Error,
                    format!("Désactivation impossible : {}", error.cause_fr()),
                    cx,
                ),
            },
            Msg::Relabel(reply) => {
                if let Ok(relabel) = *reply {
                    let running = relabel.running;
                    self.training.relabel = Some(relabel);
                    if running {
                        self.watch_relabel(cx);
                    } else {
                        self.refresh_status(cx);
                    }
                }
                cx.notify();
            }
            Msg::ExtractorTest(reply) => {
                match *reply {
                    Ok(test) => {
                        self.extractor_card.test_running = test.running;
                        self.extractor_card.answered_test(&serde_json::json!({
                            "running": test.running, "ok": test.ok, "error": test.error,
                            "progress": test.progress,
                            "result": test.result,
                        }));
                        if test.running {
                            self.watch_extractor_test(cx);
                        }
                    }
                    Err(error) => self.extractor_card.failed(error.cause_fr()),
                }
                cx.notify();
            }
            Msg::ExtractorUpload(reply) => match *reply {
                Ok(body) => {
                    self.extractor_card.answered_upload(&body);
                    self.toast(ToastKind::Info, "Extracteur installé", cx);
                    self.refresh_status(cx);
                }
                Err(error) => {
                    self.extractor_card.uploading = false;
                    self.extractor_card.error = Some(error.into());
                }
            },
            Msg::ExtractorDeleted(reply) => match *reply {
                Ok(()) => {
                    self.extractor_card
                        .answered_delete(&serde_json::json!({"ok": true}));
                    self.toast(ToastKind::Info, "Extracteur retiré", cx);
                    self.refresh_status(cx);
                }
                Err(error) => self.extractor_card.failed(error.cause_fr()),
            },
            Msg::ShareExported(reply) => {
                self.share.exporting = false;
                match *reply {
                    Ok(body) => {
                        // A plain JSON file, written where the person looks for
                        // downloads; the path is said, so it can be found.
                        let directory = directories::UserDirs::new()
                            .and_then(|dirs| dirs.download_dir().map(|dir| dir.to_path_buf()))
                            .unwrap_or_else(std::env::temp_dir);
                        let stamp = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|since| since.as_secs())
                            .unwrap_or(0);
                        let path = directory.join(format!("modele-genres-{stamp}.nsgenre.json"));
                        match std::fs::write(
                            &path,
                            serde_json::to_vec_pretty(&body).unwrap_or_default(),
                        ) {
                            Ok(()) => {
                                self.share.note =
                                    Some(format!("Modèle exporté : {}", path.display()).into());
                                self.share.error = None;
                            }
                            Err(error) => {
                                self.share.error =
                                    Some(format!("écriture impossible : {error}").into());
                            }
                        }
                    }
                    Err(error) => self.share.error = Some(error.into()),
                }
                cx.notify();
            }
            Msg::SharePreview(reply) => {
                self.share.importing = false;
                match *reply {
                    Ok(preview) => {
                        self.share.preview = Some(preview);
                        self.share.error = None;
                    }
                    Err(error) => self.share.error = Some(error.cause_fr().into()),
                }
                cx.notify();
            }
            Msg::ShareImported(reply) => {
                self.share.importing = false;
                match *reply {
                    Ok(body) => {
                        self.share.preview = None;
                        self.share.note = Some("Modèle importé".into());
                        if let Ok(job) = serde_json::from_value::<GenreShareStatus>(
                            body.get("job").cloned().unwrap_or(Value::Null),
                        ) {
                            self.share.job = Some(job);
                        }
                        self.refresh_status(cx);
                        self.watch_share(cx);
                    }
                    Err(error) => self.share.error = Some(error.cause_fr().into()),
                }
                cx.notify();
            }
            Msg::ShareJob(reply) => {
                if let Ok(job) = *reply {
                    let running = job.running;
                    self.share.job = Some(job);
                    if running {
                        self.watch_share(cx);
                    } else {
                        self.refresh_status(cx);
                    }
                }
                cx.notify();
            }
            Msg::BulkPreview(reply) => {
                match *reply {
                    Ok(body) => self.bulk.answered(&body),
                    Err(error) => self.bulk.failed(error.cause_fr()),
                }
                cx.notify();
            }
            Msg::BulkApply(reply) => {
                self.bulk.applying = false;
                match *reply {
                    Ok(body) => {
                        let applied = body
                            .get("applied")
                            .and_then(|applied| applied.as_u64())
                            .unwrap_or(0);
                        self.bulk.applied(&body);
                        self.toast(
                            ToastKind::Info,
                            format!("{applied} titre(s) étiqueté(s)"),
                            cx,
                        );
                        self.refresh_status(cx);
                        self.load_candidates(cx);
                    }
                    Err(error) => self.bulk.failed(error.cause_fr()),
                }
                cx.notify();
            }
            Msg::Album(reply) => match *reply {
                Ok(page) => {
                    let tracks: Vec<String> = page
                        .tracks
                        .iter()
                        .map(|track| track.deezer_id.to_string())
                        .collect();
                    self.preview_bulk(
                        Scope {
                            kind: ScopeKind::Album,
                            title: page.album.title,
                            tracks,
                            artist: None,
                        },
                        cx,
                    );
                }
                Err(error) => self.bulk_form_failed(error.cause_fr(), cx),
            },
            Msg::Playlist(reply) => match *reply {
                Ok(page) => {
                    let tracks: Vec<String> = page
                        .tracks
                        .iter()
                        .map(|track| track.deezer_id.to_string())
                        .collect();
                    self.preview_bulk(
                        Scope {
                            kind: ScopeKind::Playlist,
                            title: page.playlist.title,
                            tracks,
                            artist: None,
                        },
                        cx,
                    );
                }
                Err(error) => self.bulk_form_failed(error.cause_fr(), cx),
            },
            Msg::Styles(reply) => {
                match *reply {
                    Ok(body) => self.explorer.answered_styles(&body),
                    Err(error) => self.explorer.failed_styles(error.cause_fr()),
                }
                cx.notify();
            }
            Msg::Discover(reply) => {
                match *reply {
                    Ok(body) => {
                        self.explorer.answered(&body);
                        if self.explorer.state == "pending" {
                            self.schedule_poll(cx);
                        }
                    }
                    Err(error) => self.explorer.failed_discover(error.cause_fr()),
                }
                cx.notify();
            }
            Msg::Station(reply) => match *reply {
                Ok(list) => {
                    if list.tracks.is_empty() {
                        self.toast(ToastKind::Info, "Cette station n'a rien proposé", cx);
                    } else {
                        let entries: Vec<nz_player::Entry> =
                            list.tracks.iter().map(entry_from_track).collect();
                        self.state.update(cx, |state, cx| {
                            state.command(nz_player::Command::play_queue(entries, 0, None), cx)
                        });
                    }
                }
                Err(error) => self.toast(
                    ToastKind::Error,
                    format!("Station impossible : {}", error.cause_fr()),
                    cx,
                ),
            },
            Msg::Labelled(reply) => {
                // Only a failure arrives here: the fetch loop hands the prepared
                // matrix over itself (`Msg::TrainPrepared`).
                if let Err(error) = *reply {
                    self.training.running = false;
                    self.training.error = Some(error.cause_fr().into());
                }
                cx.notify();
            }
            Msg::Cover(url, file) => {
                self.covers.borrow_mut().resolved(&url, file);
                cx.notify();
            }
        }
    }

    fn refresh_status(&mut self, cx: &mut Context<Self>) {
        self.load_status(cx);
    }

    fn refresh_tree(&mut self, cx: &mut Context<Self>) {
        self.tree_loaded = false;
        self.load_tree(cx);
    }

    fn bulk_form_failed(&mut self, cause: String, cx: &mut Context<Self>) {
        if let Some(form) = self.bulk_form.as_mut() {
            form.busy = false;
            form.error = Some(cause.into());
        }
        cx.notify();
    }

    /// The tag the bulk form applies: the tagging tab's first ranked genre is
    /// NOT assumed. The form picks one explicitly, and the click that picks it
    /// is the click that previews.
    fn preview_bulk(&mut self, scope: Scope, cx: &mut Context<Self>) {
        let Some(tag) = self.bulk_tag.clone() else {
            self.bulk_form_failed("Choisissez le genre à appliquer.".into(), cx);
            return;
        };
        let Some(api) = self.api(cx) else {
            return;
        };
        self.bulk = genre_bulk_sheet::Sheet::open(scope.clone(), tag.clone());
        if let Some(form) = self.bulk_form.as_mut() {
            form.busy = false;
        }
        let body = scope.body(&tag.name);
        let tx = self.events.clone();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            let reply = api
                .send::<Value, Value>(Lane::User, Method::POST, "/genre/bulk/preview", Some(&body))
                .await
                .map(|value| value.unwrap_or(Value::Null));
            tx.send(Msg::BulkPreview(Box::new(reply)));
        });
        cx.notify();
    }

    fn on_bulk_action(&mut self, action: genre_bulk_sheet::Action, cx: &mut Context<Self>) {
        match action {
            genre_bulk_sheet::Action::Close => {
                self.bulk.close();
                cx.notify();
            }
            genre_bulk_sheet::Action::Toggle(index) => {
                self.bulk.toggle(index);
                cx.notify();
            }
            genre_bulk_sheet::Action::Correct(index) => {
                let Some(row) = self.bulk.rows.get(index).cloned() else {
                    return;
                };
                let Some(api) = self.api(cx) else {
                    return;
                };
                self.sheet = Sheet::open(row.id.clone(), row.title.clone());
                let tx = self.events.clone();
                let runtime = self.runtime(cx);
                let path = self.sheet.path();
                runtime.spawn(async move {
                    let reply = api.get::<Value>(Lane::User, &path).await;
                    tx.send(Msg::Sheet(Box::new(reply.map(|fresh| fresh.value))));
                });
                cx.notify();
            }
            genre_bulk_sheet::Action::Apply => {
                let Some(api) = self.api(cx) else {
                    return;
                };
                let Some(tag) = self.bulk.tag.clone() else {
                    return;
                };
                let selection = self.bulk.selection();
                if selection.is_empty() {
                    return;
                }
                self.bulk.applying = true;
                let tx = self.events.clone();
                let runtime = self.runtime(cx);
                runtime.spawn(async move {
                    let reply = api.genre_bulk(&selection, &tag.id.to_string()).await;
                    tx.send(Msg::BulkApply(Box::new(reply)));
                });
                cx.notify();
            }
        }
    }

    fn send_head(&mut self, outcome: &training::Outcome, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        let families = training::families(&outcome.labels, &self.tree);
        let payload = training::payload(outcome, &families);
        self.training.sending = true;
        let tx = self.events.clone();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            let reply = api.genre_model_put(&payload).await;
            tx.send(Msg::ModelPut(Box::new(reply)));
        });
        cx.notify();
    }

    /// Poll the server's re-decision every 1.2 s while it runs — its own beat,
    /// not the UI's.
    fn watch_relabel(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1_200))
                .await;
            let _ = this.update(cx, |this, cx| {
                let tx = this.events.clone();
                let runtime = this.runtime(cx);
                let api = Arc::clone(&api);
                runtime.spawn(async move {
                    let reply = api.genre_relabel().await;
                    tx.send(Msg::Relabel(Box::new(reply)));
                });
            });
        })
        .detach();
    }

    fn watch_extractor_test(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(800))
                .await;
            let _ = this.update(cx, |this, cx| {
                let tx = this.events.clone();
                let runtime = this.runtime(cx);
                let api = Arc::clone(&api);
                runtime.spawn(async move {
                    let reply = api.genre_extractor_test_status().await;
                    tx.send(Msg::ExtractorTest(Box::new(reply)));
                });
            });
        })
        .detach();
    }

    fn watch_share(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1_200))
                .await;
            let _ = this.update(cx, |this, cx| {
                let tx = this.events.clone();
                let runtime = this.runtime(cx);
                let api = Arc::clone(&api);
                runtime.spawn(async move {
                    let reply = api.genre_share_status().await;
                    tx.send(Msg::ShareJob(Box::new(reply)));
                });
            });
        })
        .detach();
    }

    fn schedule_poll(&mut self, cx: &mut Context<Self>) {
        if !self.explorer.wants_poll() {
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(genre_explorer::POLL_MS))
                .await;
            let _ = this.update(cx, |this, cx| this.poll_discover(cx));
        })
        .detach();
    }

    fn poll_discover(&mut self, cx: &mut Context<Self>) {
        if !self.explorer.wants_poll() {
            return;
        }
        let Some(api) = self.api(cx) else {
            return;
        };
        let Some(path) = self.explorer.discover_path() else {
            return;
        };
        self.explorer.polled();
        let tx = self.events.clone();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            let reply = api.get::<Value>(Lane::User, &path).await;
            tx.send(Msg::Discover(Box::new(reply.map(|fresh| fresh.value))));
        });
    }

    fn discover(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        let Some(body) = self.explorer.discover_body() else {
            return;
        };
        self.explorer.polls = 0;
        self.explorer.asking = true;
        self.explorer.state = "pending".to_owned();
        self.explorer.discovery_error = None;
        let tx = self.events.clone();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            let reply = api
                .send::<Value, Value>(Lane::User, Method::POST, "/explorer/discover", Some(&body))
                .await;
            tx.send(Msg::Discover(Box::new(
                reply.map(|value| value.unwrap_or(Value::Null)),
            )));
        });
        self.schedule_poll(cx);
        cx.notify();
    }

    fn station(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        let Some(style) = self.explorer.selected_name().map(str::to_owned) else {
            return;
        };
        let level = (f64::from(self.explorer.level) - 1.0) / 4.0;
        let tx = self.events.clone();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            let reply = api.genre_radio(&style, Some(level)).await;
            tx.send(Msg::Station(Box::new(reply)));
        });
    }

    fn play_playlist(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(playlist) = self.explorer.playlists.get(index) else {
            return;
        };
        let mut entries: Vec<nz_player::Entry> = Vec::new();
        for value in &playlist.tracks {
            let id = value
                .get("deezer_id")
                .and_then(|id| id.as_str())
                .unwrap_or_default();
            if id.is_empty() {
                continue;
            }
            let artist = value
                .get("artist")
                .and_then(|artist| artist.as_str())
                .unwrap_or_default();
            let album = value
                .get("album")
                .and_then(|album| album.as_str())
                .unwrap_or_default();
            let title = value
                .get("title")
                .and_then(|title| title.as_str())
                .unwrap_or_default();
            if title.is_empty() {
                continue;
            }
            let track = Track {
                deezer_id: nz_model::TrackId::from(id),
                title: title.to_owned(),
                duration: value.get("duration").and_then(|d| d.as_u64()).unwrap_or(0) as u32,
                artist: nz_model::TrackArtist {
                    deezer_id: nz_model::ArtistId::from(
                        value
                            .get("artist_ref")
                            .and_then(|a| a.as_str())
                            .unwrap_or_default(),
                    ),
                    name: artist.to_owned(),
                },
                album: nz_model::TrackAlbum {
                    deezer_id: nz_model::AlbumId::from(
                        value
                            .get("album_ref")
                            .and_then(|a| a.as_str())
                            .unwrap_or_default(),
                    ),
                    title: album.to_owned(),
                    cover: None,
                },
                ..Track::default()
            };
            entries.push(entry_from_track(&track));
        }
        if entries.is_empty() {
            self.toast(
                ToastKind::Info,
                "Cette playlist n'a pas de titre vérifié",
                cx,
            );
            return;
        }
        self.state.update(cx, |state, cx| {
            state.command(nz_player::Command::play_queue(entries, 0, None), cx)
        });
    }

    /// `POST /genre/embed`'s answer is the job's first state; the poll then
    /// follows it.
    fn embed_started(&mut self, body: Value, cx: &mut Context<Self>) {
        if let Ok(job) = serde_json::from_value::<GenreEmbed>(body) {
            self.embed = Phase::Ready(job);
        }
        self.watch_embed(cx);
    }

    fn watch_embed(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1_500))
                .await;
            let _ = this.update(cx, |this, cx| {
                let tx = this.events.clone();
                let runtime = this.runtime(cx);
                let api = Arc::clone(&api);
                runtime.spawn(async move {
                    let reply = api.genre_embed_status().await;
                    tx.send(Msg::EmbedStatus(Box::new(reply)));
                });
            });
        })
        .detach();
    }

    /// The measuring start is a POST the screen makes once; the poll follows
    /// while it runs (1.5 s, the page's own beat).
    fn watch_backfill(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1_500))
                .await;
            let _ = this.update(cx, |this, cx| {
                let tx = this.events.clone();
                let runtime = this.runtime(cx);
                let api = Arc::clone(&api);
                runtime.spawn(async move {
                    let reply = api.analysis_backfill_status().await;
                    tx.send(Msg::Backfill(Box::new(reply)));
                });
            });
        })
        .detach();
    }
}

// -- training: fetch, prepare, hand to the thread -----------------------------

impl Screen {
    fn start_training(&mut self, cx: &mut Context<Self>) {
        if self.training.running {
            return;
        }
        let Some(api) = self.api(cx) else {
            return;
        };
        self.load_tree(cx);
        self.training.running = true;
        self.training.error = None;
        self.training.head = None;
        self.training.sent = false;
        self.training.stage = "étiquettes".to_owned();
        self.training.fraction = 0.0;
        let tx = self.events.clone();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            // The labelled set: ONE statement on the server, all of it.
            let labelled = match api.get::<Value>(Lane::Bulk, "/genre/labelled").await {
                Ok(fresh) => fresh.value,
                Err(error) => {
                    tx.send(Msg::Labelled(Box::new(Err(error))));
                    return;
                }
            };
            let rows: Vec<training::Labelled> = labelled
                .get("labelled")
                .and_then(Value::as_array)
                .map(|rows| {
                    rows.iter()
                        .filter_map(|row| serde_json::from_value(row.clone()).ok())
                        .collect()
                })
                .unwrap_or_default();
            if rows.len() < 4 {
                tx.send(Msg::Labelled(Box::new(Err(ApiError::Decode(
                    "au moins deux genres de cinq titres sont nécessaires".to_owned(),
                )))));
                return;
            }
            let ids: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
            let mut vectors: HashMap<String, Vec<f32>> = HashMap::new();
            let batches = ids.len().div_ceil(200);
            for (index, chunk) in ids.chunks(200).enumerate() {
                let reply = api
                    .send::<Value, Value>(
                        Lane::Bulk,
                        Method::POST,
                        "/genre/embeddings",
                        Some(&serde_json::json!({"ids": chunk})),
                    )
                    .await;
                match reply {
                    Ok(Some(body)) => read_vectors(&body, &mut vectors),
                    Ok(None) => {}
                    Err(error) => {
                        tx.send(Msg::Labelled(Box::new(Err(error))));
                        return;
                    }
                }
                tx.send(Msg::TrainStage(
                    "vecteurs",
                    0.2 * (index + 1) as f64 / batches.max(1) as f64,
                ));
            }
            let prepared = match training::prepare(&rows, &vectors) {
                Ok(prepared) => prepared,
                Err(error) => {
                    tx.send(Msg::Labelled(Box::new(Err(ApiError::Decode(error)))));
                    return;
                }
            };
            tx.send(Msg::TrainStage("préparation", 0.25));
            // Self-training: untagged tracks that already have a vector, most
            // played first. Best effort — without them the head is the one the
            // labels make.
            let mut pool: Option<Vec<f32>> = None;
            if !rows.is_empty() {
                let p = api
                    .get::<Value>(Lane::Bulk, "/genre/pool?limit=600")
                    .await
                    .ok()
                    .map(|fresh| fresh.value);
                if let Some(ids) = p
                    .as_ref()
                    .and_then(|value| value.get("ids"))
                    .and_then(Value::as_array)
                {
                    let ids: Vec<String> = ids
                        .iter()
                        .filter_map(|id| match id {
                            Value::String(text) => Some(text.clone()),
                            Value::Number(number) => Some(number.to_string()),
                            _ => None,
                        })
                        .collect();
                    let mut flat: Vec<f32> = Vec::new();
                    for (index, chunk) in ids.chunks(200).enumerate() {
                        let reply = api
                            .send::<Value, Value>(
                                Lane::Bulk,
                                Method::POST,
                                "/genre/embeddings",
                                Some(&serde_json::json!({"ids": chunk})),
                            )
                            .await;
                        if let Ok(Some(body)) = reply {
                            let mut found = HashMap::new();
                            read_vectors(&body, &mut found);
                            let mut ordered: Vec<Vec<f32>> = found.into_values().collect();
                            ordered.sort_by_key(|vector| vector.len());
                            for vector in ordered {
                                if vector.len() >= prepared.d {
                                    flat.extend_from_slice(&vector[..prepared.d]);
                                }
                            }
                        }
                        tx.send(Msg::TrainStage(
                            "titres non étiquetés",
                            0.25 + 0.2 * (index + 1) as f64 / ids.len().div_ceil(200).max(1) as f64,
                        ));
                    }
                    if !flat.is_empty() {
                        pool = Some(flat);
                    }
                }
            }
            tx.send(Msg::TrainPrepared(Box::new(Ok((prepared, pool)))));
        });
        cx.notify();
    }
}

/// `{"embeddings": {ref: base64}}` → the decoded vectors, appended to `into`.
fn read_vectors(body: &Value, into: &mut HashMap<String, Vec<f32>>) {
    let Some(map) = body.get("embeddings").and_then(Value::as_object) else {
        return;
    };
    for (key, value) in map {
        if let Some(text) = value.as_str() {
            let vector = training::decode_embedding(text);
            if !vector.is_empty() {
                into.insert(key.clone(), vector);
            }
        }
    }
}

// -- the actions the page's controls take -----------------------------------

impl Screen {
    fn set_where(&mut self, place: &str, cx: &mut Context<Self>) {
        if self.where_choice == place {
            return;
        }
        let Some(api) = self.api(cx) else {
            return;
        };
        self.where_choice = place.to_owned();
        let tx = self.events.clone();
        let runtime = self.runtime(cx);
        let place = place.to_owned();
        runtime.spawn(async move {
            tx.send(Msg::Where(Box::new(api.analysis_where(&place).await)));
        });
        cx.notify();
    }

    fn start_backfill(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        let force = self.force;
        let workers = self.workers;
        self.force = false;
        let tx = self.events.clone();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            match api.analysis_backfill(force, workers).await {
                Ok(_) => {
                    let reply = api.analysis_backfill_status().await;
                    tx.send(Msg::Backfill(Box::new(reply)));
                }
                Err(error) => tx.send(Msg::Backfill(Box::new(Err(error)))),
            }
        });
        self.watch_backfill(cx);
        cx.notify();
    }

    fn stop_backfill(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        let tx = self.events.clone();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            let _ = api.analysis_backfill_stop().await;
            let reply = api.analysis_backfill_status().await;
            tx.send(Msg::Backfill(Box::new(reply)));
        });
    }

    fn start_embed(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        let force = self.force;
        self.force = false;
        let tx = self.events.clone();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            match api.genre_embed(force).await {
                Ok(body) => {
                    if let Ok(job) = serde_json::from_value::<GenreEmbed>(body) {
                        tx.send(Msg::EmbedStatus(Box::new(Ok(job))));
                    } else {
                        tx.send(Msg::EmbedStatus(Box::new(api.genre_embed_status().await)));
                    }
                }
                Err(error) => tx.send(Msg::EmbedStatus(Box::new(Err(error)))),
            }
        });
        self.watch_embed(cx);
        cx.notify();
    }

    fn set_schedule(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        let body = if enabled {
            serde_json::json!({"enabled": true, "at": "03:00", "hours": 6})
        } else {
            serde_json::json!({"enabled": false})
        };
        let tx = self.events.clone();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            let _ = api.analysis_schedule(&body).await;
            let reply = api.analysis_plan().await;
            tx.send(Msg::Plan(Box::new(reply)));
        });
    }

    fn set_explorer_switches(&mut self, enabled: bool, archive: bool, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        let tx = self.events.clone();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            let reply = api
                .explorer(Some(&serde_json::json!({
                    "enabled": enabled, "archive": archive
                })))
                .await;
            tx.send(Msg::ExplorerAdmin(Box::new(reply)));
        });
    }

    fn start_analysis_run(&mut self, cx: &mut Context<Self>) {
        if self.run.running {
            return;
        }
        let Some(api) = self.api(cx) else {
            return;
        };
        let spool = std::env::temp_dir().join(format!("neezer-analysis-{}", std::process::id()));
        if let Err(error) = std::fs::create_dir_all(&spool) {
            self.toast(
                ToastKind::Error,
                format!("Dossier temporaire impossible : {error}"),
                cx,
            );
            return;
        }
        let control = Arc::new(Control::default());
        let (runner, rx) = Runner::start(api, self.runtime(cx), spool, Arc::clone(&control));
        self.run = Run {
            control: Some(control),
            runner: Some(runner),
            running: true,
            ..Run::default()
        };
        let tx = self.events.clone();
        cx.spawn(async move |this, cx| {
            while let Some(event) = rx.next().await {
                tx.send(Msg::Analysis(event));
                let _ = this.update(cx, |_, cx| cx.notify());
            }
        })
        .detach();
        cx.notify();
    }

    fn stop_analysis_run(&mut self, cx: &mut Context<Self>) {
        if let Some(control) = &self.run.control {
            control.cancel();
        }
        self.note = Some("Arrêt demandé : le titre en cours se termine.".into());
        cx.notify();
    }

    fn on_tree_action(&mut self, action: genre_tree::Action, cx: &mut Context<Self>) {
        match action {
            genre_tree::Action::Select(index) => {
                self.tree.select(index);
            }
            genre_tree::Action::Toggle(name) => {
                self.tree.toggle(&name);
            }
            genre_tree::Action::CreateRoot => {
                self.tree.editor = Some(genre_tree::Editor::Create);
                self.tree.draft.clear();
                self.tree_parent = None;
                self.tree.linking = None;
            }
            genre_tree::Action::CreateChild => {
                let Some(row) = self.tree.selected() else {
                    return;
                };
                self.tree.editor = Some(genre_tree::Editor::Create);
                self.tree.draft.clear();
                self.tree_parent = Some(row.name);
                self.tree.linking = None;
            }
            genre_tree::Action::Rename => {
                let Some(row) = self.tree.selected() else {
                    return;
                };
                self.tree.editor = Some(genre_tree::Editor::Rename);
                self.tree.draft = row.name;
                self.tree.linking = None;
            }
            genre_tree::Action::CancelEditor => {
                self.tree.editor = None;
                self.tree.draft.clear();
                self.tree_parent = None;
            }
            genre_tree::Action::SubmitEditor => match self.tree.editor {
                Some(genre_tree::Editor::Create) => {
                    self.create_tree_node(cx);
                    return;
                }
                Some(genre_tree::Editor::Rename) => {
                    let Some(row) = self.tree.selected() else {
                        return;
                    };
                    let name = self.tree.draft_name();
                    if name.is_empty() {
                        return;
                    }
                    let Some(api) = self.api(cx) else {
                        return;
                    };
                    let id = row.tag.id;
                    self.tree.busy = true;
                    let tx = self.events.clone();
                    let runtime = self.runtime(cx);
                    runtime.spawn(async move {
                        let reply = api
                            .genre_tag_edit(id, &serde_json::json!({"name": name}))
                            .await;
                        tx.send(Msg::TreeWritten(Box::new(reply)));
                    });
                }
                None => {}
            },
            genre_tree::Action::Delete => {
                let Some(row) = self.tree.selected() else {
                    return;
                };
                if row.tag.id == 0 {
                    self.tree.error = Some("Ce genre n'a pas d'étiquette à supprimer.".into());
                    cx.notify();
                    return;
                }
                let Some(api) = self.api(cx) else {
                    return;
                };
                self.tree.busy = true;
                let tx = self.events.clone();
                let runtime = self.runtime(cx);
                runtime.spawn(async move {
                    let reply = api.genre_tag_delete(row.tag.id).await;
                    tx.send(Msg::TreeWritten(Box::new(reply.map(|()| Value::Null))));
                });
            }
            genre_tree::Action::Cut(parent) => {
                let Some(row) = self.tree.selected() else {
                    return;
                };
                let Some(api) = self.api(cx) else {
                    return;
                };
                self.tree.busy = true;
                let tx = self.events.clone();
                let runtime = self.runtime(cx);
                runtime.spawn(async move {
                    let reply = api.genre_tree_cut(&row.name, &parent).await;
                    tx.send(Msg::TreeWritten(Box::new(reply)));
                });
            }
            genre_tree::Action::LinkOpen => {
                let Some(row) = self.tree.selected() else {
                    return;
                };
                self.tree.linking = Some(row.name);
                self.tree.editor = None;
            }
            genre_tree::Action::LinkClose => {
                self.tree.linking = None;
            }
            genre_tree::Action::Link(parent) => {
                let Some(child) = self.tree.linking.clone() else {
                    return;
                };
                let Some(api) = self.api(cx) else {
                    return;
                };
                self.tree.busy = true;
                let tx = self.events.clone();
                let runtime = self.runtime(cx);
                runtime.spawn(async move {
                    let reply = api
                        .genre_tree_edge(&child, &parent, 1.0, Method::POST)
                        .await;
                    tx.send(Msg::TreeWritten(Box::new(reply)));
                });
            }
            genre_tree::Action::Undo => {
                let Some(api) = self.api(cx) else {
                    return;
                };
                self.tree.busy = true;
                let tx = self.events.clone();
                let runtime = self.runtime(cx);
                runtime.spawn(async move {
                    let reply = api.genre_tree_undo().await;
                    tx.send(Msg::TreeWritten(Box::new(reply)));
                });
            }
            genre_tree::Action::Reset => {
                let Some(api) = self.api(cx) else {
                    return;
                };
                self.tree.busy = true;
                let tx = self.events.clone();
                let runtime = self.runtime(cx);
                runtime.spawn(async move {
                    let reply = api.genre_tree_reset().await;
                    tx.send(Msg::TreeWritten(Box::new(reply)));
                });
            }
        }
        cx.notify();
    }

    fn create_tree_node(&mut self, cx: &mut Context<Self>) {
        let name = self.tree.draft_name();
        if name.is_empty() {
            return;
        }
        let Some(api) = self.api(cx) else {
            return;
        };
        let parent = self.tree_parent.clone();
        self.tree.busy = true;
        let tx = self.events.clone();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            let reply = api.genre_tag_create(&name, None, None).await;
            tx.send(Msg::TagCreated(Box::new(reply), name, parent));
        });
        cx.notify();
    }

    fn on_extractor_action(&mut self, action: genre_extractor::Action, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        match action {
            genre_extractor::Action::Upload => {
                let path = PathBuf::from(self.extractor_card.path.trim());
                let server = self.state.read(cx).server.clone();
                let session = Arc::clone(&self.state.read(cx).session);
                let tx = self.events.clone();
                let runtime = self.runtime(cx);
                self.extractor_card.uploading = true;
                runtime.spawn(async move {
                    let reply = genre_extractor::upload(&server, session, path).await;
                    tx.send(Msg::ExtractorUpload(Box::new(reply)));
                });
            }
            genre_extractor::Action::Delete => {
                self.extractor_card.deleting = true;
                let tx = self.events.clone();
                let runtime = self.runtime(cx);
                runtime.spawn(async move {
                    let reply = api.genre_extractor_delete().await;
                    tx.send(Msg::ExtractorDeleted(Box::new(reply)));
                });
            }
            genre_extractor::Action::Test => {
                self.extractor_card.test_running = true;
                let tx = self.events.clone();
                let runtime = self.runtime(cx);
                runtime.spawn(async move {
                    let _ = api.genre_extractor_test().await;
                    let reply = api.genre_extractor_test_status().await;
                    tx.send(Msg::ExtractorTest(Box::new(reply)));
                });
            }
            genre_extractor::Action::Poll => {
                let tx = self.events.clone();
                let runtime = self.runtime(cx);
                runtime.spawn(async move {
                    let reply = api.genre_extractor_test_status().await;
                    tx.send(Msg::ExtractorTest(Box::new(reply)));
                });
            }
        }
        cx.notify();
    }

    fn on_model_action(&mut self, action: ModelAction, cx: &mut Context<Self>) {
        match action {
            ModelAction::Mode(mode) => {
                self.training.mode = mode;
                cx.notify();
            }
            ModelAction::SelfTrain(on) => {
                self.training.self_train = on;
                self.save_pref("genre.selftrain", &on, cx);
                cx.notify();
            }
            ModelAction::Train => self.start_training(cx),
            ModelAction::Send => {
                if let Some(head) = self.training.head.clone() {
                    self.send_head(&head, cx);
                }
            }
            ModelAction::Disable => {
                let Some(api) = self.api(cx) else {
                    return;
                };
                let tx = self.events.clone();
                let runtime = self.runtime(cx);
                runtime.spawn(async move {
                    let reply = api.genre_model_delete().await;
                    tx.send(Msg::ModelDeleted(Box::new(reply)));
                });
            }
            ModelAction::Relabel => {
                let Some(api) = self.api(cx) else {
                    return;
                };
                let tx = self.events.clone();
                let runtime = self.runtime(cx);
                runtime.spawn(async move {
                    let _ = api.genre_relabel_start().await;
                    let reply = api.genre_relabel().await;
                    tx.send(Msg::Relabel(Box::new(reply)));
                });
                self.watch_relabel(cx);
            }
        }
    }

    fn on_share_action(&mut self, action: ShareAction, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        match action {
            ShareAction::Export => {
                self.share.exporting = true;
                let tx = self.events.clone();
                let runtime = self.runtime(cx);
                runtime.spawn(async move {
                    let reply = api
                        .get::<Value>(Lane::User, "/genre/share/export")
                        .await
                        .map(|fresh| fresh.value)
                        .map_err(|error| error.cause_fr());
                    tx.send(Msg::ShareExported(Box::new(reply)));
                });
            }
            ShareAction::Preview => {
                let path = PathBuf::from(self.share.path.trim());
                match read_share_file(&path) {
                    Ok(value) => {
                        self.share.importing = true;
                        let tx = self.events.clone();
                        let runtime = self.runtime(cx);
                        runtime.spawn(async move {
                            let reply = api.genre_share_preview(&value).await;
                            tx.send(Msg::SharePreview(Box::new(reply)));
                        });
                    }
                    Err(error) => self.share.error = Some(error.into()),
                }
            }
            ShareAction::Head(on) => {
                self.share.head = on;
            }
            ShareAction::Tags(on) => {
                self.share.tags = on;
            }
            ShareAction::Import => {
                let path = PathBuf::from(self.share.path.trim());
                let file = match read_share_file(&path) {
                    Ok(file) => file,
                    Err(error) => {
                        self.share.error = Some(error.into());
                        cx.notify();
                        return;
                    }
                };
                self.share.importing = true;
                let options = serde_json::json!({"head": self.share.head, "tags": self.share.tags});
                let tx = self.events.clone();
                let runtime = self.runtime(cx);
                runtime.spawn(async move {
                    let reply = api.genre_share_import(&file, &options).await;
                    tx.send(Msg::ShareImported(Box::new(reply)));
                });
            }
        }
        cx.notify();
    }

    fn on_explorer_action(&mut self, action: genre_explorer::Action, cx: &mut Context<Self>) {
        match action {
            genre_explorer::Action::Select(index) => {
                self.explorer.selected = index;
                self.explorer.playlists.clear();
                self.explorer.state.clear();
                self.explorer.discovery_error = None;
                cx.notify();
            }
            genre_explorer::Action::Level(level) => {
                self.explorer.level = level;
                cx.notify();
            }
            genre_explorer::Action::Discover => self.discover(cx),
            genre_explorer::Action::Poll => self.poll_discover(cx),
            genre_explorer::Action::Station => self.station(cx),
            genre_explorer::Action::Play(index) => self.play_playlist(index, cx),
            genre_explorer::Action::Reload => {
                self.explorer_loaded = false;
                self.load_explorer(cx);
                cx.notify();
            }
        }
    }

    fn save_pref<T: serde::Serialize>(&self, key: &str, value: &T, cx: &Context<Self>) {
        if let Some(cache) = &self.state.read(cx).cache {
            let _ = cache.kv().set(key, value);
        }
    }
}

/// The Model tab's buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModelAction {
    Mode(training::Mode),
    SelfTrain(bool),
    Train,
    Send,
    Disable,
    Relabel,
}

/// The share card's buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShareAction {
    Export,
    Preview,
    Import,
    Head(bool),
    Tags(bool),
}

/// The share file, parsed: the whole document is somebody else's text and the
/// SERVER validates it (`genreshare.parse`), so the client only refuses what is
/// not JSON before sending it.
fn read_share_file(path: &std::path::Path) -> Result<Value, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("lecture impossible : {error}"))?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|error| format!("fichier illisible : {error}"))?;
    if !value.is_object() {
        return Err("ce fichier n'est pas un modèle de genres".to_owned());
    }
    Ok(value)
}

// -- the page --------------------------------------------------------------

impl Screen {
    fn header(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let title = div()
            .flex()
            .flex_col()
            .gap(rems(0.125))
            .min_w_0()
            .child(
                div()
                    .text_size(rems(1.25))
                    .font_weight(FontWeight::BOLD)
                    .child("Studio de genres"),
            )
            .child(
                div()
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child(
                        "Vos étiquettes, votre arbre et votre modèle — mesuré et \
                         entraîné ici, décidé par le serveur.",
                    ),
            );
        let mut line = div().flex().flex_col().gap(rems(0.5)).child(title);
        if let Phase::Ready(_) = &self.status {
            let mut chips = div().flex().flex_wrap().items_center().gap(rems(0.375));
            chips = chips
                .child(stat_chip(
                    IconName::Tag,
                    format!("{} étiquetés", self.labelled),
                    cx,
                ))
                .child(stat_chip(
                    IconName::Activity,
                    match &self.model {
                        Some(model) => {
                            let labels = model
                                .get("labels")
                                .and_then(|labels| labels.as_array())
                                .map(Vec::len)
                                .unwrap_or(0);
                            format!("modèle · {labels} genres")
                        }
                        None => "aucun modèle".to_owned(),
                    },
                    cx,
                ))
                .child(stat_chip(
                    IconName::Cpu,
                    if self.extractor.available {
                        format!("extracteur · {}D", self.extractor.dim)
                    } else {
                        "extracteur absent".to_owned()
                    },
                    cx,
                ));
            if let Some(note) = &self.note {
                chips = chips.child(
                    div()
                        .text_size(rems(0.6875))
                        .text_color(theme::success(cx))
                        .child(note.clone()),
                );
            }
            if let Some(error) = &self.error {
                chips = chips.child(
                    div()
                        .text_size(rems(0.6875))
                        .text_color(theme::danger(cx))
                        .child(error.clone()),
                );
            }
            line = line.child(chips);
        }
        line.into_any_element()
    }

    fn tabs(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut strip = div()
            .id("genre-tabs")
            .flex()
            .items_center()
            .gap(rems(0.25))
            .border_b_1()
            .border_color(theme::border(cx));
        for tab in Tab::ALL {
            let selected = self.tab == tab;
            let mut button = div()
                .id(format!("genre-tab-{}", tab as usize))
                .flex()
                .items_center()
                .gap(rems(0.375))
                .px(rems(0.625))
                .py(rems(0.375))
                .border_b_1()
                .border_color(if selected {
                    theme::accent(cx)
                } else {
                    theme::border(cx).opacity(0.0)
                })
                .text_size(rems(0.8125))
                .text_color(if selected {
                    theme::text(cx)
                } else {
                    theme::secondary(cx)
                })
                .cursor_pointer()
                .hover(|style| style.text_color(theme::text(cx)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.tab = tab;
                    match tab {
                        Tab::Library => this.load_library(cx),
                        Tab::Tree => this.load_tree(cx),
                        Tab::Explorer => this.load_explorer(cx),
                        Tab::Model | Tab::Tagging => {}
                    }
                    cx.notify();
                }))
                .child(icon(tab.glyph()).size(rems(0.875)))
                .child(tab.label());
            if tab == Tab::Tagging && self.labelled > 0 {
                button = button.child(
                    div()
                        .px(rems(0.25))
                        .text_size(rems(0.625))
                        .font_family(theme::mono(cx))
                        .text_color(theme::secondary(cx))
                        .child(format!("{}", self.labelled)),
                );
            }
            strip = strip.child(button);
        }
        strip.into_any_element()
    }

    fn field(
        &mut self,
        key: Field,
        value: String,
        placeholder: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some(field) = self.fields.get(&key) {
            return field.clone();
        }
        let field = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(value)
                .placeholder(placeholder)
        });
        let subscription = cx.subscribe(&field, move |this, field, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let text = field.read(cx).value().to_string();
                this.on_field_change(key, text, cx);
            }
        });
        self._subscriptions.push(subscription);
        self.fields.insert(key, field.clone());
        field
    }

    fn on_field_change(&mut self, key: Field, text: String, cx: &mut Context<Self>) {
        match key {
            Field::Query => self.query = text,
            Field::TreeDraft => self.tree.draft = text,
            Field::SharePath => self.share.path = text,
            Field::ExtractorPath => self.extractor_card.path = text,
            Field::BulkRef => {
                if let Some(form) = self.bulk_form.as_mut() {
                    form.reference = text;
                }
            }
        }
        cx.notify();
    }

    // -- Étiquetage -------------------------------------------------------

    fn tagging(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if !self.is_admin(cx) {
            return card::empty(
                IconName::Lock,
                "Réservé à l'administrateur",
                "Le studio n'a qu'un modèle : les étiquettes et l'arbre lui appartiennent.",
                cx,
                None,
            )
            .into_any_element();
        }
        if let Phase::Failed(error) = &self.status {
            let cause = error.cause_fr();
            return card::failed("genres-status", cause, cx, |this: &mut Self, _, cx| {
                this.status = Phase::Loading;
                this.load_status(cx);
            })
            .into_any_element();
        }
        let mut column = div()
            .id("genre-tagging")
            .flex()
            .flex_col()
            .gap(rems(0.75))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        column = column.child(self.sort_bar(cx));
        column = column.child(self.candidate(window, cx));
        column = column.child(self.finder(window, cx));
        if self.bulk_open {
            column = column.child(self.bulk_form(window, cx));
        }
        column.into_any_element()
    }

    fn sort_bar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut row = div().flex().items_center().gap(rems(0.375)).child(
            div()
                .text_size(rems(0.6875))
                .text_color(theme::secondary(cx))
                .child("Ordre des titres"),
        );
        for (sort, label, hint) in [
            (
                Sort::Plays,
                "Les plus écoutés",
                "Le modèle s'améliore sur la musique que vous écoutez vraiment.",
            ),
            (
                Sort::Active,
                "Là où le modèle hésite",
                "Il apprend le plus vite sur les titres dont il est le moins sûr.",
            ),
        ] {
            let selected = self.sort == sort;
            row = row.child(
                div()
                    .id(format!("genre-sort-{}", sort as usize))
                    .px(rems(0.5))
                    .py(rems(0.25))
                    .border_1()
                    .border_color(if selected {
                        theme::accent(cx)
                    } else {
                        theme::border(cx)
                    })
                    .bg(if selected {
                        theme::selected(cx)
                    } else {
                        theme::chrome(cx)
                    })
                    .text_size(rems(0.75))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.sort == sort {
                            return;
                        }
                        this.sort = sort;
                        this.load_candidates(cx);
                    }))
                    .child(label),
            );
            let _ = hint;
        }
        row = row.child(div().flex_1()).child(
            div()
                .text_size(rems(0.6875))
                .text_color(theme::secondary(cx))
                .child(match &self.candidates {
                    Phase::Ready(list) => format!(
                        "{} en file · {}/9 au clavier",
                        list.len().saturating_sub(self.cursor),
                        9
                    ),
                    _ => String::new(),
                }),
        );
        row.into_any_element()
    }

    fn candidate(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(candidate) = self.current().cloned() else {
            let (title, body) = match &self.candidates {
                Phase::Loading => ("Chargement…", "Le serveur cherche les titres à étiqueter."),
                Phase::Failed(error) => {
                    let cause: SharedString = error.cause_fr().into();
                    return card::failed(
                        "genre-candidates",
                        cause,
                        cx,
                        |this: &mut Self, _, cx| this.load_candidates(cx),
                    )
                    .into_any_element();
                }
                Phase::Ready(_) => (
                    "Plus rien à étiqueter",
                    "Aucun titre ne porte encore d'empreinte : lancez une mesure dans \
                     l'onglet Bibliothèque.",
                ),
            };
            let mut empty = card::empty(
                IconName::CircleCheck,
                title,
                body,
                cx,
                Some(
                    button(
                        "genre-candidates-reload",
                        "Recharger",
                        ButtonVariant::Outline,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.load_candidates(cx)))
                    .into_any_element(),
                ),
            );
            if let Some(last) = &self.last {
                let name = last.applied.clone();
                empty = empty.child(
                    button(
                        "genre-undo-empty",
                        format!("Annuler « {name} »"),
                        ButtonVariant::Outline,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.undo(cx))),
                );
            }
            return empty.into_any_element();
        };
        let scores = candidate.scores();
        let ranked = genre_label::rank(&self.tags, &scores, &self.counts);
        let scored = ranked.first().and_then(|row| row.score).is_some();
        let picks: Vec<genre_label::Scored> = ranked.iter().take(9).cloned().collect();

        let path = candidate.cover_path();
        let file = cover::file_in(&self.covers, &path);
        {
            let tx = self.events.clone();
            let url = path.clone();
            let state = self.state.clone();
            let mut covers = self.covers.borrow_mut();
            covers.want(state.read(cx), &path, move |file| {
                tx.send(Msg::Cover(url.clone(), file));
            });
        }

        let mut card = div()
            .id("genre-candidate")
            .flex()
            .flex_col()
            .gap(rems(0.75))
            .p(rems(1.))
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx));
        let mut head = div()
            .flex()
            .items_start()
            .gap(rems(0.75))
            .child(cover::view(
                file.as_ref(),
                Some(&path),
                &candidate.title,
                Kind::Album,
                px(96.),
                cx,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(0.25))
                    .flex_1()
                    .min_w_0()
                    .child(genre_label::track_line(
                        &candidate.title,
                        &candidate.artist,
                        &candidate.album,
                        cx,
                    ))
                    .child({
                        let mut meta = div()
                            .flex()
                            .items_center()
                            .gap(rems(0.75))
                            .text_size(rems(0.6875))
                            .text_color(theme::secondary(cx));
                        if candidate.play_count > 0 {
                            meta = meta.child(format!("{} écoutes", candidate.play_count));
                        }
                        if let Some(bpm) = candidate.deezer_bpm {
                            meta = meta.child(format!("BPM Deezer · {bpm:.0}"));
                        }
                        if let Some(uncertainty) = candidate.uncertainty
                            && uncertainty >= 0.6
                        {
                            meta = meta.child("Le modèle hésite : ce titre lui apprendra beaucoup");
                        }
                        meta
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.375))
                    .child(
                        button("genre-listen", "Écouter", ButtonVariant::Outline, cx).on_click(
                            cx.listener(|this, _, _, cx| {
                                let Some(candidate) = this.current().cloned() else {
                                    return;
                                };
                                let id = candidate.reference();
                                let track = Track {
                                    deezer_id: nz_model::TrackId::from(id),
                                    title: candidate.title.clone(),
                                    duration: candidate.duration,
                                    artist: nz_model::TrackArtist {
                                        name: candidate.artist.clone(),
                                        ..Default::default()
                                    },
                                    album: nz_model::TrackAlbum {
                                        title: candidate.album.clone(),
                                        ..Default::default()
                                    },
                                    ..Track::default()
                                };
                                let entry = entry_from_track(&track);
                                this.state.update(cx, |state, cx| {
                                    state.command(
                                        nz_player::Command::play_queue(vec![entry], 0, None),
                                        cx,
                                    )
                                });
                            }),
                        ),
                    )
                    .child(
                        button("genre-sheet", "Étiqueter…", ButtonVariant::Outline, cx).on_click(
                            cx.listener(|this, _, _, cx| {
                                let Some(candidate) = this.current().cloned() else {
                                    return;
                                };
                                this.open_sheet(&candidate, cx);
                            }),
                        ),
                    ),
            );
        head = head.child(div().flex_1());
        card = card.child(head);
        // The nine most likely genres, best first, on the number keys.
        let mut picks_row = div().flex().flex_wrap().gap(rems(0.375));
        for (index, row) in picks.iter().enumerate() {
            let selected = row.score.is_some() && index == 0 && scored;
            let tag = row.tag.clone();
            let trailing = row
                .score
                .map(genre_label::percent)
                .or_else(|| (row.used > 0).then(|| row.used.to_string()));
            picks_row = picks_row.child(
                div()
                    .id(format!("genre-pick-{}", index))
                    .flex()
                    .items_center()
                    .gap(rems(0.375))
                    .px(rems(0.5))
                    .py(rems(0.375))
                    .border_1()
                    .border_color(if selected {
                        genre_label::color(&tag, cx)
                    } else {
                        theme::border(cx)
                    })
                    .bg(theme::sidebar(cx))
                    .text_size(rems(0.8125))
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::selected(cx)))
                    .on_click({
                        let picked = tag.clone();
                        cx.listener(move |this, _, _, cx| {
                            let Some(candidate) = this.current().cloned() else {
                                return;
                            };
                            this.label(candidate, picked.clone(), cx);
                        })
                    })
                    .child(
                        div()
                            .text_size(rems(0.625))
                            .font_family(theme::mono(cx))
                            .text_color(theme::secondary(cx))
                            .child(format!("{}", index + 1)),
                    )
                    .child(genre_label::dot(&tag, px(8.), cx))
                    .child(SharedString::from(tag.name.clone()))
                    .when_some(trailing, |pick, trailing| {
                        pick.child(
                            div()
                                .text_size(rems(0.625))
                                .font_family(theme::mono(cx))
                                .text_color(theme::secondary(cx))
                                .child(trailing),
                        )
                    }),
            );
        }
        card = card.child(picks_row);
        card = card.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.375))
                .child(
                    button("genre-back", "Précédent", ButtonVariant::Outline, cx)
                        .disabled(self.cursor == 0)
                        .on_click(cx.listener(|this, _, _, cx| this.back(cx))),
                )
                .when_some(self.last.clone(), |row, last| {
                    row.child(
                        button(
                            "genre-undo",
                            format!("Annuler « {} »", last.applied),
                            ButtonVariant::Outline,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.undo(cx))),
                    )
                })
                .child(div().flex_1())
                .child(
                    button(
                        "genre-bulk-toggle",
                        if self.bulk_open {
                            "Fermer le lot"
                        } else {
                            "Étiqueter en lot…"
                        },
                        ButtonVariant::Outline,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.bulk_open = !this.bulk_open;
                        cx.notify();
                    })),
                )
                .child(
                    button("genre-skip", "Passer", ButtonVariant::Outline, cx)
                        .on_click(cx.listener(|this, _, _, cx| this.skip(cx))),
                )
                .when(scored, |row| {
                    let name = picks
                        .first()
                        .map(|row| row.tag.name.clone())
                        .unwrap_or_else(|| "Confirmer".to_owned());
                    row.child(
                        button(
                            "genre-confirm",
                            format!("Confirmer {name}"),
                            ButtonVariant::Primary,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.confirm(cx))),
                    )
                }),
        );
        card = card.child(
            div()
                .text_size(rems(0.625))
                .text_color(theme::secondary(cx))
                .child(
                    "1…9 choisir · Entrée le premier · j / k avancer / reculer · z annuler · \
                     / chercher · le champ de recherche se rejoint avec Tab",
                ),
        );
        card.into_any_element()
    }

    fn finder(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(candidate) = self.current().cloned() else {
            return div().into_any_element();
        };
        let ranked = genre_label::rank(&self.tags, &candidate.scores(), &self.counts);
        let found: Vec<genre_label::Scored> = if self.query.trim().is_empty() {
            Vec::new()
        } else {
            genre_label::search(&ranked, &self.query)
                .into_iter()
                .take(30)
                .cloned()
                .collect()
        };
        let mut area = div().flex().flex_col().gap(rems(0.375));
        // The query field lives in the screen's fields map, so its draft survives
        // a repaint. It is created on first paint (a window is needed) and the
        // subscription keeps `self.query` in step.
        let field = self.field(
            Field::Query,
            self.query.clone(),
            "Chercher un genre…",
            window,
            cx,
        );
        area = area.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .child(
                    div()
                        .flex_1()
                        .child(input("genre-query", &field, window, cx)),
                )
                .child(
                    button(
                        "genre-all",
                        if self.show_all {
                            "Réduire"
                        } else {
                            "Tous les genres"
                        },
                        ButtonVariant::Outline,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_all = !this.show_all;
                        cx.notify();
                    })),
                ),
        );
        let found_any = !found.is_empty();
        if found_any {
            let mut chips = div().flex().flex_wrap().gap(rems(0.375));
            for row in found {
                let tag = row.tag.clone();
                chips = chips.child(
                    genre_label::chip(
                        gpui_kit::ElementId::named_usize("genre-found", row.tag.id.max(0) as usize),
                        &row.tag,
                        row.score.map(genre_label::percent),
                        false,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let Some(candidate) = this.current().cloned() else {
                            return;
                        };
                        this.label(candidate, tag.clone(), cx);
                    })),
                );
            }
            area = area.child(chips);
        } else if self.show_all {
            let mut chips = div().flex().flex_wrap().gap(rems(0.375));
            for row in ranked.iter().skip(9) {
                let tag = row.tag.clone();
                chips = chips.child(
                    genre_label::chip(
                        gpui_kit::ElementId::named_usize(
                            "genre-all-tag",
                            row.tag.id.max(0) as usize,
                        ),
                        &row.tag,
                        Some(row.used.to_string()),
                        false,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let Some(candidate) = this.current().cloned() else {
                            return;
                        };
                        this.label(candidate, tag.clone(), cx);
                    })),
                );
            }
            area = area.child(chips);
        }
        if !self.query.trim().is_empty() && !found_any {
            area = area.child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .child("Aucun genre ne correspond — créez-le dans l'onglet Arbre."),
            );
        }
        area.into_any_element()
    }
}

/// A small icon + value chip in the header.
fn stat_chip(glyph: IconName, text: String, cx: &App) -> gpui_kit::Div {
    div()
        .flex()
        .items_center()
        .gap(rems(0.25))
        .px(rems(0.5))
        .py(rems(0.25))
        .border_1()
        .border_color(theme::border(cx))
        .text_size(rems(0.6875))
        .text_color(theme::secondary(cx))
        .child(icon(glyph).size(rems(0.75)))
        .child(SharedString::from(text))
}

impl Screen {
    // -- the bulk form ----------------------------------------------------

    fn bulk_form(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let field = self.field(
            Field::BulkRef,
            self.bulk_form
                .as_ref()
                .map(|form| form.reference.clone())
                .unwrap_or_default(),
            "Référence de l'album / de la playlist / de l'artiste",
            window,
            cx,
        );
        let kind = self
            .bulk_form
            .as_ref()
            .map_or(ScopeKind::Album, |form| form.kind);
        let error = self.bulk_form.as_ref().and_then(|form| form.error.clone());
        let busy = self.bulk_form.as_ref().is_some_and(|form| form.busy);
        let ranked = genre_label::rank(&self.tags, &[], &self.counts);
        let picks: Vec<genre_label::Scored> = if self.show_all {
            ranked.into_iter().take(24).collect()
        } else {
            ranked.into_iter().take(12).collect()
        };
        let mut card = div()
            .id("genre-bulk-form")
            .flex()
            .flex_col()
            .gap(rems(0.5))
            .p(rems(0.75))
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx))
            .child(
                div()
                    .text_size(rems(0.8125))
                    .font_weight(FontWeight::BOLD)
                    .child("Étiqueter en lot"),
            )
            .child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .child(
                        "Un album, une playlist ou un artiste entier : le serveur compare \
                         chaque titre au genre avant de l'appliquer, et les désaccords \
                         arrivent décochés.",
                    ),
            );
        let mut kinds = div().flex().items_center().gap(rems(0.25));
        for scope_kind in [ScopeKind::Album, ScopeKind::Playlist, ScopeKind::Artist] {
            let selected = kind == scope_kind;
            kinds = kinds.child(
                div()
                    .id(format!("genre-bulk-kind-{}", scope_kind as usize))
                    .px(rems(0.5))
                    .py(rems(0.25))
                    .border_1()
                    .border_color(if selected {
                        theme::accent(cx)
                    } else {
                        theme::border(cx)
                    })
                    .bg(if selected {
                        theme::selected(cx)
                    } else {
                        theme::chrome(cx)
                    })
                    .text_size(rems(0.75))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(form) = this.bulk_form.as_mut() {
                            form.kind = scope_kind;
                        } else {
                            this.bulk_form = Some(BulkForm {
                                kind: scope_kind,
                                ..BulkForm::default()
                            });
                        }
                        cx.notify();
                    }))
                    .child(scope_kind.label()),
            );
        }
        card = card.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .child(kinds)
                .child(
                    div()
                        .flex_1()
                        .child(input("genre-bulk-ref", &field, window, cx)),
                ),
        );
        if let Some(error) = error {
            card = card.child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::danger(cx))
                    .child(error),
            );
        }
        card = card.child(
            div()
                .text_size(rems(0.625))
                .text_color(theme::secondary(cx))
                .child("Genre à appliquer (le clic ouvre l'aperçu) :"),
        );
        let mut chips = div().flex().flex_wrap().gap(rems(0.375));
        for row in picks {
            let tag = row.tag.clone();
            let form = self.bulk_form.clone().unwrap_or_default();
            let usable = !form.reference.trim().is_empty() && !busy;
            chips = chips.child(
                genre_label::chip(
                    gpui_kit::ElementId::named_usize("genre-bulk-tag", row.tag.id.max(0) as usize),
                    &row.tag,
                    Some(row.used.to_string()),
                    self.bulk_tag
                        .as_ref()
                        .is_some_and(|chosen| chosen.id == row.tag.id),
                    cx,
                )
                .when(usable, |chip| {
                    chip.on_click(cx.listener(move |this, _, _, cx| {
                        this.bulk_tag = Some(tag.clone());
                        this.open_bulk(
                            BulkForm {
                                reference: this
                                    .bulk_form
                                    .as_ref()
                                    .map(|form| form.reference.clone())
                                    .unwrap_or_default(),
                                ..this.bulk_form.clone().unwrap_or_default()
                            },
                            cx,
                        );
                    }))
                }),
            );
        }
        card = card.child(chips);
        if self.bulk_tag.is_none() {
            card = card.child(
                div()
                    .text_size(rems(0.625))
                    .text_color(theme::secondary(cx))
                    .child("Aucun genre choisi pour l'instant."),
            );
        }
        card.into_any_element()
    }

    // -- Bibliothèque -----------------------------------------------------

    fn library(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let mut column = div()
            .id("genre-library")
            .flex()
            .flex_col()
            .gap(rems(0.75))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        column = column.child(self.plan_card(cx));
        column = column.child(self.backfill_card(cx));
        column = column.child(self.embed_card(cx));
        column = column.child(self.where_card(cx));
        column = column.child(self.local_run_card(cx));
        column = column.child(self.schedule_card(cx));
        column = column.child(self.explorer_admin_card(cx));
        column = column.child(self.extractor_card(window, cx));
        column.into_any_element()
    }

    fn plan_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut card = div()
            .id("genre-plan")
            .flex()
            .flex_col()
            .gap(rems(0.5))
            .p(rems(0.75))
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(
                        div()
                            .text_size(rems(0.8125))
                            .font_weight(FontWeight::BOLD)
                            .child("État de la bibliothèque"),
                    )
                    .child(div().flex_1())
                    .child(
                        button("genre-plan-reload", "Recharger", ButtonVariant::Outline, cx)
                            .on_click(cx.listener(|this, _, _, cx| this.load_library(cx))),
                    ),
            );
        match &self.plan {
            Phase::Loading => {
                card = card.child(
                    div()
                        .text_size(rems(0.75))
                        .text_color(theme::secondary(cx))
                        .child("Lecture du plan…"),
                );
            }
            Phase::Failed(error) => {
                card = card.child(
                    div()
                        .text_size(rems(0.75))
                        .text_color(theme::danger(cx))
                        .child(error.cause_fr()),
                );
            }
            Phase::Ready(plan) => {
                card = card.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.75))
                        .text_size(rems(0.75))
                        .text_color(theme::secondary(cx))
                        .child(format!("{} titres", plan.total))
                        .child(format!("{} à jour", plan.current))
                        .child(format!("{} jamais mesurés", plan.unmeasured))
                        .child(format!(
                            "qui mesure : {}",
                            match plan.r#where.as_str() {
                                "client" => "cet ordinateur",
                                "off" => "personne",
                                _ => "le serveur",
                            }
                        )),
                );
                // One row per stage: what is behind, and by how much.
                for (name, hint, late, off) in [
                    (
                        "Mesure",
                        "tempo, style",
                        plan.unmeasured + plan.stale_measure,
                        false,
                    ),
                    (
                        "Empreinte",
                        if plan.extractor {
                            "ce que le modèle écoute"
                        } else {
                            "extracteur absent"
                        },
                        if plan.extractor {
                            plan.unmeasured + plan.stale_embed
                        } else {
                            0
                        },
                        !plan.extractor,
                    ),
                    (
                        "Classement",
                        "genre retenu",
                        plan.unmeasured + plan.stale_decide,
                        false,
                    ),
                ] {
                    let share = if plan.total > 0 {
                        (late as f64 / plan.total as f64).clamp(0., 1.)
                    } else {
                        0.
                    };
                    card =
                        card.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(rems(0.5))
                                .text_size(rems(0.6875))
                                .text_color(if off {
                                    theme::secondary(cx)
                                } else {
                                    theme::text(cx)
                                })
                                .child(div().w(rems(5.5)).child(name))
                                .child(
                                    div().flex_1().h(rems(0.375)).bg(theme::sidebar(cx)).child(
                                        div().h_full().w(gpui_kit::relative(share as f32)).bg(
                                            if off {
                                                theme::secondary(cx)
                                            } else {
                                                theme::accent(cx)
                                            },
                                        ),
                                    ),
                                )
                                .child(
                                    div()
                                        .w(rems(6.))
                                        .text_color(theme::secondary(cx))
                                        .child(hint),
                                )
                                .child(div().w(rems(3.)).font_family(theme::mono(cx)).child(
                                    if off {
                                        "—".to_owned()
                                    } else {
                                        late.to_string()
                                    },
                                )),
                        );
                }
            }
        }
        card.into_any_element()
    }

    fn backfill_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let running = matches!(&self.backfill, Phase::Ready(job) if job.job.running);
        let mut card = div()
            .id("genre-backfill")
            .flex()
            .flex_col()
            .gap(rems(0.5))
            .p(rems(0.75))
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(
                        div()
                            .text_size(rems(0.8125))
                            .font_weight(FontWeight::BOLD)
                            .child("Mesurer la bibliothèque (serveur)"),
                    )
                    .child(div().flex_1())
                    .child(
                        button(
                            "genre-backfill-force",
                            if self.force {
                                "Reclasser tout : oui"
                            } else {
                                "Reclasser tout : non"
                            },
                            ButtonVariant::Outline,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.force = !this.force;
                            cx.notify();
                        })),
                    )
                    .when(running, |row| {
                        row.child(
                            button("genre-backfill-stop", "Arrêter", ButtonVariant::Danger, cx)
                                .on_click(cx.listener(|this, _, _, cx| this.stop_backfill(cx))),
                        )
                    })
                    .when(!running, |row| {
                        row.child(
                            button(
                                "genre-backfill-start",
                                "Mesurer",
                                ButtonVariant::Primary,
                                cx,
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.start_backfill(cx))),
                        )
                    }),
            )
            .child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .child(
                        "Le serveur mesure les fichiers (ffmpeg), à son rythme : il cède dès \
                         que quelqu'un écoute ou télécharge. Reclasser tout re-mesure ce qui \
                         est déjà à jour.",
                    ),
            );
        if let Phase::Ready(job) = &self.backfill {
            card = card.child(job_lines(&job.job, job.stage.as_deref(), cx));
        }
        card.into_any_element()
    }

    fn embed_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let running = matches!(&self.embed, Phase::Ready(job) if job.job.running);
        let mut card = div()
            .id("genre-embed")
            .flex()
            .flex_col()
            .gap(rems(0.5))
            .p(rems(0.75))
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(
                        div()
                            .text_size(rems(0.8125))
                            .font_weight(FontWeight::BOLD)
                            .child("Calculer les empreintes"),
                    )
                    .child(div().flex_1())
                    .when(running, |row| {
                        row.child(
                            div()
                                .text_size(rems(0.6875))
                                .text_color(theme::secondary(cx))
                                .child("en cours"),
                        )
                    })
                    .when(!running, |row| {
                        row.child(
                            button("genre-embed-start", "Calculer", ButtonVariant::Outline, cx)
                                .on_click(cx.listener(|this, _, _, cx| this.start_embed(cx))),
                        )
                    }),
            )
            .child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .child(
                        "Chaque titre archivé sans vecteur est décodé une fois par \
                         l'extracteur. Sans extracteur, rien à calculer : la mesure et le \
                         classement du serveur continuent.",
                    ),
            );
        if let Phase::Ready(job) = &self.embed {
            card = card.child(job_lines(&job.job, None, cx));
        }
        card.into_any_element()
    }

    fn where_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut row = div()
            .id("genre-where")
            .flex()
            .items_center()
            .gap(rems(0.5))
            .p(rems(0.75))
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx))
            .child(
                div()
                    .text_size(rems(0.8125))
                    .font_weight(FontWeight::BOLD)
                    .child("Où mesurer"),
            )
            .child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .child("Le serveur (ffmpeg) ou cet ordinateur (le cœur du studio)."),
            )
            .child(div().flex_1());
        for (place, label) in [("server", "Serveur"), ("client", "Cet ordinateur")] {
            let selected = self.where_choice == place;
            row = row.child(
                div()
                    .id(format!("genre-where-choice-{}", place))
                    .px(rems(0.625))
                    .py(rems(0.25))
                    .border_1()
                    .border_color(if selected {
                        theme::accent(cx)
                    } else {
                        theme::border(cx)
                    })
                    .bg(if selected {
                        theme::selected(cx)
                    } else {
                        theme::shell(cx)
                    })
                    .text_size(rems(0.75))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.set_where(place, cx)))
                    .child(label),
            );
        }
        row.into_any_element()
    }

    fn local_run_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut card = div()
            .id("genre-local-run")
            .flex()
            .flex_col()
            .gap(rems(0.5))
            .p(rems(0.75))
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(
                        div()
                            .text_size(rems(0.8125))
                            .font_weight(FontWeight::BOLD)
                            .child("Mesurer sur cet ordinateur"),
                    )
                    .child(div().flex_1())
                    .when(self.run.running, |row| {
                        row.child(
                            button("genre-run-stop", "Arrêter", ButtonVariant::Danger, cx)
                                .on_click(cx.listener(|this, _, _, cx| this.stop_analysis_run(cx))),
                        )
                    })
                    .when(!self.run.running, |row| {
                        row.child(
                            button("genre-run-start", "Mesurer ici", ButtonVariant::Primary, cx)
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.start_analysis_run(cx)),
                                ),
                        )
                    }),
            )
            .child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .child(
                        "Le même fichier est décodé et mesuré par les cœurs du studio \
                         (spectral, sonie, tempo), puis seuls les nombres partent au serveur : \
                         il décide du genre. Le travail cède la place dès qu'un titre démarre.",
                    ),
            );
        if self.run.running || self.run.done + self.run.failed > 0 {
            let total = self.run.total.max(1);
            let share = ((self.run.done + self.run.failed) as f64 / total as f64).clamp(0., 1.);
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .text_size(rems(0.6875))
                    .child(
                        div().flex_1().h(rems(0.375)).bg(theme::sidebar(cx)).child(
                            div()
                                .h_full()
                                .w(gpui_kit::relative(share as f32))
                                .bg(theme::accent(cx)),
                        ),
                    )
                    .child(div().font_family(theme::mono(cx)).child(format!(
                        "{}/{} · {} échec(s)",
                        self.run.done + self.run.failed,
                        self.run.total,
                        self.run.failed
                    )))
                    .child(
                        div()
                            .max_w(rems(20.))
                            .text_color(theme::secondary(cx))
                            .child(SharedString::from(self.run.current.clone())),
                    ),
            );
            for (cause, count) in &self.run.causes {
                card = card.child(
                    div()
                        .text_size(rems(0.6875))
                        .text_color(theme::danger(cx))
                        .child(format!("{count} × {cause}")),
                );
            }
            if let Some(stopped) = &self.run.stopped {
                card = card.child(
                    div()
                        .text_size(rems(0.6875))
                        .text_color(theme::warning(cx))
                        .child(SharedString::from(stopped.clone())),
                );
            }
        }
        card.into_any_element()
    }

    fn schedule_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let schedule = match &self.plan {
            Phase::Ready(plan) => Some(plan.schedule.clone()),
            _ => None,
        };
        let mut card = div()
            .id("genre-schedule")
            .flex()
            .items_center()
            .gap(rems(0.5))
            .p(rems(0.75))
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx))
            .child(
                div()
                    .text_size(rems(0.8125))
                    .font_weight(FontWeight::BOLD)
                    .child("Fenêtre de nuit"),
            )
            .child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .child(match &schedule {
                        Some(schedule) if schedule.enabled => {
                            format!("chaque nuit à {} pendant {} h", schedule.at, schedule.hours)
                        }
                        Some(_) => "désactivée".to_owned(),
                        None => "inconnue".to_owned(),
                    }),
            )
            .child(div().flex_1());
        let enabled = schedule.as_ref().is_some_and(|schedule| schedule.enabled);
        card = card
            .when(!enabled, |row| {
                row.child(
                    button(
                        "genre-schedule-on",
                        "03:00 → 09:00",
                        ButtonVariant::Outline,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.set_schedule(true, cx))),
                )
            })
            .when(enabled, |row| {
                row.child(
                    button(
                        "genre-schedule-off",
                        "Désactiver",
                        ButtonVariant::Outline,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.set_schedule(false, cx))),
                )
            });
        card.into_any_element()
    }

    fn explorer_admin_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut card = div()
            .id("genre-explorer-admin")
            .flex()
            .flex_col()
            .gap(rems(0.5))
            .p(rems(0.75))
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx))
            .child(
                div()
                    .text_size(rems(0.8125))
                    .font_weight(FontWeight::BOLD)
                    .child("Explorateur de fond"),
            )
            .child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .child(
                        "La découverte de fond écoute des extraits Deezer à son rythme pour \
                         ramener des styles que le serveur ne connaît pas encore ; elle cède \
                         à la lecture et aux téléchargements.",
                    ),
            );
        match &self.explorer_admin {
            Phase::Loading => {
                card = card.child(
                    div()
                        .text_size(rems(0.6875))
                        .text_color(theme::secondary(cx))
                        .child("Chargement…"),
                );
            }
            Phase::Failed(error) => {
                card = card.child(
                    div()
                        .text_size(rems(0.6875))
                        .text_color(theme::danger(cx))
                        .child(error.cause_fr()),
                );
            }
            Phase::Ready(explorer) => {
                let enabled = explorer.enabled;
                let archive = explorer.archive;
                card = card
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(0.5))
                            .child(
                                gpui_omarchy::switch(
                                    "genre-explorer-enabled",
                                    "Explorateur",
                                    enabled,
                                    cx,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        let (enabled, archive) = match &this.explorer_admin {
                                            Phase::Ready(explorer) => {
                                                (explorer.enabled, explorer.archive)
                                            }
                                            _ => return,
                                        };
                                        this.set_explorer_switches(!enabled, archive, cx);
                                    },
                                )),
                            )
                            .child(
                                gpui_omarchy::switch(
                                    "genre-explorer-archive",
                                    "Archiver ce qu'il trouve",
                                    archive,
                                    cx,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        let (enabled, archive) = match &this.explorer_admin {
                                            Phase::Ready(explorer) => {
                                                (explorer.enabled, explorer.archive)
                                            }
                                            _ => return,
                                        };
                                        this.set_explorer_switches(enabled, !archive, cx);
                                    },
                                )),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_size(rems(0.6875))
                                    .font_family(theme::mono(cx))
                                    .text_color(theme::secondary(cx))
                                    .child(format!(
                                        "réserve {} · rejetés {}",
                                        explorer.pool_now, explorer.rejected_total
                                    )),
                            ),
                    )
                    .when(explorer.running, |row| {
                        row.child(
                            div()
                                .text_size(rems(0.6875))
                                .text_color(theme::secondary(cx))
                                .child("en cours"),
                        )
                    })
                    .when_some(explorer.last_error.clone(), |row, error| {
                        row.child(
                            div()
                                .text_size(rems(0.6875))
                                .text_color(theme::warning(cx))
                                .child(SharedString::from(error)),
                        )
                    });
            }
        }
        card.into_any_element()
    }

    fn extractor_card(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let field = self.field(
            Field::ExtractorPath,
            self.extractor_card.path.clone(),
            "/chemin/vers/discogs-effnet-bsdynamic-1.onnx",
            window,
            cx,
        );
        let info = self.extractor.clone();
        let field_element = input("genre-extractor-path", &field, window, cx).into_any_element();
        genre_extractor::view(
            &self.extractor_card,
            &info,
            cx,
            |this: &mut Self, action, cx| this.on_extractor_action(action, cx),
            Some(field_element),
        )
    }

    // -- Arbre ------------------------------------------------------------

    fn tree_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let field = self.field(
            Field::TreeDraft,
            self.tree.draft.clone(),
            "Nom du genre",
            window,
            cx,
        );
        let draft = (self.tree.editor.is_some())
            .then(|| input("genre-tree-draft", &field, window, cx).into_any_element());
        let mut column = div()
            .id("genre-tree-tab")
            .flex()
            .flex_col()
            .gap(rems(0.5))
            .flex_1()
            .min_h_0();
        if !self.tree_loaded {
            column = column.child(
                div()
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child("Chargement de l'arbre…"),
            );
        }
        column = column.child(genre_tree::view(
            &self.tree,
            draft,
            cx,
            |this: &mut Self, action, cx| this.on_tree_action(action, cx),
        ));
        column.into_any_element()
    }

    // -- Modèle -----------------------------------------------------------

    fn model_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let mut column = div()
            .id("genre-model-tab")
            .flex()
            .flex_col()
            .gap(rems(0.75))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        column = column.child(self.model_state_card(cx));
        column = column.child(self.readiness_card(cx));
        column = column.child(self.training_card(cx));
        column = column.child(self.share_card(window, cx));
        column.into_any_element()
    }

    fn model_state_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let model = self.model.clone();
        let mut card = div()
            .id("genre-model-state")
            .flex()
            .flex_col()
            .gap(rems(0.5))
            .p(rems(0.75))
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(
                        div()
                            .text_size(rems(0.8125))
                            .font_weight(FontWeight::BOLD)
                            .child("Modèle actif"),
                    )
                    .child(div().flex_1())
                    .when(model.is_some(), |row| {
                        row.child(
                            button("genre-model-off", "Désactiver", ButtonVariant::Danger, cx)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.on_model_action(ModelAction::Disable, cx)
                                })),
                        )
                    }),
            );
        match &model {
            None => {
                card = card.child(
                    div()
                        .text_size(rems(0.75))
                        .text_color(theme::secondary(cx))
                        .child(
                            "Aucun modèle : le serveur classe avec ses règles et vos \
                             étiquettes. Entraînez-en un ci-dessous.",
                        ),
                );
            }
            Some(model) => {
                let labels = model
                    .get("labels")
                    .and_then(|labels| labels.as_array())
                    .map(Vec::len)
                    .unwrap_or(0);
                let version = model.get("version").and_then(|version| version.as_i64());
                let usable = model.get("usable").and_then(|usable| usable.as_bool());
                let created = model
                    .get("created")
                    .and_then(|created| created.as_str())
                    .unwrap_or_default();
                card = card.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.75))
                        .text_size(rems(0.75))
                        .text_color(theme::secondary(cx))
                        .child(format!("{labels} genres"))
                        .child(match version {
                            Some(version) => format!("version {version}"),
                            None => "version —".to_owned(),
                        })
                        .child(if usable.unwrap_or(false) {
                            "lisible par le serveur"
                        } else {
                            "NON lisible : le serveur l'ignore"
                        })
                        .child(SharedString::from(created.to_owned())),
                );
            }
        }
        if let Some(relabel) = &self.training.relabel {
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .text_size(rems(0.6875))
                    .text_color(if relabel.running {
                        theme::secondary(cx)
                    } else {
                        theme::success(cx)
                    })
                    .child(if relabel.running {
                        format!(
                            "La bibliothèque se reclasse : {}/{} titres",
                            relabel.scanned, relabel.total
                        )
                    } else {
                        format!(
                            "Dernier reclassement : {} titres vus, {} verdicts changés",
                            relabel.scanned, relabel.changed
                        )
                    })
                    .when(!relabel.running, |row| {
                        row.child(
                            button(
                                "genre-relabel",
                                "Reclasser maintenant",
                                ButtonVariant::Outline,
                                cx,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.on_model_action(ModelAction::Relabel, cx)
                            })),
                        )
                    }),
            );
        }
        card.into_any_element()
    }

    fn readiness_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut rows: Vec<(String, u32)> = self.counts.clone().into_iter().collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let eligible = rows
            .iter()
            .filter(|(_, count)| *count >= training::MIN_EXAMPLES)
            .count();
        let mut card = div()
            .id("genre-readiness")
            .flex()
            .flex_col()
            .gap(rems(0.375))
            .p(rems(0.75))
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(
                        div()
                            .text_size(rems(0.8125))
                            .font_weight(FontWeight::BOLD)
                            .child("Genres prêts"),
                    )
                    .child(
                        div()
                            .text_size(rems(0.6875))
                            .text_color(theme::secondary(cx))
                            .child(format!(
                                "{eligible} genre(s) sur {}+ · {} titres étiquetés",
                                training::MIN_EXAMPLES,
                                self.labelled
                            )),
                    ),
            );
        if rows.is_empty() {
            card = card.child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .child("Étiquetez quelques titres pour entraîner un modèle."),
            );
        }
        for (name, count) in rows.iter().take(10) {
            let short = *count < training::MIN_EXAMPLES;
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .text_size(rems(0.6875))
                    .child(
                        div()
                            .w(rems(9.))
                            .text_color(if short {
                                theme::secondary(cx)
                            } else {
                                theme::text(cx)
                            })
                            .child(SharedString::from(name.clone())),
                    )
                    .child(
                        div().flex_1().h(rems(0.3125)).bg(theme::sidebar(cx)).child(
                            div()
                                .h_full()
                                .w(gpui_kit::relative((*count as f32 / 20.).clamp(0., 1.)))
                                .bg(if short {
                                    theme::secondary(cx)
                                } else {
                                    theme::accent(cx)
                                }),
                        ),
                    )
                    .child(
                        div()
                            .w(rems(2.))
                            .font_family(theme::mono(cx))
                            .text_color(theme::secondary(cx))
                            .child(count.to_string()),
                    ),
            );
        }
        card.into_any_element()
    }

    fn training_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut card = div()
            .id("genre-training")
            .flex()
            .flex_col()
            .gap(rems(0.5))
            .p(rems(0.75))
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(
                        div()
                            .text_size(rems(0.8125))
                            .font_weight(FontWeight::BOLD)
                            .child("Entraînement"),
                    )
                    .child(div().flex_1())
                    .child(
                        button(
                            "genre-train",
                            if self.training.running {
                                "Entraînement…"
                            } else {
                                "Entraîner"
                            },
                            ButtonVariant::Primary,
                            cx,
                        )
                        .disabled(self.training.running)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.on_model_action(ModelAction::Train, cx)
                        })),
                    ),
            );
        // The modes: Auto decides the capacity from the data.
        let mut modes = div().flex().flex_wrap().items_center().gap(rems(0.375));
        for mode in [
            training::Mode::Auto,
            training::Mode::Linear,
            training::Mode::Deep,
            training::Mode::Deep2,
        ] {
            let selected = self.training.mode == mode;
            modes = modes.child(
                div()
                    .id(format!("genre-mode-{}", mode as usize))
                    .px(rems(0.5))
                    .py(rems(0.25))
                    .border_1()
                    .border_color(if selected {
                        theme::accent(cx)
                    } else {
                        theme::border(cx)
                    })
                    .bg(if selected {
                        theme::selected(cx)
                    } else {
                        theme::shell(cx)
                    })
                    .text_size(rems(0.75))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.on_model_action(ModelAction::Mode(mode), cx)
                    }))
                    .child(mode.label()),
            );
        }
        card = card.child(modes);
        card = card.child(
            gpui_omarchy::switch(
                "genre-selftrain",
                "Apprendre aussi des titres non étiquetés",
                self.training.self_train,
                cx,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                let on = !this.training.self_train;
                this.on_model_action(ModelAction::SelfTrain(on), cx);
            })),
        );
        card = card.child(
            div()
                .text_size(rems(0.625))
                .text_color(theme::secondary(cx))
                .child(
                    "Le modèle s'entraîne ici, sur le cœur du studio ; les vecteurs restent \
                     au serveur. « Automatique » garde le réseau seulement s'il fait mieux \
                     sur des titres qu'il n'a pas vus.",
                ),
        );
        if self.training.running || self.training.fraction > 0. {
            let share = self.training.fraction.clamp(0., 1.);
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .text_size(rems(0.6875))
                    .child(
                        div().flex_1().h(rems(0.375)).bg(theme::sidebar(cx)).child(
                            div()
                                .h_full()
                                .w(gpui_kit::relative(share as f32))
                                .bg(theme::accent(cx)),
                        ),
                    )
                    .child(format!("{} · {:.0} %", self.training.stage, share * 100.)),
            );
        }
        if let Some(error) = &self.training.error {
            card = card.child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::danger(cx))
                    .child(error.clone()),
            );
        }
        if let Some(head) = &self.training.head {
            let level = head.health.level;
            let (level_text, colour) = match level {
                "good" => ("Bon modèle", theme::success(cx)),
                "watch" => ("À surveiller", theme::warning(cx)),
                _ => ("Peu fiable", theme::danger(cx)),
            };
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(
                        div()
                            .text_size(rems(0.8125))
                            .font_weight(FontWeight::BOLD)
                            .text_color(colour)
                            .child(level_text),
                    )
                    .child(
                        div()
                            .text_size(rems(0.6875))
                            .text_color(theme::secondary(cx))
                            .child(format!(
                                "{} · {} genres · {} dimensions · {}",
                                match head.kind {
                                    "linear" => "tête linéaire",
                                    "mlp" => "réseau 1 couche",
                                    _ => "réseau 2 couches",
                                },
                                head.labels.len(),
                                head.dim,
                                if head.metrics["grouped"].as_bool().unwrap_or(false) {
                                    "artistes tenus à l'écart"
                                } else {
                                    "plis ordinaires"
                                }
                            )),
                    )
                    .child(div().flex_1())
                    .when(!self.training.sent, |row| {
                        row.child(
                            button(
                                "genre-head-send",
                                "Envoyer au serveur",
                                ButtonVariant::Primary,
                                cx,
                            )
                            .disabled(self.training.sending)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.on_model_action(ModelAction::Send, cx)
                            })),
                        )
                    })
                    .when(self.training.sent, |row| {
                        row.child(
                            div()
                                .text_size(rems(0.6875))
                                .text_color(theme::success(cx))
                                .child("envoyé"),
                        )
                    }),
            );
            let balanced = head.metrics["balanced"].as_f64().unwrap_or(0.);
            let accuracy = head.metrics["accuracy"].as_f64().unwrap_or(0.);
            let fit = head.metrics["fitBalanced"].as_f64().unwrap_or(0.);
            let temperature = head.metrics["temperature"].as_f64().unwrap_or(1.);
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.75))
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .child(format!("précision équilibrée {:.0} %", balanced * 100.))
                    .child(format!("justesse {:.0} %", accuracy * 100.))
                    .child(format!("appris {:.0} %", fit * 100.))
                    .child(format!("température {temperature:.2}")),
            );
            for remark in &head.health.remarks {
                card = card.child(
                    div()
                        .text_size(rems(0.6875))
                        .text_color(theme::secondary(cx))
                        .child(SharedString::from(remark.clone())),
                );
            }
        }
        card.into_any_element()
    }

    fn share_card(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let field = self.field(
            Field::SharePath,
            self.share.path.clone(),
            "/chemin/vers/modele-genres.nsgenre.json",
            window,
            cx,
        );
        let mut card = div()
            .id("genre-share")
            .flex()
            .flex_col()
            .gap(rems(0.5))
            .p(rems(0.75))
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(
                        div()
                            .text_size(rems(0.8125))
                            .font_weight(FontWeight::BOLD)
                            .child("Partager le modèle"),
                    )
                    .child(div().flex_1())
                    .child(
                        button(
                            "genre-share-export",
                            if self.share.exporting {
                                "Export…"
                            } else {
                                "Exporter"
                            },
                            ButtonVariant::Outline,
                            cx,
                        )
                        .disabled(self.share.exporting)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.on_share_action(ShareAction::Export, cx)
                        })),
                    ),
            )
            .child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .child(
                        "Un seul fichier : les genres, chaque étiquette (titre Deezer, YouTube \
                         ou fichier local) et la tête active. Les étiquettes survivent à tout \
                         extracteur ; la tête ne sert que là où les vecteurs vivent dans le \
                         même espace.",
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(
                        div()
                            .flex_1()
                            .child(input("genre-share-path", &field, window, cx)),
                    )
                    .child(
                        button("genre-share-preview", "Lire", ButtonVariant::Outline, cx)
                            .disabled(self.share.importing)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.on_share_action(ShareAction::Preview, cx)
                            })),
                    ),
            );
        if let Some(error) = &self.share.error {
            card = card.child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::danger(cx))
                    .child(error.clone()),
            );
        }
        if let Some(note) = &self.share.note {
            card = card.child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::success(cx))
                    .child(note.clone()),
            );
        }
        if let Some(preview) = &self.share.preview {
            let head = preview.get("head").cloned().unwrap_or(Value::Null);
            let genres = preview
                .get("genres")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            let tags = preview
                .get("tags")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            card = card
                .child(div().text_size(rems(0.75)).child(format!(
                    "Ce fichier porte {genres} genre(s), {tags} étiquette(s){}",
                    if head.is_null() {
                        ", sans tête"
                    } else {
                        ", avec une tête"
                    }
                )))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.5))
                        .child(
                            gpui_omarchy::switch(
                                "genre-share-head",
                                "Importer la tête",
                                self.share.head,
                                cx,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                let on = !this.share.head;
                                this.on_share_action(ShareAction::Head(on), cx);
                            })),
                        )
                        .child(
                            gpui_omarchy::switch(
                                "genre-share-tags",
                                "Importer les étiquettes",
                                self.share.tags,
                                cx,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                let on = !this.share.tags;
                                this.on_share_action(ShareAction::Tags(on), cx);
                            })),
                        )
                        .child(div().flex_1())
                        .child(
                            button("genre-share-import", "Importer", ButtonVariant::Primary, cx)
                                .disabled(self.share.importing)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.on_share_action(ShareAction::Import, cx)
                                })),
                        ),
                );
        }
        if let Some(job) = &self.share.job {
            card = card.child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .child(format!(
                        "Import : {}/{} · {} étiquetés · {} déjà là · {} non récupérés{}",
                        job.done,
                        job.total,
                        job.tagged,
                        job.already,
                        job.not_found,
                        if job.stopped { " · arrêté" } else { "" }
                    )),
            );
        }
        card.into_any_element()
    }

    // -- Explorer ---------------------------------------------------------

    fn explorer_tab(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if !self.explorer_loaded {
            self.load_explorer(cx);
        }
        genre_explorer::view(&self.explorer, cx, |this: &mut Self, action, cx| {
            this.on_explorer_action(action, cx)
        })
    }
}

/// A job's counters and its failure CAUSES (rule 2), the one shape both the
/// measure backfill and the embed backfill use.
fn job_lines(job: &nz_model::JobStatus, stage: Option<&str>, cx: &App) -> impl IntoElement + use<> {
    let total = job.total.max(1);
    let share = (job.scanned as f64 / total as f64).clamp(0., 1.);
    let mut column = div()
        .flex()
        .flex_col()
        .gap(rems(0.25))
        .text_size(rems(0.6875))
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .child(
                    div().flex_1().h(rems(0.375)).bg(theme::sidebar(cx)).child(
                        div()
                            .h_full()
                            .w(gpui_kit::relative(share as f32))
                            .bg(theme::accent(cx)),
                    ),
                )
                .child(
                    div()
                        .font_family(theme::mono(cx))
                        .child(format!("{}/{}", job.scanned, job.total)),
                )
                .child(div().text_color(theme::secondary(cx)).child(match stage {
                    Some(stage) => {
                        format!("{stage} · {} fait(s) · {} échec(s)", job.done, job.failed)
                    }
                    None => format!("{} fait(s) · {} échec(s)", job.done, job.failed),
                })),
        );
    for (cause, count) in &job.failure_reasons {
        column = column.child(
            div()
                .text_color(theme::danger(cx))
                .child(format!("{count} × {cause}")),
        );
    }
    if let Some(error) = &job.error {
        column = column.child(
            div()
                .text_color(theme::danger(cx))
                .child(SharedString::from(error.clone())),
        );
    }
    column
}

#[cfg(test)]
mod journey {
    use std::time::Duration;

    use super::*;
    use crate::widgets::card::stub;

    const ME: &str = r#"{"user":{"id":"u1","name":"bench","admin":true,"access":{},"sources":[]}}"#;
    const STATUS: &str = r##"{
        "archetypes": ["hard", "groove"],
        "counts": {"Ambient": 9, "Frenchcore": 2, "Hardtek": 5},
        "labelled": 2,
        "model": null,
        "tags": [
            {"id": 1, "name": "Ambient"},
            {"id": 2, "name": "Frenchcore", "color": "#aa22ff"},
            {"id": 3, "name": "Hardtek", "archetype": "hard"}
        ],
        "extractor": {"available": true, "dim": 1280, "model": "m.onnx",
                      "onnxruntime": true, "uploadable": true, "version": 3}
    }"##;
    const CANDIDATES: &str = r#"{
        "candidates": [{
            "id": "row-u1", "deezer_id": "3000001",
            "title": "Balancing the Dark", "artist": "Kaya Project",
            "album": "Songs from the Inside", "duration": 312, "play_count": 42,
            "scores": [["Hardtek", 0.62, 0.7, 0.4]],
            "uncertainty": 0.38
        }],
        "labelled": 2, "sort": "plays", "truncated": false
    }"#;
    const TREE: &str = r#"{
        "version": "7", "default": false, "can_undo": true,
        "nodes": [
            {"id": 10, "name": "Hardcore", "family": "hardcore"},
            {"id": 11, "name": "Frenchcore", "family": "hardcore"},
            {"id": 12, "name": "Hardtek"}
        ],
        "edges": [{"child": "Frenchcore", "parent": "Hardcore", "weight": 1.0}],
        "roots": ["Hardcore"]
    }"#;

    fn stub_started() -> stub::Stub {
        stub::Stub::start(&[
            ("/api/login", ME),
            ("/api/me", ME),
            ("/api/genre/status", STATUS),
            ("/api/genre/candidates", CANDIDATES),
            (
                "/api/genre/label",
                r#"{"track":"row-u1","tag":{"id":3,"name":"Hardtek"}}"#,
            ),
            ("/api/genre/tree", TREE),
            ("/api/genre/tree/undo", TREE),
            ("/api/genre/tree/edge", TREE),
            ("/api/genre/tags", r#"{"id": 13, "name": "Hardtekk"}"#),
            (
                "/api/explorer/styles",
                r#"[{"name":"Hardcore","tracks":12,"weight":3.0}]"#,
            ),
            (
                "/api/analysis/plan",
                r#"{"where":"server","version":5,"total":100,"todo":4,"current":96,
                    "unmeasured":0,"stale_measure":2,"stale_embed":1,"stale_decide":1,
                    "extractor":true,"schedule":{"enabled":false,"at":"03:00","hours":6}}"#,
            ),
            (
                "/api/analysis/backfill",
                r#"{"running":false,"scanned":0,"done":0,"failed":0,"total":0}"#,
            ),
            (
                "/api/genre/embed",
                r#"{"running":false,"scanned":0,"done":0,"failed":0,"total":0}"#,
            ),
            (
                "/api/explorer",
                r#"{"enabled":false,"archive":false,"pool":0,"pool_now":0}"#,
            ),
            ("/api/genre/labelled", r#"{"labelled":[]}"#),
        ])
    }

    fn open<'a>(
        cx: &'a mut gpui::TestAppContext,
        stub: &stub::Stub,
    ) -> (
        gpui_kit::Entity<AppState>,
        gpui_kit::Entity<Screen>,
        &'a mut gpui::VisualTestContext,
        tokio::runtime::Runtime,
    ) {
        let (state, runtime) = stub::logged_in(cx, stub);
        let (view, cx) = cx.add_window_view({
            let state = state.clone();
            move |_window, cx| Screen::new(Route::Genres, state, cx)
        });
        let handle = cx.windows()[0];
        cx.simulate_window_resize(handle, gpui_kit::size(px(1280.), px(800.)));
        cx.run_until_parked();
        (state, view, cx, runtime)
    }

    fn until(
        runtime: &tokio::runtime::Runtime,
        cx: &mut gpui::VisualTestContext,
        what: &str,
        done: impl Fn() -> bool,
    ) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {what}"
            );
            runtime.block_on(async { tokio::time::sleep(Duration::from_millis(10)).await });
            cx.run_until_parked();
        }
    }

    fn until_view(
        runtime: &tokio::runtime::Runtime,
        cx: &mut gpui::VisualTestContext,
        what: &str,
        view: &gpui_kit::Entity<Screen>,
        done: impl Fn(&Screen) -> bool,
    ) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !cx.update(|_, cx| done(view.read(cx))) {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {what}"
            );
            runtime.block_on(async { tokio::time::sleep(Duration::from_millis(10)).await });
            cx.run_until_parked();
        }
    }

    /// The screen holds the VOCABULARY and one page of candidates — never the
    /// library. Measured on the object's own allocations (not a shared test
    /// process's RSS, which says nothing about this screen): 220 tags and 40
    /// candidates of 24 scores each. The 8 000-track list is the shell's and is
    /// windowed (D8a); the studio adds no per-track row of its own.
    #[test]
    fn the_studio_holds_a_vocabulary_and_a_page_not_the_library() {
        let names = [
            "Acid techno",
            "Afro house",
            "Ambient",
            "Baroque",
            "Big beat",
            "Breakcore",
            "Chanson",
            "Darkpsy",
            "Deutscher Krach",
            "Disco / funk",
            "Drum & bass",
            "Dub techno",
            "Frenchcore",
            "Gabber",
            "Hardtek",
            "Hardtekk",
            "Hitech",
            "Jazz / acoustique",
            "Rawstyle",
            "Uptempo",
        ];
        let mut bytes = 0usize;
        let mut tags = Vec::new();
        for index in 0..220 {
            let name = names[index % names.len()].to_owned() + &index.to_string();
            bytes += std::mem::size_of::<Tag>() + name.capacity();
            tags.push(Tag::new(index as i64, name));
        }
        let mut scores = Vec::new();
        for tag in &tags {
            scores.push((tag.name.clone(), 0.5, Some(0.5), Some(0.5)));
        }
        bytes += std::mem::size_of::<genre_label::ScoreRow>() * scores.len();
        bytes += scores.iter().map(|row| row.0.capacity()).sum::<usize>();
        let mut candidates = Vec::new();
        for index in 0..40 {
            let candidate = Candidate {
                id: format!("89e80170-adc3-5d61-9266-df93e52023{index:02}"),
                deezer_id: Value::from(3_000_000 + index),
                title: format!("Balancing the Dark {index}"),
                artist: "Kaya Project".to_owned(),
                album: "Songs from the Inside".to_owned(),
                duration: 312,
                scores: scores
                    .iter()
                    .map(|row| serde_json::json!([row.0, row.1, row.2, row.3]))
                    .collect(),
                ..Candidate::default()
            };
            bytes += std::mem::size_of::<Candidate>()
                + candidate.id.capacity()
                + candidate.title.capacity()
                + candidate.artist.capacity()
                + candidate.album.capacity()
                + candidate.scores.capacity() * std::mem::size_of::<Value>();
            candidates.push(candidate);
        }
        println!(
            "studio data model: {} tags + {} candidates = {} B ({:.1} KB)",
            tags.len(),
            candidates.len(),
            bytes,
            bytes as f64 / 1024.
        );
        assert!(
            bytes < 1_000_000,
            "the studio's page costs {bytes} B, over the 1 MB budget"
        );
    }

    /// Opening the studio asks the server ONCE for the vocabulary and ONCE for
    /// the page of candidates — no fan-out per genre, and the ranked order is
    /// the one the number keys reach.
    #[gpui::test]
    async fn opening_the_studio_asks_once_and_ranks_the_vocabulary(cx: &mut gpui::TestAppContext) {
        let stub = stub_started();
        let (_state, view, cx, runtime) = open(cx, &stub);
        until(&runtime, cx, "the status", || {
            stub.count("/api/genre/status") == 1
        });
        until(&runtime, cx, "the candidates", || {
            stub.count("/api/genre/candidates") == 1
        });
        let (tags, ranked, labelled) = cx.update(|_, cx| {
            let screen = view.read(cx);
            (
                screen.tags.len(),
                genre_label::rank(&screen.tags, &[], &screen.counts)
                    .first()
                    .map(|row| row.tag.name.clone()),
                screen.labelled,
            )
        });
        assert_eq!(tags, 3);
        assert_eq!(ranked.as_deref(), Some("Ambient"), "the most used leads");
        assert_eq!(labelled, 2);
        assert_eq!(stub.count("/api/genre/status"), 1, "one vocabulary request");
        runtime.shutdown_background();
        stub.stop();
    }

    /// `1` labels the candidate with the first-ranked genre and the queue moves
    /// on; `z` undoes it — two requests, no page reload, and the keyboard works
    /// with nothing but the shell's focus anchor.
    #[gpui::test]
    async fn a_number_labels_and_z_undoes(cx: &mut gpui::TestAppContext) {
        let stub = stub_started();
        let (_state, view, cx, runtime) = open(cx, &stub);
        until(&runtime, cx, "the candidates", || {
            stub.count("/api/genre/candidates") == 1
        });
        until_view(&runtime, cx, "the queue", &view, |screen| {
            screen.candidates.ready().is_some()
        });
        cx.simulate_keystrokes("1");
        until(&runtime, cx, "the label", || {
            stub.count("/api/genre/label") == 1
        });
        let count = cx.update(|_, cx| {
            let screen = view.read(cx);
            (
                screen.counts.get("Hardtek").copied().unwrap_or(0),
                screen
                    .candidates
                    .ready()
                    .map(|rows| rows.len())
                    .unwrap_or(0),
            )
        });
        assert_eq!(count.0, 6, "the genre's own count moves at once");
        // The queue refills itself when it runs dry (one candidate in this
        // stub), so its length is not the signal; the label request and the
        // counter are.
        let _ = count.1;
        cx.simulate_keystrokes("z");
        until(&runtime, cx, "the undo", || {
            stub.count("/api/genre/label") == 2
        });
        let after = cx.update(|_, cx| {
            let screen = view.read(cx);
            (
                screen.counts.get("Hardtek").copied().unwrap_or(0),
                screen
                    .candidates
                    .ready()
                    .map(|rows| rows.iter().any(|row| row.id == "row-u1"))
                    .unwrap_or(false),
            )
        });
        assert_eq!(after.0, 5, "undo takes the genre back off the count");
        assert!(after.1, "undo puts the track back in the queue");
        runtime.shutdown_background();
        stub.stop();
    }

    /// The Arbre tab asks the tree once, and `Annuler` posts the undo route —
    /// the edit is the server's, never a local guess at the DAG.
    #[gpui::test]
    async fn the_tree_asks_once_and_undo_is_one_post(cx: &mut gpui::TestAppContext) {
        let stub = stub_started();
        let (_state, view, cx, runtime) = open(cx, &stub);
        until(&runtime, cx, "the status", || {
            stub.count("/api/genre/status") == 1
        });
        cx.update(|_, cx| {
            view.update(cx, |screen, cx| {
                screen.tab = Tab::Tree;
                screen.load_tree(cx);
            })
        });
        until(&runtime, cx, "the tree", || {
            stub.count("/api/genre/tree") == 1
        });
        until_view(&runtime, cx, "the rows", &view, |screen| {
            screen.tree.rows().len() == 3
        });
        cx.update(|_, cx| {
            view.update(cx, |screen, cx| {
                screen.on_tree_action(genre_tree::Action::Undo, cx)
            })
        });
        until(&runtime, cx, "the undo", || {
            stub.count("/api/genre/tree/undo") == 1
        });
        // A reload of the tab does not ask again: the tree is kept per visit.
        cx.update(|_, cx| {
            view.update(cx, |screen, cx| screen.load_tree(cx));
        });
        cx.run_until_parked();
        assert_eq!(stub.count("/api/genre/tree"), 1);
        runtime.shutdown_background();
        stub.stop();
    }

    /// Every tab renders at the three acceptance sizes without a panic and
    /// without a second vocabulary request: the layout is flexible rails and a
    /// scroll region, so the window's height is never the page's problem.
    #[gpui::test]
    async fn every_tab_renders_at_the_three_sizes(cx: &mut gpui::TestAppContext) {
        let stub = stub_started();
        let (_state, view, cx, runtime) = open(cx, &stub);
        until(&runtime, cx, "the candidates", || {
            stub.count("/api/genre/candidates") == 1
        });
        let handle = cx.windows()[0];
        for (width, height) in [(2560., 1600.), (1280., 800.), (660., 800.)] {
            cx.simulate_window_resize(handle, gpui_kit::size(px(width), px(height)));
            for tab in Tab::ALL {
                cx.update(|_, cx| {
                    view.update(cx, |screen, cx| {
                        screen.tab = tab;
                        match tab {
                            Tab::Library => screen.load_library(cx),
                            Tab::Tree => screen.load_tree(cx),
                            Tab::Explorer => screen.load_explorer(cx),
                            _ => {}
                        }
                        cx.notify();
                    })
                });
                cx.run_until_parked();
            }
        }
        assert_eq!(
            stub.count("/api/genre/status"),
            1,
            "no tab re-asks the vocabulary"
        );
        runtime.shutdown_background();
        stub.stop();
    }
}
