//! `Api::download_to` against the real server: a file the API streams (the
//! playlist list file), written to disk chunk by chunk, renamed only when
//! complete, cancellable, and failing fast on a path that does not exist.
//!
//! The bench is `tools/perf_api.py --serve` on a throwaway SQLite database, the
//! same one `tests/bench.rs` uses: what a mock cannot prove here is that a real
//! Flask answer streams through reqwest into a file without being held in
//! memory, and that the extension the server chose (`.csv`) lands on the name
//! the caller asked for.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use nz_api::{Api, ApiError, Lane, SessionJar};

/// The repository root (`desktop/crates/nz-api` → up three).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("the repository root")
}

/// The first interpreter that can import the server (`NZ_PYTHON` overrides).
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
    panic!("no Python interpreter can `import neezer`; set NZ_PYTHON to one that can");
}

struct Bench {
    child: Child,
    port: u16,
    playlist: String,
}

impl Bench {
    async fn start() -> Self {
        let script = repo_root().join("tools/perf_api.py");
        let mut child = Command::new(python())
            .arg(script)
            .args([
                "--tracks",
                "8",
                "--playlists",
                "1",
                "--favorites",
                "2",
                "--big-playlist",
                "2",
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
        while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
            match receiver.recv_timeout(left) {
                Ok(line) if line.starts_with("READY ") => {
                    port = line
                        .split_whitespace()
                        .nth(1)
                        .and_then(|port| port.parse::<u16>().ok());
                    break;
                }
                Ok(_) => continue,
                Err(_) => break,
            }
        }
        let port = port.expect("perf_api.py never printed READY");
        let mut bench = Self {
            child,
            port,
            playlist: String::new(),
        };
        // The one playlist the seed made, by its own answer.
        let client = bench.client().await;
        bench.playlist = client
            .my_playlists()
            .await
            .expect("/api/me/playlists")
            .playlists
            .first()
            .map(|playlist| playlist.id.as_str().to_owned())
            .expect("the seed made one playlist");
        bench
    }

    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    async fn client(&self) -> Api {
        let api = Api::builder(self.base())
            .session(Arc::new(SessionJar::in_memory()))
            .build()
            .expect("a client");
        // The bench's account: the session contract itself is `tests/bench.rs`'
        // business, this file only needs a cookie to ask for a file.
        api.login("bench", "Bench1").await.expect("a real login");
        api
    }
}

impl Drop for Bench {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn temporary(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "neezer-download-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// **A real list file streams to disk.**
///
/// Counted: the file exists under the name the caller chose, no `.part` is left
/// behind, the report's byte count is the file's own length, the progress
/// callback saw the whole file, and the body is the server's CSV (its first line
/// names the columns), not JSON and not an error page.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_list_file_is_written_to_disk_and_renamed_when_complete() {
    let bench = Bench::start().await;
    let api = bench.client().await;
    let dir = temporary("csv");
    let destination = dir.join("playlist.csv");
    let progress = Arc::new(AtomicU64::new(0));
    let seen = Arc::clone(&progress);
    let cancel = AtomicBool::new(false);

    let report = api
        .download_to(
            Lane::Bulk,
            &format!("/services/list/playlist/{}?fmt=csv", bench.playlist),
            &destination,
            move |written, total| {
                seen.store(written, Ordering::Relaxed);
                if let Some(total) = total {
                    assert!(written <= total, "progress never past the length");
                }
            },
            &cancel,
        )
        .await
        .expect("the list file");
    assert_eq!(report.path, destination);
    assert!(
        report
            .content_type
            .as_deref()
            .is_some_and(|kind| kind.starts_with("text/csv")),
        "the server's own type: {:?}",
        report.content_type
    );
    let bytes = std::fs::read(&destination).expect("the written file");
    assert_eq!(report.bytes, bytes.len() as u64, "the count is the file's");
    assert_eq!(progress.load(Ordering::Relaxed), report.bytes);
    assert!(!dir.join("playlist.csv.part").exists(), "no partial left");
    let text = String::from_utf8(bytes).expect("a UTF-8 list");
    assert!(
        text.to_lowercase().contains("title") || text.to_lowercase().contains("titre"),
        "a CSV header, not an error: {text}"
    );
    std::fs::remove_dir_all(dir).ok();
}

/// **A cancelled transfer leaves nothing behind.**
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancelled_download_removes_its_partial_file() {
    let bench = Bench::start().await;
    let api = bench.client().await;
    let dir = temporary("cancel");
    let destination = dir.join("playlist.csv");
    let cancel = AtomicBool::new(true);
    let error = api
        .download_to(
            Lane::Bulk,
            &format!("/services/list/playlist/{}?fmt=csv", bench.playlist),
            &destination,
            |_, _| {},
            &cancel,
        )
        .await
        .expect_err("cancelled before the first chunk");
    assert_eq!(error.cause_fr(), "téléchargement annulé");
    assert!(!destination.exists());
    assert!(!dir.join("playlist.csv.part").exists());
    std::fs::remove_dir_all(dir).ok();
}

/// **A path that does not exist fails fast, with the server's own cause.**
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_list_is_an_http_cause_not_a_half_file() {
    let bench = Bench::start().await;
    let api = bench.client().await;
    let dir = temporary("missing");
    let destination = dir.join("nope.csv");
    let cancel = AtomicBool::new(false);
    let error = api
        .download_to(
            Lane::Bulk,
            "/services/list/playlist/00000000-0000-0000-0000-000000000000?fmt=csv",
            &destination,
            |_, _| {},
            &cancel,
        )
        .await
        .expect_err("no such playlist");
    assert!(
        matches!(error, ApiError::Http { status: 404, .. }),
        "the server's own 404: {error:?}"
    );
    assert!(!destination.exists(), "nothing was written");
    std::fs::remove_dir_all(dir).ok();
}
