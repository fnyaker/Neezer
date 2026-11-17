//! Remote control, shared: be controlled and control another player.
//!
//! **Empty on purpose.** SC0 creates the crate and its manifest so that no
//! later lot edits a shared one; **SC5** moves the protocol state machines of
//! `desktop/app/src/wiring/remote_host.rs` and `remote_controller.rs` here —
//! poll `since`/`chan`, acks, levels, the position line, command application
//! through `nz_player::Command` — and leaves the GPUI updates as desktop
//! adapters.
//!
//! It POLLS, never holds a connection, and never queues a command across an
//! outage (docs/agents/party-remote.md). Every thread or task it starts is owned
//! and joined, so nothing is left blocked in `Handle::block_on` on a runtime
//! that is being dropped.
//!
//! `#![forbid(unsafe_code)]`: neither file has an `unsafe` block, and keeping it
//! that way is cheaper than reviewing one.

#![forbid(unsafe_code)]
