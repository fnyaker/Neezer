#!/usr/bin/env node
// The Récap, end to end: what opening it costs, what walking it costs, and
// what the screen must never do.
//
//   node test/recap/run.mjs
//   DIST=/tmp/opencode/recap-dist node test/recap/run.mjs   # a build elsewhere
//   PLAYWRIGHT=/path/to/playwright-core CHROMIUM=/usr/bin/chromium node test/recap/run.mjs
//
// Starts tools/perf_api.py --recap --serve (the plan's own listening log: two
// years for `bench`, a thin year and a skips-only year for `thin`, no history
// for `fresh`; the SPA from `neezer/webui/dist`, or `DIST=`), then, in headless
// Chromium, holds the screen to its contract:
//
//   1. 1440x900, cold `#/recap`: TWO /api/recap requests (the years, then the
//      resolved year), the two year chips, the deck at most 460 px, the right
//      column on screen, no sideways overflow.
//   2. The nine cards, advanced with the keyboard: ZERO /api requests, at most
//      two cards mounted, the exact French titles and their order.
//   3. Covers: at most six for the opening card, at most 22 over the whole
//      deck, never more than six in flight (Cover.svelte's queue).
//   4. The home's card and the screen share ONE `/api/recap/years` per
//      session, whichever opens first.
//   5. A year change: ONE request, back to card 1.
//   6. The share sheet: zero requests to open and to export, a real 1080x1350
//      PNG download under 2 MB, its `toBlob` timed and printed.
//   7. Phone geometry at 390x844, 360x640 and 641x900: no sideways overflow,
//      44 px targets, tap zones 40/60, the chips scroll rather than wrap.
//   8. The 7 s auto-advance only at <= 900 px, paused while the share sheet is
//      open (and resumed after), absent under `prefers-reduced-motion`.
//   9. The states: an empty year, a thin year, no history, an out-of-range year
//      (one 400), a malformed year (ZERO requests), offline (no doomed
//      request), a cut link (LoadFailed + one request per retry).
//  10. Privacy: nothing named `recap` in localStorage/sessionStorage, no
//      `/recap` key in the offline API cache (nsupy-apicache).
//
// The covers are answered by the bench with a 1x1 PNG: the server has no
// artwork for synthetic tracks, and the pitchfork of `Cover.svelte`'s retry
// chain on a 404 would measure that chain instead of the deck's request
// pattern. What is counted is what the SCREEN asks for.
//
// Assertions are counts, geometry and exact strings; the timings (open, first
// card, toBlob, auto-advance) are printed for the report, never asserted —
// a busy machine must not turn a correct screen red.

import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { startServer, loadPlaywright, checker } from "../harness.mjs";

const { chromium } = await loadPlaywright();

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const DIST = process.env.DIST || join(REPO, "neezer/webui/dist");
const executablePath =
  process.env.CHROMIUM || (existsSync("/usr/bin/chromium") ? "/usr/bin/chromium" : undefined);

// A valid 1x1 PNG: the covers the synthetic library does not have.
const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9ZQmcAAAAASUVORK5CYII=",
  "base64"
);

const server = await startServer([
  "--recap",
  "--tracks",
  "400",
  "--playlists",
  "3",
  "--favorites",
  "50",
  "--big-playlist",
  "100",
  "--dist",
  DIST,
]);
const info = server.ids && server.ids.recap;
const { check, state } = checker();

if (!info) {
  console.log("FAIL: this server has no Récap seed (tools/perf_api.py --recap missing?)");
  server.stop();
  process.exit(1);
}

const CUR = info.current;
const PAST = info.past;
const EMPTY = info.empty;
const THIN = info.thin;
const NBSP = "\u202f";
const fmt = (n) => String(n).replace(/\B(?=(\d{3})+(?!\d))/g, NBSP);
const note = (what, value) => console.log(`note ${what}: ${value}`);

// The plan's exact seed (§8.4): 12 000 events in the running year, 316 days,
// 37 400 minutes; the finished year 25 000 events, 365 days, 77 916 minutes.
const EXPECTED_CUR = {
  minutes: 37_400,
  days: 316,
};
const EXPECTED_PAST = {
  minutes: 77_916,
  days: 365,
};

const SERVER = server.base;
const browser = await chromium.launch({
  executablePath,
  args: ["--no-sandbox", "--autoplay-policy=no-user-gesture-required"],
});

// -- helpers -----------------------------------------------------------------

async function newContext(opts = {}) {
  // `noRoutes` exists because Playwright's request routing bypasses Chromium's
  // HTTP cache: with a route registered, a second visit never revalidates with
  // If-None-Match. The context that PROVES the server's 304 on a re-open must
  // therefore let the browser cache work, and take the covers as the server
  // answers them (404 on this synthetic library).
  const { noRoutes = false, ...ctxOpts } = opts;
  const ctx = await browser.newContext({ acceptDownloads: true, ...ctxOpts });
  if (!noRoutes) {
    // Deezer's CDN is never part of a bench.
    await ctx.route(/dzcdn\.net|api\.deezer\.com|lrclib/, (r) => r.abort());
    await ctx.route("**/api/cover/**", (r) =>
      r.fulfill({ status: 200, contentType: "image/png", body: PNG })
    );
  }
  // Time the real `toBlob` call without touching the component.
  await ctx.addInitScript(() => {
    try {
      // The plan's bench #10 is the DOWNLOAD fallback: no Web Share in this
      // browser (a desktop Linux Chromium has none either, but making it
      // explicit keeps the run deterministic on every host).
      Object.defineProperty(navigator, "share", { value: undefined, configurable: true });
      Object.defineProperty(navigator, "canShare", { value: undefined, configurable: true });
    } catch {
      /* a browser that refuses the override: the check below says so */
    }
    try {
      const proto = HTMLCanvasElement.prototype;
      const original = proto.toBlob;
      window.__toBlob = [];
      proto.toBlob = function (cb, type, quality) {
        const canvas = this;
        const started = performance.now();
        return original.call(
          canvas,
          function (blob) {
            try {
              window.__toBlob.push({
                ms: performance.now() - started,
                size: blob ? blob.size : 0,
                w: canvas.width,
                h: canvas.height,
                type: blob ? blob.type : null,
              });
            } catch {
              /* a measurement must never break the page */
            }
            if (cb) cb(blob);
          },
          type,
          quality
        );
      };
    } catch {
      /* no canvas: the bench then simply has no timing */
    }
  });
  return ctx;
}

