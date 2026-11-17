//! SC2's acceptance: the EXTERNAL sink, driven by a platform's own callback.
//!
//! What a phone does on AAudio is what these tests do with a loop: `render` is
//! called with the device's buffer, in real time, and the engine's clock is the
//! same one cpal's callback writes. So the gapless seam is an assertion here and
//! not a race with a sound card, and the latency the platform reports is a
//! number the position can be checked against.
//!
//! Each assertion carries the number it was measured at (Rust 1.99.1, 16
//! threads); `--nocapture` prints them, which is how the PR's table was filled.

mod support;

use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use nz_audio::{Engine, EngineConfig, EngineEvent, ExternalSink, OutputInfo, Quality};
use support::*;

/// The engine a platform builds: no device of its own, the mixer in our hands.
fn external(
    dir: &std::path::Path,
    opener: TestOpener,
    volume: f32,
) -> (Engine, ExternalSink, Receiver<EngineEvent>) {
    let cfg = EngineConfig {
        sample_rate: 48_000,
        volume,
        spool_dir: dir.join("spool"),
        ..Default::default()
    };
    let (engine, sink) = Engine::new_external(cfg, Arc::new(opener));
    let events = engine.events();
    (engine, sink, events)
}

/// Load a track, let the decoder fill its ring while the mixer is paused, and
/// hand back the running engine: exactly the state a platform is in when it
/// starts calling `render`.
fn loaded(
    dir: &std::path::Path,
    opener: TestOpener,
    id: &str,
    quality: Quality,
) -> (Engine, ExternalSink, Receiver<EngineEvent>) {
    let (engine, mut sink, events) = external(dir, opener, 1.0);
    engine.load(id, quality, None);
    wait_for(
        &mut sink,
        &events,
        Duration::from_secs(5),
        |event| matches!(event, EngineEvent::Started { id: started, .. } if started == id),
    );
    pump_for(&mut sink, 0.15);
    (engine, sink, events)
}

/// **The seam a platform output gets is the desktop's seam, to the sample.**
///
/// `ExternalSink::render` is the call cpal's callback makes, so two FLACs cut
/// out of one 2 s cosine must come out sample-identical to the whole file, with
/// the second track's first frame starting in the SAME buffer as the first
/// track's last — no restart, no padding, no gap.
///
/// Measured: max difference from the reference **0.0000000** over 96 000 frames,
/// seam Δ **0.00083** against a sine step of 0.0576, frame 96 000 exactly 0.
#[test]
fn the_external_sink_hands_over_gaplessly_and_sample_exactly() {
    let dir = scratch("external-seam");
    let (first, second, whole) = split_cosine(&dir, 440.0, 48_000);

    // The reference: the same two seconds decoded as one file, through the same
    // kind of sink.
    let (reference_engine, mut reference_sink, _reference_events) = loaded(
        &dir,
        TestOpener::default().with_file("whole", &whole),
        "whole",
        Quality::Flac,
    );
    reference_engine.play();
    let reference = pump_for(&mut reference_sink, 2.05);
    reference_engine.pause();
    reference_engine.shutdown();

    let (engine, mut sink, events) = loaded(
        &dir,
        TestOpener::default()
            .with_file("a", &first)
            .with_file("b", &second),
        "a",
        Quality::Flac,
    );
    engine.preload_next("b", Quality::Flac);
    // The preloader opens, probes and starts decoding the next voice; give it
    // the same 150 ms the first voice got (the engine has no "next is ready"
    // event yet, D7's queue will model it).
    pump_for(&mut sink, 0.15);
    engine.play();
    let mut out = pump_for(&mut sink, 1.5);
    assert_eq!(
        engine.current_id().as_deref(),
        Some("b"),
        "after the handover the engine must name the track being heard"
    );
    out.extend(pump_for(&mut sink, 0.55));
    engine.pause();

    let mut started_b = false;
    let mut ended_a = false;
    let mut ended_b = false;
    while let Ok(event) = events.try_recv() {
        match event {
            EngineEvent::Started { id, .. } if id == "b" => started_b = true,
            EngineEvent::Ended { id } if id == "a" => ended_a = true,
            EngineEvent::Ended { id } if id == "b" => ended_b = true,
            _ => {}
        }
    }
    assert!(started_b, "the preloaded track must be the one that plays");
    assert!(!ended_a, "a must hand over, not end in silence");
    assert!(ended_b, "the second track must end the queue");

    // 1. The seam itself, at frame 48 000 (one second in).
    let step = std::f64::consts::TAU * 440.0 / 48_000.0;
    let before = out[(48_000 - 1) * 2];
    let after = out[48_000 * 2];
    let delta = (after - before).abs();
    println!("seam delta = {delta:.6} (one sine step = {step:.6})");
    assert!(
        f64::from(delta) < step,
        "the seam ({delta:.6}) must be smaller than one step of the waveform ({step:.6})"
    );

    // 2. Nothing added, nothing missing: identical to the whole file.
    let mut max_diff = 0.0f32;
    for (a, b) in out.iter().zip(reference.iter()).take(96_000 * 2) {
        max_diff = max_diff.max((a - b).abs());
    }
    println!("max difference over 96 000 frames = {max_diff:.7}");
    assert!(
        max_diff < 1e-4,
        "the two halves must be the whole file's samples, max diff {max_diff}"
    );

    // 3. The length is the sum of both, ± one frame.
    assert!(
        out[(96_000 - 1) * 2].abs() > 0.4,
        "frame 95 999 must still be the second track's audio"
    );
    assert_eq!(
        out[96_000 * 2],
        0.0,
        "frame 96 000 is where the sum of both ends: silence"
    );
    engine.shutdown();
}

