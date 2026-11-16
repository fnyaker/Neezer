//! Onboarding (picking artists, linking an account, importing what it holds)
//! and the invitation flow.
//!
//! Everything here is on the LIVE path (`lib/apicache.js#LIVE_PATHS` refuses
//! `/onboarding`, `/invites` and `/admin`), so nothing is ever served off disk:
//! a stale onboarding screen would offer people who are no longer proposed.

use reqwest::Method;
use serde_json::Value;

use crate::client::{Api, encode};
use crate::endpoints::query;
use crate::error::ApiError;
use nz_model::{OnboardingAccountOptions, OnboardingArtists, OnboardingSearch, OnboardingState};

impl Api {
    /// `GET /api/onboarding` — where the person stands, what the feed is doing,
    /// and what may still be linked.
    pub async fn onboarding_state(&self) -> Result<OnboardingState, ApiError> {
        self.get_value("/onboarding").await
    }

    /// `GET /api/onboarding/artists?after=&limit=` (or, with `q`, the search) —
    /// the artists to pick from, paged by an opaque cursor.
    pub async fn onboarding_artists(
        &self,
        after: Option<&str>,
        limit: Option<u32>,
        q: &str,
    ) -> Result<OnboardingArtists, ApiError> {
        if !q.is_empty() {
            let path = format!(
                "/onboarding/search{}",
                query(&[
                    ("q", Some(q.to_owned())),
                    ("limit", limit.map(|value| value.to_string())),
                ])
            );
            // The search answers `OnboardingSearch`; its `items` are the same
            // artist rows, and the screen shows them in the same list.
            let found: OnboardingSearch = self.get_value(&path).await?;
            return Ok(OnboardingArtists {
                items: found.items,
                partial: found.partial,
                error: found.error,
                ..OnboardingArtists::default()
            });
        }
        let path = format!(
            "/onboarding/artists{}",
            query(&[
                ("after", after.filter(|a| !a.is_empty()).map(str::to_owned)),
                ("limit", limit.map(|value| value.to_string())),
            ])
        );
        self.get_value(&path).await
    }

    /// `GET /api/onboarding/search?q=&limit=` — the search box's own answer.
    pub async fn onboarding_search(
        &self,
        q: &str,
        limit: Option<u32>,
    ) -> Result<OnboardingSearch, ApiError> {
        let path = format!(
            "/onboarding/search{}",
            query(&[
                ("q", Some(q.to_owned())),
                ("limit", limit.map(|value| value.to_string())),
            ])
        );
        self.get_value(&path).await
    }

    /// `POST /api/onboarding/artists/like` — keep one artist; `seen` is what the
    /// screen has already shown, so the feed can avoid it.
    pub async fn onboarding_like(
        &self,
        artist: &str,
        seen: Option<&[String]>,
    ) -> Result<Value, ApiError> {
        let mut body = serde_json::json!({ "deezer_id": artist });
        if let Some(seen) = seen.filter(|seen| !seen.is_empty()) {
            body["seen"] = Value::from(seen.to_vec());
        }
        self.send_json(Method::POST, "/onboarding/artists/like", &body)
            .await
    }

    /// `DELETE /api/onboarding/artists/<id>/like` — take one back.
    pub async fn onboarding_unlike(&self, artist: &str) -> Result<(), ApiError> {
        self.send_none(
            Method::DELETE,
            &format!("/onboarding/artists/{}/like", encode(artist)),
            &serde_json::json!({}),
        )
        .await
    }

    /// `GET /api/onboarding/artists/<id>/related?seen=` — neighbours of the
    /// artist being looked at.
    pub async fn onboarding_related(
        &self,
        id: &str,
        seen: Option<&[String]>,
    ) -> Result<Value, ApiError> {
        let seen = seen
            .filter(|seen| !seen.is_empty())
            .map(|seen| seen.join(","));
        let path = format!(
            "/onboarding/artists/{}/related{}",
            encode(id),
            query(&[("seen", seen)])
        );
        self.get_value(&path).await
    }

    /// `POST /api/onboarding/artists/done` — the artist step is finished; the
    /// feed is built.
    pub async fn onboarding_artists_done(
        &self,
        payload: Option<&Value>,
    ) -> Result<Value, ApiError> {
        match payload {
            Some(payload) => {
                self.send_json(Method::POST, "/onboarding/artists/done", payload)
                    .await
            }
            None => self
                .post_value::<Value>("/onboarding/artists/done")
                .await
                .map(|answer| answer.unwrap_or(Value::Null)),
        }
    }

    /// `POST /api/onboarding/feed/retry` — rebuild the feed now.
    pub async fn onboarding_feed_retry(&self) -> Result<(), ApiError> {
        self.post_action("/onboarding/feed/retry").await
    }

    /// `GET /api/onboarding/account/options` — what could be linked or
    /// imported at the account step.
    pub async fn onboarding_account_options(&self) -> Result<OnboardingAccountOptions, ApiError> {
        self.get_value("/onboarding/account/options").await
    }

    /// `POST /api/onboarding/account/link` — link an account during onboarding.
    pub async fn onboarding_account_link(&self, payload: &Value) -> Result<Value, ApiError> {
        self.send_json(Method::POST, "/onboarding/account/link", payload)
            .await
    }

    /// `POST /api/onboarding/account/import` — import what the linked account
    /// holds (a job; the answer carries its id).
    pub async fn onboarding_account_import(&self, payload: &Value) -> Result<Value, ApiError> {
        self.send_json(Method::POST, "/onboarding/account/import", payload)
            .await
    }

    /// `POST /api/onboarding/account/done` — the account step is finished.
    pub async fn onboarding_account_done(&self) -> Result<Value, ApiError> {
        self.post_value::<Value>("/onboarding/account/done")
            .await
            .map(|answer| answer.unwrap_or(Value::Null))
    }

    /// `POST /api/invites/inspect` — what an invitation link is for, before
    /// redeeming it. No session needed.
    pub async fn invite_inspect(&self, token: &str) -> Result<Value, ApiError> {
        let body = serde_json::json!({ "token": token });
        self.send_json(Method::POST, "/invites/inspect", &body)
            .await
    }

    /// `POST /api/invites/redeem` — accept an invitation (creates the account
    /// when the token is for a new person).
    pub async fn invite_redeem(&self, payload: &Value) -> Result<Value, ApiError> {
        self.send_json(Method::POST, "/invites/redeem", payload)
            .await
    }
}
