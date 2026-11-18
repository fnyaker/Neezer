//! The responder: answering a peer's `Inventory`, `Sets`, `Meta`, `Cover` and
//! `Fetch`, within that peer's [`Scope`] and within this device's own serve limits
//! (docs/agents/lan-nodes.md, *Rule 1 on the LAN*).
//!
//! **LN4 filled it.** Serving a peer is BACKGROUND work for this device (rule 1):
//! at most [`ServeLimits::default`]'s two `Play`, one `Prefetch` and one `Bulk`
//! transfer at once across all peers, a `Bulk` serve waits on this node's own
//! [`nz_api::lanes::Gates`] so it never slows what the person is listening to, and
//! a request beyond those limits is answered `Busy { retry_in_ms }` rather than
//! queued for ever. Reads go in 64 KiB chunks.
//!
//! Every handler takes `&Scope`: there is no handler without it, and an out-of-
//! scope request is answered by `Answer::absent()` — the ONE function that builds
//! `Response::NotFound` — so a friend cannot tell an unshared track from a track
//! this node does not have (T7, compared byte for byte in `tests/serve.rs`).
//!
//! # `handle` and `answer`
//!
//! [`Server::handle`] is the whole request: the answer, then the bytes, then the
//! slot. [`Server::answer`] is the decision alone (head + body, no transport), so a
//! test drives every scope and every limit without a QUIC connection — LN4's tests
//! build no `Incoming` at all, and LN6's transport feeds the same function.

use std::io::SeekFrom;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncSeekExt, ReadBuf};
use tracing::debug;

use crate::index::Index;
use crate::proto;
use crate::scope::Scope;
use crate::transport::Incoming;
use crate::types::{LanError, Urgency};

/// How long a requester is told to wait when this device serves what it will.
///
/// Two seconds, not a spin's worth: a peer that answers `Busy` waits exactly this,
/// asks again if the track is still wanted, and a long transfer therefore costs
/// one request per two seconds rather than a storm (rule 3).
pub const BUSY_RETRY_MS: u32 = 2_000;

/// How much one read of a served file takes (the note's 64 KiB).
pub const CHUNK_BYTES: usize = 64 * 1024;

/// How many transfers one device serves at once, per urgency.
///
/// Two `Play` (two of the person's own devices can tap a track at the same moment)
/// and one each of the rest: a third simultaneous `Play` gets `Busy`, which the
/// requester honours exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServeLimits {
    pub play: usize,
    pub prefetch: usize,
    pub bulk: usize,
}

impl Default for ServeLimits {
    fn default() -> Self {
        ServeLimits {
            play: 2,
            prefetch: 1,
            bulk: 1,
        }
    }
}

/// The transfers being served right now, per urgency.
#[derive(Debug, Default)]
struct Counts {
    play: AtomicUsize,
    prefetch: AtomicUsize,
    bulk: AtomicUsize,
}

impl Counts {
    fn take<'a>(&'a self, urgency: Urgency, limits: &ServeLimits) -> Option<Slot<'a>> {
        let (counter, cap) = match urgency {
            Urgency::Play => (&self.play, limits.play),
            Urgency::Prefetch => (&self.prefetch, limits.prefetch),
            Urgency::Bulk => (&self.bulk, limits.bulk),
        };
        let mut current = counter.load(Ordering::Relaxed);
        loop {
            if current >= cap {
                return None;
            }
            match counter.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    return Some(Slot {
                        counts: self,
                        urgency,
                    });
                }
                Err(seen) => current = seen,
            }
        }
    }

    fn release(&self, urgency: Urgency) {
        let counter = match urgency {
            Urgency::Play => &self.play,
            Urgency::Prefetch => &self.prefetch,
            Urgency::Bulk => &self.bulk,
        };
        counter.fetch_sub(1, Ordering::AcqRel);
    }

    fn read(&self) -> (usize, usize, usize) {
        (
            self.play.load(Ordering::Relaxed),
            self.prefetch.load(Ordering::Relaxed),
            self.bulk.load(Ordering::Relaxed),
        )
    }
}

/// One transfer's slot: dropping it frees the count.
struct Slot<'a> {
    counts: &'a Counts,
    urgency: Urgency,
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.counts.release(self.urgency);
    }
}

