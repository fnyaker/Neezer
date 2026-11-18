//! LN10's acceptance: the `Lan` service end to end — two (or five) REAL nodes on
//! loopback, `ExternalDiscovery`, against a fake server that signs LN1's contract
//! and against fake peers that speak LN6's transport (docs/agents/lan-nodes.md,
//! the LN10 lot).
//!
//! Every property is a number or an event, never a wait for a state to settle:
//! a test waits on the node's `watch` (every publish and every `touch` wakes it),
//! on the fake server's own request log, or on a channel; the `until` helper's
//! deadline is the net under it, not the assertion. Nothing here touches the
//! person's display, sound or network: one loopback listener and loopback QUIC
//! ports, every data directory under the crate's own `target/` (never `/tmp`:
//! tmpfs is RAM).
//!
//! | Property (LN10's *Acceptance*) | Measured by |
//! |---|---|
//! | two nodes enrol, find each other on `found`, connect once, exchange `Hello`, replicate inventories, snapshot with name/relation/RTT | `two_nodes_enrol_find_each_other_and_replicate_their_inventories` |
//! | a gossiped revocation list makes BOTH peers refuse the revoked device | `a_revoked_device_is_refused_by_both_peers_through_gossip` |
//! | `set_active(all false)` closes everything and **0 packets** leave (counter) | `a_paused_node_closes_everything_and_sends_nothing` |
//! | `enabled = false` stops everything; the last-known address brings it back | `switching_the_feature_off_stops_everything` |
//! | `Feature::Lan` absent → nothing started (`Err(Unsupported)`, no file, no thread) | `a_server_without_the_feature_starts_nothing` |
//! | a server that loses the feature → `LanState::Unsupported`, stopped | `a_server_that_loses_the_feature_stops_the_node` |
//! | a renewal swaps the identity and keeps the live connection | `a_renewal_swaps_the_identity_and_keeps_the_live_connection` |
//! | `shutdown()` joins the runtime, the hasher and the discovery (thread names) | `shutdown_joins_every_thread` |
//! | `memory()` counted, ≤ 64 B/item, 4 peers × 8 000 tracks | `four_peer_inventories_fit_the_memory_cap` |
//! | `memory()` flat (≤ 1 %) over 10 000 inventory deltas | `the_memory_is_flat_over_ten_thousand_inventory_deltas` |
//! | the snapshot is a clone, read without SQLite; a tap costs 0 requests | `the_snapshot_and_a_tap_never_touch_sqlite_or_the_network` |
//! | rule 1: a `Bulk` LAN request yields to the person's request | `a_bulk_lan_request_yields_to_the_persons_request` |

// The nodes of this file share one process (and one `/proc/self/task`): its tests
// take a mutex so a thread census counts only its own runtimes. That guard is held
// across awaits on purpose — it serialises the whole test, not one section of it.
#![allow(clippy::await_holding_lock)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

use aws_lc_rs::signature::{ED25519, Ed25519KeyPair, UnparsedPublicKey};
use serde_json::{Value, json};

use nz_api::Quality;
use nz_api::lanes::Lane;
use nz_api::{Api, SessionJar};
use nz_lan::keys::DeviceKey;
use nz_lan::proto::{self, Control, Request, Response, WireQuality};
use nz_lan::service::{
    Activity, DEFAULT_PORT, DiscoveryKind, Lan, LanConfig, LanSettings, LanState,
};
use nz_lan::transport::{Endpoint, Identity, PeerConn, TrustState};
use nz_lan::types::{Kid, LanError, Relation, Source, Subject, Urgency};

// --- the note's reference material, verbatim --------------------------------

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

// --- the tests ---------------------------------------------------------------

/// The nodes of this file share ONE process (and one `/proc/self/task`), so its
/// tests run one at a time: a thread census that counted another test's runtimes
/// would prove nothing. A panic poisons the lock, not the suite.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn only_node() -> std::sync::MutexGuard<'static, ()> {
    ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// **Both enrol, find each other on `found`, one connection, `Hello`, inventories,
