// The Récap's decisions, held as pure rules (src/lib/recap.js).
//
// No browser here: card order, the French copy, the numbers, the year chips and
// the poster lines are exactly what can be decided without a DOM. The screen
// and the poster are opened in a real browser by the feature's bench, not
// asserted through a screenshot in this suite.
//
// The expected strings use a literal U+202F (narrow no-break space) where the
// French thousands separator belongs: that is the whole point of formatNumber.

import test from "node:test";
import assert from "node:assert/strict";

import {
  cardIds,
  slideTitle,
  slideKicker,
  formatNumber,
  formatMinutes,
  formatSpan,
  monthBars,
  busiestMonth,
  rankRows,
  mosaicCovers,
  genreRows,
  periodLabel,
  yearChips,
  advertisedYear,
  emptyReason,
  shareText,
  posterLines,
  ellipsis,
} from "../src/lib/recap.js";

const TITLE_LONG = "Le très long titre qui ne doit jamais déborder de l'affiche partagée";

function payload(overrides = {}) {
  return {
    year: 2026,
    partial: true,
    enough: true,
    period: { from: "2026-01-01", to: "2026-11-13" },
    totals: {
      events: 2412,
      listens: 2180,
      skips: 190,
      neutrals: 42,
      seconds: 512400,
      minutes: 8540,
      days_active: 211,
    },
    months: [
      { month: 1, listens: 180, skips: 12, seconds: 41000 },
      { month: 2, listens: 165, skips: 9, seconds: 37120 },
      { month: 3, listens: 240, skips: 14, seconds: 52000 },
      { month: 4, listens: 210, skips: 11, seconds: 46000 },
      { month: 5, listens: 190, skips: 8, seconds: 42000 },
      { month: 6, listens: 170, skips: 10, seconds: 38000 },
      { month: 7, listens: 300, skips: 16, seconds: 66000 },
      { month: 8, listens: 120, skips: 6, seconds: 26000 },
      { month: 9, listens: 200, skips: 12, seconds: 43000 },
      { month: 10, listens: 225, skips: 15, seconds: 48000 },
      { month: 11, listens: 180, skips: 9, seconds: 37000 },
      { month: 12, listens: 0, skips: 0, seconds: 0 },
    ],
    streak: { days: 23, from: "2026-03-01", to: "2026-03-23" },
    biggest_day: { date: "2026-07-04", listens: 61, seconds: 14200 },
    tops: {
      tracks: [
        { track: { id: "t1", title: TITLE_LONG, album: { cover: "cover-a" } }, listens: 41 },
        {
          track: { id: "t2", title: "Titre deux", album: { cover: "cover-b" } },
          listens: 34,
        },
        { track: { id: "t3", title: "Titre trois", album: { cover: "cover-a" } }, listens: 28 },
        { track: { id: "t4", title: "Titre quatre", album: { cover: null } }, listens: 22 },
        { track: { id: "t5", title: "Titre cinq", album: { cover: "cover-c" } }, listens: 19 },
        { track: { id: "t6", title: "Titre six", album: { cover: "cover-d" } }, listens: 12 },
      ],
      artists: [
        { id: "a1", name: "June", cover: "cover-a", listens: 210, tracks: 14 },
        { id: "a2", name: "Avril", cover: null, listens: 190, tracks: 9 },
      ],
      albums: [
        { id: "al1", title: "Premier", artist: "June", cover: "cover-a", listens: 96 },
        { id: "al2", title: "Deuxième", artist: "Avril", cover: "cover-e", listens: 80 },
      ],
      genres: {
        available: true,
        coverage: 0.72,
        measured_listens: 1570,
        explored: 17,
        rows: [
          { name: "Techno", family: "techno", listens: 640, share: 0.41 },
          { name: "House", family: "house", listens: 380, share: 0.24 },
        ],
      },
    },
    first: { at: "2026-01-02T08:11:00", track: { id: "t1", title: "Titre un" } },
    last: { at: "2026-11-13T20:58:00", track: { id: "t3", title: "Titre trois" } },
    cards: ["year", "minutes", "artists", "tracks", "albums", "genres", "streak", "firstlast"],
    ...overrides,
  };
}

test("formatNumber groups with a narrow no-break space", () => {
  assert.equal(formatNumber(0), "0");
  assert.equal(formatNumber(1), "1");
  assert.equal(formatNumber(1000), "1\u202f000");
  assert.equal(formatNumber(8540), "8\u202f540");
  assert.equal(formatNumber(1234567), "1\u202f234\u202f567");
  assert.equal(formatNumber("8540"), "8\u202f540");
  assert.equal(formatNumber(NaN), "0");
});

test("formatMinutes words the count, and French counts zero as singular", () => {
  assert.equal(formatMinutes(0), "0 minute");
  assert.equal(formatMinutes(1), "1 minute");
  assert.equal(formatMinutes(1000), "1\u202f000 minutes");
  assert.equal(formatMinutes(8540), "8\u202f540 minutes");
});

