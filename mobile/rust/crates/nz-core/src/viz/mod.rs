//! The visualiser: the bars and the oscilloscope, and the analysis thread that
//! feeds them — and nothing at all while no view is visible.
//!
//! Owned by **MB21** (docs/agents/mobile.md, "Module and file ownership"). The
//! drawing is `feature/viz/**` on a Compose `Canvas`; the numbers are `nz-viz`
//! (the page's own arithmetic); this module is the SERVICE between them: one
//! thread, one subscription on the audio tap, one published frame, and the
//! settings read from the device's store.
//!
//! # The rule the whole module obeys (rule 1 on the phone)
//!
//! **The analyser is the LAST consumer of CPU, so it runs only while a scene is
//! visible.** `set_visible(view, bool)` is called from a view's start and
//! dispose — never on a timer — and the thread:
//!
//! 1. with NO view visible, holds NO subscription on the tap (MB12's
//!    `Player::subscribe_tap` is refcounted and Drop unsubscribes) and analyses
//!    nothing: `ticks()` stays 0, measured by a test that waits on the thread's
//!    own wake counter, not on a clock;
//! 2. with a view visible, opens ONE subscription, reads 10 ms blocks (the
//!    desktop's `BLOCK_FRAMES`), pushes them into `nz-viz` and publishes a frame
//!    per analysis frame;
//! 3. when the last view leaves, drops the subscription and the published
//!    frame, so a re-opened screen starts on a blank picture instead of the last
//!    frame of the previous session (`Visualizer.svelte#goIdle`).
//!
//! The thread is joined by [`Viz::shutdown`] and by `Drop` — never left to the
//! process' death path alone (worker rule 5; it borrows no runtime, so it can be
//! joined at any moment).
//!
//! # Two rates, on purpose
//!
//! ANALYSIS runs at the analyser's cadence (~94 Hz at 48 kHz) and PAINTING at
//! the person's cap (30 or 60; eco caps at 30). They are separate because a beat
//! or an onset caught at 94 Hz must not be missed because the screen was capped
//! to 30, and because a 120 Hz panel must not cost 120 analyses a second
//! (`Visualizer.svelte`: "a scene never misses an event and never paints more
//! often than the screen (or the user) wants").
//!
//! # What the surface asks for
//!
//! `set_shape(bars, cols)` on a size or mode change (never per frame: changing
//! either resets an envelope), then `frame()` per paint — one lock, one `Arc`
//! clone, four `Vec` copies, ~4.5 kB. `frame()` answers a REST frame before the
//! first analysis frame lands, so a screen has no empty state to draw.

pub mod frame;
pub mod palette;
pub mod settings;
pub mod tap;

#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use nz_viz::VizState;
use nz_viz::palette::{self as viz_palette, Palette};

use crate::core::{Core, CoreError};

use frame::{Frame, Shape};
use palette::PaletteState;
use settings::VizSettings;
use tap::{TapOpener, TapSource};

/// The thread's name: the diagnostic journal lists it (`Réglages → Assistance`),
/// and it is the same name the desktop's analysis thread has.
pub const ANALYSIS_THREAD: &str = "nz-analysis";
/// What one read takes from the tap: 10 ms at 48 kHz, stereo — `wiring/viz.rs`'s
/// own block, small enough that a 94 Hz analysis is never behind and large enough
/// that the thread wakes 100 times a second and not 3 000.
pub const BLOCK_FRAMES: usize = 480;
/// The rate the analyser is told the tap delivers (the engine's output rate; the
/// tap is post-DSP at the device's rate, 48 kHz on every path the phone has —
/// MB3a's `OutputPrefs`).
pub const SAMPLE_RATE: f32 = 48_000.0;
/// How long the thread sleeps when NO view is visible. Nothing is subscribed, so
/// this is the only cost of an idle visualiser: 20 wake-ups a second that read an
/// atomic and go back to sleep.
const HIDDEN_POLL: Duration = Duration::from_millis(50);
/// How long it sleeps when the tap answered nothing (a paused player): the
/// analysis is not behind, there is simply no audio to read.
const STARVED_POLL: Duration = Duration::from_millis(5);
/// The views that may ask for the analysis. `eq` is MB12's meter: the same
/// analysis feeds it, so it is a view here rather than a second thread.
pub const VIEWS: &[&str] = &["bars", "scope", "eq"];

