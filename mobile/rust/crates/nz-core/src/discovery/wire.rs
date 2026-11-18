//! The wire: the routes Découverte reads, their counted traffic, and the copies
//! this device keeps between two visits.
//!
//! Every call goes through `nz-api` on [`nz_api::Lane::User`] — the screen being
//! opened is the action just taken, never behind a prefetch or a download
//! (rule 1). Nothing here decides a rule: the shapes are `nz-model`'s where the
//! desktop already decodes them (releases, recap, moods, styles), and the SPA's
//! own routes otherwise (`routes/Releases.svelte`, `routes/Recap.svelte`,
//! `ExplorerStyles.svelte`, `lib/api.js`'s *discovery* block).
//!
//! The three routes `nz-api` has no typed endpoint for are called through its
//! generic verbs, on purpose: `/explorer/styles`, `/explorer/discover` and
//! `/radio/genre/<style>` are this lot's own screens, and `nz-api` is a shared
//! crate a feature lot does not edit.

use std::path::PathBuf;
use std::sync::Arc;

use nz_api::{Api, Lane, Method};
use nz_model::{FlowStyles, Mood, Moods, Recap, RecapYears, Releases, ReleasesSettings, Track};
use serde::{Deserialize, Serialize};

use super::{Core, CoreError, TAG_DISCOVERY};

/// What one tag spent, as `nz-api` counts it (SC3): requests really sent, and
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
        .find(|row| row.tag == TAG_DISCOVERY)
        .map(|row| Totals {
            requests: row.requests,
            bytes_in: row.bytes_in,
            bytes_out: row.bytes_out,
        })
        .unwrap_or_default()
}

// --- Nouveautés ---------------------------------------------------------------

/// `GET /api/releases` — the cards, their titles, the unseen badge and the
/// releases the server is still checking.
pub(super) async fn fetch_releases(api: &Api) -> Result<Releases, CoreError> {
    Ok(api.releases().await?)
}

/// `POST /api/releases/seen` — the badge goes away. One mutation, never retried
/// (`nz-api`'s own rule).
pub(super) async fn mark_seen(api: &Api) -> Result<(), CoreError> {
    api.releases_seen().await?;
    Ok(())
}

/// `GET /api/releases/settings` — what the background check watches.
pub(super) async fn fetch_release_settings(api: &Api) -> Result<ReleasesSettings, CoreError> {
    Ok(api.releases_settings().await?)
}

/// `PUT /api/releases/settings` — save what the shelf watches, then read the
/// whole status back: the screen shows the SERVER's clamp, never the request's
/// value (rule 6).
pub(super) async fn save_release_settings(
    api: &Api,
    change: &serde_json::Value,
) -> Result<ReleasesSettings, CoreError> {
    api.save_releases_settings(change).await?;
    fetch_release_settings(api).await
}

/// `POST /api/releases/check` — ask the provider now; the server runs it on a
/// worker (rule 2), so the answer carries the status and whether a pass started.
pub(super) async fn check_releases(api: &Api) -> Result<ReleasesSettings, CoreError> {
    api.releases_check().await?;
    fetch_release_settings(api).await
}

// --- Récap --------------------------------------------------------------------

/// `GET /api/recap/years` — the years that hold at least one event.
pub(super) async fn fetch_recap_years(api: &Api) -> Result<RecapYears, CoreError> {
    Ok(api.recap_years().await?)
}

/// `GET /api/recap/<year>` — one bounded year payload.
pub(super) async fn fetch_recap(api: &Api, year: i32) -> Result<Recap, CoreError> {
    Ok(api.recap(year).await?)
}

// --- moods --------------------------------------------------------------------

/// `GET /api/moods` — the six mixes built from the measured library.
pub(super) async fn fetch_moods(api: &Api) -> Result<Moods, CoreError> {
    Ok(api.moods().await?)
}

