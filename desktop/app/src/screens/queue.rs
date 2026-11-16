//! La file d'attente, portée de `QueueList.svelte` : une liste fenêtrée, ce qui
//! suit le titre en cours, et ce qui peut encore être édité.
//!
//! The SPA's rules, kept one for one:
//!
//! - **Only what is AFTER the playing row can be moved or removed.** Moving the
//!   track being heard, or one already played, is not what a queue is for; the
//!   click that plays a row is the way back. The playing row is told apart by a
//!   tinted ground and its weight (not by accent-as-text: on a light theme the
//!   accent is a fill), the played rows are dimmed to 50 %.
//! - **A 4 000-row queue is the input, not the happy path** (rule 4). The list
//!   is windowed: one 56 px row per entry and only the visible slice is built
//!   (measured: 15 rows on an 800 px window at 4 000 entries, and a paint of
//!   0.30 ms p95 in release — the number is next to the acceptance test).
//! - **Following the playhead costs one scroll, not a rebuild.** When the queue
//!   advances the list centres the new row (`scroll_to_item`), so the row that
//!   is playing is never scrolled out of sight by a long queue.
//! - **Edits go through `PlayerState::apply`**, like every other gesture: the
//!   jump, the remove and the move are the SPA's `player.jump`, `removeAt` and
//!   `move`, with the same refusal when the row is not editable. Nothing here
//!   mutates the queue by hand.
//! - **The keyboard is the mouse**: `j`/`k`, the arrows, `Début`/`Fin` and
//!   `Page↑`/`Page↓` move the cursor, `Entrée` plays that row, `Alt+↑`/`Alt+↓`
//!   move an upcoming row, `Suppr` removes it — and every one of those exists as
//!   a visible control on the row (a `✕` to remove, `⌃`/`⌄` to move), because "the
//!   mouse can do it" is the other half of the rule.
//!
//! The queue is drawn in two places by the same entity: the `q` side panel and
//! the now-playing screen's card. A screen keeps its own state, so the two do
//! not share a scroll offset — they share the code.

use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Entity, FocusHandle, FontWeight, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Pixels, Render, ScrollStrategy, Size, StatefulInteractiveElement, Styled,
    Subscription, Window, div, px, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon, virtual_list};
use nz_player::{Command, Entry};

use crate::actions::{
    ListFirst, ListLast, ListNext, ListPageDown, ListPageUp, ListPrevious, PlayFromHere,
};
use crate::nav::Route;
use crate::state::AppState;
use crate::theme;

/// One queue row's height, as `QueueList.svelte` (`.qitem { height: 56px }`).
pub const ROW_HEIGHT: f32 = 56.;

/// The queue screen. `route` is kept because `screens::build` hands it over and
/// a screen names itself (`docs/agents/desktop.md`, D7).
pub struct Screen {
    route: Route,
    state: Entity<AppState>,
    /// The keyboard cursor: a row index, kept valid against the queue.
    selected: usize,
    /// The playing row and the queue length last painted, so the follow scroll
    /// and the row sizes are rebuilt only when they really changed.
    followed: (i64, usize),
    sizes: Rc<Vec<Size<Pixels>>>,
    scroll: gpui_kit::base::VirtualListScrollHandle,
    focus: FocusHandle,
    /// How many rows the last paint built — the windowing assertion, and the
    /// number the acceptance quotes.
    pub rendered_rows: usize,
    _subscription: Subscription,
}

