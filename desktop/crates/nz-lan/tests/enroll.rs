//! LN7's acceptance: enrolment, renewal, the conditional roster sync, the trust
//! gossip and the logout wipe — against a FAKE HTTP SERVER that speaks LN1's
//! contract (`docs/agents/lan-nodes.md`, *Server*) with the note's own test
//! vectors, signed with the note's LAN seed.
//!
//! Why a hand-written server rather than a stub of `Api`: the point of the lot is
//! the WIRE — how many requests, on which lane, with which `If-None-Match`, for how
//! many bytes — and the crypto (a credential, a roster and a revocation list the
//! device must verify before it stores anything). A stub of `Api` would assert what
//! this file hopes the server says; this one signs what `neezer/lan.py` signs.
//!
//! Every number in an assertion is written beside it. The reference material (the
//! LAN seed, the kid, the subject, the credential layout and the domain strings) is
//! copied from the note, not imported from the crate: a drift on either side (the
//! Python encoder, this test) must break it.
//!
//! Nothing here touches the person's display, sound or network: one loopback
//! listener on an ephemeral port, and every data directory under the crate's own
//! `target/`.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use aws_lc_rs::signature::{ED25519, Ed25519KeyPair, UnparsedPublicKey};
use nz_api::{Api, ApiError, SessionJar};
use nz_lan::cred::Roster;
use nz_lan::enroll::{STAGED_DIR, ensure_enrolled, forget, load_cached, sync_trust};
use nz_lan::gossip;
use nz_lan::keys::KEY_FILE;
use nz_lan::proto::Control;
use nz_lan::store::Store;
use nz_lan::transport::{SharedTrust, TrustState};
use nz_lan::types::{Kid, LanError, Subject};
use serde_json::{Value, json};

// --- The note's reference material, verbatim ---------------------------------

/// The server's LAN key seed: bytes `0x00..=0x1f` (pub `03a107…31b8`).
const LAN_SEED: [u8; 32] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
];

/// The device's seed: bytes `0x20..=0x3f` (pub `29acbae1…6dd7`). Used only to
/// build a *forged* blob the device must refuse.
const DEVICE_SEED: [u8; 32] = [
    0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2d, 0x2e, 0x2f,
    0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x3b, 0x3c, 0x3d, 0x3e, 0x3f,
];

const LAN_PUB_HEX: &str = "03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8";
const KID_HEX: &str = "56475aa75463474c";
const SUBJECT_HEX: &str = "00112233445566778899aabbccddeeff";
/// The device row's id the fake server issues: 12 bytes, hex `01…01` (24 chars).
const DEVICE_ID_HEX: &str = "010101010101010101010101";

const CREDENTIAL_DOMAIN: &[u8] = b"neezer-lan/1 credential\x00";
const REVOCATIONS_DOMAIN: &[u8] = b"neezer-lan/1 revocations\x00";
const ROSTER_DOMAIN: &[u8] = b"neezer-lan/1 roster\x00";
const ENROLL_DOMAIN: &[u8] = b"neezer-lan/1 enroll\x00";

/// The instant the tests start from (the note's `not_before`).
const NOW: i64 = 1_798_761_600;
const DAY: i64 = 86_400;
/// The server's credential lifetime (`lan_credential_days`, default).
const CREDENTIAL_DAYS: i64 = 30;
/// The renewal threshold of the note (*Enrolment*): "fewer than 23 of its 30 days
/// left". Written out HERE rather than imported from the crate, so a mutant that
/// moves the constant cannot move the test's boundary with it.
const RENEW_BELOW: i64 = 23 * DAY;

fn hex(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() / 2);
    for pair in text.as_bytes().as_chunks::<2>().0 {
        let pair = std::str::from_utf8(pair).expect("ascii");
        out.push(u8::from_str_radix(pair, 16).expect("hex"));
    }
    out
}

fn fixed<const N: usize>(text: &str) -> [u8; N] {
    hex(text)
        .try_into()
        .unwrap_or_else(|bytes: Vec<u8>| panic!("{N} octets attendus, {} reçus", bytes.len()))
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Standard base64 with padding — this file's own encoder, so a broken
/// `base64_encode` in the crate cannot agree with it.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let first = u32::from(chunk[0]);
        let second = u32::from(*chunk.get(1).unwrap_or(&0));
        let third = u32::from(*chunk.get(2).unwrap_or(&0));
        let triple = (first << 16) | (second << 8) | third;
        out.push(ALPHABET[(triple >> 18) as usize & 63] as char);
        out.push(ALPHABET[(triple >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(triple >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[triple as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn base64_decode(text: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let mut accumulator = 0u32;
    let mut bits = 0u32;
    for byte in text.bytes() {
        if byte == b'=' {
            break;
        }
        let value = ALPHABET.iter().position(|candidate| *candidate == byte)? as u32;
        accumulator = (accumulator << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((accumulator >> bits) as u8);
        }
    }
    Some(out)
}

fn lan_signer() -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&LAN_SEED).expect("the LAN seed")
}

fn device_signer() -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&DEVICE_SEED).expect("the device seed")
}

fn lan_public() -> [u8; 32] {
    fixed::<32>(LAN_PUB_HEX)
}

fn lan_keys() -> nz_lan::cred::TrustKeys {
    nz_lan::cred::TrustKeys::from_status(&[(reference_kid(), lan_public())])
}

fn reference_kid() -> Kid {
    Kid(fixed::<8>(KID_HEX))
}

fn reference_subject() -> Subject {
    Subject(fixed::<16>(SUBJECT_HEX))
}

fn key_of() -> Kid {
    reference_kid()
}

fn sign_with(signer: &Ed25519KeyPair, domain: &[u8], message: &[u8]) -> Vec<u8> {
    let mut signed = domain.to_vec();
    signed.extend_from_slice(message);
    signer.sign(&signed).as_ref().to_vec()
}

// --- The fake server ---------------------------------------------------------

/// What the fake server is told to do differently, for the refusal tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tamper {
    /// The normal server.
    #[default]
    None,
    /// No `/api/lan/status` at all (an older server, `404`).
    Missing,
    /// The admin switched the feature off (`enabled: false`).
    Disabled,
    /// `POST /api/lan/enroll` answers the skew refusal (`403 stale_proof`).
    StaleProof,
    /// `POST /api/lan/enroll` refuses the proof (`403 bad_proof`), so a RENEWAL
    /// fails after a successful status.
    RefuseEnroll,
    /// The credential is signed by the right key, for ANOTHER subject.
    OtherSubject,
    /// The credential is signed for another device key.
    OtherDevicePub,
    /// The roster envelope carries a signature made with the wrong key.
    ForgedRoster,
    /// The roster is signed by the right key, but about ANOTHER subject.
    OtherRosterSubject,
}

#[derive(Clone, Debug)]
struct Config {
    tamper: Tamper,
    /// The roster serial the server would sign right now.
    serial: u64,
    /// The revocation serial, and the keys it names.
    revocation_serial: u64,
    revoked: Vec<[u8; 32]>,
    credential_days: i64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tamper: Tamper::None,
            serial: 42,
            revocation_serial: 7,
            revoked: Vec::new(),
            credential_days: CREDENTIAL_DAYS,
        }
    }
}

