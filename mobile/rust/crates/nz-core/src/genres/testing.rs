//! What the studio is tested against: a small server on a loopback port that
//! answers the genre routes with the REAL shapes, COUNTS every request, and can
//! refuse one on purpose.
//!
//! Why its own and not `admin::testing`'s or `player::tests`': those answer their
//! own screens' routes and `404` on these, and the acceptance here is a count
//! ("the SPA's own requests, no more") plus a CAUSE ("the refusal reaches the
//! screen"). A fake that speaks the genre routes makes both exact and needs no
//! interpreter, so the mobile CI's Rust job — which installs no Python — runs the
//! same tests.
//!
//! Two of these routes are STATEFUL on the real server (the tree's version and
//! its undo, the embed job), and a stateless fake would let a poll bug pass:
//! [`State`] is that little bit of memory. Nothing waits on the clock: a test
//! reads the request log once its `await` returned.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::core::{Core, CoreConfig};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

/// One test's two roots, removed on drop.
pub struct TempDir {
    path: std::path::PathBuf,
}

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let number = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("nz-genres-{}-{tag}-{number}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("data")).expect("data dir");
        std::fs::create_dir_all(path.join("cache")).expect("cache dir");
        TempDir { path }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// A core over this directory, with a server and a signed-in person: the
    /// studio is the administrator's, and every read below needs a session.
    pub fn core(&self) -> Core {
        Core::new(CoreConfig {
            data_dir: self.path.join("data"),
            cache_dir: self.path.join("cache"),
            app_version: "0.0.0-test".to_owned(),
            version_code: 2_000_001,
            sdk_int: 35,
            device_name: "bench".to_owned(),
        })
        .expect("a core over a throwaway directory")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

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

    pub fn query(&self, key: &str) -> Option<&str> {
        let query = self.path.split_once('?')?.1;
        query.split('&').find_map(|pair| {
            let (name, value) = pair.split_once('=')?;
            (name == key).then_some(value)
        })
    }
}

/// The little bit of memory the real server keeps and a poll depends on.
#[derive(Debug)]
struct State {
    /// The tree's edges, as `(child, parent, weight)` — a write answers the whole
    /// tree, and `undo` puts the last one back.
    edges: Vec<(String, String, f64)>,
    version: u32,
    undo: Option<Vec<(String, String, f64)>>,
    /// `POST /genre/embed` starts it; the first poll sees it running, the second
    /// finished — the shape the card has to survive.
    embed_running: bool,
    embed_polls: u32,
    labels_applied: Vec<(String, Option<i64>)>,
    model: Option<String>,
    /// The library has no hand labels yet (a fresh server).
    no_labels: bool,
}

impl Default for State {
    fn default() -> State {
        State {
            edges: vec![
                ("Frenchcore".into(), "Hardcore".into(), 1.0),
                ("Gabber".into(), "Hardcore".into(), 1.0),
                ("Hardtek".into(), "Hardcore".into(), 0.4),
                ("Hardtek".into(), "Gabber".into(), 0.8),
                ("Hardstyle".into(), "Hardhouse".into(), 1.0),
            ],
            version: 1,
            undo: None,
            embed_running: false,
            embed_polls: 0,
            labels_applied: Vec::new(),
            model: None,
            no_labels: false,
        }
    }
}

/// The fake server: a port, a request log, a per-route refusal, and the state.
pub struct GenreServer {
    address: String,
    log: Arc<Mutex<Vec<Hit>>>,
    broken: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    connections: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
    state: Arc<Mutex<State>>,
    /// The vectors `/genre/embeddings` answers, by reference, as base64 float16.
    vectors: Arc<Mutex<std::collections::HashMap<String, String>>>,
}

impl GenreServer {
    pub fn start() -> GenreServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the genre server");
        let address = format!("http://{}", listener.local_addr().expect("its address"));
        let log = Arc::new(Mutex::new(Vec::new()));
        let broken = Arc::new(Mutex::new(Vec::new()));
        let state = Arc::new(Mutex::new(State::default()));
        let vectors = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let connections: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>> =
            Arc::new(Mutex::new(Vec::new()));
        let shared_log = Arc::clone(&log);
        let shared_broken = Arc::clone(&broken);
        let shared_state = Arc::clone(&state);
        let shared_vectors = Arc::clone(&vectors);
        let stopping = Arc::clone(&stop);
        let shared_connections = Arc::clone(&connections);
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let log = Arc::clone(&shared_log);
                let broken = Arc::clone(&shared_broken);
                let state = Arc::clone(&shared_state);
                let vectors = Arc::clone(&shared_vectors);
                let handle = std::thread::spawn(move || {
                    let Ok(mut stream) = stream.try_clone() else {
                        return;
                    };
                    serve(&mut stream, &log, &broken, &state, &vectors);
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
        GenreServer {
            address,
            log,
            broken,
            stop,
            thread: Some(thread),
            connections,
            state,
            vectors,
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

    /// The body of the last request made with `method` to `route`.
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

    /// Makes `route` answer `status` with the server's own refusal sentence.
    pub fn break_code(&self, route: &str, status: u16, code: &str) {
        self.broken
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .push(format!("{route} {status} {code}"));
    }

    /// Gives one reference a stored vector: the embeddings route only ever serves
    /// what the library measured, so a test writes them here.
    pub fn store_vector(&self, reference: &str, vector: &[f32]) {
        let mut bytes = Vec::with_capacity(vector.len() * 2);
        for value in vector {
            bytes.extend_from_slice(&half(*value).to_le_bytes());
        }
        self.vectors
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(reference.to_owned(), base64(&bytes));
    }

    /// How many labels the server was asked to apply, and with which tag.
    pub fn labels(&self) -> Vec<(String, Option<i64>)> {
        self.state
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .labels_applied
            .clone()
    }

    /// A server nobody has hand-labelled yet.
    pub fn no_labels(&self) {
        self.state
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .no_labels = true;
    }

    /// The tree's edges, as the fake has them now.
    pub fn edges(&self) -> Vec<(String, String, f64)> {
        self.state
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .edges
            .clone()
    }

    /// An in-process server over this bench, with a signed-in administrator.
    pub async fn core(&self, dir: &TempDir) -> Arc<Core> {
        let core = dir.core();
        core.set_server(&self.base())
            .await
            .expect("the server answers /version");
        core.login("bench", "Bench1")
            .await
            .expect("the bench login");
        Arc::new(core)
    }
}

impl Drop for GenreServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
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

/// IEEE 754 binary16, truncating like the page and the desktop do: the bytes a
/// test stores are the bytes the studio decodes.
fn half(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = (((bits >> 23) & 0xff) as i32) - 127 + 15;
    let mant = bits & 0x7f_ffff;
    if exp <= 0 {
        return sign;
    }
    if exp >= 0x1f {
        return sign | 0x7c00;
    }
    sign | ((exp as u16) << 10) | ((mant >> 13) as u16)
}

/// Standard base64, as the server sends it.
pub fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(triple >> 18) as usize & 0x3f] as char);
        out.push(TABLE[(triple >> 12) as usize & 0x3f] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(triple >> 6) as usize & 0x3f] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[triple as usize & 0x3f] as char
        } else {
            '='
        });
    }
    out
}

