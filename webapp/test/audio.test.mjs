// Offline checks for the JavaScript half of the audio code: the band plan
// (lib/audio/spectrum.js — what the engine falls back to when the analyser
// cannot start, and the geometry every band reader shares), the fade
// automation in graph.js, and the equalizer's meter (lib/audio/saturation.js:
// risk classification, limiter reduction, band mapping, subscription life).
// Run with `npm test` (node --test).
//
// The ANALYSIS is no longer JavaScript: it is webapp/rhythm (Rust, compiled to
// src/lib/audio/rhythm.wasm), and test/rhythm.test.mjs drives it on real audio.
//
// What is pinned here is what went wrong before, or what would be invisible in
// the UI until someone noticed the animation was subtly lying:
//  - the band plan must not map two adjacent bars onto the same bin, which is
//    what made the old visualizer draw the bass as flat groups of four;
//  - a fade interrupted mid-curve must re-arm instead of throwing;
//  - the meter must not call a full-scale peak "ok", nor report a reduction
//    the limiter did not apply, nor keep a timer nobody listens to.

import test from "node:test";
import assert from "node:assert/strict";

import { buildBandPlan, buildEnergyPlan, readBands, readEnergy, ENERGY_BANDS } from "../src/lib/audio/spectrum.js";
import {
  BAND_FLOOR_DB,
  BANDS,
  CLIP_REDUCTION_DB,
  PEAK_FLOOR_DB,
  WARN_PEAK_DB,
  bandEdges,
  classifyRisk,
  createSaturationMeter,
  limiterReduction,
  readSaturation,
  recommendedMargin,
  subscribeSaturation,
} from "../src/lib/audio/saturation.js";
import { EQ_FREQS } from "../src/lib/audio/graph.js";

const SR = 48000;
const FFT_LO = 8192;
const FFT_HI = 2048;
const FLOOR = -96;
const CEIL = -14;

// A spectrum with a 1/f tilt, which is roughly what music looks like: enough
// structure that a band reader returning a constant is obviously broken.
function tiltedSpectrum(n, nyquist, at = () => 0) {
  const a = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    const hz = ((i + 0.5) / n) * nyquist;
    a[i] = Math.max(FLOOR, -28 - 12 * Math.log10(Math.max(20, hz) / 20) + at(hz));
  }
  return a;
}

test("the band plan never reads two adjacent bars from the same place", () => {
  const plan = buildBandPlan({ bands: 120, sampleRate: SR, fftLo: FFT_LO, fftHi: FFT_HI });
  let identical = 0;
  for (let i = 1; i < plan.plan.length; i++) {
    const a = plan.plan[i - 1];
    const b = plan.plan[i];
    const sa = a.mix < 1 ? a.lo : a.hi;
    const sb = b.mix < 1 ? b.lo : b.hi;
    // Same whole-bin range, or the same interpolation point: either way the two
    // bars can only ever draw the same height.
    if (sa.frac < 0 && sb.frac < 0 && sa.i0 === sb.i0 && sa.i1 === sb.i1) identical++;
    if (sa.frac >= 0 && sb.frac >= 0 && Math.abs(sa.frac - sb.frac) < 1e-6) identical++;
  }
  assert.equal(identical, 0, "adjacent bands collapse onto the same bins");
});

test("the low end is read from the fine analyser and the top from the fast one", () => {
  const plan = buildBandPlan({ bands: 120, sampleRate: SR, fftLo: FFT_LO, fftHi: FFT_HI });
  const lowest = plan.plan[0];
  const highest = plan.plan[plan.plan.length - 1];
  assert.equal(lowest.mix, 0, "the bottom band must come from the 8192-point FFT");
  assert.equal(highest.mix, 1, "the top band must come from the 2048-point FFT");
  // ...and the seam is a ramp, not a step: some band in between must be a blend.
  assert.ok(plan.plan.some((p) => p.mix > 0 && p.mix < 1), "no blend region");
});

