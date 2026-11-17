//! Écoute partagée — the GUEST side, and the clock both sides share.
//!
//! Lot D15b (`docs/agents/desktop.md`). The contract is
//! `neezer/webui/party.py`, the reference behaviour is `webapp/src/lib/party/`
//! (`guest.js`, `engine.js`, `timeline.js`, `clock.js`), and the notes are
//! [party-remote.md](../../../../docs/agents/party-remote.md). A guest has no
//! account and no session: everything it touches is under `/api/party/<id>/`,
//! which only ever opens what the host is playing.
//!
//! Five rules shape this file, each one the SPA's, each one measured here:
//!
//! - **The host is the reference and the guest follows a LINE, never a stream.**
//!   The server publishes `{anchor: {t, p}}` — the media position `p` at server
//!   instant `t` — and everyone derives "where we should be" from it
//!   ([`position_at`]). Continuous playback therefore survives a missed poll:
//!   nothing is extrapolated twice, nothing drifts with the poll cadence.
//! - **Audio as 6 s chunks, scheduled on the DEVICE'S OWN clock.** A chunk is
//!   `[k·6 s, (k+1)·6 s + 50 ms)` of the track, cut by the server, decoded here,
//!   and handed to [`nz_audio::PartyChannel`] with the exact OUTPUT-stream
//!   position it must be heard at. That is the one number that makes a room
//!   sound like one room, and it comes from the device's `playback` timestamp
//!   plus the NTP offset ([`to_stream`]), never from a timer.
//! - **Corrections follow ONE rule** (`timeline.js#correctionFor`): ≤ 3 ms
//!   nothing, ≤ 30 ms at the next chunk seam, behind by more skip forward now,
//!   ahead by more fall silent and resume the same audio — never repeat what the
//!   room just heard. Past 2.5 s ahead it is not a stall but the host going BACK,
//!   and waiting it out was thirty seconds of silence after a rewind.
//! - **Rule 1: the sync traffic never costs the music a frame.** The clock
//!   probes, the state polls and the chunk fetches are all `Lane::User` — the
//!   lane that never waits — coalesced to one request in flight per use, the
//!   chunks fetched four ahead at most, and every call goes through `nz-api`'s
//!   gates rather than around them.
//! - **Rule 3: a flaky link fails fast and says so.** A poll that fails keeps
//!   the timeline it has (the music carries on, extrapolated), a chunk that is
//!   not on disk yet is retried at the delay the server asked for, and after six
//!   seconds without an answer the view says « Connexion perdue — la musique
//!   continue » instead of pretending.
//!
//! The estimators are [appcore::sync]'s — the same Rust the page runs
//! (`webapp/appcore/src/sync.rs`): `ClockEstimator` for the NTP offset and
//! `Bridge` for the map from that clock onto this device's output clock.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use appcore::sync::{Bridge, ClockEstimator};
use nz_api::{Api, ApiError, Lane};
use nz_audio::Segment;
use nz_model::{PartyState, PartyTrack};
use serde_json::Value;
use tracing::{debug, info};

// --- the SPA's own constants, ported -----------------------------------------

/// A guest asks where the party is about once a second…
pub const POLL_MS: u64 = 1_000;
/// …and four times faster near a predicted handover, or while the host loads a
/// track (`guest.js#POLL_FAST_MS`): that is where a poll of delay costs audio.
pub const POLL_FAST_MS: u64 = 350;
/// How close to the announced handover the fast cadence starts.
pub const HANDOVER_SOON_MS: f64 = 4_000.0;
/// No answer for this long: the view says the link is lost (`OFFLINE_AFTER`).
pub const OFFLINE_AFTER_MS: u64 = 6_000;
/// The scheduler's own beat: it re-reads the output clock and places what is
/// due. 100 ms is a sixth of a chunk and far below the correction window.
pub const TICK: Duration = Duration::from_millis(100);
/// How often the output-clock pair is read for the bridge: 50 ms, the SPA's
/// cadence, because a mapping that steps has to be SEEN fast (`engine.js`).
pub const SAMPLE: Duration = Duration::from_millis(50);
/// Slack given to a fresh placement: a render quantum plus main-thread jitter.
pub const LEAD_S: f64 = 0.1;
/// The chunk geometry the server cuts (`party.py#CHUNK_SECONDS`/`CHUNK_OVERLAP`).
pub const CHUNK_S: f64 = 6.0;
pub const OVERLAP_S: f64 = 0.05;
/// Chunks fetched ahead of the playhead: 4 × 6 s = 24 s of audio, and 4 requests
/// for a minute of music (the SPA's `FETCH_AHEAD`/`MAX_INFLIGHT` shape).
pub const LOOKAHEAD: u64 = 4;
/// At most two chunk requests in flight (`engine.js#MAX_INFLIGHT`).
pub const MAX_INFLIGHT: usize = 2;
/// How far ahead a chunk is PLACED on the output clock, seconds. The web player
/// schedules three seconds ahead (`engine.js#SCHED_AHEAD`) while it FETCHES twenty;
/// the two are separate on purpose. A placement is made against the current
/// mapping from this device's clock onto the output's, and that mapping drifts
/// with the sound card's own crystal: at 35 ppm, a chunk placed 30 s early lands
/// 1.05 ms off (measured, this file's ten-minute test), a chunk placed 7 s early
/// 0.25 ms — and every seam resets it anyway.
pub const SCHED_AHEAD_S: f64 = 7.0;
/// The correction thresholds, in milliseconds (`timeline.js`).
pub const KEEP_MS: f64 = 3.0;
pub const SEAM_MS: f64 = 30.0;
pub const WAIT_MAX_MS: f64 = 2_500.0;
/// Clock probes: a burst of twelve 40 ms apart at the start (enough for a
/// min-RTT pick straight away), then one every two seconds (`clock.js`).
pub const BURST: u32 = 12;
pub const BURST_GAP: Duration = Duration::from_millis(40);
pub const STEADY: Duration = Duration::from_millis(2_000);
/// How many agreeing probes an offset needs before anything is placed on it.
pub const MIN_PROBES: u32 = 4;
/// The store keys the SPA keeps for a listener (`guest.js`).
pub const NAME_KEY: &str = "party.name";
pub const VOLUME_KEY: &str = "party.volume";
pub const LATENCY_KEY: &str = "party.latency";

// --- the shared clock ---------------------------------------------------------

/// This process's own monotonic milliseconds, the domain every estimate and
/// every schedule is in. One origin for the whole process: two threads that
/// disagreed by a millisecond would disagree about the music.
pub fn now_ms() -> f64 {
    PROCESS_START.elapsed().as_secs_f64() * 1000.0
}

static PROCESS_START: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);

fn local_ms() -> f64 {
    now_ms()
}

/// The NTP-style estimate of the server's clock, from `GET
/// /api/party/<pid>/clock`, and the loop that keeps it fresh.
///
/// Both sides of a party need it: a guest to turn "the room is at 41.5 s" into a
/// device instant, a host to publish `t` in the server's own milliseconds.
pub struct ServerClock {
    estimator: Mutex<ClockEstimator>,
    probes: AtomicU64,
    stop: AtomicBool,
    /// The last estimate, for the screen (`spread`, `rtt`).
    last: Mutex<Option<(f64, f64)>>,
}

impl ServerClock {
    /// Probe `base` (the party's own `/clock` route) until [`Self::stop`].
    pub fn open(api: Arc<Api>, runtime: tokio::runtime::Handle, base: String) -> Arc<ServerClock> {
        let clock = Arc::new(ServerClock {
            estimator: Mutex::new(ClockEstimator::new(64, 180_000.0, 20_000.0)),
            probes: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            last: Mutex::new(None),
        });
        let this = Arc::clone(&clock);
        runtime.spawn(async move { this.run(api, base).await });
        clock
    }

    async fn run(&self, api: Arc<Api>, base: String) {
        let mut burst = BURST;
        while !self.stop.load(Ordering::Acquire) {
            let gap = if burst > 0 { BURST_GAP } else { STEADY };
            let t0 = local_ms();
            match api.get::<Value>(Lane::User, &format!("{base}/clock")).await {
                Ok(answer) => {
                    let t3 = local_ms();
                    let t1 = answer.value.get("t1").and_then(Value::as_f64);
                    let t2 = answer.value.get("t2").and_then(Value::as_f64);
                    if let (Some(t1), Some(t2)) = (t1, t2) {
                        let rtt = t3 - t0 - (t2 - t1);
                        let offset = ((t1 - t0) + (t2 - t3)) / 2.0;
                        let mut estimator = self.estimator.lock().expect("clock poisoned");
                        estimator.add(t3, rtt.max(0.0), offset);
                        if let Some(estimate) = estimator.estimate(local_ms()) {
                            *self.last.lock().expect("clock poisoned") =
                                Some((estimate.spread, estimate.rtt));
                        }
                    }
                    burst = burst.saturating_sub(1);
                    self.probes.fetch_add(1, Ordering::AcqRel);
                }
                Err(error) => {
                    // A 404 means the party is over: the poll loop is what says
                    // so, this one just keeps asking less often.
                    debug!(%error, "sonde d'horloge de l'écoute partagée");
                    burst = 0;
                    tokio::time::sleep(STEADY * 2).await;
                    continue;
                }
            }
            tokio::time::sleep(gap).await;
        }
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }

    /// The offset `server − local` in milliseconds, or `None` before enough
    /// probes agree (`guest.js` refuses to place audio on fewer than four).
    pub fn offset_ms(&self) -> Option<f64> {
        if self.probes.load(Ordering::Acquire) < u64::from(MIN_PROBES) {
            return None;
        }
        let mut estimator = self.estimator.lock().expect("clock poisoned");
        estimator
            .estimate(local_ms())
            .map(|estimate| estimate.offset)
    }

    /// `(spread, rtt)` in milliseconds, for the panel's « Calé sur … · ±x ms ».
    pub fn quality(&self) -> Option<(f64, f64)> {
        *self.last.lock().expect("clock poisoned")
    }

    pub fn probes(&self) -> u64 {
        self.probes.load(Ordering::Acquire)
    }

    pub fn reset(&self) {
        self.probes.store(0, Ordering::Release);
        self.estimator.lock().expect("clock poisoned").reset();
    }
}

// --- the timeline (pure) ------------------------------------------------------

/// `{t, p, playing}`: the media position `p` at server instant `t`, advancing in
/// real time while `playing` (`timeline.js`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Timeline {
    pub t: f64,
    pub p: f64,
    pub playing: bool,
}

pub fn position_at(tl: Timeline, server_ms: f64) -> f64 {
    if tl.playing {
        tl.p + (server_ms - tl.t) / 1000.0
    } else {
        tl.p
    }
}

pub fn server_time_at(tl: Timeline, pos: f64) -> f64 {
    tl.t + (pos - tl.p) * 1000.0
}

/// The chunk that holds `pos` (`chunk k` covers `[k·L, k·L + L + ov)`).
pub fn chunk_index(pos: f64, len: f64) -> u64 {
    (pos / len + 1e-9).floor().max(0.0) as u64
}

