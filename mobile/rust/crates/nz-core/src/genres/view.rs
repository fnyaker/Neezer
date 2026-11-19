//! The studio's records: what each screen DRAWS, converted from the wire here so
//! no server JSON crosses the FFI boundary (`docs/agents/mobile.md`, *The FFI
//! boundary*).
//!
//! Three shapes are decided here rather than in Kotlin, on purpose:
//!
//! - **The tree is flattened in Rust** ([`tree_rows`]): a genre DAG is a graph,
//!   and the parent/child walk, the search that keeps a match's whole subtree
//!   plus the chain above it, and the row order are the SPA's
//!   (`components/GenreTree.svelte`). Porting that walk to Kotlin would be a
//!   second spelling of it; here it is one function, tested against a 1 000-node
//!   tree.
//! - **The vocabulary is RANKED in Rust** ([`super::rank`]): the order decides
//!   which genre `Entrée` takes, so the phone must agree with the page.
//! - **A head's metrics are read from the trainer's own JSON** ([`head_of`]) and
//!   turned into rows the Model tab draws, including the confusion PAIRS derived
//!   from the matrix (the studio's "same sound or same name?" question).

use std::collections::{BTreeMap, BTreeSet};

use nz_model::{GenreEdge, GenreNode, GenreStatus, GenreTree};

use super::rank::{self, RankedTag, Tag};
use super::wire::{BulkPreviewWire, CandidateWire};

/// One tag of the vocabulary, with how much of the library carries it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TagView {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
    pub archetype: Option<String>,
    /// The engine's family for this label, when it is one (`GenreNode.family`).
    pub family: Option<String>,
    /// Tracks carrying it.
    pub count: u32,
}

/// The vocabulary and everything the studio knows before it shows anything
/// (`GET /api/genre/status`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StudioView {
    /// The archetypes a tag may be given (`groove`, `voice`, …).
    pub archetypes: Vec<String>,
    pub tags: Vec<TagView>,
    /// How many library tracks carry a hand label.
    pub labelled: u32,
    /// The active head, `None` when nobody trained one.
    pub model: Option<ModelView>,
    pub extractor: ExtractorView,
}

impl StudioView {
    pub fn of(status: &GenreStatus) -> StudioView {
        StudioView {
            archetypes: status.archetypes.clone(),
            tags: tags_of(status),
            labelled: status.labelled,
            model: status.model.as_ref().map(model_of),
            extractor: ExtractorView::of(&status.extractor),
        }
    }
}

fn tags_of(status: &GenreStatus) -> Vec<TagView> {
    status
        .tags
        .iter()
        .map(|tag| TagView {
            id: tag.id,
            name: tag.name.clone(),
            color: tag.color.clone(),
            archetype: tag.archetype.clone(),
            family: tag.family.clone(),
            count: status.counts.get(&tag.name).copied().unwrap_or(0),
        })
        .collect()
}

/// One genre of the ranked vocabulary, as a chip draws it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RankedTagView {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
    pub count: u32,
    /// The blended score, `None` when neither opinion exists.
    pub score: Option<f64>,
    pub head: Option<f64>,
    pub proto: Option<f64>,
}

impl From<&RankedTag> for RankedTagView {
    fn from(row: &RankedTag) -> Self {
        RankedTagView {
            id: row.id,
            name: row.name.clone(),
            color: row.color.clone(),
            count: row.used,
            score: row.score,
            head: row.head,
            proto: row.proto,
        }
    }
}

/// The ranker's answer for one reference: `rank` then, when a query is typed,
/// `search`.
pub fn ranked(tags: &[Tag], scores: &[rank::Score], query: &str) -> Vec<RankedTagView> {
    let rows = rank::rank(tags, scores);
    rank::search(&rows, query)
        .into_iter()
        .map(RankedTagView::from)
        .collect()
}

