//! LN6's acceptance, one test per line of the note's *Acceptance* list, against
//! two REAL nodes on loopback (docs/agents/lan-nodes.md, *Transport and
//! authentication* and *Two nodes on one machine*, rung 1).
//!
//! Every property here is a number or an event, never a wait:
//!
//! | Property | Measured by |
//! |---|---|
//! | own↔own authenticates and names itself | `own_devices_authenticate_and_exchange_hello` |
//! | a friend is authorised only by `shares_to` | `a_friend_is_authorised_by_the_roster_that_shares_to_them` |
//! | the initiator drops an untrusted responder | `the_initiator_drops_a_responder_its_roster_does_not_trust` |
//! | a stranger learns **0 bytes** | `a_stranger_gets_no_identity_byte` |
//! | every refusal is ONE code | `every_refusal_is_the_same_code_and_no_identity_byte` |
//! | a wrong major is told apart (`0x11`) | `a_wrong_protocol_major_is_told_apart` |
//! | the responder closes after 2 s (paused clock) | `the_responder_closes_two_seconds_without_a_valid_auth` |
//! | … or after 2 KiB | `the_responder_refuses_more_than_two_kib_before_auth` |
//! | 0-RTT never accepted | `early_data_is_never_accepted` |
//! | TLS 1.2 refused | `tls12_cannot_establish_a_session` |
//! | duplicates collapse to the lower key's dial | `a_duplicate_pair_collapses_to_the_lower_key_dial` |
//! | 1 MiB + 1 closes, allocates ≤ 1 MiB | `an_over_cap_frame_allocates_nothing_and_stays_under_a_mebibyte` |
//! | handshake + both proofs p95 ≤ 15 ms (release) | `the_handshake_and_both_proofs_are_fast` (prints p50/p95) |
//! | `close`/drop join every task | `closing_the_endpoint_joins_the_only_task` |
//! | a busy port falls back | `a_busy_preferred_port_falls_back_to_an_ephemeral_one` |
//! | frames, bodies and control | `a_request_and_its_body_cross_a_stream`, `the_control_stream_carries_gossip_and_ends_cleanly` |
//! | rule 7 on the wire | `an_unknown_frame_is_answered_and_the_connection_lives` |
//! | the 16-connection cap | `the_connection_cap_is_sixteen` |
//!
//! The nodes are real `nz_lan::transport::Endpoint`s on `127.0.0.1`, each with its
//! own device key written under `CARGO_TARGET_TMPDIR` (never `/tmp`: tmpfs is
//! RAM), its own credential signed here by a test "server" key, and its own trust
//! state. The hostile cases speak raw quinn with the crate's own TLS
//! configuration, because that is exactly what an attacker on the Wi-Fi has: TLS is
//! not the identity, and a test must go through it to reach the proof exchange.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use aws_lc_rs::digest::{self, SHA256};
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};
use nz_lan::auth;
use nz_lan::codec::{self, MAX_FRAME_BYTES};
use nz_lan::cred::{Credential, Revocations, Role, Roster, TrustKeys, auth_message};
use nz_lan::keys::DeviceKey;
use nz_lan::proto::{self, Control, Request, Response};
use nz_lan::scope::Scope;
use nz_lan::transport::{
    self, DEFAULT_PORT, Endpoint, Identity, PeerConn, SharedTrust, TrustState,
};
use nz_lan::types::{Kid, LanError, Relation, Subject};

// --- The counting allocator (this file's own, thread-scoped) ------------------
//
// The "allocates ≤ 1 MiB" line cannot be an opinion: a `GlobalAlloc` counts what
// the tested call really asks for, on THIS thread only, so the parallel tests of
// this binary do not pollute the number. `try_with` keeps a destroyed thread-local
// from panicking inside the allocator, which is the one subtlety of an otherwise
// trivial wrapper.

struct CountingAllocator;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static COUNTED: Cell<usize> = const { Cell::new(0) };
}

fn note(size: usize) {
    let _ = COUNTING.try_with(|counting| {
        if counting.get() {
            let _ = COUNTED.try_with(|counted| counted.set(counted.get() + size));
        }
    });
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        note(size);
        unsafe { System.realloc(pointer, layout, size) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// Runs `body` with the counter on, and answers the bytes it allocated.
fn counted<R>(body: impl FnOnce() -> R) -> (R, usize) {
    let before = COUNTED.with(Cell::get);
    COUNTING.with(|counting| counting.set(true));
    let value = body();
    COUNTING.with(|counting| counting.set(false));
    let after = COUNTED.with(Cell::get);
    (value, after.saturating_sub(before))
}

/// The same, around an await point: on a current-thread runtime the node's own
/// tasks run on this thread, so the gate covers exactly the exchange being measured.
async fn counted_async<F: std::future::Future>(future: F) -> (F::Output, usize) {
    let before = COUNTED.with(Cell::get);
    COUNTING.with(|counting| counting.set(true));
    let value = future.await;
    COUNTING.with(|counting| counting.set(false));
    let after = COUNTED.with(Cell::get);
    (value, after.saturating_sub(before))
}

// --- A test server, and the devices it enrolled -------------------------------

/// The test "server" LAN key: every credential, revocation list and roster below is
/// signed by it, exactly as `neezer/lan.py` signs them for real.
const SERVER_SEED: [u8; 32] = [0x5a; 32];

fn server() -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&SERVER_SEED).expect("la clé LAN de test")
}

fn server_public() -> [u8; 32] {
    let mut public = [0u8; 32];
    public.copy_from_slice(server().public_key().as_ref());
    public
}

/// `kid = SHA-256(public key)[0..8]`, the note's definition.
fn kid() -> Kid {
    let digest = digest::digest(&SHA256, &server_public());
    let mut kid = [0u8; 8];
    kid.copy_from_slice(&digest.as_ref()[..8]);
    Kid(kid)
}

fn lan_keys() -> TrustKeys {
    TrustKeys::from_status(&[(kid(), server_public())])
}

fn sign(domain: &[u8], message: &[u8]) -> Vec<u8> {
    let mut signed = domain.to_vec();
    signed.extend_from_slice(message);
    server().sign(&signed).as_ref().to_vec()
}

fn subject_of(value: u8) -> Subject {
    Subject([value; 16])
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

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or(0)
}

/// The note's credential layout, 154 bytes, signed by the test server.
fn credential_for(
    subject: Subject,
    device_pub: [u8; 32],
    not_before: i64,
    not_after: i64,
    flags: u8,
) -> Credential {
    let mut body = Vec::with_capacity(90);
    body.extend_from_slice(b"NZLC");
    body.push(1);
    body.extend_from_slice(&kid().0);
    body.extend_from_slice(&subject.0);
    body.extend_from_slice(&[0x01; 12]);
    body.extend_from_slice(&device_pub);
    body.extend_from_slice(&not_before.to_be_bytes());
    body.extend_from_slice(&not_after.to_be_bytes());
    body.push(flags);
    assert_eq!(body.len(), 90, "le corps de l'attestation fait 90 octets");
    let mut signed = body;
    let signature = sign(b"neezer-lan/1 credential\0", &signed.clone());
    signed.extend_from_slice(&signature);
    assert_eq!(signed.len(), 154);
    Credential::decode(&signed).expect("une attestation de test valide")
}

