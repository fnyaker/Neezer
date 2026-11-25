// The gesture module's own rules, with no DOM (docs/agents/transitions.md S4,
// plan W1). The browser behaviour is measured in test/swipe/run.mjs; what is
// proven here is the maths and the window, which a Chromium bench cannot pin.
import test from "node:test";
import assert from "node:assert/strict";

import { tracker, projected, spring, PROJECT_MS } from "../src/lib/gesture.js";

/** Sample a spring every 1 ms over `seconds`, as value and velocity. */
function sweep(s, seconds = 2) {
  const out = [];
  for (let ms = 0; ms <= seconds * 1000; ms++) {
    const t = ms / 1000;
    out.push({ t, x: s.value(t), v: s.velocity(t) });
  }
  return out;
}

test("a spring seeded with the release velocity lands on its target", () => {
  const s = spring(100, 0, -1000, { response: 0.32, damping: 0.82 });
  const frames = sweep(s);
  assert.ok(s.done(0.4), "0.4 s after a 1 px/ms release it has arrived");
  const last = frames[frames.length - 1];
  assert.ok(Math.abs(last.x) < 0.5, `lands on 0, got ${last.x}`);
});

test("damping 0.82 overshoots by at most 2 % of the travel", () => {
  const s = spring(100, 0, -1000, { response: 0.32, damping: 0.82 });
  // The target is 0 and the travel is 100 px: the floor of the curve must stay
  // inside -2 px (the overshoot is what says "thrown", too much is a wobble).
  const floor = Math.min(...sweep(s).map((p) => p.x));
  assert.ok(floor < 0, `an underdamped settle crosses its target, got ${floor}`);
  assert.ok(floor >= -2, `overshoot ${Math.abs(floor).toFixed(2)} px > 2 % of 100 px`);
});

test("damping 1.0 never crosses the target", () => {
  const s = spring(100, 0, -1000, { response: 0.28, damping: 1 });
  for (const p of sweep(s)) assert.ok(p.x >= 0, `crossed at t=${p.t}: ${p.x}`);
  assert.ok(s.done(0.4), "critical damping still lands quickly");
});

test("a spring starts at the release position with the release velocity", () => {
  // Continuity is the whole point of a settle: frame 1 must not be a jump.
  const s = spring(120, 0, 900, { response: 0.32, damping: 0.82 });
  assert.equal(s.value(0), 120);
  assert.ok(Math.abs(s.velocity(0) - 900) < 1e-6);
});

test("projection is the velocity over PROJECT_MS", () => {
  assert.equal(PROJECT_MS, 160);
  assert.equal(projected(1), 160);
  assert.equal(projected(0.5), 80);
  assert.equal(projected(-1.2), -192);
});

test("the tracker only reads its trailing window", () => {
  const vt = tracker(80);
  vt.sample(0, 0); // the finger planting: slow, and stale by the release
  vt.sample(100, 10);
  vt.sample(180, 20);
  // 20 - 0 = 20 ms < 80: all three points still count.
  assert.equal(vt.velocity(), (180 - 0) / 20);
  vt.sample(200, 20); // a second point in this frame
  vt.sample(240, 100); // the first two samples are now > 80 ms old and go
  // The retained window is x 180→240 over t 20→100 ms: 60 px in 80 ms.
  assert.equal(vt.velocity(), 60 / 80);
  vt.reset();
  assert.equal(vt.velocity(), 0, "a reset gesture has no velocity");
});

test("coalesced moves sharing a timestamp do not zero the velocity", () => {
  // The browser merges the moves of one frame into one task: two samples can
  // carry the same `performance.now()`.
  const vt = tracker(80);
  vt.sample(0, 0);
  vt.sample(8, 16);
  vt.sample(16, 16); // same frame, same clock reading
  assert.equal(vt.velocity(), 16 / 16, "measured over the last real span");
});
