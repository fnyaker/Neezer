//! What Réglages' tests talk to: a small server on a loopback port that answers
//! this screen's routes with the REAL shapes, and COUNTS every request it is
//! offered.
//!
//! Why its own and not `core::testing::Bench` (MB2's): that bench answers
//! `/api/version`, the session and a cover — Réglages' routes would be `404`
//! there, and the acceptance is a count ("the SPA's own requests, no more"). A
//! fake that speaks these routes makes the numbers exact and needs no
//! interpreter, so the mobile CI's Rust job — which installs no Python — runs
//! the same tests.
//!
//! It answers on a THREAD PER CONNECTION: two sections can be read at once (the
//! admin card fires `/settings` and `/deezer/status` together), and a
//! single-threaded accept loop would deadlock the TEST rather than the code.
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
    pub body: String,
}

impl Hit {
    /// The path WITHOUT its query — what a route matches on.
    pub fn route(&self) -> &str {
        self.path.split('?').next().unwrap_or(&self.path)
    }
}

/// The fake server: a port, a request log, and a per-route status a test can
/// break on purpose.
pub struct SettingsServer {
    address: String,
    log: Arc<Mutex<Vec<Hit>>>,
    broken: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    connections: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
}

impl SettingsServer {
    pub fn start() -> SettingsServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the settings server");
        let address = format!("http://{}", listener.local_addr().expect("its address"));
        let log = Arc::new(Mutex::new(Vec::new()));
        let broken = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let connections: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>> =
            Arc::new(Mutex::new(Vec::new()));

        let shared_log = Arc::clone(&log);
        let shared_broken = Arc::clone(&broken);
        let stopping = Arc::clone(&stop);
        let shared_connections = Arc::clone(&connections);
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
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
        SettingsServer {
            address,
            log,
            broken,
            stop,
            thread: Some(thread),
            connections,
        }
    }

    /// `http://127.0.0.1:<port>` — what `set_server` is given.
    pub fn base(&self) -> String {
        self.address.clone()
    }

    /// Every request offered so far, in arrival order.
    pub fn hits(&self) -> Vec<Hit> {
        self.log
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    /// How many requests this screen asked for since the test's last `forget`.
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

    /// The body of the last request made with `method` to `route` — a read
    /// often follows a write on the same path, and the two bodies differ.
    pub fn last_body_of(&self, method: &str, route: &str) -> Option<String> {
        self.hits()
            .into_iter()
            .rev()
            .find(|hit| hit.method == method && hit.route() == route)
            .map(|hit| hit.body)
    }

    /// Forgets what was counted, so a second measure is not the sum of both.
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
}

impl Drop for SettingsServer {
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
        None => answer_for(&method, &route),
    };
    let head = format!(
        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\n{extra}\r\nConnection: close\r\n\r\n",
        answer.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&answer);
    let _ = stream.flush();
}

const ME: &str = r#"{"user":{"id":"u-settings","name":"Nadia","admin":true,"onboarding":"done","access":{},"sources":[]}}"#;

/// The order, as `sourcesapi.py#sources_payload` builds it: Deezer leads, YouTube
/// falls back (2nd), Qobuz is switched off.
const SOURCES: &str = r#"{"main":"deezer","admin":true,"sources":[
 {"id":"deezer","name":"Deezer","enabled":true,"configured":true,"can_lead":true,"switchable":true,
  "place":"main","rank":0,"quality":{"class":40,"kbps":1411,"label":"FLAC","lossless":true},
  "resting":0,"resting_why":"","setup":""},
 {"id":"youtube","name":"YouTube","enabled":true,"configured":true,"can_lead":false,"switchable":true,
  "place":"fallback","rank":1,"quality":{"class":10,"kbps":160,"label":"Opus 160","lossless":false},
  "resting":120,"resting_why":"n'a pas répondu","setup":""},
 {"id":"qobuz","name":"Qobuz","enabled":false,"configured":false,"can_lead":true,"switchable":true,
  "place":"off","rank":null,"quality":{"class":0,"kbps":0,"label":"","lossless":false},
  "resting":0,"resting_why":"","setup":"Ajoutez vos identifiants Qobuz."}]}"#;

/// One linked Deezer account, its last import and its running one.
const ACCOUNTS: &str = r#"{"default":"acc-1","can_link":{"deezer":false,"youtube":true},"accounts":[
 {"id":"acc-1","provider":"deezer","name":"Deezer","label":"nadia","mode":"import","server":false,
  "status":"ok","auto":{"playlists":true,"tracks":true},"lossless":true,"default":true,
  "kinds":["artists","tracks","albums","playlists"],"pickable":["artists"],
  "last":{"at":"2026-11-17T10:00:00Z","added":{"tracks":12,"playlists":1},"kept":{"albums":3}},
  "running":{"phase":"import","kind":"tracks","done":40,"total":120}}]}"#;

