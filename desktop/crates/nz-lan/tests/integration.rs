//! LN11's acceptance: the four adapters end to end
//! (docs/agents/lan-nodes.md, *Playback: peer first*, *Downloads and playlists*,
//! *Choosing the source*, *Rule 1 on the LAN*, and the LN11 lot).
//!
//! Two (or three) REAL `Lan` nodes on loopback, a real `nz-audio` `Engine` on
//! `new_offline`'s pull-driven sink, a real `nz-offline` worker, and real FLAC and
//! Opus fixtures made by ffmpeg: nothing here is mocked at the audio or the
//! transport level. The "server" the app's opener wraps is a finished file served
//! whole or from a byte range — byte for byte what `GET /api/stream` answers for an
//! archived FLAC — and it COUNTS its opens, which is how "the peer served it" and
//! "no LAN request was made" are asserted.
//!
//! | Property (LN11's *Acceptance*) | Measured by |
//! |---|---|
//! | a track only the peer has plays from the peer, tap → sound ≤ 300 ms, the play cache holds the PEER's quality | `a_track_only_the_peer_has_plays_from_the_peer_under_its_quality` |
//! | an Opus 64-only peer is skipped when the server is reachable, and plays offline | `a_peer_with_only_opus_64_is_skipped_online_and_used_offline` |
//! | no peer has it → 0 LAN requests, the inner opener at once | `a_track_no_peer_has_goes_straight_to_the_server` |
//! | a peer stalls mid-track on `User` → the server continues at the byte received | `a_stalled_peer_hands_the_rest_of_the_track_to_the_server` |
//! | a seek is a byte range from the SAME peer; `ready()` costs 0 requests | `a_seek_reopens_the_same_peer_at_the_byte_and_ready_costs_nothing` |
//! | a 50-track playlist from the peer that has it complete: 50 files, labels, pinned sleeves; also with the server unreachable | `a_fifty_track_playlist_comes_from_the_peer_that_has_it_complete` |
//! | rule 1: playing while downloading from a peer and serving another → 0 underruns, first byte unchanged ±10 % | `playing_while_downloading_and_serving_never_underruns` |
//!
//! Every wait is on an event: the node's own `watch`, the worker's `watch`, the
//! engine's event channel, a stall's timeout. The deadlines are nets under the
//! assertions, never the assertion itself. Nothing touches the person's display,
//! sound or network: loopback only, every data directory under the crate's
//! `target/` (never `/tmp`: tmpfs is RAM).
//!
//! Helpers copied from `tests/service.rs` (the fake server, its keys, the node
//! fixture) are this file's own copy on purpose: an integration test binary links
//! nothing from another one.

#![allow(dead_code)]
#![allow(clippy::await_holding_lock)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

use aws_lc_rs::signature::{ED25519, Ed25519KeyPair, UnparsedPublicKey};
use serde_json::{Value, json};

use nz_api::Quality;
use nz_api::{Api, SessionJar};
use nz_lan::keys::DeviceKey;
use nz_lan::proto::{self, Request, Response, WireQuality};
use nz_lan::service::{DiscoveryKind, Lan, LanConfig};
use nz_lan::transport::{Endpoint, Identity, PeerConn, TrustState};
use nz_lan::types::{Kid, Source, Subject, Urgency};

/// The server's LAN key seed: bytes `0x00..=0x1f` (pub `03a107…31b8`).
const LAN_SEED: [u8; 32] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
];

const LAN_PUB_HEX: &str = "03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8";
const KID_HEX: &str = "56475aa75463474c";
const SUBJECT_HEX: &str = "00112233445566778899aabbccddeeff";

const CREDENTIAL_DOMAIN: &[u8] = b"neezer-lan/1 credential\x00";
const REVOCATIONS_DOMAIN: &[u8] = b"neezer-lan/1 revocations\x00";
const ROSTER_DOMAIN: &[u8] = b"neezer-lan/1 roster\x00";
const ENROLL_DOMAIN: &[u8] = b"neezer-lan/1 enroll\x00";

const DAY: i64 = 86_400;
/// The default credential lifetime (`lan_credential_days`).
const CREDENTIAL_DAYS: i64 = 30;

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or(0)
}

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

fn lan_public() -> [u8; 32] {
    fixed::<32>(LAN_PUB_HEX)
}

fn reference_kid() -> Kid {
    Kid(fixed::<8>(KID_HEX))
}

fn reference_subject() -> Subject {
    Subject(fixed::<16>(SUBJECT_HEX))
}

fn sign_with(signer: &Ed25519KeyPair, domain: &[u8], message: &[u8]) -> Vec<u8> {
    let mut signed = domain.to_vec();
    signed.extend_from_slice(message);
    signer.sign(&signed).as_ref().to_vec()
}

/// The note's credential layout, 154 bytes, signed by the test server's LAN key.
fn signed_credential(
    subject: &[u8; 16],
    device_id: &[u8; 12],
    device_pub: &[u8; 32],
    not_before: i64,
    not_after: i64,
) -> Vec<u8> {
    let mut body = Vec::with_capacity(90);
    body.extend_from_slice(b"NZLC");
    body.push(1);
    body.extend_from_slice(&reference_kid().0);
    body.extend_from_slice(subject);
    body.extend_from_slice(device_id);
    body.extend_from_slice(device_pub);
    body.extend_from_slice(&not_before.to_be_bytes());
    body.extend_from_slice(&not_after.to_be_bytes());
    body.push(0b11);
    assert_eq!(body.len(), 90, "le corps de l'attestation fait 90 octets");
    let signature = sign_with(&lan_signer(), CREDENTIAL_DOMAIN, &body);
    body.extend_from_slice(&signature);
    assert_eq!(body.len(), 154, "l'attestation fait 154 octets");
    body
}

/// The note's revocation list: `NZLR`, sorted unique keys, signed.
fn signed_revocations(serial: u64, issued: i64, keys: &[[u8; 32]]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(b"NZLR");
    body.push(1);
    body.extend_from_slice(&reference_kid().0);
    body.extend_from_slice(&serial.to_be_bytes());
    body.extend_from_slice(&issued.to_be_bytes());
    body.extend_from_slice(&(keys.len() as u32).to_be_bytes());
    let mut sorted = keys.to_vec();
    sorted.sort();
    for key in &sorted {
        body.extend_from_slice(key);
    }
    let signature = sign_with(&lan_signer(), REVOCATIONS_DOMAIN, &body);
    body.extend_from_slice(&signature);
    body
}

/// The `{"body","sig"}` roster envelope `cred.rs` verifies over the exact bytes.
fn roster_envelope(serial: u64, issued: i64, days: i64) -> Value {
    let body = format!(
        r#"{{"v":1,"kid":"{KID_HEX}","subject":"{SUBJECT_HEX}","serial":{serial},"issued":{issued},"expires":{},"shares_to":[],"trusted_by":[]}}"#,
        issued + days * DAY
    );
    let signature = sign_with(&lan_signer(), ROSTER_DOMAIN, body.as_bytes());
    json!({ "body": base64(body.as_bytes()), "sig": base64(&signature) })
}

// --- the fake server ---------------------------------------------------------

/// One request as the server saw it, with the instant it was answered: the rule 1
/// measure reads that instant, so a lane's gate is OBSERVED, never guessed.
#[derive(Clone, Debug)]
struct Hit {
    method: String,
    path: String,
    background: bool,
    status: u16,
    answered: Instant,
}

impl Hit {
    fn is(&self, method: &str, path: &str) -> bool {
        self.method == method && self.path == path
    }
}

#[derive(Clone, Debug)]
struct DeviceRow {
    id: [u8; 12],
    pubkey: [u8; 32],
    name: String,
    /// How many credentials this row has been issued: a renewal's window then
    /// starts on a strictly later instant, so `key_until` is a precise event even
    /// when two enrolments land in the same second (a real server's window is
    /// `now + days`, which is the same thing one second later).
    issued: i64,
}

#[derive(Debug)]
struct ServerState {
    /// `true` once the server answers `404` to every LAN route (an older server, or
    /// an admin who removed the feature): `LanError::Unsupported`.
    missing: bool,
    credential_days: i64,
    roster_serial: u64,
    revocation_serial: u64,
    revoked: Vec<[u8; 32]>,
    devices: Vec<DeviceRow>,
}

struct Fake {
    port: u16,
    state: Arc<Mutex<ServerState>>,
    hits: Arc<Mutex<Vec<Hit>>>,
}

