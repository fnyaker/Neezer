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
mod bench;
mod keymap;
mod load_failed;
mod login;
mod nav;
mod net_indicator;
mod player_bar;
mod screens;
mod state;
#[cfg(test)]
mod testmem;
#[cfg(test)]
mod testwait;
mod theme;
mod toast;
mod widgets;
mod wiring;

// The tests that measure a screen's weight count BYTES on their own thread
// (`testmem`), never the process's RSS: the suite runs its tests in parallel in
// one process, so `VmRSS` moves with the neighbours and the allocator. The
// shipped binary links the system allocator untouched.
#[cfg(test)]
#[global_allocator]
static ALLOCATOR: testmem::Counting = testmem::Counting;

use actions::Quit;
use app::RootView;
use state::{AppConfig, AppState, event_channel};
use wiring::events;

const USAGE: &str = "\
neezer-desktop — le lecteur Neezer pour Omarchy.

USAGE:
    neezer-desktop
    neezer-desktop --self-test-quit-after-first-frame
    neezer-desktop --window-size 2560x1600 [--route ÉCRAN] [--frame-log F]
                   [--bench-events F] [--bench-status F]

OPTIONS:
    --self-test-quit-after-first-frame
        Ouvre la fenêtre, affiche `first_frame_ms=<n>` après la première image
        peinte, puis quitte. Utilisé par la CI (sous Xvfb) et par les mesures.
    --window-size LARGEURxHAUTEUR
        La taille de la fenêtre au lancement (par défaut 1280x800). Toutes les
        mesures de mémoire de docs/agents/desktop.md sont données avec la leur :
        redimensionner une fenêtre vivante court après la taille initiale de
        GPUI, la choisir au lancement est la seule mesure fiable.
    --frame-log F
        Écrit une ligne par image peinte (temps écoulé, durée de construction),
        lue par desktop/tools/journey/frame_stats.py.
    --route ÉCRAN
        Ouvre cet écran plutôt que l'accueil, une fois la session restaurée :
        `home`, `library:favorites`, `artist:<id>`, `playlist:<id>`,
        `settings:serveur`… (la forme exacte est `Route::spec`). Utilisé par le
        banc de mesure, qui photographie et compte chaque écran sans cliquer.
    --bench-events F
        Écrit les événements mesurés (chargement d'un titre, premier échantillon,
        sous-alimentation, flux complet) en JSON, une ligne chacun.
    --bench-status F
        Écrit un état JSON toutes les 10 s (sous-alimentations, position, file,
        RSS) : ce qu'une heure de lecture laisse derrière elle.
    -h, --help
        Affiche cette aide.
";

/// What the command line asked for. The app has no other option: everything is
/// in Réglages.
#[derive(Debug, Default, PartialEq)]
struct Args {
    self_test_quit_after_first_frame: bool,
    window_size: Option<(f32, f32)>,
    frame_log: Option<std::path::PathBuf>,
    bench_events: Option<std::path::PathBuf>,
    bench_status: Option<std::path::PathBuf>,
    start_route: Option<String>,
    help: bool,
}

impl Args {
    fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Self, String> {
        let mut parsed = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--self-test-quit-after-first-frame" => {
                    parsed.self_test_quit_after_first_frame = true
                }
                "--window-size" => {
                    let value = args.next().ok_or("--window-size attend WxH")?;
                    parsed.window_size = Some(
                        bench::parse_size(&value)
                            .ok_or_else(|| format!("taille invalide : {value}"))?,
                    );
                }
                "--frame-log" => {
                    parsed.frame_log =
                        Some(args.next().ok_or("--frame-log attend un fichier")?.into())
                }
                "--bench-events" => {
                    parsed.bench_events = Some(
                        args.next()
                            .ok_or("--bench-events attend un fichier")?
                            .into(),
                    )
                }
                "--bench-status" => {
                    parsed.bench_status = Some(
                        args.next()
                            .ok_or("--bench-status attend un fichier")?
                            .into(),
                    )
                }
                "--route" => {
                    parsed.start_route = Some(args.next().ok_or("--route attend un écran")?)
                }
                "-h" | "--help" => parsed.help = true,
                other => return Err(format!("argument inconnu : {other}")),
            }
        }
        Ok(parsed)
    }
}

