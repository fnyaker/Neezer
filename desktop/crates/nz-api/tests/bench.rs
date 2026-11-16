//! The real server, not a stub: `tools/perf_api.py --serve` on a throwaway
//! SQLite database, exactly as D1's fixture script starts it.
//!
//! What this proves is the part a mock cannot: the SPA's session contract
//! survives a real Flask cookie (login → `/api/me` → `401` → logout), and real
//! payloads come back through the typed endpoints and `nz-model`'s types.
//!
//! The bench needs an interpreter that can `import neezer` (the project
//! installed, `pip install -e .`). `NZ_PYTHON=/path/to/python` overrides the
//! search; otherwise `python3` and the repository's `.venv` are tried. There is
//! no `#[ignore]`: the desktop job installs the server, and a skipped test here
//! would hide a broken session round trip.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use nz_api::{Api, ApiError, Lane, Quality, SessionJar, TrackId};

/// The repository root (`desktop/crates/nz-api` → up three).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("the repository root")
}

/// The first interpreter that can import the server.
fn python() -> String {
    let candidates: Vec<String> = std::env::var("NZ_PYTHON")
        .ok()
        .into_iter()
        .chain([
            "python3".to_owned(),
            repo_root().join(".venv/bin/python").display().to_string(),
            repo_root().join("venv/bin/python").display().to_string(),
        ])
        .collect();
    for candidate in &candidates {
        let usable = Command::new(candidate)
            .args(["-c", "import peewee, neezer"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if usable {
            return candidate.clone();
        }
    }
    panic!(
        "no Python interpreter can `import neezer` (tried: {}). Install the server: \
         `python3 -m venv .venv && .venv/bin/pip install -e .`, or point NZ_PYTHON at one.",
        candidates.join(", ")
    );
}

/// A `perf_api.py` server on a throwaway database.
struct Bench {
    child: Child,
    port: u16,
    ids: serde_json::Value,
}

impl Bench {
    fn start() -> Self {
        let script = repo_root().join("tools/perf_api.py");
        let mut child = Command::new(python())
            .arg(script)
            // The seed is small on purpose: this test proves the SESSION and the
            // typed routes, not the size of a library.
            .args([
                "--tracks",
                "12",
                "--playlists",
                "2",
                "--favorites",
                "4",
                "--big-playlist",
                "4",
                "--serve",
                "--port",
                "0",
            ])
            .current_dir(repo_root())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .stdin(Stdio::null())
            .spawn()
            .expect("start perf_api.py");

        let stdout = child.stdout.take().expect("its stdout");
        let (sender, receiver) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let _ = sender.send(line);
            }
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(180);
        let mut port = None;
        let mut ids = serde_json::Value::Null;
        while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
            match receiver.recv_timeout(left) {
                Ok(line) if line.starts_with("READY ") => {
                    let mut parts = line.splitn(3, ' ');
                    parts.next();
                    port = parts.next().and_then(|port| port.parse::<u16>().ok());
                    ids = parts
                        .next()
                        .and_then(|raw| serde_json::from_str(raw).ok())
                        .unwrap_or(serde_json::Value::Null);
                    break;
                }
                Ok(_) => continue,
                Err(_) => break,
            }
        }
        let port = port.expect("perf_api.py never printed READY");
        Self { child, port, ids }
    }

    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn first_track(&self) -> TrackId {
        let id = self.ids["ids"]
            .get(0)
            .unwrap_or_else(|| panic!("the bench printed no track ids: {}", self.ids));
        TrackId::from(match id {
            serde_json::Value::String(text) => text.clone(),
            other => other.to_string(),
        })
    }
}

impl Drop for Bench {
    fn drop(&mut self) {
        stop(&mut self.child);
    }
}

/// Stop the bench the way it expects: SIGTERM first, SIGKILL only as a last
/// resort.
///
/// `tools/perf_api.py` removes its throwaway library (5 MB for a small seed,
/// 2.3 GB for the 20 000-track one) when it is asked to stop, and `/tmp` is a
/// tmpfs on most machines: `Child::kill()` is SIGKILL, which cannot be caught
/// and left that library to the next launch's sweep. Measured 2026-11-17:
/// 20 of 20 SIGTERM stops left no directory at all.
fn stop(child: &mut Child) {
    const SIGTERM: i32 = 15;
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    // SAFETY: `kill` is handed the pid of a child this struct owns and the
    // platform's own SIGTERM number, and nothing else.
    unsafe {
        kill(child.id() as i32, SIGTERM);
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn client(bench: &Bench) -> Api {
    Api::builder(bench.base())
        .session(Arc::new(SessionJar::in_memory()))
        .build()
        .expect("a client")
}

/// **Login + me + 401 round trip against `perf_api.py`.**
///
/// Counted: one real session cookie, one `/api/me` that carries it, one `401`
/// on a client that has none, and a logout that clears it. Plus the typed
/// routes decoding the real server's payloads through `nz-model`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn login_me_and_401_round_trip_against_the_bench() {
    let bench = Bench::start();
    let api = client(&bench);

    // A client with no session is refused, with the SPA's `Unauthorized`.
    let err = api.me().await.expect_err("no session yet");
    assert_eq!(err, ApiError::Unauthorized);
    assert!(!api.has_session());

    let me = api.login("bench", "Bench1").await.expect("a real login");
    assert_eq!(me.user.name, "bench");
    assert!(me.user.admin, "the bench account is an admin");
    assert!(api.has_session(), "the cookie was stored");
    assert_eq!(
        api.owner().as_deref(),
        Some(me.user.id.as_str()),
        "the cache is keyed to this account"
    );

    let again = api.me().await.expect("/api/me with the cookie");
    assert_eq!(again.user.id, me.user.id);

    // Real payloads, through the typed endpoints and D1's types.
    let playlists = api.my_playlists().await.expect("/api/me/playlists");
    assert_eq!(playlists.playlists.len(), 2, "the seed made two playlists");
    let search = api.search("night").await.expect("/api/search");
    assert!(
        search.tracks.len() + search.albums.len() + search.artists.len() <= 50,
        "a bounded answer"
    );
    let ready = api
        .ready(&bench.first_track(), Quality::Opus128)
        .await
        .expect("/api/stream/<id>/ready");
    assert!(
        !ready.ready && ready.bytes.is_none(),
        "nothing is archived on the bench: {ready:?}"
    );
    api.get::<serde_json::Value>(Lane::User, "/version")
        .await
        .expect("/api/version");

    // A SECOND device with no jar is refused by the same server.
    let other = client(&bench);
    assert_eq!(
        other.me().await.expect_err("no cookie on this one"),
        ApiError::Unauthorized
    );

    api.logout().await.expect("logout");
    assert!(!api.has_session(), "logout dropped the cookie");
    assert_eq!(
        api.me().await.expect_err("logged out"),
        ApiError::Unauthorized
    );
}
