//! What this area's tests talk to: a small server on a loopback port that
//! answers the library's routes with the REAL shapes, and COUNTS every request
//! it is offered.
//!
//! Why its own and not `core::testing::Bench` (MB2's): that bench answers
//! `/api/version`, the session and a cover, so these routes would be `404` there
//! — and this lot's acceptance is about what each LEAF asks (one request cold,
//! none before a warm open paints, and 8 000 rows in one answer). A fake that
//! speaks these routes makes the numbers exact, and needs no interpreter, so the
//! mobile CI's Rust job (which installs no Python) runs the same tests.
//!
//! It answers on a THREAD PER CONNECTION, and nothing waits on the clock: a test
//! reads the request log once its `await` returned, and the log is the proof.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::core::Core;

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

/// How many favourites the 8 000-row acceptance asks for.
pub const FAVORITES_ROWS: usize = 8_000;

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

    /// The query's RAW value for `name` (percent-escapes kept).
    pub fn query(&self, name: &str) -> Option<String> {
        let (_, query) = self.path.split_once('?')?;
        query.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (key == name).then(|| value.to_owned())
        })
    }
}

/// The fake server: a port, a request log, and a per-route status a test can
/// break on purpose.
pub struct LibraryServer {
    address: String,
    log: Arc<Mutex<Vec<Hit>>>,
    broken: Arc<Mutex<Vec<String>>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    connections: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
    accepted: Arc<AtomicUsize>,
}

impl LibraryServer {
    pub fn start() -> LibraryServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the library server");
        let address = format!("http://{}", listener.local_addr().expect("its address"));
        let log = Arc::new(Mutex::new(Vec::new()));
        let broken = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
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
        LibraryServer {
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

    /// How many requests this server was offered since the last [`forget`].
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

    /// The last request to `route`, whole (a test reads its query).
    pub fn last_hit(&self, route: &str) -> Option<Hit> {
        self.hits()
            .into_iter()
            .rev()
            .find(|hit| hit.route() == route)
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

impl Drop for LibraryServer {
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

const ME: &str = r#"{"user":{"id":"u-library","name":"Nadia","admin":false,"onboarding":"done","access":{},"sources":[]}}"#;

/// The synthetic 8 000-title favourites list: what the acceptance names, with
/// four distinct titles so a filter has something to find.
pub fn favorites_body(count: usize) -> String {
    let mut out = String::with_capacity(count * 190 + 32);
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

const PLAYLISTS: &str = r#"{
 "playlists": [
  {"id":"aaaaaaaa-1111-4111-8111-000000000001","title":"Route",
   "deezer_id":null,"cover":"/api/cover/500","nb_tracks":12,"role":"owner","owner":"Nadia",
   "editable":true,"can_manage":true,"folder_id":"ffffffff-1111-4111-8111-000000000001","position":0},
  {"id":"aaaaaaaa-1111-4111-8111-000000000002","title":"Matin",
   "deezer_id":null,"cover":null,"nb_tracks":3,"role":"owner","owner":"Nadia",
   "editable":true,"can_manage":true,"folder_id":null,"position":null},
  {"id":"aaaaaaaa-1111-4111-8111-000000000003","title":"Avec June",
   "deezer_id":null,"cover":null,"nb_tracks":8,"role":"editor","owner":"June",
   "editable":true,"can_manage":false,"folder_id":null},
  {"id":"aaaaaaaa-1111-4111-8111-000000000004","title":"Import Deezer",
   "deezer_id":"9080","cover":null,"nb_tracks":25,"role":"owner","owner":"Nadia",
   "editable":true,"can_manage":true,"folder_id":null}
 ],
 "folders": [
  {"id":"ffffffff-1111-4111-8111-000000000001","title":"Voyage","position":0,"nb_playlists":1},
  {"id":"ffffffff-1111-4111-8111-000000000002","title":"Vide","position":1,"nb_playlists":0}
 ]}"#;

const ARTISTS: &str = r#"{"artists": [
 {"id":"bbbbbbbb-1111-4111-8111-000000000001","deezer_id":"42","name":"Ascenseur",
  "picture":"/api/cover/bbbbbbbb-1111-4111-8111-000000000001","tracks":120,"plays":340,
  "starred":"2026-11-02T10:00:00"},
 {"id":"bbbbbbbb-1111-4111-8111-000000000002","deezer_id":"43","name":"Zazie",
  "picture":null,"tracks":40,"plays":12,"starred":"2026-11-12T21:00:00"}
]}"#;

const ALBUMS: &str = r#"{"albums": [
 {"id":"cccccccc-1111-4111-8111-000000000001","deezer_id":"501","title":"Le matin",
  "cover":"/api/cover/501","year":2024,"nb_tracks":12,"artist":{"deezer_id":"42","name":"Ascenseur"}},
 {"id":"cccccccc-1111-4111-8111-000000000002","deezer_id":"502","title":"Zen",
  "cover":null,"year":"2019","nb_tracks":4,"artist":{"deezer_id":"43","name":"Zazie"}}
]}"#;

const LOCAL: &str = r#"{"tracks": [
 {"id":"dddddddd-1111-4111-8111-000000000001","deezer_id":"dddddddd-1111-4111-8111-000000000001",
  "title":"Ma prise","duration":240,"local":true,
  "artist":{"deezer_id":"","name":"Nadia"},
  "album":{"deezer_id":"","title":"Mes enregistrements","cover":null}}
]}"#;

