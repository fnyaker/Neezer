//! LN15's security acceptance: the threat model of `docs/agents/lan-nodes.md`
//! proven on the REAL code, with two (or three) real endpoints, real TLS, real
//! proofs and real frames — plus a raw quinn peer for everything a well-behaved
//! `Endpoint` would refuse to say.
//!
//! Each test is one line of the note's *Threat model* table, and each was run once
//! against a deliberately broken copy of the line it guards (the mutant campaign is
//! in the note's LN15 section and in the PR). A test that survives its mutant is
//! decoration.
//!
//! | # | Adversary | Test |
//! |---|---|---|
//! | M1 | unknown `kid` (another server signed it) | `every_broken_credential_is_refused_without_one_identity_byte` |
//! | M2 | expired by 601 s | same |
//! | M3 | not yet valid | same |
//! | M4 | one flipped byte in the credential | same |
//! | M5 | revoked device key | same |
//! | M6 | a proof replayed from another connection | `a_proof_from_another_connection_is_refused` |
//! | M7 | a responder's proof reflected as an initiator's | same |
//! | M8 | a device of another account, not a friend | `another_accounts_device_is_refused_and_learns_nothing` |
//! | M9 | a friend's out-of-scope `Fetch`/`Meta`/`Cover`/`Sets` | `a_friends_out_of_scope_answers_are_the_absent_ones_byte_for_byte` |
//! | M10 | a friend's inventory holds only what is shared | `a_friends_inventory_holds_only_the_shared_tracks` |
//! | M11 | a frame of 1 MiB + 1 | `a_frame_of_one_mebibyte_plus_one_is_refused_and_answered_nothing` |
//! | M12 | `from` past the end | `a_fetch_past_the_end_and_a_resume_with_another_tag_are_the_absent_answers` |
//! | M13 | a resume carrying another `FileTag` | same |
//! | M14 | more bytes than `len` | `tests/fetch.rs` (`a_peer_that_sends_more_than_its_length_is_a_bad_file`) |
//! | M15 | a wrong SHA-256 | `tests/fetch.rs` (`a_peer_that_lies_about_its_hash_is_a_bad_file`) |
//! | M16 | a forged roster gossiped | `a_forged_roster_and_an_older_revocation_list_change_nothing` |
//! | M17 | a lower-serial revocation list | same |
//! | M18 | 0-RTT offered | `tests/transport.rs` (`early_data_is_never_accepted`) |
//! | M19 | a responder that never sends `AuthOk` | `a_responder_that_never_answers_costs_two_seconds_and_no_request` |
//! | M20 | a stranger that sends half a frame | `a_stranger_that_sends_half_a_frame_is_answered_nothing` |
//! | M21 | a remote-control session on `/api/lan/*` | `tests/test_remote.py` (the route table) |
//! | M22 | a friend removed on the server | `a_removed_friend_is_refused_after_the_sync_and_through_gossip` |
//!
//! Nothing here touches the person's display, sound or network: every endpoint is
//! bound on an ephemeral `127.0.0.1` port, the fake server listens on loopback, and
//! every data directory is under the crate's own `target/` (never `/tmp`: tmpfs is
//! RAM). Every test is wrapped in a 60 s net so a deadlock fails with its name.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant, UNIX_EPOCH};

use aws_lc_rs::digest::{self, SHA256};
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{Value, json};

use nz_api::lanes::Gates;
use nz_api::{Api, SessionJar};
use nz_lan::auth;
use nz_lan::codec;
use nz_lan::cred::{Credential, Revocations, Role, Roster, TrustKeys, auth_message};
use nz_lan::gossip;
use nz_lan::index::Index;
use nz_lan::keys::DeviceKey;
use nz_lan::proto::{self, Control, FileTag, Request, Response, Urgency, WireQuality};
use nz_lan::serve::{ServeLimits, Server};
use nz_lan::store::Store;
use nz_lan::transport::{self, Endpoint, Identity, PeerConn, SharedTrust, TrustState};
use nz_lan::types::{Kid, LanError, Subject};

// --- the note's reference material, written out again ------------------------

/// The test server's LAN key: every credential, revocation list and roster below is
/// signed by it, exactly as `neezer/lan.py` signs them.
const SERVER_SEED: [u8; 32] = [0x5a; 32];
/// Another server's key: what a credential of the same shape looks like when
/// somebody else signed it (M1).
const OTHER_SEED: [u8; 32] = [0x6b; 32];

const CREDENTIAL_DOMAIN: &[u8] = b"neezer-lan/1 credential\0";
const REVOCATIONS_DOMAIN: &[u8] = b"neezer-lan/1 revocations\0";
const ROSTER_DOMAIN: &[u8] = b"neezer-lan/1 roster\0";

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or(0)
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Standard base64 with padding, encoded here so a broken encoder in the crate
/// cannot agree with itself.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0] as u32;
        let second = chunk.get(1).copied().unwrap_or(0) as u32;
        let third = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (first << 16) | (second << 8) | third;
        out.push(ALPHABET[(triple >> 18) as usize & 0x3F] as char);
        out.push(ALPHABET[(triple >> 12) as usize & 0x3F] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(triple >> 6) as usize & 0x3F] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[triple as usize & 0x3F] as char
        } else {
            '='
        });
    }
    out
}

fn server() -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&SERVER_SEED).expect("la clé LAN de test")
}

fn other() -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&OTHER_SEED).expect("la clé de l'autre serveur")
}

fn public_of(pair: &Ed25519KeyPair) -> [u8; 32] {
    let mut public = [0u8; 32];
    public.copy_from_slice(pair.public_key().as_ref());
    public
}

fn kid_of(pair: &Ed25519KeyPair) -> Kid {
    let hash = digest::digest(&SHA256, &public_of(pair));
    let mut kid = [0u8; 8];
    kid.copy_from_slice(&hash.as_ref()[..8]);
    Kid(kid)
}

fn kid() -> Kid {
    kid_of(&server())
}

fn lan_keys() -> TrustKeys {
    TrustKeys::from_status(&[(kid(), public_of(&server()))])
}

fn sign_with(pair: &Ed25519KeyPair, domain: &[u8], message: &[u8]) -> Vec<u8> {
    let mut signed = domain.to_vec();
    signed.extend_from_slice(message);
    pair.sign(&signed).as_ref().to_vec()
}

fn sign(domain: &[u8], message: &[u8]) -> Vec<u8> {
    sign_with(&server(), domain, message)
}

fn subject_of(value: u8) -> Subject {
    Subject([value; 16])
}

/// The note's credential layout, 154 bytes.
fn credential_bytes(
    pair: &Ed25519KeyPair,
    subject: Subject,
    device_pub: [u8; 32],
    window: (i64, i64),
) -> Vec<u8> {
    let mut body = Vec::with_capacity(90);
    body.extend_from_slice(b"NZLC");
    body.push(1);
    body.extend_from_slice(&kid_of(pair).0);
    body.extend_from_slice(&subject.0);
    body.extend_from_slice(&[0x01; 12]);
    body.extend_from_slice(&device_pub);
    body.extend_from_slice(&window.0.to_be_bytes());
    body.extend_from_slice(&window.1.to_be_bytes());
    body.push(0b11);
    assert_eq!(body.len(), 90, "le corps de l'attestation fait 90 octets");
    let signature = sign_with(pair, CREDENTIAL_DOMAIN, &body);
    body.extend_from_slice(&signature);
    assert_eq!(body.len(), 154);
    body
}

