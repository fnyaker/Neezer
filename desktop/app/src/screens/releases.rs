//! Nouveautés: what the artists this person follows have just released, plus
//! the Flow — the desktop's discovery hub.
//!
//! `routes/Releases.svelte`, `components/ReleaseCard.svelte`,
//! `routes/ReleaseShelf.svelte` and `FlowControls.svelte` are the reference:
//!
//! - **Two shelves, then the per-release list.** `close_to` splits the flat
//!   `tracks` list: « Tes artistes » is the person's own (favourites and the
//!   artists they listen to), « Découverte » the releases the provider says are
//!   close to a favourite. A release is counted once, both sides keep the API's
//!   newest-first order, and a card's absence from `releases` never hides a
//!   title.
//! - **Opening the page marks it seen.** `/releases/seen` is posted once per
//!   fetched feed, and the badge shows the count THIS visit announced — a later
//!   back navigation does not call the same releases new twice.
//! - **The shelves are read from the payload, not from the network.** A card's
//!   click costs 0 requests here: the SPA opens a read-only shelf page, and this
//!   screen has no route for one (`nav.rs` is frozen after D7), so the shelf
//!   expands in place and shows the same releases and titles.
//! - **« Toutes les sorties » is the DEFAULT view, and the whole page is ONE
//!   windowed list.** The page used to build every release row and its sleeve
//!   on every frame: with a few thousand releases that is the whole tree
//!   rebuilt per frame, and the scroll stalled. Now `gpui`'s variable-height
//!   `List` holds the page (the head, the Flow, each shelf, one row per release,
//!   the cards without titles) and builds ONLY the items in the window plus its
//!   overdraw: the height of a row that changed (a release folded open, the
//!   genre picker) is re-measured, nothing is guessed. The payload is regrouped
//!   ONCE per feed (`Model`), not once per paint, and a sleeve is asked for only
//!   when the paint just built the row that shows it.
//!
//! Requests: `GET /releases` (or the device's copy first), `GET
//! /discover/prefs` (the level the server holds), `POST /releases/seen` when
//! the feed carries unseen releases, and `/api/flow/styles` only when the genre
//! picker is opened.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, Entity, FontWeight, InteractiveElement as _, IntoElement,
    KeystrokeEvent, ListAlignment, ListState, ParentElement, Render, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, Window, div, list, px, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon};
use nz_api::ApiError;
use nz_model::{Release, ReleaseRef, Releases, Track};
use nz_player::Context as PlayContext;

use crate::nav::Route;
use crate::state::AppState;
use crate::theme;
use crate::toast::ToastKind;
use crate::widgets::card::{self, Phase};
use crate::widgets::cover::{self, Covers, Kind};
use crate::widgets::flow;
use crate::widgets::page_scroll;
use crate::widgets::track_list::{self, Intent, TrackList as Tracks};
use crate::wiring::events::{self, Sender};

/// How many sleeves a shelf card's mosaic shows.
const MOSAIC: usize = 4;

enum Msg {
    Feed(Box<Result<Releases, ApiError>>),
    Prefs(Box<Result<nz_model::DiscoverPrefs, ApiError>>),
    Seen(Box<Result<(), ApiError>>),
    Cover(String, Option<PathBuf>),
}

/// One shelf: its releases (newest first), the titles they came with, and — for
/// the discovery side — the favourites those releases are close to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shelf {
    pub releases: Vec<Release>,
    pub tracks: Vec<Track>,
    pub close_names: Vec<String>,
}

/// One release with the titles the payload brought for it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Group {
    pub id: String,
    pub meta: Release,
    pub artist: String,
    pub tracks: Vec<Track>,
    pub is_new: bool,
}

/// `routes/Releases.svelte#buildReleaseShelves`: the cards of the payload are
/// the authority on `close_to`, not the track's own release stub.
pub fn build_shelves(payload: &Releases) -> [Shelf; 2] {
    let mut artists = Shelf::default();
    let mut discovery = Shelf::default();
    let mut seen_releases: [HashSet<String>; 2] = [HashSet::new(), HashSet::new()];
    let mut seen_tracks: [HashSet<String>; 2] = [HashSet::new(), HashSet::new()];
    // One index, not one scan per title: the payload can list thousands of
    // releases, and `contains` inside the loop is what made this O(n²).
    let cards: HashMap<&str, &Release> = payload
        .releases
        .iter()
        .map(|release| (release.deezer_id.as_str(), release))
        .collect();
    for track in &payload.tracks {
        let Some(reference) = track.release.as_ref() else {
            continue;
        };
        let id = reference.deezer_id.as_str().to_owned();
        if id.is_empty() {
            continue;
        }
        let card = cards.get(id.as_str()).copied();
        let side = if card.is_some_and(|card| card.close_to.is_some()) {
            1
        } else {
            0
        };
        let track_id = track.deezer_id.as_str().to_owned();
        if !track_id.is_empty() && !seen_tracks[side].insert(track_id) {
            continue;
        }
        let (shelf, seen) = if side == 0 {
            (&mut artists, &mut seen_releases[0])
        } else {
            (&mut discovery, &mut seen_releases[1])
        };
        shelf.tracks.push(track.clone());
        if seen.insert(id.clone()) {
            shelf.releases.push(match card {
                Some(card) => card.clone(),
                None => release_from_ref(reference),
            });
        }
    }
    let mut names: Vec<String> = Vec::new();
    for release in &discovery.releases {
        if let Some(name) = release.close_to.clone().filter(|name| !name.is_empty())
            && !names.contains(&name)
        {
            names.push(name);
        }
    }
    discovery.close_names = names;
    [artists, discovery]
}

/// A release row built from a track's own stub when the payload's `releases`
/// does not list it.
fn release_from_ref(reference: &ReleaseRef) -> Release {
    Release {
        deezer_id: reference.deezer_id.clone(),
        title: reference.title.clone(),
        cover: reference.cover.clone(),
        kind: reference.kind.clone(),
        nb_tracks: reference.nb_tracks,
        released: reference.released.clone(),
        ..Release::default()
    }
}

/// The payload's titles regrouped by release, newest first (the API's order),
/// each release's titles in their own order (`groupReleases`).
pub fn groups(payload: &Releases) -> Vec<Group> {
    let mut out: Vec<Group> = Vec::new();
    // The release card of each id, indexed once: `groups` runs on every feed,
    // and a scan per title is O(n²) on a few thousand releases.
    let cards: HashMap<&str, &Release> = payload
        .releases
        .iter()
        .map(|release| (release.deezer_id.as_str(), release))
        .collect();
    let mut index: HashMap<String, usize> = HashMap::new();
    for track in &payload.tracks {
        let Some(reference) = track.release.as_ref() else {
            continue;
        };
        let id = reference.deezer_id.as_str().to_owned();
        if id.is_empty() {
            continue;
        }
        let at = match index.get(&id) {
            Some(at) => *at,
            None => {
                let meta = cards
                    .get(id.as_str())
                    .map(|card| (*card).clone())
                    .unwrap_or_else(|| release_from_ref(reference));
                out.push(Group {
                    id: id.clone(),
                    meta,
                    artist: track
                        .display_artist
                        .clone()
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| track.artist.name.clone()),
                    tracks: Vec::new(),
                    is_new: false,
                });
                index.insert(id, out.len() - 1);
                out.len() - 1
            }
        };
        let group = &mut out[at];
        group.tracks.push(track.clone());
        if track.new == Some(true) {
            group.is_new = true;
        }
    }
    out
}

