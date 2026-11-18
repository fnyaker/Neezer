//! The host's decision, as a pure function: every rule of `host.js#measure` is
//! one assertion rather than a stopwatch.

use std::time::Duration;

use nz_model::{PartyNext, PartyPublish, PartyTrack};
use nz_player::{Command, Entry, PlayerState};

use crate::host::*;

fn entry(id: &str, seconds: u32) -> Entry {
    Entry {
        id: id.to_owned(),
        deezer_id: Some(id.to_owned()),
        title: format!("Titre {id}"),
        duration: seconds,
        ..Entry::default()
    }
}

fn played_queue() -> (PlayerState, Vec<Entry>) {
    let mut player = PlayerState::new();
    let rows = vec![entry("11", 200), entry("22", 180)];
    let command = Command::play_queue(rows.clone(), 0, None);
    player.apply(command, 0);
    (player, rows)
}

fn snapshot(id: &str, heard: f64) -> Snapshot {
    Snapshot {
        id: id.to_owned(),
        track: PartyTrack {
            id: id.to_owned(),
            title: format!("Titre {id}"),
            duration: 200.0,
            ..PartyTrack::default()
        },
        carries: true,
        playing: true,
        heard,
        fade: 0.0,
        duration: 200.0,
        norm: "off",
        generation: 1,
        next: None,
    }
}

/// A tick: the line, the readings and what guests believe.
fn run(
    line: &mut Line,
    snaps: &[Snapshot],
    believed: &mut Option<PartyPublish>,
    local: &mut f64,
    since: Duration,
) -> Decision {
    let snapshot = snaps.last();
    *local += 250.0;
    let decision = decide(Tick {
        snapshot,
        line,
        gaps: &Gaps::default(),
        probe: None,
        believed: believed.as_ref(),
        since_sent: since,
        local_ms: *local,
        offset_ms: 5_000.0,
    });
    if let Some(publish) = &decision.publish {
        *believed = Some(publish.clone());
    }
    decision
}

#[test]
fn two_readings_are_not_a_line_yet() {
    let mut line = Line::new();
    let mut believed = None;
    let mut local = 1_000.0;
    // The first two readings: nothing goes out (the SPA's FIT_FIRST).
    assert!(
        run(
            &mut line,
            &[snapshot("11", 10.0)],
            &mut believed,
            &mut local,
            Duration::MAX
        )
        .publish
        .is_none()
    );
    assert!(
        run(
            &mut line,
            &[snapshot("11", 10.25)],
            &mut believed,
            &mut local,
            Duration::MAX
        )
        .publish
        .is_none()
    );
    // The third is a line: it goes out, and the guest extrapolates from it.
    let decision = run(
        &mut line,
        &[snapshot("11", 10.5)],
        &mut believed,
        &mut local,
        Duration::MAX,
    );
    let publish = decision.publish.expect("a line");
    assert!(publish.playing);
    assert_eq!(
        publish.track.as_ref().map(|track| track.id.as_str()),
        Some("11")
    );
    // The line through the three readings: position = c + time, read now.
    assert!((publish.p - 10.5).abs() < 1e-9, "{}", publish.p);
    assert!(decision.quick, "a young line is read again in 40 ms");
}

#[test]
fn an_unchanged_line_is_not_republished() {
    let mut line = Line::new();
    let mut believed = None;
    let mut local = 1_000.0;
    for step in 0..10 {
        let decision = run(
            &mut line,
            &[snapshot("11", 10.0 + f64::from(step) * 0.25)],
            &mut believed,
            &mut local,
            Duration::from_millis(250),
        );
        if step < 2 {
            continue;
        }
        if step == 2 {
            assert!(decision.publish.is_some(), "the first line goes out");
        } else {
            // Same track, same plan, a quarter of a second of progress: the
            // guests extrapolate, nothing is sent.
            assert!(decision.publish.is_none(), "step {step} republished");
        }
    }
}

#[test]
fn a_seek_is_published_at_once() {
    let mut line = Line::new();
    let mut believed = None;
    let mut local = 1_000.0;
    for step in 0..8 {
        run(
            &mut line,
            &[snapshot("11", 10.0 + f64::from(step) * 0.25)],
            &mut believed,
            &mut local,
            Duration::from_millis(250),
        );
    }
    // The person seeks 40 s forward: the reading breaks the line, the fit
    // starts over — and a line of one reading is not published. The quick
    // cadence (40 ms) is what makes the seek land in two more readings, and
    // that is the SPA's own rule (`fit.n < FIT_FIRST` returns).
    let decision = run(
        &mut line,
        &[snapshot("11", 50.0)],
        &mut believed,
        &mut local,
        Duration::ZERO,
    );
    assert!(decision.publish.is_none() && decision.quick);
    assert_eq!(line.readings(), 1, "the fit broke and started over");
    run(
        &mut line,
        &[snapshot("11", 50.25)],
        &mut believed,
        &mut local,
        Duration::from_millis(40),
    );
    let decision = run(
        &mut line,
        &[snapshot("11", 50.5)],
        &mut believed,
        &mut local,
        Duration::from_millis(40),
    );
    let publish = decision
        .publish
        .expect("a seek is published once its line exists");
    assert!((publish.p - 50.5).abs() < 1e-9, "{}", publish.p);
}

