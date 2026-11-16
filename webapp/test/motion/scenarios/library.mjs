// Lot T6b — bibliothèque et pages (docs/agents/transitions.md, inventory I11).
//
// One scenario per row of I11 that HAS a clickable trigger:
//
//   · the segmented control (I3)                    → « bibliothèque · monde »
//   · the leaf strip (I3) + the tab content (I2/I1) → « bibliothèque · feuille »
//   · a group of playlists (I4)                     → « bibliothèque · groupe »
//   · the playlist edit mode's glyph swap           → « playlist · modifier »
//   · the artist's tabs (I2/I3)                     → « artiste · onglets »
//   · History's filters                             → « historique · filtre »
//
// (I1 — skeleton/LoadFailed → content — has no click of its own: the leaf
// scenario crosses it, and `viewKey` in routes/Library.svelte makes the panel
// and the data arrival ONE fade, never two.)
//
// test/motion/run.mjs runs every scenario on 1440×900 and 390×844, at full
// motion, under `prefers-reduced-motion: reduce`, and in eco, and holds it to
// the plan's generic rules (same /api and image requests in the three levels,
// nothing running in eco outside `[data-motion="status"]`, no transform and no
// duration over 140 ms under reduced motion).
//
// Four things this file has to know about the tree it runs in:
//
//  · every scenario starts on a FRESH page (`boot`). `lib/actions.js#warmPlaylists`
//    starts warming the offline cache 10 s after boot and then pulls one
//    playlist every 1.5 s — a background job, correctly, but a request in the
//    comparison window that depends on wall-clock time rather than on the
//    click. A page per scenario keeps every window under that 10 s, and makes
//    each scenario independent of the previous one. Re-loading loses the
//    bench's injected `window.__motion`, so this file re-installs the same
//    three helpers (frames/running/describe).
//  · `#/library` and `#/library/:tab` are two ROUTES in App.svelte: leaving the
//    bare `/library` for a leaf remounts the page, and no transition can play on
//    an element that did not exist yet. Every setup lands on a `/library/<leaf>`
//    URL first, which is what a deep link does too.
//  · the measurement is taken INSIDE the page, one frame after the click: a
//    Playwright click's round trip is already 40–80 ms, and a 140 ms fade read
//    from Node lands on the wrong side of its end. `act` performs the click and
//    samples the timeline at frames 1, 12 and 24 (the click's fade and, for a
//    leaf, the fade of the data arriving right behind it).
//  · sleeve fades (`Cover.svelte`, 0.35 s, lot T4's L13) and the skeleton
//    shimmer are OTHER lots' animations, still running under reduced motion on
//    this branch. This lot judges only its own (the MINE filter), then waits for
//    the tree to settle so the runner's global checks judge the app rather than
//    another lot's leftovers.
//
// Numbers: `settle` prints, per scenario × viewport × level, the animations
// this lot owns (duration + animated properties) and the LayoutCount the change
// cost.

/** The bench's three helpers, re-installed after a reload (see `boot`). */
function HELPERS() {
  window.__motion = {
    frames: (n = 2) =>
      new Promise((resolve) => {
        const step = () => (--n <= 0 ? resolve() : requestAnimationFrame(step));
        requestAnimationFrame(step);
      }),
    running() {
      return document.getAnimations().filter((animation) => {
        const target = animation.effect && animation.effect.target;
        return !(target && target.closest && target.closest("[data-motion]"));
      });
    },
    describe(animation) {
      const effect = animation.effect;
      const target = effect && effect.target;
      const keyframes = effect && effect.getKeyframes ? effect.getKeyframes() : [];
      return {
        name: (target && target.className && target.className.toString()) || (target && target.tagName) || "?",
        probe: false,
        duration: Math.round(effect ? effect.getTiming().duration : 0),
        keys: keyframes.flatMap((frame) =>
          Object.keys(frame).filter((key) => !/^(offset|easing|composite|computedOffset)$/.test(key)),
        ),
      };
    },
  };
}