// --- the answers ---------------------------------------------------------------

const ME: &str = r#"{"user":{"id":"u-admin","name":"Nadia","admin":true,"onboarding":"done","access":{},"sources":[]}}"#;
const VERSION: &str = r#"{"version":"0.1.0-test","api":2}"#;

fn status(state: &State) -> String {
    let model = match &state.model {
        Some(labels) => format!(
            r#"{{"labels":{labels},"usable":true,"version":3,"created":"2026-11-18T10:00:00",
                "metrics":{{"balanced":0.82,"kind":"linear","dim":2560,
                            "health":{{"level":"watch","issues":["weak"]}}}}}}"#
        ),
        None => "null".to_owned(),
    };
    format!(
        r#"{{"archetypes":["groove","voice","dark"],
 "tags":[
  {{"id":1,"name":"Hardcore","color":null,"archetype":"groove","family":"Hardcore"}},
  {{"id":2,"name":"Frenchcore","color":null,"archetype":null,"family":"Hardcore"}},
  {{"id":3,"name":"Gabber","color":null,"archetype":null,"family":"Hardcore"}},
  {{"id":4,"name":"Hardtek","color":null,"archetype":"groove","family":null}},
  {{"id":5,"name":"Hardstyle","color":null,"archetype":null,"family":"Hardstyle"}},
  {{"id":6,"name":"Hardhouse","color":null,"archetype":null,"family":"Hardhouse"}},
  {{"id":7,"name":"Zebda","color":null,"archetype":"voice","family":null}}],
 "counts":{{"Hardcore":30,"Frenchcore":12,"Gabber":8,"Hardtek":6,"Hardstyle":7,
           "Hardhouse":9,"Zebda":2}},
 "labelled":74,
 "model":{model},
 "extractor":{{"available":true,"dim":1280,"model":"discogs-effnet.onnx",
               "onnxruntime":true,"reason":null,"version":3,
               "session_state":"ok","session_error":null,"uploadable":true,
               "model_url":null}}}}"#
    )
}