/// A device key written by the real key path (`0600`, atomic), under the crate's own
/// `target/`, never `/tmp`.
fn device_key(name: &str) -> DeviceKey {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("ln6-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("un répertoire de test");
    DeviceKey::load_or_create(&dir).expect("une clé d'appareil")
}

/// One enrolled device: its key, its credential, and the identity a node is built
/// from.
fn device(name: &str, subject: Subject, window: (i64, i64)) -> Arc<Identity> {
    let key = device_key(name);
    let cred = credential_for(subject, key.public(), window.0, window.1, 0b11);
    Arc::new(Identity { key, cred })
}

/// A device whose public key is known to be on one side of `0x80`, so a test can
/// decide which of two keys is the lower one (the duplicate rule is stated in terms
/// of that order).
fn device_with_rank(prefix: &str, subject: Subject, lower: bool) -> Arc<Identity> {
    for attempt in 0..64 {
        let key = device_key(&format!("{prefix}-{attempt}"));
        let public = key.public();
        if (public[0] < 0x80) == lower {
            let cred = credential_for(subject, public, now() - 60, now() + 86_400, 0b11);
            return Arc::new(Identity { key, cred });
        }
    }
    panic!("aucune clé du rang demandé en 64 essais");
}

/// The empty revocation list of the test server key.
fn no_revocations() -> Revocations {
    Revocations::empty(kid())
}

/// A signed revocation list naming `keys`.
fn revocations(serial: u64, keys: &[[u8; 32]]) -> Revocations {
    let mut body = Vec::new();
    body.extend_from_slice(b"NZLR");
    body.push(1);
    body.extend_from_slice(&kid().0);
    body.extend_from_slice(&serial.to_be_bytes());
    body.extend_from_slice(&now().to_be_bytes());
    body.extend_from_slice(&(keys.len() as u32).to_be_bytes());
    for key in keys {
        body.extend_from_slice(key);
    }
    let mut signed = body;
    let signature = sign(b"neezer-lan/1 revocations\0", &signed.clone());
    signed.extend_from_slice(&signature);
    Revocations::decode_verify(&signed, &lan_keys()).expect("une liste de révocation valide")
}

/// One share of a roster body.
struct Share {
    subject: Subject,
    all: bool,
    tracks: Vec<&'static str>,
}

/// A signed roster: `shares_to` (what I give) and `trusted_by` (who gives to me).
fn roster(subject: Subject, shares_to: &[Share], trusted_by: &[(Subject, &str)]) -> Roster {
    let shares = shares_to
        .iter()
        .map(|share| {
            let playlists = if share.all {
                "[]".to_owned()
            } else {
                let tracks: Vec<String> = share
                    .tracks
                    .iter()
                    .map(|track| format!("\"{track}\""))
                    .collect();
                format!(
                    r#"[{{"id":"p1","title":"Route","tracks":[{}]}}]"#,
                    tracks.join(",")
                )
            };
            format!(
                r#"{{"subject":"{}","all":{},"playlists":{}}}"#,
                to_hex(&share.subject.0),
                share.all,
                playlists
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
        r#"{{"v":1,"kid":"{}","subject":"{}","serial":42,"issued":{},"expires":{},"shares_to":[{}],"trusted_by":[{}]}}"#,
        to_hex(&kid().0),
        to_hex(&subject.0),
        now() - 60,
        now() + 86_400,
        shares,
        trusted
    );
    let signature = sign(b"neezer-lan/1 roster\0", body.as_bytes());
    let envelope = format!(
        r#"{{"body":"{}","sig":"{}"}}"#,
        base64(body.as_bytes()),
        base64(&signature)
    );
    Roster::decode_verify(envelope.as_bytes(), &lan_keys()).expect("un roster signé valide")
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

/// Answers every request on `conn` with `Pong`, until the peer closes. The tests that
/// need an answer spawn this; the crate itself never spawns a task.
fn answer_pings(conn: PeerConn) -> tokio::task::JoinHandle<usize> {
    tokio::spawn(async move {
        let mut answered = 0;
        while let Some(incoming) = conn.next_incoming().await {
            incoming
                .respond(Response::Pong, None)
                .await
                .expect("une réponse Pong");
            answered += 1;
        }
        answered
    })
}

/// A generous bound around a test's body: a deadlock must FAIL, with its name, rather
/// than hold the test harness for ever (it is not the assertion, it is the net under
/// it). Sixty seconds is orders of magnitude above what any of these should take: the
/// whole file, 21 tests, runs in about five.
async fn bounded<F: std::future::Future>(name: &str, body: F) -> F::Output {
    match tokio::time::timeout(Duration::from_secs(60), body).await {
        Ok(value) => value,
        Err(_) => {
            panic!("{name} : la borne de 60 s est atteinte, la fermeture pend (interblocage ?)")
        }
    }
}

// --- 1. Own devices ----------------------------------------------------------

/// Two devices of one person authenticate, each verifies the other's proof over THIS
/// connection, both exchange `Hello`, and a request crosses on its own stream.
#[tokio::test]
async fn own_devices_authenticate_and_exchange_hello() {
    bounded("own_devices_authenticate_and_exchange_hello", async {
        let subject = subject_of(0x11);
        let a = device("own-a", subject, (now() - 60, now() + 86_400));
        let b = device("own-b", subject, (now() - 60, now() + 86_400));
        let endpoint_a = node(Arc::clone(&a), trust(subject, None, None), "PC");
        let endpoint_b = node(Arc::clone(&b), trust(subject, None, None), "Téléphone");

        let (client, accepted) = tokio::join!(
            endpoint_a.connect(address(&endpoint_b)),
            endpoint_b.accept()
        );
        let client = client.expect("l'initiateur s'authentifie");
        let accepted = accepted
            .expect("le répondeur accepte")
            .expect("le répondeur s'authentifie");

        // Both sides are the same subject, and each learned the other's key from the
        // credential it verified — never from the TLS certificate.
        assert_eq!(client.info().subject, subject);
        assert_eq!(accepted.info().subject, subject);
        assert_eq!(client.info().device_pub, b.key.public());
        assert_eq!(accepted.info().device_pub, a.key.public());
        assert_eq!(client.info().relation, Relation::Own);
        assert_eq!(accepted.info().relation, Relation::Own);
        assert_eq!(client.info().scope, Scope::Own);
        assert_eq!(accepted.info().scope, Scope::Own);

        // Names crossed inside the authenticated connection, and only there.
        assert_eq!(
            client
                .info()
                .hello
                .as_ref()
                .map(|hello| hello.name.as_str()),
            Some("Téléphone")
        );
        assert_eq!(
            accepted
                .info()
                .hello
                .as_ref()
                .map(|hello| hello.name.as_str()),
            Some("PC")
        );

        // The dialler knows it dialled; the responder knows it was dialled.
        assert!(client.dialled_by_us());
        assert!(!accepted.dialled_by_us());
        assert_eq!(endpoint_a.connections(), 1);
        assert_eq!(endpoint_b.connections(), 1);

        // One request, one stream, one answer.
        let answering = answer_pings(accepted);
        let reply = client.request(Request::Ping).await.expect("un Ping");
        assert_eq!(reply.head, Response::Pong);
        assert!(reply.body.is_none());

        client.close(0);
        assert_eq!(
            endpoint_a.connections(),
            0,
            "une connexion fermée est oubliée"
        );
        endpoint_a.close().await;
        endpoint_b.close().await;
        let _ = tokio::time::timeout(Duration::from_secs(5), answering).await;
    })
    .await;
}

// --- 2. Friends --------------------------------------------------------------

/// A friend is served exactly when MY roster has a `shares_to` entry for them, with
/// exactly the tracks that entry names; and the initiator's own authorisation reads
/// `trusted_by`, so a friend who shares nothing may serve nothing.
#[tokio::test]
async fn a_friend_is_authorised_by_the_roster_that_shares_to_them() {
    bounded(
        "a_friend_is_authorised_by_the_roster_that_shares_to_them",
        async {
            let owner_subject = subject_of(0x21);
            let friend_subject = subject_of(0x22);
            let owner = device("friend-owner", owner_subject, (now() - 60, now() + 86_400));
            let friend = device("friend-guest", friend_subject, (now() - 60, now() + 86_400));

            let owner_roster = roster(
                owner_subject,
                &[Share {
                    subject: friend_subject,
                    all: false,
                    tracks: vec!["3135556", "3135557"],
                }],
                &[(friend_subject, "Camille")],
            );
            let friend_roster = roster(friend_subject, &[], &[(owner_subject, "Alex")]);

            let endpoint_owner = node(
                Arc::clone(&owner),
                trust(owner_subject, Some(owner_roster), None),
                "PC",
            );
            let endpoint_friend = node(
                Arc::clone(&friend),
                trust(friend_subject, Some(friend_roster), None),
                "Téléphone",
            );

            let (client, accepted) = tokio::join!(
                endpoint_friend.connect(address(&endpoint_owner)),
                endpoint_owner.accept()
            );
            let client = client.expect("l'ami s'authentifie auprès du propriétaire");
            let accepted = accepted
                .expect("le propriétaire accepte")
                .expect("le propriétaire s'authentifie");

            // The owner serves the two shared tracks and nothing else.
            match &accepted.info().scope {
                Scope::Friend { all, .. } => {
                    assert!(!all, "un partage de playlist n'est pas « tout »");
                    assert_eq!(accepted.info().scope.shared_tracks(), 2);
                    assert!(accepted.info().scope.allows("3135556"));
                    assert!(accepted.info().scope.allows("3135557"));
                    assert!(!accepted.info().scope.allows("999"));
                }
                other => panic!("le propriétaire devait voir un ami, pas {other:?}"),
            }
            assert_eq!(
                accepted.info().relation,
                Relation::Friend {
                    name: "Camille".to_owned()
                }
            );

            // The friend may ask (its roster lists the owner in `trusted_by`), and it shares
            // nothing back: it may serve nothing, and every request it receives is answered
            // like an absent file rather than with a refusal.
            assert_eq!(
                client.info().relation,
                Relation::Friend {
                    name: "Alex".to_owned()
                }
            );
            assert_eq!(client.info().scope.shared_tracks(), 0);
            assert!(!client.info().scope.allows("3135556"));

            endpoint_friend.close().await;
            endpoint_owner.close().await;
        },
    )
    .await;
}

/// The owner's roster does not share with the friend: the owner closes before
/// writing a byte, and the connection never exists.
#[tokio::test]
async fn a_roster_that_does_not_share_refuses_the_connection() {
    bounded(
        "a_roster_that_does_not_share_refuses_the_connection",
        async {
            let owner_subject = subject_of(0x23);
            let friend_subject = subject_of(0x24);
            let owner = device("noshare-owner", owner_subject, (now() - 60, now() + 86_400));
            let friend = device(
                "noshare-guest",
                friend_subject,
                (now() - 60, now() + 86_400),
            );
            let owner_roster = roster(owner_subject, &[], &[(friend_subject, "Camille")]);
            let friend_roster = roster(friend_subject, &[], &[(owner_subject, "Alex")]);

            let endpoint_owner = node(
                Arc::clone(&owner),
                trust(owner_subject, Some(owner_roster), None),
                "PC",
            );
            let endpoint_friend = node(
                Arc::clone(&friend),
                trust(friend_subject, Some(friend_roster), None),
                "Téléphone",
            );

            let (client, accepted) = tokio::join!(
                endpoint_friend.connect(address(&endpoint_owner)),
                endpoint_owner.accept()
            );
            assert_eq!(
                client.err(),
                Some(LanError::Refused),
                "l'initiateur est refusé"
            );
            match accepted {
                Some(Err(LanError::Refused)) => {}
                other => panic!("le répondeur devait refuser, pas {other:?}"),
            }
            assert_eq!(endpoint_owner.connections(), 0);

            endpoint_friend.close().await;
            endpoint_owner.close().await;
        },
    )
    .await;
}

/// The initiator refuses a responder its own roster does not list in `trusted_by`,
/// even though the responder would happily have served it: the two directions are
/// authorised separately.
#[tokio::test]
async fn the_initiator_drops_a_responder_its_roster_does_not_trust() {
    bounded(
        "the_initiator_drops_a_responder_its_roster_does_not_trust",
        async {
            let owner_subject = subject_of(0x25);
            let friend_subject = subject_of(0x26);
            let owner = device(
                "untrusted-owner",
                owner_subject,
                (now() - 60, now() + 86_400),
            );
            let friend = device(
                "untrusted-guest",
                friend_subject,
                (now() - 60, now() + 86_400),
            );
            // The owner shares with the friend, but the friend's roster does not list the
            // owner in `trusted_by`: the friend did not consent to asking it.
            let owner_roster = roster(
                owner_subject,
                &[Share {
                    subject: friend_subject,
                    all: true,
                    tracks: Vec::new(),
                }],
                &[],
            );
            let friend_roster = roster(friend_subject, &[], &[]);

            let endpoint_owner = node(
                Arc::clone(&owner),
                trust(owner_subject, Some(owner_roster), None),
                "PC",
            );
            let endpoint_friend = node(
                Arc::clone(&friend),
                trust(friend_subject, Some(friend_roster), None),
                "Téléphone",
            );

            let (client, _accepted) = tokio::join!(
                endpoint_friend.connect(address(&endpoint_owner)),
                endpoint_owner.accept()
            );
            assert_eq!(
                client.err(),
                Some(LanError::Refused),
                "l'initiateur jette un répondeur que son roster ne lui confie pas"
            );

            endpoint_friend.close().await;
            endpoint_owner.close().await;
        },
    )
    .await;
}

// --- 3. A stranger learns nothing, and every refusal is one code --------------

/// A framed `Auth` the hostile client sends, with exactly the bytes the test chose.
fn auth_control(cred: &Credential, proof: &[u8], proto: u32, serial: u64) -> Control {
    Control::Auth {
        cred: base64(cred.raw()),
        proof: base64(proof),
        revocation_serial: serial,
        proto,
    }
}

/// A raw quinn client: the TLS the crate builds, and nothing of the crate's
/// handshake. What an attacker on the Wi-Fi has once the TLS layer is passed.
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

    /// Opens the control stream and sends one frame on it.
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

/// A stranger with a perfectly valid credential and a valid proof over THIS
/// connection, whose subject is neither mine nor in my roster: the responder writes
/// **0 bytes** and closes with the one refusal code.
#[tokio::test]
async fn a_stranger_gets_no_identity_byte() {
    bounded("a_stranger_gets_no_identity_byte", async {
        let owner_subject = subject_of(0x31);
        let stranger_subject = subject_of(0x32);
        let owner = device(
            "stranger-owner",
            owner_subject,
            (now() - 60, now() + 86_400),
        );
        let stranger = device(
            "stranger-guest",
            stranger_subject,
            (now() - 60, now() + 86_400),
        );
        let owner_roster = roster(owner_subject, &[], &[]);
        let endpoint = node(
            Arc::clone(&owner),
            trust(owner_subject, Some(owner_roster), None),
            "PC",
        );

        // The dial and the accept run together: the responder only progresses a
        // handshake while its `accept()` is polled.
        let (accepted, (client, written)) = tokio::join!(endpoint.accept(), async {
            let client = RawClient::dial(address(&endpoint), &stranger).await;
            let exporter = auth::exporter(&client.conn, Role::Initiator).expect("l'exporteur");
            let proof = stranger.key.sign(&auth_message(Role::Initiator, &exporter));
            let frame = auth_control(&stranger.cred, &proof, 1, 0);
            let (_send, recv) = client.send(&frame).await;
            let written = client.bytes_written(recv).await;
            (client, written)
        });
        assert_eq!(written, 0, "un inconnu ne reçoit aucun octet d'identité");
        assert_eq!(
            accepted.map(|result| result.err()),
            Some(Some(LanError::Refused)),
            "le répondeur refuse"
        );
        let (code, reason) = close_of(&client.conn).await;
        assert_eq!(code, auth::CLOSE_REFUSED);
        assert_eq!(reason, "", "la raison d'un refus est vide");

        client.shutdown().await;
        endpoint.close().await;
    })
    .await;
}

/// The exporter of a DIFFERENT session, both ends raw: what an attacker records on
/// one connection and replays on another.
async fn exporter_of_another_session(id: &Identity) -> [u8; 32] {
    let (server, address) = raw_server(id).await;
    let accepting = tokio::spawn(async move {
        let conn = server
            .accept()
            .await
            .expect("une connexion")
            .await
            .expect("TLS");
        let _ = conn.closed().await;
        server
    });
    let client = RawClient::dial(address, id).await;
    let exporter = auth::exporter(&client.conn, Role::Initiator).expect("l'exporteur");
    client.shutdown().await;
    let _ = tokio::time::timeout(Duration::from_secs(5), accepting).await;
    exporter
}

/// Unknown `kid`, expired credential, revoked device, a proof signed over ANOTHER
/// connection's exporter (a replay), and a responder proof replayed as an
/// initiator's (a reflection): five causes, **one** close code, **zero** bytes.
#[tokio::test]
async fn every_refusal_is_the_same_code_and_no_identity_byte() {
    bounded(
        "every_refusal_is_the_same_code_and_no_identity_byte",
        async {
            let owner_subject = subject_of(0x41);
            let owner = device("refusal-owner", owner_subject, (now() - 60, now() + 86_400));
            let expired = device(
                "refusal-expired",
                owner_subject,
                (now() - 172_800, now() - 86_400),
            );
            let revoked_device = device(
                "refusal-revoked",
                owner_subject,
                (now() - 60, now() + 86_400),
            );
            let valid = device("refusal-valid", owner_subject, (now() - 60, now() + 86_400));
            // Signed by a server key this node does not know: the credential is intact and
            // was issued by somebody else.
            let unknown_kid = {
                let key = device_key("refusal-unknown-kid");
                let mut body = Vec::new();
                body.extend_from_slice(b"NZLC");
                body.push(1);
                body.extend_from_slice(&[0xEE; 8]);
                body.extend_from_slice(&owner_subject.0);
                body.extend_from_slice(&[0x01; 12]);
                body.extend_from_slice(&key.public());
                body.extend_from_slice(&(now() - 60).to_be_bytes());
                body.extend_from_slice(&(now() + 86_400).to_be_bytes());
                body.push(0b11);
                let mut signed = body;
                let signature = sign(b"neezer-lan/1 credential\0", &signed.clone());
                signed.extend_from_slice(&signature);
                let cred = Credential::decode(&signed).expect("une attestation bien formée");
                Arc::new(Identity { key, cred })
            };
            let revoked_list = revocations(9, &[revoked_device.key.public()]);

            struct Case {
                name: &'static str,
                device: Arc<Identity>,
                trust: SharedTrust,
            }
            let cases = vec![
                Case {
                    name: "kid inconnu",
                    device: unknown_kid,
                    trust: trust(owner_subject, Some(roster(owner_subject, &[], &[])), None),
                },
                Case {
                    name: "attestation expirée",
                    device: expired,
                    trust: trust(owner_subject, Some(roster(owner_subject, &[], &[])), None),
                },
                Case {
                    name: "appareil révoqué",
                    device: revoked_device,
                    trust: trust(
                        owner_subject,
                        Some(roster(owner_subject, &[], &[])),
                        Some(revoked_list),
                    ),
                },
                Case {
                    name: "preuve rejouée d'une autre connexion",
                    device: Arc::clone(&valid),
                    trust: trust(owner_subject, Some(roster(owner_subject, &[], &[])), None),
                },
                Case {
                    name: "preuve de répondeur rejouée en initiateur",
                    device: Arc::clone(&valid),
                    trust: trust(owner_subject, Some(roster(owner_subject, &[], &[])), None),
                },
            ];

            for case in cases {
                let endpoint = node(Arc::clone(&owner), case.trust, "PC");
                let name = case.name;
                let (accepted, (client, written)) = tokio::join!(endpoint.accept(), async {
                    let client = RawClient::dial(address(&endpoint), &case.device).await;
                    let frame = match name {
                        // A proof over a DIFFERENT connection's exporter: recorded on one
                        // session, useless on the next.
                        "preuve rejouée d'une autre connexion" => {
                            let other = exporter_of_another_session(&case.device).await;
                            let proof =
                                case.device.key.sign(&auth_message(Role::Initiator, &other));
                            auth_control(&case.device.cred, &proof, 1, 0)
                        }
                        // The role in the signed message and the context of the exporter are
                        // both the responder's, and the responder checks the initiator's.
                        "preuve de répondeur rejouée en initiateur" => {
                            let other =
                                auth::exporter(&client.conn, Role::Responder).expect("l'exporteur");
                            let proof =
                                case.device.key.sign(&auth_message(Role::Responder, &other));
                            auth_control(&case.device.cred, &proof, 1, 0)
                        }
                        _ => {
                            let mine =
                                auth::exporter(&client.conn, Role::Initiator).expect("l'exporteur");
                            let proof = case.device.key.sign(&auth_message(Role::Initiator, &mine));
                            auth_control(&case.device.cred, &proof, 1, 0)
                        }
                    };
                    let (_send, recv) = client.send(&frame).await;
                    let written = client.bytes_written(recv).await;
                    (client, written)
                });
                assert_eq!(written, 0, "{name} : aucune fuite d'identité");
                match accepted {
                    Some(Err(LanError::Refused)) => {}
                    other => panic!("{name} : le répondeur devait refuser, pas {other:?}"),
                }
                let (code, reason) = close_of(&client.conn).await;
                assert_eq!(code, auth::CLOSE_REFUSED, "{name} : un seul code");
                assert_eq!(reason, "", "{name} : aucune raison sur le fil");
                client.shutdown().await;
                endpoint.close().await;
            }
        },
    )
    .await;
}

/// A protocol major this build cannot speak is the ONE refusal with its own code: a
/// version mismatch, not a security event — the screens say « mettez à jour l'app ».
#[tokio::test]
async fn a_wrong_protocol_major_is_told_apart() {
    bounded("a_wrong_protocol_major_is_told_apart", async {
        let subject = subject_of(0x51);
        let mine = device("major-own", subject, (now() - 60, now() + 86_400));
        let endpoint = node(Arc::clone(&mine), trust(subject, None, None), "PC");

        let (accepted, (client, written)) = tokio::join!(endpoint.accept(), async {
            let client = RawClient::dial(address(&endpoint), &mine).await;
            let exporter = auth::exporter(&client.conn, Role::Initiator).expect("l'exporteur");
            let proof = mine.key.sign(&auth_message(Role::Initiator, &exporter));
            let frame = auth_control(&mine.cred, &proof, 2, 0);
            let (_send, recv) = client.send(&frame).await;
            let written = client.bytes_written(recv).await;
            (client, written)
        });
        assert_eq!(
            written, 0,
            "le contrôle de version ne coûte aucun octet d'identité"
        );
        assert!(
            matches!(accepted, Some(Err(LanError::Proto(_)))),
            "le répondeur dit « version », pas « refusé » : {accepted:?}"
        );
        let (code, reason) = close_of(&client.conn).await;
        assert_eq!(code, auth::CLOSE_WRONG_MAJOR);
        assert_eq!(reason, "");

        client.shutdown().await;
        endpoint.close().await;
    })
    .await;
}

// --- 4. The pre-authentication budget, on a paused clock ---------------------

/// A peer that completes the TLS handshake and then says nothing is closed after
/// exactly [`auth::PREAUTH_DEADLINE`], with the refusal code and no byte written.
///
/// The clock is tokio's **paused** one, and the assertion is the instant the refusal
/// lands on: exactly two seconds of virtual time. A `start_paused` runtime advances
/// its own clock to the next timer as soon as it has nothing to run, so "not yet
/// finished" is not a reliable assertion — where the clock LANDS is, and a deadline
/// of one minute would land at one minute.
#[tokio::test(start_paused = true)]
async fn the_responder_closes_two_seconds_without_a_valid_auth() {
    let subject = subject_of(0x61);
    let mine = device("silent-own", subject, (now() - 60, now() + 86_400));
    let endpoint = Arc::new(node(Arc::clone(&mine), trust(subject, None, None), "PC"));
    let started = tokio::time::Instant::now();

    // The silent peer runs in its own task and signals the instant its stream is
    // open, so the deadline is measured from a real event, never from a sleep.
    let (ready, opened) = tokio::sync::oneshot::channel();
    let address = address(&endpoint);
    let silent = tokio::spawn(async move {
        let client = RawClient::dial(address, &mine).await;
        let (send, recv) = client.conn.open_bi().await.expect("un flux muet");
        let _ = ready.send(());
        (client, send, recv)
    });
    let accepting = tokio::spawn({
        let endpoint = Arc::clone(&endpoint);
        async move { endpoint.accept().await.map(|result| result.err()) }
    });
    opened.await.expect("le pair silencieux tient son flux");

    let refused = accepting.await.expect("la tâche du répondeur");
    let waited = started.elapsed();
    assert_eq!(
        waited,
        auth::PREAUTH_DEADLINE,
        "le refus tombe exactement au budget, pas ailleurs"
    );
    assert_eq!(
        refused,
        Some(Some(LanError::Refused)),
        "à deux secondes, le répondeur refuse"
    );

    let (client, _send, recv) = silent.await.expect("la tâche du pair muet");
    let (code, reason) = close_of(&client.conn).await;
    assert_eq!(code, auth::CLOSE_REFUSED);
    assert_eq!(reason, "");
    assert_eq!(client.bytes_written(recv).await, 0);
    client.shutdown().await;
    endpoint.close().await;
}

/// The other half of the budget: a peer that announces more than 2 KiB before
/// authenticating is refused **on the header**, without the body being read and
/// without the pre-authentication deadline being spent at all.
///
/// The assertion is the paused clock again: a refusal that waited for the body would
/// have to run into the 2 s deadline, and the virtual clock would land there.
#[tokio::test(start_paused = true)]
async fn the_responder_refuses_more_than_two_kib_before_auth() {
    let subject = subject_of(0x62);
    let mine = device("big-own", subject, (now() - 60, now() + 86_400));
    let endpoint = Arc::new(node(Arc::clone(&mine), trust(subject, None, None), "PC"));
    let started = tokio::time::Instant::now();

    let (accepted, (client, written)) = tokio::join!(endpoint.accept(), async {
        let client = RawClient::dial(address(&endpoint), &mine).await;
        let (mut send, recv) = client.conn.open_bi().await.expect("un flux");
        // An `Auth` whose announced body is over the 2 KiB pre-auth budget: the header
        // alone proves it, so the responder never reads the body. The padding is in the
        // base64 field, which is what a hostile peer would grow.
        let oversized = Control::Auth {
            cred: base64(mine.cred.raw()),
            proof: "A".repeat(auth::PREAUTH_MAX_BYTES),
            revocation_serial: 0,
            proto: 1,
        };
        let json = serde_json::to_vec(&oversized).expect("json");
        assert!(json.len() > auth::PREAUTH_MAX_BYTES - codec::FRAME_HEADER_BYTES);
        send.write_all(&(json.len() as u32).to_be_bytes())
            .await
            .expect("l'en-tête");
        // …and NOTHING of the body: the cap is a decision on the four header bytes.
        let written = client.bytes_written(recv).await;
        (client, written)
    });
    assert_eq!(
        started.elapsed(),
        Duration::ZERO,
        "le plafond refuse sur l'en-tête, sans dépenser le budget de 2 s"
    );
    assert_eq!(written, 0, "aucun octet écrit avant l'authentification");
    assert_eq!(
        accepted.map(|result| result.err()),
        Some(Some(LanError::Refused))
    );
    let (code, _) = close_of(&client.conn).await;
    assert_eq!(code, auth::CLOSE_REFUSED);
    client.shutdown().await;
    endpoint.close().await;
}

// --- 5. TLS: 1.2 refused, no early data --------------------------------------

/// 0-RTT is off on both sides: the server advertises `max_early_data_size = 0`, the
/// production client never enables it, and a client that DOES offer early data gets
/// nothing accepted.
#[tokio::test]
async fn early_data_is_never_accepted() {
    bounded("early_data_is_never_accepted", async {
        let subject = subject_of(0x71);
        let mine = device("rtt-own", subject, (now() - 60, now() + 86_400));

        // The two configurations the transport really builds.
        let server_tls = transport::server_tls_config(&mine).expect("le TLS serveur");
        assert_eq!(server_tls.max_early_data_size, 0, "RFC 9001 §9.2");
        assert!(!server_tls.send_half_rtt_data, "aucune donnée en demi-RTT");
        let client_tls = transport::client_tls_config(&mine).expect("le TLS client");
        assert!(
            !client_tls.enable_early_data,
            "le client n'offre jamais 0-RTT"
        );

        // A raw server that speaks no `nz-lan` handshake at all: this test is about TLS,
        // so nothing here may close the connection for a missing `Auth`.
        let (server, address) = raw_server(&mine).await;
        let echoing = tokio::spawn(async move {
            let conn = server
                .accept()
                .await
                .expect("une connexion")
                .await
                .expect("TLS");
            // One round trip, so the client certainly processed its session ticket.
            let (mut send, mut recv) = conn.accept_bi().await.expect("un flux");
            let mut byte = [0u8; 1];
            recv.read_exact(&mut byte).await.expect("un octet");
            send.write_all(&byte).await.expect("l'écho");
            send.finish().ok();
            // Keep the connection open until the client is done with it.
            let _ = conn.closed().await;
            server
        });

        // A client that DOES ask for early data, twice: the second dial has a ticket and
        // still gets no 0-RTT, because the server never advertises any.
        let mut greedy = transport::client_tls_config(&mine).expect("le TLS client");
        greedy.enable_early_data = true;
        let mut raw =
            quinn::Endpoint::client(SocketAddr::from(([127, 0, 0, 1], 0))).expect("un client");
        raw.set_default_client_config(quinn::ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(greedy).expect("client QUIC"),
        )));
        let first = raw
            .connect(address, "neezer-lan")
            .expect("un dial")
            .await
            .expect("une poignée de main");
        let (mut send, mut recv) = first.open_bi().await.expect("un flux");
        send.write_all(b"x").await.expect("un octet");
        send.finish().ok();
        let mut echo = [0u8; 1];
        recv.read_exact(&mut echo).await.expect("l'écho");
        assert_eq!(&echo, b"x");
        first.close(0u32.into(), b"");

        let second = raw.connect(address, "neezer-lan").expect("un dial");
        // The other honest outcome is `Err`: with `max_early_data_size = 0` there is
        // nothing to resume early, and quinn refuses to start 0-RTT at all.
        if let Ok((_conn, accepted)) = second.into_0rtt() {
            assert!(
                !accepted.await,
                "le serveur n'accepte jamais de 0-RTT, même proposé"
            );
        }
        raw.close(0u32.into(), b"");
        let _ = tokio::time::timeout(Duration::from_secs(5), echoing).await;
    })
    .await;
}

/// A session cannot be established with TLS 1.2: the server offers TLS 1.3 only, and
/// a client restricted to 1.2 gets an alert. The control case — the crate's own two
/// configurations — completes, so the failure is the version and nothing else.
#[tokio::test]
async fn tls12_cannot_establish_a_session() {
    bounded("tls12_cannot_establish_a_session", async {
        let subject = subject_of(0x72);
        let a = device("tls-a", subject, (now() - 60, now() + 86_400));
        let b = device("tls-b", subject, (now() - 60, now() + 86_400));

        // Control: TLS 1.3 with both of the crate's configurations.
        let server_config = transport::server_tls_config(&a).expect("le TLS serveur");
        let client_config = transport::client_tls_config(&b).expect("le TLS client");
        pump_handshake(server_config, client_config).expect("TLS 1.3 doit aboutir");

        // A client that offers only TLS 1.2. Its own verifier (an empty root store) is
        // irrelevant: the versions have to agree before a certificate is looked at.
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let mut legacy = rustls::ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS12])
            .expect("une configuration TLS 1.2")
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth();
        // Same ALPN, so the version is the ONLY thing that differs.
        legacy.alpn_protocols = vec![transport::ALPN.to_vec()];
        let server_config = transport::server_tls_config(&a).expect("le TLS serveur");
        let error = pump_handshake(server_config, legacy).expect_err("TLS 1.2 ne doit pas aboutir");
        let message = error;
        assert!(
            message.contains("Tls12NotOffered")
                || message.contains("Alert")
                || message.contains("ProtocolVersion"),
            "l'échec doit venir de la version, pas d'autre chose : {message}"
        );
        println!("TLS 1.2 refused by the server: {message}");
    })
    .await;
}

