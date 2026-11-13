// THE EQUALIZER'S METER: what the ten bands are doing to the signal, and how
// close the chain is to saturating.
//
// WHY ITS OWN TAP. The visualizer's analysers (graph.js) read the sound the
// listener HEARS, after the safety limiter — which is right for a picture and
// useless for a setting: the limiter has already pulled the peak back down, so
// a boosted band and a flat one can read alike, and nothing there can say how
// hard the limiter is working. So graph.js carries a second, on-demand
// AnalyserNode after the ten EQ filters, the bass shelf and its compressor,
// and BEFORE the limiter (`buildSaturationTap`). Its peak is what the settings
// produce; the limiter's own gain reduction is that peak minus the peak after
// it. Neither analyser moves, and the tap never carries sound anywhere: it is
// a read-only branch, so NOTHING here changes what is heard.
//
// THE BAND PLAN is one octave per EQ slider, centred on that slider's
// frequency: the boundaries are the geometric midpoints between neighbouring
// EQ_FREQS (31, 62, … 16 000 Hz), extrapolated by half an interval at each end.
// A log plan over 22 Hz..18 kHz — the visualizer's — would spread ten bars over
// the whole spectrum instead, putting the 31 Hz bar across two sliders and
// leaving the top slider measuring half of nothing. `readBands` still does the
// reading; satPre is a single 4096-point analyser, so the plan is built with
// that FFT size on both sides.
//
// COST. Nothing runs until the first subscriber: no node, no buffer, no timer.
// With a subscriber there is one AnalyserNode (16 KB of ring in the engine),
// three Float32Arrays (56 KB: one exact-length window per analyser, 4096 and
// 8192 samples, and the 2048-bin spectrum), and one 25 Hz timer. Each tick
// copies 4096 + 8192 samples and computes one 4096-point FFT — measured in
// headless Chromium 155 at 0.35 ms of main thread per tick, 0.9 % of one core
// over a minute (see the report). The 85 ms window (fftSize 4096 at 48 kHz) is
// deliberate: at 25 Hz a shorter window could fall between two late readings
// and miss the very transient the warning is about; at 85 ms a transient stays
// visible for two readings, and the value's latency is one tick (≤ 40 ms) plus
// the window's own hold.
//
// WHAT IT CANNOT SEE, stated rather than papered over:
//  - an AnalyserNode down-mixes its input to mono before measuring (Web Audio
//    spec, §1.8.5), pre and post alike, so a hard anti-phase stereo peak reads
//    low. It is a sample peak, not a true peak: inter-sample overs are not
//    counted, which is why the warning bar sits at 0 dBFS and not at -1;
//  - the meter measures the processed path only. On the direct path nothing
//    goes through the graph (the equalizer is inactive there): `available` is
//    false with reason "direct", never a fabricated reading.
//
// THE CONTRACT (what the equalizer screen consumes — it is the only consumer):
//
//   readSaturation() -> Snapshot            // the latest reading, never throws
//   subscribeSaturation(fn) -> unsubscribe  // fn(snapshot) at 25 Hz, current
//                                           // value at once, timer stops at
//                                           // the last unsubscribe
//
//   Snapshot = {
//     available: boolean,   // false: no processed sound to measure
//     measured: boolean,    // false until a first reading (updatedAt > 0)
//     peakDb: number,       // PRE-limiter sample peak, dBFS, floored at -96
//     headroomDb: number,   // -peakDb (dB below full scale; <0 when over)
//     risk: "ok" | "warn" | "clip",
//     reductionDb: number,  // limiter gain reduction, >= 0
//     marginDb: number,     // dB to pull back to release the limiter; 0
//                           // while risk is "ok"
//     bandsDb: number[10],  // per EQ band, dBFS, floored at -60, EQ order
//     bands: number[10],    // the same, mapped 0..1 over [-60, 0] dB
//     updatedAt: number,    // performance.now() of the reading, 0 before
//     reason?: "direct" | "idle"  // only when available is false
//   }
//
// `bandsDb[i]` is the level of the signal each EQ slider acts on, NOT the
// slider's gain: a flat EQ shows the track's own spectrum, and a +6 dB bass
// slider raises band 0. The snapshot is immutable (fresh arrays per reading),
// so a component may keep it.

