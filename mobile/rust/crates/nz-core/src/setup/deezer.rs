//! Linking the Deezer account from the phone (lot MB4).
//!
//! There is no email/password endpoint left that a server could call — Deezer's
//! web one answers `403` since 2024, and the mobile gateway needs keys extracted
//! from Deezer's own binaries — so the only remaining path to an ARL is a real
//! browser session: `DeezerLoginActivity` shows **Deezer's own page** in a
//! throwaway WebView with no JavaScript bridge, and reads the `arl` cookie out of
//! its jar. What arrives here is that one value, and it goes straight to the
//! server's own route (`POST /api/settings`, what `lib/nativeDeezer.js`'s caller
//! uses), which validates and probes it before storing it.
//!
//! Three rules this file keeps:
//!
//! - **The ARL never touches this device's storage.** It is not put in `kv`, not
//!   written to a file and never logged; it lives in one HTTP body and then only
//!   server-side. The Kotlin side wipes the Deezer jar before and after the
//!   screen for the same reason.
//! - **The shape is checked before the request**, with the alphabet the server
//!   itself accepts (`neezer/deezer/__init__.py`): the value goes out as a JSON
//!   field, so anything outside it is a mistake worth naming locally.
//! - **A refused ARL is a sentence, not a silent no-op**: the server answers
//!   `400` with its own French cause ("Deezer a refusé cet ARL…"), which
//!   `CoreError::Server` carries to the screen unchanged.

use serde_json::json;

use crate::core::{Core, CoreError, TAG_SESSION};

/// What the server says about the account once it has the ARL.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeezerLink {
    /// Whether the server now holds a working credential.
    pub linked: bool,
    /// The masked account the server names (`ab…yz`), when it said.
    pub hint: Option<String>,
    /// Whether the ARL came from the server's own database (an override) rather
    /// than its configuration file — what Réglages shows under the field.
    pub from_database: bool,
}

impl Core {
    /// Hands the ARL the Deezer login screen captured to the configured server.
    ///
    /// One mutation on the `User` lane (it is the action the person just took),
    /// never retried by the core: a failure is the person's to repeat.
    pub async fn save_deezer_arl(&self, arl: &str) -> Result<DeezerLink, CoreError> {
        let arl = arl.trim();
        if !looks_like_arl(arl) {
            return Err(CoreError::Invalid {
                detail: "Le jeton Deezer reçu n'a pas la bonne forme".to_owned(),
            });
        }
        let api = self.api()?;
        let body = json!({ "deezer_arl": arl });
        let sent = serde_json::to_vec(&body)
            .map(|bytes| bytes.len() as u64)
            .unwrap_or(0);
        let answer = api.set_settings(&body).await;
        self.meter().record(TAG_SESSION, 0, sent);
        let answer = answer?;
        // `{ok, deezer: {arl_set, arl_source, arl_hint, …}}`; read tolerantly —
        // an older server that answers only `{ok:true}` still linked the account.
        let deezer = answer.get("deezer");
        Ok(DeezerLink {
            linked: deezer
                .and_then(|value| value.get("arl_set"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true),
            hint: deezer
                .and_then(|value| value.get("arl_hint"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            from_database: deezer
                .and_then(|value| value.get("arl_source"))
                .and_then(serde_json::Value::as_str)
                == Some("database"),
        })
    }
}

/// The alphabet the server accepts (`valid_arl`): 32 to 255 characters of
/// `[0-9A-Za-z._-]`. Anything else is a mistyped capture, not a credential, and
/// a value outside it could only be header injection in the end.
fn looks_like_arl(raw: &str) -> bool {
    let length = raw.len();
    (32..=255).contains(&length)
        && raw
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::testing::TempDir;

    #[test]
    fn the_arl_alphabet_is_the_one_the_server_accepts() {
        // 32 characters of the accepted alphabet: the shortest real ARL.
        let shortest = "a".repeat(32);
        assert!(looks_like_arl(&shortest));
        assert!(looks_like_arl(&format!("{}._-", "A1".repeat(40))));
        assert!(!looks_like_arl(&"a".repeat(31)));
        assert!(!looks_like_arl(&"a".repeat(256)));
        // A space, a semicolon or a newline is header injection in the end.
        assert!(!looks_like_arl(&format!("{} ", "a".repeat(32))));
        assert!(!looks_like_arl(&format!("{};x", "a".repeat(32))));
        assert!(!looks_like_arl(&format!("{}\n", "a".repeat(32))));
    }

    /// A malformed ARL is refused BEFORE a request, and nothing is stored on the
    /// device either way.
    #[tokio::test]
    async fn a_malformed_arl_never_reaches_the_server() {
        let dir = TempDir::new("arl-bad");
        let core = dir.core("test");
        let error = core
            .save_deezer_arl("pas un jeton")
            .await
            .expect_err("refused");
        assert!(matches!(error, CoreError::Invalid { .. }), "{error:?}");
        assert!(core.stats().is_empty(), "not one request");
        // And nothing named after it in the settings store.
        assert!(
            core.cache()
                .kv()
                .get::<serde_json::Value>("deezer.arl")
                .is_none()
        );
    }

    /// With no server configured the answer is a cause, never a panic.
    #[tokio::test]
    async fn without_a_server_the_arl_is_not_sent_anywhere() {
        let dir = TempDir::new("arl-no-server");
        let core = dir.core("test");
        let error = core
            .save_deezer_arl(&"a".repeat(64))
            .await
            .expect_err("no server");
        assert!(matches!(error, CoreError::Invalid { .. }), "{error:?}");
        assert!(core.stats().is_empty());
    }
}
