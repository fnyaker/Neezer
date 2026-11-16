// Lot T3 — the player: one scenario per P-row of the inventory
// (docs/agents/transitions.md) that has a CLICKABLE trigger, so the bench
// (test/motion/run.mjs) can hold it at 1440×900 and 390×844 in the three motion
// levels (full / reduced / eco).
//
// What each scenario proves, beyond the runner's own per-level checks
// (identical requests, eco = nothing animates, reduced = no displacement and
// ≤ 140 ms):
//
//   P1/P2/P3  the full-screen player enters (`screen`) and leaves; the app's
//             ground is handed over on `introend`/`outrostart` (a bench that
//             finds 0 `.gb` while the player is open, and 1 again after).
//   P4        the phone's queue and lyrics sheets enter with `sheet`.
//   P7/P8/P12 a track change swaps the title block (both players, the bar) and
//             crossfades the desktop sleeve.
//   P10       the live lyric line swaps (when the track has timed lyrics).
//   P11       the desktop player's tab panels fade in.
//   P13       the transport glyph swaps on the state the press already flipped.
//   P14       shuffle/repeat light up in colour, the heart swaps.
//   P15       the bar leaves its empty state with a fade (this file's FIRST
//             scenario, because the bar must still be empty when it runs).
//   P16       the phone card's position line has NO transition at any level
//             (M6: a live progress IS the motion) and still follows a seek —
//             eco included, where a frozen line would be a lie.
//   P17       the À propos sections fade in when their answers arrive (the
//             layouts are fetched from the bench server; Deezer is unreachable,
//             so the credits/similar blocks may be empty — the panel's own fade
//             is what is measured).
//
// NOT exercised here, deliberately: P5/P6 (a gesture and the browser's own
// scroll-snap), P9/P22 (the backdrop/visualizer owners retime nothing)
// (the volume drag / seek themselves have no transition by rule M6; the
// position line they feed is asserted by `player-progression`), P18/P20 (scroll-follow and job progress have no clickable trigger
// and no transcript fixture in `tools/perf_api.py`), P19/P21 (the transcript
// menu and the credits/ticks blocks need a podcast with a finished transcript:
// the bench server has none). Their transitions are unit-covered by
// test/motion.test.mjs (durations, easing, eco and reduced paths).

/**
 * Navigate by hash (never `goto`: a full load would drop the bench's probes and
 * its `/api` request watcher).
 */
async function go(page, hash) {
  await page.evaluate((h) => {
    if (location.hash !== h) location.hash = h;
  }, hash);
}

/** The phone/desktop branch the component itself reads (lib/viewport.js). */
const phone = (page) => page.evaluate(() => matchMedia("(max-width: 640px)").matches);

/**
 * The bench library's first playlist, chosen by its ID so EVERY level plays the
 * same queue: the playlists list's order and its first painted row are not
 * stable enough to key a request multiset on (each motion level is a fresh
 * page, and the first row to render differs).
 */
async function firstPlaylistId(page) {
  return page.evaluate(async () => {
    const data = await (await fetch("/api/me/playlists")).json();
    const list = Array.isArray(data) ? data : data.playlists || [];
    return list.map((p) => String(p.id)).sort()[0] || null;
  });
}

/**
 * Once per page: let the app's own offline warm-up run its course
 * (lib/actions.js#warmPlaylists — it waits 10 s after the playlists list, then
 * pulls one `/playlist/<id>` every 1.5 s for up to 25 lists). It is the app
 * doing its job; left alone it lands inside whichever scenario is running and
 * makes the request multiset differ between the three levels, which is not the
 * transition's doing. Called from the FIRST scenario, before any measurement.
 */
async function warmUp(page) {
  if (page.__motionWarmed) return;
  page.__motionWarmed = true;
  // A page that is already past the warm-up's start (another lot's scenarios
  // ran first) needs no delay, only the drain.
  const age = await page.evaluate(() => performance.now());
  if (age < 15000) await page.waitForTimeout(12000);
  await quiet(page);
}

/** Open a playlist page and return its « Lire » button (the hero's, at the top,
 *  never under the sticky bar). */
async function openPlaylist(page) {
  const id = await firstPlaylistId(page);
  if (!id) throw new Error("the bench library has no playlist");
  await go(page, "#/playlist/" + id);
  const play = page.locator('button.play[aria-label="Lire"]').first();
  await play.waitFor({ state: "visible", timeout: 60000 });
  return play;
}

