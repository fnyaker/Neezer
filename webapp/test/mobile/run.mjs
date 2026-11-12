#!/usr/bin/env node
// The phone shell, measured at the breakpoint's edges.
//
//   npm run build && node test/mobile/run.mjs
//
//   PLAYWRIGHT=/path/to/playwright-core CHROMIUM=/usr/bin/chromium node test/mobile/run.mjs
//
// Sweeps 360 / 390 / 639 / 640 / 641 CSS px (the last one is a narrow DESKTOP
// window, not a phone). At every width it holds the frame to DESIGN §9:
//   - nothing overflows sideways (neither the document nor <main>);
//   - the tab bar is 64 px of content sitting on the bottom edge, the mini
//     player is 64 px and floats 8 px above it (641: the desktop bar is 88 and
//     the tab bar is gone);
//   - tapping Bibliothèque lands on the library;
//   - a track plays, the card opens the full-screen player, Réduire closes it;
//   - `data-theme` is the one the session booted with, before and after;
//   - a Now-Playing panel persisted from the desktop is NOT mounted on a phone
//     (it would be an off-screen third column) but still is at 641.
//
// The server is the repository's own bench server (tools/perf_api.py --serve,
// 8 000 tracks) against the BUILT SPA: run `npm run build` first.
import { existsSync } from "node:fs";
import { startServer, login, checker } from "../harness.mjs";

