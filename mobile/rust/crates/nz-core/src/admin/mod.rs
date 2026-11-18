//! Réglages, the administrator's sections. Owned by **MB17**
//! (docs/agents/mobile.md, "Module and file ownership", and its **MB17**
//! paragraph): the lot fills this directory and edits no other file of the crate.
//!
//! The behaviour is the SPA's, PORTED — never re-invented. Every card below says
//! which component it is, and every route is the one that component calls:
//!
//! | This module | The SPA |
//! |---|---|
//! | [`Core::admin_users`], [`Core::admin_user`], [`Core::admin_set_role`], [`Core::admin_set_disabled`], [`Core::admin_delete_user`] | `components/AdminUsers.svelte`, `lib/adminusers.js` |
//! | [`Core::admin_invites`], [`Core::admin_create_invite`], [`Core::admin_renew_invite`], [`Core::admin_revoke_invite`] | `components/InviteSheet.svelte` |
//! | [`Core::admin_storage`], [`Core::admin_flush_cache`], [`Core::admin_server_card`] | `Settings.svelte`'s *Stockage serveur* and *Quota d'upload* cards |
//! | [`Core::admin_rules`], [`Core::admin_save_rule`] | `Settings.svelte`'s *Règles d'archivage* |
//! | [`Core::admin_cleanup`], [`Core::admin_run_cleanup`] | `Settings.svelte`'s *Nettoyage automatique* (preview, then run) |
//! | [`Core::admin_archive_status`], [`Core::admin_start_archive`] | `Settings.svelte`'s *Archive du serveur* |
//! | [`Core::admin_migrate`], [`Core::admin_start_migrate`] | `Settings.svelte`'s *Mise à niveau des fichiers* |
//! | [`Core::admin_plan`], [`Core::admin_analysis`], [`Core::admin_start_analysis`], [`Core::admin_stop_analysis`], [`Core::admin_save_schedule`], [`Core::admin_save_analysis_place`] | `components/GenreLibrary.svelte`'s upkeep card |
//! | [`Core::admin_enrich`], [`Core::admin_start_enrich`] | `components/EnrichBackfill.svelte` |
//! | [`Core::admin_credits`], [`Core::admin_start_credits`] | `components/CreditsBackfill.svelte` |
//! | [`Core::admin_similar`], [`Core::admin_start_similar`] | `components/SimilarRebuild.svelte` |
//! | [`Core::admin_automatch`], [`Core::admin_set_automatch`], [`Core::admin_run_automatch`], [`Core::admin_cancel_automatch`] | `components/AutomatchPanel.svelte` |
//! | [`Core::admin_transcribe`], [`Core::admin_save_transcribe`], [`Core::admin_probe_transcribe`] | `components/TranscribeSettings.svelte` |
//! | [`Core::admin_job_failures`] (the [`JobFailuresView`] every job carries) | `components/JobFailures.svelte` |
//!
//! # The request budget (rule 3)
//!
//! **`Utilisateurs`** is the SPA's own two reads: `/admin/users` and
//! `/admin/invites` (the dashboard's first page and its pending invitations).
//! One page of 25, and *Charger plus* continues the key set — never the whole
//! server in one answer. `MAX_ROWS` is the SPA's 250: past that the screen asks
//! for a narrower search instead of holding the table.
//!
//! **`Serveur`** is the SPA's own ten: `/settings`, `/deezer/status`,
//! `/storage`, `/archive/rules`, `/archive/cleanup/preview`, `/archive/status`,
//! `/archive/migrate`, `/enrich/backfill`, `/credits/backfill`,
//! `/transcribe/settings` — plus the two the upkeep card owns
//! (`/analysis/plan`, `/analysis/backfill`), which the SPA reads when its
//! *Genres* page opens. Twelve, all on the `User` lane, counted under
//! [`TAG_ADMIN`].
//!
//! A WARM open asks ZERO: [`wire::Snapshot`] holds every card as it was last
//! read, so the page paints at once and refreshes behind the first frame.
//!
//! # Long work says so (rule 2)
//!
//! A start route returns at once and hands back the job's own state; the screen
//! polls ONLY while that state says `running`, and every job carries the CAUSE
//! of each failure ([`JobFailuresView`]) — never a bare count. A job that is
//! already running is refused with the server's own sentence (`409`), which the
//! card shows instead of pretending to start a second one.

mod view;
mod wire;

#[cfg(test)]
mod testing;

use nz_model::{AdminUser, AnalysisBackfill};

pub use view::{
    AnalysisJobView, ArchiveStatusView, AutomatchMissView, AutomatchView, CauseView,
    CleanupTrackView, CleanupView, CountView, CreditsView, DeleteResultView, EnrichView,
    FailureRow, InviteIssuedView, InviteView, JobFailuresView, MigrateView, PlanView, ReplacedView,
    RuleSetView, RuleSwitchView, RulesView, ScheduleView, ServerCardView, SimilarView, StageView,
    StorageView, TranscribeView, UserDetailView, UserView, UsersPageView,
};

use crate::core::{Core, CoreError};

/// The tag every request of these pages is counted under (`NzCore::stats`).
pub const TAG_ADMIN: &str = "admin";

/// How many accounts one page of *Utilisateurs* holds — the SPA's `PAGE_SIZE`.
pub const USERS_PAGE_SIZE: u32 = 25;

/// Where the SPA stops paging the account table (`adminusers.js#MAX_ROWS`): past
/// this the screen asks for a narrower search rather than holding more rows.
pub const USERS_MAX_ROWS: u32 = 250;

/// How many pending invitations the dashboard loads (`AdminUsers.svelte`).
pub const INVITES_LIMIT: u32 = 200;

/// The four validity durations the invite sheet offers, in seconds
/// (`InviteSheet.svelte#DURATIONS`), and the one it starts on (7 days).
pub const INVITE_DURATIONS: [u32; 4] = [3_600, 86_400, 604_800, 2_592_000];
pub const INVITE_DEFAULT_TTL: u32 = 604_800;

/// The scopes an archiving pass can sweep (`backfill.SCOPES`), in the screen's
/// own order: everything, favourites, playlists, podcasts.
pub const ARCHIVE_SCOPES: [&str; 4] = ["all", "favorites", "playlists", "podcasts"];

/// The three stages a measurement can be behind on (`GenreLibrary.svelte`).
pub const ANALYSIS_STAGES: [&str; 3] = ["measure", "embed", "decide"];

/// `/api/analysis/backfill` as the card needs it: the shared counters PLUS the
/// server's own word for why the job is waiting.
///
/// `AnalysisBackfill.paused` is typed as a boolean by `nz-model`, and the route
/// sends a REASON (`listening`, `download`, `load`) — reading it through the
/// typed struct would make the whole payload unreadable the moment the job
/// pauses, which is exactly when a person looks at it. The raw answer is read
/// in [`wire`] and split into the two things the card draws.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct AnalysisJob {
    #[serde(default)]
    pub job: AnalysisBackfill,
    /// Why the job is waiting, the server's own word: `None` when it is not.
    #[serde(default)]
    pub paused: Option<String>,
}

impl Core {
    // --- people ------------------------------------------------------------

    /// The device's copy of the account table, for the first frame.
    pub fn cached_admin_users(&self) -> Option<UsersPageView> {
        wire::Snapshot::read(self)
            .users
            .as_ref()
            .map(UsersPageView::from)
    }

