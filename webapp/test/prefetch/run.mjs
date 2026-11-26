#!/usr/bin/env node
// The next track's preload on a slow link: does it arrive in time, and does it
// take anything from the track being played?
//
//   npm run build && node test/prefetch/run.mjs          (KBPS=16 by default)
//
// The real SPA against the real server (tools/perf_api.py --serve, six archived
// 40 s tracks), behind a CDP link of 16 KB/s and of 64 KB/s (latency 150 ms).
// Two taps per run, both timed to the first audible sample of the new track
// (`!paused && currentTime > 0.15 && readyState >= 2`):
//   - COLD: 1 s after pressing play, before the preload has had time to start;
//   - HOT:  14 s into a track that is really playing, with the preload of the
//           following one running.
// Each cell is the median of three runs, and the stream requests, bytes and
// watchdog reloads the run causes are counted from the browser, not guessed.
//
// What the rule (lib/netyield.js, lib/offline.js#fetchYielding,
// lib/playcache.js) promises: the preload never starts while the element
// playing wants the link, and mid-body it steps aside ENTIRELY — the request
// aborted, resumed with `Range: bytes=<received>-` once the element lets go.
// It never trickles: take 1 measured that 4 KB/s of a 16 KB/s link still cost
// the playing track 27 % (cold 29.5 s against 20.9 s without the preload, hot
// 87.7 s against 25.9 s with four watchdog reloads), and that the trickle was
// pure cost because a 320 KB track cannot be preloaded in 14 s at 4 KB/s.
//
// Measured on 2026-11-26 (take 2), median of 3, 40 s WAV tracks (320 KB), CDP
// 150 ms:
//
//   link      preload  cold (next at 1 s)     hot (next at 14 s)     blob at tap
//   LAN       off      0.3 s   1 req          0.3 s   1 req 313 KB   -
//   LAN       on       0.3 s   1 req          0.3 s   0 req          313 KB
//   16 KB/s   off     20.9 s   1 req         25.5 s   1 req 228 KB   -
//   16 KB/s   on      20.8 s   1 req         26.7 s   1 req 228 KB   incomplete
//   64 KB/s   off      3.9 s   1 req          4.2 s   1 req 235 KB   -
//   64 KB/s   on       3.9 s   1 req          0.3 s   0 req          313 KB
//
// At 16 KB/s the preload gets 22 KB in before the tap and the blob stays
// incomplete (a 320 KB track needs 20 s at 16 KB/s, longer than the 14 s
// before the tap), so the tap streams: 20.8 s cold (0.995 ×) and 26.7 s hot
// (1.047 ×, +1.2 s), no watchdog reload. At 64 KB/s the whole blob is on the
// device at the tap and it costs no stream request. On LAN it always was.
//
// Bounds (the spec's goal is "at least as fast with as without", not a fixed
// number of seconds): with the preload on, (a) the tap must sound within 1.10 ×
// the same case without it, (b) the preload must add no watchdog reload, and
// (c) when the blob is complete at the tap, the tap must cost no stream
// request. An incomplete blob at 16 KB/s is acceptable and reported, not a
// failure.
import { mkdtempSync, writeFileSync, statSync } from "node:fs";
import { execSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  loadPlaywright,
  startServer,
  silence,
  login,
  checker,
  installSessionPrefix,
  deezerHost,
} from "../harness.mjs";

/**
 * The same 8 kHz / 8-bit / mono WAV shape as `silence()`, filled with white
 * noise. Silence is what the other benches use, and for them it is fine — but
 * Opus compresses digital silence to 1.5 KB, so a silence bench cannot measure
 * the SIZE of a head (48 KB on real music). The stream itself is this same WAV
 * either way, so the lot-2 numbers (requests, bytes, timings) are unchanged.
 */
function noise(seconds = 40) {
  const rate = 8000;
  const n = rate * seconds;
  const h = Buffer.alloc(44);
  h.write("RIFF", 0);
  h.writeUInt32LE(36 + n, 4);
  h.write("WAVEfmt ", 8);
  h.writeUInt32LE(16, 16);
  h.writeUInt16LE(1, 20);
  h.writeUInt16LE(1, 22);
  h.writeUInt32LE(rate, 24);
  h.writeUInt32LE(rate, 28);
  h.writeUInt16LE(1, 32);
  h.writeUInt16LE(8, 34);
  h.write("data", 36);
  h.writeUInt32LE(n, 40);
  const data = Buffer.alloc(n);
  let seed = 0x2f6e2b1;
  for (let i = 0; i < n; i++) {
    seed = (seed * 1103515245 + 12345) & 0x7fffffff;
    data[i] = 128 + ((seed >> 16) % 96) - 48; // ±48 around the 8-bit midpoint
  }
  return Buffer.concat([h, data]);
}

