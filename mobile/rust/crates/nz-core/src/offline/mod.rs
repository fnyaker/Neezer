//! Hors ligne: the downloads per playlist/album/show/track, what the device
//! holds, its storage use and the "only downloaded" filter (lot MB15).
//!
//! This module is the PHONE's half of the downloads. The queue, the store, the
//! retry rules and the causes are `nz-offline`'s (SC4, extracted from desktop
//! D13) and this file never second-guesses them: it starts that one worker over
//! the core's cache, hands it the titles a context holds, and turns what it
//! publishes into the values a screen draws. The one behaviour that is genuinely
//! the phone's is [`LinkMeter`](crate::offline::rate::LinkMeter): feeding
//! `nz-api`'s Bulk cap from a throughput nobody else measures here.
//!
//! Four rules, each with the mechanism that keeps it:
//!
//! - **Rule 1 — what the person does now comes first.** Queuing a set reads the
//!   set's own page on `Lane::User` (tagged `offline`), then everything it
//!   fetches is `Lane::Bulk` (tagged `downloads`): one in flight, 1 s of quiet,
//!   `X-NS-Background: 1`, and a body paced at 25 % of the playing stream's
//!   measured throughput. There is no second throttle here, on purpose.
//! - **Rule 2 — long work says so, and resumes.** `add_set` returns at once with
//!   the set's optimistic row; `nz-bulk` does the transfers; every change is
//!   pushed to the Kotlin listener with n of m and the CAUSE of each failure, and
//!   the queue is SQLite rows, so a kill resumes where it stopped instead of
//!   starting over.
//! - **Rule 3 — a failure is never a verdict.** The worker keeps a dead link and
//!   a `503 Retry-After` as "later" and the row stays pending; only the server's
//!   own classification, a `4xx` or a full disk marks a row failed, with its
//!   cause.
//! - **Nothing deletes archived audio**: a download lives in `filesDir` under
//!   `nz-cache`'s own table and only the person removes one (or its set).
//!
//! ## Attaching (why an FFI object is not enough)
//!
//! `NzCore::offline()` answers an `OfflineApi` that MB0 declared as a unit
//! struct, so it cannot carry `Arc<Core>` the way `PlayerApi::open(core)` does.
//! The Kotlin side therefore calls `offlineAttach(core)` once (`:feature:offline`'s
//! `OfflineCore`), which binds this module's ONE service for the process
//! ([`attach`]). Every FFI method answers `NzError::Unsupported { feature:
//! "offline:attach" }` before that, which is a cause a screen prints rather than
//! a panic.

mod rate;
mod sets;
#[cfg(test)]
mod tests;
mod view;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nz_api::Quality;
use tracing::{debug, info, warn};

use crate::core::{Core, CoreError};

pub use rate::LinkMeter;
pub use sets::{EPISODE_PAGE, known_kind, split_context};
pub use view::{
    DownloadRow, Labels, OfflineSettingsView, OfflineState, QueuedSet, SetRow, StorageView,
    TrackLabel,
};

/// What this module's own reads are counted under (`nz-api`'s per-tag totals).
pub const TAG_OFFLINE: &str = "offline";
/// What the download bodies are counted under: the acceptance's own number.
pub const TAG_DOWNLOAD: &str = "downloads";

/// The contexts this device has queued (`Vec<String>` in `nz-cache`'s `kv`).
pub const KEY_SETS: &str = "offline.sets";
/// One set's titles, under `offline.set.<kind>:<id>`: one statement per set.
pub const KEY_SET_PREFIX: &str = "offline.set.";

/// The idle loop's period: a download that finishes is reported within a second,
/// and an idle app costs one integer comparison every two (the same trade
/// `nz-offline`'s own worker makes).
const IDLE_POLL: Duration = Duration::from_secs(2);
/// While a download runs: one sample a second is what the link's meter needs.
const BUSY_POLL: Duration = Duration::from_secs(1);

