//! What the search's tests talk to: a small server on a loopback port that
//! answers this screen's routes with the REAL shapes, and COUNTS every request it
//! is offered.
//!
//! Why its own and not `core::testing::Bench`: that bench is MB2's and answers
//! `/api/version`, the session and a cover — these routes would be `404` there,
//! and the acceptance is about what THIS screen asks. A fake that speaks the
//! search's routes makes the numbers exact (one search and one fallback for a
//! settled query, none from the kept answer), and it needs no interpreter, so the
//! mobile CI's Rust job — which installs no Python — runs the same tests.
//!
//! The fixtures are the SPA's own (`webapp/test/searchrank.test.mjs` and
//! `queryradio.test.mjs`): a query that names an artist ("daft punk"), a
//! catalogue answer with a library row first, and a fallback whose first hit is
//! identical to a catalogue row (so the weaving's de-duplication is visible) and
//! whose second is a closer match.
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
pub struct SearchServer {
    address: String,
    log: Arc<Mutex<Vec<Hit>>>,
    broken: Arc<Mutex<Vec<(String, u16)>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    connections: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
    accepted: Arc<AtomicUsize>,
}

impl SearchServer {
    pub fn start() -> SearchServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the search server");
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
        SearchServer {
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

    /// The query `q` of the last request to `route`.
    pub fn last_query(&self, route: &str, name: &str) -> Option<String> {
        self.hits()
            .into_iter()
            .rev()
            .find(|hit| hit.route() == route)
            .and_then(|hit| hit.query(name))
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

impl Drop for SearchServer {
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
    {
        let mut entries = log.lock().unwrap_or_else(|err| err.into_inner());
        entries.push(hit);
    }

    let route = path.split('?').next().unwrap_or(&path).to_owned();
    let broken_status = broken
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .iter()
        .rev()
        .find_map(|(name, status)| (name == &route).then_some(*status));
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

/// The signed-in person, with the two things this screen reads: the sources (the
/// fallback's chip is offered only when YouTube is a fallback) and the grants.
const ME: &str = r#"{"user":{"id":"u-search","name":"Nadia","admin":false,"onboarding":"done",
 "access":{"youtube":["search"]},
 "sources":[{"id":"deezer","name":"Deezer","place":"main","rank":0},
            {"id":"youtube","name":"YouTube","place":"fallback","rank":1}]}}"#;

/// `/api/search?q=daft punk`: one library row, three catalogue rows — one of them
/// the exact title the fallback also has (so the weaving drops one), the artist
/// the query names, an album and a playlist.
const SEARCH: &str = r#"{
 "tracks":[
  {"id":"11111111-1111-4111-8111-111111111111","deezer_id":"1001",
   "title":"Get Lucky","duration":369,"local":true,
   "artist":{"deezer_id":"27","name":"Daft Punk"},
   "artists":[{"deezer_id":"27","name":"Daft Punk","role":"Main"},
              {"deezer_id":"28","name":"Pharrell Williams","role":"Featured"}],
   "display_artist":"Daft Punk, Pharrell Williams",
   "album":{"deezer_id":"501","title":"Random Access Memories","cover":"/api/cover/501"}},
  {"deezer_id":"1002","title":"Get Lucky","duration":370,
   "artist":{"deezer_id":"27","name":"Daft Punk"},
   "display_artist":"Daft Punk",
   "album":{"deezer_id":"501","title":"Random Access Memories","cover":"/api/cover/501"}},
  {"deezer_id":"1003","title":"One More Time","duration":320,
   "artist":{"deezer_id":"27","name":"Daft Punk"},
   "display_artist":"Daft Punk",
   "album":{"deezer_id":"502","title":"Discovery","cover":"/api/cover/502"}},
  {"deezer_id":"1004","title":"Around the World (Live)","duration":430,
   "artist":{"deezer_id":"27","name":"Daft Punk"},
   "display_artist":"Daft Punk",
   "album":{"deezer_id":"503","title":"Alive","cover":"/api/cover/503"}}],
 "artists":[
  {"id":"aaaaaaaa-1111-4111-8111-111111111111","deezer_id":"27","name":"Daft Punk",
   "picture":"/api/cover/aaaaaaaa-1111-4111-8111-111111111111"},
  {"deezer_id":"29","name":"Daftside","picture":null}],
 "albums":[
  {"id":"bbbbbbbb-1111-4111-8111-111111111111","deezer_id":"501",
   "title":"Random Access Memories","cover":"/api/cover/501","nb_tracks":13,
   "artist":{"deezer_id":"27","name":"Daft Punk"}},
  {"deezer_id":"502","title":"Discovery","cover":"/api/cover/502","nb_tracks":14,
   "artist":{"deezer_id":"27","name":"Daft Punk"}}],
 "playlists":[
  {"deezer_id":"7001","title":"French Touch","description":"","cover":"/api/cover/7001",
   "nb_tracks":40,"owner":"Nadia","is_favorite":false}],
 "podcasts":[{"deezer_id":"8001","title":"Daft Punk, une histoire","description":"Un épisode",
   "cover":"/api/cover/8001","fans":12}]}"#;

/// `/api/search?q=fete`: a BROAD answer — 34 titles of one artist — so the
/// screen's « Lancer la radio » is really offered (`queryradio.js`'s 30-result
/// bar) and the launch can be measured against a real queue.
fn broad_answer() -> String {
    let tracks: Vec<String> = (0..34)
        .map(|n| {
            format!(
                r#"{{"deezer_id":"30{n:02}","title":"Soirée {n}","duration":180,
                    "artist":{{"deezer_id":"27","name":"Fête"}},
                    "display_artist":"Fête",
                    "album":{{"deezer_id":"600","title":"La fête","cover":"/api/cover/600"}}}}"#
            )
        })
        .collect();
    format!(
        r#"{{"tracks":[{}],
            "artists":[{{"deezer_id":"27","name":"Fête","picture":null}}],
            "albums":[{{"deezer_id":"600","title":"La fête","cover":"/api/cover/600",
                        "artist":{{"deezer_id":"27","name":"Fête"}}}}],
            "playlists":[],"podcasts":[]}}"#,
        tracks.join(",")
    )
}

