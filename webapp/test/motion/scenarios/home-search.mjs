// LOT T6a — Accueil & Explorer: one scenario per row of inventory I10
// (docs/agents/transitions.md) that has a trigger this bench can pull.
//
// The runner (test/motion/run.mjs) walks every scenario in the three motion
// levels (full, reduced, eco) at 1440×900 and 390×844 and already holds the
// general rules: nothing animates in eco, nothing displaces under reduced
// motion, and the three levels ask for the same requests. This file adds what
// only the HOME and the SEARCH screen can say:
//
//   - the sections of the home enter when their answer arrives (I6) and nothing
//     animates per card;
//   - the hero's discovery panel enters (I10, "suggestion swap");
//   - "Reprendre" replaces its idle block with the track block (M7);
//   - the results area enters ONCE per visit and NOT again when a new term
//     replaces the results behind the same area (I10 + M6 — the clause this
//     lot exists for);
//   - a tab switch fades its panel in (I2) and the tablist steps its colour in
//     `--dur-instant`, never a sliding measurement (I3);
//   - a new best result enters as a NEW card, keyed on the entity (I10);
//   - the query-radio offer rises (I10).
//
// HOW IT MEASURES: `Element.prototype.animate` is wrapped once per page, so
// every intro Svelte starts (a transition is WAAPI) is LOGGED with its target,
// duration and keyframes — an event, not a clock. A duration-0 entrance takes
// Svelte's instant path and never calls `animate`, which is exactly how "eco
// animates nothing" and "a replacement inside the area does not re-enter" are
// proven: the count is 0.
//
// THE FIXTURE: tools/perf_api.py's synthetic library is neither archived nor
// measured, so `/api/search` (which only lists playable, on-disk rows) finds
// nothing, `/api/moods` has no mix and `/api/recap/years` has no year. The
// search answer is therefore STUBBED here, the way test/menu/run.mjs stubs it:
// the bench asks what the screen does with an answer, not what the provider
// says. Every payload is derived from the query, so two different terms really
// are two different identities. `MoodCard` and `RecapHomeCard` cannot be
// exercised on this fixture at all (no mood is ready, no year is seeded); their
// rows of I10 are covered by the code and by the 3-level rules the runner
// applies to every scenario that touches their section.
//
// The stubs are added in `setup` and removed in `settle`, so a scenario of
// another lot that runs after this file never sees them.

import assert from "node:assert/strict";
import { silence } from "../../harness.mjs";

// -- the page probes ---------------------------------------------------------

/** Wraps `Element.prototype.animate` once, so every Svelte entrance is a log
 *  line (target, duration, keyframe properties). */
function INSTRUMENT() {
  if (window.__animLog) return;
  window.__animLog = [];
  const native = Element.prototype.animate;
  Element.prototype.animate = function (frames, options) {
    const first = (Array.isArray(frames) ? frames[0] : frames) || {};
    window.__animLog.push({
      at: performance.now(),
      cls: typeof this.className === "string" ? this.className : "",
      tag: this.tagName,
      duration: typeof options === "object" && options ? Number(options.duration) : null,
      keys: Object.keys(first).filter((k) => !/^(offset|easing|composite|computedOffset)$/.test(k)),
    });
    return native.call(this, frames, options);
  };
}

async function install(page) {
  await page.evaluate(INSTRUMENT);
}

/** "off" | "reduced" | "full", read from the page the way lib/motion.js does. */
function motionLevel(page) {
  return page.evaluate(() =>
    document.documentElement.dataset.eco === "1"
      ? "eco"
      : matchMedia("(prefers-reduced-motion: reduce)").matches
        ? "reduced"
        : "full",
  );
}

/** Marks the instant the act begins: everything logged after it belongs to it. */
function markStart(page) {
  return page.evaluate(() => {
    window.__t0 = performance.now();
  });
}

