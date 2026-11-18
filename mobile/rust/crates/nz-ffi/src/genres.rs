//! The genre studio, the genre tree and the client-side analysis. Owned by
//! **MB18** (`docs/agents/mobile.md`, "Module and file ownership").
//!
//! The note names what this block reaches: the `/genre/*` routes the SPA's
//! `routes/Genres.svelte` and its components call, the tree with its move/undo/
//! reset, the tag and bulk sheets, the extractor's state, and the TRAINER and the
//! local MEASURE running in Rust (`webapp/trainer` and `webapp/measure` linked
//! natively through `nz-analysis`, the same crates the desktop's D16 uses).
//!
//! # The shape of this boundary
//!
//! - **Each screen is a `cached_x()` / `x()` pair**: the device's copy paints at
//!   once, the network replaces it, and a failure keeps the copy
//!   ([`crate::core::Screens`]' rule, `screenFlow` in Kotlin).
//! - **No JSON crosses.** A card says what it wants (`genre_tag_edit(id, patch)`
//!   takes a JSON *string*, `genre_bulk_preview(scope)` a JSON *string*): a body
//!   the Kotlin side builds is data, and the keys are the server's own — a body
//!   is never a type this boundary has to keep in step.
//! - **Long work says so (rule 2)**: `genre_train_start` returns at once, the
//!   screen reads `genre_training()` (or subscribes a [`GenreListener`]), and a
//!   failure carries its French CAUSE. The same for the local measure pass
//!   ([`GenreMeasureListener`]).
//! - **A list is a `ListHandle`** (rule 4): the tree's 1 000 rows never cross
//!   whole; [`Screens::genre_tree_list`] hands the screen a handle the design
//!   system's `rememberListWindow` pages through, so the 1 000-node acceptance is
//!   measured on the real path.
//!
//! UniFFI merges this `impl Screens` block into the ONE Kotlin `Screens` class, so
//! no shared file is touched (MB0's design, "The FFI boundary").

use std::sync::Arc;

use nz_core::genres as core_genres;

use crate::core::Screens;
use crate::errors::NzError;
use crate::lists::{ListHandle, TrackRow};

// --- records -------------------------------------------------------------------

/// One tag of the vocabulary, as a chip draws it.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct GenreTagRow {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
    pub archetype: Option<String>,
    /// The engine's family for this label, when it is one.
    pub family: Option<String>,
    /// How many library tracks carry it.
    pub count: u32,
}

impl From<core_genres::TagView> for GenreTagRow {
    fn from(tag: core_genres::TagView) -> Self {
        GenreTagRow {
            id: tag.id,
            name: tag.name,
            color: tag.color,
            archetype: tag.archetype,
            family: tag.family,
            count: tag.count,
        }
    }
}

/// The frozen ONNX extractor, as its card reads it.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct GenreExtractor {
    pub available: bool,
    pub dim: u32,
    pub model: Option<String>,
    /// The ONNX runtime is installed (a boolean, never a version).
    pub runtime: bool,
    pub reason: Option<String>,
    pub session_state: Option<String>,
    pub session_error: Option<String>,
}

impl From<core_genres::ExtractorView> for GenreExtractor {
    fn from(info: core_genres::ExtractorView) -> Self {
        GenreExtractor {
            available: info.available,
            dim: info.dim,
            model: info.model,
            runtime: info.runtime,
            reason: info.reason,
            session_state: info.session_state,
            session_error: info.session_error,
        }
    }
}

/// The active head, as *Modèle*'s state card reads it.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct GenreModelRow {
    /// The server can read it back; a head it cannot load is ignored.
    pub usable: bool,
    pub version: Option<i64>,
    pub created: Option<String>,
    pub labels: Vec<String>,
    pub kind: String,
    pub dim: u32,
    /// `metrics.health.level`: `good`, `watch` or `poor`.
    pub health: Option<String>,
    pub balanced: Option<f64>,
}

impl From<core_genres::ModelView> for GenreModelRow {
    fn from(model: core_genres::ModelView) -> Self {
        GenreModelRow {
            usable: model.usable,
            version: model.version,
            created: model.created,
            labels: model.labels,
            kind: model.kind,
            dim: model.dim,
            health: model.health,
            balanced: model.balanced,
        }
    }
}

/// The vocabulary and everything the studio knows before it shows anything.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct GenreStudio {
    pub archetypes: Vec<String>,
    pub tags: Vec<GenreTagRow>,
    pub labelled: u32,
    pub model: Option<GenreModelRow>,
    pub extractor: GenreExtractor,
}

