//! whisper.cpp itself: the probe, the Vulkan device, the explained CPU
//! fallback — owned by lot T2 (`docs/agents/desktop.md`, *Transcription*).
//!
//! T1 owns the files, not the engine: what is pinned here is only the vocabulary
//! the whole crate and the app already speak, taken from `engine.py`, the
//! server's worker protocol and the note, so T2 fills the implementation without
//! renegotiating names with T3 (which sends them) or T4 (which shows them).

/// The `engine` every chunk, document and database row is stamped with
/// (`engine.FORMAT_ENGINE`, `worker.ENGINE`).
pub const FORMAT_ENGINE: &str = "whisper.cpp";

/// Which processor whisper.cpp ended up on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Backend {
    /// The GPU, through whisper.cpp's `GGML_VULKAN` backend (the default).
    Vulkan,
    /// The CPU, after a named [`FallbackCause`] — never silently.
    Cpu,
}

impl Backend {
    /// The `backend` field of the `{"event":"engine"}` control line.
    pub fn as_str(self) -> &'static str {
        match self {
            Backend::Vulkan => "vulkan",
            Backend::Cpu => "cpu",
        }
    }
}

/// Why the GPU was not used: a stable code the server stores and the UI shows,
/// plus the French sentence the person reads (rule 5: the UI says why).
///
/// The five causes are the note's, and each one is *observed*, never guessed:
/// a missing loader, a loader with no device, a context that will not start, a
/// device out of memory, or the person's own "seulement sur secteur"/CPU choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FallbackCause {
    /// No Vulkan loader on the machine (`libvulkan.so.1` absent).
    NoVulkanLoader,
    /// A loader, but `list_devices()` finds no usable ICD.
    NoVulkanDevice,
    /// A device, but whisper's context would not start on it.
    GpuInitFailed,
    /// The device ran out of memory (a too-large model for the card).
    GpuOutOfMemory,
    /// The person asked for the CPU.
    UserForcedCpu,
}

impl FallbackCause {
    /// The code the `{"event":"engine","reason":…}` line and the server store.
    pub fn code(self) -> &'static str {
        match self {
            FallbackCause::NoVulkanLoader => "no_vulkan_loader",
            FallbackCause::NoVulkanDevice => "no_vulkan_device",
            FallbackCause::GpuInitFailed => "gpu_init_failed",
            FallbackCause::GpuOutOfMemory => "gpu_out_of_memory",
            FallbackCause::UserForcedCpu => "user_forced_cpu",
        }
    }

    /// The sentence Réglages → Transcription locale shows, cause included.
    ///
    /// The ratio is the measured one (44.9× Vulkan vs 13.9× CPU on the
    /// prototype), rounded down to "3×" so the promise is never optimistic.
    pub fn explain_fr(self) -> String {
        format!(
            "Transcription sur le processeur : aucun GPU Vulkan utilisable ({}). \
             Environ 3× plus lent.",
            self.code()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Backend, FORMAT_ENGINE, FallbackCause};

    /// The engine name is a wire value: `worker.py` sends it, `job.py` stores it
    /// and the SPA displays it.
    #[test]
    fn the_engine_speaks_the_servers_names() {
        assert_eq!(FORMAT_ENGINE, "whisper.cpp");
        assert_eq!(Backend::Vulkan.as_str(), "vulkan");
        assert_eq!(Backend::Cpu.as_str(), "cpu");
    }

    /// Every fallback says its cause in the code *and* in the sentence, because
    /// "no GPU" without the reason is a bug report nobody can act on (rule 2).
    #[test]
    fn a_fallback_names_its_cause() {
        let causes = [
            (FallbackCause::NoVulkanLoader, "no_vulkan_loader"),
            (FallbackCause::NoVulkanDevice, "no_vulkan_device"),
            (FallbackCause::GpuInitFailed, "gpu_init_failed"),
            (FallbackCause::GpuOutOfMemory, "gpu_out_of_memory"),
            (FallbackCause::UserForcedCpu, "user_forced_cpu"),
        ];
        for (cause, code) in causes {
            let sentence = cause.explain_fr();
            assert!(sentence.contains(code), "{sentence}");
            assert!(sentence.contains("processeur"), "{sentence}");
            assert!(sentence.contains("3× plus lent"), "{sentence}");
        }
    }
}
