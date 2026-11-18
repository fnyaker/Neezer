//! LN3 — discovery, proven where it is actually used: two processes, one user
//! network namespace, multicast on `lo`, no sudo (docs/agents/lan-nodes.md,
//! *Discovery* and *Two nodes on one machine*).
//!
//! Three parts, cheapest first:
//!
//! 1. the platform feed ([`ExternalDiscovery`]) and the random names — pure, they
//!    run on every `cargo test`;
//! 2. the two **roles** [`netns_announcer`] and [`netns_browser`]: this same test
//!    binary, run twice by `tools/netns.sh` inside `unshare -rn`. They print
//!    `NZLAN …` lines and say nothing when they are run outside the harness (a
//!    printed reason, never a silent pass);
//! 3. [`the_netns_harness_finds_the_other_process`], which runs the harness and
//!    asserts the note's acceptance numbers on what the two processes measured.
//!
//! # Why the roles are tests and not an example
//!
//! `examples/lan_node.rs` is LN15's, and LN3 needs two processes NOW: a `#[test]`
//! is a process that `cargo test` builds and that `--exact <name>` runs alone. The
//! harness passes a role in `NZ_LAN_NETNS_ROLE`, so `cargo test --workspace` (and a
//! developer's `cargo test`) never runs either role.
//!
//! # What the two processes prove, and how
//!
//! - the announcer announces `_neezer-lan._udp.local.` on `lo` and its own second
//!   daemon resolves it: `READY` is an EVENT (an announcement reached the wire and
//!   was resolved), never a sleep;
//! - the browser measures `browse start → Found` (the acceptance's latency), reads
//!   the announcement's raw record (instance name, host name, TXT) with the
//!   library's own browse — no packet capture anywhere — and asks the announcer to
//!   change its names and then to stop;
//! - the `Lost` after `stop` is the goodbye (RFC 6762 §10.1), measured on the
//!   browser's side within a second;
//! - the announcer asserts, by reading `/proc/self/task/*/comm`, that no thread
//!   named after `mdns-sd`'s daemon survives `stop()` or `Drop`.

// The two-process roles and all their helpers need `mdns-sd`, i.e. this crate's
// `mdns` feature. The Android build turns it off, and so does a
// `--no-default-features` check: the file then keeps only the platform-feed tests.
#![cfg_attr(not(feature = "mdns"), allow(dead_code, unused_imports))]

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use nz_lan::discovery::{Discovery, DiscoveryEvent, ExternalDiscovery, instance_name};

#[cfg(feature = "mdns")]
use nz_lan::discovery_mdns::MdnsDiscovery;

/// The lot's reserved ports: `45910 + 10 × 3` (worker rule 8 reserves a block per
/// lot, so two lots' harnesses never fight over a port).
const ANNOUNCE_PORT: u16 = 45_940;
const CONTROL_PORT: u16 = 45_941;
const BROWSE_PORT: u16 = 45_942;
const WITNESS_PORT: u16 = 45_943;
const DROP_PORT: u16 = 45_944;
const LOOPBACK_PROBE_PORT: u16 = 45_945;

/// How long any one phase of a role may wait before the harness fails.
const PHASE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a `Lost` may take to reach the peer: the goodbye itself is immediate,
/// the peer's cache-expiry pass adds up to its next browse retransmission (~1 s, the
/// interval mdns-sd queries at). Measured p50/max are reported by the harness test.
const GOODBYE_LIMIT: Duration = Duration::from_secs(2);

/// How long the browser looks for the link-local IPv6 address of the announcement
/// it just saw: the whole resolution arrives as one batch, so this only waits when
/// the address is not there at all.
const IPV6_GRACE: Duration = Duration::from_millis(750);

// ---------------------------------------------------------------------------
// 1. The platform feed: what Android's `NsdManager` hands the Rust core.
// ---------------------------------------------------------------------------

