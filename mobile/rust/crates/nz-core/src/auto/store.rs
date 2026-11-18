//! The device's copy of one node of the car's tree: the small, screen-shaped
//! answer, kept in the CACHE directory (`cacheDir/neezer/auto/`).
//!
//! Three decisions, the same three MB8's `library/store.rs` took:
//!
//! - **The parsed, capped rows are stored, not the server's body.** A node never
//!   holds more than [`super::MAX_CHILDREN`] rows, so a warm browse parses a
//!   couple of hundred bytes per row on the session's thread instead of an
//!   eight-thousand-title body — a JSON parse is the one thing that could cost
//!   a frame here (rule 4).
//! - **The file name is two hashes, never a server string.** The account and the
//!   media id both go through FNV-1a, so a hostile id can never choose a path.
//! - **A write is `.part` + rename**, so a process killed mid-write leaves the
//!   previous copy rather than half a document.
//!
//! A copy that cannot be written or read never costs the car: the fresh answer
//! is already in hand, and the next browse asks again.

use std::path::PathBuf;
use std::time::Duration;

use super::Item;
use crate::core::{Core, CoreError};

/// The directory under the cache root. Evictable by definition: every node can
/// be fetched again, and the system may clear `cacheDir`.
const DIR: &str = "auto";

fn path(core: &Core, owner: &str, parent: &str) -> PathBuf {
    core.cache_path(DIR).join(format!(
        "{:016x}-{:016x}.json",
        fnv1a(owner.as_bytes()),
        fnv1a(parent.as_bytes())
    ))
}

/// The node's copy, whatever its age, or `None` the first time.
pub(super) fn read(core: &Core, owner: &str, parent: &str) -> Option<Vec<Item>> {
    let body = std::fs::read(path(core, owner, parent)).ok()?;
    let items: Vec<Item> = serde_json::from_slice(&body).ok()?;
    (!items.is_empty()).then_some(items)
}

/// The node's copy while it is younger than `ttl` — the "0 requests" of a warm
/// browse. An unreadable timestamp counts as stale: asking once is cheaper than
/// a list that never learns.
pub(super) fn read_if_fresh(
    core: &Core,
    owner: &str,
    parent: &str,
    ttl: Duration,
) -> Option<Vec<Item>> {
    let path = path(core, owner, parent);
    let written = std::fs::metadata(&path).ok()?.modified().ok()?;
    let age = std::time::SystemTime::now().duration_since(written).ok()?;
    if age > ttl {
        return None;
    }
    let body = std::fs::read(&path).ok()?;
    let items: Vec<Item> = serde_json::from_slice(&body).ok()?;
    (!items.is_empty()).then_some(items)
}

/// Keeps `items` as the answer for `(owner, parent)`.
pub(super) fn write(
    core: &Core,
    owner: &str,
    parent: &str,
    items: &[Item],
) -> Result<(), CoreError> {
    let path = path(core, owner, parent);
    if let Some(parent) = path.parent() {
        core.ensure_dir(parent)?;
    }
    let temporary = path.with_extension("part");
    let inner = || -> Result<(), CoreError> {
        let body = serde_json::to_vec(items).map_err(|error| CoreError::Internal {
            detail: format!("nœud Auto {parent} : {error}"),
        })?;
        std::fs::write(&temporary, &body).map_err(|error| CoreError::Internal {
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
            tracing::debug!(%cause, "un nœud Auto n'a pas été enregistré");
            Err(cause)
        }
    }
}

/// FNV-1a, 64-bit — the same four lines `library/store.rs` and
/// `entity/store.rs` use, and for the same reason: a name that is a hash cannot
/// carry a path separator.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}