    /// `GET /api/admin/users?q=&after=&limit=` — one page of accounts.
    ///
    /// `q` filters by name on the SERVER (that is what makes the search work on
    /// a server with thousands of accounts); `after` is the previous page's
    /// `next`. The rows are appended by the caller: this is one page, always.
    pub async fn admin_users(
        &self,
        q: &str,
        after: Option<&str>,
        limit: u32,
    ) -> Result<UsersPageView, CoreError> {
        let limit = limit.clamp(1, USERS_MAX_ROWS);
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_users(&api, q, after, limit).await;
        self.note_admin_traffic(&api, before);
        let page = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.users = Some(page.clone());
        });
        Ok(UsersPageView::from(&page))
    }

    /// `GET /api/admin/users/<id>` — one account, and what deleting it would move.
    pub async fn admin_user(&self, id: &str) -> Result<UserDetailView, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_user(&api, id).await;
        self.note_admin_traffic(&api, before);
        let answer = answer?;
        let user: AdminUser = answer
            .get("user")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|error| CoreError::Internal {
                detail: format!("/api/admin/users/{id}: {error}"),
            })?
            .ok_or_else(|| CoreError::Internal {
                detail: format!("/api/admin/users/{id}: aucun compte dans la réponse"),
            })?;
        let count = |key: &str| {
            answer
                .get(key)
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default()
                .min(u64::from(u32::MAX)) as u32
        };
        Ok(UserDetailView {
            user: UserView::from(&user),
            uploads: count("uploads"),
            playlists: count("playlists"),
        })
    }

    /// `PATCH /api/admin/users/<id>` with `{admin}` — *Nommer administrateur* /
    /// *Retirer les droits*. Optimistic: the device's copy takes the tap first,
    /// and a refusal puts it back and returns the cause.
    pub async fn admin_set_role(&self, id: &str, on: bool) -> Result<UserView, CoreError> {
        self.admin_patch_user(id, serde_json::json!({ "admin": on }), |user| {
            user.admin = on
        })
        .await
    }

    /// `PATCH /api/admin/users/<id>` with `{disabled}` — *Désactiver* /
    /// *Réactiver*. The server disconnects the person everywhere; nothing is
    /// deleted.
    pub async fn admin_set_disabled(&self, id: &str, on: bool) -> Result<UserView, CoreError> {
        self.admin_patch_user(id, serde_json::json!({ "disabled": on }), |user| {
            user.disabled = on
        })
        .await
    }

    /// The shared half of the two switches: one `PATCH`, the server's own row
    /// back, and the copy restored on a refusal (`last_admin`, `self_disable`).
    async fn admin_patch_user(
        &self,
        id: &str,
        payload: serde_json::Value,
        patch: impl FnOnce(&mut AdminUser) + Copy,
    ) -> Result<UserView, CoreError> {
        let previous = wire::Snapshot::read(self).users;
        patch_copy_users(self, id, patch);
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::patch_user(&api, id, &payload).await;
        self.note_admin_traffic(&api, before);
        let answer = match answer {
            Ok(answer) => answer,
            Err(error) => {
                wire::Snapshot::restore(self, |snapshot| snapshot.users = previous);
                return Err(error);
            }
        };
        let user: AdminUser = answer
            .get("user")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|error| CoreError::Internal {
                detail: format!("/api/admin/users/{id}: {error}"),
            })?
            .ok_or_else(|| CoreError::Internal {
                detail: format!("/api/admin/users/{id}: aucun compte dans la réponse"),
            })?;
        replace_copy_user(self, &user);
        Ok(UserView::from(&user))
    }

    /// `DELETE /api/admin/users/<id>` — the account, its playlists and its
    /// history go; the archived audio STAYS (CLAUDE.md: nothing deletes it) and
    /// the files this person uploaded move to the admin.
    ///
    /// The body confirms the account's NAME: this is the one destructive action
    /// of the screen, and the server refuses anything less.
    pub async fn admin_delete_user(
        &self,
        id: &str,
        name: &str,
    ) -> Result<DeleteResultView, CoreError> {
        let previous = wire::Snapshot::read(self).users;
        drop_copy_user(self, id);
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::delete_user(&api, id, name).await;
        self.note_admin_traffic(&api, before);
        let answer = match answer {
            Ok(answer) => answer,
            Err(error) => {
                wire::Snapshot::restore(self, |snapshot| snapshot.users = previous);
                return Err(error);
            }
        };
        Ok(DeleteResultView::from_value(&answer))
    }

    // --- invitations -------------------------------------------------------

    /// The device's copy of the pending invitations.
    pub fn cached_admin_invites(&self) -> Option<Vec<InviteView>> {
        wire::Snapshot::read(self).invites.map(|page| {
            page.invites
                .iter()
                .map(view::InviteView::from)
                .collect::<Vec<_>>()
        })
    }

    /// `GET /api/admin/invites?state=pending&limit=` — what nobody has redeemed
    /// yet, newest first.
    pub async fn admin_invites(&self, state: Option<&str>) -> Result<Vec<InviteView>, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_invites(&api, state, INVITES_LIMIT, None, None).await;
        self.note_admin_traffic(&api, before);
        let page = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.invites = Some(page.clone());
        });
        Ok(page.invites.iter().map(view::InviteView::from).collect())
    }

    /// `POST /api/admin/invites` — mint a link for somebody. `note` is the
    /// admin's own reminder ("Léa"), 64 characters at most; `expires_in` is one
    /// of [`INVITE_DURATIONS`].
    ///
    /// The token comes back ONCE and is turned into the SPA's own URL here, so
    /// the screen never builds a link out of a string it does not understand.
    pub async fn admin_create_invite(
        &self,
        note: Option<&str>,
        expires_in: u32,
    ) -> Result<InviteIssuedView, CoreError> {
        let expires_in = expires_in.clamp(3_600, 2_592_000);
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::create_invite(&api, note, expires_in).await;
        self.note_admin_traffic(&api, before);
        let answer = answer?;
        let issued = self.read_issued(answer)?;
        prepend_copy_invite(self, &issued.invite);
        Ok(issued)
    }

    /// `POST /api/admin/invites/<id>/renew` — revoke the old link and mint one
    /// with the same duration. The list replaces the row it names.
    pub async fn admin_renew_invite(&self, id: &str) -> Result<InviteIssuedView, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::renew_invite(&api, id).await;
        self.note_admin_traffic(&api, before);
        let answer = answer?;
        let issued = self.read_issued(answer)?;
        prepend_copy_invite(self, &issued.invite);
        Ok(issued)
    }

    /// `POST /api/admin/invites/<id>/revoke` — the link stops working now. The
    /// copy loses the row at once (`invite_used`/`invite_not_found` still mean
    /// the row is gone, and the caller re-reads).
    pub async fn admin_revoke_invite(&self, id: &str) -> Result<(), CoreError> {
        let previous = wire::Snapshot::read(self).invites;
        drop_copy_invite(self, id);
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::revoke_invite(&api, id).await;
        self.note_admin_traffic(&api, before);
        match answer {
            Ok(()) => Ok(()),
            Err(error @ CoreError::Server { status, .. }) if status == 404 || status == 409 => {
                // `invite_not_found` / `invite_used`: the row is not revocable
                // any more, so the list must not keep it either. The sentence
                // still reaches the person.
                Err(error)
            }
            Err(error) => {
                wire::Snapshot::restore(self, |snapshot| snapshot.invites = previous);
                Err(error)
            }
        }
    }

    /// The `{invite, token}` envelope the two minting routes answer.
    fn read_issued(&self, answer: serde_json::Value) -> Result<InviteIssuedView, CoreError> {
        let invite: nz_model::AdminInvite = answer
            .get("invite")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|error| CoreError::Internal {
                detail: format!("/api/admin/invites: {error}"),
            })?
            .ok_or_else(|| CoreError::Internal {
                detail: "/api/admin/invites: aucune invitation dans la réponse".to_owned(),
            })?;
        let token = answer
            .get("token")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        Ok(InviteIssuedView {
            invite: view::InviteView::from(&invite),
            url: invite_url(self, &token),
            token,
        })
    }

    // --- the archive -------------------------------------------------------

    pub fn cached_admin_storage(&self) -> Option<StorageView> {
        wire::Snapshot::read(self)
            .storage
            .as_ref()
            .map(StorageView::from)
    }

    /// `GET /api/storage` — the archive's size, the caches' size, and the disk.
    pub async fn admin_storage(&self) -> Result<StorageView, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_storage(&api).await;
        self.note_admin_traffic(&api, before);
        let storage = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.storage = Some(storage.clone());
        });
        Ok(StorageView::from(&storage))
    }

    pub fn cached_admin_server_card(&self) -> Option<ServerCardView> {
        let snapshot = wire::Snapshot::read(self);
        snapshot
            .server
            .as_ref()
            .map(|server| ServerCardView::of(server, snapshot.deezer.as_ref()))
    }

    /// `GET /api/settings` then `GET /api/deezer/status` — the quota and the
    /// provider's verdict. Two requests, in that order, exactly as
    /// `Settings.svelte#loadSettings` + `#refreshDeezerStatus` fire them.
    pub async fn admin_server_card(&self) -> Result<ServerCardView, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_server_settings(&api).await;
        self.note_admin_traffic(&api, before);
        let server = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.server = Some(server.clone());
        });
        self.admin_deezer_status(&server, false).await
    }

    /// `GET /api/deezer/status[?force=1]` — *Tester* re-tests the credential.
    pub async fn admin_test_deezer(&self, force: bool) -> Result<ServerCardView, CoreError> {
        let snapshot = wire::Snapshot::read(self);
        let server = snapshot
            .server
            .as_ref()
            .ok_or_else(|| CoreError::Internal {
                detail: "l'état du serveur n'a pas encore été lu".to_owned(),
            })?
            .clone();
        self.admin_deezer_status(&server, force).await
    }

    async fn admin_deezer_status(
        &self,
        server: &nz_model::ServerSettings,
        force: bool,
    ) -> Result<ServerCardView, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_deezer_status(&api, force).await;
        self.note_admin_traffic(&api, before);
        let status = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.deezer = Some(status.clone());
        });
        Ok(ServerCardView::of(server, Some(&status)))
    }

    /// `POST /api/settings` with `{upload_quota_gb}` — the cap on what a
    /// non-admin may upload, in GB, 0 for no limit.
    pub async fn admin_save_quota(&self, gb: f64) -> Result<ServerCardView, CoreError> {
        let gb = gb.clamp(0.0, 1_000_000.0);
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer =
            wire::save_server_settings(&api, &serde_json::json!({ "upload_quota_gb": gb })).await;
        self.note_admin_traffic(&api, before);
        let server = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.server = Some(server.clone());
        });
        let status = wire::Snapshot::read(self).deezer;
        Ok(ServerCardView::of(&server, status.as_ref()))
    }

    /// `POST /api/cache/flush` — empties the transcodes and the proxied covers.
    /// The ARCHIVE is not a cache and no route here can touch it.
    pub async fn admin_flush_cache(&self) -> Result<u64, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::flush_cache(&api).await;
        self.note_admin_traffic(&api, before);
        answer
    }

    pub fn cached_admin_rules(&self) -> Option<RulesView> {
        wire::Snapshot::read(self)
            .rules
            .as_ref()
            .map(RulesView::from)
    }

    /// `GET /api/archive/rules` — the events that archive, the artist scope and
    /// the cleanup policy, with the vocabulary each picker draws.
    pub async fn admin_rules(&self) -> Result<RulesView, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_rules(&api).await;
        self.note_admin_traffic(&api, before);
        let rules = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.rules = Some(rules.clone());
        });
        Ok(RulesView::from(&rules))
    }

    /// `POST /api/archive/rules` — ONE field per decision, then the server's
    /// whole state back (rule 6: what the card shows is what the server holds).
    ///
    /// `field` is the server's own key (`on_fav_album`, `artist_scope`,
    /// `clean_min_free_gb`, …) and `value` its JSON, which is why this takes a
    /// string: an unknown key is refused by the server, not guessed at here.
    pub async fn admin_save_rule(&self, field: &str, value: &str) -> Result<RulesView, CoreError> {
        if !is_rule_field(field) {
            return Err(CoreError::Invalid {
                detail: format!("réglage d'archivage inconnu : {field}"),
            });
        }
        let value: serde_json::Value =
            serde_json::from_str(value).map_err(|error| CoreError::Invalid {
                detail: format!("{field} : {error}"),
            })?;
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::save_rules(&api, &serde_json::json!({ field: value })).await;
        self.note_admin_traffic(&api, before);
        let rules = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.rules = Some(rules.clone());
        });
        Ok(RulesView::from(&rules))
    }

    pub fn cached_admin_cleanup(&self) -> Option<CleanupView> {
        wire::Snapshot::read(self)
            .cleanup
            .as_ref()
            .map(CleanupView::from)
    }

    /// `GET /api/archive/cleanup/preview` — what a cleanup would delete right
    /// now, and how much room is missing. Deletes nothing.
    pub async fn admin_cleanup(&self) -> Result<CleanupView, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_cleanup(&api).await;
        self.note_admin_traffic(&api, before);
        let preview = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.cleanup = Some(preview.clone());
        });
        Ok(CleanupView::from(&preview))
    }

    /// `POST /api/archive/cleanup` — the ONLY function in this project that
    /// deletes archived audio, and only what the policy already wrote down.
    /// Answers how many files left and how many bytes came back.
    pub async fn admin_run_cleanup(&self) -> Result<CleanupRun, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::run_cleanup(&api).await;
        self.note_admin_traffic(&api, before);
        let answer = answer?;
        Ok(CleanupRun {
            deleted: answer
                .get("deleted")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default()
                .min(u64::from(u32::MAX)) as u32,
            freed: answer
                .get("freed")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default(),
        })
    }

    pub fn cached_admin_archive_status(&self) -> Option<ArchiveStatusView> {
        wire::Snapshot::read(self)
            .archive
            .as_ref()
            .map(ArchiveStatusView::from)
    }

    /// `GET /api/archive/status` — the backfill's gauge, and the poll a running
    /// pass makes.
    pub async fn admin_archive_status(&self) -> Result<ArchiveStatusView, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_archive_status(&api).await;
        self.note_admin_traffic(&api, before);
        let status = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.archive = Some(status.clone());
        });
        Ok(ArchiveStatusView::from(&status))
    }

    /// `POST /api/archive/backfill` with a scope from [`ARCHIVE_SCOPES`] — the
    /// safety net that sweeps up what the events missed. Only ever adds.
    pub async fn admin_start_archive(&self, scope: &str) -> Result<ArchiveStatusView, CoreError> {
        if !ARCHIVE_SCOPES.contains(&scope) {
            return Err(CoreError::Invalid {
                detail: format!("portée d'archivage inconnue : {scope}"),
            });
        }
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::start_archive(&api, scope).await;
        self.note_admin_traffic(&api, before);
        let status = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.archive = Some(status.clone());
        });
        Ok(ArchiveStatusView::from(&status))
    }

    pub fn cached_admin_migrate(&self) -> Option<MigrateView> {
        wire::Snapshot::read(self)
            .migrate
            .as_ref()
            .map(MigrateView::from)
    }

    /// `GET /api/archive/migrate` — the file-format migration's gauge.
    pub async fn admin_migrate(&self) -> Result<MigrateView, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_migrate(&api).await;
        self.note_admin_traffic(&api, before);
        let status = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.migrate = Some(status.clone());
        });
        Ok(MigrateView::from(&status))
    }

    /// `POST /api/archive/migrate` — rewrite the archived files in the current
    /// format. Safe to re-run: a title already migrated is left alone.
    pub async fn admin_start_migrate(&self, force: bool) -> Result<MigrateView, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::start_migrate(&api, force).await;
        self.note_admin_traffic(&api, before);
        let status = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.migrate = Some(status.clone());
        });
        Ok(MigrateView::from(&status))
    }

    // --- the analysis ------------------------------------------------------

    pub fn cached_admin_plan(&self) -> Option<PlanView> {
        wire::Snapshot::read(self).plan.as_ref().map(PlanView::from)
    }

    /// `GET /api/analysis/plan` — what is out of date, at which step, and the
    /// nightly window.
    pub async fn admin_plan(&self) -> Result<PlanView, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_plan(&api).await;
        self.note_admin_traffic(&api, before);
        let plan = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.plan = Some(plan.clone());
        });
        Ok(PlanView::from(&plan))
    }

    pub fn cached_admin_analysis(&self) -> Option<AnalysisJobView> {
        wire::Snapshot::read(self)
            .analysis
            .as_ref()
            .map(|job| AnalysisJobView::of(&job.job, job.paused.clone()))
    }

    /// `GET /api/analysis/backfill` — the measure-all job: its stage, how far it
    /// got and the CAUSE of every failure.
    pub async fn admin_analysis(&self) -> Result<AnalysisJobView, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_analysis(&api).await;
        self.note_admin_traffic(&api, before);
        let job = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.analysis = Some(job.clone());
        });
        Ok(AnalysisJobView::of(&job.job, job.paused))
    }

    /// `POST /api/analysis/backfill` — measure now. `workers` 0 means "size it
    /// from the machine", which is what the card's *auto* says.
    ///
    /// The server gates this job behind `workload.Throttle`: it pauses while
    /// anybody listens and resumes by itself (rule 1).
    pub async fn admin_start_analysis(
        &self,
        force: bool,
        workers: u32,
    ) -> Result<AnalysisJobView, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::start_analysis(&api, force, workers.min(64)).await;
        self.note_admin_traffic(&api, before);
        let job = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.analysis = Some(job.clone());
        });
        Ok(AnalysisJobView::of(&job.job, job.paused))
    }

    /// `POST /api/analysis/backfill/stop` — stop after the track in hand; the
    /// next run resumes from the checkpoint.
    pub async fn admin_stop_analysis(&self) -> Result<AnalysisJobView, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::stop_analysis(&api).await;
        self.note_admin_traffic(&api, before);
        let job = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.analysis = Some(job.clone());
        });
        Ok(AnalysisJobView::of(&job.job, job.paused))
    }

    /// `PUT /api/analysis/schedule` — the nightly window. `at` is `HH:MM` in the
    /// SERVER's local time; `hours` is how long it may run.
    pub async fn admin_save_schedule(
        &self,
        enabled: bool,
        at: &str,
        hours: u32,
    ) -> Result<PlanView, CoreError> {
        if !is_hh_mm(at) {
            return Err(CoreError::Invalid {
                detail: format!("heure attendue HH:MM, reçu « {at} »"),
            });
        }
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::save_schedule(&api, enabled, at, hours.clamp(1, 24)).await;
        self.note_admin_traffic(&api, before);
        let plan = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.plan = Some(plan.clone());
        });
        Ok(PlanView::from(&plan))
    }

    /// `PUT /api/analysis/where` — who measures the library: the `server` or
    /// `client` machines. The phone is not a measuring client (MB18 owns the
    /// studio's own worker), so the card offers the server's own choice.
    pub async fn admin_save_analysis_place(&self, place: &str) -> Result<PlanView, CoreError> {
        if place != "server" && place != "client" {
            return Err(CoreError::Invalid {
                detail: format!("lieu d'analyse inconnu : {place}"),
            });
        }
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::save_where(&api, place).await;
        self.note_admin_traffic(&api, before);
        let plan = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.plan = Some(plan.clone());
        });
        Ok(PlanView::from(&plan))
    }

    // --- the library-wide backfills ----------------------------------------

    pub fn cached_admin_enrich(&self) -> Option<EnrichView> {
        wire::Snapshot::read(self)
            .enrich
            .as_ref()
            .map(EnrichView::from)
    }

    /// `GET /api/enrich/backfill` — MusicBrainz's coverage of the archive.
    pub async fn admin_enrich(&self) -> Result<EnrichView, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_enrich(&api).await;
        self.note_admin_traffic(&api, before);
        let status = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.enrich = Some(status.clone());
        });
        Ok(EnrichView::from(&status))
    }

    /// `POST /api/enrich/backfill[?force=1]` — complete the facts of what is
    /// already archived. One request a second on the server's own side.
    pub async fn admin_start_enrich(&self, force: bool) -> Result<EnrichView, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::start_enrich(&api, force).await;
        self.note_admin_traffic(&api, before);
        let status = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.enrich = Some(status.clone());
        });
        Ok(EnrichView::from(&status))
    }

    pub fn cached_admin_credits(&self) -> Option<CreditsView> {
        wire::Snapshot::read(self)
            .credits
            .as_ref()
            .map(CreditsView::from)
    }

    /// `GET /api/credits/backfill` — who wrote, composed and produced each
    /// track, and how far the catch-up got.
    pub async fn admin_credits(&self) -> Result<CreditsView, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_credits(&api).await;
        self.note_admin_traffic(&api, before);
        let job = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.credits = Some(job.clone());
        });
        Ok(CreditsView::from(&job))
    }

    /// `POST /api/credits/backfill[?force=1]`.
    pub async fn admin_start_credits(&self, force: bool) -> Result<CreditsView, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::start_credits(&api, force).await;
        self.note_admin_traffic(&api, before);
        let job = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.credits = Some(job.clone());
        });
        Ok(CreditsView::from(&job))
    }

    pub fn cached_admin_similar(&self) -> Option<SimilarView> {
        wire::Snapshot::read(self)
            .similar
            .as_ref()
            .map(SimilarView::from)
    }

    /// `GET /api/similar/rebuild` — the fifty-closest index behind the radio,
    /// the Flow and the "sounds like" shelves.
    pub async fn admin_similar(&self) -> Result<SimilarView, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_similar(&api).await;
        self.note_admin_traffic(&api, before);
        let status = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.similar = Some(status.clone());
        });
        Ok(SimilarView::from(&status))
    }

    /// `POST /api/similar/rebuild[?force=1]`.
    pub async fn admin_start_similar(&self, force: bool) -> Result<SimilarView, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::start_similar(&api, force).await;
        self.note_admin_traffic(&api, before);
        let status = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.similar = Some(status.clone());
        });
        Ok(SimilarView::from(&status))
    }

    // --- availability (the audio auto-match) -------------------------------

    pub fn cached_admin_automatch(&self) -> Option<AutomatchView> {
        wire::Snapshot::read(self)
            .automatch
            .as_ref()
            .map(AutomatchView::from)
    }

    /// `GET /api/automatch` — the switch, the pass in flight, and what the last
    /// one replaced (with the score that justified it).
    pub async fn admin_automatch(&self) -> Result<AutomatchView, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_automatch(&api).await;
        self.note_admin_traffic(&api, before);
        let status = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.automatch = Some(status.clone());
        });
        Ok(AutomatchView::from(&status))
    }

    /// `PUT /api/automatch/settings` — the switch. Answers the whole status back,
    /// which is what the card then draws.
    pub async fn admin_set_automatch(&self, on: bool) -> Result<AutomatchView, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::save_automatch(&api, on).await;
        self.note_admin_traffic(&api, before);
        let status = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.automatch = Some(status.clone());
        });
        Ok(AutomatchView::from(&status))
    }

    /// `POST /api/automatch/run` — listen to every unavailable title once. It
    /// replaces WITHOUT asking, so the card always shows what it did.
    pub async fn admin_run_automatch(&self) -> Result<AutomatchView, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::run_automatch(&api).await;
        self.note_admin_traffic(&api, before);
        let status = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.automatch = Some(status.clone());
        });
        Ok(AutomatchView::from(&status))
    }

    /// `POST /api/automatch/cancel` — stop the pass in flight.
    pub async fn admin_cancel_automatch(&self) -> Result<AutomatchView, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::cancel_automatch(&api).await;
        self.note_admin_traffic(&api, before);
        let status = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.automatch = Some(status.clone());
        });
        Ok(AutomatchView::from(&status))
    }

    // --- transcription ------------------------------------------------------

    pub fn cached_admin_transcribe(&self) -> Option<TranscribeView> {
        wire::Snapshot::read(self)
            .transcribe
            .as_ref()
            .map(TranscribeView::from)
    }

    /// `GET /api/transcribe/settings[?probe=1]` — the engine, the model, the
    /// queue and the four numbers the card edits. `probe` is *Tester le moteur*:
    /// it runs the engine once, which is the only slow read here.
    pub async fn admin_transcribe(&self, probe: bool) -> Result<TranscribeView, CoreError> {
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_transcribe(&api, probe).await;
        self.note_admin_traffic(&api, before);
        let settings = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.transcribe = Some(settings.clone());
        });
        Ok(TranscribeView::from(&settings))
    }

    /// `PUT /api/transcribe/settings` — ONE field. `field` is the server's own
    /// key (`enabled`, `language`, `max_minutes`, `daily_minutes`,
    /// `search_segments`) and `value` its JSON.
    pub async fn admin_save_transcribe(
        &self,
        field: &str,
        value: &str,
    ) -> Result<TranscribeView, CoreError> {
        if !is_transcribe_field(field) {
            return Err(CoreError::Invalid {
                detail: format!("réglage de transcription inconnu : {field}"),
            });
        }
        let value: serde_json::Value =
            serde_json::from_str(value).map_err(|error| CoreError::Invalid {
                detail: format!("{field} : {error}"),
            })?;
        let api = self.admin_api()?;
        let before = wire::totals(&api);
        let answer = wire::save_transcribe(&api, &serde_json::json!({ field: value })).await;
        self.note_admin_traffic(&api, before);
        let settings = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.transcribe = Some(settings.clone());
        });
        Ok(TranscribeView::from(&settings))
    }

    // --- the one place a screen reads a job's causes -----------------------

    /// The failure ledger of a job, from its own counters: the CAUSE of each
    /// failure, its ranked tally, the sample of rows, and how many the server's
    /// bounded sample left out (`JobFailures.svelte`).
    ///
    /// It takes the payload a card already holds: no request, so a job that is
    /// running never costs one more poll to say why it failed.
    pub fn admin_job_failures(job: &nz_model::JobStatus) -> JobFailuresView {
        JobFailuresView::of(job)
    }
}

