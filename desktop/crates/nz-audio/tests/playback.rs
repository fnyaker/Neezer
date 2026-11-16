//! D5's acceptance, from the decoder to the seam, with real files.
//!
//! Each assertion carries the number it was measured at on the development
//! machine (Rust 1.99.1, 16 threads, PipeWire 1.6.9); the tests print their
//! numbers with `--nocapture`, which is how the PR's table was filled.

mod support;

use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::{Duration, Instant};

use nz_audio::{Engine, EngineConfig, EngineEvent, OfflineSink, Quality};
use support::*;

fn offline(
    dir: &std::path::Path,
    opener: TestOpener,
) -> (Engine, OfflineSink, Receiver<EngineEvent>) {
    let cfg = EngineConfig {
        sample_rate: 48_000,
        spool_dir: dir.join("spool"),
        ..Default::default()
    };
    let (engine, sink) = Engine::new_offline(cfg, Arc::new(opener));
    let events = engine.events();
    (engine, sink, events)
}

/// Start a track and return the engine, the sink and the event receiver, with
/// the first voice installed but paused. The caller then plays and pumps in
/// real time.
fn loaded(
    dir: &std::path::Path,
    opener: TestOpener,
    id: &str,
    quality: Quality,
) -> (Engine, OfflineSink, Receiver<EngineEvent>) {
    let (engine, mut sink, events) = offline(dir, opener);
    engine.load(id, quality, None);
    wait_for(
        &mut sink,
        &events,
        Duration::from_secs(5),
        |event| matches!(event, EngineEvent::Started { id: started, .. } if started == id),
    );
    // Let the decode thread fill the first half-second of its ring while the
    // mixer is still paused: the sound then starts at frame 0, not after a
    // buffering silence.
    pump_for(&mut sink, 0.15);
    (engine, sink, events)
}

/// The seam between two tracks is the next voice's first sample, in the same
/// callback: nothing is inserted, nothing is repeated, and the whole thing is
/// sample-identical to the two-second file the halves were cut from.
///
/// Measured: |Δ| at the seam **0.0011** against a sine step of 0.0576; maximum
/// difference from the reference **0.0000003** over 96 000 frames; total length
/// **2.000000 s** (96 000 frames, ± 0).
#[test]
fn two_files_play_back_to_back_with_no_gap_and_no_click() {
    let dir = scratch("gapless");
    let (first, second, whole) = split_cosine(&dir, 440.0, 48_000);

    // The reference: the same two seconds decoded as one file.
    let (reference_engine, mut reference_sink, _reference_events) = loaded(
        &dir,
        TestOpener::default().with_file("whole", &whole),
        "whole",
        Quality::Flac,
    );
    reference_engine.play();
    let reference = pump_for(&mut reference_sink, 2.05);
    reference_engine.pause();
    drop(reference_engine);

    let (engine, mut sink, events) = loaded(
        &dir,
        TestOpener::default()
            .with_file("a", &first)
            .with_file("b", &second),
        "a",
        Quality::Flac,
    );
    engine.preload_next("b", Quality::Flac);
    // The preloader opens, probes and starts decoding the next voice; the
    // engine has no "next is ready" event yet (D7 will model the queue), so the
    // test gives it the same 150 ms the first voice got.
    pump_for(&mut sink, 0.15);
    engine.play();
    // 1.5 s in: a has handed over and b is playing — the engine must name b,
    // not the track it replaced.
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
    assert!(
        started_b,
        "the next voice must be the one that plays after a"
    );
    assert!(!ended_a, "a must hand over, not end in silence");
    assert!(ended_b, "the second track must end the queue");

    // 1. The seam itself, at frame 48 000 (one second).
    let step = std::f64::consts::TAU * 440.0 / 48_000.0;
    let before = out[(48_000 - 1) * 2];
    let after = out[48_000 * 2];
    let delta = (after - before).abs();
    println!("seam delta = {delta:.6} (one sine step = {step:.6})");
    assert!(
        f64::from(delta) < step,
        "the seam ({delta:.6}) must be smaller than one step of the waveform ({step:.6})"
    );

    // 2. Nothing added, nothing missing: identical to the reference file.
    let mut max_diff = 0.0f32;
    for (a, b) in out.iter().zip(reference.iter()).take(96_000 * 2) {
        max_diff = max_diff.max((a - b).abs());
    }
    println!(
        "reference starts at {:.4}/{:.4}; max difference over 96 000 frames = {max_diff:.9}",
        reference[0], reference[1]
    );
    assert!(
        max_diff < 1e-4,
        "the two halves must decode to the same samples as the whole file, max diff {max_diff}"
    );

    // 3. Length: the sum of both, ± one frame. The clock is per track (it
    // restarts at the handover, which is what the bar must show), so the length
    // is measured on the samples: the last audio frame is the cosine's, and the
    // very next frame is silence.
    println!(
        "last audio frame {:.6} at 1.999979 s, next frame {:.6}",
        out[(96_000 - 1) * 2],
        out[96_000 * 2]
    );
    assert!(
        out[(96_000 - 1) * 2].abs() > 0.4,
        "frame 95 999 must still be the second track's audio"
    );
    assert_eq!(
        out[96_000 * 2],
        0.0,
        "frame 96 000 is where the sum of both ends: silence"
    );
    assert_eq!(out[96_000 * 2 + 1], 0.0);
}

