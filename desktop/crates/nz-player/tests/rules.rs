//! Every rule the lot names, as a test: the queue and the transport behave the
//! way `webapp/src/lib/stores.js#createPlayer` behaves.
//!
//! The numbers in the assertions are the presentation's, not a benchmark's: 3 s
//! is `prev`'s restart threshold, 0.25 s is how far before the end a seek
//! lands, and the listen seconds are what `flushListen` would have posted.

use nz_player::{Command, Context, Effect, Entry, PlayerState, Repeat, Snapshot};

fn entry(id: &str) -> Entry {
    Entry {
        id: id.to_owned(),
        title: format!("Titre {id}"),
        duration: 200,
        ..Entry::default()
    }
}

fn rows(ids: &[&str]) -> Vec<Entry> {
    ids.iter().map(|id| entry(id)).collect()
}

fn ids(queue: &[std::sync::Arc<Entry>]) -> Vec<String> {
    queue.iter().map(|e| e.id.clone()).collect()
}

fn load(effects: &[Effect]) -> Option<(String, bool, f64)> {
    effects.iter().find_map(|effect| match effect {
        Effect::Load {
            entry,
            autoplay,
            start_s,
        } => Some((entry.id.clone(), *autoplay, *start_s)),
        _ => None,
    })
}

fn listen(effects: &[Effect]) -> Option<(String, u32, Option<String>)> {
    effects.iter().find_map(|effect| match effect {
        Effect::ReportListen {
            id,
            listened_s,
            next_id,
            ..
        } => Some((id.clone(), *listened_s, next_id.clone())),
        _ => None,
    })
}

fn preload(effects: &[Effect]) -> Option<Option<String>> {
    effects.iter().find_map(|effect| match effect {
        Effect::PreloadNext(entry) => Some(entry.as_ref().map(|e| e.id.clone())),
        _ => None,
    })
}

fn need_more(effects: &[Effect]) -> Option<Context> {
    effects.iter().find_map(|effect| match effect {
        Effect::NeedMore { context } => Some(context.clone()),
        _ => None,
    })
}

fn persisted(effects: &[Effect]) -> bool {
    effects.iter().any(|e| matches!(e, Effect::Persist))
}

/// A player sitting on `start` of `ids`, playing.
fn playing(ids: &[&str], start: usize) -> PlayerState {
    let mut state = PlayerState::new();
    state.set_shuffle_seed(7);
    let effects = state.apply(Command::play_queue(rows(ids), start, None), 0);
    assert!(load(&effects).is_some(), "a new queue loads its row");
    state
}

// -- playQueue ---------------------------------------------------------------

#[test]
fn play_queue_loads_the_tapped_row_and_preloads_the_next() {
    let mut state = PlayerState::new();
    let effects = state.apply(
        Command::play_queue(rows(&["a", "b", "c"]), 2, Some(Context::new("album"))),
        0,
    );
    assert_eq!(load(&effects), Some(("c".into(), true, 0.0)));
    assert_eq!(
        preload(&effects),
        None,
        "nothing to preload AND nothing announced before: no second voice to clear"
    );
    assert_eq!(state.current().map(|e| e.id.as_str()), Some("c"));
    assert_eq!(state.index(), 2);
    assert!(state.playing());
    assert!(persisted(&effects));

    let effects = state.apply(Command::play_queue(rows(&["a", "b", "c"]), 0, None), 1_000);
    assert_eq!(load(&effects), Some(("a".into(), true, 0.0)));
    assert_eq!(preload(&effects), Some(Some("b".into())));
    assert_eq!(state.context(), None);
}

#[test]
fn play_queue_does_not_shift_the_start_when_a_row_is_unplayable() {
    // What `clean()` dropping an unplayable row above the tapped one would do:
    // start the wrong track. `start` is an index into what the person SAW.
    let mut state = PlayerState::new();
    let tracks = vec![Entry::new(""), entry("a"), entry("b")];
    let effects = state.apply(Command::play_queue(tracks, 1, None), 0);
    assert_eq!(load(&effects), Some(("a".into(), true, 0.0)));
    assert_eq!(ids(state.queue()), vec!["a", "b"]);
}