/// snapshot.** Each node has its own cache; only A has tracks, and B's replica of A
/// is what a tap on B reads.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_nodes_enrol_find_each_other_and_replicate_their_inventories() {
    let _one = only_node();
    let fixture = Fixture::new("two-nodes");
    let fake = Fake::start();
    let a_api = api_for(&fake, &fixture.root.join("a-api"));
    let b_api = api_for(&fake, &fixture.root.join("b-api"));
    let a = start_node(
        &fixture,
        "a",
        "PC",
        &a_api,
        &[
            ("30000001", b"premier titre"),
            ("30000002", b"deuxieme titre"),
        ],
    );
    let b = start_node(&fixture, "b", "Téléphone", &b_api, &[]);
    enrolled(&a).await;
    enrolled(&b).await;

    // `Lan::start` runs inside a runtime here, so the feature probe is the DRIVER's
    // first request: 2 status + 2 enroll for two devices, and no roster GET (the
    // enrolment's answer carries it). The probe itself is
    // `a_server_without_the_feature_starts_nothing`'s subject.
    assert_eq!(
        fake.count("GET", "/api/lan/status"),
        2,
        "{:#?}",
        fake.hits()
    );
    assert_eq!(fake.count("POST", "/api/lan/enroll"), 2);
    assert_eq!(fake.count("GET", "/api/lan/roster"), 0);
    assert!(fake.device("PC").is_some());
    assert!(fake.device("Téléphone").is_some());
    assert!(
        fake.hits().iter().all(|hit| hit.status == 200),
        "toutes les requêtes ont été acceptées : {:#?}",
        fake.hits()
    );

    assert_ne!(a.port(), 0);
    assert_ne!(b.port(), 0);
    assert!(
        !a.lan.snapshot().port_fallback,
        "un port éphémère ne bascule pas"
    );

    // Discovery is only a hint: the platform resolved B, and A dials it.
    a.lan.found(IpAddr::from([127, 0, 0, 1]), b.port());
    peers_are(&a, 1).await;
    peers_are(&b, 1).await;

    // ONE connection for the pair: A dialled once, B accepted (nothing told it
    // about A, so it dialled nobody).
    assert_eq!(a.lan.wire().dials, 1, "une seule tentative, un seul lien");
    assert_eq!(b.lan.wire().dials, 0);

    // `Hello`: the name, the platform, and the relation authentication answered.
    let view = a.lan.snapshot();
    assert_eq!(view.state, LanState::Connected(1));
    assert_eq!(view.peers[0].name, "Téléphone");
    assert_eq!(view.peers[0].relation, Relation::Own);
    assert_eq!(view.peers[0].platform, "desktop");
    assert!(
        view.peers[0].rtt_ms < 50,
        "un aller-retour en boucle locale : {} ms",
        view.peers[0].rtt_ms
    );

    // The inventories: B's replica of A holds A's two tracks, and a tap on B
    // chooses A — from memory, with no request.
    until("l'inventaire de A chez B", &b.lan, || {
        b.lan
            .snapshot()
            .peers
            .first()
            .is_some_and(|peer| peer.tracks == 2)
    })
    .await;
    match b.lan.decide("30000001", Quality::Flac, Urgency::Play) {
        Source::Peer { peer, quality } => {
            assert_eq!(quality, Quality::Flac);
            // A's own device key, as the server recorded it: the peer B holds.
            let a_pub = fake.device("PC").expect("la ligne de PC").pubkey;
            assert_eq!(peer, a_pub, "le pair choisi est celui qui a le titre");
        }
        other => panic!("un titre que seul A a doit venir de A, pas {other:?}"),
    }
    assert_eq!(
        a.lan.snapshot().peers[0].tracks,
        0,
        "B n'a rien à répliquer"
    );
    assert_eq!(
        b.lan
            .best_peer_for(&["30000001".to_owned(), "30000002".to_owned()]),
        b.lan.snapshot().peers.first().map(|peer| peer.device_pub),
        "un appareil a les deux titres de l'album"
    );

    // The wire counters are real: datagrams and bytes crossed.
    let wire = a.lan.wire();
    println!(
        "LN10 deux nœuds : {} paquets sortis, {} entrés, {} octets sortis ; rtt {} ms",
        wire.packets_out, wire.packets_in, wire.bytes_out, view.peers[0].rtt_ms
    );
    assert!(wire.packets_out >= 2, "un vrai échange : {wire:?}");
    assert!(wire.bytes_out > 0);

    a.lan.shutdown();
    b.lan.shutdown();
}

/// **Gossip: a revocation list A learnt is handed to B on their next connection,
/// and the revoked third device is refused by BOTH.**
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_revoked_device_is_refused_by_both_peers_through_gossip() {
    let _one = only_node();
    let fixture = Fixture::new("gossip");
    let fake = Fake::start();
    // 20 days, so the next contact renews and re-reads the revocation list (a
    // 30-day credential answers from the cache with zero requests).
    fake.set_credential_days(20);
    let a_api = api_for(&fake, &fixture.root.join("a-api"));
    let b_api = api_for(&fake, &fixture.root.join("b-api"));
    let c_api = api_for(&fake, &fixture.root.join("c-api"));
    let a = start_node(&fixture, "a", "PC", &a_api, &[]);
    let b = start_node(&fixture, "b", "Téléphone", &b_api, &[]);
    let c = start_node(&fixture, "c", "Tablette", &c_api, &[]);
    enrolled(&a).await;
    enrolled(&b).await;
    enrolled(&c).await;

    // The tablet is revoked on the server (a lost device).
    let tablet = fake.device("Tablette").expect("la tablette s'est inscrite");
    fake.revoke(&tablet.pubkey);
    assert_eq!(fake.with_state(|state| state.revocation_serial), 8);

    // A contacts the server again: the answer carries the list at serial 8.
    let before = *a.lan.events().borrow();
    a.lan.enroll_now();
    until("le renouvellement de A", &a.lan, || {
        *a.lan.events().borrow() > before
    })
    .await;

    // A and B meet ONLY now: a revocation list is gossiped when a connection is
    // authenticated.
    a.lan.found(IpAddr::from([127, 0, 0, 1]), b.port());
    peers_are(&a, 1).await;
    peers_are(&b, 1).await;

    // The gossip ITSELF, read from B's own store: the list A learnt from the
    // server (serial 8, the tablet's key) is what B ends up holding — the signed
    // bytes `lan.sqlite` keeps for exactly this, and nothing else puts them there.
    // B's control loop reads the frame only once it has replicated A's inventory,
    // so this waits for the fact rather than assuming it.
    let b_store =
        nz_lan::Store::open(&fixture.root.join("b").join("lan")).expect("le magasin de B");
    let key = format!("revocations:{}", to_hex(&reference_kid().0));
    let keys = nz_lan::cred::TrustKeys::from_status(&[(reference_kid(), lan_public())]);
    let list_of = |store: &nz_lan::Store| -> Option<nz_lan::cred::Revocations> {
        let blob = store.trust(&key)?;
        nz_lan::cred::Revocations::decode_verify(&blob, &keys).ok()
    };
    until_within(
        "la liste apprise par bavardage",
        &b.lan,
        Duration::from_secs(15),
        || list_of(&b_store).map(|list| list.serial) == Some(8),
    )
    .await;
    let list = list_of(&b_store).expect("B garde une liste de révocations");
    assert!(
        list.contains(&tablet.pubkey),
        "la tablette révoquée est nommée dans la liste de B"
    );

    // The tablet still believes it is a node. It dials B FIRST — the peer that only
    // knows the list because A gossiped it — and a refusal is the only outcome the
    // gossip can produce.
    let dialled = c.lan.wire().dials;
    c.lan.found(IpAddr::from([127, 0, 0, 1]), b.port());
    until_within("la réponse de B", &c.lan, Duration::from_secs(10), || {
        c.lan.wire().dials > dialled && (c.peers() > 0 || !c.lan.snapshot().failures.is_empty())
    })
    .await;
    assert_eq!(
        c.peers(),
        0,
        "B refuse l'appareil révoqué : la tablette ne s'authentifie pas"
    );
    let failures = c.lan.snapshot().failures;
    assert!(
        failures.iter().any(|(_, why)| why.contains("refusé")),
        "la cause est dite en français : {failures:?}"
    );
    assert_eq!(b.peers(), 1, "B n'a pas accepté un appareil révoqué");

    // And A, which learnt the list from the server, refuses it too.
    assert_eq!(a.peers(), 1, "A n'a pas accepté un appareil révoqué");
    let dialled = c.lan.wire().dials;
    c.lan.found(IpAddr::from([127, 0, 0, 1]), a.port());
    until_within("la réponse de A", &c.lan, Duration::from_secs(10), || {
        c.lan.wire().dials > dialled && !c.lan.snapshot().failures.is_empty()
    })
    .await;
    assert_eq!(c.peers(), 0);

    // Rule 3, the ten-minute silence: the same addresses announced again are not
    // dialled again. The marker is a never-seen address, which IS dialled, and it
    // is queued BEHIND the repeats — so seeing it proves they were handled, and
    // that they dialled nothing.
    for _ in 0..3 {
        c.lan.found(IpAddr::from([127, 0, 0, 1]), a.port());
        c.lan.found(IpAddr::from([127, 0, 0, 1]), b.port());
    }
    c.lan.found(IpAddr::from([127, 0, 0, 1]), 9);
    until_within(
        "le repère de la file",
        &c.lan,
        Duration::from_secs(10),
        || c.lan.wire().dials == 3,
    )
    .await;
    assert_eq!(
        c.lan.wire().dials,
        3,
        "un appareil qui a refusé n'est pas redialé pendant dix minutes"
    );

    a.lan.shutdown();
    b.lan.shutdown();
    c.lan.shutdown();
}

