//! The app-facing output: what to ask for, what the device gave, and the two
//! decisions the note pins (AAudio from API 27, the Kotlin pull on 26).

use nz_audio::{Engine, ExternalSink, OutputInfo};

#[cfg(target_os = "android")]
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

#[cfg(target_os = "android")]
use crate::aa::Runner;

/// What the app asks the platform for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputPrefs {
    /// What the voices are decoded at; the device MUST open at it.
    pub sample_rate: u32,
    /// The margin a normal-priority playback thread needs (bridge 12).
    pub buffer_ms_min: u32,
    /// Past this the only cost is the audio already written when a pause or a
    /// seek flushes the device, which is silent.
    pub buffer_ms_max: u32,
}

impl Default for OutputPrefs {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            // The old app's measured NativeSink lesson: 250 ms of margin is what
            // a normal-priority playback thread has to survive a GC or a decoder
            // burst, so the phone asks for half a second and up to one.
            buffer_ms_min: 500,
            buffer_ms_max: 1_000,
        }
    }
}

/// Why the output could not open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DroidOutputError {
    /// API < 27: the caller must use [`crate::PullSession`] with the Kotlin
    /// `AudioTrack` pull instead (AAudio can crash after close on Android 8.0,
    /// Oboe #40). Ask [`output_for`] FIRST: `start` consumes the sink, and this
    /// answer means "your sink is not mine to keep".
    NeedsAudioTrack,
    /// The platform refused, or this build has no device path at all.
    Open(String),
}

impl std::fmt::Display for DroidOutputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NeedsAudioTrack => {
                f.write_str("l'API 26 passe par AudioTrack (AudioTrackPull), pas par AAudio")
            }
            Self::Open(why) => write!(f, "sortie audio indisponible : {why}"),
        }
    }
}

impl std::error::Error for DroidOutputError {}

/// Where the output stands on this build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    /// AAudio (`ndk::audio`), the normal path on API >= 27.
    Aa,
    /// The API-26 `AudioTrack` pull, driven by one Kotlin thread.
    Track,
    /// Below the app's minSdk.
    Unsupported,
}

/// Which output this build uses on a device at `sdk_int`. The ONE place the
/// split is decided, so a caller cannot pick AAudio on 8.0 by accident.
pub fn output_for(sdk_int: u32) -> Output {
    if sdk_int >= 27 {
        Output::Aa
    } else if sdk_int >= 26 {
        Output::Track
    } else {
        Output::Unsupported
    }
}

/// What AAudio granted, in this crate's own words — the same three names on a
/// host build, on the API-26 path and in the journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerformanceMode {
    PowerSaving,
    LowLatency,
    /// `AAUDIO_PERFORMANCE_MODE_NONE`: AAudio silently refused `PowerSaving`
    /// (it is a request, not a guarantee — read it back, always).
    None,
    Unknown,
}

impl PerformanceMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::PowerSaving => "power-saving",
            Self::LowLatency => "low-latency",
            Self::None => "none",
            Self::Unknown => "inconnu",
        }
    }
}

/// The frame count to ask AAudio for: **the largest value within 500–1000 ms
/// that its capacity allows**, or the capacity itself when even that is shorter.
///
/// AAudio's `bufferCapacityInFrames` is the ceiling of what shared mode will
/// give (a device with a 192-frame burst may cap at a few thousand frames), so
/// asking for more than it is a silent no-op; asking for LESS than 500 ms when
/// the device could do 500 ms is the bug bridge 12 found (Media3's 250 ms floor
/// starved on a normal-priority thread).
pub fn buffer_frames(prefs: OutputPrefs, capacity_frames: i32) -> i32 {
    if capacity_frames <= 0 {
        return 0;
    }
    let capacity = i64::from(capacity_frames);
    let min = crate::frames_for_ms(prefs.sample_rate, prefs.buffer_ms_min).min(capacity);
    let max = crate::frames_for_ms(prefs.sample_rate, prefs.buffer_ms_max);
    (capacity.min(max).max(min)).clamp(0, capacity) as i32
}

