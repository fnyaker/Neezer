//! The API-response cache: the SPA's `lib/apicache.js`, over SQLite.
//!
//! A pure fallback, exactly like the page's: online, every request hits the
//! network and refreshes the copy; only a failed GET is answered from here, so
//! a playlist, an album and the person's own library stay browsable with the
//! link down (rule 3: what was fetched once is kept on the device).
//!
//! Two rules make it the SAME cache as the page's:
//!
//! - **Only the paths the SPA caches** are stored, with its `LIVE_PATHS`
//!   exclusions (an episode's chapters, the playlist version probe, the admin
//!   surfaces change when something happens and must never be served stale).
//! - **Keyed by owner**, so two accounts on one machine never see each other's
//!   library.
//!
//! The body is kept as the TEXT that came off the wire, not parsed: one copy,
//! and a re-read costs one JSON parse instead of a tree in the database.

use rusqlite::params;
use tracing::debug;

use crate::{Cache, now_ms};

/// How many responses are kept — the page's `MAX_ENTRIES`. Past it the oldest
/// go: a cache bounded by recency, never by what a screen happens to need.
pub const MAX_ENTRIES: i64 = 400;

/// The path prefixes `lib/apicache.js#CACHE_PREFIXES` accepts.
const CACHE_PREFIXES: [&str; 10] = [
    "/me/playlists",
    "/me/favorites",
    "/me/favorite-ids",
    "/me/local",
    "/playlist/",
    "/album/",
    "/artist/",
    // The mood mixes' catalogue and their track lists.
    "/moods",
    // Lyrics are asked on every track change, and the panel must work offline.
    "/lyrics/",
    // A person's credits page.
    "/credits/person",
];

/// Is this path one the offline copy may answer?
///
/// Mirrors `isCacheable` in `webapp/src/lib/apicache.js`, query string included
/// in the exclusions' shape: `is_live` is checked on the path WITHOUT its query,
/// as the page does.
pub fn is_cacheable(path: &str) -> bool {
    let path = path.split('?').next().unwrap_or("");
    if is_live(path) {
        return false;
    }
    is_track_credits(path)
        || CACHE_PREFIXES
            .iter()
            .any(|prefix| path == *prefix || path.starts_with(prefix))
}

/// The page's `LIVE_PATHS`: endpoints that only LOOK content-shaped.
fn is_live(path: &str) -> bool {
    for root in ["/admin", "/invites", "/onboarding"] {
        if path == root || path.starts_with(&format!("{root}/")) {
            return true;
        }
    }
    // /^\/playlist\/[^/]+\/version$/
    if let Some((id, tail)) = path
        .strip_prefix("/playlist/")
        .and_then(|rest| rest.split_once('/'))
        && !id.is_empty()
        && tail == "version"
    {
        return true;
    }
    // /^\/podcast\/episode\/[^/]+\/chapters$/
    if let Some((id, tail)) = path
        .strip_prefix("/podcast/episode/")
        .and_then(|rest| rest.split_once('/'))
        && !id.is_empty()
        && tail == "chapters"
    {
        return true;
    }
    false
}

/// `/^\/track\/[^/]+\/credits$/`: a track's credits page, small and worth
/// keeping (the page's `CREDITS` pattern).
fn is_track_credits(path: &str) -> bool {
    path.strip_prefix("/track/")
        .and_then(|rest| rest.strip_suffix("/credits"))
        .is_some_and(|id| !id.is_empty() && !id.contains('/'))
}

impl Cache {
    /// The cached body of a cacheable `path`, or `None`.
    ///
    /// A non-cacheable path never has a row AND never gets one: the filter is
    /// here, not only at the write, so a caller cannot resurrect a stale live
    /// endpoint by asking for it.
    pub fn get(&self, owner: &str, path: &str) -> Option<String> {
        if !is_cacheable(path) {
            return None;
        }
        match self.cached_body(owner, path) {
            Ok(body) => body,
            Err(error) => {
                debug!(%error, path, "api cache read failed");
                None
            }
        }
    }

    /// Stores a response body verbatim (the wire's text). Best effort: a cache
    /// write that fails must never fail the request that produced it.
    pub fn put(&self, owner: &str, path: &str, body: &str) {
        if !is_cacheable(path) {
            return;
        }
        if let Err(error) = self.store_body(owner, path, body) {
            debug!(%error, path, "api cache write failed");
        }
    }