/// **The position is what was rendered minus the latency the platform reports.**
///
/// AAudio knows the size of its own buffer, and that is the only difference
/// between the phone's clock and the desktop's: `position()` must be
/// `rendered_frames − latency_frames`, so the bar, the party and the analysis
/// agree with what is leaving the device. A route change (a headset, a Bluetooth
/// sink) calls `set_external_output` again and the number follows.
///
/// Measured: 4 800 frames rendered with a 1 440-frame (30 ms) latency report →
/// position **0.070000 s** exactly; after a route change to 480 frames →
/// **0.090000 s**.
#[test]
fn the_position_is_the_rendered_frames_minus_the_reported_latency() {
    let dir = scratch("external-latency");
    let file = dir.join("dc.flac");
    dc_flac(&file, 20.0, 48_000);

    let (engine, mut sink, events) = loaded(
        &dir,
        TestOpener::default().with_file("dc", &file),
        "dc",
        Quality::Flac,
    );

    // The platform opens its device: 30 ms of buffer at 48 kHz.
    let latency_frames = 1_440u64;
    engine.set_external_output(OutputInfo {
        device: "AAudio (test)".into(),
        sample_rate: 48_000,
        channels: 2,
        latency_frames,
    });
    // The app hears about it through the same event the desktop's output emits.
    let mut announced = None;
    while let Ok(event) = events.try_recv() {
        if let EngineEvent::OutputChanged { device, .. } = event {
            announced = Some(device);
        }
    }
    assert_eq!(
        announced.as_deref(),
        Some("AAudio (test)"),
        "the platform's device must be announced to the app"
    );
    assert_eq!(engine.output_info().latency_frames, latency_frames);

    // Paused, nothing has been heard yet: the latency clamps at 0.
    assert_eq!(engine.position(), 0.0);

    engine.play();
    let blocks = pump_for(&mut sink, 0.1); // 10 buffers of 480 frames
    engine.pause();
    assert_eq!(blocks.len(), 4_800 * 2, "10 buffers of 10 ms each");
    assert!(
        blocks.iter().all(|sample| sample.abs() > 0.2),
        "every rendered frame must be the track's audio, not a starved silence"
    );
    let rendered = 4_800.0;
    let expected = (rendered - latency_frames as f64) / 48_000.0;
    let position = engine.position();
    println!("position = {position:.6} s, expected {expected:.6} s");
    assert!(
        (position - expected).abs() < 1e-9,
        "position must be rendered minus latency: expected {expected:.6}, got {position:.6}"
    );

    // A route change reports the new buffer; the same audio is now 10 ms behind.
    engine.set_external_output(OutputInfo {
        device: "casque (test)".into(),
        sample_rate: 48_000,
        channels: 2,
        latency_frames: 480,
    });
    let position = engine.position();
    let expected = (rendered - 480.0) / 48_000.0;
    println!("position after the route change = {position:.6} s");
    assert!(
        (position - expected).abs() < 1e-9,
        "a route change must move the latency the clock subtracts: expected {expected:.6}, got \
         {position:.6}"
    );
    engine.shutdown();
}