#[test]
fn a_queue_of_only_unplayable_rows_is_refused() {
    let mut state = PlayerState::new();
    let effects = state.apply(
        Command::play_queue(vec![Entry::new(""), Entry::default()], 0, None),
        0,
    );
    assert!(effects.is_empty(), "{effects:?}");
    assert!(state.is_empty());
    assert!(!state.playing());
}

#[test]
fn an_empty_queue_command_changes_nothing() {
    let mut state = PlayerState::new();
    assert!(
        state
            .apply(Command::play_queue(vec![], 0, None), 0)
            .is_empty()
    );
    assert!(state.apply(Command::next(), 0).is_empty());
    assert!(state.apply(Command::prev(), 0).is_empty());
    assert!(state.apply(Command::toggle(), 0).is_empty());
    assert!(state.apply(Command::play(), 0).is_empty());
    assert!(state.apply(Command::seek(10.0), 0).is_empty());
    assert!(state.apply(Command::jump(0), 0).is_empty());
    assert!(state.apply(Command::remove_at(0), 0).is_empty());
    assert!(state.apply(Command::move_to(0, 1), 0).is_empty());
    assert!(!state.playing());
}

// -- navigation --------------------------------------------------------------

#[test]
fn next_advances_and_reports_what_was_heard() {
    let mut state = playing(&["a", "b", "c"], 0);
    let effects = state.apply(Command::next(), 91_000);
    assert_eq!(load(&effects), Some(("b".into(), true, 0.0)));
    // 91 s of playing a row, reported with the row that takes over.
    assert_eq!(listen(&effects), Some(("a".into(), 91, Some("b".into()))));
    assert_eq!(preload(&effects), Some(Some("c".into())));
    assert_eq!(state.index(), 1);
}

#[test]
fn next_at_the_end_stops_unless_repeat_all() {
    let mut state = playing(&["a", "b"], 1);
    let effects = state.apply(Command::next(), 1_000);
    assert!(load(&effects).is_none(), "the queue has nothing after b");
    assert!(!state.playing());
    assert_eq!(state.index(), 1);
    assert!(
        listen(&effects).is_none(),
        "the same row is not a new listen"
    );
    assert!(need_more(&effects).is_none(), "no context to top up");

    // The same queue with repeat all: back to the top.
    let mut state = playing(&["a", "b"], 1);
    state.apply(Command::cycle_repeat(), 1_000); // off -> all
    assert_eq!(state.repeat(), Repeat::All);
    let effects = state.apply(Command::next(), 1_000);
    assert_eq!(load(&effects), Some(("a".into(), true, 0.0)));
    assert!(state.playing());
    assert_eq!(state.index(), 0);
}

#[test]
fn next_at_the_end_of_a_flow_asks_for_more() {
    let mut state = PlayerState::new();
    state.set_shuffle_seed(7);
    state.apply(
        Command::play_queue(rows(&["a"]), 0, Some(Context::new("flow"))),
        0,
    );
    // The only row is also the last: the top-up is asked for as soon as it
    // plays, so the queue never runs dry before the answer.
    let effects = state.apply(Command::set_volume(0.5), 1_000);
    assert!(need_more(&effects).is_none(), "asked once per row");

    let effects = state.apply(Command::next(), 1_000);
    assert_eq!(need_more(&effects).map(|c| c.kind), Some("flow".to_owned()));
    assert!(
        !state.playing(),
        "paused until the answer lands, like the SPA"
    );
    assert!(load(&effects).is_none());

    // The answer: one row, appended and started at once.
    let effects = state.auto_extend(vec![entry("b")], 1_100);
    assert_eq!(load(&effects), Some(("b".into(), true, 0.0)));
    assert_eq!(state.current().map(|e| e.id.as_str()), Some("b"));
    assert!(state.playing());
}

