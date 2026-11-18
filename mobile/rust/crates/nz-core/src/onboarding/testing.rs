//! What the onboarding's and the invitation's tests talk to: a small server on a
//! loopback port that answers this lot's routes with the REAL shapes, and COUNTS
//! every request it is offered.
//!
//! Why its own and not another lot's: the search's fake answers `/api/search` and
//! the home's `/api/releases`; these routes would be `404` there, and the
//! acceptance is about what THIS screen asks. A fake that speaks the onboarding's
//! routes makes the numbers exact (the artists step's two, the account step's
//! one, a like's one, the invitation's one + one), and it needs no interpreter,
//! so the mobile CI's Rust job — which installs no Python — runs the same tests.
//!
//! Nothing waits on the clock: a test reads the request log once its `await`
//! returned, and the log is the proof.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

static NEXT_DIR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

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
pub struct OnboardingServer {
    address: String,
    log: Arc<Mutex<Vec<Hit>>>,
    broken: Arc<Mutex<Vec<(String, u16)>>>,
    answers: Arc<Mutex<Vec<(String, String, String)>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    connections: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
}

impl OnboardingServer {
    pub fn start() -> OnboardingServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the onboarding server");
        let address = format!("http://{}", listener.local_addr().expect("its address"));
        let log = Arc::new(Mutex::new(Vec::new()));
        let broken = Arc::new(Mutex::new(Vec::new()));
        let answers = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let connections = Arc::new(Mutex::new(Vec::new()));

        let shared_log = Arc::clone(&log);
        let shared_broken = Arc::clone(&broken);
        let shared_answers = Arc::clone(&answers);
        let stopping = Arc::clone(&stop);
        let shared_connections = Arc::clone(&connections);
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let log = Arc::clone(&shared_log);
                let broken = Arc::clone(&shared_broken);
                let answers = Arc::clone(&shared_answers);
                let handle = std::thread::spawn(move || {
                    let Ok(mut stream) = stream.try_clone() else {
                        return;
                    };
                    serve(&mut stream, &log, &broken, &answers);
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
        OnboardingServer {
            address,
            log,
            broken,
            answers,
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

    pub fn count(&self) -> usize {
        self.hits().len()
    }

    /// The routes hit, in order (`METHOD /api/route`) — the "which request" half
    /// of the measure.
    pub fn routes(&self) -> Vec<String> {
        self.hits()
            .into_iter()
            .map(|hit| format!("{} {}", hit.method, hit.route()))
            .collect()
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

    /// Makes `route` answer `body` with `status` from now on (a success the test
    /// wants to shape).
    pub fn answer(&self, method: &str, route: &str, status: u16, body: &str) {
        self.answers
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .push((
                format!("{method} {route}"),
                status.to_string(),
                body.to_owned(),
            ));
    }
}

impl Drop for OnboardingServer {
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
fn serve(
    stream: &mut TcpStream,
    log: &Mutex<Vec<Hit>>,
    broken: &Mutex<Vec<(String, u16)>>,
    answers: &Mutex<Vec<(String, String, String)>>,
) {
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
    let key = format!("{method} {route}");
    let overridden = answers
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .iter()
        .rev()
        .find_map(|(name, status, body)| {
            (name == &key).then(|| {
                (
                    status.parse::<u16>().unwrap_or(200),
                    "Content-Type: application/json".to_owned(),
                    body.as_bytes().to_vec(),
                )
            })
        });
    let broken_status = broken
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .iter()
        .rev()
        .find_map(|(name, status)| (name == &route).then_some(*status));
    let (status, extra, answer) = match (overridden, broken_status) {
        (Some(answer), _) => answer,
        (None, Some(status)) => (
            status,
            "Content-Type: application/json".to_owned(),
            br#"{"error":"broken on purpose"}"#.to_vec(),
        ),
        (None, None) => answer_for(&method, &route, &path, &body),
    };
    let head = format!(
        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\n{extra}\r\nConnection: close\r\n\r\n",
        answer.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&answer);
    let _ = stream.flush();
}

/// The signed-in person, with the one thing this screen reads: where the
/// onboarding stands.
const ME: &str = r#"{"user":{"id":"u-onboarding","name":"Nadia","admin":false,"onboarding":"done",
 "access":{"deezer":["links"]},
 "sources":[{"id":"deezer","name":"Deezer","place":"main","rank":0}]}}"#;

/// `/api/onboarding` — the artists step, one artist kept, a ready feed.
const STATE: &str = r#"{"step":"artists","liked":1,"min_artists":1,
 "diversity":{"window":6,"max_same_genre":3,"max_streak":2},
 "feed":{"ready":true,"building":false,"source":"deezer","error":null,"state":"ready",
         "build":null,"stale":false,"kept":0,"progress":null},
 "account":{"can_save_imports":true,
            "deezer":{"available":true,"linked":false,"auth_modes":["arl"],"account":null},
            "youtube":{"available":true,"linked":false,"auth_modes":["headers"],"account":null},
            "imports":[{"source":"deezer","label":"Deezer","mode":"public_link","available":true},
                       {"source":"spotify","label":"Spotify","mode":"public_link","available":true}]},
 "fallback":null,
 "youtube":{"enabled":true,"installed":true,"denied":false,"held":false}}"#;

/// `/api/onboarding/artists?limit=36` — six tiles, the last one already liked.
const ARTISTS: &str = r#"{"items":[
 {"deezer_id":"101","id":"101","name":"Aya Nakamura","picture":"/api/onboarding/picture/x",
  "picture_small":"/api/onboarding/picture/x","genres":["Pop"],"liked":false,
  "provider":"deezer","openable":true,"via":"chart","parent":null},
 {"deezer_id":"102","id":"102","name":"Daft Punk","picture":null,"picture_small":null,
  "genres":["Électro"],"liked":true,"provider":"deezer","openable":true,"via":"chart"},
 {"deezer_id":"103","id":"103","name":"Air","genres":["Électro"],"liked":false,
  "provider":"deezer","openable":true,"via":"chart"},
 {"deezer_id":"104","id":"104","name":"Justice","genres":["Électro"],"liked":false,
  "provider":"deezer","openable":true,"via":"chart"},
 {"deezer_id":"105","id":"105","name":"Kavinsky","genres":["Électro"],"liked":false,
  "provider":"deezer","openable":true,"via":"chart"},
 {"deezer_id":"106","id":"106","name":"Phoenix","genres":["Pop"],"liked":false,
  "provider":"deezer","openable":true,"via":"chart"}],
 "next":"c2","total":12,"source":"deezer","degraded":false,"building":false,
 "state":"ready","build":null,"seed":"s","partial":false}"#;