/// **The device's xrun count is a total, and it is added to the mixer's own.**
///
/// AAudio's `getXRunCount` and `AudioTrack`'s `getUnderrunCount` are totals since
/// the device opened; the platform hands them over as they are. Repeating a
/// reading changes nothing (a poll every second would otherwise inflate the
/// number the person reads), and what the mixer itself counted still adds up:
/// together they are what was heard stutter.
#[test]
fn the_platforms_xrun_total_is_added_to_the_mixers_own_count() {
    let dir = scratch("external-underruns");
    let file = dir.join("dc.flac");
    dc_flac(&file, 20.0, 48_000);

    let (engine, sink, _events) = external(&dir, TestOpener::default().with_file("dc", &file), 1.0);
    assert_eq!(
        engine.underruns(),
        0,
        "a fresh engine heard nothing stutter"
    );

    engine.note_underruns(3);
    assert_eq!(engine.underruns(), 3);
    // The platform repeats its reading: it is the same total, not three more.
    engine.note_underruns(3);
    assert_eq!(
        engine.underruns(),
        3,
        "a reading repeated is not a new xrun"
    );
    engine.note_underruns(7);
    assert_eq!(engine.underruns(), 7);
    assert_eq!(sink.shared().device_underruns(), 7);
    assert_eq!(
        sink.shared().underruns(),
        7,
        "the sink's own view is the same total"
    );
    engine.shutdown();
}

/// **The sink moves into the platform's own callback thread and renders there.**
///
/// AAudio calls its data callback on a real-time thread the app does not own;
/// `AudioTrack` pulls from a thread of its own. So the sink must be `Send`, and
/// the clock the app reads must be the one that thread wrote — this is the same
/// object, not a copy. (The compiler proves `Send` by accepting `thread::spawn`
/// with the sink moved in.)
///
/// Measured: 10 buffers of 480 frames rendered on another thread, peak
/// **0.2500** (the DC file's level, not a starved zero), clock advanced by
/// **0.100 s**.
#[test]
fn the_sink_moves_into_a_platform_callback_thread_and_renders_there() {
    let dir = scratch("external-thread");
    let file = dir.join("dc.flac");
    dc_flac(&file, 5.0, 48_000);

    let (engine, mut sink, _events) = loaded(
        &dir,
        TestOpener::default().with_file("dc", &file),
        "dc",
        Quality::Flac,
    );
    let shared = sink.shared();
    engine.play();

    let (tx, rx) = std::sync::mpsc::channel();
    let callback = std::thread::Builder::new()
        .name("nz-test-callback".into())
        .spawn(move || {
            let mut out = vec![0.0f32; 480 * 2];
            let mut peak = 0.0f32;
            for _ in 0..10 {
                sink.render(&mut out);
                for sample in &out {
                    peak = peak.max(sample.abs());
                }
            }
            tx.send(peak).expect("report the peak");
            sink
        })
        .expect("the sink is Send: a platform callback takes it");
    let peak = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the callback thread rendered");
    let sink = callback.join().expect("the callback thread finished");
    engine.pause();
    println!(
        "peak {peak:.4} on the callback thread, clock {:.6} s",
        shared.clock.position()
    );
    assert!(peak > 0.2, "the callback rendered the track, not silence");
    assert!(
        (shared.clock.position() - 0.1).abs() < 1e-9,
        "the app reads the clock the callback wrote: {:.6} s",
        shared.clock.position()
    );
    assert!(Arc::ptr_eq(&shared, &sink.shared()), "one clock, not a copy");
    engine.shutdown();
}

/// **With the `cpal` feature off, `nz-audio` opens nothing and says so.**
///
/// That is the whole point of the feature: on Android there is no PipeWire host
/// and no ALSA host to name, so the crate must compile without them (measured
/// before SC2: 4 × E0599 in `output.rs`), enumerate nothing, and run on the
/// silence pump until a platform hands it an [`ExternalSink`]. The device list
/// belongs to the platform then, and an empty answer is the honest one.
#[cfg(not(feature = "cpal"))]
#[test]
fn without_cpal_the_engine_lists_no_device_and_runs_on_its_pump() {
    assert!(
        nz_audio::output_devices().is_empty(),
        "no host to enumerate: the platform owns the list"
    );
    let dir = scratch("external-no-cpal");
    let cfg = EngineConfig {
        sample_rate: 48_000,
        spool_dir: dir.join("spool"),
        ..Default::default()
    };
    let engine = Engine::new(cfg, Arc::new(TestOpener::default()));
    let info = engine.output_info();
    assert_eq!(info.device, "sans sortie audio");
    assert_eq!(info.sample_rate, 48_000);
    assert!(
        engine.set_output_device(None).is_err(),
        "there is no device to switch to without cpal"
    );
    engine.shutdown();
}
