//! The administrator's payloads, screen-shaped.
//!
//! Why this layer exists at all (MB16's reason, and the FFI boundary's rule):
//! `nz-ffi` carries no `serde_json`, and three of the payloads this lot reads
//! have fields UniFFI cannot take (`AdminUsers.next`, a job's `failures` rows,
//! the automatch's `job`/`last`/`recent` objects). Each one is read HERE, once,
//! and handed to the boundary as a plain struct — never as a `Value` the Kotlin
//! side would have to guess at.
//!
//! Two rules the conversions follow:
//!
//! - **What a card DRAWS is what crosses**: a user row carries its name, its
//!   role, its state and when it was last seen — not the eighteen columns the
//!   server also sends.
//! - **A cause is never dropped.** A job's `failure_reasons` become
//!   [`CauseView`] rows, its `failures` become [`FailureRow`]s, and `hidden`
//!   says how many the server's sample left out (`JobFailures.svelte`'s rule:
//!   never let the list imply it is whole).
//!
//! The French words stay in Kotlin (`AdminCopy.kt`), exactly as MB16 kept
//! Réglages' copy there: this layer names ids and numbers.

use std::collections::BTreeMap;

use nz_model::{
    AdminInvite, AdminUser, AdminUsers, AnalysisBackfill, AnalysisPlan, ArchiveRuleSet,
    ArchiveRules, ArchiveStatus, Automatch, CleanupPreview, CreditsBackfill, DeezerStatus,
    EnrichStatus, JobStatus, MigrateStatus, ServerSettings, SimilarStatus, Storage,
    TranscribeQueue, TranscribeSettings,
};
use serde_json::Value;

// --- people ------------------------------------------------------------------

/// One account, as `/api/admin/users` sends it and the row draws it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserView {
    pub id: String,
    pub name: String,
    pub admin: bool,
    pub disabled: bool,
    /// Subsonic's own jukebox role — the SPA never draws it, and neither does
    /// this screen; it is kept so a future row can say it without a new route.
    pub jukebox: bool,
    /// RFC 3339, `None` for an account that never signed in.
    pub last_login: Option<String>,
    pub mail: Option<String>,
    /// Where the person's onboarding stands (`"done"`, `"pending"`, …).
    pub onboarding: Option<String>,
}

impl From<&AdminUser> for UserView {
    fn from(user: &AdminUser) -> Self {
        UserView {
            id: user.id.to_string(),
            name: user.name.clone(),
            admin: user.admin,
            disabled: user.disabled,
            jukebox: user.jukebox,
            last_login: user.last_login.clone(),
            mail: user.mail.clone(),
            onboarding: user.onboarding.clone(),
        }
    }
}

/// One page of `/api/admin/users`, with the keyset cursor of the next one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsersPageView {
    pub users: Vec<UserView>,
    /// How many accounts the server has IN TOTAL for this query — the count line
    /// says "12 sur 340" with it (`AdminUsers.svelte#rangeLabel`).
    pub total: u32,
    /// The last row's id, `None` on the last page.
    pub next: Option<String>,
}

impl From<&AdminUsers> for UsersPageView {
    fn from(page: &AdminUsers) -> Self {
        UsersPageView {
            users: page.users.iter().map(UserView::from).collect(),
            total: page.total,
            // The route answers the last row's id (a string), not an object.
            next: page
                .next
                .as_ref()
                .and_then(Value::as_str)
                .map(str::to_owned)
                .filter(|id| !id.is_empty()),
        }
    }
}

/// `GET /api/admin/users/<id>` — the account and what deleting it would move.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserDetailView {
    pub user: UserView,
    /// Files this account uploaded, which a deletion REASSIGNS (never deletes).
    pub uploads: u32,
    pub playlists: u32,
}

/// What `DELETE /api/admin/users/<id>` moved, for the toast.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeleteResultView {
    pub uploads_reassigned: u32,
    pub podcasts_reassigned: u32,
}

impl DeleteResultView {
    /// Reads the server's answer (`{"deleted", "uploads_reassigned",
    /// "podcasts_reassigned"}`). A missing counter is 0, never a refusal.
    pub fn from_value(answer: &Value) -> Self {
        let read = |key: &str| {
            answer
                .get(key)
                .and_then(Value::as_u64)
                .unwrap_or_default()
                .min(u64::from(u32::MAX)) as u32
        };
        DeleteResultView {
            uploads_reassigned: read("uploads_reassigned"),
            podcasts_reassigned: read("podcasts_reassigned"),
        }
    }
}

// --- invitations ---------------------------------------------------------------

/// One invitation. The LINK is not here: the server sends its token once, at
/// creation, and the screen that holds it is [`InviteIssuedView`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InviteView {
    pub id: String,
    /// The admin's own note ("pour qui ?"), never shown to the invited person.
    pub note: Option<String>,
    pub created: Option<String>,
    pub expires: Option<String>,
    /// `pending`, `used`, `revoked` or `expired`.
    pub state: String,
    pub created_by: Option<String>,
    pub used_by: Option<String>,
}

