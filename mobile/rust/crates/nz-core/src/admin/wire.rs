//! The wire: the administrator's routes, their counted traffic, and the copy
//! this device keeps between two visits.
//!
//! Every call goes through `nz-api`'s TYPED endpoints, on [`Lane::User`] — the
//! server page being opened is the action just taken, never behind a prefetch or
//! a job (rule 1). Nothing here decides a rule: the shapes are `nz-model`'s (the
//! same records the desktop decodes), and the SPA components name which route
//! each one is (`docs/agents/mobile.md`, MB17's table).
//!
//! Three routes needed a hand-written request because `nz-api`'s typed wrapper
//! throws away the answer this screen shows: `POST /api/cache/flush` (the freed
//! bytes), `PUT /api/automatch/settings` and `POST /api/automatch/cancel` (both
//! answer the whole `/api/automatch` payload). They use [`Api::send`], which is
//! the same machinery the wrappers use — same lane, same retry policy (a
//! mutation is never retried), same counters.
//!
//! # The device's copy
//!
//! One JSON file per account, `admin-<hash>.json` in the CACHE directory
//! (evictable, never the app's own data), holding each card as it was last read.
//! A warm open of *Serveur* therefore paints all eight cards before any request,
//! and a dead link still shows the disk it last reported. The file name is a hash
//! of the account, never a server string.
//!
//! Two details that keep it honest, both MB16's rule for the same reason:
//!
//! - a card is written ONLY when its read succeeded, so a failure never
//!   overwrites a good copy with an empty one;
//! - the file is written after the in-memory answer, by one atomic rename, so a
//!   process killed mid-write leaves the previous copy rather than half of one.

use std::path::PathBuf;
use std::sync::Arc;

use nz_api::{Api, Lane, Method};
use nz_model::{
    AdminInvites, AdminUsers, AnalysisBackfill, AnalysisPlan, ArchiveRules, ArchiveStatus,
    Automatch, CleanupPreview, CreditsBackfill, DeezerStatus, EnrichStatus, MigrateStatus,
    ServerSettings, SimilarStatus, Storage, TranscribeSettings, TranscribeStatus,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Core, CoreError, TAG_ADMIN};

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
        .find(|row| row.tag == TAG_ADMIN)
        .map(|row| Totals {
            requests: row.requests,
            bytes_in: row.bytes_in,
            bytes_out: row.bytes_out,
        })
        .unwrap_or_default()
}

// --- people ------------------------------------------------------------------

/// `GET /api/admin/users?q=&after=&limit=` — one page of accounts.
pub(super) async fn fetch_users(
    api: &Api,
    q: &str,
    after: Option<&str>,
    limit: u32,
) -> Result<AdminUsers, CoreError> {
    Ok(api.admin_users(q, after, limit).await?)
}

/// `GET /api/admin/users/<id>` — one account, with how much it uploaded and
/// made. The server's own envelope (`{"user": …, "uploads": n, "playlists": n}`).
pub(super) async fn fetch_user(api: &Api, id: &str) -> Result<Value, CoreError> {
    Ok(api.admin_user(id).await?)
}

/// `PATCH /api/admin/users/<id>` — `{"admin": bool}` or `{"disabled": bool}`.
pub(super) async fn patch_user(
    api: &Api,
    id: &str,
    payload: &Value,
) -> Result<Value, CoreError> {
    Ok(api.admin_user_patch(id, payload).await?)
}

/// `DELETE /api/admin/users/<id>` — the body confirms the account's NAME, which
/// is what makes the destructive action a deliberate one (`admin_users.py`).
pub(super) async fn delete_user(api: &Api, id: &str, name: &str) -> Result<Value, CoreError> {
    Ok(api.admin_user_delete(id, name).await?)
}

// --- invitations --------------------------------------------------------------

/// `GET /api/admin/invites?state=pending&limit=` — the list, newest first.
pub(super) async fn fetch_invites(
    api: &Api,
    state: Option<&str>,
    limit: u32,
    after_created: Option<&str>,
    after_id: Option<&str>,
) -> Result<AdminInvites, CoreError> {
    Ok(api
        .admin_invites(state, limit, after_created, after_id)
        .await?)
}