/** Let `n` frames pass inside the page. */
const frames = (page, n = 2) => page.evaluate((count) => window.__motion.frames(count), n);

/**
 * One JSON answer from the app's own API, through the CONTEXT's request client:
 * a page `fetch` right after a reload can race the service worker's takeover
 * ("Failed to fetch"), and this reader is not the app under test.
 */
async function api(page, path) {
  let status = 0;
  for (let attempt = 0; attempt < 3; attempt++) {
    const response = await page.request.get(new URL(path, page.url()).href);
    status = response.status();
    if (response.ok()) return response.json();
    await page.waitForTimeout(300);
  }
  throw new Error(`${path} answered ${status}`);
}

/** A fresh app on `/app/`, with the bench's helpers back in place. */
async function boot(page) {
  await page.evaluate(() => {
    location.hash = "";
  });
  await page.reload({ timeout: 60000 });
  await page.waitForSelector("footer.player, .mobilenav, main", { timeout: 60000 });
  await page.evaluate(HELPERS);
  await frames(page, 3);
}

/**
 * Click `selector` (the first element whose text contains `text`, when given)
 * INSIDE the page, then sample the timeline one frame later and twice more.
 */
async function act(page, { selector, text = null }) {
  const sample = await page.evaluate(
    async ({ selector, text }) => {
      const all = [...document.querySelectorAll(selector)];
      const el = text ? all.find((node) => (node.textContent || "").trim().includes(text)) : all[0];
      if (!el) throw new Error(`no element for ${selector}${text ? ` / ${text}` : ""}`);
      const samples = [];
      const take = () => {
        for (const animation of document.getAnimations()) {
          const effect = animation.effect;
          const target = effect && effect.target;
          const described = {
            name: (target && target.className && String(target.className)) || (target && target.tagName) || "?",
            duration: Math.round(effect ? effect.getTiming().duration : 0),
            keys:
              effect && effect.getKeyframes
                ? effect
                    .getKeyframes()
                    .flatMap((frame) =>
                      Object.keys(frame).filter((key) => !/^(offset|easing|composite|computedOffset)$/.test(key)),
                    )
                : [],
          };
          const id = `${described.name}|${described.duration}|${described.keys.join("+")}`;
          if (!samples.some((entry) => entry.id === id)) samples.push({ id, ...described });
        }
      };
      el.click();
      for (const stop of [1, 12, 24]) {
        let n = 0;
        while (n < stop) {
          await new Promise((resolve) => requestAnimationFrame(resolve));
          n++;
        }
        take();
      }
      return {
        samples: samples.map(({ id, ...rest }) => rest),
        eco: document.documentElement.dataset.eco === "1",
        reduced: window.matchMedia("(prefers-reduced-motion: reduce)").matches,
      };
    },
    { selector, text },
  );
  lastSample = sample;
  return sample;
}

/** A CDP session per page, for Performance.getMetrics (LayoutCount). */
const sessions = new WeakMap();
async function cdp(page) {
  let session = sessions.get(page);
  if (session) return session;
  try {
    session = await page.context().newCDPSession(page);
    await session.send("Performance.enable");
  } catch {
    session = null; // not Chromium: the layout figure is simply not reported
  }
  sessions.set(page, session);
  return session;
}

async function layouts(page) {
  const session = await cdp(page);
  if (!session) return null;
  const { metrics } = await session.send("Performance.getMetrics");
  return metrics.find((metric) => metric.name === "LayoutCount")?.value ?? null;
}

/** The baseline for the layout delta, taken at the end of `setup`. */
let baseline = null;
/** What `act` sampled (the runner only hands `page` to `settle`). */
let lastSample = null;
let clicks = 0;
async function mark(page) {
  baseline = await layouts(page);
}

/** The classes this lot animates (another lot's target is not judged here). */
const MINE = /(^|\s)(lib-pane|lib-podcasts|cat-pill|subtab|sec-chevron|pl-sec-body|recs|rec-list|tab-pane|pc-body|mode-glyph)(\s|$)/;

