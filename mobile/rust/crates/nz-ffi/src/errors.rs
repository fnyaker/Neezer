//! `NzError` — the ONE error the app's screens ever see, and its French
//! sentence (docs/agents/mobile.md, "The FFI boundary"). Owned by **MB2**.
//!
//! Every variant becomes a `nzErrorText(...)` the UI can show as is: a cause,
//! never a protocol code (rule 3). Cells the person cannot act on (`Provider`,
//! `Server`) still carry their status, but only for the diagnostic log.

use std::fmt;

/// The failure of any call across the boundary.
///
/// The variants are the ones the overlap of `nz-api`'s errors, the session and
/// the stream makes: a screen never invents a fourth spelling of "hors ligne".
#[derive(uniffi::Error, Debug, Clone)]
pub enum NzError {
    /// No usable network at all — `ConnectivityManager` already said so, so no
    /// request was even attempted.
    Offline,
    /// The request exceeded its own deadline (4/12/15 s, `lib/api.js#req`).
    Timeout,
    /// The session is gone (a `401`): `SessionListener::on_session_ended` also
    /// fired, so the shell goes back to the login screen.
    Session,
    /// The server stopped serving this remote/party session. `restored` says
    /// whether local playback took over by itself.
    RemoteEnded { restored: bool },
    /// The server is not ready yet (a `503` with a `Retry-After`): the UI may
    /// offer to wait exactly that long.
    NotReady { retry_after_s: u32 },
    /// The server refused the change because someone else changed the same
    /// thing (the `409`s: playlist revision, role). `detail` is the server's own
    /// sentence, empty when it had none.
    ///
    /// The field is `detail` and NOT `message`, and that is not a taste: UniFFI
    /// generates one Kotlin subclass per variant, and a property called
    /// `message` collides with `Throwable.message` — the generated file does not
    /// compile ("'message' hides member of supertype 'Throwable'"). Measured on
    /// this machine before the first lot.
    Conflict { detail: String },
    /// A provider (Deezer, YouTube) refused or failed. Never a verdict about a
    /// track (CLAUDE.md).
    Provider {
        status: u16,
        code: Option<String>,
        detail: String,
    },
    /// Our own server answered an error status.
    Server { status: u16, detail: String },
    /// The TLS chain was refused, or the pinned leaf is not the one presented.
    /// `sha256` is the leaf actually seen, colon-separated hex.
    Certificate { sha256: String, changed: bool },
    /// The server predates the feature (rule 7): say which, and hide the entry
    /// rather than failing every time.
    Unsupported { feature: String },
    /// The caller passed something the server would refuse anyway.
    Invalid { detail: String },
    /// A bug, or something the app could not classify. Logged with its cause.
    Internal { detail: String },
}

impl fmt::Display for NzError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&nz_error_text(self))
    }
}

impl std::error::Error for NzError {}

/// The French sentence a screen shows for `e` (`ApiError::cause_fr`'s register:
/// what happened, in the person's words, never a code).
#[uniffi::export]
pub fn nz_error_text(e: &NzError) -> String {
    match e {
        NzError::Offline => "Hors ligne".into(),
        NzError::Timeout => "Le serveur ne répond pas".into(),
        NzError::Session => "Session expirée, reconnecte-toi".into(),
        NzError::RemoteEnded { restored: true } => {
            "La lecture à distance est terminée, la lecture locale a repris".into()
        }
        NzError::RemoteEnded { restored: false } => "La lecture à distance est terminée".into(),
        NzError::NotReady { retry_after_s } => format!(
            "Le serveur se prépare, réessaie dans {} s",
            (*retry_after_s).max(1)
        ),
        NzError::Conflict { detail } => {
            if detail.is_empty() {
                "Quelqu'un d'autre a modifié la même chose".into()
            } else {
                detail.clone()
            }
        }
        NzError::Provider {
            status,
            code,
            detail,
        } => match code {
            Some(code) if !code.is_empty() => format!("Deezer : {} ({})", detail, code),
            _ => format!("Deezer : {} ({})", detail, status),
        },
        NzError::Server { status, detail } => {
            if detail.is_empty() {
                format!("Le serveur a répondu {}", status)
            } else {
                format!("Le serveur a répondu {} : {}", status, detail)
            }
        }
        NzError::Certificate { changed: true, .. } => "Le certificat du serveur a changé".into(),
        NzError::Certificate { changed: false, .. } => {
            "Le certificat du serveur n'est pas accepté".into()
        }
        NzError::Unsupported { feature } => {
            format!(
                "Le serveur est trop ancien pour cette fonction ({})",
                feature
            )
        }
        NzError::Invalid { detail } => {
            if detail.is_empty() {
                "Demande invalide".into()
            } else {
                detail.clone()
            }
        }
        NzError::Internal { detail } => format!("Erreur interne : {}", detail),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every variant has a sentence, and no sentence is a bare code: this is
    /// what `LoadFailed` shows (rule 3).
    #[test]
    fn every_variant_has_a_french_sentence() {
        let all = [
            NzError::Offline,
            NzError::Timeout,
            NzError::Session,
            NzError::RemoteEnded { restored: true },
            NzError::RemoteEnded { restored: false },
            NzError::NotReady { retry_after_s: 0 },
            NzError::Conflict {
                detail: String::new(),
            },
            NzError::Provider {
                status: 500,
                code: Some("DATA_ERROR".into()),
                detail: "indisponible".into(),
            },
            NzError::Server {
                status: 502,
                detail: String::new(),
            },
            NzError::Certificate {
                sha256: "ab:cd".into(),
                changed: true,
            },
            NzError::Certificate {
                sha256: "ab:cd".into(),
                changed: false,
            },
            NzError::Unsupported {
                feature: "mb0".into(),
            },
            NzError::Invalid {
                detail: String::new(),
            },
            NzError::Internal {
                detail: "rien".into(),
            },
        ];
        for e in all {
            let text = nz_error_text(&e);
            assert!(!text.trim().is_empty(), "{e:?} has no sentence");
            assert!(
                text.chars().any(|c| c.is_alphabetic()),
                "{e:?} reads like a code: {text}"
            );
        }
        assert_eq!(nz_error_text(&NzError::Offline), "Hors ligne");
        assert!(nz_error_text(&NzError::NotReady { retry_after_s: 0 }).contains("1 s"));
    }
}
