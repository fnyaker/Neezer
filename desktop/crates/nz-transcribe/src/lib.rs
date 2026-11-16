//! Local transcription: whisper.cpp on the GPU through Vulkan, in the server's
//! own segment format and sidecar.
//!
//! Owned by lots T1 to T3 (`docs/agents/desktop.md`, *Transcription*). This is
//! D0's skeleton: the crate, the library, the `neezer-transcribe` binary and
//! the one thing CI has to be able to print — the whisper.cpp build that is
//! actually linked, Vulkan included.

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

// DO NOT enumerate Vulkan devices from this crate yet. whisper.cpp's ggml
// backend answers `ggml_backend_vk_get_device_count()` by creating a
// `vk::Instance`, and with no usable Vulkan driver (a machine without
// `vulkan-radeon`/`vulkan-intel`, or `VK_ICD_FILENAMES=/nonexistent`) that
// throws `vk::IncompatibleDriverError` through Rust as a C++ exception: the
// process is `terminate()`d (SIGABRT, exit 134). Measured on this machine:
// `VK_ICD_FILENAMES=/nonexistent neezer-transcribe --version` aborted, while
// the same binary reported `vulkan : 1 appareil(s)` (RADV 780M) normally and
// `0 appareil(s)` on a driver-less GitHub runner whose loader was fine.
//
// T2 owns the probe: it must enumerate in a child process (the `engine` child
// already exists for SIGSTOP) or behind an explicit "no driver" check, and turn
// the absence into the explained CPU fallback.

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
