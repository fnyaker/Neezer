//! What the device keeps of the library between two visits: the server's own
//! answer to each of the lists, as it arrived.
//!
//! The SPA keeps exactly these bodies (`lib/apicache.js`'s `CACHE_PREFIXES`:
//! `/me/playlists`, `/me/favorites`, `/me/favorite-ids` and `/me/local` are all
//! listed) and paints them before revalidating, which is what makes a revisit
//! feel instant on a bad link. The phone's copy plays the same role one level
//! up: `cached_favorites()` and its siblings answer from disk with **zero
//! requests**, the screen paints that, and the fresh read replaces it
//! (`screenFlow`, MB2).
//!
//! Why a file and not `nz-api`'s own `ResponseCache` (which MB2 already wires to
//! `nz-cache`): that cache is served only when the link is dead, and the phone
//! must paint its copy while the link is ALIVE and the request is still in
//! flight — the whole point of a warm open that costs no blocking request. The
//! two therefore keep the same bodies for the same paths, and the SQLite one
//! stays the safety net `nz-api` owns.
//!
//! Three decisions, the same three `entity/store.rs` took (and for the same
//! reasons):
//!
//! - **The body is stored RAW.** The screen-shaped rows are assembled from it by
//!   the same converter on the way in and on the way out, so a cache hit cannot
//!   drift from a live answer.
//! - **The file name is a hash pair, never a server string.** `<account>` and
//!   the list key both go through FNV-1a: a route argument or a filter can never
//!   choose a path.
//! - **A write is `.part` + rename**, so a process killed mid-write leaves the
//!   previous copy rather than half a document (rule 7: the device's own data).
//!
//! A cache that cannot be written or read never costs the screen: the fresh
//! answer is already in hand, and the next visit asks again.

use std::path::PathBuf;

use crate::core::{Core, CoreError};

/// The kinds of answer this area keeps. Literals, so a kind can never carry a
/// path separator into a file name.
pub(super) const PLAYLISTS: &str = "playlists";
pub(super) const FAVORITES: &str = "favorites";
pub(super) const FAVORITE_IDS: &str = "favorite-ids";
pub(super) const ARTISTS: &str = "artists";
pub(super) const ALBUMS: &str = "albums";
pub(super) const LOCAL: &str = "local";
pub(super) const DISLIKES: &str = "dislikes";
pub(super) const UNAVAILABLE: &str = "unavailable";
pub(super) const UPLOAD_USAGE: &str = "upload-usage";
pub(super) const HISTORY: &str = "history";

/// The stored body for `(kind, key)`, or `None` the first time.
pub(super) fn read(core: &Core, kind: &str, key: &str) -> Option<String> {
    std::fs::read_to_string(path(core, kind, key)).ok()
}

/// Keeps `body` as the answer for `(kind, key)`.
pub(super) fn write(core: &Core, kind: &str, key: &str, body: &str) -> Result<(), CoreError> {
    let path = path(core, kind, key);
    let temporary = path.with_extension("part");
    let inner = || -> Result<(), CoreError> {
        std::fs::write(&temporary, body).map_err(|error| CoreError::Internal {
            detail: format!("{} : {error}", temporary.display()),
        })?;
        std::fs::rename(&temporary, &path).map_err(|error| CoreError::Internal {
            detail: format!("{} : {error}", path.display()),
        })?;
        Ok(())
    };
    match inner() {
        Ok(()) => Ok(()),
        Err(cause) => {
            let _ = std::fs::remove_file(&temporary);
            tracing::debug!(%cause, "a library cache file was not written");
            Err(cause)
        }
    }
}

/// Forgets one answer: what a mutation does to the list it just changed, so a
/// later read cannot paint the pre-mutation tree from disk.
pub(super) fn forget(core: &Core, kind: &str, key: &str) {
    let _ = std::fs::remove_file(path(core, kind, key));
}

