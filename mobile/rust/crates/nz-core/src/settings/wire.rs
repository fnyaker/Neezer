//! The wire: the routes Réglages reads, their counted traffic, and the copy
//! this device keeps between two visits.
//!
//! Every call goes through `nz-api`'s TYPED endpoints, which run on
//! [`nz_api::Lane::User`] — the screen being opened is the action just taken,
//! never behind a prefetch or a download (rule 1). Nothing here decides a rule:
//! the shapes are `nz-model`'s (the same records the desktop decodes) and the
//! SPA's own component calls (`routes/Settings.svelte`, `SourceOrder`,
//! `MyAccounts`, `AccessSettings`, `ServicesSettings`, `DislikedTracks`) name
//! which route each one is.
//!
//! # The device's copy
//!
//! One JSON file per account, `settings-<hash>.json` in the CACHE directory
//! (evictable, like every cache — never in the app's own data), holding each
//! section as it was last read. A warm open of `Sources` therefore paints before
//! any request, and a dead link still shows the order the server last gave. The
//! file name is a hash of the account, never a server string.
//!
//! Two details that keep it honest:
//!
//! - a section is written ONLY when its read succeeded, so a failure never
//!   overwrites a good copy with an empty one;
//! - the file is written after the in-memory answer, by one atomic rename, so a
//!   process killed mid-write leaves the previous copy rather than half of one
//!   (rule 7 for the device's own data).

use std::path::PathBuf;
use std::sync::Arc;

use nz_api::Api;
use nz_model::{
    Access, DeezerStatus, MyAccounts, MyDislikes, ServerSettings, Services, Sources, Track,
};
use serde::{Deserialize, Serialize};

use super::{Core, CoreError, TAG_SETTINGS};

/// What one tag spent, as `nz-api` counts it (SC3): requests really sent, and
/// the response bodies' bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Totals {
    pub requests: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

/// The tag's running totals on this client.
pub(super) fn totals(api: &Api) -> Totals {
    api.stats()
        .into_iter()
        .find(|row| row.tag == TAG_SETTINGS)
        .map(|row| Totals {
            requests: row.requests,
            bytes_in: row.bytes_in,
            bytes_out: row.bytes_out,
        })
        .unwrap_or_default()
}

// --- the reads ---------------------------------------------------------------

/// `GET /api/sources` — the sources of sound in their order.
pub(super) async fn fetch_sources(api: &Api) -> Result<Sources, CoreError> {
    Ok(api.sources().await?)
}

/// `PUT /api/sources` — the admin's order and switches.
pub(super) async fn save_sources(
    api: &Api,
    change: &serde_json::Value,
) -> Result<Sources, CoreError> {
    api.set_sources(change).await?;
    // The write's answer carries no `admin`, so the list is read back: the
    // screen's ranks and places come from the server, never from the request
    // (rule 6 — a value is read back, not assumed).
    fetch_sources(api).await
}

/// `GET /api/me/accounts` — the person's own provider accounts.
pub(super) async fn fetch_accounts(api: &Api) -> Result<MyAccounts, CoreError> {
    Ok(api.my_accounts().await?)
}

/// `GET /api/access` — who may use which provider, and the groups a grant names.
pub(super) async fn fetch_access(api: &Api) -> Result<Access, CoreError> {
    Ok(api.access().await?)
}

/// `GET /api/services` — the export targets and the list formats.
pub(super) async fn fetch_services(api: &Api) -> Result<Services, CoreError> {
    Ok(api.services().await?)
}

/// `GET /api/settings` — the server's own settings (admin only; a member gets
/// `403`, which the screen shows as the cause and hides the card behind).
pub(super) async fn fetch_server_settings(api: &Api) -> Result<ServerSettings, CoreError> {
    Ok(api.get_settings().await?)
}

/// `GET /api/deezer/status` — is the provider's credential still good?
pub(super) async fn fetch_deezer_status(api: &Api, force: bool) -> Result<DeezerStatus, CoreError> {
    Ok(api.deezer_status(force).await?)
}

/// `GET /api/me/dislikes` — "Je n'aime pas".
pub(super) async fn fetch_dislikes(api: &Api) -> Result<Vec<Track>, CoreError> {
    let answer: MyDislikes = api.my_dislikes().await?;
    Ok(answer.tracks)
}

// --- the device's copy -------------------------------------------------------

