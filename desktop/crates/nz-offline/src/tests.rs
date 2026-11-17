//! The rules of the queue, proven: a set fetched track by track, a cause kept
//! and shown, a dead link that is not a verdict, a restart mid-file that
//! resumes, and rule 1 measured on a shared link.
//!
//! Moved here with the worker (SC4). The two benches that play real audio
//! through the engine's opener stayed in the desktop app with that adapter, and
//! the `#[gpui::test]` one too: this crate links no GPUI and no output device.

use std::sync::Arc;
use std::time::{Duration, Instant};

use nz_api::{Api, Lane, Quality, TrackId};
use nz_cache::{DownloadState, DownloadTrack};

use super::*;
use crate::testing::*;

/// A chosen playlist is downloaded track by track, and the files land in
/// the person's own directory (never the LRU's).
#[test]
fn a_chosen_playlist_is_fetched_track_by_track() {
    let scratch = Scratch::new("fetch");
    let store = scratch.store();
    let ids = ["d1", "d2", "d3", "d4", "d5"];
    let server = TestServer::start(Link::new(None), body_map(&ids, 4096));
    let runtime = runtime();
    let api = api_for(&server, &store, &scratch);
    let service = Offline::start(store.clone());
    service.configure(api, runtime.handle().clone());

    service.choose("playlist", "p1", "Soirée", choices(&ids));
    assert!(
        wait_for(Duration::from_secs(20), || {
            service.snapshot().progress.done == ids.len()
        }),
        "the five tracks were fetched: {:?}",
        service.snapshot().progress
    );

    let snapshot = service.snapshot();
    assert_eq!(snapshot.sets.len(), 1);
    assert_eq!(snapshot.sets[0].done, ids.len());
    assert_eq!(snapshot.sets[0].title, "Soirée");
    assert_eq!(snapshot.progress.bytes, 5 * 4096);
    assert!(snapshot.failures.is_empty());
    for id in ids {
        let path = service.local(id).map(|(path, _)| path);
        assert!(path.is_some_and(|path| path.is_file()), "{id} is on disk");
    }
    // The file is under the offline directory, which no cache cap reaches.
    let offline = store.offline_dir();
    assert!(
        service
            .local("d1")
            .is_some_and(|(path, _)| path.starts_with(&offline)),
        "downloads live in their own directory"
    );
}

/// A server verdict is the track's cause, kept with the row and printed
/// (rule 2) — and the worker moves on to the next track.
#[test]
fn a_failure_keeps_its_cause_and_the_queue_moves_on() {
    let scratch = Scratch::new("failure");
    let store = scratch.store();
    let server = TestServer::start(Link::new(None), body_map(&["ok"], 2048));
    server.script(
        "gone",
        Script {
            status: 404,
            headers: Vec::new(),
            body: r#"{"error":"no such track"}"#.to_owned(),
        },
    );
    let runtime = runtime();
    let api = api_for(&server, &store, &scratch);
    let service = Offline::start(store.clone());
    service.configure(api, runtime.handle().clone());

    service.choose("album", "a1", "Album", choices(&["gone", "ok"]));
    assert!(wait_for(Duration::from_secs(20), || {
        let snapshot = service.snapshot();
        snapshot.progress.failed == 1 && snapshot.progress.done == 1
    }));

    let snapshot = service.snapshot();
    assert_eq!(snapshot.failures.len(), 1);
    assert_eq!(snapshot.failures[0].track, "gone");
    assert!(
        snapshot.failures[0].cause.contains("404"),
        "the cause is the server's own: {}",
        snapshot.failures[0].cause
    );
    assert_eq!(snapshot.failures[0].title, "Titre gone");

    // « Réessayer » really re-asks the server (and the same cause comes
    // back, which is the point: the cause is the server's, not a count).
    let asked = server.hits("gone");
    service.retry_failures();
    assert!(
        wait_for(Duration::from_secs(10), || {
            let snapshot = service.snapshot();
            snapshot.progress.failed == 1 && server.hits("gone") > asked
        }),
        "« Réessayer » re-asked the server and kept the cause"
    );
    assert_eq!(service.snapshot().failures.len(), 1);
    assert!(service.snapshot().failures[0].cause.contains("404"));
}

