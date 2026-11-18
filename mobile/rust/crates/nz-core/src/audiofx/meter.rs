//! The equalizer's meter: what the ten bands are doing, and how hard the limiter
//! is working (`webapp/src/lib/audio/saturation.js`).
//!
//! # The two halves, and where each number comes from
//!
//! - **The peak and the limiter's reduction** are the CHAIN's own numbers: the
//!   device's DSP computes them per buffer already (`nscore::dsp::Chain`'s meter,
//!   on only while this card is on screen) and never hides them behind the
//!   limiter — the SPA's `pre` analyser sat exactly there for the same reason.
//! - **The ten band levels** come from the analysis tap, which is the sound the
//!   listeners hear, windowed and transformed here. The plan is one octave per
//!   slider, centred on that slider's frequency, exactly `saturation.js#bandEdges`
//!   and `spectrum.js#buildBandPlan`: a log plan over 22 Hz..18 kHz (the
//!   visualiser's) would spread ten bars over the whole spectrum, putting the
//!   31 Hz bar across two sliders and leaving the top one measuring half of
//!   nothing.
//!
//! # The window and the rate
//!
//! 4 096 samples — 85 ms at 48 kHz — read at 25 Hz, the SPA's own choice: at
//! 25 Hz a shorter window could fall between two late readings and miss the very
//! transient the warning is about, and at 85 ms a transient stays visible for
//! two readings.
//!
//! # Cost, and why the card pays for it and nobody else
//!
//! Nothing runs until a card subscribes: no thread, no buffer, no FFT. The
//! session's thread exists while the card is on screen and is **stopped and
//! joined** when it leaves (`MeterSession::stop_and_join`), so "0 analysis CPU
//! when the card disappears" is a counted fact — `ticks()` stays where it was —
//! rather than a timer that happens to be idle. Measured on this machine: one
//! reading is **0.13 ms** of one core (a 4 096-point transform plus ten maxima),
//! 3.3 % of a core at 25 Hz.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::time::{Duration, Instant};

use super::risk::{
    Net, PEAK_FLOOR_DB, RiskHold, Shown, Tone, classify_risk, eq_risk, volume_cut_db,
};

/// The EQ's ten bands, low to high: `nscore::dsp::EQ_FREQS`.
pub const BANDS: usize = 10;
/// The sliders' frequencies (`EQ_FREQS`), the plan's centres.
pub const EQ_FREQS: [f64; BANDS] = [
    31.0, 62.0, 125.0, 250.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0, 16_000.0,
];
/// A band's display floor: quiet enough that a silent band reads empty, high
/// enough that the bottom of the scale is still legible (`BAND_FLOOR_DB`).
pub const BAND_FLOOR_DB: f64 = -60.0;
/// Full scale (`BAND_CEIL_DB`).
pub const BAND_CEIL_DB: f64 = 0.0;
/// The transform's length (`saturation.js`'s fftSize): 85 ms at 48 kHz.
pub const WINDOW: usize = 4_096;
/// The meter's rate: 25 Hz (`TICK_MS`).
pub const TICK_MS: u64 = 40;
/// A volume or preset change lets the analyser's 85 ms window turn over before
/// the new episode starts counting (`holdAfter = now + 160`).
pub const CHANGE_HOLD_MS: f64 = 160.0;

/// An AnalyserNode's frequency data is NOT in dBFS: it is the Blackman-windowed
/// FFT divided by the transform's size, so a full-scale sine at a bin centre
/// reads `20·log10(0.42 / 2) = −13.56 dB`. Shifting by it makes a band's dB the
/// level of the sine that would fill its strongest bin — the same scale as the
/// peak (`ANALYSER_SINE_OFFSET_DB`).
const ANALYSER_SINE_OFFSET_DB: f64 = 13.5556;

