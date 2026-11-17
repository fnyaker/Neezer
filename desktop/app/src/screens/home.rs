//! Accueil: what the person opens the app on.
//!
//! `routes/Home.svelte` is the reference: the newest releases of the artists
//! this person follows, those artists, a row picked for them, the albums their
//! last titles came from, the mood mixes and the year in review. Every section
//! reads the server's own database (no provider call), so the screen works with
//! Deezer unreachable, and each one paints from the last copy the device saw
//! before it corrects itself.
//!
//! Requests, the SPA's and no more: `GET /releases`, `GET /me/artists`, `GET
//! /discover/prefs` + `GET /recommend?d=`, `GET /moods`, `GET /recap/years`.
//! A section that fails EMPTIES ITSELF, not the page: the home is seven
//! independent doors, and one that is shut is one door (rule 3).

use std::cell::RefCell;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement, Render,
    StatefulInteractiveElement as _, Styled, Subscription, Window, div, px, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon};
use nz_api::ApiError;
use nz_model::{Artist, DiscoverPrefs, MoodCard, Moods, Recommend, Releases, Track};

use crate::nav::{LibraryTab, Route};
use crate::state::AppState;
use crate::theme;
use crate::toast::ToastKind;
use crate::widgets::card;
use crate::widgets::cover::{self, Covers, Kind};
use crate::widgets::track_list::{self, Intent, TrackList as List};
use crate::wiring::events::{self, Sender};

/// How many release sleeves the shelf shows at the narrowest window, and the
/// most it ever shows. The row FILLS the width it is given: six sleeves at
/// 1280, up to twelve on the 2560 panel — a fixed six left the right half of a
/// wide window empty (measured on D17's 2560×1600 captures).
const RELEASE_ROWS: usize = 6;
const RELEASE_ROWS_MAX: usize = 12;
const ARTISTS_SHOWN: usize = 12;
const RECO_ROWS: usize = 6;

/// How many sleeves one shelf row holds at `width`: one per ~190 px, clamped to
/// [`RELEASE_ROWS`]..[`RELEASE_ROWS_MAX`].
fn shelf_count(width: f32) -> usize {
    ((width / 190.0) as usize).clamp(RELEASE_ROWS, RELEASE_ROWS_MAX)
}

enum Msg {
    Releases(Box<Result<Releases, ApiError>>),
    Artists(Box<Result<Vec<Artist>, ApiError>>),
    Prefs(Box<Result<DiscoverPrefs, ApiError>>),
    Recommend(Box<Result<Recommend, ApiError>>),
    Moods(Box<Result<Moods, ApiError>>),
    Mood(String, Box<Result<nz_model::Mood, ApiError>>),
    Year(Option<i32>),
    Cover(String, Option<PathBuf>),
}

/// What a section is showing. `Failed` keeps the cause so the person can read
/// why a door is shut instead of seeing it missing.
#[derive(Default)]
enum Section<T> {
    #[default]
    Loading,
    Ready(T),
    Failed(ApiError),
}

impl<T> Section<T> {
    fn ready(&self) -> Option<&T> {
        match self {
            Section::Ready(value) => Some(value),
            _ => None,
        }
    }
}

pub struct Screen {
    route: Route,
    state: Entity<AppState>,
    releases: Section<Releases>,
    artists: Section<Vec<Artist>>,
    level: f64,
    reco: Section<Recommend>,
    reco_busy: bool,
    moods: Section<Vec<MoodCard>>,
    year: Option<i32>,
    /// The mood whose mix is being fetched, so its tile says so.
    playing_mood: Option<String>,
    list: List,
    covers: Rc<RefCell<Covers>>,
    events: Sender<Msg>,
    _subscriptions: Vec<Subscription>,
}

