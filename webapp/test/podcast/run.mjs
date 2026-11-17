#!/usr/bin/env node
import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { startServer, silence, checker } from "../harness.mjs";

async function loadPlaywright() {
  const specs = [
    process.env.PLAYWRIGHT,
    "playwright",
    "playwright-core",
    "/tmp/opencode/r1/node_modules/playwright-core/index.mjs",
  ].filter(Boolean);
  let last;
  for (const spec of specs) {
    try {
      return await import(spec);
    } catch (e) {
      last = e;
    }
  }
  throw last;
}

const { chromium } = await loadPlaywright();
const REPO = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const DIST = process.env.DIST || join(REPO, "neezer/webui/dist");
const executablePath =
  process.env.CHROMIUM || (existsSync("/usr/bin/chromium") ? "/usr/bin/chromium" : undefined);
const { check, state } = checker();
const note = (what, value) => console.log(`note ${what}: ${value}`);

const server = await startServer([
  "--tracks",
  "400",
  "--playlists",
  "3",
  "--favorites",
  "50",
  "--big-playlist",
  "100",
  "--runs",
  "1",
  "--podcasts",
  "--dist",
  DIST,
]);
const seed = server.ids && server.ids.podcast;
if (!seed) {
  console.log("skip: tools/perf_api.py --podcasts did not seed a show");
  server.stop();
  process.exit(0);
}

const browser = await chromium.launch({
  executablePath,
  args: ["--no-sandbox", "--autoplay-policy=no-user-gesture-required"],
});
const WAV = silence(1800);

function track(page, show) {
  const st = {
    show: 0,
    chapters: [],
    transcripts: [],
    showSearch: 0,
    showPost: 0,
    startPost: 0,
    api: [],
  };
  page.on("request", (request) => {
    const url = new URL(request.url());
    const path = url.pathname;
    const method = request.method();
    if (path.startsWith("/api/")) st.api.push(method + " " + path + (url.search || ""));
    if (path === "/api/podcast/" + show && method === "GET") st.show += 1;
    if (/^\/api\/podcast\/episode\/[^/]+\/chapters$/.test(path)) st.chapters.push(path);
    if (/^\/api\/podcast\/episode\/[^/]+\/transcript$/.test(path) && method === "GET") st.transcripts.push(path);
    if (path === "/api/podcast/" + show + "/transcript/search") st.showSearch += 1;
    if (path === "/api/podcast/" + show + "/transcribe" && method === "POST") st.showPost += 1;
    if (/^\/api\/podcast\/episode\/[^/]+\/transcribe$/.test(path) && method === "POST") st.startPost += 1;
  });
  return st;
}

const chapterCount = (st, episode) =>
  st.chapters.filter((path) => path.includes("/" + episode + "/")).length;
const transcriptCount = (st, episode) =>
  st.transcripts.filter((path) => path.includes("/" + episode + "/")).length;

async function newContext(viewport, phone) {
  const ctx = await browser.newContext({
    viewport,
    deviceScaleFactor: phone ? 2 : 1,
    isMobile: phone,
    hasTouch: phone,
  });
  await ctx.route(/dzcdn\.net|api\.deezer\.com|lrclib/, (r) => r.abort());
  await ctx.route(/\/api\/stream\//, async (r) => {
    const range = r.request().headers().range || "";
    const match = /bytes=(\d+)-(\d*)/.exec(range);
    if (!match) {
      return r.fulfill({
        status: 200,
        body: WAV,
        headers: {
          "Content-Type": "audio/wav",
          "Accept-Ranges": "bytes",
          "Content-Length": String(WAV.length),
        },
      });
    }
    const start = Number(match[1]);
    const end = match[2] ? Number(match[2]) : WAV.length - 1;
    return r.fulfill({
      status: 206,
      body: WAV.subarray(start, end + 1),
      headers: {
        "Content-Type": "audio/wav",
        "Accept-Ranges": "bytes",
        "Content-Range": `bytes ${start}-${end}/${WAV.length}`,
        "Content-Length": String(end - start + 1),
      },
    });
  });
  return ctx;
}

