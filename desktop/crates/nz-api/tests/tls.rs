//! SC3's acceptance, against a REAL TLS server on the loopback: a self-signed
//! certificate, the trust-on-first-use pin that makes it usable, the probe that
//! learns it without sending a byte, the redirect a session must not follow
//! off-origin, and the counters that say what each screen cost.
//!
//! Why a real socket and a real handshake: every one of these rules is about
//! what a peer can make this client do. A mocked client would prove that the
//! code calls itself; the server below counts what it RECEIVED, which is the
//! only evidence that "nothing was sent" and "the cookie stayed home".
//!
//! The two certificates are throwaway fixtures (`fixtures/tls/`, ECDSA P-256,
//! self-signed, `CN=127.0.0.1`, valid a hundred years), generated once with
//! OpenSSL: `rcgen` would have added four packages to the desktop's lockfile for
//! a test fixture, and `nz-api` already links `rustls`. Both are committed with
//! their keys, which is fine for a key that signs nothing.
//!
//! Nothing here waits on the clock: the server answers and closes, the client is
//! awaited, and every assertion is on a counted number or a returned value.

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::thread::JoinHandle;

use nz_api::{Api, ApiError, Lane, Method, parse_fingerprint};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};

const CERT_A: &[u8] = include_bytes!("fixtures/tls/cert_a.der");
const KEY_A: &[u8] = include_bytes!("fixtures/tls/key_a.der");
const CERT_B: &[u8] = include_bytes!("fixtures/tls/cert_b.der");
const KEY_B: &[u8] = include_bytes!("fixtures/tls/key_b.der");

/// One answer the test server gives, in the order the client asks for them.
struct Hop {
    status: u16,
    location: Option<String>,
    body: Vec<u8>,
}

impl Hop {
    /// A `200` whose body is `body`, used as JSON by the client.
    fn ok(body: &[u8]) -> Self {
        Self {
            status: 200,
            location: None,
            body: body.to_vec(),
        }
    }

    fn redirect(to: &str) -> Self {
        Self {
            status: 302,
            location: Some(to.to_owned()),
            body: Vec::new(),
        }
    }
}

/// What the server saw on one connection: the request head it managed to read
/// (empty when the client refused the certificate and the handshake never
/// finished), and therefore how many APPLICATION bytes it really received.
struct Seen {
    head: String,
}

impl Seen {
    /// The value of one request header, if the client sent it.
    fn header(&self, name: &str) -> Option<&str> {
        let prefix = format!("{name}:");
        self.head
            .lines()
            .find_map(|line| line.strip_prefix(&prefix))
            .map(str::trim)
    }

    /// Application bytes this server READ off the connection: the request head,
    /// plus a body only when it arrived in the same TLS record. `0` therefore
    /// means the client wrote nothing at all — which is what a refused handshake
    /// and the certificate probe both must show.
    fn application_bytes(&self) -> usize {
        self.head.len()
    }
}

fn server_config(cert: &[u8], key: &[u8]) -> Arc<ServerConfig> {
    let certificates = vec![CertificateDer::from(cert.to_vec())];
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.to_vec()));
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certificates, key)
        .expect("the fixture is a certificate and its key");
    Arc::new(config)
}

