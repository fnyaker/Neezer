//! The server's certificate: trust on first use, and the probe that learns one
//! without sending a byte (`Ssl.kt`, `Net.kt`; SC3).
//!
//! A home server on a LAN usually answers with a self-signed certificate, which
//! no system store knows. Unticking "verify SSL" used to install a trust-all
//! manager, which is worse than no TLS at all: the padlock stays and anyone on
//! the café Wi-Fi may present anything. The phone's rule, and now this crate's,
//! is **trust on first use**:
//!
//! 1. the setup screen learns the certificate the server presents, with
//!    [`Api::probe_certificate`] — a handshake that is REFUSED once the leaf is
//!    recorded, so not one application byte is sent over a connection nobody
//!    has vouched for yet — and shows its SHA-256;
//! 2. that fingerprint is pinned for THAT origin
//!    ([`ApiBuilder::pinned_leaf_sha256`](crate::ApiBuilder::pinned_leaf_sha256));
//! 3. from then on a chain the system accepts OR exactly that leaf passes, and
//!    another leaf is [`ApiError::Certificate`] ("le certificat a changé").
//!    Another host is never relaxed.
//!
//! Three properties this file exists to keep:
//!
//! - **The platform keeps the first opinion.** The pin only ADDS one leaf to
//!   what `rustls-platform-verifier` accepts (the system store on Linux and
//!   Windows, Android's own trust manager through its JNI bridge — which the
//!   app must initialise from Kotlin, lot MB2, not this crate). A certificate
//!   signed by a real CA is never refused because it is not the pinned one.
//! - **The pin is per origin.** It applies to the configured server's host
//!   only: a redirect to another HTTPS host gets the system's checks, whatever
//!   was pinned here.
//! - **The desktop is untouched.** Nothing in the desktop app calls
//!   `pinned_leaf_sha256`, so its client is built exactly as before: no custom
//!   verifier, no extra TLS configuration, the platform's verdict and nothing
//!   else.

use std::fmt;
use std::io::Read;
use std::net::ToSocketAddrs;
use std::sync::{Arc, Mutex};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, SignatureScheme};
use rustls_platform_verifier::Verifier as PlatformVerifier;

use crate::client::CONNECT_TIMEOUT;
use crate::error::ApiError;

/// How long a certificate probe may wait on the socket: `Ssl.kt`'s 6 s, for the
/// same reason (a setup screen cannot hang on a box that does not answer).
pub const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(6);

/// What a handshake presented, learnt without sending a request (`Ssl.kt`'s
/// rule, SC3's `probe_certificate`). The setup screen shows these four things
/// and pins the first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertificateInfo {
    /// The leaf's SHA-256, colon-separated uppercase hex (`AB:CD:…`) — the
    /// exact value the pin stores and [`parse_fingerprint`] reads back.
    pub sha256: String,
    /// The subject distinguished name, `CN=…, O=…`, as far as this file reads
    /// it (`inconnu` when the certificate is not readable).
    pub subject: String,
    /// The issuer distinguished name — equal to the subject for the
    /// self-signed certificate this exists for.
    pub issuer: String,
    /// `notAfter` as ISO-8601 UTC (`2126-10-24T19:53:13Z`), so a screen can
    /// say when the certificate expires without a date library.
    pub not_after: String,
    /// Whether the platform's own trust store would accept this chain. `false`
    /// is the normal answer for a home server, and the reason the fingerprint
    /// is shown at all.
    pub system_trusted: bool,
}

/// A leaf's SHA-256 as the pin stores it: uppercase hex, `:` between bytes.
pub fn fingerprint(leaf: &CertificateDer<'_>) -> String {
    hex_fingerprint(&sha256(leaf.as_ref()))
}

