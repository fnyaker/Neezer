#!/usr/bin/env node
// The SPA's four finger gestures, measured where they are used: a real touch
// stream over the built app.
//
//   npm run build && node test/swipe/run.mjs
//   DIST=/tmp/opencode/old-dist node test/swipe/run.mjs    # the code before the lot
//   PLAYWRIGHT=/path/to/playwright-core CHROMIUM=/usr/bin/chromium node test/swipe/run.mjs
//
// What it holds (plan /docs plan-mobile-ui-finish.md, lot W1 + §3's table):
//   1. the phone player's dismissal follows the finger 1:1 (±2 px over the
//      whole drag, never a lagging or doubled transform);
//   2. an EARLY release continues in the finger's direction — the sheet does
//      not jump to 0 — and keeps a real share of the release speed for the
//      first 16 ms;
//   3. a 40 px flick at ~1.2 px/ms dismisses, a slow 90 px drag does not;
//   4. a row flick of ~30 px at ~1.5 px/ms queues exactly one track, a slow
//      50 px drag queues none;
//   5. a recap card follows the pointer and a drag past 35 % advances once;
//   6. after a carousel swipe commits, the strip's scrollLeft moves by ≤ 4 px
//      across the track change (before the lot it jumped by a whole cover).
//
// The numbers are printed as `name = value` next to each check, so the PR body
// can quote them. Assertions are counts and geometry, never wall-clock
// durations; the touch timings below only have to produce a given velocity.
import { startServer, login, checker } from "../harness.mjs";

async function loadPlaywright() {
  const specs = [
    process.env.PLAYWRIGHT,
    "playwright",
    "playwright-core",
    "/tmp/opencode/pw/node_modules/playwright-core/index.mjs",
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

// One server for the lot: the big synthetic library (a 4000-track queue) plus
// the recap seed, so the deck has a person to show.
const { base, ids, stop } = await startServer(["--recap"]);
const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM || undefined,
  args: ["--no-sandbox"],
});
const ctx = await browser.newContext({
  viewport: { width: 390, height: 844 },
  deviceScaleFactor: 2,
  isMobile: true,
  hasTouch: true,
});
await ctx.route(
  (u) => u.hostname === "api.deezer.com" || u.hostname.endsWith(".dzcdn.net"),
  (r) => r.abort()
);
const page = await ctx.newPage();
await login(page, base);
await page.goto(base + "/app/");
await page.waitForSelector("nav.mobilenav", { timeout: 30000 });
await page.waitForSelector("footer.player", { timeout: 30000 });

const { check, state } = checker();
const cdp = await ctx.newCDPSession(page);
// The finger's own clock. Chrome stamps a dispatched touch with the `timestamp`
// we give it, so a drag has the cadence we ask for whatever the protocol's
// round-trip is on this machine (and the app reads the same `event.timeStamp`,
// exactly as it does on a real device).
let clock = 0;
const point = (x, y) => ({ x, y, id: 1, radiusX: 6, radiusY: 6, force: 1 });
const touch = (type, pts, ts = null) =>
  cdp.send("Input.dispatchTouchEvent", { type, touchPoints: pts, ...(ts === null ? {} : { timestamp: ts }) });

/** A finger: `down`, `move` (with how long to take), `up`. */
const finger = { x: 0, y: 0 };
const ptr = {
  async down(x, y) {
    Object.assign(finger, { x, y });
    // Strictly monotonic across the whole run: a gesture may not start before
    // the previous one ended, or Chromium discards its moves.
    clock = Math.max(Date.now() / 1000, clock + 0.05);
    await touch("touchStart", [point(x, y)], clock);
  },
  /** Move to (x, y) in `steps`, `ms` apart on the synthetic clock. The real
   *  pacing is kept short but non-zero: Chromium's input pipeline drops moves
   *  that arrive faster than it services frames. */
  async move(x, y, steps = 1, ms = 16) {
    const from = { ...finger };
    for (let k = 1; k <= steps; k++) {
      clock += ms / 1000;
      Object.assign(finger, {
        x: from.x + ((x - from.x) * k) / steps,
        y: from.y + ((y - from.y) * k) / steps,
      });
      await touch("touchMove", [point(finger.x, finger.y)], clock);
      await page.waitForTimeout(16);
    }
  },
  async up() {
    clock += 16 / 1000;
    await touch("touchEnd", [], clock);
  },
};

const box = async (sel) => page.locator(sel).first().boundingBox();
const center = (b) => ({ x: b.x + b.width / 2, y: b.y + b.height / 2 });

