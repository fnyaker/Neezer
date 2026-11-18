//! D6's acceptance, end to end: real FLACs through a real engine, rendered by
//! the same mixer the output callback owns.
//!
//! Each assertion carries the number it was measured at on the development
//! machine (Rust 1.99.1, 16 threads); `--nocapture` prints them, which is how
//! the PR's table was filled. The criterion list is `docs/agents/desktop.md`,
//! D6.

mod support;

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use nz_audio::{
    Edges, Engine, EngineConfig, EngineEvent, FadeSettings, Normalization, OfflineSink, Quality,
    TrackMeta,
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

fn ffmpeg(args: &[&str]) {
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .status()
        .expect("ffmpeg is installed (the desktop CI installs it)");
    assert!(status.success(), "ffmpeg {args:?} failed");
}

/// A stereo FLAC of `expression` over `seconds`; `|` separates the channels.
fn expression_flac(path: &Path, expression: &str, seconds: f64) {
    let source = format!("aevalsrc={expression}:s=48000:d={seconds}:c=stereo");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &source,
        "-c:a",
        "flac",
        path.to_str().unwrap(),
    ]);
}

/// A tone at `hz`, amplitude `amp`, for `seconds`.
fn tone_flac(path: &Path, hz: f64, amp: f64, seconds: f64) {
    expression_flac(path, &format!("{amp}*cos(2*PI*{hz}*t)"), seconds);
}

/// A track whose audio is `seconds` of a tone followed by `tail` seconds of
/// digital silence: what a master carries after its last note.
fn tone_then_silence(path: &Path, hz: f64, amp: f64, seconds: f64, tail: f64) {
    expression_flac(
        path,
        // ffmpeg's filtergraph parser splits on commas: the expression's own
        // commas have to be escaped for it.
        &format!("if(lt(t\\,{seconds})\\,{amp}*cos(2*PI*{hz}*t)\\,0)"),
        seconds + tail,
    );
}

/// Load `current`, wait for it, preload `next`, and fill both decode rings while
/// the mixer is paused. `metas` are set BEFORE the load, which is what the app
/// does (`gaincache` primes the gain, `Player.svelte#primeEdgesAround` the
/// bounds): a gain that arrives after the first sample would be a volume change
/// in the middle of a song.
fn primed_with(
    dir: &Path,
    opener: TestOpener,
    current: &str,
    next: Option<&str>,
    metas: &[(&str, TrackMeta)],
) -> (Engine, OfflineSink, Receiver<EngineEvent>) {
    let (engine, mut sink, events) = offline(dir, opener);
    for (id, meta) in metas {
        engine.set_track_meta(id, *meta);
    }
    engine.load(current, Quality::Flac, None);
    wait_for(
        &mut sink,
        &events,
        Duration::from_secs(5),
        |event| matches!(event, EngineEvent::Started { id, .. } if id == current),
    );
    if let Some(next) = next {
        engine.preload_next(next, Quality::Flac);
    }
    pump_for(&mut sink, 0.2);
    (engine, sink, events)
}

fn primed(
    dir: &Path,
    opener: TestOpener,
    current: &str,
    next: Option<&str>,
) -> (Engine, OfflineSink, Receiver<EngineEvent>) {
    primed_with(dir, opener, current, next, &[])
}

fn meta(duration_s: f64) -> TrackMeta {
    TrackMeta {
        duration_s: Some(duration_s),
        ..TrackMeta::default()
    }
}

/// Pump in real time for `seconds`, returning the audio rendered.
fn pump_seconds(sink: &mut OfflineSink, seconds: f64) -> Vec<f32> {
    pump_for(sink, seconds)
}

/// The RMS of a stereo block (both channels' power).
fn rms(block: &[f32]) -> f64 {
    if block.is_empty() {
        return 0.0;
    }
    (block
        .iter()
        .map(|sample| f64::from(*sample).powi(2))
        .sum::<f64>()
        / block.len() as f64)
        .sqrt()
}

