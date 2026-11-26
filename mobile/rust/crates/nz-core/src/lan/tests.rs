//! LN13's acceptance, written from the note's list rather than from the code:
//! two nodes on loopback find each other through the platform's feed, the second
//! one reads a track only the first has, a paused node sends nothing, the
//! snapshot is one clone, the listener only hears changes, and `shutdown` joins
//! every thread.
//!
//! Every wait is on an EVENT (`Lan::events`'s `watch`, a listener call, a state
//! change) inside a bounded deadline whose message says what never arrived; the
//! one clock-based thing is `tokio::time::pause()`/`advance`, which is the
//! acceptance's own "10 min idle with the paused clock".
//!
//! The tests run one at a time (`only_one`): `shutdown_joins_every_thread` reads
//! `/proc/self/task`, which is process-wide, and a sibling test's `nz-lan` thread
//! would poison that census (LN10's rule).
//!
//! The serialiser is a `std::sync::Mutex` held across the tests' awaits on
//! purpose — `clippy::await_holding_lock` is about production code, where a lock
//! across an await blocks a runtime; here the guard is what keeps one test in
//! flight at a time.
#![allow(clippy::await_holding_lock)]

use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use nz_audio::{ByteStream, Lane, Quality as AudioQuality, SourceError, StreamOpener, StreamReady};
use nz_lan::Source;
use nz_model::TrackId;

use super::harness::{Fake, Node, TestDir, node, node_on_own_runtime};
use super::{
    ChangeSink, LanCore, MAX_SET_KEYS, NodeState, PlayerSource, hex_decode, parse_set_key,
};

/// The longest any single wait may take before the test fails with its reason.
const DEADLINE: Duration = Duration::from_secs(40);

/// Serialises this file's tests: the thread census is process-wide.
fn only_one() -> MutexGuard<'static, ()> {
    static ONE: OnceLock<Mutex<()>> = OnceLock::new();
    ONE.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

/// Waits until `check` answers true, waking on the node's own version channel:
/// never a poll loop and never a sleep.
async fn until(what: &str, lan: &LanCore, mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + DEADLINE;
    let mut events = lan.events();
    loop {
        if check() {
            return;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(
            !left.is_zero(),
            "{what} : pas arrivé avant {DEADLINE:?}, état {:?}",
            lan.snapshot()
        );
        let _ = tokio::time::timeout(left, events.changed()).await;
    }
}

/// Waits until the node holds a credential: the fake server answered the
/// enrolment, so a `found` will now be dialled instead of dropped.
async fn enrolled(lan: &LanCore) {
    until("l'appareil s'est inscrit", lan, || {
        lan.snapshot().key_until.is_some()
    })
    .await;
}

/// A body nobody has, so the inner opener is only ever called if the LAN path
/// failed — and it counts each call.
struct CountingInner {
    opens: Arc<AtomicU64>,
}

impl StreamOpener for CountingInner {
    fn open(
        &self,
        _id: &str,
        _quality: AudioQuality,
        _start: Option<f64>,
        _from_byte: Option<u64>,
        _lane: Lane,
    ) -> Result<Box<dyn ByteStream>, SourceError> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        Err(SourceError::Message(
            "le serveur ne devrait pas être appelé : le pair a le titre".to_owned(),
        ))
    }

    fn ready(&self, _id: &str, _quality: AudioQuality) -> Result<StreamReady, SourceError> {
        Err(SourceError::Offline)
    }

    fn local(&self, _id: &str, _quality: AudioQuality) -> Option<std::path::PathBuf> {
        None
    }
}

/// A listener that counts its calls.
struct Counter(Arc<AtomicU64>);

