//! What the core can fail with, and how `nz-api`'s failures map onto it.
//!
//! The variants are the FFI's own (`nz-ffi::errors::NzError`), so a screen sees
//! one register of failures and `nz_error_text` owns every French sentence:
//! `nz-core` never writes UI copy, and `nz-ffi` only converts. The mapping lives
//! here, once, so "a `401` is the session and a `409` with `Retry-After` is not
//! a conflict" is decided in one place instead of at each call site.

use nz_api::ApiError;

/// The failure of one core call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    /// No usable link: `ConnectivityManager` said so, so no request was sent.
    Offline,
    /// The request outlived its own deadline.
    Timeout,
    /// The session is gone (a `401`).
    Session,
    /// A remote-control loan ended under the request.
    RemoteEnded { restored: bool },
    /// The server is not ready yet; wait exactly `retry_after_s`.
    NotReady { retry_after_s: u32 },
    /// The world moved under this client (a `409` without `Retry-After`).
    Conflict { detail: String },
    /// A provider (Deezer, YouTube) refused or failed.
    Provider {
        status: u16,
        code: Option<String>,
        detail: String,
    },
    /// Our own server answered an error status.
    Server { status: u16, detail: String },
    /// The TLS chain was refused, or the pinned leaf is not the one presented.
    Certificate { sha256: String, changed: bool },
    /// The server predates the feature (rule 7).
    Unsupported { feature: String },
    /// Something the server would refuse anyway.
    Invalid { detail: String },
    /// A bug, or a cause the core could not classify.
    Internal { detail: String },
}

impl CoreError {
    /// A short, stable name for the diagnostic journal. Never shown to a person
    /// (`nz_error_text` is the only place French copy lives).
    pub fn kind(&self) -> &'static str {
        match self {
            CoreError::Offline => "offline",
            CoreError::Timeout => "timeout",
            CoreError::Session => "session",
            CoreError::RemoteEnded { .. } => "remote_ended",
            CoreError::NotReady { .. } => "not_ready",
            CoreError::Conflict { .. } => "conflict",
            CoreError::Provider { .. } => "provider",
            CoreError::Server { .. } => "server",
            CoreError::Certificate { .. } => "certificate",
            CoreError::Unsupported { .. } => "unsupported",
            CoreError::Invalid { .. } => "invalid",
            CoreError::Internal { .. } => "internal",
        }
    }
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The journal's line: the kind and what the wire said, never a cookie.
        match self {
            CoreError::Provider {
                status,
                code,
                detail,
            } => write!(f, "provider {status} {code:?}: {detail}"),
            CoreError::Server { status, detail } => write!(f, "server {status}: {detail}"),
            CoreError::Conflict { detail } => write!(f, "conflict: {detail}"),
            CoreError::NotReady { retry_after_s } => write!(f, "not ready, {retry_after_s}s"),
            CoreError::Certificate { sha256, changed } => {
                write!(f, "certificate {sha256} (changed: {changed})")
            }
            CoreError::Unsupported { feature } => write!(f, "unsupported: {feature}"),
            CoreError::Invalid { detail } => write!(f, "invalid: {detail}"),
            CoreError::Internal { detail } => write!(f, "internal: {detail}"),
            CoreError::RemoteEnded { restored } => write!(f, "remote ended (restored: {restored})"),
            other => f.write_str(other.kind()),
        }
    }
}

impl std::error::Error for CoreError {}

/// `nz-api`'s failures, read the way the phone shows them.
///
/// The two `409`s stay told apart (`NotReady` carries the server's wait, a
/// `Conflict` is a verdict), a `Busy` is the same "wait exactly this long" as a
/// `NotReady` because that is what the person sees, and a `503`-style `Http` is
/// a `Server` refusal with its own status — never a verdict about a track.
impl From<ApiError> for CoreError {
    fn from(error: ApiError) -> Self {
        match error {
            ApiError::Offline => CoreError::Offline,
            ApiError::Unauthorized => CoreError::Session,
            ApiError::RemoteEnded { restored } => CoreError::RemoteEnded {
                restored: restored.is_some(),
            },
            ApiError::Provider {
                status,
                code,
                message,
                ..
            } => CoreError::Provider {
                status,
                code: Some(code),
                detail: message,
            },
            ApiError::NotReady { retry_after } | ApiError::Busy { retry_after } => {
                CoreError::NotReady {
                    retry_after_s: retry_after,
                }
            }
            ApiError::Conflict { message, code, .. } => CoreError::Conflict {
                detail: if message.is_empty() {
                    code.unwrap_or_default()
                } else {
                    message
                },
            },
            ApiError::Http {
                status,
                message,
                code,
            } => CoreError::Server {
                status,
                detail: match code {
                    Some(code) if !message.is_empty() => format!("{code}: {message}"),
                    Some(code) => code,
                    None => message,
                },
            },
            ApiError::Decode(detail) => CoreError::Internal { detail },
        }
    }
}

/// The local store's failures are causes a screen can print too (a full disk, a
/// database written by a newer build): they are internal, never "offline".
impl From<nz_cache::Error> for CoreError {
    fn from(error: nz_cache::Error) -> Self {
        CoreError::Internal {
            detail: error.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two `409`s keep their own meaning through the mapping, which is what
    /// lets a screen offer "wait 30 s" or "re-read, it moved".
    #[test]
    fn the_two_conflicts_stay_told_apart() {
        let not_ready: CoreError = ApiError::NotReady { retry_after: 30 }.into();
        assert_eq!(
            not_ready,
            CoreError::NotReady { retry_after_s: 30 },
            "a 409 with Retry-After is a wait"
        );
        let moved: CoreError = ApiError::Conflict {
            message: "version".into(),
            code: Some("stale".into()),
            scope: Some("content".into()),
        }
        .into();
        assert_eq!(
            moved,
            CoreError::Conflict {
                detail: "version".into()
            }
        );
        // A background `503` is the same wait for the person.
        assert_eq!(
            CoreError::from(ApiError::Busy { retry_after: 12 }),
            CoreError::NotReady { retry_after_s: 12 }
        );
    }

    /// A `401` is the session, every time: the FFI emits `SessionEnded` from it.
    #[test]
    fn a_401_is_the_session() {
        assert_eq!(CoreError::from(ApiError::Unauthorized), CoreError::Session);
    }

    /// The journal names a kind for every variant, and no kind is empty.
    #[test]
    fn every_variant_has_a_kind() {
        let all = [
            CoreError::Offline,
            CoreError::Timeout,
            CoreError::Session,
            CoreError::RemoteEnded { restored: true },
            CoreError::NotReady { retry_after_s: 5 },
            CoreError::Conflict { detail: "x".into() },
            CoreError::Provider {
                status: 502,
                code: None,
                detail: "x".into(),
            },
            CoreError::Server {
                status: 500,
                detail: "x".into(),
            },
            CoreError::Certificate {
                sha256: "ab".into(),
                changed: false,
            },
            CoreError::Unsupported {
                feature: "x".into(),
            },
            CoreError::Invalid { detail: "x".into() },
            CoreError::Internal { detail: "x".into() },
        ];
        for error in all {
            assert!(!error.kind().is_empty(), "{error:?}");
            assert!(!error.to_string().is_empty(), "{error:?}");
        }
    }
}
