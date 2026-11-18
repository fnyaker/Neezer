//! The LAN exchange between the person's devices: status, enrolment, roster,
//! enrolled devices, trusted friends and the admin settings
//! (`docs/agents/lan-nodes.md`, the *Server* route table).
//!
//! **LN0 declares the surface and nothing else.** Every body answers
//! [`absent`], which is the failure `Api::has_route` reads as "this server does
//! not have that route" — so no caller can mistake a stub for a live route, and
//! `Feature::Lan`'s probe keeps answering what it answers today. LN7 fills the
//! bodies (and only the bodies) once LN1's routes exist on the server.
//!
//! The lane of each call is part of the contract (rule 1), and is written in its
//! own doc comment so LN7 does not have to guess:
//!
//! - `Prefetch` — the status probe: it is what comes right after opening the
//!   app, never something the person is waiting in front of.
//! - `Bulk` — enrolment and the roster: background work that yields to the
//!   tapped track, and the server gates it with `X-NS-Background: 1` for the
//!   same reason.
//! - `User` — everything a screen is looking at when the person acts on it:
//!   revoking a device, renaming it, adding a friend, changing a scope, the two
//!   admin settings.

use crate::client::Api;
use crate::error::ApiError;
use nz_model::lan::{
    LanDevice, LanEnrollAnswer, LanEnrollRequest, LanFriend, LanFriends, LanRosterAnswer, LanScope,
    LanServerSettings, LanStatus,
};

/// The failure of a `/api/lan/*` route this build does not call yet.
///
/// `404` is not a placeholder status chosen for looks: it is exactly what an
/// older server answers for a route it never had, which is what
/// [`Api::has_route`] turns into `Some(false)`, and what `Feature::Lan` must
/// keep answering until LN1 registers the real routes. The sentence is the one
/// a screen would show if the failure ever escaped (`ApiError::cause_fr`).
fn absent() -> ApiError {
    ApiError::Http {
        status: 404,
        message: "la fonction « échange entre appareils proches » n'est pas sur ce serveur"
            .to_owned(),
        code: None,
    }
}

impl Api {
    /// `GET /api/lan/status` — the caller's subject, the server's LAN keys, the
    /// credential lifetime and the revocation serial (`Prefetch`: it is the
    /// probe, and the first request of an enrolment).
    pub async fn lan_status(&self) -> Result<LanStatus, ApiError> {
        Err(absent())
    }

    /// `POST /api/lan/enroll` — the possession proof in, the credential, the
    /// roster and the revocation list out (`Bulk`: it is background work and it
    /// must never wait in front of a screen).
    pub async fn lan_enroll(&self, r: &LanEnrollRequest) -> Result<LanEnrollAnswer, ApiError> {
        let _ = r;
        Err(absent())
    }

    /// `GET /api/lan/roster` with the ETag this device already holds — `None`
    /// for a `304`, i.e. nothing changed and the body costs nothing (`Bulk`).
    pub async fn lan_roster(
        &self,
        etag: Option<&str>,
    ) -> Result<Option<LanRosterAnswer>, ApiError> {
        let _ = etag;
        Err(absent())
    }

    /// `GET /api/lan/devices` — this person's enrolled devices (`User`: the
    /// Réglages screen is open in front of them).
    pub async fn lan_devices(&self) -> Result<Vec<LanDevice>, ApiError> {
        Err(absent())
    }

    /// `DELETE /api/lan/devices/<id>` — revoke one device (`self` allowed);
    /// the server bumps the revocation serial, so every peer refuses it next
    /// time it meets a device that synced (`User`).
    pub async fn lan_revoke(&self, id: &str) -> Result<(), ApiError> {
        let _ = id;
        Err(absent())
    }

    /// `PATCH /api/lan/devices/<id>` — rename one device, 1–64 characters
    /// (`User`).
    pub async fn lan_rename(&self, id: &str, name: &str) -> Result<LanDevice, ApiError> {
        let _ = (id, name);
        Err(absent())
    }

    /// `GET /api/lan/friends` — the people I share with and the people who share
    /// with me (`User`).
    pub async fn lan_friends(&self) -> Result<LanFriends, ApiError> {
        Err(absent())
    }

    /// `POST /api/lan/friends` — add a trusted friend by exact account name
    /// (`User`). An unknown name, the caller's own and a disabled account all
    /// answer the same `404`, so the route never says who exists.
    pub async fn lan_add_friend(&self, name: &str) -> Result<LanFriend, ApiError> {
        let _ = name;
        Err(absent())
    }

    /// `PATCH /api/lan/friends/<user_id>` — what that friend may fetch: all my
    /// local files, or the listed playlists (`User`).
    pub async fn lan_set_friend(
        &self,
        user_id: u32,
        scope: &LanScope,
    ) -> Result<LanFriend, ApiError> {
        let _ = (user_id, scope);
        Err(absent())
    }

    /// `DELETE /api/lan/friends/<user_id>` — stop sharing with that friend
    /// (`User`).
    pub async fn lan_remove_friend(&self, user_id: u32) -> Result<(), ApiError> {
        let _ = user_id;
        Err(absent())
    }

    /// `GET /api/lan/settings` — the server's two LAN settings (admin only,
    /// rule 6).
    pub async fn lan_settings(&self) -> Result<LanServerSettings, ApiError> {
        Err(absent())
    }

    /// `PUT /api/lan/settings` — switch the feature and set the credential
    /// lifetime (7–90 days, admin only).
    pub async fn lan_set_settings(
        &self,
        s: &LanServerSettings,
    ) -> Result<LanServerSettings, ApiError> {
        let _ = s;
        Err(absent())
    }
}
