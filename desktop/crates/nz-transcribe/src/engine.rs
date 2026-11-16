//! whisper.cpp itself: the probe, the Vulkan device, the explained CPU
//! fallback — owned by lot T2 (`docs/agents/desktop.md`, *Transcription*).
//!
//! The vocabulary ([`FORMAT_ENGINE`], [`Backend`], [`FallbackCause`]) is T1's,
//! because three places already speak it (T3 sends it, T4 shows it, the server
//! stores it). What this module adds is the engine behind it:
//!
//! - **Enumeration happens in a child process, never in this one.** whisper.cpp's
//!   `ggml_backend_vk_get_device_count()` creates a `vk::Instance`; a machine
//!   without a usable driver (no `vulkan-radeon`/`vulkan-intel`, or
//!   `VK_ICD_FILENAMES=/nonexistent`) makes that throw a C++ exception across
//!   the FFI boundary and the process is `terminate()`d — SIGABRT, exit 134,
//!   measured. So [`Availability::detect`] runs `neezer-transcribe
//!   --vulkan-devices` and reads its answer: a crash costs that child and
//!   becomes a named [`FallbackCause`], never the app's.
//! - **The ladder is observed, not guessed.** Vulkan device list → whisper
//!   context with `use_gpu(true)` → a one-second silent decode. The first GPU
//!   call compiles whisper.cpp's shaders (seconds), so it happens once per
//!   process, at load, and a failure at any rung falls back to the CPU with the
//!   cause that made it fail.
//! - **VAD is ours, because whisper-rs's is a no-op.** `FullParams::enable_vad`
//!   sets `params.vad`, but `WhisperState::full` calls `whisper_full_with_state`,
//!   and whisper.cpp only applies VAD in `whisper_full` — the flag silently does
//!   nothing (read in `whisper-rs-sys-0.15.0/whisper.cpp/src/whisper.cpp`: the
//!   two `if (params.vad)` blocks are in `whisper_full` and
//!   `whisper_full_parallel`, not in `whisper_full_with_state`). This module runs
//!   Silero VAD through the public [`whisper_rs::WhisperVadContext`] and maps the
//!   timestamps back with whisper.cpp's own algorithm, so a chunk's offsets are
//!   absolute in the *original* audio and the server's overlap rule still holds.
//!   It is **off by default**: that path is CPU-only and unbatched — measured
//!   23.6 s of CPU for 11 s of speech here, 98× the GPU decoding time it saves —
//!   so only an explicit [`EngineConfig::vad`] turns it on.
//!
//! The `engine` and `file` subcommands (`src/main.rs`) are the two callers: the
//! supervisor (T3) gives `engine` one chunk of raw 16 kHz mono `f32` and reads
//! the JSON document described in [`Document`]; `file` decodes an audio file,
//! chunks it like the server does and writes the same document for the whole
//! episode.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
    WhisperVadContext, WhisperVadParams,
};

use crate::error::Error;
use crate::models;
use crate::segments::{SAMPLE_RATE, Segment};

/// The `engine` every chunk, document and database row is stamped with
/// (`engine.FORMAT_ENGINE`, `worker.ENGINE`).
pub const FORMAT_ENGINE: &str = "whisper.cpp";

/// The argument that makes the binary enumerate Vulkan devices and print them.
///
/// Internal: it exists so enumeration can happen in a child (see the module doc),
/// and [`Availability::detect`] is the only caller. It is a flag rather than a
/// subcommand so a user typing it by accident gets devices, never a crash in the
/// app.
pub const VULKAN_DEVICES_FLAG: &str = "--vulkan-devices";

/// How much silence the warm-up decode runs through: one second is the smallest
/// input that makes whisper.cpp compile every pipeline it will use later.
const WARMUP_MS: usize = 1000;

/// The VAD model's overlap in whisper.cpp (`samples_overlap: 0.1`).
const VAD_OVERLAP_SAMPLES: i64 = SAMPLE_RATE as i64 / 10;

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
}

/// Why the GPU was not used: a stable code the server stores and the UI shows,
/// plus the French sentence the person reads (rule 5: the UI says why).
///
/// The five causes are the note's, and each one is *observed*, never guessed:
/// a missing loader, a loader with no device, a context that will not start, a
/// device out of memory, or the person's own "seulement sur secteur"/CPU choice.
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
    /// prototype, 3.2× on this machine with `ggml-base`; see the two
    /// measurements in this lot's PR), rounded down to "3×" so the promise is
    /// never optimistic.
    pub fn explain_fr(self) -> String {
        format!(
            "Transcription sur le processeur : aucun GPU Vulkan utilisable ({}). \
             Environ 3× plus lent.",
            self.code()
        )
    }
}

