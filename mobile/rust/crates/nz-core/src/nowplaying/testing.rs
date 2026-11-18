//! What this area's tests talk to: a small server on a loopback port that answers
//! the two now-playing reads with the REAL shapes, and COUNTS every request it is
//! offered.
//!
//! Why its own and not `core::testing::Bench` (MB2's): that bench answers
//! `/api/version`, the session and a cover, so these routes would be `404` there —
//! and the acceptance of this lot is about what the player's views ask. A fake
//! that speaks `/api/lyrics/<id>` and `/api/radio/track/<id>` makes the numbers
//! exact (one request for the words, one for « Titres similaires », none from the
//! copy), and needs no interpreter, so the mobile CI's Rust job (which installs no
//! Python) runs the same tests.
//!
//! It answers on a THREAD PER CONNECTION: `Core::similar_tracks` may run beside
//! the lyrics read, and a single-threaded accept loop would deadlock the test
//! rather than the code.
//!
//! Nothing waits on the clock: a test reads the request log once its `await`
//! returned, and the log is the proof.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

/// One request the server was offered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub method: String,
    /// The path with its query, as it arrived.
    pub path: String,
}

impl Hit {
    /// The path WITHOUT its query — what a route matches on.
    pub fn route(&self) -> &str {
        self.path.split('?').next().unwrap_or(&self.path)
    }
}

/// The fake server: a port, a request log, and a per-route status a test can
/// break on purpose.
pub struct NowPlayingServer {
    address: String,
    log: Arc<Mutex<Vec<Hit>>>,
    broken: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    connections: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
    accepted: Arc<AtomicUsize>,
}

impl NowPlayingServer {
    pub fn start() -> NowPlayingServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the now-playing server");
        let address = format!("http://{}", listener.local_addr().expect("its address"));
        let log = Arc::new(Mutex::new(Vec::new()));
        let broken = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let connections = Arc::new(Mutex::new(Vec::new()));
        let accepted = Arc::new(AtomicUsize::new(0));

        let shared_log = Arc::clone(&log);
        let shared_broken = Arc::clone(&broken);
        let stopping = Arc::clone(&stop);
        let shared_connections = Arc::clone(&connections);
        let counter = Arc::clone(&accepted);
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                counter.fetch_add(1, Ordering::SeqCst);
                let log = Arc::clone(&shared_log);
                let broken = Arc::clone(&shared_broken);
                let handle = std::thread::spawn(move || {
                    let Ok(mut stream) = stream.try_clone() else {
                        return;
                    };
                    serve(&mut stream, &log, &broken);
                });
                shared_connections
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .push(handle);
                if stopping.load(Ordering::SeqCst) {
                    break;
                }
            }
        });
        NowPlayingServer {
            address,
            log,
            broken,
            stop,
            thread: Some(thread),
            connections,
            accepted,
        }
    }

    /// `http://127.0.0.1:<port>` — what `set_server` is given.
    pub fn base(&self) -> String {
        self.address.clone()
    }

    pub fn hits(&self) -> Vec<Hit> {
        self.log
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    /// How many requests the screen asked for since the test's last `forget`.
    pub fn count(&self) -> usize {
        self.hits().len()
    }

    /// The routes hit, in order — the "which request" half of the measure.
    pub fn routes(&self) -> Vec<String> {
        self.hits()
            .into_iter()
            .map(|hit| format!("{} {}", hit.method, hit.route()))
            .collect()
    }

    pub fn forget(&self) {
        self.log
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clear();
    }

    /// Makes `route` answer `status` from now on (the failure tests).
    pub fn break_route(&self, route: &str, status: u16) {
        self.broken
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .push(format!("{route} {status}"));
    }

    /// How many sockets were accepted at all — the "0 sockets offline" measure.
    pub fn accepted(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }
}

impl Drop for NowPlayingServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the blocking `accept`: one connection to ourselves.
        let _ = TcpStream::connect(self.address.trim_start_matches("http://"));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let handles = {
            let mut connections = self
                .connections
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            std::mem::take(&mut *connections)
        };
        for handle in handles {
            let _ = handle.join();
        }
    }
}

