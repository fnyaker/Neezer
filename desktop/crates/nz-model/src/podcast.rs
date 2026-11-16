//! Podcasts: the subscription list, the show page, its episodes, their
//! chapters, this person's markers and their listening positions.

use serde::{Deserialize, Serialize};

use crate::ids::{ChannelId, EpisodeId};
use crate::track::TrackArtist;

/// A show, as `/api/podcasts` lists it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PodcastShow {
    pub id: ChannelId,
    /// The provider's show id (a Deezer number as a string).
    #[serde(default)]
    pub deezer_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Relative server path (`/api/cover/<id>`), or `null`.
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(default)]
    pub subscribed: bool,
    /// A show the server holds privately (a local feed, not a provider's).
    #[serde(default)]
    pub local: bool,
    /// The provider's verdict on the feed (`"ok"`, `"gone"`, …).
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub episode_count: u32,
    #[serde(default)]
    pub archived_count: u32,
    #[serde(default)]
    pub auto_archive: bool,
    #[serde(default)]
    pub auto_transcribe: bool,
    /// `"auto"` or a language code: what a transcription of this show defaults to.
    #[serde(default)]
    pub transcribe_language: Option<String>,
    /// `/api/podcast/<id>` only: how many of its episodes have a transcript.
    #[serde(default)]
    pub transcribed_count: Option<u32>,
}

/// `/api/podcasts` — the subscriptions.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PodcastList {
    #[serde(default)]
    pub podcasts: Vec<PodcastShow>,
}

/// `/api/podcast/<id>?limit=&cursor=` — one show and a page of episodes.
///
/// `next` is the keyset cursor for the following page, already opaque: pass it
/// back as `cursor`. The page is bounded precisely because a show can hold
/// thousands of episodes (rule 3: nothing fans out unbounded).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PodcastPage {
    #[serde(flatten)]
    pub show: PodcastShow,
    #[serde(default)]
    pub episodes: Vec<Episode>,
    #[serde(default)]
    pub next: Option<String>,
}

/// One episode, shaped as a playable row.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Episode {
    pub id: EpisodeId,
    /// The episode's universal id: what `/api/stream/<id>` takes.
    #[serde(default)]
    pub deezer_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Seconds.
    #[serde(default)]
    pub duration: u32,
    #[serde(default)]
    pub channel_id: ChannelId,
    #[serde(default)]
    pub cover: Option<String>,
    /// Unix seconds.
    #[serde(default)]
    pub published: Option<i64>,
    /// The file is on disk, so it plays offline and can be transcribed.
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub unavailable: bool,
    #[serde(default)]
    pub explicit: bool,
    #[serde(default)]
    pub podcast: bool,
    /// The server's own state for the archived file (`"completed"`, …).
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub chapter_count: u32,
    /// The transcript's state, `null` when the episode has none.
    #[serde(default)]
    pub transcript: Option<EpisodeTranscriptInfo>,
    /// The artist/album pair every playable row carries; for an episode both
    /// name the show.
    #[serde(default)]
    pub artist: Option<TrackArtist>,
    #[serde(default)]
    pub album: Option<EpisodeAlbum>,
}

/// The album half of an episode row: the show, as an album would be.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EpisodeAlbum {
    #[serde(default)]
    pub deezer_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub cover: Option<String>,
}

/// What a show page says about one episode's transcript, without the segments.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EpisodeTranscriptInfo {
    /// `"none"`, `"queued"`, `"running"`, `"ready"`, `"error"`.
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    /// 0..1.
    #[serde(default)]
    pub progress: f64,
    #[serde(default)]
    pub segment_count: u32,
    #[serde(default)]
    pub error: Option<TranscriptError>,
}

/// A transcription failure, as the API names it: a machine code, and a message
/// only an admin sees.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TranscriptError {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub message: Option<String>,
}

/// One chapter of an episode (from its ID3 tags or a scanned description).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Chapter {
    #[serde(default)]
    pub index: u32,
    /// Seconds.
    #[serde(default)]
    pub start: f64,
    #[serde(default)]
    pub end: f64,
    #[serde(default)]
    pub title: String,
}

