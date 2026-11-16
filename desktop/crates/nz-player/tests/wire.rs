//! The wire contract, proven against the SPA's own JSON.
//!
//! `tests/fixtures/commands.json` is PRODUCED BY THE JAVASCRIPT, never written
//! by hand: a sample an author typed only proves what that author believed. How
//! it was produced (recorded here because the generator is a throwaway — it
//! reads the repository's own module):
//!
//! ```text
//! node --import ./register.mjs gen.mjs > commands.json
//! ```
//!
//! with `gen.mjs` importing `webapp/src/lib/remote/commands.js#commandFor` and
//! `webapp/src/lib/trackkey.js` unmodified, and `register.mjs` a module hook
//! that stubs ONLY `webapp/src/lib/remote/settings.js` (it imports
//! `svelte/store` and the whole app, and `commandFor` never reads a setting
//! level). The fixture's `how` field records the base commit and the two blob
//! hashes it was generated from; the `seek` case comes from
//! `controller.js#watchSeek`, the one op `commandFor` never produces.
//!
//! Two tests per case: the SPA's JSON parses into the command the state machine
//! runs, and that command serialises back to the SAME bytes. A rename on either
//! side fails here instead of on a person's desktop.

use nz_player::{Command, Context, Entry, track_key};

/// The Flow context the fixture's queue carries.
fn flow() -> Context {
    Context {
        kind: "flow".into(),
        id: Some("0123456789abcdef".into()),
        title: None,
    }
}

fn fixture() -> serde_json::Value {
    let text = include_str!("fixtures/commands.json");
    serde_json::from_str(text).expect("the fixture is the JavaScript's own output")
}

fn cases(fixture: &serde_json::Value) -> &Vec<serde_json::Value> {
    fixture["cases"].as_array().expect("cases")
}

#[test]
fn slim_track_round_trips_the_row_the_lists_draw() {
    let fixture = fixture();
    let entry: Entry = serde_json::from_value(fixture["slim_track"].clone()).expect("slimTrack");
    assert_eq!(entry.id, "3f7c1d2e-9a4b-5c6d-8e0f-1a2b3c4d5e6f");
    assert_eq!(entry.title, "Balancing the Dark");
    assert_eq!(entry.duration, 312);
    assert_eq!(entry.artist_name(), "Kaya Project");
    assert!(!entry.is_explicit());
    assert!(entry.is_archived());
    assert_eq!(entry.artists.len(), 2);
    assert_eq!(
        entry.album.as_ref().map(|a| a.title.as_str()),
        Some("Songs from the Inside")
    );
    assert_eq!(entry.gain, Some(-7.25));

    // And back to exactly the object the SPA sent: no key added, none dropped.
    assert_eq!(serde_json::to_value(&entry).unwrap(), fixture["slim_track"]);
}

#[test]
fn a_catalogue_row_carries_the_uuid_its_row_will_have() {
    let fixture = fixture();
    let entry: Entry = serde_json::from_value(fixture["preview_track"].clone()).expect("slimTrack");
    // The SPA computed this id from the Deezer id alone; the desktop must land
    // on the same row or the same track would sit in the queue twice.
    assert_eq!(entry.id, "21408a08-d175-53a4-a02d-733a7bc83812");
    assert_eq!(
        track_key(None, entry.deezer_id.as_deref()),
        Some(entry.id.clone())
    );
    assert_eq!(
        serde_json::to_value(&entry).unwrap(),
        fixture["preview_track"]
    );
}

#[test]
fn every_command_the_spa_sends_parses_and_serialises_to_the_same_json() {
    let fixture = fixture();
    let mut seen = Vec::new();
    for case in cases(&fixture) {
        let name = case["name"].as_str().unwrap();
        let json = &case["json"];
        let command: Command = serde_json::from_value(json.clone())
            .unwrap_or_else(|e| panic!("{name}: the SPA's own JSON must parse: {e}\n{json}"));
        assert_eq!(
            serde_json::to_value(&command).unwrap(),
            *json,
            "{name}: the command must serialise back to the SPA's bytes"
        );
        // The op the state machine reports is the op the wire carried.
        assert_eq!(command.op(), json["op"].as_str().unwrap(), "{name}");
        seen.push(name.to_owned());
    }
    assert_eq!(
        seen.len(),
        19,
        "the fixture covers the whole vocabulary: {seen:?}"
    );
}