/// Read a fingerprint back into the 32 bytes a pin holds. Accepts what
/// `Ssl.kt` writes (`AB:CD:…`), lowercase, and bare hex without separators;
/// anything else — a wrong length, a non-hex character — is `None`, never a
/// guess.
pub fn parse_fingerprint(text: &str) -> Option<[u8; 32]> {
    let digits: Vec<u8> = text
        .chars()
        .filter(|character| *character != ':' && *character != ' ')
        .map(|character| character.to_digit(16).map(|digit| digit as u8))
        .collect::<Option<Vec<u8>>>()?;
    if digits.len() != 64 {
        return None;
    }
    let mut digest = [0u8; 32];
    for (index, pair) in digits.chunks(2).enumerate() {
        digest[index] = (pair[0] << 4) | pair[1];
    }
    Some(digest)
}

/// `AB:CD:…` — what the person reads on the server and compares here.
pub fn hex_fingerprint(digest: &[u8]) -> String {
    digest
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// The fingerprint's hash. `aws-lc-rs` is the provider `rustls` runs with here
/// (see [`provider`]), and a SHA-256 is the same 32 bytes whichever
/// implementation computes it.
fn sha256(bytes: &[u8]) -> [u8; 32] {
    let hashed = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, bytes);
    let mut digest = [0u8; 32];
    digest.copy_from_slice(hashed.as_ref());
    digest
}

/// The crypto provider rustls is configured with. `reqwest` installs the
/// `aws-lc-rs` one (its `rustls` feature); a build where nobody did gets the
/// same one explicitly, so the pin's SHA-256 and the handshake's signatures can
/// never disagree about which provider is running.
fn provider() -> Arc<CryptoProvider> {
    CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
}

/// The TLS handshake's verdict, recorded for the request that paid for it.
#[derive(Debug, Default, Clone, Copy)]
struct Seen {
    /// Bumped by every handshake the verifier examined: a request compares the
    /// value it read before sending, so a certificate failure is never
    /// attributed to a request whose own handshake presented nothing (a plain
    /// connection failure, or a sibling request's certificate).
    seq: u64,
    leaf: Option<[u8; 32]>,
    /// Whether the PLATFORM accepted the chain (the pin is not consulted here:
    /// this is the fact the error and the probe both report).
    trusted: bool,
}

/// One origin's pin, and what the last handshake presented for it.
///
/// Shared by every clone of an [`crate::Api`] — the pin and the counters both
/// belong to the client, not to the screen that asked for it.
#[derive(Debug)]
pub(crate) struct PinState {
    pin: Option<[u8; 32]>,
    /// The configured origin's host: the ONLY host the pin applies to.
    host: String,
    provider: Arc<CryptoProvider>,
    seen: Mutex<Seen>,
}

impl PinState {
    pub(crate) fn new(pin: Option<[u8; 32]>, host: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            pin,
            host: host.into(),
            provider: provider(),
            seen: Mutex::new(Seen::default()),
        })
    }

    /// The pinned leaf, when somebody pinned one.
    pub(crate) fn pin(&self) -> Option<[u8; 32]> {
        self.pin
    }

    fn seen(&self) -> Seen {
        *self.seen.lock().unwrap_or_else(|err| err.into_inner())
    }

    /// `true` when `server_name` is the configured server's own name: the one
    /// place a pin may relieve, or blame, a certificate.
    fn is_our_host(&self, server_name: &ServerName<'_>) -> bool {
        match ServerName::try_from(self.host.clone()) {
            Ok(ours) => ours == *server_name,
            // A host this crate cannot even name is not the configured one.
            Err(_) => false,
        }
    }

    fn record(&self, leaf: [u8; 32], trusted: bool) {
        let mut seen = self.seen.lock().unwrap_or_else(|err| err.into_inner());
        seen.seq = seen.seq.wrapping_add(1);
        seen.leaf = Some(leaf);
        seen.trusted = trusted;
    }

    /// The sequence number to read before a request is sent.
    pub(crate) fn mark(&self) -> u64 {
        self.seen().seq
    }

    /// The refusal a failed handshake means, or `None` when the failure was
    /// not a certificate this client refused (a dead link, a timeout, a
    /// handshake this origin never got to).
    ///
    /// `after` is the mark taken before the request: a stale record from an
    /// earlier request must not blame the certificate for a network failure.
    pub(crate) fn certificate_error(&self, after: u64) -> Option<ApiError> {
        let seen = self.seen();
        if seen.seq == after {
            return None;
        }
        let leaf = seen.leaf?;
        if seen.trusted {
            // The platform accepted it: whatever failed, the certificate is not
            // it (and the pin only adds leaves, it never removes one).
            return None;
        }
        match self.pin {
            // The pinned leaf WAS presented, so the handshake failed on
            // something else: not a certificate refusal.
            Some(pin) if pin == leaf => None,
            Some(_) => Some(ApiError::Certificate {
                sha256: hex_fingerprint(&leaf),
                changed: true,
            }),
            None => Some(ApiError::Certificate {
                sha256: hex_fingerprint(&leaf),
                changed: false,
            }),
        }
    }
}

