//! Onboarding and invitation redemption.
//!
//! Owned by **MB5b** (docs/agents/mobile.md, "Module and file ownership"): the
//! lot fills this file — its own `#[uniffi::export] impl` blocks, its own
//! records, its own listener — and edits no other file of the crate.
//!
//! Invitation links arrive as an `AppLink` (MB2's `links.rs`) and land on this area.
//!
//! MB0 created the file (declared in `lib.rs`) and left it empty on purpose:
//! the note does not pin this area's method names yet, and a guess here would
//! become a contract the lot has to break. What IS pinned — the shared types
//! every area reads (`TrackRow`, `AppLink`, `NzError`, `PlayerCommand`,
//! `PlayerSnapshot`) and the accessors of `NzCore` — lives in its own file.

// MB5b fills this area. UniFFI merges an `impl Screens` block written in this
// file into the ONE Kotlin `Screens` class, so no shared file is touched.
