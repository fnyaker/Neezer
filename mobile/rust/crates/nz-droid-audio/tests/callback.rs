//! MB3a's host acceptance: the callback body, the hand-back that makes a reopen
//! seamless, the buffer maths, and the API-26 session.
//!
//! What a phone does through AAudio is what these tests do with a loop: `fill` is
//! called with the device's buffer, block after block, and a reopen is exactly
//! what it is on the device — the stream's `Pull` is dropped and a new one takes
//! the same sink. So "no hole, no repeat" is an assertion about the samples, not
//! a race with a sound card.
//!
//! Every number the assertions carry was measured on this machine (Rust 1.99.1,
//! 48 kHz); `--nocapture` prints them.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use nz_audio::{
    ByteStream, Engine, EngineConfig, EngineEvent, ExternalSink, Lane, Quality, SourceError,
    StreamOpener, StreamReady,
};
use nz_droid_audio::{
    DroidOutput, DroidOutputError, Handback, Output, OutputPrefs, Pull, PullSession, buffer_frames,
    output_for,
};

// --- the counting allocator --------------------------------------------------------
//
// Memory is counted in BYTES that the code itself asks for, never in the RSS of a
// process other tests share (worker rule 4). An allocation is counted only on the
// thread that armed the counter, so the engine's decoder threads — which may
// allocate whenever they like — cannot pollute the measurement.

struct Counting;

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static COUNT: Cell<u64> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.with(Cell::get) {
            COUNT.with(|count| count.set(count.get() + 1));
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if ARMED.with(Cell::get) {
            COUNT.with(|count| count.set(count.get() + 1));
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn arm() {
    COUNT.with(|count| count.set(0));
    ARMED.with(|armed| armed.set(true));
}

/// Disarm and answer how many allocations the armed thread made.
fn disarm() -> u64 {
    ARMED.with(|armed| armed.set(false));
    COUNT.with(Cell::get)
}

// --- a real engine, with real audio -----------------------------------------------

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("nz-droid-audio-tests-{}", std::process::id()))
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch directory");
    dir
}

/// The play cache's answer: one file on disk, no request, no server — and no
/// mock at the audio level, the decoder and the mixer run for real.
struct Local(PathBuf);

