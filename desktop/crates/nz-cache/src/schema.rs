//! The schema: the PRAGMAs every connection runs under, the version, and the
//! migrations.
//!
//! `PRAGMA user_version` is the truth about what the file contains (rule 7: an
//! existing local cache keeps working after an upgrade). A migration here is a
//! plain `.sql` file under `migrations/` that only ever ADDS an object
//! (`IF NOT EXISTS`), so running it twice is a no-op — which is both the test's
//! acceptance criterion and the boot-time repair path for a file whose version
//! was lost.

use rusqlite::Connection;
use tracing::{debug, warn};

use crate::{BUSY_TIMEOUT, Error, Result};

/// The schema version this build writes and reads.
pub const SCHEMA_VERSION: i64 = 1;

/// Runs on the connection at open, before anything else touches the file.
///
/// - **WAL** is what keeps a read from waiting on a write (a background
///   download marking progress while the UI opens a playlist). Measured on this
///   machine: a reader on a second connection answers in **0.18 ms** (0.25 ms
///   in release) while a writer holds `BEGIN IMMEDIATE`, against blocking for
///   as long as the write lasts in rollback mode.
/// - **`busy_timeout`** turns a rare lock contention into a short wait instead
///   of an instant `SQLITE_BUSY` the app would report as a failure.
/// - **`synchronous = NORMAL`** is the documented safe pairing with WAL: a
///   power loss can lose the last transactions, never corrupt the file. A cache
///   does not need `FULL` (an fsync per write for data it can re-fetch).
pub(crate) fn prepare(conn: &mut Connection) -> Result<()> {
    conn.busy_timeout(BUSY_TIMEOUT)?;
    let mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        // A filesystem that cannot do WAL (some network mounts). The store
        // still works; reads then serialize behind a writer, which is why the
        // reason is logged rather than swallowed.
        warn!(mode = %mode, "local cache: SQLite refused WAL");
    }
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    migrate(conn)
}

/// The version stored in the file (`0` for a fresh or wiped database).
pub fn user_version(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("PRAGMA user_version", [], |row| row.get(0))?)
}

/// Applies every migration the file is missing.
///
/// A file from a NEWER build is refused rather than opened: the older app does
/// not know its columns, and replaying its own v1 over it is exactly how a
/// column disappears.
pub fn migrate(conn: &Connection) -> Result<()> {
    let found = user_version(conn)?;
    if found > SCHEMA_VERSION {
        return Err(Error::SchemaNewer {
            found,
            supported: SCHEMA_VERSION,
        });
    }
    if found < 1 {
        apply_v1(conn)?;
        debug!(from = found, to = 1, "local cache migrated");
    }
    Ok(())
}

/// v1: every table of `docs/agents/desktop.md` *Caches*, all `IF NOT EXISTS`.
///
/// Public because it is the repair path as much as the first migration: a boot
/// that finds `user_version` behind replays it, and the test runs it twice on
/// the same file to hold that promise.
pub fn apply_v1(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(include_str!("migrations/v1.sql"))?;
    tx.pragma_update(None, "user_version", 1)?;
    tx.commit()?;
    Ok(())
}