impl From<core_genres::StudioView> for GenreStudio {
    fn from(view: core_genres::StudioView) -> Self {
        GenreStudio {
            archetypes: view.archetypes,
            tags: view.tags.into_iter().map(GenreTagRow::from).collect(),
            labelled: view.labelled,
            model: view.model.map(GenreModelRow::from),
            extractor: view.extractor.into(),
        }
    }
}

/// One candidate of *Étiquetage*.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct GenreCandidate {
    pub reference: String,
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_s: u32,
    pub play_count: u32,
    pub bpm: Option<f64>,
    pub predicted: Option<String>,
    pub confidence: Option<f64>,
    pub uncertainty: Option<f64>,
}

impl From<core_genres::CandidateView> for GenreCandidate {
    fn from(row: core_genres::CandidateView) -> Self {
        GenreCandidate {
            reference: row.reference,
            id: row.id,
            title: row.title,
            artist: row.artist,
            album: row.album,
            duration_s: row.duration_s,
            play_count: row.play_count,
            bpm: row.bpm,
            predicted: row.predicted,
            confidence: row.confidence,
            uncertainty: row.uncertainty,
        }
    }
}

/// One page of *Étiquetage*.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct GenreCandidates {
    pub rows: Vec<GenreCandidate>,
    pub labelled: u32,
    pub sort: String,
    pub truncated: bool,
}

impl From<core_genres::CandidatesView> for GenreCandidates {
    fn from(view: core_genres::CandidatesView) -> Self {
        GenreCandidates {
            rows: view.rows.into_iter().map(GenreCandidate::from).collect(),
            labelled: view.labelled,
            sort: view.sort,
            truncated: view.truncated,
        }
    }
}

/// One genre of the ranked vocabulary, as a chip draws it.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct GenreRankedTag {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
    pub count: u32,
    /// The blended score, `None` when neither opinion exists.
    pub score: Option<f64>,
    pub head: Option<f64>,
    pub proto: Option<f64>,
}

impl From<core_genres::RankedTagView> for GenreRankedTag {
    fn from(row: core_genres::RankedTagView) -> Self {
        GenreRankedTag {
            id: row.id,
            name: row.name,
            color: row.color,
            count: row.count,
            score: row.score,
            head: row.head,
            proto: row.proto,
        }
    }
}

/// The tag sheet of one track: what it wears, and the vocabulary ranked for it.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct GenreTrackSheet {
    pub reference: String,
    pub tag: Option<GenreTagRow>,
    pub ranked: Vec<GenreRankedTag>,
}

/// The hierarchy's own state (its rows come as a `ListHandle`).
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct GenreTree {
    pub version: String,
    pub default_tree: bool,
    pub can_undo: bool,
    pub nodes: u32,
    pub edges: u32,
    pub roots: Vec<String>,
}

impl From<core_genres::TreeView> for GenreTree {
    fn from(view: core_genres::TreeView) -> Self {
        GenreTree {
            version: view.version,
            default_tree: view.default_tree,
            can_undo: view.can_undo,
            nodes: view.nodes,
            edges: view.edges,
            roots: view.roots,
        }
    }
}

/// One row of the bulk preview.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct GenreBulkRow {
    pub reference: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_s: u32,
    pub current: Option<String>,
    pub same: bool,
    /// `tag`, `model`, or empty.
    pub disagree: String,
    pub opinion: Option<String>,
    pub opinion_confidence: Option<f64>,
    pub opinion_level: String,
}

impl From<core_genres::BulkRowView> for GenreBulkRow {
    fn from(row: core_genres::BulkRowView) -> Self {
        GenreBulkRow {
            reference: row.reference,
            title: row.title,
            artist: row.artist,
            album: row.album,
            duration_s: row.duration_s,
            current: row.current,
            same: row.same,
            disagree: row.disagree,
            opinion: row.opinion,
            opinion_confidence: row.opinion_confidence,
            opinion_level: row.opinion_level,
        }
    }
}

/// The preview's counters.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct GenreBulkCounts {
    pub tracks: u32,
    pub missing: u32,
    pub same: u32,
    pub other_tag: u32,
    pub model: u32,
    pub no_vector: u32,
    pub unjudged: u32,
}

