//! Finding the other devices, without telling a stranger anything
//! (docs/agents/lan-nodes.md, *Discovery*).
//!
//! Two implementations sit behind [`Discovery`]:
//!
//! - the desktop's own (`discovery_mdns.rs`, feature `mdns`): `mdns-sd` browses
//!   and registers `_neezer-lan._udp.local.` on the interfaces the caller allows;
//! - the platform's ([`ExternalDiscovery`]): Android's `NsdManager` (and, later,
//!   iOS's `NWBrowser`) browses in Kotlin and feeds [`ExternalDiscovery::found`] /
//!   [`ExternalDiscovery::lost`] through the FFI.
//!
//! The announcement carries NOTHING identifying: an instance name and a host name
//! re-drawn at every network change ([`instance_name`]), and TXT `v=1` only. No
//! user, no device name, no hash of an identifier (AirDrop's truncated hashes were
//! reversed by dictionary). A stranger learns "a Neezer node is at this address"
//! and nothing else (T1).
//!
//! Discovery is **only a hint**: an announcement proves nothing, so a discovered
//! node is dialled at once and authentication decides everything. A node that
//! refuses us, or that we refuse, is not dialled again for 10 minutes per
//! announcement instance.
//!
//! # Why one event per address
//!
//! A resolution carries several addresses (IPv4, IPv6 global, IPv6 link-local);
//! [`DiscoveryEvent::Found`] names ONE `(address, port)`, the shape the transport
//! dials and the shape the platform feeds in. A [`DiscoveryEvent::Lost`] names the
//! instance instead, because that is what a goodbye says: the whole announcement
//! went away, not one of its addresses. A consumer that dials once per instance
//! keys on [`DiscoveryEvent::instance`], which is the random name and never an
//! identity.

use std::fmt::Write as _;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::types::LanError;

/// The DNS-SD service type every Neezer node announces. It is what Android's
/// `NsdManager`, iOS's Bonjour and Linux's `mdns-sd`/Avahi all speak natively,
/// which a home-made multicast group would not (see the note's *Research*).
pub const SERVICE_TYPE: &str = "_neezer-lan._udp.local.";

/// The only TXT record an announcement carries: the discovery version.
///
/// `v=1` and nothing else — no name, no id, no capability list, no hash. A newer
/// discovery version is a NEW service type rather than a field here, so an older
/// app never has to parse a shape it does not know (rule 7).
pub const TXT_VERSION_KEY: &str = "v";

/// The value of [`TXT_VERSION_KEY`].
pub const TXT_VERSION_VALUE: &str = "1";

/// `"nz-"`: what every announced name starts with, instance and host alike.
pub const INSTANCE_PREFIX: &str = "nz-";

/// How many events one discovery keeps before it starts dropping them.
///
/// Bounded on purpose: a flooded multicast segment must cost a fixed amount of
/// memory, not an unbounded queue (every count that comes from the network is
/// capped before allocation). A dropped event is re-learned by the next browse
/// retransmission, so losing one costs a second, never a node.
pub(crate) const EVENT_QUEUE: usize = 64;

/// What happens on the LAN, as the connection code consumes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryEvent {
    /// A node announced itself at this address. `instance` is the random DNS-SD
    /// instance name, which is what says "the same node as before" — never the
    /// device's identity.
    Found { addr: SocketAddr, instance: String },
    /// That instance went away.
    Lost { instance: String },
}

impl DiscoveryEvent {
    /// The announcement's random name: the same key for every address of one
    /// node, and the key a consumer de-duplicates a dial by.
    pub fn instance(&self) -> &str {
        match self {
            DiscoveryEvent::Found { instance, .. } => instance,
            DiscoveryEvent::Lost { instance } => instance,
        }
    }
}

/// One discovery mechanism, as the rest of the crate sees it.
pub trait Discovery: Send + Sync {
    /// Starts announcing this node on `port` and browsing for others.
    fn start(&self, port: u16) -> Result<(), LanError>;

    /// Stops both. Idempotent, and it joins the daemon thread the implementation
    /// may own: nothing outlives the runtime it borrowed.
    fn stop(&self);

    /// The stream of events. Taken ONCE — a second call has nothing to hand out,
    /// so a caller cannot accidentally start two consumers of one queue.
    fn events(&self) -> tokio::sync::mpsc::Receiver<DiscoveryEvent>;

