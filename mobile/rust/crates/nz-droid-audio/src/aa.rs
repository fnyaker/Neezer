//! AAudio, on API >= 27: one stream, one owner thread, one mixer across every
//! route change.
//!
//! AAudio's contract decides the shape of this file:
//!
//! - the **data callback** runs on a real-time thread and must not allocate,
//!   lock, wait or stop the stream — it is [`crate::Pull::fill`] and nothing
//!   else;
//! - the **error callback** must not close or reopen either: it raises a flag
//!   and wakes us;
//! - closing and re-opening therefore happens on **one thread** (`nz-out-reopen`,
//!   [The threads table](https://github.com/fnyaker/Neezer/blob/master/docs/agents/mobile.md)),
//!   which also owns the stream because `ndk`'s `AudioStream` is `!Send`.
//!
//! What makes a reopen seamless is [`crate::Handback`]: the stream's `Pull` gives
//! the sink back when the stream is dropped, and the next stream takes it. The
//! mixer, its decoded ring and its clock are the same objects, so the position
//! continues — a hole would be a new sink and a repeat would be two.

use std::ffi::c_void;
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ndk::audio::{
    AudioCallbackResult, AudioDirection, AudioError, AudioFormat, AudioPerformanceMode,
    AudioSharingMode, AudioStream, AudioStreamBuilder, AudioStreamState, Clockid,
};
use nz_audio::{Engine, ExternalSink, OutputInfo};

use crate::CHANNELS;
use crate::output::{self, DroidOutputError, OutputPrefs, PerformanceMode, Published, Shared};
use crate::pull::{Handback, Pull};

/// How often the thread wakes while a stream is playing: it refreshes the
/// device's xrun total and its measured latency, and it catches a stream
/// AAudio marked `Disconnected` without calling the error callback. Once a
/// second on a thread that owns a half-second buffer is nothing next to the
/// device's own wake-ups, and it is the only way to read a counter that lives on
/// the stream.
const TICK: Duration = Duration::from_secs(1);

/// The output state and the thread that owns the AAudio stream.
pub(crate) struct Runner {
    ctl: Arc<Ctl>,
    shared: Shared,
    thread: Option<JoinHandle<()>>,
}

impl Runner {
    /// Open the device, and return only once it is `Started` — so a caller
    /// learns about a refusal here, not from silence later.
    pub fn start(
        engine: &Engine,
        sink: ExternalSink,
        prefs: OutputPrefs,
    ) -> Result<Self, DroidOutputError> {
        let back = Handback::new(sink);
        let ctl = Arc::new(Ctl::default());
        let shared = output::shared(prefs);
        let (ready, first) = mpsc::channel();
        let handle = {
            let engine = engine.clone();
            let back = Arc::clone(&back);
            let ctl = Arc::clone(&ctl);
            let shared = Arc::clone(&shared);
            thread::Builder::new()
                .name("nz-out-reopen".to_string())
                .spawn(move || run(engine, back, prefs, ctl, shared, ready))
                .map_err(|e| DroidOutputError::Open(format!("fil nz-out-reopen : {e}")))?
        };
        // An event, not a clock: the thread sends exactly one answer to the
        // first open before it does anything else.
        match first.recv() {
            Ok(Ok(info)) => {
                tracing::info!(
                    target: "nz_droid_audio",
                    rate = info.sample_rate,
                    latency_frames = info.latency_frames,
                    "sortie AAudio ouverte"
                );
                Ok(Self {
                    ctl,
                    shared,
                    thread: Some(handle),
                })
            }
            Ok(Err(why)) => {
                let _ = handle.join();
                Err(DroidOutputError::Open(why))
            }
            Err(_) => {
                let _ = handle.join();
                Err(DroidOutputError::Open(
                    "le fil nz-out-reopen s'est arrêté avant d'ouvrir".to_string(),
                ))
            }
        }
    }

    pub fn published(&self) -> Published {
        output::published(&self.shared).clone()
    }

    /// Close the current stream and open the next one, then return what the new
    /// device reports. Waits on the published state (a stream opened / an open
    /// failed), with a safety bound so a wedged device cannot hang the caller.
    pub fn reopen(&self) -> Result<OutputInfo, String> {
        let before = {
            let published = output::published(&self.shared);
            (published.opens, published.failures)
        };
        self.ctl.ask_reopen("changement de sortie");
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut published = output::published(&self.shared);
        loop {
            if published.opens > before.0 {
                return Ok(published.info.clone());
            }
            if published.failures > before.1 {
                return Err(published
                    .last_failure
                    .clone()
                    .unwrap_or_else(|| "la sortie n'a pas pu être rouverte".to_string()));
            }
            let (next, _) = self
                .shared
                .1
                .wait_timeout(published, Duration::from_millis(250))
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            published = next;
            if Instant::now() >= deadline {
                return Err("la sortie n'a pas répondu en 5 s".to_string());
            }
        }
    }