/// Two providers and one group, as `accessapi.py` answers.
const ACCESS: &str = r#"{"me":"u-settings","admin":true,"groups":[
 {"id":"g1","name":"Maison","members":[{"id":"u2","name":"Léo"}]}],
 "users":[{"id":"u2","name":"Léo"}],
 "providers":[
  {"id":"deezer","name":"Deezer","role":"main","account":true,"accounts":[
    {"id":"acc-1","mode":"server","label":"serveur","server":true,"owner":null,"mine":true,
     "editable":true,"grants":{"*":["search","stream"]}}],
   "allowed":["search","stream"],"anonymous":false,"anonymous_use":null,
   "auth_modes":[],"features":[{"id":"search","scope":"shared"},{"id":"stream","scope":"shared"}]},
  {"id":"youtube","name":"YouTube","role":"fallback","account":false,"accounts":[],
   "allowed":[],"anonymous":true,"anonymous_use":{"editable":true,"grants":{"*":["search"]}},
   "auth_modes":[],"features":[{"id":"search","scope":"shared"}]}]}"#;

/// The export targets, as `services.py` lists them.
const SERVICES: &str = r#"{"services":[
 {"id":"spotify","name":"Spotify","linked":false,"ready":false,"account":null,
  "client_id":"","redirect_uri":null,"redirect_here":null,"file":null},
 {"id":"youtube","name":"YouTube Music","linked":true,"ready":true,"account":"nadia",
  "client_id":null,"redirect_uri":null,"redirect_here":null,"file":null},
 {"id":"file","name":"Fichier","linked":false,"ready":true,"account":null,
  "client_id":null,"redirect_uri":null,"redirect_here":null,"file":true}],
 "formats":[{"id":"m3u","name":"M3U"},{"id":"csv","name":"CSV (ISRC)"}]}"#;

/// `/api/settings` — the quota and the credential's state (admin).
const SERVER_SETTINGS: &str = r#"{"upload_quota_gb":12.5,
 "deezer":{"enabled":true,"arl_set":true,"arl_source":"database","arl_hint":"…a1b2","archive_dir_set":true}}"#;

/// `/api/deezer/status` — the credential is good.
const DEEZER_STATUS: &str = r#"{"ok":true,"account":"nadia","admin":true,"message":null}"#;

/// Two titles the person said they do not like.
const DISLIKES: &str = r#"{"tracks":[
 {"id":"22222222-2222-4222-8222-111111111111","deezer_id":"8001","title":"Bof","duration":180,
  "artist":{"deezer_id":"42","name":"Ascenseur"},"display_artist":"Ascenseur",
  "album":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"}},
 {"id":"22222222-2222-4222-8222-222222222222","deezer_id":"8002","title":"Mouais","duration":200,
  "artist":{"deezer_id":"43","name":"June"},
  "album":{"deezer_id":"502","title":"Un titre","cover":null}}]}"#;

fn answer_for(method: &str, route: &str) -> (u16, String, Vec<u8>) {
    let json = "Content-Type: application/json";
    let body = match (method, route) {
        ("GET", "/api/version") => r#"{"version":"9.9.9","build":"fake"}"#,
        ("POST", "/api/login") => ME,
        ("GET", "/api/me") => ME,
        ("GET", "/api/sources") | ("PUT", "/api/sources") => SOURCES,
        ("GET", "/api/me/accounts")
        | ("POST", "/api/me/accounts/deezer")
        | ("PUT", "/api/me/accounts/default") => ACCOUNTS,
        ("GET", "/api/access") => ACCESS,
        ("GET", "/api/services") => SERVICES,
        ("GET", "/api/settings") | ("POST", "/api/settings") => SERVER_SETTINGS,
        ("GET", "/api/deezer/status") => DEEZER_STATUS,
        ("GET", "/api/me/dislikes") => DISLIKES,
        ("POST", "/api/dislike") => r#"{"ok":true}"#,
        _ if route.starts_with("/api/me/accounts/") => ACCOUNTS,
        _ => {
            return (404, json.to_owned(), br#"{"error":"not found"}"#.to_vec());
        }
    };
    let status = if method == "POST" && route == "/api/login" {
        // The cookie the client keeps, as the real login sets it.
        return (
            200,
            format!("{json}\r\nSet-Cookie: session=good; Path=/"),
            body.as_bytes().to_vec(),
        );
    } else {
        200
    };
    (status, json.to_owned(), body.as_bytes().to_vec())
}

/// A throwaway data + cache directory for one test.
///
/// Under the WORKSPACE's `target/`, not `/tmp`: the mobile plan's rule 8 says
/// nothing big goes on the RAM-backed tmpfs, and a SQLite file per test is
/// exactly the kind of leftover a run should not accumulate there. It is
/// removed on drop, so a passing run leaves nothing behind.
pub struct TempDir {
    path: std::path::PathBuf,
}

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let unique = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nz-settings-tests")
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
///
/// The directory comes back with the two: it owns the core's files and must
/// outlive it, and a test that dropped it early would read an empty cache.
pub async fn configured() -> (SettingsServer, TempDir, super::Core) {
    let server = SettingsServer::start();
    let dir = TempDir::new("settings");
    let core = dir.core();
    core.set_server(&server.base())
        .await
        .expect("the fake server answers /api/version");
    core.login("bench", "Bench1").await.expect("the fake login");
    // The setup's own requests are not this screen's cost.
    server.forget();
    (server, dir, core)
}
