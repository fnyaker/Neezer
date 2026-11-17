//! The callback body, and the hand-back that keeps ONE sink across a re-open.
//!
//! AAudio calls the data callback on a real-time thread it owns, and the stream
//! object itself cannot leave the thread that opened it. So the sink travels:
//!
//! ```text
//! DroidOutput::start  ──►  Handback (one slot, holds the ExternalSink)
//!                             │ take()             ▲ put()
//!                             ▼                    │
//!                        nz-out-reopen  ──►  Pull (lives inside the AAudio stream)
//!                                                │ fill() on the real-time thread
//! ```
//!
//! `take` and `put` both run on the **opening thread** — `put` from
//! [`Pull`]'s `Drop`, which runs when the stream that owned it is dropped, on
//! the thread that drops it (the reopen thread). [`Pull::fill`] is the only
//! function a device's real-time thread runs, and it touches neither the slot
//! nor its lock.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use nz_audio::ExternalSink;

/// The one slot the sink waits in between two streams.
///
/// A `Mutex` here is not a lock in the callback: the only two callers are the
/// thread that opens a stream and the `Drop` of the [`Pull`] that stream owned,
/// which runs on that same thread when the stream is closed. No real-time
/// thread ever takes it.
pub struct Handback {
    slot: Mutex<Option<ExternalSink>>,
    returns: AtomicU64,
    misses: AtomicU64,
}

impl Handback {
    /// Put the engine's sink in the slot. The first [`Pull::attach`] takes it.
    pub fn new(sink: ExternalSink) -> Arc<Self> {
        Arc::new(Self {
            slot: Mutex::new(Some(sink)),
            returns: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        })
    }

    /// Take the sink for a new stream. `None` means a previous stream still
    /// holds it (or the sink is already gone): the caller must not open.
    pub fn take(&self) -> Option<ExternalSink> {
        let taken = guard(&self.slot).take();
        if taken.is_none() {
            self.misses.fetch_add(1, Ordering::Relaxed);
        }
        taken
    }

    fn put(&self, sink: ExternalSink) {
        *guard(&self.slot) = Some(sink);
        self.returns.fetch_add(1, Ordering::Release);
    }

    /// How many times a closed stream gave the sink back: the count a reopen
    /// test holds (one per stream, never two, never zero).
    pub fn returns(&self) -> u64 {
        self.returns.load(Ordering::Acquire)
    }

    /// How many times a stream was asked for the sink and it was not there.
    pub fn misses(&self) -> u64 {
        self.misses.load(Ordering::Acquire)
    }

    /// Is the sink in the slot right now (nothing playing)?
    pub fn idle(&self) -> bool {
        guard(&self.slot).is_some()
    }
}

/// A poisoned slot is not a reason to lose the sink: the guard is only ever held
/// for a move, so the data behind it is always consistent.
fn guard(slot: &Mutex<Option<ExternalSink>>) -> MutexGuard<'_, Option<ExternalSink>> {
    slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The callback body itself: everything a device's real-time thread is allowed
/// to run.
///
/// It owns the sink (so the device callback can render without a lock) and puts
/// it back when it is dropped, which is what lets the reopen thread hand the
/// SAME mixer to the next stream.
pub struct Pull {
    sink: Option<ExternalSink>,
    back: Arc<Handback>,
}

impl Pull {
    /// Take the sink out of `back` for a stream that is being opened. Runs on the
    /// opening thread, never in a callback.
    pub fn attach(back: &Arc<Handback>) -> Self {
        Self {
            sink: back.take(),
            back: Arc::clone(back),
        }
    }

    /// Is this stream actually armed? `false` means the previous stream has not
    /// given the sink back yet, and the caller must not open.
    pub fn attached(&self) -> bool {
        self.sink.is_some()
    }

    /// Fill `out` with interleaved stereo f32 at the engine's rate.
    ///
    /// **No allocation, no lock, no syscall.** A stream whose sink has already
    /// been handed back (a re-open race) writes silence rather than panicking:
    /// the clock does not advance, so the mixer resumes exactly where it stopped
    /// when the next stream is armed.
    pub fn fill(&mut self, out: &mut [f32]) {
        match self.sink.as_mut() {
            Some(sink) => sink.render(out),
            None => out.fill(0.0),
        }
    }

    /// The engine's own atomics (clock, underrun totals) for a caller that needs
    /// them: the API-26 path reports latency and xruns through them.
    pub fn shared(&self) -> Option<Arc<nz_audio::MixerShared>> {
        self.sink.as_ref().map(ExternalSink::shared)
    }
}

impl Drop for Pull {
    /// The stream that owned this `Pull` is being closed, on the thread that
    /// closes it: give the sink back so the next stream continues the same
    /// mixer.
    fn drop(&mut self) {
        if let Some(sink) = self.sink.take() {
            self.back.put(sink);
        }
    }
}