/// Pumps a rustls handshake through two in-memory buffers and answers the first error
/// either side reports.
fn pump_handshake(
    server: rustls::ServerConfig,
    client: rustls::ClientConfig,
) -> Result<(), String> {
    let mut server =
        rustls::ServerConnection::new(Arc::new(server)).map_err(|error| format!("{error:?}"))?;
    let mut client = rustls::ClientConnection::new(
        Arc::new(client),
        rustls::pki_types::ServerName::try_from("neezer-lan").expect("un nom"),
    )
    .map_err(|error| format!("{error:?}"))?;
    for _ in 0..32 {
        let mut to_server = Vec::new();
        while client.wants_write() {
            client
                .write_tls(&mut to_server)
                .map_err(|error| format!("{error:?}"))?;
        }
        if !to_server.is_empty() {
            let mut cursor = std::io::Cursor::new(to_server);
            while server
                .read_tls(&mut cursor)
                .map_err(|error| format!("{error:?}"))?
                > 0
            {}
            server
                .process_new_packets()
                .map_err(|error| format!("{error:?}"))?;
        }
        let mut to_client = Vec::new();
        while server.wants_write() {
            server
                .write_tls(&mut to_client)
                .map_err(|error| format!("{error:?}"))?;
        }
        if !to_client.is_empty() {
            let mut cursor = std::io::Cursor::new(to_client);
            while client
                .read_tls(&mut cursor)
                .map_err(|error| format!("{error:?}"))?
                > 0
            {}
            client
                .process_new_packets()
                .map_err(|error| format!("{error:?}"))?;
        }
        if !client.is_handshaking() && !server.is_handshaking() {
            return Ok(());
        }
    }
    Err("la poignée de main n'a pas abouti en 32 tours".to_owned())
}

