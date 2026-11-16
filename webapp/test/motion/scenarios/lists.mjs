// Lot T4 — lists, rows, cards, images (docs/agents/transitions.md, L1–L15).
//
// One scenario per inventory row that has a CLICKABLE trigger, driven on the
// built SPA served by `tools/perf_api.py --serve` (8 000 tracks, a 4 000-track
// playlist), at both viewports and in the three motion levels by run.mjs:
//
//   file-retirer   L3 — ✕ on an upcoming queue row: the rows after it glide up
//   file-inserer   L4 — « Lire ensuite » from a track's menu: they make room
//   file-glisser   L5 — a grip drag: the rows between slide out of the way
//   file-vider     L6 — « Vider la file »: the rows go at once, the rest fades
//   piste-survol   L7 — hovering a track row: a colour step, never a movement
//   carte-pression L12 — a card under the finger: scale(0.98), 90 ms
//
// The rows with no labelable click of their own are held elsewhere, and named
// here so the next reader does not think they were forgotten:
//
//   L1  no intro on a windowed row — every list scenario above would catch one;
//   L2  VirtualList#shift — the API L3/L4/L6 drive;
//   L8  the swipe is a touch gesture (test/queue/run.mjs --phone drives it);
//   L9  a badge arrives from the network, not from a click;
//   L10 PlaylistTracks: `flipSmall` only ≤ 50 rows, and test/motion.test.mjs
//       holds that no component imports `svelte/animate` any more;
//   L11 TrackBrowser's results fade — no bench route mounts that component with
//       a non-empty list (the synthetic library has no local file and the bench
//       user owns every playlist, so the pages that use it are read-only
//       views); checked by hand on a build carrying a scratch page, see the PR;
//   L13 the cover's 200 ms arrival — measured below on the art already on
//       screen, because a network arrival cannot be timed by a click;
//   L14 ArtistLine is text with no motion, by rule;
//   L15 scrollBehaviour() — pinned in test/motion.test.mjs, and a reduced or eco
//       device jumps instead of gliding.
//
// `file-retirer` also reports the plan's own number (lot T4's "Done when"): the
// layouts the shift costs on a 4 000-row queue, read from
// Performance.getMetrics — measured 3 against 0 while the page idles, i.e. the
// transform never asks for one.
//
// WHAT THE RUNNER'S "identical requests" CHECK CANNOT HOLD HERE, measured:
// these scenarios' windows last seconds on a page that talks to the server by
// itself — `warmPlaylists` fires ~25 `/api/playlist/<id>` requests about ten
// seconds after each boot (lib/actions.js), a track that starts asks for its
// `/api/stream`, `/api/gains` and `/api/analyses`, and a queue mutation re-keys
// every following row so its sleeve is asked for again (the plan's own L2 note:
// `trackKey:index`). The diffs between two levels are ALWAYS those paths — never
// one a transition could add, since no transition in this lot fetches anything.
// The scenarios therefore keep the healing in `setup` (which the runner does not
// compare) and the lot's own assertions in `act`/`settle`; a per-scenario
// opt-out of the request comparison is `test/motion/run.mjs`'s call to make
// (T0/T7), not this file's.
import { silence } from "../../harness.mjs";

const WAV = silence();

/** A thrown error fails the run with its message: this file's own assertions. */
function expect(ok, message) {
  if (!ok) throw new Error(`lists.mjs: ${message}`);
}

const origin = async (page) => new URL(page.url()).origin;

async function dismiss(page) {
  // The phone's player and its queue sheet survive a hash navigation: they are
  // the page's overlay, not its route. The buttons are clicked through the DOM —
  // a pointer click is what the sheet's own children intercept.
  for (let attempt = 0; attempt < 4; attempt++) {
    const left = await page.evaluate(() => {
      document.querySelector('.m .sheet button[aria-label="Fermer"]')?.click();
      document.querySelector('.m button[aria-label="Réduire"]')?.click();
      return document.querySelectorAll(".m").length;
    });
    if (!left) return;
    await page.waitForTimeout(200);
    if (attempt === 2) await page.keyboard.press("Escape");
  }
}

async function go(page, path) {
  await dismiss(page);
  await page.goto(`${await origin(page)}/app/#${path}`);
}

/** A scenario ends with the overlays it opened CLOSED: the runner shares one
 *  page across every lot's scenarios, and the player's panel and the phone's
 *  full-screen player are toggles — leaving one open turns the next lot's
 *  "open the panel" click into a close (shell.mjs waits for it). */