/// A server that says "later" is not a verdict about the track: the row
/// stays pending and the download completes once the answer comes.
#[test]
fn a_queued_track_is_retried_and_never_marked_failed() {
    let scratch = Scratch::new("queued");
    let store = scratch.store();
    let server = TestServer::start(Link::new(None), body_map(&["cold"], 3072));
    for _ in 0..2 {
        server.script(
            "cold",
            Script {
                status: 503,
                headers: vec![
                    ("Retry-After".to_owned(), "1".to_owned()),
                    ("Content-Type".to_owned(), "application/json".to_owned()),
                ],
                body: r#"{"queued":true}"#.to_owned(),
            },
        );
    }
    let runtime = runtime();
    let api = api_for(&server, &store, &scratch);
    let service = Offline::start(store.clone());
    service.configure(api, runtime.handle().clone());

    service.choose("playlist", "p1", "Froid", choices(&["cold"]));
    assert!(
        wait_for(Duration::from_secs(30), || {
            service.snapshot().progress.done == 1
        }),
        "the queued track was fetched: hits={} {:?}",
        server.hits("cold"),
        service.snapshot().progress
    );
    let snapshot = service.snapshot();
    assert_eq!(snapshot.progress.failed, 0, "a 503 is never a failure");
    assert!(
        server.hits("cold") >= 3,
        "one refusal, one repeat, then the body: {} arrivals",
        server.hits("cold")
    );
}

/// A dead link keeps the work PENDING and the state says so: nothing is
/// lost, nothing is marked failed, and the worker waits for the network.
#[test]
fn a_dead_link_keeps_the_work_queued() {
    let scratch = Scratch::new("dead");
    let store = scratch.store();
    let server = TestServer::start(Link::new(None), body_map(&["d1"], 1024));
    let runtime = runtime();
    let api = api_for(&server, &store, &scratch);
    let service = Offline::start(store.clone());
    service.configure(api.clone(), runtime.handle().clone());

    service.choose("playlist", "p1", "Hors ligne", choices(&["d1"]));
    assert!(wait_for(Duration::from_secs(20), || {
        service.snapshot().progress.done == 1
    }));

    // The link goes: the next job cannot start.
    api.set_online(false);
    service.choose("playlist", "p2", "Encore", choices(&["d2"]));
    assert!(
        wait_for(Duration::from_secs(5), || {
            service.snapshot().state == WorkerState::Offline
        }),
        "the state names the link: {:?}",
        service.snapshot().state
    );
    let snapshot = service.snapshot();
    assert_eq!(snapshot.progress.failed, 0, "a dead link is not a verdict");
    assert_eq!(snapshot.progress.pending, 1);
}

/// A pause stops the queue between two tracks, and resuming picks it up.
#[test]
fn a_pause_stops_the_queue_and_a_resume_continues_it() {
    let scratch = Scratch::new("pause");
    let store = scratch.store();
    let ids: Vec<String> = (0..60).map(|index| format!("t{index}")).collect();
    let borrowed: Vec<&str> = ids.iter().map(String::as_str).collect();
    let server = TestServer::start(Link::new(Some(200_000.0)), body_map(&borrowed, 4096));
    let runtime = runtime();
    let api = api_for(&server, &store, &scratch);
    let service = Offline::start(store.clone());
    service.configure(api, runtime.handle().clone());

    service.choose("playlist", "p1", "Long", choices(&borrowed));
    assert!(wait_for(Duration::from_secs(10), || {
        service.snapshot().progress.done >= 1
    }));
    service.pause();
    assert!(wait_for(Duration::from_secs(10), || {
        matches!(
            service.snapshot().state,
            WorkerState::Paused | WorkerState::Pausing
        )
    }));
    std::thread::sleep(Duration::from_millis(600));
    let stopped = service.snapshot().progress.done;
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(
        service.snapshot().progress.done,
        stopped,
        "a paused worker fetches nothing"
    );
    assert!(stopped < borrowed.len(), "the pause came before the end");

    service.resume();
    assert!(wait_for(Duration::from_secs(60), || {
        service.snapshot().progress.done == borrowed.len()
    }));
}

