//! The wire: every route this area reads, and the counter that says what it
//! spent.
//!
//! Each call goes through `nz-api`'s TYPED endpoints, which run on the `User`
//! lane — a show being opened is the action just taken, never behind a prefetch
//! or a download (rule 1). Nothing here decides a rule: the shapes are
//! `nz-model`'s (the same records the desktop decodes) and the SPA's own
//! component calls (`routes/Podcasts.svelte`, `routes/Show.svelte`,
//! `lib/markers.js`, `lib/chapters.js`, `lib/transcript.js`) name which route
//! each one is.
//!
//! The traffic is booked per request in the core's meter under
//! [`TAG_PODCASTS`](super::TAG_PODCASTS), so `NzCore::stats()` answers what a
//! podcast screen cost without the UI counting anything.

use std::sync::Arc;

use nz_api::{Api, Lane, Method, PodcastAutoArchive};
use nz_model::{
    EpisodeMarkers, EpisodeTranscript, Marker, PodcastList, PodcastPage, PodcastProgress,
    PodcastSearch, ShowMarkers, TranscribeSettings, TranscribeStatus, TranscriptSearch,
};
use serde::Deserialize;
use serde_json::Value;

use super::TAG_PODCASTS;
use crate::core::{Core, CoreError};

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
        .find(|row| row.tag == TAG_PODCASTS)
        .map(|row| Totals {
            requests: row.requests,
            bytes_in: row.bytes_in,
            bytes_out: row.bytes_out,
        })
        .unwrap_or_default()
}

// --- the subscriptions -------------------------------------------------------

/// `GET /api/podcasts` — the shows this server holds.
pub(super) async fn fetch_shows(api: &Api) -> Result<PodcastList, CoreError> {
    Ok(api.podcasts().await?)
}

/// `GET /api/search/podcasts?q=` — the provider's catalogue.
pub(super) async fn search_podcasts(api: &Api, q: &str) -> Result<PodcastSearch, CoreError> {
    Ok(api.search_podcasts(q).await?)
}

/// `POST /api/podcasts {url}` — the answer is the whole show page, episodes
/// included (`_channel(c, with_episodes=True)`), so a subscribe is ONE request
/// and the page it lands on needs none.
pub(super) async fn subscribe(api: &Api, url: &str) -> Result<PodcastPage, CoreError> {
    let answer: Value = api.subscribe_podcast(url).await?;
    serde_json::from_value(answer).map_err(|error| CoreError::Internal {
        detail: format!("podcast souscrit illisible : {error}"),
    })
}

/// `DELETE /api/podcast/<id>` — the archive stays.
pub(super) async fn unsubscribe(api: &Api, id: &str) -> Result<(), CoreError> {
    Ok(api.unsubscribe_podcast(id).await?)
}

/// `GET /api/podcast/<id>?limit=&cursor=` — one page of its episodes.
pub(super) async fn fetch_show(
    api: &Api,
    id: &str,
    limit: u32,
    cursor: Option<&str>,
) -> Result<PodcastPage, CoreError> {
    Ok(api.podcast(id, limit, cursor).await?)
}

/// `POST /api/podcast/<id>/auto_archive` — the show plus what it queued.
pub(super) async fn auto_archive(
    api: &Api,
    id: &str,
    on: bool,
) -> Result<PodcastAutoArchive, CoreError> {
    Ok(api.set_podcast_auto_archive(id, on).await?)
}

/// `POST /api/podcast/<id>/transcribe` — the show's language and auto-start.
/// The answer carries the show, the backlog mode, `queued` and `skipped`, which
/// the screen says out loud (rule 2: a job says what it did).
pub(super) async fn set_show_transcribe(
    api: &Api,
    id: &str,
    payload: &Value,
) -> Result<Value, CoreError> {
    Ok(api.set_show_transcribe(id, payload).await?)
}

// --- the person's positions and markers --------------------------------------

/// `GET /api/podcast/progress` — every episode this person has started.
pub(super) async fn fetch_progress(api: &Api) -> Result<PodcastProgress, CoreError> {
    Ok(api.podcast_progress().await?)
}

