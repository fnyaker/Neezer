//! The supervisor half of a run: two processes, a SIGSTOP for a pause, a resume
//! from the last chunk — owned by lot T3.
//!
//! The engine is a child *of the supervisor* (`neezer-transcribe engine`), not a
//! thread: a pause is a `SIGSTOP` of the child's process group, so a stalled
//! whisper cannot slow the person's music (rule 1), and the lease keeps being
//! heartbeated while it is stopped. T1 pins the one timing that makes the pause
//! usable instead of a flicker ([`RESUME_AFTER_MS`]).
//!
//! Three things this module refuses to do, each a rule of the project:
//!
//! - **Never block the heartbeat.** The heartbeat is its own task, so a job
//!   whose engine is stopped for a minute still renews its lease: a pause that
//!   lost the lease would be a pause that destroys an hour of work.
//! - **Never decode audio here.** A chunk's PCM comes from a [`PcmSource`], and
//!   the production one runs the crate's own decoder in its own process
//!   (`neezer-transcribe pcm <audio> --start-ms … --length-ms … --out …`, T2's
//!   `audio.rs`). The supervisor owns HTTP, chunk boundaries, the child process
//!   and its signals; a second decoder in this file would be a second thing to
//!   keep equal to the engine's.
//! - **Never invent a progress value.** A chunk's `chunk_pct` is the engine's
//!   own `progress = N %` output, or 0: a made-up percentage is a progress bar
//!   that lies (rule 2).

use std::ffi::OsString;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Process, ExitCode, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::control::{self, Command, Event};
use crate::error::Error;
use crate::protocol::{self, ChunkBody, Claim, CompleteBody, FailBody};
use crate::segments::{self, Segment};
use crate::sidecar::{self, Meta};
use crate::worker::{Client, Failure, HEARTBEAT_SECONDS};

/// How long every pause reason must be gone before the engine resumes.
///
/// A `User`-lane request, a buffer low-water mark or one slow second of frames
/// are all *bursty*: resuming on the first quiet millisecond would start and
/// stop whisper dozens of times per track, and each start costs the first GPU
/// call's pipeline compilation. Three seconds is the note's measured value.
pub const RESUME_AFTER_MS: u64 = 3_000;

/// The audio engine's buffered-ahead time under which the engine is paused.
///
/// Below two seconds of decoded audio a stalled decode thread would be audible,
/// so a transcription that needs the CPU or the GPU yields before it is (rule 1).
pub const AUDIO_LOW_WATER_MS: u64 = 2_000;

/// How long a `User`-lane request may be in flight before the engine pauses.
pub const USER_LANE_GRACE_MS: u64 = 300;

/// How often the supervisor looks at the engine child: its exit, its progress
/// output, and whether a pause is due. Two `try_wait`s and one small file read.
const ENGINE_POLL: Duration = Duration::from_millis(100);

/// Floor of an engine deadline, in seconds (`worker.run_process`).
const ENGINE_TIMEOUT_FLOOR_S: f64 = 900.0;

/// The engine gets this many times a chunk's length (`worker.run_process`).
const ENGINE_TIMEOUT_FACTOR: f64 = 6.0;

/// How long a stopped engine gets between `SIGTERM` and `SIGKILL`.
const TERM_GRACE: Duration = Duration::from_secs(5);

/// How often the idle loop looks at the control channel, in milliseconds.
const CONTROL_POLL_MS: u64 = 25;

/// The `nice` value of every child this module starts (the note's `nice 19`).
const CHILD_NICE: i32 = 19;

// Linux signal numbers. The desktop app is Linux-only (Omarchy), and pulling in
// `libc` for four constants and three calls would be a dependency the manifest's
// owner (T1) forbade the later lots to add.
const SIGCONT: i32 = 18;
const SIGSTOP: i32 = 19;
const SIGTERM: i32 = 15;
const SIGKILL: i32 = 9;

unsafe extern "C" {
    fn kill(pid: i32, signal: i32) -> i32;
    fn nice(adjustment: i32) -> i32;
}

/// How one job ended, as the caller's loop cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The transcript is stored: the server's row is `ready`, or the local
    /// sidecar is written.
    Done,
    /// Nothing was done, or the job was given back.
    Idle,
    /// The device token was refused or revoked: stop asking.
    Fatal,
}

/// The PCM seam: where one chunk's 16 kHz mono f32le audio comes from.
///
/// The returned value is the PCM's duration in whole milliseconds
/// ([`crate::audio::pcm_duration_ms`]). A SHORT read is how the local run learns
/// it reached the end of the episode, without parsing one byte of the container.
pub trait PcmSource: Send + Sync + 'static {
    /// Write `length_ms` of audio starting at `start_ms` into `out`, raw f32le.
    fn write_pcm(
        &self,
        audio: &Path,
        start_ms: u64,
        length_ms: u64,
        out: &Path,
    ) -> Result<u64, Error>;
}

/// The production [`PcmSource`]: the crate's own decoder (`audio.rs`, T2).
///
/// `decode_range` decodes only the window asked for, so a three-hour episode
/// costs one chunk's samples in memory, and the sample count it returns is the
/// chunk's real length — a file shorter than its claim is a short PCM, which is
/// exactly how the local run learns where the episode ends.
#[derive(Debug, Default, Clone, Copy)]
pub struct AudioPcm;

impl PcmSource for AudioPcm {
    fn write_pcm(
        &self,
        audio: &Path,
        start_ms: u64,
        length_ms: u64,
        out: &Path,
    ) -> Result<u64, Error> {
        let samples = crate::audio::decode_range(audio, start_ms as i64, length_ms as i64)?;
        crate::audio::write_pcm(out, &samples).map_err(|error| {
            tracing::warn!("PCM {} : {error}", out.display());
            Error::NoAudio
        })?;
        Ok(crate::audio::samples_duration_ms(samples.len()))
    }
}

/// The supervisor's state flags, shared with the heartbeat and the engine wait.
///
/// One struct of atomics rather than a channel per reason: the engine poll reads
/// these ten times a second and must never wait on a lock.
#[derive(Debug, Default)]
pub struct Halt {
    stop: AtomicBool,
    cancel: AtomicBool,
    lost: AtomicBool,
    revoked: AtomicBool,
    /// The app's pause (rule 1: audio, a `User` request, a slow frame).
    pause: AtomicBool,
    /// The server's pause. A `client` worker never receives one (`job.heartbeat`
    /// answers `pause` for `server` workers), so this bit stays false — but it is
    /// a SEPARATE bit on purpose: an answer of `pause: false` from a heartbeat
    /// must not cancel the pause the app asked for.
    server_pause: AtomicBool,
}

impl Halt {
    /// The person, the app or the CLI asked for a clean stop.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }

    /// The server asked to cancel this episode.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }

    /// The lease is gone: nothing this worker does with it can land.
    pub fn lose(&self) {
        self.lost.store(true, Ordering::Release);
    }

