//! A shared library that only DEPENDS on `nz-droid-audio`, so the four JNI entry
//! points are measured where the app will really resolve them.
//!
//! `libnz_ffi.so` reaches these symbols by name at runtime (`System.loadLibrary`
//! plus `external fun`), not through Rust, and a linker that garbage-collects
//! unreferenced sections is free to drop them. So the claim "the app's library
//! exports them" is MEASURED in the place that looks most like the app: a cdylib
//! whose only relationship with `nz-droid-audio` is a dependency and this one
//! reference.
//!
//! ```sh
//! cargo ndk -t x86_64 -P 26 build -p nz-droid-audio --example jni-export-check
//! nm -D target/x86_64-linux-android/debug/examples/libjni_export_check.so \
//!     | grep AudioTrackPull     # 4 lines, all of them 'T'
//! ```
//!
//! Without the `#[used]` reference below the same command prints **nothing**:
//! that is the measurement that made it necessary, and the reason the app's
//! library (`nz-ffi`) must carry the same one line where MB3b links this crate.
#![cfg(target_os = "android")]

/// The crate's own `#[used]` table holds the four addresses; referencing it from
/// THIS library is what keeps those functions reachable in its final link.
#[used]
static KEEP_JNI_ENTRY_POINTS: &nz_droid_audio::JniEntryPoints = &nz_droid_audio::JNI_ENTRY_POINTS;