impl Fake {
    fn start() -> Fake {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a free loopback port");
        let port = listener.local_addr().expect("an address").port();
        let state = Arc::new(Mutex::new(ServerState {
            missing: false,
            credential_days: CREDENTIAL_DAYS,
            roster_serial: 42,
            revocation_serial: 7,
            revoked: Vec::new(),
            devices: Vec::new(),
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
        Fake { port, state, hits }
    }

    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn hits(&self) -> Vec<Hit> {
        self.hits.lock().expect("hits").clone()
    }

    fn count(&self, method: &str, path: &str) -> usize {
        self.hits()
            .iter()
            .filter(|hit| hit.is(method, path))
            .count()
    }

    fn with_state<T>(&self, change: impl FnOnce(&mut ServerState) -> T) -> T {
        change(&mut self.state.lock().expect("state"))
    }

    /// The device row the server holds for the node that enrolled with this name.
    fn device(&self, name: &str) -> Option<DeviceRow> {
        self.with_state(|state| {
            state
                .devices
                .iter()
                .find(|device| device.name == name)
                .cloned()
        })
    }

    fn devices(&self) -> usize {
        self.with_state(|state| state.devices.len())
    }

    fn revoke(&self, pubkey: &[u8; 32]) {
        self.with_state(|state| {
            state.revoked.push(*pubkey);
            state.revocation_serial += 1;
        });
    }

    fn remove_feature(&self) {
        self.with_state(|state| state.missing = true);
    }

    fn set_credential_days(&self, days: i64) {
        self.with_state(|state| state.credential_days = days);
    }
}

fn serve(mut stream: TcpStream, state: &Mutex<ServerState>, hits: &Mutex<Vec<Hit>>) {
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
    // Taken BEFORE the answer is written: a lane's quiet window is measured from
    // when the request was answered, which is its conservative side.
    let answered = Instant::now();
    write_answer(&mut stream, status, &headers, &response);
    hits.lock().expect("hits").push(Hit {
        method,
        path,
        background,
        status,
        answered,
    });
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
        409 => "Conflict",
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

fn route(
    method: &str,
    path: &str,
    body: &str,
    if_none_match: &Option<String>,
    state: &mut ServerState,
) -> (u16, Vec<(String, String)>, Vec<u8>) {
    if state.missing {
        return json_answer(
            404,
            json!({ "error": "route inconnue", "code": "not_found" }),
        );
    }
    match (method, path) {
        ("OPTIONS", "/api/lan/status") => (200, Vec::new(), Vec::new()),
        ("GET", "/api/lan/status") => json_answer(
            200,
            json!({
                "enabled": true,
                "subject": SUBJECT_HEX,
                "keys": [{ "kid": KID_HEX, "pub": LAN_PUB_HEX, "until": null }],
                "credential_days": state.credential_days,
                "revocation_serial": state.revocation_serial,
            }),
        ),
        ("POST", "/api/lan/enroll") => enroll(body, state),
        ("GET", "/api/lan/roster") => {
            let etag = roster_etag(state.roster_serial, state.revocation_serial);
            if etag_matches(if_none_match.as_deref(), &etag) {
                return (304, vec![("ETag".to_owned(), etag)], Vec::new());
            }
            let issued = now();
            let (_, mut headers, bytes) = json_answer(
                200,
                json!({
                    "roster": roster_envelope(state.roster_serial, issued, CREDENTIAL_DAYS),
                    "revocations": base64(&signed_revocations(state.revocation_serial, issued, &state.revoked)),
                    "etag": etag,
                }),
            );
            headers.push(("ETag".to_owned(), etag));
            (200, headers, bytes)
        }
        ("DELETE", "/api/lan/devices/self") => (204, Vec::new(), Vec::new()),
        ("GET", "/api/lan/devices") => json_answer(200, json!([])),
        _ => json_answer(
            404,
            json!({ "error": "route inconnue", "code": "not_found" }),
        ),
    }
}

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

fn roster_etag(serial: u64, revocation_serial: u64) -> String {
    format!("\"{serial}-{revocation_serial}\"")
}

/// `POST /api/lan/enroll`: verify the possession proof, then issue (or renew) the
/// credential of the key that asked. `neezer/lan.py#enroll`'s contract, minus the
/// database.
fn enroll(body: &str, state: &mut ServerState) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let refuse = |status: u16, code: &str, message: &str| {
        json_answer(status, json!({ "error": message, "code": code }))
    };
    let Ok(request) = serde_json::from_str::<Value>(body) else {
        return refuse(400, "invalid_body", "Requête invalide.");
    };
    let pubkey_hex = request["pubkey"].as_str().unwrap_or_default().to_owned();
    let name = request["name"].as_str().unwrap_or_default().to_owned();
    let at = request["at"].as_i64().unwrap_or_default();
    let proof = request["proof"].as_str().unwrap_or_default().to_owned();
    let device_id = request["device_id"].as_str().map(str::to_owned);
    let Ok(pubkey) = <[u8; 32]>::try_from(hex(&pubkey_hex)) else {
        return refuse(400, "bad_pubkey", "La clé de l'appareil est invalide.");
    };
    let Some(proof) = base64_decode(&proof) else {
        return refuse(403, "bad_proof", "La preuve de l'appareil est invalide.");
    };
    // `"neezer-lan/1 enroll\0" ‖ subject_hex ‖ pubkey_hex ‖ str(at)` — `at` DECIMAL.
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

    // A renewal keeps the device row (its `device_id`) and rotates its key, so a
    // device never takes a second slot under the 20-device cap.
    match device_id.and_then(|id| {
        state
            .devices
            .iter()
            .position(|device| to_hex(&device.id) == id)
    }) {
        Some(index) => {
            state.devices[index].pubkey = pubkey;
            state.devices[index].name = name;
        }
        None => {
            if state.devices.len() >= 20 {
                return refuse(409, "too_many_devices", "Trop d'appareils pour ce compte.");
            }
            let mut id = [0u8; 12];
            id[0] = state.devices.len() as u8 + 1;
            id[1..5].copy_from_slice(&pubkey[..4]);
            state.devices.push(DeviceRow {
                id,
                pubkey,
                name,
                issued: 0,
            });
        }
    }
    let device = {
        let index = state
            .devices
            .iter()
            .position(|device| device.pubkey == pubkey)
            .expect("the device row was just written");
        state.devices[index].issued += 1;
        state.devices[index].clone()
    };
    let issued = now();
    let credential = signed_credential(
        &reference_subject().0,
        &device.id,
        &device.pubkey,
        at,
        at + state.credential_days * DAY + device.issued,
    );
    json_answer(
        200,
        json!({
            "device_id": to_hex(&device.id),
            "credential": base64(&credential),
            "roster": roster_envelope(state.roster_serial, issued, CREDENTIAL_DAYS),
            "revocations": base64(&signed_revocations(state.revocation_serial, issued, &state.revoked)),
        }),
    )
}

// --- one node, and its data --------------------------------------------------

/// A throwaway cache + data directory under the crate's `target/`, never `/tmp`.
struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("ln10-{tag}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("fixture dir");
        Fixture { root }
    }

    fn cache(&self, tag: &str) -> nz_cache::Cache {
        let root = self.root.join(tag);
        std::fs::create_dir_all(&root).expect("node dir");
        nz_cache::Cache::open_with_cache_dir(root.join("data"), root.join("cache")).expect("cache")
    }
}

/// One complete download through `nz-cache`'s own API: what a node's inventory is
/// made of.
fn download(cache: &nz_cache::Cache, track: &str, bytes: &[u8]) {
    let id = nz_model::TrackId::new(track);
    let downloads = cache.downloads();
    downloads
        .add_set(
            "1",
            "album",
            "900",
            "Un album",
            &[nz_cache::DownloadTrack {
                track: id.clone(),
                quality: nz_model::Quality::Flac,
            }],
        )
        .expect("add set");
    let partial = downloads
        .reserve("1", &id, nz_model::Quality::Flac)
        .expect("reserve");
    std::fs::write(&partial, bytes).expect("bytes");
    downloads
        .complete("1", &id, bytes.len() as u64)
        .expect("complete");
}

/// One running node, and what a test needs to talk to it.
struct Node {
    lan: Lan,
    cache: nz_cache::Cache,
}

impl Drop for Node {
    fn drop(&mut self) {
        // `shutdown` is idempotent, so an explicit one in the test body costs
        // nothing — and a test that PANICS still stops its threads here.
        self.lan.shutdown();
    }
}

impl Node {
    fn port(&self) -> u16 {
        self.lan.snapshot().port
    }

    fn peers(&self) -> usize {
        self.lan.snapshot().peers.len()
    }
}

/// The `Api` a node shares with its test: one client, one set of lanes, so a
/// User-lane request in a test and the node's Bulk work take the SAME gate.
fn api_for(fake: &Fake, dir: &Path) -> Arc<Api> {
    std::fs::create_dir_all(dir).expect("api dir");
    let api = Api::builder(fake.base())
        .session(Arc::new(SessionJar::at(dir.join("session.json"))))
        .build()
        .expect("a client");
    // The account whose downloads the index serves (`nz-cache`'s owner).
    api.set_owner(Some("1".to_owned()));
    Arc::new(api)
}

/// Starts one node with its OWN cache (two nodes of one test must not share a
/// library) and the downloads it already has.
fn start_node(
    fixture: &Fixture,
    tag: &str,
    name: &str,
    api: &Arc<Api>,
    downloads: &[(&str, &[u8])],
) -> Node {
    let cache = fixture.cache(tag);
    for (track, bytes) in downloads {
        download(&cache, track, bytes);
    }
    let lan = Lan::start(
        LanConfig {
            data_dir: fixture.root.join(tag).join("lan"),
            device_name: name.to_owned(),
            platform: "desktop",
            app_version: "0.1.0".to_owned(),
            discovery: DiscoveryKind::External,
            port: 0,
        },
        Arc::clone(api),
        cache.clone(),
    )
    .unwrap_or_else(|error| panic!("{tag} : le nœud démarre ({error})"));
    Node { lan, cache }
}

/// Waits until `check` is true, waking on the node's own `watch` — never on a
/// sleep. The deadline is the net under the assertion, not the assertion.
async fn until(what: &str, lan: &Lan, check: impl FnMut() -> bool) {
    until_within(what, lan, Duration::from_secs(40), check).await
}

/// The same with a shorter net: an operation that must happen AT ONCE (closing a
/// connection when the app goes away, an answer to a refusal) gets a bound far
/// under quinn's 30 s idle timeout, so "it happened later by itself" cannot pass
/// for "it happened because the node did it".
async fn until_within(what: &str, lan: &Lan, within: Duration, mut check: impl FnMut() -> bool) {
    let mut events = lan.events();
    let deadline = tokio::time::Instant::now() + within;
    loop {
        if check() {
            return;
        }
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(
            !left.is_zero(),
            "{what} : pas arrivé en {within:?} (état : {:?})",
            lan.snapshot()
        );
        let _ = tokio::time::timeout(left, events.changed()).await;
    }
}

/// Waits until the node is enrolled (`key_until` set) and its endpoint is bound.
async fn enrolled(node: &Node) {
    until("l'inscription", &node.lan, || {
        let snapshot = node.lan.snapshot();
        snapshot.key_until.is_some() && snapshot.port != 0
    })
    .await;
}

/// Waits until `node` holds exactly `count` peers.
async fn peers_are(node: &Node, count: usize) {
    peers_are_within(node, count, Duration::from_secs(40)).await;
}

async fn peers_are_within(node: &Node, count: usize, within: Duration) {
    until_within(&format!("{count} pair(s)"), &node.lan, within, || {
        node.peers() == count
    })
    .await;
}

/// A device key written by the real key path (`0600`, atomic) under `target/`, with
/// the credential the test server signs for it (the same subject: an own device).
fn fake_identity(tag: &str) -> Arc<Identity> {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("ln11-keys-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("un répertoire de clé");
    let key = DeviceKey::load_or_create(&dir).expect("une clé d'appareil");
    let raw = signed_credential(
        &reference_subject().0,
        &[0x01; 12],
        &key.public(),
        now() - 60,
        now() + 86_400,
    );
    let cred = nz_lan::cred::Credential::decode(&raw).expect("une attestation de test");
    Arc::new(Identity { key, cred })
}

/// The trust a device of the test server's subject holds: the server's LAN key and
/// an empty revocation list.
fn own_trust() -> nz_lan::transport::SharedTrust {
    Arc::new(RwLock::new(TrustState {
        subject: reference_subject(),
        keys: nz_lan::cred::TrustKeys::from_status(&[(reference_kid(), lan_public())]),
        revocations: HashMap::from([(
            reference_kid(),
            nz_lan::cred::Revocations::empty(reference_kid()),
        )]),
        roster: None,
    }))
}

/// The median of measured milliseconds.
fn median_ms(mut values: Vec<f64>) -> f64 {
    values.sort_by(|left, right| left.partial_cmp(right).expect("ordered"));
    let len = values.len();
    if len % 2 == 1 {
        values[len / 2]
    } else {
        (values[len / 2 - 1] + values[len / 2]) / 2.0
    }
}

// --- LN11: the fixtures ------------------------------------------------------

use std::sync::mpsc::Receiver;

use nz_audio::{
    ByteStream, Engine, EngineConfig, EngineEvent, Lane as AudioLane, OfflineSink,
    Quality as AudioQuality, SourceError, StreamOpener, StreamReady,
};
use nz_lan::downloader::PeerFirstDownloader;
use nz_lan::opener::PeerFirstOpener;
use nz_lan::sets::{PeerSetKey, download_peer_sets};
use nz_offline::{Downloader, Fetched, Offline, Outcome, Progress, Stop};

/// The nodes AND the engines of this file share one process: one at a time, so a
/// measurement is not another test's load.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn only_one() -> std::sync::MutexGuard<'static, ()> {
    ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn ffmpeg(args: &[&str]) {
    let status = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .status()
        .expect("ffmpeg est installé (le CI desktop l'installe)");
    assert!(status.success(), "ffmpeg {args:?} a échoué");
}

/// A FLAC of a cosine at `freq`: the first sample is NOT zero, so "the first
/// audible frame" is a real event and not a sine's own silent start.
fn tone_flac(path: &Path, seconds: f64) {
    let source = format!("aevalsrc=0.5*cos(2*PI*440*t):s=48000:d={seconds}:c=stereo");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &source,
        "-c:a",
        "flac",
        path.to_str().unwrap(),
    ]);
}

/// The same audio as Ogg Opus 64 kbit/s: the rung the default tolerance refuses
/// while the server is reachable.
fn tone_opus64(path: &Path, seconds: f64) {
    let source = format!("aevalsrc=0.5*cos(2*PI*440*t):s=48000:d={seconds}:c=stereo");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &source,
        "-c:a",
        "libopus",
        "-b:a",
        "64k",
        path.to_str().unwrap(),
    ]);
}

