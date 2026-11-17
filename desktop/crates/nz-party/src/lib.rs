//! The listen party, shared: host and guest.
//!
//! **Empty on purpose.** SC0 creates the crate and its manifest so that no
//! later lot edits a shared one; **SC6** moves
//! `desktop/app/src/wiring/party_guest.rs` whole (0 GPUI) and the protocol part
//! of `party_host.rs` here, leaving that file's GPUI uses as a desktop adapter.
//!
//! The guest's planner uses `appcore::sync`'s estimators — the SAME Rust the
//! page runs, so the desktop, the phone and the page schedule on one arithmetic
//! — and hands chunks to `nz_audio::PartyChannel` with the exact OUTPUT-stream
//! rate. On Android that clock comes from SC2's external sink, so the guest only
//! reads `Engine::output_clock()` and never invents one.
//!
//! `nz-audio` is declared with `default-features = false` for the same reason as
//! `nz-analysis`: `cpal` must never reach the Android build through feature
//! unification. Every thread or task the crate starts is owned and joined, so no
//! runtime is dropped under a blocked `Handle::block_on`.
//!
//! `#![forbid(unsafe_code)]`: neither file has an `unsafe` block.

#![forbid(unsafe_code)]
