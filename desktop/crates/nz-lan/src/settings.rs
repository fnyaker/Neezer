//! The per-device LAN settings and their keys in `nz-cache`'s `kv`
//! (docs/agents/lan-nodes.md, *Device storage*).
//!
//! **LN7 filled `load` and `save`; the four key names were LN0's and are a
//! stored format.**
//!
//! They live in the cache's `kv` table, are synced nowhere, and are per device: a
//! phone may share its files while the PC does not, and the tolerated qualities of
//! a laptop on hotel Wi-Fi are not a server setting. The server's own two settings
//! (the feature switch and the credential lifetime) are admin settings, and they
//! live on the server (`/api/lan/settings`).
//!
//! Two rules this file keeps:
//!
//! - **A stored value that no longer reads falls back to its default, never to an
//!   error** (rule 7: an older build's spelling, or a corrupt row, must not keep
//!   the app from starting). The one value with a shape is the tolerated list, and
//!   it is clamped before it is trusted: at most the six qualities `nz-model`
//!   defines, so a hand-edited or corrupted row cannot allocate a list of
//!   arbitrary length.
//! - **Nothing here is written on a UI thread by force**: `save` is one
//!   statement per key on the caller's thread, which is why a failed write is
//!   reported and not retried — the setting applies for this session and Réglages
//!   says so.

use nz_cache::Cache;

use crate::service::LanSettings;
use crate::types::Tolerance;

/// « Échanger avec mes appareils à proximité » — the whole feature on this device.
pub const ENABLED: &str = "lan.enabled";
/// « Partager mes sons avec mes appareils » — whether this device serves at all.
pub const SERVE: &str = "lan.serve";
/// « Qualités tolérées » — what may be taken from a peer while the server answers.
pub const TOLERATED: &str = "lan.tolerated";
/// The UDP port, `0` = any (default `45910`).
pub const PORT: &str = "lan.port";

/// The most entries a stored tolerated list can contribute: `nz_model::Quality`
/// has six variants, so a seventh name is either a newer build's rung (ignored
/// harmlessly) or a corrupted row (not an allocation of its own length).
const MAX_TOLERATED_ENTRIES: usize = 16;

/// The settings as this device holds them, falling back to the defaults for any
/// key that was never written (or is unreadable: a corrupt value must not stop the
/// app from starting).
pub fn load(cache: &Cache) -> LanSettings {
    let kv = cache.kv();
    let defaults = LanSettings::default();
    LanSettings {
        enabled: kv.get::<bool>(ENABLED).unwrap_or(defaults.enabled),
        serve: kv.get::<bool>(SERVE).unwrap_or(defaults.serve),
        tolerated: tolerated(&kv),
        // `0` is a legitimate value ("any port"), so the fallback is only for a
        // row that is not a `u16` at all — `get` answers `None` then.
        port: kv.get::<u16>(PORT).unwrap_or(defaults.port),
    }
}

/// The tolerated list: wire names as `nz-model` spells them (`FLAC`, `OPUS_320`,
/// …), because that is what the plan's default is written in and what a peer's
/// `WireQuality` carries.
///
/// An empty or entirely unknown list is the DEFAULT set, not "nothing": the
/// decision (`decide.rs`) reads an empty set as "no peer quality is ever
/// tolerated", which is a different feature from the one the person turned on.
fn tolerated(kv: &nz_cache::Kv<'_>) -> Tolerance {
    let Some(names) = kv.get::<Vec<String>>(TOLERATED) else {
        return Tolerance::default_set();
    };
    let parsed: Tolerance = Tolerance(
        names
            .into_iter()
            .filter_map(|name| nz_api::Quality::parse(Some(name.as_str())))
            .take(MAX_TOLERATED_ENTRIES)
            .collect(),
    );
    if parsed.0.is_empty() {
        Tolerance::default_set()
    } else {
        parsed
    }
}

