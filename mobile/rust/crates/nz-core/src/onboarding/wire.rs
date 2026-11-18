//! The onboarding's and the invitation's wire: the routes, the one snapshot the
//! device keeps between two visits, the invitation draft, and the shapes a poll
//! reads.
//!
//! Every call goes through `nz-api`'s `Lane::User` — the action just taken, never
//! behind a prefetch or a download (rule 1). Nothing here decides a rule of its
//! own: the forms are [`super::rules`]'s, the job's phases are the server's, and
//! the counting is `nz-api`'s per-tag counters read around one screen open.
//!
//! # The states a screen must never confuse
//!
//! - **A `404` on `/onboarding` is a server older than the feature** (rule 7) and
//!   crosses as `Unsupported`; a `404` on `/invites/inspect` is a link that is
//!   gone and crosses as `valid: false`, an ANSWER, because the page's "plus
//!   valide" state has no « Réessayer ».
//! - **A redeemed invitation's refusals are answers too**: the server's `400`
//!   names the field (`username`, `password`, `password_confirm` through its
//!   code), so the sentence goes UNDER that field instead of into a toast.
//! - **A provider's failure is never a verdict**: the account step paints every
//!   card from `/onboarding/account/options` even with Deezer switched off, and
//!   the import's unmatched lines are listed with their own titles.

use std::path::PathBuf;

use nz_api::{Api, ApiError, Lane, Method};
use nz_model::{Me, User};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    AccountOptions, AccountRun, ArtistCard, ArtistPage, ArtistSearch, ArtistsDone, Core, CoreError,
    Diversity, Feed, ImportItem, ImportJob, ImportOffer, ImportState, InviteField, InviteInfo,
    LIST_TTL_SECS, LinkedAccount, OnboardingData, OnboardingState, ProviderOptions, RelatedAnswer,
    RelatedState, StartedImport, Step,
};

/// What one request cost, as `nz-api` counts it (SC3): requests really sent, and
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
        .find(|row| row.tag == api.tag())
        .map(|row| Totals {
            requests: row.requests,
            bytes_in: row.bytes_in,
            bytes_out: row.bytes_out,
        })
        .unwrap_or_default()
}

/// `Meter::record` counts ONE request and its bytes; the whole run's bytes go
/// with the first row and the rest are zeros (MB6's measurement).
pub(super) fn note_traffic(core: &Core, tag: &'static str, api: &Api, before: Totals) {
    let after = totals(api);
    let requests = after.requests.saturating_sub(before.requests);
    if requests == 0 {
        return;
    }
    core.meter().record(
        tag,
        after.bytes_in.saturating_sub(before.bytes_in),
        after.bytes_out.saturating_sub(before.bytes_out),
    );
    for _ in 1..requests {
        core.meter().record(tag, 0, 0);
    }
}

