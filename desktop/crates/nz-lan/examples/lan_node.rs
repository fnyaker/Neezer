//! A headless node, for the two-node harness (docs/agents/lan-nodes.md, LN15).
//!
//! This is NOT the app: it is the piece of the desktop that the harness can drive
//! from a shell — a real `Lan`, the real transport, the real fetch path — so that
//! the journey is measured by two PROCESSES and not by one test talking to itself.
//! `tools/two_nodes.sh` runs two of them inside a user network namespace, and the
//! emulator leg runs one of them on the phone and one on the host.
//!
//! # Why it makes its own trust chain
//!
//! A node only exchanges with a device of the same ACCOUNT, and a credential must
//! be signed by the server's LAN key. The journey's point is a TRIP: the server is
//! unreachable, and everything the two devices need is already on disk. The
//! harness therefore generates one LAN key and one subject when it bootstraps the
//! first node (`--init-trust`), stores them beside that node's data, and every
//! other node of the run reads them (`--trust-dir`). Nothing here is a server:
//! the two trust rows are written straight into `lan.sqlite`, and the credential
//! is signed locally. **This is test material, generated per run and kept under
//! the run's own directory** — a shipped app never has a `--init-trust`.
//!
//! # What it prints
//!
//! Every line starts with `NZLAN ` so a shell harness can read it like an event:
//! `READY port=…` once the node is enrolled and bound, `PEER …` when a device of
//! the account authenticates, `DECIDED …` for the source decision, `FETCHED …`
//! when a peer's bytes are on disk, `MEMORY …` and `WIRE …` counted bytes and
//! datagrams, `DONE` at the end. No key, credential, roster or cookie is ever
//! printed: a public device key is an identifier the peer list shows, and it is
//! the only identity that appears.
//!
//! # Usage
//!
//! ```text
//! lan_node --data-dir DIR [--name PC] [--port 0|N] [--discovery external|mdns]
//!          [--init-trust | --trust-dir DIR]
//!          [--seed-file TONE.flac [--seed-copies 2000]] [--seed-track ID=PATH]…
//!          [--peer IP:PORT]… [--fetch TRACK=OUT] [--deadline-ms 20000]
//!          [--wait-ms N] [--server URL]
//! ```

#![allow(clippy::print_stdout)] // the harness reads stdout; there is no logger here

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use aws_lc_rs::digest;
use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};

use nz_api::{Api, SessionJar};
use nz_lan::downloader::PeerFirstDownloader;
use nz_lan::store::Store;
use nz_lan::types::Urgency;
use nz_lan::{DeviceKey, DiscoveryKind, Lan, LanConfig, LanState};
use nz_offline::{Downloader, Fetched, Outcome, Progress, Stop};

/// The credential's layout, 154 bytes: `docs/agents/lan-nodes.md`, *The credential*.
const CREDENTIAL_DOMAIN: &[u8] = b"neezer-lan/1 credential\x00";
const CREDENTIAL_LEN: usize = 154;
const CREDENTIAL_DAYS: i64 = 30;
/// The file the first node writes and every other node reads: one LAN key seed and
/// one subject, both hex, one per line. Under the run's own directory.
const HARNESS_TRUST: &str = "harness-trust";
/// The owner the seeded downloads belong to (`nz-cache`'s account key).
const OWNER: &str = "1";

fn main() -> std::process::ExitCode {
    let args = match Args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(cause) => {
            eprintln!("lan_node : {cause}");
            eprintln!("{}", Args::USAGE);
            return std::process::ExitCode::from(2);
        }
    };
    match run(args) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(cause) => {
            println!("NZLAN ERROR {cause}");
            std::process::ExitCode::FAILURE
        }
    }
}

// --- the command line --------------------------------------------------------

struct Args {
    data_dir: PathBuf,
    name: String,
    platform: &'static str,
    port: u16,
    discovery: DiscoveryKind,
    init_trust: bool,
    trust_dir: Option<PathBuf>,
    seed_file: Option<PathBuf>,
    seed_copies: usize,
    seed_tracks: Vec<(String, PathBuf)>,
    peers: Vec<SocketAddr>,
    fetch: Option<(String, PathBuf)>,
    deadline: Duration,
    wait: Duration,
    server: String,
}

