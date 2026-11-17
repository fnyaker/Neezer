//! Historique: what this person actually played, read from their own log.
//!
//! `routes/History.svelte` and `lib/playfilter.js` are the reference:
//!
//! - **One page is one request, with a keyset cursor** `(before, before_id)`:
//!   a play recorded while somebody is reading page one can never push an event
//!   off page two. « Charger plus » appends and keeps the rows already read.
//! - **The rows are windowed**: only the day headers and events on screen are
//!   built (44 px a header, 72 px an event — the SPA's exact geometry, so the
//!   virtual offsets need no measurement).
//! - **The row says what the verdict was with a glyph AND a word** (Écouté /
//!   Partiel / Passé), never colour alone, and a failure is a cause and
//!   « Réessayer », never a spinner.
//! - **Personal data stays on the server**: this screen writes nothing to the
//!   device, and `/history` is not cacheable (`lib/apicache.js`).
//!
//! Requests: `GET /history?limit=&outcome=` on open, once per filter, and once
//! per « Charger plus ». Nothing else.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::base::{ScrollbarAxis, VirtualListScrollHandle};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement,
    Render, ScrollStrategy, SharedString, Size, StatefulInteractiveElement as _, Styled,
    Subscription, Window, div, px, rems, size,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon, scrollbar, virtual_list};
use nz_api::ApiError;
use nz_model::{History, HistoryCursor, HistoryEvent, HistorySummary, Track};

use crate::actions::{ListFirst, ListLast, ListPageDown, ListPageUp};
use crate::nav::Route;
use crate::state::AppState;
use crate::theme;
use crate::toast::ToastKind;
use crate::widgets::card;
use crate::widgets::cover::{self, Covers, Kind};
use crate::wiring::events::{self, Sender};

/// How many events one page holds (`History.svelte#PAGE`).
pub const PAGE: u32 = 50;
/// A day header's height (`DAY_H`).
pub const DAY_H: f32 = 44.;
/// An event row's height (`EVENT_H`).
pub const EVENT_H: f32 = 72.;

/// The filter chips, in the SPA's order (`FILTERS`).
pub const FILTERS: [(&str, &str); 3] = [("", "Tout"), ("listen", "Écoutes"), ("skip", "Sauts")];

enum Msg {
    Page {
        first: bool,
        answer: Box<Result<History, ApiError>>,
    },
    Cover(String, Option<PathBuf>),
}

/// One line of the windowed list: a day, or an event of `events`.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Day {
        key: String,
        label: String,
        listens: u32,
        skips: u32,
        seconds: u64,
    },
    Event(usize),
}

pub struct Screen {
    route: Route,
    state: Entity<AppState>,
    events: Vec<HistoryEvent>,
    summary: Option<HistorySummary>,
    next: Option<HistoryCursor>,
    /// `""`, `listen` or `skip` — the server's own `outcome` vocabulary.
    filter: &'static str,
    loading: bool,
    error: Option<ApiError>,
    more_error: Option<ApiError>,
    /// The flattened rows and their heights, rebuilt when the data changes
    /// (never on a paint).
    rows: Vec<Row>,
    sizes: Rc<Vec<Size<gpui_kit::Pixels>>>,
    scroll: VirtualListScrollHandle,
    visible: Rc<Cell<(usize, usize)>>,
    covers: Rc<RefCell<Covers>>,
    replies: Sender<Msg>,
    _subscriptions: Vec<Subscription>,
}

impl Screen {
    pub fn new(route: Route, state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let (tx, rx) = events::channel::<Msg>();
        let mut screen = Self {
            route,
            state: state.clone(),
            events: Vec::new(),
            summary: None,
            next: None,
            filter: "",
            loading: true,
            error: None,
            more_error: None,
            rows: Vec::new(),
            sizes: Rc::new(Vec::new()),
            scroll: VirtualListScrollHandle::new(),
            visible: Rc::new(Cell::new((0, 0))),
            covers: Rc::new(RefCell::new(Covers::new())),
            replies: tx,
            _subscriptions: vec![cx.observe(&state, |_, _, cx| cx.notify())],
        };
        screen.drain(rx, cx);
        screen.load(true, cx);
        screen.bind_page_keys(cx);
        screen
    }

    /// **The history scrolls from the keyboard** (`Page suivante`, `Page
    /// précédente`, `Début`, `Fin`). The page has no cursor — a click opens a
    /// track, nothing is "selected" — so these move the VIEW, as a browser
    /// does, and they are the only way through 37 000 listens without a wheel
    /// (measured: before this, the keys did nothing at all on this page).
    ///
    /// Registered on the APP with a weak handle, like every list verb: a screen
    /// never takes the focus (D8b), and a screen that is gone answers nothing.
    fn bind_page_keys(&mut self, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        macro_rules! on {
            ($action:ty, $handler:expr) => {{
                let weak = weak.clone();
                let handler = $handler;
                gpui_kit::App::on_action(cx, move |_: &$action, cx: &mut gpui_kit::App| {
                    weak.update(cx, |this, cx| handler(this, cx)).ok();
                });
            }};
        }
        on!(ListPageDown, |this: &mut Self, cx: &mut Context<Self>| {
            this.shift_page(1);
            cx.notify();
        });
        on!(ListPageUp, |this: &mut Self, cx: &mut Context<Self>| {
            this.shift_page(-1);
            cx.notify();
        });
        on!(ListFirst, |this: &mut Self, cx: &mut Context<Self>| {
            this.scroll.scroll_to_item(0, ScrollStrategy::Top);
            cx.notify();
        });
        on!(ListLast, |this: &mut Self, cx: &mut Context<Self>| {
            let last = this.rows.len().saturating_sub(1);
            this.scroll.scroll_to_item(last, ScrollStrategy::Bottom);
            cx.notify();
        });
    }

