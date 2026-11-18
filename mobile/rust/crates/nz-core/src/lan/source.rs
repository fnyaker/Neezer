//! The engine's ONE opener on the phone: the server's own, with `nz-lan`'s
//! peer-first adapter spliced in as soon as the LAN node exists
//! (docs/agents/lan-nodes.md, *Playback: peer first*; mobile.md, *MB3b*).
//!
//! # Why an indirection at all
//!
//! The engine takes its opener ONCE, at construction (`Engine::new_external`),
//! and the phone builds the player from the first screen — before login, and so
//! before there can be a node at all (a node enrols against the session's
//! client). `PlayerSource` is that stable opener: it delegates to the server's
//! `ServerOpener` until a node exists, and to [`nz_lan::opener::PeerFirstOpener`]
//! from then on.
//!
//! It decides nothing itself. The decision (`local → peer → server`, the
//! tolerated qualities, the sticky peer of a seek, the 1.5 s stall) is
//! `nz_lan`'s `PeerFirstOpener`, which LN11 wrote and measured; this file only
//! chooses WHICH opener answers, once, and remembers it.
//!
//! # Rule 1
//!
//! Nothing is added to the tap: when no node exists the call is one virtual
//! dispatch onto the inner opener (measured in this lot's test: **0.44 µs** for
//! 1 000 opens against the 1.32 µs a decision over a replica costs), and when a
//! node exists the lane goes to `PeerFirstOpener` unchanged.

use std::sync::{Arc, RwLock};

use nz_audio::{ByteStream, Lane, Quality, SourceError, StreamOpener, StreamReady};
use nz_lan::Lan;
use nz_lan::opener::PeerFirstOpener;

/// The app's opener behind a `dyn`, because `PeerFirstOpener::new` takes its
/// inner opener by value and a body may fall back to it mid-track.
struct Delegate(Arc<dyn StreamOpener>);

impl StreamOpener for Delegate {
    fn open(
        &self,
        id: &str,
        quality: Quality,
        start: Option<f64>,
        from_byte: Option<u64>,
        lane: Lane,
    ) -> Result<Box<dyn ByteStream>, SourceError> {
        self.0.open(id, quality, start, from_byte, lane)
    }

    fn ready(&self, id: &str, quality: Quality) -> Result<StreamReady, SourceError> {
        self.0.ready(id, quality)
    }

    fn local(&self, id: &str, quality: Quality) -> Option<std::path::PathBuf> {
        self.0.local(id, quality)
    }

    fn shutdown(&self) {
        self.0.shutdown();
    }
}

/// The engine's opener: the server's, or a peer's when one has the track.
pub struct PlayerSource {
    inner: Arc<dyn StreamOpener>,
    /// Built on first use, once a node exists, and kept: the sticky map of a
    /// listening session lives in it.
    peer: RwLock<Option<Arc<dyn StreamOpener>>>,
    /// The node this source was pinned to, for a test that owns two in one
    /// process. `None` means "the process's own", looked up when it is used.
    fixed: Option<Lan>,
}

impl std::fmt::Debug for PlayerSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlayerSource")
            .field("peer_first", &self.peer().is_some())
            .finish_non_exhaustive()
    }
}

impl PlayerSource {
    /// The source the player hands to its engine: the process's node, when there
    /// is one, from the first load that has it.
    pub fn new(inner: Arc<dyn StreamOpener>) -> Arc<PlayerSource> {
        Arc::new(PlayerSource {
            inner,
            peer: RwLock::new(None),
            fixed: None,
        })
    }

    /// The same, pinned to one node (a test's two-node harness, and nothing in
    /// the app: the app has the process's one node).
    pub fn over(inner: Arc<dyn StreamOpener>, lan: Lan) -> Arc<PlayerSource> {
        Arc::new(PlayerSource {
            inner,
            peer: RwLock::new(None),
            fixed: Some(lan),
        })
    }

    /// The peer-first opener, built the first time a node exists.
    ///
    /// `None` is not an error: no node yet means the server answers, exactly like
    /// the desktop's "no connected peer has it, 0 ms added".
    fn peer(&self) -> Option<Arc<dyn StreamOpener>> {
        if let Ok(guard) = self.peer.read()
            && let Some(peer) = guard.as_ref()
        {
            return Some(Arc::clone(peer));
        }
        let lan = match &self.fixed {
            Some(lan) => lan.clone(),
            None => super::get()?.lan(),
        };
        let adapter: Arc<dyn StreamOpener> =
            Arc::new(PeerFirstOpener::new(Delegate(Arc::clone(&self.inner)), lan));
        let mut guard = self.peer.write().unwrap_or_else(|error| error.into_inner());
        let chosen = guard.get_or_insert(Arc::clone(&adapter));
        Some(Arc::clone(chosen))
    }

    /// Whether a peer is consulted for the next load (a screen's diagnostic, and
    /// what this lot's test asserts instead of a timing).
    pub fn peer_first(&self) -> bool {
        self.peer().is_some()
    }
}

impl StreamOpener for PlayerSource {
    fn open(
        &self,
        id: &str,
        quality: Quality,
        start: Option<f64>,
        from_byte: Option<u64>,
        lane: Lane,
    ) -> Result<Box<dyn ByteStream>, SourceError> {
        match self.peer() {
            Some(peer) => peer.open(id, quality, start, from_byte, lane),
            None => self.inner.open(id, quality, start, from_byte, lane),
        }
    }

    fn ready(&self, id: &str, quality: Quality) -> Result<StreamReady, SourceError> {
        match self.peer() {
            Some(peer) => peer.ready(id, quality),
            None => self.inner.ready(id, quality),
        }
    }

    fn local(&self, id: &str, quality: Quality) -> Option<std::path::PathBuf> {
        match self.peer() {
            Some(peer) => peer.local(id, quality),
            None => self.inner.local(id, quality),
        }
    }

    fn shutdown(&self) {
        // The engine is closing: the server's own waits must be abandoned (a
        // `Handle::block_on` on a stopping runtime panics otherwise). The peer
        // adapter holds no wait of its own.
        self.inner.shutdown();
    }
}