test("formatSpan picks minutes, hours, then days", () => {
  assert.equal(formatSpan(0), "0 min");
  assert.equal(formatSpan(42 * 60), "42 min");
  assert.equal(formatSpan(3 * 3600), "3 h");
  assert.equal(formatSpan(3 * 3600 + 12 * 60), "3 h 12");
  assert.equal(formatSpan(5 * 86400), "5 j");
  assert.equal(formatSpan(5 * 86400 + 22 * 3600), "5 j 22 h");
  assert.equal(formatSpan(NaN), "0 min");
});

test("cardIds keeps the server's order, drops unknown ids, appends share last", () => {
  const ids = cardIds(payload());
  assert.deepEqual(ids, [
    "year",
    "minutes",
    "artists",
    "tracks",
    "albums",
    "genres",
    "streak",
    "firstlast",
    "share",
  ]);
  assert.deepEqual(
    cardIds(payload({ cards: ["minutes", "year", "nope", "minutes"] })),
    ["minutes", "year", "share"]
  );
  assert.deepEqual(cardIds(payload({ cards: [] })), []);
  assert.deepEqual(cardIds(payload({ enough: false })), [
    "year",
    "minutes",
    "artists",
    "tracks",
    "albums",
    "genres",
    "streak",
    "firstlast",
  ]);
  assert.deepEqual(cardIds(null), []);
});

test("a deck with no card is not a deck: share is never shown alone", () => {
  assert.deepEqual(cardIds(payload({ cards: [], enough: true })), []);
});

test("slideTitle carries the exact French copy of the plan", () => {
  const p = payload();
  assert.equal(slideTitle(p, "year"), "Ton année 2026");
  assert.equal(slideTitle(p, "minutes"), "8\u202f540 minutes");
  assert.equal(slideTitle(p, "artists"), "Tes 5 artistes");
  assert.equal(slideTitle(p, "tracks"), "Tes 5 titres");
  assert.equal(slideTitle(p, "albums"), "Tes 5 albums");
  assert.equal(slideTitle(p, "genres"), "Tes styles");
  assert.equal(slideTitle(p, "streak"), "23 jours d'affilée");
  assert.equal(slideTitle(p, "firstlast"), "Ton premier titre de l'année");
  assert.equal(slideTitle(p, "share"), "Partage ton année");
  assert.equal(slideTitle(p, "unknown"), "");
});

test("slideKicker names the running year, then the reader's place", () => {
  assert.equal(slideKicker(payload(), "year"), "2026 · en cours");
  assert.equal(
    slideKicker(payload({ year: 2025, partial: false, period: { from: "2025-01-01", to: "2025-12-31" } }), "year"),
    "2025"
  );
  assert.equal(slideKicker(payload(), "artists"), "3 sur 9");
  assert.equal(slideKicker(payload(), "share"), "9 sur 9");
  assert.equal(slideKicker(payload(), "unknown"), "");
});

