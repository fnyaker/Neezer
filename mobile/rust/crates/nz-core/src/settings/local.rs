//! This device's own settings: the `kv` table, the SPA's keys, its defaults.
//!
//! Réglages writes here, and every other screen READS here, which is why the
//! vocabulary is a whitelist and not "any string the UI sends": a key outside
//! [`KNOWN`] is refused with the French cause (rule 3), and a value of the wrong
//! shape is refused before it is stored. The names are `setup::migration`'s own
//! `KNOWN_SETTINGS` — the keys the device already took over from the SPA
//! (MB4) — so a person who migrated finds their theme, their quality and their
//! prefetch count exactly where they left them (rule 7).
//!
//! # One rule a measurement taught (recorded in the note's MB16 section)
//!
//! A setting is a PREFERENCE, never a request: [`LocalSettings::read`] answers
//! in one `SELECT` per key on an indexed column — **197.6 µs for all 17 of
//! them** (measured 2026-11-18, this machine, debug build, 200 runs) — and
//! never touches the network, which is what lets the first frame of a launch
//! already be Papier when the person chose Papier
//! (`webapp/src/public/theme-boot.js`'s rule, ported to a launch).

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::core::{Core, CoreError};

/// Every local key this app stores, the SPA's own spelling. A key outside this
/// list is refused by [`Core::set_local_setting`].
pub const KNOWN: &[&str] = &[
    "ui.theme",
    "ui.accent",
    "ui.coverAc",
    "viz.eco",
    "viz.mode",
    "viz.palette",
    "viz.intensity",
    "viz.fps",
    "viz.beat",
    "viz.scope.orientation",
    "viz.scope.colour",
    "player.quality",
    "offline.quality",
    "offline.onlyDownloaded",
    "cache.limit",
    "cache.prefetch",
    "cache.prefetchCount",
];

/// `lib/theme.js#THEMES` — the three Sillon themes, Encre first.
pub const THEMES: &[&str] = &["encre", "papier", "pochette"];
/// `lib/theme.js#DEFAULT_THEME`.
pub const DEFAULT_THEME: &str = "encre";
/// `Settings.svelte#QUALITIES` — the six rates a download (or the player) may
/// ask for, in the order the sheet shows them.
pub const QUALITIES: &[&str] = &[
    "FLAC", "OPUS_320", "OPUS_256", "OPUS_192", "OPUS_128", "OPUS_64",
];
/// `Settings.svelte#CACHE_LIMITS` — the five caps the cache card offers.
pub const CACHE_LIMITS: &[u64] = &[
    256 * 1024 * 1024,
    512 * 1024 * 1024,
    1024 * 1024 * 1024,
    2 * 1024 * 1024 * 1024,
    4 * 1024 * 1024 * 1024,
];
/// `stores.js#PREFETCH_MAX` — how far ahead the prefetcher may be told to keep.
pub const PREFETCH_MAX: u32 = 10;
/// `Settings.svelte#AHEAD_STEPS`: 1..=PREFETCH_MAX, one tap each.
pub const VIZ_MODES: &[&str] = &["bars", "scope"];
/// `stores.js#vizFps`: the two caps the animations card offers.
pub const VIZ_FPS: &[u32] = &[30, 60];

/// This device's preferences, resolved: every field is a real value, the
/// SPA's default when the person never chose one (`stores.js`).
#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
pub struct LocalSettings {
    /// `ui.theme` — `encre`, `papier` or `pochette`.
    pub theme: String,
    /// `ui.accent` — the person's own colour (`#RRGGBB`), `None` for the theme's.
    pub accent: Option<String>,
    /// `ui.coverAc` — the last sleeve colour Pochette painted, so a launch
    /// paints the right accent before any track is known.
    pub cover_accent: Option<String>,
    /// `viz.eco` — the eco switch: no animation at all, anywhere.
    pub eco: bool,
    /// `viz.mode` — `bars` or `scope`.
    pub viz_mode: String,
    /// `viz.palette` — `cover`, `duo`, `mono`, …
    pub viz_palette: String,
    /// `viz.intensity` — 0..1.
    pub viz_intensity: f64,
    /// `viz.fps` — 30 or 60.
    pub viz_fps: u32,
    /// `viz.beat` — the beat detector feeds the visualiser.
    pub viz_beat: bool,
    /// `viz.scope.orientation` — `horizontal` or `vertical`.
    pub viz_scope_orientation: String,
    /// `viz.scope.colour` — `duo`, `mono`, …
    pub viz_scope_colour: String,
    /// `player.quality` — what the player asks the server for.
    pub quality: String,
    /// `offline.quality` — what a download asks for (`OPUS_320` by default: it
    /// is the one rate the SPA recommends for offline).
    pub download_quality: String,
    /// `offline.onlyDownloaded` — an offline play queues only what is on disk.
    pub offline_only: bool,
    /// `cache.limit` — the play cache's cap, in bytes.
    pub cache_limit: u64,
    /// `cache.prefetch` — keep the next titles on the device.
    pub prefetch: bool,
    /// `cache.prefetchCount` — how many, 1..=`PREFETCH_MAX`.
    pub prefetch_count: u32,
}

