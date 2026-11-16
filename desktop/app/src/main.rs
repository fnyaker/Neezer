//! `neezer-desktop`: the app's window.
//!
//! D0's whole job here is to prove the stack: an Omarchy window titled
//! "Neezer", themed by the live Omarchy palette, that quits on `Ctrl+Q` — and,
//! for CI and the acceptance numbers, `--self-test-quit-after-first-frame`,
//! which prints `first_frame_ms=<n>` once the first frame is on screen and
//! exits. D7 replaces the empty body with the real shell; the flag and the
//! quit path stay.

use std::process::ExitCode;
use std::time::Instant;

use gpui_kit::{
    App, AppContext as _, Bounds, Context, InteractiveElement as _, IntoElement, KeyBinding,
    Render, Styled, Window, WindowBounds, WindowOptions, actions, div, px, size,
};
use gpui_omarchy::ActiveTheme;

actions!(neezer, [Quit]);

const USAGE: &str = "\
neezer-desktop — le lecteur Neezer pour Omarchy.

USAGE:
    neezer-desktop
    neezer-desktop --self-test-quit-after-first-frame

OPTIONS:
    --self-test-quit-after-first-frame
        Ouvre la fenêtre, affiche `first_frame_ms=<n>` après la première image
        peinte, puis quitte. Utilisé par la CI (sous Xvfb) et par les mesures.
    -h, --help
        Affiche cette aide.
";

/// What the command line asked for. `--help` and the self test are the only two
/// things D0 understands; D7 adds nothing to the parser (the app has no other
/// option: everything is in Réglages).
#[derive(Debug, Default, PartialEq, Eq)]
struct Args {
    self_test_quit_after_first_frame: bool,
    help: bool,
}

impl Args {
    fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Self, String> {
        let mut parsed = Self::default();
        for arg in args {
            match arg.as_str() {
                "--self-test-quit-after-first-frame" => {
                    parsed.self_test_quit_after_first_frame = true
                }
                "-h" | "--help" => parsed.help = true,
                other => return Err(format!("argument inconnu : {other}")),
            }
        }
        Ok(parsed)
    }
}

/// The window's only view: D0 has no screens yet, so it paints the Omarchy
/// background under the "Neezer" key context D7's scoped keymap will hang off.
struct Shell;

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(cx.omarchy().background)
            .key_context("Neezer")
    }
}

fn window_options(cx: &App) -> WindowOptions {
    let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(gpui_kit::TitlebarOptions {
            title: Some("Neezer".into()),
            ..Default::default()
        }),
        // Wayland's app id and X11's WM_CLASS: what Hyprland's rules, the
        // launcher and packaging's StartupWMClass look for.
        app_id: Some("neezer-desktop".into()),
        ..Default::default()
    }
}

fn main() -> ExitCode {
    let started = Instant::now();
    let args = match Args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("neezer-desktop : {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    if args.help {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    // Same order as gpui-omarchy's own hello example: assets first (icons are
    // read through the asset source), then the theme, then the window.
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_omarchy::init(cx);
            // A *global* binding: an empty window has nothing focused, so a
            // binding scoped to a key context would never be looked up. The
            // handler is a global action listener for the same reason, and
            // because quitting belongs to the app, not to a screen (D7 moves
            // the rest of the keymap into `keymap.rs`).
            cx.bind_keys([KeyBinding::new("ctrl-q", Quit, None)]);
            cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

            cx.open_window(window_options(cx), move |window, cx| {
                if args.self_test_quit_after_first_frame {
                    // `on_next_frame` runs directly after the frame currently
                    // being drawn is handed to the compositor: that is the
                    // number D7's "< 300 ms to first painted frame" is about.
                    window.on_next_frame(move |_, cx| {
                        println!("first_frame_ms={}", started.elapsed().as_millis());
                        cx.quit();
                    });
                }
                cx.new(|_| Shell)
            })
            .expect("ouvrir la fenêtre");
            cx.activate(true);
        });

    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::Args;

    #[test]
    fn the_self_test_flag_is_recognised_and_unknown_ones_are_refused() {
        let args = Args::parse(["--self-test-quit-after-first-frame".to_owned()]).expect("parse");
        assert!(args.self_test_quit_after_first_frame);
        assert!(!args.help);

        assert_eq!(
            Args::parse(Vec::new()).expect("no arguments"),
            Args::default()
        );
        assert!(Args::parse(["--nope".to_owned()]).is_err());
    }
}
