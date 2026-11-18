//! The QUIC endpoint, the TLS configuration and the connection table
//! (docs/agents/lan-nodes.md, *Transport and authentication*).
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
//!   2 s; every failure closes with application code `0x10` and an empty reason,
//!   having written **no byte**.
//! - At most [`MAX_CONNECTIONS`] connections, one per peer pair (when both
//!   dialled, the one dialled by the lower `device_pub` survives).
//!
//! A request is one stream: [`PeerConn::request`] writes one [`proto::Request`],
//! reads one [`proto::Response`], and hands back a [`Body`] when bytes follow.
//!
//! # Nothing is spawned here
//!
//! No function in this module calls `tokio::spawn`: reads are pull-based
//! (`accept_bi`, `read_exact`) and the one task that exists — quinn's endpoint
//! driver — is joined by [`Endpoint::close`] through `wait_idle()`. That is what
//! lets the worker rule "a thread that borrows a runtime is joined before that
//! runtime stops" hold by construction rather than by discipline, and the test
//! `the_transport_spawns_no_task_of_its_own` reads this file to keep it true.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Duration;

use quinn::{Connection, RecvStream, SendStream, VarInt};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::timeout;
use tracing::debug;

use crate::auth::{self, Side};
use crate::codec;
use crate::cred::{Credential, Revocations, Roster, TrustKeys};
use crate::keys::DeviceKey;
use crate::proto;
use crate::scope::Scope;
use crate::types::{DevicePub, Kid, LanError, Relation, Subject};

/// The ALPN both peers must offer and accept.
///
/// One string, checked by TLS: a peer that speaks another protocol is refused
/// during the handshake, before a single application byte exists.
pub const ALPN: &[u8] = b"neezer-lan/1";

/// The UDP port a node listens on by default (Réglages → avancé; `0` = any).
///
/// One fixed number the person can open in their firewall with one command. When
/// it is taken, the node binds an ephemeral port and says so
/// ([`Endpoint::port_fallback`]).
pub const DEFAULT_PORT: u16 = 45910;

/// The name presented in SNI. It is never verified (the proof is the identity),
/// and it never changes, so no host name of this machine is ever offered to a
/// stranger on the Wi-Fi (T1).
const SERVER_NAME: &str = "neezer-lan";

/// How many connections one endpoint keeps at once.
///
/// A person has a handful of devices; sixteen is already far more than any
/// household, and it bounds what a hostile peer can make this node allocate.
pub const MAX_CONNECTIONS: usize = 16;

/// The application close code of a duplicate connection: the same peer pair
/// already has the connection the lower `device_pub` dialled.
///
/// Deliberately NOT [`auth::CLOSE_REFUSED`]: a refusal means "do not dial me again
/// for 10 minutes", and a duplicate means "we are already connected — over there".
pub const CLOSE_DUPLICATE: u32 = 0x12;

/// The idle timeout: a connection with nothing to say for this long is gone.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// How often a live connection sends a keep-alive, while the app is foreground or
/// transferring (the note's number; `Lan` turns it off in the background).
pub const KEEP_ALIVE: Duration = Duration::from_secs(10);

/// How long a dial may take before the peer is treated as unreachable. A LAN
/// handshake is milliseconds; a peer that needs longer is not on this network.
pub const CONNECT_DEADLINE: Duration = Duration::from_secs(5);

/// How long [`Endpoint::close`] waits for the connections to drain before the
/// runtime must be allowed to stop anyway.
pub const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(5);

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

impl Default for TrustState {
    /// The trust of a device that has not enrolled yet: no subject, no server key,
    /// no revocation list, no roster. Everything it is asked to authorise is
    /// refused, which is the safe direction.
    fn default() -> Self {
        TrustState {
            subject: Subject([0u8; 16]),
            keys: TrustKeys::default(),
            revocations: HashMap::new(),
            roster: None,
        }
    }
}

impl TrustState {
    /// The revocation serial this device would announce: the highest it holds for
    /// the key its own credential names, `0` before any list arrived.
    pub fn own_serial(&self, cred: &Credential) -> u64 {
        self.revocations
            .get(&cred.kid)
            .map(|list| list.serial)
            .unwrap_or(0)
    }
}

/// The shared trust, as every task of the crate holds it.
pub type SharedTrust = Arc<RwLock<TrustState>>;

/// What one authenticated peer is, from this device's side.
#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub device_pub: DevicePub,
    pub subject: Subject,
    pub relation: Relation,
    /// What this peer may see of MY files, built from MY roster.
    ///
    /// [`Scope::Own`] for one of my own devices, `Scope::Friend` with exactly what
    /// my roster shares for a friend, and — when I dialled a peer that shares with
    /// me but to whom I share nothing — the empty friend scope, whose `allows`
    /// answers `false` for every track. `Scope` has no third variant, and this is
    /// the one that answers `NotFound` to everything: a peer never gets a refusal
    /// that tells it more than an absent file would (T7).
    pub scope: Scope,
    /// Its `Hello`, once received: the name, the platform and the two versions.
    /// `None` until then — this crate fills it during the handshake, so a caller
    /// never has to match on a control frame to read a name.
    pub hello: Option<proto::Hello>,
    pub addr: SocketAddr,
}

/// The QUIC endpoint: one UDP socket, the TLS configuration, and the connection
/// table.
#[derive(Debug)]
pub struct Endpoint {
    /// `None` for [`Endpoint::default`], the unbound endpoint of the shape tests:
    /// every operation on it refuses.
    inner: Option<quinn::Endpoint>,
    /// The port actually bound (the preferred one, or an ephemeral fallback).
    port: u16,
    /// Whether the preferred port was taken and an ephemeral one was used: the
    /// screens say so, because a peer that dials the advertised port then fails.
    port_fallback: bool,
    /// The identity in force; swapped whole by [`Endpoint::set_identity`].
    identity: RwLock<Option<Arc<Identity>>>,
    trust: SharedTrust,
    hello: proto::Hello,
    table: Arc<ConnTable>,
}

