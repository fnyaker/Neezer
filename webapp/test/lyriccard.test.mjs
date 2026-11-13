// The lyric card's decisions, held as pure rules (src/lib/lyriccard.js).
//
// No browser here: line selection, wrapping, filename safety and the empty /
// unsynced shapes are exactly what can be decided without a canvas. The drawing
// itself is opened in a real browser by the feature's harness, not asserted
// through a screenshot in this suite.

import test from "node:test";
import assert from "node:assert/strict";

import {
  CARD_W,
  CARD_H,
  MAX_LINES,
  LYRIC_SIZES,
  lyricLines,
  defaultLineIndex,
  toggleLine,
  chooseLines,
  wrapText,
  fitOneLine,
  fitLyricLayout,
  cardFilename,
} from "../src/lib/lyriccard.js";

// A canvas-like measurer: every code point is `perChar` px wide. This is the
// contract wrapText reads (measureText(...).width), nothing more.
function measurer(perChar) {
  return { measureText: (s) => ({ width: [...String(s)].length * perChar }) };
}

test("the card is the 4:5 portrait format, at most three lines", () => {
  assert.equal(CARD_W, 1080);
  assert.equal(CARD_H, 1350);
  assert.equal(CARD_H / CARD_W, 1.25); // 4:5: Instagram's feed portrait, no crop
  assert.equal(MAX_LINES, 3);
  assert.deepEqual(LYRIC_SIZES, [...LYRIC_SIZES].sort((a, b) => b - a));
});

test("synced lyrics keep their source index and skip instrumental markers", () => {
  const lines = lyricLines({
    synced: [
      { time: 0, text: "Première ligne" },
      { time: 8_000, text: "" },
      { time: 9_000, text: "• • •" },
      { time: 12_000, text: "  Deuxième ligne  " },
    ],
  });
  assert.deepEqual(lines, [
    { text: "Première ligne", time: 0, source: 0 },
    { text: "Deuxième ligne", time: 12_000, source: 3 },
  ]);
});

test("plain lyrics become selectable lines, blanks and markers dropped", () => {
  const lines = lyricLines({ text: "  Ligne une\n\nLigne deux\n♪\n   \nLigne trois " });
  assert.deepEqual(lines, [
    { text: "Ligne une", time: null, source: 0 },
    { text: "Ligne deux", time: null, source: 2 },
    { text: "Ligne trois", time: null, source: 5 },
  ]);
});

test("no lyrics, an empty synced list and a non-string text offer nothing", () => {
  assert.deepEqual(lyricLines(null), []);
  assert.deepEqual(lyricLines({ synced: [] }), []);
  assert.deepEqual(lyricLines({ synced: [], text: "   " }), []);
  assert.deepEqual(lyricLines({ text: 42 }), []);
});

test("the default line is the one being heard, else the first", () => {
  const lines = lyricLines({
    synced: [
      { time: 0, text: "un" },
      { time: 1_000, text: "" }, // marker at source 1
      { time: 2_000, text: "deux" },
      { time: 3_000, text: "trois" },
    ],
  });
  assert.equal(defaultLineIndex(lines, 2), 1); // source 2 -> "deux"
  assert.equal(defaultLineIndex(lines, 1), 0); // marker -> the words before it
  assert.equal(defaultLineIndex(lines, 3), 2);
  assert.equal(defaultLineIndex(lines, -1), 0);
  assert.equal(defaultLineIndex(lines, 99), 2); // past the end -> the last words
  assert.equal(defaultLineIndex([], 4), -1);
  assert.equal(defaultLineIndex(null), -1);
});

test("a tap toggles a line, keeps lyric order, and says why a limit was hit", () => {
  let s = [1];
  let r = toggleLine(s, 0);
  assert.deepEqual(r, { indices: [0, 1], reason: null });
  s = r.indices;
  r = toggleLine(s, 2);
  assert.deepEqual(r, { indices: [0, 1, 2], reason: null });
  s = r.indices;
  // a fourth line is refused, the three already chosen stay
  r = toggleLine(s, 3);
  assert.deepEqual(r, { indices: [0, 1, 2], reason: "max" });
  // deselect the middle one
  r = toggleLine(s, 1);
  assert.deepEqual(r, { indices: [0, 2], reason: null });
  s = r.indices;
  r = toggleLine(s, 0);
  assert.deepEqual(r, { indices: [2], reason: null });
  // the last selected line cannot be removed: a card always says something
  r = toggleLine([2], 2);
  assert.deepEqual(r, { indices: [2], reason: "min" });
});