impl Args {
    const USAGE: &'static str = "usage: lan_node --data-dir DIR [--name NAME] [--port N] \
[--discovery external|mdns] [--init-trust | --trust-dir DIR] [--seed-file FLAC \
[--seed-copies N]] [--seed-track ID=PATH]… [--peer IP:PORT]… [--fetch TRACK=OUT] \
[--deadline-ms N] [--wait-ms N] [--server URL]";

    fn parse(mut argv: impl Iterator<Item = String>) -> Result<Args, String> {
        let mut args = Args {
            data_dir: PathBuf::new(),
            name: "nz-node".to_owned(),
            platform: "desktop",
            port: 0,
            discovery: DiscoveryKind::External,
            init_trust: false,
            trust_dir: None,
            seed_file: None,
            seed_copies: 0,
            seed_tracks: Vec::new(),
            peers: Vec::new(),
            fetch: None,
            deadline: Duration::from_secs(20),
            wait: Duration::ZERO,
            server: "http://127.0.0.1:9".to_owned(),
        };
        while let Some(arg) = argv.next() {
            let mut value = |what: &str| -> Result<String, String> {
                argv.next()
                    .ok_or_else(|| format!("{what} attend une valeur"))
            };
            match arg.as_str() {
                "--data-dir" => args.data_dir = PathBuf::from(value("--data-dir")?),
                "--name" => args.name = value("--name")?,
                "--platform" => {
                    args.platform = match value("--platform")?.as_str() {
                        "desktop" => "desktop",
                        "android" => "android",
                        other => return Err(format!("plateforme inconnue : {other}")),
                    }
                }
                "--port" => {
                    args.port = value("--port")?
                        .parse()
                        .map_err(|error| format!("--port : {error}"))?
                }
                "--discovery" => {
                    args.discovery = match value("--discovery")?.as_str() {
                        "external" => DiscoveryKind::External,
                        "mdns" => DiscoveryKind::Mdns,
                        other => return Err(format!("découverte inconnue : {other}")),
                    }
                }
                "--init-trust" => args.init_trust = true,
                "--trust-dir" => args.trust_dir = Some(PathBuf::from(value("--trust-dir")?)),
                "--seed-file" => args.seed_file = Some(PathBuf::from(value("--seed-file")?)),
                "--seed-copies" => {
                    args.seed_copies = value("--seed-copies")?
                        .parse()
                        .map_err(|error| format!("--seed-copies : {error}"))?
                }
                "--seed-track" => {
                    let pair = value("--seed-track")?;
                    let (track, path) = pair
                        .split_once('=')
                        .ok_or_else(|| "--seed-track attend ID=PATH".to_owned())?;
                    args.seed_tracks
                        .push((track.to_owned(), PathBuf::from(path)));
                }
                "--peer" => {
                    let text = value("--peer")?;
                    let addr: SocketAddr = text
                        .parse()
                        .map_err(|error| format!("--peer {text} : {error}"))?;
                    args.peers.push(addr);
                }
                "--fetch" => {
                    let pair = value("--fetch")?;
                    let (track, path) = pair
                        .split_once('=')
                        .ok_or_else(|| "--fetch attend TITRE=SORTIE".to_owned())?;
                    args.fetch = Some((track.to_owned(), PathBuf::from(path)));
                }
                "--deadline-ms" => {
                    args.deadline = Duration::from_millis(
                        value("--deadline-ms")?
                            .parse()
                            .map_err(|error| format!("--deadline-ms : {error}"))?,
                    )
                }
                "--wait-ms" => {
                    args.wait = Duration::from_millis(
                        value("--wait-ms")?
                            .parse()
                            .map_err(|error| format!("--wait-ms : {error}"))?,
                    )
                }
                "--server" => args.server = value("--server")?,
                "-h" | "--help" => {
                    println!("{}", Args::USAGE);
                    std::process::exit(0);
                }
                other => return Err(format!("argument inconnu : {other}")),
            }
        }
        if args.data_dir.as_os_str().is_empty() {
            return Err("--data-dir est obligatoire".to_owned());
        }
        if args.init_trust && args.trust_dir.is_some() {
            return Err("--init-trust et --trust-dir sont exclusifs".to_owned());
        }
        if args.trust_dir.is_none() && !args.init_trust {
            return Err(
                "un nœud a besoin d'une chaîne de confiance : --init-trust ou --trust-dir"
                    .to_owned(),
            );
        }
        Ok(args)
    }
}