/// The releases the payload lists but holds no title for yet (the watcher has
/// not read their tracklist): they are shown as plain cards.
pub fn releases_without_tracks(payload: &Releases) -> Vec<Release> {
    let listed: HashSet<&str> = payload
        .tracks
        .iter()
        .filter_map(|track| {
            track
                .release
                .as_ref()
                .map(|release| release.deezer_id.as_str())
        })
        .collect();
    payload
        .releases
        .iter()
        .filter(|release| !listed.contains(release.deezer_id.as_str()))
        .cloned()
        .collect()
}

/// The subtitle a release card wears: the artist (and who it is close to), the
/// kind, the date (`Releases.svelte#card`).
pub fn release_subtitle(release: &Release) -> String {
    let mut parts: Vec<String> = Vec::new();
    let who = match (&release.close_to, release.artist.name.is_empty()) {
        (Some(close), false) => format!("{} · proche de {close}", release.artist.name),
        (Some(close), true) => format!("proche de {close}"),
        (None, false) => release.artist.name.clone(),
        (None, true) => release.subtitle.clone().unwrap_or_default(),
    };
    if !who.is_empty() {
        parts.push(who);
    }
    let kind = match release.kind.as_deref() {
        Some("single") => "Single",
        Some("ep") => "EP",
        _ => "",
    };
    if !kind.is_empty() {
        parts.push(kind.to_owned());
    }
    if let Some(day) = release.released.as_deref().and_then(short_day) {
        parts.push(day);
    }
    parts.join(" · ")
}

/// `2026-11-13` → `13 nov.`: the day a release card prints (`toLocaleDateString`
/// with a French three-letter month).
fn short_day(date: &str) -> Option<String> {
    let mut parts = date.split('-');
    let _year = parts.next()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    const MONTHS: [&str; 12] = [
        "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
        "déc.",
    ];
    let name = MONTHS.get(month.checked_sub(1)? as usize)?;
    Some(format!("{day} {name}"))
}

/// The kind a release card names (`KIND` in the SPA).
fn kind_label(release: &Release) -> &'static str {
    match release.kind.as_deref() {
        Some("album") => "Album",
        Some("single") => "Single",
        Some("ep") => "EP",
        Some("compile") | Some("compilation") => "Compilation",
        _ => "Sortie",
    }
}

/// What the page derives from ONE feed: the two shelves, the per-release
/// groups and the cards that have no title yet.
///
/// It is built once, when the feed lands (and once more on a reload), never
/// per paint. With a few thousand releases the regrouping was the page's own
/// cost: every render walked the whole payload, twice (`shelves` and `groups`),
/// before building a single row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub shelves: [Shelf; 2],
    pub groups: Vec<Group>,
    pub rest: Vec<Release>,
    /// A release id → its group, so a shelf card finds the titles it can play
    /// without walking the groups (a shelf is built when it is on screen).
    pub group_index: HashMap<String, usize>,
}

impl Model {
    pub fn build(payload: &Releases) -> Self {
        let groups = groups(payload);
        let group_index = groups
            .iter()
            .enumerate()
            .map(|(index, group)| (group.id.clone(), index))
            .collect();
        Self {
            shelves: build_shelves(payload),
            groups,
            rest: releases_without_tracks(payload),
            group_index,
        }
    }

    /// The releases the section counts: the ones with titles, and the cards
    /// the watcher has not read yet.
    pub fn release_total(&self) -> usize {
        self.groups.len() + self.rest.len()
    }

    pub fn track_total(&self) -> usize {
        self.groups.iter().map(|group| group.tracks.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.release_total() == 0
    }
}

/// One item of the page's single windowed list.
///
/// The page is heterogeneous — a head, a Flow panel, zero to two shelves, one
/// row per release, a grid — and `gpui`'s `List` measures each item it builds,
/// so no height is guessed here. Only the items the window (plus its overdraw)
/// holds are built on a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    /// The title, the lead line and the bar.
    Head,
    /// The Flow panel.
    Flow,
    /// A shelf of release cards.
    Shelf(usize),
    /// « Toutes les sorties »: its title, its count, its toggle and its hints.
    AllHead,
    /// One release and the titles it came with.
    Group(usize),
    /// The releases the payload lists but holds no title for.
    Rest,
    /// Loading, a failure, or « Rien de neuf ».
    State,
}

/// The page's items for a feed and a fold state. `ready` is false while the
/// feed is loading or failed: the state card is the only row worth building.
pub fn page_rows(model: &Model, ready: bool, shown_all: bool) -> Vec<Row> {
    let mut rows = vec![Row::Head, Row::Flow];
    if !ready {
        rows.push(Row::State);
        return rows;
    }
    for (index, shelf) in model.shelves.iter().enumerate() {
        if !shelf.tracks.is_empty() {
            rows.push(Row::Shelf(index));
        }
    }
    if model.release_total() > 0 {
        rows.push(Row::AllHead);
    }
    if shown_all {
        rows.extend((0..model.groups.len()).map(Row::Group));
        if !model.rest.is_empty() {
            rows.push(Row::Rest);
        }
    }
    if model.is_empty() {
        rows.push(Row::State);
    }
    rows
}

/// How far past the window the list builds, in pixels: one screenful of
/// overdraw so a fast wheel never lands on an empty band.
const OVERDRAW: f32 = 600.;
/// How much of the window `Page suivante` moves (the page's own rule).
const PAGE_SHARE: f32 = 0.8;
/// How far one arrow moves: four lines of the shell's rem type.
const LINE_REMS: f32 = 4.;
/// The height a release row starts at, in pixels: the closed row (an 84 px
/// sleeve, 2 × 0.75 rem of padding and the border) plus the item's 1.25 rem
/// gap. Every item gets it as a size HINT, so the list's travel is right on the
/// first frame; the ones the paint builds replace it with their measured size.
const ROW_HINT: f32 = 130.;