/// Milliseconds since the epoch, for a row's order.
pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// Who hears about the downloads. `nz-ffi` implements this over
/// `OfflineListener`; the worker holds no lock while calling it, and the Kotlin
/// side hands the value to a Flow and returns at once.
pub trait ProgressSink: Send + Sync {
    fn progress(&self, state: &OfflineState);
    /// One title gave up: `key` is its stream id, `cause` is French.
    fn failure(&self, key: &str, cause: &str);
}

struct Inner {
    core: Arc<Core>,
    worker: nz_offline::Offline,
    listeners: Mutex<Vec<Arc<dyn ProgressSink>>>,
    /// One set's titles, by context. Kept in memory so the list, the failures'
    /// titles and the single-track rows never read SQLite on a frame.
    labels: Mutex<HashMap<String, QueuedSet>>,
    /// The pointer of the `Api` the worker was last configured with: a login
    /// rebuilds the client, and a queued download must follow it.
    configured: Mutex<Option<usize>>,
    meter: Mutex<LinkMeter>,
    /// The last rate handed to `nz-api` (bytes per second), for `Débit` in the
    /// diagnostic log and for the acceptance's own number.
    cap: Mutex<Option<u64>>,
    stop: AtomicBool,
    /// The loop's own lock: it sleeps here and every request wakes it, so a
    /// click never waits for the next poll.
    gate: Mutex<()>,
    wake: Condvar,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

/// The process's offline service: one worker, its labels, and the meter that
/// makes it yield.
pub struct Offline {
    inner: Arc<Inner>,
}

static SERVICE: OnceLock<Arc<Offline>> = OnceLock::new();

/// Bind the process's offline service to `core`, once.
///
/// Idempotent on purpose: `:bridge` and the settings section both reach it, and
/// the second call must not start a second `nz-bulk` (two workers over one
/// queue would both fetch the same row).
pub fn attach(core: &Arc<Core>) -> Arc<Offline> {
    Arc::clone(SERVICE.get_or_init(|| Offline::start(Arc::clone(core))))
}

/// The service, when [`attach`] has run.
pub fn get() -> Option<Arc<Offline>> {
    SERVICE.get().cloned()
}

/// Stop the process's worker and join it (worker rule 5: the loop borrows the
/// tokio runtime with `Handle::block_on`, so it must be joined before the
/// runtime is dropped — D13's pitfall).
pub fn shutdown() {
    if let Some(service) = SERVICE.get() {
        service.shutdown();
    }
}

impl Offline {
    /// Start a service over `core`'s cache. Public so a test owns its own (the
    /// process's one is [`attach`]'s).
    pub fn start(core: Arc<Core>) -> Arc<Offline> {
        let worker = nz_offline::Offline::start(core.cache().clone());
        let labels = load_labels(core.cache());
        let labels = Mutex::new(labels);
        let inner = Arc::new(Inner {
            core,
            worker,
            listeners: Mutex::new(Vec::new()),
            labels,
            configured: Mutex::new(None),
            meter: Mutex::new(LinkMeter::new()),
            cap: Mutex::new(None),
            stop: AtomicBool::new(false),
            gate: Mutex::new(()),
            wake: Condvar::new(),
            thread: Mutex::new(None),
        });
        let offline = Arc::new(Offline {
            inner: Arc::clone(&inner),
        });
        let looped = Arc::clone(&inner);
        let spawned = std::thread::Builder::new()
            .name("nz-offline".to_owned())
            .spawn(move || run(looped));
        match spawned {
            Ok(thread) => *lock(&inner.thread) = Some(thread),
            Err(error) => warn!(%error, "le guetteur hors ligne n'a pas démarré"),
        }
        offline
    }

    pub fn core(&self) -> &Arc<Core> {
        &self.inner.core
    }

    /// Stop the loop, join it, then stop `nz-bulk` and join it. Idempotent.
    pub fn shutdown(&self) {
        self.inner.stop.store(true, Ordering::Release);
        self.wake();
        let thread = lock(&self.inner.thread).take();
        if let Some(thread) = thread {
            let _ = thread.join();
        }
        self.inner.worker.shutdown();
    }