/// One candidate row, with its ranking — the server's own `_track_json` shape.
fn candidate(reference: &str, title: &str, (first, second): (&str, &str)) -> String {
    format!(
        r#"{{"id":"{reference}","deezer_id":"{reference}","title":"{title}",
 "artist":"Artiste","album":"Album","duration":212,"play_count":7,
 "deezer_bpm":174.0,"predicted":"{first}","confidence":0.42,"uncertainty":0.58,
 "scores":[["{first}",0.52,0.44,0.51],["{second}",0.31,0.22,0.38]]}}"#
    )
}

fn candidates(_state: &State) -> String {
    format!(
        r#"{{"candidates":[{},{}],"labelled":74,"sort":"plays","truncated":false}}"#,
        candidate("1001", "Premier titre", ("Hardtek", "Frenchcore")),
        candidate("1002", "Second titre", ("Frenchcore", "Hardtek"))
    )
}

fn labelled() -> String {
    // Two genres, six tracks each, two artists a genre: enough for the trainer's
    // own gates (`MIN_EXAMPLES` 5, grouped folds when most genres have two
    // artists).
    let mut rows = Vec::new();
    for index in 0..6 {
        let artist = if index % 2 == 0 {
            "a-hardtek"
        } else {
            "b-hardtek"
        };
        rows.push(format!(
            r#"{{"id":"ht{index}","title":"Hardtek {index}","artist":"A{index}",
 "artist_id":"{artist}","tag":{{"id":4,"name":"Hardtek"}}}}"#
        ));
    }
    for index in 0..6 {
        let artist = if index % 2 == 0 {
            "a-french"
        } else {
            "b-french"
        };
        rows.push(format!(
            r#"{{"id":"fc{index}","title":"Frenchcore {index}","artist":"B{index}",
 "artist_id":"{artist}","tag":{{"id":2,"name":"Frenchcore"}}}}"#
        ));
    }
    format!(r#"{{"labelled":[{}]}}"#, rows.join(","))
}

fn tree(state: &State) -> String {
    let nodes = [
        ("Hardcore", Some("Hardcore")),
        ("Frenchcore", Some("Hardcore")),
        ("Gabber", Some("Hardcore")),
        ("Hardtek", None),
        ("Hardhouse", None),
        ("Hardstyle", Some("Hardstyle")),
    ];
    let nodes: Vec<String> = nodes
        .iter()
        .enumerate()
        .map(|(index, (name, family))| {
            format!(
                r#"{{"id":{},"name":"{name}","family":{},"archetype":null,"color":null}}"#,
                index + 1,
                family.map_or_else(|| "null".to_owned(), |family| format!("\"{family}\""))
            )
        })
        .collect();
    let edges: Vec<String> = state
        .edges
        .iter()
        .map(|(child, parent, weight)| {
            format!(r#"{{"child":"{child}","parent":"{parent}","weight":{weight}}}"#)
        })
        .collect();
    let parents: std::collections::BTreeSet<&str> = state
        .edges
        .iter()
        .map(|(_, parent, _)| parent.as_str())
        .collect();
    let roots: Vec<String> = nodes_names()
        .into_iter()
        .filter(|name| !parents.contains(name))
        .map(|name| format!("\"{name}\""))
        .collect();
    format!(
        r#"{{"version":"v{}","default":{},"can_undo":{},"nodes":[{}],"edges":[{}],
 "roots":[{}]}}"#,
        state.version,
        state.undo.is_none(),
        state.undo.is_some(),
        nodes.join(","),
        edges.join(","),
        roots.join(",")
    )
}

