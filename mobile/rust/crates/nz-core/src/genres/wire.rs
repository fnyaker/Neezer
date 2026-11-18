//! The wire: the studio's routes, their counted traffic, and the copy this
//! device keeps between two visits.
//!
//! Every call goes through `nz-api`'s TYPED genre endpoints (`endpoints/admin.rs`
//! — the same records the desktop's D16 decodes), on [`Lane::User`]: the screen
//! being opened is the action just taken, never behind a prefetch or a job
//! (rule 1). Nothing here decides a rule; the shapes are `nz-model`'s and the
//! SPA components name which route each one is (`docs/agents/mobile.md`, MB18).
//!
//! Two routes have no typed wrapper because their answer is what the screen
//! draws field by field (`POST /genre/embeddings`, whose keys are track
//! references; `POST /genre/bulk/preview`, whose rows carry a per-track verdict):
//! they use [`Api::send`], the same machinery the wrappers use — same lane, same
//! retry policy, same counters.
//!
//! # The device's copy
//!
//! One JSON file per account, `genres-<hash>.json` in the CACHE directory
//! (evictable, never the app's own data), holding the vocabulary, the tree, the
//! relabel and embed states, and one page of candidates per sort order. A warm
//! open of the studio therefore paints before any request, and a dead link still
//! shows the vocabulary it last had. Two details keep it honest, both MB17's for
//! the same reason:
//!
//! - a card is written ONLY when its read succeeded, so a failure never
//!   overwrites a good copy with an empty one;
//! - the file is written by one atomic rename, so a process killed mid-write
//!   leaves the previous copy rather than half of one.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use nz_api::{Api, Lane, Method};
use nz_model::{
    ExtractorTest, GenreConstruction, GenreEmbed, GenreLabelled, GenrePool, GenreRelabel,
    GenreStatus, GenreTag, GenreTrack, GenreTree,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Core, CoreError, TAG_GENRES};

/// What one tag spent, as `nz-api` counts it (SC3): requests really sent, and
/// the bodies' bytes.
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
        .find(|row| row.tag == TAG_GENRES)
        .map(|row| Totals {
            requests: row.requests,
            bytes_in: row.bytes_in,
            bytes_out: row.bytes_out,
        })
        .unwrap_or_default()
}

// --- one candidate, as `/genre/candidates` sends it ---------------------------

/// One row of `/genre/candidates` — `_track_json` plus the ranking, as
/// `desktop/app/src/screens/genres.rs` reads it (D16's shape, which the server
/// owns).
#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct CandidateWire {
    #[serde(default)]
    pub id: String,
    /// A Deezer number, or the row's UUID for an upload; `Value` because the
    /// server sends a number here and a string there.
    #[serde(default)]
    pub deezer_id: Value,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub duration: u32,
    #[serde(default)]
    pub play_count: u32,
    #[serde(default)]
    pub deezer_bpm: Option<f64>,
    #[serde(default)]
    pub predicted: Option<String>,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub uncertainty: Option<f64>,
    /// The server's ranking for this track: `[[label, score, head, proto]]`.
    #[serde(default)]
    pub scores: Vec<nz_model::GenreScore>,
}

impl CandidateWire {
    /// The ref every route takes: the universal `deezer_id` when it exists (a
    /// number or a UUID), else the row's UUID. The SPA's `trackRef`.
    pub fn reference(&self) -> String {
        match &self.deezer_id {
            Value::String(text) if !text.is_empty() => text.clone(),
            Value::Number(number) => number.to_string(),
            _ => self.id.clone(),
        }
    }

    /// The `[label, score, head, proto]` rows the ranker takes.
    pub fn score_tuples(&self) -> Vec<super::rank::Score> {
        self.scores
            .iter()
            .map(|score| (score.label.clone(), score.score, score.head, score.proto))
            .collect()
    }
}

/// `/genre/candidates`'s answer, as the screen needs it.
#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct CandidatesWire {
    #[serde(default)]
    pub candidates: Vec<Value>,
    #[serde(default)]
    pub labelled: u32,
    #[serde(default)]
    pub sort: String,
    #[serde(default)]
    pub truncated: bool,
}

// --- the embeddings ----------------------------------------------------------

