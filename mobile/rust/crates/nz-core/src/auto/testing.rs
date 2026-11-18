//! What MB25's tests talk to: a loopback server that answers the four routes a
//! head unit browses with the REAL shapes, and COUNTS every request it is
//! offered — plus whether it came on a background lane (`X-NS-Background: 1`,
//! which is how rule 1's "a browse is `Prefetch`" is measured rather than
//! assumed).
//!
//! Why its own and not another lot's fake: MB8's answers the same routes for its
//! own assertions, but a lot may not read another lot's `#[cfg(test)]` module,
//! and MB25's numbers are about the tree (one request per node cold, none warm).
//! It needs no interpreter, so the mobile CI's Rust job runs the same tests.
//!
//! Nothing waits on the clock: a test reads the request log after its `await`
//! returned, and the log is the proof.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::core::Core;

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

/// How many favourites the "capped at a hundred" test asks for: more than the
/// node's ceiling, small enough to parse in a test.
pub const FAVORITES_ROWS: usize = 120;

/// One request the server was offered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub method: String,
    /// The path with its query, as it arrived.
    pub path: String,
    /// `true` when the request announced itself as background work
    /// (`X-NS-Background: 1`), which is what `Prefetch` and `Bulk` send.
    pub background: bool,
}

impl Hit {
    /// The path without its query — what a route matches on.
    pub fn route(&self) -> &str {
        self.path.split('?').next().unwrap_or(&self.path)
    }
}

/// The fake server: a port, a request log, and the answers of the car's routes.
pub struct AutoServer {
    address: String,
    log: Arc<Mutex<Vec<Hit>>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    connections: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
}

impl AutoServer {
    pub fn start() -> AutoServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the Auto server");
        let address = format!("http://{}", listener.local_addr().expect("its address"));
        let log: Arc<Mutex<Vec<Hit>>> = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let connections: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>> =
            Arc::new(Mutex::new(Vec::new()));
        let shared_log = Arc::clone(&log);
        let stopping = Arc::clone(&stop);
        let shared_connections = Arc::clone(&connections);
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let log = Arc::clone(&shared_log);
                let handle = std::thread::spawn(move || {
                    let Ok(mut stream) = stream.try_clone() else {
                        return;
                    };
                    serve(&mut stream, &log);
                });
                shared_connections
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(handle);
                if stopping.load(Ordering::SeqCst) {
                    break;
                }
            }
        });
        AutoServer {
            address,
            log,
            stop,
            thread: Some(thread),
            connections,
        }
    }

    /// `http://127.0.0.1:<port>` — what `set_server` is given.
    pub fn base(&self) -> String {
        self.address.clone()
    }

    pub fn hits(&self) -> Vec<Hit> {
        self.log
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// How many requests were offered — the number the acceptance counts.
    pub fn count(&self) -> usize {
        self.hits().len()
    }

    /// How many requests matched `route`.
    pub fn count_of(&self, route: &str) -> usize {
        self.hits()
            .iter()
            .filter(|hit| hit.route() == route)
            .count()
    }

    /// Forgets what happened: a test counts what IT caused, after the login.
    pub fn forget(&self) {
        self.log
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
    }
}

impl Drop for AutoServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // One probe connection wakes `incoming()` so the loop can see the flag.
        let _ = TcpStream::connect(self.address.trim_start_matches("http://"));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let connections = std::mem::take(
            &mut *self
                .connections
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        );
        for connection in connections {
            let _ = connection.join();
        }
    }
}