fn nodes_names() -> Vec<&'static str> {
    vec![
        "Hardcore",
        "Frenchcore",
        "Gabber",
        "Hardtek",
        "Hardhouse",
        "Hardstyle",
    ]
}

fn embed_running() -> String {
    r#"{"running":true,"scanned":0,"done":0,"failed":0,"skipped":0,"total":120,
 "started":null,"finished":null,"error":null,"force":false,"workers":2,
 "failure_reasons":{},"failures":[]}"#
        .to_owned()
}

fn embed(state: &mut State) -> String {
    // The first poll sees it running, the second finished: the card has to
    // survive both, and a stateless fake would never exercise the second.
    if state.embed_running {
        state.embed_polls += 1;
        if state.embed_polls >= 3 {
            state.embed_running = false;
        }
    }
    let running = state.embed_running;
    format!(
        r#"{{"running":{running},"scanned":120,"done":{},"failed":1,"skipped":0,"total":120,
 "started":null,"finished":null,"error":null,"force":false,"workers":2,
 "failure_reasons":{{"ffmpeg a disparu":1}},
 "failures":[{{"track":"/srv/a.flac","title":"Casse","reason":"ffmpeg a disparu"}}]}}"#,
        if running { 60 } else { 119 }
    )
}

const RELABEL: &str = r#"{"running":false,"scanned":900,"total":900,"changed":31,
 "tag":12,"model":9,"heuristic":8,"family":2,"started":null,"finished":null,"error":null}"#;

const CONSTRUCTION: &str = r#"{"labelled":74,"measured":50,
 "missing":[{"id":"1","bpm":174.0},{"id":"2","bpm":null}]}"#;

const EMBEDDINGS_EMPTY: &str = r#"{"embeddings":{}}"#;

fn embeddings(vectors: &std::collections::HashMap<String, String>, body: &str) -> String {
    let ids: Vec<String> = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("ids").cloned())
        .and_then(|ids| serde_json::from_value::<Vec<String>>(ids).ok())
        .unwrap_or_default();
    let mut out = Vec::new();
    for id in ids {
        if let Some(vector) = vectors.get(&id) {
            out.push(format!("\"{id}\":\"{vector}\""));
        }
    }
    format!(r#"{{"embeddings":{{{}}}}}"#, out.join(","))
}

fn bulk_preview(state: &State) -> String {
    let counts = "{ \"tracks\": 3, \"missing\": 0, \"same\": 1, \"other_tag\": 1, \
                   \"model\": 1, \"no_vector\": 0, \"unjudged\": 0 }";
    format!(
        r#"{{"tag":{{"id":2,"name":"Frenchcore","color":null,"archetype":null}},
 "tracks":[
  {{"id":"2001","deezer_id":"2001","title":"Déjà bon","artist":"A","album":"Al",
    "duration":180,"current":"Frenchcore","same":true,"disagree":null}},
  {{"id":"2002","deezer_id":"2002","title":"Autre étiquette","artist":"B","album":"Al",
    "duration":181,"current":"Gabber","disagree":"tag"}},
  {{"id":"2003","deezer_id":"2003","title":"Le modèle n'est pas d'accord","artist":"C",
    "album":"Al","duration":182,"current":null,"disagree":"model",
    "model":{{"label":"Hardtek","conf":0.91,"level":"label"}}}}],
 "counts":{counts},"model":{{"active":{},"knows":true}}}}"#,
        state.model.is_some()
    )
}

/// One request, one answer, one closed connection (every client reopens).
fn serve(
    stream: &mut TcpStream,
    log: &Mutex<Vec<Hit>>,
    broken: &Mutex<Vec<String>>,
    state: &Mutex<State>,
    vectors: &Mutex<std::collections::HashMap<String, String>>,
) {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
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
    let refusal = broken
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .iter()
        .find_map(|entry| {
            let mut fields = entry.split(' ');
            let name = fields.next()?;
            if name != route {
                return None;
            }
            let status = fields.next()?.parse::<u16>().ok()?;
            let code = fields.next().unwrap_or_default().to_owned();
            Some((status, code))
        });
    let (status, answer) = match refusal {
        Some((status, code)) => (
            status,
            format!(
                "{{\"error\":\"{}\",\"code\":\"{code}\"}}",
                refusal_sentence(&code)
            )
            .into_bytes(),
        ),
        None => answer_for(&method, &path, &route, &body, state, vectors),
    };
    let head = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n",
        answer.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&answer);
    let _ = stream.flush();
}