/// A 1×1 PNG: what a cover store sniffs as `png` (it never validates more).
const PNG: &[u8] = &[
    0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H', b'D', b'R',
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0a, b'I', b'D', b'A', b'T', 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', 0xae,
    0x42, 0x60, 0x82,
];

/// One complete download at an exact quality: what a node's inventory and a
/// peer's `Sets` are made of.
fn download_at(cache: &nz_cache::Cache, track: &str, quality: nz_model::Quality, bytes: &[u8]) {
    let id = nz_model::TrackId::new(track);
    let downloads = cache.downloads();
    downloads
        .add_set(
            "1",
            "album",
            "900",
            "Un album",
            &[nz_cache::DownloadTrack {
                track: id.clone(),
                quality,
            }],
        )
        .expect("add set");
    let partial = downloads.reserve("1", &id, quality).expect("reserve");
    std::fs::write(&partial, bytes).expect("bytes");
    downloads
        .complete("1", &id, bytes.len() as u64)
        .expect("complete");
}

/// A playlist of `tracks`, every member complete on disk: the set a peer serves
/// whole.
fn seed_playlist(
    cache: &nz_cache::Cache,
    tracks: &[String],
    quality: nz_model::Quality,
    bytes: &[u8],
) {
    let members: Vec<nz_cache::DownloadTrack> = tracks
        .iter()
        .map(|track| nz_cache::DownloadTrack {
            track: nz_model::TrackId::new(track.clone()),
            quality,
        })
        .collect();
    cache
        .downloads()
        .add_set("1", "playlist", "900", "Route", &members)
        .expect("add the playlist");
    for track in tracks {
        let id = nz_model::TrackId::new(track.clone());
        let partial = cache
            .downloads()
            .reserve("1", &id, quality)
            .expect("reserve");
        std::fs::write(&partial, bytes).expect("bytes");
        cache
            .downloads()
            .complete("1", &id, bytes.len() as u64)
            .expect("complete");
    }
}

/// The cached set page a node answers `Meta` and `Sets` from: titles, artists and
/// the sleeve URL, exactly the shape `/api/playlist/<id>` has.
fn seed_playlist_page(cache: &nz_cache::Cache, cover: &str, count: usize) {
    let tracks: Vec<Value> = (0..count)
        .map(|index| {
            json!({
                "deezer_id": format!("40000{:03}", index + 1),
                "title": format!("Titre {}", index + 1),
                "artist": { "name": "Artiste" },
                "album": { "title": "Album", "cover": cover, "deezer_id": "77" },
                "duration": 3,
            })
        })
        .collect();
    let page = json!({
        "playlist": { "id": "900", "title": "Route", "cover": cover, "nb_tracks": count },
        "tracks": tracks,
    });
    cache.put(
        "1",
        "/playlist/900",
        &serde_json::to_string(&page).expect("page"),
    );
}

/// The opens an inner opener has answered, with the byte offset asked: the LAN's
/// own evidence that the server was (or was not) used.
#[derive(Clone, Default)]
struct Opens(Arc<Mutex<Vec<Option<u64>>>>);