/// One candidate of *Étiquetage*: the track and what the head thinks of it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CandidateView {
    /// The reference every route takes (`deezer_id`, else the row's UUID).
    pub reference: String,
    /// The row's own id, for a cover or a jump to the album.
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_s: u32,
    pub play_count: u32,
    /// The served tempo, when the library measured one.
    pub bpm: Option<f64>,
    pub predicted: Option<String>,
    pub confidence: Option<f64>,
    /// The active-learning order's own number: the smaller, the less sure.
    pub uncertainty: Option<f64>,
}

impl CandidateView {
    pub(super) fn of(row: &CandidateWire) -> CandidateView {
        CandidateView {
            reference: row.reference(),
            id: row.id.clone(),
            title: row.title.clone(),
            artist: row.artist.clone(),
            album: row.album.clone(),
            duration_s: row.duration,
            play_count: row.play_count,
            bpm: row.deezer_bpm,
            predicted: row.predicted.clone(),
            confidence: row.confidence,
            uncertainty: row.uncertainty,
        }
    }
}

/// One page of *Étiquetage*.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CandidatesView {
    pub rows: Vec<CandidateView>,
    /// How many tracks already carry a hand label, as the server counts them.
    pub labelled: u32,
    /// `plays` or `active` — why these rows, in this order.
    pub sort: String,
    /// The scan hit its own cap: there may be more rows.
    pub truncated: bool,
}

/// The tag sheet of one track (`GET /api/genre/track/<id>`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackTagView {
    pub reference: String,
    /// The tag the track wears, `None` when it wears none.
    pub tag: Option<TagView>,
    pub scores: Vec<(String, f64, Option<f64>, Option<f64>)>,
}

/// What the frozen ONNX extractor is, on this server.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExtractorView {
    pub available: bool,
    /// The vector's dimension.
    pub dim: u32,
    /// The operator's model file, when one is in place.
    pub model: Option<String>,
    /// The ONNX runtime is installed (a boolean, never a version).
    pub runtime: bool,
    /// Why it is not available, in the server's own words.
    pub reason: Option<String>,
    /// `"ok"`, `"broken"`, `"checking"` — whether the model loaded.
    pub session_state: Option<String>,
    pub session_error: Option<String>,
}

impl ExtractorView {
    pub fn of(info: &nz_model::ExtractorInfo) -> ExtractorView {
        ExtractorView {
            available: info.available,
            dim: info.dim,
            model: info.model.clone(),
            runtime: info.onnxruntime,
            reason: info.reason.clone(),
            session_state: info.session_state.clone(),
            session_error: info.session_error.clone(),
        }
    }
}

/// The active head, as *Modèle*'s state card reads it (`GenreStatus.model`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelView {
    /// The server can read it back: a head it cannot load is ignored.
    pub usable: bool,
    pub version: Option<i64>,
    pub created: Option<String>,
    pub labels: Vec<String>,
    pub kind: String,
    pub dim: u32,
    /// The stored health verdict (`metrics.health`): its level and its code.
    pub health: Option<String>,
    /// `metrics.balanced`, the score the server gates on.
    pub balanced: Option<f64>,
}

/// The status's head, as the model tab draws it.
pub fn model_summary(status: &GenreStatus) -> Option<ModelView> {
    status.model.as_ref().map(model_of)
}

/// `GET /api/genre/extractor/test` — the extractor's built-in sanity check.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExtractorTestView {
    pub running: bool,
    /// `None` while it has not finished.
    pub ok: Option<bool>,
    pub error: Option<String>,
    /// The per-track lines the check printed.
    pub progress: Vec<String>,
}

impl ExtractorTestView {
    pub fn of(test: &nz_model::ExtractorTest) -> ExtractorTestView {
        ExtractorTestView {
            running: test.running,
            ok: test.ok,
            error: test.error.clone(),
            progress: test.progress.clone(),
        }
    }
}

/// `GET /api/genre/construction`'s coverage, as the card draws it.
pub fn construction_of(job: &nz_model::GenreConstruction) -> ConstructionView {
    ConstructionView {
        labelled: job.labelled,
        measured: job.measured,
        missing: job.missing.len() as u32,
    }
}