#[derive(Clone, Debug)]
struct DeviceRow {
    id: [u8; 12],
    pubkey: [u8; 32],
    name: String,
    platform: String,
}

struct State {
    config: Config,
    device: Option<DeviceRow>,
}

/// One request as the server saw it.
#[derive(Clone, Debug)]
struct Hit {
    method: String,
    path: String,
    background: bool,
    if_none_match: Option<String>,
    request_body: String,
    status: u16,
    response_bytes: usize,
}

impl Hit {
    fn is(&self, method: &str, path: &str) -> bool {
        self.method == method && self.path == path
    }
}

struct Fake {
    port: u16,
    state: Arc<Mutex<State>>,
    hits: Arc<Mutex<Vec<Hit>>>,
}

impl Fake {
    fn start(config: Config) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a free loopback port");
        let port = listener.local_addr().expect("an address").port();
        let state = Arc::new(Mutex::new(State {
            config,
            device: None,
        }));
        let hits: Arc<Mutex<Vec<Hit>>> = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::clone(&state);
        let recorded = Arc::clone(&hits);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let state = Arc::clone(&shared);
                let hits = Arc::clone(&recorded);
                std::thread::spawn(move || serve(stream, &state, &hits));
            }
        });
        Self { port, state, hits }
    }

    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn hits(&self) -> Vec<Hit> {
        self.hits.lock().expect("hits").clone()
    }

    fn clear(&self) {
        self.hits.lock().expect("hits").clear();
    }

    fn with_config(&self, change: impl FnOnce(&mut Config)) {
        change(&mut self.state.lock().expect("state").config);
    }

    fn device(&self) -> Option<DeviceRow> {
        self.state.lock().expect("state").device.clone()
    }
}

fn serve(mut stream: TcpStream, state: &Mutex<State>, hits: &Mutex<Vec<Hit>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let Ok(read) = stream.read(&mut chunk) else {
            return;
        };
        if read == 0 {
            return;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(at) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break at + 4;
        }
        if buffer.len() > 1 << 20 {
            return;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split(' ');
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or("/").to_owned();
    let mut background = false;
    let mut if_none_match = None;
    let mut length = 0usize;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().to_owned();
        match name.to_ascii_lowercase().as_str() {
            "x-ns-background" => background = value == "1",
            "if-none-match" => if_none_match = Some(value),
            "content-length" => length = value.parse().unwrap_or(0),
            _ => {}
        }
    }
    while buffer.len() < header_end + length {
        let Ok(read) = stream.read(&mut chunk) else {
            return;
        };
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let body = String::from_utf8_lossy(&buffer[header_end..header_end + length]).into_owned();

    let mut state = state.lock().expect("state");
    let (status, headers, response) = route(&method, &path, &body, &if_none_match, &mut state);
    drop(state);
    // Record the hit BEFORE answering. A test clears this log and then asserts a
    // count, and the client comes back the moment it has the response: writing
    // first let the assertion run while this thread was still on its way to the
    // `push`, so a request that DID happen read as zero (measured: 1 of 200
    // turns, 4 of 200 pinned to one core; the CI job that failed 22/23 was this).
    // Pushing first makes the hit happen-before the response, which is the only
    // order in which the log a client can see is the log it caused.
    let response_bytes = response.len();
    hits.lock().expect("hits").push(Hit {
        method,
        path,
        background,
        if_none_match,
        request_body: body,
        status,
        response_bytes,
    });
    write_answer(&mut stream, status, &headers, &response);
}

fn write_answer(stream: &mut TcpStream, status: u16, headers: &[(String, String)], body: &[u8]) {
    let mut response = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        reason(status),
        body.len()
    );
    for (name, value) in headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }
    response.push_str("Connection: close\r\n\r\n");
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        304 => "Not Modified",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Status",
    }
}

fn json_answer(status: u16, value: Value) -> (u16, Vec<(String, String)>, Vec<u8>) {
    (
        status,
        vec![("Cache-Control".to_owned(), "no-store".to_owned())],
        serde_json::to_vec(&value).expect("JSON"),
    )
}

/// One request's answer: `(status, headers, body)`.
fn route(
    method: &str,
    path: &str,
    body: &str,
    if_none_match: &Option<String>,
    state: &mut State,
) -> (u16, Vec<(String, String)>, Vec<u8>) {
    match (method, path) {
        ("OPTIONS", "/api/lan/status") => (200, Vec::new(), Vec::new()),
        ("GET", "/api/lan/status") => {
            if state.config.tamper == Tamper::Missing {
                return json_answer(404, json!({"error": "route inconnue", "code": "not_found"}));
            }
            let enabled = state.config.tamper != Tamper::Disabled;
            json_answer(
                200,
                json!({
                    "enabled": enabled,
                    "subject": SUBJECT_HEX,
                    "keys": [{"kid": KID_HEX, "pub": LAN_PUB_HEX, "until": null}],
                    "credential_days": state.config.credential_days,
                    "revocation_serial": state.config.revocation_serial,
                }),
            )
        }
        ("POST", "/api/lan/enroll") => enroll(body, state),
        ("GET", "/api/lan/roster") => {
            let etag = roster_etag(state.config.serial, state.config.revocation_serial);
            if etag_matches(if_none_match.as_deref(), &etag) {
                return (
                    304,
                    vec![
                        ("ETag".to_owned(), etag),
                        ("Cache-Control".to_owned(), "private, no-cache".to_owned()),
                    ],
                    Vec::new(),
                );
            }
            let (roster, revocations) = signed_trust(state);
            let (_, headers, bytes) = json_answer(
                200,
                json!({
                    "roster": roster,
                    "revocations": revocations,
                    "etag": etag,
                }),
            );
            let mut headers = headers;
            headers.push(("ETag".to_owned(), etag));
            (200, headers, bytes)
        }
        ("DELETE", "/api/lan/devices/self") => (204, Vec::new(), Vec::new()),
        _ => json_answer(404, json!({"error": "route inconnue", "code": "not_found"})),
    }
}

/// The server's `_etag_matches`, reproduced: a comma-separated list, `W/` weak
/// prefixes and `*`.
fn etag_matches(header: Option<&str>, etag: &str) -> bool {
    let Some(header) = header else {
        return false;
    };
    header.split(',').any(|candidate| {
        let candidate = candidate.trim();
        let candidate = candidate.strip_prefix("W/").unwrap_or(candidate);
        candidate == etag || candidate == "*"
    })
}

/// `neezer/lan.py#roster_etag`, reproduced.
fn roster_etag(serial: u64, revocation_serial: u64) -> String {
    format!("\"{serial}-{revocation_serial}\"")
}

