//! The platform seam: the journal (logcat on the device) and the one line that
//! is Android's and nobody else's.
//!
//! `nz-core` stays platform-free: no `JNIEnv`, no `Context`, no `Looper`
//! (`docs/agents/mobile.md`, *iOS later*). Two things still need the platform,
//! and both are `#[cfg(target_os = "android")]` so the desktop and iOS builds
//! compile neither:
//!
//! - **the journal**: `paranoid-android` routes `tracing` to logcat, which is
//!   the only place a device's causes are readable (`adb logcat -s neezer`).
//!   Installed ONCE, idempotently: `Application.onCreate` may be retried, and a
//!   second global subscriber panics.
//! - **the TLS trust store**: `rustls-platform-verifier`'s Android side must
//!   learn the `Context` before the first handshake. The JNI export lives in
//!   `nz-ffi` (it owns the `jni` dependency); this module is where the core
//!   documents the requirement and the desktop build proves it costs nothing.

/// Installs the logcat writer, once. A no-op away from Android.
pub fn install_journal() {
    #[cfg(target_os = "android")]
    {
        use std::sync::Once;
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            paranoid_android::init("neezer");
            tracing::info!(version = nz_core_version(), "neezer core starting");
        });
    }
}

/// The crate's version — the ONE value `nz_version()` answers, logged at start
/// so a device's log says which build produced it.
pub fn nz_core_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
