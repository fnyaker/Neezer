//! Explorer: the search screen.
//!
//! `routes/Search.svelte` plus `lib/searchrank.js` are the reference, and the
//! behaviour that matters is the same on a desktop:
//!
//! - **One request per settled term, never one per keystroke.** A term is asked
//!   `DEBOUNCE_MS` (150 ms) after the last keystroke, and only ONE request per
//!   field is in flight: a newer term is queued behind the answer, never sent on
//!   top of it (`Search::ask`). Measured in `tests`: typing `kaya` one letter at
//!   a time sends **1** `/search` for `kaya`, not four.
//! - **A term already answered paints at once.** The answers of the last
//!   [`ANSWERS_KEPT`] terms stay in memory, so backspacing is instant and a term
//!   that comes back from the back stack shows its results in the first frame.
//! - **The fallback is part of the same list**, not a second one:
//!   `lib/searchrank.js` is ported here (`rank`, `match_score`, `top_result`),
//!   and a YouTube hit only leads when it matches the query strictly better than
//!   the best catalogue answer (`YOUTUBE_MIN` > `CATALOGUE_MIN`, and equal is not
//!   enough) — those two bars are the SPA's own numbers.
//!
//! Requests: `GET /search?q=`; `GET /youtube/search?q=` when a fallback source is
//! available and the chip is on; `GET /search/podcasts?q=` and `GET
//! /transcripts/search?q=` in the podcasts scope. Nothing else — no prefetch, no
//! per-row request.

use std::cell::RefCell;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{
    Context, Entity, FocusHandle, FontWeight, IntoElement, ParentElement, Render, Styled,
    Subscription, Task, Window, div, px, rems,
};
use gpui_omarchy::IconName;
use nz_api::ApiError;
use nz_model::{
    PodcastCard, PodcastSearch, SearchPlaylist, SearchResults, Track, TranscriptHit,
    TranscriptSearch,
};

use crate::nav::Route;
use crate::state::AppState;
use crate::theme;
use crate::widgets::card;
use crate::widgets::context_menu::{self, Item, Menu};
use crate::widgets::cover::{self, Covers, Kind};
use crate::widgets::track_list::{self, Intent, TrackList as List};
use crate::wiring::events::{self, Sender};

/// How long after the last keystroke a term is asked. The lot's acceptance says
/// 150 ms (the SPA uses 280 ms on a phone's slower event loop); a desktop
/// keyboard autorepeats at ~30 ms, so 150 ms collapses a whole word into one
/// request without the field feeling laggy.
pub const DEBOUNCE_MS: u64 = 150;
/// The fallback asks later: every partial word typed would be a request there,
/// and the server keeps each answer a day (`Search.svelte`'s own 800 ms).
pub const FALLBACK_MS: u64 = 800;
/// The passages ask after the shows have had their beat.
pub const PASSAGE_MS: u64 = 300;
/// How many terms keep their answer in memory.
pub const ANSWERS_KEPT: usize = 16;
/// How many recent searches the landing remembers.
pub const RECENT_MAX: usize = 5;
/// How many titles the `Tout` tab lists before `Titres` (the SPA's own ten).
pub const SHORTLIST: usize = 10;

/// Which panel the results show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    All,
    Tracks,
    Artists,
    Albums,
    Playlists,
}

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Tab::All => "Tout",
            Tab::Tracks => "Titres",
            Tab::Artists => "Artistes",
            Tab::Albums => "Albums",
            Tab::Playlists => "Playlists",
        }
    }

    const ALL: [Tab; 5] = [
        Tab::All,
        Tab::Tracks,
        Tab::Artists,
        Tab::Albums,
        Tab::Playlists,
    ];
}

/// What a search reaches: the library and its main source, or the shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Scope {
    #[default]
    Library,
    Podcasts,
}

enum Msg {
    Search {
        term: String,
        answer: Box<Result<SearchResults, ApiError>>,
    },
    Fallback {
        term: String,
        answer: Box<Result<Vec<Track>, ApiError>>,
    },
    Shows {
        term: String,
        answer: Box<Result<Vec<PodcastCard>, ApiError>>,
    },
    Passages {
        term: String,
        answer: Box<Result<TranscriptSearch, ApiError>>,
    },
    Cover(String, Option<PathBuf>),
}