/// `POST /api/lan/enroll`: verify the possession proof, then issue (or renew) the
/// credential. `neezer/lan.py#enroll`'s contract, minus the database.
fn enroll(body: &str, state: &mut State) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let refuse = |status: u16, code: &str, message: &str| {
        json_answer(status, json!({"error": message, "code": code}))
    };
    if state.config.tamper == Tamper::StaleProof {
        return refuse(
            403,
            "stale_proof",
            "La preuve de l'appareil est trop ancienne (300 s au plus).",
        );
    }
    if state.config.tamper == Tamper::RefuseEnroll {
        return refuse(403, "bad_proof", "La preuve de l'appareil est invalide.");
    }
    let Ok(request) = serde_json::from_str::<Value>(body) else {
        return refuse(400, "invalid_body", "Requête invalide.");
    };
    let pubkey_hex = request["pubkey"].as_str().unwrap_or_default().to_owned();
    let name = request["name"].as_str().unwrap_or_default().to_owned();
    let platform = request["platform"].as_str().unwrap_or_default().to_owned();
    let at = request["at"].as_i64().unwrap_or_default();
    let proof = request["proof"].as_str().unwrap_or_default().to_owned();
    let device_id = request["device_id"].as_str().map(str::to_owned);
    let Ok(pubkey) = <[u8; 32]>::try_from(hex(&pubkey_hex)) else {
        return refuse(400, "bad_pubkey", "La clé de l'appareil est invalide.");
    };
    let Some(proof) = base64_decode(&proof) else {
        return refuse(403, "bad_proof", "La preuve de l'appareil est invalide.");
    };

    // The proof signs the SUBJECT the status answered, the pubkey in its hex
    // spelling and `at` in DECIMAL TEXT — the note's message, byte for byte.
    let mut message = ENROLL_DOMAIN.to_vec();
    message.extend_from_slice(SUBJECT_HEX.as_bytes());
    message.extend_from_slice(pubkey_hex.as_bytes());
    message.extend_from_slice(at.to_string().as_bytes());
    if UnparsedPublicKey::new(&ED25519, pubkey)
        .verify(&message, &proof)
        .is_err()
    {
        return refuse(403, "bad_proof", "La preuve de l'appareil est invalide.");
    }

    match (&state.device, device_id.as_deref()) {
        (Some(existing), Some(id)) if to_hex(&existing.id) == id => {
            state.device = Some(DeviceRow {
                id: existing.id,
                pubkey,
                name,
                platform,
            });
        }
        (Some(_), Some(_)) => {
            // A device id this account does not own: refused like an unknown one.
            return refuse(404, "unknown_device", "Cet appareil n'existe pas.");
        }
        _ => {
            state.device = Some(DeviceRow {
                id: fixed::<12>(DEVICE_ID_HEX),
                pubkey,
                name,
                platform,
            });
        }
    }
    let device = state.device.clone().expect("just written");
    let subject = fixed::<16>(SUBJECT_HEX);
    let other_subject = [0xEE; 16];
    let other_pub = [0xAA; 32];
    let credential = signed_credential(
        &fixed::<8>(KID_HEX),
        match state.config.tamper {
            Tamper::OtherSubject => &other_subject,
            _ => &subject,
        },
        &device.id,
        match state.config.tamper {
            Tamper::OtherDevicePub => &other_pub,
            _ => &device.pubkey,
        },
        at,
        at + state.config.credential_days * DAY,
    );
    let (roster, revocations) = signed_trust(state);
    json_answer(
        200,
        json!({
            "device_id": to_hex(&device.id),
            "credential": base64(&credential),
            "roster": roster,
            "revocations": revocations,
        }),
    )
}

/// The signed roster envelope and the signed revocation list, as the two fields
/// every answer carries.
fn signed_trust(state: &State) -> (Value, String) {
    let serial = state.config.serial;
    let issued = NOW;
    let subject = match state.config.tamper {
        Tamper::OtherRosterSubject => "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        _ => SUBJECT_HEX,
    };
    let body = format!(
        r#"{{"v":1,"kid":"{KID_HEX}","subject":"{subject}","serial":{serial},"issued":{issued},"expires":{},"shares_to":[{{"subject":"01010101010101010101010101010101","all":false,"playlists":[{{"id":"pl1","title":"Route","tracks":["t1","t2"]}}]}}],"trusted_by":[{{"subject":"02020202020202020202020202020202","name":"Alice"}}]}}"#,
        issued + state.config.credential_days * DAY
    );
    let signature = match state.config.tamper {
        // A signature by a key the device does not trust: the roster must be
        // refused, and the whole enrolment with it.
        Tamper::ForgedRoster => sign_with(&device_signer(), ROSTER_DOMAIN, body.as_bytes()),
        _ => sign_with(&lan_signer(), ROSTER_DOMAIN, body.as_bytes()),
    };
    let roster = json!({ "body": base64(body.as_bytes()), "sig": base64(&signature) });
    let revocations = base64(&signed_revocations(
        &fixed::<8>(KID_HEX),
        state.config.revocation_serial,
        issued,
        &state.config.revoked,
    ));
    (roster, revocations)
}

fn signed_credential(
    kid: &[u8; 8],
    subject: &[u8; 16],
    device_id: &[u8; 12],
    device_pub: &[u8; 32],
    not_before: i64,
    not_after: i64,
) -> Vec<u8> {
    let mut body = Vec::with_capacity(90);
    body.extend_from_slice(b"NZLC");
    body.push(1);
    body.extend_from_slice(kid);
    body.extend_from_slice(subject);
    body.extend_from_slice(device_id);
    body.extend_from_slice(device_pub);
    body.extend_from_slice(&not_before.to_be_bytes());
    body.extend_from_slice(&not_after.to_be_bytes());
    body.push(0b11);
    assert_eq!(body.len(), 90, "le corps fait 90 octets");
    let signature = sign_with(&lan_signer(), CREDENTIAL_DOMAIN, &body);
    let mut credential = body;
    credential.extend_from_slice(&signature);
    assert_eq!(credential.len(), 154, "l'attestation fait 154 octets");
    credential
}

fn signed_revocations(kid: &[u8; 8], serial: u64, issued: i64, keys: &[[u8; 32]]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(b"NZLR");
    body.push(1);
    body.extend_from_slice(kid);
    body.extend_from_slice(&serial.to_be_bytes());
    body.extend_from_slice(&issued.to_be_bytes());
    body.extend_from_slice(&(keys.len() as u32).to_be_bytes());
    let mut sorted = keys.to_vec();
    sorted.sort();
    for key in &sorted {
        body.extend_from_slice(key);
    }
    let signature = sign_with(&lan_signer(), REVOCATIONS_DOMAIN, &body);
    let mut list = body;
    list.extend_from_slice(&signature);
    list
}

// --- A client, a store and a runtime -----------------------------------------

struct Fixture {
    dir: PathBuf,
    fake: Fake,
    api: Api,
    runtime: tokio::runtime::Runtime,
}

impl Fixture {
    fn new(tag: &str, config: Config) -> Self {
        let dir = temp_dir(tag);
        let fake = Fake::start(config);
        let api = Api::builder(fake.base())
            .session(Arc::new(SessionJar::at(dir.join("session.json"))))
            .build()
            .expect("a client");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("nz-lan-enroll-test")
            .enable_all()
            .build()
            .expect("a runtime");
        Self {
            dir,
            fake,
            api,
            runtime,
        }
    }

    fn store(&self) -> Store {
        Store::open(&self.dir).expect("the LAN store")
    }

    /// The probe the app itself makes before it starts anything (`Feature::Lan`),
    /// then the enrolment: the first login's three requests.
    fn first_login(&self, now: i64) -> Result<nz_lan::enroll::Enrolled, LanError> {
        let probe = self.runtime.block_on(self.api.has_route("/lan/status"));
        assert_eq!(probe, Some(true), "le serveur a la route");
        self.runtime
            .block_on(ensure_enrolled(&self.api, &self.dir, "PC", "desktop", now))
    }