#[test]
fn a_flow_top_up_is_asked_once_per_row() {
    let mut state = PlayerState::new();
    state.set_shuffle_seed(7);
    let effects = state.apply(
        Command::play_queue(rows(&["a", "b"]), 0, Some(Context::new("flow"))),
        0,
    );
    assert!(need_more(&effects).is_none(), "nothing runs dry yet");
    let effects = state.apply(Command::next(), 1_000);
    assert_eq!(need_more(&effects).map(|c| c.kind), Some("flow".to_owned()));
    // Any other command on the same row must not ask twice (a storm of
    // `/api/flow` calls is what an unasked-once top-up looks like).
    for tick in 2..10 {
        let effects = state.apply(Command::set_volume(0.5), 1_000 * tick);
        assert!(need_more(&effects).is_none());
    }
    // Playing an album does not ask at all.
    let mut album = PlayerState::new();
    album.apply(
        Command::play_queue(rows(&["a", "b"]), 1, Some(Context::new("album"))),
        0,
    );
    assert!(need_more(&album.apply(Command::next(), 1_000)).is_none());
}

#[test]
fn prev_restarts_after_three_seconds_and_steps_back_before() {
    let mut state = playing(&["a", "b", "c"], 2);
    // 3 s is the threshold, strictly past it (`stores.js#prev`).
    state.set_position(3.5);
    let effects = state.apply(Command::prev(), 10_000);
    assert_eq!(load(&effects), Some(("c".into(), true, 0.0)));
    assert_eq!(state.index(), 2, "the same row, from the top");
    assert!(listen(&effects).is_none(), "a restart is not a new listen");

    state.set_position(2.9);
    let effects = state.apply(Command::prev(), 20_000);
    assert_eq!(load(&effects), Some(("b".into(), true, 0.0)));
    assert_eq!(state.index(), 1);
    assert_eq!(
        listen(&effects),
        Some(("c".into(), 20, Some("b".into()))),
        "10 s either side of the restart, on the same row"
    );
}

#[test]
fn prev_on_the_first_row_restarts_it() {
    let mut state = playing(&["a", "b"], 0);
    state.set_position(1.0);
    let effects = state.apply(Command::prev(), 2_000);
    assert_eq!(load(&effects), Some(("a".into(), true, 0.0)));
    assert_eq!(state.index(), 0);
}

#[test]
fn jump_to_the_row_that_is_playing_restarts_it() {
    let mut state = playing(&["a", "b", "c"], 1);
    let effects = state.apply(Command::jump(1), 5_000);
    assert_eq!(load(&effects), Some(("b".into(), true, 0.0)));
    let effects = state.apply(Command::jump(2), 6_000);
    assert_eq!(load(&effects), Some(("c".into(), true, 0.0)));
    assert_eq!(state.index(), 2);
}

#[test]
fn jump_by_id_finds_the_row_when_the_queue_moved() {
    // `commands.js#resolveIndex`: the index when it still holds that id, else
    // the nearest row carrying it. A Flow top-up may have inserted a row above.
    let mut state = playing(&["x", "a", "b", "c"], 0);
    let effects = state.apply(Command::jump_named(0, "c"), 5_000);
    assert_eq!(load(&effects), Some(("c".into(), true, 0.0)));
    assert_eq!(state.index(), 3);

    // A row that is gone: refused, not a wrong jump.
    let effects = state.apply(Command::jump_named(0, "gone"), 6_000);
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(state.index(), 3);
}

#[test]
fn a_jump_past_the_queue_is_refused() {
    let mut state = playing(&["a", "b"], 0);
    assert!(state.apply(Command::jump(9), 1_000).is_empty());
    assert_eq!(state.index(), 0);
}

// -- the queue ---------------------------------------------------------------

