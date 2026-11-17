//! Android's output for `nz-audio`: AAudio on API >= 27, the `AudioTrack` pull
//! hook on API 26. Owned by **MB3a** (docs/agents/mobile.md, "MB3a").
//!
//! ```text
//! // API >= 27 — the platform's own real-time thread calls Pull::fill
//! let (engine, sink) = Engine::new_external(cfg, opener);
//! let out = DroidOutput::start(&engine, sink, OutputPrefs::default(), sdk_int)?;
//! out.info(); out.underruns(); out.performance_mode(); out.reopen(); out.stop();
//!
//! // API 26 — one Kotlin thread pulls, `AudioTrackPull.kt` drives these natives
//! let session = PullSession::new(&engine, sink, OutputPrefs::default());
//! let handle = session.into_handle();          // what Kotlin stores as a long
//! ```
//!
//! Why the split, and why it is not a detail (android.md, *bridge 12*): AAudio
//! exists from API 26 but can crash after `close` on Android 8.0 (Oboe #40), so
//! 8.0 gets an `AudioTrack` the Kotlin side writes to, and 8.1+ gets AAudio.
//!
//! Four rules, each paid for by the old app's "the sound cuts every few
//! seconds" (bridge 12):
//!
//! - **The data callback is real time.** [`Pull::fill`] allocates nothing, locks
//!   nothing, makes no syscall and calls no JNI — `ExternalSink::render` is
//!   written for exactly that, and the host test arms a counting allocator
//!   around it (`tests/callback.rs`).
//! - **The buffer is 500 ms–1 s, not low latency.** A music player wants few
//!   wake-ups: 250 ms of margin is what a normal-priority playback thread has to
//!   survive a GC or a decoder burst, and that was the old sink's measured
//!   mistake (Media3 clamped it to a 250 ms floor).
//! - **A route change is handled OUTSIDE the callback.** AAudio forbids closing
//!   or reopening a stream from its own callback, so the error callback only
//!   raises a flag and the `nz-out-reopen` thread closes and re-opens, moving
//!   the SAME sink into the new stream: the engine's clock and its ring of
//!   decoded samples continue, so there is no hole and no repeated frame.
//! - **The device's own errors are totals, not deltas** (AAudio's
//!   `getXRunCount`): they are accumulated across re-opens so `underruns()` is
//!   monotonic, exactly as the engine's `note_underruns` expects.
//!
//! Nothing here needs a device to build: on a host target `start` answers an
//! error and the callback body is still the one the tests drive.

#[cfg(target_os = "android")]
mod aa;
#[cfg(target_os = "android")]
mod jni;
mod output;
mod pull;
mod track;

#[cfg(target_os = "android")]
pub use jni::{JNI_ENTRY_POINTS, JniEntryPoints};
pub use output::{
    DroidOutput, DroidOutputError, Output, OutputPrefs, PerformanceMode, buffer_frames, output_for,
};
pub use pull::{Handback, Pull};
pub use track::{PullSession, close_handle, session_mut, shared_of};

/// The interleaved sample format of every callback here: stereo f32, at the
/// engine's rate. AAudio opens `PCM_Float`, and `AudioTrack` opens
/// `ENCODING_PCM_FLOAT`.
pub const CHANNELS: i32 = 2;

/// The size of one interleaved frame in bytes (`AudioTrack`'s buffer maths).
pub const BYTES_PER_FRAME: i32 = CHANNELS * 4;

/// `ms` of audio at `rate`, rounded up: a buffer must never be shorter than the
/// margin it was asked for.
pub fn frames_for_ms(rate: u32, ms: u32) -> i64 {
    (i64::from(rate) * i64::from(ms) + 999) / 1_000
}