impl Opens {
    fn record(&self, from: Option<u64>) {
        self.0.lock().expect("opens").push(from);
    }

    fn list(&self) -> Vec<Option<u64>> {
        self.0.lock().expect("opens").clone()
    }

    fn count(&self) -> usize {
        self.0.lock().expect("opens").len()
    }
}

/// The app's opener in these tests: a finished file, served whole or from a byte
/// range — the "mock stream" of the acceptance, and what a real server answers.
struct FileOpener {
    path: PathBuf,
    opens: Opens,
}

impl FileOpener {
    fn new(path: &Path, opens: Opens) -> FileOpener {
        FileOpener {
            path: path.to_path_buf(),
            opens,
        }
    }

    fn len(&self) -> u64 {
        std::fs::metadata(&self.path)
            .map(|meta| meta.len())
            .unwrap_or(0)
    }
}

impl StreamOpener for FileOpener {
    fn open(
        &self,
        _id: &str,
        _quality: AudioQuality,
        _start: Option<f64>,
        from_byte: Option<u64>,
        _lane: AudioLane,
    ) -> Result<Box<dyn ByteStream>, SourceError> {
        self.opens.record(from_byte);
        let mut file = std::fs::File::open(&self.path).map_err(|_| SourceError::Offline)?;
        let from = from_byte.unwrap_or(0);
        std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(from))
            .map_err(|error| SourceError::Message(error.to_string()))?;
        Ok(Box::new(FileBody {
            file,
            remaining: self.len().saturating_sub(from),
        }))
    }

    fn ready(&self, _id: &str, _quality: AudioQuality) -> Result<StreamReady, SourceError> {
        Ok(StreamReady {
            ready: true,
            ranges: Some("bytes".to_owned()),
            bytes: Some(self.len()),
            cached: false,
            offset: false,
            streaming: false,
        })
    }

    fn local(&self, _id: &str, _quality: AudioQuality) -> Option<PathBuf> {
        None
    }

    fn shutdown(&self) {}
}

struct FileBody {
    file: std::fs::File,
    remaining: u64,
}

impl std::io::Read for FileBody {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.remaining == 0 {
            return Ok(0);
        }
        let want = buf.len().min(self.remaining as usize);
        let read = self.file.read(&mut buf[..want])?;
        self.remaining -= read as u64;
        Ok(read)
    }
}

impl ByteStream for FileBody {
    fn len_hint(&self) -> Option<u64> {
        Some(self.remaining)
    }
}

/// The inner fetcher the acceptance does not want called: a set taken from a peer
/// must not touch the server.
struct DeniedDownloads {
    calls: Opens,
}

impl Downloader for DeniedDownloads {
    fn fetch(
        &self,
        _track: &nz_model::TrackId,
        _quality: Quality,
        _part: &Path,
        _stop: &Stop,
        _progress: &mut dyn Progress,
    ) -> Result<Fetched, Outcome> {
        self.calls.record(None);
        Err(Outcome::Failed(
            "le serveur ne doit pas servir ce lot".to_owned(),
        ))
    }
}

/// One engine on the pull-driven sink (`new_offline`), and its events.
fn offline_engine(
    dir: &Path,
    opener: PeerFirstOpener<FileOpener>,
) -> (Engine, OfflineSink, Receiver<EngineEvent>) {
    let cfg = EngineConfig {
        sample_rate: 48_000,
        spool_dir: dir.join("spool"),
        read_ahead_bytes: 2 * 1024 * 1024,
        seek_poll_limit: Duration::from_secs(5),
        ..EngineConfig::default()
    };
    std::fs::create_dir_all(&cfg.spool_dir).expect("spool");
    let (engine, sink) = Engine::new_offline(cfg, Arc::new(opener));
    let events = engine.events();
    (engine, sink, events)
}