/// Taking a set back deletes its files, and only its files.
#[test]
fn a_removed_set_takes_its_files_back() {
    let scratch = Scratch::new("remove");
    let store = scratch.store();
    let server = TestServer::start(Link::new(None), body_map(&["keep", "drop"], 2048));
    let runtime = runtime();
    let api = api_for(&server, &store, &scratch);
    let service = Offline::start(store.clone());
    service.configure(api, runtime.handle().clone());

    service.choose("playlist", "p1", "Gardée", choices(&["keep"]));
    service.choose("playlist", "p2", "Retirée", choices(&["drop"]));
    assert!(wait_for(Duration::from_secs(20), || {
        service.snapshot().progress.done == 2
    }));
    let dropped = service.local("drop").map(|(path, _)| path).expect("a file");

    service.drop_set("playlist", "p2");
    assert!(wait_for(Duration::from_secs(10), || {
        service.snapshot().sets.len() == 1
    }));
    assert!(!dropped.exists(), "the removed set's file is gone");
    assert!(service.local("keep").is_some(), "the other set survives");
    assert_eq!(service.snapshot().progress.done, 1);
}

/// A job interrupted by a restart is `active` in the store: the next start of
/// the worker picks it up instead of starting the set over (rule 2).
#[test]
fn an_interrupted_download_resumes_after_a_restart() {
    let scratch = Scratch::new("restart");
    let store = scratch.store();
    let server = TestServer::start(Link::new(None), body_map(&["done", "cut"], 4096));
    let runtime = runtime();
    let api = api_for(&server, &store, &scratch);

    // What the previous process left: one published track, one row the crash
    // caught mid-write.
    let downloads = store.downloads();
    downloads
        .add_set(
            "42",
            "playlist",
            "p1",
            "Voyage",
            &[
                DownloadTrack {
                    track: TrackId::new("done"),
                    quality: Quality::Opus320,
                },
                DownloadTrack {
                    track: TrackId::new("cut"),
                    quality: Quality::Opus320,
                },
            ],
        )
        .expect("the set");
    let partial = downloads
        .reserve("42", &TrackId::new("done"), Quality::Opus320)
        .expect("reserve");
    std::fs::write(&partial, vec![1u8; 4096]).expect("write");
    downloads
        .complete("42", &TrackId::new("done"), 4096)
        .expect("complete");
    downloads
        .mark("42", &TrackId::new("cut"), DownloadState::Active, None)
        .expect("mark");

    let service = Offline::start(store.clone());
    service.configure(api, runtime.handle().clone());
    assert!(
        wait_for(Duration::from_secs(20), || {
            service.snapshot().progress.done == 2
        }),
        "the interrupted row was fetched: {:?}",
        service.snapshot().progress
    );
    assert_eq!(
        server.hits("done"),
        0,
        "what was published is never fetched again"
    );
    assert!(server.hits("cut") >= 1, "the interrupted track was resumed");
    assert_eq!(service.snapshot().progress.failed, 0);
}