/**
 * Play something through the UI, so the player bar and the full-screen player
 * have a track. Idempotent: a track that is already loaded is left alone.
 */
async function ensurePlaying(page) {
  await warmUp(page);
  if (await page.locator("footer.player .now .t").count()) return;
  const play = await openPlaylist(page);
  await play.click();
  await page.locator("footer.player .now .t").first().waitFor({ state: "visible", timeout: 60000 });
  // Paused at once (see `pause`): the bar and the full-screen player show the
  // same track, and nothing keeps streaming underneath.
  await pause(page);
  // Leave the 4 000-row playlist page behind: its cover queue and its rows would
  // keep working (and asking) under every later scenario. Rechercher is empty
  // until something is typed — a quiet screen with the bar still on it.
  await go(page, "#/search");
  await quiet(page);
  await settlePrefetch(page);
}

/** `.m` on a phone, `.d` on a desktop — the full-screen player's root. */
const playerRoot = (page) => page.evaluate(() => (document.querySelector(".m") ? ".m" : document.querySelector(".d") ? ".d" : null));

async function openPlayer(page) {
  if (await phone(page)) {
    await page.locator("footer.player .now").first().click();
    await page.waitForSelector(".m", { state: "attached", timeout: 15000 });
  } else {
    await page.locator('footer.player button[aria-label="Plein écran"]').first().click();
    await page.waitForSelector(".d", { state: "attached", timeout: 15000 });
  }
}

/** Close whatever full-screen player is open (the visual hand-off runs). */
async function closePlayer(page) {
  const root = await playerRoot(page);
  if (!root) return;
  await page.locator(`${root} button[aria-label="Réduire"]`).first().click();
  await page.waitForSelector(root, { state: "detached", timeout: 15000 });
}

/**
 * Wait until the CSS/WAAPI motion this element OWNS has finished. Two frames
 * first: the transition is created in a microtask, so an immediate read would
 * see none. Its own animations only — the visualizer's grain inside `.m` runs
 * forever and is not this transition.
 */
async function settled(page, selector) {
  await page.evaluate(() => window.__motion.frames(2));
  await page.waitForFunction(
    (sel) => {
      const el = document.querySelector(sel);
      if (!el) return true;
      return el.getAnimations().every((a) => a.playState !== "running");
    },
    selector,
    { polling: 16, timeout: 15000 },
  );
}

/**
 * The page at REST before the scenario is judged: no request in flight (covers,
 * streams and the screen's own data land asynchronously and would otherwise be
 * counted in the NEXT scenario's window), no loading placeholder still
 * shimmering, and no animation of another area still running. Bounded, so a
 * page that never goes quiet does not hold the bench.
 */
async function quiet(page) {
  if (!page.__motionWatched) {
    page.__motionWatched = true;
    page.__motionLastRequest = Date.now();
    page.on("request", () => (page.__motionLastRequest = Date.now()));
  }
  // "networkidle" is not enough: the shell's cover queue wakes on an idle
  // callback and a playlist's rows keep asking long after a first gap, so a
  // request was landing in the NEXT scenario's window at a moment that depends
  // on the machine rather than on the motion level. Wait for a real silence.
  // Bounded, but long enough for the app's own bursts (a playlist's prefetch
  // run is several /api/download calls with their streams). The cap only costs
  // time while the page is actually talking.
  const deadline = Date.now() + 60000;
  while (Date.now() - page.__motionLastRequest < 3000 && Date.now() < deadline) {
    await page.waitForTimeout(200);
  }
  await page.evaluate(() => window.__motion.frames(2));
  await page
    .waitForFunction(
      () => {
        if (document.querySelector(".sk")) return false;
        return !document.getAnimations().some((a) => {
          const target = a.effect && a.effect.target;
          if (target && target.closest && target.closest("[data-motion]")) return false;
          return a.playState === "running";
        });
      },
      null,
      { polling: 50, timeout: 4000 },
    )
    .catch(() => {});
}

/**
 * Playback's own background work: the player prefetches the NEXT track's audio
 * 4 s after the queue's next changes (Player.svelte#ARCHIVE_DELAY = 4000), and
 * that POST /api/download would otherwise land in a later scenario's request
 * window at a moment that depends on the machine, not on the motion level.
 * Wait it out (the delay is the app's own constant) and let the page go quiet.
 */
async function settlePrefetch(page) {
  await page.waitForTimeout(4500);
  await quiet(page);
}

