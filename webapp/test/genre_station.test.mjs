// The genre station's pure rules (webapp/src/lib/genrestation.js): what a
// station's playback context is, what may be handed to the player, and how the
// "all stations" list folds a search. No browser, no request.

import test from "node:test";
import assert from "node:assert/strict";

import {
  STATION_KIND,
  stationContext,
  stationTracks,
  stylesOf,
  filterStyles,
} from "../src/lib/genrestation.js";

test("a station is its own context, anchored on the server's canonical style", () => {
  // The server resolves an alias/folded spelling; the context must keep the
  // canonical name so a refill POST can resolve it back.
  assert.deepEqual(stationContext("hardcore", { style: "Hardcore", name: "Hardcore" }), {
    kind: STATION_KIND,
    id: "Hardcore",
    title: "Station Hardcore",
  });
  assert.equal(STATION_KIND, "genre-radio");
  // The local name is enough when the answer says nothing (a trim, never empty).
  assert.deepEqual(stationContext("  Disco / funk  ", null), {
    kind: "genre-radio",
    id: "Disco / funk",
    title: "Station Disco / funk",
  });
  assert.equal(stationContext("", null), null);
});

test("only rows the player can stream reach the queue", () => {
  const rows = stationTracks({
    tracks: [
      { id: "a", deezer_id: "a" },
      { id: "b", deezer_id: "" }, // an empty provider id is not an id; the row's own is
      { title: "no id" },
      null,
    ],
  });
  assert.equal(rows.length, 2);
  assert.deepEqual(stationTracks(null), []);
  assert.deepEqual(stationTracks({ tracks: "nope" }), []);
});

test("the all-stations list drops junk and keeps the counts", () => {
  const rows = stylesOf({
    styles: [
      { name: "Hardcore", parent: "Hard dance", branch: 92, tracks: 8 },
      { name: "  Frenchcore ", parent: "", branch: "31", tracks: null },
      { name: "", branch: 4 },
      { parent: "Orphan", branch: 4 },
      { name: "Hardcore", parent: "Kept once", branch: 1 },
      null,
    ],
  });
  assert.deepEqual(rows, [
    { name: "Hardcore", parent: "Hard dance", branch: 92, tracks: 8 },
    { name: "Frenchcore", parent: null, branch: 31, tracks: 0 },
  ]);
  assert.deepEqual(stylesOf({}), []);
});

test("a style search folds accents, case and the space around a slash", () => {
  const rows = stylesOf({
    styles: [
      { name: "Électronique", branch: 400 },
      { name: "Disco / funk", branch: 90 },
      { name: "Hardcore", parent: "Hard dance", branch: 92 },
      { name: "Jazz / acoustique", branch: 30 },
    ],
  });
  assert.deepEqual(filterStyles(rows, "electro").map((r) => r.name), ["Électronique"]);
  assert.deepEqual(filterStyles(rows, "DISCO funk").map((r) => r.name), ["Disco / funk"]);
  assert.deepEqual(filterStyles(rows, "hard dance").map((r) => r.name), ["Hardcore"]);
  assert.equal(filterStyles(rows, "   ").length, 4);
  assert.deepEqual(filterStyles(rows, "zzz"), []);
});

test("a big library's style list stays a linear filter", () => {
  // The worst realistic shape: 8 000 titles spread over 320 analysed styles
  // (25 each). The sheet filters the 320 rows it already holds; no request,
  // however many rows the person types through.
  const rows = stylesOf({
    styles: Array.from({ length: 320 }, (_, i) => ({
      name: `Genre ${i}`,
      parent: i % 4 ? `Family ${i % 4}` : null,
      branch: 25 * ((i % 8) + 1),
      tracks: 25,
    })),
  });
  assert.equal(rows.length, 320);
  assert.equal(filterStyles(rows, "genre 31").length, 11); // 31 and 310..319
  assert.equal(filterStyles(rows, "family 2").length, 80);
});
