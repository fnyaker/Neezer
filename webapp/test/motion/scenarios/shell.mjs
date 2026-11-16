// Lot T1 — the shell (docs/agents/transitions.md, inventory S1–S12).
//
// One scenario per row that has a trigger a bench can pull. Rows with no such
// trigger are NOT here, by rule and not by omission:
//
//   S2  boot screens        the answer to `GET /api/me` — a reload, which would
//                           take the bench's own probes with it;
//   S6  Deezer auto lists   the bench library has no imported Deezer playlist
//                           (Deezer is off), so the fold has nothing to open;
//   S7  a created playlist  the trigger is another lot's sheet (PlaylistPicker);
//   S8  sidebar thumbs      Cover's own fade (lot T4, row L13);
//   S12 `main`'s scroll     no animation at all, by rule: the restore is
//                           `behavior: "instant"` (lib/nav.js) and the Back
//                           scenario below lands on the offset it left.
//
// Every destination is a screen with NO cover art on purpose: a cover's own
// 0.35 s fade is lot T4's row (L13) and would otherwise be measured as if it
// were the shell's. The source page's entrances are waited out in `setup`, so
// a window contains exactly the act's own motion and requests.
//
// Every scenario is `{ name, setup(page), act(page), settle(page), midway }`:
// `act` clicks and then waits to the middle of the enter (where the pictures
// are taken), `settle` waits for the destination to exist and for its request
// burst to be over, so the three motion levels ask for the same things. A
// scenario that only exists on one viewport returns from `act` without
// touching anything on the other.

const NARROW = (page) => page.viewportSize().width <= 640;
const frames = (page, n = 3) => page.evaluate((k) => window.__motion.frames(k), n);

/** Set the hash like a link does (a PUSH: a new entry, so a page enter). */
const hash = (page, path) => page.evaluate((p) => (window.location.hash = p), "#" + path);

/** Put the page on `path` and wait for the thing that proves it painted. */
async function ensure(page, path, ready) {
  const here = await page.evaluate(() => window.location.hash);
  if (here !== "#" + path) await hash(page, path);
  await page.waitForSelector(ready, { timeout: 30000 });
}

/** A still, quiet baseline: every image on the page has settled, the network
 *  has gone quiet, and nothing is animating. Whatever the PREVIOUS route (or
 *  its covers) was doing is over before the scenario's window opens. `still` is
 *  how long the network must stay silent, in ms (the first scenario also calls
 *  `drainWarm`). */
async function baseline(page, still = 450) {
  await page
    .waitForFunction(() => [...document.querySelectorAll("main img")].every((img) => img.complete), null, {
      timeout: 10000,
    })
    .catch(() => {});
  await page.evaluate(
    async (quiet) => {
      const step = () => new Promise((resolve) => requestAnimationFrame(resolve));
      const count = () => performance.getEntriesByType("resource").length;
      let last = count();
      let held = 0;
      const until = performance.now() + 90000;
      while (performance.now() < until) {
        await step();
        const now = count();
        if (now === last) held += 16;
        else {
          last = now;
          held = 0;
        }
        if (held >= quiet) return;
      }
    },
    still,
  );
  await page.waitForFunction(() => window.__motion.running().length === 0, null, { timeout: 5000 }).catch(() => {});
  await frames(page, 2);
}

/** Wait the OFFLINE WARMER out (lib/actions.js#warmPlaylists): it starts 10 s
 *  after `/me/playlists` answers and then pulls one playlist every
 *  WARM_GAP = 1 500 ms, up to WARM_MAX = 25 of them — ~48 s of deliberate
 *  background traffic. Every window it lands in would ask for a playlist the
 *  other two levels did not, so the run waits for it to finish (or for its
 *  whole budget, so a change to those constants cannot hang the bench). */
