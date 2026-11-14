import test from "node:test";
import assert from "node:assert/strict";
import { get } from "svelte/store";
import { api } from "../src/lib/api.js";
import {
  chaptersByEpisode,
  normalizeChapters,
  chapterAt,
  nextChapter,
  chapterEnd,
  chapterLabel,
  chapterCountLabel,
  podcastAbout,
  dateLabel,
  loadChapters,
} from "../src/lib/chapters.js";

const ch = (start, end, title) => ({ start, end, title });

test("normalizeChapters sorts, rounds, trims, drops unreadable starts and keeps one per start", () => {
  const raw = [
    ch(25.4, 30.6, "  Questions  "),
    { index: 0, start: "0", end: null, title: "Introduction" },
    ch(10, 9, "Inversé"),
    { title: "Sans début" },
    ch(-2, 4, "Négatif"),
    ch(10.2, 25, "Doublon"),
    null,
    "x",
  ];
  assert.deepEqual(normalizeChapters(raw), [
    { start: 0, end: null, title: "Introduction" },
    { start: 10, end: null, title: "Inversé" },
    { start: 25, end: 31, title: "Questions" },
  ]);
});

test("normalizeChapters never invents a chapter from an unreadable answer", () => {
  assert.deepEqual(normalizeChapters(null), []);
  assert.deepEqual(normalizeChapters({ chapters: [ch(0, 10, "X")] }), []);
  assert.deepEqual(normalizeChapters([{}, { start: "abc" }, { start: NaN }]), []);
});

test("chapterAt finds the chapter containing a position, by binary search", () => {
  const list = normalizeChapters([ch(0, 60, "A"), ch(60, 150, "B"), ch(150, 240, "C")]);
  assert.equal(chapterAt(list, 0).index, 0);
  assert.equal(chapterAt(list, 59).index, 0);
  assert.equal(chapterAt(list, 60).index, 1);
  assert.equal(chapterAt(list, 149.9).index, 1);
  assert.equal(chapterAt(list, 150).index, 2);
  assert.equal(chapterAt(list, 239).index, 2);
  assert.equal(chapterAt(list, 240), null);
  assert.equal(chapterAt(list, -1), null);
  assert.equal(chapterAt([], 10), null);
});

test("chapterAt reports a gap when the current chapter has ended", () => {
  const list = normalizeChapters([ch(0, 20, "A"), ch(40, null, "B")]);
  assert.equal(chapterAt(list, 10).index, 0);
  assert.equal(chapterAt(list, 25), null);
  assert.equal(chapterAt(list, 40).index, 1);
});

test("chapterAt keeps an open-ended last chapter to the end of the episode", () => {
  const list = normalizeChapters([ch(120, null, "Fin")]);
  assert.equal(chapterAt(list, 120).index, 0);
  assert.equal(chapterAt(list, 900).index, 0);
});

test("nextChapter names the upcoming chapter, before the first and in a gap", () => {
  const list = normalizeChapters([ch(20, 60, "A"), ch(60, 150, "B"), ch(150, 240, "C")]);
  assert.equal(nextChapter(list, 0).index, 0);
  assert.equal(nextChapter(list, 70).index, 2);
  assert.equal(nextChapter(list, 160), null);
  assert.equal(nextChapter([], 0), null);
});

test("chapterEnd uses the tag's end, then the next start, then the episode duration", () => {
  assert.equal(chapterEnd([ch(0, 42, "A"), ch(90, null, "B")], 0, 300), 42);
  assert.equal(chapterEnd([ch(0, null, "A"), ch(90, null, "B")], 0, 300), 90);
  assert.equal(chapterEnd([ch(0, null, "A")], 0, 300), 300);
  assert.equal(chapterEnd([ch(10, null, "A")], 0, 4), 10);
  assert.equal(chapterEnd([], 0, 300), 0);
});

test("chapterLabel is one-based and only adds a separator with a title", () => {
  const list = [ch(0, 10, "Introduction"), ch(10, 20, ""), ch(20, 30, "Fin")];
  assert.equal(chapterLabel(list, 0), "Chapitre 1 · Introduction");
  assert.equal(chapterLabel(list, 1), "Chapitre 2");
  assert.equal(chapterLabel(list, 2), "Chapitre 3 · Fin");
  assert.equal(chapterLabel(list, 9), "");
});

