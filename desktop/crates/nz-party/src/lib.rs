//! The listen party, shared by the desktop app and the mobile core.
//!
//! SC6 moved `desktop/app/src/wiring/party_guest.rs` here whole (it had no GPUI
//! use) and the protocol part of `party_host.rs` with it: the wire types, the
//! fit, the decision, the clock probe, the publish/poll/chunk loops and the
//! scheduler. What stayed in `desktop/app/src/wiring/party_host.rs` is the
//! adapter that has to know this app — an `AppState`'s player and engine, the
//! shell's `Context` and its toast — and `party_guest.rs` is now a
//! `pub use nz_party::guest::*` so every screen keeps its path.
//!
//! The guest's planner uses [`appcore::sync`]'s estimators — the SAME Rust the
//! page runs, so the desktop, the phone and the page schedule on one arithmetic
//! — and hands chunks to [`nz_audio::PartyChannel`] with the exact OUTPUT-stream
//! rate. On Android that clock comes from SC2's external sink, so the guest only
//! reads `Engine::output_clock()` and never invents one.
//!
//! The modules are the areas a rule belongs to:
//!
//! - [`timeline`] — the line `{t, p}`, the chunk geometry and the ONE correction
//!   rule, all pure;
//! - [`planner`] — the guest's brain: where this device is, where the room is,
//!   what to place next (pure, so ten minutes of drift is an assertion);
//! - [`clock`] — the NTP-style probe both sides need, and this process's own
//!   monotonic millisecond domain;
//! - [`guest`] — the guest session: poll loop, chunk fetch and decode, the
//!   scheduler thread, the view the screen paints;
//! - [`host`] — hosting: the 250 ms measure tick, the fitted line, the handover
//!   the guests are told about, the publish loop and the recovery after a
//!   server restart.
//!
//! `nz-audio` is declared with `default-features = false` for the same reason as
//! `nz-analysis`: `cpal` must never reach the Android build through feature
//! unification. The crate links no GPUI, so `cargo ndk -t arm64-v8a -P 26 check
//! -p nz-party` builds it for the phone.
//!
//! Every thread or task the crate starts is owned and joined ([`guest::Guest`]'s
//! scheduler thread by `stop`, the tokio tasks by the runtime that spawned
//! them), so no runtime is dropped under a blocked `Handle::block_on`.
//!
//! `#![forbid(unsafe_code)]`: neither wiring file had an `unsafe` block.

#![forbid(unsafe_code)]

pub mod clock;
pub mod guest;
pub mod host;
pub mod planner;
pub mod timeline;

pub use clock::{ServerClock, now_ms};
pub use guest::{Guest, GuestView, Phase, Status, Tone, decode_chunk};
pub use host::{Host, HostStatus, Listener};
pub use planner::{Placement, Plan, Planner, Situation};
pub use timeline::{
    CHUNK_S, Correction, OVERLAP_S, Timeline, chunk_index, correction_for, last_chunk, norm_gain,
    position_at, server_at_stream, server_time_at, timeline_of, to_stream,
};

#[cfg(test)]
mod tests;