    fn wake(&self) {
        let _gate = lock(&self.inner.gate);
        self.inner.wake.notify_all();
    }

    /// Point the worker at the session's client, when it changed.
    ///
    /// Called from every async entry point (the only place a runtime handle
    /// exists — `nz-bulk` drives its futures with `Handle::block_on`). A login
    /// rebuilds the `Api`, so the pointer is what decides, not a flag.
    fn configure(&self) {
        let Ok(api) = self.inner.core.api() else {
            return;
        };
        let pointer = Arc::as_ptr(&api) as usize;
        {
            let configured = lock(&self.inner.configured);
            if *configured == Some(pointer) {
                return;
            }
        }
        // The download bodies count under their own tag: the link meter reads
        // `TAG_NONE` only, so a download can never inflate the rate it is
        // throttled by (rule 1).
        let tagged = Arc::new(api.with_tag(TAG_DOWNLOAD));
        let downloader = Arc::new(nz_offline::ApiDownloader::new(
            tagged,
            tokio::runtime::Handle::current(),
        ));
        self.inner
            .worker
            .configure_with(Arc::clone(&api), downloader);
        *lock(&self.inner.configured) = Some(pointer);
        debug!("worker hors ligne configuré");
    }

    // --- what a screen reads ------------------------------------------------

    /// The whole state, as one value: the sets, the settings, the storage and
    /// the worker's own sentence.
    pub fn state(&self) -> OfflineState {
        let snapshot = self.inner.worker.snapshot();
        let settings = self.inner.worker.settings();
        let labels = self.labels();
        let failures = failure_index(&snapshot.failures, &labels);
        let mut sets: Vec<SetRow> = snapshot
            .sets
            .iter()
            .map(|set| {
                let context = format!("{}:{}", set.kind, set.ident);
                let causes = failures
                    .iter()
                    .filter(|(_, contexts)| contexts.contains(&context))
                    .map(|(sentence, _)| sentence.clone())
                    .collect::<Vec<_>>();
                SetRow {
                    context,
                    kind: set.kind.clone(),
                    id: set.ident.clone(),
                    title: set.title.clone(),
                    done: set.done as u32,
                    total: set.tracks as u32,
                    failed: causes.len() as u32,
                    running: set.done < set.tracks,
                    paused: self.inner.worker.paused(),
                    failures: causes,
                }
            })
            .collect();
        // A set the worker has not refreshed yet (a click one frame ago) is
        // drawn from its optimistic row instead of vanishing (rule 4).
        for (context, queued) in &labels {
            if sets.iter().any(|set| &set.context == context) {
                continue;
            }
            if let Some((kind, id)) = sets::split_context(context) {
                sets.push(SetRow {
                    context: context.clone(),
                    kind: kind.to_owned(),
                    id: id.to_owned(),
                    title: queued.title.clone(),
                    done: 0,
                    total: queued.tracks.len() as u32,
                    failed: 0,
                    running: true,
                    paused: self.inner.worker.paused(),
                    failures: Vec::new(),
                });
            }
        }
        sets.sort_by(|a, b| a.context.cmp(&b.context));
        OfflineState {
            version: snapshot.version,
            state: snapshot.state.label().to_owned(),
            paused: self.inner.worker.paused(),
            sets,
            settings: OfflineSettingsView {
                quality: settings.quality.wire().unwrap_or("FLAC").to_owned(),
                offline_only: settings.offline_only,
                play_cache_cap: settings.play_cache_cap,
            },
            storage: StorageView {
                downloads_bytes: snapshot.progress.bytes.max(0) as u64,
                cache_bytes: snapshot.play_cache_bytes.max(0) as u64,
                covers_bytes: snapshot.covers_bytes.max(0) as u64,
                tracks: snapshot.progress.done as u32,
                free_bytes: 0,
            },
            note: snapshot.note.clone(),
            downloaded: Arc::clone(&snapshot.downloaded),
        }
    }