/// Tap → first audible frame, rendered at 1 ms a call like a device.
fn first_audible(_engine: &Engine, sink: &mut OfflineSink, budget: Duration) -> Option<Duration> {
    let started = Instant::now();
    let mut out = vec![0.0f32; 96];
    while started.elapsed() < budget {
        sink.render(&mut out);
        if out.iter().any(|sample| sample.abs() > 1.0e-6) {
            return Some(started.elapsed());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    None
}

/// Pumps the sink until `predicate` matches an event (or the budget expires).
fn wait_event(
    sink: &mut OfflineSink,
    events: &Receiver<EngineEvent>,
    what: &str,
    budget: Duration,
    mut predicate: impl FnMut(&EngineEvent) -> bool,
) -> EngineEvent {
    let deadline = Instant::now() + budget;
    let mut out = vec![0.0f32; 96];
    while Instant::now() < deadline {
        while let Ok(event) = events.try_recv() {
            if predicate(&event) {
                return event;
            }
        }
        sink.render(&mut out);
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("{what} : pas arrivé en {budget:?}");
}

/// Every `Underrun` count the engine has reported so far (it publishes a running
/// total, so the last one is the answer).
fn underruns(events: &Receiver<EngineEvent>) -> u64 {
    let mut last = 0;
    while let Ok(event) = events.try_recv() {
        if let EngineEvent::Underrun { count } = event {
            last = count;
        }
    }
    last
}

/// Waits for the offline worker, on ITS own `watch` (never a sleep).
async fn offline_until(offline: &Offline, what: &str, mut check: impl FnMut() -> bool) {
    let mut events = offline.events();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        if check() {
            return;
        }
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(
            !left.is_zero(),
            "{what} : pas arrivé, état {:?}",
            offline.snapshot().state
        );
        let _ = tokio::time::timeout(left, events.changed()).await;
    }
}

/// Waits until `lan` has replicated a peer's set `ident` COMPLETE with `count`
/// tracks: the state « Télécharger » re-checks against.
async fn peer_set_complete(lan: &Lan, ident: &str, count: usize) {
    until("l'ensemble complet répliqué", lan, || {
        lan.sets()
            .iter()
            .any(|row| row.set.ident == ident && row.set.complete && row.set.tracks.len() == count)
    })
    .await;
}

/// The device key of the only peer a node has (its own `PeerView`).
fn only_peer(lan: &Lan) -> [u8; 32] {
    let peers = lan.snapshot().peers;
    assert_eq!(peers.len(), 1, "un seul pair attendu : {peers:?}");
    peers[0].device_pub
}

// --- the acceptance ----------------------------------------------------------

/// **A track only the peer has plays from the peer**, the play cache then holds it
/// under the peer's quality, and the tap is audible in ≤ 300 ms.
///
/// The inner opener points at a DIFFERENT file (an unreachable server would answer
/// nothing) and counts its opens: the body can only have come from the LAN.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_track_only_the_peer_has_plays_from_the_peer_under_its_quality() {
    let _one = only_one();
    let fixture = Fixture::new("ln11-peer-play");
    let flac = fixture.root.join("tone.flac");
    tone_flac(&flac, 4.0);
    let flac_bytes = std::fs::read(&flac).expect("the fixture");
    // The inner opener points at a DIFFERENT file: if it is ever called, the
    // spool below cannot be the peer's bytes. That, and its 0 opens, is what says
    // "the LAN served it" without reading a counter the driver samples once a
    // second.
    let other = fixture.root.join("other.flac");
    tone_flac(&other, 1.0);

    let fake = Fake::start();
    let a_api = api_for(&fake, &fixture.root.join("a-api"));
    let b_api = api_for(&fake, &fixture.root.join("b-api"));
    let a = start_node(&fixture, "a", "PC", &a_api, &[("30000001", &flac_bytes)]);
    let b = start_node(&fixture, "b", "Téléphone", &b_api, &[]);
    enrolled(&a).await;
    enrolled(&b).await;
    b.lan.found(IpAddr::from([127, 0, 0, 1]), a.port());
    peers_are(&b, 1).await;
    until("A répliqué", &b.lan, || {
        matches!(
            b.lan.decide("30000001", Quality::Flac, Urgency::Play),
            Source::Peer { .. }
        )
    })
    .await;

    let opens = Opens::default();
    let opener = PeerFirstOpener::new(FileOpener::new(&other, opens.clone()), b.lan.clone());
    let (engine, mut sink, events) = offline_engine(&fixture.root.join("b-engine"), opener);
    engine.load("30000001", AudioQuality::Opus320, None);
    engine.play();
    let heard = first_audible(&engine, &mut sink, Duration::from_secs(3))
        .expect("aucun son : le pair n'a pas servi le titre");
    println!("tap → premier échantillon depuis le pair : {heard:?}");
    assert!(heard <= Duration::from_millis(300), "entendu en {heard:?}");

    // The peer served it: no inner open, and the decision says which quality the
    // LAN really had (FLAC, while Opus 320 was asked).
    assert_eq!(
        opens.list(),
        Vec::<Option<u64>>::new(),
        "le serveur a été appelé"
    );
    assert_eq!(
        b.lan.decide("30000001", Quality::Opus320, Urgency::Play),
        Source::Peer {
            peer: only_peer(&b.lan),
            quality: Quality::Flac,
        }
    );

    // The engine says which quality the body REALLY served: the app's
    // `publish_spool` then caches it under THAT key (LN5's rule) — the two lines
    // below are that rule, asserted end to end.
    let completed = wait_event(
        &mut sink,
        &events,
        "la fin du flux",
        Duration::from_secs(10),
        |event| matches!(event, EngineEvent::StreamComplete { .. }),
    );
    let EngineEvent::StreamComplete {
        quality,
        path,
        bytes,
        ..
    } = completed
    else {
        unreachable!("the predicate matched a StreamComplete");
    };
    assert_eq!(quality, AudioQuality::Flac);
    let spool = std::fs::read(&path).expect("le spool");
    assert_eq!(
        spool, flac_bytes,
        "le flux joué n'est pas le fichier du pair"
    );
    let track = nz_model::TrackId::new("30000001");
    let part = b
        .cache
        .play_cache()
        .reserve(&track, nz_model::Quality::Flac)
        .expect("reserve");
    std::fs::copy(&path, &part).expect("spool");
    b.cache
        .play_cache()
        .complete(&track, nz_model::Quality::Flac, bytes)
        .expect("complete");
    assert!(
        b.cache
            .play_cache()
            .lookup(&track, nz_model::Quality::Flac)
            .expect("lookup")
            .is_some(),
        "le cache de lecture ne porte pas la qualité du pair"
    );
    assert!(
        b.cache
            .play_cache()
            .lookup(&track, nz_model::Quality::Opus320)
            .expect("lookup")
            .is_none(),
        "et surtout pas la qualité demandée"
    );
    engine.shutdown();
}

/// **Tolerance: an Opus 64-only peer with the server reachable is skipped, and the
/// server serves; with the server unreachable the peer plays.**
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_peer_with_only_opus_64_is_skipped_online_and_used_offline() {
    let _one = only_one();
    let fixture = Fixture::new("ln11-tolerance");
    let flac = fixture.root.join("tone.flac");
    let opus = fixture.root.join("tone64.opus");
    tone_flac(&flac, 3.0);
    tone_opus64(&opus, 3.0);
    let opus_bytes = std::fs::read(&opus).expect("opus");

    let fake = Fake::start();
    let a_api = api_for(&fake, &fixture.root.join("a-api"));
    let b_api = api_for(&fake, &fixture.root.join("b-api"));
    let a = start_node(&fixture, "a", "PC", &a_api, &[]);
    download_at(&a.cache, "30000002", nz_model::Quality::Opus64, &opus_bytes);
    let b = start_node(&fixture, "b", "Téléphone", &b_api, &[]);
    enrolled(&a).await;
    enrolled(&b).await;
    b.lan.found(IpAddr::from([127, 0, 0, 1]), a.port());
    peers_are(&b, 1).await;
    until("A répliqué", &b.lan, || {
        b.lan
            .snapshot()
            .peers
            .first()
            .is_some_and(|peer| peer.tracks == 1)
    })
    .await;
    assert!(
        !b.lan.settings().tolerated.contains(Quality::Opus64),
        "Opus 64 n'est pas toléré par défaut"
    );

    // Server reachable: the tolerance sends the tap to the server.
    assert_eq!(
        b.lan.decide("30000002", Quality::Opus320, Urgency::Play),
        Source::Server(Quality::Opus320)
    );
    let opens = Opens::default();
    let opener = PeerFirstOpener::new(FileOpener::new(&flac, opens.clone()), b.lan.clone());
    let (engine, mut sink, _events) = offline_engine(&fixture.root.join("b-engine-1"), opener);
    engine.load("30000002", AudioQuality::Opus320, None);
    engine.play();
    first_audible(&engine, &mut sink, Duration::from_secs(3)).expect("le serveur n'a pas servi");
    assert_eq!(opens.count(), 1, "le serveur devait servir le titre");
    engine.shutdown();

    // Server unreachable: the peer's own rung is accepted, and it plays.
    b_api.set_online(false);
    assert_eq!(
        b.lan.decide("30000002", Quality::Opus320, Urgency::Play),
        Source::Peer {
            peer: only_peer(&b.lan),
            quality: Quality::Opus64,
        }
    );
    let opens = Opens::default();
    let opener = PeerFirstOpener::new(FileOpener::new(&flac, opens.clone()), b.lan.clone());
    let (engine, mut sink, events) = offline_engine(&fixture.root.join("b-engine-2"), opener);
    engine.load("30000002", AudioQuality::Opus320, None);
    engine.play();
    first_audible(&engine, &mut sink, Duration::from_secs(3)).expect("le pair n'a pas servi");
    assert_eq!(
        opens.count(),
        0,
        "hors ligne, le serveur ne doit pas être appelé"
    );
    let completed = wait_event(
        &mut sink,
        &events,
        "la fin du flux du pair",
        Duration::from_secs(10),
        |event| matches!(event, EngineEvent::StreamComplete { .. }),
    );
    match completed {
        EngineEvent::StreamComplete { quality, .. } => {
            assert_eq!(quality, AudioQuality::Opus64, "la qualité du pair");
        }
        _ => unreachable!(),
    }
    engine.shutdown();
}

/// **No peer has it: 0 LAN requests, and the inner opener is called at once.**
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_track_no_peer_has_goes_straight_to_the_server() {
    let _one = only_one();
    let fixture = Fixture::new("ln11-no-peer");
    let flac = fixture.root.join("tone.flac");
    tone_flac(&flac, 3.0);
    let flac_bytes = std::fs::read(&flac).expect("the fixture");

    let fake = Fake::start();
    let a_api = api_for(&fake, &fixture.root.join("a-api"));
    let b_api = api_for(&fake, &fixture.root.join("b-api"));
    let a = start_node(&fixture, "a", "PC", &a_api, &[("30000003", &flac_bytes)]);
    let b = start_node(&fixture, "b", "Téléphone", &b_api, &[]);
    enrolled(&a).await;
    enrolled(&b).await;
    b.lan.found(IpAddr::from([127, 0, 0, 1]), a.port());
    peers_are(&b, 1).await;

    // A decision over the replica costs NO request, and that is measured on the
    // decision itself: 1 000 of them in well under a single loopback round trip's
    // worth of time (a QUIC request is 0.3–1 ms, so 1 000 would be ≥ 300 ms). The
    // wire counters cannot be the evidence — the driver samples them once a second
    // and a 1.3 ms loop still straddles a tick (measured in CI).
    let started = Instant::now();
    for _ in 0..1_000 {
        assert_eq!(
            b.lan.decide("30000099", Quality::Opus320, Urgency::Play),
            Source::Server(Quality::Opus320)
        );
    }
    let elapsed = started.elapsed();
    println!("1 000 décisions : {elapsed:?}");
    assert!(
        elapsed < Duration::from_millis(100),
        "une décision est en mémoire : {elapsed:?} pour 1 000"
    );

    let opens = Opens::default();
    let opener = PeerFirstOpener::new(FileOpener::new(&flac, opens.clone()), b.lan.clone());
    let (engine, mut sink, _events) = offline_engine(&fixture.root.join("b-engine"), opener);
    let loaded = Instant::now();
    engine.load("30000099", AudioQuality::Opus320, None);
    engine.play();
    first_audible(&engine, &mut sink, Duration::from_secs(3)).expect("le serveur n'a pas servi");
    assert_eq!(opens.count(), 1, "un seul appel au serveur");
    println!("titre absent partout → serveur : {:?}", loaded.elapsed());
    assert!(
        loaded.elapsed() <= Duration::from_millis(500),
        "l'ouverture interne doit être immédiate : {:?}",
        loaded.elapsed()
    );
    // LANCER une lecture ne prouve rien sur les compteurs : `Lan::wire()` est
    // échantillonné par le pilote une fois par seconde, donc une lecture de 3 s
    // les fait bouger sans qu'un paquet soit émis (mesuré en CI : 9 → 21). Ce qui
    // prouve « 0 requête », c'est la boucle de 1 000 décisions ci-dessus (0 paquet
    // dans la même fenêtre) et l'unique ouverture du serveur.
    engine.shutdown();
}

/// A peer that answers `FetchOk`, sends `first` bytes and then STALLS: the case
/// the person feels.
struct StallingPeer {
    port: u16,
    _serving: tokio::task::JoinHandle<()>,
}

impl StallingPeer {
    fn start(tag: &str, track: &str, flac: Arc<Vec<u8>>, first: usize) -> StallingPeer {
        let identity = fake_identity(tag);
        let endpoint = Arc::new(
            Endpoint::bind(
                &tokio::runtime::Handle::current(),
                0,
                identity,
                own_trust(),
                proto::Hello {
                    name: "PC qui cale".to_owned(),
                    platform: "desktop".to_owned(),
                    app_version: "0.1.0".to_owned(),
                    inventory_version: 1,
                    sets_version: 0,
                },
            )
            .expect("un endpoint de pair"),
        );
        let port = endpoint.port();
        let track = track.to_owned();
        let serving = tokio::spawn(async move {
            loop {
                match endpoint.accept().await {
                    Some(Ok(conn)) => {
                        let flac = Arc::clone(&flac);
                        let track = track.clone();
                        tokio::spawn(async move {
                            stall_serve(conn, track, flac, first).await;
                        });
                    }
                    Some(Err(_)) => continue,
                    None => return,
                }
            }
        });
        StallingPeer {
            port,
            _serving: serving,
        }
    }
}

async fn stall_serve(conn: PeerConn, track: String, flac: Arc<Vec<u8>>, first: usize) {
    while let Some(incoming) = conn.next_incoming().await {
        let head = match &incoming.request {
            Request::Ping => Some(Response::Pong),
            Request::Inventory { page, .. } => Some(Response::InventoryFull {
                version: 1,
                page: *page,
                last: true,
                items: vec![proto::Item {
                    track: track.clone(),
                    copies: vec![proto::Copy {
                        quality: WireQuality::Flac,
                        len: flac.len() as u64,
                    }],
                }],
            }),
            Request::Sets { page } => Some(Response::Sets {
                version: 0,
                page: *page,
                last: true,
                sets: Vec::new(),
            }),
            Request::Fetch { from, .. } => {
                let start = *from as usize;
                let body = StalledBody {
                    bytes: Arc::clone(&flac),
                    cursor: start,
                    first: start + first,
                };
                if incoming
                    .respond(
                        Response::FetchOk {
                            len: flac.len() as u64,
                            tag: None,
                            quality: WireQuality::Flac,
                        },
                        Some(Box::new(body)),
                    )
                    .await
                    .is_err()
                {
                    return;
                }
                continue;
            }
            _ => None,
        };
        match head {
            Some(head) => {
                if incoming.respond(head, None).await.is_err() {
                    return;
                }
            }
            None => {
                if incoming.respond(Response::NotFound, None).await.is_err() {
                    return;
                }
            }
        }
    }
}

/// `first` bytes, then Pending for ever (a peer that stopped sending without
/// closing): what a stall IS, and what the opener's 1.5 s must survive.
struct StalledBody {
    bytes: Arc<Vec<u8>>,
    cursor: usize,
    first: usize,
}

impl tokio::io::AsyncRead for StalledBody {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if this.cursor >= this.first {
            // Never wakes: a socket that goes quiet without closing.
            return std::task::Poll::Pending;
        }
        let want = (this.first - this.cursor).min(buf.remaining());
        let slice = &this.bytes[this.cursor..this.cursor + want];
        buf.put_slice(slice);
        this.cursor += want;
        std::task::Poll::Ready(Ok(()))
    }
}