    /// The device token was refused: the whole worker must stop.
    pub fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
        self.stop();
    }

    /// Pause (or resume) the engine on the app's behalf. The hysteresis is the
    /// engine wait's.
    pub fn pause(&self, paused: bool) {
        self.pause.store(paused, Ordering::Release);
    }

    /// Pause (or resume) on the server's behalf, without touching the app's bit.
    pub fn server_pause(&self, paused: bool) {
        self.server_pause.store(paused, Ordering::Release);
    }

    /// Whether this job must end now.
    pub fn stopping(&self) -> bool {
        self.stop.load(Ordering::Acquire)
            || self.cancel.load(Ordering::Acquire)
            || self.lost.load(Ordering::Acquire)
    }

    /// Whether a pause was asked for, by either side.
    pub fn paused(&self) -> bool {
        self.pause.load(Ordering::Acquire) || self.server_pause.load(Ordering::Acquire)
    }

    /// Whether the server cancelled this episode (as opposed to a stop).
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    /// Whether the lease was lost under this job.
    pub fn lost(&self) -> bool {
        self.lost.load(Ordering::Acquire)
    }

    /// Whether the device token is no longer accepted.
    pub fn revoked(&self) -> bool {
        self.revoked.load(Ordering::Acquire)
    }

    /// Forget the per-job reasons, keeping the worker-wide ones.
    pub fn reset(&self) {
        self.cancel.store(false, Ordering::Release);
        self.lost.store(false, Ordering::Release);
        self.pause.store(false, Ordering::Release);
        self.server_pause.store(false, Ordering::Release);
    }
}

/// Where the supervisor tells the app what is happening (rule 2).
pub trait EventSink: Send + Sync + 'static {
    /// One event, delivered once.
    fn emit(&self, event: Event);
}

/// The production sink: one JSON line per event on stdout, flushed.
#[derive(Debug, Default, Clone, Copy)]
pub struct StdoutSink;

impl EventSink for StdoutSink {
    fn emit(&self, event: Event) {
        let mut stdout = std::io::stdout().lock();
        if let Err(error) = control::write_event(&mut stdout, &event) {
            tracing::warn!("événement non écrit : {error}");
        }
    }
}

/// A sink that keeps every event, for the tests and for the app.
#[derive(Debug, Default)]
pub struct ReplaySink {
    events: Mutex<Vec<Event>>,
}

impl ReplaySink {
    /// Everything emitted so far.
    pub fn events(&self) -> Vec<Event> {
        self.events.lock().expect("sink lock").clone()
    }

    /// The events, newest last, as their JSON tags.
    pub fn tags(&self) -> Vec<String> {
        self.events
            .lock()
            .expect("sink lock")
            .iter()
            .map(|event| {
                serde_json::to_value(event)
                    .ok()
                    .and_then(|value| {
                        value
                            .get("event")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
                    .unwrap_or_default()
            })
            .collect()
    }
}

impl EventSink for ReplaySink {
    fn emit(&self, event: Event) {
        self.events.lock().expect("sink lock").push(event);
    }
}

/// The engine child: `neezer-transcribe engine --pcm … --model … --lang … --out …`.
#[derive(Debug, Clone)]
pub struct Engine {
    /// The binary: this one by default.
    pub program: PathBuf,
    /// The GGML model file.
    pub model: PathBuf,
    /// Ask for the CPU ([`crate::engine::FallbackCause::UserForcedCpu`]).
    pub cpu: bool,
}

impl Engine {
    /// The child's arguments for one chunk.
    ///
    /// Four flags are the note's contract with T2: a PCM file in, a whisper JSON
    /// out, and the language.
    pub fn args(&self, pcm: &Path, out: &Path, language: &str) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec![
            "engine".into(),
            "--pcm".into(),
            pcm.into(),
            "--model".into(),
            self.model.clone().into(),
            "--lang".into(),
            language.into(),
            "--out".into(),
            out.into(),
        ];
        if self.cpu {
            args.push("--cpu".into());
        }
        args
    }
}

/// A running engine child whose process group this supervisor can signal.
#[derive(Debug)]
pub struct EngineChild {
    child: Child,
    group: i32,
    stopped: bool,
}

impl EngineChild {
    /// Start `engine` for one chunk, in its own process group, reniced.
    ///
    /// The group is what makes a pause whole: whisper runs worker threads, and
    /// `SIGSTOP` on the group stops all of them.
    pub fn spawn(
        engine: &Engine,
        pcm: &Path,
        out: &Path,
        language: &str,
        stderr: &Path,
    ) -> std::io::Result<EngineChild> {
        let log = fs::File::create(stderr)?;
        let mut command = Process::new(&engine.program);
        command
            .args(engine.args(pcm, out, language))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(log))
            .nice();
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let child = command.spawn()?;
        let group = child.id() as i32;
        Ok(EngineChild {
            child,
            group,
            stopped: false,
        })
    }

    /// The child's pid, which is also its process group.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// `SIGSTOP` the whole group, once.
    pub fn pause(&mut self) {
        if !self.stopped {
            signal_group(self.group, SIGSTOP);
            self.stopped = true;
        }
    }

    /// `SIGCONT` the whole group, once.
    pub fn resume(&mut self) {
        if self.stopped {
            signal_group(self.group, SIGCONT);
            self.stopped = false;
        }
    }

    /// Whether a `SIGSTOP` is in effect.
    pub fn is_paused(&self) -> bool {
        self.stopped
    }

    /// Poll the child without blocking.
    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    /// Stop the child for good: cont, term, then kill after [`TERM_GRACE`].
    pub fn terminate(&mut self) {
        // A stopped process does not act on SIGTERM until it is continued.
        signal_group(self.group, SIGCONT);
        self.stopped = false;
        signal_group(self.group, SIGTERM);
        let deadline = Instant::now() + TERM_GRACE;
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        signal_group(self.group, SIGKILL);
        let _ = self.child.wait();
    }
}

impl Drop for EngineChild {
    fn drop(&mut self) {
        // A dropped child that is still running would whisper for ever,
        // unsupervised, on someone's laptop.
        if matches!(self.child.try_wait(), Ok(None)) {
            self.terminate();
        }
    }
}

/// `kill(-pgid, signal)`: the whole group, because the engine has threads.
fn signal_group(group: i32, signal: i32) {
    if group <= 0 {
        return;
    }
    // SAFETY: `kill` with a negative pid signals a process group; both arguments
    // are plain integers and the call touches no memory.
    unsafe {
        kill(-group, signal);
    }
}

/// `nice(19)` on the child, inside the child. `nice` is a syscall wrapper, which
/// is what `pre_exec` requires of it.
trait NiceCommand {
    fn nice(&mut self) -> &mut Self;
}

impl NiceCommand for Process {
    fn nice(&mut self) -> &mut Self {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // SAFETY: `nice(2)` only changes the calling (child) process's
            // priority before `exec`; it allocates nothing and touches no shared
            // memory, so it is safe in the post-fork window.
            unsafe {
                self.pre_exec(|| {
                    nice(CHILD_NICE);
                    Ok(())
                });
            }
        }
        self
    }
}

/// What the engine heard in one chunk.
#[derive(Debug, Clone, PartialEq)]
pub struct Recognized {
    /// The raw `transcription` array of whisper.cpp's JSON.
    pub items: Vec<Value>,
    /// The language the engine reported, normalised.
    pub language: Option<String>,
    /// `vulkan` or `cpu`, as the engine child's document says.
    pub backend: Option<String>,
    /// The device the engine chose, when it names one.
    pub device: Option<String>,
    /// Why the CPU was used, when that is the answer.
    pub reason: Option<String>,
}

/// `engine.py#parse_json`: whisper.cpp's `-oj` output → its raw lines.
pub fn parse_engine_json(bytes: &[u8]) -> Result<Recognized, Error> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| Error::BadJson)?;
    let object = value.as_object().ok_or(Error::BadJsonShape)?;
    let items = object
        .get("transcription")
        .and_then(Value::as_array)
        .ok_or(Error::NoTranscription)?
        .clone();
    let language = object
        .get("result")
        .and_then(|result| result.get("language"))
        .and_then(Value::as_str)
        .and_then(|value| protocol::normalize_language(Some(value)));
    let text = |key: &str| object.get(key).and_then(Value::as_str).map(str::to_owned);
    Ok(Recognized {
        items,
        language,
        backend: text("backend"),
        device: text("device"),
        reason: text("reason"),
    })
}

