//! The worker protocol and the supervisor, against a server.
//!
//! The fake server below speaks the routes, the payloads and the decisions of
//! `neezer/webui/transcribe_worker.py` + `neezer/transcribe/job.py` for one
//! episode: a claim, its audio with `Range`, heartbeats that can cancel, chunks
//! that must be the next one, and a completion that refuses an incomplete
//! transcript. It is a test double, not a second implementation: every value it
//! answers is written where the real server writes it, so the acceptance
//! criteria of lot T3 (claim → audio → chunks → complete, cancel within one
//! heartbeat, a lease kept through a `SIGSTOP`, a bounded backoff) are measured
//! on the client's real code paths.
//!
//! The tests that need the REAL server are gated on `NZ_PERF_API=1` and start
//! `tools/perf_api.py --podcasts` (see `real_server.rs`'s header for the
//! command); this file is what runs on every `cargo test --workspace`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use nz_transcribe::control::Command;
use nz_transcribe::supervisor::{
    Engine, Outcome, PcmSource, ReplaySink, Supervisor, parse_engine_json,
};
use nz_transcribe::worker::{Client, Failure, Retry, Token};
use serde_json::{Value, json};

const EPISODE: &str = "b7b1c0de-0000-4000-8000-000000000000";

// ---------------------------------------------------------------------------
// A server that speaks the worker protocol for one episode
// ---------------------------------------------------------------------------

/// What the fake server remembers and what it is told to do.
#[derive(Debug)]
struct State {
    episode: String,
    lease: String,
    duration_ms: i64,
    chunk_ms: i64,
    audio: Vec<u8>,
    /// Answer the next N claims with `429` + `Retry-After: 1`.
    rate_limit_claims: u32,
    /// Answer the next N claims with `500`.
    fail_claims: u32,
    /// Cancel the job once this many heartbeats arrived.
    cancel_at_beat: Option<u32>,
    /// Answer the first `complete` with `409 incomplete` at this position (the
    /// server ahead of the worker), then the truth.
    incomplete_at: Option<i64>,
    /// The server's transcript.
    status: String,
    done_ms: i64,
    segments: usize,
    // Recorded.
    requests: Vec<(String, String)>,
    ranges: Vec<String>,
    beats: Vec<Instant>,
    chunks: Vec<Value>,
    failed: Vec<Value>,
    released: bool,
    completed: bool,
}

impl State {
    fn new() -> State {
        State {
            episode: EPISODE.to_owned(),
            lease: "lease-token".to_owned(),
            duration_ms: 1_800_000,
            chunk_ms: 600_000,
            audio: b"1800000\n".to_vec(),
            rate_limit_claims: 0,
            fail_claims: 0,
            cancel_at_beat: None,
            incomplete_at: None,
            status: "queued".to_owned(),
            done_ms: 0,
            segments: 0,
            requests: Vec::new(),
            ranges: Vec::new(),
            beats: Vec::new(),
            chunks: Vec::new(),
            failed: Vec::new(),
            released: false,
            completed: false,
        }
    }

    fn count(&self, needle: &str) -> usize {
        self.requests
            .iter()
            .filter(|(_, path)| path.contains(needle))
            .count()
    }
}

/// The fake server: one thread accepting, one per connection.
struct Fake {
    base: String,
    state: Arc<Mutex<State>>,
}

impl Fake {
    fn start(state: Arc<Mutex<State>>) -> Fake {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        let shared = state.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let state = shared.clone();
                thread::spawn(move || {
                    let mut stream = stream;
                    let _ = serve(&mut stream, &state);
                });
            }
        });
        Fake { base, state }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("state lock")
    }

    fn requests(&self) -> Vec<(String, String)> {
        self.state().requests.clone()
    }

    fn paths(&self) -> Vec<String> {
        self.requests().into_iter().map(|(_, path)| path).collect()
    }
}

/// One HTTP/1.1 request, answered and closed.
fn serve(stream: &mut TcpStream, state: &Mutex<State>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(at) = find(&buffer, b"\r\n\r\n") {
            break at + 4;
        }
        if buffer.len() > 64 * 1024 {
            return Ok(());
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let mut headers: HashMap<String, String> = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_lowercase(), value.trim().to_owned());
        }
    }
    let length: usize = headers
        .get("content-length")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let mut body = buffer[header_end..].to_vec();
    while body.len() < length {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    let body = &body[..body.len().min(length)];
    let mut state = state.lock().expect("state lock");
    state.requests.push((method.clone(), path.clone()));
    answer(stream, &method, &path, &headers, body, &mut state)
}