/// `POST /genre/embeddings`'s answer: `{embeddings: {ref: base64 float16}}`.
#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct EmbeddingsWire {
    #[serde(default)]
    pub embeddings: BTreeMap<String, String>,
}

// --- the bulk preview --------------------------------------------------------

/// One row of `POST /genre/bulk/preview`: the track, what it wears, what the
/// head thinks, and why it is (or is not) a disagreement.
#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct BulkRowWire {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub deezer_id: Value,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub duration: u32,
    /// The tag it currently wears, `None` when it is untagged.
    #[serde(default)]
    pub current: Option<String>,
    /// True when it already wears the tag being applied — nothing to do.
    #[serde(default)]
    pub same: bool,
    /// `"tag"` (it wears another one) or `"model"` (the head disagrees), else
    /// `None`.
    #[serde(default)]
    pub disagree: Option<String>,
    /// The head's own verdict, when it has one.
    #[serde(default)]
    pub model: Option<BulkOpinionWire>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct BulkOpinionWire {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub conf: f64,
    #[serde(default)]
    pub level: String,
}

impl BulkRowWire {
    pub fn reference(&self) -> String {
        match &self.deezer_id {
            Value::String(text) if !text.is_empty() => text.clone(),
            Value::Number(number) => number.to_string(),
            _ => self.id.clone(),
        }
    }
}

/// The preview's counters, as the server worded them.
#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct BulkCountsWire {
    #[serde(default)]
    pub tracks: u32,
    #[serde(default)]
    pub missing: u32,
    #[serde(default)]
    pub same: u32,
    #[serde(default)]
    pub other_tag: u32,
    #[serde(default)]
    pub model: u32,
    #[serde(default)]
    pub no_vector: u32,
    #[serde(default)]
    pub unjudged: u32,
}

/// `POST /genre/bulk/preview`'s answer.
#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct BulkPreviewWire {
    #[serde(default)]
    pub tag: Option<GenreTag>,
    #[serde(default)]
    pub tracks: Vec<Value>,
    #[serde(default)]
    pub counts: BulkCountsWire,
    #[serde(default)]
    pub model: BulkModelWire,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct BulkModelWire {
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub knows: bool,
}

/// `POST /genre/bulk`'s answer: what was applied, and what the server does with
/// the ids it was given that are not in the library.
#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct BulkAppliedWire {
    #[serde(default)]
    pub applied: u32,
    #[serde(default)]
    pub missing: u32,
}

// --- the tree ----------------------------------------------------------------

// --- the requests ------------------------------------------------------------

/// `GET /api/genre/status` — the vocabulary, its counts, the trained head and
/// the extractor. The SPA's first read (`Genres.svelte#refresh`).
pub(super) async fn status(api: &Api) -> Result<GenreStatus, CoreError> {
    Ok(api.genre_status().await?)
}

/// `GET /api/genre/candidates?limit=&sort=&last=` — what to tag next, 40 at a
/// time (`Genres.svelte#loadCandidates`), in this track's own order.
pub(super) async fn candidates(
    api: &Api,
    limit: u32,
    sort: Option<&str>,
    last: Option<&str>,
) -> Result<CandidatesWire, CoreError> {
    let answer = api.genre_candidates(limit, sort, last).await?;
    Ok(CandidatesWire {
        candidates: answer.candidates,
        labelled: answer.labelled,
        sort: answer.sort,
        truncated: answer.truncated,
    })
}

/// `GET /api/genre/track/<id>` — one track's tag and the vocabulary ranked for
/// it (the tag sheet opened from a list, MB10's menu).
pub(super) async fn track(api: &Api, reference: &str) -> Result<GenreTrack, CoreError> {
    Ok(api.genre_track(reference).await?)
}

/// `POST /api/genre/label` — one track, one tag (`null` removes the tag).
pub(super) async fn label(api: &Api, reference: &str, tag: Option<i64>) -> Result<(), CoreError> {
    // The route takes the tag ID as a string or `null`; `nz-api`'s wrapper
    // takes `&str`, so `None` is the SPA's own call with `tag: null`.
    let body = serde_json::json!({ "track": reference, "tag": tag });
    let _: Option<Value> = api
        .send(Lane::User, Method::POST, "/genre/label", Some(&body))
        .await?;
    Ok(())
}

