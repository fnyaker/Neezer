//! The two-device harness this lot's tests run on: throwaway directories under
//! the workspace's `target/` (never `/tmp`, which is RAM on this machine) and a
//! fake LAN server that signs LN1's contract the way `neezer/lan.py` does.
//!
//! Nothing here multicasts and nothing touches the person's network: the nodes
//! use `DiscoveryKind::External`, so a test feeds `found(127.0.0.1, port)` and the
//! QUIC connections stay on loopback. The fake server binds an ephemeral port on
//! 127.0.0.1 and answers only what `nz-lan` asks for.
//!
//! Why a fake at all: the mobile Rust job installs no Python (mobile.yaml), so
//! the real server cannot run here. The parts of its contract that matter for
//! this lot are the three blobs — status, credential, revocations — and they are
//! signed with the note's own domains and layout, exactly as `tests/service.rs`
//! (LN10) does for the desktop.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nz_lan::DeviceKey;

use crate::core::{Core, CoreConfig, KEY_ORIGIN};
use crate::lan::LanCore;

/// The note's LAN key id (8 bytes, hex) and the person's opaque subject.
const KID_HEX: &str = "56475aa75463474c";
const SUBJECT_HEX: &str = "00112233445566778899aabbccddeeff";
const CREDENTIAL_DOMAIN: &[u8] = b"neezer-lan/1 credential\0";
const REVOCATIONS_DOMAIN: &[u8] = b"neezer-lan/1 revocations\0";
const ROSTER_DOMAIN: &[u8] = b"neezer-lan/1 roster\0";
const DAY: i64 = 24 * 60 * 60;
const CREDENTIAL_DAYS: i64 = 30;

// --- throwaway directories under `target/` ----------------------------------

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

/// One test's roots, removed on drop. Small by nature (a few kB of SQLite and a
/// handful of fixture bytes), and under the workspace's own `target/`.
pub struct TestDir {
    path: PathBuf,
}

