//! The device's own store: `lan.sqlite`, holding the SHA-256 of every local file
//! a peer may fetch, the last address of every peer met, and the trust blobs the
//! server gave us (docs/agents/lan-nodes.md, *Device storage*).
//!
//! **LN0 declares `Store`; LN4 fills it.** One file under
//! `$data/neezer/lan/`, WAL, `PRAGMA user_version`, idempotent migrations at
//! start (rule 7 on the device: an upgrade must not lose the hashes it already
//! paid to compute).
//!
//! ```sql
//! -- v1
//! CREATE TABLE IF NOT EXISTS file_tag(path TEXT PRIMARY KEY, len INTEGER NOT NULL,
//!   mtime INTEGER NOT NULL, sha256 TEXT NOT NULL);
//! CREATE TABLE IF NOT EXISTS peer_addr(device_pub TEXT PRIMARY KEY, subject TEXT NOT NULL,
//!   name TEXT, ip TEXT NOT NULL, port INTEGER NOT NULL, seen_at INTEGER NOT NULL);
//! CREATE TABLE IF NOT EXISTS trust(key TEXT PRIMARY KEY, blob BLOB NOT NULL,
//!   updated_at INTEGER NOT NULL);
//!   -- keys: 'credential', 'roster', 'revocations:<kid>', 'ca:<kid>'
//! ```
//!
//! Two rules that the schema alone does not say: this file never holds a PATH
//! that came from the wire (a peer names a track id, and the path is this device's
//! own), and it is never read on a UI thread — [`crate::service::Lan::snapshot`]
//! hands out a clone.

use std::path::Path;

use crate::types::LanError;

/// The device's LAN database.
///
/// Cheap to clone once LN4 is written (an `Arc` inside); nothing else in the crate
/// opens the file.
#[derive(Debug, Clone, Default)]
pub struct Store;

impl Store {
    /// Opens (creating it and its directory if needed) `lan.sqlite` under `dir`.
    pub fn open(dir: &Path) -> Result<Store, LanError> {
        let _ = dir;
        Err(LanError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stub store never creates a file: an unimplemented lot must not leave a
    /// half-initialised database in the person's data directory.
    #[test]
    fn an_unimplemented_store_opens_nothing() {
        let dir = std::env::temp_dir().join(format!("nz-lan-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        assert_eq!(Store::open(&dir).err(), Some(LanError::Unsupported));
        assert_eq!(
            std::fs::read_dir(&dir).expect("readable").count(),
            0,
            "no database, no journal, no WAL file"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
