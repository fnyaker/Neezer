//! The genre studio, the genre tree and the client-side analysis. Owned by
//! **MB18** (`docs/agents/mobile.md`, "Module and file ownership"): the lot fills
//! this directory and edits no other file of the crate.
//!
//! The behaviour is the SPA's, PORTED — `webapp/src/routes/Genres.svelte`, its
//! components (`GenreLibrary`, `GenreTree`, `GenreTagSheet`, `GenreBulkSheet`,
//! `GenreExtractor`) and `webapp/src/lib/genre/*` — and the native reference is
//! the desktop's D16 (`docs/agents/desktop.md`), whose training driver this crate
//! reaches through `nz-analysis`.
//!
//! | This module | The SPA |
//! |---|---|
//! | [`Studio::studio`] | `Genres.svelte#refresh` (`/genre/status`) |
//! | [`Studio::candidates`], [`Studio::label`], [`Studio::ranked`] | the *Étiquetage* tab, `lib/genre/rank.js` |
//! | [`Studio::track`] | `GenreTagSheet.svelte`, `GET /genre/track/<id>` |
//! | [`Studio::tags_defaults`], [`Studio::tag_create`/`tag_edit`/`tag_delete`], [`Studio::embed_start`] | `GenreLibrary.svelte` |
//! | [`Studio::tree`], [`Studio::tree_rows`], [`Studio::tree_add`/`tree_cut`/`tree_move`/`tree_undo`/`tree_reset`] | `GenreTree.svelte` |
//! | [`Studio::bulk_preview`], [`Studio::bulk_apply`] | `GenreBulkSheet.svelte` |
//! | [`Studio::model_tab`], [`Studio::start_training`], [`Studio::training`], [`Studio::send_head`], [`Studio::disable_head`], [`Studio::relabel_start`] | the *Modèle* tab, `lib/genre/health.js`, `lib/genre/trainer.js` |
//! | [`Studio::start_measure`], [`Studio::measure`], [`Studio::stop_measure`] | `lib/genre/measure.js` through `lib/analysis/runner.js` |
//! | [`Studio::extractor_test`], [`Studio::extractor_test_start`] | `GenreExtractor.svelte` |
//!
//! # One object per process, because its state IS the studio
//!
//! The reads fill two caches the ranking needs on every keystroke (the
//! vocabulary with its counts, and each candidate's scores), the tree's last
//! answer, and two job threads that must be joined before the tokio runtime stops
//! (worker rule 5). So the studio is one [`Studio`] per process, [`attach`]ed to
//! the core exactly as MB15's offline service is, and the FFI's `genres.rs` calls
//! it. A plain `Core` method could not own that state without a second copy of it.
//!
//! # The request budget (rule 3)
//!
//! | Screen | Cold | Warm |
//! |---|---|---|
//! | Étiquetage (the studio's first frame) | `/genre/status` + `/genre/candidates` = **2** (the SPA's own 2) | **0** |
//! | Bibliothèque | `/genre/embed` = **1** (the vocabulary is the status's) | **0** |
//! | Arbre | `/genre/tree` = **1** | **0** |
//! | Modèle | `/genre/relabel` + `/genre/embed` + `/genre/construction` = **3** | **0** |
//! | one tag applied / undone | **1** | — |
//! | one tree edit | **1** (the answer is the WHOLE tree, which replaces the copy) | — |
//! | a training run | `/genre/labelled` + ⌈n/200⌉ `/genre/embeddings` (+ `/genre/pool` and its batches when self-training) | — |
//! | the local measure | `/analysis/todo` pages of 20 + one POST per track | — |
//!
//! A warm open asks ZERO blocking requests: [`wire::Snapshot`] holds every card as
//! it was last read.
//!
//! # Long work says so (rule 2)
//!
//! A start call returns at once; the screen polls [`Studio::training`] or listens
//! through a [`TrainingSink`], and every failure carries its CAUSE. Both job
//! threads run at `nice 10` and park while [`Studio::set_yielding`] is up, which
//! the screen raises while a track is starting (rule 1).

mod rank;
#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;
mod train;
mod view;
mod wire;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

pub use rank::{RankedTag, Score, Tag};
pub use train::{
    Labelled, MeasureSink, MeasureState, POOL_MAX, THREAD_NICE, TrainMode, TrainingSink,
    TrainingState, YieldGate, mode_of,
};
pub use view::{
    BulkAppliedView, BulkCountsView, BulkPreviewView, BulkRowView, CandidateView, CandidatesView,
    CauseView, ClassView, ConfusionView, ConstructionView, EmbedJobView, ExtractorTestView,
    ExtractorView, FailureRowView, HeadView, HealthView, ModelTabView, ModelView, RankedTagView,
    RelabelView, StudioView, TagView, TrackTagView, TreeRowView, TreeView,
};

use crate::core::{Core, CoreError};
use nz_analysis::training::{Outcome, Prepared};
use nz_analysis::{Control, current_nice, lower_priority};

/// The tag every request of the studio is counted under (`NzCore::stats`).
pub const TAG_GENRES: &str = "genres";

/// One page of candidates — the SPA's 40 (`Genres.svelte#loadCandidates`).
pub const CANDIDATES_PAGE: u32 = 40;
/// How many references' scores are kept for the ranker (a page of candidates and
/// a handful of tag sheets; the oldest goes when a new one arrives).
pub const SCORE_CACHE_MAX: usize = 96;

/// The two orders the server offers (`/genre/candidates?sort=`).
pub const SORT_PLAYS: &str = "plays";
pub const SORT_ACTIVE: &str = "active";