/// `POST /api/admin/invites` — mint one. The token is in the answer ONCE.
pub(super) async fn create_invite(
    api: &Api,
    note: Option<&str>,
    expires_in: u32,
) -> Result<Value, CoreError> {
    let mut body = serde_json::json!({ "expires_in": expires_in });
    if let Some(note) = note.map(str::trim).filter(|note| !note.is_empty()) {
        body["note"] = Value::from(note);
    }
    Ok(api.admin_invite_create(&body).await?)
}

/// `POST /api/admin/invites/<id>/renew` — revokes the old link and mints a new
/// one with the same duration (`invites.py#admin_invite_renew`).
pub(super) async fn renew_invite(api: &Api, id: &str) -> Result<Value, CoreError> {
    Ok(api.admin_invite_renew(id).await?)
}

/// `POST /api/admin/invites/<id>/revoke`.
pub(super) async fn revoke_invite(api: &Api, id: &str) -> Result<(), CoreError> {
    api.admin_invite_revoke(id).await?;
    Ok(())
}

// --- the archive ---------------------------------------------------------------

/// `GET /api/storage` — what the archive and the caches cost, and what is left.
pub(super) async fn fetch_storage(api: &Api) -> Result<Storage, CoreError> {
    Ok(api.storage().await?)
}

/// `GET /api/archive/status` — the backfill's progress and counters.
pub(super) async fn fetch_archive_status(api: &Api) -> Result<ArchiveStatus, CoreError> {
    Ok(api.archive_status().await?)
}

/// `POST /api/archive/backfill` — sweeps up what the events missed. Only adds.
pub(super) async fn start_archive(api: &Api, scope: &str) -> Result<ArchiveStatus, CoreError> {
    api.archive_backfill(scope).await?;
    // The answer is `{ok, running}`; the counters the card draws come from the
    // status route the poll already reads, so this is the same read the SPA's
    // `pollJob()` makes first (never a guess about `done`/`total`).
    fetch_archive_status(api).await
}

/// `GET /api/archive/rules` — the rules, their defaults and the vocabulary.
pub(super) async fn fetch_rules(api: &Api) -> Result<ArchiveRules, CoreError> {
    Ok(api.archive_rules().await?)
}

/// `POST /api/archive/rules` — ONE field at a time (each control is its own
/// decision, `Settings.svelte#saveRule`), then the server's whole state back.
pub(super) async fn save_rules(api: &Api, fields: &Value) -> Result<ArchiveRules, CoreError> {
    api.set_archive_rules(fields).await?;
    fetch_rules(api).await
}

/// `GET /api/archive/cleanup/preview` — what a cleanup would delete. Deletes
/// nothing, and is the only way this screen ever names a file.
pub(super) async fn fetch_cleanup(api: &Api) -> Result<CleanupPreview, CoreError> {
    Ok(api.cleanup_preview().await?)
}

/// `POST /api/archive/cleanup` — the ONE thing that deletes archived audio.
pub(super) async fn run_cleanup(api: &Api) -> Result<Value, CoreError> {
    Ok(api.run_cleanup().await?)
}

/// `POST /api/cache/flush` — empties the DERIVED caches and answers the bytes
/// it freed, which is the number the toast carries.
pub(super) async fn flush_cache(api: &Api) -> Result<u64, CoreError> {
    let answer: Option<Value> = api
        .send::<(), Value>(Lane::User, Method::POST, "/cache/flush", None)
        .await?;
    Ok(answer
        .as_ref()
        .and_then(|value| value.get("freed"))
        .and_then(Value::as_u64)
        .unwrap_or_default())
}

/// `POST /api/archive/migrate` — rewrites the archived files (a job).
pub(super) async fn start_migrate(api: &Api, force: bool) -> Result<MigrateStatus, CoreError> {
    api.archive_migrate(force).await?;
    api.archive_migrate_status().await.map_err(CoreError::from)
}

/// `GET /api/archive/migrate` — its progress, per-step counters and causes.
pub(super) async fn fetch_migrate(api: &Api) -> Result<MigrateStatus, CoreError> {
    Ok(api.archive_migrate_status().await?)
}

// --- the analysis --------------------------------------------------------------

/// `GET /api/analysis/plan` — what is out of date, at which step, and the
/// nightly window (`GenreLibrary.svelte#load`).
pub(super) async fn fetch_plan(api: &Api) -> Result<AnalysisPlan, CoreError> {
    Ok(api.analysis_plan().await?)
}