/// A device key written by the real key path (`0600`, atomic), under the crate's
/// own `target/`, never `/tmp`.
fn device_key(name: &str) -> DeviceKey {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("ln15-sec-key-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("un répertoire de test");
    DeviceKey::load_or_create(&dir).expect("une clé d'appareil")
}

/// One enrolled device: its key, its credential of the test server, and the
/// identity a node is built from.
fn device(name: &str, subject: Subject, window: (i64, i64)) -> Arc<Identity> {
    let key = device_key(name);
    let cred = Credential::decode(&credential_bytes(&server(), subject, key.public(), window))
        .expect("une attestation de test valide");
    Arc::new(Identity { key, cred })
}

/// The empty revocation list of the test server key.
fn no_revocations() -> Revocations {
    Revocations::empty(kid())
}

/// The signed bytes of a revocation list naming `keys`.
fn revocation_bytes(serial: u64, keys: &[[u8; 32]]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(b"NZLR");
    body.push(1);
    body.extend_from_slice(&kid().0);
    body.extend_from_slice(&serial.to_be_bytes());
    body.extend_from_slice(&now().to_be_bytes());
    body.extend_from_slice(&(keys.len() as u32).to_be_bytes());
    let mut sorted = keys.to_vec();
    sorted.sort();
    for key in &sorted {
        body.extend_from_slice(key);
    }
    let signature = sign(REVOCATIONS_DOMAIN, &body);
    body.extend_from_slice(&signature);
    body
}

fn revocation_list(serial: u64, keys: &[[u8; 32]]) -> Revocations {
    Revocations::decode_verify(&revocation_bytes(serial, keys), &lan_keys())
        .expect("une liste de révocation valide")
}

/// One share of a roster body: what I give to a subject.
struct Share {
    subject: Subject,
    tracks: Vec<&'static str>,
}

/// A signed roster body (the bytes that are signed) and its envelope.
fn roster_envelope(
    subject: Subject,
    serial: u64,
    shares: &[Share],
    trusted_by: &[(Subject, &str)],
) -> Vec<u8> {
    let shares = shares
        .iter()
        .map(|share| {
            let tracks: Vec<String> = share
                .tracks
                .iter()
                .map(|track| format!("\"{track}\""))
                .collect();
            format!(
                r#"{{"subject":"{}","all":false,"playlists":[{{"id":"p1","title":"Route","tracks":[{}]}}]}}"#,
                to_hex(&share.subject.0),
                tracks.join(",")
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let trusted = trusted_by
        .iter()
        .map(|(them, name)| format!(r#"{{"subject":"{}","name":"{}"}}"#, to_hex(&them.0), name))
        .collect::<Vec<_>>()
        .join(",");
    let body = format!(
        r#"{{"v":1,"kid":"{}","subject":"{}","serial":{serial},"issued":{},"expires":{},"shares_to":[{shares}],"trusted_by":[{trusted}]}}"#,
        to_hex(&kid().0),
        to_hex(&subject.0),
        now() - 60,
        now() + 86_400,
    );
    let signature = sign(ROSTER_DOMAIN, body.as_bytes());
    format!(
        r#"{{"body":"{}","sig":"{}"}}"#,
        base64(body.as_bytes()),
        base64(&signature)
    )
    .into_bytes()
}

fn roster_from(envelope: &[u8]) -> Roster {
    Roster::decode_verify(envelope, &lan_keys()).expect("un roster signé valide")
}

fn roster(subject: Subject, serial: u64, shares: &[Share]) -> Roster {
    roster_from(&roster_envelope(subject, serial, shares, &[]))
}

/// A roster envelope signed by a key nobody trusts (M16), built around a body that
/// WOULD be accepted if its signature were (it shares everything with 0xee).
fn forged_roster_envelope(subject: Subject) -> Vec<u8> {
    let body = format!(
        r#"{{"v":1,"kid":"{}","subject":"{}","serial":999,"issued":{},"expires":{},"shares_to":[{{"subject":"{}","all":true,"playlists":[]}}],"trusted_by":[]}}"#,
        to_hex(&kid().0),
        to_hex(&subject.0),
        now() - 60,
        now() + 86_400,
        to_hex(&subject_of(0xee).0),
    );
    let signature = sign_with(&other(), ROSTER_DOMAIN, body.as_bytes());
    format!(
        r#"{{"body":"{}","sig":"{}"}}"#,
        base64(body.as_bytes()),
        base64(&signature)
    )
    .into_bytes()
}

fn trust(
    subject: Subject,
    roster: Option<Roster>,
    revocations: Option<Revocations>,
) -> SharedTrust {
    let mut state = TrustState {
        subject,
        keys: lan_keys(),
        revocations: HashMap::new(),
        roster,
    };
    state
        .revocations
        .insert(kid(), revocations.unwrap_or_else(no_revocations));
    Arc::new(std::sync::RwLock::new(state))
}

fn hello(name: &str) -> proto::Hello {
    proto::Hello {
        name: name.to_owned(),
        platform: "desktop".to_owned(),
        app_version: "0.1.0".to_owned(),
        inventory_version: 3,
        sets_version: 1,
    }
}

/// Binds a node on an ephemeral loopback port.
fn node(id: Arc<Identity>, trust: SharedTrust, name: &str) -> Endpoint {
    let handle = tokio::runtime::Handle::current();
    let endpoint = Endpoint::bind(&handle, 0, id, trust, hello(name)).expect("un endpoint lié");
    assert!(!endpoint.port_fallback(), "un port éphémère ne bascule pas");
    assert_ne!(endpoint.port(), 0);
    endpoint
}

fn address(endpoint: &Endpoint) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], endpoint.port()))
}

/// A generous bound around a test's body: a deadlock must FAIL, with its name,
/// rather than hold the test harness for ever.
async fn bounded<F: std::future::Future>(name: &str, body: F) -> F::Output {
    match tokio::time::timeout(Duration::from_secs(60), body).await {
        Ok(value) => value,
        Err(_) => panic!("{name} : la borne de 60 s est atteinte (interblocage ?)"),
    }
}

// --- the raw peer: what an attacker has once TLS is passed -------------------

fn auth_control(cred: &Credential, proof: &[u8], proto: u32, serial: u64) -> Control {
    Control::Auth {
        cred: base64(cred.raw()),
        proof: base64(proof),
        revocation_serial: serial,
        proto,
    }
}

struct RawClient {
    endpoint: quinn::Endpoint,
    conn: quinn::Connection,
}

impl RawClient {
    async fn dial(address: SocketAddr, id: &Identity) -> RawClient {
        let (endpoint, connecting) = RawClient::start(address, id);
        let conn = connecting.await.expect("une poignée de main brute");
        RawClient { endpoint, conn }
    }

    /// The dial, before its handshake completes: the responder only progresses a
    /// handshake while its `accept()` is polled, so a raw test MUST start the dial
    /// and the accept in the same `join!`.
    fn start(address: SocketAddr, id: &Identity) -> (quinn::Endpoint, quinn::Connecting) {
        let tls = transport::client_tls_config(id).expect("la configuration TLS client");
        let mut endpoint = quinn::Endpoint::client(SocketAddr::from(([127, 0, 0, 1], 0)))
            .expect("un endpoint client brut");
        endpoint.set_default_client_config(quinn::ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(tls).expect("client QUIC"),
        )));
        let connecting = endpoint
            .connect(address, "neezer-lan")
            .expect("un dial brut");
        (endpoint, connecting)
    }

    async fn send(&self, frame: &Control) -> (quinn::SendStream, quinn::RecvStream) {
        let (mut send, recv) = self.conn.open_bi().await.expect("un flux de contrôle");
        codec::write_frame(&mut send, frame)
            .await
            .expect("une trame écrite");
        (send, recv)
    }

    /// Everything the responder writes before it closes: **the count is the
    /// assertion**. A refusal that leaks one byte of a credential, a name or a key
    /// makes this non-zero.
    async fn bytes_written(&self, mut recv: quinn::RecvStream) -> usize {
        let mut written = 0usize;
        let mut buffer = [0u8; 4096];
        loop {
            match recv.read(&mut buffer).await {
                Ok(Some(read)) => written += read,
                Ok(None) => break,
                Err(_) => break,
            }
        }
        written
    }

    async fn shutdown(self) {
        self.conn.close(0u32.into(), b"");
        self.endpoint.close(0u32.into(), b"");
        self.endpoint.wait_idle().await;
    }
}