/// `GET /api/podcast/<id>/markers` — every marker of a show, by episode.
pub(super) async fn show_markers(api: &Api, channel_id: &str) -> Result<ShowMarkers, CoreError> {
    Ok(api.show_markers(channel_id).await?)
}

/// `GET /api/podcast/episode/<id>/markers`.
pub(super) async fn episode_markers(
    api: &Api,
    episode_id: &str,
) -> Result<EpisodeMarkers, CoreError> {
    Ok(api.episode_markers(episode_id).await?)
}

/// `POST /api/podcast/episode/<id>/markers` — `position` in MILLISECONDS.
///
/// Read through [`Api::send`] rather than `nz-api`'s typed `add_marker`: the
/// route answers `{"ok": true, "marker": {…}}` (`webui/__init__.py`'s
/// `add_episode_marker`), and the typed function decodes the whole body as the
/// marker itself — so every field came back default and a marker could never be
/// deleted afterwards (its id was empty). The shape below is the server's, and
/// the sentence is in this lot's section of the note so the shared crate can be
/// fixed on its own.
pub(super) async fn add_marker(
    api: &Api,
    episode_id: &str,
    position_ms: u32,
    label: Option<&str>,
) -> Result<Marker, CoreError> {
    let body = serde_json::json!({ "position": position_ms, "label": label });
    let path = format!("/podcast/episode/{}/markers", encode_path(episode_id));
    let answer: Option<RawMarkerAnswer> = api
        .send(Lane::User, Method::POST, &path, Some(&body))
        .await?;
    let answer = answer.ok_or_else(|| CoreError::Internal {
        detail: "le serveur n'a pas renvoyé le marqueur créé".to_owned(),
    })?;
    Ok(Marker {
        id: answer.marker.id,
        episode_id: if answer.marker.episode_id.is_empty() {
            episode_id.to_owned()
        } else {
            answer.marker.episode_id
        }
        .into(),
        position: answer.marker.position,
        label: answer.marker.label.unwrap_or_default(),
        created: None,
    })
}

