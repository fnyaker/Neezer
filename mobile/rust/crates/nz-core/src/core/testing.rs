//! Test support for MB2's own files: throwaway directories, a server to talk
//! to, and the two counters a test asserts on (requests and SOCKETS).
//!
//! **Two servers, on purpose.** `tools/perf_api.py --serve` is the real one and
//! what the PR's numbers come from; the `Fake` is a bare HTTP server in this
//! process, used when no interpreter can `import neezer` (the mobile CI's Rust
//! job installs no Python) and for the one thing perf_api cannot do on demand:
//! refuse a cookie that was valid a moment ago (its session is a SIGNED cookie,
//! not a server-side record). Both speak the same wire (`/api/version`,
//! `/api/login`, `/api/me`, `/api/logout`, `/api/cover/<id>`, `Set-Cookie`,
//! `401`), so a test reads the same either way.
//!
//! **Nothing waits on the clock.** The bench is ready when it prints `READY` (a
//! line on a pipe), a refusal is read from the answer, and a socket count is
//! read after a PROBE connection has come back through a channel — no `sleep`
//! anywhere an assertion depends on.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use super::{Core, CoreConfig};

// --- a throwaway data + cache directory ------------------------------------

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

/// One test's two roots, removed on drop. Tiny by nature (a few kB of SQLite
/// and the covers a test fetched): the bench libraries are the big ones and
/// `perf_api.py` cleans those up itself.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let number = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("nz-core-{}-{tag}-{number}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("data")).expect("data dir");
        std::fs::create_dir_all(path.join("cache")).expect("cache dir");
        TempDir { path }
    }

    /// A core over this directory, built the way `NzBridge.install` builds it.
    pub fn core(&self, device: &str) -> Core {
        Core::new(CoreConfig {
            data_dir: self.path.join("data"),
            cache_dir: self.path.join("cache"),
            app_version: "0.0.0-test".to_owned(),
            version_code: 2_000_001,
            sdk_int: 35,
            device_name: device.to_owned(),
        })
        .expect("a core over a throwaway directory")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

// --- a listener that counts the sockets it is offered ----------------------

/// A port that answers nothing and COUNTS every connection it accepts — the
/// "0 sockets opened" of the acceptance, measured at the peer rather than
/// inferred from a duration.
pub struct CountingListener {
    address: String,
    accepted: Arc<AtomicUsize>,
    /// How many of those accepts were this harness's own probes, so the answer
    /// a test reads counts only the connections the CORE opened.
    probes: AtomicUsize,
    stop: Arc<AtomicBool>,
    probe: Arc<Mutex<Option<mpsc::Sender<usize>>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl CountingListener {
    pub fn start() -> CountingListener {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a counting port");
        let address = format!("http://{}", listener.local_addr().expect("its address"));
        let accepted = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let probe = Arc::new(Mutex::new(None::<mpsc::Sender<usize>>));
        let counter = Arc::clone(&accepted);
        let stopping = Arc::clone(&stop);
        let probe_for_thread = Arc::clone(&probe);
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let count = counter.fetch_add(1, Ordering::SeqCst) + 1;
                if let Ok(slot) = probe_for_thread.lock()
                    && let Some(sender) = slot.as_ref()
                {
                    let _ = sender.send(count);
                }
                drop(stream);
                if stopping.load(Ordering::SeqCst) {
                    break;
                }
            }
        });
        CountingListener {
            address,
            accepted,
            probes: AtomicUsize::new(0),
            stop,
            probe,
            thread: Some(thread),
        }
    }

    /// `http://127.0.0.1:<port>`, the origin a core may be configured with.
    pub fn address(&self) -> &str {
        &self.address
    }

    /// How many connections the peer accepted so far.
    pub fn accepted(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }

    /// Waits until the accept loop has caught up with every connection offered
    /// BEFORE this call, and answers how many that was. One probe connection is
    /// made and its own accept is the signal (connections are accepted in
    /// order, by one thread), so the answer never counts the probe itself.
    pub fn settled(&self) -> usize {
        let probes = self.probes.load(Ordering::SeqCst);
        let before = self.accepted() - probes;
        self.probes.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = mpsc::channel();
        *self.probe.lock().unwrap_or_else(|err| err.into_inner()) = Some(sender);
        let _ = TcpStream::connect(self.address.trim_start_matches("http://"));
        while let Ok(count) = receiver.recv() {
            if count > before {
                break;
            }
        }
        *self.probe.lock().unwrap_or_else(|err| err.into_inner()) = None;
        before
    }
}