/// What `POST /api/archive/cleanup` did: how many files left and the bytes back.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CleanupRun {
    pub deleted: u32,
    pub freed: u64,
}

// --- the copy's small edits -------------------------------------------------------

/// The account's link to the app, built from the configured origin: the SPA's
/// own shape (`adminusers.js#inviteUrl`, `${origin}${pathname}#/invite/<token>`),
/// so a link can be opened in a browser or pasted into the app (MB2's link
/// parser reads `/app/#/invite/<code>`).
fn invite_url(core: &Core, token: &str) -> String {
    if token.is_empty() {
        return String::new();
    }
    let origin = core
        .origin()
        .map(|origin| origin.as_str())
        .or_else(|| core.server().map(|server| server.origin.as_str()))
        .unwrap_or_default();
    // `encodeURIComponent`'s contract: the token is opaque, so nothing in it may
    // change the URL's shape.
    let token = token
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
                c.to_string()
            } else {
                format!("%{:02X}", c as u32 & 0xFF)
            }
        })
        .collect::<String>();
    format!("{origin}/app/#/invite/{token}")
}

/// Applies the tap's decision to the device's copy before the request.
fn patch_copy_users(core: &Core, id: &str, patch: impl FnOnce(&mut AdminUser)) {
    wire::Snapshot::write(core, |snapshot| {
        let Some(page) = snapshot.users.as_mut() else {
            return;
        };
        if let Some(user) = page.users.iter_mut().find(|user| user.id.as_str() == id) {
            patch(user);
        }
    });
}

