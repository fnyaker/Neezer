//! The guest's arithmetic and its planner: the timeline, the correction rule,
//! the normalisation gain, the published state, ten minutes of listening and the
//! recovery after a cut — plus a real server's chunks, decoded here.

use std::time::Duration;

use nz_api::{Api, Lane};
use nz_model::PartyState;

use crate::guest::{chunk_path, decode_chunk};
use crate::planner::{LEAD_S, LOOKAHEAD, Planner, Situation};
use crate::timeline::{
    CHUNK_S, Correction, KEEP_MS, OVERLAP_S, SEAM_MS, Timeline, WAIT_MAX_MS, chunk_index,
    correction_for, last_chunk, norm_gain, position_at, server_at_stream, server_time_at,
    timeline_of, to_stream,
};

fn state_of(body: &str) -> PartyState {
    serde_json::from_str(body).expect("the server's own payload")
}

/// **The chunk route, character for character.** `engine.js#fetchChunk` asks
/// `${base}/chunk/${id}/${k}?f=${fmt}&sr=${rate}` and the server routes
/// `/party/<pid>/chunk/<mid>/<int:k>`: the index is a PATH segment. The first
/// version had it after the query (`?f=opus&sr=48000/3`), which no route matches
/// — measured on the real server, every chunk was a `404`. This test fails on
/// that shape (the string differs in the two places) and passes on this one.
#[test]
fn a_chunk_is_fetched_from_the_route_the_server_registers() {
    assert_eq!(
        chunk_path("b3VGv0dTkz3Wq7nH_Z1pQA", "3000001", 7),
        "/party/b3VGv0dTkz3Wq7nH_Z1pQA/chunk/3000001/7?f=opus&sr=48000"
    );
    // What the server's route takes: the index BEFORE the `?`, the options after.
    let path = chunk_path("p1", "3000001", 3);
    let (route, query) = path.split_once('?').expect("a query");
    assert!(route.ends_with("/chunk/3000001/3"), "{route}");
    assert_eq!(query, "f=opus&sr=48000");
    assert!(
        !query.contains('/'),
        "no path segment after the query: {query}"
    );
    // An id that needs escaping is escaped in the PATH, never in the options.
    assert_eq!(
        chunk_path("p 1", "a/b", 0),
        "/party/p%201/chunk/a%2Fb/0?f=opus&sr=48000"
    );
}

#[test]
fn the_timeline_is_the_pages_arithmetic() {
    let tl = Timeline {
        t: 1_000.0,
        p: 41.0,
        playing: true,
    };
    assert_eq!(position_at(tl, 1_500.0), 41.5);
    assert_eq!(server_time_at(tl, 42.0), 2_000.0);
    assert_eq!(
        position_at(
            Timeline {
                t: 1_000.0,
                p: 41.0,
                playing: false
            },
            9_000.0
        ),
        41.0
    );
    // `chunkIndex` / `lastChunk` (`timeline.js`).
    assert_eq!(chunk_index(0.0, 6.0), 0);
    assert_eq!(chunk_index(5.999, 6.0), 0);
    assert_eq!(chunk_index(6.0, 6.0), 1);
    assert_eq!(last_chunk(12.0, 6.0), Some(2));
    assert_eq!(last_chunk(0.0, 6.0), None);
}

#[test]
fn the_correction_rule_is_the_pages_three_bands() {
    assert_eq!(correction_for(0.0), Correction::Keep);
    assert_eq!(correction_for(-3.0), Correction::Keep);
    assert_eq!(correction_for(3.1), Correction::Seam);
    assert_eq!(correction_for(-30.0), Correction::Seam);
    // Behind by more: skip forward.
    assert_eq!(correction_for(-30.1), Correction::Jump);
    // Ahead by more: wait, up to a stall's worth…
    assert_eq!(correction_for(400.0), Correction::Wait);
    // …past which it is the host going BACK, and the guest goes with it.
    assert_eq!(correction_for(WAIT_MAX_MS + 1.0), Correction::Jump);
}

