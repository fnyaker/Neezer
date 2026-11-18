//! What the administrator's pages are tested against: a small server on a
//! loopback port that answers these routes with the REAL shapes, COUNTS every
//! request it is offered, and can refuse one on purpose.
//!
//! Why its own and not `core::testing::Bench` (MB2's) or `settings::testing`'s
//! (MB16's): those answer their own screens' routes and `404` on these, and the
//! acceptance here is a count ("the SPA's own requests, no more") plus a CAUSE
//! ("the refusal's code reaches the screen"). A fake that speaks the admin routes
//! makes both exact and needs no interpreter, so the mobile CI's Rust job — which
//! installs no Python — runs the same tests.
//!
//! Three of these routes are STATEFUL on the real server (a job starts and the
//! next poll sees it running; a stop ends it), and a stateless fake would let a
//! poll bug pass: [`State`] is that little bit of memory.
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
}

/// The little bit of memory the real server keeps and a poll depends on.
#[derive(Debug)]
struct State {
    analysis_running: bool,
    automatch_running: bool,
    automatch_cancel: bool,
    archive_running: bool,
    migrate_running: bool,
    /// `POST /api/settings` stores this, and the read-back the card makes
    /// (`rule 6`: the screen draws the SERVER's state) answers it.
    quota_gb: f64,
    /// `PUT /api/transcribe/settings` stores this, same rule.
    max_minutes: u32,
}

impl Default for State {
    fn default() -> State {
        State {
            analysis_running: true,
            automatch_running: false,
            automatch_cancel: false,
            archive_running: false,
            migrate_running: false,
            quota_gb: 12.5,
            max_minutes: 180,
        }
    }
}

/// The fake server: a port, a request log, a per-route refusal, and the state.
pub struct AdminServer {
    address: String,
    log: Arc<Mutex<Vec<Hit>>>,
    broken: Arc<Mutex<Vec<String>>>,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    connections: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
}

impl AdminServer {
    pub fn start() -> AdminServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the admin server");
        let address = format!("http://{}", listener.local_addr().expect("its address"));
        let log = Arc::new(Mutex::new(Vec::new()));
        let broken = Arc::new(Mutex::new(Vec::new()));
        let state = Arc::new(Mutex::new(State::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let connections: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>> =
            Arc::new(Mutex::new(Vec::new()));

        let shared_log = Arc::clone(&log);
        let shared_broken = Arc::clone(&broken);
        let shared_state = Arc::clone(&state);
        let stopping = Arc::clone(&stop);
        let shared_connections = Arc::clone(&connections);
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let log = Arc::clone(&shared_log);
                let broken = Arc::clone(&shared_broken);
                let state = Arc::clone(&shared_state);
                let handle = std::thread::spawn(move || {
                    let Ok(mut stream) = stream.try_clone() else {
                        return;
                    };
                    serve(&mut stream, &log, &broken, &state);
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
        AdminServer {
            address,
            log,
            broken,
            state,
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

    /// The body of the last request made with `method` to `route` — a read often
    /// follows a write on the same path, and the two bodies differ.
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

    /// Makes `route` answer `status` from now on (the failure tests).
    pub fn break_route(&self, route: &str, status: u16) {
        self.broken
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .push(format!("{route} {status} "));
    }

    /// Makes `route` answer `status` with the server's own REFUSAL code, which is
    /// what `AdminCopy.guardMessage` turns into its French sentence.
    pub fn break_code(&self, route: &str, status: u16, code: &str) {
        self.broken
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .push(format!("{route} {status} {code}"));
    }

    /// Runs `change` against the server's own state (a switch the answer reads).
    fn with_state(&self, change: impl FnOnce(&mut State)) {
        change(&mut self.state.lock().unwrap_or_else(|err| err.into_inner()));
    }
}

impl Drop for AdminServer {
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
    broken: &Mutex<Vec<String>>,
    state: &Mutex<State>,
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
    let (status, extra, answer) = match refusal {
        Some((status, code)) => (
            status,
            "Content-Type: application/json".to_owned(),
            if code.is_empty() {
                br#"{"error":"broken on purpose"}"#.to_vec()
            } else {
                // The real routes answer the refusal's own French sentence and
                // its machine code (`admin_users.py#_refused`).
                format!(
                    "{{\"error\":\"{}\",\"code\":\"{code}\"}}",
                    refusal_sentence(&code)
                )
                .into_bytes()
            },
        ),
        None => answer_for(&method, &path, &body, state),
    };    let head = format!(
        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\n{extra}\r\nConnection: close\r\n\r\n",
        answer.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&answer);
    let _ = stream.flush();
}

/// The sentence the server answers for a refusal code (`webui/admin_users.py`,
/// `webui/invites.py`) — the one a person reads.
fn refusal_sentence(code: &str) -> &'static str {
    match code {
        "last_admin" => "Impossible : c'est le dernier administrateur actif.",
        "self_delete" => "Tu ne peux pas supprimer ton propre compte.",
        "self_disable" => "Tu ne peux pas désactiver ton propre compte.",
        "invite_used" => "Cette invitation a déjà servi.",
        "invite_not_found" => "Cette invitation n'existe pas.",
        "forbidden" => "Réservé aux administrateurs.",
        _ => "Cette action est impossible.",
    }
}

const ME: &str = r#"{"user":{"id":"u-admin","name":"Nadia","admin":true,"onboarding":"done","access":{},"sources":[]}}"#;

/// Two accounts of the first page of three (the cursor is the last row's id).
const USERS: &str = r#"{"users":[
 {"id":"u-1","name":"Nadia","admin":true,"disabled":false,"jukebox":false,
  "last_login":"2026-11-17T09:30:00","mail":"nadia@example.org","onboarding":"done"},
 {"id":"u-2","name":"Léo","admin":false,"disabled":false,"jukebox":false,
  "last_login":null,"mail":null}],
 "next":"u-2","total":3}"#;

/// The second page: the account the cursor pointed past.
const USERS_PAGE_2: &str = r#"{"users":[
 {"id":"u-3","name":"Sacha","admin":false,"disabled":true,"jukebox":false,
  "last_login":"2026-10-01T08:00:00","mail":null}],
 "next":null,"total":3}"#;

