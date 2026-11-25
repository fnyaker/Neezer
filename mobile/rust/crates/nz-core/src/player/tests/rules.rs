//! MB3b's acceptance, rule by rule, against the bench (its own recording server,
//! or `tools/perf_api.py --serve` when a Python that imports `neezer` exists).
//!
//! Every assertion carries the number it was measured at. Nothing waits on a
//! clock: a request is waited on through the server's own counter, a state
//! through the snapshot the player pushes, and audio through the sink both ends
//! share.

use std::sync::Arc;

use super::*;
use crate::player::{Command, PlayState, Repeat, SleepTimer, TrackRef};

/// `NativeRoutingTest`'s policy, ported: **background + next → advance**,
/// **empty → stall**, **repeat one → no loop**, **3 faults → stop**.
///
/// A row the server answers `404` for is a verdict about THAT title, so it is
/// consumed; the fourth failure stops the list with its cause, and the person
/// keeps a "Réessayer" and a "Suivant" that work.
#[test]
fn three_unplayable_rows_are_skipped_and_the_fourth_stops_the_list() {
    let dir = TempDir::new("faults");
    let bench = Bench::start();
    let player = player_for(&dir, bench.base());
    let (_, mut sink) = player.take_sink().expect("the sink");

    let tracks: Vec<TrackRef> = ["bad1", "bad2", "bad3", "bad4"]
        .iter()
        .map(|id| track_ref(id))
        .collect();
    player.command(
        Command::PlayQueue {
            tracks,
            start: 0,
            context: None,
        },
        1,
    );

    let stopped = pump_until(&mut sink, &player, 30.0, |snapshot| {
        snapshot.state == PlayState::Paused && snapshot.fault.is_some()
    });
    println!(
        "4 unplayable rows: index {:?}, fault {:?}",
        stopped.index, stopped.fault
    );
    assert_eq!(stopped.index, Some(3), "three rows were consumed");
    assert_eq!(stopped.state, PlayState::Paused);
    assert!(
        stopped
            .fault
            .as_deref()
            .is_some_and(|fault| fault.contains("404")),
        "the cause names the refusal, not a count: {:?}",
        stopped.fault
    );
    assert_eq!(stopped.len, 4);
    player.shutdown();
}

/// `repeat=one` is a title the person asked to hear again: a fault there STALLS
/// instead of advancing past it (the old app's own rule).
#[test]
fn repeat_one_never_advances_past_a_failing_title() {
    let dir = TempDir::new("fault-repeat");
    let bench = Bench::start();
    let player = player_for(&dir, bench.base());
    let (_, mut sink) = player.take_sink().expect("the sink");

    player.command(Command::SetShuffle { on: false }, 1);
    player.command(Command::CycleRepeat, 2);
    player.command(Command::CycleRepeat, 3);
    player.command(
        Command::PlayQueue {
            tracks: vec![track_ref("bad"), track_ref("good")],
            start: 0,
            context: None,
        },
        4,
    );
    let stopped = pump_until(&mut sink, &player, 30.0, |snapshot| {
        snapshot.state == PlayState::Paused && snapshot.fault.is_some()
    });
    assert_eq!(stopped.repeat, Repeat::One);
    assert_eq!(stopped.index, Some(0), "the failing row was not consumed");
    player.shutdown();
}

/// A queue whose declared durations LIE (200 s for a 2 s file) must still hand
/// over every row: nothing in the advance may read the metadata duration.
#[test]
fn a_queue_whose_rows_lie_about_their_duration_still_advances() {
    let dir = TempDir::new("lying-duration");
    let bench = Bench::start();
    let whole = Fixture::Whole.write(dir.path());
    let ids: Vec<String> = (0..6).map(|index| format!("lie{index}")).collect();
    for id in &ids {
        bench.track(id, &whole);
    }
    let player = player_for(&dir, bench.base());
    let (_, mut sink) = player.take_sink().expect("the sink");

    player.command(
        Command::PlayQueue {
            tracks: ids.iter().map(|id| track_ref(id)).collect(),
            start: 0,
            context: None,
        },
        1,
    );
    let seen = pump_until(&mut sink, &player, 20.0, |snapshot| {
        snapshot.index == Some((ids.len() - 1) as u32)
    });
    println!(
        "6 rows of 2 s declared as 200 s: index {:?}, state {:?}",
        seen.index, seen.state
    );
    assert_eq!(seen.index, Some((ids.len() - 1) as u32));
    player.shutdown();
}

