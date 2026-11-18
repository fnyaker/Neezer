//! This process's own monotonic clock, and the NTP-style probe of the server's.
//!
//! Both sides of a party need the probe — a guest to turn "the room is at 41.5 s"
//! into a device instant, a host to publish `t` in the server's own milliseconds
//! — and both need one origin for "now": two threads that disagreed by a
//! millisecond would disagree about the music.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use appcore::sync::ClockEstimator;
use nz_api::{Api, Lane};
use serde_json::Value;
use tracing::debug;

/// Clock probes: a burst of twelve 40 ms apart at the start (enough for a
/// min-RTT pick straight away), then one every two seconds (`clock.js`).
pub const BURST: u32 = 12;
pub const BURST_GAP: Duration = Duration::from_millis(40);
pub const STEADY: Duration = Duration::from_millis(2_000);
/// How many agreeing probes an offset needs before anything is placed on it.
pub const MIN_PROBES: u32 = 4;

/// This process's own monotonic milliseconds, the domain every estimate and
/// every schedule is in. One origin for the whole process: two threads that
/// disagreed by a millisecond would disagree about the music.
pub fn now_ms() -> f64 {
    PROCESS_START.elapsed().as_secs_f64() * 1000.0
}

static PROCESS_START: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);

pub(crate) fn local_ms() -> f64 {
    now_ms()
}

/// The NTP-style estimate of the server's clock, from `GET
/// /api/party/<pid>/clock`, and the loop that keeps it fresh.
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