/// **`set_active(all false)` closes everything, and 0 packets leave afterwards.**
///
/// The counters are quinn's own (`WireCounts`), and the barrier is a real request
/// (`enroll_now` still talks to the server — an HTTP call, never a LAN packet), so
/// "nothing left afterwards" is PROVEN, not waited for.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_paused_node_closes_everything_and_sends_nothing() {
    let _one = only_node();
    let fixture = Fixture::new("paused");
    let fake = Fake::start();
    let a_api = api_for(&fake, &fixture.root.join("a-api"));
    let b_api = api_for(&fake, &fixture.root.join("b-api"));
    let a = start_node(&fixture, "a", "PC", &a_api, &[]);
    let b = start_node(&fixture, "b", "Téléphone", &b_api, &[]);
    enrolled(&a).await;
    enrolled(&b).await;
    a.lan.found(IpAddr::from([127, 0, 0, 1]), b.port());
    peers_are(&a, 1).await;
    peers_are(&b, 1).await;

    // The app goes away: no screen, nothing playing, nothing downloading. The grace
    // is the note's 60 s, shortened here because a test cannot pause the clock of a
    // runtime it does not own (`the_service_numbers_are_the_notes` asserts 60 s).
    a.lan.set_sleep_grace(Duration::from_millis(0));
    b.lan.set_sleep_grace(Duration::from_millis(0));
    a.lan.set_active(Activity::Foreground, false);
    b.lan.set_active(Activity::Foreground, false);
    // Five seconds, not forty: a connection closed by the sleep is gone at once,
    // and quinn's 30 s idle timeout must not be able to pass for the node's work.
    let at_once = Duration::from_secs(5);
    peers_are_within(&a, 0, at_once).await;
    peers_are_within(&b, 0, at_once).await;

    // From here on, NOTHING may leave either node.
    let before_a = a.lan.wire();
    let before_b = b.lan.wire();
    a.lan.found(IpAddr::from([127, 0, 0, 1]), b.port());
    b.lan.found(IpAddr::from([127, 0, 0, 1]), a.port());
    a.lan.network_changed();
    b.lan.network_changed();
    a.lan.lost(IpAddr::from([127, 0, 0, 1]), b.port());

    // The barrier: `enroll_now` still reaches the server (one conditional request),
    // and the node answers it AFTER the commands above.
    let requests = fake.hits().len();
    let version = *a.lan.events().borrow();
    a.lan.enroll_now();
    until("la barrière de l'inscription", &a.lan, || {
        *a.lan.events().borrow() > version || fake.hits().len() > requests
    })
    .await;

    // 0 packets, 0 bytes, 0 dials: the battery rule, counted.
    assert_eq!(
        a.lan.wire(),
        before_a,
        "A n'a rien envoyé après son sommeil"
    );
    assert_eq!(b.lan.wire(), before_b, "B non plus");
    assert_eq!(a.peers(), 0);
    assert_eq!(b.peers(), 0);
    println!(
        "LN10 au repos : A {} paquets / {} octets, B {} / {} — aucun nouveau",
        before_a.packets_out, before_a.bytes_out, before_b.packets_out, before_b.bytes_out
    );

    a.lan.shutdown();
    b.lan.shutdown();
}