/// What the output thread publishes, and what the app reads. One mutex, taken
/// only by `start`, a reopen and a getter — never by a device callback.
#[derive(Debug, Clone)]
pub(crate) struct Published {
    pub info: OutputInfo,
    pub performance: PerformanceMode,
    /// What `setBufferSizeInFrames` really set.
    pub buffer_frames: i32,
    /// What the device can hold at most.
    pub capacity_frames: i32,
    /// AAudio's own callback size (`framesPerBurst`).
    pub burst_frames: i32,
    /// The device's xruns SINCE THIS OUTPUT STARTED, accumulated across streams
    /// (a stream's `getXRunCount` restarts at 0, the number must not).
    pub xruns: u64,
    /// The latency the engine's clock currently subtracts (so the supervision
    /// tick can tell a real drift from noise).
    #[cfg(target_os = "android")]
    pub pushed_latency: u64,
    /// Streams opened on this output: 1 after `start`, +1 per `reopen`.
    pub opens: u64,
    /// Opens that failed (a reopen that could not find a device).
    pub failures: u64,
    pub last_failure: Option<String>,
}

impl Published {
    pub fn new(prefs: OutputPrefs) -> Self {
        Self {
            info: OutputInfo {
                device: "AAudio".to_string(),
                sample_rate: prefs.sample_rate,
                channels: crate::CHANNELS as u16,
                latency_frames: 0,
            },
            performance: PerformanceMode::Unknown,
            buffer_frames: 0,
            capacity_frames: 0,
            burst_frames: 0,
            xruns: 0,
            #[cfg(target_os = "android")]
            pushed_latency: 0,
            opens: 0,
            failures: 0,
            last_failure: None,
        }
    }

    /// What a host build (which never opens anything) answers.
    #[cfg(not(target_os = "android"))]
    pub fn absent(prefs: OutputPrefs) -> Self {
        Self {
            info: OutputInfo {
                device: "aucune sortie (cible hôte)".to_string(),
                sample_rate: prefs.sample_rate,
                channels: crate::CHANNELS as u16,
                latency_frames: 0,
            },
            performance: PerformanceMode::None,
            ..Self::new(prefs)
        }
    }

    pub fn buffer_ms(&self) -> u32 {
        if self.info.sample_rate == 0 {
            return 0;
        }
        (u64::from(self.buffer_frames.max(0) as u32) * 1_000 / u64::from(self.info.sample_rate))
            as u32
    }
}

/// The published state and the condvar a `reopen` waits on: both are events
/// (a stream opened, an open failed), never a poll of the clock.
#[cfg(target_os = "android")]
pub(crate) type Shared = Arc<(Mutex<Published>, Condvar)>;

#[cfg(target_os = "android")]
pub(crate) fn shared(prefs: OutputPrefs) -> Shared {
    Arc::new((Mutex::new(Published::new(prefs)), Condvar::new()))
}

#[cfg(target_os = "android")]
pub(crate) fn published(shared: &Shared) -> MutexGuard<'_, Published> {
    shared
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The AAudio stream and the thread that re-opens it after a route change.
///
/// **`start` returns once the device is really open and playing**, so a failure
/// is the caller's error and not a silent output later; a caller on API 26 gets
/// [`DroidOutputError::NeedsAudioTrack`] instead (see [`output_for`]).
pub struct DroidOutput {
    prefs: OutputPrefs,
    #[cfg(target_os = "android")]
    runner: Runner,
}