/// **A peer that stalls mid-track on `User` hands the rest to the server, from the
/// byte already received**: measured, the takeover lands at exactly one 64 KiB
/// chunk, and the finished file is the file.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stalled_peer_hands_the_rest_of_the_track_to_the_server() {
    let _one = only_one();
    let fixture = Fixture::new("ln11-stall");
    let flac = fixture.root.join("tone.flac");
    tone_flac(&flac, 6.0);
    let flac_bytes = Arc::new(std::fs::read(&flac).expect("the fixture"));
    assert!(
        flac_bytes.len() > 200 * 1024,
        "il faut plus d'un morceau de 64 KiB : {} octets",
        flac_bytes.len()
    );

    let fake = Fake::start();
    let b_api = api_for(&fake, &fixture.root.join("b-api"));
    let b = start_node(&fixture, "b", "Téléphone", &b_api, &[]);
    enrolled(&b).await;
    let peer = StallingPeer::start(
        "ln11-stall-peer",
        "30000004",
        Arc::clone(&flac_bytes),
        96 * 1024,
    );
    b.lan.found(IpAddr::from([127, 0, 0, 1]), peer.port);
    peers_are(&b, 1).await;
    until("le pair qui cale est répliqué", &b.lan, || {
        matches!(
            b.lan.decide("30000004", Quality::Flac, Urgency::Play),
            Source::Peer { .. }
        )
    })
    .await;

    let opens = Opens::default();
    let opener = PeerFirstOpener::new(FileOpener::new(&flac, opens.clone()), b.lan.clone());
    let (engine, mut sink, events) = offline_engine(&fixture.root.join("b-engine"), opener);
    // FLAC on both sides: the only case where a mid-file takeover can splice the
    // SAME bytes (the archive is byte-identical everywhere; an Opus transcode is
    // not, and `take_over` says why in `opener.rs`).
    engine.load("30000004", AudioQuality::Flac, None);
    engine.play();
    first_audible(&engine, &mut sink, Duration::from_secs(5)).expect("aucun son du pair");

    // The body stops at the 64 KiB chunk the peer did not finish, and the server
    // continues from exactly there.
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline && opens.count() == 0 {
        sink.render(&mut vec![0.0f32; 96]);
        std::thread::sleep(Duration::from_millis(1));
    }
    let list = opens.list();
    println!("reprises sur le serveur : {list:?}");
    assert_eq!(
        list,
        vec![Some(64 * 1024)],
        "le serveur doit reprendre au premier morceau non reçu"
    );

    // And the finished spool is the WHOLE file: the splice did not corrupt it.
    let completed = wait_event(
        &mut sink,
        &events,
        "la fin du flux repris",
        Duration::from_secs(15),
        |event| matches!(event, EngineEvent::StreamComplete { .. }),
    );
    let EngineEvent::StreamComplete {
        path,
        bytes,
        quality,
        ..
    } = completed
    else {
        unreachable!()
    };
    assert_eq!(bytes, flac_bytes.len() as u64);
    let spool = std::fs::read(&path).expect("spool");
    assert_eq!(
        spool, *flac_bytes,
        "le fichier assemblé n'est pas l'original"
    );
    assert_eq!(quality, AudioQuality::Flac);
    engine.shutdown();
}

/// **A seek in a peer stream is a byte range from the SAME peer (and its
/// quality), even when a fresh decision would have picked another device**, and
/// `ready()` answers from memory.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_seek_reopens_the_same_peer_at_the_byte_and_ready_costs_nothing() {
    let _one = only_one();
    let fixture = Fixture::new("ln11-seek");
    let flac = fixture.root.join("tone.flac");
    let opus = fixture.root.join("tone128.opus");
    tone_flac(&flac, 6.0);
    ffmpeg(&[
        "-i",
        flac.to_str().unwrap(),
        "-c:a",
        "libopus",
        "-b:a",
        "128k",
        opus.to_str().unwrap(),
    ]);
    let flac_bytes = std::fs::read(&flac).expect("the fixture");
    let opus_bytes = std::fs::read(&opus).expect("the opus fixture");

    let fake = Fake::start();
    let a_api = api_for(&fake, &fixture.root.join("a-api"));
    let b_api = api_for(&fake, &fixture.root.join("b-api"));
    let c_api = api_for(&fake, &fixture.root.join("c-api"));
    // A holds the FLAC, C the Opus 128: the two rungs are the discriminator, and
    // what makes "a fresh decision would have switched" observable.
    let a = start_node(&fixture, "a", "PC", &a_api, &[]);
    download_at(&a.cache, "30000005", nz_model::Quality::Flac, &flac_bytes);
    let c = start_node(&fixture, "c", "Portable", &c_api, &[]);
    download_at(
        &c.cache,
        "30000005",
        nz_model::Quality::Opus128,
        &opus_bytes,
    );
    let b = start_node(&fixture, "b", "Téléphone", &b_api, &[]);
    enrolled(&a).await;
    enrolled(&b).await;
    enrolled(&c).await;
    b.lan.found(IpAddr::from([127, 0, 0, 1]), a.port());
    b.lan.found(IpAddr::from([127, 0, 0, 1]), c.port());
    peers_are(&b, 2).await;
    until("les deux pairs répliqués", &b.lan, || {
        b.lan.snapshot().peers.iter().all(|peer| peer.tracks == 1)
    })
    .await;

    let opens = Opens::default();
    let opener = PeerFirstOpener::new(FileOpener::new(&flac, opens.clone()), b.lan.clone());

    // `ready()` reads the replica: 10 000 calls cost less than ONE loopback round
    // trip would (measured: a QUIC request is 0.3–1 ms, so 10 000 are ≥ 3 s).
    let ready = opener
        .ready("30000005", AudioQuality::Opus320)
        .expect("prêt");
    assert!(ready.ready && ready.ranges.as_deref() == Some("bytes"));
    assert_eq!(
        ready.bytes,
        Some(flac_bytes.len() as u64),
        "la longueur du pair"
    );
    let started = Instant::now();
    for _ in 0..10_000 {
        assert!(
            opener
                .ready("30000005", AudioQuality::Opus320)
                .expect("prêt")
                .ready
        );
    }
    let per_call = started.elapsed();
    println!("10 000 ready() : {per_call:?}");
    assert!(
        per_call < Duration::from_millis(200),
        "ready() doit répondre de la mémoire : {per_call:?}"
    );

    // The first open asks for FLAC, which only A has: the stream comes from A.
    let mut whole = Vec::new();
    opener
        .open("30000005", AudioQuality::Flac, None, None, AudioLane::User)
        .expect("un flux du pair")
        .read_to_end(&mut whole)
        .expect("les octets du pair");
    assert_eq!(whole, flac_bytes, "le pair A devait servir le FLAC");
    assert_eq!(
        b.lan.decide("30000005", Quality::Opus128, Urgency::Play),
        Source::Peer {
            peer: only_peer_with(&b.lan, "Portable"),
            quality: Quality::Opus128,
        },
        "une décision neuve irait chez C"
    );

    // The seek (a byte-range reopen) asks for Opus 128 but must stay on A, at the
    // quality it really served.
    let offset = 10_000u64;
    let mut body = opener
        .open(
            "30000005",
            AudioQuality::Opus128,
            None,
            Some(offset),
            AudioLane::User,
        )
        .expect("le même pair");
    assert_eq!(body.len_hint(), Some(flac_bytes.len() as u64 - offset));
    assert_eq!(
        body.quality(),
        Some(AudioQuality::Flac),
        "la qualité du pair, pas celle demandée"
    );
    let mut second = Vec::new();
    body.read_to_end(&mut second).expect("les octets du pair");
    assert_eq!(
        second,
        flac_bytes[offset as usize..],
        "un déplacement doit rouvrir le MÊME pair, à l'octet"
    );
}