struct Inner {
    core: Arc<Core>,
    /// The vocabulary and its counts, as the last `/genre/status` gave them.
    tags: Mutex<Vec<Tag>>,
    /// Each candidate's scores, by reference (bounded by [`SCORE_CACHE_MAX`]):
    /// what [`Studio::ranked`] reads on a keystroke.
    scores: Mutex<ScoreCache>,
    /// The tree the last read (or write) answered.
    tree: Mutex<Option<nz_model::GenreTree>>,
    /// Training: its control, its state, its thread and the head it produced.
    control: Mutex<Option<Arc<Control>>>,
    training: Arc<Mutex<TrainingState>>,
    train_thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    head: Mutex<Option<(Outcome, Prepared)>>,
    train_sinks: Mutex<Vec<Arc<dyn TrainingSink>>>,
    /// The local measure pass: `nz-analysis`'s runner owns its thread; this owns
    /// the join, the state and the sinks.
    measure_control: Mutex<Option<Arc<Control>>>,
    measure: Arc<Mutex<MeasureState>>,
    measure_thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    measure_sinks: Mutex<Vec<Arc<dyn MeasureSink>>>,
    /// The yield gate both jobs read (rule 1).
    gate: Arc<YieldGate>,
}

#[derive(Default)]
struct ScoreCache {
    rows: BTreeMap<String, Vec<Score>>,
    order: std::collections::VecDeque<String>,
}

impl ScoreCache {
    fn put(&mut self, reference: &str, scores: Vec<Score>) {
        if self.rows.insert(reference.to_owned(), scores).is_none() {
            self.order.push_back(reference.to_owned());
        }
        while self.order.len() > SCORE_CACHE_MAX {
            if let Some(old) = self.order.pop_front() {
                self.rows.remove(&old);
            }
        }
    }

    fn get(&self, reference: &str) -> Option<&Vec<Score>> {
        self.rows.get(reference)
    }
}

/// The process's studio: the vocabulary cache, the tree, the two jobs.
pub struct Studio {
    inner: Arc<Inner>,
}

static SERVICE: OnceLock<Arc<Studio>> = OnceLock::new();

/// Bind the process's studio to `core`, once.
///
/// Idempotent on purpose: every FFI call reaches it, and the second call must not
/// start a second job thread (two runs over one library would both train the same
/// head and both spend the Bulk lane).
pub fn attach(core: &Arc<Core>) -> Arc<Studio> {
    Arc::clone(SERVICE.get_or_init(|| Studio::start(Arc::clone(core))))
}

/// The studio, when [`attach`] has run.
pub fn get() -> Option<Arc<Studio>> {
    SERVICE.get().cloned()
}

/// Stops both job threads and joins them (worker rule 5) — called by
/// `NzCore::shutdown`, before the runtime it borrows is dropped.
pub fn shutdown() {
    if let Some(studio) = SERVICE.get() {
        studio.shutdown();
    }
}

/// What `PUT /genre/model` did: how many genres the head names, and the
/// reclassification the server started.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SentHead {
    pub labels: u32,
    pub relabel: Option<RelabelView>,
}

impl Studio {
    /// A studio over `core`. Public so a test owns its own (the process's one is
    /// [`attach`]'s).
    pub fn start(core: Arc<Core>) -> Arc<Studio> {
        Arc::new(Studio {
            inner: Arc::new(Inner {
                core,
                tags: Mutex::new(Vec::new()),
                scores: Mutex::new(ScoreCache::default()),
                tree: Mutex::new(None),
                control: Mutex::new(None),
                training: Arc::new(Mutex::new(TrainingState::default())),
                train_thread: Mutex::new(None),
                head: Mutex::new(None),
                train_sinks: Mutex::new(Vec::new()),
                measure_control: Mutex::new(None),
                measure: Arc::new(Mutex::new(MeasureState::default())),
                measure_thread: Mutex::new(None),
                measure_sinks: Mutex::new(Vec::new()),
                gate: Arc::new(YieldGate::default()),
            }),
        })
    }

    /// The core this studio reads through.
    pub fn core(&self) -> &Arc<Core> {
        &self.inner.core
    }

    // --- the vocabulary -----------------------------------------------------

    /// The device's copy of the vocabulary — the first frame, no request.
    pub fn cached_studio(&self) -> Option<StudioView> {
        wire::Snapshot::read(&self.inner.core)
            .status
            .as_ref()
            .map(StudioView::of)
    }

    /// `GET /api/genre/status` — the vocabulary, its counts, the active head and
    /// the extractor. One request, and the SPA's first (`Genres.svelte#refresh`).
    pub async fn studio(&self) -> Result<StudioView, CoreError> {
        let api = self.inner.core.genres_api()?;
        let before = wire::totals(&api);
        let answer = wire::status(&api).await;
        self.inner.core.note_genre_traffic(&api, before);
        let status = answer?;
        let view = StudioView::of(&status);
        self.remember_vocabulary(&view);
        wire::Snapshot::write(&self.inner.core, |snapshot| {
            snapshot.status = Some(status);
        });
        Ok(view)
    }

    fn remember_vocabulary(&self, view: &StudioView) {
        let tags = view
            .tags
            .iter()
            .map(|tag| Tag {
                id: tag.id,
                name: tag.name.clone(),
                color: tag.color.clone(),
                archetype: tag.archetype.clone(),
                used: tag.count,
            })
            .collect();
        *self.lock_tags() = tags;
    }

