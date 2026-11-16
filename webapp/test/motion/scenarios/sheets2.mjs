// Lot T2c — the second family of sheets, dialogs and their two player buttons
// (docs/agents/transitions.md, inventory rows O16–O25).
//
// ONE scenario per row that has a clickable trigger, driven through the real UI
// in the bench's own fixture (tools/perf_api.py: an 8 000-track library, one
// account, Deezer unreachable, no podcast episode, no synced lyrics). Each one
// clicks the control a person clicks and then WAITS (never sleeps) for the pane
// to be in the DOM, so the runner's two frames land while the enter is still
// playing: eco must show nothing running, reduced must show no transform.
//
// Every setup ends on a QUIET page (`networkidle`): the runner takes its request
// mark right before the act, and a playlist or cover request still in flight
// from the navigation would land inside the measured window and make the three
// levels' request multisets differ for reasons that have nothing to do with
// motion. Every scenario leaves its sheet OPEN for the runner's at-rest
// picture; the next one's `setup` dismisses whatever is on screen first.
//
// Rows this fixture cannot reach, said out loud rather than silently skipped:
//   O19  a linked Deezer account (« Choisir les artistes… » lives in Réglages →
//        Sources → Mes comptes and only exists once an account is connected);
//   O23  a podcast episode (tools/perf_api.py --podcasts seeds none);
//   O24  synced lyrics (the synthetic library has none);
//   O21/O22 on a PHONE: the two buttons live in the player bar's right-hand row,
//        which the phone shell hides — on a phone they are inside the full-screen
//        player, which needs a playing track (lot T3's own scenarios drive that).
// O25 has no click trigger at all (it is those two buttons changing state when a
// session starts), so it has no scenario: it is covered by the sheets they open.

/** The panes this file opens, for the dismiss-between-scenarios step. */
const PANES = ".overlay, .backdrop, .chsheet, .lc-backdrop, .menu";

/** Per-scenario facts `act` reads (scenarios run one at a time). */
const state = { ready: true };

/** Navigate by hash: a same-document move, so the shell is not re-booted. */
async function at(page, hash) {
  const url = new URL(page.url());
  url.hash = hash;
  await page.goto(url.href);
}

/** Let everything the navigation started land BEFORE the runner's mark.
 *
 *  The runner records every /api and image request from the act to the end of
 *  settle and requires the same multiset in the three levels, so a cover or a
 *  palette read that is still in flight would be counted in one level and not
 *  another for reasons that have nothing to do with motion. Covers arrive
 *  through `lib/coverqueue.js`'s ladder, which starts them a beat AFTER the
 *  page looks idle: so the wait is `networkidle` (500 ms of silence) three
 *  times over, plus "nothing is animating" — a level, not a clock. */
async function quiet(page) {
  for (let i = 0; i < 3; i++) await page.waitForLoadState("networkidle", { timeout: 20000 }).catch(() => {});
  await page.waitForFunction(() => document.getAnimations().length === 0, null, { timeout: 8000 }).catch(() => {});
  await page.waitForLoadState("networkidle", { timeout: 20000 }).catch(() => {});
}

/** Close whatever the previous scenario left open, the way a person would. */
async function dismiss(page) {
  await page.keyboard.press("Escape").catch(() => {});
  await page.keyboard.press("Escape").catch(() => {});
  await page
    .waitForFunction((selector) => !document.querySelector(selector), PANES, { timeout: 5000 })
    .catch(() => {});
  await quiet(page);
}

/** Open a playlist page by clicking the first playlist card of the library —
 *  the page that carries O17's and O20's triggers. */
async function openPlaylist(page) {
  await at(page, "#/library/playlists");
  const card = page.locator('[role="tabpanel"] .pl-grid .card[role="button"]').first();
  await card.waitFor({ timeout: 30000 });
  await quiet(page);
  await card.click();
  await page.waitForFunction(() => location.hash.startsWith("#/playlist/"), null, { timeout: 30000 });
  await page.waitForSelector("button[aria-label^=\"Plus d'actions\"], button[aria-label=\"Ajouter des titres\"]", {
    timeout: 30000,
  });
  await quiet(page);
}