/// `POST /api/analysis/backfill` — measure the library now (gated server-side
/// by `workload.Throttle`: it pauses while anybody listens, rule 1).
pub(super) async fn start_analysis(
    api: &Api,
    force: bool,
    workers: u32,
) -> Result<super::AnalysisJob, CoreError> {
    api.analysis_backfill(force, workers).await?;
    fetch_analysis(api).await
}

/// `GET /api/analysis/backfill` — the job's state, stage and failure ledger.
///
/// Read through [`Api::send`] on a GET, and not through the typed endpoint,
/// because the route's `paused` is a WORD (`listening`, `download`, `load`) while
/// `nz-model` types it as a boolean: the typed struct would refuse the whole
/// payload the moment the job pauses — exactly when a person looks at it. The
/// split is [`split_analysis`]'s.
pub(super) async fn fetch_analysis(api: &Api) -> Result<super::AnalysisJob, CoreError> {
    let answer: Option<Value> = api
        .send::<(), Value>(Lane::User, Method::GET, "/analysis/backfill", None)
        .await?;
    Ok(answer.map(split_analysis).unwrap_or_default())
}

/// `POST /api/analysis/backfill/stop` — asks the job to stop after the track in
/// hand; it resumes from its checkpoint next time. Answers the new state.
pub(super) async fn stop_analysis(api: &Api) -> Result<super::AnalysisJob, CoreError> {
    api.analysis_backfill_stop().await?;
    fetch_analysis(api).await
}

/// The route's `paused` word, split from the counters the shared model reads.
fn split_analysis(mut answer: Value) -> super::AnalysisJob {
    let paused = match answer.get("paused") {
        Some(Value::String(word)) if !word.is_empty() => Some(word.clone()),
        // An older server answered `true`/`false` (the shape `nz-model` types);
        // anything truthy that is not a word still means "waiting".
        Some(Value::Bool(true)) => Some("paused".to_owned()),
        _ => None,
    };
    if let Some(object) = answer.as_object_mut() {
        object.insert("paused".to_owned(), Value::Bool(paused.is_some()));
    }
    // `unwrap_or_default` is deliberate and narrow: the counters are the card's
    // numbers, and a server that renames one must not turn a running job into a
    // `LoadFailed` (rule 7). The keys this build reads are the ones `nz-model`
    // declares, and the desktop's own fixtures pin them.
    let job: AnalysisBackfill = serde_json::from_value(answer).unwrap_or_default();
    super::AnalysisJob { job, paused }
}

/// `PUT /api/analysis/schedule` — the nightly window.
pub(super) async fn save_schedule(
    api: &Api,
    enabled: bool,
    at: &str,
    hours: u32,
) -> Result<AnalysisPlan, CoreError> {
    let body = serde_json::json!({ "enabled": enabled, "at": at, "hours": hours });
    api.analysis_schedule(&body).await?;
    fetch_plan(api).await
}

/// `PUT /api/analysis/where` — who measures: `server` or `client`.
pub(super) async fn save_where(api: &Api, place: &str) -> Result<AnalysisPlan, CoreError> {
    api.analysis_where(place).await?;
    fetch_plan(api).await
}

// --- the library-wide backfills -------------------------------------------------

/// `GET /api/enrich/backfill` — MusicBrainz facts' coverage.
pub(super) async fn fetch_enrich(api: &Api) -> Result<EnrichStatus, CoreError> {
    Ok(api.enrich_status().await?)
}

/// `POST /api/enrich/backfill[?force=1]`.
pub(super) async fn start_enrich(api: &Api, force: bool) -> Result<EnrichStatus, CoreError> {
    api.enrich_backfill(force).await?;
    fetch_enrich(api).await
}

/// `GET /api/credits/backfill` — the credits worker's counters.
pub(super) async fn fetch_credits(api: &Api) -> Result<CreditsBackfill, CoreError> {
    Ok(api.credits_backfill_status().await?)
}

/// `POST /api/credits/backfill[?force=1]`.
pub(super) async fn start_credits(api: &Api, force: bool) -> Result<CreditsBackfill, CoreError> {
    api.credits_backfill(force).await?;
    fetch_credits(api).await
}

/// `GET /api/similar/rebuild` — the neighbours index's state.
pub(super) async fn fetch_similar(api: &Api) -> Result<SimilarStatus, CoreError> {
    Ok(api.similar_status().await?)
}

/// `POST /api/similar/rebuild[?force=1]`.
pub(super) async fn start_similar(api: &Api, force: bool) -> Result<SimilarStatus, CoreError> {
    api.similar_rebuild(force).await?;
    fetch_similar(api).await
}

