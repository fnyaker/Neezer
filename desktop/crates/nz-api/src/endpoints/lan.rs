//! The LAN exchange between the person's devices: status, enrolment, roster,
//! enrolled devices, trusted friends and the admin settings
//! (`docs/agents/lan-nodes.md`, the *Server* route table).
//!
//! **LN7 filled every body** against the routes LN1 registered. What this file
//! is NOT allowed to do, and does not: verify a credential, a roster or a
//! revocation list. `nz-model` carries those as the base64/hex strings the
//! server signed, and `nz-lan`'s `cred.rs` verifies them over the exact bytes
//! received. A second decoder here would be a second thing to keep right.
//!
//! The lane of each call is part of the contract (rule 1), and is written in its
//! own doc comment so no caller has to guess:
//!
//! - `Prefetch` — the status probe: it is what comes right after opening the
//!   app, never something the person is waiting in front of.
//! - `Bulk` — enrolment and the roster: background work that yields to the
//!   tapped track, and the server gates it with `X-NS-Background: 1` for the
//!   same reason.
//! - `User` — everything a screen is looking at when the person acts on it:
//!   revoking a device, renaming it, adding a friend, changing a scope, the two
//!   admin settings.

use reqwest::Method;

use crate::client::{Api, encode};
use crate::error::ApiError;
use crate::lanes::Lane;
use nz_model::UserId;
use nz_model::lan::{
    LanDevice, LanEnrollAnswer, LanEnrollRequest, LanFriend, LanFriends, LanRosterAnswer, LanScope,
    LanServerSettings, LanStatus,
};

impl Api {
    /// `GET /api/lan/status` — the caller's subject, the server's LAN keys, the
    /// credential lifetime and the revocation serial (`Prefetch`: it is the
    /// probe, and the first request of an enrolment).
    ///
    /// `enabled = false` is the admin switch (rule 6): the caller hides the
    /// feature then, exactly as it does for a server without the route
    /// (`LanError::Disabled` against `LanError::Unsupported`).
    pub async fn lan_status(&self) -> Result<LanStatus, ApiError> {
        self.fetch(Lane::Prefetch, "/lan/status").await
    }

    /// `POST /api/lan/enroll` — the possession proof in, the credential, the
    /// roster and the revocation list out (`Bulk`: it is background work and it
    /// must never wait in front of a screen).
    ///
    /// Never retried (`Api::send`): replaying an enrolment would rotate a key
    /// twice and, on a renewal, retire the device's own previous key for
    /// nothing.
    pub async fn lan_enroll(&self, r: &LanEnrollRequest) -> Result<LanEnrollAnswer, ApiError> {
        self.send(Lane::Bulk, Method::POST, "/lan/enroll", Some(r))
            .await?
            .ok_or_else(|| ApiError::Decode("/lan/enroll: une réponse était attendue".to_owned()))
    }

    /// `GET /api/lan/roster` with the ETag this device already holds — `None`
    /// for a `304`, i.e. nothing changed and the body costs nothing (`Bulk`).
    ///
    /// The conditional half is [`Api::get_conditional`]: the server answers
    /// `304` when `If-None-Match` still matches, and that is the whole point —
    /// a device asks on every launch and normally pays one request, 0 bytes.
    /// `etag` `None` is an unconditional GET (a first sync, or after a gossiped
    /// roster made the held ETag stale).
    pub async fn lan_roster(
        &self,
        etag: Option<&str>,
    ) -> Result<Option<LanRosterAnswer>, ApiError> {
        match etag {
            Some(etag) => self.get_conditional(Lane::Bulk, "/lan/roster", etag).await,
            None => self.fetch(Lane::Bulk, "/lan/roster").await.map(Some),
        }
    }

    /// `GET /api/lan/devices` — this person's enrolled devices (`User`: the
    /// Réglages screen is open in front of them).
    pub async fn lan_devices(&self) -> Result<Vec<LanDevice>, ApiError> {
        self.get_value("/lan/devices").await
    }