/// Completes the handshake as a raw peer: `Auth`, the responder's `AuthOk`, then
/// `Hello` on the same control stream. The Hello is part of the exchange, not a
/// politeness: a responder whose peer never sends one closes the connection after
/// `HELLO_DEADLINE` and answers `Err(Refused)`, which is what a test that only sends
/// the proof would otherwise read as a broken fixture.
async fn authenticate_raw(client: &RawClient, id: &Identity) {
    let exporter = auth::exporter(&client.conn, Role::Initiator).expect("l'exporteur");
    let proof = id.key.sign(&auth_message(Role::Initiator, &exporter));
    let frame = auth_control(&id.cred, &proof, 1, 0);
    let (mut send, mut recv) = client.send(&frame).await;
    let answer = codec::read_frame::<Control>(&mut recv, codec::MAX_FRAME_BYTES)
        .await
        .expect("une réponse d'authentification")
        .expect("un AuthOk");
    assert!(
        matches!(answer, Control::AuthOk { .. }),
        "un pair accepté reçoit son AuthOk : {answer:?}"
    );
    codec::write_frame(
        &mut send,
        &Control::Hello {
            name: "Téléphone".to_owned(),
            platform: "desktop".to_owned(),
            app_version: "0.1.0".to_owned(),
            inventory_version: 0,
            sets_version: 0,
        },
    )
    .await
    .expect("le Hello du pair brut");
}

/// Waits for the connection's close and answers its application code and reason.
async fn close_of(conn: &quinn::Connection) -> (u32, String) {
    match conn.closed().await {
        quinn::ConnectionError::ApplicationClosed(closed) => (
            u32::try_from(closed.error_code.into_inner()).unwrap_or(u32::MAX),
            String::from_utf8_lossy(&closed.reason).to_string(),
        ),
        other => panic!("la connexion devait être fermée par l'application, pas {other:?}"),
    }
}

/// A raw quinn SERVER: the TLS this crate builds, and none of its handshake. What a
/// rogue node on the Wi-Fi answers the initiator.
async fn raw_server(id: &Identity) -> (quinn::Endpoint, SocketAddr) {
    let tls = transport::server_tls_config(id).expect("la configuration TLS serveur");
    let server = quinn::ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(tls).expect("serveur QUIC"),
    ));
    let endpoint = quinn::Endpoint::server(server, SocketAddr::from(([127, 0, 0, 1], 0)))
        .expect("un endpoint serveur brut");
    let address = endpoint.local_addr().expect("une adresse");
    (endpoint, address)
}

// --- the library a responder serves ------------------------------------------

/// A throwaway cache, data directory and `lan.sqlite` under the crate's `target/`.
struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("ln15-sec-{tag}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("fixture dir");
        Fixture { root }
    }

    fn cache(&self) -> nz_cache::Cache {
        nz_cache::Cache::open_with_cache_dir(self.root.join("data"), self.root.join("cache"))
            .expect("cache")
    }

    fn store(&self) -> Store {
        Store::open(&self.root.join("lan")).expect("lan.sqlite")
    }
}

/// One set on disk, every member complete: what an inventory and a `Sets` answer
/// are made of.
fn seed_set(
    cache: &nz_cache::Cache,
    kind: &str,
    ident: &str,
    title: &str,
    tracks: &[(&str, &[u8])],
) {
    let members: Vec<nz_cache::DownloadTrack> = tracks
        .iter()
        .map(|(track, _)| nz_cache::DownloadTrack {
            track: nz_model::TrackId::new(*track),
            quality: nz_model::Quality::Flac,
        })
        .collect();
    let downloads = cache.downloads();
    downloads
        .add_set("1", kind, ident, title, &members)
        .expect("add set");
    for (track, bytes) in tracks {
        let id = nz_model::TrackId::new(*track);
        let partial = downloads
            .reserve("1", &id, nz_model::Quality::Flac)
            .expect("reserve");
        std::fs::write(&partial, bytes).expect("bytes");
        downloads
            .complete("1", &id, bytes.len() as u64)
            .expect("complete");
    }
}

/// The cached set page a node answers `Meta`/`Cover` from: the server's shape.
fn set_page(cache: &nz_cache::Cache, ident: &str, title: &str, cover: &str, tracks: &[&str]) {
    let rows: Vec<Value> = tracks
        .iter()
        .map(|track| {
            json!({
                "deezer_id": track,
                "title": format!("Titre {track}"),
                "artist": { "name": "Artiste" },
                "album": { "title": "Album", "cover": cover, "deezer_id": "77" },
                "duration": 3,
            })
        })
        .collect();
    let page = json!({
        "playlist": { "id": ident, "title": title, "cover": cover, "nb_tracks": tracks.len() },
        "tracks": rows,
    });
    cache.put(
        "1",
        &format!("/playlist/{ident}"),
        &serde_json::to_string(&page).expect("page"),
    );
}

/// A 1×1 PNG, what a cover store sniffs as `png`.
const PNG: &[u8] = &[
    0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H', b'D', b'R',
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0a, b'I', b'D', b'A', b'T', 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', 0xae,
    0x42, 0x60, 0x82,
];

/// The gates a responder takes: an unbound `Api`'s own.
fn gates() -> Arc<Gates> {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ln15-sec-api");
    std::fs::create_dir_all(&dir).expect("api dir");
    let api = Api::builder("http://127.0.0.1:9")
        .session(Arc::new(SessionJar::at(dir.join("session.json"))))
        .build()
        .expect("un client");
    api.gates()
}

/// Answers every request `conn` sends with `server`, inside the scope the handshake
/// gave it — the loop the `Lan` service runs, minus the service.
async fn serve(conn: PeerConn, server: Arc<Server>) {
    let scope = conn.info().scope.clone();
    while let Some(incoming) = conn.next_incoming().await {
        if server.handle(&scope, incoming).await.is_err() {
            break;
        }
    }
}

/// Sends one request on a NEW stream and answers the frame as it travelled, byte
/// for byte: the only way to compare two refusals (M9).
async fn raw_request(conn: &PeerConn, request: &Request) -> Vec<u8> {
    let (mut send, mut recv) = conn
        .connection()
        .open_bi()
        .await
        .expect("un flux de requête");
    codec::write_frame(&mut send, request)
        .await
        .expect("une requête écrite");
    send.finish().expect("fin de la requête");
    codec::read_frame_bytes(&mut recv, codec::MAX_FRAME_BYTES)
        .await
        .expect("une réponse")
        .expect("une réponse non vide")
}

/// The bytes of the one refusal every "I cannot tell you" answer is built from.
fn absent_bytes() -> Vec<u8> {
    serde_json::to_vec(&Response::NotFound).expect("NotFound")
}

/// One request on its own stream, its head and `body_len` bytes of body.
///
/// The stream is opened HERE and read exactly, not through `Reply::body`:
/// `PeerConn::request` builds a body whose `remaining` is the head's `len`, which
/// is the WHOLE file, so on a resume (`from > 0`) that helper waits one byte past
/// the FIN and reads `PeerGone`. LN9's transfer path opens the stream itself for
/// this exact reason; a test that reads a resume takes the same door.
async fn request(
    conn: &PeerConn,
    request: Request,
    body_len: u64,
) -> Result<(Response, Vec<u8>), LanError> {
    let (mut send, mut recv) = conn
        .connection()
        .open_bi()
        .await
        .map_err(|_| LanError::PeerGone)?;
    codec::write_frame(&mut send, &request).await?;
    send.finish().map_err(|_| LanError::PeerGone)?;
    let head = codec::read_frame::<Response>(&mut recv, codec::MAX_FRAME_BYTES)
        .await?
        .ok_or(LanError::PeerGone)?;
    let mut body = vec![0u8; body_len as usize];
    if body_len > 0 {
        recv.read_exact(&mut body)
            .await
            .map_err(|_| LanError::PeerGone)?;
    }
    Ok((head, body))
}

// --- the shared fixture of M9/M10/M12/M13 ------------------------------------

/// An owner's node serving a real library, and a FRIEND's authenticated connection
/// to it, inside the scope the owner's roster gives that friend.
struct Shared {
    owner: Endpoint,
    friend_conn: PeerConn,
    /// The friend's own endpoint, kept alive for the connection it dialled.
    _friend: Endpoint,
    _fixture: Fixture,
    _cache: nz_cache::Cache,
    _server: Arc<Server>,
    _task: tokio::task::JoinHandle<()>,
}