    /// Drops one cached response (a mutation just made it stale).
    pub fn delete(&self, owner: &str, path: &str) {
        if let Err(error) = self.drop_body(owner, path) {
            debug!(%error, path, "api cache delete failed");
        }
    }

    /// When a cached body was fetched, in milliseconds since the epoch. The
    /// background warmer uses it to skip what it refreshed a moment ago.
    pub fn cached_at(&self, owner: &str, path: &str) -> Option<i64> {
        if !is_cacheable(path) {
            return None;
        }
        self.cache_stamp(owner, path).ok().flatten()
    }

    /// Forgets everything one account cached (logout, account switch).
    pub fn purge_owner(&self, owner: &str) -> crate::Result<usize> {
        self.take_owner_rows(owner)
    }

    /// How many responses are stored (diagnostics, Réglages).
    pub fn cached_entries(&self) -> crate::Result<i64> {
        self.count_bodies()
    }

    fn cached_body(&self, owner: &str, path: &str) -> crate::Result<Option<String>> {
        self.read(|conn| {
            self.query_optional(
                conn,
                "SELECT body FROM api_cache WHERE owner = ?1 AND path = ?2",
                params![owner, path],
                |row| row.get(0),
            )
        })
    }

    fn cache_stamp(&self, owner: &str, path: &str) -> crate::Result<Option<i64>> {
        self.read(|conn| {
            self.query_optional(
                conn,
                "SELECT fetched_at FROM api_cache WHERE owner = ?1 AND path = ?2",
                params![owner, path],
                |row| row.get(0),
            )
        })
    }

    fn store_body(&self, owner: &str, path: &str, body: &str) -> crate::Result<()> {
        self.write(|cache, conn| {
            cache.exec(
                conn,
                "INSERT INTO api_cache(owner, path, body, fetched_at) VALUES(?1, ?2, ?3, ?4) \
                 ON CONFLICT(owner, path) DO UPDATE SET body = excluded.body, \
                 fetched_at = excluded.fetched_at",
                params![owner, path, body, now_ms()],
            )?;
            cache.prune_bodies(conn)
        })
    }

    fn drop_body(&self, owner: &str, path: &str) -> crate::Result<()> {
        self.write(|cache, conn| {
            cache.exec(
                conn,
                "DELETE FROM api_cache WHERE owner = ?1 AND path = ?2",
                params![owner, path],
            )?;
            Ok(())
        })
    }

    fn take_owner_rows(&self, owner: &str) -> crate::Result<usize> {
        self.write(|cache, conn| {
            cache.exec(
                conn,
                "DELETE FROM api_cache WHERE owner = ?1",
                params![owner],
            )
        })
    }

    fn count_bodies(&self) -> crate::Result<i64> {
        self.read(|conn| self.query(conn, "SELECT COUNT(*) FROM api_cache", [], |row| row.get(0)))
    }