/// **`enabled = false` stops everything, and the last-known address brings it back
/// without any announcement** (the reconnect path when multicast is filtered).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn switching_the_feature_off_stops_everything() {
    let _one = only_node();
    let fixture = Fixture::new("disabled");
    let fake = Fake::start();
    let a_api = api_for(&fake, &fixture.root.join("a-api"));
    let b_api = api_for(&fake, &fixture.root.join("b-api"));
    let a = start_node(&fixture, "a", "PC", &a_api, &[]);
    let b = start_node(&fixture, "b", "Téléphone", &b_api, &[]);
    enrolled(&a).await;
    enrolled(&b).await;
    a.lan.found(IpAddr::from([127, 0, 0, 1]), b.port());
    peers_are(&a, 1).await;
    peers_are(&b, 1).await;

    // The person switches « Échanger avec mes appareils à proximité » off.
    a.lan.set_settings(LanSettings {
        enabled: false,
        ..a.lan.settings()
    });
    until("l'extinction", &a.lan, || {
        a.lan.snapshot().state == LanState::Off
    })
    .await;
    peers_are(&a, 0).await;
    peers_are(&b, 0).await;
    // Persisted for the next start, and it never touches the network again.
    assert!(!nz_lan::settings::load(&a.cache).enabled);
    // The socket is CLOSED, not merely idle: the port can be bound again, so
    // nothing will accept a connection nobody asked for.
    let port = a.port();
    let free = std::net::UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, port));
    assert!(
        free.is_ok(),
        "le port {port} est libéré quand c'est éteint : {:?}",
        free.err()
    );
    let frozen = a.lan.wire();
    a.lan.found(IpAddr::from([127, 0, 0, 1]), b.port());
    a.lan.enroll_now();
    assert_eq!(a.lan.wire(), frozen, "aucun paquet quand c'est éteint");

    // Back on: the cached credential answers without a request, and the stored
    // address reconnects the pair with no `found` at all.
    let dials = a.lan.wire().dials;
    a.lan.set_settings(LanSettings {
        enabled: true,
        ..a.lan.settings()
    });
    peers_are(&a, 1).await;
    peers_are(&b, 1).await;
    assert_eq!(
        a.lan.wire().dials,
        dials + 1,
        "une adresse connue, une tentative"
    );

    a.lan.shutdown();
    b.lan.shutdown();
}

/// **A server without the route starts NOTHING** — no thread, no socket, and the
/// data directory is not even created (rule 7). Run OUTSIDE a runtime, so the
/// feature probe is `Lan::start`'s own first request.
#[test]
fn a_server_without_the_feature_starts_nothing() {
    let _one = only_node();
    let fixture = Fixture::new("absent");
    let fake = Fake::start();
    fake.remove_feature();
    let api = api_for(&fake, &fixture.root.join("api"));
    let dir = fixture.root.join("absent").join("lan");
    let before = threads_named("nz-lan");

    let started = Lan::start(
        LanConfig {
            data_dir: dir.clone(),
            device_name: "PC".to_owned(),
            platform: "desktop",
            app_version: "0.1.0".to_owned(),
            discovery: DiscoveryKind::External,
            port: 0,
        },
        api,
        fixture.cache("absent"),
    );
    match started {
        Err(LanError::Unsupported) => {}
        Err(other) => panic!("un serveur sans la route doit être Unsupported, pas {other:?}"),
        Ok(_) => panic!("un serveur sans la route ne doit rien démarrer"),
    }
    assert_eq!(
        threads_named("nz-lan"),
        before,
        "aucun thread n'a été créé : {:?}",
        thread_names()
    );
    assert!(!dir.exists(), "rien n'a été écrit : {dir:?}");
    assert_eq!(
        fake.hits().len(),
        1,
        "une seule question (le sondage OPTIONS) : {:#?}",
        fake.hits()
    );
    assert!(fake.hits()[0].is("OPTIONS", "/api/lan/status"));
}

/// **« Partager mes sons » off: a peer sees nothing of this device.**
///
/// The answer is the one a node that HAS nothing gives (an empty inventory, its
/// pages), never a refusal that would tell a friend more than an absent file does.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_device_that_shares_nothing_answers_an_empty_inventory() {
    let _one = only_node();
    let fixture = Fixture::new("sharing-off");
    let fake = Fake::start();
    let a_api = api_for(&fake, &fixture.root.join("a-api"));
    let a = start_node(
        &fixture,
        "a",
        "PC",
        &a_api,
        &[
            ("30000001", b"premier titre"),
            ("30000002", b"second titre"),
        ],
    );
    // The node really has them: a local file, not a peer's copy (the inventory is
    // refreshed on the node's own runtime, so this waits for it).
    until("l'inventaire local", &a.lan, || {
        matches!(
            a.lan.decide("30000001", Quality::Flac, Urgency::Play),
            Source::Local(_)
        )
    })
    .await;
    a.lan.set_settings(LanSettings {
        serve: false,
        ..a.lan.settings()
    });
    until("« Partager mes sons » éteint", &a.lan, || {
        !a.lan.settings().serve
    })
    .await;

    // A client of the same subject asks A directly, so the FRAME is the assertion:
    // an empty inventory (what a node that has nothing answers), and a `Fetch` of a
    // track A does have outside the share is the absent-file refusal.
    let client = own_client("sharing-client");
    let conn = client
        .connect(loopback(a.port()))
        .await
        .expect("A accepte un appareil du même sujet");
    let answer = conn
        .request(Request::Inventory {
            since: None,
            page: 0,
        })
        .await
        .expect("une réponse à l'inventaire");
    match answer.head {
        Response::InventoryFull {
            version,
            last,
            items,
            ..
        } => {
            assert!(items.is_empty(), "rien n'est énuméré : {items:?}");
            assert!(last);
            assert_eq!(
                version, 0,
                "un appareil qui ne partage rien est à la version 0"
            );
        }
        other => panic!("un appareil qui ne partage rien énumère zéro titre, pas {other:?}"),
    }
    let answer = conn
        .request(Request::Fetch {
            track: "30000001".to_owned(),
            quality: WireQuality::Flac,
            from: 0,
            expect: None,
            urgency: Urgency::Bulk,
        })
        .await
        .expect("une réponse au téléchargement");
    assert!(
        matches!(answer.head, Response::NotFound),
        "aucun octet ne sort d'un appareil qui ne partage pas : {:?}",
        answer.head
    );

    a.lan.shutdown();
}

/// **A server that loses the feature** (an upgrade, an admin switch) stops the node
/// and says `Unsupported`: the cached credential is no longer renewed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_server_that_loses_the_feature_stops_the_node() {
    let _one = only_node();
    let fixture = Fixture::new("lost-feature");
    let fake = Fake::start();
    fake.set_credential_days(20);
    let api = api_for(&fake, &fixture.root.join("a-api"));
    let a = start_node(&fixture, "a", "PC", &api, &[]);
    enrolled(&a).await;

    fake.remove_feature();
    a.lan.enroll_now();
    until("l'état sans la fonction", &a.lan, || {
        a.lan.snapshot().state == LanState::Unsupported
    })
    .await;
    assert_eq!(a.peers(), 0);
    // It is asleep for good: an announcement is ignored and no packet leaves.
    let frozen = a.lan.wire();
    a.lan.found(IpAddr::from([127, 0, 0, 1]), DEFAULT_PORT);
    a.lan.enroll_now();
    assert_eq!(a.lan.wire(), frozen);

    a.lan.shutdown();
}