    /// A page is the rows the last paint built (`visible`), so `Page suivante`
    /// moves exactly what the person sees — no per-row measurement.
    fn shift_page(&mut self, pages: i64) {
        let (start, end) = self.visible.get();
        let count = self.rows.len();
        if count == 0 {
            return;
        }
        let page = end.saturating_sub(start).max(1) as i64;
        let target = (start as i64 + pages * page).clamp(0, count as i64 - 1) as usize;
        // `Center`, not `Top`: base's `Top`/`Bottom` are "scroll into view"
        // (minimal travel), so a row already on screen would not move the view
        // at all (measured: `Page suivante` moved 176 px instead of a page).
        self.scroll.scroll_to_item(target, ScrollStrategy::Center);
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

    /// The first page of a filter, or the next page of the one on screen.
    fn load(&mut self, first: bool, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        if !first && self.next.is_none() {
            return;
        }
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.replies.clone();
        let filter = self.filter;
        let cursor = if first { None } else { self.next.clone() };
        if first {
            self.loading = true;
            self.error = None;
            self.more_error = None;
        } else {
            self.loading = true;
            self.more_error = None;
        }
        runtime.spawn(async move {
            let outcome = (!filter.is_empty()).then_some(filter);
            let answer = api
                .history(
                    cursor.as_ref().map(|cursor| cursor.before.as_str()),
                    cursor.as_ref().map(|cursor| cursor.before_id),
                    PAGE,
                    outcome,
                )
                .await;
            tx.send(Msg::Page {
                first,
                answer: Box::new(answer),
            });
        });
        cx.notify();
    }

    fn on_msg(&mut self, msg: Msg, cx: &mut Context<Self>) {
        match msg {
            Msg::Page { first, answer } => {
                self.loading = false;
                match *answer {
                    Ok(page) => {
                        self.summary = Some(page.summary);
                        self.next = page.next;
                        if first {
                            self.events = page.events;
                        } else {
                            // A cursor is exact, but a defensive id check makes a
                            // duplicate impossible to show.
                            let have: Vec<i64> = self.events.iter().map(|event| event.id).collect();
                            for event in page.events {
                                if !have.contains(&event.id) {
                                    self.events.push(event);
                                }
                            }
                        }
                        self.rebuild();
                    }
                    Err(error) => {
                        if first {
                            self.events.clear();
                            self.rows.clear();
                            self.sizes = Rc::new(Vec::new());
                            self.error = Some(error);
                        } else {
                            self.more_error = Some(error);
                        }
                    }
                }
            }
            Msg::Cover(url, file) => self.covers.borrow_mut().resolved(&url, file),
        }
        cx.notify();
    }

    fn set_filter(&mut self, filter: &'static str, cx: &mut Context<Self>) {
        if filter == self.filter {
            return;
        }
        self.filter = filter;
        self.events.clear();
        self.rows.clear();
        self.sizes = Rc::new(Vec::new());
        self.next = None;
        self.error = None;
        self.more_error = None;
        self.scroll = VirtualListScrollHandle::new();
        self.load(true, cx);
    }

    /// The virtual list is rebuilt from the rows: heights are the SPA's own
    /// constants, so a header and an event are exact without measuring.
    fn rebuild(&mut self) {
        let rows = flatten(&self.events);
        self.sizes = Rc::new(
            rows.iter()
                .map(|row| {
                    size(
                        px(0.),
                        px(match row {
                            Row::Day { .. } => DAY_H,
                            Row::Event(_) => EVENT_H,
                        }),
                    )
                })
                .collect(),
        );
        self.rows = rows;
        self.visible.set((0, 0));
    }

    fn covers_for_visible(&mut self, range: std::ops::Range<usize>, cx: &mut Context<Self>) {
        let mut asked: Vec<String> = Vec::new();
        for index in range {
            let Some(Row::Event(at)) = self.rows.get(index) else {
                continue;
            };
            if let Some(event) = self.events.get(*at)
                && let Some(cover) = event.track.album.cover.clone()
            {
                asked.push(cover);
            }
        }
        let snapshot = self.state.read(cx);
        let mut covers = self.covers.borrow_mut();
        for url in asked {
            let for_request = url.clone();
            let for_message = url.clone();
            let tx = self.replies.clone();
            covers.want(snapshot, &for_request, move |file| {
                tx.send(Msg::Cover(for_message, file))
            });
        }
    }

    /// A row's click: the title plays and turns into its radio
    /// (`lib/actions.js#playThenRadio`), so the queue does not stop after one
    /// title — the Flow service keeps it fed.
    fn play(&mut self, track: &Track, cx: &mut Context<Self>) {
        let entry = crate::state::entry_from_track(track);
        let reference = entry.reference().map(str::to_owned).unwrap_or_default();
        let context = nz_player::Context {
            kind: "radio".to_owned(),
            id: (!reference.is_empty()).then_some(reference),
            title: None,
        };
        self.play_track(entry, context, cx);
    }

    /// Play one row through the one transport path.
    fn play_track(
        &mut self,
        entry: nz_player::Entry,
        context: nz_player::Context,
        cx: &mut Context<Self>,
    ) {
        self.state.update(cx, |state, cx| {
            state.command(
                nz_player::Command::play_queue(vec![entry], 0, Some(context)),
                cx,
            );
        });
    }

    fn open(&mut self, route: Route, cx: &mut Context<Self>) {
        self.state.update(cx, |state, _| state.nav.push(route));
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
}

impl Render for Screen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let visible = self.visible.get();
        self.covers_for_visible(visible.0..visible.1, cx);

        let root = div()
            .id("history-scroll")
            .size_full()
            .flex()
            .flex_col()
            .min_w(px(0.))
            .min_h(px(0.))
            .p(rems(1.5))
            .gap(rems(0.75))
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
            .gap(rems(0.75));

        // The 30-day summary: three figures and the two tops.
        if let Some(summary) = self.summary.clone()
            && !empty_history(&self.events, &summary)
        {
            page = page.child(self.summary_card(&summary, cx));
        }

        // The filters and the road to the year in review.
        let mut filters = div().flex().items_center().gap(rems(0.375));
        for (id, label) in FILTERS {
            let active = self.filter == id;
            filters = filters.child(card::chip(
                format!("history-filter-{label}").into(),
                label,
                active,
                cx,
                move |this: &mut Self, _, cx| this.set_filter(id, cx),
            ));
        }
        filters = filters.child(div().flex_1()).child(card::chip(
            "history-recap".into(),
            "Mon récap",
            false,
            cx,
            |this: &mut Self, _, cx| this.open(Route::Recap { year: None }, cx),
        ));
        page = page.child(filters);

        if let Some(error) = &self.error {
            page = page.child(card::failed(
                "Historique indisponible",
                format!("{} — réessayez dans un instant.", card::cause(error)),
                cx,
                |this: &mut Self, _, cx| this.load(true, cx),
            ));
        } else if self.loading && self.events.is_empty() {
            page = page.child(
                div()
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child("Lecture de votre historique…"),
            );
        } else if self.events.is_empty() {
            let (title, body) = match self.filter {
                "listen" => (
                    "Aucune écoute dans l'historique",
                    "Changez de filtre pour revoir tout votre historique.",
                ),
                "skip" => (
                    "Aucun saut dans l'historique",
                    "Changez de filtre pour revoir tout votre historique.",
                ),
                _ => (
                    "Aucune écoute pour l'instant",
                    "Lancez un titre : ce que vous écoutez et ce que vous passez apparaîtra ici, \
                     jour par jour.",
                ),
            };
            page = page.child(card::empty(
                IconName::Timer,
                title,
                body,
                cx,
                Some(if self.filter.is_empty() {
                    button("history-search", "Explorer", ButtonVariant::Primary, cx)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open(
                                Route::Search {
                                    query: String::new(),
                                },
                                cx,
                            )
                        }))
                        .into_any_element()
                } else {
                    button(
                        "history-all",
                        "Voir tout l'historique",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.set_filter("", cx)))
                    .into_any_element()
                }),
            ));
        } else {
            // The windowed list owns the height: one scroller, never two. The
            // closure reads the screen's own rows (no copy of the log on a
            // paint) and builds only the lines on screen.
            let entity = cx.entity();
            let sizes = Rc::clone(&self.sizes);
            let covers = Rc::clone(&self.covers);
            let visible_cell = Rc::clone(&self.visible);
            let scroll = self.scroll.clone();
            let view = virtual_list(
                entity,
                gpui_kit::ElementId::Name("history-rows".into()),
                sizes,
                move |this, range, _window, cx| {
                    visible_cell.set((range.start, range.end));
                    range
                        .filter_map(|index| match this.rows.get(index).cloned()? {
                            Row::Day {
                                label,
                                listens,
                                skips,
                                seconds,
                                ..
                            } => Some(day_row(&label, listens, skips, seconds, cx)),
                            Row::Event(at) => {
                                let event = this.events.get(at).cloned()?;
                                let file = event
                                    .track
                                    .album
                                    .cover
                                    .as_deref()
                                    .and_then(|url| covers.borrow().file(url).cloned());
                                Some(event_row(&event, file, cx))
                            }
                        })
                        .collect()
                },
                cx,
            )
            // The handle the screen keeps, handed to the list: without it the
            // list builds its own on every frame, starts at offset 0 each
            // paint, and neither the wheel nor `scroll_to_item` moves anything
            // (v1.6.1's rule, `widgets::track_list`).
            .track_scroll(&scroll)
            .h_full();
            page = page.child(div().relative().flex_1().min_h(px(0.)).child(view).child(
                scrollbar(
                    "history-scrollbar",
                    ScrollbarAxis::Vertical,
                    &scroll,
                    window,
                    cx,
                ),
            ));
            // The end of the list: one request per click, the rows already read
            // never move.
            let mut more = div().flex().items_center().gap(rems(0.5)).flex_wrap();
            if self.next.is_some() {
                let label = if self.loading {
                    "Chargement…".to_owned()
                } else {
                    "Charger plus".to_owned()
                };
                more = more.child(
                    button("history-more", label, ButtonVariant::Secondary, cx)
                        .disabled(self.loading)
                        .on_click(cx.listener(|this, _, _, cx| this.load(false, cx))),
                );
            } else {
                more = more.child(
                    div()
                        .text_size(rems(0.75))
                        .text_color(theme::secondary(cx))
                        .child("Début de votre historique."),
                );
            }
            if let Some(error) = &self.more_error {
                more = more.child(
                    div()
                        .text_size(rems(0.75))
                        .text_color(theme::danger(cx))
                        .child(format!(
                            "Impossible de charger la suite : {}.",
                            card::cause(error)
                        )),
                );
            }
            page = page.child(more);
        }

        root.child(page).into_any_element()
    }
}