/// What one `set_visible` did, for the diagnostic and the tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Visibility {
    /// Is anything visible now?
    pub live: bool,
    /// How many views are visible.
    pub views: usize,
}

/// The state the service and its thread share. The thread holds an `Arc` of
/// THIS and never one of [`Viz`], so `Viz`'s `Drop` can still join it.
struct Shared {
    settings: RwLock<VizSettings>,
    /// The sleeve, as the platform decoder handed it in: the parsed palette, or
    /// `None` for "no artwork".
    artwork: RwLock<Option<Palette>>,
    /// Bumped on every artwork change, so the thread applies one per change
    /// instead of re-reading the RGBA per frame.
    artwork_version: AtomicU64,
    views: Mutex<BTreeSet<String>>,
    shape: RwLock<Shape>,
    published: Mutex<Option<Arc<Frame>>>,
    live: AtomicBool,
    /// Analysis frames produced since the process started.
    ticks: AtomicU64,
    /// Frames published (one per analysis frame while a view is visible).
    frames: AtomicU64,
    /// Frames the tap dropped because this reader was behind, accumulated over
    /// subscriptions.
    dropped: AtomicU64,
    /// How many subscriptions have been opened: the "0 with no view" counter.
    subscribes: AtomicU64,
    /// Thread wake-ups: the EVENT the tests wait on instead of a clock.
    wakes: AtomicU64,
    stop: AtomicBool,
    opener: TapOpener,
    /// Wakes the thread when visibility, settings or the shape change, so "a
    /// view left" stops the analysis NOW and not at the next poll.
    gate: Mutex<()>,
    gate_wake: Condvar,
}

