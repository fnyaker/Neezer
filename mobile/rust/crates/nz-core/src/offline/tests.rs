//! MB15's own bench: a real queue, a real store, a real HTTP server, and the
//! numbers the lot's acceptance asks for.
//!
//! The server is this file's own, deliberately: `nz-core`'s `core::testing` is
//! MB2's private module and `nz_offline::testing::TestServer` keeps its base URL
//! to itself, so a lot that needs a *playlist route* AND a counted body brings
//! the twenty lines it needs. It speaks the same wire the real server does
//! (`/api/version`, `/api/playlist/<id>`, `/api/stream/<id>`), throttles its link
//! through a token bucket (`Link`'s rule: a loopback bench that measures the
//! kernel proves nothing about rule 1), and counts two numbers per stream: how
//! many times it was ASKED (`hits`, the "never re-fetched" proof) and how many
//! bytes it SENT (`delivered`, the acceptance's bytes).
//!
//! Nothing here sleeps to "let it settle": a test waits on the state it needs
//! (a set's `done`, the meter's cap) through [`wait_until`], which yields to the
//! runtime instead of blocking it.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nz_api::{Lane, Quality};

use super::*;

// --- a local server whose link has a speed ---------------------------------

/// A token bucket shared by every connection: the "link". Without one, a
/// loopback test measures the kernel and rule 1's criterion is unmeasurable.
struct Bucket {
    rate: f64,
    state: Mutex<(Instant, f64)>,
}

impl Bucket {
    fn new(rate: Option<f64>) -> Arc<Bucket> {
        Arc::new(Bucket {
            rate: rate.unwrap_or(0.0),
            state: Mutex::new((Instant::now(), 16.0 * 1024.0)),
        })
    }

    fn take(&self, bytes: usize) {
        if self.rate <= 0.0 {
            return;
        }
        loop {
            let wait = {
                let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
                let now = Instant::now();
                let burst = 16.0 * 1024.0;
                state.1 =
                    (state.1 + now.duration_since(state.0).as_secs_f64() * self.rate).min(burst);
                state.0 = now;
                let need = bytes as f64;
                if state.1 >= need {
                    state.1 -= need;
                    return;
                }
                (need - state.1) / self.rate
            };
            std::thread::sleep(Duration::from_secs_f64(wait.max(0.001)));
        }
    }
}

/// `GET /api/playlist/<id>`'s body: the same shape the server sends, one row per
/// id (`nz-model::PlaylistPage`), with the fields a row draws.
fn playlist_json(id: &str, title: &str, tracks: &[&str]) -> String {
    let rows: Vec<String> = tracks
        .iter()
        .map(|track| {
            format!(
                r#"{{"deezer_id":"{track}","title":"Titre {track}","duration":200,"explicit":false,"artist":{{"deezer_id":"9","name":"Artiste"}},"display_artist":"Artiste","album":{{"deezer_id":"8","title":"Album","cover":"/api/cover/8"}}}}"#
            )
        })
        .collect();
    format!(
        r#"{{"playlist":{{"id":"{id}","title":"{title}"}},"tracks":[{}]}}"#,
        rows.join(",")
    )
}

/// The bench's server: one listener, a link with a speed, counted hits and bytes.
struct Fake {
    base: String,
    stop: Arc<AtomicBool>,
    hits: Arc<Mutex<HashMap<String, usize>>>,
    delivered: Arc<Mutex<HashMap<String, u64>>>,
}