/// Puts the server's own row into the copy (`PATCH` answers the account).
fn replace_copy_user(core: &Core, user: &AdminUser) {
    wire::Snapshot::write(core, |snapshot| {
        if let Some(page) = snapshot.users.as_mut() {
            match page
                .users
                .iter_mut()
                .find(|row| row.id.as_str() == user.id.as_str())
            {
                Some(row) => *row = user.clone(),
                None => page.users.push(user.clone()),
            }
        }
    });
}

/// Takes a deleted account out of the copy, and its total with it.
fn drop_copy_user(core: &Core, id: &str) {
    wire::Snapshot::write(core, |snapshot| {
        if let Some(page) = snapshot.users.as_mut() {
            let before = page.users.len();
            page.users.retain(|user| user.id.as_str() != id);
            if page.users.len() != before {
                page.total = page.total.saturating_sub(1);
            }
        }
    });
}

/// A new invitation goes to the head of the copy: the list is newest first.
fn prepend_copy_invite(core: &Core, invite: &InviteView) {
    let row = nz_model::AdminInvite {
        id: invite.id.clone(),
        note: invite.note.clone(),
        created: invite.created.clone(),
        expires: invite.expires.clone(),
        state: invite.state.clone(),
        created_by: invite.created_by.clone(),
        used_by: invite.used_by.clone(),
        used_at: None,
    };
    wire::Snapshot::write(core, |snapshot| {
        // The card may not have been listed yet in this session: a minted
        // invitation still has to be in the copy the next open paints.
        let page = snapshot.invites.get_or_insert_with(Default::default);
        page.invites.retain(|other| other.id != row.id);
        page.invites.insert(0, row.clone());
    });
}