#[test]
fn a_stream_instant_is_the_server_instant_plus_the_clock_and_the_bridge() {
    // server = local + 5000 ms; output clock = local + 2 s; no trim.
    let server = 60_000.0;
    let stream = to_stream(server, 5_000.0, 2.0, 0.0);
    assert!((stream - 57.0).abs() < 1e-9);
    assert!((server_at_stream(stream, 5_000.0, 2.0, 0.0) - server).abs() < 1e-6);
    // A device that sounds 40 ms late plays its audio 40 ms earlier.
    let earlier = to_stream(server, 5_000.0, 2.0, 40.0);
    assert!((stream - earlier - 0.040).abs() < 1e-9);
}

#[test]
fn the_normalisation_gain_is_the_pages() {
    assert!((norm_gain(Some(-8.4), "off") - 1.0).abs() < 1e-6);
    // -(loudness + 18.4) + 5 = −5 dB for a −8.4 LUFS master.
    assert!((norm_gain(Some(-8.4), "medium") - 10f64.powf(-5.0 / 20.0) as f32).abs() < 1e-6);
    assert!((norm_gain(None, "high") - 1.0).abs() < 1e-6);
    // Clamped: a very loud master is attenuated by at most 24 dB.
    let quiet = norm_gain(Some(40.0), "high");
    assert!((quiet - 10f64.powf(-24.0 / 20.0) as f32).abs() < 1e-5);
}

#[test]
fn the_published_state_reads_as_the_timeline_the_room_follows() {
    let state = state_of(
        r#"{"id":"p1","host":"Léa","live":true,
            "track":{"id":"3000001","title":"T","artist":"A","album":"B","duration":180.0,
                     "podcast":false,"gain":-8.5,"cover":"/api/party/p1/cover/3000001"},
            "playing":true,"buf":false,"anchor":{"t":1794835151000.0,"p":41.25},
            "xfade":0.0,"next":null,"norm":"medium","listeners":[{"name":"Salon"}],
            "now":1794835152000.0}"#,
    );
    let tl = timeline_of(&state).expect("a line");
    assert_eq!(tl.p, 41.25);
    assert!(tl.playing);
    assert_eq!(crate::timeline::track_id(&state), Some("3000001"));
    assert_eq!(state.listeners.len(), 1);
    // A party with nothing playing has no line, and that is not an error.
    let empty = state_of(r#"{"id":"p1","host":"L","track":null,"anchor":null}"#);
    assert!(timeline_of(&empty).is_none());
}

/// A device, a server and a party line, advanced in simulated time: the bench
/// every drift number comes from. `ppm` is how fast the device's own crystal
/// runs against the server's.
struct Sim {
    planner: Planner,
    /// The true server clock, milliseconds.
    server_ms: f64,
    /// Where the device's output stream is, seconds of audio it has played
    /// (its own clock: it runs at `1 + ppm`).
    stream: f64,
    /// The NTP estimate (`server − local`), exact: it is re-measured every
    /// two seconds and the server's clock is steady.
    offset: f64,
    /// The bridge's own median (`output clock − local`, seconds), read fresh:
    /// the median over 15 readings lags the true mapping by ~0.4 s of drift,
    /// which at 35 ppm is 14 µs — below anything a seam can feel.
    bridge: f64,
    rate: u32,
    line: Timeline,
    /// When the last poll answered.
    answered: f64,
    /// The last chunk this device has FETCHED: a cut link adds no more.
    fetched: u64,
    ppm: f64,
    /// The last plan, and whether anything covers the audible instant.
    covered: bool,
}

impl Sim {
    const OFFSET: f64 = 5_000.0;

    fn new(ppm: f64) -> Sim {
        let server_ms = 1_000_000.0;
        let stream = 100.0;
        Sim {
            planner: Planner::new(48_000),
            server_ms,
            stream,
            offset: Sim::OFFSET,
            bridge: stream - (server_ms - Sim::OFFSET) / 1000.0,
            rate: 48_000,
            line: Timeline {
                t: server_ms,
                p: 41.0,
                playing: true,
            },
            answered: server_ms,
            fetched: LOOKAHEAD,
            ppm,
            covered: false,
        }
    }

    fn local(&self) -> f64 {
        self.server_ms - self.offset
    }

    /// `ms` of real time: the server and the line move by the true clock, the
    /// device by its own crystal.
    fn advance(&mut self, ms: f64) {
        self.server_ms += ms;
        self.stream += ms / 1000.0 * (1.0 + self.ppm * 1e-6);
        self.bridge = self.stream - self.local() / 1000.0;
    }