    fn enrolled(&self) -> nz_lan::enroll::Enrolled {
        self.first_login(NOW).expect("the first enrolment")
    }

    fn run<F, T>(&self, future: F) -> T
    where
        F: std::future::Future<Output = T>,
    {
        self.runtime.block_on(future)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("ln7-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("un répertoire de test");
    dir
}

fn trust_of(enrolled: &nz_lan::enroll::Enrolled) -> SharedTrust {
    Arc::new(RwLock::new(enrolled.trust.clone()))
}

fn read_trust(trust: &SharedTrust) -> TrustState {
    trust.read().expect("trust").clone()
}

// --- The acceptance ----------------------------------------------------------

/// **First login: 3 requests, ≤ 6 KB.** The probe (`OPTIONS`), the status and the
/// enrolment — the roster comes WITH the enrolment, so there is no fourth call.
/// Both LAN calls are on a background lane (`X-NS-Background: 1`), which is what
/// keeps them behind the tapped track (rule 1).
#[test]
fn first_login_enrols_in_three_requests() {
    let fixture = Fixture::new("first", Config::default());
    let enrolled = fixture.first_login(NOW).expect("the first enrolment");

    let hits = fixture.fake.hits();
    assert_eq!(hits.len(), 3, "3 requêtes : {hits:#?}");
    assert!(hits[0].is("OPTIONS", "/api/lan/status"), "{:#?}", hits[0]);
    assert!(hits[1].is("GET", "/api/lan/status"), "{:#?}", hits[1]);
    assert!(hits[2].is("POST", "/api/lan/enroll"), "{:#?}", hits[2]);
    for hit in &hits[1..] {
        assert!(
            hit.background,
            "{} {} doit céder à la lecture",
            hit.method, hit.path
        );
    }

    let bytes_in: usize = hits.iter().map(|hit| hit.request_body.len()).sum();
    let bytes_out: usize = hits.iter().map(|hit| hit.response_bytes).sum();
    println!(
        "LN7 premier login : 3 requêtes, {bytes_in} octets de corps de requête + {bytes_out} octets de réponse"
    );
    assert!(
        bytes_in + bytes_out <= 6 * 1024,
        "≤ 6 Ko : {bytes_in} + {bytes_out} octets"
    );

    // The credential is the server's, over this device's key, and valid now.
    let identity = &enrolled.identity;
    assert_eq!(identity.cred.subject, reference_subject());
    assert_eq!(identity.cred.device_pub, identity.key.public());
    assert_eq!(identity.cred.not_after, NOW + CREDENTIAL_DAYS * DAY);
    assert_eq!(enrolled.until, identity.cred.not_after);
    assert_eq!(enrolled.trust.subject, reference_subject());
    // The roster came with the enrolment: one share, one truster.
    let roster = enrolled.trust.roster.as_ref().expect("le roster");
    assert_eq!(roster.serial, 42);
    assert_eq!(roster.shares_to.len(), 1);
    assert_eq!(roster.shares_to[0].playlists[0].tracks, vec!["t1", "t2"]);
    assert_eq!(roster.trusted_by[0].name, "Alice");
    assert_eq!(enrolled.trust.revocations.len(), 1);

    // The key file is the device's and private; the store holds exactly the five
    // rows this lot writes.
    let key = std::fs::read(fixture.dir.join(KEY_FILE)).expect("la clé");
    assert_eq!(key.len(), 32, "une graine de 32 octets");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(fixture.dir.join(KEY_FILE))
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "device.key est privé");
    }
    let store = fixture.store();
    let mut keys = store.trust_keys();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "ca:56475aa75463474c".to_owned(),
            "credential".to_owned(),
            "revocations:56475aa75463474c".to_owned(),
            "roster".to_owned(),
            "roster.etag".to_owned(),
        ],
        "cinq lignes, pas une de plus"
    );
    assert_eq!(
        store.trust("credential").expect("the credential").len(),
        154,
        "l'attestation est gardée en octets bruts"
    );
    assert_eq!(
        String::from_utf8(store.trust("roster.etag").expect("the etag")).expect("utf8"),
        "\"42-7\"",
        "l'ETag est dérivé des deux numéros reçus : la première synchro est déjà conditionnelle"
    );
    let device = fixture.fake.device().expect("une ligne d'appareil");
    assert_eq!(device.name, "PC");
    assert_eq!(device.platform, "desktop");
}

/// **A second start within a day costs 0 requests**: the cached credential is good
/// for 30 days at the first login, so nothing is asked and nothing moves.
#[test]
fn a_second_start_within_a_day_makes_zero_requests() {
    let fixture = Fixture::new("cached", Config::default());
    let first = fixture.enrolled();
    let key_before = std::fs::read(fixture.dir.join(KEY_FILE)).expect("la clé");
    fixture.fake.clear();

    let again = fixture
        .run(ensure_enrolled(
            &fixture.api,
            &fixture.dir,
            "PC",
            "desktop",
            NOW + 3_600,
        ))
        .expect("le démarrage suivant");
    assert_eq!(fixture.fake.hits().len(), 0, "0 requête dans les 24 h");
    assert_eq!(again.until, first.until);
    assert_eq!(again.identity.cred.raw(), first.identity.cred.raw());
    assert_eq!(
        std::fs::read(fixture.dir.join(KEY_FILE)).expect("la clé"),
        key_before,
        "aucune rotation sans renouvellement"
    );
    assert!(again.trust.roster.is_some());
}

/// **After a day, one conditional roster request answered `304`**: the validator
/// the enrolment derived is exactly the server's, so the sync costs one request,
/// zero body bytes, and says "nothing moved".
#[test]
fn a_day_later_one_conditional_roster_request_answers_304() {
    let fixture = Fixture::new("sync", Config::default());
    let first = fixture.enrolled();
    let store = fixture.store();
    let trust = trust_of(&first);
    fixture.fake.clear();

    let moved = fixture
        .run(sync_trust(&fixture.api, &store, &trust))
        .expect("la synchro quotidienne");
    assert!(!moved, "rien n'a bougé");
    let hits = fixture.fake.hits();
    assert_eq!(hits.len(), 1, "une seule requête : {hits:#?}");
    assert!(hits[0].is("GET", "/api/lan/roster"));
    assert!(hits[0].background, "la synchro cède à la lecture");
    assert_eq!(
        hits[0].if_none_match.as_deref(),
        Some("\"42-7\""),
        "l'If-None-Match est le validateur du serveur"
    );
    assert_eq!(hits[0].status, 304);
    assert_eq!(hits[0].response_bytes, 0, "304 : zéro octet de corps");

    // A second sync is the same one request, not a new full roster.
    fixture.fake.clear();
    assert!(
        !fixture
            .run(sync_trust(&fixture.api, &store, &trust))
            .expect("la deuxième synchro")
    );
    assert_eq!(fixture.fake.hits().len(), 1);
    assert_eq!(fixture.fake.hits()[0].status, 304);
}