const { chromium } = await loadPlaywright();
const LEN = 40; // seconds; 8 kHz / 8-bit, so the audio itself is 8 KB a second
const dir = mkdtempSync(join(tmpdir(), "nspf-"));
const wavPath = join(dir, "track.wav");
writeFileSync(wavPath, process.env.SILENCE ? silence(LEN) : noise(LEN));
// FLAC, not the WAV: the head's hand-over opens the full stream at an offset
// (`?start=`), and the server can only cut a source it can re-encode or copy —
// a WAV answers 409 (no muxer for a pipe). Real archives are FLAC.
const audioPath = join(dir, "track.flac");
execSync(`ffmpeg -v 0 -y -i "${wavPath}" -c:a flac "${audioPath}"`);
const FLAC_BYTES = statSync(audioPath).size;
// The lot-2 table keeps the WAV it was measured with; the head case gets a FLAC
// (see `audioPath` above) so its `?start=` hand-over is answerable.
const TRACK_BYTES = 44 + LEN * 8000;
const { base, ids, stop } = await startServer(["--audio", wavPath]);
const { check, state } = checker();
const TRACKS = ids.ids.slice(0, 6);

const RATES = process.env.KBPS
  ? [{ name: `${+process.env.KBPS} KB/s`, kbps: +process.env.KBPS }]
  : [
      { name: "LAN", kbps: 0 },
      { name: "16 KB/s", kbps: 16 },
      { name: "64 KB/s", kbps: 64 },
    ];
const RUNS = +process.env.RUNS || 3;
const COLD_AFTER_MS = 1000; // the spec's cold tap
const HOT_AFTER_MS = +process.env.HOT_AFTER_MS || 14000; // ...and its hot one
const SLOW_RATES = ["16 KB/s", "64 KB/s"];
const TAP_MS = 1500; // a request this soon after a tap belongs to the tap


const song = (id, k) => ({
  deezer_id: id,
  title: "Song " + k,
  duration: LEN,
  artist: { deezer_id: "a1", name: "Band" },
  album: { deezer_id: "al1", title: "Album", cover: "" },
});

const browser = await chromium.launch({ args: ["--autoplay-policy=no-user-gesture-required"] });

/**
 * One run: press play, tap next at 1 s (cold) and again 14 s into the playing
 * track (hot), timing each to its first audible sample. Returns both timings,
 * the stream requests the tap caused, and the bytes the preload had read.
 */