async function loginAs(ctx, username, password) {
  const rv = await ctx.request.post(SERVER + "/api/login", {
    data: { username, password },
  });
  if (!rv.ok()) throw new Error(`login ${username} failed: ${rv.status()}`);
}

/** Counts what the PAGE asks for: /api, /api/recap/*, covers, in-flight peak. */
function track(page) {
  const st = {
    api: 0,
    recap: 0,
    years: 0,
    year: 0,
    covers: 0,
    coverNow: 0,
    coverMax: 0,
    coverUrls: [],
    apiUrls: [],
    recapHeaders: [],
    statuses: {},
  };
  page.on("request", (r) => {
    const u = r.url();
    if (u.includes("/api/") && !u.includes("/api/cover/")) {
      st.api += 1;
      st.apiUrls.push(u);
    }
    if (u.includes("/api/recap/")) {
      // The conditional header is not in `headers()` (Playwright filters it
      // out); `allHeaders()` is what proves the browser revalidates the year.
      r.allHeaders()
        .then((h) => st.recapHeaders.push({ url: u, inm: h["if-none-match"] || null }))
        .catch(() => {});
    }
    if (u.includes("/api/recap/years")) {
      st.recap += 1;
      st.years += 1;
    } else if (u.includes("/api/recap/")) {
      st.recap += 1;
      st.year += 1;
    }
    if (u.includes("/api/cover/")) {
      st.covers += 1;
      st.coverUrls.push(u);
      st.coverNow += 1;
      st.coverMax = Math.max(st.coverMax, st.coverNow);
    }
  });
  const done = (r) => {
    if (r.url().includes("/api/cover/")) st.coverNow = Math.max(0, st.coverNow - 1);
  };
  page.on("requestfinished", done);
  page.on("requestfailed", done);
  page.on("response", (r) => {
    const u = r.url();
    const key = u.includes("/api/recap/years")
      ? "years"
      : u.includes("/api/recap/")
        ? String(u.split("/api/recap/")[1]).split("?")[0]
        : null;
    if (key) (st.statuses[key] ||= []).push(r.status());
  });
  return st;
}

const countText = async (page) =>
  (await page.locator(".story .foot .count").innerText()).trim();
const activeTitle = async (page) =>
  (await page.locator(".slide-wrap.active .slide .title").innerText()).trim();
// The COPY, not the rendered case: the kicker is uppercased by CSS
// (text-transform), and what the plan pins is the string in the source.
const activeKicker = async (page) =>
  (await page.locator(".slide-wrap.active .slide .kicker").textContent()).trim();

async function waitDeck(page, timeout = 30000) {
  await page.waitForSelector(".story .stage .slide-wrap.active .slide .title", { timeout });
}

async function waitCount(page, shown, total, timeout = 15000) {
  await page.waitForFunction(
    ([want, n]) => {
      const el = document.querySelector(".story .foot .count");
      return !!el && el.textContent.replace(/\s+/g, " ").trim() === `${want} / ${n}`;
    },
    [shown, total],
    { timeout }
  );
}

async function waitTitle(page, text, timeout = 15000) {
  await page.waitForFunction(
    (want) => {
      const el = document.querySelector(".slide-wrap.active .slide .title");
      return !!el && el.textContent.trim() === want;
    },
    text,
    { timeout }
  );
}

/** Every image of the ACTIVE card has finished (a card with none is settled). */
async function settleActiveArt(page) {
  await page
    .waitForFunction(
      () => {
        const imgs = [...document.querySelectorAll(".slide-wrap.active img")];
        return imgs.every((img) => img.complete);
      },
      null,
      { timeout: 8000 }
    )
    .catch(() => {});
}

/** Wait until no cover request is in flight (the queue is quiescent). */
async function settleCovers(st, page, timeout = 8000) {
  const deadline = Date.now() + timeout;
  let quiet = 0;
  while (Date.now() < deadline) {
    if (st.coverNow === 0) {
      quiet += 50;
      if (quiet >= 300) return;
    } else {
      quiet = 0;
    }
    await page.waitForTimeout(50);
  }
}

async function openRecap(page, hash) {
  await page.goto(SERVER + "/app/#" + hash);
}

function pngFacts(bytes) {
  const signature = bytes.subarray(0, 8).toString("hex");
  return {
    signature,
    isPng: signature === "89504e470d0a1a0a",
    width: bytes.length >= 24 ? bytes.readUInt32BE(16) : 0,
    height: bytes.length >= 24 ? bytes.readUInt32BE(20) : 0,
    size: bytes.length,
  };
}

const near = (a, b, epsilon = 1) => Math.abs(a - b) <= epsilon;

/** The cover URLs the RÉCAP payload itself names (mosaic, tops, first/last).
 * Requests for these are the deck's; the shell's playlist thumbnails are not. */
function deckCoverPaths(payload) {
  const out = new Set();
  const add = (url) => {
    if (typeof url === "string" && url) out.add(new URL(url, SERVER).pathname);
  };
  for (const row of (payload.tops && payload.tops.albums) || []) add(row.cover);
  for (const row of (payload.tops && payload.tops.tracks) || []) add(row.track && row.track.album && row.track.album.cover);
  for (const row of (payload.tops && payload.tops.artists) || []) add(row.cover);
  if (payload.first && payload.first.track && payload.first.track.album) add(payload.first.track.album.cover);
  if (payload.last && payload.last.track && payload.last.track.album) add(payload.last.track.album.cover);
  return out;
}
const deckCoverCount = (st, paths) =>
  st.coverUrls.filter((u) => paths.has(new URL(u).pathname)).length;
