//! Fetching a file or a stream from a peer, with resume, length and SHA-256
//! checks and a decoder probe (docs/agents/lan-nodes.md, *Integrity, resume*).
//!
//! **LN9 filled it.** What is already fixed, because the callers are:
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
//!
//! # Why the transfer speaks the frames itself
//!
//! [`PeerConn::request`] answers a [`crate::transport::Body`] that caps its reads at
//! the head's `len` — the FULL length. On a resume from `from` the body is
//! `len - from` bytes, so that cap is `from` bytes too high, and a peer that sends
//! MORE than `len` cannot be told from a peer that ended cleanly: both look like "the
//! cap was not reached". This module therefore opens the stream with
//! [`PeerConn::connection`] and reads exactly `len - from` bytes, then probes one
//! byte to tell a clean FIN from an over-send. It is the SAME frames and the SAME
//! [`crate::codec`] — no second protocol, only an exact read.
//!
//! # What a cancel is
//!
//! `stop` is the caller's own signal (a download queue told to pause, the app
//! closing). It is reported as [`LanError::Internal`] with the cause « transfert
//! interrompu »: no [`LanError`] variant means "the person asked" — the enum is
//! LN0's and this lot does not fork it — and the downloader that owns the queue
//! recognises that cause and records a pause, never a failure.

use std::io::SeekFrom;
use std::path::Path;
use std::sync::Arc;

use aws_lc_rs::digest::{Context, SHA256};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::watch;
use tracing::debug;

use crate::codec;
use crate::proto;
use crate::transport::PeerConn;
use crate::types::{LanError, Urgency};

/// How much one read of a transfer takes: the same 64 KiB a serve reads, so neither
/// side holds more than a fixed amount of a 200 MB file.
pub const CHUNK_BYTES: usize = 64 * 1024;

/// The cause [`fetch_to_file`] answers when the caller's `stop` fired.
pub const INTERRUPTED: &str = "transfert interrompu";

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

/// The head of a `Fetch` answer, in the crate's own types.
struct Head {
    len: u64,
    tag: Option<proto::FileTag>,
    quality: nz_api::Quality,
}