impl Fake {
    /// `bodies` maps a stream id to its byte count; `playlist` is the JSON one
    /// `/api/playlist/<anything>` answers.
    fn start(rate: Option<f64>, bodies: HashMap<String, usize>, playlist: String) -> Fake {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("address"));
        let hits: Arc<Mutex<HashMap<String, usize>>> = Arc::default();
        let delivered: Arc<Mutex<HashMap<String, u64>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let bucket = Bucket::new(rate);
        let bodies = Arc::new(bodies);
        let playlist = Arc::new(playlist);
        let (hits_for, delivered_for, stop_for) =
            (Arc::clone(&hits), Arc::clone(&delivered), Arc::clone(&stop));
        std::thread::Builder::new()
            .name("nz-offline-test-server".to_owned())
            .spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else {
                        if stop_for.load(Ordering::Acquire) {
                            break;
                        }
                        continue;
                    };
                    let bodies = Arc::clone(&bodies);
                    let playlist = Arc::clone(&playlist);
                    let bucket = Arc::clone(&bucket);
                    let hits = Arc::clone(&hits_for);
                    let delivered = Arc::clone(&delivered_for);
                    std::thread::spawn(move || {
                        serve(stream, &bodies, &playlist, &bucket, &hits, &delivered)
                    });
                }
            })
            .expect("a server thread");
        Fake {
            base,
            stop,
            hits,
            delivered,
        }
    }

    fn hits(&self, id: &str) -> usize {
        self.hits
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(id)
            .copied()
            .unwrap_or(0)
    }

    fn delivered(&self, id: &str) -> u64 {
        self.delivered
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(id)
            .copied()
            .unwrap_or(0)
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

fn serve(
    mut stream: TcpStream,
    bodies: &HashMap<String, usize>,
    playlist: &str,
    bucket: &Bucket,
    hits: &Mutex<HashMap<String, usize>>,
    delivered: &Mutex<HashMap<String, u64>>,
) {
    let mut buffer = [0u8; 4096];
    let read = stream.read(&mut buffer).unwrap_or(0);
    let request = String::from_utf8_lossy(&buffer[..read]).to_string();
    let path = request
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .split('?')
        .next()
        .unwrap_or("/")
        .to_owned();
    let id = path.rsplit('/').next().unwrap_or_default().to_owned();

    if path == "/api/version" {
        let body = r#"{"version":"9.9.9"}"#;
        let _ = stream.write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        );
        return;
    }
    if path.starts_with("/api/playlist/") {
        let _ = stream.write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{playlist}",
                playlist.len()
            )
            .as_bytes(),
        );
        return;
    }
    let Some(&len) = bodies.get(&id) else {
        let _ = stream.write_all(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 2\r\nConnection: close\r\n\r\nno",
        );
        return;
    };
    *hits
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .entry(id.clone())
        .or_default() += 1;
    let _ = stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: audio/ogg\r\nAccept-Ranges: none\r\nConnection: close\r\n\r\n",
    );
    let mut chunk = vec![0u8; 16 * 1024];
    let mut sent = 0usize;
    while sent < len {
        let take = (len - sent).min(chunk.len());
        for (index, byte) in chunk[..take].iter_mut().enumerate() {
            *byte = ((sent + index) % 251) as u8;
        }
        bucket.take(take);
        if stream.write_all(&chunk[..take]).is_err() {
            return;
        }
        *delivered
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entry(id.clone())
            .or_default() += take as u64;
        sent += take;
    }
}

// --- the scaffolding --------------------------------------------------------