/// The verifier a pinned client installs: the platform's verdict OR an exact
/// leaf match for the configured host, and nothing else.
struct PinnedVerifier {
    platform: Arc<dyn ServerCertVerifier>,
    state: Arc<PinState>,
    algorithms: WebPkiSupportedAlgorithms,
}

impl fmt::Debug for PinnedVerifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never a certificate body: the host, and whether a pin is held.
        f.debug_struct("PinnedVerifier")
            .field("host", &self.state.host)
            .field("pinned", &self.state.pin.is_some())
            .finish()
    }
}

impl ServerCertVerifier for PinnedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let ours = self.state.is_our_host(server_name);
        let leaf = sha256(end_entity.as_ref());
        // The platform's verdict FIRST: the pin only adds one leaf, it never
        // takes away a chain a real CA signed.
        let verdict = self.platform.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        );
        if ours {
            self.state.record(leaf, verdict.is_ok());
        }
        match verdict {
            Ok(verified) => Ok(verified),
            Err(error) => {
                if ours && self.state.pin == Some(leaf) {
                    Ok(ServerCertVerified::assertion())
                } else {
                    Err(error)
                }
            }
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// A `rustls::ClientConfig` that accepts the platform's chains plus one pinned
/// leaf, and refuses everything else.
pub(crate) fn pinned_client_config(state: &Arc<PinState>) -> Result<ClientConfig, ApiError> {
    let platform = PlatformVerifier::new(Arc::clone(&state.provider)).map_err(system_verifier)?;
    let verifier = PinnedVerifier {
        platform: Arc::new(platform),
        state: Arc::clone(state),
        algorithms: state.provider.signature_verification_algorithms,
    };
    Ok(client_config(Arc::new(verifier), &state.provider))
}

/// ALPN says what `reqwest` would have said on its own: a preconfigured
/// `ClientConfig` is used as it is, so this file is the only place that decides
/// it. HTTP/2 first, HTTP/1.1 after — the phone's server is Flask (1.1) and a
/// redirect target may well be on 2.
fn client_config(
    verifier: Arc<dyn ServerCertVerifier>,
    provider: &Arc<CryptoProvider>,
) -> ClientConfig {
    let mut config = ClientConfig::builder_with_provider(Arc::clone(provider))
        .with_safe_default_protocol_versions()
        .expect("the provider supports the default protocol versions")
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    config
}

fn system_verifier(error: rustls::Error) -> ApiError {
    ApiError::Http {
        status: 0,
        message: format!("vérificateur de certificats du système indisponible : {error}"),
        code: Some("tls".to_owned()),
    }
}

impl crate::Api {
    /// The certificate `url` presents, learnt without sending a request
    /// (`Ssl.kt`'s `presentedFingerprint`, SC3).
    ///
    /// The handshake is REFUSED as soon as the leaf is read — always, even for
    /// a chain the system trusts — so not one application byte ever leaves this
    /// client for a server nobody has vouched for yet. What comes back is what
    /// the setup screen shows: the fingerprint to pin, the names, the expiry,
    /// and whether the system's own store would have accepted the chain (in
    /// which case there is nothing to pin).
    ///
    /// A `https` URL only: a plaintext server has no certificate to learn, and
    /// this crate never opens a cleartext socket off the configured origin
    /// ([`ApiError::Invalid`]).
    pub async fn probe_certificate(url: impl AsRef<str>) -> Result<CertificateInfo, ApiError> {
        let url = url.as_ref().to_owned();
        // A blocking socket on the pool: the probe is one call from a setup
        // screen, bounded by CONNECT_TIMEOUT and PROBE_TIMEOUT, and rustls's
        // handshake here is driven by `std::io` (`StreamOwned`). Nothing borrows
        // the runtime's threads, and nothing is spawned that outlives the call.
        tokio::task::spawn_blocking(move || probe_blocking(&url))
            .await
            .map_err(|error| ApiError::Http {
                status: 0,
                message: format!("sonde de certificat interrompue : {error}"),
                code: Some("probe".to_owned()),
            })?
    }
}

/// What a probe's handshake presented: the certificate itself (its names and
/// its expiry are read off it) and the platform's verdict on the chain.
struct Recorded {
    certificate: Vec<u8>,
    sha256: [u8; 32],
    system_trusted: bool,
}

/// The verifier of a probe: it records the leaf and the platform's verdict, and
/// refuses the handshake whatever they say.
struct RecordingVerifier {
    platform: Arc<dyn ServerCertVerifier>,
    algorithms: WebPkiSupportedAlgorithms,
    recorded: Mutex<Option<Recorded>>,
}

impl fmt::Debug for RecordingVerifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RecordingVerifier")
    }
}

