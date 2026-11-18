//! The studio's two long jobs, both in Rust and both on their own thread at
//! `nice 10` (rule 1, rule 2):
//!
//! - **the trainer** — `webapp/trainer`'s crate linked natively (through
//!   `nz-analysis`, which is where the desktop's D16 driver lives: one driver for
//!   both apps, so the phone and the desktop train the same head from the same
//!   rows). The matrix is built HERE (`/genre/labelled` + `/genre/embeddings`,
//!   plus `/genre/pool` for self-training), the run is `nz-analysis`'s
//!   [`nz_analysis::training::start`], and its progress crosses to the screen as
//!   one message per stage.
//! - **the local measure** — `nz-analysis`'s [`nz_analysis::analysis::Runner`],
//!   the client side of `docs/agents/analysis-genre.md` (*Analysis on the client
//!   PC*): it walks `/analysis/todo` by cursor, measures each archived track with
//!   `webapp/measure`'s arithmetic and POSTs the numbers; the server decides.
//!
//! Three rules make these jobs behave:
//!
//! - **Yielding is a flag, not a hope.** `Control` parks the run between two
//!   epochs and between two tracks while the studio's yield gate is up; the
//!   screen raises it when a track is starting (rule 1). Measured in
//!   `a_yielding_run_trains_no_epoch`.
//! - **A job says what it is doing, and every failure keeps its CAUSE** — the
//!   trainer's own French sentence, or the measure's counted causes.
//! - **Both threads are JOINED before the tokio runtime is dropped** (worker rule
//!   5): they borrow it with `Handle::block_on`, so [`Studio::shutdown`] joins
//!   them, and `nz-ffi`'s `NzCore::shutdown` calls it.
//!
//! Progress is reported per STAGE, not per epoch: `nz-analysis`'s driver steps
//! the trainer's epochs internally and only reports at a stage boundary. A run is
//! seconds of arithmetic here (linear head 0.66 s, deep stack 0.62 s, the full
//! width retry 9.3 s — the crate's own measures), so the screen shows the stage
//! it is in rather than a percentage nobody measured. Per-epoch reporting would
//! need `trainer` as a DIRECT dependency of `nz-core`, i.e. a line of a shared
//! manifest: asked for, not taken.
//!
//! Resuming: the measure run resumes from the server's own cursor
//! (`/analysis/todo?after=`), so a run cut anywhere continues where it stopped and
//! a re-run never re-sends a finished track. A training run rebuilds its matrix
//! from the server each time, and `nz-analysis`'s driver PINS the seed (1) — the
//! same rows give the same head, bit for bit, so a run stopped and started again
//! is the same run and not a lottery (`nz-analysis`'s `a_seed_repeats_a_run_exactly`
//! holds it).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nz_analysis::training::{self, Mode, Outcome, Prepared};
use nz_analysis::{Control, NICE, Runner};
use serde_json::Value;

use super::view::HeadView;
use super::wire;
use super::{Core, CoreError};

/// Untagged tracks the self-training pool holds at most (`Genres.svelte`).
pub const POOL_MAX: u32 = 600;
/// Tracks per `/genre/embeddings` request — the page's own batch.
pub const EMBED_BATCH: usize = 200;
/// The niceness these two threads ask for (`nz-analysis`'s).
pub const THREAD_NICE: i32 = NICE;

/// How hard to train. `Auto` is the studio's default: the linear head, and a
/// 256-wide network only when the data supports it and it clearly wins held out.
pub use nz_analysis::training::Mode as TrainMode;

/// What a training run reports to the screen, one message at a time.
pub trait TrainingSink: Send + Sync {
    /// The stage's French name and its share of the run, 0..1.
    fn progress(&self, stage: &str, fraction: f64);
    /// The run ended: `cause` is the sentence when it did not produce a head.
    fn finished(&self, ok: bool, cause: Option<&str>);
}

/// What the local measure run reports.
pub trait MeasureSink: Send + Sync {
    fn changed(&self, state: &MeasureState);
}

