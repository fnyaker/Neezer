//! The server's worker protocol: routes, payloads, backoff — owned by lot T3.
//!
//! T1 pins the routes and the one guard every per-episode route needs. The paths
//! are server facts (`neezer/webui/transcribe_worker.py`), and the guard exists
//! because an episode id is interpolated into a path: the id comes from the
//! server's own `claim` answer, but "comes from the server" is not a validation,
//! and a client-supplied string that reaches a URL must first be known to be an
//! id and not `../../settings` (CLAUDE.md, *Security*).
//!
//! T3 adds the payloads and the retry policy. Both are ports of
//! `docker/transcribe/worker.py` — the container's worker is the reference
//! client, and a laptop that behaved differently (a different retry schedule, a
//! different `fail` truncation) would be two clients the server has to tolerate
//! instead of one it is written for.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::backend::FORMAT_ENGINE;
use crate::segments::{CHUNK_SECONDS, OVERLAP_SECONDS, Segment};

/// `POST`: lease the next queued episode; `204` means the queue is empty.
pub const CLAIM: &str = "/api/transcribe/worker/claim";

/// The lease header of every per-episode route.
pub const LEASE_HEADER: &str = "X-NS-Lease";

/// Seconds between two claims when the queue is empty (`worker.POLL_SECONDS`).
pub const POLL_SECONDS: f64 = 5.0;

/// Attempts per request, the first one included (`worker.NEEZER_RETRY_ATTEMPTS`).
pub const RETRY_ATTEMPTS: u32 = 5;

/// First backoff step, in seconds (`worker.NEEZER_RETRY_BASE`).
pub const RETRY_BASE_SECONDS: f64 = 1.0;

/// Largest backoff step, in seconds (`worker.NEEZER_RETRY_MAX`).
pub const RETRY_MAX_SECONDS: f64 = 30.0;

/// A `Retry-After` above this many seconds is not honoured
/// (`worker.Client._backoff`: `min(120.0, retry_after)`).
pub const RETRY_AFTER_CAP_SECONDS: f64 = 120.0;

/// Connect timeout of an API call (`worker.Client.request`: `timeout=(10, 120)`).
pub const CONNECT_TIMEOUT_SECONDS: u64 = 10;

/// Read timeout of an API call, in seconds.
pub const READ_TIMEOUT_SECONDS: u64 = 120;

/// Read timeout of the audio body (`worker.NEEZER_AUDIO_TIMEOUT`), in seconds:
/// a podcast episode can be 150 MB and a slow link still has to finish it.
pub const AUDIO_TIMEOUT_SECONDS: u64 = 120;

/// Largest episode audio this client accepts (`worker.MAX_AUDIO_BYTES`).
pub const MAX_AUDIO_BYTES: u64 = 1 << 30;

/// Downloads one episode may take before giving up (`worker.download_audio`).
pub const MAX_AUDIO_REQUESTS: u32 = 20;

/// Bytes of segment text a chunk may carry (`worker.fit_segments`): the server's
/// `POST …/chunk` body is capped at 1 MiB, and 800 kB leaves room for the
/// envelope on a chunk of 2 000 lines.
pub const SEGMENT_BUDGET_BYTES: usize = 800_000;

/// `GET …/audio/<eid>` with `Authorization: Bearer <token>` and `X-NS-Lease`;
/// byte ranges supported, which is how a resumed job skips what it already has.
pub fn audio(episode_id: &str) -> Option<String> {
    if !is_episode_id(episode_id) {
        return None;
    }
    Some(format!("/api/transcribe/worker/audio/{episode_id}"))
}

/// `POST …/heartbeat`: `{chunk_pct}` in, `{cancel, pause, done_ms, expires_at}` out.
pub fn heartbeat(episode_id: &str) -> Option<String> {
    episode_path(episode_id, "/heartbeat")
}