impl Default for LocalSettings {
    /// `stores.js`'s own initial values, one for one.
    fn default() -> Self {
        LocalSettings {
            theme: DEFAULT_THEME.to_owned(),
            accent: None,
            cover_accent: None,
            eco: false,
            viz_mode: "bars".to_owned(),
            viz_palette: "cover".to_owned(),
            viz_intensity: 0.7,
            viz_fps: 60,
            viz_beat: true,
            viz_scope_orientation: "horizontal".to_owned(),
            viz_scope_colour: "duo".to_owned(),
            quality: "FLAC".to_owned(),
            download_quality: "OPUS_320".to_owned(),
            offline_only: true,
            cache_limit: 1024 * 1024 * 1024,
            prefetch: true,
            prefetch_count: 1,
        }
    }
}

impl LocalSettings {
    /// The device's settings, every key read once. Disk only, no request.
    pub fn read(core: &Core) -> LocalSettings {
        let kv = core.cache().kv();
        let defaults = LocalSettings::default();
        LocalSettings {
            theme: kv
                .get::<String>("ui.theme")
                .filter(|name| THEMES.contains(&name.as_str()))
                .unwrap_or(defaults.theme),
            accent: kv
                .get::<String>("ui.accent")
                .and_then(|hex| normalize_accent(&hex)),
            cover_accent: kv
                .get::<String>("ui.coverAc")
                .and_then(|hex| normalize_accent(&hex)),
            eco: kv.get::<bool>("viz.eco").unwrap_or(defaults.eco),
            viz_mode: kv
                .get::<String>("viz.mode")
                .filter(|mode| VIZ_MODES.contains(&mode.as_str()))
                .unwrap_or(defaults.viz_mode),
            viz_palette: kv
                .get::<String>("viz.palette")
                .unwrap_or(defaults.viz_palette),
            viz_intensity: kv
                .get::<f64>("viz.intensity")
                .map(|value| value.clamp(0.0, 1.0))
                .unwrap_or(defaults.viz_intensity),
            viz_fps: kv
                .get::<u32>("viz.fps")
                .filter(|fps| VIZ_FPS.contains(fps))
                .unwrap_or(defaults.viz_fps),
            viz_beat: kv.get::<bool>("viz.beat").unwrap_or(defaults.viz_beat),
            viz_scope_orientation: kv
                .get::<String>("viz.scope.orientation")
                .unwrap_or(defaults.viz_scope_orientation),
            viz_scope_colour: kv
                .get::<String>("viz.scope.colour")
                .unwrap_or(defaults.viz_scope_colour),
            quality: kv
                .get::<String>("player.quality")
                .filter(|q| QUALITIES.contains(&q.as_str()))
                .unwrap_or(defaults.quality),
            download_quality: kv
                .get::<String>("offline.quality")
                .filter(|q| QUALITIES.contains(&q.as_str()))
                .unwrap_or(defaults.download_quality),
            offline_only: kv
                .get::<bool>("offline.onlyDownloaded")
                .unwrap_or(defaults.offline_only),
            cache_limit: kv
                .get::<u64>("cache.limit")
                .filter(|bytes| CACHE_LIMITS.contains(bytes))
                .unwrap_or(defaults.cache_limit),
            prefetch: kv
                .get::<bool>("cache.prefetch")
                .unwrap_or(defaults.prefetch),
            prefetch_count: kv
                .get::<u32>("cache.prefetchCount")
                .map(|count| count.clamp(1, PREFETCH_MAX))
                .unwrap_or(defaults.prefetch_count),
        }
    }
}

/// `lib/theme.js#normalizeAccent`: `#D2FF3C`, `d2ff3c` and `#d2f` are the same
/// colour; anything else is not a colour and must never reach the theme.
pub fn normalize_accent(raw: &str) -> Option<String> {
    let mut hex = raw.trim().trim_start_matches('#').to_owned();
    if hex.len() == 3 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        let mut doubled = String::with_capacity(6);
        for glyph in hex.chars() {
            doubled.push(glyph);
            doubled.push(glyph);
        }
        hex = doubled;
    }
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("#{}", hex.to_uppercase()))
}