/// `/api/onboarding/search?q=aya` — the box's answer.
const SEARCH: &str = r#"{"query":"aya","items":[
 {"deezer_id":"101","id":"101","name":"Aya Nakamura","genres":["Pop"],"liked":false,
  "provider":"deezer","openable":true,"via":"search"}],
 "local":1,"partial":false,"error":null}"#;

/// `POST /api/onboarding/artists/like` — the answer already carries the
/// neighbours, so a like costs ONE request.
const LIKE: &str = r#"{"liked":true,
 "artist":{"deezer_id":"103","id":"103","name":"Air","genres":["Électro"]},
 "related":{"state":"ready","complete":true,"error":null,"retry_after":null,
   "items":[
    {"deezer_id":"201","id":"201","name":"Zero 7","genres":["Électro"],
     "provider":"deezer","openable":true,"via":"related","parent":"103"},
    {"deezer_id":"202","id":"202","name":"Télépopmusik","genres":["Électro"],
     "provider":"deezer","openable":true,"via":"related","parent":"103"}]}}"#;

/// `GET /api/onboarding/artists/<id>/related` — the poll's answer.
const RELATED: &str = r#"{"state":"ready","complete":true,"error":null,"retry_after":null,
 "items":[{"deezer_id":"201","id":"201","name":"Zero 7","genres":["Électro"],
           "provider":"deezer","openable":true,"via":"related","parent":"103"}]}"#;

