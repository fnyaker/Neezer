//! The studio's behaviour, proven against a fake server that speaks its routes
//! and COUNTS them: the SPA's request budget, the optimistic writes, the tree's
//! whole-answer contract, the ranking, the trainer's head — and the acceptance's
//! own number, a real engine that never underruns while a training run works.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use super::testing::{GenreServer, TempDir, cluster_vector};
use super::train::{self, MeasureSink, MeasureState, TrainingSink};
use super::*;

// --- the bench -----------------------------------------------------------------

/// One test's whole world, torn down in the order worker rule 5 demands: the
/// studio's job threads are joined BEFORE the runtime they borrow is dropped.
struct Bench {
    dir: TempDir,
    server: Arc<GenreServer>,
    /// Kept alive for the studio's own reads; the tests name it through
    /// `studio.core()`.
    #[allow(dead_code)]
    core: Arc<Core>,
    studio: Arc<Studio>,
    runtime: tokio::runtime::Runtime,
}

impl Bench {
    fn start(tag: &str) -> Bench {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("a test runtime");
        let dir = TempDir::new(tag);
        let server = Arc::new(GenreServer::start());
        let core = runtime.block_on(server.core(&dir));
        // The bench's own setup (a version probe and a login) is not the studio's
        // budget: the counts start at zero.
        server.forget();
        let studio = Studio::start(Arc::clone(&core));
        Bench {
            dir,
            server,
            core,
            studio,
            runtime,
        }
    }

    /// Twelve labelled rows (six a genre) with their vectors: two artists a
    /// genre, so the trainer holds whole artists out.
    fn with_vectors(self) -> Bench {
        for index in 0..6 {
            self.server
                .store_vector(&format!("ht{index}"), &vector(index, 0));
            self.server
                .store_vector(&format!("fc{index}"), &vector(index, 1));
        }
        self.server.store_vector("p1", &vector(1, 0));
        self.server.store_vector("p2", &vector(1, 1));
        self
    }
}

impl Drop for Bench {
    fn drop(&mut self) {
        self.studio.shutdown();
    }
}

fn vector(index: usize, genre: usize) -> Vec<f32> {
    cluster_vector(index, genre)
}

// --- the request budget ----------------------------------------------------------

/// The first screen's own two reads, and a warm open that asks NOTHING.
#[test]
fn the_studio_costs_the_pages_own_two_requests() {
    let bench = Bench::start("studio-budget");
    let studio = &bench.studio;
    let status = bench.runtime.block_on(studio.studio()).expect("the status");
    assert_eq!(status.tags.len(), 7);
    assert_eq!(status.labelled, 74);
    assert!(status.model.is_none());
    assert!(status.extractor.available);
    assert_eq!(
        status.tags[0].count, 30,
        "the counts ride with the vocabulary"
    );
    let page = bench
        .runtime
        .block_on(studio.candidates(SORT_PLAYS, None))
        .expect("the candidates");
    assert_eq!(page.rows.len(), 2);
    assert_eq!(page.rows[0].reference, "1001");
    assert_eq!(page.rows[0].duration_s, 212);
    // The SPA's own two, and no more.
    assert_eq!(
        bench.server.routes(),
        [
            "GET /api/genre/status".to_owned(),
            "GET /api/genre/candidates".to_owned(),
        ],
        "cold open = 2 requests"
    );

    // Warm: both cards paint from the device's copy, in ZERO requests.
    bench.server.forget();
    assert_eq!(studio.cached_studio().expect("the copy").tags.len(), 7);
    assert_eq!(
        studio
            .cached_candidates(SORT_PLAYS)
            .expect("the page's copy")
            .rows
            .len(),
        2
    );
    assert_eq!(bench.server.count(), 0, "a warm open asks nothing");
}

