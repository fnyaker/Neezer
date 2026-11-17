//! What a request can fail with, and how the UI says it in French.
//!
//! The variants are the web app's `lib/api.js#req` failure shapes — ported, not
//! reinvented (docs/agents/desktop.md, *Network errors*): a connection failure
//! is [`ApiError::Offline`], a `401` is [`ApiError::Unauthorized`] (unless the
//! body says the remote-control loan ended), a server answer carrying
//! `X-NS-Error` is [`ApiError::Provider`] and *final* (Deezer was already
//! retried server-side), a `409`/`503` + `Retry-After` is [`ApiError::NotReady`],
//! a `409` WITHOUT one is [`ApiError::Conflict`], and a background request
//! refused by `_admit_by_priority` is [`ApiError::Busy`].

use serde_json::Value;

/// Everything the client can fail with.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ApiError {
    /// No answer at all: DNS, reset, timeout, airplane mode. The SPA calls this
    /// a network failure (`status: 0`).
    #[error("offline")]
    Offline,

    /// `401` on an ordinary session: it ended. The cookie jar is cleared.
    #[error("session ended")]
    Unauthorized,

    /// `401` whose body is `{"error":"remote ended"}`: a remote-control loan
    /// ended under the request, not the person's own session. The server hands
    /// back the account the browser had before, when it has one.
    #[error("remote control ended")]
    RemoteEnded {
        /// The pre-loan `user` object the server restored, if any.
        restored: Option<Value>,
    },

    /// A failure the server already classified as the provider's, with the code
    /// it put in `X-NS-Error`. FINAL: re-asking only fetches the same refusal.
    #[error("provider error {status} {code}: {message}")]
    Provider {
        status: u16,
        code: String,
        message: String,
        /// Seconds the server suggests waiting (`retry_in`), 0 when it did not.
        retry_in: u32,
    },

    /// Nothing on disk yet, but a writer is at work: wait exactly `retry_after`
    /// seconds and ask again (a first play's 409, or a prefetch queued on a
    /// cold track). Not a verdict about the track.
    ///
    /// The `409` the server uses for this ALWAYS carries `Retry-After`
    /// (`neezer/webui/__init__.py#_not_archived`), which is what tells it apart
    /// from a [`ApiError::Conflict`].
    #[error("not ready, retry in {retry_after}s")]
    NotReady { retry_after: u32 },

    /// The world disagreed with the request: a playlist moved under this client
    /// (a stale `version`), a track became available before a replacement, a
    /// group already exists, an analysis version is not the running one, a
    /// finished file is asked for a `?start=` slice instead of a byte range.
    ///
    /// A `409` WITHOUT `Retry-After` is one of these (`neezer/webui/*`: the
    /// only 409 that sets the header is `_not_archived`). Re-sending the same
    /// body changes nothing, so this is never retried as it was: the caller
    /// re-reads what it was editing and says so (a playlist screen re-reads
    /// once and offers the action again). `scope` is the part the server names
    /// (`content`/`members` on a playlist), and it is what lets a screen say
    /// WHICH half moved.
    #[error("conflict: {message}")]
    Conflict {
        /// The server's own label (`{"error": "conflict"}`).
        message: String,
        /// The server's machine code for this refusal (`{"code": …}`), what a
        /// screen prints its own sentence for: `last_admin`, `unavailable`,
        /// `cannot`, `already_running`…
        code: Option<String>,
        /// What the server says the conflict is about, when it says.
        scope: Option<String>,
    },

    /// A background request the server did not admit (`503` + `Retry-After`).
    /// It waited once, exactly as long as it was told, and is still refused.
    #[error("busy, retry in {retry_after}s")]
    Busy { retry_after: u32 },

    /// Any other non-2xx answer, with the server's own message and code.
    #[error("http {status}: {message}")]
    Http {
        status: u16,
        message: String,
        code: Option<String>,
    },

    /// The bytes arrived but are not the JSON this app models.
    #[error("cannot decode the answer: {0}")]
    Decode(String),

    /// A request this client refuses to BUILD, so no socket was opened: a
    /// redirect that would downgrade the session to plaintext off the
    /// configured origin, a certificate probe of a plaintext URL, a redirect
    /// chain longer than ten hops (`Net.kt`'s rules, SC3).
    ///
    /// The string is already the French sentence a screen prints:
    /// [`ApiError::cause_fr`] returns it unchanged, because the refusal is
    /// decided here and the server never had a status for it.
    #[error("refused: {0}")]
    Invalid(String),

    /// The certificate a handshake presented is not the one this client
    /// trusts (`Ssl.kt`'s trust-on-first-use, SC3).
    ///
    /// `sha256` is the PRESENTED leaf's SHA-256, colon-separated hex, in the
    /// exact form the pin stores and the setup screen shows — so it can be
    /// pinned by re-saving, or compared to the one somebody read on the server.
    /// `changed` says whether this client already held a pin for that origin
    /// and a DIFFERENT leaf answered: then "le certificat a changé", which is
    /// the sentence a phone shows instead of letting somebody sign in over a
    /// channel a café's Wi-Fi could be on.
    ///
    /// Never retried: a second handshake presents the same certificate.
    #[error("certificate {sha256} (changed: {changed})")]
    Certificate {
        /// The presented leaf's SHA-256, colon-separated hex (`AB:CD:…`).
        sha256: String,
        /// `true` when a pin was held for this origin and another leaf came.
        changed: bool,
    },
}

