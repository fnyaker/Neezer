//! What the track actions keep between two openings: the picker's playlist list,
//! and the last playlist a title was added to.
//!
//! The SPA keeps both in memory for the session (`stores.js#playlists`,
//! `#lastPlaylist`); the phone keeps them on the device as well, for one measured
//! reason: the picker is opened to ADD a title, often while the link is bad, and
//! a picker that has to fetch first would have nothing to add to. It is the same
//! rule as every screen of this app — "what was fetched once is kept on the
//! device" (rule 3) — applied to a list that costs one request
//! (`GET /me/playlists` counts the tracks of every playlist: not a free call).
//!
//! Two decisions, both inherited from the entity area's store:
//!
//! - **the file name is built from hashes, never from a server string**, and it
//!   is per ACCOUNT: another person on the same device must not see this one's
//!   playlists;
//! - **a write is `.part` + rename**, so a process killed mid-write leaves the
//!   previous copy rather than half a document.
//!
//! The list lives under the CACHE directory (the system may clear it — it is a
//! copy, never the truth); the last playlist lives in the kv store (`nz-cache`),
//! which survives a cache clear, because it is a preference and not a copy.

use std::path::PathBuf;

use crate::core::{Core, CoreError};

/// The kv key of the last playlist a title was added to.
const KEY_LAST_PLAYLIST: &str = "actions.last-playlist";

/// The stored playlist list of the signed-in account, or `None` the first time.
pub(super) fn read_playlists(core: &Core) -> Option<String> {
    std::fs::read_to_string(playlists_path(core)).ok()
}

/// Keeps `body` as this account's playlist list.
pub(super) fn write_playlists(core: &Core, body: &str) -> Result<(), CoreError> {
    let path = playlists_path(core);
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
            tracing::debug!(%cause, "the picker's playlist copy was not written");
            Err(cause)
        }
    }
}

/// The last playlist used, or `None` before the first add.
pub(super) fn read_last_playlist(core: &Core) -> Option<super::LastPlaylist> {
    let raw: String = core.cache().kv().get(KEY_LAST_PLAYLIST)?;
    let parts: Vec<&str> = raw.split('\u{1f}').collect();
    let (id, title, editable) = match parts.as_slice() {
        [id, title, editable] => (*id, *title, *editable),
        _ => return None,
    };
    if id.is_empty() {
        return None;
    }
    Some(super::LastPlaylist {
        id: id.to_owned(),
        title: title.to_owned(),
        editable: !editable.is_empty(),
    })
}

/// Remembers the last playlist used (`lastPlaylist.set`). Best effort: a kv that
/// cannot be written costs the shortcut on the next launch, never the add.
pub(super) fn write_last_playlist(core: &Core, id: &str, title: &str, editable: bool) {
    let raw = format!("{id}\u{1f}{title}\u{1f}{}", if editable { "1" } else { "" });
    if let Err(cause) = core.cache().kv().set(KEY_LAST_PLAYLIST, &raw) {
        tracing::debug!(%cause, "the last playlist was not remembered");
    }
}

/// Where the account's playlist list lives.
fn playlists_path(core: &Core) -> PathBuf {
    core.cache_path(&format!(
        "actions-playlists-{:016x}.json",
        fnv1a(owner(core).as_bytes())
    ))
}

/// The cache belongs to ONE account (`apicache.js#owner`).
fn owner(core: &Core) -> String {
    core.me()
        .map(|me| me.user.id.as_str().to_owned())
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| "anon".to_owned())
}

/// FNV-1a, 64-bit: the SPA's `hashId` idea, four lines and no dependency (the
/// same function the entity store uses — a file name never comes from a string
/// the server chose).
pub(super) fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}
