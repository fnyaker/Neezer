//! The guest's scheduling brain: pure, so the ten-minute drift and the recovery
//! after a cut are assertions instead of a stopwatch.
//!
//! It remembers the placements it handed over (their chunk, where they start on
//! the output stream, and how far into the chunk they begin), which is all it
//! needs to say WHERE this device currently is: the output stream's position
//! alone does not know which chunk is sounding there.

use crate::timeline::{
    CHUNK_S, Correction, OVERLAP_S, Timeline, chunk_index, correction_for, last_chunk, position_at,
    server_at_stream, server_time_at, to_stream,
};

/// Slack given to a fresh placement: a render quantum plus main-thread jitter.
pub const LEAD_S: f64 = 0.1;
/// How far ahead a chunk is PLACED on the output clock, seconds. The web player
/// schedules three seconds ahead (`engine.js#SCHED_AHEAD`) while it FETCHES twenty;
/// the two are separate on purpose. A placement is made against the current
/// mapping from this device's clock onto the output's, and that mapping drifts
/// with the sound card's own crystal: at 35 ppm, a chunk placed 30 s early lands
/// 1.05 ms off (measured, the guest's ten-minute test), a chunk placed 7 s early
/// 0.25 ms — and every seam resets it anyway.
pub const SCHED_AHEAD_S: f64 = 7.0;
/// Chunks fetched ahead of the playhead: 4 × 6 s = 24 s of audio, and 4 requests
/// for a minute of music (the SPA's `FETCH_AHEAD`/`MAX_INFLIGHT` shape). The
/// planner places what it can of them, the guest fetches them.
pub const LOOKAHEAD: u64 = 4;

/// One chunk to hand the output: its index, the output-stream instant its first
/// heard sample belongs at, and how far into the decoded chunk to start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub chunk: u64,
    pub from_s: f64,
    pub skip_frames: u64,
}

/// What the guest must do to the output this tick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    /// Where this device is on the timeline, and where the timeline says it
    /// should be, in seconds of the track.
    pub heard_s: Option<f64>,
    pub wanted_s: Option<f64>,
    pub error_ms: f64,
    pub correction: Option<Correction>,
    /// Chunks to place, in order.
    pub place: Vec<Placement>,
    /// Drop what has not begun (a seam correction).
    pub clear_future: bool,
    /// Cut what sounds (a pause, a jump, a wait, a skip).
    pub cut: bool,
}

impl Plan {
    pub fn silent() -> Plan {
        Plan::default()
    }
}

/// Everything the planner reads this tick. A struct rather than eight arguments,
/// because each one is a measured quantity with a unit.
#[derive(Clone, Copy)]
pub struct Situation<'a> {
    pub timeline: Option<Timeline>,
    /// `server − local` milliseconds, `None` before the clock is known.
    pub offset_ms: Option<f64>,
    /// The output clock's own mapping (`output clock − local`, seconds).
    pub bridge_s: Option<f64>,
    /// The output-stream position being heard right now.
    pub stream_now: f64,
    pub rate: u32,
    pub trim_ms: f64,
    /// Milliseconds since the last poll answered.
    pub since_poll_ms: u64,
    /// The current track's duration, seconds (`0` = unknown).
    pub duration: f64,
    /// Which chunks are decoded and ready.
    pub ready: &'a dyn Fn(u64) -> bool,
}

/// The guest's scheduler: see the module note.
#[derive(Debug, Default)]
pub struct Planner {
    placed: Vec<Placement>,
    rate: u32,
}

impl Planner {
    pub fn new(rate: u32) -> Planner {
        Planner {
            placed: Vec::new(),
            rate: rate.max(1),
        }
    }

    pub fn placed(&self) -> &[Placement] {
        &self.placed
    }

    pub fn forget(&mut self) {
        self.placed.clear();
    }

    /// Put back the placement that is sounding now, after a seam correction
    /// dropped the future: what the room hears must keep sounding.
    pub fn keep_sounding(&mut self, stream_now: f64) {
        let sounding = self.placed.iter().copied().find(|placement| {
            stream_now >= placement.from_s && stream_now < placement.from_s + CHUNK_S + OVERLAP_S
        });
        self.placed.clear();
        if let Some(placement) = sounding {
            self.placed.push(placement);
        }
    }

    /// The placement sounding at `stream_s`, with how far into its media it is.
    ///
    /// The NEWEST one wins: consecutive chunks overlap by the server's 50 ms and
    /// the mixer crossfades them, so after a seam the room hears the new chunk —
    /// reporting the old one's line would measure the previous chunk's drift for
    /// ever (measured: 0.67 ms after 19 s instead of the seam's own zero).
    pub fn sounding(&self, stream_s: f64) -> Option<(Placement, f64)> {
        for placement in self.placed.iter().rev() {
            let len = CHUNK_S + OVERLAP_S;
            if stream_s >= placement.from_s && stream_s < placement.from_s + len {
                let into = stream_s - placement.from_s;
                let media = placement.chunk as f64 * CHUNK_S
                    + placement.skip_frames as f64 / f64::from(self.rate)
                    + into;
                return Some((*placement, media));
            }
        }
        None
    }