/// Fetches one file into the reserved `.part` file, resuming at `plan.from`,
/// reporting progress as `(bytes so far, total)`.
///
/// `stop` is the caller's cancel (a download queue told to pause, the app
/// closing): it is awaited between chunks, never after the fact.
///
/// The SHA-256 is over the WHOLE file, not only the bytes this call received: on a
/// resume the first `from` bytes are read back through the hasher, so the tag the
/// caller compares against the peer's is the one of the file that will be
/// published.
pub async fn fetch_to_file(
    plan: FetchPlan,
    part: &Path,
    gates: &nz_api::lanes::Gates,
    stop: watch::Receiver<bool>,
    progress: &mut (dyn FnMut(u64, u64) + Send),
) -> Result<Fetched, LanError> {
    let bulk = plan.urgency == Urgency::Bulk;
    // Rule 1: ONE Bulk lane across the server and the LAN, held for the whole
    // transfer. `Play` and `Prefetch` take none, so a tapped track never waits
    // behind a download — on either side of the connection.
    let _lane = if bulk {
        Some(gates.acquire(nz_api::lanes::Lane::Bulk).await)
    } else {
        None
    };
    if stopped(&stop) {
        return Err(LanError::Internal(INTERRUPTED.to_owned()));
    }

    let (head, mut recv) = open_fetch(&plan).await?;
    if plan.from > head.len {
        return Err(LanError::BadFile(format!(
            "reprise à {} alors que le fichier fait {} octets",
            plan.from, head.len
        )));
    }
    // A resume must land on the SAME file. The responder refuses a different one
    // with `NotFound`; a peer that answers `FetchOk` anyway is serving something
    // else than what was asked, which is a bad file, not a resume.
    if plan.from > 0
        && let Some(expect) = plan.expect.as_ref()
        && !head.tag.as_ref().is_some_and(|tag| {
            tag.len == expect.len && tag.sha256.eq_ignore_ascii_case(&expect.sha256)
        })
    {
        return Err(LanError::BadFile(
            "le pair a servi un autre fichier que celui attendu".to_owned(),
        ));
    }

    let total = head.len - plan.from;
    // Before a byte is written: the peer announced this length, and a hostile one
    // announces a wrong one (T9).
    check_free_space(part, total)?;

    let mut file = tokio::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        // `truncate(false)`: the two cases below set the length themselves, and a
        // resume must not lose the bytes it is resuming from.
        .truncate(false)
        .open(part)
        .await
        .map_err(|error| disk(&error))?;
    let existing = file.metadata().await.map_err(|error| disk(&error))?.len();
    if plan.from == 0 {
        file.set_len(0).await.map_err(|error| disk(&error))?;
    } else {
        if existing < plan.from {
            return Err(LanError::Disk(format!(
                "le fichier partiel fait {existing} octets, la reprise en demande {}",
                plan.from
            )));
        }
        // Bytes past the checkpoint are dropped: the caller's offset is the truth,
        // and a file with a stale tail would hash to something nobody announced.
        file.set_len(plan.from)
            .await
            .map_err(|error| disk(&error))?;
    }

    let mut hasher = Context::new(&SHA256);
    if plan.from > 0 {
        // The bytes already on disk go through the hasher too: the tag is of the
        // complete file, so a resume that skipped them would compare a hash of half
        // a file with a hash of all of it.
        file.seek(SeekFrom::Start(0))
            .await
            .map_err(|error| disk(&error))?;
        let mut buffer = vec![0u8; CHUNK_BYTES];
        let mut hashed = 0u64;
        while hashed < plan.from {
            let want = usize::try_from((plan.from - hashed).min(CHUNK_BYTES as u64))
                .expect("a chunk fits a usize");
            let read = file
                .read(&mut buffer[..want])
                .await
                .map_err(|error| disk(&error))?;
            if read == 0 {
                return Err(LanError::Disk(
                    "le fichier partiel est plus court que la reprise demandée".to_owned(),
                ));
            }
            hasher.update(&buffer[..read]);
            hashed += read as u64;
        }
        file.seek(SeekFrom::Start(plan.from))
            .await
            .map_err(|error| disk(&error))?;
    }
    progress(plan.from, head.len);

    // The transfer itself, in a block whose outcome is flushed before it is
    // returned: tokio's `File` BUFFERS writes (up to 2 MiB) and hands them to a
    // blocking task, so a handle dropped on an error path would lose the bytes the
    // caller is about to resume from. A cancelled or refused transfer therefore
    // still leaves exactly what the progress callback last reported on disk.
    let outcome = async {
        let mut buffer = vec![0u8; CHUNK_BYTES];
        let mut received = 0u64;
        while received < total {
            if stopped(&stop) {
                return Err(LanError::Internal(INTERRUPTED.to_owned()));
            }
            let want = usize::try_from((total - received).min(CHUNK_BYTES as u64))
                .expect("a chunk fits a usize");
            if bulk {
                // The same token bucket the server's own downloads take, so a
                // playlist pulled from a peer yields to the track the person is
                // listening to.
                gates.throttle(want as u64).await;
            }
            recv.read_exact(&mut buffer[..want])
                .await
                .map_err(|error| {
                    debug!(%error, "a peer's body ended before its announced length");
                    LanError::BadFile("le flux s'est arrêté avant la longueur annoncée".to_owned())
                })?;
            file.write_all(&buffer[..want])
                .await
                .map_err(|error| disk(&error))?;
            hasher.update(&buffer[..want]);
            received += want as u64;
            progress(plan.from + received, head.len);
        }

        // Exactly `len - from` bytes arrived. One more read tells a clean FIN from
        // a peer that kept sending: the extra bytes are never written, and a peer
        // that sent them is refused (its file is not the one it announced).
        let mut extra = [0u8; 1];
        match recv.read(&mut extra).await {
            Ok(Some(read)) if read > 0 => {
                return Err(LanError::BadFile(
                    "le pair a envoyé plus que la longueur annoncée".to_owned(),
                ));
            }
            Ok(_) => {}
            // The stream was reset right after the last announced byte: everything
            // the peer promised is here, and the hash below still decides.
            Err(error) => debug!(%error, "a peer's body stream ended without a clean FIN"),
        }
        Ok(())
    }
    .await;
    file.flush().await.map_err(|error| disk(&error))?;
    outcome?;

    let sha256 = hex(hasher.finish().as_ref());
    if let Some(tag) = head.tag.as_ref()
        && (tag.len != head.len || !tag.sha256.eq_ignore_ascii_case(&sha256))
    {
        return Err(LanError::BadFile(format!(
            "empreinte SHA-256 {sha256} au lieu de {} annoncée",
            tag.sha256
        )));
    }
    Ok(Fetched {
        len: head.len,
        sha256,
        quality: head.quality,
    })
}