test("toggleLine is forgiving about duplicates, order and bad indices", () => {
  assert.deepEqual(toggleLine([2, 0, 2], 1), { indices: [0, 1, 2], reason: null });
  assert.deepEqual(toggleLine([1], -1), { indices: [1], reason: null });
  assert.deepEqual(toggleLine([1], 1.5), { indices: [1], reason: null });
  assert.deepEqual(toggleLine(null, 0), { indices: [0], reason: null });
  assert.deepEqual(toggleLine([0, 1], 2, 2), { indices: [0, 1], reason: "max" });
});

test("chooseLines returns the selected text in lyric order, capped at three", () => {
  const lines = lyricLines({ text: "un\ndeux\ntrois\nquatre" });
  assert.deepEqual(chooseLines(lines, [2, 0]), ["un", "trois"]);
  assert.deepEqual(chooseLines(lines, [3, 1, 2, 0]), ["un", "deux", "trois"]);
  assert.deepEqual(chooseLines(lines, [9, -1, 1]), ["deux"]);
  assert.deepEqual(chooseLines(lines, []), []);
  assert.deepEqual(chooseLines([], [0]), []);
});

test("wrapText breaks on words at the measured width", () => {
  const m = measurer(10);
  assert.deepEqual(wrapText(m, "Bonjour le monde", 100), ["Bonjour le", "monde"]);
  assert.deepEqual(wrapText(m, "court", 100), ["court"]);
  assert.deepEqual(wrapText(m, "", 100), []);
  assert.deepEqual(wrapText(m, "   ", 100), []);
  assert.deepEqual(wrapText(m, "mot", 0), []);
});

test("wrapText cuts a word that is wider than the column, never dropping a letter", () => {
  const m = measurer(10);
  const word = "Anticonstitutionnellement";
  const rows = wrapText(m, word, 50);
  assert.deepEqual(rows, ["Antic", "onsti", "tutio", "nnell", "ement"]);
  assert.equal(rows.join(""), word);
  // a single glyph wider than the column still gets its own line
  assert.deepEqual(wrapText(measurer(100), "ab cd", 40), ["a", "b", "c", "d"]);
});

test("wrapText keeps hard newlines and French accents", () => {
  const m = measurer(10);
  assert.deepEqual(wrapText(m, "Été à Noël\nOù ça ?", 50), ["Été à", "Noël", "Où ça", "?"]);
  // the characters survive, not just their ASCII skeletons
  const joined = wrapText(m, "L'âme en peine, l'écho", 70).join(" ");
  assert.match(joined, /â/);
  assert.match(joined, /é/);
});

test("fitOneLine ellipsises a name that does not fit, accents intact", () => {
  const m = measurer(10);
  assert.equal(fitOneLine(m, "Court", 100), "Court");
  assert.equal(fitOneLine(m, "Un titre beaucoup trop long", 100), "Un titre…");
  assert.equal(fitOneLine(m, "Beyoncé", 100), "Beyoncé");
  assert.equal(fitOneLine(m, "", 100), "");
  assert.equal(fitOneLine(m, "rien", 0), "");
  // the ellipsis is inside the measured width, never past it
  assert.ok([...fitOneLine(m, "Un titre beaucoup trop long", 100)].length * 10 <= 100);
});

