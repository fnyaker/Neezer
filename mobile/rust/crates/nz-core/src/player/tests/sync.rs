//! What the screen names is what the speaker plays.
//!
//! The owner's report (v1.6.9, a real phone): after Previous or Next the
//! title and the cover-derived background show another track than the one
//! heard. The player has two states that must agree — the MACHINE (queue
//! index, what a snapshot names) and the ENGINE (the voice the mixer plays) —
//! and these tests drive the commands a person, a notification button or a car
//! sends, then compare the two once the engine has settled.

use super::*;
use crate::player::Command;

const IDS: [&str; 5] = ["a", "b", "c", "d", "e"];

/// Five real tracks served by the bench, a player and its pumped sink.
struct Rig {
    _dir: TempDir,
    bench: Bench,
    player: Arc<Player>,
    sink: nz_audio::ExternalSink,
}

fn rig(fixture: Fixture) -> Rig {
    let dir = TempDir::new("sync");
    let bench = Bench::start();
    let path = fixture.write(dir.path());
    for id in IDS {
        bench.track(id, &path);
    }
    let player = player_for(&dir, bench.base());
    let (_, sink) = player.take_sink().expect("the sink");
    Rig {
        _dir: dir,
        bench,
        player,
        sink,
    }
}

impl Rig {
    fn queue(&mut self, start: u32) {
        let tracks = IDS.iter().map(|id| track_ref_of(id, 3)).collect::<Vec<_>>();
        self.player.command(
            Command::PlayQueue {
                tracks,
                start,
                context: None,
            },
            1,
        );
        let _ = pump_until_audio(&mut self.sink, &self.player, 30.0, 0.05);
    }

    /// Waits until the engine's preload voice is `id`.
    fn preloaded(&mut self, id: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.player.engine.preloaded_id().as_deref() != Some(id) {
            assert!(Instant::now() < deadline, "{id} never preloaded");
            let _ = pump(&mut self.sink, 0.01);
        }
    }

    /// The machine's current key, as a snapshot (what the screen draws).
    fn shown(&self) -> Option<String> {
        self.player.snapshot().current.map(|now| now.key)
    }

    /// Pumps `seconds` of audio, so every in-flight load has landed or died.
    fn settle(&mut self, seconds: f64) {
        let _ = pump(&mut self.sink, seconds);
    }

    /// The row the speaker plays.
    fn heard(&self) -> Option<String> {
        self.player.engine.current_id()
    }

    fn agree(&self, what: &str) {
        assert_eq!(
            self.shown(),
            self.heard(),
            "{what}: the screen names {:?} but the speaker plays {:?}",
            self.shown(),
            self.heard()
        );
    }
}

/// A seek still opening, then Next onto the row the engine already decoded:
/// the promotion must supersede the seek in flight. Measured on the old code:
/// the late seek lands over the promoted `b` — the screen says `b`, the speaker
/// plays `a` again.
#[test]
fn a_skip_onto_the_preloaded_voice_supersedes_the_seek_still_opening() {
    let mut rig = rig(Fixture::Silence8);
    rig.queue(0);
    rig.preloaded("b");
    let gate = rig.bench.hold("a");
    let before = rig.bench.requests();
    rig.player.command(Command::Seek { s: 4.0 }, 1);
    wait_requests(&rig.bench, before + 1, 5.0);
    assert!(
        rig.bench.requests() > before,
        "the seek never asked the server"
    );
    rig.player.command(Command::Next, 1);
    rig.settle(0.3);
    gate.open();
    rig.settle(1.0);
    assert_eq!(rig.shown().as_deref(), Some("b"));
    rig.agree("next over a seek still opening");
    rig.player.shutdown();
}

/// The same commands in many rhythms: whatever a thumb, a headset or a car does,
/// once the engine has settled the speaker plays the row the screen names.
#[test]
fn rapid_next_and_previous_end_on_the_row_the_screen_names() {
    let sequences: [&[(Command, f64)]; 6] = [
        &[
            (Command::Next, 0.0),
            (Command::Next, 0.0),
            (Command::Next, 0.0),
        ],
        &[(Command::Next, 0.0), (Command::Prev, 0.0)],
        &[
            (Command::Next, 0.05),
            (Command::Prev, 0.05),
            (Command::Next, 0.05),
            (Command::Next, 0.0),
        ],
        &[
            (Command::Next, 0.2),
            (Command::Next, 0.2),
            (Command::Prev, 0.0),
            (Command::Prev, 0.0),
        ],
        &[
            (Command::Next, 0.0),
            (Command::Prev, 0.0),
            (Command::Next, 0.0),
            (Command::Prev, 0.0),
            (Command::Next, 0.0),
        ],
        &[
            (Command::Next, 0.4),
            (Command::Prev, 0.1),
            (Command::Next, 0.01),
            (Command::Prev, 0.3),
            (Command::Next, 0.0),
        ],
    ];
    for (n, sequence) in sequences.into_iter().enumerate() {
        let mut rig = rig(Fixture::Silence8);
        rig.queue(1);
        rig.preloaded("c");
        let mut trace = Vec::new();
        for (command, gap) in sequence {
            rig.player.command(command.clone(), 1);
            trace.push(format!("{:?}", command));
            if *gap > 0.0 {
                rig.settle(*gap);
            }
        }
        rig.settle(1.5);
        rig.agree(&format!("sequence {n} {trace:?}"));
        rig.player.shutdown();
    }
}

