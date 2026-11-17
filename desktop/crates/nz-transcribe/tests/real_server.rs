//! The worker protocol against the REAL server (`tools/perf_api.py`).
//!
//! Gated on `NZ_PERF_API=1` because the bench needs the Python server installed
//! (`pip install -e .`), and the desktop workspace's plain `cargo test` runs
//! before that in CI. Run it with the server's own venv:
//!
//! ```sh
//! cd desktop
//! NZ_PERF_API=1 NZ_PYTHON=../.venv/bin/python \
//!   cargo test -p nz-transcribe --test real_server -- --nocapture
//! ```
//!
//! What it proves, and what the fake server in `worker_protocol.rs` cannot: the
//! routes really are `/api/transcribe/worker/…`, the claim really carries
//! `chunk_ms`/`overlap_ms`/`start_ms`, `job.py` really accepts these chunk
//! payloads (its own segment bounds, its own `done_ms` advance), and the
//! transcript really comes out `ready` in the database with the lines this
//! worker sent. The decode and the engine are the test's doubles: this lot is
//! the protocol and the supervisor, and the bench's episodes are one-second WAV
//! stand-ins for an hour of speech.

#![cfg(all(feature = "engine", feature = "worker"))]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use nz_transcribe::supervisor::{Engine, Outcome, PcmSource, ReplaySink, Supervisor};
use nz_transcribe::worker::{Client, Token};
use serde_json::Value;

const EPISODE_INDEX: usize = 5;

/// A bench server, stopped when the test ends.
struct Bench {
    child: Child,
    base: String,
    episodes: Vec<String>,
}

impl Drop for Bench {
    fn drop(&mut self) {
        stop(&mut self.child);
    }
}

/// Stop the bench the way it expects: SIGTERM first, SIGKILL only as a last
/// resort.
///
/// `tools/perf_api.py` removes its throwaway library when it is asked to stop,
/// and `/tmp` is a tmpfs on most machines: `Child::kill()` is SIGKILL, which
/// cannot be caught and left the directory to the next launch's sweep.
fn stop(child: &mut Child) {
    const SIGTERM: i32 = 15;
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    // SAFETY: `kill` is handed the pid of a child this struct owns and the
    // platform's own SIGTERM number, and nothing else.
    unsafe {
        kill(child.id() as i32, SIGTERM);
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("the repository root")
}

/// Start `perf_api.py --podcasts` and wait for its `READY <port> <ids>` line.
fn start_bench() -> Bench {
    let python = std::env::var("NZ_PYTHON").unwrap_or_else(|_| "python3".to_owned());
    let root = repo_root();
    let mut child = Command::new(&python)
        .arg("tools/perf_api.py")
        .args(["--tracks", "20", "--playlists", "2", "--favorites", "5"])
        .args([
            "--big-playlist",
            "10",
            "--podcasts",
            "--serve",
            "--port",
            "0",
        ])
        .current_dir(&root)
        .env("PYTHONPATH", &root)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap_or_else(|error| panic!("{python} tools/perf_api.py : {error}"));
    let stdout = child.stdout.take().expect("piped stdout");
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if line.starts_with("READY ") && sender.send(line).is_err() {
                return;
            }
        }
    });
    let ready = receiver
        .recv_timeout(Duration::from_secs(180))
        .expect("the bench starts and prints READY");
    let mut parts = ready.splitn(3, ' ');
    assert_eq!(parts.next(), Some("READY"));
    let port = parts.next().expect("a port");
    let ids: Value = serde_json::from_str(parts.next().expect("the ids")).expect("ids json");
    let episodes = ids["podcast"]["episodes"]
        .as_array()
        .expect("the seeded episodes")
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    Bench {
        child,
        base: format!("http://127.0.0.1:{port}"),
        episodes,
    }
}

/// A logged-in client (the cookie jar is what `POST /api/login` sets).
fn browser() -> reqwest::Client {
    reqwest::Client::builder()
        .cookie_store(true)
        .build()
        .expect("client")
}

async fn login(browser: &reqwest::Client, base: &str) {
    let response = browser
        .post(format!("{base}/api/login"))
        .json(&serde_json::json!({"username": "bench", "password": "Bench1"}))
        .send()
        .await
        .expect("login");
    assert_eq!(response.status(), 200, "the bench's admin logs in");
}

/// The fake decoder: silence, at 64 bytes per millisecond, for the window asked.
struct Silence;

impl PcmSource for Silence {
    fn write_pcm(
        &self,
        _audio: &Path,
        start_ms: u64,
        length_ms: u64,
        out: &Path,
    ) -> Result<u64, nz_transcribe::Error> {
        let _ = start_ms;
        std::fs::write(out, vec![0u8; (length_ms * 64) as usize])
            .map_err(|_| nz_transcribe::Error::NoAudio)?;
        Ok(length_ms)
    }
}

