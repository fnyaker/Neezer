// The next track's preload (lib/playcache.js), on the two things that made it
// useless on a slow link: it took the whole link from the track being played,
// and it gave up for good on the server's first "not archived yet".
//
// It now reads the body chunk by chunk, and after each chunk sleeps the trickle
// pace while the PLAYER needs the link (lib/netyield.js#prefetchTrickle) — never
// on its own hold, which is always on while it reads. A 503 with Retry-After is
// the server saying "queued, come back", not a refusal: it is retried, at most
// four times, and only while the track is still wanted.
//
// The bandwidth itself is measured in the browser bench (test/prefetch/run.mjs);
// what is pinned here is the seam, on a fake IndexedDB and a fake body.
import test from "node:test";
import assert from "node:assert/strict";
import { fakeIndexedDB } from "./fakeidb.mjs";

function memoryStorage() {
  const rows = new Map();
  return {
    getItem: (k) => (rows.has(k) ? rows.get(k) : null),
    setItem: (k, v) => rows.set(k, String(v)),
    removeItem: (k) => rows.delete(k),
    key: (i) => [...rows.keys()][i] ?? null,
    get length() {
      return rows.size;
    },
  };
}

globalThis.localStorage = memoryStorage();
globalThis.sessionStorage = memoryStorage();
globalThis.window = { addEventListener() {}, removeEventListener() {}, location: { href: "http://x/" } };
const idb = fakeIndexedDB();
globalThis.indexedDB = idb;
globalThis.localStorage.setItem("auth.user", JSON.stringify({ name: "a" }));

const { get } = await import("svelte/store");
const ny = await import("../src/lib/netyield.js");
const { online } = await import("../src/lib/net.js");
const { cachedIds } = await import("../src/lib/stores.js");
const playcache = await import("../src/lib/playcache.js");
online.set(true);

// --- the browser, as far as these modules reach ------------------------------
const hdr = (o) => ({ get: (k) => o[String(k).toLowerCase()] ?? null });
const bytes = (n) => new Uint8Array(n).fill(0x42);
const chunks = [bytes(2048), bytes(2048)]; // 2 s of trickle in total, 4 KiB
let streamRequests = 0;

function streamBody(parts, { delay = 0 } = {}) {
  let i = 0;
  const reader = {
    async read() {
      if (i >= parts.length) return { done: true, value: undefined };
      if (delay) await new Promise((r) => setTimeout(r, delay));
      return { done: false, value: parts[i++] };
    },
    async cancel() {
      reader.cancelled = true;
    },
    cancelled: false,
  };
  return { getReader: () => reader, reader };
}

function ok(parts, headers = { "content-type": "audio/flac" }) {
  const body = streamBody(parts);
  return { ok: true, status: 200, headers: hdr(headers), body, bodyReader: body.reader };
}

function notArchived() {
  return {
    ok: false,
    status: 503,
    headers: hdr({ "retry-after": "2", "content-type": "application/json" }),
    body: streamBody([]),
  };
}

/** Everything that is not the stream answers an empty JSON body. */
function other() {
  return {
    ok: true,
    status: 200,
    headers: hdr({ "content-type": "application/json" }),
    body: null,
    json: async () => ({}),
  };
}

globalThis.fetch = async (url) => {
  if (String(url).includes("/api/stream/")) {
    streamRequests++;
    return ok(chunks);
  }
  return other();
};

class FakeEl extends EventTarget {
  constructor(src = "https://srv/api/stream/1") {
    super();
    this.src = src;
    this.currentSrc = src;
    this.paused = true;
    this.ended = false;
    this.currentTime = 0;
    this.duration = 40;
    this.ranges = [];
  }
  get buffered() {
    const r = this.ranges;
    return { length: r.length, start: (i) => r[i][0], end: (i) => r[i][1] };
  }
  fire(t) {
    this.dispatchEvent(new Event(t));
  }
}

/** An element playing off the network with nothing buffered: it wants the link. */
function playing() {
  ny.resetNetYield();
  const el = new FakeEl();
  ny.watchElement(el);
  el.paused = false;
  el.ranges = [[0, 2]];
  el.fire("play");
  assert.equal(ny.playerWantsLink(), true);
  return el;
}