async function loginVia(ctx, username = "bench", password = "Bench1") {
  const response = await ctx.request.post(server.base + "/api/login", {
    data: { username, password },
  });
  if (!response.ok()) throw new Error("login " + response.status());
}

async function openShow(page) {
  await page.goto(server.base + "/app/#/podcast/" + seed.show);
  await page.waitForSelector(".episodes li", { timeout: 30000 });
  await page.waitForTimeout(500);
}

async function openPlayer(page, phone) {
  await page.locator("footer.player .now").click();
  await page.waitForSelector(phone ? ".m" : ".d", { timeout: 15000 });
  await page.waitForTimeout(400);
}

const overflow = (page) =>
  page.evaluate(() => ({
    doc: document.documentElement.scrollWidth,
    width: document.documentElement.clientWidth,
  }));

const rowHeights = (page, selector) =>
  page.locator(selector).evaluateAll((els) => els.map((el) => el.getBoundingClientRect().height));

async function waitSeg(page, scope, text, timeout = 15000) {
  await page.waitForFunction(
    ([root, wanted]) => {
      const rows = [...document.querySelectorAll(root + " .seg")];
      return rows.some(
        (row) =>
          row.classList.contains("on") && row.querySelector("time")?.textContent.trim() === wanted
      );
    },
    [scope, text],
    { timeout }
  );
}

