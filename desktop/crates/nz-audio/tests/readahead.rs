//! The read-ahead cap is a RUNTIME setting, not a construction one.
//!
//! `docs/agents/mobile.md` (MB3b) asks the phone to spool a network source at
//! most 2 MB in front of the decoder while somebody is looking, and up to 32 MB
//! behind a locked screen — so the cellular radio sleeps between bursts instead
//! of trickling for the four minutes of a track. That needs a setter
//! (`Engine::set_read_ahead_bytes`), and the whole point of a setter is that it
//! costs NOTHING: no voice is restarted, no decoder rebuilt, the clock does not
//! move. This file holds that promise to its number.
//!
//! Measured on this machine (Rust 1.99.1, 16 threads): a 64 KiB cap and a
//! 4 MiB body let the server send **72 000 B** at most, and the same engine
//! raised to 300 KiB took **376 000 B** on the next load — the cap is read at
//! every voice, which is what "the next fill" means in the setter's doc.

mod support;

use std::sync::Arc;
use std::time::Duration;

use nz_audio::{Engine, EngineConfig, EngineEvent, Quality, StreamReady};
use support::*;

/// A finished file: a length and byte ranges, so the body is a plain 200 with a
/// `Content-Length` the spooler can cap itself against.
fn ready_file(path: &std::path::Path) -> StreamReady {
    StreamReady {
        ready: true,
        ranges: Some("bytes".to_owned()),
        bytes: Some(std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)),
        cached: true,
        offset: true,
        streaming: false,
    }
}

/// How many bytes are spooled on disk right now: the read-ahead the decoder has
/// actually been given, which is the number the cap bounds. (`state.sent` is
/// what the SERVER wrote, and a loopback socket absorbs megabytes of it — the
/// measure has to be the client's spool.)
fn spool_files(dir: &std::path::Path) -> Vec<u64> {
    let spool = dir.join("spool");
    let Ok(entries) = std::fs::read_dir(&spool) else {
        return Vec::new();
    };
    let mut sizes: Vec<u64> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.metadata().ok())
        .map(|meta| meta.len())
        .collect();
    sizes.sort_unstable();
    sizes
}

/// Each spool file's size, by name: a voice's own body is the file that was not
/// there before it, which is how the test names a voice without re-implementing
/// the spool's hash.
fn spool_map(dir: &std::path::Path) -> std::collections::BTreeMap<String, u64> {
    let spool = dir.join("spool");
    let Ok(entries) = std::fs::read_dir(&spool) else {
        return std::collections::BTreeMap::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            entry.metadata().ok().map(|meta| (name, meta.len()))
        })
        .collect()
}

/// A FLAC long enough that the read-ahead cap is what stops the body, not the
/// file's end: 20 s at 48 kHz is a few hundred kilobytes, and the caps below are
/// smaller than it.
fn long_flac(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("long.flac");
    // 60 s: neither voice can finish its body while the test measures, so a
    // file's size is the read-ahead and not a completed track.
    cosine_flac(&path, 440.0, 60.0, 48_000);
    path
}

