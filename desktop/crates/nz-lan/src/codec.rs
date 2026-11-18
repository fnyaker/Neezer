//! Length-prefixed frames on a QUIC stream, and the caps that make a hostile peer
//! harmless (docs/agents/lan-nodes.md, *Protocol messages*).
//!
//! **LN0 declares the cap; LN6 writes the reader and the writer.** A frame is a
//! `u32` big-endian length followed by that many bytes of UTF-8 JSON. The cap is
//! checked BEFORE the buffer is allocated, which is the whole point: a peer that
//! announces four gigabytes must cost four bytes of read, not four gigabytes of
//! memory (T9).

/// The largest frame either side accepts: 1 MiB.
///
/// A full inventory of 8 000 tracks is about 320 KB of JSON, so a legitimate
/// frame fits with room to spare; two of them never travel together, because
/// `Inventory` and `Sets` paginate instead.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

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
}