/// One physical GPU ggml can see (`whisper_rs::vulkan::list_devices`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    /// ggml's own device index, the one `WhisperContextParameters::gpu_device`
    /// takes.
    pub id: i32,
    /// The driver's description, e.g. `AMD Radeon 780M (RADV PHOENIX)`.
    pub name: String,
    /// Free VRAM at the moment of enumeration, in bytes (0 for a unified
    /// memory device that reports nothing).
    pub vram_free: u64,
    /// Total VRAM, in bytes.
    pub vram_total: u64,
}

/// Which processor the engine runs on, and why — the `{"event":"engine"}` line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineReport {
    /// [`Backend::Vulkan`] or [`Backend::Cpu`].
    pub backend: Backend,
    /// The device's description, `None` on the CPU.
    pub device: Option<String>,
    /// Why the GPU was not used, `None` when it is.
    pub cause: Option<FallbackCause>,
}

impl EngineReport {
    /// The CPU report, with its cause.
    pub fn cpu(cause: FallbackCause) -> Self {
        Self {
            backend: Backend::Cpu,
            device: None,
            cause: Some(cause),
        }
    }

    /// The whole control line, one JSON object and a newline: what `probe`
    /// prints and what T3's supervisor copies into its `{"event":"engine"}`.
    pub fn event_json(&self) -> String {
        let value = json!({
            "event": "engine",
            "backend": self.backend.as_str(),
            "device": self.device,
            "reason": self.cause.map(FallbackCause::code),
        });
        let mut text = serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_owned());
        // Serializer output is compact already; this keeps the contract (one
        // line) even if a future serde_json pretty-prints by default.
        text.retain(|character| character != '\n');
        text.push('\n');
        text
    }
}

/// What this machine can offer the engine, and whether the loader is there.
///
/// The devices come from a child process (module doc); the loader check only
/// runs when that child failed, so a machine where Vulkan works never pays for
/// the path scan and a NixOS install with the loader somewhere unusual is not
/// misreported.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Availability {
    /// Every device the child enumerated (empty when it crashed or found none).
    pub devices: Vec<Device>,
    /// Whether a `libvulkan.so.1` the loader could map was found.
    pub loader: bool,
}

impl Availability {
    /// Probe the machine: enumerate in a child, and look for the loader only if
    /// that did not answer.
    pub fn detect() -> Self {
        let devices = enumerate_in_child().unwrap_or_default();
        let loader = devices.is_empty() && vulkan_loader_present();
        Self { devices, loader }
    }

    /// The report, with `forced_cpu` (the `--cpu` flag, "seulement sur secteur")
    /// turning a working GPU into a named choice.
    pub fn report(&self, forced_cpu: bool) -> EngineReport {
        if forced_cpu {
            return EngineReport::cpu(FallbackCause::UserForcedCpu);
        }
        match self.devices.first() {
            Some(device) => EngineReport {
                backend: Backend::Vulkan,
                device: Some(device.name.clone()),
                cause: None,
            },
            None if self.loader => EngineReport::cpu(FallbackCause::NoVulkanDevice),
            None => EngineReport::cpu(FallbackCause::NoVulkanLoader),
        }
    }
}

/// The engine's report for this machine, without a model: what `probe` prints.
///
/// Cheap (one child, a few dozen milliseconds) and safe on a machine with no
/// driver at all — see the module doc.
pub fn probe() -> EngineReport {
    Availability::detect().report(false)
}

/// Enumerate every physical GPU. **Only call this from the `--vulkan-devices`
/// child**: on a machine with no usable driver it aborts the process (SIGABRT,
/// exit 134), which is exactly what the child exists to absorb.
pub fn enumerate_devices() -> Vec<Device> {
    whisper_rs::vulkan::list_devices()
        .into_iter()
        .map(|info| Device {
            id: info.id,
            name: info.name,
            vram_free: info.vram.free as u64,
            vram_total: info.vram.total as u64,
        })
        .collect()
}

/// The child's answer, or `None` when it crashed, could not start or printed
/// something else.
fn enumerate_in_child() -> Option<Vec<Device>> {
    let executable = child_executable()?;
    let output = Command::new(executable)
        .arg(VULKAN_DEVICES_FLAG)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        tracing::info!("énumération Vulkan : le fils a échoué ({})", output.status);
        return None;
    }
    #[derive(Deserialize)]
    struct Reply {
        devices: Vec<Device>,
    }
    serde_json::from_slice::<Reply>(&output.stdout)
        .map(|reply| reply.devices)
        .map_err(|error| tracing::info!("énumération Vulkan : réponse illisible ({error})"))
        .ok()
}

