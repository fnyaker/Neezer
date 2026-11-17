//! What each screen spent on the link: requests and bytes, counted per tag.
//!
//! Rule 3 and the phone's own measure table (`docs/agents/mobile.md`,
//! *Measures*, *Network*, and *Auth, security, data*) ask for a **counted**
//! cost per screen — requests and bytes, cold and warm — never a feeling and
//! never the wall-clock time a shared test process happens to take. This module
//! is that counter, and it is the only one: a screen tags its client
//! ([`crate::Api::with_tag`]) and reads the totals back
//! ([`crate::Api::stats`]).
//!
//! Three decisions the numbers depend on:
//!
//! - **A tag is a screen, not a lane.** The lanes of rule 1 decide *when* a
//!   request runs; the tag says *whose* it is (`home`, `search`, `player`).
//!   Two clients of the same [`crate::Api`] — one per screen — share the
//!   counters, so the totals add up to what the process really sent.
//! - **`requests` counts every HTTP request actually sent**, a redirect hop
//!   included: that is what a mobile link pays for, and a server that answers
//!   `302` to its own CDN costs two.
//! - **`bytes_in` counts the response BODY bytes read**, never the headers:
//!   header bytes depend on the server's own casing, its `Date` line and its
//!   `Set-Cookie` length, so they cannot be asserted against a test server's
//!   payload without measuring the server's banner instead of the app's cost.
//!   `bytes_out` is the request body exactly as it was written.
//!
//! A client that never called `with_tag` counts under [`TAG_NONE`], so no
//! desktop request disappears from the totals (the desktop calls neither and
//! pays one `Mutex` lock per request, measured in `tests/tls.rs`).

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::MutexGuard;

/// The tag of a request whose client never chose one: the desktop, and any
/// request the phone makes outside a screen (the app's own boot checks).
pub const TAG_NONE: &str = "app";

/// What one screen cost on the link, in counted units (rule 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagStats {
    /// The screen's tag — the string [`crate::Api::with_tag`] carried, or
    /// [`TAG_NONE`].
    pub tag: String,
    /// HTTP requests sent, redirect hops included.
    pub requests: u64,
    /// Response body bytes read.
    pub bytes_in: u64,
    /// Request body bytes written.
    pub bytes_out: u64,
}

/// One tag's running totals.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Counters {
    requests: u64,
    bytes_in: u64,
    bytes_out: u64,
}

/// The counters of one [`crate::Api`], shared by every clone it hands out.
///
/// A `BTreeMap` (not a `HashMap`): [`Stats::snapshot`] is a stable, sorted
/// answer, so a test or a diagnostic log can be compared as it is.
#[derive(Debug, Default)]
pub(crate) struct Stats {
    by_tag: Mutex<BTreeMap<String, Counters>>,
}

impl Stats {
    fn by_tag(&self) -> MutexGuard<'_, BTreeMap<String, Counters>> {
        // A panic in another task (a test that failed mid-request) must not
        // turn every later count into a panic of its own: these are integers.
        self.by_tag.lock().unwrap_or_else(|err| err.into_inner())
    }

    /// One request went out with this tag, carrying `bytes_out` of request
    /// body (0 for a GET).
    pub(crate) fn request(&self, tag: &str, bytes_out: u64) {
        let mut by_tag = self.by_tag();
        let counters = by_tag.entry(tag.to_owned()).or_default();
        counters.requests = counters.requests.saturating_add(1);
        counters.bytes_out = counters.bytes_out.saturating_add(bytes_out);
    }

    /// `bytes` of a response body arrived for this tag.
    pub(crate) fn bytes_in(&self, tag: &str, bytes: u64) {
        if bytes == 0 {
            return;
        }
        let mut by_tag = self.by_tag();
        let counters = by_tag.entry(tag.to_owned()).or_default();
        counters.bytes_in = counters.bytes_in.saturating_add(bytes);
    }

    /// Every tag that made at least one request, sorted by tag.
    pub(crate) fn snapshot(&self) -> Vec<TagStats> {
        self.by_tag()
            .iter()
            .map(|(tag, counters)| TagStats {
                tag: tag.clone(),
                requests: counters.requests,
                bytes_in: counters.bytes_in,
                bytes_out: counters.bytes_out,
            })
            .collect()
    }

    /// Forget every total (a diagnostic reset, and what a bench between two
    /// measures needs so the second one is not the sum of both).
    pub(crate) fn clear(&self) {
        self.by_tag().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The totals are per tag, exact, and a tag appears as soon as it sent one
    /// request — with its zero bytes rather than not at all (a screen that only
    /// asked `OPTIONS` still costs a request).
    #[test]
    fn totals_are_per_tag_and_exact() {
        let stats = Stats::default();
        assert!(stats.snapshot().is_empty(), "no request, no tag");

        stats.request("home", 0);
        stats.bytes_in("home", 1_234);
        stats.request("home", 0);
        stats.bytes_in("home", 66);
        stats.request("search", 21);
        stats.bytes_in("search", 0);

        assert_eq!(
            stats.snapshot(),
            vec![
                TagStats {
                    tag: "home".into(),
                    requests: 2,
                    bytes_in: 1_300,
                    bytes_out: 0,
                },
                TagStats {
                    tag: "search".into(),
                    requests: 1,
                    bytes_in: 0,
                    bytes_out: 21,
                },
            ],
            "home and search are counted apart, and sorted by tag"
        );

        // A reset really forgets, so two measures are not summed.
        stats.clear();
        assert!(stats.snapshot().is_empty());
        stats.request(TAG_NONE, 0);
        assert_eq!(stats.snapshot()[0].tag, TAG_NONE);
    }

    /// The counters saturate instead of wrapping: a long-lived phone must not
    /// report a negative total after 2^64 bytes (which no link can carry, but a
    /// bug that double-counts a chunk could).
    #[test]
    fn a_total_saturates_rather_than_wrapping() {
        let stats = Stats::default();
        stats.request("bulk", u64::MAX);
        stats.request("bulk", 10);
        stats.bytes_in("bulk", u64::MAX);
        stats.bytes_in("bulk", u64::MAX);
        let one = &stats.snapshot()[0];
        assert_eq!(one.requests, 2);
        assert_eq!(one.bytes_out, u64::MAX);
        assert_eq!(one.bytes_in, u64::MAX);
    }
}
