//! One typed function per SPA `api.*` method (`webapp/src/lib/api.js`), with
//! the same names in snake_case and the same routes, bodies and query strings.
//!
//! The SPA is the contract (docs/agents/desktop.md, *Feature inventory*): a
//! screen that exists there must be able to make exactly the request the web
//! app makes, so the server never has to know which client is talking. Where
//! `nz-model` (D1) typed the answer, the function returns that type; where the
//! answer is a job handle or a state object the screens read field by field,
//! it returns `serde_json::Value` — never an invented struct that would drop
//! what the server sends.
//!
//! Everything here runs on [`Lane::User`] unless the caller asks otherwise:
//! opening a screen is what the person just did. Data for the NEXT screen is
//! fetched with [`crate::Api::get`] + [`Lane::Prefetch`] by the caller, because
//! only the caller knows what comes next.

use reqwest::Method;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::client::{Api, encode};
use crate::error::ApiError;
use crate::lanes::Lane;

pub mod accounts;
pub mod admin;
pub mod auth;
pub mod discovery;
pub mod entities;
pub mod lan;
pub mod library;
pub mod media;
pub mod onboarding;
pub mod player;
pub mod playlists;
pub mod podcasts;
pub mod remote;
pub mod settings;

/// `?a=1&b=2`, `encodeURIComponent`-encoded, with `None` values dropped — the
/// SPA's `URLSearchParams` idiom.
pub(crate) fn query(pairs: &[(&str, Option<String>)]) -> String {
    let parts: Vec<String> = pairs
        .iter()
        .filter_map(|(key, value)| {
            value
                .as_ref()
                .map(|value| format!("{}={}", encode(key), encode(value)))
        })
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!("?{}", parts.join("&"))
    }
}

/// A JSON object with one key replaced or added — the SPA's
/// `{ ...fields, version }` shape, which is how every optimistic edit travels.
pub(crate) fn with(
    base: serde_json::Value,
    key: &str,
    value: serde_json::Value,
) -> serde_json::Value {
    match base {
        serde_json::Value::Object(mut map) => {
            map.insert(key.to_owned(), value);
            serde_json::Value::Object(map)
        }
        _ => {
            let mut map = serde_json::Map::new();
            map.insert(key.to_owned(), value);
            serde_json::Value::Object(map)
        }
    }
}

/// Two JSON objects spread into one — the SPA's `{ ...scope, tag }` /
/// `{ file, ...options }`. The left object wins nothing: the right one is the
/// caller's newer intent.
pub(crate) fn merge(base: serde_json::Value, extra: &serde_json::Value) -> serde_json::Value {
    let mut merged = match base {
        serde_json::Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    if let serde_json::Value::Object(extra) = extra {
        for (key, value) in extra {
            merged.insert(key.clone(), value.clone());
        }
    }
    serde_json::Value::Object(merged)
}

/// The id list the SPA sends as `(ids || []).map(String)`.
pub(crate) fn id_list<I, S>(ids: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: ToString,
{
    ids.into_iter().map(|id| id.to_string()).collect()
}

impl Api {
    /// A GET on a specific lane, its value only.
    pub(crate) async fn fetch<T: DeserializeOwned>(
        &self,
        lane: Lane,
        path: &str,
    ) -> Result<T, ApiError> {
        Ok(self.get(lane, path).await?.value)
    }

    /// A GET of one screen's data (`Lane::User`).
    pub(crate) async fn get_value<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        self.fetch(Lane::User, path).await
    }

    /// A POST/PUT/PATCH with a JSON body, whose answer the caller needs.
    pub(crate) async fn send_json<B: Serialize, T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        self.send(Lane::User, method, path, Some(body))
            .await?
            .ok_or_else(|| ApiError::Decode(format!("{path}: une réponse était attendue")))
    }

    /// A mutation whose answer is nothing (the server answers `204`, or an
    /// object no screen reads).
    pub(crate) async fn send_none<B: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: &B,
    ) -> Result<(), ApiError> {
        self.send::<B, serde_json::Value>(Lane::User, method, path, Some(body))
            .await
            .map(|_| ())
    }

    /// A mutation with no body at all (`POST /api/logout`).
    pub(crate) async fn post_action(&self, path: &str) -> Result<(), ApiError> {
        self.send::<(), serde_json::Value>(Lane::User, Method::POST, path, None)
            .await
            .map(|_| ())
    }

    /// A mutation with no body whose answer is an object the screen polls.
    pub(crate) async fn post_value<T: DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<Option<T>, ApiError> {
        self.send::<(), T>(Lane::User, Method::POST, path, None)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The query strings the SPA builds. A space, an accent and a slash must be
    /// encoded the same way `encodeURIComponent` does, or the same style flow
    /// lands on a different route.
    #[test]
    fn query_strings_match_the_spas_url_search_params() {
        assert_eq!(query(&[]), "");
        assert_eq!(query(&[("limit", Some("50".into()))]), "?limit=50");
        assert_eq!(
            query(&[("q", Some("Rap FR".into())), ("limit", None)]),
            "?q=Rap%20FR"
        );
        assert_eq!(
            query(&[("d", Some("0.30".into())), ("seed", Some("abc/def".into()))]),
            "?d=0.30&seed=abc%2Fdef"
        );
    }

    #[test]
    fn an_id_list_is_strings_the_way_the_server_resolves_them() {
        assert_eq!(
            id_list([3000168u64, 0]),
            vec!["3000168".to_owned(), "0".to_owned()]
        );
        assert_eq!(id_list(["89e80170-adc3"]), vec!["89e80170-adc3".to_owned()]);
    }
}