/// `POST …/chunk`: one chunk's `{start_ms, duration_ms, segments, language,
/// engine, model, engine_version}`; the server answers `{applied, done_ms,
/// n_segments}` and refuses a chunk that is not the next one.
pub fn chunk(episode_id: &str) -> Option<String> {
    episode_path(episode_id, "/chunk")
}

/// `POST …/complete`: the last chunk landed, the transcript becomes `ready`.
pub fn complete(episode_id: &str) -> Option<String> {
    episode_path(episode_id, "/complete")
}

/// `POST …/fail`: `{code, message}`, the cause the person will read.
pub fn fail(episode_id: &str) -> Option<String> {
    episode_path(episode_id, "/fail")
}

/// `POST …/release`: give the lease back without a verdict (a pause, a shutdown).
pub fn release(episode_id: &str) -> Option<String> {
    episode_path(episode_id, "/release")
}

/// `/api/transcribe/worker/<eid><suffix>`, or `None` when the id is not one.
fn episode_path(episode_id: &str, suffix: &str) -> Option<String> {
    if !is_episode_id(episode_id) {
        return None;
    }
    Some(format!("/api/transcribe/worker/{episode_id}{suffix}"))
}

/// Whether `value` is a UUID the server would accept as an episode id.
///
/// The server's own check is `uuid.UUID(str(value))`, which takes the canonical
/// hyphenated form (`8-4-4-4-12`) and the bare 32-hex one. This accepts exactly
/// those two shapes and nothing else — in particular no `/`, no `.` and no NUL,
/// so the value cannot escape the path segment it is written into.
pub fn is_episode_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    match bytes.len() {
        36 => {
            [8, 13, 18, 23].iter().all(|index| bytes[*index] == b'-')
                && bytes.iter().enumerate().all(|(index, byte)| {
                    [8, 13, 18, 23].contains(&index) || byte.is_ascii_hexdigit()
                })
        }
        32 => bytes.iter().all(u8::is_ascii_hexdigit),
        _ => false,
    }
}

/// `POST …/claim`'s body, field for field `transcribe_worker_claim`'s answer.
///
/// Every field but the two the protocol cannot work without is `#[serde(default)]`:
/// the route grew (`chunk_ms`, `overlap_ms`, `max_minutes`, `expires_at`) and a
/// server one version behind must still be usable, while a claim without a lease
/// or an episode is refused by [`Claim::checked`] before a byte of it is used.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Claim {
    /// The licensed episode, always a UUID string.
    pub episode_id: String,
    /// The lease token proving this worker owns the episode.
    pub lease: String,
    /// `…/audio/<eid>`, the one route the audio comes from.
    #[serde(default)]
    pub audio_url: String,
    /// The episode's duration, in milliseconds; `0` means the server cannot say.
    #[serde(default)]
    pub duration_ms: i64,
    /// The language the server wants, or absent / `auto`.
    #[serde(default)]
    pub language: Option<String>,
    /// Audio per chunk, in milliseconds; the server's value wins over ours.
    #[serde(default)]
    pub chunk_ms: i64,
    /// Milliseconds a chunk shares with the previous one.
    #[serde(default)]
    pub overlap_ms: i64,
    /// The per-episode cap the queue applied, in minutes.
    #[serde(default)]
    pub max_minutes: i64,
    /// Where the server's own rows already are: a resumed job starts there.
    #[serde(default)]
    pub start_ms: i64,
    /// When the lease expires, informational (`heartbeat` refreshes it).
    #[serde(default)]
    pub expires_at: Option<String>,
}