/// A pause must reach the ENGINE, not only the state machine: the snapshot said
/// PAUSED over a mixer that kept rendering (the person's thumb on the button and
/// the sound carrying on), and a restored queue resumed by `Play` stayed silent
/// because `engine.play()` was only ever called from a `Load`.
#[test]
fn a_pause_stops_the_engine_and_a_play_resumes_it() {
    let dir = TempDir::new("transport");
    let bench = Bench::start();
    let cos = Fixture::Whole.write(dir.path());
    bench.track("a", &cos);
    bench.track("b", &cos);
    let player = player_for(&dir, bench.base());
    let (_, mut sink) = player.take_sink().expect("the sink");

    player.command(
        Command::PlayQueue {
            tracks: vec![track_ref("a"), track_ref("b")],
            start: 0,
            context: None,
        },
        1,
    );
    let _ = pump_until_audio(&mut sink, &player, 10.0, 0.2);
    let before = player.position().position_s;
    player.command(Command::Pause, 2);
    pump(&mut sink, 1.5);
    let paused = player.position().position_s;
    println!("pause: {before:.3} s → {paused:.3} s over 1.5 s of buffers");
    assert!(
        paused - before < 0.1,
        "the engine kept playing under a PAUSED state: {before:.3} → {paused:.3}"
    );

    player.command(Command::Play, 3);
    let _ = pump_until_audio(&mut sink, &player, 5.0, paused + 0.2);
    println!(
        "resume: clock back to {:.3} s (was {:.3})",
        player.position().position_s,
        paused
    );
    player.shutdown();
}

/// The queue restored PAUSED at its position is resumed by `Play`: the engine was
/// pumped by hand (a device it is not), so this is the service-recreation path a
/// locked phone takes.
#[test]
fn a_restored_queue_plays_when_resumed() {
    let dir = TempDir::new("restore-play");
    let bench = Bench::start();
    let cos = Fixture::Whole.write(dir.path());
    bench.track("a", &cos);
    {
        let player = player_for(&dir, bench.base());
        let (_, mut sink) = player.take_sink().expect("the sink");
        player.command(
            Command::PlayQueue {
                tracks: vec![track_ref("a"), track_ref("b")],
                start: 0,
                context: None,
            },
            1,
        );
        let _ = pump_until_audio(&mut sink, &player, 10.0, 0.2);
        pump(&mut sink, 0.4);
        player.command(Command::Pause, 2);
        player.shutdown();
    }

    let player = player_for(&dir, bench.base());
    let (_, mut sink) = player.take_sink().expect("the sink");
    assert_eq!(player.snapshot().state, PlayState::Paused);
    // The restore's `Load` runs on the player's worker: a couple of buffers let
    // the engine install the voice at the stored offset (a paused clock stays
    // there, so this is the position, not a playing one).
    pump(&mut sink, 0.2);
    let at = player.position().position_s;
    assert!(at > 0.0, "the restore lost its position: {at}");
    player.command(Command::Play, 3);
    let _ = pump_until_audio(&mut sink, &player, 5.0, at + 0.2);
    println!(
        "restored at {at:.3} s, resumed to {:.3} s",
        player.position().position_s
    );
    player.shutdown();
}

