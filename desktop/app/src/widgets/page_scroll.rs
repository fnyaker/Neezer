//! The one vertical page scroller: a screen whose content is taller than the
//! window.
//!
//! **A page that scrolls is TWO facts, and a screen needs both.** (1) The
//! element with `overflow_y_scroll` must have a BOUNDED height — it is a flex
//! item with `flex_1` inside the shell's column — and (2) its direct child must
//! be as TALL AS ITS CONTENT. GPUI measures a scroll container from its direct
//! children's boxes (`Div::prepaint`, `content_size`), not from what those
//! children paint past their own bottom edge: a `flex_1` page column is exactly
//! one viewport tall, so `max_offset` is 0 and the wheel, `Page suivante` and
//! the touchpad all do nothing while the sections below the fold are clipped
//! away. That was the Accueil's bug, and the same construction was in
//! Nouveautés, le Récap, Bienvenue and the search results page (found by
//! reading every `overflow_y_scroll`, then by the wheel on each).
//!
//! So a screen writes [`PageScroll::container`] for the scroller and
//! [`column`] for the page, and [`PageScroll::own`] for the keys. The keys are
//! app-level interceptors: a screen must never take the focus (D8b), and the
//! keystroke is CONSUMED so the shared list actions (`ListPageDown` and
//! friends) do not move a row list at the same time — `j`/`k` stay the list's.

use gpui_kit::{
    App, Context, Div, ElementId, InteractiveElement as _, Pixels, Point, Render, ScrollHandle,
    Stateful, StatefulInteractiveElement as _, Styled, Subscription, Window, div, px, rems,
};

/// What a keystroke asks the page to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Page {
    /// An arrow key: a few lines.
    Line(f32),
    /// `Page précédente` / `Page suivante`: most of the window.
    Screen(f32),
    Top,
    Bottom,
}

/// How much of the window one `Page suiv.` moves. A browser takes ~0.9; 0.8
/// keeps a line of context where the person was reading.
const PAGE_SHARE: f32 = 0.8;
/// How far one arrow moves: four lines of the shell's rem type.
const LINE_REMS: f32 = 4.;
/// What `Fin` asks for before the first paint has measured the content; the
/// container clamps it to the real maximum on that frame.
const FAR: f32 = -1.0e7;

/// A page's scroll offset, its keys, and the container that draws it.
#[derive(Clone, Default)]
pub struct PageScroll {
    scroll: ScrollHandle,
}

impl PageScroll {
    pub fn new() -> Self {
        Self::default()
    }

    /// The container: full size, scrolls vertically, tracked. The screen chains
    /// its own font, background and `key_context` on top.
    pub fn container(&self, id: impl Into<ElementId>) -> Stateful<Div> {
        div()
            .id(id)
            .size_full()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
    }

    /// Where the page is, in pixels below the top (0 at the top, negative and
    /// growing as it goes down).
    pub fn offset(&self) -> Point<Pixels> {
        self.scroll.offset()
    }

    /// How far the page can travel, in pixels (positive), 0 while it fits the
    /// window. The offset itself is negative going down.
    pub fn range(&self) -> Pixels {
        self.scroll.max_offset().y
    }

    /// Move the page. `rem` and `viewport` come from the window, which is what
    /// the interceptor has: `A page` is `PAGE_SHARE` of the window.
    pub fn scroll(&mut self, how: Page, rem: Pixels, viewport: Pixels) {
        let range = self.range();
        // Before the first paint the handle knows neither bound: the container
        // clamps whatever is set here on that frame, which is also why a `Fin`
        // asked this early still lands at the bottom.
        let floor = if range > px(0.) { -range } else { px(FAR) };
        let mut offset = self.offset();
        let line = rems(LINE_REMS).to_pixels(rem);
        let asked = match how {
            // The offset is negative going down, so a positive direction
            // SUBTRACTS: `↓` and `Page suivante` take the page further down.
            Page::Line(direction) => offset.y - line * direction,
            Page::Screen(direction) => offset.y - viewport * PAGE_SHARE * direction,
            Page::Top => px(0.),
            Page::Bottom => px(FAR),
        };
        offset.y = asked.clamp(floor, px(0.));
        self.scroll.set_offset(offset);
    }

