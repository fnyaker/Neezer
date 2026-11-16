//! The local SQLite file: API cache, covers, play cache, downloads, settings,
//! queue snapshot.
//!
//! Owned by lot D3 (`docs/agents/desktop.md`, *Caches*). One file in the data
//! directory (`$XDG_DATA_HOME/neezer/desktop.sqlite`, WAL), versioned by
//! `PRAGMA user_version`, plus the files its rows point at:
//!
//! - `$XDG_CACHE_HOME/neezer/covers/` — cover art, LRU-capped;
//! - `$XDG_CACHE_HOME/neezer/audio/` — streamed audio, LRU-capped;
//! - `$XDG_DATA_HOME/neezer/offline/` — the person's downloads, never evicted.
//!
//! # The rules this file enforces
//!
//! - **Access never touches the main thread.** Every call locks the connection
//!   and finishes; the app calls this from `nz-net` and `nz-bulk` only, so the
//!   GPUI thread never waits on disk. `Cache` is `Send + Sync` and cheap to
//!   clone, so a worker keeps its own handle instead of mailing work around.
//! - **A lost cache is never a lost action.** `Cache` is a cache: every method
//!   that the app calls on a screen's path either answers (hit), answers `None`
//!   (miss) or returns an error the caller reports as a cause. Nothing panics on
//!   a full disk, a poisoned lock or a database written by a newer build.
//! - **Nothing here deletes a download** except `Downloads::remove_set` /
//!   `Downloads::remove`, which only the person's own command reaches.
//! - **Paths never come from the server.** Every file name is an FNV-1a digest
//!   of the identity it names (`track` + quality, or the cover URL), and the
//!   extension of a cover is sniffed from its own first bytes.
//!
//! ```no_run
//! # fn main() -> Result<(), nz_cache::Error> {
//! let cache = nz_cache::Cache::open("/home/me/.local/share/neezer")?;
//! cache.put("42", "/me/playlists", "[]");
//! cache.play_cache().evict_to(2 * 1024 * 1024 * 1024)?;
//! # Ok(())
//! # }
//! ```

pub mod api_cache;
pub mod covers;
pub mod downloads;
pub mod kv;
pub mod play_cache;
pub mod queue;
pub mod schema;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, Params, Row};

pub use covers::Covers;
pub use downloads::{
    DownloadJob, DownloadProgress, DownloadSet, DownloadState, DownloadTrack, Downloads,
};
pub use kv::Kv;
pub use play_cache::PlayCache;
pub use queue::{MAX_QUEUE_ENTRIES, PendingProgress, Queue, QueueSnapshot};
pub use schema::SCHEMA_VERSION;

use nz_model::{Quality, TrackId};

