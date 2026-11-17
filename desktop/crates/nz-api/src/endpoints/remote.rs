//! Remote control (this app controlled by another device, or controlling one)
//! and the listen party's host side.
//!
//! The LIVE traffic of a loan — the host's publish/poll, the controller's
//! state/commands — does not belong here: it runs on its own loops with its own
//! timeouts (`lib/remote/`, D15a). These are the routes a sheet calls: mint or
//! cut a link, claim one, start or end a party.

use reqwest::Method;
use serde::Deserialize;
use serde_json::Value;

use crate::client::{Api, encode};
use crate::endpoints::query;
use crate::error::ApiError;
use crate::lanes::Lane;
use nz_model::{Party, PartyMine, PartyPublish, PartyState, RemoteLink, RemoteLinks};

/// `POST /api/remote/links`'s answer: the new link. `link.token` is shown ONCE —
/// it is the capability another device claims.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct RemoteCreated {
    #[serde(default)]
    pub link: RemoteLink,
}

impl Api {
    /// `GET /api/remote/links?device=` — the links onto this person's players,
    /// and the levels they may use.
    pub async fn remote_links(&self, device: Option<&str>) -> Result<RemoteLinks, ApiError> {
        let path = format!(
            "/remote/links{}",
            query(&[("device", device.map(str::to_owned))])
        );
        self.get_value(&path).await
    }

    /// `POST /api/remote/links` — lend one of this person's players to another
    /// device for `ttl` seconds.
    pub async fn remote_create(&self, options: &Value) -> Result<RemoteCreated, ApiError> {
        self.send_json(Method::POST, "/remote/links", options).await
    }

    /// `DELETE /api/remote/links/<id>` — cut one link.
    pub async fn remote_revoke(&self, id: &str) -> Result<(), ApiError> {
        self.send_none(
            Method::DELETE,
            &format!("/remote/links/{}", encode(id)),
            &serde_json::json!({}),
        )
        .await
    }

    /// `DELETE /api/remote/links?device=` — cut every link onto one device.
    pub async fn remote_revoke_device(&self, device: &str) -> Result<(), ApiError> {
        let path = format!(
            "/remote/links{}",
            query(&[("device", Some(device.to_owned()))])
        );
        self.send_none(Method::DELETE, &path, &serde_json::json!({}))
            .await
    }

    /// `POST /api/remote/claim` — this device becomes the controller. The
    /// session then runs under the owner's grant (its `level` is what the UI
    /// hides), and `remote` appears in `/api/me`.
    pub async fn remote_claim(&self, token: &str) -> Result<Value, ApiError> {
        let body = serde_json::json!({ "token": token });
        self.send_json(Method::POST, "/remote/claim", &body).await
    }

    /// `POST /api/remote/leave` — hand the loan back.
    pub async fn remote_leave(&self) -> Result<(), ApiError> {
        self.post_action("/remote/leave").await
    }

    /// `POST /api/party` — start (or resume) the party this person hosts.
    pub async fn party_start(&self, options: Option<&Value>) -> Result<Party, ApiError> {
        let body = options.cloned().unwrap_or_else(|| serde_json::json!({}));
        self.send_json(Method::POST, "/party", &body).await
    }

    /// `GET /api/party/mine` — the party this person is hosting, if any.
    pub async fn party_mine(&self) -> Result<PartyMine, ApiError> {
        self.get_value("/party/mine").await
    }

    /// `DELETE /api/party/<id>` — end it; guests' chunks stop.
    pub async fn party_end(&self, id: &str) -> Result<(), ApiError> {
        self.send_none(
            Method::DELETE,
            &format!("/party/{}", encode(id)),
            &serde_json::json!({}),
        )
        .await
    }

    /// `POST /api/party/<id>/state` — the host's timeline (clock, position, the
    /// next handover, the normalisation level), published on the output clock's
    /// own beat. A heartbeat is the same route with `hb: true` and no track.
    ///
    /// The answer is the host's own view (`_host_view`), which is `Party`.
    pub async fn party_publish(&self, id: &str, state: &PartyPublish) -> Result<Party, ApiError> {
        self.send_json(Method::POST, &format!("/party/{}/state", encode(id)), state)
            .await
    }

    /// `GET /api/party/<id>` — the timeline a GUEST polls, no session needed
    /// (the SPA's `lib/party/guest.js`; `/state` is the HOST's POST, not this
    /// read). `listener` is the seat the guest holds (`?l=`), which marks it
    /// alive and gets `me` back; `quality_ms` is its current sync estimate
    /// (`?q=`, milliseconds) and `state` its word for the host's list (`?st=`).
    pub async fn party_state(
        &self,
        id: &str,
        listener: Option<&str>,
        quality_ms: Option<u32>,
        state: Option<&str>,
    ) -> Result<PartyState, ApiError> {
        let path = format!("/party/{}", encode(id));
        self.fetch(
            Lane::User,
            &format!(
                "{path}{}",
                query(&[
                    ("l", listener.map(str::to_owned)),
                    ("q", quality_ms.map(|ms| ms.to_string())),
                    ("st", state.map(str::to_owned)),
                ])
            ),
        )
        .await
    }
}
