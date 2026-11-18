//! What this node has: the complete files of `nz-cache` (the person's downloads,
//! then complete play-cache files), per quality, with a SHA-256, plus the download
//! sets they belong to (docs/agents/lan-nodes.md, *Protocol messages*).
//!
//! **LN0 declares `Index` and `LocalFile`; LN4 fills them.**
//!
//! The index is in memory and re-read from `nz-cache` on [`Index::refresh`]; a
//! change bumps [`Index::version`], and the peers that already hold a replica hear
//! it through [`Index::events`] and ask for a delta. Nothing here is on a screen's
//! path: the version is what the snapshot reads.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::watch;

use crate::proto;
use crate::scope::Scope;
use crate::store::Store;

/// One complete local file a peer may fetch: where it is, at which quality, how
/// long, and its tag once hashing has caught up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFile {
    /// The path on THIS device. It never comes from the wire and it never leaves
    /// the device: a peer names a track id and a quality.
    pub path: PathBuf,
    pub quality: nz_api::Quality,
    pub len: u64,
    /// `None` until the hashing worker has hashed this exact `(len, mtime)`.
    pub tag: Option<proto::FileTag>,
}

/// The local inventory, shared by the serving, hashing and requester tasks.
///
/// Cloning is cheap and every clone sees the same versions: it is one `Arc`
/// inside once LN4 is written.
#[derive(Debug, Clone)]
pub struct Index {
    /// The version counter the peers follow, and what `refresh` bumps.
    events: Arc<watch::Sender<u64>>,
}

impl Index {
    /// Builds an index over `cache`'s files for `owner`'s account, persisting the
    /// hashes in `store`.
    ///
    /// `owner` is the account whose copies this node may serve: the index is one
    /// person's library, and leasing it to a remote-control session must not leak
    /// somebody else's files.
    pub fn new(cache: nz_cache::Cache, owner: String, store: Store) -> Index {
        let _ = (cache, owner, store);
        Index {
            events: Arc::new(watch::channel(0).0),
        }
    }

    /// Re-reads `nz-cache` and bumps the version when anything changed. Called on
    /// the hashing worker's schedule, never on a request.
    pub fn refresh(&self) {
        // LN4: the re-read.
    }

    /// The inventory's version: it grows on every change, and a delta is only
    /// ever asked for a version the responder still remembers.
    pub fn version(&self) -> u64 {
        *self.events.borrow()
    }

    /// The download sets' version, which moves independently (a playlist can gain
    /// a track that is already on disk).
    pub fn sets_version(&self) -> u64 {
        0
    }

    /// The version counter, so a peer that hears "your inventory changed" and a
    /// screen that shows a count both follow the same signal.
    pub fn events(&self) -> watch::Receiver<u64> {
        self.events.subscribe()
    }

    /// One page of the inventory, filtered by `scope` — for a friend, only the
    /// tracks they were shared.
    pub fn inventory(&self, scope: &Scope, since: Option<u64>, page: u32) -> proto::Response {
        let _ = (scope, since);
        proto::Response::InventoryFull {
            version: 0,
            page,
            last: true,
            items: Vec::new(),
        }
    }

    /// One page of the download sets whose members are in `scope`.
    pub fn sets(&self, scope: &Scope, page: u32) -> proto::Response {
        let _ = scope;
        proto::Response::Sets {
            version: 0,
            page,
            last: true,
            sets: Vec::new(),
        }
    }

    /// Metadata for at most 200 track ids, only the ones in `scope`.
    pub fn meta(&self, scope: &Scope, tracks: &[String]) -> proto::Response {
        let _ = (scope, tracks);
        proto::Response::Meta { rows: Vec::new() }
    }

    /// The complete local file for `(track, quality)`, if this node has one.
    pub fn local(&self, track: &str, quality: nz_api::Quality) -> Option<LocalFile> {
        let _ = (track, quality);
        None
    }

    /// The cover file for a URL a metadata row named, when a track in `scope`
    /// references it.
    pub fn cover(&self, scope: &Scope, url: &str) -> Option<PathBuf> {
        let _ = (scope, url);
        None
    }

    /// The bytes this index costs, counted (rule: never the RSS of a shared
    /// process). LN4's target is ≤ 64 B per item, so 8 000 tracks ≈ 512 KB.
    pub fn memory(&self) -> usize {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stub index is empty and says so in the one shape a peer can read: a full
    /// answer with no items and `last = true`, never a page a peer waits for more
    /// of.
    #[test]
    fn an_empty_index_answers_one_last_empty_page() {
        let dir = std::env::temp_dir().join(format!("nz-lan-index-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let cache = nz_cache::Cache::open_with_cache_dir(&dir, dir.join("cache")).expect("cache");
        let index = Index::new(cache, "1".to_owned(), Store);

        match index.inventory(&Scope::Own, None, 0) {
            proto::Response::InventoryFull { last, items, .. } => {
                assert!(last, "one page, and it is the last");
                assert!(items.is_empty());
            }
            other => panic!("unexpected answer: {other:?}"),
        }
        assert_eq!(index.version(), 0);
        assert_eq!(index.sets_version(), 0);
        assert_eq!(index.memory(), 0);
        assert_eq!(index.local("3135556", nz_api::Quality::Flac), None);
        assert_eq!(index.cover(&Scope::Own, "/api/cover/1"), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
