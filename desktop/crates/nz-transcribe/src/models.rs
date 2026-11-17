//! The model ladder, what makes a file a model, and the checksums a download is
//! held to — owned by lot T2.
//!
//! Nothing here downloads anything: `engine.py` never downloads either, and the
//! note is explicit that a model arrives only from a click in Réglages →
//! Transcription locale, then goes through `Lane::Bulk` and a pinned SHA-256.
//! What this module holds is everything such a download and the settings screen
//! need to agree on with the engine:
//!
//! - the ladder and its two thresholds, copied from `engine.py` so a machine
//!   with 1 GB of memory picks `ggml-small.bin` without anyone editing a file;
//! - the cheap validation the server already runs ([`validate`]: GGML magic,
//!   ≥ 1 MiB), so a truncated download is refused before whisper.cpp sees it;
//! - the Silero VAD model's name, which is part of the ladder's directory;
//! - the size and the SHA-256 of every file, pinned here so the download (T4)
//!   and CI cannot disagree about what a file is supposed to be.
//!
//! The hashes are the ones Hugging Face publishes for `ggerganov/whisper.cpp`
//! and `ggml-org/whisper-vad` (`lfs.oid`), re-checked against the files this lot
//! downloaded and measured (`ggml-base`, `ggml-tiny`, `ggml-small` and the VAD
//! model: `sha256sum` matched, see the PR).
//!
//! The SHA-256 implementation is here, not in a crate, because the workspace
//! declares no hashing dependency (T1 pinned the manifest) and adding one for
//! 70 lines of a published algorithm would grow the lock for every crate.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crate::error::Error;

/// Best model, for a machine with memory to spare (`engine.MODEL_TURBO_Q8`).
pub const TURBO_Q8: &str = "ggml-large-v3-turbo-q8_0.bin";

/// Same model, smaller weights (`engine.MODEL_TURBO_Q5`).
pub const TURBO_Q5: &str = "ggml-large-v3-turbo-q5_0.bin";

/// The small model, the last rung on a 1 GB box (`engine.MODEL_SMALL`).
pub const SMALL: &str = "ggml-small.bin";

/// The base model: what the benchmarks and this crate's own tests use, because
/// it transcribes eleven seconds of speech in a fraction of a second.
pub const BASE: &str = "ggml-base.bin";

/// The tiny model: 78 MB, what the desktop job runs on a software Vulkan device
/// (`lavapipe`) where a bigger model would take minutes.
pub const TINY: &str = "ggml-tiny.bin";

/// The Silero VAD model whisper.cpp reads (`whisper.cpp --vad`): 885 kB of
/// voice-activity detection, so silence is not handed to the transcription.
pub const VAD: &str = "ggml-silero-v5.1.2.bin";

/// The VAD model's name in whisper.cpp's own `for-tests` releases: accepted as
/// an alternative so a file copied from a whisper.cpp checkout is recognised.
pub const VAD_FOR_TESTS: &str = "for-tests-silero-v5.1.2-ggml.bin";

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

/// The bytes every model of the ladder is supposed to have, and its SHA-256.
///
/// One row per file the app can download; [`file`] is the lookup by name. The
/// sizes are what the settings screen shows before a click, the hashes are what
/// [`verify_sha256`] checks after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelFile {
    /// The file's name, which is also its identity (`engine.py#model_name`).
    pub name: &'static str,
    /// The exact byte count of a complete download.
    pub bytes: u64,
    /// Lower-case hex SHA-256 of the file.
    pub sha256: &'static str,
}

/// Every file T4 may download, and what CI fetches: the ladder, the two models
/// this lot measures with, and the VAD model.
pub const FILES: [ModelFile; 6] = [
    ModelFile {
        name: TURBO_Q8,
        bytes: 874_188_075,
        sha256: "317eb69c11673c9de1e1f0d459b253999804ec71ac4c23c17ecf5fbe24e259a1",
    },
    ModelFile {
        name: TURBO_Q5,
        bytes: 574_041_195,
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
    },
    ModelFile {
        name: SMALL,
        bytes: 487_601_967,
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
    },
    ModelFile {
        name: BASE,
        bytes: 147_951_465,
        sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
    },
    ModelFile {
        name: TINY,
        bytes: 77_691_713,
        sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
    },
    ModelFile {
        name: VAD,
        bytes: 885_098,
        sha256: "29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf",
    },
];