impl TestDir {
    pub fn new(tag: &str) -> TestDir {
        let number = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path = target_dir().join(format!("ln13-{tag}-{}-{number}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("data")).expect("le dossier de données");
        std::fs::create_dir_all(path.join("cache")).expect("le dossier de cache");
        TestDir { path }
    }

    pub fn data(&self) -> PathBuf {
        self.path.join("data")
    }

    pub fn cache(&self) -> PathBuf {
        self.path.join("cache")
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// `mobile/rust/target/`, from this crate's manifest directory: a build's own
/// scratch space, and the one place worker rule 9 allows a test to write.
fn target_dir() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .parent()
        .and_then(Path::parent)
        .expect("crates/nz-core vit sous mobile/rust");
    let dir = root.join("target").join("ln13-tests");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// A core over `dir`, configured for `base` (the fake server's origin) BEFORE it
/// is built: `Core::new` reads the stored origin from `nz-cache`'s `kv` and
/// builds its client from it, which is rule 7's "the server survives a restart".
pub fn core_over(dir: &TestDir, base: &str, device: &str) -> Core {
    {
        // The same file `Core::new` will open, written once so the origin is
        // already there when the core builds its client.
        let cache = nz_cache::Cache::open_with_cache_dir(dir.data(), dir.cache())
            .expect("un cache ouvrable");
        cache
            .kv()
            .set(KEY_ORIGIN, &base.to_owned())
            .expect("l'origine mémorisée");
    }
    Core::new(CoreConfig {
        data_dir: dir.data(),
        cache_dir: dir.cache(),
        app_version: "0.0.0-ln13".to_owned(),
        version_code: 2_000_001,
        sdk_int: 35,
        device_name: device.to_owned(),
    })
    .expect("un noyau sur un dossier jetable")
}

/// One node of the harness, with its directory kept alive for the test's length.
pub struct Node {
    pub lan: Arc<LanCore>,
    pub core: Arc<Core>,
    _dir: TestDir,
    /// The runtime the node's client was built on, when the test needed it NOT to
    /// be the test's own (see [`node_on_own_runtime`]): it must outlive the
    /// client, and it is dropped after the node (field order).
    _runtime: Option<tokio::runtime::Runtime>,
}

impl Node {
    /// Stops the node and releases the runtime some nodes carry, off the async
    /// context: a `Runtime` may not be dropped from inside one (measured: "Cannot
    /// drop a runtime in a context where blocking is not allowed").
    pub fn close(self) {
        let Node {
            lan,
            _dir,
            _runtime,
            ..
        } = self;
        lan.shutdown();
        if let Some(runtime) = _runtime {
            let _ = std::thread::spawn(move || drop(runtime)).join();
        }
        drop(_dir);
    }
}

/// A node over a fresh directory, enrolled against `fake`.
///
/// Async because the start probes the server first (rule 7: an older server gets
/// no directory and no thread), and that probe is a real request.
pub async fn node(fake: &Fake, tag: &str, name: &str) -> Node {
    let dir = TestDir::new(tag);
    let core = Arc::new(core_over(&dir, &fake.base(), name));
    let lan = LanCore::start(Arc::clone(&core))
        .await
        .expect("un nœud LAN");
    Node {
        lan,
        core,
        _dir: dir,
        _runtime: None,
    }
}

/// The same, built on a runtime of its OWN.
///
/// `tokio::time::pause` only exists on a CURRENT-THREAD runtime, and a node
/// started from that runtime's context hands its HTTP client a handle onto it:
/// the paused clock then stops the server's answers too (measured: the enrolment
/// never came back). So a test that pauses the clock builds its nodes here, on a
/// runtime that outlives them, and keeps only the clock it means to control.
pub fn node_on_own_runtime(fake: &Fake, tag: &str, name: &str) -> Node {
    let dir = TestDir::new(tag);
    let base = fake.base();
    let device = name.to_owned();
    let (ready, built) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                let _ = ready.send(Err(format!("un runtime à part : {error}")));
                return;
            }
        };
        let core = Arc::new(core_over(&dir, &base, &device));
        let lan = runtime.block_on(LanCore::start(Arc::clone(&core)));
        let _ = ready.send(match lan {
            // The directory travels with the node, and the runtime outlives the
            // client it was built for.
            Ok(lan) => Ok(Node {
                lan,
                core,
                _dir: dir,
                _runtime: Some(runtime),
            }),
            Err(error) => Err(error.to_string()),
        });
    });
    match built.recv().expect("le nœud s'est construit") {
        Ok(node) => node,
        Err(error) => panic!("un nœud LAN : {error}"),
    }
}

// --- base64 and hex, this harness's own ---------------------------------------

