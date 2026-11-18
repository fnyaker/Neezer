//! Réglages, the administrator's sections (users, invites, server jobs,
//! backfills, transcription). Owned by **MB17**.
//!
//! The note names what this block reaches (docs/agents/mobile.md, "Screen lots,
//! wave 3", **MB17**): the user dashboard and its invitations, the server's
//! archive (status, rules, cleanup preview and run, storage, quotas, migration),
//! the analysis plan and its backfill, the credits/enrich/similar backfills, the
//! auto-match run and switch, the transcription settings and its workers, every
//! job's failures WITH their causes, the cache flush and the provider's status.
//!
//! # The shape of this boundary
//!
//! - **Each card is a `cached_x()` / `x()` pair**: the device's copy paints at
//!   once, the network replaces it, and a failure keeps the copy
//!   ([`crate::core::Screens`]' rule, `screenFlow` in Kotlin).
//! - **No JSON crosses.** A card says what it wants (`admin_save_rule("on_fav_album",
//!   "true")`, `admin_save_quota(25.0)`), never a body it built itself: the keys
//!   are the server's own, and a typo is refused in `nz-core` before it travels.
//! - **A job's failure carries its CAUSE**, not a count: [`AdminJobFailures`] is
//!   the port of `JobFailures.svelte`, and every job that has one carries it.
//! - **Nothing here decides a rule.** Every body converts to `nz_core::admin` and
//!   back; the French words live in Kotlin (`AdminCopy.kt`), the numbers here.
//!
//! UniFFI merges this `impl Screens` block into the ONE Kotlin `Screens` class,
//! so no shared file is touched (MB0's design, "The FFI boundary").

use nz_core::admin as core_admin;

use crate::core::Screens;
use crate::errors::NzError;

// --- records -------------------------------------------------------------------

/// Who is signed in, as the admin pages check before asking anything. A member
/// gets no entry AND no request: the server would refuse all twelve.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct AdminIdentity {
    pub id: String,
    pub name: String,
    /// `/api/me`'s `user.admin` — the ONE gate, no request.
    pub admin: bool,
}

/// One account, as a row of *Utilisateurs* draws it.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct AdminUserRow {
    pub id: String,
    pub name: String,
    pub admin: bool,
    pub disabled: bool,
    /// RFC 3339, absent for an account that never signed in.
    pub last_login: Option<String>,
    pub mail: Option<String>,
    pub onboarding: Option<String>,
}

impl From<core_admin::UserView> for AdminUserRow {
    fn from(user: core_admin::UserView) -> Self {
        AdminUserRow {
            id: user.id,
            name: user.name,
            admin: user.admin,
            disabled: user.disabled,
            last_login: user.last_login,
            mail: user.mail,
            onboarding: user.onboarding,
        }
    }
}

/// One page of accounts, with the cursor the next page continues from.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct AdminUsersPage {
    pub users: Vec<AdminUserRow>,
    /// How many accounts the server has for this query, in total.
    pub total: u32,
    /// The last row's id, `None` on the last page.
    pub next: Option<String>,
}

impl From<core_admin::UsersPageView> for AdminUsersPage {
    fn from(page: core_admin::UsersPageView) -> Self {
        AdminUsersPage {
            users: page.users.into_iter().map(AdminUserRow::from).collect(),
            total: page.total,
            next: page.next,
        }
    }
}

/// `GET /api/admin/users/<id>`: the account and what a deletion would move.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct AdminUserDetail {
    pub user: AdminUserRow,
    /// Uploaded files, which a deletion REASSIGNS to the admin.
    pub uploads: u32,
    pub playlists: u32,
}

impl From<core_admin::UserDetailView> for AdminUserDetail {
    fn from(detail: core_admin::UserDetailView) -> Self {
        AdminUserDetail {
            user: AdminUserRow::from(detail.user),
            uploads: detail.uploads,
            playlists: detail.playlists,
        }
    }
}

/// What a deletion moved, for the confirmation's toast.
#[derive(uniffi::Record, Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdminDeleteResult {
    pub uploads_reassigned: u32,
    pub podcasts_reassigned: u32,
}

impl From<core_admin::DeleteResultView> for AdminDeleteResult {
    fn from(result: core_admin::DeleteResultView) -> Self {
        AdminDeleteResult {
            uploads_reassigned: result.uploads_reassigned,
            podcasts_reassigned: result.podcasts_reassigned,
        }
    }
}

/// One invitation. Its LINK is not here: the token exists once, in
/// [`AdminInviteIssued`].
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct AdminInviteRow {
    pub id: String,
    /// The admin's own reminder ("Léa") — never shown to the invited person.
    pub note: Option<String>,
    pub created: Option<String>,
    pub expires: Option<String>,
    /// `pending`, `used`, `revoked` or `expired`.
    pub state: String,
    pub created_by: Option<String>,
    pub used_by: Option<String>,
}

impl From<core_admin::InviteView> for AdminInviteRow {
    fn from(invite: core_admin::InviteView) -> Self {
        AdminInviteRow {
            id: invite.id,
            note: invite.note,
            created: invite.created,
            expires: invite.expires,
            state: invite.state,
            created_by: invite.created_by,
            used_by: invite.used_by,
        }
    }
}

/// A freshly minted invitation: the row and the link to hand over.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct AdminInviteIssued {
    pub invite: AdminInviteRow,
    /// `{origin}/app/#/invite/{token}` — the SPA's own link shape.
    pub url: String,
    /// The bare token, for the app's own `neezer://invite/<code>` link.
    pub token: String,
}

