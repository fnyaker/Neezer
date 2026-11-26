#!/usr/bin/env node
// The WEB app's half of the judge's comparison (plan-mobile-tests, lot T9).
//
//   cd webapp && npm run build && node test/screens/run.mjs
//   cd webapp && node test/screens/run.mjs --check     # every reference present
//
// Captures the ten screens the judge puts beside the RELEASE APK's own
// screenshots (`mobile/tools/judge/reference/`), at the EMULATOR's dp size:
// 411 × 866 CSS px with deviceScaleFactor 2.625 — the AVD `pixel_6` is
// 1080 × 2400 at 420 dpi (1080 / 2.625 = 411 dp wide, 2400 − 24 dp of status
// bar − 24 dp of gesture bar = 866 dp of content). Same dp width and height ⇒
// a margin or a bar height compares in dp, which is the whole point.
//
// The server is the repository's own bench (`tools/perf_api.py --serve`, the
// 8 000-track synthetic library, login bench / Bench1) — the SAME seed the
// phone signs into, so both sides show the same titles, artists and ids. Run
// `npm run build` first: the bench serves the BUILT SPA.
//
// `09-lecteur-suivant` has no reference: it is a DELTA check on the Android
// side (08 vs 09: the title and the sleeve change, the layout does not). The
// player is captured once (08), its queue (10), and the sheet mid-drag (11).
//
// Two facts about this tree that the capture has to respect, both measured
// here (the lot's PR says so):
//   - « Mode voiture » is drawn in Réglages › APPARENCE, not › Lecture & son
//     (`Settings.svelte`: the card sits in `tab === "appearance"`; the plan's
//     table named `playback`, which draws only the audio effects);
//   - that card exists only where the native bridge answers `carAvailable()`
//     (`lib/car.js`), so the 07 capture runs in its OWN context with a minimal
//     `window.NSNative` stand-in — the same "stand-in app" pattern as
//     `test/offline/run.mjs`. No other capture sees it.
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { execSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { startServer, login, deezerHost } from "../harness.mjs";

// The same loader `test/mobile/run.mjs` uses: PLAYWRIGHT=… for a copy the
// repo does not carry (a shared bench machine), then the module, then the
// global install. Chromium comes from CHROMIUM when /usr/bin/chromium is absent.
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

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, "../../..");
const REFERENCE = join(repo, "mobile/tools/judge/reference");

// The ten files of the reference table, in walk order. `09` is absent on
// purpose (a delta between two Android captures, never a web twin).
const REFERENCES = [
  "01-accueil",
  "02-rechercher",
  "03-bibliotheque",
  "04-podcasts",
  "05-reglages",
  "06-lecture-et-son",
  "07-mode-voiture",
  "08-lecteur",
  "10-file",
  "11-lecteur-glisse",
];

const VIEWPORT = { width: 411, height: 866 };
const SCALE = 2.625;
const VIEWPORT_LABEL = `${VIEWPORT.width}x${VIEWPORT.height}@${SCALE}`;

// --- --check: is the reference set complete? --------------------------------
// No server, no browser: the judge's task says "a missing reference is an
// escalation" and this is what tells it. Not part of `npm test` (it needs a
// built SPA and Chromium to RE-capture, like every other bench here).
if (process.argv.includes("--check")) {
  const missing = REFERENCES.filter((name) => !existsSync(join(REFERENCE, `${name}.png`)));
  if (missing.length) {
    console.error(`missing reference(s) in ${REFERENCE}: ${missing.join(", ")}`);
    process.exit(1);
  }
  console.log(`screens=ok ${REFERENCES.length}/${REFERENCES.length} in ${REFERENCE}`);
  process.exit(0);
}

const { chromium } = await loadPlaywright();
const executablePath =
  process.env.CHROMIUM || (existsSync("/usr/bin/chromium") ? "/usr/bin/chromium" : undefined);

mkdirSync(REFERENCE, { recursive: true });

// What the capture came from, two ways: the COMMIT at HEAD (the rubric's
// staleness rule compares it with the last commit that touched `webapp/src` and
// reports a stale reference as INFO, never as a block) and the tree object of
// `webapp/src` itself (the lot names `git rev-parse HEAD:webapp/src`, which is a
// tree, not a commit — both are recorded so neither reading is a guess).
let spaCommit = "unknown";
let spaTree = "unknown";
try {
  spaCommit = execSync("git rev-parse HEAD", { cwd: repo, encoding: "utf8" }).trim();
  spaTree = execSync("git rev-parse HEAD:webapp/src", { cwd: repo, encoding: "utf8" }).trim();
} catch {
  /* a source tarball has no git: the manifest says so */
}

