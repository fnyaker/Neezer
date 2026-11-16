// LOT T6e — genres & Flow: one scenario per inventory row of
// docs/agents/transitions.md that has a clickable trigger.
//
//   Genres tabs (I2/I3)          → genres-onglets
//   GenreTree node expand (I4)   → genres-arbre
//   the studio's accordions (I4) → genres-reglages
//   ExplorerStyles veil+sheet    → explorer-stations
//   pickers' selected chips (I5) → accueil-energie, accueil-decouverte,
//                                  accueil-styles
//   FlowControls popover         → flow-controles
//
// Every `setup` navigates by HASH (`location.hash`), so a scenario never needs
// the harness' base URL and never reloads the page: `test/motion/run.mjs`
// installs its probes once per context and a reload would drop them.
//
// Two rows cannot be clicked on this bench, and both say so here rather than
// silently passing:
//   - the discovery sheet (ExplorerStyles `.tiles`) needs a listening history
//     with styles; `tools/perf_api.py` has none, so the section renders its
//     "historique vide" state. The stations sheet below is the SAME veil+sheet
//     pair in the same component, so it is the one exercised;
//   - FlowControls' door only exists while a Flow plays, and starting one needs
//     the Deezer proxy, which the bench server has switched off
//     ("Le proxy Deezer est désactivé sur ce serveur"). `flow-controles` clicks
//     the "Lancer mon Flow" button anyway and then the door when a server has
//     it, and falls back to opening the player bar so eco/reduced are still
//     held over the same region of the app.

const frames = (page, n = 3) => page.evaluate((count) => window.__motion.frames(count), n);

/**
 * Wait until the page has stopped asking for things — a screen that loads
 * covers and recommendations on mount must be quiet BEFORE the scenario's act,
 * or those requests land on either side of the runner's mark and the three
 * levels look different when nothing about them is. 150 ms of silence, capped.
 */
async function quiet(page) {
  let seen = 0;
  let last = -1;
  const bump = () => seen++;
  page.on("request", bump);
  try {
    for (let stable = 0; stable < 4; stable = seen === last ? stable + 1 : 0) {
      last = seen;
      await page.waitForTimeout(150);
    }
  } finally {
    page.off("request", bump);
  }
}

async function hash(page, route) {
  await page.evaluate((r) => {
    location.hash = r;
  }, route);
}

/** Escape closes every overlay this file opens (the components' own handler). */
/**
 * The home's own asynchronous tail: the "reprendre" playlist, the recommendation
 * shelf, and the sleeves those shelves lazily load. Waits for them (and then for
 * silence) so a scenario's act never races a request the screen was going to
 * make anyway — the runner compares each level's request multiset.
 */
async function homeReady(page, selector) {
  await page.waitForSelector(selector, { timeout: 20000 });
  await page
    .waitForFunction(() => performance.getEntriesByType("resource").some((e) => e.name.includes("/api/playlist/")), null, { timeout: 8000 })
    .catch(() => {});
  await page.waitForFunction(() => [...document.images].every((i) => i.complete), null, { timeout: 8000 }).catch(() => {});
  await frames(page, 2);
  await quiet(page);
}

async function closeOverlays(page) {
  await page.keyboard.press("Escape");
  await frames(page, 2);
}