const distinctDeckCovers = (st, paths) =>
  new Set(st.coverUrls.map((u) => new URL(u).pathname).filter((p) => paths.has(p))).size;

// -- 1-3, 5-6, 10: the desktop story -----------------------------------------

console.log(`\n-- 1440x900: cold #/recap, the nine cards, a year change, the share`);
{
  const ctx = await newContext({ viewport: { width: 1440, height: 900 } });
  await loginAs(ctx, "bench", "Bench1");
  const page = await ctx.newPage();
  const st = track(page);
  const currentPayload = await (await ctx.request.get(SERVER + "/api/recap/" + CUR)).json();
  const deckCovers = deckCoverPaths(currentPayload);

  const t0 = Date.now();
  await openRecap(page, "/recap");
  await waitDeck(page);
  const openMs = Date.now() - t0;
  await settleActiveArt(page);
  note("cold #/recap -> the first card painted", `${openMs} ms (whole boot, 1440)`);

  check(
    st.years === 1 && st.year === 1 && st.recap === 2,
    `cold #/recap asks 2 /api/recap requests: 1 years + 1 year (measured ${st.years}+${st.year})`
  );
  const openingCovers = deckCoverCount(st, deckCovers);
  check(openingCovers <= 6, `the opening card asks <= 6 of the deck's covers (measured ${openingCovers})`);
  note(
    "the opening card's covers / all /api/cover on the page",
    `${openingCovers} (mosaic 4) / ${st.covers} (the rest are the shell's playlist thumbnails)`
  );

  const chips = await page.$$eval(".chips .chip", (els) => els.map((e) => e.textContent.trim()));
  check(
    JSON.stringify(chips) === JSON.stringify([`${CUR} · en cours`, String(PAST)]),
    `the chips are ["${CUR} · en cours", "${PAST}"] (measured ${JSON.stringify(chips)})`
  );
  check(!chips.some((c) => c.includes(String(EMPTY))), `no chip for the empty year ${EMPTY}`);
  check((await countText(page)) === "1 / 9", "the deck opens on card 1 of 9");
  check(
    (await page.locator(".story .bars .bar").count()) === 9,
    "nine progress bars for nine cards"
  );
  check(
    (await activeTitle(page)) === `Ton année ${CUR}`,
    `card 1 is "Ton année ${CUR}" (measured "${await activeTitle(page)}")`
  );
  check(
    (await activeKicker(page)) === `${CUR} · en cours`,
    `the running year says "en cours" (measured "${await activeKicker(page)}")`
  );
  const headSub = (await page.locator(".recap .head-sub").innerText()).trim();
  const nowDay = new Date();
  const dayLabel = `${nowDay.getDate() === 1 ? "1ᵉʳ" : nowDay.getDate()} ${new Intl.DateTimeFormat("fr-FR", {
    month: "long",
  }).format(nowDay)} ${nowDay.getFullYear()}`;
  check(
    headSub.startsWith("du 1ᵉʳ janvier au ") && headSub.endsWith(dayLabel),
    `the running year's period ends today (measured "${headSub}", expected "... au ${dayLabel}")`
  );

  const geo = await page.evaluate(() => {
    const box = (s) => {
      const e = document.querySelector(s);
      if (!e) return null;
      const r = e.getBoundingClientRect();
      return { x: r.x, y: r.y, w: r.width, h: r.height, right: r.right, bottom: r.bottom };
    };
    const display = (s) => {
      const e = document.querySelector(s);
      return e ? getComputedStyle(e).display : null;
    };
    const main = document.querySelector("main");
    return {
      doc: [document.documentElement.scrollWidth, document.documentElement.clientWidth],
      main: main ? [main.scrollWidth, main.clientWidth] : null,
      deck: box(".deck"),
      stage: box(".stage"),
      glance: box(".glance"),
      glanceDisplay: display(".glance"),
      shareBtn: box(".share-btn"),
      shareBtnDisplay: display(".share-btn"),
      shareHeadDisplay: display(".share-head"),
      gridCols: getComputedStyle(document.querySelector(".recap-grid")).gridTemplateColumns,
    };
  });
  check(geo.deck.w >= 360 && geo.deck.w <= 460, `the deck is 360..460 px wide (measured ${geo.deck.w.toFixed(1)})`);
  check(geo.gridCols.trim().split(/\s+/).length >= 2, `the desktop grid has two columns (measured "${geo.gridCols}")`);
  check(geo.glanceDisplay !== "none", "the right column (En un coup d'œil) is on screen");
  check(
    geo.shareBtnDisplay !== "none" && geo.shareHeadDisplay === "none",
    "the wide layout shares from the right column, not the header"
  );
  check(
    geo.doc[0] <= geo.doc[1] + 1,
    `no sideways overflow (doc ${geo.doc[0]} <= ${geo.doc[1]})`
  );
  if (geo.main) check(geo.main[0] <= geo.main[1] + 1, `main does not overflow (${geo.main[0]} <= ${geo.main[1]})`);

  // -- 2, 3: the nine cards, by keyboard ------------------------------------
  const expectedTitles = [
    `Ton année ${CUR}`,
    `${fmt(EXPECTED_CUR.minutes)} minutes`,
    "Tes 5 artistes",
    "Tes 5 titres",
    "Tes 5 albums",
    "Tes styles",
    `${fmt(EXPECTED_CUR.days)} jours d'affilée`,
    "Ton premier titre de l'année",
    "Partage ton année",
  ];
  await page.locator(".story").focus();
  const apiBefore = st.api;
  const titles = [await activeTitle(page)];
  const mounted = [];
  let labelsOk = true;
  for (let i = 1; i < 9; i++) {
    await page.keyboard.press("ArrowRight");
    await waitCount(page, i + 1, 9);
    await waitTitle(page, expectedTitles[i]);
    titles.push(await activeTitle(page));
    mounted.push(await page.locator(".stage .slide-wrap").count());
    labelsOk =
      labelsOk &&
      (await page.locator(".slide-wrap.active .slide").getAttribute("aria-label")) === `${i + 1} sur 9`;
    await settleActiveArt(page);
  }
  await settleCovers(st, page);
  check(st.api === apiBefore, `advancing the 8 remaining cards costs 0 /api requests (measured ${st.api - apiBefore})`);
  check(
    JSON.stringify(titles) === JSON.stringify(expectedTitles),
    `the nine titles, in the plan's order: ${JSON.stringify(titles)}`
  );
  check(
    mounted.every((n) => n <= 2),
    `at most 2 cards mounted at any step (measured ${JSON.stringify(mounted)})`
  );
  check(labelsOk, "each card announces its place (aria-label 'N sur 9')");
  const deckRequests = deckCoverCount(st, deckCovers);
  const deckDistinct = distinctDeckCovers(st, deckCovers);
  check(
    deckRequests <= 22,
    `the whole deck asks <= 22 of its covers (measured ${deckRequests}, ${deckDistinct} distinct)`
  );
  check(st.coverMax <= 6, `never more than 6 covers in flight (measured ${st.coverMax})`);

  // -- 5: changing year ------------------------------------------------------
  const yearsBefore = st.years;
  const yearBefore = st.year;
  await page.locator(".chips .chip", { hasText: String(PAST) }).click();
  await waitTitle(page, `Ton année ${PAST}`);
  check(st.years === yearsBefore, "a year change asks no /years (the chips are kept)");
  check(st.year === yearBefore + 1, `a year change asks exactly 1 year (measured ${st.year - yearBefore})`);
  check((await countText(page)) === "1 / 9", "a year change goes back to card 1");
  check((await activeKicker(page)) === String(PAST), "a finished year is not labelled en cours");
  // The keyboard belongs to the DECK (the plan's choice, so the chips keep
  // their own arrows): focus it after the chip click.
  await page.locator(".story").focus();
  await page.keyboard.press("ArrowRight");
  await waitTitle(page, `${fmt(EXPECTED_PAST.minutes)} minutes`);
  note("the finished year's minutes card", `"${await activeTitle(page)}"`);
  await page.keyboard.press("ArrowLeft");
  await waitCount(page, 1, 9);
  await settleCovers(st, page);

  // -- 6, 10: the share poster ---------------------------------------------
  const apiAtShare = st.api;
  const coversAtShare = st.covers;
  const recapAtShare = st.recap;
  check(
    await page.evaluate(() => typeof navigator.share === "undefined"),
    "no Web Share here: the download fallback is the path under test"
  );
  await page.locator(".share-btn").click();
  await page.waitForSelector(".rs-sheet", { timeout: 10000 });
  await page.waitForFunction(() => (window.__toBlob || []).some((b) => b.size > 0), null, {
    timeout: 20000,
  });
  check(
    st.recap === recapAtShare && st.covers === coversAtShare,
    `opening the share asks no /api/recap and no cover (measured ${st.recap - recapAtShare}/${st.covers - coversAtShare})`
  );
  note("other /api requests while the share was open (the shell's, not the share's)", st.api - apiAtShare);
  const blobs = await page.evaluate(() => window.__toBlob || []);
  const lastBlob = blobs.filter((b) => b.size).pop();
  if (lastBlob) {
    note("toBlob (1080x1350 PNG)", `${lastBlob.ms.toFixed(0)} ms, ${lastBlob.size} bytes`);
    check(lastBlob.w === 1080 && lastBlob.h === 1350, `the poster canvas is 1080x1350 (measured ${lastBlob.w}x${lastBlob.h})`);
    check(lastBlob.size <= 2 * 1024 * 1024, `the PNG blob is <= 2 MB (measured ${lastBlob.size})`);
  } else {
    check(false, "the poster was drawn and turned into a PNG blob");
  }
  const [download] = await Promise.all([
    page.waitForEvent("download", { timeout: 30000 }),
    page.locator(".rs-primary").click(),
  ]);
  const file = join(process.env.TMPDIR || "/tmp", download.suggestedFilename());
  await download.saveAs(file);
  const bytes = readFileSync(file);
  const png = pngFacts(bytes);
  check(
    download.suggestedFilename() === `neezer-recap-${PAST}.png`,
    `the download is named neezer-recap-${PAST}.png (measured ${download.suggestedFilename()})`
  );
  check(png.isPng, "the exported file is a real PNG");
  check(png.width === 1080 && png.height === 1350, `the exported PNG is 1080x1350 (measured ${png.width}x${png.height})`);
  check(png.size <= 2 * 1024 * 1024, `the exported PNG is <= 2 MB (measured ${png.size} bytes)`);
  check(
    st.recap === recapAtShare && st.covers === coversAtShare,
    `exporting the PNG asks no /api/recap and no cover (measured ${st.recap - recapAtShare}/${st.covers - coversAtShare})`
  );
  note("the exported PNG", `${png.size} bytes at ${file}`);
  await page.keyboard.press("Escape");
  await page.waitForSelector(".rs-sheet", { state: "detached", timeout: 10000 });

  // -- 10: personal data stays out of the device's caches -------------------
  const privacy = await page.evaluate(async () => {
    const local = Object.keys(localStorage).filter((k) => /recap/i.test(k));
    const session = Object.keys(sessionStorage).filter((k) => /recap/i.test(k));
    const paths = await new Promise((resolve) => {
      let req;
      try {
        req = indexedDB.open("nsupy-apicache");
      } catch {
        resolve([]);
        return;
      }
      req.onerror = () => resolve([]);
      req.onsuccess = () => {
        const db = req.result;
        if (!db.objectStoreNames.contains("responses")) return resolve([]);
        const tx = db.transaction("responses", "readonly").objectStore("responses").getAllKeys();
        tx.onsuccess = () => resolve(tx.result || []);
        tx.onerror = () => resolve([]);
      };
    });
    return { local, session, entries: paths.length, hits: paths.filter((p) => /recap/i.test(String(p))) };
  });
  check(privacy.local.length === 0, `no localStorage key names the recap (${JSON.stringify(privacy.local)})`);
  check(privacy.session.length === 0, `no sessionStorage key names the recap (${JSON.stringify(privacy.session)})`);
  check(
    privacy.hits.length === 0,
    `no /recap in the offline API cache (${privacy.entries} entries, ${JSON.stringify(privacy.hits)})`
  );

  // -- 9: the empty year, the invalid year, the malformed year -------------
  const recapBeforeStates = st.recap;
  const coversBeforeStates = st.covers;
  await page.evaluate((y) => (location.hash = "#/recap/" + y), EMPTY);
  await page.waitForFunction(
    (t) => document.querySelector(".panel-title")?.textContent.trim() === t,
    `Rien à raconter pour ${EMPTY}`,
    { timeout: 15000 }
  );
  check(
    (await page.locator(".panel-title").innerText()).trim() === `Rien à raconter pour ${EMPTY}`,
    `the empty year says "Rien à raconter pour ${EMPTY}"`
  );
  check(
    (await page.locator(".panel-text").innerText()).trim() === "Aucune écoute enregistrée.",
    "the empty year names the fact, not an error"
  );
  check(
    (await page.locator(".share-head, .share-btn").count()) === 0,
    "an empty year offers no share"
  );
  check(
    (await page.locator(".panel-actions a, .panel-actions button").filter({ hasText: `Voir ${PAST}` }).count()) === 1,
    `the empty year offers the nearest full year (Voir ${PAST})`
  );
  check(st.covers === coversBeforeStates, `an empty year asks no cover (measured ${st.covers - coversBeforeStates})`);

  const recapBeforeInvalid = st.recap;
  await page.evaluate(() => (location.hash = "#/recap/1999"));
  await page.waitForFunction(
    () => document.querySelector(".panel-title")?.textContent.trim() === "Cette année n'existe pas dans ton récap",
    null,
    { timeout: 15000 }
  );
  check(
    st.recap === recapBeforeInvalid + 1,
    `a year out of range asks the server once (400) (measured ${st.recap - recapBeforeInvalid})`
  );
  check(
    (await page.locator(".panel-actions button").filter({ hasText: "Réessayer" }).count()) === 0,
    "an out-of-range year offers no pointless Réessayer"
  );

  const recapBeforeMalformed = st.recap;
  await page.evaluate(() => (location.hash = "#/recap/abc"));
  await page.waitForFunction(
    () => document.querySelector(".panel-title")?.textContent.trim() === "Cette année n'existe pas dans ton récap",
    null,
    { timeout: 15000 }
  );
  // The panel was already up (the 1999 link): give the route handler its turn
  // before asserting that NOTHING was asked.
  await page.waitForTimeout(500);
  check(
    st.recap === recapBeforeMalformed,
    `a malformed year never reaches the server (measured ${st.recap - recapBeforeMalformed} requests)`
  );

  await ctx.close();
}

