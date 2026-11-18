//! What this area's tests talk to: a small server on a loopback port that
//! answers the podcasts routes with the REAL shapes and COUNTS every request.
//!
//! Why its own and not `core::testing::Bench` (MB2's): that bench answers
//! `/api/version`, the session and a cover, so these routes would be `404` there
//! — and this lot's acceptance IS a count (a show of 2 000 episodes pages by
//! cursor; a warm open costs 0 blocking requests; a transcript read while
//! listening costs 0). A fake that speaks these routes makes the numbers exact,
//! and it needs no interpreter, so the mobile CI's Rust job runs the same tests.
//!
//! The 2 000-episode show is the acceptance's own figure, and its cursor is the
//! one the real server sends: opaque, and passed back unchanged. The fake encodes
//! `offset:<n>` in it, which is all a client may ever do with it (`nz-model`'s
//! `PodcastPage::next` says "already opaque: pass it back as `cursor`").

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::core::Core;

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

/// The episodes the acceptance's big show holds.
pub const BIG_SHOW_EPISODES: usize = 2_000;
/// The page the SPA asks for (`api.js#podcast`'s `limit = 50`).
pub const PAGE: usize = 50;
/// The id the fake gives the big show.
pub const BIG_SHOW: &str = "5ef08406-6ef3-4adf-835b-c9e9927ba70e";
/// The id of one archived episode of the big show.
pub const EPISODE: &str = "31254005-fa75-4784-805b-e3ade4d7a65a";

/// One request the server was offered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub method: String,
    /// The path WITH its query, as it arrived.
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

/// The fake server: a port, a request log, and per-route statuses a test can
/// break on purpose.
pub struct PodcastServer {
    address: String,
    log: Arc<Mutex<Vec<Hit>>>,
    broken: Arc<Mutex<Vec<String>>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    connections: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
    accepted: Arc<AtomicUsize>,
}

