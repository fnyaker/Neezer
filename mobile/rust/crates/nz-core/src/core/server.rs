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
//! - **`probe_certificate` is SC3's handshake-only probe**, joined to this crate:
//!   the leaf it records is the one the setup screen shows before trusting it,
//!   and the one `ReleaseTlsTrustTest` asks a RELEASE build to produce (v1.6.6 —
//!   the only call that makes the PLATFORM verifier answer, and therefore the
//!   only one that proves R8 kept the JNI-reached verifier class).

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
    /// `nz_api::Api::probe_certificate`, which MB4's setup probe reads the leaf
    /// through.
    ///
    /// It was an explicit `Unsupported` while SC3's prober did not exist; it
    /// does now, and this is its one FFI-facing door (`NzCore.probeCertificate`).
    /// The instrumented `ReleaseTlsTrustTest` calls it against a Let's Encrypt
    /// host on a MINIFIED build: the platform verifier is the only thing that can
    /// answer `system_trusted`, so a release whose R8 config stripped
    /// `org.rustls.platformverifier.CertificateVerifier` fails there (v1.6.5).
    pub async fn probe_certificate(&self, url: &str) -> Result<CertificateInfo, CoreError> {
        // Rule 3: a radio that is down answers at once, in zero sockets.
        if !self.is_online() {
            return Err(CoreError::Offline);
        }
        let info = nz_api::Api::probe_certificate(url)
            .await
            .map_err(CoreError::from)?;
        Ok(CertificateInfo {
            sha256: info.sha256,
            subject: info.subject,
            issuer: info.issuer,
            not_after: info.not_after,
            system_trusted: info.system_trusted,
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

    /// **End to end: the pin `build_api_with` hands the client, joined with the
    /// refusal `nz-api` performs.** Two leaves on ONE loopback origin — A, which
    /// the person pinned, then B, which the same server presents afterwards —
    /// and an ORDINARY request through `core.api()` (not the setup probe):
    ///
    /// - against A it reaches the server (application bytes are counted there);
    /// - against B it is `CoreError::Certificate { changed: true }`, the error
    ///   names the leaf that ANSWERED, and the server receives **0** bytes.
    ///
    /// The two halves being proved apart (this file's
    /// `the_core_client_carries_the_pinned_leaf`, and SC3's
    /// `another_certificate_on_the_same_origin_is_a_change`) was exactly the
    /// hole a future change could reopen; this test is the joint.
    ///
    /// The French sentence ("Le certificat du serveur a changé") is
    /// `nz-ffi::nz_error_text`'s, asserted in `nz-ffi/src/setup.rs`, because
    /// `nz-core` writes no UI copy.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_ordinary_request_refuses_the_certificate_that_changed_after_pinning() {
        use crate::core::NetworkState;

        let (port, seen, server) = tls_script(vec![
            TlsHop::new(CERT_A, KEY_A, 404, br#"{"error":"pas de pochette"}"#),
            TlsHop::new(CERT_B, KEY_B, 200, b"unused"),
        ]);
        let origin = format!("https://127.0.0.1:{port}");
        let dir = TempDir::new("pin-changed");
        let core = dir.core("test");
        // The origin first, offline (zero sockets), then the pin: the client of
        // THIS process is rebuilt over it (the completion MB4 brought to MB2).
        core.set_network(NetworkState::None);
        assert_eq!(
            core.set_server(&origin).await.expect_err("no link"),
            CoreError::Offline
        );
        core.pin_certificate(&origin, &fingerprint_of(CERT_A))
            .expect("pin A");
        core.set_network(NetworkState::Wifi { metered: false });

        // 1. An ordinary read through the pinned client: A is the leaf that was
        //    vouched for, so the handshake completes and the request goes out.
        let cover = format!("{origin}/api/cover/1");
        let first = core.cover_path(&cover, 128).await;
        assert!(
            matches!(first, Err(CoreError::Server { status: 404, .. })),
            "the request reached A and read its answer: {first:?}"
        );
        let connection = seen
            .recv_timeout(std::time::Duration::from_secs(20))
            .expect("A answered");
        assert!(
            connection.application_bytes() > 0,
            "the request was really sent over the pinned leaf"
        );
        assert!(
            connection.head.starts_with("GET /api/cover/1"),
            "{}",
            connection.head
        );

        // 2. The same origin now presents B. An ordinary read again — no probe,
        //    no setup screen — and it is refused before a single byte of the
        //    request is written.
        let refused = core
            .cover_path(&cover, 128)
            .await
            .expect_err("B is not pinned");
        match refused {
            CoreError::Certificate { sha256, changed } => {
                assert!(changed, "a pin was held and another leaf came");
                assert_eq!(
                    sha256,
                    fingerprint_of(CERT_B),
                    "the error names the certificate that answered"
                );
            }
            other => panic!("expected a changed certificate, got {other:?}"),
        }
        let connection = seen
            .recv_timeout(std::time::Duration::from_secs(20))
            .expect("B refused");
        assert_eq!(
            connection.application_bytes(),
            0,
            "not one application byte on a channel nobody vouched for"
        );
        server.join().expect("the server thread");
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

    // --- a real TLS server that changes its certificate per connection --------
    //
    // SC3's throwaway leaves (`nz-api/tests/fixtures/tls/`, ECDSA P-256,
    // self-signed, `CN=127.0.0.1`, valid a hundred years), copied here because a
    // crate's `tests/` are not reachable from another crate. A key that signs
    // nothing is fine to commit.
    const CERT_A: &[u8] = include_bytes!("fixtures/tls/cert_a.der");
    const KEY_A: &[u8] = include_bytes!("fixtures/tls/key_a.der");
    const CERT_B: &[u8] = include_bytes!("fixtures/tls/cert_b.der");
    const KEY_B: &[u8] = include_bytes!("fixtures/tls/key_b.der");

    // ---- probe_certificate (v1.6.6) -------------------------------------

    /// `Core::probe_certificate` answers what SC3's prober answers: the leaf the
    /// loopback server presents comes back with the fingerprint `Ssl.kt` spells
    /// and `system_trusted == false` — a throwaway leaf is nobody's CA. This is
    /// the call `ReleaseTlsTrustTest` makes on a device (with a Let's Encrypt
    /// host, where the verdict must be true).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_core_certificate_probe_learns_a_self_signed_leaf() {
        let (port, _seen, server) = tls_script(vec![TlsHop::new(CERT_A, KEY_A, 200, b"{}")]);
        let url = format!("https://127.0.0.1:{port}");
        let dir = TempDir::new("probe-cert");
        let core = dir.core("test");

        let info = core
            .probe_certificate(&url)
            .await
            .expect("the leaf recorded");
        assert_eq!(
            info.sha256,
            fingerprint_of(CERT_A),
            "the fingerprint the setup screen shows and pins"
        );
        assert!(!info.system_trusted, "no public authority signs this leaf");
        assert!(!info.issuer.is_empty(), "its names were read off the DER");
        server.join().expect("the server thread");
    }

    /// Rule 3: no link, no socket, no wait — the same gate `set_server` takes.
    #[tokio::test]
    async fn a_certificate_probe_offline_answers_at_once() {
        let dir = TempDir::new("probe-cert-offline");
        let core = dir.core("test");
        core.set_network(crate::core::NetworkState::None);
        assert_eq!(
            core.probe_certificate("https://neezer.example")
                .await
                .expect_err("no link"),
            CoreError::Offline
        );
        assert!(core.stats().is_empty(), "not one socket was opened");
    }

    /// The SHA-256 `Ssl.kt` spells, computed by the crate that owns the rule.
    fn fingerprint_of(cert: &'static [u8]) -> String {
        nz_api::fingerprint(&rustls::pki_types::CertificateDer::from(cert.to_vec()))
    }

    /// What the server saw on one connection.
    struct Seen {
        head: String,
    }

    impl Seen {
        /// Application bytes the server READ: the request head, and a body only
        /// when it arrived in the same TLS record. `0` therefore means the
        /// client wrote nothing at all.
        fn application_bytes(&self) -> usize {
            self.head.len()
        }
    }

    /// One connection's certificate and its answer, in the order the client asks.
    struct TlsHop {
        cert: &'static [u8],
        key: &'static [u8],
        status: u16,
        body: Vec<u8>,
    }

    impl TlsHop {
        fn new(cert: &'static [u8], key: &'static [u8], status: u16, body: &[u8]) -> Self {
            Self {
                cert,
                key,
                status,
                body: body.to_vec(),
            }
        }
    }

    /// A loopback TLS server on one port that presents a DIFFERENT certificate
    /// per connection (`script`), answers it, and reports what it read. Nothing
    /// leaves the loopback and no port is assumed.
    fn tls_script(
        script: Vec<TlsHop>,
    ) -> (
        u16,
        std::sync::mpsc::Receiver<Seen>,
        std::thread::JoinHandle<()>,
    ) {
        use std::io::{Read as _, Write as _};
        use std::sync::mpsc::channel;

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("its address").port();
        let (sender, seen) = channel();
        let handle = std::thread::spawn(move || {
            for hop in script {
                let Ok((socket, _)) = listener.accept() else {
                    return;
                };
                // A guard, not a wait: a client that hangs must not hang CI.
                let _ = socket.set_read_timeout(Some(std::time::Duration::from_secs(20)));
                let config = rustls::ServerConfig::builder()
                    .with_no_client_auth()
                    .with_single_cert(
                        vec![rustls::pki_types::CertificateDer::from(hop.cert.to_vec())],
                        rustls::pki_types::PrivateKeyDer::Pkcs8(
                            rustls::pki_types::PrivatePkcs8KeyDer::from(hop.key.to_vec()),
                        ),
                    )
                    .expect("the fixture is a certificate and its key");
                let Ok(connection) = rustls::ServerConnection::new(std::sync::Arc::new(config))
                else {
                    return;
                };
                let mut stream = rustls::StreamOwned::new(connection, socket);
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while let Ok(1) = stream.read(&mut byte) {
                    head.push(byte[0]);
                    if head.ends_with(b"\r\n\r\n") {
                        break;
                    }
                    if head.len() > 16 * 1024 {
                        break;
                    }
                }
                let head = String::from_utf8_lossy(&head).into_owned();
                let mut response = format!(
                    "HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    hop.status,
                    hop.body.len()
                )
                .into_bytes();
                response.extend_from_slice(&hop.body);
                let _ = stream.write_all(&response);
                let _ = stream.flush();
                let _ = sender.send(Seen { head });
            }
        });
        (port, seen, handle)
    }

    fn testing_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
    }
}