/// `encodeURIComponent`, the SPA's own encoding for a path segment or a query
/// value (nz-api keeps its copy private to the crate).
pub(super) fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn string(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn opt_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn number(value: &Value, key: &str) -> u32 {
    value
        .get(key)
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(u32::MAX as u64) as u32
}

fn flag(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// `onboarding.js#idOf`: the reference a tile is known by.
fn artist_from(value: &Value) -> Option<ArtistCard> {
    let name = string(value, "name");
    if name.is_empty() {
        return None;
    }
    let id = {
        let deezer = string(value, "deezer_id");
        if deezer.is_empty() {
            string(value, "id")
        } else {
            deezer
        }
    };
    if id.is_empty() {
        return None;
    }
    Some(ArtistCard {
        id: id.clone(),
        deezer_id: id,
        name,
        picture: opt_string(value, "picture"),
        picture_small: opt_string(value, "picture_small"),
        genres: array(value, "genres")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        provider: string(value, "provider"),
        via: opt_string(value, "via"),
        parent: opt_string(value, "parent"),
        liked: flag(value, "liked"),
        openable: value
            .get("openable")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        hint: opt_string(value, "hint"),
    })
}

fn artists_from(value: &Value, key: &str) -> Vec<ArtistCard> {
    array(value, key).iter().filter_map(artist_from).collect()
}

fn state_from(value: &Value) -> OnboardingState {
    let feed = value.get("feed").cloned().unwrap_or(Value::Null);
    let diversity = value.get("diversity").cloned().unwrap_or(Value::Null);
    OnboardingState {
        step: Step::parse(&string(value, "step")),
        liked: number(value, "liked"),
        min_artists: number(value, "min_artists").max(1),
        feed: Feed {
            state: string(&feed, "state"),
            building: flag(&feed, "building"),
            error: opt_string(&feed, "error"),
            source: string(&feed, "source"),
            ready: flag(&feed, "ready"),
            stale: flag(&feed, "stale"),
            progress: feed.get("progress").and_then(Value::as_f64),
        },
        diversity: Diversity {
            window: number(&diversity, "window").max(1) as usize,
            max_same_genre: number(&diversity, "max_same_genre").max(1) as usize,
            max_streak: number(&diversity, "max_streak").max(1) as usize,
        },
    }
}

fn page_from(value: &Value) -> ArtistPage {
    ArtistPage {
        items: artists_from(value, "items"),
        next: opt_string(value, "next"),
        total: number(value, "total"),
        state: string(value, "state"),
        source: string(value, "source"),
        building: flag(value, "building"),
        error: opt_string(value, "error"),
        partial: flag(value, "partial"),
    }
}

fn related_from(value: &Value) -> RelatedAnswer {
    let state = match string(value, "state").as_str() {
        "pending" => RelatedState::Pending,
        "ready" => RelatedState::Ready,
        "error" => RelatedState::Error,
        _ => RelatedState::None,
    };
    RelatedAnswer {
        state,
        items: artists_from(value, "items"),
        cause: opt_string(value, "error"),
    }
}

fn job_from(value: &Value) -> ImportJob {
    let state = ImportState::parse(&string(value, "state"));
    let items = array(value, "items")
        .iter()
        .map(|item| {
            let line = item.get("line").cloned().unwrap_or(Value::Null);
            let track = item.get("track").cloned().unwrap_or(Value::Null);
            ImportItem {
                title: string(&line, "title"),
                artists: array(&line, "artists")
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", "),
                album: string(&line, "album"),
                duration_s: number(&line, "duration"),
                matched: !track.is_null(),
                via: opt_string(item, "via"),
                sure: flag(item, "sure"),
            }
        })
        .collect();
    ImportJob {
        id: string(value, "id"),
        state,
        source: string(value, "source"),
        name: string(value, "name"),
        total: number(value, "total"),
        done: number(value, "done"),
        matched: number(value, "matched"),
        truncated: flag(value, "truncated"),
        error: opt_string(value, "error"),
        items,
    }
}

fn running_from(value: &Value) -> Option<AccountRun> {
    let running = value.get("running")?.as_object()?;
    if running.get("running").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let running = Value::Object(running.clone());
    Some(AccountRun {
        phase: string(&running, "phase"),
        done: number(&running, "done"),
        total: running
            .get("total")
            .and_then(Value::as_u64)
            .map(|total| total.min(u32::MAX as u64) as u32),
    })
}

fn account_from(value: &Value) -> Option<LinkedAccount> {
    // `_account_entry` answers an object; an older server may answer a label.
    let account = value.as_object()?;
    let account = Value::Object(account.clone());
    let last = account.get("last").cloned().unwrap_or(Value::Null);
    let added = last.get("added").cloned().unwrap_or(Value::Null);
    Some(LinkedAccount {
        id: string(&account, "id"),
        label: string(&account, "label"),
        mode: string(&account, "mode"),
        status: string(&account, "status"),
        server: flag(&account, "server") || string(&account, "mode") == "server",
        added_artists: number(&added, "artists"),
        added_tracks: number(&added, "tracks"),
        last_reason: opt_string(&last, "reason"),
        running: running_from(&account),
    })
}

fn provider_from(value: Option<&Value>) -> ProviderOptions {
    let Some(value) = value else {
        return ProviderOptions::default();
    };
    ProviderOptions {
        available: flag(value, "available"),
        can_link: flag(value, "can_link"),
        linked: flag(value, "linked"),
        denied: flag(value, "denied"),
        reason: opt_string(value, "reason"),
        account: value.get("account").and_then(account_from),
    }
}

fn options_from(value: &Value) -> AccountOptions {
    let imports = array(value, "imports")
        .iter()
        .map(|entry| ImportOffer {
            source: string(entry, "source"),
            label: string(entry, "label"),
            available: flag(entry, "available"),
            reason: opt_string(entry, "reason"),
        })
        .collect();
    AccountOptions {
        step: Step::parse(&string(value, "step")),
        deezer: provider_from(value.get("deezer")),
        youtube: provider_from(value.get("youtube")),
        imports,
        can_save_imports: flag(value, "can_save_imports"),
        job: value.get("job").filter(|job| !job.is_null()).map(job_from),
    }
}

/// A route that a server older than the feature does not have (rule 7) is
/// `Unsupported`, so the screen hides the entry once instead of failing forever.
fn onboarding_error(error: ApiError, feature: &str) -> CoreError {
    match error {
        ApiError::Http { status: 404, .. } => CoreError::Unsupported {
            feature: feature.to_owned(),
        },
        other => other.into(),
    }
}

// -- the invitation -----------------------------------------------------------

/// Why redeeming failed, before it becomes a `CoreError`: the first two are the
/// FORM's answers, and only the third is a failure.
pub(super) enum RedeemFailure {
    Field { field: InviteField, message: String },
    Invalid,
    Other(CoreError),
}

/// `POST /api/invites/inspect`. A `404` is `valid: false`; anything else is the
/// cause the page prints with « Réessayer ».
pub(super) async fn fetch_invite_inspect(api: &Api, code: &str) -> Result<InviteInfo, CoreError> {
    let body = serde_json::json!({ "token": code });
    match api
        .send::<Value, Value>(Lane::User, Method::POST, "/invites/inspect", Some(&body))
        .await
    {
        Ok(Some(answer)) => Ok(InviteInfo {
            valid: answer
                .get("valid")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            label: string(&answer, "label"),
            expires: opt_string(&answer, "expires"),
            draft_name: None,
        }),
        Ok(None) => Ok(InviteInfo::default()),
        Err(ApiError::Http { status: 404, .. }) => Ok(InviteInfo::default()),
        Err(error) => Err(error.into()),
    }
}

/// `POST /api/invites/redeem`: the form's three answers, or the cause.
pub(super) async fn post_redeem(
    api: &Api,
    code: &str,
    username: &str,
    password: &str,
    request_id: &str,
) -> Result<Me, RedeemFailure> {
    let body = serde_json::json!({
        "token": code,
        "username": username,
        "password": password,
        "password_confirm": password,
        "request_id": request_id,
    });
    match api
        .send::<Value, Value>(Lane::User, Method::POST, "/invites/redeem", Some(&body))
        .await
    {
        Ok(Some(answer)) => {
            let user = answer.get("user").cloned().unwrap_or(Value::Null);
            let user: User = serde_json::from_value(user).map_err(|error| {
                RedeemFailure::Other(CoreError::Internal {
                    detail: format!("invitation : la réponse n'a pas de personne ({error})"),
                })
            })?;
            Ok(Me { user, remote: None })
        }
        Ok(None) => Err(RedeemFailure::Other(CoreError::Internal {
            detail: "invitation : le serveur n'a pas répondu le compte créé".to_owned(),
        })),
        Err(ApiError::Http { status: 404, .. }) => Err(RedeemFailure::Invalid),
        Err(ApiError::Http {
            status: 400,
            message,
            code,
        }) => {
            let field = match code.as_deref().unwrap_or_default() {
                "password_weak" => InviteField::Password,
                "password_mismatch" => InviteField::Confirm,
                _ => InviteField::Username,
            };
            Err(RedeemFailure::Field { field, message })
        }
        Err(error) => Err(RedeemFailure::Other(error.into())),
    }
}

// -- the artists step ---------------------------------------------------------

pub(super) async fn fetch_state(api: &Api) -> Result<OnboardingState, CoreError> {
    let value = api
        .get::<Value>(Lane::User, "/onboarding")
        .await
        .map_err(|error| onboarding_error(error, "onboarding"))?
        .value;
    Ok(state_from(&value))
}

pub(super) async fn fetch_page(
    api: &Api,
    after: Option<&str>,
    limit: u32,
) -> Result<ArtistPage, CoreError> {
    let mut path = format!("/onboarding/artists?limit={limit}");
    if let Some(after) = after.filter(|after| !after.is_empty()) {
        path.push_str(&format!("&after={}", encode(after)));
    }
    let value = api
        .get::<Value>(Lane::User, &path)
        .await
        .map_err(|error| onboarding_error(error, "onboarding"))?
        .value;
    Ok(page_from(&value))
}

pub(super) async fn fetch_search(
    api: &Api,
    query: &str,
    limit: u32,
) -> Result<ArtistSearch, CoreError> {
    let path = format!("/onboarding/search?q={}&limit={limit}", encode(query));
    let value = api
        .get::<Value>(Lane::User, &path)
        .await
        .map_err(|error| onboarding_error(error, "onboarding"))?
        .value;
    Ok(ArtistSearch {
        query: string(&value, "query"),
        items: artists_from(&value, "items"),
        local: number(&value, "local"),
        partial: flag(&value, "partial"),
        error: opt_string(&value, "error"),
    })
}

pub(super) async fn post_like(
    api: &Api,
    artist: &str,
    seen: &[String],
) -> Result<RelatedAnswer, CoreError> {
    let mut body = serde_json::json!({ "deezer_id": artist });
    if !seen.is_empty() {
        body["seen"] = Value::from(seen.iter().take(200).cloned().collect::<Vec<_>>());
    }
    let answer = api
        .send::<Value, Value>(
            Lane::User,
            Method::POST,
            "/onboarding/artists/like",
            Some(&body),
        )
        .await?;
    let related = answer
        .as_ref()
        .and_then(|answer| answer.get("related").cloned())
        .unwrap_or(Value::Null);
    Ok(related_from(&related))
}

pub(super) async fn delete_like(api: &Api, artist: &str) -> Result<(), CoreError> {
    let path = format!("/onboarding/artists/{}/like", encode(artist));
    api.send::<Value, Value>(Lane::User, Method::DELETE, &path, None)
        .await?;
    Ok(())
}

pub(super) async fn fetch_related(
    api: &Api,
    artist: &str,
    seen: &[String],
) -> Result<RelatedAnswer, CoreError> {
    let mut path = format!("/onboarding/artists/{}/related", encode(artist));
    if !seen.is_empty() {
        let joined: Vec<&str> = seen.iter().take(200).map(String::as_str).collect();
        path.push_str(&format!("?seen={}", encode(&joined.join(","))));
    }
    let value = api
        .get::<Value>(Lane::User, &path)
        .await
        .map_err(|error| onboarding_error(error, "onboarding"))?
        .value;
    Ok(related_from(&value))
}

pub(super) async fn post_artists_done(api: &Api, skip: bool) -> Result<ArtistsDone, CoreError> {
    let body = if skip {
        serde_json::json!({ "skip": true })
    } else {
        serde_json::json!({})
    };
    let answer = api
        .send::<Value, Value>(
            Lane::User,
            Method::POST,
            "/onboarding/artists/done",
            Some(&body),
        )
        .await?;
    let answer = answer.unwrap_or(Value::Null);
    Ok(ArtistsDone {
        step: Step::parse(&string(&answer, "step")),
        skipped: flag(&answer, "skipped"),
    })
}

pub(super) async fn post_feed_retry(api: &Api) -> Result<(), CoreError> {
    api.send::<Value, Value>(
        Lane::User,
        Method::POST,
        "/onboarding/feed/retry",
        Some(&serde_json::json!({})),
    )
    .await?;
    Ok(())
}

pub(super) async fn post_account_done(api: &Api) -> Result<Step, CoreError> {
    let answer = api
        .send::<Value, Value>(
            Lane::User,
            Method::POST,
            "/onboarding/account/done",
            Some(&serde_json::json!({})),
        )
        .await?
        .unwrap_or(Value::Null);
    Ok(Step::parse(&string(&answer, "step")))
}

// -- the account step ---------------------------------------------------------

pub(super) async fn fetch_account_options(api: &Api) -> Result<AccountOptions, CoreError> {
    let value = api
        .get::<Value>(Lane::User, "/onboarding/account/options")
        .await
        .map_err(|error| onboarding_error(error, "onboarding"))?
        .value;
    Ok(options_from(&value))
}

pub(super) async fn post_account_link(
    api: &Api,
    provider: &str,
    secret: &str,
) -> Result<AccountOptions, CoreError> {
    let body = if provider == "deezer" {
        serde_json::json!({ "provider": "deezer", "arl": secret })
    } else {
        serde_json::json!({ "provider": "youtube", "headers": secret })
    };
    let answer = api
        .send::<Value, Value>(
            Lane::User,
            Method::POST,
            "/onboarding/account/link",
            Some(&body),
        )
        .await?;
    let options = answer
        .as_ref()
        .and_then(|answer| answer.get("options").cloned())
        .unwrap_or(Value::Null);
    Ok(options_from(&options))
}

pub(super) async fn post_import(api: &Api, url: &str) -> Result<StartedImport, CoreError> {
    let body = serde_json::json!({ "url": url });
    let answer = api
        .send::<Value, Value>(
            Lane::User,
            Method::POST,
            "/onboarding/account/import",
            Some(&body),
        )
        .await?
        .unwrap_or(Value::Null);
    let job = answer
        .get("job")
        .cloned()
        .filter(|job| !job.is_null())
        .unwrap_or_else(|| serde_json::json!({ "id": string(&answer, "id"), "state": "reading" }));
    let options = answer.get("options").cloned().unwrap_or(Value::Null);
    Ok(StartedImport {
        job: job_from(&job),
        options: options_from(&options),
    })
}

pub(super) async fn fetch_import(api: &Api, id: &str) -> Result<ImportJob, CoreError> {
    if id.is_empty() {
        return Err(CoreError::Invalid {
            detail: "aucun import à suivre".to_owned(),
        });
    }
    let path = format!("/import/playlist/{}", encode(id));
    match api.get::<Value>(Lane::User, &path).await {
        Ok(fresh) => Ok(job_from(&fresh.value)),
        Err(ApiError::Http { status: 404, .. }) => Err(CoreError::Invalid {
            detail: "l'analyse a expiré".to_owned(),
        }),
        Err(error) => Err(error.into()),
    }
}

pub(super) async fn post_import_save(
    api: &Api,
    id: &str,
    title: &str,
) -> Result<ImportJob, CoreError> {
    if id.is_empty() {
        return Err(CoreError::Invalid {
            detail: "aucun import à enregistrer".to_owned(),
        });
    }
    let path = format!("/import/playlist/{}/save", encode(id));
    let body = serde_json::json!({ "title": title, "keep": Value::Null });
    let answer = api
        .send::<Value, Value>(Lane::User, Method::POST, &path, Some(&body))
        .await?
        .unwrap_or(Value::Null);
    if answer.get("id").is_none() {
        // The save answers the job it moved; a server that answers the playlist
        // itself still leaves the job readable, so re-read it once.
        return fetch_import(api, id).await;
    }
    Ok(job_from(&answer))
}

// -- what the device keeps ----------------------------------------------------

/// The snapshot the artists step paints before a socket opens.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Snapshot {
    /// Unix seconds: `onboarding.js#CACHE_TTL`'s thirty minutes.
    at: u64,
    data: OnboardingData,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// `onboarding-<account hash>.json`, in the CACHE directory (evictable, like
/// every cache): two accounts never read each other's onboarding, and the name is
/// a hash, never a server string.
fn snapshot_path(core: &Core) -> PathBuf {
    core.cache_path(&format!(
        "onboarding-{:016x}.json",
        super::fnv1a(super::owner(core).as_bytes())
    ))
}

pub(super) fn read_snapshot(core: &Core) -> Option<OnboardingData> {
    read_snapshot_at(core, now_secs())
}

pub(super) fn read_snapshot_at(core: &Core, now: u64) -> Option<OnboardingData> {
    let body = std::fs::read_to_string(snapshot_path(core)).ok()?;
    let snapshot: Snapshot = serde_json::from_str(&body).ok()?;
    // A stale copy is not drawn at all: the SPA's sessionStorage entry expires,
    // and a step from half an hour ago could offer a form the server has already
    // left.
    (now.saturating_sub(snapshot.at) <= LIST_TTL_SECS).then_some(snapshot.data)
}

pub(super) fn write_snapshot(core: &Core, data: &OnboardingData) -> Result<(), CoreError> {
    let path = snapshot_path(core);
    let snapshot = Snapshot {
        at: now_secs(),
        data: data.clone(),
    };
    let body = serde_json::to_vec(&snapshot).map_err(|error| CoreError::Internal {
        detail: format!("{} : {error}", path.display()),
    })?;
    let temporary = path.with_extension("part");
    std::fs::write(&temporary, &body).map_err(|error| CoreError::Internal {
        detail: format!("{} : {error}", temporary.display()),
    })?;
    std::fs::rename(&temporary, &path).map_err(|error| CoreError::Internal {
        detail: format!("{} : {error}", path.display()),
    })
}

// -- the invitation draft -----------------------------------------------------

/// `nsInvite.draft`: the token, the name and the request id — NEVER the
/// password (`invite.js#rememberDraft`'s own test).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteDraft {
    /// Unix seconds when it was written.
    pub at: u64,
    pub token: String,
    pub name: String,
    pub request_id: String,
}