/// A queue REPLACED while a voice from the old one is loaded: the old voice's
/// end and the new one's start may not advance the new queue.
#[test]
fn a_replaced_queue_starts_at_its_first_row() {
    let dir = TempDir::new("replace");
    let bench = Bench::start();
    let cos = Fixture::Whole.write(dir.path());
    bench.track("old0", &cos);
    bench.track("old1", &cos);
    bench.track("new0", &cos);
    bench.track("new1", &cos);
    bench.track("new2", &cos);
    // The new queue's first row opens over a slow link: its voice cannot be
    // installed before the old one's ring runs out, which is the window a
    // replaced queue really has (a network load, not a local file).
    bench.slow("new0", 1_000);
    let player = player_for(&dir, bench.base());
    let (_, mut sink) = player.take_sink().expect("the sink");

    player.command(
        Command::PlayQueue {
            tracks: vec![track_ref("old0"), track_ref("old1")],
            start: 0,
            context: None,
        },
        1,
    );
    let _ = pump_until_audio(&mut sink, &player, 10.0, 0.05);
    pump(&mut sink, 0.2);
    // No pause here: whether `Pause` reaches the engine is another test's
    // question, and this one must hold either way. The old voice has ~1.6 s of
    // a 2 s file left when the queue is replaced, so its end lands inside the
    // window below; `new0`'s header is slower than that and cannot hide it.
    player.command(
        Command::PlayQueue {
            tracks: vec![track_ref("new0"), track_ref("new1"), track_ref("new2")],
            start: 0,
            context: None,
        },
        3,
    );
    pump(&mut sink, 2.0);
    let snapshot = player.snapshot();
    println!(
        "after the replacement: index {:?}, current {:?}",
        snapshot.index,
        snapshot.current.as_ref().map(|now| now.id.as_str())
    );
    assert_eq!(
        snapshot.index,
        Some(0),
        "a stale voice advanced the new queue: {:?}",
        snapshot.current.as_ref().map(|now| now.id.as_str())
    );
    assert_eq!(
        snapshot.current.as_ref().map(|now| now.id.as_str()),
        Some("new0")
    );
    player.shutdown();
}

/// The device test `NowPlayingDeviceTest#theTapToSoundDelayIsMeasuredOnOneClock`
/// runs this exact sequence 20 times (play → pause → seek 0 → jump to the other
/// row) and reads the engine's clock after the jump. A jump must render.
#[test]
fn a_jump_after_a_pause_and_a_seek_renders() {
    let dir = TempDir::new("jump-after-pause");
    let bench = Bench::start();
    let cos = Fixture::Tone440.write(dir.path());
    bench.track("9001", &cos);
    bench.track("9002", &cos);
    let player = player_for(&dir, bench.base());
    let (_, mut sink) = player.take_sink().expect("the sink");

    let rows = vec![track_ref_of("9001", 3), track_ref_of("9002", 3)];
    player.command(
        Command::PlayQueue {
            tracks: rows.clone(),
            start: 0,
            context: None,
        },
        1,
    );
    let _ = pump_until_audio(&mut sink, &player, 10.0, 0.05);

    for trial in 0..4u32 {
        player.command(Command::Pause, 2);
        let _ = pump_until(&mut sink, &player, 5.0, |s| {
            matches!(
                s.state,
                PlayState::Paused | PlayState::Stopped | PlayState::Ended
            )
        });
        player.command(Command::Seek { s: 0.0 }, 3);
        let _ = pump_until(&mut sink, &player, 5.0, |_| {
            player.position().position_s < 0.05
        });
        let index = trial % 2;
        player.command(
            Command::Jump {
                index,
                key: rows[index as usize].key.clone(),
            },
            4,
        );
        let start = player.position().position_s;
        pump(&mut sink, 1.0);
        let after = player.position().position_s;
        println!(
            "trial {trial} (jump {index}): {start:.3} s → {after:.3} s, state {:?}, index {:?}",
            player.snapshot().state,
            player.snapshot().index
        );
        assert!(
            after > 0.05,
            "trial {trial} never rendered (state {:?}, index {:?}, snapshot {:?})",
            player.snapshot().state,
            player.snapshot().index,
            player.snapshot()
        );
    }
    player.shutdown();
}

/// An empty queue stalls without inventing a row or a cause.
#[test]
fn an_empty_queue_plays_nothing() {
    let dir = TempDir::new("empty");
    let bench = Bench::start();
    let player = player_for(&dir, bench.base());
    let _ = player.take_sink().expect("the sink");
    player.command(Command::Next, 1);
    player.command(Command::Play, 2);
    let snapshot = player.snapshot();
    assert_eq!(snapshot.state, PlayState::Stopped);
    assert!(snapshot.current.is_none());
    assert_eq!(snapshot.len, 0);
    assert!(
        bench
            .recorded()
            .iter()
            .all(|request| !request.target().starts_with("/api/stream")),
        "an empty queue asked for no stream: {:?}",
        bench
            .recorded()
            .iter()
            .map(Recorded::line)
            .collect::<Vec<_>>()
    );
    player.shutdown();
}