/// `20·log10(x)`.
fn db(x: f64) -> f64 {
    20.0 * x.max(1e-12).log10()
}

/// The equal-power criterion: two uncorrelated tones crossfaded over 1.0 s hold
/// their summed POWER constant, window by window, instead of dipping 3 dB in the
/// middle as complementary LINEAR ramps would.
///
/// Measured: 50 ms windows across the fade, worst deviation from the pre-fade
/// level **0.06 dB** (the ±0.5 dB criterion) and 0 windows below −0.5 dB.
#[test]
fn the_crossfade_holds_the_summed_power_within_half_a_db() {
    let dir = scratch("xfade-power");
    let a = dir.join("a.flac");
    let b = dir.join("b.flac");
    tone_flac(&a, 440.0, 0.25, 2.0);
    tone_flac(&b, 610.0, 0.25, 2.0);

    let (engine, mut sink, events) = primed_with(
        &dir,
        TestOpener::default().with_file("a", &a).with_file("b", &b),
        "a",
        Some("b"),
        &[("a", meta(2.0)), ("b", meta(2.0))],
    );
    engine.set_fade(FadeSettings {
        enabled: true,
        seconds: 1.0,
        ..FadeSettings::default()
    });
    engine.play();

    // 0.9 s of a alone, then the whole fade (1.0 … 2.0 s), then b alone.
    let before = pump_seconds(&mut sink, 0.9);
    let level = rms(&before[before.len() - 24_000 * 2..]);
    assert!(
        before.iter().any(|sample| *sample != 0.0),
        "a must be audible before the fade"
    );
    let across = pump_seconds(&mut sink, 1.15);
    let after = pump_seconds(&mut sink, 0.3);
    engine.pause();

    // The handover happens at the START of the fade: at 1.0 s, b is already the
    // track the bar, the queue and MPRIS name.
    let mut started_b = false;
    while let Ok(event) = events.try_recv() {
        if let EngineEvent::Started { id, .. } = event
            && id == "b"
        {
            started_b = true;
        }
    }
    assert!(
        started_b,
        "the next voice must take over at the fade's start"
    );
    assert_eq!(engine.current_id().as_deref(), Some("b"));

    // 50 ms windows over the whole fade, compared to the level a alone had.
    let window = 2_400 * 2;
    let mut worst = 0.0f64;
    let mut dips = 0;
    let mut windows = 0;
    for slice in across.chunks_exact(window) {
        let level_db = db(rms(slice)) - db(level);
        windows += 1;
        worst = worst.max(level_db.abs());
        if level_db < -0.5 {
            dips += 1;
        }
    }
    let after_level = db(rms(&after)) - db(level);
    println!(
        "a alone {:.2} dBFS; across the fade: {windows} windows, worst {worst:.2} dB, {dips} below -0.5 dB; \
         b alone {after_level:+.2} dB re a",
        db(level)
    );
    assert!(
        worst < 0.5,
        "the crossfade's summed power moved by {worst:.2} dB (criterion ±0.5)"
    );
    assert_eq!(dips, 0, "{dips} windows dipped more than 0.5 dB");
    assert!(
        after_level.abs() < 0.5,
        "b alone must be at the same level as a, {after_level:+.2} dB"
    );
}