/// `(host, port)` from the platform becomes the events the connection code reads:
/// one `Found` per resolution, one `Lost` per loss, both naming the same instance
/// so a consumer can de-duplicate a dial by it.
#[test]
fn the_platform_feed_becomes_found_and_lost() {
    let discovery = ExternalDiscovery::new();
    let mut events = discovery.events();
    let address: IpAddr = "192.0.2.7".parse().expect("a documentation address");

    discovery.found(address, 45910);
    let found = events.try_recv().expect("a Found event");
    let DiscoveryEvent::Found { addr, instance } = found.clone() else {
        panic!("expected Found, got {found:?}");
    };
    assert_eq!(addr, SocketAddr::new(address, 45910));
    assert!(
        instance
            .strip_prefix("nz-")
            .is_some_and(|hex| hex.len() == 10),
        "{instance} is the opaque handle the crate names a platform peer by"
    );

    discovery.lost(address, 45910);
    assert_eq!(
        events.try_recv().expect("a Lost event"),
        DiscoveryEvent::Lost {
            instance: instance.clone()
        },
        "the loss names the announcement the resolution named"
    );
}

/// `nz-` and ten lowercase hex characters, never twice in a thousand draws: an
/// announcement cannot be predicted and two nodes on one segment never collide.
#[test]
fn the_announced_name_is_random() {
    let mut drawn = std::collections::HashSet::new();
    for _ in 0..1_000 {
        let name = instance_name();
        let hex = name.strip_prefix("nz-").expect("the `nz-` prefix");
        assert_eq!(hex.len(), 10, "{name}");
        assert!(
            hex.chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "{name} is lowercase hex"
        );
        assert!(drawn.insert(name), "a name was drawn twice");
    }
}

// ---------------------------------------------------------------------------
// 2. The two roles `tools/netns.sh` runs inside the namespace.
// ---------------------------------------------------------------------------

/// Whether this process was started by the harness as `wanted`, with the reason
/// printed when it was not (an unrun role must never look like a pass).
fn is_role(wanted: &str, name: &str) -> bool {
    match std::env::var("NZ_LAN_NETNS_ROLE") {
        Ok(role) if role == wanted => true,
        Ok(role) => {
            println!(
                "NZLAN SKIP {name}: the harness runs it as `{wanted}`, this process is `{role}`"
            );
            false
        }
        Err(_) => {
            println!(
                "NZLAN SKIP {name}: only runs inside tools/netns.sh (unshare -rn, multicast on lo)"
            );
            false
        }
    }
}

/// Whether the harness told this process to use the loopback mode: it does that when
/// the namespace's `dummy` device could not be created and it fell back to `lo`.
fn loopback_mode() -> bool {
    std::env::var("NZ_LAN_NETNS_LOOPBACK").is_ok_and(|value| value == "1")
}

fn env_u16(key: &str, default: u16) -> u16 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a current-thread runtime for the event waits")
}

/// The next discovery event, waiting on the channel (never on a clock) with a
/// bound so a broken announcement fails the run instead of hanging it.
fn next_event(
    rt: &tokio::runtime::Runtime,
    events: &mut tokio::sync::mpsc::Receiver<DiscoveryEvent>,
    within: Duration,
) -> Option<DiscoveryEvent> {
    rt.block_on(async {
        tokio::time::timeout(within, events.recv())
            .await
            .ok()
            .flatten()
    })
}

/// Waits for a `Found` for `port` — the other announcements on the segment (this
/// process's own one included) are ignored — and optionally skips instances we
/// already know.
fn wait_found(
    rt: &tokio::runtime::Runtime,
    events: &mut tokio::sync::mpsc::Receiver<DiscoveryEvent>,
    port: u16,
    not_instance: Option<&str>,
    within: Duration,
) -> Option<(SocketAddr, String)> {
    let deadline = Instant::now() + within;
    loop {
        let remaining = deadline.checked_duration_since(Instant::now())?;
        let event = next_event(rt, events, remaining)?;
        if let DiscoveryEvent::Found { addr, instance } = event
            && addr.port() == port
            && not_instance != Some(instance.as_str())
        {
            return Some((addr, instance));
        }
    }
}

/// Waits for BOTH halves of a network change — the `Found` of a different instance
/// at `port`, and the `Lost` of the old one — in one pass, because the library can
/// deliver them in either order.
#[cfg(feature = "mdns")]
fn wait_renamed(
    rt: &tokio::runtime::Runtime,
    events: &mut tokio::sync::mpsc::Receiver<DiscoveryEvent>,
    port: u16,
    old_instance: &str,
    within: Duration,
) -> Option<(Duration, (SocketAddr, String))> {
    let started = Instant::now();
    let deadline = started + within;
    let mut lost = None;
    let mut renamed = None;
    while lost.is_none() || renamed.is_none() {
        let remaining = deadline.checked_duration_since(Instant::now())?;
        match next_event(rt, events, remaining)? {
            DiscoveryEvent::Lost { instance } if instance == old_instance => {
                lost = Some(started.elapsed());
            }
            DiscoveryEvent::Found { addr, instance }
                if addr.port() == port && instance != old_instance && renamed.is_none() =>
            {
                renamed = Some((addr, instance));
            }
            _ => {}
        }
    }
    Some((lost?, renamed?))
}