impl Screen {
    fn summary_card(&mut self, summary: &HistorySummary, cx: &mut Context<Self>) -> AnyElement {
        let mut stats = div().flex().items_center().gap(rems(0.5));
        for (value, label) in [
            (summary.listens, "écoutes"),
            (summary.skips, "sauts"),
            (summary.minutes, "minutes"),
        ] {
            stats = stats.child(
                div()
                    .flex()
                    .flex_col()
                    .py(rems(0.5))
                    .px(rems(0.75))
                    .bg(theme::sidebar(cx))
                    .child(
                        div()
                            .text_size(rems(1.25))
                            .font_weight(FontWeight::BOLD)
                            .child(value.to_string()),
                    )
                    .child(
                        div()
                            .text_size(rems(0.6875))
                            .text_color(theme::secondary(cx))
                            .child(label),
                    ),
            );
        }

        let mut tops = div().flex().items_start().gap(rems(1.25)).flex_wrap();
        if !summary.top_artists.is_empty() {
            let mut rail = div().flex().items_center().gap(rems(0.25));
            for artist in summary.top_artists.iter().take(8) {
                let file = artist
                    .cover
                    .as_deref()
                    .and_then(|url| cover::file_in(&self.covers, url));
                let id = artist.id.clone();
                rail = rail.child(
                    div()
                        .id(SharedString::from(format!(
                            "history-artist-{}",
                            artist.name
                        )))
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(rems(0.125))
                        .w(px(92.))
                        .p(rems(0.25))
                        .cursor_pointer()
                        .hover(|style| style.bg(theme::sidebar(cx)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !id.is_empty() {
                                this.open(Route::Artist { id: id.clone() }, cx);
                            }
                        }))
                        .child(cover::view(
                            file.as_ref(),
                            artist.cover.as_deref(),
                            &artist.name,
                            Kind::Artist,
                            px(44.),
                            cx,
                        ))
                        .child(
                            div()
                                .truncate()
                                .max_w(px(84.))
                                .text_size(rems(0.6875))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(artist.name.clone()),
                        )
                        .child(
                            div()
                                .font_family(theme::mono(cx))
                                .text_size(rems(0.5625))
                                .text_color(theme::secondary(cx))
                                .child(plural(artist.count, "écoute", "écoutes")),
                        ),
                );
            }
            tops = tops.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(0.375))
                    .child(
                        div()
                            .font_family(theme::mono(cx))
                            .text_size(rems(0.5625))
                            .text_color(theme::secondary(cx))
                            .child("ARTISTES LES PLUS ÉCOUTÉS"),
                    )
                    .child(rail),
            );
        }
        if !summary.top_tracks.is_empty() {
            let mut list = div().flex().flex_col();
            for row in summary.top_tracks.iter().take(5) {
                let track = row.track.clone();
                let count = row.count;
                list = list.child(
                    div()
                        .id(SharedString::from(format!("history-top-{}", track.title)))
                        .flex()
                        .items_center()
                        .gap(rems(0.5))
                        .h(rems(2.25))
                        .px(rems(0.375))
                        .cursor_pointer()
                        .hover(|style| style.bg(theme::sidebar(cx)))
                        .on_click(cx.listener(move |this, _, _, cx| this.play(&track, cx)))
                        .child(
                            div()
                                .truncate()
                                .flex_1()
                                .text_size(rems(0.75))
                                .child(row.track.title.clone()),
                        )
                        .child(
                            div()
                                .font_family(theme::mono(cx))
                                .text_size(rems(0.5625))
                                .text_color(theme::secondary(cx))
                                .child(plural(count, "écoute", "écoutes")),
                        ),
                );
            }
            tops = tops.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(0.375))
                    .min_w(px(320.))
                    .child(
                        div()
                            .font_family(theme::mono(cx))
                            .text_size(rems(0.5625))
                            .text_color(theme::secondary(cx))
                            .child("TITRES LES PLUS ÉCOUTÉS"),
                    )
                    .child(list),
            );
        }

        div()
            .flex()
            .flex_col()
            .gap(rems(0.75))
            .p(rems(1.))
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
                            .text_size(rems(0.9375))
                            .font_weight(FontWeight::BOLD)
                            .child(format!("{} derniers jours", summary.days)),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .font_family(theme::mono(cx))
                            .text_size(rems(0.5625))
                            .text_color(theme::secondary(cx))
                            .child("VOTRE ÉCOUTE"),
                    ),
            )
            .child(stats)
            .child(tops)
            .into_any_element()
    }
}