/// One episode being transcribed, in either mode.
#[derive(Debug, Clone)]
pub struct Job {
    /// The episode, a UUID string.
    pub episode_id: String,
    /// Milliseconds of audio transcribed so far.
    pub done_ms: i64,
    /// The episode's duration: the server's claim, or `None` while a local file's
    /// length is still unknown.
    pub duration_ms: Option<i64>,
    /// The language, or `None` for `auto`.
    pub language: Option<String>,
    /// Audio per chunk.
    pub chunk_ms: i64,
    /// Milliseconds a chunk shares with the previous one.
    pub overlap_ms: i64,
    /// The lines so far (a resumed local job starts with them).
    pub segments: Vec<Segment>,
}

impl Job {
    /// A job from a server claim.
    pub fn from_claim(claim: &Claim, forced_language: Option<&str>) -> Job {
        Job {
            episode_id: claim.episode_id.clone(),
            done_ms: claim.start_ms.max(0),
            duration_ms: Some(claim.duration_ms),
            language: claim
                .language_or(forced_language)
                .filter(|language| language != "auto"),
            chunk_ms: claim.chunk_ms(),
            overlap_ms: claim.overlap_ms(),
            segments: Vec::new(),
        }
    }

    /// A fresh local job.
    pub fn local(episode_id: &str, language: Option<&str>) -> Job {
        Job {
            episode_id: episode_id.to_owned(),
            done_ms: 0,
            duration_ms: None,
            language: protocol::normalize_language(language).filter(|language| language != "auto"),
            chunk_ms: segments::CHUNK_SECONDS * 1000,
            overlap_ms: segments::OVERLAP_MS,
            segments: Vec::new(),
        }
    }

    /// The language the engine is asked for (`-l auto` when nobody knows yet).
    fn engine_language(&self) -> String {
        self.language.clone().unwrap_or_else(|| "auto".to_owned())
    }
}

/// What a local job leaves behind, so a restart resumes at its last chunk.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Partial {
    episode_id: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    done_ms: i64,
    #[serde(default)]
    segments: Vec<Segment>,
}

/// The supervisor: the job loop, the engine child and the control channel.
pub struct Supervisor {
    /// The server's worker routes; `None` in local mode.
    pub client: Option<Client>,
    /// The engine child's program and model.
    pub engine: Engine,
    /// Where a chunk's PCM comes from.
    pub pcm: Arc<dyn PcmSource>,
    /// The job's stop/pause flags.
    pub halt: Arc<Halt>,
    /// Where the events go.
    pub sink: Arc<dyn EventSink>,
    /// Scratch space (audio download, PCM, engine JSON).
    pub work: PathBuf,
    /// Where a local transcript lands.
    pub transcripts: PathBuf,
    /// Wait between two claims when the queue is empty.
    pub poll: Duration,
    /// Wait between two heartbeats.
    pub heartbeat: Duration,
    /// How long a pause reason must be gone before the engine runs again.
    pub resume_after: Duration,
    /// The engine's version, stamped on every chunk and transcript.
    pub engine_version: String,
    /// A language this machine forces, overriding the claim's.
    pub language: Option<String>,
    /// Whether the engine's backend was already announced for this job.
    pub announced: bool,
    /// Whether the empty queue was already announced, so a five-second poll does
    /// not write `{"event":"idle"}` twelve times a minute into the app's pipe.
    pub idle_announced: bool,
}

impl Supervisor {
    /// A supervisor with the note's defaults and every dependency injected.
    pub fn new(
        engine: Engine,
        pcm: Arc<dyn PcmSource>,
        sink: Arc<dyn EventSink>,
        work: PathBuf,
        transcripts: PathBuf,
    ) -> Supervisor {
        Supervisor {
            client: None,
            engine,
            pcm,
            halt: Arc::new(Halt::default()),
            sink,
            work,
            transcripts,
            poll: Duration::from_secs_f64(protocol::POLL_SECONDS),
            heartbeat: Duration::from_secs(HEARTBEAT_SECONDS),
            resume_after: Duration::from_millis(RESUME_AFTER_MS),
            engine_version: crate::WHISPER_VERSION.to_owned(),
            language: None,
            announced: false,
            idle_announced: false,
        }
    }

    /// The server loop: claim, transcribe, complete, until stopped.
    pub async fn run_worker(&mut self, mut commands: Option<mpsc::Receiver<Command>>) -> Outcome {
        loop {
            if self.halt.stopping() {
                return if self.halt.revoked() {
                    Outcome::Fatal
                } else {
                    Outcome::Idle
                };
            }
            let Some(client) = self.client.clone() else {
                return Outcome::Fatal;
            };
            match client.claim().await {
                Ok(Some(claim)) => {
                    self.idle_announced = false;
                    if self.run_claim(claim).await == Outcome::Fatal {
                        return Outcome::Fatal;
                    }
                }
                Ok(None) => {
                    if !self.idle_announced {
                        self.idle_announced = true;
                        self.sink.emit(Event::Idle);
                    }
                    if !self.idle_wait(&mut commands).await {
                        return Outcome::Idle;
                    }
                }
                Err(failure) if failure.fatal() => {
                    self.sink
                        .emit(Event::error("", failure.code(), failure.to_string()));
                    return Outcome::Fatal;
                }
                Err(failure) => {
                    tracing::warn!("claim : {failure}");
                    self.sink
                        .emit(Event::error("", failure.code(), failure.to_string()));
                    if !self.idle_wait(&mut commands).await {
                        return Outcome::Idle;
                    }
                }
            }
        }
    }

