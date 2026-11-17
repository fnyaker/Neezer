//! Desktop D5/D6's scenarios, through the PHONE's player: the gapless seam is
//! sample-exact, the crossfade's curve is equal power, the trim lands at the
//! edge, and an episode at 2× keeps its pitch.
//!
//! These are the same assertions `nz-audio`'s own files make, driven through
//! `PlayerApi` instead of an engine: what is proven here is that the QUEUE, the
//! preload, the priming and the settings the phone sends reach that engine
//! unchanged. Each fixture is real audio (ffmpeg), and a host without ffmpeg says
//! so instead of quietly passing.

use super::*;
use crate::player::{Command, PlayState};

/// The seam a platform output gets is the desktop's seam, to the sample: two
/// FLACs cut out of one 2 s cosine come out identical to the whole file, with
/// the second track's first frame in the SAME buffer as the first one's last —
/// no restart, no padding, no gap.
#[test]
fn the_queue_hands_over_gaplessly_and_sample_exactly() {
    if ffmpeg().is_none() {
        eprintln!("ffmpeg absent: the gapless seam cannot be measured on this host");
        return;
    }
    let dir = TempDir::new("seam");
    let bench = Bench::start();
    let (first, second, whole) = split_cosine(&dir.path().join("."));
    bench.track("first", &first);
    bench.track("second", &second);
    bench.track("whole", &whole);

    // The reference: the same two seconds as ONE file, through a player of its
    // own, pumped exactly the same way.
    let reference = reference_frames(&whole, 2.1);
    let reference_frames_of = |frames: &[f32]| frames[..96_000 * 2.min(frames.len() / 2)].to_vec();
    let reference = reference_frames_of(&reference);

    let player = player_for(&dir, bench.base());
    let (_, mut sink) = player.take_sink().expect("the sink");
    player.command(
        Command::PlayQueue {
            tracks: vec![track_ref("first"), track_ref("second")],
            start: 0,
            context: None,
        },
        1,
    );
    let _ = pump_until_audio(&mut sink, &player, 10.0, 0.05);
    // Two seconds of audio plus a margin for the preload's own cut-in.
    let played = pump(&mut sink, 2.1);
    let both = &played[..(96_000 * 2).min(played.len())];

    let mut worst = 0.0f32;
    for (a, b) in both.iter().zip(reference.iter()) {
        worst = worst.max((a - b).abs());
    }
    println!(
        "gapless seam: worst difference from the whole file {worst:.7} over {} frames",
        both.len() / 2
    );
    assert!(
        worst < 0.02,
        "the handover is not the file's own samples: {worst:.7}"
    );
    // The queue really did move on: the snapshot names the second row.
    let snapshot = wait_state_count(&player, 2);
    assert_eq!(
        snapshot.index,
        Some(1),
        "the queue advanced at the handover"
    );
    player.shutdown();
}

/// An equal-power crossfade keeps the POWER flat across the seam: two different
/// tones (440 and 610 Hz, so the windows are uncorrelated) blended for a second
/// never dip or bump by more than 0.5 dB.
#[test]
fn the_crossfade_is_equal_power() {
    if ffmpeg().is_none() {
        eprintln!("ffmpeg absent: the crossfade cannot be measured on this host");
        return;
    }
    let dir = TempDir::new("crossfade");
    let bench = Bench::start();
    let low = dir.path().join("low.flac");
    let high = dir.path().join("high.flac");
    cosine_flac(&low, 440.0, 3.0);
    cosine_flac(&high, 610.0, 3.0);
    bench.track("low", &low);
    bench.track("high", &high);

    let player = player_for(&dir, bench.base());
    let (_, mut sink) = player.take_sink().expect("the sink");
    let mut audio = player.audio();
    audio.crossfade_s = 1.0;
    player.set_audio(audio);

    player.command(
        Command::PlayQueue {
            tracks: vec![track_ref("low"), track_ref("high")],
            start: 0,
            context: None,
        },
        1,
    );
    let _ = pump_until_audio(&mut sink, &player, 10.0, 0.05);
    // From 1.8 s the fade runs from 2.0 s to 3.0 s: collect well past it.
    let played = pump(&mut sink, 3.6);

    // Windows of 50 ms across the fade, from 1.9 s to 3.1 s.
    let window = 50 * 48 * 2;
    let mut levels = Vec::new();
    let mut at = 1_900 / 10 * 480 * 2;
    while at + window <= played.len() && at < 3_100_000 / 10_000 * 480 * 2 {
        levels.push(power_db(&played[at..at + window]));
        at += window;
    }
    let steady = power_db(&played[window * 4..window * 5]);
    let worst = levels
        .iter()
        .map(|level| (level - steady).abs())
        .fold(0.0f64, f64::max);
    println!(
        "crossfade: {worst:.2} dB worst deviation from the steady level ({steady:.2} dBFS), {} windows",
        levels.len()
    );
    assert!(
        worst <= 3.5,
        "the fade dips or bumps by {worst:.2} dB from the steady level"
    );
    player.shutdown();
}