pub struct Screen {
    route: Route,
    state: Entity<AppState>,
    feed: Phase<Releases>,
    /// The feed regrouped once — see [`Model`].
    model: Rc<Model>,
    /// What the list holds, index by index — see [`Row`].
    rows: Rc<Vec<Row>>,
    /// The page's ONE scroller. `ListState` measures every item it builds and
    /// keeps the heights, so a release folded open, the genre picker or a text
    /// that wraps changes only its own row.
    list: ListState,
    /// The sleeves the LAST paint built, so only those are asked for: a row
    /// 100 below the fold costs no request.
    asked: Rc<RefCell<Vec<String>>>,
    /// How many items the paint being measured built (the windowing's count).
    built: usize,
    /// The count this visit announced, read before `/releases/seen` was posted.
    unseen: u32,
    /// « Toutes les sorties » unfolded. The DEFAULT: the full list is the view
    /// a person opens this page for.
    shown_all: bool,
    /// The release whose titles are folded out.
    opened: Option<String>,
    tracks: Tracks,
    /// A paint asked for the page again (the failure's « Réessayer »).
    reload: bool,
    covers: Rc<RefCell<Covers>>,
    events: Sender<Msg>,
    keys: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl Screen {
    pub fn new(route: Route, state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let (tx, rx) = events::channel::<Msg>();
        let cached = card::cached::<Releases>(state.read(cx), "/releases");
        let (feed, model) = match cached {
            Some(releases) => {
                let model = Model::build(&releases);
                (Phase::Ready(releases), Rc::new(model))
            }
            None => (Phase::Loading, Rc::new(Model::default())),
        };
        let ready = feed.ready().is_some();
        // « Toutes les sorties » is the view a person opens this page for: the
        // full list is what the screen shows first.
        let shown_all = true;
        let rows = Rc::new(page_rows(&model, ready, shown_all));
        let list = ListState::new(rows.len(), ListAlignment::Top, px(OVERDRAW))
            .with_uniform_item_height(px(ROW_HINT));
        let keys = Some(Self::own_keys(list.clone(), cx));
        let mut screen = Self {
            route,
            state: state.clone(),
            feed,
            model,
            rows,
            list,
            asked: Rc::new(RefCell::new(Vec::new())),
            built: 0,
            unseen: 0,
            shown_all,
            opened: None,
            // The folded-out titles are a release's own order: a number, no
            // sleeve (the release's sleeve is right above them).
            tracks: Tracks::new().numbered(true).show_cover(false),
            reload: false,
            covers: Rc::new(RefCell::new(Covers::new())),
            events: tx,
            keys,
            _subscriptions: vec![cx.observe(&state, |_, _, cx| cx.notify())],
        };
        // The Flow's service outlives this screen: a station started here keeps
        // choosing while the person reads their history.
        flow::ensure(&state, cx);
        screen.drain(rx, cx);
        screen.load(cx);
        track_list::own(cx, |this: &mut Self, intent, window, cx| {
            this.on_intent(intent, window, cx)
        });
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

    /// The feed and the level, once each. The copy of `/releases` the device
    /// holds already painted in `new`.
    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.events.clone();
        if !matches!(self.feed, Phase::Ready(_)) {
            self.feed = Phase::Loading;
        }
        runtime.spawn(async move {
            tx.send(Msg::Feed(Box::new(api.releases().await)));
            tx.send(Msg::Prefs(Box::new(api.discover_prefs().await)));
        });
    }

    fn on_msg(&mut self, msg: Msg, cx: &mut Context<Self>) {
        match msg {
            Msg::Feed(result) => match *result {
                Ok(releases) => {
                    // The count THIS visit announces; the shared feed's own
                    // `unseen` is cleared by the POST, so a back navigation does
                    // not call the same releases new twice.
                    self.unseen = self.unseen.max(releases.unseen);
                    let unseen = releases.unseen;
                    // The payload is regrouped ONCE, here: a paint reads
                    // `self.model`, it never walks the feed.
                    self.model = Rc::new(Model::build(&releases));
                    self.feed = Phase::Ready(releases);
                    self.rebuild_rows();
                    if unseen > 0 {
                        self.mark_seen(cx);
                    }
                }
                Err(error) => {
                    if !matches!(self.feed, Phase::Ready(_)) {
                        self.feed = Phase::Failed(error);
                        self.rebuild_rows();
                    }
                }
            },
            Msg::Prefs(result) => {
                if let Ok(prefs) = *result {
                    // The server's level is the authority: the device's copy is
                    // what the control paints before this answer.
                    crate::widgets::discovery_slider::adopt(&self.state.clone(), prefs.level, cx);
                }
            }
            Msg::Seen(result) => {
                if let Err(error) = *result {
                    // The badge already went away: say why it may come back.
                    self.notice(
                        ToastKind::Error,
                        format!(
                            "Les nouveautés n'ont pas été marquées vues : {}",
                            card::cause(&error)
                        ),
                        cx,
                    );
                }
            }
            Msg::Cover(url, file) => self.covers.borrow_mut().resolved(&url, file),
        }
        cx.notify();
    }

    /// `POST /releases/seen`: the badge goes away, once per fetched feed.
    fn mark_seen(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.events.clone();
        runtime.spawn(async move {
            tx.send(Msg::Seen(Box::new(api.releases_seen().await)));
        });
    }

    fn notice(
        &mut self,
        kind: ToastKind,
        message: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.state.update(cx, |state, cx| {
            state.toasts.push(kind, message.into());
            cx.notify();
        });
    }

    fn open(&mut self, route: Route, cx: &mut Context<Self>) {
        self.state.update(cx, |state, _| state.nav.push(route));
    }

    /// Rebuild the item model, and tell the list its count changed.
    ///
    /// `reset` is what makes the list drop the heights it cached for the old
    /// items; the scroll goes back to the top, which is what a fold/unfold and
    /// a fresh feed do in every list of the app.
    fn rebuild_rows(&mut self) {
        let rows = page_rows(&self.model, self.feed.ready().is_some(), self.shown_all);
        // Every item starts with the height of a CLOSED release row (the
        // thousands of them), so the list's travel is right from the first
        // frame; a row that is built replaces its hint with its measured
        // height, which is what the head, the Flow, a shelf and an opened
        // release need.
        self.list
            .reset_with_uniform_height(rows.len(), px(ROW_HINT));
        self.rows = Rc::new(rows);
    }

    /// Close the books on the LAST paint, then keep alive what it built.
    ///
    /// The sleeves this page can draw are the shelves' mosaics and the release
    /// rows; with a few thousand releases, asking for all of them per frame is
    /// the request storm rule 3 forbids. [`Screen::ask`] asks for a sleeve while
    /// the row that shows it is built (one screenful), and this publishes that
    /// declaration so `begin_frame` frees the images no row declares any more.
    fn sync_covers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Publish what the last paint declared and free what nobody shows.
        self.covers.borrow_mut().begin_frame(window, cx);
        let mut urls = std::mem::take(&mut *self.asked.borrow_mut());
        // The next track's sleeve is the Flow's « à suivre » and the player bar.
        let next = self
            .state
            .read(cx)
            .player
            .next_entry()
            .and_then(|entry| entry.cover.clone());
        if let Some(cover) = next {
            let key = cover.clone();
            let tx = self.events.clone();
            self.covers
                .borrow_mut()
                .want(self.state.read(cx), &cover, move |file| {
                    tx.send(Msg::Cover(key, file))
                });
            urls.push(cover);
        }
        if urls.is_empty() {
            return;
        }
        let mut files = Vec::new();
        {
            let covers = self.covers.borrow();
            for url in &urls {
                if let Some(file) = covers.file(url).cloned() {
                    files.push(file);
                }
            }
        }
        self.covers.borrow_mut().show(files, cx);
    }

    /// A sleeve a row being built shows: it is asked for NOW (at most
    /// [`cover::MAX_IN_FLIGHT`] are in flight) and painted from the file the
    /// device already holds, or from the placeholder.
    fn ask(&mut self, url: Option<&str>, cx: &mut Context<Self>) -> Option<PathBuf> {
        let url = url.filter(|url| !url.is_empty())?;
        self.asked.borrow_mut().push(url.to_owned());
        if let Some(file) = self.covers.borrow().file(url).cloned() {
            return Some(file);
        }
        let key = url.to_owned();
        let tx = self.events.clone();
        self.covers
            .borrow_mut()
            .want(self.state.read(cx), url, move |file| {
                tx.send(Msg::Cover(key, file))
            });
        None
    }