// --- the run -----------------------------------------------------------------

fn run(args: Args) -> Result<(), String> {
    std::fs::create_dir_all(&args.data_dir).map_err(|error| format!("{error}"))?;
    let (seed, subject) = bootstrap_trust(&args)?;

    let key =
        DeviceKey::load_or_create(&args.data_dir).map_err(|error| format!("clé : {error}"))?;
    let store = Store::open(&args.data_dir).map_err(|error| format!("lan.sqlite : {error}"))?;
    let (kid, lan_public) = lan_key(&seed);
    store
        .put_trust(&format!("ca:{}", hex(&kid)), &lan_public)
        .map_err(|error| format!("trust ca : {error}"))?;
    let credential = credential(&seed, &subject, &key);
    store
        .put_trust("credential", &credential)
        .map_err(|error| format!("trust credential : {error}"))?;

    // The library this node shares: real files, complete rows.
    let cache = nz_cache::Cache::open_with_cache_dir(
        args.data_dir.join("cache"),
        args.data_dir.join("cache-tmp"),
    )
    .map_err(|error| format!("cache : {error}"))?;
    seed_library(&cache, &args)?;

    let api = Arc::new(
        Api::builder(args.server.clone())
            .session(Arc::new(SessionJar::at(args.data_dir.join("session.json"))))
            .build()
            .map_err(|error| format!("client : {error}"))?,
    );
    api.set_owner(Some(OWNER.to_owned()));

    let started = Instant::now();
    let lan = Lan::start(
        LanConfig {
            data_dir: args.data_dir.clone(),
            device_name: args.name.clone(),
            platform: args.platform,
            app_version: "ln15".to_owned(),
            discovery: args.discovery,
            port: args.port,
        },
        Arc::clone(&api),
        cache.clone(),
    )
    .map_err(|error| format!("démarrage : {}", error.cause_fr()))?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("runtime : {error}"))?;

    // `READY` is the event the harness waits for: enrolled (a port bound) and
    // announcing. The wait is on the node's own `watch`, never a sleep.
    let deadline = started + args.deadline;
    runtime.block_on(async {
        let mut events = lan.events();
        loop {
            let snapshot = lan.snapshot();
            if snapshot.port != 0 && snapshot.state != LanState::NotEnrolled {
                break;
            }
            if Instant::now() >= deadline {
                panic!("le nœud n'est jamais prêt ({:?})", snapshot.state);
            }
            let _ = tokio::time::timeout(Duration::from_millis(200), events.changed()).await;
        }
    });
    let ready_ms = started.elapsed().as_millis();
    println!("NZLAN READY port={} ms={ready_ms}", lan.snapshot().port);

    for peer in &args.peers {
        lan.found(peer.ip(), peer.port());
    }

    // Wait for a peer of the account, then fetch what was asked.
    let mut fetched_ok = true;
    if let Some((track, out)) = args.fetch.as_ref() {
        let connected = wait_for_peer(&runtime, &lan, args.deadline, started);
        println!("NZLAN PEER ms={}", connected.as_millis());
        // The connect burst replicates the SETS first and the inventory second (one
        // `Bulk` window, LN11): the decision is only `Peer` once the replica is
        // there, and waiting for THAT is the phone asking the LAN whether it has
        // the file — not a sleep long enough for it to arrive.
        //
        // A retry is not a workaround, it is the product's own behaviour: when the
        // two devices dial each other on the same announcement, the duplicate rule
        // keeps one connection and the replica of the other is dropped until it is
        // re-synced (measured: a window of about one `Bulk` quiet period). A queue
        // with no server re-decides per track, so the harness does the same and
        // says how many attempts it took.
        let mut attempt = 0;
        loop {
            attempt += 1;
            let decided = wait_for_source(&runtime, &lan, track, args.deadline);
            println!(
                "NZLAN DECIDED {track} {} attempt={attempt}",
                source_name(&decided)
            );
            match fetch_from_peer(&lan, track, out) {
                Ok(()) => break,
                Err(cause) => {
                    println!("NZLAN RETRY {attempt} {cause}");
                    if attempt >= 3 || Instant::now() >= started + args.deadline {
                        println!("NZLAN FETCH-FAILED {track} {cause}");
                        fetched_ok = false;
                        break;
                    }
                }
            }
        }
    }

    if !args.wait.is_zero() {
        let until = Instant::now() + args.wait;
        let mut events = lan.events();
        let mut last = lan.snapshot().peers.len();
        while Instant::now() < until {
            let _ = runtime.block_on(async {
                tokio::time::timeout(Duration::from_millis(200), events.changed()).await
            });
            let peers = lan.snapshot().peers.len();
            if peers != last {
                println!("NZLAN PEERS {peers}");
                last = peers;
            }
        }
    }

    println!("NZLAN MEMORY {}", lan.memory());
    let wire = lan.wire();
    println!(
        "NZLAN WIRE out={} out_bytes={} in={} in_bytes={} dials={}",
        wire.packets_out, wire.bytes_out, wire.packets_in, wire.bytes_in, wire.dials
    );
    let snapshot = lan.snapshot();
    for peer in &snapshot.peers {
        println!(
            "NZLAN PEERVIEW {} {} {} tracks={} rtt_ms={}",
            hex(&peer.device_pub),
            peer.name,
            match peer.relation {
                nz_lan::Relation::Own => "own",
                nz_lan::Relation::Friend { .. } => "friend",
            },
            peer.tracks,
            peer.rtt_ms
        );
    }
    lan.shutdown();
    println!("NZLAN DONE");
    if fetched_ok {
        Ok(())
    } else {
        Err("le titre n'a pas pu être pris".to_owned())
    }
}