test("monthBars is always 12 zero-filled bars, normalised to the busiest", () => {
  const bars = monthBars(payload());
  assert.equal(bars.length, 12);
  assert.deepEqual(
    bars.map((b) => b.month),
    [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
  );
  assert.equal(Math.max(...bars.map((b) => b.pct)), 100);
  assert.equal(bars[6].listens, 300);
  assert.equal(bars[6].pct, 100);
  assert.equal(bars[11].listens, 0);
  assert.equal(bars[11].pct, 0);

  const partial = monthBars({ months: [{ month: 1, listens: 5 }] });
  assert.equal(partial.length, 12);
  assert.equal(partial[0].pct, 100);
  assert.equal(partial[1].listens, 0);

  const empty = monthBars({});
  assert.equal(empty.length, 12);
  assert.deepEqual(
    empty.map((b) => b.pct),
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
  );
});

test("busiestMonth names the strongest month in full, or nothing", () => {
  assert.deepEqual(busiestMonth(payload()), { label: "juillet", listens: 300 });
  assert.equal(busiestMonth({ months: [] }), null);
  assert.equal(busiestMonth({}), null);
});

test("rankRows never invents a row", () => {
  assert.deepEqual(rankRows({}, "tracks"), []);
  assert.deepEqual(rankRows({ tops: null }, "artists"), []);
  assert.deepEqual(rankRows({ tops: { tracks: "nope" } }, "tracks"), []);
  assert.equal(rankRows(payload(), "tracks").length, 5);
  assert.equal(rankRows(payload(), "albums").length, 2);
  assert.deepEqual(rankRows(payload(), "genres"), []);
});

test("genreRows is empty unless the server vouched for the measurement", () => {
  assert.deepEqual(genreRows({}), []);
  assert.deepEqual(genreRows({ tops: { genres: { available: false, rows: [{ name: "Techno" }] } } }), []);
  const rows = genreRows(payload());
  assert.equal(rows.length, 2);
  assert.equal(rows[0].name, "Techno");
});

test("mosaicCovers is at most four unique sleeves, albums then titles", () => {
  const covers = mosaicCovers(payload());
  assert.ok(covers.length <= 4);
  assert.equal(new Set(covers).size, covers.length);
  assert.deepEqual(covers, ["cover-a", "cover-e", "cover-b", "cover-c"]);
  assert.deepEqual(mosaicCovers({}), []);
});

test("yearChips uses the server's nowYear, never the device clock", () => {
  const chips = yearChips({
    nowYear: 2026,
    years: [
      { year: 2026, partial: true, enough: true, events: 2412 },
      { year: 2025, partial: false, enough: true, events: 9120 },
      { year: 2024, partial: false, enough: false, events: 0 },
    ],
  });
  assert.deepEqual(chips, [
    { year: 2026, label: "2026 · en cours" },
    { year: 2025, label: "2025" },
  ]);
  // Without the server's year nothing is called "en cours".
  assert.deepEqual(yearChips({ years: [{ year: 2026, events: 1 }] }), [
    { year: 2026, label: "2026" },
  ]);
  assert.deepEqual(yearChips({ years: [] }), []);
});

test("advertisedYear only advertises this year or the one before", () => {
  const state = {
    nowYear: 2026,
    years: [
      { year: 2026, partial: true, enough: true, events: 2412, listens: 2180, minutes: 8540 },
      { year: 2025, partial: false, enough: true, events: 9120, listens: 8402, minutes: 30211 },
    ],
  };
  assert.equal(advertisedYear(state).year, 2026);
  // The current year is thin: last year's full story is the honest banner.
  assert.equal(
    advertisedYear({
      nowYear: 2026,
      years: [
        { year: 2026, enough: false, events: 5 },
        { year: 2025, enough: true, events: 9120 },
      ],
    }).year,
    2025
  );
  // In 2030 there is no eternal "Ton année 2026".
  assert.equal(advertisedYear({ nowYear: 2030, years: state.years }), null);
  assert.equal(advertisedYear({ nowYear: 2026, years: [] }), null);
  assert.equal(advertisedYear({ years: state.years }), null);
  // A year without `enough` never advertises anything.
  assert.equal(advertisedYear({ nowYear: 2026, years: [{ year: 2026, enough: false, events: 5 }] }), null);
});

test("emptyReason follows the person's own data", () => {
  assert.equal(emptyReason(payload({ totals: { events: 0, listens: 0 }, cards: [] })), "none");
  assert.equal(
    emptyReason(payload({ totals: { events: 3, listens: 0 }, enough: false, cards: [] })),
    "skips-only"
  );
  assert.equal(
    emptyReason(payload({ totals: { events: 5, listens: 5, minutes: 12 }, enough: false, cards: [] })),
    "thin"
  );
  assert.equal(emptyReason(payload()), null);
  assert.equal(emptyReason(null), "none");
});

test("periodLabel distinguishes a finished year from the running one", () => {
  assert.equal(
    periodLabel({ year: 2025, partial: false, period: { from: "2025-01-01", to: "2025-12-31" } }),
    "toute l'année 2025"
  );
  assert.equal(periodLabel(payload()), "du 1ᵉʳ janvier au 13 novembre 2026");
  assert.equal(
    periodLabel({ year: 2025, partial: true, period: { from: "2025-05-03", to: "2025-12-23" } }),
    "du 3 mai au 23 décembre 2025"
  );
});

test("shareText is one exact sentence", () => {
  assert.equal(
    shareText(payload()),
    "Mon année 2026 (en cours) en musique : 2\u202f180 écoutes, 8\u202f540 minutes, avec June en tête."
  );
  assert.equal(
    shareText(
      payload({
        partial: false,
        year: 2025,
        totals: { listens: 1, minutes: 1 },
        tops: { artists: [], tracks: [], albums: [], genres: { available: false, rows: [] } },
      })
    ),
    "Mon année 2025 en musique : 1 écoute, 1 minute."
  );
});

test("posterLines gives six bounded lines", () => {
  const lines = posterLines(payload());
  assert.equal(lines.length, 6);
  for (const line of lines) assert.ok(line.length <= 38, `"${line}" is ${line.length} chars`);
  assert.equal(lines[0], "Ton année 2026, en cours");
  assert.equal(lines[1], "8\u202f540 minutes");
  assert.equal(lines[2], "2\u202f180 écoutes");
  assert.equal(lines[3], "Artiste : June");
  assert.match(lines[4], /^Titre : /);
  assert.ok(lines[4].endsWith("…"));
  assert.equal(lines[5], "Ton mois le plus fort : juillet");
});

test("ellipsis cuts to the maximum, ellipsis included", () => {
  assert.equal(ellipsis("abcdef", 10), "abcdef");
  assert.equal(ellipsis("abcdef", 4), "abc…");
  assert.equal(ellipsis("abcdef", 1), "…");
  assert.equal(ellipsis("  a\n b  ", 10), "a b");
  assert.ok(ellipsis(TITLE_LONG, 38).length <= 38);
});