impl From<core_admin::InviteIssuedView> for AdminInviteIssued {
    fn from(issued: core_admin::InviteIssuedView) -> Self {
        AdminInviteIssued {
            invite: AdminInviteRow::from(issued.invite),
            url: issued.url,
            token: issued.token,
        }
    }
}

/// `/api/storage`, as the *Stockage serveur* card draws it.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct AdminStorage {
    pub archive_dir: String,
    pub archive_bytes: u64,
    pub cache_bytes: u64,
    pub cache_limit: u64,
    pub transcode_bytes: u64,
    pub transcode_limit: u64,
    pub disk_free: u64,
    pub disk_total: u64,
    pub tracks_archived: u32,
    pub tracks_total: u32,
}

impl From<core_admin::StorageView> for AdminStorage {
    fn from(storage: core_admin::StorageView) -> Self {
        AdminStorage {
            archive_dir: storage.archive_dir,
            archive_bytes: storage.archive_bytes,
            cache_bytes: storage.cache_bytes,
            cache_limit: storage.cache_limit,
            transcode_bytes: storage.transcode_bytes,
            transcode_limit: storage.transcode_limit,
            disk_free: storage.disk_free,
            disk_total: storage.disk_total,
            tracks_archived: storage.tracks_archived,
            tracks_total: storage.tracks_total,
        }
    }
}

/// `/api/settings` + `/api/deezer/status`: the quota and the credential's state.
/// The ARL itself never appears, in either direction.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct AdminServerCard {
    /// The upload cap for non-admins, in GB, 0 for no limit.
    pub upload_quota_gb: f64,
    pub deezer_enabled: bool,
    pub arl_set: bool,
    /// `config`, `env` or `database` — where the credential comes from.
    pub arl_source: Option<String>,
    /// The masked hint (`…a1b2`), never the credential.
    pub arl_hint: Option<String>,
    /// The provider's verdict, `None` until `/deezer/status` answered.
    pub status_ok: Option<bool>,
    pub status_reason: Option<String>,
    pub status_message: Option<String>,
}

impl From<core_admin::ServerCardView> for AdminServerCard {
    fn from(card: core_admin::ServerCardView) -> Self {
        AdminServerCard {
            upload_quota_gb: card.upload_quota_gb,
            deezer_enabled: card.deezer_enabled,
            arl_set: card.arl_set,
            arl_source: card.arl_source,
            arl_hint: card.arl_hint,
            status_ok: card.status_ok,
            status_reason: card.status_reason,
            status_message: card.status_message,
        }
    }
}

/// One archiving event, with its switch.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct AdminRuleSwitch {
    /// The server's own id (`on_fav_album`, `on_play_context`, …).
    pub id: String,
    pub on: bool,
}

/// The archiving and cleanup rule set, one field per control.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct AdminRuleSet {
    pub artist_scope: String,
    pub artist_limit: u32,
    pub chapters: bool,
    pub chapter_urls: bool,
    pub clean_on: bool,
    pub clean_min_free_gb: f64,
    pub clean_stale_days: u32,
    pub clean_order: String,
    pub clean_keep_fav: bool,
    pub clean_keep_playlist: bool,
    pub clean_keep_podcast: bool,
}

impl From<core_admin::RuleSetView> for AdminRuleSet {
    fn from(rules: core_admin::RuleSetView) -> Self {
        AdminRuleSet {
            artist_scope: rules.artist_scope,
            artist_limit: rules.artist_limit,
            chapters: rules.chapters,
            chapter_urls: rules.chapter_urls,
            clean_on: rules.clean_on,
            clean_min_free_gb: rules.clean_min_free_gb,
            clean_stale_days: rules.clean_stale_days,
            clean_order: rules.clean_order,
            clean_keep_fav: rules.clean_keep_fav,
            clean_keep_playlist: rules.clean_keep_playlist,
            clean_keep_podcast: rules.clean_keep_podcast,
        }
    }
}

/// `/api/archive/rules`, as *Règles d'archivage* draws it.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct AdminRules {
    pub events: Vec<AdminRuleSwitch>,
    pub rules: AdminRuleSet,
    pub artist_scopes: Vec<String>,
    pub cleanup_orders: Vec<String>,
    /// The master switch lives in the server's configuration file: when it is
    /// off, the card says these rules do nothing.
    pub archive_library: bool,
}

impl From<core_admin::RulesView> for AdminRules {
    fn from(rules: core_admin::RulesView) -> Self {
        AdminRules {
            events: rules
                .events
                .into_iter()
                .map(|switch| AdminRuleSwitch {
                    id: switch.id,
                    on: switch.on,
                })
                .collect(),
            rules: AdminRuleSet::from(rules.rules),
            artist_scopes: rules.artist_scopes,
            cleanup_orders: rules.cleanup_orders,
            archive_library: rules.archive_library,
        }
    }
}

/// One title a cleanup would delete, and its size.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct AdminCleanupTrack {
    pub title: String,
    pub artist: String,
    pub bytes: u64,
}

/// `/api/archive/cleanup/preview` — what a cleanup would delete RIGHT NOW.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct AdminCleanup {
    pub enabled: bool,
    pub running: bool,
    pub eligible: u32,
    pub would_free: u64,
    pub free_bytes: u64,
    /// How much room is missing to get back above the threshold; 0 means the
    /// disk is fine and nothing would be deleted.
    pub needed_bytes: u64,
    pub tracks: Vec<AdminCleanupTrack>,
}