fn model_of(value: &serde_json::Value) -> ModelView {
    let metrics = value.get("metrics");
    ModelView {
        usable: value
            .get("usable")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        version: value.get("version").and_then(serde_json::Value::as_i64),
        created: value
            .get("created")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        labels: value
            .get("labels")
            .and_then(serde_json::Value::as_array)
            .map(|labels| {
                labels
                    .iter()
                    .filter_map(|label| label.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        kind: metrics
            .and_then(|metrics| metrics.get("kind"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("linear")
            .to_owned(),
        dim: metrics
            .and_then(|metrics| metrics.get("dim"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default() as u32,
        health: metrics
            .and_then(|metrics| metrics.get("health"))
            .and_then(|health| health.get("level"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        balanced: metrics
            .and_then(|metrics| metrics.get("balanced"))
            .and_then(serde_json::Value::as_f64),
    }
}

// --- the tree ------------------------------------------------------------------

/// One drawn row of the hierarchy: a node UNDER ONE PARENT (a genre with two
/// parents appears twice, as the page draws it), with its own path as the key so
/// expanding one branch never expands the other.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TreeRowView {
    /// `A/B` — the row's identity (the SPA's `row.key`).
    pub key: String,
    pub name: String,
    /// The parent's name, `None` for a root.
    pub parent: Option<String>,
    /// 0 for a root.
    pub depth: u32,
    /// How many children the node has, under any parent.
    pub children: u32,
    /// Whether its children are drawn under it.
    pub open: bool,
    /// It sits under more than one parent (named on the row, as the page does).
    pub multi_parent: bool,
    /// The link's weight: 1 is a sub-genre outright, less is "related".
    pub weight: f64,
    pub color: Option<String>,
    /// The engine's family, when the label is one.
    pub family: Option<String>,
}

/// The tree's own state, without its rows (the rows are a windowed handle).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TreeView {
    pub version: String,
    /// No edit is pending: this is the tree the engine ships.
    pub default_tree: bool,
    /// An edit can be undone.
    pub can_undo: bool,
    pub nodes: u32,
    pub edges: u32,
    /// The roots, in the order a person expects to read them.
    pub roots: Vec<String>,
}

impl TreeView {
    pub fn of(tree: &GenreTree) -> TreeView {
        TreeView {
            version: tree.version.clone(),
            default_tree: tree.default,
            can_undo: tree.can_undo,
            nodes: tree.nodes.len() as u32,
            edges: tree.edges.len() as u32,
            roots: roots_of(tree),
        }
    }
}

fn roots_of(tree: &GenreTree) -> Vec<String> {
    // A root is a genre that has no PARENT — a genre that is nobody's child.
    // (`webui/genre.py#_tree_json`: `[n for n in names if not tree.parents.get(n)]`.)
    let children: BTreeSet<&str> = tree.edges.iter().map(|edge| edge.child.as_str()).collect();
    let mut roots: Vec<String> = tree
        .nodes
        .iter()
        .map(|node| node.name.clone())
        .filter(|name| !children.contains(name.as_str()))
        .collect();
    roots.sort();
    roots
}

/// The tree flattened to the rows a list draws.
///
/// `expanded` holds the ROW paths (`A/B`) the person opened; an empty set draws
/// the roots only, which is what the studio opens on. `query` is the SPA's
/// search: the genres whose name matches, everything under them, and the chain
/// above each so a result sits in its place — while a query is typed the tree is
/// shown open (`GenreTree.svelte#shown`).
pub fn tree_rows(tree: &GenreTree, expanded: &BTreeSet<String>, query: &str) -> Vec<TreeRowView> {
    let mut children: BTreeMap<&str, Vec<&GenreEdge>> = BTreeMap::new();
    let mut parents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for edge in &tree.edges {
        children.entry(edge.parent.as_str()).or_default().push(edge);
        parents
            .entry(edge.child.as_str())
            .or_default()
            .push(edge.parent.as_str());
    }
    // Heaviest link first (1 = a sub-genre outright), then by name: the page's
    // order, so two devices draw the same tree.
    for list in children.values_mut() {
        list.sort_by(|a, b| {
            b.weight
                .partial_cmp(&a.weight)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.child.cmp(&b.child))
        });
    }
    let nodes: BTreeMap<&str, &GenreNode> = tree
        .nodes
        .iter()
        .map(|node| (node.name.as_str(), node))
        .collect();
    let folded = query.trim().to_lowercase();
    let matches = if folded.is_empty() {
        None
    } else {
        Some(keep_set(tree, &children, &parents, &folded))
    };
    let roots = roots_of(tree);
    let mut out = Vec::new();
    for root in roots {
        walk(
            &root,
            0,
            &root,
            None,
            1.0,
            &children,
            &parents,
            &nodes,
            expanded,
            matches.as_ref(),
            &mut out,
        );
    }
    out
}

/// The names a search keeps: every match, everything under it, and the chain
/// above it.
fn keep_set(
    tree: &GenreTree,
    children: &BTreeMap<&str, Vec<&GenreEdge>>,
    parents: &BTreeMap<&str, Vec<&str>>,
    needle: &str,
) -> BTreeSet<String> {
    let mut keep = BTreeSet::new();
    let down = |name: &str, keep: &mut BTreeSet<String>| {
        let mut stack = vec![name.to_owned()];
        while let Some(name) = stack.pop() {
            if !keep.insert(name.clone()) {
                continue;
            }
            for edge in children.get(name.as_str()).into_iter().flatten() {
                stack.push(edge.child.clone());
            }
        }
    };
    let up = |name: &str, keep: &mut BTreeSet<String>| {
        let mut stack = vec![name.to_owned()];
        let mut seen: BTreeSet<String> = BTreeSet::new();
        while let Some(name) = stack.pop() {
            if !seen.insert(name.clone()) {
                continue;
            }
            keep.insert(name.clone());
            for parent in parents.get(name.as_str()).into_iter().flatten() {
                stack.push((*parent).to_owned());
            }
        }
    };
    for node in &tree.nodes {
        if node.name.to_lowercase().contains(needle) {
            down(&node.name, &mut keep);
            up(&node.name, &mut keep);
        }
    }
    keep
}

#[allow(clippy::too_many_arguments)]
fn walk(
    name: &str,
    depth: u32,
    path: &str,
    parent: Option<&str>,
    weight: f64,
    children: &BTreeMap<&str, Vec<&GenreEdge>>,
    parents: &BTreeMap<&str, Vec<&str>>,
    nodes: &BTreeMap<&str, &GenreNode>,
    expanded: &BTreeSet<String>,
    matches: Option<&BTreeSet<String>>,
    out: &mut Vec<TreeRowView>,
) {
    if let Some(keep) = matches
        && !keep.contains(name)
    {
        return;
    }
    let kids = children.get(name).map(Vec::as_slice).unwrap_or_default();
    let open = match matches {
        Some(_) => true,
        None => expanded.contains(path),
    };
    let node = nodes.get(name);
    out.push(TreeRowView {
        key: path.to_owned(),
        name: name.to_owned(),
        parent: parent.map(str::to_owned),
        depth,
        children: kids.len() as u32,
        open,
        multi_parent: parents.get(name).is_some_and(|list| list.len() > 1),
        weight,
        color: node.and_then(|node| node.color.clone()),
        family: node.and_then(|node| node.family.clone()),
    });
    if !open {
        return;
    }
    for edge in kids {
        let child_path = format!("{path}/{}", edge.child);
        walk(
            &edge.child,
            depth + 1,
            &child_path,
            Some(name),
            edge.weight,
            children,
            parents,
            nodes,
            expanded,
            matches,
            out,
        );
    }
}

// --- the bulk sheet --------------------------------------------------------------

/// One row of the bulk preview: what it would do to this track.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BulkRowView {
    pub reference: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_s: u32,
    /// The tag it wears now, `None` when untagged.
    pub current: Option<String>,
    /// It already wears the tag being applied: nothing to do.
    pub same: bool,
    /// `tag` (it wears another one), `model` (the head disagrees), else empty.
    pub disagree: String,
    /// The head's own verdict for this track, when it has one.
    pub opinion: Option<String>,
    pub opinion_confidence: Option<f64>,
    /// `label`, `family` or `ancestor` — how sure the head was.
    pub opinion_level: String,
}

/// The preview's counters, and whether the head could judge at all.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BulkCountsView {
    pub tracks: u32,
    /// Ids the request named that the library does not hold.
    pub missing: u32,
    /// Already wearing this tag.
    pub same: u32,
    /// Wearing ANOTHER tag: applying would overwrite somebody's decision.
    pub other_tag: u32,
    /// The head disagrees past its gate.
    pub model: u32,
    /// No vector (never embedded), so the head had nothing to read.
    pub no_vector: u32,
    /// The preview's time budget ran out before these were judged.
    pub unjudged: u32,
}

/// `POST /genre/bulk` as the sheet reports it: what was tagged, and the ids the
/// library does not hold (counted, never created — favouriting archives them).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BulkAppliedView {
    pub applied: u32,
    pub missing: u32,
}

/// `POST /genre/bulk/preview` as the sheet draws it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BulkPreviewView {
    pub tag: Option<TagView>,
    pub rows: Vec<BulkRowView>,
    pub counts: BulkCountsView,
    pub head_active: bool,
    /// The head knows the tag being applied — without that its "disagreement"
    /// on every track of an unknown genre would bury the real ones.
    pub head_knows: bool,
}

impl BulkPreviewView {
    /// Converts the wire, keeping only the rows the server actually judged (a
    /// row it could not read is a row that is not shown rather than a blank one).
    pub(super) fn of(wire: &BulkPreviewWire) -> BulkPreviewView {
        let tag = wire.tag.as_ref().map(|tag| TagView {
            id: tag.id,
            name: tag.name.clone(),
            color: tag.color.clone(),
            archetype: tag.archetype.clone(),
            family: None,
            count: 0,
        });
        let rows = wire
            .tracks
            .iter()
            .filter_map(|value| {
                serde_json::from_value::<super::wire::BulkRowWire>(value.clone()).ok()
            })
            .map(|row| BulkRowView {
                reference: row.reference(),
                title: row.title,
                artist: row.artist,
                album: row.album,
                duration_s: row.duration,
                current: row.current,
                same: row.same,
                disagree: row.disagree.unwrap_or_default(),
                opinion: row.model.as_ref().map(|model| model.label.clone()),
                opinion_confidence: row.model.as_ref().map(|model| model.conf),
                opinion_level: row
                    .model
                    .as_ref()
                    .map(|model| model.level.clone())
                    .unwrap_or_default(),
            })
            .collect();
        BulkPreviewView {
            tag,
            rows,
            counts: BulkCountsView {
                tracks: wire.counts.tracks,
                missing: wire.counts.missing,
                same: wire.counts.same,
                other_tag: wire.counts.other_tag,
                model: wire.counts.model,
                no_vector: wire.counts.no_vector,
                unjudged: wire.counts.unjudged,
            },
            head_active: wire.model.active,
            head_knows: wire.model.knows,
        }
    }
}

// --- the model tab ---------------------------------------------------------------

/// One cause of a failed unit, counted (`JobFailures.svelte`'s ranked list): the
/// rule is that a job names WHY, never only how many.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CauseView {
    pub reason: String,
    pub count: u32,
}

/// One failed row a job kept a sample of.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FailureRowView {
    pub title: String,
    pub reason: String,
}

