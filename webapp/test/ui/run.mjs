#!/usr/bin/env node
// The UI bench: what the lists cost the main thread, in a real browser, on a
// library the size real ones reach.
//
//   node test/ui/run.mjs                 # 4x CPU throttle (a mid-range phone), 5 rounds
//   THROTTLE=1 N=9 node test/ui/run.mjs  # desktop speed, more rounds
//
// It starts tools/perf_api.py --serve (8 000 tracks, a 4 000-track playlist,
// 4 000 favourites, Deezer off) against the BUILT SPA (npm run build first),
// logs in in headless Chromium, and repeats what a person does: open the
// favourites, type a search into them letter by letter, change the sort, open
// the big playlist. Each is measured as the main thread's LONG-TASK time
// (PerformanceObserver "longtask": anything over 50 ms that blocked input),
// the median of the rounds — single runs vary by 2x under SwiftShader, which
// is why there are rounds. Cover art is blocked (the sandbox has no CDN).
//
// Measured when row recycling and the text-only offline cache went in
// (median ms, 4x throttle): favourites 315 -> 154, typing ten letters
// 470 -> 55, a sort 97 -> 52, the playlist 304 -> 216.
//
// The second half is the phone player's LAYOUT contract (`SHIFT_ONLY=1` runs
// just it): a synced lyric line appears, goes away and wraps to two or three
// rows on its own while the song plays, and none of it may move the screen.
// It walks five lines — an empty one and a three-row one included — at
// 412x915 and 360x640, and asserts the summed layout-shift of the page is 0
// and that the cover carousel's box and the transport do not move a pixel.
import { spawn } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { existsSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import { execSync } from "node:child_process";
import { stopServer, silence, login, checker, deezerHost, pathOn } from "../harness.mjs";

async function loadPlaywright() {
  for (const spec of ["playwright", "playwright-core"]) {
    try {
      return await import(spec);
    } catch {
      /* try the next */
    }
  }
  const root = execSync("npm root -g", { stdio: ["ignore", "pipe", "ignore"] }).toString().trim();
  for (const name of ["playwright", "playwright-core"]) {
    const p = join(root, name, "index.mjs");
    if (existsSync(p)) return import(pathToFileURL(p).href);
  }
  throw new Error("Playwright is not installed (npm i -g playwright).");
}
const { chromium } = await loadPlaywright();
const THROTTLE = +(process.env.THROTTLE || 4);
const N = +(process.env.N || 5);
const SHIFT_ONLY = !!process.env.SHIFT_ONLY;
const repo = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const py = existsSync(join(repo, ".venv/bin/python")) ? join(repo, ".venv/bin/python") : "python3";
const srv = spawn(py, [join(repo, "tools/perf_api.py"), "--runs", "1", "--serve"], { stdio: ["pipe", "pipe", "ignore"] });
const ready = await new Promise((res, rej) => {
  let buf = "";
  srv.stdout.on("data", (d) => { buf += d; const m = buf.match(/READY (\d+) (.*)\n/); if (m) res({ port: +m[1], ids: JSON.parse(m[2]) }); });
  srv.on("exit", () => rej(new Error("server exited")));
});
const base = `http://127.0.0.1:${ready.port}`;
const browser = await chromium.launch({ args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--autoplay-policy=no-user-gesture-required"] });
const ctx = await browser.newContext({ viewport: { width: 412, height: 915 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true });
await ctx.route(/dzcdn\.net|api\.deezer\.com/, (r) => r.abort());
await ctx.addInitScript(() => {
  window.__long = [];
  try { new PerformanceObserver((l) => l.getEntries().forEach((e) => window.__long.push({ at: e.startTime, d: e.duration }))).observe({ entryTypes: ["longtask"] }); } catch {}
});
const page = await ctx.newPage();
const cdp = await ctx.newCDPSession(page);
await page.goto(base + "/app/");
await page.evaluate(() => fetch("/api/login", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ username: "bench", password: "Bench1" }) }));
await page.reload();
await page.waitForTimeout(2500);
await cdp.send("Emulation.setCPUThrottlingRate", { rate: THROTTLE });
const now = () => page.evaluate(() => performance.now());
const longSum = async (t0) => page.evaluate((t0) => window.__long.filter((e) => e.at >= t0).reduce((a, e) => a + e.d, 0), t0);
const has4000 = () => page.waitForFunction(() => document.body.innerText.includes("4000 titres"), null, { timeout: 60000, polling: 100 });
const med = (xs) => { const s = [...xs].sort((a, b) => a - b); return s[s.length >> 1]; };
const res = { fav: [], type: [], sort: [], pl: [] };
for (let k = 0; k < (SHIFT_ONLY ? 0 : N); k++) {
  await page.goto(base + "/app/#/");
  await page.waitForTimeout(600);
  let p0 = await now();
  await page.goto(base + "/app/#/library");
  await has4000();
  await page.waitForTimeout(700);
  res.fav.push(await longSum(p0));
  const box = page.locator('input[placeholder="Rechercher dans cette liste…"]').first();
  p0 = await now();
  for (const ch of "amour nuit") await box.type(ch, { delay: 40 });
  await page.waitForTimeout(500);
  res.type.push(await longSum(p0));
  await box.fill("");
  await page.waitForTimeout(400);
  p0 = await now();
  await page.selectOption("select.sortsel", k % 2 ? "album" : "title");
  await page.waitForTimeout(500);
  res.sort.push(await longSum(p0));
  await page.selectOption("select.sortsel", "default");
  p0 = await now();
  await page.goto(base + "/app/#/playlist/" + ready.ids.big);
  await has4000();
  await page.waitForTimeout(700);
  res.pl.push(await longSum(p0));
}
if (!SHIFT_ONLY) if (!SHIFT_ONLY) for (const [k, v] of Object.entries(res)) console.log(`${k.padEnd(6)} median long-task ms ${med(v).toFixed(0).padStart(5)}   runs ${v.map((x) => x.toFixed(0)).join(" ")}`);

// ── a line that appears on its own never moves the screen ────────────────────
// The owner's rule (2026-11-27): what shows up, goes away or grows on its own
// (not because the person tapped) must not move the rest of the screen. The
// synced lyric line above the sleeve is the worst case: it appears, empties,
// and wraps to two or three rows while the song plays. This drives five of
// them — an EMPTY one and a THREE-ROW one included — at both phone sizes and
// sums Chromium's `layout-shift` entries (CLS) from the moment the player is
// open and the first line is drawn. It also reads the boxes the eye follows:
// the cover carousel and the transport must stay exactly where they were.
// Lines in MILLISECONDS, as `/api/lyrics` answers them (an LRC `[00:01.60]`
// is `time: 1600`); an EMPTY line (the box, not its text, must hold) and one
// long enough to wrap to three rows at 360 px.
const LYRICS = [
  "On a laissé la ville derrière nous",
  "Les néons s'éteignent un par un",
  "",
  "Et le silence reprend la rue, les ombres dansent sur les murs et le vent nous parle à voix basse pendant que les étoiles tombent dans la mer",
  "Tu dis que demain ne ressemble à rien",
  "On marche sans savoir où l'on va",
  "Il reste un peu de lumière au bout",
].map((text, i) => ({ time: i * 1600, text }));
const WAV = silence(60);
const { check, state } = checker();
const round = (n) => Math.round(n * 100) / 100;

for (const [w, h] of [[412, 915], [360, 640]]) {
  const c = await browser.newContext({ viewport: { width: w, height: h }, deviceScaleFactor: 2, isMobile: true, hasTouch: true });
  await c.route(deezerHost, (r) => r.abort());
  await c.route(pathOn("/api/stream/"), (r) =>
    r.fulfill({ status: 200, body: WAV, headers: { "Content-Type": "audio/wav", "Accept-Ranges": "none" } })
  );
  await c.route(pathOn("/api/lyrics/"), (r) =>
    r.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ lyrics: { synced: LYRICS } }) })
  );
  await c.addInitScript(() => {
    window.__shifts = [];
    window.__shiftFrom = 0;
    try {
      new PerformanceObserver((list) =>
        list.getEntries().forEach((e) => {
          // hadRecentInput: a shift a tap caused is the person's move, not ours.
          if (!e.hadRecentInput) window.__shifts.push({ at: e.startTime, value: e.value, sources: (e.sources || []).map((s) => s.node?.className || s.node?.nodeName || "") });
        })
      ).observe({ type: "layout-shift", buffered: true });
    } catch {
      /* an engine without layout-shift: the rect check below still holds */
    }
  });
  const p = await c.newPage();
  await login(p, base);
  await p.reload();
  await p.waitForTimeout(1500);
  await p.goto(base + "/app/#/playlist/" + ready.ids.big);
  await p.waitForFunction(() => document.body.innerText.includes("4000 titres"), null, { timeout: 60000 });
  // A notice hangs over the page instead of entering its flow: dismissing one
  // the perf server always raises (its Deezer proxy is off) must not move the
  // page at all. In flow it used to drop the whole screen by the card's
  // height the moment it landed.
  const pageTop = () => p.evaluate(() => Math.round((document.querySelector(".route-host")?.getBoundingClientRect().top || 0) * 100) / 100);
  const notices = await p.locator(".notices .notice").count();
  const withNotice = await pageTop();
  for (let i = 0; i < notices; i++) await p.locator('.notices .notice button[aria-label="Masquer"]').first().click();
  await p.waitForTimeout(500);
  const withoutNotice = await pageTop();
  check(notices > 0, `${w}x${h}: the bench has a notice to dismiss`, `${notices}`);
  check(withNotice === withoutNotice, `${w}x${h}: a notice floats, it does not push the page`, `${withNotice} -> ${withoutNotice}`);
  await p.locator('button.play[aria-label="Lire"]').first().click();
  await p.waitForTimeout(1000);
  await p.locator('button[aria-label="Ouvrir le lecteur plein écran"]').first().click();
  await p.waitForSelector(".m .cur-lyric", { timeout: 15000 });
  // Let the sheet finish entering (it `screen`-slides in): a box that is still
  // moving belongs to the opening, not to the lyric line.
  const stable = async () => {
    let last = null;
    for (let i = 0; i < 30; i++) {
      const now = await p.evaluate(() => Math.round(document.querySelector(".m .stagebox").getBoundingClientRect().top * 10) / 10);
      if (now === last) return now;
      last = now;
      await p.waitForTimeout(120);
    }
    return last;
  };
  await stable();
  // From the top, through the player's own seek rail (the range the finger
  // drags), so the whole LYRICS sequence is walked even if the player took a
  // few seconds to mount.
  await p.evaluate(() => {
    const input = document.querySelector('.m .seek input[type="range"]');
    if (!input) return;
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
    setValue.call(input, "0");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  const boxes = () =>
    p.evaluate(() => {
      const r = (sel) => {
        const b = document.querySelector(sel)?.getBoundingClientRect();
        return b ? { top: Math.round(b.top * 100) / 100, height: Math.round(b.height * 100) / 100 } : null;
      };
      return { stage: r(".m .stagebox"), controls: r(".m .controls"), lyric: r(".m .cur-lyric"), footer: r(".m .footer") };
    });
  // Wait for a line: by its text, or (the empty line) by the span going away.
  const wait = (text) =>
    text
      ? p.waitForFunction((t) => (document.querySelector(".m .cur-lyric span")?.textContent || "").trim() === t, text, { timeout: 30000 })
      : p.waitForFunction(() => !document.querySelector(".m .cur-lyric span"), null, { timeout: 30000 });

  await wait(LYRICS[0].text);
  await p.evaluate(() => (window.__shiftFrom = performance.now()));
  const seen = [await boxes()];
  for (const line of LYRICS.slice(1)) {
    await wait(line.text);
    seen.push(await boxes());
  }
  const last = seen[seen.length - 1];
  const cls = await p.evaluate(() => window.__shifts.filter((e) => e.at > window.__shiftFrom).reduce((a, e) => a + e.value, 0));
  const shifts = await p.evaluate(() => window.__shifts.filter((e) => e.at > window.__shiftFrom));
  // The numbers are the contract: the summed CLS, the lyric box's height on
  // every line, and how far the boxes the eye follows moved.
  const same = (sel) => seen.every((b) => b[sel] && round(b[sel].top) === round(seen[0][sel].top) && round(b[sel].height) === round(seen[0][sel].height));
  const heights = seen.map((b) => b.lyric && b.lyric.height);
  const fixedHeight = heights.every((h) => h !== null && round(h) === round(heights[0]));
  console.log(
    `${w}x${h}  CLS ${cls.toFixed(4)}  lyric box h ${heights.join(" ")}  stagebox top ${seen.map((b) => b.stage && b.stage.top).join(" ")}  ` +
      `controls top ${seen.map((b) => b.controls && b.controls.top).join(" ")}`
  );
  check(cls === 0, `${w}x${h}: ${LYRICS.length - 1} synced lines, an empty one included, cost no layout shift`, `CLS ${cls.toFixed(4)}${shifts.length ? " " + JSON.stringify(shifts) : ""}`);
  check(fixedHeight, `${w}x${h}: the lyric box keeps one height, empty line and three-row line included`, heights.join(" "));
  check(same("stage"), `${w}x${h}: the cover carousel does not move`, seen.map((b) => b.stage && `${b.stage.top}/${b.stage.height}`).join(" "));
  check(same("controls"), `${w}x${h}: the transport does not move`, seen.map((b) => b.controls && b.controls.top).join(" "));
  // 360x640 is the tightest phone the shell supports: nothing may be cut.
  const fits = await p.evaluate(() => {
    const f = document.querySelector(".m .footer")?.getBoundingClientRect();
    const b = document.querySelector(".m .peek")?.getBoundingClientRect();
    const last = b && b.height ? b.bottom : f ? f.bottom : 0;
    return { last: Math.round(last), viewport: window.innerHeight };
  });
  check(fits.last <= fits.viewport, `${w}x${h}: the thumb row and "À propos" fit the screen`, `${fits.last} of ${fits.viewport} px`);
  await c.close();
}

await browser.close();
// SIGTERM first: the server removes its throwaway library on it, and /tmp is
// RAM (tools/perf_api.py). SIGKILL is the fallback inside stopServer.
await stopServer(srv);
if (state.failed) process.exitCode = 1;
