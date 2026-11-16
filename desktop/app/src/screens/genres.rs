//! `genres` — stub owned by lot `D16` (`docs/agents/desktop.md`).
//!
//! D7 registers the route and this module; the lot above replaces the body and
//! keeps `Screen::new(route, state, cx)`, which is what `screens/mod.rs` calls.

pub use crate::screens::StubScreen as Screen;
