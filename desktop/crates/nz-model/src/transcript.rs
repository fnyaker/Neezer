//! Transcripts: the segments of one episode, the searches, and the server's
//! transcription status/settings.
//!
//! The desktop also transcodes locally (`nz-transcribe`, T1–T4); the types here
//! are only what the SERVER answers, so a local transcript can be shown next to
//! a server one without a second model.

use serde::{Deserialize, Serialize};

use crate::ids::{ChannelId, EpisodeId};
use crate::podcast::TranscriptError;

/// One transcribed line, in seconds from the episode's start.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TranscriptSegment {
    #[serde(default)]
    pub start: f64,
    #[serde(default)]
    pub end: f64,
    #[serde(default)]
    pub text: String,
}

/// `/api/podcast/episode/<id>/transcript?offset=&limit=&at=&q=`.
///
/// `status` is the row's lifecycle (`"none"`, `"queued"`, `"leased"`,
/// `"running"`, `"ready"`, `"error"`). `total` counts the segments the query
/// matches, `next` is the next offset (or `null` on the last page), and `query`
/// echoes the words the server searched for (with `[… highlights …]` around the
/// matches in `text`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EpisodeTranscript {
    #[serde(default)]
    pub episode_id: EpisodeId,
    #[serde(default)]
    pub status: String,
    /// The episode's audio file is on disk (a transcript can exist without it).
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// `"server"` or `"client"`: which machine did the work.
    #[serde(default)]
    pub source: Option<String>,
    /// The enrolled worker's name, for a `client` transcript.
    #[serde(default)]
    pub worker: Option<String>,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub done_ms: u64,
    /// 0..1.
    #[serde(default)]
    pub progress: f64,
    #[serde(default)]
    pub total: u32,
    #[serde(default)]
    pub offset: u32,
    #[serde(default)]
    pub limit: u32,
    /// The words searched for, or `null`.
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub segments: Vec<TranscriptSegment>,
    /// The offset of the next page, `null` when this was the last one.
    #[serde(default)]
    pub next: Option<u32>,
    #[serde(default)]
    pub error: Option<TranscriptError>,
}

/// One episode a transcript search hit.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TranscriptEpisodeRef {
    #[serde(default)]
    pub id: EpisodeId,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub cover: Option<String>,
    /// Unix seconds.
    #[serde(default)]
    pub published: Option<i64>,
}

/// One hit of a transcript search: the line, its moment, its episode.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TranscriptHit {
    #[serde(default)]
    pub idx: u32,
    #[serde(default)]
    pub start: f64,
    #[serde(default)]
    pub end: f64,
    /// The line, with the matched words highlighted by the server.
    #[serde(default)]
    pub text: String,
    /// The server's own relevance score.
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub episode_id: Option<EpisodeId>,
    /// `null` when the episode became unreadable between the two reads.
    #[serde(default)]
    pub episode: Option<TranscriptEpisodeRef>,
    #[serde(default)]
    pub channel_id: Option<ChannelId>,
    #[serde(default)]
    pub channel_title: Option<String>,
}

/// How much of the library a transcript search looked at.
///
/// The show-level search names the episodes it scanned; the library-wide one
/// also names the segments it had a budget for (`limit_segments`) and how many
/// it actually scanned (`segments`). Both live in one struct because both are
/// the same `scope` object with a different subset filled.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TranscriptScope {
    #[serde(default)]
    pub episodes: u32,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub segments: Option<u32>,
    #[serde(default)]
    pub limit_segments: Option<u32>,
}

/// `/api/podcast/<id>/transcript/search` and `/api/transcripts/search`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TranscriptSearch {
    #[serde(default)]
    pub q: String,
    #[serde(default)]
    pub hits: Vec<TranscriptHit>,
    #[serde(default)]
    pub total: u32,
    /// More hits exist than the page holds.
    #[serde(default)]
    pub more: bool,
    #[serde(default)]
    pub scope: TranscriptScope,
}

/// One episode in the transcription queue's counters.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TranscribeQueue {
    #[serde(default)]
    pub queued: u32,
    #[serde(default)]
    pub leased: u32,
    #[serde(default)]
    pub running: u32,
    #[serde(default)]
    pub ready: u32,
    #[serde(default)]
    pub error: u32,
    #[serde(default)]
    pub stopped: u32,
    #[serde(default)]
    pub budget_limit_ms: u64,
    #[serde(default)]
    pub budget_used_ms: u64,
}

/// How many enrolled devices may claim a queued episode, and how many of them
/// claimed or heartbeat recently. The same shape in `/api/transcribe/status`
/// (any account, counts only) and `/api/transcribe/settings` (admin).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TranscribeWorkers {
    #[serde(default)]
    pub enrolled: u32,
    #[serde(default)]
    pub online: u32,
}

/// One enrolled device as the admin card names it (`/api/transcribe/settings`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TranscribeMachine {
    #[serde(default)]
    pub name: String,
    /// `"client"` (a person's PC) or `"server"` (the sidecar container).
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub online: bool,
    #[serde(default)]
    pub last_seen: Option<String>,
}

