//! What this area's tests talk to: a small server on a loopback port that
//! answers the artist, album and credits routes with the REAL shapes, and COUNTS
//! every request it is offered.
//!
//! Why its own and not `core::testing::Bench` (MB2's): that bench answers
//! `/api/version`, the session and a cover, so these routes would be `404` there
//! — and the acceptance of this lot is about what each PAGE asks. A fake that
//! speaks the entity routes makes the numbers exact (three on a cold artist, two
//! on an album, one on a credit page, none from the copy), and needs no
//! interpreter, so the mobile CI's Rust job (which installs no Python) runs the
//! same tests.
//!
//! It answers on a THREAD PER CONNECTION: `Core::discography` may run beside the
//! page read, and a single-threaded accept loop would deadlock the test rather
//! than the code.
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
    /// Used by the credits tests to prove WHICH name was asked for.
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
pub struct EntityServer {
    address: String,
    log: Arc<Mutex<Vec<Hit>>>,
    broken: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    connections: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
    accepted: Arc<AtomicUsize>,
}

impl EntityServer {
    pub fn start() -> EntityServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the entity server");
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
        EntityServer {
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

    /// How many requests this page asked for since the test's last `forget`.
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

impl Drop for EntityServer {
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

const ME: &str = r#"{"user":{"id":"u-entity","name":"Nadia","admin":false,"onboarding":"done","access":{},"sources":[]}}"#;

/// A library artist: its facts, its popular titles, its albums and neighbours,
/// and the count that opens the « Crédits » tab.
const ARTIST: &str = r#"{"artist":{"id":"aaaaaaaa-1111-4111-8111-111111111111","deezer_id":"42",
   "name":"Ascenseur","picture":"/api/cover/aaaaaaaa-1111-4111-8111-111111111111","nb_fan":null,
   "is_favorite":true},
 "bio":null,
 "top":[
  {"id":"11111111-1111-4111-8111-111111111111","deezer_id":"9001","title":"Ouverture","duration":180,
   "artist":{"deezer_id":"42","name":"Ascenseur"},"display_artist":"Ascenseur",
   "album":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"}},
  {"id":"11111111-1111-4111-8111-222222222222","deezer_id":"9002","title":"Suite","duration":200,
   "artist":{"deezer_id":"42","name":"Ascenseur"},
   "album":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"}}],
 "albums":[
  {"id":"bbbbbbbb-1111-4111-8111-111111111111","deezer_id":"501","title":"Le matin",
   "cover":"/api/cover/501","year":2024,"record_type":"album","nb_tracks":2,
   "artist":{"deezer_id":"42","name":"Ascenseur"}}],
 "related":[{"id":"cccccccc-1111-4111-8111-111111111111","deezer_id":"43","name":"June","picture":null}],
 "library":true,"credited":3}"#;

const DISCOGRAPHY: &str = r#"{"discography":{
 "all":[
  {"deezer_id":"501","title":"Le matin","cover":"/api/cover/501","release_date":"2024-03-01","year":"2024","record_type":"album","nb_tracks":2},
  {"deezer_id":"502","title":"Un titre","cover":"/api/cover/502","release_date":"2023-01-01","year":"2023","record_type":"single","nb_tracks":1}],
 "album":[
  {"deezer_id":"501","title":"Le matin","cover":"/api/cover/501","release_date":"2024-03-01","year":"2024","record_type":"album","nb_tracks":2}],
 "single":[
  {"deezer_id":"502","title":"Un titre","cover":"/api/cover/502","release_date":"2023-01-01","year":"2023","record_type":"single","nb_tracks":1}],
 "featured":[
  {"deezer_id":"503","title":"Chez June","year":"2022","record_type":"album","nb_tracks":1}]}}"#;

const ARTIST_ON_DEEZER: &str = r#"{"albums":[
 {"deezer_id":"777","title":"Seulement sur Deezer","cover":"/api/cover/777","year":"2019",
  "record_type":"album","nb_tracks":9,"artist":{"deezer_id":"42","name":"Ascenseur"}}]}"#;

const ARTIST_TRACKS: &str = r#"{"tracks":[
 {"id":"11111111-1111-4111-8111-111111111111","deezer_id":"9001","title":"Ouverture","duration":180,
  "artist":{"deezer_id":"42","name":"Ascenseur"},
  "album":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"}},
 {"id":"11111111-1111-4111-8111-222222222222","deezer_id":"9002","title":"Suite","duration":200,
  "artist":{"deezer_id":"42","name":"Ascenseur"},
  "album":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"}}]}"#;

const RADIO: &str = r#"{"tracks":[
 {"id":"11111111-1111-4111-8111-444444444444","deezer_id":"9004","title":"Radio 1","duration":170,
  "artist":{"deezer_id":"43","name":"June"},
  "album":{"deezer_id":"502","title":"Un titre","cover":"/api/cover/502"}}]}"#;

