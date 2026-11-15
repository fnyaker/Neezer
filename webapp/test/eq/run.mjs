#!/usr/bin/env node
// The equalizer's saturation warning, on the screen it lives on:
//
//   npm run build && node test/eq/run.mjs
//
// A full-scale 1 kHz square for six seconds, then eight seconds of silence,
// plays through the processed path. What is pinned here:
//  - a boosted curve makes the warning appear, and it STAYS when the peak has
//    gone (it used to vanish at the next 40 ms reading);
//  - "Plat" takes it away, because a flat curve adds no energy;
//  - a flat curve with a loud, heavily normalized master never warns at all,
//    however hard the safety limiter works;
//  - switching the equalizer off hides it at once.
import { loadPlaywright, startServer, login, checker, installSessionPrefix } from "../harness.mjs";

const RATE = 48000;
const LOUD_SECONDS = 6;
const SILENT_SECONDS = 8;
const LEN = LOUD_SECONDS + SILENT_SECONDS;

// 1 kHz square at full scale for the loud part, digital silence after it.
// 16-bit PCM: Chromium plays an 8-bit file but will not route it through
// createMediaElementSource, and the meter lives behind that.
function signal() {
  const n = Math.round(RATE * LEN);
  const buf = Buffer.alloc(44 + 2 * n);
  buf.write("RIFF", 0);
  buf.writeUInt32LE(36 + 2 * n, 4);
  buf.write("WAVEfmt ", 8);
  buf.writeUInt32LE(16, 16);
  buf.writeUInt16LE(1, 20);
  buf.writeUInt16LE(1, 22);
  buf.writeUInt32LE(RATE, 24);
  buf.writeUInt32LE(RATE * 2, 28);
  buf.writeUInt16LE(2, 32);
  buf.writeUInt16LE(16, 34);
  buf.write("data", 36);
  buf.writeUInt32LE(2 * n, 40);
  const loud = Math.round(RATE * LOUD_SECONDS);
  for (let i = 0; i < n; i++) {
    const v = i < loud ? (Math.floor(i / 24) % 2 === 0 ? 32767 : -32767) : 0;
    buf.writeInt16LE(v, 44 + 2 * i);
  }
  return buf;
}

const wav = signal();
const { base, stop } = await startServer();
const { chromium } = await loadPlaywright();
const browser = await chromium.launch({ args: ["--autoplay-policy=no-user-gesture-required"] });
const { check, state } = checker();

const song = (id, gain = null) => ({
  deezer_id: id,
  title: "Song " + id,
  duration: LEN,
  gain,
  artist: { deezer_id: "a1", name: "Band" },
  album: { deezer_id: "al1", title: "Album", cover: "" },
});

async function open({ bands, bass = 0, normalize = "off", gain = null }) {
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  await ctx.route(/dzcdn\.net|api\.deezer\.com/, (r) => r.abort());
  await ctx.route(/\/api\/stream\//, (r) =>
    r.fulfill({ status: 200, body: wav, headers: { "Content-Type": "audio/wav", "Accept-Ranges": "none" } })
  );
  await ctx.addInitScript(installSessionPrefix);
  await ctx.addInitScript(() => {
    const j = sessionStorage.getItem("__inject");
    if (!j) return;
    localStorage.setItem(window.__nsPkey("player.session"), j);
    localStorage.removeItem(window.__nsPkey("player.pos"));
    sessionStorage.removeItem("__inject");
  });
  await ctx.addInitScript(
    ([bands, bass, normalize]) => {
      localStorage.setItem("audio.output", JSON.stringify("graph"));
      localStorage.setItem("fx.eq.enabled", JSON.stringify(true));
      localStorage.setItem("fx.eq.bands", JSON.stringify(bands));
      localStorage.setItem("fx.bass", JSON.stringify(bass));
      localStorage.setItem("fx.normalize", JSON.stringify(normalize));
    },
    [bands, bass, normalize]
  );
  const page = await ctx.newPage();
  page.on("pageerror", (e) => console.log("pageerror:", e.message));
  await login(page, base);
  await page.goto(base + "/app/#/");
  await page.evaluate(
    (j) => sessionStorage.setItem("__inject", j),
    JSON.stringify({
      queue: [song("s1", gain)],
      index: 0,
      start: 0,
      currentTime: 0,
      context: null,
      shuffle: false,
      repeat: "off",
      _orig: null,
      at: Date.now(),
    })
  );
  await page.reload();
  await page.waitForFunction(
    () => (document.querySelector("footer.player")?.innerText || "").includes("Song s1"),
    null,
    { timeout: 30000 }
  );
  await page.locator("footer.player .pp").first().click();
  await page.waitForFunction(
    () => /0:0[1-9]/.test(document.querySelector("footer.player")?.innerText || ""),
    null,
    { timeout: 30000 }
  );
  // Hash routing: the page (and the audio graph) stays alive.
  await page.evaluate(() => {
    location.hash = "#/settings/playback";
  });
  await page.locator(".eq-card").waitFor({ timeout: 30000 });
  // The meter only runs while its card is on screen (IntersectionObserver).
  await page.locator(".eq-card").scrollIntoViewIfNeeded();
  await page.waitForTimeout(300);
  return { ctx, page };
}

