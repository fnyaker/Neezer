//! The desktop's own discovery: `mdns-sd` browses and registers
//! `_neezer-lan._udp.local.` (docs/agents/lan-nodes.md, *Discovery*).
//!
//! **Behind the default feature `mdns`, and a stub until LN3.** The Android build
//! asks for this crate with `default-features = false`: the phone browses through
//! the platform's NSD daemon and feeds [`crate::discovery::ExternalDiscovery`], so
//! `mdns-sd`, its daemon thread and its dependencies must not be in that graph at
//! all.
//!
//! What LN3 must keep, stated here so the module is not just a name:
//!
//! - the service type is `_neezer-lan._udp.local.`, the port is this node's QUIC
//!   port, the instance and host names are `nz-<10 random hex>`, and TXT is
//!   exactly `v=1` — nothing identifying;
//! - `stop()` sends the goodbye (the other side gets `Lost` within a second) and
//!   shuts the daemon thread down, joining it: a thread named after `mdns-sd`
//!   must not survive the stop;
//! - `ScopedIp` is not `Copy` and an IPv6 link-local address keeps its interface
//!   scope — losing it produces an address nothing can reach.

use crate::types::LanError;

/// mDNS/DNS-SD discovery on the interfaces the caller allows.
///
/// **LN3 fills the bodies** (`mdns_sd::ServiceDaemon`, one browse, one register,
/// one event channel translated into [`DiscoveryEvent`]).
#[derive(Debug)]
pub struct MdnsDiscovery {
    /// Whether loopback is included: the tests and the two-node harness need it,
    /// the app does not (a node on `lo` is not a node on the LAN).
    include_loopback: bool,
}

impl MdnsDiscovery {
    /// A discovery for every up, non-loopback interface — or including loopback
    /// when `include_loopback` is set (the in-process tests and `tools/netns.sh`).
    pub fn new(include_loopback: bool) -> Result<Self, LanError> {
        Ok(MdnsDiscovery { include_loopback })
    }

    /// Whether this instance was told to include loopback.
    pub fn includes_loopback(&self) -> bool {
        self.include_loopback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stub discovers nothing and does not pretend otherwise: it answers the
    /// flag it was built with and never invents an event.
    #[test]
    fn the_stub_remembers_the_loopback_flag_and_finds_nothing() {
        let discovery = MdnsDiscovery::new(true).expect("a discovery");
        assert!(discovery.includes_loopback());
        assert!(
            !MdnsDiscovery::new(false)
                .expect("a discovery")
                .includes_loopback()
        );
    }
}