impl From<core_admin::CleanupView> for AdminCleanup {
    fn from(preview: core_admin::CleanupView) -> Self {
        AdminCleanup {
            enabled: preview.enabled,
            running: preview.running,
            eligible: preview.eligible,
            would_free: preview.would_free,
            free_bytes: preview.free_bytes,
            needed_bytes: preview.needed_bytes,
            tracks: preview
                .tracks
                .into_iter()
                .map(|track| AdminCleanupTrack {
                    title: track.title,
                    artist: track.artist,
                    bytes: track.bytes,
                })
                .collect(),
        }
    }
}

/// What `POST /api/archive/cleanup` did.
#[derive(uniffi::Record, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AdminCleanupRun {
    pub deleted: u32,
    pub freed: u64,
}

/// `/api/archive/status` — the backfill's gauge.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct AdminArchiveStatus {
    pub running: bool,
    pub scope: Option<String>,
    pub total: u32,
    pub done: u32,
    pub archived: u32,
    pub failed: u32,
    pub unavailable: u32,
    pub error: Option<String>,
}

impl From<core_admin::ArchiveStatusView> for AdminArchiveStatus {
    fn from(status: core_admin::ArchiveStatusView) -> Self {
        AdminArchiveStatus {
            running: status.running,
            scope: status.scope,
            total: status.total,
            done: status.done,
            archived: status.archived,
            failed: status.failed,
            unavailable: status.unavailable,
            error: status.error,
        }
    }
}

/// One named counter (`{"tags": 40}`).
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct AdminCount {
    pub name: String,
    pub count: u32,
}

/// One failure reason and how many tracks it took.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct AdminCause {
    pub reason: String,
    pub count: u32,
}

/// One failed track with the cause it gave.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct AdminFailureRow {
    pub track: String,
    pub path: String,
    pub reason: String,
}

/// `JobFailures.svelte`, ported: how many failed, the ranked tally of causes,
/// the sample of rows, and how many the server's bounded sample left out.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct AdminJobFailures {
    pub failed: u32,
    pub causes: Vec<AdminCause>,
    pub rows: Vec<AdminFailureRow>,
    /// `failed` minus the rows we hold: never let the list imply it is whole.
    pub hidden: u32,
}

impl From<core_admin::JobFailuresView> for AdminJobFailures {
    fn from(job: core_admin::JobFailuresView) -> Self {
        AdminJobFailures {
            failed: job.failed,
            causes: job
                .causes
                .into_iter()
                .map(|cause| AdminCause {
                    reason: cause.reason,
                    count: cause.count,
                })
                .collect(),
            rows: job
                .rows
                .into_iter()
                .map(|row| AdminFailureRow {
                    track: row.track,
                    path: row.path,
                    reason: row.reason,
                })
                .collect(),
            hidden: job.hidden,
        }
    }
}

/// `/api/archive/migrate` — the format migration's gauge and its causes.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct AdminMigrate {
    pub running: bool,
    pub done: u32,
    pub failed: u32,
    pub error: Option<String>,
    pub counts: Vec<AdminCount>,
    pub causes: Vec<AdminCause>,
    pub hidden: u32,
}

impl From<core_admin::MigrateView> for AdminMigrate {
    fn from(migrate: core_admin::MigrateView) -> Self {
        AdminMigrate {
            running: migrate.running,
            done: migrate.done,
            failed: migrate.failed,
            error: migrate.error,
            counts: migrate
                .counts
                .into_iter()
                .map(|count| AdminCount {
                    name: count.name,
                    count: count.count,
                })
                .collect(),
            causes: migrate
                .causes
                .into_iter()
                .map(|cause| AdminCause {
                    reason: cause.reason,
                    count: cause.count,
                })
                .collect(),
            hidden: migrate.hidden,
        }
    }
}

/// One step of the analysis pipeline.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct AdminStage {
    /// `measure`, `embed` or `decide`.
    pub id: String,
    /// Tracks this step still has to look at.
    pub late: u32,
    /// The step cannot run (no extractor installed).
    pub off: bool,
}

/// The nightly measurement window.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct AdminSchedule {
    pub enabled: bool,
    /// `HH:MM`, in the SERVER's local time.
    pub at: String,
    pub hours: u32,
}

/// `/api/analysis/plan` — what is out of date, at which step, where it runs.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct AdminPlan {
    pub place: String,
    pub total: u32,
    pub todo: u32,
    pub never: u32,
    pub current: u32,
    pub stages: Vec<AdminStage>,
    pub extractor: bool,
    pub schedule: AdminSchedule,
}

impl From<core_admin::PlanView> for AdminPlan {
    fn from(plan: core_admin::PlanView) -> Self {
        AdminPlan {
            place: plan.place,
            total: plan.total,
            todo: plan.todo,
            never: plan.never,
            current: plan.current,
            stages: plan
                .stages
                .into_iter()
                .map(|stage| AdminStage {
                    id: stage.id,
                    late: stage.late,
                    off: stage.off,
                })
                .collect(),
            extractor: plan.extractor,
            schedule: AdminSchedule {
                enabled: plan.schedule.enabled,
                at: plan.schedule.at,
                hours: plan.schedule.hours,
            },
        }
    }
}