export const scenarios = [
  {
    // I2/I3: the tab strip is an underline, so the active tab's colour walks
    // over --dur-instant and the panel that replaces the old one fades in
    // (opacity only, M7: the panels differ in height).
    name: "genres-onglets",
    midway: true,
    async setup(page) {
      await hash(page, "#/genres");
      await page.waitForSelector(".tabs .tab", { timeout: 20000 });
      await frames(page, 2);
      await quiet(page);
    },
    async act(page) {
      await page.locator(".tabs .tab", { hasText: "Arbre" }).click();
    },
    async settle(page) {
      await page.waitForSelector(".tree", { timeout: 20000 });
      await frames(page, 4);
    },
  },
  {
    // I4: the tree's node body IS the windowed list's rows, and a row never
    // animates on mount (M4). What moves is the chevron (--dur-base); the
    // scenario holds that a collapse paints in one frame and adds no request.
    name: "genres-arbre",
    midway: true,
    async setup(page) {
      await hash(page, "#/genres");
      await page.waitForSelector(".tabs .tab", { timeout: 20000 });
      await page.locator(".tabs .tab", { hasText: "Arbre" }).click();
      await page.waitForSelector(".tree .row", { timeout: 20000 });
      await frames(page, 3);
      await quiet(page);
    },
    async act(page) {
      const caret = page.locator(".tree .caret.open").first();
      if (await caret.count()) await caret.click();
      else await page.locator(".tree .caret").first().click();
    },
    async settle(page) {
      await frames(page, 4);
    },
  },
  {
    // I4/M10: the advanced block is far taller than 400 px, so it opens at once
    // and its content fades in — no `height` walk per frame.
    name: "genres-reglages",
    midway: true,
    async setup(page) {
      await hash(page, "#/genres");
      await page.waitForSelector(".tabs .tab", { timeout: 20000 });
      await page.locator(".tabs .tab", { hasText: "Modèle" }).click();
      await page.waitForSelector("details.advanced > summary", { timeout: 20000 });
      await frames(page, 3);
      await quiet(page);
    },
    async act(page) {
      await page.locator("details.advanced > summary").first().click();
    },
    async settle(page) {
      await page.waitForSelector("details.advanced[open] .modes", { timeout: 10000 });
      await frames(page, 4);
    },
  },
  {
    // The inventory's `veil` + `sheet` pair (M8): the overlay fades its own
    // background-colour alpha, the pane keeps its glass, and on a phone the
    // pane slides by its own height (no opacity, M8).
    name: "explorer-stations",
    midway: true,
    async setup(page) {
      await hash(page, "#/search");
      await page.waitForSelector(".styles", { timeout: 20000 });
      await frames(page, 2);
      await quiet(page);
    },
    async act(page) {
      await page.locator(".styles .allbtn").click();
    },
    async settle(page) {
      await page.waitForSelector(".veil .panel.all", { timeout: 20000 });
      await frames(page, 4);
      await closeOverlays(page);
    },
  },
  {
    // I5: a selected notch is colour on a rail six decisions wide (--dur-instant).
    name: "accueil-energie",
    midway: true,
    async setup(page) {
      await hash(page, "#/");
      await homeReady(page, ".energy button");
    },
    async act(page) {
      await page.locator(".energy button").nth(3).click();
    },
    async settle(page) {
      await frames(page, 4);
    },
  },
  {
    // The named discovery steps under the slider: colour only (a drag is the
    // motion itself, M6 — the range input gets no transition). Held on the
    // Réglages copy: the home shelves around the hero load covers lazily, and a
    // cover that enters the viewport mid-scenario would move the request
    // multisets for reasons that have nothing to do with motion.
    name: "reglages-decouverte",
    midway: true,
    async setup(page) {
      await hash(page, "#/settings/discovery");
      await page.waitForSelector(".disc .ticks button", { timeout: 20000 });
      // The section also mounts the "je n'aime pas" list, which is its own
      // request: let it land before the mark, or the three levels disagree for
      // a reason that is not motion.
      await page
        .waitForFunction(() => performance.getEntriesByType("resource").some((e) => e.name.includes("/api/playlist/")), null, { timeout: 8000 })
        .catch(() => {});
      await frames(page, 2);
      await quiet(page);
    },
    async act(page) {
      await page.locator(".disc .ticks button").nth(3).click();
    },
    async settle(page) {
      await frames(page, 4);
    },
  },
  {
    // I5 + M7: picking a family recolours its chip and opens its sub-genres, a
    // block that arrives in place and therefore only enters (fadeIn, no outro).
    name: "accueil-styles",
    midway: true,
    async setup(page) {
      await hash(page, "#/");
      await homeReady(page, ".style-block .chips .chip");
      // Select once HERE, so the shelves the choice re-renders have fetched
      // their sleeves before the act: the act then only DEselects, and the
      // request multisets stay the same for a reason that is not the sleeves.
      await page.locator(".style-block .chips .chip").nth(1).click();
      await page.waitForFunction(() => [...document.images].every((i) => i.complete), null, { timeout: 8000 }).catch(() => {});
      await frames(page, 2);
      await quiet(page);
    },
    async act(page) {
      await page.locator(".style-block .chips .chip").nth(1).click();
    },
    async settle(page) {
      await frames(page, 4);
    },
  },
  {
    // FlowControls' door is a `toBody` popover: `pop` on the panel, `veil` on
    // its scrim (its own background-colour, never an ancestor opacity).
    name: "flow-controles",
    midway: true,
    async setup(page) {
      await hash(page, "#/");
      await homeReady(page, ".energy button");
      // Best effort: on a server with the Deezer proxy this really starts a
      // Flow, which is what makes the door exist at all.
      const launch = page.getByRole("button", { name: /Lancer le Flow|Lancer mon Flow/ });
      if (await launch.count()) {
        await launch.first().click();
        await frames(page, 3);
        await page.waitForTimeout(600);
      }
      await quiet(page);
    },
    async act(page) {
      const door = page.locator(".flowctl .open");
      if (await door.count()) {
        await door.first().click();
        return;
      }
      // No Flow is live on this server (the proxy is off), so the door does not
      // exist: the nearest real control of the same hero is clicked instead, and
      // the scenario still holds that the region changes with no motion in eco
      // and nothing over 140 ms under reduced motion. The door's own `pop` is
      // covered by test/motion.test.mjs at the three levels.
      await page.locator(".hero .tune").click();
    },
    async settle(page) {
      await frames(page, 4);
      if (await page.locator(".veil .panel").count()) await closeOverlays(page);
    },
  },
];