async function drainWarm(page) {
  await page.evaluate(async () => {
    const step = () => new Promise((resolve) => requestAnimationFrame(resolve));
    const all = () => performance.getEntriesByType("resource").length;
    const pulled = () =>
      performance.getEntriesByType("resource").filter((entry) => entry.name.includes("/api/playlist/")).length;
    const start = performance.now();
    let last = all();
    let held = 0;
    let seen = pulled();
    while (performance.now() - start < 75000) {
      await step();
      const now = all();
      seen = pulled();
      if (now === last) held += 16;
      else {
        last = now;
        held = 0;
      }
      if (seen > 0 && held >= 2500) return;
      if (performance.now() - start >= 55000) return;
    }
  });
}

/** The Now-Playing column is a PERSISTED preference: leave no scenario with it
 *  open, or the next one starts with a third grid column. */
async function closePanel(page) {
  const aside = page.locator("aside.np");
  if (!(await aside.count())) return;
  await page.locator('aside.np button[aria-label="Fermer"]').first().click();
  await aside.waitFor({ state: "detached", timeout: 10000 });
}

/** The nav entry to `path` in whichever shell this viewport has. */
async function navClick(page, path) {
  const sel = NARROW(page) ? `nav.mobilenav a[href="#${path}"]` : `nav.sidebar a[href="#${path}"]`;
  await page.locator(sel).first().click();
}

/** Halfway through a 200 ms enter, where the runner takes its picture. */
const midway = (page) => frames(page, 6);

/** The destination exists and its request burst is over. */
async function settleOn(page, ready) {
  if (ready) await page.waitForSelector(ready, { timeout: 30000 }).catch(() => {});
  await frames(page, 3);
  await page.waitForTimeout(120);
}

/** The id of a playlist the bench library has, for the Back scenario. */
async function aPlaylistId(page) {
  return page.evaluate(async () => {
    const answer = await fetch("/api/me/playlists").then((r) => r.json());
    const list = Array.isArray(answer) ? answer : answer.playlists || [];
    return list.length ? String(list[0].id) : null;
  });
}

