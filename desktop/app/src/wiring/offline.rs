//! Offline: the desktop's GPUI adapter over `nz-offline` (SC4).
//!
//! The person's downloads, the `nz-bulk` worker that runs them, the snapshot a
//! screen reads and the settings they change all moved to the `nz-offline`
//! crate, which the mobile core (`mobile/rust/crates/nz-ffi/src/offline.rs`,
//! MB15) runs too. What is left here is exactly the part that has to know this
//! app:
//!
//! - [`attach`], the one line that turns an [`AppState`] — a session's `Api`,
//!   its store and the process's network runtime — into the process's
//!   [`Offline`];
//! - the tests that need this binary: the engine's opener (`NetOpener`, the
//!   proof a downloaded track plays with the link cut), the `NetRuntime` global
//!   (the proof a quit joins `nz-bulk` before its runtime goes), the real
//!   engine beside a 200-track download (rule 1's no-underrun half), and the
//!   `#[gpui::test]` that a restored session starts the worker.
//!
//! [`pub use nz_offline::*`] keeps every screen's path
//! (`crate::wiring::offline::{Offline, Snapshot, Failure, …}`) and every
//! behaviour unchanged: this file adds no rule of its own.

pub use nz_offline::*;

use crate::state::AppState;

/// The process's worker for this session, started on the first call.
///
/// One call site per session (the downloads screen, the badge, and one line in
/// `AppState` once a session is restored), so a download queued before a restart
/// is picked up again (rule 2).
pub fn attach(state: &AppState) -> Option<Offline> {
    let api = state.api.clone()?;
    let store = state.cache.clone()?;
    Some(Offline::ensure(api, store, state.runtime.clone()))
}