/// The responder: the local index, the gates it yields to, and the serve counts.
#[derive(Clone)]
pub struct Server {
    index: Index,
    gates: Arc<nz_api::lanes::Gates>,
    limits: ServeLimits,
    counts: Arc<Counts>,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server")
            .field("limits", &self.limits)
            .field("counts", &self.counts.read())
            .field("index", &self.index)
            .finish()
    }
}

/// One request's answer: the head frame, the bytes that follow it, and the two
/// permits (a serve slot, a `Bulk` lane) held until the bytes are written.
pub struct Answer<'a> {
    pub head: proto::Response,
    pub body: Option<Box<dyn AsyncRead + Unpin + Send>>,
    /// The serve count, held while the transfer runs. `Option` because an answer
    /// that is not a transfer (`Ping`, a `NotFound`) takes none.
    slot: Option<Slot<'a>>,
    /// The `Bulk` lane, held while a background transfer writes. A `Play` or
    /// `Prefetch` serve takes none: those are never slowed down.
    lane: Option<nz_api::lanes::LaneGuard<'a>>,
}

impl Answer<'_> {
    /// A small answer with no body and no transfer slot.
    fn short(head: proto::Response) -> Answer<'static> {
        Answer {
            head,
            body: None,
            slot: None,
            lane: None,
        }
    }

    /// The refusal: **one** function, so "this track is not here", "this track is
    /// not shared with you", "the offset is past the end" and "the file changed"
    /// all produce the same bytes. A friend must not learn anything from the
    /// difference (T7).
    pub fn absent() -> Answer<'static> {
        Answer::short(proto::Response::NotFound)
    }

    /// How many body bytes follow, `0` for an answer without a body.
    pub fn has_body(&self) -> bool {
        self.body.is_some()
    }
}

impl Server {
    /// A responder over `index`, sharing this device's `gates` (the SAME
    /// `nz-api` gates the app's own requests take, so one Bulk slot covers server
    /// and LAN), with `limits` as its own serve budget.
    pub fn new(index: Index, gates: Arc<nz_api::lanes::Gates>, limits: ServeLimits) -> Server {
        Server {
            index,
            gates,
            limits,
            counts: Arc::new(Counts::default()),
        }
    }

    /// Answers one request, and answers the number of body bytes written.
    ///
    /// A request that hits a limit answers `Busy` and writes no body; one this
    /// peer may not see answers the same `NotFound` an absent file does.
    pub async fn handle(&self, scope: &Scope, incoming: Incoming) -> Result<u64, LanError> {
        let request = incoming.request.clone();
        let Answer {
            head,
            body,
            slot,
            lane,
        } = self.answer(scope, &request).await;
        let written = incoming.respond(head, body).await;
        // The permits live until the bytes are written, and not one statement
        // longer: a peer that hangs up frees its slot here too.
        drop(slot);
        drop(lane);
        written
    }

