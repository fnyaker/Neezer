//! How fast a podcast plays — `webapp/src/lib/speed.js`, ported.
//!
//! The SPA keeps a rate per SHOW, never per episode and never for music: a
//! person who listens at 1,5× wants the next episode of that show at 1,5× too,
//! and nobody wants an album at 1,5×.
//!
//! Three rules, each one from the SPA:
//!
//! - **The key is the SHOW.** `channel_id`, else the album's provider id; an old
//!   queue may lack one, and then the rate is simply not remembered (rather than
//!   remembered for every episode of the app).
//! - **A value is only believed if the engine can play it**: 0.5–4, anything else
//!   reads as 1 (`sane`). A stored value written by a build whose list was
//!   different is a number, not a crash.
//! - **1 forgets the entry** — a rate nobody chose is not a setting to keep.
//!
//! The store is `nz-cache`'s typed key/value table (`core.cache().kv()`), the
//! SPA's `localStorage`: one JSON map under one key, so a change is one row.

use std::collections::BTreeMap;

use crate::core::{Core, CoreError};

/// The kv key the whole map lives under (the SPA's `podcastSpeeds` store).
pub const KV_KEY: &str = "nowplaying.podcast_speeds";

/// The rates the menu offers — `speed.js#SPEEDS`, smallest first.
pub const SPEEDS: [f32; 8] = [0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];

/// A stored rate, or 1: the SPA's `sane`, applied on the way IN and on the way
/// OUT (a value written by hand in the kv file is not trusted either).
pub(super) fn sane(rate: f32) -> f32 {
    if rate.is_finite() && (0.5..=4.0).contains(&rate) {
        rate
    } else {
        1.0
    }
}

/// The rate `show` is played at.
pub(super) fn speed_for(core: &Core, show: &str) -> f32 {
    let show = show.trim();
    if show.is_empty() {
        return 1.0;
    }
    map(core).get(show).map(|rate| sane(*rate)).unwrap_or(1.0)
}

/// Every remembered rate, show first — what the reconcile step reads.
pub(super) fn all(core: &Core) -> Vec<(String, f32)> {
    let mut rows: Vec<(String, f32)> = map(core)
        .into_iter()
        .map(|(show, rate)| (show, sane(rate)))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    rows
}

/// Remembers `rate` for `show`; 1 forgets it.
pub(super) fn set_speed(core: &Core, show: &str, rate: f32) -> Result<(), CoreError> {
    let show = show.trim();
    if show.is_empty() {
        // No show, no key: remembering it would play every episode of the app at
        // this rate, which is exactly what the per-show rule exists to prevent.
        return Ok(());
    }
    let mut speeds = map(core);
    let rate = sane(rate);
    if rate == 1.0 {
        speeds.remove(show);
    } else {
        speeds.insert(show.to_owned(), rate);
    }
    core.cache()
        .kv()
        .set(KV_KEY, &speeds)
        .map_err(|error| CoreError::Internal {
            detail: format!("{KV_KEY} : {error}"),
        })
}

/// The whole map, as the device has it (an unreadable entry reads as empty, rule
/// 7: a value written by an older build is not a crash).
fn map(core: &Core) -> BTreeMap<String, f32> {
    core.cache()
        .kv()
        .get::<BTreeMap<String, f32>>(KV_KEY)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::CoreConfig;
    use std::path::PathBuf;

    fn core(tag: &str) -> (PathBuf, Core) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nz-nowplaying-tests")
            .join(format!("{}-speed-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("data")).expect("data");
        std::fs::create_dir_all(path.join("cache")).expect("cache");
        let core = Core::new(CoreConfig {
            data_dir: path.join("data"),
            cache_dir: path.join("cache"),
            app_version: "0.0.0".into(),
            version_code: 2_000_001,
            sdk_int: 35,
            device_name: "test".into(),
        })
        .expect("the core");
        (path, core)
    }

    /// A rate is remembered per show, 1 forgets it, and an impossible rate is
    /// never stored.
    #[test]
    fn a_rate_belongs_to_its_show() {
        let (dir, core) = core("per-show");
        assert_eq!(speed_for(&core, "show-a"), 1.0);
        set_speed(&core, "show-a", 1.5).expect("store");
        set_speed(&core, "show-b", 2.0).expect("store");
        assert_eq!(speed_for(&core, "show-a"), 1.5);
        assert_eq!(speed_for(&core, "show-b"), 2.0);
        assert_eq!(
            all(&core),
            vec![("show-a".to_owned(), 1.5), ("show-b".to_owned(), 2.0)]
        );

        set_speed(&core, "show-a", 1.0).expect("forget");
        assert_eq!(speed_for(&core, "show-a"), 1.0);
        assert_eq!(all(&core), vec![("show-b".to_owned(), 2.0)]);

        set_speed(&core, "show-c", 9.0).expect("a wild rate");
        assert_eq!(
            speed_for(&core, "show-c"),
            1.0,
            "1 forgets, it does not store 9"
        );
        assert_eq!(speed_for(&core, ""), 1.0, "no show, no rate");

        core.shutdown();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The value survives a restart: it is the device's own store, not the
    /// process's memory (rule 7).
    #[test]
    fn a_rate_survives_a_reopen() {
        let (dir, core) = core("reopen");
        set_speed(&core, "42", 1.75).expect("store");
        core.shutdown();
        let again = Core::new(CoreConfig {
            data_dir: dir.join("data"),
            cache_dir: dir.join("cache"),
            app_version: "0.0.0".into(),
            version_code: 2_000_001,
            sdk_int: 35,
            device_name: "test".into(),
        })
        .expect("the core again");
        assert_eq!(speed_for(&again, "42"), 1.75);
        again.shutdown();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The menu's rates are the SPA's own list.
    #[test]
    fn the_offered_rates_are_the_spas() {
        assert_eq!(SPEEDS, [0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0]);
        for rate in SPEEDS {
            assert_eq!(sane(rate), rate);
        }
        assert_eq!(sane(0.2), 1.0);
        assert_eq!(sane(f32::NAN), 1.0);
    }
}