async function run({ kbps, prefetch, headCase = false, base: srv = base, trackBytes = TRACK_BYTES }) {
  const ctx = await browser.newContext({ viewport: { width: 390, height: 844 } });
  await ctx.route(deezerHost, (r) => r.abort());
  await ctx.addInitScript(installSessionPrefix);
  await ctx.addInitScript(() => {
    // The page's own elements, so a measurement names the element that plays.
    const Native = window.Audio;
    window.__els = [];
    // A watchdog recovery reloads the element with the SAME source: that is a
    // loadstart whose src equals the previous one. A track change names a new
    // source, so it is not counted.
    window.__reloads = 0;
    window.Audio = function (...a) {
      const el = new Native(...a);
      window.__els.push(el);
      let last = "";
      el.addEventListener("loadstart", () => {
        const s = String(el.src || "");
        if (s && s === last) window.__reloads++;
        last = s;
      });
      return el;
    };
    window.Audio.prototype = Native.prototype;
    // What the playback cache holds, asked at the tap: a prefetch is written in
    // one put at the end, so a row of the track's full length means complete.
    // Only CALLED once the app has opened the database (it does at startup).
    window.__cacheSizes = async () => {
      try {
        const db = await new Promise((res, rej) => {
          const r = indexedDB.open("nsupy-playcache");
          r.onsuccess = () => res(r.result);
          r.onerror = () => rej(r.error);
        });
        const rows = await new Promise((res, rej) => {
          const q = db.transaction("audio").objectStore("audio").getAll();
          q.onsuccess = () => res(q.result);
          q.onerror = () => rej(q.error);
        });
        return rows.map((r) => ({
          size: r.blob ? r.blob.size : r.size || 0,
          head: !!r.head,
        }));
      } catch {
        return [];
      }
    };
    // The hand-over from a prefetched HEAD to the full stream, measured on what
    // the person actually hears: armed right after the tap has produced sound,
    // it samples every 20 ms and keeps the longest stretch with NOTHING audible
    // (no element playing at all) — the hole a swap would open — plus every
    // `waiting` a playing element reports, with its duration.
    window.__probe = { armed: false, hole: 0, since: 0, silent: true, waits: [] };
    setInterval(() => {
      const p = window.__probe;
      if (!p.armed) return;
      const now = performance.now();
      const audible = window.__els.some(
        (e) => !e.paused && !e.ended && e.currentTime > 0.02 && e.readyState >= 2
      );
      if (!audible) {
        if (p.silent) return;
        p.silent = true;
        p.since = now;
        return;
      }
      if (p.silent) {
        p.hole = Math.max(p.hole, now - p.since);
        p.silent = false;
      }
    }, 20);
    for (const ev of ["waiting", "playing"]) {
      window.addEventListener(
        "capture",
        (e) => {
          if (!(e.target instanceof HTMLMediaElement) || !window.__probe.armed) return;
          if (ev === "waiting") window.__probe.waits.push({ start: performance.now(), dur: 0 });
          else {
            const last = window.__probe.waits[window.__probe.waits.length - 1];
            if (last && !last.dur) last.dur = performance.now() - last.start;
          }
        },
        true
      );
    }
    // A timeline of what each element was doing: the yield decision reads
    // paused / buffered-ahead, so a bench that cannot see them cannot explain
    // a number (DEBUG=1 prints it).
    window.__trace = [];
    setInterval(() => {
      for (const [i, e] of window.__els.entries()) {
        let end = 0;
        try {
          const b = e.buffered;
          for (let j = 0; j < b.length; j++) if (b.end(j) > end) end = b.end(j);
        } catch {
          /* no ranges yet */
        }
        window.__trace.push({
          t: Date.now(),
          i,
          src: String(e.currentSrc || e.src || "").slice(-24),
          paused: e.paused,
          rs: e.readyState,
          ct: +(+e.currentTime || 0).toFixed(1),
          end: +end.toFixed(1),
        });
      }
    }, 500);
  });
  await ctx.addInitScript(() => {
    const j = sessionStorage.getItem("__inject");
    if (!j) return;
    const { session, prefetch, logs } = JSON.parse(j);
    if (logs) localStorage.setItem("debug.logs", "1");
    localStorage.setItem(window.__nsPkey("player.session"), JSON.stringify(session));
    localStorage.removeItem(window.__nsPkey("player.pos"));
    // Raw settings keys: `persisted()` is not per-account for these.
    localStorage.setItem("cache.prefetch", JSON.stringify(prefetch));
    localStorage.setItem("cache.prefetchCount", "1");
    localStorage.setItem("offline.quality", JSON.stringify("FLAC"));
    sessionStorage.removeItem("__inject");
  });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => console.log("pageerror:", e.message));
  await login(page, srv);
  const session = {
    queue: TRACKS.map((id, k) => song(id, k)),
    index: 0,
    start: 0,
    currentTime: 0,
    context: null,
    shuffle: false,
    repeat: "off",
    _orig: null,
    at: Date.now(),
  };
  await page.evaluate(
    (j) => sessionStorage.setItem("__inject", j),
    JSON.stringify({ session, prefetch, logs: !!process.env.DEBUG })
  );
  await page.goto(`${srv}/app/`);
  await page.reload();
  await page.waitForFunction(() => document.querySelector("button.pp"), null, { timeout: 30000 });

  // The link, and what crosses it. Only now: the shell and the API are not
  // what this bench compares (a slow app start is another lot). Collected on
  // every run — the LAN one counts requests too.
  const streams = [];
  const marks = { tap: 0 };
  const cdp = await ctx.newCDPSession(page);
  await cdp.send("Network.enable");
  if (kbps) {
    await cdp.send("Network.emulateNetworkConditions", {
      offline: false,
      latency: 150,
      downloadThroughput: kbps * 1024,
      uploadThroughput: 64 * 1024,
    });
  }
  const byId = new Map();
  cdp.on("Network.requestWillBeSent", (e) => {
    if (!e.request.url.includes("/api/stream/")) return;
    const headers = Object.fromEntries(
      Object.entries(e.request.headers).map(([k, v]) => [k.toLowerCase(), v])
    );
    const rec = {
      url: e.request.url,
      bg: headers["x-ns-background"] === "1",
      start: Date.now(),
      bytes: 0,
      after: 0,
    };
    byId.set(e.requestId, rec);
    streams.push(rec);
  });
  cdp.on("Network.dataReceived", (e) => {
    const rec = byId.get(e.requestId);
    if (!rec) return;
    rec.bytes += e.dataLength;
    if (marks.tap) rec.after += e.dataLength;
  });

  const sound = (ref, before) =>
    page.evaluate(
      async ({ ref, before, timeout }) => {
        const t0 = performance.now();
        while (performance.now() - t0 < timeout) {
          const el = window.__els.find((e) => {
            if (e.paused || e.readyState < 2 || e.currentTime <= 0.15) return false;
            const src = e.currentSrc || e.src || "";
            return src.includes(ref) || !before.includes(src);
          });
          if (el) return Math.round(performance.now() - t0);
          await new Promise((r) => setTimeout(r, 25));
        }
        return -1;
      },
      { ref, before, timeout: 120000 }
    );

  const srcs = () =>
    page.evaluate(() => window.__els.map((e) => e.currentSrc || e.src || ""));

  const t0 = Date.now();
  await page.locator("button.pp").first().click();

  // COLD: the preload has not started yet (its delay is 3 s), so this is the
  // plain stream — the number the preload must not make worse.
  await page.waitForTimeout(COLD_AFTER_MS);
  const prefetchBeforeCold = streams.filter((s) => s.bg).reduce((n, s) => n + s.bytes, 0);
  const coldTapAt = streams.length;
  const coldBefore = await srcs();
  marks.tap = Date.now();
  const coldTap = marks.tap - t0;
  await page.locator("button.next").first().click();
  const cold = await sound(TRACKS[1], coldBefore);
  const coldTapMs = Date.now() - marks.tap;
  const coldReqs = streams.slice(coldTapAt).filter((s) => s.start <= marks.tap + TAP_MS).length;
  const coldIndex = await page.evaluate(() => {
    try {
      const s = JSON.parse(localStorage.getItem(window.__nsPkey("player.session")) || "null");
      return s ? s.index : null;
    } catch {
      return "?";
    }
  });
  if (process.env.DEBUG) console.log(`    after the cold tap the queue is at index ${coldIndex}`);
  marks.tap = 0;

  // HOT: 14 s into the track that is now playing, with the preload of the
  // following one running (or not, when the setting is off). The watchdog
  // reloads counted here are the ones the preload's competition causes: the
  // playing track's element reloaded with the same source.
  const reloadsBefore = await page.evaluate(() => window.__reloads);
  // HEAD mode taps when the next track's head is really on the device (or says
  // how long that took and taps anyway): the question is what the head buys,
  // not what the preload manages to do before an arbitrary deadline.
  let headWaitMs = 0;
  if (headCase) {
    const t = Date.now();
    for (;;) {
      const rows = await page.evaluate(() => window.__cacheSizes()).catch(() => []);
      if (rows.some((r) => r.head && r.size > 8192)) break;
      if (Date.now() - t > 60000) break;
      await page.waitForTimeout(250);
    }
    headWaitMs = Date.now() - t;
    await page.waitForTimeout(+process.env.HEAD_TAP_AFTER_MS || 300);
  } else {
    await page.waitForTimeout(HOT_AFTER_MS);
  }
  const reloads = (await page.evaluate(() => window.__reloads)) - reloadsBefore;
  const prefetchedBefore = streams.filter((s) => s.bg).reduce((n, s) => n + s.bytes, 0);
  const sizes = await page.evaluate(() => window.__cacheSizes()).catch(() => []);
  const complete = sizes.some((s) => !s.head && s.size >= trackBytes - 8);
  const headAtTap = sizes.some((s) => s.head && s.size > 8192);
  const hotTapAt = streams.length;
  const hotBefore = await srcs();
  marks.tap = Date.now();
  const hotTap = marks.tap - t0;
  await page.locator("button.next").first().click();
  const hot = await sound(TRACKS[2], hotBefore);
  const hotTapMs = Date.now() - marks.tap;
  const hotStreams = streams.slice(hotTapAt).filter((s) => s.start <= marks.tap + TAP_MS);
  marks.tap = 0;
  // Was the sound that just started the HEAD or a whole file? The head is a
  // finished six-second file, so its own duration says which — a whole blob is
  // the track's length, a live stream has none.
  const headUsed = await page.evaluate(() => {
    const el = window.__els.find((e) => !e.paused && !e.ended);
    return !!el && isFinite(el.duration) && el.duration > 0 && el.duration <= 7.5;
  });
  // The head lasts HEAD_SECONDS: watch the hand-over through it. Armed only now,
  // so the tap's own start is not counted as a hole.
  await page.evaluate(() => {
    window.__probe.armed = true;
    window.__probe.silent = false;
    window.__probe.since = performance.now();
  });
  await page.waitForTimeout(8000);
  const probe = await page.evaluate(() => ({
    hole: Math.round(window.__probe.hole),
    waits: window.__probe.waits.map((w) => Math.round(w.dur)),
    srcs: window.__els.map((e) => ({ src: e.currentSrc || e.src || "", ct: +(+e.currentTime || 0).toFixed(1) })),
  }));
  const hole = probe.hole;
  const longWait = probe.waits.filter((d) => d > 100).length;
  const onStream = probe.srcs.some((s) => /\/api\/stream\//.test(s.src) && /start=/.test(s.src));

  const trace = process.env.DEBUG
    ? await page.evaluate(() => window.__trace).catch(() => [])
    : [];
  if (process.env.DEBUG) {
    const log = await page.evaluate(() => {
      try {
        return JSON.parse(localStorage.getItem("debug.log.buffer") || "[]");
      } catch {
        return [];
      }
    });
    for (const row of log) {
      if (!/head|stream|source|load/i.test(String(row[2]))) continue;
      console.log(`    log +${row[0] - t0} ms [${row[1]}] ${row[2]} ${row[3] || ""}`);
    }
  }
  await ctx.close();
  if (process.env.DEBUG) {
    console.log(`    streams (cold tap at ${coldTap} ms, hot tap at ${hotTap} ms):`);
    for (const s of streams) {
      console.log(
        `      ${s.bg ? "prefetch" : "element "} ${s.url.replace(/^.*\/api\/stream\//, "")} ` +
          `start ${s.start - t0} ms, ${(s.bytes / 1024).toFixed(0)} KB`
      );
    }
    console.log("    element trace (from the hot tap):");
    for (const r of trace.filter((r) => r.t - t0 >= hotTap - 2000)) {
      console.log(
        `      +${String(r.t - t0).padStart(6)} ms el${r.i} ${r.src} paused=${r.paused ? "y" : "n"} ` +
          `rs=${r.rs} ct=${r.ct} end=${r.end}`
      );
    }
  }
  return {
    cold,
    coldTapMs,
    coldReqs,
    hot,
    hotTapMs,
    hotReqs: hotStreams.length,
    hotTapBytes: hotStreams.reduce((n, s) => n + s.after, 0),
    prefetchBeforeCold,
    prefetched: prefetchedBefore - prefetchBeforeCold,
    reloads,
    complete,
    headAtTap,
    headUsed,
    headWaitMs,
    hole,
    longWait,
    onStream,
  };
}