/// A peer's file as a live stream, for playback: byte-seekable by construction
/// because a peer file is always complete.
pub struct PeerStream {
    recv: quinn::RecvStream,
    /// The file's total length, from the head.
    len: u64,
    /// The offset this stream started at (`from_byte` on a seek).
    from: u64,
    /// How many bytes this stream has handed out.
    received: u64,
    quality: nz_api::Quality,
    gates: Arc<nz_api::lanes::Gates>,
    urgency: Urgency,
}

impl std::fmt::Debug for PeerStream {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PeerStream")
            .field("len", &self.len)
            .field("from", &self.from)
            .field("received", &self.received)
            .field("quality", &self.quality)
            .field("urgency", &self.urgency)
            .finish()
    }
}

impl PeerStream {
    /// Opens `Fetch` for streaming, taking the gates when the urgency is `Bulk`.
    ///
    /// A playback stream is `Play` or `Prefetch` and takes no lane at all: it must
    /// never wait behind a download. A `Bulk` stream (a download through the
    /// opener) takes the lane around the open and re-takes it for each chunk, so a
    /// request the person makes interleaves instead of waiting for the whole file.
    pub async fn open(
        plan: FetchPlan,
        gates: Arc<nz_api::lanes::Gates>,
    ) -> Result<PeerStream, LanError> {
        let bulk = plan.urgency == Urgency::Bulk;
        let (head, recv) = {
            let _lane = if bulk {
                Some(gates.acquire(nz_api::lanes::Lane::Bulk).await)
            } else {
                None
            };
            open_fetch(&plan).await?
        };
        if plan.from > head.len {
            return Err(LanError::BadFile(format!(
                "flux demandé à {} alors que le fichier fait {} octets",
                plan.from, head.len
            )));
        }
        Ok(PeerStream {
            recv,
            len: head.len,
            from: plan.from,
            received: 0,
            quality: head.quality,
            gates,
            urgency: plan.urgency,
        })
    }

