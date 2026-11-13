// Mono and LEFT/RIGHT balance without a browser (src/lib/audio/pan.js): the
// equal-power pan law, the W3C stereo→mono down-mix, the persisted settings and
// the readable store the graph and the settings card share.
//
// Why this file exists: the graph inserts the stage (a GainNode with
// channelCount=1, then a StereoPannerNode) between its output and its
// look-ahead, and the analysers tap BEFORE it so the rhythm analysis and the
// stereo scope keep the original signal. The browser wiring is exercised by the
// browser benches; what is pinned here is the arithmetic and the storage, and
// that the curve is the platform's, not an invented one.
//
// The convention, stated once: the gains are the amplitude sent to each output
// for a MONO source, from the W3C Web Audio API's StereoPannerNode algorithm
// (§6.3.3). At the centre they are 1/√2 on each side — the constant-power pan
// law a console pan pot uses (gL² + gR² = 1 everywhere), 3 dB below a full-gain
// hard pan per speaker. Hard left is (1, 0), hard right is (0, 1). At the
// centre the mono fold is therefore played 3 dB down per side; this stage sits
// after the safety limiter, so it is deliberately not made up (see pan.js).
//
// Run with `node --test test/pan.test.mjs`, or as part of `npm test`.

import test from "node:test";
import assert from "node:assert/strict";

import {
  BALANCE_KEY,
  CENTRE_EPS,
  MONO_GAIN,
  MONO_KEY,
  clampBalance,
  describeBalance,
  describePan,
  monoDownmix,
  panGains,
  panSettings,
  parseBalance,
  parseMono,
  readPanSettings,
  writePanSettings,
} from "../src/lib/audio/pan.js";

const close = (a, b, tol = 1e-12) => Math.abs(a - b) <= tol;

/** A localStorage stand-in: the same three methods, no browser. */
function fakeStorage(seed = {}) {
  const data = new Map(Object.entries(seed));
  return {
    getItem: (k) => (data.has(k) ? data.get(k) : null),
    setItem: (k, v) => data.set(k, String(v)),
    removeItem: (k) => data.delete(k),
    data,
  };
}

// The W3C StereoPannerNode algorithm for a MONO input (§6.3.3), written out in
// the test so pan.js is compared with the spec rather than with itself:
//   x = (clamp(pan) + 1) / 2; gainL = cos(x·π/2); gainR = sin(x·π/2).
function specMonoPan(sample, pan) {
  const p = Math.max(-1, Math.min(1, pan));
  const x = (p + 1) / 2;
  return {
    left: sample * Math.cos((x * Math.PI) / 2),
    right: sample * Math.sin((x * Math.PI) / 2),
  };
}

test("the equal-power law is the platform's, centre 1/√2 and hard ends 1/0", () => {
  const centre = panGains(0);
  // The centre of a constant-power pan pot: -3.01 dB on each side, +3 dB of
  // total power compared with sending the whole signal to one speaker.
  assert.ok(close(centre.left, Math.SQRT1_2), `left ${centre.left}`);
  assert.ok(close(centre.right, Math.SQRT1_2), `right ${centre.right}`);
  assert.ok(close(20 * Math.log10(centre.left), -3.0103, 1e-3), "the centre is -3.01 dB per side");

  const left = panGains(-1);
  assert.ok(close(left.left, 1), `hard left, left ${left.left}`);
  assert.ok(close(left.right, 0), `hard left, right ${left.right}`);
  const right = panGains(1);
  assert.ok(close(right.left, 0), `hard right, left ${right.left}`);
  assert.ok(close(right.right, 1), `hard right, right ${right.right}`);

  // Same numbers as an implementation of the spec's mono branch, at every
  // position — the curve is not a different equal-power variant.
  for (let b = -1; b <= 1.0001; b += 0.125) {
    const g = panGains(b);
    const s = specMonoPan(1, b);
    assert.ok(close(g.left, s.left) && close(g.right, s.right), `balance ${b}`);
  }
});

test("power stays constant and the image moves monotonically", () => {
  let lastLeft = Infinity;
  let lastRight = -Infinity;
  for (let b = -1; b <= 1.0001; b += 0.05) {
    const { left, right } = panGains(b);
    assert.ok(close(left * left + right * right, 1), `power at ${b}`);
    assert.ok(left <= lastLeft + 1e-12, `left must fall as balance moves right (${b})`);
    assert.ok(right >= lastRight - 1e-12, `right must rise as balance moves right (${b})`);
    lastLeft = left;
    lastRight = right;
  }
});