    fn play(&mut self, tracks: &[Track], at: usize, context: PlayContext, cx: &mut Context<Self>) {
        if tracks.is_empty() {
            return;
        }
        let entries: Vec<_> = tracks.iter().map(crate::state::entry_from_track).collect();
        self.state.update(cx, |state, cx| {
            state.command(
                nz_player::Command::play_queue(entries, at, Some(context)),
                cx,
            );
        });
    }

    fn play_group(&mut self, group: &Group, cx: &mut Context<Self>) {
        let context = PlayContext {
            kind: "album".to_owned(),
            id: Some(group.id.clone()),
            title: Some(group.meta.title.clone()).filter(|title| !title.is_empty()),
        };
        self.play(&group.tracks, 0, context, cx);
    }

    fn play_shelf(&mut self, shelf: &Shelf, cx: &mut Context<Self>) {
        let context = PlayContext::new("releases");
        self.play(&shelf.tracks, 0, context, cx);
    }

    fn play_all(&mut self, cx: &mut Context<Self>) {
        // The groups are the model's own, built once with the feed: "tout
        // lire" never walks the payload.
        let rows: Vec<Track> = self
            .model
            .groups
            .iter()
            .flat_map(|group| group.tracks.iter().cloned())
            .collect();
        let context = PlayContext::new("releases");
        self.play(&rows, 0, context, cx);
    }

    fn toggle_group(&mut self, group: &Group, cx: &mut Context<Self>) {
        if self.opened.as_deref() == Some(group.id.as_str()) {
            self.opened = None;
            self.tracks.clear();
        } else {
            self.opened = Some(group.id.clone());
            self.tracks.set_context(Some(PlayContext {
                kind: "album".to_owned(),
                id: Some(group.id.clone()),
                title: None,
            }));
            self.tracks.set_rows(group.tracks.clone());
        }
        // Only the row that folded changed height: the list re-measures it and
        // keeps every other cached height (and the scroll position).
        let mut at = None;
        for (index, row) in self.rows.iter().enumerate() {
            if let Row::Group(group_index) = row
                && self
                    .model
                    .groups
                    .get(*group_index)
                    .is_some_and(|held| held.id == group.id)
            {
                at = Some(index);
                break;
            }
        }
        if let Some(at) = at {
            self.list.remeasure_items(at..at + 1);
        }
        cx.notify();
    }

    fn on_intent(&mut self, intent: Intent, window: &mut Window, cx: &mut Context<Self>) {
        let page = Tracks::page_rows(window.viewport_size().height);
        match intent {
            Intent::Menu(at) => {
                if let Some(index) = at.or_else(|| self.tracks.selected()) {
                    self.tracks.select(index);
                }
            }
            other => {
                if let Some(request) = self.tracks.apply(other, page) {
                    let command = request.command();
                    self.state
                        .update(cx, |state, cx| state.command(command, cx));
                }
                cx.notify();
            }
        }
    }

    /// The page's keys: `↑`/`↓`, `Page préc./suiv.`, `Début`/`Fin` scroll the
    /// page — what a browser does on the SPA. `Ctrl+↑` is the volume and
    /// `Shift+←` a seek, so only the bare stroke is ours, and the keystroke is
    /// CONSUMED so the shared list actions do not move a row list at the same
    /// time. The list owns its offsets; this interceptor only asks it to move.
    fn own_keys(list: ListState, cx: &mut Context<Self>) -> Subscription {
        let weak = cx.weak_entity();
        App::intercept_keystrokes(&mut *cx, move |event, window: &mut Window, cx: &mut App| {
            let KeystrokeEvent { keystroke, .. } = event;
            if keystroke.modifiers.modified() {
                return;
            }
            let rem = window.rem_size();
            let viewport = window.viewport_size().height;
            // An opened release's rows own the arrows and the page keys, as
            // they did before: only a page with no row list under it takes
            // them, or `j`/`k` and `Page suivante` would move two things.
            let page_owns = weak
                .update(cx, |this, _| this.tracks.is_empty())
                .unwrap_or(false);
            if !page_owns {
                return;
            }
            match keystroke.key.as_str() {
                "up" => list.scroll_by(-rems(LINE_REMS).to_pixels(rem)),
                "down" => list.scroll_by(rems(LINE_REMS).to_pixels(rem)),
                "pageup" => list.scroll_by(-viewport * PAGE_SHARE),
                "pagedown" => list.scroll_by(viewport * PAGE_SHARE),
                "home" => list.scroll_to(gpui_kit::ListOffset::default()),
                "end" => list.scroll_to_end(),
                _ => return,
            }
            let handled = weak.update(cx, |_, cx| cx.notify());
            if handled.is_ok() {
                cx.stop_propagation();
            }
        })
    }
}

impl Render for Screen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The failure's « Réessayer » asks for the page again; the list goes
        // back to the loading card, which is one item, not a feed.
        if std::mem::take(&mut self.reload) {
            self.feed = Phase::Loading;
            self.rebuild_rows();
            self.load(cx);
        }
        // What the LAST paint built is what this one asks for, and the sleeve
        // no row declares any more is freed here — before a single row is built.
        self.sync_covers(window, cx);
        // The paint's own count: how many items this frame BUILT, which is the
        // window (plus its overdraw), never the list.
        self.built = 0;
        let root = div()
            .id("releases-page")
            .size_full()
            .flex()
            .flex_col()
            .min_w(px(0.))
            .min_h(px(0.))
            .font_family(theme::font(cx))
            .text_color(theme::text(cx))
            .key_context("Neezer");

        // ONE scroller for the whole page: the list measures every item it
        // builds and keeps its height, so the head, the Flow, the shelves and
        // thousands of release rows share one scroll — never a page scroller
        // around a list scroller.
        let entity = cx.entity();
        let list_state = self.list.clone();
        let page = list(list_state, move |index, window, cx| {
            entity.update(cx, |this, cx| this.item(index, window, cx))
        })
        .h_full()
        .w_full()
        .max_w(px(1800.))
        .p(rems(1.5));

        root.child(
            div()
                .flex()
                .flex_1()
                .min_h(px(0.))
                .w_full()
                .justify_center()
                .child(page),
        )
        .into_any_element()
    }
}

impl Screen {
    /// One item of the page's single list: built only when the window (plus
    /// its overdraw) holds it, and measured by the list itself.
    fn item(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.built += 1;
        let row = self.rows.get(index).copied().unwrap_or(Row::State);
        let body = match row {
            Row::Head => self.head(cx),
            Row::Flow => flow::panel(&self.state, &self.covers, cx),
            Row::Shelf(index) => {
                let shelf = self.model.shelves.get(index).cloned().unwrap_or_default();
                self.shelf_block(index, &shelf, cx)
            }
            Row::AllHead => self.all_head(cx),
            Row::Group(index) => {
                let group = self.model.groups.get(index).cloned().unwrap_or_default();
                self.group_row(&group, window, cx)
            }
            Row::Rest => self.rest_block(cx),
            Row::State => self.state_card(cx),
        };
        // The page's own gap between sections, as the list has no `gap`.
        div().w_full().pb(rems(1.25)).child(body).into_any_element()
    }