/// Waits until exactly one peer of the account is connected, on the node's events.
fn wait_for_peer(
    runtime: &tokio::runtime::Runtime,
    lan: &Lan,
    deadline: Duration,
    started: Instant,
) -> Duration {
    let until = Instant::now() + deadline;
    runtime.block_on(async {
        let mut events = lan.events();
        loop {
            if !lan.snapshot().peers.is_empty() {
                return started.elapsed();
            }
            if Instant::now() >= until {
                panic!("aucun pair : {:?}", lan.snapshot());
            }
            let _ = tokio::time::timeout(Duration::from_millis(200), events.changed()).await;
        }
    })
}

fn source_name(source: &nz_lan::Source) -> String {
    match source {
        nz_lan::Source::Local(_) => "local".to_owned(),
        nz_lan::Source::Peer { quality, .. } => format!("peer:{quality:?}"),
        nz_lan::Source::Server(quality) => format!("server:{quality:?}"),
    }
}

/// Waits until the LAN can serve `track` (the decision leaves `Server`), on the
/// node's own events: a `Bulk` inventory sync waits its quiet window first.
fn wait_for_source(
    runtime: &tokio::runtime::Runtime,
    lan: &Lan,
    track: &str,
    deadline: Duration,
) -> nz_lan::Source {
    let until = Instant::now() + deadline;
    runtime.block_on(async {
        let mut events = lan.events();
        loop {
            let source = lan.decide(track, nz_api::Quality::Flac, Urgency::Bulk);
            if !matches!(source, nz_lan::Source::Server(_)) {
                return source;
            }
            if Instant::now() >= until {
                return source;
            }
            let _ = tokio::time::timeout(Duration::from_millis(200), events.changed()).await;
        }
    })
}