impl Screen {
    pub fn new(route: Route, state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let (tx, rx) = events::channel::<Msg>();
        let level = state
            .read(cx)
            .cache
            .as_ref()
            .and_then(|cache| cache.kv().get::<f64>("discover.level"))
            .unwrap_or(0.5);
        // The device's copies first: a home already seen paints at once.
        let cached = state.read(cx);
        let releases = match card::cached::<Releases>(cached, "/releases") {
            Some(releases) => Section::Ready(releases),
            None => Section::Loading,
        };
        let artists = match card::cached::<Vec<Artist>>(cached, "/me/artists") {
            Some(artists) => Section::Ready(artists),
            None => Section::Loading,
        };
        let moods = match card::cached::<Moods>(cached, "/moods") {
            Some(moods) => Section::Ready(moods.moods),
            None => Section::Loading,
        };
        let mut screen = Self {
            route,
            state: state.clone(),
            releases,
            artists,
            level,
            reco: Section::Loading,
            reco_busy: false,
            moods,
            year: None,
            playing_mood: None,
            list: List::new(),
            covers: Rc::new(RefCell::new(Covers::new())),
            events: tx,
            _subscriptions: vec![cx.observe(&state, |_, _, cx| cx.notify())],
        };
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

    /// The seven doors, opened at once and independently: a slow one never holds
    /// the others, and a shut one never empties the page.
    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.events.clone();
        let level = self.level;
        runtime.spawn(async move {
            let (releases, artists, moods, prefs, years) = tokio::join!(
                api.releases(),
                api.my_artists(),
                api.moods(),
                api.discover_prefs(),
                api.recap_years(),
            );
            tx.send(Msg::Releases(Box::new(releases)));
            tx.send(Msg::Artists(Box::new(artists.map(|list| list.artists))));
            tx.send(Msg::Moods(Box::new(moods)));
            match prefs {
                Ok(prefs) => {
                    tx.send(Msg::Prefs(Box::new(Ok(prefs))));
                }
                Err(error) => tx.send(Msg::Prefs(Box::new(Err(error)))),
            }
            tx.send(Msg::Year(years.ok().and_then(|years| years.latest)));
            let answer = api.recommend(level, None).await;
            tx.send(Msg::Recommend(Box::new(answer)));
        });
    }

    fn refresh_reco(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        if self.reco_busy {
            return;
        }
        self.reco_busy = true;
        let level = self.level;
        let tx = self.events.clone();
        // A seed makes the server pick another set: the SPA's
        // `String(Date.now() % 1e9)`, here the queue's own length so two
        // suggestions in a row never draw the same seed twice.
        let seed = format!("{}", self.state.read(cx).player.len());
        self.state.read(cx).runtime.spawn(async move {
            let answer = api.recommend(level, Some(&seed)).await;
            tx.send(Msg::Recommend(Box::new(answer)));
        });
        cx.notify();
    }

    fn play_mood(&mut self, mood: &MoodCard, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        if self.playing_mood.as_deref() == Some(mood.id.as_str()) {
            return;
        }
        self.playing_mood = Some(mood.id.clone());
        let id = mood.id.clone();
        let name = mood.name.clone();
        let tx = self.events.clone();
        self.state.read(cx).runtime.spawn(async move {
            let answer = api.mood(&id).await;
            tx.send(Msg::Mood(name, Box::new(answer)));
        });
        cx.notify();
    }