impl From<&AdminInvite> for InviteView {
    fn from(invite: &AdminInvite) -> Self {
        InviteView {
            id: invite.id.clone(),
            note: invite.note.clone(),
            created: invite.created.clone(),
            expires: invite.expires.clone(),
            state: if invite.state.is_empty() {
                "pending".to_owned()
            } else {
                invite.state.clone()
            },
            created_by: invite.created_by.clone(),
            used_by: invite.used_by.clone(),
        }
    }
}

/// A freshly minted invitation: the row, and the URL to hand over. The token
/// exists in this answer ONCE (`invites.py`: it is not stored in clear).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InviteIssuedView {
    pub invite: InviteView,
    /// `{origin}/app/#/invite/{token}` — the SPA's own link shape
    /// (`adminusers.js#inviteUrl`), so the invited person can open it in a
    /// browser or paste it into the app.
    pub url: String,
    /// The bare token, for the app's own `neezer://invite/<code>` link (MB5b).
    pub token: String,
}

// --- the archive ------------------------------------------------------------------

/// `/api/storage`, as the *Stockage serveur* card draws it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StorageView {
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

impl From<&Storage> for StorageView {
    fn from(storage: &Storage) -> Self {
        StorageView {
            archive_dir: storage.archive_dir.clone(),
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

/// `/api/archive/status` — the backfill's gauge.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArchiveStatusView {
    pub running: bool,
    pub scope: Option<String>,
    pub total: u32,
    pub done: u32,
    pub archived: u32,
    pub failed: u32,
    pub unavailable: u32,
    pub error: Option<String>,
}

impl From<&ArchiveStatus> for ArchiveStatusView {
    fn from(status: &ArchiveStatus) -> Self {
        ArchiveStatusView {
            running: status.running,
            scope: status.scope.clone(),
            total: status.total,
            done: status.done,
            archived: status.archived,
            failed: status.failed,
            unavailable: status.unavailable,
            error: status.error.clone(),
        }
    }
}

/// `/api/archive/migrate` — the format migration's gauge and its per-step
/// counters (`tags`, `sidecar`, `bpm`, …), which the SPA shows as
/// "N étiquettes · N tempos appris".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MigrateView {
    pub running: bool,
    pub done: u32,
    pub failed: u32,
    pub error: Option<String>,
    pub counts: Vec<CountView>,
    pub causes: Vec<CauseView>,
    pub hidden: u32,
}

impl From<&MigrateStatus> for MigrateView {
    fn from(status: &MigrateStatus) -> Self {
        MigrateView {
            running: status.running,
            done: status.done,
            failed: status.failed,
            error: status.error.clone(),
            counts: counts_of(&status.counts),
            causes: causes_of(&status.failure_reasons),
            hidden: hidden_of(status.failed, status.failures.len()),
        }
    }
}

/// One row of a *Cleanup* preview: a title that would leave, and its size.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CleanupTrackView {
    pub title: String,
    pub artist: String,
    pub bytes: u64,
}

/// `/api/archive/cleanup/preview` — what a cleanup would delete RIGHT NOW.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CleanupView {
    pub enabled: bool,
    pub running: bool,
    pub eligible: u32,
    pub would_free: u64,
    pub free_bytes: u64,
    /// How much room is missing to get back above the threshold; 0 means the
    /// disk is fine and NOTHING would be deleted.
    pub needed_bytes: u64,
    pub tracks: Vec<CleanupTrackView>,
}

impl From<&CleanupPreview> for CleanupView {
    fn from(preview: &CleanupPreview) -> Self {
        CleanupView {
            enabled: preview.enabled,
            running: preview.running,
            eligible: preview.eligible,
            would_free: preview.would_free,
            free_bytes: preview.free_bytes,
            needed_bytes: preview.needed_bytes,
            // The server caps the sample; the card shows the first twelve
            // (`Settings.svelte#slice(0, 12)`), and the count line says the rest.
            tracks: preview
                .tracks
                .iter()
                .map(|row| CleanupTrackView {
                    title: text_of(row, "title"),
                    artist: text_of(row, "artist"),
                    bytes: row.get("bytes").and_then(Value::as_u64).unwrap_or_default(),
                })
                .collect(),
        }
    }
}

/// One archiving event switch: its id and whether it is on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleSwitchView {
    /// The server's own id (`on_fav_album`, `on_play_context`, …).
    pub id: String,
    pub on: bool,
}

/// The rule set itself, one field per control (`Settings.svelte#saveRule`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RuleSetView {
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

