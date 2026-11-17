//! The admin screens' payloads: users and invitations, the analysis/genre
//! workers, the auto-match, and the replacement sheet.
//!
//! Everything here is behind `@admin_required` on the server except the
//! replacement sheet, so the desktop shows these screens to an admin only —
//! and the server refuses anyway.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::ids::{ArtistId, UserId};

/// One account in `/api/admin/users`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AdminUser {
    pub id: UserId,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub admin: bool,
    #[serde(default)]
    pub disabled: bool,
    /// The jukebox role (Subsonic's own).
    #[serde(default)]
    pub jukebox: bool,
    #[serde(default)]
    pub last_login: Option<String>,
    #[serde(default)]
    pub mail: Option<String>,
    /// Where the person's onboarding stands.
    #[serde(default)]
    pub onboarding: Option<String>,
}

/// `/api/admin/users?q=&after=&limit=` — one page of accounts.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AdminUsers {
    #[serde(default)]
    pub users: Vec<AdminUser>,
    #[serde(default)]
    pub total: u32,
    /// The keyset cursor of the next page, `null` on the last one.
    #[serde(default)]
    pub next: Option<serde_json::Value>,
}

/// One invitation in `/api/admin/invites`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AdminInvite {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub expires: Option<String>,
    /// `"pending"`, `"used"`, `"expired"`, `"revoked"`.
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub used_by: Option<String>,
    #[serde(default)]
    pub used_at: Option<String>,
}

/// `/api/admin/invites?state=&limit=&after_created=&after_id=`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AdminInvites {
    #[serde(default)]
    pub invites: Vec<AdminInvite>,
    /// The keyset cursor of the next page (`after_created` + `after_id`).
    #[serde(default)]
    pub next: Option<serde_json::Value>,
}

/// A background job's progress, as every long worker reports it.
///
/// `/api/analysis/backfill`, `/api/genre/embed`, `/api/similar/rebuild`,
/// `/api/enrich/backfill`, `/api/credits/backfill` and `/api/archive/migrate`
/// share this body; each fills the counters it has.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct JobStatus {
    #[serde(default)]
    pub running: bool,
    #[serde(default)]
    pub scanned: u32,
    #[serde(default)]
    pub done: u32,
    #[serde(default)]
    pub failed: u32,
    #[serde(default)]
    pub skipped: u32,
    #[serde(default)]
    pub total: u32,
    /// Unix seconds, `null` before the run started.
    #[serde(default)]
    pub started: Option<i64>,
    #[serde(default)]
    pub finished: Option<i64>,
    #[serde(default)]
    pub error: Option<String>,
    /// Why each failure failed: the CAUSE, counted (rule 2 forbids a bare count).
    #[serde(default)]
    pub failure_reasons: BTreeMap<String, u32>,
    /// The failures' own rows, which the screen lists.
    #[serde(default)]
    pub failures: Vec<serde_json::Value>,
}

/// `/api/analysis/backfill` — the measure-all job (tempo, style, vectors).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnalysisBackfill {
    #[serde(flatten)]
    pub job: JobStatus,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub workers: u32,
    #[serde(default)]
    pub workers_setting: u32,
    #[serde(default)]
    pub workers_auto: u32,
    #[serde(default)]
    pub embed_done: u32,
    /// The stage the job is in (`"measure"`, `"embed"`, …).
    #[serde(default)]
    pub stage: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    /// Paused asks the job to stop at the next track (rule 1).
    #[serde(default)]
    pub paused: Option<bool>,
    #[serde(default)]
    pub cancel: bool,
}

/// The nightly analysis window.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnalysisSchedule {
    #[serde(default)]
    pub enabled: bool,
    /// `HH:MM`, server local time.
    #[serde(default)]
    pub at: String,
    /// How long the window may run.
    #[serde(default)]
    pub hours: u32,
}

/// `/api/analysis/plan` — what is out of date, and where the measuring happens.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnalysisPlan {
    /// `"server"`, `"client"` or `"off"`: who measures.
    #[serde(default)]
    pub r#where: String,
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub total: u32,
    #[serde(default)]
    pub todo: u32,
    #[serde(default)]
    pub stale_measure: u32,
    #[serde(default)]
    pub stale_embed: u32,
    #[serde(default)]
    pub stale_decide: u32,
    #[serde(default)]
    pub unmeasured: u32,
    #[serde(default)]
    pub current: u32,
    /// The extractor version the library was measured with.
    #[serde(default)]
    pub embed_version: u32,
    /// The frozen ONNX extractor is installed.
    #[serde(default)]
    pub extractor: bool,
    #[serde(default)]
    pub schedule: AnalysisSchedule,
}