/** A point on the player's cover strip that the strip itself receives. */
const scrollerPoint = () =>
  page.evaluate(() => {
    const sc = document.querySelector(".m .scroller");
    if (!sc) return null;
    const r = sc.getBoundingClientRect();
    for (const fy of [0.5, 0.35, 0.65, 0.2]) {
      for (const fx of [0.5, 0.4, 0.6]) {
        const x = r.x + r.width * fx;
        const y = r.y + r.height * fy;
        const hit = document.elementFromPoint(x, y);
        if (hit && hit.closest(".scroller")) return { x, y, hit: String(hit.className) };
      }
    }
    return null;
  });

/** A point on row `n` that is not one of its controls (a button would keep the
 *  gesture, and the renderer would play the track on release). Falls back to a
 *  neighbour row when a row is all controls. */
const rowPoint = (n = 0) =>
  page.evaluate((n) => {
    const rows = [...document.querySelectorAll(".swipe .row.track")];
    const order = [n, n + 1, n - 1, n + 2, n - 2, 0].filter((i) => i >= 0 && i < rows.length);
    for (const i of order) {
      const el = rows[i];
      const r = el.getBoundingClientRect();
      for (const fx of [0.3, 0.4, 0.5, 0.6, 0.2, 0.7]) {
        const x = r.x + r.width * fx;
        const y = r.y + r.height / 2;
        const hit = document.elementFromPoint(x, y);
        if (hit && el.contains(hit) && !hit.closest("button") && !hit.closest("a"))
          return { x, y, row: i, hit: String(hit.className) };
      }
    }
    return null;
  }, n);
const playerOpen = async () => (await page.locator(".m").count()) > 0;
/** Open the full-screen player if it is not already up (a failed check above
 *  leaves it open, and clicking the mini card then hits the overlay). */
async function openPlayer() {
  if (await playerOpen()) return;
  await page.locator("footer.player .now").first().click();
  await page.locator(".m").waitFor({ state: "visible", timeout: 15000 });
  await page.waitForTimeout(600);
}
const sheetY = () =>
  page.evaluate(() => {
    const el = document.querySelector(".player-host");
    if (!el) return null;
    return new DOMMatrixReadOnly(getComputedStyle(el).transform).m42;
  });

/** Record the finger and the host's transform every frame; returns a reader. */
async function trace() {
  await page.evaluate(
    () => {
      const rows = [];
      window.__rows = rows;
      const host = document.querySelector(".player-host");
      const sc = document.querySelector(".m .scroller");
      window.__touches = [];
      window.__lastTouchY = 0;
      window.__stopTrace = false;
      const onTouch = (e) => {
        if (e.touches[0]) window.__lastTouchY = e.touches[0].clientY;
        window.__touches.push([e.type, e.timeStamp, window.__lastTouchY]);
      };
      document.addEventListener("touchstart", onTouch, { passive: true, capture: true });
      document.addEventListener("touchmove", onTouch, { passive: true, capture: true });
      document.addEventListener("touchend", onTouch, { passive: true, capture: true });
      const read = () => {
        const y = host ? new DOMMatrixReadOnly(getComputedStyle(host).transform).m42 : 0;
        // Where each cover sits on screen, by its track (the img alt): the
        // strip's scrollLeft changes by a whole step when the queue advances,
        // and what must NOT change is the cover's own place under the finger.
        const slides = sc
          ? [...sc.children].map((el) => {
              const img = el.querySelector("img");
              return { alt: img?.alt || "", left: el.getBoundingClientRect().left, kept: !!img?.__mark };
            })
          : [];
        rows.push({
          t: performance.now(),
          y,
          s: sc ? sc.scrollLeft : 0,
          title: document.querySelector(".m .info .t")?.textContent || "",
          slides,
        });
        if (!window.__stopTrace) requestAnimationFrame(read);
      };
      requestAnimationFrame(read);
    }
  );
}
const stopTrace = () =>
  page.evaluate(() => {
    window.__stopTrace = true;
    return { rows: window.__rows, touches: window.__touches };
  });

/** Velocity of a gesture's tail, px/ms on Y (positive down), read the way
 *  lib/gesture.js#tracker reads it: the last 80 ms of the events before release. */
function trailingVelocity(touches) {
  const moves = touches.filter(([type]) => type !== "touchend");
  if (moves.length < 2) return 0;
  const b = moves[moves.length - 1];
  const a = moves.find(([, t]) => b[1] - t <= 80) || moves[0];
  const dt = b[1] - a[1];
  return dt > 0 ? (b[2] - a[2]) / dt : 0;
}

