//! The configured server: its origin, its pin, and the one probe that proves it
//! answers. Owned by **MB2**.
//!
//! Three decisions the phone makes and the desktop does not:
//!
//! - **The origin is stored BEFORE it is probed.** A server that is slow, off,
//!   or behind a captive portal is still the server the person chose; making
//!   them retype it because `GET /api/version` timed out would be rule 3's
//!   opposite. The screen shows the cause and "Réessayer".
//! - **The pin is remembered per origin**, in the same `kv` store as every other
//!   setting, and cleared with an empty hash. `nz-api`'s TOFU verifier (SC3) is
//!   what enforces it on the handshake; this file is what survives a restart.
//! - **`probe_certificate` needs SC3's handshake-only probe**, which `nz-api`
//!   does not have yet: until it lands, the answer is an explicit `Unsupported`
//!   (the setup screen hides the control) rather than a made-up fingerprint.

use std::sync::Arc;

use nz_api::{Api, Lane};

use super::{
    Core, CoreError, KEY_ORIGIN, KEY_PIN_PREFIX, KEY_VERSION, Origin, ServerInfo, TAG_SESSION,
};

/// What a handshake-only probe learnt (SC3's `CertificateInfo`, for MB4's
/// screen). Declared here so the FFI's record has a core type to convert from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertificateInfo {
    /// The leaf's SHA-256, colon-separated hex.
    pub sha256: String,
    pub subject: String,
    pub issuer: String,
    /// ISO-8601.
    pub not_after: String,
    /// Whether Android's own trust store accepts the chain.
    pub system_trusted: bool,
}

impl Core {
    /// Builds and installs the one HTTP client for `origin`, over the current
    /// cookie jar.
    pub(super) fn build_api(&self, origin: &Origin) -> Result<Arc<Api>, CoreError> {
        let jar = self.jar();
        self.build_api_with(origin, jar)
    }

    /// The same, over a jar the caller just replaced (`session.rs`'s offline
    /// logout must not build a client on the cookie it just forgot).
    ///
    /// **The pin is handed to every client** (SC3's `pinned_leaf_sha256`): a
    /// chain the platform accepts OR the one leaf the person vouched for passes,
    /// and another leaf is `ApiError::Certificate { changed: true }` — "le
    /// certificat a changé" — whichever request asked. MB2 stored the pin and
    /// MB4 makes it count, because a pin that only the setup screen reads would
    /// leave every real request walking over a replaced certificate.
    pub(super) fn build_api_with(
        &self,
        origin: &Origin,
        jar: Arc<nz_api::SessionJar>,
    ) -> Result<Arc<Api>, CoreError> {
        let cache: Arc<dyn nz_api::ResponseCache> = Arc::new(self.cache().clone());
        let pin = self
            .pinned(origin)
            .and_then(|sha256| nz_api::parse_fingerprint(&sha256));
        let api = Api::builder(origin.as_str())
            .session(jar)
            .cache(cache)
            .pinned_leaf_sha256(pin)
            .build()?;
        // A client built while the radio is down starts down: the first request
        // then answers `Offline` from `nz-api`'s own watch too.
        api.set_online(self.is_online());
        Ok(Arc::new(api))
    }

    /// Saves the server, then checks it answers `GET /api/version`.
    pub async fn set_server(&self, url: &str) -> Result<ServerInfo, CoreError> {
        let origin = Origin::parse(url).ok_or_else(|| CoreError::Invalid {
            detail: "Adresse du serveur invalide".to_owned(),
        })?;
        if !origin.is_secure() {
            // Allowed: LAN installs are HTTP (`docs/agents/desktop.md`, *Auth*).
            // The setup screen warns; the redirect rule makes it safe.
            tracing::info!(origin = %origin.as_str(), "cleartext server (LAN install)");
        }

        let already = self.origin().as_ref() == Some(&origin);
        // Stored first, so a failed probe leaves the person on this server.
        self.cache().kv().set(KEY_ORIGIN, &origin.as_str())?;
        if !already {
            // A new server means a new session: the old cookie belongs to
            // another origin and must not be presented to this one.
            *self.me.write().unwrap_or_else(|err| err.into_inner()) = None;
            self.emit(super::SessionEvent::Me(None));
        }
        let api = self.build_api(&origin)?;
        {
            let mut slot = self.api.write().unwrap_or_else(|err| err.into_inner());
            *slot = Some(api.clone());
        }
        {
            let mut slot = self.server.write().unwrap_or_else(|err| err.into_inner());
            let version = slot
                .as_ref()
                .filter(|server| server.origin == origin)
                .map(|server| server.version.clone())
                .unwrap_or_default();
            *slot = Some(ServerInfo {
                pinned_sha256: self.pinned(&origin),
                origin: origin.clone(),
                version,
            });
        }

        // The probe goes through the offline GATE, not straight to the client:
        // configuring a server with no link must open no socket (rule 3).
        if !self.is_online() {
            return Err(CoreError::Offline);
        }
        // One `User`-lane GET, counted with the bytes it carried.
        let version = self.probe_version(&api).await?;
        self.cache().kv().set(KEY_VERSION, &version)?;
        let info = {
            let mut slot = self.server.write().unwrap_or_else(|err| err.into_inner());
            let info = slot.take().unwrap_or(ServerInfo {
                origin: origin.clone(),
                pinned_sha256: self.pinned(&origin),
                version: String::new(),
            });
            let info = ServerInfo { version, ..info };
            *slot = Some(info.clone());
            info
        };
        Ok(info)
    }

