//! Client-side whole-track analysis: measure an archived track on THIS computer.
//!
//! `docs/agents/analysis-genre.md` (*Analysis on the client PC*) is the contract,
//! and `webapp/measure` is the implementation of the numbers: the spectral
//! descriptors, the EBU R128 loudness, the bass attacks and the beat tracker are
//! the crate the web app compiles to WebAssembly, linked here natively. The
//! desktop therefore measures with the server's own arithmetic — a second port
//! would be a second answer for the same file — and POSTs numbers only: the
//! server still reconciles the tempo with Deezer's figure and decides the style
//! (`analysis.analyze_from_client`).
//!
//! Three rules shape this module:
//!
//! - **It is background work (rule 1).** The whole run lives on one thread named
//!   `nz-analysis`, reniced to [`NICE`] through `setpriority`, and every network
//!   request carries [`Lane::Bulk`]. Between two tracks — and between two
//!   training epochs — the thread parks while [`Control::yield_now`] is set, which
//!   the screen raises when a track is starting to play; a person waiting for
//!   sound never waits on an analysis.
//! - **It says what it is doing (rule 2).** Progress is one [`Event`] per track
//!   and one at the end, carrying the counts AND the counted causes: a run that
//!   failed four FLACs says why four times, never "4 échecs".
//! - **Nothing blocks the UI thread.** The thread talks to the screen through
//!   `wiring::events`, which is the shell's one cross-thread channel; the decode
//!   and the measurement happen here, never in a paint.
//!
//! The page's `runner.js` is the reference for the loop: ask `/analysis/todo` a
//! page of 20 (`after` is the keyset cursor, so a run cut anywhere resumes with
//! exactly what is unsent), download the ORIGINAL FLAC (a lossy transcode moves
//! the descriptors — measured in the note), measure, POST
//! `/analysis/<uuid>/measures`, keep going. A `409` is a version mismatch between
//! this build's measures and the server's: it ends the run instead of failing
//! every track for the same reason.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use nz_api::{Api, ApiError, Lane, Method, Quality, TrackId};
use nz_model::AnalysisTodo;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::wiring::events::{self, Sender};

/// Seconds of audio the server's spectral pass reads at most
/// (`analysis.MAX_SECONDS`); the page honours it and so do we, which also bounds
/// the memory a decoded track costs.
pub const SPECTRAL_SECONDS: f64 = 600.;
/// Seconds the beat tracker reads (`beats.MAX_SECONDS`).
pub const TEMPO_SECONDS: f64 = 1_200.;
/// A page of `/analysis/todo`, as the page asks (`TODO_PAGE`).
pub const PAGE: u32 = 20;
/// Nice level of the analysis thread: below the UI, above nothing else that
/// matters (the transcription runs at 19).
pub const NICE: i32 = 10;
/// Largest original the runner will spool: a day-long set is not a track, and a
/// hostile `Content-Length` is refused rather than written.
pub const MAX_BYTES: u64 = 1 << 30;

/// The thread's own priority, raised (a larger niceness) without a dependency:
/// `libc` is not in the workspace's pins and the app is Linux-only.
mod sys {
    use std::os::raw::{c_int, c_uint};

    pub const PRIO_PROCESS: c_uint = 0;

    unsafe extern "C" {
        pub fn setpriority(which: c_uint, who: c_uint, prio: c_int) -> c_int;
        pub fn getpriority(which: c_uint, who: c_uint) -> c_int;
    }
}

/// Put the CALLING THREAD at `nice` (Linux niceness is per thread). Returns what
/// `setpriority` answered; a failure is not fatal — the duty cycle below still
/// holds the work back, and the screen shows the run's own progress.
pub fn lower_priority(nice: i32) -> i32 {
    // SAFETY: `setpriority` with `who = 0` acts on the calling thread; it takes
    // no pointer and no lifetime.
    unsafe { sys::setpriority(sys::PRIO_PROCESS, 0, nice) }
}

/// The CALLING THREAD's niceness, as glibc reports it (the raw syscall's
/// `20 - nice` is already converted back by libc).
pub fn current_nice() -> i32 {
    // SAFETY: no pointer, no lifetime; `who = 0` is the calling thread.
    unsafe { sys::getpriority(sys::PRIO_PROCESS, 0) }
}

/// The two flags the screen owns. Both are plain atomics because they are written
/// on the UI thread and read here, with no lock either side ever waits on.
#[derive(Debug, Default)]
pub struct Control {
    cancel: AtomicBool,
    yield_now: AtomicBool,
}

impl Control {
    /// Stop as soon as the track in hand is finished (the lease is never lost:
    /// a measure is one POST, not a resumable job).
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    /// Rule 1: a person waiting for sound wins. The thread parks here until the
    /// flag clears, checking four times a second.
    pub fn set_yield(&self, yield_now: bool) {
        self.yield_now.store(yield_now, Ordering::Release);
    }

    pub fn is_yielding(&self) -> bool {
        self.yield_now.load(Ordering::Acquire)
    }