async function loadPlaywright() {
  const specs = [
    process.env.PLAYWRIGHT,
    "playwright",
    "playwright-core",
    // The shared bench machine's copy, for when the repo has none.
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
const executablePath =
  process.env.CHROMIUM || (existsSync("/usr/bin/chromium") ? "/usr/bin/chromium" : undefined);
const { base, stop } = await startServer();
const browser = await chromium.launch({ executablePath, args: ["--no-sandbox"] });
const { check, state } = checker();

// width, height, phone? — 641 keeps a finger's context but the desktop frame.
const SIZES = [
  [360, 640, true],
  [390, 844, true],
  [639, 800, true],
  [640, 800, true],
  [641, 800, false],
];
const THEME = "papier"; // not the default: proves the boot theme is kept

for (const [w, h, phone] of SIZES) {
  const tag = `${w}px`;
  const ctx = await browser.newContext({
    viewport: { width: w, height: h },
    deviceScaleFactor: phone ? 2 : 1,
    isMobile: phone,
    hasTouch: phone,
  });
  // Deezer's CDN is never part of a bench.
  await ctx.route(
    (u) => u.hostname === "api.deezer.com" || u.hostname.endsWith(".dzcdn.net"),
    (r) => r.abort()
  );
  await ctx.addInitScript(
    ({ theme }) => {
      try {
        localStorage.setItem("ui.theme", JSON.stringify(theme));
        // A queue panel left open on a desktop, reopened on a phone: it must
        // not come back as a column nobody can reach.
        localStorage.setItem("ui.nowPlaying", "true");
      } catch {
        /* no storage */
      }
    },
    { theme: THEME }
  );
  const page = await ctx.newPage();
  await login(page, base);
  // login() authenticates AFTER the first boot already asked /api/me; a real
  // navigation is what makes the shell boot signed in.
  await page.goto(base + "/app/");
  await page.waitForSelector(phone ? "nav.mobilenav" : "nav.sidebar", { timeout: 30000 });
  await page.waitForSelector("footer.player", { timeout: 30000 });
  await page.waitForTimeout(700); // fonts settle before the geometry is read

  const frame = await page.evaluate(() => {
    const box = (s) => {
      const e = document.querySelector(s);
      if (!e) return null;
      const r = e.getBoundingClientRect();
      return { x: r.x, y: r.y, w: r.width, h: r.height, bottom: r.bottom, right: r.right };
    };
    const cs = getComputedStyle(document.documentElement);
    const main = document.querySelector("main");
    return {
      doc: { sw: document.documentElement.scrollWidth, cw: document.documentElement.clientWidth },
      main: main ? { sw: main.scrollWidth, cw: main.clientWidth } : null,
      nav: box("nav.mobilenav"),
      navVisible: (() => {
        const n = document.querySelector("nav.mobilenav");
        return !!n && getComputedStyle(n).display !== "none";
      })(),
      player: box("footer.player"),
      pp: box("footer.player .pp"),
      np: !!document.querySelector("aside.np"),
      theme: document.documentElement.dataset.theme,
      vw: innerWidth,
      vh: innerHeight,
      tokens: {
        mini: cs.getPropertyValue("--mini-h").trim(),
        tabbar: cs.getPropertyValue("--tabbar-h").trim(),
        safeBottom: cs.getPropertyValue("--safe-bottom").trim(),
      },
    };
  });

  check(frame.doc.sw <= frame.doc.cw + 1, `${tag}: no sideways overflow (document)`, `${frame.doc.sw}/${frame.doc.cw}`);
  check(
    !frame.main || frame.main.sw <= frame.main.cw + 1,
    `${tag}: no sideways overflow (main)`,
    frame.main ? `${frame.main.sw}/${frame.main.cw}` : ""
  );
  check(frame.theme === THEME, `${tag}: boot theme kept`, `${frame.theme}`);
  check(frame.player && frame.player.h === (phone ? 64 : 88), `${tag}: player is ${phone ? 64 : 88} px tall`, frame.player ? `${frame.player.h}` : "missing");

  if (phone) {
    check(frame.tokens.mini === "64px" && frame.tokens.tabbar === "64px", `${tag}: structure tokens are 64 px`, JSON.stringify(frame.tokens));
    check(frame.navVisible, `${tag}: the tab bar is shown`);
    check(frame.nav && Math.abs(frame.nav.h - 64) <= 1, `${tag}: the tab bar is 64 px of content`, frame.nav ? `${frame.nav.h}` : "missing");
    check(frame.nav && Math.abs(frame.nav.bottom - frame.vh) <= 1, `${tag}: the tab bar sits on the bottom edge`, frame.nav ? `${frame.nav.bottom} / ${frame.vh}` : "");
    check(
      frame.player && frame.nav && frame.player.bottom <= frame.nav.y - 8 + 1 && frame.player.bottom >= frame.nav.y - 8 - 1,
      `${tag}: the mini player floats 8 px above the tab bar`,
      frame.player && frame.nav ? `card bottom ${frame.player.bottom}, bar top ${frame.nav.y}` : ""
    );
    check(!frame.np, `${tag}: the desktop Now-Playing column is not mounted`);
    check(frame.player && frame.player.right <= frame.vw - 11 && frame.player.x >= 11, `${tag}: the card keeps a 12 px gutter`, frame.player ? `x ${frame.player.x}, right ${frame.player.right}` : "");

    // The four destinations live in the tab bar, with the library one wired.
    await page.locator('nav.mobilenav a[href="#/library"]').click();
    await page.waitForFunction(() => location.hash === "#/library", null, { timeout: 15000 });
    await page.waitForSelector('nav.mobilenav a[href="#/library"][aria-current="page"]', { timeout: 5000 });
    check(true, `${tag}: Bibliothèque is marked current`);
    await page.waitForSelector(".row.track", { timeout: 30000 });

    // Play, then the card opens the full screen and Réduire closes it.
    await page.locator(".row.track .titles").first().click();
    await page.locator("footer.player .pp").waitFor({ state: "visible", timeout: 15000 });
    await page.waitForTimeout(600);
    const pp = await page.evaluate(() => {
      const r = document.querySelector("footer.player .pp")?.getBoundingClientRect();
      return r ? { w: r.width, h: r.height } : null;
    });
    check(pp && pp.w === 44 && pp.h === 44, `${tag}: play is a 44 px target`, pp ? `${pp.w}x${pp.h}` : "missing");

    // Play must answer IN the card and go no further: the icon is re-rendered
    // by the press, so an ancestor check made every press open the full screen.
    await page.locator("footer.player .pp").click();
    await page.waitForTimeout(350);
    check((await page.locator(".m").count()) === 0, `${tag}: pressing play stays on the mini player`);

    // A tap on the card's own surface (not one of its buttons) is the door to
    // the full-screen player.
    await page.locator("footer.player").click({ position: { x: 4, y: 4 } });
    await page.locator(".m").waitFor({ state: "visible", timeout: 15000 });
    check(true, `${tag}: the card opens the full-screen player`);
    await page.locator('button[aria-label="Réduire"]').first().click();
    await page.locator(".m").waitFor({ state: "detached", timeout: 15000 });
    // …and so is the labelled button, for the keyboard and a screen reader.
    await page.locator("footer.player .now").click();
    await page.locator(".m").waitFor({ state: "visible", timeout: 15000 });
    await page.locator('button[aria-label="Réduire"]').first().click();
    await page.locator(".m").waitFor({ state: "detached", timeout: 15000 });
    check(await page.locator("nav.mobilenav").isVisible(), `${tag}: closing returns to the shell`);
    const after = await page.evaluate(() => ({
      theme: document.documentElement.dataset.theme,
      sw: document.documentElement.scrollWidth,
      cw: document.documentElement.clientWidth,
    }));
    check(after.theme === THEME, `${tag}: the theme survives the player`, after.theme);
    check(after.sw <= after.cw + 1, `${tag}: still no sideways overflow after the player`, `${after.sw} > ${after.cw}`);
  } else {
    check(!frame.navVisible, `${tag}: a 641 px window is a narrow desktop, no tab bar`);
    check(!frame.tokens.mini, `${tag}: the phone-only tokens are not defined`, frame.tokens.mini);
    check(frame.np, `${tag}: the desktop Now-Playing column still mounts`);
  }
  await ctx.close();
}

await browser.close();
stop();
console.log(state.failed ? `\n${state.failed} check(s) failed` : "\nall mobile frame checks passed");
process.exit(state.failed ? 1 : 0);