fn answer(
    stream: &mut TcpStream,
    method: &str,
    path: &str,
    headers: &HashMap<String, String>,
    body: &[u8],
    state: &mut State,
) -> std::io::Result<()> {
    // A device that does not present the lease gets nothing.
    let bearer = headers
        .get("authorization")
        .map(String::as_str)
        .unwrap_or_default();
    if bearer != "Bearer device-token" {
        return respond(
            stream,
            401,
            json!({"code": "unauthorized", "error": "jeton"}),
            &[],
        );
    }
    let leased = headers
        .get("x-ns-lease")
        .map(String::as_str)
        .unwrap_or_default();
    if method == "POST" && path == "/api/transcribe/worker/claim" {
        if state.rate_limit_claims > 0 {
            state.rate_limit_claims -= 1;
            return respond_with(
                stream,
                429,
                json!({"code": "rate_limited", "error": "trop de demandes"}),
                &[("Retry-After", "1")],
            );
        }
        if state.fail_claims > 0 {
            state.fail_claims -= 1;
            return respond(
                stream,
                500,
                json!({"code": "error", "error": "serveur"}),
                &[],
            );
        }
        if state.status != "queued" {
            return respond_raw(stream, "204 No Content", &[], &[]);
        }
        state.status = "leased".to_owned();
        return respond(
            stream,
            200,
            json!({
                "episode_id": state.episode,
                "lease": state.lease,
                "audio_url": format!("/api/transcribe/worker/audio/{}", state.episode),
                "duration_ms": state.duration_ms,
                "language": "fr",
                "chunk_ms": state.chunk_ms,
                "overlap_ms": 2_000,
                "max_minutes": 360,
                "start_ms": state.done_ms,
                "expires_at": "2026-11-16T16:00:00",
            }),
            &[],
        );
    }
    if method == "GET" && path == format!("/api/transcribe/worker/audio/{}", state.episode) {
        if leased != state.lease {
            return respond(
                stream,
                403,
                json!({"code": "lease_expired", "error": "aucun lease actif"}),
                &[],
            );
        }
        state
            .ranges
            .push(headers.get("range").cloned().unwrap_or_default());
        let total = state.audio.len() as u64;
        let from = headers
            .get("range")
            .and_then(|value| value.strip_prefix("bytes="))
            .and_then(|value| value.split('-').next())
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        if from >= total {
            return respond_with(
                stream,
                416,
                json!({"error": "range"}),
                &[("Content-Range", &format!("bytes */{total}"))],
            );
        }
        let slice = &state.audio[from as usize..];
        if from > 0 {
            return respond_bytes(
                stream,
                206,
                slice,
                &[
                    (
                        "Content-Range",
                        &format!("bytes {from}-{}/{total}", total - 1),
                    ),
                    ("Content-Length", &slice.len().to_string()),
                ],
            );
        }
        return respond_bytes(stream, 200, slice, &[("Accept-Ranges", "bytes")]);
    }
    if let Some(rest) = path.strip_prefix(&format!("/api/transcribe/worker/{}", state.episode)) {
        if leased != state.lease {
            return respond(
                stream,
                403,
                json!({"code": "lease_required", "error": "lease requis"}),
                &[],
            );
        }
        match (method, rest) {
            ("POST", "/heartbeat") => {
                state.beats.push(Instant::now());
                let cancel = state
                    .cancel_at_beat
                    .is_some_and(|at| state.beats.len() as u32 >= at);
                return respond(
                    stream,
                    200,
                    json!({"cancel": cancel, "pause": false, "done_ms": state.done_ms}),
                    &[],
                );
            }
            ("POST", "/chunk") => {
                let value: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
                let start = value.get("start_ms").and_then(Value::as_i64).unwrap_or(-1);
                if start != state.done_ms {
                    return respond(
                        stream,
                        409,
                        json!({"code": "chunk_out_of_order", "error": "pas le prochain"}),
                        &[],
                    );
                }
                let length = value
                    .get("duration_ms")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let lines = value
                    .get("segments")
                    .and_then(Value::as_array)
                    .map(Vec::len)
                    .unwrap_or(0);
                state.done_ms = (start + length).min(state.duration_ms);
                state.segments += lines;
                state.chunks.push(value);
                return respond(
                    stream,
                    200,
                    json!({"applied": true, "done_ms": state.done_ms, "n_segments": state.segments}),
                    &[],
                );
            }
            ("POST", "/complete") => {
                if let Some(at) = state.incomplete_at.take() {
                    state.done_ms = at;
                    return respond(
                        stream,
                        409,
                        json!({"code": "incomplete", "error": "pas complet", "done_ms": at}),
                        &[],
                    );
                }
                if state.done_ms < state.duration_ms - 5_000 {
                    return respond(
                        stream,
                        409,
                        json!({"code": "incomplete", "error": "pas complet", "done_ms": state.done_ms}),
                        &[],
                    );
                }
                state.status = "ready".to_owned();
                state.completed = true;
                return respond(
                    stream,
                    200,
                    json!({"status": "ready", "done_ms": state.done_ms, "n_segments": state.segments}),
                    &[],
                );
            }
            ("POST", "/fail") => {
                let value: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
                state.failed.push(value);
                state.status = "error".to_owned();
                return respond(
                    stream,
                    200,
                    json!({"status": "error", "done_ms": state.done_ms}),
                    &[],
                );
            }
            ("POST", "/release") => {
                state.released = true;
                state.status = "queued".to_owned();
                return respond(stream, 200, json!({"released": true}), &[]);
            }
            _ => {}
        }
    }
    respond(stream, 404, json!({"error": "inconnu"}), &[])
}

