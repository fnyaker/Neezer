//! What the device keeps of an artist, an album or a credit page between two
//! visits: the server's own answer, as it arrived.
//!
//! The SPA keeps exactly this (`lib/apicache.js`: `/artist/`, `/album/`,
//! `/credits/person` and `/track/<id>/credits` are cacheable paths) and paints
//! it before revalidating, which is what makes a revisit feel instant on a bad
//! link. The phone's copy plays the same role one level up: `cached_artist()` and
//! its siblings answer from disk with **zero requests**, the screen paints that,
//! and the fresh read replaces it (`screenFlow`, MB2).
//!
//! Three decisions:
//!
//! - **The body is stored RAW.** Storing the assembled record would need a
//!   second, serialisable copy of every screen type (`nz_core::lists::Row` has no
//!   serde), and the two would drift. A raw body goes through the SAME converter
//!   on the way in and on the way out, so a cache hit is proven by the reader.
//! - **The file name is built from hashes, never from a server string.** A route
//!   argument is a UUID or a provider id, but `person-credits` is keyed by a
//!   NAME: a name with `/`, `..` or a newline in it must not be able to choose a
//!   path. `<account>-<kind>-<fnv1a(key)>.json` under the CACHE directory (the
//!   system may clear it — it is a copy, never the truth).
//! - **A write is `.part` + rename**, so a process killed mid-write leaves the
//!   previous copy rather than half a document (rule 7: the device's own data).
//!
//! A cache that cannot be written or read never costs the screen: the fresh
//! answer is already in hand, and the next visit asks again.

use std::path::PathBuf;

use crate::core::{Core, CoreError};

/// The kinds of answer this area keeps. Literals, so a kind can never carry a
/// path separator into a file name.
pub(super) const ARTIST: &str = "artist";
pub(super) const DISCOGRAPHY: &str = "discography";
pub(super) const ARTIST_ON_DEEZER: &str = "artist-on-deezer";
pub(super) const ARTIST_TRACKS: &str = "artist-tracks";
pub(super) const ALBUM: &str = "album";
pub(super) const ALBUM_ON_DEEZER: &str = "album-on-deezer";
pub(super) const TRACK_CREDITS: &str = "track-credits";
pub(super) const PERSON_CREDITS: &str = "person-credits";

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
            tracing::debug!(%cause, "an entity cache file was not written");
            Err(cause)
        }
    }
}

/// Where `(kind, key)` lives for the signed-in account.
fn path(core: &Core, kind: &str, key: &str) -> PathBuf {
    core.cache_path(&format!(
        "entity-{:016x}-{kind}-{:016x}.json",
        fnv1a(owner(core).as_bytes()),
        fnv1a(key.as_bytes())
    ))
}

/// The cache belongs to ONE account (`apicache.js#owner`): another person on the
/// same device must not see this one's pages.
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

/// The same path, for an account chosen by the caller — the account test's door,
/// and a reminder that the owner is part of the key.
#[cfg(test)]
fn path_for(core: &Core, account: &str, kind: &str, key: &str) -> PathBuf {
    core.cache_path(&format!(
        "entity-{:016x}-{kind}-{:016x}.json",
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
            .join("../../target/nz-entity-tests")
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

    /// A body round-trips, and a second read of the same key is the same text.
    #[test]
    fn a_body_is_kept_and_read_back() {
        let (path, core) = core("roundtrip");
        assert!(
            read(&core, ARTIST, "42").is_none(),
            "nothing the first time"
        );
        write(&core, ARTIST, "42", r#"{"artist":{"name":"Nina"}}"#).expect("written");
        assert_eq!(
            read(&core, ARTIST, "42").as_deref(),
            Some(r#"{"artist":{"name":"Nina"}}"#)
        );
        // A different key is a different file.
        assert!(read(&core, ARTIST, "43").is_none());
        // And a different KIND never collides with the same key.
        assert!(read(&core, ALBUM, "42").is_none());
        let _ = std::fs::remove_dir_all(path);
    }

    /// A name that would choose a path if it were used literally cannot escape
    /// the cache directory: the file name is a hash.
    #[test]
    fn a_hostile_name_cannot_choose_a_path() {
        let (dir, core) = core("hostile");
        let name = "../../../etc/passwd\n42";
        write(&core, PERSON_CREDITS, name, "{}").expect("written");
        let file = path(&core, PERSON_CREDITS, name);
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
        assert_eq!(
            read(&core, PERSON_CREDITS, "../../../etc/passwd\n42").as_deref(),
            Some("{}")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Two accounts never read each other's copy.
    #[test]
    fn the_copy_is_per_account() {
        let (dir, core) = core("account");
        let one = path(&core, ARTIST, "42");
        let other = crate::entity::store::path_for(&core, "someone-else", ARTIST, "42");
        assert_ne!(one, other, "the account hashes into the name");
        let _ = std::fs::remove_dir_all(dir);
    }
}
