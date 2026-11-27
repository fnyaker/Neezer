//! What a title that cannot be played costs, and when the list stops.
//!
//! Owned by **MB3b**, and PORTED — the rule is `NativeRouting.fault`'s
//! (`android/app/src/main/java/org/nsupysonic/app/NativeRouting.kt`), the one
//! the old app needed once its player answered behind a locked screen:
//!
//! ```text
//! retryable                  → retry while retries are left, else stall
//! no next row, repeat-one,   → stall
//! or already three skips
//! otherwise                  → skip this title and start the next one
//! ```
//!
//! Three ideas, each of them paid for:
//!
//! - **A bad moment on the network is not a verdict about a track.** An
//!   `Offline`, a `503 + Retry-After`, a `5xx` reopens the same source, three
//!   times, **2 s, 5 s then 10 s** (android.md: "Network retry while stopped
//!   (2/5/10 s on IO, 5xx retried)"). Past that it is a stall, not a skip: the
//!   queue is not burned because the link died.
//! - **A 4xx or a decode failure IS a verdict about that title**, so it costs
//!   that title and not the list: the row is consumed and the next one starts.
//! - **At most three in a row, and never `repeat=one`.** A list whose every row
//!   fails must stop, not spin: the third skip is the last one
//!   (`FAULT_SKIPS_MAX`, unchanged from the old app), and a title the person
//!   asked to hear again is never advanced past.
//!
//! `stall` means **Paused with the cause kept** — a silent, explicit
//! `fault` the notification and the full-screen player both show. The person
//! then has a "Réessayer" and a "Suivant" that work.

use std::time::Duration;

use nz_audio::SourceError;

/// How many titles in a row a fault may consume (the old app's own constant).
pub const FAULT_SKIPS_MAX: u32 = 3;
/// Seconds between the attempts at one source: "2/5/10 s on IO".
pub const RETRY_DELAYS_S: [u64; 3] = [2, 5, 10];

/// What to do about one failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Reopen the same source after this delay.
    Retry(Duration),
    /// Consume the row and start the next one; `cause` is what the snapshot
    /// carries as `fault` while the list goes on.
    Skip,
    /// Pause, keep the position, and say why. The person decides the rest.
    Stall,
}

/// The per-attempt and per-track counters.
#[derive(Debug, Default)]
pub struct Faults {
    /// Attempts already made at the CURRENT source.
    retries: u32,
    /// Titles consumed in a row without one playing.
    skips: u32,
}

impl Faults {
    pub fn new() -> Faults {
        Faults::default()
    }

    /// Whether a failure deserves another attempt rather than a verdict:
    /// `true` for a dead link, a `503 + Retry-After` (a track not archived yet —
    /// "try later", never "failed"), a `5xx`, and an `io` sentence.
    pub fn retryable(cause: Option<&SourceError>) -> bool {
        match cause {
            Some(SourceError::Offline) | Some(SourceError::NotReady { .. }) => true,
            Some(SourceError::Provider { status, .. }) => *status >= 500,
            // A cause the opener never saw (the engine's own decode failure)
            // is not a network moment.
            Some(SourceError::Message(_)) | None => false,
        }
    }

    /// The cause as the French sentence the snapshot carries.
    pub fn cause_text(cause: Option<&SourceError>) -> String {
        match cause {
            Some(error) => error.to_string(),
            // The engine flattened it before we could read it; its sentence is
            // already the right one (rule 3: a cause, never a code).
            None => "Titre illisible".to_owned(),
        }
    }

    /// One failure of the CURRENT track. `has_next` and `repeat_one` are the
    /// queue's answer at that moment.
    pub fn failed(
        &mut self,
        cause: Option<&SourceError>,
        has_next: bool,
        repeat_one: bool,
    ) -> Action {
        if Self::retryable(cause) {
            if self.retries < RETRY_DELAYS_S.len() as u32 {
                let delay = RETRY_DELAYS_S[self.retries as usize];
                self.retries += 1;
                return Action::Retry(Duration::from_secs(delay));
            }
            // The retry budget is spent on a bad link: a stall, never a skip —
            // the queue did nothing wrong.
            return Action::Stall;
        }
        if !has_next || repeat_one || self.skips >= FAULT_SKIPS_MAX {
            return Action::Stall;
        }
        self.skips += 1;
        self.retries = 0;
        Action::Skip
    }

