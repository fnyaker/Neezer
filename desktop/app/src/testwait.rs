//! How long a test waits for the event it expects.
//!
//! Every "pump until" loop in this crate's suite is bounded by a deadline, and
//! that deadline is a SAFETY NET, never an expectation: what a test asserts is
//! the EVENT (three downloads counted, a screen's first paint, a session
//! arriving, a sleeve landing), so a deadline that expires first either means
//! the event never came — the failure the test exists for — or that the machine
//! was too slow to deliver it in time, which is not something a test should
//! measure. The bounds used to be 5 or 10 s, and CI run 44200827967 turned that
//! into a red desktop job: `screens::album::tests::downloading_an_album_costs_
//! one_request_per_title_and_nothing_the_second_time` panicked with « timed out
//! waiting for three downloads » (`widgets::card::stub::until`) on a runner
//! that took 33.9 s where the same tree takes 15.9 s on four local cores — the
//! ten seconds were not the subject, the three downloads were.
//!
//! [`WAIT`] is deliberately generous. A real hang still fails, with the same
//! `timed out waiting for <what>` message, sixty seconds later than before, and
//! nothing asserts that an event arrived *early*: a test that finishes inside
//! the budget behaves exactly as it did.

use std::time::Duration;

/// The budget of a "pump until" loop in this suite: a hang guard, never a
/// machine-speed assertion.
pub const WAIT: Duration = Duration::from_secs(60);