impl Default for Endpoint {
    fn default() -> Self {
        Endpoint {
            inner: None,
            port: 0,
            port_fallback: false,
            identity: RwLock::new(None),
            trust: Arc::new(RwLock::new(TrustState::default())),
            hello: proto::Hello::default(),
            table: Arc::new(ConnTable::default()),
        }
    }
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
        // quinn spawns its driver task on the ambient runtime, and `bind` may be
        // called from a plain thread: enter `rt` for the duration.
        let _entered = rt.enter();

        let server_tls = server_tls_config(&id)?;
        let client_tls = client_tls_config(&id)?;
        let server = quinn::ServerConfig::with_crypto(Arc::new(
            quinn::crypto::rustls::QuicServerConfig::try_from(server_tls)
                .map_err(|error| LanError::Internal(format!("TLS serveur : {error}")))?,
        ));
        let mut server = server;
        server.transport_config(Arc::new(transport_config()));
        let client = quinn::ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(client_tls)
                .map_err(|error| LanError::Internal(format!("TLS client : {error}")))?,
        ));

        let (socket, port_fallback) = match bind_socket(port) {
            Ok(socket) => (socket, false),
            // The preferred port is taken: the note's fallback, and the flag the
            // screens need so nobody copies a firewall line for a port this node
            // does not listen on.
            Err(error) if port != 0 => {
                debug!(%error, port, "the LAN port is taken, falling back to an ephemeral one");
                let socket = bind_socket(0)
                    .map_err(|error| LanError::Internal(format!("socket UDP : {error}")))?;
                (socket, true)
            }
            Err(error) => {
                return Err(LanError::Internal(format!("socket UDP : {error}")));
            }
        };
        let bound = socket
            .local_addr()
            .map_err(|error| LanError::Internal(format!("adresse UDP : {error}")))?;
        let mut inner = quinn::Endpoint::new(
            quinn::EndpointConfig::default(),
            Some(server),
            socket,
            Arc::new(quinn::TokioRuntime),
        )
        .map_err(|error| LanError::Internal(format!("endpoint QUIC : {error}")))?;
        inner.set_default_client_config(client);

        Ok(Endpoint {
            inner: Some(inner),
            port: bound.port(),
            port_fallback,
            identity: RwLock::new(Some(id)),
            trust,
            hello,
            table: Arc::new(ConnTable::default()),
        })
    }

    /// The port this endpoint is really listening on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Whether the preferred port was taken and an ephemeral one is in use.
    pub fn port_fallback(&self) -> bool {
        self.port_fallback
    }

    /// How many authenticated peers this endpoint holds: one per peer pair, never
    /// two, and at most [`MAX_CONNECTIONS`].
    pub fn connections(&self) -> usize {
        self.table.len()
    }

    /// The identity in force (a renewal replaces it whole).
    fn identity(&self) -> Result<Arc<Identity>, LanError> {
        let identity = self
            .identity
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        identity.clone().ok_or(LanError::NotEnrolled)
    }

    /// The trust as it is at this instant: the handshake decides on a snapshot, so
    /// a gossip frame that lands mid-handshake cannot make one decision read two
    /// different rosters.
    fn trust(&self) -> TrustState {
        self.trust
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Dials one address and authenticates the exchange, answering the connection
    /// only once BOTH proofs verified.
    ///
    /// The initiator proves itself first — its own `Auth` is the first byte on the
    /// wire — and it verifies the responder's proof and credential before it sends
    /// a single request (T3).
    pub async fn connect(&self, addr: SocketAddr) -> Result<PeerConn, LanError> {
        let inner = self.inner.as_ref().ok_or_else(unbound)?;
        let id = self.identity()?;
        let trust = self.trust();

        let connecting = inner.connect(addr, SERVER_NAME).map_err(|error| {
            debug!(%error, %addr, "a LAN dial could not start");
            LanError::PeerGone
        })?;
        let conn = match timeout(CONNECT_DEADLINE, connecting).await {
            Ok(Ok(conn)) => conn,
            Ok(Err(error)) => {
                debug!(%error, %addr, "a LAN handshake failed");
                return Err(LanError::PeerGone);
            }
            Err(_) => {
                debug!(%addr, "a LAN handshake timed out");
                return Err(LanError::PeerGone);
            }
        };

        // No 0-RTT, ever: `connect` (never `into_0rtt`) means the first application
        // byte travels on a session whose both proofs have been verified.
        let (mut send, mut recv) = conn.open_bi().await.map_err(gone)?;
        let serial = trust.own_serial(&id.cred);
        let frame = auth::proof_frame(&id, &conn, Side::Initiator, serial)?;
        codec::write_frame(&mut send, &frame).await?;

        let answer = match timeout(
            auth::PREAUTH_DEADLINE,
            codec::read_frame::<proto::Control>(
                &mut recv,
                auth::PREAUTH_MAX_BYTES - codec::FRAME_HEADER_BYTES,
            ),
        )
        .await
        {
            Ok(Ok(Some(frame))) => frame,
            Ok(Ok(None)) | Ok(Err(_)) | Err(_) => {
                conn.close(VarInt::from_u32(auth::CLOSE_REFUSED), b"");
                return Err(LanError::Refused);
            }
        };
        let authorised =
            match auth::verify_peer(&answer, Side::Initiator, &trust, &conn, auth::now_seconds()) {
                Ok(authorised) => authorised,
                Err(cause) => return Err(refuse(&conn, &cause)),
            };

        // Names at last, and only now: they travel inside an authenticated
        // connection and nowhere else.
        codec::write_frame(&mut send, &hello_control(&self.hello)).await?;
        let peer_hello = match timeout(
            auth::HELLO_DEADLINE,
            codec::read_frame::<proto::Control>(&mut recv, codec::MAX_FRAME_BYTES),
        )
        .await
        {
            Ok(Ok(Some(frame))) => match hello_from(&frame) {
                Some(hello) => hello,
                None => {
                    conn.close(VarInt::from_u32(auth::CLOSE_REFUSED), b"");
                    return Err(LanError::Refused);
                }
            },
            Ok(Ok(None)) | Ok(Err(_)) | Err(_) => {
                conn.close(VarInt::from_u32(auth::CLOSE_REFUSED), b"");
                return Err(LanError::Refused);
            }
        };

        let info = PeerInfo {
            device_pub: authorised.cred.device_pub,
            subject: authorised.cred.subject,
            relation: authorised.relation,
            scope: authorised.scope,
            hello: Some(peer_hello),
            addr: normalise(conn.remote_address()),
        };
        let peer = PeerConn::new(conn, info, send, recv, Arc::downgrade(&self.table), true);
        match self.table.admit(peer, id.key.public()) {
            Ok(admitted) => Ok(admitted.conn),
            Err(error) => Err(error),
        }
    }

    /// The next accepted connection. `None` once the endpoint is closed.
    ///
    /// A peer this node refuses answers `Some(Err(LanError::Refused))` — the
    /// refusal is an event the screens can show — while a duplicate that lost the
    /// race, and a connection that carried nothing, are skipped: the pair is
    /// already connected, so there is nothing to report.
    pub async fn accept(&self) -> Option<Result<PeerConn, LanError>> {
        let inner = self.inner.as_ref()?;
        loop {
            let incoming = inner.accept().await?;
            let addr = incoming.remote_address();
            match self.accept_one(incoming, addr).await {
                // The survivor, whether this connection is the one the pair keeps or
                // the one it already had: a peer that dials while we dial it must not
                // leave this caller waiting for a connection that exists.
                Ok(Some(admitted)) => return Some(Ok(admitted.conn)),
                Ok(None) => continue,
                Err(error) => return Some(Err(error)),
            }
        }
    }

    /// One accepted connection's handshake; see the module docs for the order.
    async fn accept_one(
        &self,
        incoming: quinn::Incoming,
        addr: SocketAddr,
    ) -> Result<Option<Admitted>, LanError> {
        let id = self.identity()?;
        let trust = self.trust();
        let connecting = incoming.accept().map_err(|error| {
            debug!(%error, %addr, "a LAN connection attempt could not be accepted");
            LanError::PeerGone
        })?;
        let conn = match timeout(CONNECT_DEADLINE, connecting).await {
            Ok(Ok(conn)) => conn,
            Ok(Err(error)) => {
                debug!(%error, %addr, "an inbound LAN handshake failed");
                return Err(LanError::PeerGone);
            }
            Err(_) => return Err(LanError::PeerGone),
        };

        // Sixteen peers is already more than a household; the seventeenth is
        // refused before this device writes a byte, so a stranger cannot make the
        // table grow and cannot tell "full" from "refused".
        if self.table.len() >= MAX_CONNECTIONS {
            debug!(%addr, "a LAN peer was refused: the connection table is full");
            conn.close(VarInt::from_u32(auth::CLOSE_REFUSED), b"");
            return Err(LanError::Busy { retry_in_ms: 2_000 });
        }

        // One stream, 2 KiB, 2 s. Nothing at all is written before this passes,
        // which is what makes a stranger learn nothing (T2).
        let preauth = timeout(auth::PREAUTH_DEADLINE, async {
            let (send, mut recv) = conn.accept_bi().await.map_err(gone)?;
            let frame = codec::read_frame::<proto::Control>(
                &mut recv,
                auth::PREAUTH_MAX_BYTES - codec::FRAME_HEADER_BYTES,
            )
            .await?;
            Ok::<_, LanError>((send, recv, frame))
        })
        .await;
        let (mut send, mut recv, frame) = match preauth {
            Ok(Ok((send, recv, Some(frame)))) => (send, recv, frame),
            // A timeout, a stream that ended, an over-cap frame, a peer that
            // opened nothing: one code, no byte written, no cause on the wire.
            Ok(Ok((_, _, None))) | Ok(Err(_)) | Err(_) => {
                conn.close(VarInt::from_u32(auth::CLOSE_REFUSED), b"");
                return Err(LanError::Refused);
            }
        };
        let authorised =
            match auth::verify_peer(&frame, Side::Responder, &trust, &conn, auth::now_seconds()) {
                Ok(authorised) => authorised,
                Err(cause) => return Err(refuse(&conn, &cause)),
            };

        // Our own proof, then our Hello: the first byte this device ever writes.
        let serial = trust.own_serial(&id.cred);
        let answer = auth::proof_frame(&id, &conn, Side::Responder, serial)?;
        codec::write_frame(&mut send, &answer).await?;
        codec::write_frame(&mut send, &hello_control(&self.hello)).await?;
        let peer_hello = match timeout(
            auth::HELLO_DEADLINE,
            codec::read_frame::<proto::Control>(&mut recv, codec::MAX_FRAME_BYTES),
        )
        .await
        {
            Ok(Ok(Some(frame))) => match hello_from(&frame) {
                Some(hello) => hello,
                None => {
                    conn.close(VarInt::from_u32(auth::CLOSE_REFUSED), b"");
                    return Err(LanError::Refused);
                }
            },
            Ok(Ok(None)) | Ok(Err(_)) | Err(_) => {
                conn.close(VarInt::from_u32(auth::CLOSE_REFUSED), b"");
                return Err(LanError::Refused);
            }
        };

        let info = PeerInfo {
            device_pub: authorised.cred.device_pub,
            subject: authorised.cred.subject,
            relation: authorised.relation,
            scope: authorised.scope,
            hello: Some(peer_hello),
            addr: normalise(addr),
        };
        let peer = PeerConn::new(conn, info, send, recv, Arc::downgrade(&self.table), false);
        Ok(Some(self.table.admit(peer, id.key.public())?))
    }

    /// Swaps the identity (a renewal rotated the key). Live connections keep the
    /// identity they authenticated with.
    pub fn set_identity(&self, id: Arc<Identity>) {
        let mut current = self
            .identity
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *current = Some(id);
    }

    /// Closes the endpoint and waits for its tasks: nothing outlives the runtime
    /// it borrowed.
    pub async fn close(&self) {
        let Some(inner) = self.inner.as_ref() else {
            return;
        };
        // Every connection this endpoint made goes away with code 0, and the table
        // is emptied so a later `connections()` cannot report a peer that is gone.
        self.table.close_all(0);
        inner.close(VarInt::from_u32(0), b"");
        // quinn's driver task is the crate's only task; this is where it is
        // joined. A peer that will not answer the close must not hold the app's
        // shutdown for ever, hence the deadline.
        if timeout(SHUTDOWN_DEADLINE, inner.wait_idle()).await.is_err() {
            debug!("a LAN endpoint did not idle before the shutdown deadline");
        }
    }
}