/// `/api/podcast/episode/<id>/chapters`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EpisodeChapters {
    #[serde(default)]
    pub episode_id: EpisodeId,
    #[serde(default)]
    pub chapters: Vec<Chapter>,
    /// The episode's duration in seconds.
    #[serde(default)]
    pub duration: u32,
    #[serde(default)]
    pub archived: bool,
    /// Whether the description was already scanned for chapters.
    #[serde(default)]
    pub scanned: bool,
    /// Where the chapters came from (`"id3"`, `"description"`, …).
    #[serde(default)]
    pub source: Option<String>,
}

/// A personal marker inside one episode.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub episode_id: EpisodeId,
    /// A moment inside the episode, in MILLISECONDS (the row's own unit — the
    /// chapters, above, are in seconds).
    #[serde(default)]
    pub position: u32,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub created: Option<String>,
}

/// `/api/podcast/episode/<id>/markers`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EpisodeMarkers {
    #[serde(default)]
    pub markers: Vec<Marker>,
}

/// `/api/podcast/<id>/markers` — every marker of a show, grouped by episode.
///
/// The map's key is the episode's id as the server spells it (text); it stays a
/// `String` so the payload is read exactly as sent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ShowMarkers {
    #[serde(default)]
    pub markers: std::collections::BTreeMap<String, Vec<Marker>>,
}

/// One listening position, as `/api/podcast/progress` serves it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PodcastPosition {
    /// Seconds into the episode.
    #[serde(default)]
    pub position: u32,
    /// The episode's duration in seconds, as it was when saved.
    #[serde(default)]
    pub duration: u32,
    #[serde(default)]
    pub finished: bool,
    /// Unix seconds of the last save.
    #[serde(default)]
    pub updated: i64,
}

/// `/api/podcast/progress` — every episode this person has started.
///
/// Keyed by the episode's id, which is how the player looks one up before it
/// resumes. Offline, the desktop keeps its own `pending_progress` rows and sends
/// them here when the network comes back (last write wins).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PodcastProgress {
    #[serde(default)]
    pub progress: std::collections::BTreeMap<String, PodcastPosition>,
}

/// A show as a catalogue SEARCH result: provider fields only.
///
/// Taken from `neezer/webui/__init__.py#_podcast_card`: the bench has no
/// provider, so no fixture carries one (a real server's search does).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PodcastCard {
    #[serde(default)]
    pub deezer_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub cover: Option<String>,
    /// The provider's fan count.
    #[serde(default)]
    pub fans: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_show_page_flattens_the_show_and_keeps_its_cursor() {
        let page: PodcastPage = serde_json::from_str(
            r#"{"id":"5ef08406-6ef3-4adf-835b-c9e9927ba70e","deezer_id":"9400001",
                "title":"Les Histoires","description":"Une heure de récits",
                "cover":null,"subscribed":true,"local":false,"status":"ok",
                "episode_count":8,"archived_count":8,"auto_archive":false,
                "auto_transcribe":false,"transcribe_language":"auto",
                "transcribed_count":1,
                "episodes":[{"id":"31254005-fa75-4784-805b-e3ade4d7a65a",
                  "deezer_id":"9410000","title":"Épisode 1","description":"Un épisode",
                  "duration":3600,"channel_id":"5ef08406-6ef3-4adf-835b-c9e9927ba70e",
                  "cover":null,"published":1794748649,"archived":true,"unavailable":false,
                  "explicit":false,"podcast":true,"status":"completed","chapter_count":3,
                  "transcript":null,"artist":null,"album":null}],
                "next":"2026-11-11T14:17:29|2026-11-16T14:17:29|fa7c3091"}"#,
        )
        .expect("show page");
        assert_eq!(page.show.title, "Les Histoires");
        assert_eq!(page.episodes.len(), 1);
        assert!(page.next.is_some());
    }

    #[test]
    fn a_marker_position_is_in_milliseconds() {
        // The row unit, which is NOT the chapters' seconds: getting this wrong
        // puts a marker of 10:00 at 10 seconds.
        let markers: EpisodeMarkers = serde_json::from_str(
            r#"{"markers":[{"id":"9f6","episode_id":"31254005","position":600000,
                "label":"La maison","created":"2026-11-16T14:17:29"}]}"#,
        )
        .expect("markers");
        assert_eq!(markers.markers[0].position, 600_000);
    }
}
