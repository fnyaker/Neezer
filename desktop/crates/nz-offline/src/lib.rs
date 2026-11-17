//! `nz-offline` — the downloads worker, shared by the desktop app and the
//! mobile core.
//!
//! Extracted from `desktop/app/src/wiring/offline.rs` (SC4): everything that is
//! not GPUI lives here, and the desktop file is the adapter that re-exports this
//! surface and keeps its `cx`/`Entity` seam. The mobile core
//! (`mobile/rust/crates/nz-ffi/src/offline.rs`, MB15) calls the same queue, the
//! same store and the same causes.
//!
//! Four rules shape it, and each has its own module:
//!
//! - **One worker and one lane** ([`service`]). `nz-bulk` (one thread,
//!   `nice 10`) drains `nz_cache::Downloads`, and every byte it asks for is a
//!   `Lane::Bulk` request — so rule 1 is the gates' and not this file's: 1 s of
//!   quiet, one in flight, `X-NS-Background: 1`, and a body paced by the token
//!   bucket at 25 % of the playing stream's measured throughput. There is no
//!   second throttle here, on purpose. The worker is JOINED before the runtime
//!   it borrows goes (D13's pitfall, [`service::Worker`]).
//! - **A failure is never a verdict** ([`download::Outcome`]). Only the server's
//!   own `X-NS-Error` classification, a `4xx`, or a full disk marks a row
//!   `failed`, and then the CAUSE is stored and printed one row each (rule 2),
//!   with « Réessayer ». A dead link or a `503 + Retry-After` puts the row back
//!   to `pending` and the worker waits.
//! - **The screen never reads SQLite** ([`state::Snapshot`]). The worker
//!   publishes one cloned value off the UI thread; a view takes a mutex and
//!   compares a version. [`service::Offline::events`] is the same signal for a
//!   consumer that would rather wait than poll.
//! - **A file is published by rename once complete** ([`download::Downloader`]).
//!   The store reserves a `.part`, a fetcher fills it, `complete` renames it; a
//!   `.part` is never listed, and an interrupted row is `active` for the next
//!   start (rule 2).
//!
//! The settings are the SPA's own keys (`webapp/src/lib/stores.js`):
//! `offline.quality`, `offline.onlyDownloaded`, `cache.limit`. The picker offers
//! the four rungs the engine decodes (FLAC, Opus 320/128/64): the server also
//! answers `OPUS_256`/`OPUS_192`, but no decoder in either app has a rung for
//! them, so offering them would fetch a file the app cannot describe. That is
//! the one deliberate difference from `settings.svelte`'s six.
//!
//! No `#![forbid(unsafe_code)]`: [`service`] raises its worker's niceness through
//! a hand-written `nice` binding, the same one the desktop lot measured.
//!
//! Nothing here links GPUI, cpal, zbus or whisper, so the crate compiles for
//! Android (`cargo ndk -t arm64-v8a -P 26 check -p nz-offline`).

pub mod download;
pub mod service;
pub mod settings;
pub mod state;
pub mod testing;

pub use download::{ApiDownloader, Downloader, Outcome, Progress, Stop};
pub use service::{
    Offline, keep_downloaded, keep_downloaded_with, local_path, note_stream_bytes, settings,
    shutdown_process,
};
pub use settings::{
    COVERS_CAP_BYTES, DEFAULT_PLAY_CACHE_CAP, LABELS_KEY, OFFLINE_ONLY_KEY, PLAY_CACHE_CAP_KEY,
    PLAY_CACHE_CAPS, QUALITIES, QUALITY_KEY, Settings, bytes_label, quality_label,
};
pub use state::{Choice, Failure, Labels, Running, Snapshot, WorkerState};

#[cfg(test)]
mod tests;