/// The last chunk of a track of `duration` seconds; `None` when the duration is
/// unknown (`timeline.js#lastChunk` returns Infinity, which no loop can use).
pub fn last_chunk(duration: f64, len: f64) -> Option<u64> {
    if duration > 0.0 {
        Some(((duration + 1.0) / len).ceil().max(0.0) as u64 - 1)
    } else {
        None
    }
}

/// What to do about being `err` ms off (`err` = where the guest IS minus where
/// it should be).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Correction {
    Keep,
    Seam,
    Jump,
    Wait,
}

pub fn correction_for(err_ms: f64) -> Correction {
    let magnitude = err_ms.abs();
    if magnitude <= KEEP_MS {
        Correction::Keep
    } else if magnitude <= SEAM_MS {
        Correction::Seam
    } else if err_ms < 0.0 || err_ms > WAIT_MAX_MS {
        Correction::Jump
    } else {
        Correction::Wait
    }
}

/// Where the output stream has to be for the room to hear server instant
/// `server_ms`: the NTP offset turns it into this device's milliseconds, and the
/// bridge's own median (`d` = output clock − local clock, seconds) into the
/// stream the mixer writes. The listener's own trim (a Bluetooth speaker that
/// does not report its delay) plays the audio EARLIER by that much.
pub fn to_stream(server_ms: f64, offset_ms: f64, bridge_s: f64, trim_ms: f64) -> f64 {
    (server_ms - offset_ms) / 1000.0 + bridge_s - trim_ms / 1000.0
}

/// The inverse, for the planner: the server instant an output-stream position
/// is heard at.
pub fn server_at_stream(stream_s: f64, offset_ms: f64, bridge_s: f64, trim_ms: f64) -> f64 {
    (stream_s + trim_ms / 1000.0 - bridge_s) * 1000.0 + offset_ms
}

// --- what the server publishes (nz-model's own types) ------------------------

/// The line a guest follows, from the state the server published: the anchor is
/// an OBJECT (`{"t", "p"}`), never flat fields — the shape `nz-model` used to
/// get wrong and D15b had to read as raw JSON.
pub fn timeline_of(state: &PartyState) -> Option<Timeline> {
    let track = state.track.as_ref()?;
    let anchor = state.anchor.as_ref()?;
    if track.id.is_empty() || !anchor.t.is_finite() || !anchor.p.is_finite() {
        return None;
    }
    Some(Timeline {
        t: anchor.t,
        p: anchor.p,
        playing: state.playing,
    })
}

fn track_id(state: &PartyState) -> Option<&str> {
    state.track.as_ref().map(|track| track.id.as_str())
}

/// The normalisation gain of one track, exactly as the host's graph computes it
/// (`engine.js#normGain`): Deezer's loudness, the SPA's transform, the level's
/// offset, clamped. `1.0` when the host is not normalising.
pub fn norm_gain(gain: Option<f64>, level: &str) -> f32 {
    let level = level.trim();
    let offset = match level {
        "low" => 2.0,
        "medium" => 5.0,
        "high" => 8.0,
        _ => return 1.0,
    };
    let Some(gain) = gain else {
        return 1.0;
    };
    if !gain.is_finite() {
        return 1.0;
    }
    let db = (-(gain + 18.4) + offset).clamp(-24.0, 12.0);
    10f64.powf(db / 20.0) as f32
}

// --- the planner (pure, testable) ---------------------------------------------

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

/// The guest's scheduling brain: pure, so the ten-minute drift and the recovery
/// after a cut are assertions instead of a stopwatch.
///
/// It remembers the placements it handed over (their chunk, where they start on
/// the output stream, and how far into the chunk they begin), which is all it
/// needs to say WHERE this device currently is: the output stream's position
/// alone does not know which chunk is sounding there.
#[derive(Debug, Default)]
pub struct Planner {
    placed: Vec<Placement>,
    rate: u32,
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

    /// The placement sounding at `stream_s`, with how far into its media it is.
    ///
    /// The NEWEST one wins: consecutive chunks overlap by the server's 50 ms and
    /// the mixer crossfades them, so after a seam the room hears the new chunk —
    /// reporting the old one's line would measure the previous chunk's drift for
    /// ever (measured: 0.67 ms after 19 s instead of the seam's own zero).
    fn sounding(&self, stream_s: f64) -> Option<(Placement, f64)> {
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
// --- the service --------------------------------------------------------------

/// Where a guest stands, for the screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    Joining,
    Live,
    /// The host ended it (or the server forgot it).
    Ended,
    /// The listener left.
    Left,
}

/// What the screen paints: a clone, under a mutex, with a version so a repaint
/// happens once per change.
#[derive(Clone, Debug, Default)]
pub struct GuestView {
    pub phase: Phase,
    pub host: String,
    pub track: Option<PartyTrack>,
    pub next: Option<PartyTrack>,
    pub listeners: usize,
    pub live: bool,
    pub playing: bool,
    pub loading: bool,
    /// The party's own playhead, seconds.
    pub position_s: f64,
    pub duration_s: f64,
    pub status: Status,
    pub clock: Option<(f64, f64)>,
    pub offline: bool,
    pub volume: f64,
    pub trim_ms: f64,
    pub os_latency_ms: Option<i64>,
    pub error: Option<String>,
    pub version: u64,
}

/// The green/amber line under the playhead (`Party.svelte#statusLine`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Idle,
    Synced,
    Loading,
    Paused,
}

impl GuestView {
    /// Which line the screen prints, and the tone it prints it in.
    pub fn status_line(&self) -> (Tone, String) {
        if let Some(error) = &self.error {
            return (Tone::Error, error.clone());
        }
        if self.offline {
            return (
                Tone::Warn,
                "Connexion perdue — la musique continue".to_owned(),
            );
        }
        if !self.live {
            return (
                Tone::Warn,
                if self.host.is_empty() {
                    "L'hôte s'est absenté".to_owned()
                } else {
                    format!("{} s'est absenté", self.host)
                },
            );
        }
        if self.track.is_none() {
            return (
                Tone::Wait,
                if self.host.is_empty() {
                    "En attente de l'hôte".to_owned()
                } else {
                    format!("En attente de {}", self.host)
                },
            );
        }
        if !self.playing {
            return (
                Tone::Wait,
                if self.loading {
                    format!("Chargement chez {}…", self.host)
                } else {
                    format!("En pause chez {}", self.host)
                },
            );
        }
        match self.clock {
            None => (Tone::Wait, "Synchronisation de l'horloge…".to_owned()),
            Some((spread, _)) => {
                if self.status == Status::Loading {
                    (Tone::Wait, "Chargement…".to_owned())
                } else if self.status == Status::Synced {
                    (
                        Tone::Ok,
                        format!("Calé sur {} · ±{:.1} ms", self.host, spread.max(0.1)),
                    )
                } else {
                    (Tone::Wait, "Mise en place…".to_owned())
                }
            }
        }
    }
}

/// The colour of a status line, the screen's own vocabulary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Wait,
    Ok,
    Warn,
    Error,
}

struct Shared {
    view: Mutex<GuestView>,
    api: Mutex<Option<Arc<Api>>>,
    runtime: Mutex<Option<tokio::runtime::Handle>>,
    clock: Mutex<Option<Arc<ServerClock>>>,
    /// `/party/<pid>` and the party's id.
    base: Mutex<String>,
    pid: Mutex<String>,
    /// The listener's own seat, and the name the host sees.
    lid: Mutex<Option<String>>,
    name: Mutex<String>,
    timeline: Mutex<Option<Timeline>>,
    state: Mutex<Option<PartyState>>,
    /// When the last poll ANSWERED (the offline verdict counts from here).
    answered_at: Mutex<Option<Instant>>,
    /// The decoded chunks of the current track, by index.
    chunks: Mutex<HashMap<u64, Arc<[f32]>>>,
    /// Every buffer handed to the mixer lately: the callback drops its own
    /// copy, and this keeps the allocation alive until the guest's thread lets
    /// it go — the callback never frees (see `nz-audio`'s `party.rs`).
    keep: Mutex<std::collections::VecDeque<Arc<[f32]>>>,
    /// The track the cache belongs to: a new track starts empty.
    chunks_of: Mutex<String>,
    /// A chunk the server said it is preparing, and when to ask again.
    retry_at: Mutex<HashMap<u64, Instant>>,
    inflight: AtomicUsize,
    /// The output clock's own rate, and the party's volume and trim.
    rate: AtomicU64,
    volume: Mutex<f64>,
    trim_ms: Mutex<f64>,
    engine: Mutex<Option<nz_audio::PartyChannel>>,
    /// The scheduler thread: OWNED and joined by [`Guest::stop`], like every
    /// other thread this app starts.
    scheduler: Mutex<Option<std::thread::JoinHandle<()>>>,
    stop: Arc<AtomicBool>,
    version: AtomicU64,
}

static SERVICE: std::sync::OnceLock<Guest> = std::sync::OnceLock::new();

/// The guest session the process is running, if one was started. Cheap to clone.
#[derive(Clone)]
pub struct Guest {
    shared: Arc<Shared>,
}

impl Guest {
    pub fn get() -> Option<Guest> {
        SERVICE.get().cloned()
    }

    /// The party this device is listening to, when it is listening to one.
    pub fn pid(&self) -> String {
        self.shared.pid.lock().expect("party poisoned").clone()
    }

    pub fn view(&self) -> GuestView {
        self.shared.view.lock().expect("party poisoned").clone()
    }

    /// Where the room is RIGHT NOW, seconds into the track: the line the room
    /// follows, read against this device's own clock estimate. This is the
    /// playhead the screen paints, and it moves between two polls — the poll is
    /// only what re-anchors the line.
    pub fn position_now(&self) -> f64 {
        let timeline = *self.shared.timeline.lock().expect("party poisoned");
        let Some(timeline) = timeline else {
            return 0.0;
        };
        let offset = self
            .shared
            .clock
            .lock()
            .expect("party poisoned")
            .as_ref()
            .and_then(|clock| clock.offset_ms());
        let now = offset.map(|offset| local_ms() + offset);
        now.map(|now| position_at(timeline, now).max(0.0))
            .unwrap_or(timeline.p)
    }

    /// The clock probes' own count, for the panel and the tests.
    pub fn probes(&self) -> u64 {
        self.shared
            .clock
            .lock()
            .expect("party poisoned")
            .as_ref()
            .map(|clock| clock.probes())
            .unwrap_or(0)
    }

