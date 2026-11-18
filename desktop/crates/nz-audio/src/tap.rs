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
//!
//! # Several readers (MB12)
//!
//! A tap used to carry ONE reader, taken by `Engine::tap()` and nobody else: the
//! visualiser owns it on the desktop. The phone's equalizer card needs the same
//! sound at the same time as the visualiser (both can be on screen), and neither
//! may steal the other's frames, so the tap now keeps up to
//! [`MAX_TAP_READERS`] **counted subscriptions**: each has its own ring, the
//! callback pushes into the ones that are attached, and a subscription that is
//! dropped marks itself `live = false` — the mixer removes it at the next
//! buffer. At zero subscriptions the callback pushes into the pre-created slot
//! and into nothing else, which is exactly the cost it had before.
//!
//! The legacy [`crate::Engine::tap`] keeps working unchanged (slot 0, the ring
//! both halves of that call already share): a caller that wants its own reader
//! asks for [`crate::Engine::subscribe_tap`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use rtrb::{Consumer, Producer, RingBuffer};

/// Frames the tap can hold back: 170 ms at 48 kHz.
pub const TAP_FRAMES: usize = 8_192;

/// How many analysis readers one tap carries at most: the visualiser's one
/// subscription (bars and scope read the same frames) and the equalizer card's.
/// The third slot is headroom for a later view.
pub const MAX_TAP_READERS: usize = 3;

/// One reader's end of a ring, shared by both kinds of subscription.
struct Frames {
    ring: Consumer<f32>,
    dropped: Arc<AtomicU64>,
}

impl Frames {
    /// Copy what is available into `out` (interleaved stereo) and return the
    /// number of FRAMES written. `out` is filled from its start; anything past
    /// the returned frame count is untouched.
    fn read(&mut self, out: &mut [f32]) -> usize {
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

    fn frames_available(&self) -> usize {
        self.ring.slots() / 2
    }

    fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    fn clear(&mut self) {
        while self.ring.pop().is_ok() {}
    }
}

/// The reader the analysis thread holds. One is taken from the engine
/// ([`crate::Engine::tap`]); a view that wants its OWN reader asks for a
/// [`TapSubscription`] instead, so two of them can look at the same sound.
pub struct TapReader {
    frames: Frames,
}

impl TapReader {
    /// Copy what is available into `out` (interleaved stereo) and return the
    /// number of FRAMES written. `out` is filled from its start; anything past
    /// the returned frame count is untouched.
    pub fn read(&mut self, out: &mut [f32]) -> usize {
        self.frames.read(out)
    }

    /// Frames waiting to be read.
    pub fn frames_available(&self) -> usize {
        self.frames.frames_available()
    }

    /// Frames the callback dropped because this reader was behind (or absent).
    /// A visualiser that reports this in its diagnostics is telling the truth
    /// about a stutter instead of hiding it.
    pub fn dropped(&self) -> u64 {
        self.frames.dropped()
    }

    /// Throw away what is queued: after a pause, a seek or a hidden window, the
    /// analyser wants the newest audio, not a stale second of it.
    pub fn clear(&mut self) {
        self.frames.clear()
    }
}

/// A counted subscription to the tap: its own ring, its own drop counter, and a
/// `live` flag the mixer checks at every buffer.
///
/// Dropping it stops the pushes (`live = false`, read at the next buffer) — no
/// command, no lock, nothing the output callback waits on. This is what makes
/// "0 analysis CPU when the card leaves" true at the source: an unsubscribed
/// viewer is not sent a frame at all.
pub struct TapSubscription {
    frames: Frames,
    live: Arc<AtomicBool>,
}

impl TapSubscription {
    pub fn read(&mut self, out: &mut [f32]) -> usize {
        self.frames.read(out)
    }

    pub fn frames_available(&self) -> usize {
        self.frames.frames_available()
    }

    pub fn dropped(&self) -> u64 {
        self.frames.dropped()
    }

    pub fn clear(&mut self) {
        self.frames.clear()
    }
}

impl Drop for TapSubscription {
    fn drop(&mut self) {
        self.live.store(false, Ordering::Release);
    }
}

/// A feed for a subscription with no engine behind it — tests, and a tool that
/// synthesises frames. It pushes exactly as the mixer does (whole frames,
/// dropping what does not fit), so a reader cannot tell the difference.
pub struct TapFeed {
    ring: Producer<f32>,
    dropped: Arc<AtomicU64>,
}

impl TapFeed {
    pub fn push(&mut self, stereo: &[f32]) {
        push_stereo(&mut self.ring, &self.dropped, stereo);
    }