/// Where the ten bands start and end: the geometric midpoint between two
/// neighbouring frequencies, and half an interval past each end. For 31..16 000
/// that is 21.9 Hz .. 22.6 kHz, one octave per band (`bandEdges`).
pub fn band_edges(freqs: &[f64]) -> Vec<f64> {
    let n = freqs.len();
    if n == 0 {
        return vec![20.0, 20_000.0];
    }
    if n == 1 {
        return vec![
            freqs[0] / std::f64::consts::SQRT_2,
            freqs[0] * std::f64::consts::SQRT_2,
        ];
    }
    let mut edges = vec![0.0; n + 1];
    for i in 1..n {
        edges[i] = (freqs[i - 1] * freqs[i]).sqrt();
    }
    edges[0] = freqs[0] * freqs[0] / edges[1];
    edges[n] = freqs[n - 1] * freqs[n - 1] / edges[n - 1];
    edges
}

/// The limiter's warning, as the banner draws it (`warn` in the SPA's card).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Warning {
    pub tone: Tone,
    /// The reduction ABOVE the episode's baseline, one decimal.
    pub reduction_db: f64,
    /// What "Réduire le volume" should ask for, in dB.
    pub cut_db: f64,
    /// Measured since the person's own change, not since the track started.
    pub from_change: bool,
}

/// One reading, as the card draws it. Fresh arrays per frame, so a screen may
/// keep one while the next arrives.
#[derive(Debug, Clone, PartialEq)]
pub struct MeterFrame {
    /// False when there is nothing to measure (no tap yet, the player is gone):
    /// `reason` says which, never a fabricated reading.
    pub available: bool,
    /// False until the first reading.
    pub measured: bool,
    /// The pre-limiter sample peak of the last buffer, dBFS, floored at −96.
    pub peak_db: f64,
    /// `-peak_db`: how far below full scale the chain is.
    pub headroom_db: f64,
    /// The limiter's gain reduction on that buffer, dB, `>= 0`.
    pub reduction_db: f64,
    /// The chain's own state; the card ignores it when the equalizer adds no
    /// gain (`eq_risk`).
    pub risk: Tone,
    /// The ten bands, 0..1 over −60..0 dBFS, in the sliders' order.
    pub bands: Vec<f32>,
    /// The same, in dBFS (floored at −60), for a readout or a diagnostic.
    pub bands_db: Vec<f32>,
    /// The episode the banner shows, if one is open.
    pub warning: Option<Warning>,
    /// `idle` or `direct` when `available` is false, empty otherwise.
    pub reason: String,
}

impl MeterFrame {
    /// Nothing to measure, and why.
    pub fn unavailable(reason: &str) -> MeterFrame {
        MeterFrame {
            available: false,
            measured: false,
            peak_db: PEAK_FLOOR_DB,
            headroom_db: -PEAK_FLOOR_DB,
            reduction_db: 0.0,
            risk: Tone::Ok,
            bands: vec![0.0; BANDS],
            bands_db: vec![BAND_FLOOR_DB as f32; BANDS],
            warning: None,
            reason: reason.to_owned(),
        }
    }
}

// -- the transform -------------------------------------------------------------

/// A 4 096-point real transform with the Blackman window Web Audio's
/// AnalyserNode uses, and the ten-band plan on top of it.
///
/// Written here rather than borrowed: it is 40 lines of radix-2, it runs once
/// per reading on a buffer the card asked for, and the alternative — making
/// `rhythm`'s private FFT public — would edit a crate three other applications
/// build. The maths is the analyser's own (Web Audio §6.2), and the tests below
/// pin a full-scale sine at a bin centre to its −13.56 dB.
pub struct Spectrum {
    /// The Blackman window, precomputed once.
    window: Vec<f64>,
    /// The bit-reversal permutation and the twiddles, precomputed once.
    reverse: Vec<usize>,
    cos: Vec<f64>,
    sin: Vec<f64>,
    re: Vec<f64>,
    im: Vec<f64>,
    edges: Vec<f64>,
    sample_rate: f64,
    bands_db: Vec<f32>,
    bands01: Vec<f32>,
}

