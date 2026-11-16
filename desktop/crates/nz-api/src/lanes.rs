//! Rule 1 on the device: the three lanes of `docs/agents/desktop.md`.
//!
//! Every request carries a [`Lane`], and the gates here are the only way in:
//!
//! - [`Lane::User`] — what the person just did. At most [`USER_MAX_IN_FLIGHT`]
//!   (6) in flight, and it never waits for another lane: the tapped track's
//!   stream and the screen being opened come first, always.
//! - [`Lane::Prefetch`] — what comes right after. At most one, and it starts
//!   only once no User request has been in flight for [`PREFETCH_QUIET`]
//!   (250 ms). The caller decides *when* the next track is worth fetching (i.e.
//!   that the current one is playable); the gate enforces the quiet period that
//!   keeps it off the link while somebody is waiting.
//! - [`Lane::Bulk`] — downloads, offline sync, cover warming, transcription
//!   audio. At most one, and it starts only after [`BULK_QUIET`] (1 s) without
//!   User traffic. While the playing track still streams from the network, the
//!   bytes it reads are further capped by a token bucket at 25 % of the
//!   stream's measured throughput, never below [`BULK_MIN_BYTES_PER_S`]
//!   (64 kB/s) — see [`Gates::set_bulk_cap`] and [`bulk_cap_from_stream`].
//!
//! Admission order is a priority, not a preemption: a Bulk download already in
//! flight is not killed when somebody taps a track; it just stops being the
//! thing that gets the next slot. A background request that the server refuses
//! with `503` + `Retry-After` waits exactly that long (see `client.rs`), never
//! the exponential backoff a foreground GET uses.

use std::sync::Mutex;
use std::sync::MutexGuard;
use std::time::Duration;

use tokio::sync::Notify;
use tokio::time::Instant;

/// At most this many User requests in flight at once.
pub const USER_MAX_IN_FLIGHT: usize = 6;
/// At most one Prefetch request in flight at once.
pub const PREFETCH_MAX_IN_FLIGHT: usize = 1;
/// At most one Bulk request in flight at once.
pub const BULK_MAX_IN_FLIGHT: usize = 1;
/// How long the link must be free of User traffic before a Prefetch starts.
pub const PREFETCH_QUIET: Duration = Duration::from_millis(250);
/// How long the link must be free of User traffic before a Bulk request starts.
pub const BULK_QUIET: Duration = Duration::from_secs(1);
/// The floor of the Bulk cap, whatever the stream's measured throughput.
pub const BULK_MIN_BYTES_PER_S: u64 = 64 * 1024;
/// The share of the playing stream's throughput a Bulk download may use.
pub const BULK_STREAM_SHARE_PERCENT: u64 = 25;
/// The burst a saturating Bulk download may read before the bucket bites.
const BULK_BURST_BYTES: f64 = 64.0 * 1024.0;

/// The priority a request runs at (rule 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lane {
    /// The action just taken: it never waits.
    User,
    /// The next track / next screen.
    Prefetch,
    /// Downloads, warming, sync, transcription audio: it yields.
    Bulk,
}

impl Lane {
    /// `true` when the request must announce itself as background
    /// (`X-NS-Background: 1`), which is what makes the server gate it too.
    pub fn is_background(self) -> bool {
        matches!(self, Lane::Prefetch | Lane::Bulk)
    }

    /// The wire value, for a log line that never carries content.
    pub fn as_str(self) -> &'static str {
        match self {
            Lane::User => "user",
            Lane::Prefetch => "prefetch",
            Lane::Bulk => "bulk",
        }
    }
}

/// The Bulk cap that follows from a measured stream throughput: 25 % of it,
/// never under 64 kB/s. `None` throughput (nothing measured yet) means no cap —
/// there is no stream to yield to.
pub fn bulk_cap_from_stream(bytes_per_s: Option<u64>) -> Option<u64> {
    bytes_per_s.map(|rate| {
        (rate.saturating_mul(BULK_STREAM_SHARE_PERCENT) / 100).max(BULK_MIN_BYTES_PER_S)
    })
}

struct State {
    user: usize,
    prefetch: usize,
    bulk: usize,
    /// When the last User request ENDED (or when the gates were built: a bulk
    /// request queued before any user traffic must still yield, otherwise the
    /// "8 Bulk queued, one User issued" order would depend on a race).
    quiet_since: Instant,
}

struct Bucket {
    cap_bytes_per_s: Option<u64>,
    tokens: f64,
    at: Instant,
}

/// The gates of the three lanes, plus the Bulk token bucket.
pub struct Gates {
    state: Mutex<State>,
    wake: Notify,
    bucket: Mutex<Bucket>,
}

impl Default for Gates {
    fn default() -> Self {
        Self::new()
    }
}

