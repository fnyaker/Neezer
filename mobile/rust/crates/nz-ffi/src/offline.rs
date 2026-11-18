//! The offline FFI surface — lot MB15.
//!
//! Owned by **MB15** (docs/agents/mobile.md, "Module and file ownership"): this
//! file holds the downloads' records, its listener and `OfflineApi`'s methods,
//! and converts `nz_core::offline`'s own values. No rule lives here.
//!
//! # Why `offlineAttach(core)`
//!
//! `NzCore::offline()` answers `Arc::new(OfflineApi)` — MB0 declared the object
//! as a unit struct with its final SHAPE, so it carries no `Arc<Core>` the way
//! `PlayerApi::open(core)` does. The service therefore lives in
//! `nz_core::offline`'s ONE per-process slot, and the Kotlin side binds it once:
//!
//! ```kotlin
//! offlineAttach(NzBridge.core!!)          // :feature:offline's OfflineCore
//! val api = NzBridge.core!!.offline()     // then every call answers
//! ```
//!
//! Before that call every method here answers `NzError::Internal`, whose
//! sentence names the cause — a screen that somehow reached it shows a cause and
//! « Réessayer », never a blank button (rule 3).
//!
//! # What crosses the boundary
//!
//! Screen-shaped values, never the server's JSON: a set's row ([`SetProgress`]),
//! the worker's state ([`OfflineState`]), what the device holds
//! ([`StorageReport`], and `ListHandle` for the per-title list — 4 000 rows never
//! cross whole, rule 4), and the person's three settings ([`OfflineSettings`]).
//! Writes are **optimistic**: `add_set` returns the set's row before a byte is
//! fetched, and every later change arrives through [`OfflineListener`] — the
//! Kotlin side hands it to a Flow and returns at once, and Rust holds no lock
//! while calling it.

use std::sync::Arc;

use nz_core::offline as core;

use crate::core::NzCore;
use crate::errors::NzError;
use crate::lists::ListHandle;

/// What one download set is doing. `failures` carries the CAUSE of each failure
/// as one French sentence per title (« titre — cause »), never a count (rule 2).
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct SetProgress {
    /// `playlist:42`, `album:7`, `show:3`, `track:3135556` — the group the
    /// button was pressed on, and the key every method of this file speaks.
    pub context: String,
    /// The set's own title, kept locally so the row reads offline.
    pub title: String,
    /// `playlist`, `album`, `show` or `track`.
    pub kind: String,
    /// The server's id of that entity.
    pub id: String,
    pub done: u32,
    /// What the set ASKED for: a failure is a wanted title, so a settled run
    /// with six causes reads `194 / 200`, never a reassuring `200 / 200`.
    pub total: u32,
    pub failed: u32,
    /// Still to come (queued or being written).
    pub running: bool,
    /// The person asked for a pause.
    pub paused: bool,
    /// One French sentence per failed title.
    pub failures: Vec<String>,
}

/// Counted bytes of what the device holds (rule 3, *Measures*: never an RSS).
#[derive(uniffi::Record, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StorageReport {
    /// `filesDir`: the downloads, never evicted.
    pub downloads_bytes: u64,
    /// `cacheDir`: the play cache, LRU.
    pub cache_bytes: u64,
    /// `cacheDir`: the covers, LRU.
    pub covers_bytes: u64,
    /// How many titles are published on the device.
    pub tracks: u32,
    /// The disk's free space, when the CALLER knows it: the core answers 0 here
    /// (free space is `StatFs`'s, and `nz-core` forbids the `unsafe` binding
    /// that would answer it), and the settings section prints the platform's
    /// number beside these.
    pub free_bytes: u64,
}

/// The person's offline settings, under the SPA's own keys (`stores.js`).
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct OfflineSettings {
    /// `offline.quality`: the rung a new download takes (`OPUS_320` by
    /// default; `FLAC`, `OPUS_128`, `OPUS_64` are the other three the engine
    /// decodes).
    pub quality: String,
    /// `offline.onlyDownloaded`: with the link down, queue only what is here.
    pub offline_only: bool,
    /// `cache.limit`: the play cache's cap, in bytes.
    pub play_cache_cap: i64,
}

/// One row of « Titres téléchargés »: the labels the set was queued with, and
/// the quality/bytes [`OfflineApi::download_facts`] reads for the page on
/// screen.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct DownloadEntry {
    /// The stream id, and the key `remove_download` takes.
    pub key: String,
    pub title: String,
    pub artist: String,
    pub cover: Option<String>,
    pub duration_s: u32,
    pub explicit: bool,
    /// The rung the FILE is (`FLAC`, `OPUS_320`, …), empty until `download_facts`
    /// answered for it.
    pub quality: String,
    pub bytes: u64,
    /// Milliseconds since the epoch: the list's order.
    pub added_at: i64,
}