/// The play cache's cap holds: one sweep brings it back under, and it never
/// reaches a download.
#[test]
fn the_play_cache_cap_is_enforced_and_never_touches_a_download() {
    let scratch = Scratch::new("cap");
    let store = scratch.store();
    let server = TestServer::start(Link::new(None), body_map(&["kept"], 8192));
    let runtime = runtime();
    let api = api_for(&server, &store, &scratch);
    let service = Offline::start(store.clone());
    service.configure(api, runtime.handle().clone());

    service.choose("playlist", "p1", "Hors ligne", choices(&["kept"]));
    assert!(wait_for(Duration::from_secs(20), || {
        service.snapshot().progress.done == 1
    }));
    let download = service.local("kept").map(|(path, _)| path).expect("a file");

    // Three play-cache entries of 256 kB against a 512 kB cap.
    for index in 0..3 {
        let track = TrackId::new(format!("cached-{index}"));
        let partial = store
            .play_cache()
            .reserve(&track, Quality::Opus320)
            .expect("reserve");
        std::fs::write(&partial, vec![7u8; 256 * 1024]).expect("write");
        store
            .play_cache()
            .complete(&track, Quality::Opus320, 256 * 1024)
            .expect("complete");
    }
    let before = store.play_cache().total_bytes().expect("bytes");
    assert_eq!(before, 3 * 256 * 1024);

    service.set_play_cache_cap(512 * 1024);
    assert!(
        wait_for(Duration::from_secs(10), || {
            store.play_cache().total_bytes().unwrap_or(before) <= 512 * 1024
        }),
        "the sweep brought the cache under its cap"
    );
    assert!(
        download.is_file(),
        "a cap on the play cache never deletes a download"
    );
    assert!(service.snapshot().play_cache_bytes <= 512 * 1024);
}

/// The offline-only filter keeps what is on the device, by the entry's own
/// reference (what the queue builders call when the link is down).
#[test]
fn the_offline_only_filter_keeps_the_downloaded_tracks() {
    let scratch = Scratch::new("filter");
    let store = scratch.store();
    let server = TestServer::start(Link::new(None), body_map(&["here"], 2048));
    let runtime = runtime();
    let api = api_for(&server, &store, &scratch);
    let service = Offline::start(store.clone());
    service.configure(api, runtime.handle().clone());
    service.choose("playlist", "p1", "Sur l'appareil", choices(&["here"]));
    assert!(wait_for(Duration::from_secs(20), || {
        service.snapshot().progress.done == 1
    }));

    let entries = vec!["here".to_owned(), "elsewhere".to_owned()];
    let kept = keep_downloaded_with(Some(&service), entries.clone(), |entry| Some(entry.clone()));
    assert_eq!(kept, vec!["here".to_owned()], "seul le titre local reste");
    // No service: the filter is transparent (nothing is known to be local).
    let all = keep_downloaded_with(None, entries.clone(), |entry| Some(entry.clone()));
    assert_eq!(all.len(), 2);
}

/// **A restart mid-download.** The worker is stopped while a track is being
/// written (the app quitting), and a NEW worker over the same store finishes
/// the set: the interrupted row is `active`, never published, and the file it
/// was writing is fetched again rather than trusted.
#[test]
fn a_restart_mid_download_resumes_the_set() {
    in_its_own_process("tests::the_restart_bench_in_its_own_process");
}