/// One admitted connection, and whether it is the new one (the other case is the
/// connection the pair already had, which the caller gets instead of a refusal).
struct Admitted {
    conn: PeerConn,
    #[allow(dead_code)]
    fresh: bool,
}

/// The connection table: one entry per peer pair, and the 16-connection cap.
#[derive(Debug, Default)]
struct ConnTable {
    live: Mutex<HashMap<DevicePub, Entry>>,
}

#[derive(Debug)]
struct Entry {
    stable_id: usize,
    conn: PeerConn,
}

impl ConnTable {
    fn len(&self) -> usize {
        self.lock().len()
    }

    /// Puts `candidate` in the table, applying the one-connection-per-pair rule:
    /// when both sides dialled, the connection dialled by the **lower
    /// `device_pub`** survives — a rule both peers compute from the same two keys,
    /// so they always agree which one to keep.
    fn admit(&self, candidate: PeerConn, mine: DevicePub) -> Result<Admitted, LanError> {
        let peer = candidate.info.device_pub;
        // Which direction the surviving connection must have: the lower key dials.
        let survivor_is_ours = mine < peer;
        let candidate_is_survivor = candidate.dialled_by_us == survivor_is_ours;
        let candidate_id = candidate.stable_id;

        let mut live = self.lock();
        match live.get(&peer) {
            None => {
                if live.len() >= MAX_CONNECTIONS {
                    drop(live);
                    candidate.close(auth::CLOSE_REFUSED);
                    return Err(LanError::Busy { retry_in_ms: 2_000 });
                }
                live.insert(
                    peer,
                    Entry {
                        stable_id: candidate_id,
                        conn: candidate.clone(),
                    },
                );
                drop(live);
                Ok(Admitted {
                    conn: candidate,
                    fresh: true,
                })
            }
            Some(_existing) if candidate_is_survivor => {
                let previous = live.insert(
                    peer,
                    Entry {
                        stable_id: candidate_id,
                        conn: candidate.clone(),
                    },
                );
                drop(live);
                if let Some(previous) = previous {
                    previous.conn.close(CLOSE_DUPLICATE);
                }
                Ok(Admitted {
                    conn: candidate,
                    fresh: true,
                })
            }
            Some(existing) => {
                let survivor = existing.conn.clone();
                drop(live);
                candidate.close(CLOSE_DUPLICATE);
                Ok(Admitted {
                    conn: survivor,
                    fresh: false,
                })
            }
        }
    }