/// One write of the vocabulary editor, as the sheet chose it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum VocabularyWrite {
    Create {
        name: String,
        color: Option<String>,
        archetype: Option<String>,
    },
    Edit {
        id: i64,
        patch: Value,
    },
    Delete {
        id: i64,
    },
    Defaults,
}

/// Dispatches one vocabulary write. Every one answers something the studio does
/// not draw (the server's own tag object), so the studio re-reads `/genre/status`
/// once afterwards — the server's vocabulary is the one a person then edits, never
/// a local guess.
pub(super) async fn vocabulary_write(api: &Api, write: &VocabularyWrite) -> Result<(), CoreError> {
    match write {
        VocabularyWrite::Create {
            name,
            color,
            archetype,
        } => tag_create(api, name, color.as_deref(), archetype.as_deref()).await,
        VocabularyWrite::Edit { id, patch } => tag_edit(api, *id, patch).await,
        VocabularyWrite::Delete { id } => tag_delete(api, *id).await,
        VocabularyWrite::Defaults => tags_defaults(api).await,
    }
}

/// `POST /api/genre/tags` — a new tag in the person's own vocabulary.
pub(super) async fn tag_create(
    api: &Api,
    name: &str,
    color: Option<&str>,
    archetype: Option<&str>,
) -> Result<(), CoreError> {
    api.genre_tag_create(name, color, archetype).await?;
    Ok(())
}

/// `PATCH /api/genre/tags/<id>` — rename, recolour, change the archetype.
pub(super) async fn tag_edit(api: &Api, id: i64, patch: &Value) -> Result<(), CoreError> {
    api.genre_tag_edit(id, patch).await?;
    Ok(())
}

/// `DELETE /api/genre/tags/<id>`.
pub(super) async fn tag_delete(api: &Api, id: i64) -> Result<(), CoreError> {
    api.genre_tag_delete(id).await?;
    Ok(())
}

/// `POST /api/genre/tags/defaults` — (re)add the engine's genres; only ever
/// adds what is missing, which is what makes it safe to press again.
pub(super) async fn tags_defaults(api: &Api) -> Result<(), CoreError> {
    api.genre_defaults().await?;
    Ok(())
}

/// `GET /api/genre/labelled` — every hand-labelled track, newest first.
pub(super) async fn labelled(api: &Api) -> Result<GenreLabelled, CoreError> {
    Ok(api.genre_labelled().await?)
}

/// `GET /api/genre/tree` — nodes, edges, roots, version.
pub(super) async fn tree(api: &Api) -> Result<GenreTree, CoreError> {
    Ok(api.genre_tree().await?)
}

/// `POST /api/genre/tree/edge` (add), `DELETE` (cut), `POST /api/genre/tree/move`.
///
/// **Every tree write answers the WHOLE tree** (`webui/genre.py#_tree_answer`), so
/// the answer is read as one: a second `GET /genre/tree` would be a request the
/// page does not make.
pub(super) async fn tree_edit(
    api: &Api,
    kind: TreeEdit,
    child: &str,
    parent: &str,
    old_parent: Option<&str>,
    weight: f64,
) -> Result<GenreTree, CoreError> {
    let answer = match kind {
        TreeEdit::Add => {
            api.genre_tree_edge(child, parent, weight, Method::POST)
                .await?
        }
        TreeEdit::Cut => api.genre_tree_cut(child, parent).await?,
        TreeEdit::Move => {
            api.genre_tree_move(child, parent, old_parent, weight)
                .await?
        }
    };
    tree_of(answer)
}

/// The answer of a tree write, as the tree it is.
fn tree_of(answer: Value) -> Result<GenreTree, CoreError> {
    serde_json::from_value(answer).map_err(|error| CoreError::Internal {
        detail: format!("arbre illisible: {error}"),
    })
}

/// Which tree write a row asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TreeEdit {
    Add,
    Cut,
    Move,
}

/// `POST /api/genre/tree/undo` — the last edit, undone (its answer is the tree).
pub(super) async fn tree_undo(api: &Api) -> Result<GenreTree, CoreError> {
    tree_of(api.genre_tree_undo().await?)
}

/// `POST /api/genre/tree/reset` — back to the engine's own tree.
pub(super) async fn tree_reset(api: &Api) -> Result<GenreTree, CoreError> {
    tree_of(api.genre_tree_reset().await?)
}

