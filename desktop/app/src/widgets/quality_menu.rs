//! Le choix de qualité de streaming, porté de `QualityMenu.svelte`.
//!
//! The SPA names the setting in one mono badge and opens the list of rungs with
//! a plain-French hint; the desktop does the same, with the rungs the desktops
//! engine actually has.
//!
//! **One deliberate reduction, and why it is not a subset**: the SPA lists six
//! rungs (FLAC, 320, 256, 192, 128, 64) because its `<audio>` element takes any
//! of them. This app's quality ladder is `wiring/engine.rs#PlaybackQuality`
//! (Auto, FLAC, 320, 128, 64) — the plan's own *Streams* contract — and 256/192
//! exist on the server but nowhere in the desktop's type. A menu that offered
//! them would save a value the player can never load, so the badge names what
//! the engine will really ask for.
//!
//! **A change applies at the NEXT track, and says so.** The engine's source is
//! opened by `AppState::load`, which reads `state.quality`; switching the
//! setting mid-track must not restart the sound under the person. The toast is
//! the SPA's own sentence, and the same one `AppState::cycle_quality` pushes
//! (the `q`-less path behind `Réglages → Lecture`).

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Subscription, Window, div, px, rems,
};
use gpui_omarchy::{IconName, icon};

use crate::state::{self, AppState};
use crate::theme;
use crate::toast::ToastKind;
use crate::wiring::engine::PlaybackQuality;

/// The rungs, in the order the menu lists them (best first, as the SPA does).
pub const QUALITIES: [PlaybackQuality; 5] = [
    PlaybackQuality::Auto,
    PlaybackQuality::Flac,
    PlaybackQuality::Opus320,
    PlaybackQuality::Opus128,
    PlaybackQuality::Opus64,
];

/// The badge's short form.
pub fn short(quality: PlaybackQuality) -> &'static str {
    match quality {
        PlaybackQuality::Auto => "Auto",
        PlaybackQuality::Flac => "FLAC",
        PlaybackQuality::Opus320 => "320",
        PlaybackQuality::Opus128 => "128",
        PlaybackQuality::Opus64 => "64",
    }
}

/// The list's own words: what the rung means, never a bare number.
pub fn hint(quality: PlaybackQuality) -> &'static str {
    match quality {
        PlaybackQuality::Auto => "Selon le débit mesuré",
        PlaybackQuality::Flac => "Sans perte",
        PlaybackQuality::Opus320 => "Haute qualité",
        PlaybackQuality::Opus128 => "Standard",
        PlaybackQuality::Opus64 => "Données réduites",
    }
}

/// What the badge's accessible name says.
pub fn label(quality: PlaybackQuality) -> &'static str {
    state::quality_label(quality)
}

/// Choose `quality`: saved on the device (the SPA keeps this one locally, not
/// on the server — `stores.js#quality`), announced, and picked up by the next
/// `Load`.
pub fn select(state: &mut AppState, quality: PlaybackQuality) {
    if state.quality == quality {
        return;
    }
    state.quality = quality;
    if let Some(cache) = &state.cache
        && let Err(error) = cache
            .kv()
            .set("playback.quality", &quality.key().to_owned())
    {
        tracing::warn!(%error, "la qualité n'a pas pu être enregistrée");
    }
    state.toasts.push(
        ToastKind::Info,
        format!("Qualité : {} — au prochain titre", label(quality)),
    );
}

/// The badge and its drop-down list.
pub struct QualityMenu {
    pub state: Entity<AppState>,
    open: bool,
    _subscription: Subscription,
}

impl QualityMenu {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            open: false,
            _subscription: subscription,
        }
    }

    pub(crate) fn pick(&mut self, quality: PlaybackQuality, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            select(state, quality);
            cx.notify();
        });
        self.open = false;
        cx.notify();
    }
}

impl Render for QualityMenu {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.state.read(cx).quality;
        let trigger = div()
            .id("quality-trigger")
            .flex()
            .items_center()
            .gap(rems(0.25))
            .h(px(26.))
            .px(rems(0.5))
            .border_1()
            .border_color(theme::accent(cx))
            .font_family(theme::mono(cx))
            .text_size(rems(0.6875))
            .text_color(theme::text(cx))
            .cursor_pointer()
            .hover(|style| style.bg(theme::selected(cx)))
            .on_click(cx.listener(|this, _, _, cx| {
                this.open = !this.open;
                cx.notify();
            }))
            .child(short(current))
            .child(
                icon(IconName::ChevronDown)
                    .size(rems(0.6875))
                    .text_color(theme::secondary(cx)),
            );

        let mut chip = div()
            .id("quality-menu")
            .relative()
            .flex()
            .items_center()
            .child(trigger);

        if self.open {
            let mut menu = div()
                .id("quality-list")
                .absolute()
                .top(px(32.))
                .right_0()
                .w(rems(15.))
                .flex()
                .flex_col()
                .gap(rems(0.125))
                .p(rems(0.375))
                .border_1()
                .border_color(theme::border(cx))
                .bg(theme::chrome(cx))
                .shadow_lg();
            for quality in QUALITIES {
                let selected = quality == current;
                menu = menu.child(
                    div()
                        .id(gpui_kit::ElementId::Name(gpui_kit::SharedString::from(
                            format!("quality-{}", short(quality)),
                        )))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(rems(0.5))
                        .h(rems(2.25))
                        .px(rems(0.5))
                        .cursor_pointer()
                        .hover(|style| style.bg(theme::selected(cx)))
                        .on_click(cx.listener(move |this, _, _, cx| this.pick(quality, cx)))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_size(rems(0.8125))
                                        .text_color(if selected {
                                            theme::accent(cx)
                                        } else {
                                            theme::text(cx)
                                        })
                                        .child(label(quality)),
                                )
                                .child(
                                    div()
                                        .text_size(rems(0.6875))
                                        .font_family(theme::mono(cx))
                                        .text_color(theme::secondary(cx))
                                        .child(hint(quality)),
                                ),
                        )
                        .when(selected, |row| {
                            row.child(
                                icon(IconName::Check)
                                    .size(rems(0.875))
                                    .text_color(theme::accent(cx)),
                            )
                        }),
                );
            }
            chip = chip.child(menu);
        }
        chip
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The badge's name and the list's hint are one table per rung: a rung added
    /// to `PlaybackQuality` without its two words would render a blank row.
    #[test]
    fn every_rung_of_the_ladder_is_named_and_hinted() {
        assert_eq!(QUALITIES.len(), 5);
        let mut seen = std::collections::HashSet::new();
        for quality in QUALITIES {
            assert!(!short(quality).is_empty());
            assert!(!hint(quality).is_empty());
            assert!(!label(quality).is_empty());
            assert!(seen.insert(short(quality)), "two rungs share a badge");
        }
        assert_eq!(short(PlaybackQuality::Auto), "Auto");
        assert_eq!(hint(PlaybackQuality::Flac), "Sans perte");
        assert_eq!(label(PlaybackQuality::Opus64), "Opus 64");
    }
}