/// One track through the real peer-first path: the same adapter the desktop's
/// downloads worker holds, with a server that does not answer.
fn fetch_from_peer(lan: &Lan, track: &str, out: &Path) -> Result<(), String> {
    let started = Instant::now();
    let inner = Arc::new(NoServer);
    let downloader = PeerFirstDownloader::new(inner, lan.clone());
    let part = out.with_extension("part");
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let stop = Stop::new();
    let mut progress = Counting::default();
    let fetched: Fetched = downloader
        .fetch(
            &nz_model::TrackId::new(track),
            nz_api::Quality::Flac,
            &part,
            &stop,
            &mut progress,
        )
        .map_err(|outcome| match outcome {
            Outcome::Failed(cause) => cause,
            Outcome::Later(_, cause) => cause,
            Outcome::Offline => "aucune source".to_owned(),
            Outcome::Interrupted => "interrompu".to_owned(),
        })?;
    std::fs::rename(&part, out).map_err(|error| error.to_string())?;
    let bytes = std::fs::read(out).map_err(|error| error.to_string())?;
    let hash = digest::digest(&digest::SHA256, &bytes);
    println!(
        "NZLAN FETCHED {track} bytes={} quality={:?} sha256={} ms={}",
        fetched.bytes,
        fetched.quality,
        hex(hash.as_ref()),
        started.elapsed().as_millis()
    );
    Ok(())
}

/// The inner fetcher of the harness: there is no server on a trip, and a fetch
/// that silently went to one would make the journey prove nothing.
struct NoServer;

impl Downloader for NoServer {
    fn fetch(
        &self,
        _track: &nz_model::TrackId,
        _quality: nz_api::Quality,
        _part: &Path,
        _stop: &Stop,
        _progress: &mut dyn Progress,
    ) -> Result<Fetched, Outcome> {
        Err(Outcome::Offline)
    }

    fn reachable(&self) -> bool {
        false
    }
}

#[derive(Default)]
struct Counting {
    total: Option<u64>,
    last: u64,
}

impl Progress for Counting {
    fn total(&mut self, total: Option<u64>) {
        self.total = total;
    }

    fn written(&mut self, bytes: u64) {
        self.last = bytes;
    }
}

// --- the harness trust chain -------------------------------------------------

/// Reads (or creates) the run's LAN key seed and subject.
fn bootstrap_trust(args: &Args) -> Result<([u8; 32], [u8; 16]), String> {
    let file = if args.init_trust {
        let file = args.data_dir.join(HARNESS_TRUST);
        let random = SystemRandom::new();
        let mut seed = [0u8; 32];
        let mut subject = [0u8; 16];
        random
            .fill(&mut seed)
            .map_err(|_| "le générateur aléatoire a refusé".to_owned())?;
        random
            .fill(&mut subject)
            .map_err(|_| "le générateur aléatoire a refusé".to_owned())?;
        let text = format!("seed {}\nsubject {}\n", hex(&seed), hex(&subject));
        std::fs::write(&file, text).map_err(|error| format!("{file:?} : {error}"))?;
        file
    } else {
        args.trust_dir
            .as_ref()
            .map(|dir| dir.join(HARNESS_TRUST))
            .ok_or_else(|| "pas de chaîne de confiance".to_owned())?
    };
    let text = std::fs::read_to_string(&file).map_err(|error| format!("{file:?} : {error}"))?;
    let mut seed = None;
    let mut subject = None;
    for line in text.lines() {
        let Some((name, value)) = line.split_once(' ') else {
            continue;
        };
        match name {
            "seed" => seed = from_hex::<32>(value),
            "subject" => subject = from_hex::<16>(value),
            _ => {}
        }
    }
    match (seed, subject) {
        (Some(seed), Some(subject)) => Ok((seed, subject)),
        _ => Err(format!("{file:?} est illisible")),
    }
}