    fn on_msg(&mut self, msg: Msg, cx: &mut Context<Self>) {
        match msg {
            Msg::Releases(result) => {
                self.releases = match *result {
                    Ok(releases) => Section::Ready(releases),
                    Err(error) => Section::Failed(error),
                }
            }
            Msg::Artists(result) => {
                self.artists = match *result {
                    Ok(artists) => Section::Ready(artists),
                    Err(error) => Section::Failed(error),
                }
            }
            Msg::Prefs(result) => {
                if let Ok(prefs) = *result {
                    self.level = prefs.level;
                }
            }
            Msg::Recommend(result) => {
                self.reco_busy = false;
                self.reco = match *result {
                    Ok(recommend) => {
                        let rows: Vec<Track> = recommend.tracks.clone();
                        self.list.set_context(Some(nz_player::Context {
                            kind: "reco".to_owned(),
                            id: None,
                            title: Some(recommend.step.clone()).filter(|step| !step.is_empty()),
                        }));
                        self.list.set_rows(rows);
                        Section::Ready(recommend)
                    }
                    Err(error) => Section::Failed(error),
                }
            }
            Msg::Moods(result) => {
                self.moods = match *result {
                    Ok(moods) => Section::Ready(moods.moods),
                    Err(error) => Section::Failed(error),
                }
            }
            Msg::Mood(name, result) => {
                let asked = self.playing_mood.clone();
                self.playing_mood = None;
                match *result {
                    Ok(mood) => {
                        if mood.tracks.is_empty() {
                            let _ = asked;
                            self.notice(
                                ToastKind::Info,
                                if mood.reason.as_deref() == Some("not-enough") {
                                    "Pas assez de titres mesurés pour ce mix.".to_owned()
                                } else {
                                    "Ce mix est vide pour l'instant.".to_owned()
                                },
                                cx,
                            );
                        } else {
                            let entries: Vec<_> = mood
                                .tracks
                                .iter()
                                .map(crate::state::entry_from_track)
                                .collect();
                            let context = nz_player::Context {
                                kind: "mood".to_owned(),
                                id: Some(mood.id.clone()),
                                title: Some(name),
                            };
                            self.state.update(cx, |state, cx| {
                                state.command(
                                    nz_player::Command::play_queue(entries, 0, Some(context)),
                                    cx,
                                );
                            });
                        }
                    }
                    Err(error) => {
                        let cause = card::cause(&error);
                        self.notice(
                            ToastKind::Error,
                            format!("Impossible de lancer ce mix : {cause}"),
                            cx,
                        );
                    }
                }
            }
            Msg::Year(year) => self.year = year,
            Msg::Cover(url, file) => self.covers.borrow_mut().resolved(&url, file),
        }
        cx.notify();
    }

