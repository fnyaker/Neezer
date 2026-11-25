//! The host harness: a loopback server that speaks the routes the player uses,
//! throwaway directories, and the few fixtures real audio needs.
//!
//! **Two servers, on purpose** (the same split MB2's own harness makes): when a
//! Python that can `import neezer` is available, the tests run against
//! `tools/perf_api.py --serve` — the REAL routes, the real session cookie, the
//! real `/stream` contract — and when it is not (the mobile CI's Rust job
//! installs no Python) they run against [`Bench`], an in-process server that
//! answers the same wire shapes and RECORDS every request, which is what the
//! per-request assertions (one `/flow/next` per title, one `/listen` per
//! change, `q=OPUS_64` after three slow streams) are read from.
//!
//! Nothing waits on the clock: a fixture is ready when its file exists, a player
//! change is waited on through the sink they share, and a request is counted by
//! the server that answered it.

#![allow(dead_code)]

mod audio;
mod rules;

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::core::{Core, CoreConfig, NetworkState};
use crate::player::{Player, PlayerError};

// --- a throwaway data + cache directory -------------------------------------

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

/// One test's two roots, removed on drop.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let number = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("nz-player-{}-{tag}-{number}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("data")).expect("data dir");
        std::fs::create_dir_all(path.join("cache")).expect("cache dir");
        TempDir { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn core(&self) -> Core {
        Core::new(CoreConfig {
            data_dir: self.path.join("data"),
            cache_dir: self.path.join("cache"),
            app_version: "0.0.0-test".to_owned(),
            version_code: 2_000_001,
            sdk_int: 35,
            device_name: "bench".to_owned(),
        })
        .expect("a core over a throwaway directory")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

// --- the recording server ---------------------------------------------------

/// One recorded request: the method and the path as it arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    pub body: String,
}

impl Recorded {
    /// `GET /api/stream/1?q=OPUS_64`.
    pub fn line(&self) -> String {
        format!("{} {}", self.method, self.path)
    }

    pub fn target(&self) -> &str {
        self.path.split('?').next().unwrap_or(&self.path)
    }

    pub fn query(&self, key: &str) -> Option<&str> {
        let query = self.path.split_once('?')?.1;
        query.split('&').find_map(|pair| {
            let (name, value) = pair.split_once('=')?;
            (name == key).then_some(value)
        })
    }
}

/// What a served track is: its bytes, the bounds the server would measure, and
/// whether the body trickles (a slow link, for the ladder).
pub struct Served {
    pub bytes: Vec<u8>,
    pub edges: Option<(f64, f64, f64)>,
    /// Bytes per second the body is written at; 0 = as fast as the socket takes.
    pub throttle: u64,
}

#[derive(Default)]
pub struct BenchState {
    pub tracks: Mutex<HashMap<String, Served>>,
    pub requests: Mutex<Vec<Recorded>>,
    pub listens: Mutex<Vec<String>>,
    pub progress: Mutex<Vec<String>>,
    /// What `/flow/next` answers next, one batch per call.
    pub flow: Mutex<Vec<Vec<String>>>,
    /// What `/radio/track/<id>` and `/radio/artist/<id>` answer.
    pub radio: Mutex<Vec<String>>,
    /// What `/album/<id>`, `/playlist/<id>` and `/me/favorites` answer.
    pub lists: Mutex<HashMap<String, Vec<String>>>,
    /// What `/api/gains` answers per id: the server's loudness measurement
    /// (`None` = measured as absent, which normalisation must leave alone).
    pub gains: Mutex<HashMap<String, Option<f32>>>,
    pub flow_calls: AtomicUsize,
}

/// The in-process server.
pub struct Bench {
    base: String,
    pub state: Arc<BenchState>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Bench {
    pub fn start() -> Bench {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the bench");
        let base = format!(
            "http://{}",
            listener.local_addr().expect("the bench's address")
        );
        let state = Arc::new(BenchState::default());
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::clone(&state);
        let stopping = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            for stream in listener.incoming() {
                if stopping.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let shared = Arc::clone(&shared);
                thread::spawn(move || {
                    let _ = serve(stream, &shared);
                });
            }
        });
        Bench {
            base,
            state,
            stop,
            thread: Some(thread),
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// Serves `path`'s bytes for `id`, with bounds when they are given.
    pub fn track(&self, id: &str, path: &Path) -> &Self {
        let bytes = std::fs::read(path).expect("the fixture's bytes");
        self.state.tracks.lock().unwrap().insert(
            id.to_owned(),
            Served {
                bytes,
                edges: None,
                throttle: 0,
            },
        );
        self
    }

    /// A track whose file is not on disk: the server answers `404`, which is a
    /// verdict about that title and what the fault policy skips.
    pub fn broken(&self, id: &str) {
        // No entry: `/stream/<id>` answers 404 below.
        let _ = id;
    }

    /// Serves `id` with measured bounds, so the trim reaches the engine.
    pub fn edges(&self, id: &str, start: f64, end: f64, duration: f64) {
        let mut tracks = self.state.tracks.lock().unwrap();
        if let Some(track) = tracks.get_mut(id) {
            track.edges = Some((start, end, duration));
        }
    }

    /// A slow body: the ladder's measurement reads what the socket delivers.
    pub fn slow(&self, id: &str, bytes_per_s: u64) {
        let mut tracks = self.state.tracks.lock().unwrap();
        if let Some(track) = tracks.get_mut(id) {
            track.throttle = bytes_per_s;
        }
    }

    pub fn requests(&self) -> usize {
        self.state.requests.lock().unwrap().len()
    }

    pub fn recorded(&self) -> Vec<Recorded> {
        self.state.requests.lock().unwrap().clone()
    }

    pub fn since(&self, from: usize) -> Vec<Recorded> {
        let all = self.recorded();
        all.into_iter().skip(from).collect()
    }

    pub fn listens(&self) -> Vec<String> {
        self.state.listens.lock().unwrap().clone()
    }

    pub fn progress(&self) -> Vec<String> {
        self.state.progress.lock().unwrap().clone()
    }

    pub fn flow_calls(&self) -> usize {
        self.state.flow_calls.load(Ordering::SeqCst)
    }

    /// The next `/flow/next` answers this batch of titles, in order.
    pub fn flow_batch(&self, ids: &[&str]) {
        self.state
            .flow
            .lock()
            .unwrap()
            .push(ids.iter().map(|id| (*id).to_owned()).collect());
    }

    pub fn radio(&self, ids: &[&str]) {
        *self.state.radio.lock().unwrap() = ids.iter().map(|id| (*id).to_owned()).collect();
    }

    /// What the server's ReplayGain measurement says for `id`: `None` is the
    /// honest "measured nothing", not zero (the bench answers `null`, as
    /// `/api/gains` does for a track it has no loudness for).
    pub fn gains(&self, id: &str, loudness: Option<f32>) -> &Self {
        self.state
            .gains
            .lock()
            .unwrap()
            .insert(id.to_owned(), loudness);
        self
    }

    pub fn list(&self, path: &str, ids: &[&str]) {
        self.state.lists.lock().unwrap().insert(
            path.to_owned(),
            ids.iter().map(|id| (*id).to_owned()).collect(),
        );
    }
}

impl Drop for Bench {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.base.trim_start_matches("http://"));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

const VERSION_BODY: &str = r#"{"version":"9.9.9","build":"bench"}"#;
const ME_BODY: &str = r#"{"user":{"id":"u1","name":"bench","admin":true,"onboarding":"done","access":{},"sources":[]}}"#;

/// One request, one answer, one closed connection.
fn serve(mut stream: TcpStream, state: &Arc<BenchState>) -> std::io::Result<()> {
    stream.set_nodelay(true)?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    let mut headers: Vec<(String, String)> = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
        }
    }
    let length: usize = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    let mut body = Vec::with_capacity(length);
    while body.len() < length {
        let mut rest = vec![0u8; length - body.len()];
        match reader.read(&mut rest) {
            Ok(0) | Err(_) => break,
            Ok(read) => body.extend_from_slice(&rest[..read]),
        }
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let target = parts.next().unwrap_or_default().to_owned();
    let body_text = String::from_utf8_lossy(&body).into_owned();
    state.requests.lock().unwrap().push(Recorded {
        method: method.clone(),
        path: target.clone(),
        body: body_text.clone(),
    });

    match route(&method, &target, &headers, &body_text, state) {
        Answer::Reply(status, extra, answer) => {
            let head = format!(
                "HTTP/1.1 {status} X\r\nContent-Length: {}\r\n{extra}\r\nConnection: close\r\n\r\n",
                answer.len()
            );
            stream.write_all(head.as_bytes())?;
            stream.write_all(&answer)?;
            stream.flush()
        }
        Answer::Body {
            status,
            extra,
            bytes,
            throttle,
        } => {
            let head = format!(
                "HTTP/1.1 {status} X\r\nContent-Length: {}\r\n{extra}\r\nConnection: close\r\n\r\n",
                bytes.len()
            );
            stream.write_all(head.as_bytes())?;
            stream.flush()?;
            if throttle == 0 {
                return stream.write_all(&bytes).and_then(|()| stream.flush());
            }
            // A slow link: the pieces are paced AFTER the head, so the client's
            // own clock starts where the body starts — which is what the
            // quality ladder measures.
            let chunk = 16 * 1024;
            let delay = Duration::from_secs_f64(chunk as f64 / throttle as f64);
            for piece in bytes.chunks(chunk) {
                stream.write_all(piece)?;
                stream.flush()?;
                thread::sleep(delay);
            }
            Ok(())
        }
    }
}

/// What a route answers: a whole reply, or a body the serve loop writes so a
/// paced one can be measured.
enum Answer {
    Reply(u16, String, Vec<u8>),
    Body {
        status: u16,
        extra: String,
        bytes: Vec<u8>,
        throttle: u64,
    },
}

fn json(status: u16, text: String) -> Answer {
    Answer::Reply(
        status,
        "Content-Type: application/json".to_owned(),
        text.into_bytes(),
    )
}

/// The `nz-model::Track` shape a list route answers: the id is optional, the
/// universal id is what every request names.
fn track_json(id: &str) -> String {
    format!(
        r#"{{"id":null,"deezer_id":"{id}","title":"Titre {id}","duration":200,"explicit":false,"local":false,"artist":{{"deezer_id":"a1","name":"Artiste"}},"artists":[],"album":{{"deezer_id":"al1","title":"Album","cover":null}}}}"#
    )
}

fn tracks_json(ids: &[String]) -> String {
    let rows: Vec<String> = ids.iter().map(|id| track_json(id)).collect();
    format!(r#"{{"tracks":[{}]}}"#, rows.join(","))
}

fn route(
    method: &str,
    target: &str,
    headers: &[(String, String)],
    body: &str,
    state: &Arc<BenchState>,
) -> Answer {
    let path = target.split('?').next().unwrap_or(target);
    let query = target.split_once('?').map(|(_, query)| query).unwrap_or("");
    let param = |key: &str| -> Option<String> {
        query.split('&').find_map(|pair| {
            let (name, value) = pair.split_once('=')?;
            (name == key).then(|| value.to_owned())
        })
    };
    match (method, path) {
        ("GET", "/api/version") => json(200, VERSION_BODY.to_owned()),
        ("POST", "/api/login") => Answer::Reply(
            200,
            "Content-Type: application/json\r\nSet-Cookie: session=good; Path=/".to_owned(),
            ME_BODY.as_bytes().to_vec(),
        ),
        ("GET", "/api/me") => json(200, ME_BODY.to_owned()),
        ("POST", "/api/listen") => {
            state.listens.lock().unwrap().push(body.to_owned());
            Answer::Reply(204, String::new(), Vec::new())
        }
        ("POST", "/api/podcast/progress") => {
            state.progress.lock().unwrap().push(body.to_owned());
            Answer::Reply(204, String::new(), Vec::new())
        }
        ("GET", "/api/podcast/progress") => json(200, r#"{"progress":{}}"#.to_owned()),
        ("POST", "/api/gains") => {
            let ids: Vec<String> = body
                .split('"')
                .filter(|part| {
                    !part.is_empty() && *part != "ids" && !part.contains(['{', '}', ',', ']'])
                })
                .map(str::to_owned)
                .collect();
            // The stored measurement, or `null`: a bench that answered nothing
            // but `null` could not tell a working gain from a missing one.
            let stored = state.gains.lock().unwrap();
            let rows: Vec<String> = ids
                .iter()
                .map(|id| match stored.get(id).copied().flatten() {
                    Some(loudness) => format!(r#""{id}":{loudness}"#),
                    None => format!(r#""{id}":null"#),
                })
                .collect();
            json(200, format!(r#"{{"gains":{{{}}}}}"#, rows.join(",")))
        }
        ("POST", "/api/flow/next") => {
            state.flow_calls.fetch_add(1, Ordering::SeqCst);
            let batch = {
                let mut flow = state.flow.lock().unwrap();
                if flow.is_empty() {
                    None
                } else {
                    Some(flow.remove(0))
                }
            };
            match batch {
                Some(ids) => json(200, tracks_json(&ids)),
                None => json(200, r#"{"tracks":[]}"#.to_owned()),
            }
        }
        ("GET", "/api/me/favorites") | ("GET", "/api/me/local") => {
            let ids = state
                .lists
                .lock()
                .unwrap()
                .get(path)
                .cloned()
                .unwrap_or_default();
            json(200, tracks_json(&ids))
        }
        ("GET", path) if path.starts_with("/api/album/") => {
            let ids = state
                .lists
                .lock()
                .unwrap()
                .get(path)
                .cloned()
                .unwrap_or_default();
            json(
                200,
                format!(
                    r#"{{"album":{{"id":"al1","deezer_id":"al1","title":"Album","cover":null}},"tracks":{}}}"#,
                    tracks_json(&ids)
                        .trim_start_matches(r#"{"tracks":"#)
                        .trim_end_matches('}')
                ),
            )
        }
        ("GET", path) if path.starts_with("/api/playlist/") => {
            let ids = state
                .lists
                .lock()
                .unwrap()
                .get(path)
                .cloned()
                .unwrap_or_default();
            json(
                200,
                format!(
                    r#"{{"playlist":{{"id":"p1","title":"Liste","version":1}},"tracks":{}}}"#,
                    tracks_json(&ids)
                        .trim_start_matches(r#"{"tracks":"#)
                        .trim_end_matches('}')
                ),
            )
        }
        ("GET", path) if path.starts_with("/api/radio/") => {
            let ids = state.radio.lock().unwrap().clone();
            json(200, tracks_json(&ids))
        }
        ("GET", path) if path.starts_with("/api/audio/edges/") => {
            let id = path.trim_start_matches("/api/audio/edges/");
            let edges = state
                .tracks
                .lock()
                .unwrap()
                .get(id)
                .and_then(|track| track.edges);
            match edges {
                Some((start, end, duration)) => json(
                    200,
                    format!(
                        r#"{{"ready":true,"start":{start},"end":{end},"duration":{duration},"threshold":-45.0}}"#
                    ),
                ),
                None => json(200, r#"{"ready":false}"#.to_owned()),
            }
        }
        ("GET", path) if path.starts_with("/api/stream/") && path.ends_with("/ready") => {
            let id = path
                .trim_start_matches("/api/stream/")
                .trim_end_matches("/ready");
            let known = state.tracks.lock().unwrap().contains_key(id);
            let quality = param("q").unwrap_or_else(|| "FLAC".to_owned());
            let _ = quality;
            if known {
                // A finished file: a length, byte ranges. `offset` is what the
                // server says when it must ENCODE from a time, so a plain FLAC
                // answers false — the same answer `stream_ready` gives.
                json(
                    200,
                    r#"{"ready":true,"ranges":"bytes","cached":false,"offset":false,"streaming":false}"#
                        .to_owned(),
                )
            } else {
                json(
                    200,
                    r#"{"ready":false,"cached":false,"offset":false,"streaming":true}"#.to_owned(),
                )
            }
        }
        ("GET", path) if path.starts_with("/api/stream/") => {
            let id = path.trim_start_matches("/api/stream/");
            let served = {
                let tracks = state.tracks.lock().unwrap();
                tracks.get(id).map(|track| Served {
                    bytes: track.bytes.clone(),
                    edges: track.edges,
                    throttle: track.throttle,
                })
            };
            let Some(served) = served else {
                return json(404, r#"{"error":"not found"}"#.to_owned());
            };
            stream_body(&served, headers, param("start"), state)
        }
        _ => json(404, r#"{"error":"not found"}"#.to_owned()),
    }
}

/// The `/api/stream` contract, as `neezer/webui/__init__.py` writes it: a
/// finished file answers `206` for a byte range, a `?start=` slice is generated
/// (`Accept-Ranges: none`, no length, `X-NS-Start`).
fn stream_body(
    served: &Served,
    headers: &[(String, String)],
    start: Option<String>,
    state: &Arc<BenchState>,
) -> Answer {
    let mut bytes = served.bytes.clone();
    if let Some(start) = start.as_deref().and_then(|value| value.parse::<f64>().ok()) {
        // 48 kHz, 2 channels, 16-bit FLAC: the slice starts where the server
        // says it did, and says where.
        let per_second = 48_000.0 * 2.0 * 2.0;
        let from = ((start * per_second) as usize).min(bytes.len());
        bytes = bytes[from..].to_vec();
        return Answer::Reply(
            200,
            format!("Content-Type: audio/flac\r\nAccept-Ranges: none\r\nX-NS-Start: {start:.3}"),
            bytes,
        );
    }
    if let Some(range) = headers
        .iter()
        .find(|(name, _)| name == "range")
        .map(|(_, value)| value.clone())
    {
        let from: usize = range
            .trim_start_matches("bytes=")
            .split('-')
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        let from = from.min(bytes.len());
        let slice = bytes[from..].to_vec();
        return Answer::Reply(
            206,
            format!(
                "Content-Type: audio/flac\r\nAccept-Ranges: bytes\r\nContent-Range: bytes {from}-{}/{}",
                bytes.len().saturating_sub(1),
                bytes.len()
            ),
            slice,
        );
    }
    let _ = state;
    Answer::Body {
        status: 200,
        extra: "Content-Type: audio/flac\r\nAccept-Ranges: bytes".to_owned(),
        bytes,
        throttle: served.throttle,
    }
}

// --- a signed-in core -------------------------------------------------------

/// A core configured for `base` and signed in as the bench's own user.
pub fn core_for(dir: &TempDir, base: &str) -> Core {
    let core = dir.core();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a setup runtime");
    runtime.block_on(async {
        core.set_server(base)
            .await
            .expect("the server answers /version");
        core.login("bench", "Bench1")
            .await
            .expect("the bench login");
    });
    core
}

/// A player over `dir`, signed in against `base`.
pub fn player_for(dir: &TempDir, base: &str) -> Arc<Player> {
    let core = core_for(dir, base);
    Player::open(Arc::new(core)).expect("the player starts")
}

/// The core behind a player, for the tests that change the link.
pub fn core_and_player(dir: &TempDir, base: &str) -> (Arc<Core>, Arc<Player>) {
    let core = Arc::new(core_for(dir, base));
    let player = Player::open(Arc::clone(&core)).expect("the player starts");
    (core, player)
}

// --- pumping the sink -------------------------------------------------------

/// Render `seconds` of audio in real time (10 ms buffers), collecting the
/// interleaved stereo frames: what a device's callback does.
pub fn pump(sink: &mut nz_audio::ExternalSink, seconds: f64) -> Vec<f32> {
    pump_buffers(sink, (seconds / 0.010).round().max(1.0) as usize)
}

/// The same, `buffers` times, so a caller can check a condition between
/// buffers instead of guessing a duration.
pub fn pump_buffers(sink: &mut nz_audio::ExternalSink, buffers: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(buffers * 480 * 2);
    let mut next = Instant::now();
    for _ in 0..buffers {
        let mut buffer = vec![0.0f32; 480 * 2];
        sink.render(&mut buffer);
        out.extend_from_slice(&buffer);
        next += Duration::from_millis(10);
        let now = Instant::now();
        if next > now {
            thread::sleep(next - now);
        } else {
            next = now;
        }
    }
    out
}

/// Pumps until `predicate` holds on the PLAYER's own snapshot, or panics with
/// the last one. 10 ms buffers, real time, like a device: the condition is the
/// event, the duration only a failure bound.
pub fn pump_until(
    sink: &mut nz_audio::ExternalSink,
    player: &Player,
    seconds: f64,
    mut predicate: impl FnMut(&crate::player::Snapshot) -> bool,
) -> crate::player::Snapshot {
    let buffers = (seconds / 0.010).round().max(1.0) as usize;
    for _ in 0..buffers {
        let mut buffer = vec![0.0f32; 480 * 2];
        sink.render(&mut buffer);
        let snapshot = player.snapshot();
        if predicate(&snapshot) {
            return snapshot;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!(
        "the state never arrived; last snapshot: {:?}",
        player.snapshot()
    );
}

/// Pumps until the ENGINE's own clock says audio is flowing (`seconds` in) —
/// the honest "it is playing" when the state is optimistic by design — or
/// panics with the snapshot.
pub fn pump_until_audio(
    sink: &mut nz_audio::ExternalSink,
    player: &Player,
    seconds: f64,
    after_s: f64,
) -> crate::player::Snapshot {
    let buffers = (seconds / 0.010).round().max(1.0) as usize;
    for _ in 0..buffers {
        let mut buffer = vec![0.0f32; 480 * 2];
        sink.render(&mut buffer);
        if player.position().position_s > after_s {
            return player.snapshot();
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!(
        "no audio after {seconds} s; snapshot: {:?}, position {:?}",
        player.snapshot(),
        player.position()
    );
}

/// The peak of a slice of interleaved stereo frames.
pub fn peak(frames: &[f32]) -> f32 {
    frames
        .iter()
        .fold(0.0f32, |max, sample| max.max(sample.abs()))
}

/// The power of a window, in dB relative to full scale: what the crossfade's
/// own test measures (±0.5 dB).
pub fn power_db(frames: &[f32]) -> f64 {
    if frames.is_empty() {
        return f64::NEG_INFINITY;
    }
    let sum: f64 = frames
        .iter()
        .map(|sample| f64::from(*sample) * f64::from(*sample))
        .sum();
    let mean = sum / frames.len() as f64;
    10.0 * (mean.max(1e-12)).log10()
}

// --- fixtures ---------------------------------------------------------------
//
// Real audio, COMMITTED and small (`include_bytes!`): the mobile CI's Rust job
// installs no ffmpeg, and a test that silently skips its own assertion is worse
// than no test. The files are 48 kHz — the engine's own rate — on purpose: at
// any other rate EVERY voice goes through the resampler, and the resampler
// restarts at a track change, so the gapless seam would be off by the filter's
// own delay (measured: 0.25 at 24 kHz). Made once with ffmpeg from a `lavfi`
// expression:
//
//   aevalsrc=0.5*cos(2*PI*440*t):s=48000:d=2:c=stereo   -- `whole`, and the two
//   halves cut from it at exactly one second (`first`, `second`)
//   ...d=3 at 440 and 610 Hz                             -- `tone440`, `tone610`
//   ...d=2 + `adelay=500|500,apad=pad_dur=3`             -- `padded` (5.5 s)
//   anullsrc=r=48000:cl=stereo, -t 40 / -t 8             -- `silence40`, `silence8`
//
// A cosine (not a sine) starts at +amplitude, so a missing first sample shows;
// 440 Hz over 2 s is a whole number of periods, so the seam is continuous by
// construction and any dropped or repeated frame is a visible step.

/// The committed fixtures this file's tests use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fixture {
    /// 2 s of a 440 Hz cosine.
    Whole,
    /// The first second of it.
    First,
    /// The second second of it.
    Second,
    /// 5.5 s: 0.5 s of silence, 2 s of a 440 Hz tone, 3 s of silence.
    Padded,
    /// 3 s at 440 Hz.
    Tone440,
    /// 3 s at 610 Hz.
    Tone610,
    /// 40 s of silence: an episode long enough to have a position worth keeping.
    Silence40,
    /// 8 s of silence: a finished file to seek in.
    Silence8,
}

impl Fixture {
    fn bytes(self) -> &'static [u8] {
        match self {
            Fixture::Whole => include_bytes!("fixtures/whole.flac"),
            Fixture::First => include_bytes!("fixtures/first.flac"),
            Fixture::Second => include_bytes!("fixtures/second.flac"),
            Fixture::Padded => include_bytes!("fixtures/padded.flac"),
            Fixture::Tone440 => include_bytes!("fixtures/tone440.flac"),
            Fixture::Tone610 => include_bytes!("fixtures/tone610.flac"),
            Fixture::Silence40 => include_bytes!("fixtures/silence40.flac"),
            Fixture::Silence8 => include_bytes!("fixtures/silence8.flac"),
        }
    }

    /// The fixture's own length, in seconds, as its metadata says it: the trim's
    /// plausibility check compares the server's measurement to this.
    pub fn seconds(self) -> u32 {
        match self {
            Fixture::Whole => 2,
            Fixture::First | Fixture::Second => 1,
            Fixture::Padded => 5,
            Fixture::Tone440 | Fixture::Tone610 => 3,
            Fixture::Silence40 => 40,
            Fixture::Silence8 => 8,
        }
    }

    /// Writes it under `dir` and hands the path back (each test owns its root,
    /// so two tests never share a file).
    pub fn write(self, dir: &Path) -> PathBuf {
        let name = match self {
            Fixture::Whole => "whole.flac",
            Fixture::First => "first.flac",
            Fixture::Second => "second.flac",
            Fixture::Padded => "padded.flac",
            Fixture::Tone440 => "tone440.flac",
            Fixture::Tone610 => "tone610.flac",
            Fixture::Silence40 => "silence40.flac",
            Fixture::Silence8 => "silence8.flac",
        };
        let path = dir.join(name);
        std::fs::write(&path, self.bytes()).expect("the fixture is written");
        path
    }
}

/// Decodes a FLAC through a player of its own, so a reference is the file's own
/// samples and not a second code path's idea of them. The pumping is IDENTICAL
/// to the caller's (the same wait for the first audible frames, the same
/// collection window), which is what makes the comparison sample-aligned.
pub fn reference_frames(path: &Path, seconds: f64) -> Vec<f32> {
    let dir = TempDir::new("reference");
    let bench = Bench::start();
    bench.track("ref", path);
    let core = Arc::new(core_for(&dir, bench.base()));
    let player = Player::open(core).expect("the reference player");
    let (_, mut sink) = player.take_sink().expect("the reference sink");
    player.command(
        crate::player::Command::PlayQueue {
            tracks: vec![track_ref_of("ref", 2)],
            start: 0,
            context: None,
        },
        1,
    );
    let _ = pump_until_audio(&mut sink, &player, 30.0, 0.05);
    let frames = pump(&mut sink, seconds);
    player.shutdown();
    frames
}

/// A `TrackRef` for a fixture id, the way a list screen builds one.
pub fn track_ref(id: &str) -> crate::player::TrackRef {
    crate::player::TrackRef {
        key: id.to_owned(),
        id: id.to_owned(),
        title: format!("Titre {id}"),
        artists: "Artiste".to_owned(),
        album: "Album".to_owned(),
        album_id: Some("al1".to_owned()),
        artist_id: Some("a1".to_owned()),
        cover: None,
        duration_s: 200,
        episode: false,
        liked: false,
    }
}

/// A `TrackRef` for a fixture whose real length is `seconds`: the trim's own
/// plausibility check compares the server's measurement to the track's metadata
/// (`EDGE_LENGTH_SLACK`), so a made-up 200 s would reject a real bound.
pub fn track_ref_of(id: &str, seconds: u32) -> crate::player::TrackRef {
    crate::player::TrackRef {
        duration_s: seconds,
        ..track_ref(id)
    }
}

/// A `TrackRef` for an episode (no crossfade, the podcast speed applies).
pub fn episode_ref(id: &str, duration_s: u32) -> crate::player::TrackRef {
    crate::player::TrackRef {
        duration_s,
        episode: true,
        ..track_ref(id)
    }
}

/// The `PlayerError` a failed start gives, for a test that expects one.
#[allow(dead_code)]
pub fn expect_error(result: Result<Arc<Player>, PlayerError>) -> String {
    match result {
        Ok(_) => panic!("the player started"),
        Err(error) => error.to_string(),
    }
}

/// The player's snapshot, with the pump's channel unused: what a test that only
/// sends commands reads.
pub fn wait_state(
    player: &Player,
    seconds: f64,
    mut predicate: impl FnMut(&crate::player::Snapshot) -> bool,
) -> crate::player::Snapshot {
    let deadline = Instant::now() + Duration::from_secs_f64(seconds);
    while Instant::now() < deadline {
        let snapshot = player.snapshot();
        if predicate(&snapshot) {
            return snapshot;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("the state never arrived; last: {:?}", player.snapshot());
}

/// A channel-fed timer for tests that must know a request LANDED (the server
/// counts and the count is what they wait on).
pub fn wait_requests(bench: &Bench, count: usize, seconds: f64) {
    let deadline = Instant::now() + Duration::from_secs_f64(seconds);
    while Instant::now() < deadline {
        if bench.requests() >= count {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

/// A latch a listener opens: tests wait on the EVENT, never on a duration.
#[derive(Default)]
pub struct Latch {
    open: AtomicBool,
    sender: Mutex<Option<mpsc::Sender<()>>>,
    receiver: Mutex<Option<mpsc::Receiver<()>>>,
}

impl Latch {
    pub fn new() -> Latch {
        let (sender, receiver) = mpsc::channel();
        Latch {
            open: AtomicBool::new(false),
            sender: Mutex::new(Some(sender)),
            receiver: Mutex::new(Some(receiver)),
        }
    }

    pub fn open(&self) {
        self.open.store(true, Ordering::SeqCst);
        if let Some(sender) = self.sender.lock().unwrap().take() {
            let _ = sender.send(());
        }
    }

    pub fn wait(&self, seconds: f64) -> bool {
        if self.open.load(Ordering::SeqCst) {
            return true;
        }
        let Some(receiver) = self.receiver.lock().unwrap().take() else {
            return self.open.load(Ordering::SeqCst);
        };
        receiver
            .recv_timeout(Duration::from_secs_f64(seconds))
            .is_ok()
    }
}

/// Silence a `NetworkState` change needs: the core flips `nz-api` offline at
/// once, so no request waits a timeout to learn it.
pub fn offline(core: &Core) {
    core.set_network(NetworkState::None);
}

pub fn online(core: &Core) {
    core.set_network(NetworkState::Wifi { metered: false });
}