fn draft_key(token: &str) -> String {
    format!("invite.draft.{:016x}", super::fnv1a(token.as_bytes()))
}

pub(super) fn read_draft(core: &Core, token: &str) -> Option<InviteDraft> {
    read_draft_at(core, token, now_secs())
}

/// The draft's reader with the clock handed in, so a test proves the ten-minute
/// life without waiting on it.
pub(super) fn read_draft_at(core: &Core, token: &str, now: u64) -> Option<InviteDraft> {
    if token.is_empty() {
        return None;
    }
    let draft: InviteDraft = core.cache().kv().get(&draft_key(token))?;
    if draft.request_id.is_empty() || draft.token != token {
        return None;
    }
    (now.saturating_sub(draft.at) <= super::rules::DRAFT_TTL_SECS).then_some(draft)
}

pub(super) fn write_draft(core: &Core, token: &str, name: &str, request_id: &str) {
    if token.is_empty() || request_id.is_empty() {
        return;
    }
    let draft = InviteDraft {
        at: now_secs(),
        token: token.to_owned(),
        name: name.to_owned(),
        request_id: request_id.to_owned(),
    };
    if let Err(cause) = core.cache().kv().set(&draft_key(token), &draft) {
        tracing::debug!(%cause, "the invitation draft was not kept");
    }
}