fn respond(
    stream: &mut TcpStream,
    status: u16,
    body: Value,
    extra: &[(&str, &str)],
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        416 => "Range Not Satisfiable",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        _ => "OK",
    };
    respond_raw(
        stream,
        &format!("{status} {reason}"),
        body.to_string().as_bytes(),
        extra,
    )
}

fn respond_with(
    stream: &mut TcpStream,
    status: u16,
    body: Value,
    extra: &[(&str, &str)],
) -> std::io::Result<()> {
    respond(stream, status, body, extra)
}

fn respond_bytes(
    stream: &mut TcpStream,
    status: u16,
    body: &[u8],
    extra: &[(&str, &str)],
) -> std::io::Result<()> {
    let reason = if status == 206 {
        "Partial Content"
    } else {
        "OK"
    };
    respond_raw(stream, &format!("{status} {reason}"), body, extra)
}

fn respond_raw(
    stream: &mut TcpStream,
    status: &str,
    body: &[u8],
    extra: &[(&str, &str)],
) -> std::io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in extra {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

// ---------------------------------------------------------------------------
// The fakes the supervisor runs: a decoder and an engine
// ---------------------------------------------------------------------------

/// A scratch directory of this test's own (tests run in parallel).
fn scratch(name: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "nz-t3-{}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed),
        name
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("scratch directory");
    path
}

/// Write an executable `sh` script.
fn script(directory: &Path, name: &str, body: &str) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, body).expect("script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    path
}

/// The fake decoder: the episode's length is the first line of the audio file,
/// and a chunk is `min(asked, what is left)` milliseconds of silence at 64 bytes
/// per millisecond (`audio::BYTES_PER_MS`). The real one is T2's
/// `audio::decode_range`; what the supervisor needs from it is the same two
/// facts, so the test states them without a real podcast.
struct FakePcm;

impl PcmSource for FakePcm {
    fn write_pcm(
        &self,
        audio: &Path,
        start_ms: u64,
        length_ms: u64,
        out: &Path,
    ) -> Result<u64, nz_transcribe::Error> {
        let total: u64 = std::fs::read_to_string(audio)
            .ok()
            .and_then(|text| {
                text.lines()
                    .next()
                    .and_then(|line| line.trim().parse().ok())
            })
            .unwrap_or(0);
        let length = total.saturating_sub(start_ms).min(length_ms);
        let bytes = vec![0u8; (length * 64) as usize];
        std::fs::write(out, bytes).map_err(|_| nz_transcribe::Error::NoAudio)?;
        Ok(length)
    }
}

/// The fake engine: one line at the start of the chunk and one at its end, in
/// whisper.cpp's `-oj` shape. It also writes its pid, so a test can look at
/// `/proc/<pid>/stat` and see whether the process is really stopped.
fn fake_engine(directory: &Path) -> PathBuf {
    script(
        directory,
        "fake-engine",
        r#"#!/bin/sh
pcm=""; out=""
while [ $# -gt 0 ]; do
  case "$1" in
    --pcm) pcm=$2; shift 2;;
    --out) out=$2; shift 2;;
    *) shift;;
  esac
done
dir=$(dirname "$0")
echo $$ > "$dir/pid"
echo "$(basename "$pcm" .pcm)" >> "$dir/calls"
start=$(basename "$pcm" .pcm); start=${start#chunk-}
if [ -f "$dir/crash-at" ] && [ "$(cat "$dir/crash-at")" = "$start" ]; then
  echo "boum sur le chunk $start" >&2
  exit 1
fi
if [ -f "$dir/wait-for-go" ]; then
  while [ ! -f "$dir/go" ]; do sleep 0.02; done
fi
ms=$(( $(wc -c < "$pcm") / 64 ))
if [ "$ms" -le 0 ]; then echo '{"transcription":[]}' > "$out"; exit 0; fi
half=$((ms / 2))
printf '{"transcription":[{"offsets":{"from":0,"to":%d},"text":"début"},{"offsets":{"from":%d,"to":%d},"text":"fin"}],"result":{"language":"fr"}}' \
  "$half" "$half" "$((ms - 1))" > "$out"
"#,
    )
}

/// The supervisor a test drives, with everything pointed at its own fakes.
struct Harness {
    supervisor: Supervisor,
    sink: Arc<ReplaySink>,
    client: Client,
}

fn harness(fake: &Fake, directory: &Path, retry: Retry, heartbeat: Duration) -> Harness {
    let engine = Engine {
        program: fake_engine(directory),
        model: directory.join("ggml-small.bin"),
        cpu: true,
    };
    let pcm = Arc::new(FakePcm);
    let sink = Arc::new(ReplaySink::default());
    let mut supervisor = Supervisor::new(
        engine,
        pcm,
        sink.clone(),
        directory.join("work"),
        directory.join("transcripts"),
    );
    supervisor.client = Some(
        Client::with_retry(
            &fake.base,
            Token::new("device-token").expect("token"),
            retry,
        )
        .expect("client"),
    );
    supervisor.heartbeat = heartbeat;
    let client = supervisor.client.clone().expect("client");
    Harness {
        supervisor,
        sink,
        client,
    }
}