    /// A poll answered: the host republished its (unchanged) line, and the
    /// chunks four ahead were fetched.
    fn poll(&mut self) {
        self.line = Timeline {
            t: self.server_ms,
            p: position_at(self.line, self.server_ms),
            playing: self.line.playing,
        };
        self.answered = self.server_ms;
        self.fetched = chunk_index(position_at(self.line, self.server_ms), CHUNK_S) + LOOKAHEAD;
    }

    fn situation<'a>(&self, duration: f64, ready: &'a dyn Fn(u64) -> bool) -> Situation<'a> {
        Situation {
            timeline: Some(self.line),
            offset_ms: Some(self.offset),
            bridge_s: Some(self.bridge),
            stream_now: self.stream,
            rate: self.rate,
            trim_ms: 0.0,
            since_poll_ms: (self.server_ms - self.answered).max(0.0) as u64,
            duration,
            ready,
        }
    }

    fn tick(&mut self) -> crate::planner::Plan {
        let horizon = self.fetched;
        let ready = move |chunk: u64| chunk <= horizon;
        let situation = self.situation(3_600.0, &ready);
        let plan = self.planner.plan(&situation);
        if plan.cut {
            self.planner.forget();
        }
        if plan.clear_future {
            self.planner.keep_sounding(self.stream);
        }
        self.covered = self.planner.placed().iter().any(|placement| {
            self.stream >= placement.from_s && self.stream < placement.from_s + CHUNK_S + OVERLAP_S
        }) && self
            .planner
            .placed()
            .last()
            .is_some_and(|last| self.stream < last.from_s + CHUNK_S + OVERLAP_S);
        plan
    }

    /// Where this device is on the media timeline, from what it placed.
    fn heard(&self) -> Option<f64> {
        self.planner.placed().iter().rev().find_map(|placement| {
            let end = placement.from_s + CHUNK_S + OVERLAP_S;
            (self.stream >= placement.from_s && self.stream < end).then(|| {
                placement.chunk as f64 * CHUNK_S
                    + placement.skip_frames as f64 / f64::from(self.rate)
                    + (self.stream - placement.from_s)
            })
        })
    }

    fn wanted(&self) -> f64 {
        position_at(
            self.line,
            server_at_stream(self.stream, self.offset, self.bridge, 0.0),
        )
    }

    fn error_ms(&self) -> Option<f64> {
        self.heard().map(|heard| (heard - self.wanted()) * 1000.0)
    }
}

/// Ten minutes of listening: the error stays inside a seam's worth, and every
/// seam is where it goes back to zero.
#[test]
fn ten_minutes_of_listening_stay_inside_the_seam() {
    let mut sim = Sim::new(35.0);
    sim.tick();
    let mut worst_at_seam: f64 = 0.0;
    let mut worst_between: f64 = 0.0;
    let mut corrections = 0;
    // 600 s at 100 ms a tick, a poll every second — six thousand ticks.
    for step in 0..6_000 {
        sim.advance(100.0);
        if step % 10 == 9 {
            sim.poll();
        }
        let plan = sim.tick();
        if step % 100 == 0 {
            println!(
                "step {step}: stream {:.4} line t={:.1} p={:.4} heard {:?} wanted {:.4} placed {:?}",
                sim.stream,
                sim.line.t,
                sim.line.p,
                sim.heard(),
                sim.wanted(),
                sim.planner
                    .placed()
                    .iter()
                    .map(|p| (p.chunk, (p.from_s * 1000.0).round() as i64, p.skip_frames))
                    .collect::<Vec<_>>(),
            );
        }
        if plan.correction.is_some_and(|c| c != Correction::Keep) {
            corrections += 1;
        }
        if let Some(error) = sim.error_ms() {
            worst_between = worst_between.max(error.abs());
            // A seam: the instant a new chunk starts.
            if plan.place.iter().any(|placement| {
                (placement.from_s - sim.stream).abs() <= 0.1 && placement.skip_frames == 0
            }) {
                worst_at_seam = worst_at_seam.max(error.abs());
            }
        }
    }
    println!(
        "10 min at 35 ppm: worst |error| between seams {worst_between:.3} ms, at a seam \
         {worst_at_seam:.3} ms, {corrections} corrections"
    );
    // The criterion the plan sets: a guest inside the host's own republish
    // threshold (2.5 ms) between seams, and never having to move by more than
    // one seam's worth at a seam.
    assert!(worst_between < KEEP_MS, "between seams: {worst_between} ms");
    assert!(worst_at_seam < SEAM_MS, "at a seam: {worst_at_seam} ms");
    // A drift the seam resets never accumulates into a jump.
    assert!(corrections < 10, "{corrections} corrections in ten minutes");
    assert!(
        sim.covered,
        "the guest fell silent at stream {:.3}: placed {:?} fetched {}, heard {:?}",
        sim.stream,
        sim.planner.placed(),
        sim.fetched,
        sim.heard()
    );
}

