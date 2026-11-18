//! What Découverte's tests talk to: a small server on a loopback port that
//! answers this lot's routes with the REAL shapes, and COUNTS every request it
//! is offered.
//!
//! Why its own and not MB2's `core::testing::Bench`: that bench answers
//! `/api/version`, the session and a cover — these routes would be `404` there,
//! and the acceptance is about what these four screens ask. A fake that speaks
//! them makes the numbers exact (a cold Nouveautés is one GET, or two when the
//! badge is unseen; a shelf page and a warm Récap year are zero), and needs no
//! interpreter, so the mobile CI's Rust job — which installs no Python — runs
//! the same tests.
//!
//! It answers on a THREAD PER CONNECTION: `Core::releases` fires nothing in
//! parallel, but `Core::recap` asks `/recap/years` before the year, and a
//! single-threaded accept loop would deadlock the test rather than the code.
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

    /// The query's value for `name`, decoded enough for this fake's own keys.
    pub fn query(&self, name: &str) -> Option<String> {
        let (_, query) = self.path.split_once('?')?;
        for pair in query.split('&') {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            if key == name {
                return Some(
                    value
                        .replace("%20", " ")
                        .replace("%2F", "/")
                        .replace("%C3%89", "É"),
                );
            }
        }
        None
    }
}

/// The fake server: a port, a request log, and a per-route status a test can
/// break on purpose.
pub struct DiscoveryServer {
    address: String,
    log: Arc<Mutex<Vec<Hit>>>,
    broken: Arc<Mutex<Vec<(String, u16)>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    connections: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
    accepted: Arc<AtomicUsize>,
}

impl DiscoveryServer {
    pub fn start() -> DiscoveryServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the discovery server");
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
        DiscoveryServer {
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
            .push((route.to_owned(), status));
    }

    /// How many sockets were accepted at all.
    pub fn accepted(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }
}

impl Drop for DiscoveryServer {
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
fn serve(stream: &mut TcpStream, log: &Mutex<Vec<Hit>>, broken: &Mutex<Vec<(String, u16)>>) {
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
    log.lock().unwrap_or_else(|err| err.into_inner()).push(hit);

    let route = path.split('?').next().unwrap_or(&path).to_owned();
    let broken_status = broken
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .iter()
        .find_map(|(name, status)| match name.strip_suffix('*') {
            // `route*` breaks every path under a prefix.
            Some(prefix) => route.starts_with(prefix).then_some(*status),
            None => (route == *name).then_some(*status),
        });
    let (status, extra, answer) = match broken_status {
        Some(status) => (
            status,
            "Content-Type: application/json".to_owned(),
            br#"{"error":"broken on purpose"}"#.to_vec(),
        ),
        None => answer_for(&method, &route, &body),
    };
    let head = format!(
        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\n{extra}\r\nConnection: close\r\n\r\n",
        answer.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&answer);
    let _ = stream.flush();
}

const ME: &str = r#"{"user":{"id":"u-discovery","name":"Nadia","admin":false,"onboarding":"done","access":{},"sources":[]}}"#;

const RELEASES: &str = r#"{"releases":[
 {"deezer_id":"501","title":"Le matin","artist":{"deezer_id":"42","name":"Ascenseur"},
  "kind":"album","released":"2026-11-12","nb_tracks":2,"new":true},
 {"deezer_id":"502","title":"Voisin","artist":{"deezer_id":"43","name":"June"},
  "kind":"single","released":"2026-11-10","nb_tracks":1,"close_to":"Ascenseur","new":true}],
 "tracks":[
 {"deezer_id":"9001","title":"Ouverture","duration":180,
  "artist":{"deezer_id":"42","name":"Ascenseur"},
  "album":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"},
  "release":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501","kind":"album","nb_tracks":2},
  "new":true},
 {"deezer_id":"9002","title":"Suite","duration":210,
  "artist":{"deezer_id":"42","name":"Ascenseur"},
  "album":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"},
  "release":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501","kind":"album","nb_tracks":2}},
 {"deezer_id":"9003","title":"Voisin","duration":200,
  "artist":{"deezer_id":"43","name":"June"},
  "album":{"deezer_id":"502","title":"Voisin","cover":"/api/cover/502"},
  "release":{"deezer_id":"502","title":"Voisin","cover":"/api/cover/502","kind":"single","nb_tracks":1}}],
 "unseen":2,"pending":1}"#;