/// `/api/onboarding/account/options` — one import already running, Deezer ready
/// to link.
const OPTIONS: &str = r#"{"step":"account","can_save_imports":true,
 "deezer":{"available":true,"can_link":true,"linked":false,"account":null,
           "auth_modes":["arl","native"],"reason":null},
 "youtube":{"available":true,"can_link":true,"installed":true,"enabled":true,
            "denied":false,"desktop":true,"linked":false,"account":null,
            "auth_modes":["headers"],"reason":null},
 "imports":[{"source":"deezer","label":"Deezer","mode":"public_link","available":true,"reason":null},
            {"source":"spotify","label":"Spotify","mode":"public_link","available":true,"reason":null},
            {"source":"apple","label":"Apple Music","mode":"public_link","available":true,"reason":null},
            {"source":"youtube","label":"YouTube Music","mode":"public_link","available":true,"reason":null}],
 "job":{"id":"job-running","state":"matching","source":"spotify","source_label":"Spotify",
        "kind":"playlist","name":"Mes hits","total":120,"truncated":false,"done":42,
        "matched":40,"error":null,"counts":{"deezer":38,"youtube":2}}}"#;

/// `POST /api/onboarding/account/import` — the job to poll.
const STARTED: &str = r#"{"id":"job-1",
 "job":{"id":"job-1","state":"reading","source":"spotify","source_label":"Spotify",
        "kind":"playlist","name":"Mes hits","total":0,"truncated":false,"done":0,
        "matched":0,"error":null,"counts":{}},
 "options":{"step":"account","can_save_imports":true,
   "deezer":{"available":true,"can_link":true,"linked":false,"account":null},
   "youtube":{"available":true,"can_link":true,"linked":false,"account":null},
   "imports":[],"job":{"id":"job-1","state":"reading","total":0,"done":0,"matched":0}}}"#;

/// `GET /api/import/playlist/<id>` — a finished import with two unmatched lines
/// (each one's cause is its own title), and the provider's own note.
const JOB: &str = r#"{"id":"job-1","state":"done","source":"spotify","source_label":"Spotify",
 "kind":"playlist","name":"Mes hits","total":3,"truncated":false,"done":3,"matched":1,
 "error":"Deezer ne répondait pas : certains titres n'ont pas pu être cherchés",
 "items":[
  {"line":{"title":"Get Lucky","artists":["Daft Punk"],"album":"Random Access Memories","duration":369},
   "track":{"deezer_id":"1001"},"via":"deezer","sure":true},
  {"line":{"title":"Inconnu 1","artists":["Quelqu'un"],"album":"","duration":200},
   "track":null,"via":null,"sure":false},
  {"line":{"title":"Inconnu 2","artists":[],"album":"","duration":180},
   "track":null,"via":null,"sure":false}],
 "counts":{"deezer":1}}"#;

/// `POST /api/invites/inspect` — a valid link.
const INVITE: &str = r#"{"valid":true,"expires":"2026-11-21T18:00:00","label":"Invitation"}"#;

/// `POST /api/invites/redeem` — the account, and where it lands.
const REDEEM: &str = r#"{"user":{"id":"u-new","name":"elodie","admin":false,
 "onboarding":"artists","access":{},"sources":[]}}"#;