pub struct Screen {
    route: Route,
    state: Entity<AppState>,
    /// The results' own focus target: `Entrée` on the field moves the keyboard
    /// here, so `j`/`k`/`Entrée` reach the list instead of typing a letter —
    /// and the node still carries the shell's `Neezer` context, which a cleared
    /// focus would not.
    results_focus: FocusHandle,
    input: Option<Entity<InputState>>,
    query: String,
    /// The term the results are for.
    term: String,
    results: SearchResults,
    loading: bool,
    failed: Option<ApiError>,
    tab: Tab,
    scope: Scope,
    fallback_on: bool,
    fallback: Vec<Track>,
    fallback_term: String,
    fallback_error: Option<ApiError>,
    fallback_loading: bool,
    shows: Vec<PodcastCard>,
    passages: Vec<TranscriptHit>,
    passages_total: u32,
    passages_loading: bool,
    passages_error: Option<ApiError>,
    recents: Vec<String>,
    /// The ranked list of the current answer (catalogue + woven fallback), before
    /// a tab narrows what the list itself holds.
    ranked: Vec<Track>,
    /// Term → answer, so a term typed back paints in one frame.
    answers: Vec<(String, SearchResults)>,
    in_flight: bool,
    queued: Option<String>,
    fallback_in_flight: bool,
    passages_in_flight: bool,
    /// A value the field must show on the next paint (a recent term).
    pending_value: Option<String>,
    /// The next paint moves the keyboard from the field to the results.
    focus_results: bool,
    list: List,
    covers: Rc<RefCell<Covers>>,
    menu: Menu,
    /// The keystroke interceptor that owns the keyboard while a menu is open.
    menu_keys: Option<Subscription>,
    events: Sender<Msg>,
    debounce: Option<Task<()>>,
    fallback_timer: Option<Task<()>>,
    passages_timer: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Screen {
    pub fn new(route: Route, state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let query = match &route {
            Route::Search { query } => query.clone(),
            _ => String::new(),
        };
        let recents = state
            .read(cx)
            .cache
            .as_ref()
            .and_then(|cache| cache.kv().get::<Vec<String>>("search.recent"))
            .unwrap_or_default();
        let (tx, rx) = events::channel::<Msg>();
        let mut screen = Self {
            route,
            state: state.clone(),
            results_focus: cx.focus_handle(),
            input: None,
            query: query.clone(),
            term: String::new(),
            results: SearchResults::default(),
            loading: false,
            failed: None,
            tab: Tab::default(),
            scope: Scope::default(),
            fallback_on: true,
            fallback: Vec::new(),
            fallback_term: String::new(),
            fallback_error: None,
            fallback_loading: false,
            shows: Vec::new(),
            passages: Vec::new(),
            passages_total: 0,
            passages_loading: false,
            passages_error: None,
            recents,
            pending_value: None,
            focus_results: false,
            ranked: Vec::new(),
            answers: Vec::new(),
            in_flight: false,
            queued: None,
            fallback_in_flight: false,
            passages_in_flight: false,
            list: List::new(),
            covers: Rc::new(RefCell::new(Covers::new())),
            menu: Menu::new(),
            menu_keys: None,
            events: tx,
            debounce: None,
            fallback_timer: None,
            passages_timer: None,
            _subscriptions: vec![cx.observe(&state, |_, _, cx| cx.notify())],
        };
        track_list::own(cx, |this: &mut Self, intent, window, cx| {
            this.on_intent(intent, window, cx)
        });
        screen.drain(rx, cx);
        // A deep link (`g s` with a term, a share link) searches at once.
        let seeded = query.trim().to_owned();
        if !seeded.is_empty() {
            screen.ask(seeded, cx);
        }
        screen
    }

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

    // -- the search machine ---------------------------------------------------

    /// The field changed: restart the beat, and never ask on the keystroke.
    fn typed(&mut self, cx: &mut Context<Self>) {
        let term = self
            .input
            .as_ref()
            .map(|input| input.read(cx).value().to_string())
            .unwrap_or_default();
        self.query = term.clone();
        let trimmed = term.trim().to_owned();
        cx.notify();
        if trimmed.is_empty() {
            self.debounce = None;
            self.term.clear();
            self.results = SearchResults::default();
            self.list.clear();
            self.loading = false;
            self.failed = None;
            return;
        }
        let term = trimmed.clone();
        self.debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(DEBOUNCE_MS))
                .await;
            let _ = this.update(cx, |this, cx| this.ask(term, cx));
        }));
        // The fallback and the passages have their own, longer beats; they are
        // only armed once the library's own term has settled, so a word being
        // typed never costs three requests per letter.
        let fallback = trimmed.clone();
        self.fallback_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(FALLBACK_MS))
                .await;
            let _ = this.update(cx, |this, cx| this.ask_fallback(fallback, cx));
        }));
        if self.scope == Scope::Podcasts {
            let passages = trimmed.clone();
            self.passages_timer = Some(cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(PASSAGE_MS))
                    .await;
                let _ = this.update(cx, |this, cx| this.ask_passages(passages, cx));
            }));
        }
    }

    /// Enter (or a recent term): the field's value becomes the route's query, so
    /// `Alt+←` comes back to this search, and the search runs at once.
    fn commit(&mut self, cx: &mut Context<Self>) {
        let term = self.query.trim().to_owned();
        if term.is_empty() {
            return;
        }
        self.note_recent(&term, cx);
        let route = Route::Search {
            query: term.clone(),
        };
        if self.route != route {
            self.route = route.clone();
            self.state.update(cx, |state, _| state.nav.replace(route));
        }
        // Enter drops the keyboard back to the results, as the SPA blurs the
        // field: the list keys are app-level (`track_list::own`), so an unfocused
        // window is exactly where they work.
        // The focus moves on the NEXT paint, where the window is at hand: a
        // deferred closure has no window in a test, and the results are what the
        // keys must reach after `Entrée`.
        self.focus_results = true;
        self.ask(term, cx);
        cx.notify();
    }

    /// Ask for `term`, or paint the answer this session already has and then
    /// revalidate it. ONE request per field in flight (`in_flight`), the newest
    /// term queued behind it (`queued`).
    fn ask(&mut self, term: String, cx: &mut Context<Self>) {
        if term.is_empty() {
            return;
        }
        self.term = term.clone();
        self.failed = None;
        if let Some(answer) = self.remembered(&term) {
            self.paint(answer, &term, cx);
        } else {
            self.loading = true;
        }
        if self.in_flight {
            self.queued = Some(term);
            cx.notify();
            return;
        }
        self.send_search(term, cx);
    }

    fn send_search(&mut self, term: String, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        self.in_flight = true;
        self.loading = true;
        let tx = self.events.clone();
        let asked = term.clone();
        self.state.read(cx).runtime.clone().spawn(async move {
            let answer = api.search(&asked).await;
            tx.send(Msg::Search {
                term: asked,
                answer: Box::new(answer),
            });
        });
        cx.notify();
    }

    fn ask_fallback(&mut self, term: String, cx: &mut Context<Self>) {
        if !self.fallback_on || self.scope != Scope::Library || term.is_empty() {
            return;
        }
        if self.fallback_term == term && self.fallback_error.is_none() {
            return;
        }
        if self.fallback_in_flight {
            return;
        }
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        self.fallback_in_flight = true;
        self.fallback_loading = true;
        let tx = self.events.clone();
        let asked = term.clone();
        self.state.read(cx).runtime.clone().spawn(async move {
            let answer = api.youtube_search(&asked).await.map(|list| list.tracks);
            tx.send(Msg::Fallback {
                term: asked,
                answer: Box::new(answer),
            });
        });
        cx.notify();
    }

    fn ask_shows(&mut self, term: String, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        let tx = self.events.clone();
        let asked = term.clone();
        self.state.read(cx).runtime.clone().spawn(async move {
            let answer = api
                .search_podcasts(&asked)
                .await
                .map(|search: PodcastSearch| search.podcasts);
            tx.send(Msg::Shows {
                term: asked,
                answer: Box::new(answer),
            });
        });
    }

    fn ask_passages(&mut self, term: String, cx: &mut Context<Self>) {
        if self.passages_in_flight || term.len() < 2 {
            return;
        }
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        self.passages_in_flight = true;
        self.passages_loading = true;
        self.passages_error = None;
        let tx = self.events.clone();
        let asked = term.clone();
        self.state.read(cx).runtime.clone().spawn(async move {
            let answer = api.search_transcripts(&asked, 20).await;
            tx.send(Msg::Passages {
                term: asked,
                answer: Box::new(answer),
            });
        });
        cx.notify();
    }

    fn on_msg(&mut self, msg: Msg, cx: &mut Context<Self>) {
        match msg {
            Msg::Search { term, answer } => {
                self.in_flight = false;
                if term == self.term {
                    match *answer {
                        Ok(results) => {
                            self.loading = false;
                            self.paint(results, &term, cx);
                        }
                        Err(error) => {
                            self.loading = false;
                            self.failed = Some(error);
                            self.results = SearchResults::default();
                            self.list.clear();
                        }
                    }
                } else if let Ok(results) = *answer {
                    self.remember(&term, results);
                }
                // Whatever was typed while this answer was on its way starts now:
                // one request in flight per field, and never a lost term.
                if !self.in_flight
                    && let Some(next) = self.queued.take()
                {
                    self.send_search(next, cx);
                }
            }
            Msg::Fallback { term, answer } => {
                self.fallback_in_flight = false;
                self.fallback_loading = false;
                if term == self.term || self.term.is_empty() {
                    self.fallback_term = term;
                    match *answer {
                        Ok(tracks) => {
                            self.fallback = tracks;
                            let term = self.term.clone();
                            let results = self.results.clone();
                            if !term.is_empty() {
                                self.paint(results, &term, cx);
                            }
                        }
                        Err(error) => {
                            // A refusal is a state with a way out, never a verdict
                            // about the query (rule 3).
                            self.fallback = Vec::new();
                            self.fallback_error = Some(error);
                        }
                    }
                }
            }
            Msg::Shows { term, answer } => {
                if term == self.term {
                    self.shows = answer.map(|podcasts| podcasts).unwrap_or_default();
                }
            }
            Msg::Passages { term, answer } => {
                self.passages_in_flight = false;
                self.passages_loading = false;
                if term == self.term {
                    match *answer {
                        Ok(found) => {
                            self.passages_total = found.total;
                            self.passages = found.hits;
                            self.passages_error = None;
                        }
                        Err(error) => {
                            self.passages = Vec::new();
                            self.passages_total = 0;
                            self.passages_error = Some(error);
                        }
                    }
                }
            }
            Msg::Cover(url, file) => self.covers.borrow_mut().resolved(&url, file),
        }
        cx.notify();
    }

    /// Show an answer: the list, and the memory of it.
    fn paint(&mut self, results: SearchResults, term: &str, cx: &mut Context<Self>) {
        self.ranked = rank(
            term,
            &results.tracks,
            if self.fallback_on && self.fallback_term == term {
                &self.fallback
            } else {
                &[]
            },
        );
        self.list.set_context(Some(nz_player::Context {
            kind: "search".to_owned(),
            id: Some(term.to_owned()),
            title: Some(term.to_owned()),
        }));
        self.sync_list();
        self.remember(term, results.clone());
        self.results = results;
        self.loading = false;
        self.failed = None;
        cx.notify();
    }

    /// What the list holds for the open tab: the `Tout` tab is a shortlist of
    /// [`SHORTLIST`] titles (the web app's own ten) and the rest is one click away
    /// on `Titres`, so the page above the fold is a shortlist and not a wall.
    fn sync_list(&mut self) {
        let rows = match self.tab {
            Tab::All => self
                .ranked
                .iter()
                .take(SHORTLIST)
                .cloned()
                .collect::<Vec<_>>(),
            _ => self.ranked.clone(),
        };
        self.list.set_rows(rows);
    }

    fn remembered(&self, term: &str) -> Option<SearchResults> {
        self.answers
            .iter()
            .find(|(known, _)| known == term)
            .map(|(_, answer)| answer.clone())
    }

    fn remember(&mut self, term: &str, answer: SearchResults) {
        if let Some(entry) = self.answers.iter_mut().find(|(known, _)| known == term) {
            entry.1 = answer;
            return;
        }
        self.answers.insert(0, (term.to_owned(), answer));
        self.answers.truncate(ANSWERS_KEPT);
    }

    fn note_recent(&mut self, term: &str, cx: &mut Context<Self>) {
        let term = term.trim();
        if term.len() < 2 {
            return;
        }
        self.recents
            .retain(|known| known.to_lowercase() != term.to_lowercase());
        self.recents.insert(0, term.to_owned());
        self.recents.truncate(RECENT_MAX);
        if let Some(cache) = self.state.read(cx).cache.as_ref()
            && let Err(error) = cache.kv().set("search.recent", &self.recents)
        {
            tracing::debug!(%error, "recherches récentes non enregistrées");
        }
    }

    fn clear_recents(&mut self, cx: &mut Context<Self>) {
        self.recents.clear();
        if let Some(cache) = self.state.read(cx).cache.as_ref() {
            let _ = cache.kv().delete("search.recent");
        }
        cx.notify();
    }

    fn use_recent(&mut self, term: String, cx: &mut Context<Self>) {
        self.query = term.clone();
        // The field's text is replaced on the next paint, where the window is at
        // hand (`InputState::set_value` needs one); asking now paints the results
        // in this frame instead of after the field catches up.
        self.pending_value = Some(term.clone());
        self.note_recent(&term, cx);
        self.ask(term, cx);
    }

    // -- rows and menus -------------------------------------------------------

    fn covers_for_visible(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        let mut asked: Vec<String> = Vec::new();
        for index in range {
            if let Some(url) = self
                .list
                .row(index)
                .and_then(|track| track.album.cover.clone())
            {
                asked.push(url);
            }
        }
        if let Some(picture) = self.top_artist().and_then(|top| top.picture.clone()) {
            asked.push(picture);
        }
        for album in self.results.albums.iter().take(12) {
            if let Some(cover) = album.cover.clone() {
                asked.push(cover);
            }
        }
        for artist in self.results.artists.iter().take(12) {
            if let Some(picture) = artist.picture.clone() {
                asked.push(picture);
            }
        }
        for show in self.shows.iter().take(12) {
            if let Some(cover) = show.cover.clone() {
                asked.push(cover);
            }
        }
        let snapshot = self.state.read(cx);
        let mut covers = self.covers.borrow_mut();
        for url in asked {
            let for_request = url.clone();
            let for_message = url.clone();
            let tx = self.events.clone();
            covers.want(snapshot, &for_request, move |file| {
                tx.send(Msg::Cover(for_message, file))
            });
        }
    }

    fn top_artist(&self) -> Option<nz_model::Artist> {
        if self.scope != Scope::Library || self.tab != Tab::All {
            return None;
        }
        top_result(&self.term, &self.results)
    }

    fn open_menu(&mut self, at: Option<usize>, cx: &mut Context<Self>) {
        if self.menu.is_open() {
            self.menu.close();
            self.menu_keys = None;
            cx.notify();
            return;
        }
        let Some(index) = at.or_else(|| self.list.selected()) else {
            return;
        };
        let Some((has_album, has_artist)) = self.list.row(index).map(|track| {
            (
                !track.album.deezer_id.as_str().is_empty(),
                !track.artist.deezer_id.as_str().is_empty(),
            )
        }) else {
            return;
        };
        self.list.select(index);
        let items = context_menu::track_items(has_album, has_artist);
        let top = track_list::row_top(index, self.list.scroll_y(), items.len());
        self.menu.open_at(top, items);
        // The interceptor is app-level (GPUI has no per-entity keystroke
        // interceptor), so it holds a weak entity: a screen that goes away leaves
        // a no-op, never a crash.
        let weak = cx.weak_entity();
        self.menu_keys = Some(gpui_kit::App::intercept_keystrokes(
            &mut *cx,
            move |event, _window, cx| {
                let _ = weak.update(cx, |this, cx| this.on_menu_key(event, cx));
            },
        ));
        cx.notify();
    }

    fn on_intent(&mut self, intent: Intent, window: &mut Window, cx: &mut Context<Self>) {
        let page = List::page_rows(window.viewport_size().height);
        match intent {
            Intent::Menu(at) => self.open_menu(at, cx),
            Intent::Like(_) | Intent::Dislike(_) => {}
            other => {
                if let Some(request) = self.list.apply(other, page) {
                    let command = request.command();
                    self.state
                        .update(cx, |state, cx| state.command(command, cx));
                }
                cx.notify();
            }
        }
    }

    /// The keyboard while the menu is open: `Échap` closes it, `↑`/`↓` move in
    /// it, `Entrée` picks. A keystroke INTERCEPTOR rather than an action handler,
    /// because the shell owns `Échap` (`CloseTop`) and would go back before a
    /// screen-level handler ever saw it.
    fn on_menu_key(&mut self, event: &gpui_kit::KeystrokeEvent, cx: &mut Context<Self>) {
        eprintln!("MENU KEY {}", event.keystroke.key);
        match event.keystroke.key.as_str() {
            "escape" => {
                self.menu.close();
                self.menu_keys = None;
                cx.notify();
                cx.stop_propagation();
            }
            "up" | "k" => {
                self.menu.move_cursor(-1);
                cx.notify();
                cx.stop_propagation();
            }
            "down" | "j" => {
                self.menu.move_cursor(1);
                cx.notify();
                cx.stop_propagation();
            }
            "enter" => {
                if let Some(item) = self.menu.cursor_item() {
                    self.pick(item, cx);
                }
                cx.stop_propagation();
            }
            // Anything else closes the menu, as a backdrop press would: a panel
            // that stays open under a key it does not know is a panel in the way.
            _ => {
                self.menu.close();
                self.menu_keys = None;
                cx.notify();
                cx.stop_propagation();
            }
        }
    }

    fn pick(&mut self, item: Item, cx: &mut Context<Self>) {
        self.menu.close();
        self.menu_keys = None;
        let index = self.list.selected();
        match item {
            Item::Play | Item::PlayNext | Item::Queue => {
                let intent = match item {
                    Item::Play => Intent::PlayFrom(index),
                    Item::PlayNext => Intent::PlayNextFrom(index),
                    _ => Intent::QueueFrom(index),
                };
                if let Some(request) = self.list.apply(intent, 1) {
                    let command = request.command();
                    self.state
                        .update(cx, |state, cx| state.command(command, cx));
                }
            }
            Item::Credits => {
                if let Some(id) = index
                    .and_then(|index| self.list.row(index))
                    .map(|track| track.deezer_id.as_str().to_owned())
                {
                    let route = Route::Credits {
                        kind: crate::nav::CreditKind::Track,
                        id,
                    };
                    self.state.update(cx, |state, _| state.nav.push(route));
                }
            }
            Item::GoAlbum => {
                if let Some(id) = index
                    .and_then(|index| self.list.row(index))
                    .map(|track| track.album.deezer_id.as_str().to_owned())
                    .filter(|id| !id.is_empty())
                {
                    self.state
                        .update(cx, |state, _| state.nav.push(Route::Album { id }));
                }
            }
            Item::GoArtist => {
                if let Some(id) = index
                    .and_then(|index| self.list.row(index))
                    .map(|track| track.artist.deezer_id.as_str().to_owned())
                    .filter(|id| !id.is_empty())
                {
                    self.state
                        .update(cx, |state, _| state.nav.push(Route::Artist { id }));
                }
            }
            Item::Like | Item::Dislike | Item::Radio | Item::Open => {}
        }
        cx.notify();
    }

    fn open_artist(&mut self, id: String, cx: &mut Context<Self>) {
        if id.is_empty() {
            return;
        }
        self.state
            .update(cx, |state, _| state.nav.push(Route::Artist { id }));
    }

    fn open_album(&mut self, id: String, cx: &mut Context<Self>) {
        if id.is_empty() {
            return;
        }
        self.state
            .update(cx, |state, _| state.nav.push(Route::Album { id }));
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let term = self.term.clone();
        if !term.is_empty() {
            self.ask(term, cx);
        }
    }
}