console.log("\n-- 1440x900: the show opens without chapters, then the panel asks once");
{
  const ctx = await newContext({ width: 1440, height: 900 }, false);
  await loginVia(ctx);
  const page = await ctx.newPage();
  const st = track(page, seed.show);
  page.on("pageerror", (e) => console.log("pageerror:", e.message));
  await openShow(page);

  check(st.show === 1, `1440: opening the show asks 1 podcast GET (measured ${st.show})`);
  check(st.chapters.length === 0, `1440: the show page asks no chapter list (measured ${st.chapters.length})`);
  check(st.transcripts.length === 0, `1440: the show page asks no transcript (measured ${st.transcripts.length})`);
  const ov = await overflow(page);
  check(ov.doc <= ov.width + 1, `1440: the show page does not overflow (${ov.doc}/${ov.width})`);

  const searchField = page.locator(".tr-search input");
  check(await searchField.count() === 1, "1440: a ready transcript offers the show search");
  const beforeSearch = st.showSearch;
  await searchField.pressSequentially("memoire", { delay: 40 });
  await page.waitForSelector(".tr-results-head", { timeout: 15000 });
  await page.waitForTimeout(500);
  check(st.showSearch - beforeSearch === 1, `1440: the show search asks 1 GET (measured ${st.showSearch - beforeSearch})`);
  const hits = await page.locator(".hits .hit").count();
  check(hits > 0, `1440: the show search returns passages (measured ${hits})`);
  note("1440: show search result line", (await page.locator(".tr-results-head").innerText()).replace(/\s+/g, " ").trim());
  await page.locator(".tr-results-head .tr-link").click();
  await page.waitForTimeout(300);

  check((await page.locator(".tr-progress").count()) === 1, "1440: the admin card shows the running transcript");
  check(
    (await page.locator(".tr-progress").innerText()).includes("42"),
    `1440: the running progress reads 42 % (${(await page.locator(".tr-progress").innerText()).replace(/\s+/g, " ").trim()})`
  );
  check(
    (await page.locator(".tr-head .tr-count").innerText()).trim() === "1 sur 8",
    `1440: the card counts 1 ready over 8 archived (measured ${(await page.locator(".tr-head .tr-count").innerText()).trim()})`
  );
  await page.waitForSelector(".tr-switch", { timeout: 15000 });
  const beforeMark = st.showPost;
  await page.locator(".tr-switch").click();
  await page.waitForFunction(
    () => document.querySelector(".tr-switch")?.getAttribute("aria-checked") === "true",
    null,
    { timeout: 15000 }
  );
  check(st.showPost - beforeMark === 1, `1440: marking the show sends 1 POST (measured ${st.showPost - beforeMark})`);
  note("1440: marking toast", (await page.locator(".toast").last().innerText().catch(() => "")).replace(/\s+/g, " ").trim());

  await page.locator(".episodes li .pl").first().click();
  await page.waitForSelector("footer.player .now", { timeout: 15000 });
  await openPlayer(page, false);
  await page.locator(".d .tabs button", { hasText: "À propos" }).click();
  await page.waitForSelector(".side .card-label", { timeout: 15000 });
  check(
    chapterCount(st, seed.ready) === 1,
    `1440: opening the panel asks the chapters once (measured ${chapterCount(st, seed.ready)})`
  );
  check(
    (await page.locator(".side .card-label").textContent()).trim() === "Chapitre 1 sur 3",
    `1440: the panel names chapter 1 of 3 (measured ${(await page.locator(".side .card-label").textContent()).trim()})`
  );
  check((await page.locator(".side .chapter-row").count()) === 3, "1440: the panel lists the 3 chapters");

  const beforeTranscript = transcriptCount(st, seed.ready);
  await page.locator(".side .seg-tabs button", { hasText: "Transcription" }).click();
  await page.waitForSelector(".side .seg", { timeout: 15000 });
  await page.waitForTimeout(400);
  check(
    transcriptCount(st, seed.ready) - beforeTranscript === 1,
    `1440: opening the transcript asks 1 page GET (measured ${transcriptCount(st, seed.ready) - beforeTranscript})`
  );
  check((await page.locator(".side .seg").count()) === 200, `1440: one page of 200 lines is mounted (measured ${await page.locator(".side .seg").count()})`);
  await waitSeg(page, ".side", "0:00");
  check(true, "1440: the current segment is announced at 0:00");

  await page.locator(".side .seg", { hasText: "10:00" }).first().click();
  await waitSeg(page, ".side", "10:00");
  check(true, "1440: tapping the 10:00 line makes it current");

  const beforeQuiet = transcriptCount(st, seed.ready);
  const beforeQuietAll = st.transcripts.length;
  await page.waitForTimeout(4200);
  check(
    transcriptCount(st, seed.ready) === beforeQuiet && st.transcripts.length === beforeQuietAll,
    `1440: 4 s of playback asks no transcript (measured ${transcriptCount(st, seed.ready) - beforeQuiet})`
  );

  const beforePanelSearch = transcriptCount(st, seed.ready);
  await page.locator(".side .searchbox input").fill("memoire");
  await page.waitForSelector(".side .hit", { timeout: 15000 });
  await page.waitForTimeout(400);
  check(
    transcriptCount(st, seed.ready) - beforePanelSearch === 1,
    `1440: the panel search asks 1 GET (measured ${transcriptCount(st, seed.ready) - beforePanelSearch})`
  );
  check((await page.locator(".side .hit").count()) > 0, "1440: the panel search returns hits");
  const beforeHit = transcriptCount(st, seed.ready);
  await page.locator(".side .hit").first().click();
  await page.waitForFunction(
    () => /Résultat 1 sur/.test(document.querySelector(".side .meta")?.textContent || ""),
    null,
    { timeout: 15000 }
  );
  check(
    transcriptCount(st, seed.ready) - beforeHit === 1,
    `1440: tapping a hit loads its page once (measured ${transcriptCount(st, seed.ready) - beforeHit})`
  );

  const about = await page.locator(".side").boundingBox();
  check(about && about.width >= 299 && about.width <= 361, `1440: the À propos column is 300..360 px (measured ${about && about.width.toFixed(1)})`);
  const segmentHeights = await rowHeights(page, ".side .seg");
  check(segmentHeights.every((value) => value >= 43.5), `1440: every transcript line is at least 44 px (measured ${JSON.stringify(segmentHeights.slice(0, 6).map((v) => Math.round(v)))})`);
  const ov2 = await overflow(page);
  check(ov2.doc <= ov2.width + 1, `1440: the full-screen player does not overflow (${ov2.doc}/${ov2.width})`);
  await page.locator('.d button[aria-label="Réduire"]').click();
  await ctx.close();
}

