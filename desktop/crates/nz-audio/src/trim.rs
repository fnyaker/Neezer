//! Where a track's audio really starts and stops, and what a player may cut.
//!
//! The server measures the bounds with ffmpeg's `silencedetect` and caches them
//! (`neezer/webui/edges.py`, `GET /api/audio/edges/<id>?db=`); the app fetches
//! them like it fetches the gain, ahead of the track that needs them. This
//! module is the *acceptance* side of that contract, and it is the web player's
//! (`Player.svelte#edgesFor`, `#audioEndsAt`):
//!
//! - **A trim is for the EDGES and nothing else.** At most [`MAX_EDGE_TRIM`] at
//!   either end: a quiet fifteen-second outro is music, and cutting it was heard
//!   as a skip in the middle of the song.
//! - **Bounds measured on another file are no bounds.** A re-archive or a
//!   different version has its own length, so bounds whose measured duration
//!   disagrees with the track's metadata duration by more than
//!   [`EDGE_LENGTH_SLACK`] are dropped.
//! - **Nothing is cut when the file already ends there** (`< 0.2 s` of tail):
//!   cutting at the end would only race the decoder's own end-of-stream.
//! - **Podcasts are excluded** by the caller, not here: a spoken intro that opens
//!   quietly is not silence to be cut, and an episode is not something to blend
//!   into whatever follows it.
//!
//! The podcast exclusion and the `trim` setting itself live in the app; this
//! module only answers "given these bounds, what may be skipped".

/// The server never hands back a bigger trim, whatever the file says
/// (`neezer/webui/edges.py`, `MAX_EDGE_TRIM`). Past it the answer is "play it
/// all": a track opening on ninety seconds of near-silence is a real
/// arrangement, and skipping a third of the way into it is not a trim.
pub const MAX_EDGE_TRIM: f64 = 10.0;

/// How far the measured file length may drift from the track's metadata
/// duration. Metadata durations are whole seconds (`Player.svelte`,
/// `EDGE_LENGTH_SLACK`), so a re-archive is what this catches.
pub const EDGE_LENGTH_SLACK: f64 = 2.5;

/// Below this a "trim start" is not worth a request: the web player starts a
/// track at `e.start` only when it is over a quarter second in.
pub const MIN_TRIM_START_S: f64 = 0.25;

/// A trailing "silence" shorter than this is the decoder's own end, not a trim.
const MIN_TRIM_END_S: f64 = 0.2;

/// What `GET /api/audio/edges/<id>?db=` answers, in seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edges {
    /// Where the audio starts inside the file.
    pub start: f64,
    /// Where it stops.
    pub end: f64,
    /// The length of the file these bounds were measured on.
    pub duration: f64,
}

impl Edges {
    /// The bounds the player accepts for a track whose metadata duration is
    /// `track_duration_s`, or `None` when it must play the file whole.
    pub fn accepted(self, track_duration_s: Option<f64>) -> Option<Edges> {
        if !(self.start.is_finite() && self.end.is_finite() && self.duration.is_finite()) {
            return None;
        }
        // A file that measures as silent end to end (a broken decode, or a
        // threshold the person pushed to -20 dB) is not a zero-length track,
        // and neither is a bound before the file's own start.
        if self.start < 0.0 || self.end <= self.start + 1.0 || self.duration <= 0.0 {
            return None;
        }
        if self.start > MAX_EDGE_TRIM || self.duration - self.end > MAX_EDGE_TRIM {
            return None;
        }
        if let Some(meta) = track_duration_s
            && meta > 0.0
            && (self.duration - meta).abs() > EDGE_LENGTH_SLACK
        {
            return None;
        }
        Some(self)
    }

    /// Where to start the track: the audio's own beginning when it is worth a
    /// seek, otherwise the file's.
    pub fn start_offset(self) -> f64 {
        if self.start > MIN_TRIM_START_S {
            self.start
        } else {
            0.0
        }
    }
}

/// What a voice may skip: the start it begins at, and where it must stop.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Trim {
    /// Media seconds to start at (0: the file's own head).
    pub start: f64,
    /// Media seconds to stop at, when the file plays past its audio.
    pub end: Option<f64>,
}

impl Trim {
    pub const NONE: Trim = Trim {
        start: 0.0,
        end: None,
    };

    pub fn is_none(&self) -> bool {
        self.start == 0.0 && self.end.is_none()
    }
}