/// Waits for the `Lost` naming `instance` and answers how long it took.
fn wait_lost(
    rt: &tokio::runtime::Runtime,
    events: &mut tokio::sync::mpsc::Receiver<DiscoveryEvent>,
    instance: &str,
    within: Duration,
) -> Option<Duration> {
    let started = Instant::now();
    let deadline = started + within;
    loop {
        let remaining = deadline.checked_duration_since(Instant::now())?;
        let event = next_event(rt, events, remaining)?;
        if let DiscoveryEvent::Lost { instance: lost } = event
            && lost == instance
        {
            return Some(started.elapsed());
        }
    }
}

/// The live threads whose OS name is `name` (`/proc/self/task/*/comm`).
#[cfg(target_os = "linux")]
fn threads_named(name: &str) -> usize {
    let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
        return 0;
    };
    tasks
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            std::fs::read_to_string(entry.path().join("comm"))
                .is_ok_and(|comm| comm.trim_end() == name)
        })
        .count()
}

#[cfg(not(target_os = "linux"))]
fn threads_named(_name: &str) -> usize {
    0
}

/// Asserts that no thread named `name` is alive, allowing the microseconds between
/// a `join` returning and the task disappearing from `/proc`.
fn assert_no_thread(name: &str, within: Duration) {
    let deadline = Instant::now() + within;
    loop {
        let live = threads_named(name);
        if live == 0 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{live} thread(s) named « {name} » survived the discovery being stopped"
        );
        std::thread::yield_now();
    }
}

/// The announcer: announce on the namespace's (dummy) interface, keep announcing
/// until the browser says otherwise, then say goodbye and prove that no thread
/// outlived the daemon.
#[test]
#[cfg(feature = "mdns")]
fn netns_announcer() {
    if !is_role("announce", "netns_announcer") {
        return;
    }

    let announce_port = env_u16("NZ_LAN_NETNS_ANNOUNCE_PORT", ANNOUNCE_PORT);
    let control_port = env_u16("NZ_LAN_NETNS_CONTROL_PORT", CONTROL_PORT);
    let witness_port = env_u16("NZ_LAN_NETNS_WITNESS_PORT", WITNESS_PORT);
    let drop_port = env_u16("NZ_LAN_NETNS_DROP_PORT", DROP_PORT);

    let control = UdpSocket::bind((Ipv4Addr::LOCALHOST, control_port))
        .expect("the harness's command port on loopback");
    control
        .set_read_timeout(Some(Duration::from_secs(60)))
        .expect("a read timeout, so a dead browser fails instead of hanging");

    let rt = runtime();
    // The APP's interface policy (every up, non-loopback interface): inside the
    // namespace that is the one dummy interface `tools/netns.sh` brings up, which is
    // exactly "a machine on one network". On a machine (or a namespace) where that
    // device cannot be created, the harness falls back to `lo` and asks for the
    // loopback mode instead.
    let loopback = loopback_mode();
    let discovery = MdnsDiscovery::new(loopback).expect("a discovery");
    let announced = Instant::now();
    discovery.start(announce_port).expect("announcing");
    // The announcer plays the peer the browser watches, so it does not read its own
    // events; taking the stream keeps the channel open rather than full of drops.
    let _own_events = discovery.events();

    // A SECOND daemon in this process is the only event-based proof that the
    // announcement reached the wire: it is a different browse, so it cannot be
    // handed our own registration by the daemon that made it. `READY` is that
    // event — there is no sleep anywhere in this test.
    let witness = MdnsDiscovery::new(loopback).expect("a witness discovery");
    witness.start(witness_port).expect("the witness announcing");
    let mut witnessed = witness.events();
    let (addr, instance) = wait_found(&rt, &mut witnessed, announce_port, None, PHASE_TIMEOUT)
        .expect("our own announcement must be resolvable inside this namespace");
    // The second number the harness reports: how long a node takes to BECOME
    // discoverable (RFC 6762 §8.1 probing: three queries 250 ms apart, then the
    // announcement). A `browse → Found` on an already-announced peer is the other
    // one, and it is what a tap waits for.
    println!(
        "NZLAN READY instance={instance} addr={addr} port={announce_port} announce_ms={}",
        announced.elapsed().as_millis()
    );

    let mut buffer = [0u8; 32];
    loop {
        let (len, _from) = control
            .recv_from(&mut buffer)
            .expect("a command from the browser");
        match &buffer[..len] {
            b"change" => {
                discovery.network_changed();
                println!("NZLAN RENAMED");
            }
            b"stop" => {
                // The goodbye happens HERE, while the browser is listening.
                discovery.stop();
                println!("NZLAN STOPPED");
                break;
            }
            other => println!("NZLAN IGNORED {}", String::from_utf8_lossy(other)),
        }
    }

    witness.stop();
    assert_no_thread("mDNS_daemon", Duration::from_secs(5));
    assert_no_thread("nz-lan-mdns", Duration::from_secs(5));

    // `Drop` stops what it started, with no explicit `stop()`.
    {
        let thrown_away = MdnsDiscovery::new(loopback).expect("a discovery");
        thrown_away.start(drop_port).expect("announcing");
        println!(
            "NZLAN THROWN_AWAY port={}",
            thrown_away.announced_port().unwrap_or(0)
        );
    }
    assert_no_thread("mDNS_daemon", Duration::from_secs(5));
    assert_no_thread("nz-lan-mdns", Duration::from_secs(5));
    println!("NZLAN THREADS_OK");
}