    /// `GET /api/party/<pid>` without joining: the landing card's peek. `None`
    /// when the party does not exist (ended, or never did) — not an error.
    pub async fn peek(api: &Api, pid: &str) -> Result<Option<PartyState>, ApiError> {
        match api.party_state(pid, None, None, None).await {
            Ok(state) => Ok(Some(state)),
            Err(ApiError::Http { status: 404, .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// `POST /api/party/<pid>/join` — take a seat, unless this device already
    /// holds one. Returns the listener id.
    pub async fn join(api: &Api, pid: &str, name: &str) -> Result<String, ApiError> {
        let body = serde_json::json!({ "name": name });
        let answer: Option<Value> = api
            .send(
                Lane::User,
                reqwest::Method::POST,
                &format!("/party/{}/join", path_segment(pid)),
                Some(&body),
            )
            .await?;
        Ok(answer
            .and_then(|value| value.get("lid").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_default())
    }

    /// Start (or restart) the session: the clock probes, the poll loop and the
    /// scheduler thread. `engine` is `None` on a machine with no output, which is
    /// the "no sound, but say why" case instead of a panic.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        api: Arc<Api>,
        runtime: tokio::runtime::Handle,
        pid: String,
        name: String,
        lid: Option<String>,
        engine: Option<nz_audio::PartyChannel>,
        rate: u32,
        volume: f64,
        trim_ms: f64,
    ) -> Guest {
        let service = SERVICE.get_or_init(|| Guest {
            shared: Arc::new(Shared {
                view: Mutex::new(GuestView::default()),
                api: Mutex::new(None),
                runtime: Mutex::new(None),
                clock: Mutex::new(None),
                base: Mutex::new(String::new()),
                pid: Mutex::new(String::new()),
                lid: Mutex::new(None),
                name: Mutex::new(String::new()),
                timeline: Mutex::new(None),
                state: Mutex::new(None),
                answered_at: Mutex::new(None),
                chunks: Mutex::new(HashMap::new()),
                keep: Mutex::new(std::collections::VecDeque::new()),
                chunks_of: Mutex::new(String::new()),
                retry_at: Mutex::new(HashMap::new()),
                inflight: AtomicUsize::new(0),
                rate: AtomicU64::new(48_000),
                volume: Mutex::new(1.0),
                trim_ms: Mutex::new(0.0),
                engine: Mutex::new(None),
                scheduler: Mutex::new(None),
                stop: Arc::new(AtomicBool::new(false)),
                version: AtomicU64::new(1),
            }),
        });
        // A second join (another party, or the same one after leaving): the old
        // loops stop first, so two never publish into one device.
        service.stop_threads();
        let base = format!("/party/{}", path_segment(&pid));
        *service.shared.api.lock().expect("party poisoned") = Some(Arc::clone(&api));
        *service.shared.runtime.lock().expect("party poisoned") = Some(runtime.clone());
        *service.shared.base.lock().expect("party poisoned") = base.clone();
        *service.shared.pid.lock().expect("party poisoned") = pid.clone();
        *service.shared.name.lock().expect("party poisoned") = name.clone();
        *service.shared.lid.lock().expect("party poisoned") = lid.clone();
        *service.shared.engine.lock().expect("party poisoned") = engine;
        *service.shared.volume.lock().expect("party poisoned") = volume.clamp(0.0, 1.0);
        *service.shared.trim_ms.lock().expect("party poisoned") = trim_ms;
        service
            .shared
            .rate
            .store(u64::from(rate.max(1)), Ordering::Release);
        service.shared.inflight.store(0, Ordering::Release);
        service
            .shared
            .chunks
            .lock()
            .expect("party poisoned")
            .clear();
        *service.shared.chunks_of.lock().expect("party poisoned") = String::new();
        service
            .shared
            .retry_at
            .lock()
            .expect("party poisoned")
            .clear();
        *service.shared.timeline.lock().expect("party poisoned") = None;
        *service.shared.state.lock().expect("party poisoned") = None;
        *service.shared.answered_at.lock().expect("party poisoned") = None;
        service.shared.stop.store(false, Ordering::Release);
        {
            let mut view = service.shared.view.lock().expect("party poisoned");
            *view = GuestView {
                phase: Phase::Joining,
                volume: volume.clamp(0.0, 1.0),
                trim_ms,
                ..GuestView::default()
            };
        }
        if let Some(engine) = service
            .shared
            .engine
            .lock()
            .expect("party poisoned")
            .clone()
        {
            engine.take_over(true);
            engine.set_gain((volume * volume).clamp(0.0, 1.0) as f32);
        }
        let clock = ServerClock::open(Arc::clone(&api), runtime.clone(), base.clone());
        if let Some(old) = service
            .shared
            .clock
            .lock()
            .expect("party poisoned")
            .replace(clock)
        {
            old.stop();
        }
        // The clock probes and the poll loop are tasks (the runtime owns them);
        // the scheduler is a thread, because it reads the output clock every
        // 100 ms and owns the bridge's mutable state.
        {
            let this = service.clone();
            let poll_runtime = runtime.clone();
            runtime.spawn(async move { this.poll_loop(api, poll_runtime).await });
        }
        let this = service.clone();
        let handle = std::thread::Builder::new()
            .name("nz-party".into())
            .stack_size(256 * 1024)
            .spawn(move || this.scheduler_loop());
        if let Ok(handle) = handle {
            *service.shared.scheduler.lock().expect("party poisoned") = Some(handle);
        }
        info!(pid = %service.pid(), "écoute partagée : invité");
        service.clone()
    }

    /// Stop listening: tell the host, cut the sound, and join the scheduler.
    pub fn leave(&self) {
        let api = self.shared.api.lock().expect("party poisoned").clone();
        let runtime = self.shared.runtime.lock().expect("party poisoned").clone();
        let lid = self.shared.lid.lock().expect("party poisoned").clone();
        let pid = self.pid();
        if let (Some(api), Some(runtime), Some(lid), false) = (api, runtime, lid, pid.is_empty()) {
            let base = self.shared.base.lock().expect("party poisoned").clone();
            runtime.spawn(async move {
                let body = serde_json::json!({ "lid": lid });
                let _ = api
                    .send::<Value, Value>(
                        Lane::User,
                        reqwest::Method::POST,
                        &format!("{base}/leave"),
                        Some(&body),
                    )
                    .await;
            });
        }
        self.stop_threads();
        self.set_phase(Phase::Left);
    }

    /// Give the output back and stop every loop, without a request (the host
    /// ended the party, or the app is closing).
    pub fn stop(&self) {
        self.stop_threads();
        self.set_phase(Phase::Ended);
    }

    pub fn set_volume(&self, volume: f64) {
        let volume = volume.clamp(0.0, 1.0);
        *self.shared.volume.lock().expect("party poisoned") = volume;
        if let Some(engine) = self.shared.engine.lock().expect("party poisoned").clone() {
            engine.set_gain((volume * volume) as f32);
        }
        self.touch(|view| view.volume = volume);
    }

    /// This device's own output delay (a Bluetooth speaker that does not report
    /// it): positive plays earlier, so the room lines up.
    pub fn set_trim(&self, ms: f64) {
        let ms = ms.clamp(-500.0, 1000.0).round();
        *self.shared.trim_ms.lock().expect("party poisoned") = ms;
        self.touch(|view| view.trim_ms = ms);
    }

    pub fn trim(&self) -> f64 {
        *self.shared.trim_ms.lock().expect("party poisoned")
    }

    pub fn volume(&self) -> f64 {
        *self.shared.volume.lock().expect("party poisoned")
    }

    fn stop_threads(&self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(clock) = self
            .shared
            .clock
            .lock()
            .expect("party poisoned")
            .as_ref()
            .cloned()
        {
            clock.stop();
        }
        if let Some(engine) = self.shared.engine.lock().expect("party poisoned").clone() {
            engine.close();
        }
        if let Some(handle) = self.shared.scheduler.lock().expect("party poisoned").take() {
            let _ = handle.join();
        }
    }

    fn set_phase(&self, phase: Phase) {
        self.touch(|view| {
            view.phase = phase;
            view.status = Status::Idle;
        });
    }

    /// Mutate the view and bump its version, so the screen repaints once.
    fn touch(&self, change: impl FnOnce(&mut GuestView)) {
        let mut view = self.shared.view.lock().expect("party poisoned");
        change(&mut view);
        self.shared.version.fetch_add(1, Ordering::AcqRel);
        view.version = self.shared.version.load(Ordering::Acquire);
    }

    fn stopped(&self) -> bool {
        self.shared.stop.load(Ordering::Acquire)
    }

    // --- the network side ----------------------------------------------------

    /// Ask where the party is (about once a second, four times faster near a
    /// handover), feed the planner, fetch the chunks that are coming.
    async fn poll_loop(&self, api: Arc<Api>, runtime: tokio::runtime::Handle) {
        let mut failures = 0_u32;
        while !self.stopped() {
            let started = Instant::now();
            let pid = self.pid();
            let lid = self.shared.lid.lock().expect("party poisoned").clone();
            let quality = self
                .shared
                .clock
                .lock()
                .expect("party poisoned")
                .as_ref()
                .and_then(|clock| clock.quality())
                .map(|(spread, _)| spread.max(0.0).round() as u32);
            let status = view_status(&self.shared.view);
            let mut next = POLL_MS;
            match api
                .party_state(&pid, lid.as_deref(), quality, Some(status))
                .await
            {
                Ok(state) => {
                    failures = 0;
                    *self.shared.answered_at.lock().expect("party poisoned") = Some(Instant::now());
                    // A seat the server dropped (a device that went quiet) is
                    // taken again instead of listening on unseen.
                    if state.me == Some(false) {
                        self.rejoin(&api).await;
                    }
                    self.adopt(&state);
                    if state.buf {
                        next = POLL_FAST_MS;
                    } else if let Some(handover) = handover_in_ms(&state, &self.shared)
                        && (-2_000.0..HANDOVER_SOON_MS).contains(&handover)
                    {
                        next = POLL_FAST_MS;
                    }
                    self.prefetch(&api, &runtime, &state);
                }
                Err(ApiError::Http { status: 404, .. }) => {
                    // Over (or never there): let the music stop as it would at
                    // home, and say which of the two it was.
                    self.stop_threads();
                    self.touch(|view| {
                        view.phase = Phase::Ended;
                        view.status = Status::Idle;
                    });
                    return;
                }
                Err(error) => {
                    failures += 1;
                    let offline = self
                        .shared
                        .answered_at
                        .lock()
                        .expect("party poisoned")
                        .is_none_or(|at| at.elapsed() >= Duration::from_millis(OFFLINE_AFTER_MS));
                    if offline {
                        self.touch(|view| view.offline = true);
                    }
                    debug!(%error, "interrogation de l'écoute partagée");
                    // Fail fast, then back off: a poll storm on a flaky link is
                    // rule 3's own prohibition, and the timeline in hand keeps
                    // the music going meanwhile.
                    tokio::time::sleep(backoff(failures)).await;
                    continue;
                }
            }
            let elapsed = started.elapsed();
            tokio::time::sleep(Duration::from_millis(next).saturating_sub(elapsed)).await;
        }
    }

    /// Take a seat again, after the server forgot this one.
    async fn rejoin(&self, api: &Arc<Api>) {
        let pid = self.pid();
        let name = self.shared.name.lock().expect("party poisoned").clone();
        if let Ok(lid) = Guest::join(api, &pid, &name).await
            && !lid.is_empty()
        {
            *self.shared.lid.lock().expect("party poisoned") = Some(lid);
        }
    }

    /// Adopt one poll's answer: the timeline for the planner, the rest for the
    /// screen.
    fn adopt(&self, state: &PartyState) {
        let timeline = timeline_of(state);
        *self.shared.timeline.lock().expect("party poisoned") = timeline;
        let track_changed = {
            let previous = self.shared.state.lock().expect("party poisoned").clone();
            previous.as_ref().and_then(track_id) != track_id(state)
        };
        if track_changed {
            // A new track: the chunks of the old one are not what anyone plays.
            self.shared.chunks.lock().expect("party poisoned").clear();
            self.shared.retry_at.lock().expect("party poisoned").clear();
            *self.shared.chunks_of.lock().expect("party poisoned") =
                track_id(state).unwrap_or_default().to_owned();
        }
        *self.shared.state.lock().expect("party poisoned") = Some(state.clone());
        let clock = self
            .shared
            .clock
            .lock()
            .expect("party poisoned")
            .as_ref()
            .and_then(|clock| clock.quality());
        let listeners = state.listeners.len();
        let position_s = match (timeline_of(state), clock) {
            (Some(timeline), _) => {
                let now = self
                    .shared
                    .clock
                    .lock()
                    .expect("party poisoned")
                    .as_ref()
                    .and_then(|clock| clock.offset_ms())
                    .map(|offset| local_ms() + offset)
                    .unwrap_or(state.now);
                position_at(timeline, now)
            }
            _ => 0.0,
        };
        let duration = state
            .track
            .as_ref()
            .map(|track| track.duration)
            .unwrap_or(0.0);
        self.touch(|view| {
            view.phase = if view.phase == Phase::Joining {
                Phase::Live
            } else {
                view.phase
            };
            view.host = state.host.clone();
            view.track = state.track.clone();
            view.next = state.next.as_ref().and_then(|next| next.track.clone());
            view.listeners = listeners;
            view.live = state.live;
            view.playing = state.playing;
            view.loading = state.buf;
            view.position_s =
                position_s.clamp(0.0, if duration > 0.0 { duration } else { f64::MAX });
            view.duration_s = duration;
            view.clock = clock;
            view.offline = false;
            view.error = None;
        });
    }

    /// Fetch the chunks the planner is about to need: four ahead, two in
    /// flight, and nothing the server said it is still cutting.
    fn prefetch(&self, api: &Arc<Api>, runtime: &tokio::runtime::Handle, state: &PartyState) {
        let Some(track) = state.track.as_ref() else {
            return;
        };
        let Some(timeline) = timeline_of(state) else {
            return;
        };
        let now = self
            .shared
            .clock
            .lock()
            .expect("party poisoned")
            .as_ref()
            .and_then(|clock| clock.offset_ms())
            .map(|offset| local_ms() + offset)
            .unwrap_or(state.now);
        let position = position_at(timeline, now);
        let first = chunk_index(position.max(0.0), CHUNK_S);
        let last = match last_chunk(track.duration, CHUNK_S) {
            Some(last) => last,
            None => first + LOOKAHEAD,
        };
        let mut started = 0;
        for chunk in first..=last.min(first + LOOKAHEAD) {
            if self.shared.inflight.load(Ordering::Acquire) + started >= MAX_INFLIGHT {
                break;
            }
            if self
                .shared
                .chunks
                .lock()
                .expect("party poisoned")
                .contains_key(&chunk)
            {
                continue;
            }
            let retry = self
                .shared
                .retry_at
                .lock()
                .expect("party poisoned")
                .get(&chunk)
                .copied();
            if retry.is_some_and(|at| at > Instant::now()) {
                continue;
            }
            let path = format!(
                "/party/{}/chunk/{}?f=opus&sr=48000",
                path_segment(&self.pid()),
                path_segment(&track.id)
            );
            let path = format!("{path}/{chunk}");
            let api = Arc::clone(api);
            let this = self.clone();
            let rate = self.shared.rate.load(Ordering::Acquire) as u32;
            self.shared.inflight.fetch_add(1, Ordering::AcqRel);
            started += 1;
            runtime.spawn(async move {
                let answer = api.bytes(Lane::User, &path).await;
                let this2 = this.clone();
                let decoded = match answer {
                    Ok(bytes) => tokio::task::spawn_blocking(move || decode_chunk(&bytes, rate))
                        .await
                        .unwrap_or_else(|error| Err(format!("décodage interrompu : {error}"))),
                    Err(ApiError::Http { status: 404, .. }) => {
                        // Past the end, or no longer in the party: nothing will
                        // ever bring this chunk back.
                        Err("fin de la piste".to_owned())
                    }
                    Err(error) => Err(error.cause_fr()),
                };
                this2.finish_chunk(chunk, decoded);
            });
        }
    }

    fn finish_chunk(&self, chunk: u64, decoded: Result<Arc<[f32]>, String>) {
        self.shared.inflight.fetch_sub(1, Ordering::AcqRel);
        match decoded {
            Ok(pcm) if !pcm.is_empty() => {
                self.shared
                    .chunks
                    .lock()
                    .expect("party poisoned")
                    .insert(chunk, pcm);
            }
            Ok(_) => {}
            Err(cause) => {
                debug!(chunk, cause, "chunk d'écoute partagée indisponible");
                // Two seconds, the server's own `Retry-After` for a chunk it is
                // still cutting; the planner asks again when it is due.
                self.shared
                    .retry_at
                    .lock()
                    .expect("party poisoned")
                    .insert(chunk, Instant::now() + Duration::from_secs(2));
            }
        }
    }

    // --- the scheduler -------------------------------------------------------

    /// The output's own beat: read the device clock into the bridge, ask the
    /// planner, hand the chunks to the mixer. 100 ms, and it never touches GPUI.
    fn scheduler_loop(&self) {
        let mut bridge = Bridge::new();
        let rate = self.shared.rate.load(Ordering::Acquire) as u32;
        let mut planner = Planner::new(rate);
        let mut last_track = String::new();
        while !self.stopped() {
            std::thread::sleep(TICK);
            if self.stopped() {
                break;
            }
            let Some(engine) = self.shared.engine.lock().expect("party poisoned").clone() else {
                continue;
            };
            let clock = engine.output_clock();
            let local = local_ms();
            bridge.offer(clock.at_s - local / 1000.0, clock.at_s, local);
            let bridge_s = if bridge.ready() {
                let median = bridge.median();
                if median.is_finite() {
                    Some(median)
                } else {
                    None
                }
            } else {
                None
            };
            let (timeline, state) = {
                let timeline = *self.shared.timeline.lock().expect("party poisoned");
                let state = self.shared.state.lock().expect("party poisoned").clone();
                (timeline, state)
            };
            let track_id = state
                .as_ref()
                .and_then(track_id)
                .unwrap_or_default()
                .to_owned();
            if track_id != last_track {
                planner.forget();
                last_track = track_id;
            }
            let offset = self
                .shared
                .clock
                .lock()
                .expect("party poisoned")
                .as_ref()
                .and_then(|clock| clock.offset_ms());
            let trim = *self.shared.trim_ms.lock().expect("party poisoned");
            let duration = state
                .as_ref()
                .and_then(|state| state.track.as_ref())
                .map(|track| track.duration)
                .unwrap_or(0.0);
            // Where the DEVICE is now: the anchor, extrapolated by the bridge's
            // own median rather than the instantaneous reading (a stepped
            // mapping must not move the extrapolation).
            let stream_now = match bridge_s {
                Some(bridge_s) => clock.position_s + ((local / 1000.0 + bridge_s) - clock.at_s),
                None => clock.position_s,
            };
            let ready = |chunk: u64| {
                self.shared
                    .chunks
                    .lock()
                    .expect("party poisoned")
                    .contains_key(&chunk)
            };
            let answered = *self.shared.answered_at.lock().expect("party poisoned");
            let situation = Situation {
                timeline,
                offset_ms: offset,
                bridge_s,
                stream_now,
                rate,
                trim_ms: trim,
                since_poll_ms: answered
                    .map(|at| at.elapsed().as_millis() as u64)
                    .unwrap_or(0),
                duration,
                ready: &ready,
            };
            let plan = planner.plan(&situation);
            if plan.cut {
                engine.clear();
            } else if plan.clear_future {
                engine.clear_future();
            }
            let gain = state
                .as_ref()
                .map(|state| {
                    norm_gain(
                        state.track.as_ref().and_then(|track| track.gain),
                        &state.norm,
                    )
                })
                .unwrap_or(1.0);
            for placement in &plan.place {
                let pcm = self
                    .shared
                    .chunks
                    .lock()
                    .expect("party poisoned")
                    .get(&placement.chunk)
                    .cloned();
                let Some(pcm) = pcm else { continue };
                let skip = (placement.skip_frames as usize).min(pcm.len() / 2);
                let data: Arc<[f32]> = pcm[skip * 2..].to_vec().into();
                if data.is_empty() {
                    continue;
                }
                let mut segment = Segment::at(placement.from_s, f64::from(rate), Arc::clone(&data));
                segment.gain = gain;
                engine.schedule(segment);
                // The caller keeps the buffer alive: the callback's drop is a
                // refcount decrement, the free happens here (party-remote.md's
                // rule for `nz-audio`'s callback).
                self.shared
                    .keep
                    .lock()
                    .expect("party poisoned")
                    .push_back(data);
                while self.shared.keep.lock().expect("party poisoned").len() > 32 {
                    self.shared.keep.lock().expect("party poisoned").pop_front();
                }
            }
            let status = match plan.correction {
                _ if plan.wanted_s.is_none() => Status::Idle,
                _ if plan.heard_s.is_none() => Status::Loading,
                Some(Correction::Keep) => Status::Synced,
                Some(Correction::Seam) => Status::Synced,
                Some(_) => Status::Loading,
                None if !timeline.map(|tl| tl.playing).unwrap_or(false) => Status::Paused,
                None => Status::Loading,
            };
            let changed = {
                let mut view = self.shared.view.lock().expect("party poisoned");
                let changed =
                    view.status != status || view.os_latency_ms != Some(latency_ms(clock));
                view.status = status;
                view.os_latency_ms = Some(latency_ms(clock));
                changed
            };
            if changed {
                self.shared.version.fetch_add(1, Ordering::AcqRel);
            }
        }
    }
}

fn view_status(view: &Mutex<GuestView>) -> &'static str {
    match view.lock().expect("party poisoned").status {
        Status::Synced => "sync",
        Status::Loading => "load",
        Status::Paused => "pause",
        Status::Idle => "idle",
    }
}

fn latency_ms(clock: nz_audio::OutputClock) -> i64 {
    (clock.latency_s * 1000.0).round() as i64
}

/// Milliseconds until the announced handover, when the party announced one.
fn handover_in_ms(state: &PartyState, shared: &Shared) -> Option<f64> {
    let timeline = timeline_of(state)?;
    let next = state.next.as_ref()?;
    let clock = shared.clock.lock().expect("party poisoned").clone()?;
    let now = local_ms() + clock.offset_ms()?;
    let handover = server_time_at(timeline, next.at) + next.gap * 1000.0;
    Some(handover - now)
}

/// A poll's backoff: 1, 2, 4, 8 s, capped at 8 (`rule 3`: fail fast, no storm).
pub fn backoff(failures: u32) -> Duration {
    Duration::from_millis(1_000 * (1_u64 << failures.min(3)))
}

/// One path segment, percent-encoded the way the SPA's `encodeURIComponent`
/// does for the characters an id can carry.
fn path_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

// --- decoding a chunk ---------------------------------------------------------

/// Decode one Ogg Opus chunk the server cut into interleaved stereo f32 at
/// `rate` (the device's own), dropping the resampler's own delay so the chunk
/// keeps its length — the mixer places it by stream position, and a chunk one
/// millisecond short would open a hole at every seam.
///
/// Blocking, on the blocking pool: six seconds of Opus is a few milliseconds of
/// work, and it must never run on the UI thread or in the output callback.
pub fn decode_chunk(bytes: &[u8], rate: u32) -> Result<Arc<[f32]>, String> {
    use symphonia::core::codecs::audio::well_known::CODEC_ID_OPUS;
    use symphonia::core::codecs::audio::{AudioDecoderOptions, CODEC_ID_NULL_AUDIO};
    use symphonia::core::codecs::registry::CodecRegistry;
    use symphonia::core::formats::probe::{Hint, Probe};
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
    use symphonia::core::meta::MetadataOptions;

    struct Bytes<'a>(&'a [u8], usize);
    impl std::io::Read for Bytes<'_> {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let take = out.len().min(self.0.len() - self.1);
            out[..take].copy_from_slice(&self.0[self.1..self.1 + take]);
            self.1 += take;
            Ok(take)
        }
    }
    impl std::io::Seek for Bytes<'_> {
        fn seek(&mut self, from: std::io::SeekFrom) -> std::io::Result<u64> {
            let base = match from {
                std::io::SeekFrom::Start(0) => 0,
                std::io::SeekFrom::Current(0) => self.1,
                std::io::SeekFrom::End(0) => self.0.len(),
                _ => return Err(std::io::Error::other("chunk non déplaçable")),
            };
            self.1 = base;
            Ok(base as u64)
        }
    }
    impl symphonia::core::io::MediaSource for Bytes<'_> {
        fn is_seekable(&self) -> bool {
            false
        }
        fn byte_len(&self) -> Option<u64> {
            Some(self.0.len() as u64)
        }
    }

    let mut registry = CodecRegistry::new();
    symphonia::default::register_enabled_codecs(&mut registry);
    if registry.get_audio_decoder(CODEC_ID_OPUS).is_none() {
        registry.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
    }
    let mut probe = Probe::new();
    symphonia::default::register_enabled_formats(&mut probe);
    let stream = MediaSourceStream::new(
        Box::new(Bytes(bytes, 0)),
        MediaSourceStreamOptions::default(),
    );
    let format = probe
        .probe(
            &Hint::new(),
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|error| format!("conteneur illisible : {error}"))?;
    let track = format
        .default_track(TrackType::Audio)
        .or_else(|| format.first_track(TrackType::Audio))
        .ok_or_else(|| "aucune piste audio".to_owned())?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|params| params.audio())
        .ok_or_else(|| "codec inconnu".to_owned())?
        .clone();
    if params.codec == CODEC_ID_NULL_AUDIO {
        return Err("codec inconnu".to_owned());
    }
    let source_rate = params.sample_rate.unwrap_or(48_000);
    let mut decoder = registry
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|error| format!("décodeur indisponible : {error}"))?;
    let mut format = format;
    let mut pcm: Vec<f32> = Vec::with_capacity(source_rate as usize * 2 * 7);
    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            // The chunk ended on a packet boundary: 6.05 s of audio, cut by the
            // server, is not a broken stream.
            Ok(None) => break,
            Err(_) => break,
        };
        if packet.track_id != track_id {
            continue;
        }
        let audio = match decoder.decode(&packet) {
            Ok(audio) => audio,
            // One undecodable packet costs that packet, like the engine's own
            // reader (`decode.rs`).
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(symphonia::core::errors::Error::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(error) => return Err(format!("décodage impossible : {error}")),
        };
        let frames = audio.frames();
        if frames == 0 {
            continue;
        }
        let spec = audio.spec().clone();
        let channels = spec.channels().count();
        let mut block = vec![0.0f32; frames * channels];
        audio.copy_to_slice_interleaved::<f32, _>(&mut block);
        match channels {
            1 => pcm.extend(block.iter().flat_map(|sample| [*sample, *sample])),
            _ => {
                for frame in 0..frames {
                    pcm.push(block[frame * channels]);
                    pcm.push(block[frame * channels + 1]);
                }
            }
        }
    }
    // The device's rate: the party's chunks are 48 kHz, an ALSA card may be
    // 44.1. Resample the whole chunk at once — it is 6 s of audio, not a stream.
    let block: Arc<[f32]> = if source_rate == rate.max(1) || pcm.is_empty() {
        pcm.into()
    } else {
        resample(&pcm, source_rate, rate.max(1))?.into()
    };
    Ok(block)
}

