//! A playlist server in this process: the routes this lot reads, a second
//! editor who bumps the version, refusals a test asks for, and the counters.
//!
//! Why not `tools/perf_api.py` (the harness MB2's tests use)? Two reasons, both
//! about what the acceptance must PROVE rather than hope:
//!
//! - **A `409` has to be forced deterministically.** The bench's session is real
//!   and a version only moves when another client really writes; here
//!   [`PlaylistServer::bump_version`] IS that other editor, so the conflict test
//!   is a fact and not a race the test hopes for.
//! - **The Rust job runs where no Python exists** (the mobile CI's Rust job
//!   installs none), and a test that silently skips there is a test that never
//!   ran (worker rule 10: no skipped test). This server is `std` only.
//!
//! It speaks the wire `neezer/webui/__init__.py` and `playlist_import.py`
//! answer — the same fields, the same `409` body, the same `_snapshot` — so the
//! conversions under test are the ones the app uses. The REAL server's numbers
//! (requests, bytes, milliseconds on a 4 000-track page) are measured against
//! `tools/perf_api.py --serve` and written in the lot's PR and its section of
//! `docs/agents/mobile.md`; this fake only proves the logic.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::core::{Core, CoreConfig};
use crate::lists::Row;

/// The page every test starts from: the acceptance's worst realistic input.
pub const TRACKS: usize = 4_000;