/// The fake engine: whisper.cpp's document shape, two lines per chunk.
fn fake_engine(directory: &Path) -> PathBuf {
    let path = directory.join("fake-engine");
    std::fs::write(
        &path,
        r#"#!/bin/sh
pcm=""; out=""
while [ $# -gt 0 ]; do
  case "$1" in
    --pcm) pcm=$2; shift 2;;
    --out) out=$2; shift 2;;
    *) shift;;
  esac
done
ms=$(( $(wc -c < "$pcm") / 64 ))
half=$((ms / 2))
printf '{"transcription":[{"offsets":{"from":0,"to":%d},"text":"début du chunk"},{"offsets":{"from":%d,"to":%d},"text":"fin du chunk"}],"result":{"language":"fr"},"backend":"cpu","reason":"user_forced_cpu"}' \
  "$half" "$half" "$((ms - 1))" > "$out"
"#,
    )
    .expect("script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    path
}

/// Claim → audio → chunks → complete on the real server, and the transcript row
/// the person reads.
#[tokio::test]
async fn the_real_server_stores_a_client_workers_transcript() {
    if std::env::var_os("NZ_PERF_API").is_none() {
        eprintln!("NZ_PERF_API absent : le vrai serveur n'est pas lancé ici (voir l'en-tête)");
        return;
    }
    let bench = start_bench();
    assert!(bench.episodes.len() > EPISODE_INDEX, "{:?}", bench.episodes);
    let episode = bench.episodes[EPISODE_INDEX].clone();
    let browser = browser();
    login(&browser, &bench.base).await;

    // The admin enrols this machine as a `client` worker and reads the token
    // once, exactly as Réglages → Transcription locale will (T4).
    let created: Value = browser
        .post(format!("{}/api/transcribe/workers", bench.base))
        .json(&serde_json::json!({"name": "banc-t3", "kind": "client"}))
        .send()
        .await
        .expect("enrol")
        .json()
        .await
        .expect("enrol json");
    let token = created["token"]
        .as_str()
        .expect("the token, shown once")
        .to_owned();
    assert_eq!(created["worker"]["kind"], "client");

    // Queue the episode (the bench leaves episode 6 without a transcript row).
    let queued = browser
        .post(format!(
            "{}/api/podcast/episode/{episode}/transcribe",
            bench.base
        ))
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("enqueue");
    assert_eq!(
        queued.status(),
        202,
        "{}",
        queued.text().await.unwrap_or_default()
    );

    // The worker, with the test's decoder and engine.
    let directory = std::env::temp_dir().join(format!("nz-t3-real-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("scratch");
    let sink = Arc::new(ReplaySink::default());
    let mut supervisor = Supervisor::new(
        Engine {
            program: fake_engine(&directory),
            model: directory.join("ggml-small.bin"),
            cpu: true,
        },
        Arc::new(Silence),
        sink.clone(),
        directory.join("work"),
        directory.join("transcripts"),
    );
    supervisor.client = Some(
        Client::new(&bench.base, Token::new(token).expect("the server's token")).expect("client"),
    );

    let started = Instant::now();
    let claim = supervisor
        .client
        .as_ref()
        .expect("client")
        .claim()
        .await
        .expect("claim")
        .expect("the queued episode");
    assert_eq!(claim.episode_id, episode);
    assert_eq!(
        claim.duration_ms, 3_600_000,
        "the bench's hour-long episode"
    );
    assert_eq!(claim.chunk_ms(), 600_000);
    let outcome = supervisor.run_claim(claim).await;
    let elapsed = started.elapsed();
    assert_eq!(outcome, Outcome::Done, "{:?}", sink.tags());

    // What the server now holds: `ready`, from this worker, with the lines.
    let transcript: Value = browser
        .get(format!(
            "{}/api/podcast/episode/{episode}/transcript?limit=200",
            bench.base
        ))
        .send()
        .await
        .expect("transcript")
        .json()
        .await
        .expect("transcript json");
    assert_eq!(transcript["status"], "ready");
    assert_eq!(transcript["source"], "client");
    assert_eq!(transcript["worker"], "banc-t3");
    assert_eq!(transcript["done_ms"], 3_600_000);
    assert_eq!(transcript["engine"], "whisper.cpp");
    assert_eq!(transcript["model"], "ggml-small.bin");
    let lines = transcript["segments"].as_array().expect("lines");
    assert_eq!(lines.len(), 12, "two lines per chunk, six chunks");
    assert!(
        lines
            .iter()
            .all(|line| line["text"].as_str().is_some_and(|text| !text.is_empty())),
        "{transcript}"
    );
    eprintln!(
        "real server: 6 chunks, 12 lines, ready in {:.1} s",
        elapsed.as_secs_f64()
    );
    let _ = std::fs::remove_dir_all(&directory);
}