impl Shared {
    async fn new(tag: &str) -> Shared {
        let fixture = Fixture::new(tag);
        let cache = fixture.cache();
        // In scope: set 900 (`t1`, `t2`) with its page and its cover. Out of scope:
        // set 901 (`t9`) with its own page and its own cover.
        seed_set(
            &cache,
            "playlist",
            "900",
            "Route",
            &[("t1", b"one"), ("t2", b"two")],
        );
        seed_set(&cache, "playlist", "901", "Privee", &[("t9", b"nine")]);
        set_page(&cache, "900", "Route", "/api/cover/900", &["t1", "t2"]);
        set_page(&cache, "901", "Privee", "/api/cover/901", &["t9"]);
        cache
            .covers()
            .put("/api/cover/900", PNG)
            .expect("la pochette partagée");
        cache
            .covers()
            .put("/api/cover/901", PNG)
            .expect("la pochette privée");
        let index = Index::new(cache.clone(), "1".to_owned(), fixture.store());
        index.refresh();

        let owner_subject = subject_of(0x81);
        let friend_subject = subject_of(0x82);
        let owner = device(
            &format!("{tag} owner"),
            owner_subject,
            (now() - 60, now() + 86_400),
        );
        let friend = device(
            &format!("{tag} friend"),
            friend_subject,
            (now() - 60, now() + 86_400),
        );
        let owner_endpoint = node(
            Arc::clone(&owner),
            trust(
                owner_subject,
                Some(roster(
                    owner_subject,
                    7,
                    &[Share {
                        subject: friend_subject,
                        tracks: vec!["t1", "t2"],
                    }],
                )),
                None,
            ),
            "PC",
        );
        // The friend must authorise the owner too: a connection is mutual, and the
        // owner is in the friend's `trusted_by`.
        let friend_endpoint = node(
            Arc::clone(&friend),
            trust(
                friend_subject,
                Some(roster_from(&roster_envelope(
                    friend_subject,
                    1,
                    &[],
                    &[(owner_subject, "PC")],
                ))),
                None,
            ),
            "Téléphone",
        );
        let server = Arc::new(Server::new(index, gates(), ServeLimits::default()));

        // The dial and the accept together: the responder only progresses a
        // handshake while its `accept()` is polled.
        let (accepted, dialled) = tokio::join!(owner_endpoint.accept(), async {
            friend_endpoint.connect(address(&owner_endpoint)).await
        });
        let accepted = accepted
            .expect("une connexion acceptée")
            .expect("le pair ami est autorisé");
        let friend_conn = dialled.expect("le téléphone s'authentifie");
        assert!(
            matches!(accepted.info().relation, nz_lan::Relation::Friend { .. }),
            "l'ami est autorisé comme ami : {:?}",
            accepted.info().relation
        );
        let task = tokio::spawn(serve(accepted, Arc::clone(&server)));
        Shared {
            owner: owner_endpoint,
            friend_conn,
            _friend: friend_endpoint,
            _fixture: fixture,
            _cache: cache,
            _server: server,
            _task: task,
        }
    }

    async fn close(self) {
        self._task.abort();
        self.owner.close().await;
        self._friend.close().await;
    }
}

// --- 1. M1 to M5: every broken credential -------------------------------

/// **M1–M5**: an unknown `kid`, a credential expired by 601 s, one not yet valid,
/// one with a flipped byte, and a revoked device key — five causes, **one** close
/// code (`0x10`), an EMPTY reason, and **0 application bytes** written back, each.
///
/// The control that makes it a proof: an intact credential of the same person IS
/// accepted (and gets an `AuthOk`), so the refusals are the cause and not a broken
/// fixture.
#[tokio::test]
async fn every_broken_credential_is_refused_without_one_identity_byte() {
    bounded(
        "every_broken_credential_is_refused_without_one_identity_byte",
        async {
            let subject = subject_of(0x51);
            let owner = device("broken-owner", subject, (now() - 60, now() + 86_400));
            let endpoint = node(
                Arc::clone(&owner),
                trust(subject, Some(roster(subject, 1, &[])), None),
                "PC",
            );

            let unknown_kid = {
                let key = device_key("broken-unknown-kid");
                let raw = credential_bytes(
                    &other(),
                    subject,
                    key.public(),
                    (now() - 60, now() + 86_400),
                );
                Arc::new(Identity {
                    key,
                    cred: Credential::decode(&raw).expect("une attestation de forme valide"),
                })
            };
            let expired = device("broken-expired", subject, (now() - 172_800, now() - 3_600));
            let not_yet = device("broken-not-yet", subject, (now() + 3_600, now() + 86_400));
            let flipped = {
                let key = device_key("broken-flipped");
                let mut raw = credential_bytes(
                    &server(),
                    subject,
                    key.public(),
                    (now() - 60, now() + 86_400),
                );
                // One byte of the SIGNED BODY, not the signature: the credential is
                // well-formed and says something the server never signed.
                raw[45] ^= 0x01;
                Arc::new(Identity {
                    key,
                    cred: Credential::decode(&raw).expect("une attestation de forme valide"),
                })
            };
            // The sharpest forgery a stranger can mint with nothing: a credential
            // of the right SHAPE, naming a `kid` nobody knows, signed by the
            // DEVICE's own key. An implementation that falls back to the key the
            // credential carries accepts it; the server's key is the only one that
            // may sign, so this is refused like the rest.
            let self_signed = {
                let key = device_key("broken-self-signed");
                let mut raw = credential_bytes(
                    &other(),
                    subject,
                    key.public(),
                    (now() - 60, now() + 86_400),
                );
                let mut message = CREDENTIAL_DOMAIN.to_vec();
                message.extend_from_slice(&raw[..90]);
                let signature = key.sign(&message);
                raw.truncate(90);
                raw.extend_from_slice(signature.as_ref());
                assert_eq!(raw.len(), 154);
                Arc::new(Identity {
                    key,
                    cred: Credential::decode(&raw).expect("une attestation de forme valide"),
                })
            };
            let revoked_device = device("broken-revoked", subject, (now() - 60, now() + 86_400));
            let revoked = revocation_list(9, &[revoked_device.key.public()]);
            let with_revocations = node(
                Arc::clone(&owner),
                trust(subject, Some(roster(subject, 1, &[])), Some(revoked)),
                "PC-revocations",
            );

            let mut causes = 0;
            for (name, attacker, against) in [
                ("kid inconnu", Arc::clone(&unknown_kid), &endpoint),
                ("expirée depuis 1 h", Arc::clone(&expired), &endpoint),
                ("valide dans 1 h", Arc::clone(&not_yet), &endpoint),
                ("un octet retourné", Arc::clone(&flipped), &endpoint),
                (
                    "clé révoquée",
                    Arc::clone(&revoked_device),
                    &with_revocations,
                ),
                // The self-signed forgery goes against the plain node: it must be
                // refused as an unknown kid, not as a revocation.
                (
                    "attestation auto-signée",
                    Arc::clone(&self_signed),
                    &endpoint,
                ),
            ] {
                let (accepted, (client, written)) = tokio::join!(against.accept(), async {
                    let client = RawClient::dial(address(against), &attacker).await;
                    let exporter =
                        auth::exporter(&client.conn, Role::Initiator).expect("l'exporteur");
                    let proof = attacker.key.sign(&auth_message(Role::Initiator, &exporter));
                    let frame = auth_control(&attacker.cred, &proof, 1, 0);
                    let (_send, recv) = client.send(&frame).await;
                    let written = client.bytes_written(recv).await;
                    (client, written)
                });
                assert_eq!(written, 0, "« {name} » a reçu {written} octets");
                assert_eq!(
                    accepted.map(|result| result.err()),
                    Some(Some(LanError::Refused)),
                    "« {name} » devait être refusé"
                );
                let (code, reason) = close_of(&client.conn).await;
                assert_eq!(code, auth::CLOSE_REFUSED, "« {name} » : code");
                assert_eq!(reason, "", "« {name} » : la raison d'un refus est vide");
                client.shutdown().await;
                causes += 1;
            }
            assert_eq!(causes, 6);

            // The control: the intact device of the same person IS accepted, its
            // `AuthOk` arrives and the exchange completes.
            let (accepted, client) = tokio::join!(endpoint.accept(), async {
                let client = RawClient::dial(address(&endpoint), &revoked_device).await;
                authenticate_raw(&client, &revoked_device).await;
                client
            });
            assert!(
                accepted.is_some_and(|result| result.is_ok()),
                "l'appareil intact devait être accepté par le nœud sans révocation"
            );
            client.shutdown().await;
            with_revocations.close().await;
            endpoint.close().await;
        },
    )
    .await;
}

