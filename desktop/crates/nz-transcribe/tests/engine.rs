//! The engine's acceptance, measured — `docs/agents/desktop.md`, lot T2.
//!
//! Every test here does what the person does, through the binary the app runs:
//! `neezer-transcribe probe`, `… engine`, `… file`. The fixtures are named by the
//! environment because they are big (a GGML model is 78 MB to 874 MB) and a
//! developer without one must still be able to run the unit tests:
//!
//! - `NZ_WHISPER_MODEL` — the model file. **Unset** skips the engine tests with a
//!   printed reason; **set to a missing file** FAILS them, so a broken CI cache or
//!   download cannot pass silently.
//! - `NZ_WHISPER_AUDIO` — `jfk.wav` (whisper.cpp's eleven seconds of speech).
//! - `NZ_REQUIRE_VULKAN=1` — assert that a Vulkan device was found. CI sets it on
//!   `lavapipe`; a laptop without a driver leaves it unset and the Vulkan half is
//!   skipped with a message.
//! - `NZ_WHISPER_SPEED=1` — run the throughput measurement (about a minute of
//!   CPU) and assert the lot's floors.
//!
//! The numbers are written next to the assertions and repeated in the PR.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Instant;

use nz_transcribe::audio;
use nz_transcribe::engine::{Backend, Document, Engine, EngineConfig};

/// The binary `cargo` builds for these tests.
const BINARY: &str = env!("CARGO_BIN_EXE_neezer-transcribe");

/// `NZ_WHISPER_MODEL`, or `None` when the test must be skipped.
fn model() -> Option<PathBuf> {
    match std::env::var_os("NZ_WHISPER_MODEL") {
        Some(path) => {
            let path = PathBuf::from(path);
            assert!(
                path.is_file(),
                "NZ_WHISPER_MODEL={} n'existe pas : le modèle n'a pas été téléchargé",
                path.display()
            );
            Some(path)
        }
        None => None,
    }
}

/// `NZ_WHISPER_AUDIO` (jfk.wav), or `None` when the test must be skipped.
fn speech() -> Option<PathBuf> {
    match std::env::var_os("NZ_WHISPER_AUDIO") {
        Some(path) => {
            let path = PathBuf::from(path);
            assert!(
                path.is_file(),
                "NZ_WHISPER_AUDIO={} n'existe pas",
                path.display()
            );
            Some(path)
        }
        None => None,
    }
}

/// A scratch directory of this test's own, under the OS temp dir.
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "nz-engine-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch directory");
    dir
}

/// Run the binary with `args`, optionally with extra environment.
fn run(args: &[&str], environment: &[(&str, &str)]) -> Output {
    let mut command = Command::new(BINARY);
    command.args(args).env("RUST_BACKTRACE", "1");
    for (key, value) in environment {
        command.env(key, value);
    }
    command.output().expect("spawn neezer-transcribe")
}

/// The document a `file`/`engine` run wrote, or a failure naming the run.
fn document(path: &Path, output: &Output) -> Document {
    assert!(
        output.status.success(),
        "neezer-transcribe a échoué ({:?})\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let text = std::fs::read_to_string(path).expect("document");
    Document::from_json(&text).expect("document JSON")
}

/// `file <audio> --model … --out …`, with `--cpu` when asked.
fn transcribe(
    model: &Path,
    speech: &Path,
    out: &Path,
    cpu: bool,
    environment: &[(&str, &str)],
) -> Document {
    let mut args = vec![
        "file",
        speech.to_str().expect("utf-8 path"),
        "--model",
        model.to_str().expect("utf-8 path"),
        "--lang",
        "en",
        "--out",
        out.to_str().expect("utf-8 path"),
    ];
    if cpu {
        args.push("--cpu");
    }
    let output = run(&args, environment);
    document(out, &output)
}

/// What `neezer-transcribe probe` says: the backend, and the cause when it is
/// the CPU.
fn probe() -> (String, Option<String>) {
    let output = run(&["probe"], &[]);
    assert!(output.status.success(), "{:?}", output.status);
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&output.stdout).trim()).expect("JSON");
    (
        value["backend"].as_str().unwrap_or_default().to_owned(),
        value["reason"].as_str().map(str::to_owned),
    )
}