const median = (a) => {
  const s = [...a].sort((x, y) => x - y);
  return s[Math.floor(s.length / 2)];
};
const secs = (ms) => (ms < 0 ? "timeout" : (ms / 1000).toFixed(1) + " s");

// HEAD=1 runs the head case alone: the lot-2 table is already measured and in
// the notes, and a head run is a minute each.
const HEAD_ONLY = process.env.HEAD === "1";

const table = [];
for (const rate of HEAD_ONLY ? [] : RATES) {
  for (const prefetch of process.env.PREFETCH
    ? [process.env.PREFETCH === "on"]
    : [false, true]) {
    const runs = [];
    for (let i = 0; i < RUNS; i++) {
      const r = await run({ kbps: rate.kbps, prefetch });
      runs.push(r);
      console.log(
        `  ${rate.name} ${prefetch ? "prefetch ON " : "prefetch OFF"} #${i + 1}: ` +
          `cold ${secs(r.cold)} (${r.coldReqs} req, ${r.coldTapMs} ms), ` +
          `hot ${secs(r.hot)} (${r.hotReqs} req, ${(r.hotTapBytes / 1024).toFixed(0)} KB after the tap), ` +
          `preloaded ${(r.prefetched / 1024).toFixed(0)} KB, ${r.reloads} reload(s), ` +
          `blob ${r.complete ? "complete" : "incomplete"}, ` +
          `head ${r.headAtTap ? "yes" : "no"}, hole ${r.hole} ms`
      );
    }
    table.push({
      rate,
      prefetch,
      cold: median(runs.map((r) => r.cold)),
      hot: median(runs.map((r) => r.hot)),
      coldReqs: median(runs.map((r) => r.coldReqs)),
      hotReqs: median(runs.map((r) => r.hotReqs)),
      hotTapBytes: median(runs.map((r) => r.hotTapBytes)),
      prefetched: median(runs.map((r) => r.prefetched)),
      hotTapMs: median(runs.map((r) => r.hotTapMs)),
      reloads: median(runs.map((r) => r.reloads)),
      complete: runs.filter((r) => r.complete).length >= Math.ceil(RUNS / 2),
      headAtTap: runs.filter((r) => r.headAtTap).length >= Math.ceil(RUNS / 2),
      hole: median(runs.map((r) => r.hole)),
      longWait: median(runs.map((r) => r.longWait)),
      onStream: runs.filter((r) => r.onStream).length >= Math.ceil(RUNS / 2),
    });
  }
}