/// A loopback TLS server that answers `script` in order, one connection per
/// answer, and sends back every request it read. Nothing leaves the loopback and
/// no port is assumed.
///
/// The thread stops after the script: a hop the client never asked for is simply
/// never served, so a test that expected no traffic joins without waiting.
fn tls_server(
    cert: &'static [u8],
    key: &'static [u8],
    script: Vec<Hop>,
) -> (String, Receiver<Seen>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("address").port();
    let config = server_config(cert, key);
    let (sender, seen) = channel();
    let handle = std::thread::spawn(move || {
        for hop in script {
            let Ok((socket, _)) = listener.accept() else {
                return;
            };
            // A guard, not a wait: a client that hangs must not hang CI.
            let _ = socket.set_read_timeout(Some(std::time::Duration::from_secs(20)));
            let connection = match ServerConnection::new(Arc::clone(&config)) {
                Ok(connection) => connection,
                Err(_) => continue,
            };
            let mut stream = StreamOwned::new(connection, socket);
            let mut head = Vec::new();
            let mut buffer = [0u8; 2048];
            loop {
                match stream.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        head.extend_from_slice(&buffer[..read]);
                        // The request head ends at the first CRLFCRLF; a body
                        // that arrived with it is already read and is NOT waited
                        // for (a POST's body would otherwise park this server
                        // until the client's own deadline).
                        if let Some(end) = head.windows(4).position(|window| window == b"\r\n\r\n")
                        {
                            head.truncate(end + 4);
                            break;
                        }
                    }
                    // The client refused our certificate (or went away): the
                    // handshake never completed, and no request was sent.
                    Err(_) => break,
                }
            }
            let text = String::from_utf8_lossy(&head).into_owned().to_lowercase();
            let _ = sender.send(Seen { head: text });
            if head.is_empty() {
                // Nothing was asked for: a refused handshake gets no answer.
                continue;
            }
            let mut answer = format!(
                "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
                hop.status,
                hop.body.len()
            );
            if let Some(location) = hop.location {
                answer.push_str(&format!("Location: {location}\r\n"));
            }
            answer.push_str("\r\n");
            let _ = stream.write_all(answer.as_bytes());
            let _ = stream.write_all(&hop.body);
            let _ = stream.flush();
        }
    });
    (format!("https://127.0.0.1:{port}"), seen, handle)
}

