// Offline checks for the animation layer (lib/viz/*).
//
// Two kinds of scene come out of lib/viz, and they are checked in two places.
//
// THE CANVAS SCENES — the oscilloscope and the spectrum bars' no-WebGL twin —
// only ever touch a CanvasRenderingContext2D, which is a recordable interface:
// they are driven here against a stub that writes down where they drew, and
// asked whether they fill the frame, keep off the artwork and (the scope)
// actually draw the SAMPLES.
//
// THE WORLDS are fragment shaders. Node has no GPU, so what they draw is asked
// of real pixels by the render bench (test/render/run.mjs --check), which
// renders every one in headless Chromium. What this file pins about them is
// everything that can be decided WITHOUT a GPU and that a GPU would only report
// as a black screen: the catalogue, the loader and the skins agree with each
// other; every genre either vocabulary can name resolves to a world that
// exists, with parameters that world actually reads; every P_ define and every
// uniform a shader mentions is one the engine declares; and no shader can ask
// for wall-clock time, because the engine does not give it any.

import { readFileSync } from "node:fs";
import test from "node:test";
import assert from "node:assert/strict";

import { createGeometry } from "../src/lib/viz/geometry.js";
import { vizCoreFromBytes } from "../src/lib/viz/core.js";
import { createScene, createFallback, MODES, effectiveMode, levelFor } from "../src/lib/viz/index.js";
import { tierPreset, TIERS } from "../src/lib/viz/quality.js";
import { createPalette, zonesOfPalette } from "../src/lib/viz/palette.js";
import { FAMILY_LIST, GROOVES, genreOf, grooveOf, familyLook, tempoRangeFor } from "../src/lib/audio/style.js";
import { WORLD_META, GROUPS, worldFor } from "../src/lib/viz/worlds/catalogue.js";
import { worldIds, loadWorld, hasWorld, instrumentIds } from "../src/lib/viz/worlds/index.js";
import { SKINS, ALIASES, skinId, skinFor, skinCount } from "../src/lib/viz/skins.js";
import { worldFragment, particleVertex, particleFragment, WORLD_UNIFORMS } from "../src/lib/viz/gl/glsl.js";
import { FINAL_FS, PREFILTER_FS } from "../src/lib/viz/gl/postfx.js";

// The scenes' arithmetic is Rust (lib/viz/core.js); in the app the core loads
// with the scene, here it is instantiated once from the shipped binary.
vizCoreFromBytes(readFileSync(new URL("../src/lib/audio/rhythm.wasm", import.meta.url)));

// --- a canvas that remembers where it was drawn on --------------------------
//
// `ink` collects the coordinates of everything with a POSITION: path points,
// arc and ellipse extremes, non-full-frame rectangles. A full-frame fillRect is
// deliberately NOT counted — it is the trail wash and the colour floor, every
// scene paints one, and counting it would make "does it fill the frame" pass
// for a scene that draws a dot in the middle.
function recorder(w, h) {
  const ink = [];
  let x = 0;
  let y = 0;
  // A 2×3 affine stack, because some worlds squash a shape with
  // save/translate/scale rather than recomputing its geometry — a recorder that
  // ignored that would file those points in the wrong place.
  let m = [1, 0, 0, 1, 0, 0];
  const stack = [];
  const grad = () => ({ addColorStop() {} });
  // The third element is the compositing mode the point was drawn under. Every
  // caller that only wants coordinates destructures the first two and is
  // unaffected; what it buys is telling a scene's GLOW layer from its line work
  // — and, for the oscilloscope, its trace (additive) from the graticule it is
  // drawn against (which is not).
  const put = (px, py) => {
    if (!Number.isFinite(px) || !Number.isFinite(py)) return;
    ink.push([m[0] * px + m[2] * py + m[4], m[1] * px + m[3] * py + m[5], g.globalCompositeOperation]);
  };
  const g = {
    canvas: { width: w, height: h },
    globalCompositeOperation: "source-over",
    fillStyle: "",
    strokeStyle: "",
    lineWidth: 1,
    lineCap: "butt",
    createLinearGradient: grad,
    createRadialGradient: grad,
    setTransform() {},
    save() {
      stack.push(m.slice());
    },
    restore() {
      if (stack.length) m = stack.pop();
    },
    translate(tx, ty) {
      m = [m[0], m[1], m[2], m[3], m[0] * tx + m[2] * ty + m[4], m[1] * tx + m[3] * ty + m[5]];
    },
    scale(sx, sy) {
      m = [m[0] * sx, m[1] * sx, m[2] * sy, m[3] * sy, m[4], m[5]];
    },
    rotate(r) {
      const c = Math.cos(r);
      const si = Math.sin(r);
      m = [
        m[0] * c + m[2] * si, m[1] * c + m[3] * si,
        m[0] * -si + m[2] * c, m[1] * -si + m[3] * c,
        m[4], m[5],
      ];
    },
    clearRect() {},
    beginPath() {},
    closePath() {},
    fill() {},
    stroke() {},
    moveTo(a, b) {
      x = a;
      y = b;
      put(a, b);
    },
    lineTo(a, b) {
      x = a;
      y = b;
      put(a, b);
    },
    quadraticCurveTo(cx, cy, a, b) {
      put(cx, cy);
      put(a, b);
      x = a;
      y = b;
    },
    arc(cx, cy, r) {
      put(cx - r, cy);
      put(cx + r, cy);
      put(cx, cy - r);
      put(cx, cy + r);
    },
    ellipse(cx, cy, rx, ry) {
      put(cx - rx, cy);
      put(cx + rx, cy);
      put(cx, cy - ry);
      put(cx, cy + ry);
    },
    fillRect(a, b, rw, rh) {
      // A full-frame wash says nothing about where the scene put its material.
      if (a <= 0.5 && b <= 0.5 && rw >= w - 1 && rh >= h - 1) return;
      put(a, b);
      put(a + rw, b + rh);
    },
    // Text is ink with a position like any other: the scope names its two
    // traces, and a recorder without `fillText` would take the suite down
    // rather than measure it (see `strokeRect` below, which already did).
    fillText(_text, a, b) {
      put(a, b);
    },
    measureText(t) {
      return { width: String(t).length * 6 };
    },
    // A recorder that is MISSING a canvas call does not measure the scene
    // wrongly, it throws — `industrial` draws its frames with `strokeRect` and
    // took the whole suite down with "g.strokeRect is not a function" the first
    // time anything asked it to paint.
    strokeRect(a, b, rw, rh) {
      if (a <= 0.5 && b <= 0.5 && rw >= w - 1 && rh >= h - 1) return;
      put(a, b);
      put(a + rw, b + rh);
    },
    roundRect(a, b, rw, rh) {
      put(a, b);
      put(a + rw, b + rh);
    },
    arcTo(cx, cy, a, b) {
      put(cx, cy);
      put(a, b);
      x = a;
      y = b;
    },
    bezierCurveTo(c1x, c1y, c2x, c2y, a, b) {
      put(c1x, c1y);
      put(c2x, c2y);
      put(a, b);
      x = a;
      y = b;
    },
    rect(a, b, rw, rh) {
      put(a, b);
      put(a + rw, b + rh);
    },
    ink,
    get lastPoint() {
      return [x, y];
    },
  };
  return g;
}

// --- the audio the scenes are driven with -----------------------------------
//
// The oscilloscope draws the SAMPLES, so a frame with no samples in it tests
// nothing about it — and a drawn sine would test its own drawing. This is one
// looping second of the kind of record this player exists for, written as
// samples: a pitched kick swept 180 -> 45 Hz into a soft clipper twice a second,
// a saw lead panned right, hats on the offbeats, and the two channels genuinely
// different so "one trace per channel" is a claim a test can check.
//
// Built ONCE and stored twice end to end, so any offset yields a contiguous
// window without a wrap and a frame costs a subarray rather than a synthesis.
const WAVE_SR = 48000;
const WAVE_LEN = WAVE_SR;
const waveSrcL = new Float32Array(WAVE_LEN * 2);
const waveSrcR = new Float32Array(WAVE_LEN * 2);
(function buildWave() {
  const beat = 0.5; // 120 BPM
  let hatSeed = 12345;
  const rnd = () => ((hatSeed = (hatSeed * 1664525 + 1013904223) >>> 0) / 4294967296) * 2 - 1;
  let kickPhase = 0;
  for (let i = 0; i < WAVE_LEN; i++) {
    const t = i / WAVE_SR;
    const inBeat = t % beat;
    // The kick: a sine whose frequency falls through the first 50 ms, driven
    // into a clipper. Its own low end is what the scope's trigger locks to.
    const kEnv = Math.exp(-inBeat * 9);
    const kHz = 45 + 135 * Math.exp(-inBeat * 26);
    kickPhase += (2 * Math.PI * kHz) / WAVE_SR;
    const kick = Math.tanh(Math.sin(kickPhase) * 3.2) * kEnv * 0.75;
    // The lead: three harmonics of 220 Hz, panned right.
    const lead =
      (Math.sin(2 * Math.PI * 220 * t) * 0.5 +
        Math.sin(2 * Math.PI * 440 * t) * 0.22 +
        Math.sin(2 * Math.PI * 660 * t) * 0.11) *
      0.32;
    // Hats on the offbeats: short bursts of noise.
    const hEnv = Math.exp(-((inBeat - beat / 2 + beat) % beat) * 90);
    const hat = rnd() * hEnv * 0.12;
    waveSrcL[i] = Math.max(-1, Math.min(1, kick + lead * 0.35 + hat));
    waveSrcR[i] = Math.max(-1, Math.min(1, kick + lead + hat * 0.6));
  }
  waveSrcL.copyWithin(WAVE_LEN, 0, WAVE_LEN);
  waveSrcR.copyWithin(WAVE_LEN, 0, WAVE_LEN);
})();

