// T6f — onboarding, récap, écoute partagée (docs/agents/transitions.md, I15).
//
// WHAT THIS BENCH CAN DRIVE, and what it cannot. The runner (test/motion/run.mjs,
// T0) hands a scenario the app already signed in, on the bench library. Of the
// twelve files of this lot, the ones that live IN the router are the Récap and
// the projector; the Récap is where the motion of this lot is, so it holds the
// two scenarios below:
//
//   * `recap-carte-suivante` — the story crossfades its card (opacity only, both
//     slides absolutely positioned, so the leaving one may play an outro);
//   * `recap-partager` — the share poster is an overlay like every other: a veil
//     that fades its own background alpha and a sheet.
//
// The rest of I15 renders OUTSIDE the router and needs a state this runner's
// server does not create: `Login` wants a signed-out tab, `Onboarding` an
// account whose `onboarding_step` is not `done` (there is no route to set it
// back), `Invite` a redeemable token (redeeming it switches the session, which
// would poison the shared context for every later scenario), `Party` a live
// party with a host pushing state, `RemoteClaim` a claim token whose claim also
// reloads the tab into remote mode, and `Viz` a display route with a track.
// Those were opened and looked at with the throwaway Playwright script the PR
// names — desktop 1440×900, phone 390×844, and the full-screen player's narrow
// 641×900 panel, at rest and mid-animation — and their behaviour is held by
// `test/onboarding/run.mjs`, `test/recap/run.mjs` and `test/party/run.mjs`.
//
// A scenario's `settle` also ASSERTS what the transition did: the runner's own
// four checks (requests unchanged, eco moves nothing, reduced moves nothing,
// no page error) cannot see "the card really changed" or "both slides were
// painted". A throw is the only failure signal the runner propagates (it has no
// per-scenario checker), so the messages here say what was expected and what
// came back.

const LISTENS = 25; // the Récap's ENOUGH_LISTENS is 20 (neezer/recap.py)
const HEARD = 240; // seconds — above the skip threshold, so each one is a play

/** The three levels, read the way lib/motion.js reads them (eco first). */
const LEVEL = `(() =>
  document.documentElement.dataset.eco === "1"
    ? "off"
    : matchMedia("(prefers-reduced-motion: reduce)").matches
      ? "reduced"
      : "full")()`;

/**
 * Record LISTENS plays today through the app's own endpoint, so the Récap has a
 * year to tell. Nothing is faked: these are the rows the player posts itself,
 * one per finished title, and the payload is then the server's own. Returns the
 * year to open.
 *
 * The screen is opened by its YEAR, not by `#/recap`: `/api/recap/years` is
 * aggregated and cached for 60 s, and the shell's home card has already asked
 * for it before a scenario can seed anything — so the years list can still be
 * empty while the year itself is not. `/api/recap/<year>` is the same data the
 * screen would resolve to, read directly.
 */
async function seedRecap(page) {
  const seeded = await page.evaluate(
    async ({ count, heard }) => {
      const years = await fetch("/api/recap/years", { credentials: "same-origin" })
        .then((r) => (r.ok ? r.json() : {}))
        .catch(() => ({}));
      const ids = await fetch("/api/me/favorite-ids", { credentials: "same-origin" })
        .then((r) => (r.ok ? r.json() : { ids: [] }))
        .then((d) => (Array.isArray(d.ids) ? d.ids : []).slice(0, count));
      if (!ids.length) return { plays: 0, year: years.now_year };
      for (const id of ids) {
        await fetch("/api/listen", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          credentials: "same-origin",
          body: JSON.stringify({ deezer_id: id, listened: heard }),
        });
      }
      return { plays: ids.length, year: years.now_year };
    },
    { count: LISTENS, heard: HEARD },
  );
  if (seeded.plays < LISTENS) {
    throw new Error(`misc.mjs: the bench library starred only ${seeded.plays} titles, the Récap needs ${LISTENS}`);
  }
  return seeded.year;
}

/** Any overlay a previous scenario left open is closed first (Escape is the
 *  app's own path, and every sheet is `inert` on the way out). */
async function clean(page) {
  await page.keyboard.press("Escape");
  await page.waitForFunction(() => !document.querySelector(".rs-backdrop"), null, { timeout: 5000 });
}

/**
 * Wait until the app is at rest: no animation the bench counts (status
 * indicators excluded, exactly like the runner's own eco check). Waited on the
 * animation list, never on a clock, and it is what makes the act a fair
 * measurement — the covers a freshly mounted card loads carry a 350 ms fade of
 * their own (`Cover.svelte`, another lot's file), and the runner's
 * "reduced moves nothing over 140 ms" check would otherwise judge that fade
 * instead of this transition.
 */
