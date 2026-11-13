// The equalizer card's rules without a browser (src/lib/eqpresets.js): the
// built-in curves, "which preset am I on", what "Mettre à jour" may write,
// creation/deletion against the real `fxPresets` store, the meter's risk
// vocabulary, and the one subscription's lifetime.
//
// Run with `npm test` (node --test). The meter ENGINE is toyed with in
// test/audio.test.mjs; what is pinned here is the UI side: the things that were
// inline in the component and could not be tested, and the subscription that
// must not outlive the card.
import test from "node:test";
import assert from "node:assert/strict";
import { get } from "svelte/store";

import {
  BUILTIN_PRESETS,
  EQ_BAND_COUNT,
  RISK_TONES,
  activePresetId,
  bandsEqual,
  createMeterSession,
  dbToGain,
  fmtDb,
  isBuiltinPreset,
  isUserPreset,
  presetItems,
  presetMatches,
  presetNeedsUpdate,
  presetUpdatable,
  riskTone,
  updatedPreset,
  volumeCutDb,
} from "../src/lib/eqpresets.js";
import {
  bassBoost,
  deleteFxPreset,
  eqBands,
  eqEnabled,
  fxPresets,
  normalization,
  saveFxPreset,
} from "../src/lib/stores.js";

const ZERO = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
const CURVE = [6, 5, 4, 2, 0, -1, -2, 1, 3, 4];
const chainOf = (over = {}) => ({
  eq: true,
  bands: CURVE.slice(),
  bass: 0.4,
  norm: "off",
  ...over,
});

/** Put the shared stores back where the rest of the suite expects them. */
function withStores(fn) {
  const before = {
    bands: get(eqBands).slice(),
    eq: get(eqEnabled),
    bass: get(bassBoost),
    norm: get(normalization),
    presets: get(fxPresets).slice(),
  };
  try {
    return fn();
  } finally {
    eqBands.set(before.bands);
    eqEnabled.set(before.eq);
    bassBoost.set(before.bass);
    normalization.set(before.norm);
    for (const p of get(fxPresets)) if (!before.presets.some((q) => q.id === p.id)) deleteFxPreset(p.id);
  }
}

test("the list holds all five built-ins, then the person's own", () => {
  const items = presetItems([
    { id: "p1", name: "Casque", bands: CURVE, bass: 0.2, eq: true, norm: "off" },
    { name: "sans id" },
    null,
  ]);
  assert.equal(items.length, BUILTIN_PRESETS.length + 1, "a row without an id is dropped");
  assert.equal(items[0].name, "Plat");
  assert.equal(items[0].builtin, true);
  assert.equal(items[0].label, "Plat", "the old field name stays for stored data");
  assert.equal(items[BUILTIN_PRESETS.length].name, "Casque");
  assert.equal(items[BUILTIN_PRESETS.length].builtin, false);
  assert.equal(items[BUILTIN_PRESETS.length].bands.length, EQ_BAND_COUNT);
  // The built-ins are copies: applying one must not rewrite the shipped curve.
  items[0].bands[0] = 9;
  assert.equal(BUILTIN_PRESETS[0].bands[0], 0);
});