    fn lock_tags(&self) -> std::sync::MutexGuard<'_, Vec<Tag>> {
        self.inner
            .tags
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }

    /// The vocabulary ranked for one reference — the SPA's `rankTags`, from the
    /// caches (ZERO requests). An unknown reference ranks the vocabulary alone,
    /// which is what the tag sheet of a track nobody measured gets.
    pub fn ranked(&self, reference: &str, query: &str) -> Vec<RankedTagView> {
        let tags = self.lock_tags().clone();
        let scores = self
            .inner
            .scores
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .get(reference)
            .cloned()
            .unwrap_or_default();
        view::ranked(&tags, &scores, query)
    }

    /// The same, from an explicit vocabulary — the FFI's tag sheet passes the
    /// rows it holds and asks for the order, so no second ranking exists in
    /// Kotlin.
    pub fn ranked_with(tags: &[Tag], scores: &[Score], query: &str) -> Vec<RankedTagView> {
        view::ranked(tags, scores, query)
    }

    // --- tagging ------------------------------------------------------------

    /// The device's copy of one page of candidates.
    pub fn cached_candidates(&self, sort: &str) -> Option<CandidatesView> {
        let snapshot = wire::Snapshot::read(&self.inner.core);
        let copy = snapshot.candidates.get(order(sort).key())?;
        Some(candidates_view(copy))
    }

    /// `GET /api/genre/candidates?limit=&sort=&last=` — the tracks to tag next,
    /// 40 at a time. The rows' scores are kept for [`Studio::ranked`].
    pub async fn candidates(
        &self,
        sort: &str,
        last: Option<&str>,
    ) -> Result<CandidatesView, CoreError> {
        let sort = order(sort);
        let api = self.inner.core.genres_api()?;
        let before = wire::totals(&api);
        let answer = wire::candidates(&api, CANDIDATES_PAGE, sort.wire(), last).await;
        self.inner.core.note_genre_traffic(&api, before);
        let page = answer?;
        let rows: Vec<wire::CandidateWire> = page
            .candidates
            .iter()
            .filter_map(|row| serde_json::from_value(row.clone()).ok())
            .collect();
        for row in &rows {
            self.remember_scores(&row.reference(), row.score_tuples());
        }
        let view = CandidatesView {
            rows: rows.iter().map(CandidateView::of).collect(),
            labelled: page.labelled,
            sort: sort.key().to_owned(),
            truncated: page.truncated,
        };
        wire::Snapshot::write(&self.inner.core, |snapshot| {
            snapshot.candidates.insert(
                sort.key().to_owned(),
                wire::CandidatesCopy {
                    rows: page.candidates.clone(),
                    labelled: page.labelled,
                    sort: page.sort.clone(),
                    truncated: page.truncated,
                },
            );
        });
        Ok(view)
    }

    fn remember_scores(&self, reference: &str, scores: Vec<Score>) {
        self.inner
            .scores
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .put(reference, scores);
    }

    /// `GET /api/genre/track/<id>` — one track's tag and the vocabulary ranked
    /// for it (`GenreTagSheet.svelte`). One request; its ranking is cached like a
    /// candidate's, so the sheet's search is local afterwards.
    pub async fn track(&self, reference: &str) -> Result<TrackTagView, CoreError> {
        // A tag sheet opens from a LIST (MB10's menu), before the studio was ever
        // visited: the vocabulary it ranks comes from the device's copy, or from
        // one status read when this process never had it (ZERO requests once the
        // studio has been opened).
        if self.lock_tags().is_empty() {
            if let Some(status) = wire::Snapshot::read(&self.inner.core).status {
                self.remember_vocabulary(&StudioView::of(&status));
            } else {
                self.studio().await?;
            }
        }
        let api = self.inner.core.genres_api()?;
        let before = wire::totals(&api);
        let answer = wire::track(&api, reference).await;
        self.inner.core.note_genre_traffic(&api, before);
        let answer = answer?;
        let scores: Vec<Score> = answer
            .scores
            .iter()
            .map(|score| (score.label.clone(), score.score, score.head, score.proto))
            .collect();
        self.remember_scores(reference, scores.clone());
        let tag = answer.tag.as_ref().map(|tag| TagView {
            id: tag.id,
            name: tag.name.clone(),
            color: tag.color.clone(),
            archetype: tag.archetype.clone(),
            family: None,
            count: self
                .lock_tags()
                .iter()
                .find(|row| row.name == tag.name)
                .map_or(0, |row| row.used),
        });
        Ok(TrackTagView {
            reference: answer.track,
            tag,
            scores,
        })
    }

    /// `POST /api/genre/label` — one track, one tag (`None` removes it). One
    /// request; the screen has already moved its own state (optimistic write,
    /// `Genres.svelte#label`), and a refusal is the caller's to roll back.
    pub async fn label(&self, reference: &str, tag: Option<i64>) -> Result<(), CoreError> {
        let api = self.inner.core.genres_api()?;
        let before = wire::totals(&api);
        let answer = wire::label(&api, reference, tag).await;
        self.inner.core.note_genre_traffic(&api, before);
        answer
    }

    // --- the vocabulary editor ----------------------------------------------

    /// `POST /api/genre/tags` — one new tag, then the vocabulary as the server
    /// has it now (the tag's id comes from the server, never from a guess).
    pub async fn tag_create(
        &self,
        name: &str,
        color: Option<&str>,
        archetype: Option<&str>,
    ) -> Result<StudioView, CoreError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CoreError::Invalid {
                detail: "un genre a besoin d'un nom".to_owned(),
            });
        }
        self.write_then_vocabulary(wire::VocabularyWrite::Create {
            name: name.to_owned(),
            color: color.map(str::to_owned),
            archetype: archetype.map(str::to_owned),
        })
        .await
    }

    /// `PATCH /api/genre/tags/<id>` — a rename, a colour, an archetype. Only the
    /// fields the sheet actually changed travel (`GenreLibrary.svelte` sends a
    /// patch, not the whole tag).
    pub async fn tag_edit_fields(
        &self,
        id: i64,
        name: Option<&str>,
        color: Option<&str>,
        archetype: Option<&str>,
    ) -> Result<StudioView, CoreError> {
        let mut patch = serde_json::Map::new();
        if let Some(name) = name {
            let name = name.trim();
            if name.is_empty() {
                return Err(CoreError::Invalid {
                    detail: "un genre a besoin d'un nom".to_owned(),
                });
            }
            patch.insert("name".to_owned(), serde_json::Value::from(name));
        }
        if let Some(color) = color {
            patch.insert("color".to_owned(), serde_json::Value::from(color));
        }
        if let Some(archetype) = archetype {
            patch.insert("archetype".to_owned(), serde_json::Value::from(archetype));
        }
        if patch.is_empty() {
            return Err(CoreError::Invalid {
                detail: "rien à changer dans ce genre".to_owned(),
            });
        }
        self.tag_edit(id, &serde_json::Value::Object(patch)).await
    }

    /// `PATCH /api/genre/tags/<id>` from a body the caller built.
    pub async fn tag_edit(
        &self,
        id: i64,
        patch: &serde_json::Value,
    ) -> Result<StudioView, CoreError> {
        self.write_then_vocabulary(wire::VocabularyWrite::Edit {
            id,
            patch: patch.clone(),
        })
        .await
    }

    /// `DELETE /api/genre/tags/<id>` — the tag goes, its tracks keep their rows.
    pub async fn tag_delete(&self, id: i64) -> Result<StudioView, CoreError> {
        self.write_then_vocabulary(wire::VocabularyWrite::Delete { id })
            .await
    }

    /// `POST /api/genre/tags/defaults` — (re)add the engine's genres; it only
    /// adds what is missing, so pressing it again is safe.
    pub async fn tags_defaults(&self) -> Result<StudioView, CoreError> {
        self.write_then_vocabulary(wire::VocabularyWrite::Defaults)
            .await
    }

    async fn write_then_vocabulary(
        &self,
        write: wire::VocabularyWrite,
    ) -> Result<StudioView, CoreError> {
        let api = self.inner.core.genres_api()?;
        let before = wire::totals(&api);
        let answer = wire::vocabulary_write(&api, &write).await;
        self.inner.core.note_genre_traffic(&api, before);
        answer?;
        // One read, and the caller gets the SERVER's own vocabulary: the studio
        // never paints a vocabulary it guessed (the tree's rule, applied to the
        // tags).
        self.studio().await
    }

    // --- the tree -----------------------------------------------------------

    /// The device's copy of the hierarchy.
    pub fn cached_tree(&self) -> Option<TreeView> {
        wire::Snapshot::read(&self.inner.core)
            .tree
            .as_ref()
            .map(TreeView::of)
    }

    /// `GET /api/genre/tree` — nodes, edges, roots, version. One request.
    pub async fn tree(&self) -> Result<TreeView, CoreError> {
        let api = self.inner.core.genres_api()?;
        let before = wire::totals(&api);
        let answer = wire::tree(&api).await;
        self.inner.core.note_genre_traffic(&api, before);
        let tree = answer?;
        let view = TreeView::of(&tree);
        self.remember_tree(Some(tree));
        Ok(view)
    }

    fn remember_tree(&self, tree: Option<nz_model::GenreTree>) {
        if let Some(tree) = &tree {
            wire::Snapshot::write(&self.inner.core, |snapshot| {
                snapshot.tree = Some(tree.clone());
            });
        }
        *self.lock_tree() = tree;
    }

    fn lock_tree(&self) -> std::sync::MutexGuard<'_, Option<nz_model::GenreTree>> {
        self.inner
            .tree
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }

    /// The rows a windowed list draws, flattened from the cached tree. ZERO
    /// requests: `expanded` is the person's own set of open row paths, and a
    /// keystroke only re-flattens.
    pub fn tree_rows(&self, expanded: Vec<String>, query: &str) -> Vec<TreeRowView> {
        let expanded: std::collections::BTreeSet<String> = expanded.into_iter().collect();
        // The in-memory copy first, the disk's when this process never read it.
        if let Some(tree) = self.lock_tree().clone() {
            return view::tree_rows(&tree, &expanded, query);
        }
        match wire::Snapshot::read(&self.inner.core).tree {
            Some(tree) => {
                let rows = view::tree_rows(&tree, &expanded, query);
                *self.lock_tree() = Some(tree);
                rows
            }
            None => Vec::new(),
        }
    }

    /// `POST /api/genre/tree/edge` — put `child` under `parent`.
    pub async fn tree_add(
        &self,
        child: &str,
        parent: &str,
        weight: f64,
    ) -> Result<TreeView, CoreError> {
        self.tree_write(wire::TreeEdit::Add, child, parent, None, weight)
            .await
    }

    /// `DELETE /api/genre/tree/edge` — cut one link (the genre stays).
    pub async fn tree_cut(&self, child: &str, parent: &str) -> Result<TreeView, CoreError> {
        self.tree_write(wire::TreeEdit::Cut, child, parent, None, 1.0)
            .await
    }

    /// `POST /api/genre/tree/move` — reparent a node, from a known old parent.
    pub async fn tree_move(
        &self,
        child: &str,
        parent: &str,
        old_parent: Option<&str>,
        weight: f64,
    ) -> Result<TreeView, CoreError> {
        self.tree_write(wire::TreeEdit::Move, child, parent, old_parent, weight)
            .await
    }

    async fn tree_write(
        &self,
        kind: wire::TreeEdit,
        child: &str,
        parent: &str,
        old_parent: Option<&str>,
        weight: f64,
    ) -> Result<TreeView, CoreError> {
        if child.trim().is_empty() || parent.trim().is_empty() {
            return Err(CoreError::Invalid {
                detail: "un lien a besoin de deux genres".to_owned(),
            });
        }
        if child == parent {
            return Err(CoreError::Invalid {
                detail: "un genre ne peut pas être son propre parent".to_owned(),
            });
        }
        let api = self.inner.core.genres_api()?;
        let before = wire::totals(&api);
        let answer = wire::tree_edit(&api, kind, child, parent, old_parent, weight).await;
        self.inner.core.note_genre_traffic(&api, before);
        // Every tree write answers the WHOLE tree (`webui/genre.py#_tree_answer`),
        // which is what the screen paints — never its own guess, and never a second
        // read.
        let tree = answer?;
        let view = TreeView::of(&tree);
        self.remember_tree(Some(tree));
        Ok(view)
    }

    /// `POST /api/genre/tree/undo` — the last edit, undone.
    pub async fn tree_undo(&self) -> Result<TreeView, CoreError> {
        let api = self.inner.core.genres_api()?;
        let before = wire::totals(&api);
        let answer = wire::tree_undo(&api).await;
        self.inner.core.note_genre_traffic(&api, before);
        let tree = answer?;
        let view = TreeView::of(&tree);
        self.remember_tree(Some(tree));
        Ok(view)
    }

    /// `POST /api/genre/tree/reset` — back to the tree the engine ships.
    pub async fn tree_reset(&self) -> Result<TreeView, CoreError> {
        let api = self.inner.core.genres_api()?;
        let before = wire::totals(&api);
        let answer = wire::tree_reset(&api).await;
        self.inner.core.note_genre_traffic(&api, before);
        let tree = answer?;
        let view = TreeView::of(&tree);
        self.remember_tree(Some(tree));
        Ok(view)
    }

    // --- the bulk sheet ------------------------------------------------------

    /// `POST /api/genre/bulk/preview` — what tagging this album, playlist or
    /// artist would do, track by track. One request.
    ///
    /// The scope is the SPA's own object (`GenreBulkSheet.svelte`): `album`,
    /// `playlist` or `artist`, each with the server's own id. Exactly one of them
    /// is set by [`Studio::bulk_scope`], and a request with none is refused before
    /// it travels.
    pub async fn bulk_preview_scope(
        &self,
        album: Option<&str>,
        playlist: Option<&str>,
        artist: Option<&str>,
        tag: &str,
    ) -> Result<BulkPreviewView, CoreError> {
        let scope = bulk_scope(album, playlist, artist)?;
        self.bulk_preview(&scope, tag).await
    }

    /// The scope object one of the three entry points means.
    pub async fn bulk_preview(
        &self,
        scope: &serde_json::Value,
        tag: &str,
    ) -> Result<BulkPreviewView, CoreError> {
        let api = self.inner.core.genres_api()?;
        let before = wire::totals(&api);
        let answer = wire::bulk_preview(&api, scope, tag).await;
        self.inner.core.note_genre_traffic(&api, before);
        let preview = answer?;
        Ok(BulkPreviewView::of(&preview))
    }

    /// `POST /api/genre/bulk` — tag the tracks the person kept after the review.
    /// One request; the server re-decides the verdicts on a worker.
    pub async fn bulk_apply(
        &self,
        tracks: &[String],
        tag: &str,
    ) -> Result<BulkAppliedView, CoreError> {
        if tracks.is_empty() {
            return Err(CoreError::Invalid {
                detail: "aucun titre sélectionné".to_owned(),
            });
        }
        let api = self.inner.core.genres_api()?;
        let before = wire::totals(&api);
        let answer = wire::bulk(&api, tracks, tag).await;
        self.inner.core.note_genre_traffic(&api, before);
        let applied = answer?;
        Ok(BulkAppliedView {
            applied: applied.applied,
            missing: applied.missing,
        })
    }

    // --- the model tab -------------------------------------------------------

    /// The device's copy of the model tab's cards.
    pub fn cached_model_tab(&self) -> ModelTabView {
        let snapshot = wire::Snapshot::read(&self.inner.core);
        ModelTabView {
            model: snapshot.status.as_ref().and_then(view::model_summary),
            relabel: snapshot.relabel.as_ref().map(RelabelView::of),
            embed: snapshot.embed.as_ref().map(EmbedJobView::of),
            construction: snapshot.construction.as_ref().map(view::construction_of),
        }
    }

    /// The *Modèle* tab's three reads: the reclassification, the embed job and
    /// the construction coverage. Three requests; the head comes from the status
    /// the studio already read.
    pub async fn model_tab(&self) -> Result<ModelTabView, CoreError> {
        let api = self.inner.core.genres_api()?;
        let before = wire::totals(&api);
        let relabel = wire::relabel(&api).await;
        let embed = wire::embed_status(&api).await;
        let construction = wire::construction(&api).await;
        self.inner.core.note_genre_traffic(&api, before);
        let relabel = relabel?;
        let embed = embed?;
        let construction = construction?;
        wire::Snapshot::write(&self.inner.core, |snapshot| {
            snapshot.relabel = Some(relabel.clone());
            snapshot.embed = Some(embed.clone());
            snapshot.construction = Some(construction.clone());
        });
        let status = wire::Snapshot::read(&self.inner.core).status;
        Ok(ModelTabView {
            model: status.as_ref().and_then(view::model_summary),
            relabel: Some(RelabelView::of(&relabel)),
            embed: Some(EmbedJobView::of(&embed)),
            construction: Some(view::construction_of(&construction)),
        })
    }

    /// `GET /api/genre/relabel` — the reclassification's progress (a poll, while
    /// it runs).
    pub async fn relabel(&self) -> Result<RelabelView, CoreError> {
        let api = self.inner.core.genres_api()?;
        let before = wire::totals(&api);
        let answer = wire::relabel(&api).await;
        self.inner.core.note_genre_traffic(&api, before);
        let job = answer?;
        wire::Snapshot::write(&self.inner.core, |snapshot| {
            snapshot.relabel = Some(job.clone());
        });
        Ok(RelabelView::of(&job))
    }

    /// `POST /api/genre/relabel` — re-decide every stored verdict under the
    /// active head (no re-measure). One request, then the poll.
    pub async fn relabel_start(&self) -> Result<RelabelView, CoreError> {
        let api = self.inner.core.genres_api()?;
        let before = wire::totals(&api);
        let answer = wire::relabel_start(&api).await;
        self.inner.core.note_genre_traffic(&api, before);
        answer?;
        self.relabel().await
    }

    /// `POST /api/genre/embed[?force=1]` — measure every archived track with no
    /// vector. A server job: this starts it and reads its state (2 requests).
    pub async fn embed_start(&self, force: bool) -> Result<EmbedJobView, CoreError> {
        let api = self.inner.core.genres_api()?;
        let before = wire::totals(&api);
        let answer = wire::embed_start(&api, force).await;
        self.inner.core.note_genre_traffic(&api, before);
        answer?;
        self.embed().await
    }

    /// `GET /api/genre/embed` — the embed job's progress and its failures'
    /// causes (a poll, while it runs).
    pub async fn embed(&self) -> Result<EmbedJobView, CoreError> {
        let api = self.inner.core.genres_api()?;
        let before = wire::totals(&api);
        let answer = wire::embed_status(&api).await;
        self.inner.core.note_genre_traffic(&api, before);
        let job = answer?;
        wire::Snapshot::write(&self.inner.core, |snapshot| {
            snapshot.embed = Some(job.clone());
        });
        Ok(EmbedJobView::of(&job))
    }

    /// `GET /api/genre/extractor/test` — the extractor's own check.
    pub async fn extractor_test(&self) -> Result<ExtractorTestView, CoreError> {
        let api = self.inner.core.genres_api()?;
        let before = wire::totals(&api);
        let answer = wire::extractor_test(&api).await;
        self.inner.core.note_genre_traffic(&api, before);
        Ok(ExtractorTestView::of(&answer?))
    }

    /// `POST /api/genre/extractor/test` — start it. The phone never INSTALLS a
    /// model: a 40 MB ONNX is imported where the operator keeps it (the server's
    /// extractor card, or `[deezer] embed_model`).
    pub async fn extractor_test_start(&self) -> Result<ExtractorTestView, CoreError> {
        let api = self.inner.core.genres_api()?;
        let before = wire::totals(&api);
        let answer = wire::extractor_test_start(&api).await;
        self.inner.core.note_genre_traffic(&api, before);
        answer?;
        self.extractor_test().await
    }

    // --- the trainer --------------------------------------------------------

    /// Subscribes a listener to the training run's progress (one per screen).
    pub fn subscribe_training(&self, sink: Arc<dyn TrainingSink>) {
        self.inner
            .train_sinks
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .push(sink);
    }

    /// The run's own state — what a recomposed screen reads.
    pub fn training(&self) -> TrainingState {
        self.inner
            .training
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    /// Starts a training run on its own thread: the matrix is built there
    /// (`/genre/labelled` + `/genre/embeddings`, on `Bulk`), then
    /// `nz-analysis`'s driver trains the head.
    ///
    /// Returns at once; a run already going is refused (`Conflict`), never
    /// silently doubled.
    pub fn start_training(&self, mode: TrainMode, self_train: bool) -> Result<(), CoreError> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| CoreError::Internal {
            detail: "l'entraînement doit démarrer depuis une tâche du noyau".to_owned(),
        })?;
        {
            let mut state = self
                .inner
                .training
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            if state.running {
                return Err(CoreError::Conflict {
                    detail: "un entraînement est déjà en cours".to_owned(),
                });
            }
            *state = TrainingState {
                running: true,
                stage: "préparation".to_owned(),
                fraction: 0.0,
                mode: train::mode_key(mode).to_owned(),
                self_train,
                ..TrainingState::default()
            };
        }
        let control = Arc::new(Control::default());
        *self
            .inner
            .control
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = Some(Arc::clone(&control));
        let core = Arc::clone(&self.inner.core);
        let inner = Arc::clone(&self.inner);
        let handle = std::thread::Builder::new()
            .name("nz-genre-train".into())
            .stack_size(4 * 1024 * 1024)
            .spawn(move || {
                let _ = lower_priority(THREAD_NICE);
                tracing::debug!(
                    nice = current_nice(),
                    asked = THREAD_NICE,
                    "the trainer thread started"
                );
                let result = train_on(&runtime, &core, mode, self_train, &inner, &control);
                let mut state = inner.training.lock().unwrap_or_else(|err| err.into_inner());
                state.running = false;
                state.fraction = 1.0;
                let mut cause = None;
                match result {
                    Ok((head, prepared, outcome)) => {
                        state.stage = "terminé".to_owned();
                        state.examples = prepared.n as u32;
                        state.head = Some(head);
                        state.cause = None;
                        *inner.head.lock().unwrap_or_else(|err| err.into_inner()) =
                            Some((outcome, prepared));
                    }
                    Err(error) => {
                        state.stage = "échec".to_owned();
                        state.head = None;
                        state.cause = Some(error.clone());
                        cause = Some(error);
                    }
                }
                let sinks = inner
                    .train_sinks
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .clone();
                let ok = cause.is_none();
                drop(state);
                for sink in &sinks {
                    sink.finished(ok, cause.as_deref());
                }
            })
            .map_err(|error| CoreError::Internal {
                detail: format!("le fil d'entraînement n'a pas démarré : {error}"),
            })?;
        *self
            .inner
            .train_thread
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = Some(handle);
        Ok(())
    }

    /// Stops the run at its next epoch boundary (`Control::cancel`).
    pub fn stop_training(&self) {
        if let Some(control) = self
            .inner
            .control
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .as_ref()
        {
            control.cancel();
        }
    }

    /// `PUT /api/genre/model` — store the head the trainer built, with each
    /// label's engine family (the server serves the family when the head is only
    /// sure of a sibling). One request, then the relabel poll.
    pub async fn send_head(&self) -> Result<SentHead, CoreError> {
        let outcome = {
            let guard = self
                .inner
                .head
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            guard
                .as_ref()
                .map(|(outcome, _)| outcome.clone())
                .ok_or_else(|| CoreError::Invalid {
                    detail: "aucun modèle entraîné à envoyer".to_owned(),
                })?
        };
        let tree = self
            .lock_tree()
            .clone()
            .or_else(|| wire::Snapshot::read(&self.inner.core).tree);
        let families = tree
            .as_ref()
            .map(|tree| train::families(tree, &outcome.labels))
            .unwrap_or_default();
        let payload = train::payload(&outcome, &families);
        let api = self.inner.core.genres_api()?;
        let before = wire::totals(&api);
        let answer = wire::model_put(&api, &payload).await;
        self.inner.core.note_genre_traffic(&api, before);
        let answer = answer?;
        let relabel = answer
            .get("relabel")
            .and_then(|job| serde_json::from_value::<nz_model::GenreRelabel>(job.clone()).ok());
        if let Some(job) = &relabel {
            wire::Snapshot::write(&self.inner.core, |snapshot| {
                snapshot.relabel = Some(job.clone());
            });
        }
        Ok(SentHead {
            labels: outcome.labels.len() as u32,
            relabel: relabel.as_ref().map(RelabelView::of),
        })
    }

    /// `DELETE /api/genre/model` — the server goes back to its rules and the
    /// person's tags.
    pub async fn disable_head(&self) -> Result<(), CoreError> {
        let api = self.inner.core.genres_api()?;
        let before = wire::totals(&api);
        let answer = wire::model_delete(&api).await;
        self.inner.core.note_genre_traffic(&api, before);
        answer?;
        *self
            .inner
            .head
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = None;
        Ok(())
    }

    // --- the local measure ---------------------------------------------------

    /// Subscribes a listener to the measure pass.
    pub fn subscribe_measure(&self, sink: Arc<dyn MeasureSink>) {
        self.inner
            .measure_sinks
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .push(sink);
    }

    /// The measure pass's own state.
    pub fn measure(&self) -> MeasureState {
        self.inner
            .measure
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    /// Starts the local measure pass on its own thread: it walks `/analysis/todo`
    /// by cursor (so a run cut anywhere RESUMES with exactly what is unsent),
    /// measures each archived track with `webapp/measure`'s arithmetic and POSTs
    /// the numbers.
    pub fn start_measure(&self) -> Result<(), CoreError> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| CoreError::Internal {
            detail: "la mesure doit démarrer depuis une tâche du noyau".to_owned(),
        })?;
        {
            let mut state = self
                .inner
                .measure
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            if state.running {
                return Err(CoreError::Conflict {
                    detail: "une mesure est déjà en cours".to_owned(),
                });
            }
            *state = MeasureState {
                running: true,
                ..MeasureState::default()
            };
        }
        let control = Arc::new(Control::default());
        *self
            .inner
            .measure_control
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = Some(Arc::clone(&control));
        let core = Arc::clone(&self.inner.core);
        let inner = Arc::clone(&self.inner);
        let sinks = self
            .inner
            .measure_sinks
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        let handle = std::thread::Builder::new()
            .name("nz-genre-measure".into())
            .stack_size(2 * 1024 * 1024)
            .spawn(move || {
                let _ = lower_priority(THREAD_NICE);
                let sink: Arc<dyn MeasureSink> = Arc::new(FanoutMeasure(sinks));
                let state = Arc::clone(&inner.measure);
                if let Err(error) = train::run_measure(&core, runtime, control, state, sink) {
                    let mut state = inner.measure.lock().unwrap_or_else(|err| err.into_inner());
                    state.running = false;
                    state.stopped = Some(error);
                    let sinks = inner
                        .measure_sinks
                        .lock()
                        .unwrap_or_else(|err| err.into_inner())
                        .clone();
                    for sink in &sinks {
                        sink.changed(&state);
                    }
                }
            })
            .map_err(|error| CoreError::Internal {
                detail: format!("le fil de mesure n'a pas démarré : {error}"),
            })?;
        *self
            .inner
            .measure_thread
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = Some(handle);
        Ok(())
    }

    /// Stops the measure pass at the next track boundary.
    pub fn stop_measure(&self) {
        if let Some(control) = self
            .inner
            .measure_control
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .as_ref()
        {
            control.cancel();
        }
    }

    /// Rule 1: while a person waits for sound, both jobs park. The screen sets
    /// this from the player's own state (the desktop reads
    /// `current_time() < 8 s`).
    pub fn set_yielding(&self, yielding: bool) {
        self.inner.gate.set(yielding);
        {
            let control = self
                .inner
                .control
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            if let Some(control) = control.as_ref() {
                control.set_yield(yielding);
            }
        }
        {
            let control = self
                .inner
                .measure_control
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            if let Some(control) = control.as_ref() {
                control.set_yield(yielding);
            }
        }
    }

    /// Whether the jobs are parked — the state a test reads.
    pub fn yields(&self) -> bool {
        self.inner.gate.get()
    }

    /// Joins both job threads. Their loops borrow the tokio runtime with
    /// `Handle::block_on`, so this MUST run before the runtime is dropped
    /// (worker rule 5); `NzCore::shutdown` calls it.
    pub fn shutdown(&self) {
        self.stop_training();
        self.stop_measure();
        let train = self
            .inner
            .train_thread
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .take();
        if let Some(handle) = train {
            let _ = handle.join();
        }
        let measure = self
            .inner
            .measure_thread
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .take();
        if let Some(handle) = measure {
            let _ = handle.join();
        }
    }
}