/// The `neezer-transcribe` binary that can be asked to enumerate.
///
/// It is this process when this process IS the CLI. Under `cargo test` it is the
/// test harness, which would refuse `--vulkan-devices` as an unknown libtest
/// option — and once the app embeds the crate, the caller would be the GPUI
/// binary. In both cases the CLI sits beside the harness (`target/debug/deps/…`
/// → `target/debug/neezer-transcribe`), so the search goes there instead of
/// guessing.
fn child_executable() -> Option<std::path::PathBuf> {
    let current = std::env::current_exe().ok()?;
    let name = current.file_name()?.to_string_lossy().into_owned();
    if name.starts_with("neezer-transcribe") {
        return Some(current);
    }
    let parent = current.parent()?;
    for directory in [parent, parent.parent()?] {
        let candidate = directory.join("neezer-transcribe");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    tracing::info!(
        "énumération Vulkan : pas de binaire neezer-transcribe à côté de {}",
        current.display()
    );
    None
}

/// Whether a Vulkan loader this process could map exists.
///
/// The paths every Linux distribution puts `libvulkan.so.1` in, plus
/// `LD_LIBRARY_PATH`: the loader is what ggml `dlopen`s, and it is the one thing
/// that distinguishes "no driver installed" from "a loader with no ICD".
fn vulkan_loader_present() -> bool {
    let mut candidates: Vec<PathBuf> = [
        "/usr/lib/libvulkan.so.1",
        "/usr/lib64/libvulkan.so.1",
        "/lib/libvulkan.so.1",
        "/lib64/libvulkan.so.1",
        "/usr/local/lib/libvulkan.so.1",
        "/usr/lib/x86_64-linux-gnu/libvulkan.so.1",
        "/usr/lib/aarch64-linux-gnu/libvulkan.so.1",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();
    if let Some(paths) = std::env::var_os("LD_LIBRARY_PATH") {
        for directory in std::env::split_paths(&paths) {
            candidates.push(directory.join("libvulkan.so.1"));
        }
    }
    candidates.iter().any(|path| path.is_file())
}

/// What `Engine::load` needs to know: which model, on which processor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineConfig {
    /// The whisper model file (`ggml-*.bin`).
    pub model: PathBuf,
    /// The Silero VAD model. **Off unless a caller names one** — see
    /// [`vad_beside`](crate::models::vad_beside) for the measured reason: on this
    /// machine whisper.cpp 1.8.3 runs Silero on the CPU, unbatched, at 2.1× real
    /// time (23.6 s for 11 s of speech), which is 98× the GPU time it saves. VAD
    /// is a tool the settings screen may offer, never a default that silently
    /// costs an hour of CPU on a one-hour episode.
    pub vad: Option<PathBuf>,
    /// The person asked for the CPU.
    pub cpu: bool,
    /// whisper threads; `None` uses [`default_threads`].
    pub threads: Option<i32>,
    /// The ggml device index to use; `None` uses the first device.
    pub device: Option<i32>,
}

impl EngineConfig {
    /// The configuration `file` and the supervisor use: the model, the default
    /// thread count, and no VAD (its cost is the reason, see [`EngineConfig`]).
    pub fn new(model: impl Into<PathBuf>) -> Self {
        Self {
            model: model.into(),
            vad: None,
            cpu: false,
            threads: None,
            device: None,
        }
    }
}

/// whisper threads, as `engine.py#threads` computes them: half the CPUs, at most
/// four on a big machine and at most two on a small one.
///
/// A transcription is background work (rule 1): it may use half the box and never
/// all of it, whatever else is playing.
pub fn default_threads() -> i32 {
    let cpus = std::thread::available_parallelism()
        .map(|count| count.get() as i32)
        .unwrap_or(1)
        .max(1);
    let half = (cpus / 2).max(1);
    if cpus >= 16 { half.min(4) } else { half.min(2) }
}

/// One transcription: the lines and the language whisper heard.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Transcript {
    /// The language, when whisper detected one (or was told one).
    pub language: Option<String>,
    /// The raw lines, `{"offsets":{"from":ms,"to":ms},"text":…}` — the shape
    /// whisper.cpp's `-oj` writes and `segments::normalize_segments` takes.
    pub items: Vec<Value>,
}

/// The JSON document the `engine` and `file` commands write, and the only shape
/// T3's supervisor has to understand.
///
/// ```json
/// {"result":{"language":"en"},
///  "transcription":[{"offsets":{"from":0,"to":1230},"text":"…"}],
///  "backend":"vulkan","device":"AMD Radeon 780M (RADV PHOENIX)",
///  "reason":null,"model":"ggml-base.bin"}
/// ```
///
/// `result` and `transcription` are whisper.cpp's own `-oj` keys, so
/// `engine.py#parse_json` and `segments::normalize_segments` apply to this
/// document unchanged; `backend`, `device`, `reason` and `model` are this
/// crate's, and they are what the supervisor turns into its `{"event":"engine"}`
/// line and what the server records with the transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    /// whisper's own result object.
    #[serde(default)]
    pub result: Value,
    /// The lines, in the chunk's own timeline for `engine`, absolute for `file`.
    #[serde(default)]
    pub transcription: Vec<Value>,
    /// [`Backend::as_str`].
    pub backend: String,
    /// The device description, `null` on the CPU.
    #[serde(default)]
    pub device: Option<String>,
    /// [`FallbackCause::code`] when the CPU was used, `null` otherwise.
    #[serde(default)]
    pub reason: Option<String>,
    /// The model's file name.
    #[serde(default)]
    pub model: Option<String>,
}