/// Whether a Vulkan loader is installed (the `libvulkan.so.1` ggml `dlopen`s).
///
/// `no_vulkan_loader` and `no_vulkan_device` are different causes and different
/// fixes — "install vulkan-icd-loader" against "install vulkan-radeon" — so the
/// test asserts the one this machine can have.
fn vulkan_loader_installed() -> bool {
    [
        "/usr/lib/libvulkan.so.1",
        "/usr/lib64/libvulkan.so.1",
        "/lib/libvulkan.so.1",
        "/usr/lib/x86_64-linux-gnu/libvulkan.so.1",
        "/usr/lib/aarch64-linux-gnu/libvulkan.so.1",
    ]
    .iter()
    .any(|path| Path::new(path).is_file())
}

/// This machine's load average, so a throughput number says under what load it
/// was taken (the desktop job runs on a shared box).
fn load_average() -> String {
    std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|text| text.split_whitespace().next().map(str::to_owned))
        .unwrap_or_else(|| "?".to_owned())
}

/// The lower-case text of a transcript, to compare two runs by what was said.
fn text(document: &Document) -> String {
    document
        .transcription
        .iter()
        .filter_map(|item| item.get("text").and_then(|text| text.as_str()))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// `probe` prints the control line T3's supervisor copies and T4 shows: one JSON
/// object per run, on stdout, with the two values a machine can have.
///
/// Measured here: `{"event":"engine","backend":"vulkan","device":"AMD Radeon 780M
/// (RADV PHOENIX)","reason":null}` in ~45 ms (release, one child spawn).
#[test]
fn the_probe_speaks_the_control_line() {
    let output = run(&["probe"], &[]);
    assert!(output.status.success(), "{:?}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.matches('\n').count(), 1, "{stdout:?}");
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).expect("JSON");
    assert_eq!(value["event"], "engine");
    let backend = value["backend"].as_str().expect("backend");
    assert!(backend == "vulkan" || backend == "cpu", "{backend}");
    if backend == "cpu" {
        let reason = value["reason"]
            .as_str()
            .expect("a CPU report names its cause");
        assert!(
            [
                "no_vulkan_loader",
                "no_vulkan_device",
                "gpu_init_failed",
                "gpu_out_of_memory",
                "user_forced_cpu"
            ]
            .contains(&reason),
            "{reason}"
        );
        assert!(value["device"].is_null(), "{value}");
    } else {
        assert!(value["device"].is_string(), "{value}");
        assert!(value["reason"].is_null(), "{value}");
    }

    // `probe --cpu` is the person's own choice, and says so.
    let output = run(&["probe", "--cpu"], &[]);
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&output.stdout).trim()).expect("JSON");
    assert_eq!(value["backend"], "cpu");
    assert_eq!(value["reason"], "user_forced_cpu");
}