let n = 0;
const track = (extra = {}) => ({
  id: `6c2f3d54-0000-4000-8000-${String(++n).padStart(12, "0")}`,
  deezer_id: String(9000 + n),
  title: "Song " + n,
  duration: 40,
  artist: { deezer_id: "a1", name: "Band" },
  album: { deezer_id: "al1", title: "Album", cover: "" },
  ...extra,
});

function stored(id) {
  return idb.dbs.get("nsupy-playcache")?.stores.get("audio").rows.get(id);
}

test("a prefetch read sleeps the trickle between chunks while the element wants the link", async () => {
  const el = playing();
  const t = track();
  const t0 = Date.now();
  await playcache.prefetchTrack(t, "FLAC");
  const elapsed = Date.now() - t0;
  // Two 2 KiB chunks at 4 KiB/s: 500 ms each. Anything near zero means the read
  // took the whole link, which is what made a skip slower than no prefetch.
  assert.ok(elapsed >= 800, `yielded for ${elapsed} ms, expected the trickle pace`);
  assert.equal(get(cachedIds).has(t.id), true);
  assert.equal(stored(t.id).size, 4096, "the whole body is written, in order");
  // ...and the moment the player has enough in hand, the rest is not slowed.
  el.ranges = [[0, 39]];
  el.fire("progress");
  assert.equal(ny.playerWantsLink(), false);
  const t2 = track();
  const t1 = Date.now();
  await playcache.prefetchTrack(t2, "FLAC");
  assert.ok(Date.now() - t1 < 200, `read at full speed (${Date.now() - t1} ms)`);
  ny.resetNetYield();
});

test("with nothing playing, the prefetch does not pause at all", async () => {
  ny.resetNetYield();
  const t = track();
  const t0 = Date.now();
  await playcache.prefetchTrack(t, "FLAC");
  assert.ok(Date.now() - t0 < 200, `took ${Date.now() - t0} ms`);
  assert.equal(stored(t.id).size, 4096);
});

test("a track that stops being wanted is abandoned, and nothing is written", async () => {
  ny.resetNetYield();
  const t = track();
  let asked = 0;
  // The first check (after the first chunk) still passes; the next one says a
  // skip happened, and the read must cancel rather than fill the cache.
  const p = playcache.prefetchTrack(t, "FLAC", () => ++asked <= 1);
  await p;
  assert.ok(asked >= 2, "the read asked again after a chunk");
  assert.equal(stored(t.id), undefined, "an abandoned read writes nothing");
  assert.equal(get(cachedIds).has(t.id), false);
});

test("a 503 « pas encore archivé » is retried after Retry-After, not given up on", async () => {
  ny.resetNetYield();
  const t = track();
  let attempts = 0;
  globalThis.fetch = async (url) => {
    if (!String(url).includes("/api/stream/")) return other();
    attempts++;
    return attempts === 1 ? notArchived() : ok(chunks);
  };
  const t0 = Date.now();
  await playcache.prefetchTrack(t, "FLAC");
  assert.equal(attempts, 2, "asked again once the archive had time to land");
  assert.ok(Date.now() - t0 >= 1900, "waited the Retry-After it was given");
  assert.equal(stored(t.id).size, 4096, "and the retry wrote the file");
});

test("a 503 that never clears stops after four attempts", async () => {
  ny.resetNetYield();
  const t = track();
  let attempts = 0;
  globalThis.fetch = async (url) => {
    if (!String(url).includes("/api/stream/")) return other();
    attempts++;
    return {
      ok: false,
      status: 503,
      headers: hdr({ "retry-after": "2" }),
      body: streamBody([]),
    };
  };
  await playcache.prefetchTrack(t, "FLAC");
  assert.equal(attempts, 4, "four attempts in total, then quiet");
  assert.equal(stored(t.id), undefined);
  globalThis.fetch = async (url) =>
    String(url).includes("/api/stream/") ? (streamRequests++, ok(chunks)) : other();
});