/// The browser: measure how long a fresh browse takes to find the other
/// process's announcement, read that announcement's raw record, make the
/// announcer change its names, then measure the goodbye.
#[test]
#[cfg(feature = "mdns")]
fn netns_browser() {
    if !is_role("browse", "netns_browser") {
        return;
    }

    let announce_port = env_u16("NZ_LAN_NETNS_ANNOUNCE_PORT", ANNOUNCE_PORT);
    let control_port = env_u16("NZ_LAN_NETNS_CONTROL_PORT", CONTROL_PORT);
    let browse_port = env_u16("NZ_LAN_NETNS_BROWSE_PORT", BROWSE_PORT);
    let require_ipv6 = std::env::var("NZ_LAN_NETNS_IPV6").is_ok_and(|value| value == "1");

    let control = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("a command socket");
    control
        .connect((Ipv4Addr::LOCALHOST, control_port))
        .expect("the announcer's command port");

    let rt = runtime();
    let loopback = loopback_mode();
    let mut wire =
        WireWatcher::start(announce_port, loopback).expect("the raw browse for the record");
    let discovery = MdnsDiscovery::new(loopback).expect("a discovery");
    discovery
        .start(browse_port)
        .expect("announcing and browsing");
    let mut events = discovery.events();

    // On a real interface, the other half of the interface policy is proven too: a
    // discovery restricted to `lo` must NEVER see an announcement made on a LAN
    // interface (`true` is "loopback only", not "loopback as well" — worker rule 8).
    // It runs for the whole run, so the check at the end costs no extra wait. In the
    // `lo` fallback the discovery under test IS the loopback one: nothing to compare.
    let mut loopback_only_events = None;
    let mut loopback_only = None;
    if !loopback {
        let probe = MdnsDiscovery::new(true).expect("a loopback-only discovery");
        probe
            .start(loopback_probe_port())
            .expect("announcing on loopback only");
        loopback_only_events = Some(probe.events());
        loopback_only = Some(probe);
    }

    // The acceptance's measurement: the time from the browse being started to the
    // first `Found` for the announcer's port.
    let started = Instant::now();
    let (addr, instance) = wait_found(&rt, &mut events, announce_port, None, PHASE_TIMEOUT)
        .expect("the other process's announcement must be found");
    let latency_ms = started.elapsed().as_millis();
    println!("NZLAN RUN latency_ms={latency_ms} instance={instance} addr={addr}");

    let first = wire
        .record(&instance, PHASE_TIMEOUT)
        .expect("the raw record of the announcement we just found");
    first.assert_anonymous();
    println!(
        "NZLAN WIRE instance={} host={} txt={} port={} addrs={:?}",
        first.instance,
        first.host,
        first.txt_display(),
        first.port,
        first.addresses
    );

    // The announcement also carries the namespace's link-local IPv6 address: our
    // own event for it must keep the interface scope (RFC 4007), or the address
    // reaches nothing.
    let scoped = wait_found_matching(&rt, &mut events, announce_port, |addr| {
        is_link_local_v6(addr)
    });
    match scoped {
        Some((address, _)) => {
            let SocketAddr::V6(v6) = address else {
                unreachable!("a link-local address is a V6 one")
            };
            let wire_scope = wire
                .scope_of(IpAddr::V6(*v6.ip()), PHASE_TIMEOUT)
                .expect("the announcement carries that link-local address");
            assert_eq!(
                v6.scope_id(),
                wire_scope,
                "the interface the address was resolved on is kept (RFC 4007)"
            );
            assert_ne!(
                v6.scope_id(),
                0,
                "a link-local address without its scope reaches nothing"
            );
            println!("NZLAN IPV6 addr={address} scope={}", v6.scope_id());
        }
        None => {
            assert!(
                !require_ipv6,
                "the namespace has a link-local IPv6 address on lo, so the announcement must carry it"
            );
            println!(
                "NZLAN IPV6 none: no link-local address in this namespace, scope check skipped"
            );
        }
    }

    // A network change re-draws both names. The re-drawn announcement can overtake
    // the goodbye of the old one (measured: the new name was resolved ~300 ms before
    // the old one was removed), so both are waited for in ONE loop — waiting for the
    // goodbye first would consume the `Found` that came with it.
    control
        .send(b"change")
        .expect("asking for a network change");
    let (lost, (_renamed_addr, renamed)) =
        wait_renamed(&rt, &mut events, announce_port, &instance, PHASE_TIMEOUT)
            .expect("the new name must be found and the old one said goodbye");
    let second = wire
        .record(&renamed, PHASE_TIMEOUT)
        .expect("the raw record of the re-drawn announcement");
    second.assert_anonymous();
    assert_ne!(renamed, instance, "the instance name is re-drawn");
    assert_ne!(
        second.host, first.host,
        "the host name is re-drawn with the instance name"
    );
    println!(
        "NZLAN CHANGED old={instance} new={renamed} host_old={} host_new={} lost_ms={}",
        first.host,
        second.host,
        lost.as_millis()
    );

    // The goodbye: the other side must see it within a second, or every peer waits
    // out the 75-minute TTL of a PTR record.
    control.send(b"stop").expect("asking the announcer to stop");
    let goodbye = wait_lost(&rt, &mut events, &renamed, Duration::from_secs(5))
        .expect("the goodbye must reach this side");
    // The goodbye is SENT as soon as `unregister` runs (mdns-sd broadcasts the
    // records with TTL 0 before it answers `OK`); the peer emits `Lost` at its next
    // cache-expiry pass, which the library performs when it next processes a packet
    // — its browse retransmission, i.e. up to ~1 s. The harness asserts the
    // distribution, this asserts that a goodbye always arrives at all.
    assert!(
        goodbye <= GOODBYE_LIMIT,
        "the goodbye took {} ms, far more than the peer's retransmission interval",
        goodbye.as_millis()
    );
    println!("NZLAN LOST_MS={}", goodbye.as_millis());

    // The negative half of the interface policy (see the top of this test).
    if let Some(mut probe_events) = loopback_only_events {
        let mut leaked = Vec::new();
        while let Ok(event) = probe_events.try_recv() {
            if let DiscoveryEvent::Found { addr, .. } = event
                && addr.port() == announce_port
            {
                leaked.push(addr);
            }
        }
        assert!(
            leaked.is_empty(),
            "a loopback-only discovery saw the LAN announcement at {leaked:?}"
        );
    }
    if let Some(probe) = loopback_only {
        probe.stop();
    }
    discovery.stop();
    wire.stop();
    println!("NZLAN BROWSER_OK");
}

