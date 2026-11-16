//! Discovery: Flow, radios' home, moods, releases, recap, history and search.
//!
//! The routes Home and Découverte open (`webapp/src/lib/api.js`, *discovery*).
//! A person's history and recap are personal, so `lib/apicache.js` refuses them
//! offline; nothing here is marked cacheable by hand, [`crate::is_cacheable`]
//! decides, exactly as the SPA's list does.

use reqwest::Method;
use serde_json::Value;

use crate::client::{Api, encode};
use crate::endpoints::query;
use crate::error::ApiError;
use nz_model::{
    DiscoverPrefs, FlowStyles, History, Mood, Moods, Recap, RecapYears, Recommend, Releases,
    ReleasesSettings, SearchResults, TrackList,
};

impl Api {
    /// `GET /api/flow` — the Flow's first titles with no session (Home's own
    /// preview).
    pub async fn flow(&self) -> Result<TrackList, ApiError> {
        self.get_value("/flow").await
    }

    /// `GET /api/flow/styles` — the genres a Flow can be set to, with what the
    /// library holds of each.
    pub async fn flow_styles(&self) -> Result<FlowStyles, ApiError> {
        self.get_value("/flow/styles").await
    }

    /// `POST /api/flow/next` — the next title(s), chosen with what the session
    /// says. The body is `lib/flow.js#body`'s own object, sent as built: the
    /// screen owns that vocabulary, the server validates it.
    pub async fn flow_pick(&self, payload: &Value) -> Result<TrackList, ApiError> {
        self.send_json(Method::POST, "/flow/next", payload).await
    }

    /// `PUT /api/discover/prefs` — the Flow's starting energy notch (`None`:
    /// "Libre").
    pub async fn set_flow_energy(&self, energy: Option<f64>) -> Result<Value, ApiError> {
        let body = serde_json::json!({ "energy": energy });
        self.send_json(Method::PUT, "/discover/prefs", &body).await
    }

    /// `GET /api/recommend?d=&seed=` — what the discovery notch proposes.
    pub async fn recommend(&self, level: f64, seed: Option<&str>) -> Result<Recommend, ApiError> {
        let path = format!(
            "/recommend{}",
            query(&[
                ("d", Some(format!("{level:.2}"))),
                ("seed", seed.map(str::to_owned)),
            ])
        );
        self.get_value(&path).await
    }

    /// `GET /api/discover/prefs` — the notch, its steps and the energy ladder.
    pub async fn discover_prefs(&self) -> Result<DiscoverPrefs, ApiError> {
        self.get_value("/discover/prefs").await
    }

    /// `PUT /api/discover/prefs` — move the discovery notch.
    pub async fn set_discover_level(&self, level: f64) -> Result<Value, ApiError> {
        let body = serde_json::json!({ "level": level });
        self.send_json(Method::PUT, "/discover/prefs", &body).await
    }

    /// `GET /api/history` — the person's own listens, newest first, with a
    /// 30-day summary. Paged by `(before, before_id)`, never unbounded.
    pub async fn history(
        &self,
        before: Option<&str>,
        before_id: Option<i64>,
        limit: u32,
        outcome: Option<&str>,
    ) -> Result<History, ApiError> {
        let path = format!(
            "/history{}",
            query(&[
                ("before", before.map(str::to_owned)),
                ("before_id", before_id.map(|id| id.to_string())),
                ("limit", Some(limit.to_string())),
                ("outcome", outcome.map(str::to_owned)),
            ])
        );
        self.get_value(&path).await
    }

    /// `GET /api/recap/years` — the years that hold at least one listen.
    pub async fn recap_years(&self) -> Result<RecapYears, ApiError> {
        self.get_value("/recap/years").await
    }

    /// `GET /api/recap/<year>` — one bounded year payload (the server's `ETag`
    /// makes a second visit a `304`).
    pub async fn recap(&self, year: i32) -> Result<Recap, ApiError> {
        self.get_value(&format!("/recap/{year}")).await
    }

    /// `GET /api/moods` — the mixes built from the measured library.
    pub async fn moods(&self) -> Result<Moods, ApiError> {
        self.get_value("/moods").await
    }

    /// `GET /api/moods/<id>` — one mood's tracks.
    pub async fn mood(&self, id: &str) -> Result<Mood, ApiError> {
        self.get_value(&format!("/moods/{}", encode(id))).await
    }

    /// `POST /api/discover/mute` — never propose this artist again.
    pub async fn mute_artist(&self, provider: &str, id: &str) -> Result<Value, ApiError> {
        let body = serde_json::json!({ "provider": provider, "id": id });
        self.send_json(Method::POST, "/discover/mute", &body).await
    }

    /// `GET /api/releases` — the new releases shelf.
    pub async fn releases(&self) -> Result<Releases, ApiError> {
        self.get_value("/releases").await
    }

    /// `POST /api/releases/seen` — the badge goes away.
    pub async fn releases_seen(&self) -> Result<(), ApiError> {
        self.post_action("/releases/seen").await
    }

    /// `GET /api/releases/settings` — what the shelf watches.
    pub async fn releases_settings(&self) -> Result<ReleasesSettings, ApiError> {
        self.get_value("/releases/settings").await
    }

    /// `PUT /api/releases/settings` — save what the shelf watches.
    pub async fn save_releases_settings(&self, values: &Value) -> Result<Value, ApiError> {
        self.send_json(Method::PUT, "/releases/settings", values)
            .await
    }

    /// `POST /api/releases/check` — ask the provider now (a background job).
    pub async fn releases_check(&self) -> Result<(), ApiError> {
        self.post_action("/releases/check").await
    }

    /// `GET /api/search?q=` — the five families in one answer. Empty families
    /// are normal (Deezer off means the library's own matches only).
    pub async fn search(&self, q: &str) -> Result<SearchResults, ApiError> {
        self.get_value(&format!("/search{}", query(&[("q", Some(q.to_owned()))])))
            .await
    }
}