const silentWave = new Float32Array(WAVE_LEN);

// A sustained 220 Hz tone. Music is the right material for almost everything
// here, but not for "does the trace stand still": on real music consecutive
// windows share only 60% of their audio, so the picture legitimately changes
// whatever the trigger does, and the measurement says nothing. A steady tone is
// the one signal where the trace SHOULD be identical frame to frame, which
// makes the trigger the only thing the number can be about.
const toneSrc = new Float32Array(WAVE_LEN * 2);
for (let i = 0; i < toneSrc.length; i++)
  toneSrc[i] = Math.sin((2 * Math.PI * 220 * i) / WAVE_SR) * 0.6;

function waveAt(t, size = 4096, mode = "music") {
  if (mode === "tone") {
    const off = Math.floor(t * WAVE_SR) % WAVE_LEN;
    return {
      left: toneSrc.subarray(off, off + size),
      right: toneSrc.subarray(off, off + size),
      size,
      sampleRate: WAVE_SR,
    };
  }
  if (mode === "silent")
    return {
      left: silentWave.subarray(0, size),
      right: silentWave.subarray(0, size),
      size,
      sampleRate: WAVE_SR,
    };
  const off = Math.floor(t * WAVE_SR) % WAVE_LEN;
  return {
    left: waveSrcL.subarray(off, off + size),
    right: waveSrcR.subarray(off, off + size),
    size,
    sampleRate: WAVE_SR,
  };
}

// --- a frame the scenes can be driven with ----------------------------------
const BANDS = 120;
function makeFrame(t, opts = {}) {
  const bands = new Float32Array(BANDS);
  for (let i = 0; i < BANDS; i++) bands[i] = 0.3 + 0.5 * Math.abs(Math.sin(i * 0.21 + t * 3));
  const chroma = new Float32Array(12);
  for (let i = 0; i < 12; i++) chroma[i] = i % 4 === 0 ? 0.9 : 0.2;
  const beatEvery = 0.5;
  const beat = Math.floor(t / beatEvery) !== Math.floor((t - 1 / 60) / beatEvery);
  const idx = Math.floor(t / beatEvery);
  return {
    t,
    dt: 1 / 60,
    bands,
    energy: { sub: 0.4, bass: 0.5, lowMid: 0.3, mid: 0.35, high: 0.2, air: 0.12 },
    features: {
      level: 0.7, flux: 0.02, lowFlux: 0.03, midFlux: 0.02, highFlux: 0.01,
      centroidN: 0.45, flatness: 0.35, percussivity: 0.6, vocalMod: 0.4, crest: 3,
      silent: false, kick: beat ? 0.9 : 0.2, dynamics: 1, tonal: 0.5, melody: 0.6,
      melodyPitch: 0.5, melodyFlux: 0.03, chordChange: 0.1, chroma,
    },
    beat: {
      bpm: 120, confidence: 0.8, phase: (t % beatEvery) / beatEvery, beat,
      beatIndex: idx, barPos: idx % 4, beatsPerBar: 4, downbeat: beat && idx % 4 === 0,
      onset: beat ? 0.8 : 0, kickPulse: 0.8, sinceBeat: t % beatEvery,
      period: beatEvery, locked: true,
    },
    style: opts.style ?? null,
    wave: opts.wave === null ? null : waveAt(t, opts.waveSize || 4096, opts.wave),
  };
}


// The canvas scenes are CODE-SPLIT in the app (lib/viz/index.js loads them on
// demand), which makes `createScene` a promise. These checks are about what
// they DRAW, so they load once here and stay synchronous; the loader's own
// contract is pinned separately, below.
const SCENE_FACTORY = {
  bars: (await import("../src/lib/viz/scenes/bars.js")).createBarsScene,
  scope: (await import("../src/lib/viz/scenes/scope.js")).createScopeScene,
};
const makeScene = (mode, opts) => (SCENE_FACTORY[mode] ? SCENE_FACTORY[mode](opts) : null);

// Run a scene for a while and return where it drew.
function paint(
  mode,
  {
    w, h, occl = null, style = null, tier = "high", seconds = 6,
    orientation = "horizontal", colour = "duo", wave, waveSize,
  } = {}
) {
  const preset = tierPreset(tier);
  const geometry = createGeometry();
  geometry.set(w, h, occl);
  const scene = makeScene(mode, {
    preset, layout: "full", intensity: 0.8, orientation, colour,
  });
  scene.resize(w, h, preset, geometry.out);
  const pal = createPalette("neon");
  const g = recorder(w, h);
  const dt = 1 / 60;
  for (let i = 0; i < Math.round(seconds / dt); i++) {
    const f = makeFrame(i * dt, { style, wave, waveSize });
    pal.update(f, dt);
    scene.update(f, dt, geometry.out);
    scene.draw(g, w, h, pal.out, geometry.out);
  }
  return { ink: g.ink, scene, geom: geometry.out };
}


// Where a scene spent its ink, as an 8×8 histogram normalised to sum to 1.
function signature(ink, w, h) {
  const n = 8;
  const cell = new Float64Array(n * n);
  let total = 0;
  for (const [px, py] of ink) {
    const cx = Math.min(n - 1, Math.max(0, Math.floor((px / w) * n)));
    const cy = Math.min(n - 1, Math.max(0, Math.floor((py / h) * n)));
    cell[cy * n + cx]++;
    total++;
  }
  if (total) for (let i = 0; i < cell.length; i++) cell[i] /= total;
  return cell;
}
function sigDistance(a, b) {
  let d = 0;
  for (let i = 0; i < a.length; i++) d += Math.abs(a[i] - b[i]);
  return d; // 0 identical, 2 disjoint
}

// `Math.min(...array)` on a few hundred thousand points overflows the stack,
// which is a very confusing way for a coverage test to fail.
function extent(ink) {
  let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
  for (const [x, y] of ink) {
    if (x < minX) minX = x;
    if (x > maxX) maxX = x;
    if (y < minY) minY = y;
    if (y > maxY) maxY = y;
  }
  return { minX, maxX, minY, maxY };
}

test("the geometry maps radial 1 onto the frame's own edge, not onto a circle", () => {
  const { set, out } = createGeometry();
  set(1920, 1080, null);
  for (let i = 0; i < 24; i++) {
    const a = (i / 24) * Math.PI * 2;
    const p = out.place(a, 1);
    const onEdge =
      Math.abs(p[0]) < 1 || Math.abs(p[0] - 1920) < 1 ||
      Math.abs(p[1]) < 1 || Math.abs(p[1] - 1080) < 1;
    assert.ok(onEdge, `radial 1 at ${a.toFixed(2)} landed at ${p[0].toFixed(0)},${p[1].toFixed(0)}`);
    assert.ok(p[0] >= -1 && p[0] <= 1921 && p[1] >= -1 && p[1] <= 1081, "outside the frame");
  }
  // ...and an expanding ring covers the whole frame before it is done.
  assert.ok(out.ringRx(1) >= 960, "a finished ring does not reach the sides");
  assert.ok(out.ringRy(1) >= 540, "a finished ring does not reach the top");
});

test("with artwork in the way, radial 0 is its rim", () => {
  const { set, out } = createGeometry();
  const occl = { x: 660, y: 240, w: 600, h: 600 };
  set(1920, 1080, occl);
  assert.ok(out.hole > 0.5, `hole read as ${out.hole.toFixed(2)}`);
  for (let i = 0; i < 24; i++) {
    const a = (i / 24) * Math.PI * 2;
    const p = out.place(a, 0);
    const dx = Math.abs(p[0] - out.cx);
    const dy = Math.abs(p[1] - out.cy);
    // On the boundary of the (padded) artwork box: one axis at its half-extent,
    // neither beyond it.
    assert.ok(
      Math.abs(dx - out.hw) < 1 || Math.abs(dy - out.hh) < 1,
      `radial 0 at ${a.toFixed(2)} is not on the rim`
    );
    assert.ok(dx <= out.hw + 1 && dy <= out.hh + 1, "radial 0 escaped the rim");
  }
  // The artwork can never eat the frame, however big it is reported to be.
  set(400, 400, { x: -50, y: -50, w: 500, h: 500 });
  assert.ok(out.hw < 200 && out.hh < 200, "the artwork was allowed to cover everything");
});

test("the GL engine is told where the artwork really is, not where a centred one would be", () => {
  // The mobile player's cover sits ABOVE the middle of the canvas (the
  // controls are under it). The canvas-era geometry grows its box around the
  // frame's centre so its polar primitives stay honest, and the GL engine
  // used to be handed that grown box: every world centred its motif 7% of
  // the frame below the cover it was framing and dimmed the wrong region.
  const { set, out } = createGeometry();
  const w = 390;
  const h = 844;
  const occl = { x: 40, y: 250, w: 310, h: 310 }; // centre y 405, frame centre 422
  set(w, h, occl);
  assert.equal(out.hx, 195);
  assert.equal(out.hy, 405);
  // The artwork's own padded box: its half-size plus the pad, and no more.
  assert.ok(out.ahw >= 155 && out.ahw < 155 + 0.08 * w, `ahw ${out.ahw}`);
  assert.ok(out.ahh >= 155 && out.ahh < 155 + 0.08 * w, `ahh ${out.ahh}`);
  // The grown one still contains it, for the scope's primitives.
  assert.ok(out.hh >= out.ahh + Math.abs(out.hy - out.cy) - 1e-9, "the grown box lost the artwork");
  // And the free band under the artwork starts under the ARTWORK.
  assert.ok(Math.abs(out.afloorY - (out.hy + out.ahh)) < 1e-9);
  assert.ok(out.afloorY < out.floorY, "the true floor should sit above the grown one");
});

