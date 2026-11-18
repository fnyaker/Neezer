//! The QUIC endpoint, the TLS configuration and the connection table
//! (docs/agents/lan-nodes.md, *Transport and authentication*).
//!
//! **LN0 declares the surface; LN6 fills the bodies.** The decisions that are
//! already fixed, because every later lot depends on them:
//!
//! - **quinn 0.11**, TLS **1.3 only**, ALPN `neezer-lan/1`, `max_early_data_size =
//!   0` and `enable_early_data = false` (RFC 9001 §9.2: a replayed 0-RTT request
//!   must not fetch anything).
//! - The rustls provider is **aws-lc-rs, always**, through
//!   `builder_with_provider(Arc::new(rustls::crypto::aws_lc_rs::default_provider()))`.
//!   `install_default` is never called and rustls' `ring` feature is never
//!   enabled: with both, `ClientConfig::builder()` panics on an ambiguous
//!   process default (measured on the prototype).
//! - Each node presents an **ephemeral self-signed Ed25519 certificate**, created
//!   at start (`rcgen`). The verifiers accept any certificate and still check the
//!   TLS 1.3 handshake signature with it
//!   (`rustls::crypto::verify_tls13_signature`). The certificate is NOT the
//!   identity — the exporter-bound proof in [`crate::auth`] is.
//! - Before `AuthOk` the responder reads at most one stream and 2 KiB, for at most
//!   2 s; every failure closes with application code `0x10` and an empty reason.
//! - At most 16 connections, one per peer pair (when both dialled, the one from
//!   the lower `device_pub` survives).
//!
//! A request is one stream: [`PeerConn::request`] writes one [`proto::Request`],
//! reads one [`proto::Response`], and hands back a [`Body`] when bytes follow.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use crate::cred::{Credential, Revocations, Roster, TrustKeys};
use crate::keys::DeviceKey;
use crate::proto;
use crate::scope::Scope;
use crate::types::{DevicePub, Kid, LanError, Relation, Subject};

/// What this device is on the LAN: its key and the credential that proves the
/// key belongs to a person.
///
/// One `Arc<Identity>` is swapped whole when a renewal rotates the key, so a live
/// transfer keeps the identity it started with and a new one uses the new
/// identity — no half-updated pair.
#[derive(Debug)]
pub struct Identity {
    pub key: DeviceKey,
    pub cred: Credential,
}

/// The trust this device holds: its own subject, the server's keys, the newest
/// revocation list per key, and the roster (absent until the first sync).
///
/// Behind one `RwLock` so a gossip frame that arrives on a connection updates what
/// every other connection and the screens read, without a second source of truth.
#[derive(Debug, Clone)]
pub struct TrustState {
    pub subject: Subject,
    pub keys: TrustKeys,
    /// The highest serial seen per server key (`kid`).
    pub revocations: HashMap<Kid, Revocations>,
    pub roster: Option<Roster>,
}

/// The shared trust, as every task of the crate holds it.
pub type SharedTrust = Arc<std::sync::RwLock<TrustState>>;

/// What one authenticated peer is, from this device's side.
#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub device_pub: DevicePub,
    pub subject: Subject,
    pub relation: Relation,
    /// What this peer may see of MY files (built from MY roster), or `None`
    /// before it may see anything at all.
    pub scope: Scope,
    /// Its `Hello`, once received: the name, the platform and the two versions.
    /// `None` until then.
    pub hello: Option<proto::Hello>,
    pub addr: SocketAddr,
}

/// The QUIC endpoint: one UDP socket, the TLS configuration, and the accepting
/// side of the connection table.
///
/// **LN6** puts `quinn::Endpoint` here. Nothing else in the crate touches quinn.
#[derive(Debug, Default)]
pub struct Endpoint {
    /// The port actually bound (the preferred one, or an ephemeral fallback).
    port: u16,
    /// Whether the preferred port was taken and an ephemeral one was used: the
    /// screens say so, because a peer that dials the advertised port then fails.
    port_fallback: bool,
}

impl Endpoint {
    /// Binds on `port` (`0` = any), on the runtime `rt` belongs to.
    ///
    /// The certificate and the TLS configuration are built here, once, from
    /// `id`. A port already taken is not a failure: the node binds an ephemeral
    /// one and reports `port_fallback() == true`.
    pub fn bind(
        rt: &tokio::runtime::Handle,
        port: u16,
        id: Arc<Identity>,
        trust: SharedTrust,
        hello: proto::Hello,
    ) -> Result<Endpoint, LanError> {
        let _ = (rt, port, id, trust, hello);
        Err(LanError::Unsupported)
    }