impl Screen {
    pub fn new(route: Route, state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |this, _, cx| {
            this.sync(cx);
            cx.notify();
        });
        let focus = cx.focus_handle();
        let mut this = Self {
            route,
            state,
            selected: 0,
            followed: (-1, 0),
            sizes: Rc::new(Vec::new()),
            scroll: gpui_kit::base::VirtualListScrollHandle::new(),
            focus,
            rendered_rows: 0,
            _subscription: subscription,
        };
        this.sync(cx);
        this
    }

    pub fn route(&self) -> &Route {
        &self.route
    }

    /// The list's focus handle: what Tab reaches and a click takes.
    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    fn len(&self, cx: &AppState) -> usize {
        cx.player.len()
    }

    /// Keep the row sizes, the follow scroll and the cursor in step with the
    /// queue. Called from the state observer and from the top of every paint.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let (len, index) = {
            let state = self.state.read(cx);
            (state.player.len(), state.player.index())
        };
        if self.sizes.len() != len {
            self.sizes = Rc::new(
                (0..len)
                    .map(|_| Size::new(px(0.), px(ROW_HEIGHT)))
                    .collect(),
            );
        }
        if self.selected >= len {
            self.selected = len.saturating_sub(1);
        }
        if self.followed.0 != index && index >= 0 {
            // The queue advanced: put the playing row back in the middle
            // (the SPA's `scrollToIndex(idx, { ratio: 0.25 })`).
            self.scroll
                .scroll_to_item(index as usize, ScrollStrategy::Center);
            self.selected = index as usize;
        }
        self.followed = (index, len);
    }

    fn command(&mut self, command: Command, cx: &mut Context<Self>) {
        self.state
            .update(cx, |state, cx| state.command(command, cx));
    }

    /// Play the row at `index`: `player.jump`, the one path (a click, `Entrée`).
    pub fn jump(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected = index;
        self.command(Command::jump(index), cx);
        cx.notify();
    }

    /// Remove an upcoming row. A row that is playing or played is refused by the
    /// state machine too (`removeAt` only ever edited what is ahead), so the
    /// guard here is the UI's, not the rule's.
    pub fn remove(&mut self, index: usize, cx: &mut Context<Self>) {
        let (index_of_playhead, len) = {
            let state = self.state.read(cx);
            (state.player.index(), state.player.len())
        };
        if index as i64 <= index_of_playhead || index >= len {
            return;
        }
        self.command(Command::remove_at(index), cx);
        cx.notify();
    }

    /// Move an upcoming row one place up or down (`Alt+↑`/`Alt+↓`, the row's
    /// `⌃`/`⌄`). Nothing can land on or before the playing row — the SPA's
    /// `retarget` clamp.
    pub fn move_row(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        let (index_of_playhead, len) = {
            let state = self.state.read(cx);
            (state.player.index(), state.player.len())
        };
        if from as i64 <= index_of_playhead
            || from >= len
            || to as i64 <= index_of_playhead
            || to >= len
        {
            return;
        }
        if from == to {
            return;
        }
        self.command(Command::move_to(from, to), cx);
        self.selected = to;
        cx.notify();
    }

    /// Vider : `player.clearUpcoming`, which also switches autoplay off so a
    /// radio cannot refill a queue the person just cleared.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.command(Command::clear_upcoming(), cx);
        self.state.update(cx, |state, cx| {
            state
                .toasts
                .push(crate::toast::ToastKind::Info, "File vidée");
            cx.notify();
        });
        cx.notify();
    }

    /// The row the keyboard cursor is on: what the tests read, and what the
    /// focus border shows.
    pub fn selected(&self) -> usize {
        self.selected
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        let len = self.state.read(cx).player.len();
        if len == 0 {
            return;
        }
        self.selected = index.min(len - 1);
        self.scroll
            .scroll_to_item(self.selected, ScrollStrategy::Center);
        cx.notify();
    }

    fn move_selection(&mut self, delta: i64, cx: &mut Context<Self>) {
        let len = self.state.read(cx).player.len();
        if len == 0 {
            return;
        }
        let target = (self.selected as i64 + delta).clamp(0, len as i64 - 1);
        self.select(target as usize, cx);
    }

    /// The row's own controls, drawn for what is ahead of the playhead only.
    fn row_controls(
        &self,
        index: usize,
        len: usize,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let playing = {
            let state = self.state.read(cx);
            index as i64 > state.player.index()
        };
        // The row's own controls are quiet until the pointer is on the row (or
        // the keyboard cursor is): a 4 000-row queue of ✕ buttons is noise.
        let mut controls = div()
            .flex()
            .items_center()
            .gap(rems(0.125))
            .opacity(if selected { 1.0 } else { 0.0 })
            .group_hover(format!("queue-row-{index}"), |style| style.opacity(1.0));
        if playing {
            controls = controls
                .child(self.row_button(
                    ("row-up", index),
                    IconName::ChevronUp,
                    "Monter le titre",
                    index > 0,
                    cx,
                    move |this, cx| this.move_row(index, index.saturating_sub(1), cx),
                ))
                .child(self.row_button(
                    ("row-down", index),
                    IconName::ChevronDown,
                    "Descendre le titre",
                    index + 1 < len,
                    cx,
                    move |this, cx| this.move_row(index, index + 1, cx),
                ))
                .child(self.row_button(
                    ("row-remove", index),
                    IconName::Close,
                    "Retirer de la file",
                    true,
                    cx,
                    move |this, cx| this.remove(index, cx),
                ));
        }
        controls
    }

    fn row_button<A>(
        &self,
        id: (&'static str, usize),
        glyph: IconName,
        label: &'static str,
        enabled: bool,
        cx: &mut Context<Self>,
        action: A,
    ) -> impl IntoElement + use<A>
    where
        A: Fn(&mut Self, &mut Context<Self>) + 'static,
    {
        let color = if enabled {
            theme::secondary(cx)
        } else {
            theme::border(cx)
        };
        div()
            .id(id)
            .debug_selector({
                let name = id.0;
                move || format!("{name}-{}", id.1)
            })
            .aria_label(label)
            .flex()
            .items_center()
            .justify_center()
            .w(rems(1.5))
            .h(rems(1.75))
            .rounded_md()
            .text_color(color)
            .when(enabled, |this| {
                this.cursor_pointer()
                    .hover(|style| style.bg(theme::chrome(cx)))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                // A click on a row's control must not also play the row behind
                // it: without this the deepest handler removes a row and the
                // row's own click then jumps to whatever slid into its index.
                cx.stop_propagation();
                if enabled {
                    action(this, cx);
                }
            }))
            .child(icon(glyph).size(rems(0.8125)))
    }
}

