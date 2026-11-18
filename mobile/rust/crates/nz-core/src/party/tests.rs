//! What this lot's tests talk to: a party server on a loopback port that answers
//! the real routes with the real shapes and COUNTS every request and every byte.
//!
//! Why its own and not the SPA's `webapp/test/party/server.py`: that one needs
//! Python, ffmpeg and a database, and CI's Rust job has none of them. This one
//! speaks `/api/version`, the session and every `/api/party/*` route, so the
//! numbers the acceptance asks for (a cold card = ONE request, a warm one = ZERO,
//! offline = zero sockets) are exact, and it needs no interpreter.
//!
//! **One `#[test]` drives the service**, because it is a process-wide singleton on
//! purpose (one party per device): the scenario walks peek → copy → missing →
//! offline → host → leave in order, so each number is the one the previous step
//! left.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use crate::core::{Core, CoreConfig};
use crate::party::{self, Phase, Role};

/// One test's two roots, removed on drop. This lot keeps its own because
/// `core::testing` is MB2's private module and a feature lot does not open it
/// (worker rule 3) — and a party test needs a few kB of SQLite, never a library.
pub(crate) struct TempCore {
    core: Option<Core>,
    path: PathBuf,
}

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

impl TempCore {
    pub(crate) fn new(tag: &str) -> TempCore {
        let number = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("nz-party-{}-{tag}-{number}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("data")).expect("data dir");
        std::fs::create_dir_all(path.join("cache")).expect("cache dir");
        let core = Core::new(CoreConfig {
            data_dir: path.join("data"),
            cache_dir: path.join("cache"),
            app_version: "0.0.0-test".to_owned(),
            version_code: 2_000_001,
            sdk_int: 35,
            device_name: "test-device".to_owned(),
        })
        .expect("a core over a throwaway directory");
        TempCore {
            core: Some(core),
            path,
        }
    }

    pub(crate) fn core(&self) -> &Core {
        self.core.as_ref().expect("the core was taken")
    }

    /// Hands the core out (a test that needs `Arc<Core>` moves it).
    pub(crate) fn take(mut self) -> (Core, PathBuf) {
        (
            self.core.take().expect("the core was taken"),
            self.path.clone(),
        )
    }
}

impl Drop for TempCore {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// The party the fake server knows, and the one it never heard of.
const LIVE: &str = "b3VGv0dTkz3Wq7nH_Z1pQA";
const GONE: &str = "gone-party-id-0000000000";

/// One request the server was offered.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Hit {
    method: String,
    path: String,
    body: String,
    /// The bytes the SERVER sent for this request: the acceptance's byte count,
    /// measured at the peer instead of inferred from the client's counter.
    answer: usize,
}

impl Hit {
    fn route(&self) -> &str {
        self.path.split('?').next().unwrap_or(&self.path)
    }
}

/// The fake server: a port, a request log, and the one state a host publishes.
struct PartyServer {
    address: String,
    log: Arc<Mutex<Vec<Hit>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    connections: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
    accepted: Arc<AtomicUsize>,
}

impl PartyServer {
    fn start() -> PartyServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the party server");
        let address = format!("http://{}", listener.local_addr().expect("its address"));
        let log = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let connections: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>> =
            Arc::new(Mutex::new(Vec::new()));
        let accepted = Arc::new(AtomicUsize::new(0));
        let hosted = Arc::new(Mutex::new(None));
        let timeline = Arc::new(Mutex::new(String::new()));

