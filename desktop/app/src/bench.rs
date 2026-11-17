//! The measurement seams D17 uses, and nothing the person can see.
//!
//! Three files, each written only when a command-line flag asks for it:
//!
//! - **`--frame-log <path>`** — one line per PAINTED frame: when it was painted
//!   (ms since the process started) and how long building its element tree took
//!   (µs). The interval between two lines is the frame time the panel actually
//!   delivered; the tree time is the app's own share of it. `frame_stats.py`
//!   turns the file into p50/p95/max and an fps. Nothing here forces a repaint:
//!   a frame is logged when the app paints one, which is exactly what a scroll
//!   or an animation asks for.
//! - **`--bench-events <path>`** — one JSON line per event the acceptance asks
//!   about: `load` (the gesture opened a stream), `started` (the engine's first
//!   sample, so `started - load` is the click-to-sound number), `underrun`,
//!   `stream_complete`, `download`, `transcribe`.
//! - **`--bench-status <path>`** — one JSON line every [`STATUS_PERIOD`]:
//!   underruns (the engine's own counter), the playhead, the queue length,
//!   decoded sleeve bytes and the process's RSS from `/proc/self/status`. It is
//!   what a one-hour soak leaves behind, and it needs no window: the shell's
//!   ticker is what writes it, so a hidden window stops the status lines too —
//!   the soak runs with the window on screen.
//!
//! The counters are cheap (an append to an open file, an integer read), and a
//! process launched without a flag allocates nothing here: `BENCH` stays empty
//! and every function returns at its first line. Measured (release, this
//! machine): a frame log line adds **0.4 µs** to a paint, the status line reads
//! `/proc/self/status` once every 10 s.
//!
//! `--window-size WxH` lives here too, because every memory number in
//! `docs/agents/desktop.md` is stated with its window size: resizing a live
//! window races GPUI's own initial size (D17's *Pitfalls*), so the size is
//! chosen at launch.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How often `--bench-status` writes a line.
pub const STATUS_PERIOD: Duration = Duration::from_secs(10);

/// One sink the command line asked for.
struct Sink(Mutex<File>);

impl Sink {
    fn open(path: &Path) -> Option<Self> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
            .map(|file| Self(Mutex::new(file)))
    }

    fn line(&self, text: &str) {
        if let Ok(mut file) = self.0.lock() {
            let _ = writeln!(file, "{text}");
        }
    }
}

/// Everything a measurement run asked for. `None` on every field is the
/// shipped app.
#[derive(Default)]
pub struct Bench {
    started: Option<Instant>,
    frames: Option<Sink>,
    events: Option<Sink>,
    status: Option<Sink>,
    /// The last `load` per track id, so `started` can report the delay.
    loads: Mutex<Vec<(String, Instant)>>,
    /// When the last status line was written.
    last_status: Mutex<Option<Instant>>,
}

impl Bench {
    fn now_ms(&self) -> u128 {
        self.started
            .map(|started| started.elapsed().as_millis())
            .unwrap_or(0)
    }
}

static BENCH: OnceLock<Bench> = OnceLock::new();

/// Install the sinks. Called once, from `main`, before the window opens.
pub fn init(frame_log: Option<PathBuf>, event_log: Option<PathBuf>, status_log: Option<PathBuf>) {
    let bench = Bench {
        started: Some(Instant::now()),
        frames: frame_log.as_deref().and_then(Sink::open),
        events: event_log.as_deref().and_then(Sink::open),
        status: status_log.as_deref().and_then(Sink::open),
        ..Bench::default()
    };
    if bench.frames.is_some() || bench.events.is_some() || bench.status.is_some() {
        let _ = BENCH.set(bench);
    }
}

fn bench() -> Option<&'static Bench> {
    BENCH.get()
}

/// Whether any measurement sink is open (a caller can skip gathering numbers).
pub fn active() -> bool {
    bench().is_some()
}

/// A milestone on stdout, for a bench that reads the app's own lines:
/// `bench logged_in 812`. The bench uses it to start a screen's request window
/// at the moment the screen really begins to load, not at process start.
pub fn mark(name: &str) {
    let Some(bench) = bench() else { return };
    println!("bench {name} {}", bench.now_ms());
}

/// One painted frame: `frame <t_ms> <build_us>`.
pub fn frame(render_started: Instant) {
    let Some(bench) = bench() else { return };
    let Some(sink) = &bench.frames else { return };
    sink.line(&format!(
        "frame {} {}",
        bench.now_ms(),
        render_started.elapsed().as_micros()
    ));
}