/// **A roster that moved is fetched, verified and stored**, and the NEW validator
/// is kept: the next sync is conditional on it.
#[test]
fn a_roster_that_moved_is_fetched_verified_and_stored() {
    let fixture = Fixture::new("moved", Config::default());
    let first = fixture.enrolled();
    let store = fixture.store();
    let trust = trust_of(&first);
    fixture.fake.with_config(|config| config.serial = 43);
    fixture.fake.clear();

    let moved = fixture
        .run(sync_trust(&fixture.api, &store, &trust))
        .expect("la synchro");
    assert!(moved, "le roster a bougé");
    let hits = fixture.fake.hits();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].status, 200);
    assert_eq!(
        read_trust(&trust).roster.as_ref().expect("roster").serial,
        43
    );
    assert_eq!(
        String::from_utf8(store.trust("roster.etag").expect("etag")).expect("utf8"),
        "\"43-7\"",
        "le validateur du serveur est celui qu'on renvoie ensuite"
    );

    // And now it is conditional again, on the NEW validator.
    fixture.fake.clear();
    assert!(
        !fixture
            .run(sync_trust(&fixture.api, &store, &trust))
            .expect("la synchro suivante")
    );
    assert_eq!(
        fixture.fake.hits()[0].if_none_match.as_deref(),
        Some("\"43-7\"")
    );
    assert_eq!(fixture.fake.hits()[0].status, 304);
}

/// **< 23 days left renews, with a NEW key and the SAME device row.** The
/// threshold is exact: 23 days plus a second still answers offline (0 requests);
/// exactly 23 days renews (status + enrol = 2 requests).
#[test]
fn a_credential_with_less_than_23_days_left_renews_with_a_new_key() {
    let fixture = Fixture::new("renew", Config::default());
    assert_eq!(
        nz_lan::enroll::RENEW_BELOW_SECONDS,
        RENEW_BELOW,
        "le seuil de renouvellement est celui de la note : 23 jours"
    );
    let first = fixture.enrolled();
    let first_pub = first.identity.key.public();
    let device = fixture.fake.device().expect("une ligne");
    assert_eq!(to_hex(&device.id), DEVICE_ID_HEX);
    fixture.fake.clear();

    // One second over the threshold: still cached, still 0 requests.
    let just_above = NOW + CREDENTIAL_DAYS * DAY - RENEW_BELOW - 1;
    let cached = fixture
        .run(ensure_enrolled(
            &fixture.api,
            &fixture.dir,
            "PC",
            "desktop",
            just_above,
        ))
        .expect("encore valable");
    assert_eq!(
        fixture.fake.hits().len(),
        0,
        "0 requête au-dessus du seuil de 23 jours"
    );
    assert_eq!(cached.until, first.until);

    // Exactly 23 days left: renew.
    let at_threshold = NOW + CREDENTIAL_DAYS * DAY - RENEW_BELOW;
    let renewed = fixture
        .run(ensure_enrolled(
            &fixture.api,
            &fixture.dir,
            "PC",
            "desktop",
            at_threshold,
        ))
        .expect("le renouvellement");
    let hits = fixture.fake.hits();
    assert_eq!(hits.len(), 2, "status + enroll : {hits:#?}");
    assert!(hits[0].is("GET", "/api/lan/status"));
    assert!(hits[1].is("POST", "/api/lan/enroll"));
    let request: Value = serde_json::from_str(&hits[1].request_body).expect("le corps");
    assert_eq!(
        request["device_id"].as_str(),
        Some(DEVICE_ID_HEX),
        "un renouvellement garde la ligne de l'appareil (et sa place sous le plafond de 20)"
    );
    assert_ne!(
        renewed.identity.key.public(),
        first_pub,
        "le renouvellement fait tourner la clé"
    );
    assert_eq!(
        renewed.identity.cred.device_pub,
        renewed.identity.key.public(),
        "l'attestation porte la NOUVELLE clé"
    );
    assert_eq!(
        to_hex(&renewed.identity.cred.device_id),
        DEVICE_ID_HEX,
        "le même appareil"
    );
    assert_eq!(renewed.until, at_threshold + CREDENTIAL_DAYS * DAY);
    assert_eq!(
        fixture.fake.device().expect("une ligne").pubkey,
        renewed.identity.key.public(),
        "le serveur a remplacé la clé de la même ligne"
    );
}

/// **A server without the route leaves no trace**: `Unsupported`, and the data
/// directory still holds no key and no database — nothing to clean up later.
#[test]
fn a_server_without_the_route_writes_nothing() {
    let fixture = Fixture::new(
        "missing",
        Config {
            tamper: Tamper::Missing,
            ..Config::default()
        },
    );
    let error = fixture
        .run(ensure_enrolled(
            &fixture.api,
            &fixture.dir,
            "PC",
            "desktop",
            NOW,
        ))
        .expect_err("un serveur sans la route");
    assert_eq!(error, LanError::Unsupported);
    assert!(!fixture.dir.join(KEY_FILE).exists(), "aucune clé");
    assert!(
        !fixture.dir.join("lan.sqlite").exists(),
        "aucune base : rien à nettoyer"
    );
}

/// **The admin switched the feature off**: `Disabled`, and again nothing written.
#[test]
fn a_disabled_feature_writes_nothing() {
    let fixture = Fixture::new(
        "disabled",
        Config {
            tamper: Tamper::Disabled,
            ..Config::default()
        },
    );
    let error = fixture
        .run(ensure_enrolled(
            &fixture.api,
            &fixture.dir,
            "PC",
            "desktop",
            NOW,
        ))
        .expect_err("la fonction est éteinte");
    assert_eq!(error, LanError::Disabled);
    assert!(!fixture.dir.join(KEY_FILE).exists());
    assert!(!fixture.dir.join("lan.sqlite").exists());
}

/// **An offline start loads the cached credential and trust**, with no request:
/// that is what makes the LAN work on a trip.
#[test]
fn an_offline_start_reads_the_cached_credential_and_trust() {
    let fixture = Fixture::new("offline", Config::default());
    let first = fixture.enrolled();
    let store = fixture.store();
    fixture.fake.clear();

    let offline = load_cached(&fixture.dir, &store, NOW + 3_600).expect("l'état hors ligne");
    assert_eq!(offline.identity.cred.raw(), first.identity.cred.raw());
    assert_eq!(offline.identity.key.public(), first.identity.key.public());
    assert_eq!(offline.until, first.until);
    assert_eq!(offline.trust.subject, reference_subject());
    let roster = offline.trust.roster.as_ref().expect("le roster");
    assert_eq!(roster.shares_to.len(), 1);
    assert_eq!(roster.trusted_by[0].name, "Alice");
    assert_eq!(
        offline
            .trust
            .revocations
            .get(&key_of())
            .expect("la liste")
            .serial,
        7
    );
    assert_eq!(fixture.fake.hits().len(), 0, "aucune requête hors ligne");

    // Past its end, the same credential is not served: `Expired` is not "load it
    // anyway".
    assert!(load_cached(&fixture.dir, &store, NOW + CREDENTIAL_DAYS * DAY + 601).is_none());
}