/// A revoked invitation leaves the copy.
fn drop_copy_invite(core: &Core, id: &str) {
    wire::Snapshot::write(core, |snapshot| {
        if let Some(page) = snapshot.invites.as_mut() {
            page.invites.retain(|invite| invite.id != id);
        }
    });
}

// --- the closed vocabularies -------------------------------------------------------

/// The fields `POST /api/archive/rules` accepts (`deezer/rules.py#KEYS`). The
/// screen may only send one of these, so a typo is refused HERE instead of
/// travelling to the server as a silent no-op.
fn is_rule_field(field: &str) -> bool {
    matches!(
        field,
        "on_fav_track"
            | "on_fav_album"
            | "on_fav_artist"
            | "on_fav_playlist"
            | "on_playlist_add"
            | "on_play_context"
            | "on_podcast"
            | "artist_scope"
            | "artist_limit"
            | "chapters"
            | "chapter_urls"
            | "clean_on"
            | "clean_stale_days"
            | "clean_min_free_gb"
            | "clean_order"
            | "clean_keep_fav"
            | "clean_keep_playlist"
            | "clean_keep_podcast"
    )
}

/// The fields `PUT /api/transcribe/settings` accepts (`webui/transcribe.py`).
fn is_transcribe_field(field: &str) -> bool {
    matches!(
        field,
        "enabled" | "language" | "max_minutes" | "daily_minutes" | "search_segments"
    )
}