    /// One set's row, when the device knows it.
    pub fn progress(&self, kind: &str, id: &str) -> Option<SetRow> {
        let context = format!("{kind}:{id}");
        self.state()
            .sets
            .into_iter()
            .find(|set| set.context == context)
    }

    /// The worker's sentence for a `LoadFailed` (never a code).
    pub fn worker_note(&self) -> Option<String> {
        self.inner.worker.snapshot().note
    }

    /// Every published title, as the settings list draws it — newest first.
    ///
    /// Pure memory: the labels and the published set, no SQLite statement (the
    /// list must not cost a query per row on a 4 000-download device). A title
    /// whose label was lost shows its id, like the SPA's `slimTrack` fallback.
    pub fn downloads(&self) -> Vec<DownloadRow> {
        let snapshot = self.inner.worker.snapshot();
        let labels = self.labels();
        let mut rows: Vec<DownloadRow> = snapshot
            .downloaded
            .iter()
            .map(|id| {
                let label = find_label(&labels, id);
                DownloadRow {
                    key: id.clone(),
                    title: label
                        .map(|label| label.title.clone())
                        .filter(|title| !title.is_empty())
                        .unwrap_or_else(|| id.clone()),
                    artist: label.map(|label| label.artist.clone()).unwrap_or_default(),
                    cover: label.and_then(|label| label.cover.clone()),
                    duration_s: label.map(|label| label.duration_s).unwrap_or(0),
                    explicit: label.map(|label| label.explicit).unwrap_or(false),
                    quality: String::new(),
                    bytes: 0,
                    added_at: label.map(|label| label.added_at).unwrap_or(0),
                }
            })
            .collect();
        // Most recent first, the SPA's own order (`listDownloads` sorts by
        // `lastPlayedAt`, then `addedAt` for a title never played).
        rows.sort_by(|a, b| {
            b.added_at
                .cmp(&a.added_at)
                .then_with(|| a.title.cmp(&b.title))
        });
        rows
    }

    /// The quality and the bytes of `keys`, read for the page a list is showing
    /// (never its 4 000 rows): one store read and one `stat` per visible title.
    pub fn facts(&self, keys: &[String]) -> Vec<(String, String, u64)> {
        let Ok(api) = self.inner.core.api() else {
            return keys
                .iter()
                .map(|key| (key.clone(), String::new(), 0))
                .collect();
        };
        let Some(owner) = api.owner() else {
            return keys
                .iter()
                .map(|key| (key.clone(), String::new(), 0))
                .collect();
        };
        let store = self.inner.core.cache();
        keys.iter()
            .map(|key| {
                let track = nz_api::TrackId::new(key.clone());
                match store.downloads().local(&owner, &track) {
                    Ok(Some((path, quality))) => {
                        let bytes = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
                        (
                            key.clone(),
                            quality.wire().unwrap_or("FLAC").to_owned(),
                            bytes,
                        )
                    }
                    _ => (key.clone(), String::new(), 0),
                }
            })
            .collect()
    }

    /// The play cache's own facts, for the settings section.
    pub fn play_cache_bytes(&self) -> u64 {
        self.inner
            .core
            .cache()
            .play_cache()
            .total_bytes()
            .unwrap_or(0)
            .max(0) as u64
    }

    /// `true` when `track` is downloaded and playable — the SPA's `isDownloaded`.
    pub fn is_downloaded(&self, track: &str) -> bool {
        self.inner.worker.snapshot().downloaded.contains(track)
    }

    /// `lib/offline.js#isDownloaded` over a queue: what the "only downloaded"
    /// filter keeps. `offline_only` decides whether anything is filtered at all,
    /// and the caller applies it only when the link is down.
    pub fn keep_downloaded(&self, keys: &[String]) -> Vec<String> {
        let downloaded = Arc::clone(&self.inner.worker.snapshot().downloaded);
        keys.iter()
            .filter(|key| downloaded.contains(*key))
            .cloned()
            .collect()
    }