/// The ranking reads the CANDIDATES' scores, not a second request.
#[test]
fn the_ranking_reads_the_page_it_already_has() {
    let bench = Bench::start("rank");
    let studio = &bench.studio;
    bench.runtime.block_on(studio.studio()).expect("the status");
    bench
        .runtime
        .block_on(studio.candidates(SORT_PLAYS, None))
        .expect("the candidates");
    bench.server.forget();
    let ranked = studio.ranked("1001", "");
    assert_eq!(ranked.len(), 7);
    assert_eq!(ranked[0].name, "Hardtek", "{ranked:?}");
    assert_eq!(ranked[0].head, Some(0.44));
    assert_eq!(ranked[1].name, "Frenchcore");
    // No opinion: the library's own use, then the name.
    assert_eq!(ranked[2].name, "Hardcore");
    let core_page = studio.ranked("1001", "core");
    let found: Vec<&str> = core_page.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(found, ["Frenchcore", "Hardcore"]);
    let top = studio.ranked("1001", "FRENCHCORE");
    let accents: Vec<&str> = top.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(accents, ["Frenchcore"]);
    assert_eq!(bench.server.count(), 0, "ranking a reference is local");
}

/// One tag applied is ONE write, and removing it is another — the SPA's rhythm of
/// one round trip per track (`Genres.svelte#label`).
#[test]
fn a_tag_is_one_write_and_its_removal_is_another() {
    let bench = Bench::start("label");
    bench
        .runtime
        .block_on(bench.studio.label("1001", Some(4)))
        .expect("the tag");
    bench
        .runtime
        .block_on(bench.studio.label("1001", None))
        .expect("the removal");
    assert_eq!(bench.server.count(), 2);
    assert_eq!(
        bench.server.labels(),
        [("1001".to_owned(), Some(4)), ("1001".to_owned(), None)]
    );
}

/// The tag sheet of one track: one request, and its ranking is kept so the
/// sheet's search costs nothing after.
#[test]
fn the_tag_sheet_is_one_request_and_then_local() {
    let bench = Bench::start("tag-sheet");
    let sheet = bench
        .runtime
        .block_on(bench.studio.track("1001"))
        .expect("the sheet");
    assert_eq!(sheet.reference, "1001");
    assert_eq!(
        sheet.tag.as_ref().map(|tag| tag.name.as_str()),
        Some("Hardtek")
    );
    // A sheet opened before the studio ever was: the vocabulary comes with the
    // status (one request), and the ranking is local afterwards.
    assert_eq!(
        bench.server.routes(),
        ["GET /api/genre/status", "GET /api/genre/track/1001"]
    );
    let ranked = bench.studio.ranked("1001", "french");
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].name, "Frenchcore");
    assert_eq!(bench.server.count(), 2);
    // A second sheet, once the vocabulary is here: ONE request.
    bench.server.forget();
    bench
        .runtime
        .block_on(bench.studio.track("1002"))
        .expect("the second sheet");
    assert_eq!(bench.server.count(), 1);
}