// -- 4: one /years per session, home first -----------------------------------

console.log(`\n-- home -> Récap: one /api/recap/years for the session, whichever opens first`);
{
  const ctx = await newContext({ viewport: { width: 1440, height: 900 }, noRoutes: true });
  await loginAs(ctx, "bench", "Bench1");
  const page = await ctx.newPage();
  const st = track(page);
  await page.goto(SERVER + "/app/");
  await page.waitForSelector(".recap-home", { timeout: 30000 });
  check(st.years === 1 && st.year === 0, `the home asks /years once and no year (measured ${st.years}+${st.year})`);
  // A 4x CPU throttle (what a mid-range phone feels like) from the tap to the
  // first card: a number for the report, not a budget (a busy machine must not
  // turn a correct screen red).
  const cdp = await ctx.newCDPSession(page);
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: 4 });
  const tapAt = Date.now();
  await page.locator(".recap-home").click();
  await waitDeck(page);
  note("4x CPU: tap on the home card -> the first card painted", `${Date.now() - tapAt} ms`);
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: 1 });
  check(st.years === 1, `opening the récap from the home asks no second /years (measured ${st.years})`);
  check(st.year === 1, `...only the year (measured ${st.year})`);
  // Back to the home, then the resolving #/recap: the warm years, one year.
  await page.evaluate(() => (location.hash = "#/"));
  await page.waitForSelector(".recap-home", { timeout: 15000 });
  await page.evaluate(() => (location.hash = "#/recap"));
  await waitDeck(page);
  check(st.years === 1, `#/recap with warm years asks no /years (measured ${st.years})`);
  check(st.year === 2, `#/recap re-opened asks the year once (measured ${st.year})`);
  // `allHeaders()` is a CDP round-trip: wait until every recap request has
  // had its headers captured before reading them.
  const headerDeadline = Date.now() + 3000;
  while (Date.now() < headerDeadline && st.recapHeaders.length < st.recap) {
    await page.waitForTimeout(25);
  }
  const conditional = st.recapHeaders.filter(
    (h) => h.url.endsWith("/api/recap/" + CUR) && h.inm
  );
  check(
    conditional.length === 1,
    `the re-opened year revalidates with If-None-Match (measured ${conditional.map((h) => h.inm).join(", ") || "none"})`
  );
  if (conditional.length) note("the re-opened year's If-None-Match", conditional[0].inm);
  note(
    "the re-opened year",
    `browser statuses ${JSON.stringify(st.statuses[CUR] || [])} (a revalidation is served to JS as the cached 200); perf_api measures the server's 304`
  );
  await ctx.close();
}