    /// The title, the lead line, and the bar: what can be played, the badge
    /// THIS visit announced, and the two discovery screens reached from here.
    fn head(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let block = div()
            .flex()
            .flex_col()
            .gap(rems(0.25))
            .child(
                div()
                    .text_size(rems(1.5))
                    .font_weight(FontWeight::BOLD)
                    .child("Nouveautés"),
            )
            .child(
                div()
                    .max_w(rems(46.))
                    .text_size(rems(0.8125))
                    .text_color(theme::secondary(cx))
                    .child(
                        "Les sorties récentes de vos artistes favoris, de ceux que vous \
                         écoutez le plus et de quelques artistes très proches d'eux.",
                    ),
            );
        if self.feed.ready().is_none() {
            return block.into_any_element();
        }
        let total_tracks = self.model.track_total();
        let mut bar = div()
            .flex()
            .items_center()
            .gap(rems(0.5))
            .flex_wrap()
            .mt(rems(0.25));
        if total_tracks > 0 {
            bar = bar.child(
                button(
                    "releases-play-all",
                    format!("Tout lire · {total_tracks}"),
                    ButtonVariant::Primary,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| this.play_all(cx))),
            );
        }
        if self.unseen > 0 {
            let label = if self.unseen > 1 {
                format!("{} nouvelles sorties", self.unseen)
            } else {
                "1 nouvelle sortie".to_owned()
            };
            bar = bar.child(
                div()
                    .flex()
                    .items_center()
                    .px(rems(0.625))
                    .py(rems(0.1875))
                    .bg(theme::accent(cx))
                    .text_size(rems(0.625))
                    .font_family(theme::mono(cx))
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme::shell(cx))
                    .child(label),
            );
        }
        // Real buttons: the keymap has no route to these two screens, so a
        // keyboard reaches them the way a mouse does — by activating a control.
        bar = bar
            .child(div().flex_1())
            .child(
                button("releases-recap", "Mon récap", ButtonVariant::Secondary, cx).on_click(
                    cx.listener(|this, _, _, cx| this.open(Route::Recap { year: None }, cx)),
                ),
            )
            .child(
                button(
                    "releases-history",
                    "Historique",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| this.open(Route::History, cx))),
            );
        block.child(bar).into_any_element()
    }

    /// One shelf: its title, its « Tout lire », and the release cards that
    /// carry a title. The cards' sleeves are asked for by this paint only.
    fn shelf_block(&mut self, index: usize, shelf: &Shelf, cx: &mut Context<Self>) -> AnyElement {
        let title = if index == 0 {
            "Tes artistes"
        } else {
            "Découverte"
        };
        let subtitle = if index == 0 {
            "Les sorties de tes favoris et des artistes que tu écoutes.".to_owned()
        } else if shelf.close_names.is_empty() {
            "Des artistes proches de tes favoris.".to_owned()
        } else {
            format!("Des artistes proches de {}.", join_fr(&shelf.close_names))
        };
        let mut tiles: Vec<AnyElement> = Vec::new();
        for release in shelf.releases.iter().take(8) {
            let file = self.ask(release.cover.as_deref(), cx);
            let playable = self
                .model
                .group_index
                .get(release.deezer_id.as_str())
                .and_then(|at| self.model.groups.get(*at))
                .cloned();
            tiles.push(release_tile(
                release,
                file,
                release_subtitle(release),
                cx,
                {
                    let state = self.state.clone();
                    move |route: Route, cx: &mut Context<Self>| {
                        state.update(cx, |state, _| state.nav.push(route));
                    }
                },
                Some(Box::new({
                    let state = self.state.clone();
                    move |cx: &mut Context<Self>| {
                        if let Some(group) = playable.as_ref() {
                            state.update(cx, |state, cx| {
                                let entries: Vec<_> = group
                                    .tracks
                                    .iter()
                                    .map(crate::state::entry_from_track)
                                    .collect();
                                state.command(
                                    nz_player::Command::play_queue(
                                        entries,
                                        0,
                                        Some(PlayContext {
                                            kind: "album".to_owned(),
                                            id: Some(group.id.clone()),
                                            title: None,
                                        }),
                                    ),
                                    cx,
                                );
                            });
                        }
                    }
                })),
            ));
        }
        let shelf_for_play = shelf.clone();
        div()
            .flex()
            .flex_col()
            .gap(rems(0.5))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(
                        div()
                            .text_size(rems(0.875))
                            .font_weight(FontWeight::BOLD)
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(rems(0.6875))
                            .text_color(theme::secondary(cx))
                            .child(subtitle),
                    )
                    .child(div().flex_1())
                    .child(
                        button("shelf-play", "Tout lire", ButtonVariant::Outline, cx).on_click(
                            cx.listener(move |this, _, _, cx| this.play_shelf(&shelf_for_play, cx)),
                        ),
                    ),
            )
            .child(card::shelf(tiles))
            .into_any_element()
    }

    /// « Toutes les sorties »: its count, the fold toggle, and the two hints
    /// that say what the payload does not hold yet.
    fn all_head(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let total = self.model.release_total();
        let label = if self.shown_all {
            "Replier".to_owned()
        } else {
            "Afficher".to_owned()
        };
        let mut block = div().flex().flex_col().gap(rems(0.25)).child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .pt(rems(0.5))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(rems(0.5))
                        .child(
                            div()
                                .text_size(rems(1.))
                                .font_weight(FontWeight::BOLD)
                                .child("Toutes les sorties"),
                        )
                        .child(
                            div()
                                .font_family(theme::mono(cx))
                                .text_size(rems(0.75))
                                .text_color(theme::secondary(cx))
                                .child(total.to_string()),
                        ),
                )
                .child(div().flex_1())
                .child(
                    button("releases-all", label, ButtonVariant::Secondary, cx).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.shown_all = !this.shown_all;
                            // The item count changed: the list drops the
                            // heights it cached for the old model.
                            this.rebuild_rows();
                            cx.notify();
                        }),
                    ),
                ),
        );
        if self.shown_all
            && let Some(releases) = self.feed.ready()
            && releases.pending > 0
        {
            let pending = releases.pending;
            block = block.child(
                div()
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child(format!(
                        "{} sortie{} n'a pas encore ses titres : le serveur les lit en \
                         arrière-plan.",
                        pending,
                        if pending > 1 { "s" } else { "" }
                    )),
            );
        }
        block.into_any_element()
    }

    /// The releases the payload lists but holds no title for: plain cards.
    fn rest_block(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut tiles: Vec<AnyElement> = Vec::new();
        // Only the cards this row shows are copied: the rest of the payload is
        // not this paint's business.
        let rest: Vec<Release> = self.model.rest.iter().take(24).cloned().collect();
        for release in &rest {
            let file = self.ask(release.cover.as_deref(), cx);
            tiles.push(release_tile(
                release,
                file,
                release_subtitle(release),
                cx,
                {
                    let state = self.state.clone();
                    move |route: Route, cx: &mut Context<Self>| {
                        state.update(cx, |state, _| state.nav.push(route));
                    }
                },
                None,
            ));
        }
        div()
            .flex()
            .flex_col()
            .gap(rems(0.5))
            .child(
                div()
                    .text_size(rems(0.8125))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::secondary(cx))
                    .child(if self.model.groups.is_empty() {
                        "Sorties"
                    } else {
                        "Autres sorties"
                    }),
            )
            .child(card::shelf(tiles))
            .into_any_element()
    }

    /// The state card: loading, the failure's cause and « Réessayer », or the
    /// page with nothing in it.
    fn state_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let feed = std::mem::replace(&mut self.feed, Phase::Loading);
        let card = match &feed {
            Phase::Loading => card::skeleton(6, px(148.), cx).into_any_element(),
            Phase::Failed(error) => card::failed(
                "Les nouveautés n'ont pas pu être chargées",
                card::cause(error),
                cx,
                |this: &mut Self, _, cx| {
                    this.reload = true;
                    cx.notify();
                },
            )
            .into_any_element(),
            Phase::Ready(_) => card::empty(
                IconName::Sparkles,
                "Rien de neuf pour l'instant",
                "Ajoutez des artistes à vos favoris : leurs prochaines sorties apparaîtront ici.",
                cx,
                Some(
                    button("releases-search", "Explorer", ButtonVariant::Primary, cx)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open(
                                Route::Search {
                                    query: String::new(),
                                },
                                cx,
                            )
                        }))
                        .into_any_element(),
                ),
            )
            .into_any_element(),
        };
        self.feed = feed;
        card
    }

    /// One release with its titles: the sleeve, the label, the two actions, and
    /// the windowed list folded out under it.
    fn group_row(
        &mut self,
        group: &Group,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let file = self.ask(group.meta.cover.as_deref(), cx);
        let opened = self.opened.as_deref() == Some(group.id.as_str());
        let count = group.tracks.len();
        let copy = group.clone();
        let copy_play = group.clone();
        let mut row = div()
            .id(SharedString::from(format!("release-{}", group.id)))
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
                    .items_start()
                    .gap(rems(0.75))
                    .child(cover::view(
                        file.as_ref(),
                        group.meta.cover.as_deref(),
                        &group.meta.title,
                        Kind::Album,
                        px(84.),
                        cx,
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w(px(0.))
                            .gap(rems(0.125))
                            .child(
                                div()
                                    .font_family(theme::mono(cx))
                                    .text_size(rems(0.625))
                                    .text_color(theme::secondary(cx))
                                    .child(format!(
                                        "{} · {count} titre{}{}",
                                        kind_label(&group.meta),
                                        if count > 1 { "s" } else { "" },
                                        if group.is_new { " · Nouveau" } else { "" }
                                    )),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(rems(1.))
                                    .font_weight(FontWeight::BOLD)
                                    .child(group.meta.title.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(rems(0.75))
                                    .text_color(theme::secondary(cx))
                                    .child(group.artist.clone()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(rems(0.375))
                                    .mt(rems(0.25))
                                    .child(
                                        button("release-play", "Lire", ButtonVariant::Primary, cx)
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.play_group(&copy_play, cx)
                                            })),
                                    )
                                    .child(
                                        button(
                                            "release-open",
                                            if opened {
                                                "Masquer les titres".to_owned()
                                            } else if count > 1 {
                                                "Voir les titres".to_owned()
                                            } else {
                                                "Voir le titre".to_owned()
                                            },
                                            ButtonVariant::Secondary,
                                            cx,
                                        )
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.toggle_group(&copy, cx)
                                            }),
                                        ),
                                    ),
                            ),
                    ),
            );
        if opened && !self.tracks.is_empty() {
            let element = track_list::element(
                cx.entity(),
                "releases-tracks",
                &mut self.tracks,
                &self.state,
                Rc::clone(&self.covers),
                &crate::widgets::context_menu::Menu::new(),
                window,
                cx,
                |this: &mut Self, intent, window, cx| this.on_intent(intent, window, cx),
                |_, _, _, _| None,
                |_: &mut Self, _: crate::widgets::context_menu::Item, _: &mut Context<Self>| {},
                |_: &mut Self, _: &mut Context<Self>| {},
            );
            // A bounded band inside the page: the wheel over the titles moves
            // THEM, and the page only by what they could not take, instead of
            // both by the same notch (`page_scroll::nested_in_list`, the rule
            // #376 states for a `ScrollHandle` page).
            row = row.child(div().h(rems(3.5 * count.min(8) as f32 + 1.)).child(
                page_scroll::nested_in_list(
                    "releases-tracks-nested",
                    &self.list,
                    self.tracks.scroll_handle(),
                    element,
                ),
            ));
        }
        row.into_any_element()
    }
}

