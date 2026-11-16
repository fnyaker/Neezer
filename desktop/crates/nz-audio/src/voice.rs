//! One decoded track, as the two ends of a lock-free ring.
//!
//! A voice is what the mixer can play: a `0.5 s` ring of interleaved stereo
//! f32 at the device rate, filled by one decode thread, plus a small shared
//! state the callback reads (`ended`, `failed`). The decode thread owns the
//! producer, the mixer owns the consumer, and neither ever waits for the other:
//! the producer sleeps when the ring is full (that is the whole back-pressure
//! design, `docs/agents/desktop.md`, *Design*), the consumer pops what is there.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rtrb::{Consumer, Producer, RingBuffer};

/// The half of a voice the decode thread and the mixer share. Only atomics, so
/// the output callback can read it without a lock.
#[derive(Debug, Default)]
pub(crate) struct VoiceStatus {
    /// The decoder read the last packet and every decoded frame is in the ring.
    ended: AtomicBool,
    /// The engine no longer wants this voice: the decoder stops at its next
    /// packet boundary.
    stop: AtomicBool,
}

impl VoiceStatus {
    pub(crate) fn is_ended(&self) -> bool {
        self.ended.load(Ordering::Acquire)
    }

    pub(crate) fn is_stopped(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    pub(crate) fn finish(&self) {
        self.ended.store(true, Ordering::Release);
    }

    /// A decode failure ends the voice exactly like a natural end; the cause
    /// travels as its own `EngineEvent::Error` from the decode thread.
    pub(crate) fn fail(&self) {
        self.ended.store(true, Ordering::Release);
    }

    pub(crate) fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }
}

/// The decode side of a voice.
pub(crate) struct VoiceProducer {
    ring: Producer<f32>,
    status: Arc<VoiceStatus>,
}

impl VoiceProducer {
    /// Push as many whole stereo frames as fit; returns how many samples were
    /// taken (always an even number when `samples` is even, because the ring
    /// capacity is even and the consumer pops whole frames).
    pub(crate) fn push(&mut self, samples: &[f32]) -> usize {
        let (pushed, _rest) = self.ring.push_partial_slice(samples);
        pushed.len()
    }

    pub(crate) fn is_stopped(&self) -> bool {
        self.status.is_stopped()
    }
}

/// The mixer side of a voice, plus what the event thread needs to name it.
pub(crate) struct VoiceConsumer {
    ring: Consumer<f32>,
    status: Arc<VoiceStatus>,
    /// Unique per voice; the event thread maps it back to the track.
    pub(crate) generation: u64,
    /// Where in the track this voice starts, in seconds.
    pub(crate) start_s: f64,
}

impl VoiceConsumer {
    /// One interleaved stereo frame, or `None` when the ring is empty.
    ///
    /// The ring always holds whole frames: the producer is handed even slices,
    /// the capacity is even, and the consumer only ever pops two samples.
    pub(crate) fn frame(&mut self) -> Option<(f32, f32)> {
        let left = self.ring.pop().ok()?;
        let right = self.ring.pop().ok()?;
        Some((left, right))
    }

    pub(crate) fn is_ended(&self) -> bool {
        self.status.is_ended()
    }

    /// The status the engine keeps to stop this voice later.
    pub(crate) fn status(&self) -> Arc<VoiceStatus> {
        Arc::clone(&self.status)
    }

    /// Stop the decoder behind this voice (when the engine replaces it).
    pub(crate) fn stop(&self) {
        self.status.stop();
    }
}

/// A voice's ring: `seconds` of stereo audio at `sample_rate`, never odd.
pub(crate) fn voice_ring(
    sample_rate: u32,
    seconds: f32,
    generation: u64,
    start_s: f64,
) -> (VoiceProducer, VoiceConsumer, Arc<VoiceStatus>) {
    let frames = (f64::from(sample_rate) * f64::from(seconds)).round() as usize;
    // Even, so a stereo frame can never be split across a push boundary; at
    // least 1024 samples so a small `seconds` still decodes in useful chunks.
    let capacity = (frames * 2).max(1024) & !1;
    let (producer, consumer) = RingBuffer::new(capacity);
    let status = Arc::new(VoiceStatus::default());
    (
        VoiceProducer {
            ring: producer,
            status: Arc::clone(&status),
        },
        VoiceConsumer {
            ring: consumer,
            status: Arc::clone(&status),
            generation,
            start_s,
        },
        status,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D5's ring is 0.5 s per voice, and it must be able to hold whole stereo
    /// frames only: an odd capacity would let `push_partial_slice` split a
    /// frame across the wrap, and the mixer pops two samples at a time.
    #[test]
    fn a_half_second_ring_is_even_and_fits_half_a_second() {
        let count = |seconds: f32| {
            let frames = (48_000.0 * f64::from(seconds)).round() as usize;
            (frames * 2).max(1024) & !1
        };
        assert_eq!(count(0.5), 48_000);
        assert_eq!(count(0.5) % 2, 0);
        assert!(count(0.5) >= 48_000);
        // A tenth of a second still rounds up to a whole number of frames.
        assert_eq!(count(0.1), 9_600);
    }

    #[test]
    fn a_voice_reports_failure_as_end_of_stream() {
        let status = VoiceStatus::default();
        assert!(!status.is_ended());
        status.fail();
        assert!(
            status.is_ended(),
            "the mixer must move on after a decode error"
        );
    }

    #[test]
    fn pushing_a_whole_frame_round_trips_in_order() {
        let (mut producer, mut consumer, _) = voice_ring(48_000, 0.5, 1, 0.0);
        assert_eq!(producer.push(&[0.1, -0.1, 0.2, -0.2]), 4);
        assert_eq!(consumer.frame(), Some((0.1f32, -0.1f32)));
        assert_eq!(consumer.frame(), Some((0.2f32, -0.2f32)));
        assert_eq!(consumer.frame(), None);
    }
}