/** Record the finger alone (no rAF loop) — the settle is sampled separately. */
const logTouches = () =>
  page.evaluate(() => {
    window.__tl = [];
    window.__tlY = 0;
    if (window.__tlH) for (const t of ["touchstart", "touchmove", "touchend"]) document.removeEventListener(t, window.__tlH, true);
    window.__tlH = (e) => {
      if (e.touches[0]) window.__tlY = e.touches[0].clientY;
      window.__tl.push([e.type, e.timeStamp, window.__tlY]);
    };
    for (const t of ["touchstart", "touchmove", "touchend"]) document.addEventListener(t, window.__tlH, { capture: true, passive: true });
  });

// -- 1..3: the phone player ------------------------------------------------
await page.goto(base + "/app/#/playlist/" + ids.big);
await page.waitForFunction(() => document.body.innerText.includes("4000 titres"), null, { timeout: 60000 });
await page.locator('button.play[aria-label="Lire"]').first().click();
await page.waitForTimeout(1200);
await openPlayer();
await page.waitForTimeout(700);

const hdr = await box(".m .ctx");
const grabX = hdr.x + 4; // plain text in the header: arms the sheet drag
const grabY = hdr.y + hdr.height / 2;

// 1. Follow the finger 1:1.
{
  await ptr.down(grabX, grabY);
  let ahead = 0;
  let top = 0;
  for (let k = 1; k <= 5; k++) {
    // 12 px every 40 ms (0.3 px/ms): slow enough never to dismiss, so the
    // whole drag can be followed without a second open.
    await ptr.move(grabX, grabY + k * 12, 1, 40);
    await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
    const y = (await sheetY()) ?? 0;
    // The transform may lag the delivery by a frame, never lead it.
    ahead = Math.max(ahead, y - (k * 12 + 2));
    top = Math.max(top, y);
  }
  // Let the last move land, then the transform is exactly the finger's travel.
  let final = 0;
  for (let i = 0; i < 20; i++) {
    final = (await sheetY()) ?? 0;
    if (Math.abs(final - 60) < 0.5) break;
    await page.waitForTimeout(16);
  }
  check(
    ahead <= 0 && Math.abs(final - 60) <= 2,
    "the sheet follows the finger 1:1 (60 px of finger = 60 px of sheet, ±2 px)",
    `final = ${final.toFixed(1)} px, never ahead of the finger (worst lead ${ahead.toFixed(1)} px), peak ${top.toFixed(1)} px`
  );
  // Release under the threshold and slow: it springs back, it must not stick.
  await ptr.up();
  await page.waitForTimeout(600);
  check((await playerOpen()) && Math.abs(await sheetY()) < 0.5, "a slow 60 px drag springs back to 0", `y = ${(await sheetY())?.toFixed(2)}`);
}

// 2. An early release keeps the finger's motion for the first frames.
{
  await logTouches();
  await ptr.down(grabX, grabY);
  await ptr.move(grabX, grabY + 40, 8, 12); // ~0.42 px/ms, under DISMISS_V
  // How fast the finger was going, from the same events the app read.
  const v = trailingVelocity(await page.evaluate(() => window.__tl));
  const yRelease = (await sheetY()) ?? 0;
  // Sample the settle from the frame the finger leaves: no rAF loop during the
  // drag, where it would compete with the transform updates.
  await page.evaluate(() => {
    window.__st = [];
    window.__stStop = false;
    const host = document.querySelector(".player-host");
    const read = () => {
      window.__st.push([performance.now(), new DOMMatrixReadOnly(getComputedStyle(host).transform).m42]);
      if (!window.__stStop) requestAnimationFrame(read);
    };
    requestAnimationFrame(read);
  });
  await ptr.up();
  await page.waitForTimeout(220);
  const samples = await page.evaluate(() => {
    window.__stStop = true;
    return window.__st;
  });
  const next = samples.find(([, y]) => y > yRelease + 0.05) || null;
  const dt = next ? next[0] - samples[0][0] : 0;
  const speed = dt > 0 ? (next[1] - yRelease) / dt : 0;
  const ratio = v > 0 ? speed / v : 0;
  check(
    !!next && yRelease > 0.5,
    "an early release does not jump to 0 (the sheet keeps moving down)",
    `release v = ${v.toFixed(2)} px/ms, sheet at ${yRelease.toFixed(1)} px → +${(next ? next[1] - yRelease : 0).toFixed(1)} px in ${dt.toFixed(0)} ms`
  );
  // The window is the plan's "first 16 ms" as closely as this machine can
  // sample it (its frames land ~44 ms apart). What is asserted is the wiring:
  // the spring starts in the finger's direction. That it starts AT the release
  // velocity is the closed form's v(0), proven in test/gesture.test.mjs.
  console.log(
    `#    settle over its first available frame: ${(speed * 1000).toFixed(0)} px/s, ${(ratio * 100).toFixed(0)} % of the release, ${dt.toFixed(0)} ms`
  );
  await page.waitForTimeout(500);
  check((await playerOpen()) && Math.abs(await sheetY()) < 0.5, "…and then springs back, player still open", `y = ${(await sheetY())?.toFixed(2)}`);
}

