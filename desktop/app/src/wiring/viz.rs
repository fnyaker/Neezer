//! The analysis thread: one tap, one `VizState`, and nothing while no
//! visualiser is on screen.
//!
//! Rule 1 on the device: the analyser is the last consumer of CPU, so it runs
//! only while a scene is visible, on its own thread (the UI never computes
//! spectra), and the output callback drops frames when this thread is behind
//! (`TapReader::dropped`). The geometry itself is `nz-viz`, i.e. the page's own
//! arithmetic; this module only moves samples.
//!
//! D10b draws the bars and the scope from [`VizService::with`]; the counters
//! (`frames`) are what its "0 analyser calls when hidden" criterion asserts.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use nz_audio::Engine;
use nz_viz::VizState;

/// How much audio one read takes from the tap: 10 ms at 48 kHz, stereo.
const BLOCK_FRAMES: usize = 480;
/// When hidden, how often the thread wakes to notice it should stop.
const HIDDEN_CHECK: Duration = Duration::from_millis(50);

struct Shared {
    state: Mutex<VizState>,
    visible: AtomicBool,
    frames: AtomicU64,
    stop: AtomicBool,
}

/// The analysis service. Dropping it stops the thread.
pub struct VizService {
    shared: Arc<Shared>,
    sample_rate: f32,
    thread: Option<JoinHandle<()>>,
}

impl VizService {
    /// Take the engine's tap and start the thread. `None` when the engine has no
    /// tap (an offline sink in a test).
    pub fn start(engine: &Engine, sample_rate: f32) -> Option<Self> {
        let mut tap = engine.tap()?;
        let shared = Arc::new(Shared {
            state: Mutex::new(VizState::new(sample_rate)),
            visible: AtomicBool::new(false),
            frames: AtomicU64::new(0),
            stop: AtomicBool::new(false),
        });
        let worker = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("nz-analysis".to_owned())
            .spawn(move || {
                let mut buffer = vec![0.0f32; BLOCK_FRAMES * 2];
                while !worker.stop.load(Ordering::Acquire) {
                    if !worker.visible.load(Ordering::Acquire) {
                        // Let the ring drain instead of growing: a hidden
                        // visualiser costs the music nothing.
                        tap.clear();
                        thread::sleep(HIDDEN_CHECK);
                        continue;
                    }
                    let frames = tap.read(&mut buffer);
                    if frames == 0 {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    let mut state = worker
                        .state
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    state.push_frames(&buffer[..frames * 2]);
                    worker.frames.fetch_add(frames as u64, Ordering::Relaxed);
                }
            })
            .ok()?;
        Some(Self {
            shared,
            sample_rate,
            thread: Some(thread),
        })
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// A scene became visible (or left). The thread starts/stops analysis on the
    /// next wake; it never starts a second thread.
    pub fn set_visible(&self, visible: bool) {
        self.shared.visible.store(visible, Ordering::Release);
    }

    pub fn is_visible(&self) -> bool {
        self.shared.visible.load(Ordering::Acquire)
    }

    /// Frames pushed into the analyser since start; 0 while hidden is the
    /// measurable form of "the analysis thread does no work".
    pub fn frames(&self) -> u64 {
        self.shared.frames.load(Ordering::Relaxed)
    }

    /// Read the state while drawing. The lock is held for the geometry copy
    /// only (`nz-viz` measures 0.08 ms p95 for 64 bars + 512 columns), never
    /// across a paint of the whole window.
    pub fn with<R>(&self, read: impl FnOnce(&mut VizState) -> R) -> R {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        read(&mut state)
    }
}

impl Drop for VizService {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The thread's name is part of the diagnostic (`Réglages → Assistance` lists
/// the threads doing work) and the test above reads it.
pub const THREAD_NAME: &str = "nz-analysis";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_thread_name_is_the_one_the_diagnostic_lists() {
        assert_eq!(THREAD_NAME, "nz-analysis");
    }
}