/// Where the browser's loopback-only probe announces: a port of its own, so the two
/// modes never share a registration.
#[cfg(feature = "mdns")]
fn loopback_probe_port() -> u16 {
    env_u16("NZ_LAN_NETNS_LOOPBACK_PORT", LOOPBACK_PROBE_PORT)
}

/// The first `Found` for `port` whose address satisfies `matches` — a bounded wait,
/// because the addresses of one resolution arrive as one batch microseconds apart.
#[cfg(feature = "mdns")]
fn wait_found_matching(
    rt: &tokio::runtime::Runtime,
    events: &mut tokio::sync::mpsc::Receiver<DiscoveryEvent>,
    port: u16,
    matches: impl Fn(SocketAddr) -> bool,
) -> Option<(SocketAddr, String)> {
    let deadline = Instant::now() + IPV6_GRACE;
    loop {
        let remaining = deadline.checked_duration_since(Instant::now())?;
        let event = next_event(rt, events, remaining)?;
        if let DiscoveryEvent::Found { addr, instance } = event
            && addr.port() == port
            && matches(addr)
        {
            return Some((addr, instance));
        }
    }
}

#[cfg(feature = "mdns")]
fn is_link_local_v6(addr: SocketAddr) -> bool {
    match addr {
        SocketAddr::V6(v6) => (v6.ip().segments()[0] & 0xffc0) == 0xfe80,
        SocketAddr::V4(_) => false,
    }
}