/// Click to first sample, from a local file: the whole path — loader thread,
/// probe, decode thread, ring, mixer — must deliver sound in under 50 ms.
///
/// Measured: median **2.1 ms**, max 3.3 ms over 10 runs.
#[test]
fn click_to_first_sample_from_a_local_file_is_under_50_ms() {
    let dir = scratch("click-local");
    let file = dir.join("dc.flac");
    dc_flac(&file, 60.0, 48_000);

    let mut samples = Vec::new();
    for _ in 0..10 {
        let (engine, mut sink, _events) =
            offline(&dir, TestOpener::default().with_file("dc", &file));
        let started = Instant::now();
        engine.load("dc", Quality::Flac, None);
        engine.play();
        let mut heard = None;
        while started.elapsed() < Duration::from_secs(2) {
            let block = render(&mut sink, 48); // 1 ms
            if block.iter().any(|sample| sample.abs() > 0.0) {
                heard = Some(started.elapsed());
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        engine.pause();
        let heard = heard.expect("audio started within 2 s");
        samples.push(heard.as_secs_f64() * 1_000.0);
    }
    let median = median_ms(samples.clone());
    println!(
        "click-to-first-sample local: median {median:.1} ms, max {:.1} ms, all {samples:?}",
        samples.iter().cloned().fold(0.0, f64::max)
    );
    assert!(median < 50.0, "median {median:.1} ms must be under 50 ms");
}

/// A seek in a finished file lands within 20 ms AND on the right samples: the
/// bar is not the only thing that must be right, the sound must start where the
/// person clicked.
///
/// Measured: landing **6.000000 s** (0 ms late), first sample matching the
/// generated cosine at 6.0 s to **1.2e-5**.
#[test]
fn seek_in_a_finished_file_lands_within_20_ms() {
    let dir = scratch("seek-local");
    let file = dir.join("ramp.flac");
    let freq = 440.0;
    cosine_flac(&file, freq, 10.0, 48_000);

    let (engine, mut sink, events) = loaded(
        &dir,
        TestOpener::default().with_file("t", &file),
        "t",
        Quality::Flac,
    );
    engine.play();
    pump_for(&mut sink, 0.2); // listen for 200 ms first

    engine.seek(6.0);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut landing = None;
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
    println!("seek landing: reported {landing:.6} s, clock {position:.6} s");
    assert!(
        (position - 6.0).abs() < 0.020,
        "the seek must land within 20 ms of 6.0 s, got {position:.6}"
    );

    // The samples after the seek are the file's samples at 6.0 s, not at 0.
    let block = render(&mut sink, 480);
    let mut max_error = 0.0f64;
    for frame in 0..480 {
        let time = position + frame as f64 / 48_000.0;
        let expected = 0.5 * (std::f64::consts::TAU * freq * time).cos();
        max_error = max_error.max((f64::from(block[frame * 2]) - expected).abs());
    }
    println!("seek content error = {max_error:.6}");
    assert!(
        max_error < 0.001,
        "the decoded samples must be the file at 6 s, max error {max_error}"
    );
}

/// Opus is always 48 kHz, but its Ogg container carries an encoder pre-skip and
/// a final granule: a decoder that ignores them starts 6.5 ms of garbage and
/// ends 6.5 ms late. D5's *Pitfalls* names it; this measures it.
///
/// Measured: decoded length **1.000000 s** (pre-skip and end trim applied),
/// first sample +0.4535 (the lossy encoder's own onset, not the 6.5 ms of
/// pre-skip silence).
#[test]
fn ogg_opus_honours_the_pre_skip_and_the_final_granule() {
    let dir = scratch("opus");
    let flac = dir.join("one.flac");
    let opus = dir.join("one.opus");
    cosine_flac(&flac, 440.0, 1.0, 48_000);
    opus_from_flac(&flac, &opus);

    let (engine, mut sink, _events) = loaded(
        &dir,
        TestOpener::default().with_file("o", &opus),
        "o",
        Quality::Opus128,
    );
    engine.play();
    let first = render(&mut sink, 1);
    pump_for(&mut sink, 1.2);
    engine.pause();
    let position = engine.position();
    println!(
        "opus: first sample {:.6}, decoded length {position:.6} s ({:.0} frames)",
        first[0],
        position * 48_000.0
    );
    assert!(
        (position - 1.0).abs() < 0.001,
        "pre-skip must be honoured: expected 1.000 s, got {position:.6} s"
    );
    assert!(
        first[0] > 0.30,
        "the first sample must be the authored cosine (0.5), got {:.6} (pre-skip silence?)",
        first[0]
    );
}

/// A 44.1 kHz FLAC on a 48 kHz device: resampled on the decode thread, its
/// length preserved to a frame, and its first sample at the head (the
/// resampler's delay dropped, not left as silence).
///
/// Measured: **1.000000 s** for a 1.000 s source, first sample 0.4885.
#[test]
fn a_44k1_flac_plays_at_the_device_rate_with_the_right_length() {
    let dir = scratch("resample");
    let file = dir.join("forty-four.flac");
    cosine_flac(&file, 1_000.0, 1.0, 44_100);

    let (engine, mut sink, _events) = loaded(
        &dir,
        TestOpener::default().with_file("r", &file),
        "r",
        Quality::Flac,
    );
    engine.play();
    let first = render(&mut sink, 1);
    pump_for(&mut sink, 1.2);
    engine.pause();
    let position = engine.position();
    println!(
        "44.1 kHz: length {position:.6} s ({} frames), first sample {:.6}",
        (position * 48_000.0).round(),
        first[0]
    );
    assert!(
        (position - 1.0).abs() < 0.002,
        "1.000 s at 44.1 kHz must stay 1.000 s at 48 kHz, got {position:.6}"
    );
    assert!(
        first[0] > 0.45,
        "the resampler delay must be dropped, first sample {:.6}",
        first[0]
    );
}
