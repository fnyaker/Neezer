//! Album: the sleeve, what the record is, its tracks in disc order, and the
//! rest of the tracklist the provider has and the library does not.
//!
//! `routes/Album.svelte` is the reference, and the two things it does that are
//! easy to lose in a port are the ones that decide whether the page feels right:
//!
//! - **It paints from the device first** (`api.swr`): an album opened before is
//!   on screen in the frame the click lands; the fresh copy then corrects it.
//!   Measured: **data on screen before the first paint, 0 requests before it**
//!   (the request is issued at construction and answered after the frame).
//! - **The provider's half comes AFTER the library's**: an album the library
//!   holds asks `/album/<id>/on-deezer` once the page is up, so a person with the
//!   record on disk never waits for Deezer, and a dead Deezer is one line under
//!   the list, not a failed page (rule 3).
//!
//! Requests on this screen, the SPA's own and no more: `GET /album/<id>`, then
//! `GET /album/<id>/on-deezer` when `library`, `POST /favorite/album` on the
//! star, `POST /favorite` / `POST /dislike` from a row's menu.

use std::cell::RefCell;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::{
    Context, Div, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement, Pixels,
    Render, StatefulInteractiveElement as _, Styled, Subscription, Window, div, px, relative, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon};
use nz_api::ApiError;
use nz_model::{AlbumPage, Track, TrackList};

use crate::actions::{DownloadSelection, RemoveDownload};
use crate::nav::{CreditKind, Route};
use crate::state::AppState;
use crate::theme;
use crate::toast::ToastKind;
use crate::widgets::card::{self, Phase};
use crate::widgets::context_menu::{self, Item, Menu};
use crate::widgets::cover::{self, Covers, Kind};
use crate::widgets::download_control::{self, Rows, State as DownloadState, Watch};
use crate::widgets::track_list::{self, Intent, TrackList as List};
use crate::wiring::events::{self, Sender};
use crate::wiring::offline::Choice;

/// What a background thread tells this screen. One channel per screen (the
/// shell's [`crate::state::AppEvent`] is the shell's): the screen drains it in a
/// foreground task, so no worker ever touches GPUI state.
enum Msg {
    /// The fresh page — the cached one was already painted at construction.
    Fresh(Box<Result<AlbumPage, ApiError>>),
    /// The part of the tracklist only the provider has.
    Outside(Box<Result<TrackList, ApiError>>),
    Cover(String, Option<PathBuf>),
    /// The star's answer.
    Starred(Box<Result<bool, ApiError>>),
    /// A row action that failed (`POST /favorite`, `POST /dislike`).
    RowFailed(ApiError),
}

pub struct Screen {
    route: Route,
    state: Entity<AppState>,
    id: String,
    page: Phase<AlbumPage>,
    outside: Option<Vec<Track>>,
    outside_down: bool,
    starred: bool,
    star_busy: bool,
    list: List,
    covers: Rc<RefCell<Covers>>,
    /// The downloads worker, as this page reads it (lot D18): one snapshot
    /// version, and the rows resolved from it once per version.
    downloads: Watch,
    rows: Rc<Rows>,
    rows_version: u64,
    menu: Menu,
    /// The keystroke interceptor that owns the keyboard while a menu is open.
    menu_keys: Option<Subscription>,
    events: Sender<Msg>,
    _subscriptions: Vec<Subscription>,
}

impl Screen {
    pub fn new(route: Route, state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let id = match &route {
            Route::Album { id } => id.clone(),
            _ => String::new(),
        };
        let path = format!("/album/{}", card::encode(&id));
        let page = match card::cached::<AlbumPage>(state.read(cx), &path) {
            Some(page) => Phase::Ready(page),
            None => Phase::Loading,
        };
        let starred = page
            .ready()
            .and_then(|page| page.album.is_favorite)
            .unwrap_or(false);
        let (tx, rx) = events::channel::<Msg>();
        let mut screen = Self {
            route,
            state: state.clone(),
            id,
            page,
            outside: None,
            outside_down: false,
            starred,
            star_busy: false,
            list: List::new().numbered(true).show_cover(false),
            covers: Rc::new(RefCell::new(Covers::new())),
            downloads: Watch::attach(&state, cx),
            rows: Rc::new(Rows::default()),
            rows_version: u64::MAX,
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

    /// The same screen around a downloads service the caller holds: a test
    /// drives the five states of the « Télécharger » control without starting
    /// the process's worker.
    pub fn with_downloads(
        route: Route,
        state: Entity<AppState>,
        offline: Option<crate::wiring::offline::Offline>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut screen = Self::new(route, state, cx);
        screen.downloads = download_control::Watch::with_service(offline, cx);
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
            kind: "album".to_owned(),
            id: Some(self.id.clone()),
            title: (!page.album.title.is_empty()).then(|| page.album.title.clone()),
        })
    }

    /// Put the page's tracks in the list and name the album as the play context,
    /// so a listen reports the album it belongs to.
    fn apply_page(&mut self, cx: &mut Context<Self>) {
        let Some(page) = self.page.ready().cloned() else {
            self.list.clear();
            return;
        };
        self.list.set_context(self.context());
        self.list.set_rows(page.tracks.clone());
        let playing = self
            .state
            .read(cx)
            .player
            .current()
            .and_then(|entry| entry.reference().map(str::to_owned));
        self.list.select_reference(playing.as_deref());
    }