    fn wait_turn(&self) {
        while self.is_yielding() && !self.is_cancelled() {
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

/// What the screen reads, one message at a time.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The run started: the page's own `total` when the server knows it.
    Started { total: u32 },
    /// One track finished (or failed, with `cause`).
    Track {
        done: u32,
        failed: u32,
        total: u32,
        title: String,
        cause: Option<String>,
    },
    /// The run is over. `causes` is the failure ledger, counted, and `stopped`
    /// says whether the person or a version mismatch ended it early.
    Finished {
        done: u32,
        failed: u32,
        causes: BTreeMap<String, u32>,
        stopped: Option<String>,
    },
}

/// One todo row, as `analysis.todo` sends it (the page's own keys).
#[derive(Clone, Debug, Deserialize)]
pub struct TodoRow {
    pub id: String,
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub duration: u32,
    #[serde(default = "default_rate")]
    pub rate: u32,
    #[serde(default = "default_channels")]
    pub channels: u32,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub need: Need,
}

fn default_rate() -> u32 {
    44_100
}

fn default_channels() -> u32 {
    2
}

/// What a row still lacks. The desktop measures; the vector (ONNX) stays the
/// server's, so `embed` is shown but not carried out here.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct Need {
    #[serde(default)]
    pub measure: bool,
    #[serde(default)]
    pub embed: bool,
}

/// The numbers one track yields, in the server's payload shape
/// (`POST /analysis/<id>/measures`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Measures {
    pub spectral: Spectral,
    pub tempo: Option<Tempo>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spectral {
    pub centroid: f64,
    pub spread: f64,
    pub flatness: f64,
    pub flatness_hi: f64,
    pub entropy: f64,
    pub rolloff: f64,
    pub flux_peak: f64,
    pub lufs: Option<f64>,
    pub lra: Option<f64>,
    pub frames: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tempo {
    pub bpm: f64,
    pub confidence: f64,
    pub salience: f64,
    pub alternation: f64,
    pub attacks: f64,
    pub grid: f64,
}

impl Measures {
    /// The body `analysis_measures` validates field by field, at `version`.
    pub fn payload(&self, version: u32) -> Value {
        let s = &self.spectral;
        json!({
            "version": version,
            "spectral": {
                "centroid": s.centroid,
                "spread": s.spread,
                "flatness": s.flatness,
                "flatness_hi": s.flatness_hi,
                "entropy": s.entropy,
                "rolloff": s.rolloff,
                "flux_peak": s.flux_peak,
                "lufs": s.lufs,
                "lra": s.lra,
                "frames": s.frames,
            },
            "tempo": self.tempo.as_ref().map(|t| json!({
                "bpm": t.bpm,
                "confidence": t.confidence,
                "salience": t.salience,
                "alternation": t.alternation,
                "attacks": t.attacks,
                "grid": t.grid,
            })),
        })
    }
}

/// A running analysis: the control the screen keeps, the channel it reads.
pub struct Runner {
    control: Arc<Control>,
    handle: Option<JoinHandle<()>>,
}

impl Runner {
    /// Start the loop on its own thread. `runtime` is the app's tokio handle: the
    /// HTTP work is async there, the decode is blocking here.
    pub fn start(
        api: Arc<Api>,
        runtime: tokio::runtime::Handle,
        spool_dir: PathBuf,
        control: Arc<Control>,
    ) -> (Runner, events::Receiver<Event>) {
        let (tx, rx) = events::channel::<Event>();
        let mine = Arc::clone(&control);
        let handle = std::thread::Builder::new()
            .name("nz-analysis".into())
            .stack_size(2 * 1024 * 1024)
            .spawn(move || {
                let _ = lower_priority(NICE);
                run(api, runtime, spool_dir, mine, tx);
            })
            .expect("thread d'analyse");
        (
            Runner {
                control,
                handle: Some(handle),
            },
            rx,
        )
    }

    pub fn control(&self) -> Arc<Control> {
        Arc::clone(&self.control)
    }

    /// Stop at the next track boundary (never mid-file: a measure is one POST).
    pub fn cancel(&self) {
        self.control.cancel();
    }

    /// Wait for the thread (a test, or a shutdown). The app drops the runner
    /// instead: a detached thread ends at its next check of `cancel`.
    pub fn join(mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        // A screen that closes asks the run to stop; the thread is not joined
        // here (a paint must not wait on a decode).
        self.control.cancel();
    }
}

fn run(
    api: Arc<Api>,
    runtime: tokio::runtime::Handle,
    spool_dir: PathBuf,
    control: Arc<Control>,
    tx: Sender<Event>,
) {
    let mut done = 0u32;
    let mut failed = 0u32;
    let mut causes: BTreeMap<String, u32> = BTreeMap::new();
    let mut after: Option<String> = None;
    let mut stopped: Option<String> = None;
    let mut total = 0u32;
    let mut version = 0u32;
    let mut first_page = true;

    loop {
        if control.is_cancelled() {
            stopped = Some("arrêt demandé".to_owned());
            break;
        }
        control.wait_turn();
        if control.is_cancelled() {
            stopped = Some("arrêt demandé".to_owned());
            break;
        }

        let path = todo_path(after.as_deref());
        let page = runtime.block_on(api.get::<AnalysisTodo>(Lane::Bulk, &path));
        let page = match page {
            Ok(fresh) => fresh.value,
            Err(error) => {
                stopped = Some(cause_of(&error));
                break;
            }
        };
        if first_page {
            // The server's `total` is the library's todo count; `todo` does not
            // carry it, so the run counts what it has sent and finishes when a
            // page comes back empty.
            version = page.version;
            tx.send(Event::Started { total });
            first_page = false;
        }
        if page.tracks.is_empty() {
            break;
        }
        for raw in &page.tracks {
            if control.is_cancelled() {
                stopped = Some("arrêt demandé".to_owned());
                break;
            }
            control.wait_turn();
            let row: TodoRow = match serde_json::from_value(raw.clone()) {
                Ok(row) => row,
                Err(error) => {
                    failed += 1;
                    *causes
                        .entry(format!("ligne illisible : {error}"))
                        .or_insert(0) += 1;
                    continue;
                }
            };
            if !row.need.measure {
                // Only the vector lags and the vector is the server's work.
                continue;
            }
            total += 1;
            let title = if row.title.is_empty() {
                row.reference.clone()
            } else {
                row.title.clone()
            };
            let result = one_track(&api, &runtime, &spool_dir, &row, version);
            match result {
                Ok(()) => done += 1,
                Err(OneError::Version(server)) => {
                    stopped = Some(format!(
                        "le serveur mesure une autre version ({server}) : mettez l'application à jour"
                    ));
                    tx.send(Event::Track {
                        done,
                        failed,
                        total,
                        title,
                        cause: Some(stopped.clone().unwrap_or_default()),
                    });
                    break;
                }
                Err(OneError::Cause(cause)) => {
                    failed += 1;
                    *causes.entry(cause.clone()).or_insert(0) += 1;
                    tx.send(Event::Track {
                        done,
                        failed,
                        total,
                        title,
                        cause: Some(cause),
                    });
                    continue;
                }
            }
            tx.send(Event::Track {
                done,
                failed,
                total,
                title,
                cause: None,
            });
        }
        if stopped.is_some() {
            break;
        }
        match page.next {
            Some(next) if Some(&next) != after.as_ref() => after = Some(next),
            _ => break,
        }
    }

    tx.send(Event::Finished {
        done,
        failed,
        causes,
        stopped,
    });
}

fn todo_path(after: Option<&str>) -> String {
    match after {
        Some(after) => format!("/analysis/todo?limit={PAGE}&after={}", encode(after)),
        None => format!("/analysis/todo?limit={PAGE}"),
    }
}

enum OneError {
    /// The server's `ANALYSIS_VERSION` is not this build's: every following
    /// track would fail the same way.
    Version(u32),
    Cause(String),
}

fn one_track(
    api: &Api,
    runtime: &tokio::runtime::Handle,
    spool_dir: &Path,
    row: &TodoRow,
    version: u32,
) -> Result<(), OneError> {
    let spool = spool_path(spool_dir, &row.id);
    let result = download(api, runtime, row, &spool)
        .and_then(|()| measure_file(&spool).map_err(OneError::Cause));
    let _ = std::fs::remove_file(&spool);
    let measures = result?;
    let body = measures.payload(version);
    let path = format!("/analysis/{}/measures", encode(&row.id));
    match runtime.block_on(api.send::<Value, Value>(Lane::Bulk, Method::POST, &path, Some(&body))) {
        Ok(_) => Ok(()),
        Err(ApiError::Http { status: 409, .. }) => {
            let server = runtime
                .block_on(api.analysis_todo(None, false))
                .map(|todo| todo.version)
                .unwrap_or(0);
            Err(OneError::Version(server))
        }
        Err(error) => Err(OneError::Cause(cause_of(&error))),
    }
}

/// Stream the archived original into `spool`, on the Bulk lane. The ORIGINAL
/// FLAC, never a transcode: measured in the note, an Opus copy moves the
/// centroid 2–3 % and flips the tempo on 4 files of 14.
fn download(
    api: &Api,
    runtime: &tokio::runtime::Handle,
    row: &TodoRow,
    spool: &Path,
) -> Result<(), OneError> {
    let id = TrackId::from(row.reference.as_str());
    let mut file = File::create(spool)
        .map_err(|error| OneError::Cause(format!("fichier temporaire impossible : {error}")))?;
    let mut written = 0u64;
    runtime
        .block_on(async {
            let mut stream = api
                .stream(Lane::Bulk, &id, Quality::Flac, None, None)
                .await
                .map_err(|error| OneError::Cause(cause_of(&error)))?;
            if stream.content_length.is_some_and(|len| len > MAX_BYTES) {
                return Err(OneError::Cause(format!(
                    "le titre dépasse la taille qu'une mesure accepte ({MAX_BYTES} octets)"
                )));
            }
            while let Some(chunk) = stream
                .chunk()
                .await
                .map_err(|error| OneError::Cause(cause_of(&error)))?
            {
                written += chunk.len() as u64;
                if written > MAX_BYTES {
                    return Err(OneError::Cause(format!(
                        "le titre dépasse la taille qu'une mesure accepte ({MAX_BYTES} octets)"
                    )));
                }
                file.write_all(&chunk)
                    .map_err(|error| OneError::Cause(format!("écriture impossible : {error}")))?;
            }
            Ok(())
        })
        .and_then(|()| {
            file.flush()
                .map_err(|error| OneError::Cause(format!("écriture impossible : {error}")))
        })?;
    if written == 0 {
        return Err(OneError::Cause("le flux est revenu vide".to_owned()));
    }
    Ok(())
}

fn spool_path(dir: &Path, id: &str) -> PathBuf {
    let safe: String = id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    dir.join(format!("{safe}.flac.part"))
}

/// The full measure of one file: `measure::measure` over the SOURCE channels
/// (the server measures the source signal), then the low band and the beat
/// tracker over the 44.1 kHz mono the tracker reads.
pub fn measure_file(path: &Path) -> Result<Measures, String> {
    let decoded = decode(path)?;
    let rate = f64::from(decoded.rate);
    let spectral_frames = (SPECTRAL_SECONDS * rate) as usize;
    let kept: Vec<&[f32]> = decoded
        .channels
        .iter()
        .map(|channel| &channel[..channel.len().min(spectral_frames)])
        .collect();
    let summary =
        measure::measure(&kept, rate).ok_or_else(|| "aucune trame spectrale".to_owned())?;
    let low = measure::lowband::analyse(&kept, rate);
    let tempo = measure::tempo::analyse(&decoded.onsets).map(|tempo| Tempo {
        bpm: tempo.bpm,
        confidence: tempo.confidence,
        salience: tempo.salience,
        alternation: tempo.alternation,
        attacks: low.attacks,
        grid: low.grid(tempo.bpm),
    });
    Ok(Measures {
        spectral: Spectral {
            centroid: summary.centroid,
            spread: summary.spread,
            flatness: summary.flatness,
            flatness_hi: summary.flatness_hi,
            entropy: summary.entropy,
            rolloff: summary.rolloff,
            flux_peak: summary.flux_peak,
            lufs: summary.lufs,
            lra: summary.lra,
            frames: summary.frames as u64,
        },
        tempo,
    })
}

struct Decoded {
    rate: u32,
    /// The source's channels, each capped at `SPECTRAL_SECONDS`.
    channels: Vec<Vec<f32>>,
    /// The beat tracker's 44.1 kHz mono onset function, fed as the file decodes
    /// (the tracker is streamed, so the mono signal is never held whole).
    onsets: measure::tempo::Onsets,
}

/// Decode with symphonia at the file's own rate, exactly like `decode.js`: the
/// descriptors move with a resample, so only the tracker's copy is 44.1 kHz.
fn decode(path: &Path) -> Result<Decoded, String> {
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::codecs::audio::CODEC_ID_NULL_AUDIO;
    use symphonia::core::codecs::audio::well_known::CODEC_ID_OPUS;
    use symphonia::core::codecs::registry::CodecRegistry;
    use symphonia::core::errors::Error as SymphoniaError;
    use symphonia::core::formats::probe::{Hint, Probe};
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let file = File::open(path).map_err(|error| format!("ouverture impossible : {error}"))?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut probe = Probe::new();
    symphonia::default::register_enabled_formats(&mut probe);
    let format = probe
        .probe(
            &Hint::new(),
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|error| format!("format non pris en charge : {error}"))?;
    let track = format
        .default_track(TrackType::Audio)
        .or_else(|| format.first_track(TrackType::Audio))
        .ok_or_else(|| "aucune piste audio".to_owned())?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|params| params.audio())
        .ok_or_else(|| "codec audio inconnu".to_owned())?
        .clone();
    if params.codec == CODEC_ID_NULL_AUDIO {
        return Err("codec audio inconnu".to_owned());
    }
    let rate = params
        .sample_rate
        .ok_or_else(|| "fréquence d'échantillonnage absente".to_owned())?;
    let channels = params
        .channels
        .as_ref()
        .map(|channels| channels.count())
        .unwrap_or(2)
        .max(1);
    let mut registry = CodecRegistry::new();
    symphonia::default::register_enabled_codecs(&mut registry);
    if registry.get_audio_decoder(CODEC_ID_OPUS).is_none() {
        registry.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
    }
    let mut decoder = registry
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|error| format!("codec non pris en charge : {error}"))?;
    let mut reader: Box<dyn symphonia::core::formats::FormatReader> = format;