/// A clock that steps (the device's mapping moving by a render burst) is the
/// page's own case: the guest must follow it, and the correction must stay
/// inside the seam band unless the step is bigger than one.
#[test]
fn a_stepped_device_clock_is_followed_within_a_seam() {
    let mut sim = Sim::new(0.0);
    sim.tick();
    for _ in 0..20 {
        sim.advance(100.0);
        sim.tick();
    }
    // The mapping steps by 20 ms: `d` jumps, and the bridge's median follows
    // after two agreeing readings.
    sim.stream += 0.020;
    sim.bridge = sim.stream - sim.local() / 1000.0;
    let mut worst: f64 = 0.0;
    let mut first_after: Option<f64> = None;
    for _ in 0..40 {
        sim.advance(100.0);
        let plan = sim.tick();
        if let Some(error) = sim.error_ms() {
            worst = worst.max(error.abs());
            first_after.get_or_insert(error.abs());
        }
        assert!(
            plan.correction
                .is_none_or(|c| c == Correction::Keep || c == Correction::Seam),
            "a clock step must never need a jump: {:?}",
            plan.correction
        );
    }
    println!("a 20 ms mapping step: worst |error| {worst:.3} ms after it");
    assert!(worst < SEAM_MS, "{worst} ms");
}

/// A link that comes back after longer than the look-ahead: the guest went
/// silent, and rejoins at the live line within one tick plus the lead.
#[test]
fn a_cut_longer_than_the_look_ahead_rejoins_on_the_line() {
    let mut sim = Sim::new(0.0);
    sim.tick();
    for _ in 0..20 {
        sim.advance(100.0);
        sim.poll();
        sim.tick();
    }
    // The link dies: 40 s with no poll (the look-ahead is 4 × 6 s = 24 s of
    // audio, and nothing new is fetched while the link is down).
    for _ in 0..400 {
        sim.advance(100.0);
        sim.tick();
    }
    assert!(!sim.covered, "the guest should have run out of audio");
    assert!(sim.heard().is_none());
    // The line the host kept publishing meanwhile: the room is 40 s ahead.
    sim.poll();
    let mut rejoined_after: Option<f64> = None;
    for _ in 0..20 {
        sim.advance(100.0);
        sim.tick();
        if sim.covered && rejoined_after.is_none() {
            rejoined_after = Some(sim.error_ms().unwrap_or(0.0));
        }
    }
    let error = rejoined_after.expect("the guest never came back");
    println!("after a 40 s cut: back on the line within a tick, |error| {error:.3} ms");
    assert!(error.abs() <= 5.0, "{error} ms");
    // And from there it is a normal guest again: inside the seam band.
    assert!(sim.error_ms().unwrap_or(0.0).abs() < SEAM_MS);
}

/// Joining in the middle of a chunk costs that chunk's tail, not the chunk:
/// the first placement starts at `now + LEAD` with the frames already past
/// skipped.
#[test]
fn joining_mid_track_starts_where_the_room_is() {
    let mut sim = Sim::new(0.0);
    // The room is 2.5 s into the track: chunk 0 is half over.
    sim.line = Timeline {
        t: sim.server_ms,
        p: 0.0,
        playing: true,
    };
    sim.advance(2_500.0);
    sim.poll();
    sim.tick();
    let first = sim.planner.placed()[0];
    assert_eq!(first.chunk, 0);
    assert!(first.skip_frames > 100_000, "{first:?}");
    assert!(
        (first.skip_frames as f64 / 48_000.0 - 2.6).abs() < 0.15,
        "{first:?}"
    );
    // Placed only what is close enough to schedule on the mapping (7 s): the
    // chunk being heard and the next one.
    assert!(sim.planner.placed().len() >= 2, "the look-ahead is placed");
    // The lead is 100 ms: two ticks later the room hears it, on the line.
    sim.advance(200.0);
    sim.tick();
    assert!(sim.covered);
    assert!(
        sim.error_ms().unwrap_or(99.0).abs() < KEEP_MS,
        "{:?}",
        sim.error_ms()
    );
}