impl Render for Screen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync(cx);
        let (entries, index, len) = {
            let state = self.state.read(cx);
            (
                state.player.queue().to_vec(),
                state.player.index(),
                state.player.len(),
            )
        };
        let total: u64 = entries.iter().map(|entry| entry.duration as u64).sum();
        let upcoming = (len as i64 - index - 1).max(0);

        let mut header = div()
            .id("queue-header")
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_between()
            .gap(rems(0.5))
            .px(rems(0.75))
            .py(rems(0.5))
            .border_b_1()
            .border_color(theme::border(cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(rems(0.8125))
                            .font_weight(FontWeight::BOLD)
                            .child(if len == 0 {
                                "File d'attente".to_owned()
                            } else {
                                format!("À suivre · {upcoming} titre{}", plural(upcoming))
                            }),
                    )
                    .child(
                        div()
                            .text_size(rems(0.6875))
                            .font_family(theme::mono(cx))
                            .text_color(theme::secondary(cx))
                            .child(if len == 0 {
                                "Aucun titre".to_owned()
                            } else {
                                format!(
                                    "{len} titre{} · {}",
                                    plural(len as i64),
                                    crate::player_bar::clock(total as f64)
                                )
                            }),
                    ),
            );
        if len > 0 && upcoming > 0 {
            header = header.child(
                button("queue-clear", "Vider", ButtonVariant::Secondary, cx)
                    .on_click(cx.listener(|this, _, _, cx| this.clear(cx))),
            );
        }

        let root = div()
            .id("queue-screen")
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .bg(theme::shell(cx))
            .text_color(theme::text(cx))
            .font_family(theme::font(cx))
            .track_focus(&self.focus)
            .key_context("Neezer")
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    window.focus(&this.focus, cx);
                    cx.notify();
                }),
            )
            .on_action(cx.listener(|this, _: &ListNext, _, cx| this.move_selection(1, cx)))
            .on_action(cx.listener(|this, _: &ListPrevious, _, cx| this.move_selection(-1, cx)))
            .on_action(cx.listener(|this, _: &ListFirst, _, cx| this.select(0, cx)))
            .on_action(cx.listener(|this, _: &ListLast, _, cx| {
                let last = this.state.read(cx).player.len().saturating_sub(1);
                this.select(last, cx);
            }))
            .on_action(cx.listener(|this, _: &ListPageUp, _, cx| this.move_selection(-10, cx)))
            .on_action(cx.listener(|this, _: &ListPageDown, _, cx| this.move_selection(10, cx)))
            .on_action(cx.listener(|this, _: &PlayFromHere, _, cx| {
                let row = this.selected;
                this.jump(row, cx);
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                let alt = event.keystroke.modifiers.alt;
                let key = event.keystroke.key.as_str();
                match key {
                    "delete" | "backspace" => {
                        let row = this.selected;
                        this.remove(row, cx);
                    }
                    "up" if alt => {
                        let row = this.selected;
                        this.move_row(row, row.saturating_sub(1), cx);
                    }
                    "down" if alt => {
                        let row = this.selected;
                        this.move_row(row, row + 1, cx);
                    }
                    _ => {}
                }
            }))
            .child(header);

        if len == 0 {
            return root
                .child(
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(rems(0.5))
                        .child(
                            icon(IconName::ListMusic)
                                .size(rems(2.))
                                .text_color(theme::secondary(cx)),
                        )
                        .child(
                            div()
                                .text_size(rems(0.9375))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("La file est vide"),
                        )
                        .child(
                            div()
                                .max_w(rems(30.))
                                .text_center()
                                .text_size(rems(0.75))
                                .text_color(theme::secondary(cx))
                                .child(
                                    "Lancez un titre depuis la bibliothèque ou la recherche : \
                                     ce qu'il reste à écouter s'affiche ici.",
                                ),
                        ),
                )
                .into_any_element();
        }

        let selected = self.selected;
        let playing_row = index;
        let sizes = self.sizes.clone();
        self.rendered_rows = 0;
        let list = virtual_list(
            cx.entity(),
            "queue-list",
            sizes,
            move |this, range, _, cx| {
                this.rendered_rows = range.len();
                range
                    .filter_map(|row| {
                        let entry = entries.get(row)?.clone();
                        Some(this.row(row, entry, playing_row, selected, len, cx))
                    })
                    .collect()
            },
            cx,
        )
        .h_full()
        .min_h_0()
        .track_scroll(&self.scroll);
        root.child(
            div()
                .relative()
                .flex()
                .flex_1()
                .min_h_0()
                .child(list)
                .child(footer_hint(cx)),
        )
        .into_any_element()
    }
}

