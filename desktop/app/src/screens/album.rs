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
    Render, StatefulInteractiveElement as _, Styled, Subscription, Window, div, px, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon};
use nz_api::ApiError;
use nz_model::{AlbumPage, Track, TrackList};

use crate::nav::{CreditKind, Route};
use crate::state::AppState;
use crate::theme;
use crate::toast::ToastKind;
use crate::widgets::card::{self, Phase};
use crate::widgets::context_menu::{self, Item, Menu};
use crate::widgets::cover::{self, Covers, Kind};
use crate::widgets::track_list::{self, Intent, TrackList as List};
use crate::wiring::events::{self, Sender};

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
            menu: Menu::new(),
            menu_keys: None,
            events: tx,
            _subscriptions: vec![cx.observe(&state, |_, _, cx| cx.notify())],
        };
        screen.apply_page(cx);
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
        let items = context_menu::track_items(false, has_artist);
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

        let actions = div()
            .flex()
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
            );

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
                |_, _, _, _| None,
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
            body = body.child(
                div().flex().flex_col().children(
                    outside.iter().map(|track| {
                        outside_row(track, &self.covers.borrow(), window.rem_size(), cx)
                    }),
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