/// **A renewal rotates the key and swaps the identity, and the live connection
/// keeps the one it authenticated with** (the transport's `set_identity`).
///
/// A third device is the witness: it authenticates against the NEW key while B's
/// connection — made with the OLD one — is still up and still counted.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_renewal_swaps_the_identity_and_keeps_the_live_connection() {
    let _one = only_node();
    let fixture = Fixture::new("renewal");
    let fake = Fake::start();
    // Under the 23-day threshold: the next contact renews.
    fake.set_credential_days(20);
    let a_api = api_for(&fake, &fixture.root.join("a-api"));
    let b_api = api_for(&fake, &fixture.root.join("b-api"));
    let c_api = api_for(&fake, &fixture.root.join("c-api"));
    let a = start_node(&fixture, "a", "PC", &a_api, &[]);
    let b = start_node(&fixture, "b", "Téléphone", &b_api, &[]);
    enrolled(&a).await;
    enrolled(&b).await;
    a.lan.found(IpAddr::from([127, 0, 0, 1]), b.port());
    peers_are(&a, 1).await;
    peers_are(&b, 1).await;
    let old = fake.device("PC").expect("la ligne de PC");
    let old_until = a.lan.snapshot().key_until.expect("une clé valable");
    let status_before = fake.count("GET", "/api/lan/status");

    // The renewal: status + enrol, a new key on the same device row. The
    // credential's END is the event (the fake server's window starts on a later
    // instant for a renewal, as a real server's `now + days` would one second
    // later): it moves only once this node APPLIED the new credential, which is
    // after its endpoint swapped identity.
    a.lan.enroll_now();
    until("le renouvellement de A", &a.lan, || {
        a.lan.snapshot().key_until != Some(old_until)
    })
    .await;
    let new = fake.device("PC").expect("la ligne de PC");
    assert!(
        fake.count("GET", "/api/lan/status") > status_before,
        "le renouvellement a bien demandé l'état au serveur : {:#?}",
        fake.hits()
    );
    assert_ne!(new.pubkey, old.pubkey, "le renouvellement tourne la clé");
    assert_eq!(fake.devices(), 2, "une rotation garde la ligne d'appareil");

    // The live connection kept the OLD key: B still sees ONE peer, the same one.
    assert_eq!(b.peers(), 1);
    assert_eq!(
        b.lan.snapshot().peers[0].device_pub,
        old.pubkey,
        "la connexion en cours garde l'identité avec laquelle elle s'est authentifiée"
    );

    // The live connection is not merely alive: it still SERVES. B gains a track
    // after the renewal, tells A on that connection, and A's replica grows — a
    // request over the connection A authenticated with the OLD identity.
    download(
        &b.cache,
        "30000077",
        b"un titre arrive apres le renouvellement",
    );
    until("le nouveau titre de B", &a.lan, || {
        a.lan
            .snapshot()
            .peers
            .first()
            .is_some_and(|peer| peer.tracks == 1)
    })
    .await;

    // The witness: a third device authenticates against A's NEW key.
    let c = start_node(&fixture, "c", "Tablette", &c_api, &[]);
    enrolled(&c).await;
    c.lan.found(IpAddr::from([127, 0, 0, 1]), a.port());
    peers_are(&c, 1).await;
    assert_eq!(
        c.lan.snapshot().peers[0].device_pub,
        new.pubkey,
        "un nouvel appareil voit la nouvelle clé"
    );
    // A holds both: B (old identity) and C (new). Nothing in flight was dropped.
    peers_are(&a, 2).await;
    assert_eq!(b.peers(), 1);

    a.lan.shutdown();
    b.lan.shutdown();
    c.lan.shutdown();
}

/// **`shutdown()` joins the `nz-lan` runtime, the hashing thread and the discovery
/// daemon, and a stopped node still answers its screens.**
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_joins_every_thread() {
    let _one = only_node();
    let fixture = Fixture::new("shutdown");
    let fake = Fake::start();
    let a_api = api_for(&fake, &fixture.root.join("a-api"));
    let b_api = api_for(&fake, &fixture.root.join("b-api"));
    let a = start_node(&fixture, "a", "PC", &a_api, &[]);
    let b = start_node(&fixture, "b", "Téléphone", &b_api, &[]);
    enrolled(&a).await;
    enrolled(&b).await;
    a.lan.found(IpAddr::from([127, 0, 0, 1]), b.port());
    peers_are(&a, 1).await;

    // The runtime's threads and LN4's hashing worker are both there.
    assert!(
        threads_named("nz-lan") >= 2,
        "le runtime de chaque nœud : {:?}",
        thread_names()
    );
    assert!(
        threads_named("nz-lan-hash") >= 2,
        "un worker de hachage par nœud : {:?}",
        thread_names()
    );

    // The driver's source stops all three, in the note's order: the endpoint first
    // (which closes the connections and joins quinn's only task), the discovery,
    // then the children. Scanned, like LN6's "no `tokio::spawn`" test.
    let source = include_str!("../src/service.rs");
    let finish = source
        .split("async fn finish(")
        .nth(1)
        .expect("le pilote a une séquence d'arrêt");
    let endpoint_close = finish.find("endpoint.close().await").expect("l'endpoint");
    let discovery_stop = finish.find("as_discovery().stop()").expect("la découverte");
    let children = finish.find("abort_all").expect("les tâches");
    assert!(
        endpoint_close < discovery_stop && discovery_stop < children,
        "l'ordre d'arrêt est endpoint → découverte → tâches"
    );
    let shutdown = source
        .split("fn stop(&self) {")
        .nth(1)
        .expect("le `stop` du service");
    assert!(shutdown.contains("hashing.stop()"), "le hacheur est joint");
    assert!(shutdown.contains("engine.stop()"), "le runtime est joint");

    // A node asked twice shuts down once, and still answers a screen.
    a.lan.shutdown();
    a.lan.shutdown();
    let dead = a.lan.snapshot();
    assert_eq!(dead.state, LanState::Off);
    assert_eq!(a.peers(), 0);
    assert!(a.lan.memory() < (1 << 20), "memory answers after shutdown");
    assert_eq!(
        a.lan.decide("30000001", Quality::Flac, Urgency::Play),
        Source::Server(Quality::Flac)
    );

    b.lan.shutdown();
    // `shutdown` JOINS its own thread and the hashing worker. The runtime's WORKER
    // is the one thread tokio joins from inside the drop that follows the driver,
    // a few microseconds after the join returns; a census is the only way to see
    // it (tokio publishes no event for a worker's exit), so it is bounded here.
    wait_for_threads(|names| !names.iter().any(|name| name.starts_with("nz-lan")));
    assert_eq!(threads_named("nz-lan"), 0, "{:?}", thread_names());
    assert_eq!(threads_named("nz-lan-hash"), 0, "{:?}", thread_names());
}