test("a bass tone moves the bass bars and nothing else", () => {
  const plan = buildBandPlan({ bands: 120, sampleRate: SR, fftLo: FFT_LO, fftHi: FFT_HI });
  const nyq = SR / 2;
  const flat = (n) => new Float32Array(n).fill(FLOOR);
  // A 60 Hz peak, three bins wide (the analyser's window spreads it that far).
  const lo = flat(FFT_LO / 2);
  const binHz = nyq / (FFT_LO / 2);
  const centre = Math.round(60 / binHz);
  for (let d = -1; d <= 1; d++) lo[centre + d] = -20 - Math.abs(d) * 6;
  const hi = flat(FFT_HI / 2);

  const db = new Float32Array(120);
  const v = new Float32Array(120);
  readBands(plan, lo, hi, db, v, FLOOR, CEIL);

  let peak = 0;
  for (let i = 1; i < 120; i++) if (v[i] > v[peak]) peak = i;
  assert.ok(plan.centers[peak] > 45 && plan.centers[peak] < 80,
    `peak landed at ${plan.centers[peak].toFixed(1)} Hz, expected ~60`);
  // The top of the spectrum saw nothing.
  assert.ok(v[110] < 0.02, "a bass tone leaked into the treble bands");
});

test("energy bands read the register they name", () => {
  const plan = buildEnergyPlan({ sampleRate: SR, fftLo: FFT_LO, fftHi: FFT_HI });
  const nyq = SR / 2;
  const lo = new Float32Array(FFT_LO / 2).fill(FLOOR);
  const hi = new Float32Array(FFT_HI / 2).fill(FLOOR);
  // Put everything at 4 kHz — the "high" band (2-6 kHz).
  const hiBin = Math.round(4000 / (nyq / (FFT_HI / 2)));
  for (let d = -1; d <= 1; d++) hi[hiBin + d] = -20;
  const out = new Float32Array(ENERGY_BANDS.length);
  readEnergy(plan, lo, hi, out, FLOOR);
  const names = ENERGY_BANDS.map((b) => b[0]);
  let top = 0;
  for (let i = 1; i < out.length; i++) if (out[i] > out[top]) top = i;
  assert.equal(names[top], "high");
});

// -- fade automation --------------------------------------------------------
//
// An AudioParam mock that enforces the spec rule the real engine enforces:
// setValueCurveAtTime(T, D) throws NotSupportedError if ANY automation method
// is called at a time inside [T, T+D), and cancelScheduledValues(t) removes
// only the events whose time is >= t. Modeling both is what makes these tests
// mean something — a mock that accepted everything would have passed against
// the bug.
function mockParam(value = 1) {
  const events = [];
  const inLiveCurve = (t) =>
    events.some((e) => e.type === "curve" && t >= e.t && t < e.t + e.d);
  return {
    get value() {
      return value;
    },
    events,
    setValueAtTime(v, t) {
      if (inLiveCurve(t))
        throw new DOMException("setValueAtTime overlaps", "NotSupportedError");
      value = v;
      events.push({ type: "set", v, t });
      return this;
    },
    setValueCurveAtTime(c, t, d) {
      if (inLiveCurve(t) || events.some((e) => e.t > t && e.t < t + d))
        throw new DOMException("setValueCurveAtTime overlaps", "NotSupportedError");
      value = c[c.length - 1];
      events.push({ type: "curve", c, t, d });
      return this;
    },
    cancelScheduledValues(t) {
      for (let i = events.length - 1; i >= 0; i--) if (events[i].t >= t) events.splice(i, 1);
      return this;
    },
  };
}

test("interrupting a fade mid-curve re-arms instead of throwing", async () => {
  const { fadeParam } = await import("../src/lib/audio/graph.js");
  const p = mockParam(1);
  // A six-second crossfade is in flight.
  fadeParam(p, 1, 0, 0, 6);
  assert.equal(p.events.filter((e) => e.type === "curve").length, 1);
  // Now the user skips: a new fade must start while the first one is still
  // running. This is the exact sequence that produced the
  // `NotSupportedError: setValueCurveAtTime ... overlaps` seen in the log.
  assert.doesNotThrow(() => fadeParam(p, p.value, 1, 1, 0.06));
  const curves = p.events.filter((e) => e.type === "curve");
  assert.equal(curves.length, 1, "the old curve must be gone, the new one live");
  assert.equal(curves[0].t, 1);
});

test("a fade interrupted at its very start does not throw either", async () => {
  const { fadeParam } = await import("../src/lib/audio/graph.js");
  const p = mockParam(1);
  fadeParam(p, 1, 0, 0, 4);
  // Same instant: t is inside [0, 4), and the old curve's event time is < t, so
  // cancelScheduledValues(t) alone would leave it in place and the re-arm would
  // throw. This is the case the fix exists for.
  assert.doesNotThrow(() => fadeParam(p, p.value, 0.5, 0, 3));
  assert.equal(p.events.filter((e) => e.type === "curve").length, 1);
});

