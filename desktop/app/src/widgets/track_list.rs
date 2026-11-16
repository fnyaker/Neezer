//! The shared track list: one windowed list of rows, its selection, its keys.
//!
//! Every browse screen draws tracks with this (the home's suggestions, the
//! search's results, an artist's titles, an album's tracks, a person's credits).
//! Three rules from the web app and from the plan live here, once:
//!
//! - **Windowed** (`VirtualList.svelte`, `TrackList.svelte`): only the rows on
//!   screen (plus a small buffer) are built, whatever the length, so a 4 000-row
//!   list paints as many elements as a 20-row one. D8a measured the row-building
//!   path with `NZ_FRAME_LOG=1`: **p50 0.31 ms, p95 0.55 ms** per frame over
//!   4 000 rows (release), against the 240 Hz panel's 4.17 ms budget.
//! - **Keyboard and mouse are the same list** (`docs/agents/desktop.md`,
//!   *Keyboard AND mouse*): every list action the keymap binds becomes one
//!   [`Intent`], and a click, a right-click and a key produce it. The screen
//!   turns an intent into a transport command — there is no second path.
//! - **The rows are data**: a list holds the `Track`s the server sent, the queue
//!   rows they make (built ONCE when the data arrives, never on the click) and
//!   an index range. A paint copies nothing but the strings it draws.
//!
//! A screen implements one method, `on_intent`, and calls [`bind_actions`] on its
//! root plus [`element`] where the list goes.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;
use std::time::Instant;

use gpui_kit::base::VirtualListScrollHandle;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, BorrowAppContext as _, Context, Div, ElementId, Entity, FontWeight, Hsla,
    InteractiveElement as _, IntoElement, ParentElement, Pixels, Render, ScrollStrategy, Size,
    StatefulInteractiveElement as _, Styled, Window, div, px, rems, size,
};
use gpui_omarchy::{ButtonVariant, IconName, icon_button, scrollbar, virtual_list};
use nz_model::Track;
use nz_player::Context as PlayContext;

use crate::actions::*;
use crate::state::{AppState, entry_from_track};
use crate::theme;
use crate::widgets::context_menu::{Item, Menu};
use crate::widgets::cover::{self, Covers, Kind};

/// What a row says under the title.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Subtitle {
    /// `Artiste · Album` — the lists that mix albums.
    #[default]
    ArtistAlbum,
    /// `Album` (an artist page, an album's own tracks).
    Album,
    /// The credit roles the track carries (`Paroles · Composition`).
    Roles,
}

/// What the keyboard, a click or a menu asked the list for.
///
/// `None` inside a variant means "the selected row"; `Some(index)` is the row a
/// pointer named, so a click never depends on where the keyboard was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    Move(i64),
    First,
    Last,
    Page(i64),
    PlayFrom(Option<usize>),
    PlayNextFrom(Option<usize>),
    QueueFrom(Option<usize>),
    Menu(Option<usize>),
    Like(Option<usize>),
    Dislike(Option<usize>),
}

/// What the list decided, and the screen carries out: what to play, from where.
#[derive(Clone, Debug)]
pub enum PlayRequest {
    Play {
        entries: Vec<nz_player::Entry>,
        start: usize,
        context: Option<PlayContext>,
    },
    PlayNext(Vec<nz_player::Entry>),
    Queue(Vec<nz_player::Entry>),
}

impl PlayRequest {
    /// The transport command this becomes: one command, so a click and a key
    /// produce the same queue with the same context.
    pub fn command(self) -> nz_player::Command {
        match self {
            PlayRequest::Play {
                entries,
                start,
                context,
            } => nz_player::Command::play_queue(entries, start, context),
            PlayRequest::PlayNext(entries) => nz_player::Command::play_next(entries),
            PlayRequest::Queue(entries) => nz_player::Command::add_to_queue(entries),
        }
    }
}