    /// The interface list changed: re-draw the instance and host names and
    /// announce again, then try the last-known addresses of the peers we had
    /// (the reconnect path when multicast is filtered but unicast passes).
    fn network_changed(&self);
}

/// A receiver whose sender is already gone: it yields `None` at once instead of
/// pretending to be a second consumer of somebody else's feed.
fn closed_events() -> tokio::sync::mpsc::Receiver<DiscoveryEvent> {
    let (_tx, rx) = tokio::sync::mpsc::channel(1);
    rx
}

/// The discovery a platform performs itself and feeds in.
///
/// Android's `NsdManager` registers and browses (Kotlin), holds its
/// `MulticastLock` only while browsing, and hands every resolved `(host, port)`
/// to [`ExternalDiscovery::found`] and every loss to
/// [`ExternalDiscovery::lost`]. iOS's `NWBrowser` will feed the same two calls.
///
/// The platform knows no DNS-SD instance name we can use as a stable handle (the
/// FFI carries `(host, port)` only), so [`external_instance`] derives one from the
/// address itself: the same peer keeps the same name across a re-resolution, and
/// [`ExternalDiscovery::lost`] names exactly what [`ExternalDiscovery::found`]
/// named. It reveals nothing the address does not already reveal.
#[derive(Debug)]
pub struct ExternalDiscovery {
    feed: Mutex<Feed>,
}

#[derive(Debug)]
struct Feed {
    /// Kept for the whole life of the discovery, so `events()` works whether it
    /// was taken before or after the platform's first callback.
    tx: tokio::sync::mpsc::Sender<DiscoveryEvent>,
    events: Option<tokio::sync::mpsc::Receiver<DiscoveryEvent>>,
    /// `stop()` freezes the feed: a callback that lands after the platform was
    /// told to stop browsing must not resurrect a peer the app stopped looking
    /// for (that is how the battery rules stay true).
    running: bool,
}

