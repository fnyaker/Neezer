//! The `LoadFailed.svelte` equivalent: a screen that cannot load says why and
//! offers « Réessayer », never an endless spinner (rule 3).
//!
//! D7 owns the shell's version because the shell is the first screen that can
//! fail: a stored session whose `/api/me` cannot answer. Later screens render
//! the same view with their own cause (`LoadFailed`, `Failed`, whatever their
//! data path returns), so the wording and the gesture stay identical.

use gpui_kit::{Context, Div, ParentElement, SharedString, Styled, div, rems};
use gpui_omarchy::{ButtonVariant, IconName, button, icon};

use crate::app::RootView;
use crate::theme;

/// A screen-shaped failure: what could not load, why, and what the person can
/// do about it.
pub fn view(
    title: impl Into<SharedString>,
    cause: impl Into<SharedString>,
    cx: &mut Context<RootView>,
) -> Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(rems(0.75))
        .p(rems(2.))
        .bg(theme::shell(cx))
        .child(
            icon(IconName::TriangleAlert)
                .size(rems(2.))
                .text_color(theme::danger(cx)),
        )
        .child(
            div()
                .text_size(rems(1.25))
                .font_family(theme::font(cx))
                .text_color(theme::text(cx))
                .child(title.into()),
        )
        .child(
            div()
                .max_w(rems(32.))
                .text_center()
                .text_size(rems(0.8125))
                .font_family(theme::font(cx))
                .text_color(theme::secondary(cx))
                .child(cause.into()),
        )
        .child(
            div()
                .flex()
                .gap(rems(0.5))
                .child(
                    button("load-failed-retry", "Réessayer", ButtonVariant::Primary, cx)
                        .on_click(cx.listener(|this, _, _, cx| this.retry(cx))),
                )
                .child(
                    button(
                        "load-failed-settings",
                        "Réglages",
                        ButtonVariant::Outline,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.go(
                            crate::nav::Route::Settings {
                                section: crate::nav::SettingsSection::Playback,
                            },
                            cx,
                        );
                    })),
                ),
        )
}
