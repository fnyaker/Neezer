//! **What a preloaded voice costs the link** (spec-track-change-speed, lot 6).
//!
//! The web bench (`webapp/test/prefetch/run.mjs`) counts the preload's bytes
//! from CDP; the native stack has no browser to watch, so this test reads the
//! number where the engine really spends them: the preloaded voice's own spool
//! file, which is the read-ahead the decoder was given off the network
//! (`desktop/crates/nz-audio/src/source.rs`). The question the lot has to answer
//! is whether a SECOND preloaded voice (depth 2) is worth its link: the engine
//! holds one (`Engine::preload_next`, `install_next`), and a prefetch is capped
//! at 512 KiB whatever the read-ahead setting says
//! (`engine.rs`, `cap.min(512 * 1024)`), so the answer is per voice.
//!
//! Thirty seconds of playback, then five more: the number must be the cap and
//! must not grow — a prefetch that kept filling would be a background download
//! racing the person (rule 1), not a preload.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use super::*;
use crate::player::Command;

/// The engine's own bound for a prefetch (`engine.rs`, read at every open).
const PRELOAD_CAP: u64 = 512 * 1024;
/// The over-read one or two chunks of the spooler cost on top of the cap; the
/// read-ahead test uses the same slack and measured 193 536 B for a 64 KiB cap.
const OVERREAD: u64 = 128 * 1024;

/// Every `stream-*` spool file's size: a spool name is a hash of (track,
/// quality), so the files are told apart by WHICH TRACK's body they are, which
/// this test knows from the fixture sizes, not by their names.
fn spool_sizes(dir: &Path) -> BTreeMap<String, u64> {
    let mut sizes = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return sizes;
    };
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("stream-") {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            sizes.insert(name, meta.len());
        }
    }
    sizes
}

/// The preloaded voice reads its 512 KiB cap over 30 s of playback, and then
/// STOPS: a depth-2 preload would pay this bill twice (2 × 512 KiB — 32 s of a
/// 16 KB/s link, 8 s of a 64 KB/s one), which is the number lot 6's note uses
/// to recommend against it.
#[test]
fn a_preloaded_voice_reads_its_cap_over_thirty_seconds_and_then_stops() {
    let dir = TempDir::new("preload-cost");
    let bench = Bench::start();
    // The same 45 s body on both rows, for one reason: it is longer than the
    // 35 s this test plays, so the queue cannot advance mid-measurement and
    // reset the clock. The two spool files are then told apart by SIZE, which
    // is the measurement itself: the row playing reads its whole 729 093 B
    // body (the 2 MB read-ahead of a user lane), the row preloaded stops at
    // the 512 KiB a prefetch may spool.
    let file = Fixture::Noise45.write(dir.path());
    let track_bytes = std::fs::metadata(&file).expect("the fixture").len();
    assert!(
        track_bytes > PRELOAD_CAP + OVERREAD,
        "the fixture is {track_bytes} B: the cap would not be what stops the preload"
    );
    bench.track("playing", &file);
    bench.track("next", &file);

    let player = player_for(&dir, bench.base());
    let (_, mut sink) = player.take_sink().expect("the sink");
    player.command(
        Command::PlayQueue {
            tracks: vec![track_ref_of("playing", 45), track_ref_of("next", 45)],
            start: 0,
            context: None,
        },
        1,
    );
    let _ = pump_until_audio(&mut sink, &player, 30.0, 0.05);

    // The preload is built right after `Started`; wait for the voice, which is
    // the app's own handover condition, not a guessed delay.
    let deadline = Instant::now() + Duration::from_secs(5);
    while player.engine.preloaded_id().as_deref() != Some("next") && Instant::now() < deadline {
        let _ = pump(&mut sink, 0.01);
    }
    assert_eq!(
        player.engine.preloaded_id().as_deref(),
        Some("next"),
        "the next row never became the preloaded voice"
    );

    // 30 s of playback, in the device's own 10 ms buffers.
    let _ = pump(&mut sink, 30.0);
    let spool = dir.path().join("cache/audio");
    let sizes = spool_sizes(&spool);
    let mut bodies: Vec<u64> = sizes.values().copied().filter(|size| *size > 0).collect();
    bodies.sort_unstable();
    println!(
        "after 30 s of playback (spool: {sizes:?}): playing {bodies:?} B, of a {track_bytes} B track"
    );
    assert_eq!(
        bodies.len(),
        2,
        "the playing body and the preloaded one, not {bodies:?}"
    );
    let (preloaded, playing) = (bodies[0], bodies[1]);
    assert_eq!(
        playing, track_bytes,
        "the row the person is hearing did not read its own {track_bytes} B body"
    );
    println!("preloaded voice after 30 s of playback: {preloaded} B");
    assert!(
        preloaded >= 400 * 1024,
        "the preloaded voice read only {preloaded} B in 30 s, so it is not the body being \
         measured ({sizes:?})"
    );
    assert!(
        preloaded <= PRELOAD_CAP + OVERREAD,
        "the preloaded voice read {preloaded} B: a prefetch must stop at {PRELOAD_CAP} B \
         (+{OVERREAD} of read overrun), whatever the track's length"
    );

    // Five more seconds of playback: the body must not grow. A prefetch that
    // keeps filling is a background download of the whole track.
    let _ = pump(&mut sink, 5.0);
    let later = spool_sizes(&spool);
    let later_preloaded = later
        .values()
        .copied()
        .filter(|size| *size > 0)
        .min()
        .unwrap_or(0);
    println!("preloaded voice after 35 s of playback: {later_preloaded} B");
    assert_eq!(
        later_preloaded, preloaded,
        "the preloaded voice kept filling after the cap: {later:?}"
    );

    // The person's own row kept playing throughout — the preload never took
    // the machine from it (rule 1).
    assert!(
        player.position().position_s > 34.0,
        "the playing row reached only {:.1} s in 35 s",
        player.position().position_s
    );
    assert_eq!(
        player.engine.current_id().as_deref(),
        Some("playing"),
        "the queue did not stay on the row the person was hearing"
    );
    player.shutdown();
}