    /// The decision alone: what this request is answered with, and the bytes that
    /// follow.
    ///
    /// Split from [`Server::handle`] so every scope and every limit is testable
    /// without a transport: LN4's tests build no `Incoming`.
    pub async fn answer<'a>(&'a self, scope: &Scope, request: &proto::Request) -> Answer<'a> {
        match request {
            proto::Request::Ping => Answer::short(proto::Response::Pong),
            proto::Request::Inventory { since, page } => {
                Answer::short(self.index.inventory(scope, *since, *page))
            }
            proto::Request::Sets { page } => Answer::short(self.index.sets(scope, *page)),
            proto::Request::Meta { tracks } => Answer::short(self.index.meta(scope, tracks)),
            proto::Request::Cover { url } => self.cover(scope, url).await,
            proto::Request::Fetch {
                track,
                quality,
                from,
                expect,
                urgency,
            } => {
                self.fetch(scope, track, *quality, *from, expect.as_ref(), *urgency)
                    .await
            }
        }
    }

    /// How many transfers are being served right now: `(play, prefetch, bulk)`.
    /// The screens show it and the tests count it; nothing else reads it.
    pub fn counts(&self) -> (usize, usize, usize) {
        self.counts.read()
    }

    /// Answers `Cover`: the file, or the refusal.
    async fn cover<'a>(&'a self, scope: &Scope, url: &str) -> Answer<'a> {
        // The index validates the URL's shape AND checks that a track in `scope`
        // references it; a URL outside the scope never reaches the cover store.
        let Some(path) = self.index.cover(scope, url) else {
            return Answer::absent();
        };
        let file = match tokio::fs::File::open(&path).await {
            Ok(file) => file,
            // The LRU may have taken it between the row and the open: an absent
            // cover, never an error a peer can tell apart from an unshared one.
            Err(error) => {
                debug!(%error, "a cached cover vanished between its row and its open");
                return Answer::absent();
            }
        };
        let Ok(meta) = file.metadata().await else {
            return Answer::absent();
        };
        let Ok(len) = u32::try_from(meta.len()) else {
            return Answer::absent();
        };
        Answer {
            head: proto::Response::Cover {
                mime: mime_of(&path).to_owned(),
                len,
            },
            body: Some(Box::new(file)),
            slot: None,
            lane: None,
        }
    }

    /// Answers `Fetch`: the file from `from`, or the refusal.
    async fn fetch<'a>(
        &'a self,
        scope: &Scope,
        track: &str,
        quality: proto::WireQuality,
        from: u64,
        expect: Option<&proto::FileTag>,
        urgency: Urgency,
    ) -> Answer<'a> {
        // Scope first, and the same `absent()` at the end whatever fails: the
        // refusal must not encode WHY (T7).
        if !scope.allows(track) {
            return Answer::absent();
        }
        // Then the serve budget: a third simultaneous `Play` is told to come back,
        // never queued behind a transfer that may last minutes (rule 1 on THIS
        // device's resources).
        let Some(slot) = self.counts.take(urgency, &self.limits) else {
            return Answer::short(proto::Response::Busy {
                retry_in_ms: BUSY_RETRY_MS,
            });
        };
        // The asked-for quality, or the best copy this node has (the answer names
        // what it serves, which is what makes that honest).
        let wanted = nz_api::Quality::from(quality);
        let Some(local) = self
            .index
            .local(track, wanted)
            .or_else(|| self.index.best_local(track))
        else {
            return Answer::absent();
        };
        if from > local.len {
            return Answer::absent();
        }
        // A resume must land on the SAME file: `expect` names it. Without a tag
        // (hashing has not run yet) a resume cannot be honoured — a wrong file is
        // worse than a restart — but a fetch from 0 can.
        if let Some(expect) = expect {
            let matches = local.tag.as_ref().is_some_and(|tag| {
                tag.len == expect.len && tag.sha256.eq_ignore_ascii_case(&expect.sha256)
            });
            if !matches && (from > 0 || local.tag.is_some()) {
                return Answer::absent();
            }
        }

        let mut file = match tokio::fs::File::open(&local.path).await {
            Ok(file) => file,
            // The play cache evicts and a download can be removed between the
            // inventory and the fetch: `NotFound`, never an error (the pitfall the
            // note names).
            Err(error) => {
                debug!(%error, "a listed file vanished between the index and its open");
                return Answer::absent();
            }
        };
        if let Err(error) = file.seek(SeekFrom::Start(from)).await {
            debug!(%error, "a served file could not be seeked");
            return Answer::absent();
        }

        // A background transfer waits for the person: this is the one await that
        // can be long, and the slot above is already counted, so a second `Bulk`
        // request is answered `Busy` instead of piling up.
        let lane = match urgency {
            Urgency::Bulk => Some(self.gates.acquire(nz_api::lanes::Lane::Bulk).await),
            Urgency::Play | Urgency::Prefetch => None,
        };
        let body = ServedBody {
            file,
            remaining: local.len - from,
            gates: matches!(urgency, Urgency::Bulk).then(|| Arc::clone(&self.gates)),
            wait: None,
        };
        Answer {
            head: proto::Response::FetchOk {
                len: local.len,
                tag: local.tag.clone(),
                // What is SERVED, which is the asked-for quality unless this node
                // had to fall back: the requester records this
                // (`ByteStream::quality`, LN5) and must not be told the wrong one.
                quality: proto::WireQuality::from(local.quality),
            },
            body: Some(Box::new(body)),
            slot: Some(slot),
            lane,
        }
    }
}