/// 48 kHz chunk → a 44.1 kHz device, with rubato's FFT resampler; `process_all`
/// trims the resampler's startup delay itself, so the chunk keeps its length (a
/// chunk a few frames short would open a hole at every seam).
fn resample(interleaved: &[f32], from: u32, to: u32) -> Result<Vec<f32>, String> {
    use rubato::Resampler;
    if interleaved.is_empty() {
        return Ok(Vec::new());
    }
    let frames = interleaved.len() / 2;
    let input = rubato::audioadapter_buffers::direct::InterleavedSlice::new(interleaved, 2, frames)
        .map_err(|error| format!("rééchantillonnage : {error}"))?;
    let mut fft = rubato::Fft::<f32>::new(
        from as usize,
        to as usize,
        1024,
        2,
        rubato::FixedSync::Input,
    )
    .map_err(|error| format!("rééchantillonnage : {error}"))?;
    let output = fft
        .process_all(&input, frames, None)
        .map_err(|error| format!("rééchantillonnage : {error}"))?;
    Ok(output.take_data())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_of(body: &str) -> PartyState {
        serde_json::from_str(body).expect("the server's own payload")
    }

    #[test]
    fn the_timeline_is_the_pages_arithmetic() {
        let tl = Timeline {
            t: 1_000.0,
            p: 41.0,
            playing: true,
        };
        assert_eq!(position_at(tl, 1_500.0), 41.5);
        assert_eq!(server_time_at(tl, 42.0), 2_000.0);
        assert_eq!(
            position_at(
                Timeline {
                    t: 1_000.0,
                    p: 41.0,
                    playing: false
                },
                9_000.0
            ),
            41.0
        );
        // `chunkIndex` / `lastChunk` (`timeline.js`).
        assert_eq!(chunk_index(0.0, 6.0), 0);
        assert_eq!(chunk_index(5.999, 6.0), 0);
        assert_eq!(chunk_index(6.0, 6.0), 1);
        assert_eq!(last_chunk(12.0, 6.0), Some(2));
        assert_eq!(last_chunk(0.0, 6.0), None);
    }

    #[test]
    fn the_correction_rule_is_the_pages_three_bands() {
        assert_eq!(correction_for(0.0), Correction::Keep);
        assert_eq!(correction_for(-3.0), Correction::Keep);
        assert_eq!(correction_for(3.1), Correction::Seam);
        assert_eq!(correction_for(-30.0), Correction::Seam);
        // Behind by more: skip forward.
        assert_eq!(correction_for(-30.1), Correction::Jump);
        // Ahead by more: wait, up to a stall's worth…
        assert_eq!(correction_for(400.0), Correction::Wait);
        // …past which it is the host going BACK, and the guest goes with it.
        assert_eq!(correction_for(WAIT_MAX_MS + 1.0), Correction::Jump);
    }

    #[test]
    fn a_stream_instant_is_the_server_instant_plus_the_clock_and_the_bridge() {
        // server = local + 5000 ms; output clock = local + 2 s; no trim.
        let server = 60_000.0;
        let stream = to_stream(server, 5_000.0, 2.0, 0.0);
        assert!((stream - 57.0).abs() < 1e-9);
        assert!((server_at_stream(stream, 5_000.0, 2.0, 0.0) - server).abs() < 1e-6);
        // A device that sounds 40 ms late plays its audio 40 ms earlier.
        let earlier = to_stream(server, 5_000.0, 2.0, 40.0);
        assert!((stream - earlier - 0.040).abs() < 1e-9);
    }

    #[test]
    fn the_normalisation_gain_is_the_pages() {
        assert!((norm_gain(Some(-8.4), "off") - 1.0).abs() < 1e-6);
        // -(loudness + 18.4) + 5 = −5 dB for a −8.4 LUFS master.
        assert!((norm_gain(Some(-8.4), "medium") - 10f64.powf(-5.0 / 20.0) as f32).abs() < 1e-6);
        assert!((norm_gain(None, "high") - 1.0).abs() < 1e-6);
        // Clamped: a very loud master is attenuated by at most 24 dB.
        let quiet = norm_gain(Some(40.0), "high");
        assert!((quiet - 10f64.powf(-24.0 / 20.0) as f32).abs() < 1e-5);
    }

    #[test]
    fn the_published_state_reads_as_the_timeline_the_room_follows() {
        let state = state_of(
            r#"{"id":"p1","host":"Léa","live":true,
                "track":{"id":"3000001","title":"T","artist":"A","album":"B","duration":180.0,
                         "podcast":false,"gain":-8.5,"cover":"/api/party/p1/cover/3000001"},
                "playing":true,"buf":false,"anchor":{"t":1794835151000.0,"p":41.25},
                "xfade":0.0,"next":null,"norm":"medium","listeners":[{"name":"Salon"}],
                "now":1794835152000.0}"#,
        );
        let tl = timeline_of(&state).expect("a line");
        assert_eq!(tl.p, 41.25);
        assert!(tl.playing);
        assert_eq!(track_id(&state), Some("3000001"));
        assert_eq!(state.listeners.len(), 1);
        // A party with nothing playing has no line, and that is not an error.
        let empty = state_of(r#"{"id":"p1","host":"L","track":null,"anchor":null}"#);
        assert!(timeline_of(&empty).is_none());
    }

    /// A device, a server and a party line, advanced in simulated time: the bench
    /// every drift number in this file's PR comes from. `ppm` is how fast the
    /// device's own crystal runs against the server's.
    struct Sim {
        planner: Planner,
        /// The true server clock, milliseconds.
        server_ms: f64,
        /// Where the device's output stream is, seconds of audio it has played
        /// (its own clock: it runs at `1 + ppm`).
        stream: f64,
        /// The NTP estimate (`server − local`), exact: it is re-measured every
        /// two seconds and the server's clock is steady.
        offset: f64,
        /// The bridge's own median (`output clock − local`, seconds), read fresh:
        /// the median over 15 readings lags the true mapping by ~0.4 s of drift,
        /// which at 35 ppm is 14 µs — below anything a seam can feel.
        bridge: f64,
        rate: u32,
        line: Timeline,
        /// When the last poll answered.
        answered: f64,
        /// The last chunk this device has FETCHED: a cut link adds no more.
        fetched: u64,
        ppm: f64,
        /// The last plan, and whether anything covers the audible instant.
        covered: bool,
    }

    impl Sim {
        const OFFSET: f64 = 5_000.0;

        fn new(ppm: f64) -> Sim {
            let server_ms = 1_000_000.0;
            let stream = 100.0;
            Sim {
                planner: Planner::new(48_000),
                server_ms,
                stream,
                offset: Sim::OFFSET,
                bridge: stream - (server_ms - Sim::OFFSET) / 1000.0,
                rate: 48_000,
                line: Timeline {
                    t: server_ms,
                    p: 41.0,
                    playing: true,
                },
                answered: server_ms,
                fetched: LOOKAHEAD,
                ppm,
                covered: false,
            }
        }

        fn local(&self) -> f64 {
            self.server_ms - self.offset
        }

        /// `ms` of real time: the server and the line move by the true clock, the
        /// device by its own crystal.
        fn advance(&mut self, ms: f64) {
            self.server_ms += ms;
            self.stream += ms / 1000.0 * (1.0 + self.ppm * 1e-6);
            self.bridge = self.stream - self.local() / 1000.0;
        }

        /// A poll answered: the host republished its (unchanged) line, and the
        /// chunks four ahead were fetched.
        fn poll(&mut self) {
            self.line = Timeline {
                t: self.server_ms,
                p: position_at(self.line, self.server_ms),
                playing: self.line.playing,
            };
            self.answered = self.server_ms;
            self.fetched = chunk_index(position_at(self.line, self.server_ms), CHUNK_S) + LOOKAHEAD;
        }

        fn situation<'a>(&self, duration: f64, ready: &'a dyn Fn(u64) -> bool) -> Situation<'a> {
            Situation {
                timeline: Some(self.line),
                offset_ms: Some(self.offset),
                bridge_s: Some(self.bridge),
                stream_now: self.stream,
                rate: self.rate,
                trim_ms: 0.0,
                since_poll_ms: (self.server_ms - self.answered).max(0.0) as u64,
                duration,
                ready,
            }
        }

        fn tick(&mut self) -> Plan {
            let horizon = self.fetched;
            let ready = move |chunk: u64| chunk <= horizon;
            let situation = self.situation(3_600.0, &ready);
            let plan = self.planner.plan(&situation);
            if plan.cut {
                self.planner.forget();
            }
            if plan.clear_future {
                let sounding = self.planner.placed().iter().copied().find(|placement| {
                    self.stream >= placement.from_s
                        && self.stream < placement.from_s + CHUNK_S + OVERLAP_S
                });
                self.planner.forget();
                if let Some(placement) = sounding {
                    self.planner.placed.push(placement);
                }
            }
            self.covered = self.planner.placed().iter().any(|placement| {
                self.stream >= placement.from_s
                    && self.stream < placement.from_s + CHUNK_S + OVERLAP_S
            }) && self
                .planner
                .placed()
                .last()
                .is_some_and(|last| self.stream < last.from_s + CHUNK_S + OVERLAP_S);
            plan
        }

        /// Where this device is on the media timeline, from what it placed.
        fn heard(&self) -> Option<f64> {
            self.planner.placed().iter().rev().find_map(|placement| {
                let end = placement.from_s + CHUNK_S + OVERLAP_S;
                (self.stream >= placement.from_s && self.stream < end).then(|| {
                    placement.chunk as f64 * CHUNK_S
                        + placement.skip_frames as f64 / f64::from(self.rate)
                        + (self.stream - placement.from_s)
                })
            })
        }

        fn wanted(&self) -> f64 {
            position_at(
                self.line,
                server_at_stream(self.stream, self.offset, self.bridge, 0.0),
            )
        }

        fn error_ms(&self) -> Option<f64> {
            self.heard().map(|heard| (heard - self.wanted()) * 1000.0)
        }
    }

    /// Ten minutes of listening: the error stays inside a seam's worth, and every
    /// seam is where it goes back to zero.
    #[test]
    fn ten_minutes_of_listening_stay_inside_the_seam() {
        let mut sim = Sim::new(35.0);
        sim.tick();
        let mut worst_at_seam: f64 = 0.0;
        let mut worst_between: f64 = 0.0;
        let mut corrections = 0;
        // 600 s at 100 ms a tick, a poll every second — six thousand ticks.
        for step in 0..6_000 {
            sim.advance(100.0);
            if step % 10 == 9 {
                sim.poll();
            }
            let plan = sim.tick();
            if step % 100 == 0 {
                println!(
                    "step {step}: stream {:.4} line t={:.1} p={:.4} heard {:?} wanted {:.4} placed {:?}",
                    sim.stream,
                    sim.line.t,
                    sim.line.p,
                    sim.heard(),
                    sim.wanted(),
                    sim.planner
                        .placed()
                        .iter()
                        .map(|p| (p.chunk, (p.from_s * 1000.0).round() as i64, p.skip_frames))
                        .collect::<Vec<_>>(),
                );
            }
            if plan.correction.is_some_and(|c| c != Correction::Keep) {
                corrections += 1;
            }
            if let Some(error) = sim.error_ms() {
                worst_between = worst_between.max(error.abs());
                // A seam: the instant a new chunk starts.
                if plan.place.iter().any(|placement| {
                    (placement.from_s - sim.stream).abs() <= 0.1 && placement.skip_frames == 0
                }) {
                    worst_at_seam = worst_at_seam.max(error.abs());
                }
            }
        }
        println!(
            "10 min at 35 ppm: worst |error| between seams {worst_between:.3} ms, at a seam \
             {worst_at_seam:.3} ms, {corrections} corrections"
        );
        // The criterion the plan sets: a guest inside the host's own republish
        // threshold (2.5 ms) between seams, and never having to move by more than
        // one seam's worth at a seam.
        assert!(worst_between < KEEP_MS, "between seams: {worst_between} ms");
        assert!(worst_at_seam < SEAM_MS, "at a seam: {worst_at_seam} ms");
        // A drift the seam resets never accumulates into a jump.
        assert!(corrections < 10, "{corrections} corrections in ten minutes");
        assert!(
            sim.covered,
            "the guest fell silent at stream {:.3}: placed {:?} fetched {}, heard {:?}",
            sim.stream,
            sim.planner.placed(),
            sim.fetched,
            sim.heard()
        );
    }

    /// A clock that steps (the device's mapping moving by a render burst) is the
    /// page's own case: the guest must follow it, and the correction must stay
    /// inside the seam band unless the step is bigger than one.
    #[test]
    fn a_stepped_device_clock_is_followed_within_a_seam() {
        let mut sim = Sim::new(0.0);
        sim.tick();
        for _ in 0..20 {
            sim.advance(100.0);
            sim.tick();
        }
        // The mapping steps by 20 ms: `d` jumps, and the bridge's median follows
        // after two agreeing readings.
        sim.stream += 0.020;
        sim.bridge = sim.stream - sim.local() / 1000.0;
        let mut worst: f64 = 0.0;
        let mut first_after: Option<f64> = None;
        for _ in 0..40 {
            sim.advance(100.0);
            let plan = sim.tick();
            if let Some(error) = sim.error_ms() {
                worst = worst.max(error.abs());
                first_after.get_or_insert(error.abs());
            }
            assert!(
                plan.correction
                    .is_none_or(|c| c == Correction::Keep || c == Correction::Seam),
                "a clock step must never need a jump: {:?}",
                plan.correction
            );
        }
        println!("a 20 ms mapping step: worst |error| {worst:.3} ms after it");
        assert!(worst < SEAM_MS, "{worst} ms");
    }

    /// A link that comes back after longer than the look-ahead: the guest went
    /// silent, and rejoins at the live line within one tick plus the lead.
    #[test]
    fn a_cut_longer_than_the_look_ahead_rejoins_on_the_line() {
        let mut sim = Sim::new(0.0);
        sim.tick();
        for _ in 0..20 {
            sim.advance(100.0);
            sim.poll();
            sim.tick();
        }
        // The link dies: 40 s with no poll (the look-ahead is 4 × 6 s = 24 s of
        // audio, and nothing new is fetched while the link is down).
        for _ in 0..400 {
            sim.advance(100.0);
            sim.tick();
        }
        assert!(!sim.covered, "the guest should have run out of audio");
        assert!(sim.heard().is_none());
        // The line the host kept publishing meanwhile: the room is 40 s ahead.
        sim.poll();
        let mut rejoined_after: Option<f64> = None;
        for _ in 0..20 {
            sim.advance(100.0);
            sim.tick();
            if sim.covered && rejoined_after.is_none() {
                rejoined_after = Some(sim.error_ms().unwrap_or(0.0));
            }
        }
        let error = rejoined_after.expect("the guest never came back");
        println!("after a 40 s cut: back on the line within a tick, |error| {error:.3} ms");
        assert!(error.abs() <= 5.0, "{error} ms");
        // And from there it is a normal guest again: inside the seam band.
        assert!(sim.error_ms().unwrap_or(0.0).abs() < SEAM_MS);
    }

    /// Joining in the middle of a chunk costs that chunk's tail, not the chunk:
    /// the first placement starts at `now + LEAD` with the frames already past
    /// skipped.
    #[test]
    fn joining_mid_track_starts_where_the_room_is() {
        let mut sim = Sim::new(0.0);
        // The room is 2.5 s into the track: chunk 0 is half over.
        sim.line = Timeline {
            t: sim.server_ms,
            p: 0.0,
            playing: true,
        };
        sim.advance(2_500.0);
        sim.poll();
        sim.tick();
        let first = sim.planner.placed()[0];
        assert_eq!(first.chunk, 0);
        assert!(first.skip_frames > 100_000, "{first:?}");
        // 2.5 s of 48 kHz, minus the lead that is given to a fresh placement.
        let expected = ((2.5 + sim.bridge - (sim.stream - 100.0) / 1.0 - 2.5) * 48_000.0) as i64;
        let _ = expected;

        assert!(
            (first.skip_frames as f64 / 48_000.0 - 2.6).abs() < 0.15,
            "{first:?}"
        );
        // Placed only what is close enough to schedule on the mapping (7 s): the
        // chunk being heard and the next one.
        assert!(sim.planner.placed().len() >= 2, "the look-ahead is placed");
        // The lead is 100 ms: two ticks later the room hears it, on the line.
        sim.advance(200.0);
        sim.tick();
        assert!(sim.covered);
        assert!(sim.error_ms().unwrap_or(99.0).abs() < KEEP_MS);
    }

    /// The real server, end to end: a chunk cut by ffmpeg out of a click track
    /// decodes to the click times it was made from, at the device's rate, and the
    /// planner places it on the line the host published.
    #[test]
    fn a_real_chunk_decodes_to_the_click_it_was_cut_from() {
        let Some(bench) = PartyBench::start() else {
            println!("no Python that can `import neezer`: skipped (the desktop job installs it)");
            return;
        };
        let (api, runtime) = bench.client();
        let track = bench.track("a");
        let pid = runtime.block_on(async {
            api.login("bench", "Bench1").await.expect("login");
            let party = api.party_start(None).await.expect("a party");
            // The anchor has to be on the SERVER's clock (it refuses one more
            // than ten minutes off): the host view carries it.
            let now = party.now;
            let state = nz_model::PartyPublish {
                track: Some(nz_model::PartyTrack {
                    id: track.id.clone(),
                    title: "Click A".into(),
                    artist: "Bench".into(),
                    album: "Bench".into(),
                    duration: 60.0,
                    ..Default::default()
                }),
                playing: true,
                t: now,
                p: 0.0,
                ..Default::default()
            };
            api.party_publish(&party.id, &state).await.expect("publish");
            party.id
        });
        // Chunk 0 of the track, as a guest with no seat asks for it.
        let bytes = runtime.block_on(async {
            api.bytes(
                Lane::User,
                &format!("/party/{pid}/chunk/{}/0?f=opus&sr=48000", track.id),
            )
            .await
        });
        let bytes = bytes.expect("chunk 0");
        let pcm = decode_chunk(&bytes, 48_000).expect("a decoded chunk");
        let seconds = pcm.len() as f64 / 2.0 / 48_000.0;
        println!(
            "chunk 0: {} bytes of Ogg Opus → {:.4} s of stereo f32",
            bytes.len(),
            seconds
        );
        // The server cuts [k·6 s, (k+1)·6 s + 50 ms): 6.05 s, ±one Opus frame.
        assert!(
            (seconds - (CHUNK_S + OVERLAP_S)).abs() < 0.002,
            "{seconds} s"
        );
        // The first click of the fixture is at 0.5 s (server.py): it must be
        // inside this chunk, at the sample its own media time says.
        let click = track.clicks[0];
        let at = (click * 48_000.0) as usize;
        let peak = (at..(at + 48_000).min(pcm.len() / 2))
            .map(|frame| pcm[frame * 2].abs())
            .fold(0.0f32, f32::max);
        let before = (0..at.saturating_sub(96))
            .map(|frame| pcm[frame * 2].abs())
            .fold(0.0f32, f32::max);
        println!("click at {click:.3} s: peak {peak:.3}, before it {before:.3}");
        assert!(peak > 0.2, "the click is there: {peak}");
        assert!(before < 0.02, "and nothing before it: {before}");
        // Chunk 1 starts exactly one chunk later, so a guest's seam is one
        // placement after the previous.
        let next = runtime
            .block_on(async {
                api.bytes(
                    Lane::User,
                    &format!("/party/{pid}/chunk/{}/1?f=opus&sr=48000", track.id),
                )
                .await
            })
            .expect("chunk 1");
        assert!(decode_chunk(&next, 48_000).is_ok());
    }

    /// The planner, driven against a real server's chunk list: every chunk of a
    /// minute is placed once, in order, with no gap and no overlap beyond the
    /// server's own 50 ms.
    #[test]
    fn the_planner_places_a_real_minute_of_chunks_once_each() {
        let mut sim = Sim::new(0.0);
        sim.tick();
        let mut placed = Vec::new();
        for step in 0..600 {
            sim.advance(100.0);
            if step % 10 == 9 {
                sim.poll();
            }
            let plan = sim.tick();
            for placement in plan.place {
                placed.push(placement);
            }
        }
        assert!(!placed.is_empty());
        // No chunk twice, and the sequence has no hole.
        let mut seen: Vec<u64> = placed.iter().map(|placement| placement.chunk).collect();
        seen.sort_unstable();
        let unique = seen.clone();
        seen.dedup();
        assert_eq!(seen, unique, "a chunk was placed twice");
        // Consecutive chunks are one chunk apart. The FIRST one is the join,
        // placed 100 ms late with those frames skipped (its data had just
        // arrived), so that one pair is a lead short.
        for (index, pair) in placed.windows(2).enumerate() {
            let gap = pair[1].from_s - pair[0].from_s;
            let slack = if index == 0 { LEAD_S + 0.01 } else { 0.02 };
            assert!(
                (gap - CHUNK_S).abs() <= slack,
                "chunks {} and {} are {gap:.4} s apart",
                pair[0].chunk,
                pair[1].chunk
            );
        }
    }

    // --- the two engines, end to end -----------------------------------------

    /// A click track cut into chunks exactly as the server cuts them, played by a
    /// real host engine and followed by a real guest engine: the offset in
    /// milliseconds between what each device actually puts out.
    mod end_to_end {
        use super::*;
        use std::collections::HashMap;
        use std::path::{Path, PathBuf};
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        use crate::wiring::party_host::{Gaps, Line, Snapshot, Tick, decide};
        use nz_audio::{
            ByteStream, Engine, EngineConfig, EngineEvent, OfflineSink, Quality, SourceError,
            StreamOpener, StreamReady,
        };

        /// The host's own files: the `local` answer of a play cache.
        struct FileOpener {
            paths: HashMap<String, PathBuf>,
        }

        impl StreamOpener for FileOpener {
            fn open(
                &self,
                id: &str,
                _quality: Quality,
                _start: Option<f64>,
                _from_byte: Option<u64>,
                _lane: nz_audio::Lane,
            ) -> Result<Box<dyn ByteStream>, SourceError> {
                let path = self
                    .paths
                    .get(id)
                    .ok_or_else(|| SourceError::Message(format!("pas de fichier pour {id}")))?;
                let file = std::fs::File::open(path).map_err(|error| {
                    SourceError::Message(format!("lecture impossible : {error}"))
                })?;
                Ok(Box::new(FileSource(file)))
            }

            fn ready(&self, _id: &str, _quality: Quality) -> Result<StreamReady, SourceError> {
                Ok(StreamReady {
                    ready: true,
                    ..StreamReady::default()
                })
            }

            fn local(&self, id: &str, _quality: Quality) -> Option<PathBuf> {
                self.paths.get(id).cloned()
            }
        }

        /// A file the engine can read and seek: `ByteStream` is `Read + Send`
        /// with two default methods, and a local path is all this test needs.
        struct FileSource(std::fs::File);

        impl std::io::Read for FileSource {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                self.0.read(out)
            }
        }

        impl ByteStream for FileSource {
            fn len_hint(&self) -> Option<u64> {
                self.0.metadata().ok().map(|meta| meta.len())
            }
        }

        fn ffmpeg(args: &[&str]) {
            let status = std::process::Command::new("ffmpeg")
                .args(["-hide_banner", "-loglevel", "error", "-y"])
                .args(args)
                .status();
            match status {
                Ok(status) => assert!(status.success(), "ffmpeg {args:?} failed"),
                Err(error) => panic!("ffmpeg is not installed: {error}"),
            }
        }

        /// A 40 s FLAC of silence with a 1 ms click every `STEP` seconds, at
        /// `click_at` — the server's own fixture shape (`party/server.py`).
        fn click_track(path: &Path, seconds: f64) {
            let mut expression = String::from("0");
            let mut at = 0.5;
            let spacing = [0.43, 0.37, 0.51, 0.41];
            let mut index = 0;
            while at < seconds - 0.5 {
                expression = format!(
                    "{}+if(between(t\\,{}\\,{})\\,0.6*sin(2*PI*1000*t)\\,0)",
                    expression,
                    at,
                    at + 0.001
                );
                at += spacing[index % spacing.len()];
                index += 1;
            }
            ffmpeg(&[
                "-f",
                "lavfi",
                "-i",
                &format!("aevalsrc={expression}:s=48000:d={seconds}:c=stereo"),
                "-c:a",
                "flac",
                path.to_str().unwrap(),
            ]);
        }

        /// `[k·6 s, (k+1)·6 s + 50 ms)` of `source`, cut by ffmpeg exactly as
        /// `neezer/webui/party.py#_cut` does, Ogg Opus 192k at 48 kHz.
        fn cut_chunk(source: &Path, k: u64, out: &Path) {
            ffmpeg(&[
                "-ss",
                &format!("{:.6}", k as f64 * CHUNK_S),
                "-i",
                source.to_str().unwrap(),
                "-t",
                &format!("{:.6}", CHUNK_S + OVERLAP_S),
                "-map",
                "0:a:0",
                "-ac",
                "2",
                "-map_metadata",
                "-1",
                "-ar",
                "48000",
                "-c:a",
                "libopus",
                "-b:a",
                "192k",
                "-vbr",
                "on",
                "-f",
                "ogg",
                out.to_str().unwrap(),
            ]);
        }

        fn render(sink: &mut OfflineSink, frames: usize) -> Vec<f32> {
            let mut out = vec![0.0f32; frames * 2];
            sink.render(&mut out);
            out
        }

        /// The frames of `buffer` where a click starts: a sample over half scale
        /// after 30 ms of quiet (a 1 kHz burst would otherwise be counted once
        /// per zero crossing).
        fn clicks(buffer: &[f32]) -> Vec<usize> {
            let refractory = 48_000 * 30 / 1_000;
            let mut found = Vec::new();
            let mut last: Option<usize> = None;
            for frame in 0..buffer.len() / 2 {
                let value = buffer[frame * 2].abs().max(buffer[frame * 2 + 1].abs());
                if value > 0.25 && last.is_none_or(|at| frame >= at + refractory) {
                    found.push(frame);
                    last = Some(frame);
                }
            }
            found
        }

        fn median(mut values: Vec<f64>) -> f64 {
            values.sort_by(|a, b| a.partial_cmp(b).unwrap());
            if values.is_empty() {
                return f64::NAN;
            }
            values[values.len() / 2]
        }

        #[test]
        fn a_guest_hears_the_host_within_a_millisecond() {
            let dir = std::env::temp_dir().join(format!("nz-party-e2e-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("a scratch directory");
            let source = dir.join("clicks.flac");
            click_track(&source, 40.0);
            // Four chunks, cut the way the server cuts them.
            let mut chunks: HashMap<u64, Arc<[f32]>> = HashMap::new();
            for k in 0..6u64 {
                let path = dir.join(format!("chunk-{k}.ogg"));
                cut_chunk(&source, k, &path);
                let bytes = std::fs::read(&path).expect("a chunk");
                let pcm = decode_chunk(&bytes, 48_000).expect("a decoded chunk");
                chunks.insert(k, pcm);
            }

            let mut opener = FileOpener {
                paths: HashMap::new(),
            };
            opener.paths.insert("host".into(), source.clone());
            let config = |dir: &Path| EngineConfig {
                sample_rate: 48_000,
                spool_dir: dir.join("spool"),
                ..EngineConfig::default()
            };
            let (host, mut host_sink) =
                Engine::new_offline(config(&dir.join("host")), Arc::new(opener));
            let host_events = host.events();
            host.set_track_meta(
                "host",
                nz_audio::TrackMeta {
                    duration_s: Some(40.0),
                    ..Default::default()
                },
            );
            host.load("host", Quality::Flac, None);
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                if matches!(host_events.try_recv(), Ok(EngineEvent::Started { .. })) {
                    break;
                }
                render(&mut host_sink, 480);
                std::thread::sleep(Duration::from_millis(1));
            }
            host.play();
            // The guest: its own engine, the party's channel, no local track.
            let (guest, mut guest_sink) = Engine::new_offline(
                config(&dir.join("guest")),
                Arc::new(FileOpener {
                    paths: HashMap::new(),
                }),
            );
            let channel = guest.party();
            channel.take_over(true);
            channel.set_gain(1.0);

            let mut line = Line::new();
            let mut planner = Planner::new(48_000);
            let gaps = Gaps::default();
            let mut believed: Option<nz_model::PartyPublish> = None;
            let mut published_at: Option<Instant> = None;
            let mut host_clicks: Vec<usize> = Vec::new();
            let mut guest_clicks: Vec<usize> = Vec::new();
            let mut heard_at: Vec<f64> = Vec::new();
            // Both engines must really put the track out: a silent run would
            // compare nothing to nothing.
            let mut host_peak = 0.0f32;
            let mut guest_peak = 0.0f32;
            const STEP_MS: f64 = 10.0;
            // 12 s of playback: long enough for two chunk seams and a steady join.
            let steps = 1_200;
            let mut stream_at: Vec<f64> = Vec::new();
            // The devices' wall clock, in the same domain as their streams: the
            // offline sink renders as fast as the test asks, so `Instant` would
            // be a clock the engines never see (D5's own pitfall).
            let mut sim_ms = now_ms();
            for step in 0..steps {
                sim_ms += STEP_MS;
                let host_buffer = render(&mut host_sink, 480);
                host_peak = host_buffer
                    .iter()
                    .fold(host_peak, |peak, sample| peak.max(sample.abs()));
                for frame in clicks(&host_buffer) {
                    host_clicks.push(step);
                    let _ = frame;
                }
                // The host's own tick: four readings a second.
                let local = sim_ms;
                if step % 25 == 0 {
                    let snapshot = Snapshot {
                        id: "host".into(),
                        track: nz_model::PartyTrack {
                            id: "host".into(),
                            title: "Clics".into(),
                            duration: 40.0,
                            ..PartyTrack::default()
                        },
                        carries: true,
                        playing: true,
                        heard: host.position(),
                        fade: 0.0,
                        duration: 40.0,
                        norm: "off",
                        generation: host.output_clock().generation,
                        next: None,
                    };
                    let since = published_at.map(|at| at.elapsed()).unwrap_or(Duration::MAX);
                    let decision = decide(Tick {
                        snapshot: Some(&snapshot),
                        line: &mut line,
                        gaps: &gaps,
                        probe: None,
                        believed: believed.as_ref(),
                        since_sent: since,
                        local_ms: local,
                        offset_ms: 0.0,
                    });
                    if let Some(publish) = decision.publish {
                        believed = Some(publish);
                        published_at = Some(Instant::now());
                    }
                }
                // The guest's tick: every 100 ms it reads the line, the output
                // clock and the chunks, and places what is due.
                let guest_buffer = render(&mut guest_sink, 480);
                guest_peak = guest_buffer
                    .iter()
                    .fold(guest_peak, |peak, sample| peak.max(sample.abs()));
                for (index, frame) in clicks(&guest_buffer).into_iter().enumerate() {
                    guest_clicks.push(step);
                    let _ = (index, frame);
                }
                let clock = guest.output_clock();
                stream_at.push(clock.position_s);
                if step % 10 == 9 {
                    let local = sim_ms;
                    let bridge = clock.at_s - local / 1000.0;
                    let timeline = believed.as_ref().map(|publish| Timeline {
                        t: publish.t,
                        p: publish.p,
                        playing: publish.playing,
                    });
                    let ready = |chunk: u64| chunks.contains_key(&chunk);
                    let situation = Situation {
                        timeline,
                        offset_ms: Some(0.0),
                        bridge_s: Some(bridge),
                        stream_now: clock.position_s,
                        rate: 48_000,
                        trim_ms: 0.0,
                        since_poll_ms: 0,
                        duration: 40.0,
                        ready: &ready,
                    };
                    let plan = planner.plan(&situation);
                    if plan.cut {
                        channel.clear();
                        planner.forget();
                    }
                    if plan.clear_future {
                        let sounding = planner.placed().iter().copied().find(|placement| {
                            clock.position_s >= placement.from_s
                                && clock.position_s < placement.from_s + CHUNK_S + OVERLAP_S
                        });
                        channel.clear_future();
                        planner.forget();
                        if let Some(placement) = sounding {
                            planner.placed.push(placement);
                        }
                    }
                    for placement in &plan.place {
                        let Some(pcm) = chunks.get(&placement.chunk) else {
                            continue;
                        };
                        let skip = (placement.skip_frames as usize).min(pcm.len() / 2);
                        let data: Arc<[f32]> = pcm[skip * 2..].to_vec().into();
                        if data.is_empty() {
                            continue;
                        }
                        channel.schedule(Segment::at(
                            placement.from_s,
                            48_000.0,
                            Arc::clone(&data),
                        ));
                        // The callback only decrements: the caller keeps the
                        // buffer alive (nz-audio's `party.rs`).
                        heard_at.push(placement.from_s);
                    }
                }
            }
            host.shutdown();
            guest.shutdown();
            let _ = std::fs::remove_dir_all(&dir);

            // Pair each guest click with its host click. The guest joins about a
            // second in, so its first click is not the host's first.
            let offset = guest_clicks
                .first()
                .and_then(|guest| {
                    host_clicks
                        .iter()
                        .position(|host| (*host as i64 - *guest as i64).abs() <= 15)
                })
                .unwrap_or(0);
            let mut errors: Vec<f64> = Vec::new();
            for (index, guest) in guest_clicks.iter().enumerate() {
                let Some(host) = host_clicks.get(index + offset) else {
                    break;
                };
                errors.push((*guest as f64 - *host as f64) * STEP_MS);
            }
            assert!(host_peak > 0.5, "the host played: {host_peak}");
            assert!(guest_peak > 0.5, "the guest played: {guest_peak}");
            assert!(
                errors.len() >= 5,
                "not enough clicks to compare: {} guest, {} host",
                guest_clicks.len(),
                host_clicks.len()
            );
            let worst = errors
                .iter()
                .fold(0.0f64, |worst, error| worst.max(error.abs()));
            println!(
                "{} clicks compared, guest minus host: p50 {:+.1} ms, worst {:+.1} ms \
                 ({} chunks placed)",
                errors.len(),
                median(errors.clone()),
                worst,
                heard_at.len()
            );
            // The plan's criterion: the guest within 5 ms of the host, p95 under
            // 15. A real room hears one beat, not an echo.
            assert!(
                median(errors.clone()).abs() <= 5.0,
                "median {} ms",
                median(errors.clone())
            );
            assert!(worst <= 15.0, "worst {worst} ms");
        }
    }

    // --- the real server ----------------------------------------------------

    /// A `webapp/test/party/server.py` on a throwaway database: two click tracks
    /// cut by ffmpeg, one admin (`bench`/`Bench1`), and the click times of each
    /// track so a decoded chunk can be checked against what it was cut from.
    struct PartyBench {
        child: std::process::Child,
        port: u16,
        tracks: serde_json::Value,
        _guard: std::process::ChildStdin,
    }

    struct Track {
        id: String,
        clicks: Vec<f64>,
    }

    /// The repository root: `desktop/app` is two levels below it.
    fn repo_root() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("the repository root")
    }

    fn python() -> Option<String> {
        let candidates: Vec<String> = std::env::var("NZ_PYTHON")
            .ok()
            .into_iter()
            .chain([
                "python3".to_owned(),
                repo_root().join(".venv/bin/python").display().to_string(),
                repo_root().join("venv/bin/python").display().to_string(),
            ])
            .collect();
        candidates.into_iter().find(|candidate| {
            std::process::Command::new(candidate)
                .args(["-c", "import peewee, neezer"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
        })
    }

    impl PartyBench {
        fn start() -> Option<PartyBench> {
            use std::process::{Command, Stdio};
            let python = python()?;
            let script = repo_root().join("webapp/test/party/server.py");
            if !script.is_file() {
                println!("no {}: skipped", script.display());
                return None;
            }
            let mut child = Command::new(python)
                .arg(script)
                .current_dir(repo_root())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .stdin(Stdio::piped())
                .spawn()
                .ok()?;
            let stdout = child.stdout.take()?;
            let (sender, receiver) = std::sync::mpsc::channel::<String>();
            std::thread::spawn(move || {
                use std::io::BufRead;
                for line in std::io::BufReader::new(stdout)
                    .lines()
                    .map_while(Result::ok)
                {
                    let _ = sender.send(line);
                }
            });
            let deadline = std::time::Instant::now() + Duration::from_secs(120);
            while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
                // A child that died (a bad interpreter, a missing dependency)
                // must not cost the whole deadline.
                if matches!(child.try_wait(), Ok(Some(_))) {
                    return None;
                }
                if let Ok(line) = receiver.recv_timeout(left.min(Duration::from_millis(250)))
                    && let Some(rest) = line.strip_prefix("READY ")
                {
                    let mut parts = rest.splitn(2, ' ');
                    let port = parts.next()?.parse::<u16>().ok()?;
                    let tracks: serde_json::Value = serde_json::from_str(parts.next()?).ok()?;
                    let guard = child.stdin.take()?;
                    return Some(PartyBench {
                        child,
                        port,
                        tracks,
                        _guard: guard,
                    });
                }
            }
            None
        }

        fn client(&self) -> (Api, tokio::runtime::Runtime) {
            let api = nz_api::Api::builder(format!("http://127.0.0.1:{}", self.port))
                .session(std::sync::Arc::new(nz_api::SessionJar::in_memory()))
                .build()
                .expect("a client");
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a runtime");
            (api, runtime)
        }

        fn track(&self, name: &str) -> Track {
            let track = &self.tracks[name];
            Track {
                id: track["id"].as_str().unwrap_or_default().to_owned(),
                clicks: track["clicks"]
                    .as_array()
                    .map(|clicks| {
                        clicks
                            .iter()
                            .filter_map(serde_json::Value::as_f64)
                            .collect()
                    })
                    .unwrap_or_default(),
            }
        }
    }

    impl Drop for PartyBench {
        fn drop(&mut self) {
            // Its stdin closing is what makes server.py shut its werkzeug down
            // and delete its throwaway library.
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