const INVITES: &str = r#"{"invites":[
 {"id":"i-old","note":"Léa","created":"2026-11-16T09:00:00",
  "expires":"2026-11-23T09:00:00","state":"pending","created_by":"Nadia"}],
 "next":null}"#;

const SERVER_SETTINGS: &str = r#"{"upload_quota_gb":12.5,
 "deezer":{"enabled":true,"arl_set":true,"arl_source":"database","arl_hint":"…a1b2",
           "archive_dir_set":true}}"#;

const DEEZER_STATUS: &str = r#"{"ok":true,"account":"nadia","admin":true,"message":null}"#;

const STORAGE: &str = r#"{"archive_dir":"/srv/neezer/archive","archive_bytes":987654321,
 "cache_bytes":10485760,"cache_limit":536870912,"transcode_bytes":52428800,
 "transcode_limit":268435456,"disk_free":300000000000,"disk_total":500000000000,
 "tracks_archived":812,"tracks_total":900}"#;

const ARCHIVE_STATUS: &str = r#"{"running":false,"scope":"all","total":120,"done":120,
 "archived":118,"failed":1,"unavailable":1,"error":null}"#;

const RULES: &str = r#"{"rules":{"on_fav_track":true,"on_fav_album":false,"on_fav_artist":true,
   "on_fav_playlist":true,"on_playlist_add":false,"on_play_context":true,"on_podcast":true,
   "artist_scope":"releases","artist_limit":3,"chapters":true,"chapter_urls":false,
   "clean_on":true,"clean_stale_days":180,"clean_min_free_gb":20.0,"clean_order":"oldest_play",
   "clean_keep_fav":true,"clean_keep_playlist":true,"clean_keep_podcast":false},
  "defaults":{"clean_on":false},
  "events":["on_play_context","on_fav_track","on_fav_album","on_fav_playlist","on_fav_artist",
            "on_playlist_add","on_podcast"],
  "artist_scopes":["all","releases","top"],
  "cleanup_orders":["oldest_play","least_played","largest","oldest"],
  "archive_library":true}"#;

const CLEANUP: &str = r#"{"enabled":true,"running":false,"eligible":9,"would_free":24000000,
 "free_bytes":1000000,"needed_bytes":2147483648,"tracks":[
  {"title":"Vieille prod","artist":"Anonyme","bytes":12000000},
  {"title":"Autre vieille","artist":"Quelqu'un","bytes":12000000}]}"#;

const MIGRATE: &str = r#"{"name":"archive-format","running":false,"done":40,"failed":0,
 "error":null,"counts":{"tags":40,"bpm":12},"failures":[]}"#;

