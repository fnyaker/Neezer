//! The device's own store: `lan.sqlite`, holding the SHA-256 of every local file
//! a peer may fetch, the last address of every peer met, and the trust blobs the
//! server gave us (docs/agents/lan-nodes.md, *Device storage*).
//!
//! **LN4 filled it.** One file under `$data/neezer/lan/`, WAL, `PRAGMA
//! user_version`, idempotent migrations at start (rule 7 on the device: an
//! upgrade must not lose the hashes it already paid to compute).
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
//! Three rules the schema alone does not say:
//!
//! - **A path here is this device's own.** A peer names a track id and a
//!   quality; the path is derived from them under `nz-cache`'s directories, and
//!   [`Store::open`] never writes a path that came off the wire. Nothing on the
//!   read side either: a lookup key is always a local path the index holds.
//! - **Never read on a UI thread.** Nothing here is called by a screen: the
//!   index reads the whole `file_tag` table on its own (background) refresh, and
//!   [`crate::serve::Server`] reads one row per `Fetch`.
//! - **A trust blob is logged by its KEY only**, never by its bytes: a credential,
//!   a roster and a revocation list are what an attacker with this file wants.
//!
//! # Why `pub const Store` exists
//!
//! LN0 declared `Store` as a unit struct, and `enroll.rs` (LN7) writes `&Store`
//! in its placeholder tests. LN4 gives the type an `Arc` inside — a real handle
//! cannot be a unit struct — so the same name is also declared as a VALUE: the
//! UNOPENED store, which answers `None`/`Ok(())` and touches no file. Those two
//! placeholders then keep compiling, unchanged, until LN7 replaces them. (An
//! unopened store is honest, never a fake database: every read answers "nothing
//! stored" and every write answers the same, so a lot that forgot to fill one
//! cannot leave a half-made file in the person's data directory.)

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, params};
use tracing::{debug, warn};

use crate::proto::FileTag;
use crate::types::LanError;

/// The one file this module owns, under the LAN data directory
/// (`$XDG_DATA_HOME/neezer/lan/` on the desktop, `filesDir/lan/` on Android).
pub const DB_FILE: &str = "lan.sqlite";

/// The schema version this build writes and reads.
pub const SCHEMA_VERSION: i64 = 1;

/// How long a second statement waits for SQLite's write lock. The LAN store is
/// written by the hashing worker and read by the serving path; neither should
/// ever see `SQLITE_BUSY` because of the other.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// One `file_tag` row: what the hashing worker last recorded for a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredTag {
    /// The length the file had when it was hashed.
    pub len: u64,
    /// Its modification time, in whole unix seconds — the pair that decides
    /// whether a rehash is needed.
    pub mtime: i64,
    /// Lowercase hex SHA-256 of the complete file.
    pub sha256: String,
}

/// One `peer_addr` row: where a peer was last met.
///
/// The address is a HINT (the last-known-address path of *Discovery*): it proves
/// nothing, and the connection that uses it authenticates as usual.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPeer {
    /// The peer's Ed25519 public key, hex.
    pub device_pub: String,
    /// Its person, hex (never a name).
    pub subject: String,
    /// Its display name, as the authenticated `Hello` gave it.
    pub name: Option<String>,
    /// IPv4 or IPv6 literal.
    pub ip: String,
    pub port: u16,
    /// Unix seconds of the last authenticated contact.
    pub seen_at: i64,
}

#[derive(Debug)]
struct Inner {
    conn: Mutex<Connection>,
    dir: PathBuf,
}

/// The device's LAN database: cheap to clone (one `Arc`), never opened twice.
#[derive(Debug, Clone, Default)]
pub struct Store {
    inner: Option<Arc<Inner>>,
}

/// The UNOPENED store, under the name LN0's placeholders in `enroll.rs` and
/// `gossip.rs` already use (see this module's documentation).
#[allow(non_upper_case_globals)]
pub const Store: Store = Store { inner: None };