impl ServerCertVerifier for RecordingVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let trusted = self
            .platform
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
            .is_ok();
        *self.recorded.lock().unwrap_or_else(|err| err.into_inner()) = Some(Recorded {
            certificate: end_entity.as_ref().to_vec(),
            sha256: sha256(end_entity.as_ref()),
            system_trusted: trusted,
        });
        // ALWAYS refused: recording is all this handshake is for, and an
        // unvouched-for channel must not carry a request.
        Err(rustls::Error::InvalidCertificate(
            rustls::CertificateError::Other(rustls::OtherError(Arc::new(ProbeOnly))),
        ))
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// Why a probe's handshake ends: the leaf was recorded, nothing more is wanted.
#[derive(Debug)]
struct ProbeOnly;

impl fmt::Display for ProbeOnly {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("certificate recorded, handshake refused on purpose")
    }
}

impl std::error::Error for ProbeOnly {}

fn probe_blocking(url: &str) -> Result<CertificateInfo, ApiError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|error| ApiError::Invalid(format!("adresse refusée ({url}) : {error}")))?;
    if parsed.scheme() != "https" {
        return Err(ApiError::Invalid(format!(
            "adresse refusée ({url}) : une sonde de certificat parle HTTPS, jamais {}",
            parsed.scheme()
        )));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| ApiError::Invalid(format!("adresse refusée ({url}) : aucun hôte")))?;
    let port = parsed.port_or_known_default().unwrap_or(443);
    let server_name = ServerName::try_from(host.to_owned())
        .map_err(|_| ApiError::Invalid(format!("adresse refusée ({url}) : nom d’hôte invalide")))?;

    let addresses: Vec<std::net::SocketAddr> = (host, port)
        .to_socket_addrs()
        .map_err(|_| ApiError::Offline)?
        .collect();
    if addresses.is_empty() {
        return Err(ApiError::Offline);
    }

    let provider = provider();
    let recording = Arc::new(RecordingVerifier {
        platform: PlatformVerifier::new(Arc::clone(&provider))
            .map(Arc::new)
            .map_err(system_verifier)?,
        algorithms: provider.signature_verification_algorithms,
        recorded: Mutex::new(None),
    });
    let verifier: Arc<dyn ServerCertVerifier> = recording.clone();
    let config = Arc::new(client_config(verifier, &provider));

    for address in addresses {
        let Ok(socket) = std::net::TcpStream::connect_timeout(&address, CONNECT_TIMEOUT) else {
            continue;
        };
        // Both deadlines: a peer that accepts the socket and then says nothing
        // must not park a setup screen for ever.
        let _ = socket.set_read_timeout(Some(PROBE_TIMEOUT));
        let _ = socket.set_write_timeout(Some(PROBE_TIMEOUT));
        let Ok(connection) = ClientConnection::new(Arc::clone(&config), server_name.clone()) else {
            continue;
        };
        let mut stream = rustls::StreamOwned::new(connection, socket);
        // ONE read drives the whole attempt: `Stream` writes the pending
        // ClientHello, reads the server's flight and hands the certificate to
        // the verifier, which refuses. Looping covers a server that answers in
        // several TCP segments; nothing is ever written after the handshake.
        let mut scratch = [0u8; 512];
        for _ in 0..4 {
            if stream.read(&mut scratch).is_err() {
                break;
            }
        }
        let recorded = recording
            .recorded
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .take();
        if let Some(recorded) = recorded {
            return Ok(describe(
                &recorded.certificate,
                &recorded.sha256,
                recorded.system_trusted,
            ));
        }
    }
    // Every address refused to present a certificate: a dead link, not a
    // verdict about the server (`Ssl.kt` answers null for the same reason).
    Err(ApiError::Offline)
}
/// The four things a setup screen shows, from the leaf DER and its digest.
fn describe(certificate: &[u8], sha256: &[u8; 32], system_trusted: bool) -> CertificateInfo {
    let names = names(certificate).unwrap_or_default();
    CertificateInfo {
        sha256: hex_fingerprint(sha256),
        subject: names.subject,
        issuer: names.issuer,
        not_after: names.not_after,
        system_trusted,
    }
}

