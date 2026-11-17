//! Local transcription, wired into the app: the models, the supervisor child,
//! the rule-1 pauses and the sidecar the panel reads (lot T4).
//!
//! The engine itself is NOT here. `neezer-transcribe` is a separate binary, a
//! separate process, at `nice 19` and in its own process group, because whisper
//! on the GPU shares the machine with the person's music (rule 1) and with the
//! UI's own Vulkan surface (the pitfalls of `docs/agents/desktop.md`). This
//! module is the app's whole side of it:
//!
//! - **One service for the process** ([`Service`]): the child, its JSON-lines
//!   control channel, the snapshot a screen reads (rule 2: running, progress,
//!   and each failure's CAUSE), and the monitor thread that pauses the engine
//!   when the audio or the interface needs the machine.
//! - **Models are a click, then a checksum.** Nothing downloads by itself: the
//!   screen asks for one pinned file, [`Api::download_to`] streams it on
//!   `Lane::Bulk` (so it yields to playback exactly like a download does), and
//!   [`crate::wiring::transcribe::ModelCheck`] holds the file to `nz-transcribe`'s
//!   pinned size and SHA-256 before the engine may use it.
//! - **The audio is fetched once, on the Bulk lane.** A local job needs a file
//!   on this machine; an offline episode is used as it is, anything else is
//!   streamed to `$XDG_CACHE_HOME/neezer/transcribe/<episode>.opus` at
//!   Opus 128 (speech survives it and it is the cheapest rung). The PCM the
//!   engine sees is 16 kHz mono, so a higher rung would buy nothing.
//! - **A restart resumes.** The supervisor writes `<episode>.partial.json` after
//!   every chunk (T3); a job asked again after a crash continues at its last
//!   chunk, and the panel says so before the first sample.
//! - **The panel reads the sidecar, never the process.** [`install_source`]
//!   installs D11's `LocalSource` hook with `sidecar::load`, so a transcript
//!   produced here shows and searches with the server's own segment shape.
//!
//! The app needs none of whisper.cpp to do any of this: `nz-transcribe` is
//! depended on with `default-features = false` (its `engine`/`worker` features
//! are the binary's), so no ggml symbol, no Vulkan driver and no model lives in
//! the GPUI process.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use nz_api::{Api, ApiError, Lane};
use nz_cache::Cache;
use nz_model::Quality;
use nz_transcribe::backend::{Backend, FallbackCause};
use nz_transcribe::control::{Command as Control, Event};
use nz_transcribe::models;
use nz_transcribe::sidecar;
use nz_transcribe::worker;
use serde::Deserialize;
use tracing::warn;

use crate::widgets::transcript_panel::{self, LocalTranscript};

// --- the settings (the device's own keys, like every other Réglages value) ---

/// The model the person chose (`ggml-….bin`); absent means the ladder's best.
pub const MODEL_KEY: &str = "transcribe.model";
/// `auto`, `fr`, `en`, … — the language every local job is asked for.
pub const LANGUAGE_KEY: &str = "transcribe.language";
/// « Utiliser le processeur » — a forced CPU is a named cause, never a silent
/// fallback (`user_forced_cpu`).
pub const CPU_KEY: &str = "transcribe.cpu";
/// « Seulement sur secteur » — ON by default, as the plan says.
pub const BATTERY_KEY: &str = "transcribe.battery_only";
/// How much the job yields to the audio and to the interface.
pub const PRIORITY_KEY: &str = "transcribe.priority";

/// The two pinned hosts, and nothing else: a model comes from the project the
/// crate's sizes and hashes were taken from (`whisper.cpp`'s GGML files, and the
/// Silero VAD model whisper.cpp's `--vad` reads).
pub const MODELS_BASE: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/";
/// The VAD model's own repository (`ggml-org/whisper-vad`).
pub const VAD_BASE: &str = "https://huggingface.co/ggml-org/whisper-vad/resolve/main/";

/// How long a probe answer is trusted before the screen asks again.
pub const PROBE_TTL: Duration = Duration::from_secs(300);

/// The pause hysteresis: the engine resumes after this long without a reason,
/// the supervisor's own `RESUME_AFTER_MS` (three seconds, measured).
pub const RESUME_AFTER: Duration = Duration::from_millis(3_000);

/// A paint interval above this is a visible hitch, and a job that keeps the GPU
/// busy through it yields (the priority control's « Très basse »).
///
/// 25 ms is one frame and a half of a 60 Hz screen and six of this machine's
/// 240 Hz panel: below it nothing is perceived, above it the interface stutters
/// while a chunk runs.
pub const FRAME_HITCH_MS: f32 = 25.0;

/// How many failures a snapshot keeps: the screen lists them; a session-long
/// log is what tracing is for.
pub const FAILURES_KEPT: usize = 6;

// --- what the screen reads ---------------------------------------------------

/// How much the job yields (Réglages → Transcription locale → Priorité).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Priority {
    /// Yields to the audio AND to the interface: the default, because the app
    /// must never stutter behind a background transcription (rule 1).
    #[default]
    Lowest,
    /// Yields to the audio; the interface's own hitches do not pause it.
    Low,
    /// Runs as soon as the audio is healthy, at `nice 19`.
    Normal,
}

impl Priority {
    pub const ALL: [Priority; 3] =
        [Priority::Lowest, Priority::Low, Priority::Normal];

    pub fn id(self) -> &'static str {
        match self {
            Priority::Lowest => "lowest",
            Priority::Low => "low",
            Priority::Normal => "normal",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Priority::Lowest => "Très basse",
            Priority::Low => "Basse",
            Priority::Normal => "Normale",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Priority::Lowest => "cède à la lecture et à l'interface",
            Priority::Low => "cède à la lecture",
            Priority::Normal => "cède seulement si le son se coupe",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "low" => Priority::Low,
            "normal" => Priority::Normal,
            _ => Priority::Lowest,
        }
    }
}

/// Why the engine is stopped right now (rule 1), in the person's words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseReason {
    /// The laptop is on its battery and « Seulement sur secteur » is on.
    Battery,
    /// The sound needs the machine: an underrun just happened, or the playhead
    /// stopped advancing while it was playing.
    Audio,
    /// A painted frame took longer than [`FRAME_HITCH_MS`]: the interface needs
    /// the GPU and the CPU back.
    Frame,
}

impl PauseReason {
    pub fn label_fr(self) -> &'static str {
        match self {
            PauseReason::Battery => "en pause : sur batterie",
            PauseReason::Audio => "en pause : la lecture a besoin de la machine",
            PauseReason::Frame => "en pause : l’interface a besoin du processeur graphique",
        }
    }

    /// The line a screen prints under the progress bar.
    pub fn explain_fr(self) -> &'static str {
        match self {
            PauseReason::Battery => {
                "La transcription reprend au secteur. Désactivez « Seulement sur secteur » \
                 pour la laisser tourner sur batterie."
            }
            PauseReason::Audio => {
                "La transcription reprend trois secondes après la fin de la gêne : ce qui \
                 se joue passe toujours avant le texte."
            }
            PauseReason::Frame => {
                "La transcription reprend trois secondes après la dernière saccade. Réglez \
                 la priorité sur « Basse » pour l’ignorer."
            }
        }
    }
}

/// Whether the engine is usable, and which processor it would pick.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Engine {
    /// Nobody asked yet.
    #[default]
    Unknown,
    /// The `probe` child is running.
    Probing,
    /// The GPU, through whisper.cpp's Vulkan backend.
    Vulkan { device: Option<String> },
    /// The CPU, for a named reason: the sentence says why.
    Cpu { cause: FallbackCause },
    /// The binary or the model is not there: a setup problem, with its cause.
    Unavailable { message: String },
}

impl Engine {
    pub fn label_fr(&self) -> String {
        match self {
            Engine::Unknown => "Statut inconnu".to_owned(),
            Engine::Probing => "Vérification du processeur graphique…".to_owned(),
            Engine::Vulkan { .. } => "Vulkan".to_owned(),
            Engine::Cpu { cause } => cause.explain_fr(),
            Engine::Unavailable { message } => message.clone(),
        }
    }

    /// The backend a running job is using, when a control line already said it.
    pub fn backend(&self) -> Option<Backend> {
        match self {
            Engine::Vulkan { .. } => Some(Backend::Vulkan),
            Engine::Cpu { .. } => Some(Backend::Cpu),
            Engine::Unknown | Engine::Probing | Engine::Unavailable { .. } => None,
        }
    }

    /// The cause a CPU fallback names, for the badge that explains the speed.
    pub fn cause(&self) -> Option<FallbackCause> {
        match self {
            Engine::Cpu { cause } => Some(*cause),
            _ => None,
        }
    }
}

/// One row of the model list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelRow {
    pub name: &'static str,
    pub bytes: u64,
    pub installed: bool,
    /// A file that is there but is not a model (truncated, another format).
    pub invalid: bool,
    /// The file a job will use (the person's choice, or the ladder's).
    pub chosen: bool,
    /// The ladder's best for this machine's free memory.
    pub recommended: bool,
}

impl ModelRow {
    pub fn label(&self) -> String {
        format!("{} · {}", self.name, bytes_label(self.bytes))
    }
}

/// `1,4 Go`, `812 Mo`, `0 o` — a model's size is read, never computed in one's
/// head.
pub fn bytes_label(bytes: u64) -> String {
    const UNITS: [(&str, u64); 3] = [("Go", 1 << 30), ("Mo", 1 << 20), ("ko", 1 << 10)];
    if bytes < 1024 {
        return format!("{bytes} o");
    }
    for (unit, size) in UNITS {
        if bytes >= size {
            let value = bytes as f64 / size as f64;
            return format!("{} {unit}", format!("{value:.1}").replace('.', ","));
        }
    }
    format!("{bytes} o")
}

/// A model transfer's state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadPhase {
    Downloading,
    /// The bytes are there and the SHA-256 is being computed (an 874 MB file is
    /// several seconds of one core).
    Verifying,
    /// The file is in place and held to its pin.
    Done,
    /// The transfer or the check failed: the cause, and the file was removed.
    Failed(String),
}

/// One model download, as the card draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Download {
    pub name: &'static str,
    pub done: u64,
    pub total: u64,
    pub phase: DownloadPhase,
}

impl Download {
    pub fn percent(&self) -> u32 {
        if self.total == 0 {
            return 0;
        }
        ((self.done.min(self.total) as f64 / self.total as f64) * 100.0).round() as u32
    }
}

/// The phase of the local job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Streaming the episode's audio to this machine (Bulk), with its bytes.
    Audio { done: u64, total: Option<u64> },
    /// whisper. `duration_ms` 0 means the length is not known yet (the local
    /// supervisor discovers it chunk by chunk).
    Transcribing,
    /// Writing the sidecar at the end.
    Finishing,
}

/// The local job at work, as the panel and the settings card draw it.
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub episode: String,
    pub phase: Phase,
    pub done_ms: i64,
    pub duration_ms: i64,
    /// `vulkan`/`cpu`, once the supervisor said which it got.
    pub backend: Option<Backend>,
    pub device: Option<String>,
    /// Why the engine is stopped right now, if it is.
    pub paused: Option<PauseReason>,
    /// The panel that asked, so the settings screen can name it.
    pub title: Option<String>,
}

impl Job {
    /// The percent a bar can show, or `None` while the length is unknown.
    pub fn percent(&self) -> Option<u32> {
        if self.duration_ms <= 0 {
            return None;
        }
        Some(
            ((self.done_ms.max(0) as f64 / self.duration_ms as f64) * 100.0)
                .clamp(0.0, 100.0)
                .round() as u32,
        )
    }

    /// `42 min transcrites sur 1 h 12`.
    pub fn progress_label(&self) -> String {
        if self.duration_ms <= 0 {
            return format!("{} min transcrites", self.done_ms / 60_000);
        }
        match self.percent() {
            Some(percent) => format!(
                "{} % · {} min sur {} min",
                percent,
                self.done_ms / 60_000,
                self.duration_ms / 60_000
            ),
            None => String::new(),
        }
    }
}

/// One failure with its cause (rule 2: never just a count).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub episode: String,
    pub code: String,
    pub message: String,
}

/// This PC's worker enrolment, as the admin card draws it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Worker {
    /// A device token is on disk (mode 0600).
    pub enrolled: bool,
    pub id: Option<String>,
    pub name: String,
    /// The device's row on the server: enabled, and the lease it holds.
    pub enabled: bool,
    pub last_seen: Option<String>,
    pub lease: Option<String>,
    /// The queue worker child is running.
    pub running: bool,
    pub busy: bool,
}

