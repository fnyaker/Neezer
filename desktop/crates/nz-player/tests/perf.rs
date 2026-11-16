//! What the state machine costs on the queue a real person loads: 4 000 rows.
//!
//! The lot's criterion is 1 ms per operation on a 4 000-entry queue, for the
//! three a click reaches — `toggleShuffle` (which reorders the whole queue),
//! `move` and `removeAt`. Each is measured in a loop of 1 000 with `Instant`, so
//! a single scheduler hiccup cannot pass for the cost, and the numbers printed
//! here are the ones in the PR.
//!
//! The advertised budget is the RELEASE one (measured with
//! `cargo test -p nz-player --release --test perf -- --nocapture`: 38.5 µs for
//! the 4 000-row shuffle). A debug build has no optimisation at all and ran
//! 158 µs idle, 570 µs with the rest of the workspace compiling on the same
//! cores — so its bound is 4 ms instead of 1 ms. That still fails CI on a 10×
//! regression while leaving a machine under load unable to turn this red on its
//! own.
//!
//! The rows are `Arc<Entry>`, which is what makes a 4 000-row shuffle a pointer
//! copy instead of 4 000 string copies (asserted at the bottom).

use std::sync::Arc;
use std::time::Instant;

use nz_player::{Command, Entry, PlayerState};

const ROWS: usize = 4_000;
const ROUNDS: usize = 1_000;

/// The advertised 1 ms in release, 4 ms in a debug build (see the header).
fn budget_ms() -> f64 {
    if cfg!(debug_assertions) { 4.0 } else { 1.0 }
}

fn queue() -> Vec<Entry> {
    (0..ROWS)
        .map(|n| Entry {
            id: format!("00000000-0000-5000-8000-{n:012}"),
            title: format!("Titre {n} d'une bibliothèque de quatre mille titres"),
            duration: 200 + (n % 60) as u32,
            ..Entry::default()
        })
        .collect()
}

/// Runs `op` `rounds` times and returns its per-call samples in ms.
fn measure(rounds: usize, mut op: impl FnMut(usize)) -> Vec<f64> {
    let mut samples = Vec::with_capacity(rounds);
    for round in 0..rounds {
        let started = Instant::now();
        op(round);
        samples.push(started.elapsed().as_secs_f64() * 1e3);
    }
    samples
}

/// Prints mean, p95 and max, and asserts the mean against the budget.
fn assert_budget(name: &str, mut samples: Vec<f64>) {
    samples.sort_by(f64::total_cmp);
    let at = |q: f64| samples[((samples.len() - 1) as f64 * q) as usize];
    let mean: f64 = samples.iter().sum::<f64>() / samples.len() as f64;
    println!(
        "{name} ({} calls): mean {mean:.3} ms, p95 {:.3}, max {:.3}",
        samples.len(),
        at(0.95),
        at(1.0)
    );
    let budget = budget_ms();
    assert!(
        mean < budget,
        "{name}: mean {mean:.3} ms is over the {budget} ms budget"
    );
}

#[test]
fn a_four_thousand_row_queue_answers_each_operation_in_under_a_millisecond() {
    let mut state = PlayerState::new();
    state.set_shuffle_seed(1);
    state.apply(Command::play_queue(queue(), 0, None), 0);
    assert_eq!(state.len(), ROWS);

    // One call of each first: the first touch of a 32 KB allocation asks the
    // kernel for pages, which is not what an operation costs once the app is up.
    state.apply(Command::toggle_shuffle(), 1_000);
    state.apply(Command::toggle_shuffle(), 1_000);
    state.apply(Command::move_to(0, 1), 1_000);

    // shuffle off -> on -> off, 1 000 times: 500 shuffles and 500 un-shuffles of
    // the whole queue.
    let samples = measure(ROUNDS, |_| {
        state.apply(Command::toggle_shuffle(), 1_000);
    });
    assert_budget("toggleShuffle/4 000 rows", samples);

    // A move from the top to the middle and back.
    let samples = measure(ROUNDS, |round| {
        if round % 2 == 0 {
            state.apply(Command::move_to(0, ROWS / 2), 2_000);
        } else {
            state.apply(Command::move_to(ROWS / 2, 0), 2_000);
        }
    });
    assert_budget("move/4 000 rows", samples);
    assert_eq!(state.len(), ROWS, "moves never lose a row");

    // Removing a row below the playhead (the index bookkeeping path), then
    // putting it back so the queue stays 4 000 rows for the next round.
    let samples = measure(ROUNDS, |round| {
        state.apply(Command::remove_at(ROWS - 1), 3_000);
        state.apply(
            Command::add_to_queue(vec![Entry {
                id: format!("restored-{ROWS}-{round}"),
                title: "Titre remis".into(),
                ..Entry::default()
            }]),
            3_000,
        );
    });
    assert_budget("removeAt/4 000 rows (remove + append)", samples);
    assert_eq!(state.len(), ROWS);
}

#[test]
fn the_queue_itself_is_shared_not_copied() {
    // The reason a shuffle is cheap: a reordered queue holds the same rows. A
    // `Vec<Entry>` queue would copy 4 000 rows (and their strings) instead.
    let mut state = PlayerState::new();
    state.set_shuffle_seed(2);
    state.apply(Command::play_queue(queue(), 0, None), 0);
    let before: Vec<Arc<Entry>> = state.queue().to_vec();
    state.apply(Command::toggle_shuffle(), 1_000);
    let same = state
        .queue()
        .iter()
        .filter(|entry| before.iter().any(|old| Arc::ptr_eq(old, entry)))
        .count();
    assert_eq!(same, ROWS, "every row is the same allocation as before");
}