/// The row for `name`, or `None` for a file this crate did not pin.
pub fn file(name: &str) -> Option<&'static ModelFile> {
    FILES.iter().find(|entry| entry.name == name)
}

/// The SHA-256 a download of `name` must have, as lower-case hex.
pub fn sha256_for(name: &str) -> Option<&'static str> {
    file(name).map(|entry| entry.sha256)
}

/// The size a complete download of `name` must have.
pub fn bytes_for(name: &str) -> Option<u64> {
    file(name).map(|entry| entry.bytes)
}

/// The ladder for the memory this box has right now (`engine.choose_model`).
///
/// Unknown free memory keeps the configured file, so a caller with no
/// `/proc/meminfo` (another OS, a container without it) does not silently pick
/// the smallest model.
pub fn ladder(free_bytes: Option<u64>) -> &'static [&'static str] {
    match free_bytes {
        Some(free) if free >= TURBO_FREE_BYTES => &ORDER_FREE,
        Some(_) => &ORDER_TIGHT,
        None => &[],
    }
}

/// `MemAvailable` from `/proc/meminfo`, in bytes (`engine.free_memory_bytes`).
pub fn free_memory_bytes() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    text.lines().find_map(|line| {
        let rest = line.strip_prefix("MemAvailable:")?;
        let kib: u64 = rest.split_whitespace().next()?.parse().ok()?;
        Some(kib.saturating_mul(1024))
    })
}

/// Whether `path` is a model whisper.cpp can load (`engine.valid_model`).
///
/// A regular file, at least [`MIN_MODEL_BYTES`], and starting with a GGML magic:
/// the three checks that catch a truncated download before the engine spends a
/// second on it.
pub fn is_valid(path: &Path) -> bool {
    validate(path).is_ok()
}

/// [`is_valid`], with the server's own failure code and sentence for the cause.
///
/// The size is returned on success because the settings screen shows it and the
/// engine uses it to name a GPU that is too small.
pub fn validate(path: &Path) -> Result<u64, Error> {
    let metadata = std::fs::metadata(path).map_err(|_| Error::ModelMissing)?;
    if !metadata.is_file() {
        return Err(Error::ModelMissing);
    }
    if metadata.len() < MIN_MODEL_BYTES {
        return Err(Error::ModelTooSmall);
    }
    let mut magic = [0u8; 4];
    let mut handle = File::open(path).map_err(|_| Error::ModelMissing)?;
    handle
        .read_exact(&mut magic)
        .map_err(|_| Error::ModelInvalid)?;
    if !MAGICS.contains(&magic.as_slice()) {
        return Err(Error::ModelInvalid);
    }
    Ok(metadata.len())
}

/// The file name of a model path (`engine.model_name`): what a chunk, a sidecar
/// and a database row are stamped with.
///
/// Non-UTF-8 names are lossily converted: a model name is shown to the person
/// and compared to [`FILES`], never used to open anything.
pub fn name(path: &Path) -> Option<String> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

/// The best model already on disk for the memory this box has right now.
///
/// `configured` is the file the person (or the app) named; its directory is
/// scanned for the known quantisations, so a 1 GB machine uses `ggml-small.bin`
/// without anyone editing a path. Falls back to `configured` itself when the
/// ladder finds nothing valid, exactly as `engine.choose_model` does.
pub fn choose(configured: &Path, free_bytes: Option<u64>) -> Option<PathBuf> {
    let ladder = ladder(free_bytes);
    if !ladder.is_empty() {
        let directory = configured.parent().unwrap_or_else(|| Path::new("."));
        for name in ladder {
            let candidate = directory.join(name);
            if is_valid(&candidate) {
                return Some(candidate);
            }
        }
    }
    is_valid(configured).then(|| configured.to_path_buf())
}

/// The VAD model that sits next to the whisper model, if any.
///
/// A whisper.cpp checkout, a `docker/transcribe` deployment and the app's model
/// folder all keep it beside the model it belongs to; both names whisper.cpp
/// ships are recognised, and the first one found wins.
pub fn vad_beside(model: &Path) -> Option<PathBuf> {
    let directory = model.parent().unwrap_or_else(|| Path::new("."));
    [VAD, VAD_FOR_TESTS]
        .iter()
        .map(|name| directory.join(name))
        .find(|candidate| candidate.is_file())
}