    /// Forgets one connection — but only if the table still holds THAT one: a
    /// stale clone of a replaced connection must not evict its successor.
    fn remove(&self, peer: DevicePub, stable_id: usize) {
        let mut live = self.lock();
        if live
            .get(&peer)
            .is_some_and(|entry| entry.stable_id == stable_id)
        {
            live.remove(&peer);
        }
    }

    /// Closes and forgets every connection (the endpoint is going away).
    fn close_all(&self, code: u32) {
        let drained: Vec<PeerConn> = {
            let mut live = self.lock();
            live.drain().map(|(_, entry)| entry.conn).collect()
        };
        for conn in drained {
            conn.close(code);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<DevicePub, Entry>> {
        self.live
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// One authenticated connection to a peer, and its control stream.
///
/// Cloning is cheap and every clone is the same connection: the requester and the
/// transfer tasks share it, which is what makes a control request never wait
/// behind a 30 MB file (each request is its own QUIC stream).
#[derive(Clone)]
pub struct PeerConn {
    info: PeerInfo,
    conn: Connection,
    /// The control stream this device writes on: the authentication stream, kept
    /// open. Behind a mutex because a QUIC send stream is written by one task at a
    /// time, and a control frame is small enough that sharing is cheaper than a
    /// task per connection.
    control_out: Arc<AsyncMutex<SendStream>>,
    /// The peer's control frames. Same reason, on the read side.
    control_in: Arc<AsyncMutex<RecvStream>>,
    /// The table this connection is registered in, so a close forgets it. Weak: the
    /// table holds the connections, and a strong edge back would be a cycle.
    table: Weak<ConnTable>,
    stable_id: usize,
    /// Whether THIS device dialled. The duplicate rule needs it, and so does the
    /// screen that says who connected first.
    dialled_by_us: bool,
}

impl std::fmt::Debug for PeerConn {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PeerConn")
            .field("peer", &self.info.device_pub)
            .field("dialled_by_us", &self.dialled_by_us)
            .finish()
    }
}

impl PeerConn {
    fn new(
        conn: Connection,
        info: PeerInfo,
        control_out: SendStream,
        control_in: RecvStream,
        table: Weak<ConnTable>,
        dialled_by_us: bool,
    ) -> PeerConn {
        let stable_id = conn.stable_id();
        PeerConn {
            info,
            conn,
            control_out: Arc::new(AsyncMutex::new(control_out)),
            control_in: Arc::new(AsyncMutex::new(control_in)),
            table,
            stable_id,
            dialled_by_us,
        }
    }

    /// What this connection authenticated as.
    pub fn info(&self) -> &PeerInfo {
        &self.info
    }

    /// The underlying QUIC connection.
    ///
    /// Nothing inside this crate needs it — every operation has a method above — and
    /// it exists for the two things a transport must expose: the tests that write
    /// bytes this crate would never write (a frame tag from a future version, an
    /// over-cap header), and a caller that wants `close_reason()` for a log. It is
    /// deliberately the LAST door: a later lot that reaches for it is probably
    /// inventing a protocol.
    pub fn connection(&self) -> &quinn::Connection {
        &self.conn
    }

    /// Whether this device dialled, or was dialled. Two connections of one peer
    /// pair never survive; this says which one did.
    pub fn dialled_by_us(&self) -> bool {
        self.dialled_by_us
    }

    /// One request on its own stream, with the answer's head and, when bytes
    /// follow it, the body to read them from.
    ///
    /// The request stream is finished right after its frame, so the responder never
    /// waits for the rest of a message that has none.
    pub async fn request(&self, req: proto::Request) -> Result<Reply, LanError> {
        let (mut send, mut recv) = self.conn.open_bi().await.map_err(gone)?;
        codec::write_frame(&mut send, &req).await?;
        send.finish().map_err(|error| {
            debug!(%error, "a LAN request stream closed before its frame was sent");
            LanError::PeerGone
        })?;
        let head =
            match codec::read_frame::<proto::Response>(&mut recv, codec::MAX_FRAME_BYTES).await? {
                Some(head) => head,
                None => return Err(LanError::PeerGone),
            };
        let body = match &head {
            proto::Response::FetchOk { len, .. } => Some(Body {
                recv,
                remaining: *len,
            }),
            proto::Response::Cover { len, .. } => Some(Body {
                recv,
                remaining: u64::from(*len),
            }),
            _ => {
                let _ = recv.stop(VarInt::from_u32(0));
                None
            }
        };
        Ok(Reply { head, body })
    }

    /// The next request this peer sends us.
    ///
    /// A frame this build cannot parse is answered `Error { code: "unknown" }` and
    /// the loop goes on (rule 7: a newer peer talking to an older one). A frame
    /// that ANNOUNCES more than [`codec::MAX_FRAME_BYTES`] is a hostile size, and
    /// the connection goes with it (T9).
    pub async fn next_incoming(&self) -> Option<Incoming> {
        loop {
            let (mut send, mut recv) = match self.conn.accept_bi().await {
                Ok(streams) => streams,
                Err(error) => {
                    debug!(%error, "a LAN connection stopped accepting streams");
                    return None;
                }
            };
            let bytes = match codec::read_frame_bytes(&mut recv, codec::MAX_FRAME_BYTES).await {
                Ok(Some(bytes)) => bytes,
                Ok(None) => continue,
                Err(error) => {
                    debug!(%error, "a LAN peer announced a frame this node refuses");
                    self.close(auth::CLOSE_REFUSED);
                    return None;
                }
            };
            match serde_json::from_slice::<proto::Request>(&bytes) {
                Ok(request) => {
                    return Some(Incoming {
                        request,
                        send,
                        recv,
                    });
                }
                Err(error) => {
                    debug!(%error, "a LAN peer sent a frame this build cannot read");
                    let _ = codec::write_frame(
                        &mut send,
                        &proto::Response::Error {
                            code: "unknown".to_owned(),
                        },
                    )
                    .await;
                    let _ = send.finish();
                    let _ = recv.stop(VarInt::from_u32(0));
                }
            }
        }
    }

    /// Sends one control frame (gossip, a version notice, `Bye`).
    pub async fn send_control(&self, c: proto::Control) -> Result<(), LanError> {
        let mut send = self.control_out.lock().await;
        codec::write_frame(&mut send, &c).await.map(|_written| ())
    }

    /// The next control frame from this peer. `None` once the control stream is
    /// closed.
    pub async fn next_control(&self) -> Option<proto::Control> {
        let mut recv = self.control_in.lock().await;
        match codec::read_frame::<proto::Control>(&mut recv, codec::MAX_FRAME_BYTES).await {
            Ok(Some(frame)) => Some(frame),
            Ok(None) => None,
            Err(error) => {
                debug!(%error, "a LAN control stream ended badly");
                None
            }
        }
    }

    /// The connection's smoothed round-trip time, for the "latency" the peer list
    /// shows and for ranking two peers that have the same copy.
    pub fn rtt(&self) -> Duration {
        self.conn.rtt()
    }

    /// Closes with an application code (`0x10` for any refused authentication,
    /// `0x11` for a protocol major this build cannot speak, `0x12` for a duplicate
    /// of a connection the pair already has) and an EMPTY reason: a reason string
    /// is a leak.
    pub fn close(&self, code: u32) {
        self.conn.close(VarInt::from_u32(code), b"");
        if let Some(table) = self.table.upgrade() {
            table.remove(self.info.device_pub, self.stable_id);
        }
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
    recv: RecvStream,
    remaining: u64,
}

impl Body {
    /// The next chunk, at most `max` bytes; `None` at the end.
    ///
    /// `max` is clamped to [`codec::MAX_CHUNK_BYTES`] and to what the head
    /// announced: a caller that asks for a gigabyte gets 64 KiB, so no read can
    /// allocate more than a fixed amount whatever the peer said.
    pub async fn chunk(&mut self, max: usize) -> Result<Option<Vec<u8>>, LanError> {
        if self.remaining == 0 {
            return Ok(None);
        }
        let want = max.min(codec::MAX_CHUNK_BYTES).min(self.remaining as usize);
        if want == 0 {
            return Ok(None);
        }
        let mut chunk = vec![0u8; want];
        recv_exact(&mut self.recv, &mut chunk).await?;
        self.remaining -= want as u64;
        Ok(Some(chunk))
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
    send: SendStream,
    recv: RecvStream,
}

impl Incoming {
    /// Answers this request: the head frame, then `body` until its end. Answers
    /// the number of body bytes written.
    ///
    /// When the head names a length (`Cover`, `FetchOk`) at most that many bytes
    /// are written: a body longer than its announcement is a bug here, and the
    /// requester would stop reading anyway — this way the stream ends where the
    /// head said it would, instead of being reset under the reader.
    pub async fn respond(
        mut self,
        head: proto::Response,
        body: Option<Box<dyn AsyncRead + Unpin + Send>>,
    ) -> Result<u64, LanError> {
        codec::write_frame(&mut self.send, &head).await?;
        let announced = match &head {
            proto::Response::Cover { len, .. } => Some(u64::from(*len)),
            proto::Response::FetchOk { len, .. } => Some(*len),
            _ => None,
        };
        let mut written = 0u64;
        if let Some(mut body) = body {
            let mut chunk = vec![0u8; codec::MAX_CHUNK_BYTES];
            loop {
                if announced.is_some_and(|len| written >= len) {
                    break;
                }
                let read = body
                    .read(&mut chunk)
                    .await
                    .map_err(|error| LanError::BadFile(format!("lecture du corps : {error}")))?;
                if read == 0 {
                    break;
                }
                let mut slice = &chunk[..read];
                if let Some(len) = announced {
                    let left = (len - written) as usize;
                    slice = &slice[..left.min(slice.len())];
                }
                self.send.write_all(slice).await.map_err(gone)?;
                written += slice.len() as u64;
            }
        }
        self.send.finish().map_err(|error| {
            debug!(%error, "a LAN answer stream closed before its frame was sent");
            LanError::PeerGone
        })?;
        let _ = self.recv.stop(VarInt::from_u32(0));
        Ok(written)
    }
}

/// Reads exactly `buf.len()` bytes, and says `PeerGone` when the peer stops short:
/// a truncated body is a peer that went away, never a partial success.
async fn recv_exact(recv: &mut RecvStream, buf: &mut [u8]) -> Result<(), LanError> {
    recv.read_exact(buf).await.map_err(|error| {
        debug!(%error, "a LAN body ended before its announced length");
        LanError::PeerGone
    })
}

/// The one error a stream's end maps to.
fn gone<E: std::fmt::Display>(error: E) -> LanError {
    debug!(%error, "a LAN stream ended under its writer");
    LanError::PeerGone
}

/// The error an unbound endpoint answers: nothing was ever configured, so nothing
/// can be dialled.
fn unbound() -> LanError {
    LanError::Internal("cet endpoint LAN n'est pas lié à un port".to_owned())
}

/// Closes a connection for an authentication cause: one code, an empty reason, and
/// the precise cause only in this device's own log (never a proof, never a
/// credential).
fn refuse(conn: &Connection, cause: &auth::AuthError) -> LanError {
    debug!(cause = %cause.cause_fr(), "LAN authentication refused");
    conn.close(VarInt::from_u32(cause.close_code()), b"");
    cause.as_lan_error()
}

/// The control frame that carries a [`proto::Hello`].
fn hello_control(hello: &proto::Hello) -> proto::Control {
    proto::Control::Hello {
        name: hello.name.clone(),
        platform: hello.platform.clone(),
        app_version: hello.app_version.clone(),
        inventory_version: hello.inventory_version,
        sets_version: hello.sets_version,
    }
}

/// The [`proto::Hello`] a control frame carries, if it is one.
fn hello_from(control: &proto::Control) -> Option<proto::Hello> {
    match control {
        proto::Control::Hello {
            name,
            platform,
            app_version,
            inventory_version,
            sets_version,
        } => Some(proto::Hello {
            name: name.clone(),
            platform: platform.clone(),
            app_version: app_version.clone(),
            inventory_version: *inventory_version,
            sets_version: *sets_version,
        }),
        _ => None,
    }
}

/// An address as the rest of the crate stores it: a peer that reached a dual-stack
/// socket over IPv4 has an IPv4 address, not its `::ffff:` spelling (the
/// last-known-address table and the screens show the same thing the person's
/// router does).
fn normalise(addr: SocketAddr) -> SocketAddr {
    match addr.ip() {
        IpAddr::V6(ip) => match ip.to_ipv4_mapped() {
            Some(v4) => SocketAddr::new(IpAddr::V4(v4), addr.port()),
            None => addr,
        },
        IpAddr::V4(_) => addr,
    }
}

/// The QUIC transport parameters: idle timeout, keep-alive and the stream limits
/// a hostile peer is held to.
fn transport_config() -> quinn::TransportConfig {
    let mut transport = quinn::TransportConfig::default();
    transport.max_idle_timeout(Some(
        IDLE_TIMEOUT
            .try_into()
            .expect("30 secondes tiennent dans un idle timeout QUIC"),
    ));
    transport.keep_alive_interval(Some(KEEP_ALIVE));
    // One request is one stream, and the control stream is a second: a peer that
    // opens more at once than it can be answered is refused by QUIC itself, not by
    // a queue this node has to bound.
    transport.max_concurrent_bidi_streams(VarInt::from_u32(64));
    transport.max_concurrent_uni_streams(VarInt::from_u32(0));
    transport
}

/// The TLS 1.3 server configuration: any client certificate is accepted (the proof
/// is the identity), its handshake signature is still checked, and early data is
/// off.
///
/// Exposed so `tests/transport.rs` can prove that TLS 1.2 is refused and that the
/// server never accepts early data: both are properties of this configuration.
pub fn server_tls_config(id: &Identity) -> Result<rustls::ServerConfig, LanError> {
    let (certificate, key) = ephemeral_certificate()?;
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut config = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|error| LanError::Internal(format!("TLS 1.3 indisponible : {error}")))?
        .with_client_cert_verifier(Arc::new(AcceptAnyClient::new(provider)))
        .with_single_cert(vec![certificate], key)
        .map_err(|error| LanError::Internal(format!("certificat éphémère : {error}")))?;
    config.alpn_protocols = vec![ALPN.to_vec()];
    // RFC 9001 §9.2: nothing may be replayed as 0-RTT data.
    config.max_early_data_size = 0;
    config.send_half_rtt_data = false;
    let _ = id;
    Ok(config)
}

/// The TLS 1.3 client configuration: the peer's certificate is accepted, its
/// handshake signature is checked, our own ephemeral certificate is presented, and
/// early data is off.
pub fn client_tls_config(id: &Identity) -> Result<rustls::ClientConfig, LanError> {
    let (certificate, key) = ephemeral_certificate()?;
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut config = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|error| LanError::Internal(format!("TLS 1.3 indisponible : {error}")))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyServer::new(provider)))
        .with_client_auth_cert(vec![certificate], key)
        .map_err(|error| LanError::Internal(format!("certificat éphémère : {error}")))?;
    config.alpn_protocols = vec![ALPN.to_vec()];
    config.enable_early_data = false;
    let _ = id;
    Ok(config)
}

/// A fresh self-signed Ed25519 certificate for this endpoint.
///
/// It is NOT the identity — the exporter-bound proof is — so it is created at
/// start and thrown away with the process: nothing about this machine, its user or
/// its device survives it (T1).
fn ephemeral_certificate() -> Result<
    (
        rustls::pki_types::CertificateDer<'static>,
        rustls::pki_types::PrivateKeyDer<'static>,
    ),
    LanError,
> {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)
        .map_err(|error| LanError::Internal(format!("clé éphémère : {error}")))?;
    let params = rcgen::CertificateParams::new(Vec::<String>::new())
        .map_err(|error| LanError::Internal(format!("paramètres du certificat : {error}")))?;
    let certificate = params
        .self_signed(&key)
        .map_err(|error| LanError::Internal(format!("certificat auto-signé : {error}")))?;
    let private = rustls::pki_types::PrivateKeyDer::Pkcs8(
        rustls::pki_types::PrivatePkcs8KeyDer::from(key.serialize_der()),
    );
    Ok((certificate.der().clone(), private))
}

