//! Choosing the server, before anything is saved (lot MB4).
//!
//! The old app's `SetupActivity`, `Prefs.buildBaseUrl` and `Ssl.presentedFingerprint`
//! were three ideas this file keeps as one call: the person types a host, a port
//! and whether to verify the certificate; **the probe uses those values without
//! persisting them** (persisting before validating left a bad host saved when
//! somebody hit Retry then backed out), learns the certificate the server
//! presents with a handshake that sends NOTHING, and reads `GET /api/version`
//! through that very leaf.
//!
//! Three rules, each of them learned on the old app:
//!
//! - **A certificate the system does not vouch for is shown before it is
//!   trusted.** Its SHA-256 is what the person compares with
//!   `openssl x509 -noout -fingerprint -sha256` on the server, and once accepted
//!   it is the ONLY certificate this app takes for that origin (`nz-api`'s pin,
//!   enforced on every request through `core::server::build_api_with`).
//! - **Another host is never relaxed.** The pin is looked up for THIS origin and
//!   for nothing else.
//! - **A changed certificate is not a crash, it is a sentence.** When a pin is
//!   held and the server presents a different leaf, the probe still answers (a
//!   renewed certificate must be trustable from this very screen, as re-saving
//!   the settings re-pinned on the old app) but `pin_changed` is true, and the
//!   screen says "le certificat a changé" before replacing anything.
//!
//! Nothing here logs a certificate body, a cookie or a password.

use nz_api::{Api, Lane};

use crate::core::{Core, CoreError, TAG_SESSION};

/// What the setup screen has in its three fields, normalised the way
/// `Prefs.buildBaseUrl` did: the optional port is inserted into the AUTHORITY
/// only (never after a path, which produced `…/musique:5722` on a reverse-proxied
/// host), and a host without a scheme is `https`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupAddress {
    /// `music.example.org`, `https://music.example.org/musique`, `192.168.1.10`.
    pub host: String,
    /// The optional port field, as typed (empty means the scheme's default).
    pub port: String,
    /// "Vérifier le certificat": off means trust on first use, never trust-all.
    pub verify_ssl: bool,
}

/// What a probe learnt, and what the screen shows before saving.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerProbe {
    /// `scheme://host:port`, the ONE definition of "our server".
    pub origin: String,
    /// What `GET /api/version` answered (`0.1.0`, empty when the answer had no
    /// version).
    pub version: String,
    /// Whether the origin is `https` (a LAN install may be `http`, and says so).
    pub https: bool,
    /// The leaf the handshake presented, when one was learnt. `None` with
    /// verification ON: the platform already decided, there is nothing to show.
    pub cert: Option<crate::core::CertificateInfo>,
    /// Whether the platform's own store would accept this chain.
    pub system_trusted: bool,
    /// The leaf to pin once the person accepts it. `None` when the platform
    /// trusts the chain (nothing to pin) or the origin is plaintext.
    pub pin: Option<String>,
    /// A different leaf than the one already pinned for this origin: the screen
    /// says "le certificat a changé" and replaces it only on acceptance.
    pub pin_changed: bool,
    /// The server publishes the NATIVE app's APK (`/api/version`'s
    /// `android.native.apk`): the screen offers "Mettre à jour", and the fetch
    /// goes through [`Core::fetch_update`]. False on a server that predates the
    /// native channel, even when it publishes the WebView `android.apk`.
    pub update_apk: bool,
    /// The native app's version that server declares (`android.native.version`),
    /// when it names one — what the notice says next to the action.
    pub android_version: Option<String>,
}

impl SetupAddress {
    pub fn new(host: impl Into<String>, port: impl Into<String>, verify_ssl: bool) -> Self {
        Self {
            host: host.into(),
            port: port.into(),
            verify_ssl,
        }
    }

