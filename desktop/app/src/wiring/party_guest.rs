//! Écoute partagée — the GUEST side: a re-export of `nz-party` (SC6).
//!
//! The guest had no GPUI use (0 in 2 757 lines), so SC6 moved it WHOLE into
//! `desktop/crates/nz-party/src/guest.rs`, which the mobile core runs too
//! (`mobile/rust/crates/nz-ffi/src/party.rs`, MB20). The rules — the line, the
//! chunks on the device's own output clock, the one correction rule, rule 1's
//! `Lane::User` sync traffic, rule 3's fail-fast — and their numbers are in that
//! crate's module note; this file adds nothing.
//!
//! [`pub use nz_party::guest::*`] keeps every screen's path
//! (`crate::wiring::party_guest::{Guest, GuestView, Phase, Tone, NAME_KEY, …}`)
//! and the party screen's own engine seam (`Guest::start` takes the channel of
//! *this* app's engine) unchanged.

pub use nz_party::guest::*;
