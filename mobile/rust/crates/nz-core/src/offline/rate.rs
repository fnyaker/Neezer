//! The link's own meter: how fast the playing stream is reading, so a download
//! yields to it (rule 1, 「 Bulk ≤ 25 % du débit »).
//!
//! `nz-api`'s Bulk lane already paces a background body at 25 % of the stream's
//! measured throughput and never under 64 kB/s ([`nz_api::bulk_cap_from_stream`]),
//! but SOMETHING has to measure that throughput. The desktop feeds it from the
//! player's own reader (`nz_offline::note_stream_bytes`); the phone's player has
//! no such hook, and the offline lot may not edit it. So the phone measures the
//! SAME quantity from the one place both consumers share: the HTTP client's own
//! byte counters ([`nz_api::Api::stats`]), read once a second and only while a
//! download is queued.
//!
//! Three decisions keep that honest:
//!
//! - **Only the untagged client's bytes count** (`TAG_NONE`, the player's and the
//!   core's own reads). A download is tagged `downloads`, this module's set
//!   reads are tagged `offline`, so neither can inflate the rate it is throttled
//!   by.
//! - **A quarter of a second of samples, EWMA 0.6/0.4**: one stalled second must
//!   not collapse the cap for the next track (the same weight `nz-offline` uses).
//! - **4 s without a byte releases the cap**, exactly like `release_stale_cap`:
//!   a stopped stream has no throughput to yield to, and a download must not
//!   stay throttled for a track that ended.

use std::time::{Duration, Instant};

/// How long without a byte means "nothing is streaming any more".
pub const STALE_AFTER: Duration = Duration::from_secs(4);

/// One counter reading. The bytes are CUMULATIVE for the process's client, so a
/// sample is a difference, never a level.
pub struct LinkMeter {
    bytes: u64,
    at: Option<Instant>,
    /// Bytes per second, as last published.
    rate: f64,
    /// When the link last delivered anything at all.
    fed_at: Option<Instant>,
}

impl Default for LinkMeter {
    fn default() -> Self {
        LinkMeter {
            bytes: 0,
            at: None,
            rate: 0.0,
            fed_at: None,
        }
    }
}

impl LinkMeter {
    pub fn new() -> LinkMeter {
        LinkMeter::default()
    }

    /// The rate the last sample produced, in bytes per second (0 = nothing was
    /// reading).
    pub fn rate(&self) -> f64 {
        self.rate
    }

    /// Forget the measured rate (a stopped stream).
    pub fn clear(&mut self) {
        self.rate = 0.0;
        self.bytes = 0;
        self.at = None;
        self.fed_at = None;
    }

    /// Read the cumulative byte total at `now` and answer the rate, when it
    /// MOVED. `None` means nothing was learned: no previous sample to difference
    /// against, or a sample closer than [`MIN_WINDOW`] to the last one.
    pub fn observe(&mut self, now: Instant, total: u64) -> Option<f64> {
        let previous = match self.at {
            None => {
                self.at = Some(now);
                self.bytes = total;
                return None;
            }
            Some(at) => at,
        };
        let elapsed = now.saturating_duration_since(previous).as_secs_f64();
        if elapsed < MIN_WINDOW.as_secs_f64() {
            return None;
        }
        let delta = total.saturating_sub(self.bytes);
        self.bytes = total;
        self.at = Some(now);
        if delta == 0 {
            // No byte on the untagged client this window: a stream that ended
            // reports 0 and the cap is released by `stale`.
            self.rate = 0.0;
            return Some(0.0);
        }
        self.fed_at = Some(now);
        let instant = delta as f64 / elapsed;
        self.rate = if self.rate <= 0.0 {
            instant
        } else {
            0.6 * self.rate + 0.4 * instant
        };
        Some(self.rate)
    }

    /// Whether nothing has fed the link for [`STALE_AFTER`] — the cap is released
    /// then, and `true` says the caller must clear it.
    pub fn stale(&mut self, now: Instant) -> bool {
        match self.fed_at {
            Some(at) if now.saturating_duration_since(at) < STALE_AFTER => false,
            Some(_) => {
                self.clear();
                true
            }
            None => self.rate > 0.0,
        }
    }
}

/// The shortest window a rate is computed over: two samples closer than this
/// share one window rather than measuring the jitter of the scheduler.
pub const MIN_WINDOW: Duration = Duration::from_millis(250);

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use nz_api::bulk_cap_from_stream;

    use super::*;

    /// The meter measures bytes over TIME, and the cap the phone hands `nz-api`
    /// is 25 % of it: 800 kB/s of stream → 200 kB/s of download, and a slow
    /// stream keeps `nz-api`'s 64 kB/s floor rather than starving the queue.
    ///
    /// The numbers are the ones the acceptance prints: the reader's rate comes
    /// from counted bytes (`nz_api::Api::stats`), never from a stopwatch.
    #[test]
    fn the_cap_is_a_quarter_of_the_measured_stream() {
        let start = Instant::now();
        let mut meter = LinkMeter::new();
        assert_eq!(
            meter.observe(start, 0),
            None,
            "the first sample has nothing to difference"
        );

        // 800 kB in one second, read as the client's cumulative counter.
        let rate = meter
            .observe(start + Duration::from_secs(1), 800_000)
            .expect("a second sample measures");
        assert!((rate - 800_000.0).abs() < 1.0, "800 kB in 1 s: {rate}");
        assert_eq!(bulk_cap_from_stream(Some(rate as u64)), Some(200_000));

        // A second at 400 kB/s: the EWMA pulls the rate down but not to it, so
        // one slow second does not halve the cap for the next track.
        let rate = meter
            .observe(start + Duration::from_secs(2), 1_200_000)
            .expect("a third sample measures");
        assert!(
            (rate - 640_000.0).abs() < 1.0,
            "0.6×800k + 0.4×400k: {rate}"
        );
        assert_eq!(bulk_cap_from_stream(Some(rate as u64)), Some(160_000));

        // A stream slower than 256 kB/s keeps the shared floor: that decision is
        // nz-api's, and the phone does not second-guess it.
        assert_eq!(bulk_cap_from_stream(Some(40_000)), Some(64 * 1024));
    }

    /// A sample window shorter than the minimum measures nothing, and 4 s
    /// without a byte releases the cap — a download left throttled for a track
    /// that is over would be a bug the person feels as "the queue stopped".
    #[test]
    fn a_short_window_is_ignored_and_an_idle_link_releases_the_cap() {
        let start = Instant::now();
        let mut meter = LinkMeter::new();
        meter.observe(start, 0);
        assert_eq!(
            meter.observe(start + Duration::from_millis(50), 10_000),
            None,
            "50 ms is jitter, not a rate"
        );
        let rate = meter
            .observe(start + Duration::from_secs(1), 500_000)
            .expect("a full second measures");
        assert!(rate > 0.0);

        assert!(
            !meter.stale(start + Duration::from_secs(4)),
            "3 s after the last byte is not stale"
        );
        assert!(
            meter.stale(start + Duration::from_secs(5)),
            "exactly 4 s without a byte releases the cap"
        );
        assert_eq!(meter.rate(), 0.0, "a released cap is a cleared rate");

        // A window with no byte at all reports 0 and feeds nothing.
        let mut idle = LinkMeter::new();
        idle.observe(start, 1_000);
        assert_eq!(
            idle.observe(start + Duration::from_secs(1), 1_000),
            Some(0.0)
        );
        assert!(
            !idle.stale(start + Duration::from_secs(6)),
            "nothing was ever fed"
        );
        assert_eq!(idle.rate(), 0.0);
    }
}