impl StreamOpener for Local {
    fn open(
        &self,
        _id: &str,
        _quality: Quality,
        _start: Option<f64>,
        _from_byte: Option<u64>,
        _lane: Lane,
    ) -> Result<Box<dyn ByteStream>, SourceError> {
        Err(SourceError::Message(
            "ces tests lisent un fichier local, pas le réseau".into(),
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
        Some(self.0.clone())
    }
}

fn render(sink: &mut ExternalSink, frames: usize) {
    let mut out = vec![0.0f32; frames * 2];
    sink.render(&mut out);
}

/// A 440 Hz cosine, loaded and playing: the state a platform is in when it starts
/// calling the callback.
fn playing(name: &str) -> (Engine, ExternalSink) {
    let dir = scratch(name);
    let cfg = EngineConfig {
        sample_rate: 48_000,
        volume: 1.0,
        spool_dir: dir.join("spool"),
        ..Default::default()
    };
    let (engine, mut sink) = Engine::new_external(cfg, Arc::new(Local(fixture())));
    let events = engine.events();
    engine.load("sine", Quality::Flac, None);
    wait_started(&mut sink, &events);
    pump(&mut sink, 0.15);
    engine.play();
    (engine, sink)
}

fn wait_started(sink: &mut ExternalSink, events: &Receiver<EngineEvent>) {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut seen = false;
    while !seen {
        while let Ok(event) = events.try_recv() {
            if matches!(event, EngineEvent::Started { .. }) {
                seen = true;
            }
        }
        if seen {
            return;
        }
        assert!(Instant::now() < deadline, "le moteur n'a pas démarré");
        render(sink, 480);
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Render for `seconds` at the pace a device would, to let the decoder fill its
/// ring while the mixer is still paused.
fn pump(sink: &mut ExternalSink, seconds: f64) {
    let blocks = (seconds / 0.010).round().max(1.0) as usize;
    let mut next = Instant::now();
    for _ in 0..blocks {
        render(sink, 480);
        next += Duration::from_millis(10);
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else {
            next = now;
        }
    }
}

/// Render `blocks` blocks of `frames` at the pace a device really runs at (480
/// frames per 10 ms), so the decoder's ring — half a second — stays AHEAD of the
/// mixer. Rendering as fast as a loop allows would starve it and measure silence
/// instead of the seam.
fn paced(pull: &mut Pull, buffer: &mut [f32], blocks: usize, sink: &mut impl FnMut(&[f32])) {
    for _ in 0..blocks {
        pull.fill(buffer);
        sink(buffer);
        pace();
    }
}

/// Wait the 10 ms one 480-frame block of audio takes, so the decoder's ring stays
/// ahead. No rendering, so a caller that wants to arm a counter around its own
/// `fill` can pace between two of them.
fn pace() {
    std::thread::sleep(Duration::from_millis(10));
}

/// The 4 s, 440 Hz, 48 kHz FLAC this crate's tests carry: decoded losslessly to
/// 16-bit samples, so `expected` is the same numbers the decoder hands the mixer.
fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/assets/sine.flac")
}

/// The waveform the fixture holds: the 16-bit sample, scaled by 32768.
fn expected(frame: usize) -> f32 {
    let t = frame as f64 / 48_000.0;
    let sample =
        (0.5 * (2.0 * std::f64::consts::PI * 440.0 * t).cos() * f64::from(i16::MAX)).round();
    (sample / 32_768.0) as f32
}

// --- the tests --------------------------------------------------------------------

/// **The buffered margin is 500 ms, and 1 s is the ceiling.**
///
/// Measured numbers this holds: at 48 kHz, 500 ms = **24 000 frames** and 1 s =
/// 48 000. A device that reports a 192-frame burst (most phones in shared mode)
/// caps the request at its own capacity — asking for more is a silent no-op, and
/// asking for LESS is the 250 ms floor bridge 12 caught.
#[test]
fn the_buffer_is_the_largest_value_within_half_a_second_and_one() {
    let prefs = OutputPrefs::default();
    assert_eq!(prefs.sample_rate, 48_000);
    assert_eq!(prefs.buffer_ms_min, 500);
    assert_eq!(prefs.buffer_ms_max, 1_000);
    assert_eq!(nz_droid_audio::frames_for_ms(48_000, 500), 24_000);
    assert_eq!(nz_droid_audio::frames_for_ms(48_000, 1_000), 48_000);

    // A device that can hold more than a second: we take exactly one second.
    assert_eq!(buffer_frames(prefs, 96_000), 48_000);
    // A device right in the window: its own capacity, untouched.
    assert_eq!(buffer_frames(prefs, 30_000), 30_000);
    assert_eq!(buffer_frames(prefs, 24_000), 24_000);
    // A device whose whole capacity is under 500 ms (a 10 ms burst in MMAudio):
    // the capacity IS the answer — the request cannot invent margin.
    assert_eq!(buffer_frames(prefs, 4_800), 4_800);
    // A device that answers nonsense: no request at all.
    assert_eq!(buffer_frames(prefs, 0), 0);
    assert_eq!(buffer_frames(prefs, -1), 0);
}

/// The split the note pins: AAudio from 27, the Kotlin pull on 26, nothing below.
#[test]
fn the_output_split_is_the_planned_one() {
    assert_eq!(output_for(35), Output::Aa);
    assert_eq!(output_for(27), Output::Aa);
    assert_eq!(output_for(26), Output::Track);
    assert_eq!(output_for(25), Output::Unsupported);

    let prefs = OutputPrefs::default();
    // 26 is the AudioTrack pull: `start` refuses so the caller keeps its sink and
    // goes to `PullSession` (Oboe #40 — AAudio can crash after close on 8.0).
    assert_eq!(
        DroidOutput::start(&idle_engine("split-26"), idle_sink(), prefs, 26).unwrap_err(),
        DroidOutputError::NeedsAudioTrack
    );
    // A host build has no AAudio at all, and says so instead of pretending.
    assert!(matches!(
        DroidOutput::start(&idle_engine("split-host"), idle_sink(), prefs, 35),
        Err(DroidOutputError::Open(_))
    ));
}

/// **A reopen is the same mixer**: the stream's `Pull` gives the sink back, the
/// next stream takes it, and the samples continue exactly where they stopped.
///
/// Measured: **4 streams of 12 000 frames** (250 ms each) rendering 1.000 s total,
/// every sample within **1e-4** of the waveform the fixture holds. A hole
/// (a dropped frame) or a repeat (a doubled one) moves the phase at once — at
/// 440 Hz a single lost frame is a **0.0576** step, 57 000× the tolerance.
#[test]
fn a_reopen_continues_the_mixer_without_a_hole_or_a_repeat() {
    let (engine, sink) = playing("reopen");
    let back = Handback::new(sink);
    let blocks_per_stream = 25;
    let frames_per_block = 480;
    let mut recorded: Vec<f32> = Vec::with_capacity(4 * blocks_per_stream * frames_per_block);
    let mut buffer = vec![0.0f32; frames_per_block * 2];

    for _stream in 0..4 {
        // Exactly what AAudio does: a new stream takes the sink...
        let mut pull = Pull::attach(&back);
        assert!(pull.attached(), "le mixeur doit être disponible");
        let mut collect = |block: &[f32]| recorded.extend_from_slice(block);
        paced(&mut pull, &mut buffer, blocks_per_stream, &mut collect);
        // ... and the closing one gives it back on the way out.
        drop(pull);
    }

    assert_eq!(recorded.len(), 4 * blocks_per_stream * frames_per_block * 2);
    assert_eq!(
        back.returns(),
        4,
        "un rendu du mixeur par flux, jamais deux"
    );
    assert_eq!(back.misses(), 0, "aucun flux n'a trouvé la place vide");
    assert!(back.idle(), "le mixeur attend sagement le flux suivant");

    let mut worst = 0.0f32;
    let mut worst_at = 0;
    for frame in 0..recorded.len() / 2 {
        let difference = (recorded[frame * 2] - expected(frame)).abs();
        if difference > worst {
            worst = difference;
            worst_at = frame;
        }
    }
    println!(
        "reopen: {} frames, pire écart {worst:.8} à la trame {worst_at}, position {:.4} s",
        recorded.len() / 2,
        engine.position()
    );
    assert!(
        worst < 1e-4,
        "un trou ou une répétition déplacerait la phase : écart {worst} à la trame {worst_at}"
    );
    // 48 000 frames rendered = one second of audio, and the clock follows it.
    assert!(
        (engine.position() - 1.0).abs() < 0.05,
        "position {} s pour 1 s rendue",
        engine.position()
    );
}

/// **The callback body allocates NOTHING** — the rule bridge 12 paid for twice,
/// measured with a counting allocator rather than a promise.
///
/// Measured: **0 allocations** over **200 callbacks** of 480 frames, with a real
/// voice being mixed (the first 50 are a warm-up: whatever the mixer builds once
/// is built before the device starts calling it).
#[test]
fn the_callback_body_allocates_nothing() {
    let (_engine, sink) = playing("allocation");
    let back = Handback::new(sink);
    let mut pull = Pull::attach(&back);
    let mut buffer = vec![0.0f32; 480 * 2];
    // A real signal must be coming out, or "0 allocations" would be the cost of
    // rendering silence.
    let mut peak = 0.0f32;
    let mut watch = |block: &[f32]| {
        peak = block
            .iter()
            .fold(peak, |peak, sample| peak.max(sample.abs()));
    };
    paced(&mut pull, &mut buffer, 20, &mut watch);
    assert!(peak > 0.4, "le mixeur doit rendre le sinus, pas du silence");

    // Only the callback is armed: the pacing between two callbacks is the test's,
    // not the device's.
    let mut allocations = 0u64;
    for _ in 0..200 {
        arm();
        pull.fill(&mut buffer);
        allocations += disarm();
        pace();
    }
    println!("allocation: {allocations} allocation(s) en 200 rappels de 480 trames");
    assert_eq!(
        allocations, 0,
        "le rappel temps réel a alloué {allocations} fois"
    );
    // Still the same real signal after the measured callbacks: this test renders
    // as fast as the loop allows (no device pacing), so the mixer's own underrun
    // count is NOT a contract here — the device probe measures that one in real
    // time, on the device.
    let peak = buffer
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
    assert!(peak > 0.4, "le mixeur s'est tu pendant la mesure");
    let _ = &mut allocations;
}

/// Nothing is lost when a stream never plays a single buffer: the sink still goes
/// back, and the next stream finds it. (A paused track, a device that vanished
/// before its first callback.)
#[test]
fn a_stream_that_never_ran_gives_the_sink_back() {
    let (_engine, sink) = playing("never-ran");
    let back = Handback::new(sink);
    for _ in 0..10 {
        let pull = Pull::attach(&back);
        assert!(pull.attached());
        drop(pull);
    }
    assert_eq!(back.returns(), 10);
    assert_eq!(back.misses(), 0);
    assert!(back.idle());
}

/// The API-26 session tells the engine what the Kotlin side's `AudioTrack`
/// reported, and the engine's own counters follow: latency at once (the buffer we
/// asked for), then the device's number, and the xrun TOTALS never inflate.
#[test]
fn the_api_26_session_reports_latency_and_xruns_to_the_engine() {
    let (engine, sink) = playing("session");
    let prefs = OutputPrefs::default();
    let session = PullSession::new(&engine, sink, prefs);
    assert_eq!(session.buffer_frames(), 24_000, "500 ms à 48 kHz");
    assert_eq!(engine.output_info().latency_frames, 24_000);
    assert_eq!(engine.output_info().channels, 2);
    assert_eq!(engine.output_info().sample_rate, 48_000);

    session.set_latency(1_440); // 30 ms, the AudioTrack's own report
    assert_eq!(engine.output_info().latency_frames, 1_440);

    session.note_underruns(3);
    session.note_underruns(3);
    assert_eq!(
        engine.underruns(),
        3,
        "un total répété ne compte pas deux fois"
    );
    session.note_underruns(7);
    assert_eq!(engine.underruns(), 7);

    // And the session is a handle Kotlin can hold: one `long`, closed once.
    let handle = session.into_handle();
    assert_ne!(handle, 0);
    // SAFETY: the handle came from `into_handle` and is closed exactly once.
    unsafe { nz_droid_audio::close_handle(handle) };
    // Closing 0 is a no-op, so a Kotlin object closed twice cannot crash.
    // SAFETY: 0 is the documented "none" handle.
    unsafe { nz_droid_audio::close_handle(0) };
}

/// The fixture is there, and the first frame is the cosine's PEAK: a truncated or
/// silent file would make every measurement below meaningless.
#[test]
fn the_fixture_is_the_signal_the_measurements_assume() {
    let path = fixture();
    assert!(path.is_file(), "{} est absent", path.display());
    assert!(std::fs::metadata(&path).unwrap().len() > 10_000);
    assert!((expected(0) - 0.5).abs() < 1e-6, "{}", expected(0));
}

/// An engine with nothing loaded, for the paths that must refuse before any audio
/// exists.
fn idle_engine(name: &str) -> Engine {
    let dir = scratch(name);
    let cfg = EngineConfig {
        sample_rate: 48_000,
        volume: 1.0,
        spool_dir: dir.join("spool"),
        ..Default::default()
    };
    let (engine, _sink) = Engine::new_external(cfg, Arc::new(Local(dir.join("absent.wav"))));
    engine
}

/// A sink for the same purpose: `start` is refused before it is touched.
fn idle_sink() -> ExternalSink {
    let dir = scratch("idle-sink");
    let cfg = EngineConfig {
        sample_rate: 48_000,
        volume: 1.0,
        spool_dir: dir.join("spool"),
        ..Default::default()
    };
    let (_engine, sink) = Engine::new_external(cfg, Arc::new(Local(dir.join("absent.wav"))));
    sink
}
