#!/usr/bin/env node
// Sharing a TRACK as a public link, on the real SPA and the real server:
//   · the share sheet opens on "Lien" and asks the server once (no waveform
//     request, no gain request, no provider);
//   · "Partager le lien" creates the capability and hands its URL to the Web
//     Share API (stubbed here), never a file;
//   · a guest with no session opens the page: one HTML response, one cover,
//     ZERO audio bytes until play; then the stream is audio/ogg (Opus);
//   · the page carries the Open Graph / oEmbed / twitter:player tags, an
//     embeddable compact mode, and no sideways overflow at 390 px;
//   · Réglages → Compte lists the link and revoking it kills the page on the
//     very next request.
//
//   npm run build && node test/share/run.mjs      (SHOTS=<dir> keeps pictures)
//
// Runs tools/perf_api.py --serve with a real WAV archived as the first twelve
// tracks and a real cover.jpg on every album (Deezer off). The link targets the
// first of those twelve, whose exact title/artist the tool puts in `ids.share`.
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";
import { loadPlaywright, startServer, silence, login, checker } from "../harness.mjs";

const { chromium } = await loadPlaywright();
const SHOTS = process.env.SHOTS || null;
mkdirSync("/tmp/opencode", { recursive: true });
const tmp = mkdtempSync("/tmp/opencode/share-bench-");
const WAV = join(tmp, "tone.wav");
// The track has to be a real file: the public stream transcodes it to Opus.
writeFileSync(WAV, silence(6));
const COVER = resolve(dirname(fileURLToPath(import.meta.url)), "../../../tests/assets/cover.jpg");
const cleanup = () => rmSync(tmp, { recursive: true, force: true });
process.on("exit", cleanup);

const { base, ids, stop } = await startServer([
  "--audio",
  WAV,
  "--covers",
  COVER,
]);
const { check, state } = checker();
const target = ids.share;
const duration = `${Math.floor(target.duration / 60)}:${String(target.duration % 60).padStart(2, "0")}`;

