//! The network chip.
//!
//! `Api` publishes reachability on a `watch` channel: every answer proves the
//! server is there, a connection failure proves it is not, and the app's own
//! probe is what flips it back (`docs/agents/desktop.md`, *Network errors*).
//! The chip is deliberately quiet while the link is up — a permanent green dot
//! is decoration — and names the state in French when it is not.
//!
//! Clicking the chip probes once; that is the same request the `Retry` action
//! makes, so a mouse and a keyboard retry are one path.

use gpui_kit::{Context, InteractiveElement, ParentElement, Styled, div, rems};
use gpui_omarchy::{ButtonVariant, IconName, button, icon};

use crate::app::RootView;
use crate::theme;

/// `probing` is a request in flight; `online` is the last verdict.
pub fn render(
    online: bool,
    probing: bool,
    cx: &mut Context<RootView>,
) -> impl gpui_kit::IntoElement + use<> {
    let mut chip = div()
        .id("net-indicator")
        .flex()
        .items_center()
        .gap(rems(0.375))
        .px(rems(0.5))
        .py(rems(0.1875))
        .border_1()
        .border_color(if online {
            theme::border(cx)
        } else {
            theme::danger(cx)
        })
        .text_size(rems(0.6875))
        .font_family(theme::font(cx))
        .text_color(if online {
            theme::secondary(cx)
        } else {
            theme::danger(cx)
        })
        .child(
            icon(if online {
                IconName::Wifi
            } else {
                IconName::WifiOff
            })
            .size(rems(0.875)),
        );
    if probing {
        chip = chip.child("Connexion…");
    } else if online {
        chip = chip.child("En ligne");
    } else {
        chip = chip.child("Hors ligne").child(
            button(
                "net-indicator-retry",
                "Réessayer",
                ButtonVariant::Outline,
                cx,
            )
            .on_click(cx.listener(|this, _, _, cx| this.probe(cx))),
        );
    }
    chip
}