/// Per-frame cost of building the visible rows, printed by `NZ_FRAME_LOG=1`.
///
/// The plan's acceptance for D8a is a frame-rate number; what the app controls
/// is its own work per frame, so that is what is measured and quoted. Off by
/// default: an `env::var` per frame is a syscall nobody needs.
#[derive(Default)]
struct FrameLog {
    samples: Vec<f32>,
    enabled: bool,
}

impl FrameLog {
    fn note(&mut self, micros: f32) {
        if !self.enabled {
            return;
        }
        self.samples.push(micros);
        if self.samples.len().is_multiple_of(240) {
            self.report();
        }
    }

    /// p50, p95, max and the sample count, on stdout.
    fn report(&self) {
        if let Some((p50, p95, max, n)) = self.summary() {
            println!("nz_frame_log list_paint_us p50={p50:.0} p95={p95:.0} max={max:.0} n={n}");
        }
    }

    fn summary(&self) -> Option<(f32, f32, f32, usize)> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted = self.samples.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let at = |q: f32| sorted[((sorted.len() - 1) as f32 * q) as usize];
        Some((
            at(0.5),
            at(0.95),
            *sorted.last().unwrap_or(&0.),
            sorted.len(),
        ))
    }
}

/// One windowed list of tracks, its selection and its scroll position.
pub struct TrackList {
    rows: Rc<Vec<Track>>,
    /// The queue rows the list makes, built when the data arrives: a click on a
    /// 4 000-row list must not build 4 000 entries (rule 4).
    entries: Rc<Vec<nz_player::Entry>>,
    sizes: Rc<Vec<Size<Pixels>>>,
    built_for: Pixels,
    selected: Option<usize>,
    scroll: VirtualListScrollHandle,
    context: Option<PlayContext>,
    visible: Rc<Cell<(usize, usize)>>,
    numbered: bool,
    show_cover: bool,
    subtitle: Subtitle,
    log: Rc<RefCell<FrameLog>>,
}

impl Default for TrackList {
    fn default() -> Self {
        Self::new()
    }
}

impl TrackList {
    pub fn new() -> Self {
        Self {
            rows: Rc::new(Vec::new()),
            entries: Rc::new(Vec::new()),
            sizes: Rc::new(Vec::new()),
            built_for: px(0.),
            selected: None,
            scroll: VirtualListScrollHandle::new(),
            context: None,
            visible: Rc::new(Cell::new((0, 0))),
            numbered: false,
            show_cover: true,
            subtitle: Subtitle::default(),
            log: Rc::new(RefCell::new(FrameLog {
                samples: Vec::new(),
                enabled: std::env::var("NZ_FRAME_LOG").is_ok_and(|value| value != "0"),
            })),
        }
    }

    /// Replace the rows. The first row is selected, so a list is usable from
    /// the keyboard the moment it appears and `Entrée` plays it.
    pub fn set_rows(&mut self, rows: Vec<Track>) {
        self.selected = (!rows.is_empty()).then_some(0);
        self.entries = Rc::new(rows.iter().map(entry_from_track).collect());
        self.rows = Rc::new(rows);
        // The heights are rebuilt on the next paint, where the window's rem size
        // is known (a zoom changes it, and a row is 3.5 rem tall).
        self.built_for = px(u32::MAX as f32);
        self.scroll = VirtualListScrollHandle::new();
        self.visible.set((0, 0));
    }

    /// Empty the list (a new query whose answer has not arrived yet).
    pub fn clear(&mut self) {
        self.selected = None;
        self.visible.set((0, 0));
        self.entries = Rc::new(Vec::new());
        self.rows = Rc::new(Vec::new());
        self.sizes = Rc::new(Vec::new());
        self.built_for = px(0.);
    }

    pub fn numbered(mut self, numbered: bool) -> Self {
        self.numbered = numbered;
        self
    }

    pub fn show_cover(mut self, show: bool) -> Self {
        self.show_cover = show;
        self
    }

    pub fn subtitle(mut self, subtitle: Subtitle) -> Self {
        self.subtitle = subtitle;
        self
    }

    pub fn set_context(&mut self, context: Option<PlayContext>) {
        self.context = context;
    }