/// **Both ends, on one real file**: a master that opens with a beat of digital
/// silence and closes on three seconds of it is played from the first sample of
/// its music to the last, and nowhere else.
///
/// The fixture is 0.5 s of silence, 2.0 s of tone, 3.0 s of silence (5.5 s), and
/// the bounds are the ones the server measures (`neezer/webui/edges.py`):
/// `start: 0.5, end: 2.5, duration: 5.5`. The bounds go in BEFORE the voice is
/// built (`AppState::preload` primes the next track, which is exactly why this
/// order is the one that matters): the head trim is a `?start=`/seek decision
/// taken at build time, the tail trim is a stop command the mixer obeys.
///
/// Measured: the first rendered sample is the tone's own **+0.2500**, i.e. the
/// head silence never reaches the speakers (the same file untrimmed renders
/// **0.0000** as its first sample and stays silent through the 0.2 s window);
/// the clock starts at **0.500 s**; the track ends at **2.500 s** with a play of
/// **1.81 s** of wall clock for 2.00 s of audio, so none of the 3.0 s of tail
/// silence is played.
#[test]
fn a_measured_head_and_tail_of_silence_are_both_skipped() {
    let dir = scratch("trim-both");
    let file = dir.join("both.flac");
    // 0.5 s of silence, then 2 s of tone, then 3 s of silence. The commas are
    // escaped: ffmpeg's filtergraph parser splits on them.
    let expression = "if(lt(t\\,0.5)\\,0\\,if(lt(t\\,2.5)\\,0.25*cos(2*PI*440*t)\\,0))";
    expression_flac(&file, expression, 5.5);

    let bounds = Edges {
        start: 0.5,
        end: 2.5,
        duration: 5.5,
    };

    // Untrimmed: the head silence IS played, so the first samples are zero.
    let (plain, mut plain_sink, _plain_events) = primed(
        &dir,
        TestOpener::default().with_file("both", &file),
        "both",
        None,
    );
    plain.set_track_meta("both", meta(5.5));
    plain.play();
    let plain_audio = pump_seconds(&mut plain_sink, 0.2);
    let plain_first = plain_audio.first().copied().unwrap_or(0.0);
    let plain_peak_head = plain_audio.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
    plain.pause();
    drop(plain);

    // Trimmed at both ends.
    let (trimmed, mut trimmed_sink, trimmed_events) = primed_with(
        &dir,
        TestOpener::default().with_file("both", &file),
        "both",
        None,
        &[(
            "both",
            TrackMeta {
                edges: Some(bounds),
                duration_s: Some(5.5),
                ..TrackMeta::default()
            },
        )],
    );
    let started_at = trimmed.position();
    trimmed.play();
    let head = pump_seconds(&mut trimmed_sink, 0.2);
    let first = head.first().copied().unwrap_or(0.0);
    let head_peak = head.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));

    // Run to the end of what the trim allows.
    let clock = Instant::now();
    let mut ended_after = None;
    while clock.elapsed() < Duration::from_secs(6) {
        while let Ok(event) = trimmed_events.try_recv() {
            if matches!(event, EngineEvent::Ended { .. }) {
                ended_after = Some(clock.elapsed().as_secs_f64());
            }
        }
        if ended_after.is_some() {
            break;
        }
        pump_seconds(&mut trimmed_sink, 0.02);
    }
    let end_position = trimmed.position();
    trimmed.pause();
    println!(
        "untrimmed: first sample {plain_first:.4}, head peak {plain_peak_head:.4}; \
         trimmed: starts at {started_at:.3} s, first sample {first:.4}, head peak {head_peak:.4}, \
         ends at {end_position:.3} s, the play lasted {:.3} s",
        ended_after.unwrap_or(f64::NAN)
    );

    assert!(
        plain_first.abs() < 0.01 && plain_peak_head < 0.01,
        "the fixture really opens on silence (first {plain_first:.4}, peak {plain_peak_head:.4})"
    );
    assert!(
        (started_at - 0.5).abs() < 0.02,
        "the voice starts at the audio's own beginning, got {started_at:.3} s"
    );
    assert!(
        first > 0.2 && head_peak > 0.2,
        "the first sample played is the tone, not the head silence (first {first:.4})"
    );
    let ended_after = ended_after.expect("the trimmed track must end");
    assert!(
        (end_position - 2.5).abs() < 0.1,
        "the clock stops at the audio's own end, got {end_position:.3} s"
    );
    // The wall clock bounds what could have been heard: 2.0 s of music plus the
    // pump's 20 ms granularity. A tail left in would keep it playing for 5 s.
    assert!(
        ended_after < 2.4,
        "the 3 s of tail must be cut: the play lasted {ended_after:.3} s"
    );
}