/// One throwaway data + cache root, removed on drop.
struct TempDir {
    path: std::path::PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> TempDir {
        use std::sync::atomic::AtomicU64;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("nz-playlist-{}-{tag}-{number}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("data")).expect("data dir");
        std::fs::create_dir_all(path.join("cache")).expect("cache dir");
        TempDir { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// The playlist server, one per test.
pub struct PlaylistServer {
    address: String,
    state: Arc<State>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    dirs: Mutex<Vec<TempDir>>,
}

struct State {
    /// The playlist the lot's tests are about.
    id: String,
    tracks: usize,
    /// The stored order, as the refs a write names (the row's own id).
    order: Mutex<Vec<String>>,
    version: AtomicU32,
    members_version: AtomicU32,
    role: Mutex<String>,
    members: Mutex<Vec<(String, String, String)>>,
    share: Mutex<Option<String>>,
    /// The next membership/order write is refused with this status and scope,
    /// when a test sets it (the `409` and `403` paths).
    refuse: Mutex<Option<(u16, String)>>,
    exported: AtomicBool,
    imported: AtomicBool,
    saved: AtomicBool,
    /// Every request, as `METHOD /path`.
    log: Mutex<Vec<String>>,
    accepted: AtomicUsize,
}

impl PlaylistServer {
    pub fn start(tracks: usize) -> PlaylistServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a playlist server");
        let local = listener.local_addr().expect("its address");
        let address = format!("http://{local}");
        // The id is this server's OWN (its port): tests run in parallel in one
        // process, and the open-page registry is shared, so two servers must
        // never name the same playlist.
        let id = format!("pl-{}", local.port());
        let order: Vec<String> = (0..tracks).map(|n| format!("t{n}")).collect();
        let state = Arc::new(State {
            id,
            tracks,
            order: Mutex::new(order),
            version: AtomicU32::new(1),
            members_version: AtomicU32::new(1),
            role: Mutex::new("owner".to_owned()),
            members: Mutex::new(vec![(
                "u1".to_owned(),
                "bench".to_owned(),
                "owner".to_owned(),
            )]),
            share: Mutex::new(None),
            refuse: Mutex::new(None),
            exported: AtomicBool::new(false),
            imported: AtomicBool::new(false),
            saved: AtomicBool::new(false),
            log: Mutex::new(Vec::new()),
            accepted: AtomicUsize::new(0),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::clone(&state);
        let stopping = Arc::clone(&stop);
        let counting = Arc::clone(&state);
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                counting.accepted.fetch_add(1, Ordering::SeqCst);
                serve(&mut stream, &shared);
                if stopping.load(Ordering::SeqCst) {
                    break;
                }
            }
        });
        PlaylistServer {
            address,
            state,
            stop,
            thread: Some(thread),
            dirs: Mutex::new(Vec::new()),
        }
    }

    /// The id of the playlist these tests open.
    pub fn id(&self) -> &str {
        &self.state.id
    }

    /// Its page's path (`/api/playlist/<id>`) — what a request log is counted on.
    pub fn path(&self) -> String {
        format!("/api/playlist/{}", self.state.id)
    }

    /// Its members' path.
    pub fn members_path(&self) -> String {
        format!("/api/playlist/{}/members", self.state.id)
    }

    /// A core configured with this server and logged in as `bench` — the state a
    /// screen finds after `restore()`.
    pub async fn core(&self, tag: &str) -> Core {
        let dir = TempDir::new(tag);
        let core = Core::new(CoreConfig {
            data_dir: dir.path.join("data"),
            cache_dir: dir.path.join("cache"),
            app_version: "0.0.0-test".to_owned(),
            version_code: 2_000_001,
            sdk_int: 35,
            device_name: tag.to_owned(),
        })
        .expect("a core over a throwaway directory");
        core.set_server(&self.address)
            .await
            .expect("the server answers /api/version");
        core.login("bench", "Bench1")
            .await
            .expect("the bench login");
        self.dirs
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .push(dir);
        core
    }

    /// A row shaped like the server's, for a write that appends titles.
    pub fn row(&self, number: usize) -> Row {
        let track = track_value(number);
        super::wire::row_of(
            &serde_json::from_value(track).expect("the fixture track"),
            number as u32,
        )
    }

    /// The server's order, as the refs it stores.
    pub fn order(&self) -> Vec<String> {
        self.state
            .order
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    /// Sockets the peer accepted — the "0 requests" of an offline read, measured
    /// at the server.
    pub fn accepted(&self) -> usize {
        self.state.accepted.load(Ordering::SeqCst)
    }

    /// How many `GET`s hit exactly `path` (a sub-route is another request).
    pub fn gets(&self, prefix: &str) -> usize {
        self.count("GET", prefix)
    }

    /// How many mutations of `method` hit a path starting with `prefix` (a
    /// mutation's path has a sub-route: `/order`, `/tracks`, `/members/<user>`).
    pub fn mutations(&self, method: &str, prefix: &str) -> usize {
        self.state
            .log
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .iter()
            .filter(|line| {
                line.strip_prefix(&format!("{method} "))
                    .and_then(|rest| rest.split(' ').next())
                    .is_some_and(|logged| logged.starts_with(prefix))
            })
            .count()
    }

    fn count(&self, method: &str, path: &str) -> usize {
        self.state
            .log
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .iter()
            .filter(|line| {
                line.strip_prefix(&format!("{method} "))
                    .and_then(|rest| rest.split(' ').next())
                    .is_some_and(|logged| logged == path)
            })
            .count()
    }

    /// The last body a mutation carried, as JSON (an empty object when none).
    pub fn last_body(&self) -> serde_json::Value {
        self.state
            .log
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .iter()
            .rev()
            .find_map(|line| line.split_once(" body="))
            .and_then(|(_, body)| serde_json::from_str(body).ok())
            .unwrap_or_else(|| serde_json::json!({}))
    }

    /// Another editor saved first: our next order write is stale.
    pub fn bump_version(&self) {
        self.state.version.fetch_add(1, Ordering::SeqCst);
        self.state
            .order
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .swap(0, 1);
    }

    /// Somebody else changed the collaborators.
    pub fn bump_members(&self) {
        self.state.members_version.fetch_add(1, Ordering::SeqCst);
        self.state
            .members
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .push(("u9".to_owned(), "june".to_owned(), "viewer".to_owned()));
    }

    /// The caller's role, as `role_for` would answer it on the next read.
    pub fn set_role(&self, role: &str) {
        *self
            .state
            .role
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = role.to_owned();
    }

    /// The export job finishes (`GET /api/services/export/<job>` says so).
    pub fn finish_export(&self) {
        self.state.exported.store(true, Ordering::SeqCst);
    }

    /// The import job finishes matching.
    pub fn finish_import(&self) {
        self.state.imported.store(true, Ordering::SeqCst);
    }
}

impl Drop for PlaylistServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.address.trim_start_matches("http://"));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// One track, as `_db_track` answers it: the row's UUID first, since that is what
/// a write names it by.
pub fn track_value(number: usize) -> serde_json::Value {
    let deezer = 3_000_000 + number;
    serde_json::json!({
        "id": format!("t{number}"),
        "deezer_id": deezer.to_string(),
        "title": format!("Titre {number}"),
        "duration": 180 + (number % 120),
        "explicit": number.is_multiple_of(17),
        "unavailable": false,
        "artist": {"deezer_id": "1000", "name": format!("Artiste {number}")},
        "artists": [{"deezer_id": "1000", "name": format!("Artiste {number}"), "role": "Main"}],
        "album": {
            "deezer_id": format!("2{:04}", number % 400),
            "title": format!("Album {}", number % 400),
            "cover": format!("/api/cover/2{:04}", number % 400),
        },
    })
}

fn page(state: &State, id: &str) -> serde_json::Value {
    let order = state
        .order
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone();
    let tracks: Vec<serde_json::Value> = order
        .iter()
        .filter_map(|reference| reference.strip_prefix('t'))
        .filter_map(|number| number.parse::<usize>().ok())
        .map(track_value)
        .collect();
    let role = state
        .role
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone();
    let members: Vec<serde_json::Value> = state
        .members
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .iter()
        .map(|(id, name, role)| serde_json::json!({"id": id, "name": name, "role": role}))
        .collect();
    let admin = role == "admin";
    let editable = matches!(role.as_str(), "owner" | "editor" | "admin");
    serde_json::json!({
        "playlist": {
            "id": id,
            "deezer_id": null,
            "title": format!("Playlist de {} titres", if id == state.id { state.tracks } else { 4 }),
            "description": "Une description",
            "cover": "/api/cover/2000",
            "owner": "bench",
            "is_favorite": false,
            "nb_tracks": tracks.len(),
            "private_tracks": 0,
            "role": role,
            "admin": admin,
            "editable": editable,
            "can_manage": matches!(role.as_str(), "owner" | "admin"),
            "version": state.version.load(Ordering::SeqCst),
            "members_version": state.members_version.load(Ordering::SeqCst),
            "last_edited_by": "bench",
            "last_edited_at": "2026-11-18T10:00:00",
            "members": members,
        },
        "tracks": tracks,
    })
}

fn members_answer(state: &State) -> serde_json::Value {
    let role = state
        .role
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone();
    let members: Vec<serde_json::Value> = state
        .members
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .iter()
        .map(|(id, name, role)| serde_json::json!({"id": id, "name": name, "role": role}))
        .collect();
    let is_admin = role == "admin";
    serde_json::json!({
        "members": members,
        "members_version": state.members_version.load(Ordering::SeqCst),
        "role": role,
        "can_edit": matches!(role.as_str(), "owner" | "editor" | "admin"),
        "can_manage": matches!(role.as_str(), "owner" | "admin"),
        "admin": is_admin,
    })
}

/// Whether the caller may manage the collaborators (`role_can_manage`).
fn manages(state: &State) -> bool {
    let role = state
        .role
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone();
    matches!(role.as_str(), "owner" | "admin")
}

fn conflict(state: &State, scope: &str) -> serde_json::Value {
    serde_json::json!({
        "error": "conflict",
        "scope": scope,
        "version": state.version.load(Ordering::SeqCst),
        "members_version": state.members_version.load(Ordering::SeqCst),
        "last_edited_by": "june",
        "last_edited_at": "2026-11-18T11:00:00",
    })
}

/// The refusal a test asked for, if any (`(status, scope)`).
fn take_refusal(state: &State) -> Option<(u16, String)> {
    state
        .refuse
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .take()
}

fn serve(stream: &mut TcpStream, state: &State) {
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
    let mut body = buffer[split..].to_vec();
    let head = String::from_utf8_lossy(&buffer[..split]).into_owned();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let target = parts.next().unwrap_or_default().to_owned();
    let length: usize = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse().ok())
        .unwrap_or(0);
    while body.len() < length {
        let mut rest = vec![0u8; length - body.len()];
        match stream.read(&mut rest) {
            Ok(0) | Err(_) => break,
            Ok(read) => body.extend_from_slice(&rest[..read]),
        }
    }
    let text = String::from_utf8_lossy(&body).into_owned();
    let value: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({}));
    let (path, query) = target
        .split_once('?')
        .map_or((target.as_str(), ""), |(path, query)| (path, query));
    {
        let mut log = state.log.lock().unwrap_or_else(|err| err.into_inner());
        if method == "GET" {
            log.push(format!("{method} {path}"));
        } else {
            log.push(format!("{method} {path} body={text}"));
        }
    }