/// `/api/youtube/search?q=daft punk`: the first hit is the SAME recording as the
/// catalogue's "Get Lucky" (with a cosmetic decoration), the second is a closer
/// match than any catalogue row (the exact title, by the right artist), the third
/// is under the bar (a cover band).
const YOUTUBE: &str = r#"{"tracks":[
 {"deezer_id":"yt-1","local":true,"source":"youtube","youtube_id":"v1",
  "title":"Get Lucky (Clip officiel)","duration":369,
  "artist":{"deezer_id":"","name":"Daft Punk"},"display_artist":"Daft Punk",
  "album":{"deezer_id":"","title":"Get Lucky","cover":"/api/cover/yt-1"}},
 {"deezer_id":"yt-2","local":true,"source":"youtube","youtube_id":"v2",
  "title":"Daft Punk","duration":240,
  "artist":{"deezer_id":"","name":"Daft Punk"},"display_artist":"Daft Punk",
  "album":{"deezer_id":"","title":"Daft Punk","cover":"/api/cover/yt-2"}},
 {"deezer_id":"yt-3","local":true,"source":"youtube","youtube_id":"v3",
  "title":"Get Lucky","duration":200,
  "artist":{"deezer_id":"","name":"The Cover Band"},"display_artist":"The Cover Band",
  "album":{"deezer_id":"","title":"Covers","cover":"/api/cover/yt-3"}}]}"#;

/// `/api/transcripts/search?q=memoire`: one passage, one episode scanned.
const TRANSCRIPTS: &str = r#"{"q":"memoire","hits":[
 {"idx":0,"start":600.0,"end":615.0,"text":"la memoire de la ville","score":1.5,
  "episode_id":"5ef08406-6ef3-4adf-835b-c9e9927ba70e",
  "episode":{"id":"5ef08406-6ef3-4adf-835b-c9e9927ba70e","title":"Épisode 1",
             "cover":"/api/cover/ep1"},
  "channel_title":"Les Histoires"}],
 "total":1,"more":false,
 "scope":{"episodes":1,"segments":240,"truncated":false}}"#;

/// `/api/search/podcasts?q=daft`: one show.
const SHOWS: &str = r#"{"podcasts":[{"deezer_id":"8001","title":"Daft Punk, une histoire",
 "description":"Un épisode","cover":"/api/cover/8001","fans":12}]}"#;

/// `/api/radio/track/<id>` — the player's own top-up, so a test can prove the
/// launch itself asked nothing.
const RADIO_TRACK: &str = r#"{"tracks":[
 {"deezer_id":"2001","title":"Da Funk","duration":320,
  "artist":{"deezer_id":"27","name":"Daft Punk"},
  "album":{"deezer_id":"504","title":"Homework","cover":"/api/cover/504"}}]}"#;

fn answer_for(method: &str, route: &str, path: &str, _body: &str) -> (u16, String, Vec<u8>) {
    let json = "Content-Type: application/json";
    // A broad answer for one term, so the radio's own threshold is in play.
    if (method, route) == ("GET", "/api/search") && path.contains("q=fete") {
        return (200, json.to_owned(), broad_answer().into_bytes());
    }
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
        ("GET", "/api/search") => (200, json.to_owned(), SEARCH.as_bytes().to_vec()),
        ("GET", "/api/youtube/search") => (200, json.to_owned(), YOUTUBE.as_bytes().to_vec()),
        ("GET", "/api/search/podcasts") => (200, json.to_owned(), SHOWS.as_bytes().to_vec()),
        ("GET", "/api/transcripts/search") => {
            (200, json.to_owned(), TRANSCRIPTS.as_bytes().to_vec())
        }
        ("GET", route) if route.starts_with("/api/radio/track/") => {
            (200, json.to_owned(), RADIO_TRACK.as_bytes().to_vec())
        }
        _ => (404, json.to_owned(), br#"{"error":"not found"}"#.to_vec()),
    }
}

/// A throwaway data + cache directory for one test.
///
/// Under the WORKSPACE's `target/`, not `/tmp`: the mobile plan's rule 8 says
/// nothing big goes on the RAM-backed tmpfs, and a SQLite file per test is
/// exactly the kind of leftover a run should not accumulate there. It is removed
/// on drop, so a passing run leaves nothing behind.
pub struct TempDir {
    path: std::path::PathBuf,
}

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let unique = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nz-search-tests")
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
pub async fn configured() -> (SearchServer, TempDir, super::Core) {
    let server = SearchServer::start();
    let dir = TempDir::new("search");
    let core = dir.core();
    core.set_server(&server.base())
        .await
        .expect("a reachable server");
    core.login("bench", "Bench1").await.expect("a session");
    (server, dir, core)
}