test("the balance is clamped, and junk lands at the centre", () => {
  assert.deepEqual(panGains(5), panGains(1));
  assert.deepEqual(panGains(-9), panGains(-1));
  assert.deepEqual(panGains(NaN), panGains(0));
  assert.deepEqual(panGains(""), panGains(0));
  // A hand-edited or French-formatted value uses a comma: still a number.
  assert.deepEqual(panGains("0,25"), panGains(0.25));
  assert.equal(clampBalance(-1.2), -1);
  assert.equal(clampBalance(1.2), 1);
  assert.equal(clampBalance("-0.5"), -0.5);
  assert.equal(clampBalance(undefined), 0);
  assert.equal(clampBalance("abc"), 0);
});

test("the mono down-mix is the spec's 0.5·(L + R)", () => {
  assert.equal(MONO_GAIN, 0.5);
  assert.equal(monoDownmix(1, 1), 1, "a centred full-scale pair keeps its amplitude");
  assert.equal(monoDownmix(1, -1), 0, "an anti-phase pair cancels, as a mono fold must");
  assert.equal(monoDownmix(0.5, -0.25), 0.125);
  assert.equal(monoDownmix(-1, -1), -1);
  for (const [l, r] of [
    [0.2, 0.9],
    [-0.7, 0.1],
    [0, 0],
  ])
    assert.ok(close(monoDownmix(l, r), 0.5 * (l + r)));
});

test("the chain is down-mix then panner, and the mono centre is -3 dB per side", () => {
  // What the graph's two nodes do to a centred stereo sample: the down-mix,
  // then the mono pan law. This is where the -3 dB at the centre comes from.
  const m = monoDownmix(0.8, 0.8);
  assert.ok(close(m, 0.8));
  const centre = panGains(0);
  assert.ok(close(m * centre.left, 0.8 / Math.SQRT2), "0.566 per side, not 0.8");
  // Hard left: the whole mono signal, still inside full scale.
  assert.ok(close(m * panGains(-1).left, 0.8));
  assert.ok(m * panGains(-1).right === 0);
  // No position of a mono fold at or below full scale can exceed full scale on
  // one side: max(gL, gR) is 1 at the hard ends and 1/√2 at the centre.
  for (let b = -1; b <= 1.0001; b += 0.05) {
    const g = panGains(b);
    for (const [l, r] of [
      [1, 1],
      [1, -1],
      [1, 0],
    ]) {
      const mono = monoDownmix(l, r);
      assert.ok(Math.abs(mono * g.left) <= 1 + 1e-12, `clip at balance ${b}`);
      assert.ok(Math.abs(mono * g.right) <= 1 + 1e-12, `clip at balance ${b}`);
    }
  }
});

test("the settings round-trip under audio.mono and audio.balance", () => {
  const s = fakeStorage();
  assert.equal(MONO_KEY, "audio.mono");
  assert.equal(BALANCE_KEY, "audio.balance");
  assert.deepEqual(readPanSettings(s), { mono: false, balance: 0 });

  writePanSettings({ mono: true, balance: -0.3 }, s);
  assert.equal(s.getItem(MONO_KEY), "true");
  assert.equal(s.getItem(BALANCE_KEY), "-0.3");
  assert.deepEqual(readPanSettings(s), { mono: true, balance: -0.3 });

  // Clamped on the way in, so a hand-edited value can never ask for more.
  assert.deepEqual(writePanSettings({ mono: true, balance: 7 }, s), { mono: true, balance: 1 });
  assert.equal(s.getItem(BALANCE_KEY), "1");
  assert.deepEqual(writePanSettings({ mono: false, balance: NaN }, s), {
    mono: false,
    balance: 0,
  });
  assert.equal(s.getItem(MONO_KEY), "false");
  assert.equal(s.getItem(BALANCE_KEY), "0");

  // Three decimals is what the slider's hundredths and a decimal can need.
  writePanSettings({ mono: false, balance: 0.3334 }, s);
  assert.equal(s.getItem(BALANCE_KEY), "0.333");
});