impl ApiError {
    /// `true` when the person can retry the action as it was ("Réessayer").
    ///
    /// A provider refusal is not retryable by the client (the server's job),
    /// and neither is a bad session or a payload this app cannot read. A
    /// [`ApiError::Conflict`] is not either: the request was valid and the
    /// world moved, so re-sending the same body fetches the same `409` — the
    /// caller re-reads and offers the action again.
    ///
    /// A [`ApiError::Certificate`] is not either: the same handshake presents
    /// the same leaf, so only a person can decide (re-pin it, or fix the
    /// server). A refusal this client made itself ([`ApiError::Invalid`]) has
    /// nothing to retry either: the address it named will be refused again.
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            ApiError::Offline | ApiError::NotReady { .. } | ApiError::Busy { .. }
        ) || matches!(self, ApiError::Http { status, .. } if *status >= 500)
    }

    /// The server's own machine code for a refusal, when it sent one: from the
    /// `{"code": …}` body of a `409` as well as of any other status. A screen
    /// prints a better sentence per code (`last_admin`, `unavailable`, …), and
    /// this is that lookup's key — one place, so a `Conflict` and an `Http`
    /// carrying the same code are not read two ways.
    pub fn code(&self) -> Option<&str> {
        match self {
            ApiError::Http { code, .. } | ApiError::Conflict { code, .. } => code.as_deref(),
            _ => None,
        }
    }

    /// The wait the server asked for, in seconds, when it asked for one.
    pub fn retry_after(&self) -> Option<u32> {
        match self {
            ApiError::NotReady { retry_after }
            | ApiError::Busy { retry_after }
            | ApiError::Provider {
                retry_in: retry_after,
                ..
            } => Some(*retry_after),
            _ => None,
        }
    }

    /// The failure in the few French words a screen can show, in the SPA's own
    /// wording (`webapp/src/lib/api.js#failureCause`) so both clients say the
    /// same thing for the same failure. Rule 5: French copy, never a raw status
    /// code on its own.
    pub fn cause_fr(&self) -> String {
        match self {
            ApiError::Offline => "le serveur ne répond pas (réseau)".to_owned(),
            ApiError::Unauthorized | ApiError::RemoteEnded { .. } => {
                "session expirée : reconnectez-vous".to_owned()
            }
            ApiError::Provider {
                status,
                code,
                message,
                ..
            } => {
                if code == "deezer-unreachable" {
                    "Deezer est injoignable".to_owned()
                } else if code == "deezer-denied" {
                    "Deezer n'est pas autorisé pour ce compte".to_owned()
                } else if *status >= 500 {
                    format!("erreur du serveur (HTTP {status}) : {message}")
                } else {
                    format!("HTTP {status} : {message}")
                }
            }
            ApiError::NotReady { retry_after } => {
                format!("pas encore prêt, réessayez dans {retry_after} s")
            }
            ApiError::Conflict { scope, .. } => match scope.as_deref() {
                // The playlist kept moving under this client: say which half, so
                // the screen can name it and the person knows what to re-read.
                Some("members") => "le partage a changé : la liste a été relue".to_owned(),
                Some("content") => "la playlist a changé : la liste a été relue".to_owned(),
                _ => "cette action est impossible dans l'état actuel".to_owned(),
            },
            ApiError::Busy { retry_after } => {
                format!("serveur occupé, réessayez dans {retry_after} s")
            }
            ApiError::Http {
                status, message, ..
            } => {
                if *status >= 500 {
                    let detail = if message.is_empty() || message == "network" {
                        String::new()
                    } else {
                        format!(" : {message}")
                    };
                    format!("erreur du serveur (HTTP {status}{detail})")
                } else if *status == 0 {
                    message.clone()
                } else {
                    format!("HTTP {status} : {message}")
                }
            }
            ApiError::Decode(_) => "réponse du serveur illisible".to_owned(),
            // Both new refusals already carry their French sentence (SC3): a
            // certificate refusal is a person's decision, never a status code.
            ApiError::Invalid(sentence) => sentence.clone(),
            ApiError::Certificate { changed, .. } => {
                if *changed {
                    "Le certificat du serveur a changé".to_owned()
                } else {
                    "Le certificat du serveur n'est pas reconnu".to_owned()
                }
            }
        }
    }
}