// -- 7-8: phone geometry, auto-advance, the share pause, offline -------------

console.log(`\n-- 390x844: geometry, the 7 s advance, its pause at the share, offline`);
{
  const ctx = await newContext({
    viewport: { width: 390, height: 844 },
    deviceScaleFactor: 2,
    isMobile: true,
    hasTouch: true,
  });
  await loginAs(ctx, "bench", "Bench1");
  const page = await ctx.newPage();
  const st = track(page);
  const phoneCovers = deckCoverPaths(await (await ctx.request.get(SERVER + "/api/recap/" + CUR)).json());
  await openRecap(page, `/recap/${CUR}`);
  await waitDeck(page);
  const deckAt = Date.now();
  await settleActiveArt(page);

  const g = await page.evaluate(() => {
    const box = (s) => {
      const e = document.querySelector(s);
      if (!e) return null;
      const r = e.getBoundingClientRect();
      return { x: r.x, y: r.y, w: r.width, h: r.height, right: r.right, bottom: r.bottom };
    };
    const style = (s, p) => {
      const e = document.querySelector(s);
      return e ? getComputedStyle(e)[p] : null;
    };
    const main = document.querySelector("main");
    return {
      vw: innerWidth,
      vh: innerHeight,
      doc: [document.documentElement.scrollWidth, document.documentElement.clientWidth],
      main: main ? [main.scrollWidth, main.clientWidth] : null,
      recap: (() => {
        const e = document.querySelector(".recap");
        return e ? [e.scrollWidth, e.clientWidth] : null;
      })(),
      deck: box(".deck"),
      stage: box(".stage"),
      chips: box(".chips"),
      chipsOverflow: style(".chips", "overflowX"),
      chipHeights: [...document.querySelectorAll(".chips .chip")].map(
        (e) => e.getBoundingClientRect().height
      ),
      prev: box(".tap.prev"),
      next: box(".tap.next"),
      shareHead: box(".share-head"),
      auto: document.querySelector(".story")?.classList.contains("auto"),
      glanceDisplay: style(".glance", "display"),
      shareBtnDisplay: style(".share-btn", "display"),
    };
  });
  check(g.doc[0] <= g.doc[1] + 1, `390: no sideways overflow (doc ${g.doc[0]} <= ${g.doc[1]})`);
  if (g.main) check(g.main[0] <= g.main[1] + 1, `390: main does not overflow (${g.main[0]} <= ${g.main[1]})`);
  if (g.recap) {
    // `.chips` carries a 2 px negative margin (its padding, so the focus ring
    // is not clipped): the screen's own scrollWidth is +2, `main` clips it.
    note("390: .recap scrollWidth/clientWidth", `${g.recap[0]}/${g.recap[1]} (chips' 2 px negative margin)`);
  }
  check(g.deck.w <= g.vw + 1 && g.deck.x >= -1 && g.deck.right <= g.vw + 1, `390: the deck fits the viewport (${g.deck.w.toFixed(1)} px)`);
  check(g.deck.h >= 487.5, `390: the deck keeps its 488 px floor (measured ${g.deck.h.toFixed(1)})`);
  check(g.chipsOverflow === "auto" || g.chipsOverflow === "scroll", `390: the chips row scrolls horizontally (overflow-x: ${g.chipsOverflow})`);
  check(g.chipHeights.every((h) => h >= 43.5), `390: every year chip is >= 44 px (measured ${JSON.stringify(g.chipHeights.map((h) => +h.toFixed(1)))})`);
  check(g.prev.h >= 43.5 && g.next.h >= 43.5, `390: both tap zones are >= 44 px tall (${g.prev.h.toFixed(1)}, ${g.next.h.toFixed(1)})`);
  check(
    near(g.prev.w, g.stage.w * 0.4) && near(g.next.w, g.stage.w * 0.6),
    `390: the tap zones are 40/60 (measured ${(g.prev.w / g.stage.w).toFixed(2)}/${(g.next.w / g.stage.w).toFixed(2)})`
  );
  check(g.shareHead.h >= 43.5, `390: the header's Partager is >= 44 px (measured ${g.shareHead.h.toFixed(1)})`);
  check(g.auto === true, "390: the deck is on its auto-advance path (<= 900 px)");
  check(g.glanceDisplay === "none" && g.shareBtnDisplay === "none", "390: the wide column is not rendered");
  note("390 deck / stage / viewport", `${g.deck.w.toFixed(1)}x${g.deck.h.toFixed(1)} in ${g.vw}x${g.vh}, bottom ${g.deck.bottom.toFixed(1)}`);

  // 8: the 7 s advance (wait for the event, then report the elapsed time).
  await page.waitForFunction(
    () => (document.querySelector(".story .foot .count")?.textContent || "").replace(/\s+/g, " ").trim() === "2 / 9",
    null,
    { timeout: 10000 }
  );
  note("auto-advance card 1 -> 2", `${Date.now() - deckAt} ms after the deck painted (7 s timer)`);

  // The card after the one on screen loads its art when the deck advances:
  // that is the ADVANCE's cost, and letting it settle first is what makes the
  // share window below attributable to the share.
  await settleCovers(st, page);

  // 8: paused while the share sheet is open; resumed after it closes.
  const atShare = await countText(page);
  const apiAtShare = st.api;
  const recapAtShare = st.recap;
  const deckCoversAtShare = deckCoverCount(st, phoneCovers);
  await page.locator(".share-head").click();
  await page.waitForSelector(".rs-sheet", { timeout: 10000 });
  check(
    await page.evaluate(() => document.querySelector(".story").classList.contains("paused")),
    "390: the deck is paused by the open share sheet"
  );
  check(
    st.recap === recapAtShare && deckCoverCount(st, phoneCovers) === deckCoversAtShare,
    `390: opening the share sheet asks no /api/recap and none of the deck's covers (measured ${st.recap - recapAtShare}/${deckCoverCount(st, phoneCovers) - deckCoversAtShare})`
  );
  await page.waitForTimeout(8000);
  check((await countText(page)) === atShare, `390: no auto-advance while the share sheet is open (8 s > 7 s, still "${atShare}")`);
  check(
    st.recap === recapAtShare && deckCoverCount(st, phoneCovers) === deckCoversAtShare,
    `390: 8 s of open share sheet still asks no /api/recap and none of the deck's covers (measured ${st.recap - recapAtShare}/${deckCoverCount(st, phoneCovers) - deckCoversAtShare})`
  );
  if (st.api !== apiAtShare) {
    // The app shell keeps living behind the sheet (its own favourites/playlists
    // prefetch, the remote-link probe): none of it belongs to the Récap, and
    // the share itself is the zero above.
    note(
      "390: other /api requests while the sheet was up (the shell's, not the share's)",
      JSON.stringify(st.apiUrls.slice(-(st.api - apiAtShare)))
    );
  }
  await page.keyboard.press("Escape");
  await page.waitForSelector(".rs-sheet", { state: "detached", timeout: 10000 });
  const resumeAt = Date.now();
  await page.waitForFunction(
    (prev) => {
      const el = document.querySelector(".story .foot .count");
      if (!el) return false;
      const shown = Number(el.textContent.split("/")[0].trim());
      return shown === prev + 1;
    },
    Number(atShare.split("/")[0].trim()),
    { timeout: 10000 }
  );
  note("auto-advance resumed after the share closed", `${Date.now() - resumeAt} ms`);

  // 9: offline is a state, not a doomed request.
  const recapBeforeOffline = st.recap;
  const yearBeforeOffline = st.year;
  await ctx.setOffline(true);
  await page.waitForFunction(() => navigator.onLine === false, null, { timeout: 5000 });
  await page.waitForTimeout(200); // the browser's offline event reaches lib/net.js
  await page.evaluate(() => (location.hash = "#/recap"));
  await page.waitForFunction(
    () => document.querySelector(".panel-title")?.textContent.trim() === "Ton récap a besoin du serveur",
    null,
    { timeout: 15000 }
  );
  check(
    st.recap === recapBeforeOffline,
    `offline: opening #/recap makes no doomed request (measured ${st.recap - recapBeforeOffline})`
  );
  await ctx.setOffline(false);
  await page.waitForFunction(() => navigator.onLine === true, null, { timeout: 5000 });
  await page.locator(".panel-actions button").filter({ hasText: "Réessayer" }).click();
  await waitDeck(page, 20000);
  check(
    st.year === yearBeforeOffline + 1,
    `back online, one Réessayer = one year request (measured ${st.year - yearBeforeOffline})`
  );
  await ctx.close();
}

