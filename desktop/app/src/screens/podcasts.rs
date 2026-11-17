//! Podcasts: the shows this person follows, and — for an admin — the catalogue
//! search that subscribes a new one.
//!
//! `webapp/src/routes/Podcasts.svelte` is the reference. What it decides, kept
//! here:
//!
//! - **The subscriptions are one request** (`GET /api/podcasts`), never one per
//!   show, and the page paints them as a grid of sleeves with the count of
//!   episodes and an « auto » chip when the show archives by itself.
//! - **Search is a debounced field, one request per settled term** (300 ms, the
//!   SPA's own delay), and its results are CARDS THAT SUBSCRIBE when clicked: a
//!   catalogue result is not yet a show on this server, so opening it is adding
//!   it (`POST /api/podcasts {"url"}` with the provider's own URL, exactly what
//!   `lib/actions.js#subscribeToPodcast` builds).
//! - **A pasted Deezer link subscribes too.** `Entrée` in the field with
//!   `deezer.com/…/show/…` or `…/episode/…` posts the link itself — the server
//!   resolves it — and then opens the show it created.
//! - **`/podcasts` is not a cacheable path** (`lib/apicache.js`), so a dead link
//!   shows the cause and « Réessayer » rather than a stale list presented as
//!   live; what the page already painted stays on screen while the retry fails.
//! - **The keyboard is the mouse**: `j`/`k` move a cursor over the grid,
//!   `Début`/`Fin` jump, `Entrée` opens the focused card (or subscribes a
//!   focused result), and the field is Tab-reachable. Every card is also a click.
//!
//! The screen does not take the focus (`docs/agents/desktop.md`, D8b): its verbs
//! arrive through `track_list::own`, which registers them on the app with a weak
//! handle.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Div, ElementId, Entity, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement, Pixels, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, Task, Window, div, px, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon, icon_button};
use nz_api::ApiError;
use nz_model::{PodcastCard, PodcastPage, PodcastShow};
use serde_json::Value;

use crate::nav::Route;
use crate::state::AppState;
use crate::theme;
use crate::toast::ToastKind;
use crate::widgets::card;
use crate::widgets::cover::{self, Covers, Kind};
use crate::widgets::download_control::Watch;
use crate::widgets::track_list::{self, Intent};
use crate::wiring::events::{self, Sender};
use crate::wiring::offline::Choice;

/// The search field's quiet time: the SPA's 300 ms.
pub const DEBOUNCE_MS: u64 = 300;

/// The page's reading width: the 1800 px cap the other screens use (D12), so a
/// 2560 px window is margins and the grid spreads from a sane left edge.
const PAGE_MAX: f32 = 1_800.;

/// What a background thread tells this screen.
enum Msg {
    /// The subscriptions, fresh from the server.
    Listed(Box<Result<Vec<PodcastShow>, ApiError>>),
    /// One term's catalogue results.
    Searched {
        term: String,
        answer: Box<Result<Vec<PodcastCard>, ApiError>>,
    },
    /// The answer to a subscribe (a show, or the cause it failed).
    Subscribed(Box<Result<Value, ApiError>>),
    Cover(String, Option<PathBuf>),
    /// The page a « Télécharger » on a card asked for, with the show it names.
    ShowPage(String, String, Box<Result<PodcastPage, ApiError>>),
}