import {
  EQ_FREQS,
  getSaturationTap,
  isDirect,
  requestSaturationTap,
  releaseSaturationTap,
} from "./graph.js";
import { buildBandPlan, readBands } from "./spectrum.js";

/** How many bands the meter reports: the EQ's own ten. */
export const BANDS = EQ_FREQS.length;

// The meter's display window. -60 dB is a conventional meter floor (quiet
// enough that a silent band reads empty, high enough that the bottom of the
// scale is still legible), and 0 dBFS is full scale.
export const BAND_FLOOR_DB = -60;
export const BAND_CEIL_DB = 0;
// An AnalyserNode's frequency data is NOT in dBFS: it is the Blackman-windowed
// FFT divided by fftSize, so a full-scale sine at a bin centre reads
// 20·log10(0.42 / 2) = -13.56 dB. (The visualizer's own CEIL_DB of -14 is that
// same fact, used as the top of its picture.) Shifting by it makes a band's dB
// the level of the sine that would fill its strongest bin, on the same scale as
// `peakDb`: an 80 Hz tone at -21.9 dBFS reads -21.9 dB in band 1, not -36.
// Broadband content reports its strongest component — what a per-band peak
// meter means — rather than a power sum, which would depend on bin width.
const ANALYSER_SINE_OFFSET_DB = 13.5556;
// A digital peak never really "is" -96 dB; this is the floor of the readout
// (and what silence reports, instead of -Infinity, so arithmetic works).
export const PEAK_FLOOR_DB = -96;

// RISK, and why these numbers. The chain's safety limiter sits at a -0.5 dBFS
// threshold with a 20:1 ratio, so its gain reduction is 0.95 × (pre - (-0.5)):
// a modern master peaking at -0.1 dBFS already makes it work by 0.38 dB, and
// that is inaudible and normal. A warning that fires there would be on for
// every track ever released. So the bars follow what the LIMITER is doing:
//  - ok    : it is idle (under 0.5 dB of reduction) and the pre-limiter peak is
//            below full scale;
//  - warn  : the pre-limiter peak reaches 0 dBFS, or the limiter is pulling
//            0.5-3 dB — enough to hear as a touched transient;
//  - clip  : the peak is driven 3 dB past full scale, or the limiter is pulling
//            3 dB or more (0.95 × 3.5 dB over its threshold): the chain is
//            squashing the music, and a boost did it.
// A sample at 0 dBFS is not itself clipping — the limiter is already catching
// it — which is why "clip" starts past the ceiling, not at it. (Distribution's
// -1 dBTP is a delivery spec for a file, not a playback-chain threshold: a
// finished master is meant to sit just under 0.)
export const WARN_PEAK_DB = 0;
export const CLIP_PEAK_DB = 3;
export const WARN_REDUCTION_DB = 0.5;
export const CLIP_REDUCTION_DB = 3;
// What `marginDb` pulls back to: the limiter's own threshold (-0.5 dBFS,
// graph.js#limiterNode). Pulling the pre-limiter peak down to it releases the
// limiter completely, which is the actionable answer for a boosted EQ.
export const TARGET_PEAK_DB = -0.5;

// 25 Hz: fast enough that a slider move answers under the eye, slow enough
// that a 4096-point FFT costs nothing next to the visualizer's 60 Hz reads.
const TICK_MS = 40;

/** The EQ frequencies, re-exported so a meter labels itself from one import. */
export { EQ_FREQS };

/** Classify a pre-limiter peak and the limiter's reduction. Pure. */
export function classifyRisk(peakDb, reductionDb = 0) {
  if (peakDb >= CLIP_PEAK_DB || reductionDb >= CLIP_REDUCTION_DB) return "clip";
  if (peakDb >= WARN_PEAK_DB || reductionDb >= WARN_REDUCTION_DB) return "warn";
  return "ok";
}

/** The gain reduction the limiter applied: how much lower the peak is now. */
export function limiterReduction(prePeakDb, postPeakDb) {
  return Math.max(0, prePeakDb - postPeakDb);
}

/** How far to pull the chain back to reach TARGET_PEAK_DB (0 when safe). */
export function recommendedMargin(risk, peakDb, reductionDb = 0) {
  if (risk === "ok") return 0;
  // The peak sets the target; a limiter already working hard sets a floor
  // under it — pulling back by its own reduction is what releases it.
  return Math.max(0, peakDb - TARGET_PEAK_DB, reductionDb);
}

