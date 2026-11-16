//! One entity at a time: artist, album, track, its credits, its gain, its
//! measured tempo, and the radios built from it.
//!
//! These are the routes a screen calls when it opens (`webapp/src/lib/api.js`,
//! *entities* and *radios*). Every one of them is [`Lane::User`] work: the
//! person just clicked the row.

use reqwest::Method;
use serde_json::Value;

use crate::client::{Api, encode};
use crate::endpoints::query;
use crate::error::ApiError;
use crate::lanes::Lane;
use nz_model::{
    AlbumPage, Analyses, ArtistPage, AudioEdges, Discography, Gain, Gains, Lyrics, PersonCredits,
    ReplaceCandidates, TrackAnalysis, TrackCredits, TrackList, TrackProbe, Unavailable,
    YoutubeSearch,
};

impl Api {
    /// `GET /api/artist/<id>` — the page: header, top titles, albums, neighbours.
    pub async fn artist(&self, id: &str) -> Result<ArtistPage, ApiError> {
        self.get_value(&format!("/artist/{}", encode(id))).await
    }

    /// `GET /api/artist/<id>/tracks` — everything of this artist in the library.
    pub async fn artist_tracks(&self, id: &str) -> Result<TrackList, ApiError> {
        self.get_value(&format!("/artist/{}/tracks", encode(id)))
            .await
    }

    /// `GET /api/artist/<id>/discography` — grouped by release type, newest
    /// first.
    pub async fn discography(&self, id: &str) -> Result<Discography, ApiError> {
        self.get_value(&format!("/artist/{}/discography", encode(id)))
            .await
    }

    /// `GET /api/artist/<id>/on-deezer` — the part of the page only the
    /// provider has, asked once the page is shown.
    pub async fn artist_on_deezer(&self, id: &str) -> Result<Value, ApiError> {
        self.get_value(&format!("/artist/{}/on-deezer", encode(id)))
            .await
    }

    /// `GET /api/album/<id>` — the album, its tracks in order, whether the
    /// library holds it.
    pub async fn album(&self, id: &str) -> Result<AlbumPage, ApiError> {
        self.get_value(&format!("/album/{}", encode(id))).await
    }

    /// `GET /api/album/<id>/on-deezer` — the provider-only half of an album
    /// page.
    pub async fn album_on_deezer(&self, id: &str) -> Result<Value, ApiError> {
        self.get_value(&format!("/album/{}/on-deezer", encode(id)))
            .await
    }

    /// `GET /api/lyrics/<id>` — `null` when the provider has none; not an error.
    pub async fn lyrics(&self, id: &str) -> Result<Lyrics, ApiError> {
        self.get_value(&format!("/lyrics/{}", encode(id))).await
    }

    /// `GET /api/gain/<id>` — ReplayGain in dB, `null` while unmeasured.
    pub async fn track_gain(&self, id: &str) -> Result<Gain, ApiError> {
        self.get_value(&format!("/gain/{}", encode(id))).await
    }

    /// `POST /api/gains` — a whole run of tracks in one call
    /// (`lib/gaincache.js`'s batching).
    pub async fn track_gains<I, S>(&self, ids: I) -> Result<Gains, ApiError>
    where
        I: IntoIterator<Item = S>,
        S: ToString,
    {
        let body = serde_json::json!({ "ids": crate::endpoints::id_list(ids) });
        self.send_json(Method::POST, "/gains", &body).await
    }

    /// `POST /api/analyses` — tempo and style for a run of tracks, measured
    /// server-side once per file.
    pub async fn track_analyses<I, S>(&self, ids: I) -> Result<Analyses, ApiError>
    where
        I: IntoIterator<Item = S>,
        S: ToString,
    {
        let body = serde_json::json!({ "ids": crate::endpoints::id_list(ids) });
        self.send_json(Method::POST, "/analyses", &body).await
    }

    /// `GET /api/analysis/<id>` — one track's verdict, plus `pending` when the
    /// server just queued it.
    pub async fn track_analysis(&self, id: &str) -> Result<TrackAnalysis, ApiError> {
        self.get_value(&format!("/analysis/{}", encode(id))).await
    }