/// `encodeURIComponent`, byte for byte — the ids this area puts in a path are
/// the server's own (`nz-api`'s `encode` is crate-private, and a route argument
/// must never be able to add a segment to the path).
fn encode_path(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
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
            | b')' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The route's own answer for a new marker (`{"ok": true, "marker": {…}}`).
#[derive(Debug, Clone, Deserialize)]
struct RawMarkerAnswer {
    #[serde(default)]
    marker: RawMarker,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawMarker {
    #[serde(default)]
    id: String,
    #[serde(default)]
    episode_id: String,
    #[serde(default)]
    position: u32,
    #[serde(default)]
    label: Option<String>,
}

/// `DELETE /api/podcast/marker/<id>`.
pub(super) async fn delete_marker(api: &Api, marker_id: &str) -> Result<(), CoreError> {
    Ok(api.delete_marker(marker_id).await?)
}

// --- chapters ----------------------------------------------------------------

/// `GET /api/podcast/episode/<id>/chapters` — seconds, from the archived file.
///
/// Read as bytes rather than through `nz-api`'s typed `episode_chapters`: the
/// route answers `"end": null` for a chapter that does not declare one
/// (`podcast_chapters.py`'s `_payload`), and `nz-model::Chapter::end` is an
/// `f64`, so the typed decode refuses the whole answer. The tolerant shape is
/// this route's, and the cause is in the lot's section of the note so the
/// shared crate can be corrected without this lot touching it.
pub(super) async fn chapters(api: &Api, episode_id: &str) -> Result<ChaptersAnswer, CoreError> {
    let path = format!("/podcast/episode/{}/chapters", encode_path(episode_id));
    let answer = api.get::<RawChapters>(Lane::User, &path).await?;
    let answer = answer.value;
    Ok(ChaptersAnswer {
        episode_id: if answer.episode_id.is_empty() {
            episode_id.to_owned()
        } else {
            answer.episode_id
        },
        chapters: answer
            .chapters
            .into_iter()
            .map(|chapter| (chapter.index, chapter.start, chapter.end, chapter.title))
            .collect(),
        duration_s: answer.duration,
        archived: answer.archived,
        scanned: answer.scanned,
        source: answer.source,
    })
}

/// One episode's chapters as the route sends them: `end` is an `Option`, which
/// is the whole point of not decoding `nz-model::Chapter` here.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct ChaptersAnswer {
    pub episode_id: String,
    /// `(index, start, end, title)` in SECONDS, `end` possibly absent.
    pub chapters: Vec<(u32, f64, Option<f64>, String)>,
    pub duration_s: u32,
    pub archived: bool,
    pub scanned: bool,
    pub source: Option<String>,
}

/// `/api/podcast/episode/<id>/chapters`, tolerantly: `end` may be `null`, an
/// integer or a float, and an unarchived episode holds no chapters at all.
#[derive(Debug, Clone, Default, Deserialize)]
struct RawChapters {
    #[serde(default)]
    episode_id: String,
    #[serde(default)]
    chapters: Vec<RawChapter>,
    #[serde(default)]
    duration: u32,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    scanned: bool,
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawChapter {
    #[serde(default)]
    index: u32,
    #[serde(default)]
    start: f64,
    #[serde(default)]
    end: Option<f64>,
    #[serde(default)]
    title: String,
}

// --- transcripts -------------------------------------------------------------

/// `GET /api/podcast/episode/<id>/transcript` — one page, by offset or by `at`,
/// or the lines that match `q`.
pub(super) async fn transcript(
    api: &Api,
    episode_id: &str,
    offset: Option<u32>,
    limit: Option<u32>,
    at: Option<f64>,
    q: Option<&str>,
) -> Result<EpisodeTranscript, CoreError> {
    Ok(api
        .episode_transcript(episode_id, offset, limit, at, q)
        .await?)
}

/// `GET /api/podcast/<id>/transcript/search?q=` — inside one show.
pub(super) async fn search_show_transcripts(
    api: &Api,
    channel_id: &str,
    q: &str,
    limit: u32,
) -> Result<TranscriptSearch, CoreError> {
    Ok(api.show_transcript_search(channel_id, q, limit).await?)
}

/// `GET /api/transcribe/settings` — availability, languages, the queue.
pub(super) async fn transcribe_settings(
    api: &Api,
    probe: bool,
) -> Result<TranscribeSettings, CoreError> {
    Ok(api.transcribe_settings(probe).await?)
}

/// `GET /api/transcribe/status` — the engine’s queue and its daily budget.
pub(super) async fn transcribe_status(api: &Api) -> Result<TranscribeStatus, CoreError> {
    Ok(api.transcribe_status().await?)
}

/// `POST /api/podcast/episode/<id>/transcribe` — a server JOB (rule 2): it
/// answers at once and the panel polls the transcript.
pub(super) async fn start_transcript(
    api: &Api,
    episode_id: &str,
    options: &Value,
) -> Result<(), CoreError> {
    let _: Value = api.start_episode_transcript(episode_id, options).await?;
    Ok(())
}

/// `POST /api/podcast/episode/<id>/transcribe/stop`.
pub(super) async fn stop_transcript(api: &Api, episode_id: &str) -> Result<(), CoreError> {
    Ok(api.stop_episode_transcript(episode_id).await?)
}

/// `DELETE /api/podcast/episode/<id>/transcript`.
pub(super) async fn delete_transcript(api: &Api, episode_id: &str) -> Result<(), CoreError> {
    Ok(api.delete_episode_transcript(episode_id).await?)
}

impl Core {
    /// The tagged client of one podcasts read. The offline gate is the core's,
    /// so a dead link answers `Offline` in zero sockets.
    pub(super) fn podcasts_api(&self) -> Result<Arc<Api>, CoreError> {
        Ok(Arc::new(self.api()?.with_tag(TAG_PODCASTS)))
    }

    /// Books one read's traffic in the core's meter, so `NzCore::stats()`
    /// answers what this screen cost.
    pub(super) fn note_podcasts_traffic(&self, api: &Api, before: Totals) {
        let after = totals(api);
        let requests = after.requests.saturating_sub(before.requests);
        if requests == 0 {
            return;
        }
        self.meter().record(
            TAG_PODCASTS,
            after.bytes_in.saturating_sub(before.bytes_in),
            after.bytes_out.saturating_sub(before.bytes_out),
        );
        for _ in 1..requests {
            self.meter().record(TAG_PODCASTS, 0, 0);
        }
    }
}