/// Every tree write answers the WHOLE tree, and undo puts it back — the screen
/// paints the server's answer, never its own guess.
#[test]
fn a_tree_edit_answers_the_whole_tree_and_undo_puts_it_back() {
    let bench = Bench::start("tree");
    let studio = &bench.studio;
    let tree = bench.runtime.block_on(studio.tree()).expect("the tree");
    assert_eq!(tree.nodes, 6);
    assert_eq!(tree.edges, 5);
    assert!(tree.default_tree);
    assert_eq!(tree.roots, ["Hardcore", "Hardhouse"]);
    assert!(!tree.can_undo);

    let moved = bench
        .runtime
        .block_on(studio.tree_add("Hardstyle", "Hardcore", 1.0))
        .expect("the link");
    assert_eq!(moved.edges, 6, "the answer is the whole tree");
    assert!(moved.can_undo);
    assert_eq!(
        bench.server.last_body_of("POST", "/api/genre/tree/edge"),
        Some(r#"{"child":"Hardstyle","parent":"Hardcore","weight":1.0}"#.to_owned())
    );

    let undone = bench
        .runtime
        .block_on(studio.tree_undo())
        .expect("the undo");
    assert_eq!(undone.edges, 5);
    assert_eq!(
        bench.server.routes().len(),
        3,
        "read, write, undo — no more"
    );
}

/// A genre cannot be its own parent, and the server is never asked: a cycle is
/// refused before the request.
#[test]
fn a_genre_is_never_its_own_parent() {
    let bench = Bench::start("tree-cycle");
    let error = bench
        .runtime
        .block_on(bench.studio.tree_add("Gabber", "Gabber", 1.0))
        .expect_err("the refusal");
    assert_eq!(error.kind(), "invalid");
    assert_eq!(bench.server.count(), 0);
}

/// The rows a list draws are flattened from the copy: expanding a branch or
/// typing a search costs ZERO requests.
#[test]
fn the_tree_rows_are_local() {
    let bench = Bench::start("tree-rows");
    bench
        .runtime
        .block_on(bench.studio.tree())
        .expect("the tree");
    bench.server.forget();
    let closed = bench.studio.tree_rows(Vec::new(), "");
    assert_eq!(closed.len(), 2, "Hardcore and Hardhouse have no parent");
    let mut open = vec!["Hardcore".to_owned()];
    let rows = bench.studio.tree_rows(open.clone(), "");
    assert_eq!(rows[0].name, "Hardcore");
    assert!(rows[0].open);
    assert_eq!(rows.len(), 5, "{rows:?}");
    open.push("Hardcore/Gabber".to_owned());
    let deep = bench.studio.tree_rows(open, "");
    assert!(deep.iter().any(|row| row.key == "Hardcore/Gabber/Hardtek"));
    // A search keeps the match under EVERY parent it sits under, plus the chain
    // above each — the page's `shown` walks paths, not names.
    let searched = bench.studio.tree_rows(Vec::new(), "hardtek");
    assert_eq!(searched.len(), 4, "{searched:?}");
    assert_eq!(bench.server.count(), 0, "the tree is local after one read");
}

/// A refusal reaches the caller with the server's own code, not a bare status.
#[test]
fn a_refusal_keeps_its_cause() {
    let bench = Bench::start("refusal");
    bench
        .server
        .break_code("/api/genre/tree/reset", 409, "cycle");
    let error = bench
        .runtime
        .block_on(bench.studio.tree_reset())
        .expect_err("the refusal");
    assert_eq!(error.kind(), "conflict", "{error}");
    assert!(format!("{error}").contains("cycle"), "{error}");
}

/// The *Modèle* tab: three reads cold, zero warm.
#[test]
fn the_model_tab_costs_three_requests_then_nothing() {
    let bench = Bench::start("model-tab");
    bench
        .runtime
        .block_on(bench.studio.studio())
        .expect("the status");
    bench.server.forget();
    let tab = bench
        .runtime
        .block_on(bench.studio.model_tab())
        .expect("the tab");
    assert!(!tab.relabel.expect("relabel").running);
    let embed = tab.embed.expect("embed");
    assert_eq!(embed.failed, 1);
    assert_eq!(embed.causes[0].reason, "ffmpeg a disparu");
    assert_eq!(embed.failures[0].title, "Casse");
    assert_eq!(tab.construction.expect("construction").missing, 2);
    assert_eq!(bench.server.count(), 3);
    bench.server.forget();
    let warm = bench.studio.cached_model_tab();
    assert_eq!(warm.construction.expect("the copy").measured, 50);
    assert_eq!(bench.server.count(), 0);
}

/// The embed job says what it is doing while it runs, and its failures keep their
/// causes (rule 2): a stateless fake would never exercise the second poll.
#[test]
fn the_embed_job_is_polled_until_it_ends() {
    let bench = Bench::start("embed");
    let started = bench
        .runtime
        .block_on(bench.studio.embed_start(false))
        .expect("start");
    assert!(started.running, "the first answer is the running job");
    let next = bench
        .runtime
        .block_on(bench.studio.embed())
        .expect("the poll");
    assert!(next.running, "still running on the first poll");
    let done = bench
        .runtime
        .block_on(bench.studio.embed())
        .expect("the poll");
    assert!(!done.running, "and finished on the second");
    assert_eq!(done.done, 119);
    assert_eq!(done.failed, 1);
}

/// The bulk sheet: the preview says which tracks disagree, and applying is one
/// write of the tracks the person kept.
#[test]
fn a_bulk_preview_names_the_disagreements() {
    let bench = Bench::start("bulk");
    let preview = bench
        .runtime
        .block_on(
            bench
                .studio
                .bulk_preview(&serde_json::json!({"album": "42"}), "Frenchcore"),
        )
        .expect("the preview");
    assert_eq!(preview.rows.len(), 3);
    assert!(preview.rows[0].same);
    assert_eq!(preview.rows[1].disagree, "tag");
    assert_eq!(preview.rows[1].current.as_deref(), Some("Gabber"));
    assert_eq!(preview.rows[2].disagree, "model");
    assert_eq!(preview.rows[2].opinion.as_deref(), Some("Hardtek"));
    assert_eq!(preview.counts.other_tag, 1);
    assert!(!preview.head_active, "no head yet: the counting says so");
    assert_eq!(
        bench.server.last_body_of("POST", "/api/genre/bulk/preview"),
        Some(r#"{"album":"42","tag":"Frenchcore"}"#.to_owned())
    );
    let applied = bench
        .runtime
        .block_on(
            bench
                .studio
                .bulk_apply(&["2002".to_owned(), "2003".to_owned()], "Frenchcore"),
        )
        .expect("the apply");
    assert_eq!(applied.applied, 2);
    assert_eq!(applied.missing, 0);
    let empty = bench
        .runtime
        .block_on(bench.studio.bulk_apply(&[], "Frenchcore"))
        .expect_err("an empty apply is refused");
    assert_eq!(empty.kind(), "invalid");
}

/// The trainer: twelve labelled rows with two clusters, two artists a genre. It
/// learns them, reports PROGRESS, says what it did, and its head is accepted by
/// the server.
#[test]
fn the_trainer_learns_the_labels_and_says_what_it_did() {
    let bench = Bench::start("train").with_vectors();
    let studio = Arc::clone(&bench.studio);
    let progress = Arc::new(AtomicUsize::new(0));
    let waiter = Arc::new(Waiter::default());
    studio.subscribe_training(Arc::clone(&progress_sink(&progress)) as Arc<dyn TrainingSink>);
    studio.subscribe_training(Arc::clone(&waiter) as Arc<dyn TrainingSink>);
    bench.runtime.block_on(async {
        studio
            .start_training(TrainMode::Auto, true)
            .expect("the run starts");
    });
    assert_eq!(waiter.wait(), None, "the run produced a head");
    let training = studio.training();
    assert!(!training.running);
    assert_eq!(training.stage, "terminé");
    assert!(
        progress.load(Ordering::SeqCst) > 0,
        "the run reported progress"
    );
    assert_eq!(training.examples, 12);
    let head = training.head.expect("a head");
    assert_eq!(head.examples, 12);
    assert_eq!(head.classes, 2);
    assert_eq!(head.dim, 2560);
    assert!(
        head.grouped,
        "two artists a genre: whole artists are held out"
    );
    assert!(head.balanced > 0.9, "balanced {}", head.balanced);
    assert!(head.fit_balanced > 0.9);
    assert_eq!(head.health.level, "good", "{:?}", head.health);
    assert_eq!(head.per_class.len(), 2);
    assert!(head.cleanup.is_empty(), "{:?}", head.cleanup);
    assert_eq!(
        head.seed, 1,
        "the driver pins the seed: a run repeats exactly"
    );

    // The server takes the head, and its relabel is the answer.
    let sent = bench
        .runtime
        .block_on(studio.send_head())
        .expect("the send");
    assert_eq!(sent.labels, 2);
    assert!(sent.relabel.expect("a relabel").running);
    let body = bench
        .server
        .last_body_of("PUT", "/api/genre/model")
        .expect("the put");
    assert!(body.contains("\"weights\""), "{body}");
    // No tree was read, so no family is claimed: a head with no family votes for
    // nothing rather than guessing one.
    assert!(!body.contains("families"), "{body}");

    // What the run cost: the labels, the vectors, the pool and its vectors.
    let routes = bench.server.routes();
    assert_eq!(routes[0], "GET /api/genre/labelled");
    assert_eq!(routes[1], "POST /api/genre/embeddings");
    assert_eq!(routes[2], "GET /api/genre/pool");
    assert_eq!(routes[3], "POST /api/genre/embeddings");
    assert_eq!(routes[4], "PUT /api/genre/model");
    assert_eq!(routes.len(), 5, "{routes:?}");
}

/// The head ships its labels' FAMILIES when the studio has read the tree: the
/// server then serves the family instead of falling back to its rules.
#[test]
fn a_sent_head_carries_the_trees_families() {
    let bench = Bench::start("train-families").with_vectors();
    bench
        .runtime
        .block_on(bench.studio.tree())
        .expect("the tree");
    let studio = Arc::clone(&bench.studio);
    let waiter = Arc::new(Waiter::default());
    studio.subscribe_training(Arc::clone(&waiter) as Arc<dyn TrainingSink>);
    bench.runtime.block_on(async {
        studio
            .start_training(TrainMode::Linear, false)
            .expect("the run starts");
    });
    assert_eq!(waiter.wait(), None);
    bench
        .runtime
        .block_on(studio.send_head())
        .expect("the send");
    let body = bench
        .server
        .last_body_of("PUT", "/api/genre/model")
        .expect("the put");
    assert!(body.contains("\"Frenchcore\":\"Hardcore\""), "{body}");
    assert!(
        !body.contains("\"Hardtek\":"),
        "Hardtek has no family: {body}"
    );
}

/// A second run while one is going is REFUSED, never silently doubled.
#[test]
fn a_second_run_is_refused_while_one_is_going() {
    let bench = Bench::start("train-twice").with_vectors();
    let studio = Arc::clone(&bench.studio);
    let waiter = Arc::new(Waiter::default());
    studio.subscribe_training(Arc::clone(&waiter) as Arc<dyn TrainingSink>);
    bench.runtime.block_on(async {
        studio
            .start_training(TrainMode::Linear, false)
            .expect("the first starts");
        let second = studio.start_training(TrainMode::Linear, false);
        assert!(second.is_err(), "a second run must be refused");
    });
    assert_eq!(waiter.wait(), None);
}

/// A training run with no labels says so in French, as a CAUSE.
#[test]
fn a_run_without_labels_names_its_cause() {
    // A server whose `/genre/labelled` is empty: the studio has nothing to learn
    // from, and the sentence is the run's own (not a bare "failed").
    let bench = Bench::start("train-empty");
    bench.server.no_labels();
    let studio = Arc::clone(&bench.studio);
    let waiter = Arc::new(Waiter::default());
    studio.subscribe_training(Arc::clone(&waiter) as Arc<dyn TrainingSink>);
    bench.runtime.block_on(async {
        studio
            .start_training(TrainMode::Linear, false)
            .expect("starts");
    });
    let cause = waiter.wait().expect("the run failed with a cause");
    assert!(
        cause.contains("étiqueté") || cause.contains("empreinte"),
        "{cause}"
    );
    assert!(studio.training().head.is_none());
}

/// A sink that parks the test on the run's own END, never on a duration.
#[derive(Default)]
struct Waiter {
    done: Mutex<Option<Option<String>>>,
    changed: Condvar,
}

impl TrainingSink for Waiter {
    fn progress(&self, _stage: &str, _fraction: f64) {}

    fn finished(&self, _ok: bool, cause: Option<&str>) {
        let mut done = self.done.lock().unwrap_or_else(|err| err.into_inner());
        *done = Some(cause.map(str::to_owned));
        self.changed.notify_all();
    }
}

impl Waiter {
    fn wait(&self) -> Option<String> {
        let deadline = Instant::now() + Duration::from_secs(180);
        let mut done = self.done.lock().unwrap_or_else(|err| err.into_inner());
        while done.is_none() {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "the run never ended");
            let (next, timeout) = self
                .changed
                .wait_timeout(done, left)
                .unwrap_or_else(|err| err.into_inner());
            done = next;
            assert!(
                !timeout.timed_out() || done.is_some(),
                "the run never ended"
            );
        }
        done.clone().unwrap_or_default()
    }
}

/// A sink that only counts the progress messages.
fn progress_sink(counter: &Arc<AtomicUsize>) -> Arc<Counting> {
    Arc::new(Counting(Arc::clone(counter)))
}

struct Counting(Arc<AtomicUsize>);

impl TrainingSink for Counting {
    fn progress(&self, _stage: &str, _fraction: f64) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }

    fn finished(&self, _ok: bool, _cause: Option<&str>) {}
}