/// The claim the fake server answers, as the client reads it.
async fn claim(client: &Client) -> nz_transcribe::protocol::Claim {
    client
        .claim()
        .await
        .expect("claim")
        .expect("one queued episode")
}

// ---------------------------------------------------------------------------
// The acceptance criteria of lot T3
// ---------------------------------------------------------------------------

/// **Criterion 1**: claim → audio → chunks → complete, and the server's
/// transcript comes out `ready` with every line, in six requests and not one
/// `fail` or `release`.
#[tokio::test]
async fn a_claim_becomes_a_ready_transcript() {
    let directory = scratch("ready");
    let state = Arc::new(Mutex::new(State::new()));
    let fake = Fake::start(state.clone());
    let mut run = harness(&fake, &directory, Retry::default(), Duration::from_secs(5));
    let claim = claim(&run.client).await;
    assert_eq!(claim.duration_ms, 1_800_000);
    assert_eq!(claim.chunk_ms(), 600_000);

    let outcome = run.supervisor.run_claim(claim).await;
    assert_eq!(outcome, Outcome::Done);

    let state = fake.state();
    assert_eq!(state.status, "ready");
    assert_eq!(state.done_ms, 1_800_000, "the whole episode is transcribed");
    assert_eq!(state.segments, 6, "two lines per chunk, three chunks");
    assert!(state.completed);
    assert!(state.failed.is_empty(), "{:?}", state.failed);
    assert!(!state.released);
    // Three chunks of ten minutes: start_ms 0, 600 000, 1 200 000.
    let starts: Vec<i64> = state
        .chunks
        .iter()
        .map(|chunk| chunk["start_ms"].as_i64().expect("start"))
        .collect();
    assert_eq!(starts, [0, 600_000, 1_200_000]);
    assert!(
        state
            .chunks
            .iter()
            .all(|chunk| chunk["engine"] == "whisper.cpp" && chunk["language"] == "fr"),
        "every chunk is stamped with the engine and the language"
    );
    drop(state);

    let chunk = format!("/api/transcribe/worker/{EPISODE}/chunk");
    let audio = format!("/api/transcribe/worker/audio/{EPISODE}");
    let complete = format!("/api/transcribe/worker/{EPISODE}/complete");
    assert_eq!(
        fake.paths(),
        vec![
            "/api/transcribe/worker/claim".to_owned(),
            audio,
            chunk.clone(),
            chunk.clone(),
            chunk,
            complete,
        ],
        "one claim, one audio, one chunk each, one completion"
    );
    let tags = run.sink.tags();
    assert_eq!(
        tags.iter().filter(|tag| *tag == "progress").count(),
        3,
        "{tags:?}"
    );
    assert_eq!(tags.last().map(String::as_str), Some("done"), "{tags:?}");
    let done = run
        .sink
        .events()
        .into_iter()
        .find(|event| matches!(event, nz_transcribe::control::Event::Done { .. }));
    match done {
        Some(nz_transcribe::control::Event::Done { segments, .. }) => assert_eq!(segments.len(), 6),
        other => panic!("no done event: {other:?}"),
    }
    std::fs::remove_dir_all(&directory).expect("clean up");
}

/// **Criterion 2**: a `cancel` from the server stops the job within one
/// heartbeat, gives the lease back, and leaves no engine process behind.
#[tokio::test]
async fn a_cancel_stops_within_one_heartbeat() {
    let directory = scratch("cancel");
    // The engine waits for a file the test never creates: the job can only end
    // by the server's cancel.
    std::fs::write(directory.join("wait-for-go"), b"1").expect("marker");
    let state = Arc::new(Mutex::new(State::new()));
    state.lock().expect("state").cancel_at_beat = Some(1);
    let fake = Fake::start(state.clone());
    let mut run = harness(
        &fake,
        &directory,
        Retry::default(),
        Duration::from_millis(300),
    );
    let claim = claim(&run.client).await;
    let started = Instant::now();
    let outcome = run.supervisor.run_claim(claim).await;
    let elapsed = started.elapsed();

    assert!(state.lock().expect("state").released, "the lease goes back");
    assert_eq!(outcome, Outcome::Idle);
    // One heartbeat (300 ms) plus the shutdown of a stopped child.
    assert!(
        elapsed < Duration::from_secs(3),
        "the cancel took {elapsed:?}"
    );
    let beats = state.lock().expect("state").beats.len();
    assert!(beats >= 1, "{beats} heartbeat(s)");
    let pid = std::fs::read_to_string(directory.join("pid"))
        .expect("the engine wrote its pid")
        .trim()
        .to_owned();
    // Either the process is gone, or it is a zombie its parent has not reaped
    // yet; in both cases nothing is whispering any more.
    let alive = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| {
            stat.split(')')
                .nth(1)
                .and_then(|tail| tail.trim_start().chars().next())
        })
        .is_some_and(|state| state != 'Z');
    assert!(!alive, "the engine process {pid} is still running");
    std::fs::remove_dir_all(&directory).expect("clean up");
}