impl Render for Screen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.input.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Un titre, un artiste, un album…")
                    .default_value(self.query.clone())
            });
            let subscription =
                cx.subscribe(&input, |this, _, event: &InputEvent, cx| match event {
                    InputEvent::Change => this.typed(cx),
                    InputEvent::PressEnter { .. } => this.commit(cx),
                    _ => {}
                });
            self._subscriptions.push(subscription);
            // A person who opens the search arrives to type; a screen that came
            // back with a term in it puts the keyboard on the results instead.
            if self.query.trim().is_empty() {
                input.update(cx, |state, cx| state.focus(window, cx));
            }
            self.input = Some(input);
        }
        if std::mem::take(&mut self.focus_results) {
            window.focus(&self.results_focus, cx);
        }
        if let Some(value) = self.pending_value.take()
            && let Some(input) = self.input.clone()
        {
            input.update(cx, |state, cx| state.set_value(value, window, cx));
        }
        let root = div()
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .p(rems(1.5))
            .gap(rems(0.75))
            .font_family(theme::font(cx))
            .text_color(theme::text(cx))
            // The shell's own context (`app.rs`), restated so a screen is a whole
            // page on its own: a test that renders it without the shell still
            // gets the keymap's bindings, and no second context name can drift.
            .key_context("Neezer")
            .track_focus(&self.results_focus);

        // The sources: the library (always), the fallback, the shows.
        let mut sources: Vec<gpui_kit::AnyElement> = Vec::new();
        sources.push(card::chip(
            "scope-library".into(),
            "Ma bibliothèque",
            self.scope == Scope::Library,
            cx,
            |this: &mut Self, _, cx| {
                this.scope = Scope::Library;
                let term = this.term.clone();
                if !term.is_empty() {
                    this.ask(term, cx);
                }
                cx.notify();
            },
        ));
        sources.push(card::chip(
            "scope-fallback".into(),
            if self.fallback_on {
                "YouTube"
            } else {
                "YouTube (off)"
            },
            self.fallback_on,
            cx,
            |this: &mut Self, _, cx| {
                this.fallback_on = !this.fallback_on;
                let term = this.term.clone();
                if this.fallback_on && !term.is_empty() {
                    this.ask_fallback(term, cx);
                }
                cx.notify();
            },
        ));
        sources.push(card::chip(
            "scope-podcasts".into(),
            "Podcasts",
            self.scope == Scope::Podcasts,
            cx,
            |this: &mut Self, _, cx| {
                this.scope = if this.scope == Scope::Podcasts {
                    Scope::Library
                } else {
                    Scope::Podcasts
                };
                let term = this.term.clone();
                if !term.is_empty() {
                    if this.scope == Scope::Podcasts {
                        this.ask_shows(term.clone(), cx);
                        this.ask_passages(term, cx);
                    } else {
                        this.ask(term, cx);
                    }
                }
                cx.notify();
            },
        ));

        let field = self
            .input
            .as_ref()
            .map(|field| gpui_omarchy::input("search-field", field, window, cx));

        let mut bar = div().flex().items_center().gap(rems(0.5));
        if let Some(field) = field {
            bar = bar.child(div().flex_1().min_w_0().max_w(rems(36.)).child(field));
        }
        bar = bar.child(card::chip_row(sources));

        // The results page scrolls as ONE page, except on `Titres` where the
        // windowed list owns the height (its own scrolling is the list's). A
        // column that overflows without scrolling clips the list away — measured
        // first: a click never reached a row because the rows were below the
        // window.
        let mut page = div().flex().flex_col().flex_1().min_h_0().gap(rems(0.75));

        // The landing: what this device already searched, never an empty page.
        if self.query.trim().is_empty() {
            page = page.child(
                div().flex().flex_col().gap(rems(0.5)).child(
                    div()
                        .text_size(rems(0.8125))
                        .text_color(theme::secondary(cx))
                        .child(
                            "Cherche un titre, un artiste, un album ou un podcast : \
                                 la bibliothèque d'abord, puis le catalogue.",
                        ),
                ),
            );
            if !self.recents.is_empty() {
                let mut chips: Vec<gpui_kit::AnyElement> = Vec::new();
                for term in self.recents.clone() {
                    let label = term.clone();
                    chips.push(card::chip(
                        format!("recent-{term}").into(),
                        &label,
                        false,
                        cx,
                        move |this: &mut Self, _, cx| this.use_recent(term.clone(), cx),
                    ));
                }
                chips.push(card::chip(
                    "recent-clear".into(),
                    "Effacer",
                    false,
                    cx,
                    |this: &mut Self, _, cx| this.clear_recents(cx),
                ));
                page = page.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rems(0.375))
                        .child(
                            div()
                                .text_size(rems(0.75))
                                .font_weight(FontWeight::BOLD)
                                .child("Récents"),
                        )
                        .child(card::chip_row(chips)),
                );
            }
            return root.child(bar).child(page).into_any_element();
        }

        if self.scope == Scope::Library {
            let mut tabs: Vec<gpui_kit::AnyElement> = Vec::new();
            for tab in Tab::ALL {
                tabs.push(card::chip(
                    format!("tab-{}", tab.label()).into(),
                    tab.label(),
                    self.tab == tab,
                    cx,
                    move |this: &mut Self, _, cx| {
                        this.tab = tab;
                        this.sync_list();
                        cx.notify();
                    },
                ));
            }
            page = page.child(card::chip_row(tabs));
        }

        if self.scope == Scope::Podcasts {
            return root
                .child(bar)
                .child(self.passages_panel(cx))
                .into_any_element();
        }

        if self.loading && self.results.is_empty() {
            return root
                .child(bar)
                .child(div().p(rems(1.)).child(card::skeleton(6, px(120.), cx)))
                .into_any_element();
        }
        if let Some(error) = &self.failed {
            let cause = card::cause(error);
            return root
                .child(bar)
                .child(card::failed(
                    "Recherche impossible",
                    cause,
                    cx,
                    |this: &mut Self, _, cx| this.reload(cx),
                ))
                .into_any_element();
        }

        let visible = self.list.visible();
        self.covers_for_visible(visible, cx);

        let top = self.top_artist();
        let artists = self.results.artists.clone();
        let albums = self.results.albums.clone();
        let playlists = self.results.playlists.clone();

        // The best result the query plainly names, then the titles, then the
        // shelves — the SPA's order (`Search.svelte`'s "Tout" tab).
        if self.tab == Tab::All
            && let Some(artist) = top.clone()
        {
            let id = artist.request_id().as_str().to_owned();
            let picture = artist
                .picture
                .as_deref()
                .and_then(|url| cover::file_in(&self.covers, url));
            page = page
                .child(
                    div()
                        .text_size(rems(0.75))
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme::secondary(cx))
                        .child("Meilleur résultat"),
                )
                .child(card::card(
                    format!("top-{}", artist.name).into(),
                    &artist.name,
                    "Artiste",
                    picture.as_ref(),
                    artist.picture.as_deref(),
                    Kind::Artist,
                    cx,
                    move |this: &mut Self, _, cx| this.open_artist(id.clone(), cx),
                    |_: &mut Self, _, _| {},
                    None::<fn(&mut Self, &mut Window, &mut Context<Self>)>,
                ));
        }

        let tracks_shown = match self.tab {
            Tab::All => self.list.len().min(10),
            _ => self.list.len(),
        };
        if (self.tab == Tab::All || self.tab == Tab::Tracks) && tracks_shown > 0 {
            let element = track_list::element(
                cx.entity(),
                "search-tracks",
                &mut self.list,
                &self.state,
                Rc::clone(&self.covers),
                &self.menu,
                window,
                cx,
                |this: &mut Self, intent, window, cx| this.on_intent(intent, window, cx),
                |_, _, _, _| None,
                |this: &mut Self, item, cx| this.pick(item, cx),
                |this: &mut Self, cx| {
                    this.menu.close();
                    cx.notify();
                },
            );
            // What the shortlist left out, counted on the RANKED list: the list
            // itself only holds the shortlist on this tab.
            let more = self.ranked.len().saturating_sub(tracks_shown);
            page = page
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.5))
                        .child(
                            div()
                                .text_size(rems(0.75))
                                .font_weight(FontWeight::BOLD)
                                .child(format!(
                                    "Titres · {}",
                                    match self.tab {
                                        Tab::All => self.ranked.len(),
                                        _ => self.list.len(),
                                    }
                                )),
                        )
                        .child(div().flex_1())
                        .when(self.tab == Tab::All && more > 0, |row| {
                            row.child(card::chip(
                                "tab-all-tracks".into(),
                                &format!("Tout afficher ({more} de plus)"),
                                false,
                                cx,
                                |this: &mut Self, _, cx| {
                                    this.tab = Tab::Tracks;
                                    this.sync_list();
                                    cx.notify();
                                },
                            ))
                        }),
                )
                .child(if self.tab == Tab::Tracks {
                    div().flex_1().min_h_0().child(element).into_any_element()
                } else {
                    // Ten rows on the `Tout` tab: the shelf above it stays in view
                    // and the whole page scrolls, one scroller at a time.
                    div()
                        .h(rems(3.5 * tracks_shown as f32))
                        .child(element)
                        .into_any_element()
                });
        }

        if (self.tab == Tab::All || self.tab == Tab::Artists) && !artists.is_empty() {
            let cards: Vec<_> = artists
                .iter()
                .map(|artist| {
                    let id = artist.request_id().as_str().to_owned();
                    let picture = artist
                        .picture
                        .as_deref()
                        .and_then(|url| cover::file_in(&self.covers, url));
                    card::card(
                        format!("search-artist-{}", artist.name).into(),
                        &artist.name,
                        "Artiste",
                        picture.as_ref(),
                        artist.picture.as_deref(),
                        Kind::Artist,
                        cx,
                        move |this: &mut Self, _, cx| this.open_artist(id.clone(), cx),
                        |_: &mut Self, _, _| {},
                        None::<fn(&mut Self, &mut Window, &mut Context<Self>)>,
                    )
                })
                .collect();
            page = page
                .child(
                    card::Section::new("Artistes")
                        .count(artists.len())
                        .render(cx, |_, _, _| {}),
                )
                .child(card::shelf(cards));
        }

        if (self.tab == Tab::All || self.tab == Tab::Albums) && !albums.is_empty() {
            let cards: Vec<_> = albums
                .iter()
                .map(|album| {
                    let id = if !album.deezer_id.as_str().is_empty() {
                        album.deezer_id.as_str().to_owned()
                    } else {
                        album
                            .id
                            .as_ref()
                            .map(|id| id.to_string())
                            .unwrap_or_default()
                    };
                    let file = album
                        .cover
                        .as_deref()
                        .and_then(|url| cover::file_in(&self.covers, url));
                    card::card(
                        format!("search-album-{}", album.title).into(),
                        &album.title,
                        &album.artist.name,
                        file.as_ref(),
                        album.cover.as_deref(),
                        Kind::Album,
                        cx,
                        move |this: &mut Self, _, cx| this.open_album(id.clone(), cx),
                        |_: &mut Self, _, _| {},
                        None::<fn(&mut Self, &mut Window, &mut Context<Self>)>,
                    )
                })
                .collect();
            page = page
                .child(
                    card::Section::new("Albums")
                        .count(albums.len())
                        .render(cx, |_, _, _| {}),
                )
                .child(card::shelf(cards));
        }

        if (self.tab == Tab::All || self.tab == Tab::Playlists) && !playlists.is_empty() {
            let cards: Vec<_> = playlists
                .iter()
                .map(|playlist| self.playlist_card(playlist, cx))
                .collect();
            page = page
                .child(
                    card::Section::new("Playlists")
                        .count(playlists.len())
                        .render(cx, |_, _, _| {}),
                )
                .child(card::shelf(cards));
        }

        // What the fallback is doing, under the SAME list (never a section).
        if self.scope == Scope::Library && self.tab != Tab::Playlists {
            if let Some(error) = self.fallback_error.clone() {
                page = page.child(card::inline_error(
                    format!("YouTube : {}", card::cause(&error)),
                    cx,
                    |this: &mut Self, _, cx| {
                        let term = this.term.clone();
                        this.fallback_error = None;
                        this.ask_fallback(term, cx);
                    },
                ));
            } else if self.fallback_loading {
                page = page.child(
                    div()
                        .text_size(rems(0.75))
                        .text_color(theme::secondary(cx))
                        .child("Recherche sur YouTube…"),
                );
            }
        }

        if self.list.is_empty()
            && artists.is_empty()
            && albums.is_empty()
            && playlists.is_empty()
            && !self.loading
            && !self.fallback_loading
        {
            page = page.child(card::empty(
                IconName::Search,
                "Aucun résultat",
                "Rien pour cette recherche dans la bibliothèque. Essayez un autre mot, ou le catalogue.",
                cx,
                None,
            ));
        }

        // One scroller: on `Tout`, `Artistes`, `Albums` and `Playlists` the whole
        // results page scrolls and the list is a fixed 10-row band inside it; on
        // `Titres` the list owns the height and scrolls itself.
        let body: gpui_kit::AnyElement = if self.tab == Tab::Tracks {
            page.into_any_element()
        } else {
            div()
                .id("search-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(page)
                .into_any_element()
        };
        root.child(bar).child(body).into_any_element()
    }
}