async function still(page, when) {
  try {
    await page.waitForFunction(() => window.__motion.running().length === 0, null, { timeout: 8000 });
  } catch {
    const left = await page.evaluate(() => window.__motion.running().map(window.__motion.describe));
    throw new Error(`misc.mjs: the screen never came to rest before ${when} — ${JSON.stringify(left)}`);
  }
}

/**
 * The deck at rest AND with every sleeve decoded: the card the act enters must
 * not start a cover request of its own, or the runner's "same requests in the
 * three levels" check would compare a network race (measured: four placeholder
 * sleeves landing after the act in eco and before it at full motion).
 */
async function quiet(page, when) {
  // A phone-sized deck auto-advances every 7 s (webapp.md): taking the focus
  // OUT of the story is the app's own pause (`onFocusOut` → paused), so an act
  // is measured on the card the setup chose.
  await page.evaluate(() => {
    const story = document.querySelector(".story");
    if (!story) return;
    story.focus();
    story.blur();
  });
  try {
    await page.waitForFunction(() => window.__motion.running().length === 0, null, { timeout: 8000 });
  } catch {
    const left = await page.evaluate(() => window.__motion.running().map(window.__motion.describe));
    throw new Error(`misc.mjs: the deck never came to rest before ${when} — ${JSON.stringify(left)}`);
  }
}

/** The deck's counter ("2 / 9"), or "" before it mounts. */
async function counter(page) {
  return page.evaluate(() => (document.querySelector(".story .foot .count")?.textContent || "").trim());
}

/** Wait until the counter reads something else than `before`. */
async function counterMoved(page, before) {
  await page.waitForFunction(
    (was) => (document.querySelector(".story .foot .count")?.textContent || "").trim() !== was,
    before,
    { timeout: 5000 },
  );
}

/**
 * The Récap of `year`, standing on its SECOND-TO-LAST card, with every card of
 * the deck already seen once.
 *
 * Four traps decided this, all measured on this bench:
 *
 *  - never a reload: it would take the bench's probes (`window.__motion`) with
 *    it — the hash goes away to a YEAR THE SERVER HAS NO EVENTS FOR (a single
 *    text panel, no sleeve) and back, which is a push, so the screen mounts
 *    afresh at its first card;
 *  - a phone-sized deck auto-advances every 7 s (webapp.md), so a rewind would
 *    depend on the clock and on the level (auto-advance is off under reduced
 *    motion): the focus is taken OUT of the story first, the app's own pause;
 *  - `Cover` (T4) admits art through a bounded queue, and a card that becomes
 *    visible admits its sleeves DURING the enter — the runner compares the
 *    act's `/api` and image requests by multiset across the three levels, and
 *    those admissions landed after the click in one level and before it in
 *    another;
 *  - so the deck is walked to its END once and back: every card has then been
 *    active, its sleeves are in the browser's cache, and the act — a step onto
 *    the text-only share card — starts no image request at all. The walk is
 *    identical in the three levels, which is what the comparison needs.
 */