/// The trim criterion: the 3 s of silence a master carries at its tail are never
/// played. The same file, played without bounds, runs to its end — so the
/// difference is the trim, not the fixture.
///
/// Measured: with the bounds the voice ends at **2.000 s** and the clock stops
/// there; without them the clock reads **2.60 s** and nothing has ended.
#[test]
fn a_measured_tail_of_silence_is_never_played() {
    let dir = scratch("trim-tail");
    let file = dir.join("tail.flac");
    tone_then_silence(&file, 440.0, 0.25, 2.0, 3.0);

    // Without bounds: 5 s of file, 2 s of it audible.
    let (plain, mut plain_sink, plain_events) =
        primed(&dir, TestOpener::default().with_file("t", &file), "t", None);
    plain.set_track_meta("t", meta(5.0));
    plain.play();
    pump_seconds(&mut plain_sink, 2.6);
    let plain_position = plain.position();
    let ended_without = plain_events
        .try_iter()
        .any(|event| matches!(event, EngineEvent::Ended { .. }));
    plain.pause();
    drop(plain);

    // With the server's bounds, arriving after the voice was built (the app asks
    // for the bounds of the track it is playing too): the voice stops at 2.0 s.
    let (trimmed, mut trimmed_sink, trimmed_events) =
        primed(&dir, TestOpener::default().with_file("t", &file), "t", None);
    trimmed.set_track_meta(
        "t",
        TrackMeta {
            edges: Some(Edges {
                start: 0.0,
                end: 2.0,
                duration: 5.0,
            }),
            duration_s: Some(5.0),
            ..TrackMeta::default()
        },
    );
    let started = Instant::now();
    trimmed.play();
    let mut ended_after = None;
    while started.elapsed() < Duration::from_secs(5) {
        while let Ok(event) = trimmed_events.try_recv() {
            if matches!(event, EngineEvent::Ended { .. }) {
                ended_after = Some(started.elapsed().as_secs_f64());
            }
        }
        if ended_after.is_some() {
            break;
        }
        pump_seconds(&mut trimmed_sink, 0.02);
    }
    let trimmed_position = trimmed.position();
    trimmed.pause();
    println!(
        "without bounds: position {plain_position:.3} s, ended {ended_without}; \
         with bounds: position {trimmed_position:.3} s, ended after {ended_after:?}"
    );
    assert!(
        !ended_without,
        "the untrimmed file must still be playing at 2.6 s"
    );
    assert!(
        (plain_position - 2.6).abs() < 0.05,
        "the clock must follow the file, got {plain_position:.3}"
    );
    let ended_after = ended_after.expect("the trimmed track must end");
    assert!(
        (ended_after - 2.0).abs() < 0.15,
        "the trim must end the track at 2.0 s, ended after {ended_after:.3}"
    );
    assert!(
        (trimmed_position - 2.0).abs() < 0.1,
        "the clock must stop at the trim, got {trimmed_position:.3}"
    );
}