/// The service. One per process, built by `NzCore::viz()`.
pub struct Viz {
    core: Arc<Core>,
    shared: Arc<Shared>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl Viz {
    /// Builds the service over the core, with the tap the caller can open.
    ///
    /// The THREAD IS STARTED LAZILY, at the first `set_visible(true)`: a person
    /// who never opens a visualiser starts no thread at all, and a core built by
    /// a test that only reads settings costs nothing.
    pub fn new(core: Arc<Core>, opener: TapOpener) -> Arc<Viz> {
        let settings = VizSettings::read(&core);
        Arc::new(Viz {
            core,
            shared: Arc::new(Shared {
                settings: RwLock::new(settings),
                artwork: RwLock::new(None),
                artwork_version: AtomicU64::new(0),
                views: Mutex::new(BTreeSet::new()),
                shape: RwLock::new(Shape::default()),
                published: Mutex::new(None),
                live: AtomicBool::new(false),
                ticks: AtomicU64::new(0),
                frames: AtomicU64::new(0),
                dropped: AtomicU64::new(0),
                subscribes: AtomicU64::new(0),
                wakes: AtomicU64::new(0),
                stop: AtomicBool::new(false),
                opener,
                gate: Mutex::new(()),
                gate_wake: Condvar::new(),
            }),
            handle: Mutex::new(None),
        })
    }

    /// The core this service reads its settings from.
    pub fn core(&self) -> &Arc<Core> {
        &self.core
    }

    /// Turns the tap on or off for ONE view. Called from a view's start/dispose
    /// (never on a timer), so the analysis really does stop when the last view
    /// leaves.
    pub fn set_visible(&self, view: &str, visible: bool) -> Visibility {
        let views = {
            let mut held = self.shared.views.lock().unwrap_or_else(|e| e.into_inner());
            if visible {
                held.insert(view.to_owned());
            } else {
                held.remove(view);
            }
            held.len()
        };
        let settings = self.settings();
        let live = views > 0 && settings.draws();
        self.shared.live.store(live, Ordering::Release);
        if live {
            self.ensure_thread();
        } else {
            // Nothing draws: the frame goes with the view, so a re-opened screen
            // starts on a blank picture and not on a stale one.
            self.clear_frame();
        }
        self.wake_thread();
        Visibility { live, views }
    }

    /// The settings, resolved (one `RwLock` read, no disk: the store was read
    /// when the service was built and after every write).
    pub fn settings(&self) -> VizSettings {
        self.shared
            .settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Writes one `viz.*` key and applies it at once: the store is local, so
    /// there is nothing to roll back and the next frame already carries the new
    /// choice (the SPA's optimistic write, `stores.js#persisted`).
    pub fn set_setting(
        &self,
        key: &str,
        value: serde_json::Value,
    ) -> Result<VizSettings, CoreError> {
        let settings = VizSettings::write(&self.core, key, value)?;
        {
            let mut held = self
                .shared
                .settings
                .write()
                .unwrap_or_else(|e| e.into_inner());
            *held = settings.clone();
        }
        // Eco or a mode change moves what the thread should do: recompute the
        // gate and stop it at once rather than at the next poll.
        let views = self
            .shared
            .views
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len();
        let live = views > 0 && settings.draws();
        self.shared.live.store(live, Ordering::Release);
        if !live {
            self.clear_frame();
        }
        self.wake_thread();
        Ok(settings)
    }

    /// The typed writers the FFI uses. `nz-ffi` has no `serde_json` and may not
    /// add one to the shared manifest (MB16's rule), so the JSON value the store
    /// expects is built HERE, once per setter, from a value that cannot be
    /// malformed.
    pub fn set_mode(&self, mode: &str) -> Result<VizSettings, CoreError> {
        self.set_setting("viz.mode", serde_json::Value::from(mode))
    }

    pub fn set_palette(&self, palette: &str) -> Result<VizSettings, CoreError> {
        self.set_setting("viz.palette", serde_json::Value::from(palette))
    }

    pub fn set_intensity(&self, intensity: f64) -> Result<VizSettings, CoreError> {
        self.set_setting(
            "viz.intensity",
            serde_json::Value::from(intensity.clamp(0.0, 1.0)),
        )
    }

    pub fn set_fps(&self, fps: u32) -> Result<VizSettings, CoreError> {
        self.set_setting("viz.fps", serde_json::Value::from(fps))
    }

    pub fn set_eco(&self, eco: bool) -> Result<VizSettings, CoreError> {
        self.set_setting("viz.eco", serde_json::Value::Bool(eco))
    }

    pub fn set_beat(&self, on: bool) -> Result<VizSettings, CoreError> {
        self.set_setting("viz.beat", serde_json::Value::Bool(on))
    }

    pub fn set_scope_orientation(&self, orientation: &str) -> Result<VizSettings, CoreError> {
        self.set_setting(
            "viz.scope.orientation",
            serde_json::Value::from(orientation),
        )
    }

    pub fn set_scope_colour(&self, colour: &str) -> Result<VizSettings, CoreError> {
        self.set_setting("viz.scope.colour", serde_json::Value::from(colour))
    }

    /// The shape the surface draws: how many capsules, how many columns.
    pub fn set_shape(&self, bars: u32, cols: u32) {
        let shape = Shape {
            bars: bars as usize,
            cols: cols as usize,
        }
        .settled();
        {
            let mut held = self.shared.shape.write().unwrap_or_else(|e| e.into_inner());
            if *held == shape {
                return;
            }
            *held = shape;
        }
        self.wake_thread();
    }

    /// The sleeve, as RGBA pixels the platform already decoded (64×64 is what
    /// `coverpalette.js` reads). A zero size forgets the artwork.
    ///
    /// The decode is the PLATFORM's (`BitmapFactory`, in `:feature/viz`): the
    /// note's rule, "decoding in Rust and copying RGBA across JNI would cost more
    /// than the platform decoder". What Rust does with the bytes is
    /// `coverpalette.js#analyse` itself, through `nz-viz` — the same code the
    /// desktop draws its sleeves with, so a zone is the sleeve's colour on both.
    pub fn set_artwork_pixels(&self, rgba: Vec<u8>, width: u32, height: u32) {
        let (w, h) = (width as usize, height as usize);
        let palette = if w == 0 || h == 0 || rgba.is_empty() {
            None
        } else {
            viz_palette::analyse(&rgba, w, h)
        };
        {
            let mut held = self
                .shared
                .artwork
                .write()
                .unwrap_or_else(|e| e.into_inner());
            *held = palette;
        }
        self.shared.artwork_version.fetch_add(1, Ordering::Release);
        self.wake_thread();
    }

    /// Forgets the sleeve (a track with no artwork — the row goes back to its own
    /// low → high walk, never to the last track's colours).
    pub fn clear_artwork(&self) {
        self.set_artwork_pixels(Vec::new(), 0, 0);
    }

    /// The newest frame, or a REST frame when nothing has been analysed yet.
    pub fn frame(&self) -> Arc<Frame> {
        if let Some(frame) = self
            .shared
            .published
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            return frame;
        }
        let settings = self.settings();
        let shape = self.shape();
        let mut rest = frame::rest(shape);
        // The rest frame keeps the personal palette's hue, so an idle screen is
        // not a different colour from the one it becomes.
        rest.colours = PaletteState::new(settings.palette_source()).colours();
        Arc::new(rest)
    }

    /// Whether anything is visible (the diagnostic and the tests).
    pub fn is_live(&self) -> bool {
        self.shared.live.load(Ordering::Acquire)
    }

    /// Analysis frames produced since the process started — the acceptance's "0
    /// analysis tick while no view is visible".
    pub fn ticks(&self) -> u64 {
        self.shared.ticks.load(Ordering::Acquire)
    }

    /// Frames published.
    pub fn frames(&self) -> u64 {
        self.shared.frames.load(Ordering::Acquire)
    }

    /// Tap frames this reader lost (accumulated over subscriptions).
    pub fn dropped(&self) -> u64 {
        self.shared.dropped.load(Ordering::Acquire)
    }

    /// How many subscriptions the service has opened: 0 while nothing draws.
    pub fn subscribes(&self) -> u64 {
        self.shared.subscribes.load(Ordering::Acquire)
    }

    /// The thread's wake count: an EVENT, which is how the tests wait for "the
    /// thread has looked at the gate and gone back to sleep" without a clock.
    pub fn wakes(&self) -> u64 {
        self.shared.wakes.load(Ordering::Acquire)
    }

    /// The counted bytes this service holds (worker rule 4): the published
    /// frame and the analyser's own fixed buffers.
    pub fn memory_bytes(&self) -> u64 {
        let published = self
            .shared
            .published
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|frame| frame.bytes())
            .unwrap_or(0);
        // `nz-viz`'s fixed buffers, counted: the tap window (`Wave`: 2 × 4096
        // f32), the scope's two traces (2 × 4096 × 2 f32) and the bars' input,
        // grouped and output arrays (`MAX_BANDS` + `MAX_BARS` + 2 × `MAX_BARS`).
        let wave = (2 * nz_viz::WINDOW) as u64 * 4;
        let traces = (nz_viz::MAX_COLS * 2 * 2) as u64 * 4;
        let bars = (nz_viz::MAX_BARS * 4) as u64 * 4;
        published + wave + traces + bars
    }

