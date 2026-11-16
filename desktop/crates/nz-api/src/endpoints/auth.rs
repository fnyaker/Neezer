//! Session and identity: `login`, `logout`, `me`, who may use what.
//!
//! The cookie itself lives in [`crate::SessionJar`]; this module is what the
//! login screen calls. Two things the SPA does and this must too: a successful
//! login remembers the account the cache is keyed by, and `me` is cheap enough
//! to call on every start (it is what restores a saved session).

use reqwest::Method;
use serde::Serialize;
use serde_json::Value;

use crate::client::Api;
use crate::error::ApiError;
use nz_model::{Access, Me, Version};

#[derive(Serialize)]
struct Credentials<'a> {
    username: &'a str,
    password: &'a str,
}

impl Api {
    /// `POST /api/login` — sets the session cookie. The answer is `/api/me`'s
    /// own body, so a caller needs no second request to know who it is.
    pub async fn login(&self, username: &str, password: &str) -> Result<Me, ApiError> {
        let me: Me = self
            .send_json(Method::POST, "/login", &Credentials { username, password })
            .await?;
        self.set_owner(Some(me.user.id.as_str().to_owned()));
        Ok(me)
    }

    /// `POST /api/logout` — ends the session and forgets the cookie on disk.
    ///
    /// A remote-control loan logs out of the LOAN: the server hands back the
    /// account this device had before, so the cookie is then the person's own
    /// and must not be thrown away. [`nz_model::Me::remote`] told the caller
    /// which one it was; the owner is re-read by the next `me`.
    pub async fn logout(&self) -> Result<(), ApiError> {
        self.post_action("/logout").await?;
        self.forget_session();
        Ok(())
    }

    /// `GET /api/me` — the session's own answer, and the source of what the UI
    /// may offer: `access`, `sources`, `admin`, and `remote` for a loan.
    pub async fn me(&self) -> Result<Me, ApiError> {
        let me: Me = self.get_value("/me").await?;
        self.set_owner(Some(me.user.id.as_str().to_owned()));
        Ok(me)
    }

    /// `GET /api/access` — every provider, its accounts, its features and who
    /// may use them. What Réglages → Sources renders (the server enforces).
    pub async fn access(&self) -> Result<Access, ApiError> {
        self.get_value("/access").await
    }

    /// `PUT /api/access/grants` — grant or revoke one feature of one account for
    /// a set of subjects.
    pub async fn set_grants(
        &self,
        provider: &str,
        account: &str,
        feature: &str,
        subjects: &[String],
    ) -> Result<Value, ApiError> {
        let body = serde_json::json!({
            "provider": provider,
            "account": account,
            "feature": feature,
            "subjects": subjects,
        });
        self.send_json(Method::PUT, "/access/grants", &body).await
    }

    /// `POST /api/groups` — a named set of people, the unit grants are given to.
    pub async fn create_group(&self, name: &str, members: &[String]) -> Result<Value, ApiError> {
        let body = serde_json::json!({ "name": name, "members": members });
        self.send_json(Method::POST, "/groups", &body).await
    }

    /// `PUT /api/groups/<id>` — rename a group or change its members.
    pub async fn update_group(&self, id: &str, changes: &Value) -> Result<Value, ApiError> {
        let path = format!("/groups/{}", crate::client::encode(id));
        self.send_json(Method::PUT, &path, changes).await
    }

    /// `DELETE /api/groups/<id>`.
    pub async fn delete_group(&self, id: &str) -> Result<(), ApiError> {
        let path = format!("/groups/{}", crate::client::encode(id));
        self.send_none(Method::DELETE, &path, &serde_json::json!({}))
            .await
    }

    /// `GET /api/version` — the server's build identity (and the Android
    /// release it expects).
    pub async fn version(&self) -> Result<Version, ApiError> {
        self.get_value("/version").await
    }
}