    /// `DELETE /api/lan/devices/<id>` — revoke one device (`self` allowed);
    /// the server bumps the revocation serial, so every peer refuses it next
    /// time it meets a device that synced (`User`).
    ///
    /// `<id>` and the literal `self` are both opaque to this client: no other
    /// spelling is built here, and nothing from the wire becomes a path (the id
    /// came from the server's own list).
    pub async fn lan_revoke(&self, id: &str) -> Result<(), ApiError> {
        self.send_none(
            Method::DELETE,
            &format!("/lan/devices/{}", encode(id)),
            &serde_json::json!({}),
        )
        .await
    }

    /// `PATCH /api/lan/devices/<id>` — rename one device, 1–64 characters
    /// (`User`).
    pub async fn lan_rename(&self, id: &str, name: &str) -> Result<LanDevice, ApiError> {
        self.send_json(
            Method::PATCH,
            &format!("/lan/devices/{}", encode(id)),
            &serde_json::json!({ "name": name }),
        )
        .await
    }

    /// `GET /api/lan/friends` — the people I share with and the people who share
    /// with me (`User`).
    pub async fn lan_friends(&self) -> Result<LanFriends, ApiError> {
        self.get_value("/lan/friends").await
    }

    /// `POST /api/lan/friends` — add a trusted friend by exact account name
    /// (`User`). An unknown name, the caller's own and a disabled account all
    /// answer the same `404`, so the route never says who exists.
    pub async fn lan_add_friend(&self, name: &str) -> Result<LanFriend, ApiError> {
        self.send_json(
            Method::POST,
            "/lan/friends",
            &serde_json::json!({ "name": name }),
        )
        .await
    }

    /// `PATCH /api/lan/friends/<user_id>` — what that friend may fetch: all my
    /// local files, or the listed playlists (`User`).
    ///
    /// `<user_id>` is the ACCOUNT's UUID as `GET /api/lan/friends` sent it,
    /// never the LAN's opaque subject.
    pub async fn lan_set_friend(
        &self,
        user_id: &UserId,
        scope: &LanScope,
    ) -> Result<LanFriend, ApiError> {
        self.send_json(
            Method::PATCH,
            &format!("/lan/friends/{}", encode(user_id.as_str())),
            scope,
        )
        .await
    }

    /// `DELETE /api/lan/friends/<user_id>` — stop sharing with that friend
    /// (`User`).
    pub async fn lan_remove_friend(&self, user_id: &UserId) -> Result<(), ApiError> {
        self.send_none(
            Method::DELETE,
            &format!("/lan/friends/{}", encode(user_id.as_str())),
            &serde_json::json!({}),
        )
        .await
    }

    /// `GET /api/lan/settings` — the server's two LAN settings (admin only,
    /// rule 6).
    pub async fn lan_settings(&self) -> Result<LanServerSettings, ApiError> {
        self.get_value("/lan/settings").await
    }

    /// `PUT /api/lan/settings` — switch the feature and set the credential
    /// lifetime (7–90 days, admin only).
    pub async fn lan_set_settings(
        &self,
        s: &LanServerSettings,
    ) -> Result<LanServerSettings, ApiError> {
        self.send_json(Method::PUT, "/lan/settings", s).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The routes and lanes this file speaks, held together with LN0's own doc
    /// block (rule 1: an enrolment or a roster that ran on `User` would sit in
    /// front of the tapped track). The note lists the three lanes explicitly, so
    /// a change here is a contract change and must be argued.
    #[test]
    fn enrol_and_roster_are_background_the_rest_is_the_person() {
        assert!(Lane::Prefetch.is_background(), "the status probe yields");
        assert!(Lane::Bulk.is_background(), "enrolment and the roster yield");
        assert!(
            !Lane::User.is_background(),
            "a device list is what was opened"
        );
    }
}