/// **Criterion 3**: a `SIGSTOP`ped engine keeps its lease — the heartbeat is not
/// on the engine's thread, so ≥ 3 beats arrive during a 15 s pause, and
/// `/proc/<pid>/stat` says the child is really stopped (`T`).
#[tokio::test]
async fn a_stopped_engine_keeps_its_lease() {
    let directory = scratch("stopped");
    std::fs::write(directory.join("wait-for-go"), b"1").expect("marker");
    let state = Arc::new(Mutex::new(State::new()));
    let fake = Fake::start(state.clone());
    let mut run = harness(&fake, &directory, Retry::default(), Duration::from_secs(5));
    let claim = claim(&run.client).await;

    let halt = run.supervisor.halt.clone();
    let job = async { run.supervisor.run_claim(claim).await };
    let pause = async {
        // Let the engine start and the supervisor warn the first chunk in.
        tokio::time::sleep(Duration::from_millis(600)).await;
        halt.pause(true);
        let paused_at = Instant::now();
        tokio::time::sleep(Duration::from_secs(15)).await;
        let beats = state.lock().expect("state").beats.len();
        let pid = std::fs::read_to_string(directory.join("pid")).expect("pid");
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", pid.trim())).expect("stat");
        // The state is the first field after the parenthesised command name.
        let state_letter = stat
            .split(')')
            .nth(1)
            .and_then(|tail| tail.trim_start().chars().next())
            .unwrap_or('?');
        std::fs::write(directory.join("go"), b"1").expect("go");
        halt.pause(false);
        (paused_at.elapsed(), beats, state_letter)
    };
    let (job_outcome, (paused_for, beats, letter)) = tokio::join!(job, pause);
    assert_eq!(job_outcome, Outcome::Done, "{:?}", run.sink.tags());
    assert_eq!(letter, 'T', "the engine was not stopped (state {letter:?})");
    assert!(
        beats >= 3,
        "{beats} heartbeat(s) in {paused_for:?} of pause, expected at least 3"
    );
    std::fs::remove_dir_all(&directory).expect("clean up");
}

/// **Criterion 4**: a `429` with `Retry-After` and a `500` are retried, exactly
/// as many times as the schedule says and no more — counted requests.
#[tokio::test]
async fn a_rate_limit_backs_off_without_a_storm() {
    let directory = scratch("backoff");
    let state = Arc::new(Mutex::new(State::new()));
    state.lock().expect("state").rate_limit_claims = 2;
    let fake = Fake::start(state.clone());
    let retry = Retry {
        attempts: 3,
        base: 0.01,
        max: 0.02,
        retry_after_cap: 0.02,
    };
    let run = harness(&fake, &directory, retry, Duration::from_secs(5));

    let started = Instant::now();
    let claim = claim(&run.client).await;
    let elapsed = started.elapsed();
    assert_eq!(claim.episode_id, EPISODE);
    assert_eq!(
        fake.state().count("/claim"),
        3,
        "two rate limits, then the claim"
    );
    // `Retry-After: 1` is honoured, capped by this client's 20 ms schedule.
    assert!(elapsed >= Duration::from_millis(40), "{elapsed:?}");

    // A server that keeps failing is not hammered: three attempts, then the
    // answer, and the worker reports it.
    let state = Arc::new(Mutex::new(State::new()));
    state.lock().expect("state").fail_claims = 99;
    let fake = Fake::start(state.clone());
    let retry = Retry {
        attempts: 3,
        base: 0.01,
        max: 0.02,
        retry_after_cap: 0.02,
    };
    let run = harness(&fake, &directory, retry, Duration::from_secs(5));
    let outcome = run.client.claim().await;
    assert!(
        matches!(outcome, Err(Failure::Http { status: 500, .. })),
        "{outcome:?}"
    );
    assert_eq!(fake.state().count("/claim"), 3, "attempts, not a storm");
    std::fs::remove_dir_all(&directory).expect("clean up");
}