impl Store {
    /// Opens (creating it and its directory if needed) `lan.sqlite` under `dir`.
    ///
    /// The directory is created `0700`-friendly by the caller on Android; here
    /// the only promise is that a failure answers a cause instead of a panic,
    /// and that a file this build cannot read is left untouched.
    pub fn open(dir: &Path) -> Result<Store, LanError> {
        std::fs::create_dir_all(dir).map_err(|error| LanError::Disk(error.to_string()))?;
        // The directory holds a credential and a roster: another account on this
        // machine must not read them, even though impersonating this device also
        // needs `device.key` (0600, LN2). Best effort — a filesystem without unix
        // modes (Android's app-private storage, a FAT card) is already private.
        restrict(dir, 0o700);
        let path = dir.join(DB_FILE);
        let conn = Connection::open(&path).map_err(|error| LanError::Disk(error.to_string()))?;
        restrict(&path, 0o600);
        conn.busy_timeout(BUSY_TIMEOUT)
            .map_err(|error| LanError::Disk(error.to_string()))?;
        // WAL: the hashing worker writes a tag while a peer's `Fetch` reads one.
        let mode: String = conn
            .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
            .map_err(|error| LanError::Disk(error.to_string()))?;
        if !mode.eq_ignore_ascii_case("wal") {
            // An SD card or a network mount that refuses WAL still works; reads
            // then serialise behind a write, which is worth a line in the log.
            warn!(mode = %mode, "lan.sqlite: SQLite refused WAL");
        }
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(|error| LanError::Disk(error.to_string()))?;
        migrate(&conn)?;
        debug!(path = %path.display(), "lan.sqlite open");
        Ok(Store {
            inner: Some(Arc::new(Inner {
                conn: Mutex::new(conn),
                dir: dir.to_path_buf(),
            })),
        })
    }

    /// The directory this store lives in, `None` for an unopened one.
    pub fn dir(&self) -> Option<&Path> {
        self.inner.as_ref().map(|inner| inner.dir.as_path())
    }

    /// The file this store writes, `None` for an unopened one.
    pub fn db_path(&self) -> Option<PathBuf> {
        self.dir().map(|dir| dir.join(DB_FILE))
    }

    /// Whether this handle reached a file (`false` for [`Store`] and for a
    /// device that never enrolled). A caller that must distinguish "nothing
    /// stored" from "no database" reads this.
    pub fn is_open(&self) -> bool {
        self.inner.is_some()
    }

    // --- the file tags the hashing worker keeps ----------------------------

    /// The tag recorded for `path`, if the hashing worker has hashed this exact
    /// `(len, mtime)` — one indexed lookup, which is what a `Fetch` pays for its
    /// `tag` field.
    pub fn tag(&self, path: &Path) -> Option<FileTag> {
        let row = self.tag_row(path)?;
        Some(FileTag {
            len: row.len,
            sha256: row.sha256,
        })
    }

    /// The whole row (length, mtime, digest) of `path`, for the hashing worker's
    /// "do I have to hash this again" decision.
    pub fn tag_row(&self, path: &Path) -> Option<StoredTag> {
        let inner = self.inner.as_ref()?;
        let key = key_of(path);
        let conn = lock(inner);
        match conn.query_row(
            "SELECT len, mtime, sha256 FROM file_tag WHERE path = ?1",
            params![key],
            |row| {
                Ok(StoredTag {
                    len: row.get::<_, i64>(0)?.max(0) as u64,
                    mtime: row.get(1)?,
                    sha256: row.get(2)?,
                })
            },
        ) {
            Ok(tag) => Some(tag),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(error) => {
                debug!(%error, "file_tag read failed");
                None
            }
        }
    }

