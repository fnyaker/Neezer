//! The UniFFI surface of the mobile app: ONE `cdylib` (`libnz_ffi.so`), one
//! object graph, one file per lot (docs/agents/mobile.md, "The FFI boundary").
//!
//! **One impl block per lot, in its own file.** UniFFI merges several
//! `#[uniffi::export] impl Screens` blocks written in different files of the
//! same crate into ONE Kotlin class, so `home.rs` and `search.rs` can each add
//! their own methods and a feature lot never edits a shared file. MB0 declared
//! every file of the ownership table here; a lot writes only its own.
//!
//! Rules every lot keeps:
//!
//! - **Screen-shaped records, not server JSON.** A row carries what the screen
//!   draws; a 4 000-row list never crosses whole, it is a `ListHandle`.
//! - **Reads are `async fn`** (cancellable), **writes are optimistic**, hot
//!   reads are synchronous atomics, and Rust pushes events only on CHANGE.
//! - **No lock is held while calling Kotlin**, and a Kotlin callback hands its
//!   work to a Flow at once.
//! - Every `NzError` has a French sentence through `nz_error_text`, never a
//!   code.

uniffi::setup_scaffolding!();

pub mod actions;
pub mod admin;
pub mod audiofx;
pub mod auto;
pub mod car;
pub mod core;
pub mod covers;
pub mod discovery;
pub mod entity;
pub mod errors;
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
pub mod worlds;

/// Keeps the API-26 JNI entry points in `libnz_ffi.so` (lot MB3a).
///
/// `AudioTrackPull.kt` reaches them by NAME (`System.loadLibrary` + `external
/// fun`), not through Rust, so nothing in this crate calls them and the linker is
/// free to drop an unreferenced symbol it finds in an rlib. Measured on
/// `nz-droid-audio`'s `examples/jni-export-check.rs` (a cdylib that only depends
/// on that crate): **0** of the four symbols survive without this reference, all
/// **4** with it. The `mobile.yaml` job reads them back out of the built
/// `libnz_ffi.so` on arm64-v8a, so the app's API-26 output cannot silently lose
/// its natives.
#[cfg(target_os = "android")]
#[used]
static DROID_AUDIO_JNI_ENTRY_POINTS: &nz_droid_audio::JniEntryPoints =
    &nz_droid_audio::JNI_ENTRY_POINTS;