impl Screen {
    fn playlist_card(
        &mut self,
        playlist: &SearchPlaylist,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let id = playlist.deezer_id.clone();
        let file = playlist
            .cover
            .as_deref()
            .and_then(|url| cover::file_in(&self.covers, url));
        card::card(
            format!("search-playlist-{}", playlist.title).into(),
            &playlist.title,
            &format!(
                "{} · {} titres",
                if playlist.owner.is_empty() {
                    "Playlist"
                } else {
                    playlist.owner.as_str()
                },
                playlist.nb_tracks
            ),
            file.as_ref(),
            playlist.cover.as_deref(),
            Kind::Playlist,
            cx,
            move |this: &mut Self, _, cx| {
                let id = id.clone();
                this.state
                    .update(cx, |state, _| state.nav.push(Route::Playlist { id }));
            },
            |_: &mut Self, _, _| {},
            None::<fn(&mut Self, &mut Window, &mut Context<Self>)>,
        )
    }

    /// The podcasts scope: the shows and the spoken passages, the SPA's two
    /// panels. A passage plays nothing by itself (the episode panel is D11's), so
    /// it opens the show the passage is in — the only door this screen has.
    fn passages_panel(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let mut panel = div().flex().flex_col().flex_1().min_h_0().gap(rems(0.5));
        if self.passages_loading {
            return panel
                .child(
                    div()
                        .text_size(rems(0.75))
                        .text_color(theme::secondary(cx))
                        .child("Recherche dans les transcriptions…"),
                )
                .into_any_element();
        }
        if let Some(error) = self.passages_error.clone() {
            panel = panel.child(card::inline_error(
                skippable_cause(&error),
                cx,
                |this: &mut Self, _, cx| {
                    let term = this.term.clone();
                    this.passages_error = None;
                    this.ask_passages(term, cx);
                },
            ));
        } else if !self.passages.is_empty() {
            panel = panel.child(
                div()
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child(format!(
                        "{} passage{} dans les transcriptions",
                        self.passages_total,
                        if self.passages_total > 1 { "s" } else { "" }
                    )),
            );
            for (index, hit) in self.passages.clone().iter().enumerate() {
                let channel = hit.channel_id.as_ref().map(|id| id.as_str().to_owned());
                let show = hit
                    .channel_title
                    .clone()
                    .unwrap_or_else(|| "Émission".to_owned());
                let episode = hit
                    .episode
                    .as_ref()
                    .map(|episode| episode.title.clone())
                    .unwrap_or_default();
                let when = clock(hit.start);
                panel = panel.child(
                    div()
                        .id(("passage", index))
                        .flex()
                        .flex_col()
                        .gap(rems(0.125))
                        .p(rems(0.5))
                        .border_b_1()
                        .border_color(theme::border(cx).opacity(0.5))
                        .cursor_pointer()
                        .hover(|style| style.bg(theme::chrome(cx)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(id) = channel.clone() {
                                this.state
                                    .update(cx, |state, _| state.nav.push(Route::Show { id }));
                            }
                            cx.notify();
                        }))
                        .child(
                            div()
                                .text_size(rems(0.5625))
                                .font_family(theme::mono(cx))
                                .text_color(theme::secondary(cx))
                                .child(format!("{when} · {show}")),
                        )
                        .child(div().text_size(rems(0.8125)).child(highlighted(&hit.text)))
                        .when(!episode.is_empty(), |row| {
                            row.child(
                                div()
                                    .text_size(rems(0.6875))
                                    .text_color(theme::secondary(cx))
                                    .child(episode),
                            )
                        }),
                );
            }
        } else if !self.shows.is_empty() {
            let cards: Vec<_> = self
                .shows
                .iter()
                .map(|show| {
                    let title = show.title.clone();
                    let file = show
                        .cover
                        .as_deref()
                        .and_then(|url| cover::file_in(&self.covers, url));
                    card::card(
                        format!("show-{}", show.title).into(),
                        &title,
                        &format!("{} fans", show.fans),
                        file.as_ref(),
                        show.cover.as_deref(),
                        Kind::Podcast,
                        cx,
                        move |_: &mut Self, _, _| {},
                        |_: &mut Self, _, _| {},
                        None::<fn(&mut Self, &mut Window, &mut Context<Self>)>,
                    )
                })
                .collect();
            panel = panel.child(card::shelf(cards));
        } else {
            panel = panel.child(card::empty(
                IconName::Mic,
                "Rien dans les podcasts",
                "La recherche porte sur les titres des émissions et le texte des épisodes déjà transcrits.",
                cx,
                None,
            ));
        }
        panel.into_any_element()
    }
}