#[test]
fn remove_at_keeps_the_index_on_the_right_row() {
    let mut state = playing(&["a", "b", "c", "d"], 2);
    // Removing a row above the playing one moves it up.
    let effects = state.apply(Command::remove_at(0), 4_000);
    assert_eq!(ids(state.queue()), vec!["b", "c", "d"]);
    assert_eq!(state.current().map(|e| e.id.as_str()), Some("c"));
    assert_eq!(state.index(), 1);
    assert!(load(&effects).is_none(), "the playing row did not change");
    assert!(persisted(&effects), "the queue must be written");

    // Removing a row below it changes nothing about what plays.
    let effects = state.apply(Command::remove_at(2), 5_000);
    assert_eq!(ids(state.queue()), vec!["b", "c"]);
    assert_eq!(state.current().map(|e| e.id.as_str()), Some("c"));
    assert!(load(&effects).is_none());
}

#[test]
fn removing_the_playing_row_starts_the_next_one() {
    let mut state = playing(&["a", "b", "c"], 1);
    let effects = state.apply(Command::remove_at(1), 4_000);
    assert_eq!(ids(state.queue()), vec!["a", "c"]);
    assert_eq!(load(&effects), Some(("c".into(), true, 0.0)));
    assert_eq!(listen(&effects), Some(("b".into(), 4, Some("c".into()))));

    // …and removing the LAST row leaves the previous one playing.
    let effects = state.apply(Command::remove_at(1), 8_000);
    assert_eq!(ids(state.queue()), vec!["a"]);
    assert_eq!(load(&effects), Some(("a".into(), true, 0.0)));
}

#[test]
fn emptying_the_queue_stops_the_transport() {
    let mut state = playing(&["a"], 0);
    let effects = state.apply(Command::remove_at(0), 3_000);
    assert!(state.is_empty());
    assert_eq!(state.index(), -1);
    assert!(!state.playing());
    assert!(load(&effects).is_none(), "nothing left to load");
    assert_eq!(listen(&effects), Some(("a".into(), 3, None)));
}

#[test]
fn removing_by_id_leaves_the_row_it_names() {
    let mut state = playing(&["a", "b", "x", "c"], 0);
    let effects = state.apply(Command::remove_named(1, "c"), 1_000);
    assert_eq!(ids(state.queue()), vec!["a", "b", "x"]);
    assert!(load(&effects).is_none());
}

#[test]
fn move_reorders_and_the_playing_row_keeps_playing() {
    let mut state = playing(&["a", "b", "c", "d"], 1);
    let effects = state.apply(Command::move_to(3, 0), 4_000);
    assert_eq!(ids(state.queue()), vec!["d", "a", "b", "c"]);
    assert_eq!(state.current().map(|e| e.id.as_str()), Some("b"));
    assert_eq!(state.index(), 2);
    assert!(load(&effects).is_none(), "a reorder is not a navigation");
    assert!(persisted(&effects));

    // Moving the playing row itself carries its index.
    state.apply(Command::move_to(2, 3), 5_000);
    assert_eq!(ids(state.queue()), vec!["d", "a", "c", "b"]);
    assert_eq!(state.index(), 3);

    // A no-op move changes nothing at all.
    assert!(state.apply(Command::move_to(3, 3), 6_000).is_empty());
    assert!(state.apply(Command::move_to(9, 0), 6_000).is_empty());
}

#[test]
fn move_by_wire_args_compensates_for_a_shifted_index() {
    // The controller saw [a, b, c] and asked for a to land at 2; meanwhile x
    // was inserted above, so the row is at 1 now and 2 rows later is 3.
    let mut state = playing(&["x", "a", "b", "c"], 0);
    let mut args = match Command::move_to(0, 2) {
        Command::Move { args } => args,
        _ => unreachable!(),
    };
    args.id = Some("a".into());
    state.apply(Command::Move { args }, 1_000);
    assert_eq!(ids(state.queue()), vec!["x", "b", "c", "a"]);
}