const warnText = (page) => page.locator(".eq-card .warn").innerText().catch(() => "");
const reductionOf = (text) => Number((text.match(/réduit de ([\d,]+) dB/) || [])[1]?.replace(",", ".")) || 0;
const clock = (page) =>
  page.evaluate(() => {
    const [mm, ss] = (document.querySelector("footer.player")?.innerText.match(/\d+:\d+/g)?.[0] || "0:00")
      .split(":")
      .map(Number);
    return mm * 60 + ss;
  });

// -- 1. a boosted curve: the warning appears, then outlives the peak ----------
{
  const bands = [16, 16, 16, 16, 16, 16, 16, 16, 16, 16];
  const { ctx, page } = await open({ bands });
  await page.locator(".eq-card .warn").waitFor({ state: "visible", timeout: 15000 });
  const peak = await warnText(page);
  check(/Saturation/.test(peak), "a +16 dB curve reports saturation", peak.split("\n")[0]);
  // Past the six loud seconds: only silence is left in the window.
  await page.waitForFunction(
    (floor) => {
      const [mm, ss] = (document.querySelector("footer.player")?.innerText.match(/\d+:\d+/g)?.[0] || "0:00")
        .split(":")
        .map(Number);
      return mm * 60 + ss >= floor;
    },
    LOUD_SECONDS + 1,
    { timeout: 20000 }
  );
  check((await clock(page)) >= LOUD_SECONDS + 1, "the loud part is over");
  const after = await warnText(page);
  check(after.length > 0, "the warning stays after the peak, it does not flash and go");
  check(
    reductionOf(after) >= reductionOf(peak),
    "and its numbers keep the highest value it saw (they only grow)",
    `${reductionOf(peak)} dB then ${reductionOf(after)} dB`
  );

  // A flat curve adds nothing: the warning goes away on the modification.
  await page.locator(".pitem", { hasText: "Plat" }).click();
  await page.locator(".eq-card .warn").waitFor({ state: "hidden", timeout: 3000 });
  check(true, "« Plat » takes the warning away");
  await ctx.close();
}

// -- 2. a flat EQ, a loud master: the limiter works, the warning stays off ----
{
  const { ctx, page } = await open({
    bands: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    normalize: "high",
    gain: -18,
  });
  // Normalization lifts this quiet master by 7.6 dB on a full-scale file: the
  // safety limiter is pulling, and that is the recording's level, not the EQ.
  await page.waitForTimeout(2500);
  const shown = await page.locator(".eq-card .warn").count();
  check(shown === 0, "a flat curve never warns, however hard the limiter works");
  await ctx.close();
}

// -- 3. the equalizer off: the warning is false and goes at once --------------
{
  const bands = [16, 16, 16, 16, 16, 16, 16, 16, 16, 16];
  const { ctx, page } = await open({ bands });
  await page.locator(".eq-card .warn").waitFor({ state: "visible", timeout: 15000 });
  await page.locator(".eq-card .toggle").click();
  await page.locator(".eq-card .warn").waitFor({ state: "hidden", timeout: 1000 });
  check(true, "switching the equalizer off hides the warning at once");
  await ctx.close();
}

await browser.close();
stop();
if (state.failed) {
  console.log(`\n${state.failed} check(s) failed`);
  process.exit(1);
}
console.log("\nall equalizer warning checks passed");