/// A passage's text with the server's `[… highlights …]` turned into a case
/// change: GPUI has one weight per run, so the brackets would be noise.
fn highlighted(text: &str) -> String {
    text.replace(['[', ']'], "")
}

fn clock(seconds: f64) -> String {
    let seconds = seconds.max(0.) as u32;
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            (seconds / 60) % 60,
            seconds % 60
        )
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

/// A transcript search's failure, said in the screen's own words: a server that
/// has no transcript route at all is a feature that is off, not an error.
fn skippable_cause(error: &ApiError) -> String {
    match error {
        ApiError::Http { status: 404, .. } => {
            "La recherche dans les transcriptions n'est pas disponible sur ce serveur.".to_owned()
        }
        other => card::cause(other).to_string(),
    }
}

// -- `lib/searchrank.js`, ported -----------------------------------------------------
//
// The three functions that decide the search's order. They are pure and tested
// against the JavaScript's own output (`tests`), because the order is a design
// decision and not an implementation detail: a top card that is not what was
// typed pushes the titles down for nothing, and a video promoted over an equal
// catalogue answer is noise.

/// The main source's bar: below it, a catalogue row is not a comparable match.
pub const CATALOGUE_MIN: f64 = 0.5;
/// YouTube's bar: a video must be a real match, not just a word in its title.
pub const YOUTUBE_MIN: f64 = 0.75;