fn day_row<V: Render>(
    label: &str,
    listens: u32,
    skips: u32,
    seconds: u64,
    cx: &mut Context<V>,
) -> AnyElement {
    let mut meta = div()
        .flex()
        .items_center()
        .gap(rems(0.5))
        .text_size(rems(0.6875))
        .text_color(theme::secondary(cx))
        .child(plural(listens, "écoute", "écoutes"));
    if skips > 0 {
        meta = meta.child(div().child(format!("· {}", plural(skips, "saut", "sauts"))));
    }
    if seconds > 0 {
        meta = meta.child(div().child(format!("· {} min", seconds / 60)));
    }
    div()
        .h(px(DAY_H))
        .w_full()
        .flex()
        .items_center()
        .gap(rems(0.5))
        .child(
            div()
                .text_size(rems(0.8125))
                .font_weight(FontWeight::BOLD)
                .child(label.to_owned()),
        )
        .child(meta)
        .into_any_element()
}

fn event_row(event: &HistoryEvent, file: Option<PathBuf>, cx: &mut Context<Screen>) -> AnyElement {
    let track = event.track.clone();
    let (glyph, word) = match verdict(event.outcome) {
        "listen" => (IconName::Check, "Écouté"),
        "skip" => (IconName::SkipForward, "Passé"),
        _ => (IconName::CircleMinus, "Partiel"),
    };
    let when = at_time(event.at.as_deref());
    let listened = if event.listened > 0 {
        format!(
            "{} écoutés",
            crate::widgets::track_list::duration_label(event.listened)
        )
    } else {
        "Pas écouté".to_owned()
    };
    let artist_name = track
        .display_artist
        .clone()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| track.artist.name.clone());
    let artist_id = track.artist.deezer_id.as_str().to_owned();
    let row_track = track.clone();
    let artist_link = artist_id.clone();
    let artist_name_for_link = artist_name.clone();
    let row = div()
        .id(SharedString::from(format!("history-event-{}", event.id)))
        .h(px(EVENT_H))
        .w_full()
        .flex()
        .items_center()
        .gap(rems(0.625))
        .border_b_1()
        .border_color(theme::border(cx).opacity(0.4))
        .px(rems(0.25))
        .cursor_pointer()
        .hover(|style| style.bg(theme::chrome(cx)))
        .on_click(cx.listener(move |this, _, _, cx| {
            // `lib/actions.js#playThenRadio`: one title, then its radio, so the
            // music does not stop after the row.
            let entry = crate::state::entry_from_track(&row_track);
            let reference = entry.reference().map(str::to_owned).unwrap_or_default();
            let context = nz_player::Context {
                kind: "radio".to_owned(),
                id: (!reference.is_empty()).then_some(reference),
                title: None,
            };
            this.play_track(entry, context, cx);
        }))
        .child(cover::view(
            file.as_ref(),
            track.album.cover.as_deref(),
            &track.title,
            Kind::Album,
            px(48.),
            cx,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .gap(rems(0.0625))
                .child(
                    div()
                        .truncate()
                        .text_size(rems(0.8125))
                        .child(track.title.clone()),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("history-artist-{}", event.id)))
                        .truncate()
                        .text_size(rems(0.6875))
                        .text_color(theme::secondary(cx))
                        .when(!artist_id.is_empty(), |name| {
                            name.cursor_pointer()
                                .hover(|style| style.text_color(theme::text(cx)))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.open(
                                        Route::Artist {
                                            id: artist_link.clone(),
                                        },
                                        cx,
                                    );
                                }))
                        })
                        .child(artist_name_for_link),
                )
                .child(
                    div()
                        .font_family(theme::mono(cx))
                        .text_size(rems(0.5625))
                        .text_color(theme::secondary(cx))
                        .child(if when.is_empty() {
                            listened
                        } else {
                            format!("{when} · {listened}")
                        }),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.25))
                .px(rems(0.375))
                .py(rems(0.0625))
                .border_1()
                .border_color(theme::border(cx))
                .text_size(rems(0.5625))
                .child(icon(glyph).size(rems(0.625)))
                .child(word.to_owned()),
        );
    row.into_any_element()
}