const RECAP_YEARS: &str = r#"{"years":[
 {"year":2026,"events":2412,"listens":2180,"minutes":8540,"enough":true,"partial":true},
 {"year":2025,"events":9120,"listens":8402,"minutes":30211,"enough":true,"partial":false},
 {"year":2024,"events":0,"listens":0,"minutes":0,"enough":false,"partial":false}],
 "latest":2026,"now_year":2026,"min_year":2024,"timezone":"server-local"}"#;

const RECAP: &str = r#"{"year":2026,"partial":true,"enough":true,"timezone":"server-local",
 "period":{"from":"2026-01-01","to":"2026-11-13"},
 "totals":{"events":2412,"listens":2180,"skips":190,"neutrals":42,"seconds":512400,
           "minutes":8540,"days_active":211},
 "months":[{"month":7,"listens":300},{"month":3,"listens":240}],
 "streak":{"days":23,"from":"2026-03-01","to":"2026-03-23"},
 "biggest_day":{"date":"2026-07-04","listens":61},
 "tops":{"tracks":[{"track":{"deezer_id":"9001","title":"Ouverture",
                             "artist":{"deezer_id":"42","name":"Ascenseur"},
                             "album":{"deezer_id":"501","cover":"/api/cover/501"}},"listens":41}],
         "artists":[{"id":"42","name":"Ascenseur","cover":"/api/cover/501","listens":210}],
         "albums":[{"id":"501","title":"Le matin","artist":"Ascenseur","cover":"/api/cover/501","listens":96}],
         "genres":{"available":true,"coverage":0.72,"explored":17,"measured_listens":1570,
                   "rows":[{"name":"Techno","listens":640,"share":0.41}]}},
 "first":{"at":"2026-01-02T08:11:00","track":{"deezer_id":"9001","title":"Ouverture"}},
 "last":{"at":"2026-11-13T20:58:00","track":{"deezer_id":"9003","title":"Voisin"}},
 "cards":["year","minutes","artists","tracks","albums","genres","streak","firstlast"],
 "generated":1794835151}"#;

const MOODS: &str = r#"{"moods":[
 {"id":"calme","name":"Calme","subtitle":"Doux et posé","tracks":40,"ready":true,"cover":"/api/cover/501"},
 {"id":"fete","name":"Fête","subtitle":"Ça bouge","tracks":2,"ready":false,"cover":null}]}"#;

const MOOD: &str = r#"{"id":"calme","name":"Calme","subtitle":"Doux","ready":true,"reason":null,
 "tracks":[{"deezer_id":"9007","title":"Mix 1","duration":170,
            "artist":{"deezer_id":"42","name":"Ascenseur"},
            "album":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"}}]}"#;

const EXPLORER_STYLES: &str = r#"{"styles":[
 {"name":"Techno","parent":null,"family":null,"tracks":10,"weight":1.0,"confidence":0.9,
  "leaf":false,"children":3},
 {"name":"Detroit","parent":"Techno","family":null,"tracks":2,"weight":0.4,"confidence":0.8,
  "leaf":true,"children":0},
 {"name":"House","parent":null,"family":null,"tracks":4,"weight":0.6,"confidence":0.7,
  "leaf":false,"children":0}],
 "scale":[],"analysed":0.8,"unknown":0.1,"tracks":16,"source":"history","generated":1}"#;