/// **`memory()` is counted, and four peers with 8 000 tracks each fit the cap.**
///
/// The peers are real LN6 endpoints answering LN4's `Inventory` pages: four devices
/// of one subject that all have the same 8 000 downloads, which is what a household
/// looks like.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn four_peer_inventories_fit_the_memory_cap() {
    let _one = only_node();
    let fixture = Fixture::new("memory");
    let fake = Fake::start();
    let api = api_for(&fake, &fixture.root.join("a-api"));
    let a = start_node(&fixture, "a", "PC", &api, &[]);
    enrolled(&a).await;

    let tracks = Arc::new(eight_thousand_tracks());
    let mut peers = Vec::new();
    for index in 0..4 {
        let peer = FakePeer::start(&format!("peer{index}"), Arc::clone(&tracks), 0);
        a.lan.found(IpAddr::from([127, 0, 0, 1]), peer.port());
        peers.push(peer);
    }
    until("les quatre inventaires", &a.lan, || {
        let snapshot = a.lan.snapshot();
        snapshot.peers.len() == 4 && snapshot.peers.iter().all(|peer| peer.tracks == 8_000)
    })
    .await;

    let memory = a.lan.memory();
    let items = 4 * 8_000;
    // LN9 measured the replica itself at 48 B/item; the service's own counters are
    // a rounding error beside it, which is what these two bounds say.
    let replica = items * 48;
    println!(
        "LN10 mémoire : {memory} octets comptés pour {items} titres répliqués ({:.1} o/titre)",
        memory as f64 / items as f64
    );
    assert!(
        memory >= replica * 9 / 10,
        "la réplique est comptée : {memory} contre ~{replica}"
    );
    assert!(
        memory <= replica * 11 / 10 + 64 * 1024,
        "et rien d'autre : {memory} contre ~{replica}"
    );
    assert!(
        memory <= 2_500_000,
        "4 pairs × 8 000 titres ≤ 2,5 Mo (le plan) : {memory} octets"
    );
    assert!(
        memory / items <= 64,
        "≤ 64 octets par titre répliqué : {} octets",
        memory / items
    );

    a.lan.shutdown();
}

/// **10 000 inventory deltas cost nothing**: the replica is a sorted map, and a
/// delta rewrites what changed instead of keeping a history.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_memory_is_flat_over_ten_thousand_inventory_deltas() {
    let _one = only_node();
    let fixture = Fixture::new("deltas");
    let fake = Fake::start();
    let api = api_for(&fake, &fixture.root.join("a-api"));
    let a = start_node(&fixture, "a", "PC", &api, &[]);
    enrolled(&a).await;

    let tracks = Arc::new(eight_thousand_tracks());
    let deltas = 10_000usize;
    let peer = FakePeer::start("delta-peer", Arc::clone(&tracks), deltas);
    a.lan.found(IpAddr::from([127, 0, 0, 1]), peer.port());
    until("l'inventaire du pair", &a.lan, || {
        a.lan
            .snapshot()
            .peers
            .first()
            .is_some_and(|peer| peer.tracks == 8_000)
    })
    .await;
    let baseline = a.lan.memory();

    // Every delta is one `InventoryChanged` frame the peer pushes and one round
    // trip the node makes. Waiting on the PEER's counter is not enough: QUIC
    // buffers what the peer has already written. The node bumps its version once
    // per replica that caught up, so that counter — an event, not a clock — is
    // what says the ten thousand were APPLIED.
    let version = *a.lan.events().borrow();
    let started = Instant::now();
    until("les dix mille deltas", &a.lan, || {
        *a.lan.events().borrow() >= version + deltas as u64
    })
    .await;
    let after = a.lan.memory();
    let elapsed = started.elapsed();
    let growth = 100.0 * after.abs_diff(baseline) as f64 / baseline as f64;
    println!(
        "LN10 mémoire : {baseline} → {after} octets sur {deltas} deltas ({growth:.2} %, {elapsed:?})"
    );
    assert!(
        after.abs_diff(baseline) * 100 <= baseline,
        "≤ 1 % de croissance sur 10 000 deltas : {baseline} → {after}"
    );
    assert!(
        peer.deltas_sent() >= deltas,
        "le pair a bien poussé dix mille changements : {}",
        peer.deltas_sent()
    );
    assert_eq!(
        a.lan.snapshot().peers.first().map(|peer| peer.tracks),
        Some(8_000),
        "la réplique est intacte"
    );

    a.lan.shutdown();
}

