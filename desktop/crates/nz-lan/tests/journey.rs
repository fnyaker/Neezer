//! LN15's journey: the trip, end to end, on two REAL `Lan` nodes, a real
//! `nz-offline` worker, a real `nz-audio` engine and real FLAC/Opus fixtures made by
//! ffmpeg (docs/agents/lan-nodes.md, the LN15 lot's *Journey*).
//!
//! **The scenario.** The PC has 2 000 downloaded tracks; the phone is empty; both are
//! on the same LAN and the SERVER IS UNREACHABLE (the trip). The phone takes three
//! playlists from the PC — tracks, titles, artists and sleeves — then plays them with
//! the PC gone, from its own disk. Back online, a set the PC only holds at Opus 64 is
//! decided PER TRACK: the server while it answers and the quality is not tolerated,
//! the peer once it is (or once the server is gone).
//!
//! | Property | Measured by |
//! |---|---|
//! | three playlists, 24 tracks, taken from the PC with the server unreachable | `the_trip_takes_three_playlists_from_the_pc_and_plays_them_with_the_pc_gone` |
//! | titles, artists and PINNED sleeves, so the lists look the same offline | same |
//! | played with the PC gone: from the phone's own files, 0 server opens, tap → sound | same |
//! | an Opus 64-only copy is skipped online and taken offline (and when tolerated) | `an_opus_64_only_copy_is_decided_per_track` |
//!
//! Every wait is on an event — the node's `watch`, the offline worker's `watch`, the
//! engine's channel, a set's completeness — never a sleep. Nothing touches the
//! person's display, sound or network: loopback only, every directory under the
//! crate's `target/`.
//!
//! The fake server, its keys and the node fixture are this file's own copy (an
//! integration binary links nothing from another one), taken from `tests/service.rs`
//! and `tests/integration.rs`.

#![allow(dead_code)]
#![allow(clippy::await_holding_lock)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};

use aws_lc_rs::signature::{ED25519, Ed25519KeyPair, UnparsedPublicKey};
use serde_json::{Value, json};

use nz_api::{Api, SessionJar};
use nz_audio::{
    ByteStream, Engine, EngineConfig, EngineEvent, Lane as AudioLane, OfflineSink,
    Quality as AudioQuality, SourceError, StreamOpener, StreamReady,
};
use nz_lan::downloader::PeerFirstDownloader;
use nz_lan::opener::PeerFirstOpener;
use nz_lan::service::{Activity, DiscoveryKind, Lan, LanConfig, LanSettings, LanState};
use nz_lan::sets::{PeerSetKey, download_peer_sets};
use nz_lan::types::{Kid, Subject, Tolerance};
use nz_offline::{Downloader, Fetched, Offline, Outcome, Progress, Stop};

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
const CREDENTIAL_DAYS: i64 = 30;
/// The PC's library: what a real one looks like after a few months (the note's
/// journey names 2 000 downloaded tracks).
const PC_TRACKS: usize = 2_000;
/// Three playlists of eight tracks, all taken from the PC's own files.
const PLAYLISTS: [(&str, &str, &str); 3] = [
    ("900", "Route", "/api/cover/900"),
    ("901", "Soirée", "/api/cover/901"),
    ("902", "Calme", "/api/cover/902"),
];
const PLAYLIST_TRACKS: usize = 24;

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

/// Standard base64 with padding — this file's own encoder.
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
    let signature = sign_with(&lan_signer(), CREDENTIAL_DOMAIN, &body);
    body.extend_from_slice(&signature);
    assert_eq!(body.len(), 154);
    body
}

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

fn roster_envelope(serial: u64, issued: i64, days: i64) -> Value {
    let body = format!(
        r#"{{"v":1,"kid":"{KID_HEX}","subject":"{SUBJECT_HEX}","serial":{serial},"issued":{issued},"expires":{},"shares_to":[],"trusted_by":[]}}"#,
        issued + days * DAY
    );
    let signature = sign_with(&lan_signer(), ROSTER_DOMAIN, body.as_bytes());
    json!({ "body": base64(body.as_bytes()), "sig": base64(&signature) })
}

// --- the fake server ---------------------------------------------------------

#[derive(Clone, Debug)]
struct Hit {
    method: String,
    path: String,
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
    issued: i64,
}