    /// `POST /api/analysis/<id>/live` — the analyser's own summary of how a
    /// track is built (what `lib/genre/construction.js` sends).
    pub async fn analysis_live(&self, id: &str, summary: &Value) -> Result<(), ApiError> {
        let path = format!("/analysis/{}/live", encode(id));
        self.send_none(Method::POST, &path, summary).await
    }

    /// `GET /api/audio/edges/<id>?db=` — where the audio really starts and
    /// stops, for the crossfade's trimming. `ready: false` is a normal answer
    /// for a file that is not archived yet.
    pub async fn audio_edges(&self, id: &str, db: f64) -> Result<AudioEdges, ApiError> {
        self.audio_edges_on(Lane::User, id, db).await
    }

    /// The same route on an explicit lane: the NEXT track's bounds are a
    /// prefetch (`docs/agents/desktop.md`, rule 1 — the current track's answer
    /// never queues behind the next one's).
    pub async fn audio_edges_on(
        &self,
        lane: Lane,
        id: &str,
        db: f64,
    ) -> Result<AudioEdges, ApiError> {
        let path = format!("/audio/edges/{}", encode(id));
        self.fetch(
            lane,
            &format!("{path}{}", query(&[("db", Some(format!("{db}")))])),
        )
        .await
    }

    /// `GET /api/track/<id>/credits` — the people behind one track, by role.
    pub async fn track_credits(&self, id: &str) -> Result<TrackCredits, ApiError> {
        self.get_value(&format!("/track/{}/credits", encode(id)))
            .await
    }

    /// `GET /api/credits/person?name=` — every library title one person worked
    /// on. The path alone (for a link) is [`Api::person_credits_path`].
    pub async fn person_credits(&self, name: &str) -> Result<PersonCredits, ApiError> {
        self.get_value(&Self::person_credits_path(name)).await
    }

    /// The path `/credits/person?name=` — the SPA exposes it apart from the call
    /// (`api.personCreditsPath`) because a search result links to it.
    pub fn person_credits_path(name: &str) -> String {
        format!(
            "/credits/person{}",
            query(&[("name", Some(name.to_owned()))])
        )
    }

    /// `GET /api/radio/track/<id>` — a station seeded on one track.
    pub async fn track_radio(&self, id: &str) -> Result<TrackList, ApiError> {
        self.get_value(&format!("/radio/track/{}", encode(id)))
            .await
    }

    /// `GET /api/radio/artist/<id>` — a station seeded on one artist.
    pub async fn artist_radio(&self, id: &str) -> Result<TrackList, ApiError> {
        self.get_value(&format!("/radio/artist/{}", encode(id)))
            .await
    }

    /// `POST /api/radio/next` — more of a radio already playing. The seed stays
    /// the ORIGINAL one: a radio seeded on its own last title drifts a
    /// neighbour of a neighbour at a time.
    pub async fn radio_next(
        &self,
        seed: &str,
        recent: &[String],
        skipped: &[String],
        exclude: &[String],
    ) -> Result<TrackList, ApiError> {
        let body = serde_json::json!({
            "seed": seed,
            "recent": recent,
            "skipped": skipped,
            "exclude": exclude,
        });
        self.send_json(Method::POST, "/radio/next", &body).await
    }

    /// `GET /api/radio/genre/<style>?d=` — a station anchored on one style of
    /// the tree. `level` is the discovery notch (0–1), `None` for the person's.
    pub async fn genre_radio(
        &self,
        style: &str,
        level: Option<f64>,
    ) -> Result<TrackList, ApiError> {
        let path = format!("/radio/genre/{}", encode(style));
        let suffix = match level {
            Some(level) => query(&[("d", Some(format!("{level}")))]),
            None => String::new(),
        };
        self.get_value(&format!("{path}{suffix}")).await
    }