const DISCOVER_READY: &str = r#"{"state":"ready","style":"Techno","level":3,"playlists":[
 {"id":"p1","title":"Techno profond","cover":"/api/cover/601","nb_tracks":60,"matched":12,
  "tracks":[{"deezer_id":"9009","title":"Vérifié","duration":180,
             "artist":{"deezer_id":"42","name":"Ascenseur"},
             "album":{"deezer_id":"601","title":"Deep","cover":"/api/cover/601"}}]},
 {"id":"p2","title":"Detroit","cover":null,"nb_tracks":40,"matched":0,"tracks":[]}]}"#;

const DISCOVER_PENDING: &str = r#"{"state":"pending","style":"Techno","level":3,"playlists":[]}"#;

const STYLES: &str = r#"{"styles":[
 {"name":"Techno","parent":null,"tracks":10,"branch":12},
 {"name":"House","parent":null,"tracks":4,"branch":4},
 {"name":"Detroit","parent":"Techno","tracks":2,"branch":2}]}"#;

const STATION: &str = r#"{"style":"Techno","name":"Techno","tracks":[
 {"deezer_id":"9010","title":"Station 1","duration":190,
  "artist":{"deezer_id":"42","name":"Ascenseur"},
  "album":{"deezer_id":"601","title":"Deep","cover":"/api/cover/601"}}]}"#;

const RECO: &str = r#"{"level":0.4,"step":"Voisin","albums":[],"artists":[],"tracks":[
 {"deezer_id":"9011","title":"Recommandé","duration":200,
  "artist":{"deezer_id":"42","name":"Ascenseur"},
  "album":{"deezer_id":"601","title":"Deep","cover":"/api/cover/601"},"energy":0.5}]}"#;

const SETTINGS: &str = r#"{"on":true,"every":6,"close":3,"window":14,"listened":30,
 "running":false,"watched":120,"favourites":4,"links":8,"asked":120,"releases":64,
 "last_run":"2026-11-18T06:00:00","last_error":null}"#;

fn answer_for(method: &str, route: &str, _body: &str) -> (u16, String, Vec<u8>) {
    let json = "Content-Type: application/json";
    let ok = |body: &str| (200, json.to_owned(), body.as_bytes().to_vec());
    match (method, route) {
        ("GET", "/api/version") => ok(r#"{"version":"9.9.9","build":"fake"}"#),
        ("POST", "/api/login") => (
            200,
            format!("{json}\r\nSet-Cookie: session=good; Path=/"),
            ME.as_bytes().to_vec(),
        ),
        ("GET", "/api/me") => ok(ME),
        ("GET", "/api/releases") => ok(RELEASES),
        ("POST", "/api/releases/seen") => ok(r#"{"ok":true}"#),
        ("GET", "/api/releases/settings") => ok(SETTINGS),
        ("PUT", "/api/releases/settings") => ok(SETTINGS),
        ("POST", "/api/releases/check") => ok(SETTINGS),
        ("GET", "/api/recap/years") => ok(RECAP_YEARS),
        ("GET", route) if route.starts_with("/api/recap/") => ok(RECAP),
        ("GET", "/api/moods") => ok(MOODS),
        ("GET", route) if route.starts_with("/api/moods/") => ok(MOOD),
        ("GET", "/api/explorer/styles") => ok(EXPLORER_STYLES),
        ("POST", "/api/explorer/discover") => ok(DISCOVER_PENDING),
        ("GET", "/api/explorer/discover") => ok(DISCOVER_READY),
        ("GET", "/api/flow/styles") => ok(STYLES),
        ("GET", route) if route.starts_with("/api/radio/genre/") => ok(STATION),
        ("POST", "/api/discover/mute") => ok(r#"{"ok":true,"muted":1}"#),
        ("GET", "/api/recommend") => ok(RECO),
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
            .join("../../target/nz-discovery-tests")
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
pub async fn configured() -> (DiscoveryServer, TempDir, super::Core) {
    let server = DiscoveryServer::start();
    let dir = TempDir::new("discovery");
    let core = dir.core();
    core.set_server(&server.base())
        .await
        .expect("a reachable server");
    core.login("bench", "Bench1").await.expect("a session");
    (server, dir, core)
}