// -- 7: 360x640 and 641x900 --------------------------------------------------

for (const [w, h, wantAuto] of [
  [360, 640, true],
  [641, 900, true],
]) {
  console.log(`\n-- ${w}x${h}: geometry only`);
  const ctx = await newContext({
    viewport: { width: w, height: h },
    deviceScaleFactor: w <= 640 ? 2 : 1,
    isMobile: w <= 640,
    hasTouch: w <= 640,
  });
  await loginAs(ctx, "bench", "Bench1");
  const page = await ctx.newPage();
  await openRecap(page, `/recap/${CUR}`);
  await waitDeck(page);
  const g = await page.evaluate(() => {
    const box = (s) => {
      const e = document.querySelector(s);
      if (!e) return null;
      const r = e.getBoundingClientRect();
      return { x: r.x, y: r.y, w: r.width, h: r.height, right: r.right, bottom: r.bottom };
    };
    const main = document.querySelector("main");
    const recap = document.querySelector(".recap");
    return {
      vw: innerWidth,
      doc: [document.documentElement.scrollWidth, document.documentElement.clientWidth],
      main: main ? [main.scrollWidth, main.clientWidth] : null,
      recap: recap ? [recap.scrollWidth, recap.clientWidth] : null,
      deck: box(".deck"),
      stage: box(".stage"),
      chips: box(".chips"),
      chipsOverflow: getComputedStyle(document.querySelector(".chips")).overflowX,
      chipHeights: [...document.querySelectorAll(".chips .chip")].map(
        (e) => e.getBoundingClientRect().height
      ),
      prev: box(".tap.prev"),
      next: box(".tap.next"),
      auto: document.querySelector(".story")?.classList.contains("auto"),
      glanceDisplay: getComputedStyle(document.querySelector(".glance")).display,
    };
  });
  check(g.doc[0] <= g.doc[1] + 1, `${w}: no sideways overflow (doc ${g.doc[0]} <= ${g.doc[1]})`);
  if (g.main) check(g.main[0] <= g.main[1] + 1, `${w}: main does not overflow (${g.main[0]} <= ${g.main[1]})`);
  if (g.recap) {
    note(`${w}: .recap scrollWidth/clientWidth`, `${g.recap[0]}/${g.recap[1]} (chips' 2 px negative margin)`);
  }
  check(g.deck.w <= g.vw + 1 && g.deck.right <= g.vw + 1, `${w}: the deck fits the viewport (${g.deck.w.toFixed(1)} px)`);
  check(g.deck.h >= 487.5, `${w}: the deck keeps its 488 px floor (measured ${g.deck.h.toFixed(1)})`);
  if (w === 641) {
    check(g.deck.w <= 460, `${w}: the narrow-desktop deck is <= 460 px (measured ${g.deck.w.toFixed(1)})`);
    check(g.glanceDisplay === "none", `${w}: the wide column waits for a wider container`);
  }
  check(g.chipsOverflow === "auto" || g.chipsOverflow === "scroll", `${w}: the chips row scrolls horizontally`);
  check(g.chipHeights.every((hh) => hh >= 43.5), `${w}: every year chip is >= 44 px`);
  check(g.prev.h >= 43.5 && g.next.h >= 43.5, `${w}: both tap zones are >= 44 px tall`);
  check(g.auto === wantAuto, `${w}: auto-advance ${wantAuto ? "on" : "off"} (<= 900 px)`);
  await ctx.close();
}