/// `/api/analysis/todo?limit=&embed=&after=` — the next tracks a CLIENT should
/// measure (client-side analysis, D16).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnalysisTodo {
    #[serde(default)]
    pub tracks: Vec<serde_json::Value>,
    /// The keyset cursor of the next page.
    #[serde(default)]
    pub next: Option<String>,
    /// How many seconds of audio the pending tempo work represents.
    #[serde(default)]
    pub tempo_seconds: u64,
    #[serde(default)]
    pub spectral_seconds: u64,
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub extractor: Option<String>,
}

/// `/api/explorer` — the background explorer's switches and counters.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Explorer {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub archive: bool,
    #[serde(default)]
    pub running: bool,
    #[serde(default)]
    pub paused: Option<bool>,
    #[serde(default)]
    pub pool: u32,
    #[serde(default)]
    pub pool_now: u32,
    #[serde(default)]
    pub rejected_total: u32,
    #[serde(default)]
    pub last: Option<serde_json::Value>,
    #[serde(default)]
    pub last_error: Option<String>,
}

/// `/api/automatch` — the audio auto-match's switch and last run.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Automatch {
    #[serde(default)]
    pub available: bool,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub switched_off: bool,
    #[serde(default)]
    pub youtube: bool,
    /// The similarity a match must reach to be applied.
    #[serde(default)]
    pub threshold: f64,
    #[serde(default)]
    pub job: Option<serde_json::Value>,
    #[serde(default)]
    pub last: Option<serde_json::Value>,
    #[serde(default)]
    pub recent: Vec<serde_json::Value>,
}

/// `/api/credits/backfill` — the credits/enrich workers' counters.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CreditsBackfill {
    #[serde(flatten)]
    pub job: JobStatus,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub tracks: u32,
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub counts: BTreeMap<String, u32>,
}

/// `/api/enrich/backfill` — the MusicBrainz facts' coverage.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EnrichStatus {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub running: bool,
    #[serde(default)]
    pub acoustid: bool,
    #[serde(default)]
    pub started: Option<i64>,
    #[serde(default)]
    pub coverage: EnrichCoverage,
}

/// How much of the library the enrichment has covered.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EnrichCoverage {
    #[serde(default)]
    pub tried: u32,
    #[serde(default)]
    pub untried: u32,
    #[serde(default)]
    pub found: BTreeMap<String, u32>,
    #[serde(default)]
    pub status: BTreeMap<String, u32>,
}

/// `/api/similar/rebuild` — the neighbours index's state.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SimilarStatus {
    #[serde(default)]
    pub running: bool,
    #[serde(default)]
    pub tracks: u32,
    #[serde(default)]
    pub started: Option<i64>,
    /// When the index was last rebuilt (Unix seconds).
    #[serde(default)]
    pub at: Option<i64>,
}

/// `/api/genre/status` — the vocabulary, its counts and its archetypes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GenreStatus {
    /// The archetypes a tag may be given (`"groove"`, `"voice"`, …).
    #[serde(default)]
    pub archetypes: Vec<String>,
    /// `{genre name: how many library tracks carry it}`.
    #[serde(default)]
    pub counts: BTreeMap<String, u32>,
    /// How many tracks carry a hand label.
    #[serde(default)]
    pub labelled: u32,
    /// The trained head, `null` when nobody has trained one yet.
    #[serde(default)]
    pub model: Option<serde_json::Value>,
    /// The tag vocabulary, with each tag's colour and archetype.
    #[serde(default)]
    pub tags: Vec<GenreNode>,
    /// The frozen ONNX extractor's own state.
    #[serde(default)]
    pub extractor: ExtractorInfo,
}