    /// Wait for the poll interval, applying commands meanwhile.
    ///
    /// Returns false when the worker must end: a stop, or a closed channel that
    /// was the only reason to stay.
    async fn idle_wait(&mut self, commands: &mut Option<mpsc::Receiver<Command>>) -> bool {
        let deadline = Instant::now() + self.poll;
        loop {
            if let Some(receiver) = commands.as_mut() {
                match receiver.try_recv() {
                    Ok(Command::Stop) => {
                        self.halt.stop();
                        return false;
                    }
                    Ok(Command::Pause) => self.halt.pause(true),
                    Ok(Command::Resume) => self.halt.pause(false),
                    Ok(Command::Local {
                        episode_id,
                        audio,
                        language,
                    }) => {
                        self.run_local_job(&episode_id, Path::new(&audio), language.as_deref())
                            .await;
                        return true;
                    }
                    Err(mpsc::error::TryRecvError::Empty) => {}
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        // The app is gone: keep working, there may be a queue.
                        *commands = None;
                        break;
                    }
                }
            }
            if Instant::now() >= deadline {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(CONTROL_POLL_MS)).await;
        }
        true
    }

    /// One claim from the server: download, transcribe chunk by chunk, complete.
    pub async fn run_claim(&mut self, claim: Claim) -> Outcome {
        self.halt.reset();
        self.announced = false;
        let episode_id = claim.episode_id.clone();
        let lease = claim.lease.clone();
        let Some(client) = self.client.clone() else {
            return Outcome::Fatal;
        };
        let mut job = Job::from_claim(&claim, self.language.as_deref());
        if job.duration_ms.unwrap_or(0) <= 0 {
            let failure = Failure::BadClaim("durée de l'épisode inconnue".to_owned());
            let _ = client
                .send_fail(
                    &episode_id,
                    &lease,
                    &FailBody::new("no_duration", failure.to_string()),
                )
                .await;
            self.sink.emit(Event::error(
                &episode_id,
                "no_duration",
                failure.to_string(),
            ));
            return Outcome::Idle;
        }
        let workdir = self.job_dir(&episode_id);
        if let Err(error) = fs::create_dir_all(&workdir) {
            self.sink.emit(Event::error(
                &episode_id,
                "no_space",
                format!(
                    "dossier temporaire impossible ({})",
                    crate::worker::kind_fr(error.kind())
                ),
            ));
            return Outcome::Idle;
        }

        // The heartbeat starts before the download: an episode can be 150 MB,
        // and a lease nobody renews during the download is a job lost in the
        // middle of it (`worker.process_claim` starts it the same way).
        let chunk_pct = Arc::new(AtomicU64::new(0));
        let heartbeat = {
            let client = client.clone();
            let halt = self.halt.clone();
            let episode_id = episode_id.clone();
            let lease = lease.clone();
            let pct = chunk_pct.clone();
            let interval = self.heartbeat;
            tokio::spawn(async move {
                heartbeat_loop(client, episode_id, lease, halt, pct, interval).await;
            })
        };

        let audio = workdir.join("audio");
        let halt = self.halt.clone();
        let download = client
            .download(&episode_id, &lease, &audio, move || halt.stopping())
            .await;
        if let Err(failure) = download {
            heartbeat.abort();
            let _ = fs::remove_dir_all(&workdir);
            return self
                .finish_failure(&client, &episode_id, &lease, failure)
                .await;
        }

        let outcome = self
            .transcribe(&mut job, &audio, &workdir, &client, &lease, &chunk_pct)
            .await;
        heartbeat.abort();
        let _ = fs::remove_dir_all(&workdir);
        match outcome {
            Ok(()) => Outcome::Done,
            Err(failure) => {
                self.finish_failure(&client, &episode_id, &lease, failure)
                    .await
            }
        }
    }

    /// The chunk loop of a server job: PCM, engine, chunk, advance.
    async fn transcribe(
        &mut self,
        job: &mut Job,
        audio: &Path,
        workdir: &Path,
        client: &Client,
        lease: &str,
        chunk_pct: &Arc<AtomicU64>,
    ) -> Result<(), Failure> {
        let duration = job.duration_ms.unwrap_or(0);
        loop {
            if self.halt.stopping() {
                return Err(self.halt_failure());
            }
            if job.done_ms >= duration {
                match self.complete(job, client, lease).await {
                    Ok(()) => return Ok(()),
                    Err(Failure::Incomplete { done_ms }) if done_ms > job.done_ms => {
                        // The server is ahead of us: continue from ITS position.
                        job.done_ms = done_ms;
                        continue;
                    }
                    Err(failure) => return Err(failure),
                }
            }
            let length = job.chunk_ms.min(duration - job.done_ms);
            let (recognized, written) = self
                .recognize(job, audio, workdir, length, chunk_pct)
                .await
                .map_err(engine_failure)?;
            self.announce(&recognized);
            if written == 0 {
                return Err(Failure::Engine(Error::EmptyAudio));
            }
            if written < length as u64 {
                tracing::warn!(
                    "épisode {} : {} ms de PCM pour un chunk de {} ms",
                    job.episode_id,
                    written,
                    length
                );
            }
            if let Some(language) = recognized.language
                && job.language.is_none()
            {
                job.language = Some(language);
            }
            let body = ChunkBody::new(
                job.done_ms,
                length,
                job.engine_language(),
                self.model_name(),
                self.engine_version.clone(),
                protocol::fit_segments(segments::normalize_segments(
                    &recognized.items,
                    job.done_ms,
                    length,
                    job.overlap_ms,
                )),
            );
            let start = job.done_ms;
            let reply = client.send_chunk(&job.episode_id, lease, &body).await?;
            if !reply.applied || reply.done_ms <= start {
                tracing::warn!(
                    "épisode {} : aucun progrès (done_ms {}), reprise au prochain claim",
                    job.episode_id,
                    reply.done_ms
                );
                let _ = client.send_release(&job.episode_id, lease).await;
                return Err(Failure::Cancelled);
            }
            job.done_ms = reply.done_ms;
            job.segments.extend(body.segments);
            self.sink.emit(Event::Progress {
                episode_id: job.episode_id.clone(),
                done_ms: job.done_ms,
                duration_ms: duration,
            });
            chunk_pct.store(0, Ordering::Release);
        }
    }

    /// `POST …/complete`, with the server's `incomplete` answer surfaced.
    async fn complete(&mut self, job: &Job, client: &Client, lease: &str) -> Result<(), Failure> {
        let body = CompleteBody::new(
            job.language.clone(),
            self.model_name(),
            self.engine_version.clone(),
        );
        match client.send_complete(&job.episode_id, lease, &body).await {
            Ok(reply) if reply.ready() => {
                self.sink.emit(Event::Done {
                    episode_id: job.episode_id.clone(),
                    segments: job.segments.clone(),
                });
                Ok(())
            }
            Ok(reply) => Err(Failure::Http {
                status: 200,
                code: "complete".to_owned(),
                message: format!(
                    "le serveur n'a pas rendu la transcription prête ({})",
                    reply.status
                ),
            }),
            Err(failure) => Err(failure),
        }
    }

    /// Store a job's failure where the person will read it (rule 2).
    async fn finish_failure(
        &mut self,
        client: &Client,
        episode_id: &str,
        lease: &str,
        failure: Failure,
    ) -> Outcome {
        match failure {
            Failure::Cancelled => {
                // A cancel, a pause or a shutdown gives the lease back without a
                // verdict: the job is not failed, it is not done either.
                if !self.halt.lost() && !self.halt.revoked() {
                    let _ = client.send_release(episode_id, lease).await;
                }
                if self.halt.lost() {
                    self.sink.emit(Event::error(
                        episode_id,
                        "lease_expired",
                        "le lease de cet épisode a expiré".to_owned(),
                    ));
                }
                Outcome::Idle
            }
            Failure::LeaseLost => {
                self.halt.lose();
                self.sink.emit(Event::error(
                    episode_id,
                    "lease_expired",
                    "le lease de cet épisode a expiré".to_owned(),
                ));
                Outcome::Idle
            }
            Failure::Auth {
                status,
                code,
                message,
            } => {
                self.halt.revoke();
                self.sink.emit(Event::error(
                    episode_id,
                    code,
                    format!("{status} : {message}"),
                ));
                Outcome::Fatal
            }
            Failure::Network(_) => {
                self.sink
                    .emit(Event::error(episode_id, "network", failure.to_string()));
                Outcome::Idle
            }
            Failure::Incomplete { done_ms } => {
                // The server will not advance any further: that is a verdict,
                // not something to retry (`worker.py`: "le serveur n'avance plus").
                let message = format!("le serveur s'arrête à {done_ms} ms");
                let _ = client
                    .send_fail(episode_id, lease, &FailBody::new("incomplete", &message))
                    .await;
                self.sink
                    .emit(Event::error(episode_id, "incomplete", message));
                Outcome::Idle
            }
            other => {
                let (code, message) = (other.code(), other.to_string());
                let _ = client
                    .send_fail(episode_id, lease, &FailBody::new(code, &message))
                    .await;
                self.sink.emit(Event::error(episode_id, code, message));
                Outcome::Idle
            }
        }
    }

    /// A local job: no server, a sidecar in the person's data directory.
    pub async fn run_local_job(
        &mut self,
        episode_id: &str,
        audio: &Path,
        language: Option<&str>,
    ) -> Outcome {
        if !protocol::is_episode_id(episode_id) {
            self.sink.emit(Event::error(
                episode_id,
                "bad_claim",
                "l'identifiant d'épisode n'est pas un UUID".to_owned(),
            ));
            return Outcome::Idle;
        }
        self.halt.reset();
        self.announced = false;
        let mut job = Job::local(episode_id, language);
        let partial = self.partial_path(episode_id);
        if let Some(saved) = load_partial(&partial) {
            job.done_ms = saved.done_ms.max(0);
            job.segments = saved.segments;
            job.language = saved.language.or(job.language);
            self.sink.emit(Event::Progress {
                episode_id: episode_id.to_owned(),
                done_ms: job.done_ms,
                duration_ms: job.done_ms,
            });
        }
        let workdir = self.job_dir(episode_id);
        if let Err(error) = fs::create_dir_all(&workdir) {
            self.sink.emit(Event::error(
                episode_id,
                "no_space",
                format!(
                    "dossier temporaire impossible ({})",
                    crate::worker::kind_fr(error.kind())
                ),
            ));
            return Outcome::Idle;
        }
        let chunk_pct = Arc::new(AtomicU64::new(0));
        let outcome = self
            .local_chunks(&mut job, audio, &workdir, &partial, &chunk_pct)
            .await;
        let _ = fs::remove_dir_all(&workdir);
        match outcome {
            Ok(()) => Outcome::Done,
            Err(Failure::Cancelled) => Outcome::Idle,
            Err(failure) => {
                let (code, message) = match &failure {
                    Failure::Engine(inner) => (inner.code(), inner.to_string()),
                    other => (other.code(), other.to_string()),
                };
                self.sink.emit(Event::error(episode_id, code, message));
                Outcome::Idle
            }
        }
    }

    /// The chunk loop of a local job: the decoder says where the episode ends.
    async fn local_chunks(
        &mut self,
        job: &mut Job,
        audio: &Path,
        workdir: &Path,
        partial: &Path,
        chunk_pct: &Arc<AtomicU64>,
    ) -> Result<(), Failure> {
        loop {
            if self.halt.stopping() {
                return Err(self.halt_failure());
            }
            let length = job.chunk_ms;
            let (recognized, written) = self
                .recognize(job, audio, workdir, length, chunk_pct)
                .await
                .map_err(engine_failure)?;
            self.announce(&recognized);
            if let Some(language) = recognized.language
                && job.language.is_none()
            {
                job.language = Some(language);
            }
            let lines = protocol::fit_segments(segments::normalize_segments(
                &recognized.items,
                job.done_ms,
                length,
                job.overlap_ms,
            ));
            job.segments.extend(lines);
            job.done_ms += written as i64;
            self.save_partial(partial, job);
            self.sink.emit(Event::Progress {
                episode_id: job.episode_id.clone(),
                done_ms: job.done_ms,
                duration_ms: job.done_ms,
            });
            chunk_pct.store(0, Ordering::Release);
            // A chunk shorter than the one asked for is the end of the audio
            // (`audio.pcm_duration_ms`: a short read is how the end is known).
            if written < length as u64 {
                break;
            }
        }
        let document = sidecar::build(
            Meta {
                episode_id: job.episode_id.clone(),
                language: job.language.clone(),
                engine: Some(crate::engine::FORMAT_ENGINE.to_owned()),
                model: Some(self.model_name()),
                engine_version: Some(self.engine_version.clone()),
                duration_ms: job.done_ms,
                created: Some(now_iso()),
                updated: Some(now_iso()),
            },
            &job.segments,
        )
        .map_err(|error| Failure::Config(error.to_string()))?;
        let pair = sidecar::pair_for(&self.transcripts.join(&job.episode_id));
        sidecar::write(&pair, &document)
            .map_err(|error| Failure::Config(format!("écriture du transcript : {error}")))?;
        let _ = fs::remove_file(partial);
        self.sink.emit(Event::Done {
            episode_id: job.episode_id.clone(),
            segments: job.segments.clone(),
        });
        Ok(())
    }

    /// Produce one chunk: PCM, then the engine child, then its JSON.
    ///
    /// Blocking work (a decode process, a child that whispers for seconds) runs
    /// on the blocking pool, so the heartbeat task keeps its place on the
    /// runtime's threads (rule 1: the lease outranks the audio).
    async fn recognize(
        &self,
        job: &Job,
        audio: &Path,
        workdir: &Path,
        length_ms: i64,
        chunk_pct: &Arc<AtomicU64>,
    ) -> Result<(Recognized, u64), Error> {
        let pcm = self.pcm.clone();
        let engine = self.engine.clone();
        let halt = self.halt.clone();
        let pct = chunk_pct.clone();
        let resume_after = self.resume_after;
        let audio = audio.to_path_buf();
        let workdir = workdir.to_path_buf();
        let start = job.done_ms.max(0) as u64;
        let length = length_ms.max(1) as u64;
        let language = job.engine_language();
        tokio::task::spawn_blocking(move || {
            let pcm_path = workdir.join(format!("chunk-{start}.pcm"));
            let json_path = workdir.join(format!("chunk-{start}.json"));
            let stderr_path = workdir.join(format!("chunk-{start}.err"));
            let _ = fs::remove_file(&json_path);
            let written = pcm.write_pcm(&audio, start, length, &pcm_path)?;
            if written == 0 {
                let _ = fs::remove_file(&pcm_path);
                return Ok((
                    Recognized {
                        items: Vec::new(),
                        language: None,
                        backend: None,
                        device: None,
                        reason: None,
                    },
                    0,
                ));
            }
            let recognized = run_engine(
                &engine,
                &pcm_path,
                &json_path,
                &stderr_path,
                &language,
                written,
                &halt,
                &pct,
                resume_after,
            )?;
            let _ = fs::remove_file(&stderr_path);
            Ok((recognized, written))
        })
        .await
        .map_err(|_| Error::NoAudio)?
    }

    /// Tell the app which processor ran, once per job (rule 2: it says why).
    fn announce(&mut self, recognized: &Recognized) {
        if self.announced {
            return;
        }
        let Some(backend) = recognized.backend.as_deref() else {
            return;
        };
        self.announced = true;
        let backend = if backend == "vulkan" {
            crate::engine::Backend::Vulkan
        } else {
            crate::engine::Backend::Cpu
        };
        let cause = recognized.reason.as_deref().and_then(|code| {
            [
                crate::engine::FallbackCause::NoVulkanLoader,
                crate::engine::FallbackCause::NoVulkanDevice,
                crate::engine::FallbackCause::GpuInitFailed,
                crate::engine::FallbackCause::GpuOutOfMemory,
                crate::engine::FallbackCause::UserForcedCpu,
            ]
            .into_iter()
            .find(|candidate| candidate.code() == code)
        });
        self.sink
            .emit(Event::engine(backend, recognized.device.clone(), cause));
    }

    /// A scratch directory of this job's own.
    fn job_dir(&self, episode_id: &str) -> PathBuf {
        let short: String = episode_id.chars().take(8).collect();
        self.work.join(format!("{short}-{}", std::process::id()))
    }

    /// `<episode_id>.partial.json`, next to the transcript it will become.
    pub fn partial_path(&self, episode_id: &str) -> PathBuf {
        self.transcripts.join(format!("{episode_id}.partial.json"))
    }

    /// The `Done` sidecar's two paths, for the app.
    pub fn transcript_pair(&self, episode_id: &str) -> sidecar::Pair {
        sidecar::pair_for(&self.transcripts.join(episode_id))
    }

    /// Remember a local job's progress, atomically: a restart resumes from the
    /// last chunk that landed, never from a half-written file.
    fn save_partial(&self, path: &Path, job: &Job) {
        let partial = Partial {
            episode_id: job.episode_id.clone(),
            language: job.language.clone(),
            done_ms: job.done_ms,
            segments: job.segments.clone(),
        };
        let Ok(bytes) = serde_json::to_vec(&partial) else {
            tracing::warn!("progression non enregistrée : sérialisation");
            return;
        };
        if let Some(parent) = path.parent()
            && let Err(error) = fs::create_dir_all(parent)
        {
            tracing::warn!(
                "progression non enregistrée ({})",
                crate::worker::kind_fr(error.kind())
            );
            return;
        }
        let temporary = path.with_extension("json.part");
        let written = fs::File::create(&temporary)
            .and_then(|mut file| file.write_all(&bytes).and_then(|()| file.sync_all()));
        match written {
            Ok(()) => {
                if let Err(error) = fs::rename(&temporary, path) {
                    tracing::warn!(
                        "progression non enregistrée ({})",
                        crate::worker::kind_fr(error.kind())
                    );
                }
            }
            Err(error) => tracing::warn!(
                "progression non enregistrée ({})",
                crate::worker::kind_fr(error.kind())
            ),
        }
    }

    /// The failure a stopped job ends with.
    fn halt_failure(&self) -> Failure {
        if self.halt.lost() {
            Failure::LeaseLost
        } else {
            Failure::Cancelled
        }
    }

    /// The model's file name, which is what the server stores.
    fn model_name(&self) -> String {
        self.engine
            .model
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// The engine's backend line, once (the app shows which processor it got).
    pub fn announce_backend(
        &self,
        backend: crate::engine::Backend,
        device: Option<String>,
        cause: Option<crate::engine::FallbackCause>,
    ) {
        self.sink.emit(Event::engine(backend, device, cause));
    }
}