    /// Trims the cache back to [`MAX_ENTRIES`], oldest first.
    ///
    /// One `COUNT(*)` in the usual case (the `(fetched_at)` index makes it
    /// cheap), and the delete only runs when something is actually over.
    fn prune_bodies(&self, conn: &rusqlite::Connection) -> crate::Result<()> {
        let count: i64 =
            self.query(conn, "SELECT COUNT(*) FROM api_cache", [], |row| row.get(0))?;
        let excess = count - MAX_ENTRIES;
        if excess <= 0 {
            return Ok(());
        }
        self.exec(
            conn,
            "DELETE FROM api_cache WHERE rowid IN (\
               SELECT rowid FROM api_cache ORDER BY fetched_at ASC, rowid ASC LIMIT ?1)",
            params![excess],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use crate::Result;
    use crate::testing::TempDir;

    /// The same table `webapp/src/lib/apicache.js` answers: the paths the page
    /// caches offline, and the ones it deliberately refuses.
    #[test]
    fn the_cacheable_paths_are_the_pages_own_table() {
        let cacheable = [
            "/me/playlists",
            "/me/playlists?page=2",
            "/me/favorites",
            "/me/favorite-ids",
            "/me/local",
            "/playlist/abc",
            "/playlist/abc?x=1",
            "/album/2000000",
            "/artist/42",
            "/moods",
            "/moods/abc",
            "/moods/abc/tracks",
            "/lyrics/3135556",
            "/credits/person/xyz",
            "/track/3135556/credits",
        ];
        for path in cacheable {
            assert!(super::is_cacheable(path), "{path} must be cached");
        }

        let live = [
            "/me",
            "/search?q=kaya",
            "/flow",
            "/releases",
            "/podcast/progress",
            "/playlist/abc/version",
            "/playlist/abc/version?since=3",
            "/admin/users",
            "/admin",
            "/invites/new",
            "/onboarding/artists",
            "/podcast/episode/e-1/chapters",
            "/track/3135556",
            "/track/3135556/credits/x",
            "/track//credits",
        ];
        for path in live {
            assert!(!super::is_cacheable(path), "{path} must NOT be cached");
        }
    }

    #[test]
    fn a_body_comes_back_byte_for_byte() -> Result<()> {
        let dir = TempDir::new("api-roundtrip");
        let cache = dir.open();
        let body = r#"{"playlists":[{"id":"a","name":"Soirée — été \"26\""}],"n":1}"#;

        cache.put("42", "/me/playlists", body);
        assert_eq!(cache.get("42", "/me/playlists").as_deref(), Some(body));
        assert!(
            cache
                .cached_at("42", "/me/playlists")
                .is_some_and(|at| at > 0)
        );
        Ok(())
    }

    #[test]
    fn two_accounts_never_read_each_others_copy() -> Result<()> {
        let dir = TempDir::new("api-owners");
        let cache = dir.open();

        cache.put("42", "/me/favorites", r#"["mine"]"#);
        cache.put("7", "/me/favorites", r#"["theirs"]"#);

        assert_eq!(
            cache.get("42", "/me/favorites").as_deref(),
            Some(r#"["mine"]"#)
        );
        assert_eq!(
            cache.get("7", "/me/favorites").as_deref(),
            Some(r#"["theirs"]"#)
        );

        assert_eq!(cache.purge_owner("7")?, 1);
        assert_eq!(cache.get("7", "/me/favorites"), None);
        assert!(cache.get("42", "/me/favorites").is_some());
        Ok(())
    }

    /// A live endpoint cannot be stored even if a caller insists, and cannot be
    /// read back: the filter sits on both sides.
    #[test]
    fn a_live_path_is_never_stored() -> Result<()> {
        let dir = TempDir::new("api-live");
        let cache = dir.open();

        cache.put("42", "/playlist/abc/version", r#"{"v":9}"#);
        assert_eq!(cache.get("42", "/playlist/abc/version"), None);

        let conn = Connection::open(cache.db_path()).expect("second handle");
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM api_cache", [], |row| row.get(0))
            .expect("count");
        assert_eq!(rows, 0);
        Ok(())
    }

    #[test]
    fn delete_removes_one_body() -> Result<()> {
        let dir = TempDir::new("api-delete");
        let cache = dir.open();

        cache.put("42", "/album/a", "{}");
        cache.put("42", "/album/b", "{}");
        cache.delete("42", "/album/a");

        assert_eq!(cache.get("42", "/album/a"), None);
        assert!(cache.get("42", "/album/b").is_some());
        Ok(())
    }

    /// Criterion (rule 3, bounded): past [`MAX_ENTRIES`] the OLDEST go, so a
    /// long listening session cannot grow the file without bound.
    #[test]
    fn the_cache_keeps_the_newest_400_entries() -> Result<()> {
        let dir = TempDir::new("api-prune");
        let cache = dir.open();

        for n in 0..(super::MAX_ENTRIES + 5) {
            cache.put("42", &format!("/album/{n}"), "{}");
        }

        assert_eq!(cache.cached_entries()?, super::MAX_ENTRIES);
        for gone in 0..5 {
            assert_eq!(
                cache.get("42", &format!("/album/{gone}")),
                None,
                "entry {gone}"
            );
        }
        assert!(cache.get("42", "/album/5").is_some());
        assert!(
            cache
                .get("42", &format!("/album/{}", super::MAX_ENTRIES + 4))
                .is_some()
        );
        Ok(())
    }

    /// A put is one `INSERT`/`UPSERT` plus the cheap `COUNT(*)` guard; the
    /// delete only appears once the cache is over its cap.
    #[test]
    fn a_put_under_the_cap_is_two_statements() -> Result<()> {
        let dir = TempDir::new("api-statements");
        let cache = dir.open();

        let before = cache.statements();
        cache.put("42", "/me/playlists", "[]");
        assert_eq!(cache.statements() - before, 2);
        Ok(())
    }
}
