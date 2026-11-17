//! The genre studio, the genre tree and the client-side analysis.
//!
//! Owned by **MB18** (docs/agents/mobile.md, "Module and file ownership"): the
//! lot fills this file — its own `#[uniffi::export] impl` blocks, its own
//! records, its own listener — and edits no other file of the crate.
//!
//! The analyser is the SAME Rust the page compiles (`measure`, `trainer`): the phone links it natively.
//!
//! MB0 created the file (declared in `lib.rs`) and left it empty on purpose:
//! the note does not pin this area's method names yet, and a guess here would
//! become a contract the lot has to break. What IS pinned — the shared types
//! every area reads (`TrackRow`, `AppLink`, `NzError`, `PlayerCommand`,
//! `PlayerSnapshot`) and the accessors of `NzCore` — lives in its own file.

// MB18 fills this area. UniFFI merges an `impl Screens` block written in this
// file into the ONE Kotlin `Screens` class, so no shared file is touched.