/// `/api/analysis/backfill` — the measure-all job.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct AdminAnalysisJob {
    pub running: bool,
    pub scanned: u32,
    pub done: u32,
    pub total: u32,
    /// `measure`, `embed` or `decide`.
    pub stage: Option<String>,
    /// Why the job is waiting: the server's own word (`listening`, `download`,
    /// `load`), never a number.
    pub paused: Option<String>,
    pub error: Option<String>,
    pub workers: u32,
    pub workers_setting: u32,
    pub workers_auto: u32,
    pub force: bool,
    pub failures: AdminJobFailures,
}

impl From<core_admin::AnalysisJobView> for AdminAnalysisJob {
    fn from(job: core_admin::AnalysisJobView) -> Self {
        AdminAnalysisJob {
            running: job.running,
            scanned: job.scanned,
            done: job.done,
            total: job.total,
            stage: job.stage,
            paused: job.paused,
            error: job.error,
            workers: job.workers,
            workers_setting: job.workers_setting,
            workers_auto: job.workers_auto,
            force: job.force,
            failures: AdminJobFailures::from(job.failures),
        }
    }
}

/// `/api/enrich/backfill` — MusicBrainz's coverage of the archive.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct AdminEnrich {
    pub enabled: bool,
    pub running: bool,
    pub acoustid: bool,
    pub found: Vec<AdminCount>,
    pub tried: u32,
    pub untried: u32,
    pub error: Option<String>,
    pub failures: AdminJobFailures,
}

impl From<core_admin::EnrichView> for AdminEnrich {
    fn from(status: core_admin::EnrichView) -> Self {
        AdminEnrich {
            enabled: status.enabled,
            running: status.running,
            acoustid: status.acoustid,
            found: status
                .found
                .into_iter()
                .map(|count| AdminCount {
                    name: count.name,
                    count: count.count,
                })
                .collect(),
            tried: status.tried,
            untried: status.untried,
            error: status.error,
            failures: AdminJobFailures::from(status.failures),
        }
    }
}

/// `/api/credits/backfill` — the credits worker's counters.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct AdminCredits {
    pub running: bool,
    pub done: u32,
    pub failed: u32,
    pub total: u32,
    pub tracks: u32,
    pub completed: bool,
    pub error: Option<String>,
    pub sources: Vec<AdminCount>,
    pub failures: AdminJobFailures,
}

impl From<core_admin::CreditsView> for AdminCredits {
    fn from(job: core_admin::CreditsView) -> Self {
        AdminCredits {
            running: job.running,
            done: job.done,
            failed: job.failed,
            total: job.total,
            tracks: job.tracks,
            completed: job.completed,
            error: job.error,
            sources: job
                .sources
                .into_iter()
                .map(|count| AdminCount {
                    name: count.name,
                    count: count.count,
                })
                .collect(),
            failures: AdminJobFailures::from(job.failures),
        }
    }
}

/// `/api/similar/rebuild` — the "sounds like" index.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct AdminSimilar {
    pub running: bool,
    pub tracks: u32,
    /// Unix seconds of the last rebuild, absent when it never ran.
    pub at: Option<i64>,
}

impl From<core_admin::SimilarView> for AdminSimilar {
    fn from(status: core_admin::SimilarView) -> Self {
        AdminSimilar {
            running: status.running,
            tracks: status.tracks,
            at: status.at,
        }
    }
}

/// One title the auto-match replaced, with the score that justified it.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct AdminReplaced {
    pub at: String,
    pub from_title: String,
    pub from_artist: String,
    pub to_title: String,
    pub to_source: String,
    /// 0..1 — 99 % or nothing (`audiomatch.THRESHOLD`).
    pub score: f64,
}

impl From<core_admin::ReplacedView> for AdminReplaced {
    fn from(row: core_admin::ReplacedView) -> Self {
        AdminReplaced {
            at: row.at,
            from_title: row.from_title,
            from_artist: row.from_artist,
            to_title: row.to_title,
            to_source: row.to_source,
            score: row.score,
        }
    }
}

/// One title the auto-match left for a person to handle.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct AdminAutomatchMiss {
    pub title: String,
    pub artist: String,
    /// The server's own cause, in its own words (the card translates the known
    /// ones).
    pub reason: String,
    /// `error`, `no_match`, `no_reference`, …
    pub status: String,
}

/// `/api/automatch` — the switch, the pass in flight, and the last one's result.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct AdminAutomatch {
    pub available: bool,
    pub enabled: bool,
    pub switched_off: bool,
    pub youtube: bool,
    pub threshold: f64,
    pub running: bool,
    pub total: Option<u32>,
    pub done: u32,
    pub matched: u32,
    pub cancel: bool,
    pub last_counts: Vec<AdminCount>,
    pub last_stopped: Option<String>,
    pub misses: Vec<AdminAutomatchMiss>,
    pub recent: Vec<AdminReplaced>,
}

impl From<core_admin::AutomatchView> for AdminAutomatch {
    fn from(status: core_admin::AutomatchView) -> Self {
        AdminAutomatch {
            available: status.available,
            enabled: status.enabled,
            switched_off: status.switched_off,
            youtube: status.youtube,
            threshold: status.threshold,
            running: status.running,
            total: status.total,
            done: status.done,
            matched: status.matched,
            cancel: status.cancel,
            last_counts: status
                .last_counts
                .into_iter()
                .map(|count| AdminCount {
                    name: count.name,
                    count: count.count,
                })
                .collect(),
            last_stopped: status.last_stopped,
            misses: status
                .misses
                .into_iter()
                .map(|miss| AdminAutomatchMiss {
                    title: miss.title,
                    artist: miss.artist,
                    reason: miss.reason,
                    status: miss.status,
                })
                .collect(),
            recent: status.recent.into_iter().map(AdminReplaced::from).collect(),
        }
    }
}