impl PodcastServer {
    pub fn start() -> PodcastServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the podcasts server");
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
        PodcastServer {
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
    #[allow(dead_code)]
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

impl Drop for PodcastServer {
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
    let (status, answer) = match broken_status {
        Some(status) => (status, br#"{"error":"broken on purpose"}"#.to_vec()),
        None => answer_for(&method, &route, &path, &body),
    };
    let head = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        answer.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&answer);
    let _ = stream.flush();
}

fn answer_for(method: &str, route: &str, path: &str, body: &str) -> (u16, Vec<u8>) {
    let query = path.split_once('?').map(|(_, query)| query).unwrap_or("");
    let json = |value: String| (200u16, value.into_bytes());
    // The feature probe (`has_route`): a registered route answers OPTIONS with
    // an Allow header, and the client reads that as "this server has it".
    if method == "OPTIONS" {
        return match route {
            "/api/transcribe/status" => (200, Vec::new()),
            _ => (404, br#"{"error":"not found"}"#.to_vec()),
        };
    }
    match (method, route) {
        ("GET", "/api/version") => json(r#"{"version":"9.9.9","api":3}"#.to_owned()),
        ("GET", "/api/me") => json(ME.to_owned()),
        ("POST", "/api/login") => json(ME.to_owned()),
        ("POST", "/api/logout") => (204, Vec::new()),
        ("GET", "/api/podcasts") => json(shows_body().to_owned()),
        ("POST", "/api/podcasts") => json(subscribed_body().to_owned()),
        ("GET", "/api/search/podcasts") => json(search_body().to_owned()),
        ("GET", "/api/podcast/progress") => json(progress_body().to_owned()),
        ("GET", "/api/transcribe/settings") => json(settings_body().to_owned()),
        ("GET", "/api/transcribe/status") => json(status_body().to_owned()),
        ("POST", "/api/podcast/episode/placeholder/transcribe") => (202, b"{}".to_vec()),
        ("POST", "/api/podcast/episode/placeholder/transcribe/stop") => (204, Vec::new()),
        ("DELETE", "/api/podcast/episode/placeholder/transcript") => (204, Vec::new()),
        _ if route.starts_with("/api/podcast/episode/") => {
            episode_answer(method, route, query, body)
        }
        _ if route.starts_with("/api/podcast/marker/") => (204, Vec::new()),
        _ if route.starts_with("/api/podcast/") => show_answer(method, route, path, body),
        _ => (404, br#"{"error":"not found"}"#.to_vec()),
    }
}

fn episode_answer(method: &str, route: &str, query: &str, body: &str) -> (u16, Vec<u8>) {
    // The id the caller named, so a test that asks for ten different episodes
    // gets ten different answers instead of one shared fixture.
    let episode = route
        .trim_start_matches("/api/podcast/episode/")
        .split('/')
        .next()
        .unwrap_or(EPISODE)
        .to_owned();
    if route.ends_with("/markers") {
        return match method {
            "POST" => (
                200,
                format!(
                    r#"{{"ok":true,"marker":{{"id":"m-new","episode_id":"{episode}","position":{},"label":"{}"}}}}"#,
                    field(body, "position"),
                    field(body, "label")
                )
                .into_bytes(),
            ),
            _ => (200, markers_body(&episode).into_bytes()),
        };
    }
    if route.ends_with("/chapters") {
        return (200, chapters_body(&episode).into_bytes());
    }
    if route.ends_with("/transcribe") {
        return (202, b"{\"queued\":1}".to_vec());
    }
    if route.ends_with("/transcribe/stop") {
        return (204, Vec::new());
    }
    if route.ends_with("/transcript") {
        return match method {
            "DELETE" => (204, Vec::new()),
            _ => (200, transcript_body(&episode, query).into_bytes()),
        };
    }
    (404, br#"{"error":"not found"}"#.to_vec())
}

fn show_answer(method: &str, route: &str, path: &str, _body: &str) -> (u16, Vec<u8>) {
    if route.ends_with("/transcript/search") {
        return (200, transcript_search_body(path).into_bytes());
    }
    if route.ends_with("/auto_archive") {
        return (200, auto_archive_body().into_bytes());
    }
    if route.ends_with("/markers") {
        return (200, show_markers_body().into_bytes());
    }
    if route.ends_with("/transcribe") {
        return (
            202,
            r#"{"id":"5ef08406-6ef3-4adf-835b-c9e9927ba70e","title":"Les Histoires",
                "auto_transcribe":true,"transcribe_language":"fr","backlog":"recent",
                "queued":12,"skipped":1}"#
                .as_bytes()
                .to_vec(),
        );
    }
    match method {
        "DELETE" => (200, br#"{"ok":true,"kept":8,"archived":true}"#.to_vec()),
        _ => {
            let (limit, cursor) = page_params(path);
            (200, show_page_body(limit, cursor).into_bytes())
        }
    }
}

/// The `limit` and `cursor` of a show request, as the routes read them.
fn page_params(path: &str) -> (usize, usize) {
    let query = path.split_once('?').map(|(_, query)| query).unwrap_or("");
    let limit = query
        .split('&')
        .find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (key == "limit").then(|| value.parse::<usize>().ok())?
        })
        .unwrap_or(PAGE)
        .clamp(1, 200);
    let cursor = query
        .split('&')
        .find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (key == "cursor").then_some(value.to_owned())
        })
        .and_then(|value| {
            value
                .strip_prefix("offset%3A")
                .or_else(|| value.strip_prefix("offset:"))
                .map(str::to_owned)
        })
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    (limit, cursor)
}

/// One JSON string field of a request body, as a test writes it.
fn field(body: &str, name: &str) -> String {
    let needle = format!("\"{name}\":");
    body.split(&needle)
        .nth(1)
        .map(|rest| {
            rest.trim_start()
                .split([',', '}'])
                .next()
                .unwrap_or("0")
                .trim()
                .trim_matches('"')
                .to_owned()
        })
        .unwrap_or_else(|| "0".to_owned())
}

const ME: &str = r#"{"user":{"id":"u-podcasts","name":"Nadia","admin":true,"onboarding":"done","access":{},"sources":[]}}"#;

fn shows_body() -> String {
    format!(
        r#"{{"podcasts":[
            {{"id":"{BIG_SHOW}","deezer_id":"9400001","title":"Les Histoires",
              "description":"Une heure de récits","cover":"/api/cover/{BIG_SHOW}",
              "subscribed":true,"local":false,"status":"ok","episode_count":{BIG_SHOW_EPISODES},
              "archived_count":8,"auto_archive":true,"auto_transcribe":false,
              "transcribe_language":"auto"}},
            {{"id":"7c7f9b1e-0000-4000-8000-000000000001","deezer_id":"9400002",
              "title":"Météo du soir","description":"","cover":null,"subscribed":true,
              "local":false,"status":"ok","episode_count":12,"archived_count":12,
              "auto_archive":false,"auto_transcribe":true,"transcribe_language":"fr"}}
        ]}}"#
    )
}

