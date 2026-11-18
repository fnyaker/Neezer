//! The wire: the routes the sheet and the two loops call, their counted traffic.
//!
//! The live traffic of a loan (the host's poll/publish, the controller's
//! state/queue/cmd) is `nz-remote`'s (`SC5`) and runs on its own loops with its
//! own timeouts; what lives here is everything a SCREEN asks of the server — mint
//! a link, cut one, list them, claim or leave a loan — plus the tag those calls
//! are counted under. `webapp/src/lib/remote/controller.js#leaveRemote`,
//! `host.js#cutAll` and `RemoteSheet.svelte#create/cut/cutEverything` name the
//! same routes.
//!
//! Every call is on [`nz_api::Lane::User`]: making a link is the action the person
//! just took (rule 1), never background work. Nothing here decides a rule — the
//! server's level table (`neezer/webui/remote.py#POLICY`) is the enforcement and
//! this file only refuses to send what is not even shaped like a request.

use std::sync::Arc;

use nz_api::{Api, Lane, Method};
use nz_model::{RemoteLink, RemoteLinks};
use serde_json::{Value, json};

use crate::core::CoreError;

/// What this screen's requests are counted under (`NzCore::stats`).
pub const TAG_REMOTE: &str = "remote";

/// `/api/remote/leave`'s answer, read for the account the server hands back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Left {
    /// The `user.name` of the account this browser had before the loan, when the
    /// server still knows it (`_end_remote_session`).
    pub restored: Option<String>,
    /// Whether the session really was a loan (a `false` here is not an error).
    pub was_remote: bool,
}

/// What one tag spent, as `nz-api` counts it (SC3): requests really sent, and
/// the response bodies' bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    pub requests: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

/// The tag's running totals on this client.
pub fn totals(api: &Api) -> Totals {
    api.stats()
        .into_iter()
        .find(|row| row.tag == TAG_REMOTE)
        .map(|row| Totals {
            requests: row.requests,
            bytes_in: row.bytes_in,
            bytes_out: row.bytes_out,
        })
        .unwrap_or_default()
}

/// The client every call here and every loop of this module shares: the session's
/// own, counting under [`TAG_REMOTE`].
pub fn client(api: &Arc<Api>) -> Arc<Api> {
    Arc::new(api.with_tag(TAG_REMOTE))
}

/// `GET /api/remote/links[?device=]` — the live links, newest first, and the
/// levels the server would accept.
pub async fn links(api: &Api, device: Option<&str>) -> Result<RemoteLinks, CoreError> {
    Ok(api.remote_links(device).await?)
}

/// `POST /api/remote/links` — lend one of this person's players for `ttl`
/// seconds (`None` = unlimited), under `level`, to the device this phone is.
pub async fn create(
    api: &Api,
    level: &str,
    device: &str,
    device_name: &str,
    label: Option<&str>,
    ttl: Option<u32>,
) -> Result<RemoteLink, CoreError> {
    let body = json!({
        "level": level,
        "device": device,
        "device_name": device_name,
        "label": label,
        "ttl": ttl,
    });
    let created = api.remote_create(&body).await?;
    Ok(created.link)
}

/// `DELETE /api/remote/links/<id>` — cut one link; the sessions it opened end on
/// their very next request.
pub async fn revoke(api: &Api, id: &str) -> Result<(), CoreError> {
    Ok(api.remote_revoke(id).await?)
}

/// `DELETE /api/remote/links?device=` — cut every link onto this device, the
/// owner's own button.
pub async fn revoke_device(api: &Api, device: &str) -> Result<(), CoreError> {
    Ok(api.remote_revoke_device(device).await?)
}

/// `POST /api/remote/claim` — this device becomes the controller of the player
/// `token` names. The answer's `remote` grant is what the banner paints.
pub async fn claim(api: &Api, token: &str) -> Result<Value, CoreError> {
    // The token is checked for SHAPE first (`host.js#claimToken`'s regexp): a
    // malformed one never leaves the device, so a pasted word costs nothing.
    let token =
        nz_remote::controller::token_from_input(token).ok_or_else(|| CoreError::Invalid {
            detail: "Ce lien de télécommande n'est pas valide".to_owned(),
        })?;
    Ok(api.remote_claim(&token).await?)
}

/// `POST /api/remote/leave` — hand the loan back. The answer carries the account
/// the server restored, which is what the "Contrôle terminé" card names.
pub async fn leave(api: &Api) -> Result<Left, CoreError> {
    let answer: Option<Value> = api
        .send::<Value, Value>(Lane::User, Method::POST, "/remote/leave", Some(&json!({})))
        .await?;
    let answer = answer.unwrap_or(Value::Null);
    let restored = answer
        .get("restored")
        .and_then(|user| user.get("name"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(Left {
        restored,
        was_remote: answer
            .get("restored")
            .is_some_and(|restored| !restored.is_null()),
    })
}