impl From<&ArchiveRuleSet> for RuleSetView {
    fn from(rules: &ArchiveRuleSet) -> Self {
        RuleSetView {
            artist_scope: rules.artist_scope.clone(),
            artist_limit: rules.artist_limit,
            chapters: rules.chapters,
            chapter_urls: rules.chapter_urls,
            clean_on: rules.clean_on,
            clean_min_free_gb: rules.clean_min_free_gb,
            clean_stale_days: rules.clean_stale_days,
            clean_order: rules.clean_order.clone(),
            clean_keep_fav: rules.clean_keep_fav,
            clean_keep_playlist: rules.clean_keep_playlist,
            clean_keep_podcast: rules.clean_keep_podcast,
        }
    }
}

/// `/api/archive/rules` — the rules, the events in the screen's order, and the
/// vocabulary each picker offers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RulesView {
    pub events: Vec<RuleSwitchView>,
    pub rules: RuleSetView,
    pub artist_scopes: Vec<String>,
    pub cleanup_orders: Vec<String>,
    /// The master switch lives in the server's config file, not in these rules
    /// and not in this app: when it is off the card says the rules do nothing.
    pub archive_library: bool,
}

impl From<&ArchiveRules> for RulesView {
    fn from(rules: &ArchiveRules) -> Self {
        let on = |id: &str| match id {
            "on_fav_track" => rules.rules.on_fav_track,
            "on_fav_album" => rules.rules.on_fav_album,
            "on_fav_artist" => rules.rules.on_fav_artist,
            "on_fav_playlist" => rules.rules.on_fav_playlist,
            "on_playlist_add" => rules.rules.on_playlist_add,
            "on_play_context" => rules.rules.on_play_context,
            "on_podcast" => rules.rules.on_podcast,
            _ => false,
        };
        RulesView {
            events: rules
                .events
                .iter()
                .map(|id| RuleSwitchView {
                    id: id.clone(),
                    on: on(id),
                })
                .collect(),
            rules: RuleSetView::from(&rules.rules),
            artist_scopes: rules.artist_scopes.clone(),
            cleanup_orders: rules.cleanup_orders.clone(),
            archive_library: rules.archive_library,
        }
    }
}

// --- jobs ------------------------------------------------------------------------

/// One counter, named (`{"archived": 12}`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CountView {
    pub name: String,
    pub count: u32,
}

/// One reason a job failed, and how many tracks it took: the tally that names
/// the DOMINANT cause instead of the first one (`JobFailures.svelte`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CauseView {
    pub reason: String,
    pub count: u32,
}

/// One failed track, with the cause it gave (`{track, path, id, reason}` on the
/// wire, `deezer/analysis.py#_record_failure`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FailureRow {
    pub track: String,
    pub path: String,
    pub reason: String,
}

/// A library job, as `JobFailures.svelte` draws it: how many failed, the tally of
/// causes, the sample of rows, and how many the server's sample left out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobFailuresView {
    pub failed: u32,
    pub causes: Vec<CauseView>,
    pub rows: Vec<FailureRow>,
    /// `failed` minus the rows we hold: the server keeps a bounded sample, and
    /// saying so is what keeps the list honest.
    pub hidden: u32,
}

impl JobFailuresView {
    /// From the shared job counters (`AnalysisBackfill`, `CreditsBackfill`, …).
    pub fn of(job: &JobStatus) -> Self {
        JobFailuresView {
            failed: job.failed,
            causes: causes_of(&job.failure_reasons),
            rows: job.failures.iter().map(failure_row_of).collect(),
            hidden: hidden_of(job.failed, job.failures.len()),
        }
    }
}

/// One step of the analysis pipeline, with how many tracks are behind on it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StageView {
    /// `measure`, `embed` or `decide`.
    pub id: String,
    /// Tracks this step still has to look at. 0 means it is up to date.
    pub late: u32,
    /// The step cannot run at all (no extractor installed).
    pub off: bool,
}

/// The nightly window (`GET /api/analysis/plan`'s `schedule`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScheduleView {
    pub enabled: bool,
    /// `HH:MM`, the server's own local time.
    pub at: String,
    pub hours: u32,
}

/// `/api/analysis/plan` — what is out of date, at which step, and where it runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanView {
    /// `server` or `client`: who measures the library.
    pub place: String,
    pub total: u32,
    /// Tracks with something to do.
    pub todo: u32,
    /// Tracks never measured at all.
    pub never: u32,
    /// Tracks whose measurement is current.
    pub current: u32,
    pub stages: Vec<StageView>,
    /// The frozen ONNX extractor is installed; step 2 is dead without it.
    pub extractor: bool,
    pub schedule: ScheduleView,
}