/// `GET /api/genre/embed` — measuring every archived track that has no vector
/// (the slow one; a server job, so the phone only starts it and reads it).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EmbedJobView {
    pub running: bool,
    pub scanned: u32,
    pub done: u32,
    pub failed: u32,
    pub total: u32,
    /// The server's own sentence when the job itself broke.
    pub error: Option<String>,
    pub causes: Vec<CauseView>,
    pub failures: Vec<FailureRowView>,
    /// `force`: re-measure the ones that already have a vector.
    pub force: bool,
}

impl EmbedJobView {
    pub fn of(job: &nz_model::GenreEmbed) -> EmbedJobView {
        let mut causes: Vec<CauseView> = job
            .job
            .failure_reasons
            .iter()
            .map(|(reason, count)| CauseView {
                reason: reason.clone(),
                count: *count,
            })
            .collect();
        causes.sort_by_key(|row| std::cmp::Reverse(row.count));
        EmbedJobView {
            running: job.job.running,
            scanned: job.job.scanned,
            done: job.job.done,
            failed: job.job.failed,
            total: job.job.total,
            error: job.job.error.clone(),
            causes,
            failures: job
                .job
                .failures
                .iter()
                .filter_map(|row| {
                    let title = row
                        .get("title")
                        .and_then(serde_json::Value::as_str)
                        .or_else(|| row.get("track").and_then(serde_json::Value::as_str))
                        .unwrap_or_default();
                    let reason = row.get("reason").and_then(serde_json::Value::as_str)?;
                    Some(FailureRowView {
                        title: title.to_owned(),
                        reason: reason.to_owned(),
                    })
                })
                .collect(),
            force: job.force,
        }
    }
}