/// The whole offline state, as one value: what a screen paints, and the version
/// it compares to know whether to paint again.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct OfflineState {
    /// Bumped by the worker on every change; a listener's value is the one this
    /// record carries, so a screen reads the state once per change and never on
    /// a timer.
    pub version: u64,
    /// The worker's own French sentence (« Téléchargement en cours », « Hors
    /// ligne : reprise automatique », or the cause that stopped it).
    pub state: String,
    pub paused: bool,
    pub sets: Vec<SetProgress>,
    pub settings: OfflineSettings,
    pub storage: StorageReport,
    /// What the last run did, in one sentence (rule 2).
    pub note: Option<String>,
}

/// The download queue changed, or a set finished/failed. Pushed on a Rust
/// thread: Kotlin hands it to a Flow and returns at once.
#[uniffi::export(callback_interface)]
pub trait OfflineListener: Send + Sync {
    /// The whole state, cheap: a queued set, a finished file, a pause.
    fn on_progress(&self, state: OfflineState);
    /// One entry failed; `cause` is French and the queue goes on.
    fn on_failure(&self, key: String, cause: String);
}

/// The downloads API — one per process (`NzCore::offline()`).
#[derive(uniffi::Object)]
pub struct OfflineApi;

impl From<core::SetRow> for SetProgress {
    fn from(row: core::SetRow) -> Self {
        SetProgress {
            context: row.context,
            title: row.title,
            kind: row.kind,
            id: row.id,
            done: row.done,
            total: row.total,
            failed: row.failed,
            running: row.running,
            paused: row.paused,
            failures: row.failures,
        }
    }
}

impl From<core::StorageView> for StorageReport {
    fn from(view: core::StorageView) -> Self {
        StorageReport {
            downloads_bytes: view.downloads_bytes,
            cache_bytes: view.cache_bytes,
            covers_bytes: view.covers_bytes,
            tracks: view.tracks,
            free_bytes: view.free_bytes,
        }
    }
}

impl From<core::OfflineSettingsView> for OfflineSettings {
    fn from(view: core::OfflineSettingsView) -> Self {
        OfflineSettings {
            quality: view.quality,
            offline_only: view.offline_only,
            play_cache_cap: view.play_cache_cap,
        }
    }
}

impl From<core::OfflineState> for OfflineState {
    fn from(state: core::OfflineState) -> Self {
        OfflineState {
            version: state.version,
            state: state.state,
            paused: state.paused,
            sets: state.sets.into_iter().map(SetProgress::from).collect(),
            settings: state.settings.into(),
            storage: state.storage.into(),
            note: state.note,
        }
    }
}

impl From<core::DownloadRow> for DownloadEntry {
    fn from(row: core::DownloadRow) -> Self {
        DownloadEntry {
            key: row.key,
            title: row.title,
            artist: row.artist,
            cover: row.cover,
            duration_s: row.duration_s,
            explicit: row.explicit,
            quality: row.quality,
            bytes: row.bytes,
            added_at: row.added_at,
        }
    }
}

/// The Kotlin listener as the core's sink: it runs on the Rust thread that
/// fired it, holds no lock, and returns at once.
struct Sink(Arc<dyn OfflineListener>);

impl core::ProgressSink for Sink {
    fn progress(&self, state: &core::OfflineState) {
        self.0.on_progress(OfflineState::from(state.clone()));
    }

    fn failure(&self, key: &str, cause: &str) {
        self.0.on_failure(key.to_owned(), cause.to_owned());
    }
}

impl OfflineApi {
    /// The process's service, or the cause that it was never bound.
    fn service(&self) -> Result<Arc<core::Offline>, NzError> {
        core::get().ok_or_else(unbound)
    }
}

fn unbound() -> NzError {
    NzError::Internal {
        detail: "les téléchargements ne sont pas reliés au noyau (offlineAttach)".to_owned(),
    }
}