console.log("\n-- 1440x900: the show's inline chapters, the 100-chapter sheet, the chapter seek");
{
  const ctx = await newContext({ width: 1440, height: 900 }, false);
  await loginVia(ctx);
  const page = await ctx.newPage();
  const st = track(page, seed.show);
  await openShow(page);
  check(st.chapters.length === 0, "show: still no chapter request at open");

  const first = page.locator(".episodes li").first();
  await first.locator(".chapters-chip").click();
  await page.waitForSelector(".chapters.inline .chapter-row", { timeout: 15000 });
  await page.waitForTimeout(300);
  check(chapterCount(st, seed.ready) === 1, `show: expanding asks 1 chapter GET (measured ${chapterCount(st, seed.ready)})`);
  check((await first.locator(".chapters.inline .chapter-row").count()) === 3, "show: the 3 chapters are listed");
  await first.locator(".chapters-chip").click();
  await first.locator(".chapters-chip").click();
  await page.waitForTimeout(400);
  check(chapterCount(st, seed.ready) === 1, `show: collapsing and reopening asks 0 more (measured ${chapterCount(st, seed.ready)})`);
  check((await first.locator(".chapters.inline .chapter-row").count()) === 3, "show: the 3 rows are still there");

  const second = page.locator(".episodes li").nth(1);
  await second.locator(".chapters-chip").click();
  await page.waitForFunction(
    () => document.querySelectorAll(".episodes li")[1]?.querySelectorAll(".chapters.inline .chapter-row").length === 8,
    null,
    { timeout: 15000 }
  );
  check(chapterCount(st, seed.chapters100) === 1, `show: the 100-chapter episode asks 1 GET (measured ${chapterCount(st, seed.chapters100)})`);
  check((await second.locator(".chapters.inline .chapter-row").count()) === 8, "show: the inline list caps at 8 chapters");
  await second.locator(".ch-see").click();
  await page.waitForSelector(".chsheet", { timeout: 15000 });
  // The sheet arrives with `veil` + `sheet` (transitions.md O23): on a desktop
  // the pane scales 0.97 → 1 over 200 ms, so a bounding box read mid-enter
  // measures the transform (44 × 0.97 = 42.68), not the layout. Wait for the
  // enter to settle, then measure the box the person ends up with.
  await page
    .waitForFunction(
      () =>
        !document
          .getAnimations()
          .some((animation) => animation.playState === "running" && animation.effect?.target?.closest?.(".chsheet")),
      null,
      { timeout: 5000 },
    )
    .catch(() => {});
  const mounted = await page.locator(".chsheet .chapter-row").count();
  check(mounted > 0 && mounted <= 33, `show: the sheet mounts at most 33 of the 100 chapters (measured ${mounted})`);
  const close = await page.locator('.chsheet button[aria-label="Fermer les chapitres"]').boundingBox();
  check(close && close.width >= 44 && close.height >= 44, `show: the sheet's close is 44 x 44 (${close && close.width}x${close && close.height})`);
  await page.locator('.chsheet button[aria-label="Fermer les chapitres"]').click();
  await page.waitForSelector(".chsheet", { state: "detached", timeout: 10000 });
  check(
    await page.evaluate(() => document.activeElement?.classList.contains("ch-see")),
    "show: closing the sheet returns the focus to the button that opened it"
  );

  await first.locator(".chapter-btn").nth(1).click();
  await page.waitForFunction(
    () => document.querySelector("footer.player .seek .time")?.textContent.trim() === "10:00",
    null,
    { timeout: 15000 }
  );
  await page.waitForSelector("footer.player .now", { timeout: 15000 });
  await openPlayer(page, false);
  await page.locator(".d .tabs button", { hasText: "À propos" }).click();
  await page.waitForSelector(".side .card-label", { timeout: 15000 });
  await page
    .waitForFunction(
      () => document.querySelector(".side .card-label")?.textContent.trim() === "Chapitre 2 sur 3",
      null,
      { timeout: 15000 }
    )
    .catch(() => {});
  note(
    "show: footer time after opening the panel",
    (await page.locator("footer.player .seek .time").first().textContent()).trim()
  );
  check(
    (await page.locator(".side .card-label").textContent()).trim() === "Chapitre 2 sur 3",
    `show: the chapter tap lands chapter 2 (measured ${(await page.locator(".side .card-label").textContent()).trim()})`
  );
  check(
    (await page.locator(".side .card-title").textContent()).trim() === "La maison",
    "show: the chapter card names La maison"
  );
  check(
    chapterCount(st, seed.ready) === 1,
    `show: the panel reuses the chapter cache (measured ${chapterCount(st, seed.ready)})`
  );
  await page.locator(".side .seg-tabs button", { hasText: "Transcription" }).click();
  await page.waitForSelector(".side .seg", { timeout: 15000 });
  await waitSeg(page, ".side", "10:00");
  check(true, "show: the transcript follows the chapter to 10:00");
  await page.locator('.d button[aria-label="Réduire"]').click();
  await ctx.close();
}