impl From<&AnalysisPlan> for PlanView {
    fn from(plan: &AnalysisPlan) -> Self {
        // A track never measured is behind on every step; the `stale_*` counts
        // are the rows behind on that step only (`GenreLibrary.svelte#steps`).
        let never = plan.unmeasured;
        PlanView {
            place: plan.r#where.clone(),
            total: plan.total,
            todo: plan.todo,
            never,
            current: plan.current,
            stages: vec![
                StageView {
                    id: "measure".to_owned(),
                    late: never.saturating_add(plan.stale_measure),
                    off: false,
                },
                StageView {
                    id: "embed".to_owned(),
                    late: if plan.extractor {
                        never.saturating_add(plan.stale_embed)
                    } else {
                        0
                    },
                    off: !plan.extractor,
                },
                StageView {
                    id: "decide".to_owned(),
                    late: never.saturating_add(plan.stale_decide),
                    off: false,
                },
            ],
            extractor: plan.extractor,
            schedule: ScheduleView {
                enabled: plan.schedule.enabled,
                at: plan.schedule.at.clone(),
                hours: plan.schedule.hours,
            },
        }
    }
}

/// `/api/analysis/backfill` — the measure-all job, its stage and its ledger.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnalysisJobView {
    pub running: bool,
    pub scanned: u32,
    pub done: u32,
    pub total: u32,
    /// `measure`, `embed` or `decide`.
    pub stage: Option<String>,
    /// Why the job is waiting — the server's own word (`listening`,
    /// `download`, `load`), never a number.
    pub paused: Option<String>,
    pub error: Option<String>,
    /// The parallelism in force and what the admin asked for (0 = auto).
    pub workers: u32,
    pub workers_setting: u32,
    pub workers_auto: u32,
    pub force: bool,
    pub failures: JobFailuresView,
}

impl AnalysisJobView {
    /// `paused` is the server's word, but the shared model types it as a
    /// boolean: a string would make the WHOLE payload unreadable, so the raw
    /// answer is read here and handed on as the word the card prints.
    pub fn of(job: &AnalysisBackfill, paused: Option<String>) -> Self {
        AnalysisJobView {
            running: job.job.running,
            scanned: job.job.scanned,
            done: job.job.done,
            total: job.job.total,
            stage: job.stage.clone(),
            paused,
            error: job.job.error.clone(),
            workers: job.workers,
            workers_setting: job.workers_setting,
            workers_auto: job.workers_auto,
            force: job.force,
            failures: JobFailuresView::of(&job.job),
        }
    }
}

/// `/api/enrich/backfill` — MusicBrainz's coverage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnrichView {
    /// Off in the server's configuration (`[enrich] enabled`): the button is
    /// refused, and the card says why.
    pub enabled: bool,
    pub running: bool,
    /// The sound-fingerprint lookup is usable (fpcalc + a key).
    pub acoustid: bool,
    /// Titles enriched so far, and how, by method (`isrc`, `acoustid`, `search`).
    pub found: Vec<CountView>,
    pub tried: u32,
    pub untried: u32,
    pub error: Option<String>,
    pub failures: JobFailuresView,
}

impl From<&EnrichStatus> for EnrichView {
    fn from(status: &EnrichStatus) -> Self {
        // The route carries the COVERAGE (`tried`, `untried`, `found` by method)
        // and no per-title ledger: `EnrichBackfill.svelte` draws `job.failed`
        // from counters this route does not send, so the card shows the coverage
        // and invents no failure block.
        EnrichView {
            enabled: status.enabled,
            running: status.running,
            acoustid: status.acoustid,
            found: counts_of(&status.coverage.found),
            tried: status.coverage.tried,
            untried: status.coverage.untried,
            error: None,
            failures: JobFailuresView::default(),
        }
    }
}

/// `/api/credits/backfill` — the credits worker.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreditsView {
    pub running: bool,
    pub done: u32,
    pub failed: u32,
    pub total: u32,
    /// How many titles already have their credits.
    pub tracks: u32,
    /// The library archived before the credits existed has been completed.
    pub completed: bool,
    pub error: Option<String>,
    /// Where the credits came from (`deezer`, `local`).
    pub sources: Vec<CountView>,
    pub failures: JobFailuresView,
}

impl From<&CreditsBackfill> for CreditsView {
    fn from(job: &CreditsBackfill) -> Self {
        CreditsView {
            running: job.job.running,
            done: job.job.done,
            failed: job.job.failed,
            total: job.job.total,
            tracks: job.tracks,
            completed: job.completed,
            error: job.job.error.clone(),
            sources: counts_of(&job.counts),
            failures: JobFailuresView::of(&job.job),
        }
    }
}

/// `/api/similar/rebuild` — the "sounds like" index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SimilarView {
    pub running: bool,
    /// Titles that already have their fifty neighbours.
    pub tracks: u32,
    /// Unix seconds of the last rebuild, `None` when it never ran.
    pub at: Option<i64>,
}