/// `GET /api/genre/relabel` — the stored verdicts re-decided under the active
/// head, with no re-measure.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RelabelView {
    pub running: bool,
    pub scanned: u32,
    pub total: u32,
    pub changed: u32,
    /// How many verdicts each source decided: a tag, the head, the rules, the
    /// family.
    pub by_tag: u32,
    pub by_model: u32,
    pub by_heuristic: u32,
    pub by_family: u32,
    pub error: Option<String>,
}

impl RelabelView {
    pub fn of(job: &nz_model::GenreRelabel) -> RelabelView {
        RelabelView {
            running: job.running,
            scanned: job.scanned,
            total: job.total,
            changed: job.changed,
            by_tag: job.tag,
            by_model: job.model,
            by_heuristic: job.heuristic,
            by_family: job.family,
            error: job.error.clone(),
        }
    }
}

/// `GET /api/genre/construction` — how much of the library the player has heard
/// enough of to summarise (the head reads it next to the embedding).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConstructionView {
    pub labelled: u32,
    pub measured: u32,
    /// Hand-labelled tracks with no summary yet: what a measure pass would do.
    pub missing: u32,
}

/// The *Modèle* tab's own reads, together: the active head (already in the
/// status), the reclassification, the embed job and the construction coverage.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelTabView {
    pub model: Option<ModelView>,
    pub relabel: Option<RelabelView>,
    pub embed: Option<EmbedJobView>,
    pub construction: Option<ConstructionView>,
}