/// The verdict a row carries (`History.svelte#verdict`).
pub fn verdict(outcome: i32) -> &'static str {
    if outcome > 0 {
        "listen"
    } else if outcome < 0 {
        "skip"
    } else {
        "neutral"
    }
}

/// A history with nothing at all does not need a card full of zeros
/// (`History.svelte#emptyHistory`).
pub fn empty_history(events: &[HistoryEvent], summary: &HistorySummary) -> bool {
    events.is_empty()
        && summary.listens == 0
        && summary.skips == 0
        && summary.minutes == 0
        && summary.top_artists.is_empty()
        && summary.top_tracks.is_empty()
}

/// Group the events by day, in the order the server sent them (newest first),
/// and flatten the two kinds of line (`groupByDay` + `flatten`).
pub fn flatten(events: &[HistoryEvent]) -> Vec<Row> {
    let today = today();
    let mut rows: Vec<Row> = Vec::new();
    let mut current: Option<String> = None;
    // The day header's own index: the figures go there, not on the last row
    // pushed (which is an event as soon as the day holds two).
    let mut day_at: Option<usize> = None;
    for (index, event) in events.iter().enumerate() {
        let Some(at) = event.at.as_deref().and_then(parse_at) else {
            continue;
        };
        let key = format!("{:04}-{:02}-{:02}", at.0, at.1, at.2);
        if current.as_deref() != Some(key.as_str()) {
            current = Some(key.clone());
            rows.push(Row::Day {
                label: day_label(at, today),
                key,
                listens: 0,
                skips: 0,
                seconds: 0,
            });
            day_at = Some(rows.len() - 1);
        }
        if let Some(at) = day_at
            && let Some(Row::Day {
                listens,
                skips,
                seconds,
                ..
            }) = rows.get_mut(at)
        {
            if event.outcome > 0 {
                *listens += 1;
            } else if event.outcome < 0 {
                *skips += 1;
            }
            *seconds += u64::from(event.listened);
        }
        rows.push(Row::Event(index));
    }
    rows
}