// --- 2. M6 and M7: replay and reflection --------------------------------

/// **M6/M7**: a proof signed over ANOTHER connection's exporter (a recorded
/// handshake, replayed) and a responder's `AuthOk` proof replayed as an
/// initiator's (a reflection) are both refused, **0 bytes** and one code — while the
/// same peer's `Auth` over its OWN connection completes the exchange.
#[tokio::test]
async fn a_proof_from_another_connection_is_refused() {
    bounded("a_proof_from_another_connection_is_refused", async {
        let subject = subject_of(0x61);
        let owner = device("replay-owner", subject, (now() - 60, now() + 86_400));
        let attacker = device("replay-attacker", subject, (now() - 60, now() + 86_400));
        let endpoint = node(
            Arc::clone(&owner),
            trust(subject, Some(roster(subject, 1, &[])), None),
            "PC",
        );

        // 1. A first connection, whose exporter the attacker RECORDS, then leaves
        //    without authenticating — nothing is sent, nothing is learnt.
        let (accepted, recorded) = tokio::join!(endpoint.accept(), async {
            let client = RawClient::dial(address(&endpoint), &attacker).await;
            let exporter = auth::exporter(&client.conn, Role::Initiator).expect("l'exporteur");
            client.shutdown().await;
            exporter
        });
        assert!(
            !accepted.is_some_and(|result| result.is_ok()),
            "une connexion sans preuve ne s'authentifie pas"
        );

        // 2. Replayed on a SECOND connection: well-formed, valid key, intact
        //    credential — and it does not verify, because it signs the FIRST
        //    connection's exporter. The reflection is made on the same live session,
        //    where the responder-role exporter has the same bytes on both sides.
        let (accepted, (client, written, reflected)) = tokio::join!(endpoint.accept(), async {
            let client = RawClient::dial(address(&endpoint), &attacker).await;
            let replayed = attacker.key.sign(&auth_message(Role::Initiator, &recorded));
            let frame = auth_control(&attacker.cred, &replayed, 1, 0);
            let (_send, recv) = client.send(&frame).await;
            let written = client.bytes_written(recv).await;
            // What this connection's responder signs: a proof over ITS role. Replayed
            // as an initiator's, the signed bytes are a different message.
            let responder_exporter =
                auth::exporter(&client.conn, Role::Responder).expect("l'exporteur");
            let reflected = owner
                .key
                .sign(&auth_message(Role::Responder, &responder_exporter));
            (client, written, reflected)
        });
        assert_eq!(written, 0, "une preuve rejouée ne reçoit aucun octet");
        assert_eq!(
            accepted.map(|result| result.err()),
            Some(Some(LanError::Refused))
        );
        let (code, reason) = close_of(&client.conn).await;
        assert_eq!(code, auth::CLOSE_REFUSED);
        assert_eq!(reason, "");
        client.shutdown().await;

        // 3. The REFLECTION on a third connection: the owner's own device key, a
        //    proof that is valid on the connection it was made for, in the WRONG
        //    role. Both sides sign `role ‖ exporter`, so the responder's verifier
        //    (which reads `Role::Initiator`) does not accept it.
        let (accepted, (client, written)) = tokio::join!(endpoint.accept(), async {
            let client = RawClient::dial(address(&endpoint), &owner).await;
            let frame = auth_control(&owner.cred, &reflected, 1, 0);
            let (_send, recv) = client.send(&frame).await;
            let written = client.bytes_written(recv).await;
            (client, written)
        });
        assert_eq!(written, 0, "une preuve réfléchie ne reçoit aucun octet");
        assert_eq!(
            accepted.map(|result| result.err()),
            Some(Some(LanError::Refused))
        );
        let (code, _) = close_of(&client.conn).await;
        assert_eq!(code, auth::CLOSE_REFUSED);
        client.shutdown().await;

        // 4. The control: the same device, the same connection type, its own proof
        //    over its own exporter — the exchange completes.
        let (accepted, client) = tokio::join!(endpoint.accept(), async {
            let client = RawClient::dial(address(&endpoint), &attacker).await;
            authenticate_raw(&client, &attacker).await;
            client
        });
        assert!(
            accepted.is_some_and(|result| result.is_ok()),
            "le même appareil, avec sa vraie preuve, est accepté"
        );
        client.shutdown().await;

        endpoint.close().await;
    })
    .await;
}

// --- 3. M8: another account's device -----------------------------------

/// **M8**: a device of another account (its own subject, its own valid credential
/// and proof) is refused and gets **0 bytes** — not its own `AuthOk`, not the
/// responder's name, not one identifier.
#[tokio::test]
async fn another_accounts_device_is_refused_and_learns_nothing() {
    bounded(
        "another_accounts_device_is_refused_and_learns_nothing",
        async {
            let mine = subject_of(0x71);
            let theirs = subject_of(0x72);
            let owner = device("other-account-owner", mine, (now() - 60, now() + 86_400));
            let stranger = device("other-account-guest", theirs, (now() - 60, now() + 86_400));
            // My roster shares with somebody ELSE (0x73), never with 0x72.
            let endpoint = node(
                Arc::clone(&owner),
                trust(
                    mine,
                    Some(roster(
                        mine,
                        1,
                        &[Share {
                            subject: subject_of(0x73),
                            tracks: vec!["t1"],
                        }],
                    )),
                    None,
                ),
                "PC",
            );

            let (accepted, (client, written)) = tokio::join!(endpoint.accept(), async {
                let client = RawClient::dial(address(&endpoint), &stranger).await;
                let exporter = auth::exporter(&client.conn, Role::Initiator).expect("l'exporteur");
                let proof = stranger.key.sign(&auth_message(Role::Initiator, &exporter));
                let frame = auth_control(&stranger.cred, &proof, 1, 0);
                let (_send, recv) = client.send(&frame).await;
                let written = client.bytes_written(recv).await;
                (client, written)
            });
            assert_eq!(
                written, 0,
                "un autre compte ne reçoit aucun octet d'identité"
            );
            assert_eq!(
                accepted.map(|result| result.err()),
                Some(Some(LanError::Refused))
            );
            let (code, reason) = close_of(&client.conn).await;
            assert_eq!(code, auth::CLOSE_REFUSED);
            assert_eq!(reason, "");
            client.shutdown().await;
            endpoint.close().await;
        },
    )
    .await;
}

// --- 4. M9 and M10: a friend's scope ------------------------------------

