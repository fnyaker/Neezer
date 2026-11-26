// The next track's preload (lib/playcache.js), on the two things that made it
// useless on a slow link: it took the whole link from the track being played,
// and it gave up for good on the server's first "not archived yet".
//
// A prefetch now reads through `offline.js#fetchYielding` with the prefetch's
// own rules: it never starts a request while a watched element wants the link,
// and mid-body it steps aside ENTIRELY — the request aborted, then resumed with
// `Range: bytes=<received>-` once the element lets go. It does NOT trickle: 4
// KB/s of a 16 KB/s link still cost the playing track 27 %. A 503 with
// Retry-After is the server saying "queued, come back", not a refusal: it is
// retried, at most four times, and only while the track is still wanted.
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

// 4 KiB, one byte per position, so a resumed file can be compared byte for byte.
const TOTAL = 4096;
const ALL = new Uint8Array(TOTAL);
for (let i = 0; i < TOTAL; i++) ALL[i] = i & 0xff;
const CHUNK = 1024;

let requests = [];

/** A resumable body of `ALL` from `offset` on, one chunk every 5 ms. */
function streamResponse(offset, signal, onChunk) {
  const rest = ALL.slice(offset);
  let i = 0;
  const reader = {
    async read() {
      if (i >= rest.length) return { done: true, value: undefined };
      await new Promise((r) => setTimeout(r, 5));
      if (signal && signal.aborted) {
        const e = new Error("aborted");
        e.name = "AbortError";
        throw e;
      }
      const value = rest.slice(i, i + CHUNK);
      i += CHUNK;
      if (onChunk) onChunk(offset + i);
      return { done: false, value };
    },
    async cancel() {
      reader.cancelled = true;
    },
    cancelled: false,
  };
  const headers = {
    "content-type": "audio/flac",
    "accept-ranges": "bytes",
    "content-length": String(rest.length),
  };
  if (offset) headers["content-range"] = `bytes ${offset}-${TOTAL - 1}/${TOTAL}`;
  return {
    ok: true,
    status: offset ? 206 : 200,
    headers: hdr(headers),
    body: { getReader: () => reader },
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

/** Install the stream: one resumable body, Range honoured, requests recorded. */
function serveStream({ onChunk } = {}) {
  requests = [];
  globalThis.fetch = async (url, init = {}) => {
    if (!String(url).includes("/api/stream/")) return other();
    const headers = init.headers || {};
    const m = /^bytes=(\d+)-$/.exec(headers.Range || headers.range || "");
    const offset = m ? +m[1] : 0;
    const rec = { url: String(url), headers: { ...headers }, signal: init.signal, offset, aborted: false };
    requests.push(rec);
    if (init.signal) init.signal.addEventListener("abort", () => (rec.aborted = true));
    return streamResponse(offset, init.signal, onChunk);
  };
}

function notArchived() {
  return {
    ok: false,
    status: 503,
    headers: hdr({ "retry-after": "2", "content-type": "application/json" }),
    body: { cancel: async () => {} },
  };
}

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

async function waitFor(cond, ms = 3000) {
  const t0 = Date.now();
  while (!cond()) {
    if (Date.now() - t0 > ms) throw new Error("timed out waiting");
    await new Promise((r) => setTimeout(r, 5));
  }
}

test("with nothing playing, the prefetch is a single request and is never aborted", async () => {
  ny.resetNetYield();
  serveStream();
  const t = track();
  await playcache.prefetchTrack(t, "FLAC");
  assert.equal(requests.length, 1, "one request, no resume");
  assert.equal(requests[0].aborted, false);
  assert.equal(stored(t.id).size, TOTAL, "the whole body is written");
  assert.deepEqual(new Uint8Array(await stored(t.id).blob.arrayBuffer()), ALL);
});

test("an element that wants the link mid-body aborts the request, then resumes it with Range", async () => {
  ny.resetNetYield();
  // The element starts with its file all in hand (it does not want the link),
  // then wants it again after the first chunk of the prefetch.
  const el = new FakeEl();
  ny.watchElement(el);
  el.paused = false;
  el.ranges = [[0, 39]];
  el.fire("play");
  assert.equal(ny.playerWantsLink(), false);
  let flipped = false;
  serveStream({
    onChunk: () => {
      if (flipped) return;
      flipped = true;
      el.ranges = [[0, 2]];
      el.fire("progress");
    },
  });
  const t = track();
  const p = playcache.prefetchTrack(t, "FLAC");
  await waitFor(() => requests.length === 1 && requests[0].aborted);
  assert.equal(ny.playerWantsLink(), true);
  assert.equal(requests.length, 1, "no new request while the element wants the link");
  assert.equal(stored(t.id), undefined, "nothing is written from a half read");
  // The element lets go: the read resumes at the byte it had, not from zero.
  el.ranges = [[0, 39]];
  el.fire("progress");
  await p;
  assert.equal(requests.length, 2);
  assert.equal(requests[1].headers.Range, `bytes=${CHUNK}-`, "resumed at the received byte");
  assert.equal(stored(t.id).size, TOTAL);
  assert.deepEqual(new Uint8Array(await stored(t.id).blob.arrayBuffer()), ALL, "identical to the source");
});

test("no request is started while the element wants the link; it starts when free", async () => {
  const el = playing();
  serveStream();
  const t = track();
  const p = playcache.prefetchTrack(t, "FLAC");
  await new Promise((r) => setTimeout(r, 100));
  assert.equal(requests.length, 0, "the prefetch waits for the element to let go");
  el.ranges = [[0, 39]];
  el.fire("progress");
  await p;
  assert.equal(requests.length, 1);
  assert.equal(stored(t.id).size, TOTAL);
  ny.resetNetYield();
});

test("a track that stops being wanted is abandoned, and nothing is written", async () => {
  ny.resetNetYield();
  serveStream();
  const t = track();
  let asked = 0;
  // The check before the request and the one before the first read pass; the
  // one after the first chunk says a skip happened.
  await playcache.prefetchTrack(t, "FLAC", () => ++asked <= 2);
  assert.ok(asked >= 3, "the read asked again after a chunk");
  assert.equal(requests[0].aborted, true, "the request is aborted, not left running");
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
    return attempts === 1 ? notArchived() : streamResponse(0, null, null);
  };
  const t0 = Date.now();
  await playcache.prefetchTrack(t, "FLAC");
  assert.equal(attempts, 2, "asked again once the archive had time to land");
  assert.ok(Date.now() - t0 >= 1900, "waited the Retry-After it was given");
  assert.equal(stored(t.id).size, TOTAL, "and the retry wrote the file");
});

test("a 503 that never clears stops after four attempts", async () => {
  ny.resetNetYield();
  const t = track();
  let attempts = 0;
  globalThis.fetch = async (url) => {
    if (!String(url).includes("/api/stream/")) return other();
    attempts++;
    return notArchived();
  };
  await playcache.prefetchTrack(t, "FLAC");
  assert.equal(attempts, 4, "four attempts in total, then quiet");
  assert.equal(stored(t.id), undefined);
  serveStream();
});

// --- the HEAD (lot 5): the track's first seconds, as one finished file ------
//
// The head is what makes a tap sound at once on a link that could never deliver
// the track, and it is deliberately NOT the track: six seconds, stored under its
// own key, never handed out by `getCachedAudioURL` / `isCached`. The player
// starts on it and hands over to the full stream before it runs out.

const HEAD_BYTES = 48 * 1024;
const headBody = new Uint8Array(HEAD_BYTES);
for (let i = 0; i < HEAD_BYTES; i++) headBody[i] = (i * 7) & 0xff;
// An Ogg page magic: the route answers an Ogg file, and the prefetch refuses
// anything that is not one (a misrouted answer must never be stored as a head).
headBody.set([0x4f, 0x67, 0x67, 0x53], 0);

/** The /head answer: a FINISHED file — a length and byte ranges, like the route. */
function serveHead({ onChunk } = {}) {
  const reqs = [];
  globalThis.fetch = async (url, init = {}) => {
    if (!String(url).endsWith("/head")) return other();
    const headers = init.headers || {};
    const m = /^bytes=(\d+)-$/.exec(headers.Range || headers.range || "");
    const offset = m ? +m[1] : 0;
    const rec = { url: String(url), headers: { ...headers }, signal: init.signal, offset, aborted: false };
    reqs.push(rec);
    if (init.signal) init.signal.addEventListener("abort", () => (rec.aborted = true));
    const rest = headBody.slice(offset);
    let i = 0;
    const reader = {
      async read() {
        if (i >= rest.length) return { done: true, value: undefined };
        await new Promise((r) => setTimeout(r, 5));
        if (init.signal && init.signal.aborted) {
          const e = new Error("aborted");
          e.name = "AbortError";
          throw e;
        }
        const value = rest.slice(i, i + 1024);
        i += 1024;
        if (onChunk) onChunk(offset + i);
        return { done: false, value };
      },
      async cancel() {},
    };
    const h = {
      "content-type": "audio/ogg",
      "accept-ranges": "bytes",
      "content-length": String(rest.length),
    };
    if (offset) h["content-range"] = `bytes ${offset}-${HEAD_BYTES - 1}/${HEAD_BYTES}`;
    return {
      ok: true,
      status: offset ? 206 : 200,
      headers: hdr(h),
      body: { getReader: () => reader },
    };
  };
  return () => reqs;
}

test("the head is stored apart from the track, and never stands in for it", async () => {
  ny.resetNetYield();
  const reqs = serveHead();
  const t = track();
  await playcache.prefetchHead(t, "FLAC");
  assert.equal(reqs().length, 1);
  assert.ok(reqs()[0].url.endsWith(`/api/stream/${t.id}/head`), "the head route, named by the row id");
  assert.equal(reqs()[0].headers["X-NS-Background"], "1", "a preload says so to the server");
  // Written under its own key, marked as a head, and NOT the track.
  const row = stored("head:" + t.id);
  assert.ok(row, "the head is on the device");
  assert.equal(row.size, HEAD_BYTES);
  assert.equal(row.head, true);
  assert.match(row.quality, /^HEAD:/);
  assert.equal(get(cachedIds).has(t.id), false, "the track itself is NOT cached");
  assert.equal(await playcache.getCachedAudioURL(t.id, "FLAC"), null, "never served as the whole track");
  const url = await playcache.getCachedHeadURL(t.id);
  assert.ok(url && url.startsWith("blob:"), "the head is handed out as a blob URL");
  URL.revokeObjectURL(url);
  // A track that already has its head is not asked for again.
  await playcache.prefetchHead(t, "FLAC");
  assert.equal(reqs().length, 1);
});

test("the head steps aside by abort + Range, like the full preload", async () => {
  ny.resetNetYield();
  const el = new FakeEl();
  ny.watchElement(el);
  el.paused = false;
  el.ranges = [[0, 39]];
  el.fire("play");
  assert.equal(ny.playerWantsLink(), false);
  let flipped = false;
  const reqs = serveHead({
    onChunk: () => {
      if (flipped) return;
      flipped = true;
      el.ranges = [[0, 2]];
      el.fire("progress");
    },
  });
  const t = track();
  const p = playcache.prefetchHead(t, "FLAC");
  await waitFor(() => reqs().length === 1 && reqs()[0].aborted);
  assert.equal(stored("head:" + t.id), undefined, "a half-read head is not written");
  el.ranges = [[0, 39]];
  el.fire("progress");
  await p;
  assert.equal(reqs().length, 2);
  assert.equal(reqs()[1].headers.Range, "bytes=1024-", "resumed at the received byte");
  assert.equal(stored("head:" + t.id).size, HEAD_BYTES);
  ny.resetNetYield();
});

test("no head is asked for when the whole track is already on the device", async () => {
  ny.resetNetYield();
  const reqs = serveHead();
  const t = track();
  cachedIds.update((s) => new Set(s).add(t.id));
  await playcache.prefetchHead(t, "FLAC");
  assert.equal(reqs().length, 0, "the whole file is better than its head");
  cachedIds.update((s) => {
    const n = new Set(s);
    n.delete(t.id);
    return n;
  });
});

test("a body that is not an Ogg stream is not stored as a head", async () => {
  ny.resetNetYield();
  const t = track();
  globalThis.fetch = async (url) => {
    if (!String(url).endsWith("/head")) return other();
    return {
      ok: true,
      status: 200,
      headers: hdr({ "content-type": "audio/wav", "content-length": "16" }),
      body: {
        getReader: () => {
          let sent = false;
          return {
            async read() {
              if (sent) return { done: true, value: undefined };
              sent = true;
              return { done: false, value: new Uint8Array(16).fill(7) };
            },
            async cancel() {},
          };
        },
      },
    };
  };
  await playcache.prefetchHead(t, "FLAC");
  assert.equal(stored("head:" + t.id), undefined, "a WAV is not a head");
  assert.equal(await playcache.getCachedHeadURL(t.id), null);
});

test("a cold track's head is given up on quietly — the full preload queues it", async () => {
  ny.resetNetYield();
  const t = track();
  let attempts = 0;
  globalThis.fetch = async (url) => {
    if (!String(url).endsWith("/head")) return other();
    attempts++;
    return notArchived();
  };
  await playcache.prefetchHead(t, "FLAC");
  assert.equal(attempts, 1, "one look: it retries nothing, the full preload comes next");
  assert.equal(stored("head:" + t.id), undefined);
});