    let keep = (SPECTRAL_SECONDS * f64::from(rate)) as usize;
    let mut kept: Vec<Vec<f32>> = vec![Vec::new(); channels];
    let mut onsets = measure::tempo::Onsets::new();
    let mut mono = Mono44::new(rate);
    let mut raw: Vec<f32> = Vec::new();
    let mut frame: Vec<f32> = Vec::new();
    let mut fed = 0usize;

    loop {
        let packet = match reader.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(SymphoniaError::IoError(error)) if error.kind() == ErrorKind::Interrupted => {
                continue;
            }
            Err(SymphoniaError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(error) => return Err(format!("lecture impossible : {error}")),
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(SymphoniaError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(error) => return Err(format!("décodage impossible : {error}")),
        };
        let frames = decoded.frames();
        if frames == 0 {
            continue;
        }
        let samples = frames * channels;
        if raw.len() != samples {
            raw.resize(samples, 0.0);
        }
        decoded.copy_to_slice_interleaved::<f32, _>(&mut raw[..samples]);
        // The descriptors: the SOURCE signal, every channel, capped like the
        // page caps it (`caps.spectral`).
        if fed < keep {
            let take = frames.min(keep - fed);
            for (index, channel) in kept.iter_mut().enumerate() {
                channel.reserve(take);
                for f in 0..take {
                    channel.push(raw[f * channels + index]);
                }
            }
            fed += take;
        }
        // The tracker: the mean of the channels, resampled to 44.1 kHz as it
        // arrives, and only the first `TEMPO_SECONDS` of it.
        frame.clear();
        frame.reserve(frames);
        for f in 0..frames {
            let mut sum = 0.0f32;
            for channel in 0..channels {
                sum += raw[f * channels + channel];
            }
            frame.push(sum / channels as f32);
        }
        mono.push(&frame, &mut onsets);
        if mono.past_cap() {
            break;
        }
    }
    mono.finish(&mut onsets);
    if kept.first().is_none_or(|channel| channel.len() < 2048) {
        return Err("aucune trame spectrale".to_owned());
    }
    Ok(Decoded {
        rate,
        channels: kept,
        onsets,
    })
}

/// The tracker's input: channel-mean, 44.1 kHz, at most `TEMPO_SECONDS`.
struct Mono44 {
    /// `None` when the file already is 44.1 kHz: the mean is fed straight in.
    resampler: Option<MonoResampler>,
    fed: usize,
    cap: usize,
}

impl Mono44 {
    fn new(rate: u32) -> Self {
        Self {
            resampler: (rate != 44_100).then(|| MonoResampler::new(rate, 44_100)),
            fed: 0,
            cap: (TEMPO_SECONDS * 44_100.) as usize,
        }
    }

    fn push(&mut self, mono: &[f32], onsets: &mut measure::tempo::Onsets) {
        if self.fed >= self.cap {
            return;
        }
        let remaining = self.cap - self.fed;
        match self.resampler.as_mut() {
            None => {
                let take = mono.len().min(remaining);
                onsets.push(&mono[..take]);
                self.fed += take;
            }
            Some(resampler) => {
                // The cap is in OUTPUT frames; the resampler emits as it goes.
                let before = resampler.emitted;
                resampler.push(mono, onsets);
                self.fed += resampler.emitted - before;
            }
        }
    }

    fn past_cap(&self) -> bool {
        self.fed >= self.cap
    }

    fn finish(&mut self, onsets: &mut measure::tempo::Onsets) {
        if let Some(resampler) = self.resampler.as_mut() {
            let before = resampler.emitted;
            resampler.finish(onsets);
            self.fed += resampler.emitted - before;
        }
    }
}

/// Mono FFT resampling for the beat tracker, the engine's own recipe
/// (`nz-audio` `decode.rs`): 1024-frame chunks, the filter's startup delay
/// dropped, and the tail trimmed to the ratio so a 1.000 s source stays 1.000 s.
struct MonoResampler {
    fft: rubato::Fft<f32>,
    input: Vec<f32>,
    output: Vec<f32>,
    filled: usize,
    skip: usize,
    /// Source frames handed to the resampler, and output frames emitted after
    /// the delay: their ratio is the track's length, which the last partial
    /// chunk must respect (a partial chunk still produces a full FFT block —
    /// the engine measured 960 extra frames on a one-second track).
    fed: usize,
    emitted: usize,
}

impl MonoResampler {
    fn new(from: u32, to: u32) -> Self {
        use rubato::{Fft, FixedSync, Resampler};
        let mut fft = Fft::<f32>::new(from as usize, to as usize, 1024, 1, FixedSync::Input)
            .expect("rééchantillonneur FFT");
        let input = fft.input_frames_next();
        let output = fft.output_frames_max();
        let skip = fft.output_delay();
        fft.reset();
        Self {
            fft,
            input: vec![0.0; input],
            output: vec![0.0; output],
            filled: 0,
            skip,
            fed: 0,
            emitted: 0,
        }
    }

    fn push(&mut self, mono: &[f32], onsets: &mut measure::tempo::Onsets) {
        let chunk = self.input.len();
        self.fed += mono.len();
        let mut offset = 0;
        while offset < mono.len() {
            let take = (chunk - self.filled).min(mono.len() - offset);
            self.input[self.filled..self.filled + take]
                .copy_from_slice(&mono[offset..offset + take]);
            self.filled += take;
            offset += take;
            if self.filled == chunk {
                self.filled = 0;
                self.emit(onsets, None);
            }
        }
    }

    /// The source ended: process the partial chunk, trimmed to the length the
    /// ratio promises, so a 1.000 s source stays 1.000 s.
    fn finish(&mut self, onsets: &mut measure::tempo::Onsets) {
        if self.filled > 0 {
            let filled = self.filled;
            self.filled = 0;
            self.emit(onsets, Some(filled));
        }
    }