impl Drop for CountingListener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the blocking `accept`: one connection to ourselves.
        let _ = TcpStream::connect(self.address.trim_start_matches("http://"));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

// --- the session listener a test records ----------------------------------

pub mod sink {
    use super::super::{SessionEvent, SessionSink};
    use std::sync::mpsc;

    /// Keeps what it heard, so a test waits on the event itself.
    pub struct Recorder(mpsc::Sender<SessionEvent>);

    impl Recorder {
        pub fn new() -> (Recorder, mpsc::Receiver<SessionEvent>) {
            let (sender, receiver) = mpsc::channel();
            (Recorder(sender), receiver)
        }
    }

    impl SessionSink for Recorder {
        fn publish(&self, event: SessionEvent) {
            let _ = self.0.send(event);
        }
    }
}

// --- the bench -------------------------------------------------------------

/// The server a test talks to: the real `perf_api.py` when an interpreter can
/// import it, the in-process fake otherwise.
pub enum Bench {
    Real(Box<RealBench>),
    Fake(FakeBench),
}

impl Bench {
    /// The real server when it is runnable, the fake otherwise.
    pub fn start(cover: bool) -> Bench {
        match python() {
            Some(python) => Bench::Real(Box::new(RealBench::start(&python, cover))),
            None => Bench::Fake(FakeBench::start()),
        }
    }

    /// The controlled server: the only one that can refuse a cookie on demand.
    pub fn fake() -> Bench {
        Bench::Fake(FakeBench::start())
    }

    pub fn base(&self) -> String {
        match self {
            Bench::Real(real) => real.base(),
            Bench::Fake(fake) => fake.base(),
        }
    }

    /// The bytes the server's `GET /api/version` body carries, measured on the
    /// wire by the test itself.
    pub fn version_body_len(&self) -> u64 {
        match self {
            Bench::Real(real) => real.version_body_len(),
            Bench::Fake(fake) => fake.version_body_len(),
        }
    }

    /// An absolute cover URL of an album the server has art for, and the exact
    /// number of bytes that URL sends — the number the counters are held to.
    ///
    /// The URL is the one a SCREEN would draw: the test logs in with its own
    /// client, reads the favourites and takes the first card's `album.cover`
    /// path, so nothing here guesses at a route.
    pub async fn cover(&self) -> (String, usize) {
        match self {
            Bench::Real(real) => real.cover().await,
            Bench::Fake(fake) => (fake.cover_url(), fake.cover_body_len()),
        }
    }

    /// Makes the server refuse the cookie it accepted a moment ago.
    pub fn invalidate_session(&self) {
        match self {
            Bench::Real(_) => panic!(
                "perf_api.py's session is a signed cookie and cannot be expired on demand: \
                 this test needs the controlled server"
            ),
            Bench::Fake(fake) => fake.invalidate_session(),
        }
    }
}

/// `tools/perf_api.py --serve` on a throwaway library.
pub struct RealBench {
    child: Child,
    port: u16,
}