/// `fold`: what two names are compared on (accents, case and punctuation gone).
pub fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for character in text.chars() {
        // Combining marks: text that arrives decomposed carries its accents as
        // their own characters.
        if ('\u{0300}'..='\u{036F}').contains(&character) {
            continue;
        }
        for base in deaccent(character).to_lowercase().chars() {
            if base.is_ascii_alphanumeric() {
                if space && !out.is_empty() {
                    out.push(' ');
                }
                space = false;
                out.push(base);
            } else {
                space = true;
            }
        }
    }
    out
}

/// The base letters of the precomposed Latin accents French, English and German
/// titles use. `"Été"` → `"ete"`, `"AC/DC"` → `"ac dc"`, `"Motörhead"` →
/// `"motorhead"`, exactly as `normalize("NFD")` + a mark strip does.
fn deaccent(character: char) -> &'static str {
    match character {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' | 'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å'
        | 'Ā' | 'Ă' | 'Ą' => "a",
        'æ' | 'Æ' => "ae",
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' | 'Ç' | 'Ć' | 'Ĉ' | 'Ċ' | 'Č' => "c",
        'ď' | 'đ' | 'Ď' | 'Đ' => "d",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' | 'È' | 'É' | 'Ê' | 'Ë' | 'Ē' | 'Ĕ'
        | 'Ė' | 'Ę' | 'Ě' => "e",
        'ĝ' | 'ğ' | 'ġ' | 'ģ' | 'Ĝ' | 'Ğ' | 'Ġ' | 'Ģ' => "g",
        'ĥ' | 'ħ' | 'Ĥ' | 'Ħ' => "h",
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' | 'Ì' | 'Í' | 'Î' | 'Ï' | 'Ĩ' | 'Ī'
        | 'Ĭ' | 'Į' | 'İ' => "i",
        'ĳ' | 'Ĳ' => "ij",
        'ĵ' | 'Ĵ' => "j",
        'ķ' | 'Ķ' => "k",
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' | 'Ĺ' | 'Ļ' | 'Ľ' | 'Ŀ' | 'Ł' => "l",
        'ñ' | 'ń' | 'ņ' | 'ň' | 'ŉ' | 'Ñ' | 'Ń' | 'Ņ' | 'Ň' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø'
        | 'Ō' | 'Ŏ' | 'Ő' => "o",
        'œ' | 'Œ' => "oe",
        'ŕ' | 'ŗ' | 'ř' | 'Ŕ' | 'Ŗ' | 'Ř' => "r",
        'ś' | 'ŝ' | 'ş' | 'š' | 'ß' | 'Ś' | 'Ŝ' | 'Ş' | 'Š' => "s",
        'ţ' | 'ť' | 'ŧ' | 'Ţ' | 'Ť' | 'Ŧ' => "t",
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' | 'Ù' | 'Ú' | 'Û' | 'Ü' | 'Ũ'
        | 'Ū' | 'Ŭ' | 'Ů' | 'Ű' | 'Ų' => "u",
        'ŵ' | 'Ŵ' => "w",
        'ý' | 'ÿ' | 'ŷ' | 'Ý' | 'Ÿ' | 'Ŷ' => "y",
        'ź' | 'ż' | 'ž' | 'Ź' | 'Ż' | 'Ž' => "z",
        other => {
            // Not an accent this table knows: the character is its own base, and
            // the caller lowercases it.
            return match other {
                'A'..='Z' => Box::leak(other.to_lowercase().to_string().into_boxed_str()),
                _ => Box::leak(other.to_string().into_boxed_str()),
            };
            #[allow(unreachable_code)]
            ""
        }
    }
}

fn words(text: &str) -> Vec<String> {
    fold(text)
        .split(' ')
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

fn set(text: &str) -> Vec<String> {
    let mut out = words(text);
    out.sort();
    out.dedup();
    out
}

/// The name a track is credited to (the full credit line when there is one).
pub fn artist_of(track: &Track) -> String {
    let listed = track
        .artists
        .iter()
        .map(|artist| artist.name.as_str())
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    track
        .display_artist
        .clone()
        .filter(|name| !name.is_empty())
        .or_else(|| Some(track.artist.name.clone()).filter(|name| !name.is_empty()))
        .or_else(|| (!listed.is_empty()).then_some(listed))
        .unwrap_or_default()
}

fn overlap_f1(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.;
    }
    let mut hit = 0usize;
    for word in a {
        if b.contains(word) {
            hit += 1;
        }
    }
    if hit == 0 {
        return 0.;
    }
    let precision = hit as f64 / b.len() as f64;
    let recall = hit as f64 / a.len() as f64;
    (2. * precision * recall) / (precision + recall)
}

const VARIANT: [&str; 25] = [
    "cover",
    "live",
    "remix",
    "remixed",
    "mix",
    "edit",
    "version",
    "instrumental",
    "karaoke",
    "tribute",
    "acoustic",
    "demo",
    "reprise",
    "extended",
    "radio",
    "sped",
    "slowed",
    "nightcore",
    "bootleg",
    "mashup",
    "reaction",
    "8d",
    "orchestral",
    "piano",
    "acapella",
];