/// Everything a screen draws, cloned in one mutex read (rule 4: a click answers
/// at once). No path, no SQLite and no child handle is in it.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub version: u64,
    pub settings: Settings,
    pub engine: Engine,
    pub models: Vec<ModelRow>,
    pub free_memory: Option<u64>,
    pub download: Option<Download>,
    pub job: Option<Job>,
    pub failures: Vec<Failure>,
    pub worker: Worker,
    /// The last episode this machine finished, so a panel reloads its text.
    pub last_done: Option<String>,
    /// One sentence about something that just happened, for the card.
    pub note: Option<String>,
}

/// The person's transcription settings, read from `kv` and published in the
/// snapshot so a screen never touches SQLite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub model: Option<String>,
    pub language: String,
    pub cpu: bool,
    pub battery_only: bool,
    pub priority: Priority,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            model: None,
            language: "auto".to_owned(),
            cpu: false,
            battery_only: true,
            priority: Priority::Lowest,
        }
    }
}

impl Settings {
    pub fn read(cache: Option<&Cache>) -> Self {
        let Some(cache) = cache else {
            return Settings::default();
        };
        let kv = cache.kv();
        Settings {
            model: kv.get::<String>(MODEL_KEY).filter(|name| !name.is_empty()),
            language: kv
                .get::<String>(LANGUAGE_KEY)
                .filter(|code| !code.trim().is_empty())
                .unwrap_or_else(|| "auto".to_owned()),
            cpu: kv.get::<bool>(CPU_KEY).unwrap_or(false),
            battery_only: kv.get::<bool>(BATTERY_KEY).unwrap_or(true),
            priority: kv
                .get::<String>(PRIORITY_KEY)
                .map(|value| Priority::parse(&value))
                .unwrap_or_default(),
        }
    }

    /// The model file a job runs, or `None` when nothing usable is installed.
    pub fn model_path(&self) -> Option<PathBuf> {
        model_in(&models::default_dir(), self, models::free_memory_bytes())
    }
}

/// [`Settings::model_path`] against a directory: the person's own choice is
/// AUTHORITATIVE (a small model chosen on purpose must not be replaced by the
/// ladder's biggest), and nothing chosen means the ladder's best for this
/// machine's memory — the server's `choose_model`.
pub fn model_in(dir: &Path, settings: &Settings, free: Option<u64>) -> Option<PathBuf> {
    if let Some(name) = &settings.model {
        let chosen = dir.join(name);
        if models::is_valid(&chosen) {
            return Some(chosen);
        }
    }
    models::choose(&dir.join(models::TURBO_Q8), free)
}

// --- rule 1: the governor ----------------------------------------------------

/// What the machine is doing, sampled by a screen and by the monitor thread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Signals {
    pub playing: bool,
    pub position_s: f64,
    pub underruns: u64,
    /// The worst paint interval of the last second, when a transcribe-aware
    /// screen painted in it.
    pub frame_ms: Option<f32>,
    pub on_battery: bool,
}

impl Default for Signals {
    fn default() -> Self {
        Self {
            playing: false,
            position_s: 0.0,
            underruns: 0,
            frame_ms: None,
            on_battery: false,
        }
    }
}

/// Why the engine would be stopped now, if at all.
pub fn pause_reason(
    signals: &Signals,
    settings: &Settings,
    audio_stalled: bool,
) -> Option<PauseReason> {
    if settings.battery_only && signals.on_battery {
        return Some(PauseReason::Battery);
    }
    if signals.playing && (audio_stalled || signals.underruns > 0) {
        return Some(PauseReason::Audio);
    }
    if settings.priority == Priority::Lowest
        && signals.frame_ms.is_some_and(|ms| ms > FRAME_HITCH_MS)
    {
        return Some(PauseReason::Frame);
    }
    None
}

/// The pause hysteresis: stop at once, resume only after [`RESUME_AFTER`] with
/// no reason at all.
///
/// A bursty trigger (one slow frame, one underrun) must be ONE pause, not a
/// start/stop per chunk: the first GPU call of a run compiles whisper.cpp's
/// shaders, so flapping costs seconds each time (T2).
#[derive(Debug, Default)]
pub struct Governor {
    /// The reason the engine is stopped for, if it is.
    pub paused: Option<PauseReason>,
    /// When every reason first went away.
    quiet_since: Option<Instant>,
    /// The last audio sample: what tells a stall from a quiet passage.
    last: Option<(Instant, f64, u64)>,
}

impl Governor {
    /// Feed one sample; returns the command to send, if the state changed.
    pub fn step(
        &mut self,
        now: Instant,
        signals: Signals,
        settings: &Settings,
    ) -> Option<Control> {
        let stalled = match self.last {
            Some((at, position, underruns)) => {
                let elapsed = now.duration_since(at).as_secs_f64();
                let advanced = signals.position_s - position;
                let fresh_underrun = signals.underruns > underruns;
                self.last = Some((now, signals.position_s, signals.underruns));
                // Half the wall time is the line: a decode that keeps up reads
                // ~1.0, a starving one stalls well under it.
                signals.playing && (fresh_underrun || (elapsed > 0.05 && advanced < elapsed * 0.5))
            }
            None => {
                self.last = Some((now, signals.position_s, signals.underruns));
                false
            }
        };
        let reason = pause_reason(&signals, settings, stalled);
        match reason {
            Some(reason) => {
                self.quiet_since = None;
                if self.paused == Some(reason) {
                    None
                } else {
                    self.paused = Some(reason);
                    Some(Control::Pause)
                }
            }
            None => {
                let Some(paused) = self.paused else {
                    return None;
                };
                let since = *self.quiet_since.get_or_insert(now);
                if now.duration_since(since) < RESUME_AFTER {
                    return None;
                }
                self.quiet_since = None;
                self.paused = None;
                let _ = paused;
                Some(Control::Resume)
            }
        }
    }

    /// Forget the pause (a new job starts unpaused).
    pub fn reset(&mut self) {
        self.paused = None;
        self.quiet_since = None;
        self.last = None;
    }
}

// --- the child's command line ------------------------------------------------

/// The supervisor's arguments for these settings.
///
/// `local` when no device token is installed (the person transcribes on this
/// PC), `worker` when there is one (the server's queue runs here too, and the
/// same process still accepts `local` commands in its idle loop — one engine,
/// never two, rule 1).
pub fn supervisor_args(
    settings: &Settings,
    model: &Path,
    worker_mode: Option<&str>,
    language: &str,
) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    match worker_mode {
        Some(url) => {
            args.push("worker".to_owned());
            args.push("--url".to_owned());
            args.push(url.to_owned());
        }
        None => args.push("local".to_owned()),
    }
    args.push("--model".to_owned());
    args.push(model.display().to_string());
    let language = if language.trim().is_empty() {
        "auto"
    } else {
        language.trim()
    };
    if language != "auto" {
        args.push("--lang".to_owned());
        args.push(language.to_owned());
    }
    if settings.cpu {
        args.push("--cpu".to_owned());
    }
    args.push("--work-dir".to_owned());
    args.push(work_dir().display().to_string());
    if worker_mode.is_none() {
        args.push("--out-dir".to_owned());
        args.push(worker::transcripts_dir().display().to_string());
    }
    args
}

/// Where the supervisor writes its scratch files (downloaded audio, PCM, the
/// engine's JSON). Under the cache, never `/tmp`: an hour of PCM is 230 MB.
pub fn work_dir() -> PathBuf {
    worker::cache_dir().join("transcribe")
}

/// `https://huggingface.co/…/<name>` for a pinned file, or `None` for a name
/// this build does not pin (a file a person put there by hand is not fetched).
pub fn download_url(name: &str) -> Option<String> {
    let entry = models::file(name)?;
    let base = if entry.name == models::VAD || entry.name == models::VAD_FOR_TESTS {
        VAD_BASE
    } else {
        MODELS_BASE
    };
    Some(format!("{base}{}", entry.name))
}

/// What checking a downloaded model says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelCheck {
    /// The file has exactly the pinned size and digest.
    Pinned,
    /// A different size: the transfer was truncated or answered something else.
    WrongSize { got: u64, want: u64 },
    /// The right size, another digest: the file is not the one the crate pins.
    WrongDigest,
    /// There, but not a GGML model the engine may load.
    NotAModel(String),
}

/// Hold a downloaded file to the crate's pin. A file that fails is the caller's
/// to delete: keeping it would make the next click use a corrupt model.
pub fn check_model(name: &str, path: &Path) -> ModelCheck {
    let Some(entry) = models::file(name) else {
        return ModelCheck::NotAModel(format!("{name} n’est pas un modèle épinglé"));
    };
    let Ok(metadata) = std::fs::metadata(path) else {
        return ModelCheck::NotAModel("le fichier téléchargé est absent".to_owned());
    };
    if metadata.len() != entry.bytes {
        return ModelCheck::WrongSize {
            got: metadata.len(),
            want: entry.bytes,
        };
    }
    if !models::verify_sha256(path, entry.sha256) {
        return ModelCheck::WrongDigest;
    }
    // The VAD model is a GGML file but not a whisper model: `validate`'s 1 MiB
    // floor and magic are for the ladder.
    if entry.name != models::VAD && entry.name != models::VAD_FOR_TESTS {
        if let Err(error) = models::validate(path) {
            return ModelCheck::NotAModel(error.to_string());
        }
    }
    ModelCheck::Pinned
}

impl ModelCheck {
    /// The French sentence a failed check shows, or `None` when it passed.
    pub fn cause_fr(&self, name: &str) -> Option<String> {
        match self {
            ModelCheck::Pinned => None,
            ModelCheck::WrongSize { got, want } => Some(format!(
                "Téléchargement incomplet : {} reçus sur {} attendus pour {name}. Le fichier \
                 a été supprimé ; relancez le téléchargement.",
                bytes_label(*got),
                bytes_label(*want)
            )),
            ModelCheck::WrongDigest => Some(format!(
                "Le SHA-256 de {name} ne correspond pas à celui épinglé dans l’application. \
                 Le fichier a été supprimé ; réessayez depuis un autre réseau."
            )),
            ModelCheck::NotAModel(message) => Some(format!(
                "{name} n’est pas un modèle whisper.cpp valide : {message}."
            )),
        }
    }
}

// --- the state a screen reads, and the service behind it ---------------------

/// One service for the process: the child, its reader, the monitor and the
/// snapshot. Cloned by `Arc`; nothing in it is GPUI.
pub struct Service {
    shared: Arc<Shared>,
    /// The monitor thread (rule 1), joined by [`Service::shutdown`].
    monitor: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// The child's stdout reader, joined by [`Service::shutdown`].
    reader: Mutex<Option<std::thread::JoinHandle<()>>>,
    stopped: AtomicBool,
}

struct Shared {
    snapshot: Mutex<Snapshot>,
    /// The engine to sample for rule 1, installed by the first screen that has
    /// one.
    audio: Mutex<Option<nz_audio::Engine>>,
    api: Mutex<Option<Arc<Api>>>,
    cache: Mutex<Option<Cache>>,
    runtime: Mutex<Option<tokio::runtime::Handle>>,
    /// The last paint interval a screen published, with its instant.
    frame: Mutex<Option<(f32, Instant)>>,
    /// Whether the machine is on its battery, refreshed by the monitor.
    battery: AtomicBool,
    /// The download in flight, so « Annuler » can reach it.
    cancel: Mutex<Arc<AtomicBool>>,
    /// The child and its stdin, the one place both live.
    child: Mutex<Option<Child>>,
    stdin: Mutex<Option<ChildStdin>>,
    /// The supervisor binary a test names; `None` is the sibling of the app.
    program: Mutex<Option<PathBuf>>,
    /// How to reach the server when the worker mode starts.
    server: Mutex<Option<String>>,
    /// The probe's last answer, for [`PROBE_TTL`].
    probed_at: Mutex<Option<Instant>>,
    /// When the worker rows were last read, so a paint does not ask the server.
    worker_checked: Mutex<Option<Instant>>,
    /// The stderr file of the child, read back when it dies silently.
    stderr: Mutex<Option<PathBuf>>,
    /// A deliberate stop: the reader must not report it as a crash.
    stopping: AtomicBool,
    /// The monitor's own stop flag, so a test's service joins its thread too.
    stop: AtomicBool,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Snapshot> {
        self.snapshot.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn publish(&self, change: impl FnOnce(&mut Snapshot)) {
        let mut snapshot = self.lock();
        change(&mut snapshot);
        snapshot.version = snapshot.version.wrapping_add(1);
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}

static SERVICE: OnceLock<Arc<Service>> = OnceLock::new();
static SOURCE: OnceLock<()> = OnceLock::new();

/// The process's service, created on first use.
pub fn service() -> Arc<Service> {
    SERVICE
        .get_or_init(|| {
            let service = Arc::new(Service::new());
            service.start_monitor();
            service
        })
        .clone()
}

/// The service only when something already created it: the close path must not
/// start a monitor thread on its way out.
pub fn started() -> Option<Arc<Service>> {
    SERVICE.get().cloned()
}

/// Attach the session to the service: the API, the store, the runtime and the
/// audio engine, all refreshed by every screen that calls this. Installs the
/// panel's sidecar reader once.
pub fn ensure(state: &crate::state::AppState) -> Arc<Service> {
    let service = service();
    service.attach(state);
    install_source();
    service
}

/// Install D11's local-source hook: the panel reads this PC's sidecars.
pub fn install_source() {
    SOURCE.get_or_init(|| {
        transcript_panel::set_local_source(Some(Arc::new(|episode| {
            read_sidecar(&worker::transcripts_dir(), episode)
        })));
    });
}

/// `true` when an episode id is a UUID-ish token: a sidecar path is built from
/// it, so a slash, a dot or a percent-escape never reaches the filesystem.
pub fn is_episode_id(value: &str) -> bool {
    let value = value.trim();
    if value.len() != 36 && value.len() != 32 {
        // The bench and uploads use UUIDs; a numeric id never names an episode,
        // and the panel only ever holds one the server sent.
        return value.len() <= 64
            && !value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() || byte == b'-');
    }
    value
        .bytes()
        .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
}

/// One path segment, percent-encoded: an id used to be a UUID, but a route this
/// app builds must never let a slash or a dot become another path.
pub fn encode_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => {
                use std::fmt::Write as _;
                let _ = write!(out, "%{other:02X}");
            }
        }
    }
    out
}