const PLAN: &str = r#"{"where":"server","version":2,"total":900,"todo":30,"stale_measure":5,
 "stale_embed":20,"stale_decide":15,"unmeasured":10,"current":860,"embed_version":2,
 "extractor":false,"schedule":{"enabled":true,"at":"03:00","hours":6}}"#;

fn analysis(running: bool) -> String {
    format!(
        r#"{{"running":{running},"started":null,"finished":null,"force":false,"workers":2,
 "total":50,"scanned":30,"done":28,"skipped":0,"failed":2,
 "error":"a.flac : ffmpeg a disparu","failures":[{{"track":"a.flac","path":"/srv/a.flac",
 "id":"t1","title":"A","reason":"ffmpeg a disparu"}}],
 "failure_reasons":{{"ffmpeg a disparu":1,"fichier tronqué":1}},
 "cancel":false,"stage":"measure","mode":"now","embed_done":0,
 "paused":{paused},"workers_setting":2,"workers_auto":8}}"#,
        running = if running { "true" } else { "false" },
        paused = if running { "\"listening\"" } else { "null" },
    )
}

const ENRICH: &str = r#"{"enabled":true,"running":false,"acoustid":true,"started":null,
 "coverage":{"tried":12,"untried":3,"found":{"isrc":9,"search":2},
             "status":{"ok":11,"none":1}}}"#;

const CREDITS: &str = r#"{"name":"credits","running":true,"done":3,"failed":0,"total":50,
 "scanned":3,"skipped":0,"tracks":7,"completed":true,"error":null,
 "counts":{"deezer":5,"local":2},"failure_reasons":{},"failures":[]}"#;

const SIMILAR: &str = r#"{"running":true,"tracks":12,"started":null,"at":null}"#;

fn automatch(running: bool, cancel: bool, enabled: bool) -> String {
    format!(
        r#"{{"available":true,"enabled":{enabled},"switched_off":false,"threshold":0.99,
 "youtube":true,
 "job":{job},
 "last":{{"running":false,"stopped":null,"counts":{{"match":2,"no_candidate":5}},
          "replaced":[{{"status":"match"}}],
          "failures":[{{"status":"no_candidate",
            "reason":"nothing with the same title and artist",
            "track":{{"id":"1","title":"Perdu","artist":"Anonyme"}}}}]}},
 "recent":[{{"at":"2026-11-17T10:00:00","score":0.995,
             "track":{{"id":"1","title":"Perdu","artist":"Anonyme"}},
             "by":{{"id":"2","title":"Trouvé","artist":"Anonyme","album":"Raretés",
                    "source":"youtube"}}}}]}}"#,
        enabled = if enabled { "true" } else { "false" },
        job = if running {
            format!(
                r#"{{"id":"j1","running":true,"started":"2026-11-17T10:00:00","total":40,
                    "done":12,"counts":{{"match":3}},"reasons":{{}},"replaced":[],
                    "failures":[],"stopped":null,"cancel":{}}}"#,
                if cancel { "true" } else { "false" }
            )
        } else {
            "null".to_owned()
        },
    )
}

const TRANSCRIBE: &str = r#"{"available":true,"enabled":true,"runner":"server","reason":null,
 "language":"fr","languages":["auto","fr","en"],"daily_minutes":60,"max_minutes":180,
 "threads":4,"search_segments":100000,
 "engine":{"name":"whisper.cpp","ok":true,"version":"1.7.4"},
 "model":{"name":"ggml-small.bin","ok":true,"size_mb":466},
 "queue":{"queued":3,"leased":1,"running":1,"ready":9,"error":2,
          "budget_limit_ms":3600000,"budget_used_ms":600000}}"#;

fn transcribe_with(max_minutes: u32) -> String {
    TRANSCRIBE.replace("\"max_minutes\":180", &format!("\"max_minutes\":{max_minutes}"))
}

fn server_settings_with(quota: f64) -> String {
    SERVER_SETTINGS.replace("\"upload_quota_gb\":12.5", &format!("\"upload_quota_gb\":{quota}"))
}

/// The account a `PATCH` answers, with the switch the request asked for applied
/// — the real route answers the row it changed, which is what the card draws.
fn patched_user(body: &str) -> String {
    let admin = body.contains("\"admin\":true");
    let disabled = body.contains("\"disabled\":true");
    format!(
        r#"{{"user":{{"id":"u-2","name":"Léo","admin":{admin},"disabled":{disabled},
             "jukebox":false,"last_login":null,"mail":null}}}}"#,
        admin = if admin { "true" } else { "false" },
        disabled = if disabled { "true" } else { "false" },
    )
}