/// **The snapshot is a clone, and a tap costs nothing.** The source scan is the
/// structural half (`snapshot`'s body cannot reach SQLite); the numbers are the
/// measured half.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_snapshot_and_a_tap_never_touch_sqlite_or_the_network() {
    let _one = only_node();
    let fixture = Fixture::new("snapshot");
    let fake = Fake::start();
    let api = api_for(&fake, &fixture.root.join("a-api"));
    let a = start_node(
        &fixture,
        "a",
        "PC",
        &api,
        &[("30000001", b"un titre local")],
    );
    enrolled(&a).await;
    let tracks = Arc::new(eight_thousand_tracks());
    let peer = FakePeer::start("snapshot-peer", Arc::clone(&tracks), 0);
    a.lan.found(IpAddr::from([127, 0, 0, 1]), peer.port());
    until("l'inventaire du pair", &a.lan, || {
        a.lan
            .snapshot()
            .peers
            .first()
            .is_some_and(|peer| peer.tracks == 8_000)
    })
    .await;

    // The structural half: `snapshot` may not name a database, a file or a cache.
    let source = include_str!("../src/service.rs");
    let snapshot = source
        .split("pub fn snapshot(&self) -> LanSnapshot {")
        .nth(1)
        .expect("`Lan::snapshot`");
    let body = snapshot
        .split("/// The version counter")
        .next()
        .unwrap_or("");
    for forbidden in ["Store", "store", "rusqlite", "SQLite", "cache", "tag("] {
        assert!(
            !body.contains(forbidden),
            "`snapshot` ne doit pas toucher `{forbidden}` :\n{body}"
        );
    }

    // The measured half: 1 000 snapshots and 1 000 taps, with 0 requests. The
    // requests are counted on the PEER (what the node asked it), never the wire
    // bytes: a background ACK is not a request, and this is the property that
    // matters (rule 3: a tap costs no round trip).
    let requests = peer.requests();
    let started = Instant::now();
    let mut last = a.lan.snapshot();
    for _ in 0..1_000 {
        last = a.lan.snapshot();
    }
    let snapshot_ns = started.elapsed().as_nanos() / 1_000;
    let started = Instant::now();
    for _ in 0..1_000 {
        let _ = a.lan.decide("30000000", Quality::Flac, Urgency::Play);
    }
    let decide_ns = started.elapsed().as_nanos() / 1_000;
    println!(
        "LN10 instantané : {snapshot_ns} ns par appel, décision {decide_ns} ns ({} pair, {} requêtes)",
        last.peers.len(),
        peer.requests() - requests
    );
    assert_eq!(last.peers.len(), 1);
    assert!(
        snapshot_ns < 5_000_000,
        "un instantané est un clone, pas une requête : {snapshot_ns} ns"
    );
    assert!(
        decide_ns < 1_000_000,
        "une décision est en mémoire : {decide_ns} ns"
    );
    assert_eq!(peer.requests(), requests, "un tap n'envoie AUCUNE requête");

    a.lan.shutdown();
}

/// **Rule 1: the node's background requests yield to what the person just did.**
///
/// A `User`-lane request answers at once; the next `Bulk` one waits the note's one
/// second of quiet (`nz_api::lanes::BULK_QUIET`) — the SAME gate the server path
/// takes, one throttle and not two.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bulk_lan_request_yields_to_the_persons_request() {
    let _one = only_node();
    let fixture = Fixture::new("rule-one");
    let fake = Fake::start();
    let api = api_for(&fake, &fixture.root.join("a-api"));
    let a = start_node(&fixture, "a", "PC", &api, &[]);
    enrolled(&a).await;

    // The person's action: a User-lane call, the shape of a tap's request.
    let started = Instant::now();
    api.get::<Vec<Value>>(Lane::User, "/lan/devices")
        .await
        .expect("la requête de la personne");
    let user = started.elapsed();
    assert!(
        user < Duration::from_millis(100),
        "ce que la personne fait n'attend pas : {user:?}"
    );
    let released = Instant::now();
    let hits_before = fake.hits().len();

    // The node's background work: a renewal, which is `Lane::Bulk`.
    a.lan.enroll_now();
    until("la requête de fond", &a.lan, || {
        fake.hits().len() > hits_before
    })
    .await;
    let background = fake
        .hits()
        .into_iter()
        .nth(hits_before)
        .expect("une requête de fond est arrivée");
    assert!(
        background.background,
        "la requête de fond s'annonce (`X-NS-Background`) : {background:?}"
    );
    let waited = background.answered.duration_since(released);
    assert!(
        waited >= Duration::from_millis(950),
        "le fond attend le calme d'une seconde : {waited:?}"
    );

    // The LAN side is the SAME throttle: a peer's inventory sync is a `Bulk`
    // transfer, and it waits the person's second too (one mechanism, not two).
    let tracks = Arc::new(eight_thousand_tracks());
    let peer = FakePeer::start("rule-one-peer", tracks, 0);
    let person = Instant::now();
    api.get::<Vec<Value>>(Lane::User, "/lan/devices")
        .await
        .expect("la deuxième requête de la personne");
    let released = Instant::now();
    let asked = person.elapsed();
    a.lan.found(IpAddr::from([127, 0, 0, 1]), peer.port());
    until("le premier inventaire du pair", &a.lan, || {
        peer.first_request().is_some()
    })
    .await;
    let lan_waited = peer
        .first_request()
        .expect("une première requête")
        .duration_since(released);
    println!(
        "LN10 règle 1 : la personne en {asked:?} (HTTP) / {user:?}, le serveur après {waited:?}, \
         le pair après {lan_waited:?}"
    );
    assert!(
        lan_waited >= Duration::from_millis(950),
        "l'inventaire d'un pair attend le calme d'une seconde : {lan_waited:?}"
    );

    a.lan.shutdown();
}

// --- fake peers: the real transport, scripted inventories --------------------

/// One device of the same subject, with its own key and credential, answering
/// `Inventory` from a fixed list and (optionally) pushing `InventoryChanged`.
struct FakePeer {
    port: u16,
    deltas: Arc<AtomicUsize>,
    requests: Arc<AtomicUsize>,
    first_request: Arc<Mutex<Option<Instant>>>,
    _serving: tokio::task::JoinHandle<()>,
}