#[test]
fn the_spas_methods_map_onto_the_state_machines_commands() {
    let fixture = fixture();
    let by_name = |name: &str| -> Command {
        let case = cases(&fixture)
            .iter()
            .find(|c| c["name"] == name)
            .unwrap_or_else(|| panic!("case {name}"));
        serde_json::from_value(case["json"].clone()).expect("parses")
    };
    // The transport verbs, one by one.
    assert_eq!(by_name("play"), Command::play());
    assert_eq!(by_name("pause"), Command::pause());
    assert_eq!(by_name("toggle"), Command::toggle());
    assert_eq!(by_name("next"), Command::next());
    assert_eq!(by_name("prev"), Command::prev());
    assert_eq!(by_name("toggle_mute"), Command::toggle_mute());
    assert_eq!(by_name("toggle_shuffle"), Command::toggle_shuffle());
    assert_eq!(by_name("cycle_repeat"), Command::cycle_repeat());
    assert_eq!(by_name("clear_upcoming"), Command::clear_upcoming());
    assert_eq!(by_name("seek"), Command::seek(12.5));
    assert_eq!(by_name("set_volume"), Command::set_volume(0.42));
    assert_eq!(
        by_name("jump"),
        Command::jump_named(2, "5e6f7a8b-9c0d-5e1f-a2b3-c4d5e6f70819")
    );
    assert_eq!(
        by_name("remove_at"),
        Command::remove_named(1, "21408a08-d175-53a4-a02d-733a7bc83812")
    );
    match by_name("move") {
        Command::Move { args } => {
            assert_eq!((args.i, args.to), (0, 3));
            assert_eq!(
                args.id.as_deref(),
                Some("3f7c1d2e-9a4b-5c6d-8e0f-1a2b3c4d5e6f")
            );
        }
        other => panic!("move parsed as {other:?}"),
    }

    // The queue commands carry slim rows, a start and a context.
    match by_name("play_queue") {
        Command::PlayQueue { args } => {
            assert_eq!(args.tracks.len(), 4);
            assert_eq!(args.start, 1);
            assert_eq!(args.context, Some(flow()));
            assert_eq!(args.tracks[1].deezer_id.as_deref(), Some("3000001"));
        }
        other => panic!("play_queue parsed as {other:?}"),
    }
    match by_name("play_queue_no_context") {
        Command::PlayQueue { args } => assert!(args.context.is_none()),
        other => panic!("play_queue parsed as {other:?}"),
    }
    match by_name("shuffle_play") {
        Command::ShufflePlay { args } => {
            assert_eq!(args.tracks.len(), 4);
            assert_eq!(args.context, Some(flow()));
        }
        other => panic!("shuffle_play parsed as {other:?}"),
    }
    match by_name("add_to_queue") {
        Command::AddToQueue { args } => assert_eq!(args.tracks.len(), 2),
        other => panic!("add parsed as {other:?}"),
    }
    match by_name("play_next") {
        Command::PlayNext { args } => assert_eq!(args.tracks.len(), 1),
        other => panic!("play_next parsed as {other:?}"),
    }
}

#[test]
fn a_row_the_server_could_not_name_does_not_make_the_command_unreadable() {
    // `slimTrack` keeps `id: null` when the row has no key at all; the state
    // machine must read it and drop the row rather than fail the whole queue.
    let json = serde_json::json!({
        "op": "play_queue",
        "args": {"tracks": [{"id": null, "title": null}, {"id": "row", "title": "Kept"}]}
    });
    let command: Command = serde_json::from_value(json).expect("tolerant");
    match command {
        Command::PlayQueue { args } => {
            assert_eq!(args.tracks.len(), 2);
            assert!(!args.tracks[0].is_playable());
            assert!(args.tracks[1].is_playable());
        }
        other => panic!("{other:?}"),
    }
}