/// `HH:MM`, 00..23 and 00..59 — the shape `PUT /api/analysis/schedule` takes.
fn is_hh_mm(text: &str) -> bool {
    let Some((hours, minutes)) = text.split_once(':') else {
        return false;
    };
    if hours.len() != 2 || minutes.len() != 2 {
        return false;
    }
    match (hours.parse::<u32>(), minutes.parse::<u32>()) {
        (Ok(hours), Ok(minutes)) => hours < 24 && minutes < 60,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::NetworkState;

    /// The number the acceptance is held to: *Utilisateurs* is the SPA's own two
    /// reads, a WARM open is ZERO, and the core's own meter says both.
    #[tokio::test]
    async fn a_cold_users_page_costs_two_and_a_warm_one_none() {
        let (server, _dir, core) = testing::configured().await;

        let page = core
            .admin_users("", None, USERS_PAGE_SIZE)
            .await
            .expect("the first page");
        let invites = core.admin_invites(Some("pending")).await.expect("invites");
        assert_eq!(
            server.routes(),
            vec!["GET /api/admin/users", "GET /api/admin/invites"],
            "one request per card, exactly the SPA's own two"
        );
        assert!(
            server.hits()[0].path.contains("limit=25"),
            "one page, 25 rows"
        );
        assert_eq!(page.users.len(), 2);
        assert_eq!(page.total, 3, "the count line says how many there are");
        assert_eq!(page.next.as_deref(), Some("u-2"), "the cursor continues");
        assert_eq!(invites.len(), 1);
        assert_eq!(invites[0].note.as_deref(), Some("Léa"));

        let counted = core
            .stats()
            .into_iter()
            .find(|row| row.tag == TAG_ADMIN)
            .expect("the tag is counted");
        assert_eq!(counted.requests, 2, "NzCore::stats() says what it cost");
        assert!(counted.bytes_in > 0);

        // --- warm: the device's copy paints, no request ----------------------
        server.forget();
        assert!(core.cached_admin_users().is_some());
        assert!(core.cached_admin_invites().is_some());
        assert_eq!(server.count(), 0);
    }

    /// A page is continued by its cursor and never restarts, and the server
    /// filters by name — the search is not a client-side scan of 25 rows.
    #[tokio::test]
    async fn the_second_page_continues_the_cursor_and_the_search_travels() {
        let (server, _dir, core) = testing::configured().await;
        core.admin_users("", None, USERS_PAGE_SIZE)
            .await
            .expect("one");
        core.admin_users("", Some("u-2"), USERS_PAGE_SIZE)
            .await
            .expect("two");
        core.admin_users("Léo", None, USERS_PAGE_SIZE)
            .await
            .expect("search");
        assert!(
            server.hits()[1].path.contains("after=u-2"),
            "{:?}",
            server.hits()
        );
        assert!(
            server.hits()[2].path.contains("q=L%C3%A9o"),
            "the query is encoded and goes to the server: {:?}",
            server.hits()
        );
    }

    /// Every mutation here is a real write with a real body, and the answer is
    /// what the card draws (rule 6).
    #[tokio::test]
    async fn the_two_account_switches_post_one_field_and_draw_the_servers_row() {
        let (server, _dir, core) = testing::configured().await;
        core.admin_users("", None, USERS_PAGE_SIZE)
            .await
            .expect("the page");

        let user = core.admin_set_role("u-2", true).await.expect("promoted");
        assert_eq!(
            server
                .last_body_of("PATCH", "/api/admin/users/u-2")
                .as_deref(),
            Some(r#"{"admin":true}"#),
            "one field, the SPA's own body"
        );
        assert!(user.admin, "the server's row, not the request");
        assert!(
            core.cached_admin_users()
                .expect("copy")
                .users
                .iter()
                .any(|row| row.id == "u-2" && row.admin),
            "a warm open shows the new role"
        );

        let user = core
            .admin_set_disabled("u-2", true)
            .await
            .expect("disabled");
        assert_eq!(
            server
                .last_body_of("PATCH", "/api/admin/users/u-2")
                .as_deref(),
            Some(r#"{"disabled":true}"#)
        );
        assert!(user.disabled);
    }

    /// A refusal puts the copy back and names the cause: the server's own code
    /// (`last_admin`) is what `AdminCopy.guardMessage` turns into its sentence.
    #[tokio::test]
    async fn a_refused_switch_rolls_the_copy_back_and_keeps_its_code() {
        let (server, _dir, core) = testing::configured().await;
        core.admin_users("", None, USERS_PAGE_SIZE)
            .await
            .expect("the page");
        server.break_code("/api/admin/users/u-2", 409, "last_admin");

        let error = core
            .admin_set_role("u-2", false)
            .await
            .expect_err("the last admin cannot lose the role");
        // A `409` without `Retry-After` is `nz-api`'s `Conflict`, and its detail is
        // the server's OWN French sentence — the one `guardMessage` shows. The
        // refusal's code (`last_admin`) is what the server used to choose that
        // sentence; the sentence is what a person reads.
        match &error {
            CoreError::Conflict { detail } => {
                assert!(detail.contains("dernier administrateur"), "{detail}");
            }
            other => panic!("a refusal, not {other:?}"),
        }
        let copy = core.cached_admin_users().expect("copy");
        let row = copy
            .users
            .iter()
            .find(|row| row.id == "u-2")
            .expect("the account is still there");
        assert!(!row.admin, "the optimistic tap was rolled back");
    }

    /// Deleting an account confirms its NAME (the server refuses anything else),
    /// removes it from the copy, and reports what was reassigned.
    #[tokio::test]
    async fn deleting_an_account_confirms_its_name_and_says_what_moved() {
        let (server, _dir, core) = testing::configured().await;
        core.admin_users("", None, USERS_PAGE_SIZE)
            .await
            .expect("the page");
        let moved = core.admin_delete_user("u-2", "Léo").await.expect("deleted");
        assert_eq!(
            server
                .last_body_of("DELETE", "/api/admin/users/u-2")
                .as_deref(),
            Some(r#"{"confirm":"Léo"}"#)
        );
        assert_eq!(moved.uploads_reassigned, 4);
        assert_eq!(moved.podcasts_reassigned, 1);
        assert!(
            !core
                .cached_admin_users()
                .expect("copy")
                .users
                .iter()
                .any(|row| row.id == "u-2"),
            "the gone account is gone from the copy too"
        );
    }

    /// An invitation's URL is the SPA's own link, built from the configured
    /// origin: a person can open it in a browser or paste it into the app.
    #[tokio::test]
    async fn an_invitation_link_is_the_spas_own_shape() {
        let (server, _dir, core) = testing::configured().await;
        let issued = core
            .admin_create_invite(Some("Léa"), 604_800)
            .await
            .expect("minted");
        assert_eq!(
            server.last_body_of("POST", "/api/admin/invites").as_deref(),
            Some(r#"{"expires_in":604800,"note":"Léa"}"#)
        );
        assert!(
            issued.url.ends_with("/app/#/invite/tok-abc123"),
            "{}",
            issued.url
        );
        assert!(
            issued.url.starts_with("http://127.0.0.1:"),
            "{}",
            issued.url
        );
        assert_eq!(issued.token, "tok-abc123");
        assert!(
            core.cached_admin_invites()
                .expect("copy")
                .iter()
                .any(|invite| invite.id == "i-new"),
            "the new row heads the copy"
        );

        core.admin_revoke_invite("i-new").await.expect("revoked");
        assert!(
            !core
                .cached_admin_invites()
                .expect("copy")
                .iter()
                .any(|invite| invite.id == "i-new"),
            "a revoked invitation leaves the copy"
        );
    }

    /// *Serveur*'s ten reads are the SPA's own, in the same order, and a warm
    /// open asks nothing.
    #[tokio::test]
    async fn a_cold_server_page_costs_the_spas_reads_and_a_warm_one_none() {
        let (server, _dir, core) = testing::configured().await;
        core.admin_server_card()
            .await
            .expect("the quota and the verdict");
        core.admin_storage().await.expect("the disk");
        core.admin_rules().await.expect("the rules");
        core.admin_cleanup().await.expect("the preview");
        core.admin_archive_status().await.expect("the archive");
        core.admin_migrate().await.expect("the migration");
        core.admin_enrich().await.expect("the facts");
        core.admin_credits().await.expect("the credits");
        core.admin_transcribe(false)
            .await
            .expect("the transcription");
        core.admin_plan().await.expect("the plan");
        core.admin_analysis().await.expect("the job");

        assert_eq!(
            server.routes(),
            vec![
                "GET /api/settings",
                "GET /api/deezer/status",
                "GET /api/storage",
                "GET /api/archive/rules",
                "GET /api/archive/cleanup/preview",
                "GET /api/archive/status",
                "GET /api/archive/migrate",
                "GET /api/enrich/backfill",
                "GET /api/credits/backfill",
                "GET /api/transcribe/settings",
                "GET /api/analysis/plan",
                "GET /api/analysis/backfill",
            ],
            "twelve reads: the SPA's ten plus the upkeep card's two"
        );
        let counted = core
            .stats()
            .into_iter()
            .find(|row| row.tag == TAG_ADMIN)
            .expect("counted");
        assert_eq!(counted.requests, 12);

        server.forget();
        assert!(core.cached_admin_server_card().is_some());
        assert!(core.cached_admin_storage().is_some());
        assert!(core.cached_admin_rules().is_some());
        assert!(core.cached_admin_cleanup().is_some());
        assert!(core.cached_admin_archive_status().is_some());
        assert!(core.cached_admin_migrate().is_some());
        assert!(core.cached_admin_enrich().is_some());
        assert!(core.cached_admin_credits().is_some());
        assert!(core.cached_admin_transcribe().is_some());
        assert!(core.cached_admin_plan().is_some());
        assert!(core.cached_admin_analysis().is_some());
        assert_eq!(server.count(), 0, "a warm open asks nothing");
    }

    /// The one destructive action of the app: the preview names the titles, and
    /// the run answers what it freed. Nothing else here can delete audio.
    #[tokio::test]
    async fn the_cleanup_previews_then_deletes_and_says_what_it_freed() {
        let (server, _dir, core) = testing::configured().await;
        let preview = core.admin_cleanup().await.expect("the preview");
        assert_eq!(preview.needed_bytes, 2_147_483_648, "2 GB missing");
        assert_eq!(preview.tracks.len(), 2);
        assert_eq!(preview.tracks[0].title, "Vieille prod");
        assert_eq!(preview.tracks[0].bytes, 12_000_000);

        let run = core.admin_run_cleanup().await.expect("the cleanup");
        assert_eq!(run.deleted, 2);
        assert_eq!(run.freed, 24_000_000);
        assert_eq!(
            server.routes(),
            vec![
                "GET /api/archive/cleanup/preview",
                "POST /api/archive/cleanup"
            ]
        );
    }

    /// A rule is saved ONE field at a time, and the card draws the server's whole
    /// state back — never the value it hoped would stick. A field the server does
    /// not know never travels.
    #[tokio::test]
    async fn one_rule_is_saved_at_a_time_and_an_unknown_one_never_travels() {
        let (server, _dir, core) = testing::configured().await;
        let rules = core
            .admin_save_rule("on_fav_album", "true")
            .await
            .expect("saved");
        assert_eq!(
            server.last_body_of("POST", "/api/archive/rules").as_deref(),
            Some(r#"{"on_fav_album":true}"#),
            "one field per decision"
        );
        assert!(rules.rules.clean_on, "the rest is the server's own state");

        server.forget();
        let refused = core
            .admin_save_rule("surprise", "1")
            .await
            .expect_err("unknown");
        assert!(matches!(refused, CoreError::Invalid { .. }), "{refused:?}");
        let refused = core
            .admin_save_rule("clean_min_free_gb", "beaucoup")
            .await
            .expect_err("not a number");
        assert!(matches!(refused, CoreError::Invalid { .. }), "{refused:?}");
        assert_eq!(server.count(), 0, "a typo costs no request");
    }

    /// The quota is one field on `/settings`, clamped to something the server
    /// would accept, and the card keeps the provider's verdict it already had.
    #[tokio::test]
    async fn the_upload_quota_is_one_field_and_keeps_the_verdict() {
        let (server, _dir, core) = testing::configured().await;
        core.admin_server_card().await.expect("the first read");
        let card = core.admin_save_quota(25.0).await.expect("saved");
        assert_eq!(
            server.last_body_of("POST", "/api/settings").as_deref(),
            Some(r#"{"upload_quota_gb":25.0}"#)
        );
        assert_eq!(card.upload_quota_gb, 25.0);
        assert_eq!(card.status_ok, Some(true), "the verdict did not vanish");
        assert_eq!(card.arl_hint.as_deref(), Some("…a1b2"));
    }

    /// *Tester* re-tests the credential instead of reading the server's cached
    /// verdict, which is what the card's button promises.
    #[tokio::test]
    async fn testing_the_account_forces_the_server_to_re_check_it() {
        let (server, _dir, core) = testing::configured().await;
        core.admin_server_card().await.expect("the first read");
        server.forget();
        let card = core.admin_test_deezer(true).await.expect("re-tested");
        assert_eq!(server.routes(), vec!["GET /api/deezer/status"]);
        assert!(server.hits()[0].path.contains("force=1"));
        assert_eq!(card.status_ok, Some(true));
    }

    /// A job's poll is ONE read, the start returns at once, and every failure
    /// carries its cause — the numbers a person needs (rule 2).
    #[tokio::test]
    async fn a_job_starts_returns_at_once_and_its_failures_carry_their_cause() {
        let (server, _dir, core) = testing::configured().await;
        let job = core.admin_start_analysis(true, 0).await.expect("started");
        assert!(job.running);
        assert_eq!(
            server.routes(),
            vec!["POST /api/analysis/backfill", "GET /api/analysis/backfill"],
            "the start, then the state the card draws"
        );
        assert_eq!(
            server
                .last_body_of("POST", "/api/analysis/backfill")
                .as_deref(),
            Some(r#"{"force":true,"workers":0}"#)
        );
        assert_eq!(job.failures.failed, 2);
        assert_eq!(job.failures.causes[0].reason, "ffmpeg a disparu");
        assert_eq!(job.failures.rows[0].track, "a.flac");
        assert_eq!(job.failures.hidden, 1, "the sample is not the whole story");
        assert_eq!(
            job.paused.as_deref(),
            Some("listening"),
            "the server's own word for why it waits"
        );

        let stopped = core.admin_stop_analysis().await.expect("stopped");
        assert!(!stopped.running);
    }

    /// The analysis job's `paused` is a WORD on the wire and a boolean in the
    /// shared model: a pause must not make the whole payload unreadable.
    #[tokio::test]
    async fn a_paused_analysis_job_still_parses() {
        let (server, _dir, core) = testing::configured().await;
        let job = core.admin_analysis().await.expect("the job");
        assert_eq!(job.paused.as_deref(), Some("listening"));
        assert!(job.running);
        assert_eq!(server.routes(), vec!["GET /api/analysis/backfill"]);
    }

    /// One backfill per kind: the start answers the job's own state, and the
    /// screen polls only while it says `running`.
    #[tokio::test]
    async fn the_three_backfills_start_and_draw_their_own_state() {
        let (server, _dir, core) = testing::configured().await;
        let credits = core.admin_start_credits(true).await.expect("credits");
        assert!(credits.running);
        assert_eq!(credits.sources[0].name, "deezer");
        let enrich = core.admin_start_enrich(false).await.expect("facts");
        assert!(enrich.enabled);
        assert_eq!(enrich.found[0].count, 9);
        let similar = core.admin_start_similar(false).await.expect("neighbours");
        assert!(similar.running);
        assert_eq!(
            server.routes(),
            vec![
                "POST /api/credits/backfill",
                "GET /api/credits/backfill",
                "POST /api/enrich/backfill",
                "GET /api/enrich/backfill",
                "POST /api/similar/rebuild",
                "GET /api/similar/rebuild",
            ]
        );
        assert!(server.hits()[0].path.contains("force=1"));
        assert!(!server.hits()[2].path.contains("force=1"));
    }

    /// The auto-match replaces titles WITHOUT asking, so its card must show what
    /// it replaced and why a title was left alone.
    #[tokio::test]
    async fn the_automatch_switch_and_its_replacements_are_drawn() {
        let (server, _dir, core) = testing::configured().await;
        let status = core.admin_automatch().await.expect("the switch");
        assert!(status.available);
        assert!(status.enabled);
        assert!(!status.running);
        assert_eq!(status.recent.len(), 1);
        assert_eq!(status.recent[0].from_title, "Perdu");
        assert_eq!(status.recent[0].to_source, "youtube");
        assert_eq!(status.misses[0].status, "no_candidate");

        let status = core.admin_set_automatch(false).await.expect("off");
        assert!(!status.enabled);
        assert_eq!(
            server
                .last_body_of("PUT", "/api/automatch/settings")
                .as_deref(),
            Some(r#"{"enabled":false}"#)
        );
        let status = core.admin_run_automatch().await.expect("a pass");
        assert!(status.running);
        let status = core.admin_cancel_automatch().await.expect("stopped");
        assert!(status.cancel);
    }

    /// The transcription card: one field per decision, the engine's verdict read
    /// back, and *Tester* running the engine once.
    #[tokio::test]
    async fn transcription_saves_one_field_and_probes_on_demand() {
        let (server, _dir, core) = testing::configured().await;
        let view = core.admin_transcribe(false).await.expect("read");
        assert_eq!(view.max_minutes, 180);
        assert_eq!(view.model_size_mb, 466);

        let view = core
            .admin_save_transcribe("max_minutes", "30")
            .await
            .expect("saved");
        assert_eq!(
            server
                .last_body_of("PUT", "/api/transcribe/settings")
                .as_deref(),
            Some(r#"{"max_minutes":30}"#)
        );
        assert_eq!(view.max_minutes, 30);

        server.forget();
        let refused = core
            .admin_save_transcribe("threads", "8")
            .await
            .expect_err("not a settings field");
        assert!(matches!(refused, CoreError::Invalid { .. }), "{refused:?}");
        assert_eq!(server.count(), 0);

        core.admin_transcribe(true).await.expect("probed");
        assert!(server.hits()[0].path.contains("probe=1"));
    }

    /// Off a network the gate answers before a socket is opened, and every card
    /// keeps what the person already saw (rule 3).
    #[tokio::test]
    async fn a_dead_link_keeps_every_card_and_opens_no_socket() {
        let (_server, _dir, core) = testing::configured().await;
        core.admin_storage().await.expect("the disk");
        core.admin_rules().await.expect("the rules");

        let server = _server;
        server.forget();
        core.set_network(NetworkState::None);
        assert_eq!(
            core.admin_storage().await.expect_err("offline"),
            CoreError::Offline
        );
        assert_eq!(
            core.admin_rules().await.expect_err("offline"),
            CoreError::Offline
        );
        assert_eq!(server.count(), 0, "not one socket while offline");
        assert!(core.cached_admin_storage().is_some());
        assert!(core.cached_admin_rules().is_some());
    }

    /// The vocabularies this screen may send are closed: a typo is refused here,
    /// and the schedule's `HH:MM` too.
    #[test]
    fn the_screens_vocabularies_are_closed() {
        assert!(is_rule_field("clean_keep_podcast"));
        assert!(!is_rule_field("clean_keep_everything"));
        assert!(is_transcribe_field("search_segments"));
        assert!(!is_transcribe_field("model"));
        assert!(is_hh_mm("03:00"));
        assert!(is_hh_mm("23:59"));
        assert!(!is_hh_mm("24:00"));
        assert!(!is_hh_mm("3:00"));
        assert!(!is_hh_mm("03:60"));
        assert!(!is_hh_mm(""));
    }
}