// --- 6. Duplicates -----------------------------------------------------------

/// When both devices dial, exactly one connection survives, and it is the one the
/// LOWER `device_pub` dialled — a rule both sides compute from the same two keys, so
/// both keep the same one. Proven by a control frame: the survivor on the responder's
/// side is the connection the lower key dialled, and a frame written there arrives on
/// the lower key's own `PeerConn`.
#[tokio::test]
async fn a_duplicate_pair_collapses_to_the_lower_key_dial() {
    bounded("a_duplicate_pair_collapses_to_the_lower_key_dial", async {
        let subject = subject_of(0x81);
        let a = device_with_rank("dup-a", subject, true);
        let b = device_with_rank("dup-b", subject, false);
        assert!(
            a.key.public() < b.key.public(),
            "le test a choisi les clés pour que A soit la plus petite"
        );
        let endpoint_a = node(Arc::clone(&a), trust(subject, None, None), "A");
        let endpoint_b = node(Arc::clone(&b), trust(subject, None, None), "B");

        let (from_a, from_b) = tokio::join!(
            async {
                let (client, accepted) = tokio::join!(
                    endpoint_a.connect(address(&endpoint_b)),
                    endpoint_b.accept()
                );
                (client, accepted)
            },
            async {
                let (client, accepted) = tokio::join!(
                    endpoint_b.connect(address(&endpoint_a)),
                    endpoint_a.accept()
                );
                (client, accepted)
            }
        );
        let a_dial = from_a.0.expect("A joint B");
        let b_incoming = from_a.1.expect("B accepte A").expect("B s'authentifie");
        let b_dial = from_b.0.expect("B joint A");
        let _a_incoming = from_b.1;

        // One connection per pair on each side, and the survivor is A's dial.
        assert_eq!(endpoint_a.connections(), 1, "une seule connexion par paire");
        assert_eq!(endpoint_b.connections(), 1, "une seule connexion par paire");

        // The rule, proven by which handle still writes: at B the survivor is the
        // connection A dialled (B was dialled), and B's own dial was closed with the
        // duplicate code. A control frame written on the survivor arrives on A's dialled
        // `PeerConn`, which is what proves both sides kept the SAME connection.
        assert!(
            b_incoming
                .send_control(Control::InventoryChanged { version: 7 })
                .await
                .is_ok(),
            "B garde la connexion que la plus petite clé a composée"
        );
        assert!(
            b_dial
                .send_control(Control::InventoryChanged { version: 8 })
                .await
                .is_err(),
            "B abandonne la connexion qu'il a composée lui-même"
        );
        let received = tokio::time::timeout(Duration::from_secs(5), a_dial.next_control())
            .await
            .expect("la trame arrive")
            .expect("le flux de contrôle est ouvert");
        assert_eq!(received, Control::InventoryChanged { version: 7 });
        assert!(
            a_dial.dialled_by_us(),
            "A garde la connexion qu'il a composée"
        );

        endpoint_a.close().await;
        endpoint_b.close().await;
    })
    .await;
}

