//! The settings a controller may MIRROR — and the ones it may never touch.
//!
//! `webapp/src/lib/remote/settings.js`'s rule, ported to the phone's store: the
//! controlled player publishes the device settings it lends, the controller shows
//! them in its own Réglages and sends back what it changes there. The list is
//! EXPLICIT — a setting is lent only by being named in [`MIRRORED`] — and it is
//! the SPA's own list cut down to what TWO gates let through:
//!
//! 1. **`nz-remote`'s own whitelist** (`host::decode_pending`): a `set` for a key
//!    its `MIRRORED` does not name is decoded as `Pending::Ignored` and the value
//!    is gone before this core sees it. That list is the DESKTOP's five keys
//!    (`player.quality`, `podcast.speeds`, `offline.quality`,
//!    `offline.onlyDownloaded`, `cache.limit`), and MB19 may not edit a shared
//!    crate: publishing a key the crate drops would show a controller a switch
//!    that does nothing, which is worse than not offering it. The `viz.*` keys
//!    the SPA lends a `read` controller therefore arrive with the shared-crate lot
//!    that parametrises that whitelist — named as a follow-up in the note.
//! 2. **this build's own store** (`settings::local::KNOWN`): `podcast.speeds`
//!    belongs to MB13 and is not a key this build has yet, so it is not published
//!    either (a controller would set a value nothing reads).
//!
//! Never lent, whatever the level: `viz.flash.ack`, `viz.lookahead` (a latency of
//! THIS device's output path), `audio.output`/`audio.glitchRoutes` (which path the
//! sound takes to THIS device's speakers), `ui.*` and `recent.*` (this device's
//! look and this person's history) and the transport (`player.volume`,
//! `player.muted`, `player.shuffle`, `player.repeat`), which travels as commands.
//!
//! The writes go through [`Core::set_local_setting`], so the validation Réglages
//! already applies (a value of the wrong SHAPE is refused with its French cause)
//! is the same one a controller meets — and a value a previous version wrote is
//! never re-read as something else.

use serde_json::{Value, json};

use crate::core::{Core, CoreError};
use crate::settings::LocalSettings;

/// The device settings a controller may see and change, in the SPA's spelling.
pub const MIRRORED: &[&str] = &[
    // the sound and the stream — a `full` loan may change these
    "player.quality",
    "offline.quality",
    "offline.onlyDownloaded",
    "cache.limit",
];

/// The level a setting needs: the animations from `read`, everything else from
/// `full` (`remote.py#READ_SETTING_PREFIXES`).
pub fn setting_level(key: &str) -> &'static str {
    if key.starts_with("viz.") {
        "read"
    } else {
        "full"
    }
}

/// Every mirrored setting as this device holds it, in the SPA's shape and
/// spellings — what a host publishes (`snapshotSettings`).
///
/// Read ONCE from the resolved [`LocalSettings`], so an unset key publishes its
/// DEFAULT: a controller that opens its own Réglages on a fresh player shows the
/// same values the owner's would, instead of an empty card.
pub fn snapshot(core: &Core) -> Value {
    let local = LocalSettings::read(core);
    json!({
        "player.quality": local.quality,
        "offline.quality": local.download_quality,
        "offline.onlyDownloaded": local.offline_only,
        "cache.limit": local.cache_limit,
        "cache.prefetch": local.prefetch,
        "cache.prefetchCount": local.prefetch_count,
    })
}

/// One mirrored setting as this device holds it now (`mirrored_value`), or `None`
/// for a key this build does not lend.
pub fn value(core: &Core, key: &str) -> Option<Value> {
    if !MIRRORED.contains(&key) {
        return None;
    }
    snapshot(core).get(key).cloned()
}

/// The keys two snapshots disagree on: the controller applies only those, so a
/// 1 Hz read writes nothing (the desktop's rule, and what keeps a phone's flash
/// and its SQLite writes to the one key the owner touched).
pub fn changed(current: &Value, next: &Value) -> Vec<String> {
    let mut keys = Vec::new();
    for key in MIRRORED {
        if current.get(*key) != next.get(*key) {
            keys.push((*key).to_owned());
        }
    }
    keys
}

/// Apply one `set` a controller sent: the same validated write Réglages makes,
/// returning the value this device actually holds afterwards (so a clamp or a
/// refusal is visible to the controller on its next read, instead of a lie).
///
/// `None` for a key this build does not lend — the honest answer, which the
/// controller's card shows as unchanged.
pub fn apply(core: &Core, key: &str, value: &Value) -> Option<Result<Value, CoreError>> {
    if !MIRRORED.contains(&key) {
        return None;
    }
    Some(core.set_local_setting(key, value.clone()))
}
