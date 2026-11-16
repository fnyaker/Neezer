//! How long a track was actually heard.
//!
//! The server is told `listened` seconds on every track change
//! (`Player.svelte#flushListen`), and it counts wall time only while the
//! transport was PLAYING: an hour on pause used to be reported as an hour
//! listened. The state is the only thing that knows when playback started and
//! stopped, so it keeps the marks itself and the caller's `now_ms` closes them.
//!
//! A track restarted from the top (`prev` past 3 s, repeat `one`) does not
//! report: it is the same listen, still running. The SPA resets its accumulator
//! there and loses the seconds heard before the restart; counting them is the
//! only difference, and it can only under-report less.

/// The hear-time of the row the transport is on.
#[derive(Debug, Clone, Default)]
pub(crate) struct ListenLedger {
    id: Option<String>,
    accum_ms: f64,
    /// When playback last started, `None` while paused.
    mark: Option<u64>,
}

impl ListenLedger {
    /// Adds the time played since the last mark and stops counting. Idempotent:
    /// a second call without an `open` between them adds nothing.
    pub fn close(&mut self, now_ms: u64) {
        if let Some(mark) = self.mark.take() {
            // A clock that went backwards (a suspend, a test) gives 0, never a
            // negative listen.
            self.accum_ms += now_ms.saturating_sub(mark) as f64;
        }
    }

    /// Starts counting again when the transport plays.
    pub fn open(&mut self, playing: bool, now_ms: u64) {
        self.close(now_ms);
        if playing {
            self.mark = Some(now_ms);
        }
    }

    /// Takes what the current row heard, in whole seconds, and forgets it.
    /// `None` when nothing was being tracked (the first track of a session).
    pub fn take(&mut self, now_ms: u64) -> Option<(String, u32)> {
        self.close(now_ms);
        let id = self.id.take()?;
        let seconds = (self.accum_ms / 1000.0).round().max(0.0) as u32;
        self.accum_ms = 0.0;
        Some((id, seconds))
    }

    /// Tracks `id` from here, counting only if the transport is playing.
    pub fn start(&mut self, id: Option<String>, playing: bool, now_ms: u64) {
        self.id = id;
        self.accum_ms = 0.0;
        self.mark = None;
        self.open(playing, now_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_playing_time_counts() {
        let mut ledger = ListenLedger::default();
        ledger.start(Some("a".into()), true, 0);
        ledger.open(false, 10_000); // paused at 10 s: 10 s heard
        ledger.open(true, 130_000); // a minute and a half on pause, then play
        let (id, seconds) = ledger.take(140_000).unwrap();
        assert_eq!(id, "a");
        assert_eq!(seconds, 20, "10 s before the pause + 10 s after it");
    }

    #[test]
    fn nothing_tracked_reports_nothing() {
        let mut ledger = ListenLedger::default();
        assert_eq!(ledger.take(5_000), None);
    }

    #[test]
    fn closing_twice_does_not_count_twice() {
        let mut ledger = ListenLedger::default();
        ledger.start(Some("a".into()), true, 0);
        ledger.close(4_000);
        ledger.close(4_000);
        let (_, seconds) = ledger.take(4_000).unwrap();
        assert_eq!(seconds, 4);
    }

    #[test]
    fn a_backwards_clock_never_goes_negative() {
        let mut ledger = ListenLedger::default();
        ledger.start(Some("a".into()), true, 10_000);
        let (_, seconds) = ledger.take(8_000).unwrap();
        assert_eq!(seconds, 0);
    }
}