/// `POST /genre/bulk/preview`, as the sheet draws it.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct GenreBulkPreview {
    pub tag: Option<GenreTagRow>,
    pub rows: Vec<GenreBulkRow>,
    pub counts: GenreBulkCounts,
    pub head_active: bool,
    pub head_knows: bool,
}

impl From<core_genres::BulkPreviewView> for GenreBulkPreview {
    fn from(view: core_genres::BulkPreviewView) -> Self {
        GenreBulkPreview {
            tag: view.tag.map(GenreTagRow::from),
            rows: view.rows.into_iter().map(GenreBulkRow::from).collect(),
            counts: GenreBulkCounts {
                tracks: view.counts.tracks,
                missing: view.counts.missing,
                same: view.counts.same,
                other_tag: view.counts.other_tag,
                model: view.counts.model,
                no_vector: view.counts.no_vector,
                unjudged: view.counts.unjudged,
            },
            head_active: view.head_active,
            head_knows: view.head_knows,
        }
    }
}

/// `POST /genre/bulk`'s answer.
#[derive(uniffi::Record, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GenreBulkApplied {
    pub applied: u32,
    /// The ids the request named that the library does not hold (counted, never
    /// created — favouriting archives them).
    pub missing: u32,
}

impl From<core_genres::BulkAppliedView> for GenreBulkApplied {
    fn from(view: core_genres::BulkAppliedView) -> Self {
        GenreBulkApplied {
            applied: view.applied,
            missing: view.missing,
        }
    }
}

/// One cause of a failed unit, counted (`JobFailures.svelte`'s ranked list).
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct GenreCause {
    pub reason: String,
    pub count: u32,
}

/// One failed row a job kept a sample of.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct GenreFailure {
    pub title: String,
    pub reason: String,
}

/// `GET /genre/embed` — measuring every archived track that has no vector.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct GenreEmbedJob {
    pub running: bool,
    pub scanned: u32,
    pub done: u32,
    pub failed: u32,
    pub total: u32,
    pub error: Option<String>,
    pub causes: Vec<GenreCause>,
    pub failures: Vec<GenreFailure>,
    pub force: bool,
}

impl From<core_genres::EmbedJobView> for GenreEmbedJob {
    fn from(job: core_genres::EmbedJobView) -> Self {
        GenreEmbedJob {
            running: job.running,
            scanned: job.scanned,
            done: job.done,
            failed: job.failed,
            total: job.total,
            error: job.error,
            causes: job
                .causes
                .into_iter()
                .map(|cause| GenreCause {
                    reason: cause.reason,
                    count: cause.count,
                })
                .collect(),
            failures: job
                .failures
                .into_iter()
                .map(|row| GenreFailure {
                    title: row.title,
                    reason: row.reason,
                })
                .collect(),
            force: job.force,
        }
    }
}

/// `GET /genre/relabel` — the stored verdicts re-decided under the active head.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct GenreRelabelJob {
    pub running: bool,
    pub scanned: u32,
    pub total: u32,
    pub changed: u32,
    pub by_tag: u32,
    pub by_model: u32,
    pub by_heuristic: u32,
    pub by_family: u32,
    pub error: Option<String>,
}

impl From<core_genres::RelabelView> for GenreRelabelJob {
    fn from(job: core_genres::RelabelView) -> Self {
        GenreRelabelJob {
            running: job.running,
            scanned: job.scanned,
            total: job.total,
            changed: job.changed,
            by_tag: job.by_tag,
            by_model: job.by_model,
            by_heuristic: job.by_heuristic,
            by_family: job.by_family,
            error: job.error,
        }
    }
}

/// How much of the library has a construction summary (the player writes it).
#[derive(uniffi::Record, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GenreConstruction {
    pub labelled: u32,
    pub measured: u32,
    /// Hand-labelled tracks with no summary yet.
    pub missing: u32,
}

/// The *Modèle* tab's cards together.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct GenreModelTab {
    pub model: Option<GenreModelRow>,
    pub relabel: Option<GenreRelabelJob>,
    pub embed: Option<GenreEmbedJob>,
    pub construction: Option<GenreConstruction>,
}

impl From<core_genres::ModelTabView> for GenreModelTab {
    fn from(view: core_genres::ModelTabView) -> Self {
        GenreModelTab {
            model: view.model.map(GenreModelRow::from),
            relabel: view.relabel.map(GenreRelabelJob::from),
            embed: view.embed.map(GenreEmbedJob::from),
            construction: view.construction.map(|view| GenreConstruction {
                labelled: view.labelled,
                measured: view.measured,
                missing: view.missing,
            }),
        }
    }
}