fn serve(stream: &mut TcpStream, log: &Arc<Mutex<Vec<Hit>>>) {
    let mut buffer = [0u8; 8192];
    let read = stream.read(&mut buffer).unwrap_or(0);
    let request = String::from_utf8_lossy(&buffer[..read]).to_string();
    let mut lines = request.split("\r\n");
    let head = lines.next().unwrap_or_default();
    let mut head = head.split(' ');
    let method = head.next().unwrap_or_default().to_owned();
    let path = head.next().unwrap_or_default().to_owned();
    let background = request.to_ascii_lowercase().contains("x-ns-background: 1");
    log.lock()
        .unwrap_or_else(|error| error.into_inner())
        .push(Hit {
            method: method.clone(),
            path: path.clone(),
            background,
        });
    let body = String::new();
    let (status, answer, extra) = answer_for(&method, &path, &body);
    let head = format!(
        "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
        answer.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(answer.as_bytes());
    let _ = stream.flush();
}

const ME: &str = r#"{"user":{"id":"u-auto","name":"Nadia","admin":false,"onboarding":"done","access":{},"sources":[]}}"#;

/// The synthetic favourites: `FAVORITES_ROWS` titles, four names among them so
/// an assertion can read a title rather than only a count.
pub fn favorites_body(count: usize) -> String {
    let mut out = String::with_capacity(count * 180 + 32);
    out.push_str("{\"tracks\":[");
    for index in 0..count {
        let (title, artist) = match index % 4 {
            0 => ("Ouverture", "Ascenseur"),
            1 => ("Sinnerman", "Nina Simone"),
            2 => ("Été", "Zazie"),
            _ => ("Beyoncé", "Beyoncé"),
        };
        if index > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            r#"{{"id":"11111111-1111-4111-8111-{:012x}","deezer_id":"{}","title":"{} {}",
               "duration":{},"artist":{{"deezer_id":"42","name":"{}"}},
               "album":{{"deezer_id":"{}","title":"Album {}","cover":"/api/cover/{}"}}}}"#,
            index,
            9_000_000 + index,
            title,
            index,
            150 + index % 180,
            artist,
            500 + index % 40,
            index % 40,
            index % 40
        ));
    }
    out.push_str("]}");
    out
}

/// The playlist library: one shared list FIRST (the server's order), two of the
/// person's, so the "own first" ordering is measured and not assumed.
const PLAYLISTS: &str = r#"{
 "playlists": [
  {"id":"aaaaaaaa-1111-4111-8111-000000000003","title":"Avec June",
   "deezer_id":null,"cover":null,"nb_tracks":8,"role":"editor","owner":"June",
   "editable":true,"can_manage":false,"folder_id":null,"position":null},
  {"id":"aaaaaaaa-1111-4111-8111-000000000001","title":"Route",
   "deezer_id":null,"cover":"/api/cover/500","nb_tracks":12,"role":"owner","owner":"Nadia",
   "editable":true,"can_manage":true,"folder_id":null,"position":0},
  {"id":"aaaaaaaa-1111-4111-8111-000000000002","title":"Matin",
   "deezer_id":null,"cover":null,"nb_tracks":3,"role":"owner","owner":"Nadia",
   "editable":true,"can_manage":true,"folder_id":null,"position":1}
 ],
 "folders": []}"#;