/// The training run's own state, read by the model tab (and by a poll after a
/// recomposition, which is why it lives beside the sink rather than in it).
#[derive(Debug, Clone, Default)]
pub struct TrainingState {
    pub running: bool,
    /// `préparation`, `vecteurs`, `entraînement`, `réseau`, `terminé`…
    pub stage: String,
    /// The furthest the reported stages reached, 0..1.
    pub fraction: f64,
    /// The head the run produced, with its verdict (`None` while it runs or
    /// after a failure).
    pub head: Option<HeadView>,
    /// Why the run produced nothing, in French (`None` when it succeeded).
    pub cause: Option<String>,
    pub mode: String,
    pub self_train: bool,
    /// The labelled rows the matrix was built from, and the pool's rows.
    pub examples: u32,
    pub pool: u32,
}

impl TrainingState {
    /// The stage as the tab words it, with the last one standing while the run
    /// finishes.
    pub fn label(&self) -> String {
        self.stage.clone()
    }
}

/// What the local measure run reports.
#[derive(Debug, Clone, Default)]
pub struct MeasureState {
    pub running: bool,
    pub done: u32,
    pub failed: u32,
    pub total: u32,
    /// The track in hand, as the card shows it.
    pub current: String,
    /// Every failure's cause, counted — never a bare count (rule 2).
    pub causes: BTreeMap<String, u32>,
    /// Why the run ended early, when it did.
    pub stopped: Option<String>,
}

/// One row of `/genre/labelled`, read the way `nz-analysis`'s trainer takes it.
pub type Labelled = training::Labelled;

/// The matrix one run trains on, and the pool that rides with it.
#[derive(Clone)]
pub struct Matrix {
    pub prepared: Prepared,
    /// Flat vectors (row-major, the prepared width) of untagged tracks.
    pub pool: Vec<f32>,
    /// How many pool rows the common width allowed.
    pub pool_rows: u32,
}

/// Builds the training matrix: the hand-labelled rows, their stored vectors, and
/// (when `self_train`) the untagged pool. Network on the `Bulk` lane — a
/// training run is never the action just taken.
pub async fn build_matrix(core: &Core, self_train: bool) -> Result<Matrix, CoreError> {
    let api = core.genres_api()?;
    let before = wire::totals(&api);
    let answer = read_matrix(&api, self_train).await;
    core.note_genre_traffic(&api, before);
    answer
}

async fn read_matrix(api: &nz_api::Api, self_train: bool) -> Result<Matrix, CoreError> {
    let labelled = wire::labelled(api).await?;
    let rows: Vec<Labelled> = labelled
        .labelled
        .iter()
        .filter_map(|row| serde_json::from_value::<Labelled>(row.clone()).ok())
        .collect();
    if rows.is_empty() {
        return Err(CoreError::Invalid {
            detail: "aucun titre étiqueté : étiquetez d'abord quelques titres".to_owned(),
        });
    }
    let mut vectors: std::collections::HashMap<String, Vec<f32>> = std::collections::HashMap::new();
    for chunk in rows.chunks(EMBED_BATCH) {
        let ids: Vec<String> = chunk.iter().map(|row| row.id.clone()).collect();
        let part = wire::embeddings(api, &ids).await?;
        for (reference, blob) in part.embeddings {
            vectors.insert(reference, training::decode_embedding(&blob));
        }
    }
    let prepared =
        training::prepare(&rows, &vectors).map_err(|detail| CoreError::Invalid { detail })?;
    let mut pool = Vec::new();
    let mut pool_rows = 0;
    if self_train {
        // Best effort: without a pool the head is the one the labels make
        // (`Genres.svelte` swallows this failure the same way).
        if let Ok(answer) = wire::pool(api, POOL_MAX).await {
            let width = prepared.d;
            for chunk in answer.ids.chunks(EMBED_BATCH) {
                let Ok(part) = wire::embeddings(api, chunk).await else {
                    continue;
                };
                for blob in part.embeddings.values() {
                    let vector = training::decode_embedding(blob);
                    if vector.len() < width {
                        continue;
                    }
                    pool.extend_from_slice(&vector[..width]);
                    pool_rows += 1;
                }
            }
        }
    }
    Ok(Matrix {
        prepared,
        pool,
        pool_rows,
    })
}