impl Document {
    /// Build the document from a transcript and the engine that produced it.
    ///
    /// The lines are the `engine` child's chunk-relative ones or `file`'s
    /// absolute ones; `report` says which backend the run ended on and why.
    pub fn from_transcript(
        transcript: Transcript,
        report: &EngineReport,
        model: Option<&str>,
    ) -> Self {
        Self {
            result: json!({ "language": transcript.language }),
            transcription: transcript.items,
            backend: report.backend.as_str().to_owned(),
            device: report.device.clone(),
            reason: report.cause.map(|cause| cause.code().to_owned()),
            model: model.map(str::to_owned),
        }
    }

    /// The compact one-line-per-document JSON the commands write, with a trailing
    /// newline (a file a shell tool can read line by line).
    pub fn to_json(&self) -> String {
        let mut text = serde_json::to_string(self).unwrap_or_else(|_| "{}".to_owned());
        text.push('\n');
        text
    }

    /// Read a document back, refusing anything that is not one.
    pub fn from_json(text: &str) -> Result<Self, Error> {
        serde_json::from_str(text).map_err(|error| {
            tracing::warn!("document du moteur : {error}");
            Error::BadJsonShape
        })
    }

    /// The language whisper reported, as a language code.
    pub fn language(&self) -> Option<String> {
        self.result
            .get("language")
            .and_then(Value::as_str)
            .map(str::to_owned)
    }

    /// The engine report this document was produced with.
    pub fn report(&self) -> EngineReport {
        EngineReport {
            backend: if self.backend == "cpu" {
                Backend::Cpu
            } else {
                Backend::Vulkan
            },
            device: self.device.clone(),
            cause: self.reason.as_deref().and_then(|code| {
                [
                    FallbackCause::NoVulkanLoader,
                    FallbackCause::NoVulkanDevice,
                    FallbackCause::GpuInitFailed,
                    FallbackCause::GpuOutOfMemory,
                    FallbackCause::UserForcedCpu,
                ]
                .into_iter()
                .find(|cause| cause.code() == code)
            }),
        }
    }
}

/// A loaded whisper engine: one model, one state, one backend.
///
/// Loading is the expensive part (the model, then the first GPU call's shader
/// compilation); a caller that transcribes several chunks reuses it, which is why
/// `file` chunks in one process.
pub struct Engine {
    state: WhisperState,
    vad: Option<WhisperVadContext>,
    report: EngineReport,
    threads: i32,
    model: Option<String>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Engine")
            .field("report", &self.report)
            .field("threads", &self.threads)
            .field("model", &self.model)
            .field("vad", &self.vad.is_some())
            .finish()
    }
}

impl Engine {
    /// Load the model and decide the backend, with the cause when it is the CPU.
    ///
    /// The ladder of the module doc, in order: availability → Vulkan context →
    /// one-second silent decode → CPU on any failure, named. A GPU that cannot
    /// hold the model is `gpu_out_of_memory`; any other refusal to start is
    /// `gpu_init_failed`.
    pub fn load(config: &EngineConfig) -> Result<Self, Error> {
        let model_bytes = models::validate(&config.model)?;
        let threads = config.threads.unwrap_or_else(default_threads).max(1);
        let availability = if config.cpu {
            Availability::default()
        } else {
            Availability::detect()
        };
        let mut report = availability.report(config.cpu);

        if report.backend == Backend::Vulkan {
            let device = config
                .device
                .or_else(|| availability.devices.first().map(|entry| entry.id))
                .unwrap_or(0);
            match load_state(config, threads, true, device) {
                Ok(state) => {
                    return Self::finish(state, config, threads, report);
                }
                Err(detail) => {
                    let cause = if device_too_small(&availability, device, model_bytes) {
                        FallbackCause::GpuOutOfMemory
                    } else {
                        FallbackCause::GpuInitFailed
                    };
                    tracing::warn!(
                        "GPU Vulkan indisponible ({detail}), repli sur le processeur ({})",
                        cause.code()
                    );
                    report = EngineReport::cpu(cause);
                }
            }
        }

        let state = load_state(config, threads, false, 0)
            .map_err(|detail| Error::Exit(format!("whisper.cpp : {detail}")))?;
        Self::finish(state, config, threads, report)
    }

    /// A state on the decided backend, with its VAD model attached.
    ///
    /// A VAD model the caller NAMED is not optional: if it cannot be loaded the
    /// call fails with `model_invalid`, because silently transcribing without
    /// what was asked for is the "button that seems to do nothing" rule 2 forbids.
    fn finish(
        state: WhisperState,
        config: &EngineConfig,
        threads: i32,
        report: EngineReport,
    ) -> Result<Self, Error> {
        let vad = match config.vad.as_deref() {
            Some(path) => Some(load_vad(path, threads).ok_or(Error::ModelInvalid)?),
            None => None,
        };
        Ok(Self {
            state,
            vad,
            report,
            threads,
            model: models::name(&config.model),
        })
    }