impl DroidOutput {
    /// Open the device and start pulling from the engine's sink.
    ///
    /// `sdk_int` is `Build.VERSION.SDK_INT`: below 27 the answer is
    /// [`DroidOutputError::NeedsAudioTrack`] and the caller keeps its sink for
    /// the `AudioTrack` pull (ask [`output_for`] before calling this).
    pub fn start(
        engine: &Engine,
        sink: ExternalSink,
        prefs: OutputPrefs,
        sdk_int: u32,
    ) -> Result<Self, DroidOutputError> {
        match output_for(sdk_int) {
            Output::Track => Err(DroidOutputError::NeedsAudioTrack),
            Output::Unsupported => Err(DroidOutputError::Open(format!(
                "API {sdk_int} est sous le minSdk 26 : aucune sortie"
            ))),
            Output::Aa => {
                #[cfg(target_os = "android")]
                {
                    let runner = Runner::start(engine, sink, prefs)?;
                    Ok(Self { prefs, runner })
                }
                #[cfg(not(target_os = "android"))]
                {
                    let _ = (engine, sink, prefs);
                    Err(DroidOutputError::Open(
                        "AAudio n'existe que sur Android (cible hôte)".to_string(),
                    ))
                }
            }
        }
    }

    /// What this output was asked for (the app shows it in Réglages next to what
    /// the device gave).
    pub fn prefs(&self) -> OutputPrefs {
        self.prefs
    }

    /// The buffer and rate the device really opened — never the request.
    pub fn info(&self) -> OutputInfo {
        self.published().info
    }

    /// The device's xruns since this output started plus the mixer's own
    /// starvations, which is the number `Réglages` shows.
    pub fn underruns(&self) -> u64 {
        let published = self.published();
        published.xruns
    }

    /// What AAudio actually granted (`PowerSaving` may be refused silently).
    pub fn performance_mode(&self) -> &'static str {
        self.published().performance.name()
    }

    /// The buffer AAudio really set, in milliseconds — the number in the PR.
    pub fn buffer_ms(&self) -> u32 {
        self.published().buffer_ms()
    }

    /// The buffer AAudio really set, in frames.
    pub fn buffer_frames(&self) -> i32 {
        self.published().buffer_frames
    }

    /// What the device can hold at most (`bufferCapacityInFrames`): the ceiling
    /// the 500–1000 ms request is clamped to.
    pub fn capacity_frames(&self) -> i32 {
        self.published().capacity_frames
    }

    /// AAudio's own callback size (`framesPerBurst`): how many frames one
    /// wake-up of the real-time thread renders.
    pub fn burst_frames(&self) -> i32 {
        self.published().burst_frames
    }

    /// How many re-opens failed (a device that vanished, a HAL that refused).
    pub fn failures(&self) -> u64 {
        self.published().failures
    }

    /// The last open failure, worded for the person — `None` while it works.
    pub fn last_failure(&self) -> Option<String> {
        self.published().last_failure
    }

    /// How many streams this output has opened (1 after `start`): what a route
    /// change test waits on.
    pub fn opens(&self) -> u64 {
        self.published().opens
    }

    /// Close the stream and open a new one on the current device — a headset
    /// plugged in, a Bluetooth sink gone. **Returns once the new stream is
    /// playing** (or the open failed), so a caller never guesses.
    ///
    /// The mixer is not recreated: `Pull` gives the sink back when the old
    /// stream is dropped and the new stream takes it, so the position continues
    /// with no hole and no repeated frame.
    pub fn reopen(&self) -> Result<OutputInfo, String> {
        #[cfg(target_os = "android")]
        {
            self.runner.reopen()
        }
        #[cfg(not(target_os = "android"))]
        {
            Err("AAudio n'existe que sur Android (cible hôte)".to_string())
        }
    }

    /// Close the stream and JOIN the reopen thread (a thread that borrows the
    /// engine is joined before the engine stops — worker rule 5).
    pub fn stop(self) {
        #[cfg(target_os = "android")]
        self.runner.stop();
    }

    #[cfg(target_os = "android")]
    fn published(&self) -> Published {
        self.runner.published()
    }

    #[cfg(not(target_os = "android"))]
    fn published(&self) -> Published {
        Published::absent(self.prefs)
    }
}

impl std::fmt::Debug for DroidOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let published = self.published();
        f.debug_struct("DroidOutput")
            .field("device", &published.info.device)
            .field("sample_rate", &published.info.sample_rate)
            .field("performance", &published.performance)
            .field("buffer_frames", &published.buffer_frames)
            .field("opens", &published.opens)
            .field("xruns", &published.xruns)
            .finish()
    }
}
