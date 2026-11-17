//! Podcasts: subscriptions, episodes paged by cursor, per-person progress,
//! markers, chapters, transcripts and the server's transcription controls.
//!
//! Rule 2 in this area: starting a transcription (server-side or on this PC) is
//! a JOB, and the UI polls what it returns. `worker/...` routes are T3's and are
//! not part of the app's client.

use reqwest::Method;
use serde_json::Value;

use crate::client::{Api, encode};
use crate::endpoints::query;
use crate::error::ApiError;
use nz_model::{
    EpisodeChapters, EpisodeMarkers, EpisodeTranscript, Marker, PodcastList, PodcastPage,
    PodcastProgress, PodcastSearch, ShowMarkers, TranscribeSettings, TranscribeStatus,
    TranscriptSearch,
};

/// `POST /api/podcast/<id>/auto_archive`: how many episodes the switch queued,
/// and the show's own payload untouched (`serde(flatten)` keeps every field a
/// screen may read next, typed by `nz-model` where it reads one).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PodcastAutoArchive {
    #[serde(default)]
    pub queued: u32,
    #[serde(flatten)]
    pub show: serde_json::Map<String, serde_json::Value>,
}

impl Api {
    /// `GET /api/podcasts` — the shows this person follows.
    pub async fn podcasts(&self) -> Result<PodcastList, ApiError> {
        self.get_value("/podcasts").await
    }