// --- the head -------------------------------------------------------------------

/// One genre's line of the per-class table: what it was asked to learn, and
/// what the held-out folds did with it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClassView {
    pub label: String,
    pub examples: u32,
    pub recall: f64,
    pub precision: f64,
}

/// Two labels the held-out head keeps swapping: either two genres that sound
/// alike, or one genre tagged inconsistently (`health.js#confusions`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConfusionView {
    pub from: String,
    pub to: String,
    pub count: u32,
    pub share: f64,
}

/// The one verdict plus the sentences worth reading (`health.js#assess`, whose
/// thresholds `nz-analysis` carries).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HealthView {
    /// `good`, `watch` or `poor`.
    pub level: String,
    pub remarks: Vec<String>,
}

/// A finished training run, as *Modèle* draws it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HeadView {
    /// `linear`, `mlp` or `mlp2`.
    pub kind: String,
    pub labels: Vec<String>,
    /// The width one row of the matrix had (2560 or 3840).
    pub dim: u32,
    pub hidden: u32,
    pub hidden2: u32,
    pub examples: u32,
    pub classes: u32,
    pub folds: u32,
    pub accuracy: f64,
    pub balanced: f64,
    /// The same fits on their OWN training rows: against `balanced`, the
    /// memorising gap.
    pub fit_balanced: f64,
    pub temperature: f64,
    /// Whole artists were held out: the score says "the genre", not "the artist".
    pub grouped: bool,
    pub projected: bool,
    pub bagged: u32,
    pub seed: u32,
    /// What the clean-up did, in sentences (`health.js#prepare`).
    pub cleanup: Vec<String>,
    /// Rows left out because their vector was missing.
    pub skipped: u32,
    pub per_class: Vec<ClassView>,
    pub confusions: Vec<ConfusionView>,
    pub health: HealthView,
}