impl From<&SimilarStatus> for SimilarView {
    fn from(status: &SimilarStatus) -> Self {
        SimilarView {
            running: status.running,
            tracks: status.tracks,
            at: status.at,
        }
    }
}

// --- availability (the audio auto-match) ------------------------------------------

/// One title the auto-match replaced: what left, what arrived, and the score.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReplacedView {
    /// `at`, RFC 3339 — the run's own stamp on a row of `recent`.
    pub at: String,
    pub from_title: String,
    pub from_artist: String,
    pub to_title: String,
    pub to_source: String,
    /// 0..1 — the confidence that made it the same recording (99 % or nothing).
    pub score: f64,
}

/// One title the auto-match left for a person to handle.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AutomatchMissView {
    pub title: String,
    pub artist: String,
    /// The server's own English cause; the card translates the known ones.
    pub reason: String,
    /// `error`, `no_match`, `no_reference`, …
    pub status: String,
}

/// `/api/automatch` — the switch, the running pass, and the last one's outcome.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AutomatchView {
    /// numpy and ffmpeg are both there: without them nothing can be compared.
    pub available: bool,
    pub enabled: bool,
    /// The admin switched it off explicitly (as opposed to never set).
    pub switched_off: bool,
    /// The YouTube fallback may stand in.
    pub youtube: bool,
    pub threshold: f64,
    /// The pass in flight, when one is.
    pub running: bool,
    /// `None` until the server has filled the total in.
    pub total: Option<u32>,
    pub done: u32,
    pub matched: u32,
    pub cancel: bool,
    /// The last finished pass: its counts, whether it stopped early, and the
    /// titles it left for a person.
    pub last_counts: Vec<CountView>,
    pub last_stopped: Option<String>,
    pub misses: Vec<AutomatchMissView>,
    /// The most recent replacements, newest first.
    pub recent: Vec<ReplacedView>,
}