fn variant_penalty(title: &[String], asked: &[String]) -> f64 {
    let mut extra = 0;
    for word in title {
        if VARIANT.contains(&word.as_str()) && !asked.contains(word) {
            extra += 1;
        }
    }
    if extra == 0 {
        1.
    } else {
        0.72_f64.powi(extra.min(3))
    }
}

/// How much of `query` a track's title and artist account for, 0..1 — half
/// coverage, half the title's F1 against what the artist's name does not explain.
pub fn match_score(query: &str, track: &Track) -> f64 {
    let asked = set(query);
    if asked.is_empty() {
        return 0.;
    }
    let title_words = words(&track.title);
    let artist = set(&artist_of(track));
    let title_asked: Vec<String> = asked
        .iter()
        .filter(|word| !artist.contains(word))
        .cloned()
        .collect();
    let title_score = if title_asked.is_empty() {
        1.
    } else {
        overlap_f1(&title_asked, &title_words)
    };
    let mut covered = 0usize;
    for word in &asked {
        if title_words.contains(word) || artist.contains(word) {
            covered += 1;
        }
    }
    let coverage = covered as f64 / asked.len() as f64;
    let score = (coverage + title_score) / 2.;
    (score * variant_penalty(&title_words, &asked)).clamp(0., 1.)
}

const COSMETIC: [&str; 14] = [
    "official",
    "officiel",
    "officielle",
    "clip",
    "video",
    "audio",
    "lyrics",
    "paroles",
    "visualizer",
    "visualiser",
    "hd",
    "hq",
    "4k",
    "mv",
];

/// A title without its cosmetic decorations, so two uploads of the same
/// recording compare equal. A LIVE or REMIX marker is NOT removed: it names
/// another recording on purpose.
fn canonical_title(track: &Track) -> String {
    let raw = track.title.as_str();
    let mut kept = String::with_capacity(raw.len());
    let mut inside: Option<String> = None;
    for character in raw.chars() {
        match character {
            '(' | '[' => inside = Some(String::new()),
            ')' | ']' => {
                let content = inside.take().unwrap_or_default();
                let folded = words(&content);
                let cosmetic = folded.iter().any(|word| COSMETIC.contains(&word.as_str()));
                let variant = folded.iter().any(|word| VARIANT.contains(&word.as_str()));
                if !(cosmetic && !variant) {
                    kept.push_str(&format!("({content})"));
                } else {
                    kept.push(' ');
                }
            }
            other => match inside.as_mut() {
                Some(content) => content.push(other),
                None => kept.push(other),
            },
        }
    }
    fold(&kept)
}

/// The identity of a recording for de-duplication.
pub fn recording_key(track: &Track) -> String {
    let title = canonical_title(track);
    if title.is_empty() {
        String::new()
    } else {
        format!("{title}\u{0}{}", fold(&artist_of(track)))
    }
}

/// The catalogue's results, library rows first, with the fallback's hits woven
/// into the SAME list: a video only LEADS when it is strictly closer than the
/// best comparable catalogue answer, and duplicates are dropped.
pub fn rank(query: &str, catalogue: &[Track], youtube: &[Track]) -> Vec<Track> {
    let mut library: Vec<Track> = Vec::new();
    let mut rest: Vec<Track> = Vec::new();
    for track in catalogue {
        if track.local {
            library.push(track.clone());
        } else {
            rest.push(track.clone());
        }
    }
    let mut best = 0f64;
    let mut seen: Vec<String> = Vec::new();
    let mut ids: Vec<String> = Vec::new();
    for track in catalogue {
        let score = match_score(query, track);
        if score >= CATALOGUE_MIN && score > best {
            best = score;
        }
        let key = recording_key(track);
        if !key.is_empty() && !seen.contains(&key) {
            seen.push(key);
        }
        if !track.deezer_id.as_str().is_empty() {
            ids.push(track.deezer_id.as_str().to_owned());
        }
    }
    let mut kept: Vec<(f64, Track)> = Vec::new();
    for track in youtube {
        if ids.contains(&track.deezer_id.as_str().to_owned()) {
            continue;
        }
        let key = recording_key(track);
        if !key.is_empty() && seen.contains(&key) {
            continue;
        }
        let score = match_score(query, track);
        if score < YOUTUBE_MIN {
            continue;
        }
        if !key.is_empty() {
            seen.push(key);
        }
        kept.push((score, track.clone()));
    }
    kept.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut out = library;
    let mut tail: Vec<Track> = Vec::new();
    for (score, track) in kept {
        if score > best {
            out.push(track);
        } else {
            tail.push(track);
        }
    }
    out.extend(rest);
    out.extend(tail);
    out
}

/// The ONE artist the query plainly names, shown first and large — or nothing.
pub fn top_result(query: &str, results: &SearchResults) -> Option<nz_model::Artist> {
    let exact = fold(query);
    if exact.is_empty() {
        return None;
    }
    if let Some(named) = results
        .artists
        .iter()
        .take(3)
        .find(|artist| fold(&artist.name) == exact)
    {
        return Some(named.clone());
    }
    let first = results.artists.first()?;
    if exact.len() >= 3 && fold(&first.name).starts_with(&exact) {
        return Some(first.clone());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str, artist: &str) -> Track {
        Track {
            deezer_id: nz_model::TrackId::new("1"),
            title: title.to_owned(),
            artist: nz_model::TrackArtist {
                deezer_id: nz_model::ArtistId::new("9"),
                name: artist.to_owned(),
            },
            ..Track::default()
        }
    }

    /// The JavaScript's own answers, produced by running `lib/genre/rank.js#fold`
    /// under Node on this machine (`node -e` with the module's `fold` and the
    /// inputs below). A port that folds differently ranks differently, and the
    /// order is what a person sees.
    #[test]
    fn fold_matches_the_web_apps_own_output() {
        for (input, expected) in [
            ("Été", "ete"),
            ("AC/DC", "ac dc"),
            ("Motörhead", "motorhead"),
            ("Daft Punk", "daft punk"),
            ("  Kaya   Project ", "kaya project"),
            ("Beyoncé — Halo", "beyonce halo"),
            ("Sigur Rós", "sigur ros"),
            ("!!!", ""),
            ("Ça va?", "ca va"),
            // Not Latin: the JavaScript's `[^a-z0-9]+` drops it, so must ours.
            ("Нирвана", ""),
        ] {
            assert_eq!(fold(input), expected, "fold({input:?})");
        }
    }

    #[test]
    fn a_score_prefers_the_title_that_was_asked_for() {
        // The JavaScript's own numbers for the same two tracks (Node, on this
        // machine): `1` for the plain title, `0.648` for the live one (a 0.72
        // variant penalty on a perfectly covered title).
        let plain = track("Bohemian Rhapsody", "Queen");
        let live = track("Bohemian Rhapsody (Live)", "Queen");
        assert!((match_score("bohemian rhapsody", &plain) - 1.0).abs() < 1e-9);
        assert!((match_score("bohemian rhapsody", &live) - 0.648).abs() < 1e-9);
    }

    #[test]
    fn a_video_only_leads_when_it_is_strictly_closer() {
        let catalogue = vec![track("Kaya", "Bob Marley")];
        let equal = track("Kaya", "Bob Marley");
        let closer = track("Kaya", "Bob Marley");
        let ranked = rank("kaya", &catalogue, &[equal, closer]);
        assert_eq!(ranked.len(), 1, "an identical video is a duplicate");
        let ranked = rank("kaya", &catalogue, &[track("Kaya Project live", "Kaya")]);
        assert_eq!(ranked.len(), 1, "a video under YOUTUBE_MIN is dropped");
    }

    #[test]
    fn the_top_result_is_only_the_name_that_was_typed() {
        let results = SearchResults {
            artists: vec![nz_model::Artist {
                name: "Kaya".to_owned(),
                ..nz_model::Artist::default()
            }],
            ..SearchResults::default()
        };
        assert!(top_result("kaya", &results).is_some());
        assert!(
            top_result("kay", &results).is_some(),
            "a prefix of three letters names it"
        );
        assert!(top_result("ka", &results).is_none(), "two is too short");
        assert!(top_result("someone else", &results).is_none());
    }

    #[test]
    fn a_cosmetic_bracket_is_stripped_and_a_variant_is_kept() {
        let official = track("Halo (Official Video)", "Beyoncé");
        let plain = track("Halo", "Beyoncé");
        assert_eq!(recording_key(&official), recording_key(&plain));
        let live = track("Halo (Live)", "Beyoncé");
        assert_ne!(recording_key(&live), recording_key(&plain));
    }
}

