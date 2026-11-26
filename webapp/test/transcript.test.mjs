import test from "node:test";
import assert from "node:assert/strict";
import { get } from "svelte/store";
import { fakeIndexedDB } from "./fakeidb.mjs";
import {
  batchOptions,
  destinationText,
  excerptAround,
  foldText,
  highlightParts,
  hitEpisode,
  languageLabel,
  machinesText,
  pageInfo,
  pageKey,
  panelRequest,
  reasonText,
  requestEpisodePanel,
  retryAfterSeconds,
  segmentAt,
  takePanelRequest,
  transcriptNote,
  noteTranscript,
  withRateLimitRetry,
} from "../src/lib/transcript.js";
import {
  MAX_BYTES,
  cacheStats,
  cacheReadAllowed,
  cacheWriteAllowed,
  clearTranscriptCache,
  firstFor,
  forget,
  readAt,
  readPage,
  resetTranscriptCache,
  searchCached,
  writePage,
} from "../src/lib/transcriptcache.js";

globalThis.indexedDB = fakeIndexedDB();

function seg(start, end, text) {
  return { start, end, text };
}

const EP = {
  id: "e1",
  title: "La mémoire de travail",
  cover: "/api/cover/e1",
  channel_id: "c1",
  channel_title: "Le Labo",
  duration: 3600,
  album: { title: "Le Labo" },
};

function page(offset, total = 500, extra = {}) {
  return {
    episode_id: "e1",
    status: "ready",
    limit: 100,
    offset,
    total,
    language: "fr",
    duration_ms: 3600000,
    segments: [seg(offset, offset + 4, `Segment ${offset} sur la mémoire`)],
    ...extra,
  };
}

test("segmentAt returns the last segment whose start is reached", () => {
  const segments = [seg(0, 5, "a"), seg(10, 15, "b"), seg(10, 12, "b2"), seg(30, 40, "c")];
  assert.equal(segmentAt(segments, -1), -1);
  assert.equal(segmentAt(segments, 0), 0);
  assert.equal(segmentAt(segments, 9.9), 0);
  assert.equal(segmentAt(segments, 10), 2);
  assert.equal(segmentAt(segments, 22), 2);
  assert.equal(segmentAt(segments, 99), 3);
  assert.equal(segmentAt([], 10), -1);
  assert.equal(segmentAt(null, 10), -1);
});

test("pageInfo clamps the offset and handles a partial last page", () => {
  assert.deepEqual(pageInfo(0, 200, 0), { page: 1, pages: 1, from: 0, to: 0 });
  assert.deepEqual(pageInfo(0, 200, 500), { page: 1, pages: 3, from: 0, to: 200 });
  assert.deepEqual(pageInfo(400, 200, 500), { page: 3, pages: 3, from: 400, to: 500 });
  assert.deepEqual(pageInfo(9999, 200, 500), { page: 3, pages: 3, from: 400, to: 500 });
  assert.deepEqual(pageInfo(-40, 200, 500), { page: 1, pages: 3, from: 0, to: 200 });
});

test("batchOptions offers only what the eligible episodes can fill", () => {
  assert.deepEqual(batchOptions(0), []);
  assert.deepEqual(batchOptions(1), [1]);
  assert.deepEqual(batchOptions(3), [3]);
  assert.deepEqual(batchOptions(5), [5]);
  assert.deepEqual(batchOptions(9), [5]);
  assert.deepEqual(batchOptions(10), [5, 10]);
  assert.deepEqual(batchOptions(11), [5, 10]);
  assert.deepEqual(batchOptions(20), [5, 10, 20]);
  assert.deepEqual(batchOptions(50), [5, 10, 20]);
});