/// Every section Réglages shows, as last read. A field is `None` until its own
/// read succeeded once (a failure never blanks what the person already saw).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct Snapshot {
    #[serde(default)]
    pub sources: Option<Sources>,
    #[serde(default)]
    pub accounts: Option<MyAccounts>,
    #[serde(default)]
    pub access: Option<Access>,
    #[serde(default)]
    pub services: Option<Services>,
    #[serde(default)]
    pub server: Option<ServerSettings>,
    #[serde(default)]
    pub deezer: Option<DeezerStatus>,
    #[serde(default)]
    pub dislikes: Option<Vec<Track>>,
}

impl Snapshot {
    /// The device's copy, or an empty one when the person never opened Réglages.
    pub(super) fn read(core: &Core) -> Snapshot {
        let path = snapshot_path(core);
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|body| serde_json::from_str(&body).ok())
            .unwrap_or_default()
    }

    /// Writes one section and keeps the others: a section's read is the only
    /// thing that may replace it.
    pub(super) fn write(core: &Core, change: impl FnOnce(&mut Snapshot)) {
        let mut snapshot = Snapshot::read(core);
        change(&mut snapshot);
        let path = snapshot_path(core);
        let Ok(body) = serde_json::to_vec(&snapshot) else {
            return;
        };
        // One rename: a process killed mid-write leaves the previous copy, never
        // half of one.
        let temporary = path.with_extension("json.part");
        if std::fs::write(&temporary, &body).is_ok() {
            let _ = std::fs::rename(&temporary, &path);
        }
    }
}

/// Where the account's copy lives: the CACHE directory (evictable), never the
/// app's own data.
fn snapshot_path(core: &Core) -> PathBuf {
    core.cache_path(&format!(
        "settings-{:016x}.json",
        fnv1a(owner(core).as_bytes())
    ))
}

/// The account the copy belongs to: `/me`'s id, `""` before a session.
fn owner(core: &Core) -> String {
    core.me()
        .map(|me| me.user.id.to_string())
        .filter(|id| !id.is_empty())
        .unwrap_or_default()
}

/// The copy's path, for a test that reads the file back (the ARL must never be
/// in it).
#[cfg(test)]
pub(super) fn snapshot_path_for_test(core: &Core) -> PathBuf {
    snapshot_path(core)
}

/// FNV-1a: a stable file name for an account id, and no dependency for it.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

impl Core {
    /// The tagged client of one Réglages read. The offline gate is the core's,
    /// so a dead link answers `Offline` in zero sockets.
    pub(super) fn settings_api(&self) -> Result<Arc<Api>, CoreError> {
        Ok(Arc::new(self.api()?.with_tag(TAG_SETTINGS)))
    }

    /// Books one section's traffic in the core's meter, so `NzCore::stats()`
    /// answers what this screen cost without the UI counting anything.
    pub(super) fn note_settings_traffic(&self, api: &Api, before: Totals) {
        let after = totals(api);
        let requests = after.requests.saturating_sub(before.requests);
        if requests == 0 {
            return;
        }
        // `Meter::record` counts ONE request and its bytes; the run's bytes go
        // with the first, the rest are zeros.
        self.meter().record(
            TAG_SETTINGS,
            after.bytes_in.saturating_sub(before.bytes_in),
            after.bytes_out.saturating_sub(before.bytes_out),
        );
        for _ in 1..requests {
            self.meter().record(TAG_SETTINGS, 0, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::testing::TempDir as CoreTempDir;

    /// The file name is a hash of the account, never the account id, and two
    /// accounts never share a copy.
    #[test]
    fn the_copy_is_named_after_a_hash_of_the_account() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_ne!(fnv1a(b"1"), fnv1a(b"2"));
        let path = format!("settings-{:016x}.json", fnv1a(b"42"));
        assert!(!path.contains("42"), "{path}");
    }

    /// A section written is a section read back, and writing one leaves the
    /// others in place.
    #[test]
    fn one_section_never_blanks_another() {
        let dir = CoreTempDir::new("settings-snapshot");
        let core = dir.core();
        Snapshot::write(&core, |snapshot| {
            snapshot.sources = Some(Sources {
                main: Some("deezer".into()),
                admin: true,
                ..Sources::default()
            });
        });
        Snapshot::write(&core, |snapshot| {
            snapshot.dislikes = Some(vec![Track {
                deezer_id: nz_model::TrackId::new("7"),
                title: "Titre".into(),
                ..Track::default()
            }]);
        });
        let snapshot = Snapshot::read(&core);
        assert_eq!(
            snapshot.sources.expect("sources").main.as_deref(),
            Some("deezer")
        );
        assert_eq!(snapshot.dislikes.expect("dislikes").len(), 1);
        assert!(snapshot.accounts.is_none());
    }
}
