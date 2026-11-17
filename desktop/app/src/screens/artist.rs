//! Artist: the header, the latest releases, the titles, the discography, the
//! rest of the catalogue the library does not hold, and the people page.
//!
//! `routes/Artist.svelte` is the reference. What it gets right and what this
//! screen keeps:
//!
//! - **The page paints from the device** and revalidates (`api.swr`), then asks
//!   the provider for the half the library does not hold — AFTER the page is on
//!   screen, so a person with the artist's records on disk never waits for
//!   Deezer, and a refused `/artist/<id>/on-deezer` is one sentence, never a
//!   failed page (rule 3).
//! - **Two views** (`Aperçu` / `Titres`): the overview is the curated page, the
//!   titles tab is the artist's whole library, fetched the first time the tab is
//!   opened and remembered afterwards — the SPA's lazy load, and the reason a
//!   first visit is three requests and not four.
//! - **The credits tab** is the person page for an artist the library knows
//!   (`/credits/person?name=`), because a performer is often a writer too.
//!
//! Requests: `GET /artist/<id>`, then `GET /artist/<id>/discography` and, when
//! the library holds the artist, `GET /artist/<id>/on-deezer`; `GET
//! /artist/<id>/tracks` when `Titres` opens; `GET /credits/person?name=` when
//! `Crédits` opens; `POST /favorite/artist`; `GET /artist/<id>/radio`.

use std::cell::RefCell;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::{
    Context, Div, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement, Render,
    StatefulInteractiveElement as _, Styled, Subscription, Window, div, px, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon};
use nz_api::ApiError;
use nz_model::{Album, AlbumList, ArtistPage, Discography, PersonCredits, Track, TrackList};

use crate::actions::{DownloadSelection, RemoveDownload};
use crate::nav::{CreditKind, Route};
use crate::state::AppState;
use crate::theme;
use crate::toast::ToastKind;
use crate::widgets::card::{self, Phase};
use crate::widgets::context_menu::{self, Item, Menu};
use crate::widgets::cover::{self, Covers, Kind};
use crate::widgets::download_control::{self, Watch};
use crate::widgets::track_list::{self, Intent, Subtitle, TrackList as List};
use crate::wiring::events::{self, Sender};
use gpui_kit::SharedString;

/// Which half of the artist page is open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Overview,
    Titles,
    Credits,
}

enum Msg {
    Fresh(Box<Result<ArtistPage, ApiError>>),
    Discography(Box<Discography>),
    Outside(Vec<Album>),
    OutsideDown,
    Titles(Box<Result<TrackList, ApiError>>),
    Credits(Box<Result<PersonCredits, ApiError>>),
    Cover(String, Option<PathBuf>),
    Starred(Box<Result<bool, ApiError>>),
    Radio(Box<Result<TrackList, ApiError>>),
    RowFailed(ApiError),
}

pub struct Screen {
    route: Route,
    state: Entity<AppState>,
    id: String,
    page: Phase<ArtistPage>,
    discography: Option<Discography>,
    outside: Option<Vec<Album>>,
    outside_down: bool,
    starred: bool,
    star_busy: bool,
    radio_busy: bool,
    tab: Tab,
    titles_asked: bool,
    list: List,
    credit_list: List,
    credits: Option<PersonCredits>,
    covers: Rc<RefCell<Covers>>,
    /// The downloads worker, as this screen reads it (lot D18).
    downloads: Watch,
    menu: Menu,
    /// The keystroke interceptor that owns the keyboard while a menu is open.
    menu_keys: Option<Subscription>,
    /// Term → answer, so a term typed back is instant and costs no request.
    events: Sender<Msg>,
    _subscriptions: Vec<Subscription>,
}