/// The engine's failure as the job's: a cancellation is a cancellation.
fn engine_failure(error: Error) -> Failure {
    match error {
        Error::Cancelled => Failure::Cancelled,
        other => Failure::Engine(other),
    }
}

/// Read a partial sidecar, if the file is one.
fn load_partial(path: &Path) -> Option<Partial> {
    let bytes = fs::read(path).ok()?;
    if bytes.len() > (1 << 20) {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

/// The heartbeat task: keep the lease, carry the server's `cancel` and `pause`.
async fn heartbeat_loop(
    client: Client,
    episode_id: String,
    lease: String,
    halt: Arc<Halt>,
    chunk_pct: Arc<AtomicU64>,
    interval: Duration,
) {
    let mut failures = 0u32;
    loop {
        tokio::time::sleep(interval).await;
        if halt.stopping() {
            return;
        }
        let pct = Some(chunk_pct.load(Ordering::Acquire) as f64);
        match client.beat(&episode_id, &lease, pct).await {
            Ok(beat) => {
                failures = 0;
                if beat.cancel {
                    halt.cancel();
                }
                halt.server_pause(beat.pause);
            }
            Err(failure) if failure.lease_lost() => {
                halt.lose();
                return;
            }
            Err(failure) if failure.fatal() => {
                halt.revoke();
                return;
            }
            Err(failure) => {
                failures += 1;
                if failures == 1 || failures.is_multiple_of(12) {
                    tracing::warn!("heartbeat : {failure}");
                }
            }
        }
    }
}

/// Run the engine child for one chunk, pausing its process group as asked.
// Every argument is a fact the wait loop needs and none of them has a home in a
// struct yet; grouping them would be a struct with nine fields used once.
#[allow(clippy::too_many_arguments)]
fn run_engine(
    engine: &Engine,
    pcm: &Path,
    out: &Path,
    stderr: &Path,
    language: &str,
    length_ms: u64,
    halt: &Halt,
    chunk_pct: &AtomicU64,
    resume_after: Duration,
) -> Result<Recognized, Error> {
    let mut child =
        EngineChild::spawn(engine, pcm, out, language, stderr).map_err(|_| Error::NoAudio)?;
    let deadline = ENGINE_TIMEOUT_FLOOR_S.max(length_ms as f64 / 1000.0 * ENGINE_TIMEOUT_FACTOR);
    let mut paused_total = Duration::ZERO;
    let mut paused_at: Option<Instant> = None;
    let mut quiet_since: Option<Instant> = None;
    let mut log_seen = 0usize;
    let started = Instant::now();
    let status: ExitStatus = loop {
        if let Some(status) = child.try_wait().map_err(|_| Error::NoAudio)? {
            break status;
        }
        if halt.stopping() {
            child.terminate();
            let _ = fs::remove_file(pcm);
            return Err(Error::Cancelled);
        }
        // The pause state machine: stop at once; resume only after every reason
        // has been gone for `resume_after`, so a bursty trigger is one pause.
        if halt.paused() {
            quiet_since = None;
            child.pause();
            if paused_at.is_none() {
                paused_at = Some(Instant::now());
            }
        } else if child.is_paused() {
            let quiet = *quiet_since.get_or_insert_with(Instant::now);
            if quiet.elapsed() >= resume_after {
                child.resume();
                if let Some(at) = paused_at.take() {
                    paused_total += at.elapsed();
                }
                quiet_since = None;
            }
        }
        let running = started.elapsed().saturating_sub(paused_total);
        if running.as_secs_f64() > deadline {
            child.terminate();
            let _ = fs::remove_file(pcm);
            return Err(Error::Timeout);
        }
        // The engine's own progress, when whisper.cpp prints it. Never a guess:
        // a percentage this file made up would be a progress bar that lies.
        if let Ok(bytes) = fs::read(stderr)
            && bytes.len() > log_seen
        {
            log_seen = bytes.len();
            if let Some(percent) = last_percent(&bytes) {
                chunk_pct.store(u64::from(percent), Ordering::Release);
            }
        }
        std::thread::sleep(ENGINE_POLL);
    };
    let _ = fs::remove_file(pcm);
    if !status.success() {
        let detail =
            last_line(stderr).unwrap_or_else(|| format!("code {}", status.code().unwrap_or(-1)));
        return Err(Error::Exit(detail));
    }
    let bytes = fs::read(out).map_err(|_| Error::BadJson)?;
    let _ = fs::remove_file(out);
    parse_engine_json(&bytes)
}

/// The last non-empty line of a log file, cut at 120 characters.
fn last_line(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let line = text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())?
        .trim();
    if line.is_empty() {
        return None;
    }
    Some(line.chars().take(120).collect())
}

/// The last `progress = N %` of a whisper.cpp log (`worker._PROGRESS`).
fn last_percent(log: &[u8]) -> Option<u32> {
    let text = String::from_utf8_lossy(log);
    let marker = "progress =";
    let mut percent = None;
    let mut rest = text.as_ref();
    while let Some(at) = rest.find(marker) {
        rest = &rest[at + marker.len()..];
        if let Some(value) = parse_percent(rest) {
            percent = Some(value);
        }
    }
    percent
}

/// `N %` at the start of `rest`, if it is one.
fn parse_percent(rest: &str) -> Option<u32> {
    let text = rest.trim_start();
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() || !text[digits.len()..].trim_start().starts_with('%') {
        return None;
    }
    digits.parse::<u32>().ok().map(|percent| percent.min(100))
}

/// `2026-11-16T15:04:05`: the server's naive local time
/// (`job.sidecar_payload`), so a local sidecar reads like the server's.
pub fn now_iso() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0);
    format_local(seconds)
}