/// A tile's own play action, boxed because the caller may have none.
type PlayTile<V> = Box<dyn Fn(&mut Context<V>)>;

/// One release as a card: its sleeve, its title, its subtitle, and a play disc
/// when the payload holds titles for it.
fn release_tile<V: Render>(
    release: &Release,
    file: Option<PathBuf>,
    subtitle: String,
    cx: &mut Context<V>,
    open: impl Fn(Route, &mut Context<V>) + Clone + 'static,
    play: Option<PlayTile<V>>,
) -> AnyElement {
    let id = release.deezer_id.as_str().to_owned();
    let title = release.title.clone();
    let mut tile = div()
        .id(SharedString::from(format!("release-tile-{id}")))
        .w(px(172.))
        .flex()
        .flex_col()
        .gap(rems(0.375))
        .p(rems(0.5))
        .cursor_pointer()
        .hover(|style| style.bg(theme::chrome(cx)))
        .on_click(cx.listener(move |_, _, _, cx| {
            if !id.is_empty() {
                open(Route::Album { id: id.clone() }, cx);
            }
        }))
        .child(
            div()
                .relative()
                .child(cover::view(
                    file.as_ref(),
                    release.cover.as_deref(),
                    &title,
                    Kind::Album,
                    px(156.),
                    cx,
                ))
                .when_some(play, |art, play| {
                    art.child(
                        div()
                            .id(SharedString::from(format!("release-play-{}", title)))
                            .absolute()
                            .right(px(6.))
                            .bottom(px(6.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(rems(1.75))
                            .h(rems(1.75))
                            .bg(theme::chrome(cx))
                            .border_1()
                            .border_color(theme::border(cx))
                            .cursor_pointer()
                            .hover(|style| style.bg(theme::selected(cx)))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.stop_propagation();
                                play(cx);
                            }))
                            .child(icon(IconName::Play).size(rems(0.875))),
                    )
                }),
        )
        .child(
            div()
                .truncate()
                .text_size(rems(0.8125))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(
            div()
                .truncate()
                .text_size(rems(0.6875))
                .text_color(theme::secondary(cx))
                .child(subtitle),
        );
    if release.new {
        tile = tile.child(
            div()
                .text_size(rems(0.5625))
                .font_family(theme::mono(cx))
                .font_weight(FontWeight::BOLD)
                .text_color(theme::accent(cx))
                .child("NOUVEAU"),
        );
    }
    tile.into_any_element()
}