/// One announcement as it came off the wire, read with the DNS-SD library's own
/// browse (never a packet capture).
#[cfg(feature = "mdns")]
#[derive(Debug, Clone)]
struct WireRecord {
    instance: String,
    host: String,
    port: u16,
    txt: Vec<(String, String)>,
    addresses: Vec<(IpAddr, u32)>,
}

#[cfg(feature = "mdns")]
impl WireRecord {
    /// The announcement carries nothing identifying: a random instance name, a
    /// random host name (never the machine's), TXT `v=1` and nothing else.
    fn assert_anonymous(&self) {
        assert!(
            is_random_name(&self.instance),
            "the instance name `{}` is not `nz-<10 hex>`",
            self.instance
        );
        let host = self.host.strip_suffix(".local.").unwrap_or(&self.host);
        assert!(
            is_random_name(host),
            "the host name `{}` is not `nz-<10 hex>` (the machine's host name is never announced)",
            self.host
        );
        assert_eq!(
            self.txt,
            vec![("v".to_owned(), "1".to_owned())],
            "the TXT record is exactly v=1 — no user, no device name, no hash"
        );
        assert!(self.port > 0, "the announcement carries the node's port");
    }

    fn txt_display(&self) -> String {
        self.txt
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join(",")
    }
}

#[cfg(feature = "mdns")]
fn is_random_name(label: &str) -> bool {
    label.strip_prefix("nz-").is_some_and(|hex| {
        hex.len() == 10
            && hex
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    })
}

/// A raw `mdns-sd` browse, used only to LOOK at what the other process announced:
/// the record's host name and TXT are not part of [`DiscoveryEvent`], so the shapes
/// the acceptance names are read here, from the wire.
#[cfg(feature = "mdns")]
struct WireWatcher {
    daemon: mdns_sd::ServiceDaemon,
    records: mpsc::Receiver<WireRecord>,
    thread: Option<JoinHandle<()>>,
    seen: Vec<WireRecord>,
}

#[cfg(feature = "mdns")]
impl WireWatcher {
    /// Browses with the same interface policy as the discovery under test and records
    /// every resolution whose port is the announcer's.
    fn start(port: u16, loopback: bool) -> Result<WireWatcher, String> {
        use mdns_sd::IfKind;
        let daemon = mdns_sd::ServiceDaemon::new().map_err(|error| error.to_string())?;
        if loopback {
            daemon
                .disable_interface(IfKind::All)
                .map_err(|error| error.to_string())?;
            daemon
                .enable_interface(vec![IfKind::LoopbackV4, IfKind::LoopbackV6])
                .map_err(|error| error.to_string())?;
        } else {
            daemon
                .disable_interface(vec![IfKind::LoopbackV4, IfKind::LoopbackV6])
                .map_err(|error| error.to_string())?;
        }
        let browse = daemon
            .browse(nz_lan::discovery::SERVICE_TYPE)
            .map_err(|error| error.to_string())?;

        let (tx, records) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("nz-lan-wire-watch".to_owned())
            .spawn(move || {
                while let Ok(event) = browse.recv() {
                    if let mdns_sd::ServiceEvent::ServiceResolved(resolved) = event
                        && resolved.get_port() == port
                    {
                        let _ = tx.send(WireRecord::of(&resolved));
                    }
                }
            })
            .map_err(|error| error.to_string())?;

        Ok(WireWatcher {
            daemon,
            records,
            thread: Some(thread),
            seen: Vec::new(),
        })
    }

