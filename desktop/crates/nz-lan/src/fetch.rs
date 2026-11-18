//! Fetching a file or a stream from a peer, with resume, length and SHA-256
//! checks and a decoder probe (docs/agents/lan-nodes.md, *Integrity, resume*).
//!
//! **LN0 declares `FetchPlan`, `Fetched`, `fetch_to_file`, `PeerStream` and
//! `probe`; LN9 fills them.**
//!
//! What is already fixed, because the callers are:
//!
//! - a `Bulk` fetch takes this app's own [`nz_api::lanes::Gates`] — one Bulk
//!   transfer across server AND LAN — and is paced by `gates.throttle(bytes)` per
//!   chunk; a `Play` fetch never waits on them (rule 1);
//! - a resume carries the `FileTag` the requester already holds, so resuming onto
//!   a file that changed in between is refused rather than producing a file that
//!   is half of each;
//! - the free space of the target's filesystem is checked BEFORE writing (a peer
//!   announces a length, and a hostile one announces a wrong one);
//! - the file is published only once its length, its SHA-256 and a decoder probe
//!   (`probe`: codec and duration, ±2 s) all agree. A peer that fails is marked
//!   bad for the session ([`crate::peers::Peers::mark_bad`]).

use std::path::Path;
use std::sync::Arc;

use tokio::sync::watch;

use crate::proto;
use crate::transport::PeerConn;
use crate::types::{LanError, Urgency};

/// Everything one transfer needs to start.
#[derive(Debug)]
pub struct FetchPlan {
    /// The connection to fetch over.
    pub peer: PeerConn,
    pub track: String,
    /// The quality asked. A peer that does not have it answers what it serves,
    /// and `FetchOk`'s own quality is the truth.
    pub quality: nz_api::Quality,
    /// `Play` never waits; `Bulk` takes the gates.
    pub urgency: Urgency,
    /// Resume from this offset (0 = from the start).
    pub from: u64,
    /// The tag of the file the requester already holds, on a resume.
    pub expect: Option<proto::FileTag>,
}

/// What arrived, and what it is: the length, the SHA-256 (hex) computed over the
/// bytes written, and the quality the peer actually served.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    pub len: u64,
    pub sha256: String,
    pub quality: nz_api::Quality,
}

/// Fetches one file into the reserved `.part` file, resuming at `plan.from`,
/// reporting progress as `(bytes so far, total)`.
///
/// `stop` is the caller's cancel (a download queue told to pause, the app
/// closing): it is awaited between chunks, never after the fact.
pub async fn fetch_to_file(
    plan: FetchPlan,
    part: &Path,
    gates: &nz_api::lanes::Gates,
    stop: watch::Receiver<bool>,
    progress: &mut (dyn FnMut(u64, u64) + Send),
) -> Result<Fetched, LanError> {
    let _ = (plan, part, gates, stop, progress);
    Err(LanError::Unsupported)
}

/// A peer's file as a live stream, for playback: byte-seekable by construction
/// because a peer file is always complete.
#[derive(Debug)]
pub struct PeerStream;

impl PeerStream {
    /// Opens `Fetch` for streaming, taking the gates when the urgency is `Bulk`.
    pub async fn open(
        plan: FetchPlan,
        gates: Arc<nz_api::lanes::Gates>,
    ) -> Result<PeerStream, LanError> {
        let _ = (plan, gates);
        Err(LanError::Unsupported)
    }

    /// The file's total length: what `ByteStream::len_hint` answers, without a
    /// request.
    pub fn len(&self) -> u64 {
        0
    }

    /// Whether this stream is empty — never true for a fetched file (a zero-byte
    /// audio file is not a file), and here so `len` has its pair.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The quality the peer actually serves — LN5's `ByteStream::quality()` reads
    /// this, so the spool is published into the play cache under the right key.
    pub fn quality(&self) -> nz_api::Quality {
        nz_api::Quality::Flac
    }

    /// The next chunk, `None` at the end.
    pub async fn chunk(&mut self) -> Result<Option<Vec<u8>>, LanError> {
        Ok(None)
    }
}

/// Opens `path` with the decoder and checks it is what it was announced to be: the
/// right codec for `quality`, and, when the metadata says so, `duration_s` ±2 s.
///
/// This is what stops a truncated file or a different file from being published:
/// QUIC authenticated the transit, the tag checked the bytes, and the decoder is
/// the only thing that can say the bytes are AUDIO.
pub fn probe(
    path: &Path,
    quality: nz_api::Quality,
    duration_s: Option<u32>,
) -> Result<(), LanError> {
    let _ = (path, quality, duration_s);
    Err(LanError::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stub refuses, and refuses with the SAME variant every time: nothing here
    /// can be mistaken for "this peer does not have it" (`NotFound`), which is a
    /// verdict the decision path acts on.
    #[test]
    fn an_unimplemented_fetch_refuses_without_a_verdict() {
        assert_eq!(
            probe(Path::new("/does/not/exist"), nz_api::Quality::Flac, None),
            Err(LanError::Unsupported)
        );
    }
}