    /// Every tag this device holds, keyed by path — ONE statement, because the
    /// index reads the whole table on every refresh (8 000 files, one query) and
    /// the hashing worker compares against the same map.
    pub fn tags(&self) -> HashMap<String, StoredTag> {
        let Some(inner) = self.inner.as_ref() else {
            return HashMap::new();
        };
        let conn = lock(inner);
        let mut stmt = match conn.prepare("SELECT path, len, mtime, sha256 FROM file_tag") {
            Ok(stmt) => stmt,
            Err(error) => {
                debug!(%error, "file_tag scan failed");
                return HashMap::new();
            }
        };
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                StoredTag {
                    len: row.get::<_, i64>(1)?.max(0) as u64,
                    mtime: row.get(2)?,
                    sha256: row.get(3)?,
                },
            ))
        });
        let mut out = HashMap::new();
        match rows {
            Ok(rows) => {
                for row in rows {
                    match row {
                        Ok((path, tag)) => {
                            out.insert(path, tag);
                        }
                        Err(error) => {
                            debug!(%error, "file_tag row unreadable");
                        }
                    }
                }
            }
            Err(error) => debug!(%error, "file_tag scan failed"),
        }
        out
    }

    /// Records the digest of `path` at `(len, mtime)`. One upsert.
    pub fn put_tag(&self, path: &Path, len: u64, mtime: i64, sha256: &str) -> Result<(), LanError> {
        let Some(inner) = self.inner.as_ref() else {
            return Ok(());
        };
        let key = key_of(path);
        let conn = lock(inner);
        conn.execute(
            "INSERT INTO file_tag(path, len, mtime, sha256) VALUES(?1, ?2, ?3, ?4) \
             ON CONFLICT(path) DO UPDATE SET len = excluded.len, mtime = excluded.mtime, \
             sha256 = excluded.sha256",
            params![key, len as i64, mtime, sha256],
        )
        .map_err(|error| LanError::Disk(error.to_string()))?;
        Ok(())
    }

    /// Forgets a file nothing lists any more (a download removed, a play-cache
    /// entry evicted), so a re-download is hashed instead of being served with
    /// the old file's digest.
    pub fn forget_tag(&self, path: &Path) -> Result<bool, LanError> {
        let Some(inner) = self.inner.as_ref() else {
            return Ok(false);
        };
        let key = key_of(path);
        let conn = lock(inner);
        let removed = conn
            .execute("DELETE FROM file_tag WHERE path = ?1", params![key])
            .map_err(|error| LanError::Disk(error.to_string()))?;
        Ok(removed > 0)
    }

    // --- the trust blobs the server signed ---------------------------------

    /// Stores one signed blob under `key` (`credential`, `roster`,
    /// `revocations:<kid>`, `ca:<kid>`). One upsert.
    ///
    /// The KEY is what the log may name; the bytes never are.
    pub fn put_trust(&self, key: &str, blob: &[u8]) -> Result<(), LanError> {
        let Some(inner) = self.inner.as_ref() else {
            return Ok(());
        };
        let conn = lock(inner);
        conn.execute(
            "INSERT INTO trust(key, blob, updated_at) VALUES(?1, ?2, ?3) \
             ON CONFLICT(key) DO UPDATE SET blob = excluded.blob, \
             updated_at = excluded.updated_at",
            params![key, blob, now()],
        )
        .map_err(|error| LanError::Disk(error.to_string()))?;
        Ok(())
    }

    /// The blob stored under `key`, if any.
    pub fn trust(&self, key: &str) -> Option<Vec<u8>> {
        let inner = self.inner.as_ref()?;
        let conn = lock(inner);
        match conn.query_row(
            "SELECT blob FROM trust WHERE key = ?1",
            params![key],
            |row| row.get::<_, Vec<u8>>(0),
        ) {
            Ok(blob) => Some(blob),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(error) => {
                debug!(%error, "trust read failed");
                None
            }
        }
    }

    /// Which blobs this device holds, WITHOUT their bytes: a log line and a
    /// "have I ever enrolled" check read this.
    pub fn trust_keys(&self) -> Vec<String> {
        let Some(inner) = self.inner.as_ref() else {
            return Vec::new();
        };
        let conn = lock(inner);
        let mut stmt = match conn.prepare("SELECT key FROM trust ORDER BY key") {
            Ok(stmt) => stmt,
            Err(error) => {
                debug!(%error, "trust scan failed");
                return Vec::new();
            }
        };
        let rows = stmt.query_map([], |row| row.get::<_, String>(0));
        match rows {
            Ok(rows) => rows.filter_map(|row| row.ok()).collect(),
            Err(error) => {
                debug!(%error, "trust scan failed");
                Vec::new()
            }
        }
    }

    /// Forgets every trust blob (logout: the key, the credential, the roster).
    pub fn forget_trust(&self) -> Result<usize, LanError> {
        let Some(inner) = self.inner.as_ref() else {
            return Ok(0);
        };
        let conn = lock(inner);
        conn.execute("DELETE FROM trust", [])
            .map_err(|error| LanError::Disk(error.to_string()))
    }

    // --- the peers met -----------------------------------------------------

    /// Records where a peer was last met. One upsert; a peer that moved keeps
    /// one row.
    pub fn put_peer_addr(&self, peer: &StoredPeer) -> Result<(), LanError> {
        let Some(inner) = self.inner.as_ref() else {
            return Ok(());
        };
        let conn = lock(inner);
        conn.execute(
            "INSERT INTO peer_addr(device_pub, subject, name, ip, port, seen_at) \
             VALUES(?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(device_pub) DO UPDATE SET subject = excluded.subject, \
             name = excluded.name, ip = excluded.ip, port = excluded.port, \
             seen_at = excluded.seen_at",
            params![
                peer.device_pub,
                peer.subject,
                peer.name,
                peer.ip,
                i64::from(peer.port),
                peer.seen_at
            ],
        )
        .map_err(|error| LanError::Disk(error.to_string()))?;
        Ok(())
    }

    /// Every peer this device met, newest first — the last-known-address path.
    pub fn peer_addrs(&self) -> Vec<StoredPeer> {
        let Some(inner) = self.inner.as_ref() else {
            return Vec::new();
        };
        let conn = lock(inner);
        let mut stmt = match conn.prepare(
            "SELECT device_pub, subject, name, ip, port, seen_at FROM peer_addr \
             ORDER BY seen_at DESC",
        ) {
            Ok(stmt) => stmt,
            Err(error) => {
                debug!(%error, "peer_addr scan failed");
                return Vec::new();
            }
        };
        let rows = stmt.query_map([], |row| {
            Ok(StoredPeer {
                device_pub: row.get(0)?,
                subject: row.get(1)?,
                name: row.get(2)?,
                ip: row.get(3)?,
                port: row.get::<_, i64>(4)?.clamp(1, i64::from(u16::MAX)) as u16,
                seen_at: row.get(5)?,
            })
        });
        match rows {
            Ok(rows) => rows.filter_map(|row| row.ok()).collect(),
            Err(error) => {
                debug!(%error, "peer_addr scan failed");
                Vec::new()
            }
        }
    }

    /// Forgets every peer address (Réglages → « oublier les appareils vus »).
    pub fn forget_peers(&self) -> Result<usize, LanError> {
        let Some(inner) = self.inner.as_ref() else {
            return Ok(0);
        };
        let conn = lock(inner);
        conn.execute("DELETE FROM peer_addr", [])
            .map_err(|error| LanError::Disk(error.to_string()))
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        // WAL leaves `-wal`/`-shm` behind; a clean close checkpoints and removes
        // them, which keeps the data directory looking like one file.
        if let Ok(conn) = self.conn.lock() {
            let _: Result<String, _> =
                conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0));
        }
    }
}

