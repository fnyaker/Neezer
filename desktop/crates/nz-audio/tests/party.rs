//! D15b's audio seam, end to end: PCM scheduled on the OUTPUT clock, through a
//! real engine and the same mixer the device callback owns.
//!
//! A listen-party guest does not play a file: it hands the mixer runs of PCM that
//! must be HEARD at a given position of the output stream
//! (`nz_audio::Segment`), because that position is what it derived from the host's
//! published timeline. These tests are the proof that the placement is exact —
//! which is the whole difference between a room that is in time and one that is
//! 20 ms out.
//!
//! The numbers each assertion carries were measured on this machine (Rust 1.99.1,
//! 16 threads); `--nocapture` prints them.

mod support;

use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::Receiver;

use nz_audio::{
    Engine, EngineConfig, EngineEvent, Normalization, OfflineSink, Quality, Segment, TrackMeta,
};
use support::*;

fn offline(dir: &Path, opener: TestOpener) -> (Engine, OfflineSink, Receiver<EngineEvent>) {
    let cfg = EngineConfig {
        sample_rate: 48_000,
        spool_dir: dir.join("spool"),
        ..Default::default()
    };
    let (engine, sink) = Engine::new_offline(cfg, Arc::new(opener));
    let events = engine.events();
    (engine, sink, events)
}

/// A click: `frames` of a 1 kHz burst at full scale, padded with `pad` frames of
/// silence either side. Returns the interleaved buffer and where the burst's first
/// frame is inside it.
fn click(frames: usize, pad: usize) -> (Arc<[f32]>, usize) {
    let total = frames + pad * 2;
    let mut data = vec![0.0f32; total * 2];
    for index in 0..frames {
        let value = if index % 2 == 0 { 0.5 } else { -0.5 };
        data[(pad + index) * 2] = value;
        data[(pad + index) * 2 + 1] = value;
    }
    (data.into(), pad)
}

/// The first frame whose left sample is not silence.
fn first_sound(out: &[f32]) -> Option<usize> {
    out.iter().step_by(2).position(|sample| sample.abs() > 1e-6)
}

#[test]
fn a_segment_is_heard_exactly_at_the_stream_position_it_was_given() {
    let dir = scratch("party-placement");
    let (engine, mut sink, _events) = offline(&dir, TestOpener::default());
    let clock = engine.output_clock();
    assert_eq!(
        clock.at_s, clock.position_s,
        "an offline sink is its own clock"
    );

    // The guest's own math, done the way it does it: the 6 s chunk holding media
    // position 20.5 s is scheduled so that ITS start (media 18 s, i.e. chunk 3)
    // is heard at stream position 2.000 s.
    let rate = 48_000.0;
    let (data, pad) = click(480, 24_000);
    let mut segment = Segment::at(2.0, rate, Arc::clone(&data));
    // The click sits 0.5 s inside the buffer: pad frames of silence.
    segment.gain = 1.0;
    engine.party().schedule(segment);
    engine.party().take_over(true);

    let out = {
        let mut out = Vec::with_capacity(3 * 48_000 * 2);
        while out.len() < 3 * 48_000 * 2 {
            out.extend(render(&mut sink, 480));
        }
        out
    };
    let first = first_sound(&out).expect("the party's audio reached the speakers");
    let expected = (2.0 * rate) as usize + pad;
    let error_ms = (first as f64 - expected as f64) / rate * 1000.0;
    println!(
        "click at frame {first}, expected {expected}: {error_ms:+.4} ms, {} underruns",
        engine.underruns()
    );
    // The contract: the stream position it asked for, to the frame.
    assert!(
        first.abs_diff(expected) <= 1,
        "first sound at {first}, expected {expected}"
    );
    assert!(error_ms.abs() < 0.05);
    // Before it, the output is digital silence — nothing early, nothing leaking.
    assert!(out[..expected * 2].iter().all(|sample| *sample == 0.0));
}

#[test]
fn the_persons_equaliser_applies_to_the_party_and_does_not_move_it() {
    let dir = scratch("party-dsp");
    let (engine, mut sink, _events) = offline(&dir, TestOpener::default());
    // The device settings are the person's, and a guest's audio goes through the
    // same chain: +6 dB on the 250 Hz band, loudness normalisation, mono.
    engine.set_eq(2, 6.0);
    engine.set_normalization(Normalization::High);
    engine.set_mono(true);
    engine.set_bass(9.0);

    let rate = 48_000.0;
    let (data, pad) = click(480, 24_000);
    engine.party().schedule(Segment::at(1.5, rate, data));
    engine.party().take_over(true);
    let mut out = Vec::new();
    while out.len() < 3 * 48_000 * 2 {
        out.extend(render(&mut sink, 480));
    }
    let first = first_sound(&out).expect("audible");
    let expected = (1.5 * rate) as usize + pad;
    println!(
        "with the chain engaged: first sound at {first}, expected {expected} ({} frames off)",
        first as i64 - expected as i64
    );
    // Only the filter's own group delay may move it, and this chain's is zero at
    // the biquads' cutoff (a minimum-phase EQ is not a delay line).
    assert!(
        first.abs_diff(expected) <= 2,
        "the DSP chain shifted the party's audio: {first} vs {expected}"
    );
    assert!(engine.underruns() == 0);
}

#[test]
fn the_party_takes_the_output_and_the_local_track_keeps_its_place() {
    let dir = scratch("party-takeover");
    dc_flac(&dir.join("local.flac"), 4.0, 48_000);
    let opener = TestOpener::default().with_file("local", dir.join("local.flac"));
    let (engine, mut sink, events) = offline(&dir, opener);
    engine.set_track_meta(
        "local",
        TrackMeta {
            duration_s: Some(4.0),
            ..Default::default()
        },
    );
    engine.load("local", Quality::Flac, None);
    wait_for(
        &mut sink,
        &events,
        std::time::Duration::from_secs(5),
        |event| matches!(event, EngineEvent::Started { .. }),
    );
    engine.play();
    pump_for(&mut sink, 0.2);
    let before = engine.position();
    assert!(before > 0.1, "the local track is playing: {before}");

    // A guest arrives: the party takes the output over.
    engine.party().take_over(true);
    let (data, _pad) = click(48_000, 0);
    engine.party().schedule(Segment::at(
        engine.output_clock().position_s + 0.05,
        48_000.0,
        data,
    ));
    let out = pump_for(&mut sink, 0.5);
    let after = engine.position();
    println!("local position before {before:.3} s, after {after:.3} s");
    // The local voice is not mixed in (a DC of 0.25 would be plain to see) and
    // its clock does not run: the person's queue is exactly where they left it.
    assert!(
        (out.iter().step_by(2).map(|s| s.abs()).fold(0.0, f32::max) - 0.5).abs() < 0.05,
        "the party's click, not the local track's DC"
    );
    assert!((after - before).abs() < 1e-9, "{before} vs {after}");

    // Leaving gives the output back.
    engine.party().close();
    let out = pump_for(&mut sink, 0.2);
    let peak = out.iter().step_by(2).map(|s| s.abs()).fold(0.0, f32::max);
    assert!(
        (peak - 0.25).abs() < 0.01,
        "the local track is back: {peak}"
    );
    assert!(engine.underruns() == 0);
}