/// Records what a listener was told, in the order it was told.
#[derive(Default)]
struct Told {
    versions: Mutex<Vec<(u64, Option<u32>)>>,
}

impl crate::player::Sink for Told {
    fn on_snapshot(&self, snapshot: crate::player::Snapshot) {
        // A Kotlin callback is not free (a UniFFI crossing and a state write):
        // a little work between "taken" and "delivered" is the real gap.
        thread::sleep(Duration::from_micros(200));
        self.versions
            .lock()
            .unwrap()
            .push((snapshot.version, snapshot.index));
    }

    fn on_queue_changed(&self, _version: u64) {}

    fn on_fault(&self, _cause: String) {}
}

/// Two threads push snapshots (the engine's worker after a skip, and the
/// platform's focus/route/sleep callbacks, which emit on THEIR thread). Each
/// takes the snapshot, then delivers it: without one order a stale snapshot
/// delivered late is the screen's last word, and a snapshot is only pushed on a
/// CHANGE, so it stays wrong until the next change.
#[test]
fn snapshots_reach_a_listener_in_order_and_the_last_is_the_newest() {
    let mut rig = rig(Fixture::Silence8);
    rig.queue(2);
    let told = Arc::new(Told::default());
    rig.player.subscribe(told.clone());
    let player = Arc::clone(&rig.player);
    let stop = Arc::new(AtomicBool::new(false));
    let flipper = {
        let stop = Arc::clone(&stop);
        thread::spawn(move || {
            let mut on = false;
            while !stop.load(Ordering::SeqCst) {
                on = !on;
                player.set_ducked(on);
            }
        })
    };
    for round in 0..60 {
        rig.player.command(
            if round % 2 == 0 {
                Command::Next
            } else {
                Command::Prev
            },
            1,
        );
        let _ = pump(&mut rig.sink, 0.02);
    }
    stop.store(true, Ordering::SeqCst);
    flipper.join().unwrap();
    rig.settle(1.0);
    let told = told.versions.lock().unwrap().clone();
    let newest = rig.player.snapshot();
    let mut last = 0;
    for (version, _) in &told {
        assert!(
            *version >= last,
            "a snapshot of version {version} was delivered after version {last}"
        );
        last = *version;
    }
    assert_eq!(
        told.last().map(|(version, index)| (*version, *index)),
        Some((newest.version, newest.index)),
        "the last snapshot a listener got is not the newest"
    );
    rig.player.shutdown();
}

/// A track that ends by itself (a gapless handover, nobody tapping): the screen
/// moves to the next row when the speaker does, row after row.
#[test]
fn a_natural_handover_moves_the_screen_with_the_speaker() {
    let mut rig = rig(Fixture::Tone440);
    rig.queue(0);
    for expected in ["b", "c"] {
        let deadline = Instant::now() + Duration::from_secs(15);
        while rig.shown().as_deref() != Some(expected) {
            assert!(Instant::now() < deadline, "{expected} never became current");
            let _ = pump(&mut rig.sink, 0.05);
        }
        rig.agree(&format!("handover to {expected}"));
    }
    rig.player.shutdown();
}

/// Next onto a row whose stream is still opening (a slow link): the screen
/// names it at once, so the OLD row must stop being heard under the new title.
/// Measured on the old code: `a` kept playing for as long as `b` took to open,
/// under `b`'s title and cover. Jump already silenced the old voice; the skip
/// buttons did not.
#[test]
fn the_old_row_is_not_heard_under_the_new_title_while_the_next_one_opens() {
    let mut rig = rig(Fixture::Silence8);
    let gate = rig.bench.hold("b");
    rig.queue(0);
    rig.player.command(Command::Next, 1);
    let _ = wait_state(&rig.player, 5.0, |snapshot| {
        snapshot.current.as_ref().is_some_and(|now| now.key == "b")
    });
    rig.settle(0.3);
    assert_ne!(
        rig.heard().as_deref(),
        Some("a"),
        "the screen names b while a is still heard"
    );
    gate.open();
    let deadline = Instant::now() + Duration::from_secs(10);
    while rig.heard().as_deref() != Some("b") {
        assert!(Instant::now() < deadline, "b never played");
        let _ = pump(&mut rig.sink, 0.05);
    }
    rig.agree("the held row finally opened");
    rig.player.shutdown();
}