#[test]
#[ignore = "run by its parent, a process of its own (RSS neighbours)"]
fn the_restart_bench_in_its_own_process() {
    let scratch = Scratch::new("mid-restart");
    let store = scratch.store();
    let ids: Vec<String> = (0..40).map(|index| format!("r{index}")).collect();
    let borrowed: Vec<&str> = ids.iter().map(String::as_str).collect();
    // A link slow enough to catch the worker in the middle of a body.
    let server = TestServer::start(Link::new(Some(120_000.0)), body_map(&borrowed, 64 * 1024));
    let runtime = runtime();
    let api = api_for(&server, &store, &scratch);

    let first = Offline::start(store.clone());
    first.configure(api.clone(), runtime.handle().clone());
    first.choose("playlist", "p1", "Voyage", choices(&borrowed));
    assert!(
        wait_for(Duration::from_secs(20), || {
            let progress = first.snapshot().progress;
            progress.done >= 1 && progress.pending > 1
        }),
        "the set is being downloaded: {:?}",
        first.snapshot().progress
    );
    first.shutdown();
    let stopped_at = first.snapshot().progress.done;
    // A stopped worker publishes nothing more.
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(first.snapshot().progress.done, stopped_at);
    drop(first);
    // The interruption landed mid-body: exactly one row is `active` in the
    // store, which is what the next start resumes (a row published before
    // the stop is `done` and is never fetched again).
    let jobs = store.downloads().pending("42", 100).expect("the queue");
    let active = jobs
        .iter()
        .filter(|job| job.state == DownloadState::Active)
        .count();
    assert_eq!(active, 1, "one track was interrupted while being written");
    // What the SERVER says it sent, per track: the acceptance counts bytes
    // fetched, never a file's size on disk (a published file looks the same
    // whether it came down once or twice).
    let published: Vec<String> = ids
        .iter()
        .filter(|id| {
            store
                .downloads()
                .local("42", &TrackId::new((*id).clone()))
                .expect("the store")
                .is_some()
        })
        .cloned()
        .collect();
    let before: Vec<(String, usize, u64)> = published
        .iter()
        .map(|id| (id.clone(), server.hits(id), server.delivered(id)))
        .collect();
    println!(
        "interrompu : {active} titre actif, {stopped_at} publiés, {} restants, \
             {} octets déjà livrés pour eux",
        jobs.len(),
        before.iter().map(|(_, _, bytes)| *bytes).sum::<u64>()
    );

    let second = Offline::start(store.clone());
    second.configure(api, runtime.handle().clone());
    assert!(
        wait_for(Duration::from_secs(60), || {
            second.snapshot().progress.done == borrowed.len()
        }),
        "the restarted worker finished the set: {:?}",
        second.snapshot().progress
    );
    assert_eq!(
        second.snapshot().progress.failed,
        0,
        "an interruption is not a failure"
    );
    // A file that was published before the stop costs neither a request nor
    // a byte the second time (`an_interrupted_download_resumes_after_a_restart`
    // proves the store's half; this is the server's own count).
    for (id, hits, bytes) in &before {
        assert_eq!(
            server.hits(id),
            *hits,
            "{id} was published before the stop and is never asked again"
        );
        assert_eq!(server.delivered(id), *bytes, "{id} cost no further byte");
    }
    // The row the crash caught was resumed, and no track was fetched twice.
    for id in &ids {
        assert!(
            second.local(id).is_some(),
            "{id} is on disk after the restart"
        );
    }
}
/// **A session change mid-download does NOT touch the runtime.**
///
/// Signing out and in again swaps the `Api` (`Offline::configure`) and leaves
/// the thread and the runtime it borrows alone: only the app CLOSING joins it,
/// which is why a logout cannot panic `nz-bulk` — the shared panic is a
/// lifetime, not a session. The two people's queues also stay apart (the store
/// keys a row by owner), so the new session never fetches the previous
/// person's row.
#[test]
fn a_session_change_mid_download_keeps_the_worker_and_separates_the_queues() {
    in_its_own_process("tests::the_account_bench_in_its_own_process");
}