/// The `file_tag` key of a local path: its bytes as text.
///
/// The paths here are `nz-cache`'s own (`<dir>/<hex digest>.<quality>`), so they
/// are UTF-8; a path that is not is written lossily rather than refused, and
/// answers the same lossy key on read, so a lookup still matches.
pub(crate) fn key_of(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Gives a path `mode`, on unix only, and never fails: the caller is already
/// private on the platforms that have no modes.
fn restrict(path: &Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
}

fn lock(inner: &Inner) -> MutexGuard<'_, Connection> {
    // A panic in another thread must not turn every later read into a panic: the
    // connection is a file handle, and SQLite's own state is not in Rust memory.
    inner
        .conn
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The version stored in the file (`0` for a fresh one).
fn user_version(conn: &Connection) -> Result<i64, LanError> {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| LanError::Disk(error.to_string()))
}

/// Applies every migration the file is missing (rule 7 on the device).
///
/// A file from a NEWER build is refused, exactly as `nz-cache` refuses one: the
/// older app does not know its columns, and replaying its own v1 over it is how
/// a column disappears. A file whose version was lost but whose tables are there
/// is repaired rather than re-created: `CREATE … IF NOT EXISTS` only ever adds.
fn migrate(conn: &Connection) -> Result<(), LanError> {
    let found = user_version(conn)?;
    if found > SCHEMA_VERSION {
        return Err(LanError::Internal(format!(
            "lan.sqlite en v{found}, ce build lit la v{SCHEMA_VERSION}"
        )));
    }
    if found < SCHEMA_VERSION || !tables_present(conn)? {
        apply_v1(conn)?;
        debug!(from = found, to = SCHEMA_VERSION, "lan.sqlite migrated");
    }
    Ok(())
}

/// Whether the three v1 tables are there (the boot-time repair check).
fn tables_present(conn: &Connection) -> Result<bool, LanError> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' \
             AND name IN ('file_tag', 'peer_addr', 'trust')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| LanError::Disk(error.to_string()))?;
    Ok(count == 3)
}