    /// The settings the SPA keeps under `offline.*` and `cache.limit`.
    pub fn settings(&self) -> OfflineSettingsView {
        let settings = self.inner.worker.settings();
        OfflineSettingsView {
            quality: settings.quality.wire().unwrap_or("FLAC").to_owned(),
            offline_only: settings.offline_only,
            play_cache_cap: settings.play_cache_cap,
        }
    }

    /// Counted bytes of this module's own memory (worker rule 4).
    pub fn memory_bytes(&self) -> u64 {
        self.labels()
            .values()
            .map(|queued| {
                queued.title.len()
                    + std::mem::size_of::<QueuedSet>()
                    + queued
                        .tracks
                        .iter()
                        .map(|label| {
                            std::mem::size_of::<TrackLabel>()
                                + label.id.len()
                                + label.title.len()
                                + label.artist.len()
                                + label.cover.as_ref().map(String::len).unwrap_or(0)
                        })
                        .sum::<usize>()
            })
            .sum::<usize>() as u64
    }

    fn labels(&self) -> HashMap<String, QueuedSet> {
        lock(&self.inner.labels).clone()
    }

    // --- the session -------------------------------------------------------

    /// Point the worker at the signed-in client and let a queue left by the last
    /// process carry on (rule 2: a download resumes instead of starting over).
    ///
    /// `offlineAttach` calls it from an async context, which is the only place a
    /// runtime handle exists for `nz-bulk` to drive its futures with. Idempotent.
    pub async fn start_session(&self) {
        self.configure();
        self.wake();
    }

    // --- what a person asks for --------------------------------------------

    /// Queue a context (`playlist:42`, `album:7`, `show:3`, `track:<id>`).
    ///
    /// The set's titles are read on `Lane::User` (the person just pressed the
    /// button), the titles are labelled in one `kv` statement, and the transfer
    /// itself is `nz-bulk`'s. Returns at once: the row is optimistic, and the
    /// worker corrects it a frame later (rule 4).
    pub async fn add_set(
        &self,
        kind: &str,
        id: &str,
        quality: &str,
        title_hint: &str,
        artist_hint: &str,
    ) -> Result<SetRow, CoreError> {
        if !sets::known_kind(kind) {
            return Err(CoreError::Unsupported {
                feature: format!("téléchargement de « {kind} »"),
            });
        }
        let api = self.inner.core.api()?;
        self.configure();
        let api = api.with_tag(TAG_OFFLINE);
        let resolved = sets::resolve(&api, kind, id, title_hint, artist_hint).await?;
        if resolved.tracks.is_empty() {
            return Err(CoreError::Invalid {
                detail: format!(
                    "« {} » ne contient aucun titre à télécharger",
                    if resolved.title.is_empty() {
                        id
                    } else {
                        &resolved.title
                    }
                ),
            });
        }
        // An empty rung is the settings' own (`offline.quality`); anything the
        // server would accept but the engine cannot decode is refused here, with
        // the cause a screen prints.
        let quality = match quality.trim() {
            "" => self.inner.worker.settings().quality,
            raw => Quality::parse(Some(raw)).ok_or_else(|| CoreError::Invalid {
                detail: format!("qualité inconnue : « {raw} »"),
            })?,
        };
        let context = format!("{kind}:{id}");
        {
            let mut labels = lock(&self.inner.labels);
            labels.insert(
                context.clone(),
                QueuedSet {
                    title: resolved.title.clone(),
                    tracks: resolved
                        .tracks
                        .iter()
                        .map(|wanted| wanted.label.clone())
                        .collect(),
                },
            );
            save_labels(self.inner.core.cache(), &labels);
        }
        let choices = resolved
            .tracks
            .iter()
            .map(|wanted| nz_offline::Choice {
                track: wanted.id.clone(),
                title: wanted.label.title.clone(),
                artist: wanted.label.artist.clone(),
                quality: Some(quality),
            })
            .collect::<Vec<_>>();
        info!(
            context,
            tracks = choices.len(),
            quality = quality.wire().unwrap_or("FLAC"),
            "téléchargement demandé"
        );
        self.inner
            .worker
            .add_set(kind, id, &resolved.title, choices);
        self.wake();
        Ok(self.progress(kind, id).unwrap_or(SetRow {
            context,
            kind: kind.to_owned(),
            id: id.to_owned(),
            title: resolved.title,
            done: 0,
            total: resolved.tracks.len() as u32,
            failed: 0,
            running: true,
            paused: false,
            failures: Vec::new(),
        }))
    }

