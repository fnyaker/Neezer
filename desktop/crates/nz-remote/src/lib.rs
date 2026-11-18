//! `nz-remote` — remote control, shared by the desktop app and the mobile core.
//!
//! Extracted from `desktop/app/src/wiring/remote_host.rs` and
//! `remote_controller.rs` (SC5, `docs/agents/mobile.md`): the protocol state
//! machines of both sides live here, and those two files are now the adapters
//! that keep their `AppState`/`cx` seam. The desktop's behaviour and its tests
//! did not change; the phone's core (MB19) calls the same code.
//!
//! Two modules, one per side, and they are NOT one module on purpose: the
//! controlled player and the controller have their own constants (two
//! `RETRY_MAX`, one 15 s for a lent player and one 8 s for a loan), their own
//! `TICK`, and a screen imports one side's names. `host` is
//! [`host::RemoteHost`] (this device's player, lent), `controller` is
//! [`controller::Controller`] (this device driving somebody else's).
//!
//! The rules both sides share, and where they are written down:
//!
//! - **Polling, never a held connection** ([party-remote.md](../../../../docs/agents/party-remote.md)).
//!   One `GET` every 700 ms (1.5 s when nobody drives) for the host, one
//!   `/remote/state` a second for the controller, and both stop when the grant
//!   ends. Nothing here holds a socket open.
//! - **Commands are never queued across an outage.** A tap that cannot be posted
//!   is dropped and reported ([`controller::Controller::flush`]); a host that
//!   cannot poll keeps its player playing and backs off.
//! - **Rule 1**: every request is [`nz_api::Lane::User`], the lane that never
//!   waits — somebody else's tap must not cost the owner a frame of their own
//!   music — and both loops stop rather than slow when nothing is lent.
//! - **Every task is owned by the app that spawned it.** The loops run on the
//!   caller's runtime and end when it is stopped, its `stop()` flag set; nothing
//!   here starts a thread of its own, so nothing can be left blocked in
//!   `Handle::block_on` on a runtime that is being dropped.
//!
//! Nothing here names GPUI, an app state, `cpal`, `zbus` or `whisper`, so the
//! crate compiles for Android (`cargo ndk -t arm64-v8a -P 26 check -p nz-remote`).
//!
//! `#![forbid(unsafe_code)]`: neither file has an `unsafe` block, and keeping it
//! that way is cheaper than reviewing one.

#![forbid(unsafe_code)]

pub mod controller;
pub mod host;

#[cfg(test)]
mod testing;