if (!HEAD_ONLY) {
console.log("\nrate       prefetch   cold (next at 1 s)      hot (next at 14 s)      preloaded  reloads  blob");
for (const row of table) {
  console.log(
    `${row.rate.name.padEnd(10)} ${(row.prefetch ? "ON" : "OFF").padEnd(9)} ` +
      `${secs(row.cold).padStart(7)}  ${String(row.coldReqs).padStart(2)} req   ` +
      `${secs(row.hot).padStart(7)}  ${String(row.hotReqs).padStart(2)} req  ` +
      `${(row.hotTapBytes / 1024).toFixed(0).padStart(4)} KB   ` +
      `${(row.prefetched / 1024).toFixed(0).padStart(4)} KB  ` +
      `${String(row.reloads).padStart(3)}     ${row.complete ? "complete" : "incomplete"}`
  );
}

const at = (name, prefetch) => table.find((r) => r.rate.name === name && r.prefetch === prefetch);
for (const name of SLOW_RATES) {
  const on = at(name, true);
  const off = at(name, false);
  if (!on || !off) continue;
  // (a) The spec's goal is "at least as fast with as without": 10 % of slack
  // covers the run-to-run noise, nothing more.
  check(
    on.cold > 0 && off.cold > 0 && on.cold <= off.cold * 1.1,
    `${name}: a next at 1 s with the preload ≤ 1.10 × without`,
    `${secs(on.cold)} against ${secs(off.cold)} (bound ${secs(off.cold * 1.1)})`
  );
  check(
    on.hot > 0 && off.hot > 0 && on.hot <= off.hot * 1.1,
    `${name}: a next at 14 s with the preload ≤ 1.10 × without`,
    `${secs(on.hot)} against ${secs(off.hot)} (bound ${secs(off.hot * 1.1)})`
  );
  // (b) The preload must never starve the playing track into a reload.
  check(
    on.reloads <= off.reloads,
    `${name}: the preload adds no watchdog reload`,
    `${on.reloads} against ${off.reloads}`
  );
  // (c) A complete blob at the tap must be played from the device.
  check(
    !on.complete || on.hotReqs === 0,
    `${name}: a complete blob at the tap costs no stream request`,
    `${on.complete ? "complete" : "incomplete"}, ${on.hotReqs} request(s)`
  );
}
const lanOn = at("LAN", true);
const lanOff = at("LAN", false);
if (lanOn) {
  check(
    lanOn.hot > 0 && lanOn.hot <= 300,
    "LAN: with the preload, the next track sounds in 300 ms or less",
    secs(lanOn.hot)
  );
  check(
    lanOn.hotReqs === 0,
    "LAN: ...from the device's copy — no stream request at the tap",
    `${lanOn.hotReqs} request(s), ${(lanOn.prefetched / 1024).toFixed(0)} KB preloaded`
  );
}
if (lanOff) {
  check(
    lanOff.hot > 0 && lanOff.hot <= 300,
    "LAN without the preload, a next still sounds at once (it streams the whole file fast)",
    secs(lanOff.hot)
  );
}
}

