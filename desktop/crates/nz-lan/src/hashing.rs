//! The background SHA-256 of the local files, so a peer can be told exactly which
//! bytes it will receive (docs/agents/lan-nodes.md, *Integrity, resume*).
//!
//! **LN0 declares `Hashing`; LN4 fills it.** One thread, named `nz-lan-hash`,
//! reniced **+10** (rule 1: the person's own playback and the serving paths come
//! first), which hashes what the index lists and stops hashing while a `User`
//! request is in flight. It is JOINED by [`Hashing::stop`], and by
//! `Lan::shutdown` after it: a thread that borrows the `nz-lan` runtime must not
//! outlive it.
//!
//! A hash is recomputed only when `(len, mtime)` changed — the store keeps both —
//! so a restart does not re-read a 2 000-track library to learn what it already
//! knew.

use crate::index::Index;
use crate::store::Store;

/// The hashing worker.
///
/// A stub holds nothing: LN4 owns the thread handle (which is why `stop` takes
/// `self` by value — stopping is the last thing a caller does with it).
#[derive(Debug)]
pub struct Hashing;

impl Hashing {
    /// Starts hashing `index`'s files, persisting the tags in `store`.
    pub fn start(index: Index, store: Store) -> Hashing {
        let _ = (index, store);
        Hashing
    }

    /// Stops the worker and JOINS its thread.
    pub fn stop(self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Starting and stopping is all a stub does — and stopping twice is not
    /// possible by construction (`self`), so a shutdown cannot join a thread that
    /// has already been joined.
    #[test]
    fn a_stub_hashing_worker_starts_and_stops() {
        let dir = std::env::temp_dir().join(format!("nz-lan-hash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let cache = nz_cache::Cache::open_with_cache_dir(&dir, dir.join("cache")).expect("cache");
        let worker = Hashing::start(Index::new(cache, "1".to_owned(), Store), Store);
        worker.stop();
        std::fs::remove_dir_all(&dir).ok();
    }
}