test("the canvas scenes survive every tier, aspect and artwork, at any frame", () => {
  // The cheap insurance: a scene that throws takes the whole render loop with
  // it, and a projector nobody is looking at is exactly where that happens.
  // The bars are what a device with no WebGL2 gets instead of every world.
  const shapes = [
    [1920, 1080, null],
    [390, 844, { x: 20, y: 300, w: 350, h: 350 }],
    [3440, 1440, null],
    [800, 800, { x: 100, y: 100, w: 600, h: 600 }],
    [200, 120, null],
  ];
  for (const mode of ["bars", "scope"]) {
    for (const tier of TIERS) {
      for (const [w, h, occl] of shapes) {
        assert.doesNotThrow(
          () => paint(mode, { w, h, occl, tier, seconds: 1 }),
          `${mode} @ ${tier} ${w}x${h}${occl ? " with artwork" : ""}`
        );
      }
    }
  }
});

test("the canvas bars draw the Papier ink row, and hold still under reduced motion", () => {
  // The fallback is what a device with no WebGL2 gets, and on Papier its strip
  // must be ink on the cream scrim. The geometry is the same row: 24 thick
  // beads at 390 px, the count the GL driver also decides.
  const preset = tierPreset("high");
  const scene = makeScene("bars", { layout: "strip", light: true, reducedMotion: true });
  scene.resize(390, 44, preset);
  assert.equal(scene.bars, 24, `the canvas row drew ${scene.bars} beads`);
  const pal = createPalette("neon");
  const g = recorder(390, 44);
  const dt = 1 / 60;
  for (let i = 0; i < 60; i++) {
    const f = makeFrame(i * dt);
    pal.update(f, dt);
    scene.update(f, dt);
    scene.draw(g, 390, 44, pal.out);
  }
  assert.ok(g.ink.length > 100, `the ink row drew almost nothing (${g.ink.length})`);
  // The ink path and the light path are the same scene: switching the theme
  // mid-play must not rebuild it or change the count.
  scene.setOptions({ light: false });
  scene.draw(g, 390, 44, pal.out);
  assert.equal(scene.bars, 24);
  scene.dispose();
});

test("the scope fills a 16:9 frame and keeps off the artwork", () => {
  const w = 1920;
  const h = 1080;
  const { ink } = paint("scope", { w, h });
  assert.ok(ink.length > 50, `drew almost nothing (${ink.length} points)`);
  const { minX, maxX, minY, maxY } = extent(ink);
  assert.ok(minX < w * 0.15 && maxX > w * 0.85, `left/right ${minX.toFixed(0)}..${maxX.toFixed(0)}`);
  assert.ok(minY < h * 0.2 && maxY > h * 0.8, `top/bottom ${minY.toFixed(0)}..${maxY.toFixed(0)}`);
  // A phone in the full-screen player: the cover is a square across most of
  // the width, dead centre. Anything drawn inside it is drawn for nobody.
  const pw = 420;
  const ph = 900;
  const side = pw * 0.72;
  const occl = { x: (pw - side) / 2, y: (ph - side) / 2, w: side, h: side };
  const r = paint("scope", { w: pw, h: ph, occl });
  const inside = r.ink.filter(
    (p) => Math.abs(p[0] - r.geom.cx) < r.geom.hw && Math.abs(p[1] - r.geom.cy) < r.geom.hh
  ).length;
  assert.ok(inside / r.ink.length < 0.25, `${((inside / r.ink.length) * 100) | 0}% is behind the cover`);
});

// --- the oscilloscope -------------------------------------------------------
//
// The two tests above ask the scope whether it fills the frame and keeps off
// the artwork, and it answers both. Neither of them would notice if it
// drew its graticule and nothing else — which is why what follows drives the
// SAMPLES through it and measures the trace on its own. The recorder tags each
// point with the compositing mode it was drawn under, and the scope's trace is
// the only thing it draws additively, so `lighter` separates the beam from the
// instrument it is drawn on.

const SCOPE_W = 1600;
const SCOPE_H = 900;

/**
 * Drive the scope and hand back, per frame, the TRACE's points split by lane.
 * `rows` is the trace read back one frame at a time, which is what a question
 * about stability needs; the generic `paint` above accumulates everything.
 */
function scopeRun({
  w = SCOPE_W, h = SCOPE_H, tier = "high", orientation = "horizontal",
  wave = "music", occl = null, seconds = 4, settle = 1,
} = {}) {
  const preset = tierPreset(tier);
  const geometry = createGeometry();
  geometry.set(w, h, occl);
  const scene = makeScene("scope", { preset, layout: "full", intensity: 0.8, orientation });
  scene.resize(w, h, preset, geometry.out);
  const pal = createPalette("neon");
  const dt = 1 / 60;
  const rows = [];
  const axis = orientation === "horizontal" ? 1 : 0;
  const cut = orientation === "horizontal" ? h / 2 : w / 2;
  for (let i = 0; i < Math.round(seconds / dt); i++) {
    const f = makeFrame(i * dt, { wave });
    pal.update(f, dt);
    scene.update(f, dt, geometry.out);
    const g = recorder(w, h);
    scene.draw(g, w, h, pal.out, geometry.out);
    if (i * dt < settle) continue; // let the auto-range and the lock settle
    const trace = g.ink.filter((pt) => pt[2] === "lighter");
    rows.push([
      trace.filter((pt) => pt[axis] < cut).map((pt) => pt[axis]),
      trace.filter((pt) => pt[axis] >= cut).map((pt) => pt[axis]),
    ]);
  }
  return { rows, scene, axis, geom: geometry.out };
}

const deflection = (rows, lane, zero) => {
  let max = 0;
  let sum = 0;
  let n = 0;
  for (const r of rows)
    for (const v of r[lane]) {
      const d = Math.abs(v - zero);
      if (d > max) max = d;
      sum += d;
      n++;
    }
  return { max, mean: n ? sum / n : 0, n };
};

test("the scope draws the signal, one trace per channel, and goes flat without one", () => {
  // Horizontal, no artwork: lane G is the top half (zero line at h/4), lane D
  // the bottom (3h/4).
  const zeroG = SCOPE_H / 4;
  const zeroD = (SCOPE_H * 3) / 4;

  const music = scopeRun({ wave: "music" });
  const g = deflection(music.rows, 0, zeroG);
  const d = deflection(music.rows, 1, zeroD);
  assert.ok(g.n > 1000 && d.n > 1000, "the scope drew almost no trace");
  // Measured on the bench material: 168.2 px and 196.1 px of peak deflection
  // against a lane half-height of 199 — the auto-range lets the loudest kicks
  // reach the rails on purpose, which is what a scope does.
  assert.ok(g.max > 60, `left trace barely moved (${g.max.toFixed(1)} px)`);
  assert.ok(d.max > 60, `right trace barely moved (${d.max.toFixed(1)} px)`);

  // THE TWO CHANNELS ARE READ SEPARATELY, which is the whole reason there are
  // two of them. The bench signal puts the lead almost three times louder on
  // the right, and between kicks that is most of what is in the window —
  // measured, the right trace's mean deflection is 1.26x the left's. A scope
  // fed one summed signal (which is all an AnalyserNode can give, hence the
  // dedicated tap in lib/audio/graph.js) would read exactly 1.00 here.
  const ratio = d.mean / g.mean;
  assert.ok(ratio > 1.1, `the two channels drew the same thing (ratio ${ratio.toFixed(3)})`);

  // Silence is a flat line ON the zero line, not an empty canvas: an
  // instrument showing nothing and an instrument that has stopped are
  // different things, and the auto-range must not invent a signal out of the
  // noise floor either.
  const quiet = scopeRun({ wave: "silent" });
  const qg = deflection(quiet.rows, 0, zeroG);
  assert.ok(qg.n > 1000, "the scope stopped drawing on silence");
  assert.ok(qg.max < 1, `silence deflected the trace by ${qg.max.toFixed(2)} px`);
  assert.equal(quiet.scene.locked < 0.05, true, "silence must not report a trigger lock");

  // ...and with no tap at all (an older projector feed, or before the graph
  // exists) it still draws its face rather than throwing or blanking.
  const none = scopeRun({ wave: null });
  assert.ok(deflection(none.rows, 0, zeroG).n > 1000, "no tap left the scope blank");
});

test("triggering is what holds the trace still", () => {
  // On a sustained tone a triggered trace must be the SAME trace every frame.
  // The reference is the same windows read from the end of the buffer, which
  // is exactly what a scope with no trigger shows.
  const tone = scopeRun({ wave: "tone", tier: "high", seconds: 3 });
  assert.ok(tone.scene.locked > 0.9, `no lock on a steady tone (${tone.scene.locked.toFixed(2)})`);

  const drift = (rows, lane) => {
    let sum = 0;
    let n = 0;
    for (let i = 1; i < rows.length; i++) {
      const a = rows[i - 1][lane];
      const b = rows[i][lane];
      const m = Math.min(a.length, b.length);
      for (let c = 0; c < m; c++) {
        sum += Math.abs(a[c] - b[c]);
        n++;
      }
    }
    return n ? sum / n : 0;
  };
  const moved = drift(tone.rows, 0);
  // Measured: 0.007 px triggered against 86.5 px free-running on this tone —
  // four orders of magnitude, and the difference between an instrument and a
  // waveform sliding off the side of the screen.
  assert.ok(moved < 2, `the trace slid ${moved.toFixed(3)} px a frame on a steady tone`);
});