// --- the measure pass ---------------------------------------------------------------

/// The measure pass on a server with nothing to measure: it ends at once, says
/// so, and its state is clean — the shape the card draws. Resuming is the
/// server's own cursor (`/analysis/todo?after=`), which `nz-analysis` owns.
#[test]
fn the_measure_pass_ends_on_an_empty_todo_list() {
    let bench = Bench::start("measure");
    let waiter = Arc::new(MeasureWaiter::default());
    bench
        .studio
        .subscribe_measure(Arc::clone(&waiter) as Arc<dyn MeasureSink>);
    bench.runtime.block_on(async {
        bench.studio.start_measure().expect("the measure starts");
    });
    waiter.wait();
    let state = bench.studio.measure();
    assert!(!state.running);
    assert_eq!(state.failed, 0, "{state:?}");
    assert!(state.causes.is_empty(), "{state:?}");
    assert!(
        bench
            .server
            .routes()
            .iter()
            .all(|route| route.starts_with("GET /api/analysis/todo")),
        "{:?}",
        bench.server.routes()
    );
}

#[derive(Default)]
struct MeasureWaiter {
    done: Mutex<bool>,
    changed: Condvar,
}

impl MeasureSink for MeasureWaiter {
    fn changed(&self, state: &MeasureState) {
        if state.running {
            return;
        }
        let mut done = self.done.lock().unwrap_or_else(|err| err.into_inner());
        *done = true;
        self.changed.notify_all();
    }
}

