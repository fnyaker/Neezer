//! What the home's tests talk to: a small server on a loopback port that answers
//! the seven routes with the REAL shapes, and COUNTS every request it is offered.
//!
//! Why its own and not `core::testing::Bench`: that bench is MB2's and answers
//! `/api/version`, the session and a cover — the home's routes would be `404`
//! there, and the acceptance is about what this screen asks. A fake that speaks
//! the home's routes makes the numbers exact (seven on a cold open, none from
//! the cache), and needs no interpreter, so the mobile CI's Rust job — which
//! installs no Python — runs the same tests.
//!
//! It answers on a THREAD PER CONNECTION: `Core::home` fires its branches with
//! `tokio::join!`, so five requests can be in flight at once, exactly as the SPA's
//! `loadAll` does. A single-threaded accept loop would deadlock the test rather
//! than the code, which would be a lie about the code.
//!
//! Nothing waits on the clock: a test reads the request log once its `await`
//! returned, and the log is the proof.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);
use std::sync::{Arc, Mutex};

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
pub struct HomeServer {
    address: String,
    log: Arc<Mutex<Vec<Hit>>>,
    broken: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    connections: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
    accepted: Arc<AtomicUsize>,
}

impl HomeServer {
    pub fn start() -> HomeServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the home server");
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
        HomeServer {
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

    /// How many sockets were accepted at all.
    pub fn accepted(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }
}

impl Drop for HomeServer {
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
    let hit = Hit {
        method: method.clone(),
        path: path.clone(),
        body: body.clone(),
    };
    {
        let mut entries = log.lock().unwrap_or_else(|err| err.into_inner());
        entries.push(hit);
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

const ME: &str = r#"{"user":{"id":"u-home","name":"Nadia","admin":false,"onboarding":"done","access":{},"sources":[]}}"#;

/// Three new titles over two releases, so the "one card per release" rule is
/// visible in the answer.
const RELEASES: &str = r#"{"releases":[],"unseen":2,"pending":1,"tracks":[
 {"id":"11111111-1111-4111-8111-111111111111","deezer_id":"9001","title":"Ouverture","duration":180,
  "artist":{"deezer_id":"42","name":"Ascenseur"},"display_artist":"Ascenseur",
  "album":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"},
  "release":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"}},
 {"id":"11111111-1111-4111-8111-222222222222","deezer_id":"9002","title":"Suite","duration":200,
  "artist":{"deezer_id":"42","name":"Ascenseur"},
  "album":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"},
  "release":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"}},
 {"id":"11111111-1111-4111-8111-333333333333","deezer_id":"9003","title":"Single","duration":210,
  "artist":{"deezer_id":"43","name":"June"},"display_artist":"June",
  "album":{"deezer_id":"502","title":"Un titre","cover":"/api/cover/502"},
  "release":{"deezer_id":"502","title":"Un titre","cover":"/api/cover/502"}}]}"#;

const ARTISTS: &str = r#"{"artists":[
 {"id":"aaaaaaaa-1111-4111-8111-111111111111","deezer_id":"42","name":"Ascenseur",
  "picture":"/api/cover/aaaaaaaa-1111-4111-8111-111111111111","tracks":12,"plays":40,"starred":"2025-01-01"},
 {"deezer_id":"43","name":"June","picture":null,"tracks":3,"plays":1}]}"#;

const MOODS: &str = r#"{"moods":[
 {"id":"calme","name":"Calme","subtitle":"Doux et posé","tracks":40,"ready":true,"cover":"/api/cover/501"},
 {"id":"fete","name":"Fête","subtitle":"Ça bouge","tracks":2,"ready":false,"cover":null}]}"#;

const MOOD: &str = r#"{"id":"calme","name":"Calme","subtitle":"Doux","ready":true,"tracks":[
 {"id":"11111111-1111-4111-8111-777777777777","deezer_id":"9007","title":"Mix 1","duration":170,
  "artist":{"deezer_id":"42","name":"Ascenseur"},
  "album":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"}}]}"#;

const PREFS: &str = r#"{"level":0.6,"step":"Exploration","steps":[{"name":"Proche","from":0.0}],
 "energy":0.3,"energy_step":"Calme","energy_steps":[]}"#;