// --- 7. Frames, bodies and control ------------------------------------------

/// A request crosses on its own stream, its body arrives in capped chunks, and the
/// announced length is what the requester is held to.
#[tokio::test]
async fn a_request_and_its_body_cross_a_stream() {
    bounded("a_request_and_its_body_cross_a_stream", async {
        let subject = subject_of(0x91);
        let a = device("body-a", subject, (now() - 60, now() + 86_400));
        let b = device("body-b", subject, (now() - 60, now() + 86_400));
        let endpoint_a = node(Arc::clone(&a), trust(subject, None, None), "A");
        let endpoint_b = node(Arc::clone(&b), trust(subject, None, None), "B");

        // 200 KiB, so the body is four chunks of at most 64 KiB.
        let payload: Vec<u8> = (0..200 * 1024).map(|index| (index % 251) as u8).collect();
        let announced = payload.len() as u64;

        let (client, accepted) = tokio::join!(
            endpoint_a.connect(address(&endpoint_b)),
            endpoint_b.accept()
        );
        let client = client.expect("authentifié");
        let accepted = accepted.expect("accepté").expect("authentifié");

        let body = payload.clone();
        let answering = tokio::spawn(async move {
            let incoming = accepted.next_incoming().await.expect("une requête");
            assert!(matches!(incoming.request, Request::Fetch { .. }));
            incoming
                .respond(
                    Response::FetchOk {
                        len: announced,
                        tag: None,
                        quality: proto::WireQuality::Flac,
                    },
                    Some(Box::new(std::io::Cursor::new(body))),
                )
                .await
        });

        let reply = client
            .request(Request::Fetch {
                track: "3135556".to_owned(),
                quality: proto::WireQuality::Flac,
                from: 0,
                expect: None,
                urgency: proto::Urgency::Play,
            })
            .await
            .expect("une réponse");
        assert_eq!(
            reply.head,
            Response::FetchOk {
                len: announced,
                tag: None,
                quality: proto::WireQuality::Flac,
            }
        );
        let mut body = reply.body.expect("un corps annoncé");
        assert_eq!(body.remaining(), announced);
        let mut received = Vec::new();
        let mut chunks = 0;
        while let Some(chunk) = body.chunk(usize::MAX).await.expect("un morceau") {
            assert!(
                chunk.len() <= codec::MAX_CHUNK_BYTES,
                "un morceau ne dépasse jamais 64 KiB, même demandé plus grand"
            );
            chunks += 1;
            received.extend_from_slice(&chunk);
        }
        assert_eq!(received, payload);
        assert_eq!(chunks, 4, "200 KiB en quatre morceaux de 64 KiB au plus");
        assert_eq!(body.remaining(), 0);
        assert_eq!(
            answering
                .await
                .expect("la tâche du répondeur")
                .expect("la réponse écrite"),
            announced
        );

        endpoint_a.close().await;
        endpoint_b.close().await;
    })
    .await;
}