#[test]
#[ignore = "run by its parent, a process of its own (a panic hook is process-wide)"]
fn the_account_bench_in_its_own_process() {
    let panics = Panics::install();
    let scratch = Scratch::new("account");
    let store = scratch.store();
    let mine = ["mine1", "mine2"];
    let server = TestServer::start(
        Link::new(Some(50_000.0)),
        body_map(&["mine1", "mine2", "theirs"], 64 * 1024),
    );
    let runtime = runtime();
    let api = api_for(&server, &store, &scratch); // owner 42
    let service = Offline::start(store.clone());
    service.configure(api, runtime.handle().clone());
    service.choose("playlist", "p1", "Ma liste", choices(&mine));
    assert!(
        wait_for(Duration::from_secs(20), || {
            service.snapshot().running.is_some()
        }),
        "the first account's queue runs: {:?}",
        service.snapshot().progress
    );

    // Somebody else signs in: `configure` swaps the client, nothing else.
    let theirs = api_for(&server, &store, &scratch);
    theirs.set_owner(Some("7".to_owned()));
    service.configure(theirs, runtime.handle().clone());
    service.choose_track(Choice::new("theirs", "Leur titre"));
    assert!(
        wait_for(Duration::from_secs(20), || service
            .local("theirs")
            .is_some()),
        "the new session's download runs: {:?}",
        service.snapshot().progress
    );
    // The previous person's queue is untouched, and their remaining track was
    // not fetched with the new session.
    let left = store.downloads().pending("42", 100).expect("the queue");
    assert!(
        left.iter()
            .any(|job| job.track.as_str() == "mine2" && job.state == DownloadState::Pending),
        "the previous account's queue is kept: {:?}",
        left.iter()
            .map(|job| (job.track.as_str(), job.state))
            .collect::<Vec<_>>()
    );
    assert!(
        store
            .downloads()
            .local("42", &TrackId::new("mine2"))
            .expect("the store")
            .is_none(),
        "the new session does not download into the previous account's library"
    );
    println!(
        "changement de compte : « theirs » téléchargé, {} ligne(s) gardées pour 42",
        left.len()
    );
    service.shutdown();
    drop(runtime);
    assert!(
        panics.take().is_empty(),
        "a session change must not panic the worker"
    );
}
// --- the acceptance ----------------------------------------------------

/// **Rule 1, measured.** A 200-track playlist is downloaded through
/// `Lane::Bulk` while a stream is being read like the player reads one: the
/// stream keeps its share of the link (the drop is under 30 %), because the
/// meter feeds the Bulk cap at 25 % of it.
///
/// The link is a shared token bucket (400 kB/s) rather than loopback's
/// unlimited bandwidth, so the measurement is the rule and not the kernel.
#[test]
fn a_two_hundred_track_download_keeps_the_playing_streams_share() {
    in_its_own_process("tests::the_share_bench_in_its_own_process");
}

/// The bench itself: see the parent above for why it has a process of its
/// own.
#[test]
#[ignore = "run by its parent, a process of its own (RSS neighbours)"]
fn the_share_bench_in_its_own_process() {
    let scratch = Scratch::new("share");
    let store = scratch.store();
    let ids: Vec<String> = (0..200).map(|index| format!("bulk{index}")).collect();
    let borrowed: Vec<&str> = ids.iter().map(String::as_str).collect();
    let mut bodies = body_map(&borrowed, 8 * 1024);
    bodies.insert("alone".to_owned(), filler(6 * 1024 * 1024, 1));
    bodies.insert("shared".to_owned(), filler(6 * 1024 * 1024, 2));
    let server = TestServer::start(Link::new(Some(800_000.0)), bodies);
    let runtime = runtime();
    let api = api_for(&server, &store, &scratch);
    let service = Offline::start(store.clone());
    service.configure(api.clone(), runtime.handle().clone());

    // Phase 1: the stream has the link to itself.
    // Four seconds, not two: the first half second of phase 2 still has the
    // cap the ALONE phase measured, and a short window weighs that
    // transient far too heavily (measured 38 % at 2 s, 22–29 % at 3–4 s).
    let window = Duration::from_secs(4);
    let alone = read_stream(&runtime, &api, &service, "alone", window);

    // Phase 2: 200 tracks download at the same time.
    service.choose("playlist", "p1", "Voyage", choices(&borrowed));
    assert!(
        wait_for(Duration::from_secs(10), || {
            let snapshot = service.snapshot();
            snapshot.progress.pending > 0 || snapshot.progress.active > 0
        }),
        "the queue started"
    );
    // Let the meter converge before measuring: the cap still carries the
    // ALONE phase's rate for the first few hundred milliseconds, and the two
    // sockets' buffers absorb a burst on top of it. What this test is about
    // is the steady state, which is what a person lives through.
    std::thread::sleep(Duration::from_secs(1));
    let shared = read_stream(&runtime, &api, &service, "shared", window);

    // The mechanism, on its own: the cap the meter armed is a quarter of the
    // stream it measured, never under 64 kB/s.
    let cap = service
        .bulk_cap_for_test()
        .expect("the meter armed the cap");
    let quarter = (alone / 4.0) as u64;
    println!(
        "bulk cap armed at {cap} B/s for a stream measured at {alone:.0} B/s (quarter {quarter})"
    );
    assert!(
        cap <= quarter + quarter / 10,
        "the Bulk cap is 25 % of the playing stream: {cap} against {quarter}"
    );

    let drop = (1.0 - shared / alone) * 100.0;
    println!(
        "playing stream: {alone:.0} B/s alone, {shared:.0} B/s while 200 tracks download \
             ({drop:.1} % drop, budget 30 %)"
    );
    assert!(
        drop < 30.0,
        "the playing stream kept at least 70 % of its throughput (measured {drop:.1} % drop)"
    );
    assert!(
        drop > 5.0,
        "the downloads really were competing (measured {drop:.1} % drop)"
    );
    let snapshot = service.snapshot();
    assert!(
        snapshot.progress.done > 0,
        "the bulk lane made progress: {:?}",
        snapshot.progress
    );
    // Close like the app does: the worker is joined before its runtime goes
    // (`NetRuntime::drop`), which is the contract this bench used to break.
    service.shutdown();
}

