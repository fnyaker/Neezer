//! The device's own Ed25519 key: one file, mode `0600`, written atomically, and
//! rotated at every credential renewal (docs/agents/lan-nodes.md, *Trust chain*).
//!
//! **LN2 fills the bodies** with `aws-lc-rs`
//! (`signature::Ed25519KeyPair::from_seed_unchecked`, `rand::fill`) — no other
//! crypto crate, and no `unsafe`.
//!
//! The file is a 32-byte seed at `$data/neezer/lan/device.key`. It is the one
//! secret on the device: whoever copies it is this device until the person
//! revokes it (T11), which is why it is written with a temp file, `fsync` and a
//! rename, kept at `0600` (a key restored from a backup with wider permissions is
//! tightened when it is read), excluded from Android backup, and wiped at logout.
//!
//! # What LN2 decided, and why
//!
//! - **A crash never leaves half a key.** The seed is written to a temp file in the
//!   same directory, `fsync`ed, then renamed over `device.key`: a reader sees the
//!   old key or the new one, never 0 bytes and never 17 (`a_reader_never_sees_half
//!   _a_key_while_it_rotates`). A leftover temp file from a crash is ignored by
//!   `load_or_create` and removed by `wipe`.
//! - **A key file that exists is never silently replaced.** A file of the wrong
//!   length is an error, not a reason to generate a new identity: a device that
//!   loses its key silently becomes a different device (and its peers refuse it).
//! - **Reads are bounded by the file's own size**, so a symlink to a 10 GB file
//!   is refused instead of read into memory.
//! - **The seed is scrubbed after use** (`fill(0)` + `black_box`), and `Debug`
//!   prints the public key only: a key that reaches a log is a key that leaked.
//! - **The enrol proof signs `at` as its decimal text**, like the two hex strings
//!   beside it in the note's formula (`"neezer-lan/1 enroll\0" ‖ subject_hex ‖
//!   pubkey_hex ‖ at`): the note spells out `i64` where it means the binary
//!   encoding (`not_before`/`not_after`), and it does not here. LN7 and LN1 must
//!   spell it the same way; `the_enroll_proof_signs_the_note_message` is the test
//!   that fails if either side changes it.

use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::Path;

use aws_lc_rs::rand;
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};

use crate::cred::hex_encode;
use crate::types::DevicePub;

/// The key file's name under the device's LAN directory.
pub const KEY_FILE: &str = "device.key";

/// A seed and an Ed25519 public key are both 32 bytes; the file holds the seed.
const SEED_LEN: usize = 32;

/// The prefix of the temp file an interrupted write leaves behind.
const TEMP_PREFIX: &str = ".device.key.tmp-";

/// The mode the seed file must have: owner reads and writes, nobody else at all.
#[cfg(unix)]
const SEED_MODE: u32 = 0o600;

/// The mode the LAN directory is created with (it holds the key).
#[cfg(unix)]
const DIR_MODE: u32 = 0o700;

/// The domain an enrolment's possession proof is computed over.
const ENROLL_DOMAIN: &[u8] = b"neezer-lan/1 enroll\x00";

/// This device's key pair.
///
/// Built from the 32-byte seed on disk through `load_or_create` or `rotate`: a
/// later lot never constructs one any other way, so a key that exists is always a
/// key that was persisted.
pub struct DeviceKey {
    /// The aws-lc-rs pair (the only place the seed lives in memory).
    key: Ed25519KeyPair,
    /// The public half, cached: it is asked on every connection and every
    /// credential renewal.
    public: DevicePub,
}

impl DeviceKey {
    /// Loads `device.key` from `dir`, creating it if it is absent (mode `0600`,
    /// temp file + `fsync` + rename, so a crash mid-write leaves the previous key
    /// intact — a device that loses its key silently becomes a different device).
    ///
    /// The directory is created `0700` when it does not exist. A file whose
    /// permissions are wider than `0600` is tightened here: the post-condition is
    /// "a `0600` key file", whatever a backup or an old version left behind.
    pub fn load_or_create(dir: &Path) -> std::io::Result<DeviceKey> {
        ensure_dir(dir)?;
        match read_seed(&dir.join(KEY_FILE))? {
            Some(mut seed) => {
                let key = from_seed(&seed);
                scrub(&mut seed);
                key
            }
            None => {
                let (key, mut seed) = generate()?;
                let written = write_atomic(dir, &seed);
                scrub(&mut seed);
                written.map(|()| key)
            }
        }
    }