test("the scope's precision climbs with the quality tier, and ultra is the exact one", () => {
  // Precision, not timebase: every tier shows the same slice of time. What
  // changes is how much of it survives to the screen.
  //
  // Measured on a 4K-wide frame, where the lane is wider than the tiers'
  // ceilings and each one's own limit is what bites: 256 / 512 / 2048 / 4044
  // columns. (On a 1600-wide frame high and ultra both reach one column per
  // pixel, which is as precise as a display can be asked to be.)
  const cols = TIERS.map((t) => scopeRun({ tier: t, w: 4096, h: 2160, seconds: 0.6, settle: 0 }).scene.cols);
  for (let i = 1; i < cols.length; i++)
    assert.ok(cols[i] > cols[i - 1], `${TIERS[i]} is no finer than ${TIERS[i - 1]} (${cols.join(" / ")})`);
  assert.ok(cols[3] >= 4000, `ultra capped at ${cols[3]} columns on a 4K lane`);

  // And the step that is NOT a column count: the sub-sample trigger. Below
  // `high` the window starts on a whole sample, so the trace shifts a whole
  // sample at a time — ~0.95 px on a 1920-wide lane, seen as a shimmer down
  // the whole trace. Measured on the steady tone: 1.03 px a frame at medium,
  // 0.007 px at high, a factor of 158.
  const drift = (run) => {
    const rows = run.rows;
    let sum = 0;
    let n = 0;
    for (let i = 1; i < rows.length; i++) {
      const a = rows[i - 1][0];
      const b = rows[i][0];
      const m = Math.min(a.length, b.length);
      for (let c = 0; c < m; c++) {
        sum += Math.abs(a[c] - b[c]);
        n++;
      }
    }
    return n ? sum / n : 0;
  };
  const coarse = drift(scopeRun({ wave: "tone", tier: "medium", seconds: 3 }));
  const fine = drift(scopeRun({ wave: "tone", tier: "high", seconds: 3 }));
  assert.ok(
    fine < coarse * 0.2,
    `the sub-sample trigger bought nothing (medium ${coarse.toFixed(3)} px, high ${fine.toFixed(3)} px)`
  );
});

test("the scope's orientation moves the lanes, and both keep off the artwork", () => {
  // Horizontal is two bands stacked; vertical is two columns side by side. The
  // claim is that the SAME signal lands somewhere else, not that a label
  // changed.
  const flat = paint("scope", { w: SCOPE_W, h: SCOPE_H, orientation: "horizontal" });
  const tall = paint("scope", { w: SCOPE_W, h: SCOPE_H, orientation: "vertical" });
  const a = signature(flat.ink, SCOPE_W, SCOPE_H);
  const b = signature(tall.ink, SCOPE_W, SCOPE_H);
  assert.ok(
    sigDistance(a, b) > 0.5,
    `the two orientations drew in the same places (${sigDistance(a, b).toFixed(2)})`
  );

  // Both have to route around the cover in the full-screen player — the
  // horizontal one into the strips above and below it, the vertical one into
  // the columns beside it. The generic test above only ever asked the default.
  const w = 420;
  const h = 900;
  const side = w * 0.72;
  const occl = { x: (w - side) / 2, y: (h - side) / 2, w: side, h: side };
  for (const orientation of ["horizontal", "vertical"]) {
    const r = paint("scope", { w, h, occl, orientation });
    const inside = r.ink.filter(
      (pt) => Math.abs(pt[0] - r.geom.cx) < r.geom.hw && Math.abs(pt[1] - r.geom.cy) < r.geom.hh
    ).length;
    assert.ok(r.ink.length > 50, `scope/${orientation}: drew almost nothing`);
    assert.ok(
      inside / r.ink.length < 0.25,
      `scope/${orientation}: ${((inside / r.ink.length) * 100) | 0}% of the drawing is behind the cover`
    );
  }
});
// --- the registry and the loader ---------------------------------------------

test("the mode registry stays consistent with what the scenes can do", () => {
  for (const m of MODES) {
    assert.equal(typeof m.label, "string");
    assert.ok(m.label.length > 0);
    if (m.id === "off") continue;
    assert.equal(typeof levelFor(m.id), "number");
  }
  // Eco mode means none, and a rhythm scene with the analysis off degrades
  // rather than rendering a dead canvas.
  assert.equal(effectiveMode("smart", true, true), "off");
  assert.equal(effectiveMode("smart", false, false), "aurora");
  assert.equal(effectiveMode("bars", false, false), "bars");
});

test("the scene loader hands back the right KIND of scene, on demand", async () => {
  // The app never imports a scene statically: `createScene` fetches the module
  // the first time a mode is asked for. The host reads `kind` to decide which
  // context to give the canvas — a canvas cannot switch from 2D to WebGL once
  // it has one — so the kind is part of the contract, not a detail.
  const opts = { preset: tierPreset("high"), layout: "full", intensity: 0.8 };
  for (const m of MODES) {
    const p = createScene(m.id, opts);
    if (m.id === "off") {
      assert.equal(p, null, "the off mode must not load a scene");
      continue;
    }
    assert.ok(p && typeof p.then === "function", `${m.id} did not return a promise`);
    const scene = await p;
    assert.equal(typeof scene?.update, "function", `${m.id} has no update()`);
    assert.equal(typeof scene?.draw, "function", `${m.id} has no draw()`);
    // Every mode is the GL engine now, the oscilloscope included (the scope
    // instrument, worlds/scope.js). The canvas scenes carry no `kind`: the
    // host reads anything that is not "gl" as a 2D scene.
    assert.equal(scene.kind === "gl" ? "gl" : "2d", "gl", `${m.id} is the wrong kind of scene`);
    scene.dispose?.();
  }
  const [a, b] = await Promise.all([createScene("smart", opts), createScene("smart", opts)]);
  assert.ok(a && b && a !== b, "each call must build its own scene");
  // Without WebGL2, each mode's canvas version: the scope for the scope, the
  // spectrum bars for everything else — both on the same Rust arithmetic.
  const fbScope = await createFallback("scope", opts);
  const fbBars = await createFallback("smart", opts);
  assert.notEqual(fbScope.kind, "gl");
  assert.notEqual(fbBars.kind, "gl");
  assert.equal(typeof fbScope.setOptions, "function", "the scope fallback takes the scope's settings");
  assert.ok("bars" in fbBars, "every other mode falls back to the bars");
  fbScope.dispose();
  fbBars.dispose();
});

test("the spectrum's band count is bounded, stable and pitches with the screen", async () => {
  // The Sillon row is one bead per 16 CSS px in the strip and one per 20.5 full
  // screen, under the tier's ceiling: a thicker row than the old one per 7 px,
  // a constant physical pitch on any display, and a count that only changes
  // when the window does. Measured here on the driver, which is the only place
  // the count is decided.
  const def = (await import("../src/lib/viz/worlds/spectrum.js")).default;
  const count = (layout, width, tier = "high") => {
    const state = new Float32Array(8);
    const driver = def.create({
      preset: tierPreset(tier),
      state,
      flash() {},
      params: { ...def.params },
      opts: { layout },
    });
    const height = layout === "strip" ? 40 : 360;
    driver.step(
      1 / 60,
      { ease: (v) => v, stamp: { drop: -1 } },
      { width, height, aspect: width / height, spec: new Float32Array(128) }
    );
    return state[3];
  };
  for (const tier of TIERS) {
    for (const [layout, width] of [["strip", 390], ["strip", 520], ["full", 640], ["full", 1440]]) {
      const n = count(layout, width, tier);
      assert.ok(n >= 18 && n <= tierPreset(tier).bars, `${tier} ${layout} ${width}: ${n} bands`);
      assert.equal(n, count(layout, width, tier), `${tier} ${layout} ${width}: the count moved on its own`);
    }
  }
  // The player's 390 px strip: 24 thick beads (the old row had 55 thin ones).
  assert.equal(count("strip", 390), 24);
  assert.equal(count("strip", 520), 33);
  assert.equal(count("full", 640), 31);
  assert.equal(count("full", 1440), 70);
  // A small device draws fewer beads, not thinner ones: the low tier caps it.
  assert.equal(count("full", 1440, "low"), 40);
});

test("every world in the loader is in the catalogue, and the reverse", async () => {
  // The loader (worlds/index.js: one literal import() per world, so each is its
  // own chunk) and the catalogue (worlds/catalogue.js: names only, no shader)
  // are separate files so that naming a world costs nothing. They have to
  // agree, or a gallery offers a world that cannot load.
  assert.deepEqual([...worldIds()].sort(), Object.keys(WORLD_META).sort());
  const groups = new Set(GROUPS.map((g) => g.id));
  const used = new Set();
  for (const id of worldIds()) {
    const meta = WORLD_META[id];
    assert.ok(groups.has(meta.group), `${id}: unknown shelf "${meta.group}"`);
    used.add(meta.group);
    assert.ok(meta.label, `${id}: no label`);
    const def = await loadWorld(id);
    assert.equal(def?.id, id, `${id}: the module calls itself "${def?.id}"`);
  }
  for (const g of GROUPS) assert.ok(used.has(g.id), `shelf "${g.id}" is empty`);
  assert.ok(!hasWorld("no-such-world"));
  // The floor under the skins names real worlds too.
  for (const a of ["sustain", "voice", "groove", "hard", "rock", "?"]) assert.ok(WORLD_META[worldFor(a)], a);
});

// --- the shaders, as far as they can be checked without a GPU ------------------
//
// A shader that does not compile is a black screen and a line in the console of
// a browser nobody is looking at — a projector at a party. The render bench
// compiles every world for real; these are the mistakes it has actually caught,
// turned into checks that run in a second on every `npm test`.

const WORLDS = [];
for (const id of worldIds()) WORLDS.push([id, await loadWorld(id)]);
// The instruments (the scope) are GL scenes too, held to the same shader rules;
// they are nobody's genre, so the catalogue's own checks leave them out.
const INSTRUMENTS = [];
for (const id of instrumentIds()) INSTRUMENTS.push([id, await loadWorld(id)]);
const { paramDefines } = await import("../src/lib/viz/scenes/gl.js");