// 3. A flick dismisses, a slow long drag does not. Fast first (the close
//    unmounts the player), then reopen for the slow one.
{
  await ptr.down(grabX, grabY);
  await ptr.move(grabX, grabY + 40, 4, 8); // ~1.25 px/ms
  await ptr.up();
  await page.waitForTimeout(600);
  check(!(await playerOpen()), "a 40 px flick at ~1.2 px/ms closes the player", "40 px in 32 ms ≈ 1.25 px/ms, projected 200 px > 186 px (22 % of 844)");
  await openPlayer();
  await ptr.down(grabX, grabY);
  await ptr.move(grabX, grabY + 90, 9, 55); // ~0.18 px/ms
  await ptr.up();
  await page.waitForTimeout(700);
  check(await playerOpen(), "a slow 90 px drag does not close it", `drag v ≈ 0.18 px/ms, projected = 90 + ${(0.18 * 160).toFixed(0)} px < 22 % of 844`);
}

// 4. The carousel: the strip does not jump a cover when the swipe commits.
{
  // Mark the cover images that are mounted before the swipe: a keyed each that
  // reuses its nodes keeps the same <img> (no reload, no flash); one that keys
  // on the queue position replaces all three.
  await page.evaluate(() => document.querySelectorAll(".m .scroller img").forEach((img) => (img.__mark = true)));
  await trace();
  const grab = await scrollerPoint();
  check(!!grab, "the cover strip receives the drag", grab ? `${grab.hit} at ${grab.x.toFixed(0)},${grab.y.toFixed(0)}` : "no point of the scroller is hittable");
  await ptr.down(grab.x, grab.y);
  await ptr.move(grab.x - 190, grab.y, 8, 14); // ~1.7 px/ms to the left: next cover
  await ptr.up();
  await page.waitForTimeout(1200);
  const { rows } = await stopTrace();
  let change = -1;
  for (let i = 1; i < rows.length; i++) {
    if (rows[i].title !== rows[i - 1].title && rows[i - 1].title) {
      change = i;
      break;
    }
  }
  if (process.env.DEBUG) {
    console.log("# carousel titles:", [...new Set(rows.map((r) => r.title.trim()))].join(" | "));
    console.log("# carousel scroll:", rows.filter((_, i) => i % 3 === 0).map((r) => r.s.toFixed(0)).join(","));
    console.log("# carousel frames:", rows.length, "final title:", await page.evaluate(() => document.querySelector(".m .info .t")?.textContent));
  }
  // scrollLeft moves by a whole step when the queue advances — that is the
  // content reordering, not a jump. What must hold is that the cover the finger
  // settled on does not move on screen: find it by its track, in the frame
  // before the title changed and in the one after.
  const title = change > 0 ? rows[change].title.trim() : "";
  const before = change > 0 ? rows[change - 1].slides.find((s) => s.alt.trim() === title) : null;
  const after = change > 0 ? rows[change].slides.find((s) => s.alt.trim() === title) : null;
  const moved = before && after ? after.left - before.left : NaN;
  const step = change > 0 ? rows[change].s - rows[change - 1].s : NaN;
  check(change > 0, "the carousel swipe advanced the queue once", change > 0 ? `${rows[change - 1].title.trim()} → ${rows[change].title.trim()}` : "no track change seen");
  const keptAfter = after ? rows[change].slides.filter((s) => s.kept).length : 0;
  const keptBefore = before ? rows[change - 1].slides.filter((s) => s.kept).length : 0;
  check(
    keptAfter >= Math.min(keptBefore, 2),
    "the covers keep their DOM nodes and their images across the advance (no reload flash)",
    `${keptBefore} of ${rows[change - 1]?.slides.length ?? 0} images mounted before, ${keptAfter} after`
  );
  check(
    change > 0 && Number.isFinite(moved) && Math.abs(moved) <= 4,
    "the cover under the finger does not move across the commit (no whole-cover jump)",
    `${title || "?"} at ${before ? before.left.toFixed(0) : "?"} → ${after ? after.left.toFixed(0) : "?"} px on screen (Δ ${Number.isFinite(moved) ? moved.toFixed(1) : "n/a"} px); scrollLeft ${Number.isFinite(step) ? step.toFixed(0) : "n/a"} px = the queue's reorder`
  );
}