/// Seconds since the epoch as `YYYY-MM-DDTHH:MM:SS` in local time.
pub fn format_local(seconds: i64) -> String {
    let total = seconds + local_offset_seconds(seconds);
    let days = total.div_euclid(86_400);
    let rest = total.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// The local UTC offset at `seconds`, in seconds.
///
/// libc's `localtime_r` is the only dependency-free source of the timezone rules
/// and this is Linux-only code. An offset that cannot be read is UTC: a
/// timestamp one hour off is cosmetic, a panic is not.
fn local_offset_seconds(seconds: i64) -> i64 {
    #[repr(C)]
    struct Tm {
        tm_sec: i32,
        tm_min: i32,
        tm_hour: i32,
        tm_mday: i32,
        tm_mon: i32,
        tm_year: i32,
        tm_wday: i32,
        tm_yday: i32,
        tm_isdst: i32,
        tm_gmtoff: i64,
        tm_zone: *const std::ffi::c_char,
    }
    unsafe extern "C" {
        fn localtime_r(timep: *const i64, result: *mut Tm) -> *mut Tm;
    }
    let mut tm = Tm {
        tm_sec: 0,
        tm_min: 0,
        tm_hour: 0,
        tm_mday: 0,
        tm_mon: 0,
        tm_year: 0,
        tm_wday: 0,
        tm_yday: 0,
        tm_isdst: 0,
        tm_gmtoff: 0,
        tm_zone: std::ptr::null(),
    };
    // SAFETY: a stack `Tm` in glibc's x86_64 layout, and `localtime_r` writes
    // only into it. A null return leaves the zeroed struct, whose offset is 0.
    let written = unsafe { localtime_r(&seconds, &mut tm) };
    if written.is_null() { 0 } else { tm.tm_gmtoff }
}

/// Days since 1970-01-01 → `(year, month, day)`, proleptic Gregorian.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

// ---------------------------------------------------------------------------
// The two subcommands: `neezer-transcribe worker` and `neezer-transcribe local`
// ---------------------------------------------------------------------------

/// `neezer-transcribe worker`: the enrolled-client loop, with its control channel.
pub fn worker_main(args: Vec<OsString>) -> ExitCode {
    let usage = "USAGE:\n    neezer-transcribe worker --url <http(s)://hôte[:port]> --model <ggml.bin>\n        [--token-file <fichier>] [--lang auto|xx] [--engine-program <binaire>]\n        [--pcm-program <binaire>] [--work-dir <dossier>] [--once] [--cpu]\n";
    let parsed = match Options::parse(&args) {
        Ok(options) => options,
        Err(message) => return usage_error(&message, usage),
    };
    let Some(url) = parsed.value("--url") else {
        return usage_error("--url est requis", usage);
    };
    let Some(model) = parsed.value("--model") else {
        return usage_error("--model est requis (le fichier GGML du modèle)", usage);
    };
    let token_file = parsed
        .value("--token-file")
        .map(PathBuf::from)
        .unwrap_or_else(crate::worker::token_path);
    let token = match crate::worker::Token::from_file(&token_file) {
        Ok(token) => token,
        Err(failure) => {
            eprintln!("neezer-transcribe : {failure}\n\n{usage}");
            return ExitCode::from(2);
        }
    };
    let client = match Client::new(url, token) {
        Ok(client) => client,
        Err(failure) => {
            eprintln!("neezer-transcribe : {failure}");
            return ExitCode::from(2);
        }
    };
    let program = parsed
        .value("--engine-program")
        .map(PathBuf::from)
        .or_else(current_exe)
        .unwrap_or_else(|| PathBuf::from("neezer-transcribe"));
    let sink: Arc<dyn EventSink> = Arc::new(StdoutSink);
    let work = parsed
        .value("--work-dir")
        .map(PathBuf::from)
        .unwrap_or_else(crate::worker::cache_dir);
    let mut supervisor = Supervisor::new(
        Engine {
            program,
            model: PathBuf::from(model),
            cpu: parsed.flag("--cpu"),
        },
        Arc::new(AudioPcm),
        sink,
        work,
        crate::worker::transcripts_dir(),
    );
    supervisor.client = Some(client);
    supervisor.language = parsed.value("--lang").map(str::to_owned);
    if parsed.flag("--once") {
        supervisor.poll = Duration::from_millis(0);
    }
    let (sender, receiver) = mpsc::channel(8);
    spawn_control_reader(sender);
    let outcome = block_on(supervisor.run_worker(Some(receiver)));
    outcome_code(outcome)
}

/// `neezer-transcribe local`: one episode on this PC, no server.
///
/// With `--audio`, one job runs and the process exits. Without it, the process
/// waits for `{"cmd":"local",…}` lines on stdin — the app's way of using it.
pub fn local_main(args: Vec<OsString>) -> ExitCode {
    let usage = "USAGE:\n    neezer-transcribe local --model <ggml.bin> (--episode <uuid> --audio <fichier> | < stdin)\n        [--lang auto|xx] [--engine-program <binaire>] [--pcm-program <binaire>]\n        [--work-dir <dossier>] [--out-dir <dossier>]\n";
    let parsed = match Options::parse(&args) {
        Ok(options) => options,
        Err(message) => return usage_error(&message, usage),
    };
    let Some(model) = parsed.value("--model") else {
        return usage_error("--model est requis (le fichier GGML du modèle)", usage);
    };
    let program = parsed
        .value("--engine-program")
        .map(PathBuf::from)
        .or_else(current_exe)
        .unwrap_or_else(|| PathBuf::from("neezer-transcribe"));
    let sink: Arc<dyn EventSink> = Arc::new(StdoutSink);
    let mut supervisor = Supervisor::new(
        Engine {
            program,
            model: PathBuf::from(model),
            cpu: parsed.flag("--cpu"),
        },
        Arc::new(AudioPcm),
        sink,
        parsed
            .value("--work-dir")
            .map(PathBuf::from)
            .unwrap_or_else(crate::worker::cache_dir),
        parsed
            .value("--out-dir")
            .map(PathBuf::from)
            .unwrap_or_else(crate::worker::transcripts_dir),
    );
    supervisor.language = parsed.value("--lang").map(str::to_owned);

    let direct = match (parsed.value("--episode"), parsed.value("--audio")) {
        (Some(episode), Some(audio)) => Some((episode.to_owned(), PathBuf::from(audio))),
        (None, None) => None,
        _ => return usage_error("--episode et --audio vont ensemble", usage),
    };
    let code = block_on(async {
        if let Some((episode, audio)) = direct {
            return match supervisor
                .run_local_job(&episode, &audio, parsed.value("--lang"))
                .await
            {
                Outcome::Done => 0,
                Outcome::Fatal => 2,
                Outcome::Idle => 1,
            };
        }
        // The app's mode: one job per command, `idle` between them, exit on EOF.
        let mut stdin = std::io::stdin().lock();
        loop {
            match control::read_command(&mut stdin) {
                Ok(Some(Command::Local {
                    episode_id,
                    audio,
                    language,
                })) => {
                    supervisor
                        .run_local_job(&episode_id, Path::new(&audio), language.as_deref())
                        .await;
                }
                Ok(Some(Command::Stop)) | Ok(None) => return 0,
                Ok(Some(Command::Pause)) => supervisor.halt.pause(true),
                Ok(Some(Command::Resume)) => supervisor.halt.pause(false),
                Err(error) => tracing::warn!("{error}"),
            }
        }
    });
    ExitCode::from(code)
}

/// Run a future on a small runtime: the supervisor is two tasks and some I/O.
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => {
            eprintln!("neezer-transcribe : runtime indisponible ({error})");
            std::process::exit(2);
        }
    }
}