/// The local transcript of an episode, read off this machine.
pub fn read_sidecar(dir: &Path, episode: &str) -> Option<LocalTranscript> {
    if !is_episode_id(episode) {
        return None;
    }
    let pair = sidecar::pair_for(&dir.join(episode));
    let document = sidecar::load(&pair.json).ok()?;
    Some(LocalTranscript {
        engine: document.engine.clone(),
        model: document.model.clone(),
        language: document.language.clone(),
        segments: document
            .segments
            .iter()
            .map(|segment| nz_model::TranscriptSegment {
                start: segment.start_ms as f64 / 1_000.0,
                end: segment.end_ms as f64 / 1_000.0,
                text: segment.text.clone(),
            })
            .collect(),
        created: document.created.clone(),
    })
}

/// The progress a restarted job would resume at: `<episode>.partial.json`, as
/// T3 writes it after every chunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Partial {
    pub episode: String,
    pub done_ms: i64,
    pub segments: usize,
}

pub fn partial_of(dir: &Path, episode: &str) -> Option<Partial> {
    if !is_episode_id(episode) {
        return None;
    }
    let path = dir.join(format!("{episode}.partial.json"));
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() > 8 * 1024 * 1024 {
        return None;
    }
    #[derive(Deserialize)]
    struct Saved {
        #[serde(default)]
        episode_id: String,
        #[serde(default)]
        done_ms: i64,
        #[serde(default)]
        segments: Vec<serde_json::Value>,
    }
    let saved: Saved = serde_json::from_slice(&bytes).ok()?;
    Some(Partial {
        episode: saved.episode_id,
        done_ms: saved.done_ms.max(0),
        segments: saved.segments.len(),
    })
}

impl Service {
    /// The binary this app starts: the `neezer-transcribe` beside it (the Arch
    /// package installs both, and `cargo build --workspace` puts them in one
    /// `target/…/`), or a bare name for `PATH`. A test names its own.
    pub fn program(&self) -> PathBuf {
        if let Some(program) = self
            .shared
            .program
            .lock()
            .ok()
            .and_then(|program| program.clone())
        {
            return program;
        }
        // `cargo build --workspace` puts both binaries in `target/<profile>/`;
        // a test binary lives one level deeper (`target/debug/deps/`), so both
        // places are looked at before giving up on `PATH`.
        if let Ok(current) = std::env::current_exe()
            && let Some(dir) = current.parent()
        {
            for candidate in [dir.join("neezer-transcribe"), dir.join("../neezer-transcribe")] {
                if candidate.is_file() {
                    return candidate;
                }
            }
        }
        PathBuf::from("neezer-transcribe")
    }

    #[cfg(test)]
    pub fn set_program(&self, program: PathBuf) {
        *self.shared.program.lock().unwrap_or_else(|e| e.into_inner()) = Some(program);
    }
    fn new() -> Service {
        Service {
            shared: Arc::new(Shared {
                snapshot: Mutex::new(Snapshot {
                    settings: Settings::default(),
                    ..Snapshot::default()
                }),
                audio: Mutex::new(None),
                api: Mutex::new(None),
                cache: Mutex::new(None),
                runtime: Mutex::new(None),
                frame: Mutex::new(None),
                battery: AtomicBool::new(false),
                cancel: Mutex::new(Arc::new(AtomicBool::new(false))),
                child: Mutex::new(None),
                stdin: Mutex::new(None),
                program: Mutex::new(None),
                server: Mutex::new(None),
                probed_at: Mutex::new(None),
                worker_checked: Mutex::new(None),
                stderr: Mutex::new(None),
                stopping: AtomicBool::new(false),
                stop: AtomicBool::new(false),
            }),
            monitor: Mutex::new(None),
            reader: Mutex::new(None),
            stopped: AtomicBool::new(false),
        }
    }

    /// Take the session's handles, and read the settings again (a screen may
    /// have changed them on another page).
    pub fn attach(&self, state: &crate::state::AppState) {
        *self.shared.api.lock().unwrap_or_else(|e| e.into_inner()) = state.api.clone();
        *self
            .shared
            .runtime
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(state.runtime.clone());
        *self.shared.audio.lock().unwrap_or_else(|e| e.into_inner()) =
            state.audio.as_ref().map(|audio| audio.engine.clone());
        *self
            .shared
            .server
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(state.server.clone());
        // The settings and the model list are read ONCE: after that the service
        // is what changes them (`set_setting`), so a paint is a clone and a
        // comparison, never six `stat`s and two SQLite reads (rule 4).
        let first = self
            .shared
            .cache
            .lock()
            .map(|cache| cache.is_none())
            .unwrap_or(false);
        if !first {
            return;
        }
        *self.shared.cache.lock().unwrap_or_else(|e| e.into_inner()) = state.cache.clone();
        let settings = Settings::read(state.cache.as_ref());
        let worker = worker_file_present();
        self.shared.publish(|snapshot| {
            snapshot.settings = settings;
            snapshot.worker.enrolled = worker;
            snapshot.worker.name = hostname();
        });
        self.publish_models();
    }

    pub fn snapshot(&self) -> Snapshot {
        self.shared.lock().clone()
    }

    /// Read the model ladder off the disk: six `stat`s, no parse.
    pub fn publish_models(&self) {
        let dir = models::default_dir();
        let free = models::free_memory_bytes();
        let settings = self.shared.lock().settings.clone();
        let recommended = models::ladder(free)
            .first()
            .map(|name| (*name).to_owned());
        let chosen = settings.model_path().and_then(|path| models::name(&path));
        let rows: Vec<ModelRow> = models::FILES
            .iter()
            .map(|entry| {
                let path = dir.join(entry.name);
                let installed = path.is_file();
                let valid = installed
                    && if entry.name == models::VAD || entry.name == models::VAD_FOR_TESTS {
                        true
                    } else {
                        models::is_valid(&path)
                    };
                ModelRow {
                    name: entry.name,
                    bytes: entry.bytes,
                    installed,
                    invalid: installed && !valid,
                    chosen: chosen.as_deref() == Some(entry.name),
                    recommended: recommended.as_deref() == Some(entry.name),
                }
            })
            .collect();
        self.shared.publish(|snapshot| {
            snapshot.models = rows;
            snapshot.free_memory = free;
        });
    }

    /// Record one painted frame of a transcribe-aware screen: the governor's
    /// interface signal.
    pub fn note_frame(&self, interval: Duration) {
        let ms = interval.as_secs_f32() * 1_000.0;
        if let Ok(mut frame) = self.shared.frame.lock() {
            *frame = Some((ms, Instant::now()));
        }
    }

    /// The worst paint interval seen in the last second, if any.
    fn frame_ms(&self) -> Option<f32> {
        let frame = *self.shared.frame.lock().ok()?;
        frame.and_then(|(ms, at)| (at.elapsed() <= Duration::from_secs(1)).then_some(ms))
    }

    /// The audio sample rule 1 needs, read on the monitor thread.
    fn signals(&self) -> Signals {
        let engine = self
            .shared
            .audio
            .lock()
            .ok()
            .and_then(|audio| audio.clone());
        let (playing, position_s, underruns) = match engine {
            Some(engine) => (engine.is_playing(), engine.position(), engine.underruns()),
            None => (false, 0.0, 0),
        };
        Signals {
            playing,
            position_s,
            underruns,
            frame_ms: self.frame_ms(),
            on_battery: self.shared.battery.load(Ordering::Relaxed),
        }
    }

    /// Start the monitor: 250 ms of sampling, one pause/resume per state change.
    fn start_monitor(self: &Arc<Self>) {
        let shared = self.shared.clone();
        let mut governor = Governor::default();
        let mut battery_checked = Instant::now() - Duration::from_secs(60);
        let handle = std::thread::Builder::new()
            .name("nz-transcribe-governor".to_owned())
            .spawn(move || {
                while !shared.stop.load(Ordering::Relaxed) {
                    let now = Instant::now();
                    if battery_checked.elapsed() >= Duration::from_secs(30) {
                        battery_checked = now;
                        let on_battery = on_battery();
                        shared.battery.store(on_battery, Ordering::Relaxed);
                    }
                    let settings = shared.lock().settings.clone();
                    let engine = shared.audio.lock().ok().and_then(|audio| audio.clone());
                    let (playing, position_s, underruns) = match engine {
                        Some(engine) => {
                            (engine.is_playing(), engine.position(), engine.underruns())
                        }
                        None => (false, 0.0, 0),
                    };
                    let frame_ms = shared.frame.lock().ok().and_then(|frame| {
                        frame.and_then(|(ms, at)| {
                            (at.elapsed() <= Duration::from_secs(1)).then_some(ms)
                        })
                    });
                    let signals = Signals {
                        playing,
                        position_s,
                        underruns,
                        frame_ms,
                        on_battery: shared.battery.load(Ordering::Relaxed),
                    };
                    if let Some(command) = governor.step(now, signals, &settings) {
                        let paused = governor.paused;
                        shared.publish(|snapshot| {
                            if let Some(job) = snapshot.job.as_mut() {
                                job.paused = paused;
                            }
                        });
                        send_via(&shared, command);
                    }
                    std::thread::sleep(Duration::from_millis(250));
                }
            })
            .ok();
        *self.monitor.lock().unwrap_or_else(|e| e.into_inner()) = handle;
    }

    /// Write one command to the supervisor's stdin, flushed (a pause that sits
    /// in a buffer is a pause that does not happen).
    pub fn send(&self, command: Control) {
        send_via(&self.shared, command);
    }

    /// Probe the machine's processor through the binary's child (never
    /// in-process: enumeration aborts without a driver).
    pub fn probe(&self, force: bool) {
        if !force
            && self
                .shared
                .probed_at
                .lock()
                .ok()
                .and_then(|at| *at)
                .is_some_and(|at| at.elapsed() < PROBE_TTL)
        {
            return;
        }
        let settings = self.shared.lock().settings.clone();
        self.shared.publish(|snapshot| snapshot.engine = Engine::Probing);
        let program = self.program();
        let shared = self.shared.clone();
        if let Ok(mut at) = self.shared.probed_at.lock() {
            *at = Some(Instant::now());
        }
        // The probe is one child process (a few dozen milliseconds, seconds on
        // a cold driver): never the UI thread, and not a `Handle::block_on`
        // from a thread we would owe a join either — a plain thread ending at
        // the answer is enough.
        std::thread::Builder::new()
            .name("nz-transcribe-probe".to_owned())
            .spawn(move || {
                let report = run_probe(&program, settings.cpu);
                let mut snapshot = shared.lock();
                snapshot.engine = report;
                snapshot.version = snapshot.version.wrapping_add(1);
            })
            .ok();
    }

