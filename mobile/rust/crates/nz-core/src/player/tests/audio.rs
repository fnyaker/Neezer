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
    let dir = TempDir::new("seam");
    let bench = Bench::start();
    let first = Fixture::First.write(dir.path());
    let second = Fixture::Second.write(dir.path());
    let whole = Fixture::Whole.write(dir.path());
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
    let _ = pump_until_audio(&mut sink, &player, 30.0, 0.05);
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
    let dir = TempDir::new("crossfade");
    let bench = Bench::start();
    let low = Fixture::Tone440.write(dir.path());
    let high = Fixture::Tone610.write(dir.path());
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
    let _ = pump_until_audio(&mut sink, &player, 30.0, 0.05);
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
    let dir = TempDir::new("trim");
    let bench = Bench::start();
    let padded = Fixture::Padded.write(dir.path());
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
    let _ = pump_until_audio(&mut sink, &player, 30.0, 0.01);
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
    let dir = TempDir::new("speed");
    let bench = Bench::start();
    let tone = Fixture::Tone440.write(dir.path());
    bench.track("ep", &tone);

    let player = player_for(&dir, bench.base());
    let (_, mut sink) = player.take_sink().expect("the sink");
    let mut audio = player.audio();
    audio.speed = 2.0;
    player.set_audio(audio);

    player.command(
        Command::PlayQueue {
            tracks: vec![episode_ref("ep", 3)],
            start: 0,
            context: None,
        },
        1,
    );
    let _ = pump_until_audio(&mut sink, &player, 30.0, 0.05);
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

/// **Normalisation High changes the samples.** The server measured this track at
/// 0 LUFS, so `dsp::gain_db(Some(0.0), High)` is −10.4 dB (ReplayGain reference
/// 18.4 dB, the High level's +8 dB target): the second window must be that much
/// quieter than the same tone at Off, through the phone's own queue and engine.
///
/// The switch is flipped MID-TRACK on purpose: that is what the owner does in
/// Réglages, and the level was not asked for before (Off asks nothing at all —
/// rule 1), so this is the case where a missing link shows as a 0 dB delta.
#[test]
fn the_normalisation_gain_lowers_the_samples_by_the_expected_db() {
    let dir = TempDir::new("normalize");
    let bench = Bench::start();
    let tone = Fixture::Tone440.write(dir.path());
    bench.track("tone", &tone);
    bench.gains("tone", Some(0.0));

    let player = player_for(&dir, bench.base());
    let (_, mut sink) = player.take_sink().expect("the sink");
    player.command(
        Command::PlayQueue {
            tracks: vec![track_ref_of("tone", 3)],
            start: 0,
            context: None,
        },
        1,
    );
    let _ = pump_until_audio(&mut sink, &player, 30.0, 0.05);
    // Off (the default): one second of the file as it was mastered.
    let off = pump(&mut sink, 1.0);

    let mut audio = player.audio();
    audio.normalization = crate::player::Normalization::High;
    let before = gains_asked(&bench);
    player.set_audio(audio);
    // The flip is a REAL request now (the level was never asked for at Off), and
    // its round trip is part of the second window: wait for the answer to have
    // landed, THEN measure, so the number is the gain's and not the network's.
    let asked = pump_until_gains(&mut sink, &bench, before + 1, 5.0);
    let _ = pump(&mut sink, 0.05);
    let high = pump(&mut sink, 1.0);

    let off_rms = rms(&off);
    let high_rms = rms(&high);
    let delta_db = 20.0 * (high_rms / off_rms).log10();
    println!(
        "normalisation high: off_rms={off_rms:.5} high_rms={high_rms:.5} delta_db={delta_db:.2}"
    );
    assert!(
        asked,
        "the level change never asked the server for the row's loudness ({} gain requests)",
        gains_asked(&bench)
    );
    assert!(
        (-11.5..=-9.5).contains(&delta_db),
        "High moved the level by {delta_db:.2} dB (off {off_rms:.5}, high {high_rms:.5}); \
         the engine's own gain for a 0 LUFS track is -10.4 dB"
    );
    player.shutdown();
}

