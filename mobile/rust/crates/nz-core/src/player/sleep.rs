//! The sleep timer: stop after a while, or at the end of the track.
//!
//! Owned by **MB3b**, ported from `webapp/src/lib/sleep.js`:
//!
//! - **The deadline is a wall-clock instant, never a countdown of ticks.** A
//!   phone on a bedside table is throttled, frozen or asleep: comparing
//!   `now_ms` with the instant it should end is the only arithmetic that still
//!   lands where it should after an hour of Doze.
//! - **The last 15 s bring the level down instead of cutting it**
//!   ([`SLEEP_FADE_MS`], `fadeAt`: `(left / 15 s)²`, an amplitude-squared ramp —
//!   what the ear hears as even). The fade rides on top of the person's own
//!   volume, so it is never written into the setting and never has to be put
//!   back.
//! - **It PAUSES, it does not stop.** The queue and the position stay where they
//!   were, and one tap on play resumes.
//! - **"End of track" is an advance the player refuses**, not a timer: the row
//!   that is playing is finished, and the next one never starts
//!   (`finishSleepAtTrackEnd`).
//!
//! The timer's own fields are here; the player owns one and gives it the
//! caller's `now_ms` on every command and on its tick, so nothing in this file
//! reads a clock.

use serde::{Deserialize, Serialize};

/// `minutes` after the command, or the end of the playing track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SleepTimer {
    Minutes { minutes: u32 },
    EndOfTrack,
}

/// What the button's label shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SleepState {
    /// Minutes left, rounded UP (a timer set to 10 says "10" for the first
    /// second), or `0` for "at the end of this track".
    pub minutes: u32,
    pub at_end_of_track: bool,
}

/// The last 15 s of a sleep timer ramp the level down (`sleep.js`'s number).
pub const SLEEP_FADE_MS: u64 = 15_000;

/// One running timer. `None` in the player means "no timer".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sleep {
    /// The instant the music stops, for a minutes timer.
    ends_at_ms: Option<u64>,
    end_of_track: bool,
}

impl Sleep {
    /// A timer set NOW.
    pub fn new(timer: SleepTimer, now_ms: u64) -> Sleep {
        match timer {
            SleepTimer::Minutes { minutes } => Sleep {
                ends_at_ms: Some(now_ms.saturating_add(u64::from(minutes) * 60_000)),
                end_of_track: false,
            },
            SleepTimer::EndOfTrack => Sleep {
                ends_at_ms: None,
                end_of_track: true,
            },
        }
    }

    pub fn at_end_of_track(&self) -> bool {
        self.end_of_track
    }

    /// Whether the timer is a deadline (a minutes timer), which is what tells
    /// the player it must fade.
    pub fn is_deadline(&self) -> bool {
        self.ends_at_ms.is_some()
    }

    /// What the button shows: the minutes left (rounded up), or the flag.
    pub fn state(&self, now_ms: u64) -> SleepState {
        match self.ends_at_ms {
            Some(ends_at) => {
                let left = ends_at.saturating_sub(now_ms);
                SleepState {
                    minutes: left.div_ceil(60_000) as u32,
                    at_end_of_track: false,
                }
            }
            None => SleepState {
                minutes: 0,
                at_end_of_track: true,
            },
        }
    }

    /// The multiplier the player applies on top of the person's volume: 1 until
    /// the last 15 s, then the squared ramp, 0 at the instant.
    pub fn fade(&self, now_ms: u64) -> f32 {
        let Some(ends_at) = self.ends_at_ms else {
            return 1.0;
        };
        let left = ends_at.saturating_sub(now_ms);
        if left >= SLEEP_FADE_MS {
            return 1.0;
        }
        let x = left as f64 / SLEEP_FADE_MS as f64;
        (x * x) as f32
    }

    /// The deadline has passed: pause now.
    pub fn expired(&self, now_ms: u64) -> bool {
        match self.ends_at_ms {
            Some(ends_at) => now_ms >= ends_at,
            None => false,
        }
    }

    /// Only a minutes timer needs the fine tick, and only while it fades: a
    /// player that wrote the volume once a second would step the last 15 s in
    /// fifteen clicks.
    pub fn fading(&self, now_ms: u64) -> bool {
        match self.ends_at_ms {
            Some(ends_at) => {
                let left = ends_at.saturating_sub(now_ms);
                left < SLEEP_FADE_MS
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ten minutes means ten minutes, the label counts down, and the fade is 1
    /// until its own last 15 s.
    #[test]
    fn a_minutes_timer_counts_down_and_fades_at_the_end() {
        let start = 1_000_000;
        let sleep = Sleep::new(SleepTimer::Minutes { minutes: 10 }, start);
        assert_eq!(
            sleep.state(start),
            SleepState {
                minutes: 10,
                at_end_of_track: false
            }
        );
        assert_eq!(sleep.fade(start), 1.0);
        assert!(!sleep.fading(start));

        let ends_at = start + 10 * 60_000;
        // One minute in, the label says 9 (rounded up from 9 exactly).
        assert_eq!(sleep.state(start + 60_000).minutes, 9);
        assert_eq!(sleep.fade(ends_at - SLEEP_FADE_MS), 1.0, "still full");
        assert!(sleep.fading(ends_at - SLEEP_FADE_MS + 1));

        // Halfway through the fade: (0.5)² = 0.25 — the ear's even ramp.
        let half = sleep.fade(ends_at - SLEEP_FADE_MS / 2);
        assert!((half - 0.25).abs() < 1e-6, "{half}");
        assert_eq!(sleep.fade(ends_at), 0.0);
        assert!(sleep.expired(ends_at));
        assert!(!sleep.expired(ends_at - 1));
        assert_eq!(sleep.state(ends_at).minutes, 0);
    }

    /// End of track is not a deadline: no fade, no expiry, and it stops the
    /// advance.
    #[test]
    fn end_of_track_is_a_refused_advance_not_a_clock() {
        let sleep = Sleep::new(SleepTimer::EndOfTrack, 0);
        assert!(sleep.at_end_of_track());
        assert!(!sleep.is_deadline());
        assert_eq!(sleep.fade(u64::MAX / 2), 1.0);
        assert!(!sleep.expired(u64::MAX));
        assert_eq!(
            sleep.state(1234),
            SleepState {
                minutes: 0,
                at_end_of_track: true
            }
        );
    }

    /// A deadline already behind the clock is expired at once, never a panic on
    /// an underflow (a restored timer, a clock that jumped).
    #[test]
    fn a_deadline_in_the_past_is_simply_expired() {
        let sleep = Sleep::new(SleepTimer::Minutes { minutes: 1 }, 0);
        assert!(sleep.expired(60_000));
        assert_eq!(sleep.fade(60_000), 0.0);
        assert_eq!(sleep.state(60_000).minutes, 0);
    }
}