test("highlightParts marks exact occurrences only, case-insensitively", () => {
  assert.deepEqual(highlightParts("La Mémoire vive", "mémoire"), [
    { t: "La ", mark: false },
    { t: "Mémoire", mark: true },
    { t: " vive", mark: false },
  ]);
  assert.deepEqual(highlightParts("memoire", "mémoire"), [{ t: "memoire", mark: false }]);
  assert.deepEqual(highlightParts("un 100 %_ juste", "100 %_"), [
    { t: "un ", mark: false },
    { t: "100 %_", mark: true },
    { t: " juste", mark: false },
  ]);
  assert.deepEqual(highlightParts("aaaa", "aa"), [
    { t: "aa", mark: true },
    { t: "aa", mark: true },
  ]);
  assert.deepEqual(highlightParts("rien", ""), [{ t: "rien", mark: false }]);
  assert.deepEqual(highlightParts("", "abc"), [{ t: "", mark: false }]);
});

test("excerptAround keeps a window around the match, within the budget", () => {
  const text = "x".repeat(300) + " mémoire " + "y".repeat(300);
  const excerpt = excerptAround(text, "mémoire", 240);
  assert.ok(excerpt.length <= 240);
  assert.ok(excerpt.includes("mémoire"));
  assert.ok(excerpt.startsWith("…"));
  assert.ok(excerpt.endsWith("…"));
  assert.equal(excerptAround("court", "court", 240), "court");
  const start = excerptAround("mémoire " + "z".repeat(400), "mémoire", 240);
  assert.ok(start.startsWith("mémoire"));
  assert.ok(start.endsWith("…"));
});

test("languageLabel and foldText stay honest", () => {
  assert.equal(languageLabel("fr"), "Français");
  assert.equal(languageLabel("en"), "Anglais");
  assert.equal(languageLabel("es"), "Espagnol");
  assert.equal(languageLabel("de"), "Allemand");
  assert.equal(languageLabel("it"), "Italien");
  assert.equal(languageLabel("pt"), "Portugais");
  assert.equal(languageLabel("auto"), "Détection automatique");
  assert.equal(languageLabel("ru"), "RU");
  assert.equal(languageLabel(null), "");
  assert.equal(foldText("Mémoire"), "memoire");
  assert.equal(foldText("Привет"), "привет");
});