/// `/api/transcribe/status` — what the server's transcription is doing.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TranscribeStatus {
    #[serde(default)]
    pub available: bool,
    #[serde(default)]
    pub enabled: bool,
    /// `"server"` or `"external"`: whether this server runs the engine itself or
    /// waits for an enrolled worker (the desktop can be that worker).
    #[serde(default)]
    pub runner: Option<String>,
    /// Why transcription is off, when it is (`"no_worker"`, `"disabled"`, …).
    #[serde(default)]
    pub reason: Option<String>,
    /// Where a queued episode will run: `"server"`, `"external"` or `"workers"`.
    #[serde(default)]
    pub destination: Option<String>,
    #[serde(default)]
    pub workers: TranscribeWorkers,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub daily_minutes: u32,
    #[serde(default)]
    pub max_minutes: u32,
    #[serde(default)]
    pub search_segments: u64,
    #[serde(default)]
    pub queue: TranscribeQueue,
    #[serde(default)]
    pub last_error: Option<TranscriptError>,
}

/// The engine's own verdict (status/settings show it).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TranscribeEngine {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub version: Option<String>,
}

/// The model the server would use, and its size on disk.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TranscribeModel {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub size_mb: u32,
}

/// `/api/transcribe/settings` — the admin's controls.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TranscribeSettings {
    #[serde(default)]
    pub available: bool,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub runner: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    /// Where a queued episode will run: `"server"`, `"external"` or `"workers"`.
    #[serde(default)]
    pub destination: Option<String>,
    #[serde(default)]
    pub workers: TranscribeWorkers,
    /// The enrolled devices, named — the admin card alone reads this list.
    #[serde(default)]
    pub machines: Vec<TranscribeMachine>,
    #[serde(default)]
    pub language: String,
    /// The languages the picker offers, `"auto"` first.
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub daily_minutes: u32,
    #[serde(default)]
    pub max_minutes: u32,
    #[serde(default)]
    pub threads: u32,
    #[serde(default)]
    pub search_segments: u64,
    #[serde(default)]
    pub queue: TranscribeQueue,
    #[serde(default)]
    pub engine: Option<TranscribeEngine>,
    #[serde(default)]
    pub model: Option<TranscribeModel>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transcript_page_carries_its_own_cursor_and_status() {
        let page: EpisodeTranscript = serde_json::from_str(
            r#"{"archived":true,"done_ms":3600000,"duration_ms":3600000,
                "engine":"whisper.cpp","episode_id":"31254005","error":null,
                "language":"fr","limit":5,"model":"ggml-small.bin","next":5,
                "offset":0,"progress":1.0,"query":null,
                "segments":[{"end":15.0,"start":0.0,"text":"Ligne 1."}],
                "source":"server","status":"ready","total":240,"worker":null}"#,
        )
        .expect("transcript page");
        assert_eq!(page.status, "ready");
        assert_eq!(page.next, Some(5));
        assert_eq!(page.segments[0].end, 15.0);
    }

    #[test]
    fn an_unknown_transcript_is_a_status_not_an_error() {
        let page: EpisodeTranscript = serde_json::from_str(
            r#"{"episode_id":"31254005","status":"none","archived":false,"language":null,
                "engine":null,"model":null,"source":null,"worker":null,
                "duration_ms":0,"done_ms":0,"progress":0.0,"total":0,"offset":0,
                "limit":50,"query":null,"segments":[],"next":null}"#,
        )
        .expect("no transcript");
        assert_eq!(page.status, "none");
        assert!(page.error.is_none());
    }

    /// The server's settings card now says WHERE a job will run and names the
    /// enrolled PCs; a server one version behind (no `destination`/`workers`)
    /// must still parse, and a status payload (counts, no machines) must too.
    #[test]
    fn the_settings_and_status_say_where_a_job_will_run() {
        let settings: TranscribeSettings = serde_json::from_str(
            r#"{"available":true,"enabled":true,"runner":"inline","reason":null,
                "destination":"workers","workers":{"enrolled":2,"online":1},
                "machines":[{"name":"Bureau","kind":"client","online":true,
                             "last_seen":"2026-11-23T10:00:00"},
                            {"name":"Portable","kind":"client","online":false,
                             "last_seen":null}],
                "language":"fr","languages":["auto","fr"],"max_minutes":360,
                "daily_minutes":120,"search_segments":1200000,"threads":4,
                "queue":{"queued":1},"engine":{"name":"whisper.cpp","ok":false},
                "model":{"name":"ggml-small.bin","ok":false,"size_mb":0}}"#,
        )
        .expect("settings payload");
        assert_eq!(settings.destination.as_deref(), Some("workers"));
        assert_eq!(settings.workers.enrolled, 2);
        assert_eq!(settings.workers.online, 1);
        assert_eq!(settings.machines.len(), 2);
        assert!(!settings.machines[1].online);

        let status: TranscribeStatus = serde_json::from_str(
            r#"{"enabled":true,"available":true,"reason":null,"runner":"inline",
                "destination":"workers","workers":{"enrolled":1,"online":0},
                "language":"fr","model":null,"daily_minutes":120,"max_minutes":360,
                "search_segments":1200000,"queue":{},"last_error":null}"#,
        )
        .expect("status payload");
        assert_eq!(status.destination.as_deref(), Some("workers"));
        assert_eq!(status.workers.online, 0);

        let old: TranscribeSettings = serde_json::from_str(
            r#"{"available":true,"enabled":false,"reason":null,"language":"fr",
                "languages":[],"max_minutes":360,"daily_minutes":120,
                "search_segments":1200000,"threads":0,"queue":{}}"#,
        )
        .expect("a server one version behind");
        assert!(old.destination.is_none());
        assert_eq!(old.workers.enrolled, 0);
        assert!(old.machines.is_empty());
    }
}