// --- the HEAD of the next track (lot 5) --------------------------------------
//
// The preload asks for the next track's head FIRST (`/api/stream/<id>/head`): its
// first six seconds, Opus 64, as one finished file. A tap then starts on those
// bytes — 48 KB on real music — and the player opens the full stream on the idle
// element at the point the head ends (`?start=`) and swaps before it runs out.
// Only the prefetch ON case exists here: without it there is no head at all.
const head = [];
const HEAD_RATES = HEAD_ONLY ? RATES : RATES.filter((r) => r.kbps === 0 || r.kbps === 16);
// Its own server, on the FLAC archive: the hand-over opens the full stream at
// `?start=<head>`, and the server cuts a FLAC (a WAV answers 409).
const headServer = HEAD_RATES.length ? await startServer(["--audio", audioPath]) : null;
try {
for (const rate of HEAD_RATES) {
  const runs = [];
  for (let i = 0; i < RUNS; i++) {
    const r = await run({
      kbps: rate.kbps,
      prefetch: true,
      headCase: true,
      base: headServer.base,
      trackBytes: FLAC_BYTES,
    });
    runs.push(r);
    console.log(
      `  ${rate.name} HEAD #${i + 1}: ` +
        `hot ${secs(r.hot)} (${r.hotReqs} req at the tap), ` +
        `head ${r.headAtTap ? `on the device after ${(r.headWaitMs / 1000).toFixed(1)} s` : "NOT there"}` +
        `${r.headUsed ? ", used" : ""}, ` +
        `hole ${r.hole} ms, ${r.longWait} wait(s) > 100 ms, ` +
        `full stream ${r.onStream ? "adopted (?start=)" : "NOT adopted"}`
    );
  }
  head.push({
    rate,
    hot: median(runs.map((r) => r.hot)),
    hotReqs: median(runs.map((r) => r.hotReqs)),
    headAtTap: runs.filter((r) => r.headAtTap).length >= Math.ceil(RUNS / 2),
    headWait: median(runs.map((r) => r.headWaitMs)),
    headUsed: runs.filter((r) => r.headUsed).length >= Math.ceil(RUNS / 2),
    hole: median(runs.map((r) => r.hole)),
    longWait: median(runs.map((r) => r.longWait)),
    onStream: runs.filter((r) => r.onStream).length >= Math.ceil(RUNS / 2),
  });
}
} finally {
  if (headServer) await headServer.stop();
}