test("reasonText names each cause and never claims a configuration", () => {
  assert.match(reasonText("disabled"), /désactivée/);
  assert.match(reasonText("engine_missing"), /moteur/);
  assert.match(reasonText("model_missing"), /modèle/);
  assert.match(reasonText("model_invalid"), /whisper\.cpp/);
  assert.match(reasonText(null), /n'est pas configurée/);
});

test("reasonText says when nothing can run the queue at all", () => {
  // The reported bug: a server with no whisper.cpp refused activation even
  // though an enrolled PC was there. The gate now names both halves, so the
  // sentence must too.
  assert.match(reasonText("no_worker"), /Aucun ordinateur n'est enrôlé/);
  assert.match(reasonText("no_worker"), /serveur n'a pas de moteur/);
});

test("destinationText says where a queued episode will run", () => {
  assert.equal(destinationText("server", { enrolled: 0, online: 0 }), "Sur ce serveur");
  assert.equal(
    destinationText("external", { enrolled: 1, online: 1 }),
    "Dans le conteneur de transcription",
  );
  assert.equal(
    destinationText("workers", { enrolled: 1, online: 1 }),
    "Sur l'ordinateur enrôlé, en ligne",
  );
  // An unreachable PC is not a verdict: the queue still accepts, so the
  // sentence says the PC is offline right now, never that nothing runs.
  assert.equal(
    destinationText("workers", { enrolled: 1, online: 0 }),
    "Sur l'ordinateur enrôlé, hors ligne pour l'instant",
  );
  assert.equal(
    destinationText("workers", { enrolled: 2, online: 1 }),
    "Sur 2 ordinateurs enrôlés, 1 en ligne",
  );
  assert.equal(destinationText(null, null), "");
});

test("machinesText lists each PC and its state, never an empty line", () => {
  assert.equal(machinesText(null), "");
  assert.equal(machinesText([]), "");
  assert.equal(
    machinesText([
      { name: "Bureau", online: true },
      { name: "Portable", online: false },
    ]),
    "Bureau (en ligne), Portable (hors ligne)",
  );
});

test("a hit becomes a playable episode without another request", () => {
  const episode = hitEpisode({
    episode_id: "e1",
    episode_title: "La mémoire de travail",
    channel_id: "c1",
    channel_title: "Le Labo",
    start: 512,
    episode: { id: "e1", title: "La mémoire de travail", cover: "/cover", duration: 3600 },
  });
  assert.equal(episode.id, "e1");
  assert.equal(episode.deezer_id, "e1");
  assert.equal(episode.podcast, true);
  assert.equal(episode.duration, 3600);
  assert.equal(episode.channel_id, "c1");
  assert.equal(episode.album.title, "Le Labo");
  assert.equal(episode.cover, "/cover");
  assert.equal(episode.transcript.status, "ready");
});

test("the panel request and the transcript note are short-lived and sequenced", () => {
  panelRequest.set(null);
  requestEpisodePanel("e1", "transcript", 42);
  const request = takePanelRequest();
  assert.equal(request.episodeId, "e1");
  assert.equal(request.position, 42);
  assert.equal(get(panelRequest), null);
  assert.equal(takePanelRequest(), null);

  panelRequest.set({ episodeId: "e1", at: Date.now() - 5000 });
  assert.equal(takePanelRequest(), null);

  noteTranscript("e1", { status: "ready", progress: 1 });
  const note = get(transcriptNote);
  assert.equal(note.episodeId, "e1");
  assert.equal(note.transcript.status, "ready");
});

test("pageKey is stable across a reload", () => {
  assert.equal(pageKey("e1", 200, 400), "e1:200:400");
});

test("cache refuses a remote control and a page that is not ready", async () => {
  assert.equal(cacheWriteAllowed(null), true);
  assert.equal(cacheWriteAllowed({ level: "read" }), false);
  assert.equal(cacheReadAllowed(null), true);
  assert.equal(cacheReadAllowed({ level: "read" }), false);
  resetTranscriptCache();
  await clearTranscriptCache();
  assert.equal(await writePage({ ...page(0), status: "running" }, EP), false);
  assert.equal(await writePage({ ...page(0), segments: [] }, EP), false);
  assert.equal((await cacheStats()).pages, 0);
});

test("the cache reads a page back with its episode card", async () => {
  resetTranscriptCache();
  await clearTranscriptCache();
  assert.equal(await writePage(page(0), EP), true);
  const record = await readPage("e1", 100, 0);
  assert.equal(record.total, 500);
  assert.equal(record.language, "fr");
  assert.equal(record.episode.title, "La mémoire de travail");
  assert.equal(record.episode.channel_title, "Le Labo");
  assert.equal(record.segments[0].text, "Segment 0 sur la mémoire");
  assert.equal(await readPage("e1", 100, 100), null);
});

test("readAt picks the cached page that contains the position", async () => {
  resetTranscriptCache();
  await clearTranscriptCache();
  await writePage(page(0), EP);
  await writePage(page(100), EP);
  const at = await readAt("e1", 100, 150);
  assert.equal(at.offset, 100);
  const before = await readAt("e1", 100, 10);
  assert.equal(before.offset, 0);
});

test("the cache keeps four pages per episode, newest first", async () => {
  resetTranscriptCache();
  await clearTranscriptCache();
  for (let offset = 0; offset <= 400; offset += 100) await writePage(page(offset), EP);
  const stats = await cacheStats();
  assert.equal(stats.pages, 4);
  assert.equal(await readPage("e1", 100, 0), null);
  assert.equal((await firstFor("e1")).offset, 100);
});

test("a different total purges the episode before writing", async () => {
  resetTranscriptCache();
  await clearTranscriptCache();
  await writePage(page(0, 500), EP);
  await writePage(page(100, 500), EP);
  await writePage(page(0, 600), EP);
  const stats = await cacheStats();
  assert.equal(stats.pages, 1);
  assert.equal((await firstFor("e1")).offset, 0);
  assert.equal((await readPage("e1", 100, 0)).total, 600);
});

test("forget removes one episode and leaves the others", async () => {
  resetTranscriptCache();
  await clearTranscriptCache();
  await writePage(page(0), EP);
  await writePage({ ...page(0), episode_id: "e2" }, { ...EP, id: "e2" });
  await forget("e1");
  assert.equal(await readPage("e1", 100, 0), null);
  assert.notEqual(await readPage("e2", 100, 0), null);
});

test("the cache never exceeds fifty pages", async () => {
  resetTranscriptCache();
  await clearTranscriptCache();
  for (let index = 0; index < 55; index++) {
    const id = "e" + index;
    await writePage(
      { ...page(0, 100), episode_id: id, segments: [seg(0, 4, `mémoire ${index}`)] },
      { ...EP, id },
    );
  }
  assert.equal((await cacheStats()).pages, 50);
});

test("the cache never keeps more than five megabytes", async () => {
  resetTranscriptCache();
  await clearTranscriptCache();
  const big = "z".repeat(1_900_000);
  for (let index = 0; index < 3; index++) {
    const id = "big" + index;
    await writePage(
      { ...page(0, 100), episode_id: id, segments: [seg(0, 4, big)] },
      { ...EP, id },
    );
  }
  const stats = await cacheStats();
  assert.ok(stats.bytes <= MAX_BYTES, `${stats.bytes} bytes`);
  assert.equal(stats.pages, 2);
});

test("the offline search finds a folded word in the cached pages", async () => {
  resetTranscriptCache();
  await clearTranscriptCache();
  await writePage(page(0), EP);
  const result = await searchCached("memoire", 20);
  assert.equal(result.local, true);
  assert.equal(result.total, 1);
  assert.equal(result.hits[0].episode_id, "e1");
  assert.equal(result.hits[0].episode.title, "La mémoire de travail");
  assert.ok(result.hits[0].text.includes("mémoire"));
});

test("the API builds the transcript URLs without inventing a query", async () => {
  const calls = [];
  globalThis.fetch = async (url) => {
    calls.push(String(url));
    return {
      ok: true,
      status: 200,
      headers: { get: () => null },
      json: async () => ({ ok: true }),
      clone() {
        return this;
      },
      text: async () => "{}",
    };
  };
  const { api } = await import("../src/lib/api.js");
  for (const name of [
    "transcribeStatus",
    "transcribeSettings",
    "saveTranscribeSettings",
    "setShowTranscribe",
    "startEpisodeTranscript",
    "stopEpisodeTranscript",
    "deleteEpisodeTranscript",
    "episodeTranscript",
    "showTranscriptSearch",
    "searchTranscripts",
  ]) {
    assert.equal(typeof api[name], "function", name);
  }

  calls.length = 0;
  await api.episodeTranscript("a b", { at: 0 });
  assert.equal(calls[0], "/api/podcast/episode/a%20b/transcript?at=0");

  calls.length = 0;
  await api.episodeTranscript("e1", { offset: 200, limit: 100 });
  assert.equal(calls[0], "/api/podcast/episode/e1/transcript?offset=200&limit=100");

  calls.length = 0;
  await api.episodeTranscript("e1");
  assert.equal(calls[0], "/api/podcast/episode/e1/transcript");

  calls.length = 0;
  await api.showTranscriptSearch("c1", "mémoire");
  assert.match(calls[0], /^\/api\/podcast\/c1\/transcript\/search\?q=m%C3%A9moire&limit=50$/);

  calls.length = 0;
  await api.searchTranscripts("mémoire", 20);
  assert.equal(calls[0], "/api/transcripts/search?q=m%C3%A9moire&limit=20");

  const bodies = [];
  globalThis.fetch = async (url, options = {}) => {
    bodies.push({ url: String(url), body: options.body });
    return {
      ok: true,
      status: 200,
      headers: { get: () => null },
      json: async () => ({}),
      clone() {
        return this;
      },
      text: async () => "{}",
    };
  };
  await api.onboardingArtistsDone();
  await api.onboardingArtistsDone({ skip: true });
  assert.equal(bodies[0].body, undefined);
  assert.equal(bodies[1].body, '{"skip":true}');
});

test("a retry_in in the error body is readable by the screen", async () => {
  globalThis.fetch = async () => ({
    ok: false,
    status: 429,
    statusText: "Too Many Requests",
    headers: { get: () => null },
    json: async () => ({ error: "trop de demandes", code: "rate_limited", retry_in: 42 }),
  });
  const { api } = await import("../src/lib/api.js");
  await assert.rejects(
    () => api.searchTranscripts("memoire"),
    (error) => {
      assert.equal(error.status, 429);
      assert.equal(error.retryIn, 42);
      assert.equal(error.body.retry_in, 42);
      assert.equal(error.body.code, "rate_limited");
      return true;
    },
  );
});

test("retryAfterSeconds reads the server's wait and invents none", () => {
  assert.equal(retryAfterSeconds({ status: 429, retryIn: 30 }), 30);
  assert.equal(retryAfterSeconds({ status: 429, body: { retry_in: 12 } }), 12);
  assert.equal(retryAfterSeconds({ status: 429, retryIn: 0.2 }), 1);
  assert.equal(retryAfterSeconds({ status: 429 }), 0);
  assert.equal(retryAfterSeconds({ status: 500, retryIn: 9 }), 9);
  assert.equal(retryAfterSeconds(null), 0);
});

test("withRateLimitRetry waits the server's delay and retries exactly once", async () => {
  const waited = [];
  let calls = 0;
  const value = await withRateLimitRetry(
    async () => {
      calls += 1;
      if (calls === 1) throw { status: 429, retryIn: 7 };
      return "ok";
    },
    { wait: async (seconds) => waited.push(seconds) },
  );
  assert.equal(value, "ok");
  assert.equal(calls, 2);
  assert.deepEqual(waited, [7]);
});

test("withRateLimitRetry tells the caller how long it waits", async () => {
  const notices = [];
  const waited = [];
  await withRateLimitRetry(
    async () => {
      throw { status: 429, body: { retry_in: 4 } };
    },
    {
      wait: async (seconds) => waited.push(seconds),
      onWait: (seconds) => notices.push(seconds),
    },
  ).catch(() => {});
  assert.deepEqual(notices, [4]);
  assert.deepEqual(waited, [4]);
});

test("withRateLimitRetry does not retry a second 429, a non-429 or a bare 429", async () => {
  const waited = [];
  const wait = async (seconds) => waited.push(seconds);
  let calls = 0;
  await assert.rejects(
    withRateLimitRetry(
      async () => {
        calls += 1;
        throw { status: 429, retryIn: 3 };
      },
      { wait },
    ),
    (error) => error.status === 429,
  );
  // One wait, two calls: the second refusal is the person's answer.
  assert.deepEqual(waited, [3]);
  assert.equal(calls, 2);

  calls = 0;
  waited.length = 0;
  await assert.rejects(
    withRateLimitRetry(
      async () => {
        calls += 1;
        throw { status: 500 };
      },
      { wait },
    ),
    (error) => error.status === 500,
  );
  assert.deepEqual(waited, []);
  assert.equal(calls, 1);

  calls = 0;
  waited.length = 0;
  await assert.rejects(
    withRateLimitRetry(
      async () => {
        calls += 1;
        throw { status: 429 };
      },
      { wait },
    ),
    (error) => error.status === 429,
  );
  // No Retry-After: no invented wait, no second call.
  assert.deepEqual(waited, []);
  assert.equal(calls, 1);
});