impl RealBench {
    fn start(python: &str, cover: bool) -> RealBench {
        let mut arguments: Vec<String> = [
            // Small on purpose: what this proves is the SESSION, the counters
            // and the cover path, not the size of a library (the list tests
            // build their 8 000 rows in Rust).
            "--tracks",
            "24",
            "--playlists",
            "2",
            "--favorites",
            "4",
            "--big-playlist",
            "4",
            "--serve",
            "--port",
            "0",
        ]
        .iter()
        .map(|argument| (*argument).to_owned())
        .collect();
        let cover_path = repo_root().join("tests/assets/cover.jpg");
        if cover {
            // Without a real JPEG every `/api/cover` is a 404 and the test would
            // measure a placeholder.
            arguments.push("--covers".to_owned());
            arguments.push(cover_path.display().to_string());
        }
        let mut child = Command::new(python)
            .arg(repo_root().join("tools/perf_api.py"))
            .args(&arguments)
            .current_dir(repo_root())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .stdin(Stdio::null())
            .spawn()
            .expect("start perf_api.py");

        let stdout = child.stdout.take().expect("its stdout");
        let (sender, receiver) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let _ = sender.send(line);
            }
        });
        // The READY line, on the pipe: the event that says the library is
        // filled, never a duration. Its ids payload is not needed here — the
        // cover URL is discovered from the server itself, as a screen would.
        let mut port = None;
        while let Ok(line) = receiver.recv() {
            if let Some(rest) = line.strip_prefix("READY ") {
                port = rest
                    .split_whitespace()
                    .next()
                    .and_then(|port| port.parse::<u16>().ok());
                if port.is_some() {
                    break;
                }
            }
        }
        let port = port.expect("perf_api.py never printed READY");
        RealBench { child, port }
    }

    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn version_body_len(&self) -> u64 {
        raw_get(&self.base(), "/api/version")
            .map(|(_, body)| body.len() as u64)
            .unwrap_or(0)
    }

    async fn cover(&self) -> (String, usize) {
        let api = nz_api::Api::new(self.base()).expect("a client for the bench");
        api.login("bench", "Bench1").await.expect("the bench login");
        let favorites: serde_json::Value = api
            .get(nz_api::Lane::User, "/me/favorites")
            .await
            .expect("the favourites")
            .value;
        // `tracks[].album.cover` is `/api/cover/<album uuid>` — the SPA's own
        // cover URL for a card, which is what a screen hands `cover_path`.
        let path = favorites["tracks"]
            .get(0)
            .and_then(|track| track["album"]["cover"].as_str())
            .unwrap_or_else(|| panic!("no cover on the first favourite: {favorites}"));
        let lane_path = path.strip_prefix("/api").unwrap_or(path);
        let bytes = api
            .bytes(nz_api::Lane::User, lane_path)
            .await
            .expect("the cover bytes");
        (format!("{}{}", self.base(), path), bytes.len())
    }
}

impl Drop for RealBench {
    fn drop(&mut self) {
        stop(&mut self.child);
    }
}

/// The controlled server: the same routes, written with `std`'s sockets, plus
/// the one power perf_api's signed cookie does not have.
pub struct FakeBench {
    address: String,
    state: Arc<FakeState>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// The version body the fake sends: a real `nz_model::Version` shape, so the
/// core's own deserialization is what is under test.
const FAKE_VERSION_BODY: &str = r#"{"version":"9.9.9","build":"fake"}"#;
const FAKE_ME_BODY: &str = r#"{"user":{"id":"u1","name":"bench","admin":true,"onboarding":"done","access":{},"sources":[]}}"#;
const FAKE_COVER: &[u8] = include_bytes!("../../../../../../tests/assets/cover.jpg");

struct FakeState {
    /// Whether the cookie the client holds is still accepted.
    session_valid: AtomicBool,
}

impl FakeBench {
    pub fn start() -> FakeBench {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a fake server");
        let address = format!("http://{}", listener.local_addr().expect("its address"));
        let state = Arc::new(FakeState {
            session_valid: AtomicBool::new(true),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::clone(&state);
        let stopping = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                serve_fake(&mut stream, &shared);
                if stopping.load(Ordering::SeqCst) {
                    break;
                }
            }
        });
        FakeBench {
            address,
            state,
            stop,
            thread: Some(thread),
        }
    }

    pub fn base(&self) -> String {
        self.address.clone()
    }

    fn version_body_len(&self) -> u64 {
        FAKE_VERSION_BODY.len() as u64
    }

    fn cover_url(&self) -> String {
        format!("{}/api/cover/album-1", self.address)
    }

    fn cover_body_len(&self) -> usize {
        FAKE_COVER.len()
    }

    fn invalidate_session(&self) {
        self.state.session_valid.store(false, Ordering::SeqCst);
    }
}