/// The directory the app keeps its downloaded models in.
///
/// `$XDG_DATA_HOME/neezer/models`, or `~/.local/share/neezer/models`: the same
/// root as the local transcripts (`$XDG_DATA_HOME/neezer/transcripts`), which is
/// what the note names for the app's own data.
pub fn default_dir() -> PathBuf {
    data_home().join("neezer").join("models")
}

/// `$XDG_DATA_HOME`, or `$HOME/.local/share` (the XDG base directory rule).
fn data_home() -> PathBuf {
    if let Some(home) = std::env::var_os("XDG_DATA_HOME")
        && !home.is_empty()
    {
        return PathBuf::from(home);
    }
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home).join(".local").join("share")
}

/// Whether `path` hashes to `expected_hex` (lower-case hex, 64 characters).
///
/// A file that cannot be read is `false`, not an error: the caller is checking a
/// download it just wrote, and "not the file we pinned" is the answer either way.
pub fn verify_sha256(path: &Path, expected_hex: &str) -> bool {
    match sha256_file(path) {
        Ok(digest) => to_hex(&digest).eq_ignore_ascii_case(expected_hex),
        Err(_) => false,
    }
}

/// The SHA-256 of a file, computed in 64 KiB blocks so an 874 MB model costs
/// 64 KiB of memory.
pub fn sha256_file(path: &Path) -> io::Result<[u8; 32]> {
    let mut handle = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = handle.read(&mut buffer)?;
        if read == 0 {
            return Ok(hasher.finish());
        }
        hasher.update(&buffer[..read]);
    }
}

/// Lower-case hex, in the order the digest bytes are in (what `sha256sum`
/// prints).
pub fn to_hex(digest: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// SHA-256 (FIPS 180-4), for the pinned model checksums.
///
/// Small, standard and dependency-free: the workspace has no hashing crate, and
/// one is not worth adding to a lock file every desktop crate shares.
#[derive(Debug)]
pub struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    /// Bytes in `buffer`.
    filled: usize,
    /// Total message length in bytes.
    length: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    /// The standard initial state.
    pub fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buffer: [0; 64],
            filled: 0,
            length: 0,
        }
    }

    /// Feed more message bytes.
    pub fn update(&mut self, mut data: &[u8]) {
        self.length = self.length.saturating_add(data.len() as u64);
        if self.filled > 0 {
            let take = (64 - self.filled).min(data.len());
            self.buffer[self.filled..self.filled + take].copy_from_slice(&data[..take]);
            self.filled += take;
            data = &data[take..];
            if self.filled == 64 {
                let block = self.buffer;
                self.compress(&block);
                self.filled = 0;
            }
        }
        while data.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&data[..64]);
            self.compress(&block);
            data = &data[64..];
        }
        if !data.is_empty() {
            self.buffer[..data.len()].copy_from_slice(data);
            self.filled = data.len();
        }
    }

    /// The digest; the hasher must not be fed afterwards.
    pub fn finish(mut self) -> [u8; 32] {
        let bit_length = self.length.saturating_mul(8);
        self.update(&[0x80]);
        while self.filled != 56 {
            self.update(&[0]);
        }
        // `update` counted the padding in `length`, but the length field is the
        // message's, so it is written from the value saved before the padding.
        let mut tail = [0u8; 8];
        tail.copy_from_slice(&bit_length.to_be_bytes());
        self.buffer[56..64].copy_from_slice(&tail);
        let block = self.buffer;
        self.compress(&block);
        let mut digest = [0u8; 32];
        for (index, word) in self.state.iter().enumerate() {
            digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        digest
    }

    /// One 64-byte compression round.
    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        let (words, _) = block.as_chunks::<4>();
        for (index, word) in words.iter().enumerate() {
            w[index] = u32::from_be_bytes(*word);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(ROUND_CONSTANTS[index])
                .wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        for (slot, value) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
        }
    }
}

/// SHA-256's round constants (FIPS 180-4, 4.2.2): the first 32 bits of the
/// fractional parts of the cube roots of the first 64 primes.
const ROUND_CONSTANTS: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

#[cfg(test)]
mod tests {
    use super::{
        BASE, FILES, MAGICS, MIN_MODEL_BYTES, ORDER_FREE, ORDER_TIGHT, SMALL, Sha256, TINY,
        TURBO_FREE_BYTES, TURBO_Q5, TURBO_Q8, VAD, VAD_FOR_TESTS, bytes_for, choose, file, ladder,
        name, sha256_for, to_hex, vad_beside,
    };
    use std::path::{Path, PathBuf};