console.log("\n-- 390x844: the under panel, its tabs, the segment and its close");
{
  const ctx = await newContext({ width: 390, height: 844 }, true);
  await loginVia(ctx, seed.listeners[0].name); // the bench fixture password, not in the payload
  const page = await ctx.newPage();
  const st = track(page, seed.show);
  await openShow(page);
  await page.locator(".episodes li .pl").first().click();
  await openPlayer(page, true);
  check((await page.locator(".peek").count()) === 1, "390: the À propos door is present");
  await page.locator(".peek").click();
  await page.waitForSelector(".under .card-label", { timeout: 15000 });
  await page
    .waitForFunction(
      () => document.querySelector(".under .card-label")?.textContent.trim() === "Chapitre 1 sur 3",
      null,
      { timeout: 15000 }
    )
    .catch(() => {});
  check(chapterCount(st, seed.ready) === 1, `390: the first panel open asks 1 chapter GET (measured ${chapterCount(st, seed.ready)})`);
  check(
    (await page.locator(".under .card-label").textContent()).trim() === "Chapitre 1 sur 3",
    `390: the card says Chapitre 1 sur 3 (measured ${(await page.locator(".under .card-label").textContent()).trim()})`
  );
  const chapterHeights = await rowHeights(page, ".under .chapter-row");
  check(chapterHeights.length === 3 && chapterHeights.every((value) => Math.abs(value - 60) <= 1), `390: the chapter rows are 60 px (measured ${JSON.stringify(chapterHeights)})`);
  const tabs = await rowHeights(page, ".under .seg-tabs button");
  check(tabs.length === 2 && tabs.every((value) => value >= 43.5), `390: the Chapitres/Transcription tabs are 44 px (measured ${JSON.stringify(tabs.map((v) => Math.round(v)))})`);
  const close = await page.locator('.under button[aria-label="Fermer"]').boundingBox();
  check(close && close.width >= 44 && close.height >= 44, `390: the panel close is 44 x 44 (${close && close.width}x${close && close.height})`);
  await page.locator('.under .seg-tabs button', { hasText: "Transcription" }).click();
  await page.waitForSelector(".under .seg.on", { timeout: 15000 });
  check(
    (await page.locator(".under .seg[aria-current='true']").count()) === 1,
    "390: one transcript line is current"
  );
  note("390: current transcript line", (await page.locator(".under .seg.on time").textContent()).trim());
  const segHeights = await rowHeights(page, ".under .seg");
  check(segHeights.every((value) => value >= 43.5), "390: every transcript line is at least 44 px");
  const search = await page.locator(".under .searchbox").boundingBox();
  check(search && search.height >= 43.5, `390: the transcript search is 44 px (${search && search.height})`);
  const ov = await overflow(page);
  check(ov.doc <= ov.width + 1, `390: the under panel does not overflow (${ov.doc}/${ov.width})`);
  await page.locator('.under button[aria-label="Fermer"]').click();
  await page.waitForTimeout(300);
  check((await page.locator(".under").getAttribute("aria-hidden")) === "true", "390: closing the panel hides it");
  await page.locator('.m button[aria-label="Réduire"]').click();
  await ctx.close();
}