impl Screen {
    pub fn new(route: Route, state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let id = match &route {
            Route::Artist { id } => id.clone(),
            _ => String::new(),
        };
        let path = format!("/artist/{}", card::encode(&id));
        let page = match card::cached::<ArtistPage>(state.read(cx), &path) {
            Some(page) => Phase::Ready(page),
            None => Phase::Loading,
        };
        let starred = page
            .ready()
            .and_then(|page| page.artist.is_favorite)
            .unwrap_or(false);
        let (tx, rx) = events::channel::<Msg>();
        let mut screen = Self {
            route,
            state: state.clone(),
            id,
            page,
            discography: None,
            outside: None,
            outside_down: false,
            starred,
            star_busy: false,
            radio_busy: false,
            tab: Tab::default(),
            titles_asked: false,
            list: List::new().numbered(true).show_cover(false),
            credit_list: List::new().subtitle(Subtitle::Roles),
            credits: None,
            covers: Rc::new(RefCell::new(Covers::new())),
            downloads: Watch::attach(&state, cx),
            menu: Menu::new(),
            menu_keys: None,
            events: tx,
            _subscriptions: vec![cx.observe(&state, |_, _, cx| cx.notify())],
        };
        screen.apply_page(cx);
        screen.drain(rx, cx);
        screen.bind_download(cx);
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

    fn context(&self) -> Option<nz_player::Context> {
        let page = self.page.ready()?;
        Some(nz_player::Context {
            kind: "artist".to_owned(),
            id: Some(self.id.clone()),
            title: (!page.artist.name.is_empty()).then(|| page.artist.name.clone()),
        })
    }

    fn apply_page(&mut self, cx: &mut Context<Self>) {
        let Some(page) = self.page.ready().cloned() else {
            self.list.clear();
            return;
        };
        self.list.set_context(self.context());
        // The overview is the SPA's: the top titles, not the whole library.
        self.list
            .set_rows(page.top.iter().take(10).cloned().collect::<Vec<Track>>());
        let playing = self
            .state
            .read(cx)
            .player
            .current()
            .and_then(|entry| entry.reference().map(str::to_owned));
        self.list.select_reference(playing.as_deref());
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        let runtime = self.state.read(cx).runtime.clone();
        let id = self.id.clone();
        let tx = self.events.clone();
        runtime.spawn(async move {
            match api.artist(&id).await {
                Ok(page) => {
                    let library = page.library;
                    tx.send(Msg::Fresh(Box::new(Ok(page))));
                    // The discography is what the overview shelves by type; it
                    // never blocks the page.
                    if let Ok(discography) = api.discography(&id).await {
                        tx.send(Msg::Discography(Box::new(discography)));
                    }
                    if library {
                        match api.artist_on_deezer(&id).await {
                            Ok(value) => {
                                let list: AlbumList =
                                    serde_json::from_value(value).unwrap_or_default();
                                tx.send(Msg::Outside(list.albums));
                            }
                            Err(_) => tx.send(Msg::OutsideDown),
                        }
                    }
                }
                Err(error) => tx.send(Msg::Fresh(Box::new(Err(error)))),
            }
        });
    }

    fn load_titles(&mut self, cx: &mut Context<Self>) {
        if self.titles_asked {
            return;
        }
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        self.titles_asked = true;
        let runtime = self.state.read(cx).runtime.clone();
        let id = self.id.clone();
        let tx = self.events.clone();
        runtime.spawn(async move { tx.send(Msg::Titles(Box::new(api.artist_tracks(&id).await))) });
    }

    fn load_credits(&mut self, cx: &mut Context<Self>) {
        if self.credits.is_some() {
            return;
        }
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        let Some(name) = self.page.ready().map(|page| page.artist.name.clone()) else {
            return;
        };
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.events.clone();
        runtime
            .spawn(async move { tx.send(Msg::Credits(Box::new(api.person_credits(&name).await))) });
    }

    fn on_msg(&mut self, msg: Msg, cx: &mut Context<Self>) {
        match msg {
            Msg::Fresh(result) => match *result {
                Ok(page) => {
                    if let Some(starred) = page.artist.is_favorite {
                        self.starred = starred;
                    }
                    self.page = Phase::Ready(page);
                    self.apply_page(cx);
                }
                Err(error) => {
                    if self.page.ready().is_none() {
                        self.page = Phase::Failed(error);
                    }
                }
            },
            Msg::Discography(discography) => self.discography = Some(*discography),
            Msg::Outside(albums) => self.outside = Some(albums),
            Msg::OutsideDown => {
                self.outside = Some(Vec::new());
                self.outside_down = true;
            }
            Msg::Titles(result) => {
                let rows = match *result {
                    Ok(list) => list.tracks,
                    Err(_) => Vec::new(),
                };
                self.list.set_context(self.context());
                self.list.set_rows(rows);
            }
            Msg::Credits(result) => {
                if let Ok(credits) = *result {
                    self.credit_list.set_context(Some(nz_player::Context {
                        kind: "credits".to_owned(),
                        id: Some(credits.name.clone()),
                        title: Some(credits.name.clone()),
                    }));
                    self.credit_list.set_rows(credits.tracks.clone());
                    self.credits = Some(credits);
                }
            }
            Msg::Cover(url, file) => self.covers.borrow_mut().resolved(&url, file),
            Msg::Starred(result) => {
                self.star_busy = false;
                match *result {
                    Ok(on) => self.starred = on,
                    Err(error) => self.notice(
                        ToastKind::Error,
                        format!("Favori non enregistré : {}", card::cause(&error)),
                        cx,
                    ),
                }
            }
            Msg::Radio(result) => {
                self.radio_busy = false;
                match *result {
                    Ok(list) if !list.tracks.is_empty() => {
                        let entries: Vec<_> = list
                            .tracks
                            .iter()
                            .map(crate::state::entry_from_track)
                            .collect();
                        let context = nz_player::Context {
                            kind: "artist-radio".to_owned(),
                            id: Some(self.id.clone()),
                            title: None,
                        };
                        self.command(
                            nz_player::Command::play_queue(entries, 0, Some(context)),
                            cx,
                        );
                    }
                    Ok(_) => {
                        self.notice(ToastKind::Info, "Cette radio est vide pour l'instant.", cx)
                    }
                    Err(error) => {
                        let cause = card::cause(&error);
                        self.notice(ToastKind::Error, format!("Radio impossible : {cause}"), cx)
                    }
                }
            }
            Msg::RowFailed(error) => {
                self.notice(ToastKind::Error, card::cause(&error), cx);
            }
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

    fn command(&mut self, command: nz_player::Command, cx: &mut Context<Self>) {
        self.state
            .update(cx, |state, cx| state.command(command, cx));
    }

    fn play_all(&mut self, cx: &mut Context<Self>) {
        let entries = self.list.entries().to_vec();
        if !entries.is_empty() {
            self.command(
                nz_player::Command::play_queue(entries, 0, self.context()),
                cx,
            );
        }
    }

    fn star(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        if self.star_busy {
            return;
        }
        self.star_busy = true;
        let on = !self.starred;
        let id = self.id.clone();
        let name = self
            .page
            .ready()
            .map(|page| page.artist.name.clone())
            .unwrap_or_default();
        let tx = self.events.clone();
        self.state.read(cx).runtime.spawn(async move {
            let meta = serde_json::json!({ "title": name, "artist": name });
            let answer = api
                .favorite_entity("artist", &id, on, &meta)
                .await
                .map(|_| on);
            tx.send(Msg::Starred(Box::new(answer)));
        });
    }

    fn radio(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        if self.radio_busy {
            return;
        }
        self.radio_busy = true;
        let id = self.id.clone();
        let tx = self.events.clone();
        self.state
            .read(cx)
            .runtime
            .spawn(async move { tx.send(Msg::Radio(Box::new(api.artist_radio(&id).await))) });
    }

    fn row_verdict(&mut self, like: bool, cx: &mut Context<Self>) {
        let Some(reference) = self
            .list
            .selected()
            .and_then(|index| self.list.row(index))
            .map(|track| track.deezer_id.as_str().to_owned())
        else {
            return;
        };
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        let tx = self.events.clone();
        self.state.read(cx).runtime.spawn(async move {
            let answer = match like {
                true => api.favorite(&reference, true).await,
                false => api.dislike(&reference, true).await,
            };
            if let Err(error) = answer {
                tx.send(Msg::RowFailed(error));
            }
        });
        self.notice(
            ToastKind::Info,
            if like {
                "Titre ajouté à vos favoris."
            } else {
                "Titre écarté."
            },
            cx,
        );
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
        if let Some(page) = self.page.ready()
            && let Some(picture) = page.artist.picture.clone()
        {
            asked.push(picture);
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
        let download = self
            .list
            .row(index)
            .map(|track| {
                download_control::track(self.downloads.get(), &download_control::key(track), false)
            })
            .map(|state| context_menu::Download::from(&state))
            .unwrap_or_default();
        let items = context_menu::track_items(has_album, has_artist, download);
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
            Intent::Like(_) => self.row_verdict(true, cx),
            Intent::Dislike(_) => self.row_verdict(false, cx),
            other => {
                if let Some(request) = self.list.apply(other, page) {
                    self.command(request.command(), cx);
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

    // --- downloads (lot D18) ------------------------------------------------

    /// One row's download, from its menu or from `t` / `Maj+T`: the same verb
    /// whichever hand asked (`download_control::row_verb`).
    fn row_download(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(service) = self.downloads.service().cloned() else {
            self.state.update(cx, |state, cx| {
                state.toasts.push(
                    ToastKind::Error,
                    "Les téléchargements ne sont pas disponibles sans stockage local.",
                );
                cx.notify();
            });
            return;
        };
        let Some(track) = self.list.row(index) else {
            return;
        };
        let key = download_control::key(track);
        let title = track.title.clone();
        if let Some(message) =
            download_control::row_verb(&service, self.downloads.get(), &key, &title, false)
        {
            self.state.update(cx, |state, cx| {
                state
                    .toasts
                    .push(ToastKind::Info, SharedString::from(message));
                cx.notify();
            });
        }
    }

    /// `t` asks for the focused row, `Maj+T` takes it back (or asks again): one
    /// handler, because the row's own state decides which verb applies.
    fn bind_download(&mut self, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        let for_ask = weak.clone();
        gpui_kit::App::on_action(cx, move |_: &DownloadSelection, cx: &mut gpui_kit::App| {
            for_ask
                .update(cx, |this, cx| {
                    if let Some(index) = this.list.selected() {
                        this.row_download(index, cx);
                    }
                })
                .ok();
        });
        gpui_kit::App::on_action(cx, move |_: &RemoveDownload, cx: &mut gpui_kit::App| {
            weak.update(cx, |this, cx| {
                if let Some(index) = this.list.selected() {
                    this.row_download(index, cx);
                }
            })
            .ok();
        });
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
                    self.command(request.command(), cx);
                }
            }
            Item::Like => self.row_verdict(true, cx),
            Item::Dislike => self.row_verdict(false, cx),
            Item::Download | Item::RetryDownload | Item::CancelDownload | Item::RemoveDownload => {
                if let Some(index) = index {
                    self.row_download(index, cx);
                }
            }
            Item::Credits => {
                if let Some(id) = index
                    .and_then(|index| self.list.row(index))
                    .map(|track| track.deezer_id.as_str().to_owned())
                {
                    let route = Route::Credits {
                        kind: CreditKind::Track,
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
            Item::GoArtist | Item::Radio | Item::Open => {}
        }
        cx.notify();
    }

    fn open_album(&mut self, album: &Album, cx: &mut Context<Self>) {
        let id = if !album.deezer_id.as_str().is_empty() {
            album.deezer_id.as_str().to_owned()
        } else {
            album
                .id
                .as_ref()
                .map(|id| id.to_string())
                .unwrap_or_default()
        };
        if id.is_empty() {
            return;
        }
        self.state
            .update(cx, |state, _| state.nav.push(Route::Album { id }));
    }

    fn open_artist(&mut self, artist_id: &str, cx: &mut Context<Self>) {
        if artist_id.is_empty() {
            return;
        }
        let id = artist_id.to_owned();
        self.state
            .update(cx, |state, _| state.nav.push(Route::Artist { id }));
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.page = Phase::Loading;
        self.discography = None;
        self.outside = None;
        self.outside_down = false;
        self.titles_asked = false;
        self.credits = None;
        self.covers.borrow_mut().retry_failed();
        self.load(cx);
        cx.notify();
    }
}

impl Render for Screen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div()
            .size_full()
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
        let content = match self.page.ready().cloned() {
            Some(page) => self.ready(page, window, cx),
            None => match &self.page {
                Phase::Failed(error) => {
                    let cause = card::cause(error);
                    card::failed(
                        "Artiste introuvable",
                        cause,
                        cx,
                        |this: &mut Self, _, cx| this.reload(cx),
                    )
                    .into_any_element()
                }
                _ => div()
                    .p(rems(1.5))
                    .child(card::skeleton(6, px(148.), cx))
                    .into_any_element(),
            },
        };
        root.child(content)
    }
}

impl Screen {
    fn ready(
        &mut self,
        page: ArtistPage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let visible = self.list.visible();
        self.covers_for_visible(visible, cx);
        let artist = page.artist.clone();
        let picture = artist
            .picture
            .as_deref()
            .and_then(|url| cover::file_in(&self.covers, url));
        let facts = card::facts(&[
            "Artiste".to_owned(),
            match artist.nb_fan {
                Some(fans) if fans > 0 => format!("{fans} fans"),
                _ => String::new(),
            },
            match page.credited {
                0 => String::new(),
                credited => format!("crédité sur {credited} titres"),
            },
        ]);
        let starred = self.starred;
        let star_busy = self.star_busy;
        let radio_busy = self.radio_busy;
        let tab = self.tab;

        let header = card::hero_band(cx)
            .child(cover::view(
                picture.as_ref(),
                artist.picture.as_deref(),
                &artist.name,
                Kind::Artist,
                card::hero_sleeve(),
                cx,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(0.375))
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .text_size(rems(0.6875))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::secondary(cx))
                            .child(facts),
                    )
                    .child(
                        div()
                            .text_size(rems(2.))
                            .font_weight(FontWeight::BOLD)
                            .truncate()
                            .child(artist.name.clone()),
                    ),
            );

        let tabs = card::chip_row(vec![
            card::chip(
                "artist-overview".into(),
                "Aperçu",
                tab == Tab::Overview,
                cx,
                |this: &mut Self, _, cx| {
                    this.tab = Tab::Overview;
                    this.apply_page(cx);
                    cx.notify();
                },
            ),
            card::chip(
                "artist-titles".into(),
                "Titres",
                tab == Tab::Titles,
                cx,
                |this: &mut Self, _, cx| {
                    this.tab = Tab::Titles;
                    this.load_titles(cx);
                    cx.notify();
                },
            ),
            card::chip(
                "artist-credits".into(),
                "Crédits",
                tab == Tab::Credits,
                cx,
                |this: &mut Self, _, cx| {
                    this.tab = Tab::Credits;
                    this.load_credits(cx);
                    cx.notify();
                },
            ),
        ]);

        let actions = div()
            .flex()
            .items_center()
            .gap(rems(0.5))
            .px(rems(1.))
            .py(rems(0.75))
            .child(
                button("artist-play", "Lire", ButtonVariant::Primary, cx)
                    .on_click(cx.listener(|this, _, _, cx| this.play_all(cx))),
            )
            .child(
                button(
                    "artist-radio",
                    if radio_busy { "Radio…" } else { "Radio" },
                    ButtonVariant::Outline,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| this.radio(cx))),
            )
            .child(
                div()
                    .id("artist-star")
                    .flex()
                    .items_center()
                    .gap(rems(0.375))
                    .h(rems(2.))
                    .px(rems(0.75))
                    .border_1()
                    .border_color(if starred {
                        theme::accent(cx)
                    } else {
                        theme::border(cx)
                    })
                    .text_size(rems(0.75))
                    .text_color(if starred {
                        theme::accent(cx)
                    } else {
                        theme::secondary(cx)
                    })
                    .cursor_pointer()
                    .hover(|style| style.text_color(theme::text(cx)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !star_busy {
                            this.star(cx);
                        }
                    }))
                    .child(icon(IconName::Star).size(rems(0.875)))
                    .child(if starred { "Suivi" } else { "Suivre" }),
            )
            .child(tabs);

        let body: gpui_kit::AnyElement = match tab {
            Tab::Titles => self.titles_body(window, cx).into_any_element(),
            Tab::Credits => self.credits_body(window, cx).into_any_element(),
            Tab::Overview => self.overview_body(&page, window, cx),
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .child(header)
            .child(actions)
            .child(body)
            .into_any_element()
    }

    fn titles_body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        if self.list.is_empty() {
            return div().p(rems(1.)).child(card::empty(
                IconName::Music,
                "Aucun titre dans la bibliothèque",
                "Cet artiste n'a encore aucun fichier sur le serveur.",
                cx,
                None,
            ));
        }
        let element = track_list::element(
            cx.entity(),
            "artist-titles-list",
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
        div().flex_1().min_h_0().px(rems(1.)).child(element)
    }

    fn credits_body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let Some(credits) = self.credits.clone() else {
            return div().p(rems(1.)).child(
                div()
                    .text_size(rems(0.8125))
                    .text_color(theme::secondary(cx))
                    .child("Chargement des crédits…"),
            );
        };
        if credits.tracks.is_empty() {
            return div().p(rems(1.)).child(card::empty(
                IconName::Info,
                "Aucun crédit de composition",
                "La bibliothèque ne crédite cet artiste sur aucun autre titre.",
                cx,
                None,
            ));
        }
        let roles = card::facts(
            &credits
                .roles
                .iter()
                .map(|role| format!("{} {}", role.count, role.label.to_lowercase()))
                .collect::<Vec<_>>(),
        );
        let element = track_list::element(
            cx.entity(),
            "artist-credits-list",
            &mut self.credit_list,
            &self.state,
            Rc::clone(&self.covers),
            &Menu::new(),
            window,
            cx,
            |this: &mut Self, intent, window, cx| this.on_intent(intent, window, cx),
            |_, _, _, _| None,
            |_: &mut Self, _: Item, _: &mut Context<Self>| {},
            |_: &mut Self, _: &mut Context<Self>| {},
        );
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .px(rems(1.))
            .gap(rems(0.5))
            .child(
                div()
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child(roles),
            )
            .child(div().flex_1().min_h_0().child(element))
    }

    fn overview_body(
        &mut self,
        page: &ArtistPage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        // The overview's shelves are a page: it scrolls as one column (the list
        // on `Titres` scrolls itself, which is why the two branches differ).
        let mut body = div()
            .id("artist-overview")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(rems(1.))
            .pb(rems(1.))
            .gap(rems(0.75));

        // The SPA's shelves: latest releases from the discography, the top
        // titles, the albums, then the catalogue's half and the neighbours.
        let latest: Vec<Album> = self
            .discography
            .as_ref()
            .map(|disco| {
                let source = if disco.discography.all.is_empty() {
                    &disco.discography.album
                } else {
                    &disco.discography.all
                };
                source.iter().take(5).cloned().collect()
            })
            .unwrap_or_default();
        if !latest.is_empty() {
            let cards: Vec<_> = latest
                .iter()
                .map(|album| self.album_card(album, cx))
                .collect();
            body = body
                .child(card::Section::new("Dernières sorties").render(cx, |_, _, _| {}))
                .child(card::shelf(cards));
        }

        if !page.top.is_empty() {
            body = body
                .child(card::Section::new("Titres populaires").render(cx, |_, _, _| {}))
                .child(
                    div().flex().flex_col().gap(rems(0.125)).children(
                        page.top
                            .iter()
                            .take(6)
                            .enumerate()
                            .map(|(index, track)| self.top_row(index, track, cx)),
                    ),
                );
        }

        let albums: Vec<Album> = self
            .discography
            .as_ref()
            .map(|disco| disco.discography.album.clone())
            .filter(|albums| !albums.is_empty())
            .unwrap_or_else(|| page.albums.clone());
        if !albums.is_empty() {
            let cards: Vec<_> = albums
                .iter()
                .map(|album| self.album_card(album, cx))
                .collect();
            body = body
                .child(
                    card::Section::new("Albums")
                        .count(albums.len())
                        .render(cx, |_, _, _| {}),
                )
                .child(card::shelf(cards));
        }

        if let Some(singles) = self
            .discography
            .as_ref()
            .map(|disco| disco.discography.single.clone())
            .filter(|singles| !singles.is_empty())
        {
            let cards: Vec<_> = singles
                .iter()
                .map(|album| self.album_card(album, cx))
                .collect();
            body = body
                .child(
                    card::Section::new("Singles et EP")
                        .count(singles.len())
                        .render(cx, |_, _, _| {}),
                )
                .child(card::shelf(cards));
        }

        if let Some(outside) = self.outside.clone()
            && !outside.is_empty()
        {
            let cards: Vec<_> = outside
                .iter()
                .map(|album| self.album_card(album, cx))
                .collect();
            body = body
                .child(
                    card::Section::new("Disponible sur Deezer")
                        .count(outside.len())
                        .render(cx, |_, _, _| {}),
                )
                .child(card::shelf(cards));
        } else if self.outside_down {
            body = body.child(
                div()
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child("Deezer est injoignable : le reste du catalogue n'est pas affiché."),
            );
        }

        if !page.related.is_empty() {
            let cards: Vec<_> = page
                .related
                .iter()
                .map(|related| {
                    let id = related.request_id();
                    let name = related.name.clone();
                    let picture = related
                        .picture
                        .as_deref()
                        .and_then(|url| cover::file_in(&self.covers, url));
                    let url = related.picture.clone();
                    card::card(
                        format!("related-{name}").into(),
                        &name,
                        "Artiste",
                        picture.as_ref(),
                        url.as_deref(),
                        Kind::Artist,
                        cx,
                        move |this: &mut Self, _, cx| this.open_artist(id.as_str(), cx),
                        |_: &mut Self, _, _| {},
                        None::<fn(&mut Self, &mut Window, &mut Context<Self>)>,
                    )
                })
                .collect();
            body = body
                .child(card::Section::new("Artistes similaires").render(cx, |_, _, _| {}))
                .child(card::shelf(cards));
        }

        if let Some(bio) = page.bio.clone().filter(|bio| !bio.trim().is_empty()) {
            body = body
                .child(card::Section::new("Biographie").render(cx, |_, _, _| {}))
                .child(
                    div()
                        .max_w(rems(52.))
                        .text_size(rems(0.8125))
                        .text_color(theme::secondary(cx))
                        .child(bio),
                );
        }

        // The list keeps the keyboard on a page whose shelves are all cards:
        // `Entrée` plays the top titles, `↑`/`↓` move in them.
        if !page.top.is_empty() {
            let element = track_list::element(
                cx.entity(),
                "artist-top-list",
                &mut self.list,
                &self.state,
                Rc::clone(&self.covers),
                &Menu::new(),
                window,
                cx,
                |this: &mut Self, intent, window, cx| this.on_intent(intent, window, cx),
                |_, _, _, _| None,
                |_: &mut Self, _: Item, _: &mut Context<Self>| {},
                |_: &mut Self, _: &mut Context<Self>| {},
            );
            body = body
                .child(div().h(rems(0.)))
                .child(div().hidden().child(element));
        }

        body.into_any_element()
    }

    /// One album tile of the overview: opening it is one click, and its sleeve
    /// is asked for with the row sleeves (visible-first, bounded).
    fn album_card(&mut self, album: &Album, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let title = album.title.clone();
        let subtitle = card::facts(&[
            album
                .record_type
                .clone()
                .unwrap_or_else(|| "Album".to_owned()),
            album.year.clone().unwrap_or_default(),
        ]);
        let file = album
            .cover
            .as_deref()
            .and_then(|url| cover::file_in(&self.covers, url));
        let url = album.cover.clone();
        let copy = album.clone();
        card::card(
            format!("album-{title}").into(),
            &title,
            &subtitle,
            file.as_ref(),
            url.as_deref(),
            Kind::Album,
            cx,
            move |this: &mut Self, _, cx| this.open_album(&copy, cx),
            |_: &mut Self, _, _| {},
            None::<fn(&mut Self, &mut Window, &mut Context<Self>)>,
        )
    }

    /// One of the overview's top titles: a plain row (the list below it is what
    /// the keys move in), with a play button and the album under the title.
    fn top_row(
        &mut self,
        index: usize,
        track: &Track,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let file = track
            .album
            .cover
            .as_deref()
            .and_then(|url| cover::file_in(&self.covers, url));
        let row = track.clone();
        div()
            .id(("artist-top", index))
            .flex()
            .items_center()
            .gap(rems(0.625))
            .h(rems(3.25))
            .px(rems(0.5))
            .cursor_pointer()
            .hover(|style| style.bg(theme::chrome(cx)))
            .on_click(cx.listener(move |this, _, _, cx| {
                let entries: Vec<_> = this.list.entries().to_vec();
                let context = this.context();
                if !entries.is_empty() {
                    this.command(nz_player::Command::play_queue(entries, index, context), cx);
                }
                let _ = &row;
            }))
            .child(
                div()
                    .w(rems(1.25))
                    .text_size(rems(0.6875))
                    .font_family(theme::mono(cx))
                    .text_color(theme::secondary(cx))
                    .child((index + 1).to_string()),
            )
            .child(cover::view(
                file.as_ref(),
                track.album.cover.as_deref(),
                &track.album.title,
                Kind::Album,
                rems(2.25).to_pixels(px(16.)),
                cx,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_size(rems(0.8125))
                            .child(track.title.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(rems(0.6875))
                            .text_color(theme::secondary(cx))
                            .child(track.album.title.clone()),
                    ),
            )
            .child(
                div()
                    .text_size(rems(0.6875))
                    .font_family(theme::mono(cx))
                    .text_color(theme::secondary(cx))
                    .child(track_list::duration_label(track.duration)),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::card::stub;

    const ME: &str =
        r#"{"user":{"id":"u1","name":"bench","admin":false,"access":{},"sources":[]}}"#;
    const ARTIST: &str = r#"{"artist":{"deezer_id":"42","id":"a1","name":"Kaya Project",
        "picture":null,"is_favorite":false,"nb_fan":1200},"top":[{"deezer_id":"3000001",
        "title":"Balancing the Dark","duration":200,
        "artist":{"deezer_id":"42","name":"Kaya Project"},
        "album":{"deezer_id":"9","title":"Songs from the Inside"}}],
        "albums":[],"related":[],"bio":null,"library":false,"credited":0}"#;
    const DISCO: &str = r#"{"discography":{"all":[],"album":[],"single":[],"ep":[]}}"#;
    const TRACKS: &str = r#"{"tracks":[{"deezer_id":"3000001","title":"Balancing the Dark",
        "duration":200,"artist":{"deezer_id":"42","name":"Kaya Project"},
        "album":{"deezer_id":"9","title":"Songs from the Inside"}},
        {"deezer_id":"3000002","title":"Sirens","duration":220,
        "artist":{"deezer_id":"42","name":"Kaya Project"},
        "album":{"deezer_id":"9","title":"Songs from the Inside"}}]}"#;

    /// One artist page asks for the page and its discography, and NOTHING else
    /// until a tab is opened: the SPA's own count (the artist page + the
    /// discography + the provider's half only when the library holds the artist).
    #[gpui::test]
    async fn an_artist_page_asks_twice_and_a_tab_asks_once_more(cx: &mut gpui::TestAppContext) {
        let stub = stub::Stub::start(&[
            ("/api/login", ME),
            ("/api/me", ME),
            ("/api/artist/42", ARTIST),
            ("/api/artist/42/discography", DISCO),
            ("/api/artist/42/tracks", TRACKS),
        ]);
        let (state, runtime) = stub::logged_in(cx, &stub);
        let (screen, cx) = cx.add_window_view({
            let state = state.clone();
            move |_window, cx| Screen::new(Route::Artist { id: "42".into() }, state, cx)
        });
        stub::until(&runtime, cx, "the artist's page", |cx| {
            screen.read(cx).page.ready().is_some()
        });
        stub::until(&runtime, cx, "the discography", |cx| {
            screen.read(cx).discography.is_some()
        });
        assert_eq!(stub.count("/api/artist/42"), 1);
        assert_eq!(stub.count("/api/artist/42/discography"), 1);
        assert_eq!(stub.count("/api/artist/42/tracks"), 0);
        assert_eq!(
            stub.count("/api/artist/42/on-deezer"),
            0,
            "the library does not hold this artist: no provider half"
        );

        // The `Titres` tab is lazy: it asks once, and only when opened.
        cx.update(|_, cx| {
            let screen = screen.read(cx);
            let _ = screen;
        });
        cx.update(|_, cx| {
            screen.update(cx, |screen, cx| {
                screen.tab = Tab::Titles;
                screen.load_titles(cx);
                cx.notify();
            });
        });
        stub::until(&runtime, cx, "the titles", |cx| {
            screen.read(cx).list.len() == 2
        });
        assert_eq!(stub.count("/api/artist/42/tracks"), 1);
        runtime.shutdown_background();
        stub.stop();
    }
}