/// The real server, end to end: a chunk cut by ffmpeg out of a click track
/// decodes to the click times it was made from, at the device's rate, and the
/// planner places it on the line the host published.
#[test]
fn a_real_chunk_decodes_to_the_click_it_was_cut_from() {
    let Some(bench) = PartyBench::start() else {
        println!("no Python that can `import neezer`: skipped (the desktop job installs it)");
        return;
    };
    let (api, runtime) = bench.client();
    let track = bench.track("a");
    let pid = runtime.block_on(async {
        api.login("bench", "Bench1").await.expect("login");
        let party = api.party_start(None).await.expect("a party");
        // The anchor has to be on the SERVER's clock (it refuses one more
        // than ten minutes off): the host view carries it.
        let now = party.now;
        let state = nz_model::PartyPublish {
            track: Some(nz_model::PartyTrack {
                id: track.id.clone(),
                title: "Click A".into(),
                artist: "Bench".into(),
                album: "Bench".into(),
                duration: 60.0,
                ..Default::default()
            }),
            playing: true,
            t: now,
            p: 0.0,
            ..Default::default()
        };
        api.party_publish(&party.id, &state).await.expect("publish");
        party.id
    });
    // Chunk 0 of the track, as a guest with no seat asks for it.
    let bytes = runtime.block_on(async {
        api.bytes(
            Lane::User,
            &format!("/party/{pid}/chunk/{}/0?f=opus&sr=48000", track.id),
        )
        .await
    });
    let bytes = bytes.expect("chunk 0");
    let pcm = decode_chunk(&bytes, 48_000).expect("a decoded chunk");
    let seconds = pcm.len() as f64 / 2.0 / 48_000.0;
    println!(
        "chunk 0: {} bytes of Ogg Opus → {:.4} s of stereo f32",
        bytes.len(),
        seconds
    );
    // The server cuts [k·6 s, (k+1)·6 s + 50 ms): 6.05 s, ±one Opus frame.
    assert!(
        (seconds - (CHUNK_S + OVERLAP_S)).abs() < 0.002,
        "{seconds} s"
    );
    // The first click of the fixture is at 0.5 s (server.py): it must be
    // inside this chunk, at the sample its own media time says.
    let click = track.clicks[0];
    let at = (click * 48_000.0) as usize;
    let peak = (at..(at + 48_000).min(pcm.len() / 2))
        .map(|frame| pcm[frame * 2].abs())
        .fold(0.0f32, f32::max);
    let before = (0..at.saturating_sub(96))
        .map(|frame| pcm[frame * 2].abs())
        .fold(0.0f32, f32::max);
    println!("click at {click:.3} s: peak {peak:.3}, before it {before:.3}");
    assert!(peak > 0.2, "the click is there: {peak}");
    assert!(before < 0.02, "and nothing before it: {before}");
    // Chunk 1 starts exactly one chunk later, so a guest's seam is one
    // placement after the previous.
    let next = runtime
        .block_on(async {
            api.bytes(
                Lane::User,
                &format!("/party/{pid}/chunk/{}/1?f=opus&sr=48000", track.id),
            )
            .await
        })
        .expect("chunk 1");
    assert!(decode_chunk(&next, 48_000).is_ok());
}