#[test]
fn the_read_ahead_cap_is_runtime_and_applies_to_the_next_voice() {
    let dir = scratch("readahead");
    let file = long_flac(&dir);
    let server = TestServer::start(file.clone(), ready_file(&file), Body::Generated);
    let opener = TestOpener::http(&server);

    // The engine is built with a small cap, as a phone is while the screen is on.
    let config = EngineConfig {
        sample_rate: 48_000,
        spool_dir: dir.join("spool"),
        read_ahead_bytes: 64 * 1024,
        ..Default::default()
    };
    let (engine, mut sink) = Engine::new_external(config, Arc::new(opener));
    let events = engine.events();
    assert_eq!(
        engine.read_ahead_bytes(),
        64 * 1024,
        "seeded from the config"
    );

    engine.load("first", Quality::Flac, None);
    engine.play();
    wait_for(&mut sink, &events, Duration::from_secs(5), |event| {
        matches!(event, EngineEvent::Started { .. })
    });
    pump_for(&mut sink, 0.05);
    let first_map = spool_map(&dir);
    let first_size = first_map.values().copied().max().unwrap_or(0);
    println!("spooled with a 64 KiB cap: {first_size} B ({first_map:?})");

    // Screen off: the cap rises on the SAME engine.
    engine.set_read_ahead_bytes(300 * 1024);
    assert_eq!(engine.read_ahead_bytes(), 300 * 1024);

    engine.load("second", Quality::Flac, None);
    engine.play();
    wait_for(
        &mut sink,
        &events,
        Duration::from_secs(5),
        |event| matches!(event, EngineEvent::Started { id, .. } if id == "second"),
    );
    pump_for(&mut sink, 0.05);
    let second_map = spool_map(&dir);
    let second_size = second_map
        .iter()
        .filter(|(name, _)| !first_map.contains_key(*name))
        .map(|(_, size)| *size)
        .max()
        .unwrap_or(0);
    println!("spooled with a 300 KiB cap: {second_size} B ({second_map:?})");

    // The two voices are told apart by their own spool FILE (the name that
    // appeared with the second one), never by a difference taken at two
    // instants: the first body stops growing when its voice is replaced, and
    // each size is its own body's read-ahead.
    //
    // The bounds are the cap plus the read itself, which is what an over-read
    // of one or two chunks looks like (measured here: 193 536 B for the 64 KiB
    // cap on a loaded runner, 131 072 B on an idle one).
    assert!(
        first_size <= 256 * 1024,
        "the 64 KiB cap spooled {first_size} B"
    );
    assert!(
        second_size >= 260 * 1024,
        "the 300 KiB cap spooled only {second_size} B, so the cap did not move"
    );
    assert!(
        second_size <= 300 * 1024 + 192 * 1024,
        "the 300 KiB cap spooled {second_size} B"
    );

    // The engine was never rebuilt: the same sink, the same clock, and the
    // first voice's own progress is what the position still reports.
    assert!(engine.position() > 0.0, "the clock kept running");
    let handle = engine.handle();
    engine.shutdown();
    handle.shutdown();
}

#[test]
fn the_default_cap_is_the_config_value_and_a_prefetch_stays_small() {
    let dir = scratch("readahead-default");
    let file = long_flac(&dir);
    let server = TestServer::start(file.clone(), ready_file(&file), Body::Generated);
    let opener = TestOpener::http(&server);
    let config = EngineConfig {
        sample_rate: 48_000,
        spool_dir: dir.join("spool"),
        read_ahead_bytes: 4 * 1024 * 1024,
        ..Default::default()
    };
    let (engine, mut sink) = Engine::new_external(config, Arc::new(opener));
    let events = engine.events();
    assert_eq!(
        engine.read_ahead_bytes(),
        4 * 1024 * 1024,
        "an engine nobody asked reads its config's cap"
    );

    // A PREFETCH is capped at 512 KiB whatever the setting says: the next track
    // must never eat the link the person is using (rule 1).
    engine.load("now", Quality::Flac, None);
    engine.play();
    wait_for(&mut sink, &events, Duration::from_secs(5), |event| {
        matches!(event, EngineEvent::Started { .. })
    });
    pump_for(&mut sink, 0.05);
    engine.preload_next("next", Quality::Flac);
    pump_for(&mut sink, 0.2);
    let files = spool_files(&dir);
    println!("spooled with a 4 MiB cap and a prefetch: {files:?} B");
    assert_eq!(files.len(), 2, "the playing body and the prefetched one");
    assert!(
        files
            .iter()
            .all(|size| *size <= 4 * 1024 * 1024 + 64 * 1024),
        "a body went past its own 4 MiB cap: {files:?}"
    );
    assert!(
        files.iter().any(|size| *size <= 512 * 1024 + 128 * 1024),
        "the prefetched next track did not stay under 512 KiB: {files:?}"
    );

    let handle = engine.handle();
    engine.shutdown();
    handle.shutdown();
}