/// The bytes of a served file: 64 KiB reads, and — for a `Bulk` serve only — the
/// same token bucket the server's own downloads take, so a playlist pulled from
/// this device yields to the track the person is listening to (rule 1).
struct ServedBody {
    file: tokio::fs::File,
    remaining: u64,
    /// `Some` for a `Bulk` serve, `None` for a `Play`/`Prefetch` one: a tapped
    /// track is never paced.
    gates: Option<Arc<nz_api::lanes::Gates>>,
    /// The throttle future a chunk is waiting on, so a `Poll::Pending` is never
    /// forgotten.
    wait: Option<Pin<Box<dyn std::future::Future<Output = ()> + Send>>>,
}

impl AsyncRead for ServedBody {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if this.remaining == 0 || buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        if let Some(mut wait) = this.wait.take() {
            match wait.as_mut().poll(cx) {
                Poll::Pending => {
                    this.wait = Some(wait);
                    return Poll::Pending;
                }
                Poll::Ready(()) => {}
            }
        }
        let want = buf
            .remaining()
            .min(CHUNK_BYTES)
            .min(usize::try_from(this.remaining).unwrap_or(usize::MAX));
        if let Some(gates) = &this.gates {
            let gates = Arc::clone(gates);
            let mut wait: Pin<Box<dyn std::future::Future<Output = ()> + Send>> =
                Box::pin(async move { gates.throttle(want as u64).await });
            match wait.as_mut().poll(cx) {
                Poll::Pending => {
                    this.wait = Some(wait);
                    return Poll::Pending;
                }
                Poll::Ready(()) => {}
            }
        }
        let read = {
            let unfilled = buf.initialize_unfilled();
            let want = want.min(unfilled.len());
            let mut scratch = ReadBuf::new(&mut unfilled[..want]);
            match Pin::new(&mut this.file).poll_read(cx, &mut scratch) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(())) => scratch.filled().len(),
            }
        };
        buf.advance(read);
        this.remaining -= read as u64;
        Poll::Ready(Ok(()))
    }
}

/// The MIME type of a cover file, from the extension `nz-cache` sniffed from the
/// bytes when it stored it.
fn mime_of(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The note's serve budget: 2 Play, 1 Prefetch, 1 Bulk. A bigger number here
    /// is a device that serves four peers and stutters its own playback.
    #[test]
    fn the_default_serve_limits_are_two_one_one() {
        let limits = ServeLimits::default();
        assert_eq!((limits.play, limits.prefetch, limits.bulk), (2, 1, 1));
    }

    /// The budget is per urgency and it is released by dropping the slot: three
    /// Play at once is two served and one refused, and a released slot admits the
    /// next request.
    #[test]
    fn the_third_play_is_refused_and_a_released_slot_admits_the_next() {
        let limits = ServeLimits::default();
        let counts = Counts::default();
        let first = counts.take(Urgency::Play, &limits).expect("first");
        let second = counts.take(Urgency::Play, &limits).expect("second");
        assert!(counts.take(Urgency::Play, &limits).is_none(), "the third");
        assert_eq!(counts.read(), (2, 0, 0));
        drop(first);
        let replacement = counts.take(Urgency::Play, &limits).expect("the freed slot");
        assert_eq!(counts.read(), (2, 0, 0));
        drop(replacement);
        drop(second);
    }

    /// A refusal is one frame, whatever the reason: `absent()` is the only
    /// constructor, and its bytes are the wire's `NotFound`.
    #[test]
    fn every_refusal_is_the_same_frame() {
        let bytes = serde_json::to_string(&Answer::absent().head).expect("json");
        assert_eq!(bytes, r#"{"t":"NotFound"}"#);
        assert!(!Answer::absent().has_body());
    }

    /// The MIME types the four formats `nz-cache` writes get.
    #[test]
    fn a_cover_says_what_it_is() {
        use std::path::Path;
        assert_eq!(mime_of(Path::new("a.jpg")), "image/jpeg");
        assert_eq!(mime_of(Path::new("a.png")), "image/png");
        assert_eq!(mime_of(Path::new("a.gif")), "image/gif");
        assert_eq!(mime_of(Path::new("a.webp")), "image/webp");
        assert_eq!(mime_of(Path::new("a.img")), "application/octet-stream");
        assert_eq!(mime_of(Path::new("a")), "application/octet-stream");
    }
}