/// **The enrol proof signs the subject, the key and the DECIMAL instant.** The
/// control is the fake server: it verifies exactly the note's message, so a
/// successful enrolment already proves it. This test also proves the other
/// reading is refused — an 8-byte `at` would not verify.
#[test]
fn the_enrol_proof_binds_the_subject_the_key_and_the_decimal_instant() {
    let fixture = Fixture::new("proof", Config::default());
    let enrolled = fixture.enrolled();
    let request: Value =
        serde_json::from_str(&fixture.fake.hits()[2].request_body).expect("le corps");
    let at = request["at"].as_i64().expect("at");
    assert_eq!(at, NOW);
    let pubkey_hex = to_hex(&enrolled.identity.key.public());
    assert_eq!(request["pubkey"].as_str(), Some(pubkey_hex.as_str()));
    let proof = base64_decode(request["proof"].as_str().expect("proof")).expect("base64");

    let mut message = ENROLL_DOMAIN.to_vec();
    message.extend_from_slice(SUBJECT_HEX.as_bytes());
    message.extend_from_slice(pubkey_hex.as_bytes());
    message.extend_from_slice(at.to_string().as_bytes());
    assert_eq!(
        UnparsedPublicKey::new(&ED25519, enrolled.identity.key.public()).verify(&message, &proof),
        Ok(()),
        "la preuve est celle de la note : sujet ‖ clé ‖ at décimal"
    );

    let mut binary_at = ENROLL_DOMAIN.to_vec();
    binary_at.extend_from_slice(SUBJECT_HEX.as_bytes());
    binary_at.extend_from_slice(pubkey_hex.as_bytes());
    binary_at.extend_from_slice(&at.to_be_bytes());
    assert!(
        UnparsedPublicKey::new(&ED25519, enrolled.identity.key.public())
            .verify(&binary_at, &proof)
            .is_err(),
        "un `at` en 8 octets big-endian n'est pas la preuve que le serveur accepte"
    );
}

/// **A credential signed for another subject is refused**, and nothing is stored:
/// the device must not act as somebody else.
#[test]
fn a_credential_signed_for_another_subject_is_refused() {
    let fixture = Fixture::new(
        "other-subject",
        Config {
            tamper: Tamper::OtherSubject,
            ..Config::default()
        },
    );
    let error = fixture
        .run(ensure_enrolled(
            &fixture.api,
            &fixture.dir,
            "PC",
            "desktop",
            NOW,
        ))
        .expect_err("une attestation pour un autre sujet");
    assert_eq!(error, LanError::Mismatch);
    assert!(
        fixture.store().trust("credential").is_none(),
        "aucune attestation n'est écrite"
    );
}

/// **A credential for another device key is refused**: the server would otherwise
/// be able to hand a device a credential it cannot prove.
#[test]
fn a_credential_for_another_device_key_is_refused() {
    let fixture = Fixture::new(
        "other-key",
        Config {
            tamper: Tamper::OtherDevicePub,
            ..Config::default()
        },
    );
    let error = fixture
        .run(ensure_enrolled(
            &fixture.api,
            &fixture.dir,
            "PC",
            "desktop",
            NOW,
        ))
        .expect_err("une attestation pour une autre clé");
    assert_eq!(error, LanError::Mismatch);
    assert!(fixture.store().trust("credential").is_none());
}

/// **A forged roster refuses the WHOLE enrolment**: the credential is fine, but a
/// device that trusted a roster it could not verify would act on another person's
/// shares. Nothing at all is written.
#[test]
fn a_roster_with_a_bad_signature_refuses_the_enrolment_and_writes_nothing() {
    let fixture = Fixture::new(
        "forged-roster",
        Config {
            tamper: Tamper::ForgedRoster,
            ..Config::default()
        },
    );
    let error = fixture
        .run(ensure_enrolled(
            &fixture.api,
            &fixture.dir,
            "PC",
            "desktop",
            NOW,
        ))
        .expect_err("un roster forgé");
    assert_eq!(error, LanError::Refused);
    let store = fixture.store();
    assert!(
        store.trust("credential").is_none(),
        "l'attestation n'est pas écrite avant que TOUT soit vérifié"
    );
    assert!(store.trust("roster").is_none());
    assert!(
        store.trust_keys().iter().all(|key| !key.starts_with("ca:")),
        "aucune clé de serveur n'est enregistrée non plus"
    );
}

/// **A roster about somebody else refuses the enrolment**, even when its signature
/// is the server's: a device that trusted it would act on another person's shares.
#[test]
fn a_roster_about_another_subject_refuses_the_enrolment() {
    let fixture = Fixture::new(
        "other-roster",
        Config {
            tamper: Tamper::OtherRosterSubject,
            ..Config::default()
        },
    );
    let error = fixture
        .run(ensure_enrolled(
            &fixture.api,
            &fixture.dir,
            "PC",
            "desktop",
            NOW,
        ))
        .expect_err("un roster d'un autre sujet");
    assert_eq!(error, LanError::Mismatch);
    let store = fixture.store();
    assert!(store.trust("credential").is_none(), "rien n'est écrit");
    assert!(store.trust("roster").is_none());
}

/// **A roster about somebody else is refused at sync too**, and what the device
/// already trusted is left alone.
#[test]
fn a_roster_about_another_subject_is_refused_at_sync() {
    let fixture = Fixture::new("other-roster-sync", Config::default());
    let first = fixture.enrolled();
    let store = fixture.store();
    let trust = trust_of(&first);
    fixture.fake.with_config(|config| {
        config.tamper = Tamper::OtherRosterSubject;
        config.serial = 43;
    });
    fixture.fake.clear();

    let error = fixture
        .run(sync_trust(&fixture.api, &store, &trust))
        .expect_err("un roster d'un autre sujet à la synchro");
    assert_eq!(error, LanError::Mismatch);
    assert_eq!(
        read_trust(&trust).roster.as_ref().expect("roster").serial,
        42,
        "le roster détenu n'a pas bougé"
    );
    assert_eq!(
        String::from_utf8(store.trust("roster.etag").expect("etag")).expect("utf8"),
        "\"42-7\"",
        "ni le validateur"
    );
}

/// **A refused renewal keeps the device a node.** The server answered the status
/// and then refused the enrolment: the key this device still holds must survive, or
/// a single bad answer would cost it the credential it was renewing (rule 3: what
/// works keeps working). The retry, once the server accepts, rotates the key on the
/// same device row.
#[test]
fn a_refused_renewal_keeps_the_device_a_node() {
    let fixture = Fixture::new("renew-refused", Config::default());
    let first = fixture.enrolled();
    let store = fixture.store();
    let key_before = std::fs::read(fixture.dir.join(KEY_FILE)).expect("la clé");
    fixture.fake.clear();
    fixture
        .fake
        .with_config(|config| config.tamper = Tamper::RefuseEnroll);

    let at_threshold = NOW + CREDENTIAL_DAYS * DAY - RENEW_BELOW;
    let error = fixture
        .run(ensure_enrolled(
            &fixture.api,
            &fixture.dir,
            "PC",
            "desktop",
            at_threshold,
        ))
        .expect_err("le serveur refuse l'enrôlement");
    assert!(matches!(error, LanError::Server(_)), "{error:?}");
    assert_eq!(
        std::fs::read(fixture.dir.join(KEY_FILE)).expect("la clé"),
        key_before,
        "la clé de l'appareil n'a pas tourné : le renouvellement échoué ne coûte rien"
    );
    assert!(
        !fixture.dir.join(STAGED_DIR).exists(),
        "aucune clé en attente ne reste"
    );
    let still = load_cached(&fixture.dir, &store, at_threshold).expect("toujours un nœud");
    assert_eq!(still.identity.cred.raw(), first.identity.cred.raw());
    assert_eq!(still.identity.key.public(), first.identity.key.public());

    // The server accepts again: the renewal goes through, on the same device row.
    fixture
        .fake
        .with_config(|config| config.tamper = Tamper::None);
    fixture.fake.clear();
    let renewed = fixture
        .run(ensure_enrolled(
            &fixture.api,
            &fixture.dir,
            "PC",
            "desktop",
            at_threshold,
        ))
        .expect("le renouvellement qui aboutit");
    assert_ne!(renewed.identity.key.public(), first.identity.key.public());
    assert_eq!(
        read_trust(&trust_of(&first)).subject,
        renewed.identity.cred.subject
    );
    assert!(!fixture.dir.join(STAGED_DIR).exists());
    assert_eq!(
        to_hex(&renewed.identity.cred.device_id),
        DEVICE_ID_HEX,
        "le renouvellement retarde garde le même appareil"
    );
}