    /// The server root, `Prefs.buildBaseUrl`'s own algorithm: trim, default
    /// scheme `https`, the port field inserted into the authority only when the
    /// host does not already carry one, and the path a reverse proxy adds kept.
    pub fn base_url(&self) -> Option<String> {
        let mut raw = self.host.trim().trim_end_matches('/').to_owned();
        if raw.is_empty() {
            return None;
        }
        if !raw.contains("://") {
            raw = format!("https://{raw}");
        }
        let scheme_end = raw.find("://")? + 3;
        let scheme = &raw[..scheme_end];
        let rest = &raw[scheme_end..];
        let slash = rest.find('/');
        let authority = match slash {
            Some(at) => &rest[..at],
            None => rest,
        };
        let path = match slash {
            Some(at) => &rest[at..],
            None => "",
        };
        let port = self.port.trim();
        let authority = if !port.is_empty() && !authority.contains(':') {
            format!("{authority}:{port}")
        } else {
            authority.to_owned()
        };
        Some(format!("{scheme}{authority}{path}"))
    }

    /// The SPA's entry point (`neezer/webui/spa.py`); the migration's hidden
    /// WebView needs the same origin, not this path.
    pub fn app_url(&self) -> Option<String> {
        self.base_url().map(|base| format!("{base}/app/"))
    }
}

impl Core {
    /// Learns the certificate and checks the server answers, WITHOUT saving
    /// anything (`SetupActivity`'s rule). Only [`Core::set_server`] commits.
    ///
    /// Offline, the answer is `Offline` in **zero sockets** — the setup screen
    /// offers "continuer quand même" from that cause, exactly as the old app's
    /// "Continue anyway" did for a server reachable only on another link.
    pub async fn probe_server(&self, address: &SetupAddress) -> Result<ServerProbe, CoreError> {
        let base = address.base_url().ok_or_else(|| CoreError::Invalid {
            detail: "Adresse du serveur invalide".to_owned(),
        })?;
        let origin = crate::core::Origin::parse(&base).ok_or_else(|| CoreError::Invalid {
            detail: "Adresse du serveur invalide".to_owned(),
        })?;
        if !self.network().is_online() {
            return Err(CoreError::Offline);
        }

        let https = origin.is_secure();
        let mut cert = None;
        let mut system_trusted = true;
        let mut pin = None;
        let mut pin_changed = false;

        if https && !address.verify_ssl {
            // The handshake is refused once the leaf is recorded: not one
            // application byte leaves this client for a server nobody has
            // vouched for yet (`Ssl.presentedFingerprint`).
            let info = Api::probe_certificate(base.as_str()).await?;
            system_trusted = info.system_trusted;
            if !info.system_trusted || self.pinned(&origin).is_some() {
                pin = Some(info.sha256.clone());
                pin_changed = match self.pinned(&origin) {
                    Some(stored) => !same_fingerprint(&stored, &info.sha256),
                    None => false,
                };
            }
            cert = Some(crate::core::CertificateInfo {
                sha256: info.sha256,
                subject: info.subject,
                issuer: info.issuer,
                not_after: info.not_after,
                system_trusted: info.system_trusted,
            });
        }

        // `GET /api/version`, through the leaf the person is about to trust:
        // the probe is the setup screen's own trust decision, so a renewed
        // certificate does not make the screen unusable — accepting it is one
        // tap away, and `pin_changed` says so.
        let mut builder = Api::builder(origin.as_str());
        if let Some(hash) = pin.as_deref() {
            builder = builder.pinned_leaf_sha256(nz_api::parse_fingerprint(hash));
        }
        let api = builder.build()?;
        api.set_online(true);
        let body = api.bytes(Lane::User, "/version").await?;
        self.meter().record(TAG_SESSION, body.len() as u64, 0);
        let version: nz_model::Version =
            serde_json::from_slice(&body).map_err(|error| CoreError::Internal {
                detail: format!("version illisible : {error}"),
            })?;

        Ok(ServerProbe {
            origin: origin.as_str(),
            version: version.version.unwrap_or_default(),
            https,
            cert,
            system_trusted,
            pin,
            pin_changed,
            // The NATIVE channel only (docs/agents/mobile.md, "Deux apps
            // Android"): `android.apk` is the WebView app's file, and a native
            // app that fell back to it would download an APK for another
            // package — the installer would refuse it, after 10 MB. `None` is
            // the honest answer on an old server: no update to offer.
            update_apk: version
                .android
                .native
                .as_ref()
                .is_some_and(|native| native.apk),
            android_version: version
                .android
                .native
                .and_then(|native| native.version)
                .filter(|v| !v.trim().is_empty()),
        })
    }
}

