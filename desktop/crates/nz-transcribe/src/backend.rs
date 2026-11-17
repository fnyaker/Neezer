//! The transcription vocabulary that crosses a process, a wire and the app.
//!
//! Three places already speak these words: the supervisor sends them
//! (`{"event":"engine"}`), the settings screen shows them, and the server stores
//! the cause's code. They live apart from [`crate::engine`] on purpose: the
//! engine module links whisper.cpp and Vulkan, while the GPUI app shows this
//! vocabulary without ever loading a GPU stack (see the crate's `features`).
//! A `Build` of the app that pulled whisper.cpp into its address space would pay
//! the model's memory for a settings card.

use serde::{Deserialize, Serialize};

/// The `engine` every chunk, document and database row is stamped with
/// (`engine.FORMAT_ENGINE`, `worker.ENGINE`).
pub const FORMAT_ENGINE: &str = "whisper.cpp";

/// Which processor whisper.cpp ended up on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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

    /// The backend a control line names, or `None` for a word this build does
    /// not know (a newer supervisor, rule 7): the caller shows its raw string.
    pub fn parse(value: &str) -> Option<Backend> {
        match value {
            "vulkan" => Some(Backend::Vulkan),
            "cpu" => Some(Backend::Cpu),
            _ => None,
        }
    }
}

/// Why the GPU was not used: a stable code the server stores and the UI shows,
/// plus the French sentence the person reads (rule 5: the UI says why).
///
/// The five causes are the note's, and each one is *observed*, never guessed:
/// a missing loader, a loader with no device, a context that will not start, a
/// device out of memory, or the person's own CPU choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
    /// Every cause, in the order the note lists them: what the supervisor maps
    /// a raw `reason` code back onto, and what the settings screen enumerates.
    pub const ALL: [FallbackCause; 5] = [
        FallbackCause::NoVulkanLoader,
        FallbackCause::NoVulkanDevice,
        FallbackCause::GpuInitFailed,
        FallbackCause::GpuOutOfMemory,
        FallbackCause::UserForcedCpu,
    ];

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

    /// The cause a control line's `reason` names, or `None` for a code this
    /// build does not know.
    pub fn parse(code: &str) -> Option<FallbackCause> {
        FallbackCause::ALL
            .into_iter()
            .find(|cause| cause.code() == code)
    }

    /// The sentence Réglages → Transcription locale shows, cause included.
    ///
    /// The ratio is the measured one: **5.0×** on 176 s of speech with
    /// `ggml-base` (Vulkan 57.8× real time against CPU 11.6×, T4's bench), the
    /// largest measured on this machine (T2 saw 3.2–3.4× on shorter inputs). The
    /// sentence quotes the largest, so it never promises a speed the machine
    /// does not deliver.
    pub fn explain_fr(self) -> String {
        if self == FallbackCause::UserForcedCpu {
            return "Transcription sur le processeur : c'est votre choix. Environ 5× plus \
                    lent."
                .to_owned();
        }
        format!(
            "Transcription sur le processeur : aucun GPU Vulkan utilisable ({}). \
             Environ 5× plus lent.",
            self.code()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Backend, FallbackCause};

    /// The codes are a wire contract (the server stores them, the SPA's panel
    /// switches on them), so each one round-trips by name.
    #[test]
    fn every_cause_round_trips_through_its_server_code() {
        for cause in FallbackCause::ALL {
            assert_eq!(FallbackCause::parse(cause.code()), Some(cause));
        }
        assert_eq!(FallbackCause::parse("no_such_cause"), None);
        assert_eq!(Backend::parse("vulkan"), Some(Backend::Vulkan));
        assert_eq!(Backend::parse("cpu"), Some(Backend::Cpu));
        assert_eq!(Backend::parse("webgpu"), None);
    }

    /// The sentence says which cause, in French, and never promises a speed the
    /// measurements do not support.
    #[test]
    fn the_cpu_fallback_explains_itself() {
        let sentence = FallbackCause::NoVulkanDevice.explain_fr();
        assert!(sentence.contains("processeur"), "{sentence}");
        assert!(sentence.contains("no_vulkan_device"), "{sentence}");
        assert!(sentence.contains("5×"), "{sentence}");
        // A forced CPU is a CHOICE, and its sentence says so instead of naming
        // a GPU it never tried.
        let forced = FallbackCause::UserForcedCpu.explain_fr();
        assert!(forced.contains("votre choix"), "{forced}");
        assert!(!forced.contains("aucun GPU"), "{forced}");
    }
}
