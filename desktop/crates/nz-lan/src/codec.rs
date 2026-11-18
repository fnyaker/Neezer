//! Length-prefixed frames on a QUIC stream, and the caps that make a hostile peer
//! harmless (docs/agents/lan-nodes.md, *Protocol messages*).
//!
//! **LN0 declared the cap; LN6 wrote the reader and the writer.** A frame is a
//! `u32` big-endian length followed by that many bytes of UTF-8 JSON. The cap is
//! checked BEFORE the buffer is allocated, which is the whole point: a peer that
//! announces four gigabytes must cost four bytes of read, not four gigabytes of
//! memory (T9).
//!
//! # Why the length comes first, and why nothing is read before it is checked
//!
//! [`read_frame`] reads exactly four bytes, compares the announced length with the
//! caller's cap (never more than [`MAX_FRAME_BYTES`]) and only then allocates the
//! body. A one-mebibyte-plus-one frame is therefore refused having allocated
//! nothing at all, and a `u32` length of `0xFFFF_FFFF` costs the same four bytes
//! (measured in `tests/transport.rs`, with a counting allocator).
//!
//! # A clean end of stream is not an error
//!
//! The control stream ends when the peer closes it: a reader that has read zero
//! bytes of a header answers `Ok(None)`, so `next_control()` can say "the stream is
//! over" instead of inventing a transport error. A header that is cut short IS an
//! error: the peer sent something and it is not a frame.

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::types::LanError;

/// The largest frame either side accepts: 1 MiB.
///
/// A full inventory of 8 000 tracks is about 320 KB of JSON, so a legitimate
/// frame fits with room to spare; two of them never travel together, because
/// `Inventory` and `Sets` paginate instead.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// The `u32` big-endian length that precedes every frame's JSON.
pub const FRAME_HEADER_BYTES: usize = 4;

/// The largest chunk [`crate::transport::Body::chunk`] hands back, whatever a
/// caller asks for: a request for a hundred megabytes is a bug, not a hint.
pub const MAX_CHUNK_BYTES: usize = 64 * 1024;

/// Writes one frame: the `u32` big-endian length, then the JSON.
///
/// Answers the number of bytes written (header included), which is what the
/// "control bytes" measure counts. A frame over [`MAX_FRAME_BYTES`] is refused
/// here too: nothing this crate sends may need a reader to allocate more than a
/// mebibyte, and `Inventory`/`Sets` paginate for exactly that reason.
pub async fn write_frame<T>(send: &mut quinn::SendStream, frame: &T) -> Result<usize, LanError>
where
    T: Serialize + ?Sized,
{
    let body = serde_json::to_vec(frame)
        .map_err(|error| LanError::Internal(format!("trame non encodable : {error}")))?;
    if body.len() > MAX_FRAME_BYTES {
        return Err(LanError::Proto(format!(
            "trame de {} octets au-delà du plafond de {MAX_FRAME_BYTES}",
            body.len()
        )));
    }
    let header = u32::try_from(body.len())
        .map_err(|_| LanError::Internal("longueur de trame hors u32".to_owned()))?
        .to_be_bytes();
    send.write_all(&header).await.map_err(stream_gone)?;
    send.write_all(&body).await.map_err(stream_gone)?;
    Ok(FRAME_HEADER_BYTES + body.len())
}

/// The most bytes a frame's JSON body may take, after the four-byte header has been
/// read and before anything is allocated for it.
///
/// Pure, so the two properties that make a hostile frame harmless are testable
/// without a socket: the announced length is checked against the cap, and the check
/// itself allocates nothing at all (a `u32` can say 4 GiB; this answers in four
/// bytes of arithmetic).
pub fn frame_body_len(header: [u8; FRAME_HEADER_BYTES], cap: usize) -> Result<usize, LanError> {
    let cap = cap.min(MAX_FRAME_BYTES);
    let len = u32::from_be_bytes(header) as usize;
    if len > cap {
        return Err(LanError::Proto(format!(
            "trame de {len} octets au-delà du plafond de {cap}"
        )));
    }
    Ok(len)
}

/// Reads one frame, refusing anything over `cap` **before** allocating it, and
/// answering `None` when the peer ended the stream cleanly.
///
/// `cap` counts the JSON body, never the header; it is clamped to
/// [`MAX_FRAME_BYTES`] so no caller can raise the crate's own ceiling by accident.
pub async fn read_frame<T>(recv: &mut quinn::RecvStream, cap: usize) -> Result<Option<T>, LanError>
where
    T: DeserializeOwned,
{
    let Some(body) = read_frame_bytes(recv, cap).await? else {
        return Ok(None);
    };
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|error| LanError::Proto(format!("trame illisible : {error}")))
}

/// The bytes of one frame, for a caller that wants to see the tag itself (the
/// responder answers `Error { code: "unknown" }` for a tag it does not know).
pub async fn read_frame_bytes(
    recv: &mut quinn::RecvStream,
    cap: usize,
) -> Result<Option<Vec<u8>>, LanError> {
    let mut header = [0u8; FRAME_HEADER_BYTES];
    match recv.read_exact(&mut header).await {
        Ok(()) => {}
        // The peer finished the stream before sending a byte of header: a clean
        // end, which is how a control stream closes.
        Err(quinn::ReadExactError::FinishedEarly(0)) => return Ok(None),
        Err(error) => return Err(stream_gone(error)),
    }
    // The decision, and only then the allocation: this one line is the whole of T9's
    // "every size from the wire is capped before allocation".
    let len = frame_body_len(header, cap)?;
    let mut body = vec![0u8; len];
    recv.read_exact(&mut body).await.map_err(stream_gone)?;
    Ok(Some(body))
}

/// One cause for every way a stream ends under the reader: the peer closed it, the
/// connection went away, or the frame is truncated. A peer that hangs up is
/// [`LanError::PeerGone`]; nothing finer is needed, and nothing finer is told to
/// the peer — the precise cause stays in this device's own log.
fn stream_gone<E: std::fmt::Display>(error: E) -> LanError {
    tracing::debug!(%error, "a LAN stream ended under its reader");
    LanError::PeerGone
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cap is one mebibyte, not one megabyte, and it is what both the reader
    /// and the writer check: a `u32` length field can say 4 GiB and must still be
    /// refused at this number.
    #[test]
    fn the_frame_cap_is_one_mebibyte_and_below_a_u32_length() {
        assert_eq!(MAX_FRAME_BYTES, 1_048_576);
        assert!(u32::try_from(MAX_FRAME_BYTES).is_ok());
        assert!((MAX_FRAME_BYTES as u64) < u64::from(u32::MAX));
    }

    /// The header is four bytes and a chunk is 64 KiB: the two numbers the
    /// allocation bounds are computed from, held where a reader of the code sees
    /// them together.
    #[test]
    fn the_header_and_the_chunk_are_the_documented_sizes() {
        assert_eq!(FRAME_HEADER_BYTES, 4);
        assert_eq!(MAX_CHUNK_BYTES, 65_536);
        const { assert!(MAX_CHUNK_BYTES < MAX_FRAME_BYTES) };
    }
}