// -- 9: thin, skips-only, no history -----------------------------------------

console.log(`\n-- the empty states: thin year, skips-only year, no history at all`);
{
  const ctx = await newContext({ viewport: { width: 390, height: 844 } });
  await loginAs(ctx, "thin", "Thin1");
  const page = await ctx.newPage();
  const st = track(page);
  await openRecap(page, `/recap/${THIN.year}`);
  await page.waitForFunction(
    (t) => document.querySelector(".panel-title")?.textContent.trim() === t,
    `Trop peu d'écoutes en ${THIN.year}`,
    { timeout: 20000 }
  );
  check(
    (await page.locator(".panel-title").innerText()).trim() === `Trop peu d'écoutes en ${THIN.year}`,
    `the thin year says "Trop peu d'écoutes en ${THIN.year}"`
  );
  check(
    (await page.locator(".panel-text").innerText()).trim() ===
      `${THIN.listens} écoutes, ${THIN.minutes} minutes.`,
    "the thin year reports its real numbers (5 listens, 12 minutes)"
  );
  check((await page.locator(".share-head, .share-btn").count()) === 0, "a thin year offers no share");
  check(st.covers === 0, `a thin year asks no cover (measured ${st.covers})`);

  const coversBefore = st.covers;
  await page.evaluate((y) => (location.hash = "#/recap/" + y), THIN.skips_year);
  await page.waitForFunction(
    (t) => document.querySelector(".panel-title")?.textContent.trim() === t,
    `Aucune écoute en ${THIN.skips_year}`,
    { timeout: 20000 }
  );
  check(
    (await page.locator(".panel-text").innerText()).trim() === "Seulement des titres passés.",
    "a skips-only year names the skips, and says there was no listen"
  );
  check(st.covers === coversBefore, "a skips-only year asks no cover");
  await ctx.close();
}