/// What a stored value must be for one key, checked BEFORE it is written: a
/// settings store that accepts a number where the theme goes paints nothing and
/// says nothing (the SPA's own bug class, `theme.js#readTheme`).
fn check(key: &str, value: &Value) -> Result<Value, CoreError> {
    let invalid = |why: &str| CoreError::Invalid {
        detail: format!("{key} : {why}"),
    };
    match key {
        "ui.theme" => {
            let name = value
                .as_str()
                .ok_or_else(|| invalid("un nom de thème était attendu"))?;
            if !THEMES.contains(&name) {
                return Err(invalid("thème inconnu (encre, papier, pochette)"));
            }
            Ok(Value::String(name.to_owned()))
        }
        "ui.accent" | "ui.coverAc" => {
            // `null` is a real choice ("back to the theme's own accent").
            if value.is_null() {
                return Ok(Value::Null);
            }
            let raw = value
                .as_str()
                .ok_or_else(|| invalid("une couleur #RRGGBB était attendue"))?;
            let hex = normalize_accent(raw)
                .ok_or_else(|| invalid("couleur invalide : six chiffres hexadécimaux"))?;
            Ok(Value::String(hex))
        }
        "viz.eco" | "viz.beat" | "offline.onlyDownloaded" | "cache.prefetch" => value
            .as_bool()
            .map(Value::Bool)
            .ok_or_else(|| invalid("un booléen était attendu")),
        "viz.mode" => {
            let mode = value
                .as_str()
                .ok_or_else(|| invalid("un mode était attendu (bars, scope)"))?;
            if !VIZ_MODES.contains(&mode) {
                return Err(invalid("mode inconnu (bars, scope)"));
            }
            Ok(Value::String(mode.to_owned()))
        }
        "viz.palette" | "viz.scope.orientation" | "viz.scope.colour" => {
            let name = value
                .as_str()
                .ok_or_else(|| invalid("une valeur texte était attendue"))?;
            if name.len() > 32 {
                return Err(invalid("valeur trop longue"));
            }
            Ok(Value::String(name.to_owned()))
        }
        "viz.intensity" => {
            let level = value
                .as_f64()
                .ok_or_else(|| invalid("un nombre entre 0 et 1 était attendu"))?;
            Ok(Value::from(level.clamp(0.0, 1.0)))
        }
        "viz.fps" => {
            let fps = value
                .as_u64()
                .and_then(|fps| u32::try_from(fps).ok())
                .ok_or_else(|| invalid("un nombre d'images par seconde était attendu"))?;
            if !VIZ_FPS.contains(&fps) {
                return Err(invalid("30 ou 60 images par seconde"));
            }
            Ok(Value::from(fps))
        }
        "player.quality" | "offline.quality" => {
            let rate = value
                .as_str()
                .ok_or_else(|| invalid("une qualité était attendue"))?;
            if !QUALITIES.contains(&rate) {
                return Err(invalid("qualité inconnue (FLAC, OPUS_320, …)"));
            }
            Ok(Value::String(rate.to_owned()))
        }
        "cache.limit" => {
            let bytes = value
                .as_u64()
                .ok_or_else(|| invalid("une taille en octets était attendue"))?;
            if !CACHE_LIMITS.contains(&bytes) {
                return Err(invalid("256 Mo, 512 Mo, 1 Go, 2 Go ou 4 Go"));
            }
            Ok(Value::from(bytes))
        }
        "cache.prefetchCount" => {
            let count = value
                .as_u64()
                .and_then(|count| u32::try_from(count).ok())
                .ok_or_else(|| invalid("un nombre de titres était attendu"))?;
            if !(1..=PREFETCH_MAX).contains(&count) {
                return Err(invalid("entre 1 et 10 titres"));
            }
            Ok(Value::from(count))
        }
        _ => Err(CoreError::Unsupported {
            feature: format!("setting:{key}"),
        }),
    }
}

impl Core {
    /// This device's settings, resolved (one `SELECT` per key, no request).
    pub fn local_settings(&self) -> LocalSettings {
        LocalSettings::read(self)
    }

    /// One stored value as JSON text (`None` when it was never set) — what the
    /// Aide screen dumps and what a later lot reads without a typed accessor.
    pub fn local_setting(&self, key: &str) -> Option<String> {
        self.cache()
            .kv()
            .get::<Value>(key)
            .map(|value| value.to_string())
    }

