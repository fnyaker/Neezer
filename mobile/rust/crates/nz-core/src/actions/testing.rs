//! What this area's tests talk to: a small server on a loopback port that
//! answers the routes the track actions call, with the REAL shapes, and COUNTS
//! every request it is offered.
//!
//! Why its own and not `core::testing::Bench` (MB2's) or the entity area's: those
//! answer other routes, so these would be `404` there — and this lot's acceptance
//! is about what each ACTION costs (0 for the menu, 1 for a favourite, 1 for the
//! picker's list, 1 for a replacement's candidates, 1 per upload deleted) and
//! about what happens when the server says no (the rollback). A fake that speaks
//! these routes makes both exact, and needs no interpreter, so the mobile CI's
//! Rust job runs the same tests.
//!
//! It answers on a THREAD PER CONNECTION, and nothing waits on the clock: a test
//! reads the request log once its `await` returned, and the log is the proof.

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
    pub body: String,
}

impl Hit {
    /// The path WITHOUT its query — what a route matches on.
    pub fn route(&self) -> &str {
        self.path.split('?').next().unwrap_or(&self.path)
    }

    /// The query's value for `name`, decoded enough for this fake's own keys.
    pub fn query(&self, name: &str) -> Option<String> {
        let (_, query) = self.path.split_once('?')?;
        for pair in query.split('&') {
            let (key, value) = pair.split_once('=')?;
            if key == name {
                return Some(value.replace("%20", " ").replace("%2C", ","));
            }
        }
        None
    }
}

/// The fake server: a port, a request log, and a per-route status a test can
/// break on purpose.
pub struct ActionsServer {
    address: String,
    log: Arc<Mutex<Vec<Hit>>>,
    broken: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    connections: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
    accepted: Arc<AtomicUsize>,
}

