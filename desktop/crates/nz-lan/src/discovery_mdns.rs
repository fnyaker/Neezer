//! The desktop's own discovery: `mdns-sd` browses and registers
//! `_neezer-lan._udp.local.` (docs/agents/lan-nodes.md, *Discovery*).
//!
//! Behind the default feature `mdns`. The Android build asks for this crate with
//! `default-features = false`: the phone browses through the platform's NSD daemon
//! and feeds [`crate::discovery::ExternalDiscovery`], so `mdns-sd`, its daemon
//! thread and its dependencies must not be in that graph at all.
//!
//! What this module keeps, and why:
//!
//! - the service type is [`SERVICE_TYPE`], the port is this node's QUIC port, and
//!   TXT is exactly `v=1` — nothing identifying ([`crate::discovery`]);
//! - the instance and host names are `nz-<10 random hex>` and are re-drawn by
//!   [`Discovery::network_changed`], because an announcement a stranger can follow
//!   across a network change is the one thing discovery would otherwise leak;
//! - [`Discovery::stop`] sends the goodbye at once (RFC 6762 §10.1; the peer emits
//!   `Lost` at its next cache-expiry pass, measured ~1 s) and shuts the daemon
//!   thread down, JOINING it: no thread outlives the discovery that started it;
//! - `ScopedIp` is not `Copy` and an IPv6 link-local address keeps its interface
//!   scope — losing it produces an address nothing can reach.
//!
//! # Which interfaces
//!
//! [`MdnsDiscovery::new`]'s flag is the whole policy, and it is deliberately two
//! modes and not three:
//!
//! - `false` (the app): every up, non-loopback interface. A node on `lo` is not a
//!   node on the LAN, and announcing `127.0.0.1` to the LAN would send peers to
//!   themselves.
//! - `true` (a test that has only loopback): **loopback only**. Not "loopback as
//!   well" — a test that added loopback must never also multicast on the machine's
//!   real network (worker rule 8).
//!
//! `tools/netns.sh` uses the APP's mode on the dummy interface of its namespace,
//! because that is what a machine on one network looks like, and because Linux does
//! not loop an IPv6 multicast packet on `lo` at all (measured: 3 sent, 0 received).

use std::net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV6};
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::Duration;

use mdns_sd::{IfKind, ScopedIp, ServiceDaemon, ServiceEvent, ServiceInfo};
use tokio::sync::mpsc;

use crate::discovery::{
    Discovery, DiscoveryEvent, EVENT_QUEUE, SERVICE_TYPE, TXT_VERSION_KEY, TXT_VERSION_VALUE,
    host_name, instance_name,
};
use crate::types::LanError;

/// How long [`Discovery::stop`] waits for the goodbye to be acknowledged before it
/// shuts the daemon down anyway: a stopped node must stop, never hang on its own
/// announcement (rule 2's "a button that seems to do nothing is a bug").
const UNREGISTER_TIMEOUT: Duration = Duration::from_millis(500);

/// How long [`Discovery::stop`] waits for the daemon to confirm its shutdown.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

/// The translation thread this module owns and joins.
const FORWARD_THREAD: &str = "nz-lan-mdns";

/// The name `mdns-sd` gives the thread it spawns; `stop()` leaves none of these
/// behind, and the netns suite asserts it by reading `/proc/self/task`.
pub const MDNS_DAEMON_THREAD: &str = "mDNS_daemon";

/// At most this many addresses are turned into events for one resolution.
///
/// A hostile or broken announcement can carry more address records than any real
/// node has; every count that comes from the wire is capped before it costs
/// anything (worker rule 10). Eight covers IPv4, IPv6 global and IPv6 link-local
/// on four interfaces.
const MAX_ADDRESSES: usize = 8;

/// The longest instance name that becomes an event: a DNS label is 63 bytes, and
/// anything longer is not an announcement this crate made.
const MAX_INSTANCE_LEN: usize = 63;