    /// Writes one local setting, after [`check`] accepted its shape. The next
    /// screen already reads it: no request, nothing to roll back.
    pub fn set_local_setting(&self, key: &str, value: Value) -> Result<Value, CoreError> {
        let checked = check(key, &value)?;
        self.cache().kv().set(key, &checked)?;
        Ok(checked)
    }

    /// The generic typed read a lot may need for its OWN key (`ui.theme` from
    /// the shell before the first frame): `None` when unset or unreadable.
    pub fn setting<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.cache().kv().get::<T>(key)
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::TempDir;
    use super::*;

    /// The defaults are `stores.js`'s own, field by field: a device that never
    /// opened Réglages behaves exactly like a browser that never did.
    #[test]
    fn the_defaults_are_the_spas_own() {
        let dir = TempDir::new("local-defaults");
        let core = dir.core();
        let settings = core.local_settings();
        assert_eq!(settings.theme, "encre");
        assert_eq!(settings.accent, None);
        assert_eq!(settings.viz_intensity, 0.7);
        assert_eq!(settings.viz_fps, 60);
        assert_eq!(settings.download_quality, "OPUS_320");
        assert!(settings.offline_only);
        assert_eq!(settings.cache_limit, 1024 * 1024 * 1024);
        assert_eq!(settings.prefetch_count, 1);
        assert_eq!(settings, LocalSettings::default());
    }

    /// A written setting comes back typed, and a value of the wrong shape is
    /// refused with a French cause instead of stored.
    #[test]
    fn a_setting_round_trips_and_an_impossible_one_is_refused() {
        let dir = TempDir::new("local-write");
        let core = dir.core();

        core.set_local_setting("ui.theme", Value::from("papier"))
            .expect("a theme");
        assert_eq!(core.local_settings().theme, "papier");
        assert_eq!(
            core.local_setting("ui.theme").as_deref(),
            Some("\"papier\"")
        );

        let refused = core
            .set_local_setting("ui.theme", Value::from("fluo"))
            .expect_err("an unknown theme");
        assert!(matches!(refused, CoreError::Invalid { .. }), "{refused:?}");
        assert_eq!(
            core.local_settings().theme,
            "papier",
            "a refused write changes nothing"
        );

        assert!(matches!(
            core.set_local_setting("nope.key", Value::from(1)),
            Err(CoreError::Unsupported { .. })
        ));
        assert!(matches!(
            core.set_local_setting("cache.prefetchCount", Value::from(11)),
            Err(CoreError::Invalid { .. })
        ));
        assert!(matches!(
            core.set_local_setting("cache.limit", Value::from(123_u64)),
            Err(CoreError::Invalid { .. })
        ));
    }

    /// An accent is canonicalised the way `theme.js#normalizeAccent` does, and a
    /// value an older build stored in another spelling is still a colour.
    #[test]
    fn an_accent_is_canonicalised_or_refused() {
        assert_eq!(normalize_accent("#d2ff3c").as_deref(), Some("#D2FF3C"));
        assert_eq!(normalize_accent("d2f").as_deref(), Some("#DD22FF"));
        assert_eq!(normalize_accent("#d2f").as_deref(), Some("#DD22FF"));
        assert_eq!(normalize_accent(" #7c5cff ").as_deref(), Some("#7C5CFF"));
        assert_eq!(normalize_accent("bleu"), None);
        assert_eq!(normalize_accent("#12345"), None);

        let dir = TempDir::new("local-accent");
        let core = dir.core();
        core.set_local_setting("ui.accent", Value::from("#7c5cff"))
            .expect("a colour");
        assert_eq!(
            core.local_settings().accent.as_deref(),
            Some("#7C5CFF"),
            "stored canonical, so the theme never sees a second spelling"
        );
        // `null` is "back to the theme's own accent", never an error.
        core.set_local_setting("ui.accent", Value::Null)
            .expect("a reset");
        assert_eq!(core.local_settings().accent, None);
    }

    /// A stored value this build no longer accepts (a theme renamed, a quality
    /// dropped) reads as the default instead of failing the launch (rule 7).
    #[test]
    fn an_unknown_stored_value_reads_as_the_default() {
        let dir = TempDir::new("local-stale");
        let core = dir.core();
        // What an older build (or the SPA's migration) could have written.
        core.cache()
            .kv()
            .set("ui.theme", &Value::from("sepiA"))
            .expect("stored");
        core.cache()
            .kv()
            .set("player.quality", &Value::from("MP3"))
            .expect("stored");
        let settings = core.local_settings();
        assert_eq!(settings.theme, DEFAULT_THEME);
        assert_eq!(settings.quality, "FLAC");
    }
}