/// A transport action during a fade wins AT ONCE (`playback.md`, *A transport
/// action always wins*): the new track takes over in the next buffer, and the
/// overlap in flight is cut in the web player's 60 ms instead of running its
/// remaining 1.7 s.
#[test]
fn a_skip_during_a_fade_wins_at_once() {
    let dir = scratch("xfade-skip");
    let a = dir.join("a.flac");
    let b = dir.join("b.flac");
    let c = dir.join("c.flac");
    tone_flac(&a, 440.0, 0.25, 3.0);
    tone_flac(&b, 610.0, 0.25, 3.0);
    dc_flac(&c, 3.0, 48_000);

    let (engine, mut sink, events) = primed_with(
        &dir,
        TestOpener::default()
            .with_file("a", &a)
            .with_file("b", &b)
            .with_file("c", &c),
        "a",
        Some("b"),
        &[("a", meta(3.0)), ("b", meta(3.0)), ("c", meta(3.0))],
    );
    engine.set_fade(FadeSettings {
        enabled: true,
        seconds: 2.0,
        ..FadeSettings::default()
    });
    engine.play();
    // The fade covers 1.0 … 3.0 s. Sit in its middle.
    pump_seconds(&mut sink, 1.3);
    assert_eq!(engine.current_id().as_deref(), Some("b"));
    let mid = pump_seconds(&mut sink, 0.05);
    assert!(
        rms(&mid) > 0.01,
        "two tracks must be audible mid-fade, rms {}",
        rms(&mid)
    );

    engine.load("c", Quality::Flac, None);
    wait_for(
        &mut sink,
        &events,
        Duration::from_secs(5),
        |event| matches!(event, EngineEvent::Started { id, .. } if id == "c"),
    );
    // Buffer by buffer: the first one that is only c's constant 0.25.
    let mut settled = None;
    for buffer in 1..=20 {
        let block = pump_seconds(&mut sink, 0.01);
        let worst = block
            .iter()
            .map(|sample| (f64::from(*sample) - 0.25).abs())
            .fold(0.0, f64::max);
        if worst < 1e-4 {
            settled = Some(buffer * 10);
            break;
        }
    }
    println!("only c left after {settled:?} ms (the fade had 1700 ms to run)");
    let settled = settled.expect("c must take over");
    assert!(
        settled <= 100,
        "the skip must win within the 60 ms cut plus a buffer, took {settled} ms"
    );
    assert_eq!(engine.current_id().as_deref(), Some("c"));
    engine.pause();
}

/// The DSP the callback applies is the web graph's: +6 dB on the 250 Hz band
/// lifts a 250 Hz tone by 6.000 dB, measured on the samples (relative to the
/// same chain with a transparent band, so the limiter's static makeup cancels).
///
/// Measured: **5.997 dB** (criterion ±0.1 dB).
#[test]
fn the_engine_eq_is_the_web_graphs_six_db() {
    let dir = scratch("dsp-eq");
    let file = dir.join("tone.flac");
    tone_flac(&file, 250.0, 0.1, 2.0);
    let level = |eq_db: f32| {
        let (engine, mut sink, _events) = primed_with(
            &dir,
            TestOpener::default().with_file("t", &file),
            "t",
            None,
            &[("t", meta(2.0))],
        );
        engine.set_eq(3, eq_db);
        engine.play();
        pump_seconds(&mut sink, 0.5);
        let tail = pump_seconds(&mut sink, 0.5);
        engine.pause();
        db(rms(&tail))
    };
    let flat = level(0.001);
    let boosted = level(6.0);
    let lift = boosted - flat;
    println!(
        "250 Hz: transparent band {flat:.3} dB, +6 dB band {boosted:.3} dB, lift {lift:.3} dB"
    );
    assert!(
        (lift - 6.0).abs() < 0.1,
        "the band's lift must be 6 dB, measured {lift:.3}"
    );
}

/// The analysis tap is post-DSP and PRE-mono/balance: with the mono fold on, the
/// speakers get one folded signal while the tap still carries the two different
/// channels the scope exists to draw (`pan.js`).
///
/// Measured: speakers **0.1768/0.1768**, tap **0.4000/0.1000**.
#[test]
fn the_tap_is_post_dsp_and_pre_pan() {
    let dir = scratch("dsp-tap");
    let file = dir.join("stereo.flac");
    // Left 0.4, right 0.1: a stereo signal the mono fold collapses and the tap
    // must keep.
    expression_flac(&file, "0.4|0.1", 2.0);
    let (engine, mut sink, _events) = primed_with(
        &dir,
        TestOpener::default().with_file("t", &file),
        "t",
        None,
        &[("t", meta(2.0))],
    );
    let mut tap = engine.tap().expect("the tap is taken once");
    engine.set_mono(true);
    engine.play();
    pump_seconds(&mut sink, 0.5);
    let out = pump_seconds(&mut sink, 0.1);
    engine.pause();

    let mut tapped = vec![0.0f32; 480 * 2];
    let frames = tap.read(&mut tapped);
    assert!(frames > 0, "the tap must have frames after 0.1 s");
    let last = &out[out.len() - 480 * 2..];
    let speaker_left = f64::from(last[0]).abs();
    let speaker_right = f64::from(last[1]).abs();
    let tap_left = f64::from(tapped[0]).abs();
    let tap_right = f64::from(tapped[1]).abs();
    println!("speakers {speaker_left:.4}/{speaker_right:.4}, tap {tap_left:.4}/{tap_right:.4}");
    // The mono fold is 0.5·(0.4+0.1)·cos(π/4) = 0.1768 on both sides.
    assert!(
        (speaker_left - speaker_right).abs() < 1e-4 && (speaker_left - 0.1768).abs() < 0.01,
        "the speakers must get the mono fold, got {speaker_left:.4}/{speaker_right:.4}"
    );
    assert!(
        (tap_left - 0.4).abs() < 0.02 && (tap_right - 0.1).abs() < 0.02,
        "the tap must keep the stereo signal, got {tap_left:.4}/{tap_right:.4}"
    );
}