/// `GET /api/genre/pool?limit=` — untagged tracks that have a vector (the
/// self-training pool).
pub(super) async fn pool(api: &Api, limit: u32) -> Result<GenrePool, CoreError> {
    Ok(api.genre_pool(limit).await?)
}

/// `POST /api/genre/embeddings` — the stored vectors of these tracks. The route
/// NEVER extracts, so a track without one is simply absent from the answer.
pub(super) async fn embeddings(api: &Api, ids: &[String]) -> Result<EmbeddingsWire, CoreError> {
    let answer = api.genre_embeddings(ids).await?;
    serde_json::from_value(answer).map_err(|error| CoreError::Internal {
        detail: format!("empreintes illisibles: {error}"),
    })
}

/// `PUT /api/genre/model` — store the head the trainer just built. The answer
/// carries the reclassification's state (`relabel`) when the server started one.
pub(super) async fn model_put(api: &Api, payload: &Value) -> Result<Value, CoreError> {
    Ok(api.genre_model_put(payload).await?)
}

/// `DELETE /api/genre/model`.
pub(super) async fn model_delete(api: &Api) -> Result<(), CoreError> {
    api.genre_model_delete().await?;
    Ok(())
}

/// `GET /api/genre/relabel` — the stored verdicts re-decided under the active
/// head (no re-measure).
pub(super) async fn relabel(api: &Api) -> Result<GenreRelabel, CoreError> {
    Ok(api.genre_relabel().await?)
}

/// `POST /api/genre/relabel` — start it.
pub(super) async fn relabel_start(api: &Api) -> Result<Value, CoreError> {
    Ok(api.genre_relabel_start().await?)
}

/// `GET /api/genre/construction` — how much of the library has a construction
/// summary (the player writes it; the studio only reports it).
pub(super) async fn construction(api: &Api) -> Result<GenreConstruction, CoreError> {
    Ok(api.genre_construction().await?)
}

/// `POST /api/genre/bulk/preview` — what tagging a whole album, playlist or
/// artist would do, track by track.
pub(super) async fn bulk_preview(
    api: &Api,
    scope: &Value,
    tag: &str,
) -> Result<BulkPreviewWire, CoreError> {
    let answer = api.genre_bulk_preview(scope, tag).await?;
    serde_json::from_value(answer).map_err(|error| CoreError::Internal {
        detail: format!("aperçu de lot illisible: {error}"),
    })
}

/// `POST /api/genre/bulk` — tag the tracks the person kept after the preview.
pub(super) async fn bulk(
    api: &Api,
    tracks: &[String],
    tag: &str,
) -> Result<BulkAppliedWire, CoreError> {
    let answer = api.genre_bulk(tracks, tag).await?;
    serde_json::from_value(answer).map_err(|error| CoreError::Internal {
        detail: format!("réponse de lot illisible: {error}"),
    })
}

/// `POST /api/genre/embed[?force=1]` — measure every archived track without a
/// vector (a server job; the poll is [`embed_status`]).
pub(super) async fn embed_start(api: &Api, force: bool) -> Result<(), CoreError> {
    api.genre_embed(force).await?;
    Ok(())
}

/// `GET /api/genre/embed` — its progress and its failures' causes.
pub(super) async fn embed_status(api: &Api) -> Result<GenreEmbed, CoreError> {
    Ok(api.genre_embed_status().await?)
}

/// `GET /api/genre/extractor/test` — the ONNX extractor's own check, when one
/// is running (the phone only reads it: the model is installed from the server).
pub(super) async fn extractor_test(api: &Api) -> Result<ExtractorTest, CoreError> {
    Ok(api.genre_extractor_test_status().await?)
}

/// `POST /api/genre/extractor/test` — start the check.
pub(super) async fn extractor_test_start(api: &Api) -> Result<(), CoreError> {
    api.genre_extractor_test().await?;
    Ok(())
}

// --- the device's copy -------------------------------------------------------

/// Everything the studio paints before its first request, per account.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct Snapshot {
    #[serde(default)]
    pub status: Option<GenreStatus>,
    /// One page of candidates per sort order (`plays`, `active`), so switching
    /// the order back is a disk hit, not a request.
    #[serde(default)]
    pub candidates: BTreeMap<String, CandidatesCopy>,
    #[serde(default)]
    pub tree: Option<GenreTree>,
    #[serde(default)]
    pub relabel: Option<GenreRelabel>,
    #[serde(default)]
    pub embed: Option<GenreEmbed>,
    #[serde(default)]
    pub construction: Option<GenreConstruction>,
}