/// Read the control channel on its own thread, one command per line.
fn spawn_control_reader(sender: mpsc::Sender<Command>) {
    std::thread::Builder::new()
        .name("neezer-transcribe-control".to_owned())
        .spawn(move || {
            let mut stdin = std::io::stdin().lock();
            loop {
                match control::read_command(&mut stdin) {
                    Ok(Some(command)) => {
                        if sender.blocking_send(command).is_err() {
                            return;
                        }
                    }
                    Ok(None) => return,
                    Err(error) => tracing::warn!("{error}"),
                }
            }
        })
        .map(|_| ())
        .unwrap_or_else(|error| tracing::warn!("canal de contrôle indisponible : {error}"));
}

/// The process's own path, for the engine and decoder children.
fn current_exe() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// The exit code of a run: 0 done or idle, 2 when the token was refused.
fn outcome_code(outcome: Outcome) -> ExitCode {
    match outcome {
        Outcome::Done | Outcome::Idle => ExitCode::SUCCESS,
        Outcome::Fatal => ExitCode::from(2),
    }
}

fn usage_error(message: &str, usage: &str) -> ExitCode {
    eprintln!("neezer-transcribe : {message}\n\n{usage}");
    ExitCode::from(2)
}

/// `--name value` and `--flag`, the two shapes a subcommand needs.
#[derive(Debug, Default)]
struct Options {
    values: Vec<(String, String)>,
    flags: Vec<String>,
}