/// One request, one answer, one closed connection (every client reopens).
fn serve(stream: &mut TcpStream, log: &Mutex<Vec<Hit>>, broken: &Mutex<Vec<String>>) {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 2048];
    while !buffer.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
        }
    }
    let split = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
        .unwrap_or(buffer.len());
    let head = String::from_utf8_lossy(&buffer[..split.min(buffer.len())]).into_owned();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    {
        let mut entries = log.lock().unwrap_or_else(|err| err.into_inner());
        entries.push(Hit {
            method: method.clone(),
            path: path.clone(),
        });
    }

    let route = path.split('?').next().unwrap_or(&path).to_owned();
    let broken_status = broken
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .iter()
        .find_map(|entry| {
            let (name, status) = entry.split_once(' ')?;
            (name == route).then(|| status.parse::<u16>().ok())?
        });
    let (status, answer) = match broken_status {
        Some(status) => (status, br#"{"error":"broken on purpose"}"#.to_vec()),
        None => answer_for(&method, &route),
    };
    let head = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        answer.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&answer);
    let _ = stream.flush();
}

const ME: &str = r#"{"user":{"id":"u-nowplaying","name":"Nadia","admin":false,"onboarding":"done","access":{},"sources":[]}}"#;

/// A track's words: the `.lrc` sidecar's own shape, a marker between verses
/// included (that is what a card must skip without shifting the selection).
pub const LYRICS: &str = r#"{"lyrics":{"text":"Il est cinq heures\n♪\nParis s'éveille",
 "synced":[
  {"time":0,"text":"Il est cinq heures"},
  {"time":12000,"text":"♪"},
  {"time":24000,"text":"Paris s'éveille"}],
 "source":"archive"}}"#;

/// A title the server has no words for: an answer, kept like any other.
const NO_LYRICS: &str = r#"{"lyrics":null}"#;

/// `/api/radio/track/<id>`: the seed comes back in the list, as the server does
/// (`radio.build` seeds the radio with the track itself).
const TRACK_RADIO: &str = r#"{"tracks":[
 {"id":"11111111-1111-4111-8111-111111111111","deezer_id":"9001","title":"Le chant des sirènes",
  "duration":215,"artist":{"deezer_id":"42","name":"Fauve"},
  "album":{"deezer_id":"501","title":"Vieux frères","cover":"/api/cover/501"}},
 {"id":"11111111-1111-4111-8111-222222222222","deezer_id":"9002","title":"Blizzard",
  "duration":198,"artist":{"deezer_id":"42","name":"Fauve"},
  "album":{"deezer_id":"501","title":"Vieux frères","cover":"/api/cover/501"}}]}"#;

fn answer_for(method: &str, route: &str) -> (u16, Vec<u8>) {
    let ok = |body: &str| (200, body.as_bytes().to_vec());
    match (method, route) {
        ("GET", "/api/version") => ok(r#"{"version":"9.9.9","build":"fake"}"#),
        ("POST", "/api/login") => ok(ME),
        ("GET", "/api/me") => ok(ME),
        ("GET", "/api/lyrics/9001") => ok(LYRICS),
        ("GET", "/api/lyrics/9002") => ok(NO_LYRICS),
        ("GET", "/api/radio/track/9001") => ok(TRACK_RADIO),
        ("GET", "/api/radio/track/9002") => ok(r#"{"tracks":[]}"#),
        _ => (404, br#"{"error":"not found"}"#.to_vec()),
    }
}

/// A throwaway data + cache directory for one test, under the WORKSPACE's
/// `target/` (worker rule 8: nothing big on the RAM-backed tmpfs) and removed on
/// drop, so a passing run leaves nothing behind.
pub struct TempDir {
    path: std::path::PathBuf,
}

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let unique = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nz-nowplaying-tests")
            .join(format!("{}-{tag}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("data")).expect("data dir");
        std::fs::create_dir_all(path.join("cache")).expect("cache dir");
        TempDir { path }
    }

    /// A core over this directory, built the way `NzBridge.install` builds it.
    pub fn core(&self) -> super::Core {
        super::Core::new(crate::core::CoreConfig {
            data_dir: self.path.join("data"),
            cache_dir: self.path.join("cache"),
            app_version: "0.0.0-test".to_owned(),
            version_code: 2_000_001,
            sdk_int: 35,
            device_name: "test".to_owned(),
        })
        .expect("a core over a throwaway directory")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A server the core is configured for and signed in to, the way the app is at
/// `Application.onCreate` (a stored origin and a restored session).
pub async fn configured() -> (NowPlayingServer, TempDir, super::Core) {
    let server = NowPlayingServer::start();
    let dir = TempDir::new("nowplaying");
    let core = dir.core();
    core.set_server(&server.base())
        .await
        .expect("a reachable server");
    core.login("bench", "Bench1").await.expect("a session");
    (server, dir, core)
}