async function openRecap(page, year) {
  await page.evaluate((value) => {
    location.hash = "#/recap/" + value;
  }, Number(year) - 1);
  await page.waitForFunction(() => /^#\/recap\//.test(location.hash), null, { timeout: 10000 });
  await page.waitForSelector(".recap-grid .panel, .story", { timeout: 15000 });
  await page.evaluate((value) => {
    location.hash = "#/recap/" + value;
  }, year);
  await page.waitForSelector(".story .slide-wrap.active", { timeout: 30000 });
  await page.waitForFunction(
    () => /^\d+ \/ \d+$/.test((document.querySelector(".story .foot .count")?.textContent || "").trim()),
    null,
    { timeout: 10000 },
  );

  const first = await counter(page);
  const total = Number(first.split("/")[1]);
  if (total < 3) throw new Error(`misc.mjs: the Récap only has ${total} card(s), there is no step to measure`);

  /** Click the deck's own arrow until the card number is `want`. */
  const goTo = async (want) => {
    for (let step = 0; step < 3 * total; step++) {
      const at = await counter(page);
      const index = Number(at.split("/")[0]);
      if (index === want) return;
      const arrow = index < want ? ".story .tap.next" : ".story .tap.prev";
      await page.locator(arrow).click();
      await counterMoved(page, at);
    }
    throw new Error(`misc.mjs: the deck never reached card ${want} of ${total}`);
  };

  await quiet(page, "the walk");
  for (let card = 1; card <= total; card++) await goTo(card); // every sleeve asked for once
  const target = total - 1; // the act then steps onto the share card
  await goTo(target);
  await quiet(page, "the act");
  return { total, index: target };
}

/** What the deck looks like right now: the counter, the slide wrappers, and
 *  every animation running in the story. `frames` waits first. */
async function deckState(page, frames = 1) {
  return page.evaluate(
    async ({ levelSource, frames }) => {
      const level = eval(levelSource);
      await window.__motion.frames(frames);
      const story = document.querySelector(".story");
      const wraps = [...document.querySelectorAll(".story .slide-wrap")];
      const running = (story ? story.getAnimations({ subtree: true }) : [])
        .map((animation) => {
          const effect = animation.effect;
          const target = effect && effect.target;
          const keys = effect && effect.getKeyframes ? effect.getKeyframes() : [];
          return {
            name: (target && (target.className || target.tagName)) || "?",
            // The running countdown bar is a status indicator (M13): the
            // runner's own eco check filters it out the same way.
            status: !!(target && target.closest && target.closest("[data-motion]")),
            duration: Math.round((effect && effect.getTiming().duration) || 0),
            keys: keys.flatMap((frame) => Object.keys(frame)),
          };
        })
        // 0.01 ms remnants: eco's own guard gives every element a transition
        // duration, so ANY style write starts one (measured in eco on a card
        // that just mounted). T0's bench tolerates the same remnant on a
        // hovered `.btn`; they are gone three frames later.
        .filter((animation) => animation.duration > 0.5 && !animation.status);
      return {
        level,
        count: (document.querySelector(".story .foot .count")?.textContent || "").trim(),
        wraps: wraps.map((wrap) => ({
          active: wrap.classList.contains("active"),
          visibility: getComputedStyle(wrap).visibility,
          opacity: Number(getComputedStyle(wrap).opacity),
        })),
        running,
      };
    },
    { levelSource: LEVEL, frames },
  );
}

/** The share overlay's own animations, and where it lives. */
async function readShare(page, frames = 1) {
  return page.evaluate(
    async ({ levelSource, frames }) => {
      const level = eval(levelSource);
      await window.__motion.frames(frames);
      const read = (element) =>
        element
          ? element.getAnimations({ subtree: false }).map((animation) => ({
              keys: (animation.effect.getKeyframes ? animation.effect.getKeyframes() : []).flatMap((frame) =>
                Object.keys(frame),
              ),
              duration: Math.round(animation.effect.getTiming().duration || 0),
            }))
          : [];
      const backdrop = document.querySelector(".rs-backdrop");
      const sheet = document.querySelector(".rs-sheet");
      return {
        level,
        inBody: !!backdrop && backdrop.parentElement === document.body,
        backdrop: read(backdrop),
        sheet: read(sheet),
      };
    },
    { levelSource: LEVEL, frames },
  );
}

/** Where the story scenario stands: set by its `setup`, read by its `settle`. */
let storyCard = { before: 1, total: 1, snapshot: null, share: null };

export const scenarios = [
  {
    // I15 — "Recap story slide change: opacity crossfade DUR.base" (M5: the
    // leaving card gets the shorter exit; M7: nothing in the flow moves).
    name: "recap-carte-suivante",
    midway: true,
    async setup(page) {
      await clean(page);
      const year = await seedRecap(page);
      const { index, total } = await openRecap(page, year);
      storyCard = { before: index, total };
    },
    async act(page) {
      await page.locator(".story .tap.next").click();
      // TWO frames after the click (a CSS transition starts on the style recalc
      // after the class change), and before any picture the runner takes with
      // SHOTS (a screenshot takes longer than the 200 ms fade): the state the
      // assertions below are about.
      storyCard.snapshot = await deckState(page, 2);
    },
    async settle(page) {
      const { before, total } = storyCard;
      const state = storyCard.snapshot;
      const expected = `${before + 1} / ${total}`;
      if (state.count !== expected) {
        throw new Error(`recap-carte-suivante: the card did not change — counter "${state.count}", expected "${expected}"`);
      }
      // One card, one `.active` (M7: the bridge mounts the active card and the
      // next one; the card being left is gone in the same frame).
      const actives = state.wraps.filter((wrap) => wrap.active);
      if (actives.length !== 1) {
        throw new Error(`recap-carte-suivante: expected exactly one active slide, got ${JSON.stringify(state.wraps)}`);
      }
      if (state.level === "full") {
        // The entering card walks in over 200 ms: half way, it is neither 0 nor 1.
        if (!(actives[0].opacity > 0.01 && actives[0].opacity < 0.99)) {
          throw new Error(
            `recap-carte-suivante: the entering card should be mid-fade one frame after the click, ` +
              `opacity ${actives[0].opacity}`,
          );
        }
        const fade = state.running.find(
          (animation) => /slide-wrap/.test(String(animation.name)) && animation.keys.includes("opacity"),
        );
        if (!fade || fade.duration !== 200) {
          throw new Error(
            `recap-carte-suivante: expected a 200 ms opacity transition on .slide-wrap, got ${JSON.stringify(
              state.running,
            )}`,
          );
        }
        const moved = state.running.filter((animation) =>
          animation.keys.some((key) => /^(transform|translate|scale|rotate)$/.test(key)),
        );
        if (moved.length) {
          throw new Error(`recap-carte-suivante: a story card must only fade, got ${JSON.stringify(moved)}`);
        }
      }
      if (state.level === "off" && state.running.length) {
        throw new Error(`recap-carte-suivante: eco must not animate anything, got ${JSON.stringify(state.running)}`);
      }
      if (state.level === "reduced") {
        const over = state.running.filter((animation) => animation.duration > 140);
        if (over.length) {
          throw new Error(`recap-carte-suivante: reduced motion caps at 140 ms, got ${JSON.stringify(over)}`);
        }
        const moved = state.running.filter((animation) =>
          animation.keys.some((key) => /^(transform|translate|scale|rotate)$/.test(key)),
        );
        if (moved.length) {
          throw new Error(`recap-carte-suivante: reduced motion moves nothing, got ${JSON.stringify(moved)}`);
        }
      }
      if (state.level === "off") {
        // The runner's own eco check is taken right after this returns: let the
        // deck (and any sleeve still decoding) come to rest, so it judges the
        // transition and not a cover's rim.
        await quiet(page, "the eco check");
      }
    },
  },
  {
    // I15 — "RecapShare .rs-backdrop → veil + sheet" (M8: the veil never fades
    // the overlay's opacity, or the pane's blur would switch off for its whole
    // enter), and M9: the fixed overlay is portalled to <body>.
    name: "recap-partager",
    midway: true,
    async setup(page) {
      await clean(page);
      const year = await seedRecap(page);
      await openRecap(page, year);
    },
    async act(page) {
      await page.locator("button.share-head:visible, button.share-btn:visible").first().click();
      await page.waitForSelector(".rs-backdrop .rs-sheet", { timeout: 15000 });
      storyCard.share = await readShare(page, 1);
    },
    async settle(page) {
      const state = storyCard.share;

      if (!state.inBody) throw new Error("recap-partager: the fixed overlay must be portalled to <body> (M9)");

      if (state.level === "full") {
        const veil = state.backdrop.find((animation) => animation.keys.includes("backgroundColor"));
        if (!veil || veil.duration < 200) {
          throw new Error(`recap-partager: expected a veil on background-color, got ${JSON.stringify(state.backdrop)}`);
        }
        if (state.backdrop.some((animation) => animation.keys.includes("opacity"))) {
          throw new Error("recap-partager: the veil must never animate the overlay's opacity (M8)");
        }
        if (!state.sheet.length) {
          throw new Error("recap-partager: expected the sheet's own enter, got none");
        }
      }
      if (state.level === "off" && (state.backdrop.length || state.sheet.length)) {
        throw new Error(
          `recap-partager: eco must not animate the sheet, got ${JSON.stringify({
            backdrop: state.backdrop,
            sheet: state.sheet,
          })}`,
        );
      }
      if (state.level === "reduced") {
        const moved = [...state.backdrop, ...state.sheet].filter((animation) =>
          animation.keys.some((key) => /^(transform|translate|scale|rotate)$/.test(key)),
        );
        if (moved.length) throw new Error(`recap-partager: reduced motion moves nothing, got ${JSON.stringify(moved)}`);
      }

      if (state.level === "off") await quiet(page, "the eco check");

      // Leave the level where the next scenario expects it: closed, and gone
      // from the DOM (Svelte removes it on the outro's `inert` end).
      await page.locator(".rs-close").click();
      await page.waitForFunction(() => !document.querySelector(".rs-backdrop"), null, { timeout: 10000 });
    },
  },
];