/// Bind the downloads' service to `core`, once per process, and let a queue left
/// by the last process carry on.
///
/// `:feature:offline`'s `OfflineCore` calls this once at process start
/// (`NzBridge.install` cannot: `:bridge` is another lot's file). It is `async`
/// because binding is where `nz-bulk` learns the runtime it drives its futures
/// with, and only an async call has one. Idempotent.
///
/// **`async_runtime = "tokio"` is not optional.** Without it UniFFI polls the
/// future on no reactor at all: the JVM suites never see it (their own runtime
/// hides the gap), and on a DEVICE the call answers
/// `InternalException: there is no reactor running, must be called from the
/// context of a Tokio 1.x runtime` — measured on the emulator on 2026-11-18, where
/// it made every download impossible (`OfflineCore.api()` swallowed it, so the
/// screens said "les téléchargements ne sont pas reliés au noyau").
#[uniffi::export(async_runtime = "tokio")]
pub async fn offline_attach(core: Arc<NzCore>) {
    core::attach(core.core()).start_session().await;
}

#[uniffi::export(async_runtime = "tokio")]
impl OfflineApi {
    /// Queue one context (a playlist, an album, a show, a single title) on the
    /// `Bulk` lane. Returns at once, with the set's optimistic row: the worker
    /// runs in the background and SAYS SO.
    ///
    /// `title` and `artist` are the caller's own labels for a single title
    /// (`kind = "track"`): the phone has no route that turns a track id back
    /// into a row, so the surface that pressed the button is the only place they
    /// exist. They are ignored for the three other kinds, whose page carries
    /// them.
    pub async fn add_set(
        &self,
        kind: String,
        id: String,
        quality: String,
        title: String,
        artist: String,
    ) -> Result<SetProgress, NzError> {
        let service = self.service()?;
        service
            .add_set(&kind, &id, &quality, &title, &artist)
            .await
            .map(SetProgress::from)
            .map_err(NzError::from)
    }

    /// Take a set back, files and all. The files go now; the click never waits
    /// on the disk.
    pub async fn remove_set(&self, kind: String, id: String) -> Result<(), NzError> {
        self.service()?
            .remove_set(&kind, &id)
            .await
            .map_err(NzError::from)
    }

    /// Remove ONE downloaded title (the settings list's bin).
    pub async fn remove_download(&self, key: String) -> Result<(), NzError> {
        self.service()?
            .remove_download(&key)
            .await
            .map_err(NzError::from)
    }

    /// Retry ONE failed title (a stream id) or one whole set (`kind:id`), with
    /// its cause cleared. Answers `Invalid` when nothing under that key failed:
    /// a button that does nothing is a bug.
    pub async fn retry(&self, key: String) -> Result<(), NzError> {
        self.service()?.retry(&key).await.map_err(NzError::from)
    }

    /// Give every failure another chance.
    pub async fn retry_all(&self) -> Result<(), NzError> {
        self.service()?.retry_all().await.map_err(NzError::from)
    }

    /// Pauses the queue; the files already published stay.
    pub async fn pause(&self) -> Result<(), NzError> {
        self.service()?.pause().await.map_err(NzError::from)
    }

    /// Resumes from SQLite, never from the start (rule 2).
    pub async fn resume(&self) -> Result<(), NzError> {
        self.service()?.resume().await.map_err(NzError::from)
    }

    /// `offline.quality` — the rung a new download takes.
    pub async fn set_quality(&self, quality: String) -> Result<(), NzError> {
        self.service()?
            .set_quality(&quality)
            .await
            .map_err(NzError::from)
    }

    /// `offline.onlyDownloaded`.
    pub async fn set_offline_only(&self, offline_only: bool) -> Result<(), NzError> {
        self.service()?
            .set_offline_only(offline_only)
            .await
            .map_err(NzError::from)
    }

    /// `cache.limit` — the play cache's cap, in bytes. Lowering it trims the
    /// cache at once (the worker's own sweep).
    pub async fn set_play_cache_cap(&self, bytes: i64) -> Result<(), NzError> {
        self.service()?
            .set_play_cache_cap(bytes)
            .await
            .map_err(NzError::from)
    }

    /// Empty the play cache. Downloads are untouched: nothing but the person
    /// deletes one.
    pub async fn clear_play_cache(&self) -> Result<(), NzError> {
        self.service()?
            .clear_play_cache()
            .await
            .map_err(NzError::from)
    }
}

#[uniffi::export]
impl OfflineApi {
    /// The whole state, as one value (a mutex and a clone: never a statement on
    /// the UI thread).
    pub fn state(&self) -> Result<OfflineState, NzError> {
        Ok(OfflineState::from(self.service()?.state()))
    }

    /// Every set the device knows, by context.
    pub fn sets(&self) -> Result<Vec<SetProgress>, NzError> {
        Ok(self
            .service()?
            .state()
            .sets
            .into_iter()
            .map(SetProgress::from)
            .collect())
    }