/// **Criterion 5**: a local job resumes at its last chunk after a crash, and the
/// sidecar it writes is the server's shape.
#[tokio::test]
async fn a_local_job_resumes_at_its_last_chunk() {
    let directory = scratch("local");
    let audio = directory.join("episode.mp3");
    // 1 500 000 ms: two full chunks and a half one.
    std::fs::write(&audio, b"1500000\n").expect("audio");
    let transcripts = directory.join("transcripts");
    let engine = Engine {
        program: fake_engine(&directory),
        model: directory.join("ggml-small.bin"),
        cpu: true,
    };
    let build = || {
        let sink = Arc::new(ReplaySink::default());
        let mut supervisor = Supervisor::new(
            engine.clone(),
            Arc::new(FakePcm),
            sink.clone(),
            directory.join("work"),
            transcripts.clone(),
        );
        supervisor.heartbeat = Duration::from_secs(5);
        (supervisor, sink)
    };

    // First run: the engine dies on the second chunk (a real crash, not a
    // signal), so the job stops with one chunk stored.
    std::fs::write(directory.join("crash-at"), b"600000").expect("crash marker");
    let (mut first, _) = build();
    let outcome = first.run_local_job(EPISODE, &audio, None).await;
    assert_eq!(outcome, Outcome::Idle, "a crash is not a completion");
    let partial = std::fs::read_to_string(transcripts.join(format!("{EPISODE}.partial.json")))
        .expect("partial sidecar");
    let partial: Value = serde_json::from_str(&partial).expect("json");
    assert_eq!(partial["done_ms"], 600_000, "{partial}");
    assert_eq!(
        partial["segments"].as_array().map(Vec::len),
        Some(2),
        "{partial}"
    );

    // Second run: it resumes at 600 000, from the lines already kept.
    std::fs::remove_file(directory.join("crash-at")).ok();
    let calls_before = std::fs::read_to_string(directory.join("calls")).expect("calls");
    let (mut second, sink) = build();
    let outcome = second.run_local_job(EPISODE, &audio, None).await;
    assert_eq!(outcome, Outcome::Done, "{:?}", sink.tags());
    let calls = std::fs::read_to_string(directory.join("calls")).expect("calls");
    let resumed: Vec<&str> = calls[calls_before.len()..]
        .lines()
        .filter(|line| !line.is_empty())
        .collect();
    assert_eq!(
        resumed,
        ["chunk-600000", "chunk-1200000"],
        "a resumed job starts at its last chunk"
    );

    let document =
        nz_transcribe::sidecar::load(&transcripts.join(format!("{EPISODE}.transcript.json")))
            .expect("the sidecar");
    assert_eq!(document.duration_ms, 1_500_000);
    assert_eq!(document.engine.as_deref(), Some("whisper.cpp"));
    assert_eq!(document.language.as_deref(), Some("fr"));
    assert_eq!(document.segments.len(), 6, "three chunks, two lines each");
    assert!(
        !transcripts.join(format!("{EPISODE}.partial.json")).exists(),
        "a finished job leaves no partial file"
    );
    assert!(
        transcripts
            .join(format!("{EPISODE}.transcript.srt"))
            .exists(),
        "the SRT goes with the JSON"
    );
    std::fs::remove_dir_all(&directory).expect("clean up");
}

/// A `stop` on the control channel ends a job that is running, and says so.
#[tokio::test]
async fn the_control_channel_stops_a_running_job() {
    let directory = scratch("control");
    std::fs::write(directory.join("wait-for-go"), b"1").expect("marker");
    let audio = directory.join("episode.mp3");
    std::fs::write(&audio, b"600000\n").expect("audio");
    let engine = Engine {
        program: fake_engine(&directory),
        model: directory.join("ggml-small.bin"),
        cpu: true,
    };
    let sink = Arc::new(ReplaySink::default());
    let mut supervisor = Supervisor::new(
        engine,
        Arc::new(FakePcm),
        sink.clone(),
        directory.join("work"),
        directory.join("transcripts"),
    );
    let halt = supervisor.halt.clone();
    let job = async { supervisor.run_local_job(EPISODE, &audio, None).await };
    let stop = async {
        tokio::time::sleep(Duration::from_millis(400)).await;
        halt.stop();
    };
    let (outcome, ()) = tokio::join!(job, stop);
    assert_eq!(outcome, Outcome::Idle);
    assert!(
        !sink.tags().contains(&"error".to_owned()),
        "a stop the person asked for is not a failure: {:?}",
        sink.tags()
    );
    // A stopped engine is terminated, not left whispering: its pid is gone.
    let pid = std::fs::read_to_string(directory.join("pid")).expect("pid");
    assert!(!Path::new(&format!("/proc/{}", pid.trim())).exists());
    std::fs::remove_dir_all(&directory).expect("clean up");
}