    fn emit(&mut self, onsets: &mut measure::tempo::Onsets, partial: Option<usize>) {
        use rubato::Resampler;
        use rubato::audioadapter_buffers::direct::InterleavedSlice;
        let chunk = self.input.len();
        let out = self.output.len();
        let produced = self.fft.output_frames_next().min(out);
        let indexing = rubato::Indexing {
            input_offset: 0,
            output_offset: 0,
            partial_len: partial,
            active_channels_mask: None,
        };
        let Ok(input) = InterleavedSlice::new(&self.input, 1, chunk) else {
            return;
        };
        let Ok(mut output) = InterleavedSlice::new_mut(&mut self.output, 1, out) else {
            return;
        };
        if self
            .fft
            .process_into_buffer(&input, &mut output, Some(&indexing))
            .is_err()
        {
            return;
        }
        let mut frames = match partial {
            Some(_) => {
                let target = (self.fed as f64 * self.fft.resample_ratio()).round() as usize;
                produced.min(target.saturating_sub(self.emitted))
            }
            None => produced,
        };
        let skip = self.skip.min(frames);
        self.skip -= skip;
        frames -= skip;
        self.emitted += frames;
        if frames > 0 {
            onsets.push(&self.output[skip..skip + frames]);
        }
    }
}

/// A failure's cause, as a person reads it: the SPA's `failureCause` vocabulary,
/// never `[object Object]`.
pub fn cause_of(error: &ApiError) -> String {
    match error {
        ApiError::Offline => "le serveur est injoignable".to_owned(),
        ApiError::Unauthorized => "la session a expiré".to_owned(),
        ApiError::Decode(message) => message.clone(),
        ApiError::Provider { code, message, .. } => format!("{code} : {message}"),
        ApiError::Busy { retry_after } => format!("le serveur est occupé ({} s)", retry_after),
        ApiError::Http {
            status,
            message,
            code,
            ..
        } => {
            let message = message.trim();
            let code = code.as_deref().map(str::trim).unwrap_or_default();
            if !message.is_empty() {
                format!("HTTP {status} : {message}")
            } else if !code.is_empty() {
                format!("HTTP {status} : {code}")
            } else {
                format!("HTTP {status}")
            }
        }
        other => other.to_string(),
    }
}

fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Read a spooled original back — `Read` is implemented so a test can hand the
/// decoder bytes it built itself.
pub fn read_spool(path: &Path) -> Result<Vec<u8>, String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

/// Time one page of the loop, printed by the acceptance bench: what a track
/// costs on this machine. Kept next to the loop so the number cannot drift from
/// the code that produced it.
pub fn timed<T>(work: impl FnOnce() -> T) -> (T, Duration) {
    let start = Instant::now();
    let value = work();
    (value, start.elapsed())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_payload_is_the_servers_shape() {
        let measures = Measures {
            spectral: Spectral {
                centroid: 1.5,
                spread: 2.0,
                flatness: 0.25,
                flatness_hi: 0.3,
                entropy: 1.1,
                rolloff: 9_000.,
                flux_peak: 1.2,
                lufs: Some(-9.5),
                lra: Some(6.0),
                frames: 12_345,
            },
            tempo: Some(Tempo {
                bpm: 150.,
                confidence: 0.8,
                salience: 1.4,
                alternation: 0.1,
                attacks: 120.,
                grid: 0.9,
            }),
        };
        let body = measures.payload(5);
        assert_eq!(body["version"], 5);
        assert_eq!(body["spectral"]["frames"], 12_345);
        assert_eq!(body["spectral"]["lra"], 6.0);
        assert_eq!(body["tempo"]["bpm"], 150.0);
        // A file with no beat sends `null`, which the server accepts as "no
        // tempo" rather than inventing one.
        let empty = Measures::default().payload(5);
        assert_eq!(empty["tempo"], Value::Null);
    }

    #[test]
    fn the_todo_path_is_the_pages_keyset_cursor() {
        assert_eq!(todo_path(None), "/analysis/todo?limit=20");
        assert_eq!(
            todo_path(Some("89e80170-adc3-5d61-9266-df93e520230f")),
            "/analysis/todo?limit=20&after=89e80170-adc3-5d61-9266-df93e520230f"
        );
    }

    /// The rule-1 seam, as an assertion: while the screen says a person is
    /// waiting for sound, the thread does not advance; clearing the flag lets it
    /// finish at once.
    #[test]
    fn the_run_parks_while_a_person_waits_for_sound() {
        let control = Arc::new(Control::default());
        control.set_yield(true);
        let started = Instant::now();
        let mine = Arc::clone(&control);
        let waiter = std::thread::spawn(move || {
            mine.wait_turn();
            Instant::now()
        });
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            started.elapsed() >= Duration::from_millis(300),
            "the waiter is parked, not spinning through"
        );
        control.set_yield(false);
        let ended = waiter.join().expect("waiter");
        assert!(ended >= started + Duration::from_millis(300));
    }

    /// 48 kHz mono → the tracker's 44.1 kHz: the length follows the ratio, and
    /// the filter's delay is dropped rather than left as leading silence (the
    /// engine measured 246 frames lost to the opposite mistake).
    #[test]
    fn the_trackers_resampler_keeps_the_duration() {
        let mut resampler = MonoResampler::new(48_000, 44_100);
        let mut onsets = measure::tempo::Onsets::new();
        let source: Vec<f32> = (0..48_000)
            .map(|i| (std::f64::consts::TAU * 440.0 * f64::from(i) / 48_000.0).sin() as f32)
            .collect();
        resampler.push(&source, &mut onsets);
        resampler.finish(&mut onsets);
        let frames = resampler.emitted;
        // The FFT's output delay is DROPPED (147 frames at 1024/hop, 3.3 ms),
        // and the tracker reads onsets, not a duration: the tail's last
        // milliseconds are silence the beat grid does not need. The criterion is
        // therefore 5 ms, not sample-exact (the engine, which must keep the
        // duration, compensates by feeding the filter's delay back in).
        let expected = 44_100.0;
        assert!(
            (frames as f64 - expected).abs() <= 220.0,
            "one second stays one second within 5 ms: {frames} frames"
        );
        assert!(!onsets.df.is_empty(), "the tracker received its onsets");
    }

    /// Rule 1's CPU half: the analysis thread is reniced, and it can say so.
    /// Run on a THREAD of its own: niceness is per thread, and a test worker
    /// left at nice 10 would slow every test after it (raising it back needs a
    /// privilege we do not ask for).
    #[test]
    fn the_analysis_thread_runs_below_the_ui() {
        let nice = std::thread::spawn(|| {
            let before = current_nice();
            assert_eq!(
                lower_priority(NICE),
                0,
                "setpriority accepts a lower priority"
            );
            let after = current_nice();
            (before, after)
        })
        .join()
        .expect("thread");
        let (before, after) = nice;
        assert_eq!(
            after, NICE,
            "the analysis thread runs at {NICE} (was {before})"
        );
    }

    /// A cause is a sentence a person can act on, never a code alone.
    #[test]
    fn every_failure_names_its_cause() {
        assert_eq!(cause_of(&ApiError::Offline), "le serveur est injoignable");
        assert_eq!(
            cause_of(&ApiError::Http {
                status: 404,
                message: "unknown track".to_owned(),
                code: None
            }),
            "HTTP 404 : unknown track"
        );
    }
}

// -- training the head, on this PC --------------------------------------------
//
// `webapp/src/lib/genre/core.js` + `health.js` are the reference, and this is
// the same crate (`webapp/trainer`) linked natively: the folds, the bags, the
// projection and its retry, the temperature and the confusion matrix are the
// page's arithmetic, so a head trained here and a head trained in the browser
// from the same rows agree. What this module adds is the loop around it — the
// pre-flight cleanup (`health.js#prepare`) and one thread that yields.
//
// The cleanup is not optional. A genre with five examples, the same recording
// tagged twice, one artist making up most of a genre: each would bias the head
// or flatter its score, and the studio's own measurements (analysis-genre.md)
// are the reason the thresholds are these numbers.
pub mod training {
    use std::collections::{BTreeMap, BTreeSet, HashMap};
    use std::sync::Arc;

    use serde::Deserialize;
    use serde_json::{Value, json};

    use super::{Control, NICE, lower_priority};
    use crate::wiring::events::{self, Sender};

    /// Under this many tracks a genre is left out of training (`health.js`):
    /// five folds cannot each hold one out and still train on four.
    pub const MIN_EXAMPLES: u32 = 5;
    /// One artist may supply at most this share of a genre's examples, and never
    /// fewer than [`ARTIST_CAP_MIN`].
    pub const ARTIST_SHARE_MAX: f64 = 0.4;
    pub const ARTIST_CAP_MIN: u32 = 5;
    /// Balanced accuracy under this and the model is not worth serving.
    pub const POOR_BALANCED: f64 = 0.6;
    /// Training-fit minus held-out: memorising, measured at 0.6+ on labels
    /// unrelated to the audio.
    pub const GAP_WATCH: f64 = 0.15;
    pub const GAP_BAD: f64 = 0.3;
    /// The v2 prefix and the v3 vector widths — one matrix has ONE width.
    pub const V2_DIM: usize = 2_560;
    pub const V3_DIM: usize = 3_840;

    /// One row of `/genre/labelled`, as the trainer needs it.
    #[derive(Clone, Debug, Default, Deserialize)]
    pub struct Labelled {
        #[serde(default)]
        pub id: String,
        #[serde(default)]
        pub title: String,
        #[serde(default)]
        pub artist: String,
        #[serde(default)]
        pub artist_id: Option<String>,
        #[serde(default)]
        pub tag: Option<LabelledTag>,
    }

    #[derive(Clone, Debug, Default, Deserialize)]
    pub struct LabelledTag {
        #[serde(default)]
        pub name: String,
    }

