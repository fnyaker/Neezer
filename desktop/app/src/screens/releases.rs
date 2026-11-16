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
//!
//! Requests: `GET /releases` (or the device's copy first), `GET
//! /discover/prefs` (the level the server holds), `POST /releases/seen` when
//! the feed carries unseen releases, and `/api/flow/styles` only when the genre
//! picker is opened.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement as _, Styled, Subscription, Window, div, px,
    rems,
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
use crate::widgets::track_list::{self, Intent, TrackList as List};
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
    for track in &payload.tracks {
        let Some(reference) = track.release.as_ref() else {
            continue;
        };
        let id = reference.deezer_id.as_str().to_owned();
        if id.is_empty() {
            continue;
        }
        let card = payload
            .releases
            .iter()
            .find(|release| release.deezer_id.as_str() == id);
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
    for track in &payload.tracks {
        let Some(reference) = track.release.as_ref() else {
            continue;
        };
        let id = reference.deezer_id.as_str().to_owned();
        if id.is_empty() {
            continue;
        }
        let at = match out.iter().position(|group| group.id == id) {
            Some(at) => at,
            None => {
                let meta = payload
                    .releases
                    .iter()
                    .find(|release| release.deezer_id.as_str() == id)
                    .cloned()
                    .unwrap_or_else(|| release_from_ref(reference));
                out.push(Group {
                    id,
                    meta,
                    artist: track
                        .display_artist
                        .clone()
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| track.artist.name.clone()),
                    tracks: Vec::new(),
                    is_new: false,
                });
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

pub struct Screen {
    route: Route,
    state: Entity<AppState>,
    feed: Phase<Releases>,
    /// The count this visit announced, read before `/releases/seen` was posted.
    unseen: u32,
    /// « Toutes les sorties » unfolded.
    shown_all: bool,
    /// The release whose titles are folded out.
    opened: Option<String>,
    list: List,
    /// A paint asked for the page again (the failure's « Réessayer »).
    reload: bool,
    covers: Rc<RefCell<Covers>>,
    events: Sender<Msg>,
    _subscriptions: Vec<Subscription>,
}

impl Screen {
    pub fn new(route: Route, state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let (tx, rx) = events::channel::<Msg>();
        let cached = card::cached::<Releases>(state.read(cx), "/releases");
        let feed = match cached {
            Some(releases) => Phase::Ready(releases),
            None => Phase::Loading,
        };
        let mut screen = Self {
            route,
            state: state.clone(),
            feed,
            unseen: 0,
            shown_all: false,
            opened: None,
            // The folded-out titles are a release's own order: a number, no
            // sleeve (the release's sleeve is right above them).
            list: List::new().numbered(true).show_cover(false),
            reload: false,
            covers: Rc::new(RefCell::new(Covers::new())),
            events: tx,
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
                    self.feed = Phase::Ready(releases.clone());
                    if unseen > 0 {
                        self.mark_seen(cx);
                    }
                    // With no shelf title at all, the per-release list is the
                    // only thing worth showing: open it (`Releases.svelte`).
                    let shelves = build_shelves(&releases);
                    if shelves.iter().all(|shelf| shelf.tracks.is_empty())
                        && !groups(&releases).is_empty()
                    {
                        self.shown_all = true;
                    }
                }
                Err(error) => {
                    if !matches!(self.feed, Phase::Ready(_)) {
                        self.feed = Phase::Failed(error);
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

    /// The sleeves this page draws: the shelves' mosaics, the release rows, the
    /// open group's titles and the Flow's « à suivre ».
    fn want_covers(&mut self, cx: &mut Context<Self>) {
        let mut asked: Vec<String> = Vec::new();
        if let Some(releases) = self.feed.ready() {
            let shelves = build_shelves(releases);
            for shelf in &shelves {
                for release in shelf.releases.iter().take(12) {
                    if let Some(cover) = release.cover.clone() {
                        asked.push(cover);
                    }
                }
            }
            for group in groups(releases) {
                if let Some(cover) = group.meta.cover.clone() {
                    asked.push(cover);
                }
                if self.opened.as_deref() == Some(group.id.as_str()) {
                    for track in &group.tracks {
                        if let Some(cover) = track.album.cover.clone() {
                            asked.push(cover);
                        }
                    }
                }
            }
            for release in releases_without_tracks(releases) {
                if let Some(cover) = release.cover.clone() {
                    asked.push(cover);
                }
            }
        }
        if let Some(entry) = self.state.read(cx).player.next_entry()
            && let Some(cover) = entry.cover.clone()
        {
            asked.push(cover);
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
        let Some(releases) = self.feed.ready().cloned() else {
            return;
        };
        let rows: Vec<Track> = groups(&releases)
            .into_iter()
            .flat_map(|group| group.tracks)
            .collect();
        let context = PlayContext::new("releases");
        self.play(&rows, 0, context, cx);
    }

    fn toggle_group(&mut self, group: &Group, cx: &mut Context<Self>) {
        if self.opened.as_deref() == Some(group.id.as_str()) {
            self.opened = None;
            self.list.clear();
        } else {
            self.opened = Some(group.id.clone());
            self.list.set_context(Some(PlayContext {
                kind: "album".to_owned(),
                id: Some(group.id.clone()),
                title: None,
            }));
            self.list.set_rows(group.tracks.clone());
        }
        cx.notify();
    }

    fn on_intent(&mut self, intent: Intent, window: &mut Window, cx: &mut Context<Self>) {
        let page = List::page_rows(window.viewport_size().height);
        match intent {
            Intent::Menu(at) => {
                if let Some(index) = at.or_else(|| self.list.selected()) {
                    self.list.select(index);
                }
            }
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
}

impl Render for Screen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.want_covers(cx);
        let root = div()
            .id("releases-scroll")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .min_w(px(0.))
            .min_h(px(0.))
            .font_family(theme::font(cx))
            .text_color(theme::text(cx))
            .key_context("Neezer");

        let mut page = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .w_full()
            .max_w(px(1800.))
            .gap(rems(1.25))
            .p(rems(1.5));

        page = page.child(
            div()
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
                ),
        );

        // The Flow: the desktop's door to a station that never runs dry.
        page = page.child(flow::panel(&self.state, &self.covers, cx));

        // The payload is moved out for the paint and put back: cloning a
        // 117 kB feed on every notify would be the page's own cost (rule 4).
        let feed = std::mem::replace(&mut self.feed, Phase::Loading);
        match &feed {
            Phase::Loading => {
                page = page.child(card::skeleton(6, px(148.), cx));
            }
            Phase::Failed(error) => {
                page = page.child(card::failed(
                    "Les nouveautés n'ont pas pu être chargées",
                    card::cause(error),
                    cx,
                    |this: &mut Self, _, cx| {
                        this.reload = true;
                        cx.notify();
                    },
                ));
            }
            Phase::Ready(releases) => {
                page = self.ready_page(page, releases, window, cx);
            }
        }
        self.feed = feed;
        if std::mem::take(&mut self.reload) {
            self.feed = Phase::Loading;
            self.load(cx);
        }

        root.child(page).into_any_element()
    }
}

impl Screen {
    fn ready_page(
        &mut self,
        mut page: gpui_kit::Div,
        releases: &Releases,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let shelves = build_shelves(releases);
        let rows = groups(releases);
        let rest = releases_without_tracks(releases);
        let total_tracks: usize = rows.iter().map(|group| group.tracks.len()).sum();
        let releases_total = rows.len() + rest.len();

        // The bar: what can be played, the badge THIS visit announced, and the
        // two discovery screens a person reaches from here.
        let mut bar = div().flex().items_center().gap(rems(0.5)).flex_wrap();
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
        page = page.child(bar);

        // The two shelves.
        for (index, shelf) in shelves.iter().enumerate() {
            if shelf.tracks.is_empty() {
                continue;
            }
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
                let file = release
                    .cover
                    .as_deref()
                    .and_then(|url| cover::file_in(&self.covers, url));
                let group = rows
                    .iter()
                    .find(|group| group.id == release.deezer_id.as_str())
                    .cloned();
                let playable = group.clone();
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
            page = page.child(
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
                                button("shelf-play", "Tout lire", ButtonVariant::Outline, cx)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.play_shelf(&shelf_for_play, cx)
                                    })),
                            ),
                    )
                    .child(card::shelf(tiles)),
            );
        }

        // « Toutes les sorties »: folded by default, opened when no shelf holds
        // a title (then it is the only thing worth reading).
        if releases_total > 0 {
            let label = if self.shown_all {
                "Replier".to_owned()
            } else {
                "Afficher".to_owned()
            };
            page = page.child(
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
                                    .child(releases_total.to_string()),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        button("releases-all", label, ButtonVariant::Secondary, cx).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.shown_all = !this.shown_all;
                                cx.notify();
                            }),
                        ),
                    ),
            );
        }
        if self.shown_all {
            if releases.pending > 0 {
                page = page.child(
                    div()
                        .text_size(rems(0.75))
                        .text_color(theme::secondary(cx))
                        .child(format!(
                            "{} sortie{} n'a pas encore ses titres : le serveur les lit en \
                             arrière-plan.",
                            releases.pending,
                            if releases.pending > 1 { "s" } else { "" }
                        )),
                );
            }
            for group in &rows {
                page = self.group_row(page, group, window, cx);
            }
            if !rest.is_empty() {
                let mut tiles: Vec<AnyElement> = Vec::new();
                for release in rest.iter().take(24) {
                    let file = release
                        .cover
                        .as_deref()
                        .and_then(|url| cover::file_in(&self.covers, url));
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
                page = page.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rems(0.5))
                        .child(
                            div()
                                .text_size(rems(0.8125))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::secondary(cx))
                                .child(if rows.is_empty() {
                                    "Sorties"
                                } else {
                                    "Autres sorties"
                                }),
                        )
                        .child(card::shelf(tiles)),
                );
            }
        }

        if total_tracks == 0 && releases_total == 0 {
            page = page.child(card::empty(
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
            ));
        }
        page
    }

    /// One release with its titles: the sleeve, the label, the two actions, and
    /// the windowed list folded out under it.
    fn group_row(
        &mut self,
        page: gpui_kit::Div,
        group: &Group,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let file = group
            .meta
            .cover
            .as_deref()
            .and_then(|url| cover::file_in(&self.covers, url));
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
        if opened && !self.list.is_empty() {
            let element = track_list::element(
                cx.entity(),
                "releases-tracks",
                &mut self.list,
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
            row = row.child(div().h(rems(3.5 * count.min(8) as f32 + 1.)).child(element));
        }
        page.child(row)
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
}
