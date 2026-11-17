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
//!
//! # One entry point, and a test never observes the person's configuration
//!
//! [`init`] is the only place the app starts gpui-omarchy, because that call
//! also starts the theme *observer*: `Theme::follow_system` watches
//! `$HOME/.config/omarchy` (and its ancestors) with notify/inotify and wakes a
//! GPUI task from that OS thread. A deterministic GPUI test scheduler treats
//! such a wake as non-determinism and fails the test at its end with
//! « Detected activity on thread Some("notify-rs inotify loop") …, but test
//! scheduler is running on … »; a test that watched the real configuration
//! therefore failed whenever anything wrote `~/.config/omarchy/shell.json`
//! beside the theme — the whole `cargo test --workspace` failed about one run
//! in two on the user's machine (SC4's report). What a test paints with is
//! [`init`]'s fallback palette, or one read from a temporary file the test owns
//! ([`init_at`]), and nothing is watched either way.

#[cfg(test)]
use std::path::Path;
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

/// Initialise gpui-omarchy's components and the palette the shell paints with:
/// the one call the app and every GPUI test make.
///
/// `gpui_omarchy`'s init is what registers the widgets AND starts following the
/// person's Omarchy theme (this module's header has the abort that causes in a
/// test); a test stops following at once, and [`init_at`] is how a test gets a
/// palette of its own from a temporary file.
/// `no_other_module_starts_the_omarchy_observer` fails if any other file calls
/// gpui-omarchy's init directly.
pub fn init(cx: &mut App) {
    gpui_omarchy::init(cx);
    #[cfg(test)]
    stop_following(cx);
}

/// A test paints with a fixed palette and observes nothing: `Theme::apply` is
/// gpui-omarchy's own "stop following" (it removes the watcher's global, which
/// drops the task and with it the inotify instance), and it runs here before
/// the deterministic scheduler ever polls that task, so no event from a real
/// thread can reach a test. Tokyo Night is gpui-omarchy's own fallback palette,
/// the one a machine without an Omarchy theme (CI's runner) paints with.
#[cfg(test)]
fn stop_following(cx: &mut App) {
    gpui_omarchy::Theme::tokyo_night().apply(cx);
}

/// A test's palette, read from `root/colors.toml` — a temporary file the test
/// owns — with no observer, so rewriting it can never wake the test scheduler.
/// The real app's palette comes from [`init`] and follows `omarchy-theme-set`
/// live.
#[cfg(test)]
pub fn init_at(cx: &mut App, root: &Path) -> Result<(), String> {
    init(cx);
    apply_from(cx, root)
}

/// Re-read a test's palette: what [`init_at`] does on its own temporary file,
/// and how a test proves a changed file reaches the shell's tokens.
#[cfg(test)]
pub fn apply_from(cx: &mut App, root: &Path) -> Result<(), String> {
    let file = root.join("colors.toml");
    let text =
        std::fs::read_to_string(&file).map_err(|error| format!("{}: {error}", file.display()))?;
    let theme =
        gpui_omarchy::Theme::from_colors_toml("Test", &text).map_err(|error| error.to_string())?;
    theme.apply(cx);
    Ok(())
}

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

    /// **Only this file starts the theme observer.** `gpui_omarchy::init` is
    /// what begins watching `$HOME/.config/omarchy` from notify's inotify
    /// thread, and a deterministic GPUI test scheduler turns that thread's wake
    /// into a failing test — the whole workspace suite failed about one run in
    /// two while the person's `shell.json` was being written. Ten screen modules
    /// had copy-pasted the call, so a guard beats a comment: a new test that
    /// pastes it again fails here instead of failing at random later.
    #[test]
    fn no_other_module_starts_the_omarchy_observer() {
        // Split so that this guard's own text is not one of the hits it counts.
        let observer = ["gpui_omarchy::", "init("].concat();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = 0;
        let mut offenders = Vec::new();
        let mut pending = vec![root.clone()];
        while let Some(dir) = pending.pop() {
            for entry in std::fs::read_dir(&dir).expect("read the crate's sources") {
                let path = entry.expect("a source entry").path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    files += 1;
                    let text = std::fs::read_to_string(&path).expect("read a source file");
                    let hits = text.matches(observer.as_str()).count();
                    if path.ends_with("theme.rs") {
                        // The single legitimate call, in `init` itself.
                        assert_eq!(hits, 1, "{} should start the observer once", path.display());
                    } else if hits > 0 {
                        offenders.push(path);
                    }
                }
            }
        }
        assert!(files > 30, "walked only {files} source files");
        assert!(
            offenders.is_empty(),
            "these files start the Omarchy theme observer instead of calling \
             `crate::theme::init`: {offenders:?}"
        );
    }
}