/// `/api/transcribe/settings` — the engine, the model, the queue, the numbers.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct AdminTranscribe {
    pub available: bool,
    pub enabled: bool,
    pub runner: Option<String>,
    pub reason: Option<String>,
    pub language: String,
    pub languages: Vec<String>,
    pub max_minutes: u32,
    pub daily_minutes: u32,
    pub search_segments: u64,
    pub threads: u32,
    pub engine_name: Option<String>,
    pub engine_version: Option<String>,
    pub engine_ok: bool,
    pub model_name: Option<String>,
    pub model_size_mb: u32,
    pub model_ok: bool,
    /// `running + leased`: what the queue line calls "en cours".
    pub running_now: u32,
    pub queued: u32,
    pub ready: u32,
    pub errored: u32,
    pub budget_limit_ms: u64,
    pub budget_used_ms: u64,
}

impl From<core_admin::TranscribeView> for AdminTranscribe {
    fn from(settings: core_admin::TranscribeView) -> Self {
        AdminTranscribe {
            available: settings.available,
            enabled: settings.enabled,
            runner: settings.runner,
            reason: settings.reason,
            language: settings.language,
            languages: settings.languages,
            max_minutes: settings.max_minutes,
            daily_minutes: settings.daily_minutes,
            search_segments: settings.search_segments,
            threads: settings.threads,
            engine_name: settings.engine_name,
            engine_version: settings.engine_version,
            engine_ok: settings.engine_ok,
            model_name: settings.model_name,
            model_size_mb: settings.model_size_mb,
            model_ok: settings.model_ok,
            running_now: settings.running_now,
            queued: settings.queued,
            ready: settings.ready,
            errored: settings.errored,
            budget_limit_ms: settings.budget_limit_ms,
            budget_used_ms: settings.budget_used_ms,
        }
    }
}

// --- the screens' methods ---------------------------------------------------------

#[uniffi::export(async_runtime = "tokio")]
impl Screens {
    // --- who is asking -----------------------------------------------------

    /// Who is signed in, from the session the core restored: **no request**, and
    /// `None` before a login. A member sees no admin entry at all (the server
    /// would refuse every route below).
    pub fn admin_identity(&self) -> Option<AdminIdentity> {
        self.core().me().map(|me| AdminIdentity {
            id: me.user.id.to_string(),
            name: me.user.name,
            admin: me.user.admin,
        })
    }

    // --- people ------------------------------------------------------------

    /// The account table as this device last read it — disk only, 0 requests.
    pub fn cached_admin_users(&self) -> Option<AdminUsersPage> {
        self.core().cached_admin_users().map(AdminUsersPage::from)
    }

    /// `GET /api/admin/users?q=&after=&limit=` — ONE page (25 rows by default).
    pub async fn admin_users(
        &self,
        q: String,
        after: Option<String>,
        limit: u32,
    ) -> Result<AdminUsersPage, NzError> {
        self.core()
            .admin_users(&q, after.as_deref(), limit)
            .await
            .map(AdminUsersPage::from)
            .map_err(NzError::from)
    }

    /// `GET /api/admin/users/<id>` — one account and what a deletion would move.
    pub async fn admin_user(&self, id: String) -> Result<AdminUserDetail, NzError> {
        self.core()
            .admin_user(&id)
            .await
            .map(AdminUserDetail::from)
            .map_err(NzError::from)
    }

    /// `PATCH /api/admin/users/<id>` with `{admin}` — *Nommer administrateur* /
    /// *Retirer les droits*. Optimistic: the copy takes the tap first, a refusal
    /// puts it back and returns the cause.
    pub async fn admin_set_role(&self, id: String, on: bool) -> Result<AdminUserRow, NzError> {
        self.core()
            .admin_set_role(&id, on)
            .await
            .map(AdminUserRow::from)
            .map_err(NzError::from)
    }

    /// `PATCH /api/admin/users/<id>` with `{disabled}` — *Désactiver* /
    /// *Réactiver*. The person is disconnected everywhere; nothing is deleted.
    pub async fn admin_set_disabled(&self, id: String, on: bool) -> Result<AdminUserRow, NzError> {
        self.core()
            .admin_set_disabled(&id, on)
            .await
            .map(AdminUserRow::from)
            .map_err(NzError::from)
    }

    /// `DELETE /api/admin/users/<id>` — confirms the account's NAME, and reports
    /// what was reassigned. The archived audio stays (CLAUDE.md).
    pub async fn admin_delete_user(
        &self,
        id: String,
        name: String,
    ) -> Result<AdminDeleteResult, NzError> {
        self.core()
            .admin_delete_user(&id, &name)
            .await
            .map(AdminDeleteResult::from)
            .map_err(NzError::from)
    }

    // --- invitations -------------------------------------------------------

    /// The pending invitations as this device last read them — 0 requests.
    pub fn cached_admin_invites(&self) -> Option<Vec<AdminInviteRow>> {
        self.core()
            .cached_admin_invites()
            .map(|invites| invites.into_iter().map(AdminInviteRow::from).collect())
    }

    /// `GET /api/admin/invites?state=pending&limit=200`.
    pub async fn admin_invites(
        &self,
        state: Option<String>,
    ) -> Result<Vec<AdminInviteRow>, NzError> {
        self.core()
            .admin_invites(state.as_deref())
            .await
            .map(|invites| invites.into_iter().map(AdminInviteRow::from).collect())
            .map_err(NzError::from)
    }