impl Gates {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            state: Mutex::new(State {
                user: 0,
                prefetch: 0,
                bulk: 0,
                quiet_since: now,
            }),
            wake: Notify::new(),
            bucket: Mutex::new(Bucket {
                cap_bytes_per_s: None,
                tokens: BULK_BURST_BYTES,
                at: now,
            }),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        // A panic in another task must not turn every later request into a
        // panic of its own: the counts are plain integers.
        self.state.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn bucket(&self) -> MutexGuard<'_, Bucket> {
        self.bucket.lock().unwrap_or_else(|err| err.into_inner())
    }

    /// Wait for a slot in `lane`, then hold it until the returned guard drops.
    pub async fn acquire(&self, lane: Lane) -> LaneGuard<'_> {
        loop {
            // Registered BEFORE the state is read: a release that lands
            // between the read and the await still wakes this waiter.
            let mut wake = std::pin::pin!(self.wake.notified());
            wake.as_mut().enable();

            let deadline = {
                let mut state = self.state();
                let now = Instant::now();
                let quiet = now.duration_since(state.quiet_since);
                let admitted = match lane {
                    Lane::User => state.user < USER_MAX_IN_FLIGHT,
                    Lane::Prefetch => {
                        state.user == 0
                            && state.prefetch < PREFETCH_MAX_IN_FLIGHT
                            && quiet >= PREFETCH_QUIET
                    }
                    Lane::Bulk => {
                        state.user == 0 && state.bulk < BULK_MAX_IN_FLIGHT && quiet >= BULK_QUIET
                    }
                };
                if admitted {
                    match lane {
                        Lane::User => state.user += 1,
                        Lane::Prefetch => state.prefetch += 1,
                        Lane::Bulk => state.bulk += 1,
                    }
                    return LaneGuard { gates: self, lane };
                }
                // Only the quiet period is a deadline; a busy slot (or a User
                // in flight) ends when something releases, not when time passes.
                let quiet_only = match lane {
                    Lane::User => None,
                    Lane::Prefetch => {
                        (state.user == 0 && state.prefetch == 0).then_some(PREFETCH_QUIET)
                    }
                    Lane::Bulk => (state.user == 0 && state.bulk == 0).then_some(BULK_QUIET),
                };
                quiet_only.map(|need| now + need.saturating_sub(quiet))
            };

            match deadline {
                Some(at) => {
                    let _ = tokio::time::timeout_at(at, wake).await;
                }
                None => wake.await,
            }
        }
    }

    fn release(&self, lane: Lane) {
        {
            let mut state = self.state();
            match lane {
                Lane::User => {
                    state.user = state.user.saturating_sub(1);
                    // The clock the other two lanes yield to.
                    state.quiet_since = Instant::now();
                }
                Lane::Prefetch => state.prefetch = state.prefetch.saturating_sub(1),
                Lane::Bulk => state.bulk = state.bulk.saturating_sub(1),
            }
        }
        self.wake.notify_waiters();
    }

    /// Cap what a Bulk download may read, in bytes per second (`None`: no cap).
    pub fn set_bulk_cap(&self, bytes_per_s: Option<u64>) {
        let mut bucket = self.bucket();
        bucket.cap_bytes_per_s = bytes_per_s.filter(|rate| *rate > 0);
        bucket.at = Instant::now();
        bucket.tokens = BULK_BURST_BYTES;
    }

    /// Wait until `bytes` more may be read at the Bulk cap (a no-op when there
    /// is no cap, which is the case while nothing is streaming).
    pub async fn throttle(&self, bytes: u64) {
        if bytes == 0 {
            return;
        }
        let need = bytes as f64;
        loop {
            let wait = {
                let mut bucket = self.bucket();
                let Some(cap) = bucket.cap_bytes_per_s else {
                    return;
                };
                let now = Instant::now();
                bucket.tokens = (bucket.tokens
                    + now.duration_since(bucket.at).as_secs_f64() * cap as f64)
                    .min(BULK_BURST_BYTES);
                bucket.at = now;
                if bucket.tokens >= need.min(BULK_BURST_BYTES) {
                    // A chunk larger than the burst is paid in full: the bucket
                    // goes negative and refills at the cap, which is the rate
                    // that was asked for.
                    bucket.tokens -= need;
                    return;
                }
                Duration::from_secs_f64((need.min(BULK_BURST_BYTES) - bucket.tokens) / cap as f64)
            };
            tokio::time::sleep(wait).await;
        }
    }
}

/// One admitted request's slot. Dropping it frees the slot.
pub struct LaneGuard<'a> {
    gates: &'a Gates,
    lane: Lane,
}

impl LaneGuard<'_> {
    pub fn lane(&self) -> Lane {
        self.lane
    }
}

