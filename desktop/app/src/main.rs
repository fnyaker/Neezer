#![allow(dead_code)]
//! `neezer-desktop`: the app's window.
//!
//! The shell is [`crate::app::RootView`]; this file owns the process: the
//! command line, the two-thread tokio runtime every HTTP request runs on, the
//! single-instance name, the Omarchy theme (gpui-omarchy follows
//! `omarchy-theme-set` live) and the window. It deliberately does very little
//! synchronously: a cold start's first painted frame is the acceptance number,
//! so nothing here waits for the network, PipeWire, D-Bus or a session.
//!
//! `--self-test-quit-after-first-frame` prints `first_frame_ms=<n>` after the
//! first frame reached the compositor and exits; CI uses it under Xvfb and the
//! acceptance measurement uses it in a release build.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::{AppContext as _, Bounds, KeyBinding, WindowBounds, WindowOptions, px, size};

// The shell is the contract every later screen lot fills: route variants,
// theme tokens and helpers are declared now and called when the screen that
// needs them lands (docs/agents/desktop.md, D7). Dead code here is the plan's
// own "no lot edits mod.rs after D7", not an oversight — clippy still holds
// every other lint to `-D warnings`.
mod actions;
mod app;
mod keymap;
mod load_failed;
mod login;
mod nav;
mod net_indicator;
mod player_bar;
mod screens;
mod state;
mod theme;
mod toast;
mod widgets;
mod wiring;

use actions::Quit;
use app::RootView;
use state::{AppConfig, AppState, event_channel};
use wiring::events;

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

/// What the command line asked for. The app has no other option: everything is
/// in Réglages.
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

