//! The line the room follows, the chunk geometry, and the ONE correction rule.
//!
//! All of it is pure arithmetic ported from the page
//! (`webapp/src/lib/party/timeline.js`), so every rule is a unit test rather
//! than a stopwatch: the guest's scheduler and the host's fit both read this
//! module, and so does the mobile core through this crate.

use nz_model::PartyState;

/// The chunk geometry the server cuts (`party.py#CHUNK_SECONDS`/`CHUNK_OVERLAP`).
pub const CHUNK_S: f64 = 6.0;
pub const OVERLAP_S: f64 = 0.05;
/// The correction thresholds, in milliseconds (`timeline.js`).
pub const KEEP_MS: f64 = 3.0;
pub const SEAM_MS: f64 = 30.0;
pub const WAIT_MAX_MS: f64 = 2_500.0;

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

pub(crate) fn track_id(state: &PartyState) -> Option<&str> {
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