const RECO: &str = r#"{"level":0.6,"step":"Exploration","albums":[],"artists":[],"tracks":[
 {"id":"11111111-1111-4111-8111-444444444444","deezer_id":"9004","title":"Pour toi","duration":150,
  "artist":{"deezer_id":"42","name":"Ascenseur"},
  "album":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"}, "energy":0.55}]}"#;

const YEARS: &str = r#"{"years":[{"year":2025,"events":40,"listens":38,"minutes":120,"enough":true,"partial":false},
 {"year":2024,"events":0,"listens":0,"minutes":0,"enough":false,"partial":false}],
 "latest":2025,"now_year":2026,"min_year":2020,"timezone":"server-local"}"#;

const STYLES: &str = r#"{"styles":[
 {"name":"Techno","parent":null,"tracks":10,"branch":12},
 {"name":"House","parent":null,"tracks":4,"branch":4},
 {"name":"Électronique","parent":null,"tracks":0,"branch":16},
 {"name":"Detroit","parent":"Techno","tracks":2,"branch":2}]}"#;

const FLOW_NEXT: &str = r#"{"tracks":[
 {"id":"11111111-1111-4111-8111-555555555555","deezer_id":"9005","title":"Flow 1","duration":190,
  "artist":{"deezer_id":"42","name":"Ascenseur"},
  "album":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"},"energy":0.5},
 {"id":"11111111-1111-4111-8111-666666666666","deezer_id":"9006","title":"Flow 2","duration":191,
  "artist":{"deezer_id":"42","name":"Ascenseur"},
  "album":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"},"energy":0.5}]}"#;

fn answer_for(method: &str, route: &str, path: &str, body: &str) -> (u16, String, Vec<u8>) {
    let json = "Content-Type: application/json";
    match (method, route) {
        ("GET", "/api/version") => (
            200,
            json.to_owned(),
            br#"{"version":"9.9.9","build":"fake"}"#.to_vec(),
        ),
        ("POST", "/api/login") => (
            200,
            format!("{json}\r\nSet-Cookie: session=good; Path=/"),
            ME.as_bytes().to_vec(),
        ),
        ("GET", "/api/me") => (200, json.to_owned(), ME.as_bytes().to_vec()),
        ("GET", "/api/releases") => (200, json.to_owned(), RELEASES.as_bytes().to_vec()),
        ("GET", "/api/me/artists") => (200, json.to_owned(), ARTISTS.as_bytes().to_vec()),
        ("GET", "/api/moods") => (200, json.to_owned(), MOODS.as_bytes().to_vec()),
        ("GET", route) if route.starts_with("/api/moods/") => {
            (200, json.to_owned(), MOOD.as_bytes().to_vec())
        }
        ("GET", "/api/discover/prefs") => (200, json.to_owned(), PREFS.as_bytes().to_vec()),
        ("PUT", "/api/discover/prefs") => {
            // Echoes what it was given, the way the route does.
            let level = body
                .split('"')
                .skip_while(|part| *part != "level")
                .nth(2)
                .and_then(|raw| {
                    raw.trim_matches(|ch: char| !ch.is_ascii_digit() && ch != '.')
                        .parse::<f64>()
                        .ok()
                });
            let answer = match level {
                Some(level) => format!(r#"{{"level":{level},"step":"Exploration"}}"#),
                None => r#"{"energy":0.7,"energy_step":"Énergique"}"#.to_owned(),
            };
            (200, json.to_owned(), answer.into_bytes())
        }
        ("GET", "/api/recommend") => {
            let _ = path;
            (200, json.to_owned(), RECO.as_bytes().to_vec())
        }
        ("GET", "/api/recap/years") => (200, json.to_owned(), YEARS.as_bytes().to_vec()),
        ("GET", "/api/flow/styles") => (200, json.to_owned(), STYLES.as_bytes().to_vec()),
        ("POST", "/api/flow/next") => (200, json.to_owned(), FLOW_NEXT.as_bytes().to_vec()),
        _ => (404, json.to_owned(), br#"{"error":"not found"}"#.to_vec()),
    }
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
            .join("../../target/nz-home-tests")
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
/// outlive it, and a test that dropped it early would read an empty home.
pub async fn configured() -> (HomeServer, TempDir, super::Core) {
    let server = HomeServer::start();
    let dir = TempDir::new("home");
    let core = dir.core();
    core.set_server(&server.base())
        .await
        .expect("a reachable server");
    core.login("bench", "Bench1").await.expect("a session");
    (server, dir, core)
}
