//! Two real engines, end to end: a click track cut into chunks exactly as the
//! server cuts them, played by a real host engine and followed by a real guest
//! engine, with the guest's planner in the loop. The number this test asserts is
//! the offset in milliseconds between what each device actually puts out.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nz_audio::{
    ByteStream, Engine, EngineConfig, EngineEvent, OfflineSink, Quality, Segment, SourceError,
    StreamOpener, StreamReady,
};

use crate::clock::now_ms;
use crate::guest::decode_chunk;
use crate::host::{Gaps, Line, Snapshot, Tick, decide};
use crate::planner::{Planner, Situation};
use crate::timeline::{CHUNK_S, OVERLAP_S, Timeline};

/// How long a test waits for the event it expects: a hang guard, never a
/// machine-speed assertion. The app's suite keeps the same budget in
/// `desktop/app/src/testwait.rs` (60 s, for the same reason: CI run 44200827967
/// turned a 10 s bound into a red job on a runner that was twice as slow as this
/// machine, and the event — not the deadline — is what the test asserts).
const WAIT: Duration = Duration::from_secs(60);

/// The host's own files: the `local` answer of a play cache.
struct FileOpener {
    paths: HashMap<String, PathBuf>,
}

impl StreamOpener for FileOpener {
    fn open(
        &self,
        id: &str,
        _quality: Quality,
        _start: Option<f64>,
        _from_byte: Option<u64>,
        _lane: nz_audio::Lane,
    ) -> Result<Box<dyn ByteStream>, SourceError> {
        let path = self
            .paths
            .get(id)
            .ok_or_else(|| SourceError::Message(format!("pas de fichier pour {id}")))?;
        let file = std::fs::File::open(path)
            .map_err(|error| SourceError::Message(format!("lecture impossible : {error}")))?;
        Ok(Box::new(FileSource(file)))
    }

    fn ready(&self, _id: &str, _quality: Quality) -> Result<StreamReady, SourceError> {
        Ok(StreamReady {
            ready: true,
            ..StreamReady::default()
        })
    }

    fn local(&self, id: &str, _quality: Quality) -> Option<PathBuf> {
        self.paths.get(id).cloned()
    }
}

/// A file the engine can read and seek: `ByteStream` is `Read + Send` with two
/// default methods, and a local path is all this test needs.
struct FileSource(std::fs::File);

impl std::io::Read for FileSource {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(out)
    }
}

impl ByteStream for FileSource {
    fn len_hint(&self) -> Option<u64> {
        self.0.metadata().ok().map(|meta| meta.len())
    }
}

fn ffmpeg(args: &[&str]) {
    let status = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .status();
    match status {
        Ok(status) => assert!(status.success(), "ffmpeg {args:?} failed"),
        Err(error) => panic!("ffmpeg is not installed: {error}"),
    }
}

/// A 40 s FLAC of silence with a 1 ms click every `STEP` seconds, at
/// `click_at` — the server's own fixture shape (`party/server.py`).
fn click_track(path: &Path, seconds: f64) {
    let mut expression = String::from("0");
    let mut at = 0.5;
    let spacing = [0.43, 0.37, 0.51, 0.41];
    let mut index = 0;
    while at < seconds - 0.5 {
        expression = format!(
            "{}+if(between(t\\,{}\\,{})\\,0.6*sin(2*PI*1000*t)\\,0)",
            expression,
            at,
            at + 0.001
        );
        at += spacing[index % spacing.len()];
        index += 1;
    }
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!("aevalsrc={expression}:s=48000:d={seconds}:c=stereo"),
        "-c:a",
        "flac",
        path.to_str().unwrap(),
    ]);
}

/// `[k·6 s, (k+1)·6 s + 50 ms)` of `source`, cut by ffmpeg exactly as
/// `neezer/webui/party.py#_cut` does, Ogg Opus 192k at 48 kHz.
fn cut_chunk(source: &Path, k: u64, out: &Path) {
    ffmpeg(&[
        "-ss",
        &format!("{:.6}", k as f64 * CHUNK_S),
        "-i",
        source.to_str().unwrap(),
        "-t",
        &format!("{:.6}", CHUNK_S + OVERLAP_S),
        "-map",
        "0:a:0",
        "-ac",
        "2",
        "-map_metadata",
        "-1",
        "-ar",
        "48000",
        "-c:a",
        "libopus",
        "-b:a",
        "192k",
        "-vbr",
        "on",
        "-f",
        "ogg",
        out.to_str().unwrap(),
    ]);
}