    /// The whisper parameters every chunk runs with: the server's `-l`, `-t`,
    /// `-sns`, and none of the printing.
    fn params<'a>(&self, language: Option<&'a str>) -> FullParams<'a, 'a> {
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: -1 });
        params.set_n_threads(self.threads);
        params.set_translate(false);
        params.set_no_context(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_no_timestamps(false);
        params.set_single_segment(false);
        // `-sns`, as `engine.py#recognize` passes it.
        params.set_suppress_nst(true);
        match language {
            Some(language) => {
                params.set_language(Some(language));
                params.set_detect_language(false);
            }
            None => {
                params.set_language(None);
                params.set_detect_language(true);
            }
        }
        params
    }

    /// Which processor this engine ended up on.
    pub fn report(&self) -> &EngineReport {
        &self.report
    }

    /// The model's file name, for the sidecar and the server row.
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    /// Whether Silero VAD filtered this engine's audio.
    pub fn vad_enabled(&self) -> bool {
        self.vad.is_some()
    }

    /// Transcribe 16 kHz mono samples, one chunk.
    ///
    /// `language` is `None` for "auto" (whisper detects it), or a code whisper
    /// knows; an unknown code is treated as "auto", as `engine.normalize_language`
    /// does. The returned offsets are relative to *these* samples, in
    /// milliseconds, mapped back to the original audio when VAD filtered it.
    pub fn transcribe(&mut self, pcm: &[f32], language: Option<&str>) -> Result<Transcript, Error> {
        if pcm.is_empty() {
            return Err(Error::EmptyAudio);
        }
        let language =
            language.filter(|code| !code.contains('\0') && whisper_rs::get_lang_id(code).is_some());
        let filtered = match self.vad.as_mut() {
            Some(vad) => filter_speech(vad, pcm),
            None => None,
        };
        let samples: &[f32] = filtered
            .as_ref()
            .map(|(samples, _)| samples.as_slice())
            .unwrap_or(pcm);
        if samples.is_empty() {
            // VAD found no speech at all: an empty chunk, not an error
            // (whisper.cpp's `whisper_full` returns 0 lines in that case too).
            return Ok(Transcript {
                language: language.map(str::to_owned),
                items: Vec::new(),
            });
        }
        let params = self.params(language);
        self.state
            .full(params, samples)
            .map_err(|error| Error::Exit(format!("whisper.cpp : {error}")))?;
        let detected = language.map(str::to_owned).or_else(|| {
            whisper_rs::get_lang_str(self.state.full_lang_id_from_state()).map(str::to_owned)
        });
        let count = self.state.full_n_segments();
        let mut items = Vec::with_capacity(count.max(0) as usize);
        for index in 0..count {
            let Some(segment) = self.state.get_segment(index) else {
                continue;
            };
            let text = segment.to_str_lossy().unwrap_or_default().into_owned();
            if text.trim().is_empty() {
                continue;
            }
            let mut start_ms = segment.start_timestamp().saturating_mul(10);
            let mut end_ms = segment.end_timestamp().saturating_mul(10);
            if let Some((_, mapping)) = filtered.as_ref() {
                // whisper.cpp's own mapping: the VAD removed silence, and the
                // lines must land where the words were, not where the filtered
                // audio put them.
                start_ms = mapping.map(start_ms / 10).saturating_mul(10);
                end_ms = mapping.map(end_ms / 10).saturating_mul(10);
                end_ms = end_ms.max(start_ms.saturating_add(10));
            }
            items.push(json!({
                "offsets": {"from": start_ms, "to": end_ms},
                "text": text,
            }));
        }
        Ok(Transcript {
            language: detected,
            items,
        })
    }
}

/// Create a whisper state on the GPU or the CPU and warm it.
///
/// The context is dropped once the state exists: `WhisperState` holds its own
/// `Arc` on the context, so whisper.cpp's context outlives this call.
fn load_state(
    config: &EngineConfig,
    threads: i32,
    use_gpu: bool,
    device: i32,
) -> Result<WhisperState, String> {
    let mut parameters = WhisperContextParameters::new();
    parameters.use_gpu(use_gpu).gpu_device(device);
    let context = WhisperContext::new_with_params(&config.model, parameters)
        .map_err(|error| format!("chargement du modèle : {error}"))?;
    let mut state = context
        .create_state()
        .map_err(|error| format!("état du moteur : {error}"))?;
    warm_state(&mut state, threads).map_err(|error| error.to_string())?;
    Ok(state)
}