#[test]
fn another_track_or_another_mapping_starts_a_new_line() {
    let mut line = Line::new();
    let mut local = 0.0;
    for step in 0..5 {
        local += 250.0;
        line.add(
            "11",
            1,
            local,
            Reading {
                pos: f64::from(step) * 0.25,
                broke: false,
            },
        );
    }
    assert_eq!(line.readings(), 5);
    // Another track.
    line.add(
        "22",
        1,
        local + 250.0,
        Reading {
            pos: 0.0,
            broke: false,
        },
    );
    assert_eq!(line.readings(), 1);
    // The device's output mapping stepped (a new stream generation).
    for step in 0..4 {
        local += 250.0;
        line.add(
            "22",
            1,
            local,
            Reading {
                pos: f64::from(step) * 0.25,
                broke: false,
            },
        );
    }
    assert_eq!(line.readings(), 5);
    line.add(
        "22",
        2,
        local + 250.0,
        Reading {
            pos: 0.0,
            broke: false,
        },
    );
    assert_eq!(line.readings(), 1);
}

#[test]
fn a_paused_player_says_where_it_stopped_and_says_it_once() {
    let mut line = Line::new();
    let mut believed = None;
    let mut local = 1_000.0;
    for step in 0..5 {
        run(
            &mut line,
            &[snapshot("11", 20.0 + f64::from(step) * 0.25)],
            &mut believed,
            &mut local,
            Duration::from_millis(250),
        );
    }
    let mut paused = snapshot("11", 21.0);
    paused.playing = false;
    let decision = run(
        &mut line,
        &[paused.clone()],
        &mut believed,
        &mut local,
        Duration::from_millis(250),
    );
    let publish = decision.publish.expect("a pause is published");
    assert!(!publish.playing);
    assert!((publish.p - 21.0).abs() < 1e-9);
    // The same pause again: nothing (the line does not move).
    let again = run(
        &mut line,
        &[paused.clone()],
        &mut believed,
        &mut local,
        Duration::from_millis(250),
    );
    assert!(again.publish.is_none());
    // A seek while paused: published, because the place changed.
    let mut seeked = paused;
    seeked.heard = 30.0;
    let decision = run(
        &mut line,
        &[seeked],
        &mut believed,
        &mut local,
        Duration::from_millis(250),
    );
    let publish = decision.publish.expect("a paused seek is published");
    assert!((publish.p - 30.0).abs() < 1e-9);
}

#[test]
fn an_empty_queue_is_announced_once() {
    let mut line = Line::new();
    let local = 1_000.0;
    let decision = decide(Tick {
        snapshot: None,
        line: &mut line,
        gaps: &Gaps::default(),
        probe: None,
        believed: None,
        since_sent: Duration::MAX,
        local_ms: local,
        offset_ms: 5_000.0,
    });
    // Nothing was ever published: there is no "the room stopped" to say.
    assert!(decision.publish.is_none());
    let believed = Some(PartyPublish {
        track: Some(PartyTrack::default()),
        playing: true,
        ..PartyPublish::default()
    });
    let decision = decide(Tick {
        snapshot: None,
        line: &mut line,
        gaps: &Gaps::default(),
        probe: None,
        believed: believed.as_ref(),
        since_sent: Duration::MAX,
        local_ms: local,
        offset_ms: 5_000.0,
    });
    let publish = decision.publish.expect("the room is told it stopped");
    assert!(publish.track.is_none() && !publish.playing);
}

#[test]
fn the_next_track_is_announced_with_where_it_lands() {
    let (player, rows) = played_queue();
    let gaps = Gaps::default();
    let plan = next_plan(&player, &rows[0], &gaps, 0.0).expect("a handover");
    assert_eq!(
        plan.track.as_ref().map(|track| track.id.as_str()),
        Some("22")
    );
    // A cut lands on the track's end, and the gap is the SPA's 150 ms.
    assert_eq!(plan.at, 200.0);
    assert_eq!(plan.fade, 0.0);
    assert_eq!(plan.gap, 0.15);
    // A crossfade begins that much before the end.
    let plan = next_plan(&player, &rows[0], &gaps, 3.0).expect("a handover");
    assert_eq!(plan.at, 197.0);
    assert_eq!(plan.fade, 3.0);
    assert_eq!(plan.gap, 0.02);
    // A paused player announces nothing: a guest must not start a track the
    // host is not playing.
    let mut paused = player;
    paused.apply(Command::pause(), 0);
    assert!(next_plan(&paused, &rows[0], &gaps, 0.0).is_none());
}

#[test]
fn the_gap_is_learnt_from_the_players_own_handovers() {
    let mut gaps = Gaps::default();
    assert_eq!(gaps.get(0.0), 0.15);
    // This player starts a cut 0.4 s after the point its plan named.
    gaps.learn(false, 0.4);
    assert!((gaps.get(0.0) - 0.275).abs() < 1e-9);
    gaps.learn(false, 0.4);
    assert!((gaps.get(0.0) - 0.3375).abs() < 1e-9);
    // A gap no start-up could explain is not a measurement.
    let before = gaps.get(0.0);
    gaps.learn(false, 9.0);
    assert_eq!(gaps.get(0.0), before);
    // The two are learnt apart: a crossfade has its own.
    assert_eq!(gaps.get(3.0), 0.02);
}

#[test]
fn two_plans_are_the_same_plan_only_when_every_field_is() {
    let one = PartyNext {
        track: Some(PartyTrack {
            id: "22".into(),
            ..PartyTrack::default()
        }),
        at: 197.0,
        start: 0.0,
        fade: 3.0,
        gap: 0.02,
    };
    assert_eq!(next_key(Some(&one)), next_key(Some(&one.clone())));
    let mut moved = one.clone();
    moved.at = 196.0;
    assert_ne!(next_key(Some(&one)), next_key(Some(&moved)));
    assert_eq!(next_key(None), "");
}