/// Whether the frozen ONNX extractor is installed, and what it is.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExtractorInfo {
    #[serde(default)]
    pub available: bool,
    /// The vector's dimension.
    #[serde(default)]
    pub dim: u32,
    /// The model file in place, `null` when it is not.
    #[serde(default)]
    pub model: Option<String>,
    /// Where the model would come from, when it has to be fetched.
    #[serde(default)]
    pub model_url: Option<String>,
    /// The ONNX runtime is installed (a BOOLEAN, not a version).
    #[serde(default)]
    pub onnxruntime: bool,
    /// Why it is not available, in the server's words (`"numpy is not installed"`).
    #[serde(default)]
    pub reason: Option<String>,
    /// The extractor's own version counter.
    #[serde(default)]
    pub version: u32,
    /// `"ok"`, `"broken"`… — whether the operator's own model loaded.
    #[serde(default)]
    pub session_state: Option<String>,
    #[serde(default)]
    pub session_error: Option<String>,
    /// The operator may upload their own copy of the model.
    #[serde(default)]
    pub uploadable: bool,
}

/// One node of the genre tree.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GenreNode {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub archetype: Option<String>,
    /// `#rrggbb`, the colour the studio paints the node with.
    #[serde(default)]
    pub color: Option<String>,
}

/// One parent→child link of the genre tree.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GenreEdge {
    #[serde(default)]
    pub child: String,
    #[serde(default)]
    pub parent: String,
    #[serde(default)]
    pub weight: f64,
}

/// `/api/genre/tree` — the hierarchy and its version (edits are optimistic).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GenreTree {
    #[serde(default)]
    pub nodes: Vec<GenreNode>,
    #[serde(default)]
    pub edges: Vec<GenreEdge>,
    #[serde(default)]
    pub roots: Vec<String>,
    #[serde(default)]
    pub version: String,
    /// No edit is pending: the tree is the default one.
    #[serde(default)]
    pub default: bool,
    /// An edit can be undone.
    #[serde(default)]
    pub can_undo: bool,
}

/// `/api/genre/candidates` — the tracks to label next.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GenreCandidates {
    /// The candidate rows; the studio's own shape (D16).
    #[serde(default)]
    pub candidates: Vec<serde_json::Value>,
    /// How many tracks already carry a hand label.
    #[serde(default)]
    pub labelled: u32,
    /// `"plays"` or `"active"` — why these rows, in this order.
    #[serde(default)]
    pub sort: String,
    /// The scan stopped on its own cap: there may be more.
    #[serde(default)]
    pub truncated: bool,
}

/// `/api/genre/labelled` — the hand-labelled tracks.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GenreLabelled {
    #[serde(default)]
    pub labelled: Vec<serde_json::Value>,
}

/// `/api/genre/pool?limit=` — untagged tracks with a vector, for self-training.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GenrePool {
    #[serde(default)]
    pub ids: Vec<String>,
    #[serde(default)]
    pub scanned: u32,
}

/// One score of `/api/genre/track/<id>` and of the studio's candidates: the
/// vocabulary ranked for one track.
///
/// The server sends a TUPLE, not an object — `[label, score, head, proto]`
/// (`neezer/deezer/genre.py#rank_scores`, served by `genre.py#_scores_json`) —
/// and this type declared fields, which no fixture caught because the bench had
/// nothing labelled and the list was always `[]`. `score` (0..1) only ORDERS the
/// vocabulary; `head` is the trained head's calibrated probability and `proto`
/// the nearest prototype's share, each `null` when that opinion does not exist.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "GenreScoreWire", into = "GenreScoreWire")]
pub struct GenreScore {
    pub label: String,
    pub score: f64,
    pub head: Option<f64>,
    pub proto: Option<f64>,
}

/// The wire shape: one JSON array of four, which is what the route sends.
type GenreScoreWire = (String, f64, Option<f64>, Option<f64>);

impl From<GenreScoreWire> for GenreScore {
    fn from((label, score, head, proto): GenreScoreWire) -> Self {
        Self {
            label,
            score,
            head,
            proto,
        }
    }
}

impl From<GenreScore> for GenreScoreWire {
    fn from(score: GenreScore) -> Self {
        (score.label, score.score, score.head, score.proto)
    }
}

/// `/api/genre/track/<id>` — one track's tag and the vocabulary ranked for it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GenreTrack {
    #[serde(default)]
    pub track: String,
    /// The tag the track wears, as the server sends it (`_tag_json`: an OBJECT,
    /// not its name — this field declared a `String` until a fixture carried one).
    #[serde(default)]
    pub tag: Option<GenreTag>,
    #[serde(default)]
    pub scores: Vec<GenreScore>,
}

/// One genre tag, as `_tag_json` writes it: what a chip draws.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GenreTag {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub archetype: Option<String>,
}

/// `/api/genre/model` — whether a trained head is in place.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GenreModel {
    #[serde(default)]
    pub ready: bool,
}