/// The engine's JSON is read as `engine.py#parse_json` reads it, and a broken
/// one is the server's `bad_json`, never a panic.
#[test]
fn the_engines_json_is_read_like_the_servers() {
    let good = br#"{"transcription":[{"offsets":{"from":0,"to":900},"text":"bonjour"}],"result":{"language":"fr"}}"#;
    let recognized = parse_engine_json(good).expect("a transcript");
    assert_eq!(recognized.items.len(), 1);
    assert_eq!(recognized.language.as_deref(), Some("fr"));
    assert!(matches!(
        parse_engine_json(b"nope"),
        Err(nz_transcribe::Error::BadJson)
    ));
    assert!(matches!(
        parse_engine_json(b"[]"),
        Err(nz_transcribe::Error::BadJsonShape)
    ));
    assert!(matches!(
        parse_engine_json(br#"{"result":{}}"#),
        Err(nz_transcribe::Error::NoTranscription)
    ));
}

/// A pause the app asks for is not a resume the moment it stops asking: the
/// engine waits `RESUME_AFTER_MS` of quiet (rule 1: a bursty trigger is one
/// pause, not dozens), and an engine that is not paused is not signalled.
#[test]
fn a_pause_has_three_seconds_of_hysteresis() {
    assert_eq!(nz_transcribe::supervisor::RESUME_AFTER_MS, 3_000);
    const {
        assert!(
            nz_transcribe::supervisor::RESUME_AFTER_MS
                > nz_transcribe::supervisor::AUDIO_LOW_WATER_MS
        );
        assert!(
            nz_transcribe::supervisor::AUDIO_LOW_WATER_MS
                > nz_transcribe::supervisor::USER_LANE_GRACE_MS
        );
    };
    let halt = nz_transcribe::supervisor::Halt::default();
    assert!(!halt.paused());
    halt.pause(true);
    assert!(halt.paused());
    halt.pause(false);
    assert!(!halt.paused());
    halt.cancel();
    assert!(halt.stopping() && halt.cancelled());
    halt.reset();
    assert!(!halt.stopping() && !halt.cancelled());
    halt.revoke();
    assert!(halt.stopping() && halt.revoked());
}

/// The control channel's four commands reach a waiting worker: `stop` ends it,
/// `pause`/`resume` flip the flag, and `local` runs one job.
#[tokio::test]
async fn a_stop_command_ends_an_idle_worker() {
    let directory = scratch("idle");
    let state = Arc::new(Mutex::new(State::new()));
    state.lock().expect("state").status = "ready".to_owned();
    let fake = Fake::start(state);
    let mut run = harness(&fake, &directory, Retry::default(), Duration::from_secs(5));
    run.supervisor.poll = Duration::from_millis(50);
    let (sender, receiver) = tokio::sync::mpsc::channel(4);
    sender.send(Command::Pause).await.expect("send");
    sender.send(Command::Resume).await.expect("send");
    sender.send(Command::Stop).await.expect("send");
    // The channel is still open when the worker starts, so the stop arrives.
    let started = Instant::now();
    let outcome = run.supervisor.run_worker(Some(receiver)).await;
    assert_eq!(outcome, Outcome::Idle);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(
        run.sink.tags().contains(&"idle".to_owned()),
        "an empty queue says so: {:?}",
        run.sink.tags()
    );
    std::fs::remove_dir_all(&directory).expect("clean up");
}

/// The production decoder seam, on a real file: `AudioPcm` (T2's
/// `audio::decode_range` + `write_pcm`) answers the window it was asked for, at
/// 64 bytes of PCM per millisecond, whatever the source's rate and channels.
///
/// Skipped with a printed reason when `ffmpeg` is not installed; fails instead
/// when `NS_REQUIRE_FFMPEG=1`, the repository's rule for a suite that must not
/// silently lose its real-audio coverage.
#[test]
fn the_real_decoder_answers_the_window_it_was_asked_for() {
    if std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| !status.success())
        .unwrap_or(true)
    {
        assert!(
            std::env::var_os("NS_REQUIRE_FFMPEG").is_none(),
            "ffmpeg est absent alors que NS_REQUIRE_FFMPEG=1"
        );
        eprintln!("ffmpeg absent : le décodeur réel n'est pas mesuré ici");
        return;
    }
    let directory = scratch("decode");
    let audio = directory.join("episode.mp3");
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=3:sample_rate=44100",
            "-ac",
            "2",
            "-b:a",
            "96k",
        ])
        .arg(&audio)
        .status()
        .expect("ffmpeg");
    assert!(status.success(), "ffmpeg n'a pas produit l'audio de test");

    let out = directory.join("chunk.pcm");
    let written = nz_transcribe::supervisor::AudioPcm
        .write_pcm(&audio, 1_000, 1_000, &out)
        .expect("the window decodes");
    let bytes = std::fs::metadata(&out).expect("pcm").len();
    assert_eq!(written, bytes / 64, "64 bytes of f32 per millisecond");
    assert!(
        (950..=1_050).contains(&written),
        "{written} ms of PCM for a 1 000 ms window"
    );
    // The whole file, then a window past its end: a short read, never an error.
    let whole = nz_transcribe::supervisor::AudioPcm
        .write_pcm(&audio, 0, 60_000, &directory.join("whole.pcm"))
        .expect("the whole file decodes");
    assert!(
        (2_900..=3_100).contains(&whole),
        "{whole} ms for a 3 s file"
    );
    let past = nz_transcribe::supervisor::AudioPcm
        .write_pcm(&audio, 5_000, 1_000, &directory.join("past.pcm"))
        .expect("past the end");
    eprintln!("MEASURE past-the-end window: {past} ms (a 5 s window start on a 3 s file)");
    assert_eq!(past, 0, "past the end is zero milliseconds, not an error");
    // Where the window STARTS, measured on a file that is loud for half a
    // second and silent after: a window at 600 ms must be silence. This is what
    // catches a decoder that treats `start_ms` as a sample count (`nz-audio`'s
    // `ResamplerStage` and this file's `Collector` both count 16 kHz samples).
    let tone = directory.join("tone.mp3");
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=if(lt(t\\,0.5)\\,sin(2*PI*440*t)\\,0):d=3:s=44100",
            "-ac",
            "1",
        ])
        .arg(&tone)
        .status()
        .expect("ffmpeg");
    assert!(status.success());
    let window = directory.join("window.pcm");
    let ms = nz_transcribe::supervisor::AudioPcm
        .write_pcm(&tone, 600, 100, &window)
        .expect("the window decodes");
    assert_eq!(ms, 100);
    let bytes = std::fs::read(&window).expect("pcm");
    let samples: Vec<f32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
        .collect();
    let peak = samples
        .iter()
        .fold(0f32, |peak, sample| peak.max(sample.abs()));
    assert!(
        peak < 0.01,
        "a window at 600 ms of a file that is silent after 500 ms is silence (peak {peak})"
    );
    std::fs::remove_dir_all(&directory).expect("clean up");
}