/// A progress sink for a direct call: the worker's own is what a screen sees, and
/// this test has no screen.
struct Silent;

impl Progress for Silent {
    fn total(&mut self, _total: Option<u64>) {}
    fn written(&mut self, _bytes: u64) {}
}

/// **A 50-track playlist one peer has complete comes from that peer**, with its
/// labels and its pinned sleeves — queued as ONE job in the real `nz-offline`
/// worker.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_fifty_track_playlist_comes_from_the_peer_that_has_it_complete() {
    let _one = only_one();
    let fixture = Fixture::new("ln11-playlist");
    let flac = fixture.root.join("small.flac");
    tone_flac(&flac, 0.5);
    let flac_bytes = std::fs::read(&flac).expect("the fixture");
    // The server's own shape: a RELATIVE path (`nz-model`'s `cover`), which the
    // index validates before a peer may ask for it.
    let cover = "/api/cover/900";

    let fake = Fake::start();
    let a_api = api_for(&fake, &fixture.root.join("a-api"));
    let b_api = api_for(&fake, &fixture.root.join("b-api"));
    let a = start_node(&fixture, "a", "PC", &a_api, &[]);
    let tracks: Vec<String> = (0..50)
        .map(|index| format!("4000{:04}", index + 1))
        .collect();
    seed_playlist(&a.cache, &tracks, nz_model::Quality::Flac, &flac_bytes);
    seed_playlist_page(&a.cache, cover, 50);
    a.cache.covers().put(cover, PNG).expect("the peer's sleeve");

    let b = start_node(&fixture, "b", "Téléphone", &b_api, &[]);
    enrolled(&a).await;
    enrolled(&b).await;
    let memory_before = b.lan.memory();
    b.lan.found(IpAddr::from([127, 0, 0, 1]), a.port());
    peers_are(&b, 1).await;
    peer_set_complete(&b.lan, "900", 50).await;
    // The connect burst replicates the sets FIRST, then the inventory (one Bulk
    // window): the size of a set comes from the inventory, so wait for it.
    until("l'inventaire d'A répliqué", &b.lan, || {
        b.lan
            .snapshot()
            .peers
            .first()
            .is_some_and(|peer| peer.tracks == 50)
    })
    .await;
    until("la taille de l'ensemble", &b.lan, || {
        b.lan.sets().iter().any(|row| row.bytes > 0)
    })
    .await;
    let memory_after = b.lan.memory();
    println!(
        "réplique d'un ensemble de 50 titres + son inventaire : {} octets comptés",
        memory_after - memory_before
    );
    assert!(
        memory_after - memory_before <= 50 * (64 + 48) + 2_048,
        "un ensemble de 50 titres coûte {} octets comptés",
        memory_after - memory_before
    );

    let inner = Arc::new(DeniedDownloads {
        calls: Opens::default(),
    });
    let offline = Offline::start(b.cache.clone());
    offline.configure_with(
        Arc::clone(&b_api),
        Arc::new(PeerFirstDownloader::new(inner.clone(), b.lan.clone())),
    );
    let key = PeerSetKey {
        peer: only_peer(&b.lan),
        kind: "playlist".to_owned(),
        ident: "900".to_owned(),
    };
    let created = download_peer_sets(&b.lan, &offline, &[key]).expect("la liste part");
    assert_eq!(created, 1);
    offline_until(&offline, "les 50 titres", || {
        offline.snapshot().progress.done == 50
    })
    .await;
    assert_eq!(inner.calls.count(), 0, "le serveur a servi un titre du lot");

    let owner = b_api.owner().expect("un compte");
    for track in &tracks {
        let local = b
            .cache
            .downloads()
            .local(&owner, &nz_model::TrackId::new(track.clone()))
            .expect("local");
        assert!(local.is_some(), "« {track} » n'est pas téléchargé");
    }
    assert_playlist_labels_and_cover(&b.cache, &tracks, cover);
    offline.shutdown();
}

/// The labels `nz-offline` keeps (its own kv, keyed by track) and the sleeve,
/// PINNED: a full sweep evicts nothing of a download's art.
fn assert_playlist_labels_and_cover(cache: &nz_cache::Cache, tracks: &[String], cover: &str) {
    let labels: HashMap<String, nz_offline::Labels> = cache
        .kv()
        .get(nz_offline::LABELS_KEY)
        .expect("les libellés enregistrés");
    for track in tracks {
        let label = labels
            .get(track)
            .unwrap_or_else(|| panic!("« {track} » sans libellé"));
        assert!(!label.title.is_empty(), "« {track} » sans titre");
        assert_eq!(label.artist, "Artiste", "l'artiste du pair");
    }
    cache.covers().evict_to(0).expect("éviction");
    assert!(
        cache.covers().get(cover).is_some(),
        "la pochette du lot doit être épinglée"
    );
}

/// **The same playlist downloads with the server unreachable**, queued in the
/// real `nz-offline` worker: the track list comes from the peer's `Sets`, and the
/// worker's gate asks the FETCHER whether it has a source (`Downloader::reachable`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_same_playlist_downloads_with_the_server_unreachable() {
    let _one = only_one();
    let fixture = Fixture::new("ln11-playlist-offline");
    let flac = fixture.root.join("small.flac");
    tone_flac(&flac, 0.5);
    let flac_bytes = std::fs::read(&flac).expect("the fixture");
    let cover = "/api/cover/900";

    let fake = Fake::start();
    let a_api = api_for(&fake, &fixture.root.join("a-api"));
    let b_api = api_for(&fake, &fixture.root.join("b-api"));
    let a = start_node(&fixture, "a", "PC", &a_api, &[]);
    let tracks: Vec<String> = (0..50)
        .map(|index| format!("4000{:04}", index + 1))
        .collect();
    seed_playlist(&a.cache, &tracks, nz_model::Quality::Flac, &flac_bytes);
    seed_playlist_page(&a.cache, cover, 50);
    a.cache.covers().put(cover, PNG).expect("the peer's sleeve");
    let b = start_node(&fixture, "b", "Téléphone", &b_api, &[]);
    enrolled(&a).await;
    enrolled(&b).await;
    b.lan.found(IpAddr::from([127, 0, 0, 1]), a.port());
    peers_are(&b, 1).await;
    peer_set_complete(&b.lan, "900", 50).await;
    until("l'inventaire d'A répliqué", &b.lan, || {
        b.lan
            .snapshot()
            .peers
            .first()
            .is_some_and(|peer| peer.tracks == 50)
    })
    .await;

    // The trip: no server. The LAN is the only source, and the worker must run.
    b_api.set_online(false);
    let inner = Arc::new(DeniedDownloads {
        calls: Opens::default(),
    });
    let offline = Offline::start(b.cache.clone());
    offline.configure_with(
        Arc::clone(&b_api),
        Arc::new(PeerFirstDownloader::new(inner.clone(), b.lan.clone())),
    );
    let key = PeerSetKey {
        peer: only_peer(&b.lan),
        kind: "playlist".to_owned(),
        ident: "900".to_owned(),
    };
    assert_eq!(
        download_peer_sets(&b.lan, &offline, &[key]).expect("la liste part du pair"),
        1
    );
    offline_until(&offline, "les 50 titres hors ligne", || {
        offline.snapshot().progress.done == 50
    })
    .await;
    assert_ne!(
        offline.snapshot().state,
        nz_offline::WorkerState::Offline,
        "un pair qui a le lot est une source"
    );
    assert_eq!(inner.calls.count(), 0, "le serveur était injoignable");

    let owner = b_api.owner().expect("un compte");
    for track in &tracks {
        assert!(
            b.cache
                .downloads()
                .local(&owner, &nz_model::TrackId::new(track.clone()))
                .expect("local")
                .is_some(),
            "« {track} » n'est pas téléchargé hors ligne"
        );
    }
    assert_playlist_labels_and_cover(&b.cache, &tracks, cover);
    offline.shutdown();
}