    /// Take a set back, files and all. Returns at once; the worker deletes.
    pub async fn remove_set(&self, kind: &str, id: &str) -> Result<(), CoreError> {
        let context = format!("{kind}:{id}");
        self.configure();
        self.inner.worker.remove_set(kind, id);
        {
            let mut labels = lock(&self.inner.labels);
            labels.remove(&context);
            save_labels(self.inner.core.cache(), &labels);
        }
        self.wake();
        Ok(())
    }

    /// Remove ONE downloaded title (the settings list's bin).
    pub async fn remove_download(&self, key: &str) -> Result<(), CoreError> {
        self.configure();
        self.inner.worker.drop_track(key);
        {
            let mut labels = lock(&self.inner.labels);
            for queued in labels.values_mut() {
                queued.tracks.retain(|label| label.id != key);
            }
            labels.retain(|_, queued| !queued.tracks.is_empty());
            save_labels(self.inner.core.cache(), &labels);
        }
        self.wake();
        Ok(())
    }

    /// Give failures another chance: one title (`key` is a stream id) or one
    /// context (`kind:id`), with the cause cleared.
    pub async fn retry(&self, key: &str) -> Result<(), CoreError> {
        self.configure();
        let api = self.inner.core.api()?;
        let owner = api.owner().ok_or_else(|| CoreError::Invalid {
            detail: "aucune session : impossible de réessayer".to_owned(),
        })?;
        let store = self.inner.core.cache();
        let tracks: Vec<String> = if let Some((kind, id)) = sets::split_context(key) {
            let context = format!("{kind}:{id}");
            lock(&self.inner.labels)
                .get(&context)
                .map(|queued| queued.tracks.iter().map(|label| label.id.clone()).collect())
                .unwrap_or_default()
        } else {
            vec![key.to_owned()]
        };
        let mut retried = 0usize;
        for track in &tracks {
            let id = nz_api::TrackId::new(track.clone());
            if store
                .downloads()
                .mark(&owner, &id, nz_cache::DownloadState::Pending, None)
                .unwrap_or(false)
            {
                retried += 1;
            }
        }
        if retried == 0 {
            // Nothing was failed under that key: an HONEST answer, not a
            // silent success (rule 2: a button that seems to do nothing is a bug).
            return Err(CoreError::Invalid {
                detail: "Aucun échec à réessayer".to_owned(),
            });
        }
        self.wake();
        Ok(())
    }

    /// Retry every failure of every set — the settings section's one button.
    pub async fn retry_all(&self) -> Result<(), CoreError> {
        self.configure();
        self.inner.worker.retry();
        self.wake();
        Ok(())
    }

    pub async fn pause(&self) -> Result<(), CoreError> {
        self.configure();
        self.inner.worker.pause();
        self.wake();
        Ok(())
    }

    pub async fn resume(&self) -> Result<(), CoreError> {
        self.configure();
        self.inner.worker.resume();
        self.wake();
        Ok(())
    }

    pub async fn set_quality(&self, quality: &str) -> Result<(), CoreError> {
        let parsed = Quality::parse(Some(quality)).ok_or_else(|| CoreError::Invalid {
            detail: format!("qualité inconnue : « {quality} »"),
        })?;
        self.inner.worker.set_quality(parsed);
        self.wake();
        Ok(())
    }

