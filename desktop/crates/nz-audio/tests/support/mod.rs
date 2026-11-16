//! Test support for `nz-audio`: real audio, and a local server that speaks the
//! server's own `/api/stream` contract.
//!
//! Nothing here is mocked at the audio level: files are made by ffmpeg (FLAC,
//! Ogg Opus), and the HTTP routes reproduce `neezer/webui/__init__.py`'s
//! `stream` and `stream_ready` headers — `Accept-Ranges: none` and no length for
//! a body generated as it is sent, `Range`/`206` for a finished file,
//! `X-NS-Start` on a `?start=` slice, and `409 + Retry-After` semantics left to
//! the caller's `ready` value.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use nz_audio::{ByteStream, EngineEvent, Lane, Quality, SourceError, StreamOpener, StreamReady};

/// A scratch directory per test, emptied on creation.
pub fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("nz-audio-tests-{}", std::process::id()))
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch directory");
    dir
}

fn ffmpeg(args: &[&str]) {
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .status()
        .expect("ffmpeg is installed (the desktop CI installs it; see the workflow)");
    assert!(status.success(), "ffmpeg {args:?} failed");
}

/// A FLAC of a cosine at `freq` Hz: the first sample is +amplitude (a sine
/// starts at 0, which would hide a missing first sample).
pub fn cosine_flac(path: &Path, freq: f64, seconds: f64, rate: u32) {
    let expression = format!("0.5*cos(2*PI*{freq}*t)");
    let source = format!("aevalsrc={expression}:s={rate}:d={seconds}:c=stereo");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &source,
        "-c:a",
        "flac",
        path.to_str().unwrap(),
    ]);
}

/// A FLAC of a constant 0.25: every slice of it is the same signal, so a seek
/// whose landing can only be checked by ear is still a valid fixture.
pub fn dc_flac(path: &Path, seconds: f64, rate: u32) {
    let source = format!("aevalsrc=0.25:s={rate}:d={seconds}:c=stereo");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &source,
        "-c:a",
        "flac",
        path.to_str().unwrap(),
    ]);
}

/// Two FLACs cut out of ONE two-second cosine, at exactly one second: the
/// second file's first sample is the waveform's natural continuation, which is
/// what "gapless" has to preserve. Returns the path of the whole file too, as
/// the reference.
pub fn split_cosine(dir: &Path, freq: f64, rate: u32) -> (PathBuf, PathBuf, PathBuf) {
    let whole = dir.join("whole.flac");
    cosine_flac(&whole, freq, 2.0, rate);
    let first = dir.join("first.flac");
    let second = dir.join("second.flac");
    ffmpeg(&[
        "-i",
        whole.to_str().unwrap(),
        "-af",
        "atrim=start=0:end=1",
        "-c:a",
        "flac",
        first.to_str().unwrap(),
    ]);
    ffmpeg(&[
        "-i",
        whole.to_str().unwrap(),
        "-af",
        "atrim=start=1:end=2",
        "-c:a",
        "flac",
        second.to_str().unwrap(),
    ]);
    (first, second, whole)
}

/// The same audio as Ogg Opus 128 kbit/s: the encoder writes a pre-skip and a
/// final granule, and a decoder that ignores them is audibly wrong at both ends.
pub fn opus_from_flac(flac: &Path, opus: &Path) {
    ffmpeg(&[
        "-i",
        flac.to_str().unwrap(),
        "-c:a",
        "libopus",
        "-b:a",
        "128k",
        opus.to_str().unwrap(),
    ]);
}

/// The decoded length of a file, through ffprobe, as a cross-check of what the
/// engine produced. FLAC is exact; Opus reports the container's duration.
pub fn probe_duration_ms(path: &Path) -> f64 {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=duration",
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(path)
        .output()
        .expect("ffprobe");
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .unwrap_or(0.0)
        * 1000.0
}

/// A request the test server saw, so a test can assert the exact wire contract
/// (`?start=` on a generated stream, `Range` on a finished file).
#[derive(Clone, Debug)]
pub struct Recorded {
    pub path: String,
    pub headers: Vec<(String, String)>,
}

impl Recorded {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// How the server answers `/api/stream/<id>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Body {
    /// A finished file: `Content-Length`, `Accept-Ranges: bytes`, `Range` works.
    Finished,
    /// A body generated as it is sent: no length, no ranges, `X-NS-Start`.
    Generated,
}

#[derive(Default)]
pub struct ServerState {
    pub ready: Mutex<StreamReady>,
    pub requests: Mutex<Vec<Recorded>>,
    /// Bytes of a generated body actually written: a prefetch that runs away
    /// with the link shows up here.
    pub sent: AtomicU64,
}

/// A loopback server for one track.
pub struct TestServer {
    pub url: String,
    pub state: Arc<ServerState>,
    stop: Arc<Mutex<bool>>,
    join: Option<thread::JoinHandle<()>>,
}

