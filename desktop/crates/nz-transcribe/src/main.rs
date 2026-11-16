//! `neezer-transcribe`: the standalone half of local transcription.
//!
//! The app only ever starts this binary; it can run on its own on a machine
//! with no Neezer window. D0 shipped the one subcommand CI needs to prove the
//! whisper.cpp build it links, Vulkan included; T2 adds the engine:
//!
//! ```text
//! neezer-transcribe probe [--cpu]
//!     Which processor the engine would use, and why — the `{"event":"engine"}`
//!     line on stdout. Safe everywhere: the Vulkan enumeration happens in a
//!     child (see `engine.rs`), so a machine with no usable driver prints the
//!     cause instead of aborting.
//!
//! neezer-transcribe engine --pcm <raw f32le 16k mono> --model <ggml-*.bin>
//!                           [--lang auto] [--vad <path>] [--no-vad] [--cpu]
//!                           [--threads N] [--device N] --out <x.json>
//!     One chunk, the way T3's supervisor runs it: raw PCM in, the whisper
//!     document out (see `engine::Document`). No decoding, no chunking, no
//!     server: this is the child a `SIGSTOP` pauses.
//!
//! neezer-transcribe file <audio> [--lang auto] [--model <path>] [--cpu]
//!                        [--vad <path>] [--no-vad] [--threads N] --out <x.json>
//!     A whole audio file (MP3, M4A, FLAC, Ogg Opus, WAV), decoded in-process,
//!     chunked like the server chunks it, and written as one document with
//!     absolute offsets. This is what the benches and the acceptance tests run.
//! ```
//!
//! T3 adds the two the app runs:
//!
//! ```text
//! neezer-transcribe worker --url <http(s)://hôte[:port]> --model <ggml.bin>
//!                          [--token-file <fichier>] [--lang auto|xx] [--once] [--cpu]
//!     The enrolled `client` worker: claim, heartbeat, chunk, complete, and the
//!     JSON-lines control channel the app speaks on stdin/stdout.
//!
//! neezer-transcribe local --model <ggml.bin> [--episode <uuid> --audio <fichier>]
//!     One episode on this PC, no server; without `--audio`, one job per
//!     `{"cmd":"local",…}` line on stdin.
//! ```
//!
//! `--version` and `--help` never touch a GPU on purpose (see `lib.rs`): a
//! `--version` that aborts on a machine without a Vulkan driver would be the
//! first thing a new person meets.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use nz_transcribe::audio;
use nz_transcribe::engine::{self, Document, EngineConfig, VULKAN_DEVICES_FLAG};
use nz_transcribe::models;
use nz_transcribe::segments::{CHUNK_SECONDS, OVERLAP_MS};
use nz_transcribe::supervisor;

const USAGE: &str = "\
neezer-transcribe — transcription locale des épisodes, sur le GPU via Vulkan.

USAGE:
    neezer-transcribe --version
    neezer-transcribe --help
    neezer-transcribe probe [--cpu]
    neezer-transcribe engine --pcm <fichier> --model <modèle> --out <x.json>
                             [--lang auto] [--vad <modèle>] [--no-vad] [--cpu]
                             [--threads N] [--device N]
    neezer-transcribe file <audio> --out <x.json> [--model <modèle>]
                          [--lang auto] [--vad <modèle>] [--no-vad] [--cpu]
                          [--threads N] [--device N]
    neezer-transcribe worker --url <URL> --model <ggml.bin> [options]
    neezer-transcribe local --model <ggml.bin> [--episode <uuid> --audio <fichier>]
";

/// A failure to report to the person: the message, and whether it is a usage
/// error (2) or a failed run (1).
enum Failure {
    Usage(String),
    Run(String),
}

impl Failure {
    fn exit_code(&self) -> ExitCode {
        match self {
            Failure::Usage(_) => ExitCode::from(2),
            Failure::Run(_) => ExitCode::from(1),
        }
    }

    fn message(&self) -> &str {
        match self {
            Failure::Usage(message) | Failure::Run(message) => message,
        }
    }
}

/// `--version`: what this binary is, which whisper.cpp it links and what that
/// build was compiled with. No GPU is touched here on purpose (see the crate
/// doc): a `--version` that aborts on a machine without a Vulkan driver would
/// be the first thing a new user meets.
fn version() -> String {
    format!(
        "neezer-transcribe {}\nwhisper.cpp {} (backend vulkan compilé)\n{}",
        env!("CARGO_PKG_VERSION"),
        nz_transcribe::WHISPER_VERSION,
        nz_transcribe::system_info(),
    )
}

/// The arguments this parser understands: `--key value`, `--key=value`, and
/// switches (`--cpu`, `--no-vad`), plus positional paths.
struct Arguments {
    values: Vec<(String, String)>,
    switches: Vec<String>,
    positional: Vec<String>,
}