    /// One tick: where we are, whether that is where we should be, and what to
    /// place next.
    pub fn plan(&mut self, s: &Situation<'_>) -> Plan {
        let mut plan = Plan::default();
        let Some(timeline) = s.timeline else {
            // No line: nothing can be placed in time. Whatever sounds is left
            // alone (it is what the room hears) but nothing new goes out.
            if !self.placed.is_empty() && !timeline_playing(&self.placed, s) {
                self.placed.clear();
            }
            return plan;
        };
        let Some(offset) = s.offset_ms else {
            return plan;
        };
        let Some(bridge) = s.bridge_s else {
            return plan;
        };
        let wanted = position_at(
            timeline,
            server_at_stream(s.stream_now, offset, bridge, s.trim_ms),
        );
        plan.wanted_s = Some(wanted);
        if !timeline.playing {
            // Paused (or the host is loading): cut, and say where the room is so
            // a resume from here carries on from the right place.
            if !self.placed.is_empty() {
                plan.cut = true;
                self.placed.clear();
            }
            if let Some((_, media)) = self.sounding(s.stream_now) {
                plan.heard_s = Some(media);
            }
            return plan;
        }

        // Where we are, if a placement covers this instant.
        if let Some((placement, media)) = self.sounding(s.stream_now) {
            plan.heard_s = Some(media);
            let error = (media - wanted) * 1000.0;
            plan.error_ms = error;
            let correction = correction_for(error);
            plan.correction = Some(correction);
            match correction {
                Correction::Keep => {}
                // The next chunk seam absorbs it: the sounding chunk plays out,
                // the future is placed again on the corrected line.
                Correction::Seam => {
                    plan.clear_future = true;
                    self.placed.retain(|other| other.chunk <= placement.chunk);
                }
                // Behind: skip forward. Ahead: fall silent and resume the same
                // audio where the line is — never repeat what the room heard.
                Correction::Jump | Correction::Wait => {
                    plan.cut = true;
                    self.placed.clear();
                }
            }
        }

        // What to place next: the chunk the line is in, then the look-ahead.
        self.schedule(s, timeline, offset, bridge, &mut plan);
        plan
    }

    fn schedule(
        &mut self,
        s: &Situation<'_>,
        timeline: Timeline,
        offset: f64,
        bridge: f64,
        plan: &mut Plan,
    ) {
        let len = CHUNK_S;
        // What has played out is forgotten before the table is measured: the
        // mixer holds [`nz_audio::PARTY_SLOTS`] segments, and a schedule that
        // pushed a seventh would lose it at the next tick and place it again
        // (measured: the same chunk placed every 100 ms, and nothing left
        // covering the room).
        self.placed
            .retain(|placement| placement.from_s + CHUNK_S + OVERLAP_S > s.stream_now);
        let last = last_chunk(s.duration, len);
        let mut chunk = chunk_index(
            position_at(
                timeline,
                server_at_stream(s.stream_now, offset, bridge, s.trim_ms),
            ),
            len,
        );
        let horizon = chunk + LOOKAHEAD;
        while chunk <= horizon && self.placed.len() < nz_audio::PARTY_SLOTS {
            if let Some(last) = last
                && chunk > last
            {
                break;
            }
            if !(s.ready)(chunk) {
                chunk += 1;
                continue;
            }
            let already = self
                .placed
                .iter()
                .any(|placement| placement.chunk == chunk && placement.from_s.is_finite());
            if already {
                chunk += 1;
                continue;
            }
            // Placed only when it is close enough for the mapping to still be
            // right; the rest is fetched and waits in the cache.
            let due = to_stream(
                server_time_at(timeline, chunk as f64 * len),
                offset,
                bridge,
                s.trim_ms,
            );
            if due > s.stream_now + SCHED_AHEAD_S {
                break;
            }
            // Where the chunk's first sample belongs: the line's own instant for
            // its start. A chunk whose start is already past (a join mid-track, a
            // catch-up) starts at `now + LEAD` with the right number of frames
            // skipped, which is what makes joining cost one chunk and not a
            // restart.
            let start_media = chunk as f64 * len;
            let mut from_s = to_stream(
                server_time_at(timeline, start_media),
                offset,
                bridge,
                s.trim_ms,
            );
            let mut skip = 0.0;
            if from_s < s.stream_now + LEAD_S {
                from_s = s.stream_now + LEAD_S;
                let at = position_at(
                    timeline,
                    server_at_stream(from_s, offset, bridge, s.trim_ms),
                );
                skip = (at - start_media).max(0.0);
                if skip >= len + OVERLAP_S {
                    // This chunk is behind the room: the next one is what plays.
                    chunk += 1;
                    continue;
                }
            }
            let skip_frames = (skip * f64::from(s.rate)).round() as u64;
            let placement = Placement {
                chunk,
                from_s,
                skip_frames,
            };
            self.placed.push(placement);
            plan.place.push(placement);
            chunk += 1;
        }
        self.placed.sort_by(|a, b| a.from_s.total_cmp(&b.from_s));
    }
}

/// Whether anything is still placed (used only where a line is missing: the
/// streaming audio is left to finish rather than cut for no reason).
fn timeline_playing(placed: &[Placement], s: &Situation<'_>) -> bool {
    placed
        .iter()
        .any(|placement| placement.from_s + CHUNK_S > s.stream_now)
}