#[test]
fn clear_upcoming_keeps_the_playing_row_and_ends_autoplay() {
    let mut state = PlayerState::new();
    state.set_shuffle_seed(7);
    state.apply(
        Command::play_queue(rows(&["a", "b", "c"]), 1, Some(Context::new("flow"))),
        0,
    );
    let effects = state.apply(Command::clear_upcoming(), 1_000);
    assert_eq!(ids(state.queue()), vec!["a", "b"]);
    assert!(load(&effects).is_none());
    assert!(!state.autoplay(), "a cleared queue must not refill itself");
    assert_eq!(preload(&effects), Some(None));
    // …and no top-up is asked for a queue the person just cleared.
    assert!(need_more(&effects).is_none());
}

#[test]
fn adding_to_an_empty_queue_starts_it() {
    let mut state = PlayerState::new();
    let effects = state.apply(Command::add_to_queue(rows(&["a", "b"])), 0);
    assert_eq!(load(&effects), Some(("a".into(), true, 0.0)));
    assert_eq!(preload(&effects), Some(Some("b".into())));
    assert!(state.playing());
}

#[test]
fn a_queue_column_follows_the_web_apps_caps() {
    // 2 000 rows per add, 5 000 per queue: the server's own limits.
    let many: Vec<Entry> = (0..2_500).map(|n| entry(&format!("t{n}"))).collect();
    let mut state = PlayerState::new();
    state.apply(Command::add_to_queue(many.clone()), 0);
    assert_eq!(state.len(), nz_player::MAX_ADD);
    state.apply(Command::add_to_queue(many), 1_000);
    assert_eq!(state.len(), 2 * nz_player::MAX_ADD, "appending keeps going");
    let huge: Vec<Entry> = (0..6_000).map(|n| entry(&format!("u{n}"))).collect();
    let mut state = PlayerState::new();
    state.apply(Command::play_queue(huge, 0, None), 0);
    assert_eq!(state.len(), nz_player::MAX_QUEUE);
}

#[test]
fn extend_de_duplicates_and_says_what_landed() {
    let mut state = playing(&["a", "b"], 0);
    // A radio that answers with rows already queued must not spin: 0 landed.
    let (added, effects) = state.extend(rows(&["b", "a"]), 1_000);
    assert_eq!(added, 0);
    assert!(effects.is_empty(), "{effects:?}");
    let (added, effects) = state.extend(rows(&["b", "c", "a", "d"]), 2_000);
    assert_eq!(added, 2);
    assert_eq!(ids(state.queue()), vec!["a", "b", "c", "d"]);
    assert_eq!(state.index(), 0, "extending never moves the playhead");
    assert_eq!(preload(&effects), None, "the next row did not change");
    assert!(persisted(&effects), "the longer queue must be written");
}

#[test]
fn replace_upcoming_keeps_the_playing_row() {
    let mut state = playing(&["a", "b", "c"], 1);
    // What follows `b` is replaced, `b` itself is dropped from the answer
    // (already playing): `stores.js#replaceUpcoming` de-duplicates against what
    // is kept.
    let effects = state.replace_upcoming(rows(&["c", "d", "b"]), 3_000);
    assert_eq!(ids(state.queue()), vec!["a", "b", "c", "d"]);
    assert_eq!(state.current().map(|e| e.id.as_str()), Some("b"));
    assert!(
        load(&effects).is_none(),
        "the Flow re-tune does not restart"
    );
    assert!(persisted(&effects));
}

// -- shuffle and repeat ------------------------------------------------------