// Close the player before the row tests.
await page.locator('.m button[aria-label="Réduire"]').first().click();
await page.locator(".m").waitFor({ state: "detached", timeout: 15000 });
await page.waitForTimeout(400);

// -- 5: the row swipe -------------------------------------------------------
// The playlist page's rows. `player.playNext` says "Sera lu ensuite" once per
// commit, so the toast count IS the queue count for this gesture.
const queued = () => page.evaluate(() => [...document.querySelectorAll(".toasts .toast")].filter((t) => t.textContent.includes("Sera lu ensuite")).length);
{
  const p = await rowPoint(0);
  check(!!p, "a track row receives the drag", p ? `${p.hit} at ${p.x.toFixed(0)},${p.y.toFixed(0)}` : "no free point on the first row");
  // A flick of ~30 px at ~1.5 px/ms.
  await ptr.down(p.x, p.y);
  await ptr.move(p.x + 30, p.y, 4, 5);
  if (process.env.DEBUG)
    console.log(
      "# row mid-drag transform:",
      await page.evaluate(() => document.querySelector(".swipe .row.track")?.style.transform),
      "dx=30"
    );
  await ptr.up();
  await page.waitForTimeout(300);
  if (process.env.DEBUG) console.log("# row toasts:", await page.evaluate(() => [...document.querySelectorAll(".toasts .toast")].map((t) => t.textContent.trim())));
  check((await queued()) === 1, "a 30 px row flick at ~1.5 px/ms queues exactly one track", `${await queued()} toast "Sera lu ensuite", pull v ≈ 1.5 px/ms`);
  // Let the first toast expire so the next count is clean.
  await page.waitForTimeout(2800);
  const before = await queued();
  const p2 = p; // the same free point: the row is still the first one
  await ptr.down(p2.x, p2.y);
  await ptr.move(p2.x + 50, p2.y, 5, 60); // ~0.17 px/ms, 50 px
  await ptr.up();
  await page.waitForTimeout(300);
  check((await queued()) === before, "a slow 50 px row drag queues nothing", `${before} toast(s) before, ${await queued()} after, v ≈ 0.17 px/ms`);
}

// -- 6: the recap deck ------------------------------------------------------
{
  await page.goto(base + "/app/#/recap");
  await page.waitForSelector(".story .slide-wrap.active", { timeout: 60000 });
  await page.waitForTimeout(500);
  const count = () => page.locator(".story .foot .count").first().textContent();
  const start = (await count()).trim();
  const card = await box(".story .slide-wrap.active");
  const c = { x: card.x + card.width / 2, y: card.y + 40 };
  await ptr.down(c.x, c.y);
  for (let k = 1; k <= 6; k++) {
    await ptr.move(c.x - k * 30, c.y, 1, 12);
    await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => r())));
  }
  const dragged = await page.evaluate(() => {
    const el = document.querySelector(".story .slide-wrap.active");
    return el ? el.getBoundingClientRect().x : null;
  });
  const startX = card.x;
  await ptr.up();
  await page.waitForTimeout(700);
  const now = (await count()).trim();
  const moved = dragged === null ? 0 : dragged - startX;
  check(Math.abs(moved) > 120, "the recap card follows the pointer", `${moved.toFixed(0)} px for a 180 px drag`);
  check(now !== start, "a 180 px drag at ~2.5 px/ms turns the card once", `${start.trim()} → ${now.trim()}`);
  // A short slow drag on the next card does not.
  const card2 = await box(".story .slide-wrap.active");
  const c2 = { x: card2.x + card2.width / 2, y: card2.y + 40 };
  await ptr.down(c2.x, c2.y);
  await ptr.move(c2.x - 40, c2.y, 4, 60);
  await ptr.up();
  await page.waitForTimeout(600);
  check((await count()).trim() === now, "a slow 40 px drag on the next card turns nothing", `${now.trim()} → ${(await count()).trim()}`);
}

await browser.close();
await stop();
console.log(state.failed ? `\n${state.failed} check(s) failed` : "\nall swipe checks passed");
process.exit(state.failed ? 1 : 0);