pub struct Screen {
    route: Route,
    state: Entity<AppState>,
    channels: Vec<PodcastShow>,
    loading: bool,
    /// The list could not be read and there is nothing to paint.
    failed: Option<ApiError>,
    /// The keyboard's card, in the grid the person is looking at.
    cursor: usize,
    input: Option<Entity<InputState>>,
    query: String,
    /// The term the results are for (`None`: no search has answered yet).
    searched: Option<String>,
    results: Vec<PodcastCard>,
    searching: bool,
    search_failed: Option<ApiError>,
    /// A subscribe is in flight: the card says so instead of looking dead.
    subscribing: Option<String>,
    covers: Rc<RefCell<Covers>>,
    /// The downloads worker, as this screen reads it (lot D18).
    downloads: Watch,
    scroll: ScrollHandle,
    events: Sender<Msg>,
    debounce: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Screen {
    pub fn new(route: Route, state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let (tx, rx) = events::channel::<Msg>();
        let mut screen = Self {
            route,
            state: state.clone(),
            channels: Vec::new(),
            loading: true,
            failed: None,
            cursor: 0,
            input: None,
            query: String::new(),
            searched: None,
            results: Vec::new(),
            searching: false,
            search_failed: None,
            subscribing: None,
            covers: Rc::new(RefCell::new(Covers::new())),
            downloads: Watch::attach(&state, cx),
            scroll: ScrollHandle::new(),
            events: tx,
            debounce: None,
            _subscriptions: vec![cx.observe(&state, |_, _, cx| cx.notify())],
        };
        screen.drain(rx, cx);
        // The person's playback positions, kept for the life of the process: this
        // page is one of the two ways into a podcast, so it is one of the two
        // places the keeper can start (`screens::show`).
        crate::screens::show::attach_keeper(state, cx);
        track_list::own(cx, |this: &mut Self, intent, window, cx| {
            this.on_intent(intent, window, cx)
        });
        screen.load(cx);
        screen
    }

    pub fn route(&self) -> &Route {
        &self.route
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

    /// The one request the page makes on open.
    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        self.loading = true;
        let tx = self.events.clone();
        self.state.read(cx).runtime.spawn(async move {
            let answer = api.podcasts().await.map(|list| list.podcasts);
            tx.send(Msg::Listed(Box::new(answer)));
        });
    }

    fn on_msg(&mut self, msg: Msg, cx: &mut Context<Self>) {
        match msg {
            Msg::ShowPage(id, title, answer) => {
                let Some(service) = self.downloads.service().cloned() else {
                    return;
                };
                match *answer {
                    Ok(page) => {
                        let choices: Vec<_> = page
                            .episodes
                            .iter()
                            .map(|episode| {
                                Choice::new(
                                    crate::widgets::episode_panel::key(episode),
                                    episode.title.clone(),
                                )
                            })
                            .filter(|choice| !choice.track.is_empty())
                            .collect();
                        if choices.is_empty() {
                            self.notice(
                                ToastKind::Info,
                                "Cette émission n'a aucun épisode à télécharger.",
                                cx,
                            );
                            return;
                        }
                        let count = choices.len();
                        service.choose("show", &id, &title, choices);
                        self.notice(
                            ToastKind::Info,
                            format!(
                                "« {title} » : {count} épisode{} en file de téléchargement",
                                if count > 1 { "s" } else { "" }
                            ),
                            cx,
                        );
                    }
                    Err(error) => self.notice(
                        ToastKind::Error,
                        format!("Téléchargement impossible : {}", card::cause(&error)),
                        cx,
                    ),
                }
                cx.notify();
            }
            Msg::Listed(answer) => {
                self.loading = false;
                match *answer {
                    Ok(channels) => {
                        self.failed = None;
                        if self.cursor >= channels.len() {
                            self.cursor = channels.len().saturating_sub(1);
                        }
                        self.channels = channels;
                    }
                    Err(error) => {
                        // What is on screen stays: it was true a minute ago, and
                        // the cause says why it may be old (rule 3).
                        if self.channels.is_empty() {
                            self.failed = Some(error);
                        } else {
                            self.notice(
                                ToastKind::Error,
                                format!("Liste non actualisée : {}", card::cause(&error)),
                                cx,
                            );
                        }
                    }
                }
            }
            Msg::Searched { term, answer } => {
                if self.searched.as_deref() != Some(term.as_str()) {
                    return; // a newer term is being typed
                }
                self.searching = false;
                match *answer {
                    Ok(results) => {
                        self.search_failed = None;
                        self.cursor = 0;
                        self.results = results;
                    }
                    Err(error) => {
                        self.search_failed = Some(error);
                        self.results.clear();
                    }
                }
            }
            Msg::Subscribed(answer) => {
                self.subscribing = None;
                match *answer {
                    Ok(show) => {
                        let id = show
                            .get("id")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                            .unwrap_or_default();
                        self.notice(ToastKind::Info, "Podcast ajouté", cx);
                        self.query.clear();
                        self.searched = None;
                        self.results.clear();
                        self.load(cx);
                        if !id.is_empty() {
                            self.state
                                .update(cx, |state, _| state.nav.push(Route::Show { id }));
                        }
                    }
                    Err(error) => {
                        self.notice(
                            ToastKind::Error,
                            format!("Podcast non ajouté : {}", card::cause(&error)),
                            cx,
                        );
                    }
                }
            }
            Msg::Cover(url, file) => self.covers.borrow_mut().resolved(&url, file),
        }
        cx.notify();
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

    /// The field changed: restart the 300 ms beat, never ask on the keystroke.
    fn typed(&mut self, cx: &mut Context<Self>) {
        let term = self
            .input
            .as_ref()
            .map(|input| input.read(cx).value().to_string())
            .unwrap_or_default();
        self.query = term;
        self.debounce = None;
        let trimmed = self.query.trim().to_owned();
        if trimmed.is_empty() {
            self.searched = None;
            self.results.clear();
            self.searching = false;
            self.search_failed = None;
            cx.notify();
            return;
        }
        self.searching = true;
        self.debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(DEBOUNCE_MS))
                .await;
            let _ = this.update(cx, |this, cx| this.search(trimmed, cx));
        }));
        cx.notify();
    }

    /// Ask the catalogue for one term. The term is remembered BEFORE the request
    /// so a late answer for an older word is dropped, not painted.
    fn search(&mut self, term: String, cx: &mut Context<Self>) {
        if term.is_empty() {
            return;
        }
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        self.searched = Some(term.clone());
        self.searching = true;
        self.search_failed = None;
        let tx = self.events.clone();
        self.state.read(cx).runtime.spawn(async move {
            let answer = api
                .search_podcasts(&term)
                .await
                .map(|search| search.podcasts);
            tx.send(Msg::Searched {
                term,
                answer: Box::new(answer),
            });
        });
    }

    /// `Entrée`: a pasted link subscribes, anything else searches at once.
    fn submit(&mut self, cx: &mut Context<Self>) {
        let term = self.query.trim().to_owned();
        if term.is_empty() {
            return;
        }
        self.debounce = None;
        if is_deezer_link(&term) {
            self.subscribe(term, cx);
            return;
        }
        self.search(term, cx);
    }

    /// Add a show to the subscriptions: the server resolves the link and imports
    /// it (a job of its own, behind anything played).
    fn subscribe(&mut self, url: String, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        if self.subscribing.is_some() {
            return;
        }
        self.subscribing = Some(url.clone());
        let tx = self.events.clone();
        self.state.read(cx).runtime.spawn(async move {
            let answer = api.subscribe_podcast(&url).await;
            tx.send(Msg::Subscribed(Box::new(answer)));
        });
        cx.notify();
    }

    /// Clicking a search result subscribes it: a catalogue card is not a show on
    /// this server until it is (`PodcastCard.svelte#add`).
    fn subscribe_card(&mut self, card: &PodcastCard, cx: &mut Context<Self>) {
        if card.deezer_id.is_empty() {
            return;
        }
        self.subscribe(
            format!("https://www.deezer.com/show/{}", card.deezer_id),
            cx,
        );
    }

    /// « Télécharger l'émission » from its card: the first page of episodes, on
    /// the Bulk lane once it reaches the worker.
    ///
    /// The card carries a count, not the episodes, so one READ is what turns it
    /// into a download; the show's own page offers the same verb with its state.
    fn download_show(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(show) = self.channels.get(index) else {
            return;
        };
        let id = show.id.as_str().to_owned();
        let title = show.title.clone();
        if self.downloads.service().is_none() {
            self.notice(
                ToastKind::Error,
                "Les téléchargements ne sont pas disponibles sans stockage local.",
                cx,
            );
            return;
        }
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        let tx = self.events.clone();
        self.state.read(cx).runtime.spawn(async move {
            let answer = api.podcast(&id, crate::screens::show::PAGE, None).await;
            tx.send(Msg::ShowPage(id, title, Box::new(answer)));
        });
        cx.notify();
    }

    fn open_show(&mut self, id: &str, cx: &mut Context<Self>) {
        if id.is_empty() {
            return;
        }
        let route = Route::Show { id: id.to_owned() };
        self.state.update(cx, |state, _| state.nav.push(route));
    }

    /// The cards the cursor walks: results while a search is on screen, the
    /// subscriptions otherwise.
    fn cursor_len(&self) -> usize {
        if self.searching || self.searched.is_some() {
            self.results.len()
        } else {
            self.channels.len()
        }
    }

    fn on_intent(&mut self, intent: Intent, _window: &mut Window, cx: &mut Context<Self>) {
        let len = self.cursor_len();
        match intent {
            Intent::Move(delta) => {
                if len == 0 {
                    return;
                }
                let next = (self.cursor as i64 + delta).clamp(0, len as i64 - 1) as usize;
                self.cursor = next;
            }
            Intent::First => self.cursor = 0,
            Intent::Last => self.cursor = len.saturating_sub(1),
            Intent::Page(delta) => {
                let page = 6usize;
                let next = (self.cursor as i64 + (delta * page as i64))
                    .clamp(0, len.saturating_sub(1) as i64) as usize;
                self.cursor = next;
            }
            Intent::PlayFrom(_) => self.activate_cursor(cx),
            // A show has no next/queue meaning before its episodes are listed,
            // and the menu of a show is its own page: both are the show page's.
            Intent::PlayNextFrom(_)
            | Intent::QueueFrom(_)
            | Intent::Menu(_)
            | Intent::Like(_)
            | Intent::Dislike(_) => {}
        }
        cx.notify();
    }

    fn activate_cursor(&mut self, cx: &mut Context<Self>) {
        if self.searching || self.searched.is_some() {
            if let Some(card) = self.results.get(self.cursor).cloned() {
                self.subscribe_card(&card, cx);
            }
            return;
        }
        if let Some(show) = self.channels.get(self.cursor).cloned() {
            self.open_show(show.id.as_str(), cx);
        }
    }

    /// Ask for the sleeves of what this page shows — one request per card, never
    /// a fan-out (`cover::Covers` bounds it at 8).
    fn want_covers(&mut self, cx: &mut Context<Self>) {
        let snapshot = self.state.read(cx);
        let mut covers = self.covers.borrow_mut();
        let urls: Vec<String> = if self.searching || self.searched.is_some() {
            self.results
                .iter()
                .filter_map(|card| card.cover.clone())
                .collect()
        } else {
            self.channels
                .iter()
                .filter_map(|show| show.cover.clone())
                .collect()
        };
        for url in urls {
            let for_request = url.clone();
            let for_message = url.clone();
            let tx = self.events.clone();
            covers.want(snapshot, &for_request, move |file| {
                tx.send(Msg::Cover(for_message, file));
            });
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.failed = None;
        self.covers.borrow_mut().retry_failed();
        self.load(cx);
        if let Some(term) = self.searched.clone() {
            self.search(term, cx);
        }
        cx.notify();
    }
}