fn subscribed_body() -> String {
    String::from(
        r#"{"id":"7c7f9b1e-0000-4000-8000-000000000002","deezer_id":"9400003",
            "title":"Nouveau podcast","description":"","cover":null,"subscribed":true,
            "local":false,"status":"ok","episode_count":3,"archived_count":0,
            "auto_archive":false,"auto_transcribe":false,"transcribe_language":"auto",
            "episodes":[],"next":null}"#,
    )
}

fn search_body() -> String {
    r#"{"podcasts":[{"deezer_id":"9400001","title":"Les Histoires",
        "description":"Une heure de récits","cover":"/api/cover/9400001","fans":1200}]}"#
        .to_owned()
}

fn auto_archive_body() -> String {
    format!(
        r#"{{"id":"{BIG_SHOW}","deezer_id":"9400001","title":"Les Histoires",
            "description":"","cover":null,"subscribed":true,"local":false,"status":"ok",
            "episode_count":{BIG_SHOW_EPISODES},"archived_count":9,"auto_archive":true,
            "auto_transcribe":false,"transcribe_language":"auto","queued":17}}"#
    )
}

fn progress_body() -> String {
    format!(
        r#"{{"progress":{{"{EPISODE}":{{"position":1200,"duration":3600,"finished":false,
            "updated":1794960000}}}}}}"#
    )
}

fn markers_body(episode: &str) -> String {
    format!(
        r#"{{"markers":[{{"id":"m-1","episode_id":"{episode}","position":600000,
            "label":"La maison","created":"2026-11-16T14:17:29"}}]}}"#
    )
}

fn show_markers_body() -> String {
    format!(
        r#"{{"markers":{{"{EPISODE}":[{{"id":"m-1","episode_id":"{EPISODE}",
            "position":600000,"label":"La maison","created":"2026-11-16T14:17:29"}}]}}}}"#
    )
}

fn chapters_body(episode: &str) -> String {
    format!(
        r#"{{"episode_id":"{episode}","archived":true,"scanned":true,"source":"id3",
            "duration":3600,"chapters":[
              {{"index":0,"start":0,"end":900,"title":"Ouverture"}},
              {{"index":1,"start":900,"end":1800,"title":"Le village"}},
              {{"index":2,"start":1800,"end":null,"title":""}}
            ]}}"#
    )
}

/// One page of the ready transcript: 240 lines, 15 s each.
fn transcript_body(episode: &str, query: &str) -> String {
    let limit = query_value(query, "limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(200)
        .clamp(1, 500);
    let screened = query_value(query, "q");
    let mut total = 240usize;
    let mut from = query_value(query, "offset")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    if let (Some(at), true) = (
        query_value(query, "at").and_then(|v| v.parse::<f64>().ok()),
        screened.is_none(),
    ) {
        let index = (at / 15.0).floor().max(0.0) as usize;
        from = (index.min(total.saturating_sub(1)) / limit) * limit;
    }
    let words = screened.unwrap_or_default();
    if !words.is_empty() {
        total = 3;
        from = 0;
    }
    let mut segments = Vec::new();
    for index in from..(from + limit).min(total) {
        let start = index as f64 * 15.0;
        segments.push(format!(
            r#"{{"start":{start},"end":{},"text":"Ligne {index}{}"}}"#,
            start + 15.0,
            if words.is_empty() {
                String::new()
            } else {
                format!(" [{}] ici", words)
            }
        ));
    }
    let next = if from + segments.len() < total {
        (from + segments.len()).to_string()
    } else {
        "null".to_owned()
    };
    format!(
        r#"{{"episode_id":"{episode}","status":"ready","archived":true,"language":"fr",
            "engine":"whisper.cpp","model":"ggml-small.bin","source":"server","worker":null,
            "duration_ms":3600000,"done_ms":3600000,"progress":1.0,"total":{total},
            "offset":{from},"limit":{limit},"query":{},
            "segments":[{}],"next":{next},"error":null}}"#,
        match &words {
            value if value.is_empty() => "null".to_owned(),
            value => format!("\"{value}\""),
        },
        segments.join(","),
    )
}

