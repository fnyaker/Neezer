//! A SCHEDULED PCM channel on the OUTPUT clock: what a listen-party guest plays.
//!
//! Lot D15b (`docs/agents/desktop.md`). A party guest does not play a track: it
//! receives the host's timeline (`webapp/src/lib/party/timeline.js`) and six-second
//! chunks of that track (`neezer/webui/party.py#party_chunk`), decodes them, and
//! must have each one come out of THIS machine's speakers at the instant the host's
//! does. A media element cannot do that (it starts when it is ready), and neither
//! can `Engine::load` (it opens a source and starts when the decoder has audio):
//! both are "play as soon as you can", and a guest needs "play exactly here".
//!
//! So the mixer grows one more channel, and it is addressed in the OUTPUT's own
//! time rather than in a track's. The device tells us, on every callback, which
//! stream position it is about to make audible and when (cpal's `playback`
//! timestamp, the same one [`crate::Clock::note_output`] keeps): the guest turns
//! "the host is at 41.5 s at server time T" into "the samples of media 41.5 s must
//! occupy stream positions [u, u + len)", and the callback writes whichever
//! samples cover the positions it is filling. The alignment is therefore the
//! device's, not a timer's — the plan's "scheduled on the device clock".
//!
//! Two properties matter and are tested:
//!
//! - **Sample-exact placement.** A segment scheduled at `from_s` starts at the
//!   first stream frame at or after `from_s`, and nowhere else: the test renders a
//!   click scheduled at 1.000 s through an offline sink and finds it at frame
//!   `48 000`, not one frame before.
//! - **No allocation, no lock, no syscall in the callback.** Segments arrive on an
//!   `rtrb` ring into a preallocated slot table; the audio is an `Arc<[f32]>` the
//!   CALLER keeps a clone of, so the callback dropping a finished segment is a
//!   refcount decrement and the buffer is freed on the guest's own thread. The
//!   edges of every segment are faded over [`EDGE_S`] (12 ms, the seam the web
//!   player crossfades), which is what makes a correction inaudible instead of a
//!   click.
//!
//! The correction RULES (keep / next seam / jump / wait) are the guest's, not
//! this channel's: it plays what it was given, where it was told. They live in
//! `app/src/wiring/party_guest.rs`, with the SPA's `timeline.correctionFor` ported
//! next to them ([party-remote.md](../../../../docs/agents/party-remote.md)).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::clock::AtomicF32;

/// How long a segment's own edges are faded, in seconds. The web player's
/// `engine.js#EDGE` (12 ms): inaudible as a fade, and long enough that a cut in
/// the middle of a waveform is not a click.
pub const EDGE_S: f64 = 0.012;

/// How many segments one channel holds at once. The guest prefetches four chunks
/// ahead (20 s) and a correction may schedule one more before the old ones are
/// consumed; six is that with slack, and the table is preallocated.
pub const SLOTS: usize = 6;

/// A run of PCM — interleaved stereo, at the device's rate — to be HEARD over
/// `[from_s, to_s)` of the output stream.
///
/// `from_s` and `to_s` are stream positions (see
/// [`crate::Engine::output_clock`]), so a guest that schedules a chunk at 41.5 s
/// hears it at 41.5 s of the stream whatever else the mixer is doing.
#[derive(Clone, Debug)]
pub struct Segment {
    pub from_s: f64,
    pub to_s: f64,
    /// Per-segment gain: the party's loudness normalisation rides here (a track
    /// the host normalised sits at the same relative level on every device, the
    /// SPA's `engine.js#normGain`).
    pub gain: f32,
    /// The ramp up at `from_s` and down at `to_s`. A neighbour that starts inside
    /// this segment turns both into the overlap's length, so the two sides of a
    /// chunk seam CROSSFADE (the web player's `engine.js#setTail`) instead of
    /// summing into a 6 dB swell.
    pub fade_in_s: f64,
    pub fade_out_s: f64,
    pub data: Arc<[f32]>,
}

impl Segment {
    /// A segment of `data` starting at `from_s`, its length read from the buffer.
    /// `rate` is the device's own (the guest resamples to it when it decodes).
    pub fn at(from_s: f64, rate: f64, data: Arc<[f32]>) -> Segment {
        let frames = data.len() / 2;
        let len = frames as f64 / rate.max(1.0);
        Segment {
            from_s,
            to_s: from_s + len,
            gain: 1.0,
            fade_in_s: EDGE_S,
            fade_out_s: EDGE_S,
            data,
        }
    }

    pub fn frames(&self) -> usize {
        self.data.len() / 2
    }
}

