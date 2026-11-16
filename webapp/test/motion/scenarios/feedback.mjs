// Lot T5 (feedback) of docs/agents/transitions.md: one scenario per row of the
// inventory that a person can TRIGGER BY HAND, in the runner's own shape
// (`{ name, setup(page), act(page), settle(page), midway }`).
//
// Here:
//   F1  toasts      — a row's heart ("Ajouté aux favoris") rises in, and the
//                     toast is dismissed by tapping it;
//   F2  notices     — the persistent notice this bench's server produces (its
//                     Deezer proxy is off) is dismissed by its ×;
//   F6  load-failed — a 500 on /api/me/albums paints LoadFailed, and
//                     « Réessayer » asks once more (the block re-enters).
//
// Deliberately absent, with the reason each one has no clickable trigger here:
//   F3  the network pill follows the CONNECTION (CDP offline), not a click, and
//       cutting the link mid-run would change the request multiset the runner
//       compares across the three levels;
//   F4/F5/P23  the remote banner, the host chip and their top↔bottom move only
//       exist on a real remote session, which needs two accounts and a grant:
//       test/remote/run.mjs drives them end to end, and its --shots pictures are
//       what shows F4/F5/P23 on a real screen;
//   F7  the skeleton is what a screen shows while it waits — no trigger of its
//       own (its shimmer is decorative: eco stops it through the central guard,
//       prefers-reduced-motion makes it a still --s2 block, and both are CSS, so
//       a scenario would only be photographing a screen that is waiting);
//   F8  the source badge only speaks for a YouTube title, and this bench's
//       library has none.
//
// EVERY scenario quiets the page before it is measured: the home keeps fetching
// sleeves for a while after it paints, and those requests would otherwise land
// inside the act's window and make the runner's "same requests in the three
// levels" comparison flip a coin. `quiet()` is that wait, and it is also what
// lets a 0.01 ms eco transition finish before the runner looks at the timeline.

const frames = (page, n = 2) => page.evaluate((count) => window.__motion.frames(count), n);

/** No resource asked for 2.2 s, then enough real time for the longest entrance
 *  we can leave running on this page to finish, then two frames.
 *
 *  NOT `page.waitForLoadState("networkidle")`: that resolves at once here — the
 *  lifecycle event fired at the document's own load, long before the scenario,
 *  and it never fires again for the same document (measured: the covers were
 *  still arriving after it "resolved"). The page's resource timeline is the
 *  clock that sees `fetch`, an `<img>` and an `/api` call alike.
 *
 *  It also has to clear the app's own background work, or the runner's "same
 *  requests in the three levels" comparison flips a coin: a long list loads its
 *  sleeves six at a time, and `lib/coverqueue.js` holds a hung one's slot for
 *  4 s. `seedWarmCache()` takes the other source out of the picture (the
 *  playlist warmer). */
async function quiet(page) {
  await page
    .waitForFunction(
      () => {
        const mark = (window.__motionQuiet = window.__motionQuiet || { count: -1, at: 0 });
        const count = performance.getEntriesByType("resource").length;
        if (count !== mark.count) {
          mark.count = count;
          mark.at = performance.now();
          return false;
        }
        // 4.6 s: longer than `lib/coverqueue.js`'s STALL_MS (4 s), the time a
        // hung cover holds its slot before the queue moves on. A shorter window
        // closes in the middle of a list's own cover load and the rest of it
        // lands inside the act's window (measured: 15 sleeves, eco run only).
        return performance.now() - mark.at > 4600;
      },
      null,
      { timeout: 90000, polling: 100 },
    )
    .catch(() => {});
  await page.waitForTimeout(420);
  await frames(page, 2);
}

/** The bench's own server has its Deezer proxy off, so the app's health check
 *  plants one persistent notice at boot. Clicking its × is F2's dismissal. */
const noticeX = '.notice button[aria-label="Masquer"]';

/** The app warms its playlist cache 10 s after boot, then one `/api/playlist/…`
 *  every 1.5 s for twenty-five of them (`lib/actions.js#warmPlaylists`) — its
 *  own background work, never anything a scenario did, but it would land inside
 *  any measured window and make the runner's request comparison a coin flip.
 *  Filling the cache here exactly as the warmer would leave it makes the
 *  background pass skip every path (the same fixture test/menu/run.mjs uses). */
async function seedWarmCache(page) {
  await page.evaluate(async () => {
    const me = await fetch("/api/me").then((r) => r.json());
    const uid = me && me.user && me.user.id;
    if (uid == null) return;
    const owner = location.origin + "/id:" + String(uid);
    const lists = await fetch("/api/me/playlists").then((r) => r.json());
    const paths = ["/me/favorites", ...((lists && lists.playlists) || []).slice(0, 25).map((p) => "/playlist/" + p.id)];
    const db = await new Promise((resolve, reject) => {
      const req = indexedDB.open("nsupy-apicache", 2);
      req.onupgradeneeded = () => {
        const d = req.result;
        if (d.objectStoreNames.contains("responses")) d.deleteObjectStore("responses");
        d.createObjectStore("responses", { keyPath: "key" });
      };
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => reject(req.error);
    });
    const tx = db.transaction("responses", "readwrite");
    const store = tx.objectStore("responses");
    for (const path of paths) store.put({ key: owner + "\u0000" + path, owner, path, text: "{}", ts: Date.now() });
    await new Promise((resolve, reject) => {
      tx.oncomplete = resolve;
      tx.onerror = () => reject(tx.error);
    });
  });
}