    /// Replaces the key with a fresh one and answers it. A renewal rotates the
    /// key: the old one is not kept unless the person revokes the device.
    ///
    /// The new seed goes through the same temp-file-then-rename path as a create,
    /// so a rotation that crashes leaves the previous key usable.
    pub fn rotate(dir: &Path) -> std::io::Result<DeviceKey> {
        ensure_dir(dir)?;
        let (key, mut seed) = generate()?;
        let written = write_atomic(dir, &seed);
        scrub(&mut seed);
        written.map(|()| key)
    }

    /// Removes the key file (logout). Absent is success: the post-condition is
    /// "no key on disk", not "a file was deleted".
    ///
    /// A temp file left by an interrupted write is removed too, because the seed
    /// it holds is as good as the key file itself.
    pub fn wipe(dir: &Path) -> std::io::Result<()> {
        match fs::remove_file(dir.join(KEY_FILE)) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with(TEMP_PREFIX) {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        // The deletion must survive a power cut too, or the "wipe" comes back.
        if let Ok(handle) = File::open(dir) {
            let _ = handle.sync_all();
        }
        Ok(())
    }

    /// The public half: what a peer checks and what a credential carries.
    pub fn public(&self) -> DevicePub {
        self.public
    }

    /// Signs `msg` with this key (Ed25519, deterministic per RFC 8032).
    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        let signature = self.key.sign(msg);
        let mut out = [0u8; 64];
        out.copy_from_slice(signature.as_ref());
        out
    }

    /// The possession proof of an enrolment:
    /// `Ed25519(device key, "neezer-lan/1 enroll\0" ‖ subject_hex ‖ pubkey_hex ‖ at)`.
    ///
    /// `subject_hex` comes from `GET /api/lan/status` and is signed **verbatim** —
    /// the exact characters the server sent, lowercase hex in practice — because
    /// the server recomputes the message with its own spelling. `pubkey_hex` is
    /// this key's public half in lowercase hex, and `at` is this device's clock in
    /// decimal unix seconds; the server allows ±300 s around its own, which is why
    /// a refusal for skew must say « l'heure de l'appareil est décalée » (LN7).
    pub fn enroll_proof(&self, subject_hex: &str, at: i64) -> [u8; 64] {
        let public = hex_encode(&self.public);
        let at = at.to_string();
        let mut message =
            Vec::with_capacity(ENROLL_DOMAIN.len() + subject_hex.len() + public.len() + at.len());
        message.extend_from_slice(ENROLL_DOMAIN);
        message.extend_from_slice(subject_hex.as_bytes());
        message.extend_from_slice(public.as_bytes());
        message.extend_from_slice(at.as_bytes());
        self.sign(&message)
    }
}

/// Prints the public key and nothing else: a `Debug` of the pair would be a seed
/// in a log the first time somebody logged an `Identity` (T11).
impl std::fmt::Debug for DeviceKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DeviceKey")
            .field("public", &hex_encode(&self.public))
            .finish_non_exhaustive()
    }
}

/// Creates the LAN directory when it is missing, `0700` because it holds the key.
fn ensure_dir(dir: &Path) -> std::io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(DIR_MODE))?;
    }
    Ok(())
}

