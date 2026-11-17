//! The MB3a acceptance run, on the device: a rendered sine through the real
//! output for however many seconds the caller asks, then the numbers.
//!
//! Why a binary and not an instrumented test: `nz-droid-audio` is a Rust crate
//! that nothing in the APK drives yet (MB3b owns that wiring), and the crate's
//! acceptance is about the OUTPUT — AAudio's own thread, its buffer, its xruns —
//! not about the app's UI. `cargo ndk … build --bin droid-probe`, `adb push`, run
//! it: the same code path the app links, on the emulator's real AudioFlinger, with
//! no window and no speakers.
//!
//! ```sh
//! adb -s emulator-5620 shell /data/local/tmp/droid-probe 60
//! ```
//!
//! It prints one `MEASURE {…}` line (the PR's table is filled from it) and exits
//! non-zero when a number is out of contract: 0 xruns expected, the position must
//! follow the wall clock, and the reopen must neither rewind it nor restart it.

fn main() {
    #[cfg(target_os = "android")]
    android::run();
    #[cfg(not(target_os = "android"))]
    {
        eprintln!("droid-probe ne s'exécute que sur Android (cible hôte : rien à mesurer)");
        std::process::exit(2);
    }
}

#[cfg(target_os = "android")]
mod android {
    use std::ffi::CStr;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::mpsc::{Receiver, RecvTimeoutError};
    use std::time::{Duration, Instant};

    use nz_audio::{
        ByteStream, Engine, EngineConfig, EngineEvent, Lane, Quality, SourceError, StreamOpener,
        StreamReady,
    };
    use nz_droid_audio::{DroidOutput, Output, OutputPrefs, PullSession, output_for};

    unsafe extern "C" {
        fn __system_property_get(
            name: *const libc::c_char,
            value: *mut libc::c_char,
        ) -> libc::c_int;
    }

    /// `Build.VERSION.SDK_INT`, read from the property the platform itself uses.
    fn sdk_int() -> u32 {
        let name = b"ro.build.version.sdk\0";
        let mut value = [0 as libc::c_char; 92];
        // SAFETY: both pointers are valid, NUL-terminated/zeroed buffers.
        let read = unsafe { __system_property_get(name.as_ptr().cast(), value.as_mut_ptr()) };
        if read <= 0 {
            return 0;
        }
        // SAFETY: the platform NUL-terminates the value it wrote.
        let text = unsafe { CStr::from_ptr(value.as_ptr()) };
        text.to_str()
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    }

    /// The play cache's answer: one file on disk, no request, no server.
    struct LocalOpener {
        path: PathBuf,
    }

    impl StreamOpener for LocalOpener {
        fn open(
            &self,
            _id: &str,
            _quality: Quality,
            _start: Option<f64>,
            _from_byte: Option<u64>,
            _lane: Lane,
        ) -> Result<Box<dyn ByteStream>, SourceError> {
            Err(SourceError::Message(
                "la sonde lit le fichier local, pas le réseau".into(),
            ))
        }

        fn ready(&self, _id: &str, _quality: Quality) -> Result<StreamReady, SourceError> {
            Ok(StreamReady {
                ready: true,
                ranges: Some("bytes".into()),
                bytes: None,
                cached: true,
                offset: false,
                streaming: false,
            })
        }

        fn local(&self, _id: &str, _quality: Quality) -> Option<PathBuf> {
            Some(self.path.clone())
        }
    }

    /// The output under test, in the two shapes the platform offers.
    enum Out {
        Aa(DroidOutput),
        /// Boxed: a `PullSession` carries the mixer's whole handle and the enum
        // would otherwise be as big as it.
        Track(Box<PullSession>),
    }

    impl Out {
        /// Pull one block for the API-26 path. AAudio pulls by itself on its own
        /// thread, so there is nothing to do there.
        fn pull(&mut self, frames: usize) {
            if let Out::Track(session) = self {
                let mut block = vec![0.0f32; frames * 2];
                session.fill(&mut block);
            }
        }

        fn pumps(&self) -> bool {
            matches!(self, Out::Track(_))
        }