#[test]
fn toggle_shuffle_keeps_the_playing_row_first_and_gives_the_order_back() {
    let mut state = PlayerState::new();
    state.set_shuffle_seed(11);
    state.apply(
        Command::play_queue(rows(&["a", "b", "c", "d", "e", "f"]), 3, None),
        0,
    );
    assert_eq!(state.current().map(|e| e.id.as_str()), Some("d"));

    state.apply(Command::toggle_shuffle(), 1_000);
    assert!(state.shuffle());
    assert_eq!(state.index(), 0);
    assert_eq!(state.current().map(|e| e.id.as_str()), Some("d"));
    let mut shuffled = ids(state.queue());
    assert_eq!(shuffled.len(), 6);
    assert_eq!(shuffled.remove(0), "d");
    shuffled.sort();
    assert_eq!(
        shuffled,
        vec!["a", "b", "c", "e", "f"],
        "a permutation, no loss"
    );

    state.apply(Command::toggle_shuffle(), 2_000);
    assert!(!state.shuffle());
    assert_eq!(ids(state.queue()), vec!["a", "b", "c", "d", "e", "f"]);
    assert_eq!(state.index(), 3, "the same row is playing");
    assert_eq!(state.current().map(|e| e.id.as_str()), Some("d"));
}

#[test]
fn a_queue_started_while_shuffled_puts_the_tapped_row_first() {
    let mut state = PlayerState::new();
    state.set_shuffle_seed(3);
    state.apply(Command::toggle_shuffle(), 0);
    let effects = state.apply(
        Command::play_queue(rows(&["a", "b", "c", "d"]), 2, None),
        100,
    );
    assert_eq!(load(&effects), Some(("c".into(), true, 0.0)));
    assert_eq!(state.index(), 0);
    assert_eq!(state.current().map(|e| e.id.as_str()), Some("c"));
    // Un-shuffling still knows the order the list came in.
    state.apply(Command::toggle_shuffle(), 200);
    assert_eq!(ids(state.queue()), vec!["a", "b", "c", "d"]);
    assert_eq!(state.index(), 2);
}

#[test]
fn un_shuffle_does_not_resurrect_a_removed_row() {
    let mut state = PlayerState::new();
    state.set_shuffle_seed(5);
    state.apply(Command::play_queue(rows(&["a", "b", "c", "d"]), 0, None), 0);
    state.apply(Command::toggle_shuffle(), 1_000);
    let at = ids(state.queue()).iter().position(|id| id == "b").unwrap();
    state.apply(Command::remove_at(at), 2_000);
    state.apply(Command::toggle_shuffle(), 3_000);
    assert_eq!(ids(state.queue()), vec!["a", "c", "d"]);
    assert_eq!(state.current().map(|e| e.id.as_str()), Some("a"));
}

#[test]
fn shuffle_play_starts_a_shuffled_queue_with_shuffle_on() {
    let mut state = PlayerState::new();
    state.set_shuffle_seed(2);
    let effects = state.apply(
        Command::shuffle_play(rows(&["a", "b", "c"]), Some(Context::new("album"))),
        0,
    );
    assert!(state.shuffle());
    assert!(load(&effects).is_some());
    assert_eq!(state.index(), 0);
    assert_eq!(state.len(), 3);
    assert_eq!(state.context().map(|c| c.kind.as_str()), Some("album"));
}

#[test]
fn cycle_repeat_walks_off_all_one() {
    let mut state = playing(&["a", "b"], 0);
    let seen: Vec<Repeat> = (0..4)
        .map(|_| {
            state.apply(Command::cycle_repeat(), 1_000);
            state.repeat()
        })
        .collect();
    assert_eq!(
        seen,
        vec![Repeat::All, Repeat::One, Repeat::Off, Repeat::All]
    );
}

#[test]
fn repeat_one_replays_the_row_and_repeat_all_wraps() {
    let mut state = playing(&["a", "b"], 0);
    state.apply(Command::cycle_repeat(), 1_000);
    state.apply(Command::cycle_repeat(), 2_000); // one
    let effects = state.track_ended(3_000);
    assert_eq!(load(&effects), Some(("a".into(), true, 0.0)));
    assert!(state.playing());
    assert!(listen(&effects).is_none(), "a loop is the same listen");

    let mut state = playing(&["a", "b"], 1);
    state.apply(Command::cycle_repeat(), 1_000); // all
    let effects = state.track_ended(2_000);
    assert_eq!(load(&effects), Some(("a".into(), true, 0.0)));
    assert_eq!(state.index(), 0);
}