    /// Joins the analysis thread. Idempotent; also called by `Drop`.
    pub fn shutdown(&self) {
        stop_and_join(&self.shared, &self.handle);
    }

    fn shape(&self) -> Shape {
        *self.shared.shape.read().unwrap_or_else(|e| e.into_inner())
    }

    fn clear_frame(&self) {
        *self
            .shared
            .published
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
    }

    fn wake_thread(&self) {
        let _guard = self.shared.gate.lock().unwrap_or_else(|e| e.into_inner());
        self.shared.gate_wake.notify_all();
    }

    /// Starts the analysis thread once (the first visible view). A thread that
    /// cannot be created is not a crash: the screen draws the rest frame and the
    /// cause is in the journal.
    fn ensure_thread(&self) {
        let mut slot = self.handle.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_some() || self.shared.stop.load(Ordering::Acquire) {
            return;
        }
        let worker = Arc::clone(&self.shared);
        match thread::Builder::new()
            .name(ANALYSIS_THREAD.to_owned())
            .spawn(move || run(worker))
        {
            Ok(handle) => *slot = Some(handle),
            Err(error) => tracing::warn!(%error, "the analysis thread could not start"),
        }
    }
}

impl Drop for Viz {
    fn drop(&mut self) {
        stop_and_join(&self.shared, &self.handle);
    }
}