const MOVEMENT = /^(transform|translate|scale|rotate)$/;

/** The lot's numbers + its own level checks, printed once per run. */
async function report(page, label) {  const sample = lastSample;
  lastSample = null;
  const after = await layouts(page);
  const delta = baseline === null || after === null ? null : after - baseline;
  baseline = null;
  clicks++;

  const mine = sample.samples.filter((animation) => MINE.test(animation.name));
  const others = sample.samples.filter((animation) => !MINE.test(animation.name));
  const list = mine.map((a) => `${a.name.split(" ")[0]}:${a.duration}ms[${a.keys.join("+")}]`);
  console.log(
    `    · #${clicks} ${label}: mine ${JSON.stringify(list)}; ` +
      `${others.length} other animation(s); layouts ${delta}`,
  );

  if (sample.eco) {
    // T0's own contract: eco leaves a 0 ms remnant (the guard writes 0.01 ms) and
    // nothing after it. A remnant is no motion.
    const moving = mine.filter((animation) => animation.duration > 0);
    if (moving.length) throw new Error(`${label}: eco still animates ${JSON.stringify(moving)}`);
  }
  if (sample.reduced) {
    for (const animation of mine) {
      if (animation.duration > 140)
        throw new Error(`${label}: ${animation.name} runs ${animation.duration} ms under reduced motion`);
      const moved = animation.keys.filter((key) => MOVEMENT.test(key));
      if (moved.length) throw new Error(`${label}: ${animation.name} carries ${moved.join(",")} under reduced motion`);
    }
  }
  return mine;
}

/**
 * Wait until the page has stopped moving and fetching: no animation outside the
 * bench's own probes, and the page's resource count no longer changes. Bounded
 * and spoken, never waited on forever.
 */
async function settleTree(page, limit = 8000) {
  const started = Date.now();
  let streak = 0;
  let last = "";
  while (Date.now() - started < limit) {
    const state = await page.evaluate(() => ({
      running: window.__motion ? window.__motion.running().length : document.getAnimations().length,
      fetched: performance.getEntriesByType("resource").length,
    }));
    const print = `${state.running}|${state.fetched}`;
    streak = state.running === 0 && print === last ? streak + 1 : 0;
    last = print;
    if (streak >= 6) return true;
    await frames(page, 2);
    await page.waitForTimeout(80);
  }
  const left = await page.evaluate(() => window.__motion.running().map(window.__motion.describe));
  console.log(`    ! not settled after ${limit} ms: ${JSON.stringify(left)}`);
  return false;
}

const LIBRARY = "#/library/favorites";

