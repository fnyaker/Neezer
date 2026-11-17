//! **A restart in the middle of a file, against the real server.**
//!
//! `tools/perf_api.py --serve` archives one small WAV as its whole library, so
//! `/api/stream` serves a real body from disk. The worker fetches the set
//! through a slow `Lane::Bulk` (its own floor, 64 kB/s), is killed while a
//! second file is being written, and a NEW worker over the same store finishes
//! the set.
//!
//! What is counted is BYTES, per track, in the fetcher — never a file's size on
//! disk: a published file looks identical whether it was fetched once or twice,
//! which is exactly the mistake this test exists to catch. The assertions are
//! the acceptance's:
//!
//! - the stop really landed mid-file (the `.part` holds bytes, fewer than the
//!   body's length);
//! - a track that was already published costs **0** further requests and **0**
//!   further bytes;
//! - the interrupted track is fetched from its first byte again (its `.part` is
//!   not trusted), and nothing else is;
//! - the total fetched is the library plus the bytes the interruption wasted.
//!
//! The fetcher is the crate's `Downloader` seam with a counter around the
//! default `ApiDownloader`, which is also the seam a platform with another HTTP
//! stack supplies.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nz_api::{Api, Quality, SessionJar, TrackId};
use nz_cache::Cache;
use nz_offline::{ApiDownloader, Choice, Downloader, Offline, Outcome, Progress, Stop};

/// The body every archived track serves, in bytes: 3 s of silence at 25 kHz,
/// mono, 16-bit — about 4.7 s on the lane's 64 kB/s floor, long enough to stop
/// inside one.
const WAV_BYTES: usize = 300_044;
/// 25 % of this is the lane's floor (64 kB/s), so the first body is slow enough
/// to be caught mid-file.
const STREAM_BYTES_PER_S: u64 = 1;

// --- the bench --------------------------------------------------------------

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
}

impl Bench {
    /// Six tracks, all archived (the bench archives the first twelve), inside
    /// the one playlist.
    fn start(audio: &Path) -> Bench {
        let script = repo_root().join("tools/perf_api.py");
        let mut child = Command::new(python())
            .arg(script)
            .args([
                "--tracks",
                "6",
                "--playlists",
                "1",
                "--big-playlist",
                "6",
                "--favorites",
                "1",
                "--audio",
            ])
            .arg(audio)
            .args(["--serve", "--port", "0"])
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
        let deadline = Instant::now() + Duration::from_secs(180);
        let mut port = None;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
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
        Bench { child, port }
    }

    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

impl Drop for Bench {
    fn drop(&mut self) {
        stop(&mut self.child);
    }
}

/// Stop the bench the way it expects: SIGTERM first, SIGKILL only as a last
/// resort (its library is a throwaway directory it removes when asked to stop).
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
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "neezer-nzoffline-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// A real WAV of `WAV_BYTES`: 25 kHz, mono, 16-bit PCM, digital silence. The
/// server probes it like any archived file, and every track shares it, so a
/// track's length is a known number.
fn write_wav(path: &Path) {
    let data = WAV_BYTES - 44;
    let mut bytes = Vec::with_capacity(WAV_BYTES);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&((data + 36) as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&25_000u32.to_le_bytes());
    bytes.extend_from_slice(&50_000u32.to_le_bytes()); // byte rate
    bytes.extend_from_slice(&2u16.to_le_bytes()); // block align
    bytes.extend_from_slice(&16u16.to_le_bytes()); // bits
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data as u32).to_le_bytes());
    bytes.resize(WAV_BYTES, 0);
    std::fs::write(path, bytes).expect("the WAV");
}

// --- the counting fetcher ---------------------------------------------------

/// Counts what actually came over the wire, per track, around the default
/// fetcher: requests, announced length and bytes written.
#[derive(Default)]
struct Counts {
    requests: Mutex<HashMap<String, usize>>,
    fetched: Mutex<HashMap<String, u64>>,
    totals: Mutex<HashMap<String, u64>>,
}

impl Counts {
    fn requests(&self, id: &str) -> usize {
        self.requests
            .lock()
            .expect("counts")
            .get(id)
            .copied()
            .unwrap_or(0)
    }

    fn fetched(&self, id: &str) -> u64 {
        self.fetched
            .lock()
            .expect("counts")
            .get(id)
            .copied()
            .unwrap_or(0)
    }

    fn total(&self, id: &str) -> Option<u64> {
        self.totals.lock().expect("counts").get(id).copied()
    }
}

struct Counting {
    inner: ApiDownloader,
    counts: Arc<Counts>,
}