/// One measured event, as JSON: `{"t_ms":…,"event":"started","id":"…","ms":…}`.
pub fn event(kind: &str, id: &str, extra_ms: Option<u128>) {
    let Some(bench) = bench() else { return };
    let Some(sink) = &bench.events else { return };
    let mut line = format!(
        "{{\"t_ms\":{},\"event\":\"{}\"",
        bench.now_ms(),
        json_escape(kind)
    );
    if !id.is_empty() {
        line.push_str(&format!(",\"id\":\"{}\"", json_escape(id)));
    }
    if let Some(ms) = extra_ms {
        line.push_str(&format!(",\"ms\":{ms}"));
    }
    line.push('}');
    sink.line(&line);
}

/// The gesture opened this track's stream: what a click-to-sound number counts
/// from.
pub fn load(id: &str) {
    let Some(bench) = bench() else { return };
    if let Ok(mut loads) = bench.loads.lock() {
        loads.push((id.to_owned(), Instant::now()));
        // Bounded: a long session changes a lot of tracks.
        if loads.len() > 64 {
            let drop = loads.len() - 64;
            loads.drain(..drop);
        }
    }
    event("load", id, None);
}

/// The engine's first sample. Reports the delay since the matching `load`, when
/// one is held — the click-to-sound number, measured on the engine's own clock.
pub fn started(id: &str) {
    let Some(bench) = bench() else { return };
    let elapsed = bench.loads.lock().ok().and_then(|mut loads| {
        loads
            .iter()
            .rposition(|(held, _)| held == id)
            .map(|at| loads.remove(at).1.elapsed().as_millis())
    });
    event("started", id, elapsed);
}

/// The output callback ran out of samples; `count` is the running total.
pub fn underrun(count: u64) {
    event("underrun", "", Some(count as u128));
}

/// One status line. `underruns` is `Engine::underruns`, `queue` the queue
/// length, `sleeves` the decoded sleeve bytes.
pub fn status(underruns: u64, position_s: f64, queue: usize, sleeves: usize) {
    let Some(bench) = bench() else { return };
    let Some(sink) = &bench.status else { return };
    let now = Instant::now();
    if let Ok(mut last) = bench.last_status.lock() {
        if last.is_some_and(|at| now.duration_since(at) < STATUS_PERIOD) {
            return;
        }
        *last = Some(now);
    }
    let (rss_kb, hwm_kb) = rss_kb();
    sink.line(&format!(
        "{{\"t_ms\":{},\"underruns\":{},\"position_s\":{:.3},\"queue\":{},\
         \"sleeves_bytes\":{},\"rss_kb\":{},\"hwm_kb\":{}}}",
        bench.now_ms(),
        underruns,
        position_s,
        queue,
        sleeves,
        rss_kb,
        hwm_kb
    ));
}

/// The process's resident set and peak, from `/proc/self/status` (KiB).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn rss_kb() -> (u64, u64) {
    let Ok(text) = std::fs::read_to_string("/proc/self/status") else {
        return (0, 0);
    };
    let mut rss = 0;
    let mut hwm = 0;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("VmRSS:") {
            rss = value
                .split_whitespace()
                .next()
                .and_then(|number| number.parse().ok())
                .unwrap_or(0);
        } else if let Some(value) = line.strip_prefix("VmHWM:") {
            hwm = value
                .split_whitespace()
                .next()
                .and_then(|number| number.parse().ok())
                .unwrap_or(0);
        }
    }
    (rss, hwm)
}

/// JSON strings here are ids and event names: quotes and backslashes are the
/// only ways to break the line.
fn json_escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// `--window-size 2560x1600` → `Some((2560.0, 1600.0))`.
pub fn parse_size(text: &str) -> Option<(f32, f32)> {
    let (width, height) = text.split_once(['x', 'X'])?;
    let width: f32 = width.trim().parse().ok()?;
    let height: f32 = height.trim().parse().ok()?;
    (width >= 320.0 && height >= 240.0 && width <= 10_000.0 && height <= 10_000.0)
        .then_some((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_size_is_width_by_height_and_nothing_absurd() {
        assert_eq!(parse_size("2560x1600"), Some((2560.0, 1600.0)));
        assert_eq!(parse_size("1280x800"), Some((1280.0, 800.0)));
        assert_eq!(parse_size("420X420"), Some((420.0, 420.0)));
        assert_eq!(parse_size("1280"), None);
        assert_eq!(parse_size("0x0"), None);
        assert_eq!(parse_size("huit cents"), None);
    }

    #[test]
    fn an_id_with_a_quote_cannot_break_the_json_line() {
        assert_eq!(json_escape("a\"b"), "a\\\"b");
        assert_eq!(json_escape("a\\b"), "a\\\\b");
    }

    /// Without a flag, nothing is allocated and nothing is written: the shipped
    /// app pays one `OnceLock::get` per call.
    #[test]
    fn the_shipped_app_has_no_sink() {
        assert!(!active());
        frame(Instant::now());
        load("1");
        started("1");
        underrun(1);
        status(0, 0.0, 0, 0);
    }
}