fn transcript_search_body(path: &str) -> String {
    let query = path.split_once('?').map(|(_, query)| query).unwrap_or("");
    let words = query_value(query, "q").unwrap_or_default();
    format!(
        r#"{{"q":"{words}","hits":[{{"idx":12,"start":180.0,"end":195.0,
            "text":"…[la maison]…","score":1.0,"episode_id":"{EPISODE}",
            "episode":{{"id":"{EPISODE}","title":"Épisode 12","cover":null,
              "published":1794748649}},"channel_id":"{BIG_SHOW}",
            "channel_title":"Les Histoires"}}],"total":1,"more":false,
            "scope":{{"episodes":24,"truncated":false}}}}"#
    )
}

fn settings_body() -> String {
    r#"{"available":true,"enabled":true,"runner":"server","reason":null,"language":"fr",
        "languages":["auto","fr","en","es","de","it","pt"],"daily_minutes":120,
        "max_minutes":240,"threads":4,"search_segments":1000,
        "queue":{"queued":1,"leased":0,"running":1,"ready":8,"error":0,"stopped":0,
          "budget_limit_ms":7200000,"budget_used_ms":0},
        "engine":{"name":"whisper.cpp","ok":true,"version":"1.0"},
        "model":{"name":"ggml-small.bin","ok":true,"size_mb":466}}"#
        .to_owned()
}

fn status_body() -> String {
    r#"{"available":true,"enabled":true,"runner":"server","reason":null,"language":"fr",
        "model":"ggml-small.bin","daily_minutes":120,"max_minutes":240,
        "search_segments":1000,"queue":{"queued":1,"leased":0,"running":1,"ready":8,
          "error":0,"stopped":0,"budget_limit_ms":7200000,"budget_used_ms":0},
        "last_error":null}"#
        .to_owned()
}

/// One show page: the header plus `limit` episodes from `cursor`, and the next
/// cursor (opaque, `offset:<n>` here).
fn show_page_body(limit: usize, cursor: usize) -> String {
    let mut episodes = Vec::with_capacity(limit);
    let end = (cursor + limit).min(BIG_SHOW_EPISODES);
    for index in cursor..end {
        let archived = index % 250 == 0;
        episodes.push(format!(
            r#"{{"id":"ep-{index:05}","deezer_id":"ep-{index:05}","title":"Épisode {index}",
              "description":"Un épisode de plus.","duration":3600,
              "channel_id":"{BIG_SHOW}","cover":null,"published":{},
              "archived":{archived},"unavailable":false,"explicit":false,"podcast":true,
              "status":"{}","chapter_count":{},"transcript":{},"artist":null,"album":null}}"#,
            1_794_960_000i64 - (index as i64 * 3_600),
            if archived { "completed" } else { "" },
            if index % 250 == 0 { 3 } else { 0 },
            if index % 250 == 0 {
                r#"{"status":"ready","source":"server","language":"fr","progress":1.0,"segment_count":240,"error":null}"#
            } else {
                "null"
            },
        ));
    }
    // The cursor is OPAQUE to the client and goes back into a query string
    // unchanged (`nz-api` percent-encodes it once, like `URLSearchParams`), so
    // the fake states it PLAIN here: an already-escaped cursor would be escaped
    // a second time and never match again.
    let next = if end < BIG_SHOW_EPISODES {
        format!("\"offset:{end}\"")
    } else {
        "null".to_owned()
    };
    format!(
        r#"{{"id":"{BIG_SHOW}","deezer_id":"9400001","title":"Les Histoires",
            "description":"Une heure de récits","cover":"/api/cover/{BIG_SHOW}",
            "subscribed":true,"local":false,"status":"ok",
            "episode_count":{BIG_SHOW_EPISODES},"archived_count":8,"auto_archive":true,
            "auto_transcribe":false,"transcribe_language":"auto","transcribed_count":8,
            "episodes":[{}],"next":{next}}}"#,
        episodes.join(",")
    )
}

/// One query parameter, percent-decoded enough for a test's own values.
fn query_value(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        if key != name {
            return None;
        }
        Some(
            value
                .replace("%20", " ")
                .replace("%3A", ":")
                .replace("%2F", "/")
                .replace('+', " "),
        )
    })
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
            .join("../../target/nz-podcast-tests")
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
pub async fn configured(tag: &str) -> (PodcastServer, TempDir, Core) {
    let server = PodcastServer::start();
    let dir = TempDir::new(tag);
    let core = dir.core();
    core.set_server(&server.base())
        .await
        .expect("a reachable server");
    core.login("bench", "Bench1").await.expect("a session");
    server.forget();
    (server, dir, core)
}