// `--audio-all` gives every track a real silent WAV, so the tapped title PLAYS
// instead of being skipped as unarchived: the reference is then a stable track,
// not whichever one the auto-skip had reached when the shutter opened. The
// titles, artists and ids are the same seeded library either way.
const { base, stop } = await startServer(["--audio-all"]);
const browser = await chromium.launch({ executablePath, args: ["--no-sandbox"] });
const capturedAt = new Date().toISOString();
const manifest = {};

/** One context at the emulator's dp size, Deezer's CDN never part of a bench. */
async function newPhoneContext({ native = false } = {}) {
  const context = await browser.newContext({
    viewport: VIEWPORT,
    deviceScaleFactor: SCALE,
    isMobile: true,
    hasTouch: true,
  });
  await context.route(
    (u) => deezerHost(u),
    (r) => r.abort()
  );
  await context.addInitScript((withBridge) => {
    try {
      // The phone's own theme, so a capture is the same two runs in a row.
      localStorage.setItem("ui.theme", JSON.stringify("papier"));
    } catch {
      /* no storage */
    }
    // 07 only: the car card is drawn where the installed APK says the mode
    // exists (`lib/car.js`). A browser has no bridge, so the capture supplies
    // the one answer that card asks for.
    if (withBridge) window.NSNative = { carAvailable: () => true };
  }, native);
  return context;
}

/** Boots the shell signed in, as `test/mobile/run.mjs` does. */
async function openShell(context) {
  const page = await context.newPage();
  await login(page, base);
  // login() authenticates AFTER the first boot already asked /api/me; a real
  // navigation is what makes the shell boot signed in.
  await page.goto(base + "/app/");
  await page.waitForSelector("nav.mobilenav", { timeout: 30000 });
  await page.waitForSelector("footer.player", { timeout: 30000 });
  await page.waitForTimeout(700); // fonts settle before the pixels are read
  // The bench server has the Deezer proxy off, so the shell raises a persistent
  // notice above the content ("Le proxy Deezer est désactivé sur ce serveur").
  // It is the SERVER's state, not the screen the judge compares: dismissed
  // here, or it overlaps the very margin C2 measures. A notice that arrives
  // after this line is dismissed the same way before each shot (see `shoot`).
  await dismissNotices(page);
  return page;
}

/** No server notice over the content: the reference is the screen, not the bench. */
async function dismissNotices(page) {
  for (let i = 0; i < 5; i += 1) {
    const close = page.locator('.notices button[aria-label="Masquer"]');
    if ((await close.count()) === 0) return;
    await close.first().click({ timeout: 5000 }).catch(() => {});
    await page.waitForTimeout(150);
  }
}

/** The screen's own marker, then the shot. */
async function shoot(page_, name, route, marker) {
  if (marker) await page_.waitForSelector(marker, { timeout: 30000 });
  // A skeleton is a screen that has not answered yet: never photograph it.
  await page_
    .waitForFunction(() => !document.querySelector(".sk, .sk-row, .sk-header"), null, { timeout: 30000 })
    .catch(() => {});
  await page_.waitForTimeout(500);
  // The sheet/route transitions are CSS animations: a shot taken inside one is
  // a half-slid screen (measured: two runs of the same command disagreed on the
  // player). Wait for every animation to finish AND for the full-screen player
  // to cover the viewport exactly, then photograph.
  await page_
    .waitForFunction(
      () => {
        if (document.getAnimations().some((a) => a.playState === "running")) return false;
        const m = document.querySelector(".m");
        if (!m) return true;
        const r = m.getBoundingClientRect();
        return Math.abs(r.x) < 1 && Math.abs(r.y) < 1 && Math.abs(r.width - innerWidth) < 1;
      },
      null,
      { timeout: 15000 }
    )
    .catch(() => {});
  await page_.waitForTimeout(150);
  // A toast is a transient state ("Titre indisponible, passage au suivant" when
  // the bench's synthetic track cannot stream): a reference must not depend on
  // where in its 3 s it was photographed. Bounded, never a failure.
  await page_
    .waitForFunction(() => !document.querySelector(".toasts .toast"), null, { timeout: 8000 })
    .catch(() => {});
  await page_.screenshot({
    path: join(REFERENCE, `${name}.png`),
    animations: "disabled",
    caret: "hide",
  });
  manifest[`${name}.png`] = { spaCommit, spaTree, capturedAt, route, viewport: VIEWPORT_LABEL };
  console.log(`captured ${name}.png (${route})`);
}

// --- 01…06: the shell's own screens ----------------------------------------
const shell = await newPhoneContext();
const page = await openShell(shell);

await page.evaluate(() => (location.hash = "#/"));
await shoot(page, "01-accueil", "#/", ".home");

await page.evaluate(() => (location.hash = "#/search"));
await shoot(page, "02-rechercher", "#/search", 'input[placeholder^="Titres"]');