/// Does the typed text name a Deezer show or episode?
pub fn is_deezer_link(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("deezer.com/") && (lower.contains("/show/") || lower.contains("/episode/"))
}

impl Render for Screen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.input.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx).placeholder("Nom du podcast ou lien Deezer…")
            });
            let subscription =
                cx.subscribe(&input, |this, _, event: &InputEvent, cx| match event {
                    InputEvent::Change => this.typed(cx),
                    InputEvent::PressEnter { .. } => this.submit(cx),
                    _ => {}
                });
            self._subscriptions.push(subscription);
            self.input = Some(input);
        }
        self.want_covers(cx);
        let is_admin = self.state.read(cx).is_admin();
        let online = self.state.read(cx).online;

        let mut page = div()
            .id("podcasts-screen")
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .gap(rems(0.75))
            .px(rems(1.5))
            .py(rems(1.))
            .font_family(theme::font(cx))
            .text_color(theme::text(cx))
            .key_context("Neezer");

        // -- the head: the title, then the admin's field -------------------
        page = page.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .child(
                    icon(IconName::Mic)
                        .size(rems(1.5))
                        .text_color(theme::accent(cx)),
                )
                .child(
                    div()
                        .text_size(rems(1.375))
                        .font_weight(FontWeight::BOLD)
                        .child("Podcasts"),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(rems(0.75))
                        .font_family(theme::mono(cx))
                        .text_color(theme::secondary(cx))
                        .child(SharedString::from(format!(
                            "{} abonnement{}",
                            self.channels.len(),
                            if self.channels.len() > 1 { "s" } else { "" }
                        ))),
                ),
        );

        if is_admin {
            let field = self
                .input
                .as_ref()
                .map(|input| gpui_omarchy::input("podcasts-search", input, window, cx));
            let mut row = div().flex().items_center().gap(rems(0.375)).w_full();
            if let Some(field) = field {
                row = row.child(div().flex_1().min_w_0().max_w(rems(46.)).child(field));
            }
            if self.subscribing.is_some() {
                row = row.child(
                    div()
                        .text_size(rems(0.75))
                        .text_color(theme::accent(cx))
                        .child("Ajout…"),
                );
            } else if !self.query.trim().is_empty() {
                row = row.child(
                    button(
                        "podcasts-retry",
                        "Rechercher",
                        gpui_omarchy::ButtonVariant::Secondary,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                );
            }
            page = page.child(row);
        }

        // -- the body: results while a search is on, subscriptions otherwise --
        let searching = self.searching || self.searched.is_some();
        let mut body = div()
            .id("podcasts-body")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(rems(0.5))
            .overflow_y_scroll()
            .track_scroll(&self.scroll);

        if searching {
            body = body.child(subsection(
                "Résultats",
                self.results.len(),
                self.searching,
                cx,
            ));
            if let Some(error) = self.search_failed.clone() {
                let term = self.searched.clone().unwrap_or_default();
                body =
                    body.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(rems(0.25))
                            .child(
                                div()
                                    .text_size(rems(0.75))
                                    .text_color(theme::danger(cx))
                                    .child(SharedString::from(format!(
                                        "La recherche a échoué : {}",
                                        card::cause(&error)
                                    ))),
                            )
                            .child(
                                div()
                                    .id("podcasts-search-retry")
                                    .self_start()
                                    .flex()
                                    .items_center()
                                    .gap(rems(0.25))
                                    .h(rems(1.75))
                                    .px(rems(0.5))
                                    .border_1()
                                    .border_color(theme::border(cx))
                                    .text_size(rems(0.75))
                                    .text_color(theme::accent(cx))
                                    .cursor_pointer()
                                    .hover(|style| style.bg(theme::chrome(cx)))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.search(term.clone(), cx)
                                    }))
                                    .child(icon(IconName::RefreshCw).size(rems(0.75)))
                                    .child("Réessayer"),
                            ),
                    );
            } else if self.searching && self.results.is_empty() {
                body = body.child(card::skeleton(6, px(148.), cx));
            } else if self.results.is_empty() {
                body = body.child(card::empty(
                    IconName::Mic,
                    "Aucun podcast trouvé",
                    &format!(
                        "Aucun podcast pour « {} » chez le fournisseur.",
                        self.searched.clone().unwrap_or_default()
                    ),
                    cx,
                    None,
                ));
            } else {
                let mut grid = div().flex().flex_wrap().gap(rems(0.75)).w_full();
                for (index, item) in self.results.iter().enumerate() {
                    let selected = index == self.cursor;
                    let busy = self
                        .subscribing
                        .as_deref()
                        .is_some_and(|url| url.contains(&item.deezer_id));
                    let file = item
                        .cover
                        .as_deref()
                        .and_then(|url| cover::file_in(&self.covers, url));
                    let card = item.clone();
                    grid = grid.child(
                        self.card_frame(selected, cx)
                            .id(("result-card", index))
                            .debug_selector(move || format!("result-card-{index}"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.cursor = index;
                                this.subscribe_card(&card, cx)
                            }))
                            .child(cover::view(
                                file.as_ref(),
                                item.cover.as_deref(),
                                &item.title,
                                Kind::Podcast,
                                px(148.),
                                cx,
                            ))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(rems(0.125))
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(rems(0.8125))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(item.title.clone()),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(rems(0.25))
                                            .text_size(rems(0.6875))
                                            .text_color(if busy {
                                                theme::accent(cx)
                                            } else {
                                                theme::secondary(cx)
                                            })
                                            .child(
                                                icon(if busy {
                                                    IconName::LoaderCircle
                                                } else {
                                                    IconName::Plus
                                                })
                                                .size(rems(0.75)),
                                            )
                                            .when(busy, |row| row.child("Ajout…"))
                                            .when(!busy, |row| row.child("S'abonner")),
                                    ),
                            ),
                    );
                }
                body = body.child(grid);
            }
        } else if self.loading && self.channels.is_empty() {
            body = body.child(card::skeleton(8, px(148.), cx));
        } else if let Some(error) = self.failed.clone() {
            body = body.child(card::failed(
                "Podcasts indisponibles",
                if !online {
                    "Le serveur est injoignable : les abonnements sont sur le serveur.".to_owned()
                } else {
                    card::cause(&error).to_string()
                },
                cx,
                |this: &mut Self, _, cx| this.reload(cx),
            ));
        } else if self.channels.is_empty() {
            body = body.child(card::empty(
                IconName::Mic,
                "Aucun abonnement",
                if is_admin {
                    "Cherchez un podcast ci-dessus, ou collez un lien Deezer, pour vous abonner."
                } else {
                    "Aucun podcast suivi pour l'instant."
                },
                cx,
                None,
            ));
        } else {
            body = body.child(subsection(
                "Mes podcasts",
                self.channels.len(),
                self.loading,
                cx,
            ));
            let mut grid = div().flex().flex_wrap().gap(rems(0.75)).w_full();
            for (index, show) in self.channels.iter().enumerate() {
                let selected = index == self.cursor;
                let file = show
                    .cover
                    .as_deref()
                    .and_then(|url| cover::file_in(&self.covers, url));
                let id = show.id.as_str().to_owned();
                let subtitle = format!(
                    "{} épisode{}{}",
                    show.episode_count,
                    if show.episode_count > 1 { "s" } else { "" },
                    if show.auto_archive { " · auto" } else { "" }
                );
                grid = grid.child(
                    self.card_frame(selected, cx)
                        .id(("show-card", index))
                        .debug_selector(move || format!("show-card-{index}"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.cursor = index;
                            this.open_show(&id, cx)
                        }))
                        .child(cover::view(
                            file.as_ref(),
                            show.cover.as_deref(),
                            &show.title,
                            Kind::Podcast,
                            px(148.),
                            cx,
                        ))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(rems(0.125))
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(rems(0.8125))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(show.title.clone()),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(rems(0.25))
                                        .text_size(rems(0.6875))
                                        .text_color(theme::secondary(cx))
                                        .child(SharedString::from(subtitle)),
                                )
                                .when(show.local, |meta| {
                                    meta.child(
                                        div()
                                            .text_size(rems(0.625))
                                            .text_color(theme::warning(cx))
                                            .child("Local"),
                                    )
                                })
                                .child(
                                    div()
                                        .debug_selector({
                                            let key = format!("show-download-{}", show.id.as_str());
                                            move || key.clone()
                                        })
                                        .child(
                                            icon_button(
                                                ElementId::Name(
                                                    format!("show-download-{}", show.id.as_str())
                                                        .into(),
                                                ),
                                                IconName::Download,
                                                format!("Télécharger l'émission {}", show.title),
                                                ButtonVariant::Secondary,
                                                cx,
                                            )
                                            .on_click(
                                                cx.listener(move |this, _, _, cx| {
                                                    cx.stop_propagation();
                                                    this.download_show(index, cx);
                                                }),
                                            ),
                                        ),
                                ),
                        ),
                );
            }
            body = body.child(grid);
        }

        page = page.child(body);
        div()
            .size_full()
            .flex()
            .justify_center()
            .child(page.max_w(px(PAGE_MAX)))
            .into_any_element()
    }
}