/// Where `(kind, key)` lives for the signed-in account.
fn path(core: &Core, kind: &str, key: &str) -> PathBuf {
    core.cache_path(&format!(
        "library-{:016x}-{kind}-{:016x}.json",
        fnv1a(owner(core).as_bytes()),
        fnv1a(key.as_bytes())
    ))
}

/// The cache belongs to ONE account (`apicache.js#owner`): another person on the
/// same device must not see this one's lists.
fn owner(core: &Core) -> String {
    core.me()
        .map(|me| me.user.id.as_str().to_owned())
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| "anon".to_owned())
}

/// FNV-1a, 64-bit: the SPA's `hashId` idea, four lines and no dependency.
pub(super) fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// The same path, for an account chosen by the caller: the account test's own
/// door, and a reminder that the owner is part of the key.
#[cfg(test)]
fn path_for(core: &Core, account: &str, kind: &str, key: &str) -> PathBuf {
    core.cache_path(&format!(
        "library-{:016x}-{kind}-{:016x}.json",
        fnv1a(account.as_bytes()),
        fnv1a(key.as_bytes())
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::CoreConfig;

    fn core(tag: &str) -> (PathBuf, Core) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nz-library-tests")
            .join(format!("{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let core = Core::new(CoreConfig {
            data_dir: path.join("data"),
            cache_dir: path.join("cache"),
            app_version: "0.0.0-test".to_owned(),
            version_code: 2_000_001,
            sdk_int: 35,
            device_name: "test".to_owned(),
        })
        .expect("a core over a throwaway directory");
        (path, core)
    }

    /// A body round-trips; a different key and a different kind are different
    /// files, so one list can never paint another's rows.
    #[test]
    fn a_body_is_kept_and_read_back() {
        let (path, core) = core("roundtrip");
        assert!(
            read(&core, FAVORITES, "").is_none(),
            "nothing the first time"
        );
        write(&core, FAVORITES, "", r#"{"tracks":[{"title":"Nina"}]}"#).expect("written");
        assert_eq!(
            read(&core, FAVORITES, "").as_deref(),
            Some(r#"{"tracks":[{"title":"Nina"}]}"#)
        );
        assert!(
            read(&core, PLAYLISTS, "").is_none(),
            "a kind is its own key"
        );
        // The history's filters are keys of their own.
        write(&core, HISTORY, "listen", r#"{"events":[]}"#).expect("written");
        assert!(read(&core, HISTORY, "skip").is_none());
        assert!(read(&core, HISTORY, "listen").is_some());
        // And a mutation can forget exactly one answer.
        forget(&core, FAVORITES, "");
        assert!(read(&core, FAVORITES, "").is_none());
        assert!(read(&core, HISTORY, "listen").is_some(), "the others stay");
        let _ = std::fs::remove_dir_all(path);
    }

    /// A key that would choose a path if it were used literally cannot escape
    /// the cache directory: the file name is a hash.
    #[test]
    fn a_hostile_key_cannot_choose_a_path() {
        let (dir, core) = core("hostile");
        let key = "../../../etc/passwd\n42";
        write(&core, HISTORY, key, "{}").expect("written");
        let file = path(&core, HISTORY, key);
        assert!(
            file.starts_with(core.config().cache_dir.as_path()),
            "{} stays under the cache directory",
            file.display()
        );
        let name = file.file_name().and_then(std::ffi::OsStr::to_str).unwrap();
        assert!(
            name.chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.'),
            "a file name of hex and dashes: {name}"
        );
        assert_eq!(read(&core, HISTORY, key).as_deref(), Some("{}"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Two accounts never read each other's copy — the SPA's `apicache.js#owner`.
    #[test]
    fn the_copy_is_per_account() {
        let (dir, core) = core("account");
        let one = path(&core, FAVORITES, "");
        let other = path_for(&core, "someone-else", FAVORITES, "");
        assert_ne!(one, other, "the account hashes into the name");
        let _ = std::fs::remove_dir_all(dir);
    }
}