/// Reads the seed from `path`, or `None` when there is no key file yet.
///
/// The size is checked from the metadata and the read is bounded to one byte more
/// than a seed, so a symlink to a huge file answers an error without being read
/// into memory.
fn read_seed(path: &Path) -> std::io::Result<Option<[u8; SEED_LEN]>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(invalid("device.key : ce n'est pas un fichier"));
    }
    if metadata.len() != SEED_LEN as u64 {
        return Err(invalid(&format!(
            "device.key : {} octets au lieu de {SEED_LEN}",
            metadata.len()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            fs::set_permissions(path, fs::Permissions::from_mode(SEED_MODE))?;
        }
    }
    let mut seed = [0u8; SEED_LEN];
    let mut reader = file.take(SEED_LEN as u64 + 1);
    reader
        .read_exact(&mut seed)
        .map_err(|error| invalid(&format!("device.key : {error}")))?;
    let mut extra = [0u8; 1];
    if reader.read(&mut extra)? != 0 {
        return Err(invalid("device.key : plus de 32 octets"));
    }
    Ok(Some(seed))
}

/// Builds the pair from a seed, and the public half with it.
fn from_seed(seed: &[u8; SEED_LEN]) -> std::io::Result<DeviceKey> {
    let key = Ed25519KeyPair::from_seed_unchecked(seed)
        .map_err(|_| invalid("device.key : graine Ed25519 refusée"))?;
    let mut public = [0u8; 32];
    public.copy_from_slice(key.public_key().as_ref());
    Ok(DeviceKey { key, public })
}

/// A fresh seed and its pair, from the system's CSPRNG.
fn generate() -> std::io::Result<(DeviceKey, [u8; SEED_LEN])> {
    let mut seed = [0u8; SEED_LEN];
    rand::fill(&mut seed).map_err(|_| random_unavailable())?;
    let key = from_seed(&seed)?;
    Ok((key, seed))
}

/// Writes the seed atomically: a temp file in `dir`, `fsync`, then a rename.
///
/// The temp file is removed when anything before the rename fails, so a failure
/// leaves the directory as it was found (plus nothing).
fn write_atomic(dir: &Path, seed: &[u8; SEED_LEN]) -> std::io::Result<()> {
    let mut suffix = [0u8; 6];
    rand::fill(&mut suffix).map_err(|_| random_unavailable())?;
    let temp = dir.join(format!(
        "{TEMP_PREFIX}{}-{}",
        std::process::id(),
        hex_encode(&suffix)
    ));
    match write_temp_and_rename(dir, &temp, seed) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&temp);
            Err(error)
        }
    }
}

fn write_temp_and_rename(dir: &Path, temp: &Path, seed: &[u8; SEED_LEN]) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(SEED_MODE);
    }
    let mut file = options.open(temp)?;
    file.write_all(seed)?;
    // The seed must be on the platter before a name points at it: a crash between
    // these two lines leaves the previous key, never a half-written one.
    file.sync_all()?;
    drop(file);
    let key_path = dir.join(KEY_FILE);
    fs::rename(temp, &key_path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // `OpenOptions::mode` is masked by the umask; the post-condition is `0600`.
        fs::set_permissions(&key_path, fs::Permissions::from_mode(SEED_MODE))?;
    }
    // The rename is what must survive a power cut. A filesystem that cannot fsync
    // a directory says so, and that is no reason to fail an enrolment.
    if let Ok(handle) = File::open(dir) {
        let _ = handle.sync_all();
    }
    Ok(())
}

/// Overwrites a seed that is no longer needed.
///
/// Best effort by nature (the compiler may keep a copy, the kernel may swap), but
/// it is the only cheap thing between a rotation and a seed sitting in freed heap
/// until the allocator reuses it.
fn scrub(seed: &mut [u8; SEED_LEN]) {
    seed.fill(0);
    std::hint::black_box(seed);
}

fn invalid(what: &str) -> std::io::Error {
    std::io::Error::new(ErrorKind::InvalidData, what.to_owned())
}

fn random_unavailable() -> std::io::Error {
    std::io::Error::other("générateur aléatoire du système indisponible")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A seed is 32 bytes and a public key is 32 bytes: the file is exactly one
    /// seed long, and the type it becomes is the `DevicePub` a credential carries.
    #[test]
    fn the_key_is_a_32_byte_seed_and_nothing_else() {
        assert_eq!(SEED_LEN, 32);
        assert_eq!(std::mem::size_of::<DevicePub>(), 32);
        assert_eq!(KEY_FILE, "device.key");
    }

    /// The enrolment proof's domain is NUL-terminated and its parts are the
    /// note's, in order: a typo here is a server-side `403` nobody can debug.
    #[test]
    fn the_enroll_domain_is_the_note_string() {
        assert_eq!(ENROLL_DOMAIN, b"neezer-lan/1 enroll\x00");
        assert_eq!(ENROLL_DOMAIN.len(), 20);
    }
}