impl HeadView {
    /// Reads the trainer's own `metrics` JSON (`nz-analysis`'s
    /// `training::metrics`) into the rows the tab draws.
    pub fn of(
        outcome: &nz_analysis::training::Outcome,
        prepared: &nz_analysis::training::Prepared,
    ) -> HeadView {
        let metrics = &outcome.metrics;
        let number = |key: &str| metrics.get(key).and_then(serde_json::Value::as_f64);
        let integer = |key: &str| {
            metrics
                .get(key)
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default()
        };
        let per_class: Vec<ClassView> = metrics
            .get("perClass")
            .and_then(serde_json::Value::as_array)
            .map(|rows| {
                rows.iter()
                    .map(|row| ClassView {
                        label: text(row, "label"),
                        examples: row
                            .get("examples")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or_default() as u32,
                        recall: row
                            .get("recall")
                            .and_then(serde_json::Value::as_f64)
                            .unwrap_or_default(),
                        precision: row
                            .get("precision")
                            .and_then(serde_json::Value::as_f64)
                            .unwrap_or_default(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        HeadView {
            kind: outcome.kind.to_owned(),
            labels: outcome.labels.clone(),
            dim: outcome.dim as u32,
            hidden: outcome.hidden,
            hidden2: outcome.hidden2,
            examples: integer("examples") as u32,
            classes: integer("classes") as u32,
            folds: integer("folds") as u32,
            accuracy: number("accuracy").unwrap_or_default(),
            balanced: number("balanced").unwrap_or_default(),
            fit_balanced: number("fitBalanced").unwrap_or_default(),
            temperature: number("temperature").unwrap_or(1.0),
            grouped: metrics
                .get("grouped")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            projected: metrics
                .get("projected")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default()
                > 0,
            bagged: integer("bagged") as u32,
            seed: integer("seed") as u32,
            cleanup: prepared.cleaned.done.clone(),
            skipped: prepared.skipped as u32,
            confusions: confusions_of(metrics, &per_class),
            per_class,
            health: HealthView {
                level: outcome.health.level.to_owned(),
                remarks: outcome.health.remarks.clone(),
            },
        }
    }
}

fn text(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// The pairs read straight off the confusion matrix, most-mixed-up first.
fn confusions_of(metrics: &serde_json::Value, per_class: &[ClassView]) -> Vec<ConfusionView> {
    let Some(flat) = metrics
        .get("confusion")
        .and_then(serde_json::Value::as_array)
    else {
        return Vec::new();
    };
    let classes = per_class.len();
    if classes == 0 || flat.len() < classes * classes {
        return Vec::new();
    }
    let counts: Vec<u32> = flat
        .iter()
        .map(|value| value.as_u64().unwrap_or_default() as u32)
        .collect();
    let mut out = Vec::new();
    for from in 0..classes {
        for to in 0..classes {
            if from == to {
                continue;
            }
            let count = counts[from * classes + to];
            if count == 0 {
                continue;
            }
            let examples = per_class[from].examples.max(1);
            out.push(ConfusionView {
                from: per_class[from].label.clone(),
                to: per_class[to].label.clone(),
                count,
                share: round3(f64::from(count) / f64::from(examples)),
            });
        }
    }
    out.sort_by_key(|row| std::cmp::Reverse(row.count));
    out.truncate(8);
    out
}

fn round3(value: f64) -> f64 {
    (value * 1000.).round_ties_even() / 1000.
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, family: Option<&str>) -> GenreNode {
        GenreNode {
            id: 0,
            name: name.to_owned(),
            family: family.map(str::to_owned),
            archetype: None,
            color: None,
        }
    }

    fn edge(child: &str, parent: &str, weight: f64) -> GenreEdge {
        GenreEdge {
            child: child.to_owned(),
            parent: parent.to_owned(),
            weight,
        }
    }

    /// A genre under TWO parents is drawn once per parent, each with its own
    /// path — so opening one branch never opens the other (`GenreTree.svelte`).
    #[test]
    fn a_two_parent_genre_is_drawn_under_both() {
        let tree = GenreTree {
            nodes: vec![
                node("Hardcore", Some("Hardcore")),
                node("Frenchcore", None),
                node("Gabber", None),
                node("Hardtek", None),
            ],
            edges: vec![
                edge("Frenchcore", "Hardcore", 1.0),
                edge("Gabber", "Hardcore", 1.0),
                edge("Hardtek", "Hardcore", 0.4),
                edge("Hardtek", "Gabber", 0.8),
            ],
            roots: vec![],
            version: "v1".into(),
            default: true,
            can_undo: false,
        };
        let mut expanded = BTreeSet::new();
        expanded.insert("Hardcore".to_owned());
        expanded.insert("Hardcore/Gabber".to_owned());
        let rows = tree_rows(&tree, &expanded, "");
        let keys: Vec<&str> = rows.iter().map(|row| row.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "Hardcore",
                "Hardcore/Frenchcore",
                "Hardcore/Gabber",
                "Hardcore/Gabber/Hardtek",
                "Hardcore/Hardtek",
            ]
        );
        assert_eq!(rows[3].depth, 2);
        assert!(rows[3].multi_parent, "Hardtek sits under two parents");
        assert_eq!(rows[1].parent.as_deref(), Some("Hardcore"));
    }

    /// A closed root draws one row and nothing under it: the studio's first
    /// frame on a 1 000-node tree is a handful of rows, not the whole graph.
    #[test]
    fn a_closed_tree_draws_its_roots_only() {
        let tree = GenreTree {
            nodes: vec![node("A", None), node("B", None), node("C", None)],
            edges: vec![edge("B", "A", 1.0)],
            roots: vec![],
            version: String::new(),
            default: false,
            can_undo: true,
        };
        let rows = tree_rows(&tree, &BTreeSet::new(), "");
        assert_eq!(rows.len(), 2, "A and C have no parent: {rows:?}");
        assert_eq!(rows[0].name, "A");
        assert!(!rows[0].open);
        assert_eq!(rows[0].children, 1);
    }

    /// A search keeps the match, everything under it, and the chain above it —
    /// a result always sits in its place (the page's `shown`).
    #[test]
    fn a_search_keeps_the_subtree_and_the_chain_above() {
        let tree = GenreTree {
            nodes: vec![
                node("Racine", None),
                node("Milieu", None),
                node("Cible", None),
                node("Dessous", None),
                node("Ailleurs", None),
            ],
            edges: vec![
                edge("Milieu", "Racine", 1.0),
                edge("Cible", "Milieu", 1.0),
                edge("Dessous", "Cible", 1.0),
            ],
            roots: vec![],
            version: String::new(),
            default: false,
            can_undo: false,
        };
        let rows = tree_rows(&tree, &BTreeSet::new(), "cible");
        let keys: Vec<&str> = rows.iter().map(|row| row.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "Racine",
                "Racine/Milieu",
                "Racine/Milieu/Cible",
                "Racine/Milieu/Cible/Dessous"
            ]
        );
    }

    /// The pairs come off the matrix, most mixed-up first, and a diagonal is
    /// never a pair.
    #[test]
    fn confusions_are_the_off_diagonal_pairs() {
        let per_class = vec![
            ClassView {
                label: "Hardtek".into(),
                examples: 10,
                recall: 0.5,
                precision: 0.5,
            },
            ClassView {
                label: "Frenchcore".into(),
                examples: 10,
                recall: 0.5,
                precision: 0.5,
            },
        ];
        let metrics = serde_json::json!({ "confusion": [5, 5, 2, 8] });
        let pairs = confusions_of(&metrics, &per_class);
        assert_eq!(pairs.len(), 2);
        assert_eq!(
            (pairs[0].from.as_str(), pairs[0].to.as_str()),
            ("Hardtek", "Frenchcore")
        );
        assert_eq!(pairs[0].count, 5);
        assert_eq!(pairs[0].share, 0.5);
        assert_eq!(pairs[1].count, 2);
    }
}