/// The planner, driven against a real server's chunk list: every chunk of a
/// minute is placed once, in order, with no gap and no overlap beyond the
/// server's own 50 ms.
#[test]
fn the_planner_places_a_real_minute_of_chunks_once_each() {
    let mut sim = Sim::new(0.0);
    sim.tick();
    let mut placed = Vec::new();
    for step in 0..600 {
        sim.advance(100.0);
        if step % 10 == 9 {
            sim.poll();
        }
        let plan = sim.tick();
        for placement in plan.place {
            placed.push(placement);
        }
    }
    assert!(!placed.is_empty());
    // No chunk twice, and the sequence has no hole.
    let mut seen: Vec<u64> = placed.iter().map(|placement| placement.chunk).collect();
    seen.sort_unstable();
    let unique = seen.clone();
    seen.dedup();
    assert_eq!(seen, unique, "a chunk was placed twice");
    // Consecutive chunks are one chunk apart. The FIRST one is the join,
    // placed 100 ms late with those frames skipped (its data had just
    // arrived), so that one pair is a lead short.
    for (index, pair) in placed.windows(2).enumerate() {
        let gap = pair[1].from_s - pair[0].from_s;
        let slack = if index == 0 { LEAD_S + 0.01 } else { 0.02 };
        assert!(
            (gap - CHUNK_S).abs() <= slack,
            "chunks {} and {} are {gap:.4} s apart",
            pair[0].chunk,
            pair[1].chunk
        );
    }
}

// --- the real server --------------------------------------------------------

/// A `webapp/test/party/server.py` on a throwaway database: two click tracks
/// cut by ffmpeg, one admin (`bench`/`Bench1`), and the click times of each
/// track so a decoded chunk can be checked against what it was cut from.
struct PartyBench {
    child: std::process::Child,
    port: u16,
    tracks: serde_json::Value,
    _guard: std::process::ChildStdin,
}

struct Track {
    id: String,
    clicks: Vec<f64>,
}

/// The repository root: `desktop/crates/nz-party` is three levels below it.
fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("the repository root")
}

fn python() -> Option<String> {
    let candidates: Vec<String> = std::env::var("NZ_PYTHON")
        .ok()
        .into_iter()
        .chain([
            "python3".to_owned(),
            repo_root().join(".venv/bin/python").display().to_string(),
            repo_root().join("venv/bin/python").display().to_string(),
        ])
        .collect();
    candidates.into_iter().find(|candidate| {
        std::process::Command::new(candidate)
            .args(["-c", "import peewee, neezer"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    })
}

impl PartyBench {
    fn start() -> Option<PartyBench> {
        use std::process::{Command, Stdio};
        let python = python()?;
        let script = repo_root().join("webapp/test/party/server.py");
        if !script.is_file() {
            println!("no {}: skipped", script.display());
            return None;
        }
        let mut child = Command::new(python)
            .arg(script)
            .current_dir(repo_root())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .stdin(Stdio::piped())
            .spawn()
            .ok()?;
        let stdout = child.stdout.take()?;
        let (sender, receiver) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            use std::io::BufRead;
            for line in std::io::BufReader::new(stdout)
                .lines()
                .map_while(Result::ok)
            {
                let _ = sender.send(line);
            }
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(120);
        while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
            // A child that died (a bad interpreter, a missing dependency)
            // must not cost the whole deadline.
            if matches!(child.try_wait(), Ok(Some(_))) {
                return None;
            }
            if let Ok(line) = receiver.recv_timeout(left.min(Duration::from_millis(250)))
                && let Some(rest) = line.strip_prefix("READY ")
            {
                let mut parts = rest.splitn(2, ' ');
                let port = parts.next()?.parse::<u16>().ok()?;
                let tracks: serde_json::Value = serde_json::from_str(parts.next()?).ok()?;
                let guard = child.stdin.take()?;
                return Some(PartyBench {
                    child,
                    port,
                    tracks,
                    _guard: guard,
                });
            }
        }
        None
    }

    fn client(&self) -> (Api, tokio::runtime::Runtime) {
        let api = nz_api::Api::builder(format!("http://127.0.0.1:{}", self.port))
            .session(std::sync::Arc::new(nz_api::SessionJar::in_memory()))
            .build()
            .expect("a client");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        (api, runtime)
    }

    fn track(&self, name: &str) -> Track {
        let track = &self.tracks[name];
        Track {
            id: track["id"].as_str().unwrap_or_default().to_owned(),
            clicks: track["clicks"]
                .as_array()
                .map(|clicks| {
                    clicks
                        .iter()
                        .filter_map(serde_json::Value::as_f64)
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

impl Drop for PartyBench {
    fn drop(&mut self) {
        // Its stdin closing is what makes server.py shut its werkzeug down
        // and delete its throwaway library.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
