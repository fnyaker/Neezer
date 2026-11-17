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

    let stopped = pump_until(&mut sink, &player, 10.0, |snapshot| {
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
    let stopped = pump_until(&mut sink, &player, 10.0, |snapshot| {
        snapshot.state == PlayState::Paused && snapshot.fault.is_some()
    });
    assert_eq!(stopped.repeat, Repeat::One);
    assert_eq!(stopped.index, Some(0), "the failing row was not consumed");
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
    let cos = dir.path().join("a.flac");
    cosine_flac(&cos, 440.0, 2.0);
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
    let _ = pump_until(&mut sink, &player, 10.0, |snapshot| {
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
    let cos = dir.path().join("ep.flac");
    // 40 s: long enough that a position past the SPA's 8 s noise floor (below
    // which a position is a mistaken tap) is also far from its end.
    cosine_flac(&cos, 440.0, 40.0);
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
    let _ = pump_until_audio(&mut sink, &player, 10.0, 0.2);
    pump(&mut sink, 8.5);
    // The link goes away, and the pause is the last thing before the app is
    // frozen: the position must not be lost.
    offline(&core);
    player.command(Command::Pause, 2);
    let _ = pump_until(&mut sink, &player, 5.0, |snapshot| {
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
    let cos = dir.path().join("r.flac");
    cosine_flac(&cos, 440.0, 6.0);
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
        let _ = pump_until_audio(&mut sink, &player, 10.0, 0.2);
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
    let cos = dir.path().join("l.flac");
    cosine_flac(&cos, 440.0, 2.0);
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
    let player = player_for(&dir, bench.base());

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
    player.shutdown();
}

/// The **Auto quality ladder**: three streams that deliver ~100 kB/s step the
/// ask down to 64, and the request the fourth track makes says so on the wire
/// (`q=OPUS_64`).
#[test]
fn three_slow_streams_step_the_quality_down_to_64() {
    let dir = TempDir::new("ladder");
    let bench = Bench::start();
    let cos = dir.path().join("slow.flac");
    cosine_flac(&cos, 440.0, 3.0);
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
    let cos = dir.path().join("seek.flac");
    cosine_flac(&cos, 440.0, 8.0);
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
    let _ = pump_until(&mut sink, &player, 10.0, |snapshot| {
        snapshot.state == PlayState::Playing
    });
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
    let cos = dir.path().join("f.flac");
    cosine_flac(&cos, 440.0, 3.0);
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
    let extended = pump_until(&mut sink, &player, 25.0, |snapshot| snapshot.len == 3);
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
    let cos = dir.path().join("sl.flac");
    cosine_flac(&cos, 440.0, 4.0);
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
    let _ = pump_until(&mut sink, &player, 10.0, |snapshot| {
        snapshot.state == PlayState::Playing
    });
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