#[cfg(test)]
mod journey {
    use super::*;
    use crate::state::AppState;
    use crate::widgets::card::stub;
    use gpui_kit::{Modifiers, MouseButton, point, px};

    const ME: &str =
        r#"{"user":{"id":"u1","name":"bench","admin":false,"access":{},"sources":[]}}"#;

    fn track(deezer_id: &str, title: &str, artist: &str) -> String {
        format!(
            r#"{{"id":"row-{deezer_id}","deezer_id":"{deezer_id}","title":"{title}",
                "duration":200,"gain":null,
                "artist":{{"deezer_id":"42","name":"{artist}"}},
                "artists":[{{"deezer_id":"42","name":"{artist}","role":"Main"}}],
                "album":{{"deezer_id":"9","title":"Songs from the Inside"}}}}"#
        )
    }

    fn results() -> String {
        format!(
            r#"{{"tracks":[{},{}],
                "albums":[{{"deezer_id":"9","title":"Songs from the Inside",
                    "artist":{{"deezer_id":"42","name":"Kaya Project"}}}}],
                "artists":[{{"deezer_id":"42","name":"Kaya Project","picture":null}}],
                "playlists":[],"podcasts":[]}}"#,
            track("3000001", "Balancing the Dark", "Kaya Project"),
            track("3000002", "Sirens", "Kaya Project"),
        )
    }

    fn stub_started() -> stub::Stub {
        let body = results();
        stub::Stub::start(&[
            ("/api/login", ME),
            ("/api/me", ME),
            ("/api/search", Box::leak(body.into_boxed_str())),
            ("/api/youtube/search", r#"{"tracks":[]}"#),
        ])
    }

    /// A search screen in a window of the acceptance's size, logged in: the
    /// screen is a whole page (it restates the `Neezer` key context), so the
    /// real keymap drives it.
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
            move |_window, cx| {
                Screen::new(
                    Route::Search {
                        query: String::new(),
                    },
                    state,
                    cx,
                )
            }
        });
        let handle = cx.windows()[0];
        cx.simulate_window_resize(handle, gpui_kit::size(px(1280.), px(800.)));
        cx.run_until_parked();
        (state, view, cx, runtime)
    }

    /// Typing a word asks ONCE, for the word — not once per letter; a second
    /// letter is a second settled term; and one request per field is in flight.
    /// D8a's acceptance, counted on the server.
    #[gpui::test]
    async fn typing_a_word_asks_once_for_the_word(cx: &mut gpui::TestAppContext) {
        let stub = stub_started();
        let (_state, _view, cx, runtime) = open(cx, &stub);
        cx.simulate_input("kaya");
        cx.run_until_parked();
        assert_eq!(
            stub.count("/api/search"),
            0,
            "a keystroke never asks: the beat has not elapsed"
        );
        cx.cx.dispatcher.advance_clock(Duration::from_millis(200));
        cx.cx.run_until_parked();
        stub::until(&runtime, cx, "the search's answer", |_| {
            stub.count("/api/search") == 1
        });
        assert_eq!(
            stub.queries("/api/search"),
            vec!["q=kaya".to_owned()],
            "one request, for the whole word"
        );

        cx.simulate_input("s");
        cx.cx.dispatcher.advance_clock(Duration::from_millis(200));
        cx.cx.run_until_parked();
        stub::until(&runtime, cx, "the refined term", |_| {
            stub.count("/api/search") == 2
        });
        assert_eq!(
            stub.queries("/api/search"),
            vec!["q=kaya".to_owned(), "q=kayas".to_owned()]
        );
        runtime.shutdown_background();
        stub.stop();
    }

    /// `Menu` opens the row's menu and `Entrée` runs the item the cursor is on —
    /// the keyboard half of D8a's menu acceptance.
    #[gpui::test]
    async fn the_menu_opens_from_the_keyboard_and_plays(cx: &mut gpui::TestAppContext) {
        let stub = stub_started();
        let (state, view, cx, runtime) = open(cx, &stub);
        cx.simulate_input("kaya");
        cx.cx.dispatcher.advance_clock(Duration::from_millis(200));
        cx.cx.run_until_parked();
        // The ROWS, not the request: a count on the stub says the question left,
        // and `Entrée`/`Menu` below act on the answer
        // (`docs/agents/desktop.md`, D12 *Pitfalls*).
        stub::until(&runtime, cx, "the search's results", |cx| {
            view.read(cx).list.len() > 0
        });
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(
            cx.update(|window, cx| view.read(cx).results_focus.is_focused(window)),
            "Entrée met le clavier sur les résultats"
        );

        cx.simulate_keystrokes("menu");
        cx.run_until_parked();
        assert!(
            cx.update(|_, cx| view.read(cx).menu.is_open()),
            "Menu ouvre le menu de la ligne"
        );
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        let current = cx.update(|_, cx| {
            state
                .read(cx)
                .player
                .current()
                .map(|entry| entry.id.clone())
        });
        assert_eq!(current.as_deref(), Some("row-3000001"));
        runtime.shutdown_background();
        stub.stop();
    }

    /// A right click opens the same menu on the row under the pointer, and the
    /// arrows/`Entrée` run its items: two `Bas` reach `Ajouter à la file`, which
    /// appends ONE title without starting anything — an outcome the list's own
    /// `Entrée` cannot produce.
    #[gpui::test]
    async fn a_right_click_opens_the_row_menu(cx: &mut gpui::TestAppContext) {
        let stub = stub_started();
        let (state, view, cx, runtime) = open(cx, &stub);
        cx.simulate_input("kaya");
        cx.cx.dispatcher.advance_clock(Duration::from_millis(200));
        cx.cx.run_until_parked();
        stub::until(&runtime, cx, "the search's answer", |_| {
            stub.count("/api/search") == 1
        });
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();

        // The list sits under the field, the chips, the tabs and the best result's
        // card, whose heights are the design's business. A LEFT press that starts
        // playback is a press on a row, so it finds the geometry for the right
        // one: the row answers the pointer, whatever the spacing becomes.
        let mut row_y = None;
        for y in (140..700).step_by(20) {
            let at = point(px(600.), px(y as f32));
            cx.simulate_mouse_move(at, None, Modifiers::default());
            cx.simulate_click(at, Modifiers::default());
            cx.run_until_parked();
            if cx.update(|_, cx| state.read(cx).player.current().is_some()) {
                row_y = Some(y);
                break;
            }
        }
        let row_y = row_y.expect("un clic sur une ligne lance la lecture");
        let before = cx.update(|_, cx| {
            (
                state.read(cx).player.len(),
                state
                    .read(cx)
                    .player
                    .current()
                    .map(|entry| entry.id.clone()),
            )
        });

        let at = point(px(600.), px(row_y as f32));
        cx.simulate_mouse_move(at, None, Modifiers::default());
        cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::default());
        cx.run_until_parked();
        assert!(
            cx.update(|_, cx| view.read(cx).menu.is_open()),
            "un clic droit ouvre le menu de la ligne"
        );
        cx.simulate_keystrokes("down");
        cx.simulate_keystrokes("down");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        cx.update(|_, cx| {
            let state = state.read(cx);
            assert_eq!(
                state.player.len(),
                before.0 + 1,
                "Ajouter à la file ajoute un titre"
            );
            assert_eq!(
                state.player.current().map(|entry| entry.id.clone()),
                before.1,
                "et ne change pas ce qui joue"
            );
        });
        runtime.shutdown_background();
        stub.stop();
    }
}