    /// A scratch directory under the OS temp dir, unique per test.
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nz-models-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch directory");
        dir
    }

    /// A file of `bytes` length that starts with a GGML magic.
    fn model(path: &Path, length: u64) {
        let mut content = vec![0u8; length as usize];
        content[..4].copy_from_slice(b"ggml");
        std::fs::write(path, content).expect("write the fake model");
    }

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
        // Unknown memory keeps the configured file: an empty ladder.
        assert!(ladder(None).is_empty());
        assert_eq!(ladder(Some(0)), ORDER_TIGHT);
        assert_eq!(ladder(Some(TURBO_FREE_BYTES)), ORDER_FREE);
        assert_eq!(ladder(Some(TURBO_FREE_BYTES - 1)), ORDER_TIGHT);
    }

    /// The two thresholds are sizes, and both have to be plausible: a 1 MiB floor
    /// and 2.5 GB of free memory for the biggest model.
    #[test]
    fn the_thresholds_are_sizes() {
        assert_eq!(MIN_MODEL_BYTES, 1_048_576);
        assert_eq!(TURBO_FREE_BYTES, 2_500_000_000);
        const { assert!(TURBO_FREE_BYTES > MIN_MODEL_BYTES) };
    }

    /// Every pinned file has a 64-hex digest, a plausible size, and a name the
    /// ladder or the VAD actually uses: a hash nobody can reach is not a pin.
    #[test]
    fn every_pinned_file_is_complete() {
        assert_eq!(FILES.len(), 6);
        for entry in FILES {
            assert_eq!(entry.sha256.len(), 64, "{}", entry.name);
            assert!(
                entry.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "{}",
                entry.name
            );
            assert!(
                entry.sha256 == entry.sha256.to_ascii_lowercase(),
                "{}",
                entry.name
            );
            // The VAD model is 885 kB — a GGML file, just not a *whisper* model:
            // `MIN_MODEL_BYTES` is the ladder's floor, and `validate` is only
            // applied to the ladder.
            let floor = if entry.name == VAD {
                100_000
            } else {
                MIN_MODEL_BYTES
            };
            assert!(entry.bytes > floor, "{}", entry.name);
            assert_eq!(file(entry.name).map(|f| f.sha256), Some(entry.sha256));
            assert_eq!(
                sha256_for(entry.name).map(str::len),
                Some(64),
                "{}",
                entry.name
            );
            assert_eq!(bytes_for(entry.name), Some(entry.bytes));
        }
        assert!(file("ggml-nope.bin").is_none());
        assert_eq!(sha256_for(TURBO_Q8).map(str::len), Some(64));
        assert_eq!(sha256_for(TINY).map(str::len), Some(64));
        assert_eq!(sha256_for(BASE).map(str::len), Some(64));
        assert_eq!(sha256_for(VAD).map(str::len), Some(64));

        // One row per name (two hashes for one file is a download that can never
        // pass), and every name is shaped like the GGML file it names: the app
        // appends it to a URL and writes it in `$XDG_DATA_HOME/neezer/models`.
        for (index, entry) in FILES.iter().enumerate() {
            assert!(entry.name.starts_with("ggml-"), "{}", entry.name);
            assert!(entry.name.ends_with(".bin"), "{}", entry.name);
            assert!(!entry.name.contains('/'), "{}", entry.name);
            assert!(
                FILES[index + 1..]
                    .iter()
                    .all(|other| other.name != entry.name),
                "{} appears twice in FILES",
                entry.name
            );
        }
        // The digest of the row is the digest the download is held to: one table,
        // not two that can drift.
        assert_eq!(sha256_for(VAD), Some(FILES[5].sha256));
        assert_eq!(file(VAD).map(|entry| entry.bytes), Some(885_098));
    }

    /// SHA-256 against the FIPS 180-4 / NIST vectors, in one shot and split at
    /// every buffer boundary the implementation has.
    #[test]
    fn sha256_matches_the_published_vectors() {
        let hash = |data: &[u8]| {
            let mut hasher = Sha256::new();
            hasher.update(data);
            to_hex(&hasher.finish())
        };
        assert_eq!(
            hash(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hash(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hash(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        let million_a = vec![b'a'; 1_000_000];
        assert_eq!(
            hash(&million_a),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
        // The same million bytes fed 63 at a time: every `buffer` path runs.
        let mut hasher = Sha256::new();
        for chunk in million_a.chunks(63) {
            hasher.update(chunk);
        }
        assert_eq!(
            to_hex(&hasher.finish()),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    /// The server's `valid_model`, one refusal per cause, and the name the
    /// sidecar is stamped with.
    #[test]
    fn a_model_is_a_magic_and_a_size() {
        let dir = scratch("validate");
        let good = dir.join("ggml-good.bin");
        model(&good, MIN_MODEL_BYTES);
        assert!(super::is_valid(&good));
        assert_eq!(super::validate(&good), Ok(MIN_MODEL_BYTES));

        let small = dir.join("ggml-small.bin");
        model(&small, MIN_MODEL_BYTES - 1);
        assert_eq!(
            super::validate(&small),
            Err(crate::error::Error::ModelTooSmall)
        );

        let broken = dir.join("ggml-broken.bin");
        let mut content = vec![0u8; MIN_MODEL_BYTES as usize];
        content[..4].copy_from_slice(b"junk");
        std::fs::write(&broken, content).expect("write");
        assert_eq!(
            super::validate(&broken),
            Err(crate::error::Error::ModelInvalid)
        );

        let missing = dir.join("nope.bin");
        assert_eq!(
            super::validate(&missing),
            Err(crate::error::Error::ModelMissing)
        );
        assert!(!super::is_valid(&dir), "a directory is not a model");
        assert_eq!(name(&good).as_deref(), Some("ggml-good.bin"));
        assert_eq!(name(Path::new("")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `choose_model`: the ladder is scanned in the configured file's directory,
    /// the q8 is skipped below 2.5 GB, and the configured file is the last
    /// resort when the ladder finds nothing.
    #[test]
    fn the_ladder_picks_the_best_file_on_disk() {
        let dir = scratch("choose");
        let q5 = dir.join(TURBO_Q5);
        let small = dir.join(SMALL);
        model(&q5, MIN_MODEL_BYTES);
        model(&small, MIN_MODEL_BYTES);
        let configured = dir.join(TURBO_Q8);

        assert_eq!(choose(&configured, Some(8_000_000_000)), Some(q5.clone()));
        assert_eq!(choose(&configured, Some(1_000_000_000)), Some(q5.clone()));

        std::fs::remove_file(&q5).expect("remove");
        assert_eq!(
            choose(&configured, Some(8_000_000_000)),
            Some(small.clone())
        );
        // Unknown memory keeps the configured file — and refuses it when it is
        // not a model, which is `engine.choose_model`'s `None`.
        assert_eq!(choose(&configured, None), None);

        model(&configured, MIN_MODEL_BYTES);
        assert_eq!(choose(&configured, None), Some(configured.clone()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The VAD model is looked for beside the whisper model, under both names
    /// whisper.cpp ships, and an empty directory answers nothing.
    #[test]
    fn the_vad_model_lives_beside_the_model() {
        let dir = scratch("vad");
        let whisper = dir.join(BASE);
        model(&whisper, MIN_MODEL_BYTES);
        assert_eq!(vad_beside(&whisper), None);
        let vad = dir.join(VAD);
        std::fs::write(&vad, b"vad").expect("write");
        assert_eq!(vad_beside(&whisper), Some(vad.clone()));
        std::fs::remove_file(&vad).expect("remove");
        let alternative = dir.join(VAD_FOR_TESTS);
        std::fs::write(&alternative, b"vad").expect("write");
        assert_eq!(vad_beside(&whisper), Some(alternative));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The digest of a file is the digest of its bytes, whether they arrived in
    /// one read or several.
    #[test]
    fn a_files_digest_is_its_bytes_digest() {
        let dir = scratch("file");
        let path = dir.join("blob");
        let content: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &content).expect("write");
        let mut hasher = Sha256::new();
        hasher.update(&content);
        let expected = to_hex(&hasher.finish());
        assert_eq!(
            super::sha256_file(&path).map(|digest| to_hex(&digest)).ok(),
            Some(expected.clone())
        );
        assert!(super::verify_sha256(&path, &expected));
        assert!(super::verify_sha256(&path, &expected.to_ascii_uppercase()));
        assert!(!super::verify_sha256(&path, &"0".repeat(64)));
        assert!(!super::verify_sha256(&dir.join("nope"), &expected));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