await page.evaluate(() => (location.hash = "#/library"));
await shoot(page, "03-bibliotheque", "#/library", ".row.track");

await page.evaluate(() => (location.hash = "#/podcasts"));
await shoot(page, "04-podcasts", "#/podcasts", "h1.page-title, .head h1");

await page.evaluate(() => (location.hash = "#/settings"));
await shoot(page, "05-reglages", "#/settings", 'input[aria-label="Chercher un réglage"]');

await page.evaluate(() => (location.hash = "#/settings/playback"));
await shoot(page, "06-lecture-et-son", "#/settings/playback", ".pane .pane-body .card");

// --- 08, 10, 11: the player ------------------------------------------------
// The sequence `test/mobile/run.mjs` already performs: a track from the
// library, then the mini player's own surface is the door to the full screen.
await page.evaluate(() => (location.hash = "#/library"));
await page.locator(".row.track .titles").first().click();
await page.locator("footer.player .pp").waitFor({ state: "visible", timeout: 15000 });
await page.waitForTimeout(600);
await page.locator("footer.player").click({ position: { x: 4, y: 4 } });
await page.locator(".m").waitFor({ state: "visible", timeout: 15000 });
// PAUSE AT 0:00, then let the sheet settle. A moving clock, a moving
// visualiser or a queue that advances is a reference that differs between two
// runs of the same command; a paused player at the start is the state every
// screen share of this app begins with, and the only one that is byte-stable.
const pause = page.locator('.m button[aria-label="Lecture/Pause"]');
if ((await pause.count()) > 0) await pause.click();
await page
  .locator('.m input[aria-label="Progression"]')
  .evaluate((el) => {
    el.value = "0";
    el.dispatchEvent(new Event("input", { bubbles: true }));
  })
  .catch(() => {});
await page.waitForTimeout(1200);
await shoot(page, "08-lecteur", "#/library → player card", ".m");

await page.locator('.m button[aria-label="File d\'attente"]').click();
await page
  .locator('[role="dialog"][aria-label="File d\'attente"]')
  .waitFor({ state: "visible", timeout: 15000 });
await shoot(page, "10-file", 'player → button[aria-label="File d\'attente"]', null);

// --- 11: the sheet mid-drag ------------------------------------------------
// The phone's sheet follows TOUCH events only (`MobileNowPlaying.svelte`:
// `on:touchstart` / `on:touchmove`), so the drag is injected through CDP and
// the shot is taken BEFORE `touchEnd`: the sheet is at 60 % of its travel, by
// construction, not by luck.
await page.locator('.sheet button[aria-label="Fermer la file d\'attente"]').click();
await page.locator(".m").waitFor({ state: "visible", timeout: 15000 });
await page.waitForTimeout(400);
const sheet = await page.locator(".m").boundingBox();
const cdp = await page.context().newCDPSession(page);
const x = Math.round(sheet.x + sheet.width / 2);
const y0 = Math.round(sheet.y + Math.min(120, sheet.height * 0.12));
const travel = Math.round(sheet.height * 0.6);
const touch = (type, point) =>
  cdp.send("Input.dispatchTouchEvent", {
    type,
    touchPoints: point ? [point] : [],
  });
await touch("touchStart", { x, y: y0 });
for (let i = 1; i <= 6; i += 1) {
  await touch("touchMove", { x, y: Math.round(y0 + (travel * i) / 6) });
  await page.waitForTimeout(40);
}
await shoot(page, "11-lecteur-glisse", "player, dragged down 60 %", null);
await touch("touchEnd");

await shell.close();

// --- 07: the car card, in its own context ----------------------------------
// Own page on purpose: `window.NSNative` is injected only here, so no other
// reference is captured under a bridge the browser does not have.
const carContext = await newPhoneContext({ native: true });
const carPage = await openShell(carContext);
await carPage.evaluate(() => (location.hash = "#/settings/appearance"));
await carPage.waitForSelector("h2", { timeout: 30000 });
await carPage
  .locator("h2", { hasText: "Mode voiture" })
  .scrollIntoViewIfNeeded({ timeout: 30000 });
await shoot(
  carPage,
  "07-mode-voiture",
  "#/settings/appearance (scrolled to Mode voiture)",
  "h2"
);
await carContext.close();

await browser.close();
await stop();

const ordered = Object.fromEntries(
  Object.entries(manifest).sort(([a], [b]) => a.localeCompare(b))
);
writeFileSync(join(REFERENCE, "manifest.json"), JSON.stringify(ordered, null, 2) + "\n");
console.log(`references=${Object.keys(ordered).length} viewport=${VIEWPORT_LABEL} spa=${spaCommit}`);
process.exit(0);