async function logged(page, match) {
  const all = await page.evaluate(() =>
    window.__animLog.filter((entry) => entry.at >= (window.__t0 || 0)),
  );
  return all.filter((entry) => match.test(entry.cls));
}

/** The three levels, asserted per entrance we asked for: full and reduced play
 *  it (opacity only under reduced, ≤ 200 ms), eco plays nothing at all. */
function expectEntrance(level, entries, label) {
  if (level === "eco") {
    assert.equal(entries.length, 0, `${label}: nothing animates in eco — got ${JSON.stringify(entries)}`);
    return;
  }
  assert.ok(entries.length >= 1, `${label}: the entrance plays (${level}) — got ${JSON.stringify(entries)}`);
  for (const entry of entries) {
    assert.ok(entry.duration <= 200, `${label}: ${entry.duration} ms is under the 200 ms cap`);
    assert.ok(entry.keys.includes("opacity"), `${label}: it fades (opacity in ${JSON.stringify(entry.keys)})`);
    if (level === "reduced") {
      const moved = entry.keys.filter((key) => /^(transform|translate|scale|rotate)$/.test(key));
      assert.equal(moved.length, 0, `${label}: no displacement under reduced motion — ${JSON.stringify(moved)}`);
    }
  }
}

// -- the stubs ---------------------------------------------------------------

const isSearch = (url) => url.pathname === "/api/search";
const isStream = (url) => url.pathname.startsWith("/api/stream/");
const WAV = silence(60);

/** The answer a library WITH matches would give for `q`; names AND ids carry
 *  the query, which is what makes one best result really give way to another
 *  (an identity is the entity's own ref, not its name). */
function searchPayload(q) {
  const slug = q.toLowerCase().replace(/[^a-z0-9]+/g, "-");
  const person = (n) => ({ deezer_id: `stub-${slug}-art-${n}`, name: `${q} ${n}`, picture: null });
  const album = (n) => ({
    deezer_id: `stub-${slug}-alb-${n}`,
    title: `${q} Album ${n}`,
    cover: null,
    artist: { deezer_id: `stub-${slug}-art-123`, name: `${q} 123` },
    nb_tracks: 4,
    year: "2026",
  });
  return {
    artists: [person(123), person(218), person(44), person("encore 233")],
    albums: [0, 1, 2, 3, 4, 5].map(album),
    tracks: Array.from({ length: 40 }, (_, i) => ({
      deezer_id: `stub-${slug}-trk-${i}`,
      title: `${q} Titre ${i + 1}`,
      duration: 180 + i,
      artist: { deezer_id: `stub-${slug}-art-123`, name: `${q} 123` },
      album: { deezer_id: `stub-${slug}-alb-0`, title: `${q} Album 0`, cover: null },
    })),
    playlists: [0, 1, 2].map((n) => ({
      deezer_id: `stub-${slug}-pl-${n}`,
      title: `${q} Playlist ${n}`,
      cover: null,
      nb_tracks: 12,
      owner: "bench",
    })),
    podcasts: [],
  };
}

async function stubSearch(page) {
  await page.route(isSearch, (route) => {
    const q = new URL(route.request().url()).searchParams.get("q") || "";
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(searchPayload(q)),
    });
  });
}

/** The synthetic library has no audio files: without this the first load fails
 *  and the player skips through the queue mid-scenario (test/menu/run.mjs's
 *  WAV, the same one-minute of silence). */
async function stubStream(page) {
  await page.route(isStream, (route) =>
    route.fulfill({
      status: 200,
      body: WAV,
      headers: { "Content-Type": "audio/wav", AcceptRanges: "none" },
    }),
  );
}

// -- navigation helpers ------------------------------------------------------

/** Follows a nav link with a real DOM click. The bench pins its own probe
 *  button over the phone's tab bar (bottom-left, `z-index` last), so a mouse
 *  click aimed at « Accueil » there lands on the probe: `element.click()` runs
 *  exactly the handlers the browser would (the SPA's `link` action listens for
 *  `click`) and the same state change follows. */