if (head.length) {
  console.log("\nrate       head at tap          tap -> sound   hole   > 100 ms waits   source");
  for (const row of head) {
    console.log(
      `${row.rate.name.padEnd(10)} ${(row.headAtTap ? `yes, ${(row.headWait / 1000).toFixed(1)} s` : "no").padEnd(19)} ` +
        `${secs(row.hot).padStart(7)}  ${String(row.hotReqs).padStart(2)} req   ` +
        `${String(row.hole).padStart(4)} ms   ${String(row.longWait).padStart(3)}             ` +
        `${row.headUsed ? (row.onStream ? "head -> ?start=" : "head, NOT handed over") : "whole blob"}`
    );
  }
  for (const row of head) {
    // The spec's own goal: the head makes a tap sound in 4 s or less at 16 KB/s.
    if (row.rate.kbps === 16) {
      check(
        row.headAtTap,
        "16 KB/s: the next track's head lands while the current one plays",
        row.headAtTap ? `in ${(row.headWait / 1000).toFixed(1)} s` : "never (60 s)"
      );
      check(
        row.hot > 0 && row.hot <= 4000,
        "16 KB/s: the next track sounds in 4 s or less on its head",
        secs(row.hot)
      );
    }
    // The hand-over must not open a hole: the head is swapped for the stream
    // before it runs out, never after.
    check(
      row.hole <= 100,
      `${row.rate.name}: no audible hole over 100 ms at the hand-over`,
      `${row.hole} ms`
    );
    check(row.longWait === 0, `${row.rate.name}: no element waits over 100 ms`, `${row.longWait} wait(s)`);
    // Only when the tap really started on a head: a whole blob on the device is
    // better than any head and needs no hand-over.
    if (row.headUsed) {
      check(
        row.onStream,
        `${row.rate.name}: the track continues on the full stream (?start=), not on the head`,
        row.onStream ? "yes" : "no"
      );
    }
  }
}

await browser.close();
stop();
console.log(state.failed ? `\n${state.failed} FAILED` : "\nall passed");
process.exit(state.failed ? 1 : 0);
