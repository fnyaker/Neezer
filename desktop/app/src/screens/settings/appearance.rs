//! Réglages → Apparence: the theme, the motion and the size of the interface.
//!
//! One deliberate difference from the SPA's `AppearanceSettings.svelte`, and it
//! is the whole design of this app: **the palette is Omarchy's**, followed live
//! (`cx.omarchy()`, D7's rule). There is no "Encre / Papier / Pochette" switch
//! and no custom accent to store, because a desktop app that painted its own
//! colours inside an Omarchy session would be the "second design system" rule 5
//! forbids — `omarchy-theme-set` repaints this window without a restart, and
//! the card below shows the tokens it just applied.
//!
//! What the person CAN change here is what belongs to this app: whether things
//! slide (reduced motion, saved as `ui.reduce_motion`, read by the shell's
//! `theme::motion_ms`) and how large the interface is (the rem size `Ctrl+=`,
//! `Ctrl+-` and `Ctrl+0` drive).

use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement, StatefulInteractiveElement as _, Styled,
    Window, div, px, rems,
};
use gpui_omarchy::ActiveTheme;

use super::{Screen, card, card_title, hint, keys, value_line};
use crate::theme;

/// The rem sizes the zoom control moves between: the shell clamps to the same
/// range (`RootView::zoom`), so a person cannot zoom past legibility.
pub const ZOOM_MIN: f32 = 0.75;
pub const ZOOM_MAX: f32 = 1.875;
/// The default, in CSS pixels, exactly as `main`'s window opens.
pub const ZOOM_BASE_PX: f32 = 16.;

pub fn body(screen: &mut Screen, _window: &mut Window, cx: &mut Context<Screen>) -> AnyElement {
    let palette = Palette::read(cx);
    let reduced = screen.state.read(cx).reduced_motion;
    let zoom = screen.state.read(cx).zoom;

    // The theme, as applied right now.
    let mut theme_card = card("settings-theme", cx)
        .child(card_title("Thème", cx))
        .child(hint(
            "Neezer suit le thème d'Omarchy : changez-le depuis le menu Omarchy et cette \
             fenêtre se repeint à chaud, sans redémarrage et sans rien recharger. La palette \
             ci-dessous est celle qui est appliquée en ce moment.",
            cx,
        ))
        .child(value_line("Thème", palette.name.clone(), cx))
        .child(value_line(
            "Fond",
            if palette.light { "Clair" } else { "Sombre" },
            cx,
        ))
        .child(value_line("Police", palette.font.clone(), cx))
        .child(value_line("Police à chasse fixe", palette.mono.clone(), cx));
    let mut swatches = div().flex().items_center().gap(rems(0.375));
    for (label, colour) in [
        ("Fond", palette.background),
        ("Surface", palette.surface),
        ("Accent", palette.accent),
        ("Texte", palette.foreground),
        ("Secondaire", palette.secondary),
        ("Danger", palette.danger),
    ] {
        swatches = swatches.child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(rems(0.1875))
                .child(
                    div()
                        .w(rems(2.25))
                        .h(rems(1.25))
                        .border_1()
                        .border_color(theme::border(cx))
                        .bg(colour),
                )
                .child(
                    div()
                        .text_size(rems(0.625))
                        .text_color(theme::secondary(cx))
                        .child(label),
                ),
        );
    }
    theme_card = theme_card.child(swatches);

    // Motion: the one animation switch that belongs to the app's own shell.
    let motion_card = card("settings-motion", cx)
        .child(card_title("Animations", cx))
        .child(hint(
            "Réduire les animations remplace chaque glissement par un fondu de 90 ms : les \
             écrans, le lecteur plein écran et la file se changent en douceur, sans \
             mouvement. La préférence est enregistrée sur cet appareil.",
            cx,
        ))
        .child(
            gpui_omarchy::switch(
                "settings-reduced-motion",
                "Réduire les animations",
                reduced,
                cx,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                let reduced = !this.state.read(cx).reduced_motion;
                this.state.update(cx, |state, cx| {
                    state.reduced_motion = reduced;
                    cx.notify();
                });
                this.save(keys::REDUCE_MOTION, &reduced, cx);
                this.toast(
                    crate::toast::ToastKind::Info,
                    if reduced {
                        "Animations réduites : fondus de 90 ms"
                    } else {
                        "Animations complètes"
                    },
                    cx,
                );
            })),
        )
        .child(value_line(
            "Durée d'un changement d'écran",
            format!(
                "{} ms",
                theme::motion_ms(theme::SCREEN_MS, reduced).as_millis()
            ),
            cx,
        ));

    // Size: the same zoom the keyboard drives, with the value in front of it.
    let size_card = card("settings-size", cx)
        .child(card_title("Taille de l'interface", cx))
        .child(hint(
            "Agrandit ou réduit tout ce qui est en rem : textes, contrôles, listes. Ctrl+= \
             agrandit, Ctrl+- réduit, Ctrl+0 revient à la taille normale.",
            cx,
        ))
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .child(
                    gpui_omarchy::button(
                        "settings-zoom-out",
                        "Réduire",
                        gpui_omarchy::ButtonVariant::Secondary,
                        cx,
                    )
                    .disabled(zoom <= ZOOM_MIN + f32::EPSILON)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.zoom_by(1. / 1.1, window, cx);
                    })),
                )
                .child(
                    gpui_omarchy::button(
                        "settings-zoom-reset",
                        "Taille normale",
                        gpui_omarchy::ButtonVariant::Outline,
                        cx,
                    )
                    .disabled((zoom - 1.).abs() < f32::EPSILON)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.zoom_to(1., window, cx);
                    })),
                )
                .child(
                    gpui_omarchy::button(
                        "settings-zoom-in",
                        "Agrandir",
                        gpui_omarchy::ButtonVariant::Secondary,
                        cx,
                    )
                    .disabled(zoom >= ZOOM_MAX - f32::EPSILON)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.zoom_by(1.1, window, cx);
                    })),
                )
                .child(
                    div()
                        .text_size(rems(0.75))
                        .font_family(theme::mono(cx))
                        .child(format!("{} %", (zoom * 100.).round())),
                ),
        );

    div()
        .flex()
        .flex_col()
        .gap(rems(1.))
        .child(theme_card)
        .child(motion_card)
        .child(size_card)
        .into_any_element()
}

