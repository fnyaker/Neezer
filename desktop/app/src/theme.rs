//! The shell's design tokens.
//!
//! Every colour comes from the live Omarchy theme (`cx.omarchy()`), never a
//! hard-coded hex: `omarchy-theme-set` rewrites `colors.toml`, gpui-omarchy's
//! watcher applies the new palette and refreshes the windows, and the shell
//! repaints with it — that is the whole "follows a theme change live" criterion
//! (`docs/agents/desktop.md`, D7 *Acceptance*). This module only names the few
//! combinations the shell uses, so a screen never reaches for `foreground`
//! where the design wants `secondary`.
//!
//! The durations are the Omarchy shell's own (`gpui-omarchy/motion.rs`) plus
//! the three a player needs; the reduced-motion setting collapses the slides
//! to a 90 ms fade without changing any call site.

use std::time::Duration;

use gpui_kit::{App, Hsla, Pixels, Window, px, rems};
use gpui_omarchy::ActiveTheme;

/// Colour of a control on hover (Omarchy's `CONTROL_COLOR`).
pub const CONTROL_COLOR_MS: u64 = 120;
/// A pointer's own glide.
pub const CURSOR_MS: u64 = 60;
/// A popup's fade.
pub const POPUP_MS: u64 = 140;
/// A screen push/pop: slide 24 px + fade.
pub const SCREEN_MS: u64 = 180;
/// Now-playing expand/collapse.
pub const EXPAND_MS: u64 = 240;
/// A queue row's reorder.
pub const REORDER_MS: u64 = 160;
/// What reduced motion leaves of every slide.
pub const REDUCED_MS: u64 = 90;

/// The shell's surfaces.
pub fn shell(app: &App) -> Hsla {
    app.omarchy().background
}

/// The sidebar: one step off the content, never a second palette.
pub fn sidebar(app: &App) -> Hsla {
    app.omarchy().inset
}

/// The header and the player bar sit on `surface`, the content on `background`,
/// so the eye reads a frame around the page.
pub fn chrome(app: &App) -> Hsla {
    app.omarchy().surface
}

pub fn text(app: &App) -> Hsla {
    app.omarchy().foreground
}

pub fn secondary(app: &App) -> Hsla {
    app.omarchy().secondary
}

pub fn accent(app: &App) -> Hsla {
    app.omarchy().accent
}

pub fn border(app: &App) -> Hsla {
    app.omarchy().border
}

pub fn selected(app: &App) -> Hsla {
    app.omarchy().selection
}

pub fn danger(app: &App) -> Hsla {
    app.omarchy().danger
}

pub fn warning(app: &App) -> Hsla {
    app.omarchy().warning
}

pub fn success(app: &App) -> Hsla {
    app.omarchy().success
}

/// The one place a focus ring is drawn: gpui-omarchy's `focus_border`, 1 px.
pub fn focus_ring(app: &App) -> Hsla {
    app.omarchy().focus_border()
}

/// The app's font (the Omarchy font setting), at a named scale.
pub fn font(app: &App) -> gpui_kit::SharedString {
    app.omarchy().font.clone()
}

pub fn mono(app: &App) -> gpui_kit::SharedString {
    app.omarchy().mono_font.clone()
}

/// Header height: one 48 px row at the default 16 px rem.
pub fn header_height(window: &Window) -> Pixels {
    rems(3.).to_pixels(window.rem_size())
}

/// Player bar height.
pub fn player_height(window: &Window) -> Pixels {
    rems(4.5).to_pixels(window.rem_size())
}

/// Sidebar width (labels are hidden below the compact breakpoint).
pub fn sidebar_width(window: &Window) -> Pixels {
    rems(12.5).to_pixels(window.rem_size())
}

/// The queue/side panel's width: fixed at 420 px, the width the plan's visual
/// acceptance is measured at.
pub const SIDE_PANEL: Pixels = px(420.);

/// The duration a slide should use, honouring the reduced-motion setting.
pub fn motion_ms(full_ms: u64, reduced: bool) -> Duration {
    Duration::from_millis(if reduced { REDUCED_MS } else { full_ms })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduced_motion_collapses_every_slide_to_the_same_fade() {
        assert_eq!(motion_ms(SCREEN_MS, false), Duration::from_millis(180));
        assert_eq!(motion_ms(EXPAND_MS, true), Duration::from_millis(90));
        assert_eq!(motion_ms(REORDER_MS, false), Duration::from_millis(160));
    }
}