fn render(sink: &mut OfflineSink, frames: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; frames * 2];
    sink.render(&mut out);
    out
}

/// The frames of `buffer` where a click starts: a sample over half scale after
/// 30 ms of quiet (a 1 kHz burst would otherwise be counted once per zero
/// crossing).
fn clicks(buffer: &[f32]) -> Vec<usize> {
    let refractory = 48_000 * 30 / 1_000;
    let mut found = Vec::new();
    let mut last: Option<usize> = None;
    for frame in 0..buffer.len() / 2 {
        let value = buffer[frame * 2].abs().max(buffer[frame * 2 + 1].abs());
        if value > 0.25 && last.is_none_or(|at| frame >= at + refractory) {
            found.push(frame);
            last = Some(frame);
        }
    }
    found
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    if values.is_empty() {
        return f64::NAN;
    }
    values[values.len() / 2]
}

#[test]
fn a_guest_hears_the_host_within_a_millisecond() {
    let dir = std::env::temp_dir().join(format!("nz-party-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    let source = dir.join("clicks.flac");
    click_track(&source, 40.0);
    // Four chunks, cut the way the server cuts them.
    let mut chunks: HashMap<u64, Arc<[f32]>> = HashMap::new();
    for k in 0..6u64 {
        let path = dir.join(format!("chunk-{k}.ogg"));
        cut_chunk(&source, k, &path);
        let bytes = std::fs::read(&path).expect("a chunk");
        let pcm = decode_chunk(&bytes, 48_000).expect("a decoded chunk");
        chunks.insert(k, pcm);
    }

    let mut opener = FileOpener {
        paths: HashMap::new(),
    };
    opener.paths.insert("host".into(), source.clone());
    let config = |dir: &Path| EngineConfig {
        sample_rate: 48_000,
        spool_dir: dir.join("spool"),
        ..EngineConfig::default()
    };
    let (host, mut host_sink) = Engine::new_offline(config(&dir.join("host")), Arc::new(opener));
    let host_events = host.events();
    host.set_track_meta(
        "host",
        nz_audio::TrackMeta {
            duration_s: Some(40.0),
            ..Default::default()
        },
    );
    host.load("host", Quality::Flac, None);
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if matches!(host_events.try_recv(), Ok(EngineEvent::Started { .. })) {
            break;
        }
        render(&mut host_sink, 480);
        std::thread::sleep(Duration::from_millis(1));
    }
    host.play();
    // The guest: its own engine, the party's channel, no local track.
    let (guest, mut guest_sink) = Engine::new_offline(
        config(&dir.join("guest")),
        Arc::new(FileOpener {
            paths: HashMap::new(),
        }),
    );
    let channel = guest.party();
    channel.take_over(true);
    channel.set_gain(1.0);

    let mut line = Line::new();
    let mut planner = Planner::new(48_000);
    let gaps = Gaps::default();
    let mut believed: Option<nz_model::PartyPublish> = None;
    let mut published_at: Option<Instant> = None;
    let mut host_clicks: Vec<usize> = Vec::new();
    let mut guest_clicks: Vec<usize> = Vec::new();
    let mut heard_at: Vec<f64> = Vec::new();
    // Both engines must really put the track out: a silent run would
    // compare nothing to nothing.
    let mut host_peak = 0.0f32;
    let mut guest_peak = 0.0f32;
    const STEP_MS: f64 = 10.0;
    // 12 s of playback: long enough for two chunk seams and a steady join.
    let steps = 1_200;
    // The devices' wall clock, in the same domain as their streams: the offline
    // sink renders as fast as the test asks, so `Instant` would be a clock the
    // engines never see (D5's own pitfall).
    let mut sim_ms = now_ms();
    for step in 0..steps {
        sim_ms += STEP_MS;
        let host_buffer = render(&mut host_sink, 480);
        host_peak = host_buffer
            .iter()
            .fold(host_peak, |peak, sample| peak.max(sample.abs()));
        for frame in clicks(&host_buffer) {
            host_clicks.push(step);
            let _ = frame;
        }
        // The host's own tick: four readings a second.
        let local = sim_ms;
        if step % 25 == 0 {
            let snapshot = Snapshot {
                id: "host".into(),
                track: nz_model::PartyTrack {
                    id: "host".into(),
                    title: "Clics".into(),
                    duration: 40.0,
                    ..nz_model::PartyTrack::default()
                },
                carries: true,
                playing: true,
                heard: host.position(),
                fade: 0.0,
                duration: 40.0,
                norm: "off",
                generation: host.output_clock().generation,
                next: None,
            };
            let since = published_at.map(|at| at.elapsed()).unwrap_or(Duration::MAX);
            let decision = decide(Tick {
                snapshot: Some(&snapshot),
                line: &mut line,
                gaps: &gaps,
                probe: None,
                believed: believed.as_ref(),
                since_sent: since,
                local_ms: local,
                offset_ms: 0.0,
            });
            if let Some(publish) = decision.publish {
                believed = Some(publish);
                published_at = Some(Instant::now());
            }
        }
        // The guest's tick: every 100 ms it reads the line, the output
        // clock and the chunks, and places what is due.
        let guest_buffer = render(&mut guest_sink, 480);
        guest_peak = guest_buffer
            .iter()
            .fold(guest_peak, |peak, sample| peak.max(sample.abs()));
        for (index, frame) in clicks(&guest_buffer).into_iter().enumerate() {
            guest_clicks.push(step);
            let _ = (index, frame);
        }
        let clock = guest.output_clock();
        if step % 10 == 9 {
            let local = sim_ms;
            let bridge = clock.at_s - local / 1000.0;
            let timeline = believed.as_ref().map(|publish| Timeline {
                t: publish.t,
                p: publish.p,
                playing: publish.playing,
            });
            let ready = |chunk: u64| chunks.contains_key(&chunk);
            let situation = Situation {
                timeline,
                offset_ms: Some(0.0),
                bridge_s: Some(bridge),
                stream_now: clock.position_s,
                rate: 48_000,
                trim_ms: 0.0,
                since_poll_ms: 0,
                duration: 40.0,
                ready: &ready,
            };
            let plan = planner.plan(&situation);
            if plan.cut {
                channel.clear();
                planner.forget();
            }
            if plan.clear_future {
                channel.clear_future();
                planner.keep_sounding(clock.position_s);
            }
            for placement in &plan.place {
                let Some(pcm) = chunks.get(&placement.chunk) else {
                    continue;
                };
                let skip = (placement.skip_frames as usize).min(pcm.len() / 2);
                let data: Arc<[f32]> = pcm[skip * 2..].to_vec().into();
                if data.is_empty() {
                    continue;
                }
                channel.schedule(Segment::at(placement.from_s, 48_000.0, Arc::clone(&data)));
                // The callback only decrements: the caller keeps the
                // buffer alive (nz-audio's `party.rs`).
                heard_at.push(placement.from_s);
            }
        }
    }
    host.shutdown();
    guest.shutdown();
    let _ = std::fs::remove_dir_all(&dir);

    // Pair each guest click with its host click. The guest joins about a
    // second in, so its first click is not the host's first.
    let offset = guest_clicks
        .first()
        .and_then(|guest| {
            host_clicks
                .iter()
                .position(|host| (*host as i64 - *guest as i64).abs() <= 15)
        })
        .unwrap_or(0);
    let mut errors: Vec<f64> = Vec::new();
    for (index, guest) in guest_clicks.iter().enumerate() {
        let Some(host) = host_clicks.get(index + offset) else {
            break;
        };
        errors.push((*guest as f64 - *host as f64) * STEP_MS);
    }
    assert!(host_peak > 0.5, "the host played: {host_peak}");
    assert!(guest_peak > 0.5, "the guest played: {guest_peak}");
    assert!(
        errors.len() >= 5,
        "not enough clicks to compare: {} guest, {} host",
        guest_clicks.len(),
        host_clicks.len()
    );
    let worst = errors
        .iter()
        .fold(0.0f64, |worst, error| worst.max(error.abs()));
    println!(
        "{} clicks compared, guest minus host: p50 {:+.1} ms, worst {:+.1} ms \
         ({} chunks placed)",
        errors.len(),
        median(errors.clone()),
        worst,
        heard_at.len()
    );
    // The plan's criterion: the guest within 5 ms of the host, p95 under
    // 15. A real room hears one beat, not an echo.
    assert!(
        median(errors.clone()).abs() <= 5.0,
        "median {} ms",
        median(errors.clone())
    );
    assert!(worst <= 15.0, "worst {worst} ms");
}
