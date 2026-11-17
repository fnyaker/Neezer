//! The API-26 path: one Kotlin thread pulls, and this is what it pulls from.
//!
//! `AudioTrackPull.kt` owns an `AudioTrack` in `MODE_STREAM`, a direct
//! `ByteBuffer`, and a loop: `nativeRender` fills the buffer from the engine's
//! sink, `AudioTrack.write(…, WRITE_BLOCKING)` hands it to the device. This
//! session is the Rust half: the sink, and the two numbers the device must tell
//! the engine (its latency and its xrun total).
//!
//! A session is a `Box` turned into a handle ([`PullSession::into_handle`]); the
//! Kotlin object keeps the `long` and gives it back with `nativeClose`. 0 is
//! "none" everywhere, so a double close is a no-op.

use std::sync::Arc;

use nz_audio::{Engine, ExternalSink, OutputInfo};

use crate::CHANNELS;
use crate::output::OutputPrefs;
use crate::pull::{Handback, Pull};

/// The thing a jlong points at.
pub struct PullSession {
    pull: Pull,
    engine: Engine,
    prefs: OutputPrefs,
    /// The buffer the Kotlin side will ask `AudioTrack` for, in frames: the
    /// first latency the engine is told, before the device reports its own.
    buffer_frames: u64,
}

impl PullSession {
    /// Take the sink for the API-26 pull. The engine is told the device's shape
    /// at once (rate, channels, the buffer we are about to request), so a
    /// position read before the first write is already sane.
    pub fn new(engine: &Engine, sink: ExternalSink, prefs: OutputPrefs) -> Self {
        let handback = Handback::new(sink);
        let pull = Pull::attach(&handback);
        let buffer_frames =
            crate::frames_for_ms(prefs.sample_rate, prefs.buffer_ms_min).max(0) as u64;
        engine.set_external_output(OutputInfo {
            device: "AudioTrack (API 26)".to_string(),
            sample_rate: prefs.sample_rate,
            channels: CHANNELS as u16,
            latency_frames: buffer_frames,
        });
        Self {
            pull,
            engine: engine.clone(),
            prefs,
            buffer_frames,
        }
    }

    /// Fill `out` (interleaved stereo f32). This is the function the Kotlin
    /// thread's `nativeRender` reaches, and it is as real-time as AAudio's
    /// callback: no allocation, no lock, no syscall.
    pub fn fill(&mut self, out: &mut [f32]) {
        self.pull.fill(out);
    }

    /// What the Kotlin side asked `AudioTrack` for, in frames — the number it
    /// must size its direct `ByteBuffer` from.
    pub fn buffer_frames(&self) -> u64 {
        self.buffer_frames
    }

    pub fn prefs(&self) -> OutputPrefs {
        self.prefs
    }

    /// The device's own buffer (`AudioTrack.getLatency()` × rate, or its
    /// `getBufferSizeInFrames()`): the engine's clock subtracts it.
    pub fn set_latency(&self, frames: u64) {
        self.engine.set_external_output(OutputInfo {
            device: "AudioTrack (API 26)".to_string(),
            sample_rate: self.prefs.sample_rate,
            channels: CHANNELS as u16,
            latency_frames: frames,
        });
    }

    /// `AudioTrack.getUnderrunCount()`, a TOTAL since that track was created.
    /// The Kotlin side accumulates across the tracks it re-creates on a route
    /// change and passes the running total, which is what `note_underruns`
    /// expects (a repeated reading must not inflate it).
    pub fn note_underruns(&self, total: u64) {
        self.engine.note_underruns(total);
    }

    /// Turn the session into the `long` Kotlin holds. The caller must hand it
    /// back to [`close_handle`] (or the JNI `nativeClose`).
    pub fn into_handle(self) -> i64 {
        Box::into_raw(Box::new(self)) as i64
    }
}

/// A session from a handle, or `None` for 0/a free'd one.
///
/// # Safety
/// `handle` must be a value [`PullSession::into_handle`] returned and not yet
/// closed, and the caller must not use it concurrently with [`close_handle`].
pub unsafe fn session_mut<'a>(handle: i64) -> Option<&'a mut PullSession> {
    if handle == 0 {
        return None;
    }
    // SAFETY: the caller promises the handle came from `into_handle` and is live.
    unsafe { (handle as *mut PullSession).as_mut() }
}

/// Close a session: the sink goes back to its hand-back slot and is dropped with
/// it.
///
/// # Safety
/// `handle` must have come from [`PullSession::into_handle`] and must not be used
/// again.
pub unsafe fn close_handle(handle: i64) {
    if handle == 0 {
        return;
    }
    // SAFETY: the caller promises the handle is live and not used again.
    drop(unsafe { Box::from_raw(handle as *mut PullSession) });
}

/// The atomics behind a session's sink, for a caller that wants the clock
/// without going through the engine.
pub fn shared_of(session: &PullSession) -> Option<Arc<nz_audio::MixerShared>> {
    session.pull.shared()
}