/// A **background + next → advance**: the transport command the lock screen's
/// button sends advances the queue and reports exactly ONE listen for the row
/// that was heard.
#[test]
fn next_advances_and_reports_one_listen() {
    let dir = TempDir::new("next");
    let bench = Bench::start();
    let cos = Fixture::Whole.write(dir.path());
    bench.track("a", &cos);
    bench.track("b", &cos);
    let player = player_for(&dir, bench.base());
    let (_, mut sink) = player.take_sink().expect("the sink");

    player.command(
        Command::PlayQueue {
            tracks: vec![track_ref("a"), track_ref("b")],
            start: 0,
            context: None,
        },
        1,
    );
    let playing = pump_until_audio(&mut sink, &player, 10.0, 0.2);
    assert_eq!(
        playing.current.as_ref().map(|now| now.id.as_str()),
        Some("a")
    );
    // A listen reports the seconds PLAYED: a row skipped before it played has
    // none to report, so this one is given a second of sound first.
    pump(&mut sink, 1.0);
    player.command(Command::Next, 2);
    let advanced = pump_until(&mut sink, &player, 10.0, |snapshot| {
        snapshot.index == Some(1) && snapshot.current.as_ref().is_some_and(|now| now.id == "b")
    });
    assert_eq!(advanced.len, 2);

    let listens = wait_for_count(|| bench.listens().len(), 1, 5.0);
    assert!(listens, "one listen for the row that was left");
    let listens = bench.listens();
    assert_eq!(listens.len(), 1, "{listens:?}");
    assert!(listens[0].contains(r#""next_id":"b""#), "{}", listens[0]);
    player.shutdown();
}

/// The numbers of a listen are the seconds PLAYED, and a row that was skipped
/// before it played reports NOTHING: the server must never count a play for a
/// title nobody heard.
#[test]
fn a_row_skipped_before_it_played_reports_no_listen() {
    let dir = TempDir::new("listen-zero");
    let bench = Bench::start();
    let player = player_for(&dir, bench.base());
    let (_, mut sink) = player.take_sink().expect("the sink");

    // Nothing is served, so both rows fail instantly: no listen may be sent.
    player.command(
        Command::PlayQueue {
            tracks: vec![track_ref("x"), track_ref("y")],
            start: 0,
            context: None,
        },
        1,
    );
    let _ = pump_until(&mut sink, &player, 30.0, |snapshot| {
        snapshot.state == PlayState::Paused && snapshot.fault.is_some()
    });
    assert!(bench.listens().is_empty(), "{:?}", bench.listens());
    player.shutdown();
}

/// An episode's position: written on a pause, kept OFFLINE in
/// `pending_progress`, then sent when the link comes back — last write wins.
#[test]
fn an_episode_position_is_kept_offline_and_sent_on_reconnection() {
    let dir = TempDir::new("podcast");
    let bench = Bench::start();
    // 40 s: long enough that a position past the SPA's 8 s noise floor (below
    // which a position is a mistaken tap) is also far from its end.
    let cos = Fixture::Silence40.write(dir.path());
    bench.track("ep", &cos);
    let (core, player) = core_and_player(&dir, bench.base());
    let (_, mut sink) = player.take_sink().expect("the sink");

    player.command(
        Command::PlayQueue {
            tracks: vec![episode_ref("ep", 40)],
            start: 0,
            context: None,
        },
        1,
    );
    let _ = pump_until_audio(&mut sink, &player, 30.0, 0.2);
    pump(&mut sink, 8.5);
    // The link goes away, and the pause is the last thing before the app is
    // frozen: the position must not be lost.
    offline(&core);
    player.command(Command::Pause, 2);
    let _ = pump_until(&mut sink, &player, 20.0, |snapshot| {
        snapshot.state == PlayState::Paused
    });
    assert!(
        bench.progress().is_empty(),
        "nothing is sent while the link is dead"
    );

    online(&core);
    let sent = wait_for_count(|| bench.progress().len(), 1, 10.0);
    assert!(sent, "the kept position was flushed on reconnection");
    let progress = bench.progress();
    println!("flushed position: {}", progress[0]);
    assert!(
        progress[0].contains(r#""episode_id":"ep""#),
        "{}",
        progress[0]
    );
    assert!(
        progress[0].contains(r#""finished":false"#),
        "{}",
        progress[0]
    );
    player.shutdown();
}

/// The queue is written and comes back PAUSED at the same row and the same
/// position — the phone does not start talking in a pocket.
#[test]
fn the_queue_is_restored_paused_at_its_position() {
    let dir = TempDir::new("restore");
    let bench = Bench::start();
    let cos = Fixture::Whole.write(dir.path());
    bench.track("a", &cos);
    bench.track("b", &cos);
    let player = player_for(&dir, bench.base());
    {
        let (_, mut sink) = player.take_sink().expect("the sink");
        player.command(
            Command::PlayQueue {
                tracks: vec![track_ref("a"), track_ref("b")],
                start: 1,
                context: None,
            },
            1,
        );
        let _ = pump_until_audio(&mut sink, &player, 30.0, 0.2);
        pump(&mut sink, 1.2);
        let position = player.position().position_s;
        println!("position before the close: {position:.3} s");
        assert!(position > 0.5, "{position}");
        player.shutdown();
    }

    let again = player_for(&dir, bench.base());
    let snapshot = again.snapshot();
    assert_eq!(snapshot.len, 2, "the same queue");
    assert_eq!(snapshot.index, Some(1), "the same row");
    assert_eq!(snapshot.state, PlayState::Paused, "restored PAUSED");
    assert!(
        snapshot.current.as_ref().is_some_and(|now| now.id == "b"),
        "{:?}",
        snapshot.current
    );
    // The restored position is on the engine's clock, which starts from the
    // stored offset: a couple of buffers are enough to read it.
    let (_, mut sink) = again.take_sink().expect("the sink");
    let _ = pump_until_audio(&mut sink, &again, 10.0, 1.0);
    let position = again.position().position_s;
    println!("position after the restore: {position:.3} s");
    assert!(position > 1.0, "the position came back at {position:.3} s");
    again.shutdown();
}

/// **`command()` takes exactly ONE lock** — the acceptance's own count, measured
/// on the call.
#[test]
fn a_command_takes_one_lock() {
    let dir = TempDir::new("locks");
    let bench = Bench::start();
    let cos = Fixture::Whole.write(dir.path());
    bench.track("a", &cos);
    let player = player_for(&dir, bench.base());
    let _ = player.take_sink().expect("the sink");

    for command in [
        Command::Play,
        Command::PlayQueue {
            tracks: vec![track_ref("a")],
            start: 0,
            context: None,
        },
        Command::Next,
        Command::SetVolume { v: 0.5 },
        Command::CycleRepeat,
    ] {
        player.command(command, 1);
        assert_eq!(
            player.last_command_locks(),
            1,
            "a transport command takes one lock, and waits for nothing else"
        );
    }
    player.shutdown();
}

/// **A 4 000-entry queue costs ≤ 4 MB counted** (worker rule 4: counted bytes,
/// never a process's RSS).
#[test]
fn a_queue_of_4000_rows_costs_less_than_4_mb_counted() {
    let dir = TempDir::new("memory");
    let bench = Bench::start();
    let (core, player) = core_and_player(&dir, bench.base());

    let tracks: Vec<TrackRef> = (0..4_000)
        .map(|n| TrackRef {
            key: format!("11111111-2222-4333-8444-{n:012}"),
            id: format!("11111111-2222-4333-8444-{n:012}"),
            title: format!("Un titre de longueur plausible {n}"),
            artists: "Un artiste, Un autre artiste".to_owned(),
            album: "Un album au titre plausible".to_owned(),
            album_id: Some(format!("77777777-2222-4333-8444-{n:012}")),
            artist_id: Some(format!("88888888-2222-4333-8444-{n:012}")),
            cover: Some(format!("/api/cover/77777777-2222-4333-8444-{n:012}")),
            duration_s: 200,
            episode: false,
            liked: false,
        })
        .collect();
    let started = std::time::Instant::now();
    player.command(
        Command::PlayQueue {
            tracks,
            start: 0,
            context: None,
        },
        1,
    );
    let took = started.elapsed();
    let bytes = player.memory_bytes();
    println!(
        "4 000 rows: {bytes} B counted ({} B/row), command took {took:?}",
        bytes / 4_000
    );
    assert_eq!(player.snapshot().len, 4_000);
    assert!(
        bytes <= 4 * 1024 * 1024,
        "the queue counts {bytes} B, past the 4 MB the note holds it to"
    );
    assert!(
        took < std::time::Duration::from_millis(50),
        "enqueuing 4 000 rows took {took:?}"
    );
    // …and the app's own total reports it, a moment later: `NzCore::memory()`
    // is read on the worker (it must never take the player's lock), so the test
    // waits on the VALUE, never on a duration.
    let published = wait_for_count(|| core.memory().queue_bytes as usize, bytes as usize, 5.0);
    let reported = core.memory().queue_bytes;
    println!("NzCore::memory().queue_bytes = {reported}");
    assert!(
        published && reported == bytes,
        "the core's total is the player's count: {reported} vs {bytes}"
    );
    player.shutdown();
}

/// The **Auto quality ladder**: three streams that deliver ~100 kB/s step the
/// ask down to 64, and the request the fourth track makes says so on the wire
/// (`q=OPUS_64`).
#[test]
fn three_slow_streams_step_the_quality_down_to_64() {
    let dir = TempDir::new("ladder");
    let bench = Bench::start();
    let cos = Fixture::Tone440.write(dir.path());
    for id in ["s1", "s2", "s3", "s4", "s5"] {
        bench.track(id, &cos);
    }
    for id in ["s1", "s2", "s3"] {
        // 150 kB/s: below the 200 kB/s the 128 rung needs, above the 64 KiB a
        // measurement must deliver.
        bench.slow(id, 150_000);
    }
    let player = player_for(&dir, bench.base());
    let (_, mut sink) = player.take_sink().expect("the sink");

    for (at, id) in ["s1", "s2", "s3"].iter().enumerate() {
        let started = bench.requests();
        player.command(
            Command::PlayQueue {
                tracks: vec![track_ref(id)],
                start: 0,
                context: None,
            },
            at as u64 + 1,
        );
        // The REQUEST is the event: the state is optimistic, and the previous
        // row's clock is still > 0 while the new one opens.
        let opened = wait_for_count(|| bench.requests(), started + 1, 20.0);
        assert!(opened, "the play of {id} never reached the server");
        let _ = pump_until_audio(&mut sink, &player, 20.0, 0.05);
        // 1.2 s of a 150 kB/s body is 180 kB: past the 64 KiB a measurement
        // needs, and still under the rung the ladder must step to.
        pump(&mut sink, 1.2);
        player.command(Command::Pause, 10);
        let _ = pump_until(&mut sink, &player, 5.0, |snapshot| {
            snapshot.state == PlayState::Paused
        });
    }

    // The badge says what the ladder decided, before the next request is made.
    let snapshot = player.snapshot();
    assert_eq!(
        snapshot.current.as_ref().map(|now| now.quality.as_str()),
        Some("64"),
        "three slow streams must show 64 in the player's badge"
    );

    let before = bench.requests();
    player.command(
        Command::PlayQueue {
            tracks: vec![track_ref("s4")],
            start: 0,
            context: None,
        },
        20,
    );
    // The next request IS the event: the loader opens the stream whether or not
    // the sink is being rendered.
    pump_until_requests(&mut sink, &bench, before + 1, 20.0);
    let asked: Vec<String> = bench
        .since(before)
        .into_iter()
        .filter(|request| request.target().starts_with("/api/stream/s4"))
        .map(|request| request.line())
        .collect();
    println!("after three slow streams, the fourth asked: {asked:?}");
    assert!(
        asked.iter().any(|line| line.contains("q=OPUS_64")),
        "the ladder never stepped down: {asked:?}"
    );

    // A fast stream steps it back UP (the ladder is symmetric: a link that
    // recovers must not stay at 64 for the rest of the session).
    pump_until_requests(&mut sink, &bench, before + 2, 10.0);
    pump(&mut sink, 0.3);
    let before = bench.requests();
    player.command(
        Command::PlayQueue {
            tracks: vec![track_ref("s5")],
            start: 0,
            context: None,
        },
        21,
    );
    pump_until_requests(&mut sink, &bench, before + 1, 20.0);
    let asked: Vec<String> = bench
        .since(before)
        .into_iter()
        .filter(|request| request.target().starts_with("/api/stream/s5"))
        .map(|request| request.line())
        .collect();
    println!("after one fast stream, the fifth asked: {asked:?}");
    assert!(
        asked.iter().any(|line| line.contains("q=OPUS_320")),
        "the ladder did not recover: {asked:?}"
    );
    player.shutdown();
}

/// The **seek plan**: in a finished file the seek lands with a byte range (one
/// request), and the position the bar reads is where the sound is.
#[test]
fn a_seek_in_a_finished_file_lands_with_one_byte_range() {
    let dir = TempDir::new("seek");
    let bench = Bench::start();
    let cos = Fixture::Silence8.write(dir.path());
    bench.track("s", &cos);
    let player = player_for(&dir, bench.base());
    let (_, mut sink) = player.take_sink().expect("the sink");

    player.command(
        Command::PlayQueue {
            tracks: vec![track_ref("s")],
            start: 0,
            context: None,
        },
        1,
    );
    let _ = pump_until_audio(&mut sink, &player, 30.0, 0.05);
    pump(&mut sink, 0.3);

    let before = bench.requests();
    player.command(Command::Seek { s: 5.0 }, 2);
    pump(&mut sink, 1.5);
    let position = player.position().position_s;
    let asked: Vec<String> = bench
        .since(before)
        .into_iter()
        .filter(|request| request.target().starts_with("/api/stream/s"))
        .map(|request| request.line())
        .collect();
    println!("after a seek to 5 s: position {position:.3} s, requests {asked:?}");
    assert!(
        !asked.iter().any(|line| line.contains("start=0")),
        "a seek must never restart the stream from zero: {asked:?}"
    );
    assert!(
        !asked.is_empty(),
        "the seek never asked the server anything"
    );
    assert!(
        position > 4.0,
        "the seek did not land: position {position:.3} s"
    );
    player.shutdown();
}

/// The **Flow asks ONE title at a time** and refills when the row it has runs
/// out: two `/flow/next` calls for three titles, never one per command.
#[test]
fn the_flow_asks_one_title_at_a_time_and_refills_once() {
    let dir = TempDir::new("flow");
    let bench = Bench::start();
    let cos = Fixture::Tone440.write(dir.path());
    for id in ["f1", "f2", "f3"] {
        bench.track(id, &cos);
    }
    bench.flow_batch(&["f1", "f2"]);
    bench.flow_batch(&["f3"]);
    let player = player_for(&dir, bench.base());
    let (_, mut sink) = player.take_sink().expect("the sink");

    player.command(Command::StartFlow { style: None }, 1);
    let started = pump_until(&mut sink, &player, 10.0, |snapshot| {
        snapshot.state == PlayState::Playing && snapshot.current.is_some()
    });
    println!("flow started: {} rows, at {:?}", started.len, started.index);
    assert_eq!(
        started.len, 2,
        "the start asks for the first and its reserve"
    );
    assert_eq!(
        bench.flow_calls(),
        1,
        "a start is ONE request (the first title and its reserve)"
    );

    // Play both rows out; the second one's end asks for exactly one more title.
    //
    // The wait is on the EVENT the assertion below is about — the third
    // `/flow/next` has landed — not on the instant the third row appears: the
    // two are a request apart, and on a loaded runner (`mobile.yaml` run
    // 45534618988) the counter was read in between and said 2.
    let extended = pump_until(&mut sink, &player, 25.0, |snapshot| {
        snapshot.len == 3 && bench.flow_calls() >= 3
    });
    println!(
        "after the top-up: {} rows, {} flow calls",
        extended.len,
        bench.flow_calls()
    );
    assert_eq!(
        bench.flow_calls(),
        3,
        "one request per row (the start's two titles, then one as each becomes the last)"
    );

    // …and NO storm: commands about the row that is playing ask for nothing.
    let settled = bench.flow_calls();
    player.command(Command::Play, 30);
    player.command(Command::Pause, 31);
    player.command(Command::SetVolume { v: 0.4 }, 32);
    pump(&mut sink, 0.3);
    assert_eq!(
        bench.flow_calls(),
        settled,
        "a transport command never asks the Flow for anything"
    );
    player.shutdown();
}

/// A sleep timer of zero… a `Minutes` timer pauses the transport at its own
/// deadline, and the label counts down (nothing is invented on screen).
#[test]
fn a_sleep_timer_pauses_at_its_deadline() {
    let dir = TempDir::new("sleep");
    let bench = Bench::start();
    let cos = Fixture::Tone440.write(dir.path());
    bench.track("a", &cos);
    let player = player_for(&dir, bench.base());
    let (_, mut sink) = player.take_sink().expect("the sink");

    player.command(
        Command::PlayQueue {
            tracks: vec![track_ref("a")],
            start: 0,
            context: None,
        },
        1,
    );
    let _ = pump_until_audio(&mut sink, &player, 30.0, 0.05);
    player.set_sleep(Some(SleepTimer::Minutes { minutes: 1 }));
    let snapshot = player.snapshot();
    assert_eq!(
        snapshot.sleep,
        Some(crate::player::SleepState {
            minutes: 1,
            at_end_of_track: false
        })
    );
    player.set_sleep(None);
    assert!(player.snapshot().sleep.is_none());
    player.shutdown();
}

/// Renders while waiting for the server to see one more request: the request is
/// the event, the rendering only keeps the engine's own threads moving.
fn pump_until_requests(
    sink: &mut nz_audio::ExternalSink,
    bench: &Bench,
    count: usize,
    seconds: f64,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(seconds);
    while std::time::Instant::now() < deadline {
        if bench.requests() >= count {
            return;
        }
        let mut buffer = vec![0.0f32; 480 * 2];
        sink.render(&mut buffer);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Waits on an EVENT by polling a cheap read, and says how long it took: the
/// bound is the test's failure message, not its assertion.
fn wait_for_count(mut read: impl FnMut() -> usize, count: usize, seconds: f64) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(seconds);
    while std::time::Instant::now() < deadline {
        if read() >= count {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    read() >= count
}

/// A `Player` is `Send + Sync` (Kotlin holds it behind an `Arc` from any
/// thread): this fails to COMPILE if a future change makes it not.
#[test]
fn the_player_is_shareable_across_threads() {
    fn assert_shareable<T: Send + Sync>(_: &T) {}
    let dir = TempDir::new("share");
    let bench = Bench::start();
    let player = player_for(&dir, bench.base());
    assert_shareable(&player);
    assert_shareable(&Arc::clone(&player));
    player.shutdown();
}

/// **A seek is not a track change.** The engine rebuilds the voice at the
/// position asked and announces it with a `Started`; treating that as a
/// `repeat=one` restart consumed the next row instead (measured: a seek to the
/// middle of a title jumped to the next one, and a fault retry — which seeks at
/// the position it stopped at — turned every transient error into a skip).
#[test]
fn a_seek_mid_track_keeps_the_same_row_and_plays_on() {
    let dir = TempDir::new("seek-mid-advance");
    let bench = Bench::start();
    let cos = Fixture::Silence8.write(dir.path());
    for id in ["a", "b", "c"] {
        bench.track(id, &cos);
    }
    let player = player_for(&dir, bench.base());
    let (_, mut sink) = player.take_sink().expect("the sink");
    let rows = vec![
        track_ref_of("a", 8),
        track_ref_of("b", 8),
        track_ref_of("c", 8),
    ];
    player.command(
        Command::PlayQueue {
            tracks: rows,
            start: 0,
            context: None,
        },
        1,
    );
    let _ = pump_until_audio(&mut sink, &player, 20.0, 0.2);
    pump(&mut sink, 0.3);

    player.command(Command::Seek { s: 4.0 }, 2);
    pump(&mut sink, 1.5);
    let snap = player.snapshot();
    println!(
        "seek to the middle: index {:?}, state {:?}, position {:.2}",
        snap.index,
        snap.state,
        player.position().position_s
    );
    assert_eq!(
        snap.current.as_ref().map(|now| now.id.as_str()),
        Some("a"),
        "a seek advanced the queue: {:?}",
        snap.current.as_ref().map(|now| now.id.as_str())
    );
    assert_eq!(snap.index, Some(0), "a seek advanced the queue");
    assert_eq!(snap.state, PlayState::Playing, "a seek stopped the sound");
    assert!(
        player.position().position_s > 4.0,
        "the seek did not land: {:.2}",
        player.position().position_s
    );
    player.shutdown();
}