test("fitLyricLayout picks the largest size whose block fits", () => {
  // every glyph is 0.5 * the font size wide
  const measureAt = (size) => ({ measureText: (s) => ({ width: [...String(s)].length * size * 0.5 }) });
  const short = fitLyricLayout(measureAt, ["Un vers court"], { maxWidth: 800, maxHeight: 400 });
  assert.equal(short.size, 96);
  assert.deepEqual(short.rows, ["Un vers court"]);
  assert.deepEqual(short.blocks, [["Un vers court"]]);

  const long = "Un vers bien plus long que les autres, avec assez de mots pour occuper plusieurs lignes";
  const fitted = fitLyricLayout(measureAt, [long, "Et un second vers"], {
    maxWidth: 800,
    maxHeight: 300,
    sizes: [96, 64, 48, 32],
  });
  assert.ok(fitted.size < 96, `picked ${fitted.size}`);
  assert.equal(fitted.blocks.length, 2);
  assert.ok(fitted.rows.length * fitted.size * 1.24 + fitted.size * 0.14 <= 300, "the chosen block fits its height");
  for (const row of fitted.rows) {
    assert.ok([...row].length * fitted.size * 0.5 <= 800, `"${row}" fits the width`);
  }
});

test("three full lyric lines fit the card instead of hiding the last one", () => {
  // 0.52 em per glyph is close to Bricolage Grotesque's average at weight 800.
  const measureAt = (size) => ({ measureText: (s) => ({ width: [...String(s)].length * size * 0.52 }) });
  const texts = [
    "Je marche dans la nuit avec des étoiles plein les yeux et le cœur qui bat trop fort",
    "On ne rentrera pas avant que le jour se lève sur les toits de la ville endormie",
    "Et si tu veux on prendra le temps de regarder le monde tourner sans nous attendre",
  ];
  const fitted = fitLyricLayout(measureAt, texts, { maxWidth: 888, maxHeight: 760, lineHeight: 1.24 });
  assert.ok(fitted.size < 64, `picked ${fitted.size} px`);
  assert.equal(fitted.blocks.length, 3, "each selected line stays a block");
  assert.ok(fitted.rows.length * fitted.size * 1.24 + 2 * fitted.size * 0.14 <= 760, "the whole block fits");
  // every selected line is on the card, including the end of the third
  const all = fitted.rows.join(" ");
  for (const text of texts) {
    assert.ok(all.includes(text.split(" ").at(-1)), `missing the end of "${text}"`);
  }
});

test("fitLyricLayout handles empty text and an impossible block", () => {
  const measureAt = (size) => measurer(size * 0.5);
  assert.deepEqual(fitLyricLayout(measureAt, [], { maxWidth: 800, maxHeight: 300 }), {
    size: LYRIC_SIZES[0],
    rows: [],
    blocks: [],
  });
  const tiny = fitLyricLayout(measureAt, ["Un texte vraiment long ".repeat(20)], {
    maxWidth: 100,
    maxHeight: 20,
    sizes: [64, 32],
  });
  assert.equal(tiny.size, 32); // the smallest, and the draw clips it
  assert.ok(tiny.rows.length > 0);
});

test("cardFilename is a safe neezer-<artist>-<title>.png", () => {
  assert.equal(cardFilename("Beyoncé", "Halo"), "neezer-beyonce-halo.png");
  assert.equal(cardFilename("AC/DC", "Back: In Black?"), "neezer-ac-dc-back-in-black.png");
  assert.equal(cardFilename("D'Angelo", "Untitled (How Does It Feel)"), "neezer-d-angelo-untitled-how-does-it-feel.png");
  assert.equal(cardFilename("", "Seule"), "neezer-seule.png");
  assert.equal(cardFilename(null, null), "neezer-titre.png");
  assert.equal(cardFilename("Artiste", "???"), "neezer-artiste-titre.png");
  assert.equal(cardFilename("Artiste", "  ...  "), "neezer-artiste-titre.png");
  assert.equal(cardFilename("A\u0000B", "T\u0007itre"), "neezer-ab-titre.png");
  assert.equal(cardFilename("Artiste", "a".repeat(300)).length, "neezer-artiste-".length + 64 + 4);
  assert.ok(!cardFilename("Artiste", "a".repeat(300)).includes("--"));
  assert.match(cardFilename("Artiste", "Titre"), /^neezer-[a-z0-9-]+\.png$/);
  assert.ok(cardFilename("Artiste", "Titre").endsWith(".png"));
});