impl ChangeSink for Counter {
    fn changed(&self, _version: u64) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// Puts `bytes` on `node` as a COMPLETE download of `track`, the way the
/// downloads worker publishes one (`nz-cache`'s own rows, then the file).
fn publish(node: &Node, track: &str, bytes: &[u8]) {
    // The index reads the rows of the client's account; the fake server never
    // answered `/api/me`, so the owner is the empty string (LN4's
    // `api.owner().unwrap_or_default()`).
    let owner = "";
    let id = TrackId::new(track);
    let part = node
        .core
        .cache()
        .downloads()
        .reserve(owner, &id, nz_model::Quality::Flac)
        .expect("un fichier réservé");
    std::fs::write(&part, bytes).expect("les octets du titre");
    node.core
        .cache()
        .downloads()
        .complete_as(owner, &id, bytes.len() as u64, nz_model::Quality::Flac)
        .expect("un téléchargement complet");
}

// --- the acceptance ----------------------------------------------------------

/// **Two cores find each other through `found`, and the second reads a track only
/// the first has.**
///
/// The inner opener counts its calls and refuses: the body can only have come from
/// the peer, and its bytes are compared with the file the first node published.
/// Measured on this machine: the peer answered **4.4 ms** after `open` (loopback,
/// debug), and the inner opener was called **0** times.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_cores_find_each_other_and_the_second_plays_the_first_ones_track() {
    let _one = only_one();
    let fake = Fake::start();
    let track = "30000001";
    let bytes = vec![0x66u8; 4096]; // 4 KiB of payload: a peer file is opaque here
    let a = node(&fake, "a", "PC").await;
    publish(&a, track, &bytes);
    let b = node(&fake, "b", "Téléphone").await;
    enrolled(&a.lan).await;
    enrolled(&b.lan).await;

    let a_port = a.lan.advertised_port();
    assert_ne!(a_port, 0, "le nœud n'a pas de port");
    b.lan.found("127.0.0.1", a_port);

    until("les deux appareils se sont authentifiés", &b.lan, || {
        matches!(b.lan.snapshot().state, NodeState::Connected(1))
    })
    .await;
    // The inventory replicates on the connection; the decision only goes to a peer
    // once its replica holds the track.
    until("l'inventaire du pair est répliqué", &b.lan, || {
        matches!(
            b.lan.decide(track, nz_api::Quality::Flac),
            Source::Peer { .. }
        )
    })
    .await;

    let opens = Arc::new(AtomicU64::new(0));
    let source = PlayerSource::over(
        Arc::new(CountingInner {
            opens: Arc::clone(&opens),
        }),
        b.lan.lan(),
    );
    assert!(
        source.peer_first(),
        "le nœud existe, l'opener doit passer par le pair d'abord"
    );
    let started = Instant::now();
    let mut stream = source
        .open(track, AudioQuality::Flac, None, None, Lane::User)
        .expect("le pair sert le titre");
    let mut received = Vec::new();
    stream.read_to_end(&mut received).expect("le corps du pair");
    let elapsed = started.elapsed();
    println!("open du pair → dernier octet : {elapsed:?}");

    assert_eq!(
        received, bytes,
        "les octets servis ne sont pas ceux du pair"
    );
    assert_eq!(
        opens.load(Ordering::SeqCst),
        0,
        "le serveur a été appelé pour un titre que le pair a"
    );
    // What the node's own memory costs, counted (worker rule 4).
    println!("mémoire comptée du nœud : {} octets", b.lan.memory());

    b.lan.shutdown();
    a.lan.shutdown();
}