async function navTo(page, href) {
  const found = await page.evaluate((target) => {
    const link = [...document.querySelectorAll(`nav a[href="#${target}"]`)].find((a) => {
      const box = a.getBoundingClientRect();
      return box.width > 0 && box.height > 0;
    });
    if (!link) return false;
    link.click();
    return true;
  }, href);
  assert.ok(found, `a visible nav link to ${href} is on screen`);
}

async function openSearch(page) {
  // A FRESH visit every time: the screen remembers its tab and its answers for
  // a Back (`recallScreen`), and a click on the nav item you are already on is
  // a no-op — so going through the home is what gives each scenario its own
  // visit, hence its own entrances to measure.
  await navTo(page, "/");
  await page.waitForSelector(".home .hero", { timeout: 20000 });
  await navTo(page, "/search");
  await page.waitForSelector('input[type="search"]', { timeout: 20000 });
}

/** Types a term and returns once the server ANSWERED it (an event, not a
 *  clock): the screen is then settled and the entrance it played is logged. */
async function search(page, term) {
  const box = page.locator('input[type="search"]').first();
  const answered = page.waitForResponse(
    (response) => response.url().includes("/api/search?") && decodeURIComponent(response.url()).includes(`q=${term}`),
    { timeout: 20000 },
  );
  await box.fill(term);
  await box.press("Enter");
  await answered;
  await page.evaluate(() => window.__motion.frames(3));
}

/** Ends a scenario on a quiet page. Two things otherwise make the three levels
 *  disagree: the cover queue is time-sliced (six in flight, a frame between
 *  batches), so a scenario that ends mid-batch has asked for a different set of
 *  covers at each level; and an instant transition (eco's guard is 0.01 ms) is
 *  on the Web Animations timeline for the frame it starts in, which the runner
 *  reads as "something still animates". So this waits, FRAME BY FRAME (an
 *  event, not a clock), until no image is loading and no instant animation is
 *  left — and gives up after 240 frames rather than ever hanging the bench. */
async function finish(page) {
  await page.evaluate(async () => {
    const frame = () => new Promise((resolve) => requestAnimationFrame(resolve));
    const quiet = () => {
      const pending = [...document.images].some((img) => img.isConnected && !img.complete);
      const instant = document
        .getAnimations()
        .some((animation) => animation.effect && animation.effect.getTiming().duration < 0.1);
      return !pending && !instant;
    };
    for (let i = 0; i < 240; i++) {
      if (quiet()) {
        // One more frame, so a decode that landed on this frame is seen too.
        await frame();
        if (quiet()) return;
      }
      await frame();
    }
  });
}

// -- the scenarios -----------------------------------------------------------