    /// How hard to train. `Auto` is the studio's default: the linear head, and a
    /// 256-wide network only when the data supports it and the network clearly
    /// wins held out.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum Mode {
        #[default]
        Auto,
        Linear,
        Deep,
        Deep2,
    }

    impl Mode {
        pub fn label(self) -> &'static str {
            match self {
                Mode::Auto => "Automatique",
                Mode::Linear => "Rapide",
                Mode::Deep => "Approfondi",
                Mode::Deep2 => "Maximum",
            }
        }
    }

    /// What the cleanup did, in sentences a person reads (`health.js#prepare`).
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct Cleaned {
        pub done: Vec<String>,
        pub excluded: Vec<String>,
        pub kept: usize,
        pub conflicts: usize,
    }

    /// The matrix one run trains on, built once so every mode sees the same rows.
    #[derive(Clone, Debug)]
    pub struct Prepared {
        pub x: Vec<f32>,
        pub y: Vec<i32>,
        pub groups: Vec<i32>,
        pub n: usize,
        pub d: usize,
        pub labels: Vec<String>,
        pub grouped: bool,
        pub cleaned: Cleaned,
        /// How many rows had every vector (a missing vector is a row left out,
        /// never a padded one).
        pub skipped: usize,
    }

    /// A cheap identity for a vector (`health.js#vectorKey`): identical
    /// recordings embed to the same numbers, and FNV-1a over the first 96
    /// values at 1/500 resolution is enough to bring them together.
    pub fn vector_key(vector: &[f32]) -> u32 {
        let mut h: u32 = 0x811c9dc5;
        for value in vector.iter().take(96) {
            let q = (f64::from(*value) * 500.0).round() as i64;
            h ^= (q & 0xffff) as u32;
            h = h.wrapping_mul(0x01000193);
            h ^= ((q >> 16) & 0xffff) as u32;
            h = h.wrapping_mul(0x01000193);
        }
        h
    }

    /// `health.js#prepare`: duplicates, a recording tagged two ways, an artist
    /// making up most of a genre, and the width of the matrix.
    ///
    /// `labelled` is newest first (the server's order), `vectors` keyed by the
    /// track's row ref. The returned rows are the ones that survive, in the
    /// clean-up's order.
    pub fn prepare(
        labelled: &[Labelled],
        vectors: &HashMap<String, Vec<f32>>,
    ) -> Result<Prepared, String> {
        // Only what the server can train on: a tag and a vector.
        let rows: Vec<&Labelled> = labelled
            .iter()
            .filter(|row| {
                row.tag
                    .as_ref()
                    .is_some_and(|tag| !tag.name.trim().is_empty())
                    && vectors.contains_key(&row.id)
            })
            .collect();
        let skipped = labelled.len() - rows.len();
        if rows.len() < 4 {
            return Err(
                "il faut au moins deux genres de cinq titres, avec une empreinte".to_owned(),
            );
        }
        // 1. The same recording twice: one copy kept (the newest tag), and when
        //    the copies disagree, neither — the head cannot learn both.
        let mut by_key: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        for (index, row) in rows.iter().enumerate() {
            by_key
                .entry(vector_key(&vectors[&row.id]))
                .or_default()
                .push(index);
        }
        let mut drop: BTreeSet<usize> = BTreeSet::new();
        let mut cleaned = Cleaned::default();
        let mut dupes = 0;
        for indexes in by_key.values() {
            if indexes.len() < 2 {
                continue;
            }
            let labels: BTreeSet<&str> = indexes
                .iter()
                .filter_map(|index| rows[*index].tag.as_ref().map(|tag| tag.name.as_str()))
                .collect();
            if labels.len() > 1 {
                cleaned.conflicts += 1;
                for index in indexes {
                    drop.insert(*index);
                }
            } else {
                for index in indexes.iter().skip(1) {
                    drop.insert(*index);
                }
                dupes += indexes.len() - 1;
            }
        }
        if dupes > 0 {
            cleaned.done.push(format!(
                "{dupes} doublon{} écarté{} (le même enregistrement étiqueté deux fois)",
                if dupes > 1 { "s" } else { "" },
                if dupes > 1 { "s" } else { "" }
            ));
        }
        if cleaned.conflicts > 0 {
            cleaned.done.push(format!(
                "{} enregistrement(s) étiqueté(s) de deux façons : écarté(s)",
                cleaned.conflicts
            ));
        }
        // 2. One artist making up most of a genre: its newest tracks are kept up
        //    to the cap, the rest sit this run out.
        let mut live: Vec<usize> = (0..rows.len())
            .filter(|index| !drop.contains(index))
            .collect();
        let mut per_label: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for index in &live {
            let label = rows[*index]
                .tag
                .as_ref()
                .map(|tag| tag.name.clone())
                .unwrap_or_default();
            per_label.entry(label).or_default().push(*index);
        }
        for (label, indexes) in &per_label {
            let cap = ((indexes.len() as f64) * ARTIST_SHARE_MAX).ceil() as u32;
            let cap = cap.max(ARTIST_CAP_MIN);
            let mut seen: BTreeMap<&str, u32> = BTreeMap::new();
            let mut dropped = 0;
            for index in indexes {
                let Some(artist) = rows[*index].artist_id.as_deref() else {
                    continue;
                };
                let count = seen.entry(artist).or_insert(0);
                *count += 1;
                if *count > cap {
                    drop.insert(*index);
                    dropped += 1;
                }
            }
            if dropped > 0 {
                cleaned.done.push(format!(
                    "{label} : {dropped} titre(s) écarté(s), un seul artiste en faisait la \
                     majorité"
                ));
            }
        }
        live = (0..rows.len())
            .filter(|index| !drop.contains(index))
            .collect();
        // 3. A genre too thin to learn or test is left out (its tag still
        //    applies; the head is just not asked to predict it yet).
        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        for index in &live {
            if let Some(tag) = &rows[*index].tag {
                *counts.entry(tag.name.clone()).or_insert(0) += 1;
            }
        }
        for (label, count) in &counts {
            if *count < MIN_EXAMPLES {
                cleaned.excluded.push(label.clone());
            }
        }
        let keep: Vec<usize> =
            live.into_iter()
                .filter(|index| {
                    rows[*index].tag.as_ref().is_some_and(|tag| {
                        counts.get(&tag.name).copied().unwrap_or(0) >= MIN_EXAMPLES
                    })
                })
                .collect();
        if !cleaned.excluded.is_empty() {
            cleaned.done.push(format!(
                "{} genre(s) sous {MIN_EXAMPLES} titres : laissé(s) de côté pour cette fois",
                cleaned.excluded.len()
            ));
        }
        cleaned.kept = keep.len();
        let mut labels: Vec<String> = keep
            .iter()
            .filter_map(|index| rows[*index].tag.as_ref().map(|tag| tag.name.clone()))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        labels.sort();
        if labels.len() < 2 {
            return Err(format!(
                "il faut au moins deux genres de {MIN_EXAMPLES} titres ou plus{}",
                if cleaned.excluded.is_empty() {
                    " — étiquetez encore un peu".to_owned()
                } else {
                    format!(" (trop peu pour {})", cleaned.excluded.join(", "))
                }
            ));
        }
        // 4. One width: the loud block (v3) only when EVERY row has it.
        let older = keep
            .iter()
            .filter(|index| vectors[&rows[**index].id].len() < V3_DIM)
            .count();
        let d = if older > 0 { V2_DIM } else { V3_DIM };
        let mut x = Vec::with_capacity(keep.len() * d);
        let mut y = Vec::with_capacity(keep.len());
        let mut groups = Vec::with_capacity(keep.len());
        let mut artists: BTreeMap<&str, i32> = BTreeMap::new();
        for index in &keep {
            let row = rows[*index];
            let vector = &vectors[&row.id];
            if vector.len() < d {
                return Err("une empreinte est plus courte que la largeur commune".to_owned());
            }
            x.extend_from_slice(&vector[..d]);
            let label = row
                .tag
                .as_ref()
                .map(|tag| tag.name.as_str())
                .unwrap_or_default();
            y.push(labels.iter().position(|name| name == label).unwrap_or(0) as i32);
            let group = match row.artist_id.as_deref() {
                Some(artist) => {
                    let next = artists.len() as i32;
                    *artists.entry(artist).or_insert(next)
                }
                None => -1,
            };
            groups.push(group);
        }
        // Whole artists are held out whenever most genres have two of them:
        // the score then says whether the head learnt the GENRE.
        let grouped = can_group(&keep, &rows);
        Ok(Prepared {
            x,
            y,
            groups,
            n: keep.len(),
            d,
            labels,
            grouped,
            cleaned,
            skipped,
        })
    }

    fn can_group(keep: &[usize], rows: &[&Labelled]) -> bool {
        let mut per_label: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for index in keep {
            if let (Some(tag), Some(artist)) =
                (rows[*index].tag.as_ref(), rows[*index].artist_id.as_ref())
            {
                per_label
                    .entry(tag.name.clone())
                    .or_default()
                    .insert(artist.clone());
            }
        }
        if per_label.is_empty() {
            return false;
        }
        let with_two = per_label.values().filter(|set| set.len() >= 2).count();
        with_two * 2 >= per_label.len()
    }

    /// What the run produced.
    #[derive(Clone, Debug)]
    pub struct Outcome {
        pub kind: &'static str,
        pub hidden: u32,
        pub hidden2: u32,
        pub labels: Vec<String>,
        pub dim: usize,
        pub layers: Vec<Layer>,
        pub metrics: Value,
        pub health: Health,
    }

    #[derive(Clone, Debug, Default)]
    pub struct Layer {
        pub rows: usize,
        pub cols: usize,
        pub w: Vec<f32>,
        pub b: Vec<f32>,
    }

    /// The one verdict plus at most three things worth knowing
    /// (`health.js#assess`).
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct Health {
        /// `"good"`, `"watch"`, `"poor"`.
        pub level: &'static str,
        pub remarks: Vec<String>,
    }

    /// What the training thread reports.
    #[derive(Clone, Debug)]
    pub enum Event {
        /// `("réduction", 0.2)` — the stage, and its share of the run.
        Progress {
            stage: &'static str,
            fraction: f64,
        },
        Done(Box<Outcome>),
        Failed(String),
    }

    /// The head as `PUT /genre/model` takes it: base64 float16, one flat buffer
    /// in the order the server reads back (W then b, layer by layer), and the
    /// metrics beside it.
    pub fn payload(outcome: &Outcome, families: &BTreeMap<String, String>) -> Value {
        let mut halves: Vec<u16> = Vec::new();
        for layer in &outcome.layers {
            for value in &layer.w {
                halves.push(f32_to_f16(*value));
            }
            for value in &layer.b {
                halves.push(f32_to_f16(*value));
            }
        }
        let mut bytes = Vec::with_capacity(halves.len() * 2);
        for half in halves {
            bytes.extend_from_slice(&half.to_le_bytes());
        }
        let mut metrics = outcome.metrics.clone();
        if !families.is_empty() {
            metrics["families"] = json!(families);
        }
        json!({
            "labels": outcome.labels,
            "weights": base64(&bytes),
            "dim": outcome.dim,
            "kind": outcome.kind,
            "hidden": outcome.hidden,
            "metrics": metrics,
        })
    }

    /// `wire.js#f32ToF16`, truncating like the page (not round-to-nearest): the
    /// stored bytes are the page's, so a head trained here and one trained in
    /// the browser serialise identically.
    pub fn f32_to_f16(value: f32) -> u16 {
        let bits = value.to_bits();
        let sign = ((bits >> 16) & 0x8000) as u16;
        let exp = (((bits >> 23) & 0xff) as i32) - 127 + 15;
        let mut mant = bits & 0x7f_ffff;
        if exp <= 0 {
            return sign;
        }
        if exp >= 0x1f {
            return sign | 0x7c00;
        }
        mant >>= 13;
        sign | ((exp as u16) << 10) | (mant as u16)
    }

    /// Standard base64 with padding; the workspace has no base64 dependency and
    /// this is the only place one would be used.
    pub fn base64(bytes: &[u8]) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
            let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
            let triple = (b0 << 16) | (b1 << 8) | b2;
            out.push(TABLE[(triple >> 18) as usize & 0x3f] as char);
            out.push(TABLE[(triple >> 12) as usize & 0x3f] as char);
            out.push(if chunk.len() > 1 {
                TABLE[(triple >> 6) as usize & 0x3f] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                TABLE[triple as usize & 0x3f] as char
            } else {
                '='
            });
        }
        out
    }

    /// `wire.js#decodeEmbedding`: base64 float16 → f32, for the vectors the
    /// server hands back. An unreadable blob is an EMPTY vector — a row left
    /// out, never a padded one.
    pub fn decode_embedding(text: &str) -> Vec<f32> {
        let Some(bytes) = unbase64(text) else {
            return Vec::new();
        };
        let (pairs, _) = bytes.as_chunks::<2>();
        pairs
            .iter()
            .map(|pair| f16_to_f32(u16::from_le_bytes(*pair)))
            .collect()
    }

    fn unbase64(text: &str) -> Option<Vec<u8>> {
        let mut lookup = [255u8; 256];
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        for (index, byte) in TABLE.iter().enumerate() {
            lookup[*byte as usize] = index as u8;
        }
        let mut out = Vec::with_capacity(text.len() / 4 * 3);
        let mut acc = 0u32;
        let mut bits = 0u32;
        for byte in text.bytes() {
            if byte == b'=' {
                break;
            }
            let value = lookup[byte as usize];
            if value == 255 {
                return None;
            }
            acc = (acc << 6) | u32::from(value);
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((acc >> bits) as u8);
            }
        }
        Some(out)
    }

    pub fn f16_to_f32(half: u16) -> f32 {
        let sign = if half & 0x8000 != 0 { -1.0 } else { 1.0 };
        let exp = (half >> 10) & 0x1f;
        let mant = half & 0x3ff;
        if exp == 0 {
            sign * (mant as f32) * 2f32.powi(-24)
        } else if exp == 0x1f {
            if mant != 0 {
                f32::NAN
            } else {
                sign * f32::INFINITY
            }
        } else {
            sign * ((mant + 1024) as f32) * 2f32.powi(i32::from(exp) - 25)
        }
    }

    /// Start a training run on its own thread: renices itself, parks while
    /// [`Control::is_yielding`] and reports progress at most every 100 ms.
    pub fn start(
        prepared: Prepared,
        mode: Mode,
        pool: Option<Vec<f32>>,
        control: Arc<Control>,
    ) -> (std::thread::JoinHandle<()>, events::Receiver<Event>) {
        let (tx, rx) = events::channel::<Event>();
        let handle = std::thread::Builder::new()
            .name("nz-genre-train".into())
            .stack_size(4 * 1024 * 1024)
            .spawn(move || {
                let _ = lower_priority(NICE);
                train(prepared, mode, pool, control, tx);
            })
            .expect("thread d'entraînement");
        (handle, rx)
    }

    fn train(
        prepared: Prepared,
        mode: Mode,
        pool: Option<Vec<f32>>,
        control: Arc<Control>,
        tx: Sender<Event>,
    ) {
        let classes = prepared.labels.len();
        tx.send(Event::Progress {
            stage: "préparation",
            fraction: 0.1,
        });
        let width = prepared.d;
        // The pool (untagged rows with a vector) is appended as class -1; the
        // trainer only learns from it when `pseudo` is set (self-training).
        let pool = pool.unwrap_or_default();
        let pool_n = pool.len().checked_div(width).unwrap_or(0);
        let data_for = || {
            let n = prepared.n + pool_n;
            let mut data = trainer::train::Data::new(n, width, classes);
            data.x = prepared.x.clone();
            data.x.extend_from_slice(&pool[..pool_n * width]);
            data.y = prepared.y.clone();
            data.y.extend(std::iter::repeat_n(-1, pool_n));
            data.groups = prepared.groups.clone();
            data.groups.extend(std::iter::repeat_n(-1, pool_n));
            data
        };
        let mut opts = trainer::train::Opts::linear();
        opts.grouped = prepared.grouped;
        opts.seed = 1;
        if pool_n > 0 {
            opts.pseudo = 0.9;
        }
        match mode {
            Mode::Linear => {}
            Mode::Deep | Mode::Deep2 | Mode::Auto => {
                opts = trainer::train::Opts::deep();
                opts.grouped = prepared.grouped;
                opts.seed = 1;
                if pool_n > 0 {
                    opts.pseudo = 0.9;
                }
                if mode == Mode::Deep2 {
                    opts.hidden = 512;
                    opts.hidden2 = 512;
                    opts.epochs = 180;
                }
            }
        }
        let mut first = match run_job(data_for(), opts, &control) {
            Ok(outcome) => outcome,
            Err(error) => {
                tx.send(Event::Failed(error));
                return;
            }
        };
        tx.send(Event::Progress {
            stage: "évaluation",
            fraction: 0.7,
        });
        // `Auto`: a 256-wide network only where the data supports it (ten tracks
        // a genre) and only when it wins two points held out. Otherwise the
        // linear head ships — its tasks ran first, same seed, same folds.
        if mode == Mode::Auto {
            let per_class = prepared.n as f64 / classes.max(1) as f64;
            if per_class >= 10.0 && first.outcome.balanced < 0.97 {
                let mut deep = trainer::train::Opts::deep();
                deep.grouped = prepared.grouped;
                deep.seed = 1;
                if pool_n > 0 {
                    deep.pseudo = 0.9;
                }
                tx.send(Event::Progress {
                    stage: "réseau",
                    fraction: 0.4,
                });
                if let Ok(second) = run_job(data_for(), deep, &control)
                    && second.outcome.balanced > first.outcome.balanced + 0.02
                {
                    first = second;
                }
            }
        }
        let health = assess(&first.outcome, prepared.cleaned.kept);
        // A linear head is one layer (classes × dim); a stack is
        // (hidden × dim), (classes × hidden) [, (classes × hidden2)].
        let kind = match first.outcome.kind {
            2 => "mlp2",
            1 => "mlp",
            _ => "linear",
        };
        let hidden = if kind == "linear" {
            0
        } else {
            first
                .outcome
                .layers
                .first()
                .map_or(0, |layer| layer.rows as u32)
        };
        let hidden2 = if kind == "mlp2" {
            first
                .outcome
                .layers
                .get(1)
                .map_or(0, |layer| layer.rows as u32)
        } else {
            0
        };
        let outcome = Outcome {
            kind,
            hidden,
            hidden2,
            labels: prepared.labels.clone(),
            dim: width,
            layers: first
                .outcome
                .layers
                .iter()
                .map(|layer| Layer {
                    rows: layer.rows,
                    cols: layer.cols,
                    w: layer.w.clone(),
                    b: layer.b.clone(),
                })
                .collect(),
            metrics: metrics(&first.outcome, &prepared, classes),
            health,
        };
        tx.send(Event::Done(Box::new(outcome)));
    }

    struct Finished {
        outcome: trainer::train::Outcome,
    }

    fn run_job(
        data: trainer::train::Data,
        opts: trainer::train::Opts,
        control: &Control,
    ) -> Result<Finished, String> {
        let mut job = trainer::train::Job::new(data, opts);
        job.start().map_err(|error| match error {
            trainer::train::StartError::TooFewLinear => {
                "pas assez de titres étiquetés pour une tête linéaire".to_owned()
            }
            trainer::train::StartError::TooFewDeep => {
                "le réseau demande trois titres par genre".to_owned()
            }
            trainer::train::StartError::TooFewClasses => "il faut au moins deux genres".to_owned(),
            trainer::train::StartError::BadData => {
                "les données ne sont pas utilisables (une valeur non finie ?)".to_owned()
            }
        })?;
        let mut progress = 0.0;
        while progress < 1.0 {
            if control.is_cancelled() {
                return Err("entraînement interrompu".to_owned());
            }
            control.wait_turn();
            progress = job.step(1);
        }
        job.outcome
            .take()
            .map(|outcome| Finished { outcome })
            .ok_or_else(|| "l'entraînement n'a rien produit".to_owned())
    }

    fn metrics(outcome: &trainer::train::Outcome, prepared: &Prepared, classes: usize) -> Value {
        let examples = prepared.n;
        let mut per_class = Vec::with_capacity(classes);
        for (index, label) in prepared.labels.iter().enumerate() {
            let mut seen = 0i64;
            let mut predicted = 0i64;
            let mut hit = 0i64;
            for row in 0..classes {
                for column in 0..classes {
                    let count =
                        i64::from(*outcome.confusion.get(row * classes + column).unwrap_or(&0));
                    if row == index {
                        seen += count;
                    }
                    if column == index {
                        predicted += count;
                    }
                    if row == index && column == index {
                        hit += count;
                    }
                }
            }
            per_class.push(json!({
                "label": label,
                "examples": prepared
                    .y
                    .iter()
                    .filter(|value| **value == index as i32)
                    .count(),
                "recall": if seen > 0 { round3(hit as f64 / seen as f64) } else { 0.0 },
                "precision": if predicted > 0 { round3(hit as f64 / predicted as f64) } else { 0.0 },
            }));
        }
        json!({
            "examples": examples,
            "classes": classes,
            "folds": outcome.folds,
            "projected": outcome.projected,
            "accuracy": outcome.accuracy,
            "balanced": outcome.balanced,
            "fitBalanced": outcome.fit_balanced,
            "temperature": outcome.temperature,
            "grouped": prepared.grouped,
            "seed": 1,
            "bagged": outcome.bagged,
            "perClass": per_class,
            "confusion": outcome.confusion,
        })
    }

    fn round3(value: f64) -> f64 {
        (value * 1000.).round_ties_even() / 1000.
    }

    /// `health.js#assess`: ONE verdict and at most three remarks.
    pub fn assess(outcome: &trainer::train::Outcome, examples: usize) -> Health {
        let mut level = "good";
        let mut remarks = Vec::new();
        if outcome.balanced < POOR_BALANCED {
            level = "poor";
        }
        let gap = outcome.fit_balanced - outcome.balanced;
        if gap >= GAP_BAD {
            level = "poor";
            remarks.push(
                "Le modèle apprend ses exemples par cœur : étiquetez plus d'artistes différents."
                    .to_owned(),
            );
        } else if gap >= GAP_WATCH {
            if level == "good" {
                level = "watch";
            }
            remarks.push(
                "Le modèle retient un peu ses exemples : la note sera moins bonne sur un titre \
                 inconnu."
                    .to_owned(),
            );
        }
        if outcome.balanced >= POOR_BALANCED && outcome.balanced < 0.75 {
            if level == "good" {
                level = "watch";
            }
            remarks.push(format!(
                "Précision équilibrée {:.0} % : étiquetez les genres qui se confondent.",
                outcome.balanced * 100.
            ));
        }
        if examples < 40 {
            remarks.push(format!(
                "{examples} exemples seulement : le modèle s'améliore surtout en étiquetant plus."
            ));
        }
        remarks.truncate(3);
        Health { level, remarks }
    }

    /// The `families` map a head ships with (`metrics.families`): each label's
    /// engine family, from the tree. A label with no family votes for nothing.
    pub fn families(
        labels: &[String],
        tree: &crate::widgets::genre_tree::Tree,
    ) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for label in labels {
            if let Some(family) = tree.family_of(label) {
                out.insert(label.clone(), family);
            }
        }
        out
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn row(id: &str, label: &str, artist: &str) -> Labelled {
            Labelled {
                id: id.to_owned(),
                title: format!("Titre {id}"),
                artist: artist.to_owned(),
                artist_id: Some(artist.to_owned()),
                tag: Some(LabelledTag {
                    name: label.to_owned(),
                }),
            }
        }

        /// Every queued event, with no executor: the thread has already joined,
        /// so each `next()` is ready.
        fn drain<T>(rx: &events::Receiver<T>) -> Vec<T> {
            use std::task::{Context, Poll, Waker};

            let mut context = Context::from_waker(Waker::noop());
            let mut out = Vec::new();
            loop {
                let mut next = Box::pin(rx.next());
                match next.as_mut().poll(&mut context) {
                    Poll::Ready(Some(value)) => out.push(value),
                    _ => break,
                }
            }
            out
        }

        fn vector(seed: f32) -> Vec<f32> {
            (0..V3_DIM as u32)
                .map(|index| (index as f32 * 0.01 + seed).sin())
                .collect()
        }

        /// The width is the common one: a v2 row lowers every row to 2560.
        #[test]
        fn one_matrix_has_one_width() {
            let mut rows = Vec::new();
            for index in 0..5 {
                rows.push(row(&format!("h{index}"), "Hardtek", &format!("a{index}")));
                rows.push(row(&format!("j{index}"), "Jazz", &format!("b{index}")));
            }
            let mut vectors = HashMap::new();
            for (index, item) in rows.iter().enumerate() {
                let mut vec = vector(index as f32);
                if item.id == "h0" {
                    vec.truncate(V2_DIM);
                }
                vectors.insert(item.id.clone(), vec);
            }
            let prepared = prepare(&rows, &vectors).expect("prepared");
            assert_eq!(prepared.d, V2_DIM);
            assert_eq!(prepared.labels, ["Hardtek", "Jazz"]);
            assert_eq!(prepared.n, 10);
        }

        #[test]
        fn a_duplicate_recording_is_kept_once_and_a_thin_genre_is_left_out() {
            let mut rows = Vec::new();
            for index in 0..5 {
                rows.push(row(&format!("h{index}"), "Hardtek", &format!("a{index}")));
            }
            for index in 0..6 {
                rows.push(row(&format!("j{index}"), "Jazz", &format!("b{index}")));
            }
            rows.push(row("s0", "Ambient", "c0"));
            rows.push(row("s1", "Ambient", "c1"));
            let mut vectors = HashMap::new();
            for (index, item) in rows.iter().enumerate() {
                vectors.insert(item.id.clone(), vector(index as f32));
            }
            // j1 is the same recording as j0 under another id: one copy goes.
            vectors.insert("j1".to_owned(), vectors["j0"].clone());
            let prepared = prepare(&rows, &vectors).expect("prepared");
            assert_eq!(
                prepared.cleaned.kept, 10,
                "5 Hardtek + 5 Jazz after the dupe"
            );
            assert!(
                prepared
                    .cleaned
                    .done
                    .iter()
                    .any(|line| line.contains("doublon")),
                "{:?}",
                prepared.cleaned.done
            );
            assert!(prepared.cleaned.excluded.contains(&"Ambient".to_owned()));
            assert_eq!(prepared.labels, ["Hardtek", "Jazz"]);
        }

        /// The float16 round trip is the page's, and base64 is standard: the
        /// bytes `PUT /genre/model` receives are the ones the browser sends.
        #[test]
        fn a_head_serialises_to_the_pages_float16_base64() {
            assert_eq!(f32_to_f16(0.0), 0);
            assert_eq!(f32_to_f16(1.0), 0x3c00);
            assert_eq!(f32_to_f16(-2.0), 0xc000);
            assert!((f16_to_f32(0x3c00) - 1.0).abs() < 1e-6);
            assert_eq!(base64(b"Man"), "TWFu");
            assert_eq!(base64(b"Ma"), "TWE=");
            assert_eq!(base64(b"M"), "TQ==");
            // A round trip through the wire form keeps the values the server
            // reads: 0x3c00 is 1.0 in float16.
            let wire = base64(&0x3c00u16.to_le_bytes());
            assert_eq!(decode_embedding(&wire), vec![1.0]);
            assert!(decode_embedding("not base64!").is_empty());
        }

        /// A linear run on two separable genres converges, and the health
        /// verdict is built from its own numbers.
        #[test]
        fn a_separable_head_trains_and_reports_its_health() {
            let mut x = Vec::new();
            let mut y: Vec<i32> = Vec::new();
            let mut groups = Vec::new();
            for class in 0..2u32 {
                for index in 0..20u32 {
                    let mut row = vec![0.0f32; 16];
                    row[class as usize] = 1.0;
                    row[2 + index as usize % 5] = 0.1;
                    x.extend_from_slice(&row);
                    y.push(class as i32);
                    groups.push(index as i32);
                }
            }
            let prepared = Prepared {
                x,
                y,
                groups,
                n: 40,
                d: 16,
                labels: vec!["A".to_owned(), "B".to_owned()],
                grouped: false,
                cleaned: Cleaned::default(),
                skipped: 0,
            };
            let control = Arc::new(Control::default());
            let (handle, rx) = start(prepared, Mode::Linear, None, control);
            handle.join().expect("training thread");
            let mut outcome = None;
            for event in drain(&rx) {
                match event {
                    Event::Done(head) => outcome = Some(head),
                    Event::Failed(error) => panic!("training failed: {error}"),
                    Event::Progress { .. } => {}
                }
            }
            let outcome = outcome.expect("an outcome");
            assert_eq!(outcome.kind, "linear");
            assert_eq!(outcome.layers.len(), 1, "a linear head is one layer");
            assert_eq!(outcome.layers[0].rows, 2);
            assert_eq!(outcome.layers[0].cols, 16);
            assert!(outcome.metrics["balanced"].as_f64().unwrap() > 0.9);
            assert!(outcome.health.level != "poor");
            let payload = payload(&outcome, &BTreeMap::new());
            assert_eq!(payload["dim"], 16);
            assert_eq!(payload["kind"], "linear");
            assert!(payload["weights"].as_str().unwrap().len() > 16);
        }
    }
}