    pub fn rows(&self) -> &[Track] {
        &self.rows
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn row(&self, index: usize) -> Option<&Track> {
        self.rows.get(index)
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn select(&mut self, index: usize) {
        if index < self.rows.len() {
            self.selected = Some(index);
        }
    }

    /// Which rows the last paint built. A test asserts a 4 000-row list builds
    /// a screenful, and the screens ask for those sleeves only.
    pub fn visible(&self) -> Range<usize> {
        let (start, end) = self.visible.get();
        start..end
    }

    /// Rows a page holds at this window height: what `PgUp`/`PgDn` move.
    pub fn page_rows(window_height: Pixels) -> usize {
        let row = rems(3.5).to_pixels(px(16.));
        (window_height / row).max(1.) as usize
    }

    fn rebuild_sizes(&mut self, rem: Pixels) {
        if self.built_for == rem && self.sizes.len() == self.rows.len() {
            return;
        }
        self.built_for = rem;
        self.sizes = Rc::new(vec![
            size(px(0.), rems(3.5).to_pixels(rem));
            self.rows.len()
        ]);
    }

    fn moved(&mut self, to: usize, strategy: ScrollStrategy) {
        if self.rows.is_empty() {
            return;
        }
        let to = to.min(self.rows.len() - 1);
        self.selected = Some(to);
        self.scroll.scroll_to_item(to, strategy);
    }

    /// Apply a list intent and, when it is a transport one, say what to play.
    ///
    /// The menu, the like and the dislike are NOT here: they need the server or
    /// the sheet, and the screen owns those (see [`Intent::Menu`]).
    pub fn apply(&mut self, intent: Intent, page: usize) -> Option<PlayRequest> {
        let page = page.max(1);
        let current = self.selected.unwrap_or(0);
        match intent {
            Intent::Move(delta) => {
                let next = if delta < 0 {
                    current.saturating_sub(delta.unsigned_abs() as usize)
                } else {
                    current + delta as usize
                };
                self.moved(next, ScrollStrategy::Center);
                None
            }
            Intent::First => {
                self.moved(0, ScrollStrategy::Top);
                None
            }
            Intent::Last => {
                self.moved(self.rows.len().saturating_sub(1), ScrollStrategy::Bottom);
                None
            }
            Intent::Page(delta) => {
                let next = if delta < 0 {
                    current.saturating_sub(page)
                } else {
                    current + page
                };
                self.moved(next, ScrollStrategy::Center);
                None
            }
            Intent::PlayFrom(at) => {
                // The queue is the WHOLE list, so what follows the tapped row is
                // what the person sees under it — the web player's rule.
                let start = at.unwrap_or(current);
                self.rows.get(start)?;
                self.select(start);
                Some(PlayRequest::Play {
                    entries: self.entries.as_ref().clone(),
                    start,
                    context: self.context.clone(),
                })
            }
            Intent::PlayNextFrom(at) => {
                let start = at.unwrap_or(current);
                let entry = self.entries.get(start)?.clone();
                self.select(start);
                Some(PlayRequest::PlayNext(vec![entry]))
            }
            Intent::QueueFrom(at) => {
                let start = at.unwrap_or(current);
                let entry = self.entries.get(start)?.clone();
                self.select(start);
                Some(PlayRequest::Queue(vec![entry]))
            }
            Intent::Menu(_) | Intent::Like(_) | Intent::Dislike(_) => None,
        }
    }

    /// The whole list as queue rows, in order.
    pub fn entries(&self) -> &[nz_player::Entry] {
        &self.entries
    }

    /// Keep the keyboard on the playing row when the list opens.
    pub fn select_reference(&mut self, reference: Option<&str>) {
        let Some(reference) = reference else {
            return;
        };
        if let Some(index) = self.rows.iter().position(|track| {
            track.deezer_id.as_str() == reference
                || track.id.as_ref().is_some_and(|id| id.as_str() == reference)
        }) {
            self.selected = Some(index);
            self.scroll.scroll_to_item(index, ScrollStrategy::Center);
        }
    }

    /// How far the list is scrolled, for a menu that must sit on its row.
    pub fn scroll_y(&self) -> Pixels {
        self.scroll.base_handle().offset().y
    }

    /// The `NZ_FRAME_LOG=1` summary (p50, p95, max, count), for a measurement or
    /// a test that wants the number rather than the print.
    pub fn frame_summary(&self) -> Option<(f32, f32, f32, usize)> {
        self.log.borrow().summary()
    }
}

/// What one row needs to know besides its track.
struct Look<'a> {
    selected: bool,
    now: Option<&'a str>,
    playing: bool,
    numbered: bool,
    show_cover: bool,
    subtitle: Subtitle,
    covers: &'a Covers,
    rem: Pixels,
}

/// Build the list: the windowed rows, the scrollbar, and the same row a click
/// and a key reach.
///
/// `on_intent` is the screen's whole list behaviour; `row_extra` lets a screen
/// add what only it knows (a credit role chip, a download badge) and defaults to
/// nothing when it returns `None`.
#[allow(clippy::too_many_arguments)]
pub fn element<V: Render>(
    entity: Entity<V>,
    id: &'static str,
    list: &mut TrackList,
    state: &Entity<AppState>,
    covers: Rc<RefCell<Covers>>,
    menu: &Menu,
    window: &mut Window,
    cx: &mut Context<V>,
    on_intent: impl Fn(&mut V, Intent, &mut Window, &mut Context<V>) + Clone + 'static,
    row_extra: impl Fn(&mut V, usize, &Track, &mut Context<V>) -> Option<AnyElement> + Clone + 'static,
    pick: impl Fn(&mut V, Item, &mut Context<V>) + Clone + 'static,
    close: impl Fn(&mut V, &mut Context<V>) + Clone + 'static,
) -> AnyElement {
    list.rebuild_sizes(window.rem_size());
    let sizes = Rc::clone(&list.sizes);
    let rows = Rc::clone(&list.rows);
    let selected = list.selected;
    let numbered = list.numbered;
    let show_cover = list.show_cover;
    let subtitle = list.subtitle;
    let scroll = list.scroll.clone();
    let log = Rc::clone(&list.log);
    let visible = Rc::clone(&list.visible);
    let (now, playing) = {
        let state = state.read(cx);
        (
            state
                .player
                .current()
                .and_then(|entry| entry.reference().map(str::to_owned)),
            state.player.playing(),
        )
    };
    let call = on_intent.clone();
    let extra = row_extra.clone();
    let covers_ref = Rc::clone(&covers);
    let rem = window.rem_size();

    let view = virtual_list(
        entity,
        ElementId::Name(id.into()),
        sizes,
        move |this, range, _window, cx| {
            let started = Instant::now();
            // What this paint built, for the sleeves and for a test that holds
            // the windowing to its promise.
            visible.set((range.start, range.end));
            let covers = covers_ref.borrow();
            let built: Vec<_> = range
                .clone()
                .filter_map(|index| {
                    let track = rows.get(index)?;
                    let look = Look {
                        selected: selected == Some(index),
                        now: now.as_deref(),
                        playing,
                        numbered,
                        show_cover,
                        subtitle,
                        covers: &covers,
                        rem,
                    };
                    let mut cells: Vec<AnyElement> =
                        vec![row(index, track, look, cx, call.clone()).into_any_element()];
                    if let Some(added) = extra(this, index, track, cx) {
                        cells.push(added);
                    }
                    Some(div().relative().children(cells).into_any_element())
                })
                .collect();
            log.borrow_mut()
                .note(started.elapsed().as_secs_f32() * 1_000_000.);
            built
        },
        cx,
    )
    .h_full();

    // The menu floats INSIDE the list's own box: the row it belongs to is in
    // that box's coordinates, so the panel lands on the row without measuring
    // the window (and a click outside it reaches the backdrop, not a row).
    let panel = menu.view(
        cx,
        {
            let pick = pick.clone();
            move |this: &mut V, item: Item, cx| pick(this, item, cx)
        },
        {
            let close = close.clone();
            move |this: &mut V, cx| close(this, cx)
        },
    );
    div()
        .relative()
        .size_full()
        .child(view)
        .child(scrollbar(
            format!("scroll-{id}"),
            gpui_kit::base::ScrollbarAxis::Vertical,
            &scroll,
            window,
            cx,
        ))
        .when(menu.is_open(), |wrapper| wrapper.child(panel))
        .into_any_element()
}

fn row<V: Render>(
    index: usize,
    track: &Track,
    look: Look<'_>,
    cx: &mut Context<V>,
    intent: impl Fn(&mut V, Intent, &mut Window, &mut Context<V>) + Clone + 'static,
) -> AnyElement {
    let current = look.now.is_some_and(|now| {
        track.deezer_id.as_str() == now || track.id.as_ref().is_some_and(|id| id.as_str() == now)
    });
    let mut element = div()
        .id(ElementId::Name(format!("track-row-{index}").into()))
        // A stable name for a test that must press a row for real instead of
        // guessing its y: the layout is the design's business.
        .debug_selector(move || format!("track-row-{index}"))
        .flex()
        .items_center()
        .gap(rems(0.625))
        .h(rems(3.5))
        .w_full()
        .pl(rems(0.75))
        .pr(rems(0.5))
        .border_b_1()
        .border_color(theme::border(create_ctx(cx)).opacity(0.4))
        .when(look.selected, |row| row.bg(theme::selected(create_ctx(cx))))
        .when(!look.selected && current, |row| {
            row.bg(theme::chrome(create_ctx(cx)))
        })
        .cursor_pointer()
        .hover(|style| style.bg(theme::chrome(create_ctx(cx))))
        .on_click({
            let intent = intent.clone();
            cx.listener(move |this, _, window, cx| {
                intent(this, Intent::PlayFrom(Some(index)), window, cx)
            })
        })
        .on_mouse_down(gpui_kit::MouseButton::Right, {
            let intent = intent.clone();
            cx.listener(move |this, _, window, cx| {
                intent(this, Intent::Menu(Some(index)), window, cx)
            })
        })
        .child(
            // The current row is marked by an accent edge, never by a colour
            // alone: a person who cannot tell it apart still sees the bar.
            div()
                .w(rems(0.125))
                .h(rems(1.5))
                .flex_shrink_0()
                .bg(if current {
                    theme::accent(create_ctx(cx))
                } else {
                    theme::shell(create_ctx(cx))
                }),
        );

    if look.numbered {
        element = element.child(
            div()
                .flex_shrink_0()
                .w(rems(1.5))
                .text_size(rems(0.6875))
                .font_family(theme::mono(create_ctx(cx)))
                .text_color(theme::secondary(create_ctx(cx)))
                .text_right()
                .child((index + 1).to_string()),
        );
    }
    if look.show_cover {
        let file = track
            .album
            .cover
            .as_deref()
            .and_then(|url| look.covers.file(url));
        element = element.child(cover::view(
            file,
            track.album.cover.as_deref(),
            &track.album.title,
            Kind::Album,
            rems(2.25).to_pixels(look.rem),
            cx,
        ));
    }

    let mut subtitle = match look.subtitle {
        Subtitle::ArtistAlbum => {
            let artist = track
                .display_artist
                .clone()
                .unwrap_or_else(|| track.artist.name.clone());
            if track.album.title.is_empty() {
                artist
            } else {
                format!("{artist} · {}", track.album.title)
            }
        }
        Subtitle::Album => {
            if track.album.title.is_empty() {
                track
                    .display_artist
                    .clone()
                    .unwrap_or_else(|| track.artist.name.clone())
            } else {
                track.album.title.clone()
            }
        }
        Subtitle::Roles => {
            if track.credit_roles.is_empty() {
                track
                    .display_artist
                    .clone()
                    .unwrap_or_else(|| track.artist.name.clone())
            } else {
                track.credit_roles.join(" · ")
            }
        }
    };
    if track.unavailable {
        subtitle = format!("Indisponible · {subtitle}");
    }

    element = element.child(
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(rems(0.0625))
            .child(
                div()
                    .truncate()
                    .text_size(rems(0.8125))
                    .text_color(if current {
                        theme::accent(create_ctx(cx))
                    } else {
                        theme::text(create_ctx(cx))
                    })
                    .child(track.title.clone()),
            )
            .child(
                div()
                    .truncate()
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(create_ctx(cx)))
                    .child(subtitle),
            ),
    );