export const scenarios = [
  {
    // I6 — the home's shelves are a block that ENTERS when its answer arrives,
    // never a card at a time (M11). Coming back to the home remounts it and
    // refetches, so the entrance really is the data arriving after mount.
    name: "accueil: sections après le chargement",
    midway: true,
    async setup(page) {
      await install(page);
      await navTo(page, "/library");
      await page.waitForSelector("main h1", { timeout: 20000 });
    },
    async act(page) {
      await markStart(page);
      await navTo(page, "/");
    },
    async settle(page) {
      await page.waitForSelector(".home .releases .rel", { timeout: 20000 });
      const level = await motionLevel(page);
      expectEntrance(level, await logged(page, /(^|\s)releases(\s|$)/), "accueil: Nouveautés");
      // No per-card entrance: the six sleeves are inside the section's one fade.
      const cards = await logged(page, /(^|\s)rel(\s|$)/);
      assert.equal(cards.length, 0, "accueil: the cards do not animate one by one (M11)");
      await finish(page);
    },
  },
  {
    // I10, "HomeHero carousel/suggestion swap": the discovery panel enters
    // (M7: it replaces nothing, and it goes at once when it closes).
    name: "accueil: panneau de découverte",
    midway: true,
    async setup(page) {
      await install(page);
      await navTo(page, "/");
      await page.waitForSelector(".hero .tune", { timeout: 20000 });
    },
    async act(page) {
      await markStart(page);
      await page.locator(".hero .tune").first().click();
    },
    async settle(page) {
      await page.waitForSelector(".hero-wrap .tune-panel", { timeout: 20000 });
      const level = await motionLevel(page);
      expectEntrance(level, await logged(page, /tune-panel/), "héros: panneau de découverte");
      // Closed again: the panel goes at once (in: only, no outro holds it).
      await page.locator(".hero .tune").first().click();
      await page.waitForSelector(".hero-wrap .tune-panel", { state: "detached", timeout: 5000 });
      await finish(page);
    },
  },
  {
    // M7 — "Reprendre" swaps its idle block for the track block in place. The
    // trigger is a real play: the first Nouveautés card queues its release.
    name: "accueil: reprendre remplace le bloc vide",
    midway: true,
    async setup(page) {
      await install(page);
      await stubStream(page);
      await navTo(page, "/");
      await page.waitForSelector(".home .releases .rel", { timeout: 20000 });
      await page.waitForSelector(".resume .idle", { timeout: 20000 });
    },
    async act(page) {
      await markStart(page);
      await page.locator(".home .releases .rel").first().click();
    },
    async settle(page) {
      await page.waitForSelector(".resume .body", { timeout: 20000 });
      await page.evaluate(() => window.__motion.frames(3));
      const level = await motionLevel(page);
      expectEntrance(level, await logged(page, /(^|\s)body(\s|$)/), "reprendre: le titre entre");
      assert.equal(await page.locator(".resume .idle").count(), 0, "reprendre: the idle block is gone (M7)");
      // The next scenarios must find a quiet player: pause it through the bar's
      // own control (a playing glyph has no triangle).
      await page.evaluate(() => {
        const button = document.querySelector("footer.player .pp, .mini .pp");
        if (button && !button.querySelector("svg polygon")) button.click();
      });
      await finish(page);
      await page.unroute(isStream);
    },
  },
  {
    // I10 + M6 — the results area enters ONCE per visit; the next term (which
    // re-creates it behind the Skeleton branch) must NOT fade the list again.
    name: "explorer: les résultats entrent une seule fois",
    midway: true,
    async setup(page) {
      await install(page);
      await stubSearch(page);
      await openSearch(page);
    },
    async act(page) {
      await markStart(page);
      await search(page, "Amour");
    },
    async settle(page) {
      await page.waitForSelector(".leadwrap", { timeout: 20000 });
      const level = await motionLevel(page);
      expectEntrance(level, await logged(page, /leadwrap/), "explorer: la zone de résultats");
      // The shelves that came with the answer enter with it — one block each.
      expectEntrance(level, await logged(page, /shelf-sec/), "explorer: les rayons de résultats");
      // A second term replaces the answers inside the same area: no entrance.
      await markStart(page);
      await search(page, "Amour n");
      const again = await logged(page, /(leadwrap|shelf-sec)/);
      assert.equal(again.length, 0, `explorer: a new term does not re-enter the area (M6) — ${JSON.stringify(again)}`);
      assert.ok((await page.locator(".leadwrap").count()) === 1, "explorer: the results are still on screen");
      await finish(page);
      await page.unroute(isSearch);
    },
  },
  {
    // I2/I3 — a tab switch fades the panel it brings in (opacity only: panels
    // differ in height) and the tablist steps its colour, it does not slide.
    name: "explorer: onglets",
    midway: true,
    async setup(page) {
      await install(page);
      await stubSearch(page);
      await openSearch(page);
      await search(page, "Amour");
      await page.waitForSelector(".leadwrap", { timeout: 20000 });
      // « Artistes » is already on screen under « Tout »: nothing enters.
      await page.getByRole("tab", { name: "Artistes", exact: true }).first().click();
      await page.waitForSelector('.shelf-sec:has(h2:text("Artistes"))', { timeout: 20000 });
    },
    async act(page) {
      await markStart(page);
      await page.getByRole("tab", { name: "Albums", exact: true }).first().click();
    },
    async settle(page) {
      await page.waitForSelector('.shelf-sec:has(h2:text("Albums"))', { timeout: 20000 });
      await page.evaluate(() => window.__motion.frames(3));
      const level = await motionLevel(page);
      expectEntrance(level, await logged(page, /shelf-sec/), "explorer: le panneau de l'onglet");
      // I3: the selection steps in the instant beat (never a longer colour walk,
      // never a per-render measurement of the rail).
      const beat = await page.evaluate(() => {
        const tab = document.querySelector('[role="tab"][aria-selected="true"]');
        return tab ? getComputedStyle(tab).transitionDuration : null;
      });
      assert.ok(beat, "explorer: the selected tab carries a transition");
      for (const time of String(beat).split(",")) {
        const value = parseFloat(time) * (time.includes("ms") ? 0.001 : 1);
        assert.ok(value <= 0.14, `explorer: the tab steps in ≤ 140 ms (M6) — ${time}`);
      }
      await finish(page);
      await page.unroute(isSearch);
    },
  },
  {
    // I10 — the best-result card is keyed on the entity, so another artist is a
    // NEW card that enters, not a card mutating in place.
    name: "explorer: meilleur résultat d'une autre identité",
    midway: true,
    async setup(page) {
      await install(page);
      await stubSearch(page);
      await openSearch(page);
      await search(page, "Amour");
      await page.waitForSelector(".top .name", { timeout: 20000 });
      assert.match(
        await page.locator(".top .name").first().textContent(),
        /Amour/,
        "explorer: the first term's best result is on screen",
      );
    },
    async act(page) {
      await markStart(page);
      await search(page, "Blue");
    },
    async settle(page) {
      await page.waitForFunction(() => /Blue/.test(document.querySelector(".top .name")?.textContent || ""), null, {
        timeout: 20000,
      });
      await page.evaluate(() => window.__motion.frames(3));
      const level = await motionLevel(page);
      expectEntrance(level, await logged(page, /(^|\s)top(\s|$)/), "explorer: le nouveau meilleur résultat");
      await finish(page);
      await page.unroute(isSearch);
    },
  },
  {
    // I10 — a broad search offers the radio beside the field; the offer RISES
    // (in only: it goes at once when the count drops back under the bar).
    name: "explorer: l'offre de radio se lève",
    midway: true,
    async setup(page) {
      await install(page);
      await stubSearch(page);
      await openSearch(page);
    },
    async act(page) {
      await markStart(page);
      await search(page, "Amour");
    },
    async settle(page) {
      await page.waitForSelector(".radio-launch", { timeout: 20000 });
      const level = await motionLevel(page);
      const entries = await logged(page, /radio-launch/);
      if (level === "eco") {
        assert.equal(entries.length, 0, "explorer: the offer does not rise in eco");
      } else {
        assert.ok(entries.length >= 1, `explorer: the offer rises — got ${JSON.stringify(entries)}`);
        for (const entry of entries) {
          assert.ok(entry.duration <= 200, `explorer: the rise is ${entry.duration} ms`);
          const moved = entry.keys.filter((key) => /^(transform|translate)$/.test(key));
          if (level === "full") assert.ok(moved.length > 0, "explorer: at full motion the offer rises 8 px");
          else assert.equal(moved.length, 0, "explorer: under reduced motion it only fades (M14)");
        }
      }
      await finish(page);
      await page.unroute(isSearch);
    },
  },
];