/// **With no server AND no peer, the worker stays offline and asks nobody**: the
/// behaviour every existing server-only download keeps.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_worker_with_no_source_stays_offline() {
    let _one = only_one();
    let fixture = Fixture::new("ln11-no-source");
    let fake = Fake::start();
    let b_api = api_for(&fake, &fixture.root.join("b-api"));
    let b = start_node(&fixture, "b", "Téléphone", &b_api, &[]);
    enrolled(&b).await;
    b_api.set_online(false);

    let inner = Arc::new(DeniedDownloads {
        calls: Opens::default(),
    });
    let offline = Offline::start(b.cache.clone());
    offline.configure_with(
        Arc::clone(&b_api),
        Arc::new(PeerFirstDownloader::new(inner.clone(), b.lan.clone())),
    );
    offline.add_set(
        "track",
        "30000001",
        "Titre",
        vec![nz_offline::Choice::new("30000001", "Titre")],
    );
    offline_until(&offline, "l'état hors ligne", || {
        offline.snapshot().state == nz_offline::WorkerState::Offline
    })
    .await;
    assert_eq!(offline.snapshot().progress.done, 0);
    assert_eq!(inner.calls.count(), 0, "personne ne doit être appelé");
    offline.shutdown();
}

/// **Rule 1: playing a network track while this node downloads from a peer and
/// serves another gives 0 underruns and the same first byte.**
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn playing_while_downloading_and_serving_never_underruns() {
    let _one = only_one();
    let fixture = Fixture::new("ln11-rule-one");
    let flac = fixture.root.join("tone.flac");
    tone_flac(&flac, 5.0);
    let flac_bytes = std::fs::read(&flac).expect("the fixture");
    let small = fixture.root.join("small.flac");
    tone_flac(&small, 0.5);
    let small_bytes = std::fs::read(&small).expect("the small fixture");

    let fake = Fake::start();
    let a_api = api_for(&fake, &fixture.root.join("a-api"));
    let b_api = api_for(&fake, &fixture.root.join("b-api"));
    let c_api = api_for(&fake, &fixture.root.join("c-api"));
    let a = start_node(&fixture, "a", "PC", &a_api, &[("30000006", &flac_bytes)]);
    let b = start_node(&fixture, "b", "Téléphone", &b_api, &[]);
    let c = start_node(&fixture, "c", "Portable", &c_api, &[]);

    // The set B serves while it plays, and the one B downloads from A.
    let for_c: Vec<String> = (0..10)
        .map(|index| format!("5000{:04}", index + 1))
        .collect();
    seed_playlist(&b.cache, &for_c, nz_model::Quality::Flac, &small_bytes);
    let for_b: Vec<String> = (0..10)
        .map(|index| format!("6000{:04}", index + 1))
        .collect();
    seed_playlist(&a.cache, &for_b, nz_model::Quality::Flac, &small_bytes);

    enrolled(&a).await;
    enrolled(&b).await;
    enrolled(&c).await;
    b.lan.found(IpAddr::from([127, 0, 0, 1]), a.port());
    b.lan.found(IpAddr::from([127, 0, 0, 1]), c.port());
    c.lan.found(IpAddr::from([127, 0, 0, 1]), b.port());
    peers_are(&b, 2).await;
    peers_are(&c, 1).await;
    until("le titre d'A répliqué", &b.lan, || {
        matches!(
            b.lan.decide("30000006", Quality::Flac, Urgency::Play),
            Source::Peer { .. }
        )
    })
    .await;
    peer_set_complete(&b.lan, "900", 10).await;
    peer_set_complete(&c.lan, "900", 10).await;

    // The baseline: the tap alone, three times.
    let mut baseline = Vec::new();
    for _ in 0..3 {
        let opens = Opens::default();
        let opener = PeerFirstOpener::new(FileOpener::new(&flac, opens.clone()), b.lan.clone());
        let (engine, mut sink, _events) = offline_engine(&fixture.root.join("b-baseline"), opener);
        engine.load("30000006", AudioQuality::Opus320, None);
        engine.play();
        baseline.push(
            first_audible(&engine, &mut sink, Duration::from_secs(3))
                .expect("le pair n'a pas servi")
                .as_secs_f64()
                * 1_000.0,
        );
        engine.shutdown();
    }
    let baseline_ms = median_ms(baseline.clone());

    // Loaded: B plays from A while B downloads a set from A and serves C.
    let b_offline = Offline::start(b.cache.clone());
    let b_inner = Arc::new(DeniedDownloads {
        calls: Opens::default(),
    });
    b_offline.configure_with(
        Arc::clone(&b_api),
        Arc::new(PeerFirstDownloader::new(b_inner, b.lan.clone())),
    );
    let c_offline = Offline::start(c.cache.clone());
    let c_inner = Arc::new(DeniedDownloads {
        calls: Opens::default(),
    });
    c_offline.configure_with(
        Arc::clone(&c_api),
        Arc::new(PeerFirstDownloader::new(c_inner, c.lan.clone())),
    );

    let opens = Opens::default();
    let opener = PeerFirstOpener::new(FileOpener::new(&flac, opens.clone()), b.lan.clone());
    let (engine, mut sink, events) = offline_engine(&fixture.root.join("b-loaded"), opener);
    engine.load("30000006", AudioQuality::Opus320, None);
    engine.play();

    // The background work starts AFTER the tap, on its own threads: it must yield.
    let downloader = {
        let lan = b.lan.clone();
        let key = PeerSetKey {
            peer: only_peer_with(&b.lan, "PC"),
            kind: "playlist".to_owned(),
            ident: "900".to_owned(),
        };
        let offline = b_offline.clone();
        std::thread::spawn(move || download_peer_sets(&lan, &offline, &[key]))
    };
    let server = {
        let lan = c.lan.clone();
        let key = PeerSetKey {
            peer: only_peer_with(&c.lan, "Téléphone"),
            kind: "playlist".to_owned(),
            ident: "900".to_owned(),
        };
        let offline = c_offline.clone();
        std::thread::spawn(move || download_peer_sets(&lan, &offline, &[key]))
    };

    let loaded = first_audible(&engine, &mut sink, Duration::from_secs(3))
        .expect("le pair n'a pas servi sous charge");
    let _ = downloader.join().expect("le fil de téléchargement");
    let _ = server.join().expect("le fil de service");

    let loaded_ms = loaded.as_secs_f64() * 1_000.0;
    println!("premier échantillon : {baseline_ms:.1} ms à vide, {loaded_ms:.1} ms sous charge");
    assert!(
        loaded_ms <= baseline_ms * 1.10 + 20.0,
        "le titre tapé doit être inchangé à ±10 % : {baseline_ms:.1} ms → {loaded_ms:.1} ms"
    );

    // Play the whole track out and count the underruns: zero, whatever happened
    // on the LAN.
    engine.position();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut out = vec![0.0f32; 96];
    while Instant::now() < deadline {
        sink.render(&mut out);
        if let Ok(EngineEvent::Ended { .. }) = events.try_recv() {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let count = underruns(&events);
    println!("sous-alimentations : {count}");
    assert_eq!(count, 0, "le moteur a manqué d'échantillons sous charge");
    engine.shutdown();
    b_offline.shutdown();
    c_offline.shutdown();
}

/// The device a node knows by the NAME it introduced itself with: the tests name
/// their nodes ("PC", "Téléphone"), and two peers are common in rule 1's test.
fn only_peer_with(lan: &Lan, name: &str) -> [u8; 32] {
    let peer = lan
        .snapshot()
        .peers
        .into_iter()
        .find(|peer| peer.name == name)
        .unwrap_or_else(|| panic!("aucun pair nommé {name}"));
    peer.device_pub
}