/// One playlist page: three titles, the third one unavailable.
const PLAYLIST_PAGE: &str = r#"{
 "playlist": {"id":"aaaaaaaa-1111-4111-8111-000000000001","title":"Route",
   "cover":"/api/cover/500","nb_tracks":3,"role":"owner","editable":true,"can_manage":true},
 "tracks": [
  {"id":"11111111-1111-4111-8111-0000000000a1","deezer_id":"9100001","title":"Départ",
   "duration":180,"artist":{"deezer_id":"42","name":"Ascenseur"},
   "album":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"}},
  {"id":"11111111-1111-4111-8111-0000000000a2","deezer_id":"9100002","title":"Nationale 7",
   "duration":240,"explicit":true,"artist":{"deezer_id":"43","name":"Zazie"},
   "album":{"deezer_id":"502","title":"Zen","cover":null}},
  {"id":"11111111-1111-4111-8111-0000000000a3","deezer_id":"9100003","title":"Arrivée",
   "duration":200,"unavailable":true,"artist":{"deezer_id":"42","name":"Ascenseur"},
   "album":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"}}
 ]}"#;

/// The history: three events, the FIRST title played twice, so the "one row per
/// title" rule is measured.
const HISTORY: &str = r#"{
 "events": [
  {"id": 103, "at": "2026-11-18T08:10:00", "listened": 12, "outcome": 1, "source": "album",
   "track": {"id":"11111111-1111-4111-8111-000000000001","deezer_id":"9000001","title":"Ouverture",
             "duration":200,"artist":{"deezer_id":"42","name":"Ascenseur"},
             "album":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"}}},
  {"id": 102, "at": "2026-11-18T08:04:00", "listened": 187, "outcome": 1, "source": "album",
   "track": {"id":"11111111-1111-4111-8111-000000000001","deezer_id":"9000001","title":"Ouverture",
             "duration":200,"artist":{"deezer_id":"42","name":"Ascenseur"},
             "album":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"}}},
  {"id": 101, "at": "2026-11-17T20:58:00", "listened": 12, "outcome": -1,
   "track": {"id":"11111111-1111-4111-8111-000000000002","deezer_id":"9000002","title":"Suite",
             "duration":90,"artist":{"deezer_id":"42","name":"Ascenseur"},
             "album":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"}}}
 ],
 "next": {"before": "2026-11-17T20:58:00", "before_id": 101},
 "summary": {"days": 30, "listens": 41, "skips": 3, "minutes": 220,
   "top_artists": [], "top_tracks": []}}"#;

fn answer_for(method: &str, path: &str, _body: &str) -> (u16, String, String) {
    let route = path.split('?').next().unwrap_or(path);
    match (method, route) {
        ("GET", "/api/version") => (
            200,
            r#"{"version":"9.9.9","build":"fake"}"#.to_owned(),
            String::new(),
        ),
        ("POST", "/api/login") => (
            200,
            ME.to_owned(),
            "Set-Cookie: session=good; Path=/\r\n".to_owned(),
        ),
        ("GET", "/api/me") => (200, ME.to_owned(), String::new()),
        ("GET", "/api/me/favorites") => (200, favorites_body(FAVORITES_ROWS), String::new()),
        ("GET", "/api/me/playlists") => (200, PLAYLISTS.to_owned(), String::new()),
        ("GET", "/api/history") => (200, HISTORY.to_owned(), String::new()),
        ("GET", route) if route.starts_with("/api/playlist/") => {
            (200, PLAYLIST_PAGE.to_owned(), String::new())
        }
        _ => (404, r#"{"error":"not found"}"#.to_owned(), String::new()),
    }
}

/// A throwaway data + cache directory for one test, under the workspace's
/// `target/` (worker rule 8: nothing big on the RAM-backed tmpfs), removed on
/// drop.
pub struct TempDir {
    path: std::path::PathBuf,
}

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let unique = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nz-auto-tests")
            .join(format!("{}-{tag}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("data")).expect("data dir");
        std::fs::create_dir_all(path.join("cache")).expect("cache dir");
        TempDir { path }
    }

    /// A core over this directory, built the way `NzBridge.install` builds it.
    pub fn core(&self) -> Core {
        Core::new(crate::core::CoreConfig {
            data_dir: self.path.join("data"),
            cache_dir: self.path.join("cache"),
            app_version: "0.0.0-test".to_owned(),
            version_code: 2_000_001,
            sdk_int: 35,
            device_name: "test".to_owned(),
        })
        .expect("a core over a throwaway directory")
    }

    pub fn cache_dir(&self) -> std::path::PathBuf {
        self.path.join("cache")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A server the core is configured for and signed in to, the way the app is at
/// `Application.onCreate`, with the request log emptied so a test counts only
/// what it causes.
pub async fn configured() -> (AutoServer, TempDir, Core) {
    let server = AutoServer::start();
    let dir = TempDir::new("auto");
    let core = dir.core();
    core.set_server(&server.base())
        .await
        .expect("a reachable server");
    core.login("bench", "Bench1").await.expect("a session");
    server.forget();
    (server, dir, core)
}