    /// Ask the thread to close the stream and stop, then JOIN it.
    pub fn stop(mut self) {
        self.ctl.ask_stop();
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::warn!(target: "nz_droid_audio", "le fil nz-out-reopen a paniqué");
        }
    }
}

/// What a `wait` came back with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wake {
    Stop,
    Reopen,
    /// The supervision tick (or a spurious wake-up): refresh and keep playing.
    Tick,
}

/// Why the output is being asked to close and reopen.
#[derive(Default)]
struct CtlState {
    stop: bool,
    /// Bumped by the error callback and by `DroidOutput::reopen`.
    reopen: u64,
    reason: Option<String>,
}

/// The ask-and-wake between the app, AAudio's error callback and the thread.
///
/// A `Mutex`+`Condvar` here is the "signal another thread" AAudio itself
/// prescribes for its error callback; the DATA callback never comes near it.
struct Ctl {
    state: Mutex<CtlState>,
    wake: Condvar,
}

impl Default for Ctl {
    fn default() -> Self {
        Self {
            state: Mutex::new(CtlState::default()),
            wake: Condvar::new(),
        }
    }
}

impl Ctl {
    fn lock(&self) -> MutexGuard<'_, CtlState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn ask_stop(&self) {
        self.lock().stop = true;
        self.wake.notify_all();
    }

    fn ask_reopen(&self, reason: impl Into<String>) {
        let mut state = self.lock();
        state.reopen += 1;
        state.reason = Some(reason.into());
        drop(state);
        self.wake.notify_all();
    }

    /// Wait for an ask. `limit = None` waits for ever (an idle output with no
    /// stream to supervise); `Some(TICK)` returns `Wake::Tick` on the timeout.
    fn wait(&self, seen: u64, limit: Option<Duration>) -> (Wake, u64) {
        let guard = self.lock();
        let state = match limit {
            Some(limit) => {
                self.wake
                    .wait_timeout(guard, limit)
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .0
            }
            None => self
                .wake
                .wait(guard)
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        };
        let generation = state.reopen;
        let wake = if state.stop {
            Wake::Stop
        } else if generation > seen {
            Wake::Reopen
        } else {
            Wake::Tick
        };
        (wake, generation)
    }
}

/// The thread: open, supervise, close, and do it again only if asked.
fn run(
    engine: Engine,
    back: Arc<Handback>,
    prefs: OutputPrefs,
    ctl: Arc<Ctl>,
    shared: Shared,
    ready: mpsc::Sender<Result<OutputInfo, String>>,
) {
    let mut ready = Some(ready);
    let mut seen = 0u64;
    // The device's xrun count restarts with every stream; `underruns()` must not.
    let mut xrun_base = 0u64;

    loop {
        if ctl.lock().stop {
            return;
        }
        match open(&engine, &back, prefs, &ctl, xrun_base, &shared) {
            Ok(stream) => {
                let info = output::published(&shared).info.clone();
                if let Some(ready) = ready.take() {
                    let _ = ready.send(Ok(info));
                }
                let mut stop = false;
                loop {
                    let (wake, generation) = ctl.wait(seen, Some(TICK));
                    seen = generation;
                    let xruns = stream.x_run_count().max(0) as u64;
                    supervise(&engine, &stream, xrun_base + xruns, &shared);
                    match wake {
                        Wake::Stop => {
                            stop = true;
                            break;
                        }
                        Wake::Reopen => break,
                        Wake::Tick => {
                            // A device that went away without an error callback
                            // (some HALs only flip the state).
                            if matches!(
                                stream.state(),
                                AudioStreamState::Disconnected
                                    | AudioStreamState::Closed
                                    | AudioStreamState::Stopping
                                    | AudioStreamState::Stopped
                            ) {
                                tracing::warn!(
                                    target: "nz_droid_audio",
                                    "flux AAudio hors service, réouverture"
                                );
                                break;
                            }
                        }
                    }
                }
                xrun_base += close(stream);
                if stop {
                    return;
                }
            }
            Err(why) => {
                tracing::warn!(target: "nz_droid_audio", why = %why, "AAudio n'a pas pu ouvrir");
                let mut published = output::published(&shared);
                published.failures += 1;
                published.last_failure = Some(why.clone());
                drop(published);
                shared.1.notify_all();
                if let Some(ready) = ready.take() {
                    let _ = ready.send(Err(why));
                    return;
                }
                // Nothing is playing. Wait for the app to ask again (a headset
                // plugged in) instead of spinning on a dead device.
                let (wake, generation) = ctl.wait(seen, None);
                seen = generation;
                if wake == Wake::Stop {
                    return;
                }
            }
        }
    }
}