/// **M9**: for a friend, every out-of-scope answer is the SAME frame as an absent
/// one, byte for byte — an unshared track, an unshared cover, an unshared set, and
/// an id nothing knows. **The control**: the shared track IS served, so the frames
/// above are the scope and not an empty library.
#[tokio::test]
async fn a_friends_out_of_scope_answers_are_the_absent_ones_byte_for_byte() {
    bounded(
        "a_friends_out_of_scope_answers_are_the_absent_ones_byte_for_byte",
        async {
            let shared = Shared::new("scope").await;
            let conn = &shared.friend_conn;
            let absent = absent_bytes();

            // 1. `Fetch`: in scope, out of scope, unknown.
            let (head, body) = request(
                conn,
                Request::Fetch {
                    track: "t1".to_owned(),
                    quality: WireQuality::Flac,
                    from: 0,
                    expect: None,
                    urgency: Urgency::Play,
                },
                3,
            )
            .await
            .expect("le titre partagé se sert");
            assert!(
                matches!(head, Response::FetchOk { len: 3, .. }),
                "le titre partagé est servi : {head:?}"
            );
            assert_eq!(body, b"one", "les octets du titre partagé");

            let unshared = raw_request(
                conn,
                &Request::Fetch {
                    track: "t9".to_owned(),
                    quality: WireQuality::Flac,
                    from: 0,
                    expect: None,
                    urgency: Urgency::Play,
                },
            )
            .await;
            let unknown = raw_request(
                conn,
                &Request::Fetch {
                    track: "t-inexistant".to_owned(),
                    quality: WireQuality::Flac,
                    from: 0,
                    expect: None,
                    urgency: Urgency::Play,
                },
            )
            .await;
            assert_eq!(unshared, absent, "un titre non partagé est « absent »");
            assert_eq!(unknown, absent, "un titre inconnu est « absent »");
            assert_eq!(unshared, unknown, "les deux refus sont identiques");

            // 2. `Meta`: the same empty answer for an unshared id and an unknown
            //    one (LN4's correction: a 200-id call is not refused whole).
            let unshared = raw_request(
                conn,
                &Request::Meta {
                    tracks: vec!["t9".to_owned()],
                },
            )
            .await;
            let unknown = raw_request(
                conn,
                &Request::Meta {
                    tracks: vec!["t-inexistant".to_owned()],
                },
            )
            .await;
            assert_eq!(unshared, unknown, "Meta : même trame pour les deux");
            let metadata: Response = serde_json::from_slice(&unshared).expect("une trame Meta");
            assert!(
                matches!(metadata, Response::Meta { ref rows } if rows.is_empty()),
                "aucune métadonnée hors partage : {metadata:?}"
            );

            // 3. `Cover`: the art of a set the friend cannot see is absent, exactly
            //    like a cover nobody has.
            let unshared = raw_request(
                conn,
                &Request::Cover {
                    url: "/api/cover/901".to_owned(),
                },
            )
            .await;
            let unknown = raw_request(
                conn,
                &Request::Cover {
                    url: "/api/cover/999999".to_owned(),
                },
            )
            .await;
            assert_eq!(unshared, absent, "la pochette privée est « absente »");
            assert_eq!(
                unshared, unknown,
                "pochette privée et inconnue : même trame"
            );

            // 4. `Sets`: the private set is not advertised at all.
            let sets = raw_request(conn, &Request::Sets { page: 0 }).await;
            let text = String::from_utf8_lossy(&sets);
            assert!(
                text.contains("900"),
                "l'ensemble partagé est annoncé : {text}"
            );
            assert!(
                !text.contains("901") && !text.contains("Privee") && !text.contains("t9"),
                "l'ensemble privé ne doit pas apparaître : {text}"
            );

            shared.close().await;
        },
    )
    .await;
}

/// **M10**: a friend's inventory lists the shared tracks and NOTHING else — not
/// their ids, not their qualities, not their lengths.
#[tokio::test]
async fn a_friends_inventory_holds_only_the_shared_tracks() {
    bounded("a_friends_inventory_holds_only_the_shared_tracks", async {
        let shared = Shared::new("inventory").await;
        let frame = raw_request(
            &shared.friend_conn,
            &Request::Inventory {
                since: None,
                page: 0,
            },
        )
        .await;
        let answer: Response = serde_json::from_slice(&frame).expect("une trame Inventory");
        let Response::InventoryFull { items, last, .. } = answer else {
            panic!("une réponse d'inventaire attendue, reçu {answer:?}");
        };
        assert!(last, "deux titres tiennent sur une page");
        let mut tracks: Vec<String> = items.iter().map(|item| item.track.clone()).collect();
        tracks.sort();
        assert_eq!(tracks, vec!["t1".to_owned(), "t2".to_owned()]);
        let text = String::from_utf8_lossy(&frame);
        assert!(
            !text.contains("t9"),
            "l'inventaire d'un ami ne nomme pas un titre non partagé : {text}"
        );
        shared.close().await;
    })
    .await;
}

/// **M12/M13**: a `Fetch` whose `from` is past the end, and a resume that carries
/// another `FileTag`, are the ABSENT frame — the same bytes a title nobody has
/// answers — while a resume of the same file at a real offset is served.
#[tokio::test]
async fn a_fetch_past_the_end_and_a_resume_with_another_tag_are_the_absent_answers() {
    bounded(
        "a_fetch_past_the_end_and_a_resume_with_another_tag_are_the_absent_answers",
        async {
            let shared = Shared::new("resume").await;
            let conn = &shared.friend_conn;
            let fetch = |from: u64, expect: Option<FileTag>| Request::Fetch {
                track: "t1".to_owned(),
                quality: WireQuality::Flac,
                from,
                expect,
                urgency: Urgency::Play,
            };

            let past_the_end = raw_request(conn, &fetch(4, None)).await;
            assert_eq!(past_the_end, absent_bytes(), "from > len est « absent »");

            let other_file = raw_request(
                conn,
                &fetch(
                    1,
                    Some(FileTag {
                        len: 3,
                        sha256: "00".repeat(32),
                    }),
                ),
            )
            .await;
            assert_eq!(
                other_file,
                absent_bytes(),
                "une reprise sur un autre fichier est « absente »"
            );

            // The control: the same file, from a real offset, IS served — the
            // refusal above is the tag and not a fetch that never worked.
            let (head, body) = request(conn, fetch(1, None), 2)
                .await
                .expect("une reprise sans tag sur le même fichier");
            assert!(
                matches!(head, Response::FetchOk { len: 3, .. }),
                "une reprise du même fichier se sert : {head:?}"
            );
            assert_eq!(body, b"ne", "les octets à partir de l'offset");

            shared.close().await;
        },
    )
    .await;
}

// --- 5. M11: a frame over the cap --------------------------------------

/// **M11**: a peer that announces a frame of 1 MiB + 1 is closed with the one
/// refusal code and answered **nothing** — the cap is checked before anything is
/// allocated for the body (the allocation count itself is `tests/transport.rs`'s
/// counting allocator; this is the same property on a real authenticated peer).
#[tokio::test]
async fn a_frame_of_one_mebibyte_plus_one_is_refused_and_answered_nothing() {
    bounded(
        "a_frame_of_one_mebibyte_plus_one_is_refused_and_answered_nothing",
        async {
            let subject = subject_of(0x91);
            let owner = device("cap-owner", subject, (now() - 60, now() + 86_400));
            let endpoint = node(
                Arc::clone(&owner),
                trust(subject, Some(roster(subject, 1, &[])), None),
                "PC",
            );
            let (accepted, client) = tokio::join!(endpoint.accept(), async {
                let client = RawClient::dial(address(&endpoint), &owner).await;
                authenticate_raw(&client, &owner).await;
                client
            });
            let accepted = accepted
                .expect("une connexion")
                .expect("le pair authentifié");
            let server = Arc::new(Server::new(
                Index::new(
                    Fixture::new("cap").cache(),
                    "1".to_owned(),
                    Fixture::new("cap-store").store(),
                ),
                gates(),
                ServeLimits::default(),
            ));
            let task = tokio::spawn(serve(accepted, server));

            // A header that announces one byte over the cap, and not one byte of
            // body: the responder must not wait for it.
            let (mut send, mut recv) = client.conn.open_bi().await.expect("un flux hostile");
            send.write_all(&((codec::MAX_FRAME_BYTES + 1) as u32).to_be_bytes())
                .await
                .expect("un en-tête hors plafond");
            let mut received = Vec::new();
            let mut buffer = [0u8; 1024];
            while let Ok(Some(read)) = recv.read(&mut buffer).await {
                received.extend_from_slice(&buffer[..read]);
            }
            assert!(
                received.is_empty(),
                "une trame hors plafond ne reçoit aucun octet, reçu {}",
                received.len()
            );
            let (code, reason) = close_of(&client.conn).await;
            assert_eq!(code, auth::CLOSE_REFUSED);
            assert_eq!(reason, "");
            task.abort();
            client.shutdown().await;
            endpoint.close().await;
        },
    )
    .await;
}

// --- 6. M16 and M17: gossip of forged or older trust blobs --------------