impl Screen {
    /// The square a card is drawn in, with the keyboard's ring on the focused one.
    fn card_frame(&self, selected: bool, cx: &App) -> Div {
        div()
            .w(px(172.))
            .flex()
            .flex_col()
            .gap(rems(0.5))
            .p(rems(0.5))
            .border_1()
            .border_color(if selected {
                theme::focus_ring(cx)
            } else {
                theme::shell(cx)
            })
            .bg(if selected {
                theme::chrome(cx)
            } else {
                theme::shell(cx)
            })
            .cursor_pointer()
            .hover(|style| style.bg(theme::chrome(cx)))
    }
}

/// A section's own line: its name and a count (a job still running says so).
fn subsection(title: &str, count: usize, busy: bool, cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .gap(rems(0.375))
        .w_full()
        .child(
            div()
                .text_size(rems(0.875))
                .font_weight(FontWeight::BOLD)
                .child(title.to_owned()),
        )
        .child(
            div()
                .text_size(rems(0.6875))
                .font_family(theme::mono(cx))
                .text_color(theme::secondary(cx))
                .child(count.to_string()),
        )
        .when(busy, |row| {
            row.child(
                icon(IconName::LoaderCircle)
                    .size(rems(0.75))
                    .text_color(theme::accent(cx)),
            )
        })
}

