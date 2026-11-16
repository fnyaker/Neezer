// T2a — the menus (docs/agents/transitions.md, inventory O1–O6). One scenario
// per row that has a clickable trigger:
//
//   O1  contextual menu, desktop ....... pop, from the corner nearest the pointer
//   O2  contextual menu, phone (≤640) .. sheet + veil on the scrim (no keyframes)
//   O3  its sub-menu ................... collapse (≤ 12 items, M10)
//   O4  quality list ................... pop, origin from placement/align
//   O5  sleep timer's list ............. idem
//   O6  podcast speed chip ............. idem
//
// The runner walks each scenario at 1440×900 and 390×844, in the three motion
// levels. `act` performs the click and returns mid-animation (that is the frame
// SHOTS keeps); `settle` waits two frames only, so the runner's eco ("nothing
// animates") and reduced ("no transform, ≤ 140 ms") checks read the animation
// WHILE it runs instead of after it.
//
// O6's chip only exists while a podcast episode is playing, and the motion
// bench's server (tools/perf_api.py --serve, started by run.mjs without
// `--podcasts`) seeds none: that scenario is a no-op there and runs wherever a
// podcast is in the queue. O4/O5 reach the phone's own controls by opening the
// full-screen player, which is the only place `footer.player .extra` lives on a
// phone (it is `display: none` in the mini card).

import { silence } from "../../harness.mjs";

const WAV = silence();
const WAIT = { timeout: 30000 };

/** Is this the phone viewport of the bench (390×844)? */
const phone = (page) => (page.viewportSize()?.width || 1440) <= 640;

const frames = (page, n = 2) =>
  page.evaluate(
    (count) =>
      new Promise((done) => {
        const step = () => (--count <= 0 ? done() : requestAnimationFrame(step));
        requestAnimationFrame(step);
      }),
    n,
  );

/**
 * A real 60 s silence for /api/stream. The synthetic library has no audio
 * files: without this the player fails the load and skips a title mid-scenario,
 * which would add requests that depend on the machine's speed. Registered once
 * per page (a page = one viewport × one motion level) and never removed, so
 * every level of the same viewport sees the same page.
 *
 * The artwork watcher is installed HERE, before the first navigation, so the
 * count is honest: a listener attached later sees `requestfinished` for requests
 * whose `request` it missed and reads a negative number (which made an earlier
 * version of `quiet()` return while the covers were still arriving).
 */
async function arm(page) {
  if (!page.__menusArmed) {
    const art = { inflight: 0 };
    const seen = new WeakSet();
    const isArt = (request) => {
      const url = new URL(request.url());
      return (
        request.resourceType() === "image" ||
        url.pathname.startsWith("/api/cover/") ||
        url.pathname.startsWith("/images/cover/")
      );
    };
    page.on("request", (request) => {
      if (!isArt(request)) return;
      seen.add(request);
      art.inflight++;
    });
    const done = (request) => {
      if (!seen.has(request)) return;
      seen.delete(request);
      art.inflight--;
    };
    page.on("requestfinished", done);
    page.on("requestfailed", done);
    page.__menusArt = art;
    page.__menusArmed = true;
  }
  if (!page.__menusStream) {
    page.__menusStream = true;
    await page.route(/\/api\/stream\//, (route) =>
      route.fulfill({
        status: 200,
        body: WAV,
        headers: { "Content-Type": "audio/wav", "Accept-Ranges": "none" },
      }),
    );
  }
}

/** A hash navigation: the SPA owns the URL, the bench does not reload it. */
async function go(page, hash) {
  await page.evaluate((h) => {
    location.hash = h;
  }, hash);
}

/**
 * What a person does between two menus: Escape. On a phone that also closes
 * the full-screen player (ImmersivePlayer listens for it), which is why every
 * scenario that needs the player re-opens it in its own setup.
 */
async function clean(page) {
  await page.keyboard.press("Escape");
  await frames(page, 2);
  await page.waitForTimeout(300);
  if (await page.locator(".menu[role=menu]").count()) {
    await page.locator(".backdrop").first().click({ force: true }).catch(() => {});
    await page.waitForTimeout(300);
  }
}

/**
 * Fill the offline metadata cache exactly as test/menu/run.mjs does, so
 * `lib/actions.js#warmPlaylists` (one /api/playlist request every 1.5 s, ten
 * seconds after boot) skips every path instead of firing inside an act window.
 * The cache is a pure fallback (api.js always hits the network online), so a
 * placeholder entry costs the page nothing.
 */