impl TestServer {
    pub fn start(file: PathBuf, ready: StreamReady, body: Body) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        let state = Arc::new(ServerState {
            ready: Mutex::new(ready),
            ..ServerState::default()
        });
        let stop = Arc::new(Mutex::new(false));
        let stopped = Arc::clone(&stop);
        let shared = Arc::clone(&state);
        let join = thread::spawn(move || {
            for incoming in listener.incoming() {
                if *stopped.lock().unwrap() {
                    return;
                }
                let Ok(stream) = incoming else { continue };
                let shared = Arc::clone(&shared);
                let file = file.clone();
                thread::spawn(move || {
                    let _ = serve(stream, &file, body, &shared);
                });
            }
        });
        Self {
            url,
            state,
            stop,
            join: Some(join),
        }
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.state.requests.lock().unwrap().clone()
    }

    pub fn set_ready(&self, ready: StreamReady) {
        *self.state.ready.lock().unwrap() = ready;
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        *self.stop.lock().unwrap() = true;
        // Unblock the accept loop with one connection to ourselves.
        if let Some(rest) = self.url.strip_prefix("http://")
            && let Ok(address) = rest.parse::<std::net::SocketAddr>()
        {
            let _ = TcpStream::connect_timeout(&address, Duration::from_millis(100));
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn serve(
    stream: TcpStream,
    file: &Path,
    body: Body,
    state: &Arc<ServerState>,
) -> std::io::Result<()> {
    stream.set_nodelay(true)?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    loop {
        let mut request_line = String::new();
        if reader.read_line(&mut request_line)? == 0 {
            return Ok(());
        }
        let request_line = request_line.trim_end().to_string();
        if request_line.is_empty() {
            return Ok(());
        }
        let mut headers = Vec::new();
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
                headers.push((name.trim().to_string(), value.trim().to_string()));
            }
        }
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or("GET");
        let target = parts.next().unwrap_or("/");
        state.requests.lock().unwrap().push(Recorded {
            path: target.to_string(),
            headers: headers.clone(),
        });
        let header = |name: &str| {
            headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.clone())
        };
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        if path.ends_with("/ready") {
            let ready = state.ready.lock().unwrap().clone();
            let body = format!(
                "{{\"ready\":{},\"ranges\":{},\"bytes\":{},\"cached\":{},\"offset\":{},\"streaming\":{}}}",
                ready.ready,
                match ready.ranges.as_deref() {
                    Some(ranges) => format!("\"{ranges}\""),
                    None => "null".into(),
                },
                match ready.bytes {
                    Some(bytes) => bytes.to_string(),
                    None => "null".into(),
                },
                ready.cached,
                ready.offset,
                ready.streaming
            );
            write!(
                writer,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{}",
                body.len(),
                body
            )?;
            writer.flush()?;
            continue;
        }
        let start = query
            .split('&')
            .find_map(|pair| pair.strip_prefix("start="))
            .and_then(|value| value.parse::<f64>().ok());
        let range_from = header("Range").and_then(|value| {
            value
                .strip_prefix("bytes=")
                .and_then(|rest| rest.split('-').next())
                .and_then(|first| first.parse::<u64>().ok())
        });
        let mut bytes = std::fs::read(file)?;
        let total = bytes.len() as u64;
        if let Some(from) = range_from {
            let from = (from as usize).min(bytes.len());
            bytes.drain(..from);
            write!(
                writer,
                "HTTP/1.1 206 Partial Content\r\nContent-Type: audio/flac\r\nAccept-Ranges: bytes\r\nContent-Range: bytes {}-{}/{}\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                from,
                total.saturating_sub(1),
                total,
                bytes.len()
            )?;
        } else if body == Body::Finished {
            write!(
                writer,
                "HTTP/1.1 200 OK\r\nContent-Type: audio/flac\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                bytes.len()
            )?;
        } else {
            let started = start.unwrap_or(0.0);
            write!(
                writer,
                "HTTP/1.1 200 OK\r\nContent-Type: audio/ogg\r\nAccept-Ranges: none\r\nCache-Control: no-store\r\nX-NS-Start: {started:.3}\r\nConnection: close\r\n\r\n"
            )?;
        }
        if method != "HEAD" {
            state.sent.fetch_add(bytes.len() as u64, Ordering::Relaxed);
            writer.write_all(&bytes)?;
        }
        writer.flush()?;
        if body == Body::Generated {
            return Ok(());
        }
    }
}

/// An HTTP body, read as a blocking `Read`.
struct HttpBody {
    reader: BufReader<TcpStream>,
    len: Option<u64>,
    start_s: f64,
}

impl Read for HttpBody {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.reader.read(buf)
    }
}

impl ByteStream for HttpBody {
    fn len_hint(&self) -> Option<u64> {
        self.len
    }

    fn start_offset_s(&self) -> f64 {
        self.start_s
    }
}

/// The app's side of the contract, over files and over [`TestServer`].
#[derive(Default)]
pub struct TestOpener {
    pub locals: std::collections::HashMap<String, PathBuf>,
    pub base: Option<String>,
    pub state: Option<Arc<ServerState>>,
}

impl TestOpener {
    /// `id` is played from this file, with no request at all: the play cache's
    /// answer.
    pub fn with_file(mut self, id: &str, path: impl Into<PathBuf>) -> Self {
        self.locals.insert(id.to_string(), path.into());
        self
    }