    /// The port this endpoint is really listening on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Whether the preferred port was taken and an ephemeral one is in use.
    pub fn port_fallback(&self) -> bool {
        self.port_fallback
    }

    /// Dials one address and authenticates the exchange, answering the connection
    /// only once BOTH proofs verified.
    pub async fn connect(&self, addr: SocketAddr) -> Result<PeerConn, LanError> {
        let _ = addr;
        Err(LanError::Unsupported)
    }

    /// The next accepted connection. `None` once the endpoint is closed.
    pub async fn accept(&self) -> Option<Result<PeerConn, LanError>> {
        None
    }

    /// Swaps the identity (a renewal rotated the key). Live connections keep the
    /// identity they authenticated with.
    pub fn set_identity(&self, id: Arc<Identity>) {
        let _ = id;
    }

    /// Closes the endpoint and waits for its tasks: nothing outlives the runtime
    /// it borrowed.
    pub async fn close(&self) {}
}

/// One authenticated connection to a peer, and its control stream.
///
/// Cloning is cheap and every clone is the same connection: the requester and the
/// transfer tasks share it, which is what makes a control request never wait
/// behind a 30 MB file (each request is its own QUIC stream).
#[derive(Debug, Clone)]
pub struct PeerConn {
    info: PeerInfo,
}

impl PeerConn {
    /// What this connection authenticated as.
    pub fn info(&self) -> &PeerInfo {
        &self.info
    }

    /// One request on its own stream, with the answer's head and, when bytes
    /// follow it, the body to read them from.
    pub async fn request(&self, req: proto::Request) -> Result<Reply, LanError> {
        let _ = req;
        Err(LanError::Unsupported)
    }

    /// The next request this peer sends us.
    pub async fn next_incoming(&self) -> Option<Incoming> {
        None
    }

    /// Sends one control frame (gossip, a version notice, `Bye`).
    pub async fn send_control(&self, c: proto::Control) -> Result<(), LanError> {
        let _ = c;
        Err(LanError::Unsupported)
    }

    /// The next control frame from this peer. `None` once the control stream is
    /// closed.
    pub async fn next_control(&self) -> Option<proto::Control> {
        None
    }

    /// The connection's smoothed round-trip time, for the "latency" the peer list
    /// shows and for ranking two peers that have the same copy.
    pub fn rtt(&self) -> std::time::Duration {
        std::time::Duration::ZERO
    }

    /// Closes with an application code (`0x10` for any refused authentication,
    /// `0x11` for a protocol major this build cannot speak) and an EMPTY reason:
    /// a reason string is a leak.
    pub fn close(&self, code: u32) {
        let _ = code;
    }
}

/// One answer's head, and the bytes that follow it when the answer carries a body.
#[derive(Debug)]
pub struct Reply {
    pub head: proto::Response,
    pub body: Option<Body>,
}

/// The raw bytes of a `Fetch` or a `Cover`, read chunk by chunk so a 30 MB file
/// never lands in memory whole.
#[derive(Debug)]
pub struct Body {
    remaining: u64,
}

impl Body {
    /// The next chunk, at most `max` bytes; `None` at the end.
    pub async fn chunk(&mut self, max: usize) -> Result<Option<Vec<u8>>, LanError> {
        let _ = max;
        Ok(None)
    }

    /// How many bytes are still to come.
    pub fn remaining(&self) -> u64 {
        self.remaining
    }
}

/// One request a peer sent, with the stream its answer must go to.
#[derive(Debug)]
pub struct Incoming {
    pub request: proto::Request,
}

impl Incoming {
    /// Answers this request: the head frame, then `body` until its end. Answers
    /// the number of body bytes written.
    pub async fn respond(
        self,
        head: proto::Response,
        body: Option<Box<dyn tokio::io::AsyncRead + Unpin + Send>>,
    ) -> Result<u64, LanError> {
        let _ = (head, body);
        Err(LanError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one decision that must not drift: a stub endpoint reports the port it
    /// was given and no fallback, so a screen cannot invent a firewall line for a
    /// port nobody listens on.
    #[test]
    fn an_unbound_endpoint_reports_no_listening_port() {
        let endpoint = Endpoint::default();
        assert_eq!(endpoint.port(), 0);
        assert!(!endpoint.port_fallback());
    }

    /// The authentication codes are the note's, and `0x10` must stay distinct from
    /// `0x11`: one is "refused", the other is "your app is too old", and the UI
    /// says something different for each.
    #[test]
    fn the_two_close_codes_are_distinct() {
        const REFUSED: u32 = 0x10;
        const WRONG_MAJOR: u32 = 0x11;
        assert_ne!(REFUSED, WRONG_MAJOR);
        assert_eq!(REFUSED, 16);
        assert_eq!(WRONG_MAJOR, 17);
    }
}