/// `GET /api/moods/<id>` — one mix's titles.
pub(super) async fn fetch_mood(api: &Api, id: &str) -> Result<Mood, CoreError> {
    Ok(api.mood(id).await?)
}

// --- Explorer -----------------------------------------------------------------

/// One row of `GET /api/explorer/styles` — the person's own listened styles, as
/// the server's taste profile ranks them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExplorerStyleRow {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub tracks: u32,
    #[serde(default)]
    pub weight: f64,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default)]
    pub leaf: bool,
    #[serde(default)]
    pub children: u32,
}

/// `/api/explorer/styles` — the ten parent styles plus the scale behind them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExplorerStyles {
    #[serde(default)]
    pub styles: Vec<ExplorerStyleRow>,
    /// `history`, `unanalysed` or `none`: whose taste this is, when it is none.
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub tracks: u32,
    #[serde(default)]
    pub analysed: f64,
    #[serde(default)]
    pub unknown: f64,
}

/// `GET /api/explorer/styles` — no provider is asked; an account with no history
/// answers an empty list and a `source` that says why.
pub(super) async fn fetch_explorer_styles(api: &Api) -> Result<ExplorerStyles, CoreError> {
    let fresh = api
        .get::<ExplorerStyles>(Lane::User, "/explorer/styles")
        .await?;
    Ok(fresh.value)
}

/// One playlist of a finished discovery (`deezer/explorer.py#discover_style`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DiscoverPlaylist {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(default)]
    pub nb_tracks: u32,
    /// How many of its titles the server's own analysis verified.
    #[serde(default)]
    pub matched: Option<u32>,
    #[serde(default)]
    pub tracks: Vec<Track>,
}

/// `/api/explorer/discover` — `state` is `idle`, `pending`, `ready` or carries
/// an `error`; the playlists come with their checked titles, so playing one
/// costs no second request (`ExplorerStyles.svelte#play`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DiscoverAnswer {
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub style: String,
    #[serde(default)]
    pub level: u32,
    #[serde(default)]
    pub playlists: Vec<DiscoverPlaylist>,
    /// `provider` or `failed`: the sweep is over and the cause is the provider.
    #[serde(default)]
    pub error: Option<String>,
}

/// `POST /api/explorer/discover` — starts ONE sweep and answers `pending`, or a
/// cached answer. Never polled by this call: the screen polls with
/// [`poll_explorer_discover`], which never starts anything.
pub(super) async fn start_explorer_discover(
    api: &Api,
    style: &str,
    level: u32,
) -> Result<DiscoverAnswer, CoreError> {
    let body = serde_json::json!({ "style": style, "level": level });
    let answer = api
        .send::<serde_json::Value, DiscoverAnswer>(
            Lane::User,
            Method::POST,
            "/explorer/discover",
            Some(&body),
        )
        .await?;
    Ok(answer.unwrap_or(DiscoverAnswer {
        state: "pending".to_owned(),
        style: style.to_owned(),
        level,
        ..DiscoverAnswer::default()
    }))
}

/// `GET /api/explorer/discover?style=&level=` — the poll. The server answers
/// `idle` when nothing is cached or running, and starts no work for a GET.
pub(super) async fn poll_explorer_discover(
    api: &Api,
    style: &str,
    level: u32,
) -> Result<DiscoverAnswer, CoreError> {
    let path = format!(
        "/explorer/discover?style={}&level={level}",
        encode_component(style)
    );
    let fresh = api.get::<DiscoverAnswer>(Lane::User, &path).await?;
    Ok(fresh.value)
}

/// `GET /api/flow/styles` — every analysed genre with its counts, which is both
/// the Flow's vocabulary (MB5) and the all-stations list (this lot).
pub(super) async fn fetch_station_styles(api: &Api) -> Result<FlowStyles, CoreError> {
    Ok(api.flow_styles().await?)
}

