//! The person's offline settings, and the words a screen prints.
//!
//! The keys are the SPA's own (`webapp/src/lib/stores.js`): `offline.quality`,
//! `offline.onlyDownloaded`, `cache.limit`. They live in `nz-cache`'s `kv`, so
//! a value set in one app is where the other reads it, and a settings write
//! that fails costs one preference, never the app.
//!
//! Moved here verbatim from `desktop/app/src/wiring/offline.rs` (SC4): a
//! setting is a fact about the downloads, not about GPUI.

use nz_api::Quality;
use nz_cache::Cache;
use tracing::warn;

// --- the person's settings (the SPA's own keys) -----------------------------

/// Quality of a new download (`offline.quality`).
pub const QUALITY_KEY: &str = "offline.quality";
/// « Hors-ligne : ne lire que les titres téléchargés » (`offline.onlyDownloaded`).
pub const OFFLINE_ONLY_KEY: &str = "offline.onlyDownloaded";
/// The play cache's cap, in bytes (`cache.limit`).
pub const PLAY_CACHE_CAP_KEY: &str = "cache.limit";
/// Where the titles of the chosen sets live, so a list reads without the server.
pub const LABELS_KEY: &str = "downloads.labels";

/// The cover cache's cap: the plan's 300 MB, enforced by the same sweep.
pub const COVERS_CAP_BYTES: i64 = 300 * 1024 * 1024;
/// The play cache's default: the SPA's 1 GB (`stores.js#playCacheLimit`).
pub const DEFAULT_PLAY_CACHE_CAP: i64 = 1024 * 1024 * 1024;

/// The cap choices `settings.svelte` offers, in bytes.
pub const PLAY_CACHE_CAPS: [(i64, &str); 5] = [
    (256 * 1024 * 1024, "256 Mo"),
    (512 * 1024 * 1024, "512 Mo"),
    (1024 * 1024 * 1024, "1 Go"),
    (2 * 1024 * 1024 * 1024, "2 Go"),
    (4 * 1024 * 1024 * 1024, "4 Go"),
];

/// The qualities the picker offers: the rungs `nz-audio` can decode.
pub const QUALITIES: [(Quality, &str, &str); 4] = [
    (Quality::Flac, "FLAC", "Sans perte — lourd"),
    (Quality::Opus320, "Opus 320", "Haute qualité (recommandé)"),
    (Quality::Opus128, "Opus 128", "Standard, léger"),
    (Quality::Opus64, "Opus 64", "Données réduites"),
];

/// What a quality is called in the UI.
pub fn quality_label(quality: Quality) -> &'static str {
    match quality {
        Quality::Flac => "FLAC",
        Quality::Opus320 => "Opus 320",
        Quality::Opus256 => "Opus 256",
        Quality::Opus192 => "Opus 192",
        Quality::Opus128 => "Opus 128",
        Quality::Opus64 => "Opus 64",
    }
}

/// `1,4 Go`, `812 ko`, `0 o`. A byte count is read, never computed in one's head.
pub fn bytes_label(bytes: u64) -> String {
    const UNITS: [(&str, u64); 3] = [("Go", 1 << 30), ("Mo", 1 << 20), ("ko", 1 << 10)];
    if bytes < 1024 {
        return format!("{bytes} o");
    }
    for (unit, size) in UNITS {
        if bytes >= size {
            let value = bytes as f64 / size as f64;
            return format!("{} {unit}", format!("{value:.1}").replace('.', ","));
        }
    }
    format!("{bytes} o")
}

/// The person's offline settings, mirrored from `kv` and published in a
/// [`Snapshot`] so a screen never reads SQLite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Quality of a new download.
    pub quality: Quality,
    /// « Hors-ligne : ne lire que les titres téléchargés ».
    pub offline_only: bool,
    /// The play cache's cap, in bytes.
    pub play_cache_cap: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            quality: Quality::Opus320,
            offline_only: true,
            play_cache_cap: DEFAULT_PLAY_CACHE_CAP,
        }
    }
}

impl Settings {
    pub(crate) fn read(cache: &Cache) -> Self {
        let kv = cache.kv();
        let quality = kv
            .get::<String>(QUALITY_KEY)
            .and_then(|raw| Quality::parse(Some(&raw)))
            .unwrap_or(Quality::Opus320);
        let offline_only = kv.get::<bool>(OFFLINE_ONLY_KEY).unwrap_or(true);
        let play_cache_cap = kv
            .get::<i64>(PLAY_CACHE_CAP_KEY)
            .filter(|cap| *cap > 0)
            .unwrap_or(DEFAULT_PLAY_CACHE_CAP);
        Self {
            quality,
            offline_only,
            play_cache_cap,
        }
    }

    pub(crate) fn write(&self, cache: &Cache) {
        let kv = cache.kv();
        // `FLAC` is spelled out rather than stored as the absence of a `q=`: the
        // key is read by a human in `sqlite3` as often as by the code.
        let wire = self.quality.wire().unwrap_or("FLAC").to_owned();
        // A settings write that fails costs one preference, not the app: the
        // cause is logged and the in-memory value stays what the person chose.
        for (key, result) in [
            (QUALITY_KEY, kv.set(QUALITY_KEY, &wire)),
            (
                OFFLINE_ONLY_KEY,
                kv.set(OFFLINE_ONLY_KEY, &self.offline_only),
            ),
            (
                PLAY_CACHE_CAP_KEY,
                kv.set(PLAY_CACHE_CAP_KEY, &self.play_cache_cap),
            ),
        ] {
            if let Err(error) = result {
                warn!(key, %error, "réglage hors ligne non enregistré");
            }
        }
    }
}