async function leaveClean(page) {
  await page.evaluate(() => {
    document.querySelector('.m .sheet button[aria-label="Fermer"]')?.click();
    document.querySelector('.m button[aria-label="Réduire"]')?.click();
    document.querySelector('.np button[aria-label="Fermer"]')?.click();
  });
  await page.waitForTimeout(250);
}

const isPhone = (page) => page.evaluate(() => matchMedia("(max-width: 640px)").matches);

/** True when the app is told not to move: eco (the attribute the shell sets on
 *  <html>) or the OS's reduced-motion setting. The scenarios that assert a
 *  DISPLACEMENT only speak when motion is on; in the other two levels the
 *  runner's own checks take over (nothing in eco, nothing displacing and
 *  ≤ 140 ms under reduced motion). */
const motionOff = (page) =>
  page.evaluate(
    () =>
      document.documentElement.dataset.eco === "1" || matchMedia("(prefers-reduced-motion: reduce)").matches,
  );

const frames = (page, count = 2) =>
  page.evaluate(
    (n) =>
      new Promise((resolve) => {
        const step = () => (--n <= 0 ? resolve() : requestAnimationFrame(step));
        requestAnimationFrame(step);
      }),
    count,
  );

/** Every FINITE animation on the page has finished. The equalizer bars, the row
 *  spinner and a podcast card's `spin` are infinite by design and never count. */
const quiet = (page) =>
  page
    .waitForFunction(
      () =>
        document.getAnimations().filter((animation) => {
          const timing = animation.effect && animation.effect.getTiming();
          return !timing || timing.iterations !== Infinity;
        }).length === 0,
      null,
      { timeout: 5000, polling: 16 },
    )
    .catch(() => {});

/** One 1x1 transparent PNG: what the sleeve CDN answers in this bench. The
 *  runner ABORTS the real one, and a failed image is not cached — so every row
 *  that remounts (a keyed `<img>` in a windowed list) asks again, and the number
 *  of those requests depends on how long the window was. Stubbed, the art
 *  settles and the three levels compare like for like. */
const PIXEL = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8AAAwAB/wD/9oP8BwAAAABJRU5ErkJggg==",
  "base64",
);

/** The synthetic library's tracks have no audio: without a stream that answers,
 *  the player fails the first load and skips to the next title, moving the
 *  playhead (and the queue's rows) under the scenario. One minute of silence,
 *  exactly what test/queue/run.mjs and test/menu/run.mjs serve. */
async function stubAudio(page) {
  if (page.__audioStubbed) return;
  await page.route(/\/api\/stream\//, (route) =>
    route.fulfill({ status: 200, body: WAV, headers: { "Content-Type": "audio/wav", "Accept-Ranges": "none" } }),
  );
  // Registered AFTER the runner's abort, so it wins: a sleeve, not an error.
  await page.route(
    (url) => url.hostname === "dzcdn.net" || url.hostname.endsWith(".dzcdn.net"),
    (route) =>
      route.fulfill({
        status: 200,
        body: PIXEL,
        headers: { "Content-Type": "image/png", "Cache-Control": "public, max-age=86400" },
      }),
  );
  page.__audioStubbed = true;
}

/** The library's 8 000 titles, played from the first row: a queue long enough
 *  that only the mounted rows can possibly animate. The row's own play button
 *  is the target — the title line holds the artist LINKS, and a bench that
 *  clicks the middle of it navigates to an artist page instead of playing.
 *
 *  Idempotent on purpose: the scenarios share one page per viewport and level,
 *  so by the third one the row is already the current track and this press is a
 *  pause. The QUEUE is what the scenarios below need, and it is the same. */
async function playLibrary(page) {
  await stubAudio(page);
  await go(page, "/library/favorites");
  await page.waitForSelector(".track .play", { timeout: 30000 });
  if ((await page.locator("footer.player .info .t").count()) === 0) {
    await page.locator(".track .play").first().click();
  }
  await page.waitForFunction(
    () => {
      const name = document.querySelector("footer.player .info .t");
      return !!name && name.textContent.trim().length > 0;
    },
    null,
    { timeout: 15000 },
  );
}

/** Open the queue wherever this viewport keeps it: the desktop's third column,
 *  the phone's full-screen player and its sheet. Both doors are TOGGLES, so a
 *  scenario that follows another one must not press them again. */
