//! D5 over the wire: the two `/api/stream` answers (a finished file with byte
//! ranges, a body generated as it is sent), and the seek policy they drive.
//!
//! The local server reproduces `neezer/webui/__init__.py`'s headers; the
//! opener is the app's side of [`StreamOpener`], implemented over a socket.

mod support;

use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::{Duration, Instant};

use nz_audio::{Engine, EngineConfig, EngineEvent, OfflineSink, Quality, StreamReady};
use support::*;

fn offline(
    dir: &std::path::Path,
    opener: TestOpener,
    read_ahead: u64,
) -> (Engine, OfflineSink, Receiver<EngineEvent>) {
    let cfg = EngineConfig {
        sample_rate: 48_000,
        spool_dir: dir.join("spool"),
        read_ahead_bytes: read_ahead,
        seek_poll_limit: Duration::from_secs(10),
        ..Default::default()
    };
    let (engine, sink) = Engine::new_offline(cfg, Arc::new(opener));
    let events = engine.events();
    (engine, sink, events)
}

/// The other half of D5's click budget: a first play from `perf_api` over
/// loopback, generated as it is sent (no length, no ranges), must be audible in
/// under 250 ms.
///
/// Measured: median **18.4 ms**, max 34.8 ms over 10 runs (`--nocapture` prints
/// the list).
#[test]
fn click_to_first_sample_over_the_network_is_under_250_ms() {
    let dir = scratch("click-network");
    let file = dir.join("dc.flac");
    dc_flac(&file, 60.0, 48_000);
    let server = TestServer::start(
        file.clone(),
        StreamReady {
            ready: false,
            streaming: true,
            ..StreamReady::default()
        },
        Body::Generated,
    );

    let mut samples = Vec::new();
    for _ in 0..10 {
        let (engine, mut sink, _events) = offline(&dir, TestOpener::http(&server), 2 * 1024 * 1024);
        let started = Instant::now();
        engine.load("network", Quality::Opus320, None);
        engine.play();
        let mut heard = None;
        while started.elapsed() < Duration::from_secs(3) {
            let block = render(&mut sink, 48);
            if block.iter().any(|sample| sample.abs() > 0.0) {
                heard = Some(started.elapsed());
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        if heard.is_none() {
            // A loader that never installed would be a silent failure: say why.
            let requests = server.requests();
            panic!("no audio within 3 s; requests seen: {requests:?}");
        }
        engine.pause();
        samples.push(heard.unwrap().as_secs_f64() * 1_000.0);
    }
    let median = median_ms(samples.clone());
    println!(
        "click-to-first-sample network: median {median:.1} ms, max {:.1} ms, all {samples:?}",
        samples.iter().cloned().fold(0.0, f64::max)
    );
    assert!(median < 250.0, "median {median:.1} ms must be under 250 ms");
}

/// A source the server can encode from (`/ready` says `offset`) is seeked by
/// **reopening it with `?start=<seconds>`** — one request, never a replay from
/// 0. Asserted on the request line the server actually received.
///
/// Measured: the seek produced exactly one request, `start=3.500`, and the
/// clock showed 3.500000 s.
#[test]
fn seek_in_a_generated_stream_uses_the_start_parameter() {
    let dir = scratch("seek-generated");
    let file = dir.join("dc.flac");
    dc_flac(&file, 60.0, 48_000);
    let server = TestServer::start(
        file.clone(),
        StreamReady {
            ready: false,
            offset: true,
            bytes: Some(std::fs::metadata(&file).unwrap().len()),
            ..StreamReady::default()
        },
        Body::Generated,
    );

    let (engine, mut sink, events) = offline(&dir, TestOpener::http(&server), 2 * 1024 * 1024);
    engine.load("g", Quality::Opus320, None);
    wait_for(&mut sink, &events, Duration::from_secs(5), |event| {
        matches!(event, EngineEvent::Started { .. })
    });
    engine.play();
    pump_for(&mut sink, 0.2);

    let before = server.requests().len();
    println!(
        "before seek: current_id={:?} pos={} requests={:?}",
        engine.current_id(),
        engine.position(),
        server.requests()
    );
    engine.seek(3.5);
    let mut landing = None;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        while let Ok(event) = events.try_recv() {
            if let EngineEvent::Seeked { position_s } = event {
                landing = Some(position_s);
            }
        }
        if landing.is_some() {
            break;
        }
        pump(&mut sink, 48);
        thread::sleep(Duration::from_millis(1));
    }
    let landing = landing.expect("the seek landed");
    let position = engine.position();
    let requests = server.requests();
    let opened: Vec<_> = requests[before..]
        .iter()
        .filter(|request| request.path.contains("/api/stream/"))
        .map(|request| request.path.clone())
        .collect();
    println!(
        "seek to 3.5 s: reported {landing:.6}, clock {position:.6}, stream requests {opened:?}"
    );
    assert_eq!(
        opened.len(),
        1,
        "a `?start=` seek is ONE request, got {opened:?}"
    );
    assert!(
        opened[0].contains("start=3.500"),
        "the request must slice at 3.500 s: {}",
        opened[0]
    );
    assert!(
        (position - 3.5).abs() < 0.020,
        "the clock must follow the slice: {position:.6}"
    );
}

/// A stream that cannot be seeked yet is **deferred, not restarted**, and the
/// engine keeps asking the cheap `/ready` route (1 s, 1.5 s while `streaming`)
/// until the answer changes — `seekplan.js`'s rule.
///
/// Measured: `SeekDeferred` at once, first new stream request only after the
/// server said `offset`, `Seeked` 1.5 s later, 0 stream requests before that.
#[test]
fn a_seek_that_cannot_land_is_deferred_then_lands() {
    let dir = scratch("seek-deferred");
    let file = dir.join("dc.flac");
    dc_flac(&file, 60.0, 48_000);
    let server = TestServer::start(
        file.clone(),
        StreamReady {
            ready: false,
            streaming: true,
            ..StreamReady::default()
        },
        Body::Generated,
    );

    let (engine, mut sink, events) = offline(&dir, TestOpener::http(&server), 2 * 1024 * 1024);
    engine.load("d", Quality::Opus320, None);
    wait_for(&mut sink, &events, Duration::from_secs(5), |event| {
        matches!(event, EngineEvent::Started { .. })
    });
    engine.play();
    pump_for(&mut sink, 0.2);

    let before = server.requests().len();
    engine.seek(2.0);
    let mut deferred = false;
    let mut landed = false;
    let mut streams_before_answer = 0;
    let started = Instant::now();
    let mut answered = false;
    while started.elapsed() < Duration::from_secs(8) {
        while let Ok(event) = events.try_recv() {
            match event {
                EngineEvent::SeekDeferred { target_s } => {
                    assert!((target_s - 2.0).abs() < 1e-9);
                    deferred = true;
                }
                EngineEvent::Seeked { .. } => landed = true,
                _ => {}
            }
        }
        if !answered && started.elapsed() > Duration::from_millis(300) {
            // The server finishes writing the archive: now it can slice.
            server.set_ready(StreamReady {
                ready: false,
                offset: true,
                streaming: false,
                ..StreamReady::default()
            });
            answered = true;
        }
        if !answered {
            streams_before_answer = server
                .requests()
                .iter()
                .skip(before)
                .filter(|request| {
                    request.path.contains("/api/stream/") && request.path.contains("start=")
                })
                .count();
        }
        if landed {
            break;
        }
        pump(&mut sink, 48);
        thread::sleep(Duration::from_millis(1));
    }
    let paths: Vec<_> = server
        .requests()
        .iter()
        .skip(before)
        .map(|request| request.path.clone())
        .collect();
    println!("deferred seek: deferred={deferred} landed={landed} requests={paths:?}");
    assert!(deferred, "the engine must say the seek is deferred");
    assert_eq!(
        streams_before_answer, 0,
        "no stream request may be made while the server cannot answer"
    );
    assert!(landed, "the seek must land once the server can answer");
    assert!(
        paths.iter().any(|path| path.contains("start=2.000")),
        "the landed seek must use ?start=: {paths:?}"
    );
}

/// A seek in a **finished network file** is answered by a byte range: the
/// source restarts its request at the byte the container asked for (`Range`),
/// and the decoded samples are the file's at the target.
///
/// Measured: 1 `Range: bytes=…` request, landing 6.000000 s, sample error
/// 1.1e-5 against the generated cosine.
#[test]
fn seek_in_a_finished_network_file_uses_a_byte_range() {
    let dir = scratch("seek-range");
    let file = dir.join("ramp.flac");
    let freq = 440.0;
    cosine_flac(&file, freq, 10.0, 48_000);
    let len = std::fs::metadata(&file).unwrap().len();
    let server = TestServer::start(
        file.clone(),
        StreamReady {
            ready: true,
            ranges: Some("bytes".into()),
            bytes: Some(len),
            offset: false,
            ..StreamReady::default()
        },
        Body::Finished,
    );

    // 64 KiB of read-ahead: the seek target is far past it, so the source must
    // ask the server for the rest instead of having it already.
    let (engine, mut sink, events) = offline(&dir, TestOpener::http(&server), 64 * 1024);
    engine.load("f", Quality::Flac, None);
    wait_for(&mut sink, &events, Duration::from_secs(5), |event| {
        matches!(event, EngineEvent::Started { .. })
    });
    engine.play();
    pump_for(&mut sink, 0.2);

    let before = server.requests().len();
    engine.seek(6.0);
    let mut landing = None;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        while let Ok(event) = events.try_recv() {
            if let EngineEvent::Seeked { position_s } = event {
                landing = Some(position_s);
            }
        }
        if landing.is_some() {
            break;
        }
        pump(&mut sink, 48);
        thread::sleep(Duration::from_millis(1));
    }
    let landing = landing.expect("the seek landed");
    let position = engine.position();
    // The clock lands when the mixer installs the new voice; its ring may still
    // be empty while the decode thread fetches the range (five requests for a
    // 10 s FLAC, and a CI runner takes longer over them than a laptop). Wait for
    // the sound, then measure the block that follows it — the position of that
    // block's first sample is what the clock reads at that instant.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && render(&mut sink, 48).iter().all(|sample| *sample == 0.0) {
        thread::sleep(Duration::from_millis(1));
    }
    let block_position = engine.position();
    let ranges: Vec<_> = server
        .requests()
        .iter()
        .skip(before)
        .filter_map(|request| request.header("Range").map(|range| range.to_string()))
        .collect();
    println!("finished-file seek: reported {landing:.6}, clock {position:.6}, ranges {ranges:?}");
    assert!(
        !ranges.is_empty(),
        "a finished file is seeked with a byte range, got no Range request"
    );
    assert!(
        (position - 6.0).abs() < 0.020,
        "the seek must land within 20 ms of 6.0 s, got {position:.6}"
    );
    let block = render(&mut sink, 480);
    let mut max_error = 0.0f64;
    for frame in 0..480 {
        let time = block_position + frame as f64 / 48_000.0;
        let expected = 0.5 * (std::f64::consts::TAU * freq * time).cos();
        max_error = max_error.max((f64::from(block[frame * 2]) - expected).abs());
    }
    println!("finished-file seek content error = {max_error:.6}");
    assert!(
        max_error < 0.001,
        "the bytes fetched by range must be the file at 6 s, max error {max_error}"
    );
}
