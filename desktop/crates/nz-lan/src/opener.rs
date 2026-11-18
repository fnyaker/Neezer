//! `PeerFirstOpener`: the app's own opener, with a peer asked first
//! (docs/agents/lan-nodes.md, *Playback: peer first*).
//!
//! **LN11 filled the body.** Wrapping the existing opener — the desktop's
//! `NetOpener`, the phone's `ServerOpener` — is what keeps the engine unchanged:
//! it is still one `nz_audio::StreamOpener`, and every rule it already has
//! (lanes, the spool, the seek plan) applies.
//!
//! What the wrapper adds:
//!
//! - `local()` delegates: a local file always wins.
//! - `open` runs [`crate::decide`] (through `Lan::decide`); for a peer it opens a
//!   `Fetch` with the urgency its lane implies (`User → Play`, `Prefetch →
//!   Prefetch`, `Bulk → Bulk`) and returns a byte-seekable stream whose
//!   `len_hint()` is what remains of the file (a peer file is always complete, so
//!   seeks are byte ranges) and whose `quality()` is the quality actually served
//!   (LN5), so the engine publishes the spool into the play cache under the right
//!   key. `start` (a `?start=` slice) never goes to a peer.
//! - `ready()` answers from the replica inventory, with **no request**: a peer
//!   track is always complete on that peer, so a seek lands at once.
//! - it fails FAST: no connected peer has the track → the inner opener, at once,
//!   with 0 ms added and no LAN request (rule 3).
//! - a peer that stalls **1.5 s** without a byte while the engine waits on `User`
//!   is abandoned, and the server takes over from the byte already received (or,
//!   with no server, the peer's copy restarts at 0).
//! - the choice is **sticky per track**: a seek (a byte-range reopen) asks the
//!   same peer again with `from_byte`, instead of re-deciding onto another device.
//!
//! Everything that touches the runtime goes through [`crate::import::run_on`],
//! which never panics when it is called from inside a runtime: the engine calls
//! this from its decoder threads, and a test may call it from an async one.

use std::collections::HashMap;
use std::io::{self, Read};
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nz_audio::{ByteStream, Lane, Quality, SourceError, StreamOpener, StreamReady};

use crate::fetch::{FetchPlan, PeerStream};
use crate::import::run_on;
use crate::service::Lan;
use crate::types::{DevicePub, Source, Urgency};

/// How long a peer may stay silent while the person waits before the server takes
/// over (`docs/agents/lan-nodes.md`, *Playback*: 1.5 s).
pub const STALL: Duration = Duration::from_millis(1_500);

/// How many tracks keep their last peer. A seek re-opens one track, so this only
/// bounds a long listening session's map.
pub const STICKY_CAP: usize = 256;

/// The app's opener with a peer asked first.
pub struct PeerFirstOpener<O: StreamOpener> {
    /// The app's own opener, behind `dyn` so a stream can hold it and fall back
    /// to it mid-track. `O` stays as the type parameter the contract names.
    inner: Arc<dyn StreamOpener>,
    lan: Lan,
    /// `track → (peer, quality)`: the sticky choice a seek reopens from.
    sticky: Mutex<HashMap<String, (DevicePub, nz_api::Quality)>>,
    marker: PhantomData<fn() -> O>,
}

impl<O: StreamOpener> PeerFirstOpener<O> {
    /// The node behind this opener (LN11's decision and the tests read it).
    pub fn lan(&self) -> &Lan {
        &self.lan
    }

    /// The peer that served `track` last, if any.
    fn sticky(&self, track: &str) -> Option<(DevicePub, nz_api::Quality)> {
        self.sticky
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(track)
            .copied()
    }

    fn remember(&self, track: &str, peer: DevicePub, quality: nz_api::Quality) {
        let mut sticky = self
            .sticky
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if sticky.len() >= STICKY_CAP && !sticky.contains_key(track) {
            // The oldest entry goes: a listening session is a queue, and the
            // tracks behind it are the ones nobody will seek any more.
            if let Some(any) = sticky.keys().next().cloned() {
                sticky.remove(&any);
            }
        }
        sticky.insert(track.to_owned(), (peer, quality));
    }
}