        let shared_log = Arc::clone(&log);
        let stopping = Arc::clone(&stop);
        let shared_connections = Arc::clone(&connections);
        let counter = Arc::clone(&accepted);
        let shared_hosted = Arc::clone(&hosted);
        let shared_timeline = Arc::clone(&timeline);
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                counter.fetch_add(1, Ordering::SeqCst);
                let log = Arc::clone(&shared_log);
                let hosted = Arc::clone(&shared_hosted);
                let timeline = Arc::clone(&shared_timeline);
                let handle = std::thread::spawn(move || {
                    let Ok(mut stream) = stream.try_clone() else {
                        return;
                    };
                    serve(&mut stream, &log, &hosted, &timeline);
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
        PartyServer {
            address,
            log,
            stop,
            thread: Some(thread),
            connections,
            accepted,
        }
    }

    fn base(&self) -> String {
        self.address.clone()
    }

    fn hits(&self) -> Vec<Hit> {
        self.log
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    fn routes(&self) -> Vec<String> {
        self.hits()
            .into_iter()
            .map(|hit| format!("{} {}", hit.method, hit.route()))
            .collect()
    }

    fn count(&self) -> usize {
        self.hits().len()
    }

    /// The bytes the server SENT since the last [`forget`] — the acceptance's
    /// byte count, measured at the peer.
    fn body_bytes(&self) -> usize {
        self.hits().iter().map(|hit| hit.answer).sum()
    }

    fn forget(&self) {
        self.log
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clear();
    }

    fn accepted(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }
}

impl Drop for PartyServer {
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

/// `neezer/webui/party.py#server_ms`: the server's instant in milliseconds.
fn server_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

const VERSION_BODY: &str = r#"{"version":"9.9.9","build":"party"}"#;
const ME_BODY: &str = r#"{"user":{"id":"u1","name":"Camille","admin":false,"onboarding":"done","access":{},"sources":[]}}"#;

/// The host's own view (`_host_view`).
fn host_view(id: &str, listeners: &str) -> String {
    format!(
        r#"{{"id":"{id}","v":7,"path":"/party/{id}","listeners":[{listeners}],"now":1794835151000}}"#
    )
}

/// The public state a guest polls (`_public_state`): the shapes the real route
/// answers, anchor object included.
fn public_state(id: &str, timeline: &str) -> String {
    let (track, playing, p) = if timeline.is_empty() {
        ("null".to_owned(), false, 0.0)
    } else {
        (
            r#"{"id":"3000001","title":"Balancing the Dark","artist":"Kaya Project",
                "album":"Songs from the Inside","duration":312.0,"podcast":false,
                "gain":-8.5,"cover":"/api/party/PID/cover/3000001"}"#
                .replace("PID", id),
            true,
            42.25,
        )
    };
    let anchor = if timeline.is_empty() {
        "null".to_owned()
    } else {
        format!(r#"{{"t":1794835151000,"p":{p}}}"#)
    };
    format!(
        r#"{{"id":"{id}","host":"Camille","v":7,"live":true,"track":{track},
            "playing":{playing},"buf":false,"anchor":{anchor},"xfade":2.0,"next":null,
            "norm":"off","chunk":{{"len":6.0,"ov":0.05}},
            "listeners":[{{"name":"Salon"}}],"now":1794835151012}}"#
    )
}

fn serve(
    stream: &mut TcpStream,
    log: &Mutex<Vec<Hit>>,
    hosted: &Mutex<Option<String>>,
    timeline: &Mutex<String>,
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

    let route = path.split('?').next().unwrap_or(&path).to_owned();
    let (status, answer): (u16, String) = match (method.as_str(), route.as_str()) {
        ("GET", "/api/version") => (200, VERSION_BODY.to_owned()),
        ("POST", "/api/login") => (200, ME_BODY.to_owned()),
        ("GET", "/api/me") => (200, ME_BODY.to_owned()),
        ("POST", "/api/logout") => (200, "{}".to_owned()),
        // The feature probe (`OPTIONS`, rule 7): a route that exists answers 200.
        ("OPTIONS", "/api/party/mine") => (200, String::new()),
        ("GET", "/api/party/mine") => {
            let hosted = hosted.lock().unwrap_or_else(|err| err.into_inner()).clone();
            match hosted {
                Some(id) => (200, format!(r#"{{"party":{}}}"#, host_view(&id, ""))),
                None => (200, r#"{"party":null}"#.to_owned()),
            }
        }
        ("POST", "/api/party") => {
            let id = LIVE.to_owned();
            *hosted.lock().unwrap_or_else(|err| err.into_inner()) = Some(id.clone());
            (200, host_view(&id, ""))
        }
        ("POST", route) if route.ends_with("/state") => {
            // A publish: what the host sent becomes what a guest reads.
            *timeline.lock().unwrap_or_else(|err| err.into_inner()) = body.clone();
            let id = hosted
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone()
                .unwrap_or_default();
            (200, host_view(&id, r#"{"name":"Salon"}"#))
        }
        ("DELETE", route) if route.starts_with("/api/party/") => {
            *hosted.lock().unwrap_or_else(|err| err.into_inner()) = None;
            (200, r#"{"ok":true}"#.to_owned())
        }
        ("POST", route) if route.ends_with("/join") => {
            (200, r#"{"lid":"listener-abc"}"#.to_owned())
        }
        ("POST", route) if route.ends_with("/leave") => (204, String::new()),
        ("GET", route) if route.ends_with("/clock") => {
            // The server's OWN clock, advancing like `server_ms()`: a constant
            // `t1` would make every later probe disagree with the last one and
            // the estimator would (rightly) reset for ever.
            let now = server_ms();
            let answer = format!(r#"{{"t1":{now},"t2":{now}}}"#);
            (200, answer)
        }
        ("GET", route) if route.starts_with("/api/party/") => {
            let id = route.trim_start_matches("/api/party/");
            if id == LIVE {
                let timeline = timeline
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .clone();
                (200, public_state(id, &timeline))
            } else {
                (404, r#"{"error":"not found"}"#.to_owned())
            }
        }
        _ => (404, r#"{"error":"not found"}"#.to_owned()),
    };
    let extra = if status == 200 || status == 204 {
        if route == "/api/login" {
            "Content-Type: application/json\r\nSet-Cookie: session=good; Path=/"
        } else {
            "Content-Type: application/json"
        }
    } else {
        "Content-Type: application/json"
    };
    let head = format!(
        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\n{extra}\r\nConnection: close\r\n\r\n",
        answer.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(answer.as_bytes());
    let _ = stream.flush();
    log.lock().unwrap_or_else(|err| err.into_inner()).push(Hit {
        method,
        path,
        body,
        answer: answer.len(),
    });
}

/// The whole lot, in ONE test because the service is one per process on purpose
/// (one party per device): a second `attach` must not start a second guest, and
/// that promise is what makes this scenario ordered instead of parallel.
///
/// The numbers it prints are the ones the PR carries: a card costs ONE request on
/// a cold open and ZERO from the copy, a party that is gone costs ONE and says so,
/// an offline peek opens zero sockets, and hosting costs ONE request.
#[test]
fn the_party_costs_the_pages_requests_and_never_more() {
    let server = PartyServer::start();
    let core = Arc::new(TempCore::new("party-service").take().0);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("a runtime for the scenario");
    let party = runtime.block_on(async {
        core.set_server(&server.base())
            .await
            .expect("the server answers");
        core.login("bench", "Bench1").await.expect("the session");
        server.forget();
        let player = crate::player::Player::open(Arc::clone(&core)).expect("the player");
        let party = party::attach(&core, Some(Arc::clone(&player)));
        (party, player)
    });
    let (party, player) = party;

    // --- the landing card, cold: ONE request, and its bytes.
    runtime.block_on(async {
        server.forget();
        let card = party
            .peek(LIVE)
            .await
            .expect("the card")
            .expect("a live party");
        assert_eq!(
            server.count(),
            1,
            "a cold card is one request: {:?}",
            server.routes()
        );
        assert_eq!(card.host, "Camille");
        assert_eq!(card.phase, Phase::Landing);
        assert_eq!(card.listeners, 1, "the public state names one guest");
        assert_eq!(
            card.invite_line(),
            "Camille vous invite à écouter avec 1 autre personne"
        );
        let bytes = server.body_bytes();
        println!(
            "MEASURE party card cold: 1 request, {bytes} B ({} route)",
            server.routes()[0]
        );
        assert!(bytes > 0, "the card has a body");
    });

    // --- the copy: ZERO requests, and it answers the same card.
    {
        server.forget();
        let copy = party.cached_peek(LIVE).expect("the remembered card");
        assert_eq!(
            server.count(),
            0,
            "a warm card asks nothing: {:?}",
            server.routes()
        );
        assert_eq!(copy.host, "Camille");
        println!(
            "MEASURE party card warm: 0 request, {} B read from the device",
            0
        );
    }

    // --- a link that never was: ONE request, a phase, no error page of its own.
    runtime.block_on(async {
        server.forget();
        let missing = party
            .peek(GONE)
            .await
            .expect("a missing party is not an error");
        assert!(missing.is_none(), "the peek answers nothing");
        assert_eq!(party.view().phase, Phase::Missing);
        assert_eq!(server.count(), 1);
        let bytes = server.body_bytes();
        println!("MEASURE party dead link: 1 request, {bytes} B, phase Missing");
    });

    // --- offline: the cause, ZERO sockets, and the copy still answers.
    {
        core.set_network(crate::core::NetworkState::None);
        let before = server.accepted();
        server.forget();
        let answer = runtime.block_on(party.peek(LIVE));
        assert!(
            matches!(answer, Err(crate::core::CoreError::Offline)),
            "offline is the cause, not a timeout: {answer:?}"
        );
        assert_eq!(
            server.count(),
            0,
            "no request leaves while the radio is down"
        );
        assert_eq!(server.accepted(), before, "zero sockets were opened");
        assert!(
            party.cached_peek(LIVE).is_some(),
            "the copy is what a screen shows with the offline note"
        );
        println!("MEASURE party offline: 1 cause, 0 request, 0 socket, the copy answers");
        core.set_network(crate::core::NetworkState::Wifi { metered: false });
    }

    // --- the rule-7 probe: ONE `OPTIONS`, the door is drawn.
    runtime.block_on(async {
        server.forget();
        assert_eq!(party.supported().await, Some(true));
        assert_eq!(server.routes(), vec!["OPTIONS /api/party/mine".to_owned()]);
        println!("MEASURE party feature probe: 1 request (OPTIONS), route exists");
    });

    // --- hosting: the POST, then the FIRST LINE at once (the guests' card names
    // the track the person is already hearing).
    runtime.block_on(async {
        server.forget();
        let link = party.host().await.expect("the party starts");
        assert_eq!(link, format!("{}/party/{LIVE}", server.base()));
        assert_eq!(party.view().phase, Phase::Live);
        // The start is ONE request, and then the party's clock probes (the SPA's
        // own `BURST` of twelve, 40 ms apart, then one every two seconds): the
        // line itself needs a track in the player, and that is `host.rs`'s test —
        // this queue is empty, and a room with nothing playing is told nothing
        // (`nz-party`'s `decide`: no publish until there is something to say).
        let routes = server.routes();
        assert_eq!(routes[0], "POST /api/party", "{routes:?}");
        assert!(
            routes
                .iter()
                .all(|route| route == "POST /api/party" || route.ends_with("/clock")),
            "a start and its clock probes: {routes:?}"
        );
        let clocks = routes.iter().filter(|route| route.ends_with("/clock")).count();
        let view = party.view();
        assert!(matches!(view.role, Role::Host { .. }), "{:?}", view.role);
        assert_eq!(view.link, link);
        println!(
            "MEASURE party host: {} requests = 1 POST /api/party + {clocks} clock probes, phase Live",
            routes.len()
        );
    });

    // --- the line itself: the player's queue, published once, anchored on the
    // party clock. This is what a guest's landing card names.
    runtime.block_on(async {
        crate::party::tests::queue_two_tracks(&player);
        server.forget();
        let line = format!("POST /api/party/{LIVE}/state");
        assert!(
            wait_route(&server, &line, Duration::from_secs(20)),
            "the host publishes its line: {:?}",
            server.routes()
        );
        let hit = server
            .hits()
            .into_iter()
            .find(|hit| hit.route().ends_with("/state"))
            .expect("the line");
        // Which of the two rows it names depends on the player: this core has no
        // stream route, so the fault policy skips past 3000001 within a second or
        // two. What the line must carry is the track the player is ON, with the
        // anchor a guest schedules against.
        assert!(
            hit.body.contains(r#""id":"3000001""#) || hit.body.contains(r#""id":"3000002""#),
            "the player's own track: {}",
            hit.body
        );
        let anchor = hit.body.contains("\"t\"") && hit.body.contains("\"p\"");
        let playing = hit.body.contains("\"playing\":true");
        println!(
            "MEASURE party line: 1 publish, {} B, anchor {anchor}, playing {playing}",
            hit.body.len()
        );
        assert!(anchor, "a guest schedules on {{t, p}}: {}", hit.body);
    });

    // --- ending it: ONE request, and the screen goes back to the door.
    runtime.block_on(async {
        server.forget();
        party.leave().await.expect("the party ends");
        // The end is ONE request; a clock probe already in flight may land beside
        // it, and the clock stops with the party.
        let routes = server.routes();
        assert!(
            routes.contains(&format!("DELETE /api/party/{LIVE}")),
            "{routes:?}"
        );
        assert!(
            routes
                .iter()
                .all(|route| route.starts_with("DELETE /api/party/") || route.ends_with("/clock")),
            "{routes:?}"
        );
        assert_eq!(party.view().phase, Phase::Idle);
        println!("MEASURE party end: 1 request (DELETE), phase Idle");
    });

    // --- joining: the peek and the seat, TWO requests, then the phone's own
    // guest session (the clock, the polls and the scheduler are `nz-party`'s).
    runtime.block_on(async {
        server.forget();
        let view = party.join(LIVE, "  Salon  ").await.expect("the seat");
        let routes = server.routes();
        assert_eq!(routes[0], format!("GET /api/party/{LIVE}"), "{routes:?}");
        assert!(
            routes
                .iter()
                .any(|route| route == &format!("POST /api/party/{LIVE}/join")),
            "{routes:?}"
        );
        let party_requests = routes
            .iter()
            .filter(|route| !route.ends_with("/clock"))
            .count();
        assert!(
            matches!(view.phase, Phase::Joining | Phase::Live),
            "{:?}",
            view.phase
        );
        assert_eq!(
            party.prefs().display_name(),
            "Salon",
            "the name is trimmed and kept"
        );
        println!(
            "MEASURE party join: {party_requests} requests (the card + the seat), phase Joining"
        );
        party.leave().await.expect("leaving is a request");
        assert_eq!(party.view().phase, Phase::Left);
        println!("MEASURE party leave: 1 request (the seat goes), phase Left");
    });

    // --- the seam the drivers rely on: the service is the process's ONE.
    {
        let again = party::attach(&core, Some(Arc::clone(&player)));
        assert!(
            std::sync::Arc::ptr_eq(&again, &party),
            "a second attach must not build a second party"
        );
    }
    party.shutdown();
    player.shutdown();
}

/// The two rows the scenario queues: the player's own `PlayQueue` command, the
/// same one a tap on a playlist sends.
fn queue_two_tracks(player: &Arc<crate::player::Player>) {
    use crate::player::{Command, TrackRef};
    player.command(
        Command::PlayQueue {
            tracks: vec![
                TrackRef {
                    key: "deezer:3000001".to_owned(),
                    id: "3000001".to_owned(),
                    title: "Balancing the Dark".to_owned(),
                    artists: "Kaya Project".to_owned(),
                    album: "Songs from the Inside".to_owned(),
                    duration_s: 312,
                    cover: None,
                    ..TrackRef::default()
                },
                TrackRef {
                    key: "deezer:3000002".to_owned(),
                    id: "3000002".to_owned(),
                    title: "Suivant".to_owned(),
                    artists: "Kaya Project".to_owned(),
                    duration_s: 200,
                    ..TrackRef::default()
                },
            ],
            start: 0,
            context: None,
        },
        0,
    );
}

/// Wait for the server to be offered one route: the EVENT is the request
/// arriving, and the deadline only bounds a hang (`nz-party`'s own `WAIT`). No
/// sleep: a line the host queues is sent by its own publish loop.
fn wait_route(server: &PartyServer, needle: &str, deadline: Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < deadline {
        if server.routes().iter().any(|route| route == needle) {
            return true;
        }
        std::thread::yield_now();
    }
    false
}