export const scenarios = [
  // -- S1 — the page enter, on a PUSH --------------------------------------
  // Explorer → Réglages: two different screens, and Réglages is an ASYNC route,
  // so this is also the row's own trap — `onRouteLoaded` fires when the chunk
  // has landed, and the rise plays then, not on the hash change.
  // Its setup is the one that waits the offline warmer out (see `baseline`).
  {
    name: "s1-page-push",
    midway: true,
    async setup(page) {
      await closePanel(page);
      await ensure(page, "/library", "main .row.track");
      await baseline(page);
      await drainWarm(page);
      await baseline(page);
    },
    async act(page) {
      await navClick(page, "/settings");
      await midway(page);
    },
    settle: (page) => settleOn(page, "main .side-title"),
  },

  // -- S1 — the page enter, on a TRAVERSAL ---------------------------------
  // The same move arrived at by going back: opacity only over 140 ms, because
  // the offset the scroll memory restores must not be seen to slide.
  {
    name: "s1-page-retour",
    midway: true,
    async setup(page) {
      await closePanel(page);
      await ensure(page, "/library", "main .row.track");
      await baseline(page);
      await navClick(page, "/settings");
      await settleOn(page, "main .side-title");
    },
    async act(page) {
      // A real traversal, from the page itself: `page.goBack()` waits on
      // Playwright's navigation bookkeeping and a hash-only move has none.
      await page.evaluate(() => window.history.back());
      await midway(page);
    },
    settle: (page) => settleOn(page, "main .row.track"),
  },

  // -- S3 — the Now-Playing column -----------------------------------------
  // The player bar's queue button: `panel` (opacity + a 16 px lean) in, and
  // `.layout.np-open` held until the aside's own `outroend`.
  {
    name: "s3-colonne-file",
    midway: true,
    async setup(page) {
      if (NARROW(page)) return;
      await closePanel(page);
      await ensure(page, "/", "main .home");
      await page.waitForSelector("footer.player", { timeout: 30000 });
      await baseline(page);
    },
    async act(page) {
      if (NARROW(page)) return;
      await page.locator('footer.player button[aria-label="File d\'attente"]').first().click();
      await midway(page);
    },
    async settle(page) {
      if (NARROW(page)) return;
      await page.waitForSelector("aside.np", { timeout: 10000 });
      await settleOn(page, null);
    },
  },

  // -- S4 — the sidebar's active item --------------------------------------
  // Colour only, on `--dur-instant` (M6: a frequent thing barely moves).
  {
    name: "s4-sidebar-actif",
    midway: true,
    async setup(page) {
      if (NARROW(page)) return;
      await closePanel(page);
      await ensure(page, "/", "main .home");
      await baseline(page);
    },
    async act(page) {
      if (NARROW(page)) return;
      await page.locator('nav.sidebar a[href="#/search"]').first().click();
      await midway(page);
    },
    settle: (page) => settleOn(page, "main h1.page-title"),
  },

  // -- S5 — a sidebar folder ------------------------------------------------
  // The chevron's turn and the body's height walk (M10, ≤ 30 playlists). The
  // folder is created through its own sheet in `setup` (a real user path; the
  // bench library has none), so the act is only the disclosure itself.
  {
    name: "s5-dossier",
    midway: true,
    async setup(page) {
      if (NARROW(page)) return;
      await closePanel(page);
      await ensure(page, "/", "main .home");
      if (!(await page.locator("nav.sidebar [data-folder-row]").count())) {
        await page.locator('nav.sidebar button[aria-label="Nouveau dossier"]').first().click();
        await page.waitForSelector("#folder-name", { timeout: 10000 });
        await page.fill("#folder-name", "Banc mouvement");
        await page.locator('.sheet button[type="submit"]').first().click();
        await page.waitForSelector("nav.sidebar [data-folder-row]", { timeout: 10000 });
      }
      // A folder the person has just created is already OPEN (actions.js opens
      // it), and the folder a previous run created may be open too: the act is
      // an opening, so close it first and let the outro finish.
      const fold = page.locator("nav.sidebar [data-folder-row] button.pl-row.fold").first();
      if ((await fold.getAttribute("aria-expanded")) === "true") {
        await fold.click();
        await page.waitForFunction(() => !document.querySelector("nav.sidebar ul.pl-list.nested"), null, {
          timeout: 10000,
        });
      }
      await baseline(page);
    },
    async act(page) {
      if (NARROW(page)) return;
      await page.locator("nav.sidebar [data-folder-row] button.pl-row.fold").first().click();
      await midway(page);
    },
    settle: (page) => settleOn(page, null),
  },

  // -- S9 — the phone tab bar ----------------------------------------------
  // The pill's colour and the arriving glyph's `swap` (opacity + 4 px, 140 ms).
  {
    name: "s9-onglets",
    midway: true,
    async setup(page) {
      if (!NARROW(page)) return;
      await ensure(page, "/library", "main .row.track");
      await baseline(page);
    },
    async act(page) {
      if (!NARROW(page)) return;
      await page.locator('nav.mobilenav a[href="#/settings"]').first().click();
      await midway(page);
    },
    settle: (page) => settleOn(page, "main .side-title"),
  },

  // -- S10 — the top bar's search field ------------------------------------
  // A 140 ms fade both ways as `/search` comes and goes. Opacity only: the bar
  // is sticky, and a transform there jitters over the scrolling page.
  {
    name: "s10-champ-recherche",
    midway: true,
    async setup(page) {
      await closePanel(page);
      await ensure(page, "/", "main .home");
      await baseline(page);
    },
    async act(page) {
      await page.locator(".topbar button.search").first().click();
      await midway(page);
    },
    settle: (page) => settleOn(page, "main h1.page-title"),
  },

  // -- S11 — the back button ------------------------------------------------
  // A detail page → the button fades out over 140 ms, its box held for the
  // whole outro so the field beside it does not jump.
  {
    name: "s11-retour",
    midway: true,
    async setup(page) {
      await closePanel(page);
      await ensure(page, "/settings", "main .side-title");
      await baseline(page);
      const id = await aPlaylistId(page);
      if (!id) return;
      await hash(page, "/playlist/" + id);
      await page.waitForSelector('button[aria-label="Retour"]', { timeout: 20000 });
      await baseline(page);
    },
    async act(page) {
      await page.locator('button[aria-label="Retour"]').first().click();
      await midway(page);
    },
    settle: (page) => settleOn(page, "main .side-title"),
  },
];