/// Podcast speed, end to end: at 2× the clock advances two media seconds per
/// real second, the pitch does not move, and music is untouched by the same
/// setting.
///
/// Measured: an episode at 2× reads **1.000 s** of media after 0.5 s of output
/// and 440 Hz stays 440 Hz; the same file as music reads **0.500 s**.
#[test]
fn the_speed_applies_to_an_episode_only_and_keeps_the_pitch() {
    let dir = scratch("dsp-speed");
    let file = dir.join("tone.flac");
    tone_flac(&file, 440.0, 0.25, 4.0);

    let measure = |podcast: bool| {
        let (engine, mut sink, _events) = primed_with(
            &dir,
            TestOpener::default().with_file("t", &file),
            "t",
            None,
            &[(
                "t",
                TrackMeta {
                    duration_s: Some(4.0),
                    podcast,
                    ..TrackMeta::default()
                },
            )],
        );
        engine.set_speed(2.0);
        engine.play();
        let out = pump_seconds(&mut sink, 0.5);
        let position = engine.position();
        engine.pause();
        (position, out)
    };

    let (episode_position, episode) = measure(true);
    let (music_position, _) = measure(false);
    // The pitch: zero crossings over the last 0.2 s of the 2× episode.
    let tail = &episode[episode.len() - 19_200..];
    let left: Vec<f32> = tail
        .as_chunks::<2>()
        .0
        .iter()
        .map(|frame| frame[0])
        .collect();
    let crossings = left
        .windows(2)
        .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
        .count();
    let hz = crossings as f64 / 2.0 / (left.len() as f64 / 48_000.0);
    println!(
        "episode at 2x: 0.5 s of output = {episode_position:.3} s of media, {hz:.1} Hz; \
         music at 2x: {music_position:.3} s of media"
    );
    assert!(
        (episode_position - 1.0).abs() < 0.05,
        "an episode at 2× must advance 1.0 s of media in 0.5 s, got {episode_position:.3}"
    );
    assert!(
        (hz - 440.0).abs() < 15.0,
        "the pitch must not move, measured {hz:.1} Hz"
    );
    assert!(
        (music_position - 0.5).abs() < 0.05,
        "music always plays at 1×, got {music_position:.3}"
    );
}