impl<O: StreamOpener + 'static> PeerFirstOpener<O> {
    /// Wraps `inner`, asking `lan` before it for every track.
    ///
    /// `'static` is what lets a stream hold the inner opener (and fall back to it
    /// mid-track); an opener lives for the process, so that is no constraint in
    /// practice.
    pub fn new(inner: O, lan: Lan) -> Self {
        PeerFirstOpener {
            inner: Arc::new(inner),
            lan,
            sticky: Mutex::new(HashMap::new()),
            marker: PhantomData,
        }
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
        let urgency = crate::import::urgency_of(lane);
        let wanted = api_quality(quality);
        // A seek (a byte-range reopen) goes back to the peer that served the
        // track, as long as it is still connected and still has that rung.
        let decision = match (from_byte, self.sticky(id)) {
            (Some(_), Some((peer, served))) if self.lan.conn(&peer).is_some() => Source::Peer {
                peer,
                quality: served,
            },
            _ => self.lan.decide(id, wanted, urgency),
        };
        let Source::Peer {
            peer,
            quality: served,
        } = decision
        else {
            // Local and Server go to the app's opener unchanged: the LAN adds
            // nothing to a file on disk, and a server request is the same request.
            return self.inner.open(id, quality, start, from_byte, lane);
        };
        let (Some(handle), Some(conn)) = (self.lan.handle(), self.lan.conn(&peer)) else {
            return self.inner.open(id, quality, start, from_byte, lane);
        };
        let from = from_byte.unwrap_or(0);
        let plan = FetchPlan {
            peer: conn,
            track: id.to_owned(),
            quality: served,
            urgency,
            from,
            expect: None,
        };
        let gates = self.lan.api().gates();
        let opening = async move { PeerStream::open(plan, gates).await };
        // `User` is the person: a peer that does not answer the head in 1.5 s is
        // abandoned and the server answers instead (one request fewer than
        // waiting for quinn's own timeout).
        let opened = if lane == Lane::User {
            run_on(&handle, async move {
                tokio::time::timeout(STALL, opening).await.ok()
            })
        } else {
            run_on(&handle, async move { Some(opening.await) })
        };
        let Some(Some(Ok(stream))) = opened else {
            // Fail fast: nothing was received, so the server starts exactly where
            // the inner opener would have started without the LAN.
            return self.inner.open(id, quality, start, from_byte, lane);
        };
        self.remember(id, peer, served);
        Ok(Box::new(PeerBody {
            handle,
            inner: Arc::clone(&self.inner),
            lan: self.lan.clone(),
            track: id.to_owned(),
            peer,
            requested: quality,
            served,
            from,
            received: 0,
            user: lane == Lane::User,
            pending: Vec::new(),
            stream: Some(stream),
            fallback: None,
            done: false,
        }))
    }

    /// Whether the track can be played now. From a peer: answered from memory,
    /// with the file's length and **no request**.
    fn ready(&self, id: &str, quality: Quality) -> Result<StreamReady, SourceError> {
        let wanted = api_quality(quality);
        match self.lan.decide(id, wanted, Urgency::Play) {
            Source::Peer {
                quality: served, ..
            } => Ok(StreamReady {
                ready: true,
                // A peer file is finished on disk: a byte range lands anywhere.
                ranges: Some("bytes".to_owned()),
                bytes: self.lan.peer_len(id, served),
                cached: false,
                offset: false,
                streaming: false,
            }),
            // A produced body is never seeked by symphonia: the engine reopens it
            // with `?start=`, which is the server's own contract.
            _ => self.inner.ready(id, quality),
        }
    }

    /// A complete local file, if the play cache (or a download) has one: local
    /// files stay first.
    fn local(&self, id: &str, quality: Quality) -> Option<PathBuf> {
        self.inner.local(id, quality)
    }

    /// The engine is closing: abandon every in-flight wait, then let the inner
    /// opener do the same. A thread that borrows the runtime is joined before that
    /// runtime stops.
    fn shutdown(&self) {
        self.sticky
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
        self.inner.shutdown();
    }
}

/// One peer's file as a blocking body the decoder reads.
///
/// The `PeerStream` is MOVED into the short future that reads each chunk and
/// moved back, so no lock is held across an await and no thread of this struct's
/// own is ever created: the runtime's task is the only other party.
struct PeerBody {
    handle: tokio::runtime::Handle,
    inner: Arc<dyn StreamOpener>,
    lan: Lan,
    track: String,
    /// The device serving this body, so a body that cannot go on takes THAT peer
    /// out of the track's next decision.
    peer: DevicePub,
    requested: Quality,
    served: nz_api::Quality,
    /// The file offset this body starts at (a byte-range reopen).
    from: u64,
    /// How many bytes this body has handed to the decoder.
    received: u64,
    /// `true` for the person's own lane: the one a stall falls back from.
    user: bool,
    pending: Vec<u8>,
    stream: Option<PeerStream>,
    /// The server, once it took over: the rest of the track comes from it.
    fallback: Option<Box<dyn ByteStream>>,
    done: bool,
}

impl ByteStream for PeerBody {
    fn len_hint(&self) -> Option<u64> {
        // The bytes this body will hand out: the file's length MINUS its start,
        // which is what `HttpSource` adds its `target` to (a server Range answers
        // the same way).
        self.stream
            .as_ref()
            .map(|stream| stream.len().saturating_sub(self.from))
    }

    fn start_offset_s(&self) -> f64 {
        // A peer never gets `?start=`: the decoder seeks by bytes.
        0.0
    }

    fn quality(&self) -> Option<Quality> {
        Some(audio_quality(self.served))
    }
}