    /// `POST /api/radio/genre/next` — more of a genre station.
    pub async fn genre_radio_next(
        &self,
        style: &str,
        recent: &[String],
        skipped: &[String],
        exclude: &[String],
    ) -> Result<TrackList, ApiError> {
        let body = serde_json::json!({
            "style": style,
            "recent": recent,
            "skipped": skipped,
            "exclude": exclude,
        });
        self.send_json(Method::POST, "/radio/genre/next", &body)
            .await
    }

    /// `GET /api/youtube/search?q=` — the fallback's hits, shaped as ordinary
    /// tracks. A refusal is `503` + `X-NS-Error`, never an empty list.
    pub async fn youtube_search(&self, q: &str) -> Result<YoutubeSearch, ApiError> {
        self.get_value(&format!(
            "/youtube/search{}",
            query(&[("q", Some(q.to_owned()))])
        ))
        .await
    }

    /// `GET /api/track/<id>/probe` — is this track dead, or was that just a bad
    /// moment?
    pub async fn probe_track(&self, id: &str) -> Result<TrackProbe, ApiError> {
        self.get_value(&format!("/track/{}/probe", encode(id)))
            .await
    }

    /// `GET /api/unavailable` — the tracks the provider no longer has.
    pub async fn unavailable_tracks(&self) -> Result<Unavailable, ApiError> {
        self.get_value("/unavailable").await
    }

    /// `GET /api/replace/candidates/<id>` — what could stand in for a dead
    /// track. `fallbacks` also asks the fallback sources; `q` replaces the
    /// search box's query.
    pub async fn replacement_candidates(
        &self,
        id: &str,
        fallbacks: bool,
        q: &str,
    ) -> Result<ReplaceCandidates, ApiError> {
        let path = format!("/replace/candidates/{}", encode(id));
        let suffix = query(&[
            ("fallbacks", fallbacks.then(|| "1".to_owned())),
            ("q", (!q.is_empty()).then(|| q.to_owned())),
        ]);
        self.get_value(&format!("{path}{suffix}")).await
    }

    /// `GET /api/replace/candidates/<id>?best=<n>` — only the matches safe
    /// enough to apply in one click, best first.
    pub async fn best_replacements(&self, id: &str, n: u32) -> Result<ReplaceCandidates, ApiError> {
        let path = format!("/replace/candidates/{}", encode(id));
        self.get_value(&format!(
            "{path}{}",
            query(&[("best", Some(n.to_string()))])
        ))
        .await
    }

    /// `POST /api/replace` — swap one track for another everywhere. Answers the
    /// job handle: the replacement runs in the background (rule 2).
    pub async fn replace_track(&self, from: &str, to: &str) -> Result<Value, ApiError> {
        let body = serde_json::json!({ "from": from, "to": to });
        self.send_json(Method::POST, "/replace", &body).await
    }

    /// `GET /api/replace/status/<job>` — how the replacement is going.
    pub async fn replace_status(&self, job: &str) -> Result<Value, ApiError> {
        self.get_value(&format!("/replace/status/{}", encode(job)))
            .await
    }

    /// `DELETE /api/track/<id>` — drop a track that is on neither the provider
    /// nor the disk. The server re-checks both and answers `409` if either one
    /// still has it.
    pub async fn delete_track(&self, id: &str) -> Result<(), ApiError> {
        let path = format!("/track/{}", encode(id));
        self.send_none(Method::DELETE, &path, &serde_json::json!({}))
            .await
    }

    /// `POST /api/tracks/delete` — the same for a selection (at most 200): one
    /// background job, each track re-checked in turn.
    pub async fn delete_tracks<I, S>(&self, ids: I) -> Result<Value, ApiError>
    where
        I: IntoIterator<Item = S>,
        S: ToString,
    {
        let body = serde_json::json!({ "ids": crate::endpoints::id_list(ids) });
        self.send_json(Method::POST, "/tracks/delete", &body).await
    }

    /// A GET on the Prefetch lane: data for the screen that comes next. Same
    /// routes, same cache, off the link while somebody waits.
    pub async fn prefetch_get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, ApiError> {
        self.fetch(Lane::Prefetch, path).await
    }
}