/// A machine whose Vulkan loader cannot find a driver (`VK_ICD_FILENAMES` points
/// nowhere) is the case that used to ABORT the process (`SIGABRT`, exit 134):
/// the probe must instead report `no_vulkan_device`, and a file must still
/// transcribe on the CPU.
#[test]
fn a_missing_vulkan_driver_falls_back_to_the_cpu_with_its_cause() {
    let Some(model) = model() else {
        eprintln!("NZ_WHISPER_MODEL absent : test ignoré");
        return;
    };
    let Some(speech) = speech() else {
        eprintln!("NZ_WHISPER_AUDIO absent : test ignoré");
        return;
    };
    let broken = [("VK_ICD_FILENAMES", "/nonexistent")];
    let output = run(&["probe"], &broken);
    assert!(
        output.status.success(),
        "le probe a échoué au lieu de nommer la cause : {:?}\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&output.stdout).trim()).expect("JSON");
    assert_eq!(value["backend"], "cpu", "{value}");
    // With a loader installed, a dead ICD is `no_vulkan_device`; a machine that
    // has no loader at all names that instead — never a crash, never "unknown".
    let expected = if vulkan_loader_installed() {
        "no_vulkan_device"
    } else {
        "no_vulkan_loader"
    };
    assert_eq!(value["reason"], expected, "{value}");

    // And the transcript still arrives, on the CPU, with the cause written down.
    let dir = scratch("no-icd");
    let out = dir.join("no-icd.json");
    let document = transcribe(&model, &speech, &out, false, &broken);
    assert_eq!(document.backend, "cpu");
    assert_eq!(document.reason.as_deref(), Some(expected));
    assert!(
        text(&document).contains("ask not what your country"),
        "{}",
        text(&document)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The same eleven seconds through the GPU and through the CPU say the same
/// thing, word for word: the backend is a speed choice, not a different engine.
#[test]
fn cpu_and_vulkan_transcribe_jfk_identically() {
    let Some(model) = model() else {
        eprintln!("NZ_WHISPER_MODEL absent : test ignoré");
        return;
    };
    let Some(speech) = speech() else {
        eprintln!("NZ_WHISPER_AUDIO absent : test ignoré");
        return;
    };
    let (backend, reason) = probe();
    if backend != "vulkan" {
        if std::env::var_os("NZ_REQUIRE_VULKAN").is_some() {
            panic!("aucun GPU Vulkan alors que NZ_REQUIRE_VULKAN est posé : {reason:?}");
        }
        eprintln!("aucun GPU Vulkan ({reason:?}) : moitié Vulkan du test ignorée");
        return;
    }

    let dir = scratch("backends");
    let gpu = transcribe(&model, &speech, &dir.join("gpu.json"), false, &[]);
    let cpu = transcribe(&model, &speech, &dir.join("cpu.json"), true, &[]);
    assert_eq!(gpu.backend, "vulkan", "{gpu:?}");
    assert_eq!(cpu.backend, "cpu", "{cpu:?}");
    assert_eq!(cpu.reason.as_deref(), Some("user_forced_cpu"));

    assert!(!gpu.transcription.is_empty(), "{gpu:?}");
    assert_eq!(
        gpu.transcription,
        cpu.transcription,
        "Vulkan et le processeur doivent dire la même chose\ngpu: {}\ncpu: {}",
        text(&gpu),
        text(&cpu)
    );
    assert!(
        text(&gpu).contains("ask not what your country"),
        "{}",
        text(&gpu)
    );
    // Same document fields save the processor itself.
    assert_eq!(gpu.model, cpu.model);
    assert!(gpu.device.is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

/// `engine` is the chunk contract T3's supervisor runs: raw 16 kHz mono `f32` in,
/// the same lines as `file` out (chunk 0 starts at 0, so the offsets agree).
#[test]
fn the_engine_child_transcribes_the_same_chunk_as_the_file_command() {
    let Some(model) = model() else {
        eprintln!("NZ_WHISPER_MODEL absent : test ignoré");
        return;
    };
    let Some(speech) = speech() else {
        eprintln!("NZ_WHISPER_AUDIO absent : test ignoré");
        return;
    };
    let dir = scratch("engine");
    let samples = audio::decode_file(&speech).expect("decode jfk.wav");
    let pcm = dir.join("chunk.pcm");
    audio::write_pcm(&pcm, &samples).expect("write pcm");
    // The `--pcm` file's duration is a multiplication, not a decode: that is what
    // the supervisor uses to know how far a chunk got.
    assert_eq!(
        audio::pcm_duration_ms(std::fs::metadata(&pcm).expect("metadata").len()),
        audio::samples_duration_ms(samples.len())
    );

    let out = dir.join("chunk.json");
    let output = run(
        &[
            "engine",
            "--pcm",
            pcm.to_str().expect("utf-8"),
            "--model",
            model.to_str().expect("utf-8"),
            "--lang",
            "en",
            "--out",
            out.to_str().expect("utf-8"),
        ],
        &[],
    );
    let chunk = document(&out, &output);
    let whole = transcribe(&model, &speech, &dir.join("file.json"), false, &[]);
    assert!(!chunk.transcription.is_empty());
    // The child writes whisper's RAW lines (leading space and all, as `-oj`
    // does) and `file` writes them normalised: what must agree is what the
    // supervisor sends the server, i.e. `normalize_segments` of both.
    let samples_ms = audio::samples_duration_ms(samples.len()) as i64;
    let from_child = nz_transcribe::normalize_segments(
        &chunk.transcription,
        0,
        samples_ms,
        nz_transcribe::segments::OVERLAP_MS,
    );
    let from_file = nz_transcribe::normalize_segments(
        &whole.transcription,
        0,
        samples_ms,
        nz_transcribe::segments::OVERLAP_MS,
    );
    assert_eq!(from_child, from_file, "{}", text(&chunk));
    assert_eq!(from_child.len(), 2, "{from_child:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// An MP3 and an M4A episode decode to the duration they have, in this process,
/// without `ffmpeg` at runtime: `ffmpeg` only writes the fixtures.
#[test]
fn an_mp3_and_an_m4a_decode_to_the_right_duration() {
    let dir = scratch("formats");
    let seconds = 5.0f64;
    let tone = dir.join("tone.wav");
    let status = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=44100:duration=5",
            "-ac",
            "2",
            "-y",
        ])
        .arg(&tone)
        .status();
    let Ok(status) = status else {
        eprintln!("ffmpeg absent : test ignoré");
        return;
    };
    assert!(status.success(), "ffmpeg n'a pas écrit le ton");

    for (name, codec) in [
        ("episode.mp3", vec!["-c:a", "libmp3lame", "-b:a", "96k"]),
        (
            "episode.m4a",
            vec!["-c:a", "aac", "-b:a", "96k", "-movflags", "+faststart"],
        ),
        ("episode.ogg", vec!["-c:a", "libopus", "-b:a", "64k"]),
        ("episode.flac", vec!["-c:a", "flac"]),
    ] {
        let path = dir.join(name);
        let mut command = Command::new("ffmpeg");
        command.args(["-nostdin", "-v", "error", "-i"]).arg(&tone);
        command.args(&codec).arg("-y").arg(&path);
        let status = command.status().expect("spawn ffmpeg");
        assert!(status.success(), "ffmpeg n'a pas écrit {name}");

        let samples = audio::decode_file(&path).expect("decode");
        let duration_ms = audio::samples_duration_ms(samples.len()) as f64;
        let expected_ms = seconds * 1000.0;
        // The lot's tolerance: ±50 ms. Measured: MP3 5025 ms (its encoder delay
        // is 1152 samples at 44.1 kHz), M4A 5000, Ogg Opus 5000, FLAC 5000.
        assert!(
            (duration_ms - expected_ms).abs() <= 50.0,
            "{name} : {duration_ms:.0} ms au lieu de {expected_ms:.0} ms"
        );
        // 16 kHz mono, and nothing padded at the head: the resampler's own delay
        // is dropped, so the first 50 ms carry the tone.
        assert!(samples.iter().take(800).any(|sample| sample.abs() > 0.01));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file that is not a model is refused before whisper.cpp sees it, with the
/// server's code — and the command fails, so a caller knows.
#[test]
fn a_file_that_is_not_a_model_is_refused() {
    let dir = scratch("bad-model");
    let broken = dir.join("ggml-broken.bin");
    std::fs::write(&broken, [0u8; 4096]).expect("write");
    let output = run(
        &[
            "file",
            "nope.mp3",
            "--model",
            broken.to_str().expect("utf-8"),
            "--out",
            dir.join("out.json").to_str().expect("utf-8"),
        ],
        &[],
    );
    assert_eq!(output.status.code(), Some(1), "{:?}", output.status);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("modèle trop petit"), "{stderr}");
    assert!(!dir.join("out.json").exists(), "aucun document écrit");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The lot's throughput floors, on this machine, once the engine is warm:
/// `ggml-base` ≥ 40× real time on Vulkan and ≥ 12× on the CPU.
///
/// Warm is what a 600 s chunk sees: the model is loaded and the first GPU call
/// (which compiles whisper.cpp's shaders) has happened, so the measurement runs
/// the same 330 s of speech twice and times the second pass. `NZ_WHISPER_SPEED=1`
/// because the CPU pass costs ~20 s.
///
/// Measured 2026-11-16 (Rust 1.99.1, RADV 780M, ggml-base, 330 s of speech):
/// Vulkan **45.6×** (7.2 s), CPU **14.1×** (23.4 s) — the prototype's 44.9× and
/// 13.9×, reproduced.
#[test]
fn the_engine_reaches_the_lots_speed_floors() {
    if std::env::var_os("NZ_WHISPER_SPEED").is_none() {
        eprintln!("NZ_WHISPER_SPEED absent : mesure de vitesse ignorée");
        return;
    }
    let Some(model) = model() else {
        eprintln!("NZ_WHISPER_MODEL absent : test ignoré");
        return;
    };
    let Some(speech) = speech() else {
        eprintln!("NZ_WHISPER_AUDIO absent : test ignoré");
        return;
    };
    let once = audio::decode_file(&speech).expect("decode jfk.wav");
    let mut long = Vec::with_capacity(once.len() * 30);
    for _ in 0..30 {
        long.extend_from_slice(&once);
    }
    let audio_s = long.len() as f64 / 16_000.0;

    for (cpu, floor) in [(false, 40.0), (true, 12.0)] {
        let mut config = EngineConfig::new(&model);
        config.cpu = cpu;
        // No VAD: the lot's floors are the prototype's, measured through
        // whisper-cli without `--vad`; VAD changes what the model sees, and its
        // own cost would be folded into the ratio.
        config.vad = None;
        let mut engine = Engine::load(&config).expect("engine");
        if !cpu && engine.report().backend != Backend::Vulkan {
            if std::env::var_os("NZ_REQUIRE_VULKAN").is_some() {
                panic!("aucun GPU Vulkan : {:?}", engine.report());
            }
            eprintln!("aucun GPU Vulkan : mesure Vulkan ignorée");
            continue;
        }
        // The first pass pays the model's own caches; the timed ones are the
        // number, and the best of two is reported because this machine is shared
        // (a `cargo build` in another session halves the throughput: load
        // average is printed next to the result).
        engine
            .transcribe(&long, Some("en"))
            .expect("first transcription");
        let mut best = f64::MAX;
        let mut transcript = None;
        for _ in 0..2 {
            let started = Instant::now();
            transcript = Some(engine.transcribe(&long, Some("en")).expect("transcription"));
            best = best.min(started.elapsed().as_secs_f64());
        }
        let transcript = transcript.expect("a transcription ran");
        let ratio = audio_s / best;
        eprintln!(
            "{} : {:.1}× temps réel ({:.2} s pour {:.0} s d'audio, {} ligne(s), charge {})",
            engine.report().backend.as_str(),
            ratio,
            best,
            audio_s,
            transcript.items.len(),
            load_average()
        );
        assert!(!transcript.items.is_empty());
        assert!(
            ratio >= floor,
            "{} : {ratio:.1}× < {floor}× temps réel ({best:.2} s, charge {})",
            engine.report().backend.as_str(),
            load_average()
        );
    }
}