/// Open one stream, measure it, start it, and tell the engine what it got.
fn open(
    engine: &Engine,
    back: &Arc<Handback>,
    prefs: OutputPrefs,
    ctl: &Arc<Ctl>,
    xrun_base: u64,
    shared: &Shared,
) -> Result<AudioStream, String> {
    let mut pull = Pull::attach(back);
    if !pull.attached() {
        return Err("le mixeur est encore tenu par le flux précédent".to_string());
    }
    let error_ctl = Arc::clone(ctl);
    let builder = AudioStreamBuilder::new()
        .map_err(|e| format!("constructeur AAudio : {}", e.to_text()))?
        .direction(AudioDirection::Output)
        .sharing_mode(AudioSharingMode::Shared)
        .performance_mode(AudioPerformanceMode::PowerSaving)
        .format(AudioFormat::PCM_Float)
        .channel_count(CHANNELS)
        .sample_rate(prefs.sample_rate as i32)
        // Ask for the capacity the app WANTS (500 ms), not the platform's default:
        // in shared mode AAudio's default capacity on a normal-priority stream
        // measured **4 320 frames = 90 ms** on the emulator, far under the margin
        // bridge 12 requires. The read-back below is what decides the actual size.
        .buffer_capacity_in_frames(
            crate::frames_for_ms(prefs.sample_rate, prefs.buffer_ms_min) as i32
        )
        // Usage and content type are NOT set: AAudio's defaults are
        // AAUDIO_USAGE_MEDIA and AAUDIO_CONTENT_TYPE_MUSIC (aaudio/AAudio.h, the
        // two setters' own documentation), and the setters only exist from API
        // 28 — calling one would make `libaaudio` fail to resolve the symbol on
        // an API 27 phone, for a value that is already right.
        .data_callback(Box::new(
            move |_stream: &AudioStream, audio_data: *mut c_void, frames: i32| {
                if frames > 0 && !audio_data.is_null() {
                    let samples = frames as usize * CHANNELS as usize;
                    // SAFETY: AAudio hands the data callback a buffer of `frames`
                    // frames, `CHANNELS` f32 samples each, valid for this call.
                    let out = unsafe {
                        std::slice::from_raw_parts_mut(audio_data.cast::<f32>(), samples)
                    };
                    pull.fill(out);
                }
                AudioCallbackResult::Continue
            },
        ))
        .error_callback(Box::new(move |_stream: &AudioStream, error: AudioError| {
            // AAudio's own documentation: do not close or reopen from here.
            // Signal the thread that can.
            tracing::warn!(
                target: "nz_droid_audio",
                error = %error.to_text(),
                "AAudio a signalé une erreur, réouverture demandée"
            );
            error_ctl.ask_reopen(error.to_text().into_owned());
        }));

    let stream = builder
        .open_stream()
        .map_err(|e| format!("ouverture AAudio : {}", e.to_text()))?;

    let channels = stream.channel_count();
    if channels != CHANNELS {
        return Err(format!(
            "AAudio a ouvert {channels} canaux, {CHANNELS} demandés"
        ));
    }
    let rate = stream.sample_rate();
    if rate != prefs.sample_rate as i32 {
        // Shared mode resamples inside the framework, so this is a report, not a
        // refusal — the mix stays at the engine's rate (SC2's contract).
        tracing::warn!(
            target: "nz_droid_audio",
            opened = rate,
            engine = prefs.sample_rate,
            "AAudio n'a pas ouvert au taux du moteur"
        );
    }
    let capacity = stream.buffer_capacity_in_frames();
    let want = crate::output::buffer_frames(prefs, capacity);
    let buffer = stream
        .set_buffer_size_in_frames(want)
        .unwrap_or_else(|_| stream.buffer_size_in_frames());
    stream
        .request_start()
        .map_err(|e| format!("démarrage AAudio : {}", e.to_text()))?;
    wait_started(&stream)?;

    let mode = performance_of(stream.performance_mode());
    let latency = latency_frames(&stream, buffer);
    let info = OutputInfo {
        device: "AAudio (sortie système)".to_string(),
        sample_rate: rate.max(0) as u32,
        channels: channels.max(0) as u16,
        latency_frames: latency,
    };
    engine.set_external_output(info.clone());
    engine.note_underruns(xrun_base + stream.x_run_count().max(0) as u64);

    let mut published = output::published(shared);
    published.info = info;
    published.performance = mode;
    published.buffer_frames = buffer;
    published.capacity_frames = capacity;
    published.burst_frames = stream.frames_per_burst();
    published.xruns = xrun_base;
    published.opens += 1;
    published.pushed_latency = latency;
    drop(published);
    shared.1.notify_all();

    tracing::info!(
        target: "nz_droid_audio",
        rate,
        channel = channels,
        capacity,
        buffer,
        buffer_ms = buffer.max(0) as u64 * 1_000 / u64::from(rate.max(1) as u32),
        burst = stream.frames_per_burst(),
        mode = mode.name(),
        latency,
        "AAudio : tampon réglé"
    );
    Ok(stream)
}