test("the fade still lands on its end value", async () => {
  const { fadeParam } = await import("../src/lib/audio/graph.js");
  const p = mockParam(1);
  fadeParam(p, 1, 0, 0, 2);
  const c = p.events.find((e) => e.type === "curve");
  assert.equal(c.c[c.c.length - 1], 0, "a fade to 0 must reach 0");
  fadeParam(p, 0, 1, 5, 0); // instantaneous (a skip's hard cut path)
  assert.equal(p.events.at(-1).v, 1);
});

// The Android WebView (Chrome 154) does not truncate a curve that
// cancelAndHoldAtTime interrupts: it restarts the REST of it at the cancel
// time. Measured from the log it produced — a 60 ms skip ramp interrupted
// 9.3 ms in was reported as `setValueCurveAtTime(..., T, 0.0507)` with T the
// cancel time — and the player's next `setValueAtTime(1, T)` threw
// NotSupportedError inside the track load, which then never finished. A mock
// with exactly that behaviour:
function webviewParam(value = 1) {
  const p = mockParam(value);
  p.cancelAndHoldAtTime = (t) => {
    const ev = p.events;
    for (let i = ev.length - 1; i >= 0; i--) {
      const e = ev[i];
      if (e.type === "curve" && e.t < t && t < e.t + e.d) {
        ev[i] = { type: "curve", c: e.c, t, d: e.t + e.d - t }; // the remainder, from t
      } else if (e.t > t) ev.splice(i, 1);
    }
    return p;
  };
  return p;
}

test("an engine that restarts an interrupted curve cannot make the player's gain writes throw", async () => {
  const { fadeParam, setParam } = await import("../src/lib/audio/graph.js");
  // The unguarded write throws on this engine — the mock is faithful to it.
  const raw = webviewParam(0);
  fadeParam(raw, 0, 1, 0, 0.06);
  raw.cancelAndHoldAtTime(0.0093);
  assert.throws(() => raw.setValueAtTime(1, 0.0093), /overlaps/);

  // A softened skip's rise (60 ms), then the next skip 9.3 ms in.
  const p = webviewParam(0);
  fadeParam(p, 0, 1, 0, 0.06);
  assert.doesNotThrow(() => setParam(p, 1, 0.0093));
  const last = p.events.at(-1);
  assert.deepEqual([last.type, last.v, last.t], ["set", 1, 0.0093]);
  assert.ok(
    !p.events.some((e) => e.type === "curve" && e.t + e.d > 0.0093),
    "nothing left sounding over the value just set"
  );

  // Same engine, the next skip softened too: the re-armed fade must still be
  // a curve (a cut would click), starting where it was asked to.
  const q = webviewParam(0);
  fadeParam(q, 0, 1, 0, 0.06);
  assert.doesNotThrow(() => fadeParam(q, 0.4, 0, 0.0093, 0.06));
  const curves = q.events.filter((e) => e.type === "curve");
  assert.equal(curves.length, 1);
  assert.deepEqual([curves[0].t, curves[0].d], [0.0093, 0.06]);
});

// -- the equalizer's meter (lib/audio/saturation.js) -------------------------
//
// The meter reads two AnalyserNodes, so the tests drive it with two fakes that
// answer the two getters it calls. Everything below the getter — the band plan,
// readBands' interpolation, the peak-to-dB arithmetic, the classification — is
// the shipped code.

const PRE_FFT = 4096; // graph.js#SAT_FFT
const POST_FFT = 8192; // analyserLo
const RATE = 48000;

// A square wave of the chosen amplitude (its abs peak is exactly `amplitude`,
// unlike a sine's, which a bin or two can miss) and, optionally, one spectral
// line at `tone` Hz, everything else at the analyser's -100 dB floor.
function fakeAnalyser(fftSize, { amplitude = 0, tone = 0, toneDb = -6 } = {}) {
  return {
    fftSize,
    frequencyBinCount: fftSize / 2,
    getFloatTimeDomainData(arr) {
      const n = Math.min(arr.length, fftSize);
      for (let i = 0; i < n; i++) arr[i] = amplitude ? (i & 1 ? -amplitude : amplitude) : 0;
    },
    getFloatFrequencyData(arr) {
      arr.fill(-100);
      if (tone > 0) {
        const binHz = RATE / 2 / arr.length;
        arr[Math.min(arr.length - 1, Math.round(tone / binHz))] = toneDb;
      }
    },
  };
}