    pub async fn set_offline_only(&self, offline_only: bool) -> Result<(), CoreError> {
        self.inner.worker.set_offline_only(offline_only);
        self.wake();
        Ok(())
    }

    pub async fn set_play_cache_cap(&self, bytes: i64) -> Result<(), CoreError> {
        if bytes <= 0 {
            return Err(CoreError::Invalid {
                detail: "capacité de cache invalide".to_owned(),
            });
        }
        self.inner.worker.set_play_cache_cap(bytes);
        self.wake();
        Ok(())
    }

    /// Empty the play cache. Downloads are untouched: they are not a cache, and
    /// nothing but the person deletes one.
    pub async fn clear_play_cache(&self) -> Result<(), CoreError> {
        self.inner.worker.clear_play_cache();
        self.wake();
        Ok(())
    }

    /// Where a downloaded track is, at the quality it really is.
    pub fn local(&self, track: &str) -> Option<(PathBuf, Quality)> {
        self.inner.worker.local(track)
    }

    /// Every published title, as the shell's rows draw it (id first).
    pub fn downloaded_ids(&self) -> Vec<String> {
        self.inner
            .worker
            .snapshot()
            .downloaded
            .iter()
            .cloned()
            .collect()
    }

    pub fn subscribe(&self, sink: Arc<dyn ProgressSink>) {
        lock(&self.inner.listeners).push(Arc::clone(&sink));
        // The first value at once: a screen that subscribes after a download
        // started must not wait for the next change to paint (rule 2). The
        // listener is called with NO lock held: a Kotlin callback that reads
        // the state back must not deadlock the worker.
        let state = self.state();
        sink.progress(&state);
    }

    /// The last rate the link meter measured, in bytes per second — the number
    /// the acceptance prints.
    pub fn stream_rate(&self) -> u64 {
        lock(&self.inner.meter).rate() as u64
    }

    /// The cap handed to `nz-api`'s Bulk lane, in bytes per second.
    pub fn bulk_cap(&self) -> Option<u64> {
        *lock(&self.inner.cap)
    }

    /// Read the client's counted bytes and hand `nz-api` the cap that follows.
    fn sample_link(&self) {
        let Ok(api) = self.inner.core.api() else {
            return;
        };
        // The untagged client is the player's and the core's own: a download is
        // tagged `downloads` and a set read `offline`, so neither is measured as
        // the stream it must yield to.
        let total: u64 = api
            .stats()
            .into_iter()
            .filter(|stats| stats.tag == nz_api::TAG_NONE)
            .map(|stats| stats.bytes_in)
            .sum();
        let now = Instant::now();
        let (rate, stale) = {
            let mut meter = lock(&self.inner.meter);
            (meter.observe(now, total), meter.stale(now))
        };
        if let Some(rate) = rate.filter(|rate| *rate > 0.0) {
            api.set_bulk_cap_from_stream(Some(rate as u64));
            *lock(&self.inner.cap) = nz_api::bulk_cap_from_stream(Some(rate as u64));
        } else if stale || rate == Some(0.0) {
            api.set_bulk_cap_from_stream(None);
            *lock(&self.inner.cap) = None;
        }
    }

    fn publish(&self, state: &OfflineState) {
        let listeners: Vec<Arc<dyn ProgressSink>> = lock(&self.inner.listeners).clone();
        if listeners.is_empty() {
            return;
        }
        for listener in listeners {
            listener.progress(state);
        }
    }