export const scenarios = [
  {
    // I3 — the segmented control: one accent pill moved by `transform` (200 ms,
    // --ease-standard), the labels' colour following (M6). The podcast grid is
    // MOUNTED ONCE across a world switch, so its appearance is an imperative
    // `animate()` (opacity only) — this scenario crosses both.
    name: "bibliothèque · monde",
    async setup(page) {
      await boot(page);
      await page.evaluate((hash) => (location.hash = hash), LIBRARY);
      await page.waitForSelector("#lib-cat-podcasts");
      await settleTree(page);
      await mark(page);
    },
    act: (page) => act(page, { selector: "#lib-cat-podcasts" }),
    async settle(page) {
      await report(page, "monde Musique → Podcasts");
      await settleTree(page);
    },
  },
  {
    // I3/I2/I1 — the leaf strip's underline (transform) and the panel's fade
    // (opacity only, 140 ms). The artists list arrives after the switch: I1
    // rides in the SAME fade (the panel's identity is `tab:loaded`).
    name: "bibliothèque · feuille",
    async setup(page) {
      await boot(page);
      await page.evaluate((hash) => (location.hash = hash), LIBRARY);
      await page.waitForSelector("#lib-leaf-artists");
      await settleTree(page);
      await mark(page);
    },
    act: (page) => act(page, { selector: "#lib-leaf-artists" }),
    async settle(page) {
      await report(page, "feuille Titres → Artistes");
      await settleTree(page);
    },
  },
  {
    // I4 — a group of playlists. A folder's grid is taller than M10 allows, so
    // the body takes I4's other branch: it appears with `fadeIn` (no height
    // walk, no layout per frame), and the chevron ROTATES (200 ms) instead of
    // swapping glyphs. The bench library has no folders, so the first group is
    // « Sans dossier » — the same component, the same toggle.
    name: "bibliothèque · groupe",
    async setup(page) {
      await boot(page);
      await page.evaluate(() => (location.hash = "#/library/playlists"));
      await page.waitForSelector(".pl-section .sec-toggle", { timeout: 30000 });
      // Closed before the measurement, so the click OPENS it.
      if ((await page.getAttribute(".pl-section .sec-toggle", "aria-expanded")) === "true") {
        await page.evaluate(() => document.querySelector(".pl-section .sec-toggle").click());
        await settleTree(page);
      }
      await mark(page);
    },
    act: (page) => act(page, { selector: ".pl-section .sec-toggle" }),
    async settle(page) {
      await report(page, "groupe fermé → ouvert");
      await settleTree(page);
    },
  },
  {
    // The playlist's edit mode: the glyph SWAPS (140 ms, opacity + 4 px) in the
    // same frame as the state; the list itself is the answer (M2/M7) and gets
    // no entrance — a 4 000-row edit list never animates (M4).
    name: "playlist · modifier",
    async setup(page) {
      await boot(page);
      const payload = await api(page, "/api/me/playlists");
      const list = payload.playlists || [];
      const pick = list.find((p) => (p.nb_tracks || 0) > 10 && (p.nb_tracks || 0) < 200) || list[0];
      const id = pick && pick.id;
      if (!id) throw new Error("the bench library has no playlist");
      await page.evaluate((playlist) => (location.hash = "#/playlist/" + playlist), String(id));
      await page.waitForSelector(".actions .btn", { timeout: 30000 });
      await settleTree(page);
      await mark(page);
    },
    act: (page) => act(page, { selector: ".actions .btn", text: "Modifier" }),
    async settle(page) {
      await report(page, "modifier (entrée en édition)");
      await settleTree(page);
    },
  },
  {
    // I2/I3 — an artist's tabs are in-screen state on the same page key, so the
    // tab panel is the motion: one keyed block, opacity only (the tabs differ
    // in height), and the chip's colour (M6).
    name: "artiste · onglets",
    async setup(page) {
      await boot(page);
      const payload = await api(page, "/api/me/artists");
      const id = (payload.artists || [])[0]?.id || null;
      if (!id) throw new Error("the bench library has no followed artist");
      await page.evaluate((artist) => (location.hash = "#/artist/" + artist), String(id));
      await page.waitForSelector(".tabs .chip", { timeout: 30000 });
      await page.evaluate(() => {
        const chip = [...document.querySelectorAll(".tabs .chip")].find((node) =>
          (node.textContent || "").includes("Aperçu"),
        );
        if (chip && chip.getAttribute("aria-selected") !== "true") chip.click();
      });
      await settleTree(page);
      await mark(page);
    },
    act: (page) => act(page, { selector: ".tabs .chip", text: "Titres" }),
    async settle(page) {
      await report(page, "onglet Aperçu → Titres");
      await settleTree(page);
    },
  },
  {
    // History has NO motion on its day groups (M4: a long windowed list); the
    // only clickable thing there is the filter chip — colour only, and its
    // answer (one page of events) arrives in the list's own fade.
    name: "historique · filtre",
    async setup(page) {
      await boot(page);
      await page.evaluate(() => (location.hash = "#/library/history"));
      await page.waitForSelector(".history .filters .chip", { timeout: 30000 });
      await settleTree(page);
      await mark(page);
    },
    act: (page) => act(page, { selector: ".history .filters .chip", text: "Écoutes" }),
    async settle(page) {
      await report(page, "filtre Tout → Écoutes");
      await settleTree(page);
    },
  },
];
