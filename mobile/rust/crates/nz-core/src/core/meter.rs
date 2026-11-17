//! What one screen cost on the link (rule 3, "Counted, not timed").
//!
//! The counters are the core's own, not a guess: a request is counted when it
//! is really issued, its body bytes when they really arrived, and a screen's
//! tag is the name its lot chose (`home`, `search`, `session`, `cover`). The
//! desktop measured "a screen costs ≤ the SPA's count" the same way
//! (`webapp/test/requests/run.mjs`), and a warm open of a visited screen must
//! read **0** requests here.
//!
//! `nz-api` gains the same counters per tag (SC3's `Api::stats`); until then
//! every core call records through this meter, and a later lot that asks
//! `nz-api` directly records what it knows. `NzCore::stats()` is the one answer
//! the UI reads either way.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// One screen's traffic, as the UI shows it and a lot writes in its PR.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TagStats {
    pub tag: String,
    pub requests: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

#[derive(Debug, Default)]
struct Counters {
    requests: AtomicU64,
    bytes_in: AtomicU64,
    bytes_out: AtomicU64,
}

/// The tags the core itself issues, so the FFI never spells a string twice.
pub const TAG_SESSION: &str = "session";
pub const TAG_COVER: &str = "cover";

/// Requests and bytes per tag. Cheap to share (`Arc` inside when a caller needs
/// one) and lock-free on the request path: the map is taken once per tag and
/// the counters are atomics.
#[derive(Debug, Default)]
pub struct Meter {
    tags: Mutex<BTreeMap<&'static str, Counters>>,
}

impl Meter {
    pub fn new() -> Meter {
        Meter::default()
    }

    /// One request left this device on `tag`, carrying `bytes_out` and answered
    /// with `bytes_in` (what is known; a JSON body the client decodes internally
    /// may report `0` — the note says which numbers a lot may claim).
    pub fn record(&self, tag: &'static str, bytes_in: u64, bytes_out: u64) {
        let mut tags = self.tags.lock().unwrap_or_else(|err| err.into_inner());
        let counters = tags.entry(tag).or_default();
        counters.requests.fetch_add(1, Ordering::Relaxed);
        counters.bytes_in.fetch_add(bytes_in, Ordering::Relaxed);
        counters.bytes_out.fetch_add(bytes_out, Ordering::Relaxed);
    }

    /// One request that never reached the wire (the offline gate, a refused
    /// URL): counted NOWHERE, because that is the number the acceptance holds —
    /// "0 sockets opened" is "0 requests recorded".
    pub fn stats(&self) -> Vec<TagStats> {
        let tags = self.tags.lock().unwrap_or_else(|err| err.into_inner());
        tags.iter()
            .map(|(tag, counters)| TagStats {
                tag: (*tag).to_owned(),
                requests: counters.requests.load(Ordering::Relaxed),
                bytes_in: counters.bytes_in.load(Ordering::Relaxed),
                bytes_out: counters.bytes_out.load(Ordering::Relaxed),
            })
            .collect()
    }

    /// The total the diagnostic screen shows, in counted requests.
    pub fn requests(&self) -> u64 {
        self.stats().iter().map(|stats| stats.requests).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_and_bytes_add_up_per_tag() {
        let meter = Meter::new();
        meter.record(TAG_SESSION, 120, 48);
        meter.record(TAG_SESSION, 30, 0);
        meter.record(TAG_COVER, 3_306, 0);
        let stats = meter.stats();
        assert_eq!(stats.len(), 2, "one row per tag, never per request");
        // Tags come back in name order, so a screen's row is looked up by name.
        let by_tag = |tag: &str| stats.iter().find(|row| row.tag == tag).cloned();
        assert_eq!(
            by_tag(TAG_SESSION),
            Some(TagStats {
                tag: TAG_SESSION.to_owned(),
                requests: 2,
                bytes_in: 150,
                bytes_out: 48,
            })
        );
        assert_eq!(
            by_tag(TAG_COVER),
            Some(TagStats {
                tag: TAG_COVER.to_owned(),
                requests: 1,
                bytes_in: 3_306,
                bytes_out: 0,
            })
        );
        assert_eq!(meter.requests(), 3);
    }

    #[test]
    fn an_unused_meter_reports_nothing_at_all() {
        let meter = Meter::new();
        assert!(meter.stats().is_empty());
        assert_eq!(meter.requests(), 0);
    }
}