impl FakePeer {
    fn start(tag: &str, tracks: Arc<Vec<String>>, deltas: usize) -> FakePeer {
        let identity = fake_identity(tag);
        let endpoint = Arc::new(
            Endpoint::bind(
                &tokio::runtime::Handle::current(),
                0,
                identity,
                own_trust(),
                proto::Hello {
                    name: tag.to_owned(),
                    platform: "desktop".to_owned(),
                    app_version: "0.1.0".to_owned(),
                    inventory_version: 1,
                    sets_version: 0,
                },
            )
            .expect("un endpoint de pair"),
        );
        let port = endpoint.port();
        let sent = Arc::new(AtomicUsize::new(0));
        let served = Arc::new(AtomicUsize::new(0));
        let first = Arc::new(Mutex::new(None));
        let serving = {
            let endpoint = Arc::clone(&endpoint);
            let tracks = Arc::clone(&tracks);
            let sent = Arc::clone(&sent);
            let served = Arc::clone(&served);
            let first = Arc::clone(&first);
            tokio::spawn(async move {
                loop {
                    match endpoint.accept().await {
                        Some(Ok(conn)) => {
                            let tracks = Arc::clone(&tracks);
                            let sent = Arc::clone(&sent);
                            let served = Arc::clone(&served);
                            let first = Arc::clone(&first);
                            tokio::spawn(async move {
                                fake_serve(conn, tracks, deltas, sent, served, first).await;
                            });
                        }
                        Some(Err(_)) => continue,
                        None => return,
                    }
                }
            })
        };
        FakePeer {
            port,
            deltas: sent,
            requests: served,
            first_request: first,
            _serving: serving,
        }
    }

    fn port(&self) -> u16 {
        self.port
    }

    /// How many `InventoryChanged` frames this peer has pushed.
    fn deltas_sent(&self) -> usize {
        self.deltas.load(Ordering::Relaxed)
    }

    /// How many requests this peer has ANSWERED (the node's round trips).
    fn requests(&self) -> usize {
        self.requests.load(Ordering::Relaxed)
    }

    /// When the node first asked this peer anything (the LAN side of rule 1).
    fn first_request(&self) -> Option<Instant> {
        *self.first_request.lock().expect("first request")
    }
}

async fn fake_serve(
    conn: PeerConn,
    tracks: Arc<Vec<String>>,
    deltas: usize,
    sent: Arc<AtomicUsize>,
    requests: Arc<AtomicUsize>,
    first_request: Arc<Mutex<Option<Instant>>>,
) {
    if deltas > 0 {
        // One task pushes the version notices; this one answers the requests. The
        // QUIC stream's flow control is the backpressure: the peer cannot outrun
        // the node by more than a window.
        let pushing = conn.clone();
        tokio::spawn(async move {
            for index in 0..deltas {
                if pushing
                    .send_control(Control::InventoryChanged {
                        version: index as u64 + 2,
                    })
                    .await
                    .is_err()
                {
                    return;
                }
                sent.store(index + 1, Ordering::Relaxed);
            }
        });
    }
    while let Some(incoming) = conn.next_incoming().await {
        requests.fetch_add(1, Ordering::Relaxed);
        {
            let mut first = first_request.lock().expect("first request");
            if first.is_none() {
                *first = Some(Instant::now());
            }
        }
        let head = match &incoming.request {
            Request::Ping => Response::Pong,
            Request::Inventory { since, page } => match since {
                Some(from) => Response::InventoryDelta {
                    from: *from,
                    to: from + 1,
                    added: Vec::new(),
                    removed: Vec::new(),
                },
                None => {
                    let start = (*page as usize) * 4_000;
                    let end = (start + 4_000).min(tracks.len());
                    let items = if start >= end {
                        Vec::new()
                    } else {
                        tracks[start..end]
                            .iter()
                            .map(|track| proto::Item {
                                track: track.clone(),
                                copies: vec![proto::Copy {
                                    quality: WireQuality::Flac,
                                    len: 16,
                                }],
                            })
                            .collect()
                    };
                    Response::InventoryFull {
                        version: 1,
                        page: *page,
                        last: end >= tracks.len(),
                        items,
                    }
                }
            },
            _ => Response::NotFound,
        };
        if incoming.respond(head, None).await.is_err() {
            return;
        }
    }
}

/// The address a test dials, spelled once.
fn loopback(port: u16) -> std::net::SocketAddr {
    std::net::SocketAddr::new(IpAddr::from([127, 0, 0, 1]), port)
}

/// A real client endpoint of the test server's subject: what another device of the
/// same person is, for a test that wants to speak the protocol itself.
fn own_client(tag: &str) -> Endpoint {
    Endpoint::bind(
        &tokio::runtime::Handle::current(),
        0,
        fake_identity(tag),
        own_trust(),
        proto::Hello {
            name: tag.to_owned(),
            platform: "desktop".to_owned(),
            app_version: "0.1.0".to_owned(),
            inventory_version: 1,
            sets_version: 0,
        },
    )
    .expect("un endpoint client")
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

/// The 8 000 track ids of a library: `3` + 7 digits, the shape the server makes.
fn eight_thousand_tracks() -> Vec<String> {
    (0..8_000).map(|index| format!("3{index:07}")).collect()
}

/// A device key written by the real key path (`0600`, atomic) under `target/`, with
/// the credential the test server signs for it (the same subject: an own device).
fn fake_identity(tag: &str) -> Arc<Identity> {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("ln10-keys-{tag}"));
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

// --- threads -----------------------------------------------------------------

/// Every thread name of this process (`/proc/self/task/<tid>/comm`): how LN4 and
/// LN3 assert "the worker is niced" and "nothing outlived its thread".
fn thread_names() -> Vec<String> {
    let mut names = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc/self/task") else {
        return names;
    };
    for entry in entries.flatten() {
        if let Ok(name) = std::fs::read_to_string(entry.path().join("comm")) {
            names.push(name.trim_end().to_owned());
        }
    }
    names
}

fn threads_named(name: &str) -> usize {
    thread_names().iter().filter(|kept| *kept == name).count()
}

/// Waits (bounded) for the thread census to satisfy `done`. It is the net under a
/// JOIN, never the assertion: the join is what `shutdown` does, and this only
/// absorbs the handful of microseconds before the kernel retires the tid.
fn wait_for_threads(done: impl Fn(&[String]) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let names = thread_names();
        if done(&names) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "les threads ne sont pas joints : {names:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