/// **M16/M17**: a roster whose signature is not the server's, and a revocation
/// list with a LOWER serial than the one the device holds, arrive on a real control
/// stream and change NOTHING — while the next serial of the genuine list is taken.
#[tokio::test]
async fn a_forged_roster_and_an_older_revocation_list_change_nothing() {
    bounded(
        "a_forged_roster_and_an_older_revocation_list_change_nothing",
        async {
            let subject = subject_of(0xa1);
            let owner = device("gossip-owner", subject, (now() - 60, now() + 86_400));
            let second = device("gossip-second", subject, (now() - 60, now() + 86_400));
            let fixture = Fixture::new("gossip");
            let store = fixture.store();
            let held_roster = roster(subject, 7, &[]);
            let held_raw = held_roster.raw_signed();
            let held = trust(subject, Some(held_roster), Some(revocation_list(3, &[])));
            let endpoint = node(Arc::clone(&owner), Arc::clone(&held), "PC");
            let client_endpoint = node(
                Arc::clone(&second),
                trust(subject, Some(roster(subject, 7, &[])), None),
                "Téléphone",
            );

            let (accepted, dialled) = tokio::join!(endpoint.accept(), async {
                client_endpoint.connect(address(&endpoint)).await
            });
            let accepted = accepted
                .expect("une connexion")
                .expect("un appareil du même compte");
            let dialled = dialled.expect("l'appareil s'authentifie");

            // 1. A forged roster: the exact body a real one has, signed by a key
            //    this device does not know.
            let forged = forged_roster_envelope(subject);
            dialled
                .send_control(Control::Roster {
                    blob: base64(&forged),
                })
                .await
                .expect("une trame de gossip");
            let frame = accepted.next_control().await.expect("la trame reçue");
            let taken = gossip::accept(&held, &store, &frame, true);
            assert_eq!(
                taken,
                Err(nz_lan::CredError::Signature),
                "un roster forgé est refusé"
            );
            {
                let state = held.read().expect("trust");
                let roster = state.roster.as_ref().expect("le roster tenu");
                assert_eq!(roster.serial, 7, "le serial n'a pas bougé");
                assert_eq!(roster.raw_signed(), held_raw, "les octets n'ont pas bougé");
            }

            // 2. A revocation list OLDER than the one it holds (serial 2 < 3).
            let older = revocation_bytes(2, &[]);
            dialled
                .send_control(Control::Revocations {
                    blob: base64(&older),
                })
                .await
                .expect("une trame de gossip");
            let frame = accepted.next_control().await.expect("la trame reçue");
            assert_eq!(
                gossip::accept(&held, &store, &frame, true),
                Ok(false),
                "une liste plus ancienne est ignorée"
            );
            assert_eq!(
                held.read()
                    .expect("trust")
                    .revocations
                    .get(&kid())
                    .map(|list| list.serial),
                Some(3),
                "le serial de révocation n'a pas reculé"
            );

            // 3. The control: the NEXT serial of the genuine list is taken.
            let newer = revocation_bytes(4, &[]);
            dialled
                .send_control(Control::Revocations {
                    blob: base64(&newer),
                })
                .await
                .expect("une trame de gossip");
            let frame = accepted.next_control().await.expect("la trame reçue");
            assert_eq!(
                gossip::accept(&held, &store, &frame, true),
                Ok(true),
                "la liste plus récente est prise"
            );
            assert_eq!(
                held.read()
                    .expect("trust")
                    .revocations
                    .get(&kid())
                    .map(|list| list.serial),
                Some(4)
            );

            dialled.close(0);
            endpoint.close().await;
            client_endpoint.close().await;
        },
    )
    .await;
}

// --- 7. M19: a responder that never answers -----------------------------

/// **M19**: a rogue responder that finishes the TLS handshake and never sends
/// `AuthOk` costs the initiator **2 s**, after which it gives up — and it has sent
/// **no request**: the only bytes that left were the one `Auth` frame.
#[tokio::test]
async fn a_responder_that_never_answers_costs_two_seconds_and_no_request() {
    bounded(
        "a_responder_that_never_answers_costs_two_seconds_and_no_request",
        async {
            let subject = subject_of(0xb1);
            let initiator = device("silent-initiator", subject, (now() - 60, now() + 86_400));
            let rogue = device_key("silent-rogue");
            let rogue_identity = Arc::new(Identity {
                cred: Credential::decode(&credential_bytes(
                    &server(),
                    subject,
                    rogue.public(),
                    (now() - 60, now() + 86_400),
                ))
                .expect("une attestation"),
                key: rogue,
            });
            let (server, address) = raw_server(&rogue_identity).await;

            // The rogue node's side: the TLS handshake, the Auth frame, and then
            // SILENCE. It counts the frames it received and any request stream.
            let rogue_task = tokio::spawn(async move {
                let conn = server
                    .accept()
                    .await
                    .expect("une connexion")
                    .await
                    .expect("TLS");
                let (_send, mut recv) = conn.accept_bi().await.expect("le flux d'auth");
                let mut frames = 0;
                let _ = codec::read_frame::<Control>(&mut recv, codec::MAX_FRAME_BYTES).await;
                frames += 1;
                // A request would be a SECOND stream: give the initiator the time of
                // its own deadline to open one.
                let extra = tokio::time::timeout(Duration::from_millis(300), conn.accept_bi())
                    .await
                    .map(|result| result.is_ok())
                    .unwrap_or(false);
                let _ = conn.closed().await;
                server.close(0u32.into(), b"");
                (frames, extra)
            });

            let endpoint = node(
                Arc::clone(&initiator),
                trust(subject, Some(roster(subject, 1, &[])), None),
                "Téléphone",
            );
            let started = Instant::now();
            let outcome = endpoint.connect(address).await;
            let elapsed = started.elapsed();
            assert_eq!(
                outcome.err(),
                Some(LanError::Refused),
                "l'initiateur abandonne"
            );
            assert!(
                elapsed >= Duration::from_millis(1_900),
                "il abandonne après le délai de 2 s, mesuré {elapsed:?}"
            );
            assert!(
                elapsed <= Duration::from_secs(4),
                "et pas plus tard : mesuré {elapsed:?}"
            );
            let (frames, extra_stream) = rogue_task.await.expect("le faux répondeur");
            assert_eq!(frames, 1, "un seul cadre reçu : l'Auth");
            assert!(!extra_stream, "aucune requête n'a été envoyée");
            println!("répondeur muet : abandon à {elapsed:?}, 0 requête");
            endpoint.close().await;
        },
    )
    .await;
}

// --- 8. M20: a stranger that sends half a frame -------------------------

/// **M20**: a stranger that opens the stream and sends a header for 1 000 bytes it
/// never sends is closed at the 2 s deadline having received **0 application
/// bytes** — the same emptiness as any other refusal.
#[tokio::test]
async fn a_stranger_that_sends_half_a_frame_is_answered_nothing() {
    bounded(
        "a_stranger_that_sends_half_a_frame_is_answered_nothing",
        async {
            let subject = subject_of(0xc1);
            let owner = device("half-owner", subject, (now() - 60, now() + 86_400));
            let stranger = device(
                "half-stranger",
                subject_of(0xc2),
                (now() - 60, now() + 86_400),
            );
            let endpoint = node(
                Arc::clone(&owner),
                trust(subject, Some(roster(subject, 1, &[])), None),
                "PC",
            );
            let (accepted, (client, written)) = tokio::join!(endpoint.accept(), async {
                let client = RawClient::dial(address(&endpoint), &stranger).await;
                let (mut send, recv) = client.conn.open_bi().await.expect("un flux");
                send.write_all(&1_000u32.to_be_bytes())
                    .await
                    .expect("un en-tête");
                let written = client.bytes_written(recv).await;
                (client, written)
            });
            assert_eq!(
                written, 0,
                "un inconnu qui n'envoie qu'un en-tête ne reçoit aucun octet"
            );
            assert_eq!(
                accepted.map(|result| result.err()),
                Some(Some(LanError::Refused))
            );
            let (code, _) = close_of(&client.conn).await;
            assert_eq!(code, auth::CLOSE_REFUSED);
            client.shutdown().await;
            endpoint.close().await;
        },
    )
    .await;
}

// --- 9. M22: a friend removed on the server -----------------------------

/// A one-route HTTP server: `GET /api/lan/roster` answers whatever the test last
/// put in it. LN1's route contract, minus the database.
struct RosterServer {
    port: u16,
    body: Arc<Mutex<Vec<u8>>>,
}