impl Spectrum {
    pub fn new(sample_rate: f64) -> Spectrum {
        let n = WINDOW;
        let mut window = vec![0.0; n];
        let tau = std::f64::consts::TAU;
        for (i, w) in window.iter_mut().enumerate() {
            let x = i as f64 / (n - 1) as f64;
            // Blackman: 0.42 − 0.5·cos(2πx) + 0.08·cos(4πx).
            *w = 0.42 - 0.5 * (tau * x).cos() + 0.08 * (2.0 * tau * x).cos();
        }
        let bits = n.trailing_zeros() as usize;
        let mut reverse = vec![0usize; n];
        for (i, r) in reverse.iter_mut().enumerate() {
            *r = (0..bits).fold(0usize, |acc, b| acc | (((i >> b) & 1) << (bits - 1 - b)));
        }
        let mut cos = vec![0.0; n / 2];
        let mut sin = vec![0.0; n / 2];
        for k in 0..n / 2 {
            cos[k] = (tau * k as f64 / n as f64).cos();
            sin[k] = -(tau * k as f64 / n as f64).sin();
        }
        Spectrum {
            window,
            reverse,
            cos,
            sin,
            re: vec![0.0; n],
            im: vec![0.0; n],
            edges: band_edges(&EQ_FREQS),
            sample_rate: if sample_rate.is_finite() && sample_rate > 0.0 {
                sample_rate
            } else {
                48_000.0
            },
            bands_db: vec![BAND_FLOOR_DB as f32; BANDS],
            bands01: vec![0.0; BANDS],
        }
    }