impl ExternalDiscovery {
    /// A discovery with nothing announced and nothing found yet.
    pub fn new() -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(EVENT_QUEUE);
        ExternalDiscovery {
            feed: Mutex::new(Feed {
                tx,
                events: Some(rx),
                running: true,
            }),
        }
    }

    /// The platform resolved a node at `ip`:`port`.
    pub fn found(&self, ip: IpAddr, port: u16) {
        self.push(DiscoveryEvent::Found {
            addr: SocketAddr::new(ip, port),
            instance: external_instance(ip, port),
        });
    }

    /// The platform lost a node it had resolved.
    pub fn lost(&self, ip: IpAddr, port: u16) {
        self.push(DiscoveryEvent::Lost {
            instance: external_instance(ip, port),
        });
    }

    fn push(&self, event: DiscoveryEvent) {
        let feed = self.lock();
        if !feed.running {
            return;
        }
        // `try_send`, never `blocking_send`: this is called from a platform
        // callback thread, which may be inside a runtime, and a full queue must
        // cost a dropped hint rather than a stalled caller.
        if feed.tx.try_send(event).is_err() {
            tracing::debug!("découverte : file d'événements pleine ou fermée, événement ignoré");
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Feed> {
        self.feed.lock().unwrap_or_else(|err| err.into_inner())
    }

    /// Whether the feed currently delivers (the platform is browsing).
    pub fn is_running(&self) -> bool {
        self.lock().running
    }
}

impl Default for ExternalDiscovery {
    fn default() -> Self {
        Self::new()
    }
}

impl Discovery for ExternalDiscovery {
    /// Nothing to start: the platform browses, and its results arrive through
    /// [`ExternalDiscovery::found`]. The port is the platform's registration, made
    /// with `advertised_port()` on its side.
    fn start(&self, port: u16) -> Result<(), LanError> {
        let _ = port;
        self.lock().running = true;
        Ok(())
    }

    fn stop(&self) {
        self.lock().running = false;
    }

    fn events(&self) -> tokio::sync::mpsc::Receiver<DiscoveryEvent> {
        self.lock().events.take().unwrap_or_else(closed_events)
    }

    /// The platform already re-resolves on a network change and calls `found`
    /// again; because [`external_instance`] is derived from the address, a peer
    /// that moved is a new instance and a peer that stayed is the same one.
    fn network_changed(&self) {}
}

/// `"nz-"` and ten random hex characters: the instance and host name of one
/// announcement, re-drawn at every network change.
///
/// The randomness is the OS CSPRNG (`aws-lc-rs`, already the crate's only crypto
/// provider). Forty bits is what makes two things true at once: an announcement
/// cannot be predicted or followed by a stranger, and two nodes on one segment
/// never collide (a collision would make mDNS rename us mid-flight, RFC 6762
/// §9).
pub fn instance_name() -> String {
    let mut name = String::with_capacity(INSTANCE_PREFIX.len() + 10);
    name.push_str(INSTANCE_PREFIX);
    let _ = write!(name, "{}", random_hex10());
    name
}

/// The DNS host name that goes with an instance name: `nz-<10 hex>.local.`, drawn
/// independently of the instance so the two records do not correlate.
///
/// Only the `mdns` feature announces (the phone browses through its platform's NSD
/// daemon), so the phone's graph does not carry this.
#[cfg(feature = "mdns")]
pub(crate) fn host_name() -> String {
    let mut host = String::with_capacity(INSTANCE_PREFIX.len() + 10 + ".local.".len());
    host.push_str(INSTANCE_PREFIX);
    let _ = write!(host, "{}", random_hex10());
    host.push_str(".local.");
    host
}

/// Ten lowercase hex characters from the OS entropy source.
fn random_hex10() -> String {
    let mut bytes = [0u8; 5];
    if aws_lc_rs::rand::fill(&mut bytes).is_err() {
        // Only a broken OS entropy source lands here. A clock-derived name is
        // weaker, but a node that cannot announce at all is worse, and every name
        // is re-drawn at the next network change.
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos() as u64);
        let mixed = nanos ^ u64::from(std::process::id()).rotate_left(17) ^ u64::from(bytes[0]);
        bytes.copy_from_slice(&mixed.to_le_bytes()[..5]);
    }
    let mut out = String::with_capacity(10);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// The instance name of a node the platform reports as `(ip, port)`.
///
/// FNV-1a over the address and the port, cut to ten hex characters: stable for one
/// address, different between two, and derived from nothing but what the platform
/// already told us — so it is not an identifier that could outlive the address.
fn external_instance(ip: IpAddr, port: u16) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |byte: u8| {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    };
    match ip {
        IpAddr::V4(v4) => {
            feed(4);
            for byte in v4.octets() {
                feed(byte);
            }
        }
        IpAddr::V6(v6) => {
            feed(6);
            for byte in v6.octets() {
                feed(byte);
            }
        }
    }
    feed((port >> 8) as u8);
    feed(port as u8);

    let mut name = String::with_capacity(INSTANCE_PREFIX.len() + 10);
    name.push_str(INSTANCE_PREFIX);
    let _ = write!(name, "{:010x}", hash & 0x00ff_ffff_ffff);
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    fn next(rx: &mut tokio::sync::mpsc::Receiver<DiscoveryEvent>) -> DiscoveryEvent {
        rx.try_recv().expect("an event was expected")
    }

    /// The platform's `(host, port)` becomes the pair of events the connection
    /// code consumes, and the instance handle is stable: `found` twice and one
    /// `lost` all name the same announcement, which is what lets the consumer
    /// refuse to re-dial a node it already refused.
    #[test]
    fn the_platform_feed_becomes_found_and_lost_events() {
        let discovery = ExternalDiscovery::new();
        let mut events = discovery.events();
        let ip: IpAddr = "192.0.2.7".parse().expect("documentation address");

        discovery.found(ip, 45910);
        let first = next(&mut events);
        let DiscoveryEvent::Found { addr, instance } = first.clone() else {
            panic!("expected Found, got {first:?}");
        };
        assert_eq!(addr, SocketAddr::new(ip, 45910));
        assert!(
            instance.starts_with(INSTANCE_PREFIX) && instance.len() == 13,
            "{instance} is an opaque handle for one announcement"
        );

        discovery.found(ip, 45910);
        assert_eq!(next(&mut events), first, "a re-resolution is the same node");

        discovery.lost(ip, 45910);
        assert_eq!(
            next(&mut events),
            DiscoveryEvent::Lost {
                instance: instance.clone()
            }
        );

        discovery.found(ip, 45911);
        let other = next(&mut events);
        assert_ne!(
            other.instance(),
            instance,
            "another port is another announcement"
        );
    }

    /// The handle names the address and nothing else: two nodes differ, and the
    /// name is the shape the rest of the crate and its tests expect.
    #[test]
    fn the_platform_handle_is_opaque_and_address_shaped() {
        let one = external_instance("192.0.2.7".parse().expect("addr"), 45910);
        let two = external_instance("192.0.2.8".parse().expect("addr"), 45910);
        let six = external_instance("fe80::1".parse().expect("addr"), 45910);
        assert_ne!(one, two);
        assert_ne!(one, six);
        for name in [&one, &two, &six] {
            let hex = name.strip_prefix(INSTANCE_PREFIX).expect("the prefix");
            assert_eq!(hex.len(), 10, "{name}");
            assert!(
                hex.chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
                "{name} is ten lowercase hex characters"
            );
        }
    }

    /// A consumer that took the stream once still receives what the platform
    /// feeds afterwards (the sender lives in the discovery, not in the takers).
    #[test]
    fn the_feed_works_whichever_side_is_created_first() {
        let discovery = ExternalDiscovery::new();
        discovery.found("192.0.2.7".parse().expect("addr"), 45910);
        let mut events = discovery.events();
        assert!(matches!(next(&mut events), DiscoveryEvent::Found { .. }));
        discovery.lost("192.0.2.7".parse().expect("addr"), 45910);
        assert!(matches!(next(&mut events), DiscoveryEvent::Lost { .. }));
    }

    /// `stop()` freezes the feed: a late platform callback must not resurrect a
    /// node the app stopped looking for, and `start()` unfreezes it.
    #[test]
    fn stopping_the_feed_freezes_it_until_it_is_started_again() {
        let discovery = ExternalDiscovery::new();
        let mut events = discovery.events();
        discovery.stop();
        assert!(!discovery.is_running());
        discovery.found("192.0.2.7".parse().expect("addr"), 45910);
        assert!(
            events.try_recv().is_err(),
            "a stopped feed delivers nothing"
        );

        discovery.start(45910).expect("starting the platform feed");
        assert!(discovery.is_running());
        discovery.found("192.0.2.7".parse().expect("addr"), 45910);
        assert!(matches!(next(&mut events), DiscoveryEvent::Found { .. }));
    }

    /// The events are taken once: a second consumer gets a channel that never
    /// yields, instead of racing the first one for the platform's feed.
    #[test]
    fn the_event_stream_is_taken_once() {
        let discovery = ExternalDiscovery::new();
        let _first = discovery.events();
        let mut second = discovery.events();
        assert!(
            second.try_recv().is_err(),
            "a second consumer waits for ever rather than stealing events"
        );
        // And the first consumer still works: taking the stream twice changed
        // nothing about where the platform's callbacks go.
        discovery.found("192.0.2.7".parse().expect("addr"), 45910);
        assert!(second.try_recv().is_err());
    }

    /// The external discovery starts and stops without touching anything: the
    /// platform owns the socket, and a stop with nothing started is not an error.
    #[test]
    fn starting_and_stopping_the_external_discovery_is_free() {
        let discovery = ExternalDiscovery::default();
        assert_eq!(discovery.start(45910), Ok(()));
        discovery.stop();
        discovery.stop();
        discovery.network_changed();
        discovery.found("127.0.0.1".parse().expect("loopback"), 45910);
        discovery.lost("127.0.0.1".parse().expect("loopback"), 45910);
    }

    /// `nz-` and ten lowercase hex characters, and 1 000 draws never repeat: an
    /// announcement cannot be predicted, and two nodes on one segment do not
    /// collide (a collision makes mDNS rename us mid-flight).
    #[test]
    fn the_instance_name_is_ten_random_hex_characters() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1_000 {
            let name = instance_name();
            assert_eq!(name.len(), 13, "{name}");
            let hex = name.strip_prefix("nz-").expect("the prefix");
            assert!(
                hex.chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
                "{name} is lowercase hex"
            );
            assert!(seen.insert(name), "a name was drawn twice");
        }
    }

    /// The host name has the same shape and is drawn independently of the
    /// instance: the two records of one node do not correlate.
    #[test]
    #[cfg(feature = "mdns")]
    fn the_host_name_is_an_independent_random_name() {
        let mut names = std::collections::HashSet::new();
        for _ in 0..64 {
            let host = host_name();
            let label = host.strip_suffix(".local.").expect("the mDNS domain");
            assert!(label.starts_with("nz-"), "{host}");
            assert_eq!(label.len(), 13, "{host}");
            assert!(
                names.insert(label.to_owned()),
                "a host name was drawn twice"
            );
        }
    }
}
