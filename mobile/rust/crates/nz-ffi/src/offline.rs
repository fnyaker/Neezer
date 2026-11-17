//! Offline: the downloads queue, what is on the device, and its storage use.
//! Owned by **MB15** (the worker itself is SC4's `nz-offline`).
//!
//! The note pins this one's method NAMES (docs/agents/mobile.md, "Screen lots,
//! wave 3": `add_set(kind, id, quality), remove_set, sets(), progress(), pause(),
//! resume(), retry(key), storage(), subscribe(OfflineListener)`); MB0 declared
//! them so `DownloadButton` and Réglages → Téléchargements compile today. Writes
//! answer `Unsupported` — a button that cannot download yet must say so, not
//! pretend.

use crate::errors::NzError;

/// What one download set is doing. [failures] carries the CAUSE of each failure,
/// never a count (rule 2).
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct SetProgress {
    /// `playlist:42`, `album:7`, `show:3` — the group the button was pressed on.
    pub context: String,
    pub done: u32,
    pub total: u32,
    pub running: bool,
    /// One French sentence per distinct failure cause.
    pub failures: Vec<String>,
}

/// Counted bytes of what the device holds (rule 3, *Measures*: never an RSS).
#[derive(uniffi::Record, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StorageReport {
    /// `filesDir`: downloads, never evicted.
    pub downloads_bytes: u64,
    /// `cacheDir`: the play cache, LRU.
    pub cache_bytes: u64,
    pub free_bytes: u64,
}

/// The download queue changed, or a set finished/failed. Pushed on a Rust
/// thread: Kotlin hands it to a Flow and returns at once.
#[uniffi::export(callback_interface)]
pub trait OfflineListener: Send + Sync {
    /// The whole list, cheap: `AddSet`/`RemoveSet`/a finished file.
    fn on_progress(&self, sets: Vec<SetProgress>);
    /// One entry failed; `cause` is French and the queue goes on.
    fn on_failure(&self, key: String, cause: String);
}

/// The downloads API. MB15 fills it; MB0 refuses every write.
#[derive(uniffi::Object)]
pub struct OfflineApi;

#[uniffi::export(async_runtime = "tokio")]
impl OfflineApi {
    /// Queues one context (a playlist, an album, a show) on the `Bulk` lane.
    /// Returns at once: the worker runs in the background and SAYS SO.
    pub async fn add_set(&self, kind: String, id: String, quality: String) -> Result<(), NzError> {
        let _ = (kind, id, quality);
        Err(NzError::Unsupported {
            feature: "mb15:add_set".into(),
        })
    }

    pub async fn remove_set(&self, kind: String, id: String) -> Result<(), NzError> {
        let _ = (kind, id);
        Err(NzError::Unsupported {
            feature: "mb15:remove_set".into(),
        })
    }

    /// Every set the device knows, running first.
    pub fn sets(&self) -> Vec<SetProgress> {
        Vec::new()
    }

    /// One set's progress, or `None` when nothing of it was ever queued.
    pub fn progress(&self, kind: String, id: String) -> Option<SetProgress> {
        let _ = (kind, id);
        None
    }

    /// Pauses the queue; the files already published stay.
    pub async fn pause(&self) -> Result<(), NzError> {
        Err(NzError::Unsupported {
            feature: "mb15:pause".into(),
        })
    }

    /// Resumes from SQLite, never from the start (rule 2).
    pub async fn resume(&self) -> Result<(), NzError> {
        Err(NzError::Unsupported {
            feature: "mb15:resume".into(),
        })
    }

    /// Retries ONE failed entry, with its cause cleared.
    pub async fn retry(&self, key: String) -> Result<(), NzError> {
        let _ = key;
        Err(NzError::Unsupported {
            feature: "mb15:retry".into(),
        })
    }

    pub fn storage(&self) -> StorageReport {
        StorageReport::default()
    }

    pub fn subscribe(&self, listener: Box<dyn OfflineListener>) {
        drop(listener);
    }
}