/// One test's two roots, removed on drop (a few kB of SQLite and the files it
/// downloaded; nothing big ever lands in `/tmp`'s tmpfs).
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "neezer-mb15-{tag}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("data")).expect("data dir");
        std::fs::create_dir_all(dir.join("cache")).expect("cache dir");
        Scratch(dir)
    }

    fn core(&self) -> Core {
        Core::new(crate::core::CoreConfig {
            data_dir: self.0.join("data"),
            cache_dir: self.0.join("cache"),
            app_version: "0.0.0-test".to_owned(),
            version_code: 2_000_001,
            sdk_int: 35,
            device_name: "test".to_owned(),
        })
        .expect("a core over a throwaway directory")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A core pointed at `server`, with a session owner (what `restore` leaves).
async fn core_for(server: &Fake, scratch: &Scratch) -> Arc<Core> {
    let core = Arc::new(scratch.core());
    core.set_server(&server.base)
        .await
        .expect("the probe answers /api/version");
    core.api()
        .expect("a client")
        .set_owner(Some("7".to_owned()));
    core
}

/// Wait on a state, yielding to the runtime — never a sleep to "let it settle"
/// (worker rule 6: a test waits on an event).
async fn wait_until(timeout: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if done() {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return done();
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn stream_bodies(ids: &[&str], size: usize) -> HashMap<String, usize> {
    ids.iter().map(|id| ((*id).to_owned(), size)).collect()
}

// --- the tests --------------------------------------------------------------

/// **The acceptance's first row.** A playlist's download queues every title it
/// holds with ONE read, publishes them through the worker, and the whole thing
/// costs the counted requests the PR prints: **1** read on the `offline` tag and
/// **1** body per title on `downloads`, 16 384 bytes for two 8 KiB files.
///
/// The list the settings section draws comes from the labels this module wrote
/// (`kv`, one blob for the set), so a restart reads them back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_playlist_download_queues_every_title_and_publishes_them() {
    let scratch = Scratch::new("playlist");
    let server = Fake::start(
        None,
        stream_bodies(&["101", "102"], 8 * 1024),
        playlist_json("42", "Nuit", &["101", "102"]),
    );
    let core = core_for(&server, &scratch).await;
    let api = core.api().expect("a client");
    api.clear_stats();
    let offline = Offline::start(Arc::clone(&core));

    let row = offline
        .add_set("playlist", "42", "OPUS_320", "", "")
        .await
        .expect("the set is queued");
    assert_eq!(row.context, "playlist:42");
    assert_eq!(row.total, 2, "both titles of the page are wanted");
    assert_eq!(row.title, "Nuit");

    assert!(
        wait_until(Duration::from_secs(30), || {
            offline
                .state()
                .sets
                .first()
                .is_some_and(|set| set.done == 2 && set.settled())
        })
        .await,
        "both titles are published: {:?}",
        offline.state().sets
    );

    let stats = api.stats();
    let read = stats
        .iter()
        .find(|row| row.tag == TAG_OFFLINE)
        .map(|row| row.requests)
        .unwrap_or(0);
    let body_stats = stats
        .iter()
        .find(|row| row.tag == TAG_DOWNLOAD)
        .map(|row| (row.requests, row.bytes_in))
        .unwrap_or((0, 0));
    assert_eq!(read, 1, "a 2-title playlist costs ONE read");
    assert_eq!(body_stats, (2, 16 * 1024), "two bodies, 8 KiB each");

    let state = offline.state();
    let set = &state.sets[0];
    assert!(set.complete(), "nothing failed: {:?}", set.failures);
    assert_eq!(state.storage.tracks, 2);
    assert_eq!(state.storage.downloads_bytes, 16 * 1024);

    let downloads = offline.downloads();
    assert_eq!(downloads.len(), 2, "the settings list has both rows");
    assert!(
        downloads.iter().all(|row| row.title.starts_with("Titre ")),
        "the titles come from the page, not from the id: {downloads:?}"
    );
    assert_eq!(
        offline.keep_downloaded(&["101".to_owned(), "404".to_owned()]),
        vec!["101".to_owned()],
        "the only-downloaded filter keeps exactly what is here"
    );

    // The labels survive a restart: the list is readable with no network.
    let labels = load_labels(core.cache());
    assert_eq!(
        labels.get("playlist:42").map(|set| set.tracks.len()),
        Some(2)
    );
    assert_eq!(
        labels.get("playlist:42").map(|set| set.title.clone()),
        Some("Nuit".to_owned())
    );
    offline.shutdown();
}

/// **The acceptance's "reprise après kill".** Four titles, a slow link, and the
/// worker stopped mid-set exactly as a process death stops it: the row stays
/// `active` in SQLite, and the next start finishes the set WITHOUT asking for a
/// title it already published (counted at the server: one hit, ever).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_kill_mid_download_resumes_without_refetching_finished_titles() {
    let scratch = Scratch::new("resume");
    let ids = ["201", "202", "203", "204"];
    // 64 KiB per title at 256 kB/s: the first two land, the third is in flight
    // when the worker is stopped.
    let server = Fake::start(
        Some(256.0 * 1024.0),
        stream_bodies(&ids, 64 * 1024),
        playlist_json("7", "Long", &ids),
    );
    let core = core_for(&server, &scratch).await;
    let first = Offline::start(Arc::clone(&core));
    first
        .add_set("playlist", "7", "OPUS_320", "", "")
        .await
        .expect("queued");
    assert!(
        wait_until(Duration::from_secs(30), || {
            first.state().sets.first().is_some_and(|set| set.done >= 2)
        })
        .await,
        "two titles are published before the kill: {:?}",
        first.state().sets
    );
    first.shutdown();
    let published: Vec<String> = first.downloaded_ids();
    assert!(!published.is_empty(), "something was published");
    let hits_before: HashMap<String, usize> = ids
        .iter()
        .map(|id| ((*id).to_owned(), server.hits(id)))
        .collect();

    // A new process, the same directories: the queue is the store's.
    let second_core = Arc::new(scratch.core());
    second_core
        .api()
        .expect("the stored server")
        .set_owner(Some("7".to_owned()));
    let second = Offline::start(second_core);
    // What `offlineAttach` does at process start: bind the client and let the
    // queue left by the killed process carry on.
    second.start_session().await;
    assert!(
        wait_until(Duration::from_secs(60), || {
            second
                .state()
                .sets
                .first()
                .is_some_and(|set| set.complete())
        })
        .await,
        "the new process finishes the set: {:?}",
        second.state().sets
    );
    for id in &published {
        assert_eq!(
            server.hits(id),
            *hits_before.get(id).unwrap_or(&1),
            "a published title is never fetched again ({id})"
        );
    }
    let finished: Vec<String> = second.downloaded_ids();
    assert!(
        published.iter().all(|id| finished.contains(id)),
        "what was published survives the kill: {published:?} vs {finished:?}"
    );
    second.shutdown();
}

/// **The acceptance's "mode avion".** With the radio down, queuing a set fails
/// at once with `Offline` (no socket, no timeout), while everything already on
/// the device is still listed, still filtered in, and still answerable as a
/// LOCAL file — which is what the engine's opener asks before it reaches the
/// network.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_no_link_a_downloaded_set_still_lists_and_plays_locally() {
    let scratch = Scratch::new("airplane");
    let server = Fake::start(
        None,
        stream_bodies(&["301"], 4 * 1024),
        playlist_json("5", "Avion", &["301"]),
    );
    let core = core_for(&server, &scratch).await;
    let offline = Offline::start(Arc::clone(&core));
    offline
        .add_set("playlist", "5", "", "", "")
        .await
        .expect("queued");
    assert!(
        wait_until(Duration::from_secs(30), || {
            offline.state().storage.tracks == 1
        })
        .await
    );

    core.set_network(crate::core::NetworkState::None);
    let refused = offline
        .add_set("playlist", "5", "", "", "")
        .await
        .expect_err("no link, no request");
    assert_eq!(refused, CoreError::Offline);

    assert_eq!(offline.state().storage.tracks, 1, "the list still reads");
    assert_eq!(offline.downloads().len(), 1);
    let (path, quality) = offline.local("301").expect("the file is on the device");
    assert!(path.is_file(), "the engine opens this path: {path:?}");
    assert_eq!(quality, Quality::Opus320);
    assert_eq!(offline.keep_downloaded(&["301".to_owned()]).len(), 1);
    offline.shutdown();
}