async function primeCache(page) {
  if (page.__menusPrimed) return;
  page.__menusPrimed = true;
  await page.evaluate(async () => {
    const me = await fetch("/api/me").then((r) => r.json());
    const uid = me?.user?.id;
    if (uid == null) return;
    const owner = location.origin + "/id:" + String(uid);
    const lists = await fetch("/api/me/playlists").then((r) => r.json());
    const paths = ["/me/favorites", ...(lists.playlists || []).slice(0, 25).map((p) => "/playlist/" + p.id)];
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
    for (const path of paths) {
      store.put({ key: owner + "\u0000" + path, owner, path, text: "{}", ts: Date.now() });
    }
    await new Promise((resolve, reject) => {
      tx.oncomplete = resolve;
      tx.onerror = () => reject(tx.error);
    });
  });
}

/**
 * Wait until no artwork is in flight, then past the cover's own 350 ms fade.
 * A cover that decodes during `act` starts a `loaded` transition the runner
 * would read as part of the menu's animation window (and a Cover fade is T4's
 * row, not this lot's); one quiet check after 1.3 s keeps the window clean.
 * Measured on this page: the whole artwork burst is 168 requests over ~3 s, and
 * coverqueue.js re-measures its pending covers once a second, so the window has
 * to be longer than that or an admission slips through the gap.
 */
async function quiet(page, limit = 20000) {
  const art = page.__menusArt || { inflight: 0 };
  const until = Date.now() + limit;
  while (Date.now() < until) {
    if (art.inflight <= 0) {
      await page.waitForTimeout(1300);
      if (art.inflight <= 0) return;
    } else {
      await page.waitForTimeout(50);
    }
  }
}

/**
 * A page with track rows, reached without knowing an id: Bibliothèque →
 * Playlists → the first card. `.row.track` is TrackRow's own class, the surface
 * test/menu/run.mjs already drives at both viewports.
 *
 * The row is centred HERE (a sticky top bar would otherwise swallow the click)
 * and not inside `act`: a scroll mounts rows and lets their covers in, and
 * `coverqueue.js` trickles those over the next seconds — a request inside the
 * act's window that has nothing to do with the menu.
 */
async function ensureRows(page) {
  if (!(await page.locator(".row.track").count())) {
    await go(page, "#/library/playlists");
    await page.waitForSelector(".card", WAIT);
    await page.waitForTimeout(250);
    await page.locator(".card").first().click({ position: { x: 8, y: 8 } });
    await page.waitForSelector(".row.track", WAIT);
    await page.waitForTimeout(600);
  }
  await page
    .locator(".row.track")
    .first()
    .evaluate((el) => el.scrollIntoView({ block: "center", inline: "nearest" }))
    .catch(() => {});
  await page.waitForTimeout(250);
  await quiet(page);
}

/**
 * A track in the bar. Only a PHONE needs one: its quality and sleep controls
 * live in the full-screen player, while the desktop bar shows them with no
 * track at all — so a desktop run starts no playback and pays for nothing.
 *
 * The player answers a first play with two delayed background requests (the
 * next track's archive at 4 s, its on-device prefetch at 12 s). They are the
 * app doing rule 1b, not the menu, but they would land in whichever window
 * happened to be open — so they are waited out HERE, in a setup, before any
 * act.
 */
async function ensurePlaying(page) {
  if (!phone(page)) return;
  const title = page.locator("footer.player .now .t");
  if ((await title.count()) && (await title.first().innerText()).trim()) return;
  const started = Date.now();
  const all = page.locator("button.btn-primary", { hasText: "Lire" }).first();
  if (await all.count()) await all.click();
  else await page.locator(".row.track .play").first().click();
  await page.waitForFunction(
    () => {
      const el = document.querySelector("footer.player .now .t");
      return !!el && !!el.textContent.trim();
    },
    null,
    { timeout: 20000 },
  );
  await page.waitForTimeout(800);
  await quiet(page);
  const left = 14500 - (Date.now() - started);
  if (left > 0) await page.waitForTimeout(left);
  await page.waitForTimeout(1200);
  // Then pause. The phone bar's playhead is a live progress line with a CSS
  // width transition: in eco it leaves 0 ms remnants that the runner reads as
  // "something animates" — a live indicator must have none by rule (M6), and
  // the file is T3's. A paused bar updates nothing, so the windows stay clean.
  const pp = page.locator("footer.player .pp").first();
  if (await pp.count()) await pp.click();
  await page.waitForTimeout(300);
}

/** The phone's full-screen player holds the same controls as the desktop bar. */
async function openPlayer(page) {
  if (!phone(page)) return;
  if (!(await page.locator(".m").count())) {
    await page.locator("footer.player .now").first().click();
    await page.waitForSelector(".m", WAIT);
  }
  await page.waitForTimeout(450);
}

/** `.qm` / `.sl` / `.sp` trigger, in the bar or inside the phone's player. */
function trigger(page, cls) {
  return phone(page) ? `.m ${cls} .trigger` : `footer.player ${cls} .trigger`;
}