fn window_options(cx: &gpui_kit::App) -> WindowOptions {
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

/// The HTTP runtime, kept alive in the app's globals. `Application::run`'s
/// launch closure may be dropped once it has run, and a runtime dropped with it
/// stops every worker: a request spawned a second later would never be polled
/// (the symptom was a login that stayed « Connexion… » forever). A `Global`
/// lives exactly as long as the `App`.
struct NetRuntime(Arc<tokio::runtime::Runtime>);

impl gpui_kit::Global for NetRuntime {}

// The icon catalogue the shell draws. `gpui_kit::assets::Assets` is the
// DEFAULT bundle only — registering it makes every other Lucide glyph render
// nothing at all, silently (the transport lost its previous/next/volume icons,
// and the sidebar its library and sparkle). The full `AllAssets` catalogue fixes
// that and costs ~18 MB in the binary, so the shell borrows exactly the icons it
// uses plus the ones gpui-base's own controls require. `the_icon_bundle_has_every_icon_the_shell_draws`
// holds the two lists together.
gpui_kit::assets::icon_assets!(
    ShellAssets,
    [
        // The shell.
        House,
        Search,
        LibraryBig,
        Mic,
        Sparkles,
        Disc3,
        Music,
        Settings,
        Keyboard,
        ChevronLeft,
        ChevronRight,
        ChevronDown,
        LogOut,
        Construction,
        AudioLines,
        Wifi,
        WifiOff,
        CircleAlert,
        TriangleAlert,
        ShieldAlert,
        Info,
        Close,
        // The transport.
        SkipBack,
        SkipForward,
        Play,
        Pause,
        Shuffle,
        Repeat,
        Repeat1,
        Volume1,
        Volume2,
        VolumeX,
        ListMusic,
        Maximize2,
        // gpui-kit's own controls (checked, menus, empty states, calendars).
        Check,
        Minus,
        Plus,
        Calendar,
        Star,
        ExternalLink,
        Menu,
        LoaderCircle,
        CircleCheck,
        TriangleAlert,
    ]
);

/// Every glyph the shell names. The list above is a macro argument, so the two
/// cannot share a constant; the test in this file is what keeps them in step.
const SHELL_ICONS: &[gpui_kit::assets::IconName] = &[
    gpui_kit::assets::IconName::House,
    gpui_kit::assets::IconName::Search,
    gpui_kit::assets::IconName::LibraryBig,
    gpui_kit::assets::IconName::Mic,
    gpui_kit::assets::IconName::Sparkles,
    gpui_kit::assets::IconName::Disc3,
    gpui_kit::assets::IconName::Music,
    gpui_kit::assets::IconName::Settings,
    gpui_kit::assets::IconName::Keyboard,
    gpui_kit::assets::IconName::ChevronLeft,
    gpui_kit::assets::IconName::ChevronRight,
    gpui_kit::assets::IconName::ChevronDown,
    gpui_kit::assets::IconName::LogOut,
    gpui_kit::assets::IconName::Construction,
    gpui_kit::assets::IconName::AudioLines,
    gpui_kit::assets::IconName::Wifi,
    gpui_kit::assets::IconName::WifiOff,
    gpui_kit::assets::IconName::CircleAlert,
    gpui_kit::assets::IconName::TriangleAlert,
    gpui_kit::assets::IconName::ShieldAlert,
    gpui_kit::assets::IconName::Info,
    gpui_kit::assets::IconName::Close,
    gpui_kit::assets::IconName::SkipBack,
    gpui_kit::assets::IconName::SkipForward,
    gpui_kit::assets::IconName::Play,
    gpui_kit::assets::IconName::Pause,
    gpui_kit::assets::IconName::Shuffle,
    gpui_kit::assets::IconName::Repeat,
    gpui_kit::assets::IconName::Repeat1,
    gpui_kit::assets::IconName::Volume1,
    gpui_kit::assets::IconName::Volume2,
    gpui_kit::assets::IconName::VolumeX,
    gpui_kit::assets::IconName::ListMusic,
    gpui_kit::assets::IconName::Maximize2,
    gpui_kit::assets::IconName::Check,
    gpui_kit::assets::IconName::Minus,
    gpui_kit::assets::IconName::Plus,
    gpui_kit::assets::IconName::Calendar,
    gpui_kit::assets::IconName::Star,
    gpui_kit::assets::IconName::ExternalLink,
    gpui_kit::assets::IconName::Menu,
    gpui_kit::assets::IconName::LoaderCircle,
    gpui_kit::assets::IconName::CircleCheck,
];

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

    // Two workers: rule 1's lanes decide order, not more threads competing for
    // the cores. The runtime lives as long as the process, so a request a
    // screen started can always finish.
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("nz-net")
        .enable_all()
        .build()
    {
        Ok(runtime) => Arc::new(runtime),
        Err(error) => {
            eprintln!("neezer-desktop : le runtime réseau n'a pas pu démarrer : {error}");
            return ExitCode::from(1);
        }
    };
    let (events_tx, events_rx) = event_channel();

    // One instance per session, claimed in the background: a slow or absent
    // session bus must not delay the first frame (`CLAIM_BUDGET` is measured
    // after the window is up). A second launch asks the running app to raise
    // its window and quits as soon as the answer is in.
    let claim_events = events_tx.clone();
    runtime.spawn(async move {
        let claim = tokio::time::timeout(
            wiring::instance::CLAIM_BUDGET,
            wiring::instance::claim(claim_events.clone()),
        )
        .await;
        match claim {
            Ok(claim) => claim_events.send(state::AppEvent::Instance(claim)),
            Err(_) => {
                tracing::debug!("pas d'instance unique : le bus n'a pas répondu dans le budget");
            }
        }
    });

    let config = AppConfig {
        server: std::env::var("NEEZER_SERVER")
            .ok()
            .filter(|value| !value.trim().is_empty()),
        ..AppConfig::default()
    };
    let self_test = args.self_test_quit_after_first_frame;
    let runtime_handle = runtime.handle().clone();

    gpui_kit::application()
        .with_assets(ShellAssets)
        .run(move |cx| {
            cx.set_global(NetRuntime(Arc::clone(&runtime)));
            gpui_omarchy::init(cx);
            keymap::install(cx);

            // Quitting is the one action that is not the shell's: it must work
            // before a view exists, and from the login screen too.
            cx.bind_keys([KeyBinding::new("ctrl-q", Quit, None)]);
            cx.on_action(|_: &Quit, cx: &mut gpui_kit::App| cx.quit());

            let events_rx: events::Receiver<state::AppEvent> = events_rx;
            let state_config = config.clone();
            let tx = events_tx.clone();
            let rt = runtime_handle.clone();
            let window = cx
                .open_window(window_options(cx), move |window, cx| {
                    if self_test {
                        // `on_next_frame` runs just after the frame currently
                        // being drawn is handed to the compositor: the number
                        // the acceptance measures.
                        window.on_next_frame(move |_, cx| {
                            println!("first_frame_ms={}", started.elapsed().as_millis());
                            cx.quit();
                        });
                    }
                    let state = cx.new(move |_| AppState::new(state_config, rt, tx, None));
                    cx.new(|cx| RootView::new(state, events_rx, window, cx))
                })
                .expect("ouvrir la fenêtre");
            let _ = window;
            cx.activate(true);
        });

    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::Args;

    /// Every icon the shell draws is in the bundle it registers. The default
    /// `Assets` bundle only carries gpui-kit's component icons, so a glyph
    /// outside it renders NOTHING and the button keeps its hit box: measured on
    /// the first real run, the transport showed only Play and the sidebar lost
    /// Library and Nouveautés.
    #[test]
    fn the_icon_bundle_has_every_icon_the_shell_draws() {
        use gpui_kit::AssetSource as _;
        for icon in crate::SHELL_ICONS {
            let path = icon.path();
            let bytes = super::ShellAssets.load(&path).expect("read the bundle");
            assert!(
                bytes.is_some_and(|bytes| !bytes.is_empty()),
                "{path} is missing"
            );
        }
    }

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