/// **An answer older than what the device holds never moves it backwards.** A proxy
/// or a server that answers a stale roster (serial 42 after a gossiped 43) must not
/// replace what this device trusts: only the validator is taken, so the next sync is
/// conditional on the server's own current answer.
#[test]
fn an_older_roster_never_moves_the_device_backwards() {
    let fixture = Fixture::new("backwards", Config::default());
    let first = fixture.enrolled();
    let store = fixture.store();
    let trust = trust_of(&first);
    // The server moves to 43: the device takes it.
    fixture.fake.with_config(|config| config.serial = 43);
    assert!(
        fixture
            .run(sync_trust(&fixture.api, &store, &trust))
            .expect("la synchro à 43")
    );
    assert_eq!(
        read_trust(&trust).roster.as_ref().expect("roster").serial,
        43
    );
    // A stale answer (42) comes back: nothing moves, the validator is kept.
    fixture.fake.with_config(|config| config.serial = 42);
    fixture.fake.clear();
    assert!(
        !fixture
            .run(sync_trust(&fixture.api, &store, &trust))
            .expect("la réponse périmée"),
        "un serial plus bas ne change rien"
    );
    assert_eq!(
        fixture.fake.hits()[0].status,
        200,
        "le validateur ne matchait plus"
    );
    assert_eq!(
        read_trust(&trust).roster.as_ref().expect("roster").serial,
        43,
        "l'état en mémoire n'a pas reculé"
    );
    let stored = Roster::decode_verify(
        &store.trust("roster").expect("le roster stocké"),
        &lan_keys(),
    )
    .expect("le roster stocké est vérifiable");
    assert_eq!(stored.serial, 43, "ni le roster stocké");
    assert_eq!(
        String::from_utf8(store.trust("roster.etag").expect("etag")).expect("utf8"),
        "\"42-7\"",
        "le validateur du serveur, lui, est pris"
    );
}

/// **A proof refused for its date says the device's clock is wrong** — the one
/// refusal a person can act on.
#[test]
fn a_stale_proof_says_the_device_clock_is_wrong() {
    let fixture = Fixture::new(
        "skew",
        Config {
            tamper: Tamper::StaleProof,
            ..Config::default()
        },
    );
    let error = fixture
        .run(ensure_enrolled(
            &fixture.api,
            &fixture.dir,
            "PC",
            "desktop",
            NOW,
        ))
        .expect_err("une preuve périmée");
    let cause = error.cause_fr();
    assert!(
        cause.contains("l'heure de l'appareil est décalée"),
        "la cause dit quoi corriger : {cause}"
    );
}

/// **Gossip: a higher serial is verified, stored and USED; a forged, older or
/// unknown-key list is ignored.** "Used" is measured on `load_cached`: the same
/// credential stops loading once its device key is revoked.
#[test]
fn gossip_takes_a_higher_revocation_list_and_uses_it() {
    let fixture = Fixture::new("gossip-rev", Config::default());
    let first = fixture.enrolled();
    let store = fixture.store();
    let trust = trust_of(&first);
    let device_pub = first.identity.key.public();
    // The key set a device that enrolled against this server holds.
    assert_eq!(lan_keys().get(&reference_kid()), Some(&lan_public()));

    // A peer teaches a list that revokes THIS device, serial 9.
    let higher = signed_revocations(&fixed::<8>(KID_HEX), 9, NOW, &[device_pub]);
    let accepted = gossip::accept(
        &trust,
        &store,
        &Control::Revocations {
            blob: base64(&higher),
        },
        true,
    )
    .expect("une liste valide");
    assert!(accepted, "une liste plus récente est acceptée");
    assert_eq!(
        store
            .trust("revocations:56475aa75463474c")
            .expect("la liste stockée"),
        higher,
        "elle est stockée TELLE QUELLE, octet pour octet"
    );
    assert_eq!(read_trust(&trust).revocations[&key_of()].serial, 9);
    // Used: the device it revokes can no longer load its own credential.
    assert!(
        load_cached(&fixture.dir, &store, NOW + 60).is_none(),
        "l'appareil révoqué n'est plus un nœud"
    );

    // The same list again, and an older one: silence, not a refusal.
    for serial in [9, 8] {
        let blob = base64(&signed_revocations(&fixed::<8>(KID_HEX), serial, NOW, &[]));
        assert!(
            !gossip::accept(&trust, &store, &Control::Revocations { blob }, true).expect("ignorée"),
            "serial {serial} n'écrase pas 9"
        );
    }

    // Forged by another key: an error, and nothing moves.
    let forged = {
        let mut body = higher[..higher.len() - 64].to_vec();
        let signature = sign_with(&device_signer(), REVOCATIONS_DOMAIN, &body);
        body.extend_from_slice(&signature);
        body
    };
    assert!(
        gossip::accept(
            &trust,
            &store,
            &Control::Revocations {
                blob: base64(&forged)
            },
            true
        )
        .is_err(),
        "une signature d'une autre clé est refusée"
    );
    assert_eq!(
        read_trust(&trust).revocations[&key_of()].serial,
        9,
        "la liste en mémoire n'a pas bougé"
    );

    // Signed by a KID this device does not know: refused too.
    let unknown_kid = signed_revocations(&[0x99; 8], 99, NOW, &[]);
    assert!(
        gossip::accept(
            &trust,
            &store,
            &Control::Revocations {
                blob: base64(&unknown_kid)
            },
            true
        )
        .is_err(),
        "un kid inconnu n'est pas une liste"
    );

    // Over the cap: refused before it is decoded.
    let oversized = "A".repeat((1 << 20) + 4);
    assert!(
        gossip::accept(
            &trust,
            &store,
            &Control::Revocations { blob: oversized },
            true
        )
        .is_err(),
        "un blob au-delà du plafond est refusé"
    );
}