    /// One set's progress, or `None` when nothing of it was ever queued — which
    /// is the `DownloadButton`'s "absent" state.
    pub fn progress(&self, kind: String, id: String) -> Result<Option<SetProgress>, NzError> {
        Ok(self.service()?.progress(&kind, &id).map(SetProgress::from))
    }

    /// Counted bytes of what the device holds.
    pub fn storage(&self) -> Result<StorageReport, NzError> {
        Ok(self.service()?.state().storage.into())
    }

    /// The person's three offline settings.
    pub fn settings(&self) -> Result<OfflineSettings, NzError> {
        Ok(self.service()?.settings().into())
    }

    /// The play cache's own size (the settings section's gauge).
    pub fn play_cache_bytes(&self) -> Result<u64, NzError> {
        Ok(self.service()?.play_cache_bytes())
    }

    /// Every downloaded title, as a WINDOWED list (rule 4): 4 000 downloads are
    /// pages of 64 rows through `rememberListWindow`, never one `Vec` across the
    /// boundary.
    pub fn downloads(&self) -> Result<Arc<ListHandle>, NzError> {
        let service = self.service()?;
        let rows = service
            .downloads()
            .into_iter()
            .enumerate()
            .map(|(position, row)| nz_core::lists::Row {
                key: row.key.clone(),
                id: row.key,
                title: row.title,
                artists: row.artist,
                album: String::new(),
                album_id: None,
                duration_s: row.duration_s,
                cover: row.cover,
                explicit: row.explicit,
                liked: false,
                disliked: false,
                unavailable: false,
                downloaded: true,
                source: "deezer".to_owned(),
                position: position as u32,
                played_at: 0,
                added_at: row.added_at.max(0) as u64,
            })
            .collect();
        Ok(crate::lists::open_rows(service.core(), rows))
    }

    /// The quality and the bytes of the rows a list is SHOWING (one store read
    /// and one `stat` per key): a 4 000-download device reads 64 of them per
    /// page, never 4 000.
    pub fn download_facts(&self, keys: Vec<String>) -> Result<Vec<DownloadEntry>, NzError> {
        let service = self.service()?;
        Ok(service
            .facts(&keys)
            .into_iter()
            .map(|(key, quality, bytes)| DownloadEntry {
                key,
                title: String::new(),
                artist: String::new(),
                cover: None,
                duration_s: 0,
                explicit: false,
                quality,
                bytes,
                added_at: 0,
            })
            .collect())
    }

    /// The settings list's rows in one call, for a test and for a device whose
    /// download count is small: a screen uses [`OfflineApi::downloads`].
    pub fn download_entries(&self) -> Result<Vec<DownloadEntry>, NzError> {
        let service = self.service()?;
        let rows = service.downloads();
        let mut entries: Vec<DownloadEntry> = rows.into_iter().map(DownloadEntry::from).collect();
        let keys: Vec<String> = entries.iter().map(|entry| entry.key.clone()).collect();
        for (entry, fact) in entries.iter_mut().zip(service.facts(&keys)) {
            entry.quality = fact.1;
            entry.bytes = fact.2;
        }
        Ok(entries)
    }

    /// `true` when `track` is downloaded and playable — a row's « téléchargé »
    /// flag, answered from the published set (no statement).
    pub fn is_downloaded(&self, key: String) -> Result<bool, NzError> {
        Ok(self.service()?.is_downloaded(&key))
    }

    /// The "only downloaded" filter (`lib/offline.js`): what a queue builder
    /// keeps when the link is down and `offline.only_downloaded` is on.
    pub fn keep_downloaded(&self, keys: Vec<String>) -> Result<Vec<String>, NzError> {
        Ok(self.service()?.keep_downloaded(&keys))
    }

    /// The throughput the link meter last measured, in bytes per second, and the
    /// cap `nz-api`'s Bulk lane holds — the acceptance's own numbers, for the
    /// diagnostic screen and the PR.
    pub fn link_report(&self) -> Result<LinkReport, NzError> {
        let service = self.service()?;
        Ok(LinkReport {
            stream_bytes_per_s: service.stream_rate(),
            bulk_cap_bytes_per_s: service.bulk_cap(),
        })
    }

    /// The listener subscribes ONCE (the section and the shell share it): the
    /// current state arrives at once, then every change.
    pub fn subscribe(&self, listener: Box<dyn OfflineListener>) -> Result<(), NzError> {
        self.service()?
            .subscribe(Arc::new(Sink(Arc::from(listener))));
        Ok(())
    }
}

/// What the downloads cost the link, in counted bytes per second.
#[derive(uniffi::Record, Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkReport {
    pub stream_bytes_per_s: u64,
    pub bulk_cap_bytes_per_s: Option<u64>,
}