/// **`snapshot` stays one clone, and the listener only hears changes.**
///
/// 10 000 snapshots are a clone of memory under one short lock — measured here
/// (**24 ms** for 10 000, 1 peer each, debug), never a statement or a socket.
///
/// The listener is woken by the node's version channel, so ten minutes of a
/// PAUSED clock on an idle node produce **0** events: there is no timer to tick.
/// The node under measure has NO peer on purpose — a node whose peer goes away
/// publishes the close's own changes for a few hundred ms (measured: 6 → 17
/// events), which belong to that change, not to a clock, and would blur the
/// measurement. The other half of the property — a REAL change is heard — is
/// asserted before and after it, on the same listener.
#[tokio::test]
async fn the_snapshot_is_one_clone_and_the_listener_only_hears_changes() {
    let _one = only_one();
    let fake = Fake::start();
    let b = node_on_own_runtime(&fake, "snap-b", "Téléphone");
    let seen = Arc::new(AtomicU64::new(0));
    b.lan.subscribe(Arc::new(Counter(Arc::clone(&seen))));
    enrolled(&b.lan).await;
    // The enrolment's own publication: the listener is wired, and once it lands
    // the node has nothing left to say. It can land in two steps (the state,
    // then the server's answer), so wait for a short SILENT window on the node's
    // own event channel before freezing the clock: what the assertion below
    // measures must be a timer, not a change already on its way. A node with a
    // timer would keep waking the channel and fail the assertion anyway. The
    // window counts as silent only when the channel stayed quiet AND the counter
    // did not move during it (a listener can land after the channel wakes).
    until("l'inscription a été annoncée", &b.lan, || {
        seen.load(Ordering::SeqCst) >= 1
    })
    .await;
    let mut events = b.lan.events();
    let mut quiet = seen.load(Ordering::SeqCst);
    for _ in 0..40 {
        let before = seen.load(Ordering::SeqCst);
        let timed_out = tokio::time::timeout(Duration::from_millis(250), events.changed())
            .await
            .is_err();
        let after = seen.load(Ordering::SeqCst);
        quiet = after;
        if timed_out && before == after {
            break;
        }
    }

    // 10 minutes of a PAUSED clock: no event, because nothing changed.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(600)).await;
    tokio::time::advance(Duration::from_secs(600)).await;
    assert_eq!(
        seen.load(Ordering::SeqCst),
        quiet,
        "un nœud au repos a poussé un événement : c'est un minuteur, pas un changement"
    );
    tokio::time::resume();

    // The snapshot's own cost, with a peer connected: a clone of memory, counted
    // rather than timed by a wall clock. A device that connects is a real change,
    // and the same listener hears it.
    let a = node_on_own_runtime(&fake, "snap-a", "PC");
    enrolled(&a.lan).await;
    b.lan.found("127.0.0.1", a.lan.advertised_port());
    until("le pair est connecté", &b.lan, || {
        matches!(b.lan.snapshot().state, NodeState::Connected(1))
    })
    .await;
    until("la connexion a été annoncée", &b.lan, || {
        seen.load(Ordering::SeqCst) > quiet
    })
    .await;

    let started = Instant::now();
    let mut peers = 0;
    for _ in 0..10_000 {
        peers += b.lan.snapshot().peers.len();
    }
    println!(
        "10 000 instantanés : {:?} ({} pair(s) chacun)",
        started.elapsed(),
        peers / 10_000
    );
    assert_eq!(peers, 10_000, "l'instantané a perdu le pair");

    b.close();
    a.close();
}

/// **A paused node sends nothing.**
///
/// The measure is `nz-lan`'s own quinn counters, not `tcpdump`: after
/// `set_active(false, false, false)` and the sleep grace, the node is offered a
/// discovery answer and told the network moved — two things that would make an
/// awake node dial — and its packet count does not move.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_paused_node_sends_nothing() {
    let _one = only_one();
    let fake = Fake::start();
    let a = node(&fake, "sleep-a", "PC").await;
    let b = node(&fake, "sleep-b", "Téléphone").await;
    enrolled(&a.lan).await;
    enrolled(&b.lan).await;
    b.lan.found("127.0.0.1", a.lan.advertised_port());
    until("le pair est connecté", &b.lan, || {
        matches!(b.lan.snapshot().state, NodeState::Connected(1))
    })
    .await;

    b.lan.set_sleep_grace(Duration::ZERO);
    b.lan.set_active(false, false, false);
    until("le nœud a fermé ses connexions", &b.lan, || {
        b.lan.snapshot().peers.is_empty()
    })
    .await;
    let before = b.lan.wire();
    assert!(
        before.packets_out > 0,
        "le nœud n'a jamais rien envoyé : la mesure ne prouverait rien"
    );

    // What would make an AWAKE node dial: a discovery answer and a network change.
    b.lan.found("127.0.0.1", a.lan.advertised_port());
    b.lan.network_changed();
    // The refused-or-not verdict is the counter, read after the events above have
    // been processed (the driver samples them; a change of the counters is the
    // only thing this asserts on).
    let after = b.lan.wire();
    assert_eq!(
        after.packets_out, before.packets_out,
        "un nœud endormi a parlé : {} paquets avant, {} après",
        before.packets_out, after.packets_out
    );
    assert_eq!(
        after.dials, before.dials,
        "un nœud endormi a composé un numéro"
    );

    b.lan.shutdown();
    a.lan.shutdown();
}