/**
 * The sound itself is not what this lot measures, and a PLAYING track starts
 * streams, an analysis pass and a prefetch download whose timing belongs to the
 * machine, not to the motion level. Where the state is known to be playing, the
 * scenarios pause it; the animations are identical either way.
 */
async function pause(page) {
  await page.locator("footer.player .pp").first().click();
}

const inPlayer = (page, selector) => page.locator(selector).first();

export const scenarios = [
  // --- P15 first, while the bar is still empty ------------------------------
  {
    name: "player-premier-titre",
    // The bar must start empty: pausing is harmless, playing a playlist is not.
    async setup(page) {
      await warmUp(page);
      await openPlaylist(page);
      await quiet(page);
    },
    // The click that takes the bar from "Rien en lecture" to a track.
    async act(page) {
      await page.locator('button.play[aria-label="Lire"]').first().click();
      await page.waitForSelector("footer.player .now .t", { state: "visible", timeout: 15000 });
    },
    midway: true,
    async settle(page) {
      await settled(page, "footer.player .now .info");
      await pause(page);
      await go(page, "#/search");
      await quiet(page);
      await settlePrefetch(page);
    },
  },

  // --- P1 (phone) / P2 (desktop) / P3: the full-screen player ---------------
  {
    name: "player-plein-ecran",
    async setup(page) {
      await ensurePlaying(page);
    },
    async act(page) {
      await openPlayer(page);
    },
    midway: true,
    async settle(page) {
      // The `screen` transition lives on ImmersivePlayer's host, not on the
      // component's root element (a local transition cannot play there).
      await settled(page, ".player-host");
      // P3: the app's ground is gone at rest (the player brings its own) …
      await page.evaluate(() => {
        const gb = document.querySelector(".gb");
        if (!gb) return;
        const shown = getComputedStyle(gb).display !== "none";
        if (shown) throw new Error("GlobalBackdrop still painted under the open player");
      });
      await closePlayer(page);
      // … and it comes back on `outrostart`, not on a timer.
      await page.waitForFunction(() => {
        const gb = document.querySelector(".gb");
        return !!gb && getComputedStyle(gb).display !== "none";
      }, null, { polling: 16, timeout: 15000 });
      await settled(page, ".gb-art");
      await quiet(page);
    },
  },

  // --- P4 (phone lyrics sheet) / P11 + P17 (desktop À propos tab) -----------
  {
    name: "player-panneau",
    async setup(page) {
      await ensurePlaying(page);
      await openPlayer(page);
      await settled(page, ".player-host");
    },
    async act(page) {
      if (await phone(page)) {
        // « Paroles »: the phone's lyrics VIEW (P4), not a hidden gesture.
        await page.locator('.m .footer button[aria-label="Paroles"]').first().click();
      } else {
        // The À propos tab next to the queue (P11) — its sections fade in as
        // their answers arrive (P17).
        await page.locator('.d .tabs button:has-text("À propos")').first().click();
      }
    },
    midway: true,
    async settle(page) {
      if (await phone(page)) {
        await page.waitForSelector(".m .sheet[aria-label='Paroles']", { state: "attached", timeout: 15000 });
        await settled(page, ".m .sheet[aria-label='Paroles']");
        await page.locator(".m .sheet button[aria-label='Fermer les paroles']").first().click();
        await page.waitForSelector(".m .sheet[aria-label='Paroles']", { state: "detached", timeout: 15000 });
      } else {
        await settled(page, ".d .side-body");
        await page.locator(`.d .tabs button:has-text("File d'attente")`).first().click();
        await settled(page, ".d .queue");
      }
      await closePlayer(page);
      await quiet(page);
    },
  },

  // --- P4 (phone queue sheet) / P11 (desktop queue tab) ---------------------
  {
    name: "player-file",
    async setup(page) {
      await ensurePlaying(page);
      await openPlayer(page);
      await settled(page, ".player-host");
    },
    async act(page) {
      if (await phone(page)) {
        await page.locator(`.m .footer button[aria-label="File d'attente"]`).first().click();
      } else {
        await page.locator(`.d .tabs button:has-text("File d'attente")`).first().click();
      }
    },
    midway: true,
    async settle(page) {
      if (await phone(page)) {
        await page.waitForSelector(".m .sheet[aria-label=\"File d'attente\"]", { state: "attached", timeout: 15000 });
        await settled(page, ".m .sheet[aria-label=\"File d'attente\"]");
        await page.locator(`.m .sheet button[aria-label="Fermer la file d'attente"]`).first().click();
        await page.waitForSelector(".m .sheet[aria-label=\"File d'attente\"]", { state: "detached", timeout: 15000 });
      } else {
        await settled(page, ".d .queue");
      }
      await closePlayer(page);
      await quiet(page);
    },
  },

  // --- P7 / P8 / P10 / P12: a track change ----------------------------------
  {
    name: "player-piste-suivante",
    async setup(page) {
      await ensurePlaying(page);
      await openPlayer(page);
      await settled(page, ".player-host");
    },
    async act(page) {
      const root = await playerRoot(page);
      await inPlayer(page, `${root} button[aria-label="Suivant"]`).click();
    },
    midway: true,
    async settle(page) {
      const root = await playerRoot(page);
      await settled(page, `${root} .txt`);
      // The bar's own title block swapped too (P12) — same string key.
      await settled(page, "footer.player .now .txt");
      await closePlayer(page);
      await quiet(page);
      await settlePrefetch(page);
    },
  },

  // --- P13: the transport glyph ---------------------------------------------
  {
    name: "player-glyphe",
    async setup(page) {
      await ensurePlaying(page);
    },
    async act(page) {
      // The state flips first (M2); the glyph is what plays on top.
      await page.locator("footer.player .pp").first().click();
    },
    midway: true,
    async settle(page) {
      await settled(page, "footer.player .pp .glyph");
      // The press started the sound; put it back to rest before the next
      // scenario (the glyph swap above is the whole point of this row).
      await pause(page);
      await quiet(page);
    },
  },

  // --- P16: the position line is information, never motion ------------------
  {
    name: "player-progression",
    async setup(page) {
      await ensurePlaying(page);
    },
    // A real seek through the bar the phone card shows.
    async act(page) {
      await page.evaluate(() => {
        const rail = document.querySelector("footer.player .seek input[type=range]");
        if (!rail) return;
        rail.value = String(Number(rail.max) * 0.42);
        rail.dispatchEvent(new Event("input", { bubbles: true }));
      });
    },
    async settle(page) {
      await page.evaluate(() => window.__motion.frames(2));
      const state = await page.evaluate(() => {
        const el = document.querySelector(".mini-progress");
        if (!el) return null;
        const ms = (v) => (String(v).endsWith("ms") ? parseFloat(v) : parseFloat(v) * 1000);
        return {
          eco: document.documentElement.dataset.eco === "1",
          line: ms(getComputedStyle(el).transitionDuration),
          after: ms(getComputedStyle(el, "::after").transitionDuration),
          running: el.getAnimations({ subtree: true }).length,
          p: getComputedStyle(el).getPropertyValue("--p").trim(),
        };
      });
      if (state) {
        if (state.line > 0.02 || state.after > 0.02)
          throw new Error(
            `the position line carries a transition (${state.line} ms / ${state.after} ms): M6 says a live progress has none`,
          );
        if (state.running)
          throw new Error(`the position line is animating (${state.running} animation(s)) — nothing may run on a live progress`);
        // The line still tells the truth after a seek, in every level.
        if (!(parseFloat(state.p) > 0.1))
          throw new Error(`the position line did not follow the seek (--p: ${state.p || "empty"})`);
      }
      await quiet(page);
    },
  },

  // --- P14: shuffle, repeat, the favourite heart ----------------------------
  {
    name: "player-favori",
    async setup(page) {
      await ensurePlaying(page);
      if (await phone(page)) {
        // The mini bar keeps only the sleeve, the transport and the doors; the
        // shuffle/repeat/heart controls live in the thumb player on a phone.
        await openPlayer(page);
        await settled(page, ".player-host");
      }
    },
    async act(page) {
      const root = (await phone(page)) ? ".m" : "footer.player";
      await page.locator(`${root} button[aria-label='Aléatoire']`).first().click();
      await page.locator(`${root} button[aria-label='Répéter']`).first().click();
      const heart = page.locator(`${root} .fav:visible`).first();
      if (await heart.count()) await heart.click();
    },
    midway: true,
    async settle(page) {
      const root = (await phone(page)) ? ".m" : "footer.player";
      await settled(page, `${root} .fav`);
      // Put shuffle and repeat back the way they were, so the state a later
      // scenario (and the next level) sees is the same.
      await page.locator(`${root} button[aria-label='Aléatoire']`).first().click();
      await page.locator(`${root} button[aria-label='Répéter']`).first().click();
      await closePlayer(page);
      await quiet(page);
    },
  },
];