fn refusal_sentence(code: &str) -> &'static str {
    match code {
        "forbidden" => "Réservé aux administrateurs.",
        "unknown_tag" => "Ce genre n'existe pas.",
        "cycle" => "Ce lien créerait un cycle.",
        _ => "Cette action est impossible.",
    }
}

fn answer_for(
    method: &str,
    _path: &str,
    route: &str,
    body: &str,
    state: &Mutex<State>,
    vectors: &Mutex<std::collections::HashMap<String, String>>,
) -> (u16, Vec<u8>) {
    let ok = |text: String| (200, text.into_bytes());
    let no_content = || (204, Vec::new());
    match (method, route) {
        ("GET", "/api/version") => ok(VERSION.to_owned()),
        ("GET", "/api/me") => ok(ME.to_owned()),
        ("POST", "/api/login") => ok(ME.to_owned()),
        ("GET", "/api/genre/status") => {
            let state = state.lock().unwrap_or_else(|err| err.into_inner());
            ok(status(&state))
        }
        ("GET", "/api/genre/candidates") => {
            let state = state.lock().unwrap_or_else(|err| err.into_inner());
            ok(candidates(&state))
        }
        ("GET", "/api/genre/labelled") => {
            let state = state.lock().unwrap_or_else(|err| err.into_inner());
            if state.no_labels {
                ok(r#"{"labelled":[]}"#.to_owned())
            } else {
                ok(labelled())
            }
        }
        ("GET", "/api/genre/tree") => {
            let state = state.lock().unwrap_or_else(|err| err.into_inner());
            ok(tree(&state))
        }
        ("POST", "/api/genre/tree/edge") | ("DELETE", "/api/genre/tree/edge") => {
            let mut state = state.lock().unwrap_or_else(|err| err.into_inner());
            let value: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
            let child = value
                .get("child")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_owned();
            let parent = value
                .get("parent")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_owned();
            let weight = value
                .get("weight")
                .and_then(|value| value.as_f64())
                .unwrap_or(1.0);
            state.undo = Some(state.edges.clone());
            state.version += 1;
            if method == "POST" {
                if !state
                    .edges
                    .iter()
                    .any(|(c, p, _)| *c == child && *p == parent)
                {
                    state.edges.push((child, parent, weight));
                }
            } else {
                state
                    .edges
                    .retain(|(c, p, _)| !(*c == child && *p == parent));
            }
            ok(tree(&state))
        }
        ("POST", "/api/genre/tree/move") => {
            let mut state = state.lock().unwrap_or_else(|err| err.into_inner());
            let value: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
            let child = value
                .get("child")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_owned();
            let parent = value
                .get("parent")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_owned();
            let weight = value
                .get("weight")
                .and_then(|value| value.as_f64())
                .unwrap_or(1.0);
            state.undo = Some(state.edges.clone());
            state.version += 1;
            state.edges.retain(|(c, _, _)| *c != child);
            state.edges.push((child, parent, weight));
            ok(tree(&state))
        }
        ("POST", "/api/genre/tree/undo") => {
            let mut state = state.lock().unwrap_or_else(|err| err.into_inner());
            if let Some(edges) = state.undo.take() {
                state.edges = edges;
                state.version += 1;
            }
            ok(tree(&state))
        }
        ("POST", "/api/genre/tree/reset") => {
            let mut state = state.lock().unwrap_or_else(|err| err.into_inner());
            state.undo = Some(state.edges.clone());
            state.version += 1;
            state.edges = State::default().edges;
            state.undo = None;
            ok(tree(&state))
        }
        ("POST", "/api/genre/label") => {
            let mut state = state.lock().unwrap_or_else(|err| err.into_inner());
            let value: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
            let reference = value
                .get("track")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_owned();
            let tag = value.get("tag").and_then(|value| value.as_i64());
            state.labels_applied.push((reference, tag));
            ok(r#"{"ok":true}"#.to_owned())
        }
        ("GET", "/api/genre/track/1001") => ok(r#"{"track":"1001",
 "tag":{"id":4,"name":"Hardtek","color":null},
 "scores":[["Hardtek",0.52,0.44,0.51]]}"#
            .to_owned()),
        ("GET", "/api/genre/track/1002") => {
            ok(r#"{"track":"1002","tag":null,"scores":[]}"#.to_owned())
        }
        ("POST", "/api/genre/tags") | ("PATCH", _) if route.starts_with("/api/genre/tags/") => {
            ok(r#"{"ok":true}"#.to_owned())
        }
        ("POST", "/api/genre/tags") => ok(r#"{"ok":true}"#.to_owned()),
        ("DELETE", _) if route.starts_with("/api/genre/tags/") => no_content(),
        ("POST", "/api/genre/tags/defaults") => ok(r#"{"ok":true}"#.to_owned()),
        ("POST", "/api/genre/embeddings") => {
            let vectors = vectors.lock().unwrap_or_else(|err| err.into_inner());
            ok(embeddings(&vectors, body))
        }
        ("GET", "/api/genre/pool") => ok(r#"{"ids":["p1","p2"],"scanned":900}"#.to_owned()),
        ("GET", "/api/genre/embed") => {
            let mut state = state.lock().unwrap_or_else(|err| err.into_inner());
            ok(embed(&mut state))
        }
        ("POST", "/api/genre/embed") => {
            let mut state = state.lock().unwrap_or_else(|err| err.into_inner());
            state.embed_running = true;
            state.embed_polls = 0;
            // The START answers the job it started; only a POLL advances it, which
            // is what makes "a poll never starts work" testable.
            ok(embed_running())
        }
        ("GET", "/api/genre/relabel") => ok(RELABEL.to_owned()),
        ("POST", "/api/genre/relabel") => ok(RELABEL.to_owned()),
        ("GET", "/api/genre/construction") => ok(CONSTRUCTION.to_owned()),
        ("PUT", "/api/genre/model") => {
            let mut state = state.lock().unwrap_or_else(|err| err.into_inner());
            let value: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
            state.model = value.get("labels").map(|labels| labels.to_string());
            ok(
                r#"{"ok":true,"relabel":{"running":true,"scanned":0,"total":900,
 "changed":0,"tag":0,"model":0,"heuristic":0,"family":0}}"#
                    .to_owned(),
            )
        }
        ("DELETE", "/api/genre/model") => {
            let mut state = state.lock().unwrap_or_else(|err| err.into_inner());
            state.model = None;
            no_content()
        }
        ("POST", "/api/genre/bulk/preview") => {
            let state = state.lock().unwrap_or_else(|err| err.into_inner());
            ok(bulk_preview(&state))
        }
        ("POST", "/api/genre/bulk") => ok(r#"{"applied":2,"missing":0}"#.to_owned()),
        ("GET", "/api/genre/extractor/test") => ok(
            r#"{"running":false,"ok":true,"error":null,"progress":[],"result":null}"#.to_owned(),
        ),
        ("POST", "/api/genre/extractor/test") => ok(r#"{"ok":true,"running":true}"#.to_owned()),
        ("GET", _) if route.starts_with("/api/genre/track/") => {
            ok(r#"{"track":"","tag":null,"scores":[]}"#.to_owned())
        }
        ("GET", "/api/analysis/todo") => {
            // An empty page: the measure pass finishes at once, which is what a
            // test needs to see the run's shape without a background job.
            ok(r#"{"version":5,"tracks":[]}"#.to_owned())
        }
        _ => (
            404,
            format!("{{\"error\":\"pas de route {method} {route}\"}}").into_bytes(),
        ),
    }
}

/// The vectors a test can hand the studio: two clusters, so a head can learn them.
pub fn cluster_vector(index: usize, genre: usize) -> Vec<f32> {
    // 2560 values: the v2 width (the studio trains on ONE width, and a v2-only
    // library is the common case on an upgraded server).
    (0..2560)
        .map(|position| {
            let base = if genre == 0 { 1.0 } else { -1.0 };
            base + ((position as f32 + index as f32) * 0.001).sin() * 0.05
        })
        .collect()
}