    pub fn http(server: &TestServer) -> Self {
        Self {
            locals: std::collections::HashMap::new(),
            base: Some(server.url.clone()),
            state: Some(Arc::clone(&server.state)),
        }
    }
}

impl StreamOpener for TestOpener {
    fn open(
        &self,
        id: &str,
        quality: Quality,
        start: Option<f64>,
        from_byte: Option<u64>,
        _lane: Lane,
    ) -> Result<Box<dyn ByteStream>, SourceError> {
        let base = self
            .base
            .as_ref()
            .ok_or_else(|| SourceError::Message("pas de serveur dans ce test".into()))?;
        let mut path = format!("/api/stream/{id}");
        let mut query = Vec::new();
        if let Some(q) = quality.as_query() {
            query.push(format!("q={q}"));
        }
        if let Some(start) = start {
            query.push(format!("start={start:.3}"));
        }
        if !query.is_empty() {
            path.push('?');
            path.push_str(&query.join("&"));
        }
        let address = base
            .strip_prefix("http://")
            .ok_or_else(|| SourceError::Message("url invalide".into()))?
            .parse::<std::net::SocketAddr>()
            .map_err(|e| SourceError::Message(e.to_string()))?;
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))
            .map_err(|e| SourceError::Message(format!("connexion : {e}")))?;
        let range = match from_byte {
            Some(byte) => format!("Range: bytes={byte}-\r\n"),
            None => String::new(),
        };
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: neezer.test\r\nAccept-Encoding: identity\r\n{range}Connection: close\r\n\r\n"
        )
        .map_err(|e| SourceError::Message(e.to_string()))?;
        let mut reader = BufReader::new(stream);
        let mut status = String::new();
        reader
            .read_line(&mut status)
            .map_err(|e| SourceError::Message(e.to_string()))?;
        let status_code: u16 = status
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);
        let mut len = None;
        let mut start_s = 0.0;
        loop {
            let mut line = String::new();
            reader
                .read_line(&mut line)
                .map_err(|e| SourceError::Message(e.to_string()))?;
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                let value = value.trim();
                if name.eq_ignore_ascii_case("content-length") {
                    len = value.parse::<u64>().ok();
                } else if name.eq_ignore_ascii_case("x-ns-start") {
                    start_s = value.parse::<f64>().unwrap_or(0.0);
                }
            }
        }
        if status_code >= 400 {
            return Err(SourceError::Provider {
                status: status_code,
                message: status.trim().to_string(),
            });
        }
        Ok(Box::new(HttpBody {
            reader,
            len,
            start_s,
        }))
    }

    fn ready(&self, _id: &str, _quality: Quality) -> Result<StreamReady, SourceError> {
        Ok(self
            .state
            .as_ref()
            .map(|state| state.ready.lock().unwrap().clone())
            .unwrap_or_default())
    }

    fn local(&self, id: &str, _quality: Quality) -> Option<PathBuf> {
        self.locals.get(id).cloned()
    }
}

/// Render `frames` stereo frames through the sink and return them interleaved.
pub fn render(sink: &mut nz_audio::OfflineSink, frames: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; frames * 2];
    sink.render(&mut out);
    out
}

/// Pump buffers until `predicate` matches an event, or fail with the events
/// that did arrive. 10 ms buffers, the same size the output callback gets.
pub fn wait_for(
    sink: &mut nz_audio::OfflineSink,
    events: &std::sync::mpsc::Receiver<EngineEvent>,
    timeout: Duration,
    mut predicate: impl FnMut(&EngineEvent) -> bool,
) -> EngineEvent {
    let deadline = Instant::now() + timeout;
    let mut seen = VecDeque::new();
    while Instant::now() < deadline {
        while let Ok(event) = events.try_recv() {
            seen.push_back(event.clone());
            if predicate(&event) {
                return event;
            }
        }
        render(sink, 480);
        thread::sleep(Duration::from_millis(1));
    }
    panic!("timed out after {timeout:?}; events seen: {seen:?}");
}

/// Non-blocking render for tests that only need the decoder to make progress.
pub fn pump(sink: &mut nz_audio::OfflineSink, frames: usize) {
    let mut out = vec![0.0f32; frames * 2];
    sink.render(&mut out);
}

/// Render in real time (10 ms buffers, paced), like the device would: what the
/// decoder needs in order to stay ahead instead of being raced by a test that
/// fills 72 000 frames in a microsecond.
pub fn pump_for(sink: &mut nz_audio::OfflineSink, seconds: f64) -> Vec<f32> {
    let buffers = (seconds / 0.010).round().max(1.0) as usize;
    let mut out = Vec::with_capacity(buffers * 480 * 2);
    let mut next = Instant::now();
    for _ in 0..buffers {
        out.extend(render(sink, 480));
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

/// The median of a list of durations, in milliseconds.
pub fn median_ms(mut values: Vec<f64>) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = values.len();
    if n % 2 == 1 {
        values[n / 2]
    } else {
        (values[n / 2 - 1] + values[n / 2]) / 2.0
    }
}