pub fn base64(bytes: &[u8]) -> String {
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

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

// --- the signed blobs --------------------------------------------------------

/// The note's credential layout, 154 bytes, signed by the fake server's LAN key.
fn signed_credential(lan: &DeviceKey, device_pub: &[u8; 32], device_id: [u8; 12]) -> Vec<u8> {
    let issued = now();
    let mut body = Vec::with_capacity(90);
    body.extend_from_slice(b"NZLC");
    body.push(1);
    body.extend_from_slice(&hex_decode_fixed::<8>(KID_HEX));
    body.extend_from_slice(&hex_decode_fixed::<16>(SUBJECT_HEX));
    body.extend_from_slice(&device_id);
    body.extend_from_slice(device_pub);
    body.extend_from_slice(&(issued - 60).to_be_bytes());
    body.extend_from_slice(&(issued + CREDENTIAL_DAYS * DAY).to_be_bytes());
    body.push(0b11);
    assert_eq!(body.len(), 90, "le corps de l'attestation fait 90 octets");
    let mut signed = CREDENTIAL_DOMAIN.to_vec();
    signed.extend_from_slice(&body);
    body.extend_from_slice(&lan.sign(&signed));
    assert_eq!(body.len(), 154, "l'attestation fait 154 octets");
    body
}

/// `NZLR`, version 1, a serial and no revoked key: what a fresh server holds.
fn signed_revocations(lan: &DeviceKey, serial: u64) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(b"NZLR");
    body.push(1);
    body.extend_from_slice(&hex_decode_fixed::<8>(KID_HEX));
    body.extend_from_slice(&serial.to_be_bytes());
    body.extend_from_slice(&now().to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    let mut signed = REVOCATIONS_DOMAIN.to_vec();
    signed.extend_from_slice(&body);
    body.extend_from_slice(&lan.sign(&signed));
    body
}

/// The `{"body","sig"}` roster envelope `cred.rs` verifies over the exact bytes.
fn roster_envelope(lan: &DeviceKey) -> serde_json::Value {
    let issued = now();
    let body = format!(
        r#"{{"v":1,"kid":"{KID_HEX}","subject":"{SUBJECT_HEX}","serial":42,"issued":{issued},"expires":{},"shares_to":[],"trusted_by":[]}}"#,
        issued + CREDENTIAL_DAYS * DAY
    );
    let mut signed = ROSTER_DOMAIN.to_vec();
    signed.extend_from_slice(body.as_bytes());
    serde_json::json!({ "body": base64(body.as_bytes()), "sig": base64(&lan.sign(&signed)) })
}

fn hex_decode_fixed<const N: usize>(text: &str) -> [u8; N] {
    let bytes = crate::lan::hex_decode(text).expect("hex lisible");
    <[u8; N]>::try_from(bytes).expect("la longueur attendue")
}

// --- the fake server ---------------------------------------------------------

pub struct Fake {
    port: u16,
    /// The fake server's LAN key and its hex public form: a `DeviceKey` in a
    /// directory of its own, which is this crate's only Ed25519 signer (no new
    /// dependency).
    state: Arc<ServerState>,
    _dir: TestDir,
    hits: Arc<Mutex<Vec<Hit>>>,
}

/// One request the fake server answered, for a test that counts them.
#[derive(Clone, Debug)]
pub struct Hit {
    pub method: String,
    pub path: String,
}

impl Fake {
    pub fn start() -> Fake {
        let dir = TestDir::new("fake-server");
        let lan = DeviceKey::load_or_create(dir.path()).expect("la clé du faux serveur");
        let listener = TcpListener::bind("127.0.0.1:0").expect("un port de boucle locale");
        let port = listener.local_addr().expect("une adresse").port();
        let hits: Arc<Mutex<Vec<Hit>>> = Arc::new(Mutex::new(Vec::new()));
        let public = hex(&lan.public());
        let state = Arc::new(ServerState {
            lan,
            public_hex: public,
            missing: std::sync::atomic::AtomicBool::new(false),
        });
        let recorded = Arc::clone(&hits);
        let shared = Arc::clone(&state);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let state = Arc::clone(&shared);
                let hits = Arc::clone(&recorded);
                std::thread::spawn(move || serve(stream, &state, &hits));
            }
        });
        Fake {
            port,
            state,
            _dir: dir,
            hits,
        }
    }

    pub fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn hits(&self) -> Vec<Hit> {
        self.hits.lock().expect("les requêtes").clone()
    }

    pub fn count(&self, method: &str, path: &str) -> usize {
        self.hits()
            .iter()
            .filter(|hit| hit.method == method && hit.path == path)
            .count()
    }

    /// The server stops answering every LAN route: an older server, or an admin
    /// who switched the feature off (rule 7).
    pub fn remove_feature(&self) {
        self.state.missing.store(true, Ordering::SeqCst);
    }
}

struct ServerState {
    lan: DeviceKey,
    public_hex: String,
    /// `true` once the server answers `404` to every LAN route.
    missing: std::sync::atomic::AtomicBool,
}