/// Sets the stop flag, knocks on the gate and joins — one place, so `shutdown`
/// and `Drop` cannot drift.
fn stop_and_join(shared: &Shared, handle: &Mutex<Option<JoinHandle<()>>>) {
    shared.stop.store(true, Ordering::Release);
    {
        let _guard = shared.gate.lock().unwrap_or_else(|e| e.into_inner());
        shared.gate_wake.notify_all();
    }
    let taken = handle.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(handle) = taken {
        let _ = handle.join();
    }
}

/// The analysis thread: subscribe, read, analyse, publish — and nothing at all
/// while no view is visible.
fn run(shared: Arc<Shared>) {
    let mut state = VizState::new(SAMPLE_RATE);
    let mut palette: Option<PaletteState> = None;
    let mut palette_key: Option<String> = None;
    // `None` on purpose: a sleeve handed in BEFORE the thread started is
    // applied on the first pass, not skipped as "already seen".
    let mut seen_artwork: Option<u64> = None;
    let mut source: Option<Box<dyn TapSource>> = None;
    let mut buffer = vec![0.0f32; BLOCK_FRAMES * 2];

    loop {
        shared.wakes.fetch_add(1, Ordering::Release);
        if shared.stop.load(Ordering::Acquire) {
            break;
        }
        if !shared.live.load(Ordering::Acquire) {
            // No view: NOTHING is subscribed and nothing is analysed. The
            // subscription is dropped here, which is what makes "0 cost with no
            // viewer" true on the tap's own side too (MB12: Drop unsubscribes).
            if let Some(mut tap) = source.take() {
                let lost = tap.dropped();
                tap.clear();
                drop(tap);
                shared.dropped.fetch_add(lost, Ordering::AcqRel);
            }
            wait(&shared, HIDDEN_POLL);
            continue;
        }

        let settings = shared
            .settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        // The palette follows the setting; the sleeve follows the artwork. Both
        // are applied once per CHANGE, never per frame.
        if palette_key.as_deref() != Some(settings.palette.as_str()) {
            palette_key = Some(settings.palette.clone());
            let source = settings.palette_source();
            match &mut palette {
                Some(existing) => existing.set_source(source),
                None => palette = Some(PaletteState::new(source)),
            }
        }
        let version = shared.artwork_version.load(Ordering::Acquire);
        if seen_artwork != Some(version) {
            seen_artwork = Some(version);
            let held = shared.artwork.read().unwrap_or_else(|e| e.into_inner());
            if let Some(state) = palette.as_mut() {
                state.set_artwork(held.as_ref());
            }
        }

        if source.is_none() {
            source = (shared.opener)();
            if source.is_none() {
                // No engine has ever played: nothing to analyse yet. The screen
                // keeps its rest frame; this is not an error.
                wait(&shared, HIDDEN_POLL);
                continue;
            }
            shared.subscribes.fetch_add(1, Ordering::AcqRel);
        }
        let Some(tap) = source.as_mut() else {
            continue;
        };

        let got = tap.read(&mut buffer);
        if got == 0 {
            wait(&shared, STARVED_POLL);
            continue;
        }
        state.push_frames(&buffer[..got * 2]);
        let Some(palette) = palette.as_mut() else {
            continue;
        };
        let shape = *shared.shape.read().unwrap_or_else(|e| e.into_inner());
        // The block's own duration: `nz-viz` ACCUMULATES it and spends it only
        // when an analysis frame arrives, so the envelopes follow seconds even
        // though this loop reads 10 ms at a time (its own measured rule).
        let dt = got as f64 / SAMPLE_RATE as f64;
        let frame = frame::sample(&mut state, shape, palette, dt, settings.needs_wave());
        shared.ticks.fetch_add(1, Ordering::AcqRel);
        shared.frames.fetch_add(1, Ordering::AcqRel);
        *shared.published.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(frame));
    }

    // The thread owns the subscription: dropping it here is the unsubscribe.
    drop(source);
}

/// Waits on the gate for `at_most`, and returns at once when a caller knocks.
fn wait(shared: &Shared, at_most: Duration) {
    let guard = shared.gate.lock().unwrap_or_else(|e| e.into_inner());
    let _ = shared
        .gate_wake
        .wait_timeout(guard, at_most)
        .unwrap_or_else(|e| e.into_inner());
}