    /// Save one setting (`kv`), then republish. A model change reaches the next
    /// job; the running one keeps the model it started with.
    pub fn set_setting(&self, key: &'static str, value: &serde_json::Value) {
        let cache = self
            .shared
            .cache
            .lock()
            .ok()
            .and_then(|cache| cache.clone());
        if let Some(cache) = &cache
            && let Err(error) = cache.kv().set(key, value)
        {
            warn!(%error, key, "réglage de transcription non enregistré");
        }
        let settings = Settings::read(cache.as_ref());
        self.shared.publish(|snapshot| snapshot.settings = settings);
        self.publish_models();
    }

    /// Start (or resume) one episode on this machine.
    ///
    /// `audio` is a local file when the caller already has one (an offline
    /// download); anything else is streamed to the cache at Opus 128 on
    /// `Lane::Bulk`. Everything after the first line is async work on `nz-net`.
    pub fn start_local(&self, episode: &str, title: Option<String>, duration_ms: u64) {
        self.start_with(episode, title, duration_ms, None);
    }

    fn start_with(
        &self,
        episode: &str,
        title: Option<String>,
        duration_ms: u64,
        audio: Option<PathBuf>,
    ) {
        if !is_episode_id(episode) {
            self.note("Cet identifiant d’épisode n’est pas valide.");
            return;
        }
        let snapshot = self.snapshot();
        if let Some(job) = &snapshot.job {
            if job.episode == episode {
                self.shared.publish(|snapshot| {
                    snapshot.note = Some("Cet épisode est déjà en cours de transcription.".to_owned());
                });
            } else {
                self.shared.publish(|snapshot| {
                    snapshot.note = Some(
                        "Une transcription est déjà en cours sur cet ordinateur : elle doit \
                         finir ou être arrêtée avant d’en lancer une autre."
                            .to_owned(),
                    );
                });
            }
            return;
        }
        let Some(model) = snapshot.settings.model_path() else {
            let dir = models::default_dir();
            self.shared.publish(|snapshot| {
                snapshot.failures.push(Failure {
                    episode: episode.to_owned(),
                    code: "model_missing".to_owned(),
                    message: format!(
                        "Aucun modèle installé dans {}. Ouvrez Réglages → Transcription locale et \
                         téléchargez-en un : le plus petit suffit pour essayer.",
                        dir.display()
                    ),
                });
                Snapshot::trim_failures(&mut snapshot.failures);
            });
            return;
        };
        self.shared.publish(|snapshot| {
            snapshot.note = None;
            snapshot.job = Some(Job {
                episode: episode.to_owned(),
                phase: audio
                    .as_ref()
                    .map(|_| Phase::Transcribing)
                    .unwrap_or(Phase::Audio { done: 0, total: None }),
                done_ms: 0,
                // The show page's own length: the card has a percent while the
                // first chunk runs instead of a bar that waits for it.
                duration_ms: duration_ms.min(i64::MAX as u64) as i64,
                backend: snapshot.engine.backend(),
                device: match &snapshot.engine {
                    Engine::Vulkan { device } => device.clone(),
                    _ => None,
                },
                paused: None,
                title,
            });
        });

        let Some(runtime) = self.shared.runtime.lock().ok().and_then(|rt| rt.clone()) else {
            warn!("aucun runtime : la transcription locale ne peut pas démarrer");
            return;
        };
        let shared = self.shared.clone();
        let service = SERVICE.get().cloned();
        let episode = episode.to_owned();
        runtime.spawn(async move {
            let file = match audio {
                Some(path) => path,
                None => match stream_audio(&shared, &episode).await {
                    Ok(path) => path,
                    Err(cause) => {
                        shared.publish(|snapshot| {
                            snapshot.job = None;
                            snapshot.failures.push(Failure {
                                episode: episode.clone(),
                                code: "audio_http".to_owned(),
                                message: cause,
                            });
                            Snapshot::trim_failures(&mut snapshot.failures);
                        });
                        return;
                    }
                },
            };
            let Some(service) = service else {
                return;
            };
            service.start_supervisor(&episode, &file, &shared, &model);
        });
    }

    /// Stop the job: the supervisor gives its lease back, keeps the `.partial`
    /// file, and the next start resumes from the last chunk.
    pub fn stop_local(&self) {
        self.shared.stopping.store(true, Ordering::SeqCst);
        self.send(Control::Stop);
        self.shared.publish(|snapshot| {
            snapshot.note = Some(
                "Transcription arrêtée : ce qui était transcrit est gardé, et « Reprendre » \
                 repart du dernier passage."
                    .to_owned(),
            );
        });
    }

    /// Ensure the supervisor child is running, then ask it for this episode.
    fn start_supervisor(&self, episode: &str, audio: &Path, shared: &Shared, model: &Path) {
        let settings = shared.lock().settings.clone();
        let program = self.program();
        let stderr_path = work_dir().join("supervisor.err");
        let worker_mode = if worker_file_present() {
            shared.server.lock().ok().and_then(|server| server.clone())
        } else {
            None
        };
        if !self.ensure_child(&program, model, &settings, worker_mode.as_deref(), &stderr_path) {
            return;
        }
        self.send(Control::Local {
            episode_id: episode.to_owned(),
            audio: audio.display().to_string(),
            language: Some(settings.language.clone()),
        });
        shared.publish(|snapshot| {
            if let Some(job) = snapshot.job.as_mut() {
                job.phase = Phase::Transcribing;
                job.paused = None;
            }
        });
    }

