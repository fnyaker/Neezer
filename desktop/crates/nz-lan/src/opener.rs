//! `PeerFirstOpener`: the app's own opener, with a peer asked first
//! (docs/agents/lan-nodes.md, *Playback: peer first*).
//!
//! **LN0 declares the wrapper; LN11 fills the behaviour.**
//!
//! Wrapping the existing opener — the desktop's `NetOpener`, the phone's
//! `ServerOpener` — is what keeps the engine unchanged: it is still one
//! `nz_audio::StreamOpener`, and every rule it already has (lanes, the spool, the
//! seek plan) applies. What the wrapper adds, once LN11 is written:
//!
//! - `local()` delegates: a local file always wins.
//! - `open` runs [`crate::decide`]; for a peer it opens a `Fetch` with the
//!   urgency its lane implies (`User → Play`, `Prefetch → Prefetch`, `Bulk → Bulk`)
//!   and returns a byte-seekable stream whose length is the file's.
//! - `ready()` answers from the replica inventory, with no request: a peer track
//!   is always complete on that peer.
//! - it fails FAST: no connected peer has the track → the inner opener, at once,
//!   with 0 ms added and no LAN request (rule 3).
//! - a peer that stalls 1.5 s without a byte while the engine waits on `User` is
//!   abandoned, and the server takes over.
//!
//! Until LN11, every method delegates to the inner opener: the app compiles with
//! the wrapper in place, and behaves exactly as it does today.

use std::path::PathBuf;

use nz_audio::{ByteStream, Lane, Quality, SourceError, StreamOpener, StreamReady};

use crate::service::Lan;

/// The app's opener with a peer asked first.
pub struct PeerFirstOpener<O: StreamOpener> {
    inner: O,
    lan: Lan,
}

impl<O: StreamOpener> PeerFirstOpener<O> {
    /// Wraps `inner`, asking `lan` before it for every track.
    pub fn new(inner: O, lan: Lan) -> Self {
        PeerFirstOpener { inner, lan }
    }

    /// The node behind this opener (LN11's decision and the tests read it).
    pub fn lan(&self) -> &Lan {
        &self.lan
    }
}

impl<O: StreamOpener> StreamOpener for PeerFirstOpener<O> {
    /// The body of a track: from a peer when [`crate::decide`] says so, from the
    /// inner opener otherwise.
    fn open(
        &self,
        id: &str,
        quality: Quality,
        start: Option<f64>,
        from_byte: Option<u64>,
        lane: Lane,
    ) -> Result<Box<dyn ByteStream>, SourceError> {
        // LN0: delegate, so wrapping changes nothing yet.
        self.inner.open(id, quality, start, from_byte, lane)
    }

    /// Whether the track can be played now. From a peer: answered from memory,
    /// with the file's length and no request.
    fn ready(&self, id: &str, quality: Quality) -> Result<StreamReady, SourceError> {
        self.inner.ready(id, quality)
    }

    /// A complete local file, if the play cache has one: local files stay first.
    fn local(&self, id: &str, quality: Quality) -> Option<PathBuf> {
        self.inner.local(id, quality)
    }

    /// The engine is closing: abandon every in-flight wait, then let the inner
    /// opener do the same. A thread that borrows the runtime is joined before that
    /// runtime stops.
    fn shutdown(&self) {
        self.inner.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nz_audio::{ByteStream, Quality, SourceError, StreamReady};

    /// The barest opener: it has nothing, and it answers "offline", which is what
    /// a wrapper must be able to wrap.
    struct Nothing;

    impl StreamOpener for Nothing {
        fn open(
            &self,
            _id: &str,
            _quality: Quality,
            _start: Option<f64>,
            _from_byte: Option<u64>,
            _lane: Lane,
        ) -> Result<Box<dyn ByteStream>, SourceError> {
            Err(SourceError::Offline)
        }

        fn ready(&self, _id: &str, _quality: Quality) -> Result<StreamReady, SourceError> {
            Ok(StreamReady::default())
        }

        fn local(&self, _id: &str, _quality: Quality) -> Option<PathBuf> {
            None
        }
    }

    /// The engine calls an opener from the spooler thread and from the UI thread,
    /// so the wrapper must be `Send + Sync` AND one `StreamOpener` — that is the
    /// whole contract the engine has, and LN11 fills the bodies behind it.
    ///
    /// A live `Lan` only exists once LN10 can start a node on a server that has the
    /// feature, so LN0 proves the shape and LN11 proves the behaviour with two real
    /// nodes.
    #[test]
    fn the_wrapper_is_one_send_sync_stream_opener() {
        fn assert_opener<O: StreamOpener + Send + Sync>() {}
        // Named once so the compiler does not read the fake as dead code.
        #[allow(clippy::let_underscore_untyped, reason = "the fake exists to be named")]
        let _inner = Nothing;
        assert_opener::<PeerFirstOpener<Nothing>>();
        // The trait's own types are `nz-audio`'s, not the API's: the engine speaks
        // them, and a wrapper cannot quietly swap one enum for another.
        let _lane = Lane::User;
        let _quality = Quality::Flac;
    }
}