test("chapterCountLabel says the singular for one and zero", () => {
  assert.equal(chapterCountLabel(0), "0 chapitre");
  assert.equal(chapterCountLabel(1), "1 chapitre");
  assert.equal(chapterCountLabel(2), "2 chapitres");
  assert.equal(chapterCountLabel(12), "12 chapitres");
  assert.equal(chapterCountLabel("3"), "3 chapitres");
});

test("podcastAbout only opens a door that shows something", () => {
  assert.equal(podcastAbout({ podcast: true, description: "  ", chapter_count: 0 }), false);
  assert.equal(podcastAbout({ podcast: true, description: "Un épisode" }), true);
  assert.equal(podcastAbout({ podcast: true, chapter_count: 3 }), true);
  assert.equal(podcastAbout({ podcast: true, archived: true }), true);
  assert.equal(podcastAbout({ podcast: false, description: "Un titre" }), false);
  assert.equal(podcastAbout(null), false);
});

test("dateLabel formats a publication date in French and refuses zero", () => {
  assert.equal(dateLabel(0), "");
  assert.equal(dateLabel(null), "");
  const ts = Date.UTC(2026, 0, 12, 12) / 1000;
  const label = dateLabel(ts);
  assert.match(label, /2026/);
  assert.match(label, /janv/);
});

test("loadChapters asks once, dedupes in flight and serves the memory copy", async () => {
  const original = api.swr;
  const calls = [];
  let resolveFirst;
  api.swr = (path) => {
    calls.push(path);
    return new Promise((resolve) => (resolveFirst = resolve));
  };
  chaptersByEpisode.set({});
  try {
    const a = loadChapters("ep-a");
    const b = loadChapters("ep-a");
    assert.equal(a, b);
    assert.equal(calls.length, 1);
    resolveFirst({
      archived: true,
      scanned: true,
      source: "id3",
      duration: 240,
      chapters: [ch(0, 60, "A"), ch(60, null, "B"), ch(60, 90, "Doublon")],
    });
    const entry = await a;
    assert.equal(entry.state, "ready");
    assert.equal(entry.archived, true);
    assert.equal(entry.duration, 240);
    assert.equal(entry.chapters.length, 2);
    assert.equal(get(chaptersByEpisode)["ep-a"].chapters[1].end, null);

    const again = await loadChapters("ep-a");
    assert.equal(again, entry);
    assert.equal(calls.length, 1);

    const forced = loadChapters("ep-a", { force: true });
    assert.equal(calls.length, 2);
    resolveFirst({ archived: true, scanned: true, source: "id3", duration: 240, chapters: [ch(0, null, "Seul")] });
    const fresh = await forced;
    assert.equal(fresh.chapters.length, 1);
  } finally {
    api.swr = original;
    chaptersByEpisode.set({});
  }
});

test("loadChapters is fail-soft: an offline failure is an error state, never a throw", async () => {
  const original = api.swr;
  chaptersByEpisode.set({});
  api.swr = () => Promise.reject({ status: 0, message: "network", offline: true });
  try {
    const entry = await loadChapters("ep-offline");
    assert.equal(entry.state, "error");
    assert.equal(entry.error.offline, true);
    assert.deepEqual(entry.chapters, []);
    assert.equal(get(chaptersByEpisode)["ep-offline"].state, "error");
  } finally {
    api.swr = original;
    chaptersByEpisode.set({});
  }
});

test("a cached copy that cannot be revalidated stays readable and says it is offline", async () => {
  const original = api.swr;
  chaptersByEpisode.set({});
  api.swr = (_path, onData) => {
    onData({ archived: true, scanned: true, source: "id3", duration: 300, chapters: [ch(0, 300, "A")] }, true);
    return Promise.reject({ status: 0, message: "network", offline: true });
  };
  try {
    const entry = await loadChapters("ep-cached");
    assert.equal(entry.state, "ready");
    assert.equal(entry.offline, true);
    assert.equal(entry.cached, true);
    assert.equal(entry.chapters.length, 1);
  } finally {
    api.swr = original;
    chaptersByEpisode.set({});
  }
});