/// Two fingerprints for the same leaf compare equal whatever case or separator
/// they were written with (`Ssl.kt` spells `AB:CD:…`, a person may paste bare
/// lowercase hex).
fn same_fingerprint(left: &str, right: &str) -> bool {
    left.chars()
        .filter(|c| *c != ':' && *c != ' ')
        .flat_map(char::to_uppercase)
        .eq(right
            .chars()
            .filter(|c| *c != ':' && *c != ' ')
            .flat_map(char::to_uppercase))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::testing::TempDir;

    /// `Prefs.buildBaseUrl`'s cases, one for one: the shape a person's address
    /// really takes on a home server.
    #[test]
    fn an_address_becomes_the_url_the_old_app_built() {
        let cases = [
            (("music.example.org", ""), Some("https://music.example.org")),
            (("192.168.1.10", "4533"), Some("https://192.168.1.10:4533")),
            (("http://192.168.1.10", ""), Some("http://192.168.1.10")),
            // The port field never lands after a path.
            (
                ("https://host/musique", "5722"),
                Some("https://host:5722/musique"),
            ),
            // A host that already carries a port keeps its own.
            (("https://host:9000", "5722"), Some("https://host:9000")),
            (("  https://host/  ", " "), Some("https://host")),
            (("", "5722"), None),
            (("   ", ""), None),
        ];
        for ((host, port), expected) in cases {
            let address = SetupAddress::new(host, port, true);
            assert_eq!(address.base_url().as_deref(), expected, "…{host} / {port}…");
        }
        assert_eq!(
            SetupAddress::new("host", "80", true).app_url().as_deref(),
            Some("https://host:80/app/")
        );
    }

    /// A fingerprint compares the same however it is written — the pin stores
    /// what `Ssl.kt` spells, and the probe must not call it a change.
    #[test]
    fn a_fingerprint_is_the_same_whatever_its_case_or_separators() {
        let upper = "AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99";
        let lower = "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";
        assert!(same_fingerprint(upper, lower));
        assert!(same_fingerprint(upper, upper));
        let other = "AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:9A";
        assert!(!same_fingerprint(upper, other), "one different byte");
    }

    /// A probe with no link opens **no socket** and says `Offline`, which is
    /// what "Continuer quand même" hangs on: it must not wait a timeout to
    /// learn there is no radio.
    #[tokio::test]
    async fn a_probe_offline_answers_at_once() {
        let dir = TempDir::new("probe-offline");
        let core = dir.core("test");
        core.set_network(crate::core::NetworkState::None);
        let error = core
            .probe_server(&SetupAddress::new("music.example.org", "", true))
            .await
            .expect_err("no link");
        assert_eq!(error, CoreError::Offline);
        assert!(core.stats().is_empty(), "not one request was recorded");
    }

    /// A plaintext address has no certificate to learn, and an address that is
    /// not an origin is refused before anything is opened.
    #[tokio::test]
    async fn a_bad_address_never_reaches_the_network() {
        let dir = TempDir::new("probe-bad");
        let core = dir.core("test");
        for host in ["", "   ", "javascript:alert(1)", "ftp://host"] {
            let error = core
                .probe_server(&SetupAddress::new(host, "", false))
                .await
                .expect_err(host);
            assert!(
                matches!(error, CoreError::Invalid { .. }),
                "{host}: {error:?}"
            );
        }
        assert!(core.stats().is_empty());
    }
}
