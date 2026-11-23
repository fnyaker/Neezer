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
    App, Context, Div, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    Point, Render, ScrollHandle, Stateful, StatefulInteractiveElement as _, Styled, Subscription,
    Window, div, px, rems,
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

    /// This page's own handle, for a nested scrollable that must chain its
    /// wheel to the page ([`nested`]).
    pub fn handle(&self) -> ScrollHandle {
        self.scroll.clone()
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

/// What an inner scroller could not take of `delta`, in the PAGE's sign
/// (negative = further down the page).
///
/// The one piece of arithmetic [`nested`] and [`nested_in_list`] share: the
/// inner container has already applied `delta` (it is deeper, so GPUI ran its
/// listener first), its PREVIOUS offset is `offset - delta`, and its bound
/// says how much of the notch it could really take.
fn leftover(inner: &ScrollHandle, delta: Pixels) -> Pixels {
    let max = inner.max_offset().y.max(px(0.));
    let before = (inner.offset().y - delta).clamp(-max, px(0.));
    let room = if delta < px(0.) {
        (before + max).max(px(0.))
    } else {
        (-before).max(px(0.))
    };
    let rest = (delta.abs() - room).max(px(0.));
    if delta < px(0.) { -rest } else { rest }
}

/// A scrollable that lives INSIDE a page: the wheel moves it first, and the
/// page only by the part it could not take.
///
/// **The rule, one wheel notch has one effect.** GPUI hands a wheel event to
/// EVERY scroll container whose hitbox is under the pointer, not only the
/// innermost (`Hitbox::should_handle_scroll` tests the whole hit-test stack,
/// `gpui-pre` 0.3.9 `window.rs`), so a bounded list inside a page scroller
/// scrolled BOTH by the same notch: two bars, twice the travel, or a list that
/// looks stuck because the page moved underneath it. Every browser chains the
/// scroll instead — the inner container consumes it and what is left goes to
/// the ancestor once the boundary is reached (MDN, *Scroll chaining*; the CSS
/// `overscroll-behavior` property). GPUI has no `overscroll-behavior`; this
/// wrapper is it, in the bubble phase:
///
/// - the inner scroll container's own listener has already applied `delta`
///   (it is deeper, so GPUI runs it first), and it is clamped to its bound on
///   the next prepaint, so its PREVIOUS offset is `offset - delta`;
/// - whatever its remaining travel could not take is applied to `page`;
/// - the event is stopped there, so the page's own listener never applies the
///   same notch a second time.
///
/// `inner` is the handle the list scrolls with (`TrackList::scroll_handle`,
/// a `VirtualListScrollHandle`'s base handle, …). The wrapper must be a direct
/// child of a bounded box: it is `size_full`, so give it the list's height.
pub fn nested(
    id: impl Into<ElementId>,
    page: &ScrollHandle,
    inner: &ScrollHandle,
    child: impl IntoElement,
) -> Stateful<Div> {
    let page = page.clone();
    let inner = inner.clone();
    div()
        .id(id)
        .size_full()
        .on_scroll_wheel(move |event, window, cx| {
            let delta = event.delta.pixel_delta(window.line_height()).y;
            if delta == px(0.) {
                return;
            }
            let shift = leftover(&inner, delta);
            if shift != px(0.) {
                let bound = page.max_offset().y.max(px(0.));
                let mut offset = page.offset();
                offset.y = (offset.y + shift).clamp(-bound, px(0.));
                page.set_offset(offset);
                window.refresh();
            }
            cx.stop_propagation();
        })
        .child(child)
}

/// [`nested`] for a page whose scroller is a `gpui::list` (`ListState`), which
/// has no `ScrollHandle`: the part the inner band could not take is given to
/// the list with `scroll_by`, the same rule with the list's own arithmetic.
pub fn nested_in_list(
    id: impl Into<ElementId>,
    list: &gpui_kit::ListState,
    inner: &ScrollHandle,
    child: impl IntoElement,
) -> Stateful<Div> {
    let list = list.clone();
    let inner = inner.clone();
    div()
        .id(id)
        .size_full()
        .on_scroll_wheel(move |event, window, cx| {
            let delta = event.delta.pixel_delta(window.line_height()).y;
            if delta == px(0.) {
                return;
            }
            let shift = leftover(&inner, delta);
            if shift != px(0.) {
                // `leftover` is in the PAGE's sign (negative = down); the list
                // moves by pixels, positive going down.
                list.scroll_by(-shift);
                window.refresh();
            }
            cx.stop_propagation();
        })
        .child(child)
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

    /// A page with a bounded, scrollable list at its top: the shape of
    /// l'Accueil's « Pour toi » band, de Nouveautés' unfolded titles, du panneau
    /// de lecture. The list is 300 px tall and holds 1 200 px of rows (900 px of
    /// travel), and the page holds 1 200 px more below it.
    struct Nested {
        page: PageScroll,
        inner: ScrollHandle,
        /// The bug, reproduced on demand: the list placed in the page with no
        /// chaining wrapper (what every nested list looked like before).
        bare: bool,
    }

    impl Render for Nested {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let list = div()
                .id("nested-inner")
                .h(px(300.))
                .overflow_y_scroll()
                .track_scroll(&self.inner)
                .child(div().flex().flex_col().children(
                    (0..12).map(|index| div().h(px(100.)).child(format!("ligne {index}"))),
                ));
            let list: gpui_kit::AnyElement = if self.bare {
                list.into_any_element()
            } else {
                nested("nested-list", &self.page.handle(), &self.inner, list).into_any_element()
            };
            // The list's height is the page's (`nested` is `size_full`).
            let mut page = column().child(div().h(px(300.)).child(list));
            for index in 0..12 {
                page = page.child(div().h(px(100.)).child(format!("bloc {index}")));
            }
            self.page.container("nested-page").child(page)
        }
    }

    fn open_nested(
        cx: &mut gpui::TestAppContext,
        bare: bool,
    ) -> (gpui_kit::Entity<Nested>, &mut gpui::VisualTestContext) {
        let (bench, cx) = cx.add_window_view(move |_, _| Nested {
            page: PageScroll::new(),
            inner: ScrollHandle::new(),
            bare,
        });
        cx.simulate_resize(size(px(1280.), px(800.)));
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        (bench, cx)
    }

    fn wheel(cx: &mut gpui::VisualTestContext, y: Pixels) {
        cx.simulate_mouse_move(point(px(100.), y), None, Modifiers::default());
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(100.), y),
            delta: ScrollDelta::Lines(point(0., -3.)),
            modifiers: Modifiers::default(),
            touch_phase: TouchPhase::Moved,
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
    }

    fn wheel_up(cx: &mut gpui::VisualTestContext, y: Pixels) {
        cx.simulate_mouse_move(point(px(100.), y), None, Modifiers::default());
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(100.), y),
            delta: ScrollDelta::Lines(point(0., 3.)),
            modifiers: Modifiers::default(),
            touch_phase: TouchPhase::Moved,
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
    }

    /// **The rule, one notch has one effect.** GPUI applies a wheel event to
    /// EVERY scroll container under the pointer, so before this rule the list
    /// and the page both moved by the same 78 px notch: the page's `range` is
    /// 1 200+ px and the list's 900, so a person scrolling the « Pour toi » band
    /// watched the whole page slide with it, or the list looked stuck while the
    /// page moved. Measured here: over the list, one notch moves the list
    /// **78 px** and the page **0** (it moved 78 before); the list stops at its
    /// **900 px** bound and the notch then moves the page. Over the page below
    /// the list, one notch moves the page and leaves the list alone.
    #[gpui::test]
    async fn the_inner_list_takes_the_wheel_and_the_page_gets_the_rest(
        cx: &mut gpui::TestAppContext,
    ) {
        // The bug, pinned: without the wrapper GPUI gives the notch to BOTH
        // containers, exactly what a person saw (page and list at 78 px).
        let (bare, bare_cx) = open_nested(cx, true);
        wheel(bare_cx, px(100.));
        let (page, inner) = bare_cx.update(|_, cx| {
            let bare = bare.read(cx);
            (bare.page.offset().y, bare.inner.offset().y)
        });
        assert_eq!(page, inner, "both scrollers took the same notch: the bug");
        assert!(page < px(-10.), "and both really moved: {page:?}");

        let (bench, cx) = open_nested(cx, false);
        let offsets = |cx: &mut gpui::VisualTestContext| {
            cx.update(|_, cx| {
                let bench = bench.read(cx);
                (bench.page.offset().y, bench.inner.offset().y)
            })
        };

        // The list is at the top of the page: y = 100 is inside it.
        wheel(cx, px(100.));
        let (page, inner) = offsets(cx);
        assert!(inner < px(-10.), "the list took the notch: {inner:?}");
        assert_eq!(page, px(0.), "the page did not take it too: {page:?}");

        // The list reaches its 900 px bound; the next notches belong to the page.
        for _ in 0..20 {
            wheel(cx, px(100.));
        }
        let (page, inner) = offsets(cx);
        assert_eq!(inner, px(-900.), "the list stopped at its own bound");
        assert!(
            page < px(-100.),
            "the page got what the list could not take: {page:?}"
        );

        // And over the page itself the list does not move at all. The page is
        // at its bottom here, so the notch is upward.
        let (_, before) = offsets(cx);
        wheel_up(cx, px(500.));
        let (page_after, inner_after) = offsets(cx);
        assert_eq!(inner_after, before, "the page's notch left the list alone");
        assert!(
            page_after > page,
            "the page moved: {page_after:?} from {page:?}"
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

    /// A page whose scroller is a `gpui::list` (Nouveautés), with a 300 px
    /// band of 1 200 px of rows as its first item: the shape
    /// [`nested_in_list`] exists for.
    struct ListedPage {
        list: gpui_kit::ListState,
        inner: ScrollHandle,
    }

    impl Render for ListedPage {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let inner = self.inner.clone();
            let list_state = self.list.clone();
            gpui_kit::list(self.list.clone(), move |index, _window, _cx| {
                if index != 0 {
                    return div()
                        .h(px(300.))
                        .child(format!("bloc {index}"))
                        .into_any_element();
                }
                let band = div()
                    .id("listed-inner")
                    .h(px(300.))
                    .overflow_y_scroll()
                    .track_scroll(&inner)
                    .child(div().flex().flex_col().children(
                        (0..12).map(|row| div().h(px(100.)).child(format!("ligne {row}"))),
                    ));
                nested_in_list("listed-band", &list_state, &inner, band).into_any_element()
            })
            .w_full()
            .h(px(300.))
        }
    }

    /// **The rule, one notch has one effect — with a LIST as the page.**
    /// [`nested`] states it for a `ScrollHandle` page; Nouveautés' page is a
    /// `gpui::list`, which has none. Measured: one notch over the band moves
    /// the BAND 78 px and leaves the list at item 0 (before the wrapper GPUI
    /// gave the notch to both), and once the band is at its bound the notches
    /// move the list.
    #[gpui::test]
    async fn a_band_in_a_list_page_takes_the_wheel_and_the_list_gets_the_rest(
        cx: &mut gpui::TestAppContext,
    ) {
        let (page, cx) = cx.add_window_view(|_, _| ListedPage {
            list: gpui_kit::ListState::new(10, gpui_kit::ListAlignment::Top, px(200.)),
            inner: ScrollHandle::new(),
        });
        cx.simulate_resize(size(px(1280.), px(800.)));
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });

        wheel(cx, px(100.));
        let (band, top) = cx.update(|_, cx| {
            let page = page.read(cx);
            (page.inner.offset().y, page.list.logical_scroll_top())
        });
        assert!(band < px(-10.), "the band took the notch: {band:?}");
        assert_eq!(top.item_ix, 0, "the list did not move with it");
        assert_eq!(top.offset_in_item, px(0.), "the list did not move with it");

        // Twelve more notches exhaust the band's 900 px of travel; the rest
        // go to the list (which is how a person reaches the rows below it).
        for _ in 0..12 {
            wheel(cx, px(100.));
        }
        let top = cx.update(|_, cx| page.read(cx).list.logical_scroll_top());
        assert!(
            top.item_ix > 0 || top.offset_in_item > px(0.),
            "the list took what the band could not: {top:?}"
        );
    }
}