    /// `GET /api/version`, read through the BYTES so the counter knows exactly
    /// what the screen cost (`nz-api`'s typed get consumes the body internally).
    async fn probe_version(&self, api: &Arc<Api>) -> Result<String, CoreError> {
        let body = api.bytes(Lane::User, "/version").await?;
        self.meter().record(TAG_SESSION, body.len() as u64, 0);
        let version: nz_model::Version =
            serde_json::from_slice(&body).map_err(|error| CoreError::Internal {
                detail: format!("version illisible : {error}"),
            })?;
        Ok(version.version.unwrap_or_default())
    }

    /// The pinned leaf for `origin`, colon-separated hex.
    pub fn pinned(&self, origin: &Origin) -> Option<String> {
        self.cache()
            .kv()
            .get::<String>(&format!("{KEY_PIN_PREFIX}{}", origin.as_str()))
    }

    /// Pins one leaf for one origin (TOFU), or clears it with an empty hash.
    ///
    /// The origin is normalized first: a pin learnt with `https://Host/` must
    /// match the `https://host:443` every request compares against.
    pub fn pin_certificate(&self, origin: &str, sha256: &str) -> Result<(), CoreError> {
        let origin = Origin::parse(origin).ok_or_else(|| CoreError::Invalid {
            detail: "Adresse du serveur invalide".to_owned(),
        })?;
        let key = format!("{KEY_PIN_PREFIX}{}", origin.as_str());
        let sha256 = sha256.trim();
        if sha256.is_empty() {
            self.cache().kv().delete(&key)?;
        } else {
            if !looks_like_sha256(sha256) {
                return Err(CoreError::Invalid {
                    detail: "Empreinte de certificat invalide".to_owned(),
                });
            }
            self.cache().kv().set(&key, &sha256)?;
        }
        let mut slot = self.server.write().unwrap_or_else(|err| err.into_inner());
        if let Some(server) = slot.as_mut()
            && server.origin == origin
        {
            server.pinned_sha256 = if sha256.is_empty() {
                None
            } else {
                Some(sha256.to_owned())
            };
        }
        drop(slot);
        // The client that carries every request is rebuilt over the new pin, so
        // accepting (or clearing) one takes effect in THIS process — a pin that
        // only counted from the next launch would leave the session the person
        // just configured on the old verdict.
        if self.origin().as_ref() == Some(&origin)
            && let Ok(api) = self.build_api(&origin)
        {
            *self.api.write().unwrap_or_else(|err| err.into_inner()) = Some(api);
        }
        Ok(())
    }

    /// Learns a certificate with a handshake that sends nothing — SC3's
    /// `nz-api::probe_certificate`, which MB4's setup screen calls. Not in
    /// `nz-api` yet, so the answer says exactly that instead of inventing a
    /// fingerprint (rule 7: hide the control, do not fail every time).
    pub async fn probe_certificate(&self, _url: &str) -> Result<CertificateInfo, CoreError> {
        Err(CoreError::Unsupported {
            feature: "mb4:probe_certificate".to_owned(),
        })
    }
}