/// A download that finds bytes already on disk asks only for the rest, with
/// `Range`, and finishes the file (`worker.download_audio`).
#[tokio::test]
async fn a_partial_download_resumes_with_range() {
    let directory = scratch("resume");
    let state = Arc::new(Mutex::new(State::new()));
    state.lock().expect("state").audio = b"0123456789".to_vec();
    let fake = Fake::start(state);
    let client =
        Client::new(&fake.base, Token::new("device-token").expect("token")).expect("client");
    let dest = directory.join("audio");
    std::fs::write(&dest, b"01234").expect("the first five bytes");

    let bytes = client
        .download(EPISODE, "lease-token", &dest, || false)
        .await
        .expect("the rest downloads");
    assert_eq!(bytes, 10);
    assert_eq!(std::fs::read(&dest).expect("audio"), b"0123456789");
    assert_eq!(fake.state().ranges, ["bytes=5-"], "the request resumes");
    assert_eq!(fake.state().requests.len(), 1, "one request, not a restart");
    std::fs::remove_dir_all(&directory).expect("clean up");
}

/// `POST …/complete` answering `409 incomplete` at or behind the worker's own
/// position is a verdict, not something to retry: the worker stores the cause
/// and stops, once (`worker.py`: "le serveur n'avance plus").
#[tokio::test]
async fn an_incomplete_answer_is_a_verdict_not_a_retry_loop() {
    let directory = scratch("incomplete");
    let state = Arc::new(Mutex::new(State::new()));
    state.lock().expect("state").incomplete_at = Some(1_200_000);
    let fake = Fake::start(state);
    let mut run = harness(&fake, &directory, Retry::default(), Duration::from_secs(5));
    let claim = claim(&run.client).await;
    let outcome = run.supervisor.run_claim(claim).await;
    assert_eq!(outcome, Outcome::Idle, "{:?}", run.sink.tags());
    let state = fake.state();
    assert_eq!(state.count("/complete"), 1, "one attempt, not a loop");
    let failed = state.failed.first().expect("the cause is stored");
    assert_eq!(failed["code"], "incomplete", "{failed}");
    assert!(
        failed["message"]
            .as_str()
            .is_some_and(|text| text.contains("1200000")),
        "{failed}"
    );
    drop(state);
    assert_eq!(
        run.sink.tags().last().map(String::as_str),
        Some("error"),
        "{:?}",
        run.sink.tags()
    );
    std::fs::remove_dir_all(&directory).expect("clean up");
}

/// The `local` command on the control channel runs one local job on a worker
/// that is enrolled and idle, and the app sees its events.
#[tokio::test]
async fn a_local_command_runs_a_job_between_two_claims() {
    let directory = scratch("local-command");
    let audio = directory.join("episode.mp3");
    std::fs::write(&audio, b"1200000\n").expect("audio");
    let state = Arc::new(Mutex::new(State::new()));
    // Nothing in the server's queue: the worker is idle.
    state.lock().expect("state").status = "ready".to_owned();
    let fake = Fake::start(state);
    let mut run = harness(&fake, &directory, Retry::default(), Duration::from_secs(5));
    run.supervisor.poll = Duration::from_millis(50);
    let (sender, receiver) = tokio::sync::mpsc::channel(4);
    sender
        .send(Command::Local {
            episode_id: EPISODE.to_owned(),
            audio: audio.to_string_lossy().into_owned(),
            language: Some("fr".to_owned()),
        })
        .await
        .expect("send");
    sender.send(Command::Stop).await.expect("send");
    let outcome = run.supervisor.run_worker(Some(receiver)).await;
    assert_eq!(outcome, Outcome::Idle);
    let tags = run.sink.tags();
    assert!(tags.contains(&"done".to_owned()), "{tags:?}");
    let document = nz_transcribe::sidecar::load(
        &directory
            .join("transcripts")
            .join(format!("{EPISODE}.transcript.json")),
    )
    .expect("the local sidecar");
    assert_eq!(document.duration_ms, 1_200_000);
    assert_eq!(document.segments.len(), 4, "two chunks, two lines each");
    std::fs::remove_dir_all(&directory).expect("clean up");
}