#[cfg(test)]
mod acceptance {
    //! The lot's own measurement: 100 tracks measured on this PC while music
    //! plays, 0 underruns. Gated by `NZ_ANALYSIS_BENCH=1` because it needs a
    //! real output device (PipeWire here) and 100 generated FLACs; CI has
    //! neither, and a bench that skips silently is a bench nobody trusts — so it
    //! PRINTS why it did nothing and the PR quotes the numbers it produced here.

    use std::sync::Arc;
    use std::sync::atomic::AtomicU64;

    use nz_audio::{
        ByteStream, Engine, EngineConfig, EngineEvent, OutputChoice, Quality, SourceError,
        StreamOpener, StreamReady,
    };

    use super::*;

    struct Files(std::collections::HashMap<String, PathBuf>);

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

        fn local(&self, id: &str, _quality: Quality) -> Option<PathBuf> {
            self.0.get(id).cloned()
        }
    }

    fn ffmpeg_flac(path: &std::path::Path, seconds: u32) {
        let status = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i"])
            .arg(format!("sine=frequency=220:duration={seconds}"))
            .args(["-c:a", "flac", "-y"])
            .arg(path)
            .status()
            .expect("ffmpeg");
        assert!(
            status.success(),
            "ffmpeg could not write {}",
            path.display()
        );
    }

    #[test]
    fn a_hundred_measures_run_reniced_and_never_interrupt_playback() {
        if std::env::var("NZ_ANALYSIS_BENCH").as_deref() != Ok("1") {
            println!(
                "acceptance bench skipped: set NZ_ANALYSIS_BENCH=1 on a machine with \
                 PipeWire and ffmpeg"
            );
            return;
        }
        let dir = std::env::temp_dir().join(format!("neezer-d16-bench-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("bench dir");
        // One long file to listen to, and 100 short ones to measure. 20 s each
        // keeps the whole run to a couple of minutes while every file still
        // yields spectral frames and a beat.
        let playing = dir.join("playing.flac");
        ffmpeg_flac(&playing, 90);
        let mut files = std::collections::HashMap::new();
        files.insert("playing".to_owned(), playing);
        for index in 0..100 {
            let path = dir.join(format!("track-{index}.flac"));
            ffmpeg_flac(&path, 20);
            files.insert(format!("t{index}"), path);
        }

        let engine = Engine::new(
            EngineConfig {
                device: OutputChoice::Default,
                spool_dir: dir.join("spool"),
                ..EngineConfig::default()
            },
            Arc::new(Files(files.clone())),
        );
        let events = engine.events();
        engine.load("playing", Quality::Flac, None);
        engine.set_volume(0.05);
        engine.play();
        // Let the device start and the voice fill before any measurement.
        std::thread::sleep(Duration::from_millis(1_500));
        let started = std::time::Instant::now();
        let mut underruns = 0u64;
        let mut device_error = None;
        while let Ok(event) = events.try_recv() {
            match event {
                EngineEvent::Underrun { count } => underruns = underruns.max(count),
                EngineEvent::Error { id: None, message } => device_error = Some(message),
                _ => {}
            }
        }
        if let Some(message) = device_error {
            println!(
                "acceptance bench skipped: no usable output device ({message}); \
                 CI has no PipeWire either"
            );
            return;
        }

        let measured = Arc::new(AtomicU64::new(0));
        let failures = Arc::new(AtomicU64::new(0));
        let control = Arc::new(Control::default());
        let counter = Arc::clone(&measured);
        let failed = Arc::clone(&failures);
        let mine = Arc::clone(&control);
        let paths: Vec<(String, PathBuf)> = (0..100)
            .map(|index| (format!("t{index}"), files[&format!("t{index}")].clone()))
            .collect();
        let worker = std::thread::Builder::new()
            .name("nz-analysis".into())
            .spawn(move || {
                let _ = lower_priority(NICE);
                println!("analysis thread nice={} (asked {NICE})", current_nice());
                assert!(
                    current_nice() >= NICE,
                    "the analysis thread must yield to the UI and the audio"
                );
                for (_, path) in paths {
                    if mine.is_cancelled() {
                        break;
                    }
                    match measure_file(&path) {
                        Ok(measures) => {
                            assert!(measures.spectral.frames > 0);
                            counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                        Err(error) => {
                            println!("measure failed: {error}");
                            failed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                }
            })
            .expect("analysis thread");

        // The music must stay audible throughout: the bench listens to the
        // engine's own events while the 100 files are measured.
        while measured.load(std::sync::atomic::Ordering::Relaxed)
            + failures.load(std::sync::atomic::Ordering::Relaxed)
            < 100
        {
            if started.elapsed() > Duration::from_secs(600) {
                break;
            }
            while let Ok(event) = events.try_recv() {
                if let EngineEvent::Underrun { count } = event {
                    underruns = underruns.max(count);
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        control.cancel();
        worker.join().expect("worker");
        // Drain what the callback reported after the last sample.
        std::thread::sleep(Duration::from_millis(300));
        while let Ok(event) = events.try_recv() {
            if let EngineEvent::Underrun { count } = event {
                underruns = underruns.max(count);
            }
        }
        let done = measured.load(std::sync::atomic::Ordering::Relaxed);
        let failed = failures.load(std::sync::atomic::Ordering::Relaxed);
        let elapsed = started.elapsed();
        println!(
            "100 measures: done={done} failed={failed} in {:.1} s ({:.0} ms/track), \
             underruns={underruns}",
            elapsed.as_secs_f64(),
            elapsed.as_secs_f64() * 1000. / done.max(1) as f64
        );
        // The criterion: the background work yields (rule 1). A machine whose
        // device cannot keep up reports it here rather than hiding it.
        assert_eq!(underruns, 0, "the analysis interrupted the sound");
        assert_eq!(failed, 0, "every file measured");
        assert_eq!(done, 100);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