    fn publish_failure(&self, key: &str, cause: &str) {
        let listeners: Vec<Arc<dyn ProgressSink>> = lock(&self.inner.listeners).clone();
        for listener in listeners {
            listener.failure(key, cause);
        }
    }
}

/// The loop: sample the link while a download is queued, and publish when the
/// worker's version moved. Nothing is published on a timer — the version is what
/// decides (`nz-offline` bumps it only on change).
fn run(inner: Arc<Inner>) {
    let mut seen_version = 0u64;
    let mut seen_failures: HashSet<String> = HashSet::new();
    loop {
        if inner.stop.load(Ordering::Acquire) {
            return;
        }
        let busy = {
            let snapshot = inner.worker.snapshot();
            snapshot.progress.pending + snapshot.progress.active > 0 && !inner.worker.paused()
        };
        {
            let gate = lock(&inner.gate);
            let _ = inner
                .wake
                .wait_timeout(gate, if busy { BUSY_POLL } else { IDLE_POLL })
                .unwrap_or_else(|error| error.into_inner());
        }
        if inner.stop.load(Ordering::Acquire) {
            return;
        }
        if busy {
            sample_link_inner(&inner);
        }
        let snapshot = inner.worker.snapshot();
        if snapshot.version != seen_version {
            seen_version = snapshot.version;
            let offline = Offline {
                inner: Arc::clone(&inner),
            };
            let state = offline.state();
            offline.publish(&state);
            let failures: HashSet<String> = snapshot
                .failures
                .iter()
                .map(|failure| failure.track.clone())
                .collect();
            for failure in &snapshot.failures {
                if !seen_failures.contains(&failure.track) {
                    offline.publish_failure(&failure.track, &failure.cause);
                }
            }
            seen_failures = failures;
        }
    }
}

fn sample_link_inner(inner: &Arc<Inner>) {
    let offline = Offline {
        inner: Arc::clone(inner),
    };
    offline.sample_link();
}

fn lock<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic in another task must not turn every later call into a panic: what
    // is behind these locks is plain data.
    lock.lock().unwrap_or_else(|error| error.into_inner())
}

/// The labels of every set, read once at start (`LABELS` is the phone's own
/// store: one blob per set, so a 4 000-track playlist is one statement).
fn load_labels(cache: &nz_cache::Cache) -> HashMap<String, QueuedSet> {
    let contexts: Vec<String> = cache.kv().get(KEY_SETS).unwrap_or_default();
    let mut labels = HashMap::new();
    for context in contexts {
        let key = format!("{KEY_SET_PREFIX}{context}");
        if let Some(queued) = cache.kv().get::<QueuedSet>(&key)
            && !queued.tracks.is_empty()
        {
            labels.insert(context, queued);
        }
    }
    labels
}

/// Persist the labels: one statement per set, and the index.
fn save_labels(cache: &nz_cache::Cache, labels: &HashMap<String, QueuedSet>) {
    let contexts: Vec<String> = labels.keys().cloned().collect();
    for (context, queued) in labels {
        let key = format!("{KEY_SET_PREFIX}{context}");
        if let Err(error) = cache.kv().set(&key, queued) {
            warn!(context, %error, "libellés des téléchargements non enregistrés");
        }
    }
    if let Err(error) = cache.kv().set(KEY_SETS, &contexts) {
        warn!(%error, "index des téléchargements non enregistré");
    }
}

fn find_label<'a>(labels: &'a HashMap<String, QueuedSet>, track: &str) -> Option<&'a TrackLabel> {
    labels
        .values()
        .flat_map(|queued| queued.tracks.iter())
        .find(|label| label.id == track)
}

/// One French sentence per failed title, with the contexts that wanted it.
fn failure_index(
    failures: &[nz_offline::Failure],
    labels: &HashMap<String, QueuedSet>,
) -> Vec<(String, Vec<String>)> {
    if failures.is_empty() {
        return Vec::new();
    }
    failures
        .iter()
        .map(|failure| {
            let contexts: Vec<String> = labels
                .iter()
                .filter(|(_, queued)| queued.tracks.iter().any(|label| label.id == failure.track))
                .map(|(context, _)| context.clone())
                .collect();
            let title = if failure.title.is_empty() {
                failure.track.clone()
            } else {
                failure.title.clone()
            };
            (format!("{title} — {}", failure.cause), contexts)
        })
        .collect()
}