/// A library album: its header (with the artist's ROW id, which is what a link
/// prefers), its two tracks, and `library: true`.
const ALBUM: &str = r#"{"album":{"id":"bbbbbbbb-1111-4111-8111-111111111111","deezer_id":"501",
   "title":"Le matin","cover":"/api/cover/501","release_date":"2024-03-01","year":2024,
   "artist":{"id":"aaaaaaaa-1111-4111-8111-111111111111","deezer_id":"42","name":"Ascenseur"},
   "nb_tracks":2,"is_favorite":false},
 "tracks":[
  {"id":"11111111-1111-4111-8111-111111111111","deezer_id":"9001","title":"Ouverture","duration":180,
   "artist":{"deezer_id":"42","name":"Ascenseur"},
   "album":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"}},
  {"id":"11111111-1111-4111-8111-222222222222","deezer_id":"9002","title":"Suite","duration":200,
   "artist":{"deezer_id":"42","name":"Ascenseur"},
   "album":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"}}],
 "library":true}"#;

const ALBUM_ON_DEEZER: &str = r#"{"tracks":[
 {"deezer_id":"9009","title":"Bonus","duration":150,
  "artist":{"deezer_id":"42","name":"Ascenseur"},
  "album":{"deezer_id":"501","title":"Le matin"}}]}"#;

const TRACK_CREDITS: &str = r#"{"credits":[
  {"role":"composer","label":"Compositeur","people":[
    {"name":"Nina Simone","id":"dddddddd-1111-4111-8111-111111111111","credited":3}]},
  {"role":"producer","label":"Producteur","people":[{"name":"Bob Power"}]}],
 "info":{"isrc":"FR1234567890","released":"2001-03-12","label":"Blue Note","copyright":"(c) 2001",
   "album":"Le matin"}}"#;

const EMPTY_TRACK_CREDITS: &str = r#"{"credits":[],"info":{}}"#;

const PERSON_CREDITS: &str = r#"{"name":"Nina Simone",
 "roles":[{"role":"composer","label":"Compositeur","count":1}],
 "labels":{"composer":"Compositeur"},
 "tracks":[
  {"id":"11111111-1111-4111-8111-111111111111","deezer_id":"9001","title":"Sinnerman","duration":600,
   "credit_roles":["composer"],
   "artist":{"deezer_id":"42","name":"Nina Simone"},
   "album":{"deezer_id":"501","title":"Pastel Blues","cover":"/api/cover/501"}}]}"#;

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
        ("GET", "/api/artist/42") => ok(ARTIST),
        ("GET", "/api/artist/aaaaaaaa-1111-4111-8111-111111111111") => ok(ARTIST),
        ("GET", "/api/artist/42/tracks") => ok(ARTIST_TRACKS),
        ("GET", "/api/artist/42/discography") => ok(DISCOGRAPHY),
        ("GET", "/api/artist/42/on-deezer") => ok(ARTIST_ON_DEEZER),
        ("GET", "/api/album/501") => ok(ALBUM),
        ("GET", "/api/album/501/on-deezer") => ok(ALBUM_ON_DEEZER),
        ("GET", "/api/radio/artist/42") => ok(RADIO),
        ("GET", "/api/track/11111111-1111-4111-8111-111111111111/credits") => ok(TRACK_CREDITS),
        ("GET", "/api/track/9001/credits") => ok(TRACK_CREDITS),
        ("GET", "/api/track/empty/credits") => ok(EMPTY_TRACK_CREDITS),
        ("GET", "/api/credits/person") => {
            if path.contains("name=Inconnu") {
                (404, json.to_owned(), br#"{"error":"not found"}"#.to_vec())
            } else {
                ok(PERSON_CREDITS)
            }
        }
        ("POST", _) if route.starts_with("/api/favorite/") => {
            let _ = body;
            ok(r#"{"ok":true}"#)
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
            .join("../../target/nz-entity-tests")
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
/// outlive it, and a test that dropped it early would read an empty page.
pub async fn configured() -> (EntityServer, TempDir, super::Core) {
    let server = EntityServer::start();
    let dir = TempDir::new("entity");
    let core = dir.core();
    core.set_server(&server.base())
        .await
        .expect("a reachable server");
    core.login("bench", "Bench1").await.expect("a session");
    (server, dir, core)
}