{
  const ctx = await newContext({ viewport: { width: 390, height: 844 } });
  await loginAs(ctx, "fresh", "Fresh1");
  const page = await ctx.newPage();
  const st = track(page);
  await openRecap(page, "/recap");
  await page.waitForFunction(
    () =>
      document.querySelector(".panel-title")?.textContent.trim() ===
      "Pas encore de récap",
    null,
    { timeout: 20000 }
  );
  check(
    (await page.locator(".panel-title").innerText()).trim() ===
      "Pas encore de récap",
    "no history at all: the screen promises, it does not error"
  );
  check(st.years === 1 && st.year === 0, `no history: one /years, no year (measured ${st.years}+${st.year})`);
  check(st.covers === 0, "no history: no cover");
  await ctx.close();
}

// -- 9: a cut link, and one request per retry --------------------------------

console.log(`\n-- a cut link: LoadFailed + Réessayer, one request per retry`);
{
  const ctx = await newContext({ viewport: { width: 1440, height: 900 } });
  await loginAs(ctx, "bench", "Bench1");
  const page = await ctx.newPage();
  const st = track(page);
  await page.route("**/api/recap/**", (r) => r.abort());
  await openRecap(page, `/recap/${CUR}`);
  // The error is a `.panel.card` whose LoadFailed paragraph is the cause.
  await page.waitForFunction(
    () => /Récap indisponible/.test(document.querySelector(".panel.card")?.textContent || ""),
    null,
    { timeout: 30000 }
  );
  check(
    /Récap indisponible/.test(await page.locator(".panel.card").innerText()),
    "a cut link says the récap is unavailable (LoadFailed), not an endless spinner"
  );
  check(
    (await page.locator(".panel-actions button").filter({ hasText: "Réessayer" }).count()) === 1,
    "and offers Réessayer"
  );
  note("the aborted attempts before the ladder gave up", `${st.recap} requests`);
  await page.unroute("**/api/recap/**");
  const yearBefore = st.year;
  await page.locator(".panel-actions button").filter({ hasText: "Réessayer" }).click();
  await waitDeck(page, 30000);
  check(
    st.year === yearBefore + 1,
    `one Réessayer = one year request, no replayed budget (measured ${st.year - yearBefore})`
  );
  await ctx.close();
}

// -- 8: reduced motion -------------------------------------------------------

console.log(`\n-- prefers-reduced-motion: no auto-advance at all`);
{
  const ctx = await newContext({
    viewport: { width: 390, height: 844 },
    reducedMotion: "reduce",
    deviceScaleFactor: 2,
    isMobile: true,
    hasTouch: true,
  });
  await loginAs(ctx, "bench", "Bench1");
  const page = await ctx.newPage();
  await openRecap(page, `/recap/${CUR}`);
  await waitDeck(page);
  const g = await page.evaluate(() => ({
    reduce: matchMedia("(prefers-reduced-motion: reduce)").matches,
    auto: document.querySelector(".story").classList.contains("auto"),
    fillAnimation: (() => {
      const fill = document.querySelector(".story .bars .bar .fill");
      return fill ? getComputedStyle(fill).animationName : null;
    })(),
  }));
  check(g.reduce && g.auto === false, "reduced motion: the deck is not on the auto path");
  check(g.fillAnimation === "none", `reduced motion: the progress bar does not animate (measured ${g.fillAnimation})`);
  await page.waitForTimeout(8000);
  check((await countText(page)) === "1 / 9", "reduced motion: still card 1 after 8 s");
  await ctx.close();
}

await browser.close();
server.stop();

console.log("");
if (state.failed) {
  console.log(`FAIL: ${state.failed} check(s) failed`);
  process.exit(1);
}
console.log(`ok — ${server.base} (seed ${PAST}/${CUR}, empty ${EMPTY})`);