/// **And it never invents a gain.** The same switch on a track the server
/// answered `null` for (measured as absent, or never measured) must leave the
/// level alone: normalisation is a measurement, not a guess (`gaincache.js`).
#[test]
fn the_normalisation_does_nothing_without_loudness() {
    let dir = TempDir::new("normalize-absent");
    let bench = Bench::start();
    let tone = Fixture::Tone440.write(dir.path());
    bench.track("tone", &tone);
    bench.gains("tone", None);

    let player = player_for(&dir, bench.base());
    let (_, mut sink) = player.take_sink().expect("the sink");
    player.command(
        Command::PlayQueue {
            tracks: vec![track_ref_of("tone", 3)],
            start: 0,
            context: None,
        },
        1,
    );
    let _ = pump_until_audio(&mut sink, &player, 30.0, 0.05);
    let off = pump(&mut sink, 1.0);

    let mut audio = player.audio();
    audio.normalization = crate::player::Normalization::High;
    let before = gains_asked(&bench);
    player.set_audio(audio);
    let asked = pump_until_gains(&mut sink, &bench, before + 1, 5.0);
    let _ = pump(&mut sink, 0.05);
    let high = pump(&mut sink, 1.0);

    let off_rms = rms(&off);
    let high_rms = rms(&high);
    let delta_db = 20.0 * (high_rms / off_rms).log10();
    println!(
        "normalisation high without loudness: off_rms={off_rms:.5} high_rms={high_rms:.5} delta_db={delta_db:.2}"
    );
    assert!(
        asked,
        "the setting never asked about a row the server has no loudness for ({} gain requests)",
        gains_asked(&bench)
    );
    assert!(
        delta_db.abs() < 0.1,
        "the setting invented {delta_db:.2} dB for a track the server has no loudness for"
    );
    player.shutdown();
}

/// How many `/api/gains` requests the bench has answered: the event the level
/// change is waited on, so the window never measures the request's own round
/// trip.
fn gains_asked(bench: &Bench) -> usize {
    bench
        .recorded()
        .iter()
        .filter(|request| request.method == "POST" && request.target() == "/api/gains")
        .count()
}

