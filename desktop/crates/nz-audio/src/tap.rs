//! The analysis tap: the sound, after the DSP, into a ring the visualiser reads.
//!
//! The page taps its graph at the `output` node (`graph.js`:
//! `output → analysisBus → analyserLo/Hi / scope / rhythm`), i.e. **after** the
//! EQ, the bass compressor and the limiter, and **before** the mono/balance
//! stage — a mono fold would erase the two channels the scope exists to draw,
//! and a balance would move an image the animation is not supposed to inherit
//! (`pan.js`). This tap is at the same point of the desktop's mix.
//!
//! **It never blocks the output callback.** The callback pushes with
//! `push_partial_slice` and drops whatever does not fit: a visualiser that is
//! not reading (the window is hidden, the person turned it off) must cost the
//! music nothing. The analysis thread reads on its own schedule, in whole stereo
//! frames, and reports how many it got.
//!
//! The ring is 8 192 frames — 170 ms at 48 kHz, 64 KiB — which is what the
//! `rhythm` analyser needs to be handed a batch per paint while dropping less
//! than one frame per second of a 240 Hz panel.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rtrb::{Consumer, Producer, RingBuffer};

/// Frames the tap can hold back: 170 ms at 48 kHz.
pub const TAP_FRAMES: usize = 8_192;

/// The reader the analysis thread holds. One is taken from the engine
/// ([`crate::Engine::tap`]): the analysis is one consumer by design, and a
/// second one would steal frames from the first.
pub struct TapReader {
    ring: Consumer<f32>,
    dropped: Arc<AtomicU64>,
}

impl TapReader {
    /// Copy what is available into `out` (interleaved stereo) and return the
    /// number of FRAMES written. `out` is filled from its start; anything past
    /// the returned frame count is untouched.
    pub fn read(&mut self, out: &mut [f32]) -> usize {
        let room = (out.len() / 2).min(self.ring.slots() / 2);
        let mut frames = 0;
        for _ in 0..room {
            let Ok(left) = self.ring.pop() else { break };
            let Ok(right) = self.ring.pop() else { break };
            out[frames * 2] = left;
            out[frames * 2 + 1] = right;
            frames += 1;
        }
        frames
    }

    /// Frames waiting to be read.
    pub fn frames_available(&self) -> usize {
        self.ring.slots() / 2
    }

    /// Frames the callback dropped because this reader was behind (or absent).
    /// A visualiser that reports this in its diagnostics is telling the truth
    /// about a stutter instead of hiding it.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Throw away what is queued: after a pause, a seek or a hidden window, the
    /// analyser wants the newest audio, not a stale second of it.
    pub fn clear(&mut self) {
        while self.ring.pop().is_ok() {}
    }
}

/// The callback's side of the tap: a producer and the drop counter, no
/// allocation, no wait.
pub(crate) struct Tap {
    ring: Producer<f32>,
    dropped: Arc<AtomicU64>,
}

impl Tap {
    /// Build the ring and both ends: the callback keeps the producer, the app
    /// gets the reader.
    pub(crate) fn pair(frames: usize) -> (Tap, TapReader) {
        // Even, so a stereo frame is never split across the wrap.
        let capacity = (frames.max(2) * 2) & !1;
        let (producer, consumer) = RingBuffer::new(capacity);
        let dropped = Arc::new(AtomicU64::new(0));
        (
            Tap {
                ring: producer,
                dropped: Arc::clone(&dropped),
            },
            TapReader {
                ring: consumer,
                dropped,
            },
        )
    }

    /// Offer `stereo` to the reader, whole frames only, and drop what does not
    /// fit. Never waits: this runs in the output callback.
    pub(crate) fn push(&mut self, stereo: &[f32]) {
        let room = self.ring.slots() & !1;
        let offered = stereo.len() & !1;
        let take = room.min(offered);
        if take > 0 {
            let _ = self.ring.push_partial_slice(&stereo[..take]);
        }
        if offered > take {
            self.dropped
                .fetch_add(((offered - take) / 2) as u64, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What is pushed comes back frame for frame, in order, and the reader only
    /// ever sees whole frames.
    #[test]
    fn the_tap_round_trips_whole_frames_in_order() {
        let (mut tap, mut reader) = Tap::pair(64);
        tap.push(&[0.1, -0.1, 0.2, -0.2, 0.3, -0.3]);
        assert_eq!(reader.frames_available(), 3);
        let mut out = vec![0.0f32; 8];
        assert_eq!(reader.read(&mut out), 3);
        assert_eq!(&out[..6], &[0.1, -0.1, 0.2, -0.2, 0.3, -0.3]);
        assert_eq!(reader.frames_available(), 0);
        assert_eq!(reader.dropped(), 0);
    }

    /// A reader that is behind (or absent) costs the music nothing: the callback
    /// drops the newest frames and returns at once. Measured: the ring keeps its
    /// 32 frames and a 480-frame push leaves 448 of them unqueued and counted.
    #[test]
    fn a_full_ring_drops_instead_of_blocking() {
        let (mut tap, mut reader) = Tap::pair(32);
        let block = vec![0.5f32; 480 * 2];
        tap.push(&block);
        assert_eq!(reader.frames_available(), 32, "the ring is full, not grown");
        assert_eq!(reader.dropped(), 480 - 32);
        // And the reader still gets the OLDEST frames, in order: the newest are
        // the ones that were dropped.
        let mut out = vec![0.0f32; 32 * 2];
        assert_eq!(reader.read(&mut out), 32);
        assert!(out.iter().all(|sample| *sample == 0.5));
    }

    /// `clear` is the post-pause answer: the analyser must read the newest audio,
    /// not a second of stale samples.
    #[test]
    fn clearing_drops_what_was_queued() {
        let (mut tap, mut reader) = Tap::pair(64);
        tap.push(&[1.0; 200]);
        assert!(reader.frames_available() > 0);
        reader.clear();
        assert_eq!(reader.frames_available(), 0);
        tap.push(&[0.25, 0.25]);
        let mut out = vec![0.0f32; 4];
        assert_eq!(reader.read(&mut out), 1);
        assert_eq!(&out[..2], &[0.25, 0.25]);
    }
}