impl Counting {
    fn new(inner: ApiDownloader, counts: Arc<Counts>) -> Counting {
        Counting { inner, counts }
    }
}

impl Downloader for Counting {
    fn fetch(
        &self,
        track: &TrackId,
        quality: Quality,
        part: &Path,
        stop: &Stop,
        progress: &mut dyn Progress,
    ) -> Result<u64, Outcome> {
        let id = track.as_str().to_owned();
        *self
            .counts
            .requests
            .lock()
            .expect("counts")
            .entry(id.clone())
            .or_default() += 1;
        let mut counted = Counted {
            inner: progress,
            counts: Arc::clone(&self.counts),
            track: id.clone(),
        };
        let outcome = self.inner.fetch(track, quality, part, stop, &mut counted);
        let bytes = match &outcome {
            Ok(bytes) => *bytes,
            // An interrupted body keeps its `.part`: what it holds is what came
            // over the wire before the stop.
            Err(Outcome::Interrupted) => {
                std::fs::metadata(part).map(|file| file.len()).unwrap_or(0)
            }
            Err(_) => 0,
        };
        *self
            .counts
            .fetched
            .lock()
            .expect("counts")
            .entry(id)
            .or_default() += bytes;
        outcome
    }
}

struct Counted<'a> {
    inner: &'a mut dyn Progress,
    counts: Arc<Counts>,
    track: String,
}

impl Progress for Counted<'_> {
    fn total(&mut self, total: Option<u64>) {
        if let Some(total) = total {
            self.counts
                .totals
                .lock()
                .expect("counts")
                .insert(self.track.clone(), total);
        }
        self.inner.total(total);
    }

    fn written(&mut self, bytes: u64) {
        self.inner.written(bytes);
    }
}

// --- the test ---------------------------------------------------------------

/// Poll `done` until it says so or `timeout`: a test waits on the queue's own
/// state, never on a sleep.
fn wait_for(timeout: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    done()
}

fn net_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("nz-test-net")
        .enable_all()
        .build()
        .expect("a runtime")
}