/// The one file this crate reads and writes.
const DB_FILE: &str = "desktop.sqlite";
/// How long a second connection waits for SQLite's write lock before giving up.
/// Long enough that a big download's `mark` never fails on contention, short
/// enough that a wedged database is a reported cause and not a frozen thread.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// What can go wrong with the local store.
///
/// The variants carry the cause verbatim so a screen can print it (rule 3: a
/// screen that cannot load says why).
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// SQLite itself: a corrupted file, a locked database, a failed write.
    #[error("base de données locale : {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// The directory or a cache file could not be created or removed.
    #[error("fichiers locaux : {0}")]
    Io(#[from] std::io::Error),
    /// A settings value or a queue snapshot that will not serialize.
    #[error("données locales illisibles : {0}")]
    Json(#[from] serde_json::Error),
    /// Another thread panicked while holding the connection.
    #[error("cache local verrouillé")]
    Locked,
    /// The file was written by a NEWER build. Refusing is the safe direction:
    /// an older app replaying its own migrations over it would delete columns.
    #[error("cache local en v{found}, ce build lit la v{supported} : mets l'application à jour")]
    SchemaNewer {
        /// The version found in the file.
        found: i64,
        /// The newest version this build knows.
        supported: i64,
    },
}

/// The crate's result type.
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
struct Inner {
    conn: Mutex<Connection>,
    data_dir: PathBuf,
    cache_dir: PathBuf,
    /// Every SQL statement issued through this handle, which is how the tests
    /// hold a criterion the code cannot quietly drift away from ("saving the
    /// queue is one statement").
    statements: AtomicU64,
}

/// The local store: a database, its directories, and the stores built on it.
///
/// Clone it freely (`Arc` inside): a worker thread keeps its own handle. Two
/// handles, or two connections on the same file, read while one writes — that
/// is what WAL buys and what `tests` assert.
#[derive(Clone, Debug)]
pub struct Cache {
    inner: Arc<Inner>,
}

impl Cache {
    /// Opens (creating it and its directories if needed) the store under
    /// `data_dir`: the SQLite file and the offline downloads live there, and
    /// the evictable covers/audio under `$XDG_CACHE_HOME/neezer`.
    ///
    /// ```
    /// # let dir = std::env::temp_dir().join("nz-cache-doc-open");
    /// # let _ = std::fs::remove_dir_all(&dir);
    /// # std::fs::create_dir_all(&dir).unwrap();
    /// let cache = nz_cache::Cache::open(&dir)?;
    /// assert_eq!(cache.db_path(), dir.join("desktop.sqlite"));
    /// # drop(cache);
    /// # std::fs::remove_dir_all(&dir).unwrap();
    /// # Ok::<(), nz_cache::Error>(())
    /// ```
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self> {
        let data_dir = data_dir.as_ref().to_path_buf();
        let cache_dir = default_cache_dir(&data_dir);
        Self::open_with_cache_dir(data_dir, cache_dir)
    }

    /// `open`, with the evictable directory chosen by the caller. Tests use it
    /// so a run never writes into the person's real `$XDG_CACHE_HOME`; the app
    /// uses [`Cache::open`].
    pub fn open_with_cache_dir(
        data_dir: impl AsRef<Path>,
        cache_dir: impl AsRef<Path>,
    ) -> Result<Self> {
        let data_dir = data_dir.as_ref().to_path_buf();
        let cache_dir = cache_dir.as_ref().to_path_buf();
        for dir in [
            data_dir.clone(),
            cache_dir.join("covers"),
            cache_dir.join("audio"),
            data_dir.join("offline"),
        ] {
            fs::create_dir_all(&dir)?;
        }

        let mut conn = Connection::open(data_dir.join(DB_FILE))?;
        schema::prepare(&mut conn)?;
        let version = schema::user_version(&conn)?;
        tracing::debug!(
            db = %data_dir.join(DB_FILE).display(),
            schema = version,
            "local cache open"
        );

        Ok(Self {
            inner: Arc::new(Inner {
                conn: Mutex::new(conn),
                data_dir,
                cache_dir,
                statements: AtomicU64::new(0),
            }),
        })
    }

    /// The directory the database and the downloads live under.
    pub fn data_dir(&self) -> &Path {
        &self.inner.data_dir
    }

    /// The directory the evictable files live under.
    pub fn cache_dir(&self) -> &Path {
        &self.inner.cache_dir
    }

    /// The SQLite file itself.
    pub fn db_path(&self) -> PathBuf {
        self.inner.data_dir.join(DB_FILE)
    }

    /// Where the person's downloads are written.
    pub fn offline_dir(&self) -> PathBuf {
        self.inner.data_dir.join("offline")
    }

    /// Where cover art is written.
    pub fn covers_dir(&self) -> PathBuf {
        self.inner.cache_dir.join("covers")
    }

    /// Where streamed audio is written.
    pub fn audio_dir(&self) -> PathBuf {
        self.inner.cache_dir.join("audio")
    }

    /// How many SQL statements this handle has issued. The acceptance criteria
    /// count statements rather than guessing ("the 4 000-entry queue is ONE
    /// statement"), and a bench can hold this number.
    pub fn statements(&self) -> u64 {
        self.inner.statements.load(Ordering::Relaxed)
    }

    /// The API-response cache, as the HTTP client sees it (`nz_api`'s trait).
    pub fn covers(&self) -> Covers<'_> {
        Covers::new(self)
    }

    /// The evictable cache of streamed audio.
    pub fn play_cache(&self) -> PlayCache<'_> {
        PlayCache::new(self)
    }

    /// The person's offline downloads.
    pub fn downloads(&self) -> Downloads<'_> {
        Downloads::new(self)
    }

    /// Typed settings, the SPA's `localStorage` equivalent.
    pub fn kv(&self) -> Kv<'_> {
        Kv::new(self)
    }

    /// The player's saved queue (one row, at most [`MAX_QUEUE_ENTRIES`]).
    pub fn queue(&self) -> Queue<'_> {
        Queue::new(self)
    }

    /// Podcast positions written offline, waiting for the server.
    pub fn pending_progress(&self) -> PendingProgress<'_> {
        PendingProgress::new(self)
    }

    /// Takes the connection. Every public method goes through here, which is
    /// what makes "never on the main thread" a call-site rule and not a hope.
    pub(crate) fn lock(&self) -> Result<MutexGuard<'_, Connection>> {
        self.inner.conn.lock().map_err(|_| Error::Locked)
    }

    /// Runs `f` with the connection inside a transaction, so a multi-statement
    /// change (a download set and its members) is one atomic step.
    pub(crate) fn write<T>(&self, f: impl FnOnce(&Cache, &Connection) -> Result<T>) -> Result<T> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        let out = f(self, &tx)?;
        tx.commit()?;
        Ok(out)
    }

    /// Runs `f` with the connection, read-only.
    pub(crate) fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let conn = self.lock()?;
        f(&conn)
    }

    /// One counted statement: every write and every read of this crate goes
    /// through [`Cache::exec`], [`Cache::query`] or [`Cache::rows`].
    pub(crate) fn exec(&self, conn: &Connection, sql: &str, params: impl Params) -> Result<usize> {
        self.inner.statements.fetch_add(1, Ordering::Relaxed);
        Ok(conn.execute(sql, params)?)
    }

    pub(crate) fn query<T>(
        &self,
        conn: &Connection,
        sql: &str,
        params: impl Params,
        f: impl FnOnce(&Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<T> {
        self.inner.statements.fetch_add(1, Ordering::Relaxed);
        Ok(conn.query_row(sql, params, f)?)
    }

    /// `query`, with "no row" as `None` instead of an error — the shape every
    /// lookup in this crate wants.
    pub(crate) fn query_optional<T>(
        &self,
        conn: &Connection,
        sql: &str,
        params: impl Params,
        f: impl FnOnce(&Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Option<T>> {
        self.inner.statements.fetch_add(1, Ordering::Relaxed);
        match conn.query_row(sql, params, f) {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub(crate) fn rows<T>(
        &self,
        conn: &Connection,
        sql: &str,
        params: impl Params,
        f: impl FnMut(&Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Vec<T>> {
        self.inner.statements.fetch_add(1, Ordering::Relaxed);
        let mut stmt = conn.prepare(sql)?;
        let mapped = stmt.query_map(params, f)?;
        let mut out = Vec::new();
        for row in mapped {
            out.push(row?);
        }
        Ok(out)
    }
}

/// This store as the HTTP client sees it: `nz-api`'s [`nz_api::ResponseCache`].
///
/// The trait lives in `nz-api` so the client never depends on SQLite, and its
/// three methods are exactly this crate's API-cache store — the paths the page
/// caches, the wire's TEXT, keyed by owner. The cacheable-path filter stays in
/// [`api_cache`] and runs on both sides, so a caller cannot resurrect a live
/// endpoint (`/playlist/<id>/version`) by asking for it.
impl nz_api::ResponseCache for Cache {
    fn get(&self, owner: &str, path: &str) -> Option<String> {
        Cache::get(self, owner, path)
    }

    fn put(&self, owner: &str, path: &str, body: &str) {
        Cache::put(self, owner, path, body);
    }

    fn delete(&self, owner: &str, path: &str) {
        Cache::delete(self, owner, path);
    }
}

/// `$XDG_CACHE_HOME/neezer`, or `<data_dir>/cache` on a machine without a home
/// directory (a container, a test). The evictable files must never land next to
/// the database by accident when the environment is unusual.
fn default_cache_dir(data_dir: &Path) -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.cache_dir().join("neezer"))
        .unwrap_or_else(|| data_dir.join("cache"))
}

/// Milliseconds since the Unix epoch, the unit every `*_at`/`used_at` column
/// uses. A clock before 1970 cannot happen on a desktop that boots; it reads as
/// 0 rather than panicking.
pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// FNV-1a, 64 bits: the file name of a cached object.
///
/// Not a cryptographic hash and not asked to be: it names a file this process
/// also reads the row for, and a collision would need two tracks to share a
/// digest (a 4 000-track library lands at a birthday probability of ~4e-13).
/// What it must be is STABLE across runs and platforms — `std`'s `DefaultHasher`
/// is explicitly allowed to change between releases, and a cache whose file
/// names move is a cache that re-downloads everything on an upgrade.
#[derive(Debug, Clone)]
pub(crate) struct Fnv1a(u64);

impl Fnv1a {
    pub(crate) fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    pub(crate) fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    pub(crate) fn hex(self) -> String {
        format!("{:016x}", self.0)
    }
}

/// The digest naming a track's audio at one quality: `(track, quality)`, never
/// a server-supplied path (docs/agents/desktop.md, *Caches*).
pub(crate) fn track_digest(track: &TrackId, quality: Quality) -> String {
    let mut hash = Fnv1a::new();
    hash.write(track.as_str().as_bytes());
    hash.write(&[0]);
    hash.write(quality_name(quality).as_bytes());
    hash.hex()
}

/// The digest naming a cover: the remote URL the UI renders.
pub(crate) fn url_digest(url: &str) -> String {
    let mut hash = Fnv1a::new();
    hash.write(url.as_bytes());
    hash.hex()
}

/// The TEXT spelling of a quality in the database: the wire form (`OPUS_320`),
/// FLAC spelled out because "no `q`" is not a column value.
pub(crate) fn quality_name(quality: Quality) -> &'static str {
    quality.wire().unwrap_or("FLAC")
}

/// Reads back what [`quality_name`] wrote. An unknown or older spelling is not
/// a crash: a cached file whose quality the app no longer knows is a miss.
pub(crate) fn parse_quality(raw: &str) -> Option<Quality> {
    Quality::parse(Some(raw))
}

#[cfg(test)]
pub(crate) mod testing {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    /// A throwaway data + cache directory pair, removed on drop. Its own name
    /// per test, so `cargo test`'s parallel threads never share a file.
    pub(crate) struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        pub(crate) fn new(tag: &str) -> Self {
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("nz-cache-{}-{tag}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("temp dir");
            Self { path }
        }

        pub(crate) fn path(&self) -> &Path {
            &self.path
        }

        pub(crate) fn cache_dir(&self) -> PathBuf {
            self.path.join("cache")
        }

        /// A store whose two directories are both throwaway.
        pub(crate) fn open(&self) -> crate::Cache {
            crate::Cache::open_with_cache_dir(self.path.join("data"), self.cache_dir())
                .expect("open cache")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_store_is_send_and_sync_and_cheap_to_clone() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<Cache>();

        let dir = testing::TempDir::new("clone");
        let cache = dir.open();
        let copy = cache.clone();
        // Both handles reach the same file: a write through one is read by the
        // other, which is how the app's worker threads share it.
        copy.put("42", "/me/playlists", "[]");
        assert_eq!(cache.get("42", "/me/playlists").as_deref(), Some("[]"));
    }

    #[test]
    fn the_database_is_in_the_data_dir_and_the_evictable_files_in_the_cache_dir() {
        let dir = testing::TempDir::new("paths");
        let cache = dir.open();

        assert_eq!(cache.db_path(), dir.path().join("data/desktop.sqlite"));
        assert!(cache.db_path().is_file(), "the database file exists");
        assert_eq!(cache.offline_dir(), dir.path().join("data/offline"));
        assert_eq!(cache.covers_dir(), dir.cache_dir().join("covers"));
        assert_eq!(cache.audio_dir(), dir.cache_dir().join("audio"));
        for dir in [cache.offline_dir(), cache.covers_dir(), cache.audio_dir()] {
            assert!(dir.is_dir(), "{} was created at open", dir.display());
        }
    }

    #[test]
    fn opening_the_same_directory_twice_keeps_what_was_written() {
        let dir = testing::TempDir::new("reopen");
        {
            let cache = dir.open();
            cache.put("42", "/me/favorites", "[]");
            cache.kv().set("ui.theme", &"omarchy").expect("kv set");
        }
        let again = dir.open();
        assert_eq!(again.get("42", "/me/favorites").as_deref(), Some("[]"));
        assert_eq!(
            again.kv().get::<String>("ui.theme").as_deref(),
            Some("omarchy")
        );
    }
}