/// `kid = SHA-256(public key)[0..8]`, `docs/agents/lan-nodes.md` (*Trust chain*).
fn lan_key(seed: &[u8; 32]) -> ([u8; 8], [u8; 32]) {
    let pair = Ed25519KeyPair::from_seed_unchecked(seed).expect("une graine Ed25519 valide");
    let public: [u8; 32] = pair.public_key().as_ref().try_into().expect("32 octets");
    let hash = digest::digest(&digest::SHA256, &public);
    let mut kid = [0u8; 8];
    kid.copy_from_slice(&hash.as_ref()[..8]);
    (kid, public)
}

/// The 154-byte credential of this device, signed by the run's LAN key.
fn credential(seed: &[u8; 32], subject: &[u8; 16], key: &DeviceKey) -> Vec<u8> {
    let (kid, _) = lan_key(seed);
    let pair = Ed25519KeyPair::from_seed_unchecked(seed).expect("une graine Ed25519 valide");
    let mut device_id = [0u8; 12];
    SystemRandom::new()
        .fill(&mut device_id)
        .expect("le générateur aléatoire");
    let now = unix_now();
    let mut body = Vec::with_capacity(90);
    body.extend_from_slice(b"NZLC");
    body.push(1);
    body.extend_from_slice(&kid);
    body.extend_from_slice(subject);
    body.extend_from_slice(&device_id);
    body.extend_from_slice(&key.public());
    body.extend_from_slice(&(now - 60).to_be_bytes());
    body.extend_from_slice(&(now + CREDENTIAL_DAYS * 86_400).to_be_bytes());
    body.push(0b11);
    let mut signed = CREDENTIAL_DOMAIN.to_vec();
    signed.extend_from_slice(&body);
    let signature = pair.sign(&signed);
    body.extend_from_slice(signature.as_ref());
    assert_eq!(body.len(), CREDENTIAL_LEN);
    body
}

// --- the library -------------------------------------------------------------

/// Puts complete downloads in the cache: what this node's inventory is made of.
fn seed_library(cache: &nz_cache::Cache, args: &Args) -> Result<(), String> {
    for (track, path) in &args.seed_tracks {
        let bytes = std::fs::read(path).map_err(|error| format!("{path:?} : {error}"))?;
        seed_one(cache, track, &bytes)?;
    }
    if args.seed_copies > 0 {
        let path = args
            .seed_file
            .as_ref()
            .ok_or_else(|| "--seed-copies demands --seed-file".to_owned())?;
        let bytes = std::fs::read(path).map_err(|error| format!("{path:?} : {error}"))?;
        for index in 0..args.seed_copies {
            let track = format!("70{:06}", index + 1);
            seed_one(cache, &track, &bytes)?;
        }
    }
    Ok(())
}

fn seed_one(cache: &nz_cache::Cache, track: &str, bytes: &[u8]) -> Result<(), String> {
    let id = nz_model::TrackId::new(track);
    let downloads = cache.downloads();
    downloads
        .add_set(
            OWNER,
            "album",
            "ln15",
            "Le lot du voyage",
            &[nz_cache::DownloadTrack {
                track: id.clone(),
                quality: nz_model::Quality::Flac,
            }],
        )
        .map_err(|error| format!("ensemble : {error}"))?;
    let part = downloads
        .reserve(OWNER, &id, nz_model::Quality::Flac)
        .map_err(|error| format!("réservation : {error}"))?;
    std::fs::write(&part, bytes).map_err(|error| format!("{part:?} : {error}"))?;
    downloads
        .complete(OWNER, &id, bytes.len() as u64)
        .map_err(|error| format!("publication : {error}"))?;
    Ok(())
}

// --- the spellings -----------------------------------------------------------

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn from_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let bytes = text.as_bytes();
    if bytes.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for (index, pair) in bytes.as_chunks::<2>().0.iter().enumerate() {
        let pair = std::str::from_utf8(pair).ok()?;
        out[index] = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(out)
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or(0)
}