pub(super) fn clear_draft(core: &Core, token: &str) {
    if let Err(cause) = core.cache().kv().delete(&draft_key(token)) {
        tracing::debug!(%cause, "the invitation draft was not cleared");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tile_reads_the_reference_the_like_button_sends() {
        let card = artist_from(&serde_json::json!({
            "deezer_id": "42",
            "id": "42",
            "name": "Aya Nakamura",
            "genres": ["Pop"],
            "via": "related",
            "parent": "7",
            "liked": true,
            "openable": true
        }))
        .expect("a tile");
        assert_eq!(card.key(), "42");
        assert_eq!(card.via.as_deref(), Some("related"));
        assert_eq!(card.parent.as_deref(), Some("7"));
        assert!(card.liked);
        assert_eq!(card.genres, ["Pop"]);
    }

    #[test]
    fn an_unknown_step_is_done_never_an_unfinished_flow() {
        assert_eq!(state_from(&serde_json::json!({})).step, Step::Done);
        assert_eq!(
            state_from(&serde_json::json!({ "step": "n'importe quoi" })).step,
            Step::Done
        );
        assert_eq!(
            state_from(&serde_json::json!({ "step": "artists", "liked": 3 })).step,
            Step::Artists
        );
    }

    #[test]
    fn an_import_job_carries_the_cause_of_each_line() {
        let job = job_from(&serde_json::json!({
            "id": "j1",
            "state": "done",
            "total": 3,
            "done": 3,
            "matched": 1,
            "error": "Deezer ne répondait pas",
            "items": [
                { "line": { "title": "A", "artists": ["X", "Y"] }, "track": { "id": "1" } },
                { "line": { "title": "B", "artists": ["Z"] }, "track": null },
                { "line": { "title": "C", "artists": [] }, "track": null }
            ]
        }));
        assert_eq!(job.state, ImportState::Done);
        assert_eq!(job.items.len(), 3);
        assert_eq!(job.items[0].artists, "X, Y");
        assert!(job.items[0].matched);
        assert_eq!(job.unmatched_count(), 2);
        assert_eq!(
            job.unmatched()
                .iter()
                .map(|item| item.title.as_str())
                .collect::<Vec<_>>(),
            ["B", "C"]
        );
        assert_eq!(job.error.as_deref(), Some("Deezer ne répondait pas"));
    }

    #[test]
    fn an_accounts_running_import_is_read_from_its_own_shape() {
        let account = account_from(&serde_json::json!({
            "id": "7",
            "label": "Deezer",
            "mode": "arl",
            "status": "ok",
            "running": { "running": true, "phase": "import", "done": 4, "total": 10 },
            "last": { "added": { "artists": 42, "tracks": 118 } }
        }))
        .expect("an account");
        assert_eq!(account.added_artists, 42);
        assert_eq!(account.added_tracks, 118);
        let running = account.running.expect("running");
        assert_eq!(running.phase, "import");
        assert_eq!(running.done, 4);
        assert_eq!(running.total, Some(10));
    }

    #[test]
    fn a_disabled_provider_still_paints_its_card() {
        let options = options_from(&serde_json::json!({
            "step": "account",
            "deezer": { "available": false, "can_link": false, "linked": false, "reason": "deezer_disabled" },
            "youtube": { "available": true, "can_link": true, "linked": false },
            "imports": [{ "source": "spotify", "label": "Spotify", "available": true }],
            "can_save_imports": true
        }));
        assert!(!options.deezer.available);
        assert_eq!(options.deezer.reason.as_deref(), Some("deezer_disabled"));
        assert!(options.youtube.can_link);
        assert_eq!(options.imports.len(), 1);
        assert!(options.imports[0].available);
    }

    #[test]
    fn the_encoding_matches_the_pages_own() {
        assert_eq!(encode("a b/c"), "a%20b%2Fc");
        assert_eq!(encode("é"), "%C3%A9");
        assert_eq!(encode("a.b-c_d~"), "a.b-c_d~");
    }
}