fn serve(mut stream: TcpStream, state: &Arc<ServerState>, hits: &Mutex<Vec<Hit>>) {
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
    let mut length = 0usize;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            length = value.trim().parse().unwrap_or(0);
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
    let body =
        String::from_utf8_lossy(&buffer[header_end..(header_end + length).min(buffer.len())])
            .into_owned();
    let (status, response) = route(&method, &path, &body, state);
    // Recorded BEFORE the answer leaves: a test reads `count()` as soon as the
    // client saw the response, and the client's own thread can run before this
    // one reaches the push. Measured 2026-11-25 (mobile CI run 45511279190,
    // `a_server_without_the_feature_starts_nothing`): the probe was answered 404
    // and counted 0 — a fake that answers before it books the hit makes a count
    // racy by construction.
    hits.lock()
        .expect("les requêtes")
        .push(Hit { method, path });
    let answer = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        if status == 200 { "OK" } else { "No Content" },
        response.len()
    );
    let _ = stream.write_all(answer.as_bytes());
    let _ = stream.write_all(&response);
    let _ = stream.flush();
}

fn route(method: &str, path: &str, body: &str, state: &Arc<ServerState>) -> (u16, Vec<u8>) {
    if state.missing.load(Ordering::SeqCst) {
        return not_found();
    }
    match (method, path) {
        ("OPTIONS", "/api/lan/status") => (200, Vec::new()),
        ("GET", "/api/lan/status") => json(serde_json::json!({
            "enabled": true,
            "subject": SUBJECT_HEX,
            "keys": [{ "kid": KID_HEX, "pub": state.public_hex, "until": null }],
            "credential_days": CREDENTIAL_DAYS,
            "revocation_serial": 7,
        })),
        ("POST", "/api/lan/enroll") => enroll(body, state),
        ("GET", "/api/lan/roster") => json(serde_json::json!({
            "roster": roster_envelope(&state.lan),
            "revocations": base64(&signed_revocations(&state.lan, 7)),
            "etag": "\"42-7\"",
        })),
        ("GET", "/api/lan/devices") => json(serde_json::json!([])),
        ("DELETE", "/api/lan/devices/self") => (204, Vec::new()),
        _ => not_found(),
    }
}

/// `POST /api/lan/enroll`, `neezer/lan.py#enroll`'s contract minus the database:
/// the credential of the key that asked, the roster and the revocations.
///
/// The possession proof is NOT verified here — it is LN1's and LN2's contract and
/// both assert the vector; what this lot needs is a device that can authenticate
/// on the LAN the way the note's trust chain does.
fn enroll(body: &str, state: &Arc<ServerState>) -> (u16, Vec<u8>) {
    let Ok(request) = serde_json::from_str::<serde_json::Value>(body) else {
        return json(serde_json::json!({ "error": "Requête invalide.", "code": "invalid_body" }));
    };
    let pubkey_hex = request["pubkey"].as_str().unwrap_or_default();
    let Some(device_pub) =
        crate::lan::hex_decode(pubkey_hex).and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
    else {
        return json(serde_json::json!({ "error": "Clé invalide.", "code": "bad_pubkey" }));
    };
    // The device row's id, derived from the key so a renewal keeps it.
    let mut device_id = [0u8; 12];
    device_id.copy_from_slice(&device_pub[..12]);
    let credential = signed_credential(&state.lan, &device_pub, device_id);
    json(serde_json::json!({
        "device_id": hex(&device_id),
        "credential": base64(&credential),
        "roster": roster_envelope(&state.lan),
        "revocations": base64(&signed_revocations(&state.lan, 7)),
    }))
}

fn json(value: serde_json::Value) -> (u16, Vec<u8>) {
    (200, serde_json::to_vec(&value).expect("du JSON"))
}

/// The answer an older server gives to a route it never heard of: the status is
/// what `Api::has_route` reads (rule 7), never the body.
fn not_found() -> (u16, Vec<u8>) {
    (
        404,
        serde_json::to_vec(&serde_json::json!({
            "error": "route inconnue",
            "code": "not_found",
        }))
        .expect("du JSON"),
    )
}