/// One genre's line of the per-class table.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct GenreClass {
    pub label: String,
    pub examples: u32,
    pub recall: f64,
    pub precision: f64,
}

/// Two labels the held-out head keeps swapping.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct GenreConfusion {
    pub from: String,
    pub to: String,
    pub count: u32,
    pub share: f64,
}

/// A finished training run, as *Modèle* draws it.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct GenreHead {
    pub kind: String,
    pub labels: Vec<String>,
    pub dim: u32,
    pub hidden: u32,
    pub hidden2: u32,
    pub examples: u32,
    pub classes: u32,
    pub folds: u32,
    pub accuracy: f64,
    pub balanced: f64,
    /// The same fits on their OWN training rows: the memorising gap.
    pub fit_balanced: f64,
    pub temperature: f64,
    /// Whole artists were held out.
    pub grouped: bool,
    pub projected: bool,
    pub bagged: u32,
    pub seed: u32,
    pub skipped: u32,
    /// What the clean-up did, in sentences.
    pub cleanup: Vec<String>,
    pub per_class: Vec<GenreClass>,
    pub confusions: Vec<GenreConfusion>,
    /// `good`, `watch` or `poor`.
    pub health_level: String,
    pub health_remarks: Vec<String>,
}

impl From<core_genres::HeadView> for GenreHead {
    fn from(head: core_genres::HeadView) -> Self {
        GenreHead {
            kind: head.kind,
            labels: head.labels,
            dim: head.dim,
            hidden: head.hidden,
            hidden2: head.hidden2,
            examples: head.examples,
            classes: head.classes,
            folds: head.folds,
            accuracy: head.accuracy,
            balanced: head.balanced,
            fit_balanced: head.fit_balanced,
            temperature: head.temperature,
            grouped: head.grouped,
            projected: head.projected,
            bagged: head.bagged,
            seed: head.seed,
            skipped: head.skipped,
            cleanup: head.cleanup,
            per_class: head
                .per_class
                .into_iter()
                .map(|row| GenreClass {
                    label: row.label,
                    examples: row.examples,
                    recall: row.recall,
                    precision: row.precision,
                })
                .collect(),
            confusions: head
                .confusions
                .into_iter()
                .map(|row| GenreConfusion {
                    from: row.from,
                    to: row.to,
                    count: row.count,
                    share: row.share,
                })
                .collect(),
            health_level: head.health.level,
            health_remarks: head.health.remarks,
        }
    }
}

/// The training run's own state — what a recomposed screen reads.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct GenreTraining {
    pub running: bool,
    pub stage: String,
    /// The furthest the reported stages reached, 0..1.
    pub fraction: f64,
    pub cause: Option<String>,
    pub mode: String,
    pub self_train: bool,
    pub examples: u32,
    pub pool: u32,
    pub head: Option<GenreHead>,
}

impl From<core_genres::TrainingState> for GenreTraining {
    fn from(state: core_genres::TrainingState) -> Self {
        GenreTraining {
            running: state.running,
            stage: state.stage,
            fraction: state.fraction,
            cause: state.cause,
            mode: state.mode,
            self_train: state.self_train,
            examples: state.examples,
            pool: state.pool,
            head: state.head.map(GenreHead::from),
        }
    }
}

/// `GET /api/genre/extractor/test` — the extractor's built-in sanity check.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct GenreExtractorTest {
    pub running: bool,
    /// `None` while it has not finished.
    pub ok: Option<bool>,
    pub error: Option<String>,
    /// The per-track lines the check printed.
    pub progress: Vec<String>,
}

impl From<core_genres::ExtractorTestView> for GenreExtractorTest {
    fn from(view: core_genres::ExtractorTestView) -> Self {
        GenreExtractorTest {
            running: view.running,
            ok: view.ok,
            error: view.error,
            progress: view.progress,
        }
    }
}

/// What `PUT /genre/model` did.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct GenreSent {
    pub labels: u32,
    pub relabel: Option<GenreRelabelJob>,
}

/// The local measure pass's state.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct GenreMeasure {
    pub running: bool,
    pub done: u32,
    pub failed: u32,
    pub total: u32,
    pub current: String,
    pub causes: Vec<GenreCause>,
    pub stopped: Option<String>,
}