/// The window's app id: `neezer-desktop` on a normal install (the `StartupWMClass`
/// of `desktop/packaging/arch/neezer-desktop.desktop`), and the sandbox's own app
/// id when Flatpak runs it — the export names the launcher entry
/// `<app-id>.desktop`, and the compositor matches the window by that string.
fn flatpak_app_id() -> String {
    std::env::var("FLATPAK_ID").unwrap_or_else(|_| "neezer-desktop".to_owned())
}

fn window_options(cx: &gpui_kit::App, chosen: Option<(f32, f32)>) -> WindowOptions {
    let (width, height) = chosen.unwrap_or((1280., 800.));
    let bounds = Bounds::centered(None, size(px(width), px(height)), cx);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(gpui_kit::TitlebarOptions {
            title: Some("Neezer".into()),
            ..Default::default()
        }),
        // Wayland's app id and X11's WM_CLASS: what Hyprland's rules, the
        // launcher and packaging's StartupWMClass look for.
        //
        // Inside a sandbox it must be the sandbox's app id instead, or the
        // compositor has nothing to match the window against: Flatpak exports a
        // launcher entry named `<app-id>.desktop`, and the release bundle's app
        // id is `fr.neezer.Desktop` (`desktop/packaging/flatpak/`). The same
        // value is what that package's `StartupWMClass` names.
        app_id: Some(flatpak_app_id()),
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

impl Drop for NetRuntime {
    /// The threads that borrow this runtime are joined HERE, before the
    /// `Arc<Runtime>` field above is dropped (Rust drops the fields after this
    /// body runs). Two of them do: `nz-bulk` (the download worker) and every
    /// thread the audio engine started — a loader, a decode thread, a spooler.
    ///
    /// Both drive the network with `Handle::block_on`, and tokio panics — « A
    /// Tokio 1.x context was found, but it is being shutdown. » — when a timer
    /// is polled on a runtime that is shutting down. Closing the window during a
    /// download did exactly that (measured in CI, run 43964917017); closing it
    /// during a track, or while the next one preloaded, was the same bug with
    /// the engine's threads.
    fn drop(&mut self) {
        crate::wiring::offline::shutdown_process();
        crate::wiring::engine::shutdown_process();
        // The LAN node goes AFTER both: `nz-bulk` and the engine's threads drive
        // `nz-net`, while `nz-lan` owns its own runtime whose tasks are the
        // peers' requests (and `nz-lan`'s adapters run on those two). Joining it
        // last means nothing of the node is left inside a runtime that is about
        // to stop, and the reverse order would leave a peer's fetch holding a
        // thread the engine already abandoned. Measured: 0 `nz-lan` and 0
        // `nz-lan-hash` threads when this returns (`wiring::lan`'s bench).
        crate::wiring::lan::shutdown_process();
        // The transcription supervisor is a child process: stopped and joined
        // here so no whisper process outlives the window (T4).
        if let Some(service) = crate::wiring::transcribe::started() {
            service.shutdown();
        }
    }
}

// The icon catalogue the app draws. `gpui_kit::assets::Assets` is the DEFAULT
// bundle only — registering it makes every other Lucide glyph render nothing at
// all, silently (the transport lost its previous/next/volume icons, and the
// sidebar its library and sparkle). The full `AllAssets` catalogue fixes that
// and costs ~18 MB in the binary, so the app borrows the glyphs its screens
// name, listed here once.
//
// This list is the SUPERSET every remaining screen lot draws, mapped from the
// web app's own icon names (`webapp/src/components/Icon.svelte`, 92 of them) to
// Lucide, plus what the lots still to land describe in
// `docs/agents/desktop.md` (browse, now playing, visualiser, podcasts,
// discovery, offline, Réglages, remote, party, genre studio, lists). A lot ADDS
// a glyph here when it needs one; it never edits `keymap.rs` or a `mod.rs`.
//
// The `every_icon_name_used_in_the_app_is_in_the_bundle` test below is the
// other half of this list: it walks `app/src` for every `IconName::<Nom>` and fails
// when one is not registered, because a missing glyph renders NOTHING and keeps
// its hit box — a button that is there and invisible.
gpui_kit::assets::icon_assets!(
    ShellAssets,
    [
        Activity,
        Archive,
        ArrowDown,
        ArrowLeftRight,
        ArrowUp,
        ArrowUpDown,
        AudioLines,
        AudioWaveform,
        BadgeCheck,
        Bluetooth,
        Bookmark,
        BookmarkPlus,
        Calendar,
        Cast,
        Check,
        ChevronDown,
        ChevronFirst,
        ChevronLast,
        ChevronLeft,
        ChevronRight,
        ChevronUp,
        CircleAlert,
        CircleCheck,
        CircleCheckBig,
        CircleMinus,
        CirclePause,
        CirclePlay,
        Close,
        Cloud,
        CloudDownload,
        CloudOff,
        CloudUpload,
        Compass,
        Construction,
        Copy,
        Cpu,
        Database,
        Disc3,
        Download,
        Ellipsis,
        EllipsisVertical,
        ExternalLink,
        Eye,
        Folder,
        FolderInput,
        FolderMinus,
        FolderOpen,
        FolderPlus,
        FolderTree,
        Gauge,
        GitBranch,
        Globe,
        GripVertical,
        HardDrive,
        Hash,
        Headphones,
        Heart,
        House,
        Infinity,
        Info,
        KeyRound,
        Keyboard,
        KeyboardMusic,
        Languages,
        Laptop,
        Leaf,
        Library,
        LibraryBig,
        Link,
        Link2,
        List,
        ListChecks,
        ListMusic,
        ListPlus,
        LoaderCircle,
        Lock,
        LogOut,
        Maximize2,
        Menu,
        Mic,
        MicOff,
        MicVocal,
        Minimize2,
        Minus,
        Monitor,
        Moon,
        Music,
        Music2,
        PartyPopper,
        Pause,
        Pencil,
        Pin,
        Play,
        Plus,
        Power,
        Rabbit,
        Radio,
        RadioTower,
        Radar,
        RefreshCw,
        Repeat,
        Repeat1,
        Replace,
        RotateCcw,
        RotateCw,
        Save,
        ScanLine,
        Scissors,
        ScrollText,
        Search,
        Server,
        Settings,
        Share2,
        Shield,
        ShieldAlert,
        ShieldCheck,
        Shuffle,
        SkipBack,
        SkipForward,
        SlidersHorizontal,
        SlidersVertical,
        Smartphone,
        Sparkles,
        Speaker,
        SquareCheckBig,
        SquarePen,
        Star,
        Sun,
        Tag,
        Tags,
        ThumbsDown,
        ThumbsUp,
        Timer,
        Trash,
        TriangleAlert,
        Type,
        Upload,
        User,
        UserPlus,
        Users,
        Video,
        Volume1,
        Volume2,
        VolumeX,
        Wifi,
        WifiOff,
        Zap,
        ZoomIn,
        ZoomOut
    ]
);

/// Every glyph the app names, for the test above (the macro's list is an
/// argument, so the two cannot share a constant).
const SHELL_ICONS: &[gpui_kit::assets::IconName] = &[
    gpui_kit::assets::IconName::Activity,
    gpui_kit::assets::IconName::Archive,
    gpui_kit::assets::IconName::ArrowDown,
    gpui_kit::assets::IconName::ArrowLeftRight,
    gpui_kit::assets::IconName::ArrowUp,
    gpui_kit::assets::IconName::ArrowUpDown,
    gpui_kit::assets::IconName::AudioLines,
    gpui_kit::assets::IconName::AudioWaveform,
    gpui_kit::assets::IconName::BadgeCheck,
    gpui_kit::assets::IconName::Bluetooth,
    gpui_kit::assets::IconName::Bookmark,
    gpui_kit::assets::IconName::BookmarkPlus,
    gpui_kit::assets::IconName::Calendar,
    gpui_kit::assets::IconName::Cast,
    gpui_kit::assets::IconName::Check,
    gpui_kit::assets::IconName::ChevronDown,
    gpui_kit::assets::IconName::ChevronFirst,
    gpui_kit::assets::IconName::ChevronLast,
    gpui_kit::assets::IconName::ChevronLeft,
    gpui_kit::assets::IconName::ChevronRight,
    gpui_kit::assets::IconName::ChevronUp,
    gpui_kit::assets::IconName::CircleAlert,
    gpui_kit::assets::IconName::CircleCheck,
    gpui_kit::assets::IconName::CircleCheckBig,
    gpui_kit::assets::IconName::CircleMinus,
    gpui_kit::assets::IconName::CirclePause,
    gpui_kit::assets::IconName::CirclePlay,
    gpui_kit::assets::IconName::Close,
    gpui_kit::assets::IconName::Cloud,
    gpui_kit::assets::IconName::CloudDownload,
    gpui_kit::assets::IconName::CloudOff,
    gpui_kit::assets::IconName::CloudUpload,
    gpui_kit::assets::IconName::Compass,
    gpui_kit::assets::IconName::Construction,
    gpui_kit::assets::IconName::Copy,
    gpui_kit::assets::IconName::Cpu,
    gpui_kit::assets::IconName::Database,
    gpui_kit::assets::IconName::Disc3,
    gpui_kit::assets::IconName::Download,
    gpui_kit::assets::IconName::Ellipsis,
    gpui_kit::assets::IconName::EllipsisVertical,
    gpui_kit::assets::IconName::ExternalLink,
    gpui_kit::assets::IconName::Eye,
    gpui_kit::assets::IconName::Folder,
    gpui_kit::assets::IconName::FolderInput,
    gpui_kit::assets::IconName::FolderMinus,
    gpui_kit::assets::IconName::FolderOpen,
    gpui_kit::assets::IconName::FolderPlus,
    gpui_kit::assets::IconName::FolderTree,
    gpui_kit::assets::IconName::Gauge,
    gpui_kit::assets::IconName::GitBranch,
    gpui_kit::assets::IconName::Globe,
    gpui_kit::assets::IconName::GripVertical,
    gpui_kit::assets::IconName::HardDrive,
    gpui_kit::assets::IconName::Hash,
    gpui_kit::assets::IconName::Headphones,
    gpui_kit::assets::IconName::Heart,
    gpui_kit::assets::IconName::House,
    gpui_kit::assets::IconName::Infinity,
    gpui_kit::assets::IconName::Info,
    gpui_kit::assets::IconName::KeyRound,
    gpui_kit::assets::IconName::Keyboard,
    gpui_kit::assets::IconName::KeyboardMusic,
    gpui_kit::assets::IconName::Languages,
    gpui_kit::assets::IconName::Laptop,
    gpui_kit::assets::IconName::Leaf,
    gpui_kit::assets::IconName::Library,
    gpui_kit::assets::IconName::LibraryBig,
    gpui_kit::assets::IconName::Link,
    gpui_kit::assets::IconName::Link2,
    gpui_kit::assets::IconName::List,
    gpui_kit::assets::IconName::ListChecks,
    gpui_kit::assets::IconName::ListMusic,
    gpui_kit::assets::IconName::ListPlus,
    gpui_kit::assets::IconName::LoaderCircle,
    gpui_kit::assets::IconName::Lock,
    gpui_kit::assets::IconName::LogOut,
    gpui_kit::assets::IconName::Maximize2,
    gpui_kit::assets::IconName::Menu,
    gpui_kit::assets::IconName::Mic,
    gpui_kit::assets::IconName::MicOff,
    gpui_kit::assets::IconName::MicVocal,
    gpui_kit::assets::IconName::Minimize2,
    gpui_kit::assets::IconName::Minus,
    gpui_kit::assets::IconName::Monitor,
    gpui_kit::assets::IconName::Moon,
    gpui_kit::assets::IconName::Music,
    gpui_kit::assets::IconName::Music2,
    gpui_kit::assets::IconName::PartyPopper,
    gpui_kit::assets::IconName::Pause,
    gpui_kit::assets::IconName::Pencil,
    gpui_kit::assets::IconName::Pin,
    gpui_kit::assets::IconName::Play,
    gpui_kit::assets::IconName::Plus,
    gpui_kit::assets::IconName::Power,
    gpui_kit::assets::IconName::Rabbit,
    gpui_kit::assets::IconName::Radio,
    gpui_kit::assets::IconName::RadioTower,
    gpui_kit::assets::IconName::Radar,
    gpui_kit::assets::IconName::RefreshCw,
    gpui_kit::assets::IconName::Repeat,
    gpui_kit::assets::IconName::Repeat1,
    gpui_kit::assets::IconName::Replace,
    gpui_kit::assets::IconName::RotateCcw,
    gpui_kit::assets::IconName::RotateCw,
    gpui_kit::assets::IconName::Save,
    gpui_kit::assets::IconName::ScanLine,
    gpui_kit::assets::IconName::Scissors,
    gpui_kit::assets::IconName::ScrollText,
    gpui_kit::assets::IconName::Search,
    gpui_kit::assets::IconName::Server,
    gpui_kit::assets::IconName::Settings,
    gpui_kit::assets::IconName::Share2,
    gpui_kit::assets::IconName::Shield,
    gpui_kit::assets::IconName::ShieldAlert,
    gpui_kit::assets::IconName::ShieldCheck,
    gpui_kit::assets::IconName::Shuffle,
    gpui_kit::assets::IconName::SkipBack,
    gpui_kit::assets::IconName::SkipForward,
    gpui_kit::assets::IconName::SlidersHorizontal,
    gpui_kit::assets::IconName::SlidersVertical,
    gpui_kit::assets::IconName::Smartphone,
    gpui_kit::assets::IconName::Sparkles,
    gpui_kit::assets::IconName::Speaker,
    gpui_kit::assets::IconName::SquareCheckBig,
    gpui_kit::assets::IconName::SquarePen,
    gpui_kit::assets::IconName::Star,
    gpui_kit::assets::IconName::Sun,
    gpui_kit::assets::IconName::Tag,
    gpui_kit::assets::IconName::Tags,
    gpui_kit::assets::IconName::ThumbsDown,
    gpui_kit::assets::IconName::ThumbsUp,
    gpui_kit::assets::IconName::Timer,
    gpui_kit::assets::IconName::Trash,
    gpui_kit::assets::IconName::TriangleAlert,
    gpui_kit::assets::IconName::Type,
    gpui_kit::assets::IconName::Upload,
    gpui_kit::assets::IconName::User,
    gpui_kit::assets::IconName::UserPlus,
    gpui_kit::assets::IconName::Users,
    gpui_kit::assets::IconName::Video,
    gpui_kit::assets::IconName::Volume1,
    gpui_kit::assets::IconName::Volume2,
    gpui_kit::assets::IconName::VolumeX,
    gpui_kit::assets::IconName::Wifi,
    gpui_kit::assets::IconName::WifiOff,
    gpui_kit::assets::IconName::Zap,
    gpui_kit::assets::IconName::ZoomIn,
    gpui_kit::assets::IconName::ZoomOut,
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

    let start_route = match args.start_route.as_deref() {
        Some(spec) => match nav::Route::parse_spec(spec) {
            Some(route) => Some(route),
            None => {
                eprintln!("neezer-desktop : écran inconnu : {spec}\n\n{USAGE}");
                return ExitCode::from(2);
            }
        },
        None => None,
    };
    let config = AppConfig {
        server: std::env::var("NEEZER_SERVER")
            .ok()
            .filter(|value| !value.trim().is_empty()),
        start_route,
        ..AppConfig::default()
    };
    let self_test = args.self_test_quit_after_first_frame;
    // The measurement sinks, before anything can paint or load.
    bench::init(
        args.frame_log.clone(),
        args.bench_events.clone(),
        args.bench_status.clone(),
    );
    let window_size = args.window_size;
    let runtime_handle = runtime.handle().clone();

    gpui_kit::application()
        .with_assets(ShellAssets)
        .run(move |cx| {
            cx.set_global(NetRuntime(Arc::clone(&runtime)));
            crate::theme::init(cx);
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
                .open_window(window_options(cx, window_size), move |window, cx| {
                    if self_test {
                        // `on_next_frame` runs just after the frame currently
                        // being drawn is handed to the compositor: the number
                        // the acceptance measures.
                        window.on_next_frame(move |_, cx| {
                            println!("first_frame_ms={}", started.elapsed().as_millis());
                            // The flag PROMISES "print, then quit": a subsystem
                            // whose shutdown does not join (the audio engine on
                            // a runner with no sound card, measured on CI: the
                            // line was printed and the process stayed alive)
                            // must not hang the job. The watchdog is this
                            // flag's own, never a shipped path.
                            std::thread::spawn(|| {
                                std::thread::sleep(std::time::Duration::from_secs(10));
                                std::process::exit(0);
                            });
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

    /// Every `IconName::<Nom>` the app's sources name is registered in `ShellAssets`.
    /// A glyph outside the bundle renders NOTHING and keeps its hit box, so a
    /// button would be there, invisible (D7 measured a player bar with only
    /// Play and a sidebar without Bibliothèque). This is the check that makes
    /// a new screen lot's icon list impossible to forget.
    #[test]
    fn every_icon_name_used_in_the_app_is_in_the_bundle() {
        use std::collections::BTreeSet;

        fn walk(dir: &std::path::Path, found: &mut BTreeSet<String>) {
            for entry in std::fs::read_dir(dir).expect("read the source tree") {
                let path = entry.expect("a directory entry").path();
                if path.is_dir() {
                    walk(&path, found);
                    continue;
                }
                if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).expect("read a source file");
                let mut rest = text.as_str();
                while let Some(at) = rest.find("IconName::") {
                    rest = &rest[at + "IconName::".len()..];
                    let name: String = rest
                        .chars()
                        .take_while(|character| {
                            character.is_ascii_alphanumeric() || *character == '_'
                        })
                        .collect();
                    if !name.is_empty() {
                        found.insert(name);
                    }
                }
            }
        }

        let mut used = BTreeSet::new();
        walk(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut used,
        );
        let bundled: BTreeSet<String> = crate::SHELL_ICONS
            .iter()
            .map(|icon| format!("{icon:?}"))
            .collect();
        let missing: Vec<&String> = used
            .iter()
            .filter(|name| !bundled.contains(*name))
            .collect();
        assert!(
            missing.is_empty(),
            "these icons are drawn but not registered in ShellAssets: {missing:?}"
        );
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

    /// D17's measurement flags: a window size, a route token, and the three
    /// logs. `--window-size` is refused when it is not `WxH`, and `--route`
    /// stays a string here (the app parses it as `Route::spec`).
    #[test]
    fn the_bench_flags_are_parsed_and_a_bad_size_is_refused() {
        let args = Args::parse([
            "--window-size".to_owned(),
            "2560x1600".to_owned(),
            "--route".to_owned(),
            "library:favorites".to_owned(),
            "--frame-log".to_owned(),
            "/tmp/f.log".to_owned(),
            "--bench-events".to_owned(),
            "/tmp/e.jsonl".to_owned(),
            "--bench-status".to_owned(),
            "/tmp/s.jsonl".to_owned(),
        ])
        .expect("parse");
        assert_eq!(args.window_size, Some((2560., 1600.)));
        assert_eq!(args.start_route.as_deref(), Some("library:favorites"));
        assert!(args.frame_log.is_some() && args.bench_events.is_some());
        assert!(args.bench_status.is_some());
        assert!(Args::parse(["--window-size".to_owned(), "huit".to_owned()]).is_err());
        assert!(Args::parse(["--window-size".to_owned()]).is_err());
    }
}