async function openQueue(page) {
  if (await isPhone(page)) {
    if ((await page.locator(".m").count()) === 0) {
      await page.locator("footer.player .now").first().click();
      await page.waitForSelector(".m", { timeout: 15000 });
    }
    if ((await page.locator(".sheet .qitem").count()) === 0) {
      await page.locator('.m button[aria-label="File d\'attente"]').first().click();
    }
    await page.waitForSelector(".sheet .qitem", { timeout: 15000 });
  } else {
    if ((await page.locator(".np").count()) === 0) {
      await page.locator('button[aria-label="File d\'attente"]').first().click();
    }
    await page.waitForSelector(".np .qitem", { timeout: 15000 });
  }
  // The rows' own height has been measured by now (VirtualList#measure), so the
  // shift's `rowH` is the real one, not the 56 px estimate.
  await page.waitForTimeout(250);
  // And the mounted art has settled: lib/coverqueue.js admits covers a few at a
  // time, and a row's picture is what a window misses first.
  await coversReady(page, ".qitem .cover img");
  await quietNet(page);
}

async function queue(page) {
  await playLibrary(page);
  await openQueue(page);
}

/** One CDP session per page, kept across scenarios: a LayoutCount delta, never
 *  an absolute. A fresh session per scenario would enable and disable
 *  Performance over and over for one counter. */
const sessions = new WeakMap();
async function layouts(page) {
  let entry = sessions.get(page);
  if (!entry) {
    const cdp = await page.context().newCDPSession(page);
    await cdp.send("Performance.enable");
    entry = {
      count: async () => {
        const { metrics } = await cdp.send("Performance.getMetrics");
        return metrics.find((metric) => metric.name === "LayoutCount").value;
      },
    };
    sessions.set(page, entry);
  }
  return entry;
}

const rows = (page) =>
  page.evaluate(() =>
    [...document.querySelectorAll(".qitem")].map((row) => ({
      i: +row.dataset.qi,
      title: row.querySelector(".qt")?.textContent || "",
      up: row.classList.contains("up"),
      down: row.classList.contains("down"),
      tf: getComputedStyle(row).transform,
    })),
  );

/** What the queue's mounted rows look like ONE FRAME after a mutation: the
 *  evidence that they are gliding, taken before a screenshot could outlast the
 *  200 ms shift. */
const snapshot = (page) =>
  page.evaluate(async () => {
    await new Promise((resolve) => requestAnimationFrame(() => resolve()));
    return [...document.querySelectorAll(".qitem")].map((row) => ({
      i: +row.dataset.qi,
      title: row.querySelector(".qt")?.textContent || "",
      tf: getComputedStyle(row).transform,
    }));
  });

const queueCount = async (page) => {
  const text = (await page.locator(".qcount").first().textContent()) || "";
  return Number((text.replace(/[^\d]/g, "") || "0").slice(0, 6));
};

const rowBox = async (page, index) => page.locator(`.qitem[data-qi="${index}"]`).first().boundingBox();

/** Cover art is loaded LAZILY and by a priority queue (lib/coverqueue.js), so the
 *  set of `/api/cover` requests inside a 500 ms window depends on when the rows
 *  appeared — which is exactly what an animation changes. The runner compares
 *  the request multisets of the three levels, so a scenario about a LIST lets
 *  the art go quiet before it starts and again before it ends: what differs
 *  between full, reduced and eco is then only the motion. */
const trackers = new WeakMap();
function tracker(page) {
  let entry = trackers.get(page);
  if (!entry) {
    entry = { last: 0 };
    page.on("request", (request) => {
      const url = new URL(request.url());
      if (url.pathname.startsWith("/api/") || request.resourceType() === "image") entry.last = Date.now();
    });
    trackers.set(page, entry);
  }
  return entry;
}

/** The page has stopped talking to the server and to the CDN. Two things a
 *  scenario must not be judged against, and both are the app's own background
 *  work rather than its transitions: the playlist cache warming (`warmPlaylists`
 *  fires 25 requests about ten seconds after a boot, see lib/actions.js) and a
 *  sleeve that remounts with its keyed row. The runner compares the request
 *  multisets of the three levels, so a window that contains a warming burst says
 *  nothing about the motion. */
async function quietNet(page, quietMs = 2000, timeoutMs = 25000) {
  const { last } = tracker(page);
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    // `warmPlaylists` starts ten seconds after a boot and fires ~25 requests:
    // a quiet spell before that means nothing.
    const age = await page.evaluate(() => performance.now());
    if (age > 13000 && Date.now() - last >= quietMs) return;
    await page.waitForTimeout(120);
  }
}

