// T6d — Réglages (inventory I13), one scenario per row that has a clickable
// trigger in the bench's library.
//
// Every scenario navigates by hash (svelte-spa-router) and leaves the state
// change in the SAME handler as the click: the animation is what the runner
// then looks at, never something the act waits for (M2). `settle` is two
// frames, so the enter is still running when the checks read the timeline —
// in eco nothing must be there, under reduced nothing may carry a transform.
//
// NOT reachable on tools/perf_api.py --serve (an empty provider list and an
// empty "Indisponibles" list), so those rows have no scenario here and the lot
// relies on the source-level rules for them: SourceOrder's reorder (needs two
// fallbacks), AutomatchPanel's detail (needs a failed run) and JobFailures'
// accordion (needs failures), UnavailableList (needs unavailable tracks).

const frames = (page, n = 2) => page.evaluate((count) => window.__motion.frames(count), n);

/** Setup only: let the SHELL finish. The player bar's and the sidebar's own
 *  covers (and anything a route change triggers) load after the screen is in,
 *  and their requests would otherwise land inside the measured window — which
 *  criterion 5 compares, as a multiset, across the three levels. */
async function quiet(page) {
  await page.waitForLoadState("networkidle", { timeout: 15000 }).catch(() => {});
  await page.waitForFunction(() => window.__motion.running().length === 0, null, { timeout: 15000 }).catch(() => {});
}

/** A dialog left open by the previous scenario (same route: the Settings
 *  component keeps its state) would cover the next click. Close it and wait
 *  for its exit to finish — the overlay is `inert` while it plays. */
async function closeDialogs(page) {
  if (!(await page.locator('.dialog[role="dialog"]').count())) return;
  await page.evaluate(() => {
    document
      .querySelectorAll('.dialog[role="dialog"] button[aria-label="Fermer"]')
      .forEach((button) => button.click());
  });
  await page.waitForSelector('.dialog[role="dialog"]', { state: "detached", timeout: 15000 });
}

/** Open Réglages (its chunk is async: wait for whichever screen renders). */
async function settings(page, hash = "#/settings") {
  await closeDialogs(page);
  await page.evaluate((h) => {
    location.hash = h;
  }, hash);
  await page.waitForSelector(".side-title, .pane-title, .pane .sub-head", { timeout: 30000 });
  await quiet(page);
}

/** A category of the list — "Apparence" is the one row every account has that
 *  is not the desktop's default section. */
const category = (page) => page.locator(".cat", { hasText: "Apparence" }).first();

export const scenarios = [
  {
    // I13: Settings category → section. Same `screenKey` (/settings), so the
    // page does not enter again: on the desktop the section's content fades in
    // (`fadeIn`, keyed on the section), and on a phone the pane itself rises
    // (the two-level list → section move).
    name: "reglages-section",
    setup: (page) => settings(page),
    act: async (page) => {
      await category(page).click();
    },
    settle: (page) => frames(page, 4),
  },

  {
    // I13: the phone's way back, list ↔ section. A desktop has no back button
    // (the list is a column), so the act is a no-op there and the run only
    // proves the setup did not leave anything running.
    name: "reglages-retour",
    setup: async (page) => {
      await settings(page);
      await category(page).click();
      await page.waitForSelector(".pane", { timeout: 15000 });
      await quiet(page);
    },
    act: async (page) => {
      const back = page.locator(".pane .sub-head .back").first();
      if (await back.count()) await back.click();
    },
    settle: (page) => frames(page, 4),
  },

  {
    // I13 / I5: a switch. The thumb (transform) and the track (colour) both
    // move over `--dur-instant`; under reduced motion the CSS drops the
    // displacement, which is what the runner's "no transform" check reads.
    name: "reglages-interrupteur",
    setup: async (page) => {
      await settings(page, "#/settings/downloads");
      await page.waitForSelector('button.toggle[role="switch"]', { timeout: 15000 });
    },
    act: async (page) => {
      await page.locator('button.toggle[role="switch"]').first().click();
    },
    settle: (page) => frames(page, 4),
  },

  {
    // I13: AdminUsers' detail dialog — `veil` on the scrim (its own
    // background-colour alpha, M8) and `dialog` on the pane, both inside the
    // same `{#if}`.
    name: "reglages-utilisateur",
    setup: async (page) => {
      await settings(page, "#/settings/users");
      await page.waitForSelector('button[aria-label^="Ouvrir le compte de"]', { timeout: 30000 });
    },
    act: async (page) => {
      await page.locator('button[aria-label^="Ouvrir le compte de"]').first().click();
      await page.waitForSelector('.dialog[role="dialog"]', { timeout: 15000 });
    },
    settle: (page) => frames(page, 4),
  },

];