    /// Start the child if there is none. `false` when it cannot start, with a
    /// cause in the snapshot (rule 2).
    fn ensure_child(
        &self,
        program: &Path,
        model: &Path,
        settings: &Settings,
        worker_mode: Option<&str>,
        stderr: &Path,
    ) -> bool {
        let mut child = self.shared.child.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(running) = child.as_mut() {
            match running.try_wait() {
                Ok(None) => return true,
                // A child that has gone leaves a pipe that looks open until the
                // first write: a job sent to it then vanished (measured: the card
                // stayed at 0 % for minutes while an idle supervisor sat there).
                _ => {
                    child.take();
                    *self.shared.stdin.lock().unwrap_or_else(|e| e.into_inner()) = None;
                }
            }
        }
        self.shared.stopping.store(false, Ordering::SeqCst);
        if let Some(parent) = stderr.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            self.fail_child("no_space", error.to_string());
            return false;
        }
        let log = match std::fs::File::create(stderr) {
            Ok(file) => file,
            Err(error) => {
                self.fail_child("no_space", error.to_string());
                return false;
            }
        };
        let args = supervisor_args(settings, model, worker_mode, &settings.language);
        let mut command = Command::new(program);
        command
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(log));
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
            // SAFETY: `nice` is a syscall wrapper that changes only the child's
            // scheduling priority before `exec`; it allocates nothing and
            // touches no shared memory, so it is safe after `fork`.
            unsafe {
                command.pre_exec(|| {
                    lower_priority();
                    Ok(())
                });
            }
        }
        match command.spawn() {
            Ok(mut spawned) => {
                *self.shared.stdin.lock().unwrap_or_else(|e| e.into_inner()) = spawned.stdin.take();
                *self.shared.stderr.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(stderr.to_path_buf());
                let stdout = spawned.stdout.take();
                *child = Some(spawned);
                if let Some(stdout) = stdout {
                    self.start_reader(stdout);
                }
                true
            }
            Err(error) => {
                self.fail_child("engine_missing", format!("{program:?} : {error}"));
                false
            }
        }
    }

    /// Read the supervisor's events on their own thread (no runtime borrow).
    fn start_reader(&self, stdout: std::process::ChildStdout) {
        let shared = self.shared.clone();
        let handle = std::thread::Builder::new()
            .name("nz-transcribe-events".to_owned())
            .spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines() {
                    let Ok(line) = line else { break };
                    if line.trim().is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<Event>(&line) {
                        Ok(event) => apply_event(&shared, event),
                        // A newer supervisor (rule 7): log it, keep reading.
                        Err(error) => warn!("événement de transcription inconnu ({error}): {line}"),
                    }
                }
                // The pipe closed: the child is gone. A job still marked
                // running was interrupted, and the partial file is where it
                // stopped — say so (rule 2), never an endless bar.
                let stopped = shared.stopping.swap(false, Ordering::SeqCst);
                // The pipe closed: the handles are dead. Keeping them made the
                // next `ensure_child` answer "already running" and write into a
                // closed pipe.
                if let Ok(mut child) = shared.child.lock() {
                    child.take();
                }
                *shared.stdin.lock().unwrap_or_else(|e| e.into_inner()) = None;
                shared.publish(|snapshot| {
                    if stopped {
                        snapshot.job = None;
                        snapshot.worker.running = false;
                        if snapshot.note.is_none() {
                            snapshot.note = Some(
                                "Transcription arrêtée : le texte déjà produit est gardé, et \
                                 « Reprendre » repart du dernier passage."
                                    .to_owned(),
                            );
                        }
                        return;
                    }
                    if let Some(job) = snapshot.job.take() {
                        let tail = read_stderr(&shared);
                        snapshot.failures.push(Failure {
                            episode: job.episode,
                            code: "engine_crashed".to_owned(),
                            message: if tail.is_empty() {
                                "Le superviseur de transcription s’est arrêté avant la fin. \
                                 « Reprendre » repartira du dernier passage transcrit."
                                    .to_owned()
                            } else {
                                format!(
                                    "Le superviseur de transcription s’est arrêté : {tail}"
                                )
                            },
                        });
                        Snapshot::trim_failures(&mut snapshot.failures);
                    }
                });
            })
            .ok();
        *self.reader.lock().unwrap_or_else(|e| e.into_inner()) = handle;
    }

    fn fail_child(&self, code: &str, message: String) {
        self.shared.publish(|snapshot| {
            snapshot.job = None;
            snapshot.failures.push(Failure {
                episode: String::new(),
                code: code.to_owned(),
                message: message.clone(),
            });
            Snapshot::trim_failures(&mut snapshot.failures);
            snapshot.note = Some(message);
        });
    }

    /// Read the model list, then start downloading one pinned file.
    pub fn download_model(&self, name: &str) {
        let Some(url) = download_url(name) else {
            self.note("Ce fichier n’est pas un modèle épinglé par l’application.");
            return;
        };
        let Some(entry) = models::file(name) else {
            return;
        };
        if self
            .snapshot()
            .download
            .as_ref()
            .is_some_and(|download| matches!(download.phase, DownloadPhase::Downloading | DownloadPhase::Verifying))
        {
            return;
        }
        let Some(api) = self.shared.api.lock().ok().and_then(|api| api.clone()) else {
            self.note("Connectez-vous pour télécharger un modèle.");
            return;
        };
        let dir = models::default_dir();
        let destination = dir.join(entry.name);
        let cancel = Arc::new(AtomicBool::new(false));
        *self.shared.cancel.lock().unwrap_or_else(|e| e.into_inner()) = cancel.clone();
        self.shared.publish(|snapshot| {
            snapshot.download = Some(Download {
                name: entry.name,
                done: 0,
                total: entry.bytes,
                phase: DownloadPhase::Downloading,
            });
            snapshot.note = None;
        });
        let Some(runtime) = self.shared.runtime.lock().ok().and_then(|rt| rt.clone()) else {
            return;
        };
        let shared = self.shared.clone();
        let name = name.to_owned();
        runtime.spawn(async move {
            let progress_shared = shared.clone();
            let progress = move |done: u64, total: Option<u64>| {
                progress_shared.publish(|snapshot| {
                    if let Some(download) = snapshot.download.as_mut()
                        && download.name == name.as_str()
                    {
                        download.done = done;
                        download.total = total.unwrap_or(entry.bytes).max(download.done);
                    }
                });
            };
            let answer = api
                .download_to(Lane::Bulk, &url, &destination, progress, &cancel)
                .await;
            if let Err(error) = answer {
                let cause = error.cause_fr().to_owned();
                if !cancel.load(Ordering::Relaxed) {
                    let _ = std::fs::remove_file(&destination);
                }
                shared.publish(|snapshot| {
                    snapshot.download = Some(Download {
                        name: entry.name,
                        done: 0,
                        total: entry.bytes,
                        phase: DownloadPhase::Failed(cause),
                    });
                });
                return;
            }
            shared.publish(|snapshot| {
                snapshot.download = Some(Download {
                    name: entry.name,
                    done: entry.bytes,
                    total: entry.bytes,
                    phase: DownloadPhase::Verifying,
                });
            });
            let check_path = destination.clone();
            let check_name = entry.name;
            let check = tokio::task::spawn_blocking(move || check_model(check_name, &check_path))
                .await
                .unwrap_or(ModelCheck::NotAModel("vérification interrompue".to_owned()));
            let cause = check.cause_fr(entry.name);
            if cause.is_some() {
                let _ = std::fs::remove_file(&destination);
            }
            shared.publish(|snapshot| {
                snapshot.download = Some(Download {
                    name: entry.name,
                    done: entry.bytes,
                    total: entry.bytes,
                    phase: match &cause {
                        Some(cause) => DownloadPhase::Failed(cause.clone()),
                        None => DownloadPhase::Done,
                    },
                });
            });
            if let Some(service) = SERVICE.get() {
                service.publish_models();
            }
        });
    }

    /// « Annuler » on a model transfer: the partial file goes, and the card says
    /// the transfer was cancelled.
    pub fn cancel_download(&self) {
        self.shared.cancel_flag().store(true, Ordering::Relaxed);
    }

    /// Throw a finished or failed transfer off the card.
    pub fn dismiss_download(&self) {
        self.shared.publish(|snapshot| snapshot.download = None);
    }

    /// Enrol this PC as a `client` worker (admin): the token is written 0600 and
    /// then started, never shown again.
    pub fn enrol(&self, name: &str) {
        let name = if name.trim().is_empty() {
            hostname()
        } else {
            name.trim().to_owned()
        };
        let Some(api) = self.shared.api.lock().ok().and_then(|api| api.clone()) else {
            self.note("Connectez-vous pour enrôler cet ordinateur.");
            return;
        };
        self.shared.publish(|snapshot| snapshot.worker.busy = true);
        let Some(runtime) = self.shared.runtime.lock().ok().and_then(|rt| rt.clone()) else {
            return;
        };
        let shared = self.shared.clone();
        runtime.spawn(async move {
            #[derive(Deserialize)]
            struct Row {
                #[serde(default)]
                id: String,
                #[serde(default)]
                name: String,
            }
            #[derive(Deserialize)]
            struct Issued {
                #[serde(default)]
                worker: Option<Row>,
                #[serde(default)]
                token: String,
            }
            let body = serde_json::json!({ "name": name, "kind": "client" });
            let answer: Result<Option<Issued>, ApiError> = api
                .send(
                    Lane::User,
                    reqwest::Method::POST,
                    "/transcribe/workers",
                    Some(&body),
                )
                .await;
            match answer {
                Ok(Some(issued)) if !issued.token.is_empty() => {
                    let path = worker::token_path();
                    let written = write_token(&path, &issued.token);
                    let (id, row_name) = issued
                        .worker
                        .map(|row| (row.id, row.name))
                        .unwrap_or_default();
                    shared.publish(|snapshot| {
                        snapshot.worker = Worker {
                            enrolled: written.is_ok(),
                            id: (!id.is_empty()).then_some(id),
                            name: if row_name.is_empty() {
                                name.clone()
                            } else {
                                row_name
                            },
                            enabled: true,
                            running: false,
                            busy: false,
                            ..Worker::default()
                        };
                        snapshot.note = Some(match &written {
                            Ok(()) => format!(
                                "Cet ordinateur est enrôlé. Le jeton est dans {} (lisible par \
                                 vous seul) ; il n’est plus affiché.",
                                path.display()
                            ),
                            Err(cause) => format!(
                                "L’ordinateur est enrôlé mais le jeton n’a pas pu être écrit \
                                 dans {} : {cause}",
                                path.display()
                            ),
                        });
                    });
                    if let Some(service) = SERVICE.get() {
                        service.dismiss_download();
                    }
                }
                Ok(_) => shared.publish(|snapshot| {
                    snapshot.worker.busy = false;
                    snapshot.note =
                        Some("Le serveur n’a pas renvoyé de jeton d’appareil.".to_owned());
                }),
                Err(error) => shared.publish(|snapshot| {
                    snapshot.worker.busy = false;
                    snapshot.note = Some(error.cause_fr().to_owned());
                }),
            }
        });
    }

    /// Revoke this PC's device and forget its token.
    ///
    /// The id may not be known yet (a token written before any list was read):
    /// the task asks the server for the rows and revokes the `client` row this
    /// machine's token belongs to by NAME, then deletes the token whatever the
    /// server answered — a credential this app can no longer use must not stay
    /// readable on the disk.
    pub fn revoke(&self) {
        let Some(api) = self.shared.api.lock().ok().and_then(|api| api.clone()) else {
            self.note("Connectez-vous pour révoquer cet ordinateur.");
            return;
        };
        let Some(runtime) = self.shared.runtime.lock().ok().and_then(|rt| rt.clone()) else {
            return;
        };
        let snapshot = self.snapshot();
        let known = snapshot.worker.id.clone();
        let name = if snapshot.worker.name.is_empty() {
            hostname()
        } else {
            snapshot.worker.name.clone()
        };
        self.shared.stopping.store(true, Ordering::SeqCst);
        self.send(Control::Stop);
        self.shared.publish(|snapshot| snapshot.worker.busy = true);
        let shared = self.shared.clone();
        runtime.spawn(async move {
            #[derive(Deserialize, Default)]
            struct Row {
                #[serde(default)]
                id: String,
                #[serde(default)]
                name: String,
                #[serde(default)]
                kind: String,
                #[serde(default)]
                revoked: Option<String>,
            }
            #[derive(Deserialize, Default)]
            struct List {
                #[serde(default)]
                workers: Vec<Row>,
            }
            let mut id = known;
            if id.is_none() {
                if let Ok(fresh) = api.get::<List>(Lane::User, "/transcribe/workers").await {
                    id = fresh
                        .value
                        .workers
                        .into_iter()
                        .find(|row| {
                            row.kind == "client"
                                && row.revoked.is_none()
                                && row.name.trim() == name.trim()
                        })
                        .map(|row| row.id);
                }
            }
            let mut cause = None;
            if let Some(id) = id.filter(|id| !id.is_empty()) {
                let path = format!("/transcribe/workers/{}", encode_segment(&id));
                let result: Result<Option<serde_json::Value>, ApiError> = api
                    .send::<(), serde_json::Value>(Lane::User, reqwest::Method::DELETE, &path, None)
                    .await;
                cause = result.err().map(|error| error.cause_fr().to_owned());
            }
            let _ = std::fs::remove_file(worker::token_path());
            shared.publish(|snapshot| {
                snapshot.worker = Worker::default();
                snapshot.worker.running = false;
                snapshot.note = Some(match &cause {
                    Some(cause) => format!(
                        "Jeton supprimé de cet ordinateur, mais le serveur n’a pas confirmé la \
                         révocation : {cause}"
                    ),
                    None => "Cet ordinateur n’est plus enrôlé et son jeton a été supprimé.".to_owned(),
                });
            });
        });
    }

    /// Ask the server for this PC's worker row (the lease it holds, its state).
    pub fn refresh_worker(&self) {
        if self
            .shared
            .worker_checked
            .lock()
            .ok()
            .and_then(|at| *at)
            .is_some_and(|at| at.elapsed() < Duration::from_secs(10))
        {
            return;
        }
        if let Ok(mut at) = self.shared.worker_checked.lock() {
            *at = Some(Instant::now());
        }
        let Some(api) = self.shared.api.lock().ok().and_then(|api| api.clone()) else {
            return;
        };
        let Some(runtime) = self.shared.runtime.lock().ok().and_then(|rt| rt.clone()) else {
            return;
        };
        let shared = self.shared.clone();
        let token = std::fs::read_to_string(worker::token_path()).ok();
        let our_name = hostname();
        runtime.spawn(async move {
            #[derive(Deserialize, Default)]
            struct Lease {
                #[serde(default)]
                episode_id: String,
            }
            #[derive(Deserialize, Default)]
            struct Row {
                #[serde(default)]
                id: String,
                #[serde(default)]
                name: String,
                #[serde(default)]
                kind: String,
                #[serde(default)]
                enabled: bool,
                #[serde(default)]
                last_seen: Option<String>,
                #[serde(default)]
                revoked: Option<String>,
                #[serde(default)]
                lease: Option<Lease>,
            }
            #[derive(Deserialize, Default)]
            struct List {
                #[serde(default)]
                workers: Vec<Row>,
            }
            let Ok(fresh) = api.get::<List>(Lane::User, "/transcribe/workers").await else {
                return;
            };
            let known = shared.lock().worker.id.clone();
            let mine = fresh.value.workers.into_iter().find(|row| {
                token.is_some()
                    && row.kind == "client"
                    && row.revoked.is_none()
                    && (known.as_deref() == Some(row.id.as_str())
                        || row.name.trim() == our_name.trim())
            });
            let Some(row) = mine else {
                return;
            };
            let _ = our_name;
            shared.publish(|snapshot| {
                snapshot.worker = Worker {
                    enrolled: token.is_some(),
                    id: Some(row.id),
                    name: row.name,
                    enabled: row.enabled,
                    last_seen: row.last_seen,
                    lease: row.lease.map(|lease| lease.episode_id).filter(|id| !id.is_empty()),
                    running: snapshot.worker.running,
                    busy: false,
                };
            });
        });
    }

    /// Start the queue worker on this PC (once enrolled, an admin's switch).
    pub fn start_worker(&self) {
        if self.snapshot().job.is_some() {
            self.note(
                "Une transcription locale tourne déjà : elle utilise le même moteur. \
                 Attendez la fin ou arrêtez-la.",
            );
            return;
        }
        let settings = self.shared.lock().settings.clone();
        let Some(model) = settings.model_path() else {
            self.note("Téléchargez un modèle avant de faire tourner la file du serveur ici.");
            return;
        };
        let Some(server) = self.shared.server.lock().ok().and_then(|s| s.clone()) else {
            self.note("Aucun serveur n’est configuré.");
            return;
        };
        let program = self.program();
        let stderr = work_dir().join("supervisor.err");
        // One engine: a child started in `local` mode cannot become a worker
        // without being restarted, and it is idle (no job above), so this is
        // free.
        if self.shared.child.lock().ok().is_some_and(|child| child.is_some()) {
            self.shared.stopping.store(true, Ordering::SeqCst);
            self.send(Control::Stop);
            self.wait_child_exit(Duration::from_secs(2));
        }
        if self.ensure_child(&program, &model, &settings, Some(&server), &stderr) {
            self.shared.publish(|snapshot| {
                snapshot.worker.running = true;
                snapshot.note =
                    Some("La file de transcription du serveur tourne sur cet ordinateur.".to_owned());
            });
            self.probe(false);
        }
    }

    /// Wait (bounded) for the child to leave after a `stop`, then take it out.
    fn wait_child_exit(&self, budget: Duration) {
        let deadline = Instant::now() + budget;
        loop {
            {
                let mut child = self.shared.child.lock().unwrap_or_else(|e| e.into_inner());
                match child.as_mut().map(|child| child.try_wait()) {
                    Some(Ok(Some(_))) | Some(Err(_)) | None => {
                        child.take();
                        *self.shared.stdin.lock().unwrap_or_else(|e| e.into_inner()) = None;
                        return;
                    }
                    _ => {}
                }
            }
            if Instant::now() >= deadline {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Stop the child (and the engine with it).
    pub fn stop_child(&self) {
        self.send(Control::Stop);
        let mut child = self.shared.child.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(mut running) = child.take() {
            terminate(&mut running);
        }
        *self.shared.stdin.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.shared.publish(|snapshot| {
            snapshot.worker.running = false;
            snapshot.job = None;
        });
    }

    fn note(&self, message: impl Into<String>) {
        let message = message.into();
        self.shared.publish(|snapshot| snapshot.note = Some(message));
    }

    /// Stop the supervisor and JOIN the threads that talk to it. Called at
    /// process exit (before the network runtime goes) and by the tests.
    ///
    /// Neither thread borrows `NetRuntime`: the reader blocks on the child's
    /// pipe, the monitor on its own clock, and the HTTP work runs as runtime
    /// tasks (which tokio drops). The child is killed here so no whisper process
    /// outlives the window (rule 1 after the person is gone).
    pub fn shutdown(&self) {
        if self.stopped.swap(true, Ordering::SeqCst) {
            return;
        }
        self.shared.stop.store(true, Ordering::SeqCst);
        self.send(Control::Stop);
        *self.shared.stdin.lock().unwrap_or_else(|e| e.into_inner()) = None;
        if let Some(mut child) = self
            .shared
            .child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            terminate(&mut child);
        }
        if let Some(monitor) = self
            .monitor
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = monitor.join();
        }
        if let Some(reader) = self.reader.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = reader.join();
        }
    }
}

impl Snapshot {
    fn trim_failures(failures: &mut Vec<Failure>) {
        let extra = failures.len().saturating_sub(FAILURES_KEPT);
        if extra > 0 {
            failures.drain(0..extra);
        }
    }
}

// --- the bits that only touch the filesystem, the child or the wire ----------

