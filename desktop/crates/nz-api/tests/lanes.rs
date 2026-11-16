//! Rule 1 and rule 3, measured on the wire — against a local server that
//! records WHEN each request arrived and what it carried.
//!
//! These are the lot's acceptance criteria (docs/agents/desktop.md, D2
//! *Acceptance*). Every assertion is a count, an order or a measured gap, never
//! a wall-clock guess: the server keeps the arrival instant and the instant the
//! answer was written, so "the User request is sent first" and "the Prefetch
//! starts 250 ms after the User request ended" are facts about the traffic.
//!
//! No `#[ignore]`: a skip here would hide the very thing the lot exists to
//! prove.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use nz_api::{Api, ApiError, Fresh, Lane, Method, Quality, ResponseCache, SessionJar, TrackId};
use serde_json::Value;

// -- a recording HTTP server ---------------------------------------------------

/// What the server answers one request with.
#[derive(Clone, Debug)]
struct Reply {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: String,
    /// How long the answer is held before it is written (a slow screen).
    delay: Duration,
}

impl Reply {
    fn ok(body: &str) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: body.to_owned(),
            delay: Duration::ZERO,
        }
    }

    fn status(status: u16) -> Self {
        Self {
            status,
            ..Self::ok("{}")
        }
    }

    fn header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }

    fn body(mut self, body: &str) -> Self {
        self.body = body.to_owned();
        self
    }

    fn after(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

/// One request as the server saw it.
#[derive(Clone, Debug)]
struct Hit {
    path: String,
    background: bool,
    accept_encoding: Option<String>,
    range: Option<String>,
    /// When the request line was read.
    at: Instant,
    /// When the answer was written (the request stopped being in flight).
    done: Instant,
}

type Plan = dyn Fn(&str, u32) -> Reply + Send + Sync;

struct Bench {
    port: u16,
    hits: Arc<Mutex<Vec<Hit>>>,
}

impl Bench {
    /// Start a server whose every answer comes from `plan(path, nth-of-path)`.
    fn start(plan: impl Fn(&str, u32) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
        let port = listener.local_addr().unwrap().port();
        let hits: Arc<Mutex<Vec<Hit>>> = Arc::new(Mutex::new(Vec::new()));
        let plan: Arc<Plan> = Arc::new(plan);
        let recorded = Arc::clone(&hits);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let hits = Arc::clone(&recorded);
                let plan = Arc::clone(&plan);
                thread::spawn(move || serve(stream, &hits, plan.as_ref()));
            }
        });
        Self { port, hits }
    }

    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn hits(&self) -> Vec<Hit> {
        self.hits.lock().unwrap().clone()
    }

    fn count(&self) -> usize {
        self.hits.lock().unwrap().len()
    }
}