/// `GET /api/radio/genre/<style>` — one station: the tree's canonical node and
/// its titles. `<path:style>`: a genre may hold a slash ("Disco / funk"), so the
/// segment is percent-encoded the way `api.js#genreRadio` encodes it.
pub(super) async fn fetch_genre_station(
    api: &Api,
    style: &str,
) -> Result<StationAnswer, CoreError> {
    let path = format!("/radio/genre/{}", encode_component(style));
    let fresh = api.get::<StationAnswer>(Lane::User, &path).await?;
    Ok(fresh.value)
}

/// `/api/radio/genre/<style>` — `style` and `name` are the tree's canonical node.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StationAnswer {
    #[serde(default)]
    pub style: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub tracks: Vec<Track>,
}

// --- discovery settings, mutes, recommendations -------------------------------

/// `POST /api/discover/mute` — never propose this artist again.
pub(super) async fn mute_artist(api: &Api, provider: &str, id: &str) -> Result<(), CoreError> {
    let body = serde_json::json!({ "provider": provider, "id": id });
    api.send::<serde_json::Value, serde_json::Value>(
        Lane::User,
        Method::POST,
        "/discover/mute",
        Some(&body),
    )
    .await?;
    Ok(())
}

/// `GET /api/recommend?d=` — what the discovery level proposes.
pub(super) async fn fetch_recommend(
    api: &Api,
    level: f64,
) -> Result<nz_model::Recommend, CoreError> {
    Ok(api.recommend(level, None).await?)
}

/// Percent-encodes one path segment the way `encodeURIComponent` does, so a
/// genre name with a space or a slash reaches `<path:style>` unchanged.
fn encode_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
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
            | b')' => out.push(*byte as char),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

// --- the copies this device keeps --------------------------------------------

/// Where a snapshot of `name` lives: the CACHE directory (evictable), named after
/// a hash of the account so two accounts never read each other's screen.
pub(super) fn snapshot_path(core: &Core, name: &str) -> PathBuf {
    core.cache_path(&format!(
        "{name}-{:016x}.json",
        super::fnv1a(super::owner(core).as_bytes())
    ))
}

/// Reads one JSON snapshot, `None` when it is absent or unreadable.
pub(super) fn read_snapshot<T: serde::de::DeserializeOwned>(core: &Core, name: &str) -> Option<T> {
    let body = std::fs::read_to_string(snapshot_path(core, name)).ok()?;
    serde_json::from_str(&body).ok()
}

/// Writes it whole, through a temporary file and a rename: a process killed
/// mid-write leaves the previous copy, never half a JSON document (rule 7 for
/// the device's own data).
pub(super) fn write_snapshot<T: Serialize>(
    core: &Core,
    name: &str,
    value: &T,
) -> Result<(), CoreError> {
    let path = snapshot_path(core, name);
    let body = serde_json::to_vec(value).map_err(|error| CoreError::Internal {
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

/// The API's client under this screen's tag (a cheap `Arc` clone).
pub(super) fn tagged(core: &Core) -> Result<Arc<Api>, CoreError> {
    Ok(Arc::new(core.api()?.with_tag(TAG_DISCOVERY)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A genre with a space or a slash reaches `<path:style>` as ONE segment,
    /// exactly as `encodeURIComponent` sends it (`api.js#genreRadio`).
    #[test]
    fn a_genre_name_is_one_percent_encoded_segment() {
        assert_eq!(encode_component("Techno"), "Techno");
        assert_eq!(encode_component("Disco / funk"), "Disco%20%2F%20funk");
        assert_eq!(encode_component("Électro"), "%C3%89lectro");
        assert_eq!(encode_component("Drum & bass"), "Drum%20%26%20bass");
    }

    /// The SPA's four `stats()` numbers are the meter's, not a guess: a tagged
    /// client records under the tag this lot reads.
    #[test]
    fn the_tag_is_this_lots_own() {
        assert_eq!(TAG_DISCOVERY, "discovery");
    }
}