/// The readable part of a certificate, all of it optional: a certificate this
/// file cannot walk still yields its fingerprint, which is what is pinned.
#[derive(Debug, Default, PartialEq, Eq)]
struct Names {
    subject: String,
    issuer: String,
    not_after: String,
}

const UNKNOWN: &str = "inconnu";

fn names(certificate: &[u8]) -> Option<Names> {
    let mut outer = Der::new(certificate);
    let certificate = outer.expect(0x30)?;
    let mut body = Der::new(certificate);
    let tbs = body.expect(0x30)?;
    let mut tbs = Der::new(tbs);
    // version [0] EXPLICIT is optional (v1 certificates have none).
    if tbs.peek() == Some(0xa0) {
        tbs.skip()?;
    }
    tbs.skip()?; // serialNumber
    tbs.skip()?; // signature AlgorithmIdentifier
    let issuer = tbs.expect(0x30)?;
    let validity = tbs.expect(0x30)?;
    let subject = tbs.expect(0x30)?;
    let mut validity = Der::new(validity);
    validity.skip()?; // notBefore
    let (tag, not_after) = validity.next()?;
    Some(Names {
        subject: distinguished_name(subject).unwrap_or_else(|| UNKNOWN.to_owned()),
        issuer: distinguished_name(issuer).unwrap_or_else(|| UNKNOWN.to_owned()),
        not_after: time_to_iso(tag, not_after).unwrap_or_default(),
    })
}

