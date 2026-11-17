//! MPRIS wiring: the player bar, the media keys and the Omarchy bar read this.
//!
//! `nz-mpris` owns the D-Bus surface; this module starts it off the UI thread
//! (`Mpris::start` blocks until the bus name is ours, up to five seconds — a
//! first frame must not wait for it) and publishes what the player shows. The
//! transport commands come back as [`crate::state::AppEvent::Mpris`] and are
//! applied through `PlayerState::apply`, exactly like a click, so MPRIS can do
//! nothing the app cannot do itself.
//!
//! D9 owns the publication, and three of its rules are worth stating here
//! because they are what a client sees:
//!
//! - **A new track announces its FULL metadata once, then only its position
//!   moves.** `payload` is the shell's author: `set_track` resets the position to
//!   0 (a new track starts there, and MPRIS must not show the previous track's
//!   progress for a quarter of a second), and the 4 Hz tick never re-announces
//!   the track — `nz-mpris` turns a position into a signal only when it departed
//!   from what a client extrapolated.
//! - **An empty queue is `Stopped`, not "Paused on the last track".** The bar's
//!   widget otherwise shows a play button over a track nobody is playing, and
//!   `playerctl status` lies. The last metadata is kept (the spec expects it),
//!   but the transport says stopped.
//! - **The mapping table is the contract**: `Repeat` ↔ `LoopStatus`,
//!   `playing` ↔ `PlaybackStatus`, and the three `publish`d setters are the only
//!   place they are named, so a wrong icon cannot be produced by a second path.
//!
//! Measured on this machine (`playerctl` 2.4.1, a private `dbus-run-session`,
//! the test at the bottom of this file): the wiring's `publish` puts
//! `xesam:title` / `xesam:artist` / `xesam:album` / `mpris:length` and
//! `PlaybackStatus = Playing` on the bus, `playerctl -p neezer play-pause` and a
//! direct `PlayPause` call both arrive as `AppEvent::Mpris(PlayPause)` within
//! 100 ms, and with the queue emptied the status reads `Stopped` while the
//! metadata is still the last track's.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use nz_mpris::{
    LoopStatus, Metadata, Mpris, MprisCommand, MprisHandle, MprisTicker, PlaybackStatus,
};
use nz_player::Repeat;

use crate::state::AppEvent;
use crate::wiring::events::Sender;

/// What the shell tells the bus about the transport.
#[derive(Clone, Debug, Default)]
pub struct NowPlaying {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: f64,
    pub playing: bool,
    pub position: f64,
    pub volume: f64,
    pub shuffle: bool,
    pub repeat: Repeat,
}

/// Start the server on a worker, then announce it. `runtime` drives the command
/// inbox (a tokio channel) and forwards each command to the shell.
pub fn spawn(runtime: tokio::runtime::Handle, events: Sender<AppEvent>) {
    let spawned = std::thread::Builder::new()
        .name("nz-mpris-start".to_owned())
        .spawn(move || {
            let (tx, mut rx) = tokio::sync::mpsc::channel::<MprisCommand>(32);
            let handle = Mpris::start("neezer", tx);
            let forward = events.clone();
            runtime.spawn(async move {
                while let Some(command) = rx.recv().await {
                    forward.send(AppEvent::Mpris(command));
                }
            });
            events.send(AppEvent::MprisReady(handle));
        });
    if spawned.is_err() {
        // No thread: the player still works, the bar and media keys do not.
        tracing::warn!("MPRIS indisponible : le fil de démarrage n'a pas pu être créé");
    }
}