// GLSL ES 3.00's reserved words and the built-in functions a world calls.
// Declaring a local with one of these names is legal right up to the first
// call it shadows — `float all = …` compiled, and then `all(lessThan(…))` a few
// lines down did not, and the world rendered black.
const RESERVED = new Set(
  (
    "attribute const uniform varying layout centroid flat smooth break continue do for while switch " +
    "case default if else in out inout float int uint void bool true false invariant discard return " +
    "struct precision highp mediump lowp coherent volatile restrict readonly writeonly resource " +
    "atomic_uint noperspective patch sample subroutine common partition active asm class union enum " +
    "typedef template this goto inline noinline public static extern external interface long short " +
    "double half fixed unsigned superp input output filter sizeof cast namespace using " +
    "all any not min max step mix length distance dot cross normalize reflect refract sign floor ceil " +
    "round trunc fract mod modf clamp smoothstep abs sin cos tan asin acos atan pow exp exp2 log log2 " +
    "sqrt inversesqrt texture textureLod texelFetch equal notEqual lessThan greaterThan lessThanEqual " +
    "greaterThanEqual radians degrees transpose inverse determinant outerProduct matrixCompMult " +
    "faceforward isnan isinf fwidth dFdx dFdy"
  ).split(/\s+/)
);

function glslBodies(def) {
  const out = [["fragment", def.fragment]];
  if (def.particles) {
    out.push(["particle vertex", def.particles.vertex]);
    out.push(["particle fragment", def.particles.fragment]);
  }
  return out;
}

test("every world's shader asks only for what the engine declares", () => {
  const declared = new Set(WORLD_UNIFORMS.match(/\bu[A-Z]\w*/g));
  assert.ok(INSTRUMENTS.length > 0, "the scope instrument is checked too");
  for (const [id, def] of [...WORLDS, ...INSTRUMENTS]) {
    const names = Object.keys(def.params || {});
    assert.ok(names.length <= 16, `${id}: ${names.length} params, the engine packs 16`);
    for (const [k, v] of Object.entries(def.params || {}))
      assert.ok(Number.isFinite(v), `${id}: param ${k} is not a number`);
    const defines = paramDefines(names);
    for (const [where, body] of glslBodies(def)) {
      assert.equal(typeof body, "string", `${id}: no ${where}`);
      // Every P_ a world mentions is one of its own parameters: an undefined
      // define is a compile error, and a skin can only ever set a param that
      // exists.
      for (const p of new Set(body.match(/\bP_[A-Z0-9_]+\b/g) || []))
        assert.ok(defines.includes(`#define ${p} `), `${id} (${where}): ${p} is not one of its params`);
      for (const u of new Set(body.match(/\bu[A-Z]\w*/g) || []))
        assert.ok(declared.has(u), `${id} (${where}): uniform ${u} does not exist`);
      for (const [open, close] of [["{", "}"], ["(", ")"]]) {
        const a = body.split(open).length;
        const b = body.split(close).length;
        assert.equal(a, b, `${id} (${where}): unbalanced ${open}${close}`);
      }
      for (const m of body.matchAll(/\b(?:float|int|uint|bool|vec[234]|ivec[234]|mat[234])\s+([A-Za-z_]\w*)\s*[=;,)[]/g))
        assert.ok(!RESERVED.has(m[1]), `${id} (${where}): a variable called "${m[1]}" shadows GLSL`);
    }
    // And the whole thing assembles: every chunk it uses exists.
    assert.doesNotThrow(() => worldFragment(def.fragment, def.uses || [], defines), `${id}: fragment`);
    if (def.particles) {
      assert.match(def.particles.vertex, /void particle\(int id, out vec2 pos, out vec2 axis, out float width, out vec4 col, out float kind\)/, `${id}: particle() signature`);
      assert.match(def.particles.fragment, /vec4 sprite\(vec2 \w+, vec4 \w+, float \w+\)/, `${id}: sprite() signature`);
      assert.doesNotThrow(() => particleVertex(def.particles.vertex, def.particles.uses || [], defines));
      assert.doesNotThrow(() => particleFragment(def.particles.fragment, def.particles.uses || [], defines));
    }
  }
});

test("a world cannot see wall-clock time, because the engine gives it none", () => {
  // Every clock a shader has is counted in beats, bars and phrases (uClock),
  // so a rate written in a shader is a rate per beat by construction and the
  // picture plays the track at any tempo. That only holds while there is no
  // seconds uniform to reach for; this is the line.
  const declared = WORLD_UNIFORMS.match(/\bu[A-Z]\w*/g);
  for (const u of declared) assert.doesNotMatch(u, /time|sec|second/i, `${u} looks like wall-clock time`);
  for (const [id, def] of WORLDS)
    for (const [where, body] of glslBodies(def))
      assert.doesNotMatch(body, /\biTime\b|\buTime\b|\bu_time\b/, `${id} (${where}) reaches for a seconds clock`);
});

// --- the skins -----------------------------------------------------------------