/// `nice 19` for the supervisor (its engine child applies it again for itself).
fn lower_priority() {
    const NICE: i32 = 19;
    unsafe extern "C" {
        fn nice(adjustment: i32) -> i32;
    }
    // SAFETY: `nice` takes an int and returns the new priority; no memory.
    unsafe {
        nice(NICE);
    }
}

/// Stop a child (a process group of its own): CONT, TERM, then KILL.
fn terminate(child: &mut Child) {
    let group = child.id() as i32;
    #[cfg(unix)]
    {
        const SIGCONT: i32 = 18;
        const SIGTERM: i32 = 15;
        const SIGKILL: i32 = 9;
        unsafe extern "C" {
            fn kill(pid: i32, signal: i32) -> i32;
        }
        // SAFETY: `kill` with a negative pid signals a process group; both
        // arguments are plain integers and nothing in memory is touched.
        unsafe {
            kill(-group, SIGCONT);
            kill(-group, SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // SAFETY: same call, same reasoning; a stopped process ignores TERM.
        unsafe {
            kill(-group, SIGKILL);
        }
        let _ = child.wait();
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// One control command on the child's stdin, flushed.
fn send_via(shared: &Shared, command: Control) {
    let mut stdin = shared.stdin.lock().unwrap_or_else(|e| e.into_inner());
    let Some(pipe) = stdin.as_mut() else {
        return;
    };
    let Ok(mut line) = serde_json::to_vec(&command) else {
        return;
    };
    line.push(b'\n');
    if let Err(error) = pipe.write_all(&line).and_then(|()| pipe.flush()) {
        // The child is gone or its pipe broke: the command did not happen, and
        // a job that waits for it must say why (rule 2), not spin.
        warn!("canal du superviseur : {error}");
        shared.publish(|snapshot| {
            snapshot.failures.push(Failure {
                episode: snapshot
                    .job
                    .as_ref()
                    .map(|job| job.episode.clone())
                    .unwrap_or_default(),
                code: "control_channel".to_owned(),
                message: format!("Le superviseur de transcription ne répond plus ({error})."),
            });
            Snapshot::trim_failures(&mut snapshot.failures);
            snapshot.job = None;
        });
    }
}

fn apply_event(shared: &Shared, event: Event) {
    match event {
        Event::Engine {
            backend,
            device,
            reason,
        } => {
            let backend = Backend::parse(&backend);
            let cause = reason.as_deref().and_then(FallbackCause::parse);
            shared.publish(|snapshot| {
                let next = match (backend, cause) {
                    (Some(Backend::Cpu), Some(cause)) => Some(Engine::Cpu { cause }),
                    (Some(Backend::Vulkan), _) => Some(Engine::Vulkan {
                        device: device.clone(),
                    }),
                    (Some(Backend::Cpu), None) => Some(Engine::Cpu {
                        cause: FallbackCause::NoVulkanDevice,
                    }),
                    (None, _) => None,
                };
                if let Some(next) = next {
                    snapshot.engine = next;
                }
                if let Some(job) = snapshot.job.as_mut() {
                    job.backend = backend;
                    if let Some(device) = device {
                        job.device = Some(device);
                    }
                }
            });
        }
        Event::Progress {
            episode_id,
            done_ms,
            duration_ms,
        } => shared.publish(|snapshot| {
            if let Some(job) = snapshot.job.as_mut()
                && job.episode == episode_id
            {
                job.done_ms = done_ms.max(0);
                if duration_ms > 0 {
                    job.duration_ms = duration_ms;
                }
                job.phase = Phase::Transcribing;
            }
        }),
        Event::Done {
            episode_id,
            segments,
        } => shared.publish(|snapshot| {
            if snapshot
                .job
                .as_ref()
                .is_some_and(|job| job.episode == episode_id)
            {
                snapshot.job = None;
            }
            snapshot.last_done = Some(episode_id.clone());
            snapshot.note = Some(if segments.is_empty() {
                "Transcription terminée : aucune parole reconnue dans cet épisode.".to_owned()
            } else {
                format!(
                    "Transcription terminée : {} ligne{} écrite{} sur cet ordinateur.",
                    segments.len(),
                    if segments.len() > 1 { "s" } else { "" },
                    if segments.len() > 1 { "s" } else { "" }
                )
            });
        }),
        Event::Error {
            episode_id,
            code,
            message,
        } => shared.publish(|snapshot| {
            if snapshot
                .job
                .as_ref()
                .is_some_and(|job| job.episode == episode_id)
            {
                snapshot.job = None;
            }
            snapshot.failures.push(Failure {
                episode: episode_id,
                code,
                message,
            });
            Snapshot::trim_failures(&mut snapshot.failures);
        }),
        Event::Idle => shared.publish(|snapshot| {
            snapshot.worker.running = true;
            snapshot.worker.busy = false;
        }),
    }
}

/// The stderr tail of a supervisor that died: enough for the cause, never a
/// path to a model or a token (the supervisor does not print either).
fn read_stderr(shared: &Shared) -> String {
    let path = shared.stderr.lock().ok().and_then(|path| path.clone());
    let Some(path) = path else {
        return String::new();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return String::new();
    };
    let text = String::from_utf8_lossy(&bytes);
    let tail = text.lines().rev().take(3).collect::<Vec<_>>().join(" ");
    let tail = tail.trim();
    if tail.is_empty() {
        return String::new();
    }
    let short: String = tail.chars().take(240).collect();
    short
}

/// Run `neezer-transcribe probe [--cpu]` and read its one control line.
fn run_probe(program: &Path, cpu: bool) -> Engine {
    let mut command = Command::new(program);
    command.arg("probe");
    if cpu {
        command.arg("--cpu");
    }
    match command.output() {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            parse_probe(&text).unwrap_or_else(|| Engine::Unavailable {
                message: "Le moteur de transcription n’a pas répondu lisiblement.".to_owned(),
            })
        }
        Ok(output) => Engine::Unavailable {
            message: format!(
                "Le binaire de transcription s’est arrêté ({}) : {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .next()
                    .unwrap_or("sans message")
            ),
        },
        Err(error) => Engine::Unavailable {
            message: format!(
                "Le binaire neezer-transcribe est introuvable ({error}). Réinstallez \
                 l’application : il est livré avec elle."
            ),
        },
    }
}

/// The `{"event":"engine"}` line a probe printed.
pub fn parse_probe(text: &str) -> Option<Engine> {
    let line = text.lines().find(|line| line.contains("\"event\":\"engine\""))?;
    let event: Event = serde_json::from_str(line).ok()?;
    let Event::Engine {
        backend,
        device,
        reason,
    } = event
    else {
        return None;
    };
    match (Backend::parse(&backend), reason.as_deref().and_then(FallbackCause::parse)) {
        (Some(Backend::Vulkan), _) => Some(Engine::Vulkan { device }),
        (Some(Backend::Cpu), Some(cause)) => Some(Engine::Cpu { cause }),
        (Some(Backend::Cpu), None) => Some(Engine::Cpu {
            cause: FallbackCause::NoVulkanDevice,
        }),
        (None, _) => None,
    }
}

/// Where a local job's audio comes from, and whether it must be fetched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioPlan {
    /// A file already on this machine (an offline download).
    Local(PathBuf),
    /// Stream to the cache at Opus 128.
    Stream(PathBuf),
}

/// Decide: an offline download wins (no network, no wait), otherwise the
/// episode is streamed once into the cache.
pub fn audio_plan(offline: Option<PathBuf>, episode: &str) -> AudioPlan {
    match offline {
        Some(path) if path.is_file() => AudioPlan::Local(path),
        _ => AudioPlan::Stream(work_dir().join(format!("{episode}.opus"))),
    }
}

/// Fetch one episode's audio on `Lane::Bulk`: the plan's lane for a
/// transcription's bytes, so the request waits for a quiet second, sends
/// `X-NS-Background: 1` and is paced by the token bucket — the tapped track is
/// never slowed by a text nobody is waiting on. Its progress is on the job card
/// (rule 2), and the download is one bounded Opus 128 body, kept for a retry.
async fn stream_audio(shared: &Arc<Shared>, episode: &str) -> Result<PathBuf, String> {
    let offline = crate::wiring::offline::local_path(episode);
    let plan = audio_plan(offline, episode);
    if let AudioPlan::Local(path) = plan {
        return Ok(path);
    }
    let AudioPlan::Stream(destination) = plan else {
        unreachable!()
    };
    if let Ok(metadata) = std::fs::metadata(&destination)
        && metadata.len() > 1024
    {
        return Ok(destination);
    }
    let api = shared
        .api
        .lock()
        .ok()
        .and_then(|api| api.clone())
        .ok_or_else(|| "Connectez-vous pour transcrire cet épisode sur cet ordinateur.".to_owned())?;
    let cancel = Arc::new(AtomicBool::new(false));
    let progress_shared = shared.clone();
    let name = episode.to_owned();
    let progress = move |done: u64, total: Option<u64>| {
        progress_shared.publish(|snapshot| {
            if let Some(job) = snapshot.job.as_mut()
                && job.episode == name
            {
                job.phase = Phase::Audio { done, total };
            }
        });
    };
    let quality = Quality::Opus128.wire().unwrap_or("OPUS_128");
    let path = format!("/stream/{episode}?q={quality}");
    api.download_to(Lane::Bulk, &path, &destination, progress, &cancel)
        .await
        .map_err(|error| error.cause_fr().to_owned())?;
    Ok(destination)
}

/// Write the device token with mode 0600 (a full-account-grade credential is
/// not readable by another user), through a `.part` file and a rename.
fn write_token(path: &Path, token: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("token.part");
    {
        use std::io::Write as _;
        let mut file = std::fs::File::create(&temporary)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(token.trim().as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }
    std::fs::rename(&temporary, path)
}

fn worker_file_present() -> bool {
    std::fs::metadata(worker::token_path()).is_ok_and(|meta| meta.len() > 0)
}

/// This machine's name, for the enrolment dialog's default and the worker row.
pub fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .or_else(|_| std::fs::read_to_string("/etc/hostname"))
        .map(|name| name.trim().to_owned())
        .ok()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "neezer-desktop".to_owned())
}

/// Whether the machine runs on its battery (a laptop unplugged). No battery
/// entry is a desktop on the mains, never a pause.
pub fn on_battery() -> bool {
    let Ok(entries) = std::fs::read_dir("/sys/class/power_supply") else {
        return false;
    };
    entries.flatten().any(|entry| {
        let dir = entry.path();
        let kind = std::fs::read_to_string(dir.join("type")).unwrap_or_default();
        let status = std::fs::read_to_string(dir.join("status")).unwrap_or_default();
        kind.trim() == "Battery" && status.trim() == "Discharging"
    })
}

/// The governor's own reading of `on_battery`, testable without a laptop.
pub fn battery_from(entries: &[(&str, &str)]) -> bool {
    entries
        .iter()
        .any(|(kind, status)| kind.trim() == "Battery" && status.trim() == "Discharging")
}

/// Serialize the snapshot's model rows for a test that counts bytes: the screen
/// holds this and nothing else (rule 4, `testmem`).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testmem::Meter;
    use std::sync::atomic::AtomicU32;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nz-t4-{tag}-{}-{}",
            std::process::id(),
            TEST_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    static TEST_ID: AtomicU32 = AtomicU32::new(0);

    /// The pause policy: the battery first (a choice), then the sound, then the
    /// interface, and `Normal` ignores the frames.
    #[test]
    fn the_governor_yields_to_battery_sound_then_the_interface() {
        let mut settings = Settings::default();
        let quiet = Signals {
            playing: false,
            ..Signals::default()
        };
        assert_eq!(pause_reason(&quiet, &settings, false), None);

        let mut battery = quiet;
        battery.on_battery = true;
        assert_eq!(
            pause_reason(&battery, &settings, false),
            Some(PauseReason::Battery)
        );
        settings.battery_only = false;
        assert_eq!(pause_reason(&battery, &settings, false), None);

        let mut playing = quiet;
        playing.playing = true;
        assert_eq!(
            pause_reason(&playing, &settings, true),
            Some(PauseReason::Audio)
        );
        playing.playing = false;
        assert_eq!(pause_reason(&playing, &settings, true), None, "not playing");

        let mut frame = quiet;
        frame.frame_ms = Some(40.0);
        assert_eq!(
            pause_reason(&frame, &settings, false),
            Some(PauseReason::Frame)
        );
        settings.priority = Priority::Low;
        assert_eq!(pause_reason(&frame, &settings, false), None);
        settings.priority = Priority::Lowest;
        frame.frame_ms = Some(FRAME_HITCH_MS);
        assert_eq!(pause_reason(&frame, &settings, false), None, "at the line");
    }

    /// The hysteresis is the measured one: one pause for a burst, a resume only
    /// after three seconds of quiet, and no command when nothing changes.
    #[test]
    fn one_burst_is_one_pause_and_the_resume_waits_three_seconds() {
        let settings = Settings::default();
        let mut governor = Governor::default();
        let start = Instant::now();
        let playing = Signals {
            playing: true,
            position_s: 0.0,
            ..Signals::default()
        };
        assert_eq!(governor.step(start, playing, &settings), None, "no sample yet");
        assert_eq!(
            governor.step(start + Duration::from_millis(250), playing, &settings),
            Some(Control::Pause),
            "the playhead did not advance"
        );
        assert_eq!(governor.paused, Some(PauseReason::Audio));
        // Stable at 1.0×: still paused (no repeated command), the quiet clock
        // has not started.
        let mut healthy = playing;
        healthy.position_s = 0.3;
        assert_eq!(
            governor.step(start + Duration::from_millis(550), healthy, &settings),
            None
        );
        healthy.position_s = 0.6;
        assert_eq!(
            governor.step(start + Duration::from_millis(850), healthy, &settings),
            None
        );
        healthy.position_s = 3.15;
        assert_eq!(
            governor.step(start + Duration::from_millis(3_400), healthy, &settings),
            None,
            "2.85 s of quiet is not the three seconds the hysteresis asks for"
        );
        healthy.position_s = 3.35;
        assert_eq!(
            governor.step(start + Duration::from_millis(3_600), healthy, &settings),
            Some(Control::Resume),
            "3.05 s of quiet since the first healthy sample"
        );
        assert_eq!(governor.paused, None);
    }

    /// A model the person chose is the one used, even when a "better" rung of the
    /// ladder is on disk; with nothing chosen, the ladder decides.
    #[test]
    fn the_chosen_model_is_authoritative() {
        let dir = scratch("choice");
        let model = |name: &str, bytes: u64| {
            let path = dir.join(name);
            let mut content = vec![0u8; bytes as usize];
            content[..4].copy_from_slice(b"ggml");
            std::fs::write(&path, content).expect("write");
            path
        };
        let tiny = model(models::TINY, models::MIN_MODEL_BYTES);
        let small = model(models::SMALL, models::MIN_MODEL_BYTES);
        let settings = Settings {
            model: Some(models::TINY.to_owned()),
            ..Settings::default()
        };
        assert_eq!(
            model_in(&dir, &settings, Some(32_000_000_000)),
            Some(tiny),
            "an explicit choice must not be replaced by the ladder"
        );
        let automatic = Settings::default();
        assert_eq!(
            model_in(&dir, &automatic, Some(32_000_000_000)),
            Some(small),
            "without a choice the ladder's best installed rung wins"
        );
        assert_eq!(model_in(&dir, &Settings::default(), None), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A model download is held to the crate's pin by its size, then its digest,
    /// then `valid_model`; a failed check names the cause in French.
    #[test]
    fn a_model_is_kept_only_when_its_size_and_digest_match() {
        let dir = scratch("check");
        let short = dir.join("short.bin");
        std::fs::write(&short, b"ggml").expect("write");
        match check_model(models::BASE, &short) {
            ModelCheck::WrongSize { got, want } => {
                assert_eq!(got, 4);
                assert_eq!(want, models::bytes_for(models::BASE).unwrap());
            }
            other => panic!("unexpected: {other:?}"),
        }
        // A file of the right size with other bytes is a digest failure, even
        // with the GGML magic.
        let wrong = dir.join("wrong.bin");
        let length = models::bytes_for(models::VAD).unwrap() as usize;
        let mut content = vec![0u8; length];
        content[..4].copy_from_slice(b"ggml");
        std::fs::write(&wrong, &content).expect("write");
        assert_eq!(check_model(models::VAD, &wrong), ModelCheck::WrongDigest);
        assert!(
            check_model(models::VAD, &wrong)
                .cause_fr(models::VAD)
                .unwrap()
                .contains("SHA-256")
        );
        assert!(check_model("ggml-nope.bin", &wrong).cause_fr("ggml-nope.bin").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The pinned files are the only ones we fetch, from the two hosts the
    /// hashes came from.
    #[test]
    fn the_download_urls_are_the_pinned_hosts() {
        for entry in models::FILES {
            let url = download_url(entry.name).expect("every pinned file has a URL");
            assert!(url.ends_with(entry.name), "{url}");
            if entry.name == models::VAD {
                assert!(url.starts_with(VAD_BASE), "{url}");
            } else {
                assert!(url.starts_with(MODELS_BASE), "{url}");
            }
        }
        assert_eq!(download_url("ggml-nope.bin"), None);
        // The settings screen may also download `ggml-tiny`/`ggml-base`: both
        // are pinned, so both are reachable.
        assert!(download_url(models::TINY).is_some());
        assert!(download_url(models::BASE).is_some());
    }

    /// The child's command line carries the model, the language and the CPU
    /// choice; the worker mode adds the server URL and leaves `--out-dir` to the
    /// local mode (the supervisor has one transcript folder for both).
    #[test]
    fn the_supervisor_is_asked_for_the_chosen_model_and_language() {
        let settings = Settings {
            language: "fr".to_owned(),
            cpu: true,
            ..Settings::default()
        };
        let args = supervisor_args(&settings, Path::new("/m/ggml-base.bin"), None, "fr");
        assert_eq!(args[0], "local");
        assert!(args.windows(2).any(|pair| pair == ["--model", "/m/ggml-base.bin"]));
        assert!(args.windows(2).any(|pair| pair == ["--lang", "fr"]));
        assert!(args.contains(&"--cpu".to_owned()));
        assert!(args.contains(&"--out-dir".to_owned()));

        let worker = supervisor_args(
            &Settings::default(),
            Path::new("/m/ggml-base.bin"),
            Some("http://127.0.0.1:5000"),
            "auto",
        );
        assert_eq!(worker[0], "worker");
        assert!(worker.windows(2).any(|pair| pair == ["--url", "http://127.0.0.1:5000"]));
        assert!(!worker.contains(&"--lang".to_owned()), "auto is the default");
        assert!(!worker.contains(&"--out-dir".to_owned()));
    }

    /// A sidecar read into the panel's shape: absolute milliseconds become
    /// seconds, and the origin is kept for the badge.
    #[test]
    fn a_sidecar_becomes_the_panels_lines() {
        let dir = scratch("sidecar");
        let episode = "11111111-2222-3333-4444-555555555555";
        let document = sidecar::build(
            sidecar::Meta {
                episode_id: episode.to_owned(),
                language: Some("fr".to_owned()),
                engine: Some("whisper.cpp".to_owned()),
                model: Some(models::BASE.to_owned()),
                engine_version: Some("1.8.3".to_owned()),
                duration_ms: 3_000,
                created: Some("2026-11-17T10:00:00Z".to_owned()),
                updated: None,
            },
            &[nz_transcribe::Segment::new(1_500, 3_000, "Bonjour.")],
        )
        .expect("document");
        let pair = sidecar::pair_for(&dir.join(episode));
        sidecar::write(&pair, &document).expect("write");
        let local = read_sidecar(&dir, episode).expect("the transcript");
        assert_eq!(local.language.as_deref(), Some("fr"));
        assert_eq!(local.model.as_deref(), Some(models::BASE));
        assert_eq!(local.segments.len(), 1);
        assert!((local.segments[0].start - 1.5).abs() < f64::EPSILON);
        assert!((local.segments[0].end - 3.0).abs() < f64::EPSILON);
        assert_eq!(local.segments[0].text, "Bonjour.");

        // No transcript, a directory traversal attempt, and a hostile id.
        assert!(read_sidecar(&dir, "99999999-2222-3333-4444-555555555555").is_none());
        assert!(read_sidecar(&dir, "../../etc/passwd").is_none());
        assert!(!is_episode_id(".."));
        assert!(!is_episode_id("a/b"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A partial sidecar is what a restarted job resumes at, and the panel says
    /// so before the first sample.
    #[test]
    fn a_partial_sidecar_is_the_resume_point() {
        let dir = scratch("partial");
        let episode = "11111111-2222-3333-4444-555555555555";
        let body = serde_json::json!({
            "episode_id": episode,
            "done_ms": 600_000,
            "segments": [{"start_ms": 0, "end_ms": 1000, "text": "a"}]
        });
        std::fs::write(
            dir.join(format!("{episode}.partial.json")),
            serde_json::to_vec(&body).unwrap(),
        )
        .expect("write");
        let partial = partial_of(&dir, episode).expect("a resume point");
        assert_eq!(partial.done_ms, 600_000);
        assert_eq!(partial.segments, 1);
        assert!(partial_of(&dir, "22222222-2222-3333-4444-555555555555").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The probe's line becomes the settings card's state, cause and sentence.
    #[test]
    fn a_probe_line_becomes_the_engine_state() {
        assert_eq!(
            parse_probe(
                "{\"event\":\"engine\",\"backend\":\"vulkan\",\"device\":\"AMD Radeon 780M\"}\n"
            ),
            Some(Engine::Vulkan {
                device: Some("AMD Radeon 780M".to_owned())
            })
        );
        let cpu = parse_probe(
            "{\"event\":\"engine\",\"backend\":\"cpu\",\"reason\":\"no_vulkan_device\"}\n",
        )
        .expect("CPU");
        assert_eq!(cpu, Engine::Cpu { cause: FallbackCause::NoVulkanDevice });
        assert!(cpu.label_fr().contains("no_vulkan_device"));
        assert_eq!(parse_probe(""), None);
        assert_eq!(parse_probe("{\"event\":\"other\"}"), None);
    }

    /// The battery parser: a laptop discharging pauses, a desktop with no
    /// battery never does.
    #[test]
    fn only_a_discharging_battery_pauses() {
        assert!(battery_from(&[("Mains", "Unknown"), ("Battery", "Discharging")]));
        assert!(!battery_from(&[("Mains", "Unknown")]));
        assert!(!battery_from(&[("Battery", "Charging")]));
        assert!(!battery_from(&[("Battery", "Full")]));
    }

    /// The audio plan prefers a file already on the machine (offline) and
    /// otherwise streams once into the cache.
    #[test]
    fn an_offline_episode_is_used_as_it_is() {
        let dir = scratch("audio");
        let file = dir.join("episode.opus");
        std::fs::write(&file, b"audio").expect("write");
        assert_eq!(
            audio_plan(Some(file.clone()), "e"),
            AudioPlan::Local(file)
        );
        match audio_plan(Some(dir.join("missing")), "e") {
            AudioPlan::Stream(path) => assert!(path.ends_with("e.opus")),
            other => panic!("unexpected: {other:?}"),
        }
        assert!(matches!(audio_plan(None, "e"), AudioPlan::Stream(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The token file is written 0600, through a rename, and never printed.
    #[test]
    fn the_device_token_is_a_private_file() {
        let dir = scratch("token");
        let path = dir.join("transcribe.token");
        write_token(&path, "secret-token").expect("write");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read").trim(),
            "secret-token"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{mode:o}");
        }
        assert!(!format!("{:?}", worker::Token::from_file(&path)).contains("secret-token"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What a screen holds, counted in bytes: the snapshot of the worst
    /// realistic state (six pinned models, six failures, a job in progress) is
    /// a few kilobytes, not the transcript and not a model path's contents.
    #[test]
    fn the_snapshot_costs_kilobytes_not_megabytes() {
        let before = Meter::now();
        let snapshot = Snapshot {
            version: 9,
            settings: Settings {
                model: Some(models::TURBO_Q8.to_owned()),
                language: "fr".to_owned(),
                ..Settings::default()
            },
            free_memory: Some(31_000_000_000),
            engine: Engine::Vulkan {
                device: Some("AMD Radeon 780M (RADV PHOENIX)".to_owned()),
            },
            models: models::FILES
                .iter()
                .map(|entry| ModelRow {
                    name: entry.name,
                    bytes: entry.bytes,
                    installed: false,
                    invalid: false,
                    chosen: entry.name == models::TURBO_Q8,
                    recommended: entry.name == models::TURBO_Q8,
                })
                .collect(),
            download: Some(Download {
                name: models::TURBO_Q8,
                done: 12_000_000,
                total: models::bytes_for(models::TURBO_Q8).unwrap(),
                phase: DownloadPhase::Downloading,
            }),
            job: Some(Job {
                episode: "11111111-2222-3333-4444-555555555555".to_owned(),
                phase: Phase::Audio {
                    done: 1_000,
                    total: Some(2_000),
                },
                done_ms: 0,
                duration_ms: 0,
                backend: Some(Backend::Vulkan),
                device: Some("AMD Radeon 780M".to_owned()),
                paused: Some(PauseReason::Frame),
                title: Some("Épisode 12 · une heure de conversation".to_owned()),
            }),
            failures: (0..FAILURES_KEPT)
                .map(|index| Failure {
                    episode: "11111111-2222-3333-4444-555555555555".to_owned(),
                    code: "engine_crashed".to_owned(),
                    message: format!("{index} : le moteur s’est arrêté avant la fin"),
                })
                .collect(),
            worker: Worker {
                enrolled: true,
                id: Some("99999999-2222-3333-4444-555555555555".to_owned()),
                name: "portable".to_owned(),
                enabled: true,
                ..Worker::default()
            },
            last_done: None,
            note: Some("Transcription terminée".to_owned()),
        };
        let kept = before.since();
        assert!(
            kept < 16_384,
            "the snapshot of the worst state must stay under 16 KB, it held {kept} bytes"
        );
        // The strings themselves are the only large part: 6 × ~25 B of name, 6
        // failures of ~55 B and one model's path-free label.
        assert!(snapshot.failures.len() <= FAILURES_KEPT);
    }

    /// The service: a fake supervisor on the control channel drives a job from
    /// its first line to its sidecar, and a `stop` reaches the child.
    #[test]
    fn the_supervisor_drives_a_local_job_from_a_line_to_its_sidecar() {
        let dir = scratch("service");
        let episode = "11111111-2222-3333-4444-555555555555";
        let transcript = dir.join("transcript.json");
        let script = dir.join("fake-transcribe.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\n\
                 read line\n\
                 echo '{{\"event\":\"engine\",\"backend\":\"vulkan\",\"device\":\"RADV 780M\"}}'\n\
                 echo '{{\"event\":\"progress\",\"episode_id\":\"{episode}\",\"done_ms\":600000,\"duration_ms\":1200000}}'\n\
                 echo '{{\"event\":\"done\",\"episode_id\":\"{episode}\",\"segments\":[{{\"start_ms\":0,\"end_ms\":1500,\"text\":\"Bonjour.\"}}]}}'\n\
                 echo \"$line\" > {}/command.txt\n\
                 echo $$ > {}/pid.txt\n\
                 sleep 0.2\n",
                dir.display(),
                dir.display()
            ),
        )
        .expect("script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
        }
        let service = Arc::new(Service::new());
        service.set_program(script);
        service.start_monitor();
        let audio = dir.join("audio.opus");
        std::fs::write(&audio, b"audio").expect("write");

        // Drive the job directly (no model download, no network): the thread
        // that reads the fake supervisor's lines is what this test proves.
        let model = dir.join(models::BASE);
        let settings = Settings::default();
        let program = service.program();
        assert!(service.ensure_child(
            &program,
            &model,
            &settings,
            None,
            &dir.join("stderr.log")
        ));
        service.shared.publish(|snapshot| {
            snapshot.job = Some(Job {
                episode: episode.to_owned(),
                phase: Phase::Transcribing,
                done_ms: 0,
                duration_ms: 0,
                backend: None,
                device: None,
                paused: None,
                title: None,
            });
        });
        service.send(Control::Local {
            episode_id: episode.to_owned(),
            audio: audio.display().to_string(),
            language: Some("fr".to_owned()),
        });
        // Wait for the reader thread's events (an event, never a clock).
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let snapshot = service.snapshot();
            if snapshot.job.is_none() && snapshot.last_done.as_deref() == Some(episode) {
                assert_eq!(snapshot.engine, Engine::Vulkan {
                    device: Some("RADV 780M".to_owned())
                });
                assert!(snapshot.note.unwrap_or_default().contains("Transcription terminée"));
                break;
            }
            assert!(Instant::now() < deadline, "the fake supervisor said nothing: {snapshot:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
        service.shutdown();
        assert!(!transcript.exists());
        // The child is killed and the two threads that talk to it are joined:
        // `shutdown` returning is the assertion (a join is what it does), and
        // the pid is gone from /proc.
        let pid = std::fs::read_to_string(dir.join("pid.txt")).unwrap_or_default();
        let pid: u32 = pid.trim().parse().unwrap_or(0);
        assert!(pid > 0, "the fake supervisor wrote its pid");
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "the supervisor child outlived shutdown"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
    /// A local opener for the offline engine: the file is already on this
    /// machine, so the engine's `local()` seam answers and no body is opened.
    struct LocalOpener(PathBuf);

    impl nz_audio::StreamOpener for LocalOpener {
        fn open(
            &self,
            _id: &str,
            _quality: nz_audio::Quality,
            _start: Option<f64>,
            _from_byte: Option<u64>,
            _lane: nz_audio::Lane,
        ) -> Result<Box<dyn nz_audio::ByteStream>, nz_audio::SourceError> {
            Err(nz_audio::SourceError::Message(
                "fichier local dans ce banc".to_owned(),
            ))
        }

        fn ready(
            &self,
            _id: &str,
            _quality: nz_audio::Quality,
        ) -> Result<nz_audio::StreamReady, nz_audio::SourceError> {
            Ok(nz_audio::StreamReady::default())
        }

        fn local(&self, _id: &str, _quality: nz_audio::Quality) -> Option<PathBuf> {
            Some(self.0.clone())
        }
    }

    /// The two files a real measurement needs, from the environment (CI sets
    /// them for the engine's own tests; a laptop names its own).
    fn fixtures() -> Option<(PathBuf, PathBuf)> {
        let model = std::env::var_os("NZ_WHISPER_MODEL").map(PathBuf::from)?;
        let audio = std::env::var_os("NZ_WHISPER_AUDIO").map(PathBuf::from)?;
        (model.is_file() && audio.is_file()).then_some((model, audio))
    }

    /// A speech file long enough for a throughput number: `jfk.wav` repeated
    /// with ffmpeg (the sample is eleven seconds).
    fn long_speech(dir: &Path, source: &Path, repeats: u32) -> PathBuf {
        let out = dir.join("long.wav");
        let status = std::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-stream_loop"])
            .arg(repeats.to_string())
            .arg("-i")
            .arg(source)
            .args(["-c", "copy"])
            .arg(&out)
            .status()
            .expect("ffmpeg is installed");
        assert!(status.success(), "ffmpeg -stream_loop failed");
        out
    }

    /// The same speech through `neezer-transcribe file`, on Vulkan and on the
    /// CPU, best of two warm runs; prints the real-time factor of each and
    /// asserts the GPU is the faster one.
    ///
    /// Ignored by default: 176 s of speech costs ~3 s on Vulkan and ~14 s on the
    /// CPU here, which is a measurement, not a check a suite should pay on every
    /// run. `NZ_WHISPER_MODEL=… NZ_WHISPER_AUDIO=… cargo test -p neezer-desktop
    /// vulkan_is_faster -- --ignored --nocapture`.
    #[test]
    #[ignore = "mesure locale : un modèle et un audio réels"]
    fn vulkan_is_faster_than_the_cpu_on_the_same_audio() {
        let Some((model, source)) = fixtures() else {
            eprintln!("NZ_WHISPER_MODEL/NZ_WHISPER_AUDIO absent : mesure de vitesse sautée");
            return;
        };
        let service = service();
        let program = service.program();
        if !program.is_file() {
            eprintln!("{} absent : mesure de vitesse sautée", program.display());
            return;
        }
        let dir = scratch("speed");
        let audio = long_speech(&dir, &source, 15);
        let probe = std::process::Command::new("ffprobe")
            .args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"])
            .arg(&audio)
            .output()
            .expect("ffprobe");
        let seconds: f64 = String::from_utf8_lossy(&probe.stdout)
            .trim()
            .parse()
            .expect("duration");

        let run = |cpu: bool, tag: &str| -> f64 {
            let out = dir.join(format!("{tag}.json"));
            let mut command = std::process::Command::new(&program);
            command
                .arg("file")
                .arg(&audio)
                .args(["--model"])
                .arg(&model)
                .args(["--out"])
                .arg(&out);
            if cpu {
                command.arg("--cpu");
            }
            let started = Instant::now();
            let output = command.output().expect("spawn neezer-transcribe");
            assert!(
                output.status.success(),
                "{tag}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            started.elapsed().as_secs_f64()
        };
        // Warm both paths once (page cache, shader cache) then time them.
        let _ = run(false, "vulkan-warm");
        let _ = run(true, "cpu-warm");
        let vulkan = run(false, "vulkan").min(run(false, "vulkan-2"));
        let cpu = run(true, "cpu").min(run(true, "cpu-2"));
        let (v_factor, c_factor) = (seconds / vulkan, seconds / cpu);
        println!(
            "Vulkan {v_factor:.1}× temps réel ({vulkan:.2} s) · CPU {c_factor:.1}× \
             ({cpu:.2} s) sur {seconds:.0} s — facteur {:.1}×",
            v_factor / c_factor
        );
        assert!(
            vulkan < cpu,
            "Vulkan ({vulkan:.2} s) doit battre le CPU ({cpu:.2} s) sur le même audio"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Rule 1, measured: a real transcription on this machine while the engine
    /// renders a real track in real time cannot cut the sound. The engine is the
    /// app's own (offline sink, so the bench is the mixer, not a sound card) and
    /// the transcription is the app's service driving the real binary.
    ///
    /// Ignored by default, like the speed bench:
    /// `NZ_WHISPER_MODEL=… NZ_WHISPER_AUDIO=… cargo test -p neezer-desktop
    /// a_local_transcription_does_not_cut -- --ignored --nocapture`.
    #[test]
    #[ignore = "mesure locale : lecture réelle + transcription réelle"]
    fn a_local_transcription_does_not_cut_the_playback() {
        let Some((model, source)) = fixtures() else {
            eprintln!("NZ_WHISPER_MODEL/NZ_WHISPER_AUDIO absent : mesure d'underrun sautée");
            return;
        };
        let service = Arc::new(Service::new());
        let program = service.program();
        if !program.is_file() {
            eprintln!("{} absent : mesure d'underrun sautée", program.display());
            return;
        }
        service.set_program(program);
        service.start_monitor();
        let dir = scratch("underrun");
        // Thirty seconds of tone to play, and the real speech to transcribe:
        // two files, because a transcription of the track being heard would be
        // the same decoder twice.
        let track = dir.join("track.flac");
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "aevalsrc=0.5*cos(2*PI*220*t):s=48000:d=30:c=stereo",
                "-c:a",
                "flac",
            ])
            .arg(&track)
            .status()
            .expect("ffmpeg");
        assert!(status.success());
        let speech = long_speech(&dir, &source, 15);

        let mut config = nz_audio::EngineConfig::default();
        config.spool_dir = dir.join("spool");
        let (engine, mut sink) = nz_audio::Engine::new_offline(
            config,
            Arc::new(LocalOpener(track.clone())),
        );
        engine.set_volume(0.0);
        engine.load("track", nz_audio::Quality::Flac, None);
        engine.play();

        let episode = "11111111-2222-3333-4444-555555555555";
        // The CPU is the worst case for rule 1 (it contends with the decoder for
        // cores), so the measurement runs the slower path: a GPU job would only
        // make the window easier.
        service.shared.publish(|snapshot| {
            snapshot.settings.cpu = true;
            snapshot.job = Some(Job {
                episode: episode.to_owned(),
                phase: Phase::Transcribing,
                done_ms: 0,
                duration_ms: 0,
                backend: None,
                device: None,
                paused: None,
                title: None,
            });
        });
        service.start_supervisor(episode, &speech, &service.shared, &model);

        let frames = 480usize;
        let mut buffer = vec![0f32; frames * 2];
        let started = Instant::now();
        let window = Duration::from_secs(10);
        let (mut active, mut samples) = (0u32, 0u32);
        while started.elapsed() < window {
            sink.render(&mut buffer);
            samples += 1;
            if service.snapshot().job.is_some() {
                active += 1;
            }
            let due = started
                + Duration::from_millis((started.elapsed().as_millis() as u64 / 10 + 1) * 10);
            let now = Instant::now();
            if due > now {
                std::thread::sleep(due - now);
            }
        }
        let underruns = engine.underruns();
        let done = service.snapshot();
        println!(
            "10 s de lecture, transcription active sur {active}/{samples} échantillons : \
             {underruns} underrun(s), terminée = {}, {} ms transcrites",
            done.last_done.as_deref() == Some(episode),
            done.job.as_ref().map(|job| job.done_ms).unwrap_or(0)
        );
        assert_eq!(underruns, 0, "la transcription a coupé la lecture");
        assert!(
            active > 0,
            "the transcription must have run while the track played: {done:?}"
        );
        engine.shutdown();
        service.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

}