impl From<&Automatch> for AutomatchView {
    fn from(status: &Automatch) -> Self {
        let job = status.job.as_ref();
        let last = status.last.as_ref();
        let running = job
            .and_then(|job| job.get("running"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let counts = last
            .and_then(|last| last.get("counts"))
            .and_then(Value::as_object)
            .map(|counts| {
                let mut rows: Vec<CountView> = counts
                    .iter()
                    .map(|(name, count)| CountView {
                        name: name.clone(),
                        count: u32_of(count),
                    })
                    .collect();
                rows.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));
                rows
            })
            .unwrap_or_default();
        AutomatchView {
            available: status.available,
            enabled: status.enabled,
            switched_off: status.switched_off,
            youtube: status.youtube,
            threshold: status.threshold,
            running,
            total: job
                .and_then(|job| job.get("total"))
                .and_then(Value::as_u64)
                .map(|total| total.min(u64::from(u32::MAX)) as u32),
            done: job
                .and_then(|job| job.get("done"))
                .map(u32_of)
                .unwrap_or_default(),
            matched: job
                .and_then(|job| job.get("counts"))
                .and_then(|counts| counts.get("match"))
                .map(u32_of)
                .unwrap_or_default(),
            cancel: job
                .and_then(|job| job.get("cancel"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            last_counts: counts,
            last_stopped: last
                .and_then(|last| last.get("stopped"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            misses: last
                .and_then(|last| last.get("failures"))
                .and_then(Value::as_array)
                .map(|rows| rows.iter().map(miss_of).collect())
                .unwrap_or_default(),
            recent: status.recent.iter().map(replaced_of).collect(),
        }
    }
}

// --- transcription -----------------------------------------------------------------

/// `/api/transcribe/settings` — the engine, the model, the queue and the four
/// numbers the card edits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TranscribeView {
    /// The engine exists on the server: without it the card only says so.
    pub available: bool,
    pub enabled: bool,
    /// `server` or `external`.
    pub runner: Option<String>,
    /// Why it is off (`no_model`, `disabled`, …) — the server's word.
    pub reason: Option<String>,
    pub language: String,
    /// The languages the picker offers, `auto` first.
    pub languages: Vec<String>,
    pub max_minutes: u32,
    pub daily_minutes: u32,
    pub search_segments: u64,
    /// Threads the engine may use on the server; 0 when it did not say.
    pub threads: u32,
    pub engine_name: Option<String>,
    pub engine_version: Option<String>,
    pub engine_ok: bool,
    pub model_name: Option<String>,
    pub model_size_mb: u32,
    pub model_ok: bool,
    /// The queue: what is running, leased by a worker, or waiting.
    pub running_now: u32,
    pub queued: u32,
    pub ready: u32,
    pub errored: u32,
    pub budget_limit_ms: u64,
    pub budget_used_ms: u64,
}

impl From<&TranscribeSettings> for TranscribeView {
    fn from(settings: &TranscribeSettings) -> Self {
        let queue = settings.queue.clone();
        TranscribeView {
            available: settings.available,
            enabled: settings.enabled,
            runner: settings.runner.clone(),
            reason: settings.reason.clone(),
            language: settings.language.clone(),
            languages: settings.languages.clone(),
            max_minutes: settings.max_minutes,
            daily_minutes: settings.daily_minutes,
            search_segments: settings.search_segments,
            threads: settings.threads,
            engine_name: settings.engine.as_ref().map(|engine| engine.name.clone()),
            engine_version: settings
                .engine
                .as_ref()
                .and_then(|engine| engine.version.clone()),
            engine_ok: settings.engine.as_ref().is_some_and(|engine| engine.ok),
            model_name: settings.model.as_ref().and_then(|model| model.name.clone()),
            model_size_mb: settings
                .model
                .as_ref()
                .map(|model| model.size_mb)
                .unwrap_or_default(),
            model_ok: settings.model.as_ref().is_some_and(|model| model.ok),
            running_now: active_of(&queue),
            queued: queue.queued,
            ready: queue.ready,
            errored: queue.error,
            budget_limit_ms: queue.budget_limit_ms,
            budget_used_ms: queue.budget_used_ms,
        }
    }
}

// --- the server's settings and the provider's verdict ---------------------------------

/// `/api/settings` + `/api/deezer/status`, as the server page's two cards draw
/// them. The ARL itself is never here — only the masked hint and its source.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ServerCardView {
    /// The upload quota for non-admins, in GB (0 = no limit).
    pub upload_quota_gb: f64,
    pub deezer_enabled: bool,
    pub arl_set: bool,
    pub arl_source: Option<String>,
    pub arl_hint: Option<String>,
    /// The provider's own verdict: `None` until `/deezer/status` answered.
    pub status_ok: Option<bool>,
    /// `disabled`, `no_arl`, `expired`, `unreachable`, …
    pub status_reason: Option<String>,
    pub status_message: Option<String>,
}

impl ServerCardView {
    pub fn of(settings: &ServerSettings, status: Option<&DeezerStatus>) -> Self {
        ServerCardView {
            upload_quota_gb: settings.upload_quota_gb,
            deezer_enabled: settings.deezer.enabled,
            arl_set: settings.deezer.arl_set,
            arl_source: settings.deezer.arl_source.clone(),
            arl_hint: settings.deezer.arl_hint.clone(),
            status_ok: status.map(|status| status.ok),
            status_reason: status.and_then(|status| status.reason.clone()),
            status_message: status.and_then(|status| status.message.clone()),
        }
    }
}

// --- shared readers -------------------------------------------------------------------

/// A `Value`'s string field, empty when it is missing or not a string.
pub(super) fn text_of(row: &Value, key: &str) -> String {
    row.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// A `Value` as a count, saturating at `u32::MAX` (`done`/`total` are counts on
/// the wire; a server that grows past 4 G rows is not this screen's problem).
pub(super) fn u32_of(value: &Value) -> u32 {
    value.as_u64().unwrap_or_default().min(u64::from(u32::MAX)) as u32
}

/// A map of counters, in its own key order.
fn counts_of(map: &BTreeMap<String, u32>) -> Vec<CountView> {
    map.iter()
        .map(|(name, count)| CountView {
            name: name.clone(),
            count: *count,
        })
        .collect()
}

/// A job's failure tally, biggest cause first — the one the summary line names.
fn causes_of(map: &BTreeMap<String, u32>) -> Vec<CauseView> {
    let mut rows: Vec<CauseView> = map
        .iter()
        .map(|(reason, count)| CauseView {
            reason: reason.clone(),
            count: *count,
        })
        .collect();
    rows.sort_by(|a, b| b.count.cmp(&a.count).then(a.reason.cmp(&b.reason)));
    rows
}

/// How much of a job's failures the server's bounded sample left out.
fn hidden_of(failed: u32, kept: usize) -> u32 {
    failed.saturating_sub(kept.min(u32::MAX as usize) as u32)
}

/// One failure row of the server's ledger (`deezer/analysis.py#_record_failure`).
fn failure_row_of(row: &Value) -> FailureRow {
    let track = text_of(row, "track");
    FailureRow {
        track: if track.is_empty() {
            text_of(row, "title")
        } else {
            track
        },
        path: text_of(row, "path"),
        reason: text_of(row, "reason"),
    }
}

/// `running + leased` — what the queue line calls "en cours".
fn active_of(queue: &TranscribeQueue) -> u32 {
    queue.running.saturating_add(queue.leased)
}

/// One row of `recent` (`automatch.py#process`): what left, what arrived.
fn replaced_of(row: &Value) -> ReplacedView {
    let track = row.get("track").cloned().unwrap_or(Value::Null);
    let by = row.get("by").cloned().unwrap_or(Value::Null);
    ReplacedView {
        at: text_of(row, "at"),
        from_title: text_of(&track, "title"),
        from_artist: text_of(&track, "artist"),
        to_title: {
            let album = text_of(&by, "album");
            if album.is_empty() {
                text_of(&by, "title")
            } else {
                album
            }
        },
        to_source: text_of(&by, "source"),
        score: row.get("score").and_then(Value::as_f64).unwrap_or_default(),
    }
}

/// One title the auto-match could not settle.
fn miss_of(row: &Value) -> AutomatchMissView {
    let track = row.get("track").cloned().unwrap_or(Value::Null);
    AutomatchMissView {
        title: text_of(&track, "title"),
        artist: text_of(&track, "artist"),
        reason: text_of(row, "reason"),
        status: text_of(row, "status"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The row the list draws carries the account's name, role and state, and the
    /// cursor is the LAST ROW'S ID — the route's own shape, not an object.
    #[test]
    fn a_users_page_carries_its_rows_and_its_cursor() {
        let page: AdminUsers = serde_json::from_str(
            r#"{"users":[
                 {"id":"8d1f0f5a-0000-4000-8000-000000000001","name":"Nadia","admin":true,
                  "disabled":false,"jukebox":false,"last_login":"2026-11-17T09:30:00",
                  "mail":"n@example.org","onboarding":"done"},
                 {"id":"8d1f0f5a-0000-4000-8000-000000000002","name":"Léo","admin":false,
                  "disabled":true,"last_login":null}],
                "next":"8d1f0f5a-0000-4000-8000-000000000002","total":42}"#,
        )
        .expect("a page of accounts");
        let view = UsersPageView::from(&page);
        assert_eq!(view.users.len(), 2);
        assert_eq!(view.users[0].name, "Nadia");
        assert!(view.users[0].admin);
        assert_eq!(view.total, 42);
        assert_eq!(
            view.next.as_deref(),
            Some("8d1f0f5a-0000-4000-8000-000000000002")
        );
        assert!(view.users[1].disabled);
        assert!(view.users[1].last_login.is_none());
    }

    /// A job's tally is ordered by count, so the summary names the DOMINANT
    /// cause, and `hidden` says how many the sample left out.
    #[test]
    fn a_jobs_failures_are_a_ranked_tally_and_its_sample() {
        let job: CreditsBackfill = serde_json::from_str(
            r#"{"failed":12,"done":30,"total":50,"tracks":7,"completed":true,
                "counts":{"deezer":5,"local":2},
                "failure_reasons":{"ffmpeg a disparu":10,"fichier tronqué":2},
                "failures":[{"track":"a.flac","path":"/srv/a.flac","reason":"ffmpeg a disparu"},
                            {"track":"b.flac","path":"/srv/b.flac","reason":"fichier tronqué"}]}"#,
        )
        .expect("the credits job");
        let view = CreditsView::from(&job);
        assert_eq!(view.failures.failed, 12);
        assert_eq!(view.failures.causes[0].reason, "ffmpeg a disparu");
        assert_eq!(view.failures.causes[0].count, 10);
        assert_eq!(view.failures.rows.len(), 2);
        assert_eq!(view.failures.rows[0].path, "/srv/a.flac");
        assert_eq!(view.failures.hidden, 10, "the server kept a sample");
        assert_eq!(view.tracks, 7);
        assert!(view.completed);
    }

    /// The analysis plan's three steps: a track never measured is behind on all
    /// of them, and the embedder is dead when the extractor is missing.
    #[test]
    fn the_plan_counts_each_step_the_way_the_studio_does() {
        let plan: AnalysisPlan = serde_json::from_str(
            r#"{"where":"server","total":100,"todo":30,"current":60,"unmeasured":10,
                "stale_measure":5,"stale_embed":20,"stale_decide":15,"extractor":false,
                "schedule":{"enabled":true,"at":"03:00","hours":6}}"#,
        )
        .expect("a plan");
        let view = PlanView::from(&plan);
        assert_eq!(view.stages[0].late, 15, "never + stale_measure");
        assert_eq!(view.stages[1].late, 0, "no extractor, nothing to do");
        assert!(view.stages[1].off);
        assert_eq!(view.stages[2].late, 25);
        assert_eq!(view.schedule.at, "03:00");
    }

    /// The auto-match's `job` and `last` are free-form on the wire: the counts
    /// come out biggest-first, and the replaced titles keep both sides.
    #[test]
    fn the_automatch_reads_its_job_and_its_recent_titles() {
        let status: Automatch = serde_json::from_str(
            r#"{"available":true,"enabled":true,"threshold":0.99,"youtube":false,
                "job":{"running":true,"total":40,"done":12,"counts":{"match":3},"cancel":false},
                "last":{"counts":{"match":2,"no_candidate":5,"error":1},
                        "stopped":"Deezer is unreachable",
                        "failures":[{"status":"no_candidate","reason":"nothing with the same title and artist",
                                     "track":{"id":"1","title":"Perdu","artist":"Anonyme"}}]},
                "recent":[{"at":"2026-11-17T10:00:00","score":0.995,
                           "track":{"id":"1","title":"Perdu","artist":"Anonyme"},
                           "by":{"id":"2","title":"Trouvé","artist":"Anonyme","album":"Raretés",
                                 "source":"youtube"}}]}"#,
        )
        .expect("the auto-match");
        let view = AutomatchView::from(&status);
        assert!(view.running);
        assert_eq!(view.done, 12);
        assert_eq!(view.matched, 3);
        assert_eq!(view.total, Some(40));
        assert_eq!(view.last_counts[0].name, "no_candidate");
        assert_eq!(view.last_counts[0].count, 5);
        assert_eq!(view.last_stopped.as_deref(), Some("Deezer is unreachable"));
        assert_eq!(view.misses[0].title, "Perdu");
        assert_eq!(view.recent[0].to_title, "Raretés");
        assert_eq!(view.recent[0].to_source, "youtube");
        assert!((view.recent[0].score - 0.995).abs() < f64::EPSILON);
    }