    fn notice(
        &mut self,
        kind: ToastKind,
        message: impl Into<gpui_kit::SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.state.update(cx, |state, cx| {
            state.toasts.push(kind, message.into());
            cx.notify();
        });
    }

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
        if let Some(releases) = self.releases.ready() {
            for release in releases.releases.iter().take(RELEASE_ROWS_MAX) {
                if let Some(cover) = release.cover.clone() {
                    asked.push(cover);
                }
            }
        }
        if let Some(artists) = self.artists.ready() {
            for artist in artists.iter().take(ARTISTS_SHOWN) {
                if let Some(picture) = artist.picture.clone() {
                    asked.push(picture);
                }
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

    fn on_intent(&mut self, intent: Intent, window: &mut Window, cx: &mut Context<Self>) {
        let page = List::page_rows(window.viewport_size().height);
        match intent {
            Intent::Menu(_) | Intent::Like(_) | Intent::Dislike(_) => {}
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

    fn play_release(&mut self, at: usize, cx: &mut Context<Self>) {
        let Some(releases) = self.releases.ready() else {
            return;
        };
        let entries: Vec<_> = releases
            .tracks
            .iter()
            .map(crate::state::entry_from_track)
            .collect();
        if entries.is_empty() {
            return;
        }
        let context = nz_player::Context::new("releases");
        self.state.update(cx, |state, cx| {
            state.command(
                nz_player::Command::play_queue(entries, at, Some(context)),
                cx,
            );
        });
    }

    fn play_all_releases(&mut self, cx: &mut Context<Self>) {
        self.play_release(0, cx)
    }

    fn open(&mut self, route: Route, cx: &mut Context<Self>) {
        self.state.update(cx, |state, _| state.nav.push(route));
    }
}

impl Render for Screen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div()
            .id("home-scroll")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .font_family(theme::font(cx))
            .text_color(theme::text(cx))
            // The shell's own context (`app.rs`), restated so a screen is a
            // whole page on its own: a test that renders it without the shell
            // still gets the keymap's bindings, and no second context name can
            // drift from the shell's.
            .key_context("Neezer");

        let visible = self.list.visible();
        self.covers_for_visible(visible, cx);
        // A shelf is as wide as the window (see `shelf_count`).
        let shelf = shelf_count(f32::from(window.viewport_size().width));

        let mut page = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(rems(1.5))
            .p(rems(1.5));

        // -- the hero: who is listening, and what they were doing ------------
        let hello = self
            .state
            .read(cx)
            .user_name()
            .map(|name| format!("Bonsoir, {name}"))
            .unwrap_or_else(|| "Bonsoir".to_owned());
        let resume = self.state.read(cx).player.current().cloned();
        let mut hero = div()
            .flex()
            .items_center()
            .gap(rems(1.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(0.25))
                    .child(
                        div()
                            .text_size(rems(1.75))
                            .font_weight(FontWeight::BOLD)
                            .child(hello),
                    )
                    .child(
                        div()
                            .text_size(rems(0.8125))
                            .text_color(theme::secondary(cx))
                            .child(if resume.is_some() {
                                "Reprenez ce que vous écoutiez, ou laissez-vous surprendre."
                            } else {
                                "Vos artistes, leurs nouveautés, et une sélection pour vous."
                            }),
                    ),
            )
            .child(div().flex_1());
        if let Some(entry) = resume {
            let cover = entry
                .cover
                .as_deref()
                .and_then(|url| cover::file_in(&self.covers, url));
            hero = hero.child(
                div()
                    .id("home-resume")
                    .flex()
                    .items_center()
                    .gap(rems(0.75))
                    .p(rems(0.5))
                    .border_1()
                    .border_color(theme::border(cx))
                    .bg(theme::chrome(cx))
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::selected(cx)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.state.update(cx, |state, cx| {
                            state.command(nz_player::Command::toggle(), cx)
                        });
                    }))
                    .child(cover::view(
                        cover.as_ref(),
                        entry.cover.as_deref(),
                        &entry.title,
                        Kind::Album,
                        px(56.),
                        cx,
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .max_w(rems(18.))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(rems(0.8125))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(entry.title.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(rems(0.6875))
                                    .text_color(theme::secondary(cx))
                                    .child(entry.artist_name().to_owned()),
                            ),
                    )
                    .child(icon(IconName::Play).size(rems(1.25))),
            );
        }
        page = page.child(hero);

        // -- nouveautés ------------------------------------------------------
        let mut release_cards: Vec<gpui_kit::AnyElement> = Vec::new();
        match &self.releases {
            Section::Ready(releases) => {
                let mut seen: Vec<String> = Vec::new();
                for track in &releases.tracks {
                    let key = track
                        .release
                        .as_ref()
                        .map(|release| release.deezer_id.as_str().to_owned())
                        .unwrap_or_else(|| track.album.deezer_id.as_str().to_owned());
                    if seen.contains(&key) {
                        continue;
                    }
                    seen.push(key);
                    let title = track
                        .release
                        .as_ref()
                        .map(|release| release.title.clone())
                        .filter(|title| !title.is_empty())
                        .unwrap_or_else(|| track.album.title.clone());
                    let subtitle = track.artist.name.clone();
                    let cover = track
                        .release
                        .as_ref()
                        .and_then(|release| release.cover.clone())
                        .or_else(|| track.album.cover.clone());
                    let file = cover
                        .as_deref()
                        .and_then(|url| cover::file_in(&self.covers, url));
                    let at = releases
                        .tracks
                        .iter()
                        .position(|candidate| candidate.title == track.title)
                        .unwrap_or(0);
                    release_cards.push(card::card(
                        format!("release-{}", track.title).into(),
                        &title,
                        &subtitle,
                        file.as_ref(),
                        cover.as_deref(),
                        Kind::Album,
                        cx,
                        move |this: &mut Self, _, cx| this.play_release(at, cx),
                        |_: &mut Self, _, _| {},
                        Some(
                            move |this: &mut Self, _: &mut Window, cx: &mut Context<Self>| {
                                this.play_release(at, cx)
                            },
                        ),
                    ));
                    if release_cards.len() >= shelf {
                        break;
                    }
                }
                if !release_cards.is_empty() {
                    let unseen = releases.unseen;
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
                                            .child(format!(
                                                "Nouveautés · {} titres",
                                                releases.tracks.len()
                                            )),
                                    )
                                    .when(unseen > 0, |row| {
                                        row.child(
                                            div()
                                                .px(rems(0.375))
                                                .bg(theme::accent(cx))
                                                .text_size(rems(0.5625))
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(theme::shell(cx))
                                                .child(format!("{unseen} nouvelles")),
                                        )
                                    })
                                    .child(div().flex_1())
                                    .child(
                                        button(
                                            "home-play-new",
                                            "Tout lire",
                                            ButtonVariant::Outline,
                                            cx,
                                        )
                                        .on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.play_all_releases(cx)
                                            }),
                                        ),
                                    )
                                    .child(card::chip(
                                        "home-all-releases".into(),
                                        "Tout afficher",
                                        false,
                                        cx,
                                        |this: &mut Self, _, cx| {
                                            this.open(Route::Releases, cx);
                                        },
                                    )),
                            )
                            .child(card::shelf(release_cards)),
                    );
                }
            }
            Section::Failed(_) => {}
            Section::Loading => {
                page = page.child(card::skeleton(shelf, px(148.), cx));
            }
        }

        // -- tes artistes ----------------------------------------------------
        if let Some(artists) = self.artists.ready().cloned()
            && !artists.is_empty()
        {
            let cards: Vec<_> = artists
                .iter()
                .take(shelf * 2)
                .map(|artist| {
                    let id = artist.request_id().as_str().to_owned();
                    let picture = artist
                        .picture
                        .as_deref()
                        .and_then(|url| cover::file_in(&self.covers, url));
                    card::card(
                        format!("home-artist-{}", artist.name).into(),
                        &artist.name,
                        artist
                            .tracks
                            .map(|tracks| format!("{tracks} titres"))
                            .unwrap_or_default()
                            .as_str(),
                        picture.as_ref(),
                        artist.picture.as_deref(),
                        Kind::Artist,
                        cx,
                        move |this: &mut Self, _, cx| {
                            this.open(Route::Artist { id: id.clone() }, cx)
                        },
                        |_: &mut Self, _, _| {},
                        None::<fn(&mut Self, &mut Window, &mut Context<Self>)>,
                    )
                })
                .collect();
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
                                    .child("Tes artistes"),
                            )
                            .child(div().flex_1())
                            .child(card::chip(
                                "home-all-artists".into(),
                                "Tout afficher",
                                false,
                                cx,
                                |this: &mut Self, _, cx| {
                                    this.open(
                                        Route::Library {
                                            tab: LibraryTab::Artists,
                                        },
                                        cx,
                                    );
                                },
                            )),
                    )
                    .child(card::shelf(cards)),
            );
        }

        // -- pour toi --------------------------------------------------------
        if self.list.is_empty() && !matches!(self.reco, Section::Ready(_)) {
            // Nothing picked yet: the section says it is coming.
            page = page.child(card::skeleton(RECO_ROWS, px(44.), cx));
        } else if !self.list.is_empty() {
            let step = self
                .reco
                .ready()
                .map(|reco| reco.step.clone())
                .unwrap_or_default();
            let element = track_list::element(
                cx.entity(),
                "home-reco",
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
                                    .child(if step.is_empty() {
                                        "Pour toi".to_owned()
                                    } else {
                                        format!("Pour toi · {step}")
                                    }),
                            )
                            .child(div().flex_1())
                            .child(
                                button(
                                    "home-refresh-reco",
                                    "Autres suggestions",
                                    ButtonVariant::Outline,
                                    cx,
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.refresh_reco(cx))),
                            ),
                    )
                    .child(div().h(rems(15.)).child(element)),
            );
        }

        // -- mixes d'ambiance ------------------------------------------------
        if let Some(moods) = self.moods.ready().cloned() {
            let ready: Vec<MoodCard> = moods.into_iter().filter(|mood| mood.ready).collect();
            if !ready.is_empty() {
                let playing = self.playing_mood.clone();
                let cards: Vec<_> =
                    ready
                        .iter()
                        .map(|mood| {
                            let busy = playing.as_deref() == Some(mood.id.as_str());
                            let copy = mood.clone();
                            let copy_for_play = mood.clone();
                            card::card(
                            format!("mood-{}", mood.id).into(),
                            &mood.name,
                            if busy { "Lancement…" } else { &mood.subtitle },
                            None,
                            None,
                            Kind::Playlist,
                            cx,
                            move |this: &mut Self, _, cx| this.play_mood(&copy, cx),
                            |_: &mut Self, _, _| {},
                            Some(move |this: &mut Self, _: &mut Window, cx: &mut Context<Self>| {
                                this.play_mood(&copy_for_play, cx)
                            }),
                        )
                        })
                        .collect();
                page = page.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rems(0.5))
                        .child(
                            div()
                                .text_size(rems(0.875))
                                .font_weight(FontWeight::BOLD)
                                .child("Mixes d'ambiance"),
                        )
                        .child(card::shelf(cards)),
                );
            }
        }

        // -- the year in review ----------------------------------------------
        if let Some(year) = self.year {
            page = page.child(
                div()
                    .id("home-recap")
                    .flex()
                    .items_center()
                    .gap(rems(0.75))
                    .p(rems(1.))
                    .border_1()
                    .border_color(theme::border(cx))
                    .bg(theme::chrome(cx))
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::selected(cx)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open(Route::Recap { year: Some(year) }, cx)
                    }))
                    .child(
                        icon(IconName::Sparkles)
                            .size(rems(1.25))
                            .text_color(theme::accent(cx)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(rems(0.875))
                                    .font_weight(FontWeight::BOLD)
                                    .child(format!("Votre {year} en musique")),
                            )
                            .child(
                                div()
                                    .text_size(rems(0.6875))
                                    .text_color(theme::secondary(cx))
                                    .child("Les titres, les artistes et les minutes de l'année."),
                            ),
                    ),
            );
        }

        // -- nothing at all --------------------------------------------------
        let empty = self.list.is_empty()
            && self.releases.ready().is_none_or(|r| r.tracks.is_empty())
            && self.artists.ready().is_none_or(|a| a.is_empty())
            && self.moods.ready().is_none_or(|m| m.is_empty());
        if empty {
            let fails = [
                matches!(self.releases, Section::Failed(_)),
                matches!(self.artists, Section::Failed(_)),
                matches!(self.reco, Section::Failed(_)),
                matches!(self.moods, Section::Failed(_)),
            ]
            .iter()
            .filter(|failed| **failed)
            .count();
            page = page.child(if fails >= 2 {
                let cause = match (&self.releases, &self.artists, &self.reco) {
                    (Section::Failed(error), _, _)
                    | (_, Section::Failed(error), _)
                    | (_, _, Section::Failed(error)) => card::cause(error),
                    _ => "Le serveur n'a pas répondu.".into(),
                };
                card::failed(
                    "L'accueil n'a pas pu se charger",
                    cause,
                    cx,
                    |this: &mut Self, _, cx| {
                        this.releases = Section::Loading;
                        this.artists = Section::Loading;
                        this.reco = Section::Loading;
                        this.moods = Section::Loading;
                        this.covers.borrow_mut().retry_failed();
                        this.load(cx);
                    },
                )
                .into_any_element()
            } else {
                card::empty(
                    IconName::House,
                    "Ta bibliothèque est vide pour l'instant",
                    "Cherche un titre ou un artiste et lance la lecture. Ajoute des artistes à tes favoris pour voir ici leurs nouveautés.",
                    cx,
                    Some(
                        button("home-search", "Explorer", ButtonVariant::Primary, cx)
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
                .into_any_element()
            });
        }

        root.child(page).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D17: the shelf fills the window it is given. Six sleeves at 1280 (its
    /// right half was empty at 2560 with a fixed six), twelve on the panel —
    /// the defect the 2560×1600 capture showed and this pins.
    #[test]
    fn the_release_shelf_fills_the_width_it_is_given() {
        assert_eq!(shelf_count(1280.), 6);
        assert_eq!(shelf_count(1600.), 8);
        assert_eq!(shelf_count(2560.), 12);
        assert_eq!(shelf_count(600.), RELEASE_ROWS, "never fewer than six");
        assert_eq!(shelf_count(4000.), RELEASE_ROWS_MAX, "and never more than twelve");
    }
    use super::*;
    use crate::widgets::card::stub;

    const ME: &str =
        r#"{"user":{"id":"u1","name":"bench","admin":false,"access":{},"sources":[]}}"#;
    const RELEASES: &str = r#"{"releases":[],"tracks":[],"unseen":0,"pending":0}"#;
    const ARTISTS: &str = r#"{"artists":[{"deezer_id":"42","name":"Kaya Project","picture":null,
        "tracks":12,"plays":3}]}"#;
    const MOODS: &str = r#"{"moods":[{"id":"calme","name":"Calme","subtitle":"Souffle lent",
        "ready":true,"tracks":40,"cover":null}]}"#;
    const PREFS: &str = r#"{"level":0.5,"steps":[],"energy":[],"mute":[]}"#;
    const RECO: &str = r#"{"level":0.5,"step":"Voisin","tracks":[{"deezer_id":"3000001",
        "title":"Balancing the Dark","duration":200,
        "artist":{"deezer_id":"42","name":"Kaya Project"},
        "album":{"deezer_id":"9","title":"Songs from the Inside"}}],"albums":[],"artists":[]}"#;
    const YEARS: &str = r#"{"years":[],"latest":null,"now_year":2026,"min_year":2020,
        "timezone":"server-local"}"#;

    /// The home asks each of its seven doors ONCE, and no more: the SPA's own
    /// count (`/releases`, `/me/artists`, `/discover/prefs`, `/recommend`,
    /// `/moods`, `/recap/years`) — and it paints the shelf the answers made.
    #[gpui::test]
    async fn the_home_asks_each_section_once(cx: &mut gpui::TestAppContext) {
        let stub = stub::Stub::start(&[
            ("/api/login", ME),
            ("/api/me", ME),
            ("/api/releases", RELEASES),
            ("/api/me/artists", ARTISTS),
            ("/api/moods", MOODS),
            ("/api/discover/prefs", PREFS),
            ("/api/recommend", RECO),
            ("/api/recap/years", YEARS),
        ]);
        let (state, runtime) = stub::logged_in(cx, &stub);
        let (screen, cx) = cx.add_window_view({
            let state = state.clone();
            move |_window, cx| Screen::new(Route::Home, state, cx)
        });
        stub::until(&runtime, cx, "the home's answers", |cx| {
            screen.read(cx).list.len() == 1
        });
        for (path, expected) in [
            ("/api/releases", 1),
            ("/api/me/artists", 1),
            ("/api/moods", 1),
            ("/api/discover/prefs", 1),
            ("/api/recommend", 1),
            ("/api/recap/years", 1),
        ] {
            assert_eq!(stub.count(path), expected, "{path}");
        }
        assert_eq!(
            stub.total(),
            7,
            "six sections plus the login: no fan-out on a screen open"
        );
        runtime.shutdown_background();
        stub.stop();
    }
}