    /// The file's total length: what `ByteStream::len_hint` answers, without a
    /// request.
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Whether this stream is empty — never true for a fetched file (a zero-byte
    /// audio file is not a file), and here so `len` has its pair.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The quality the peer actually serves — LN5's `ByteStream::quality()` reads
    /// this, so the spool is published into the play cache under the right key.
    pub fn quality(&self) -> nz_api::Quality {
        self.quality
    }

    /// The next chunk, `None` at the end.
    pub async fn chunk(&mut self) -> Result<Option<Vec<u8>>, LanError> {
        let remaining = self.len.saturating_sub(self.from + self.received);
        if remaining == 0 {
            return Ok(None);
        }
        let want =
            usize::try_from(remaining.min(CHUNK_BYTES as u64)).expect("a chunk fits a usize");
        let bulk = self.urgency == Urgency::Bulk;
        // A `Bulk` stream re-takes the lane for each chunk, so a request the person
        // makes interleaves instead of waiting for the whole file; a `Play`/`Prefetch`
        // one takes nothing at all and is never paced.
        let _lane = if bulk {
            Some(self.gates.acquire(nz_api::lanes::Lane::Bulk).await)
        } else {
            None
        };
        if bulk {
            self.gates.throttle(want as u64).await;
        }
        let mut chunk = vec![0u8; want];
        self.recv.read_exact(&mut chunk).await.map_err(|error| {
            debug!(%error, "a peer's stream ended before its announced length");
            LanError::PeerGone
        })?;
        self.received += want as u64;
        Ok(Some(chunk))
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
    use symphonia::core::codecs::audio::well_known::{CODEC_ID_FLAC, CODEC_ID_OPUS};
    use symphonia::core::formats::probe::{Hint, Probe};
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let file = std::fs::File::open(path)
        .map_err(|error| LanError::BadFile(format!("ouverture impossible : {error}")))?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut prober = Probe::new();
    symphonia::default::register_enabled_formats(&mut prober);
    let format = prober
        .probe(
            &Hint::new(),
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|error| LanError::BadFile(format!("ce n'est pas de l'audio : {error}")))?;
    let track = format
        .default_track(TrackType::Audio)
        .or_else(|| format.first_track(TrackType::Audio))
        .ok_or_else(|| LanError::BadFile("aucune piste audio".to_owned()))?;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|params| params.audio())
        .ok_or_else(|| LanError::BadFile("codec audio inconnu".to_owned()))?;
    // A peer serves the archive's FLAC or the server's Opus, and the head names
    // which: a file that is neither is not the file that was announced.
    let expected = match quality {
        nz_api::Quality::Flac => CODEC_ID_FLAC,
        nz_api::Quality::Opus64
        | nz_api::Quality::Opus128
        | nz_api::Quality::Opus192
        | nz_api::Quality::Opus256
        | nz_api::Quality::Opus320 => CODEC_ID_OPUS,
    };
    if params.codec != expected {
        return Err(LanError::BadFile(format!(
            "codec {:?} au lieu de {expected:?} pour {quality:?}",
            params.codec
        )));
    }
    if let Some(expected_s) = duration_s {
        let info = format.media_info();
        let actual = info
            .time_base
            .zip(info.duration)
            .and_then(|(base, duration)| base.calc_duration(duration))
            .map(|time| time.as_secs_f64());
        // A container that states no duration cannot be checked, and inventing one
        // would refuse every legitimate raw stream: the codec check stands alone.
        if let Some(actual) = actual
            && (actual - f64::from(expected_s)).abs() > 2.0
        {
            return Err(LanError::BadFile(format!(
                "{actual:.1} s de musique au lieu de {expected_s} s annoncées"
            )));
        }
    }
    Ok(())
}

/// Opens the `Fetch` stream and reads its head: the same request frame
/// [`PeerConn::request`] writes, read exactly (see the module's note).
async fn open_fetch(plan: &FetchPlan) -> Result<(Head, quinn::RecvStream), LanError> {
    let (mut send, mut recv) = plan.peer.connection().open_bi().await.map_err(|error| {
        debug!(%error, "a LAN fetch could not open its stream");
        LanError::PeerGone
    })?;
    let request = proto::Request::Fetch {
        track: plan.track.clone(),
        quality: proto::WireQuality::from(plan.quality),
        from: plan.from,
        expect: plan.expect.clone(),
        urgency: plan.urgency,
    };
    codec::write_frame(&mut send, &request).await?;
    send.finish().map_err(|error| {
        debug!(%error, "a LAN fetch stream closed before its frame was sent");
        LanError::PeerGone
    })?;
    let head = match codec::read_frame::<proto::Response>(&mut recv, codec::MAX_FRAME_BYTES).await?
    {
        Some(head) => head,
        None => return Err(LanError::PeerGone),
    };
    match head {
        proto::Response::FetchOk { len, tag, quality } => Ok((
            Head {
                len,
                tag,
                quality: quality.into(),
            },
            recv,
        )),
        // Absent, out of scope, a resume onto another file, an offset past the end:
        // ONE answer from the responder, and one verdict here.
        proto::Response::NotFound => Err(LanError::NotFound),
        proto::Response::Busy { retry_in_ms } => Err(LanError::Busy { retry_in_ms }),
        proto::Response::Error { code } => {
            Err(LanError::Proto(format!("le pair a répondu « {code} »")))
        }
        other => Err(LanError::Proto(format!(
            "une réponse inattendue à un fichier : {other:?}"
        ))),
    }
}

/// Whether the caller's cancel fired.
fn stopped(stop: &watch::Receiver<bool>) -> bool {
    *stop.borrow()
}

/// The free space of `part`'s filesystem, checked BEFORE a byte is written: a peer
/// announces a length, and a hostile one announces a wrong one (T9). Refused with
/// « espace disque insuffisant » when the file would not fit.
fn check_free_space(part: &Path, needed: u64) -> Result<(), LanError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    // `statvfs` wants a path that exists: the `.part` may not yet, and its directory
    // is on the same filesystem (the store reserved it there).
    let probe = if part.exists() {
        part.to_path_buf()
    } else {
        part.parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    };
    let path = CString::new(probe.as_os_str().as_bytes())
        .map_err(|_| LanError::Disk("chemin de destination invalide".to_owned()))?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `path` is a valid NUL-terminated C string and `stat` a valid out
    // pointer for the duration of the call; `statvfs` writes only into it.
    if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
        let error = std::io::Error::last_os_error();
        return Err(LanError::Disk(format!(
            "espace libre illisible pour {} : {error}",
            probe.display()
        )));
    }
    // `c_ulong` is 64 bits on the desktop and 32 on Android: cast, do not convert.
    let free = (stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64);
    if free < needed {
        return Err(LanError::Disk(format!(
            "il manque {} octets libres",
            needed - free
        )));
    }
    Ok(())
}