impl Arguments {
    fn parse(args: &[String]) -> Result<Self, Failure> {
        let mut parsed = Self {
            values: Vec::new(),
            switches: Vec::new(),
            positional: Vec::new(),
        };
        let mut index = 0;
        while index < args.len() {
            let argument = &args[index];
            if let Some(rest) = argument.strip_prefix("--") {
                if let Some((key, value)) = rest.split_once('=') {
                    parsed.values.push((key.to_owned(), value.to_owned()));
                } else {
                    let next = args.get(index + 1);
                    // A switch takes no value; a known-value option takes the
                    // next argument. Recognising the switches here is what makes
                    // `--cpu --model x` parse instead of eating `--model`.
                    if is_switch(rest)
                        || next.is_none()
                        || next.is_some_and(|v| v.starts_with("--"))
                    {
                        parsed.switches.push(rest.to_owned());
                    } else {
                        parsed
                            .values
                            .push((rest.to_owned(), next.cloned().unwrap_or_default()));
                        index += 1;
                    }
                }
            } else {
                parsed.positional.push(argument.clone());
            }
            index += 1;
        }
        Ok(parsed)
    }

    fn value(&self, key: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    fn required(&self, key: &str, usage: &str) -> Result<&str, Failure> {
        self.value(key)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| Failure::Usage(format!("--{key} est obligatoire\n\n{usage}")))
    }

    fn switch(&self, key: &str) -> bool {
        self.switches.iter().any(|name| name == key)
    }

    /// Reject a flag this subcommand does not know, so a typo is an error and
    /// not a silently ignored option. `positional` says whether a bare path is
    /// expected.
    fn only(
        &self,
        keys: &[&str],
        switches: &[&str],
        positional: bool,
        usage: &str,
    ) -> Result<(), Failure> {
        for (name, _) in &self.values {
            if !keys.contains(&name.as_str()) {
                return Err(Failure::Usage(format!(
                    "--{name} n'est pas une option de cette commande\n\n{usage}"
                )));
            }
        }
        for name in &self.switches {
            if !switches.contains(&name.as_str()) {
                return Err(Failure::Usage(format!(
                    "--{name} n'est pas une option de cette commande\n\n{usage}"
                )));
            }
        }
        if !self.positional.is_empty() && !positional {
            return Err(Failure::Usage(format!(
                "argument inattendu : {}\n\n{usage}",
                self.positional.join(" ")
            )));
        }
        Ok(())
    }
}

/// `--cpu`, `--no-vad`: options that never take a value. (`-h`/`--help` are
/// handled before any subcommand parses its arguments.)
fn is_switch(name: &str) -> bool {
    matches!(name, "cpu" | "no-vad")
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args
        .iter()
        .any(|argument| argument == "-h" || argument == "--help")
    {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let outcome = match args.first().map(String::as_str) {
        Some("--version" | "-V") | None => {
            println!("{}", version());
            Ok(())
        }
        // T3's two subcommands: the supervisor owns its own arguments (it has
        // more than a screenful of them) and its own exit codes.
        Some("worker") => {
            return supervisor::worker_main(args[1..].iter().map(Into::into).collect());
        }
        Some("local") => {
            return supervisor::local_main(args[1..].iter().map(Into::into).collect());
        }
        Some(VULKAN_DEVICES_FLAG) => vulkan_devices(),
        Some("probe") => probe(&args[1..]),
        Some("engine") => engine_command(&args[1..]),
        Some("file") => file_command(&args[1..]),
        Some(other) => Err(Failure::Usage(format!(
            "argument inconnu : {other}\n\n{USAGE}"
        ))),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            eprintln!("neezer-transcribe : {}", failure.message());
            failure.exit_code()
        }
    }
}

/// `--vulkan-devices`: the enumeration, in a child (module doc). On a machine
/// with no usable driver this process dies; the parent turns that into a named
/// cause. Printing nothing but the JSON keeps it pipeable.
fn vulkan_devices() -> Result<(), Failure> {
    let devices = engine::enumerate_devices();
    let value = serde_json::json!({ "devices": devices });
    match serde_json::to_string(&value) {
        Ok(text) => {
            println!("{text}");
            Ok(())
        }
        Err(error) => Err(Failure::Run(format!("JSON : {error}"))),
    }
}

/// `probe`: which processor the engine would use, and why.
fn probe(args: &[String]) -> Result<(), Failure> {
    let parsed = Arguments::parse(args)?;
    parsed.only(&[], &["cpu"], false, USAGE)?;
    let report = engine::Availability::detect().report(parsed.switch("cpu"));
    print!("{}", report.event_json());
    Ok(())
}