#[test]
fn a_restart_mid_file_resumes_without_refetching_a_finished_track() {
    let dir = scratch("resume");
    let audio = dir.join("bench.wav");
    write_wav(&audio);
    let bench = Bench::start(&audio);
    let store =
        Cache::open_with_cache_dir(dir.join("data"), dir.join("cache")).expect("the local store");
    let runtime = net_runtime();

    // The session, and the six tracks the playlist holds (all archived).
    let (api, ids) = runtime.block_on(async {
        let api = Api::builder(bench.base())
            .cache(Arc::new(store.clone()))
            .session(Arc::new(SessionJar::at(dir.join("session.json"))))
            .build()
            .expect("a client");
        api.login("bench", "Bench1").await.expect("a real login");
        let playlists = api.my_playlists().await.expect("the bench's playlists");
        let playlist = playlists
            .playlists
            .first()
            .expect("the seed made one playlist");
        let page = api.playlist(&playlist.id).await.expect("its tracks");
        let ids: Vec<TrackId> = page.tracks.iter().map(|t| t.deezer_id.clone()).collect();
        (Arc::new(api), ids)
    });
    assert!(
        ids.len() >= 3,
        "the bench's playlist has tracks to stop between: {}",
        ids.len()
    );
    let owner = api.owner().expect("a session owner").to_owned();
    println!(
        "banc : {} titres, {} octets par titre, propriétaire {owner}",
        ids.len(),
        WAV_BYTES
    );

    // The lane's floor: 25 % of 1 B/s is still 64 kB/s, so one body lasts
    // seconds and the stop lands inside it. Nothing else throttles a download
    // (rule 1: the lane IS the throttle).
    api.set_bulk_cap_from_stream(Some(STREAM_BYTES_PER_S));

    // --- run 1: stopped in the middle of a file -----------------------------
    let counts = Arc::new(Counts::default());
    let downloader = Arc::new(Counting::new(
        ApiDownloader::new(Arc::clone(&api), runtime.handle().clone()),
        Arc::clone(&counts),
    ));
    let first = Offline::start(store.clone());
    first.configure_with(Arc::clone(&api), downloader);
    let choices: Vec<Choice> = ids
        .iter()
        .map(|id| {
            Choice::new(id.as_str(), format!("Titre {}", id.as_str())).with_quality(Quality::Flac)
        })
        .collect();
    first.add_set("playlist", "banc", "Le banc", choices);

    assert!(
        wait_for(Duration::from_secs(120), || {
            let snapshot = first.snapshot();
            snapshot.progress.done >= 1
                && snapshot
                    .running
                    .as_ref()
                    .is_some_and(|running| running.bytes > 0)
        }),
        "a first file is published and a second is being written: {:?} / {:?}",
        first.snapshot().progress,
        first.snapshot().running
    );
    let interrupted = first
        .snapshot()
        .running
        .as_ref()
        .expect("a body in flight")
        .track
        .clone();

    // The app closing: the worker is joined, the `.part` stays, the row stays
    // `active`.
    first.shutdown();
    drop(first);
    let published: Vec<TrackId> = ids
        .iter()
        .filter(|id| {
            store
                .downloads()
                .local(&owner, id)
                .expect("the store")
                .is_some()
        })
        .cloned()
        .collect();
    assert!(
        !published.is_empty() && !published.iter().any(|id| id.as_str() == interrupted),
        "the stop landed after a file was published, not after the one in flight: {published:?}"
    );
    let pending = store.downloads().pending(&owner, 100).expect("the queue");
    assert!(
        pending
            .iter()
            .any(|job| job.track.as_str() == interrupted && job.state.as_str() == "active"),
        "the interrupted row is `active`, which is what the next start resumes"
    );
    let part_bytes = part_length(&store);
    let interrupted_total = counts.total(&interrupted).expect("its announced length");
    println!(
        "arrêt en plein fichier : {part_bytes} octets dans le .part de « {interrupted} », \
         {} annoncés ; {} titre(s) déjà publiés",
        interrupted_total,
        published.len()
    );
    assert!(
        part_bytes > 0 && part_bytes < interrupted_total,
        "the stop really landed inside the body: {part_bytes} of {interrupted_total} bytes"
    );

    // --- run 2: a new worker over the same store ----------------------------
    // The rest is not the subject of this test: lift the cap.
    api.set_bulk_cap_from_stream(Some(50_000_000));
    let fetched_before: HashMap<String, u64> = published
        .iter()
        .map(|id| (id.as_str().to_owned(), counts.fetched(id.as_str())))
        .collect();
    let asked_before: HashMap<String, usize> = published
        .iter()
        .map(|id| (id.as_str().to_owned(), counts.requests(id.as_str())))
        .collect();

    let second_downloader = Arc::new(Counting::new(
        ApiDownloader::new(Arc::clone(&api), runtime.handle().clone()),
        Arc::clone(&counts),
    ));
    let second = Offline::start(store.clone());
    second.configure_with(Arc::clone(&api), second_downloader);
    assert!(
        wait_for(Duration::from_secs(120), || {
            second.snapshot().progress.done == ids.len()
        }),
        "the restarted worker finished the set: {:?}",
        second.snapshot().progress
    );
    let snapshot = second.snapshot();
    assert_eq!(
        snapshot.progress.failed, 0,
        "an interruption is not a failure"
    );

    // What was already published is never asked again: 0 requests, 0 bytes.
    for id in &published {
        let key = id.as_str();
        assert_eq!(
            counts.requests(key),
            asked_before[key],
            "{key} was published before the stop and must not be fetched again"
        );
        assert_eq!(
            counts.fetched(key),
            fetched_before[key],
            "{key} cost no further byte"
        );
    }

    // The interrupted track came down WHOLE from its first byte (the `.part` is
    // not trusted), and it is the only waste: run 1's bytes are the part's, run
    // 2 brought the whole body again.
    assert_eq!(
        counts.fetched(&interrupted) - part_bytes,
        interrupted_total,
        "the interrupted track was fetched again from its first byte"
    );
    let library: u64 = ids
        .iter()
        .map(|id| {
            counts
                .total(id.as_str())
                .expect("every track announced its length")
        })
        .sum();
    let total_fetched: u64 = ids.iter().map(|id| counts.fetched(id.as_str())).sum();
    println!(
        "reprise : {total_fetched} octets récupérés pour {library} publiés \
         ({part_bytes} perdus par l'interruption), {} titres déjà publiés jamais redemandés",
        published.len()
    );
    assert_eq!(
        total_fetched,
        library + part_bytes,
        "the only bytes fetched twice are the interrupted part"
    );
    assert_eq!(snapshot.progress.bytes, library as i64);

    second.shutdown();
    drop(second);
    // The worker borrows this runtime: it is joined before the runtime goes.
    drop(runtime);
    std::fs::remove_dir_all(&dir).ok();
}

/// The bytes the interruption left in the store's one `.part`.
fn part_length(store: &Cache) -> u64 {
    std::fs::read_dir(store.offline_dir())
        .expect("the offline directory")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "part"))
        .map(|entry| entry.metadata().map(|meta| meta.len()).unwrap_or(0))
        .max()
        .unwrap_or(0)
}