/** Every `<img>` the mounted rows already hold has SETTLED (a decoded picture or
 *  a refused one — both are `complete`). Lazy loading is what makes the request
 *  window move between levels: a row translated by the shift is nearer the
 *  viewport than the same row at rest, so it asks for its art a moment earlier.
 *  Waiting for the art to settle at both ends of the scenario means every level
 *  ends with the same pictures requested — which is what the runner compares. */
async function coversReady(page, selector = ".qitem .cover img", timeoutMs = 8000) {
  await page
    .waitForFunction(
      (sel) => {
        const pictures = [...document.querySelectorAll(sel)];
        return pictures.length > 0 && pictures.every((img) => img.complete);
      },
      selector,
      { timeout: timeoutMs, polling: 100 },
    )
    .catch(() => {});
}

export const scenarios = [
  // -- L3 ---------------------------------------------------------------------
  {
    name: "file-retirer",
    midway: true,
    async setup(page) {
      await queue(page);
    },
    async act(page) {
      const metric = await layouts(page);
      // What the page costs on its own, over the same 250 ms the shift will
      // take. The equalizer used to lay out EVERY frame, which is why the raw
      // count means nothing without this baseline (it is a transform now).
      let mark = await metric.count();
      await page.waitForTimeout(250);
      await page.evaluate((value) => (window.__idle = value), (await metric.count()) - mark);
      await page.evaluate((titles) => (window.__qBefore = titles), (await rows(page)).map((row) => row.title));
      const before = await page.evaluate(() => window.__qBefore);
      expect(before.length > 4 && before[2] !== before[3], "the queue shows the rows this scenario edits");
      await page.locator('.qitem[data-qi="2"] .qx').first().click();
      // Two frames: the store, the DOM and VirtualList's own re-window have
      // happened. What is left is the shift itself.
      await frames(page, 2);
      await page.evaluate(async (value) => (window.__shiftFrom = value), await metric.count());
      await page.evaluate((titles) => (window.__qDuring = titles), await snapshot(page));
    },
    async settle(page) {
      const metric = await layouts(page);
      const during = await page.evaluate(() => window.__qDuring);
      const off = await motionOff(page);
      const moving = during.filter((row) => row.i >= 2 && row.tf !== "none");
      if (off) {
        // No displacement by rule (M14/M13): the rows land in place at once.
        expect(moving.length === 0, "nothing is displaced when motion is off");
      } else {
        expect(moving.length > 0, `the rows after a removed one are displaced (${JSON.stringify(during.slice(0, 4))})`);
      }
      // The plan's T4 number (a removal on a 4 000-row queue), measured the way
      // its prototype was: the animation's own window against the page idling for
      // the same time. LOGGED, not asserted — LayoutCount is global to the
      // renderer and this page lays out in bursts of its own (lazy art being
      // admitted by lib/coverqueue.js, the clock, a screenshot when SHOTS is
      // set), so it moves between 0 and 15 per window whatever the shift does.
      // The paired measurement (the same removal with the shift off) is in the PR.
      await page.waitForTimeout(250);
      const shift = (await metric.count()) - (await page.evaluate(() => window.__shiftFrom));
      await quiet(page);
      let mark = await metric.count();
      await page.waitForTimeout(250);
      const idleAgain = (await metric.count()) - mark;
      const idle = Math.max(await page.evaluate(() => window.__idle), idleAgain);
      console.log(
        `    file-retirer: ${moving.length} row(s) displaced, ${shift} layouts over the shift (idle ${idle})`,
      );
      await quiet(page);

      const before = await page.evaluate(() => window.__qBefore);
      const after = (await rows(page)).map((row) => row.title);
      expect(after[0] === before[0], "the playing row is untouched");
      expect(after[1] === before[1], "a row above the removed one does not move");
      expect(after[2] === before[3], `the row after the removed one took its place (${after[2]} ≠ ${before[3]})`);
      expect(!after.includes(before[2]), `the removed row is gone (${before[2]})`);
      await page.evaluate(() => {
        const scroller = document.querySelector(".np .body, .np .queue, .sheet .queue, .sheet .body");
        if (scroller) scroller.scrollTop = 0;
      });
      await frames(page, 3);
      await coversReady(page, ".qitem .cover img");
      await leaveClean(page);
    },
  },

  // -- L4 ---------------------------------------------------------------------
  // The insertion point has to be MOUNTED for anything to move, and on a phone
  // the queue's sheet covers the page it would be inserted from — so this one
  // is driven on the desktop, where the track list and the third column are on
  // screen together. The phone's insert is the swipe (L8), and it reaches the
  // same VirtualList#shift that `file-retirer` proves on both viewports.
  {
    name: "file-inserer",
    midway: true,
    async setup(page) {
      if (await isPhone(page)) return;
      await queue(page);
    },
    async act(page) {
      if (await isPhone(page)) return;
      await page.evaluate(async () => (window.__qCount = document.querySelector(".qcount")?.textContent || ""));
      // The queue is 8 000 titles long: the panel shows the playing row and the
      // ones right after it, and « Lire ensuite » inserts at index 1, in sight.
      await page.mouse.move(2, 2);
      const row = page.locator(".track").nth(4);
      await row.hover();
      await row.locator(".more").click();
      await page
        .locator(".menu[role=menu] [role=menuitem]", { hasText: "Lire ensuite" })
        .first()
        .click();
      await page.evaluate((titles) => (window.__qDuring = titles), await snapshot(page));
    },
    async settle(page) {
      if (await isPhone(page)) return;
      const during = await page.evaluate(() => window.__qDuring);
      const moving = during.filter((row) => row.i >= 1 && row.tf !== "none");
      if (await motionOff(page)) {
        expect(moving.length === 0, "nothing is displaced when motion is off");
      } else {
        expect(moving.length > 0, `the rows below an insertion make room (${JSON.stringify(during.slice(0, 4))})`);
      }
      await quiet(page);
      // The header counts what is left of the queue: it grew by exactly one.
      const was = Number((await page.evaluate(() => window.__qCount)).replace(/[^\d]/g, ""));
      const now = await queueCount(page);
      expect(now === was + 1, `the queue gained exactly one title (${was} → ${now})`);
      const first = await rows(page);
      expect(first[1]?.title?.length > 0, "the inserted row sits at index 1");
      await coversReady(page, ".qitem .cover img");
      await leaveClean(page);
    },
  },

  // -- L5 ---------------------------------------------------------------------
  {
    name: "file-glisser",
    midway: true,
    async setup(page) {
      await queue(page);
    },
    async act(page) {
      const grip = await page.locator('.qitem[data-qi="1"] .grip').first().boundingBox();
      await page.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2);
      await page.mouse.down();
      const target = await rowBox(page, 4);
      await page.mouse.move(grip.x + grip.width / 2, target.y + target.height / 2, { steps: 8 });
      await page.evaluate(async () => {
        await new Promise((resolve) => requestAnimationFrame(() => resolve()));
      });
    },
    async settle(page) {
      // The rows between the held row and its landing place slid one row out of
      // the way as the drag crossed them: the list MAKES ROOM, no line-only cue.
      const during = await rows(page);
      const up = during.filter((row) => row.up);
      expect(up.length > 0, `the rows between make room for a downward drag (${JSON.stringify(during.slice(0, 5))})`);
      expect(during.filter((row) => row.down).length === 0, "no row is pushed the wrong way");
      expect(
        up.every((row) => row.i > 1 && row.i <= 4),
        `the displaced rows are exactly the ones in between (${up.map((row) => row.i).join(",")})`,
      );
      const duration = await page.evaluate(() => {
        const row = document.querySelector(".qitem.up");
        return row ? getComputedStyle(row).transitionDuration : "";
      });
      if (await motionOff(page)) {
        // M14: no displacement ANIMATES. The rows take their place at once, and
        // eco's guard has already zeroed the transition.
        expect(parseFloat(duration) * 1000 < 1, `no make-room glide when motion is off (${duration})`);
      } else {
        expect(parseFloat(duration) > 0, `the make-room glide is a token, not a jump (${duration || "none"})`);
      }
      await page.mouse.up();
      await quiet(page);
      const left = await page.evaluate(() => document.querySelectorAll(".qitem.up, .qitem.down").length);
      expect(left === 0, "on drop the transforms clear in the same frame as the move");
      await coversReady(page, ".qitem .cover img");
      await leaveClean(page);
    },
  },

  // -- L6 ---------------------------------------------------------------------
  {
    name: "file-vider",
    midway: true,
    async setup(page) {
      await queue(page);
    },
    async act(page) {
      await page.locator(".qclear").first().click();
    },
    async settle(page) {
      // The rows are gone in the same frame as the store (no row outro), and what
      // is left of the queue — the playing row — fades back in over 140 ms.
      const left = await rows(page);
      expect(left.length === 1, `only the playing row is left (${left.length})`);
      expect(left[0].i === 0 && left[0].title.length > 0, "and that row is the one playing");
      const header = await page.locator(".qhead").count();
      expect(header === 0, "the header goes with the titles it counted");
      await quiet(page);
      await coversReady(page, ".qitem .cover img");
      await leaveClean(page);
    },
  },

  // -- L7 ---------------------------------------------------------------------
  {
    name: "piste-survol",
    async setup(page) {
      await go(page, "/library/favorites");
      await page.waitForSelector(".track", { timeout: 30000 });
    },
    async act(page) {
      const box = await page.locator(".track").first().boundingBox();
      await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
      await page.evaluate(async () => {
        await new Promise((resolve) => requestAnimationFrame(() => resolve()));
      });
    },
    async settle(page) {
      const step = await page.evaluate(() => {
        const title = document.querySelector(".track .t");
        const style = getComputedStyle(title);
        return { duration: style.transitionDuration, properties: style.transitionProperty };
      });
      // ≤ 140 ms and a COLOUR: a row never moves under the pointer (M6/L7).
      const durations = step.duration.split(",").map((value) => parseFloat(value) * 1000);
      expect(
        durations.every((ms) => ms <= 140),
        `a track row's hover step stays under 140 ms (${step.duration})`,
      );
      expect(/color|opacity/.test(step.properties), `and it is a colour step (${step.properties})`);
      await page.mouse.move(2, 2);
      await quiet(page);
      await coversReady(page, ".qitem .cover img");
      await leaveClean(page);
    },
  },

  // -- L12 --------------------------------------------------------------------
  {
    name: "carte-pression",
    async setup(page) {
      await go(page, "/library/albums");
      await page.waitForSelector(".card", { timeout: 30000 });
      await page.waitForTimeout(300);
      await coversReady(page, ".card .cover img");
      await leaveClean(page);
      await quietNet(page);
    },
    async act(page) {
      // `hover()` scrolls the card into view first: a coordinate below the fold
      // is a press on nothing, and the page keeps the scroll of the scenario
      // before it.
      const card = page.locator(".card").first();
      await card.hover();
      const box = await card.boundingBox();
      await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
      await page.mouse.down();
      await page.evaluate(async () => {
        await new Promise((resolve) => requestAnimationFrame(() => resolve()));
      });
    },
    async settle(page) {
      // The press transition is 90 ms: let it land, then read the held card.
      await quiet(page);
      const pressed = await page.evaluate(() => {
        const cards = [...document.querySelectorAll(".card")];
        const card = cards.find((element) => element.matches(":active"));
        if (!card) return { active: false, transform: null, duration: 0, index: -1 };
        const style = getComputedStyle(card);
        const duration = style.transitionDuration.split(",").map((value) => parseFloat(value) * 1000);
        return { transform: style.transform, duration: Math.max(...duration), active: true, index: cards.indexOf(card) };
      });
      expect(pressed.active, "the card under the pointer is the pressed one");
      // scale(0.98): a press, not a lift — and 90 ms, the frequent-things budget.
      const scale = /matrix\(([\d.]+)/.exec(pressed.transform);
      expect(scale && Math.abs(parseFloat(scale[1]) - 0.98) < 0.005, `a pressed card is at 0.98 (${pressed.transform})`);
      expect(pressed.duration <= 140, `the press's step stays under 140 ms (${pressed.duration})`);
      // The cover's arrival is the tokens' 200 ms at full motion (L13), read
      // here on the art already on screen (a network arrival cannot be timed by
      // a click). Eco and reduced replace it with the guard's own numbers.
      if (!(await motionOff(page))) {
        const cover = await page.evaluate(() => {
          const img = document.querySelector(".cover img:not(.low)");
          return img ? getComputedStyle(img).transitionDuration : "";
        });
        if (cover) expect(parseFloat(cover) * 1000 === 200, `a cover's arrival is 200 ms (${cover})`);
      }
      // Release off the card: a click here would navigate, and this scenario is
      // the press, not the page.
      await page.mouse.move(2, 2);
      await page.mouse.up();
      await quiet(page);
      await coversReady(page, ".card .cover img");
      await leaveClean(page);
    },
  },
];