#[test]
fn a_track_that_ends_advances_then_stops_and_reports_the_last_listen() {
    let mut state = playing(&["a", "b"], 0);
    let effects = state.track_ended(30_000);
    assert_eq!(load(&effects), Some(("b".into(), true, 0.0)));
    assert_eq!(listen(&effects), Some(("a".into(), 30, Some("b".into()))));

    let effects = state.track_ended(50_000);
    assert!(load(&effects).is_none());
    assert!(!state.playing());
    assert_eq!(
        listen(&effects),
        Some(("b".into(), 20, None)),
        "the last row of a queue is reported with nothing after it"
    );

    // Pressing play again plays the row that stopped, and counts it.
    let effects = state.apply(Command::play(), 60_000);
    assert!(load(&effects).is_none(), "the row is already loaded");
    assert!(state.playing());
    let effects = state.flush_listen(65_000);
    assert_eq!(
        listen(&effects),
        Some(("b".into(), 5, None)),
        "a replay is a new listen, reported by the quit flush"
    );
}

// -- the transport's settings ------------------------------------------------

#[test]
fn toggle_play_and_pause_follow_the_queue() {
    let mut state = playing(&["a", "b"], 0);
    let effects = state.apply(Command::toggle(), 1_000);
    assert!(!state.playing());
    assert!(load(&effects).is_none());
    assert!(persisted(&effects));
    state.apply(Command::toggle(), 2_000);
    assert!(state.playing());
    state.apply(Command::pause(), 3_000);
    assert!(!state.playing());
    // Pausing something already paused changes nothing.
    assert!(state.apply(Command::pause(), 4_000).is_empty());
}

#[test]
fn volume_clamps_and_unmutes() {
    let mut state = playing(&["a"], 0);
    state.apply(Command::toggle_mute(), 1_000);
    assert!(state.muted());
    let effects = state.apply(Command::set_volume(0.4), 2_000);
    assert_eq!(state.volume(), 0.4);
    assert!(
        !state.muted(),
        "turning the volume up is how a person unmutes"
    );
    assert!(persisted(&effects));

    state.apply(Command::set_volume(2.0), 3_000);
    assert_eq!(state.volume(), 1.0);
    state.apply(Command::set_volume(-1.0), 4_000);
    assert_eq!(state.volume(), 0.0);
    state.apply(Command::set_volume(f64::NAN), 5_000);
    assert_eq!(state.volume(), 0.0);
}

#[test]
fn a_seek_lands_just_before_the_end_and_reloads_at_that_offset() {
    let mut state = playing(&["a"], 0); // 200 s rows
    let effects = state.apply(Command::seek(12.5), 4_000);
    assert_eq!(load(&effects), Some(("a".into(), true, 12.5)));
    assert_eq!(state.current_time(), 12.5);
    assert!(listen(&effects).is_none(), "a seek is not a new listen");

    // 0.25 s before the end, like `runCommand`'s clamp.
    let effects = state.apply(Command::seek(999.0), 5_000);
    assert_eq!(load(&effects), Some(("a".into(), true, 199.75)));
    let effects = state.apply(Command::seek(-4.0), 6_000);
    assert_eq!(load(&effects), Some(("a".into(), true, 0.0)));
}

// -- listening ---------------------------------------------------------------

#[test]
fn a_listen_counts_playing_time_only() {
    let mut state = playing(&["a", "b"], 0);
    state.apply(Command::pause(), 5_000); // 5 s heard
    state.apply(Command::play(), 65_000); // a minute on pause
    let effects = state.apply(Command::next(), 70_000); // 5 s more
    assert_eq!(
        listen(&effects),
        Some(("a".into(), 10, Some("b".into()))),
        "10 s heard, not 70 s of wall time"
    );
}