/// mDNS/DNS-SD discovery on the interfaces the caller allows.
///
/// Nothing runs before [`Discovery::start`]: `new` only prepares the event
/// channel, so a node that decides not to use the LAN (an older server, the
/// feature switched off) owns no thread and no socket at all. A machine where the
/// daemon cannot start answers the failure from `start`, with its cause.
pub struct MdnsDiscovery {
    /// Loopback only, or every up non-loopback interface (see the module docs).
    include_loopback: bool,
    /// Kept for the whole life of the discovery: `events()` hands the receiving
    /// half out once, and a `stop`/`start` cycle must not leave that half dead
    /// (the battery rules stop and restart discovery all day).
    tx: mpsc::Sender<DiscoveryEvent>,
    events: Mutex<Option<mpsc::Receiver<DiscoveryEvent>>>,
    running: Mutex<Option<Running>>,
}

/// One announced-and-browsing daemon.
struct Running {
    daemon: ServiceDaemon,
    /// The thread translating `mdns-sd`'s events; joined by `stop`.
    forward: Option<JoinHandle<()>>,
    /// What the goodbye is addressed to: the full DNS-SD name of the registration
    /// that is currently announced.
    fullname: String,
    port: u16,
}

impl MdnsDiscovery {
    /// A discovery for every up, non-loopback interface — or loopback ONLY when
    /// `include_loopback` is set (the tests and `tools/netns.sh`).
    pub fn new(include_loopback: bool) -> Result<Self, LanError> {
        let (tx, rx) = mpsc::channel(EVENT_QUEUE);
        Ok(MdnsDiscovery {
            include_loopback,
            tx,
            events: Mutex::new(Some(rx)),
            running: Mutex::new(None),
        })
    }

    /// Whether this instance was told to restrict itself to loopback.
    pub fn includes_loopback(&self) -> bool {
        self.include_loopback
    }

    /// Whether a daemon is announcing and browsing right now.
    pub fn is_running(&self) -> bool {
        self.lock_running().is_some()
    }

    /// The port the current announcement carries, if any.
    pub fn announced_port(&self) -> Option<u16> {
        self.lock_running().as_ref().map(|running| running.port)
    }

    fn lock_running(&self) -> std::sync::MutexGuard<'_, Option<Running>> {
        self.running.lock().unwrap_or_else(|err| err.into_inner())
    }

    /// The interface policy of the module docs, expressed as the daemon's own
    /// selection list (later entries win, so the order of these two calls is the
    /// policy).
    fn select_interfaces(&self, daemon: &ServiceDaemon) -> Result<(), LanError> {
        if self.include_loopback {
            daemon
                .disable_interface(IfKind::All)
                .map_err(mdns_failure)?;
            daemon
                .enable_interface(vec![IfKind::LoopbackV4, IfKind::LoopbackV6])
                .map_err(mdns_failure)?;
        } else {
            daemon
                .disable_interface(vec![IfKind::LoopbackV4, IfKind::LoopbackV6])
                .map_err(mdns_failure)?;
        }
        Ok(())
    }

    /// One announcement: a fresh instance and host name, the port, and TXT `v=1`.
    ///
    /// No address is passed in: `addr_auto` fills them from the interfaces the
    /// daemon selected, which is what makes the addresses follow a network change
    /// instead of being frozen at registration.
    fn service_info(&self, port: u16) -> Result<(ServiceInfo, String), LanError> {
        let instance = instance_name();
        let host = host_name();
        let mut properties = std::collections::HashMap::new();
        properties.insert(TXT_VERSION_KEY.to_owned(), TXT_VERSION_VALUE.to_owned());

        let info = ServiceInfo::new(SERVICE_TYPE, &instance, &host, (), port, properties)
            .map_err(mdns_failure)?
            .enable_addr_auto();
        let fullname = info.get_fullname().to_owned();
        Ok((info, fullname))
    }
}

impl std::fmt::Debug for MdnsDiscovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MdnsDiscovery")
            .field("include_loopback", &self.include_loopback)
            .field("running", &self.is_running())
            .finish()
    }
}

impl Discovery for MdnsDiscovery {
    fn start(&self, port: u16) -> Result<(), LanError> {
        // One announcement at a time: a second `start` re-announces on the new
        // port rather than leaving two services registered.
        self.stop();

        let daemon = ServiceDaemon::new().map_err(mdns_failure)?;
        self.select_interfaces(&daemon)?;

        let browse = daemon.browse(SERVICE_TYPE).map_err(mdns_failure)?;
        let (info, fullname) = self.service_info(port)?;
        daemon.register(info).map_err(mdns_failure)?;

        let forward = spawn_forwarder(browse, self.tx.clone());
        *self.lock_running() = Some(Running {
            daemon,
            forward,
            fullname,
            port,
        });
        Ok(())
    }