// Band boundaries: the geometric midpoint between two neighbouring EQ
// frequencies, and half an interval past each end. For 31..16000 (octaves)
// this is 21.9 Hz .. 22.6 kHz, one octave per band.
export function bandEdges(freqs = EQ_FREQS) {
  const n = freqs.length;
  if (n < 2) return [freqs[0] / Math.SQRT2, freqs[0] * Math.SQRT2];
  const edges = new Array(n + 1);
  for (let i = 1; i < n; i++) edges[i] = Math.sqrt(freqs[i - 1] * freqs[i]);
  edges[0] = (freqs[0] * freqs[0]) / edges[1];
  edges[n] = (freqs[n - 1] * freqs[n - 1]) / edges[n - 1];
  return edges;
}

const round1 = (v) => Math.round(v * 10) / 10;
const round3 = (v) => Math.round(v * 1000) / 1000;
const nowMs = () =>
  typeof performance !== "undefined" && performance.now ? performance.now() : Date.now();

// The peak of a window, in dBFS. `start..end` is the range read: the two
// analysers have different fftSizes (graph.js#buildSaturationTap), so the
// caller takes the most recent min(fftSize) samples of each — the same 85 ms of
// audio — and the difference stays a gain reduction rather than the difference
// of two windows.
function peakDb(wave, start, end) {
  let peak = 0;
  for (let i = start; i < end; i++) {
    const v = wave[i];
    const a = v < 0 ? -v : v;
    if (a > peak) peak = a;
  }
  if (!(peak > 0)) return PEAK_FLOOR_DB;
  const db = 20 * Math.log10(peak);
  return db < PEAK_FLOOR_DB ? PEAK_FLOOR_DB : db;
}

/**
 * A reading engine, allocated on first use and reused (no allocation per
 * frame). Exported so the measurement can be driven with fake analysers in the
 * unit tests; the app uses the one at the bottom of this file.
 */
export function createSaturationMeter() {
  const edges = bandEdges(EQ_FREQS);
  let plan = null;
  let rate = 0;
  let planFft = 0;
  let preWave = null; // the pre-limiter window (SAT_FFT samples)
  let postWave = null; // the post-limiter window (analyserLo's fftSize)
  let freq = null; // the pre-limiter spectrum
  const bandsDb = new Float32Array(BANDS);
  const bands01 = new Float32Array(BANDS);

  function prepare(tap) {
    const pre = tap.pre;
    const post = tap.post;
    if (
      plan &&
      rate === tap.sampleRate &&
      planFft === pre.fftSize &&
      preWave.length === pre.fftSize &&
      postWave.length === post.fftSize &&
      freq.length === pre.frequencyBinCount
    )
      return;
    plan = buildBandPlan({
      bands: BANDS,
      sampleRate: tap.sampleRate,
      fftLo: pre.fftSize,
      fftHi: pre.fftSize,
      edges,
    });
    preWave = new Float32Array(pre.fftSize);
    postWave = new Float32Array(post.fftSize);
    freq = new Float32Array(pre.frequencyBinCount);
    rate = tap.sampleRate;
    planFft = pre.fftSize;
  }

  return {
    /**
     * One reading. `tap` is `{ pre, post, sampleRate }` — two AnalyserNodes
     * (anything with fftSize / frequencyBinCount / the two getters will do) and
     * the context's rate. Read-only: it changes nothing on the analysers.
     */
    measure(tap, now = nowMs()) {
      prepare(tap);
      const pre = tap.pre;
      const post = tap.post;
      const n = Math.min(pre.fftSize, post.fftSize);

      // post first (its window is the larger one): the peaks are taken from the
      // most recent n samples of each, so the two sides of the limiter are
      // judged on the same 85 ms of audio.
      post.getFloatTimeDomainData(postWave);
      const postPeak = peakDb(postWave, postWave.length - n, postWave.length);
      pre.getFloatTimeDomainData(preWave);
      const prePeak = peakDb(preWave, preWave.length - n, preWave.length);

      pre.getFloatFrequencyData(freq);
      // readBands' window is shifted into analyser units, and the dB it writes
      // is shifted back below; 0..1 is already over the -60..0 dBFS display.
      readBands(
        plan,
        freq,
        freq,
        bandsDb,
        bands01,
        BAND_FLOOR_DB - ANALYSER_SINE_OFFSET_DB,
        BAND_CEIL_DB - ANALYSER_SINE_OFFSET_DB
      );

      const reductionDb = limiterReduction(prePeak, postPeak);
      const risk = classifyRisk(prePeak, reductionDb);
      const bandsDbOut = new Array(BANDS);
      const bandsOut = new Array(BANDS);
      for (let i = 0; i < BANDS; i++) {
        bandsDbOut[i] = round1(bandsDb[i] + ANALYSER_SINE_OFFSET_DB);
        bandsOut[i] = round3(bands01[i]);
      }
      return {
        available: true,
        measured: true,
        // The displayed peak is rounded; the risk above was classified on the
        // measured value, so 2.96 dB cannot read "warn" beside "3.0 dB".
        peakDb: round1(prePeak),
        headroomDb: round1(-prePeak),
        risk,
        reductionDb: round1(reductionDb),
        marginDb: round1(recommendedMargin(risk, prePeak, reductionDb)),
        bandsDb: bandsDbOut,
        bands: bandsOut,
        updatedAt: now,
      };
    },
  };
}