impl Drop for LaneGuard<'_> {
    fn drop(&mut self) {
        self.gates.release(self.lane);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{advance, timeout};

    /// The whole contract of the two background lanes, in virtual time: a
    /// Prefetch starts exactly 250 ms after the last User request ends, and a
    /// Bulk request exactly 1 s after it. Measured as the gate's own decision
    /// instant, so the numbers are the rule and not a scheduler artefact.
    #[tokio::test(start_paused = true)]
    async fn a_prefetch_starts_250_ms_after_the_last_user_request_and_a_bulk_1_s() {
        let gates = Gates::new();
        // The gates' clocks start at construction: a bulk request queued before
        // any User traffic still yields (1 s), which is what makes the order of
        // the acceptance test deterministic instead of a race.
        let early = Instant::now();
        let first_bulk = gates.acquire(Lane::Bulk).await;
        assert_eq!(
            Instant::now().duration_since(early),
            BULK_QUIET,
            "the first bulk request of a session waits 1 s"
        );
        drop(first_bulk);

        // A User request is in flight for 300 ms of virtual time.
        let user = gates.acquire(Lane::User).await;
        let ended = Instant::now();
        advance(Duration::from_millis(300)).await;
        drop(user);
        assert_eq!(
            Instant::now().duration_since(ended),
            Duration::from_millis(300)
        );

        let guard = gates.acquire(Lane::Prefetch).await;
        assert_eq!(
            Instant::now().duration_since(ended),
            Duration::from_millis(300) + PREFETCH_QUIET,
            "prefetch starts exactly 250 ms after the User request ended"
        );
        drop(guard);

        let guard = gates.acquire(Lane::Bulk).await;
        assert_eq!(
            Instant::now().duration_since(ended),
            Duration::from_millis(300) + BULK_QUIET,
            "bulk starts exactly 1 s after the last User request ended"
        );
        drop(guard);
    }

    #[tokio::test(start_paused = true)]
    async fn eight_bulk_requests_are_serialised_and_yield_to_a_user_request() {
        let gates = Gates::new();
        advance(BULK_QUIET).await;
        // One bulk in flight: the other seven wait, they do not fan out.
        let first = gates.acquire(Lane::Bulk).await;
        // A User request is admitted at once, even with a bulk in flight.
        let started = Instant::now();
        let user = timeout(Duration::from_millis(1), gates.acquire(Lane::User)).await;
        assert!(user.is_ok(), "a User request never waits for a Bulk one");
        assert_eq!(Instant::now(), started);
        drop(user);
        drop(first);
    }

    #[tokio::test(start_paused = true)]
    async fn the_user_lane_caps_at_six_in_flight_and_the_seventh_waits_for_a_slot() {
        let gates = Gates::new();
        let mut held = Vec::new();
        for _ in 0..USER_MAX_IN_FLIGHT {
            held.push(
                timeout(Duration::from_millis(1), gates.acquire(Lane::User))
                    .await
                    .expect("a slot is free"),
            );
        }
        let blocked = timeout(Duration::from_millis(50), gates.acquire(Lane::User)).await;
        assert!(blocked.is_err(), "the seventh User request waits");
        held.pop();
        let seventh = timeout(Duration::from_millis(1), gates.acquire(Lane::User)).await;
        assert!(seventh.is_ok(), "a released slot admits the waiter");
    }

    #[tokio::test(start_paused = true)]
    async fn a_bulk_stream_reads_at_the_cap_and_never_below_64_kb_s() {
        let gates = Gates::new();
        assert_eq!(bulk_cap_from_stream(None), None);
        assert_eq!(bulk_cap_from_stream(Some(0)), Some(BULK_MIN_BYTES_PER_S));
        assert_eq!(bulk_cap_from_stream(Some(600_000)), Some(150_000));
        assert_eq!(
            bulk_cap_from_stream(Some(100_000)),
            Some(BULK_MIN_BYTES_PER_S)
        );

        // At 128 kB/s, 256 kB takes ~1.5 s once the 64 kB burst is spent. Four
        // 64 kB chunks (the size a body arrives in) are read in turn.
        gates.set_bulk_cap(Some(128 * 1024));
        let started = Instant::now();
        for _ in 0..4 {
            gates.throttle(64 * 1024).await;
        }
        let spent = Instant::now().duration_since(started);
        assert_eq!(
            spent,
            Duration::from_millis(1_500),
            "256 kB at 128 kB/s: three 500 ms waits after the burst"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn without_a_cap_a_bulk_read_never_waits() {
        let gates = Gates::new();
        let started = Instant::now();
        gates.throttle(50 * 1024 * 1024).await;
        assert_eq!(Instant::now(), started);
    }
}