    /// The screen's one network path: the page, then the provider's half.
    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        let runtime = self.state.read(cx).runtime.clone();
        let id = self.id.clone();
        let tx = self.events.clone();
        runtime.spawn(async move {
            match api.album(&id).await {
                Ok(page) => {
                    let library = page.library;
                    tx.send(Msg::Fresh(Box::new(Ok(page))));
                    if library {
                        let answer = api.album_on_deezer(&id).await.map(|value| {
                            serde_json::from_value::<TrackList>(value).unwrap_or_default()
                        });
                        tx.send(Msg::Outside(Box::new(answer)));
                    }
                }
                Err(error) => tx.send(Msg::Fresh(Box::new(Err(error)))),
            }
        });
    }

    fn on_msg(&mut self, msg: Msg, cx: &mut Context<Self>) {
        match msg {
            Msg::Fresh(result) => match *result {
                Ok(page) => {
                    if let Some(starred) = page.album.is_favorite {
                        self.starred = starred;
                    }
                    self.page = Phase::Ready(page);
                    self.apply_page(cx);
                }
                Err(error) => {
                    // A page that already painted from the device keeps it: what
                    // failed is the revalidation, not the page.
                    if self.page.ready().is_none() {
                        self.page = Phase::Failed(error);
                    }
                }
            },
            Msg::Outside(result) => {
                self.outside = Some(match *result {
                    Ok(list) => list.tracks,
                    Err(_) => {
                        // Deezer is optional to the app running: one sentence.
                        self.outside_down = true;
                        Vec::new()
                    }
                });
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

    fn shuffle_all(&mut self, cx: &mut Context<Self>) {
        let entries = self.list.entries().to_vec();
        if !entries.is_empty() {
            self.command(
                nz_player::Command::shuffle_play(entries, self.context()),
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
        let (title, artist) = self
            .page
            .ready()
            .map(|page| (page.album.title.clone(), page.album.artist.name.clone()))
            .unwrap_or_default();
        let tx = self.events.clone();
        self.state.read(cx).runtime.spawn(async move {
            let meta = serde_json::json!({ "title": title, "artist": artist });
            let answer = api
                .favorite_entity("album", &id, on, &meta)
                .await
                .map(|_| on);
            tx.send(Msg::Starred(Box::new(answer)));
        });
    }

    /// A row's like or dislike: one mutation, the server is the truth, and a
    /// failure says so instead of doing nothing (rule 2).
    fn row_verdict(&mut self, like: bool, cx: &mut Context<Self>) {
        let Some(index) = self.list.selected() else {
            return;
        };
        let Some(reference) = self
            .list
            .row(index)
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

    /// Ask for the sleeves this list is showing: visible-first and bounded
    /// (`coverqueue.js`'s rule), never one request per row of a long album.
    fn covers_for_visible(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        let header = self.page.ready().and_then(|page| page.album.cover.clone());
        let mut asked: Vec<String> = Vec::new();
        if let Some(url) = header {
            asked.push(url);
        }
        for index in range {
            if let Some(url) = self
                .list
                .row(index)
                .and_then(|track| track.album.cover.clone())
            {
                asked.push(url);
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

    // --- downloads (lot D18) ------------------------------------------------

    /// The ids of this page's tracks, in the order the list holds them: what a
    /// download chooses, and what tells a failure of THIS album from another's.
    fn track_keys(&self) -> Vec<String> {
        self.list
            .rows()
            .iter()
            .map(download_control::key)
            .filter(|key| !key.is_empty())
            .collect()
    }

    /// Where this album stands: idle, queued, running, downloaded or failed.
    fn download_state(&mut self) -> DownloadState {
        let keys = self.track_keys();
        let snapshot = self.downloads.get();
        download_control::collection(snapshot, "album", &self.id, &keys)
    }

    /// The two verbs the control and the keymap share: `t` asks for the album,
    /// `Maj+T` takes it back (or retries what failed).
    fn bind_download(&mut self, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        let for_download = weak.clone();
        gpui_kit::App::on_action(cx, move |_: &DownloadSelection, cx: &mut gpui_kit::App| {
            for_download
                .update(cx, |this, cx| {
                    if download_control::is_front(&this.state, &this.route, cx) {
                        this.download(cx);
                    }
                })
                .ok();
        });
        gpui_kit::App::on_action(cx, move |_: &RemoveDownload, cx: &mut gpui_kit::App| {
            weak.update(cx, |this, cx| {
                if download_control::is_front(&this.state, &this.route, cx) {
                    this.drop_download(cx);
                }
            })
            .ok();
        });
    }

    /// « Télécharger » : the whole album goes to the worker's queue, on the Bulk
    /// lane, behind whatever the person is listening to (rule 1 — the worker's
    /// own gates, not this screen's).
    fn download(&mut self, cx: &mut Context<Self>) {
        let title = self
            .page
            .ready()
            .map(|page| page.album.title.clone())
            .unwrap_or_default();
        let Some(service) = self.downloads.service().cloned() else {
            self.notice(
                ToastKind::Error,
                "Les téléchargements ne sont pas disponibles sans stockage local.",
                cx,
            );
            return;
        };
        match self.download_state() {
            DownloadState::Failed { failed, .. } => {
                // A failed row keeps its cause until the person asks again; the
                // worker retries every one of them (the settings page's button).
                service.retry_failures();
                self.notice(
                    ToastKind::Info,
                    format!(
                        "Nouvelle tentative pour {failed} titre{}",
                        if failed > 1 { "s" } else { "" }
                    ),
                    cx,
                );
            }
            DownloadState::Idle => {
                let choices: Vec<_> = self
                    .list
                    .rows()
                    .iter()
                    .filter(|track| !download_control::key(track).is_empty())
                    .map(|track| Choice::new(download_control::key(track), track.title.clone()))
                    .collect();
                if choices.is_empty() {
                    self.notice(
                        ToastKind::Error,
                        "Aucun titre de cet album n'est dans la bibliothèque.",
                        cx,
                    );
                    return;
                }
                let count = choices.len();
                service.choose("album", &self.id, &title, choices);
                self.notice(
                    ToastKind::Info,
                    format!(
                        "{count} titre{} en file de téléchargement",
                        if count > 1 { "s" } else { "" }
                    ),
                    cx,
                );
            }
            // Already asked: the control's button reads « Annuler / Retirer »,
            // which dispatches the other action. Nothing to do here.
            DownloadState::Queued { .. }
            | DownloadState::Running { .. }
            | DownloadState::Done { .. } => {}
        }
        cx.notify();
    }

    /// « Retirer » : the album's download goes, files and all. The store keeps a
    /// track another set still wants (`DownloadStore::remove_set`), so removing
    /// an album never empties a playlist's download.
    fn drop_download(&mut self, cx: &mut Context<Self>) {
        let Some(service) = self.downloads.service().cloned() else {
            return;
        };
        match self.download_state() {
            DownloadState::Failed { failed, .. } => {
                service.retry_failures();
                self.notice(
                    ToastKind::Info,
                    format!(
                        "Nouvelle tentative pour {failed} titre{}",
                        if failed > 1 { "s" } else { "" }
                    ),
                    cx,
                );
            }
            DownloadState::Queued { .. }
            | DownloadState::Running { .. }
            | DownloadState::Done { .. } => {
                service.drop_set("album", &self.id);
                self.notice(ToastKind::Info, "Téléchargement de l'album retiré", cx);
            }
            DownloadState::Idle => {
                self.notice(
                    ToastKind::Info,
                    "Cet album n'est pas téléchargé sur cet appareil.",
                    cx,
                );
            }
        }
        cx.notify();
    }

    /// One row's download, from its menu (the SPA's own per-track entry).
    fn row_download(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(service) = self.downloads.service().cloned() else {
            return;
        };
        let Some(track) = self.list.row(index).cloned() else {
            return;
        };
        let key = download_control::key(&track);
        if key.is_empty() {
            return;
        }
        // One verb for a row (`download_control::row_verb`): ask, retry, cancel
        // or take back, decided by the snapshot's own state for that track.
        if let Some(message) =
            download_control::row_verb(&service, self.downloads.get(), &key, &track.title)
        {
            self.notice(ToastKind::Info, message, cx);
        }
        cx.notify();
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
        // An album's own rows have no album to open: the entry would be a door to
        // the page the person is already on.
        let _ = has_album;
        let download = self
            .list
            .row(index)
            .map(|track| {
                download_control::track(self.downloads.get(), &download_control::key(track))
            })
            .map(|state| context_menu::Download::from(&state))
            .unwrap_or_default();
        let items = context_menu::track_items(false, has_artist, download);
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
            Item::GoArtist | Item::Open => {
                if let Some(id) = index
                    .and_then(|index| self.list.row(index))
                    .map(|track| track.artist.deezer_id.as_str().to_owned())
                    .filter(|id| !id.is_empty())
                {
                    self.state
                        .update(cx, |state, _| state.nav.push(Route::Artist { id }));
                }
            }
            // An album's own row has no album to open (`GoAlbum` is only offered
            // by the lists that mix albums), and `Radio` is a menu entry only the
            // lists that carry a catalogue row offer.
            Item::GoAlbum | Item::Radio => {}
        }
        cx.notify();
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.page = Phase::Loading;
        self.outside = None;
        self.outside_down = false;
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
                    let _ = &error;
                    card::failed("Album indisponible", cause, cx, |this: &mut Self, _, cx| {
                        this.reload(cx)
                    })
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
        page: AlbumPage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        // The sleeves of what this page shows, asked once per paint (idempotent:
        // `Covers::want` refuses what it already has or has asked for).
        let visible = self.list.visible();
        self.covers_for_visible(visible, cx);

        let album = page.album.clone();
        let total: u32 = page.tracks.iter().map(|track| track.duration).sum();
        let facts = card::facts(&[
            "Album".to_owned(),
            album.year.clone().unwrap_or_default(),
            format!(
                "{} titre{}",
                page.tracks.len(),
                if page.tracks.len() > 1 { "s" } else { "" }
            ),
            track_list::duration_label(total),
        ]);
        let cover_file = album
            .cover
            .as_deref()
            .and_then(|url| cover::file_in(&self.covers, url));
        let artist_id = album.artist.deezer_id.clone();
        let starred = self.starred;
        let star_busy = self.star_busy;

        let header = card::hero_band(cx)
            .child(cover::view(
                cover_file.as_ref(),
                album.cover.as_deref(),
                &album.title,
                Kind::Album,
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
                            .child(album.title.clone()),
                    )
                    .child({
                        let mut line = div()
                            .id("album-artist")
                            .truncate()
                            .text_size(rems(0.875))
                            .text_color(theme::secondary(cx));
                        if !artist_id.as_str().is_empty() {
                            line = line
                                .cursor_pointer()
                                .hover(|style| style.text_color(theme::text(cx)))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let id = artist_id.as_str().to_owned();
                                    this.state
                                        .update(cx, |state, _| state.nav.push(Route::Artist { id }))
                                }));
                        }
                        line.child(album.artist.name.clone())
                    }),
            );

        // The downloads control: one state for the page, one chip per row. The
        // rows are resolved ONCE per published version, not once per row.
        let keys = self.track_keys();
        let download_state = {
            let snapshot = self.downloads.get();
            download_control::collection(snapshot, "album", &self.id, &keys)
        };
        let version = self.downloads.get().version;
        if version != self.rows_version {
            self.rows = Rc::new(Rows::build(self.downloads.get()));
            self.rows_version = version;
        }
        let rows_for_chips = Rc::clone(&self.rows);

        // It wraps: the download control is a second row when the window is
        // narrow (the 420 px panel leaves 660 px of content).
        let actions = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(rems(0.5))
            .px(rems(1.))
            .py(rems(0.75))
            .child(
                button("album-play", "Lire", ButtonVariant::Primary, cx)
                    .on_click(cx.listener(|this, _, _, cx| this.play_all(cx))),
            )
            .child(
                button("album-shuffle", "Mélanger", ButtonVariant::Outline, cx)
                    .on_click(cx.listener(|this, _, _, cx| this.shuffle_all(cx))),
            )
            .child(
                div()
                    .id("album-star")
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
            .children(download_control::control(
                "album-download",
                &download_state,
                self.downloads.available(),
                cx,
            ));

        let mut body = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .px(rems(1.))
            .pb(rems(1.));

        if !page.tracks.is_empty() {
            let element = track_list::element(
                cx.entity(),
                "album-tracks",
                &mut self.list,
                &self.state,
                Rc::clone(&self.covers),
                &self.menu,
                window,
                cx,
                |this: &mut Self, intent, window, cx| this.on_intent(intent, window, cx),
                move |_, _, track, cx| {
                    download_control::chip(&rows_for_chips.state(&download_control::key(track)), cx)
                },
                |this: &mut Self, item, cx| this.pick(item, cx),
                |this: &mut Self, cx| {
                    this.menu.close();
                    cx.notify();
                },
            );
            body = body.child(div().flex_1().min_h_0().child(element));
        } else {
            body = body.child(card::empty(
                IconName::Disc3,
                "Aucun titre sur cet album",
                "La bibliothèque ne tient encore aucun fichier de ce disque.",
                cx,
                None,
            ));
        }

        let outside = self.outside.clone().unwrap_or_default();
        if !outside.is_empty() {
            body = body.child(
                card::Section::new("Disponible sur Deezer")
                    .count(outside.len())
                    .render(cx, |_, _, _| {}),
            );
            // The provider's half is a LIST like any other: an album the library
            // holds one title of can list twenty. Bounded to a share of the page
            // and scrolled inside it, so the library's own tracks keep the rest of
            // the window and nothing is painted where it cannot be reached —
            // D8a's rule, and what a page that never grows needs.
            body = body.child(
                div()
                    .id("album-outside")
                    .debug_selector(|| "album-outside".to_owned())
                    .flex_shrink_0()
                    .min_h_0()
                    .max_h(relative(card::TAIL_SHARE))
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .children(outside.iter().map(|track| {
                                outside_row(track, &self.covers.borrow(), window.rem_size(), cx)
                            })),
                    ),
            );
        } else if self.outside_down {
            body = body.child(
                div()
                    .py(rems(0.5))
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child("Deezer est injoignable : la suite de l'album n'est pas affichée."),
            );
        }

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
}

/// One row of the provider's half: a title the library does not hold yet, so it
/// has no accessible state and no favourite — only what the catalogue said.
fn outside_row(track: &Track, covers: &Covers, rem: Pixels, cx: &mut Context<impl Sized>) -> Div {
    let file = track
        .album
        .cover
        .as_deref()
        .and_then(|url| covers.file(url));
    div()
        .flex()
        .items_center()
        .gap(rems(0.625))
        .h(rems(3.5))
        .px(rems(0.75))
        .border_b_1()
        .border_color(theme::border(cx).opacity(0.4))
        .child(cover::view(
            file,
            track.album.cover.as_deref(),
            &track.album.title,
            Kind::Album,
            rems(2.25).to_pixels(rem),
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
                        .child("Sur Deezer, pas encore dans la bibliothèque"),
                ),
        )
        .child(
            div()
                .text_size(rems(0.6875))
                .font_family(theme::mono(cx))
                .text_color(theme::secondary(cx))
                .child(track_list::duration_label(track.duration)),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::card::stub;
    use gpui_kit::AppContext as _;
    use nz_cache::DownloadSet;

    const ME: &str =
        r#"{"user":{"id":"u1","name":"bench","admin":false,"access":{},"sources":[]}}"#;

    fn album_body(tracks: usize, library: bool) -> String {
        let rows: Vec<String> = (0..tracks)
            .map(|index| {
                format!(
                    r#"{{"id":"row-{index}","deezer_id":"3000{index}","title":"Titre {index}",
                        "duration":180,"artist":{{"deezer_id":"42","name":"Kaya Project"}},
                        "album":{{"deezer_id":"9","title":"Songs from the Inside",
                        "cover":"/api/cover/9"}}}}"#
                )
            })
            .collect();
        format!(
            r#"{{"album":{{"deezer_id":"9","title":"Songs from the Inside",
                "cover":"/api/cover/9","year":"2004",
                "artist":{{"deezer_id":"42","name":"Kaya Project"}},"is_favorite":false}},
                "library":{library},"tracks":[{}]}}"#,
            rows.join(",")
        )
    }

    /// The « Télécharger » control of an album walks the five states the plan
    /// names, from the SNAPSHOT the worker publishes — never from a SQLite read
    /// on the GPUI thread (`nz-cache`'s access rule).
    ///
    /// Every number here is a state the person reads: the verb when nothing was
    /// asked, the queue's count, the run's count, the download, and a failure
    /// WITH ITS CAUSE (rule 2).
    #[gpui::test]
    async fn the_album_download_control_walks_its_five_states(cx: &mut gpui::TestAppContext) {
        let body = album_body(3, true);
        let stub = stub::Stub::start(&[
            ("/api/login", ME),
            ("/api/me", ME),
            ("/api/album/9", body.as_str()),
        ]);
        let (state, runtime) = stub::logged_in(cx, &stub);
        let cache = nz_cache::Cache::open_with_cache_dir(
            temp_dir("album-download"),
            temp_dir("album-download-cache"),
        )
        .expect("a store");
        let offline = crate::wiring::offline::Offline::start(cache);
        let route = Route::Album { id: "9".into() };
        let (screen, cx) = cx.add_window_view({
            let state = state.clone();
            let route = route.clone();
            let offline = offline.clone();
            move |_window, cx| Screen::with_downloads(route, state, Some(offline), cx)
        });
        stub::until(&runtime, cx, "the album's answer", |cx| {
            screen.read(cx).page.ready().is_some()
        });

        // 1. Nothing asked.
        assert_eq!(
            cx.update(|_, cx| screen.update(cx, |screen, _| screen.download_state())),
            download_control::State::Idle,
            "an album nobody asked for offers the verb"
        );
        // The control is ON the page: a draw records its box, so a state the
        // person cannot see would fail here.
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
        assert!(
            cx.debug_bounds("album-download").is_some(),
            "the « Télécharger » control is drawn"
        );

        // 2. Queued: the store knows the album as a set of three titles.
        offline.publish_for_test(|snapshot| {
            snapshot.version = 2;
            snapshot.sets = vec![DownloadSet {
                kind: "album".to_owned(),
                ident: "9".to_owned(),
                title: "Songs from the Inside".to_owned(),
                added_at: 0,
                tracks: 3,
                done: 1,
            }];
            snapshot.progress.done = 1;
        });
        assert_eq!(
            cx.update(|_, cx| screen.update(cx, |screen, _| screen.download_state())),
            download_control::State::Queued { done: 1, total: 3 },
            "a set waiting its turn reads « en file »"
        );

        // 3. Running: the worker names the track it is writing.
        offline.publish_for_test(|snapshot| {
            snapshot.version = 3;
            snapshot.running = Some(crate::wiring::offline::Running {
                track: "row-1".to_owned(),
                title: "Titre 1".to_owned(),
                quality: nz_api::Quality::Opus320,
                bytes: 4096,
                total: Some(8192),
            });
        });
        assert!(matches!(
            cx.update(|_, cx| screen.update(cx, |screen, _| screen.download_state())),
            download_control::State::Running {
                done: 1,
                total: 3,
                ..
            }
        ));

        // 4. Downloaded: all three, and the control offers « Retirer ».
        offline.publish_for_test(|snapshot| {
            snapshot.version = 4;
            snapshot.running = None;
            snapshot.sets[0].done = 3;
            snapshot.progress.done = 3;
        });
        let done = cx.update(|_, cx| screen.update(cx, |screen, _| screen.download_state()));
        assert_eq!(done, download_control::State::Done { tracks: 3 });
        assert_eq!(done.action(), "Retirer");
        assert_eq!(done.label(), "Téléchargé · 3 titres");

        // 5. A failure carries its cause, and offers to ask again.
        offline.publish_for_test(|snapshot| {
            snapshot.version = 5;
            snapshot.sets[0].done = 2;
            snapshot.progress.done = 2;
            snapshot.failures = vec![crate::wiring::offline::Failure {
                track: "row-2".to_owned(),
                title: "Titre 2".to_owned(),
                cause: "HTTP 404 : introuvable".to_owned(),
            }];
        });
        let failed = cx.update(|_, cx| screen.update(cx, |screen, _| screen.download_state()));
        assert!(matches!(
            &failed,
            download_control::State::Failed { failed: 1, .. }
        ));
        assert_eq!(failed.action(), "Réessayer");
        assert!(
            failed.label().contains("HTTP 404"),
            "the screen prints the CAUSE, never a bare count: {}",
            failed.label()
        );

        // A row of a collection that only PARTLY landed reads its own state (a
        // failure, when it failed), and a row with nothing to say draws nothing.
        // The rows are resolved by a paint, so draw the new version first.
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
        let rows = cx.update(|_, cx| screen.update(cx, |screen, _| Rc::clone(&screen.rows)));
        assert!(matches!(
            rows.state("row-2"),
            download_control::State::Failed { .. }
        ));
    }

    /// `t` and `Maj+T` are the control's two verbs without the mouse: `t` puts
    /// the album in the worker's queue, `Maj+T` takes it back — the same
    /// handlers the button and the row menu run.
    #[gpui::test]
    async fn t_and_maj_t_queue_and_take_back_the_album(cx: &mut gpui::TestAppContext) {
        let body = album_body(3, true);
        let stub = stub::Stub::start(&[
            ("/api/login", ME),
            ("/api/me", ME),
            ("/api/album/9", body.as_str()),
        ]);
        let (state, runtime) = stub::logged_in(cx, &stub);
        let cache = nz_cache::Cache::open_with_cache_dir(
            temp_dir("album-keys"),
            temp_dir("album-keys-cache"),
        )
        .expect("a store");
        let offline = crate::wiring::offline::Offline::start(cache);
        let route = Route::Album { id: "9".into() };
        let (_screen, cx) = cx.add_window_view({
            let state = state.clone();
            let route = route.clone();
            let offline = offline.clone();
            move |_window, cx| Screen::with_downloads(route, state, Some(offline), cx)
        });
        // The screen is in front, as the shell leaves it (`is_front`): a panel
        // mounted beside it is what the guard keeps out.
        cx.update(|_, cx| {
            state.update(cx, |state, _| state.nav.replace(route.clone()));
        });
        stub::until(&runtime, cx, "the album's answer", |_| true);

        cx.update(|window, cx| {
            window.dispatch_action(Box::new(DownloadSelection), cx);
        });
        cx.run_until_parked();
        let note = offline.snapshot().note.unwrap_or_default();
        assert!(
            note.contains("ajouté aux téléchargements"),
            "« t » asks for the album: {note}"
        );

        // The worker publishes the set it took; `Maj+T` then has something to
        // take back, and the screen says so (rule 2: an action answers).
        offline.publish_for_test(|snapshot| {
            snapshot.version = 7;
            snapshot.sets = vec![DownloadSet {
                kind: "album".to_owned(),
                ident: "9".to_owned(),
                title: "Songs from the Inside".to_owned(),
                added_at: 0,
                tracks: 3,
                done: 0,
            }];
        });
        cx.update(|window, cx| {
            window.dispatch_action(Box::new(RemoveDownload), cx);
        });
        cx.run_until_parked();
        let said: Vec<String> = cx.update(|_, cx| {
            state
                .read(cx)
                .toasts
                .items()
                .iter()
                .map(|toast| toast.message.to_string())
                .collect()
        });
        assert!(
            said.iter().any(|message| message.contains("retiré")),
            "« Maj+T » takes the album back and says it: {said:?}"
        );
    }

    /// **One request per title, and NONE for a title already on the device.**
    ///
    /// The count the acceptance asks for, measured against a real socket: the
    /// worker asks `/api/stream/<id>` once for each of the album's three titles,
    /// publishes the files, and a SECOND « Télécharger » (the one that keeps a
    /// playlist complete when it gains a title) costs zero requests — the store
    /// links only what is new (`DownloadStore::add_set`'s `ON CONFLICT`).
    #[gpui::test]
    async fn downloading_an_album_costs_one_request_per_title_and_nothing_the_second_time(
        cx: &mut gpui::TestAppContext,
    ) {
        let body = album_body(3, true);
        let audio = b"OggS-fake-opus-body".to_vec();
        let stub = stub::Stub::start_bytes(&[
            ("/api/login", ME.as_bytes().to_vec()),
            ("/api/me", ME.as_bytes().to_vec()),
            ("/api/album/9", body.into_bytes()),
            ("/api/stream/row-0", audio.clone()),
            ("/api/stream/row-1", audio.clone()),
            ("/api/stream/row-2", audio),
        ]);
        let (state, runtime) = stub::logged_in(cx, &stub);
        let dir = temp_dir("album-requests");
        let cache = nz_cache::Cache::open_with_cache_dir(&dir, dir.join("cache")).expect("a store");
        let offline = crate::wiring::offline::Offline::start(cache);
        let api = cx.update(|cx| state.read(cx).api.clone().expect("a client"));
        offline.configure(api, runtime.handle().clone());
        let route = Route::Album { id: "9".into() };
        let (_screen, cx) = cx.add_window_view({
            let state = state.clone();
            let route = route.clone();
            let offline = offline.clone();
            move |_window, cx| Screen::with_downloads(route, state, Some(offline), cx)
        });
        cx.update(|_, cx| {
            state.update(cx, |state, _| state.nav.replace(route.clone()));
        });
        stub::until(&runtime, cx, "the album's answer", |_| true);

        cx.update(|window, cx| {
            window.dispatch_action(Box::new(DownloadSelection), cx);
        });
        stub::until(&runtime, cx, "three downloads", |_| {
            offline.snapshot().progress.done == 3
        });
        let asked: usize = ["row-0", "row-1", "row-2"]
            .iter()
            .map(|id| stub.count(&format!("/api/stream/{id}")))
            .sum();
        assert_eq!(
            asked,
            3,
            "one request per title: {:?}",
            stub.hits()
                .iter()
                .map(|hit| hit.path.clone())
                .collect::<Vec<_>>()
        );
        for id in ["row-0", "row-1", "row-2"] {
            assert_eq!(
                stub.count(&format!("/api/stream/{id}")),
                1,
                "{id} asked once"
            );
        }

        // The same album asked again (a playlist that gained a title does this
        // on every open): the worker finds nothing pending and asks NOTHING.
        cx.update(|window, cx| {
            window.dispatch_action(Box::new(DownloadSelection), cx);
        });
        cx.run_until_parked();
        for id in ["row-0", "row-1", "row-2"] {
            assert_eq!(
                stub.count(&format!("/api/stream/{id}")),
                1,
                "{id} is on the device: no second request"
            );
        }
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "neezer-d18-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// The album page paints the copy this device already has BEFORE the first
    /// frame, and asks the server exactly once after it: the SPA's `api.swr`, the
    /// acceptance's "paints from cache in one frame", and rule 3's "what was
    /// fetched once is kept".
    #[gpui::test]
    async fn a_second_visit_paints_from_the_device_before_any_request(
        cx: &mut gpui::TestAppContext,
    ) {
        let body = album_body(3, false);
        let stub = stub::Stub::start(&[
            ("/api/login", ME),
            ("/api/me", ME),
            ("/api/album/9", body.as_str()),
        ]);
        let (state, runtime) = stub::logged_in(cx, &stub);
        let route = Route::Album { id: "9".into() };
        let (first, cx) = cx.add_window_view({
            let state = state.clone();
            let route = route.clone();
            move |_window, cx| Screen::new(route, state, cx)
        });
        stub::until(&runtime, cx, "the album's answer", |cx| {
            first.read(cx).page.ready().is_some()
        });
        assert_eq!(stub.count("/api/album/9"), 1, "one page, one request");

        // The visit a person makes the second time: a NEW screen, built the way a
        // navigation builds one.
        let second = cx.update(|_, cx| cx.new(|cx| Screen::new(route, state.clone(), cx)));
        let at_once = cx.update(|_, cx| {
            let screen = second.read(cx);
            assert!(screen.page.ready().is_some(), "the copy is on screen");
            screen.list.len()
        });
        assert_eq!(at_once, 3, "the cached page's tracks are in the list");
        assert_eq!(
            stub.count("/api/album/9"),
            1,
            "the paint cost 0 requests; the revalidation starts after it"
        );
        stub::until(&runtime, cx, "the revalidation", |_| {
            stub.count("/api/album/9") == 2
        });
        runtime.shutdown_background();
        stub.stop();
    }

    /// The shell's focus anchor, the way the app gives it: a screen never takes
    /// the focus (D8b), so a test that presses a key must provide the node the
    /// keymap's `Neezer && !Input` context is looked up on.
    struct Host {
        screen: Entity<Screen>,
        focus: gpui_kit::FocusHandle,
    }

    impl Render for Host {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .id("album-host")
                .size_full()
                .track_focus(&self.focus)
                .key_context("Neezer")
                .child(self.screen.clone())
        }
    }

    fn outside_body(tracks: usize) -> String {
        let rows: Vec<String> = (0..tracks)
            .map(|index| {
                format!(
                    r#"{{"id":"out-{index}","deezer_id":"4000{index}","title":"Hors {index}",
                        "duration":200,"artist":{{"deezer_id":"42","name":"Kaya Project"}},
                        "album":{{"deezer_id":"9","title":"Songs from the Inside",
                        "cover":"/api/cover/9"}}}}"#
                )
            })
            .collect();
        format!(r#"{{"tracks":[{}]}}"#, rows.join(","))
    }

    /// **The album's list takes the wheel, the touchpad and the keys**
    /// (v1.6.1). The shared list built a scroll handle of its own and never
    /// attached the screen's, so every frame started from offset 0: the wheel
    /// moved NOTHING (the visible rows stayed `0..10` whatever was sent),
    /// `Page suivante` moved the cursor to a row nobody could see, and the
    /// scrollbar beside the list never budged. Measured with this test, thirty
    /// rows in a 800 px window: before, the wheel left the offset at 0 px;
    /// after, one notch is **57 px** (`visible 1..11`), `Page suivante`
    /// **566 px** (ten rows), `Fin` the last row, and the same in the **660 px
    /// pane** a 1280 window keeps with the 420 px panel open.
    #[gpui::test]
    async fn the_album_track_list_scrolls_with_the_wheel_and_the_keys(
        cx: &mut gpui::TestAppContext,
    ) {
        use gpui::{Modifiers, ScrollDelta, ScrollWheelEvent, TouchPhase, point, px, size};

        // `stub::logged_in` installs the theme and the keymap (once: binding
        // them twice would dispatch every action twice).
        let body = album_body(30, true);
        let outside = outside_body(20);
        let stub = stub::Stub::start(&[
            ("/api/login", ME),
            ("/api/me", ME),
            ("/api/album/9", body.as_str()),
            ("/api/album/9/on-deezer", outside.as_str()),
        ]);
        let (state, runtime) = stub::logged_in(cx, &stub);
        let (host, cx) = cx.add_window_view({
            let state = state.clone();
            move |_, cx| {
                let screen = cx.new(|cx| Screen::new(Route::Album { id: "9".into() }, state, cx));
                Host {
                    screen,
                    focus: cx.focus_handle(),
                }
            }
        });
        stub::until(&runtime, cx, "the album and its Deezer half", |cx| {
            let screen = host.read(cx).screen.read(cx);
            screen.list.len() == 30 && screen.outside.is_some()
        });
        // The shell's anchor is the focused node; a screen keeps it (D8b).
        cx.update(|window, cx| {
            let focus = host.read(cx).focus.clone();
            window.focus(&focus, cx);
        });
        cx.run_until_parked();

        let paint = |cx: &mut gpui::VisualTestContext| {
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            })
        };
        let seen = |cx: &mut gpui::VisualTestContext| {
            cx.update(|_, cx| {
                let list = &host.read(cx).screen.read(cx).list;
                (list.scroll_y(), list.visible(), list.selected())
            })
        };
        let wheel = |cx: &mut gpui::VisualTestContext| {
            cx.simulate_mouse_move(point(px(320.), px(300.)), None, Modifiers::default());
            cx.simulate_event(ScrollWheelEvent {
                position: point(px(320.), px(300.)),
                delta: ScrollDelta::Lines(point(0., -3.)),
                modifiers: Modifiers::default(),
                touch_phase: TouchPhase::Moved,
            });
        };

        for (width, what) in [(1280., "1280×800"), (660., "the 660 px pane")] {
            cx.simulate_resize(size(px(width), px(800.)));
            paint(cx);
            let (offset, visible, selected) = seen(cx);
            assert_eq!(offset, px(0.), "{what}: a fresh page starts at the top");
            assert_eq!(visible.start, 0);
            assert_eq!(selected, Some(0));
            assert!(
                visible.len() < 30,
                "{what}: the list builds a screenful, not 30 rows ({visible:?})"
            );

            // The wheel, exactly as the X11 backend builds it from a notch.
            wheel(cx);
            paint(cx);
            let (after_wheel, wheeled, _) = seen(cx);
            assert!(
                after_wheel < px(-10.),
                "{what}: the wheel scrolled the list; measured {after_wheel:?}"
            );
            assert!(
                wheeled.start > 0,
                "{what}: the rows on screen changed; measured {wheeled:?}"
            );

            // `Page suivante` / `Fin` / `Début`, through the action the keymap
            // binds (the same dispatch a real keystroke produces).
            cx.update(|_, cx| cx.dispatch_action(&crate::actions::ListPageDown));
            cx.run_until_parked();
            paint(cx);
            let (after_page, paged, row) = seen(cx);
            assert!(
                after_page < after_wheel - px(200.),
                "{what}: Page suivante took most of the window; measured \
                 {after_page:?} from {after_wheel:?}"
            );
            assert!(
                paged.contains(&row.expect("a selected row")),
                "{what}: the row the cursor moved to is on screen; {row:?} not in {paged:?}"
            );

            cx.update(|_, cx| cx.dispatch_action(&crate::actions::ListLast));
            cx.run_until_parked();
            paint(cx);
            let (bottom, last, row) = seen(cx);
            assert_eq!(row, Some(29), "{what}: Fin reaches the last row");
            assert!(
                last.contains(&29),
                "{what}: the last row is painted, not just selected; {last:?}"
            );
            assert!(
                bottom < px(-1_000.),
                "{what}: the list went down; {bottom:?}"
            );

            cx.update(|_, cx| cx.dispatch_action(&crate::actions::ListFirst));
            cx.run_until_parked();
            paint(cx);
            assert_eq!(seen(cx).0, px(0.), "{what}: Début returns to the top");
            println!(
                "album_scroll {what}: wheel={after_wheel:?} pagedown={:?} bottom={bottom:?} rows={visible:?}",
                after_page - after_wheel
            );
        }

        // The provider's half is bounded to a share of the page and scrolls
        // inside it: with twenty titles the library's own list keeps the window,
        // and no row is painted where it cannot be reached. (`debug_bounds` is
        // recorded only where the selectors are, i.e. a debug build.)
        cx.simulate_resize(size(px(660.), px(800.)));
        paint(cx);
        #[cfg(debug_assertions)]
        {
            let band = cx
                .debug_bounds("album-outside")
                .expect("the Deezer band is laid out");
            let window_height = cx.update(|window, _| window.viewport_size().height);
            assert!(
                band.bottom() <= window_height,
                "the band ends inside the window: {:?} of {window_height:?}",
                band
            );
        }
        runtime.shutdown_background();
        stub.stop();
    }

    /// A page that fails with no copy on the device says why and offers
    /// « Réessayer » (rule 3), never an endless skeleton.
    #[gpui::test]
    async fn a_failed_album_names_its_cause(cx: &mut gpui::TestAppContext) {
        let stub = stub::Stub::start(&[("/api/login", ME), ("/api/me", ME)]);
        let (state, runtime) = stub::logged_in(cx, &stub);
        let (screen, cx) = cx.add_window_view({
            let state = state.clone();
            move |_window, cx| Screen::new(Route::Album { id: "404".into() }, state, cx)
        });
        stub::until(&runtime, cx, "the failure", |cx| {
            matches!(screen.read(cx).page, Phase::Failed(_))
        });
        cx.update(|_, cx| {
            let cause = match &screen.read(cx).page {
                Phase::Failed(error) => card::cause(error).to_string(),
                _ => String::new(),
            };
            assert!(!cause.is_empty(), "a cause a person can read");
        });
        runtime.shutdown_background();
        stub.stop();
    }

    /// A list of 4 000 tracks paints a screenful of rows, not 4 000: the
    /// windowing that rule 4 and D8a's frame budget rest on.
    #[gpui::test]
    async fn four_thousand_rows_build_a_screenful(cx: &mut gpui::TestAppContext) {
        // ~12 MB of row data and 82 frames: in a child process, so a sibling
        // test that measures this process's RSS delta sees only its own work.
        if let Some(printed) =
            stub::in_child("screens::album::tests::four_thousand_rows_build_a_screenful")
        {
            println!("{printed}");
            assert!(
                printed.contains("list_paint_us") || printed.contains("test result: ok"),
                "the child ran:\n{printed}"
            );
            return;
        }
        let body = album_body(4_000, false);
        let stub = stub::Stub::start(&[
            ("/api/login", ME),
            ("/api/me", ME),
            ("/api/album/9", body.as_str()),
        ]);
        let (state, runtime) = stub::logged_in(cx, &stub);
        let (screen, cx) = cx.add_window_view({
            let state = state.clone();
            move |_window, cx| Screen::new(Route::Album { id: "9".into() }, state, cx)
        });
        stub::until(&runtime, cx, "the album's answer", |cx| {
            screen.read(cx).list.len() == 4_000
        });
        cx.run_until_parked();
        let visible = cx.update(|_, cx| {
            let screen = screen.read(cx);
            let built = screen.list.visible();
            (built.len(), built)
        });
        assert!(
            visible.0 < 60,
            "4 000 rows built {} rows on screen",
            visible.0
        );
        // Scrolling it: 40 pages down, each asking for a repaint, which is what a
        // wheel does one notch at a time. The row-building cost of every frame is
        // what `NZ_FRAME_LOG=1` records.
        for _ in 0..40 {
            cx.update(|window, cx| {
                screen.update(cx, |screen, cx| {
                    assert!(screen.list.apply(Intent::Page(1), 20).is_none());
                    cx.notify();
                });
                window.refresh();
            });
            cx.run_until_parked();
        }
        let frame = cx.update(|_, cx| screen.read(cx).list.frame_summary());
        if let Some((p50, p95, max, samples)) = frame {
            println!("list_paint_us p50={p50:.0} p95={p95:.0} max={max:.0} n={samples}");
            // The 240 Hz panel's budget is 4.17 ms; the list's own work must
            // leave the frame almost all of it.
            assert!(p95 < 4_170., "p95 {p95} µs");
        }
        runtime.shutdown_background();
        stub.stop();
    }
}