    /// Frames this feed had to drop because the reader was behind.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// A subscription and its feed, with no engine in between ([`TapFeed`]).
pub fn tap_channel(frames: usize) -> (TapFeed, TapSubscription) {
    let capacity = (frames.max(2) * 2) & !1;
    let (producer, consumer) = RingBuffer::new(capacity);
    let dropped = Arc::new(AtomicU64::new(0));
    let live = Arc::new(AtomicBool::new(true));
    (
        TapFeed {
            ring: producer,
            dropped: Arc::clone(&dropped),
        },
        TapSubscription {
            frames: Frames {
                ring: consumer,
                dropped,
            },
            live,
        },
    )
}

/// Offer `stereo` to one ring, whole frames only, and count what does not fit.
/// Never waits: this runs in the output callback.
fn push_stereo(ring: &mut Producer<f32>, dropped: &AtomicU64, stereo: &[f32]) {
    let room = ring.slots() & !1;
    let offered = stereo.len() & !1;
    let take = room.min(offered);
    if take > 0 {
        let _ = ring.push_partial_slice(&stereo[..take]);
    }
    if offered > take {
        dropped.fetch_add(((offered - take) / 2) as u64, Ordering::Relaxed);
    }
}

/// What the engine hands the mixer to attach one subscription's producer.
pub(crate) struct TapSlot {
    ring: Producer<f32>,
    dropped: Arc<AtomicU64>,
    live: Arc<AtomicBool>,
}

impl TapSlot {
    /// Build the ring and both ends: the slot goes to the mixer, the reader to
    /// the app.
    pub(crate) fn pair(frames: usize) -> (TapSlot, TapSubscription) {
        let capacity = (frames.max(2) * 2) & !1;
        let (producer, consumer) = RingBuffer::new(capacity);
        let dropped = Arc::new(AtomicU64::new(0));
        let live = Arc::new(AtomicBool::new(true));
        (
            TapSlot {
                ring: producer,
                dropped: Arc::clone(&dropped),
                live: Arc::clone(&live),
            },
            TapSubscription {
                frames: Frames {
                    ring: consumer,
                    dropped,
                },
                live,
            },
        )
    }
}

/// The callback's side of the tap: one producer per reader, no allocation, no
/// wait.
pub(crate) struct Tap {
    slots: [Option<TapSlot>; MAX_TAP_READERS],
}

impl Tap {
    /// Build the ring and both ends: the callback keeps the producer, the app
    /// gets the reader.
    pub(crate) fn pair(frames: usize) -> (Tap, TapReader) {
        // Even, so a stereo frame is never split across the wrap.
        let capacity = (frames.max(2) * 2) & !1;
        let (producer, consumer) = RingBuffer::new(capacity);
        let dropped = Arc::new(AtomicU64::new(0));
        // The legacy reader never detaches: its ring stays the first slot for
        // the life of the engine, exactly as before MB12.
        let live = Arc::new(AtomicBool::new(true));
        let mut slots = std::array::from_fn(|_| None);
        slots[0] = Some(TapSlot {
            ring: producer,
            dropped: Arc::clone(&dropped),
            live,
        });
        (
            Tap { slots },
            TapReader {
                frames: Frames {
                    ring: consumer,
                    dropped,
                },
            },
        )
    }

    /// Attach a subscription's producer. Answers false when every slot is taken.
    pub(crate) fn attach(&mut self, slot: TapSlot) -> bool {
        for entry in self.slots.iter_mut() {
            if entry.is_none() {
                *entry = Some(slot);
                return true;
            }
        }
        false
    }

    /// How many readers the callback will push to right now (a dead slot counts
    /// until the next `push` frees it). Tests read this; nothing else does.
    #[cfg(test)]
    pub(crate) fn attached(&self) -> usize {
        self.slots.iter().flatten().count()
    }

    /// Offer `stereo` to every attached reader, whole frames only, and drop what
    /// does not fit. Never waits: this runs in the output callback.
    pub(crate) fn push(&mut self, stereo: &[f32]) {
        for entry in self.slots.iter_mut() {
            let Some(slot) = entry else {
                continue;
            };
            // A subscription dropped on another thread takes effect here, at a
            // buffer boundary: from then on it costs nothing.
            if !slot.live.load(Ordering::Relaxed) {
                *entry = None;
                continue;
            }
            push_stereo(&mut slot.ring, &slot.dropped, stereo);
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

    /// **Two readers, the same sound (MB12).** The visualiser and the equalizer
    /// card are on screen together: each has its own ring, both get every frame,
    /// and neither can steal the other's.
    #[test]
    fn two_subscriptions_each_read_every_frame() {
        let (mut tap, mut legacy) = Tap::pair(64);
        let (slot, mut second) = TapSlot::pair(64);
        assert!(tap.attach(slot), "a free slot");
        tap.push(&[0.1, -0.1, 0.2, -0.2, 0.3, -0.3]);

        let mut a = vec![0.0f32; 8];
        let mut b = vec![0.0f32; 8];
        assert_eq!(legacy.read(&mut a), 3);
        assert_eq!(second.read(&mut b), 3);
        assert_eq!(a, b);
        assert_eq!(&a[..6], &[0.1, -0.1, 0.2, -0.2, 0.3, -0.3]);
        // One reader clearing its own ring changes nothing for the other.
        second.clear();
        assert_eq!(second.frames_available(), 0);
        tap.push(&[0.5, 0.5]);
        let mut c = vec![0.0f32; 2];
        assert_eq!(legacy.read(&mut c), 1, "the legacy reader keeps its frames");
    }

    /// **A subscription that goes away stops costing anything**: dropping the
    /// reader marks it dead, the mixer frees the slot at the next buffer, and
    /// from then on nothing is pushed into its ring. This is the source of
    /// "0 analysis CPU when the card leaves".
    #[test]
    fn a_dropped_subscription_is_removed_at_the_next_buffer() {
        let (mut tap, _legacy) = Tap::pair(64);
        let (slot, second) = TapSlot::pair(64);
        assert!(tap.attach(slot));
        assert_eq!(tap.attached(), 2, "the legacy slot plus the subscription");
        drop(second);
        tap.push(&[0.25, 0.25]);
        assert_eq!(tap.attached(), 1, "the dead slot was freed by the push");
    }
}