    /// `POST /api/admin/invites` — mint a link. `expires_in` is one of the four
    /// durations the sheet offers (`AdminCopy.INVITE_DURATIONS`).
    pub async fn admin_create_invite(
        &self,
        note: Option<String>,
        expires_in: u32,
    ) -> Result<AdminInviteIssued, NzError> {
        self.core()
            .admin_create_invite(note.as_deref(), expires_in)
            .await
            .map(AdminInviteIssued::from)
            .map_err(NzError::from)
    }

    /// `POST /api/admin/invites/<id>/renew` — the old link is revoked, a new one
    /// minted with the same duration.
    pub async fn admin_renew_invite(&self, id: String) -> Result<AdminInviteIssued, NzError> {
        self.core()
            .admin_renew_invite(&id)
            .await
            .map(AdminInviteIssued::from)
            .map_err(NzError::from)
    }

    /// `POST /api/admin/invites/<id>/revoke` — the link stops working now.
    pub async fn admin_revoke_invite(&self, id: String) -> Result<(), NzError> {
        self.core()
            .admin_revoke_invite(&id)
            .await
            .map_err(NzError::from)
    }

    // --- the archive -------------------------------------------------------

    pub fn cached_admin_storage(&self) -> Option<AdminStorage> {
        self.core().cached_admin_storage().map(AdminStorage::from)
    }

    /// `GET /api/storage` — the archive, the caches and the disk.
    pub async fn admin_storage(&self) -> Result<AdminStorage, NzError> {
        self.core()
            .admin_storage()
            .await
            .map(AdminStorage::from)
            .map_err(NzError::from)
    }

    pub fn cached_admin_server_card(&self) -> Option<AdminServerCard> {
        self.core()
            .cached_admin_server_card()
            .map(AdminServerCard::from)
    }

    /// `GET /api/settings` then `GET /api/deezer/status` — the quota and the
    /// provider's verdict (two requests, the SPA's own two).
    pub async fn admin_server_card(&self) -> Result<AdminServerCard, NzError> {
        self.core()
            .admin_server_card()
            .await
            .map(AdminServerCard::from)
            .map_err(NzError::from)
    }

    /// `GET /api/deezer/status[?force=1]` — *Tester* re-tests the credential.
    pub async fn admin_test_deezer(&self, force: bool) -> Result<AdminServerCard, NzError> {
        self.core()
            .admin_test_deezer(force)
            .await
            .map(AdminServerCard::from)
            .map_err(NzError::from)
    }

    /// `POST /api/settings` with `{upload_quota_gb}` — 0 means no limit.
    pub async fn admin_save_quota(&self, gb: f64) -> Result<AdminServerCard, NzError> {
        self.core()
            .admin_save_quota(gb)
            .await
            .map(AdminServerCard::from)
            .map_err(NzError::from)
    }

    /// `POST /api/cache/flush` — the DERIVED caches only (transcodes, proxied
    /// covers). The archive is not a cache. Answers the freed bytes.
    pub async fn admin_flush_cache(&self) -> Result<u64, NzError> {
        self.core().admin_flush_cache().await.map_err(NzError::from)
    }

    pub fn cached_admin_rules(&self) -> Option<AdminRules> {
        self.core().cached_admin_rules().map(AdminRules::from)
    }

    /// `GET /api/archive/rules` — which events archive, and the cleanup policy.
    pub async fn admin_rules(&self) -> Result<AdminRules, NzError> {
        self.core()
            .admin_rules()
            .await
            .map(AdminRules::from)
            .map_err(NzError::from)
    }

    /// `POST /api/archive/rules` with ONE field (`{"on_fav_album": true}`), whose
    /// value travels as JSON TEXT so the screen never builds a body.
    pub async fn admin_save_rule(
        &self,
        field: String,
        value: String,
    ) -> Result<AdminRules, NzError> {
        self.core()
            .admin_save_rule(&field, &value)
            .await
            .map(AdminRules::from)
            .map_err(NzError::from)
    }

    pub fn cached_admin_cleanup(&self) -> Option<AdminCleanup> {
        self.core().cached_admin_cleanup().map(AdminCleanup::from)
    }

    /// `GET /api/archive/cleanup/preview` — what a cleanup would delete, and how
    /// much room is missing. Deletes nothing.
    pub async fn admin_cleanup(&self) -> Result<AdminCleanup, NzError> {
        self.core()
            .admin_cleanup()
            .await
            .map(AdminCleanup::from)
            .map_err(NzError::from)
    }

    /// `POST /api/archive/cleanup` — the ONE function that deletes archived
    /// audio, and only what the admin's own policy wrote down.
    pub async fn admin_run_cleanup(&self) -> Result<AdminCleanupRun, NzError> {
        self.core()
            .admin_run_cleanup()
            .await
            .map(|run| AdminCleanupRun {
                deleted: run.deleted,
                freed: run.freed,
            })
            .map_err(NzError::from)
    }

    pub fn cached_admin_archive_status(&self) -> Option<AdminArchiveStatus> {
        self.core()
            .cached_admin_archive_status()
            .map(AdminArchiveStatus::from)
    }

    /// `GET /api/archive/status` — the backfill's gauge (and its poll).
    pub async fn admin_archive_status(&self) -> Result<AdminArchiveStatus, NzError> {
        self.core()
            .admin_archive_status()
            .await
            .map(AdminArchiveStatus::from)
            .map_err(NzError::from)
    }

