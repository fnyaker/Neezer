//! What the device keeps of the podcasts between two visits, and the only place
//! that turns a key into a file name.
//!
//! The SPA keeps these bodies in `lib/apicache.js` (`/podcasts` is listed; a
//! show page is re-read on every visit) and keeps the transcripts in IndexedDB
//! (`lib/transcriptcache.js`). The phone's copy plays the same role one level
//! up: `cached_x()` answers from disk with **zero requests**, the screen paints
//! it, and the fresh read replaces it (`screenFlow`, MB2). Three decisions, the
//! same three `library/store.rs` and `entity/store.rs` took, for the same
//! reasons:
//!
//! - **the body is stored RAW** (the screen-shaped record, serialised), so a
//!   cache hit and a live answer come out of the SAME converter;
//! - **the file name is a hash pair, never a server string**: the account and
//!   the key each go through FNV-1a, so an id from a route argument can never
//!   choose a path;
//! - **a write is `.part` + rename**, so a process killed mid-write leaves the
//!   previous copy rather than half a document (rule 7 for the device's data).
//!
//! A cache that cannot be written never costs the screen: the fresh answer is
//! already in hand and the next visit asks again.

use std::path::PathBuf;

use crate::core::{Core, CoreError};

/// The subscription list (`/api/podcasts`), key `""`.
pub(super) const SHOWS: &str = "shows";
/// One show's page bundle (the header, the loaded episodes and the cursor), key
/// the show id.
pub(super) const SHOW: &str = "show";
/// One episode's chapters, key the episode id.
pub(super) const CHAPTERS: &str = "chapters";
/// One episode's transcript pages, key the episode id.
pub(super) const TRANSCRIPT: &str = "transcript";
/// One show's markers by episode, key the show id.
pub(super) const MARKERS: &str = "markers";
/// The server's copy of this person's listening positions, key `""`.
pub(super) const PROGRESS: &str = "progress";

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
            tracing::debug!(%cause, "a podcasts cache file was not written");
            Err(cause)
        }
    }
}

/// Forgets one answer: what a mutation does to the thing it changed, so a later
/// read cannot paint the pre-mutation copy from disk.
pub(super) fn forget(core: &Core, kind: &str, key: &str) {
    let _ = std::fs::remove_file(path(core, kind, key));
}

/// Where `(kind, key)` lives for the signed-in account.
fn path(core: &Core, kind: &str, key: &str) -> PathBuf {
    core.cache_path(&format!(
        "podcasts-{:016x}-{kind}-{:016x}.json",
        fnv1a(owner(core).as_bytes()),
        fnv1a(key.as_bytes())
    ))
}

/// The files this account holds for `kind` (the transcripts' cache and the
/// chapters' LRU both prune by them).
pub(super) fn files(core: &Core, kind: &str) -> Vec<PathBuf> {
    let directory = core.config().cache_dir.as_path();
    let prefix = format!("podcasts-{:016x}-{kind}-", fnv1a(owner(core).as_bytes()));
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".json"))
        })
        .collect();
    files.sort();
    files
}

/// The path `(kind, key)` lives at — what a prune needs to keep one file.
pub(super) fn path_of(core: &Core, kind: &str, key: &str) -> PathBuf {
    path(core, kind, key)
}

/// The cache belongs to ONE account (`apicache.js#owner`): another person on the
/// same device must not see this one's subscriptions, positions or pages.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn core(tag: &str) -> std::path::PathBuf {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nz-podcast-tests")
            .join(format!("{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        Core::new(crate::core::CoreConfig {
            data_dir: path.join("data"),
            cache_dir: path.join("cache"),
            app_version: "0.0.0-test".to_owned(),
            version_code: 2_000_001,
            sdk_int: 35,
            device_name: "test".to_owned(),
        })
        .expect("a core over a throwaway directory");
        path
    }

    /// A body round-trips, and a kind and a key are each part of the name.
    #[test]
    fn a_body_is_kept_and_read_back() {
        let dir = core("roundtrip");
        let core = Core::new(crate::core::CoreConfig {
            data_dir: dir.join("data"),
            cache_dir: dir.join("cache"),
            app_version: "0.0.0-test".to_owned(),
            version_code: 2_000_001,
            sdk_int: 35,
            device_name: "test".to_owned(),
        })
        .expect("core");
        assert!(read(&core, SHOWS, "").is_none());
        write(&core, SHOWS, "", r#"{"shows":[]}"#).expect("written");
        assert_eq!(read(&core, SHOWS, "").as_deref(), Some(r#"{"shows":[]}"#));
        assert!(read(&core, SHOW, "x").is_none(), "a kind is its own key");
        write(&core, SHOW, "x", "{}").expect("written");
        assert!(read(&core, SHOW, "y").is_none(), "so is a key");
        forget(&core, SHOW, "x");
        assert!(read(&core, SHOW, "x").is_none());
        assert!(read(&core, SHOWS, "").is_some(), "the others stay");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A hostile key cannot choose a path: the name is a hash of it.
    #[test]
    fn a_hostile_key_cannot_choose_a_path() {
        let dir = core("hostile");
        let core = Core::new(crate::core::CoreConfig {
            data_dir: dir.join("data"),
            cache_dir: dir.join("cache"),
            app_version: "0.0.0-test".to_owned(),
            version_code: 2_000_001,
            sdk_int: 35,
            device_name: "test".to_owned(),
        })
        .expect("core");
        let key = "../../../etc/passwd\n42";
        write(&core, SHOW, key, "{}").expect("written");
        let file = path(&core, SHOW, key);
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
        assert_eq!(read(&core, SHOW, key).as_deref(), Some("{}"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