    /// The latest record naming `instance`, waiting on the channel for one.
    ///
    /// "Latest" and not "first": the library resolves an instance once its A record
    /// is known and AGAIN when the AAAA records arrive, so the first record is the
    /// IPv4-only one.
    fn record(&mut self, instance: &str, within: Duration) -> Option<WireRecord> {
        let deadline = Instant::now() + within;
        loop {
            if let Some(found) = self
                .seen
                .iter()
                .rev()
                .find(|record| record.instance == instance)
            {
                return Some(found.clone());
            }
            let remaining = deadline.checked_duration_since(Instant::now())?;
            let record = self.records.recv_timeout(remaining).ok()?;
            self.seen.push(record);
        }
    }

    /// The scope `mdns-sd` itself gave `address`, waiting for a record that carries
    /// it (the AAAA records come in a second resolution).
    fn scope_of(&mut self, address: IpAddr, within: Duration) -> Option<u32> {
        let deadline = Instant::now() + within;
        loop {
            if let Some((_, scope)) = self
                .seen
                .iter()
                .flat_map(|record| record.addresses.iter())
                .find(|(ip, _)| *ip == address)
            {
                return Some(*scope);
            }
            let remaining = deadline.checked_duration_since(Instant::now())?;
            let record = self.records.recv_timeout(remaining).ok()?;
            self.seen.push(record);
        }
    }