/// Writes the four keys. Silent when it succeeds; a failed write is not fatal
/// either — the setting applies for this session and is reported in Réglages.
pub fn save(cache: &Cache, s: &LanSettings) {
    let kv = cache.kv();
    let names: Vec<&str> = s
        .tolerated
        .0
        .iter()
        // FLAC's `q=` value is the ABSENCE of `q`; the stored spelling is its
        // name, exactly as the default set and the Réglages checkboxes say it.
        .map(|quality| quality.wire().unwrap_or("FLAC"))
        .collect();
    for (key, written) in [
        (ENABLED, kv.set(ENABLED, &s.enabled)),
        (SERVE, kv.set(SERVE, &s.serve)),
        (TOLERATED, kv.set(TOLERATED, &names)),
        (PORT, kv.set(PORT, &s.port)),
    ] {
        if let Err(error) = written {
            tracing::warn!(key, %error, "réglage LAN non enregistré");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nz_api::Quality;
    use std::path::PathBuf;

    /// A cache under the WORKSPACE's `target/`, never `/tmp` (the worker rule
    /// that keeps a build machine's RAM free).
    fn cache(tag: &str) -> (Cache, PathBuf) {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nz-lan-tests")
            .join(format!("settings-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("test dir");
        (Cache::open(&dir).expect("cache"), dir)
    }

    /// The key names are a stored format: changing one silently resets everybody's
    /// setting, so they are written down once and asserted.
    #[test]
    fn the_settings_keys_are_the_stored_format() {
        assert_eq!(ENABLED, "lan.enabled");
        assert_eq!(SERVE, "lan.serve");
        assert_eq!(TOLERATED, "lan.tolerated");
        assert_eq!(PORT, "lan.port");
    }

    /// The note's defaults: sharing on, serving on, the three tolerated rungs, and
    /// the port the firewall line names.
    #[test]
    fn the_defaults_are_shared_serve_and_three_rungs() {
        let settings = LanSettings::default();
        assert!(settings.enabled);
        assert!(settings.serve);
        assert_eq!(settings.port, 45910);
        assert!(settings.tolerated.contains(nz_api::Quality::Flac));
        assert!(!settings.tolerated.contains(nz_api::Quality::Opus64));
    }

    /// Nothing written yet reads as the defaults, and a read never creates a row:
    /// the four keys exist only once the person has changed something.
    #[test]
    fn an_untouched_cache_reads_the_defaults_and_writes_nothing() {
        let (cache, dir) = cache("empty");
        let loaded = load(&cache);
        assert_eq!(loaded, LanSettings::default());
        for key in [ENABLED, SERVE, TOLERATED, PORT] {
            assert!(
                !cache.kv().contains(key),
                "{key} must not be created by a read"
            );
        }
        drop(cache);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A round trip through the four keys: what `save` wrote is what `load`
    /// reads, including `port = 0` ("any port") which is a VALUE, not a missing
    /// setting.
    #[test]
    fn a_round_trip_reads_back_exactly_what_was_saved() {
        let (cache, dir) = cache("roundtrip");
        let settings = LanSettings {
            enabled: false,
            serve: false,
            tolerated: Tolerance([Quality::Opus320, Quality::Opus64].into_iter().collect()),
            port: 0,
        };
        save(&cache, &settings);
        assert_eq!(load(&cache), settings);
        // The stored spelling is `nz-model`'s wire name (FLAC is its name, since
        // its `q=` value is the absence of `q`), in the set's own order — which
        // IS the quality order, so the row is stable across saves.
        assert_eq!(
            cache.kv().get::<Vec<String>>(TOLERATED),
            Some(vec!["OPUS_64".to_owned(), "OPUS_320".to_owned()])
        );
        drop(cache);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// RULE 7 on the device: a row written by another build (or corrupted) reads
    /// as its default instead of failing the boot, and an empty tolerated list is
    /// the DEFAULT set — an empty set would mean "never take anything from a
    /// peer", which is a different feature from the one the person switched on.
    #[test]
    fn a_corrupt_row_falls_back_to_its_default() {
        let (cache, dir) = cache("corrupt");
        let kv = cache.kv();
        kv.set(ENABLED, &"yes").expect("a stored string");
        kv.set(TOLERATED, &Vec::<String>::new())
            .expect("an empty list");
        kv.set(PORT, &"quatre mille").expect("a stored string");
        let loaded = load(&cache);
        assert!(loaded.enabled, "not a bool -> the default, not false");
        assert_eq!(loaded.port, 45910);
        assert_eq!(loaded.tolerated, Tolerance::default_set());
        drop(cache);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A tolerated list longer than the qualities this build knows is clamped
    /// before allocation and keeps only what parses: the set can never hold more
    /// than the six rungs `nz-model` defines, whatever the row says.
    #[test]
    fn an_oversized_or_unknown_tolerated_list_is_clamped() {
        let (cache, dir) = cache("clamped");
        let names: Vec<String> = std::iter::once("OPUS_128".to_owned())
            .chain((0..64).map(|index| format!("NOT_A_QUALITY_{index}")))
            .chain(["FLAC", "OPUS_320", "OPUS_64", "OPUS_192", "OPUS_256"].map(str::to_owned))
            .collect();
        cache.kv().set(TOLERATED, &names).expect("stored list");
        // 70 names in; the six known rungs out, none duplicated and none lost.
        let all_six = Tolerance(
            [
                Quality::Opus64,
                Quality::Opus128,
                Quality::Opus192,
                Quality::Opus256,
                Quality::Opus320,
                Quality::Flac,
            ]
            .into_iter()
            .collect(),
        );
        assert_eq!(load(&cache).tolerated, all_six);

        // Only unknown names: the default set, never an empty one.
        cache
            .kv()
            .set(TOLERATED, &vec!["BOGUS".to_owned()])
            .expect("stored list");
        assert_eq!(load(&cache).tolerated, Tolerance::default_set());
        drop(cache);
        let _ = std::fs::remove_dir_all(dir);
    }
}