/// A local I/O failure, in the words a screen shows.
fn disk(error: &std::io::Error) -> LanError {
    LanError::Disk(error.to_string())
}

/// Lowercase hex, which is the spelling `FileTag::sha256` carries.
fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A path that does not exist is not audio: the probe says so with a cause, and
    /// never with `NotFound` (which is a verdict about a track, not about bytes).
    #[test]
    fn a_missing_file_is_a_bad_file_and_never_a_verdict_about_a_track() {
        let error =
            probe(Path::new("/does/not/exist"), nz_api::Quality::Flac, None).expect_err("no file");
        assert!(matches!(error, LanError::BadFile(_)), "{error:?}");
    }

    /// The chunk size is the serve's own 64 KiB: a transfer never holds more than
    /// one chunk of a 200 MB file, on either side.
    #[test]
    fn a_chunk_is_sixty_four_kibibytes() {
        assert_eq!(CHUNK_BYTES, 65_536);
        assert_eq!(CHUNK_BYTES, crate::serve::CHUNK_BYTES);
    }

    /// The hex spelling is the one a `FileTag` carries: lowercase, two characters a
    /// byte, 64 characters for a SHA-256.
    #[test]
    fn a_hash_is_lowercase_hex() {
        assert_eq!(hex(&[0x00, 0x0f, 0xff]), "000fff");
        let digest = aws_lc_rs::digest::digest(&SHA256, b"neezer");
        assert_eq!(hex(digest.as_ref()).len(), 64);
        assert_eq!(hex(digest.as_ref()), hex(digest.as_ref()).to_lowercase());
    }

    /// The free-space check reads the target's own filesystem, and a file of a few
    /// bytes fits in it: the refusal path is exercised by `tests/fetch.rs`, which
    /// asks for more than the disk has.
    #[test]
    fn the_free_space_of_the_target_is_readable() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        check_free_space(&root.join("does-not-exist-yet.part"), 1_024).expect("a kilobyte fits");
    }
}
