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
import { loadPlaywright, startServer, login, checker, installSessionPrefix, deezerHost, pathOn } from "../harness.mjs";

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
const LONG_SECONDS = 30;
const LONG_LEN = LONG_SECONDS + SILENT_SECONDS;
function longSignal() {
  const n = Math.round(RATE * LONG_LEN);
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
  const loud = Math.round(RATE * LONG_SECONDS);
  for (let i = 0; i < n; i++) {
    const v = i < loud ? Math.round(32767 * 0.99 * Math.sin((2 * Math.PI * 60 * i) / RATE)) : 0;
    buf.writeInt16LE(v, 44 + 2 * i);
  }
  return buf;
}
const wavLong = longSignal();
const { base, stop } = await startServer();
const { chromium } = await loadPlaywright();
const browser = await chromium.launch({ args: ["--autoplay-policy=no-user-gesture-required"] });
const { check, state } = checker();

const song = (id, gain = null, duration = LEN) => ({
  deezer_id: id,
  title: "Song " + id,
  duration,
  gain,
  artist: { deezer_id: "a1", name: "Band" },
  album: { deezer_id: "al1", title: "Album", cover: "" },
});

async function open({ bands, bass = 0, normalize = "off", gain = null, tracks = 1, body = wav, duration = LEN }) {
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  await ctx.route(deezerHost, (r) => r.abort());
  await ctx.route(pathOn("/api/stream/"), (r) =>
    r.fulfill({ status: 200, body, headers: { "Content-Type": "audio/wav", "Accept-Ranges": "none" } })
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
      queue: Array.from({ length: tracks }, (_, i) => song("s" + (i + 1), gain, duration)),
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
const cutOf = (text) => Number((text.match(/Réduire le volume de ([\d,]+) dB/) || [])[1]?.replace(",", ".")) || 0;
const meterOf = (page) =>
  page.evaluate(() => {
    const t = document.querySelector(".eq-card .meter-read")?.innerText || "";
    const n = (re) => Number((t.match(re) || [])[1]?.replace(",", ".").replace("\u2212", "-")) || 0;
    return {
      peak: n(/Pic\s+([\d,.\u2212-]+) dB/),
      reduction: n(/Limiteur\s+([\d,.\u2212-]+) dB/),
    };
  });
async function bandPoint(page, i, db) {
  return page.evaluate(
    ([i, db]) => {
      const f = document.querySelectorAll(".eq-card .eq .fader")[i];
      const r = f.getBoundingClientRect();
      const p = (db + 16) / 32;
      return { x: r.left + r.width / 2, y: r.bottom - 11 - p * (r.height - 22) };
    },
    [i, db]
  );
}
async function clickBand(page, i, db) {
  await page.locator(".eq-card").scrollIntoViewIfNeeded();
  const pt = await bandPoint(page, i, db);
  await page.mouse.click(pt.x, pt.y);
}
async function setVolume(page, v) {
  await page.evaluate((v) => {
    const el = document.querySelector(".vol-input");
    el.value = String(v);
    el.dispatchEvent(new Event("input", { bubbles: true }));
  }, v);
}
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
  check(!/de plus/.test(peak), "a setting loaded as-is is reported without a baseline claim");
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

{
  const { ctx, page } = await open({
    bands: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    normalize: "high",
    gain: -18,
    tracks: 2,
    body: wavLong,
    duration: LONG_LEN,
  });
  await page.locator(".eq-card").scrollIntoViewIfNeeded();
  await page.locator("footer.player .next").click();
  await page.waitForFunction(
    () => (document.querySelector("footer.player")?.innerText || "").includes("Song s2"),
    null,
    { timeout: 30000 }
  );
  await page.waitForFunction(
    () => {
      const t = document.querySelector(".eq-card .meter-read")?.innerText || "";
      return /Pic\s+[5-9][\d,.]* dB/.test(t);
    },
    null,
    { timeout: 20000 }
  );
  const hot = await meterOf(page);
  check(hot.peak >= 5, "a hot master drives the pre-limiter peak past +5 dBFS", `pic ${hot.peak} dBFS`);
  check((await page.locator(".eq-card .warn").count()) === 0, "a flat curve never warns, however hot the master");

  await clickBand(page, 1, 1);
  await page.waitForTimeout(1200);
  const one = await meterOf(page);
  check((await page.locator(".eq-card .warn").count()) === 0, "a +1 dB band on a hot master does not warn");
  check(one.reduction >= 5, "the raw limiter reduction is still measured and shown", `limiteur ${one.reduction} dB`);

  await clickBand(page, 1, 9);
  await page.waitForTimeout(1200);
  await page.locator(".eq-card .warn").waitFor({ state: "visible", timeout: 5000 });
  const nine = await warnText(page);
  const net = reductionOf(nine);
  const raw = (await meterOf(page)).reduction;
  check(raw >= 10, "the +9 dB curve drives the limiter hard", `limiteur mesuré ${raw} dB`);
  check(net >= 4 && net <= 8, "the banner reports the band's share, not the raw reduction", `${net} dB of ${raw} dB`);
  check(/de plus depuis votre réglage/.test(nine), "and says it is the share above the setting's baseline");
  check(cutOf(nine) <= 8, "and the button offers that measured share", `${cutOf(nine)} dB`);

  await page.locator(".eq-card .warn-btn").click();
  await page.waitForTimeout(900);
  const pct = await page.evaluate(
    () => document.querySelector("div.vol").parentElement.querySelector(".val").textContent
  );
  const cut = cutOf(nine);
  check(
    Math.abs(parseFloat(pct) - 100 * Math.pow(10, -cut / 20)) <= 1,
    "the button really lowers the volume by the measured share",
    `${pct} for a ${cut} dB cut`
  );
  await page.locator(".eq-card .warn").waitFor({ state: "hidden", timeout: 5000 });
  check(true, "and the banner goes: the new episode starts from the quieter chain");

  await setVolume(page, 0.14);
  await page.waitForTimeout(900);
  check((await meterOf(page)).reduction === 0, "at 14 % the limiter no longer reduces anything");
  check((await page.locator(".eq-card .warn").count()) === 0, "so there is nothing left to warn about");
  await ctx.close();
}

{
  const bands = [6, 5, 4, 2, 0, 0, 0, -1, -2, -3];
  const { ctx, page } = await open({ bands });
  const geometry = () =>
    page.evaluate(() => {
      const box = (el) => {
        const b = el.getBoundingClientRect();
        return { x: b.x, y: b.y, w: b.width, h: b.height, cx: b.x + b.width / 2, cy: b.y + b.height / 2 };
      };
      const bands = JSON.parse(localStorage.getItem("fx.eq.bands") || "[]");
      const rows = [...document.querySelectorAll(".eq-card .eq .band")].map((band, i) => {
        const fader = box(band.querySelector(".fader"));
        const track = box(band.querySelector(".fader-track"));
        const thumb = box(band.querySelector(".fader-thumb"));
        const p = (bands[i] + 16) / 32;
        return {
          i,
          fader,
          track,
          thumb,
          axis: Math.abs(thumb.cx - track.cx),
          value: Math.abs(thumb.cy - (track.y + (1 - p) * track.h)),
          thinner: track.w < thumb.w,
        };
      });
      const volTrack = box(document.querySelector(".vol-track"));
      const volThumb = box(document.querySelector(".vol-thumb"));
      const volInput = document.querySelector(".vol-input").value;
      return {
        rows,
        vol: {
          axis: Math.abs(volThumb.cy - volTrack.cy),
          value: Math.abs(volThumb.cx - (volTrack.x + Number(volInput) * volTrack.w)),
          hit: box(document.querySelector(".vol")),
        },
        scrollW: document.documentElement.scrollWidth,
        innerW: window.innerWidth,
        vw: window.innerWidth,
      };
    });
  const report = async (label) => {
    const g = await geometry();
    const worstAxis = Math.max(...g.rows.map((r) => r.axis));
    const worstValue = Math.max(...g.rows.map((r) => r.value));
    const minHit = Math.min(...g.rows.map((r) => Math.min(r.fader.w, r.fader.h)));
    const thinner = g.rows.every((r) => r.thinner);
    console.log(
      `    ${label}: thumb axis off by ${worstAxis.toFixed(2)} px, value off by ${worstValue.toFixed(2)} px, ` +
        `fader ${g.rows[0].fader.w}×${g.rows[0].fader.h} px, volume hit ${g.vol.hit.w.toFixed(0)}×${g.vol.hit.h} px, ` +
        `scroll ${g.scrollW}/${g.innerW}`
    );
    check(worstAxis <= 0.6, `${label}: every thumb is centred on its track`, `${worstAxis.toFixed(2)} px`);
    check(worstValue <= 0.8, `${label}: every thumb sits on its band's value`, `${worstValue.toFixed(2)} px`);
    check(minHit >= 44, `${label}: every fader is a 44 px target`, `${minHit.toFixed(0)} px`);
    check(thinner, `${label}: the track is thinner than the thumb`);
    check(g.vol.axis <= 0.6 && g.vol.value <= 0.8, `${label}: the volume knob is centred and on its value`);
    check(g.vol.hit.h >= 44, `${label}: the volume row is a 44 px target`, `${g.vol.hit.h} px`);
    check(g.scrollW <= g.innerW, `${label}: no horizontal overflow`, `${g.scrollW}/${g.innerW}`);
  };
  await page.locator(".eq-card").scrollIntoViewIfNeeded();
  await setVolume(page, 0.72);
  await page.waitForTimeout(400);
  await report("1280");
  await page.setViewportSize({ width: 390, height: 844 });
  await page.locator(".eq-card").scrollIntoViewIfNeeded();
  await page.waitForTimeout(400);
  await report("390");
  await page.locator("div.vol").scrollIntoViewIfNeeded();
  const volBox = await page.locator("div.vol").boundingBox();
  await page.mouse.click(volBox.x + 11 + 0.14 * (volBox.width - 22), volBox.y + volBox.height / 2);
  const volPct = await page.evaluate(
    () => document.querySelector("div.vol").parentElement.querySelector(".val").textContent
  );
  check(volPct === "14%", "clicking the volume track sets that level", volPct);
  await ctx.close();
}

await browser.close();
stop();
if (state.failed) {
  console.log(`\n${state.failed} check(s) failed`);
  process.exit(1);
}
console.log("\nall equalizer warning checks passed");