    /// `POST /api/archive/backfill` — `all`, `favorites`, `playlists` or
    /// `podcasts` (`AdminCopy.ARCHIVE_SCOPES`). Only ever adds.
    pub async fn admin_start_archive(
        &self,
        scope: String,
    ) -> Result<AdminArchiveStatus, NzError> {
        self.core()
            .admin_start_archive(&scope)
            .await
            .map(AdminArchiveStatus::from)
            .map_err(NzError::from)
    }

    pub fn cached_admin_migrate(&self) -> Option<AdminMigrate> {
        self.core().cached_admin_migrate().map(AdminMigrate::from)
    }

    /// `GET /api/archive/migrate` — the file-format migration's gauge.
    pub async fn admin_migrate(&self) -> Result<AdminMigrate, NzError> {
        self.core()
            .admin_migrate()
            .await
            .map(AdminMigrate::from)
            .map_err(NzError::from)
    }

    /// `POST /api/archive/migrate` — rewrite the archived files. Safe to re-run.
    pub async fn admin_start_migrate(&self, force: bool) -> Result<AdminMigrate, NzError> {
        self.core()
            .admin_start_migrate(force)
            .await
            .map(AdminMigrate::from)
            .map_err(NzError::from)
    }

    // --- the analysis ------------------------------------------------------

    pub fn cached_admin_plan(&self) -> Option<AdminPlan> {
        self.core().cached_admin_plan().map(AdminPlan::from)
    }

    /// `GET /api/analysis/plan` — what is out of date, at which step.
    pub async fn admin_plan(&self) -> Result<AdminPlan, NzError> {
        self.core()
            .admin_plan()
            .await
            .map(AdminPlan::from)
            .map_err(NzError::from)
    }

    pub fn cached_admin_analysis(&self) -> Option<AdminAnalysisJob> {
        self.core()
            .cached_admin_analysis()
            .map(AdminAnalysisJob::from)
    }

    /// `GET /api/analysis/backfill` — the measure-all job and its causes.
    pub async fn admin_analysis(&self) -> Result<AdminAnalysisJob, NzError> {
        self.core()
            .admin_analysis()
            .await
            .map(AdminAnalysisJob::from)
            .map_err(NzError::from)
    }

    /// `POST /api/analysis/backfill` — measure now. `workers` 0 sizes the run
    /// from the machine. The server gates it behind its own throttle (rule 1).
    pub async fn admin_start_analysis(
        &self,
        force: bool,
        workers: u32,
    ) -> Result<AdminAnalysisJob, NzError> {
        self.core()
            .admin_start_analysis(force, workers)
            .await
            .map(AdminAnalysisJob::from)
            .map_err(NzError::from)
    }

    /// `POST /api/analysis/backfill/stop` — stop after the track in hand.
    pub async fn admin_stop_analysis(&self) -> Result<AdminAnalysisJob, NzError> {
        self.core()
            .admin_stop_analysis()
            .await
            .map(AdminAnalysisJob::from)
            .map_err(NzError::from)
    }

    /// `PUT /api/analysis/schedule` — the nightly window (`HH:MM`, server time).
    pub async fn admin_save_schedule(
        &self,
        enabled: bool,
        at: String,
        hours: u32,
    ) -> Result<AdminPlan, NzError> {
        self.core()
            .admin_save_schedule(enabled, &at, hours)
            .await
            .map(AdminPlan::from)
            .map_err(NzError::from)
    }

    /// `PUT /api/analysis/where` — who measures: `server` or `client`.
    pub async fn admin_save_analysis_place(&self, place: String) -> Result<AdminPlan, NzError> {
        self.core()
            .admin_save_analysis_place(&place)
            .await
            .map(AdminPlan::from)
            .map_err(NzError::from)
    }

    // --- the library-wide backfills ----------------------------------------

    pub fn cached_admin_enrich(&self) -> Option<AdminEnrich> {
        self.core().cached_admin_enrich().map(AdminEnrich::from)
    }

    /// `GET /api/enrich/backfill` — MusicBrainz's coverage.
    pub async fn admin_enrich(&self) -> Result<AdminEnrich, NzError> {
        self.core()
            .admin_enrich()
            .await
            .map(AdminEnrich::from)
            .map_err(NzError::from)
    }

    /// `POST /api/enrich/backfill[?force]`.
    pub async fn admin_start_enrich(&self, force: bool) -> Result<AdminEnrich, NzError> {
        self.core()
            .admin_start_enrich(force)
            .await
            .map(AdminEnrich::from)
            .map_err(NzError::from)
    }

    pub fn cached_admin_credits(&self) -> Option<AdminCredits> {
        self.core().cached_admin_credits().map(AdminCredits::from)
    }

    /// `GET /api/credits/backfill` — who wrote, composed and produced each track.
    pub async fn admin_credits(&self) -> Result<AdminCredits, NzError> {
        self.core()
            .admin_credits()
            .await
            .map(AdminCredits::from)
            .map_err(NzError::from)
    }

    /// `POST /api/credits/backfill[?force]`.
    pub async fn admin_start_credits(&self, force: bool) -> Result<AdminCredits, NzError> {
        self.core()
            .admin_start_credits(force)
            .await
            .map(AdminCredits::from)
            .map_err(NzError::from)
    }

    pub fn cached_admin_similar(&self) -> Option<AdminSimilar> {
        self.core().cached_admin_similar().map(AdminSimilar::from)
    }