/// The supervision wake-up: the device's xrun total, and the latency it is
/// really playing at (an AAudio buffer can be resized under us).
fn supervise(engine: &Engine, stream: &AudioStream, xruns: u64, shared: &Shared) {
    let mut published = output::published(shared);
    let buffer = published.buffer_frames;
    let latency = latency_frames(stream, buffer);
    published.xruns = xruns;
    published.info.latency_frames = latency;
    let burst = i64::from(published.burst_frames.max(1));
    let drift = (i64::try_from(latency).unwrap_or(i64::MAX)
        - i64::try_from(published.pushed_latency).unwrap_or(0))
    .abs();
    let push = drift > burst;
    if push {
        published.pushed_latency = latency;
    }
    let info = published.info.clone();
    drop(published);
    engine.note_underruns(xruns);
    if push {
        // The clock subtracts this: keep it honest when the device resized.
        engine.set_external_output(info);
    }
}

/// Close a stream the way AAudio asks: request a stop, let it reach one, then
/// drop it (its `Pull` hands the sink back on the way out). Returns the xruns
/// this stream accumulated.
fn close(stream: AudioStream) -> u64 {
    let xruns = stream.x_run_count().max(0) as u64;
    let _ = stream.request_stop();
    let mut state = stream.state();
    for _ in 0..4 {
        if matches!(
            state,
            AudioStreamState::Stopped | AudioStreamState::Closed | AudioStreamState::Disconnected
        ) {
            break;
        }
        state = match stream.wait_for_state_change(state, 500_000_000) {
            Ok(next) => next,
            Err(_) => break,
        };
    }
    drop(stream);
    xruns
}

/// Wait until the stream is `Started`, on the state machine's own events.
fn wait_started(stream: &AudioStream) -> Result<(), String> {
    let mut state = stream.state();
    for _ in 0..8 {
        match state {
            AudioStreamState::Started => return Ok(()),
            AudioStreamState::Uninitialized
            | AudioStreamState::Closed
            | AudioStreamState::Closing
            | AudioStreamState::Disconnected
            | AudioStreamState::Stopped => {
                return Err(format!("AAudio s'est arrêté ({})", state.to_text()));
            }
            _ => {}
        }
        state = stream
            .wait_for_state_change(state, 2_000_000_000)
            .map_err(|e| format!("attente du démarrage AAudio : {}", e.to_text()))?;
    }
    Err("AAudio n'a pas démarré en 16 s".to_string())
}

/// Frames that are written and not yet heard, from AAudio's own timestamp: the
/// number the engine's clock subtracts. Before the first callback has played
/// anything there is no timestamp, and the buffer we set is the honest answer.
fn latency_frames(stream: &AudioStream, buffer: i32) -> u64 {
    let written = stream.frames_written();
    match stream.timestamp(Clockid::Monotonic) {
        Ok(ts) if ts.frame_position > 0 && written >= ts.frame_position => {
            (written - ts.frame_position).clamp(0, i64::from(stream.buffer_capacity_in_frames()))
                as u64
        }
        _ => u64::from(buffer.max(0) as u32),
    }
}

fn performance_of(mode: AudioPerformanceMode) -> PerformanceMode {
    match mode {
        AudioPerformanceMode::PowerSaving => PerformanceMode::PowerSaving,
        AudioPerformanceMode::LowLatency => PerformanceMode::LowLatency,
        AudioPerformanceMode::None => PerformanceMode::None,
        _ => PerformanceMode::Unknown,
    }
}