impl PeerBody {
    /// The next chunk, or `None` at the end. A stall on the person's lane hands
    /// the rest of the track to the server.
    fn next_chunk(&mut self) -> io::Result<Option<Vec<u8>>> {
        loop {
            let Some(mut stream) = self.stream.take() else {
                return Ok(None);
            };
            let user = self.user;
            let timeout = STALL;
            let read = run_on(&self.handle, async move {
                match tokio::time::timeout(timeout, stream.chunk()).await {
                    Ok(result) => (stream, Some(result)),
                    Err(_) => (stream, None),
                }
            });
            let Some((stream, result)) = read else {
                // The node's runtime is gone (the app is closing): the body ends
                // rather than blocking a thread the engine is about to join.
                self.stream = None;
                return Ok(None);
            };
            match result {
                Some(Ok(Some(chunk))) => {
                    self.stream = Some(stream);
                    return Ok(Some(chunk));
                }
                Some(Ok(None)) => {
                    self.stream = None;
                    self.done = true;
                    return Ok(None);
                }
                Some(Err(error)) => {
                    self.stream = Some(stream);
                    if user {
                        self.take_over()?;
                        return Ok(None);
                    }
                    return Err(io::Error::other(error.to_string()));
                }
                None => {
                    self.stream = Some(stream);
                    if user {
                        self.take_over()?;
                        return Ok(None);
                    }
                    // Not the person's lane: a background read keeps waiting.
                    continue;
                }
            }
        }
    }

    /// The peer stalled or died while the person waits: the SERVER continues from
    /// the byte already received, which is only honest when both sides send the
    /// same audio.
    ///
    /// The archive is: a peer's FLAC left the server's own archive, so the same
    /// file answers a `Range: bytes=<n>-` with the same bytes. An Opus transcode
    /// is NOT byte-reproducible (the plan's *Integrity* row says so), and a peer
    /// that serves a tolerated rung other than the one asked sends another
    /// encoding altogether — splicing either would hand the decoder a file that is
    /// half of each. In those cases the body stops and the peer is marked bad for
    /// the track, so the app's retry (rule 3's « Réessayer ») goes to the server
    /// from 0 with a clean file.
    fn take_over(&mut self) -> io::Result<()> {
        let absolute = self.from + self.received;
        let same_bytes = api_quality(self.requested) == nz_api::Quality::Flac
            && self.served == nz_api::Quality::Flac;
        if same_bytes
            && self.lan.api().is_online()
            && let Ok(body) =
                self.inner
                    .open(&self.track, Quality::Flac, None, Some(absolute), Lane::User)
        {
            self.fallback = Some(body);
            self.stream = None;
            tracing::debug!(
                track = self.track.as_str(),
                absolute,
                "LAN : le pair a calé, le serveur reprend au même octet"
            );
            return Ok(());
        }
        self.lan.mark_bad(&self.peer, &self.track);
        Err(io::Error::other("le pair ne répond plus"))
    }
}

impl Read for PeerBody {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            if let Some(fallback) = self.fallback.as_mut() {
                return fallback.read(buf);
            }
            if self.pending.is_empty() {
                match self.next_chunk()? {
                    Some(chunk) => self.pending = chunk,
                    None => {
                        if self.fallback.is_some() {
                            // The server took over: read it, from the next turn.
                            continue;
                        }
                        self.done = true;
                        return Ok(0);
                    }
                }
            }
            let take = buf.len().min(self.pending.len());
            buf[..take].copy_from_slice(&self.pending[..take]);
            self.pending.drain(..take);
            self.received += take as u64;
            return Ok(take);
        }
    }
}

/// The engine's four rungs in this crate's words (LN0's rule: the wrapper converts
/// at its boundary and never swaps one enum for the other).
pub(crate) fn api_quality(quality: Quality) -> nz_api::Quality {
    match quality {
        Quality::Flac => nz_api::Quality::Flac,
        Quality::Opus320 => nz_api::Quality::Opus320,
        Quality::Opus128 => nz_api::Quality::Opus128,
        Quality::Opus64 => nz_api::Quality::Opus64,
    }
}

/// The same, back: what `ByteStream::quality` answers the engine.
pub(crate) fn audio_quality(quality: nz_api::Quality) -> Quality {
    match quality {
        nz_api::Quality::Flac => Quality::Flac,
        nz_api::Quality::Opus320 | nz_api::Quality::Opus256 | nz_api::Quality::Opus192 => {
            Quality::Opus320
        }
        nz_api::Quality::Opus128 => Quality::Opus128,
        nz_api::Quality::Opus64 => Quality::Opus64,
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

    /// The two enums meet at this boundary only, and every rung survives the round
    /// trip the engine relies on: a body that says `Opus320` must reach the play
    /// cache as `OPUS_320`, not as FLAC.
    #[test]
    fn every_rung_survives_the_conversion() {
        for quality in [
            Quality::Flac,
            Quality::Opus320,
            Quality::Opus128,
            Quality::Opus64,
        ] {
            assert_eq!(audio_quality(api_quality(quality)), quality);
        }
        // The server's two extra rungs are never what a peer serves: the nearest
        // rung below is what the engine can decode and cache.
        assert_eq!(audio_quality(nz_api::Quality::Opus256), Quality::Opus320);
        assert_eq!(audio_quality(nz_api::Quality::Opus192), Quality::Opus320);
    }
}
