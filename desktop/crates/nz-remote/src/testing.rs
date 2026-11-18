//! The remote-control bench: the reference server, a scratch store and a
//! runtime, for THIS crate's tests.
//!
//! The server is `webapp/test/remote/server.py`, the fixture the SPA's own
//! `webapp/test/remote/run.mjs` drives: a throwaway database, an owner
//! ("owner"/"Owner1") whose player is lent, a friend ("friend"/"Friend1") and
//! four short WAV tracks — WAV so the bench needs no ffmpeg. Proving the crate
//! against the same server the reference client uses is the whole point: the
//! protocol is the server's, not this crate's idea of it.
//!
//! `cfg(test)` on purpose: the desktop app's remote tests are this crate's now,
//! and no app bench drives a remote link (the engine benches stay in the app).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nz_api::{Api, SessionJar};
use nz_cache::Cache;
use nz_player::{Command, Effect, Entry};
use serde_json::Value;

/// The repository root (`desktop/crates/nz-remote` → up three).
pub(crate) fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("the repository root")
}

/// The first interpreter that can import the server and its fixture.
pub(crate) fn python() -> String {
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
        let usable = std::process::Command::new(candidate)
            .args(["-c", "import peewee, neezer, mediafile"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if usable {
            return candidate.clone();
        }
    }
    panic!(
        "no Python interpreter can `import neezer` (tried: {}). Install the server \
         (`pip install -e .`), or point NZ_PYTHON at one.",
        candidates.join(", ")
    );
}

/// A scratch directory under the system temp dir, removed on drop. A store
/// opened here writes nothing into the person's real cache directory.
pub(crate) struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    pub(crate) fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("nz-remote-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        Self { dir }
    }

    /// A store of its own, with its covers and audio beside it, never in the
    /// person's `$XDG_CACHE_HOME`.
    pub(crate) fn cache(&self) -> Cache {
        Cache::open_with_cache_dir(self.dir.join("data"), self.dir.join("cache"))
            .expect("a scratch store")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A current-thread runtime: the tests drive every step by hand, so nothing
/// needs a worker (and no thread outlives the runtime).
pub(crate) fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
}

/// The remote-control bench's own server, as a child process.
pub(crate) struct Bench {
    child: std::process::Child,
    pub(crate) port: u16,
    /// The four tracks, `{id, title}`.
    pub(crate) tracks: Value,
}

impl Bench {
    pub(crate) fn start() -> Self {
        use std::io::{BufRead, BufReader};
        let script = repo_root().join("webapp/test/remote/server.py");
        let mut child = std::process::Command::new(python())
            .arg(script)
            .current_dir(repo_root())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .stdin(std::process::Stdio::piped())
            .spawn()
            .expect("start webapp/test/remote/server.py");
        let stdout = child.stdout.take().expect("its stdout");
        let (sender, receiver) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let _ = sender.send(line);
            }
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(180);
        let mut port = None;
        let mut tracks = Value::Null;
        while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
            match receiver.recv_timeout(left) {
                Ok(line) if line.starts_with("READY ") => {
                    let mut parts = line.splitn(3, ' ');
                    parts.next();
                    port = parts.next().and_then(|port| port.parse::<u16>().ok());
                    tracks = parts
                        .next()
                        .and_then(|raw| serde_json::from_str(raw).ok())
                        .unwrap_or(Value::Null);
                    break;
                }
                Ok(_) => continue,
                Err(_) => break,
            }
        }
        let port = port.expect("the bench never printed READY");
        Self {
            child,
            port,
            tracks,
        }
    }

    pub(crate) fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// One client with its own cookie jar: `get`, `send`, a session.
    pub(crate) fn client(&self) -> Api {
        Api::builder(self.base())
            .session(Arc::new(SessionJar::in_memory()))
            .build()
            .expect("a client")
    }

    /// A queue of the bench's own tracks, as this player would hold them.
    pub(crate) fn queue(&self) -> Vec<Entry> {
        self.tracks
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(|track| {
                let id = track
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let mut entry = Entry::new(id.clone());
                entry.deezer_id = Some(id);
                entry.title = track
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                entry.duration = 40;
                entry
            })
            .collect()
    }

    /// A queue already playing, returned with the effects its first apply
    /// produced — the caller's own state machine, the same one a tap drives.
    pub(crate) fn playing_queue(&self) -> (nz_player::PlayerState, Vec<Effect>) {
        let mut player = nz_player::PlayerState::new();
        let effects = player.apply(Command::play_queue(self.queue(), 0, None), 0);
        (player, effects)
    }
}

impl Drop for Bench {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