function fakeTap(preOpts, postOpts = {}) {
  return {
    pre: fakeAnalyser(PRE_FFT, preOpts),
    post: fakeAnalyser(POST_FFT, postOpts),
    sampleRate: RATE,
  };
}

test("the meter's bands are one octave each, centred on the EQ's frequencies", () => {
  const edges = bandEdges(EQ_FREQS);
  assert.equal(edges.length, EQ_FREQS.length + 1);
  for (let i = 1; i < edges.length; i++) assert.ok(edges[i] > edges[i - 1], "edges must rise");
  const plan = buildBandPlan({
    bands: BANDS,
    sampleRate: RATE,
    fftLo: PRE_FFT,
    fftHi: PRE_FFT,
    edges,
  });
  assert.equal(plan.centers.length, EQ_FREQS.length);
  for (let i = 0; i < EQ_FREQS.length; i++)
    assert.ok(
      Math.abs(plan.centers[i] / EQ_FREQS[i] - 1) < 5e-3,
      `band ${i} centres on ${plan.centers[i].toFixed(1)} Hz, not ${EQ_FREQS[i]}`
    );
  // ...and each boundary is the geometric midpoint of the two sliders it
  // splits, which is what makes "one octave per slider" true.
  for (let i = 1; i < EQ_FREQS.length; i++)
    assert.ok(Math.abs(edges[i] - Math.sqrt(EQ_FREQS[i - 1] * EQ_FREQS[i])) < 1e-9);
});

test("a tone lands in the band whose slider owns it, at its own level, and only there", () => {
  const meter = createSaturationMeter();
  // The analyser's frequency data is shifted by 13.5556 dB from dBFS (its
  // Blackman window and 1/N scaling): -19.5556 is a -6 dBFS line.
  const snap = meter.measure(
    fakeTap({ amplitude: 0.1, tone: 1000, toneDb: -19.5556 }),
    1000
  );
  let peak = 0;
  for (let i = 1; i < snap.bandsDb.length; i++) if (snap.bandsDb[i] > snap.bandsDb[peak]) peak = i;
  assert.equal(peak, 5, `the 1 kHz band is index 5 (${EQ_FREQS[5]} Hz), got ${EQ_FREQS[peak]}`);
  assert.equal(snap.bandsDb[5], -6, "a -6 dBFS line reads -6 dB, not the analyser's -19.6");
  // 0..1 is the same number over the meter's -60..0 dB window.
  assert.ok(Math.abs(snap.bands[5] - 0.9) < 1e-3, `${snap.bands[5]} should be 0.9`);
  assert.equal(snap.bandsDb[0], BAND_FLOOR_DB);
  assert.equal(snap.bands[0], 0);
});

test("silence reads the meter floor, never -Infinity or NaN", () => {
  const snap = createSaturationMeter().measure(fakeTap({ amplitude: 0 }), 7);
  assert.equal(snap.peakDb, PEAK_FLOOR_DB);
  assert.equal(snap.headroomDb, -PEAK_FLOOR_DB);
  assert.equal(snap.risk, "ok");
  assert.equal(snap.reductionDb, 0);
  assert.ok(snap.bandsDb.every((v) => v === BAND_FLOOR_DB));
  assert.ok(snap.bands.every((v) => v === 0));
});

test("the risk is ok, warn or clip at the documented bars", () => {
  assert.equal(classifyRisk(-20, 0), "ok");
  // A modern master peaking at -0.1 dBFS is normal, not a warning.
  assert.equal(classifyRisk(-0.1, 0.38), "ok");
  assert.equal(classifyRisk(WARN_PEAK_DB, 0), "warn", "a sample at full scale is a warning");
  assert.equal(classifyRisk(-3, 0.5), "warn", "the limiter starting to work is a warning");
  assert.equal(classifyRisk(2.999, 0), "warn");
  assert.equal(classifyRisk(3, 0), "clip", "3 dB past full scale");
  assert.equal(classifyRisk(-30, CLIP_REDUCTION_DB), "clip", "3 dB of limiting convicts on its own");
});

