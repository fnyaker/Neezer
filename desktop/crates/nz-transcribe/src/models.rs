//! The model ladder and what makes a file a model — owned by lot T2.
//!
//! Nothing here downloads anything: `engine.py` never downloads either, and the
//! note is explicit that a model arrives only from a click in Réglages →
//! Transcription locale, then goes through `Lane::Bulk` and a pinned SHA-256.
//! What T1 pins is the ladder and the cheap validation the server already runs,
//! so the settings screen can name a size and the worker can recognise the files
//! a `docker/transcribe` deployment already has on disk.

/// Best model, for a machine with memory to spare (`engine.MODEL_TURBO_Q8`).
pub const TURBO_Q8: &str = "ggml-large-v3-turbo-q8_0.bin";

/// Same model, smaller weights (`engine.MODEL_TURBO_Q5`).
pub const TURBO_Q5: &str = "ggml-large-v3-turbo-q5_0.bin";

/// The small model, the last rung on a 1 GB box (`engine.MODEL_SMALL`).
pub const SMALL: &str = "ggml-small.bin";

/// The ladder when at least [`TURBO_FREE_BYTES`] is free (`engine.MODEL_ORDER_FREE`).
pub const ORDER_FREE: [&str; 3] = [TURBO_Q8, TURBO_Q5, SMALL];

/// The ladder below that (`engine.MODEL_ORDER_TIGHT`): the turbo model in q8 does
/// not fit, so it is not even attempted.
pub const ORDER_TIGHT: [&str; 2] = [TURBO_Q5, SMALL];

/// Free memory above which the q8 turbo model is used (`engine.TURBO_FREE_BYTES`,
/// read from `MemAvailable`).
pub const TURBO_FREE_BYTES: u64 = 2_500_000_000;

/// Below this, a file is not a model (`engine.MIN_MODEL_BYTES`).
pub const MIN_MODEL_BYTES: u64 = 1 << 20;

/// The magic a GGML file starts with (`engine.MODEL_MAGICS`): old and new
/// little-endian writes of "ggml".
pub const MAGICS: [&[u8]; 2] = [b"ggml", b"lmgg"];

#[cfg(test)]
mod tests {
    use super::{
        MAGICS, MIN_MODEL_BYTES, ORDER_FREE, ORDER_TIGHT, SMALL, TURBO_FREE_BYTES, TURBO_Q5,
        TURBO_Q8,
    };

    /// The ladder is the server's: a tighter ladder is a prefix of the free one,
    /// so a machine that cannot fit the q8 still picks the best file it has.
    #[test]
    fn the_ladder_matches_the_servers() {
        assert_eq!(ORDER_FREE.len(), 3);
        assert_eq!(ORDER_TIGHT, [TURBO_Q5, SMALL]);
        assert_eq!(&ORDER_FREE[1..], ORDER_TIGHT);
        assert!(MAGICS.contains(&b"ggml".as_slice()));
        assert!(ORDER_FREE.iter().all(|name| name.ends_with(".bin")));
        assert_eq!(TURBO_Q8, "ggml-large-v3-turbo-q8_0.bin");
    }

    /// The two thresholds are sizes, and both have to be plausible: a 1 MiB floor
    /// and 2.5 GB of free memory for the biggest model.
    #[test]
    fn the_thresholds_are_sizes() {
        assert_eq!(MIN_MODEL_BYTES, 1_048_576);
        assert_eq!(TURBO_FREE_BYTES, 2_500_000_000);
        const { assert!(TURBO_FREE_BYTES > MIN_MODEL_BYTES) };
    }
}