const EXTRA = (() => {
  const py = readFileSync(new URL("../../neezer/deezer/analysis.py", import.meta.url), "utf8");
  const block = py.match(/EXTRA_GENRES = \[([\s\S]*?)\n\]/);
  assert.ok(block, "EXTRA_GENRES is no longer where this test looks for it");
  return [...block[1].matchAll(/\("([^"]+)",\s*"([a-z]+)"\)/g)].map((m) => [m[1], m[2]]);
})();

test("a transparent row's bloom is weighted by coverage, and the strip is printed flat", () => {
  // The band that sat behind the player's row came from these two lines being
  // absent, measured on the real player: the spectrum writes a STRAIGHT colour
  // across the whole canvas with an alpha of zero outside its capsules, the
  // bloom sampled `.rgb` (so it bloomed a full-frame field of colour), and the
  // final pass turned that bloom's luminance into ALPHA — 100% of the canvas
  // had a non-zero alpha on Encre, tinted by the sleeve. Papier, where the
  // bloom was already off, read 0. Nothing here can be seen without a GPU, so
  // what is pinned is the two rules the pixels depend on.
  assert.match(PREFILTER_FS, /vec3\s+c\s*=\s*t\.rgb\s*\*\s*t\.a/, "the bloom no longer weights by coverage");
  // The flat branch of the final pass: the strip's own value, its own coverage,
  // and an early return BEFORE the AgX grade and the grain.
  const flat = FINAL_FS.slice(FINAL_FS.indexOf("uPost2.y > 1.5"));
  assert.ok(flat.length > 0, "the final pass has no flat-strip branch");
  assert.match(flat, /fragColor = vec4\(flatC \* s\.a, s\.a\)/, "the flat strip is not premultiplied coverage");
  assert.ok(
    flat.indexOf("return") < flat.indexOf("agx("),
    "the flat strip still runs through the tone map"
  );
  assert.match(FINAL_FS, /uniform float uHead/, "the flat strip cannot undo the 8-bit headroom");
});

test("every family the classifier can name has its own skin, by id and by label", () => {
  for (const f of FAMILY_LIST) {
    assert.equal(skinId(f.id), f.id, `family "${f.id}" has no row of its own`);
    // The classifier's French label is what a served verdict or the readout
    // hands over, and it must land on the same row.
    assert.equal(skinId(f.label), f.id, `label "${f.label}" resolves to "${skinId(f.label)}"`);
  }
});

test("every genre the studio offers is a genre the animation can dress", () => {
  // The two halves of the vocabulary are written in different languages and
  // have to agree: `analysis.known_genres()` is what the genre studio offers an
  // admin to tag with, and this table is what the animation does with the tag
  // that comes back. A label that only reaches the archetype fallback is a
  // track tagged carefully and then animated generically.
  assert.ok(EXTRA.length > 100, `only ${EXTRA.length} extra genres parsed`);
  const orphans = EXTRA.filter(([label]) => !SKINS[skinId(label)] || !skinId(label)).map(([l]) => l);
  assert.deepEqual(orphans, [], `no skin for: ${orphans.join(", ")}`);
  const arches = new Set(EXTRA.map((r) => r[1]));
  for (const a of arches) assert.ok(skinId("a name nothing will ever match", a), `no fallback for ${a}`);
});

test("every genre the studio offers reaches the analyser by its name", () => {
  // The studio's labels are what a tag and the trained model hand back, and a
  // genre somebody chose has to steer the analysis: its family's look, its bar
  // model, its tempo band. Resolving by a WORD of the name is what used to
  // happen, and it read "Garage rock" as UK garage and "Drumfunk" as funk, so
  // every label must be known whole.
  const all = [...FAMILY_LIST.map((f) => [f.label, f.archetype]), ...EXTRA];
  for (const [label] of all) {
    const g = genreOf(label);
    assert.ok(g && g.by === "name", `"${label}" is not known by name (${JSON.stringify(g)})`);
    assert.ok(familyLook(label), `"${label}" carries no look`);
    if (g.band) assert.ok(g.band[0] >= 40 && g.band[0] < g.band[1] && g.band[1] <= 320, `"${label}" band ${g.band}`);
  }
  for (const f of FAMILY_LIST) {
    assert.equal(genreOf(f.id).family, f.id);
    assert.equal(genreOf(f.label).family, f.id, `label "${f.label}"`);
  }
});

test("a sub-genre keeps its family's groove unless it is built otherwise, and its own tempo where it has one", () => {
  // Tagged hard dance reads its bar from the rolls, as its family does.
  for (const n of ["Schranz", "Gabber", "Rawphase", "Tekk", "Terrorcore", "Euphoric hardstyle"])
    assert.equal(grooveOf(n), GROOVES.hard, n);
  assert.equal(grooveOf("Deep house"), GROOVES.four);
  assert.equal(grooveOf("Liquid DnB"), GROOVES.broken);
  assert.equal(grooveOf("Hardcore punk"), GROOVES.live, "a punk band, not a hardcore kick");
  // ...and the exceptions are exceptions: gqom breaks afro house's kick, and
  // hardbass is a plain four under a hard techno sound.
  assert.equal(grooveOf("Gqom"), GROOVES.broken);
  assert.equal(grooveOf("Hardbass"), GROOVES.four);
  // The bands exist to rule an octave out, so the traps are octaves. A 100 BPM
  // nu metal groove under metal's 120-200 would be read at 200.
  const inside = (n, bpm) => {
    const r = tempoRangeFor(n);
    return !!r && bpm >= r[0] && bpm <= r[1];
  };
  assert.ok(inside("Nu metal", 100) && !inside("Nu metal", 200));
  assert.ok(inside("Doom", 70) && !inside("Doom", 140));
  assert.ok(inside("Hard house", 145) && !inside("Hard house", 125));
  assert.ok(inside("Hitech", 190) && !inside("Psytrance", 190));
  assert.ok(inside("Garage rock", 150) && !inside("UK garage", 150));
  assert.ok(inside("Drumfunk", 172) && !inside("Funk", 172));
  // A name that spans octaves gets no band rather than a wrong one.
  assert.equal(tempoRangeFor("Dembow"), null);
  assert.equal(tempoRangeFor("IDM"), null);
  // A hand-typed tag still resolves from its words: the specific one first...
  assert.equal(genreOf("uptempo hardcore").family, "uptempo");
  // ...and a name without separators by its longest known part.
  assert.equal(genreOf("hardtechnoedit").family, "hardtechno");
  assert.equal(genreOf("a name nothing will ever match"), null);
  assert.equal(grooveOf("a name nothing will ever match"), GROOVES.unknown);
});

test("every skin names a real world, with parameters that world reads", () => {
  const params = Object.fromEntries(WORLDS.map(([id, def]) => [id, def.params || {}]));
  for (const [id, s] of Object.entries(SKINS)) {
    assert.ok(WORLD_META[s.world], `${id}: unknown world "${s.world}"`);
    for (const [k, v] of Object.entries(s.p || {})) {
      // A key the world does not declare is silently ignored by the engine,
      // which is exactly how a skin ends up looking like its anchor row.
      assert.ok(k in params[s.world], `${id}: "${s.world}" has no parameter "${k}"`);
      assert.ok(Number.isFinite(v), `${id}: ${k} is not a number`);
    }
    for (const k of ["hue", "sat", "light", "speed", "energy"])
      if (k in s) assert.ok(Number.isFinite(s[k]), `${id}: ${k} is not a number`);
    for (const k of ["sat", "light", "speed", "energy"]) if (k in s) assert.ok(s[k] > 0, `${id}: ${k} <= 0`);
  }
  for (const [from, to] of Object.entries(ALIASES)) assert.ok(SKINS[to], `alias ${from} -> missing ${to}`);
});

test("every world is somebody's genre, and the catalogue is far wider than the detector", () => {
  // A world no genre resolves to is one "auto" never shows: forty-seven
  // worlds were written so that the genres would stop sharing five.
  const used = new Set(Object.values(SKINS).map((s) => s.world));
  const unused = Object.keys(WORLD_META).filter((w) => !used.has(w));
  assert.deepEqual(unused, [], `no genre resolves to: ${unused.join(", ")}`);
  // Measured when the catalogue was written: 226 rows across 49 worlds, the
  // busiest world (slices, every break-driven genre) carrying 12.
  assert.ok(skinCount() >= 220, `only ${skinCount()} genres are dressed`);
  const per = {};
  for (const s of Object.values(SKINS)) per[s.world] = (per[s.world] || 0) + 1;
  assert.ok(Math.max(...Object.values(per)) <= 14, `one world carries ${Math.max(...Object.values(per))} genres`);
});

test("a name arrives however it likes and still finds its skin", () => {
  assert.equal(skinId("Drum & Bass"), "dnb");
  assert.equal(skinId("drum and bass"), "dnb");
  assert.equal(skinId("PSY"), "psytrance");
  assert.equal(skinId("Cordes / classique"), "strings");
  assert.equal(skinId("Hard Ping-Pong"), "hardpingpong");
  assert.equal(skinId("Deutscher Krach"), "krach");
  assert.equal(skinId("Électronique"), "electronic");
  assert.equal(skinId("musique classique"), "classical");
  assert.equal(skinId("8-bit"), "eightbit");
  assert.equal(skinId("UK garage"), "garage");
  // Unknown names still dress the scene rather than leaving it blank, and the
  // floor agrees with the catalogue's own archetype fallback.
  for (const a of ["sustain", "voice", "groove", "hard", "rock"])
    assert.equal(skinFor("a genre nobody has named", a).world, worldFor(a), a);
  assert.ok(WORLD_META[skinFor("total nonsense").world]);
});

test("neighbours sharing a world are told apart by a SHAPE switch, not a brightness", () => {
  // The pairs the catalogue exists for: same world, and at least one parameter
  // that changes what the motif IS rather than how much of it there is.
  const SHAPE = {
    kaleido: ["mirror", "web", "sectors", "iter"],
    wobble: ["wave"],
    lasers: ["raw"],
    forge: ["anvils"],
    microwave: ["dish"],
    slices: ["axis", "angle"],
    shatter: ["shards"],
    tunnel: ["sides", "twist", "dash", "dir"],
    bounce: ["twins"],
    vinyl: ["rpm", "arm"],
    carnival: ["rings"],
  };
  const pairs = [
    ["goa", "darkpsy"], ["dubstep", "brostep"], ["brostep", "riddim"], ["hardstyle", "euphorichardstyle"], ["rawstyle", "rawphase"],
    ["krach", "zaag"], ["zaag", "uptempo"], ["krach", "uptempo"],
    ["dnb", "jungle"], ["gabber", "speedcore"], ["techno", "ebm"], ["jumpstyle", "hardbass"],
    ["boombap", "swing"], ["salsa", "dembow"], ["psytrance", "hitech"],
  ];
  for (const [a, b] of pairs) {
    const sa = SKINS[a];
    const sb = SKINS[b];
    assert.equal(sa.world, sb.world, `${a}/${b} no longer share a world`);
    const keys = SHAPE[sa.world];
    const differ = keys.some((k) => (sa.p?.[k] ?? "default") !== (sb.p?.[k] ?? "default"));
    assert.ok(differ, `${a} and ${b} differ on no shape switch of ${sa.world} (${keys.join(", ")})`);
  }
});

// --- the sleeve's own colours -------------------------------------------------
//
// A row of bars runs across the whole width of the frame, so its colours come
// from the whole width of the artwork: the cover decoder answers with the
// sleeve's horizontal zones (lib/coverpalette.js, one decode), the palette walks
// them, and BOTH paths — the canvas fallback and the GL world — draw with the
// same numbers. What is pinned here is the walk and the two consumers; the
// extraction itself is `test/coverpalette.test.mjs`'s.

const hslOf = (s) => {
  const m = String(s).match(/hsla?\(\s*([-\d.]+)\s*,\s*([\d.]+)%\s*,\s*([\d.]+)%\s*,\s*([\d.]+)\s*\)/);
  return m ? { h: +m[1], s: +m[2] / 100, l: +m[3] / 100, a: +m[4] } : null;
};

// A canvas that keeps the FILL STYLE in force at every fill: the bars' colours
// are exactly what this catches, and the shared recorder does not look at them
// (it is about where things are drawn). `beads` is one entry per fill call, so
// the strip's three layers per band arrive in the order the scene drew them.
function colourRecorder(w, h) {
  const out = { beads: [], styles: [], clears: 0, fills: 0, full: 0 };
  let last = "";
  const g = {
    canvas: { width: w, height: h },
    get fillStyle() {
      return last;
    },
    set fillStyle(v) {
      last = v;
    },
    clearRect() {
      out.clears++;
    },
    fillRect(x, y, rw, rh) {
      if (x <= 0.5 && y <= 0.5 && rw >= w - 1 && rh >= h - 1) out.full++;
    },
    beginPath() {},
    closePath() {},
    moveTo() {},
    lineTo() {},
    quadraticCurveTo() {},
    fill() {
      out.fills++;
      out.styles.push(last);
      const c = hslOf(last);
      if (c) out.beads.push(c);
    },
  };
  out.g = g;
  return out;
}

const blueGreenOrange = [
  [30, 80, 220],
  [40, 200, 120],
  [250, 150, 40],
];

test("the palette walks the cover's zones, and only when the source is the cover", () => {
  const pal = createPalette("cover");
  const frame = makeFrame(0);
  pal.setCover([250, 140, 40]);
  pal.setZones(blueGreenOrange);
  pal.update(frame, 1 / 60);
  assert.equal(pal.out.hasZones, true, "the zones were not taken");
  assert.equal(pal.out.zones.length, 21, "seven zones of h, s, l");
  assert.equal(pal.out.zoneRgb.length, 28, "seven vec4 the GL engine can upload");

  // The ends of the row are the two ends of the sleeve. The hue carries the
  // tempo's drift and the genre's lean, both applied after one frame, so the
  // comparison allows for that (it is a fraction of a degree on the first).
  const at0 = [0, 0, 0];
  const at1 = [0, 0, 0];
  assert.equal(pal.colourAt(0, at0), true);
  assert.equal(pal.colourAt(1, at1), true);
  assert.ok(Math.abs(at0[0] - 226) < 6, `the left end is ${at0[0].toFixed(1)}°`);
  assert.ok(Math.abs(at1[0] - 36) < 6, `the right end is ${at1[0].toFixed(1)}°`);
  // Three anchors resampled to seven stops: the middle of the row is the
  // middle of the sleeve, not an interpolation of the two ends.
  const mid = [0, 0, 0];
  pal.colourAt(0.5, mid);
  assert.ok(Math.abs(mid[0] - 150) < 8, `the middle is ${mid[0].toFixed(1)}°`);
  // The walk is monotonic in value: no zone is outside the row's own band.
  for (let i = 0; i < 7; i++) {
    const l = pal.out.zones[i * 3 + 2];
    assert.ok(l >= 0.5 && l <= 0.72, `zone ${i} has value ${l}`);
    const rgb = Array.from(pal.out.zoneRgb.subarray(i * 4, i * 4 + 3));
    assert.ok(rgb.every((c) => c >= 0 && c <= 1), `zone ${i} is not sRGB: ${rgb}`);
    assert.equal(pal.out.zoneRgb[i * 4 + 3], 1);
  }

  // The PALETTE SOURCE is the user's choice: a neon row is not repainted by the
  // sleeve, and going back to the cover brings the zones back untouched.
  pal.setMode("neon");
  pal.update(frame, 1 / 60);
  assert.equal(pal.out.hasZones, false, "the zones overrode the chosen source");
  const none = [0, 0, 0];
  assert.equal(pal.colourAt(0, none), false);
  pal.setMode("cover");
  pal.update(frame, 1 / 60);
  assert.equal(pal.out.hasZones, true, "the zones did not come back");

  // A grey band borrows the sleeve's hue: a row of grey beads on a coloured
  // sleeve reads as a bug, not as a palette.
  const grey = createPalette("cover");
  grey.setCover([255, 0, 0]);
  grey.setZones([
    [128, 128, 128],
    [136, 136, 136],
    [120, 120, 120],
  ]);
  grey.update(frame, 1 / 60);
  const g0 = [0, 0, 0];
  grey.colourAt(0, g0);
  assert.ok(Math.min(Math.abs(g0[0]), 360 - Math.abs(g0[0])) < 6, `a grey sleeve walks ${g0[0]}°`);
  assert.ok(g0[1] > 0.34, "a washed-out sleeve still has colour");
  // A sleeve with no hue at all gets the Sillon accent, never hue 0 (red).
  const greyCover = createPalette("cover");
  greyCover.setCover([128, 128, 128]);
  greyCover.update(frame, 1 / 60);
  assert.ok(Math.abs(greyCover.out.hue - 73.8) < 2, `a grey sleeve walks ${greyCover.out.hue}°`);
  // ...and a sleeve that cannot be read at all comes back to it too, rather
  // than keeping the previous track's colour (the documented fallback).
  greyCover.setCover([20, 60, 200]);
  for (let i = 0; i < 40; i++) greyCover.update(frame, 1 / 60);
  assert.ok(greyCover.out.hue > 150, `a blue sleeve walks ${greyCover.out.hue}°`);
  greyCover.setCover(null);
  for (let i = 0; i < 180; i++) greyCover.update(frame, 1 / 60);
  assert.ok(Math.abs(greyCover.out.hue - 73.8) < 16, `a lost sleeve walks ${greyCover.out.hue}°`);
});

test("the sleeve's zones are the colours it CONTAINS, not its column means", () => {
  // The decoder hands back a 3x3 grid of means. Averaging a column's top with
  // its bottom is how a blue sky over an amber field comes out olive — a
  // colour that is nowhere in the artwork and that sits wrong on the blurred
  // backdrop. The walk takes the most SATURATED cell of each column instead.
  const cells = [
    [16, 22, 74], // top-left: night sky
    [230, 226, 214], // top-middle: cream
    [10, 12, 20], // top-right: black
    [22, 54, 160], // middle-left: blue
    [200, 60, 48], // middle-middle: red
    [255, 183, 3], // middle-right: amber
    [18, 40, 120], // bottom-left: blue
    [160, 60, 50], // bottom-middle: red
    [250, 170, 20], // bottom-right: amber
  ];
  const decoded = {
    cells,
    zones: { left: [20, 40, 120], center: [180, 110, 100], right: [140, 120, 60] },
    middles: [cells[3], cells[4], cells[5]],
    average: [120, 100, 90],
    dominant: [22, 54, 160],
  };
  const zones = zonesOfPalette(decoded);
  assert.equal(zones.length, 3);
  // Each zone is one of its column's cells — a colour the sleeve actually has —
  // and never the column mean (which for these is 22,40,120 / 197,115,104 /
  // 140,120,60: a muddy set that appears nowhere in the artwork).
  for (let c = 0; c < 3; c++) {
    const column = [cells[c], cells[3 + c], cells[6 + c]];
    assert.ok(
      column.some((cell) => cell[0] === zones[c][0] && cell[1] === zones[c][1] && cell[2] === zones[c][2]),
      `zone ${c} is ${zones[c]}, which is not one of its column's colours`
    );
  }
  // ...and it is the usable one: blue, red, amber, not the darkest cell.
  assert.ok(zones[0][2] > zones[0][0], "the left zone is not the blue the sleeve has");
  assert.ok(zones[1][0] > zones[1][1] + 60, "the middle zone is not the red the sleeve has");
  assert.ok(zones[2][0] > zones[2][1] && zones[2][1] > zones[2][2], "the right zone is not the amber");
  // A decoder with no grid keeps working: columns, then the middle row, then
  // nothing at all.
  assert.deepEqual(zonesOfPalette({ zones: { left: [1, 2, 3], center: [4, 5, 6], right: [7, 8, 9] } }), [
    [1, 2, 3],
    [4, 5, 6],
    [7, 8, 9],
  ]);
  assert.deepEqual(zonesOfPalette({ middles: [[4, 5, 6]] }), [[4, 5, 6]]);
  assert.equal(zonesOfPalette(null), null);
  assert.equal(zonesOfPalette({}), null);
});

test("the canvas bars take their colours from the sleeve, band by band", () => {
  const preset = tierPreset("high");
  const W = 520;
  const H = 56;
  const run = ({ zones = true, eco = false, light = false, solid = "" } = {}) => {
    const scene = makeScene("bars", { layout: "strip", light, eco, solid });
    scene.resize(W, H, preset);
    const pal = createPalette("cover");
    if (zones) {
      pal.setCover([250, 140, 40]);
      pal.setZones(blueGreenOrange);
    } else {
      pal.setCover([250, 140, 40]);
    }
    const rec = colourRecorder(W, H);
    const dt = 1 / 60;
    let frame;
    for (let i = 0; i < 30; i++) {
      frame = makeFrame(i * dt);
      pal.update(frame, dt);
      scene.update(frame, dt);
    }
    scene.draw(rec.g, W, H, pal.out);
    const bars = scene.bars;
    scene.dispose();
    return { rec, bars, pal };
  };

  const zoned = run({ zones: true });
  assert.equal(zoned.rec.clears, 1, "the strip must clear, never wash");
  assert.equal(zoned.rec.full, 0, "the strip drew a field");
  // One rounded shape per bead at least (the strip draws the body, its lit
  // inside and the axis core, in that order), and the hues walk left -> right.
  assert.ok(zoned.rec.beads.length >= zoned.bars * 3 - 2, `${zoned.rec.beads.length} fills for ${zoned.bars} beads`);
  const bodies = zoned.rec.beads.filter((_, i) => i % 3 === 0);
  assert.ok(bodies.length >= zoned.bars - 1, `${bodies.length} bodies for ${zoned.bars} beads`);
  const leftHue = bodies[0].h;
  const rightHue = bodies[bodies.length - 1].h;
  assert.ok(Math.abs(leftHue - 234) < 12, `the bass end is ${leftHue.toFixed(0)}°`);
  assert.ok(Math.abs(rightHue - 44) < 12, `the treble end is ${rightHue.toFixed(0)}°`);
  const distinct = new Set(bodies.map((b) => Math.round(b.h / 10)));
  assert.ok(distinct.size >= 4, `only ${distinct.size} hues across the row`);
  // The middle of the row is the middle of the sleeve.
  const middle = bodies[Math.floor(bodies.length / 2)];
  assert.ok(Math.abs(middle.h - 158) < 16, `the middle of the row is ${middle.h.toFixed(0)}°`);
  // The value stays in the row's band: a dark sleeve's zone is a deeper bead,
  // never a black one.
  for (const b of bodies) assert.ok(b.l >= 0.48 && b.l <= 0.73, `a body has value ${b.l}`);

  // No zones (no artwork, or another palette source): the palette's own
  // low -> high walk, which is what the row drew before the sleeve existed.
  const plain = run({ zones: false });
  const plainBodies = plain.rec.beads.filter((_, i) => i % 3 === 0);
  const pLeft = plainBodies[0].h;
  const pRight = plainBodies[plainBodies.length - 1].h;
  assert.ok(Math.abs(pLeft - (plain.pal.out.low + 14)) < 0.5, "the fallback walk changed");
  assert.ok(Math.abs(pRight - (plain.pal.out.high + 14)) < 0.5, "the fallback walk changed");

  // Eco: the same row in ONE flat fill per bead — no inside, no axis core, no
  // peak ghost — and still no field behind it.
  const eco = run({ zones: true, eco: true });
  assert.equal(eco.rec.full, 0, "eco drew a field");
  assert.equal(eco.rec.beads.length, eco.bars, `eco drew ${eco.rec.beads.length} shapes for ${eco.bars} beads`);
  const alphas = new Set(eco.rec.beads.map((b) => b.a));
  assert.equal(alphas.size, 1, "eco's row is not flat");
  const ecoHues = new Set(eco.rec.beads.map((b) => Math.round(b.h / 10)));
  assert.ok(ecoHues.size >= 4, "eco's row lost the sleeve's colours");
});

test("the player's strip is ONE flat colour, and it is the theme's accent", () => {
  // What the person asked for: the strip is a caption in the app's own readable
  // accent — not the sleeve's salmon-to-red walk, not a warm tip, not a pale
  // core, not a level-to-level brightness. One shape per capsule, one colour,
  // opacity 1; the music moves the height and the width only.
  const preset = tierPreset("high");
  const W = 520;
  const H = 56;
  const ACCENT = "#d2ff3c";
  const scene = makeScene("bars", { layout: "strip", solid: ACCENT });
  scene.resize(W, H, preset);
  const pal = createPalette("cover");
  pal.setCover([250, 140, 40]);
  pal.setZones(blueGreenOrange);
  const rec = colourRecorder(W, H);
  const dt = 1 / 60;
  let frame;
  for (let i = 0; i < 30; i++) {
    frame = makeFrame(i * dt);
    pal.update(frame, dt);
    scene.update(frame, dt);
  }
  scene.draw(rec.g, W, H, pal.out);
  assert.equal(rec.clears, 1, "the strip must clear, never wash");
  assert.equal(rec.full, 0, "the strip drew a field behind the row");
  assert.equal(rec.fills, scene.bars, `${rec.fills} fills for ${scene.bars} beads: not one shape per bead`);
  assert.deepEqual(new Set(rec.styles), new Set([ACCENT]), "the row is not one colour");

  // THE THEME CHANGES WITHOUT A REBUILD: the host pushes a new accent into the
  // live scene (setOptions), so a theme switch or a custom accent repaints the
  // row on the next frame instead of resetting every envelope.
  const CYAN = "#00e5ff";
  scene.setOptions({ solid: CYAN });
  const rec2 = colourRecorder(W, H);
  scene.draw(rec2.g, W, H, pal.out);
  assert.equal(rec2.styles.length, scene.bars, "the live accent change lost a shape");
  assert.deepEqual(new Set(rec2.styles), new Set([CYAN]), "the live accent did not reach the row");

  // ECO IS THE SAME COLOUR AND THE SAME ROW, with nothing on top: the flat
  // accent is the cheapest picture, not a different one.
  const ecoScene = makeScene("bars", { layout: "strip", solid: CYAN, eco: true, reducedMotion: true });
  ecoScene.resize(W, H, preset);
  const recEco = colourRecorder(W, H);
  for (let i = 0; i < 30; i++) {
    frame = makeFrame(i * dt);
    pal.update(frame, dt);
    ecoScene.update(frame, dt);
  }
  ecoScene.draw(recEco.g, W, H, pal.out);
  assert.equal(recEco.full, 0, "eco drew a field");
  assert.equal(recEco.fills, ecoScene.bars, "eco's flat row is not one shape per bead");
  assert.deepEqual(new Set(recEco.styles), new Set([CYAN]), "eco changed the theme's colour");

  // FULL SCREEN IS NOT THE STRIP: the same scene, given the same accent, keeps
  // the sleeve's colours — the accent is the player's strip only.
  const fullScene = makeScene("bars", { layout: "full", solid: ACCENT });
  fullScene.resize(640, 360, preset);
  const recFull = colourRecorder(640, 360);
  for (let i = 0; i < 30; i++) {
    frame = makeFrame(i * dt);
    pal.update(frame, dt);
    fullScene.update(frame, dt);
  }
  fullScene.draw(recFull.g, 640, 360, pal.out);
  assert.ok(recFull.fills > fullScene.bars, "the full-screen row lost its layers");
  assert.ok(
    recFull.styles.some((s) => /^hsl/.test(s)),
    "the full-screen row stopped using the sleeve's palette"
  );
  scene.dispose();
  ecoScene.dispose();
  fullScene.dispose();
});

test("the GL engine gives the worlds the sleeve's zones, and the row is transparent", async () => {
  // The GL half has no GPU here, so what is pinned is what the ENGINE hands the
  // world: the zone uniforms (28 floats), the transparency flag, the eco grade,
  // and the spectrum's own trail parameter at zero in eco.
  const { createGLScene } = await import("../src/lib/viz/scenes/gl.js");
  const frame = makeFrame(0);
  let simT = 0;

  async function run({ zones = true, eco = false, layout = "full", solid = "" } = {}) {
    const seen = { look: null, uniforms: {} };
    const h = {
      f: (n, v) => (seen.uniforms[n] = v),
      v2: () => {},
      v4: (n, ...v) => (seen.uniforms[n] = v),
      v4a: (n, arr) => (seen.uniforms[n] = Array.from(arr)),
      use: () => {},
      ready: true,
    };
    const target = { a: true, out: null, fresh: true, dispose() {} };
    const renderer = {
      lost: false,
      gpuMs: 0,
      program: () => ({ ready: true, use: () => {}, failed: false }),
      worldTarget: () => target,
      dataTexture: () => ({ upload() {}, dispose() {} }),
      setSize() {},
      setCover() {},
      clear() {},
      beginFrame: () => true,
      drawWorld: (_h, _t, fill) => {
        fill(h);
        return true;
      },
      present: (_a, _b, _t, _seed, look) => {
        seen.look = { ...look };
        return true;
      },
    };
    const scene = createGLScene({
      preset: tierPreset("high"),
      fixed: "spectrum",
      now: () => simT,
      fps: 60,
      flash: "soft",
      layout,
      solid,
      eco,
      core: vizCoreFromBytes(readFileSync(new URL("../src/lib/audio/rhythm.wasm", import.meta.url))),
    });
    const pal = createPalette("cover");
    if (zones) {
      pal.setCover([250, 140, 40]);
      pal.setZones(blueGreenOrange);
    }
    scene.attach(renderer);
    scene.resize(640, 360, tierPreset("high"), createGeometry().out, 1);
    pal.update(frame, 1 / 60);
    scene.update(frame, 1 / 60);
    await loadWorld("spectrum");
    await new Promise((r) => setImmediate(r));
    for (let i = 0; i < 12; i++) {
      simT += 1 / 60;
      scene.draw(null, 640, 360, pal.out, null);
    }
    scene.dispose();
    return seen;
  }

  const on = await run({ zones: true });
  assert.ok(on.look, "the frame never reached the post pass");
  assert.equal(on.look.transparent, true, "the spectrum's row is not transparent");
  assert.ok(on.look.bloom > 0, "the engine's bloom was lost outside eco");
  assert.equal(on.uniforms.uZoneOK, 1, "the world was not told there are zones");
  assert.ok(Array.isArray(on.uniforms["uZone[0]"]), "no zone uniform was uploaded");
  assert.equal(on.uniforms["uZone[0]"].length, 28, "seven vec4, one per zone");
  // The left end of the sleeve is the first zone: hue 226° at s ~0.76, l ~0.49.
  const [r0, g0, b0] = on.uniforms["uZone[0]"].slice(0, 3);
  assert.ok(b0 > r0 && b0 > g0, `the first zone is not the blue one (${r0}, ${g0}, ${b0})`);

  const off = await run({ zones: false });
  assert.equal(off.uniforms.uZoneOK, 0, "a palette with no zones still claimed some");

  const eco = await run({ zones: true, eco: true });
  assert.equal(eco.look.bloom, 0, "eco kept the bloom");
  assert.equal(eco.look.grain, 0, "eco kept the grain");
  assert.equal(eco.look.ca, 0, "eco kept the fringe");
  assert.equal(eco.look.transparent, true);
  // The spectrum's params are width, curve, trail, pitch — in eco the trail is
  // the first thing a flat picture gives up.
  assert.equal(eco.uniforms.uP0?.[2], 0, `eco left the afterglow at ${eco.uniforms.uP0?.[2]}`);

  // THE PLAYER'S STRIP IS PRINTED, NOT LIT. With the theme's accent in hand
  // (the host's --ac-text, read live) the engine turns the whole post grade off
  // for this layout: the bloom is what used to become alpha and veil the
  // backdrop, and the grain/fringe/lift are finish a flat caption does not
  // have. The world is told the accent and prints it.
  const strip = await run({ zones: true, layout: "strip", solid: "#d2ff3c" });
  assert.equal(strip.look.transparent, true, "the strip is not transparent");
  assert.equal(strip.look.flat, true, "the accent did not switch the row to a flat print");
  assert.equal(strip.look.bloom, 0, "the strip kept a bloom (it becomes alpha over the canvas)");
  assert.equal(strip.look.grain, 0, "the strip kept the grain");
  assert.equal(strip.look.ca, 0, "the strip kept the fringe");
  assert.equal(strip.look.lift, 0, "the strip brightens on an impact");
  assert.equal(strip.uniforms.uSolidOK, 1, "the accent never reached the world");
  const solid = strip.uniforms.uSolid || [];
  const want = [210 / 255, 255 / 255, 60 / 255, 1];
  assert.equal(solid.length, 4, "uSolid is not one vec4");
  for (let i = 0; i < 3; i++) {
    assert.ok(Math.abs(solid[i] - want[i]) < 1e-6, `uSolid[${i}] = ${solid[i]}, not ${want[i]}`);
  }

  // ...and that is the STRIP's business only: the full-screen row, handed the
  // same accent, keeps its sleeve colours and its bloom.
  const wide = await run({ zones: true, layout: "full", solid: "#d2ff3c" });
  assert.equal(wide.look.flat, false, "the full-screen row was printed flat");
  assert.ok(wide.look.bloom > 0, "the full-screen row lost its bloom");

  // Eco on the strip keeps exactly the same flats (it only removes finish that
  // is already removed here).
  const stripEco = await run({ zones: true, layout: "strip", solid: "#d2ff3c", eco: true });
  assert.equal(stripEco.look.flat, true, "eco lost the flat print");
  assert.equal(stripEco.look.transparent, true);
  assert.equal(stripEco.look.bloom, 0);
});