test("the reduction is the limiter's own work, and never negative", () => {
  assert.equal(limiterReduction(-6, -6), 0, "nothing to catch");
  assert.equal(limiterReduction(-6, -3), 0, "a post peak above the pre one is not a reduction");
  assert.equal(limiterReduction(-6, -12), 6);
  // ...and through the meter: 0.5 → 0.4 is 1.94 dB of reduction.
  const pulled = createSaturationMeter().measure(
    fakeTap({ amplitude: 0.5 }, { amplitude: 0.4 }),
    1
  );
  assert.ok(Math.abs(pulled.reductionDb - 1.9) <= 0.1, `measured ${pulled.reductionDb} dB`);
  assert.equal(pulled.risk, "warn", "a limiter pulling 1.9 dB is a warning");
  assert.ok(pulled.marginDb > 0, "and it recommends pulling the chain back");
  // Same level both sides: nothing was reduced.
  const even = createSaturationMeter().measure(
    fakeTap({ amplitude: 0.2 }, { amplitude: 0.2 }),
    1
  );
  assert.equal(even.reductionDb, 0);
  assert.equal(even.risk, "ok");
  assert.equal(even.marginDb, 0);
  // At full scale the limiter has just started: warn, and half a dB to release it.
  const full = createSaturationMeter().measure(
    fakeTap({ amplitude: 1 }, { amplitude: 1 }),
    1
  );
  assert.equal(full.peakDb, 0);
  assert.equal(full.risk, "warn");
  assert.equal(full.reductionDb, 0, "the limiter held it at full scale without pulling");
  assert.equal(full.marginDb, 0.5);
  // Three dB over full scale is the clip bar.
  const over = createSaturationMeter().measure(
    fakeTap({ amplitude: Math.pow(10, 3 / 20) }, { amplitude: 1 }),
    1
  );
  assert.equal(over.peakDb, 3);
  assert.equal(over.risk, "clip");
  assert.equal(over.marginDb, 3.5);
  assert.equal(recommendedMargin("ok", 0), 0);
  assert.equal(recommendedMargin("warn", 0), 0.5);
  assert.equal(recommendedMargin("clip", 3), 3.5);
  assert.equal(recommendedMargin("warn", -30, 1.9), 1.9, "releasing the limiter is a margin too");
});

test("the direct path answers unavailable, with no reading and no exception", async () => {
  const { audioOutput } = await import("../src/lib/stores.js");
  audioOutput.set("direct");
  try {
    const snap = readSaturation();
    assert.equal(snap.available, false);
    assert.equal(snap.reason, "direct");
    assert.equal(snap.measured, false);
    assert.equal(snap.bandsDb.length, BANDS);
    assert.equal(snap.bands.length, BANDS);
    assert.equal(snap.peakDb, PEAK_FLOOR_DB);
    assert.equal(snap.risk, "ok");
  } finally {
    audioOutput.set("auto");
  }
  // Nothing is routed and no graph exists: still no throw, and no fake reading.
  const idle = readSaturation();
  assert.equal(idle.available, false);
  assert.equal(idle.reason, "idle");
});

test("the meter runs only while somebody listens", () => {
  const realSet = globalThis.setInterval;
  const realClear = globalThis.clearInterval;
  let started = 0;
  let stopped = 0;
  globalThis.setInterval = (fn, ms) => {
    started++;
    return realSet(fn, ms);
  };
  globalThis.clearInterval = (id) => {
    stopped++;
    return realClear(id);
  };
  try {
    const seen = [];
    const a = subscribeSaturation((s) => seen.push(s));
    assert.equal(started, 1, "the first subscriber starts one timer");
    assert.ok(seen.length >= 1, "and is answered at once");
    assert.equal(typeof seen[0].available, "boolean");
    assert.equal(seen[0].bandsDb.length, BANDS);

    const b = subscribeSaturation(() => {});
    assert.equal(started, 1, "a second subscriber shares the timer");

    a();
    assert.equal(stopped, 0, "one subscriber is left: the timer stays");
    b();
    assert.equal(stopped, 1, "the last unsubscribe stops it");
  } finally {
    globalThis.setInterval = realSet;
    globalThis.clearInterval = realClear;
  }
});
