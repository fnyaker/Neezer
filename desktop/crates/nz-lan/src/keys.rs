//! The device's own Ed25519 key: one file, mode `0600`, written atomically, and
//! rotated at every credential renewal (docs/agents/lan-nodes.md, *Trust chain*).
//!
//! **LN0 declares the surface; LN2 fills the bodies** with `aws-lc-rs`
//! (`signature::Ed25519KeyPair::from_seed_unchecked`) — no other crypto crate,
//! and no `unsafe`.
//!
//! The file is a 32-byte seed at `$data/neezer/lan/device.key`. It is the one
//! secret on the device: whoever copies it is this device until the person
//! revokes it (T11), which is why it is written with a temp file, `fsync` and a
//! rename, excluded from Android backup, and wiped at logout.

use std::path::Path;

use crate::types::DevicePub;

/// This device's key pair.
///
/// **LN2** turns this into an `aws_lc_rs::signature::Ed25519KeyPair` built from
/// the 32-byte seed on disk. A later lot never constructs one any other way:
/// `load_or_create` is the only door, so a key that exists is always a key that
/// was persisted.
#[derive(Debug)]
pub struct DeviceKey {
    /// The public half, cached: it is asked on every connection and every
    /// credential renewal.
    public: DevicePub,
}

impl DeviceKey {
    /// Loads `device.key` from `dir`, creating it if it is absent (mode `0600`,
    /// temp file + `fsync` + rename, so a crash mid-write leaves the previous key
    /// intact — a device that loses its key silently becomes a different device).
    pub fn load_or_create(dir: &Path) -> std::io::Result<DeviceKey> {
        let _ = dir;
        Err(unsupported(
            "la clé d'appareil arrive avec le lot LN2 (nz-lan/keys.rs)",
        ))
    }

    /// Replaces the key with a fresh one and answers it. A renewal rotates the
    /// key: the old one is not kept unless the person revokes the device.
    pub fn rotate(dir: &Path) -> std::io::Result<DeviceKey> {
        let _ = dir;
        Err(unsupported(
            "la rotation de clé arrive avec le lot LN2 (nz-lan/keys.rs)",
        ))
    }

    /// Removes the key file (logout). Absent is success: the post-condition is
    /// "no key on disk", not "a file was deleted".
    pub fn wipe(dir: &Path) -> std::io::Result<()> {
        let _ = dir;
        Err(unsupported(
            "l'effacement de clé arrive avec le lot LN2 (nz-lan/keys.rs)",
        ))
    }

    /// The public half: what a peer checks and what a credential carries.
    pub fn public(&self) -> DevicePub {
        self.public
    }

    /// Signs `msg` with this key (Ed25519, deterministic per RFC 8032).
    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        let _ = msg;
        [0u8; 64]
    }

    /// The possession proof of an enrolment:
    /// `Ed25519(device key, "neezer-lan/1 enroll\0" ‖ subject_hex ‖ pubkey_hex ‖ at)`.
    ///
    /// `subject_hex` comes from `GET /api/lan/status` and `at` is THIS device's
    /// clock; the server allows ±300 s around its own, which is why a refusal for
    /// skew must say « l'heure de l'appareil est décalée » (LN7).
    pub fn enroll_proof(&self, subject_hex: &str, at: i64) -> [u8; 64] {
        let _ = (subject_hex, at);
        [0u8; 64]
    }
}

/// `io::ErrorKind::Unsupported`: the create's own vocabulary for "this build
/// does not do that yet", which is what every stub in this crate answers.
fn unsupported(what: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Unsupported, what.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;

    /// A stub never invents a key: `load_or_create` refuses, and no file appears
    /// under a directory it was given. A lot that forgot to fill this cannot
    /// silently enrol a device with an all-zero key.
    #[test]
    fn an_unimplemented_key_file_refuses_and_writes_nothing() {
        let dir = std::env::temp_dir().join(format!("nz-lan-keys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");

        let error = DeviceKey::load_or_create(&dir).expect_err("refused");
        assert_eq!(error.kind(), ErrorKind::Unsupported);
        assert_eq!(
            DeviceKey::rotate(&dir).expect_err("refused").kind(),
            ErrorKind::Unsupported
        );
        assert_eq!(
            DeviceKey::wipe(&dir).expect_err("refused").kind(),
            ErrorKind::Unsupported
        );
        assert_eq!(
            std::fs::read_dir(&dir).expect("readable").count(),
            0,
            "nothing was written"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The seed file is the only place a key exists, and its permissions are part
    /// of the contract (0600): a test of LN2 asserts it, and this one only names
    /// the promise so the next reader cannot miss it.
    #[test]
    fn the_key_is_a_32_byte_seed_and_nothing_else() {
        assert_eq!(std::mem::size_of::<DevicePub>(), 32);
    }
}