/// **Gossip: a roster only from my own subject, only forward.** A roster about
/// somebody else, an equal serial, or a frame from another subject is not taken.
#[test]
fn gossip_takes_a_roster_only_from_my_subject_and_only_forward() {
    let fixture = Fixture::new("gossip-roster", Config::default());
    let first = fixture.enrolled();
    let store = fixture.store();
    let trust = trust_of(&first);

    let control = |serial: u64, subject: &str| {
        let body = format!(
            r#"{{"v":1,"kid":"{KID_HEX}","subject":"{subject}","serial":{serial},"issued":{NOW},"expires":{},"shares_to":[],"trusted_by":[{{"subject":"03030303030303030303030303030303","name":"Bob"}}]}}"#,
            NOW + CREDENTIAL_DAYS * DAY
        );
        let signature = sign_with(&lan_signer(), ROSTER_DOMAIN, body.as_bytes());
        Control::Roster {
            blob: base64(
                &serde_json::to_vec(
                    &json!({"body": base64(body.as_bytes()), "sig": base64(&signature)}),
                )
                .expect("JSON"),
            ),
        }
    };

    // A newer roster of my subject: accepted and stored as the server's envelope.
    assert!(
        gossip::accept(&trust, &store, &control(43, SUBJECT_HEX), true).expect("valide"),
        "serial 43 > 42 : accepté"
    );
    assert_eq!(
        read_trust(&trust).roster.as_ref().expect("roster").serial,
        43
    );
    assert!(store.trust("roster").is_some());

    // The same serial again: ignored. An older serial: ignored.
    assert!(
        !gossip::accept(&trust, &store, &control(43, SUBJECT_HEX), true).expect("ignoré"),
        "un serial égal n'écrase rien"
    );
    assert!(!gossip::accept(&trust, &store, &control(41, SUBJECT_HEX), true).expect("ignoré"));
    // Another subject, correctly signed: ignored (only MY roster may be trusted).
    assert!(
        !gossip::accept(
            &trust,
            &store,
            &control(44, "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"),
            true
        )
        .expect("ignoré"),
        "le roster d'un autre sujet n'est pas le mien"
    );
    // My subject, but the connection is not one of my devices.
    assert!(
        !gossip::accept(&trust, &store, &control(45, SUBJECT_HEX), false).expect("ignoré"),
        "pas de bavardage de roster avec un ami"
    );
    assert_eq!(
        read_trust(&trust).roster.as_ref().expect("roster").serial,
        43,
        "rien de tout cela n'a bougé l'état"
    );

    // A roster signed by another key is an error, not an ignore.
    let body = format!(
        r#"{{"v":1,"kid":"{KID_HEX}","subject":"{SUBJECT_HEX}","serial":46,"issued":{NOW},"expires":{},"shares_to":[],"trusted_by":[]}}"#,
        NOW + CREDENTIAL_DAYS * DAY
    );
    let signature = sign_with(&device_signer(), ROSTER_DOMAIN, body.as_bytes());
    let forged = Control::Roster {
        blob: base64(
            &serde_json::to_vec(
                &json!({"body": base64(body.as_bytes()), "sig": base64(&signature)}),
            )
            .expect("JSON"),
        ),
    };
    assert!(gossip::accept(&trust, &store, &forged, true).is_err());
}

/// **`offers` says what to send, and only what is newer.** Nothing to teach: no
/// frame; a roster only to one's own subject.
#[test]
fn offers_only_sends_what_the_peer_does_not_have() {
    let fixture = Fixture::new("offers", Config::default());
    let first = fixture.enrolled();

    // The peer knows nothing: one revocation list and one roster.
    let frames = gossip::offers(&first.trust, 0, true, None);
    assert_eq!(frames.len(), 2, "{frames:#?}");
    assert!(matches!(frames[0], Control::Revocations { .. }));
    assert!(matches!(frames[1], Control::Roster { .. }));

    // The peer has both: nothing to send.
    assert!(gossip::offers(&first.trust, 7, true, Some(42)).is_empty());
    // A friend gets the revocation list, never the roster (it names other people).
    let frames = gossip::offers(&first.trust, 0, false, None);
    assert_eq!(frames.len(), 1);
    assert!(matches!(frames[0], Control::Revocations { .. }));
    // A peer that already has a NEWER revocation list gets nothing for it.
    assert!(
        gossip::offers(&first.trust, 8, false, None).is_empty(),
        "un serial plus haut chez le pair n'apprend rien"
    );
}

/// **Logout wipes the key, the trust rows and revokes the device when online.**
/// Offline, the local wipe still happens and no request is made.
#[test]
fn forget_wipes_everything_and_revokes_once() {
    let fixture = Fixture::new("forget", Config::default());
    let first = fixture.enrolled();
    assert!(first.trust.roster.is_some());
    assert!(
        fixture.api.is_online(),
        "le serveur vient d'answérer : le client se sait en ligne"
    );
    let store = fixture.store();
    assert!(!store.trust_keys().is_empty());
    assert!(fixture.dir.join(KEY_FILE).is_file());
    fixture.fake.clear();

    fixture.run(forget(Some(&fixture.api), &fixture.dir, &store));
    let hits = fixture.fake.hits();
    assert_eq!(hits.len(), 1, "DELETE /devices/self : {hits:#?}");
    assert!(hits[0].is("DELETE", "/api/lan/devices/self"));
    assert!(
        !hits[0].background,
        "une déconnexion est l'action de la personne (`User`), pas du fond"
    );
    assert!(
        !fixture.dir.join(KEY_FILE).exists(),
        "device.key est effacé"
    );
    assert!(store.trust_keys().is_empty(), "aucune ligne de confiance");
    assert!(load_cached(&fixture.dir, &store, NOW + 60).is_none());

    // Offline: the local wipe happens and no request leaves.
    let second = fixture.enrolled();
    assert!(second.trust.roster.is_some());
    fixture.fake.clear();
    fixture.run(forget(None, &fixture.dir, &fixture.store()));
    assert_eq!(fixture.fake.hits().len(), 0, "aucune requête hors ligne");
    assert!(!fixture.dir.join(KEY_FILE).exists());
    assert!(fixture.store().trust_keys().is_empty());
}

/// **An unreachable server is a cause, not a panic**, and still writes nothing.
#[test]
fn an_unreachable_server_is_a_cause_not_a_panic() {
    let dir = temp_dir("unreachable");
    // Port 1 on loopback: nothing listens there.
    let api = Api::builder("http://127.0.0.1:1")
        .session(Arc::new(SessionJar::at(dir.join("session.json"))))
        .build()
        .expect("a client");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("a runtime");
    let error = runtime
        .block_on(ensure_enrolled(&api, &dir, "PC", "desktop", NOW))
        .expect_err("aucun serveur");
    assert!(matches!(error, LanError::Server(_)), "{error:?}");
    assert!(error.cause_fr().starts_with("Le serveur a refusé"));
    assert!(!dir.join(KEY_FILE).exists());
    let _ = std::fs::remove_dir_all(dir);
}

/// The `Api` classifications this lot reads, held here so a change there is caught
/// by THIS lot's suite too.
#[test]
fn the_api_failures_this_lot_reads_are_the_ones_it_expects() {
    assert_eq!(
        ApiError::Offline.cause_fr(),
        "le serveur ne répond pas (réseau)"
    );
    // An empty roster answer is a decode failure, never a silent success.
    assert!(serde_json::from_str::<nz_model::lan::LanRosterAnswer>("").is_err());
}
