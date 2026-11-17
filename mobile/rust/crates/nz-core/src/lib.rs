//! The mobile app's core: every decision, every byte of network and every
//! piece of state the screens show (docs/agents/mobile.md).
//!
//! **MB0 created the modules and nothing else.** One module per area of the
//! ownership table ("Module and file ownership"), each with the lot that fills
//! it named in its own doc comment. A lot writes only its directory; a shared
//! file (`this one`, a `Cargo.toml`) is changed by asking the orchestrator,
//! never by editing it.
//!
//! The crate is platform-free on purpose: it compiles for iOS unchanged, so no
//! Android type (`JNIEnv`, `Context`, a `Looper`) and no UniFFI type crosses
//! into it. `nz-ffi` converts between this crate's types and the records the
//! Kotlin side sees.

#![forbid(unsafe_code)]

pub mod actions;
pub mod admin;
pub mod audiofx;
pub mod core;
pub mod discovery;
pub mod entity;
pub mod genres;
pub mod home;
pub mod library;
pub mod links;
pub mod lists;
pub mod nowplaying;
pub mod offline;
pub mod onboarding;
pub mod party;
pub mod player;
pub mod playlist;
pub mod podcasts;
pub mod remote;
pub mod search;
pub mod settings;
pub mod setup;
pub mod viz;

/// The version the app shows in Aide, and the ONE value `nz_version()` answers.
///
/// It is the crate's own version (the workspace's), so a release build and a
/// debug build answer the same string for the same commit. MB0 made this the
/// only working call in the whole FFI surface on purpose: it is what proves the
/// `.so` is loaded, the JNA binding found its symbols and the Rust runtime is
/// reachable from Kotlin.
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