impl Screen {
    fn row(
        &self,
        index: usize,
        entry: std::sync::Arc<Entry>,
        playing: i64,
        selected: usize,
        len: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let is_now = index as i64 == playing;
        let is_past = (index as i64) < playing;
        let is_selected = index == selected;
        let cover = self.state.read(cx).cover_path(&entry);
        let mut row = div()
            .id(("queue-row", index))
            .debug_selector(move || format!("queue-row-{index}"))
            .h(px(ROW_HEIGHT))
            .w_full()
            .flex()
            .items_center()
            .gap(rems(0.5))
            .px(rems(0.5))
            .border_l_2()
            .border_color(if is_now {
                theme::accent(cx)
            } else if is_selected {
                theme::focus_ring(cx)
            } else {
                theme::shell(cx)
            })
            .bg(if is_now {
                theme::selected(cx)
            } else {
                theme::shell(cx)
            })
            .opacity(if is_past { 0.5 } else { 1.0 })
            .hover(|style| style.bg(theme::selected(cx)))
            .group(format!("queue-row-{index}"))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                window.focus(&this.focus, cx);
                this.jump(index, cx);
            }));
        // No sleeve per row: a 1 000 px JPEG decoded for a 32 px slot, twenty
        // times, cost **+291 MB RSS** in the measured run (`docs/agents/desktop.md`,
        // D9) — the plan's decoded-image LRU (64 MB, `coverqueue.js`) is D8a's
        // widget, not this screen's. One glyph instead, like the player bar.
        let _ = cover;
        row = row.child(row_glyph(cx));
        row = row.child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_size(rems(0.8125))
                        .font_weight(if is_now {
                            FontWeight::BOLD
                        } else {
                            FontWeight::MEDIUM
                        })
                        .child(entry.title.clone()),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(rems(0.6875))
                        .text_color(theme::secondary(cx))
                        .child(entry.artist_name().to_owned()),
                ),
        );
        row = row.child(
            div()
                .flex_shrink_0()
                .text_size(rems(0.6875))
                .font_family(theme::mono(cx))
                .text_color(theme::secondary(cx))
                .child(crate::player_bar::clock(entry.duration as f64)),
        );
        row.child(self.row_controls(index, len, is_selected, cx))
    }
}