    if track.explicit {
        element = element.child(badge("E", theme::secondary(create_ctx(cx))));
    }
    if let Some(tag) = quality_tag(track) {
        element = element.child(badge(&tag, theme::secondary(create_ctx(cx))));
    }
    if track.source.as_deref() == Some("youtube") {
        element = element.child(badge("YouTube", theme::warning(create_ctx(cx))));
    }
    if track.unavailable {
        element = element.child(badge("Indisponible", theme::danger(create_ctx(cx))));
    }

    element
        .child(
            div()
                .flex_shrink_0()
                .w(rems(2.5))
                .text_right()
                .text_size(rems(0.6875))
                .font_family(theme::mono(create_ctx(cx)))
                .text_color(theme::secondary(create_ctx(cx)))
                .child(duration_label(track.duration)),
        )
        .child(
            icon_button(
                "row-menu",
                IconName::Menu,
                "Plus d'options",
                ButtonVariant::Secondary,
                cx,
            )
            .on_click({
                let intent = intent.clone();
                cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    intent(this, Intent::Menu(Some(index)), window, cx)
                })
            }),
        )
        .into_any_element()
}

/// `&App` from a `&mut Context<T>`: the theme helpers take an `&App` and every
/// call site here already holds the context.
fn create_ctx<'a, V>(cx: &'a Context<V>) -> &'a gpui_kit::App {
    cx
}