fn answer_for(method: &str, route: &str, path: &str, _body: &str) -> (u16, String, Vec<u8>) {
    let json = "Content-Type: application/json";
    // The invited person's own account, for the redeem's `Set-Cookie`.
    let cookie = format!("{json}\r\nSet-Cookie: session=good; Path=/");
    match (method, route) {
        ("GET", "/api/version") => (
            200,
            json.to_owned(),
            br#"{"version":"9.9.9","build":"fake"}"#.to_vec(),
        ),
        ("POST", "/api/login") => (200, cookie.clone(), ME.as_bytes().to_vec()),
        ("GET", "/api/me") => (200, json.to_owned(), ME.as_bytes().to_vec()),
        ("GET", "/api/onboarding") => (200, json.to_owned(), STATE.as_bytes().to_vec()),
        ("GET", "/api/onboarding/artists") if path.contains("after=") => (
            200,
            json.to_owned(),
            r#"{"items":[
                 {"deezer_id":"107","id":"107","name":"M83","genres":["Électro"],"liked":false,
                  "provider":"deezer","openable":true,"via":"chart"}],
                 "next":null,"total":12,"source":"deezer","building":false,"state":"ready"}"#
                .as_bytes()
                .to_vec(),
        ),
        ("GET", "/api/onboarding/artists") => {
            (200, json.to_owned(), ARTISTS.as_bytes().to_vec())
        }
        ("GET", "/api/onboarding/search") => (200, json.to_owned(), SEARCH.as_bytes().to_vec()),
        ("POST", "/api/onboarding/artists/like") => {
            (200, json.to_owned(), LIKE.as_bytes().to_vec())
        }
        ("DELETE", route) if route.starts_with("/api/onboarding/artists/") => {
            (200, json.to_owned(), br#"{"liked":false}"#.to_vec())
        }
        ("GET", route) if route.starts_with("/api/onboarding/artists/") => {
            (200, json.to_owned(), RELATED.as_bytes().to_vec())
        }
        ("POST", "/api/onboarding/artists/done") => (
            200,
            json.to_owned(),
            br#"{"step":"account"}"#.to_vec(),
        ),
        ("POST", "/api/onboarding/feed/retry") => (
            200,
            json.to_owned(),
            br#"{"building":true,"ready":false}"#.to_vec(),
        ),
        ("POST", "/api/onboarding/account/done") => (
            200,
            json.to_owned(),
            br#"{"step":"done","user":{"onboarding":"done"}}"#.to_vec(),
        ),
        ("GET", "/api/onboarding/account/options") => {
            (200, json.to_owned(), OPTIONS.as_bytes().to_vec())
        }
        ("POST", "/api/onboarding/account/link") => (
            200,
            json.to_owned(),
            r#"{"provider":"deezer","account":{"id":"7"},
                "options":{"step":"account","can_save_imports":true,
                  "deezer":{"available":true,"can_link":false,"linked":true,
                            "account":{"id":"7","label":"Deezer","mode":"arl","status":"ok",
                                       "running":{"running":true,"phase":"import","done":0,"total":42},
                                       "last":null}},
                  "youtube":{"available":true,"can_link":true,"linked":false},
                  "imports":[]}}"#
                .as_bytes()
                .to_vec(),
        ),
        ("POST", "/api/onboarding/account/import") => {
            (202, json.to_owned(), STARTED.as_bytes().to_vec())
        }
        ("GET", route) if route.starts_with("/api/import/playlist/") => {
            (200, json.to_owned(), JOB.as_bytes().to_vec())
        }
        ("POST", route) if route.ends_with("/save") => {
            (200, json.to_owned(), JOB.as_bytes().to_vec())
        }
        ("POST", "/api/invites/inspect") => (200, json.to_owned(), INVITE.as_bytes().to_vec()),
        ("POST", "/api/invites/redeem") => (201, cookie, REDEEM.as_bytes().to_vec()),
        _ => (404, json.to_owned(), br#"{"error":"not found"}"#.to_vec()),
    }
}

/// A throwaway data + cache directory for one test.
///
/// Under the WORKSPACE's `target/`, not `/tmp`: rule 8 says nothing big goes on
/// the RAM-backed tmpfs, and a SQLite file per test is exactly the kind of
/// leftover a run should not accumulate there. It is removed on drop.
pub struct TempDir {
    path: std::path::PathBuf,
}

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let unique = NEXT_DIR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nz-onboarding-tests")
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
pub async fn configured() -> (OnboardingServer, TempDir, super::Core) {
    let server = OnboardingServer::start();
    let dir = TempDir::new("onboarding");
    let core = dir.core();
    core.set_server(&server.base())
        .await
        .expect("a reachable server");
    core.login("bench", "Bench1").await.expect("a session");
    (server, dir, core)
}

/// The same, WITHOUT a session: the invitation page's person is not signed in.
pub async fn anonymous() -> (OnboardingServer, TempDir, super::Core) {
    let server = OnboardingServer::start();
    let dir = TempDir::new("invite");
    let core = dir.core();
    core.set_server(&server.base())
        .await
        .expect("a reachable server");
    (server, dir, core)
}

/// A test's token: the server's own `<22>.<24>` shape.
pub fn token() -> String {
    format!("{}.{}", "a".repeat(22), "b".repeat(24))
}