/// "2026-11-13T20:31:00" (a naive server-local ISO time) → (year, month, day).
fn parse_at(value: &str) -> Option<(i32, u32, u32)> {
    let date = value.split(['T', ' ']).next()?;
    let mut parts = date.split('-');
    let year = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    ((1..=12).contains(&month) && (1..=31).contains(&day)).then_some((year, month, day))
}

/// The hour a row prints (`History.svelte#when`).
pub fn at_time(value: Option<&str>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    let Some(time) = value.split(['T', ' ']).nth(1) else {
        return String::new();
    };
    let mut parts = time.split(':');
    let (Some(hour), Some(minute)) = (parts.next(), parts.next()) else {
        return String::new();
    };
    format!("{hour}:{minute}")
}

const WEEKDAYS: [&str; 7] = [
    "Dimanche", "Lundi", "Mardi", "Mercredi", "Jeudi", "Vendredi", "Samedi",
];
const MONTHS: [&str; 12] = [
    "janvier",
    "février",
    "mars",
    "avril",
    "mai",
    "juin",
    "juillet",
    "août",
    "septembre",
    "octobre",
    "novembre",
    "décembre",
];

/// "Aujourd'hui", "Hier", or "Vendredi 13 novembre" (with the year when it is
/// another one) — `dayLabel` in the SPA.
pub fn day_label(at: (i32, u32, u32), today: (i32, u32, u32)) -> String {
    let days = days_from_civil(at.0, at.1, at.2);
    let now = days_from_civil(today.0, today.1, today.2);
    if days == now {
        return "Aujourd'hui".to_owned();
    }
    if days == now - 1 {
        return "Hier".to_owned();
    }
    let weekday = WEEKDAYS[(days + 4).rem_euclid(7) as usize];
    let month = MONTHS
        .get(at.1.saturating_sub(1) as usize)
        .copied()
        .unwrap_or("");
    if at.0 != today.0 {
        format!("{weekday} {} {month} {}", at.2, at.0)
    } else {
        format!("{weekday} {} {month}", at.2)
    }
}

/// The day's date in UTC: the labels above are relative to *today*, and a
/// device's clock is all the app has (`History.svelte` uses `new Date()`).
pub fn today() -> (i32, u32, u32) {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or(0);
    civil_from_days(seconds.div_euclid(86_400))
}

/// Days since 1970-01-01 for a civil date (Hinnant's algorithm).
pub fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year } as i64;
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let mp = (month as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date of a day count (the inverse of [`days_from_civil`]).
pub fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    (year as i32, month as u32, day as u32)
}

/// "3 écoutes" / "1 écoute" — French counts zero as singular.
fn plural(count: u32, one: &str, many: &str) -> String {
    format!("{count} {}", if count > 1 { many } else { one })
}

#[cfg(test)]
mod tests {
    use super::*;
    const ME: &str =
        r#"{"user":{"id":"u1","name":"bench","admin":false,"access":{},"sources":[]}}"#;