/** A 500 on one endpoint, removable, with a hit counter (the runner compares
 *  the request multisets across motion levels, so the count has to be stable). */
function breakRoute(pattern) {
  const state = { hits: 0 };
  const handler = async (route) => {
    state.hits++;
    await route.fulfill({ status: 500, contentType: "application/json", body: JSON.stringify({ error: "boom" }) });
  };
  return {
    state,
    async on(page) {
      await page.context().route(pattern, handler);
    },
    async off(page) {
      await page.context().unroute(pattern, handler);
    },
  };
}

const broken = breakRoute(/\/api\/me\/albums$/);

export const scenarios = [
  {
    // F1 — `rise` from +8 px (the toast lives above the player bar) over 200 ms,
    // `flipSmall` on the stack, 140 ms out on the tap that dismisses it.
    //
    // The action is « Vider le cache » (Réglages › Serveur), not a list row: a
    // heart added or removed changes what a list CONTAINS, so a row enters or
    // leaves the window and asks for its sleeve — three requests whose identity
    // depends on which row was pushed in (measured: two runs of the same page,
    // two different sleeves). This one click asks for one thing and moves no
    // list, so the window is the same every run.
    name: "f1-toasts",
    async setup(page) {
      // Before the warmer wakes up: from here on the page has no background
      // traffic of its own, so `quiet()` waits on the act's own work only.
      await seedWarmCache(page);
      await page.evaluate(() => (location.hash = "#/settings/server"));
      await page.waitForSelector('button:text-is("Vider le cache")', { timeout: 30000 });
      // The settings page can open on a remembered scroll; the shot is meant to
      // show the pill over a still page, and the scroll must not be a variable
      // (the notice strip is sticky and has no fill, so a scrolled page would
      // photograph it over the cards).
      await page.evaluate(() => document.querySelector("main")?.scrollTo({ top: 0, behavior: "instant" }));
      await quiet(page);
    },
    async act(page) {
      await page.locator('button:text-is("Vider le cache")').first().click();
      // The toast is pushed once the server has answered: wait on the pill
      // itself, never on a clock.
      await page.waitForSelector(".toast", { timeout: 15000 });
    },
    midway: true,
    async settle(page) {
      await frames(page, 2);
      // Let the 200 ms entry finish, then tap the pill (the toast is its own
      // dismiss button) so the 140 ms exit plays too.
      await page.waitForTimeout(260);
      const toast = page.locator(".toast").first();
      if (await toast.count()) await toast.click({ timeout: 5000 });
      await page.waitForFunction(() => !document.querySelector(".toast"), null, { timeout: 5000 });
      await quiet(page);
    },
  },

  {
    // F2 — `rise` from −8 px (the strip hangs under the top bar) over 200 ms,
    // 140 ms out when the × is tapped, `flipSmall` closing the gap.
    name: "f2-notices",
    async setup(page) {
      await page.waitForSelector(noticeX, { timeout: 30000 });
      await quiet(page);
    },
    async act(page) {
      await page.locator(noticeX).first().click();
    },
    midway: true,
    async settle(page) {
      await page
        .waitForFunction((selector) => !document.querySelector(selector), noticeX, { timeout: 5000 })
        .catch(() => {});
      await quiet(page);
    },
  },

  {
    // F6 — LoadFailed enters with `rise` (opacity + 4 px, 140 ms) when the
    // failing load lands, and re-enters when « Réessayer » fails again (the
    // retry clears the error and re-nulls the list, so the block really is
    // rebuilt). The Albums leaf is the one whose payload nothing else loads at
    // boot, so the block is LoadFailed and not a list painted from cache.
    name: "f6-load-failed",
    async setup(page) {
      await broken.on(page);
      await page.evaluate(() => (location.hash = "#/library/albums"));
      await page.waitForSelector(".failed", { timeout: 30000 });
      await quiet(page);
    },
    async act(page) {
      await page.locator('.failed button:has-text("Réessayer")').first().click();
      // The block is destroyed (error cleared) and rebuilt on the failure.
      await page.waitForSelector(".failed", { timeout: 15000 });
    },
    midway: true,
    async settle(page) {
      await frames(page, 2);
      await page.waitForTimeout(220);
      // The failing route goes away, so the leaf works for whatever runs next;
      // the page itself stays here on purpose — navigating home would load a
      // whole screen's sleeves inside this measured window, and their tail is
      // what makes the request multiset differ between two runs of the same
      // page (measured before this line existed).
      await broken.off(page);
      await quiet(page);
    },
  },
];