/// The user tables the file holds, sorted. Diagnostics and tests.
pub fn tables(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
         ORDER BY name",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use rusqlite::Connection;

    use crate::Result;
    use crate::testing::TempDir;

    const TABLES: [&str; 8] = [
        "api_cache",
        "covers",
        "download_members",
        "download_sets",
        "downloads",
        "kv",
        "pending_progress",
        "play_cache",
    ];

    fn raw(dir: &TempDir) -> Connection {
        std::fs::create_dir_all(dir.path()).expect("dir");
        Connection::open(dir.path().join("desktop.sqlite")).expect("open")
    }

    /// Criterion: "migrations idempotent (open twice, run v1 twice)".
    #[test]
    fn v1_replayed_twice_leaves_the_same_schema() -> Result<()> {
        let dir = TempDir::new("v1-twice");
        let conn = raw(&dir);
        super::apply_v1(&conn)?;
        let once = super::tables(&conn)?;

        // Replay it on the SAME file: the boot-time repair path.
        super::apply_v1(&conn)?;
        let twice = super::tables(&conn)?;

        assert_eq!(once, twice);
        assert_eq!(once, TABLES);
        assert_eq!(super::user_version(&conn)?, 1);
        Ok(())
    }

    /// Criterion: "opening a v0 empty file and a v1 file both work".
    #[test]
    fn a_zero_byte_file_is_migrated_to_v1() -> Result<()> {
        let dir = TempDir::new("v0-empty");
        std::fs::write(dir.path().join("desktop.sqlite"), b"").expect("empty file");

        let cache = crate::Cache::open_with_cache_dir(dir.path(), dir.cache_dir())?;
        let conn = cache.lock()?;

        assert_eq!(super::user_version(&conn)?, 1);
        assert_eq!(super::tables(&conn)?, TABLES);
        Ok(())
    }

    #[test]
    fn a_v1_file_is_opened_without_replaying_or_losing_rows() -> Result<()> {
        let dir = TempDir::new("v1-reopen");
        {
            let cache = crate::Cache::open_with_cache_dir(dir.path(), dir.cache_dir())?;
            cache.kv().set("apparence.zoom", &1.25)?;
            let guard = cache.lock()?;
            assert_eq!(super::user_version(&guard)?, 1);
        }

        let reopened = crate::Cache::open_with_cache_dir(dir.path(), dir.cache_dir())?;
        {
            let guard = reopened.lock()?;
            assert_eq!(super::user_version(&guard)?, 1);
        }
        assert_eq!(reopened.kv().get::<f64>("apparence.zoom"), Some(1.25));
        Ok(())
    }

    #[test]
    fn a_file_written_by_a_newer_build_is_refused_with_its_version() {
        let dir = TempDir::new("newer");
        {
            let conn = raw(&dir);
            super::apply_v1(&conn).expect("v1");
            conn.pragma_update(None, "user_version", 2).expect("bump");
        }

        let error = crate::Cache::open_with_cache_dir(dir.path(), dir.cache_dir())
            .expect_err("a newer schema must not open");
        assert!(
            matches!(
                error,
                crate::Error::SchemaNewer {
                    found: 2,
                    supported: 1
                }
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn the_journal_is_wal() -> Result<()> {
        let dir = TempDir::new("wal");
        let cache = crate::Cache::open_with_cache_dir(dir.path(), dir.cache_dir())?;
        let mode: String = cache
            .lock()?
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
        assert_eq!(mode, "wal");
        Ok(())
    }

    /// Criterion: "concurrent read while a write runs (WAL) does not block".
    ///
    /// Measured: the reader answers in **0.18 ms** (0.25 ms release) while the
    /// writer holds `BEGIN IMMEDIATE` (budget 250 ms, which only a
    /// rollback-journal database could miss by waiting for the writer).
    #[test]
    fn a_reader_is_not_blocked_by_an_open_write_transaction() -> Result<()> {
        let dir = TempDir::new("wal-reader");
        let cache = crate::Cache::open_with_cache_dir(dir.path(), dir.cache_dir())?;
        cache.downloads().add_set(
            "42",
            "playlist",
            "7",
            "Soirée",
            &[crate::DownloadTrack {
                track: crate::TrackId::new("3135556"),
                quality: crate::Quality::Opus320,
            }],
        )?;
        let path = cache.db_path();

        // A second connection, exactly like the app's `nz-bulk` thread while the
        // UI's `nz-net` thread writes.
        let writer = Connection::open(&path).expect("second handle");
        writer
            .execute_batch("BEGIN IMMEDIATE; INSERT INTO kv(key, value) VALUES('w', '1');")
            .expect("hold the write lock");

        let reader = std::thread::spawn(move || -> (std::time::Duration, i64) {
            let conn = Connection::open(&path).expect("reader");
            conn.busy_timeout(std::time::Duration::from_secs(5))
                .expect("busy timeout");
            // Time the READ, not the connection open or the thread spawn.
            let started = Instant::now();
            let count = conn
                .query_row("SELECT COUNT(*) FROM downloads", [], |row| row.get(0))
                .expect("read while a writer holds the lock");
            (started.elapsed(), count)
        });
        let (elapsed, seen) = reader.join().expect("reader thread");

        writer.execute_batch("COMMIT;").expect("commit");
        assert_eq!(seen, 1);
        assert!(
            elapsed < std::time::Duration::from_millis(250),
            "the read waited {elapsed:?} on the writer: the file is not in WAL mode"
        );
        eprintln!("read during write: {elapsed:?}");
        Ok(())
    }

    /// A number on the startup path: opening an existing, migrated store (what
    /// `app` does before its first frame) costs one pragma + one version read.
    #[test]
    fn opening_an_existing_store_is_fast() -> Result<()> {
        let dir = TempDir::new("open-cost");
        drop(crate::Cache::open_with_cache_dir(
            dir.path(),
            dir.cache_dir(),
        )?);

        let started = Instant::now();
        let cache = crate::Cache::open_with_cache_dir(dir.path(), dir.cache_dir())?;
        let elapsed = started.elapsed();
        assert!(cache.db_path().exists());
        assert!(
            elapsed < std::time::Duration::from_millis(250),
            "opening the store took {elapsed:?}"
        );
        eprintln!("open existing store: {elapsed:?}");
        Ok(())
    }
}