/// The families a head ships with: each label's engine family, read from the
/// tree the studio already loaded (`GenreNode.family`). A label with no family
/// votes for nothing.
pub fn families(tree: &nz_model::GenreTree, labels: &[String]) -> BTreeMap<String, String> {
    training::families(labels, |label| {
        tree.nodes
            .iter()
            .find(|node| node.name == label)
            .and_then(|node| node.family.clone())
    })
}

/// The body of `PUT /genre/model`, from a finished run.
pub fn payload(outcome: &Outcome, families: &BTreeMap<String, String>) -> Value {
    training::payload(outcome, families)
}

/// The `mode` key the screen stores (`auto`, `linear`, `deep`, `deep2`).
pub fn mode_key(mode: Mode) -> &'static str {
    match mode {
        Mode::Auto => "auto",
        Mode::Linear => "linear",
        Mode::Deep => "deep",
        Mode::Deep2 => "deep2",
    }
}

/// The mode a stored key names (an unknown key is the default, never a panic).
pub fn mode_of(key: &str) -> Mode {
    match key {
        "linear" => Mode::Linear,
        "deep" => Mode::Deep,
        "deep2" => Mode::Deep2,
        _ => Mode::Auto,
    }
}

/// The flag a parked run reads: set while a person waits for sound (rule 1).
///
/// It is one atomic, written by the screen and read by the trainer's thread
/// through `nz-analysis`'s `Control` — no lock either side ever waits on.
#[derive(Debug, Default)]
pub struct YieldGate {
    yielding: AtomicBool,
}

impl YieldGate {
    pub fn set(&self, yielding: bool) {
        self.yielding.store(yielding, Ordering::Release);
    }

    pub fn get(&self) -> bool {
        self.yielding.load(Ordering::Acquire)
    }
}

/// The control a training run parks on, and the sink it reports to.
pub struct Trainer {
    pub control: Arc<Control>,
    pub sink: Arc<dyn TrainingSink>,
}

impl std::fmt::Debug for Trainer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Trainer")
            .field("cancelled", &self.control.is_cancelled())
            .finish()
    }
}

/// Runs one training job IN THE CALLER'S THREAD (the studio spawns it on its own
/// `nz-genre-train` thread): `nz-analysis` starts the trainer on a thread of its
/// own and reports each stage; this loop forwards it and joins that thread.
///
/// Returns the head AND the matrix description the view needs (the clean-up's
/// sentences, the rows left out) — `Prepared` is consumed by the trainer, so it
/// is cloned once.
pub fn run_training(
    runtime: tokio::runtime::Handle,
    matrix: Matrix,
    mode: Mode,
    trainer: &Trainer,
) -> Result<(Outcome, Prepared), String> {
    let sink = Arc::clone(&trainer.sink);
    let kept = matrix.prepared.clone();
    sink.progress("entraînement", 0.2);
    let (handle, receiver) = training::start(
        matrix.prepared,
        mode,
        if matrix.pool_rows > 0 {
            Some(matrix.pool)
        } else {
            None
        },
        Arc::clone(&trainer.control),
    );
    let mut outcome: Option<Outcome> = None;
    let mut cause: Option<String> = None;
    let mut reached = 0.0_f64;
    while let Some(event) = runtime.block_on(receiver.next()) {
        match event {
            training::Event::Progress { stage, fraction } => {
                // `nz-analysis` numbers its stages inside the run; the screen
                // shows the furthest one, never a bar that walks backwards.
                reached = reached.max(fraction.clamp(0.0, 1.0));
                sink.progress(stage, reached);
            }
            training::Event::Done(head) => outcome = Some(*head),
            training::Event::Failed(error) => cause = Some(error),
        }
    }
    let _ = handle.join();
    if trainer.control.is_cancelled() {
        return Err("entraînement interrompu".to_owned());
    }
    match (outcome, cause) {
        (Some(head), _) => Ok((head, kept)),
        (None, Some(error)) => Err(error),
        (None, None) => Err("l'entraînement n'a rien produit".to_owned()),
    }
}