const browser = await chromium.launch({ args: ["--autoplay-policy=no-user-gesture-required"] });
try {
  // -- the owner's share sheet ------------------------------------------------
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  await ctx.route(/dzcdn\.net|api\.deezer\.com|scdn\.co/, (r) => r.abort());
  // Headless Chromium has no OS share panel: capture what the app hands it.
  await ctx.addInitScript(() => {
    try {
      Object.defineProperty(navigator, "share", {
        configurable: true,
        value: async (data) => {
          window.__nsShared = data;
        },
      });
    } catch {
      /* no navigator.share: the app falls back to the clipboard */
    }
  });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => console.log("pageerror:", e.message));

  const api = [];
  page.on("request", (r) => {
    const u = r.url();
    if (u.startsWith(base) && /\/api\//.test(u)) api.push({ url: u.slice(base.length), method: r.method() });
  });

  await login(page, base);
  await page.reload();
  await page.waitForTimeout(1400);

  // Find the exact title the tool archived, in the real search screen.
  await page.goto(base + "/app/#/search");
  await page.locator(".bar input").fill(target.title);
  await page.waitForTimeout(1400);
  await page.waitForSelector(".row.track", { timeout: 30000 });
  let rows = page.locator(".row.track").filter({ hasText: target.title }).filter({ hasText: target.artist });
  if ((await rows.count()) > 1) rows = rows.filter({ hasText: duration });
  const row = rows.first();
  check((await row.count()) === 1, `the archived track is findable by its title, artist and duration (${duration})`);
  const box = await row.boundingBox();
  await page.mouse.click(box.x + 24, box.y + 20, { button: "right" });
  await page.waitForSelector(".menu[role=menu]", { timeout: 5000 });
  await page.locator(".menu[role=menu] [role=menuitem]", { hasText: "Partager…" }).first().click();
  await page.waitForSelector(".sheet[role=dialog]", { timeout: 5000 });
  await page.waitForSelector(".linkpane", { timeout: 5000 });

  check(
    (await page.locator(".modes button.on", { hasText: "Lien" }).count()) === 1,
    "the sheet opens on the Lien tab, not the waveform"
  );
  check((await page.locator(".wave-wrap").count()) === 0, "the excerpt waveform is not even mounted");
  const shareGets = api.filter((r) => r.method === "GET" && /^\/api\/track\/[^/]+\/share$/.test(r.url));
  check(shareGets.length === 1, "opening the sheet asks the server ONCE for the live link", `${shareGets.length} GET`);
  const waveCalls = api.filter((r) => r.url.startsWith("/api/share/waveform/"));
  check(waveCalls.length === 0, "and asks for NO waveform (the ffmpeg decode is deferred)", `${waveCalls.length} waveform`);
  const gainCalls = api.filter((r) => /\/gain$/.test(r.url));
  check(gainCalls.length === 0, "and asks for NO ReplayGain (the link preview plays nothing)", `${gainCalls.length} gain`);

  const beforePost = api.length;
  await page.locator(".sheet footer .primary", { hasText: "Partager le lien" }).click();
  await page.waitForFunction(() => window.__nsShared && window.__nsShared.url, { timeout: 30000 });
  const shared = await page.evaluate(() => window.__nsShared);
  const post = api.slice(beforePost).filter((r) => r.method === "POST" && /^\/api\/track\/[^/]+\/share$/.test(r.url));
  check(post.length === 1, "the button creates the capability in ONE POST", `${post.length} POST`);
  check(
    /^http:\/\/127\.0\.0\.1:\d+\/t\/[A-Za-z0-9_-]{16,24}\.[A-Za-z0-9_-]{24}$/.test(shared.url),
    "the shared value is a public /t/<id>.<mac> URL, not a file",
    shared.url
  );
  check(!shared.files, "no File is handed to the share sheet");
  const shown = await page.locator(".linkbox input").inputValue();
  check(shown === shared.url, "the sheet shows the same link it shared");
  if (SHOTS) await page.screenshot({ path: join(SHOTS, "share-sheet-link.png") });

  // -- a guest with no session -------------------------------------------------
  const guestCtx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const guestApi = [];
  const guestPage = await guestCtx.newPage();
  guestPage.on("pageerror", (e) => console.log("guest pageerror:", e.message));
  guestPage.on("request", (r) => {
    const u = r.url();
    if (u.startsWith(base) && /\/api\//.test(u)) guestApi.push({ url: u.slice(base.length), method: r.method() });
  });
  const resp = await guestPage.goto(shared.url, { waitUntil: "load" });
  check(resp.status() === 200, "the guest page answers 200", `${resp.status()}`);
  check(resp.headers()["cache-control"] === "no-store", "and is never cached (revocation is immediate)");
  check(resp.headers()["x-robots-tag"] === "noindex, nofollow", "and stays out of crawlers");
  check(resp.headers()["referrer-policy"] === "no-referrer", "and leaks no referrer");
  const raw = (await resp.body()).length;
  const packed = gzipSync(await resp.body()).length;
  check(raw <= 14_000, `the HTML is small: ${raw} B raw`, `${raw} B`);
  check(packed <= 4_500, `and ${packed} B gzipped`, `${packed} B gzip`);
  const streamBefore = guestApi.filter((r) => r.url.endsWith("/stream"));
  check(streamBefore.length === 0, "no audio byte is fetched before the play button", `${streamBefore.length} stream`);
  const preview = await guestPage.evaluate(() => {
    const meta = (p) => document.querySelector(`meta[property='${p}']`)?.content || "";
    const name = (n) => document.querySelector(`meta[name='${n}']`)?.content || "";
    const a = document.querySelector("audio");
    return {
      title: document.title,
      ogType: meta("og:type"),
      ogTitle: meta("og:title"),
      ogImage: meta("og:image"),
      ogAudio: meta("og:audio"),
      twitterPlayer: name("twitter:player"),
      oembed: document.querySelector("link[rel=alternate]")?.href || "",
      preload: a?.preload,
      src: a?.getAttribute("src"),
      dataSrc: a?.getAttribute("data-src"),
      overflow: document.documentElement.scrollWidth - window.innerWidth,
    };
  });
  check(preview.title.includes(target.title), "the page names the title", preview.title);
  check(preview.ogType === "music.song", "Open Graph says music.song", preview.ogType);
  check(preview.ogTitle.includes(target.title), "og:title carries the title", preview.ogTitle);
  check(preview.ogImage.startsWith(base + "/api/shared/track/"), "og:image is the capability cover", preview.ogImage);
  check(preview.ogAudio.endsWith("/stream"), "og:audio points at the Opus stream", preview.ogAudio);
  check(preview.twitterPlayer.includes("embed=1"), "twitter:player points at the compact player", preview.twitterPlayer);
  check(preview.oembed.includes("/api/oembed?url="), "the page discovers its oEmbed endpoint", preview.oembed);
  check(preview.preload === "none" && !preview.src && !!preview.dataSrc, "the audio element is lazy: no src until play");
  check(preview.overflow <= 0, "no horizontal overflow on the page", `${preview.overflow}px`);
  if (SHOTS) await guestPage.screenshot({ path: join(SHOTS, "share-page-desktop.png"), fullPage: true });

  const streamResponses = [];
  guestPage.on("response", (r) => {
    if (r.url().endsWith("/stream")) streamResponses.push(r);
  });
  await guestPage.locator("#play").click();
  await guestPage.waitForFunction(
    () => {
      const a = document.querySelector("#a");
      return a && a.currentTime > 0.2;
    },
    { timeout: 30000 }
  );
  const playing = await guestPage.evaluate(() => {
    const a = document.querySelector("#a");
    return { paused: a.paused, src: a.currentSrc, time: a.currentTime };
  });
  check(!playing.paused && playing.time > 0.2, "the guest player really plays", `${playing.time.toFixed(2)} s`);
  check(playing.src.endsWith("/stream"), "and its source is the capability stream", playing.src);
  check(streamResponses.length >= 1, "the audio request happened after the click", `${streamResponses.length} stream`);
  const streamType = streamResponses[0]?.headers()["content-type"] || "";
  check(streamType.startsWith("audio/ogg"), "the stream is Opus (audio/ogg), never the FLAC", streamType);

  // The compact player an oEmbed consumer embeds.
  const embed = await guestCtx.newPage();
  await embed.goto(shared.url + "?embed=1", { waitUntil: "load" });
  const embedState = await embed.evaluate(() => ({
    cls: document.body.className,
    width: document.querySelector(".card").getBoundingClientRect().width,
    overflow: document.documentElement.scrollWidth - window.innerWidth,
  }));
  check(embedState.cls.includes("embed"), "?embed=1 renders the compact player");
  check(embedState.width <= 680, "the compact card fits an iframe", `${Math.round(embedState.width)}px`);
  check(embedState.overflow <= 0, "and does not scroll sideways", `${embedState.overflow}px`);
  if (SHOTS) await embed.screenshot({ path: join(SHOTS, "share-page-embed.png") });

  const oe = await guestCtx.request.get(
    base + "/api/oembed?url=" + encodeURIComponent(shared.url)
  );
  const oembed = await oe.json();
  check(oe.status() === 200, "the oEmbed endpoint answers", `${oe.status()}`);
  check(oembed.type === "rich" && oembed.html.includes("<iframe"), "with a rich iframe embed", oembed.type);
  check(oembed.width === 480 && oembed.height === 180, "and its own dimensions");
  check((oembed.thumbnail_url || "").includes("/cover?w=640"), "and the 640 px cover", oembed.thumbnail_url);

  // Phone: the same page, one column, no sideways scroll.
  const phoneCtx = await browser.newContext({
    viewport: { width: 390, height: 844 },
    deviceScaleFactor: 2,
    isMobile: true,
    hasTouch: true,
  });
  const phone = await phoneCtx.newPage();
  await phone.goto(shared.url, { waitUntil: "load" });
  const phoneOverflow = await phone.evaluate(
    () => document.documentElement.scrollWidth - window.innerWidth
  );
  check(phoneOverflow <= 0, "390 px: no horizontal overflow", `${phoneOverflow}px`);
  if (SHOTS) await phone.screenshot({ path: join(SHOTS, "share-page-phone.png"), fullPage: true });

  // -- Réglages → Compte: list and revoke -------------------------------------
  await page.keyboard.press("Escape");
  await page.waitForSelector(".sheet[role=dialog]", { state: "detached", timeout: 5000 });
  await page.goto(base + "/app/#/settings/account");
  await page.waitForSelector("h2:has-text('Liens partagés')", { timeout: 20000 });
  await page.waitForSelector(".links li", { timeout: 20000 });
  const listed = await page.locator(".links li", { hasText: target.title }).count();
  check(listed === 1, "Réglages lists the link with its track", `${listed} row`);
  await page.locator(".links li", { hasText: target.title }).locator(".icon.cut").click();
  await page.locator(".links li", { hasText: target.title }).locator(".danger", { hasText: "Révoquer" }).click();
  await page.waitForFunction(
    (title) => ![...document.querySelectorAll(".links li")].some((li) => li.textContent.includes(title)),
    target.title,
    { timeout: 10000 }
  );
  const dead = await guestPage.goto(shared.url, { waitUntil: "load" });
  check(dead.status() === 404, "after the revocation the public page is a 404", `${dead.status()}`);
  check(
    (await guestPage.locator("body").innerText()).includes("Ce lien n’est plus valide."),
    "and says it calmly, with no server detail"
  );

  await Promise.allSettled([ctx.close(), guestCtx.close(), phoneCtx.close()]);
} finally {
  await browser.close();
  await stop();
  cleanup();
}

console.log(state.failed ? `\nFAIL: ${state.failed} check(s) failed` : "\nshare: all checks passed");
process.exit(state.failed ? 1 : 0);