/// The retry budget of a GET: three tries, the note's contract
/// (`docs/agents/desktop.md`, *Network errors*). `lib/api.js` allows a fourth
/// fetch (`attempt < 3` after the first); three is the desktop's own cap — a
/// phone-linked laptop that keeps asking a dead server is what rule 3 forbids.
pub(crate) const MAX_GET_ATTEMPTS: usize = 3;

/// `min(400·2^attempt, 6000) ms`, plus `jitter` (0–250 ms) so a fleet of
/// reconnects does not land on the same instant.
pub(crate) fn backoff(attempt: usize, jitter_ms: u64) -> std::time::Duration {
    let step = 400u64.saturating_mul(1 << attempt.min(8)).min(6_000);
    std::time::Duration::from_millis(step + jitter_ms.min(250))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backoff_doubles_from_400_ms_and_caps_at_6_s() {
        // Measured values, jitter excluded: 400, 800, 1600, 3200, 6000 (capped).
        let steps: Vec<u64> = (0..6)
            .map(|attempt| backoff(attempt, 0).as_millis() as u64)
            .collect();
        assert_eq!(steps, vec![400, 800, 1600, 3200, 6000, 6000]);
        assert_eq!(backoff(0, 250).as_millis() as u64, 650);
        // Jitter never exceeds the 250 ms the SPA adds, even if asked for more.
        assert_eq!(backoff(0, 9_000).as_millis() as u64, 650);
    }

    #[test]
    fn a_provider_refusal_is_final_and_says_so() {
        let err = ApiError::Provider {
            status: 503,
            code: "deezer-unreachable".into(),
            message: "Deezer est injoignable".into(),
            retry_in: 30,
        };
        assert!(!err.retryable());
        assert_eq!(err.retry_after(), Some(30));
        assert_eq!(err.cause_fr(), "Deezer est injoignable");
    }

    #[test]
    fn a_dead_network_and_a_rented_session_are_retryable_in_the_right_sense() {
        assert!(ApiError::Offline.retryable());
        assert_eq!(
            ApiError::Offline.cause_fr(),
            "le serveur ne répond pas (réseau)"
        );
        assert!(!ApiError::Unauthorized.retryable());
        assert_eq!(
            ApiError::Unauthorized.cause_fr(),
            "session expirée : reconnectez-vous"
        );
        assert!(ApiError::NotReady { retry_after: 2 }.retryable());
        assert_eq!(ApiError::NotReady { retry_after: 2 }.retry_after(), Some(2));
    }

    /// A `409` the server means as "wait" and a `409` the server means as "the
    /// world moved" are two different failures, and only the first is retried
    /// as it was. Measured through the two real routes in
    /// `tests/bench.rs` (`/stream/<id>?start=` and a stale playlist
    /// `version`), so the pair cannot drift from the server.
    #[test]
    fn a_conflict_is_not_a_not_ready_and_says_which_half_moved() {
        let conflict = ApiError::Conflict {
            message: "conflict".into(),
            code: None,
            scope: Some("content".into()),
        };
        assert!(
            !conflict.retryable(),
            "re-sending the same body re-conflicts"
        );
        assert_eq!(conflict.retry_after(), None);
        assert_eq!(
            conflict.cause_fr(),
            "la playlist a changé : la liste a été relue"
        );
        assert_eq!(
            ApiError::Conflict {
                message: "track is available".into(),
                code: None,
                scope: None,
            }
            .cause_fr(),
            "cette action est impossible dans l'état actuel"
        );
        // The two are distinct values, never the same shape read two ways.
        assert_ne!(
            conflict,
            ApiError::NotReady { retry_after: 0 },
            "a caller matching one must not catch the other"
        );
        // The server's machine code survives the classification: Réglages →
        // Utilisateurs and Serveur print their own sentence per code, and those
        // refusals are 409s.
        let coded = ApiError::Conflict {
            message: "Impossible : c'est le dernier administrateur actif.".into(),
            code: Some("last_admin".into()),
            scope: None,
        };
        assert_eq!(coded.code(), Some("last_admin"));
        assert_eq!(
            ApiError::Http {
                status: 403,
                message: "forbidden".into(),
                code: Some("forbidden".into()),
            }
            .code(),
            Some("forbidden")
        );
        assert_eq!(ApiError::Offline.code(), None);
    }

    /// SC3's two refusals. A certificate is a PERSON's decision (re-pin it, or
    /// fix the server), so it is never retried by the client, and a refusal this
    /// client made itself has nothing to retry either. Both already carry their
    /// French sentence, which is what `LoadFailed` shows.
    #[test]
    fn a_certificate_and_an_invalid_request_are_not_retried() {
        let changed = ApiError::Certificate {
            sha256: "AB:CD".into(),
            changed: true,
        };
        assert_eq!(changed.cause_fr(), "Le certificat du serveur a changé");
        assert!(!changed.retryable());
        assert_eq!(changed.code(), None);
        assert_eq!(changed.retry_after(), None);

        let unrecognised = ApiError::Certificate {
            sha256: "AB:CD".into(),
            changed: false,
        };
        assert_eq!(
            unrecognised.cause_fr(),
            "Le certificat du serveur n'est pas reconnu"
        );
        assert_ne!(
            changed, unrecognised,
            "a pinned leaf that changed and a chain nobody vouched for are two different answers"
        );

        let refused = ApiError::Invalid(
            "adresse refusée (http://ailleurs/x) : le trafic en clair n'est permis que vers le serveur configuré"
                .into(),
        );
        assert!(
            refused.cause_fr().contains("en clair"),
            "the sentence says why: {}",
            refused.cause_fr()
        );
        assert!(!refused.retryable());
        assert_eq!(refused.retry_after(), None);
    }
}