/// The quota a `POST /api/settings` carried, so the answer is the server's state
/// and not the sample the fake started with.
fn quota_of(body: &str) -> f64 {
    let Some(start) = body.find("\"upload_quota_gb\":") else {
        return 12.5;
    };
    let rest = &body[start + "\"upload_quota_gb\":".len()..];
    let digits: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    digits.parse().unwrap_or(12.5)
}

fn json(status: u16, body: String) -> (u16, String, Vec<u8>) {
    (status, "Content-Type: application/json".to_owned(), body.into_bytes())
}

fn answer_for(
    method: &str,
    path: &str,
    body: &str,
    state: &Mutex<State>,
) -> (u16, String, Vec<u8>) {
    let route = path.split('?').next().unwrap_or(path);
    {
        let mut state = state.lock().unwrap_or_else(|err| err.into_inner());
        match (method, route) {
            ("POST", "/api/analysis/backfill") => state.analysis_running = true,
            ("POST", "/api/analysis/backfill/stop") => state.analysis_running = false,
            ("POST", "/api/automatch/run") => {
                state.automatch_running = true;
                state.automatch_cancel = false;
            }
            ("POST", "/api/automatch/cancel") => state.automatch_cancel = true,
            ("POST", "/api/settings") => state.quota_gb = quota_of(body),
            ("PUT", "/api/transcribe/settings") => {
                if body.contains("\"max_minutes\":30") {
                    state.max_minutes = 30;
                }
            }
            _ => {}
        }
    }
    let snapshot = {
        let state = state.lock().unwrap_or_else(|err| err.into_inner());
        State {
            analysis_running: state.analysis_running,
            automatch_running: state.automatch_running,
            automatch_cancel: state.automatch_cancel,
            archive_running: state.archive_running,
            migrate_running: state.migrate_running,
            quota_gb: state.quota_gb,
            max_minutes: state.max_minutes,
        }
    };

    match (method, route) {
        ("GET", "/api/version") => json(200, r#"{"version":"9.9.9","build":"fake"}"#.to_owned()),
        ("POST", "/api/login") => {
            let (status, extra, body) = json(200, ME.to_owned());
            (
                status,
                format!("{extra}\r\nSet-Cookie: session=good; Path=/"),
                body,
            )
        }
        ("GET", "/api/me") => json(200, ME.to_owned()),

        // -- people ---------------------------------------------------------
        ("GET", "/api/admin/users") => {
            // `after=` continues where the last page stopped; the mobile sends
            // the cursor the server answered, so a fake that ignored it would
            // hide a paging bug.
            if path.contains("after=u-2") {
                json(200, USERS_PAGE_2.to_owned())
            } else {
                json(200, USERS.to_owned())
            }
        }
        ("GET", "/api/admin/users/u-2") => json(
            200,
            r#"{"user":{"id":"u-2","name":"Léo","admin":false,"disabled":false},
                "uploads":4,"playlists":2}"#
                .to_owned(),
        ),
        ("PATCH", "/api/admin/users/u-2") => json(200, patched_user(body)),
        ("DELETE", "/api/admin/users/u-2") => json(
            200,
            r#"{"deleted":true,"uploads_reassigned":4,"podcasts_reassigned":1}"#.to_owned(),
        ),

        // -- invitations ----------------------------------------------------
        ("GET", "/api/admin/invites") => json(200, INVITES.to_owned()),
        ("POST", "/api/admin/invites") => json(
            201,
            r#"{"invite":{"id":"i-new","note":"Léa","created":"2026-11-17T10:00:00",
                "expires":"2026-11-24T10:00:00","state":"pending","created_by":"Nadia"},
                "token":"tok-abc123"}"#
                .to_owned(),
        ),
        ("POST", "/api/admin/invites/i-new/revoke") => json(200, r#"{"revoked":true}"#.to_owned()),

        // -- the server's own settings --------------------------------------
        ("GET", "/api/settings") => json(200, server_settings_with(snapshot.quota_gb)),
        ("POST", "/api/settings") => json(200, server_settings_with(snapshot.quota_gb)),
        ("GET", "/api/deezer/status") => json(200, DEEZER_STATUS.to_owned()),
        ("GET", "/api/storage") => json(200, STORAGE.to_owned()),
        ("POST", "/api/cache/flush") => json(200, r#"{"ok":true,"freed":1048576}"#.to_owned()),

        // -- the archive ----------------------------------------------------
        ("GET", "/api/archive/status") => json(
            200,
            if snapshot.archive_running {
                r#"{"running":true,"scope":"all","total":120,"done":40,"archived":38,
                    "failed":1,"unavailable":0,"error":null}"#
                    .to_owned()
            } else {
                ARCHIVE_STATUS.to_owned()
            },
        ),
        ("POST", "/api/archive/backfill") => {
            json(200, r#"{"ok":true,"running":true}"#.to_owned())
        }
        ("GET", "/api/archive/rules") => json(200, RULES.to_owned()),
        ("POST", "/api/archive/rules") => json(200, RULES.to_owned()),
        ("GET", "/api/archive/cleanup/preview") => json(200, CLEANUP.to_owned()),
        ("POST", "/api/archive/cleanup") => json(
            200,
            r#"{"ok":true,"deleted":2,"freed":24000000,"skipped":null}"#.to_owned(),
        ),
        ("POST", "/api/archive/migrate") => json(200, MIGRATE.to_owned()),
        ("GET", "/api/archive/migrate") => json(200, MIGRATE.to_owned()),

        // -- the analysis ---------------------------------------------------
        ("GET", "/api/analysis/plan") => json(200, PLAN.to_owned()),
        ("PUT", "/api/analysis/schedule") => json(
            200,
            r#"{"ok":true,"schedule":{"enabled":true,"at":"03:00","hours":6}}"#.to_owned(),
        ),
        ("PUT", "/api/analysis/where") => json(200, r#"{"ok":true,"where":"server"}"#.to_owned()),
        ("GET", "/api/analysis/backfill") => json(200, analysis(snapshot.analysis_running)),
        ("POST", "/api/analysis/backfill") => json(200, analysis(true)),
        ("POST", "/api/analysis/backfill/stop") => json(200, analysis(false)),

        // -- the library-wide backfills --------------------------------------
        ("GET", "/api/enrich/backfill") => json(200, ENRICH.to_owned()),
        ("POST", "/api/enrich/backfill") => json(200, ENRICH.to_owned()),
        ("GET", "/api/credits/backfill") => json(200, CREDITS.to_owned()),
        ("POST", "/api/credits/backfill") => json(200, CREDITS.to_owned()),
        ("GET", "/api/similar/rebuild") => json(200, SIMILAR.to_owned()),
        ("POST", "/api/similar/rebuild") => json(200, SIMILAR.to_owned()),

        // -- the auto-match ---------------------------------------------------
        ("GET", "/api/automatch") => json(
            200,
            automatch(
                snapshot.automatch_running,
                snapshot.automatch_cancel,
                true,
            ),
        ),
        ("PUT", "/api/automatch/settings") => json(
            200,
            automatch(false, false, body.contains("\"enabled\":true")),
        ),
        ("POST", "/api/automatch/run") => json(
            200,
            format!(
                r#"{{"ok":true,"job":{}}}"#,
                r#"{"id":"j1","running":true,"total":40,"done":0,"counts":{},"cancel":false}"#
            ),
        ),
        ("POST", "/api/automatch/cancel") => {
            json(200, automatch(true, true, true).replace("\"cancel\":false", "\"cancel\":true"))
        }

        // -- transcription -----------------------------------------------------
        ("GET", "/api/transcribe/settings") => json(200, transcribe_with(snapshot.max_minutes)),
        ("PUT", "/api/transcribe/settings") => json(200, transcribe_with(snapshot.max_minutes)),

        _ => json(404, r#"{"error":"not found"}"#.to_owned()),
    }
}

/// A throwaway data + cache directory for one test.
///
/// Under the WORKSPACE's `target/`, not `/tmp`: the mobile plan's rule 8 says
/// nothing big goes on the RAM-backed tmpfs. Removed on drop, so a passing run
/// leaves nothing behind.
pub struct TempDir {
    path: std::path::PathBuf,
}

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let unique = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nz-admin-tests")
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
/// outlive it, and a test that dropped it early would read an empty cache.
pub async fn configured() -> (AdminServer, TempDir, super::Core) {
    let server = AdminServer::start();
    let dir = TempDir::new("admin");
    let core = dir.core();
    core.set_server(&server.base())
        .await
        .expect("the fake server answers /api/version");
    core.login("bench", "Bench1").await.expect("the fake login");
    // The setup's own requests are not this screen's cost.
    server.forget();
    (server, dir, core)
}
