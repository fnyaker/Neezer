//! What is worth keeping on the device, and who keeps it.
//!
//! The trait lives here so `nz-api` never depends on SQLite: `nz-cache` (D3)
//! implements it over the `api_cache` table, and a test can implement it in
//! memory. The rule is the web app's `lib/apicache.js#isCacheable` — same
//! prefixes, same live-path exclusions — ported rather than re-derived, so both
//! clients keep the same screens browsable offline for the same paths.

/// The whole offline copy of the API's GET answers: response TEXT, keyed by the
/// signed-in account's id and the request path.
///
/// Text, not parsed values: `lib/apicache.js#cachePut` stores the response's
/// own text, so a fixture and a live answer cannot disagree about a field the
/// client's type ignores. Calls are made from the network thread (rule 3), so a
/// SQLite-backed implementation may block briefly; it must never panic.
pub trait ResponseCache: Send + Sync {
    /// The stored body for this account and path, or `None`.
    fn get(&self, owner: &str, path: &str) -> Option<String>;
    /// Store (or replace) the body.
    fn put(&self, owner: &str, path: &str, body: &str);
    /// Forget one path (a `401`/`403`/`404` answer, or a body that no longer
    /// decodes).
    fn delete(&self, owner: &str, path: &str);
}

/// A cache that keeps nothing: what a test, or a client without a database
/// file yet, gets.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoCache;

impl ResponseCache for NoCache {
    fn get(&self, _owner: &str, _path: &str) -> Option<String> {
        None
    }
    fn put(&self, _owner: &str, _path: &str, _body: &str) {}
    fn delete(&self, _owner: &str, _path: &str) {}
}

/// Paths whose answer is worth having offline — `lib/apicache.js`'s
/// `CACHE_PREFIXES`, in the same order, plus the one regex for a track's
/// credits.
const CACHE_PREFIXES: [&str; 10] = [
    "/me/playlists",
    "/me/favorites",
    "/me/favorite-ids",
    "/me/local",
    "/playlist/",
    "/album/",
    "/artist/",
    // Built from the measured library and slow to change: the Home row and a
    // mood page paint offline.
    "/moods",
    // Asked again on every track change: caching them makes Paroles work
    // offline.
    "/lyrics/",
    // A person's credits page.
    "/credits/person",
];

/// Would `path` be kept on disk? The SPA's answer, path for path.
pub fn is_cacheable(path: &str) -> bool {
    let path = path.split('?').next().unwrap_or(path);
    // Live or personal endpoints that only look content-shaped.
    if is_live(path) {
        return false;
    }
    if is_track_credits(path) {
        return true;
    }
    CACHE_PREFIXES
        .iter()
        .any(|prefix| path == *prefix || path.starts_with(prefix))
}

/// `/playlist/<id>/version` is a poll; the admin/invite/onboarding surfaces and
/// an episode's chapters change under the client's feet. Never served off disk.
fn is_live(path: &str) -> bool {
    is_version_probe(path)
        || starts_with_segment(path, "/admin")
        || starts_with_segment(path, "/invites")
        || starts_with_segment(path, "/onboarding")
        || is_episode_chapters(path)
}

fn starts_with_segment(path: &str, prefix: &str) -> bool {
    path == prefix || path.starts_with(&format!("{prefix}/"))
}

/// `^/playlist/[^/]+/version$`, without a regex dependency.
fn is_version_probe(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("/playlist/") else {
        return false;
    };
    let mut parts = rest.split('/');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(id), Some("version"), None, None) if !id.is_empty()
    )
}

/// `^/podcast/episode/[^/]+/chapters$`.
fn is_episode_chapters(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("/podcast/episode/") else {
        return false;
    };
    let mut parts = rest.split('/');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(id), Some("chapters"), None, None) if !id.is_empty()
    )
}

/// `^/track/[^/]+/credits$`: small, asked again on every visit.
fn is_track_credits(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("/track/") else {
        return false;
    };
    let mut parts = rest.split('/');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(id), Some("credits"), None, None) if !id.is_empty()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cases `lib/apicache.js` accepts, and the ones it refuses. Each one
    /// was read off the SPA's own lists (`CACHE_PREFIXES`, `LIVE_PATHS`,
    /// `CREDITS`), because a desktop that caches a different set either shows a
    /// stale admin screen or an empty playlist on a dead link.
    #[test]
    fn the_cacheable_set_is_the_web_apps() {
        for path in [
            "/me/playlists",
            "/me/playlists?x=1",
            "/me/favorites",
            "/me/favorite-ids",
            "/me/local",
            "/playlist/8f14-2b",
            "/playlist/8f14-2b?version=3",
            "/album/12345",
            "/artist/6789",
            "/artist/6789/tracks",
            "/moods",
            "/moods/energie",
            "/lyrics/3000168",
            "/credits/person?name=Camille",
            "/track/3000168/credits",
        ] {
            assert!(is_cacheable(path), "{path} should be cached");
        }

        for path in [
            "/me",
            "/me/accounts",
            "/playlist/8f14-2b/version",
            "/admin/users",
            "/admin/invites?limit=5",
            "/invites/inspect",
            "/onboarding",
            "/onboarding/artists?limit=36",
            "/podcast/episode/e1/chapters",
            "/track/3000168/probe",
            "/stream/3000168/ready?q=OPUS_128",
        ] {
            assert!(!is_cacheable(path), "{path} must never come off disk");
        }
    }

    #[test]
    fn the_no_cache_implementation_keeps_nothing() {
        let cache = NoCache;
        cache.put("u1", "/me/playlists", "{}");
        assert_eq!(cache.get("u1", "/me/playlists"), None);
    }
}
