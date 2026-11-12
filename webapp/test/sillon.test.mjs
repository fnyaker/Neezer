// Sillon's palette rules (lib/sillon.js): the placeholder groove, the avatar
// initials, and the WCAG lift the Pochette theme applies to a sleeve's colour.
//
// The numbers below are what makes the fallbacks trustworthy: a deterministic
// groove (not a colour that changes on reload), a spread that never parks a
// page on one disc, and an accent that is ALWAYS legible on --onac.

import test from "node:test";
import assert from "node:assert/strict";

import {
  GROOVES,
  ACCENT_FALLBACK,
  ON_ACCENT,
  hashId,
  grooveFor,
  grooveBackground,
  grooveTint,
  initialsOf,
  luminance,
  contrast,
  accentFor,
  readableAccent,
  toHex,
  fromHex,
} from "../src/lib/sillon.js";

test("the same seed always picks the same groove", () => {
  const ids = [];
  for (let i = 0; i < 2000; i++) ids.push("track-" + i, "album-" + i, "artist:" + i);
  for (const id of ids) {
    const [a, b] = grooveFor(id);
    assert.equal(grooveFor(id)[0], a);
    assert.ok(
      GROOVES.some(([c, d]) => c === a && d === b),
      `${id} picked a pair that is not one of the eight`
    );
    assert.equal(hashId(id), hashId(String(id)));
  }
});

test("eight grooves, and a screenful of ids does not land on one", () => {
  const hits = new Map();
  for (let i = 0; i < 800; i++) {
    const [a] = grooveFor("album-" + i);
    hits.set(a, (hits.get(a) || 0) + 1);
  }
  const counts = [...hits.values()].sort((x, y) => y - x);
  // A uniform 800 over 8 would be 100 a bucket; allow the ordinary spread of a
  // hash but no bucket taking a quarter of the page.
  console.log(`# grooves: ${hits.size}/8 used, largest bucket ${counts[0]}/800`);
  assert.equal(hits.size, 8, "every groove is reachable");
  assert.ok(counts[0] <= 200, `largest bucket ${counts[0]} of 800`);
});

test("consecutive ids are not all the same disc", () => {
  // The ids a page shows really are consecutive-ish (album-217, album-218…).
  let same = 0;
  for (let i = 0; i < 200; i++) {
    if (grooveFor("album-" + i)[0] === grooveFor("album-" + (i + 1))[0]) same++;
  }
  console.log(`# consecutive pairs sharing a groove: ${same}/200`);
  assert.ok(same < 80, `${same}/200 neighbours shared a groove`);
});

test("a missing cover draws a groove, not a flat square", () => {
  const css = grooveBackground("42|Titre|7|album");
  assert.match(css, /^repeating-radial-gradient\(circle at 68% 32%/);
  const [a, b] = grooveFor("42|Titre|7|album");
  assert.ok(css.includes(a) && css.includes(b));
  assert.notEqual(grooveBackground("a"), grooveBackground("b"));
  // the avatar tile is the same pair, softer
  assert.ok(grooveTint("a").includes(grooveFor("a")[0]));
});

test("initials are two letters, accents kept, punctuation dropped", () => {
  assert.equal(initialsOf("Angerfist"), "AN");
  assert.equal(initialsOf("Dr. Peacock"), "DP");
  assert.equal(initialsOf("DJ TÖRKE"), "DT");
  assert.equal(initialsOf("Le Wanski"), "LW");
  assert.equal(initialsOf("AC/DC"), "AC");
  assert.equal(initialsOf("Tovaritch"), "TO");
  assert.equal(initialsOf(""), "");
  assert.equal(initialsOf(null), "");
  // never more than two, whatever the name
  for (const n of ["a b c d", "Été 2024", "x", "  "]) assert.ok(initialsOf(n).length <= 2, n);
});

test("hex round-trips and luminance follows WCAG", () => {
  assert.equal(toHex([12, 12, 11]), "#0C0C0B");
  assert.deepEqual(fromHex("#D2FF3C"), [210, 255, 60]);
  assert.equal(luminance([0, 0, 0]), 0);
  assert.equal(luminance([255, 255, 255]).toFixed(3), "1.000");
  assert.equal(contrast([255, 255, 255], [0, 0, 0]).toFixed(2), "21.00");
});

test("the Pochette accent always carries 4.5:1 on --onac", () => {
  const bg = fromHex(ON_ACCENT);
  // Every groove colour, both of its pair — the exact colours the fallbacks use.
  const seeds = GROOVES.flat();
  // Plus a sweep of the whole cube, coarse enough to run and dense enough to
  // catch a dark, saturated sleeve.
  const sweep = [];
  for (let r = 0; r <= 255; r += 51)
    for (let g = 0; g <= 255; g += 51) for (let b = 0; b <= 255; b += 51) sweep.push([r, g, b]);

  let lifted = 0;
  let worst = Infinity;
  for (const rgb of [...seeds, ...sweep].map((c) => (typeof c === "string" ? fromHex(c) : c))) {
    const out = accentFor(rgb);
    const ratio = contrast(fromHex(out), bg);
    worst = Math.min(worst, ratio);
    if (contrast(rgb, bg) < 4.5) lifted++;
    assert.ok(ratio >= 4.5, `${toHex(rgb)} → ${out} only ${ratio.toFixed(2)}:1`);
  }
  console.log(
    `# ${seeds.length + sweep.length} colours, ${lifted} lifted, worst ratio ${worst.toFixed(2)}:1`
  );
});

test("a colour already legible is left exactly as the sleeve gave it", () => {
  const bg = fromHex(ON_ACCENT);
  for (const hex of ["#D2FF3C", "#F1EFE8", "#FF6B4A", "#5CF2C0", "#FFC94A"]) {
    const rgb = fromHex(hex);
    assert.ok(contrast(rgb, bg) >= 4.5, `${hex} should pass as-is`);
    assert.deepEqual(readableAccent(rgb), rgb);
  }
});

test("a dark sleeve is lifted by the least brightening that works", () => {
  const bg = fromHex(ON_ACCENT);
  // A deep, saturated blue: the case that used to come out grey-lilac when the
  // lift mixed white in instead of scaling.
  const deep = [0x14, 0x30, 0x4a];
  const out = readableAccent(deep);
  assert.ok(contrast(out, bg) >= 4.5);
  // One step less (the scale loop is ×1.05) must NOT already pass: we lighten
  // until it is readable, not until it is pale.
  const oneLess = out.map((v) => Math.round(v / 1.05));
  assert.ok(contrast(oneLess, bg) < 4.5, `one step back ${toHex(oneLess)} already passes`);
  // …and the hue is kept: blue is still the largest channel.
  assert.ok(out[2] > out[0] && out[2] > out[1], toHex(out));
});

test("a sleeve with no colour at all still gives a readable accent", () => {
  for (const black of [
    [0, 0, 0],
    [4, 4, 4],
    [0, 0, 30],
  ]) {
    const out = accentFor(black);
    assert.ok(contrast(fromHex(out), fromHex(ON_ACCENT)) >= 4.5, out);
  }
  // The designed fallback is itself legible, so an empty state can use it.
  assert.ok(contrast(fromHex(ACCENT_FALLBACK), fromHex(ON_ACCENT)) >= 4.5);
});