impl From<core_genres::MeasureState> for GenreMeasure {
    fn from(state: core_genres::MeasureState) -> Self {
        GenreMeasure {
            running: state.running,
            done: state.done,
            failed: state.failed,
            total: state.total,
            current: state.current,
            causes: state
                .causes
                .into_iter()
                .map(|(reason, count)| GenreCause { reason, count })
                .collect(),
            stopped: state.stopped,
        }
    }
}

// --- listeners -------------------------------------------------------------------

/// The training run's progress, delivered on the runtime's worker: Kotlin hands
/// it to a `Flow`/channel and returns at once (`docs/agents/mobile.md`,
/// *Pitfalls*).
#[uniffi::export(callback_interface)]
pub trait GenreListener: Send + Sync {
    fn progress(&self, stage: String, fraction: f64);
    /// The run ended; `cause` is its French sentence when it produced no head.
    fn finished(&self, ok: bool, cause: Option<String>);
}

/// The measure pass's progress: one message per track, with the CAUSE of each
/// failure counted.
#[uniffi::export(callback_interface)]
pub trait GenreMeasureListener: Send + Sync {
    fn changed(&self, state: GenreMeasure);
}

struct TrainingRelay(Box<dyn GenreListener>);

impl core_genres::TrainingSink for TrainingRelay {
    fn progress(&self, stage: &str, fraction: f64) {
        self.0.progress(stage.to_owned(), fraction);
    }

    fn finished(&self, ok: bool, cause: Option<&str>) {
        self.0.finished(ok, cause.map(str::to_owned));
    }
}

struct MeasureRelay(Box<dyn GenreMeasureListener>);

impl core_genres::MeasureSink for MeasureRelay {
    fn changed(&self, state: &core_genres::MeasureState) {
        self.0.changed(GenreMeasure::from(state.clone()));
    }
}

// --- the screens -------------------------------------------------------------------