    /// `GET /api/podcast/<id>?limit=&cursor=` — one show and a page of its
    /// episodes. Paged, never the whole archive in one answer.
    pub async fn podcast(
        &self,
        id: &str,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<PodcastPage, ApiError> {
        let path = format!(
            "/podcast/{}{}",
            encode(id),
            query(&[
                ("limit", Some(limit.to_string())),
                ("cursor", cursor.map(str::to_owned)),
            ])
        );
        self.get_value(&path).await
    }

    /// `GET /api/search/podcasts?q=` — shows found in the provider's catalogue.
    pub async fn search_podcasts(&self, q: &str) -> Result<PodcastSearch, ApiError> {
        self.get_value(&format!(
            "/search/podcasts{}",
            query(&[("q", Some(q.to_owned()))])
        ))
        .await
    }

    /// `POST /api/podcasts {"url"}` — subscribe by feed URL.
    pub async fn subscribe_podcast(&self, url: &str) -> Result<Value, ApiError> {
        let body = serde_json::json!({ "url": url });
        self.send_json(Method::POST, "/podcasts", &body).await
    }

    /// `DELETE /api/podcast/<id>` — unsubscribe (the archive stays).
    pub async fn unsubscribe_podcast(&self, id: &str) -> Result<(), ApiError> {
        self.send_none(
            Method::DELETE,
            &format!("/podcast/{}", encode(id)),
            &serde_json::json!({}),
        )
        .await
    }

    /// `POST /api/podcast/<id>/auto_archive` — archive new episodes of this
    /// show as they appear, and queue the ones already listed.
    ///
    /// The answer is the show's payload PLUS `queued` (`neezer/webui`'s
    /// `podcast_auto_archive` returns `{**_channel(c), "queued": n}`, `n` from
    /// `backfill.archive_show`), and the SPA says « N épisodes en file » with it.
    /// This function dropped the body (`send_none`), so a screen could only
    /// re-read the whole show to count what it had just queued.
    pub async fn set_podcast_auto_archive(
        &self,
        id: &str,
        on: bool,
    ) -> Result<PodcastAutoArchive, ApiError> {
        let path = format!("/podcast/{}/auto_archive", encode(id));
        self.send_json(Method::POST, &path, &serde_json::json!({ "on": on }))
            .await
    }

    /// `POST /api/podcast/<id>/transcribe` — this show's transcription settings
    /// (language, auto-start).
    pub async fn set_show_transcribe(&self, id: &str, payload: &Value) -> Result<Value, ApiError> {
        let path = format!("/podcast/{}/transcribe", encode(id));
        self.send_json(Method::POST, &path, payload).await
    }

    /// `GET /api/podcast/progress` — every episode's position, which follows the
    /// person to every device.
    pub async fn podcast_progress(&self) -> Result<PodcastProgress, ApiError> {
        self.get_value("/podcast/progress").await
    }

    /// `POST /api/podcast/progress` — where an episode stands, in seconds.
    ///
    /// Sent on pause, every 30 s while playing, and at quit. Offline, the
    /// caller keeps the last write in `pending_progress` and sends it on
    /// reconnection (last write wins); this function never retries it.
    pub async fn save_podcast_progress(
        &self,
        episode_id: &str,
        position: u32,
        duration: u32,
        finished: bool,
    ) -> Result<(), ApiError> {
        let body = serde_json::json!({
            "episode_id": episode_id,
            "position": position,
            "duration": duration,
            "finished": finished,
        });
        self.send_none(Method::POST, "/podcast/progress", &body)
            .await
    }

    /// `GET /api/podcast/<id>/markers` — every marker of one show, by episode.
    pub async fn show_markers(&self, channel_id: &str) -> Result<ShowMarkers, ApiError> {
        self.get_value(&format!("/podcast/{}/markers", encode(channel_id)))
            .await
    }

    /// `GET /api/podcast/episode/<id>/markers`.
    pub async fn episode_markers(&self, episode_id: &str) -> Result<EpisodeMarkers, ApiError> {
        self.get_value(&format!("/podcast/episode/{}/markers", encode(episode_id)))
            .await
    }

    /// `POST /api/podcast/episode/<id>/markers` — a marker at `position`
    /// MILLISECONDS (a marker's unit; a chapter's `start` is seconds).
    pub async fn add_marker(
        &self,
        episode_id: &str,
        position: u32,
        label: Option<&str>,
    ) -> Result<Marker, ApiError> {
        let path = format!("/podcast/episode/{}/markers", encode(episode_id));
        let body = serde_json::json!({ "position": position, "label": label });
        self.send_json(Method::POST, &path, &body).await
    }

    /// `DELETE /api/podcast/marker/<id>`.
    pub async fn delete_marker(&self, marker_id: &str) -> Result<(), ApiError> {
        self.send_none(
            Method::DELETE,
            &format!("/podcast/marker/{}", encode(marker_id)),
            &serde_json::json!({}),
        )
        .await
    }

    /// `GET /api/podcast/episode/<id>/chapters` — the episode's own chapters
    /// (from its feed), in seconds.
    pub async fn episode_chapters(&self, episode_id: &str) -> Result<EpisodeChapters, ApiError> {
        self.get_value(&format!("/podcast/episode/{}/chapters", encode(episode_id)))
            .await
    }

    /// `GET /api/podcast/episode/<id>/transcript` — a page of segments, narrowed
    /// by `at` (seconds) or `q` (a search inside the episode).
    pub async fn episode_transcript(
        &self,
        episode_id: &str,
        offset: Option<u32>,
        limit: Option<u32>,
        at: Option<f64>,
        q: Option<&str>,
    ) -> Result<EpisodeTranscript, ApiError> {
        let path = format!(
            "/podcast/episode/{}/transcript{}",
            encode(episode_id),
            query(&[
                ("offset", offset.map(|value| value.to_string())),
                ("limit", limit.map(|value| value.to_string())),
                ("at", at.map(|value| value.to_string())),
                ("q", q.map(str::to_owned)),
            ])
        );
        self.get_value(&path).await
    }

    /// `POST /api/podcast/episode/<id>/transcribe` — start transcribing on the
    /// SERVER (a job; the panel polls `episode_transcript`).
    pub async fn start_episode_transcript(
        &self,
        episode_id: &str,
        options: &Value,
    ) -> Result<Value, ApiError> {
        let path = format!("/podcast/episode/{}/transcribe", encode(episode_id));
        self.send_json(Method::POST, &path, options).await
    }

    /// `POST /api/podcast/episode/<id>/transcribe/stop`.
    pub async fn stop_episode_transcript(&self, episode_id: &str) -> Result<(), ApiError> {
        let path = format!("/podcast/episode/{}/transcribe/stop", encode(episode_id));
        self.send_none(Method::POST, &path, &serde_json::json!({}))
            .await
    }

    /// `DELETE /api/podcast/episode/<id>/transcript` — delete a server-side
    /// transcript.
    pub async fn delete_episode_transcript(&self, episode_id: &str) -> Result<(), ApiError> {
        let path = format!("/podcast/episode/{}/transcript", encode(episode_id));
        self.send_none(Method::DELETE, &path, &serde_json::json!({}))
            .await
    }

    /// `GET /api/podcast/<id>/transcript/search?q=` — search inside one show.
    pub async fn show_transcript_search(
        &self,
        channel_id: &str,
        q: &str,
        limit: u32,
    ) -> Result<TranscriptSearch, ApiError> {
        let path = format!(
            "/podcast/{}/transcript/search{}",
            encode(channel_id),
            query(&[
                ("q", Some(q.to_owned())),
                ("limit", Some(limit.to_string()))
            ])
        );
        self.get_value(&path).await
    }

    /// `GET /api/transcripts/search?q=` — search every transcript of the server.
    pub async fn search_transcripts(
        &self,
        q: &str,
        limit: u32,
    ) -> Result<TranscriptSearch, ApiError> {
        let path = format!(
            "/transcripts/search{}",
            query(&[
                ("q", Some(q.to_owned())),
                ("limit", Some(limit.to_string()))
            ])
        );
        self.get_value(&path).await
    }

    /// `GET /api/transcribe/status` — the engine, the queue and the daily budget.
    pub async fn transcribe_status(&self) -> Result<TranscribeStatus, ApiError> {
        self.get_value("/transcribe/status").await
    }

    /// `GET /api/transcribe/settings` — every transcription option, plus a live
    /// engine probe when `probe` is set.
    pub async fn transcribe_settings(&self, probe: bool) -> Result<TranscribeSettings, ApiError> {
        let path = if probe {
            "/transcribe/settings?probe=1"
        } else {
            "/transcribe/settings"
        };
        self.get_value(path).await
    }

    /// `PUT /api/transcribe/settings` — save the transcription options.
    pub async fn save_transcribe_settings(&self, fields: &Value) -> Result<Value, ApiError> {
        self.send_json(Method::PUT, "/transcribe/settings", fields)
            .await
    }
}