/// **Rule 1, the acceptance's own sentence**: while a track streams, a queued
/// set reads at most a quarter of what that stream reads.
///
/// Both figures are COUNTED BYTES over the same window: the reader's own chunks
/// (the untagged client, as the player is) and the server's `delivered` for the
/// set's titles. The 64 KiB allowance is `nz-api`'s own burst — the bucket's
/// capacity, documented rather than hidden: the cap bites after it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_long_download_yields_to_the_streaming_track() {
    let scratch = Scratch::new("yield");
    let ids: Vec<String> = (0..40).map(|n| format!("40{n:02}")).collect();
    let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let mut bodies = stream_bodies(&id_refs, 64 * 1024);
    // The stream the person is reading: one body the reader loops over.
    bodies.insert("999".to_owned(), 64 * 1024);
    let server = Fake::start(
        Some(800.0 * 1024.0),
        bodies,
        playlist_json("9", "Débit", &id_refs),
    );
    let core = core_for(&server, &scratch).await;
    let api = core.api().expect("a client");
    let offline = Offline::start(Arc::clone(&core));

    // The reader: the untagged client, the User lane, whole bodies read the way
    // the player reads its stream. Its bytes are counted as they arrive.
    let stop = Arc::new(AtomicBool::new(false));
    let read = Arc::new(AtomicU64::new(0));
    let reader = {
        let api = Arc::clone(&api);
        let stop = Arc::clone(&stop);
        let read = Arc::clone(&read);
        tokio::spawn(async move {
            while !stop.load(Ordering::Acquire) {
                match api.bytes(Lane::User, "/stream/999").await {
                    Ok(body) => {
                        read.fetch_add(body.len() as u64, Ordering::AcqRel);
                    }
                    Err(_) => break,
                }
            }
        })
    };

    offline
        .add_set("playlist", "9", "OPUS_320", "", "")
        .await
        .expect("queued");
    // An EVENT: the meter has bound a cap, so a download is really paced.
    assert!(
        wait_until(Duration::from_secs(20), || offline.bulk_cap().is_some()).await,
        "the link meter bound a cap (rate {} B/s)",
        offline.stream_rate()
    );
    let cap = offline.bulk_cap().expect("a cap");
    assert!(cap > 0);

    let read_before = read.load(Ordering::Acquire);
    let delivered_before: u64 = ids.iter().map(|id| server.delivered(id)).sum();
    // One window of the download's own work, ended on an EVENT: the queue
    // moving ten titles forward (or finishing).
    let target = offline
        .state()
        .storage
        .tracks
        .saturating_add(10)
        .min(ids.len() as u32);
    let _ = wait_until(Duration::from_secs(30), || {
        offline.state().storage.tracks >= target
    })
    .await;
    let read_during = read.load(Ordering::Acquire) - read_before;
    let delivered_during: u64 =
        ids.iter().map(|id| server.delivered(id)).sum::<u64>() - delivered_before;

    assert!(read_during > 0, "the stream really read");
    assert!(
        delivered_during <= read_during / 4 + 64 * 1024,
        "the download read {delivered_during} B against the stream's {read_during} B \
         (a quarter plus nz-api's 64 KiB burst), cap {cap} B/s at {} B/s measured",
        offline.stream_rate()
    );
    stop.store(true, Ordering::Release);
    let _ = reader.await;
    offline.shutdown();
}
