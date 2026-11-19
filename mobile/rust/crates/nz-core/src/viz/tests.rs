//! The visualiser's rules, held to: no view means no analysis and no
//! subscription, eco means none, a frame matches the shape the surface asked
//! for, the sleeve colours the row, and the thread is joined.
//!
//! **Nothing here waits on a clock.** "The analysis stopped" is the counter the
//! thread bumps on every wake (`wakes()`), the tick count it must leave alone,
//! and the number of subscriptions the opener was asked for — the same events the
//! desktop's `VizService` test would use, on a machine with no audio device.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::settings::ECO_FPS;
use super::tap::{SyntheticTap, TapSource};
use super::testing::TempDir;
use super::*;

/// Waits for a condition, yielding the CPU — an EVENT, not a duration. The bound
/// is a large iteration count so a slow shared machine does not fail a correct
/// run, and a real regression fails with the message instead of hanging.
fn until(mut check: impl FnMut() -> bool, what: &str) {
    for _ in 0..4_000_000 {
        if check() {
            return;
        }
        std::thread::yield_now();
    }
    panic!("waited for {what} and it never happened");
}

/// A tap the test can watch: the synthetic generator, plus what the engine would
/// have dropped for a reader that fell behind.
struct WatchedTap {
    inner: SyntheticTap,
    reads: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
}

impl TapSource for WatchedTap {
    fn read(&mut self, out: &mut [f32]) -> usize {
        self.reads.fetch_add(1, Ordering::AcqRel);
        self.inner.read(out)
    }
    fn frames_available(&self) -> usize {
        self.inner.frames_available()
    }
    fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Acquire)
    }
    fn clear(&mut self) {
        self.inner.clear();
    }
}

/// What the opener handed out, so a test can count subscriptions and reads.
#[derive(Default)]
struct Bench {
    opened: AtomicU64,
    reads: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
    /// `false` = the engine has no tap (nothing ever played).
    available: Mutex<bool>,
}

impl Bench {
    fn opener(self: &Arc<Self>) -> TapOpener {
        let bench = Arc::clone(self);
        Arc::new(move || {
            if !*bench.available.lock().unwrap_or_else(|e| e.into_inner()) {
                return None;
            }
            bench.opened.fetch_add(1, Ordering::AcqRel);
            Some(Box::new(WatchedTap {
                inner: SyntheticTap::new(),
                reads: Arc::clone(&bench.reads),
                dropped: Arc::clone(&bench.dropped),
            }))
        })
    }

    fn opened(&self) -> u64 {
        self.opened.load(Ordering::Acquire)
    }
}

fn viz(dir: &TempDir) -> (Arc<Viz>, Arc<Bench>) {
    let bench = Arc::new(Bench {
        available: Mutex::new(true),
        ..Bench::default()
    });
    let core = Arc::new(dir.core("viz-test"));
    (Viz::new(core, bench.opener()), bench)
}

#[test]
fn no_view_means_no_thread_no_subscription_and_no_tick() {
    let dir = TempDir::new("viz-hidden");
    let (viz, bench) = viz(&dir);
    // A service nobody looked at: the thread is not even started.
    assert_eq!(viz.ticks(), 0);
    assert_eq!(
        viz.wakes(),
        0,
        "the thread starts at the first visible view"
    );
    assert_eq!(bench.opened(), 0);

    // Visible: one subscription, analysis frames.
    viz.set_visible("bars", true);
    until(|| viz.ticks() > 0, "the first analysis frame");
    assert_eq!(bench.opened(), 1, "ONE subscription for one view");
    assert_eq!(viz.subscribes(), 1);
    assert!(viz.is_live());
    // The levelling needs a few frames to settle: the EVENT is "a bar is lit",
    // not "one frame was published".
    until(
        || viz.frame().bars.iter().any(|&v| v > 0.02),
        "the synthetic tone to light the row",
    );

    // Hidden again: the subscription goes, and the tick count must not move.
    // The block already in flight finishes (10 ms of audio, one tick at most);
    // the count is frozen once the thread has SEEN the gate.
    viz.set_visible("bars", false);
    assert!(!viz.is_live());
    let wakes = viz.wakes();
    until(|| viz.wakes() >= wakes + 3, "the thread to notice the gate");
    let frozen = viz.ticks();
    until(|| viz.wakes() >= wakes + 8, "the thread to settle");
    assert_eq!(
        viz.ticks(),
        frozen,
        "0 analysis tick while no view is visible"
    );
    assert_eq!(bench.opened(), 1, "the subscription was not re-opened");

    // And a second view subscribes again — the refcount is the views', not the
    // service's.
    viz.set_visible("scope", true);
    until(|| bench.opened() == 2, "the second subscription");
    viz.set_visible("scope", false);
    viz.shutdown();
}