/// The tests that stayed with the app, and the harness the app's own benches
/// (`wiring::engine`) reuse: one local server, one scratch store, one panic
/// recorder, from the crate.
#[cfg(test)]
pub(crate) mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use nz_api::{Api, SessionJar};
    use nz_cache::DownloadState;

    use super::*;

    pub(crate) use nz_offline::testing::*;

    /// **Closing the app during a download: the worker stops, nothing panics.**
    ///
    /// `NetRuntime` — the global that owns the app's network runtime — joins
    /// `nz-bulk` in its `Drop`, BEFORE the runtime it holds goes. Tokio panics
    /// when a timer is polled on a runtime that is shutting down
    /// (`TimerEntry::poll_elapsed`: « A Tokio 1.x context was found, but it is
    /// being shutdown. » — master's CI log, run 43964917017), and the suite still
    /// passed because a `catch_unwind` swallowed it. This test drops EXACTLY what
    /// the app drops, mid-body, and proves no thread panicked and that the worker
    /// was gone before the runtime.
    #[test]
    fn closing_the_app_mid_download_stops_the_worker_without_a_panic() {
        in_its_own_process("wiring::offline::tests::the_close_bench_in_its_own_process");
    }

    #[test]
    #[ignore = "run by its parent, a process of its own (a panic hook is process-wide)"]
    fn the_close_bench_in_its_own_process() {
        let panics = Panics::install();
        let scratch = Scratch::new("close");
        let store = scratch.store();
        let ids: Vec<String> = (0..40).map(|index| format!("c{index}")).collect();
        let borrowed: Vec<&str> = ids.iter().map(String::as_str).collect();
        // A link slow enough that a body is in flight when the window closes, and
        // a Bulk cap that makes the worker SLEEP between its chunks: that sleep is
        // the timer tokio asserted on.
        let server = TestServer::start(Link::new(Some(400_000.0)), body_map(&borrowed, 128 * 1024));
        let runtime = Arc::new(runtime());
        let api = api_for(&server, &store, &scratch);
        api.set_bulk_cap_from_stream(Some(8_000));
        let service = Offline::ensure(api, store.clone(), runtime.handle().clone());
        service.choose("playlist", "p1", "Voyage", choices(&borrowed));
        assert!(
            wait_for(Duration::from_secs(20), || {
                service
                    .snapshot()
                    .running
                    .as_ref()
                    .is_some_and(|running| running.bytes > 0)
            }),
            "a body is being written: {:?}",
            service.snapshot().running
        );
        let interrupted = service
            .snapshot()
            .running
            .expect("a download in flight")
            .track;

        // The window closes: this is the app's own global, and its `Drop` is what
        // a quit runs. The runtime the worker borrows goes after it.
        let net = crate::NetRuntime(Arc::clone(&runtime));
        let closing = Instant::now();
        drop(net);
        let closed = closing.elapsed();
        drop(runtime);
        // A worker that survived the close would panic right here.
        std::thread::sleep(Duration::from_millis(500));

        let panicked = panics.take();
        println!(
            "fermeture pendant un téléchargement : rendue en {closed:?}, {} panique(s)",
            panicked.len()
        );
        assert!(
            panicked.is_empty(),
            "no thread may panic while the app closes: {panicked:?}"
        );
        assert!(
            closed < Duration::from_millis(500),
            "the close answered in {closed:?}, not behind the chunk in flight"
        );
        // The row is still `active` and its `.part` is on disk: the next launch
        // resumes it (`a_restart_mid_download_resumes_the_set`), and an
        // interruption is not a verdict about the track.
        let jobs = store.downloads().pending("42", 100).expect("the queue");
        let active: Vec<&str> = jobs
            .iter()
            .filter(|job| job.state == DownloadState::Active)
            .map(|job| job.track.as_str())
            .collect();
        assert_eq!(
            active,
            vec![interrupted.as_str()],
            "exactly the body in flight is interrupted"
        );
        assert!(
            std::fs::read_dir(store.offline_dir())
                .map(|entries| entries
                    .filter_map(Result::ok)
                    .any(|entry| entry.path().extension().is_some_and(|ext| ext == "part")))
                .unwrap_or(false),
            "the interrupted body is kept as a .part"
        );
        assert_eq!(
            service.snapshot().progress.failed,
            0,
            "an interruption is not a failure"
        );
    }

    /// **The app starts the worker at restore/login** (D13's second seam, in
    /// `AppState`): a session that comes back starts the download thread, which
    /// is what makes a queue left by a previous run resume without anybody
    /// opening Réglages.
    #[gpui::test]
    async fn a_restored_session_starts_the_download_worker(cx: &mut gpui::TestAppContext) {
        use gpui_kit::AppContext as _;

        use crate::state::{AppConfig, AppEvent, AppState, event_channel};

        cx.update(crate::theme::init);
        let scratch = Scratch::new("seam");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let (events_tx, _events_rx) = event_channel();
        let config = AppConfig {
            server: Some("http://127.0.0.1:1".to_owned()),
            start_audio: false,
            start_mpris: false,
            ..AppConfig::for_test(scratch.dir())
        };
        let state = {
            let runtime = runtime.handle().clone();
            cx.update(|cx| cx.new(move |_| AppState::new(config, runtime, events_tx, None)))
        };
        let me: nz_model::Me = serde_json::from_str(
            r#"{"user":{"id":"u1","name":"bench","admin":false,"access":{},"sources":[]}}"#,
        )
        .expect("a session");
        state.update(cx, |state, cx| {
            state.on_event(AppEvent::Restored(Ok(Box::new(me))), cx);
        });
        let service = Offline::get().expect("the restored session started the worker");
        let snapshot = service.snapshot();
        println!(
            "worker démarré à la restauration : {:?}, {:?}",
            snapshot.state, snapshot.progress
        );
        assert!(
            matches!(
                snapshot.state,
                WorkerState::Idle | WorkerState::Offline | WorkerState::Unavailable
            ),
            "the worker exists and says where it stands: {:?}",
            snapshot.state
        );
        service.shutdown();
        drop(runtime);
    }

    /// **The network is really cut.** A track that was downloaded plays through
    /// the engine with a client whose only address is a dead port: the engine's
    /// opener finds the file on disk (`NetOpener::local`, D13's seam), so nothing
    /// is asked of the network and the audio still starts.
    #[test]
    fn a_downloaded_track_plays_with_the_network_cut() {
        in_its_own_process("wiring::offline::tests::the_offline_playback_bench_in_its_own_process");
    }

    #[test]
    #[ignore = "run by its parent, a process of its own (RSS neighbours)"]
    fn the_offline_playback_bench_in_its_own_process() {
        let scratch = Scratch::new("cut");
        let store = scratch.store();
        let flac = scratch.dir().join("offline.flac");
        if !tone_flac(&flac) {
            println!("ffmpeg absent : la lecture hors ligne n'a pas été mesurée");
            return;
        }
        let audio = std::fs::read(&flac).expect("the generated FLAC");
        let mut bodies: HashMap<String, Payload> = HashMap::new();
        bodies.insert("kept".to_owned(), Payload::Bytes(audio));
        let server = TestServer::start(Link::new(None), bodies);
        let runtime = runtime();
        let api = api_for(&server, &store, &scratch);
        let service = Offline::start(store.clone());
        service.configure(api, runtime.handle().clone());
        service.choose("track", "kept", "Titre gardé", choices(&["kept"]));
        assert!(wait_for(Duration::from_secs(30), || {
            service.snapshot().progress.done == 1
        }));
        let fetched = server.hits("kept");
        let (path, quality) = service.local("kept").expect("the download is on disk");
        println!(
            "téléchargé : {} ({:?}, {} requête(s))",
            path.display(),
            quality,
            fetched
        );

        // The network goes: a client for a port nothing listens on, and the
        // verdict the app would have.
        drop(server);
        let dead = Arc::new(
            Api::builder("http://127.0.0.1:1")
                .cache(Arc::new(store.clone()))
                .session(Arc::new(SessionJar::at(scratch.dir().join("dead.json"))))
                .build()
                .expect("a client"),
        );
        dead.set_owner(Some("42".to_owned()));
        dead.set_online(false);
        assert!(!dead.is_online());

        let opener = Arc::new(crate::wiring::engine::NetOpener::new(
            dead,
            runtime.handle().clone(),
            store.clone(),
        ));
        let config = nz_audio::EngineConfig {
            sample_rate: 48_000,
            // A quarter-second ring: the audio a test needs, and half the memory
            // — `cargo test` runs the screens' RSS assertions in this process.
            ring_seconds: 0.25,
            spool_dir: store.audio_dir(),
            ..nz_audio::EngineConfig::default()
        };
        let (engine, mut sink) = nz_audio::Engine::new_offline(config, opener);
        let events = engine.events();
        engine.load("kept", nz_audio::Quality::Opus320, None);
        assert!(
            pump_until(
                &mut sink,
                &events,
                Duration::from_secs(10),
                |event| matches!(event, nz_audio::EngineEvent::Started { .. })
            ),
            "the downloaded track started with no network at all"
        );
        engine.play();
        let mut buffer = vec![0f32; 480 * 2];
        let mut heard = 0.0f32;
        for _ in 0..40 {
            sink.render(&mut buffer);
            heard = heard.max(
                buffer
                    .iter()
                    .fold(0.0f32, |loudest, sample| loudest.max(sample.abs())),
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        println!("niveau entendu hors ligne : {heard:.4}");
        assert!(heard > 0.01, "the audio really played ({heard:.4})");
        assert_eq!(engine.underruns(), 0, "no underrun with the link cut");
        assert_eq!(service.snapshot().progress.done, 1);
    }

    /// **Rule 1, the other half: no underrun.** A real FLAC plays on the audio
    /// engine while the same 200-track download runs: the decoder is never
    /// starved, and the run finishes.
    ///
    /// The audio is generated with ffmpeg (real decoding, real resampling), like
    /// `nz-audio`'s own suites; without ffmpeg the test says so and passes.
    #[test]
    fn a_two_hundred_track_download_while_a_track_plays_has_no_underrun() {
        in_its_own_process("wiring::offline::tests::the_underrun_bench_in_its_own_process");
    }

    #[test]
    #[ignore = "run by its parent, a process of its own (RSS neighbours)"]
    fn the_underrun_bench_in_its_own_process() {
        let scratch = Scratch::new("underrun");
        let store = scratch.store();
        let flac = scratch.dir().join("tone.flac");
        if !tone_flac(&flac) {
            println!("ffmpeg absent : le critère « 0 sous-alimentation » n'a pas été mesuré");
            return;
        }
        let ids: Vec<String> = (0..200).map(|index| format!("dl{index}")).collect();
        let borrowed: Vec<&str> = ids.iter().map(String::as_str).collect();
        let mut bodies = body_map(&borrowed, 16 * 1024);
        bodies.insert(
            "play".to_owned(),
            Payload::Bytes(std::fs::read(&flac).expect("the generated FLAC")),
        );
        let server = TestServer::start(Link::new(None), bodies);
        let runtime = runtime();
        let api = api_for(&server, &store, &scratch);
        let service = Offline::start(store.clone());
        service.configure(api.clone(), runtime.handle().clone());

        let opener = Arc::new(crate::wiring::engine::NetOpener::new(
            api.clone(),
            runtime.handle().clone(),
            store.clone(),
        ));
        let config = nz_audio::EngineConfig {
            sample_rate: 48_000,
            // A quarter-second ring: the audio a test needs, and half the memory
            // — `cargo test` runs the screens' RSS assertions in this process.
            ring_seconds: 0.25,
            spool_dir: store.audio_dir(),
            ..nz_audio::EngineConfig::default()
        };
        let (engine, mut sink) = nz_audio::Engine::new_offline(config, opener);
        let events = engine.events();
        engine.load("play", nz_audio::Quality::Flac, None);
        assert!(
            pump_until(
                &mut sink,
                &events,
                Duration::from_secs(10),
                |event| matches!(event, nz_audio::EngineEvent::Started { .. })
            ),
            "the track started"
        );
        engine.play();

        service.choose("playlist", "p1", "Voyage", choices(&borrowed));
        let rendered = pump_for(&mut sink, Duration::from_secs(6));

        let underruns = engine.underruns();
        let snapshot = service.snapshot();
        println!(
            "played {rendered:.2} s of audio while {} tracks downloaded: {underruns} underruns, \
             {} done, {} failed",
            borrowed.len(),
            snapshot.progress.done,
            snapshot.progress.failed
        );
        assert_eq!(underruns, 0, "the download never starved the decoder");
        assert!(
            wait_for(Duration::from_secs(20), || {
                service.snapshot().progress.done == borrowed.len()
            }),
            "all 200 were fetched: {:?}",
            service.snapshot().progress
        );
        assert_eq!(service.snapshot().progress.failed, 0);
        // The worker borrows this runtime: it is joined first.
        service.shutdown();
        drop(runtime);
    }

    /// A 60 s stereo FLAC of a 440 Hz tone: real audio, and small enough that
    /// the link is not the subject of this test (nor does it sit in memory while
    /// the other tests of this binary measure their own RSS). `false` without
    /// ffmpeg.
    pub(crate) fn tone_flac(path: &PathBuf) -> bool {
        let Ok(status) = std::process::Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "aevalsrc=0.2*cos(2*PI*440*t):s=48000:d=60:c=stereo",
                "-c:a",
                "flac",
            ])
            .arg(path)
            .status()
        else {
            return false;
        };
        status.success() && path.is_file()
    }

    /// Render the mixer in real time for `seconds` (exactly like a callback),
    /// answering how much audio was rendered.
    pub(crate) fn pump_for(sink: &mut nz_audio::OfflineSink, seconds: Duration) -> f64 {
        let frames = (48_000.0 * seconds.as_secs_f64()) as usize;
        let mut buffer = vec![0f32; 480 * 2];
        let mut rendered = 0usize;
        let started = Instant::now();
        while rendered < frames {
            sink.render(&mut buffer);
            rendered += 480;
            let target = Duration::from_secs_f64(rendered as f64 / 48_000.0);
            let spent = started.elapsed();
            if target > spent {
                std::thread::sleep(target - spent);
            }
        }
        rendered as f64 / 48_000.0
    }

    /// Pump until `event` arrives: the engine's own signal, never a clock.
    pub(crate) fn pump_until(
        sink: &mut nz_audio::OfflineSink,
        events: &std::sync::mpsc::Receiver<nz_audio::EngineEvent>,
        timeout: Duration,
        mut done: impl FnMut(&nz_audio::EngineEvent) -> bool,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        let mut buffer = vec![0f32; 480 * 2];
        while Instant::now() < deadline {
            sink.render(&mut buffer);
            while let Ok(event) = events.try_recv() {
                if done(&event) {
                    return true;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }
}