    fn page_json() -> String {
        r#"{"events":[
            {"id":3,"at":"2026-11-16T20:31:00","listened":210,"outcome":1,
             "track":{"deezer_id":"3003","title":"Trois","duration":200}},
            {"id":2,"at":"2026-11-16T20:28:00","listened":4,"outcome":-1,
             "track":{"deezer_id":"3002","title":"Deux","duration":200}}],
            "next":{"before":"2026-11-16T20:28:00","before_id":2},
            "summary":{"days":30,"listens":1,"skips":1,"minutes":4,
                       "top_artists":[],"top_tracks":[]}}"#
            .to_owned()
    }

    /// One page is one request, and « Charger plus » appends without ever
    /// showing the same event twice — the cursor is exact, the id check makes a
    /// duplicate impossible.
    #[gpui::test]
    async fn a_page_costs_one_request_and_charger_plus_never_duplicates(
        cx: &mut gpui::TestAppContext,
    ) {
        let page = page_json();
        let stub = crate::widgets::card::stub::Stub::start(&[
            ("/api/login", ME),
            ("/api/me", ME),
            ("/api/history", page.as_str()),
        ]);
        let (state, runtime) = crate::widgets::card::stub::logged_in(cx, &stub);
        let (screen, cx) =
            cx.add_window_view(move |_, cx| Screen::new(Route::History, state.clone(), cx));
        crate::widgets::card::stub::until(&runtime, cx, "the first page", |cx| {
            screen.read(cx).events.len() == 2
        });
        assert_eq!(stub.count("/api/history"), 1);
        cx.update(|_, cx| {
            let screen = screen.read(cx);
            assert_eq!(screen.rows.len(), 3, "a day header and two events");
            assert_eq!(
                screen.rows[0],
                Row::Day {
                    label: day_label((2026, 11, 16), today()),
                    key: "2026-11-16".to_owned(),
                    listens: 1,
                    skips: 1,
                    seconds: 214,
                }
            );
            assert!(screen.next.is_some(), "the cursor is kept");
        });
        // « Charger plus »: the same body again, so the id check is what keeps
        // the list honest — two events, never four.
        cx.update(|_, cx| screen.update(cx, |screen, cx| screen.load(false, cx)));
        crate::widgets::card::stub::until(&runtime, cx, "the second page", |cx| {
            let _ = cx;
            stub.count("/api/history") == 2
        });
        cx.update(|_, cx| {
            assert_eq!(screen.read(cx).events.len(), 2, "no duplicate event");
        });
        runtime.shutdown_background();
        stub.stop();
    }

    /// **`Page suivante`, `Page précédente`, `Début` and `Fin` scroll the
    /// history** (v1.6.1). The page has no cursor, and it had no key handler at
    /// all: on a 37 000-listen log the wheel was the only way down. Measured
    /// here with sixty events in a 800 px window: `Page suivante` moves the
    /// view by the rows it shows, `Fin` reaches the bottom, `Début` the top,
    /// at 1280×800 and in the 660 px pane.
    #[gpui::test]
    async fn the_keys_scroll_the_history(cx: &mut gpui::TestAppContext) {
        let mut events: Vec<String> = Vec::new();
        for index in 0..60 {
            let minute = 59 - index;
            events.push(format!(
                r#"{{"id":{index},"at":"2026-11-16T20:{:02}:00","listened":200,"outcome":1,
                    "track":{{"deezer_id":"30{index:02}","title":"Titre {index}","duration":200}}}}"#,
                minute
            ));
        }
        let page = format!(
            r#"{{"events":[{}],"next":null,"summary":{{"days":30,"listens":60,"skips":0,
                "minutes":200,"top_artists":[],"top_tracks":[]}}}}"#,
            events.join(",")
        );
        let stub = crate::widgets::card::stub::Stub::start(&[
            ("/api/login", ME),
            ("/api/me", ME),
            ("/api/history", page.as_str()),
        ]);
        let (state, runtime) = crate::widgets::card::stub::logged_in(cx, &stub);
        let (screen, cx) =
            cx.add_window_view(move |_, cx| Screen::new(Route::History, state.clone(), cx));
        crate::widgets::card::stub::until(&runtime, cx, "the page", |cx| {
            screen.read(cx).rows.len() > 60
        });
        let paint = |cx: &mut gpui::VisualTestContext| {
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            })
        };
        let offset =
            |cx: &mut gpui::VisualTestContext| cx.update(|_, cx| screen.read(cx).scroll.offset().y);

        for (width, what) in [(1280., "1280×800"), (660., "the 660 px pane")] {
            cx.simulate_resize(size(px(width), px(800.)));
            paint(cx);
            assert_eq!(offset(cx), px(0.), "{what}: a fresh page starts at the top");

            cx.update(|_, cx| cx.dispatch_action(&ListPageDown));
            cx.run_until_parked();
            paint(cx);
            let paged = offset(cx);
            assert!(
                paged < px(-200.),
                "{what}: Page suivante moves the view; measured {paged:?}"
            );

            cx.update(|_, cx| cx.dispatch_action(&ListPageUp));
            cx.run_until_parked();
            paint(cx);
            assert_eq!(offset(cx), px(0.), "{what}: Page précédente comes back");

            cx.update(|_, cx| cx.dispatch_action(&ListLast));
            cx.run_until_parked();
            paint(cx);
            let bottom = offset(cx);
            assert!(bottom < paged, "{what}: Fin reaches the end; {bottom:?}");

            cx.update(|_, cx| cx.dispatch_action(&ListFirst));
            cx.run_until_parked();
            paint(cx);
            assert_eq!(offset(cx), px(0.), "{what}: Début returns to the top");
        }
        runtime.shutdown_background();
        stub.stop();
    }

    /// The filter chips ask the server's own vocabulary, one page at a time.
    #[gpui::test]
    async fn a_filter_change_asks_one_page(cx: &mut gpui::TestAppContext) {
        let page = page_json();
        let stub = crate::widgets::card::stub::Stub::start(&[
            ("/api/login", ME),
            ("/api/me", ME),
            ("/api/history", page.as_str()),
        ]);
        let (state, runtime) = crate::widgets::card::stub::logged_in(cx, &stub);
        let (screen, cx) =
            cx.add_window_view(move |_, cx| Screen::new(Route::History, state.clone(), cx));
        crate::widgets::card::stub::until(&runtime, cx, "the first page", |cx| {
            screen.read(cx).events.len() == 2
        });
        cx.update(|_, cx| screen.update(cx, |screen, cx| screen.set_filter("listen", cx)));
        crate::widgets::card::stub::until(&runtime, cx, "the filtered page", |cx| {
            let _ = cx;
            stub.count("/api/history") == 2
        });
        let queries = stub.queries("/api/history");
        assert!(
            queries
                .last()
                .is_some_and(|query| query.contains("outcome=listen")),
            "the filter is the server's own word: {queries:?}"
        );
        runtime.shutdown_background();
        stub.stop();
    }

    fn event(id: i64, at: &str, outcome: i32, listened: u32) -> HistoryEvent {
        HistoryEvent {
            id,
            at: Some(at.to_owned()),
            listened,
            outcome,
            track: Track {
                deezer_id: nz_model::TrackId::new(format!("{id}")),
                title: format!("Titre {id}"),
                ..Track::default()
            },
            ..HistoryEvent::default()
        }
    }

    /// The cursor pages never repeat an event, and a day's header carries the
    /// day's own figures.
    #[test]
    fn events_group_under_their_day_with_the_days_figures() {
        let events = vec![
            event(3, "2026-11-16T20:31:00", 1, 210),
            event(2, "2026-11-16T20:30:00", -1, 4),
            event(1, "2026-11-15T09:02:00", 0, 42),
        ];
        let rows = flatten(&events);
        assert_eq!(rows.len(), 5, "two day headers and three events");
        match &rows[0] {
            Row::Day {
                listens,
                skips,
                seconds,
                ..
            } => {
                assert_eq!(*listens, 1);
                assert_eq!(*skips, 1);
                assert_eq!(*seconds, 214);
            }
            other => panic!("a day header came first: {other:?}"),
        }
        assert_eq!(rows[1], Row::Event(0));
        assert_eq!(
            rows[3],
            Row::Day {
                label: day_label((2026, 11, 15), today()),
                key: "2026-11-15".to_owned(),
                listens: 0,
                skips: 0,
                seconds: 42,
            }
        );
    }

    /// The day labels are the SPA's: today, yesterday, then the weekday.
    #[test]
    fn a_day_reads_as_the_page_writes_it() {
        let today = (2026, 11, 16);
        assert_eq!(day_label(today, today), "Aujourd'hui");
        assert_eq!(day_label((2026, 11, 15), today), "Hier");
        assert_eq!(day_label((2026, 11, 13), today), "Vendredi 13 novembre");
        assert_eq!(day_label((2025, 11, 13), today), "Jeudi 13 novembre 2025");
        assert_eq!(at_time(Some("2026-11-13T20:31:00")), "20:31");
        assert_eq!(at_time(None), "");
        assert_eq!(parse_at("2026-11-13 20:31:00"), Some((2026, 11, 13)));
        assert_eq!(parse_at("pas une date"), None);
    }

    /// The calendar arithmetic is the shim every label rests on: the epoch, a
    /// leap day and a 400-year boundary.
    #[test]
    fn the_civil_calendar_round_trips() {
        for (year, month, day) in [
            (1970, 1, 1),
            (2000, 2, 29),
            (2026, 11, 16),
            (1600, 3, 1),
            (2100, 12, 31),
        ] {
            let days = days_from_civil(year, month, day);
            assert_eq!(civil_from_days(days), (year, month, day));
        }
        assert_eq!(days_from_civil(1970, 1, 1), 0);
    }

    #[test]
    fn a_verdict_is_a_word_never_a_colour() {
        assert_eq!(verdict(1), "listen");
        assert_eq!(verdict(-1), "skip");
        assert_eq!(verdict(0), "neutral");
    }

    #[test]
    fn an_empty_history_is_not_a_card_full_of_zeros() {
        let summary = HistorySummary {
            days: 30,
            ..HistorySummary::default()
        };
        assert!(empty_history(&[], &summary));
        let busy = HistorySummary {
            days: 30,
            listens: 3,
            ..HistorySummary::default()
        };
        assert!(!empty_history(&[], &busy));
        assert!(!empty_history(
            &[event(1, "2026-11-16T20:31:00", 1, 20)],
            &summary
        ));
    }

    /// The worst realistic log this screen can hold: 5 000 events, one per
    /// line plus its day headers. The footprint is computed from the OBJECT's
    /// own allocations (`size_of` + the strings' capacity), never from a test
    /// process's RSS: two tests measuring one process's RSS cannot both be
    /// right (D8a's `in_child` recorded exactly that).
    #[test]
    fn five_thousand_events_stay_a_windowed_model_not_five_thousand_widgets() {
        let events: Vec<HistoryEvent> = (0..5_000)
            .map(|index| {
                let day = 1 + index / 50;
                event(
                    i64::from(index),
                    &format!("2026-11-{:02}T20:31:00", day.min(30)),
                    1,
                    200,
                )
            })
            .collect();
        let rows = flatten(&events);
        let headers = rows
            .iter()
            .filter(|row| matches!(row, Row::Day { .. }))
            .count();
        assert_eq!(rows.len(), 5_000 + headers, "each day carries its header");
        assert_eq!(headers, 30, "the log spans the 30 days the fixture builds");
        let mut model = 0usize;
        for row in &rows {
            model += std::mem::size_of::<Row>();
            if let Row::Day { key, label, .. } = row {
                model += key.capacity() + label.capacity();
            }
        }
        let sizes = rows.len() * std::mem::size_of::<Size<gpui_kit::Pixels>>();
        println!(
            "history_5000: {} rows, model {} KB + size vec {} KB = {} KB",
            rows.len(),
            model / 1024,
            sizes / 1024,
            (model + sizes) / 1024
        );
        // The SPA's rows are 44 px and 72 px of geometry, not data: one row is
        // a district and an index, and the sizes vector is two pixels each.
        assert!(
            model + sizes < 400 * 1024,
            "5 000 events hold {} KB of model",
            (model + sizes) / 1024
        );
    }

    /// The filter is the server's own `outcome` vocabulary (`lib/api.js`).
    #[test]
    fn the_filters_are_the_servers_words() {
        assert_eq!(
            FILTERS,
            [("", "Tout"), ("listen", "Écoutes"), ("skip", "Sauts")]
        );
    }
}