/// Publish the transport. `last_track` is the shell's memory of what the bus
/// was last told: a new track is announced once, a position tick never is
/// (`nz-mpris` owns the seek tolerance).
pub fn publish(handle: &MprisHandle, now: &NowPlaying, last_track: &mut Option<String>) {
    if !handle.is_serving() {
        return;
    }
    if now.id.is_empty() {
        // Nothing in the queue: keep the metadata (MPRIS expects a player to
        // remember what it last played) and say the transport is stopped.
        handle.set_status(PlaybackStatus::Stopped);
        return;
    }
    if last_track.as_deref() != Some(now.id.as_str()) {
        let mut metadata = Metadata::new(now.id.clone(), now.title.clone());
        if !now.artist.is_empty() {
            metadata = metadata.artist(now.artist.clone());
        }
        if !now.album.is_empty() {
            metadata = metadata.album(now.album.clone());
        }
        if now.duration > 0.0 {
            metadata = metadata.length(now.duration);
        }
        handle.set_track(metadata);
        *last_track = Some(now.id.clone());
    }
    handle.set_status(if now.playing {
        PlaybackStatus::Playing
    } else {
        PlaybackStatus::Paused
    });
    handle.set_position_s(now.position);
    handle.set_volume(now.volume.clamp(0.0, 1.0));
    handle.set_shuffle(now.shuffle);
    handle.set_loop(match now.repeat {
        Repeat::Off => LoopStatus::None,
        Repeat::All => LoopStatus::Playlist,
        Repeat::One => LoopStatus::Track,
    });
}

/// How long the shell waits before retrying MPRIS after an inert start. The
/// music never stops for it; this is only so a bus that appeared late is picked
/// up by the next track.
pub const RETRY: Duration = Duration::from_secs(30);

/// The position clock's period: the 4 Hz [`crate::state::AppState::tick`] uses,
/// so MPRIS and the bar agree on how often a playhead moves.
pub const POSITION_PERIOD: Duration = Duration::from_millis(250);

/// Publishes the audible position on its OWN thread, four times a second.
///
/// Why it is not the shell's tick (D7's report, measured again in D17): GPUI
/// drives `spawn`ed timers from the app's foreground executor, and a window
/// that is hidden — minimised, or on another Hyprland workspace — stops
/// painting. `playerctl position` then answered the same value for as long as
/// the window was away, and the Omarchy bar's playhead froze. This clock reads
/// the audio engine's own mixer clock and writes the (silent) MPRIS position
/// property with no window, no frame and no GPUI in the path; it is stopped
/// when the shell drops it (quit) or replaces the engine.
pub struct PositionClock {
    stop: Arc<AtomicBool>,
}

impl PositionClock {
    /// Start the clock. `position` is `Audio::position` — the mixer's clock,
    /// which keeps running while the window does not paint because the sound
    /// keeps playing.
    pub fn start(ticker: MprisTicker, position: impl Fn() -> f64 + Send + 'static) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let for_thread = stop.clone();
        let spawned = std::thread::Builder::new()
            .name("nz-mpris-clock".to_owned())
            .spawn(move || {
                run_clock(&for_thread, POSITION_PERIOD, &position, &mut |seconds| {
                    ticker.set_position_s(seconds)
                });
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "horloge MPRIS indisponible");
        }
        Self { stop }
    }
}