// --- the shared reading, and the timer that only exists for a subscriber -----

const meter = createSaturationMeter();
const entries = new Set();
let ticker = 0;
let current = blank(true);
let lastReason = "";

function blank(available, reason) {
  const snap = {
    available,
    measured: false,
    peakDb: PEAK_FLOOR_DB,
    headroomDb: -PEAK_FLOOR_DB,
    risk: "ok",
    reductionDb: 0,
    marginDb: 0,
    bandsDb: new Array(BANDS).fill(BAND_FLOOR_DB),
    bands: new Array(BANDS).fill(0),
    updatedAt: 0,
  };
  if (reason) snap.reason = reason;
  return snap;
}

function emit(snap) {
  for (const e of entries) {
    try {
      e.fn(snap);
    } catch {
      /* a broken subscriber must not stop the meter or the others */
    }
  }
}

function publish(snap) {
  current = snap;
  lastReason = "";
  emit(snap);
}

// An unavailable state is published once, not 25 times a second: a screen that
// is showing "direct" has nothing changing behind it.
function publishUnavailable(reason) {
  if (!current.available && lastReason === reason) return;
  current = blank(false, reason);
  lastReason = reason;
  emit(current);
}

function tick() {
  if (!entries.size) return;
  if (isDirect()) {
    publishUnavailable("direct");
    return;
  }
  const tap = getSaturationTap();
  if (!tap) {
    publishUnavailable("idle");
    return;
  }
  // Suspended (paused, or the page put it to sleep): both windows hold their
  // last samples, so a reading would be a frozen one dressed as a live one.
  // Keep the last value; `updatedAt` ages, which is the honest signal.
  if (!tap.running) return;
  try {
    publish(meter.measure(tap));
  } catch {
    // A context torn down between the tap and the reads: say so next tick.
    publishUnavailable("idle");
  }
}

function startTicker() {
  if (ticker) return;
  tick(); // the first reading is there when the screen mounts
  ticker = setInterval(tick, TICK_MS);
}

function stopTicker() {
  if (ticker) clearInterval(ticker);
  ticker = 0;
}

/**
 * The latest reading. Never throws and never measures: it reports the last
 * tick, or why nothing can be measured right now. Call it from a subscriber,
 * or once for a diagnostic; `available === false` means ignore the numbers.
 * With no subscriber there is no tap (nothing is being measured), so on the
 * processed path this answers "idle" rather than inventing a value — the
 * screen's reading path is `subscribeSaturation`.
 */
export function readSaturation() {
  if (isDirect()) return blank(false, "direct");
  if (!getSaturationTap()) return blank(false, "idle");
  return current.available ? current : blank(true);
}

/**
 * Follow the meter at 25 Hz. `fn` is called once immediately with the current
 * value, then on every reading. The timer, the analyser and the buffers exist
 * only while at least one subscriber does; the returned function is idempotent.
 */
export function subscribeSaturation(fn) {
  if (typeof fn !== "function") return () => {};
  const entry = { fn };
  entries.add(entry);
  if (entries.size === 1) {
    requestSaturationTap();
    startTicker();
  }
  try {
    fn(readSaturation());
  } catch {
    /* a broken subscriber must not stop the meter */
  }
  return () => {
    if (!entries.delete(entry)) return;
    if (entries.size) return;
    stopTicker();
    releaseSaturationTap();
  };
}