impl Options {
    fn parse(args: &[OsString]) -> Result<Options, String> {
        let mut options = Options::default();
        let mut index = 0;
        while index < args.len() {
            let raw = args[index].to_string_lossy().into_owned();
            if !raw.starts_with("--") {
                return Err(format!("argument inattendu : {raw}"));
            }
            let (name, inline) = match raw.split_once('=') {
                Some((name, value)) => (name.to_owned(), Some(value.to_owned())),
                None => (raw, None),
            };
            if let Some(value) = inline {
                options.values.push((name, value));
                index += 1;
                continue;
            }
            let takes_value = !matches!(name.as_str(), "--cpu" | "--once" | "--help" | "-h");
            if takes_value {
                let Some(value) = args.get(index + 1) else {
                    return Err(format!("{name} attend une valeur"));
                };
                options
                    .values
                    .push((name, value.to_string_lossy().into_owned()));
                index += 2;
            } else {
                options.flags.push(name);
                index += 1;
            }
        }
        Ok(options)
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|flag| flag == name)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EngineChild, Halt, Options, civil_from_days, format_local, last_percent, parse_percent,
    };
    use std::ffi::OsString;
    use std::time::Duration;

    /// The Gregorian conversion is a multiplication and a few divisions; it is
    /// pinned against known dates because a report whose `created` is 1970 is
    /// worse than no date at all.
    #[test]
    fn a_day_count_becomes_a_calendar_date() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(1), (1970, 1, 2));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29), "a leap day");
        assert_eq!(civil_from_days(20_520), (2026, 3, 8));
        assert_eq!(civil_from_days(20_517), (2026, 3, 5));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        // A local timestamp is `YYYY-MM-DDTHH:MM:SS`, the server's shape.
        let stamp = format_local(0);
        assert_eq!(stamp.len(), 19, "{stamp}");
        assert_eq!(stamp.as_bytes()[10], b'T');
        assert_eq!(civil_from_days(0).0, 1970);
    }

    /// The progress a heartbeat carries is whisper.cpp's own line, read as
    /// `worker._PROGRESS` reads it; a log with none is None, never 0.
    #[test]
    fn progress_comes_from_the_engines_own_output() {
        let log =
            b"whisper_print_timings: progress =  12 %\nwhisper_print_timings: progress =  87 %\n";
        assert_eq!(last_percent(log), Some(87));
        assert_eq!(last_percent(b"riens"), None);
        assert_eq!(parse_percent(" 42 % of audio"), Some(42));
        assert_eq!(
            parse_percent(" 150 %"),
            Some(100),
            "a percentage is clamped"
        );
        assert_eq!(parse_percent("42"), None);
        assert_eq!(parse_percent(" %"), None);
    }

    /// `--name value`, `--name=value` and the two switches of the subcommands.
    #[test]
    fn a_subcommand_reads_its_own_options() {
        let args: Vec<OsString> = [
            "--url",
            "http://127.0.0.1:5722",
            "--model=/m.bin",
            "--cpu",
            "--once",
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        let options = Options::parse(&args).expect("parsed");
        assert_eq!(options.value("--url"), Some("http://127.0.0.1:5722"));
        assert_eq!(options.value("--model"), Some("/m.bin"));
        assert!(options.flag("--cpu") && options.flag("--once"));
        assert!(!options.flag("--lang"));
        assert!(Options::parse(&[OsString::from("positional")]).is_err());
        assert!(Options::parse(&[OsString::from("--url")]).is_err());
        // A flag is never a value: `--cpu --once` is two flags, not one pair.
        let switches: Vec<OsString> = ["--cpu", "--once"]
            .into_iter()
            .map(OsString::from)
            .collect();
        let options = Options::parse(&switches).expect("parsed");
        assert!(options.flag("--cpu") && options.flag("--once"));
        assert_eq!(Duration::from_millis(super::RESUME_AFTER_MS).as_secs(), 3);
    }

    /// An engine child this test spawns for real: it is in its own process
    /// group, it stops and continues, and it is gone after `terminate`.
    #[test]
    fn an_engine_child_can_be_stopped_and_terminated() {
        let directory = std::env::temp_dir().join(format!("nz-t3-child-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("scratch");
        let script = directory.join("sleepy");
        std::fs::write(&script, "#!/bin/sh\nsleep 30\n").expect("script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
        }
        let engine = super::Engine {
            program: script,
            model: directory.join("model.bin"),
            cpu: false,
        };
        let mut child = EngineChild::spawn(
            &engine,
            &directory.join("a.pcm"),
            &directory.join("a.json"),
            "fr",
            &directory.join("a.err"),
        )
        .expect("spawn");
        std::thread::sleep(Duration::from_millis(200));
        child.pause();
        assert!(child.is_paused());
        // A signal is delivered by the scheduler: read the state until it lands,
        // for at most a second (a `SIGSTOP` is not synchronous with `kill`).
        let letter = |pid: u32| {
            std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .ok()
                .and_then(|stat| {
                    stat.split(')')
                        .nth(1)
                        .and_then(|tail| tail.trim_start().chars().next())
                })
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while letter(child.pid()) != Some('T') && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(letter(child.pid()), Some('T'), "the child is stopped");
        child.resume();
        std::thread::sleep(Duration::from_millis(100));
        assert!(
            child.try_wait().expect("wait").is_none(),
            "it is running again"
        );
        child.terminate();
        assert!(
            matches!(child.try_wait(), Ok(Some(_))),
            "terminate reaps the child"
        );
        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    /// The two pause bits are independent: the server's answer cannot resume
    /// what the app paused (measured as a defect before this test existed).
    #[test]
    fn a_server_answer_does_not_clear_the_apps_pause() {
        let halt = Halt::default();
        halt.pause(true);
        halt.server_pause(false);
        assert!(halt.paused(), "the app's pause stands");
        halt.server_pause(true);
        halt.pause(false);
        assert!(halt.paused(), "the server's pause stands on its own");
        halt.reset();
        assert!(!halt.paused());
    }
}