impl MeasureWaiter {
    fn wait(&self) {
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut done = self.done.lock().unwrap_or_else(|err| err.into_inner());
        while !*done {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "the measure pass never ended");
            let (next, _) = self
                .changed
                .wait_timeout(done, left)
                .unwrap_or_else(|err| err.into_inner());
            done = next;
        }
    }
}

// --- the acceptance: no underrun while training ---------------------------------------

/// A real engine, fed the way a device's callback feeds it, never underruns while
/// a training run works. The number is the mixer's own counter (`underruns`), not
/// a clock and not the RSS of a shared test process.
#[test]
fn a_training_run_never_starves_the_engine() {
    use std::sync::Arc as StdArc;

    use nz_audio::{
        ByteStream, Engine, EngineConfig, Quality, SourceError, StreamOpener, StreamReady,
    };

    struct Files(std::path::PathBuf);

    impl StreamOpener for Files {
        fn open(
            &self,
            _id: &str,
            _quality: Quality,
            _start: Option<f64>,
            _from_byte: Option<u64>,
            _lane: nz_audio::Lane,
        ) -> Result<Box<dyn ByteStream>, SourceError> {
            Err(SourceError::Message("bench: local files only".into()))
        }

        fn ready(&self, _id: &str, _quality: Quality) -> Result<StreamReady, SourceError> {
            Ok(StreamReady {
                ready: true,
                ranges: Some("bytes".to_owned()),
                ..StreamReady::default()
            })
        }

        fn local(&self, id: &str, _quality: Quality) -> Option<std::path::PathBuf> {
            (id == "tone").then(|| self.0.clone())
        }
    }

    let bench = Bench::start("underruns").with_vectors();
    let fixture = bench.dir.path().join("tone.flac");
    std::fs::write(
        &fixture,
        include_bytes!("../player/tests/fixtures/tone440.flac"),
    )
    .expect("the fixture");
    let (engine, mut sink) = Engine::new_external(
        EngineConfig {
            spool_dir: bench.dir.path().join("spool"),
            ..EngineConfig::default()
        },
        StdArc::new(Files(fixture)),
    );
    engine.load("tone", Quality::Flac, None);
    engine.set_volume(0.05);
    engine.play();

    // A matrix built ONCE, then trained again and again while the audio plays: the
    // work that could starve the mixer is the arithmetic, not the network.
    let matrix = bench
        .runtime
        .block_on(train::build_matrix(bench.studio.core(), false))
        .expect("the matrix");
    let stop = StdArc::new(AtomicBool::new(false));
    let runs = StdArc::new(AtomicUsize::new(0));
    let trainer_stop = StdArc::clone(&stop);
    let trainer_runs = StdArc::clone(&runs);
    let sleeper = bench.runtime.handle().clone();
    let worker = std::thread::Builder::new()
        .name("nz-genre-train".into())
        .stack_size(4 * 1024 * 1024)
        .spawn(move || {
            // The same niceness the job thread asks for (rule 1).
            let _ = nz_analysis::lower_priority(train::THREAD_NICE);
            while !trainer_stop.load(Ordering::SeqCst) {
                let again = train::Matrix {
                    prepared: matrix.prepared.clone(),
                    pool: Vec::new(),
                    pool_rows: 0,
                };
                let trainer = train::Trainer {
                    control: StdArc::new(nz_analysis::Control::default()),
                    sink: StdArc::new(Silent),
                };
                let _ = train::run_training(sleeper.clone(), again, TrainMode::Linear, &trainer);
                trainer_runs.fetch_add(1, Ordering::SeqCst);
            }
        })
        .expect("the trainer thread");

    // Feed the sink the way a device's callback does: 10 ms buffers, real time.
    let started = Instant::now();
    let mut peak = 0.0_f32;
    let mut rendered = 0usize;
    while started.elapsed() < Duration::from_secs(2) {
        let mut buffer = vec![0.0_f32; 480 * 2];
        sink.render(&mut buffer);
        for sample in &buffer {
            peak = peak.max(sample.abs());
        }
        rendered += 1;
        std::thread::sleep(Duration::from_millis(4));
    }
    let underruns = engine.underruns();
    stop.store(true, Ordering::SeqCst);
    engine.shutdown();
    let _ = worker.join();
    let done = runs.load(Ordering::SeqCst);
    println!(
        "acceptance: {rendered} buffers, peak {peak:.3}, {done} training runs, \
         underruns {underruns}"
    );
    assert!(peak > 0.001, "the tone played (peak {peak})");
    assert!(done > 0, "a training run happened while the audio played");
    assert_eq!(underruns, 0, "the training starved the mixer");
}

/// A sink nobody listens to.
struct Silent;

impl TrainingSink for Silent {
    fn progress(&self, _stage: &str, _fraction: f64) {}

    fn finished(&self, _ok: bool, _cause: Option<&str>) {}
}