/// v1: the three tables of *Device storage*, all `IF NOT EXISTS` and therefore
/// replayable. Public because it IS the repair path as much as the first
/// migration.
pub fn apply_v1(conn: &Connection) -> Result<(), LanError> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| LanError::Disk(error.to_string()))?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS file_tag(\
           path TEXT PRIMARY KEY, len INTEGER NOT NULL, mtime INTEGER NOT NULL, \
           sha256 TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS peer_addr(\
           device_pub TEXT PRIMARY KEY, subject TEXT NOT NULL, name TEXT, \
           ip TEXT NOT NULL, port INTEGER NOT NULL, seen_at INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS trust(\
           key TEXT PRIMARY KEY, blob BLOB NOT NULL, updated_at INTEGER NOT NULL);",
    )
    .map_err(|error| LanError::Disk(error.to_string()))?;
    // One query the serving path makes per page: a track's tags by path prefix
    // would scan, and `seen_at` orders the last-known addresses.
    tx.execute_batch("CREATE INDEX IF NOT EXISTS peer_addr_seen ON peer_addr(seen_at);")
        .map_err(|error| LanError::Disk(error.to_string()))?;
    tx.pragma_update(None, "user_version", SCHEMA_VERSION)
        .map_err(|error| LanError::Disk(error.to_string()))?;
    tx.commit()
        .map_err(|error| LanError::Disk(error.to_string()))?;
    Ok(())
}

/// The user tables this file holds, sorted. Diagnostics and tests.
pub fn tables(conn: &Connection) -> Result<Vec<String>, LanError> {
    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table' \
             AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .map_err(|error| LanError::Disk(error.to_string()))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| LanError::Disk(error.to_string()))?;
    Ok(rows.filter_map(|row| row.ok()).collect())
}