    /// The page's keys, for as long as the screen that owns this handle lives:
    /// the returned [`Subscription`] MUST be kept by the screen (`cx`'s own
    /// subscriptions would do, a field is clearer), or the interceptor outlives
    /// the page it scrolls.
    ///
    /// `↑`/`↓`, `Page préc./suiv.`, `Début`/`Fin` scroll the PAGE — what a
    /// browser does on the SPA — and nothing else: `Ctrl+↑` is the volume and
    /// `Shift+←` a seek, so only the bare stroke is ours. A screen that has
    /// been dropped consumes nothing: the key goes on to whatever is on screen.
    pub fn own<V: Render + 'static>(&self, cx: &mut Context<V>) -> Subscription {
        let mut page = self.clone();
        let weak = cx.weak_entity();
        App::intercept_keystrokes(&mut *cx, move |event, window: &mut Window, cx: &mut App| {
            let gpui_kit::KeystrokeEvent { keystroke, .. } = event;
            if keystroke.modifiers.modified() {
                return;
            }
            let how = match keystroke.key.as_str() {
                "up" => Page::Line(-1.),
                "down" => Page::Line(1.),
                "pageup" => Page::Screen(-1.),
                "pagedown" => Page::Screen(1.),
                "home" => Page::Top,
                "end" => Page::Bottom,
                _ => return,
            };
            let rem = window.rem_size();
            let viewport = window.viewport_size().height;
            let handled = weak.update(cx, |_, cx| {
                page.scroll(how, rem, viewport);
                cx.notify();
            });
            if handled.is_ok() {
                cx.stop_propagation();
            }
        })
    }
}

/// The page column a [`PageScroll::container`] holds: as tall as its CONTENT
/// (`flex_shrink_0`, never `flex_1` — see this module's note), as wide as the
/// window, and capped at `max_w` when the screen asks for a reading measure.
pub fn column() -> Div {
    div().flex().flex_col().flex_shrink_0().w_full()
}

#[cfg(test)]
mod tests {
    use gpui_kit::{
        Context, IntoElement, Modifiers, ParentElement, Render, ScrollDelta, ScrollWheelEvent,
        Styled, TouchPhase, Window, div, point, px, size,
    };

    use super::*;

    /// A stand-in page: a container with two dozen 100 px blocks in it.
    struct Bench {
        page: PageScroll,
        /// The bug, reproduced on demand: the page column with `flex_1`.
        clamped: bool,
    }

    impl Render for Bench {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let mut column = if self.clamped {
                div().flex().flex_col().flex_1().min_h_0()
            } else {
                column()
            };
            for index in 0..24 {
                column = column.child(div().h(px(100.)).child(format!("bloc {index}")));
            }
            self.page.container("bench-page").child(column)
        }
    }

    fn open(
        cx: &mut gpui::TestAppContext,
        clamped: bool,
    ) -> (gpui_kit::Entity<Bench>, &mut gpui::VisualTestContext) {
        let page = PageScroll::new();
        let (bench, cx) = cx.add_window_view(move |_, _| Bench { page, clamped });
        cx.simulate_resize(size(px(1280.), px(800.)));
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        (bench, cx)
    }

    /// The rule this module exists for: a page column at its CONTENT height
    /// gives the scroller something to scroll (24 × 100 px in a 800 px window
    /// is 1 600 px of travel), and the same page with `flex_1` gives ZERO —
    /// which is exactly the Accueil of v1.6.0, where the wheel, the touchpad and
    /// the keys did nothing and the sections below the fold were unreachable.
    #[gpui::test]
    async fn a_content_sized_column_scrolls_and_a_flex_1_one_does_not(
        cx: &mut gpui::TestAppContext,
    ) {
        let (bench, cx) = open(cx, false);
        let range = cx.update(|_, cx| bench.read(cx).page.range());
        assert_eq!(range, px(1600.), "24 blocks of 100 in an 800 px window");

        let (clamped, cx) = open(cx, true);
        let range = cx.update(|_, cx| clamped.read(cx).page.range());
        assert_eq!(
            range,
            px(0.),
            "a flex_1 page column is one viewport tall: the bug, pinned"
        );
    }

    /// The wheel and the keys move the page; the offset is read through the
    /// same handle the screen uses.
    #[gpui::test]
    async fn the_wheel_and_the_keys_move_the_page(cx: &mut gpui::TestAppContext) {
        let (bench, cx) = open(cx, false);
        // A screen installs the keys on itself; this bench does the same thing.
        let subscription = cx.update(|_, cx| bench.update(cx, |bench, cx| bench.page.own(cx)));
        cx.simulate_mouse_move(point(px(640.), px(400.)), None, Modifiers::default());
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(640.), px(400.)),
            delta: ScrollDelta::Lines(point(0., -3.)),
            modifiers: Modifiers::default(),
            touch_phase: TouchPhase::Moved,
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let wheeled = cx.update(|_, cx| bench.read(cx).page.offset().y);
        assert!(wheeled < px(-10.), "the wheel scrolled: {wheeled:?}");

        cx.simulate_keystrokes("pagedown");
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let paged = cx.update(|_, cx| bench.read(cx).page.offset().y);
        assert!(
            paged < wheeled - px(200.),
            "Page suivante took most of a window: {paged:?}"
        );

        cx.simulate_keystrokes("end");
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let bottom = cx.update(|_, cx| bench.read(cx).page.offset().y);
        assert_eq!(bottom, px(-1600.), "Fin reaches the bottom");
        drop(subscription);
    }
}