const DISLIKES: &str = r#"{"tracks": [
 {"id":"eeeeeeee-1111-4111-8111-000000000001","deezer_id":"7001","title":"Trop fort",
  "duration":200,"artist":{"deezer_id":"44","name":"Bruit"},
  "album":{"deezer_id":"600","title":"Volume","cover":null}}
]}"#;

const UNAVAILABLE: &str = r#"{"tracks": [
 {"id":"12121212-1111-4111-8111-000000000001","deezer_id":"7002","title":"Parti",
  "duration":180,"unavailable":true,"artist":{"deezer_id":"45","name":"Absent"},
  "album":{"deezer_id":"601","title":"Retiré","cover":null}}
]}"#;

const USAGE: &str = r#"{"used":1073741824,"quota":5368709120,"unlimited":false}"#;

const FAVORITE_IDS: &str = r#"{"ids":["11111111-1111-4111-8111-000000000001","9000001"],
 "albums":["501"],"artists":["42"],"playlists":[],"disliked":["7001"]}"#;

/// One page of the history: two events, a cursor, and the 30-day summary.
const HISTORY: &str = r#"{
 "events": [
  {"id": 102, "at": "2026-11-17T21:04:00", "listened": 187, "outcome": 1, "source": "album",
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
   "top_artists": [{"id":"42","name":"Ascenseur","cover":"/api/cover/42","count":9}],
   "top_tracks": [{"count": 4, "track": {"id":"11111111-1111-4111-8111-000000000001",
     "deezer_id":"9000001","title":"Ouverture","artist":{"deezer_id":"42","name":"Ascenseur"},
     "album":{"deezer_id":"501","title":"Le matin"}}}]}
}"#;

/// An EMPTY history: the empty state's own answer.
const EMPTY_HISTORY: &str = r#"{"events":[],"next":null,
 "summary":{"days":30,"listens":0,"skips":0,"minutes":0,"top_artists":[],"top_tracks":[]}}"#;

fn answer_for(method: &str, route: &str, path: &str, body: &str) -> (u16, String, Vec<u8>) {
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
        ("GET", "/api/me/playlists") => ok(PLAYLISTS),
        ("GET", "/api/me/favorites") => ok(&favorites_body(FAVORITES_ROWS)),
        ("GET", "/api/me/favorite-ids") => ok(FAVORITE_IDS),
        ("GET", "/api/me/artists") => ok(ARTISTS),
        ("GET", "/api/me/albums") => ok(ALBUMS),
        ("GET", "/api/me/local") => ok(LOCAL),
        ("GET", "/api/me/dislikes") => ok(DISLIKES),
        ("GET", "/api/unavailable") => ok(UNAVAILABLE),
        ("GET", "/api/upload/usage") => ok(USAGE),
        ("GET", "/api/history") => {
            if path.contains("outcome=skip") {
                ok(EMPTY_HISTORY)
            } else {
                ok(HISTORY)
            }
        }
        // The folder routes: the SPA's own answers, and the `OPTIONS` that tells
        // an older server from this one (`features.rs`).
        ("OPTIONS", "/api/playlist-folders") => (200, "Allow: GET, POST".to_owned(), Vec::new()),
        ("POST", "/api/playlist-folders") => {
            let title = title_of(body);
            (
                201,
                json.to_owned(),
                format!(
                    r#"{{"ok":true,"id":"99999999-9999-4999-8999-999999999999","title":"{title}","position":2}}"#
                )
                .into_bytes(),
            )
        }
        ("PATCH", _) if route.starts_with("/api/playlist-folders/") => ok(r#"{"ok":true}"#),
        ("DELETE", _) if route.starts_with("/api/playlist-folders/") => {
            ok(r#"{"ok":true,"playlists":["aaaaaaaa-1111-4111-8111-000000000001"]}"#)
        }
        ("PUT", "/api/playlist-folders/order") => ok(r#"{"ok":true,"folders":[]}"#),
        ("PUT", _) if route.starts_with("/api/playlist-folders/") => ok(r#"{"ok":true}"#),
        ("POST", "/api/dislike") => ok(r#"{"ok":true}"#),
        _ => (404, json.to_owned(), br#"{"error":"not found"}"#.to_vec()),
    }
}

/// The `title` of a JSON body, without a parser: the tests' own bodies are
/// built by this crate and carry no escaped quote in a title.
fn title_of(body: &str) -> String {
    body.split("\"title\":\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap_or_default()
        .to_owned()
}

/// A throwaway data + cache directory for one test, under the workspace's
/// `target/` (rule 8: nothing big on the RAM-backed tmpfs), removed on drop.
pub struct TempDir {
    path: std::path::PathBuf,
}

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let unique = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nz-library-tests")
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
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A server the core is configured for and signed in to, the way the app is at
/// `Application.onCreate` (a stored origin and a restored session).
pub async fn configured() -> (LibraryServer, TempDir, Core) {
    let server = LibraryServer::start();
    let dir = TempDir::new("library");
    let core = dir.core();
    core.set_server(&server.base())
        .await
        .expect("a reachable server");
    core.login("bench", "Bench1").await.expect("a session");
    server.forget();
    (server, dir, core)
}