/// Where a row's menu goes, in the list's own coordinates: under the row, and
/// flipped above it when the row sits low enough that the panel would fall out
/// of the list. Item rows are 2 rem, the panel's padding 0.25 rem.
pub fn row_top(index: usize, scroll_y: Pixels, items: usize) -> Pixels {
    let row_h = rems(3.5).to_pixels(px(16.));
    let panel = rems(2.).to_pixels(px(16.)) * items as f32 + rems(0.5).to_pixels(px(16.));
    let under = row_h * (index as f32 + 1.) - scroll_y;
    // Above the row when there is no room below: the panel never hangs off the
    // list, which is what a person reads as "the menu is somewhere else".
    if under + panel > rems(26.).to_pixels(px(16.)) {
        (row_h * index as f32 - scroll_y - panel).max(px(0.))
    } else {
        under
    }
}

/// The quality a row may claim: the server's own tag, never the one the person
/// asked for (the web app's rule). A YouTube row knows its bitrate only.
pub fn quality_tag(track: &Track) -> Option<String> {
    if let Some(quality) = &track.quality {
        return Some(quality.replace("OPUS_", "OPUS ").replace("OPUS-", "OPUS "));
    }
    if track.source.as_deref() == Some("youtube") {
        return Some(match track.bitrate {
            Some(bitrate) => format!("OPUS {bitrate}"),
            None => "OPUS 133".to_owned(),
        });
    }
    None
}