    fn stop(mut self) {
        if let Ok(status) = self.daemon.shutdown() {
            let _ = status.recv_timeout(Duration::from_secs(2));
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(feature = "mdns")]
impl WireRecord {
    fn of(resolved: &mdns_sd::ResolvedService) -> WireRecord {
        let suffix = format!(".{}", nz_lan::discovery::SERVICE_TYPE);
        let instance = resolved
            .get_fullname()
            .strip_suffix(&suffix)
            .unwrap_or_else(|| resolved.get_fullname())
            .to_owned();
        let txt = resolved
            .get_properties()
            .iter()
            .map(|property| (property.key().to_owned(), property.val_str().to_owned()))
            .collect();
        let addresses = resolved
            .get_addresses()
            .iter()
            .map(|address| match address {
                mdns_sd::ScopedIp::V4(v4) => (IpAddr::V4(*v4.addr()), 0),
                mdns_sd::ScopedIp::V6(v6) => (IpAddr::V6(*v6.addr()), v6.scope_id().index),
                _ => (IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
            })
            .collect();
        WireRecord {
            instance,
            host: resolved.get_hostname().to_owned(),
            port: resolved.get_port(),
            txt,
            addresses,
        }
    }
}

// ---------------------------------------------------------------------------
// 3. The harness itself: 20 runs of the two processes in a user netns.
// ---------------------------------------------------------------------------

/// The note's acceptance, measured: 20 runs of `tools/netns.sh`, each a fresh
/// browse of a process that is already announcing.
///
/// A developer machine (or a CI runner) that does not allow unprivileged user
/// namespaces SKIPS with the harness's printed reason; with
/// `NZ_LAN_REQUIRE_NETNS=1` — the dedicated step of `desktop.yaml` — the same
/// situation FAILS, because a job that silently stopped testing discovery is worse
/// than no job.
#[test]
fn the_netns_harness_finds_the_other_process() {
    const RUNS: usize = 20;
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools/netns.sh");
    assert!(script.is_file(), "{} is missing", script.display());

    let output = Command::new("sh")
        .arg(&script)
        .arg("--runs")
        .arg(RUNS.to_string())
        .arg("--binary")
        .arg(std::env::current_exe().expect("this test binary"))
        .output()
        .expect("the harness runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    print!("{stdout}");
    if !stderr.is_empty() {
        print!("{stderr}");
    }

    assert!(
        output.status.success(),
        "tools/netns.sh failed ({}):\n{stdout}\n{stderr}",
        output.status
    );
    if stdout.contains("NZLAN HARNESS SKIP") {
        // The harness already said why; `NZ_LAN_REQUIRE_NETNS` is what decides
        // whether that is a skip or a failure, and it passed above.
        return;
    }

    let mut latencies = Vec::new();
    let mut goodbyes = Vec::new();
    let mut announces = Vec::new();
    for line in stdout.lines() {
        let Some(rest) = line.strip_prefix("NZLAN RUN ") else {
            continue;
        };
        let mut fields = rest.split(' ');
        let latency = fields
            .next()
            .and_then(|field| field.strip_prefix("latency_ms="))
            .and_then(|value| value.parse::<u64>().ok())
            .expect("every run reports its latency");
        latencies.push(latency);
    }
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("NZLAN LOST_MS=") {
            goodbyes.push(rest.trim().parse::<u64>().expect("a goodbye latency"));
        }
        if let Some(rest) = line.strip_prefix("NZLAN READY ") {
            let announce = rest
                .split(' ')
                .find_map(|field| field.strip_prefix("announce_ms="))
                .and_then(|value| value.parse::<u64>().ok());
            // The announce measurement is a bonus (it shows what probing costs), so a
            // missing one is not a failure; the discovery latency is the acceptance.
            if let Some(announce) = announce {
                announces.push(announce);
            }
        }
    }

    latencies.sort_unstable();
    let mut sorted_goodbyes = goodbyes.clone();
    sorted_goodbyes.sort_unstable();
    let mut sorted_announces = announces.clone();
    sorted_announces.sort_unstable();
    let p50 = nearest_rank(&latencies, 50);
    let p95 = nearest_rank(&latencies, 95);
    println!(
        "NZLAN p50={p50}ms p95={p95}ms runs={} goodbye_p50={}ms goodbye_max={}ms announce_p50={}ms announce_max={}ms",
        latencies.len(),
        nearest_rank(&sorted_goodbyes, 50),
        sorted_goodbyes.last().copied().unwrap_or(0),
        sorted_announces
            .get(nearest_rank_index(&sorted_announces, 50))
            .copied()
            .unwrap_or(0),
        sorted_announces.last().copied().unwrap_or(0)
    );
    // The goodbye leaves the responder at once; the PEER reports `Lost` at its next
    // cache-expiry pass, which mdns-sd runs when it next processes a packet — its
    // browse retransmission, i.e. ~1 s (measured 1 000–1 121 ms on this machine).
    // What matters against T1/T6 is that a peer stops dialling in a second instead
    // of waiting out the 4 500 s PTR TTL.
    assert_eq!(
        latencies.len(),
        RUNS,
        "every run must report one discovery measurement: \n{stdout}"
    );
    assert_eq!(
        goodbyes.len(),
        RUNS,
        "every run must report its goodbye: \n{stdout}"
    );
    assert_eq!(
        stdout.matches("NZLAN CHANGED").count(),
        RUNS,
        "every run re-draws the instance and host names"
    );
    assert_eq!(
        stdout.matches("NZLAN THREADS_OK").count(),
        RUNS,
        "every run proves no daemon thread survives: \n{stdout}"
    );
    assert_eq!(
        stdout.matches("NZLAN IPV6 ").count(),
        RUNS,
        "every run reports its IPv6 verdict: \n{stdout}"
    );

    assert!(
        sorted_goodbyes
            .get(nearest_rank_index(&sorted_goodbyes, 95))
            .copied()
            .unwrap_or(0)
            <= 1_500,
        "p95 goodbye {sorted_goodbyes:?} ms (≤ 1 500 ms expected)"
    );
    assert!(
        p50 <= 1_000,
        "p50 discovery {p50} ms (≤ 1 000 ms expected): {latencies:?}"
    );
    assert!(
        p95 <= 2_000,
        "p95 discovery {p95} ms (≤ 2 000 ms expected): {latencies:?}"
    );
    assert!(
        goodbyes.iter().all(|ms| *ms <= 2_000),
        "every goodbye must reach the other process: {goodbyes:?}"
    );
}

/// Nearest-rank percentile (`p` in 0..=100) of a sorted, non-empty slice.
fn nearest_rank(sorted: &[u64], p: usize) -> u64 {
    sorted[nearest_rank_index(sorted, p)]
}

fn nearest_rank_index(sorted: &[u64], p: usize) -> usize {
    let rank = (p * sorted.len()).div_ceil(100).max(1);
    (rank - 1).min(sorted.len().saturating_sub(1))
}