/// What the guest asks the callback to do. Everything is a whole segment: a
/// correction is `Clear` + a new `Schedule`, which is exactly the web player's
/// "drop the nodes placed on the old mapping and place them again".
pub(crate) enum PartyCmd {
    Schedule(Segment),
    /// Cut what sounds over one edge and drop everything still to come (a pause,
    /// a skip, a jump).
    Clear,
    /// Drop only what has not begun: the segment being heard plays out. What a
    /// correction inside a seam's worth needs — dropping the future placements is
    /// what makes the NEXT chunk land on the corrected line, and cutting the
    /// sounding one as well would be an audible hole for a few milliseconds.
    ClearFuture,
    /// The party takes the output over (the person's own track is not mixed in
    /// while they listen to somebody else's) — or gives it back.
    TakeOver(bool),
}

/// The callback's side of the channel.
pub(crate) struct PartyVoice {
    cmds: rtrb::Consumer<PartyCmd>,
    slots: Vec<Segment>,
    gain: Arc<AtomicF32>,
    own: Arc<AtomicBool>,
    taken_over: bool,
}

impl PartyVoice {
    pub(crate) fn new(
        cmds: rtrb::Consumer<PartyCmd>,
        gain: Arc<AtomicF32>,
        own: Arc<AtomicBool>,
    ) -> PartyVoice {
        PartyVoice {
            cmds,
            slots: Vec::with_capacity(SLOTS),
            gain,
            own,
            taken_over: false,
        }
    }

    /// Whether the party owns the output: the local voices are then left alone
    /// (they keep their position, the person's queue is intact) instead of being
    /// mixed under somebody else's music.
    pub(crate) fn taken_over(&self) -> bool {
        self.taken_over
    }

    /// Apply what the guest queued. Called at the top of every buffer, like the
    /// mixer's own command drain; `now_s` is the stream position the buffer
    /// starts at, which is what tells a segment that already sounds from one
    /// that has not begun.
    pub(crate) fn drain(&mut self, now_s: f64) {
        while let Ok(cmd) = self.cmds.pop() {
            match cmd {
                PartyCmd::Schedule(segment) => self.place(segment),
                // A correction, a pause, a skip: whatever has not started goes,
                // and what sounds is cut over one edge — the web player's
                // `stopVoice(v, now + LEAD, EDGE)`, so the room hears a 12 ms
                // fade, never a click. It is `Clear` + a new `Schedule` that
                // moves a guest, forward to catch up or back after a rewind.
                PartyCmd::ClearFuture => self.slots.retain(|slot| slot.from_s <= now_s),
                PartyCmd::Clear => {
                    self.slots.retain(|slot| slot.from_s <= now_s);
                    for slot in self
                        .slots
                        .iter_mut()
                        .filter(|slot| slot.from_s <= now_s && slot.to_s > now_s)
                    {
                        slot.to_s = now_s + EDGE_S;
                        slot.fade_out_s = EDGE_S;
                    }
                }
                PartyCmd::TakeOver(on) => {
                    self.taken_over = on;
                    self.own.store(on, Ordering::Release);
                }
            }
        }
    }