    /// `GET /api/similar/rebuild` — the fifty-closest index.
    pub async fn admin_similar(&self) -> Result<AdminSimilar, NzError> {
        self.core()
            .admin_similar()
            .await
            .map(AdminSimilar::from)
            .map_err(NzError::from)
    }

    /// `POST /api/similar/rebuild[?force]`.
    pub async fn admin_start_similar(&self, force: bool) -> Result<AdminSimilar, NzError> {
        self.core()
            .admin_start_similar(force)
            .await
            .map(AdminSimilar::from)
            .map_err(NzError::from)
    }

    // --- availability (the audio auto-match) -------------------------------

    pub fn cached_admin_automatch(&self) -> Option<AdminAutomatch> {
        self.core()
            .cached_admin_automatch()
            .map(AdminAutomatch::from)
    }

    /// `GET /api/automatch` — the switch, the pass in flight, the last result.
    pub async fn admin_automatch(&self) -> Result<AdminAutomatch, NzError> {
        self.core()
            .admin_automatch()
            .await
            .map(AdminAutomatch::from)
            .map_err(NzError::from)
    }

    /// `PUT /api/automatch/settings` — the switch, answered with the whole state.
    pub async fn admin_set_automatch(&self, on: bool) -> Result<AdminAutomatch, NzError> {
        self.core()
            .admin_set_automatch(on)
            .await
            .map(AdminAutomatch::from)
            .map_err(NzError::from)
    }

    /// `POST /api/automatch/run` — a library-wide pass. A `409` means one is
    /// already running (`Conflict`, and the card says so).
    pub async fn admin_run_automatch(&self) -> Result<AdminAutomatch, NzError> {
        self.core()
            .admin_run_automatch()
            .await
            .map(AdminAutomatch::from)
            .map_err(NzError::from)
    }

    /// `POST /api/automatch/cancel` — stop the pass in flight.
    pub async fn admin_cancel_automatch(&self) -> Result<AdminAutomatch, NzError> {
        self.core()
            .admin_cancel_automatch()
            .await
            .map(AdminAutomatch::from)
            .map_err(NzError::from)
    }

    // --- transcription -----------------------------------------------------

    pub fn cached_admin_transcribe(&self) -> Option<AdminTranscribe> {
        self.core()
            .cached_admin_transcribe()
            .map(AdminTranscribe::from)
    }

    /// `GET /api/transcribe/settings[?probe=1]` — the engine, the model, the
    /// queue. `probe` is *Tester le moteur* (the one slow read).
    pub async fn admin_transcribe(&self, probe: bool) -> Result<AdminTranscribe, NzError> {
        self.core()
            .admin_transcribe(probe)
            .await
            .map(AdminTranscribe::from)
            .map_err(NzError::from)
    }

    /// `PUT /api/transcribe/settings` with ONE field (`{"max_minutes": 30}`),
    /// whose value travels as JSON TEXT.
    pub async fn admin_save_transcribe(
        &self,
        field: String,
        value: String,
    ) -> Result<AdminTranscribe, NzError> {
        self.core()
            .admin_save_transcribe(&field, &value)
            .await
            .map(AdminTranscribe::from)
            .map_err(NzError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{CoreConfig, NzCore};

    /// The boundary really is a `Screens` block: the records convert, and every
    /// number the Kotlin side reads is the one `nz-core` decided.
    #[test]
    fn the_records_carry_what_the_cards_draw() {
        let row = AdminUserRow::from(core_admin::UserView {
            id: "u-2".to_owned(),
            name: "Léo".to_owned(),
            admin: false,
            disabled: true,
            jukebox: false,
            last_login: None,
            mail: Some("leo@example.org".to_owned()),
            onboarding: Some("done".to_owned()),
        });
        assert_eq!(row.id, "u-2");
        assert!(row.disabled);
        assert_eq!(row.mail.as_deref(), Some("leo@example.org"));

        let failures = AdminJobFailures::from(core_admin::JobFailuresView {
            failed: 12,
            causes: vec![core_admin::CauseView {
                reason: "ffmpeg a disparu".to_owned(),
                count: 10,
            }],
            rows: vec![core_admin::FailureRow {
                track: "a.flac".to_owned(),
                path: "/srv/a.flac".to_owned(),
                reason: "ffmpeg a disparu".to_owned(),
            }],
            hidden: 11,
        });
        assert_eq!(failures.failed, 12);
        assert_eq!(failures.causes[0].reason, "ffmpeg a disparu");
        assert_eq!(failures.rows[0].track, "a.flac");
        assert_eq!(failures.hidden, 11, "the count the card prints");

        let storage = AdminStorage::from(core_admin::StorageView {
            archive_dir: "/srv/archive".to_owned(),
            archive_bytes: 12,
            tracks_archived: 8,
            tracks_total: 9,
            ..core_admin::StorageView::default()
        });
        assert_eq!(storage.archive_bytes, 12);
        assert_eq!(storage.tracks_archived, 8);
    }

    /// A member is told so by the session, with no request: the admin pages ask
    /// nothing at all for them.
    #[test]
    fn the_identity_answers_from_the_session_alone() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nz-ffi-admin-tests")
            .join(format!("identity-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a directory");
        let core = NzCore::new(CoreConfig {
            data_dir: dir.join("data").display().to_string(),
            cache_dir: dir.join("cache").display().to_string(),
            app_version: "0.0.0-test".to_owned(),
            version_code: 1,
            sdk_int: 35,
            device_name: "test".to_owned(),
        })
        .expect("a core");
        // No session yet: no identity, and nothing asked.
        assert!(core.screens().admin_identity().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