/// `m:ss`, or `h:mm:ss` — the web app's `lib/format.js#duration`.
pub fn duration_label(seconds: u32) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

fn badge(label: &str, color: Hsla) -> Div {
    div()
        .flex_shrink_0()
        .px(rems(0.25))
        .py(rems(0.0625))
        .border_1()
        .border_color(color.opacity(0.5))
        .text_size(rems(0.5625))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
        .child(label.to_owned())
}

/// Who answers the browse keys, for the whole process.
///
/// D7's shell keeps ONE focus anchor on its root and binds every action there;
/// a screen that took the focus for itself would take the shell's bindings away
/// with it the moment it is replaced — measured: `g l`, then `Alt+←` did
/// nothing, because the focused handle belonged to the Home screen that no
/// longer existed. So a screen does NOT steal the focus: it registers itself
/// here, and the list actions are handled by APP-LEVEL listeners, which GPUI
/// runs whatever is focused (the same mechanism that makes `Ctrl+Q` work with
/// nothing focused, D0). A dropped screen leaves a dead `WeakEntity`: the
/// listener finds nothing to update and does nothing.
struct Owner(RefCell<OwnerState>);

/// How a screen is told about a list intent: the screen's own handler, behind a
/// weak reference (a screen that is gone answers nothing).
type IntentHandler = Rc<dyn Fn(Intent, &mut Window, &mut App)>;