fn serve(mut stream: TcpStream, hits: &Mutex<Vec<Hit>>, plan: &Plan) {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let Ok(read) = stream.read(&mut chunk) else {
            return;
        };
        if read == 0 {
            return;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(at) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break at + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let mut lines = head.split("\r\n");
    let path = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .unwrap_or("/")
        .to_owned();
    let mut background = false;
    let mut accept_encoding = None;
    let mut range = None;
    let mut length = 0usize;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().to_owned();
        match name.to_ascii_lowercase().as_str() {
            "x-ns-background" => background = value == "1",
            "accept-encoding" => accept_encoding = Some(value),
            "range" => range = Some(value),
            "content-length" => length = value.parse().unwrap_or(0),
            _ => {}
        }
    }
    // Read the body too, or a POST's write hits a closed socket.
    while buffer.len() < header_end + length {
        let Ok(read) = stream.read(&mut chunk) else {
            return;
        };
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }

    let at = Instant::now();
    let (position, nth) = {
        let mut hits = hits.lock().unwrap();
        let nth = hits.iter().filter(|hit| hit.path == path).count() as u32;
        hits.push(Hit {
            path: path.clone(),
            background,
            accept_encoding,
            range,
            at,
            done: at,
        });
        (hits.len() - 1, nth)
    };
    let reply = plan(&path, nth);
    if !reply.delay.is_zero() {
        thread::sleep(reply.delay);
    }
    let body = reply.body.as_bytes();
    let mut response = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        reply.status,
        reason(reply.status),
        body.len()
    );
    for (name, value) in &reply.headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }
    response.push_str("Connection: close\r\n\r\n");
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
    let done = Instant::now();
    if let Some(hit) = hits.lock().unwrap().get_mut(position) {
        hit.done = done;
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        401 => "Unauthorized",
        409 => "Conflict",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

// -- a cache and a client ------------------------------------------------------

#[derive(Default)]
struct MemoryCache(Mutex<BTreeMap<(String, String), String>>);

impl ResponseCache for MemoryCache {
    fn get(&self, owner: &str, path: &str) -> Option<String> {
        self.0
            .lock()
            .unwrap()
            .get(&(owner.to_owned(), path.to_owned()))
            .cloned()
    }
    fn put(&self, owner: &str, path: &str, body: &str) {
        self.0
            .lock()
            .unwrap()
            .insert((owner.to_owned(), path.to_owned()), body.to_owned());
    }
    fn delete(&self, owner: &str, path: &str) {
        self.0
            .lock()
            .unwrap()
            .remove(&(owner.to_owned(), path.to_owned()));
    }
}

fn client(bench: &Bench, cache: Arc<MemoryCache>) -> Api {
    Api::builder(bench.base())
        .session(Arc::new(SessionJar::in_memory()))
        .cache(cache)
        .build()
        .expect("a client")
}

// -- the acceptance criteria ---------------------------------------------------

/// **With 8 Bulk requests queued and one User request issued, the User request
/// is sent first.** Measured: the server's arrival order, and that the eight
/// bulk requests never overlap each other (at most one in flight).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_user_request_is_sent_before_eight_queued_bulk_requests() {
    let bench = Bench::start(|_, _| Reply::ok("{}"));
    let api = Arc::new(client(&bench, Arc::new(MemoryCache::default())));

    let mut bulk = Vec::new();
    for index in 0..8u32 {
        let api = Arc::clone(&api);
        bulk.push(tokio::spawn(async move {
            api.get::<Value>(Lane::Bulk, &format!("/bulk/{index}"))
                .await
                .map(|_| ())
        }));
    }
    // Let the eight reach the gate before the person taps anything.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let user = Arc::clone(&api);
    let started = Instant::now();
    let tapped =
        tokio::spawn(async move { user.get::<Value>(Lane::User, "/user").await.map(|_| ()) });
    tapped
        .await
        .expect("join")
        .expect("the User request succeeds");
    let tapped_ms = started.elapsed().as_millis();

    for task in bulk {
        task.await.expect("join").expect("a Bulk request succeeds");
    }

    let hits = bench.hits();
    assert_eq!(hits.len(), 9, "8 bulk + 1 user");
    assert_eq!(
        hits[0].path, "/api/user",
        "the User request is sent FIRST, even with eight Bulk requests already queued"
    );
    assert!(!hits[0].background, "a User request never announces itself");
    assert!(
        hits[1..].iter().all(|hit| hit.background),
        "every background request carries X-NS-Background: 1"
    );
    assert!(
        hits[1..]
            .iter()
            .all(|hit| hit.path.starts_with("/api/bulk/")),
        "and they are exactly the eight bulk ones"
    );
    // Serialised, not a fan-out: each bulk request starts after the previous one
    // was answered (the slot is released before the next is admitted).
    for pair in hits[1..].windows(2) {
        assert!(
            pair[1].at >= pair[0].done,
            "two bulk requests overlapped: {:?} then {:?}",
            pair[0],
            pair[1]
        );
    }
    assert!(
        tapped_ms < 400,
        "the tapped request answered in {tapped_ms} ms: it waited behind the bulk lane"
    );
}

/// **A Prefetch queued while a User request is in flight starts after the User's
/// end + 250 ms.** Measured as the gap between the User answer being written and
/// the Prefetch request arriving.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_prefetch_starts_250_ms_after_the_user_request_ends() {
    let bench = Bench::start(|path, _| {
        if path == "/api/user" {
            // A slow screen: the person is waiting for 200 ms.
            Reply::ok("{}").after(Duration::from_millis(200))
        } else {
            Reply::ok("{}")
        }
    });
    let api = Arc::new(client(&bench, Arc::new(MemoryCache::default())));

    let user = Arc::clone(&api);
    let user =
        tokio::spawn(async move { user.get::<Value>(Lane::User, "/user").await.map(|_| ()) });
    // Long enough that the request is really in flight on the server.
    tokio::time::sleep(Duration::from_millis(80)).await;

    let prefetch = Arc::clone(&api);
    let prefetch = tokio::spawn(async move {
        prefetch
            .get::<Value>(Lane::Prefetch, "/prefetch")
            .await
            .map(|_| ())
    });

    user.await.expect("join").expect("the User request");
    prefetch.await.expect("join").expect("the Prefetch request");

    let hits = bench.hits();
    let user_hit = hits
        .iter()
        .find(|hit| hit.path == "/api/user")
        .expect("the user hit");
    let prefetch_hit = hits
        .iter()
        .find(|hit| hit.path == "/api/prefetch")
        .expect("the prefetch hit");
    let gap = prefetch_hit.at.duration_since(user_hit.done);
    assert!(
        gap >= Duration::from_millis(250),
        "the Prefetch arrived {gap:?} after the User answer: less than the 250 ms quiet period"
    );
    assert!(
        gap < Duration::from_millis(600),
        "the Prefetch arrived {gap:?} after the User answer: the gate stalled instead of waiting 250 ms"
    );
    assert!(prefetch_hit.background, "a Prefetch announces itself too");
}