/** The runner's frames: the enter is still on the timeline when it reads, and
 *  a one-frame remnant (a 0.01 ms colour transition moved by the focus the
 *  sheet takes on mount) is gone. */
const frames = (page, n = 3) => page.evaluate((count) => window.__motion.frames(count), n);

/**
 * The pane is mounted, the enter is still playing, and the runner's two checks
 * can be taken. In ECO there is nothing left to sample — the enter is instant
 * — so the page is allowed to come all the way to rest: the runner's eco check
 * asserts an EMPTY timeline, and a sheet takes the focus on mount, which
 * restarts a 0.01 ms colour transition on the field it focuses.
 */
async function settleOn(page, selector) {
  await page.waitForSelector(selector, { timeout: 15000 });
  await frames(page);
  if (await page.evaluate(() => document.documentElement.dataset.eco === "1")) {
    await page.waitForFunction(() => document.getAnimations().length === 0, null, { timeout: 5000 }).catch(() => {});
  }
}

/** A row this fixture cannot reach: say why and do nothing. */
function unreachable(row, why) {
  console.log(`  · ${row} non joué par ce banc : ${why}`);
}

export const scenarios = [
  // O16 — FolderSheet, "Nouveau dossier" from the library's add menu.
  {
    name: "O16 dossier",
    midway: true,
    async setup(page) {
      await dismiss(page);
      await at(page, "#/library");
      await page.waitForSelector('button[aria-label="Ajouter à la bibliothèque"]', { timeout: 30000 });
      await quiet(page);
    },
    async act(page) {
      await page.click('button[aria-label="Ajouter à la bibliothèque"]');
      await page.getByRole("menuitem", { name: /Nouveau dossier/ }).click();
    },
    async settle(page) {
      await settleOn(page, '.sheet[aria-label="Nouveau dossier"]');
    },
  },

  // O17 — MoveToFolderSheet, the playlist page's ⋯ menu.
  {
    name: "O17 déplacer",
    midway: true,
    async setup(page) {
      await dismiss(page);
      await openPlaylist(page);
      await page.waitForSelector("button[aria-label^=\"Plus d'actions\"]", { timeout: 30000 });
    },
    async act(page) {
      await page.click("button[aria-label^=\"Plus d'actions\"]");
      await page.getByRole("menuitem", { name: /Déplacer vers un dossier/ }).click();
    },
    async settle(page) {
      await settleOn(page, '.sheet[aria-label="Déplacer vers"]');
    },
  },

  // O18 — InviteSheet, admin → Utilisateurs → « Inviter ».
  {
    name: "O18 invitation",
    midway: true,
    async setup(page) {
      await dismiss(page);
      await at(page, "#/settings/users");
      await page.waitForSelector('button[aria-label^="Ouvrir le compte de"]', { timeout: 30000 });
      await quiet(page);
    },
    async act(page) {
      await page.getByRole("button", { name: /Inviter/ }).first().click();
    },
    async settle(page) {
      await settleOn(page, '.sheet[aria-labelledby="invite-title"]');
    },
  },

  // O19 — AccountPicker: needs a linked account (Réglages → Sources → Mes comptes).
  {
    name: "O19 sélecteur de compte",
    midway: true,
    async setup(page) {
      await dismiss(page);
      await at(page, "#/settings/sources");
      await page.waitForSelector("main", { timeout: 30000 });
      await quiet(page);
      state.ready = (await page.getByRole("button", { name: /Choisir les artistes/ }).count()) > 0;
      if (!state.ready) unreachable("O19", "aucun compte Deezer relié, donc aucun bouton « Choisir les artistes… »");
    },
    async act(page) {
      if (!state.ready) return;
      await page.getByRole("button", { name: /Choisir les artistes/ }).first().click();
    },
    async settle(page) {
      if (!state.ready) return frames(page);
      await settleOn(page, '.sheet[aria-label="Choisir ce qui est importé"]');
    },
  },

  // O20 — AddTracksSheet, the playlist page's « Ajouter des titres ».
  {
    name: "O20 ajouter des titres",
    midway: true,
    async setup(page) {
      await dismiss(page);
      await openPlaylist(page);
      await page.waitForSelector('button[aria-label="Ajouter des titres"]', { timeout: 30000 });
    },
    async act(page) {
      await page.click('button[aria-label="Ajouter des titres"]');
    },
    async settle(page) {
      await settleOn(page, '.sheet[aria-label="Ajouter des titres"]');
    },
  },

  // O21 — PartySheet, the listen-party button of the player bar.
  {
    name: "O21 écoute partagée",
    midway: true,
    async setup(page) {
      await dismiss(page);
      await at(page, "#/library");
      await page.waitForSelector("footer.player", { timeout: 30000 });
      await quiet(page);
      state.ready = (await page.locator('button[aria-label^="Listen party"]:visible').count()) > 0;
      if (!state.ready)
        unreachable("O21", "le shell téléphone cache cette rangée : le bouton est dans le lecteur plein écran");
    },
    async act(page) {
      if (!state.ready) return;
      await page.locator('button[aria-label^="Listen party"]:visible').first().click();
    },
    async settle(page) {
      if (!state.ready) return frames(page);
      await settleOn(page, '.sheet[aria-label="Listen party"]');
    },
  },

  // O22 — RemoteSheet, the remote-control button of the player bar.
  {
    name: "O22 télécommande",
    midway: true,
    async setup(page) {
      await dismiss(page);
      await at(page, "#/library");
      await page.waitForSelector("footer.player", { timeout: 30000 });
      await quiet(page);
      state.ready = (await page.locator('button[aria-label^="Contrôle à distance"]:visible').count()) > 0;
      if (!state.ready)
        unreachable("O22", "le shell téléphone cache cette rangée : le bouton est dans le lecteur plein écran");
    },
    async act(page) {
      if (!state.ready) return;
      await page.locator('button[aria-label^="Contrôle à distance"]:visible').first().click();
    },
    async settle(page) {
      if (!state.ready) return frames(page);
      await settleOn(page, '.sheet[aria-label="Contrôle à distance"]');
    },
  },

  // O23 — ChapterSheet: a podcast episode's chapters (the bench seeds none).
  {
    name: "O23 chapitres",
    midway: true,
    async setup(page) {
      await dismiss(page);
      await at(page, "#/library/podcasts");
      await page.waitForSelector('[role="tabpanel"]', { timeout: 30000 }).catch(() => {});
      await quiet(page);
      state.ready = (await page.locator('[role="tabpanel"] a[href*="/podcast/"]').count()) > 0;
      if (!state.ready) unreachable("O23", "le banc ne sème aucun podcast (tools/perf_api.py --podcasts)");
    },
    async act(page) {
      // The show page opens the chapters from the episode's own chip; with no
      // show in the fixture there is nothing to click, and the reason is above.
      if (!state.ready) return;
    },
    async settle(page) {
      await frames(page);
    },
  },

  // O24 — LyricCard: the lyrics panel's « Créer une carte ».
  {
    name: "O24 carte de paroles",
    midway: true,
    async setup(page) {
      await dismiss(page);
      await at(page, "#/library");
      await page.waitForSelector("main", { timeout: 30000 });
      await quiet(page);
      state.ready = (await page.getByRole("button", { name: /Créer une carte/ }).count()) > 0;
      if (!state.ready) unreachable("O24", "l'ouverture se fait depuis le lecteur plein écran, sur un titre à paroles");
    },
    async act(page) {
      if (!state.ready) return;
      await page.getByRole("button", { name: /Créer une carte/ }).first().click();
    },
    async settle(page) {
      if (!state.ready) return frames(page);
      await settleOn(page, ".lc-backdrop .lc-sheet");
    },
  },
];