/// `/api/genre/relabel` — re-deciding the stored verdicts (no re-measure).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GenreRelabel {
    #[serde(default)]
    pub running: bool,
    #[serde(default)]
    pub scanned: u32,
    #[serde(default)]
    pub total: u32,
    #[serde(default)]
    pub changed: u32,
    #[serde(default)]
    pub tag: u32,
    #[serde(default)]
    pub model: u32,
    #[serde(default)]
    pub heuristic: u32,
    #[serde(default)]
    pub family: u32,
    #[serde(default)]
    pub started: Option<i64>,
    #[serde(default)]
    pub finished: Option<i64>,
    #[serde(default)]
    pub error: Option<String>,
}

/// `/api/genre/embed` — measuring every archived track that has no vector.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GenreEmbed {
    #[serde(flatten)]
    pub job: JobStatus,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub workers: u32,
}

/// `/api/genre/construction` — how much of the library has a construction
/// summary (D16's client-side analysis).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GenreConstruction {
    #[serde(default)]
    pub labelled: u32,
    #[serde(default)]
    pub measured: u32,
    /// The labels with no measurement yet: `{"id", "bpm"}` each (this declared a
    /// list of plain ids until a bench with hand labels was captured).
    #[serde(default)]
    pub missing: Vec<GenreConstructionMissing>,
}

/// One label waiting for its measurement (`/genre/construction`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GenreConstructionMissing {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub bpm: Option<f64>,
}

/// `/api/genre/references` — the genres a reference lookup can be asked about.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GenreReferences {
    #[serde(default)]
    pub genres: Vec<String>,
}

/// `/api/genre/references?genre=` — the well-known recordings of one genre.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GenreReferencesFor {
    #[serde(default)]
    pub genre: String,
    /// Deezer's own answer is available (the provider is reachable).
    #[serde(default)]
    pub listed: bool,
    #[serde(default)]
    pub artists: Vec<ArtistId>,
    #[serde(default)]
    pub tracks: Vec<serde_json::Value>,
}

/// `/api/genre/extractor/test` — the ONNX extractor's sanity check.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExtractorTest {
    #[serde(default)]
    pub running: bool,
    #[serde(default)]
    pub ok: Option<bool>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub started: Option<i64>,
    /// The per-track lines the check printed.
    #[serde(default)]
    pub progress: Vec<String>,
    #[serde(default)]
    pub result: Option<serde_json::Value>,
}

/// `/api/genre/share/import` — a shared model's import job.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GenreShareStatus {
    #[serde(default)]
    pub running: bool,
    #[serde(default)]
    pub done: u32,
    #[serde(default)]
    pub total: u32,
    #[serde(default)]
    pub failed: u32,
    #[serde(default)]
    pub kept: u32,
    #[serde(default)]
    pub tagged: u32,
    #[serde(default)]
    pub already: u32,
    #[serde(default)]
    pub not_found: u32,
    #[serde(default)]
    pub stopped: bool,
    #[serde(default)]
    pub finished: Option<i64>,
}

/// One candidate of `/api/replace/candidates/<id>`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReplaceCandidate {
    /// The candidate's own row shape (the sheet's, D8a).
    #[serde(flatten)]
    pub row: BTreeMap<String, serde_json::Value>,
}