impl Claim {
    /// The server's answer, checked before it is used.
    ///
    /// Three facts are load-bearing. The id must be a UUID, because it becomes a
    /// path segment ([`is_episode_id`]). The lease must be a single non-space
    /// token, because it becomes a header value. And `audio_url` must be the
    /// audio route of THAT episode: the server sends it, so it is data, not a
    /// command — a compromised answer must not be able to point this client at
    /// another host or another episode (CLAUDE.md, *Security*, SSRF).
    pub fn checked(self) -> Result<Claim, String> {
        if !is_episode_id(&self.episode_id) {
            return Err("l'identifiant d'épisode du claim n'est pas un UUID".to_owned());
        }
        let lease = self.lease.trim();
        if lease.is_empty() || lease.len() > 200 || lease.chars().any(char::is_whitespace) {
            return Err("le lease du claim est invalide".to_owned());
        }
        let expected = audio(&self.episode_id).unwrap_or_default();
        if self.audio_url != expected {
            return Err(format!(
                "l'URL d'audio du claim n'est pas celle de l'épisode ({})",
                self.audio_url
            ));
        }
        Ok(self)
    }

    /// Audio per chunk in milliseconds: the server's `chunk_ms` when it sends
    /// one, [`CHUNK_SECONDS`] otherwise.
    pub fn chunk_ms(&self) -> i64 {
        if self.chunk_ms > 0 {
            self.chunk_ms
        } else {
            CHUNK_SECONDS * 1000
        }
    }

    /// Milliseconds a chunk shares with the previous one: the server's value
    /// when it sends one, [`OVERLAP_SECONDS`] otherwise.
    pub fn overlap_ms(&self) -> i64 {
        if self.overlap_ms > 0 {
            self.overlap_ms
        } else {
            OVERLAP_SECONDS * 1000
        }
    }

    /// The language to transcribe in: the claim's, unless this machine forces
    /// one, and `auto` when nobody says.
    pub fn language_or(&self, forced: Option<&str>) -> Option<String> {
        normalize_language(forced).or_else(|| normalize_language(self.language.as_deref()))
    }
}

/// `POST …/heartbeat`'s answer: the server's decisions and how far it already is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Beat {
    /// The person asked the server to stop this transcription.
    #[serde(default)]
    pub cancel: bool,
    /// The server wants the engine paused (server workers only, honoured anyway).
    #[serde(default)]
    pub pause: bool,
    /// The server's own view of how much is transcribed.
    #[serde(default)]
    pub done_ms: i64,
    /// The refreshed lease expiry.
    #[serde(default)]
    pub expires_at: Option<String>,
}

/// `POST …/<eid>/chunk`'s body: one chunk's lines, in absolute milliseconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChunkBody {
    /// The chunk's first millisecond.
    pub start_ms: i64,
    /// Its length.
    pub duration_ms: i64,
    /// The language the engine heard.
    pub language: String,
    /// [`FORMAT_ENGINE`].
    pub engine: String,
    /// The model's file name, which is what the server stores.
    pub model: String,
    /// The engine's version string, cut at 32 characters server-side.
    pub engine_version: String,
    /// The lines, absolute and already cleaned.
    pub segments: Vec<Segment>,
}

impl ChunkBody {
    /// A chunk body stamped with [`FORMAT_ENGINE`].
    pub fn new(
        start_ms: i64,
        duration_ms: i64,
        language: impl Into<String>,
        model: impl Into<String>,
        engine_version: impl Into<String>,
        segments: Vec<Segment>,
    ) -> ChunkBody {
        ChunkBody {
            start_ms,
            duration_ms,
            language: language.into(),
            engine: FORMAT_ENGINE.to_owned(),
            model: model.into(),
            engine_version: engine_version.into(),
            segments,
        }
    }
}

/// `POST …/<eid>/chunk`'s answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ChunkReply {
    /// False when the chunk was not the next one (a retry after a lost answer).
    #[serde(default)]
    pub applied: bool,
    /// How far the server now is, in milliseconds.
    #[serde(default)]
    pub done_ms: i64,
    /// How many lines the server holds.
    #[serde(default)]
    pub n_segments: i64,
}

/// `POST …/<eid>/complete`'s body: what the transcript is stamped with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompleteBody {
    /// The language of the whole episode.
    pub language: Option<String>,
    /// [`FORMAT_ENGINE`].
    pub engine: String,
    /// The model's file name.
    pub model: String,
    /// The engine's version string.
    pub engine_version: String,
}

