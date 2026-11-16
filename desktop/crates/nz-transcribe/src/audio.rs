//! Decoding an episode to 16 kHz mono in this process (symphonia, the libopus
//! adapter, rubato) — owned by lot T2.
//!
//! The desktop has no ffmpeg dependency: the server shells out to `ffmpeg` for a
//! chunk (`engine.py#extract_chunk`), this crate decodes in-process instead. T1
//! pins the one thing the `engine` child's file contract is built on, because it
//! crosses a process boundary: a `--pcm` file is RAW little-endian `f32`, one
//! channel, 16 kHz, no header. Its duration is therefore a multiplication, which
//! is how the supervisor knows how far a chunk got without parsing audio.

use crate::segments::SAMPLE_RATE;

/// Bytes one sample of a `--pcm` file takes: an `f32`.
pub const SAMPLE_BYTES: u64 = 4;

/// Bytes of a `--pcm` file per millisecond: 4 bytes × 16 000 samples/s ÷ 1 000.
pub const BYTES_PER_MS: u64 = SAMPLE_BYTES * SAMPLE_RATE as u64 / 1000;

/// Duration of a raw `--pcm` file in whole milliseconds, rounded down.
///
/// The caller that resumes a job compares this to the chunk it asked for, so a
/// short read (a killed child, a truncated file) is visible instead of silent.
pub fn pcm_duration_ms(bytes: u64) -> u64 {
    bytes / BYTES_PER_MS
}

#[cfg(test)]
mod tests {
    use super::{BYTES_PER_MS, pcm_duration_ms};
    use crate::segments::{CHUNK_SECONDS, SAMPLE_RATE};

    /// A chunk of ten minutes is exactly `CHUNK_SECONDS` of this file, and the
    /// conversion is a division, not a decode.
    #[test]
    fn a_chunk_of_pcm_is_a_multiplication() {
        assert_eq!(SAMPLE_RATE, 16_000);
        assert_eq!(BYTES_PER_MS, 64);
        assert_eq!(pcm_duration_ms(64), 1);
        assert_eq!(pcm_duration_ms(0), 0);
        assert_eq!(
            pcm_duration_ms(BYTES_PER_MS * CHUNK_SECONDS as u64 * 1000),
            (CHUNK_SECONDS * 1000) as u64
        );
        // Half a millisecond of samples is not a millisecond.
        assert_eq!(pcm_duration_ms(95), 1);
    }
}