    let (status, answer) = route(&method, path, query, &value, state);
    let body = answer.to_string();
    let cookie = if path == "/api/login" {
        "\r\nSet-Cookie: session=good; Path=/"
    } else {
        ""
    };
    let kind = if path.contains("/list/") {
        "text/csv"
    } else {
        "application/json"
    };
    let head = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: {kind}\r\nContent-Length: {}{cookie}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
    let _ = stream.flush();
}

fn route(
    method: &str,
    path: &str,
    query: &str,
    body: &serde_json::Value,
    state: &State,
) -> (u16, serde_json::Value) {
    use serde_json::json;
    let version = state.version.load(Ordering::SeqCst);
    let members_version = state.members_version.load(Ordering::SeqCst);
    match (method, path) {
        ("GET", "/api/version") => (200, json!({"version": "9.9.9", "build": "test"})),
        ("POST", "/api/login") => (
            200,
            json!({"user": {"id": "u1", "name": "bench", "admin": true,
                           "onboarding": "done", "access": {}, "sources": []}}),
        ),
        ("GET", "/api/me") => (
            200,
            json!({"user": {"id": "u1", "name": "bench", "admin": true,
                           "onboarding": "done", "access": {}, "sources": []}}),
        ),
        ("GET", "/api/export/formats") => (
            200,
            json!({"default": "opus320", "formats": [
                {"id": "opus320", "label": "Opus 320", "ext": "opus"},
                {"id": "flac", "label": "FLAC", "ext": "flac"}]}),
        ),
        ("GET", "/api/services") => (
            200,
            json!({"services": [
                {"id": "spotify", "name": "Spotify", "ready": true, "linked": true,
                 "account": "bench"},
                {"id": "apple", "name": "Apple Music", "ready": false, "linked": false,
                 "file": true}],
                "formats": [
                {"id": "csv", "label": "CSV (TuneMyMusic, Soundiiz…)", "ext": "csv"},
                {"id": "xspf", "label": "XSPF (VLC, lecteurs)", "ext": "xspf"}]}),
        ),
        ("POST", "/api/services/spotify/export") => (202, json!({"job": "job-1"})),
        ("GET", "/api/services/export/job-1") => {
            if state.exported.load(Ordering::SeqCst) {
                (
                    200,
                    json!({"service": "spotify", "stage": "done", "running": false,
                           "total": 4, "done": 4, "matched": 3,
                           "missing": [{"title": "Absent", "artist": "Personne"}],
                           "missing_count": 1,
                           "url": "https://open.spotify.com/playlist/xyz", "error": null,
                           "relink": false}),
                )
            } else {
                (
                    200,
                    json!({"service": "spotify", "stage": "match", "running": true,
                           "total": 4, "done": 0, "matched": 0, "missing": [],
                           "missing_count": 0, "url": null, "error": null, "relink": false}),
                )
            }
        }
        ("GET", path) if path.starts_with("/api/services/list/") => {
            let _ = query;
            (200, json!("Artiste 1 - Titre 1\nArtiste 2 - Titre 2\n"))
        }
        ("POST", "/api/import/playlist") => (202, json!({"id": "job-import"})),
        ("GET", "/api/import/playlist/job-import") => {
            if state.imported.load(Ordering::SeqCst) {
                let items = json!([
                    {"line": {"title": "Titre 0", "artists": ["Artiste 0"],
                              "album": "Album 0", "duration": 180},
                     "track": track_value(0), "via": "local", "sure": true},
                    {"line": {"title": "Titre 1", "artists": ["Artiste 1"],
                              "album": "Album 1", "duration": 181},
                     "track": track_value(1), "via": "deezer", "sure": true},
                    {"line": {"title": "Titre 2", "artists": ["Artiste 2"],
                              "album": "Album 2", "duration": 182},
                     "track": track_value(2), "via": "deezer", "sure": false},
                    {"line": {"title": "Introuvable", "artists": ["Nul"],
                              "album": null, "duration": 0},
                     "track": null, "via": "", "sure": false}
                ]);
                let saved = state.saved.load(Ordering::SeqCst);
                (
                    200,
                    json!({"id": "job-import",
                           "state": if saved { "saved" } else { "done" },
                           "source": "spotify", "source_label": "Spotify",
                           "kind": "playlist", "url": "https://open.spotify.com/playlist/abc",
                           "name": "Une playlist de quelqu'un", "cover": null,
                           "total": 4, "truncated": false, "done": 4, "matched": 3,
                           "error": null,
                           "playlist_id": if saved { json!("playlist-importee") } else { json!(null) },
                           "counts": {"local": 1, "deezer": 2},
                           "items": items}),
                )
            } else {
                (
                    200,
                    json!({"id": "job-import", "state": "matching", "source": "spotify",
                           "source_label": "Spotify", "kind": "playlist",
                           "url": "https://open.spotify.com/playlist/abc",
                           "name": "Une playlist de quelqu'un", "cover": null,
                           "total": 4, "truncated": false, "done": 1, "matched": 1,
                           "error": null, "playlist_id": null,
                           "counts": {"local": 1}, "items": []}),
                )
            }
        }
        ("POST", "/api/import/playlist/job-import/save") => {
            state.saved.store(true, Ordering::SeqCst);
            (200, json!({"ok": true}))
        }
        ("GET", path) if path.starts_with("/api/shared/playlist/") => {
            let token = path.trim_start_matches("/api/shared/playlist/");
            let rows: Vec<serde_json::Value> = (0..6)
                .map(|number| {
                    let mut track = track_value(number);
                    if let Some(object) = track.as_object_mut() {
                        object.insert("mid".to_owned(), json!(format!("t{number}")));
                        object.insert(
                            "cover".to_owned(),
                            json!(format!("/api/shared/playlist/{token}/cover/t{number}")),
                        );
                        // Two of the six are not archived: listed, not playable.
                        object.insert("streamable".to_owned(), json!(number < 4));
                    }
                    track
                })
                .collect();
            (
                200,
                json!({"playlist": {"title": "Partagée", "description": "Par bench",
                                    "cover": "/api/cover/2000", "nb_tracks": 6,
                                    "owner": "bench"},
                       "tracks": rows}),
            )
        }
        ("GET", path)
            if path.starts_with("/api/playlist/")
                && !path.contains("/order")
                && !path.contains("/tracks")
                && !path.ends_with("/members")
                && !path.ends_with("/version")
                && !path.ends_with("/share")
                && !path.ends_with("/recommendations")
                && !path.ends_with("/leave") =>
        {
            (200, page(state, path.trim_start_matches("/api/playlist/")))
        }
        ("GET", path) if path.ends_with("/recommendations") => {
            let tracks: Vec<serde_json::Value> =
                (0..12).map(|number| track_value(9_000 + number)).collect();
            (
                200,
                json!({"playlist": {"id": state.id, "title": "Playlist"},
                       "tracks": tracks}),
            )
        }
        ("GET", path) if path.ends_with("/members") => (200, members_answer(state)),
        ("GET", path) if path.ends_with("/version") => (
            200,
            json!({"version": version, "members_version": members_version,
                   "nb_tracks": state.tracks, "last_edited_by": "bench",
                   "last_edited_at": "2026-11-18T10:00:00"}),
        ),
        ("GET", path) if path.ends_with("/share") => {
            let share = state
                .share
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone();
            (
                200,
                match share {
                    Some(token) => json!({"link": {"token": token,
                        "url": format!("/pl/{token}"),
                        "created": "2026-11-18T10:00:00",
                        "expires": "2026-12-01T00:00:00"}}),
                    None => json!({"link": null}),
                },
            )
        }
        ("POST", path) if path.ends_with("/share") => {
            let token = "tok000000000000".to_owned();
            *state.share.lock().unwrap_or_else(|err| err.into_inner()) = Some(token.clone());
            (
                200,
                json!({"link": {"token": token, "url": format!("/pl/{token}"),
                       "created": "2026-11-18T10:00:00",
                       "expires": "2026-12-01T00:00:00"}}),
            )
        }
        ("DELETE", path) if path.ends_with("/share") => {
            *state.share.lock().unwrap_or_else(|err| err.into_inner()) = None;
            (200, json!({"revoked": 1}))
        }
        ("PUT", path) if path.ends_with("/order") => {
            if let Some((status, scope)) = take_refusal(state) {
                return if status == 409 {
                    (409, conflict(state, &scope))
                } else {
                    (status, json!({"error": "refused"}))
                };
            }
            let expected = body.get("version").and_then(serde_json::Value::as_u64);
            if expected.is_some_and(|expected| expected as u32 != version) {
                return (409, conflict(state, "content"));
            }
            let refs: Vec<String> = body
                .get("tracks")
                .and_then(serde_json::Value::as_array)
                .map(|rows| {
                    rows.iter()
                        .filter_map(|row| row.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            if refs.is_empty() {
                return (400, json!({"error": "no order"}));
            }
            *state.order.lock().unwrap_or_else(|err| err.into_inner()) = refs;
            let next = state.version.fetch_add(1, Ordering::SeqCst) + 1;
            (200, json!({"ok": true, "version": next}))
        }
        ("POST", path) if path.ends_with("/tracks") => {
            if let Some((status, scope)) = take_refusal(state) {
                return if status == 409 {
                    (409, conflict(state, &scope))
                } else {
                    (status, json!({"error": "refused"}))
                };
            }
            let expected = body.get("version").and_then(serde_json::Value::as_u64);
            if expected.is_some_and(|expected| expected as u32 != version) {
                return (409, conflict(state, "content"));
            }
            let refs: Vec<String> = body
                .get("tracks")
                .and_then(serde_json::Value::as_array)
                .map(|rows| {
                    rows.iter()
                        .filter_map(|row| row.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            let added = refs.len();
            state
                .order
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .extend(refs);
            let next = state.version.fetch_add(1, Ordering::SeqCst) + 1;
            let nb_tracks = state
                .order
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .len();
            (
                200,
                json!({"ok": true, "added": added, "version": next, "nb_tracks": nb_tracks}),
            )
        }
        ("DELETE", path) if path.ends_with("/tracks") => {
            if let Some((status, scope)) = take_refusal(state) {
                return if status == 409 {
                    (409, conflict(state, &scope))
                } else {
                    (status, json!({"error": "refused"}))
                };
            }
            let expected = body.get("version").and_then(serde_json::Value::as_u64);
            if expected.is_some_and(|expected| expected as u32 != version) {
                return (409, conflict(state, "content"));
            }
            let mut indexes: Vec<usize> = body
                .get("indexes")
                .and_then(serde_json::Value::as_array)
                .map(|rows| {
                    rows.iter()
                        .filter_map(serde_json::Value::as_u64)
                        .map(|index| index as usize)
                        .collect()
                })
                .unwrap_or_default();
            indexes.sort_unstable();
            indexes.reverse();
            let mut order = state.order.lock().unwrap_or_else(|err| err.into_inner());
            let removed = indexes.len();
            for index in indexes {
                if index < order.len() {
                    order.remove(index);
                }
            }
            drop(order);
            let next = state.version.fetch_add(1, Ordering::SeqCst) + 1;
            (
                200,
                json!({"ok": true, "removed": removed, "version": next}),
            )
        }
        ("PATCH", path) if path.starts_with("/api/playlist/") && !path.contains("/members") => {
            let expected = body.get("version").and_then(serde_json::Value::as_u64);
            if expected.is_some_and(|expected| expected as u32 != version) {
                return (409, conflict(state, "content"));
            }
            let next = state.version.fetch_add(1, Ordering::SeqCst) + 1;
            (200, json!({"ok": true, "version": next}))
        }
        ("POST", path) if path.ends_with("/leave") => (200, json!({"ok": true})),
        ("POST", path) if path.ends_with("/members") => {
            if !manages(state) {
                return (403, json!({"error": "forbidden"}));
            }
            if let Some((status, scope)) = take_refusal(state) {
                return match status {
                    409 => (409, conflict(state, &scope)),
                    403 => (403, json!({"error": "forbidden"})),
                    other => (other, json!({"error": "refused"})),
                };
            }
            let expected = body
                .get("members_version")
                .and_then(serde_json::Value::as_u64);
            if expected.is_some_and(|expected| expected as u32 != members_version) {
                return (409, conflict(state, "members"));
            }
            let name = body
                .get("username")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("invite");
            let role = body
                .get("role")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("editor");
            state
                .members
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .push((format!("u-{name}"), name.to_owned(), role.to_owned()));
            state.members_version.fetch_add(1, Ordering::SeqCst);
            (200, members_answer(state))
        }
        ("PATCH", path) if path.contains("/members/") => {
            if !manages(state) {
                return (403, json!({"error": "forbidden"}));
            }
            if let Some((status, scope)) = take_refusal(state) {
                return match status {
                    409 => (409, conflict(state, &scope)),
                    403 => (403, json!({"error": "forbidden"})),
                    other => (other, json!({"error": "refused"})),
                };
            }
            state.members_version.fetch_add(1, Ordering::SeqCst);
            (200, members_answer(state))
        }
        ("DELETE", path) if path.contains("/members/") => {
            if !manages(state) {
                return (403, json!({"error": "forbidden"}));
            }
            state.members_version.fetch_add(1, Ordering::SeqCst);
            (200, members_answer(state))
        }
        ("DELETE", path) if path.starts_with("/api/playlist/") => (200, json!({"ok": true})),
        _ => (404, json!({"error": "not found"})),
    }
}
