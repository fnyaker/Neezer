//! Finding the other devices, without telling a stranger anything
//! (docs/agents/lan-nodes.md, *Discovery*).
//!
//! **LN0 declares the seam; LN3 fills it.** Two implementations sit behind
//! [`Discovery`]:
//!
//! - the desktop's own (`discovery_mdns.rs`, feature `mdns`): `mdns-sd` browses
//!   and registers `_neezer-lan._udp.local.` on every up, non-loopback interface;
//! - the platform's ([`ExternalDiscovery`]): Android's `NsdManager` (and, later,
//!   iOS's `NWBrowser`) browses in Kotlin and feeds [`ExternalDiscovery::found`] /
//!   [`ExternalDiscovery::lost`] through the FFI.
//!
//! The announcement carries NOTHING identifying: an instance name and a host name
//! re-drawn at every network change, and TXT `v=1` only. No user, no device name,
//! no hash of an identifier (AirDrop's truncated hashes were reversed by
//! dictionary). A stranger learns "a Neezer node is at this address" and nothing
//! else (T1).
//!
//! Discovery is **only a hint**: an announcement proves nothing, so a discovered
//! node is dialled at once and authentication decides everything. A node that
//! refuses us, or that we refuse, is not dialled again for 10 minutes per
//! announcement instance.

use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;

use crate::types::LanError;

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

/// The discovery a platform performs itself and feeds in.
///
/// **LN0 declares it; LN3 wires the events, LN14 calls it from Kotlin.** Each
/// resolved `(host, port)` becomes a [`DiscoveryEvent::Found`], each loss a
/// [`DiscoveryEvent::Lost`].
#[derive(Debug)]
pub struct ExternalDiscovery {
    /// The receiving half, taken by whoever consumes the events.
    events: Mutex<Option<tokio::sync::mpsc::Receiver<DiscoveryEvent>>>,
}

impl ExternalDiscovery {
    /// A discovery with nothing announced and nothing found yet.
    pub fn new() -> Self {
        // A closed channel: the sender is dropped at once, so a consumer that
        // reads before any platform feeds it simply waits.
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        ExternalDiscovery {
            events: Mutex::new(Some(rx)),
        }
    }

    /// The platform resolved a node at `ip`:`port`.
    pub fn found(&self, ip: IpAddr, port: u16) {
        let _ = (ip, port);
    }

    /// The platform lost a node it had resolved.
    pub fn lost(&self, ip: IpAddr, port: u16) {
        let _ = (ip, port);
    }
}

impl Default for ExternalDiscovery {
    fn default() -> Self {
        Self::new()
    }
}

impl Discovery for ExternalDiscovery {
    /// Nothing to start: the platform browses, and its results arrive through
    /// [`ExternalDiscovery::found`].
    fn start(&self, port: u16) -> Result<(), LanError> {
        let _ = port;
        Ok(())
    }

    fn stop(&self) {}

    fn events(&self) -> tokio::sync::mpsc::Receiver<DiscoveryEvent> {
        let mut slot = self.events.lock().unwrap_or_else(|err| err.into_inner());
        match slot.take() {
            Some(events) => events,
            // Already taken: answer a channel that never yields, rather than
            // inventing a second consumer of the same feed.
            None => {
                let (_tx, rx) = tokio::sync::mpsc::channel(1);
                rx
            }
        }
    }

    fn network_changed(&self) {}
}

/// `"nz-"` and ten random hex characters: the instance and host name of one
/// announcement, re-drawn at every network change.
///
/// **LN3 fills it** (10 hex characters from the OS RNG); the empty string is LN0's
/// "nothing announced yet".
pub fn instance_name() -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// LN0 answers nothing for the name; LN3 makes it `^nz-[0-9a-f]{10}$`.
    #[test]
    fn the_instance_name_is_empty_until_ln3_fills_it() {
        assert_eq!(instance_name(), "");
    }
}