/// Runs the local measure pass on the caller's thread (the studio's
/// `nz-genre-measure`), forwarding `nz-analysis`'s own events.
///
/// The runner owns its own thread; this owns the sentence each failure carries
/// and the state the card draws. `Runner::join` is what makes worker rule 5 true
/// on the way out.
pub fn run_measure(
    core: &Core,
    runtime: tokio::runtime::Handle,
    control: Arc<Control>,
    state: Arc<Mutex<MeasureState>>,
    sink: Arc<dyn MeasureSink>,
) -> Result<(), String> {
    let api = core
        .api()
        .map_err(|error| format!("{error}"))?
        .with_tag(super::TAG_GENRES);
    let spool = core.cache_path("analysis-spool");
    let _ = core.ensure_dir(&spool);
    let (runner, receiver) = Runner::start(Arc::new(api), runtime.clone(), spool, control);
    let mut done = 0;
    let mut failed = 0;
    let mut total = 0;
    let mut causes: BTreeMap<String, u32> = BTreeMap::new();
    let mut stopped = None;
    let mut version: Option<String> = None;
    while let Some(event) = runtime.block_on(receiver.next()) {
        let mut moved = false;
        match event {
            nz_analysis::Event::Started { total: announced } => {
                total = announced.max(total);
                moved = true;
            }
            nz_analysis::Event::Track {
                done: finished,
                failed: broken,
                total: announced,
                title,
                cause,
            } => {
                done = finished;
                failed = broken;
                total = announced.max(total);
                if let Some(cause) = cause {
                    if cause.contains("version") {
                        version = Some(cause.clone());
                    }
                    *causes.entry(cause).or_insert(0) += 1;
                }
                let mut state = state.lock().unwrap_or_else(|err| err.into_inner());
                state.done = done;
                state.failed = failed;
                state.total = total;
                state.current = title;
                state.causes = causes.clone();
                state.stopped = version.clone();
                sink.changed(&state);
                drop(state);
            }
            nz_analysis::Event::Finished {
                done: finished,
                failed: broken,
                causes: ledger,
                stopped: why,
            } => {
                done = finished;
                failed = broken;
                causes = ledger;
                stopped = why;
                moved = true;
            }
        }
        if moved {
            let mut state = state.lock().unwrap_or_else(|err| err.into_inner());
            state.done = done;
            state.failed = failed;
            state.total = total;
            sink.changed(&state);
        }
    }
    runner.join();
    let mut state = state.lock().unwrap_or_else(|err| err.into_inner());
    state.running = false;
    state.done = done;
    state.failed = failed;
    state.total = total;
    state.current.clear();
    state.causes = causes;
    state.stopped = stopped.or(version);
    sink.changed(&state);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The families a head ships with come from the tree, and only for the
    /// labels that HAVE one: a label with no family votes for nothing.
    #[test]
    fn families_come_from_the_tree() {
        let tree = nz_model::GenreTree {
            nodes: vec![
                nz_model::GenreNode {
                    id: 1,
                    name: "Frenchcore".into(),
                    family: Some("Hardcore".into()),
                    archetype: None,
                    color: None,
                },
                nz_model::GenreNode {
                    id: 2,
                    name: "Zebda".into(),
                    family: None,
                    archetype: None,
                    color: None,
                },
            ],
            ..Default::default()
        };
        let labels = vec!["Frenchcore".to_owned(), "Zebda".to_owned()];
        let map = families(&tree, &labels);
        assert_eq!(map.get("Frenchcore").map(String::as_str), Some("Hardcore"));
        assert!(!map.contains_key("Zebda"));
    }

    /// The gate is one atomic and it round-trips: the screen sets it, the parked
    /// run reads it.
    #[test]
    fn the_yield_gate_round_trips() {
        let gate = YieldGate::default();
        assert!(!gate.get());
        gate.set(true);
        assert!(gate.get());
        gate.set(false);
        assert!(!gate.get());
    }

    /// A stored mode key names a mode, and an unknown one is the default: a
    /// setting edited by hand never panics a training run.
    #[test]
    fn a_stored_mode_key_names_a_mode() {
        assert_eq!(mode_of("linear"), Mode::Linear);
        assert_eq!(mode_of("deep2"), Mode::Deep2);
        assert_eq!(mode_of(""), Mode::Auto);
        assert_eq!(mode_of("n_importe_quoi"), Mode::Auto);
        assert_eq!(mode_key(Mode::Deep), "deep");
    }
}