impl ActionsServer {
    pub fn start() -> ActionsServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the actions server");
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
        ActionsServer {
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

    /// How many requests were made since the test's last `forget`.
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

    /// The body of the last request to `route`.
    pub fn last_body(&self, route: &str) -> Option<String> {
        self.hits()
            .into_iter()
            .rev()
            .find(|hit| hit.route() == route)
            .map(|hit| hit.body)
    }

    /// Forgets the requests AND the sockets, so a test that starts here measures
    /// its own action and not the login that preceded it.
    pub fn forget(&self) {
        self.log
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clear();
        self.accepted.store(0, Ordering::SeqCst);
    }

    /// Makes `route` answer `status` from now on (the failure tests).
    pub fn break_route(&self, route: &str, status: u16) {
        self.broken
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .push(format!("{route} {status}"));
    }

    /// The size of the `/me/playlists` answer this fake serves, in bytes: the
    /// number a screen's own meter must hold after one cold read.
    pub fn playlists_bytes(&self) -> usize {
        PLAYLISTS.len()
    }

    /// How many sockets were accepted at all — the "0 sockets offline" measure.
    pub fn accepted(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }
}

impl Drop for ActionsServer {
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
    let mut body: Vec<u8> = buffer[split..].to_vec();
    let head = String::from_utf8_lossy(&buffer[..split.min(buffer.len())]).into_owned();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let length: usize = lines
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .filter_map(|(_, value)| value.trim().parse().ok())
        .next()
        .unwrap_or(0);
    while body.len() < length {
        let mut rest = vec![0u8; length - body.len()];
        match stream.read(&mut rest) {
            Ok(0) | Err(_) => break,
            Ok(read) => body.extend_from_slice(&rest[..read]),
        }
    }
    let body = String::from_utf8_lossy(&body).into_owned();
    {
        let mut entries = log.lock().unwrap_or_else(|err| err.into_inner());
        entries.push(Hit {
            method: method.clone(),
            path: path.clone(),
            body: body.clone(),
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
    let (status, extra, answer) = match broken_status {
        Some(status) => (
            status,
            "Content-Type: application/json".to_owned(),
            br#"{"error":"broken on purpose"}"#.to_vec(),
        ),
        None => answer_for(&method, &route, &path, &body),
    };
    let head = format!(
        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\n{extra}\r\nConnection: close\r\n\r\n",
        answer.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&answer);
    let _ = stream.flush();
}

const ME: &str = r#"{"user":{"id":"u-actions","name":"Nadia","admin":true,"onboarding":"done","access":{},"sources":[]}}"#;

/// `GET /me/playlists`: two writable lists and one a viewer may read — the
/// picker's own filter is what drops the third.
const PLAYLISTS: &str = r#"{"folders":[],"playlists":[
 {"id":"11111111-1111-4111-8111-111111111111","title":"Route du soir","cover":null,
  "editable":true,"role":"owner","nb_tracks":4,"version":2},
 {"id":"22222222-2222-4222-8222-222222222222","title":"Le matin","cover":"/api/cover/2",
  "editable":true,"role":"owner","nb_tracks":1,"version":1},
 {"id":"33333333-3333-4333-8333-333333333333","title":"Celle de June","cover":null,
  "editable":false,"role":"viewer","owner":"June","nb_tracks":9,"version":5}]}"#;

/// `GET /replace/candidates/<id>`: a local file first, then a provider title.
const CANDIDATES: &str = r#"{"query":"Ascenseur Ouverture",
 "candidates":[
  {"id":"44444444-4444-4444-8444-444444444444","deezer_id":"44444444-4444-4444-8444-444444444444",
   "title":"Ouverture","duration":181,"local":true,
   "artist":{"deezer_id":"42","name":"Ascenseur"},
   "album":{"deezer_id":"bbbbbbbb-1111-4111-8111-111111111111","title":"Le matin","cover":null}},
  {"deezer_id":"9002","title":"Ouverture (live)","duration":240,"source":"youtube",
   "artist":{"deezer_id":"42","name":"Ascenseur"},
   "album":{"deezer_id":"501","title":"En concert"}}],
 "fallback_auto":null}"#;

fn answer_for(method: &str, route: &str, path: &str, body: &str) -> (u16, String, Vec<u8>) {
    let json = "Content-Type: application/json";
    let ok = |body: &str| (200, json.to_owned(), body.as_bytes().to_vec());
    let _ = body;
    match (method, route) {
        ("GET", "/api/version") => ok(r#"{"version":"9.9.9","build":"fake"}"#),
        ("POST", "/api/login") => (
            200,
            format!("{json}\r\nSet-Cookie: session=good; Path=/"),
            ME.as_bytes().to_vec(),
        ),
        ("GET", "/api/me") => ok(ME),
        ("GET", "/api/me/playlists") => ok(PLAYLISTS),
        ("POST", "/api/favorite") => ok(r#"{"ok":true,"favorite":true}"#),
        ("POST", "/api/dislike") => ok(r#"{"ok":true,"disliked":true}"#),
        ("POST", "/api/playlists") => ok(r#"{"id":"55555555-5555-4555-8555-555555555555",
            "title":"Ma playlist 4","editable":true,"role":"owner","nb_tracks":1,"version":1}"#),
        ("POST", route) if route.starts_with("/api/playlist/") && route.ends_with("/tracks") => {
            ok(r#"{"ok":true,"added":1,"version":3,"nb_tracks":5}"#)
        }
        ("GET", route) if route.starts_with("/api/replace/candidates/") => {
            if path.contains("q=") {
                ok(r#"{"query":"June","candidates":[],"fallback_auto":"youtube"}"#)
            } else {
                ok(CANDIDATES)
            }
        }
        ("POST", "/api/replace") => ok(r#"{"ok":true,"job":"job-replace"}"#),
        ("GET", "/api/replace/status/job-replace") => ok(
            r#"{"kind":"replace","running":false,"ok":true,"playlists":2,"favorites":1,"error":null}"#,
        ),
        ("POST", "/api/tracks/delete") => ok(r#"{"ok":true,"job":"job-delete","total":2}"#),
        ("GET", "/api/replace/status/job-delete") => ok(
            r#"{"kind":"delete-many","running":false,"ok":true,"deleted":["deezer:9001"],
                "kept":[{"id":"deezer:9002","reason":"playable"}],"missing":[],"done":2,"total":2}"#,
        ),
        ("DELETE", route) if route.starts_with("/api/local/") => ok(r#"{"ok":true}"#),
        ("GET", route) if route.ends_with("/share") => ok(r#"{"link":null}"#),
        ("POST", route) if route.ends_with("/share") => ok(
            r#"{"link":{"token":"tok-1","url":"/shared/playlist/tok-1","created":"2026-11-18T10:00:00"}}"#,
        ),
        _ => (404, json.to_owned(), br#"{"error":"not found"}"#.to_vec()),
    }
}

/// A throwaway data + cache directory for one test, under the WORKSPACE's
/// `target/` (worker rule 8: nothing big on the RAM-backed tmpfs).
pub struct TempDir {
    path: std::path::PathBuf,
}

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let unique = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nz-actions-tests")
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
pub async fn configured() -> (ActionsServer, TempDir, super::Core) {
    let server = ActionsServer::start();
    let dir = TempDir::new("actions");
    let core = dir.core();
    core.set_server(&server.base())
        .await
        .expect("a reachable server");
    core.login("bench", "Bench1").await.expect("a session");
    server.forget();
    (server, dir, core)
}