/// Read a `Lane::User` stream the way a player does — chunk by chunk, for
/// `window` — and answer the average throughput, feeding the meter the
/// player's reader feeds in the app.
fn read_stream(
    runtime: &tokio::runtime::Runtime,
    api: &Api,
    service: &Offline,
    id: &str,
    window: Duration,
) -> f64 {
    let mut response = runtime
        .block_on(api.stream(Lane::User, &TrackId::new(id), Quality::Opus320, None, None))
        .expect("a stream");
    let started = Instant::now();
    let mut bytes = 0u64;
    while started.elapsed() < window {
        match runtime.block_on(response.chunk()) {
            Ok(Some(chunk)) => {
                bytes += chunk.len() as u64;
                service.note_stream(chunk.len() as u64);
            }
            Ok(None) => break,
            Err(error) => panic!("stream {id}: {}", error.cause_fr()),
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    bytes as f64 / elapsed
}

/// **A process ending mid-download: the last `Offline` clone joins `nz-bulk`
/// before the runtime it borrows goes.**
///
/// This is D13's pitfall in the crate's own words (the fix PR #246 landed): the
/// worker owns its thread, `shutdown` and `Drop` both join it, `Stop` abandons a
/// body in flight through `select!`, and nothing `catch_unwind`s the panic away.
/// The desktop's `NetRuntime` bench proves the app's global does the same; this
/// one proves the CRATE's contract, which is what the mobile core relies on when
/// it drops its runtime.
#[test]
fn dropping_the_worker_joins_it_before_its_runtime_goes() {
    in_its_own_process("tests::the_drop_bench_in_its_own_process");
}

#[test]
#[ignore = "run by its parent, a process of its own (a panic hook is process-wide)"]
fn the_drop_bench_in_its_own_process() {
    let panics = Panics::install();
    let scratch = Scratch::new("drop");
    let store = scratch.store();
    let ids: Vec<String> = (0..20).map(|index| format!("d{index}")).collect();
    let borrowed: Vec<&str> = ids.iter().map(String::as_str).collect();
    // A body in flight, and a Bulk cap that makes the worker SLEEP between its
    // chunks: that sleep is the timer tokio asserts on once its runtime goes.
    let server = TestServer::start(Link::new(Some(400_000.0)), body_map(&borrowed, 128 * 1024));
    let runtime = Arc::new(runtime());
    let api = api_for(&server, &store, &scratch);
    api.set_bulk_cap_from_stream(Some(8_000));
    let service = Offline::start(store.clone());
    service.configure(Arc::clone(&api), runtime.handle().clone());
    service.choose("playlist", "p1", "Voyage", choices(&borrowed));
    assert!(
        wait_for(Duration::from_secs(20), || {
            service
                .snapshot()
                .running
                .as_ref()
                .is_some_and(|running| running.bytes > 0)
        }),
        "a body is being written: {:?}",
        service.snapshot().running
    );

    // The process ends: the last handle goes, and only then its runtime.
    let closing = Instant::now();
    drop(service);
    let closed = closing.elapsed();
    drop(runtime);
    // A worker that outlived the drop would poll a shutdown runtime HERE.
    std::thread::sleep(Duration::from_millis(300));

    let recorded = panics.take();
    println!(
        "fermeture par le dernier handle : rendue en {closed:?}, {} panique(s)",
        recorded.len()
    );
    assert!(
        recorded.is_empty(),
        "no thread may panic while the process ends: {recorded:?}"
    );
    assert!(
        closed < Duration::from_millis(500),
        "the drop answered in {closed:?}, not behind the body in flight"
    );
    // The interrupted row stays `active` and its `.part` is on disk: the next
    // start resumes it, and an interruption is not a verdict (rule 2).
    let jobs = store.downloads().pending("42", 100).expect("the queue");
    assert_eq!(
        jobs.iter()
            .filter(|job| job.state == DownloadState::Active)
            .count(),
        1,
        "exactly the body in flight is interrupted: {jobs:?}"
    );
}

/// The picker offers exactly the rungs an engine can decode.
///
/// This is the deliberate difference from the SPA's six: the server also answers
/// `OPUS_256`/`OPUS_192`, but no decoder in either app has a rung for them, so
/// downloading one would fetch a file the app can play only by accident.
#[test]
fn the_picker_offers_only_the_rungs_the_engine_decodes() {
    let mut offered = Vec::new();
    for (quality, label, hint) in QUALITIES {
        // A lossy rung names itself on the wire; FLAC is the absence of `q`.
        if let Some(wire) = quality.wire() {
            assert!(
                nz_audio::Quality::parse(wire).is_some(),
                "« {label} » ({wire}) is a rung the engine decodes — {hint}"
            );
        } else {
            assert_eq!(quality, Quality::Flac, "only FLAC is spelled by silence");
        }
        offered.push(quality);
    }
    assert_eq!(offered.len(), 4, "four rungs, not the SPA's six");
    for wire in ["OPUS_256", "OPUS_192"] {
        // The server answers them, the crate can name them, no engine decodes one.
        assert!(
            Quality::parse(Some(wire)).is_some(),
            "{wire} is a known quality"
        );
        assert!(
            nz_audio::Quality::parse(wire).is_none(),
            "{wire} has no rung: the picker must not offer it"
        );
    }
}

/// A publication reaches a waiter through `events`, without a poll and without
/// a clock: the signal carries the `version` the snapshot now has.
#[test]
fn a_publication_wakes_a_waiter_with_its_version() {
    let scratch = Scratch::new("events");
    let service = Offline::start(scratch.store());
    let runtime = runtime();
    let mut events = service.events();
    let before = service.snapshot().version;

    service.set_quality(Quality::Opus128);
    assert!(
        runtime.block_on(events.changed()).is_ok(),
        "the publication wakes the waiter"
    );
    let seen = service.snapshot();
    assert_eq!(
        *events.borrow_and_update(),
        seen.version,
        "the signal IS the snapshot's version"
    );
    assert!(seen.version > before, "and it moved");
    assert_eq!(seen.settings.quality, Quality::Opus128);
    service.shutdown();
}