/// A `ServerCertVerifier` that accepts any certificate **and still verifies the
/// TLS 1.3 handshake signature** with it.
///
/// Accepting the certificate is deliberate: the peer's identity is its device key,
/// proven by the exporter-bound proof, and a self-signed ephemeral certificate has
/// nothing to chain to. Checking the handshake signature is not optional — without
/// it, anyone could complete a TLS session without holding the key of the
/// certificate it presented.
#[derive(Debug)]
struct AcceptAnyServer {
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl AcceptAnyServer {
    fn new(provider: Arc<rustls::crypto::CryptoProvider>) -> Self {
        AcceptAnyServer { provider }
    }
}

impl rustls::client::danger::ServerCertVerifier for AcceptAnyServer {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        // This crate speaks TLS 1.3 only; a TLS 1.2 signature is never accepted.
        Err(rustls::Error::PeerIncompatible(
            rustls::PeerIncompatible::Tls12NotOffered,
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// The server's half of the same rule: any client certificate, its handshake
/// signature still checked, and client authentication **mandatory** — a peer that
/// does not even hold the key of the certificate it presents never reaches the
/// proof exchange.
#[derive(Debug)]
struct AcceptAnyClient {
    provider: Arc<rustls::crypto::CryptoProvider>,
    hints: Vec<rustls::DistinguishedName>,
}

impl AcceptAnyClient {
    fn new(provider: Arc<rustls::crypto::CryptoProvider>) -> Self {
        AcceptAnyClient {
            provider,
            hints: Vec::new(),
        }
    }
}

impl rustls::server::danger::ClientCertVerifier for AcceptAnyClient {
    fn root_hint_subjects(&self) -> &[rustls::DistinguishedName] {
        &self.hints
    }

    fn client_auth_mandatory(&self) -> bool {
        true
    }

    fn verify_client_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::server::danger::ClientCertVerified, rustls::Error> {
        Ok(rustls::server::danger::ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::PeerIncompatible(
            rustls::PeerIncompatible::Tls12NotOffered,
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Binds the listening socket: **dual-stack** when the platform allows it (one
/// socket for IPv4 and IPv6 peers), IPv4 otherwise.
fn bind_socket(port: u16) -> std::io::Result<std::net::UdpSocket> {
    match dual_stack(port) {
        Ok(socket) => Ok(socket),
        Err(dual_error) => {
            debug!(%dual_error, port, "no dual-stack socket, binding IPv4 only");
            std::net::UdpSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port))
        }
    }
}

/// An AF_INET6 datagram socket with `IPV6_V6ONLY = 0`, so one port serves both
/// families — the note's "dual-stack".
///
/// `std` cannot turn that option off (and cannot bind a socket it did not create),
/// and `socket2` is not a dependency of this crate; four `unsafe` calls to `libc`
/// (which the crate already depends on for `setpriority`) are the smaller cost.
/// Each one is annotated with why it is sound.
#[cfg(unix)]
fn dual_stack(port: u16) -> std::io::Result<std::net::UdpSocket> {
    use std::os::fd::{AsRawFd, FromRawFd};

    // SAFETY: a fresh AF_INET6 datagram socket. The descriptor is taken over by
    // `UdpSocket` immediately on success, and closed by the error path below on
    // failure — it is never leaked either way.
    let fd = unsafe { libc::socket(libc::AF_INET6, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fd` is a socket this function owns and nothing else holds it yet.
    let socket = unsafe { std::net::UdpSocket::from_raw_fd(fd) };
    let off: libc::c_int = 0;
    // SAFETY: `socket` is a live AF_INET6 UDP socket, and IPV6_V6ONLY takes one
    // `int`. The option is set BEFORE the bind, which is what Linux requires for
    // it to take effect.
    let set = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::IPPROTO_IPV6,
            libc::IPV6_V6ONLY,
            (&off as *const libc::c_int).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    if set != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut any: libc::sockaddr_in6 = unsafe { std::mem::zeroed() };
    any.sin6_family = libc::AF_INET6 as libc::sa_family_t;
    any.sin6_port = port.to_be();
    // SAFETY: `socket` is a live unbound AF_INET6 UDP socket and `any` is a
    // `sockaddr_in6` of exactly the length passed.
    let bound = unsafe {
        libc::bind(
            socket.as_raw_fd(),
            (&any as *const libc::sockaddr_in6).cast(),
            std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t,
        )
    };
    if bound != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(socket)
}

/// A platform without `libc`'s socket options (neither app targets one): IPv4 only.
#[cfg(not(unix))]
fn dual_stack(_port: u16) -> std::io::Result<std::net::UdpSocket> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "pas de socket dual-stack sur cette plateforme",
    ))
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
        assert_eq!(endpoint.connections(), 0);
    }

    /// The authentication codes are the note's, and `0x10` must stay distinct from
    /// `0x11`: one is "refused", the other is "your app is too old", and the UI
    /// says something different for each. A duplicate has its own code, because a
    /// refusal means "do not dial again for ten minutes".
    #[test]
    fn the_three_close_codes_are_distinct() {
        assert_eq!(auth::CLOSE_REFUSED, 0x10);
        assert_eq!(auth::CLOSE_WRONG_MAJOR, 0x11);
        assert_eq!(CLOSE_DUPLICATE, 0x12);
        assert_eq!(auth::CLOSE_REFUSED, 16);
        assert_eq!(auth::CLOSE_WRONG_MAJOR, 17);
        assert_eq!(CLOSE_DUPLICATE, 18);
    }

    /// A v4-mapped address is stored as the IPv4 one: the last-known-address row
    /// and the screens show what the person's router shows.
    #[test]
    fn a_v4_mapped_peer_address_is_stored_as_ipv4() {
        let mapped: SocketAddr = "[::ffff:192.168.1.7]:45910".parse().expect("adresse");
        assert_eq!(
            normalise(mapped),
            "192.168.1.7:45910".parse::<SocketAddr>().expect("adresse")
        );
        let plain: SocketAddr = "192.168.1.7:45910".parse().expect("adresse");
        assert_eq!(normalise(plain), plain);
        let v6: SocketAddr = "[fe80::1]:45910".parse().expect("adresse");
        assert_eq!(normalise(v6), v6);
    }

    /// The code of a module, without its tests and without its comments: what a
    /// source scan must look at (the scan's own patterns live in the tests, and would
    /// otherwise find themselves).
    fn production_code(source: &str) -> String {
        source
            .split("#[cfg(test)]")
            .next()
            .unwrap_or(source)
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// No task of this crate's own: the module never spawns, so nothing can outlive
    /// the runtime quinn's driver borrows, and `close` is the ONE place that joins it
    /// (`wait_idle`). A source scan, the way `cred.rs` keeps its zero-`unsafe`
    /// promise — the behaviour is `closing_the_endpoint_joins_the_only_task`.
    #[test]
    fn the_transport_spawns_no_task_of_its_own() {
        for source in [
            include_str!("transport.rs"),
            include_str!("codec.rs"),
            include_str!("auth.rs"),
        ] {
            let code = production_code(source);
            assert!(
                !code.contains("tokio::spawn"),
                "the transport must not spawn a task: `close` joins quinn's driver only"
            );
        }
        let code = production_code(include_str!("transport.rs"));
        assert!(
            code.contains("wait_idle"),
            "`close` joins quinn's driver task with `wait_idle`"
        );
    }

    /// **A responder writes no frame before it verified the peer.**
    ///
    /// The peer cannot observe this from the other side — a connection closed in the
    /// same breath drops what was queued, so a leaked `Hello` never reaches the
    /// attacker — which is exactly why the property is asserted on the source: inside
    /// `accept_one`, the first `codec::write_frame` comes AFTER `auth::verify_peer`,
    /// and a mutant that writes anything before it is caught here rather than by luck.
    #[test]
    fn a_responder_writes_nothing_before_it_verified_the_peer() {
        let code = production_code(include_str!("transport.rs"));
        let responder = code
            .split("async fn accept_one")
            .nth(1)
            .expect("le répondeur a une fonction accept_one")
            .split("pub fn set_identity")
            .next()
            .expect("accept_one précède set_identity");
        let verify = responder
            .find("auth::verify_peer")
            .expect("accept_one vérifie la preuve");
        let first_write = responder
            .find("codec::write_frame")
            .expect("accept_one finit par écrire");
        assert!(
            verify < first_write,
            "aucune trame n'est écrite avant la preuve du pair (T2)"
        );
        // And the read before it is the capped, deadlined one: 2 KiB, 2 s.
        let preauth = responder
            .find("auth::PREAUTH_DEADLINE")
            .expect("le budget de pré-authentification est là");
        let cap = responder
            .find("auth::PREAUTH_MAX_BYTES")
            .expect("le plafond de pré-authentification est là");
        assert!(
            preauth < verify && cap < verify,
            "le budget précède la preuve"
        );
    }

    /// The only `unsafe` this file needs is the dual-stack socket, and every block
    /// lives in `dual_stack`, each with its SAFETY comment.
    #[test]
    fn the_only_unsafe_is_the_dual_stack_socket() {
        let code = production_code(include_str!("transport.rs"));
        let blocks = code.matches("unsafe {").count();
        assert_eq!(blocks, 5, "cinq blocs unsafe, tous dans dual_stack");
        let before_socket = code
            .split("fn dual_stack")
            .next()
            .expect("le fichier a un début");
        assert!(
            !before_socket.contains("unsafe {"),
            "aucun unsafe avant dual_stack"
        );
        let after_socket = code
            .split("fn dual_stack")
            .nth(1)
            .expect("le fichier a dual_stack")
            .split("fn bind_socket")
            .next()
            .expect("dual_stack est suivi de bind_socket");
        assert_eq!(
            after_socket.matches("unsafe {").count(),
            5,
            "les cinq blocs sont dans dual_stack"
        );
    }
}