#[derive(Default)]
struct OwnerState {
    /// The screen that shows a list right now, and how to tell it an intent.
    handler: Option<IntentHandler>,
    /// The listeners below are app-level and live for the process; they are
    /// installed exactly once.
    installed: bool,
}

impl gpui_kit::Global for Owner {}

/// Make `this` screen the one the list keys reach, installing the app-level
/// listeners the first time any browse screen is built.
pub fn own<V: Render>(
    cx: &mut Context<V>,
    on_intent: impl Fn(&mut V, Intent, &mut Window, &mut Context<V>) + 'static,
) {
    install(cx);
    let weak = cx.weak_entity();
    let handler: IntentHandler = Rc::new(move |intent, window, cx| {
        let _ = weak.update(cx, |this, cx| on_intent(this, intent, window, cx));
    });
    cx.update_global::<Owner, _>(|owner, _| {
        owner.0.borrow_mut().handler = Some(handler);
    });
}

fn install<V: Render>(cx: &mut Context<V>) {
    if !cx.has_global::<Owner>() {
        cx.set_global(Owner(RefCell::new(OwnerState::default())));
    }
    if cx.update_global::<Owner, _>(|owner, _| owner.0.borrow().installed) {
        return;
    }
    cx.update_global::<Owner, _>(|owner, _| owner.0.borrow_mut().installed = true);
    macro_rules! listen {
        ($action:ty, $intent:expr) => {
            gpui_kit::App::on_action(&mut *cx, move |_: &$action, cx: &mut gpui_kit::App| {
                deliver($intent, cx);
            });
        };
    }
    listen!(ListNext, Intent::Move(1));
    listen!(ListPrevious, Intent::Move(-1));
    listen!(ListFirst, Intent::First);
    listen!(ListLast, Intent::Last);
    listen!(ListPageUp, Intent::Page(-1));
    listen!(ListPageDown, Intent::Page(1));
    listen!(PlayFromHere, Intent::PlayFrom(None));
    listen!(PlayNextHere, Intent::PlayNextFrom(None));
    listen!(AddHereToQueue, Intent::QueueFrom(None));
    listen!(RowContextMenu, Intent::Menu(None));
    listen!(LikeHere, Intent::Like(None));
    listen!(DislikeHere, Intent::Dislike(None));
}