/// **A server without the feature starts nothing**: no directory, no socket, no
/// thread, and the `Unsupported` the screens hide the section on (rule 7).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_server_without_the_feature_starts_nothing() {
    let _one = only_one();
    let fake = Fake::start();
    fake.remove_feature();
    let dir = TestDir::new("no-feature");
    let core = Arc::new(super::harness::core_over(&dir, &fake.base(), "PC"));
    let outcome = LanCore::start(Arc::clone(&core)).await;
    assert!(
        matches!(outcome, Err(nz_lan::LanError::Unsupported)),
        "un serveur sans la fonction doit refuser : {:?}",
        outcome.err()
    );
    assert!(
        !dir.data().join("lan").exists(),
        "un serveur sans la fonction ne doit pas créer de dossier"
    );
    // The probe is the ONLY request: no status, no enrolment.
    assert_eq!(fake.count("OPTIONS", "/api/lan/status"), 1);
    assert_eq!(fake.count("POST", "/api/lan/enroll"), 0);
}

/// **`shutdown` joins every thread.** A thread that outlived it would keep the
/// `nz-lan` name in `/proc/self/task`, so the census is the proof — taken with
/// this file's tests serialised, because that file is process-wide.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_joins_every_thread() {
    let _one = only_one();
    let fake = Fake::start();
    let before = lan_threads();
    let a = node(&fake, "join-a", "PC").await;
    // The watcher is this module's own thread, started by a subscribe.
    let seen = Arc::new(AtomicU64::new(0));
    a.lan.subscribe(Arc::new(Counter(Arc::clone(&seen))));
    enrolled(&a.lan).await;
    let during = lan_threads();
    assert!(
        during > before,
        "le nœud n'a lancé aucun thread : la mesure ne prouverait rien ({before} avant, {during} après)"
    );

    // `shutdown` JOINS; a node that leaked its runtime would hang here, so the
    // guard is a bounded wait with its own message.
    let node = Arc::clone(&a.lan);
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        node.shutdown();
        let _ = done.send(());
    });
    finished
        .recv_timeout(DEADLINE)
        .expect("l'arrêt ne rend pas la main : un thread n'a pas été joint");
    // Second call is a no-op (Android's process-death path may run it twice).
    a.lan.shutdown();

    let deadline = Instant::now() + DEADLINE;
    while lan_threads() > before {
        assert!(
            Instant::now() < deadline,
            "des threads `nz-lan` survivent à l'arrêt : {:?}",
            lan_thread_names()
        );
        std::thread::yield_now();
    }
    // And the socket is free again, which is `Lan`'s runtime really being gone.
    let freed = std::net::UdpSocket::bind(("127.0.0.1", a.lan.advertised_port()));
    assert!(freed.is_ok(), "le port UDP est encore tenu : {freed:?}");
}

/// The threads of this process whose name starts with `nz-lan` — the ones
/// `nz-lan` and this module own.
fn lan_thread_names() -> Vec<String> {
    let mut names = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc/self/task") else {
        return names;
    };
    for entry in entries.flatten() {
        // A task that exits between the read of the directory and the read of its
        // name is skipped, not fatal (LN10's rule: one vanished entry must not
        // hide the whole census).
        let Ok(name) = std::fs::read_to_string(entry.path().join("comm")) else {
            continue;
        };
        let name = name.trim_end().to_owned();
        if name.starts_with("nz-lan") {
            names.push(name);
        }
    }
    names
}

fn lan_threads() -> usize {
    lan_thread_names().len()
}