/// `/api/replace/candidates/<id>` — the other sources' answers for a dead track.
///
/// Two shapes come back from one route: with `best=n` the server answers
/// `fallback`/`youtube`, without it `fallback_auto`/`youtube_auto`. Both are
/// optional here so either answer parses.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReplaceCandidates {
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub candidates: Vec<serde_json::Value>,
    /// The source reached for on its own, `null` when none was.
    #[serde(default)]
    pub fallback: Option<String>,
    #[serde(default)]
    pub fallback_auto: Option<String>,
    /// The `best=n` answer: whether the fallback was YouTube.
    #[serde(default)]
    pub youtube: Option<bool>,
    #[serde(default)]
    pub youtube_auto: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shared_job_status_parses_the_counters_every_worker_sends() {
        let embed: GenreEmbed = serde_json::from_str(
            r#"{"done":0,"error":null,"failed":0,"failure_reasons":{},"failures":[],
                "finished":null,"force":false,"running":false,"scanned":0,"skipped":0,
                "started":null,"total":300,"workers":4}"#,
        )
        .expect("embed status");
        assert_eq!(embed.job.total, 300);
        assert_eq!(embed.workers, 4);
    }

    /// A ranked genre is a TUPLE on the wire, and the type had fields: the bench
    /// answered `[]` (nothing labelled), so the fixture never said otherwise.
    /// This pins the server's own list, `neezer/deezer/genre.py#rank_scores`:
    /// `[label, score, head, proto]`, `head`/`proto` possibly null.
    #[test]
    fn a_genre_score_is_the_servers_tuple_not_an_object() {
        let answer: GenreTrack = serde_json::from_str(
            r#"{"track":"3f7c1d2e-9a4b-5c6d-8e0f-1a2b3c4d5e6f",
                "tag":null,
                "scores":[["Hardtek",0.62,0.7,0.4],["Ambient",0.21,null,0.21],
                          ["Techno",0.17,0.12,null]]}"#,
        )
        .expect("the route's answer");
        assert_eq!(answer.scores.len(), 3);
        assert_eq!(answer.scores[0].label, "Hardtek");
        assert_eq!(answer.scores[0].score, 0.62);
        assert_eq!(answer.scores[0].head, Some(0.7));
        assert_eq!(answer.scores[1].head, None, "the head does not know it");
        assert_eq!(answer.scores[2].proto, None, "no prototype for it");

        // And it goes back out as the same array: a PUT carries what a GET read.
        let wire = serde_json::to_value(&answer.scores[0]).expect("serialise");
        assert_eq!(wire, serde_json::json!(["Hardtek", 0.62, 0.7, 0.4]));

        // The object shape the type used to declare is NOT the server's, and
        // must fail loudly rather than parse into a silently empty ranking.
        assert!(serde_json::from_str::<GenreScore>(r#"{"name":"Hardtek","score":0.6}"#).is_err());
    }

    #[test]
    fn the_two_replacement_answers_parse_with_one_type() {
        let plain: ReplaceCandidates = serde_json::from_str(
            r#"{"query":"Blue","candidates":[],"fallback_auto":null,"youtube_auto":false}"#,
        )
        .expect("plain answer");
        assert!(plain.fallback.is_none());

        let best: ReplaceCandidates = serde_json::from_str(
            r#"{"query":"Blue","candidates":[],"fallback":"youtube","youtube":true}"#,
        )
        .expect("best=n answer");
        assert_eq!(best.fallback.as_deref(), Some("youtube"));
        assert_eq!(best.youtube, Some(true));
    }
}

/// One track's measured verdict, as the player reads it (tempo, style, energy).
///
/// This payload is the API's only camelCase one (`deezer/analysis.py#payload_for`),
/// so every field carries the name the server writes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackVerdict {
    #[serde(default)]
    pub bpm: Option<f64>,
    #[serde(default)]
    pub bpm_confidence: f64,
    #[serde(default)]
    pub bpm_source: Option<String>,
    /// The style's machine name (`"techno"`).
    #[serde(default)]
    pub style: Option<String>,
    /// The French label to show.
    #[serde(default)]
    pub style_label: Option<String>,
    #[serde(default)]
    pub style_confidence: f64,
    #[serde(default)]
    pub archetype: Option<String>,
    /// `{archetype: share}`.
    #[serde(default)]
    pub archetypes: BTreeMap<String, f64>,
    /// `"measured"`, `"tag"`, `"file"`, `"provisional"`…
    #[serde(default)]
    pub style_source: Option<String>,
    #[serde(default)]
    pub embedded: bool,
    /// The measured pulse (0..1).
    #[serde(default)]
    pub pulse: f64,
    #[serde(default)]
    pub lufs: Option<f64>,
    #[serde(default)]
    pub lra: Option<f64>,
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub analysed: bool,
}

/// `/api/analyses` (POST) — the verdicts of a whole run of tracks.
///
/// `pending` is the client's to-do list: tracks the server has just queued to
/// measure itself, which the player polls for.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Analyses {
    #[serde(default)]
    pub analyses: BTreeMap<String, TrackVerdict>,
    #[serde(default)]
    pub pending: Vec<String>,
    /// The verdicts' generation: when a new head is applied, the client drops
    /// what it kept on the device.
    #[serde(default, rename = "gen")]
    pub generation: u32,
}

/// `/api/analysis/<id>` — one track's verdict, or `ready: false`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackAnalysis {
    #[serde(default)]
    pub ready: bool,
    #[serde(flatten)]
    pub verdict: TrackVerdict,
    #[serde(default)]
    pub pending: Vec<String>,
}