#[test]
fn two_views_share_one_subscription_and_one_thread() {
    let dir = TempDir::new("viz-two-views");
    let (viz, bench) = viz(&dir);
    viz.set_visible("bars", true);
    viz.set_visible("eq", true);
    until(|| viz.ticks() > 0, "an analysis frame");
    assert_eq!(bench.opened(), 1, "one thread, one subscription, two views");
    viz.set_visible("bars", false);
    assert!(viz.is_live(), "the meter is still looking");
    viz.set_visible("eq", false);
    assert!(!viz.is_live());
    viz.shutdown();
}

#[test]
fn eco_means_none_and_caps_at_thirty() {
    let dir = TempDir::new("viz-eco");
    let (viz, bench) = viz(&dir);
    viz.set_visible("bars", true);
    until(|| viz.ticks() > 0, "an analysis frame");

    let settings = viz
        .set_setting("viz.eco", serde_json::Value::Bool(true))
        .expect("the eco switch is a known key");
    assert_eq!(settings.mode, "off", "stores.js#ecoMode: none, not smaller");
    assert_eq!(settings.fps, ECO_FPS);
    assert!(!viz.is_live(), "eco turns the analysis off at once");
    // The block already in flight finishes (one tick at most, 10 ms of audio);
    // the count is frozen from the moment the thread has seen the gate.
    let wakes = viz.wakes();
    until(|| viz.wakes() >= wakes + 3, "the thread to notice eco");
    let frozen = viz.ticks();
    until(|| viz.wakes() >= wakes + 8, "the thread to settle");
    assert_eq!(viz.ticks(), frozen, "0 tick in eco once the gate is seen");
    // The still frame is the rest frame: a flat row, never a frozen last one.
    let frame = viz.frame();
    assert!(frame.bars.iter().all(|&v| v == 0.0));

    // Turning it off restores the chosen mode and the chosen cap.
    let settings = viz
        .set_setting("viz.eco", serde_json::Value::Bool(false))
        .expect("eco off");
    assert_eq!(settings.mode, "bars");
    assert_eq!(settings.fps, 60);
    until(|| viz.ticks() > frozen, "the analysis to resume");
    // Eco released the subscription when nothing was drawing and turning it
    // off took a new one: that IS the "0 cost with no viewer" rule on the tap's
    // own side (MB12's Drop unsubscribes), not a leak.
    assert_eq!(bench.opened(), 2, "one subscription per visible stretch");
    viz.shutdown();
}

#[test]
fn the_chosen_scope_and_its_shape_come_back_from_the_store() {
    let dir = TempDir::new("viz-settings");
    let (viz, _bench) = viz(&dir);
    let settings = viz
        .set_setting("viz.mode", serde_json::Value::String("scope".to_owned()))
        .expect("viz.mode is a known key");
    assert_eq!(settings.mode, "scope");
    assert!(settings.needs_wave());
    // Persisted: a fresh read of the device's store says the same thing.
    assert_eq!(VizSettings::read(viz.core()).mode, "scope");

    viz.set_shape(200, 4_000);
    viz.set_visible("scope", true);
    // A full trigger window (4 096 samples ≈ 9 reads) before the probe reports
    // a signal: the event is the signal, not the first frame.
    until(|| viz.frame().signal, "the probe to see the tone");
    let frame = viz.frame();
    assert_eq!(
        frame.bars.len(),
        super::frame::MAX_BARS,
        "clamped, not refused"
    );
    assert_eq!(frame.scope_a.len(), super::frame::MAX_COLS * 2);
    assert!(frame.gain > 0.0 && frame.signal, "the probe sees the tone");
    viz.shutdown();
}

#[test]
fn bars_mode_computes_no_scope_and_scope_mode_does() {
    let dir = TempDir::new("viz-needs-wave");
    let (viz, _bench) = viz(&dir);
    viz.set_visible("bars", true);
    until(|| viz.ticks() > 0, "an analysis frame in bars mode");
    assert!(
        viz.frame().scope_a.is_empty(),
        "modes.js#needsWave is false for the bars"
    );
    viz.set_visible("bars", false);
    viz.set_setting("viz.mode", serde_json::Value::String("scope".to_owned()))
        .expect("viz.mode");
    viz.set_visible("scope", true);
    until(|| !viz.frame().scope_a.is_empty(), "a trace in scope mode");
    viz.shutdown();
}

#[test]
fn a_refused_setting_changes_nothing() {
    let dir = TempDir::new("viz-refused");
    let (viz, _bench) = viz(&dir);
    let before = viz.settings();
    // `viz.mode` is validated by the store's own whitelist and vocabulary: a
    // number is not a mode, and the screen must keep what it had.
    let err = viz
        .set_setting("viz.mode", serde_json::Value::from(42))
        .expect_err("a number is not a mode");
    assert!(matches!(err, CoreError::Invalid { .. }), "{err:?}");
    assert_eq!(viz.settings(), before);
    viz.shutdown();
}