        fn path(&self) -> &'static str {
            match self {
                Out::Aa(_) => "aaudio",
                Out::Track(_) => "audiotrack",
            }
        }
    }

    /// Everything the PR's table needs, from one run.
    struct Measure {
        path: &'static str,
        sdk: u32,
        seconds: u64,
        rate: u32,
        buffer_frames: i32,
        buffer_ms: u32,
        mode: String,
        latency_frames: u64,
        xruns: u64,
        engine_underruns: u64,
        opens: u64,
        position_start: f64,
        position_end: f64,
        wall: f64,
        worst_gap_ms: f64,
        monotonic: bool,
        reopen_at: f64,
        reopen_position_before: f64,
        reopen_position_after: f64,
    }

    pub fn run() {
        let seconds: u64 = std::env::args()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(60)
            .max(1);
        let sdk = sdk_int();
        let prefs = OutputPrefs::default();
        let split = output_for(sdk);
        println!("SPLIT sdk={sdk} output={split:?}");

        let dir = PathBuf::from("/data/local/tmp/nz-droid-audio");
        std::fs::create_dir_all(&dir).expect("répertoire de travail");
        // The fixture is pushed by the host (`ffmpeg` here, `adb push` there): the
        // crate's decoder surface is the app's, and it does not read WAV.
        let music = std::env::args()
            .nth(2)
            .map(PathBuf::from)
            .unwrap_or_else(|| dir.join("sine.flac"));
        assert!(
            music.is_file(),
            "{} est absent (adb push ?)",
            music.display()
        );

        let cfg = EngineConfig {
            sample_rate: prefs.sample_rate,
            volume: 1.0,
            spool_dir: dir.join("spool"),
            ..Default::default()
        };
        let (engine, sink) = Engine::new_external(cfg, Arc::new(LocalOpener { path: music }));
        let events = engine.events();

        // The output first: AAudio's thread is already pulling, so the engine's
        // state machine advances exactly as it does in the app.
        let mut out = match split {
            Output::Aa => match DroidOutput::start(&engine, sink, prefs, sdk) {
                Ok(output) => {
                    println!(
                        "OPEN path=aaudio info={:?} mode={} buffer_ms={}",
                        output.info(),
                        output.performance_mode(),
                        output.buffer_ms()
                    );
                    Out::Aa(output)
                }
                Err(error) => {
                    println!("FAIL ouverture AAudio : {error}");
                    std::process::exit(1);
                }
            },
            Output::Track => {
                let session = PullSession::new(&engine, sink, prefs);
                println!(
                    "OPEN path=audiotrack buffer_frames={} info={:?}",
                    session.buffer_frames(),
                    engine.output_info()
                );
                Out::Track(Box::new(session))
            }
            Output::Unsupported => {
                println!("FAIL API {sdk} < 26 : aucune sortie");
                std::process::exit(1);
            }
        };

        engine.load("sine", Quality::Flac, None);
        if !wait_started(&mut out, &events) {
            println!("FAIL le moteur n'a pas démarré");
            std::process::exit(1);
        }
        // Everything is loaded; start the transport. The clock only advances for
        // frames the mixer really plays.
        engine.play();

        let measure = measure(&engine, &mut out, sdk, seconds, prefs);

        match out {
            Out::Aa(output) => output.stop(),
            Out::Track(session) => {
                // Close the session the way Kotlin does: its thread has stopped,
                // so nothing can be in flight.
                let handle = (*session).into_handle();
                // SAFETY: the handle came from `into_handle` and is used once.
                unsafe { nz_droid_audio::close_handle(handle) };
            }
        }
        engine.shutdown();

        println!(
            "MEASURE {{\"sdk\":{},\"path\":\"{}\",\"seconds\":{},\"rate\":{},\
             \"buffer_frames\":{},\"buffer_ms\":{},\"mode\":\"{}\",\"latency_frames\":{},\
             \"xruns\":{},\"engine_underruns\":{},\"opens\":{},\"position_start\":{:.3},\
             \"position_end\":{:.3},\"wall\":{:.3},\"worst_gap_ms\":{:.1},\
             \"monotonic\":{},\"reopen_at\":{:.3},\"reopen_position_before\":{:.3},\
             \"reopen_position_after\":{:.3}}}",
            measure.sdk,
            measure.path,
            measure.seconds,
            measure.rate,
            measure.buffer_frames,
            measure.buffer_ms,
            measure.mode,
            measure.latency_frames,
            measure.xruns,
            measure.engine_underruns,
            measure.opens,
            measure.position_start,
            measure.position_end,
            measure.wall,
            measure.worst_gap_ms,
            measure.monotonic,
            measure.reopen_at,
            measure.reopen_position_before,
            measure.reopen_position_after,
        );

        let mut failures = Vec::new();
        if measure.xruns > 0 {
            failures.push(format!(
                "{} sous-alimentation(s) signalée(s) par le périphérique",
                measure.xruns
            ));
        }
        if measure.engine_underruns > 0 {
            failures.push(format!(
                "{} sous-alimentation(s) du mixeur",
                measure.engine_underruns
            ));
        }
        if !measure.monotonic {
            failures.push("la position est revenue en arrière".to_string());
        }
        // The audio must run at the device's real rate. The clock subtracts the
        // latency the device reported, so what was played is the wall minus that
        // latency; the emulator's scheduling is allowed 1 s over a minute.
        let played = measure.position_end - measure.position_start;
        let latency_s = measure.latency_frames as f64 / f64::from(measure.rate.max(1));
        let expected = (measure.wall - latency_s).max(0.0);
        if (played - expected).abs() > 1.0 {
            failures.push(format!(
                "{played:.3} s d'audio pour {expected:.3} s attendues (temps réel {:.3} s, latence {latency_s:.3} s)",
                measure.wall
            ));
        }
        if split == Output::Aa && measure.opens < 2 {
            failures.push("la réouverture n'a pas eu lieu".to_string());
        }
        if failures.is_empty() {
            println!("OK aucun échec");
        } else {
            println!("FAIL {}", failures.join(" ; "));
            std::process::exit(1);
        }
    }

    fn measure(
        engine: &Engine,
        out: &mut Out,
        sdk: u32,
        seconds: u64,
        prefs: OutputPrefs,
    ) -> Measure {
        let block = (prefs.sample_rate as usize / 4).max(480); // 250 ms at 48 kHz
        let started = Instant::now();
        let position_start = engine.position();
        let mut last = position_start;
        let mut worst_gap_ms = 0.0f64;
        let mut monotonic = true;
        let mut reopened = false;
        let mut reopen_at = 0.0;
        let mut reopen_before = 0.0;
        let mut reopen_after = 0.0;

        while started.elapsed() < Duration::from_secs(seconds) {
            if out.pumps() {
                out.pull(block);
                // What `AudioTrack.write(…, WRITE_BLOCKING)` does to the Kotlin
                // thread: hand over half the buffer, then wait for room.
                std::thread::sleep(Duration::from_millis(250));
            } else {
                std::thread::sleep(Duration::from_millis(50));
            }

            let position = engine.position();
            if position + 1e-9 < last {
                monotonic = false;
            }
            worst_gap_ms = worst_gap_ms.max((position - last).abs() * 1_000.0);
            last = position;

            if !reopened && started.elapsed() >= Duration::from_secs(seconds / 2) {
                reopened = true;
                reopen_at = position;
                reopen_before = position;
                match out {
                    Out::Aa(output) => match output.reopen() {
                        Ok(info) => println!(
                            "REOPEN fait info={info:?} opens={} mode={}",
                            output.opens(),
                            output.performance_mode()
                        ),
                        Err(why) => println!("FAIL réouverture : {why}"),
                    },
                    Out::Track(session) => {
                        // The Kotlin counterpart: it releases its AudioTrack, opens
                        // a new one, and tells the session the buffer it holds.
                        session.set_latency(session.buffer_frames());
                        println!("REOPEN fait path=audiotrack");
                    }
                }
                reopen_after = engine.position();
            }
        }

        let (xruns, info, mode, buffer_ms, opens) = match out {
            Out::Aa(output) => (
                output.underruns(),
                output.info(),
                output.performance_mode().to_string(),
                output.buffer_ms(),
                output.opens(),
            ),
            Out::Track(session) => (
                0,
                engine.output_info(),
                "n/a (AudioTrack)".to_string(),
                (session.buffer_frames() * 1_000 / u64::from(prefs.sample_rate)) as u32,
                1,
            ),
        };
        let buffer_frames = match out {
            Out::Aa(_) => (u64::from(info.sample_rate) * u64::from(buffer_ms) / 1_000) as i32,
            Out::Track(session) => session.buffer_frames() as i32,
        };

        Measure {
            path: out.path(),
            sdk,
            seconds,
            rate: info.sample_rate,
            buffer_frames,
            buffer_ms,
            mode,
            latency_frames: info.latency_frames,
            xruns,
            engine_underruns: engine.underruns(),
            opens,
            position_start,
            position_end: engine.position(),
            wall: started.elapsed().as_secs_f64(),
            worst_gap_ms,
            monotonic,
            reopen_at,
            reopen_position_before: reopen_before,
            reopen_position_after: reopen_after,
        }
    }

    /// Wait for `Started` on the engine's own events, pulling for the API-26 path
    /// (nothing pulls for AAudio: its thread does).
    fn wait_started(out: &mut Out, events: &Receiver<EngineEvent>) -> bool {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            match events.recv_timeout(Duration::from_millis(50)) {
                Ok(EngineEvent::Started { .. }) => return true,
                Ok(_) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return false,
            }
            if out.pumps() {
                out.pull(480);
            }
        }
        false
    }
}
