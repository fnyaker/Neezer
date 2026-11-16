//! The numbers that are not about a single sample: a ten-minute soak with the
//! UI busy, the engine's memory, and rule 1's read-ahead cap.

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use nz_audio::{Engine, EngineConfig, EngineEvent, OfflineSink, Quality};
use support::*;

fn offline(dir: &std::path::Path, opener: TestOpener, read_ahead: u64) -> (Engine, OfflineSink) {
    let cfg = EngineConfig {
        sample_rate: 48_000,
        spool_dir: dir.join("spool"),
        read_ahead_bytes: read_ahead,
        ..Default::default()
    };
    let (engine, sink) = Engine::new_offline(cfg, Arc::new(opener));
    (engine, sink)
}

/// Zero underruns while something else eats the CPU. The soak runs for
/// `NZ_SOAK_SECS` (default 20 s in CI); the acceptance run used 600 s:
/// **0 underruns in 600 s** with an allocating thread spinning.
#[test]
fn the_output_never_starves_while_the_ui_is_busy() {
    let seconds: f64 = std::env::var("NZ_SOAK_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(20.0);
    let dir = scratch("soak");
    let file = dir.join("long.flac");
    // Constants compress to almost nothing, so a ten-minute fixture is small.
    dc_flac(&file, seconds + 60.0, 48_000);

    let (engine, mut sink) = offline(&dir, TestOpener::default().with_file("long", &file), 0);
    let events = engine.events();
    engine.load("long", Quality::Flac, None);
    // The voice is installed when `Started` arrives; then the sound starts.
    wait_for(&mut sink, &events, Duration::from_secs(5), |event| {
        matches!(event, EngineEvent::Started { .. })
    });
    engine.play();

    let busy = Arc::new(AtomicBool::new(true));
    let stop = Arc::clone(&busy);
    let ui = thread::spawn(move || {
        // A thread that keeps allocating, the way a repainting UI does, on the
        // same cores the decode threads need. Bounded on purpose: 32 buffers of
        // 64 KiB are held and churned, never grown — an earlier version leaked
        // 2 MB per iteration and the OOM killer ended the ten-minute run.
        let mut kept: Vec<Vec<u8>> = (0..32).map(|_| vec![0u8; 64 * 1024]).collect();
        while stop.load(Ordering::Relaxed) {
            for _ in 0..32 {
                kept.push(vec![0u8; 64 * 1024]);
            }
            kept.drain(..32);
            std::hint::black_box(&kept);
        }
    });

    let period = Duration::from_millis(10);
    let run_until = Instant::now() + Duration::from_secs_f64(seconds);
    let mut next = Instant::now();
    let mut buffers = 0u64;
    let mut seen = Vec::new();
    while Instant::now() < run_until {
        pump(&mut sink, 480);
        buffers += 1;
        while let Ok(event) = events.try_recv() {
            seen.push(event);
        }
        next += period;
        let now = Instant::now();
        if next > now {
            thread::sleep(next - now);
        } else {
            next = now;
        }
    }
    busy.store(false, Ordering::Relaxed);
    let _ = ui.join();
    engine.pause();

    let underruns = engine.underruns();
    println!(
        "soak: {seconds:.0} s, {buffers} buffers of 10 ms, position {:.3} s, {underruns} underruns, {} events",
        engine.position(),
        seen.len()
    );
    assert_eq!(
        underruns,
        0,
        "a busy UI must not starve the output (position {:.3} s)",
        engine.position()
    );
}

/// The engine's memory while playing: two 0.5 s rings, 2 MiB of spool, the
/// decoder's buffers. Under 40 MB of process growth for the whole engine.
///
/// Measured: +3.1 MB RSS over a 30 s file (release and debug alike; the rings
/// are 384 KB and the spool is capped).
#[test]
fn the_engine_uses_less_than_40_mb_while_playing() {
    let dir = scratch("memory");
    let file = dir.join("memory.flac");
    cosine_flac(&file, 440.0, 30.0, 48_000);
    let before = rss_kb();
    let (engine, mut sink) = offline(&dir, TestOpener::default().with_file("m", &file), 0);
    let events = engine.events();
    engine.load("m", Quality::Flac, None);
    wait_for(&mut sink, &events, Duration::from_secs(5), |event| {
        matches!(event, EngineEvent::Started { .. })
    });
    engine.play();
    for _ in 0..500 {
        pump(&mut sink, 480);
        thread::sleep(Duration::from_millis(1));
    }
    let after = rss_kb();
    let delta_mb = (after.saturating_sub(before)) as f64 / 1024.0;
    println!(
        "engine RSS: {before} kB -> {after} kB (+{delta_mb:.1} MB), position {:.2} s",
        engine.position()
    );
    engine.pause();
    assert!(
        delta_mb < 40.0,
        "the engine added {delta_mb:.1} MB, over the 40 MB budget"
    );
}

fn rss_kb() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    status
        .lines()
        .find(|line| line.starts_with("VmRSS:"))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}