/// A SHA-256 in the colon-separated hex `Ssl.kt` shows (`ab:cd:…`), or the bare
/// 64 hex digits a person may paste.
fn looks_like_sha256(raw: &str) -> bool {
    let digits: String = raw.chars().filter(|c| *c != ':' && *c != ' ').collect();
    digits.len() == 64 && digits.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::testing::TempDir;

    #[test]
    fn a_pin_is_normalized_and_survives_a_restart() {
        let dir = TempDir::new("pin");
        let core = dir.core("test");
        let hash = "aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:99";
        core.pin_certificate("https://Neezer.Example/", hash)
            .expect("pin");
        let origin = Origin::parse("https://neezer.example").unwrap();
        assert_eq!(core.pinned(&origin).as_deref(), Some(hash));

        // A new core on the same directories remembers it (rule 7).
        let again = dir.core("test");
        assert_eq!(again.pinned(&origin).as_deref(), Some(hash));

        again
            .pin_certificate("https://neezer.example", "")
            .expect("clear");
        assert!(again.pinned(&origin).is_none());
    }

    #[test]
    fn a_refused_origin_or_hash_is_never_stored() {
        let dir = TempDir::new("pin-invalid");
        let core = dir.core("test");
        assert!(matches!(
            core.pin_certificate("neezer.example", "aa"),
            Err(CoreError::Invalid { .. })
        ));
        let origin = Origin::parse("https://neezer.example").unwrap();
        assert!(core.pinned(&origin).is_none(), "nothing was written");
        assert!(matches!(
            core.pin_certificate("https://neezer.example", "pas une empreinte"),
            Err(CoreError::Invalid { .. })
        ));
        assert!(core.pinned(&origin).is_none());
    }

    #[test]
    fn a_sha256_is_64_hex_digits_with_or_without_colons() {
        assert!(looks_like_sha256(
            "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff"
        ));
        assert!(looks_like_sha256(
            "00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff"
        ));
        assert!(!looks_like_sha256("0011"));
        assert!(!looks_like_sha256(
            "zz112233445566778899aabbccddeeff00112233445566778899aabbccddeeff"
        ));
    }

    #[test]
    fn a_bad_origin_is_refused_before_anything_is_stored() {
        let dir = TempDir::new("server-bad");
        let core = dir.core("test");
        let runtime = testing_runtime();
        let error = runtime
            .block_on(core.set_server("javascript:alert(1)"))
            .expect_err("refused");
        assert!(matches!(error, CoreError::Invalid { .. }));
        assert!(core.server().is_none());
        assert_eq!(
            core.cache().kv().get::<String>(KEY_ORIGIN),
            None,
            "nothing was written for a URL that is not an origin"
        );
    }

    /// **The pin reaches EVERY request, not only the setup screen** (MB4's
    /// half of SC3): the client the core hands out carries the stored leaf, and
    /// a pin accepted in this process is on the client of this process — no
    /// relaunch. `nz-api`'s `tests/tls.rs`
    /// (`another_certificate_on_the_same_origin_is_a_change`) proves what that
    /// client then does with a server whose certificate changed: an ordinary
    /// `GET` is refused with `ApiError::Certificate { changed: true }`, zero
    /// application bytes sent, sentence "Le certificat du serveur a changé".
    #[test]
    fn the_core_client_carries_the_pinned_leaf() {
        let pin = "AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99";
        let dir = TempDir::new("pin-client");
        let core = dir.core("test");
        // Offline: the origin is stored and a client built, without a socket
        // (rule 3's own test proves that part) — the state a first launch is in.
        core.set_network(crate::core::NetworkState::None);
        let runtime = testing_runtime();
        let refused = runtime
            .block_on(core.set_server("https://neezer.example"))
            .expect_err("no link, no probe");
        assert_eq!(refused, CoreError::Offline);
        core.set_network(crate::core::NetworkState::Wifi { metered: false });

        let unpinned = core.api().expect("a client");
        assert_eq!(
            unpinned.pinned_fingerprint(),
            None,
            "a server nobody pinned keeps the platform's verdict and nothing else"
        );

        core.pin_certificate("https://neezer.example", pin)
            .expect("pin");
        let client = core.api().expect("a client");
        assert_eq!(
            client.pinned_fingerprint(),
            Some(nz_api::hex_fingerprint(
                &nz_api::parse_fingerprint(pin).expect("a digest")
            )),
            "the client the whole app uses carries the pin, in this process"
        );
        // The client is rebuilt rather than mutated: the old handle keeps its
        // own verdict, so nothing can downgrade a request already in flight.
        assert_eq!(unpinned.pinned_fingerprint(), None);

        // A cleared pin (a renewed certificate, re-saved) takes effect at once.
        core.pin_certificate("https://neezer.example", "")
            .expect("clear");
        assert_eq!(core.api().expect("a client").pinned_fingerprint(), None);
    }

    fn testing_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
    }
}