#[test]
fn a_restart_keeps_the_listen_running() {
    // Restarting mid-track is the same listen, so its seconds are reported
    // once, at the next real change (the SPA drops the first part instead).
    let mut state = playing(&["a", "b"], 0);
    state.set_position(4.0);
    let effects = state.apply(Command::prev(), 30_000);
    assert!(listen(&effects).is_none());
    let effects = state.apply(Command::next(), 40_000);
    assert_eq!(listen(&effects), Some(("a".into(), 40, Some("b".into()))));
}

#[test]
fn the_quit_flush_reports_what_was_heard() {
    let mut state = playing(&["a", "b"], 0);
    state.apply(Command::pause(), 7_000);
    let effects = state.flush_listen(9_000);
    assert_eq!(listen(&effects), Some(("a".into(), 7, None)));
    // Flushing twice is harmless; the accumulated seconds are gone.
    let effects = state.flush_listen(10_000);
    assert_eq!(listen(&effects), Some(("a".into(), 0, None)));
}

// -- the session -------------------------------------------------------------

#[test]
fn snapshot_round_trips_the_queue_and_the_un_shuffle_order() {
    let mut state = PlayerState::new();
    state.set_shuffle_seed(13);
    state.apply(
        Command::play_queue(rows(&["a", "b", "c", "d"]), 1, Some(Context::new("album"))),
        0,
    );
    state.apply(Command::toggle_shuffle(), 1_000);
    state.apply(Command::set_volume(0.6), 2_000);
    state.apply(Command::cycle_repeat(), 3_000);
    state.set_position(42.5);

    let snapshot = state.snapshot();
    let json = serde_json::to_string(&snapshot).unwrap();
    let back: Snapshot = serde_json::from_str(&json).unwrap();
    assert_eq!(back, snapshot);

    let (restored, effects) = PlayerState::restore(back);
    assert_eq!(ids(restored.queue()), ids(state.queue()));
    assert_eq!(restored.index(), state.index());
    assert_eq!(restored.volume(), 0.6);
    assert_eq!(restored.repeat(), Repeat::All);
    assert_eq!(restored.current_time(), 42.5);
    assert!(!restored.playing(), "never auto-resume audio");
    assert_eq!(restored.context().map(|c| c.kind.as_str()), Some("album"));
    // The playhead is where it was, and the sound waits for a click.
    assert_eq!(
        load(&effects),
        Some((restored.current().unwrap().id.clone(), false, 42.5))
    );
    // …and un-shuffling still works after a restart.
    let (mut restored, _) = PlayerState::restore(restored.snapshot());
    restored.apply(Command::toggle_shuffle(), 4_000);
    assert_eq!(ids(restored.queue()), vec!["a", "b", "c", "d"]);
    assert_eq!(restored.current().map(|e| e.id.as_str()), Some("b"));
}

#[test]
fn an_empty_session_restores_to_nothing() {
    let (state, effects) = PlayerState::restore(Snapshot::default());
    assert!(state.is_empty());
    assert_eq!(state.index(), -1);
    assert!(!state.playing());
    assert!(load(&effects).is_none());
}

#[test]
fn a_context_can_be_re_labelled_without_touching_the_queue() {
    let mut state = PlayerState::new();
    state.set_shuffle_seed(7);
    state.apply(Command::play_queue(rows(&["a", "b"]), 0, None), 0);
    let effects = state.set_context(Some(Context::new("flow")), 1_000);
    assert_eq!(state.context().map(|c| c.kind.as_str()), Some("flow"));
    assert!(load(&effects).is_none());
    assert!(persisted(&effects));
    assert!(
        state
            .set_context(Some(Context::new("flow")), 2_000)
            .is_empty()
    );
}