test("a preserved curve matches on ten gains, a whole preset on the chain too", () => {
  assert.equal(bandsEqual(ZERO, [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), true);
  assert.equal(bandsEqual(ZERO, [0, 0, 0, 0, 0, 0, 0, 0, 0]), false, "nine bands is not a curve");
  assert.equal(bandsEqual(ZERO, null), false);
  assert.equal(bandsEqual([0, 0, 0, 0, 0, 0, 0, 0, 0, "x"], ZERO), false);

  const p = { id: "p", name: "Casque", eq: true, norm: "medium", bands: CURVE, bass: 0.4 };
  assert.equal(presetMatches(p, chainOf({ norm: "medium" })), true);
  assert.equal(presetMatches(p, chainOf({ norm: "high" })), false, "normalization is part of the preset");
  assert.equal(presetMatches(p, chainOf({ eq: false })), false, "so is the EQ switch");
  assert.equal(presetMatches(p, chainOf({ bass: 0.2 })), false, "and the bass lift");
});

test("the highlighted preset is the person's own when it is the more precise answer", () => {
  const flat = { eq: true, bands: ZERO.slice(), bass: 0, norm: "off" };
  assert.equal(activePresetId(presetItems([]), flat), "flat");
  assert.equal(activePresetId([], flat), null);

  const copy = { id: "p1", name: "Mon plat", eq: true, norm: "off", bands: ZERO.slice(), bass: 0 };
  assert.equal(
    activePresetId(presetItems([copy]), flat),
    "p1",
    "a user preset carries the EQ flag and normalization too, so it wins the tie"
  );
  // A user preset saved with the EQ off does not match a live EQ that is on.
  const off = { ...copy, eq: false };
  assert.equal(activePresetId([off], flat), null);
});

test("Mettre à jour writes the ten bands and the bass lift, and nothing else", () => {
  const p = {
    id: "p1",
    name: "Casque",
    eq: false,
    norm: "medium",
    bands: ZERO.slice(),
    bass: 0.1,
  };
  const next = updatedPreset(p, chainOf());
  assert.ok(next);
  assert.deepEqual(next.bands, CURVE, "the ten current gains");
  assert.equal(next.bass, 0.4, "the current bass lift");
  assert.equal(next.eq, false, "the preset's EQ switch is how it is applied, not its curve");
  assert.equal(next.norm, "medium", "normalization is not what this button writes");
  assert.equal(next.id, "p1");
  assert.equal(next.name, "Casque");
  assert.deepEqual(p.bands, ZERO, "the stored row is replaced, never mutated in place");
});

test("a built-in preset cannot be updated", () => {
  for (const p of BUILTIN_PRESETS) {
    const asShown = { ...p, name: p.label, builtin: true, bands: p.bands.slice() };
    assert.equal(updatedPreset(asShown, chainOf()), null, `${p.label} is code, not storage`);
    assert.equal(updatedPreset({ ...p, bands: p.bands.slice() }, chainOf()), null, `${p.label}'s id is enough`);
    assert.equal(presetUpdatable(asShown), false);
  }
  assert.equal(updatedPreset({ id: "bass", label: "Basses+" }, chainOf()), null);
  assert.equal(updatedPreset(null, chainOf()), null);
});

test("an old preset only gains the fields it already carries", () => {
  const noBass = { id: "p2", name: "Vieux", eq: true, norm: "off", bands: ZERO.slice() };
  const a = updatedPreset(noBass, chainOf());
  assert.deepEqual(a.bands, CURVE);
  assert.equal("bass" in a, false, "no bass lift was ever stored: none is invented");

  const noBands = { id: "p3", name: "Plus vieux", bass: 0 };
  const b = updatedPreset(noBands, chainOf());
  assert.equal("bands" in b, false, "a preset with no curve keeps its shape");
  assert.equal(b.bass, 0.4);

  assert.equal(updatedPreset({ id: "p4", name: "Rien" }, chainOf()), null, "nothing to write");
});

test("the update offer follows exactly the fields it writes", () => {
  const p = { id: "p", name: "Casque", eq: true, norm: "off", bands: CURVE.slice(), bass: 0.4 };
  assert.equal(presetNeedsUpdate(p, chainOf()), false);
  assert.equal(presetNeedsUpdate(p, chainOf({ bands: ZERO })), true);
  assert.equal(presetNeedsUpdate(p, chainOf({ bass: 0.8 })), true);
  assert.equal(
    presetNeedsUpdate(p, chainOf({ norm: "high" })),
    false,
    "the button does not write normalization, so a different level is not an unsaved change"
  );
  assert.equal(presetNeedsUpdate({ ...p, id: "flat", name: "Plat", builtin: true }, chainOf({ bands: ZERO })), false);
});

test("creating and deleting a preset goes through the real store", () =>
  withStores(() => {
    eqBands.set(CURVE.slice());
    eqEnabled.set(true);
    bassBoost.set(0.4);
    normalization.set("medium");

    const saved = saveFxPreset("  Neezer test — casque  ");
    assert.ok(saved, "a non-empty name saves");
    assert.equal(saved.name, "Neezer test — casque");
    assert.deepEqual(saved.bands, CURVE);
    assert.equal(saved.bass, 0.4);
    assert.equal(saved.eq, true);
    assert.equal(saved.norm, "medium");

    const stored = get(fxPresets).find((p) => p.name === saved.name);
    assert.ok(stored, "the preset is in the store");
    assert.deepEqual(stored.bands, CURVE);
    assert.equal(isUserPreset(stored), true);

    deleteFxPreset(stored.id);
    assert.equal(get(fxPresets).some((p) => p.id === stored.id), false, "delete removes the row");
  }));

test("re-saving a name replaces its row, keeping the row's own id", () =>
  withStores(() => {
    eqBands.set(ZERO.slice());
    const first = saveFxPreset("Neezer test — doublon");
    eqBands.set(CURVE.slice());
    const second = saveFxPreset("neezer test — doublon");
    const rows = get(fxPresets).filter((p) => p.name.toLowerCase() === "neezer test — doublon");
    assert.equal(rows.length, 1, "one row, not near-duplicates");
    assert.equal(rows[0].id, first.id, "the stored row keeps its identity");
    assert.deepEqual(rows[0].bands, CURVE, "with the new curve");
    assert.notEqual(second.id, rows[0].id, "saveFxPreset returns the snapshot it built");
    const found = presetItems(get(fxPresets)).find((p) => p.name.toLowerCase() === "neezer test — doublon");
    assert.equal(found.id, rows[0].id, "the card selects what storage holds, not the returned id");
  }));

test("risk is three tones, and an unknown reading is not a warning", () => {
  assert.deepEqual(RISK_TONES, ["ok", "warn", "clip"]);
  assert.equal(riskTone("ok"), "ok");
  assert.equal(riskTone("warn"), "warn");
  assert.equal(riskTone("clip"), "clip");
  assert.equal(riskTone(undefined), "ok");
  assert.equal(riskTone("boom"), "ok");
  assert.equal(isBuiltinPreset({ id: "flat" }), true);
  assert.equal(isUserPreset({ id: "flat" }), false);
});

test("the warning's numbers read in French, and the cut is a half-decibel step", () => {
  assert.equal(fmtDb(2.44), "2,4 dB");
  assert.equal(fmtDb(-3.5), "\u22123,5 dB");
  assert.equal(fmtDb(-0.04), "0,0 dB", "no -0.0 on a silent meter");
  assert.equal(fmtDb(2.44, { plus: true }), "+2,4 dB");
  assert.equal(fmtDb(3, { unit: false }), "3,0");
  assert.equal(volumeCutDb(2.24), 2);
  assert.equal(volumeCutDb(2.26), 2.5);
  assert.equal(volumeCutDb(0), 0.5, "the button always does something");
  assert.equal(volumeCutDb(-4), 0.5);
  assert.ok(Math.abs(dbToGain(6) - 0.501187) < 1e-6, "-6 dB halves the level");
  assert.equal(dbToGain(0), 1);
});

test("the meter's subscription starts once, stops on demand, and comes back", () => {
  let subscribed = 0;
  let unsubscribed = 0;
  const seen = [];
  const subscribe = (fn) => {
    subscribed++;
    fn({ available: true, n: subscribed });
    return () => {
      unsubscribed++;
    };
  };
  const session = createMeterSession(subscribe, (s) => seen.push(s));

  assert.equal(session.running, false);
  assert.equal(session.start(), true);
  assert.equal(subscribed, 1, "one subscription for the card");
  assert.deepEqual(seen, [{ available: true, n: 1 }], "the engine's immediate reading is delivered");

  assert.equal(session.start(), false, "a second start joins the one subscription");
  assert.equal(subscribed, 1);
  assert.equal(session.running, true);

  assert.equal(session.stop(), true);
  assert.equal(unsubscribed, 1, "leaving the card stops the 25 Hz tap");
  assert.equal(session.running, false);
  assert.equal(session.stop(), false, "stopping twice never unsubscribes twice");
  assert.equal(unsubscribed, 1);

  assert.equal(session.start(), true, "scrolling back subscribes again");
  assert.equal(subscribed, 2);
  session.stop();
  assert.equal(unsubscribed, 2);
});
