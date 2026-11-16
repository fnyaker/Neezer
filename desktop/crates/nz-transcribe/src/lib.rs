//! Local transcription: whisper.cpp on the GPU through Vulkan, in the server's
//! own segment format and sidecar.
//!
//! Owned by lots T1 to T3 (`docs/agents/desktop.md`, *Transcription*). The crate
//! is standalone on purpose — `neezer-transcribe` runs, is paused (`SIGSTOP`) and
//! is tested without the app — so the app is only one of its callers.
//!
//! What is finished, and what the later lots fill:
//!
//! - [`segments`] (T1): whisper's raw output → absolute, clamped, clean lines,
//!   exactly as `neezer/transcribe/engine.py` produces them.
//! - [`sidecar`] (T1): the transcript document and its SRT, the server's bytes.
//! - [`error`] (T1): the failure codes the server stores and the French sentences
//!   the person reads.
//! - [`engine`] (T2): the probe, Vulkan, the model ladder and the explained CPU
//!   fallback; [`audio`] (T2) and [`models`] (T2) are its two halves.
//! - [`worker`], [`protocol`], [`supervisor`], [`control`] (T3): the enrolled
//!   client, the server's routes, the two-process run and its control channel.
//!
//! The numbers that cross a process or a wire boundary are pinned here in T1
//! ([`segments::CHUNK_SECONDS`], [`worker::HEARTBEAT_SECONDS`],
//! [`models::ORDER_FREE`]), so the lots can be written against them and a change
//! is visible in one diff instead of three.

pub mod audio;
pub mod control;
pub mod engine;
pub mod error;
pub mod models;
pub mod protocol;
pub mod segments;
pub mod sidecar;
pub mod supervisor;
pub mod worker;

pub use engine::{Backend, FORMAT_ENGINE, FallbackCause};
pub use error::{Error, SidecarError};
pub use segments::{Segment, clean_text, normalize_segments};
pub use sidecar::Sidecar;

/// The whisper.cpp version this binary links, straight from the C build.
pub const WHISPER_VERSION: &str = whisper_rs::WHISPER_CPP_VERSION;

/// What whisper.cpp says it was built with (`whisper_print_system_info`): its
/// backend flags and the CPU features it uses.
///
/// Calling it is also the cheapest way to LINK the C library: a crate that only
/// reads [`WHISPER_VERSION`] (an `env!` from the build script) would compile
/// without ever pulling whisper.cpp into the binary (it came out at 324 KB).
/// It is safe on a machine with no Vulkan driver: it reads a compile-time
/// string and creates no instance.
pub fn system_info() -> &'static str {
    whisper_rs::print_system_info()
}

// DO NOT enumerate Vulkan devices in this process. whisper.cpp's ggml backend
// answers `ggml_backend_vk_get_device_count()` by creating a `vk::Instance`, and
// with no usable Vulkan driver (a machine without `vulkan-radeon`/`vulkan-intel`,
// or `VK_ICD_FILENAMES=/nonexistent`) that throws `vk::IncompatibleDriverError`
// through Rust as a C++ exception: the process is `terminate()`d (SIGABRT, exit
// 134). Measured on this machine: `VK_ICD_FILENAMES=/nonexistent
// neezer-transcribe --version` aborted, while the same binary reported
// `vulkan : 1 appareil(s)` (RADV 780M) normally and `0 appareil(s)` on a
// driver-less GitHub runner whose loader was fine.
//
// T2 owns the probe, and does exactly that: [`engine::Availability::detect`]
// enumerates in the `--vulkan-devices` child of this same binary and turns a
// crash or an empty list into a named [`engine::FallbackCause`]. Enumerating
// in-process still aborts, so nothing else — no `--version`, no startup path, no
// test — may call [`engine::enumerate_devices`].

#[cfg(test)]
mod tests {
    use super::{WHISPER_VERSION, system_info};

    /// Linking whisper.cpp for real, and keeping its Vulkan backend compiled in.
    #[test]
    fn whisper_cpp_is_linked_with_its_vulkan_backend() {
        let info = system_info();
        assert!(info.starts_with("WHISPER :"), "{info}");
        assert!(
            WHISPER_VERSION.split('.').count() >= 2,
            "unexpected whisper.cpp version: {WHISPER_VERSION:?}"
        );

        // Compile-time assertion, deliberately never CALLED: `whisper_rs::vulkan`
        // only exists when whisper.cpp was built with GGML_VULKAN, so dropping
        // the feature breaks this line instead of silently shipping a CPU-only
        // binary.
        let _enumerate: fn() -> Vec<whisper_rs::vulkan::VkDeviceInfo> =
            whisper_rs::vulkan::list_devices;
    }
}