/// The normalisation is the web graph's ReplayGain: a track whose measured
/// loudness is 0 dB comes out 10.4 dB down at `high` (`-(0 + 18.4) + 8`), and
/// `off` leaves it exactly alone.
///
/// Measured: **−10.40 dB** at `high`, **0.00 dB** at `off`.
#[test]
fn the_normalisation_is_the_web_graphs_replaygain() {
    let dir = scratch("dsp-gain");
    let file = dir.join("dc.flac");
    dc_flac(&file, 3.0, 48_000); // a constant 0.25

    let level = |normalization: Normalization| {
        let (engine, mut sink, _events) = primed_with(
            &dir,
            TestOpener::default().with_file("t", &file),
            "t",
            None,
            &[(
                "t",
                TrackMeta {
                    loudness: Some(0.0),
                    duration_s: Some(3.0),
                    ..TrackMeta::default()
                },
            )],
        );
        engine.set_normalization(normalization);
        engine.play();
        pump_seconds(&mut sink, 0.2);
        let out = pump_seconds(&mut sink, 0.1);
        engine.pause();
        f64::from(out[out.len() - 2])
    };
    let off = level(Normalization::Off);
    let high = level(Normalization::High);
    let change = db(high / off);
    println!("normalisation: off {off:.4}, high {high:.4}, {change:+.2} dB");
    assert!((off - 0.25).abs() < 1e-6, "off must not touch the samples");
    assert!(
        (change - (-10.4)).abs() < 0.05,
        "high must be the web's −10.4 dB, measured {change:+.2}"
    );
}

/// **Two analysis readers and the equalizer's meter (MB12).** The visualiser's
/// subscription and the equalizer card's each get every frame of the same sound,
/// neither steals the other's, and the meter reads the chain's own pre-limiter
/// peak and limiter reduction while the card asks for it — and nothing at all
/// once it stops.
///
/// Measured: both taps 0.2500/0.2500 of a 1 kHz tone at 0.25, meter peak
/// **−12.04 dB** with a transparent chain, and **−96.00 dB** (the floor) after
/// the card leaves.
#[test]
fn two_taps_read_the_same_sound_and_the_meter_measures_the_chain() {
    let dir = scratch("dsp-tap-multi");
    let file = dir.join("tone.flac");
    tone_flac(&file, 1000.0, 0.25, 4.0);
    let (engine, mut sink, _events) = primed_with(
        &dir,
        TestOpener::default().with_file("t", &file),
        "t",
        None,
        &[("t", meta(4.0))],
    );
    // The one the visualiser owns, and the card's own subscription.
    let mut viz = engine.tap().expect("the legacy reader");
    let mut card = engine.subscribe_tap().expect("a counted subscription");
    engine.play();
    pump_seconds(&mut sink, 0.3);

    let mut a = vec![0.0f32; 4_800];
    let mut b = vec![0.0f32; 4_800];
    let na = viz.read(&mut a);
    let nb = card.read(&mut b);
    println!("taps: {na} and {nb} frames, a {:.4} b {:.4}", a[0], b[0]);
    assert!(na > 0 && nb > 0, "both readers got frames: {na} / {nb}");
    assert_eq!(na, nb, "the same sound, frame for frame");
    assert!((a[0] - b[0]).abs() < 1e-6, "{} vs {}", a[0], b[0]);

    // The meter, while the card asks for it: the chain is transparent, so the
    // pre-limiter peak is the tone's own −12.04 dBFS and nothing pulls it down.
    engine.set_meter(true);
    pump_seconds(&mut sink, 0.2);
    let on = engine.chain_meter();
    println!("meter on: peak {:.2} dB, reduction {:.2} dB", on.peak_db, on.reduction_db);
    assert!((on.peak_db - db(0.25)).abs() < 0.5, "peak {:.2}", on.peak_db);
    assert_eq!(on.reduction_db, 0.0, "a clean master never touches the limiter");

    // It leaves: the reading is forgotten in the same call, not left standing
    // for the next visit to show as a live number.
    engine.set_meter(false);
    engine.pause();
    let off = engine.chain_meter();
    assert_eq!(off.peak_db, nz_audio::METER_FLOOR_DB, "a hidden card has no reading");
    assert_eq!(off.reduction_db, 0.0);
    // And a dropped subscription stops: the card's reader is gone, the
    // visualiser's keeps working.
    drop(card);
    engine.play();
    pump_seconds(&mut sink, 0.1);
    let mut c = vec![0.0f32; 480];
    assert!(viz.read(&mut c) > 0, "the visualiser still reads after the card left");
    engine.pause();
}