// --- availability (the audio auto-match) ----------------------------------------

/// `GET /api/automatch` — the switch, the running job and what the last run did.
pub(super) async fn fetch_automatch(api: &Api) -> Result<Automatch, CoreError> {
    Ok(api.automatch().await?)
}

/// `PUT /api/automatch/settings` — the server answers the whole payload back,
/// which is what the screen then draws (`AutomatchPanel#toggle`).
pub(super) async fn save_automatch(api: &Api, enabled: bool) -> Result<Automatch, CoreError> {
    let body = serde_json::json!({ "enabled": enabled });
    let answer: Option<Automatch> = api
        .send::<Value, Automatch>(Lane::User, Method::PUT, "/automatch/settings", Some(&body))
        .await?;
    answer.ok_or_else(|| CoreError::Internal {
        detail: "/api/automatch/settings: une réponse était attendue".to_owned(),
    })
}

/// `POST /api/automatch/run` — a library-wide pass. A `409` means one is
/// already running, and its `job` is in the error body; the caller re-reads.
pub(super) async fn run_automatch(api: &Api) -> Result<Automatch, CoreError> {
    api.automatch_run().await?;
    fetch_automatch(api).await
}

/// `POST /api/automatch/cancel` — asks the running pass to stop; the answer is
/// the new state, as the SPA draws it.
pub(super) async fn cancel_automatch(api: &Api) -> Result<Automatch, CoreError> {
    let answer: Option<Automatch> = api
        .send::<(), Automatch>(Lane::User, Method::POST, "/automatch/cancel", None)
        .await?;
    answer.ok_or_else(|| CoreError::Internal {
        detail: "/api/automatch/cancel: une réponse était attendue".to_owned(),
    })
}

// --- transcription ---------------------------------------------------------------

/// `GET /api/transcribe/settings[?probe=1]` — the engine's options, its last
/// probe and the queue. `probe` is the *Tester* button: it runs the engine once.
pub(super) async fn fetch_transcribe(api: &Api, probe: bool) -> Result<TranscribeSettings, CoreError> {
    Ok(api.transcribe_settings(probe).await?)
}

/// `PUT /api/transcribe/settings` — ONE field, then the server's whole state.
pub(super) async fn save_transcribe(
    api: &Api,
    fields: &Value,
) -> Result<TranscribeSettings, CoreError> {
    api.save_transcribe_settings(fields).await?;
    fetch_transcribe(api, false).await
}

/// `GET /api/transcribe/status` — what the server's worker is doing now.
pub(super) async fn fetch_transcribe_status(api: &Api) -> Result<TranscribeStatus, CoreError> {
    Ok(api.transcribe_status().await?)
}

// --- the server's own settings and the provider's verdict -------------------------

/// `GET /api/settings` — the upload quota and the Deezer credential's state.
pub(super) async fn fetch_server_settings(api: &Api) -> Result<ServerSettings, CoreError> {
    Ok(api.get_settings().await?)
}

/// `POST /api/settings` — one field, then the server's whole state.
pub(super) async fn save_server_settings(
    api: &Api,
    fields: &Value,
) -> Result<ServerSettings, CoreError> {
    api.set_settings(fields).await?;
    fetch_server_settings(api).await
}

/// `GET /api/deezer/status[?force=1]` — is the credential still good?
pub(super) async fn fetch_deezer_status(api: &Api, force: bool) -> Result<DeezerStatus, CoreError> {
    Ok(api.deezer_status(force).await?)
}

// --- the device's copy ------------------------------------------------------------

/// Every card the administrator's pages show, as last read. A field is `None`
/// until its own read succeeded once (a failure never blanks what the person
/// already saw).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct Snapshot {
    #[serde(default)]
    pub users: Option<AdminUsers>,
    #[serde(default)]
    pub invites: Option<AdminInvites>,
    #[serde(default)]
    pub storage: Option<Storage>,
    #[serde(default)]
    pub server: Option<ServerSettings>,
    #[serde(default)]
    pub deezer: Option<DeezerStatus>,
    #[serde(default)]
    pub rules: Option<ArchiveRules>,
    #[serde(default)]
    pub cleanup: Option<CleanupPreview>,
    #[serde(default)]
    pub archive: Option<ArchiveStatus>,
    #[serde(default)]
    pub migrate: Option<MigrateStatus>,
    #[serde(default)]
    pub plan: Option<AnalysisPlan>,
    #[serde(default)]
    pub analysis: Option<super::AnalysisJob>,
    #[serde(default)]
    pub enrich: Option<EnrichStatus>,
    #[serde(default)]
    pub credits: Option<CreditsBackfill>,
    #[serde(default)]
    pub similar: Option<SimilarStatus>,
    #[serde(default)]
    pub automatch: Option<Automatch>,
    #[serde(default)]
    pub transcribe: Option<TranscribeSettings>,
}