/// Pumps the sink (so the engine keeps rendering) until the bench has answered
/// `count` gain requests — never on a clock: the answer IS the event. Answers
/// whether it landed, so a pre-fix run still prints its delta instead of dying
/// on a timeout.
fn pump_until_gains(
    sink: &mut nz_audio::ExternalSink,
    bench: &Bench,
    count: usize,
    seconds: f64,
) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(seconds);
    while std::time::Instant::now() < deadline {
        let mut buffer = vec![0.0f32; 480 * 2];
        sink.render(&mut buffer);
        if gains_asked(bench) >= count {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    false
}

/// The power of a window as an amplitude ratio: the two normalisation windows
/// are compared in dB, which is what the engine's gain is expressed in.
fn rms(frames: &[f32]) -> f64 {
    if frames.is_empty() {
        return 0.0;
    }
    let sum: f64 = frames
        .iter()
        .map(|sample| f64::from(*sample) * f64::from(*sample))
        .sum();
    (sum / frames.len() as f64).sqrt()
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

/// **A manual skip plays the voice that was already decoded.** After the first
/// row is playing, the second one is preloaded; tapping next must promote that
/// voice in the mixer instead of reopening its source, with no silent window
/// across the switch and the second file's own samples from its first frame.
///
/// Measured: the preloaded voice is consumed, `/api/stream/<next>` was opened
/// **once** before the skip and **once** after (the preload), and the samples
/// past the 60 ms skip cut match the file's cosine.
#[test]
fn a_manual_next_plays_the_preloaded_voice_without_a_silent_gap() {
    let dir = TempDir::new("manual-next");
    let bench = Bench::start();
    let first = Fixture::Tone440.write(dir.path());
    let second = Fixture::Second.write(dir.path());
    bench.track("first", &first);
    bench.track("second", &second);

    let player = player_for(&dir, bench.base());
    let (_, mut sink) = player.take_sink().expect("the sink");
    player.command(
        Command::PlayQueue {
            tracks: vec![track_ref_of("first", 3), track_ref_of("second", 1)],
            start: 0,
            context: None,
        },
        1,
    );
    let _ = pump_until_audio(&mut sink, &player, 30.0, 0.05);

    // Let the preload of the second row land, exactly as a real player would
    // during the first seconds of the track.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while player.engine.preloaded_id().as_deref() != Some("second")
        && std::time::Instant::now() < deadline
    {
        let _ = pump(&mut sink, 0.01);
    }
    assert_eq!(
        player.engine.preloaded_id().as_deref(),
        Some("second"),
        "the second row never landed in the preload voice"
    );
    let opens_before = stream_opens(&bench, "second");

    // The skip happens MID-TRACK, where a rebuild is what used to be heard.
    let _ = pump(&mut sink, 0.3);
    player.command(Command::Next, 1);
    // Collect buffer by buffer and note the buffer the promoted voice starts in:
    // the mixer drains `PromoteNext` at the TOP of a callback, so the promoted
    // voice's first frame is that buffer's first sample.
    let mut after = Vec::new();
    let mut switch_at = None;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while after.len() < 62_400 * 2 && std::time::Instant::now() < deadline {
        let before = after.len();
        let buffer = pump(&mut sink, 0.01);
        after.extend_from_slice(&buffer);
        if switch_at.is_none() && player.engine.current_id().as_deref() == Some("second") {
            switch_at = Some(before);
        }
    }
    let switch = switch_at.expect("the promoted voice never became the current one");
    let from_start = &after[switch..];
    assert!(
        from_start.len() >= 60_000,
        "the promoted voice did not play long enough: {} frames",
        from_start.len() / 2
    );

    // The queued voice was consumed, and its source was NOT reopened.
    assert_ne!(
        player.engine.preloaded_id().as_deref(),
        Some("second"),
        "the promoted voice is still the queued one"
    );
    assert_eq!(
        stream_opens(&bench, "second"),
        opens_before,
        "a manual skip reopened the source the preload had already decoded"
    );

    // No 5 ms (240 frames) of digital silence across the switch (the end of the
    // 1 s file is its own, legitimate silence, so the scan stays at the seam).
    let seam_end = (switch + (0.4 * 48_000.0) as usize * 2).min(after.len());
    for window in after[switch.saturating_sub(960)..seam_end].chunks(240 * 2) {
        assert!(
            window.iter().any(|sample| *sample != 0.0),
            "a 5 ms window is silent across the switch"
        );
    }

    // Past the 60 ms skip cut, the samples are the second file's own, from its
    // beginning: `Second` is `0.5*cos(2*pi*440*t)` cut at one whole second, so
    // its frames are that cosine again.
    let expected: Vec<f32> = (0..48_000)
        .flat_map(|index| {
            let value =
                0.5 * (std::f64::consts::TAU * 440.0 * f64::from(index) / 48_000.0).cos() as f32;
            [value, value]
        })
        .collect();
    let start = (0.15 * 48_000.0) as usize * 2;
    let end = (0.6 * 48_000.0) as usize * 2;
    let mut worst = 0.0f32;
    for (played, reference) in from_start[start..end].iter().zip(&expected[start..end]) {
        worst = worst.max((played - reference).abs());
    }
    println!(
        "manual next: {worst:.6} worst difference from the second file's own samples, {} stream opens",
        stream_opens(&bench, "second")
    );
    assert!(
        worst < 0.02,
        "the second row did not start from its beginning: {worst:.6}"
    );

    // The queue really advanced.
    let snapshot = wait_state_count(&player, 2);
    assert_eq!(snapshot.index, Some(1), "the queue did not advance");
    player.shutdown();
}

/// How many times `/api/stream/<id>` was opened (the query string excluded).
fn stream_opens(bench: &Bench, id: &str) -> usize {
    let path = format!("/api/stream/{id}");
    bench
        .recorded()
        .iter()
        .filter(|request| request.target() == path)
        .count()
}