impl Drop for FakeBench {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.address.trim_start_matches("http://"));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// One request, one answer, one closed connection (reqwest reopens).
fn serve_fake(stream: &mut TcpStream, state: &FakeState) {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    while !buffer.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
        }
    }
    // The request may arrive with its body in the same read as its head: what
    // is after the blank line is the body's first bytes.
    let split = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
        .unwrap_or(buffer.len());
    let carried: Vec<u8> = buffer[split..].to_vec();
    let head = String::from_utf8_lossy(&buffer[..split.min(buffer.len())]).into_owned();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    let length: usize = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    let mut body = carried;
    while body.len() < length {
        let mut rest = vec![0u8; length - body.len()];
        match stream.read(&mut rest) {
            Ok(0) | Err(_) => break,
            Ok(read) => body.extend_from_slice(&rest[..read]),
        }
    }

    let cookie = headers
        .iter()
        .find(|(name, _)| name == "cookie")
        .map(|(_, value)| value.as_str())
        .unwrap_or_default();
    let has_cookie = cookie.contains("session=good");

    let (status, extra, answer): (u16, &str, Vec<u8>) = match (method.as_str(), path.as_str()) {
        ("GET", "/api/version") => (
            200,
            "Content-Type: application/json",
            FAKE_VERSION_BODY.into(),
        ),
        ("POST", "/api/login") => {
            let text = String::from_utf8_lossy(&body);
            if text.contains("Bench1") {
                state.session_valid.store(true, Ordering::SeqCst);
                (
                    200,
                    "Content-Type: application/json\r\nSet-Cookie: session=good; Path=/",
                    FAKE_ME_BODY.into(),
                )
            } else {
                (
                    401,
                    "Content-Type: application/json",
                    r#"{"error":"unauthorized"}"#.into(),
                )
            }
        }
        ("GET", "/api/me") => {
            if has_cookie && state.session_valid.load(Ordering::SeqCst) {
                (200, "Content-Type: application/json", FAKE_ME_BODY.into())
            } else {
                (
                    401,
                    "Content-Type: application/json",
                    r#"{"error":"unauthorized"}"#.into(),
                )
            }
        }
        ("POST", "/api/logout") => {
            state.session_valid.store(false, Ordering::SeqCst);
            (200, "Content-Type: application/json", b"{}".to_vec())
        }
        ("GET", path) if path.starts_with("/api/cover/") => {
            (200, "Content-Type: image/jpeg", FAKE_COVER.to_vec())
        }
        _ => (
            404,
            "Content-Type: application/json",
            br#"{"error":"not found"}"#.to_vec(),
        ),
    };

    let head = format!(
        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\n{extra}\r\nConnection: close\r\n\r\n",
        answer.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&answer);
    let _ = stream.flush();
}

/// A raw `GET` outside `nz-api`, so a test can measure what the server really
/// sent instead of trusting the client's own counter.
fn raw_get(base: &str, path: &str) -> Option<(u16, String)> {
    let address = base.trim_start_matches("http://");
    let mut stream = TcpStream::connect(address).ok()?;
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).ok()?;
    let mut answer = String::new();
    stream.read_to_string(&mut answer).ok()?;
    let (head, body) = answer.split_once("\r\n\r\n")?;
    let status = head.split_whitespace().nth(1)?.parse().ok()?;
    Some((status, body.to_owned()))
}

/// The repository root (`mobile/rust/crates/nz-core` → up four).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../..")
        .canonicalize()
        .expect("the repository root")
}

/// The first interpreter that can `import neezer`, or `None` — in which case a
/// test uses the controlled server and says so.
fn python() -> Option<String> {
    let candidates: Vec<String> = std::env::var("NZ_PYTHON")
        .ok()
        .into_iter()
        .chain([
            repo_root().join(".venv/bin/python").display().to_string(),
            repo_root().join("venv/bin/python").display().to_string(),
            "python3".to_owned(),
        ])
        .collect();
    for candidate in candidates {
        let usable = Command::new(&candidate)
            .args(["-c", "import peewee, neezer"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if usable {
            return Some(candidate);
        }
    }
    eprintln!(
        "nz-core: no interpreter can `import peewee, neezer` — the session tests run against \
         the controlled server (install the repo's `.venv` to exercise tools/perf_api.py)"
    );
    None
}

/// Stop a bench the way `perf_api.py` expects: SIGTERM first (it removes its
/// throwaway library), SIGKILL only as a last resort. Never `pkill`: this kills
/// the child this struct owns, by pid.
///
/// SIGTERM is sent through `kill(1)` rather than a `libc` binding: `nz-core`
/// forbids `unsafe`, and a test has no reason to lift that for one signal.
fn stop(child: &mut Child) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    let _ = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
}
