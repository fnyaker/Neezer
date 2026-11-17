//! Android's output for `nz-audio`: AAudio on API >= 27, the `AudioTrack` pull
//! hook on API 26. Owned by **MB3a**.
//!
//! The contract is the note's (docs/agents/mobile.md, "MB3a"):
//!
//! ```text
//! DroidOutput::start(&Engine, ExternalSink, OutputPrefs, sdk_int) -> Result<Self, DroidOutputError>
//! DroidOutput::{info, underruns, performance_mode, stop}
//! ```
//!
//! **MB0 created the crate and nothing else.** There is no device, no callback
//! and no thread yet: `start` answers [`DroidOutputError::NeedsAudioTrack`] below
//! API 27 and [`DroidOutputError::Unavailable`] everywhere else, so the app
//! installs and runs with no audio path rather than with a silent one that looks
//! like a bug.
//!
//! What the lot must not get wrong (android.md's bridge 12 paid for it twice):
//!
//! - The data callback is real time: **no allocation, no lock, no syscall, no
//!   JNI** — `ExternalSink::render` is written for exactly that.
//! - The device is opened at `EngineConfig::sample_rate`; a different rate is
//!   reported through `set_external_output` and logged, never silently mixed at.
//! - `note_underruns` takes a TOTAL (AAudio's `getXRunCount` is a total since the
//!   device opened), so a repeated reading does not inflate it.
//! - The buffer is 500 ms–1 s on purpose: a music player wants few wake-ups, not
//!   low latency. `PowerSaving` may be silently refused — read it back.

/// What the app asks the platform for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputPrefs {
    /// What the voices are decoded at; the device MUST open at it.
    pub sample_rate: u32,
    pub buffer_ms_min: u32,
    pub buffer_ms_max: u32,
}

impl Default for OutputPrefs {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            // The old app's measured NativeSink lesson: 250 ms of margin is what
            // a normal-priority playback thread has to survive a GC, so the
            // phone asks for half a second and up to one.
            buffer_ms_min: 500,
            buffer_ms_max: 1_000,
        }
    }
}

/// Why the output could not open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DroidOutputError {
    /// API < 27: the caller must use the Kotlin `AudioTrack` pull (Oboe #40 —
    /// AAudio can crash after close on Android 8.0).
    NeedsAudioTrack,
    /// Nothing opens it yet (MB0), or the platform refused.
    Unavailable(String),
}

impl std::fmt::Display for DroidOutputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NeedsAudioTrack => f.write_str("l'API 26 passe par AudioTrack"),
            Self::Unavailable(why) => write!(f, "sortie audio indisponible : {why}"),
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

/// Which output this build would use on a device at `sdk_int`.
pub fn output_for(sdk_int: u32) -> Output {
    if sdk_int >= 27 {
        Output::Aa
    } else if sdk_int >= 26 {
        Output::Track
    } else {
        Output::Unsupported
    }
}

/// The output handle MB3a fills: it owns the AAudio stream and the thread that
/// reopens it after a route change.
#[derive(Debug)]
pub struct DroidOutput {
    prefs: OutputPrefs,
    sdk_int: u32,
}

impl DroidOutput {
    /// Opens the device and starts pulling from the engine's sink.
    ///
    /// MB0 opens nothing: this is the one place a later lot replaces with the
    /// real AAudio stream, and until then `NeedsAudioTrack`/`Unavailable` are
    /// the honest answers.
    pub fn start(prefs: OutputPrefs, sdk_int: u32) -> Result<Self, DroidOutputError> {
        // MB3a uses the preferences to size the stream; MB0 opens nothing.
        let _ = prefs;
        match output_for(sdk_int) {
            Output::Aa => {
                tracing::debug!("nz-droid-audio: AAudio requested (MB0: not opened)");
                Err(DroidOutputError::Unavailable(
                    "MB0: AAudio arrive avec MB3a".into(),
                ))
            }
            Output::Track => Err(DroidOutputError::NeedsAudioTrack),
            Output::Unsupported => {
                Err(DroidOutputError::Unavailable(format!("API {sdk_int} < 26")))
            }
        }
    }

    pub fn prefs(&self) -> OutputPrefs {
        self.prefs
    }

    /// The buffer and rate the device really opened (MB3a reads them back from
    /// AAudio, never from the request).
    pub fn info(&self) -> (u32, u32) {
        (self.prefs.sample_rate, 0)
    }

    /// AAudio's `getXRunCount` — a TOTAL since the device opened.
    pub fn underruns(&self) -> u64 {
        0
    }

    /// What AAudio actually granted (`PowerSaving`, or what it fell back to).
    pub fn performance_mode(&self) -> &'static str {
        "none"
    }

    /// Closes the stream and JOINS the reopen thread (a thread that borrows a
    /// runtime is joined before it stops — worker rule 5).
    pub fn stop(self) {
        let _ = self.sdk_int;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The split is the plan's: AAudio from 27, the AudioTrack hook on 26, and
    /// nothing below the app's minSdk.
    #[test]
    fn the_output_split_is_the_planned_one() {
        assert_eq!(output_for(35), Output::Aa);
        assert_eq!(output_for(27), Output::Aa);
        assert_eq!(output_for(26), Output::Track);
        assert_eq!(output_for(25), Output::Unsupported);

        let prefs = OutputPrefs::default();
        assert_eq!(prefs.buffer_ms_min, 500);
        assert_eq!(prefs.buffer_ms_max, 1_000);
        assert_eq!(
            DroidOutput::start(prefs, 26).unwrap_err(),
            DroidOutputError::NeedsAudioTrack
        );
        assert!(matches!(
            DroidOutput::start(prefs, 35),
            Err(DroidOutputError::Unavailable(_))
        ));
    }
}