    /// The invite's row names the note the admin typed, and its state defaults to
    /// pending when the server omits it.
    #[test]
    fn an_invite_carries_its_note_and_its_state() {
        let invites: nz_model::AdminInvites = serde_json::from_str(
            r#"{"invites":[{"id":"i1","note":"Léa","created":"2026-11-17T09:00:00",
                            "expires":"2026-11-24T09:00:00","created_by":"Nadia"}],"next":null}"#,
        )
        .expect("invites");
        let view = InviteView::from(&invites.invites[0]);
        assert_eq!(view.note.as_deref(), Some("Léa"));
        assert_eq!(view.state, "pending");
    }

    /// The rule set maps each event id to its own switch: a typo would silently
    /// turn a rule off, so every id the server sends is answered.
    #[test]
    fn every_event_id_the_server_sends_has_its_switch() {
        let rules: ArchiveRules = serde_json::from_str(
            r#"{"rules":{"on_fav_track":true,"on_fav_album":false,"on_fav_artist":true,
                         "on_fav_playlist":true,"on_playlist_add":false,"on_play_context":true,
                         "on_podcast":true,"artist_scope":"releases","artist_limit":3,
                         "chapters":true,"chapter_urls":false,"clean_on":true,
                         "clean_stale_days":180,"clean_min_free_gb":20.5,
                         "clean_order":"oldest_play","clean_keep_fav":true,
                         "clean_keep_playlist":true,"clean_keep_podcast":false},
                "events":["on_play_context","on_fav_track","on_fav_album","on_fav_playlist",
                          "on_fav_artist","on_playlist_add","on_podcast"],
                "artist_scopes":["all","releases","top"],
                "cleanup_orders":["oldest_play","least_played","largest","oldest"],
                "archive_library":true}"#,
        )
        .expect("the rules");
        let view = RulesView::from(&rules);
        assert_eq!(view.events.len(), 7);
        assert!(view.events[0].on, "on_play_context");
        assert!(!view.events[2].on, "on_fav_album");
        assert!(view.events[6].on, "on_podcast");
        assert_eq!(view.rules.clean_min_free_gb, 20.5);
        assert_eq!(view.rules.artist_limit, 3);
        assert_eq!(view.artist_scopes.len(), 3);
        assert!(view.archive_library);
    }

    /// The transcription settings: the engine's verdict, the model's size and the
    /// queue's own split ("2 en cours" = running + leased).
    #[test]
    fn the_transcription_settings_name_the_engine_and_the_queue() {
        let settings: TranscribeSettings = serde_json::from_str(
            r#"{"available":true,"enabled":true,"language":"fr","languages":["auto","fr","en"],
                "daily_minutes":60,"max_minutes":180,"threads":4,"search_segments":100000,
                "engine":{"name":"whisper.cpp","ok":true,"version":"1.7.4"},
                "model":{"name":"ggml-small.bin","ok":true,"size_mb":466},
                "queue":{"queued":3,"leased":1,"running":1,"ready":9,"error":2,
                         "budget_limit_ms":3600000,"budget_used_ms":600000}}"#,
        )
        .expect("the transcription settings");
        let view = TranscribeView::from(&settings);
        assert!(view.available);
        assert_eq!(view.engine_name.as_deref(), Some("whisper.cpp"));
        assert_eq!(view.engine_version.as_deref(), Some("1.7.4"));
        assert_eq!(view.model_size_mb, 466);
        assert_eq!(view.running_now, 2);
        assert_eq!(view.queued, 3);
        assert_eq!(view.errored, 2);
        assert_eq!(view.languages.first().map(String::as_str), Some("auto"));
    }
}