/// A measure sink that hands each state to every listener the screen
/// subscribed.
struct FanoutMeasure(Vec<Arc<dyn MeasureSink>>);

impl MeasureSink for FanoutMeasure {
    fn changed(&self, state: &MeasureState) {
        for sink in &self.0 {
            sink.changed(state);
        }
    }
}

/// Builds the matrix and trains the head, on the job's own thread.
fn train_on(
    runtime: &tokio::runtime::Handle,
    core: &Arc<Core>,
    mode: TrainMode,
    self_train: bool,
    inner: &Arc<Inner>,
    control: &Arc<Control>,
) -> Result<(HeadView, Prepared, Outcome), String> {
    report_training(inner, "vecteurs", 0.1);
    let matrix = runtime
        .block_on(train::build_matrix(core, self_train))
        .map_err(|error| format!("{error}"))?;
    {
        let mut state = inner.training.lock().unwrap_or_else(|err| err.into_inner());
        state.examples = matrix.prepared.n as u32;
        state.pool = matrix.pool_rows;
    }
    let trainer = train::Trainer {
        control: Arc::clone(control),
        sink: Arc::new(StateSink {
            inner: Arc::clone(inner),
        }),
    };
    let (outcome, prepared) = train::run_training(runtime.clone(), matrix, mode, &trainer)?;
    let head = HeadView::of(&outcome, &prepared);
    Ok((head, prepared, outcome))
}