console.log("\n-- 390x844: a running transcript polls once per 10 s, not per segment");
{
  const ctx = await newContext({ width: 390, height: 844 }, true);
  await loginVia(ctx);
  const page = await ctx.newPage();
  const st = track(page, seed.show);
  await openShow(page);
  await page.locator(".episodes li").nth(2).locator(".pl").click();
  await openPlayer(page, true);
  await page.locator(".peek").click();
  await page.waitForFunction(
    () => /Transcription en cours — 42 %/.test(document.querySelector(".under")?.textContent || ""),
    null,
    { timeout: 15000 }
  );
  check(true, "390: the running transcript says 42 %");
  check(
    (await page.locator(".under .bar[role='progressbar']").getAttribute("aria-valuenow")) === "42",
    "390: the progress bar reports 42"
  );
  const stop = await page.locator(".under .btn", { hasText: "Arrêter" }).first().boundingBox();
  check(stop && stop.height >= 43.5, `390: Arrêter is 44 px (${stop && stop.height})`);
  const beforePoll = transcriptCount(st, seed.running);
  await page.waitForTimeout(10500);
  check(
    transcriptCount(st, seed.running) - beforePoll === 1,
    `390: 10 s of running transcript asks 1 poll (measured ${transcriptCount(st, seed.running) - beforePoll})`
  );
  const ov = await overflow(page);
  check(ov.doc <= ov.width + 1, `390: the running panel does not overflow (${ov.doc}/${ov.width})`);
  await page.locator('.under button[aria-label="Fermer"]').click();
  await page.locator('.m button[aria-label="Réduire"]').click();
  await ctx.close();
}

console.log("\n-- 641x900: the narrow desktop side, 300 px, 56 px rows");
{
  const ctx = await newContext({ width: 641, height: 900 }, false);
  await loginVia(ctx);
  const page = await ctx.newPage();
  const st = track(page, seed.show);
  await openShow(page);
  await page.locator(".episodes li .pl").first().click();
  await openPlayer(page, false);
  await page.locator(".d .tabs button", { hasText: "À propos" }).click();
  await page.waitForSelector(".side .card-label", { timeout: 15000 });
  const side = await page.locator(".side").boundingBox();
  check(side && side.width >= 299 && side.width <= 301, `641: the side column is 300 px (measured ${side && side.width.toFixed(1)})`);
  check(chapterCount(st, seed.ready) === 1, `641: the side panel asks 1 chapter GET (measured ${chapterCount(st, seed.ready)})`);
  const heights = await rowHeights(page, ".side .chapter-row");
  check(heights.length === 3 && heights.every((value) => Math.abs(value - 56) <= 1), `641: chapter rows are 56 px (measured ${JSON.stringify(heights)})`);
  const tabs = await rowHeights(page, ".side .seg-tabs button");
  check(tabs.length === 2 && tabs.every((value) => value >= 39.5), `641: the tabs are 40 px (measured ${JSON.stringify(tabs.map((v) => Math.round(v)))})`);
  const nav = await page.locator(".side .card-nav .nav").first().boundingBox();
  check(nav && nav.width >= 44 && nav.height >= 44, `641: chapter navigation is 44 x 44 (${nav && nav.width}x${nav && nav.height})`);
  const ov = await overflow(page);
  check(ov.doc <= ov.width + 1, `641: the full-screen player does not overflow (${ov.doc}/${ov.width})`);
  await page.locator('.d button[aria-label="Réduire"]').click();
  await ctx.close();
}

console.log("\n-- 1440x900: the show page stays whole at the chapter seams");
{
  const ctx = await newContext({ width: 1440, height: 900 }, false);
  await loginVia(ctx);
  const page = await ctx.newPage();
  const st = track(page, seed.show);
  await openShow(page);
  const chip = await page.locator(".episodes li .chapters-chip").first().boundingBox();
  check(chip && chip.height >= 31.5, `1440: the chapter chip keeps its desktop 32 px (measured ${chip && chip.height})`);
  const ov = await overflow(page);
  check(ov.doc <= ov.width + 1, `1440: the show page does not overflow after all interactions (${ov.doc}/${ov.width})`);
  check(st.show >= 1, "1440: the show was asked at least once");
  await ctx.close();
}

await browser.close();
server.stop();
console.log("");
if (state.failed) {
  console.log(`FAIL: ${state.failed} check(s) failed`);
  process.exit(1);
}
console.log("ok — show chapters, under/about panels, transcript current segment, search and the running poll");