#[derive(Debug)]
struct ServerState {
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
    let mut if_none_match = None;
    let mut length = 0usize;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().to_owned();
        match name.to_ascii_lowercase().as_str() {
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
    write_answer(&mut stream, status, &headers, &response);
    hits.lock().expect("hits").push(Hit { method, path });
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

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("ln15-journey-{tag}"));
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

/// One complete download at an exact quality.
fn download_at(
    cache: &nz_cache::Cache,
    set: &str,
    track: &str,
    quality: nz_model::Quality,
    bytes: &[u8],
) {
    let id = nz_model::TrackId::new(track);
    let downloads = cache.downloads();
    downloads
        .add_set(
            "1",
            "album",
            set,
            "Le fond du PC",
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

/// A playlist whose members are ALREADY downloaded (the PC's own files): the set a
/// peer serves whole.
fn add_playlist(cache: &nz_cache::Cache, ident: &str, title: &str, tracks: &[String]) {
    let members: Vec<nz_cache::DownloadTrack> = tracks
        .iter()
        .map(|track| nz_cache::DownloadTrack {
            track: nz_model::TrackId::new(track.clone()),
            quality: nz_model::Quality::Flac,
        })
        .collect();
    cache
        .downloads()
        .add_set("1", "playlist", ident, title, &members)
        .expect("add the playlist");
}

/// The cached set page a node answers `Meta`/`Cover` from: titles, artists, sleeve.
fn seed_playlist_page(
    cache: &nz_cache::Cache,
    ident: &str,
    title: &str,
    cover: &str,
    tracks: &[String],
) {
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

/// A 1×1 PNG: what a cover store sniffs as `png`.
const PNG: &[u8] = &[
    0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H', b'D', b'R',
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0a, b'I', b'D', b'A', b'T', 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', 0xae,
    0x42, 0x60, 0x82,
];

/// One running node.
struct Node {
    lan: Lan,
    cache: nz_cache::Cache,
}

impl Drop for Node {
    fn drop(&mut self) {
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

fn api_for(fake: &Fake, dir: &Path) -> Arc<Api> {
    std::fs::create_dir_all(dir).expect("api dir");
    let api = Api::builder(fake.base())
        .session(Arc::new(SessionJar::at(dir.join("session.json"))))
        .build()
        .expect("a client");
    api.set_owner(Some("1".to_owned()));
    Arc::new(api)
}

fn start_node(fixture: &Fixture, tag: &str, name: &str, api: &Arc<Api>) -> Node {
    let cache = fixture.cache(tag);
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

/// Waits until `check` is true, waking on the node's own `watch` — never on a sleep.
async fn until(what: &str, lan: &Lan, mut check: impl FnMut() -> bool) {
    let mut events = lan.events();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        if check() {
            return;
        }
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(
            !left.is_zero(),
            "{what} : pas arrivé en 120 s (état : {:?})",
            lan.snapshot()
        );
        let _ = tokio::time::timeout(left, events.changed()).await;
    }
}

async fn enrolled(node: &Node) {
    until("l'inscription", &node.lan, || {
        let snapshot = node.lan.snapshot();
        snapshot.key_until.is_some() && snapshot.port != 0
    })
    .await;
}

async fn peers_are(node: &Node, count: usize) {
    until(&format!("{count} pair(s)"), &node.lan, || {
        node.peers() == count
    })
    .await;
}

/// Waits until `lan` holds a peer's set `ident` COMPLETE with `count` tracks.
async fn peer_set_complete(lan: &Lan, ident: &str, count: usize) {
    until("l'ensemble complet répliqué", lan, || {
        lan.sets()
            .iter()
            .any(|row| row.set.ident == ident && row.set.complete && row.set.tracks.len() == count)
    })
    .await;
}

/// The device key of the only peer a node has.
fn only_peer(lan: &Lan) -> [u8; 32] {
    let peers = lan.snapshot().peers;
    assert_eq!(peers.len(), 1, "un seul pair attendu : {peers:?}");
    peers[0].device_pub
}

/// Waits for the offline worker, on ITS own `watch`.
async fn offline_until(offline: &Offline, what: &str, mut check: impl FnMut() -> bool) {
    let mut events = offline.events();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(240);
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

// --- the app's two seams -----------------------------------------------------

/// The opens and locals an inner opener has answered: the LAN's own evidence that
/// the server was (or was not) used.
#[derive(Clone, Default)]
struct Calls(Arc<Mutex<(Vec<Option<u64>>, usize)>>);

impl Calls {
    fn open(&self, from: Option<u64>) {
        self.0.lock().expect("calls").0.push(from);
    }

    fn local(&self) {
        self.0.lock().expect("calls").1 += 1;
    }

    fn opens(&self) -> Vec<Option<u64>> {
        self.0.lock().expect("calls").0.clone()
    }

    fn locals(&self) -> usize {
        self.0.lock().expect("calls").1
    }
}

/// The app's opener: what the SERVER would answer (a finished file, or a byte
/// range) and what the play cache holds. The two are told apart on purpose — the
/// local file is the phone's download, the server file is another one — so "it
/// played from its own disk" is a fact about the bytes, not a counter.
struct AppOpener {
    server_file: PathBuf,
    local_file: PathBuf,
    calls: Calls,
}

impl AppOpener {
    fn len(path: &Path) -> u64 {
        std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
    }
}

impl StreamOpener for AppOpener {
    fn open(
        &self,
        _id: &str,
        _quality: AudioQuality,
        _start: Option<f64>,
        from_byte: Option<u64>,
        _lane: AudioLane,
    ) -> Result<Box<dyn ByteStream>, SourceError> {
        self.calls.open(from_byte);
        let mut file = std::fs::File::open(&self.server_file).map_err(|_| SourceError::Offline)?;
        let from = from_byte.unwrap_or(0);
        std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(from))
            .map_err(|error| SourceError::Message(error.to_string()))?;
        Ok(Box::new(FileBody {
            file,
            remaining: AppOpener::len(&self.server_file).saturating_sub(from),
        }))
    }

    fn ready(&self, _id: &str, _quality: AudioQuality) -> Result<StreamReady, SourceError> {
        Ok(StreamReady {
            ready: true,
            ranges: Some("bytes".to_owned()),
            bytes: Some(AppOpener::len(&self.server_file)),
            cached: false,
            offset: false,
            streaming: false,
        })
    }

    /// The play cache: this is where the app looks before it asks anybody, and it is
    /// what the phone plays once the PC is gone.
    fn local(&self, _id: &str, _quality: AudioQuality) -> Option<PathBuf> {
        self.calls.local();
        Some(self.local_file.clone())
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

/// The inner fetcher the journey must never call: there is no server on a trip, and
/// a download that silently went to one would prove nothing.
struct DeniedDownloads {
    calls: Calls,
}

impl Downloader for DeniedDownloads {
    fn fetch(
        &self,
        _track: &nz_model::TrackId,
        _quality: nz_api::Quality,
        _part: &Path,
        _stop: &Stop,
        _progress: &mut dyn Progress,
    ) -> Result<Fetched, Outcome> {
        self.calls.open(None);
        Err(Outcome::Offline)
    }

    fn reachable(&self) -> bool {
        false
    }
}

/// One engine on the pull-driven sink, and its events.
fn offline_engine(
    dir: &Path,
    opener: PeerFirstOpener<AppOpener>,
) -> (Engine, OfflineSink, std::sync::mpsc::Receiver<EngineEvent>) {
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
fn first_audible(sink: &mut OfflineSink, budget: Duration) -> Option<Duration> {
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

// --- ffmpeg fixtures ---------------------------------------------------------

fn ffmpeg(args: &[&str]) {
    let status = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error"])
        .args(args)
        .status()
        .expect("ffmpeg doit être installé (le lot le mesure, il ne le suppose pas)");
    assert!(status.success(), "ffmpeg a échoué : {args:?}");
}

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

// --- the tests ---------------------------------------------------------------

static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn only_one() -> std::sync::MutexGuard<'static, ()> {
    ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// **The trip**: three playlists taken from a PC holding 2 000 tracks, with the
/// server unreachable, then played with the PC gone — plus the numbers the note's
/// LN15 section carries (the PC's library, the replica's counted bytes, the journey's
/// quinn counters, tap → sound).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_trip_takes_three_playlists_from_the_pc_and_plays_them_with_the_pc_gone() {
    let _one = only_one();
    let fixture = Fixture::new("trip");
    let tone = fixture.root.join("tone.flac");
    tone_flac(&tone, 0.1);
    let flac = std::fs::read(&tone).expect("the fixture");
    println!("le morceau du PC : {} octets", flac.len());

    let fake = Fake::start();
    let pc_api = api_for(&fake, &fixture.root.join("pc-api"));
    let phone_api = api_for(&fake, &fixture.root.join("phone-api"));
    let pc = start_node(&fixture, "pc", "PC", &pc_api);
    let tracks: Vec<String> = (0..PC_TRACKS)
        .map(|index| format!("70{:06}", index + 1))
        .collect();
    // The 2 000 files of the PC's library, then the three playlists, whose members
    // are files it already has.
    for track in &tracks {
        download_at(&pc.cache, "1000", track, nz_model::Quality::Flac, &flac);
    }
    // Each playlist has its OWN eight tracks: a track in three pages would answer
    // one cover (the `Meta` a peer serves is per track), and the journey is about
    // three lists that look like three lists.
    let per_playlist = PLAYLIST_TRACKS / PLAYLISTS.len();
    for (index, (ident, title, cover)) in PLAYLISTS.iter().enumerate() {
        let members: Vec<String> =
            tracks[index * per_playlist..(index + 1) * per_playlist].to_vec();
        add_playlist(&pc.cache, ident, title, &members);
        seed_playlist_page(&pc.cache, ident, title, cover, &members);
        pc.cache.covers().put(cover, PNG).expect("the sleeve");
    }

    let phone = start_node(&fixture, "phone", "Téléphone", &phone_api);
    enrolled(&pc).await;
    enrolled(&phone).await;
    let memory_before = phone.lan.memory();
    let started = Instant::now();
    phone.lan.found(IpAddr::from([127, 0, 0, 1]), pc.port());
    peers_are(&phone, 1).await;
    println!(
        "découverte + poignée de main + preuves : {:?}",
        started.elapsed()
    );
    until("l'inventaire des 2 000 titres du PC", &phone.lan, || {
        phone
            .lan
            .snapshot()
            .peers
            .first()
            .is_some_and(|peer| peer.tracks as usize == PC_TRACKS)
    })
    .await;
    for (ident, _, _) in PLAYLISTS {
        peer_set_complete(&phone.lan, ident, PLAYLIST_TRACKS / PLAYLISTS.len()).await;
    }
    let memory_after = phone.lan.memory();
    println!(
        "le téléphone, un pair de {PC_TRACKS} titres + 3 listes : {} → {} octets comptés ({} B/titre de réplique)",
        memory_before,
        memory_after,
        (memory_after.saturating_sub(memory_before)) / PC_TRACKS
    );

    // The trip: the server is gone. Everything below must work from the PC.
    phone_api.set_online(false);
    assert!(!phone_api.is_online(), "le serveur est injoignable");

    let inner = Arc::new(DeniedDownloads {
        calls: Calls::default(),
    });
    let offline = Offline::start(phone.cache.clone());
    offline.configure_with(
        Arc::clone(&phone_api),
        Arc::new(PeerFirstDownloader::new(inner.clone(), phone.lan.clone())),
    );
    let keys: Vec<PeerSetKey> = PLAYLISTS
        .iter()
        .map(|(ident, _, _)| PeerSetKey {
            peer: only_peer(&phone.lan),
            kind: "playlist".to_owned(),
            ident: (*ident).to_owned(),
        })
        .collect();
    assert_eq!(
        download_peer_sets(&phone.lan, &offline, &keys).expect("les trois listes partent"),
        3,
        "un seul travail pour les trois listes"
    );
    let download_started = Instant::now();
    offline_until(&offline, "les 24 titres du voyage", || {
        offline.snapshot().progress.done == PLAYLIST_TRACKS
    })
    .await;
    println!(
        "24 titres pris du PC en {:?}, état {:?}",
        download_started.elapsed(),
        offline.snapshot().state
    );
    assert_eq!(
        inner.calls.opens(),
        Vec::<Option<u64>>::new(),
        "le serveur n'a servi aucun titre du voyage"
    );

    // The phone holds them, with their labels and their pinned sleeves.
    let owner = phone_api.owner().expect("un compte");
    let labels: HashMap<String, nz_offline::Labels> = phone
        .cache
        .kv()
        .get(nz_offline::LABELS_KEY)
        .expect("les libellés enregistrés");
    for track in tracks.iter().take(PLAYLIST_TRACKS) {
        assert!(
            phone
                .cache
                .downloads()
                .local(&owner, &nz_model::TrackId::new(track.clone()))
                .expect("local")
                .is_some(),
            "« {track} » n'est pas sur le téléphone"
        );
        let label = labels
            .get(track)
            .unwrap_or_else(|| panic!("« {track} » sans libellé"));
        assert_eq!(label.title, format!("Titre {track}"));
        assert_eq!(label.artist, "Artiste");
    }
    phone
        .cache
        .covers()
        .evict_to(0)
        .expect("une éviction qui ne doit rien prendre");
    for (_, _, cover) in PLAYLISTS {
        assert!(
            phone.cache.covers().get(cover).is_some(),
            "la pochette {cover} d'une liste téléchargée doit être épinglée"
        );
    }

    // The LAN index re-reads `nz-cache` on its own timer (`REFRESH_EVERY`, 5 s): the
    // downloads become *local* to the node when its version moves, which is the
    // event waited on here.
    until("les 24 titres indexés par le nœud", &phone.lan, || {
        tracks.iter().take(PLAYLIST_TRACKS).all(|track| {
            matches!(
                phone
                    .lan
                    .decide(track, nz_api::Quality::Flac, nz_lan::types::Urgency::Play),
                nz_lan::Source::Local(_)
            )
        })
    })
    .await;

    // The PC goes away: the phone is alone on the network.
    let wire_before = phone.lan.wire();
    pc.lan.shutdown();
    peers_are(&phone, 0).await;
    let (local_path, local_quality) = phone
        .cache
        .downloads()
        .local(&owner, &nz_model::TrackId::new(tracks[0].clone()))
        .expect("local")
        .expect("le premier titre est sur le téléphone");
    assert_eq!(
        local_quality,
        nz_model::Quality::Flac,
        "le PC a servi du FLAC et le fichier dit ce qu'il est"
    );
    match phone.lan.decide(
        &tracks[0],
        nz_api::Quality::Flac,
        nz_lan::types::Urgency::Play,
    ) {
        nz_lan::Source::Local(path) => assert_eq!(path, local_path),
        other => panic!("le titre doit venir du disque du téléphone, décision {other:?}"),
    }

    // It PLAYS: through the app's opener, whose `local()` answers the phone's own
    // file and whose server path points at a DIFFERENT file (so the bytes below can
    // only be the download).
    let other = fixture.root.join("server.flac");
    tone_flac(&other, 0.3);
    let calls = Calls::default();
    let opener = PeerFirstOpener::new(
        AppOpener {
            server_file: other.clone(),
            local_file: local_path.clone(),
            calls: calls.clone(),
        },
        phone.lan.clone(),
    );
    let (engine, mut sink, _events) = offline_engine(&fixture.root.join("phone-engine"), opener);
    let tap = Instant::now();
    engine.load(&tracks[0], AudioQuality::Opus320, None);
    engine.play();
    let heard = first_audible(&mut sink, Duration::from_secs(3))
        .expect("aucun son : le téléphone n'a pas joué son propre fichier");
    println!("appui → premier échantillon, PC éteint : {heard:?}");
    assert!(
        heard <= Duration::from_millis(300),
        "un fichier local doit sonner tout de suite, mesuré {heard:?}"
    );
    assert_eq!(calls.locals(), 1, "l'app a lu son cache de lecture");
    assert_eq!(
        calls.opens(),
        Vec::<Option<u64>>::new(),
        "le chemin serveur n'a pas été ouvert"
    );
    assert!(
        std::fs::metadata(&local_path)
            .expect("le fichier local")
            .len()
            == flac.len() as u64,
        "les octets joués sont ceux du PC"
    );
    let _ = tap;

    // Nothing left the phone after the PC went: `wire()` is quinn's own counter, and
    // the sample is taken while a peer exists (none does), so this is the last one.
    let wire_after = phone.lan.wire();
    assert!(phone.lan.snapshot().peers.is_empty());
    println!(
        "le voyage, compteurs du téléphone : {} octets sortis, {} entrés, {} dialogues (avant arrêt : {} / {})",
        wire_after.bytes_out,
        wire_after.bytes_in,
        wire_after.dials,
        wire_before.bytes_out,
        wire_before.bytes_in
    );
    println!(
        "le téléphone, PC éteint : {} octets comptés, état {:?}",
        phone.lan.memory(),
        phone.lan.snapshot().state
    );
    offline.shutdown();
}

/// **The tolerance, decided per track**: a copy the PC only has at Opus 64 is
/// skipped while the server answers (the default tolerated set excludes it), taken
/// from the PC once the server is gone, and taken again from the PC when the person
/// adds Opus 64 to the tolerated set.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_opus_64_only_copy_is_decided_per_track() {
    let _one = only_one();
    let fixture = Fixture::new("tolerance");
    let opus = fixture.root.join("tone.opus");
    tone_opus64(&opus, 0.4);
    let bytes = std::fs::read(&opus).expect("the fixture");

    let fake = Fake::start();
    let pc_api = api_for(&fake, &fixture.root.join("pc-api"));
    let phone_api = api_for(&fake, &fixture.root.join("phone-api"));
    let pc = start_node(&fixture, "pc", "PC", &pc_api);
    download_at(
        &pc.cache,
        "950",
        "70009001",
        nz_model::Quality::Opus64,
        &bytes,
    );
    let phone = start_node(&fixture, "phone", "Téléphone", &phone_api);
    enrolled(&pc).await;
    enrolled(&phone).await;
    phone.lan.found(IpAddr::from([127, 0, 0, 1]), pc.port());
    peers_are(&phone, 1).await;
    until("l'inventaire du PC", &phone.lan, || {
        phone
            .lan
            .snapshot()
            .peers
            .first()
            .is_some_and(|peer| peer.tracks == 1)
    })
    .await;

    // The server answers: Opus 64 is NOT in the default tolerated set, so the PC is
    // skipped and the server is asked for the FLAC.
    phone_api.set_online(true);
    assert_eq!(
        phone.lan.decide(
            "70009001",
            nz_api::Quality::Flac,
            nz_lan::types::Urgency::Bulk
        ),
        nz_lan::Source::Server(nz_api::Quality::Flac),
        "Opus 64 n'est pas toléré tant que le serveur répond"
    );

    // The trip: the PC is the only source, so its quality is taken.
    phone_api.set_online(false);
    assert_eq!(
        phone.lan.decide(
            "70009001",
            nz_api::Quality::Flac,
            nz_lan::types::Urgency::Bulk
        ),
        nz_lan::Source::Peer {
            peer: only_peer(&phone.lan),
            quality: nz_api::Quality::Opus64,
        },
        "sans serveur, la qualité du pair est toujours acceptée"
    );

    // Back online, with Opus 64 tolerated: the PC is used again, per track.
    phone_api.set_online(true);
    let mut settings: LanSettings = phone.lan.settings();
    let mut tolerated = Tolerance::default_set();
    tolerated.0.insert(nz_api::Quality::Opus64);
    settings.tolerated = tolerated;
    phone.lan.set_settings(settings);
    until("Opus 64 toléré", &phone.lan, || {
        phone
            .lan
            .settings()
            .tolerated
            .contains(nz_api::Quality::Opus64)
    })
    .await;
    assert_eq!(
        phone.lan.decide(
            "70009001",
            nz_api::Quality::Flac,
            nz_lan::types::Urgency::Bulk
        ),
        nz_lan::Source::Peer {
            peer: only_peer(&phone.lan),
            quality: nz_api::Quality::Opus64,
        },
        "avec Opus 64 toléré, le pair sert le titre même serveur joignable"
    );

    // And a track NOBODY has still goes to the server: the decision is per track.
    assert_eq!(
        phone.lan.decide(
            "70009999",
            nz_api::Quality::Flac,
            nz_lan::types::Urgency::Bulk
        ),
        nz_lan::Source::Server(nz_api::Quality::Flac)
    );
    let _ = Activity::Foreground;
    let _ = LanState::Connected(1);
}
