//! Covers and metadata copied from a peer, so a playlist taken on the LAN looks
//! exactly the same offline (docs/agents/lan-nodes.md, *Downloads and playlists*).
//!
//! **LN0 declares `import_for_set` and `ImportReport`; LN11 fills them.**
//!
//! For every set taken from a peer: the tracks whose `TrackMeta` this device does
//! not have are asked with `Meta`, and their covers with `Cover`. It all runs on
//! `Bulk` (rule 1: it yields to the tapped track), covers are stored PINNED
//! (`Covers::put_pinned`, so the LRU never evicts a download's art), and the titles
//! and artists go to `nz-offline` as its `Choice` labels — which is what makes the
//! downloads screen and the playlist page read the same offline.
//!
//! Nothing here writes anything the person did not ask for: a set's rows are
//! already in the downloads queue when this runs.

use crate::service::Lan;
use crate::types::{DevicePub, LanError};

/// What one import brought back, counted (rule 2: the screen says what happened).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// Metadata rows stored.
    pub meta: usize,
    /// Covers fetched and pinned.
    pub covers: usize,
    /// Bytes those covers took.
    pub bytes: u64,
}

/// Brings the missing metadata and covers of `tracks` over from `peer`, for a set
/// already being downloaded.
pub async fn import_for_set(
    lan: &Lan,
    cache: &nz_cache::Cache,
    peer: &DevicePub,
    tracks: &[String],
) -> Result<ImportReport, LanError> {
    let _ = (lan, cache, peer, tracks);
    Err(LanError::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh report counts nothing: a screen that draws it shows "0" rather than
    /// a number the import did not produce.
    #[test]
    fn an_empty_report_counts_nothing() {
        let report = ImportReport::default();
        assert_eq!((report.meta, report.covers, report.bytes), (0, 0, 0));
    }
}