/// The fingerprint of one fixture, as a pin holds it — read off the DER by the
/// crate's own helper, so a test cannot pass with a pin the app would refuse.
fn pin_of(cert: &'static [u8]) -> [u8; 32] {
    let leaf = CertificateDer::from(cert.to_vec());
    parse_fingerprint(&nz_api::fingerprint(&leaf)).expect("a fingerprint")
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nz-api-tls-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// **The phone's rule, first half.** A certificate no system store knows is
/// REFUSED, and refused as a CERTIFICATE (`changed: false`) rather than as a
/// dead link: the setup screen shows the fingerprint and offers to pin it, which
/// it could not do with an "offline". One handshake, not the SPA's three tries —
/// a certificate does not change between two attempts.
#[tokio::test]
async fn an_unpinned_self_signed_server_is_refused_with_its_fingerprint() {
    let (base, seen, server) = tls_server(CERT_A, KEY_A, vec![Hop::ok(b"\"unused\"")]);
    let api = Api::builder(&base)
        .pinned_leaf_sha256(None)
        .build()
        .expect("a client");
    let error = api
        .get::<String>(Lane::User, "/me")
        .await
        .expect_err("a self-signed certificate is not in the system store");
    match error {
        ApiError::Certificate { sha256, changed } => {
            assert_eq!(
                sha256,
                nz_api::hex_fingerprint(&pin_of(CERT_A)),
                "the error carries the PRESENTED leaf, the value a person pins"
            );
            assert!(!changed, "nothing was pinned yet, so nothing changed");
            assert_eq!(
                ApiError::Certificate { sha256, changed }.cause_fr(),
                "Le certificat du serveur n'est pas reconnu"
            );
        }
        other => panic!("expected a Certificate refusal, got {other:?}"),
    }
    let refusal = seen.recv().expect("the connection");
    assert_eq!(
        refusal.application_bytes(),
        0,
        "the handshake was refused before a request could be written"
    );
    assert_eq!(
        api.stats()[0].requests,
        1,
        "a certificate refusal is not retried"
    );
    assert!(
        api.is_online(),
        "the server answered with a certificate: the link is alive"
    );
    server.join().expect("the server thread");
}

/// **The phone's rule, second half.** The same certificate, pinned, is accepted,
/// and the answer arrives: that is the whole point of trust on first use — the
/// home server works, and nobody else's certificate does.
#[tokio::test]
async fn a_pinned_leaf_is_accepted_and_its_answer_arrives() {
    let body = b"\"maison\"".to_vec();
    let (base, seen, server) = tls_server(CERT_A, KEY_A, vec![Hop::ok(&body)]);
    let api = Api::builder(&base)
        .pinned_leaf_sha256(Some(pin_of(CERT_A)))
        .build()
        .expect("a client");
    assert_eq!(
        api.pinned_fingerprint().as_deref(),
        Some(nz_api::hex_fingerprint(&pin_of(CERT_A)).as_str()),
        "the client says what it pinned"
    );
    let me = api
        .get::<String>(Lane::User, "/me")
        .await
        .expect("the pinned leaf is accepted")
        .value;
    assert_eq!(me, "maison");

    let request = seen.recv().expect("the request");
    assert!(
        request.head.starts_with("get /api/me "),
        "the API path is asked as it is: {}",
        request.head
    );
    assert_eq!(
        request.header("cookie"),
        None,
        "no session was held, so none is sent"
    );
    let stats = &api.stats()[0];
    assert_eq!(stats.requests, 1);
    assert_eq!(
        stats.bytes_in,
        body.len() as u64,
        "the counters are exactly the bytes the server sent"
    );
    server.join().expect("the server thread");
}

/// A pin is a pin: the same origin serving ANOTHER certificate is a change, and
/// the person is told so instead of being signed in over a channel somebody else
/// may be on.
#[tokio::test]
async fn another_certificate_on_the_same_origin_is_a_change() {
    let (base, seen, server) = tls_server(CERT_B, KEY_B, vec![Hop::ok(b"\"unused\"")]);
    let api = Api::builder(&base)
        .pinned_leaf_sha256(Some(pin_of(CERT_A)))
        .build()
        .expect("a client");
    let error = api
        .get::<String>(Lane::User, "/me")
        .await
        .expect_err("certificate B is not the pinned leaf A");
    match error {
        ApiError::Certificate { sha256, changed } => {
            assert_eq!(
                sha256,
                nz_api::hex_fingerprint(&pin_of(CERT_B)),
                "the error names the certificate that answered, not the lost pin"
            );
            assert!(changed, "a pin was held and a different leaf came");
            assert_eq!(
                ApiError::Certificate { sha256, changed }.cause_fr(),
                "Le certificat du serveur a changé",
                "the sentence the person reads"
            );
        }
        other => panic!("expected a changed certificate, got {other:?}"),
    }
    assert_eq!(seen.recv().expect("the connection").application_bytes(), 0);
    server.join().expect("the server thread");
}

/// **The probe.** It reads the certificate — fingerprint, names, expiry, and
/// whether the system would have accepted the chain — and the server receives
/// ZERO application bytes: the handshake is refused once the leaf is in hand,
/// exactly as `Ssl.kt` does it.
#[tokio::test]
async fn a_probe_reads_the_fingerprint_and_sends_no_request() {
    let (base, seen, server) = tls_server(CERT_A, KEY_A, vec![Hop::ok(b"\"unused\"")]);
    let info = Api::probe_certificate(&base).await.expect("a certificate");
    assert_eq!(
        info.sha256,
        nz_api::hex_fingerprint(&pin_of(CERT_A)),
        "the fingerprint is the one a pin stores"
    );
    assert!(
        info.subject.contains("CN=127.0.0.1"),
        "the setup screen names the certificate: {}",
        info.subject
    );
    assert_eq!(
        info.issuer, info.subject,
        "a self-signed certificate is its own issuer"
    );
    assert_eq!(
        info.not_after, "2126-10-24T19:53:13Z",
        "the expiry, as the certificate says it"
    );
    assert!(
        !info.system_trusted,
        "no system store knows a self-signed certificate"
    );

    let probe = seen.recv().expect("the connection");
    assert_eq!(
        probe.application_bytes(),
        0,
        "the probe wrote NOT an application byte: {:?}",
        probe.head
    );
    assert!(probe.head.is_empty(), "not even an HTTP request line");
    server.join().expect("the server thread");
}

/// The probe's fingerprint is what a pin can be built from: the value the setup
/// screen shows is the value a client accepts.
#[tokio::test]
async fn a_probe_and_a_pin_agree_on_the_same_certificate() {
    let (base, _seen, server) = tls_server(
        CERT_A,
        KEY_A,
        vec![Hop::ok(b"\"unused\""), Hop::ok(b"\"ok\"")],
    );
    let info = Api::probe_certificate(&base).await.expect("a certificate");
    let pin = parse_fingerprint(&info.sha256).expect("the shown fingerprint parses back");
    let api = Api::builder(&base)
        .pinned_leaf_sha256(Some(pin))
        .build()
        .expect("a client");
    let answer = api
        .get::<String>(Lane::User, "/me")
        .await
        .expect("the certificate the probe showed is accepted once pinned")
        .value;
    assert_eq!(answer, "ok");
    server.join().expect("the server thread");
}

/// **The desktop is untouched.** A client that never asked for a pin keeps the
/// platform's verdict alone — no custom verifier — and a self-signed server is
/// then an ordinary connection failure, retried three times like any other.
/// That number (3 handshakes, not 1) is the proof the two paths really differ.
#[tokio::test]
async fn the_default_client_keeps_the_platform_verdict_and_its_retries() {
    let (base, seen, server) = tls_server(
        CERT_A,
        KEY_A,
        vec![
            Hop::ok(b"\"unused\""),
            Hop::ok(b"\"unused\""),
            Hop::ok(b"\"unused\""),
        ],
    );
    let api = Api::builder(&base).build().expect("a client");
    assert_eq!(api.pinned_fingerprint(), None, "nothing was pinned");
    let error = api
        .get::<String>(Lane::User, "/me")
        .await
        .expect_err("the platform refuses a self-signed certificate");
    assert!(
        matches!(error, ApiError::Offline),
        "without a pin this is a connection failure, not a certificate verdict: {error:?}"
    );
    for _ in 0..3 {
        assert_eq!(seen.recv().expect("a handshake").application_bytes(), 0);
    }
    assert_eq!(
        api.stats()[0].requests,
        3,
        "the SPA's three attempts on a connection failure are still what the desktop gets"
    );
    server.join().expect("the server thread");
}

/// **The session never leaves the server.** A redirect to another origin (same
/// host, another port — a genuine origin change) is followed, and the cookie
/// goes with the FIRST hop only. Both servers present the pinned leaf, so what
/// differs between them is the origin and nothing else.
#[tokio::test]
async fn a_redirect_to_another_origin_carries_no_cookie() {
    let body = b"\"elsewhere\"".to_vec();
    // The target first: the first server's `Location` needs its URL, so its port
    // must be known before its own script is answered. One hop each: the script
    // is the number of connections the client is expected to open, and the
    // server thread stops when it is spent.
    let (target, target_seen, target_server) = tls_server(CERT_A, KEY_A, vec![Hop::ok(&body)]);
    let elsewhere = format!("{target}/api/elsewhere");
    let (base, seen, server) = tls_server(CERT_A, KEY_A, vec![Hop::redirect(&elsewhere)]);
    assert_ne!(base, target, "a different port is a different origin");

    // The session the server would have set for ITS origin, written where the
    // app keeps it (`FilesDir/neezer/session.json` on a phone). The base string
    // IS the origin: scheme, host, port.
    let dir = scratch("redirect");
    let session = dir.join("session.json");
    std::fs::write(
        &session,
        format!(r#"{{"{base}":[["session","topsecret"]]}}"#),
    )
    .expect("a session file");

    let api = Api::builder(&base)
        .session_file(&session)
        .pinned_leaf_sha256(Some(pin_of(CERT_A)))
        .build()
        .expect("a client");
    assert!(api.has_session(), "the jar holds the server's cookie");
    let answer = api
        .get::<String>(Lane::User, "/me")
        .await
        .expect("the redirect is followed")
        .value;
    assert_eq!(answer, "elsewhere");

    let first = seen.recv().expect("the first hop");
    assert_eq!(
        first.header("cookie"),
        Some("session=topsecret"),
        "the configured origin gets the session: {}",
        first.head
    );
    let second = target_seen.recv().expect("the redirected hop");
    assert_eq!(
        second.header("cookie"),
        None,
        "another origin must never see the session: {}",
        second.head
    );
    assert!(
        second.head.starts_with("get /api/elsewhere "),
        "the redirected path is what is asked: {}",
        second.head
    );

    let stats = &api.stats()[0];
    assert_eq!(stats.requests, 2, "two hops, two requests on the link");
    assert_eq!(
        stats.bytes_in,
        body.len() as u64,
        "only the body that arrived is counted"
    );
    server.join().expect("the first server");
    target_server.join().expect("the second server");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A `Location` pointing at a plaintext address is refused BEFORE a socket
/// opens: a redirect must not be able to downgrade the session out of TLS.
#[tokio::test]
async fn a_cleartext_redirect_off_the_origin_is_refused() {
    let (target, target_seen, target_server) = tls_server(CERT_A, KEY_A, Vec::new());
    let plaintext = target.replace("https://", "http://");
    let (base, seen, server) = tls_server(CERT_A, KEY_A, vec![Hop::redirect(&plaintext)]);
    let api = Api::builder(&base)
        .pinned_leaf_sha256(Some(pin_of(CERT_A)))
        .build()
        .expect("a client");
    let error = api
        .get::<String>(Lane::User, "/me")
        .await
        .expect_err("plaintext off the configured origin is refused");
    match error {
        ApiError::Invalid(sentence) => assert!(
            sentence.contains("en clair"),
            "the sentence says what was refused: {sentence}"
        ),
        other => panic!("expected an Invalid refusal, got {other:?}"),
    }
    seen.recv().expect("the first hop");
    assert!(
        target_seen.try_recv().is_err(),
        "the redirected origin was never connected to"
    );
    assert_eq!(
        api.stats()[0].requests,
        1,
        "the refused hop was never a request"
    );
    server.join().expect("the first server");
    target_server.join().expect("the unused server");
}

/// **Rule 3's measure, in numbers.** Two screens, two clients on ONE `Api`: the
/// counters say exactly what each one cost — requests, response body bytes and
/// request body bytes — and the tags never mix.
#[tokio::test]
async fn the_counters_are_exact_and_separate_per_screen() {
    let home_body = b"\"home ok\"".to_vec();
    let search_body = b"\"search ok\"".to_vec();
    let (base, _seen, server) = tls_server(
        CERT_A,
        KEY_A,
        vec![Hop::ok(&home_body), Hop::ok(&search_body)],
    );
    let api = Api::builder(&base)
        .pinned_leaf_sha256(Some(pin_of(CERT_A)))
        .build()
        .expect("a client");
    assert_eq!(api.tag(), nz_api::TAG_NONE);

    let home = api.with_tag("home");
    assert_eq!(home.tag(), "home");
    let answer = home
        .get::<String>(Lane::User, "/home")
        .await
        .expect("the home screen's read")
        .value;
    assert_eq!(answer, "home ok");

    let search = api.with_tag("search");
    // A JSON string body, written by `serde_json` inside `send`: `"nézer"` is 8
    // bytes (two quotes, n, é as two UTF-8 bytes, z, e, r).
    let query = "nézer";
    search
        .send::<_, String>(Lane::User, Method::POST, "/search", Some(&query))
        .await
        .expect("the search screen's request");

    let stats = api.stats();
    assert_eq!(stats.len(), 2, "two tags, two lines: {stats:?}");
    let home_stats = stats.iter().find(|tag| tag.tag == "home").expect("home");
    assert_eq!(home_stats.requests, 1);
    assert_eq!(home_stats.bytes_in, home_body.len() as u64);
    assert_eq!(home_stats.bytes_out, 0, "a GET writes no body");
    let search_stats = stats
        .iter()
        .find(|tag| tag.tag == "search")
        .expect("search");
    assert_eq!(search_stats.requests, 1);
    assert_eq!(search_stats.bytes_in, search_body.len() as u64);
    assert_eq!(
        search_stats.bytes_out, 8,
        "the request body is counted exactly as written"
    );

    // A reset really forgets, and the shared counters stay shared: a new tag
    // starts from zero whatever the others spent.
    api.clear_stats();
    assert!(api.stats().is_empty());
    server.join().expect("the server thread");
}