/** The contextual menu, opened the way a person does on each device. */
async function openContextMenu(page) {
  const row = page.locator(".row.track").first();
  if (phone(page)) {
    await row.evaluate((el) => {
      const r = el.getBoundingClientRect();
      el.dispatchEvent(
        new MouseEvent("contextmenu", {
          bubbles: true,
          cancelable: true,
          clientX: Math.round(r.left + Math.min(24, r.width / 2)),
          clientY: Math.round(r.top + Math.min(20, r.height / 2)),
        }),
      );
    });
  } else {
    const box = await row.boundingBox();
    if (!box) throw new Error("no box for .row.track");
    await page.mouse.click(box.x + Math.min(24, box.width / 2), box.y + Math.min(20, box.height / 2), {
      button: "right",
    });
  }
  await page.waitForSelector(".menu[role=menu]", { timeout: 10000 });
}

/** `settle`: leave the enter animation running for the runner's own checks. */
const settle = (page) => frames(page, 2);

export const scenarios = [
  // O1 — desktop pop. On a phone the same click opens the sheet, so this row's
  // own shape is only observable above 640 px; the action is the same either way.
  {
    name: "O1 menu contextuel",
    midway: true,
    setup: async (page) => {
      await arm(page);
      await primeCache(page);
      await clean(page);
      await ensureRows(page);
      // The phone's later rows need a track, and starting it here means its two
      // delayed prefetches are over before the first act.
      await ensurePlaying(page);
    },
    act: async (page) => {
      await openContextMenu(page);
      await page.waitForTimeout(phone(page) ? 120 : 80);
    },
    settle,
  },

  // O2 — the phone sheet + its veil. It is the same menu, so the action is
  // identical; below 640 px the sheet slides up over a tinted scrim.
  {
    name: "O2 menu contextuel (feuille)",
    midway: true,
    setup: async (page) => {
      await arm(page);
      await primeCache(page);
      await clean(page);
      await ensureRows(page);
    },
    act: async (page) => {
      await openContextMenu(page);
      await page.waitForTimeout(120);
    },
    settle,
  },

  // O3 — the sub-menu (a track's « Télécharger en… », or the artist picker):
  // `collapse`, and the chevron turns on the same standard curve.
  {
    name: "O3 sous-menu",
    midway: true,
    setup: async (page) => {
      await arm(page);
      await primeCache(page);
      await clean(page);
      await ensureRows(page);
      await openContextMenu(page);
      await page.waitForTimeout(450);
    },
    act: async (page) => {
      const sub = page.locator(".menu[role=menu] .item.has-sub").first();
      if (!(await sub.count())) throw new Error("this menu has no item with a sub-menu");
      await sub.click();
      await page.waitForTimeout(80);
    },
    settle,
  },

  // O4 — the quality list, from the badge (bar on a desktop, the full-screen
  // player's header on a phone), at `placement`/`align`'s own corner.
  {
    name: "O4 qualité",
    midway: true,
    setup: async (page) => {
      await arm(page);
      await primeCache(page);
      await clean(page);
      await ensureRows(page);
      await ensurePlaying(page);
      await openPlayer(page);
      await quiet(page);
    },
    act: async (page) => {
      await page.locator(trigger(page, ".qm")).first().click();
      await page.waitForTimeout(80);
    },
    settle,
  },

  // O5 — the sleep timer's list: same pop, from the moon's corner.
  {
    name: "O5 minuteur",
    midway: true,
    setup: async (page) => {
      await arm(page);
      await primeCache(page);
      await clean(page);
      await ensureRows(page);
      await ensurePlaying(page);
      await openPlayer(page);
      await quiet(page);
    },
    act: async (page) => {
      await page.locator(trigger(page, ".sl")).first().click();
      await page.waitForTimeout(80);
    },
    settle,
  },

  // O6 — the podcast speed chip. Absent on the motion bench (no podcast is
  // seeded by run.mjs's server): the scenario then does nothing and the
  // runner's checks are vacuously true, which is noted in the PR.
  {
    name: "O6 vitesse",
    midway: true,
    setup: async (page) => {
      await arm(page);
      // No podcast, no chip: do nothing at all rather than re-open the player
      // for an act that will not happen (that would mount rows and admit their
      // covers, which the runner would read as this row's requests).
      if (!(await page.locator(trigger(page, ".sp")).count())) return;
      await primeCache(page);
      await clean(page);
      await ensureRows(page);
      await ensurePlaying(page);
      await openPlayer(page);
      await quiet(page);
    },
    act: async (page) => {
      const chip = page.locator(trigger(page, ".sp")).first();
      if (!(await chip.count())) return;
      await chip.click();
      await page.waitForTimeout(80);
    },
    settle,
  },
];