test("a stored setting is parsed, not trusted", () => {
  for (const raw of [true, 1, "true", "TRUE", "1", "on", " yes ", '"true"'])
    assert.equal(parseMono(raw), true, `mono ${JSON.stringify(raw)}`);
  for (const raw of [false, 0, "false", "0", "off", "no", "oui", "", null, undefined])
    assert.equal(parseMono(raw), false, `mono ${JSON.stringify(raw)}`);
  assert.equal(parseBalance(null), 0);
  assert.equal(parseBalance(""), 0);
  assert.equal(parseBalance("0.42"), 0.42);
  assert.equal(parseBalance("0,42"), 0.42);
  assert.equal(parseBalance("-4"), -1);
  assert.equal(parseBalance("loud"), 0);
  assert.equal(parseBalance("Infinity"), 0, "an infinite value is not a balance");

  // A storage that throws (private mode, a full quota) reads as the defaults
  // and still answers the value that applies for this session.
  const broken = {
    getItem() {
      throw new Error("denied");
    },
    setItem() {
      throw new Error("quota");
    },
  };
  assert.deepEqual(readPanSettings(broken), { mono: false, balance: 0 });
  assert.deepEqual(writePanSettings({ mono: true, balance: 0.4 }, broken), {
    mono: true,
    balance: 0.4,
  });
  assert.deepEqual(readPanSettings(null), { mono: false, balance: 0 });
});

test("describeBalance says what the slider shows", () => {
  assert.equal(describeBalance(0), "Centre");
  assert.equal(describeBalance(CENTRE_EPS / 2), "Centre");
  assert.equal(describeBalance(-0.3), "30 % à gauche");
  assert.equal(describeBalance(0.7), "70 % à droite");
  assert.equal(describeBalance(-0.306), "31 % à gauche");
  assert.equal(describeBalance(-1), "À gauche");
  assert.equal(describeBalance(1), "À droite");
  assert.equal(describeBalance(-0.999), "À gauche");
  assert.equal(describeBalance(1.4), "À droite", "clamped first");
  assert.equal(describePan({ mono: true, balance: 0 }), "Mono · Centre");
  assert.equal(describePan({ mono: false, balance: -0.3 }), "Stéréo · 30 % à gauche");
  assert.equal(describePan(null), "Stéréo · Centre");
});

test("the live store reads, notifies and persists through localStorage", async () => {
  // A fresh module instance: the shared one's lazy state is already seeded by
  // the import above, and this test needs to watch the first read.
  const fake = fakeStorage({ [MONO_KEY]: "true", [BALANCE_KEY]: "-0.25" });
  const previous = globalThis.localStorage;
  globalThis.localStorage = fake;
  try {
    const pan = await import("../src/lib/audio/pan.js?store");
    const seen = [];
    const stop = pan.panSettings.subscribe((v) => seen.push(v));
    assert.deepEqual(seen, [{ mono: true, balance: -0.25 }], "a subscriber is answered at once");

    pan.setMono(false);
    assert.deepEqual(seen.at(-1), { mono: false, balance: -0.25 });
    assert.equal(fake.getItem(MONO_KEY), "false");
    assert.equal(fake.getItem(BALANCE_KEY), "-0.25");

    pan.setBalance(0.4);
    assert.deepEqual(seen.at(-1), { mono: false, balance: 0.4 });
    assert.equal(fake.getItem(BALANCE_KEY), "0.4");

    pan.resetBalance();
    assert.deepEqual(seen.at(-1), { mono: false, balance: 0 });
    assert.equal(fake.getItem(BALANCE_KEY), "0");

    pan.applyPanSettings({ mono: true });
    assert.deepEqual(seen.at(-1), { mono: true, balance: 0 }, "a patch keeps the other setting");

    const at = seen.length;
    stop();
    pan.setMono(false);
    assert.equal(seen.length, at, "an unsubscriber stops hearing");
    pan.panSettings.subscribe(() => {});
  } finally {
    if (previous === undefined) delete globalThis.localStorage;
    else globalThis.localStorage = previous;
  }
});

test("the module's own store answers the persisted settings (or the defaults)", () => {
  // The static import reads its settings lazily with no fake installed; in a
  // browser that is this device's localStorage, here it is whatever the test
  // process has (or nothing at all). Either way the store answers at once with
  // a well-formed pair — this is what the graph's subscription sees at import.
  let first = null;
  const stop = panSettings.subscribe((v) => (first = v));
  stop();
  assert.equal(typeof first.mono, "boolean");
  assert.equal(Number.isFinite(first.balance), true);
  assert.ok(Math.abs(first.balance) <= 1);
});