impl RosterServer {
    fn start() -> RosterServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("un port loopback");
        let port = listener.local_addr().expect("une adresse").port();
        let body: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::clone(&body);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let answer = Arc::clone(&shared);
                let mut buffer = Vec::new();
                let mut chunk = [0u8; 1024];
                while !buffer.windows(4).any(|window| window == b"\r\n\r\n") {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => buffer.extend_from_slice(&chunk[..read]),
                    }
                }
                let head = String::from_utf8_lossy(&buffer).to_string();
                let payload = if head.starts_with("GET /api/lan/roster") {
                    answer.lock().expect("body").clone()
                } else {
                    json!({ "error": "route inconnue", "code": "not_found" })
                        .to_string()
                        .into_bytes()
                };
                let status = if head.starts_with("GET /api/lan/roster") {
                    "200 OK"
                } else {
                    "404 Not Found"
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    payload.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(&payload);
                let _ = stream.flush();
            }
        });
        RosterServer { port, body }
    }

    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// The answer of `GET /api/lan/roster`: the roster envelope, an empty (but
    /// signed) revocation list, and an ETag that moves with the serial.
    fn serve_roster(&self, subject: Subject, serial: u64, shares: &[Share]) {
        let envelope = roster_envelope(subject, serial, shares, &[]);
        let roster: Value = serde_json::from_slice(&envelope).expect("l'enveloppe");
        let answer = json!({
            "roster": roster,
            "revocations": base64(&revocation_bytes(1, &[])),
            "etag": format!("\"{serial}-1\""),
        });
        *self.body.lock().expect("body") =
            serde_json::to_vec(&answer).expect("la réponse du serveur");
    }
}

/// **M22**: a friend the server removed is refused — by the owner's device after
/// its next roster sync, AND by a second device of the owner that only met the
/// first one (the roster gossiped between two devices of the same person).
///
/// The live connection the friend opened BEFORE the removal keeps serving until it
/// closes: that is the documented residual (authorisation is decided at the
/// handshake). The next connection is refused, with 0 bytes.
#[tokio::test]
async fn a_removed_friend_is_refused_after_the_sync_and_through_gossip() {
    bounded(
        "a_removed_friend_is_refused_after_the_sync_and_through_gossip",
        async {
            let owner_subject = subject_of(0xd1);
            let friend_subject = subject_of(0xd2);
            let pc = device("removed-pc", owner_subject, (now() - 60, now() + 86_400));
            let laptop = device(
                "removed-laptop",
                owner_subject,
                (now() - 60, now() + 86_400),
            );
            let friend = device(
                "removed-friend",
                friend_subject,
                (now() - 60, now() + 86_400),
            );
            let share = [Share {
                subject: friend_subject,
                tracks: vec!["t1"],
            }];

            let pc_state = trust(owner_subject, Some(roster(owner_subject, 1, &share)), None);
            let laptop_state = trust(owner_subject, Some(roster(owner_subject, 1, &share)), None);
            let friend_state = trust(
                friend_subject,
                Some(roster_from(&roster_envelope(
                    friend_subject,
                    1,
                    &[],
                    &[(owner_subject, "PC")],
                ))),
                None,
            );
            let pc_endpoint = node(Arc::clone(&pc), Arc::clone(&pc_state), "PC");
            let laptop_endpoint = node(Arc::clone(&laptop), Arc::clone(&laptop_state), "Portable");
            let friend_endpoint = node(Arc::clone(&friend), Arc::clone(&friend_state), "Téléphone");

            // 1. Before the removal: the friend is welcome on the LAPTOP too (the
            //    control that makes the refusal below the roster and not a fixture
            //    that never worked).
            let (accepted, dialled) = tokio::join!(laptop_endpoint.accept(), async {
                friend_endpoint.connect(address(&laptop_endpoint)).await
            });
            assert!(
                accepted.is_some_and(|result| result.is_ok()),
                "l'ami est accepté avant le retrait"
            );
            dialled.expect("le téléphone s'authentifie").close(0);

            // 2. The friend's live connection to the PC, THEN the server removes the
            //    share: the PC's next roster sync (one conditional GET) makes the
            //    new roster the one its endpoint trusts.
            let (accepted, dialled) = tokio::join!(pc_endpoint.accept(), async {
                friend_endpoint.connect(address(&pc_endpoint)).await
            });
            let accepted = accepted
                .expect("une connexion")
                .expect("l'ami est accepté avant le retrait");
            let live = dialled.expect("le téléphone s'authentifie");

            let server = RosterServer::start();
            server.serve_roster(owner_subject, 2, &[]);
            let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ln15-sec-m22-api");
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("api dir");
            let api = Api::builder(server.base())
                .session(Arc::new(SessionJar::at(dir.join("session.json"))))
                .build()
                .expect("un client");
            let store = Fixture::new("m22-store").store();
            let synced = nz_lan::enroll::sync_trust(&api, &store, &pc_state)
                .await
                .expect("la synchronisation du roster");
            assert!(synced, "le roster du serveur a changé");
            assert_eq!(
                pc_state
                    .read()
                    .expect("trust")
                    .roster
                    .as_ref()
                    .map(|roster| roster.serial),
                Some(2),
                "le PC tient le nouveau roster"
            );
            // The residual, stated: the connection opened BEFORE the removal keeps
            // its authorisation (it was decided at the handshake).
            assert!(
                matches!(accepted.info().relation, nz_lan::Relation::Friend { .. }),
                "la connexion en cours garde son autorisation"
            );
            live.close(0);
            accepted.close(0);

            // 3. The next connection is refused, with 0 bytes.
            let (accepted, (client, written)) = tokio::join!(pc_endpoint.accept(), async {
                let client = RawClient::dial(address(&pc_endpoint), &friend).await;
                let exporter = auth::exporter(&client.conn, Role::Initiator).expect("l'exporteur");
                let proof = friend.key.sign(&auth_message(Role::Initiator, &exporter));
                let frame = auth_control(&friend.cred, &proof, 1, 0);
                let (_send, recv) = client.send(&frame).await;
                let written = client.bytes_written(recv).await;
                (client, written)
            });
            assert_eq!(written, 0, "l'ami retiré ne reçoit aucun octet");
            assert_eq!(
                accepted.map(|result| result.err()),
                Some(Some(LanError::Refused)),
                "le PC refuse l'ami retiré"
            );
            client.shutdown().await;

            // 4. The GOSSIP half: the laptop only ever met the PC. The PC offers its
            //    new roster on the control stream; the laptop takes it, and then
            //    refuses the friend ITSELF.
            let (accepted, dialled) = tokio::join!(laptop_endpoint.accept(), async {
                pc_endpoint.connect(address(&laptop_endpoint)).await
            });
            let accepted = accepted
                .expect("une connexion")
                .expect("deux appareils du même compte");
            let dialled = dialled.expect("le PC s'authentifie");
            let pc_roster = pc_state
                .read()
                .expect("trust")
                .roster
                .as_ref()
                .expect("le roster du PC")
                .raw_signed();
            dialled
                .send_control(Control::Roster {
                    blob: base64(&pc_roster),
                })
                .await
                .expect("le gossip du roster");
            let frame = accepted.next_control().await.expect("la trame reçue");
            let laptop_store = Fixture::new("m22-laptop-store").store();
            assert_eq!(
                gossip::accept(&laptop_state, &laptop_store, &frame, true),
                Ok(true),
                "le portable prend le roster de son autre appareil"
            );
            assert_eq!(
                laptop_state
                    .read()
                    .expect("trust")
                    .roster
                    .as_ref()
                    .map(|roster| roster.serial),
                Some(2)
            );
            let (accepted, (client, written)) = tokio::join!(laptop_endpoint.accept(), async {
                let client = RawClient::dial(address(&laptop_endpoint), &friend).await;
                let exporter = auth::exporter(&client.conn, Role::Initiator).expect("l'exporteur");
                let proof = friend.key.sign(&auth_message(Role::Initiator, &exporter));
                let frame = auth_control(&friend.cred, &proof, 1, 0);
                let (_send, recv) = client.send(&frame).await;
                let written = client.bytes_written(recv).await;
                (client, written)
            });
            assert_eq!(
                written, 0,
                "un appareil qui n'a vu que son pair refuse aussi l'ami retiré"
            );
            assert_eq!(
                accepted.map(|result| result.err()),
                Some(Some(LanError::Refused)),
                "le portable refuse l'ami retiré après le gossip"
            );
            client.shutdown().await;
            dialled.close(0);

            pc_endpoint.close().await;
            laptop_endpoint.close().await;
            friend_endpoint.close().await;
        },
    )
    .await;
}