impl Screen {
    /// One zoom step, exactly the factor the shell's `Ctrl+=` uses.
    fn zoom_by(&mut self, factor: f32, window: &mut Window, cx: &mut Context<Self>) {
        let zoom = (self.state.read(cx).zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
        self.zoom_to(zoom, window, cx);
    }

    fn zoom_to(&mut self, zoom: f32, window: &mut Window, cx: &mut Context<Self>) {
        window.set_rem_size(px(ZOOM_BASE_PX * zoom));
        self.state.update(cx, |state, cx| {
            state.zoom = zoom;
            cx.notify();
        });
        cx.notify();
    }
}

/// The live Omarchy palette, read where it is drawn so a theme change repaints
/// with it.
struct Palette {
    name: gpui_kit::SharedString,
    font: gpui_kit::SharedString,
    mono: gpui_kit::SharedString,
    light: bool,
    background: gpui_kit::Hsla,
    surface: gpui_kit::Hsla,
    accent: gpui_kit::Hsla,
    foreground: gpui_kit::Hsla,
    secondary: gpui_kit::Hsla,
    danger: gpui_kit::Hsla,
}

impl Palette {
    fn read(cx: &gpui_kit::App) -> Self {
        let theme = cx.omarchy();
        Self {
            name: theme.name.clone(),
            font: theme.font.clone(),
            mono: theme.mono_font.clone(),
            // A light palette is one whose background sits above mid-grey; the
            // theme's own `appearance` enum is not re-exported by the crate.
            light: theme.background.l > 0.5,
            background: theme.background,
            surface: theme.surface,
            accent: theme.accent,
            foreground: theme.foreground,
            secondary: theme.secondary,
            danger: theme.danger,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_zoom_bounds_are_the_shells_own() {
        // `RootView::zoom` clamps to 0.75…1.875 on a 16 px base: the settings
        // buttons must not offer a size the keyboard cannot reach.
        assert_eq!(ZOOM_BASE_PX, 16.);
        const { assert!(ZOOM_MIN < 1. && ZOOM_MAX > 1.) };
        let zoomed_in = (1.0_f32 * 1.1).clamp(ZOOM_MIN, ZOOM_MAX);
        assert!((zoomed_in - 1.1).abs() < 1e-6);
        assert_eq!((10.0_f32 * 1.1).clamp(ZOOM_MIN, ZOOM_MAX), ZOOM_MAX);
        assert_eq!((0.1_f32 * 0.9).clamp(ZOOM_MIN, ZOOM_MAX), ZOOM_MIN);
    }

    #[test]
    fn reduced_motion_shortens_every_slide_to_the_same_fade() {
        // The shell's `theme::motion_ms` is what the card prints; a mismatch
        // between the sentence and the behaviour would be the bug.
        assert_eq!(
            theme::motion_ms(theme::SCREEN_MS, true).as_millis(),
            u128::from(theme::REDUCED_MS)
        );
        assert_eq!(
            theme::motion_ms(theme::SCREEN_MS, false).as_millis(),
            u128::from(theme::SCREEN_MS)
        );
    }
}
