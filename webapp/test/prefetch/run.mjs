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
// Each cell is the median of three runs, and the stream requests and bytes the
// tap itself causes are counted from the network (CDP), not guessed.
//
// What the rule (lib/netyield.js#prefetchTrickle, lib/playcache.js) promises:
// the preload reads at the trickle pace while the element playing needs the
// link, and at full speed once it has enough in hand — so a slow link pays for
// the preload at most what the player leaves, never more.
//
// Measured on 2026-11-26, median of 3, 40 s WAV tracks (320 KB), CDP 150 ms:
//
//   link      preload  next at 1 s        next at 14 s          preloaded
//   LAN       off      0.3 s   1 req      0.3 s   1 req 313 KB   0
//   LAN       on       0.3 s   1 req      0.3 s   0 req            313 KB
//   16 KB/s   off     20.9 s   1 req     25.9 s   1 req 226 KB   0
//   16 KB/s   on      29.5 s   1 req     87.7 s   1 req 177 KB   196 KB
//   64 KB/s   off      3.9 s   1 req      4.2 s   1 req 235 KB   0
//   64 KB/s   on       3.9 s   1 req      0.3 s   0 req            313 KB
//
// On LAN and at 64 KB/s the preload does what it is for (the whole blob on the
// device, no stream request at the tap). At 16 KB/s a 320 KB track cannot be
// preloaded inside the 14 s before the tap, so the trickle is pure cost: 4 KB/s
// is 27 % of the link, the cold start goes past COLD_START_MS (30 s), the
// watchdog reloads the stream four times from zero — and 25 s is already missed
// by the run with no preload at all (25.9 s).
import { mkdtempSync, writeFileSync } from "node:fs";
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

const { chromium } = await loadPlaywright();
const LEN = 40; // seconds; the bench WAV is 8 KB a second, so 320 KB a track
const wavPath = join(mkdtempSync(join(tmpdir(), "nspf-")), "track.wav");
writeFileSync(wavPath, silence(LEN));
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
const HOT_AFTER_MS = 14000; // ...and its hot one
const BOUND_MS = 25000; // the lot's bound on a slow link

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
async function run({ kbps, prefetch }) {
  const ctx = await browser.newContext({ viewport: { width: 390, height: 844 } });
  await ctx.route(deezerHost, (r) => r.abort());
  await ctx.addInitScript(installSessionPrefix);
  await ctx.addInitScript(() => {
    // The page's own elements, so a measurement names the element that plays.
    const Native = window.Audio;
    window.__els = [];
    window.Audio = function (...a) {
      const el = new Native(...a);
      window.__els.push(el);
      return el;
    };
    window.Audio.prototype = Native.prototype;
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
    const { session, prefetch } = JSON.parse(j);
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
  await login(page, base);
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
    JSON.stringify({ session, prefetch })
  );
  await page.goto(`${base}/app/`);
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
  const coldReqs = streams.slice(coldTapAt).filter((s) => s.start <= marks.tap + 1500).length;
  marks.tap = 0;

  // HOT: 14 s into the track that is now playing, with the preload of the
  // following one running (or not, when the setting is off).
  await page.waitForTimeout(HOT_AFTER_MS);
  const prefetchedBefore = streams.filter((s) => s.bg).reduce((n, s) => n + s.bytes, 0);
  const hotTapAt = streams.length;
  const hotBefore = await srcs();
  marks.tap = Date.now();
  const hotTap = marks.tap - t0;
  await page.locator("button.next").first().click();
  const hot = await sound(TRACKS[2], hotBefore);
  const hotTapMs = Date.now() - marks.tap;
  const hotStreams = streams.slice(hotTapAt).filter((s) => s.start <= marks.tap + 1500);
  marks.tap = 0;

  const trace = process.env.DEBUG
    ? await page.evaluate(() => window.__trace).catch(() => [])
    : [];
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
  };
}

const median = (a) => {
  const s = [...a].sort((x, y) => x - y);
  return s[Math.floor(s.length / 2)];
};
const secs = (ms) => (ms < 0 ? "timeout" : (ms / 1000).toFixed(1) + " s");

const table = [];
for (const rate of RATES) {
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
          `preloaded ${(r.prefetched / 1024).toFixed(0)} KB`
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
    });
  }
}

console.log("\nrate       prefetch   cold (next at 1 s)      hot (next at 14 s)      preloaded");
for (const row of table) {
  console.log(
    `${row.rate.name.padEnd(10)} ${(row.prefetch ? "ON" : "OFF").padEnd(9)} ` +
      `${secs(row.cold).padStart(7)}  ${String(row.coldReqs).padStart(2)} req   ` +
      `${secs(row.hot).padStart(7)}  ${String(row.hotReqs).padStart(2)} req  ` +
      `${(row.hotTapBytes / 1024).toFixed(0).padStart(4)} KB   ` +
      `${(row.prefetched / 1024).toFixed(0).padStart(4)} KB`
  );
}

const at = (name, prefetch) => table.find((r) => r.rate.name === name && r.prefetch === prefetch);
const checked = new Set();
for (const row of table) {
  if (!row.rate.kbps || checked.has(row.rate.name)) continue;
  checked.add(row.rate.name);
  const on = at(row.rate.name, true);
  const off = at(row.rate.name, false);
  check(
    on.cold > 0 && on.cold <= BOUND_MS,
    `${row.rate.name}: with the preload, a next at 1 s still sounds within ${BOUND_MS / 1000} s`,
    secs(on.cold)
  );
  check(
    on.hot > 0 && on.hot <= BOUND_MS,
    `${row.rate.name}: with the preload, a next 14 s in sounds within ${BOUND_MS / 1000} s`,
    secs(on.hot)
  );
  if (!off) continue;
  check(
    on.hot > 0 && off.hot > 0 && on.hot <= off.hot + 2000,
    `${row.rate.name}: the preload never costs the tap more than 2 s`,
    `${secs(on.hot)} against ${secs(off.hot)}`
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

await browser.close();
stop();
console.log(state.failed ? `\n${state.failed} FAILED` : "\nall passed");
process.exit(state.failed ? 1 : 0);