/// The width one card takes, for a test that counts a row of them.
pub const CARD: Pixels = px(172.);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::card::stub;

    const ME: &str = r#"{"user":{"id":"u1","name":"bench","admin":true,"access":{},"sources":[]}}"#;

    fn show_body() -> String {
        r#"{"podcasts":[
            {"id":"c1","deezer_id":"9400001","title":"Les Histoires","description":"",
             "cover":null,"subscribed":true,"local":false,"status":"ok",
             "episode_count":8,"archived_count":8,"auto_archive":true,
             "auto_transcribe":false,"transcribe_language":"auto"},
            {"id":"c2","deezer_id":"9400002","title":"Autre émission","description":"",
             "cover":null,"subscribed":true,"local":true,"status":"ok",
             "episode_count":2,"archived_count":2,"auto_archive":false,
             "auto_transcribe":false,"transcribe_language":"auto"}]}"#
            .to_owned()
    }

    /// One request paints the subscriptions, and a click (or `Entrée` on the
    /// cursor) opens the show: the grid's two gestures are the same one.
    #[gpui::test]
    async fn the_subscriptions_are_one_request_and_a_card_opens_its_show(
        cx: &mut gpui::TestAppContext,
    ) {
        let body = show_body();
        let stub = stub::Stub::start(&[
            ("/api/login", ME),
            ("/api/me", ME),
            ("/api/podcasts", body.as_str()),
        ]);
        let (state, runtime) = stub::logged_in(cx, &stub);
        let (screen, cx) = cx.add_window_view({
            let state = state.clone();
            move |_window, cx| Screen::new(Route::Podcasts, state, cx)
        });
        stub::until(&runtime, cx, "the subscriptions", |cx| {
            !screen.read(cx).channels.is_empty()
        });
        assert_eq!(stub.count("/api/podcasts"), 1, "one page, one request");
        cx.update(|_, cx| {
            let screen = screen.read(cx);
            assert_eq!(screen.channels.len(), 2);
            assert!(screen.channels[0].auto_archive);
            assert!(screen.channels[1].local);
        });

        // `Entrée` on the cursor opens the show — through the app-level list
        // verbs, so the keyboard and the click are one path.
        cx.update(|_, cx| {
            screen.update(cx, |screen, cx| screen.activate_cursor(cx));
            assert!(
                matches!(state.read(cx).nav.current(), Some(Route::Show { id }) if id == "c1"),
                "the focused card is the show that opens"
            );
        });
        runtime.shutdown_background();
        stub.stop();
    }

    /// The field asks once per settled term (300 ms), and the results replace
    /// each other: a slow answer for an older word never paints over a newer one.
    #[gpui::test]
    async fn a_typed_term_costs_one_request_after_it_settles(cx: &mut gpui::TestAppContext) {
        let body = show_body();
        let found = r#"{"podcasts":[{"deezer_id":"9400009","title":"Trouvé","description":"",
            "cover":null,"fans":3}]}"#;
        let stub = stub::Stub::start(&[
            ("/api/login", ME),
            ("/api/me", ME),
            ("/api/podcasts", body.as_str()),
            ("/api/search/podcasts", found),
        ]);
        let (state, runtime) = stub::logged_in(cx, &stub);
        let (screen, cx) = cx.add_window_view({
            let state = state.clone();
            move |_window, cx| Screen::new(Route::Podcasts, state, cx)
        });
        stub::until(&runtime, cx, "the subscriptions", |cx| {
            !screen.read(cx).channels.is_empty()
        });

        cx.update(|_, cx| {
            screen.update(cx, |screen, cx| screen.search("trouvé".into(), cx));
        });
        stub::until(&runtime, cx, "the results", |_| {
            stub.count("/api/search/podcasts") == 1
        });
        stub::until(&runtime, cx, "the answer", |cx| {
            screen.read(cx).results.len() == 1
        });
        assert_eq!(
            stub.queries("/api/search/podcasts"),
            vec!["q=trouv%C3%A9".to_owned()],
            "the term travels percent-encoded, once"
        );
        runtime.shutdown_background();
        stub.stop();
    }

    /// Only a Deezer link subscribes on `Entrée`; a plain word searches.
    #[test]
    fn only_a_deezer_link_is_a_subscription() {
        assert!(is_deezer_link("https://www.deezer.com/fr/show/9400001"));
        assert!(is_deezer_link("https://deezer.com/episode/123"));
        assert!(!is_deezer_link("les histoires"));
        assert!(!is_deezer_link("https://example.com/show/1"));
    }
}