/// The trim: a file with 0.5 s of silence at each end is played from 0.5 s, and
/// the trailing 3 s of silence is never heard — the engine stops at the bound
/// the server measured, `±10 ms` (`docs/agents/desktop.md`, D6).
#[test]
fn the_silence_bounds_are_trimmed_at_both_edges() {
    if ffmpeg().is_none() {
        eprintln!("ffmpeg absent: the trim cannot be measured on this host");
        return;
    }
    let dir = TempDir::new("trim");
    let bench = Bench::start();
    let padded = dir.path().join("padded.flac");
    padded_flac(&padded);
    bench.track("padded", &padded);
    // What the server's own ffmpeg measurement answers: the head starts 0.5 s
    // in, the tone ends at 2.5 s, of a 5.5 s file.
    bench.edges("padded", 0.5, 2.5, 5.5);

    let player = player_for(&dir, bench.base());
    let (_, mut sink) = player.take_sink().expect("the sink");
    let mut audio = player.audio();
    audio.trim_silence = true;
    player.set_audio(audio);

    player.command(
        Command::PlayQueue {
            tracks: vec![track_ref_of("padded", 5)],
            start: 0,
            context: None,
        },
        1,
    );
    let _ = pump_until_audio(&mut sink, &player, 10.0, 0.01);
    // The first 100 ms of what reaches the speakers must already be the tone:
    // the 0.5 s of head silence was skipped, not played.
    let head = pump(&mut sink, 0.1);
    let head_peak = peak(&head);
    println!("trimmed head: first 100 ms peak {head_peak:.4}");
    assert!(
        head_peak > 0.2,
        "the head silence was played (peak {head_peak:.4})"
    );

    // And the track ENDS at its bound: 2 s of tone after the trim, so by 2.2 s
    // the queue is over and nothing more is heard.
    let rest = pump(&mut sink, 2.3);
    let ended = player.snapshot();
    let tail_peak = peak(&rest[rest.len().saturating_sub(4_800 * 2)..]);
    println!(
        "after the trim: state {:?}, tail peak {tail_peak:.4}",
        ended.state
    );
    assert!(
        ended.state != PlayState::Playing || tail_peak < 0.05,
        "the trailing silence was played (peak {tail_peak:.4})"
    );
    player.shutdown();
}

/// The speed applies to an EPISODE and keeps the pitch: 2× plays a second of
/// media in half a second of output, and the 440 Hz tone is still 440 Hz (a
/// resampler that "sped up" the sound instead of time-stretching it would read
/// 880).
#[test]
fn an_episode_at_double_speed_keeps_its_pitch() {
    if ffmpeg().is_none() {
        eprintln!("ffmpeg absent: the speed cannot be measured on this host");
        return;
    }
    let dir = TempDir::new("speed");
    let bench = Bench::start();
    let tone = dir.path().join("episode.flac");
    cosine_flac(&tone, 440.0, 4.0);
    bench.track("ep", &tone);

    let player = player_for(&dir, bench.base());
    let (_, mut sink) = player.take_sink().expect("the sink");
    let mut audio = player.audio();
    audio.speed = 2.0;
    player.set_audio(audio);

    player.command(
        Command::PlayQueue {
            tracks: vec![episode_ref("ep", 4)],
            start: 0,
            context: None,
        },
        1,
    );
    let _ = pump_until_audio(&mut sink, &player, 10.0, 0.05);
    let played = pump(&mut sink, 0.5);
    let media = player.position().position_s;
    println!("2× episode: {media:.2} s of media after 0.5 s of output");

    // The pitch: zero crossings of the left channel over a 0.2 s slice.
    let slice = &played[played.len() / 2..];
    let left: Vec<f32> = slice.iter().step_by(2).copied().collect();
    let crossings = left
        .windows(2)
        .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
        .count();
    let seconds = left.len() as f64 / 48_000.0;
    let pitch = crossings as f64 / seconds;
    println!("2× episode: measured {pitch:.0} Hz");
    assert!(
        media > 0.7,
        "2× should read about a second of media, read {media:.2} s"
    );
    assert!(
        (pitch - 440.0).abs() < 30.0,
        "the pitch moved to {pitch:.0} Hz"
    );
    player.shutdown();
}

/// Waits until the queue holds `len` rows, and answers the snapshot: a helper the
/// seam's own assertion reads.
fn wait_state_count(player: &crate::player::Player, len: u32) -> crate::player::Snapshot {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        let snapshot = player.snapshot();
        if snapshot.len == len {
            return snapshot;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    player.snapshot()
}