/// **A background `503 Retry-After: 2` is retried exactly once, after at least
/// 2 s.** Measured: two arrivals, the second ≥ 2 s after the first, and no third.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_background_503_retry_after_is_retried_exactly_once_after_the_delay() {
    let bench = Bench::start(|_, index| {
        if index == 0 {
            Reply::status(503)
                .header("Retry-After", "2")
                .body(r#"{"error":"busy","background":true}"#)
        } else {
            Reply::ok(r#"{"tracks":[]}"#)
        }
    });
    let api = client(&bench, Arc::new(MemoryCache::default()));
    let started = Instant::now();
    api.get::<Value>(Lane::Bulk, "/queued")
        .await
        .expect("the second answer is the real one");
    let elapsed = started.elapsed();

    let hits = bench.hits();
    assert_eq!(hits.len(), 2, "exactly one retry: {hits:?}");
    let gap = hits[1].at.duration_since(hits[0].at);
    assert!(
        gap >= Duration::from_secs(2),
        "the retry came {gap:?} after the refusal: sooner than the Retry-After said"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "the retry waited {elapsed:?}: it must honour Retry-After, not a backoff"
    );
}

/// **A GET while offline returns the cache with 0 network attempts.**
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_get_while_offline_comes_off_the_disk_with_no_network_attempt() {
    let bench = Bench::start(|_, _| Reply::ok(r#"{"folders":[],"playlists":[]}"#));
    let cache = Arc::new(MemoryCache::default());
    cache.put("u1", "/me/playlists", r#"{"folders":[],"playlists":[]}"#);
    let api = client(&bench, Arc::clone(&cache));
    api.set_owner(Some("u1".into()));
    api.set_online(false);

    let fresh: Fresh<nz_model::PlaylistList> = api
        .get(Lane::User, "/me/playlists")
        .await
        .expect("the cached copy");
    assert!(fresh.from_cache, "the answer came from the disk");
    assert!(fresh.value.playlists.is_empty());
    assert_eq!(
        bench.count(),
        0,
        "no request was made at all: a known-dead link is not asked again"
    );
}

/// **A request started while the link is known dead fails fast** — one attempt,
/// no backoff budget (the SPA's `wasOnline` rule).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_request_started_while_offline_never_retries() {
    // Nothing listens on port 1: every attempt is an instant connection refusal.
    let api = Api::builder("http://127.0.0.1:1")
        .session(Arc::new(SessionJar::in_memory()))
        .build()
        .expect("a client");
    api.set_online(false);
    let started = Instant::now();
    let err = api
        .get::<Value>(Lane::User, "/me")
        .await
        .expect_err("a dead port");
    let elapsed = started.elapsed();
    assert_eq!(err, ApiError::Offline);
    assert!(
        elapsed < Duration::from_millis(400),
        "one attempt was allowed {elapsed:?}: a retry would have slept at least 400 ms"
    );
    assert!(!api.is_online(), "a connection failure marks the link dead");
}

/// **A POST is never retried.**
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_post_is_never_retried() {
    let bench = Bench::start(|_, _| {
        Reply::status(503)
            .header("Retry-After", "1")
            .body(r#"{"error":"busy"}"#)
    });
    let api = client(&bench, Arc::new(MemoryCache::default()));
    let started = Instant::now();
    let err = api
        .send::<(), Value>(Lane::User, Method::POST, "/favorite", None)
        .await
        .expect_err("a 503");
    assert!(matches!(err, ApiError::Http { status: 503, .. }), "{err:?}");
    assert_eq!(bench.count(), 1, "a mutation is sent once, ever");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "and it fails at once instead of backing off"
    );
}

/// **`X-NS-Error` gives `Provider` after exactly 1 attempt**, with the server's
/// own code and `retry_in`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_provider_refusal_is_final_after_one_attempt() {
    let bench = Bench::start(|_, _| {
        Reply::status(503)
            .header("X-NS-Error", "deezer-unreachable")
            .header("Retry-After", "30")
            .body(r#"{"error":"Deezer est injoignable","code":"deezer-unreachable","retry_in":12}"#)
    });
    let api = client(&bench, Arc::new(MemoryCache::default()));
    let err = api
        .get::<Value>(Lane::User, "/artist/6789")
        .await
        .expect_err("a refusal");
    match &err {
        ApiError::Provider {
            status,
            code,
            retry_in,
            ..
        } => {
            assert_eq!(*status, 503);
            assert_eq!(code, "deezer-unreachable");
            assert_eq!(*retry_in, 12);
        }
        other => panic!("expected Provider, got {other:?}"),
    }
    assert_eq!(
        bench.count(),
        1,
        "never re-asked: the server already retried"
    );
    assert!(!err.retryable(), "the UI offers no Réessayer for a refusal");
}

/// **A 409 on a stream is `NotReady` with the server's own Retry-After**, and the
/// stream request carries `Accept-Encoding: identity` and the byte range.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cold_stream_is_not_ready_and_a_stream_asks_for_raw_bytes() {
    let stream_calls = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let bench = Bench::start(move |path, _| {
        // The first stream of this track is cold (a 409); the next is the real
        // body. The two requests differ by their query, so count them here.
        if path.starts_with("/api/stream/")
            && stream_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0
        {
            Reply::status(409).header("Retry-After", "2")
        } else {
            Reply::ok("abc")
                .header("Accept-Ranges", "none")
                .header("X-NS-Start", "6.000")
        }
    });
    let api = client(&bench, Arc::new(MemoryCache::default()));
    let id = TrackId::from("3000168");
    let err = api
        .stream(Lane::User, &id, Quality::Opus128, None, None)
        .await
        .expect_err("nothing on disk yet");
    assert_eq!(err, ApiError::NotReady { retry_after: 2 });

    let mut stream = api
        .stream(Lane::User, &id, Quality::Flac, Some(6.0), Some(1024))
        .await
        .expect("the second answer");
    assert_eq!(stream.status, 200);
    assert!(
        stream.is_generated(),
        "Accept-Ranges: none is a generated body"
    );
    assert_eq!(stream.start_s, Some(6.0));
    let mut body = Vec::new();
    while let Some(chunk) = stream.chunk().await.expect("a readable body") {
        body.extend_from_slice(&chunk);
    }
    assert_eq!(body, b"abc");

    let hits = bench.hits();
    assert_eq!(
        hits[1].accept_encoding.as_deref(),
        Some("identity"),
        "a gzip'd body has no usable byte offsets"
    );
    assert_eq!(
        hits[1].range.as_deref(),
        Some("bytes=1024-"),
        "the byte range is asked of a finished file"
    );
    assert_eq!(
        hits[0].path, "/api/stream/3000168?q=OPUS_128",
        "a quality is a q=, and the id is the SPA's own path segment"
    );
    assert_eq!(
        hits[1].path, "/api/stream/3000168?start=6.000",
        "FLAC is the ABSENCE of q= (the server's contract), and start is three decimals"
    );
}

/// The session round trip on the wire: a login keeps the cookie, a `401` ends it
/// and clears the jar, and a `{"error":"remote ended"}` 401 is the LOAN ending,
/// not the person's session.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_login_keeps_the_cookie_and_a_401_ends_the_session() {
    let bench = Bench::start(|path, _| match path {
        "/api/login" => Reply::ok(r#"{"user":{"id":"u1","name":"bench","admin":true}}"#).header(
            "Set-Cookie",
            "session=abc123; Path=/; HttpOnly; SameSite=Lax",
        ),
        "/api/me" => Reply::status(401).body(r#"{"error":"unauthorized"}"#),
        "/api/remote" => Reply::status(401)
            .body(r#"{"error":"remote ended","restored":{"user":{"id":"u1","name":"bench"}}}"#),
        _ => Reply::ok("{}"),
    });
    let api = client(&bench, Arc::new(MemoryCache::default()));
    let me = api.login("bench", "Bench1").await.expect("a login");
    assert_eq!(me.user.name, "bench");
    assert_eq!(
        api.owner().as_deref(),
        Some("u1"),
        "the cache is keyed to the person"
    );
    assert!(api.has_session(), "the cookie is kept");

    let err = api.me().await.expect_err("an ended session");
    assert_eq!(err, ApiError::Unauthorized);
    assert!(!api.has_session(), "a 401 clears the jar");
    assert_eq!(api.owner(), None);

    let err = api
        .get::<Value>(Lane::User, "/remote")
        .await
        .expect_err("a loan ends");
    match err {
        ApiError::RemoteEnded { restored } => {
            assert_eq!(
                restored
                    .as_ref()
                    .and_then(|value| value.pointer("/user/id"))
                    .and_then(|value| value.as_str()),
                Some("u1")
            );
        }
        other => panic!("expected RemoteEnded, got {other:?}"),
    }
}