/// An unknown tag is answered `Error { code: "unknown" }` and the connection lives on
/// (rule 7: a newer peer talking to an older one).
#[tokio::test]
async fn an_unknown_frame_is_answered_and_the_connection_lives() {
    bounded(
        "an_unknown_frame_is_answered_and_the_connection_lives",
        async {
            let subject = subject_of(0x92);
            let a = device("unknown-a", subject, (now() - 60, now() + 86_400));
            let b = device("unknown-b", subject, (now() - 60, now() + 86_400));
            let endpoint_a = node(Arc::clone(&a), trust(subject, None, None), "A");
            let endpoint_b = node(Arc::clone(&b), trust(subject, None, None), "B");
            let (client, accepted) = tokio::join!(
                endpoint_a.connect(address(&endpoint_b)),
                endpoint_b.accept()
            );
            let client = client.expect("authentifié");
            let accepted = accepted.expect("accepté").expect("authentifié");
            let answering = answer_pings(accepted);

            // A frame from a future version of this protocol, on its own stream.
            let (mut send, mut recv) = client.connection().open_bi().await.expect("un flux");
            let unknown = br#"{"t":"Futur","champ":1}"#;
            send.write_all(&(unknown.len() as u32).to_be_bytes())
                .await
                .expect("l'en-tête");
            send.write_all(unknown).await.expect("la trame");
            let answer: Response = codec::read_frame(&mut recv, MAX_FRAME_BYTES)
                .await
                .expect("une réponse")
                .expect("une trame");
            assert_eq!(
                answer,
                Response::Error {
                    code: "unknown".to_owned()
                }
            );

            // …and the connection still answers a request it does understand.
            let reply = client.request(Request::Ping).await.expect("un Ping");
            assert_eq!(reply.head, Response::Pong);
            let _ = tokio::time::timeout(Duration::from_secs(5), answering).await;

            endpoint_a.close().await;
            endpoint_b.close().await;
        },
    )
    .await;
}