/// Reads the version of a `lan.sqlite` without opening a [`Store`] over it: the
/// tests' upgrade path (`open` twice) and a diagnostics line.
pub fn version_at(dir: &Path) -> Option<i64> {
    let path = dir.join(DB_FILE);
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    user_version(&conn).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nz-lan-store-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// The three tables of *Device storage*, at v1, and the file appears where
    /// the caller asked — one file, in the LAN directory.
    #[test]
    fn opening_creates_the_three_tables_at_version_one() {
        let dir = temp("open");
        let store = Store::open(&dir).expect("open");
        assert!(store.is_open());
        assert_eq!(store.db_path(), Some(dir.join(DB_FILE)));
        assert_eq!(version_at(&dir), Some(SCHEMA_VERSION));

        let conn = Connection::open(dir.join(DB_FILE)).expect("second handle");
        assert_eq!(
            tables(&conn).expect("tables"),
            vec![
                "file_tag".to_owned(),
                "peer_addr".to_owned(),
                "trust".to_owned()
            ]
        );
        drop(store);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Rule 7 on the device: a second boot keeps every hash it already paid to
    /// compute, and replaying the migration changes nothing.
    #[test]
    fn a_second_open_keeps_the_tags_and_replays_nothing() {
        let dir = temp("reopen");
        let path = dir.join("audio/abc.FLAC");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("audio dir");
        {
            let store = Store::open(&dir).expect("open");
            store
                .put_tag(&path, 4_096, 1_700_000_000, &"ab".repeat(32))
                .expect("tag");
        }
        let store = Store::open(&dir).expect("reopen");
        let row = store.tag_row(&path).expect("the tag survived");
        assert_eq!(row.len, 4_096);
        assert_eq!(row.mtime, 1_700_000_000);
        assert_eq!(row.sha256.len(), 64);
        assert_eq!(
            store.tag(&path),
            Some(FileTag {
                len: 4_096,
                sha256: "ab".repeat(32),
            })
        );
        // The repair path is idempotent: the same three tables, no v2.
        assert_eq!(version_at(&dir), Some(SCHEMA_VERSION));
        drop(store);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The file and its directory are private: the credential and the roster are
    /// in there, and another account on this machine has no business reading them.
    #[cfg(unix)]
    #[test]
    fn the_database_and_its_directory_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp("modes");
        let store = Store::open(&dir).expect("open");
        let db = dir.join(DB_FILE);
        let file = std::fs::metadata(&db)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        let parent = std::fs::metadata(&dir)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(file, 0o600, "lan.sqlite is the person's");
        assert_eq!(parent, 0o700, "and so is its directory");
        drop(store);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A file written by a NEWER build is refused with its cause, and nothing is
    /// replayed over it.
    #[test]
    fn a_newer_database_is_refused() {
        let dir = temp("newer");
        {
            let store = Store::open(&dir).expect("open");
            drop(store);
            let conn = Connection::open(dir.join(DB_FILE)).expect("handle");
            conn.pragma_update(None, "user_version", 9).expect("bump");
        }
        match Store::open(&dir) {
            Err(LanError::Internal(cause)) => assert!(cause.contains("v9"), "{cause}"),
            other => panic!("a newer file must not open: {other:?}"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The unopened store (LN0's `&Store` placeholder) answers "nothing stored"
    /// and writes nothing: no file appears next to it.
    #[test]
    fn the_unopened_store_touches_no_file() {
        assert!(!Store.is_open());
        assert_eq!(Store.dir(), None);
        assert_eq!(Store.db_path(), None);
        assert!(Store.tags().is_empty());
        assert_eq!(Store.tag(Path::new("/nowhere")), None);
        assert_eq!(
            Store.put_tag(Path::new("/nowhere"), 1, 2, "00"),
            Ok(()),
            "a write on an unopened store is a no-op, never a fake database"
        );
        assert!(Store.trust_keys().is_empty());
        assert!(Store.peer_addrs().is_empty());
    }

    /// One peer that moved keeps ONE row, newest address, and the list is newest
    /// first — that order is the last-known-address retry order.
    #[test]
    fn a_peer_that_moved_keeps_one_row() {
        let dir = temp("peers");
        let store = Store::open(&dir).expect("open");
        let peer = StoredPeer {
            device_pub: "aa".repeat(32),
            subject: "bb".repeat(16),
            name: Some("PC".to_owned()),
            ip: "192.0.2.7".to_owned(),
            port: 45_910,
            seen_at: 10,
        };
        store.put_peer_addr(&peer).expect("first");
        store
            .put_peer_addr(&StoredPeer {
                ip: "192.0.2.8".to_owned(),
                seen_at: 20,
                ..peer
            })
            .expect("moved");

        let rows = store.peer_addrs();
        assert_eq!(rows.len(), 1, "one row per device key");
        assert_eq!(rows[0].ip, "192.0.2.8");
        assert_eq!(rows[0].seen_at, 20);
        assert_eq!(rows[0].port, 45_910);

        assert_eq!(store.forget_peers(), Ok(1));
        assert!(store.peer_addrs().is_empty());
        drop(store);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A trust blob round-trips byte for byte, and the key list never carries
    /// the bytes (`trust_keys` is what a log line may name).
    #[test]
    fn a_trust_blob_round_trips_and_its_key_is_not_its_bytes() {
        let dir = temp("trust");
        let store = Store::open(&dir).expect("open");
        let blob = vec![0u8, 1, 2, 255, 254];
        store.put_trust("credential", &blob).expect("put");
        assert_eq!(store.trust("credential"), Some(blob));
        assert_eq!(store.trust("roster"), None);
        assert_eq!(store.trust_keys(), vec!["credential".to_owned()]);

        assert_eq!(store.forget_trust(), Ok(1));
        assert!(store.trust_keys().is_empty());
        drop(store);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A tag is one upsert: re-hashing the same path REPLACES it, and forgetting
    /// it answers whether there was one.
    #[test]
    fn a_tag_is_replaced_not_duplicated() {
        let dir = temp("tags");
        let store = Store::open(&dir).expect("open");
        let path = dir.join("track.OPUS_320");

        store
            .put_tag(&path, 10, 100, &"aa".repeat(32))
            .expect("put");
        store
            .put_tag(&path, 20, 200, &"bb".repeat(32))
            .expect("put");
        let tags = store.tags();
        assert_eq!(tags.len(), 1, "one row per path");
        assert_eq!(tags[&key_of(&path)].len, 20);
        assert!(tags[&key_of(&path)].sha256.starts_with("bb"));

        assert_eq!(store.forget_tag(&path), Ok(true));
        assert_eq!(store.forget_tag(&path), Ok(false));
        assert!(store.tags().is_empty());
        drop(store);
        std::fs::remove_dir_all(&dir).ok();
    }
}
