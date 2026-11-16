//! The discovery level: how far the Flow and the suggestions reach, and the one
//! control that moves it.
//!
//! `lib/discovery.js` is the reference. Its two load-bearing rules are kept:
//!
//! - **A click is a value, never a drag of requests.** The control is six named
//!   notches (`lib/flow.js#DIVERSITY_STEPS`), so one click is one level, and a
//!   save waits [`SETTLE_MS`] after the last click: twenty clicks in three
//!   seconds are ONE `PUT /api/discover/prefs`, with the last level.
//! - **The device paints first, the server decides.** The level lives on the
//!   server (`/api/discover/prefs`) so the phone and the desktop agree, and the
//!   copy in `nz-cache`'s `kv` (`discover.level`) is what the control shows
//!   before any answer — the SPA's `localStorage` key, byte for byte.
//!
//! A level moved while a Flow plays re-tunes it: the next title is chosen again
//! at the new breadth, never the one being heard (`lib/flow.js#retune`).

use std::cell::RefCell;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use gpui_kit::{
    AnyElement, App, AsyncApp, BorrowAppContext as _, Context, Entity, InteractiveElement as _,
    IntoElement, ParentElement, Render, StatefulInteractiveElement as _, Styled, div, rems,
};
use gpui_omarchy::{toggle, toggle_group};

use crate::state::AppState;
use crate::theme;
use crate::widgets::flow::{self, DIVERSITY_STEPS, diversity_index};

/// How long after the last click the level is saved (`lib/discovery.js`).
pub const SETTLE_MS: u64 = 450;

/// The generation of the last click on this process: an older save whose level
/// was already replaced does nothing.
#[derive(Default)]
struct Settling(RefCell<u64>);

impl gpui_kit::Global for Settling {}

fn next_generation(cx: &mut App) -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    if !cx.has_global::<Settling>() {
        cx.set_global(Settling::default());
    }
    let _ = COUNTER;
    cx.update_global::<Settling, _>(|settling, _| {
        let mut generation = settling.0.borrow_mut();
        *generation += 1;
        *generation
    })
}

/// The generation of the last click, read from a task (`AsyncApp` has no
/// `&App`), so a save that was overtaken knows to stay quiet.
fn generation(cx: &AsyncApp) -> u64 {
    cx.try_read_global::<Settling, _>(|settling, _| *settling.0.borrow())
        .unwrap_or(0)
}

/// The six notches, drawn as the Flow's own segmented control: one click is one
/// level, `Left`/`Right` move between them once one has the focus
/// (`gpui_omarchy::toggle` is a real, focusable button).
pub fn notches<V: Render>(state: &Entity<AppState>, level: f64, cx: &mut Context<V>) -> AnyElement {
    let current = diversity_index(level);
    let mut group = toggle_group("discovery-notches", cx);
    for (index, (label, value, _)) in DIVERSITY_STEPS.iter().enumerate() {
        let value = *value;
        group = group.child(
            toggle(("discovery-notch", index), *label, current == index, cx)
                .debug_selector(move || format!("discovery-notch-{index}"))
                .on_click(cx.listener({
                    let state = state.clone();
                    move |_, _, _, cx| pick(&state, value, cx)
                })),
        );
    }
    group.into_any_element()
}

/// Move the level: the device's copy at once, one `PUT` once the hand settles,
/// and the Flow retuned if it is the queue playing.
pub fn pick(state: &Entity<AppState>, level: f64, cx: &mut App) {
    let Some(api) = state.read(cx).api.clone() else {
        return;
    };
    let runtime = state.read(cx).runtime.clone();
    let cache = state.read(cx).cache.clone();
    let level = if level.is_finite() {
        level.clamp(0., 1.)
    } else {
        return;
    };
    if let Some(cache) = &cache
        && let Err(error) = cache.kv().set(flow::LEVEL_KEY, &level)
    {
        tracing::debug!(%error, "le niveau de découverte n'a pas pu être enregistré");
    }
    let mine = next_generation(cx);
    let entity = state.clone();
    state.update(cx, |_, cx: &mut Context<AppState>| {
        cx.notify();
        // The save: one task per click, and only the newest one may speak.
        cx.spawn(async move |_state, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(SETTLE_MS))
                .await;
            if generation(cx) != mine {
                return;
            }
            runtime.spawn(async move {
                if let Err(error) = api.set_discover_level(level).await {
                    tracing::debug!(%error, "le niveau de découverte n'a pas été enregistré");
                }
            });
            // A Flow playing chooses its next title at the new breadth.
            entity.update(cx, |_, cx| {
                flow::retune(&entity, cx);
            });
        })
        .detach();
    });
}

/// Save a level that came from the SERVER (`loadDiscovery`): the device's copy
/// only, no request — the server is where it came from.
pub fn adopt(state: &Entity<AppState>, level: f64, cx: &mut App) {
    if !level.is_finite() {
        return;
    }
    let level = level.clamp(0., 1.);
    if let Some(cache) = state.read(cx).cache.clone()
        && let Err(error) = cache.kv().set(flow::LEVEL_KEY, &level)
    {
        tracing::debug!(%error, "le niveau de découverte n'a pas pu être recopié");
    }
    state.update(cx, |_, cx| cx.notify());
}

/// The name of the notch a level reads back on, for a screen that shows it
/// without the control.
pub fn name(level: f64) -> &'static str {
    crate::widgets::flow::diversity_step(level).0
}

/// The one-line hint of that notch.
pub fn hint(level: f64) -> &'static str {
    crate::widgets::flow::diversity_step(level).2
}

/// A small caption row other screens can put under a heading: the notch's name
/// and what it means.
pub fn caption(level: f64, cx: &mut App) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(rems(0.375))
        .text_size(rems(0.6875))
        .text_color(theme::secondary(cx))
        .child(name(level))
        .child("·")
        .child(hint(level))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The control offers every notch the Flow's body understands: six, in the
    /// SPA's order, each with its level.
    #[test]
    fn the_control_offers_the_six_notches_of_the_body() {
        assert_eq!(DIVERSITY_STEPS.len(), 6);
        assert_eq!(DIVERSITY_STEPS[0].1, 0.);
        assert_eq!(DIVERSITY_STEPS[5].1, 1.);
        assert_eq!(name(0.4), "Voisin");
        assert_eq!(hint(0.4), "Des artistes proches de vos favoris.");
        // A value between two stops paints the nearest one, never nothing.
        for level in [0.0, 0.1, 0.35, 0.5, 0.79, 0.8, 1.0] {
            assert!(!name(level).is_empty());
        }
    }
}