/// One second of silence through the whole graph: whisper.cpp compiles its
/// shaders on the first call (seconds on Vulkan), and this is that call.
fn warm_state(state: &mut WhisperState, threads: i32) -> Result<(), Error> {
    let silence = vec![0.0f32; SAMPLE_RATE as usize * WARMUP_MS / 1000];
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: -1 });
    params.set_n_threads(threads);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_no_context(true);
    params.set_language(Some("en"));
    params.set_detect_language(false);
    state
        .full(params, &silence)
        .map_err(|error| Error::Exit(format!("whisper.cpp (préchauffage) : {error}")))
}

/// Whether the model could not have fitted in the device's free memory: the
/// difference between `gpu_out_of_memory` and `gpu_init_failed`.
fn device_too_small(availability: &Availability, device: i32, model_bytes: u64) -> bool {
    availability
        .devices
        .iter()
        .find(|entry| entry.id == device)
        .map(|entry| entry.vram_total > 0 && model_bytes > entry.vram_free)
        .unwrap_or(false)
}

/// Load the Silero VAD model; a broken one disables VAD instead of failing the
/// transcription (a transcript without VAD is still a transcript).
fn load_vad(path: &Path, threads: i32) -> Option<WhisperVadContext> {
    let mut parameters = whisper_rs::WhisperVadContextParams::new();
    parameters.set_n_threads(threads.min(4));
    parameters.set_use_gpu(false);
    let text = path.to_string_lossy().into_owned();
    match WhisperVadContext::new(&text, parameters) {
        Ok(context) => Some(context),
        Err(error) => {
            tracing::warn!("modèle VAD {} inutilisable : {error}", path.display());
            None
        }
    }
}

/// Run Silero VAD over a chunk and return the speech-only samples plus the map
/// that puts whisper's timestamps back where the words were.
///
/// A faithful port of `whisper_vad` in whisper.cpp (which whisper-rs cannot
/// reach, see the module doc): every speech segment is copied, the segments are
/// separated by 100 ms of silence, and the mapping table records both ends of
/// every segment plus a point every 200 ms inside it.
fn filter_speech(vad: &mut WhisperVadContext, pcm: &[f32]) -> Option<(Vec<f32>, TimeMap)> {
    let segments = vad
        .segments_from_samples(WhisperVadParams::new(), pcm)
        .ok()?;
    let count = segments.num_segments();
    if count <= 0 {
        return Some((Vec::new(), TimeMap::default()));
    }
    let last_sample = (pcm.len() as i64 - 1).max(0);
    let mut filtered: Vec<f32> = Vec::with_capacity(pcm.len());
    let mut points: Vec<(i64, i64)> = Vec::with_capacity(count as usize * 4);
    let mut offset: i64 = 0;
    for index in 0..count {
        let (Some(start_cs), Some(end_cs)) = (
            segments.get_segment_start_timestamp(index),
            segments.get_segment_end_timestamp(index),
        ) else {
            continue;
        };
        let original_start = start_cs as i64;
        let original_end = end_cs as i64;
        let start = cs_to_samples(original_start).min(last_sample);
        let mut end = cs_to_samples(original_end);
        if index < count - 1 {
            end += VAD_OVERLAP_SAMPLES;
        }
        end = end.min(last_sample);
        if end <= start {
            continue;
        }
        let length = end - start;
        let vad_start = samples_to_cs(offset);
        let vad_end = samples_to_cs(offset + length);
        points.push((vad_start, original_start));
        points.push((vad_end, original_end));
        // A point every 200 ms inside a segment longer than a second: the map
        // interpolates between them, so a 30 s segment does not drift.
        if vad_end - vad_start > 100 {
            let duration = vad_end - vad_start;
            let step = 20;
            let intermediate = duration / step - 1;
            for point in 1..=intermediate {
                let vad_time = vad_start + point * step;
                if vad_time >= vad_end {
                    continue;
                }
                let original_time = original_start
                    + ((vad_time - vad_start) * (original_end - original_start)) / duration;
                points.push((vad_time, original_time));
            }
        }
        let from = start.max(0) as usize;
        let to = end.max(0) as usize;
        if to > from && to <= pcm.len() {
            filtered.extend_from_slice(&pcm[from..to]);
        }
        offset += length;
        if index < count - 1 {
            // whisper.cpp's 100 ms of silence between speech segments.
            let silence = VAD_OVERLAP_SAMPLES;
            if let Some(next) = segments.get_segment_start_timestamp(index + 1) {
                points.push((samples_to_cs(offset), original_end));
                points.push((samples_to_cs(offset + silence), next as i64));
            }
            filtered.resize(filtered.len() + silence as usize, 0.0);
            offset += silence;
        }
    }
    points.sort_unstable();
    points.dedup_by_key(|(processed, _)| *processed);
    if filtered.is_empty() {
        return Some((filtered, TimeMap::default()));
    }
    Some((filtered, TimeMap { points }))
}

/// `whisper.cpp`'s `cs_to_samples`: centiseconds → samples, half up.
fn cs_to_samples(cs: i64) -> i64 {
    ((cs as f64 / 100.0) * f64::from(SAMPLE_RATE) + 0.5) as i64
}