#[test]
fn the_sleeve_colours_the_row_and_a_track_without_one_forgets_it() {
    let dir = TempDir::new("viz-artwork");
    let (viz, _bench) = viz(&dir);
    // A 30×30 sleeve: red on the left, green in the middle, blue on the right —
    // `nz-viz::palette`'s own fixture shape.
    let (w, h) = (30usize, 30usize);
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let (r, g, b) = if x < 10 {
                (200, 20, 20)
            } else if x < 20 {
                (20, 200, 20)
            } else {
                (20, 20, 200)
            };
            let i = (y * w + x) * 4;
            rgba[i..i + 4].copy_from_slice(&[r, g, b, 255]);
        }
    }
    viz.set_artwork_pixels(rgba, w as u32, h as u32);
    viz.set_visible("bars", true);
    until(
        || viz.frame().has_zones,
        "the sleeve's zones to reach the frame",
    );
    let frame = viz.frame();
    let hues: Vec<f64> = frame.zones.iter().map(|z| z.hue).collect();
    assert!(
        hues[0] < 30.0 && hues[super::palette::ZONE_COUNT - 1] > 200.0,
        "red at the left end, blue at the right: {hues:?}"
    );

    viz.clear_artwork();
    until(
        || !viz.frame().has_zones,
        "the row to go back to its low -> high walk",
    );
    viz.shutdown();
}

#[test]
fn what_the_reader_lost_is_reported_and_counted() {
    let dir = TempDir::new("viz-dropped");
    let bench = Arc::new(Bench {
        available: Mutex::new(true),
        ..Bench::default()
    });
    bench.dropped.store(7, Ordering::Release);
    let core = Arc::new(dir.core("viz-test"));
    let viz = Viz::new(core, bench.opener());
    viz.set_visible("bars", true);
    until(|| viz.ticks() > 0, "an analysis frame");
    viz.set_visible("bars", false);
    // The reader's own lost frames are added when the subscription is dropped.
    until(|| viz.dropped() >= 7, "the dropped frames to be reported");
    assert!(bench.reads.load(Ordering::Acquire) > 0);
    viz.shutdown();
}

#[test]
fn with_no_engine_the_service_parks_and_says_nothing() {
    let dir = TempDir::new("viz-no-tap");
    let bench = Arc::new(Bench {
        available: Mutex::new(false),
        ..Bench::default()
    });
    let core = Arc::new(dir.core("viz-test"));
    let viz = Viz::new(core, bench.opener());
    viz.set_visible("bars", true);
    // The opener was ASKED (the engine simply has no tap yet) and no analysis
    // ran: the screen keeps its rest frame, and this is not an error.
    until(|| viz.wakes() >= 3, "the thread to try the tap");
    assert_eq!(viz.ticks(), 0);
    assert_eq!(viz.subscribes(), 0);
    assert_eq!(viz.frame().bars.iter().filter(|&&v| v > 0.0).count(), 0);
    viz.shutdown();
}

#[test]
fn the_frame_is_counted_in_bytes_and_stays_small() {
    let dir = TempDir::new("viz-memory");
    let (viz, _bench) = viz(&dir);
    viz.set_visible("bars", true);
    until(|| viz.ticks() > 0, "an analysis frame");
    let bytes = viz.memory_bytes();
    // The published frame (~4.5 kB) plus `nz-viz`'s fixed buffers, and nothing
    // that grows with time (worker rule 4: counted bytes, no RSS).
    assert!(bytes > 4_000, "{bytes} bytes");
    assert!(
        bytes < 128 * 1024,
        "{bytes} bytes is a frame and its buffers"
    );
    let before = viz.memory_bytes();
    until(|| viz.ticks() >= 40, "forty frames");
    let after = viz.memory_bytes();
    assert_eq!(
        after, before,
        "the published frame is replaced, not accumulated"
    );
    viz.shutdown();
}

#[test]
fn the_thread_is_joined_and_never_restarted() {
    let dir = TempDir::new("viz-join");
    let (viz, bench) = viz(&dir);
    viz.set_visible("bars", true);
    until(|| viz.ticks() > 0, "an analysis frame");
    viz.shutdown();
    // Idempotent, and a screen opened after the exit path does not resurrect it.
    viz.shutdown();
    viz.set_visible("bars", true);
    let wakes = viz.wakes();
    std::thread::yield_now();
    std::thread::yield_now();
    assert_eq!(viz.wakes(), wakes, "no thread is left to wake");
    assert_eq!(bench.opened(), 1);
    viz.shutdown();
}
