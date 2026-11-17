//! What a screen reads: the published snapshot, and the words in it.
//!
//! [`Snapshot`] is a small, cloned value: version, what is running with its byte
//! count, the whole workload, every failure with its CAUSE (rule 2), the
//! settings, and the two caches' sizes. The worker republishes it off the UI
//! thread and a screen reads it with a mutex and a clone — never a SQLite
//! statement on the GPUI thread (`nz-cache`'s own rule).
//!
//! Moved here verbatim from `desktop/app/src/wiring/offline.rs` (SC4).

use std::collections::HashMap;

use nz_api::Quality;
use nz_cache::{DownloadProgress, DownloadSet};

use crate::settings::Settings;

// --- what a screen (and the badge) reads ------------------------------------

/// Where the worker stands. Every variant has a sentence: rule 2, a job that
/// runs says so, and a job that cannot run says why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkerState {
    Idle,
    Running,
    /// The person asked for a pause; the track in flight finishes first.
    Pausing,
    Paused,
    Offline,
    /// No session (or no local store): there is nothing to download into.
    Unavailable,
    /// The last sweep or save failed, with its cause.
    Blocked(String),
}

impl WorkerState {
    pub fn label(&self) -> &str {
        match self {
            WorkerState::Idle => "À jour",
            WorkerState::Running => "Téléchargement en cours",
            WorkerState::Pausing => "Pause demandée",
            WorkerState::Paused => "En pause",
            WorkerState::Offline => "Hors ligne : reprise automatique",
            WorkerState::Unavailable => "Aucune session ouverte",
            WorkerState::Blocked(cause) => cause,
        }
    }
}

/// One downloaded track that could not be fetched, with the cause the server or
/// the link gave (never a bare count).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub track: String,
    pub title: String,
    pub cause: String,
}

/// The track being written right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Running {
    pub track: String,
    pub title: String,
    pub quality: Quality,
    /// Bytes written so far, and the size the server announced (absent for a
    /// body generated as it is sent).
    pub bytes: u64,
    pub total: Option<u64>,
}

/// A title kept beside the download, so a list reads offline.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Labels {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
}

/// Everything the downloads screen and the badge draw, as one value.
///
/// `version` changes on every republication: a view compares it with the one it
/// painted and skips the frame when nothing moved.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub version: u64,
    pub state: WorkerState,
    pub settings: Settings,
    /// The whole workload: how many tracks are pending, being written, done,
    /// failed, and how many bytes are published.
    pub progress: DownloadProgress,
    /// The playlists, albums, shows and single tracks chosen for offline.
    pub sets: Vec<DownloadSet>,
    pub failures: Vec<Failure>,
    /// The labels of the single-track rows only: a set's row carries its own
    /// title, and every failure carries its own. Keeping the whole map here
    /// would clone four thousand entries on every publication.
    pub labels: HashMap<String, Labels>,
    pub running: Option<Running>,
    /// What the two evictable caches hold (their caps are in `settings`).
    pub play_cache_bytes: i64,
    pub covers_bytes: i64,
    /// What the last run did, in one sentence (rule 2).
    pub note: Option<String>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            version: 0,
            state: WorkerState::Idle,
            settings: Settings::default(),
            progress: DownloadProgress::default(),
            sets: Vec::new(),
            failures: Vec::new(),
            labels: HashMap::new(),
            running: None,
            play_cache_bytes: 0,
            covers_bytes: 0,
            note: None,
        }
    }
}

impl Snapshot {
    /// Tracks still to fetch, including the one in flight.
    pub fn remaining(&self) -> usize {
        self.progress.pending + self.progress.active
    }

    /// `12 / 200` for the run. The denominator counts what the queue ASKED for:
    /// a failure is a track that was wanted and is not there, so a run that ends
    /// with six failures reads `215 / 221`, not a reassuring `215 / 215`.
    pub fn done_label(&self) -> String {
        let total = self.progress.done + self.remaining() + self.progress.failed;
        format!("{} / {total}", self.progress.done)
    }

    /// The title of a downloaded single track, or its id when it was never
    /// labelled (a set added by an older build).
    pub fn title_of(&self, track: &str) -> String {
        self.labels
            .get(track)
            .map(|labels| labels.title.clone())
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| track.to_owned())
    }
}

// --- the work the UI hands to the worker ------------------------------------

/// One track a caller asks to download.
#[derive(Clone, Debug)]
pub struct Choice {
    pub track: String,
    pub title: String,
    pub artist: String,
    /// `None`: the quality of the settings.
    pub quality: Option<Quality>,
}

impl Choice {
    /// A track with only its id and title (a test, or a caller that has no
    /// credits at hand).
    pub fn new(track: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            track: track.into(),
            title: title.into(),
            artist: String::new(),
            quality: None,
        }
    }

    pub fn with_quality(mut self, quality: Quality) -> Self {
        self.quality = Some(quality);
        self
    }
}
