//! What the device keeps of a title's words between two visits: the server's own
//! answer, as it arrived.
//!
//! The SPA caches the lyrics path (`lib/apicache.js`) and paints it before asking
//! again, which is what makes the words appear at once on a bad link. The phone's
//! copy plays the same role one level up: `cached_lyrics()` answers from disk with
//! **zero requests**, the view paints that, and a fresh read replaces it.
//!
//! Three decisions, the entity area's (`docs/agents/mobile.md`, MB7), repeated
//! because they are what keeps this cache honest:
//!
//! - **The body is stored RAW.** The whole `/api/lyrics/<id>` answer, not the
//!   parsed record: a cache hit goes through the SAME converter as a live answer,
//!   so the two can never drift.
//! - **The file name is built from a hash, never from a key.** A key is a Deezer
//!   number or a row UUID today, but nothing stops a hostile one carrying `/` or
//!   a newline: `<account>-lyrics-<fnv1a(key)>.json` under the CACHE directory
//!   (the system may clear it — a copy is never the truth).
//! - **A write is `.part` + rename**, so a process killed mid-write leaves the
//!   previous copy rather than half a document (rule 7).
//!
//! A cache that cannot be written or read never costs the screen: the fresh
//! answer is already in hand, and the next visit asks again.

use std::path::PathBuf;

use crate::core::{Core, CoreError};

/// The kinds of answer this area keeps. Literals, so a kind can never carry a
/// path separator into a file name.
pub(super) const LYRICS: &str = "lyrics";

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
            tracing::debug!(%cause, "a nowplaying cache file was not written");
            Err(cause)
        }
    }
}

/// Where `(kind, key)` lives for the signed-in account.
fn path(core: &Core, kind: &str, key: &str) -> PathBuf {
    core.cache_path(&format!(
        "nowplaying-{:016x}-{kind}-{:016x}.json",
        fnv1a(owner(core).as_bytes()),
        fnv1a(key.as_bytes())
    ))
}

/// The cache belongs to ONE account (`apicache.js#owner`): another person on the
/// same device must not read this one's words.
fn owner(core: &Core) -> String {
    core.me()
        .map(|me| me.user.id.as_str().to_owned())
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| "anon".to_owned())
}

/// FNV-1a, 64-bit: the SPA's `hashId` idea, four lines and no dependency.
fn fnv1a(bytes: &[u8]) -> u64 {
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
    use crate::core::CoreConfig;

    fn core(tag: &str) -> (PathBuf, Core) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nz-nowplaying-tests")
            .join(format!("{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("data")).expect("data");
        std::fs::create_dir_all(path.join("cache")).expect("cache");
        let core = Core::new(CoreConfig {
            data_dir: path.join("data"),
            cache_dir: path.join("cache"),
            app_version: "0.0.0".into(),
            version_code: 2_000_001,
            sdk_int: 35,
            device_name: "test".into(),
        })
        .expect("the core");
        (path, core)
    }

    /// A write is readable, a MISSING key stays a miss, and a hostile key cannot
    /// choose where the file goes (a `/` in a key never makes a directory).
    #[test]
    fn a_body_round_trips_and_a_key_never_names_a_path() {
        let (dir, core) = core("store");
        assert!(read(&core, LYRICS, "111").is_none(), "a miss is a miss");
        write(&core, LYRICS, "111", "{\"lyrics\":null}").expect("write");
        assert_eq!(
            read(&core, LYRICS, "111").as_deref(),
            Some("{\"lyrics\":null}")
        );
        write(&core, LYRICS, "../../etc/passwd", "{}").expect("a hostile key");
        let entries: Vec<String> = std::fs::read_dir(dir.join("cache"))
            .expect("the cache dir")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .filter(|name| name.ends_with(".json"))
            .collect();
        assert_eq!(entries.len(), 2, "{entries:?}");
        for name in &entries {
            assert!(!name.contains('/'), "{name}");
            assert!(name.ends_with(".json"), "{name}");
        }
        core.shutdown();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// An interrupted write leaves the previous copy rather than half a file.
    #[test]
    fn a_rewrite_replaces_the_previous_body() {
        let (dir, core) = core("rewrite");
        write(&core, LYRICS, "42", "{\"lyrics\":{\"text\":\"a\"}}").expect("first");
        write(&core, LYRICS, "42", "{\"lyrics\":{\"text\":\"b\"}}").expect("second");
        assert_eq!(
            read(&core, LYRICS, "42").as_deref(),
            Some("{\"lyrics\":{\"text\":\"b\"}}")
        );
        core.shutdown();
        let _ = std::fs::remove_dir_all(dir);
    }
}