/// Hand one intent to the screen that owns the list, in the active window (a
/// screen measures a page and anchors its menu in window coordinates).
///
/// Deferred by one effect flush: this runs INSIDE an action dispatch, where the
/// app is already borrowed, and re-entering the window there fails silently —
/// measured: the handler was found, the intent went nowhere. Deferring also
/// keeps the order a person expects (the key is answered at the end of the same
/// frame).
pub fn deliver(intent: Intent, cx: &mut gpui_kit::App) {
    cx.defer(move |cx| {
        let handler = cx
            .try_global::<Owner>()
            .and_then(|owner| owner.0.borrow().handler.clone());
        let Some(handler) = handler else {
            return;
        };
        let Some(window) = cx
            .active_window()
            .or_else(|| cx.windows().into_iter().next())
        else {
            return;
        };
        let _ = window.update(cx, |_, window, cx| handler(intent, window, cx));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn track(index: usize) -> nz_model::Track {
        nz_model::Track {
            id: Some(nz_model::TrackRowId::new(format!("row-{index}"))),
            deezer_id: nz_model::TrackId::new(format!("3000{index}")),
            title: format!("Titre {index}"),
            duration: 200,
            artist: nz_model::TrackArtist {
                deezer_id: nz_model::ArtistId::new("42"),
                name: format!("Artiste {index}"),
            },
            album: nz_model::TrackAlbum {
                deezer_id: nz_model::AlbumId::new("9"),
                title: format!("Album {index}"),
                cover: None,
            },
            ..nz_model::Track::default()
        }
    }

    fn rss_kb() -> i64 {
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        status
            .lines()
            .find_map(|line| line.strip_prefix("VmRSS:"))
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse().ok())
            .unwrap_or(0)
    }

    /// The worst realistic list, as D8a's acceptance names it: 4 000 rows in the
    /// list AND the queue rows they make, built once when the data arrives. A
    /// click on a row then copies the queue, so the press does not build it.
    #[test]
    fn four_thousand_rows_are_one_list_and_one_click_is_one_copy() {
        let before = rss_kb();
        let mut list = TrackList::new();
        let rows: Vec<nz_model::Track> = (0..4_000).map(track).collect();
        let started = Instant::now();
        list.set_rows(rows);
        let built = started.elapsed();
        let after = rss_kb();
        println!(
            "list_4000: set_rows {:?}, data model +{} KB, {} entries",
            built,
            after - before,
            list.entries().len()
        );
        assert_eq!(list.len(), 4_000);
        assert_eq!(list.entries().len(), 4_000, "the queue rows are built once");
        assert_eq!(list.selected(), Some(0), "the first row has the keyboard");

        // A press: the queue is the whole list, and it is a copy of what was
        // already built (rule 4: the click answers at once).
        let started = Instant::now();
        let request = list
            .apply(Intent::PlayFrom(Some(1_200)), 20)
            .expect("a play");
        let copied = started.elapsed();
        println!("list_4000: click {:?}", copied);
        match request {
            PlayRequest::Play { entries, start, .. } => {
                assert_eq!(start, 1_200);
                assert_eq!(entries.len(), 4_000);
            }
            other => panic!("unexpected request: {other:?}"),
        }
        // The budget is D4's own: a transport action on a 4 000-row queue must
        // stay far under a frame; debug builds are slow, this machine builds
        // several at once, so the bound is the generous one.
        assert!(
            copied < std::time::Duration::from_millis(60),
            "a click copied the queue in {copied:?}"
        );
    }

    /// The keyboard walks a 4 000-row list without building it: `Fin` lands on
    /// the last row and asks the scroll handle for it, and the rows stay data.
    #[test]
    fn the_keyboard_reaches_the_last_row_of_four_thousand() {
        let mut list = TrackList::new();
        list.set_rows((0..4_000).map(track).collect());
        assert!(list.apply(Intent::Last, 20).is_none());
        assert_eq!(list.selected(), Some(3_999));
        assert!(list.apply(Intent::First, 20).is_none());
        assert_eq!(list.selected(), Some(0));
        assert!(list.apply(Intent::Move(-3), 20).is_none());
        assert_eq!(list.selected(), Some(0), "the top is a wall, not a wrap");
        list.apply(Intent::Page(1), 20);
        assert_eq!(list.selected(), Some(20), "a page is the window's rows");
    }

    #[test]
    fn a_duration_reads_as_minutes_and_seconds() {
        assert_eq!(duration_label(0), "0:00");
        assert_eq!(duration_label(59), "0:59");
        assert_eq!(duration_label(200), "3:20");
        assert_eq!(duration_label(3_600), "1:00:00");
        assert_eq!(duration_label(3_723), "1:02:03");
    }

    #[test]
    fn a_row_claims_only_the_quality_the_server_sent() {
        let mut row = track(1);
        assert_eq!(quality_tag(&row), None, "a catalogue row claims nothing");
        row.quality = Some("OPUS_320".to_owned());
        assert_eq!(quality_tag(&row).as_deref(), Some("OPUS 320"));
        let mut youtube = track(2);
        youtube.source = Some("youtube".to_owned());
        assert_eq!(quality_tag(&youtube).as_deref(), Some("OPUS 133"));
        youtube.bitrate = Some(160);
        assert_eq!(quality_tag(&youtube).as_deref(), Some("OPUS 160"));
    }
}