    /// A title started: the skip streak is over, the retry budget is fresh.
    pub fn started(&mut self) {
        self.skips = 0;
        self.retries = 0;
    }

    /// A new source was asked for (a load, a seek): only the retry budget
    /// resets — a skip streak survives, or a list of unplayable rows would
    /// loop.
    pub fn reloaded(&mut self) {
        self.retries = 0;
    }

    /// How many titles in a row failed (a test reads it).
    pub fn skips(&self) -> u32 {
        self.skips
    }

    pub fn retries(&self) -> u32 {
        self.retries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn io() -> SourceError {
        SourceError::Message("connexion interrompue".into())
    }

    fn not_found() -> SourceError {
        SourceError::Provider {
            status: 404,
            message: "introuvable".into(),
        }
    }

    fn unavailable() -> SourceError {
        SourceError::Provider {
            status: 503,
            message: "pas encore archivé".into(),
        }
    }

    /// **2 s, 5 s, 10 s, then a stall** — the retry ladder is android.md's, and
    /// a dead link never consumes a title.
    #[test]
    fn a_bad_link_is_retried_on_the_old_apps_ladder_then_stalls() {
        let mut faults = Faults::new();
        assert_eq!(
            faults.failed(Some(&SourceError::Offline), true, false),
            Action::Retry(Duration::from_secs(2))
        );
        assert_eq!(
            faults.failed(Some(&SourceError::Offline), true, false),
            Action::Retry(Duration::from_secs(5))
        );
        assert_eq!(
            faults.failed(Some(&unavailable()), true, false),
            Action::Retry(Duration::from_secs(10))
        );
        assert_eq!(
            faults.failed(Some(&SourceError::Offline), true, false),
            Action::Stall
        );
        assert_eq!(faults.skips(), 0, "a bad link never consumed a title");
    }

    /// A 404 or a decode failure costs THAT title: it is skipped, three times
    /// at most, and the fourth stops the list.
    #[test]
    fn three_titles_are_skipped_and_the_fourth_stops_the_list() {
        let mut faults = Faults::new();
        assert_eq!(faults.failed(Some(&not_found()), true, false), Action::Skip);
        assert_eq!(faults.failed(Some(&io()), true, false), Action::Skip);
        assert_eq!(faults.failed(None, true, false), Action::Skip);
        assert_eq!(faults.skips(), FAULT_SKIPS_MAX);
        assert_eq!(
            faults.failed(Some(&not_found()), true, false),
            Action::Stall,
            "a list whose every row is unplayable stops with its cause"
        );
    }

    /// No next row, or a title asked to be heard again: a stall, never an
    /// advance past something the person chose.
    #[test]
    fn the_last_row_and_repeat_one_stall() {
        let mut faults = Faults::new();
        assert_eq!(
            faults.failed(Some(&not_found()), false, false),
            Action::Stall
        );
        assert_eq!(faults.failed(Some(&not_found()), true, true), Action::Stall);
        assert_eq!(faults.skips(), 0);
    }

    /// A title that starts clears the streak, so a single bad row in a long
    /// list does not count against the next ones.
    #[test]
    fn playing_again_clears_the_streak() {
        let mut faults = Faults::new();
        faults.failed(Some(&not_found()), true, false);
        faults.failed(Some(&not_found()), true, false);
        faults.started();
        assert_eq!(faults.skips(), 0);
        assert_eq!(faults.failed(Some(&not_found()), true, false), Action::Skip);
    }

    /// The 5xx/4xx distinction is the whole point of keeping the typed cause.
    #[test]
    fn only_a_server_failure_over_500_is_retryable() {
        assert!(Faults::retryable(Some(&SourceError::Offline)));
        assert!(Faults::retryable(Some(&SourceError::NotReady {
            retry_after: 30
        })));
        assert!(Faults::retryable(Some(&unavailable())));
        assert!(!Faults::retryable(Some(&not_found())));
        assert!(!Faults::retryable(Some(&io())));
        assert!(!Faults::retryable(None));
        assert_eq!(
            Faults::cause_text(Some(&not_found())),
            "titre introuvable sur le serveur",
            "a cause the person can read, never a code — and never the server's \
             own English body (nz-audio's `SourceError` Display)"
        );
    }
}