impl CompleteBody {
    /// A completion body stamped with [`FORMAT_ENGINE`].
    pub fn new(
        language: Option<String>,
        model: impl Into<String>,
        engine_version: impl Into<String>,
    ) -> CompleteBody {
        CompleteBody {
            language,
            engine: FORMAT_ENGINE.to_owned(),
            model: model.into(),
            engine_version: engine_version.into(),
        }
    }
}

/// `POST …/<eid>/complete`'s answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct CompleteReply {
    /// `ready` when the transcript is whole.
    #[serde(default)]
    pub status: String,
    /// The final position, in milliseconds.
    #[serde(default)]
    pub done_ms: i64,
    /// The number of lines stored.
    #[serde(default)]
    pub n_segments: i64,
}

impl CompleteReply {
    /// Whether the server now holds a complete transcript.
    pub fn ready(&self) -> bool {
        self.status == "ready"
    }
}

/// `POST …/<eid>/fail`'s body: the cause the person will read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FailBody {
    /// At most 24 characters (`job.py#fail` truncates there).
    pub code: String,
    /// At most 240 characters (`worker.py#_fail`).
    pub message: String,
}

impl FailBody {
    /// The server's two limits, applied here so the stored cause is the whole
    /// cause and not a 400.
    pub fn new(code: impl AsRef<str>, message: impl AsRef<str>) -> FailBody {
        FailBody {
            code: truncate(code.as_ref(), 24),
            message: truncate(message.as_ref(), 240),
        }
    }
}

/// The delay before attempt `attempt + 1`, as `worker.Client._backoff` computes
/// it: `min(RETRY_MAX, RETRY_BASE × 2^(attempt−1))`, raised to a `Retry-After`
/// the server sent (capped at [`RETRY_AFTER_CAP_SECONDS`]).
///
/// The server's `Retry-After` wins over our own schedule: it is the one party
/// that knows when it will be ready again, and asking sooner is the storm this
/// schedule exists to avoid.
pub fn backoff(attempt: u32, retry_after: Option<&str>) -> Duration {
    let exponent = attempt.saturating_sub(1).min(20) as i32;
    let mut seconds = (RETRY_BASE_SECONDS * 2f64.powi(exponent)).min(RETRY_MAX_SECONDS);
    if let Some(raw) = retry_after
        && let Ok(value) = raw.trim().parse::<f64>()
        && value.is_finite()
    {
        seconds = seconds.max(value.clamp(0.0, RETRY_AFTER_CAP_SECONDS));
    }
    Duration::from_secs_f64(seconds.max(0.0))
}

/// Whether a status is worth another attempt: a rate limit or a server fault.
///
/// `404`, `409` and `403` are answers, not accidents: retrying them would only
/// repeat a decision the server already made (`worker.Client.request`).
pub fn is_transient(status: u16) -> bool {
    status == 429 || (500..600).contains(&status)
}

/// `engine.normalize_language`: `auto` or a two-to-three letter code, lowercased.
pub fn normalize_language(value: Option<&str>) -> Option<String> {
    let text = value?.trim().to_lowercase();
    if text == "auto" {
        return Some(text);
    }
    let letters = text.len();
    if (2..=3).contains(&letters) && text.bytes().all(|byte| byte.is_ascii_lowercase()) {
        Some(text)
    } else {
        None
    }
}

/// Keep only as much of a chunk as the server's body limit allows
/// (`worker.fit_segments`): whole lines, in order, until the budget is spent.
pub fn fit_segments(segments: Vec<Segment>) -> Vec<Segment> {
    let mut used = 0usize;
    let mut kept = Vec::with_capacity(segments.len().min(64));
    for segment in segments {
        let size = segment.text.len().saturating_add(64);
        if used.saturating_add(size) > SEGMENT_BUDGET_BYTES {
            break;
        }
        used += size;
        kept.push(segment);
    }
    kept
}