fn report_training(inner: &Arc<Inner>, stage: &str, fraction: f64) {
    {
        let mut state = inner.training.lock().unwrap_or_else(|err| err.into_inner());
        state.stage = stage.to_owned();
        state.fraction = state.fraction.max(fraction.clamp(0.0, 1.0));
    }
    let sinks = inner
        .train_sinks
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone();
    for sink in &sinks {
        sink.progress(stage, fraction);
    }
}

/// The sink that keeps [`TrainingState`] in step with what the driver reports.
struct StateSink {
    inner: Arc<Inner>,
}

impl TrainingSink for StateSink {
    fn progress(&self, stage: &str, fraction: f64) {
        report_training(&self.inner, stage, fraction);
    }

    fn finished(&self, _ok: bool, _cause: Option<&str>) {
        // The job thread reports the end (it owns the head); this sink only
        // forwards progress, so a driver that ends early cannot end the screen's
        // state twice.
    }
}

/// The one scope a bulk sheet can name: an album, a playlist or an artist (the
/// page's own three doors). More than one — or none — is a body the server would
/// refuse, so it is refused here.
fn bulk_scope(
    album: Option<&str>,
    playlist: Option<&str>,
    artist: Option<&str>,
) -> Result<serde_json::Value, CoreError> {
    let named: Vec<(&str, &str)> = [("album", album), ("playlist", playlist), ("artist", artist)]
        .into_iter()
        .filter_map(|(key, value)| value.map(|value| (key, value)))
        .collect();
    if named.len() != 1 {
        return Err(CoreError::Invalid {
            detail: "un lot désigne un album, une playlist ou un artiste — un seul".to_owned(),
        });
    }
    let mut scope = serde_json::Map::new();
    scope.insert(named[0].0.to_owned(), serde_json::Value::from(named[0].1));
    Ok(serde_json::Value::Object(scope))
}