    /// Add one segment, keeping the soonest [`SLOTS`] when the table is full: a
    /// guest that re-schedules after a correction replaces the FAR future, never
    /// the chunk it is about to play.
    fn place(&mut self, mut segment: Segment) {
        if !segment.from_s.is_finite() || !segment.to_s.is_finite() || segment.data.is_empty() {
            return;
        }
        // A neighbour this one begins inside: the seam they share becomes a
        // crossfade over their overlap, on both sides.
        for slot in self.slots.iter_mut() {
            if slot.from_s < segment.from_s && slot.to_s > segment.from_s {
                let overlap = (slot.to_s - segment.from_s).max(EDGE_S);
                slot.fade_out_s = overlap;
                segment.fade_in_s = segment
                    .fade_in_s
                    .max(overlap)
                    .min(segment.to_s - segment.from_s);
            }
        }
        if self.slots.len() >= SLOTS {
            // The furthest-out one goes — and if that is the newcomer itself there
            // is nothing to gain: a schedule is a look-ahead, not a backlog.
            let furthest = self
                .slots
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.from_s.total_cmp(&b.1.from_s))
                .map(|(index, slot)| (index, slot.from_s));
            match furthest {
                Some((index, from_s)) if from_s > segment.from_s => {
                    self.slots.swap_remove(index);
                }
                _ => return,
            }
        }
        self.slots.push(segment);
        self.slots.sort_by(|a, b| a.from_s.total_cmp(&b.from_s));
    }

    /// Forget segments that are entirely in the past.
    pub(crate) fn retire(&mut self, stream_s: f64) {
        self.slots.retain(|slot| slot.to_s > stream_s);
    }

    /// Mix this buffer's share of the schedule into `out`.
    ///
    /// `first_s` is the stream position of the buffer's first frame; frame `i` is
    /// heard at `first_s + i/rate`. Returns true when at least one frame was
    /// written, which the mixer uses to keep the DSP and the tap running for a
    /// party buffer with nothing else in it.
    pub(crate) fn mix(&mut self, out: &mut [f32], first_s: f64, rate: f64, frames: usize) -> bool {
        let gain = self.gain.load();
        if gain <= 0.0 || self.slots.is_empty() || frames == 0 {
            return false;
        }
        let gain = f64::from(gain);
        let mut wrote = false;
        let end_s = first_s + frames as f64 / rate;
        for slot in &self.slots {
            if slot.to_s <= first_s || slot.from_s >= end_s {
                continue;
            }
            let data = &slot.data;
            let slot_gain = gain * f64::from(slot.gain);
            // The frames of this buffer the segment covers.
            let first = (((slot.from_s - first_s) * rate).ceil().max(0.0) as usize).min(frames);
            let last = (((slot.to_s - first_s) * rate).ceil().max(0.0) as usize).min(frames);
            for index in first..last {
                let heard = first_s + index as f64 / rate;
                let offset = ((heard - slot.from_s) * rate).round();
                let sample = if offset < 0.0 { 0 } else { offset as usize };
                if sample * 2 + 1 >= data.len() {
                    break;
                }
                // The two edges: a segment that starts mid-word still starts
                // without a click, and a seam crossfades over the chunks' own
                // overlap.
                let env = ((heard - slot.from_s) / slot.fade_in_s.max(1e-9))
                    .min((slot.to_s - heard) / slot.fade_out_s.max(1e-9))
                    .clamp(0.0, 1.0);
                let scale = slot_gain * env;
                out[index * 2] += (data[sample * 2] as f64 * scale) as f32;
                out[index * 2 + 1] += (data[sample * 2 + 1] as f64 * scale) as f32;
                wrote = true;
            }
        }
        wrote
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(frames: usize, value: f32) -> Arc<[f32]> {
        vec![value; frames * 2].into()
    }

    fn voice() -> (PartyVoice, rtrb::Producer<PartyCmd>, Arc<AtomicF32>) {
        let (tx, rx) = rtrb::RingBuffer::new(8);
        let gain = Arc::new(AtomicF32::new(1.0));
        let own = Arc::new(AtomicBool::new(false));
        (PartyVoice::new(rx, Arc::clone(&gain), own), tx, gain)
    }

    #[test]
    fn a_segment_is_placed_where_it_was_asked_and_nowhere_else() {
        let (mut voice, mut tx, _) = voice();
        // 1 s of a tone at 0.25, scheduled at stream position 1.0 s.
        tx.push(PartyCmd::Schedule(Segment::at(
            1.0,
            48_000.0,
            tone(48_000, 0.25),
        )))
        .ok();
        voice.drain(47_990.0 / 48_000.0);
        // Frames 47 990..48 010 of the stream: the tone starts at 48 000.
        let mut out = vec![0.0f32; 40 * 2];
        voice.mix(&mut out, 47_990.0 / 48_000.0, 48_000.0, 40);
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        // The 12 ms fade means the first frames ramp in; what matters is WHERE
        // the sound starts.
        assert!(left[..10].iter().all(|s| *s == 0.0), "{left:?}");
        assert!(left[20] > 0.0, "{left:?}");
    }

    #[test]
    fn a_full_table_keeps_the_soonest_segments() {
        let (mut voice, mut tx, _) = voice();
        // Six-second chunks carrying their 50 ms overlap, as the server cuts them.
        for k in 0..(SLOTS + 2) {
            tx.push(PartyCmd::Schedule(Segment::at(
                k as f64 * 6.0,
                48_000.0,
                tone(291_600, 0.1),
            )))
            .ok();
        }
        voice.drain(0.0);
        assert_eq!(voice.slots.len(), SLOTS);
        assert_eq!(voice.slots[0].from_s, 0.0);
        assert_eq!(voice.slots[SLOTS - 1].from_s, (SLOTS - 1) as f64 * 6.0);
    }

    #[test]
    fn clear_future_lets_the_sounding_segment_finish() {
        let (mut voice, mut tx, _) = voice();
        tx.push(PartyCmd::Schedule(Segment::at(
            0.0,
            48_000.0,
            tone(290_400, 0.1),
        )))
        .ok();
        tx.push(PartyCmd::Schedule(Segment::at(
            6.0,
            48_000.0,
            tone(290_400, 0.1),
        )))
        .ok();
        voice.drain(0.0);
        tx.push(PartyCmd::ClearFuture).ok();
        voice.drain(3.0);
        assert_eq!(voice.slots.len(), 1);
        // Untouched: it ends where its chunk ends (6.05 s), still crossfading over
        // the 50 ms overlap the server cut into its tail.
        assert!((voice.slots[0].to_s - 6.05).abs() < 1e-9);
        assert!((voice.slots[0].fade_out_s - 0.05).abs() < 1e-12);
    }

    #[test]
    fn clear_cuts_what_sounds_and_drops_the_future() {
        let (mut voice, mut tx, _) = voice();
        tx.push(PartyCmd::Schedule(Segment::at(
            0.0,
            48_000.0,
            tone(291_600, 0.1),
        )))
        .ok();
        tx.push(PartyCmd::Schedule(Segment::at(
            6.0,
            48_000.0,
            tone(291_600, 0.1),
        )))
        .ok();
        voice.drain(0.0);
        assert_eq!(voice.slots.len(), 2);
        tx.push(PartyCmd::Clear).ok();
        voice.drain(3.0);
        // The one sounding is cut over one edge (12 ms), the one still to come
        // is gone: a correction never leaves the old track playing under the new
        // one (the bug `party-remote.md` records).
        assert_eq!(voice.slots.len(), 1);
        assert!((voice.slots[0].to_s - (3.0 + EDGE_S)).abs() < 1e-12);
        assert!((voice.slots[0].fade_out_s - EDGE_S).abs() < 1e-12);
    }

    #[test]
    fn the_edge_fades_are_the_only_thing_that_touches_the_gain() {
        let (mut voice, mut tx, gain) = voice();
        tx.push(PartyCmd::Schedule(Segment::at(
            0.0,
            48_000.0,
            tone(48_000, 1.0),
        )))
        .ok();
        voice.drain(0.0);
        let mut out = vec![0.0f32; 48_000 * 2];
        assert!(voice.mix(&mut out, 0.0, 48_000.0, 48_000));
        // 12 ms in, at full gain.
        let at = 600;
        assert!((out[at * 2] - 1.0).abs() < 1e-6);
        gain.store(0.5);
        let mut out = vec![0.0f32; 48_000 * 2];
        voice.mix(&mut out, 0.0, 48_000.0, 48_000);
        assert!((out[at * 2] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn two_overlapping_segments_crossfade_over_their_edges() {
        let (mut voice, mut tx, _) = voice();
        // The party's chunks carry 50 ms of overlap: two neighbours sum there.
        tx.push(PartyCmd::Schedule(Segment {
            from_s: 0.0,
            to_s: 6.0,
            gain: 1.0,
            fade_in_s: EDGE_S,
            fade_out_s: EDGE_S,
            data: tone(6 * 48_000, 0.5),
        }))
        .ok();
        tx.push(PartyCmd::Schedule(Segment {
            from_s: 5.95,
            to_s: 11.95,
            gain: 1.0,
            fade_in_s: EDGE_S,
            fade_out_s: EDGE_S,
            data: tone(6 * 48_000, 0.5),
        }))
        .ok();
        voice.drain(5.95);
        let mut out = vec![0.0f32; 48_000 * 2];
        // The seam at 6 s: complementary ramps over the chunks' 50 ms overlap, so
        // the sum stays at the track's own level — no hole, no 6 dB swell.
        voice.mix(&mut out, 5.95, 48_000.0, 48_000);
        let at = |ms: f64| out[((ms / 1000.0) * 48_000.0) as usize * 2];
        assert!((at(25.0) - 0.5).abs() < 0.02, "{}", at(25.0));
        assert!((at(40.0) - 0.5).abs() < 0.02, "{}", at(40.0));
        assert!((at(60.0) - 0.5).abs() < 0.02, "{}", at(60.0));
        assert!(out.iter().all(|s| s.is_finite() && *s <= 1.0));
    }

    #[test]
    fn the_caller_keeps_the_buffer_alive_and_the_callback_only_drops_it() {
        let (mut voice, mut tx, _) = voice();
        let data = tone(48_000, 0.2);
        let keep = Arc::clone(&data);
        assert_eq!(Arc::strong_count(&data), 2);
        tx.push(PartyCmd::Schedule(Segment::at(0.0, 48_000.0, data)))
            .ok();
        voice.drain(0.0);
        // Past its end: the mixer retires it, the callback's copy goes, and the
        // caller's clone is what frees the memory — on the guest's own thread.
        voice.retire(7.0);
        assert!(voice.slots.is_empty());
        assert_eq!(Arc::strong_count(&keep), 1);
    }

    #[test]
    fn taking_over_is_what_the_mixer_reads_to_leave_the_local_voices_alone() {
        let (mut voice, mut tx, _) = voice();
        tx.push(PartyCmd::TakeOver(true)).ok();
        voice.drain(0.0);
        assert!(voice.taken_over());
        tx.push(PartyCmd::TakeOver(false)).ok();
        voice.drain(0.0);
        assert!(!voice.taken_over());
    }
}