/// The control stream carries gossip after `Hello`, and its end is a clean `None`,
/// not an error: that is how `Bye` and a closed connection arrive.
#[tokio::test]
async fn the_control_stream_carries_gossip_and_ends_cleanly() {
    bounded(
        "the_control_stream_carries_gossip_and_ends_cleanly",
        async {
            let subject = subject_of(0x93);
            let a = device("control-a", subject, (now() - 60, now() + 86_400));
            let b = device("control-b", subject, (now() - 60, now() + 86_400));
            let endpoint_a = node(Arc::clone(&a), trust(subject, None, None), "A");
            let endpoint_b = node(Arc::clone(&b), trust(subject, None, None), "B");
            let (client, accepted) = tokio::join!(
                endpoint_a.connect(address(&endpoint_b)),
                endpoint_b.accept()
            );
            let client = client.expect("authentifié");
            let accepted = accepted.expect("accepté").expect("authentifié");

            accepted
                .send_control(Control::SetsChanged { version: 12 })
                .await
                .expect("une trame de contrôle");
            accepted
                .send_control(Control::Bye)
                .await
                .expect("un au revoir");
            assert_eq!(
                client.next_control().await,
                Some(Control::SetsChanged { version: 12 })
            );
            assert_eq!(client.next_control().await, Some(Control::Bye));

            // The responder's side of the stream is still open; closing the connection ends
            // it, and the reader says so with `None` rather than an error.
            client.close(0);
            assert_eq!(client.next_control().await, None);

            endpoint_a.close().await;
            endpoint_b.close().await;
        },
    )
    .await;
}

// --- 8. Caps -----------------------------------------------------------------

/// The two halves of "a 1 MiB + 1 frame closes the connection and allocates ≤ 1 MiB":
/// the decision on the announced length allocates **nothing**, and the whole
/// exchange that closes the connection stays under the mebibyte the note allows.
#[tokio::test]
async fn an_over_cap_frame_allocates_nothing_and_stays_under_a_mebibyte() {
    bounded(
        "an_over_cap_frame_allocates_nothing_and_stays_under_a_mebibyte",
        async {
            // 1. The decision alone: the refusal's cost does NOT grow with the announced
            //    size. A mebibyte too many and four gigabytes cost the same handful of bytes
            //    (the error's own short sentence), where an implementation that allocated
            //    first would show two very different numbers.
            let mut deltas = Vec::new();
            for announced in [MAX_FRAME_BYTES + 1, 8 * MAX_FRAME_BYTES, u32::MAX as usize] {
                let header = (announced as u32).to_be_bytes();
                let (refused, allocated) =
                    counted(|| codec::frame_body_len(header, MAX_FRAME_BYTES));
                assert!(refused.is_err(), "{announced} octets sont refusés");
                deltas.push(allocated);
            }
            assert_eq!(
                deltas[0], deltas[2],
                "refuser 4 GiB ne coûte pas plus que refuser 1 MiB + 1 : {deltas:?}"
            );
            assert!(
                deltas.iter().all(|delta| *delta < 512),
                "la décision n'alloue que sa phrase d'erreur : {deltas:?}"
            );
            // …and a frame at the cap is accepted by the same function, so the cap is a
            // boundary and not a blanket refusal.
            let (accepted, allocated) = counted(|| {
                codec::frame_body_len((MAX_FRAME_BYTES as u32).to_be_bytes(), MAX_FRAME_BYTES)
            });
            assert_eq!(accepted.expect("un mébioctet passe"), MAX_FRAME_BYTES);
            assert_eq!(allocated, 0);

            // 2. End to end: the responder reads the over-cap header, refuses, and closes the
            //    connection — having allocated less than the mebibyte the note allows.
            let subject = subject_of(0xa1);
            let a = device("cap-a", subject, (now() - 60, now() + 86_400));
            let b = device("cap-b", subject, (now() - 60, now() + 86_400));
            let endpoint_a = node(Arc::clone(&a), trust(subject, None, None), "A");
            let endpoint_b = node(Arc::clone(&b), trust(subject, None, None), "B");
            let (client, accepted) = tokio::join!(
                endpoint_a.connect(address(&endpoint_b)),
                endpoint_b.accept()
            );
            let client = client.expect("authentifié");
            let accepted = accepted.expect("accepté").expect("authentifié");

            let (mut send, _recv) = client.connection().open_bi().await.expect("un flux");
            send.write_all(&((MAX_FRAME_BYTES + 1) as u32).to_be_bytes())
                .await
                .expect("l'en-tête dit un octet de trop");
            let (closed, allocated) =
                counted_async(async { accepted.next_incoming().await.is_none() }).await;
            assert!(
                closed,
                "le répondeur cesse d'accepter des flux et ferme la connexion"
            );
            println!("over-cap frame refused, {allocated} bytes allocated end to end");
            assert!(
                allocated <= MAX_FRAME_BYTES,
                "l'échange reste sous le mébioctet : {allocated}"
            );
            let (code, reason) = close_of(client.connection()).await;
            assert_eq!(
                code,
                auth::CLOSE_REFUSED,
                "une taille hostile ferme la connexion"
            );
            assert_eq!(reason, "");

            endpoint_a.close().await;
            endpoint_b.close().await;
        },
    )
    .await;
}