/// The trim a voice may use, from the server's bounds, the length the decoder
/// found and the track's metadata duration.
///
/// `container_s` is the length of what is actually being decoded (a `?start=`
/// slice is shorter than the track, and then only the metadata duration can say
/// whether the tail is worth cutting).
pub fn trim_for(edges: Option<Edges>, container_s: Option<f64>, track_s: Option<f64>) -> Trim {
    let Some(edges) = edges.and_then(|edges| edges.accepted(track_s)) else {
        return Trim::NONE;
    };
    // The track's own length is the reference; the container's only stands in
    // when nothing else is known.
    let total = track_s
        .filter(|duration| *duration > 0.0)
        .or(container_s.filter(|duration| *duration > 0.0));
    let end = match total {
        // The decoder already stops there: cutting would only race it.
        Some(duration) if edges.end > duration - MIN_TRIM_END_S => None,
        // Past the decoder's own end is not this file's audio.
        Some(duration) if edges.end > duration => None,
        _ => Some(edges.end),
    };
    Trim {
        start: edges.start_offset(),
        end,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bounds the server sends for a normal master: 0.4 s of silence at the
    /// head, 3 s at the tail, a 240 s file.
    fn master() -> Edges {
        Edges {
            start: 0.4,
            end: 237.0,
            duration: 240.0,
        }
    }

    #[test]
    fn a_measured_master_is_trimmed_at_both_ends() {
        let trim = trim_for(Some(master()), Some(240.0), Some(240.0));
        assert_eq!(trim.start, 0.4, "the head silence is skipped");
        assert_eq!(trim.end, Some(237.0), "and the 3 s tail with it");
    }

    /// A quarter second is not worth a request: the web player only starts a
    /// track at `e.start` above 0.25 s.
    #[test]
    fn a_head_shorter_than_a_quarter_second_is_played_from_zero() {
        let edges = Edges {
            start: 0.2,
            ..master()
        };
        assert_eq!(trim_for(Some(edges), Some(240.0), Some(240.0)).start, 0.0);
    }

    /// Bounds that would cut more than ten seconds at either end are not a trim
    /// (`Player.svelte#edgesFor`): a long ambient outro is music.
    #[test]
    fn bounds_that_would_cut_more_than_ten_seconds_are_dropped() {
        let long_head = Edges {
            start: 12.0,
            ..master()
        };
        assert_eq!(
            trim_for(Some(long_head), Some(240.0), Some(240.0)),
            Trim::NONE
        );
        let long_tail = Edges {
            end: 225.0,
            ..master()
        };
        assert_eq!(
            trim_for(Some(long_tail), Some(240.0), Some(240.0)),
            Trim::NONE
        );
    }

    /// Bounds measured on a different file (a re-archive, another version) would
    /// skip part of the song: their length must agree with the track's.
    #[test]
    fn bounds_from_another_file_are_not_used() {
        // 2.0 s apart: whole-second metadata, so still the same file.
        let same = Edges {
            start: 0.4,
            end: 235.0,
            duration: 238.0,
        };
        assert_eq!(
            trim_for(Some(same), Some(238.0), Some(240.0)).end,
            Some(235.0)
        );
        // 5.0 s apart: another version, so no bounds at all.
        let stale = Edges {
            duration: 235.0,
            ..same
        };
        assert_eq!(trim_for(Some(stale), Some(235.0), Some(240.0)), Trim::NONE);
    }

    /// The decoder already stops at the audio's end: no cut, or the end of the
    /// track races the decoder's own end-of-stream.
    #[test]
    fn a_file_that_ends_where_its_audio_does_keeps_its_tail() {
        let edges = Edges {
            start: 0.0,
            end: 239.9,
            duration: 240.0,
        };
        let trim = trim_for(Some(edges), Some(240.0), Some(240.0));
        assert_eq!(trim.end, None);
    }

    /// A file that measures as silent end to end is a broken decode, never a
    /// zero-length track.
    #[test]
    fn a_degenerate_measurement_is_refused() {
        let edges = Edges {
            start: 12.0,
            end: 12.5,
            duration: 240.0,
        };
        assert_eq!(edges.accepted(Some(240.0)), None);
        for value in [f64::NAN, f64::INFINITY, -1.0] {
            let edges = Edges {
                start: value,
                ..master()
            };
            assert_eq!(edges.accepted(Some(240.0)), None, "start {value}");
        }
    }

    /// A `?start=` slice has its own (short) container length; the metadata
    /// duration is still what says whether the tail is silence.
    #[test]
    fn a_slice_uses_the_track_length_not_the_slice_length() {
        let trim = trim_for(Some(master()), Some(3.0), Some(240.0));
        assert_eq!(trim.end, Some(237.0));
    }
}