/// `CN=…, O=…` from a `Name ::= SEQUENCE OF SET OF SEQUENCE { OID, value }`.
fn distinguished_name(name: &[u8]) -> Option<String> {
    let mut parts = Vec::new();
    let mut sets = Der::new(name);
    while !sets.finished() {
        let set = sets.expect(0x31)?;
        let mut set = Der::new(set);
        let attribute = set.expect(0x30)?;
        let mut attribute = Der::new(attribute);
        let oid = attribute.expect(0x06)?;
        let (tag, value) = attribute.next()?;
        let value = string_value(tag, value)?;
        let label = match oid {
            [0x55, 0x04, 0x03] => "CN",
            [0x55, 0x04, 0x06] => "C",
            [0x55, 0x04, 0x07] => "L",
            [0x55, 0x04, 0x08] => "ST",
            [0x55, 0x04, 0x0a] => "O",
            [0x55, 0x04, 0x0b] => "OU",
            // 1.2.840.113549.1.9.1, emailAddress
            [0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x01] => "E",
            _ => continue,
        };
        parts.push(format!("{label}={value}"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

/// The string types a distinguished name uses, decoded; anything else is
/// skipped rather than shown as bytes.
fn string_value(tag: u8, value: &[u8]) -> Option<String> {
    match tag {
        // UTF8String, PrintableString, IA5String, TeletexString
        0x0c | 0x13 | 0x16 | 0x14 => Some(String::from_utf8_lossy(value).into_owned()),
        // BMPString (UTF-16 big endian, the `UCS-2` X.509 allows)
        0x1e => {
            let units: Vec<u16> = value
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_be_bytes(*pair))
                .collect();
            Some(String::from_utf16_lossy(&units))
        }
        _ => None,
    }
}

/// `notAfter` as ISO-8601 UTC. X.509 allows two encodings and both are here:
/// `UTCTime` (`YYMMDDHHMMSSZ`, two-digit years per RFC 5280) and
/// `GeneralizedTime` (`YYYYMMDDHHMMSSZ`).
fn time_to_iso(tag: u8, value: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(value).ok()?;
    let digits: String = text.chars().filter(char::is_ascii_digit).collect();
    let (year, rest) = match tag {
        0x17 => {
            if digits.len() < 12 {
                return None;
            }
            let short: u32 = digits.get(0..2)?.parse().ok()?;
            // RFC 5280 4.1.2.5.1: 00–49 is 20xx, 50–99 is 19xx.
            let year = if short < 50 {
                2000 + short
            } else {
                1900 + short
            };
            (year, &digits[2..])
        }
        0x18 => {
            if digits.len() < 14 {
                return None;
            }
            (digits.get(0..4)?.parse().ok()?, &digits[4..])
        }
        _ => return None,
    };
    let month: u32 = rest.get(0..2)?.parse().ok()?;
    let day: u32 = rest.get(2..4)?.parse().ok()?;
    let hour: u32 = rest.get(4..6)?.parse().ok()?;
    let minute: u32 = rest.get(6..8)?.parse().ok()?;
    let second: u32 = rest.get(8..10)?.parse().ok()?;
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
    ))
}

/// Just enough DER to walk a certificate: tag, length, value. Rejects the
/// indefinite length (DER forbids it) and every truncation — a hostile
/// certificate is a `None`, never a panic and never a read past the buffer.
struct Der<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Der<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    fn finished(&self) -> bool {
        self.at >= self.bytes.len()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    /// The next tag-length-value: its tag, and its value.
    fn next(&mut self) -> Option<(u8, &'a [u8])> {
        let tag = *self.bytes.get(self.at)?;
        let first = *self.bytes.get(self.at + 1)?;
        let (length, header) = if first < 0x80 {
            (usize::from(first), 2)
        } else {
            let count = usize::from(first & 0x7f);
            // 0x80 is the indefinite form (BER only), and DER never needs more
            // than four length bytes for the sizes this reads.
            if count == 0 || count > 4 {
                return None;
            }
            let mut length = 0usize;
            for offset in 0..count {
                length = (length << 8) | usize::from(*self.bytes.get(self.at + 2 + offset)?);
            }
            (length, 2 + count)
        };
        let start = self.at + header;
        let end = start.checked_add(length)?;
        let value = self.bytes.get(start..end)?;
        self.at = end;
        Some((tag, value))
    }

    fn expect(&mut self, tag: u8) -> Option<&'a [u8]> {
        match self.next()? {
            (found, value) if found == tag => Some(value),
            _ => None,
        }
    }

    fn skip(&mut self) -> Option<()> {
        self.next().map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two throwaway certificates `tests/tls.rs` serves (self-signed,
    /// ECDSA P-256, valid a hundred years — generated once with OpenSSL and
    /// committed, because `rcgen` would have added four packages to the
    /// desktop's lockfile for a test fixture).
    const CERT_A: &[u8] = include_bytes!("../tests/fixtures/tls/cert_a.der");

    #[test]
    fn a_fingerprint_is_uppercase_colon_separated_hex_and_parses_back() {
        let digest = sha256(CERT_A);
        let written = hex_fingerprint(&digest);
        assert_eq!(written.len(), 95, "32 bytes, 32 ':' separators");
        assert!(
            written.split(':').all(|byte| byte.len() == 2
                && byte
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())),
            "{written}"
        );
        // The form `Ssl.kt` writes is the form a pin reads back, and so is
        // lowercase and bare hex.
        assert_eq!(parse_fingerprint(&written), Some(digest));
        assert_eq!(parse_fingerprint(&written.to_lowercase()), Some(digest));
        assert_eq!(
            parse_fingerprint(&written.replace(':', "")),
            Some(digest),
            "the server's own `sha256sum` prints it without separators"
        );
        // Anything that is not 32 bytes of hex is refused, never guessed.
        assert_eq!(parse_fingerprint(""), None);
        assert_eq!(parse_fingerprint("AB:CD"), None);
        assert_eq!(parse_fingerprint(&format!("{written}:00")), None);
        assert_eq!(parse_fingerprint("ZZ:CD"), None);
    }

    /// The names and the expiry come off the DER, without a date crate and
    /// without trusting the certificate's own text layout.
    #[test]
    fn the_names_and_the_expiry_are_read_from_the_der() {
        let read = names(CERT_A).expect("the fixture is a certificate");
        assert_eq!(read.subject, "CN=127.0.0.1, O=Neezer tests, OU=SC3");
        assert_eq!(
            read.issuer, read.subject,
            "a self-signed certificate is its own issuer"
        );
        assert_eq!(read.not_after, "2126-10-24T19:53:13Z");

        // What a certificate cannot be read as is missing, not a panic.
        assert_eq!(names(&[]), None);
        assert_eq!(names(b"not a certificate at all"), None);
        assert_eq!(names(&CERT_A[..CERT_A.len() / 2]), None);
    }

    #[test]
    fn both_x509_time_encodings_become_iso_8601() {
        assert_eq!(
            time_to_iso(0x17, b"261024195313Z").as_deref(),
            Some("2026-10-24T19:53:13Z"),
            "UTCTime: 26 is 2026 (RFC 5280's 50-year pivot)"
        );
        assert_eq!(
            time_to_iso(0x17, b"491231235959Z").as_deref(),
            Some("2049-12-31T23:59:59Z")
        );
        assert_eq!(
            time_to_iso(0x17, b"500101000000Z").as_deref(),
            Some("1950-01-01T00:00:00Z")
        );
        assert_eq!(
            time_to_iso(0x18, b"21261024195313Z").as_deref(),
            Some("2126-10-24T19:53:13Z")
        );
        assert_eq!(time_to_iso(0x18, b"2126"), None, "a truncated time");
        assert_eq!(time_to_iso(0x05, b"21261024195313Z"), None, "not a time");
    }

    /// A certificate whose names this file cannot walk still yields its
    /// fingerprint and an honest "inconnu" — never empty strings pretending to
    /// be a name.
    #[test]
    fn an_unreadable_certificate_still_has_its_fingerprint() {
        let info = describe(CERT_A, &sha256(CERT_A), false);
        assert_eq!(info.subject, "CN=127.0.0.1, O=Neezer tests, OU=SC3");
        assert!(!info.system_trusted);
        let broken = Names::default();
        assert_eq!(
            (broken.subject.as_str(), broken.not_after.as_str()),
            ("", "")
        );
    }

    /// The pin's arithmetic, without a socket: no pin and a refused chain is
    /// `changed: false`, a pin and another leaf is `changed: true`, and a mark
    /// taken before a request that saw no handshake blames nothing.
    #[test]
    fn the_error_says_whether_the_certificate_changed() {
        let leaf = sha256(CERT_A);
        let other = [0x11u8; 32];

        let unpinned = PinState::new(None, "127.0.0.1");
        let mark = unpinned.mark();
        unpinned.record(leaf, false);
        assert_eq!(
            unpinned.certificate_error(mark),
            Some(ApiError::Certificate {
                sha256: hex_fingerprint(&leaf),
                changed: false,
            })
        );

        let pinned = PinState::new(Some(other), "127.0.0.1");
        let mark = pinned.mark();
        pinned.record(leaf, false);
        assert_eq!(
            pinned.certificate_error(mark),
            Some(ApiError::Certificate {
                sha256: hex_fingerprint(&leaf),
                changed: true,
            })
        );

        // The pinned leaf itself: the handshake failed on something else.
        let matches = PinState::new(Some(leaf), "127.0.0.1");
        let mark = matches.mark();
        matches.record(leaf, false);
        assert_eq!(matches.certificate_error(mark), None);

        // The platform accepted the chain: the certificate is not the failure.
        let trusted = PinState::new(None, "127.0.0.1");
        let mark = trusted.mark();
        trusted.record(leaf, true);
        assert_eq!(trusted.certificate_error(mark), None);

        // No handshake since the mark: a dead link, not a certificate.
        let quiet = PinState::new(None, "127.0.0.1");
        let mark = quiet.mark();
        quiet.record(leaf, false);
        assert_eq!(quiet.certificate_error(mark + 1), None);
    }

    /// The pin is the configured host's, and only its own: another name gets
    /// the platform's verdict and nothing else.
    #[test]
    fn another_host_is_never_relaxed() {
        let state = PinState::new(Some([1u8; 32]), "music.example.org");
        assert!(state.is_our_host(&ServerName::try_from("music.example.org").unwrap()));
        assert!(!state.is_our_host(&ServerName::try_from("cdn.example.org").unwrap()));
        assert!(!state.is_our_host(&ServerName::try_from("127.0.0.1").unwrap()));
        assert_eq!(state.pin(), Some([1u8; 32]));

        // A numeric host compares as the address rustls will actually send.
        let numeric = PinState::new(None, "127.0.0.1");
        assert!(numeric.is_our_host(&ServerName::try_from("127.0.0.1").unwrap()));
        assert_eq!(numeric.pin(), None);
    }

    /// Pin state's `Debug` names the host and never a certificate body.
    #[test]
    fn debug_names_the_host_and_never_a_certificate() {
        let state = PinState::new(Some([2u8; 32]), "music.example.org");
        state.record([3u8; 32], false);
        let printed = format!("{state:?}");
        assert!(printed.contains("music.example.org"), "{printed}");
        assert!(!printed.contains("03:03"), "{printed}");
    }

    #[test]
    fn a_probe_of_a_plaintext_url_is_refused_before_any_socket() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a test runtime");
        for url in [
            "http://127.0.0.1:1/x",
            "http://music.example.org",
            "file:///etc/passwd",
            "not a url",
        ] {
            let error = runtime
                .block_on(crate::Api::probe_certificate(url))
                .expect_err(url);
            assert!(
                matches!(error, ApiError::Invalid(_)),
                "{url} must be refused as invalid: {error:?}"
            );
        }
        drop(runtime);
    }
}