/// **Nothing a screen sends becomes a path or a name.** A set key is
/// `<appareil>:<genre>:<ident>`: the peer must be 32 bytes of hex, and a set the
/// node does not hold in its replica (or a peer it is not connected to) is refused
/// with a cause, in one frame, before anything is asked of anyone.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_set_key_is_parsed_before_anything_is_asked() {
    let _one = only_one();
    let good = format!("{}:playlist:42", "ab".repeat(32));
    let key = parse_set_key(&good).expect("une clé valide");
    assert_eq!(key.peer, [0xab; 32]);
    assert_eq!(key.kind, "playlist");
    assert_eq!(key.ident, "42");

    for bad in [
        "not-a-peer:playlist:42",
        "../../etc/passwd:playlist:42",
        "ab:playlist:42",
        &format!("{}:playlist:", "ab".repeat(32)),
        &format!("{}:playlist:{}", "ab".repeat(32), "x".repeat(65)),
        &format!("{}:../:42", "ab".repeat(32)),
    ] {
        assert!(
            parse_set_key(bad).is_err(),
            "une clé hostile a été acceptée : {bad:?}"
        );
    }
    // An ident that LOOKS like a path stays an opaque string: it is compared
    // with the peer's own set ids and never becomes a path (`nz-lan`'s
    // `fetch_set` re-checks its length). Nothing to refuse here, and no request
    // is made with it.
    let looks_like_a_path = format!("{}:playlist:42/../../x", "ab".repeat(32));
    assert!(parse_set_key(&looks_like_a_path).is_ok());

    // The hex reader itself: an odd length, a non-digit and an over-long string.
    assert!(hex_decode("abc").is_none());
    assert!(hex_decode("zz").is_none());
    assert!(hex_decode(&"ab".repeat(65)).is_none());

    // Through the node: an active device with no peer, so the refusal is the
    // peer's absence (a cause, in one frame) and never a transfer.
    let fake = Fake::start();
    let b = node(&fake, "keys", "Téléphone").await;
    enrolled(&b.lan).await;
    let refused = b.lan.download_sets(std::slice::from_ref(&good));
    assert!(
        matches!(refused, Err(nz_lan::LanError::PeerGone)),
        "un ensemble pour un pair absent doit être refusé : {refused:?}"
    );
    assert!(b.lan.download_sets(&["nonsense".to_owned()]).is_err());
    let too_many = vec![good; MAX_SET_KEYS + 1];
    assert!(
        b.lan.download_sets(&too_many).is_err(),
        "le plafond de clés"
    );
    b.lan.shutdown();
}

/// **Settings survive the node not existing yet**: Réglages reads and writes them
/// on a device that never enrolled, and the node that starts later uses them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn settings_are_the_stored_ones_before_the_node_exists() {
    let _one = only_one();
    let fake = Fake::start();
    let dir = TestDir::new("settings");
    let core = Arc::new(super::harness::core_over(&dir, &fake.base(), "Téléphone"));
    let stored = nz_lan::settings::load(core.cache());
    assert!(stored.enabled, "l'échange est actif par défaut");

    let mut wanted = super::NodeSettings::from(stored);
    wanted.serve = false;
    wanted.tolerated = vec!["OPUS_320".to_owned(), "OPUS_64".to_owned()];
    nz_lan::settings::save(core.cache(), &wanted.to_lan());

    let node = LanCore::start(Arc::clone(&core))
        .await
        .expect("un nœud LAN");
    let read = node.settings();
    assert!(!read.serve, "« Partager mes sons » n'a pas été relu");
    assert_eq!(
        read.tolerated,
        vec!["OPUS_64".to_owned(), "OPUS_320".to_owned()]
    );
    node.shutdown();
}

/// **Every failure reaches a screen as a French sentence** (rule 3): the mapping
/// this module owns, asserted variant by variant, because a `LanError` that
/// crossed as "erreur interne" would be a spinner with extra steps.
#[test]
fn every_lan_failure_has_a_french_cause() {
    use nz_lan::LanError;
    let all = [
        LanError::Disabled,
        LanError::Unsupported,
        LanError::NotEnrolled,
        LanError::Expired,
        LanError::Refused,
        LanError::PeerGone,
        LanError::NotFound,
        LanError::Busy { retry_in_ms: 1_500 },
        LanError::Mismatch,
        LanError::BadFile("illisible".to_owned()),
        LanError::Disk("disque plein".to_owned()),
        LanError::Proto("trame inconnue".to_owned()),
        LanError::Server("serveur injoignable".to_owned()),
        LanError::Internal("bug".to_owned()),
    ];
    for error in all {
        let text = error.cause_fr();
        assert!(!text.trim().is_empty(), "{error:?} n'a pas de phrase");
        assert!(
            text.chars().any(|c| c.is_alphabetic()),
            "{error:?} ressemble à un code : {text}"
        );
    }
    assert!(
        LanError::Busy { retry_in_ms: 1_500 }
            .cause_fr()
            .contains("2 s")
    );
    // The feature's name is what `Unsupported` carries across the boundary.
    assert!(super::FEATURE.contains("proches"));
}