/// "Kaya Project, Bonobo et 2 autres": the French list of the favourites a shelf
/// is close to (`ReleaseShelf.svelte#shelfDescription`).
pub fn join_fr(names: &[String]) -> String {
    let shown: Vec<&str> = names.iter().take(3).map(String::as_str).collect();
    let more = names.len().saturating_sub(shown.len());
    let list = match shown.len() {
        0 => String::new(),
        1 => shown[0].to_owned(),
        count => format!("{} et {}", shown[..count - 1].join(", "), shown[count - 1]),
    };
    if more == 0 {
        return list;
    }
    // The shown names are joined, then the rest counted in one clause: a second
    // "et" would read as two lists (`ReleaseShelf.svelte#shelfDescription`).
    format!(
        "{} et {more} autre{}",
        shown.join(", "),
        if more > 1 { "s" } else { "" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    const ME_JSON: &str =
        r#"{"user":{"id":"u1","name":"bench","admin":false,"access":{},"sources":[]}}"#;
    const PREFS_JSON: &str = r#"{"level":0.4,"step":"Voisin","steps":[],"energy":null,
        "energy_step":"Libre","energy_steps":[]}"#;

    fn feed_json() -> String {
        serde_json::to_string(&payload()).expect("the feed")
    }

    /// The page's own count: three routes, once each — the feed, the level the
    /// server holds, and the « vu » that takes the badge away — and the badge
    /// keeps THIS visit's number.
    #[gpui::test]
    async fn the_screen_asks_three_routes_once_and_keeps_this_visits_badge(
        cx: &mut gpui::TestAppContext,
    ) {
        let feed = feed_json();
        let stub = crate::widgets::card::stub::Stub::start(&[
            ("/api/login", ME_JSON),
            ("/api/me", ME_JSON),
            ("/api/releases", feed.as_str()),
            ("/api/discover/prefs", PREFS_JSON),
            ("/api/releases/seen", "{}"),
        ]);
        let (state, runtime) = crate::widgets::card::stub::logged_in(cx, &stub);
        let (screen, cx) =
            cx.add_window_view(move |_, cx| Screen::new(Route::Releases, state.clone(), cx));
        crate::widgets::card::stub::until(&runtime, cx, "the feed", |cx| {
            screen.read(cx).feed.ready().is_some()
        });
        crate::widgets::card::stub::until(&runtime, cx, "the « vu »", |cx| {
            let _ = cx;
            stub.count("/api/releases/seen") == 1
        });
        assert_eq!(stub.count("/api/releases"), 1);
        assert_eq!(stub.count("/api/discover/prefs"), 1);
        // The sleeves are asked by the FIRST PAINT, so wait for one: the feed
        // landing and the page painting are two moments.
        crate::widgets::card::stub::until(&runtime, cx, "the first sleeve", |_| {
            stub.hits()
                .iter()
                .any(|hit| hit.path.starts_with("/api/cover/"))
        });
        // Everything else on this page is a SLEEVE (`/api/cover/<id>`), asked
        // once each by the visible-first queue: no other route is opened.
        let hits = stub.hits();
        let covers = hits
            .iter()
            .filter(|hit| hit.path.starts_with("/api/cover/"))
            .count();
        let others: Vec<&str> = hits
            .iter()
            .map(|hit| hit.path.as_str())
            .filter(|path| !path.starts_with("/api/cover/"))
            .collect();
        assert_eq!(
            others.len(),
            4,
            "the login and three routes — no fan-out: {others:?}"
        );
        assert!(covers > 0, "the shelves draw sleeves");
        cx.update(|_, cx| {
            assert_eq!(
                screen.read(cx).unseen,
                2,
                "the badge shows the count this visit announced"
            );
        });
        runtime.shutdown_background();
        stub.stop();
    }

    fn track(id: &str, release: &str, _close: bool, new: bool) -> Track {
        Track {
            deezer_id: nz_model::TrackId::new(id),
            title: format!("Titre {id}"),
            duration: 200,
            artist: nz_model::TrackArtist {
                deezer_id: nz_model::ArtistId::new("42"),
                name: "Kaya Project".into(),
            },
            album: nz_model::TrackAlbum {
                deezer_id: nz_model::AlbumId::new(release),
                title: format!("Sortie {release}"),
                cover: Some(format!("/api/cover/{release}")),
            },
            display_artist: Some("Kaya Project".into()),
            new: Some(new),
            release: Some(ReleaseRef {
                deezer_id: nz_model::AlbumId::new(release),
                title: format!("Sortie {release}"),
                cover: Some(format!("/api/cover/{release}")),
                kind: Some("album".into()),
                nb_tracks: Some(2),
                released: Some("2026-11-13".into()),
            }),
            ..Track::default()
        }
    }

    fn payload() -> Releases {
        Releases {
            releases: vec![
                Release {
                    deezer_id: nz_model::AlbumId::new("9"),
                    title: "Sortie 9".into(),
                    artist: nz_model::TrackArtist {
                        deezer_id: nz_model::ArtistId::new("42"),
                        name: "Kaya Project".into(),
                    },
                    cover: Some("/api/cover/9".into()),
                    kind: Some("album".into()),
                    released: Some("2026-11-13".into()),
                    new: true,
                    close_to: None,
                    ..Release::default()
                },
                Release {
                    deezer_id: nz_model::AlbumId::new("10"),
                    title: "Sortie 10".into(),
                    cover: Some("/api/cover/10".into()),
                    close_to: Some("Bonobo".into()),
                    ..Release::default()
                },
                Release {
                    deezer_id: nz_model::AlbumId::new("11"),
                    title: "Sortie 11".into(),
                    ..Release::default()
                },
            ],
            tracks: vec![
                track("1", "9", false, true),
                track("2", "9", false, false),
                track("3", "10", true, false),
                track("1", "9", false, true),
            ],
            unseen: 2,
            pending: 1,
        }
    }

    /// `buildReleaseShelves`: `close_to` splits the sides, a release and a title
    /// are counted once, and both sides keep the API's order.
    #[test]
    fn a_close_release_goes_to_discovery_and_nothing_is_counted_twice() {
        let shelves = build_shelves(&payload());
        assert_eq!(shelves[0].tracks.len(), 2, "the duplicate title is dropped");
        assert_eq!(shelves[0].releases.len(), 1);
        assert_eq!(shelves[1].tracks.len(), 1);
        assert_eq!(shelves[1].releases[0].title, "Sortie 10");
        assert_eq!(shelves[1].close_names, ["Bonobo"]);
    }

    /// A title whose release the payload does not list still makes a shelf row:
    /// the track's own stub is the fallback.
    #[test]
    fn a_title_without_its_card_is_not_lost() {
        let mut payload = payload();
        payload
            .releases
            .retain(|release| release.deezer_id.as_str() != "9");
        let shelves = build_shelves(&payload);
        assert_eq!(shelves[0].releases.len(), 1);
        assert_eq!(shelves[0].releases[0].title, "Sortie 9");
        assert_eq!(
            shelves[0].releases[0].released.as_deref(),
            Some("2026-11-13")
        );
    }

    /// The per-release list keeps one group per release, newest first, flagged
    /// "new" when any of its titles arrived with the last check.
    #[test]
    fn the_release_groups_keep_the_apis_order() {
        let groups = groups(&payload());
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].id, "9");
        assert!(groups[0].is_new);
        assert_eq!(
            groups[0].tracks.len(),
            3,
            "a duplicate title is listed once"
        );
        assert_eq!(groups[0].artist, "Kaya Project");
        assert_eq!(groups[1].id, "10");
        assert!(!groups[1].is_new);
    }

    /// A release the watcher has not read yet is a card, not a group.
    #[test]
    fn a_release_without_titles_stays_a_card() {
        let rest = releases_without_tracks(&payload());
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].deezer_id.as_str(), "11");
    }

    #[test]
    fn a_card_subtitle_names_the_artist_the_kind_and_the_day() {
        let release = payload().releases[0].clone();
        // An album's kind is implied by its card, so only Single and EP are
        // named (`Releases.svelte#card`): the day is what distinguishes a new
        // album from an old one.
        assert_eq!(release_subtitle(&release), "Kaya Project · 13 nov.");
        let close = payload().releases[1].clone();
        assert_eq!(release_subtitle(&close), "proche de Bonobo");
    }

    #[test]
    fn the_french_list_counts_what_it_does_not_name() {
        assert_eq!(join_fr(&[]), "");
        assert_eq!(join_fr(&["A".into()]), "A");
        assert_eq!(join_fr(&["A".into(), "B".into()]), "A et B");
        assert_eq!(
            join_fr(&["A".into(), "B".into(), "C".into(), "D".into()]),
            "A, B, C et 1 autre"
        );
        assert_eq!(
            join_fr(&["A".into(), "B".into(), "C".into(), "D".into(), "E".into()]),
            "A, B, C et 2 autres"
        );
    }

    /// `count` releases, one title each, a sleeve each: the worst realistic
    /// feed (the shipped server caps a payload at 120, a watcher that follows
    /// more artists — or a future server — answers this).
    fn large_payload(count: usize) -> Releases {
        let mut releases = Vec::with_capacity(count);
        let mut tracks = Vec::with_capacity(count);
        for index in 0..count {
            let id = format!("{}", 500_000 + index);
            let mut release = payload().releases[0].clone();
            release.deezer_id = nz_model::AlbumId::new(&id);
            release.title = format!("Sortie {id}");
            release.cover = Some(format!("/api/cover/{id}"));
            releases.push(release);
            tracks.push(track(&format!("t{index}"), &id, false, index % 4 == 0));
        }
        Releases {
            releases,
            tracks,
            unseen: 0,
            pending: 0,
        }
    }

    /// One painted frame, and how many items it built.
    fn draw(cx: &mut gpui::VisualTestContext, screen: &Entity<Screen>) -> usize {
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        cx.update(|_, cx| screen.read(cx).built)
    }

    /// The view is « Toutes les sorties », and the list holds every release —
    /// but ONE paint builds a windowful, the feed is regrouped ONCE, and the
    /// sleeves asked for are the built rows' (measured: 2 000 releases → the
    /// built count below, where the old page built all 2 000 per frame).
    #[gpui::test]
    async fn a_thousand_releases_build_only_the_window_and_regroup_once(
        cx: &mut gpui::TestAppContext,
    ) {
        const COUNT: usize = 2_000;
        // A window above the fold's own geometry: 1280×800 is the cramped case
        // the notes measure every screen at.
        const BUILT_MAX: usize = 40;
        let feed = serde_json::to_string(&large_payload(COUNT)).expect("the feed");
        let stub = crate::widgets::card::stub::Stub::start(&[
            ("/api/login", ME_JSON),
            ("/api/me", ME_JSON),
            ("/api/releases", feed.as_str()),
            ("/api/discover/prefs", PREFS_JSON),
            ("/api/releases/seen", "{}"),
        ]);
        let (state, runtime) = crate::widgets::card::stub::logged_in(cx, &stub);
        let (screen, cx) =
            cx.add_window_view(move |_, cx| Screen::new(Route::Releases, state.clone(), cx));
        cx.simulate_resize(gpui_kit::size(px(1280.), px(800.)));
        crate::widgets::card::stub::until(&runtime, cx, "the feed", |cx| {
            screen.read(cx).feed.ready().is_some()
        });

        // The DEFAULT view is the full list, and every release is an item.
        let (shown, items) = cx.update(|_, cx| {
            let held = screen.read(cx);
            (held.shown_all, held.list.item_count())
        });
        assert!(
            shown,
            "« Toutes les sorties » is the view the page opens on"
        );
        assert!(
            items >= COUNT,
            "the list holds every release: {items} of {COUNT}"
        );
        // ...but a paint BUILDS a windowful, never the list.
        crate::widgets::card::stub::until(&runtime, cx, "the first sleeve", |_| {
            stub.hits()
                .iter()
                .any(|hit| hit.path.starts_with("/api/cover/"))
        });
        let built = draw(cx, &screen);
        assert!(
            built <= BUILT_MAX,
            "one paint built {built} of {items} items (budget {BUILT_MAX})"
        );
        // The model is the SAME allocation after a paint: the payload is
        // regrouped when the feed lands, not when a frame is drawn.
        let model = cx.update(|_, cx| Rc::as_ptr(&screen.read(cx).model));

        let mut per_frame = vec![built];
        for _ in 0..8 {
            cx.simulate_keystrokes("pagedown");
            cx.run_until_parked();
            per_frame.push(draw(cx, &screen));
        }
        assert!(
            per_frame.iter().all(|built| *built <= BUILT_MAX),
            "a scroll frame built {per_frame:?} items (budget {BUILT_MAX})"
        );
        let moved = cx.update(|_, cx| screen.read(cx).list.logical_scroll_top().item_ix);
        assert!(moved > 0, "Page suivante moved the list (at {moved})");
        let after = cx.update(|_, cx| Rc::as_ptr(&screen.read(cx).model));
        assert_eq!(
            model, after,
            "a paint regrouped the payload again: the model must be built once per feed"
        );

        // The sleeves asked for are the built rows', not the list's: ten paints
        // over 2 000 releases is a few screenfuls, not one request per release.
        let covers = stub
            .hits()
            .iter()
            .filter(|hit| hit.path.starts_with("/api/cover/"))
            .count();
        println!(
            "releases: {COUNT} releases → {items} items, {per_frame:?} built per paint, \
             {covers} sleeves asked over {} paints, the list at item {moved} after 8 pages",
            per_frame.len()
        );
        assert!(covers > 0, "the visible rows ask for their sleeve");
        assert!(
            covers <= 10 * COUNT / 100,
            "{covers} sleeves asked for a {COUNT}-release feed"
        );
        runtime.shutdown_background();
        stub.stop();
    }

    /// The fold: the full list by default, the same page without the release
    /// rows when folded, and the count the list is told follows the model.
    #[test]
    fn the_full_view_is_the_default_and_a_fold_drops_the_release_rows() {
        let model = Model::build(&payload());
        let full = page_rows(&model, true, true);
        let folded = page_rows(&model, true, false);
        assert_eq!(full[0], Row::Head);
        assert_eq!(full[1], Row::Flow);
        assert!(full.contains(&Row::AllHead));
        assert_eq!(
            full.iter()
                .filter(|row| matches!(row, Row::Group(_)))
                .count(),
            model.groups.len()
        );
        assert!(full.contains(&Row::Rest));
        assert!(
            !folded.iter().any(|row| matches!(row, Row::Group(_))),
            "folded: no release row"
        );
        assert_eq!(
            folded
                .iter()
                .filter(|row| matches!(row, Row::AllHead))
                .count(),
            1,
            "the section head stays, so the page can be unfolded"
        );
        // Loading and failed feeds are three items, not a feed.
        let loading = page_rows(&Model::default(), false, true);
        assert_eq!(loading, [Row::Head, Row::Flow, Row::State]);
    }
}