/// A raw quinn server built from the crate's TLS configuration, for the tests that
/// speak the wire directly (the codec's caps).
async fn raw_server(id: &Identity) -> (quinn::Endpoint, SocketAddr) {
    let tls = transport::server_tls_config(id).expect("le TLS serveur");
    let server = quinn::Endpoint::server(
        quinn::ServerConfig::with_crypto(Arc::new(
            quinn::crypto::rustls::QuicServerConfig::try_from(tls).expect("serveur QUIC"),
        )),
        SocketAddr::from(([127, 0, 0, 1], 0)),
    )
    .expect("un serveur brut");
    let address = server.local_addr().expect("son adresse");
    (server, address)
}

// --- 9. Measures -------------------------------------------------------------

/// The note's headline number, printed: a full TLS handshake, both exporter-bound
/// proofs, both `Hello` frames, 100 times on loopback.
///
/// The assertion is the note's own threshold (p95 ≤ 15 ms) in **release**; the debug
/// build — what CI runs — asserts a ceiling that a loaded machine cannot cross and
/// prints the same two numbers, so a regression is visible there too.
#[tokio::test]
async fn the_handshake_and_both_proofs_are_fast() {
    bounded("the_handshake_and_both_proofs_are_fast", async {
    let subject = subject_of(0xb1);
    let a = device("bench-a", subject, (now() - 60, now() + 86_400));
    let b = device("bench-b", subject, (now() - 60, now() + 86_400));
    let endpoint_a = node(Arc::clone(&a), trust(subject, None, None), "A");
    let endpoint_b = node(Arc::clone(&b), trust(subject, None, None), "B");
    let target = address(&endpoint_b);

    const RUNS: usize = 100;
    let mut samples = Vec::with_capacity(RUNS);
    for _ in 0..RUNS {
        let started = Instant::now();
        let (client, accepted) = tokio::join!(endpoint_a.connect(target), endpoint_b.accept());
        samples.push(started.elapsed());
        client.expect("authentifié").close(0);
        if let Some(Ok(conn)) = accepted {
            conn.close(0);
        }
    }
    samples.sort();
    let p50 = samples[RUNS / 2];
    let p95 = samples[(RUNS * 95) / 100];
    println!(
        "handshake + both proofs, {RUNS} runs, {} build: p50 {:.2} ms, p95 {:.2} ms, max {:.2} ms",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        p50.as_secs_f64() * 1e3,
        p95.as_secs_f64() * 1e3,
        samples[RUNS - 1].as_secs_f64() * 1e3,
    );

    let ceiling = if cfg!(debug_assertions) {
        Duration::from_millis(300)
    } else {
        Duration::from_millis(15)
    };
    assert!(
        p95 <= ceiling,
        "p95 {:.2} ms au-delà du plafond {:?}",
        p95.as_secs_f64() * 1e3,
        ceiling
    );

    endpoint_a.close().await;
    endpoint_b.close().await;
    })
    .await;
}

/// `close` joins the crate's only task — quinn's endpoint driver — and the runtime
/// can stop right after: dropping it does not panic, `accept()` has already answered
/// `None`, and a fresh runtime still runs.
#[tokio::test]
async fn closing_the_endpoint_joins_the_only_task() {
    bounded("closing_the_endpoint_joins_the_only_task", async {
        let subject = subject_of(0xb2);
        let a = device("close-a", subject, (now() - 60, now() + 86_400));
        let b = device("close-b", subject, (now() - 60, now() + 86_400));
        let endpoint_a = node(Arc::clone(&a), trust(subject, None, None), "A");
        let endpoint_b = node(Arc::clone(&b), trust(subject, None, None), "B");
        let (client, accepted) = tokio::join!(
            endpoint_a.connect(address(&endpoint_b)),
            endpoint_b.accept()
        );
        let client = client.expect("authentifié");
        let _accepted = accepted.expect("accepté").expect("authentifié");

        let started = Instant::now();
        endpoint_a.close().await;
        endpoint_b.close().await;
        let took = started.elapsed();
        assert!(
            took < transport::SHUTDOWN_DEADLINE,
            "la fermeture joint le pilote quinn sans attendre la limite : {took:?}"
        );
        assert_eq!(endpoint_a.connections(), 0, "plus aucune connexion");
        // A closed endpoint refuses every operation, and `accept` answers `None` without
        // waiting: the driver task is gone.
        assert!(endpoint_a.accept().await.is_none());
        assert!(matches!(
            endpoint_a.connect(address(&endpoint_b)).await,
            Err(LanError::PeerGone) | Err(LanError::Internal(_))
        ));
        drop(client);
        // The runtime keeps running: nothing was left to abort.
        tokio::task::yield_now().await;
    })
    .await;
}

/// A preferred port that is taken makes the node bind an ephemeral one, and it says
/// so: a screen that copied a firewall line for a port nobody listens on would be
/// the bug this flag prevents.
#[tokio::test]
async fn a_busy_preferred_port_falls_back_to_an_ephemeral_one() {
    bounded(
        "a_busy_preferred_port_falls_back_to_an_ephemeral_one",
        async {
            // Take the note's default port; if this machine already has it, take any port —
            // what is asserted is the fallback, not the number.
            let guard = std::net::UdpSocket::bind(("0.0.0.0", DEFAULT_PORT))
                .or_else(|_| std::net::UdpSocket::bind(("0.0.0.0", 0)))
                .expect("un port occupé");
            let busy = guard.local_addr().expect("l'adresse du garde").port();

            let subject = subject_of(0xb3);
            let a = device("port-a", subject, (now() - 60, now() + 86_400));
            let handle = tokio::runtime::Handle::current();
            let endpoint = Endpoint::bind(
                &handle,
                busy,
                Arc::clone(&a),
                trust(subject, None, None),
                hello("A"),
            )
            .expect("un endpoint lié malgré le port occupé");
            assert!(endpoint.port_fallback(), "le repli est signalé");
            assert_ne!(endpoint.port(), busy, "le port occupé n'est pas réutilisé");
            assert_ne!(endpoint.port(), 0);
            endpoint.close().await;
            drop(guard);
        },
    )
    .await;
}

/// Sixteen is the cap: the seventeenth peer is refused rather than queued behind a
/// table that would grow with every stranger on the Wi-Fi (T9).
#[tokio::test]
async fn the_connection_cap_is_sixteen() {
    bounded("the_connection_cap_is_sixteen", async {
        let subject = subject_of(0xb4);
        let owner = device("cap-owner", subject, (now() - 60, now() + 86_400));
        let endpoint = node(Arc::clone(&owner), trust(subject, None, None), "PC");

        let mut held = Vec::new();
        let mut peers = Vec::new();
        for index in 0..16 {
            let peer = device(
                &format!("cap-peer-{index}"),
                subject,
                (now() - 60, now() + 86_400),
            );
            let other = node(Arc::clone(&peer), trust(subject, None, None), "peer");
            let (client, accepted) =
                tokio::join!(other.connect(address(&endpoint)), endpoint.accept());
            held.push(client.expect("une connexion de plus"));
            accepted.expect("accepté").expect("authentifié");
            // The dialler's endpoint must stay alive, or its connection goes with it.
            peers.push(other);
        }
        assert_eq!(endpoint.connections(), 16, "seize connexions tenues");

        let seventeenth = device("cap-peer-16", subject, (now() - 60, now() + 86_400));
        let other = node(Arc::clone(&seventeenth), trust(subject, None, None), "peer");
        let (client, accepted) = tokio::join!(other.connect(address(&endpoint)), endpoint.accept());
        assert!(
            matches!(client, Err(LanError::Refused) | Err(LanError::PeerGone)),
            "le dix-septième est refusé, pas mis en attente"
        );
        assert!(
            matches!(
                accepted,
                Some(Err(LanError::Refused)) | Some(Err(LanError::Busy { .. }))
            ),
            "le répondeur refuse la dix-septième"
        );
        assert_eq!(endpoint.connections(), 16, "la table n'a pas grandi");

        for conn in held {
            conn.close(0);
        }
        endpoint.close().await;
        other.close().await;
        for peer in peers {
            peer.close().await;
        }
    })
    .await;
}