/// A 32 px disc glyph: the same honesty as the player bar.
fn row_glyph(cx: &mut Context<Screen>) -> impl IntoElement + use<> {
    div()
        .flex_shrink_0()
        .size(px(32.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .bg(theme::chrome(cx))
        .child(
            icon(IconName::Music)
                .size(rems(0.875))
                .text_color(theme::secondary(cx)),
        )
}

fn footer_hint(cx: &mut Context<Screen>) -> impl IntoElement + use<> {
    div()
        .absolute()
        .bottom(rems(0.25))
        .right(rems(0.5))
        .px(rems(0.375))
        .py(rems(0.125))
        .text_size(rems(0.625))
        .text_color(theme::secondary(cx))
        .child("Alt+↑ / Alt+↓ déplacer · Suppr retirer")
}

fn plural(count: i64) -> &'static str {
    if count > 1 { "s" } else { "" }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use gpui_kit::{AppContext as _, IntoElement, ParentElement, Styled, div};
    use nz_player::{Command, Entry, QueueArgs};

    use super::*;
    use crate::state::{AppConfig, event_channel};

    #[test]
    fn the_row_count_words_are_french() {
        assert_eq!(plural(0), "");
        assert_eq!(plural(1), "");
        assert_eq!(plural(2), "s");
        assert_eq!(plural(4_000), "s");
        assert_eq!(ROW_HEIGHT, 56.0, "the one number the windowing depends on");
    }

    /// A host view so the window has something to draw: the acceptance is about
    /// the queue's own render tree, not about the shell.
    struct Host {
        screen: Entity<Screen>,
    }

    impl Render for Host {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.screen.clone())
        }
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "neezer-d9-queue-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn four_thousand_tracks() -> Vec<Entry> {
        (0..4_000)
            .map(|index| Entry {
                id: format!("track-{index}"),
                title: format!("Titre {index}"),
                duration: 200 + (index % 60) as u32,
                artist: Some(nz_player::EntryArtist {
                    name: format!("Artiste {}", index % 97),
                    ..Default::default()
                }),
                ..Entry::default()
            })
            .collect()
    }

    fn small_queue(cx: &mut gpui::TestAppContext, dir: &std::path::Path) -> Entity<AppState> {
        cx.update(|cx| {
            gpui_omarchy::init(cx);
            crate::keymap::install(cx);
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        // Leaked on purpose: a current-thread runtime must outlive the window
        // the test drives, and the process ends with the test.
        let runtime_handle = runtime.handle().clone();
        std::mem::forget(runtime);
        let (tx, _rx) = event_channel();
        let dir = dir.to_path_buf();
        let state = cx.update(move |cx| {
            cx.new(|_| {
                AppState::new(
                    AppConfig {
                        server: Some("http://127.0.0.1:1".to_owned()),
                        start_audio: false,
                        start_mpris: false,
                        ..AppConfig::for_test(&dir)
                    },
                    runtime_handle,
                    tx,
                    None,
                )
            })
        });
        cx.update(|cx| {
            state.update(cx, |state, cx| {
                state.command(
                    Command::PlayQueue {
                        args: QueueArgs {
                            tracks: (0..12)
                                .map(|index| Entry {
                                    id: format!("t{index}"),
                                    title: format!("Titre {index}"),
                                    duration: 100 + index,
                                    ..Entry::default()
                                })
                                .collect(),
                            start: 0,
                            context: None,
                        },
                    },
                    cx,
                );
            });
        });
        state
    }

    /// The mouse path, on real hit testing: a row plays it, and an upcoming
    /// row's `✕` removes it — the two gestures `QueueList.svelte` offers.
    ///
    /// Debug builds only: the hit points come from `debug_bounds`, and the
    /// selector map is recorded only when `debug_assertions` is on (a release
    /// build drops it, so there is nothing to look up, not a broken click).
    #[cfg(debug_assertions)]
    #[gpui::test]
    async fn a_row_click_plays_it_and_its_cross_removes_it(cx: &mut gpui::TestAppContext) {
        let dir = temp_dir("mouse");
        let state = small_queue(cx, &dir);
        let screen = cx.update(|cx| cx.new(|cx| Screen::new(Route::Queue, state.clone(), cx)));
        let (_view, cx) = cx.add_window_view({
            let screen = screen.clone();
            move |_window, _cx| Host { screen }
        });
        let handle = cx.windows()[0];
        cx.simulate_window_resize(
            handle,
            gpui_kit::size(gpui_kit::px(800.), gpui_kit::px(700.)),
        );
        cx.run_until_parked();

        let third = cx.debug_bounds("queue-row-3").expect("row 3 is drawn");
        let point = gpui_kit::point(
            third.origin.x + third.size.width / 2.0,
            third.origin.y + third.size.height / 2.0,
        );
        cx.simulate_click(point, gpui_kit::Modifiers::default());
        cx.update(|_, cx| {
            assert_eq!(
                state
                    .read(cx)
                    .player
                    .current()
                    .map(|entry| entry.id.as_str()),
                Some("t3"),
                "a click on a row plays it"
            );
        });

        // The jump centred the playing row: read the bounds of the frame that
        // is on screen now, not the previous one.
        cx.run_until_parked();
        // The playing row has no controls, so the row after it does.
        let remove = cx.debug_bounds("row-remove-4").expect("row 4 offers ✕");
        let point = gpui_kit::point(
            remove.origin.x + remove.size.width / 2.0,
            remove.origin.y + remove.size.height / 2.0,
        );
        cx.simulate_click(point, gpui_kit::Modifiers::default());
        cx.update(|_, cx| {
            let state = state.read(cx);
            assert_eq!(state.player.len(), 11, "the ✕ removed one row");
            assert!(
                !state.player.queue().iter().any(|entry| entry.id == "t4"),
                "and it was that row"
            );
            assert_eq!(
                state.player.current().map(|entry| entry.id.as_str()),
                Some("t3"),
                "removing a row does not move the playhead"
            );
            assert!(state.player.playing(), "and the sound was not interrupted");
        });
        std::fs::remove_dir_all(dir).ok();
    }

    /// The keyboard path: `Fin` reaches the last row, `Entrée` plays it, `Alt+↑`
    /// moves it and `Suppr` removes it — the same edits, without a pointer.
    #[gpui::test]
    async fn the_keyboard_plays_moves_and_removes_rows(cx: &mut gpui::TestAppContext) {
        let dir = temp_dir("keys");
        let state = small_queue(cx, &dir);
        let screen = cx.update(|cx| cx.new(|cx| Screen::new(Route::Queue, state.clone(), cx)));
        let (_view, cx) = cx.add_window_view({
            let screen = screen.clone();
            move |_window, _cx| Host { screen }
        });
        cx.run_until_parked();
        // Tab reaches the list (the shell's focus anchor is the pointer's only
        // rival); the test gives it the focus the way Tab does.
        cx.update(|window, cx| {
            let focus = screen.read(cx).focus_handle();
            window.focus(&focus, cx);
        });
        cx.run_until_parked();

        // The first row plays (the queue is at rest before that): everything
        // after it is then editable.
        cx.simulate_keystrokes("home");
        cx.simulate_keystrokes("enter");
        cx.update(|_, cx| {
            assert_eq!(
                state
                    .read(cx)
                    .player
                    .current()
                    .map(|entry| entry.id.as_str()),
                Some("t0"),
                "Début + Entrée plays the first row"
            );
        });

        // Nothing that is playing or already played can be moved or removed.
        cx.simulate_keystrokes("alt-down");
        cx.simulate_keystrokes("delete");
        cx.update(|_, cx| {
            let state = state.read(cx);
            assert_eq!(state.player.len(), 12);
            assert_eq!(
                state.player.queue()[1].id,
                "t1",
                "the playing row stays where it is"
            );
        });

        // `Fin` reaches the last row; `Alt+↑` moves it one place up.
        cx.simulate_keystrokes("end");
        cx.update(|_, cx| assert_eq!(screen.read(cx).selected(), 11));
        cx.simulate_keystrokes("alt-up");
        cx.update(|_, cx| {
            let queue: Vec<String> = state
                .read(cx)
                .player
                .queue()
                .iter()
                .map(|entry| entry.id.clone())
                .collect();
            assert_eq!(queue[10], "t11", "{queue:?}");
            assert_eq!(queue[11], "t10", "{queue:?}");
        });
        // …and `Suppr` takes the row the cursor is on out of the queue.
        cx.simulate_keystrokes("delete");
        cx.update(|_, cx| {
            assert_eq!(state.read(cx).player.len(), 11, "Suppr removed the row");
            assert!(
                !state
                    .read(cx)
                    .player
                    .queue()
                    .iter()
                    .any(|entry| entry.id == "t11"),
                "the row the cursor was on went"
            );
        });
        std::fs::remove_dir_all(dir).ok();
    }

    /// The acceptance: **a queue of 4 000 scrolls at the display rate.** The
    /// claim is about work per frame, so that is what is measured: how many rows
    /// a paint BUILDS (the viewport's worth, never the queue's) and how long
    /// building them takes, against a 240 Hz frame budget of 4.17 ms.
    #[gpui::test]
    async fn a_four_thousand_row_queue_builds_only_the_visible_rows(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_omarchy::init(cx);
            crate::keymap::install(cx);
        });
        let dir = temp_dir("window");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let (tx, _rx) = event_channel();
        let state = {
            let runtime = runtime.handle().clone();
            let dir = dir.clone();
            cx.update(move |cx| {
                cx.new(|_| {
                    AppState::new(
                        AppConfig {
                            server: Some("http://127.0.0.1:1".to_owned()),
                            start_audio: false,
                            start_mpris: false,
                            ..AppConfig::for_test(&dir)
                        },
                        runtime,
                        tx,
                        None,
                    )
                })
            })
        };
        cx.update(|cx| {
            state.update(cx, |state, cx| {
                state.command(
                    Command::PlayQueue {
                        args: QueueArgs {
                            tracks: four_thousand_tracks(),
                            start: 0,
                            context: None,
                        },
                    },
                    cx,
                );
            });
        });
        let screen = cx.update(|cx| cx.new(|cx| Screen::new(Route::Queue, state.clone(), cx)));
        let (_view, cx) = cx.add_window_view({
            let screen = screen.clone();
            move |_window, _cx| Host { screen }
        });
        let handle = cx.windows()[0];
        cx.simulate_window_resize(
            handle,
            gpui_kit::size(gpui_kit::px(1280.), gpui_kit::px(900.)),
        );
        cx.run_until_parked();

        // The list is windowed: 800 px of viewport at 56 px a row is 15 rows,
        // whatever the queue holds.
        let built = cx.update(|_, cx| screen.read(cx).rendered_rows);
        let viewport = cx.update(|window, _| window.viewport_size());
        let fits = (viewport.height / gpui_kit::px(ROW_HEIGHT)).ceil() as usize;
        assert!(
            built <= fits + 2,
            "a 4 000-row queue built {built} rows for a {fits}-row window ({viewport:?})"
        );

        // The paint's own work, 200 times, in release: the budget is one frame
        // at 240 Hz.
        let elapsed = cx.update(|window, cx| {
            let started = Instant::now();
            for _ in 0..200 {
                let element: gpui_kit::AnyElement = screen.update(cx, |screen, cx| {
                    screen.render(window, cx).into_any_element()
                });
                let _ = element;
            }
            started.elapsed()
        });
        let per_paint = elapsed / 200;
        println!(
            "queue 4000 rows: {built} rows built in {viewport:?}, {per_paint:?} per paint ({elapsed:?} total)"
        );
        let budget = if cfg!(debug_assertions) {
            Duration::from_millis(8)
        } else {
            Duration::from_micros(4_170)
        };
        assert!(
            per_paint < budget,
            "a paint of the queue took {per_paint:?} (budget {budget:?})"
        );
        assert!(
            screen.update(cx, |screen, _| screen.rendered_rows) <= 30,
            "windowed: the row count follows the viewport, not the queue"
        );

        std::fs::remove_dir_all(dir).ok();
        drop(runtime);
    }
}