impl Snapshot {
    /// The device's copy, or an empty one when the person never opened the
    /// administrator's pages.
    pub(super) fn read(core: &Core) -> Snapshot {
        std::fs::read_to_string(snapshot_path(core))
            .ok()
            .and_then(|body| serde_json::from_str(&body).ok())
            .unwrap_or_default()
    }

    /// Writes one card and keeps the others: a card's read is the only thing
    /// that may replace it.
    pub(super) fn write(core: &Core, change: impl FnOnce(&mut Snapshot)) {
        let mut snapshot = Snapshot::read(core);
        change(&mut snapshot);
        let path = snapshot_path(core);
        let Ok(body) = serde_json::to_vec(&snapshot) else {
            return;
        };
        let temporary = path.with_extension("json.part");
        if std::fs::write(&temporary, &body).is_ok() {
            let _ = std::fs::rename(&temporary, &path);
        }
    }

    /// Puts ONE card back the way it was — the rollback an optimistic write
    /// needs when the server refuses it. `None` REMOVES the card, which is what a
    /// failed first read of a section that never succeeded must do.
    pub(super) fn restore(core: &Core, change: impl FnOnce(&mut Snapshot)) {
        Snapshot::write(core, change);
    }
}

/// Where the account's copy lives: the CACHE directory (evictable), never the
/// app's own data.
fn snapshot_path(core: &Core) -> PathBuf {
    core.cache_path(&format!(
        "admin-{:016x}.json",
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

/// The copy's path, for a test that reads the file back.
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
    /// The tagged client of one administrator read. The offline gate is the
    /// core's, so a dead link answers `Offline` in zero sockets.
    pub(super) fn admin_api(&self) -> Result<Arc<Api>, CoreError> {
        Ok(Arc::new(self.api()?.with_tag(TAG_ADMIN)))
    }

    /// Books one read's traffic in the core's meter, so `NzCore::stats()` answers
    /// what these pages cost without the UI counting anything.
    pub(super) fn note_admin_traffic(&self, api: &Api, before: Totals) {
        let after = totals(api);
        let requests = after.requests.saturating_sub(before.requests);
        if requests == 0 {
            return;
        }
        // `Meter::record` counts ONE request and its bytes; the run's bytes go
        // with the first, the rest are zeros.
        self.meter().record(
            TAG_ADMIN,
            after.bytes_in.saturating_sub(before.bytes_in),
            after.bytes_out.saturating_sub(before.bytes_out),
        );
        for _ in 1..requests {
            self.meter().record(TAG_ADMIN, 0, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The file name is a hash of the account, never the account id, and two
    /// accounts never share a copy.
    #[test]
    fn the_copy_is_named_after_a_hash_of_the_account() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_ne!(fnv1a(b"1"), fnv1a(b"2"));
        let path = format!("admin-{:016x}.json", fnv1a(b"42"));
        assert!(!path.contains("42"), "{path}");
    }

    /// A card written is a card read back, and writing one leaves the others in
    /// place: *Serveur* closing and reopening never blanks *Utilisateurs*.
    #[test]
    fn one_card_never_blanks_another() {
        let dir = crate::admin::testing::TempDir::new("admin-snapshot");
        let core = dir.core();
        Snapshot::write(&core, |snapshot| {
            snapshot.storage = Some(Storage {
                archive_dir: "/srv/archive".into(),
                archive_bytes: 42,
                ..Storage::default()
            });
        });
        Snapshot::write(&core, |snapshot| {
            snapshot.credits = Some(CreditsBackfill {
                tracks: 7,
                ..CreditsBackfill::default()
            });
        });
        let snapshot = Snapshot::read(&core);
        assert_eq!(snapshot.storage.expect("storage").archive_bytes, 42);
        assert_eq!(snapshot.credits.expect("credits").tracks, 7);
        assert!(snapshot.transcribe.is_none());
    }
}