/// `whisper.cpp`'s `samples_to_cs`: samples → centiseconds, half up.
fn samples_to_cs(samples: i64) -> i64 {
    ((samples as f64 / f64::from(SAMPLE_RATE)) * 100.0 + 0.5) as i64
}

/// The VAD mapping table: processed time (centiseconds) → original time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TimeMap {
    /// Sorted by processed time, one entry per processed time.
    points: Vec<(i64, i64)>,
}

impl TimeMap {
    /// `map_processed_to_original_time`, integer for integer.
    fn map(&self, processed: i64) -> i64 {
        let Some(first) = self.points.first() else {
            return processed;
        };
        if processed <= first.0 {
            return first.1;
        }
        let last = self.points.last().expect("non-empty");
        if processed >= last.0 {
            return last.1;
        }
        // The first entry at or after `processed`.
        let upper = self.points.partition_point(|(time, _)| *time < processed);
        let (upper_time, upper_original) = self.points[upper];
        if upper_time == processed {
            return upper_original;
        }
        let (lower_time, lower_original) = self.points[upper - 1];
        let processed_diff = upper_time - lower_time;
        if processed_diff == 0 {
            return lower_original;
        }
        let original_diff = upper_original - lower_original;
        let offset = processed - lower_time;
        lower_original + (offset * original_diff) / processed_diff
    }
}

/// Chunk a whole file like the server does and write one document: what
/// `neezer-transcribe file` runs.
///
/// `chunk_ms` is normally [`crate::segments::CHUNK_SECONDS`] × 1 000, and the
/// overlap is the server's [`crate::segments::OVERLAP_MS`]; the lines are made
/// absolute and clamped by `segments::normalize_segments`, so the local file and
/// the server produce the same text from the same audio. `progress` is called
/// with the chunk's end in milliseconds after each chunk, for the CLI to print.
pub fn transcribe_file(
    engine: &mut Engine,
    samples: &[f32],
    language: Option<&str>,
    chunk_ms: i64,
    overlap_ms: i64,
    mut progress: impl FnMut(i64),
) -> Result<Transcript, Error> {
    let total_ms = crate::audio::samples_duration_ms(samples.len()) as i64;
    let chunk_ms = chunk_ms.max(1_000);
    let mut language_out: Option<String> = None;
    let mut items: Vec<Value> = Vec::new();
    let mut start_ms = 0i64;
    while start_ms < total_ms.max(1) {
        let duration_ms = (total_ms - start_ms).clamp(1, chunk_ms);
        let from = (start_ms as usize).saturating_mul(crate::audio::FRAMES_PER_MS);
        let until = from
            .saturating_add((duration_ms as usize).saturating_mul(crate::audio::FRAMES_PER_MS))
            .min(samples.len());
        let chunk = &samples[from.min(samples.len())..until];
        if !chunk.is_empty() {
            let transcript = engine.transcribe(chunk, language)?;
            language_out = language_out.or(transcript.language.clone());
            let segments: Vec<Segment> = crate::segments::normalize_segments(
                &transcript.items,
                start_ms,
                duration_ms,
                overlap_ms,
            );
            items.extend(segments.iter().map(|segment| {
                json!({
                    "offsets": {"from": segment.start_ms, "to": segment.end_ms},
                    "text": segment.text,
                })
            }));
        }
        start_ms += duration_ms;
        progress(start_ms.min(total_ms));
    }
    Ok(Transcript {
        language: language_out,
        items,
    })
}