    fn stop(&self) {
        let running = self.lock_running().take();
        if let Some(running) = running {
            shutdown(running);
        }
    }

    fn events(&self) -> mpsc::Receiver<DiscoveryEvent> {
        let mut slot = self.events.lock().unwrap_or_else(|err| err.into_inner());
        slot.take().unwrap_or_else(|| {
            let (_tx, rx) = mpsc::channel(1);
            rx
        })
    }

    /// Re-draw the instance and host names and announce again, then leave the
    /// browsing to the browse that is already running.
    ///
    /// The goodbye for the old name goes first: a peer that was about to dial the
    /// old instance learns it is gone (`Lost`), and the new name cannot be
    /// correlated with it — that is the whole point of re-drawing on a network
    /// change.
    fn network_changed(&self) {
        let mut guard = self.lock_running();
        let Some(running) = guard.as_mut() else {
            return;
        };
        let Ok((info, fullname)) = self.service_info(running.port) else {
            tracing::warn!("découverte : impossible de préparer la nouvelle annonce");
            return;
        };
        if let Err(error) = running.daemon.unregister(&running.fullname) {
            // The old name was renamed by the daemon's conflict resolution or is
            // already gone: nothing to say goodbye to.
            tracing::debug!("découverte : ancienne annonce déjà retirée ({error})");
        }
        if let Err(error) = running.daemon.register(info) {
            tracing::warn!("découverte : impossible de réannoncer ({error})");
            return;
        }
        running.fullname = fullname;
    }
}

impl Drop for MdnsDiscovery {
    fn drop(&mut self) {
        // A discovery that is dropped stops what it started: the daemon thread is
        // joined here, so no thread outlives the value the app held.
        self.stop();
    }
}

/// Sends the goodbye, shuts the daemon down and joins the translating thread.
fn shutdown(running: Running) {
    // RFC 6762 §10.1: a goodbye is the same records with TTL 0, which is what makes
    // the other side emit `Lost` instead of waiting the record out (75 minutes for
    // PTR/TXT).
    unregister(&running.daemon, &running.fullname);

    // Closing our browse makes the translating thread's `recv()` fail, so the join
    // below cannot wait on a daemon that is already gone.
    if let Err(error) = running.daemon.stop_browse(SERVICE_TYPE) {
        tracing::debug!("découverte : arrêt du browse ({error})");
    }

    if let Ok(status) = running.daemon.shutdown() {
        let _ = status.recv_timeout(SHUTDOWN_TIMEOUT);
    }
    // Dropping the last handle is what makes the daemon thread return if the
    // shutdown command itself was lost.
    drop(running.daemon);

    if let Some(forward) = running.forward
        && forward.join().is_err()
    {
        tracing::debug!("découverte : le fil de traduction a terminé sur une panique");
    }
}

/// The goodbye, retried only on the library's "try again" (its command channel was
/// momentarily full — a stopped node must still say it is gone).
fn unregister(daemon: &ServiceDaemon, fullname: &str) {
    for _ in 0..3 {
        match daemon.unregister(fullname) {
            Ok(status) => {
                let _ = status.recv_timeout(UNREGISTER_TIMEOUT);
                return;
            }
            Err(mdns_sd::Error::Again) => std::thread::yield_now(),
            Err(error) => {
                tracing::debug!("découverte : arrêt de l'annonce ({error})");
                return;
            }
        }
    }
}

/// Starts the thread that turns `mdns-sd`'s events into [`DiscoveryEvent`]s.
fn spawn_forwarder(
    browse: mdns_sd::Receiver<ServiceEvent>,
    tx: mpsc::Sender<DiscoveryEvent>,
) -> Option<JoinHandle<()>> {
    match std::thread::Builder::new()
        .name(FORWARD_THREAD.to_owned())
        .spawn(move || forward(browse, tx))
    {
        Ok(handle) => Some(handle),
        Err(error) => {
            tracing::warn!("découverte : impossible de démarrer le fil de traduction ({error})");
            None
        }
    }
}