/// The candidates page as a view, from the device's copy.
fn candidates_view(copy: &wire::CandidatesCopy) -> CandidatesView {
    let rows: Vec<CandidateView> = copy
        .rows
        .iter()
        .filter_map(|row| serde_json::from_value::<wire::CandidateWire>(row.clone()).ok())
        .map(|row| CandidateView::of(&row))
        .collect();
    CandidatesView {
        rows,
        labelled: copy.labelled,
        sort: copy.sort.clone(),
        truncated: copy.truncated,
    }
}

/// The two sort orders, spelled once: an unknown string is the default
/// (`plays`), never a request the server would refuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Order {
    Plays,
    Active,
}

impl Order {
    fn key(self) -> &'static str {
        match self {
            Order::Plays => SORT_PLAYS,
            Order::Active => SORT_ACTIVE,
        }
    }

    fn wire(self) -> Option<&'static str> {
        match self {
            Order::Plays => None,
            Order::Active => Some(SORT_ACTIVE),
        }
    }
}

fn order(sort: &str) -> Order {
    if sort == SORT_ACTIVE {
        Order::Active
    } else {
        Order::Plays
    }
}

#[cfg(test)]
mod order_tests {
    use super::*;

    /// An unknown order is the SPA's default, never a request the server refuses.
    #[test]
    fn an_unknown_order_is_plays() {
        assert_eq!(order("active"), Order::Active);
        assert_eq!(order(""), Order::Plays);
        assert_eq!(order("n_importe_quoi"), Order::Plays);
        assert_eq!(order("active").wire(), Some("active"));
        assert_eq!(order("plays").wire(), None);
    }
}