impl Screens {
    /// The process's studio, bound to this core once (as MB15's offline service
    /// is). Idempotent: two calls are one studio and one set of job threads.
    fn studio(&self) -> Arc<core_genres::Studio> {
        core_genres::attach(self.core())
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl Screens {
    /// The device's copy of the vocabulary — the first frame, no request.
    pub fn genre_cached_studio(&self) -> Option<GenreStudio> {
        self.studio().cached_studio().map(GenreStudio::from)
    }

    /// `GET /api/genre/status` — the vocabulary, its counts, the active head and
    /// the extractor. ONE request.
    pub async fn genre_studio(&self) -> Result<GenreStudio, NzError> {
        Ok(GenreStudio::from(
            self.studio().studio().await.map_err(NzError::from)?,
        ))
    }

    /// The device's copy of one page of candidates, by sort order.
    pub fn genre_cached_candidates(&self, sort: String) -> Option<GenreCandidates> {
        self.studio()
            .cached_candidates(&sort)
            .map(GenreCandidates::from)
    }

    /// `GET /api/genre/candidates?limit=40&sort=&last=` — what to tag next. ONE
    /// request.
    pub async fn genre_candidates(
        &self,
        sort: String,
        last: Option<String>,
    ) -> Result<GenreCandidates, NzError> {
        Ok(GenreCandidates::from(
            self.studio()
                .candidates(&sort, last.as_deref())
                .await
                .map_err(NzError::from)?,
        ))
    }

    /// The vocabulary ranked for one reference — the SPA's `rankTags`, from the
    /// caches. ZERO requests (the candidate's own ranking came with the page).
    pub fn genre_ranked(&self, reference: String, query: String) -> Vec<GenreRankedTag> {
        self.studio()
            .ranked(&reference, &query)
            .into_iter()
            .map(GenreRankedTag::from)
            .collect()
    }

    /// `GET /api/genre/track/<id>` — one track's tag and the vocabulary ranked
    /// for it (the tag sheet MB10's menu opens). One request.
    pub async fn genre_track_sheet(&self, reference: String) -> Result<GenreTrackSheet, NzError> {
        let studio = self.studio();
        let sheet = studio.track(&reference).await.map_err(NzError::from)?;
        Ok(GenreTrackSheet {
            reference: sheet.reference.clone(),
            tag: sheet.tag.map(GenreTagRow::from),
            ranked: studio
                .ranked(&reference, "")
                .into_iter()
                .map(GenreRankedTag::from)
                .collect(),
        })
    }

    /// `POST /api/genre/label` — one track, one tag (`None` removes it). ONE
    /// write; the screen has already moved its own state.
    pub async fn genre_label(&self, reference: String, tag: Option<i64>) -> Result<(), NzError> {
        self.studio()
            .label(&reference, tag)
            .await
            .map_err(NzError::from)
    }

    /// `POST /api/genre/tags` — one new tag, then the vocabulary as the server
    /// has it now.
    pub async fn genre_tag_create(
        &self,
        name: String,
        color: Option<String>,
        archetype: Option<String>,
    ) -> Result<GenreStudio, NzError> {
        Ok(GenreStudio::from(
            self.studio()
                .tag_create(&name, color.as_deref(), archetype.as_deref())
                .await
                .map_err(NzError::from)?,
        ))
    }

    /// `PATCH /api/genre/tags/<id>` — only what the sheet changed: a name, a
    /// colour, an archetype (`None` leaves the field alone).
    pub async fn genre_tag_edit(
        &self,
        id: i64,
        name: Option<String>,
        color: Option<String>,
        archetype: Option<String>,
    ) -> Result<GenreStudio, NzError> {
        Ok(GenreStudio::from(
            self.studio()
                .tag_edit_fields(id, name.as_deref(), color.as_deref(), archetype.as_deref())
                .await
                .map_err(NzError::from)?,
        ))
    }

    /// `DELETE /api/genre/tags/<id>`.
    pub async fn genre_tag_delete(&self, id: i64) -> Result<GenreStudio, NzError> {
        Ok(GenreStudio::from(
            self.studio().tag_delete(id).await.map_err(NzError::from)?,
        ))
    }

    /// `POST /api/genre/tags/defaults` — (re)add the engine's genres; only adds
    /// what is missing.
    pub async fn genre_tags_defaults(&self) -> Result<GenreStudio, NzError> {
        Ok(GenreStudio::from(
            self.studio().tags_defaults().await.map_err(NzError::from)?,
        ))
    }

    /// The device's copy of the hierarchy's state.
    pub fn genre_cached_tree(&self) -> Option<GenreTree> {
        self.studio().cached_tree().map(GenreTree::from)
    }

    /// `GET /api/genre/tree` — the hierarchy's state. ONE request.
    pub async fn genre_tree(&self) -> Result<GenreTree, NzError> {
        Ok(GenreTree::from(
            self.studio().tree().await.map_err(NzError::from)?,
        ))
    }

    /// The tree's rows as a windowed list (rule 4): `expanded` is the person's
    /// open row paths (`A/B`), `query` the SPA's search. ZERO requests — the
    /// flattening is local, and the 1 000-node acceptance scrolls this handle
    /// through `rememberListWindow`.
    ///
    /// A tree row travels in the app's one row type: `id` is the genre's name (the
    /// input of every write), `title` its label, `artists` its parent's label,
    /// `album` its engine family, `source` `root`/`branch`/`leaf`,
    /// `duration_s` how many children it has, and `position` its depth (the
    /// indent). One row type for every list is what lets the design system's
    /// window — with its fling policy, measured in MB1 — serve this screen too.
    pub fn genre_tree_list(
        self: Arc<Self>,
        expanded: Vec<String>,
        query: String,
    ) -> Arc<ListHandle> {
        let studio = core_genres::attach(self.core());
        let rows = studio.tree_rows(expanded, &query);
        let rows: Vec<nz_core::lists::Row> = rows
            .into_iter()
            .enumerate()
            .map(|(index, row)| nz_core::lists::Row {
                key: row.key.clone(),
                id: row.name.clone(),
                title: row.name.clone(),
                artists: row.parent.clone().unwrap_or_default(),
                album: row.family.clone().unwrap_or_default(),
                album_id: None,
                duration_s: row.children,
                cover: row.color.clone(),
                explicit: false,
                liked: false,
                disliked: false,
                unavailable: false,
                downloaded: row.open,
                source: if row.depth == 0 {
                    "root".to_owned()
                } else if row.children > 0 {
                    "branch".to_owned()
                } else {
                    "leaf".to_owned()
                },
                position: row.depth,
                played_at: 0,
                added_at: index as u64,
            })
            .collect();
        crate::lists::open_rows(self.core(), rows)
    }

    /// `POST /api/genre/tree/edge` — put `child` under `parent`. ONE write; the
    /// answer is the WHOLE tree.
    pub async fn genre_tree_add(
        &self,
        child: String,
        parent: String,
        weight: f64,
    ) -> Result<GenreTree, NzError> {
        Ok(GenreTree::from(
            self.studio()
                .tree_add(&child, &parent, weight)
                .await
                .map_err(NzError::from)?,
        ))
    }

    /// `DELETE /api/genre/tree/edge` — cut one link (the genre stays).
    pub async fn genre_tree_cut(
        &self,
        child: String,
        parent: String,
    ) -> Result<GenreTree, NzError> {
        Ok(GenreTree::from(
            self.studio()
                .tree_cut(&child, &parent)
                .await
                .map_err(NzError::from)?,
        ))
    }

    /// `POST /api/genre/tree/move` — reparent a node, from a known old parent.
    pub async fn genre_tree_move(
        &self,
        child: String,
        parent: String,
        old_parent: Option<String>,
        weight: f64,
    ) -> Result<GenreTree, NzError> {
        Ok(GenreTree::from(
            self.studio()
                .tree_move(&child, &parent, old_parent.as_deref(), weight)
                .await
                .map_err(NzError::from)?,
        ))
    }

    /// `POST /api/genre/tree/undo` — the last edit, undone.
    pub async fn genre_tree_undo(&self) -> Result<GenreTree, NzError> {
        Ok(GenreTree::from(
            self.studio().tree_undo().await.map_err(NzError::from)?,
        ))
    }

    /// `POST /api/genre/tree/reset` — back to the engine's own tree.
    pub async fn genre_tree_reset(&self) -> Result<GenreTree, NzError> {
        Ok(GenreTree::from(
            self.studio().tree_reset().await.map_err(NzError::from)?,
        ))
    }

    /// `POST /api/genre/bulk/preview` — the scope is ONE of an album, a playlist
    /// or an artist (`GenreBulkSheet.svelte`'s three doors), each with the
    /// server's own id. Two of them, or none, is refused before it travels.
    pub async fn genre_bulk_preview(
        &self,
        album: Option<String>,
        playlist: Option<String>,
        artist: Option<String>,
        tag: String,
    ) -> Result<GenreBulkPreview, NzError> {
        Ok(GenreBulkPreview::from(
            self.studio()
                .bulk_preview_scope(
                    album.as_deref(),
                    playlist.as_deref(),
                    artist.as_deref(),
                    &tag,
                )
                .await
                .map_err(NzError::from)?,
        ))
    }

    /// `POST /api/genre/bulk` — tag the tracks the person kept after the review.
    pub async fn genre_bulk_apply(
        &self,
        tracks: Vec<String>,
        tag: String,
    ) -> Result<GenreBulkApplied, NzError> {
        Ok(GenreBulkApplied::from(
            self.studio()
                .bulk_apply(&tracks, &tag)
                .await
                .map_err(NzError::from)?,
        ))
    }

    /// The device's copy of the *Modèle* tab's cards.
    pub fn genre_cached_model_tab(&self) -> GenreModelTab {
        GenreModelTab::from(self.studio().cached_model_tab())
    }

    /// The *Modèle* tab's three reads (relabel, embed job, construction). THREE
    /// requests; the active head comes from the status already read.
    pub async fn genre_model_tab(&self) -> Result<GenreModelTab, NzError> {
        Ok(GenreModelTab::from(
            self.studio().model_tab().await.map_err(NzError::from)?,
        ))
    }

    /// `GET /api/genre/relabel` — the reclassification's progress.
    pub async fn genre_relabel(&self) -> Result<GenreRelabelJob, NzError> {
        Ok(GenreRelabelJob::from(
            self.studio().relabel().await.map_err(NzError::from)?,
        ))
    }

    /// `POST /api/genre/relabel` — re-decide every stored verdict under the active
    /// head, then the poll.
    pub async fn genre_relabel_start(&self) -> Result<GenreRelabelJob, NzError> {
        Ok(GenreRelabelJob::from(
            self.studio().relabel_start().await.map_err(NzError::from)?,
        ))
    }

    /// `POST /api/genre/embed[?force=1]` — measure every archived track with no
    /// vector (a server job), then read it.
    pub async fn genre_embed_start(&self, force: bool) -> Result<GenreEmbedJob, NzError> {
        Ok(GenreEmbedJob::from(
            self.studio()
                .embed_start(force)
                .await
                .map_err(NzError::from)?,
        ))
    }

    /// `GET /api/genre/embed` — the embed job's progress and causes.
    pub async fn genre_embed(&self) -> Result<GenreEmbedJob, NzError> {
        Ok(GenreEmbedJob::from(
            self.studio().embed().await.map_err(NzError::from)?,
        ))
    }

    /// `GET /api/genre/extractor/test` — the extractor's own check.
    pub async fn genre_extractor_test(&self) -> Result<GenreExtractorTest, NzError> {
        Ok(GenreExtractorTest::from(
            self.studio()
                .extractor_test()
                .await
                .map_err(NzError::from)?,
        ))
    }

    /// `POST /api/genre/extractor/test` — start it, then read it.
    pub async fn genre_extractor_test_start(&self) -> Result<GenreExtractorTest, NzError> {
        Ok(GenreExtractorTest::from(
            self.studio()
                .extractor_test_start()
                .await
                .map_err(NzError::from)?,
        ))
    }

    /// The training run's own state (`None`-free: the default is "never ran").
    pub fn genre_training(&self) -> GenreTraining {
        GenreTraining::from(self.studio().training())
    }

    /// Starts a training run: the matrix is built on the job's own thread (on the
    /// `Bulk` lane) and `nz-analysis`'s driver trains the head. Returns at once.
    ///
    /// `mode` is `auto`, `linear`, `deep` or `deep2`; an unknown word is `auto`.
    /// The run is REPRODUCIBLE: the driver pins its seed, so the same rows give the
    /// same head.
    pub async fn genre_train_start(&self, mode: String, self_train: bool) -> Result<(), NzError> {
        self.studio()
            .start_training(core_genres::mode_of(&mode), self_train)
            .map_err(NzError::from)
    }

    /// Stops the run at its next epoch boundary.
    pub fn genre_train_stop(&self) {
        self.studio().stop_training();
    }

    /// Subscribes a listener to the run's progress (one per screen; the screen
    /// must drop it when it closes).
    pub fn genre_subscribe_training(&self, listener: Box<dyn GenreListener>) {
        self.studio()
            .subscribe_training(Arc::new(TrainingRelay(listener)));
    }

    /// `PUT /api/genre/model` — store the head the trainer built, with each
    /// label's engine family.
    pub async fn genre_send_head(&self) -> Result<GenreSent, NzError> {
        let sent = self.studio().send_head().await.map_err(NzError::from)?;
        Ok(GenreSent {
            labels: sent.labels,
            relabel: sent.relabel.map(GenreRelabelJob::from),
        })
    }

    /// `DELETE /api/genre/model` — back to the server's rules and the person's
    /// tags.
    pub async fn genre_disable_head(&self) -> Result<(), NzError> {
        self.studio().disable_head().await.map_err(NzError::from)
    }

    /// The local measure pass's state.
    pub fn genre_measure(&self) -> GenreMeasure {
        GenreMeasure::from(self.studio().measure())
    }

    /// Starts the local measure pass (`/analysis/todo`, page by page, with
    /// `webapp/measure`'s arithmetic). Resumes from the server's own cursor.
    pub async fn genre_measure_start(&self) -> Result<(), NzError> {
        self.studio().start_measure().map_err(NzError::from)
    }

    /// Stops the measure pass at the next track boundary.
    pub fn genre_measure_stop(&self) {
        self.studio().stop_measure();
    }

    /// Subscribes a listener to the measure pass (one per screen).
    pub fn genre_subscribe_measure(&self, listener: Box<dyn GenreMeasureListener>) {
        self.studio()
            .subscribe_measure(Arc::new(MeasureRelay(listener)));
    }

    /// Rule 1: while a person waits for sound, both jobs park. The screen raises
    /// this while a track is starting (the desktop's `current_time() < 8 s`).
    pub fn genre_set_yielding(&self, yielding: bool) {
        self.studio().set_yielding(yielding);
    }

    /// Stops both job threads and joins them (worker rule 5).
    ///
    /// `NzCore::shutdown` is MB2's file and does not call it yet; Android kills the
    /// process without a shutdown anyway, so nothing correct depends on it — the
    /// studio's state is the SERVER's (and the device's copy, written atomically),
    /// never a half-finished write. A test and the bridge call it before the core
    /// goes away.
    pub fn genre_shutdown(&self) {
        core_genres::shutdown();
    }
}

/// The row type the tree's list hands out is the app's own: this re-export keeps
/// the FFI file self-describing for a reader looking for `TrackRow`.
pub type GenreTreeRow = TrackRow;
