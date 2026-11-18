//! The per-device LAN settings and their keys in `nz-cache`'s `kv`
//! (docs/agents/lan-nodes.md, *Device storage*).
//!
//! **LN0 declares the four keys, `load` and `save`; LN7 fills them.**
//!
//! They live in the cache's `kv` table, are synced nowhere, and are per device: a
//! phone may share its files while the PC does not, and the tolerated qualities of
//! a laptop on hotel Wi-Fi are not a server setting. The server's own two settings
//! (the feature switch and the credential lifetime) are admin settings, and they
//! live on the server (`/api/lan/settings`).

use nz_cache::Cache;

use crate::service::LanSettings;

/// « Échanger avec mes appareils à proximité » — the whole feature on this device.
pub const ENABLED: &str = "lan.enabled";
/// « Partager mes sons avec mes appareils » — whether this device serves at all.
pub const SERVE: &str = "lan.serve";
/// « Qualités tolérées » — what may be taken from a peer while the server answers.
pub const TOLERATED: &str = "lan.tolerated";
/// The UDP port, `0` = any (default `45910`).
pub const PORT: &str = "lan.port";

/// The settings as this device holds them, falling back to the defaults for any
/// key that was never written (or is unreadable: a corrupt value must not stop the
/// app from starting).
pub fn load(cache: &Cache) -> LanSettings {
    let _ = cache;
    LanSettings::default()
}

/// Writes the four keys. Silent when it succeeds; a failed write is not fatal
/// either — the setting applies for this session and is reported in Réglages.
pub fn save(cache: &Cache, s: &LanSettings) {
    let _ = (cache, s);
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