/// One cached page of candidates: the rows AND what the server said about them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct CandidatesCopy {
    #[serde(default)]
    pub rows: Vec<Value>,
    #[serde(default)]
    pub labelled: u32,
    #[serde(default)]
    pub sort: String,
    #[serde(default)]
    pub truncated: bool,
}

impl Snapshot {
    /// The device's copy, or an empty one when the person never opened the
    /// studio.
    pub(super) fn read(core: &Core) -> Snapshot {
        std::fs::read_to_string(snapshot_path(core))
            .ok()
            .and_then(|body| serde_json::from_str(&body).ok())
            .unwrap_or_default()
    }

    /// Writes one card and keeps the others: a card's read is the only thing
    /// that may replace it.
    pub(super) fn write(core: &Core, change: impl FnOnce(&mut Snapshot)) {
        let mut snapshot = Snapshot::read(core);
        change(&mut snapshot);
        let path = snapshot_path(core);
        let Ok(body) = serde_json::to_vec(&snapshot) else {
            return;
        };
        let temporary = path.with_extension("json.part");
        if std::fs::write(&temporary, &body).is_ok() {
            let _ = std::fs::rename(&temporary, &path);
        }
    }
}

/// Where the account's copy lives: the CACHE directory (evictable), never the
/// app's own data.
fn snapshot_path(core: &Core) -> PathBuf {
    core.cache_path(&format!(
        "genres-{:016x}.json",
        fnv1a(owner(core).as_bytes())
    ))
}

/// The account the copy belongs to: `/me`'s id, `""` before a session.
fn owner(core: &Core) -> String {
    core.me()
        .map(|me| me.user.id.to_string())
        .filter(|id| !id.is_empty())
        .unwrap_or_default()
}

/// FNV-1a: a stable file name for an account id, and no dependency for it.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

impl Core {
    /// The tagged client of one studio read. The offline gate is the core's, so
    /// a dead link answers `Offline` in zero sockets.
    pub(super) fn genres_api(&self) -> Result<Arc<Api>, CoreError> {
        Ok(Arc::new(self.api()?.with_tag(TAG_GENRES)))
    }

    /// Books one read's traffic in the core's meter, so `NzCore::stats()` answers
    /// what the studio cost without the UI counting anything.
    pub(super) fn note_genre_traffic(&self, api: &Api, before: Totals) {
        let after = totals(api);
        let requests = after.requests.saturating_sub(before.requests);
        if requests == 0 {
            return;
        }
        self.meter().record(
            TAG_GENRES,
            after.bytes_in.saturating_sub(before.bytes_in),
            after.bytes_out.saturating_sub(before.bytes_out),
        );
        for _ in 1..requests {
            self.meter().record(TAG_GENRES, 0, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The copy is named after a hash of the account, never the account id, and
    /// two accounts never share a copy.
    #[test]
    fn the_copy_is_named_after_a_hash_of_the_account() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_ne!(fnv1a(b"1"), fnv1a(b"2"));
        let path = format!("genres-{:016x}.json", fnv1a(b"42"));
        assert!(!path.contains("42"), "{path}");
    }

    /// One card written is one card read back, and it never blanks the others:
    /// closing the tree and reopening the studio keeps the vocabulary.
    #[test]
    fn one_card_never_blanks_another() {
        let dir = crate::genres::testing::TempDir::new("genres-snapshot");
        let core = dir.core();
        Snapshot::write(&core, |snapshot| {
            snapshot.status = Some(GenreStatus {
                labelled: 12,
                ..GenreStatus::default()
            });
        });
        Snapshot::write(&core, |snapshot| {
            snapshot.relabel = Some(GenreRelabel {
                scanned: 40,
                ..GenreRelabel::default()
            });
        });
        let snapshot = Snapshot::read(&core);
        assert_eq!(snapshot.status.expect("status").labelled, 12);
        assert_eq!(snapshot.relabel.expect("relabel").scanned, 40);
        assert!(snapshot.tree.is_none());
    }
}