/// Read a whole file of raw `--pcm` samples, or the whole of an audio file
/// decoded to 16 kHz mono — the two inputs the `engine` and `file` commands take.
pub fn read_input(path: &Path, pcm: bool) -> Result<Vec<f32>, Error> {
    if pcm {
        crate::audio::read_pcm(path)
    } else {
        crate::audio::decode_file(path)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Availability, Backend, Device, Document, EngineReport, FallbackCause, TimeMap,
        cs_to_samples, default_threads, samples_to_cs,
    };

    /// The engine name is a wire value: `worker.py` sends it, `job.py` stores it
    /// and the SPA displays it.
    #[test]
    fn the_engine_speaks_the_servers_names() {
        assert_eq!(super::FORMAT_ENGINE, "whisper.cpp");
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

    /// No device but a loader is `no_vulkan_device`; no device and no loader is
    /// `no_vulkan_loader`; a device is Vulkan; `--cpu` is the person's choice.
    #[test]
    fn availability_names_the_reason_the_gpu_was_not_used() {
        let none = Availability::default();
        assert_eq!(
            none.report(false),
            EngineReport::cpu(FallbackCause::NoVulkanLoader)
        );
        let loader_only = Availability {
            devices: Vec::new(),
            loader: true,
        };
        assert_eq!(
            loader_only.report(false),
            EngineReport::cpu(FallbackCause::NoVulkanDevice)
        );
        let gpu = Availability {
            devices: vec![Device {
                id: 0,
                name: "AMD Radeon 780M (RADV PHOENIX)".into(),
                vram_free: 1 << 30,
                vram_total: 2 << 30,
            }],
            loader: true,
        };
        let report = gpu.report(false);
        assert_eq!(report.backend, Backend::Vulkan);
        assert_eq!(
            report.device.as_deref(),
            Some("AMD Radeon 780M (RADV PHOENIX)")
        );
        assert_eq!(report.cause, None);
        assert_eq!(
            gpu.report(true),
            EngineReport::cpu(FallbackCause::UserForcedCpu)
        );
    }

    /// The control line is one JSON object, with the server's names and one
    /// trailing newline: T3 copies it, T4 switches on it.
    #[test]
    fn the_engine_event_is_one_json_line() {
        let vulkan = EngineReport {
            backend: Backend::Vulkan,
            device: Some("AMD Radeon 780M (RADV PHOENIX)".into()),
            cause: None,
        };
        let line = vulkan.event_json();
        assert!(line.ends_with('\n'), "{line:?}");
        assert_eq!(line.matches('\n').count(), 1, "{line:?}");
        let value: serde_json::Value = serde_json::from_str(&line).expect("JSON");
        assert_eq!(value["event"], "engine");
        assert_eq!(value["backend"], "vulkan");
        assert_eq!(value["device"], "AMD Radeon 780M (RADV PHOENIX)");
        assert!(value["reason"].is_null());

        let cpu = EngineReport::cpu(FallbackCause::NoVulkanDevice);
        let value: serde_json::Value = serde_json::from_str(&cpu.event_json()).expect("JSON");
        assert_eq!(value["backend"], "cpu");
        assert_eq!(value["reason"], "no_vulkan_device");
        assert!(value["device"].is_null());
    }

    /// A document reads back as what wrote it, including the cause and the model
    /// name: T3 sends those to the app and the server.
    #[test]
    fn a_document_round_trips() {
        let report = EngineReport::cpu(FallbackCause::NoVulkanDevice);
        let transcript = super::Transcript {
            language: Some("en".into()),
            items: vec![serde_json::json!({
                "offsets": {"from": 0, "to": 1230}, "text": "one small step"
            })],
        };
        let document =
            Document::from_transcript(transcript.clone(), &report, Some("ggml-base.bin"));
        let text = document.to_json();
        assert!(text.ends_with('\n'));
        let read = Document::from_json(&text).expect("document");
        assert_eq!(read.language().as_deref(), Some("en"));
        assert_eq!(read.transcription, transcript.items);
        assert_eq!(read.model.as_deref(), Some("ggml-base.bin"));
        assert_eq!(read.report(), report);
        assert_eq!(Document::from_json("{"), Err(super::Error::BadJsonShape));
        // `segments::normalize_segments` takes this document's lines unchanged.
        let normalized = crate::segments::normalize_segments(&read.transcription, 0, 60_000, 0);
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].text, "one small step");
    }

    /// Centiseconds are whisper's unit and milliseconds are the crate's; both
    /// conversions round half up, as `whisper.cpp` does.
    #[test]
    fn centiseconds_convert_the_way_whisper_converts_them() {
        assert_eq!(cs_to_samples(0), 0);
        assert_eq!(cs_to_samples(1), 160);
        assert_eq!(cs_to_samples(100), 16_000);
        assert_eq!(cs_to_samples(1501), 240_160);
        assert_eq!(samples_to_cs(0), 0);
        assert_eq!(samples_to_cs(160), 1);
        assert_eq!(samples_to_cs(16_000), 100);
        assert_eq!(samples_to_cs(24_016), 150);
    }

    /// The VAD mapping interpolates inside a speech segment and clamps outside
    /// it: a line at the start of the chunk stays at the start, a line after the
    /// last speech stays at the last speech.
    #[test]
    fn the_vad_map_puts_the_words_back_where_they_were() {
        // Speech 0–2 s of a chunk, then silence, then speech 10–12 s: processed
        // 0–200 cs maps to 0–200 cs, processed 300–500 cs maps to 1000–1200 cs.
        let map = TimeMap {
            points: vec![(0, 0), (200, 200), (300, 1000), (500, 1200)],
        };
        assert_eq!(map.map(0), 0);
        assert_eq!(map.map(200), 200);
        assert_eq!(map.map(250), 600);
        assert_eq!(map.map(400), 1100);
        assert_eq!(map.map(900), 1200);
        assert_eq!(TimeMap::default().map(42), 42);
    }

    /// Half the CPUs, four at most on a big machine, two at most on a small one
    /// (`engine.py#threads`).
    #[test]
    fn whisper_takes_half_the_box_at_most() {
        let threads = default_threads();
        assert!((1..=4).contains(&threads), "{threads}");
    }
}