/// The translating loop: a plain thread, not a task, so it borrows no tokio
/// runtime and cannot outlive one.
fn forward(browse: mdns_sd::Receiver<ServiceEvent>, tx: mpsc::Sender<DiscoveryEvent>) {
    while let Ok(event) = browse.recv() {
        for translated in translate(event) {
            // A full queue (a flooded segment, a consumer stuck on the UI thread)
            // costs this hint, never the node: browse retransmits, and the queue is
            // bounded so the memory is too.
            let _ = tx.try_send(translated);
        }
    }
}

/// One `mdns-sd` event, as the events the connection code consumes.
///
/// A resolution becomes one `Found` per address (the shape the transport dials); a
/// removal becomes one `Lost` naming the instance. Everything else — search
/// bookkeeping, the daemon's own name changes — is not a peer appearing or
/// disappearing and is not translated.
fn translate(event: ServiceEvent) -> Vec<DiscoveryEvent> {
    match event {
        ServiceEvent::ServiceResolved(resolved) => {
            let instance = instance_of(resolved.get_fullname());
            if !is_plausible_instance(&instance) {
                return Vec::new();
            }
            let mut addresses: Vec<SocketAddr> = resolved
                .get_addresses()
                .iter()
                .filter_map(|ip| socket_addr(ip, resolved.get_port()))
                .collect();
            // Sorted and capped: two runs of the same announcement produce the same
            // first event, and a hostile one cannot cost more than eight dials.
            addresses.sort();
            addresses.truncate(MAX_ADDRESSES);
            addresses
                .into_iter()
                .map(|addr| DiscoveryEvent::Found {
                    addr,
                    instance: instance.clone(),
                })
                .collect()
        }
        ServiceEvent::ServiceRemoved(_ty, fullname) => {
            let instance = instance_of(&fullname);
            if is_plausible_instance(&instance) {
                vec![DiscoveryEvent::Lost { instance }]
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

/// `nz-abc._neezer-lan._udp.local.` → `nz-abc`.
fn instance_of(fullname: &str) -> String {
    let suffix = format!(".{SERVICE_TYPE}");
    fullname
        .strip_suffix(&suffix)
        .unwrap_or(fullname)
        .to_owned()
}

/// A label this crate could have announced: non-empty and a legal DNS label.
fn is_plausible_instance(instance: &str) -> bool {
    !instance.is_empty() && instance.len() <= MAX_INSTANCE_LEN
}

/// A `ScopedIp` becomes the `SocketAddr` the transport dials, scope included.
///
/// `None` for an address family this build does not dial (`ScopedIp` is
/// `#[non_exhaustive]`): a future family is skipped, never guessed at.
fn socket_addr(ip: &ScopedIp, port: u16) -> Option<SocketAddr> {
    match ip {
        ScopedIp::V4(v4) => Some(SocketAddr::new(IpAddr::V4(*v4.addr()), port)),
        ScopedIp::V6(v6) => {
            let scope = ipv6_scope(*v6.addr(), v6.scope_id().index);
            Some(SocketAddr::V6(SocketAddrV6::new(
                *v6.addr(),
                port,
                0,
                scope,
            )))
        }
        _ => None,
    }
}

/// The scope id an address keeps: the one it was resolved with for a link-local
/// (or multicast) IPv6 address, `0` for anything else.
///
/// RFC 4007: a scope id names an interface, and only a scoped address has one. A
/// link-local address without its scope is the bug this function exists to avoid
/// (it reaches nothing); a global address carrying one is noise, and can make a
/// connect fail on some stacks.
fn ipv6_scope(ip: Ipv6Addr, scope_id: u32) -> u32 {
    let is_link_local = (ip.segments()[0] & 0xffc0) == 0xfe80;
    if is_link_local || ip.is_multicast() {
        scope_id
    } else {
        0
    }
}

/// The one error every `mdns-sd` failure becomes, with the library's cause kept for
/// this device's own log (never for a peer).
fn mdns_failure(error: mdns_sd::Error) -> LanError {
    LanError::Internal(format!("découverte mDNS : {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn resolved(ips: &[&str], port: u16) -> ServiceEvent {
        let mut properties = HashMap::new();
        properties.insert(TXT_VERSION_KEY.to_owned(), TXT_VERSION_VALUE.to_owned());
        let info = ServiceInfo::new(
            SERVICE_TYPE,
            "nz-0123456789",
            "nz-9876543210.local.",
            ips.join(",").as_str(),
            port,
            properties,
        )
        .expect("a service info");
        ServiceEvent::ServiceResolved(Box::new(info.as_resolved_service()))
    }

    /// A resolution becomes one `Found` per address, IPv4 first (so two runs agree
    /// on the first event), each naming the same random instance.
    #[test]
    fn a_resolution_becomes_one_found_per_address() {
        let events = translate(resolved(&["2001:db8::1", "192.0.2.7"], 45910));
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0],
            DiscoveryEvent::Found {
                addr: "192.0.2.7:45910".parse().expect("addr"),
                instance: "nz-0123456789".to_owned(),
            }
        );
        assert_eq!(events[1].instance(), "nz-0123456789");
        assert!(matches!(
            events[1],
            DiscoveryEvent::Found { addr, .. } if addr.is_ipv6()
        ));
    }

    /// A goodbye becomes exactly one `Lost` naming the instance the announcements
    /// carried.
    #[test]
    fn a_goodbye_becomes_one_lost() {
        let events = translate(ServiceEvent::ServiceRemoved(
            SERVICE_TYPE.to_owned(),
            "nz-0123456789._neezer-lan._udp.local.".to_owned(),
        ));
        assert_eq!(
            events,
            vec![DiscoveryEvent::Lost {
                instance: "nz-0123456789".to_owned()
            }]
        );
    }

    /// An announcement with more address records than any node has costs at most
    /// eight events (a count from the wire is capped before it costs anything).
    #[test]
    fn a_flooded_resolution_is_capped() {
        let names: Vec<String> = (1..=40).map(|n| format!("192.0.2.{n}")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        assert_eq!(translate(resolved(&names, 45910)).len(), MAX_ADDRESSES);
    }

    /// A fullname that is not a legal label, or that is not ours, yields nothing:
    /// this crate announces three-to-thirteen-byte labels, not anything a rogue
    /// responder felt like.
    #[test]
    fn an_implausible_instance_is_ignored() {
        assert!(
            translate(ServiceEvent::ServiceRemoved(
                SERVICE_TYPE.to_owned(),
                format!("{}._neezer-lan._udp.local.", "x".repeat(64)),
            ))
            .is_empty()
        );
        assert!(
            translate(ServiceEvent::ServiceRemoved(
                SERVICE_TYPE.to_owned(),
                String::new(),
            ))
            .is_empty()
        );
    }

    /// A link-local IPv6 address keeps the interface it was resolved on; a global
    /// one carries no scope (RFC 4007), because a global address with a scope id
    /// is noise and can fail a connect.
    #[test]
    fn a_link_local_address_keeps_its_scope() {
        let link_local: Ipv6Addr = "fe80::1".parse().expect("addr");
        let global: Ipv6Addr = "2001:db8::1".parse().expect("addr");
        assert_eq!(ipv6_scope(link_local, 7), 7, "the scope is kept");
        assert_eq!(ipv6_scope(global, 7), 0, "a global address has no scope");
        assert_eq!(ipv6_scope("::1".parse().expect("addr"), 7), 0);
        assert_eq!(ipv6_scope("ff02::fb".parse().expect("addr"), 3), 3);
    }

    /// The mapping from `mdns-sd`'s address type to the one the transport dials
    /// keeps the port and the scope, and answers the loopback address unchanged.
    #[test]
    fn the_scoped_address_becomes_a_socket_address() {
        let loopback: IpAddr = "127.0.0.1".parse().expect("loopback");
        let mapped = socket_addr(&ScopedIp::from(loopback), 45940).expect("a dialable address");
        assert_eq!(mapped, SocketAddr::new(loopback, 45940));
        let v6: IpAddr = "::1".parse().expect("addr");
        let mapped = socket_addr(&ScopedIp::from(v6), 45940).expect("a dialable address");
        assert_eq!(mapped, SocketAddr::new(v6, 45940));
    }

    /// A discovery that was never started owns no thread and no socket, and its
    /// `stop`, its `Drop` and its `network_changed` all do nothing.
    #[test]
    fn nothing_runs_before_start() {
        let discovery = MdnsDiscovery::new(true).expect("a discovery");
        assert!(!discovery.is_running());
        assert_eq!(discovery.announced_port(), None);
        discovery.network_changed();
        discovery.stop();
        assert!(!discovery.is_running());
        drop(discovery);
    }
}
