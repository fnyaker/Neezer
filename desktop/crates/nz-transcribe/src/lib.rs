//! Local transcription: whisper.cpp on the GPU through Vulkan, in the server's
//! own segment format and sidecar.
//!
//! Owned by lots T1 to T3 (`docs/agents/desktop.md`, *Transcription*). This is
//! D0's skeleton: the crate, the library, the `neezer-transcribe` binary, and
//! the two things CI has to be able to print — the whisper.cpp build that is
//! actually linked and the Vulkan backends it can see.

/// The whisper.cpp version this binary links, straight from the C build.
pub const WHISPER_VERSION: &str = whisper_rs::WHISPER_CPP_VERSION;

/// What whisper.cpp says it was built with (`whisper_print_system_info`): its
/// backend flags and the CPU features it uses.
///
/// Calling it is also the cheapest way to LINK the C library: a crate that only
/// reads [`WHISPER_VERSION`] (an `env!` from the build script) would compile
/// without ever pulling whisper.cpp into the binary.
pub fn system_info() -> &'static str {
    whisper_rs::print_system_info()
}

/// The Vulkan devices ggml can see, as `(name, free_bytes, total_bytes)`.
///
/// Empty is a supported answer — a machine with no GPU falls back to the CPU,
/// and T2 turns that into the explained fallback message. Reading it is what
/// proves the `vulkan` feature is really on: `whisper_rs::vulkan` only exists
/// when whisper.cpp was built with `GGML_VULKAN`.
pub fn vulkan_devices() -> Vec<(String, usize, usize)> {
    whisper_rs::vulkan::list_devices()
        .into_iter()
        .map(|device| (device.name, device.vram.free, device.vram.total))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{system_info, vulkan_devices};

    /// The `vulkan` feature on the pinned `whisper-rs` is what buys the GPU: if
    /// it is ever dropped from the manifest, this stops COMPILING instead of the
    /// app silently transcribing 3× slower on the CPU (rule 1).
    #[test]
    fn the_linked_whisper_cpp_has_its_vulkan_backend() {
        let info = system_info();
        assert!(info.starts_with("WHISPER :"), "{info}");

        // The call is the assertion: it links ggml's Vulkan backend, and every
        // device it reports has a name (CI's lavapipe included).
        let devices = vulkan_devices();
        assert!(devices.iter().all(|(name, ..)| !name.is_empty()));
    }
}