impl Drop for PositionClock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// One publication per `period`, until `stop` says otherwise. Factored out so
/// the test drives the REAL loop at a 1 ms period, in a thread, with no window.
fn run_clock(
    stop: &AtomicBool,
    period: Duration,
    position: &(impl Fn() -> f64 + ?Sized),
    publish: &mut impl FnMut(f64),
) {
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(period);
        if stop.load(Ordering::Relaxed) {
            break;
        }
        publish(position());
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::process::Command;
    use std::time::Instant;

    use zbus::OwnedObjectPath;
    use zbus::proxy::CacheProperties;
    use zbus::{Connection, OwnedVariant};

    use super::*;
    use crate::state::event_channel;
    use crate::wiring::events::Receiver;

    const SERVICE: &str = "org.mpris.MediaPlayer2.neezer";
    const PLAYER_PATH: &str = "/org/mpris/MediaPlayer2";
    const PLAYER_INTERFACE: &str = "org.mpris.MediaPlayer2.Player";
    /// Set in the child `dbus-run-session` starts, so the real work happens on a
    /// bus of our own and the developer's own player is never shadowed.
    const PRIVATE_BUS_CHILD: &str = "NZ_MPRIS_WIRING_BUS_CHILD";
    const NO_BUS_CHILD: &str = "NZ_MPRIS_WIRING_NO_BUS_CHILD";

    /// D17: the position advances with NO window and no GPUI in the path — the
    /// clock is a thread, so a hidden window cannot stop it. Measured on a
    /// virtual 1 ms period: 20 publications, strictly increasing positions,
    /// while nothing paints.
    #[test]
    fn the_position_clock_moves_without_a_window() {
        use std::sync::Mutex;
        use std::sync::atomic::AtomicU64;

        let stop = Arc::new(AtomicBool::new(false));
        let clock = Arc::new(AtomicU64::new(0));
        let published: Arc<Mutex<Vec<f64>>> = Arc::new(Mutex::new(Vec::new()));
        let clock_for_thread = clock.clone();
        let published_for_thread = published.clone();
        let stop_for_thread = stop.clone();
        // The SAME loop `PositionClock::start` runs, at a 1 ms period, with a
        // clock that advances 0.25 s per read: no window exists in this test.
        let thread = std::thread::spawn(move || {
            let position =
                move || clock_for_thread.fetch_add(250, Ordering::Relaxed) as f64 / 1000.0;
            run_clock(
                &stop_for_thread,
                Duration::from_millis(1),
                &position,
                &mut |seconds| published_for_thread.lock().unwrap().push(seconds),
            );
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        while published.lock().unwrap().len() < 20 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        stop.store(true, Ordering::Relaxed);
        thread.join().expect("the clock thread stops");
        let values = published.lock().unwrap();
        assert!(
            values.len() >= 20,
            "the clock published {} times",
            values.len()
        );
        assert!(
            values.windows(2).all(|pair| pair[1] > pair[0]),
            "every tick moved the position forward: {values:?}"
        );
    }

    #[test]
    fn the_three_repeat_modes_map_onto_mpris_loop_statuses() {
        // The mapping is the contract a client reads; a wrong one shows a
        // repeat icon that lies. `publish` is the only place that names them.
        let map = |repeat| match repeat {
            Repeat::Off => LoopStatus::None,
            Repeat::All => LoopStatus::Playlist,
            Repeat::One => LoopStatus::Track,
        };
        assert_eq!(map(Repeat::Off), LoopStatus::None);
        assert_eq!(map(Repeat::All), LoopStatus::Playlist);
        assert_eq!(map(Repeat::One), LoopStatus::Track);
    }

    fn now_playing(id: &str) -> NowPlaying {
        NowPlaying {
            id: id.to_owned(),
            title: "Balancing the Dark".to_owned(),
            artist: "Kaya Project".to_owned(),
            album: "Songs from the Inside".to_owned(),
            duration: 312.0,
            playing: true,
            position: 0.0,
            volume: 0.8,
            shuffle: false,
            repeat: Repeat::All,
        }
    }

    /// The shell's own publication, on a private bus: what the Omarchy bar reads
    /// and what a media key does.
    #[test]
    fn the_wiring_publishes_what_the_bar_reads_and_takes_the_media_keys() {
        if std::env::var_os(PRIVATE_BUS_CHILD).is_some() {
            private_bus_body();
            return;
        }
        let binary = std::env::current_exe().expect("the test binary's path");
        let mut command = Command::new("dbus-run-session");
        command
            .arg("--")
            .arg(&binary)
            .args([
                "--exact",
                "wiring::mpris::tests::the_wiring_publishes_what_the_bar_reads_and_takes_the_media_keys",
                "--nocapture",
            ])
            .env(PRIVATE_BUS_CHILD, "1")
            .env_remove("DBUS_SESSION_BUS_ADDRESS");
        run_child(command, Duration::from_secs(120));
    }

    /// `publish` on an inert handle (no session bus at all) is a no-op: the
    /// player plays, the bar does not, and nothing panics or waits.
    #[test]
    fn without_a_bus_the_publication_is_a_no_op() {
        if std::env::var_os(NO_BUS_CHILD).is_some() {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");
            let (tx, rx) = event_channel();
            spawn(runtime.handle().clone(), tx);
            let handle = next_handle(&runtime, &rx);
            assert!(!handle.is_serving(), "there is no session bus here");
            let mut last = None;
            publish(&handle, &now_playing("3135556"), &mut last);
            assert_eq!(
                last, None,
                "nothing was announced, so nothing is remembered"
            );
            // And an empty queue on a dead bus is the same no-op.
            let mut empty = now_playing("");
            empty.playing = false;
            publish(&handle, &empty, &mut last);
            return;
        }
        let binary = std::env::current_exe().expect("the test binary's path");
        let mut command = Command::new(&binary);
        command
            .args([
                "--exact",
                "wiring::mpris::tests::without_a_bus_the_publication_is_a_no_op",
                "--nocapture",
            ])
            .env(NO_BUS_CHILD, "1")
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                "unix:path=/nonexistent/neezer-wiring-test",
            );
        run_child(command, Duration::from_secs(60));
    }

    fn next_handle(runtime: &tokio::runtime::Runtime, events: &Receiver<AppEvent>) -> MprisHandle {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let next = runtime.block_on(async {
                tokio::time::timeout(Duration::from_millis(50), events.next()).await
            });
            match next {
                Ok(Some(AppEvent::MprisReady(handle))) => return handle,
                Ok(Some(_)) => {}
                Ok(None) => panic!("the event channel closed before MPRIS was ready"),
                Err(_) if Instant::now() < deadline => {}
                Err(_) => panic!("MPRIS never announced itself"),
            }
        }
    }

    /// The child's body: a real bus, the real publication, a real client.
    fn private_bus_body() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        let (events_tx, events_rx) = event_channel();
        spawn(runtime.handle().clone(), events_tx);
        let handle = next_handle(&runtime, &events_rx);
        assert!(handle.is_serving(), "the private bus is ours");

        let mut last = None;
        let mut now = now_playing("3135556");
        publish(&handle, &now, &mut last);
        assert_eq!(last.as_deref(), Some("3135556"));

        let connection = runtime
            .block_on(Connection::session())
            .expect("the private bus");
        let player: zbus::Proxy<'_> = runtime
            .block_on(
                zbus::proxy::Builder::new(&connection)
                    .destination(SERVICE)
                    .path(PLAYER_PATH)
                    .interface(PLAYER_INTERFACE)
                    // zbus caches properties lazily; a test that reads a value
                    // right after a change would get the stale snapshot.
                    .cache_properties(CacheProperties::No)
                    .build(),
            )
            .expect("the player proxy");

        // What the bar's widget shows: the track's own facts, then the status.
        let metadata = wait_for(&runtime, || {
            let value: Option<HashMap<String, OwnedVariant>> =
                runtime.block_on(player.get_property("Metadata")).ok();
            value
        });
        let metadata = metadata.expect("a Metadata property");
        assert_eq!(
            text(&metadata, "xesam:title").as_deref(),
            Some("Balancing the Dark")
        );
        let artist = metadata.get("xesam:artist").expect("an artist");
        let artists: Vec<String> = artist
            .try_clone()
            .ok()
            .and_then(|value| value.try_into().ok())
            .unwrap_or_default();
        assert_eq!(artists, ["Kaya Project"]);
        assert_eq!(
            text(&metadata, "xesam:album").as_deref(),
            Some("Songs from the Inside")
        );
        let track_id = metadata
            .get("mpris:trackid")
            .and_then(|value| value.try_clone().ok())
            .and_then(|value| OwnedObjectPath::try_from(value).ok())
            .map(|path| path.to_string());
        assert_eq!(track_id.as_deref(), Some("/fr/neezer/track/3135556"));
        let status = wait_for(&runtime, || {
            let value: Option<String> =
                runtime.block_on(player.get_property("PlaybackStatus")).ok();
            value
        });
        assert_eq!(status.as_deref(), Some("Playing"));

        // A media key: exactly what Hyprland's binding runs (`playerctl`), and
        // the D-Bus method it calls underneath. Both must reach the shell.
        if which("playerctl") {
            let output = Command::new("playerctl")
                .args(["-p", "neezer", "play-pause"])
                .output()
                .expect("playerctl runs");
            assert!(
                output.status.success(),
                "playerctl play-pause: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        } else {
            runtime
                .block_on(player.call::<_, (), ()>("PlayPause", &()))
                .expect("PlayPause");
        }
        assert_eq!(
            wait_for_command(&runtime, &events_rx),
            MprisCommand::PlayPause,
            "the media key rejoins the shell as a transport command"
        );

        // Pause then an emptied queue: the status must say Stopped, and the
        // metadata must survive (a client still shows the last track).
        now.playing = false;
        publish(&handle, &now, &mut last);
        let status = wait_for(&runtime, || {
            let value: Option<String> =
                runtime.block_on(player.get_property("PlaybackStatus")).ok();
            value
        });
        assert_eq!(status.as_deref(), Some("Paused"));
        let mut empty = now.clone();
        empty.id = String::new();
        publish(&handle, &empty, &mut last);
        let status = wait_for(&runtime, || {
            let value: Option<String> =
                runtime.block_on(player.get_property("PlaybackStatus")).ok();
            value
        });
        assert_eq!(status.as_deref(), Some("Stopped"));
        let metadata: HashMap<String, OwnedVariant> = runtime
            .block_on(player.get_property("Metadata"))
            .expect("metadata after the queue emptied");
        assert_eq!(
            text(&metadata, "xesam:title").as_deref(),
            Some("Balancing the Dark"),
            "the last track is remembered while stopped"
        );
    }

    fn text(metadata: &HashMap<String, OwnedVariant>, key: &str) -> Option<String> {
        metadata
            .get(key)?
            .try_clone()
            .ok()?
            .try_into()
            .ok()
            .map(|value: String| value)
    }

    /// Poll `read` until it answers, so the assertions wait on an event instead
    /// of a fixed sleep. 5 s is a hang, not a duration.
    fn wait_for<T>(
        runtime: &tokio::runtime::Runtime,
        mut read: impl FnMut() -> Option<T>,
    ) -> Option<T> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(value) = read() {
                return Some(value);
            }
            if Instant::now() >= deadline {
                return None;
            }
            runtime.block_on(async {
                tokio::time::sleep(Duration::from_millis(10)).await;
            });
        }
    }

    fn wait_for_command(
        runtime: &tokio::runtime::Runtime,
        events: &Receiver<AppEvent>,
    ) -> MprisCommand {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let next = runtime.block_on(async {
                tokio::time::timeout(Duration::from_millis(50), events.next()).await
            });
            match next {
                Ok(Some(AppEvent::Mpris(command))) => return command,
                Ok(Some(_)) => {}
                Ok(None) => panic!("the event channel closed"),
                Err(_) if Instant::now() < deadline => {}
                Err(_) => panic!("the media key never reached the shell"),
            }
        }
    }

    fn which(binary: &str) -> bool {
        Command::new("sh")
            .args(["-c", &format!("command -v {binary}")])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    fn run_child(mut command: Command, timeout: Duration) {
        let label = format!("{command:?}");
        let mut child = command.spawn().expect("the child test binary starts");
        let deadline = Instant::now() + timeout;
        loop {
            match child.try_wait().expect("waiting for the child") {
                Some(status) => {
                    assert!(status.success(), "{label} failed: {status}");
                    return;
                }
                None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
                None => {
                    let _ = child.kill();
                    panic!("{label} did not finish in {timeout:?}");
                }
            }
        }
    }
}