/// `engine`: one chunk of raw PCM in, the whisper document out — the child T3's
/// supervisor spawns, pauses and reads.
fn engine_command(args: &[String]) -> Result<(), Failure> {
    let parsed = Arguments::parse(args)?;
    parsed.only(
        &["pcm", "model", "out", "lang", "vad", "threads", "device"],
        &["cpu", "no-vad"],
        false,
        USAGE,
    )?;
    let pcm_path = PathBuf::from(parsed.required("pcm", USAGE)?);
    let out = PathBuf::from(parsed.required("out", USAGE)?);
    let config = config_from(&parsed, Some(parsed.required("model", USAGE)?))?;
    // The model is the standing configuration: refuse it before reading the
    // chunk (rule 3, fail fast on the cause the person can fix).
    models::validate(&config.model).map_err(|error| Failure::Run(error.to_string()))?;

    let samples = audio::read_pcm(&pcm_path).map_err(|error| Failure::Run(error.to_string()))?;
    if samples.is_empty() {
        return Err(Failure::Run(nz_transcribe::Error::EmptyAudio.to_string()));
    }
    let mut runner =
        engine::Engine::load(&config).map_err(|error| Failure::Run(error.to_string()))?;
    let language = language_from(&parsed);
    let transcript = runner
        .transcribe(&samples, language)
        .map_err(|error| Failure::Run(error.to_string()))?;
    let document = Document::from_transcript(transcript, runner.report(), runner.model());
    write_document(&out, &document)
}

/// `file`: a whole audio file, decoded and chunked like the server chunks it.
fn file_command(args: &[String]) -> Result<(), Failure> {
    let parsed = Arguments::parse(args)?;
    parsed.only(
        &["model", "out", "lang", "vad", "threads", "device"],
        &["cpu", "no-vad"],
        true,
        USAGE,
    )?;
    let input = parsed
        .positional
        .first()
        .ok_or_else(|| Failure::Usage(format!("un fichier audio est requis\n\n{USAGE}")))?;
    let out = PathBuf::from(parsed.required("out", USAGE)?);
    let config = config_from(&parsed, parsed.value("model"))?;
    models::validate(&config.model).map_err(|error| Failure::Run(error.to_string()))?;

    let samples =
        audio::decode_file(Path::new(input)).map_err(|error| Failure::Run(error.to_string()))?;
    if samples.is_empty() {
        return Err(Failure::Run(nz_transcribe::Error::EmptyAudio.to_string()));
    }
    let mut runner =
        engine::Engine::load(&config).map_err(|error| Failure::Run(error.to_string()))?;
    let language = language_from(&parsed);
    let chunk_ms = CHUNK_SECONDS * 1000;
    let total_ms = audio::samples_duration_ms(samples.len());
    let transcript = engine::transcribe_file(
        &mut runner,
        &samples,
        language,
        chunk_ms,
        OVERLAP_MS,
        |done_ms| {
            // Progress on stderr: stdout stays free for the document.
            eprintln!("transcription : {done_ms} / {total_ms} ms");
        },
    )
    .map_err(|error| Failure::Run(error.to_string()))?;
    let document = Document::from_transcript(transcript, runner.report(), runner.model());
    write_document(&out, &document)
}

/// The engine configuration the two commands share: the model (`--model`, or the
/// best one on disk for `file`), the VAD model, the CPU switch and the threads.
fn config_from(parsed: &Arguments, model: Option<&str>) -> Result<EngineConfig, Failure> {
    let model = match model {
        Some(path) => PathBuf::from(path),
        None => default_model()?,
    };
    let mut config = EngineConfig::new(&model);
    config.cpu = parsed.switch("cpu");
    if parsed.switch("no-vad") {
        config.vad = None;
    } else if let Some(path) = parsed.value("vad") {
        config.vad = Some(PathBuf::from(path));
    }
    if let Some(threads) = parsed.value("threads") {
        config.threads = Some(
            threads
                .parse::<i32>()
                .map_err(|_| Failure::Usage(format!("--threads : {threads}")))?,
        );
    }
    if let Some(device) = parsed.value("device") {
        config.device = Some(
            device
                .parse::<i32>()
                .map_err(|_| Failure::Usage(format!("--device : {device}")))?,
        );
    }
    Ok(config)
}

/// The model `file` uses when nobody names one: the best already on disk in the
/// app's model folder, by the server's ladder.
fn default_model() -> Result<PathBuf, Failure> {
    let directory = models::default_dir();
    let configured = directory.join(models::BASE);
    models::choose(&configured, models::free_memory_bytes()).ok_or_else(|| {
        Failure::Run(format!(
            "{} : aucun modèle installé (Réglages → Transcription locale)",
            directory.display()
        ))
    })
}

/// `--lang`: `auto` (or absent) asks whisper; anything else is a code it knows.
fn language_from(parsed: &Arguments) -> Option<&str> {
    parsed
        .value("lang")
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("auto"))
}

/// Write the document where the caller asked, atomically: a killed child leaves
/// the previous file intact instead of half a JSON.
fn write_document(path: &Path, document: &Document) -> Result<(), Failure> {
    let temporary = match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) if !extension.is_empty() => path.with_extension(format!("{extension}.tmp")),
        _ => path.with_extension("tmp"),
    };
    std::fs::write(&temporary, document.to_json())
        .map_err(|error| Failure::Run(format!("{} : {error}", temporary.display())))?;
    std::fs::rename(&temporary, path)
        .map_err(|error| Failure::Run(format!("{} : {error}", path.display())))
}