/// The first `limit` characters of `text` (not bytes: a French cause may be cut
/// in the middle of an accent otherwise).
fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    text.chars().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::{
        Beat, CHUNK_SECONDS, CLAIM, ChunkBody, Claim, CompleteBody, CompleteReply, FailBody,
        OVERLAP_SECONDS, RETRY_AFTER_CAP_SECONDS, RETRY_MAX_SECONDS, SEGMENT_BUDGET_BYTES, audio,
        backoff, chunk, complete, fail, fit_segments, heartbeat, is_episode_id, is_transient,
        normalize_language, release,
    };
    use crate::segments::Segment;
    use std::time::Duration;

    /// The routes the server exposes (`transcribe_worker.py`), with the id in the
    /// one place it is allowed.
    #[test]
    fn the_routes_are_the_servers() {
        let id = "b7b1c0de-0000-4000-8000-000000000000";
        assert_eq!(CLAIM, "/api/transcribe/worker/claim");
        assert_eq!(
            audio(id).as_deref(),
            Some("/api/transcribe/worker/audio/b7b1c0de-0000-4000-8000-000000000000")
        );
        assert_eq!(
            heartbeat(id).as_deref(),
            Some("/api/transcribe/worker/b7b1c0de-0000-4000-8000-000000000000/heartbeat")
        );
        assert!(chunk(id).unwrap().ends_with("/chunk"));
        assert!(complete(id).unwrap().ends_with("/complete"));
        assert!(fail(id).unwrap().ends_with("/fail"));
        assert!(release(id).unwrap().ends_with("/release"));
    }

    /// An id that is not one never becomes a path: not a traversal, not a query,
    /// not an empty string.
    #[test]
    fn only_a_uuid_becomes_a_path() {
        assert!(is_episode_id("00000000-0000-0000-0000-000000000000"));
        assert!(is_episode_id("0123456789abcdef0123456789abcdef"));
        assert!(!is_episode_id(""));
        assert!(!is_episode_id("../../settings"));
        assert!(!is_episode_id("0/0"));
        assert!(!is_episode_id("00000000-0000-0000-0000-00000000000"));
        assert!(!is_episode_id("zzzzzzzz-0000-0000-0000-000000000000"));
        assert_eq!(audio("../settings"), None);
        assert_eq!(fail("0"), None);
    }

    /// The claim is the server's real answer, field for field
    /// (`transcribe_worker_claim`), and a server one version behind — one that
    /// does not send `chunk_ms` yet — is still usable.
    #[test]
    fn a_claim_is_the_servers_answer() {
        let id = "b7b1c0de-0000-4000-8000-000000000000";
        let payload = serde_json::json!({
            "episode_id": id,
            "lease": "lease-token",
            "audio_url": format!("/api/transcribe/worker/audio/{id}"),
            "duration_ms": 3_600_000,
            "language": "fr",
            "chunk_ms": 600_000,
            "overlap_ms": 2_000,
            "max_minutes": 360,
            "start_ms": 0,
            "expires_at": "2026-11-16T15:00:00",
        });
        let claim: Claim = serde_json::from_value(payload).expect("the server's claim");
        assert_eq!(claim.chunk_ms(), 600_000);
        assert_eq!(claim.overlap_ms(), 2_000);
        assert_eq!(claim.language_or(None).as_deref(), Some("fr"));
        assert!(claim.clone().checked().is_ok());

        // One version behind: no chunking, no language.
        let old = serde_json::json!({
            "episode_id": id,
            "lease": "lease-token",
            "audio_url": format!("/api/transcribe/worker/audio/{id}"),
            "duration_ms": 60_000,
        });
        let claim: Claim = serde_json::from_value(old).expect("an older claim");
        assert_eq!(claim.chunk_ms(), CHUNK_SECONDS * 1000);
        assert_eq!(claim.overlap_ms(), OVERLAP_SECONDS * 1000);
        assert_eq!(claim.language_or(None), None);
        assert_eq!(claim.chunk_ms(), 600_000);
    }

    /// Three facts in a claim are load-bearing and checked before use: the id
    /// becomes a path segment, the lease becomes a header value, and the audio
    /// URL is data — never a command that could point at another host.
    #[test]
    fn a_claim_that_could_escape_its_route_is_refused() {
        let id = "b7b1c0de-0000-4000-8000-000000000000";
        let base = |episode: &str, lease: &str, url: &str| Claim {
            episode_id: episode.to_owned(),
            lease: lease.to_owned(),
            audio_url: url.to_owned(),
            duration_ms: 1_000,
            language: None,
            chunk_ms: 0,
            overlap_ms: 0,
            max_minutes: 0,
            start_ms: 0,
            expires_at: None,
        };
        let good = format!("/api/transcribe/worker/audio/{id}");
        assert!(base(id, "lease", &good).checked().is_ok());
        assert!(base("../../settings", "lease", &good).checked().is_err());
        assert!(base(id, "", &good).checked().is_err());
        assert!(base(id, "two words", &good).checked().is_err());
        assert!(
            base(id, "lease", "https://evil.invalid/audio")
                .checked()
                .is_err()
        );
        assert!(
            base(id, "lease", "/api/transcribe/worker/audio/other")
                .checked()
                .is_err(),
            "another episode's audio is not this job's audio"
        );
    }

    /// The retry schedule is `worker.Client._backoff`: doubling from one second,
    /// capped at 30, and a `Retry-After` raises it (never lowers it) up to 120.
    #[test]
    fn the_backoff_doubles_and_obeys_retry_after() {
        let schedule: Vec<f64> = (1..=7)
            .map(|attempt| backoff(attempt, None).as_secs_f64())
            .collect();
        assert_eq!(schedule, [1.0, 2.0, 4.0, 8.0, 16.0, 30.0, 30.0]);
        assert_eq!(backoff(1, Some("2")).as_secs_f64(), 2.0);
        assert_eq!(backoff(1, Some("0")).as_secs_f64(), 1.0);
        assert_eq!(backoff(3, Some("1")).as_secs_f64(), 4.0);
        assert_eq!(
            backoff(1, Some("500")).as_secs_f64(),
            RETRY_AFTER_CAP_SECONDS,
            "a Retry-After of a week is not a week of silence"
        );
        assert_eq!(backoff(1, Some("soon")).as_secs_f64(), 1.0);
        assert_eq!(backoff(1, Some("-4")).as_secs_f64(), 1.0);
        assert_eq!(backoff(0, None), Duration::from_secs_f64(1.0));
        assert_eq!(backoff(u32::MAX, None).as_secs_f64(), RETRY_MAX_SECONDS);
    }

    /// A rate limit and a server fault are worth another attempt; a decision is
    /// not (`worker.Client.request` retries 429 and 5xx only).
    #[test]
    fn only_a_rate_limit_or_a_server_fault_is_retried() {
        assert!(is_transient(429));
        assert!(is_transient(500));
        assert!(is_transient(503));
        assert!(is_transient(599));
        for status in [200, 204, 400, 401, 403, 404, 409, 413, 416, 499, 600] {
            assert!(!is_transient(status), "{status}");
        }
    }

    /// The chunk body is the server's payload, in the contract's order and with
    /// the engine's own name.
    #[test]
    fn a_chunk_is_the_servers_payload() {
        let body = ChunkBody::new(
            600_000,
            600_000,
            "fr",
            "ggml-small.bin",
            "1.9.5",
            vec![Segment::new(600_000, 601_000, "Bonjour.")],
        );
        let value = serde_json::to_value(&body).expect("serialize");
        let text = serde_json::to_string(&body).expect("serialize");
        assert!(
            text.starts_with(
                "{\"start_ms\":600000,\"duration_ms\":600000,\"language\":\"fr\",\
                 \"engine\":\"whisper.cpp\",\"model\":\"ggml-small.bin\",\
                 \"engine_version\":\"1.9.5\",\"segments\":["
            ),
            "{text}"
        );
        assert_eq!(value["engine"], "whisper.cpp");
        assert_eq!(value["segments"][0]["text"], "Bonjour.");
    }

    /// The two replies the supervisor switches on, and `complete`'s body.
    #[test]
    fn the_replies_are_read_the_way_the_server_writes_them() {
        let done: CompleteReply = serde_json::from_value(
            serde_json::json!({"status": "ready", "done_ms": 3600000, "n_segments": 240}),
        )
        .expect("a completion");
        assert!(done.ready());
        assert_eq!(done.n_segments, 240);
        let queued: CompleteReply = serde_json::from_value(serde_json::json!({"status": "leased"}))
            .expect("a lease answer");
        assert!(!queued.ready());
        let beat: Beat =
            serde_json::from_value(serde_json::json!({"cancel": true, "pause": false}))
                .expect("a beat");
        assert!(beat.cancel);
        assert!(!beat.pause);
        assert_eq!(beat.done_ms, 0);

        let body = CompleteBody::new(Some("fr".to_owned()), "ggml-small.bin", "1.9.5");
        let value = serde_json::to_value(&body).expect("serialize");
        assert_eq!(value["language"], "fr");
        assert_eq!(value["engine"], "whisper.cpp");
    }

    /// `/fail` truncates at the server's two limits, in characters: a cause that
    /// does not fit would be a 400 and the person would see nothing at all.
    #[test]
    fn a_failure_is_cut_at_the_servers_limits() {
        let body = FailBody::new("é".repeat(40), "mot ".repeat(200));
        assert_eq!(body.code.chars().count(), 24);
        assert_eq!(body.message.chars().count(), 240);
        assert_eq!(
            body.code.chars().next(),
            Some('é'),
            "cut in characters, not bytes"
        );
        assert_eq!(FailBody::new("exit", "court").code, "exit");
    }

    /// `fit_segments` keeps whole lines until the budget: never a partial line,
    /// never more than the server's body limit.
    #[test]
    fn a_chunk_keeps_what_fits_in_the_servers_body() {
        let line = "x".repeat(1000);
        let segments: Vec<Segment> = (0..1000)
            .map(|i| Segment::new(i * 1000, i * 1000 + 900, line.clone()))
            .collect();
        let kept = fit_segments(segments);
        let used: usize = kept.iter().map(|s| s.text.len() + 64).sum();
        assert!(used <= SEGMENT_BUDGET_BYTES, "{used}");
        assert!(kept.len() < 1000);
        assert_ne!(kept.len(), 1, "the budget is spent on many lines, not one");
        assert_eq!(kept.len(), SEGMENT_BUDGET_BYTES / 1064);
        // An empty chunk stays empty, and one that fits in full is untouched.
        assert!(fit_segments(Vec::new()).is_empty());
        let two = vec![Segment::new(0, 1, "a"), Segment::new(1, 2, "b")];
        assert_eq!(fit_segments(two.clone()), two);
    }

    /// The language rule of `engine.normalize_language`: `auto`, or two to three
    /// lowercase letters, or nothing.
    #[test]
    fn a_language_is_auto_or_a_short_code() {
        assert_eq!(normalize_language(Some("auto")).as_deref(), Some("auto"));
        assert_eq!(normalize_language(Some("FR")).as_deref(), Some("fr"));
        assert_eq!(normalize_language(Some(" yue ")).as_deref(), Some("yue"));
        assert_eq!(normalize_language(Some("f")), None);
        assert_eq!(normalize_language(Some("français")), None);
        assert_eq!(normalize_language(Some("f1")), None);
        assert_eq!(normalize_language(None), None);
        assert_eq!(normalize_language(Some("")), None);
    }
}