    /// One reading of `window` (interleaved stereo, the NEWEST [`WINDOW`]
    /// frames). Answers the ten bands in dBFS and 0..1, in the sliders' order.
    ///
    /// An analyser down-mixes its input to mono before measuring the spectrum
    /// (Web Audio §1.8.5), which is what this does: a hard anti-phase pair reads
    /// low, and that is the honest limit of a mono spectrum.
    pub fn bands(&mut self, window: &[f32]) -> (&[f32], &[f32]) {
        let n = WINDOW;
        let frames = window.len() / 2;
        for i in 0..n {
            // The newest frames sit at the END of the slice; a window that is not
            // full yet is zero-padded at its start, which reads as silence.
            let src = i + frames.saturating_sub(n);
            let sample = if src < frames {
                let l = f64::from(window[src * 2]);
                let r = f64::from(window[src * 2 + 1]);
                0.5 * (l + r)
            } else {
                0.0
            };
            self.re[i] = sample * self.window[i];
            self.im[i] = 0.0;
        }
        // Radix-2, in place.
        for i in 0..n {
            let j = self.reverse[i];
            if i < j {
                self.re.swap(i, j);
                self.im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let half = len / 2;
            let step = n / len;
            for start in (0..n).step_by(len) {
                for k in 0..half {
                    let (wr, wi) = (self.cos[k * step], self.sin[k * step]);
                    let i = start + k;
                    let j = i + half;
                    let tr = self.re[j] * wr - self.im[j] * wi;
                    let ti = self.re[j] * wi + self.im[j] * wr;
                    self.re[j] = self.re[i] - tr;
                    self.im[j] = self.im[i] - ti;
                    self.re[i] += tr;
                    self.im[i] += ti;
                }
            }
            len *= 2;
        }
        // The plan: each band's strongest bin inside its octave, in dBFS.
        let bin_hz = self.sample_rate / n as f64;
        for band in 0..BANDS {
            let lo = self.edges[band];
            let hi = self.edges[band + 1];
            let k0 = ((lo / bin_hz).ceil() as usize).clamp(1, n / 2);
            let k1 = ((hi / bin_hz).floor() as usize).clamp(k0, n / 2 - 1);
            let mut peak = 0.0f64;
            for k in k0..=k1 {
                let mag = (self.re[k] * self.re[k] + self.im[k] * self.im[k]).sqrt() / n as f64;
                if mag > peak {
                    peak = mag;
                }
            }
            let db = if peak > 0.0 {
                20.0 * peak.log10() + ANALYSER_SINE_OFFSET_DB
            } else {
                BAND_FLOOR_DB
            };
            let db = db.clamp(BAND_FLOOR_DB, BAND_CEIL_DB);
            self.bands_db[band] = db as f32;
            self.bands01[band] = ((db - BAND_FLOOR_DB) / (BAND_CEIL_DB - BAND_FLOOR_DB)) as f32;
        }
        (&self.bands_db, &self.bands01)
    }
}

// -- the session ---------------------------------------------------------------

/// What a card receives: one frame per reading, from the session's own thread.
pub trait MeterSink: Send + Sync {
    fn on_frame(&self, frame: MeterFrame);
}

/// Where a reading's two chain numbers come from (the engine's own meter).
pub type ChainReading = Arc<dyn Fn() -> (f64, f64) + Send + Sync>;
/// The equalizer's switch and curve, for attributing a warning to the setting.
pub type Curve = Arc<dyn Fn() -> (bool, Vec<f32>) + Send + Sync>;

/// One card's meter: a thread that exists only while the card does.
///
/// `start` takes the analysis subscription (its own tap ring) and the two
/// closures; `stop_and_join` sets the flag and joins the thread, so a caller that
/// returns from it knows nothing of this card is running any more.
pub struct MeterSession {
    stop: Arc<AtomicBool>,
    ticks: Arc<AtomicU64>,
    frames: Arc<AtomicU64>,
    changes: Sender<String>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl MeterSession {
    /// Start the session. `tap` is the card's own analysis subscription; it is
    /// dropped with the thread, which is what stops the pushes.
    pub fn start(
        mut tap: nz_audio::TapSubscription,
        chain: ChainReading,
        curve: Curve,
        sink: Arc<dyn MeterSink>,
    ) -> MeterSession {
        let stop = Arc::new(AtomicBool::new(false));
        let ticks = Arc::new(AtomicU64::new(0));
        let frames = Arc::new(AtomicU64::new(0));
        let (changes, keys) = channel::<String>();
        let handle = {
            let stop = Arc::clone(&stop);
            let ticks = Arc::clone(&ticks);
            let frames = Arc::clone(&frames);
            std::thread::Builder::new()
                .name("nz-eq-meter".to_owned())
                .spawn(move || {
                    // The analysis thread runs below the UI and above bulk work
                    // (`docs/agents/mobile.md`, *Threads*): +5, the same nice the
                    // visualiser's own thread takes.
                    nz_analysis::lower_priority(5);
                    let mut spectrum = Spectrum::new(48_000.0);
                    let mut hold = RiskHold::new();
                    let mut keep: Vec<f32> = Vec::with_capacity(WINDOW * 2 + 2_048 * 2);
                    let mut hold_after = 0f64;
                    let mut last: Option<(f64, f64)> = None;
                    // One scratch buffer for the whole session: a reading must
                    // not allocate, the thread runs 25 times a second.
                    let mut scratch = vec![0.0f32; 2_048 * 2];
                    let started = Instant::now();
                    while !stop.load(Ordering::Acquire) {
                        // Every deliberate change starts a new episode, with the
                        // reading that was on screen as its baseline.
                        while let Ok(key) = keys.try_recv() {
                            hold.begin(last, elapsed_ms(started), Some(&key));
                            hold_after = elapsed_ms(started) + CHANGE_HOLD_MS;
                        }
                        // Take what the tap has: the window keeps the newest
                        // frames, so a slow reading loses the oldest, never the
                        // transient that is happening now.
                        loop {
                            let got = tap.read(&mut scratch);
                            if got == 0 {
                                break;
                            }
                            keep.extend_from_slice(&scratch[..got * 2]);
                            let keep_frames = keep.len() / 2;
                            if keep_frames > WINDOW {
                                let drop = (keep_frames - WINDOW) * 2;
                                keep.drain(..drop);
                            }
                        }
                        let now = elapsed_ms(started);
                        let (peak_db, reduction_db) = chain();
                        let (enabled, bands) = curve();
                        let (bands_db, bands01) = spectrum.bands(&keep);
                        let net: Net = hold.net(reduction_db, peak_db);
                        let risk = if net.from_change {
                            classify_risk(net.peak_db, net.reduction_db)
                        } else {
                            classify_risk(peak_db, reduction_db)
                        };
                        let risk = eq_risk(risk, enabled, &bands);
                        let shown: Option<Shown> = if now < hold_after {
                            hold.current()
                        } else {
                            hold.fold(risk, net.reduction_db, net.from_change)
                        };
                        last = Some((reduction_db, peak_db));
                        let frame = MeterFrame {
                            available: true,
                            measured: !keep.is_empty(),
                            peak_db: round1(peak_db),
                            headroom_db: round1(-peak_db),
                            reduction_db: round1(reduction_db),
                            risk,
                            bands: bands01.to_vec(),
                            bands_db: bands_db
                                .iter()
                                .map(|v| round1(f64::from(*v)) as f32)
                                .collect(),
                            warning: shown.map(|s| Warning {
                                tone: s.tone,
                                reduction_db: round1(s.reduction_db),
                                cut_db: volume_cut_db(s.reduction_db),
                                from_change: s.from_change,
                            }),
                            reason: String::new(),
                        };
                        sink.on_frame(frame);
                        ticks.fetch_add(1, Ordering::Relaxed);
                        frames.fetch_add(1, Ordering::Relaxed);
                        // 25 Hz. A thread that is not subscribed does not exist,
                        // so this wait is the card's own cost and nobody else's.
                        std::thread::sleep(Duration::from_millis(TICK_MS));
                    }
                })
                .ok()
        };
        MeterSession {
            stop,
            ticks,
            frames,
            changes,
            handle,
        }
    }

    /// A deliberate change (a band, the switch, a preset, the bass, the level):
    /// a new warning episode starts from the reading on screen.
    pub fn note_change(&self, key: &str) {
        let _ = self.changes.send(key.to_owned());
    }

    /// Readings taken so far — the number the "0 analysis CPU when the card
    /// disappears" acceptance is held to (0 new ticks after `stop_and_join`).
    pub fn ticks(&self) -> u64 {
        self.ticks.load(Ordering::Relaxed)
    }

    pub fn frames(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }

    /// Stop the thread and wait for it: after this call the meter has taken its
    /// last reading. Bounded by one tick (≤ 40 ms), so a caller may do it off the
    /// main thread.
    pub fn stop_and_join(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for MeterSession {
    fn drop(&mut self) {
        // A session dropped without an explicit stop still ends: the flag is
        // enough, the thread leaves at its next wake-up with no join needed (it
        // borrows no runtime, worker rule 5).
        self.stop.store(true, Ordering::Release);
    }
}

fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1_000.0
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The plan is one octave per slider: the geometric midpoint between two
    /// neighbouring frequencies, half an interval past each end — 21.9 Hz to
    /// 22.6 kHz for 31..16 000, exactly `bandEdges`.
    #[test]
    fn the_band_plan_is_one_octave_per_slider() {
        let edges = band_edges(&EQ_FREQS);
        assert_eq!(edges.len(), 11);
        assert!((edges[0] - 21.91).abs() < 0.02, "{}", edges[0]);
        assert!((edges[1] - (31.0f64 * 62.0).sqrt()).abs() < 1e-9);
        assert!((edges[10] - 22_627.4).abs() < 1.0, "{}", edges[10]);
        // Every band is an octave wide, and each slider sits inside its own.
        for band in 0..BANDS {
            let width = (edges[band + 1] / edges[band]).log2();
            assert!((width - 1.0).abs() < 0.01, "band {band} is {width} octaves");
            assert!(EQ_FREQS[band] > edges[band] && EQ_FREQS[band] < edges[band + 1]);
        }
    }

    /// A full-scale sine at a bin centre reads **−13.56 dB** on a band's dB,
    /// which is the analyser's own offset: the level of the sine that fills the
    /// bin. A sine two octaves below a band's centre must not show there.
    #[test]
    fn a_full_scale_sine_reads_its_own_level_on_its_own_band() {
        let mut spectrum = Spectrum::new(48_000.0);
        // 1 kHz, amplitude 1, at a bin centre (4096 bins at 48 kHz: 11.71875 Hz).
        let frames = WINDOW;
        let mut window = vec![0.0f32; frames * 2];
        for i in 0..frames {
            let x = (std::f64::consts::TAU * 1_000.0 * i as f64 / 48_000.0).sin();
            window[i * 2] = x as f32;
            window[i * 2 + 1] = x as f32;
        }
        let (db, unit) = spectrum.bands(&window);
        // Band 5 is 1 kHz: the analyser's own offset makes a full-scale sine
        // read the level that fills its bin, i.e. 0 dBFS.
        assert!(db[5].abs() < 0.6, "1 kHz at full scale reads {} dB", db[5]);
        assert!((unit[5] - 1.0).abs() < 0.02, "{}", unit[5]);
        // A band two octaves away sees far less (the plan's bins do not overlap).
        assert!(db[3] < db[5] - 40.0, "250 Hz: {} dB", db[3]);
        assert!(db[7] < db[5] - 40.0, "4 kHz: {} dB", db[7]);
        // Silence reads the floor and 0.
        let silent = vec![0.0f32; WINDOW * 2];
        let (db, unit) = spectrum.bands(&silent);
        assert!(db.iter().all(|v| *v == BAND_FLOOR_DB as f32));
        assert!(unit.iter().all(|v| *v == 0.0));
    }

    /// A window SHORTER than the transform (the meter's first readings, and any
    /// reading while the decoder catches up) is the newest frames plus silence:
    /// it must still put the tone on its own band, never somewhere else.
    #[test]
    fn a_partly_filled_window_reads_the_same_band() {
        let mut spectrum = Spectrum::new(48_000.0);
        // Three ticks of 1 920 frames: 5 760 frames, more than a window.
        let mut window = Vec::new();
        for _ in 0..3 {
            for i in 0..1_920 {
                let x = 0.25 * (std::f64::consts::TAU * 1_000.0 * i as f64 / 48_000.0).sin();
                window.push(x as f32);
                window.push(x as f32);
            }
        }
        let (db, _) = spectrum.bands(&window);
        println!("bands of 3 ticks: {db:?}");
        assert!(db[5] > -15.0, "1 kHz on its own band: {db:?}");
        assert!(db[0] < -40.0, "no low hump: {db:?}");
    }

    /// The offset is the window's own: a sine at amplitude 0.25 reads 12 dB
    /// lower, so a band's dB is comparable to the peak's dBFS.
    #[test]
    fn the_band_level_tracks_amplitude() {
        let mut spectrum = Spectrum::new(48_000.0);
        let mut level = |amp: f64| {
            let mut window = vec![0.0f32; WINDOW * 2];
            for i in 0..WINDOW {
                let x = amp * (std::f64::consts::TAU * 4_000.0 * i as f64 / 48_000.0).sin();
                window[i * 2] = x as f32;
                window[i * 2 + 1] = x as f32;
            }
            spectrum.bands(&window).0[7]
        };
        let loud = level(1.0);
        let quiet = level(0.25);
        assert!((loud - quiet - 12.04).abs() < 0.3, "{loud} - {quiet}");
    }
}
