// Lot T2b — the first family of sheets (inventory rows O7–O15 of
// docs/agents/transitions.md), one scenario per row that has a trigger a
// person can click in this bench's fixture.
//
// The runner (test/motion/run.mjs) opens each scenario at 1440×900 and
// 390×844 in three motion levels and checks the plan's cross-cutting rules
// (no request differences, nothing in eco, no transform under reduced
// motion). These scenarios add the two things only the rows know:
//
//   · the enter really is `veil` + `sheet`: the OVERLAY animates its own
//     background-colour alpha and the PANE moves for the row's duration
//     (280 ms on a phone, 200 ms on the desktop, 0 in eco, ≤ 140 ms and no
//     transform under reduced motion). A component that fell back to
//     `svelte/transition`'s fade would fail here;
//   · M8 holds: the overlay's computed `opacity` is 1 while it enters (an
//     opacity on the glass pane's ancestor is what switched its
//     `backdrop-filter` off: measured 5.0 → 127.4 in the plan), and the pane
//     keeps its own 22 px backdrop-filter.
//
// WHY THE ANIMATIONS ARE RECORDED, NOT POLLED. A Playwright click can take
// more than a whole enter duration (measured 63 to 769 ms here, because the
// row's invisible-on-a-mouse ⋯ button is hovered and stabilised first), so
// reading `document.getAnimations()` at settle time raced the 200/280 ms
// enter and sometimes read it already over. The bench installs one wrapper on
// `Element.prototype.animate` before the act and reads the durations AND
// keyframes Svelte actually created for THIS pane and THIS overlay — a paused
// or finished animation still has them.
//
// O13 (PlaylistMembersSheet) has no scenario: its trigger is the playlist
// header's member stack, which only exists for a playlist with more than one
// member, and tools/perf_api.py seeds no shared playlist. Its transition is the
// same `veil` + `sheet` pair the O7–O12 rows assert here, and the plan's own
// row names no other motion for it.

const TRACK_MENU_BUTTON = '.row.track button.more[aria-label="Plus d\'options"]';
const MENU = '.menu[role="menu"]';
const ITEM = `${MENU} [role="menuitem"]`;

const phoneNow = (page) => page.evaluate(() => matchMedia("(max-width: 640px)").matches);
const frames = (page, n = 2) => page.evaluate((n) => window.__motion.frames(n), n);

function fail(message) {
  // A scenario cannot reach the runner's checker; a wrong duration or a lost
  // blur is a bug, so it stops the bench instead of passing silently.
  throw new Error(message);
}

/** Record every WAAPI animation Svelte creates from now on (one wrapper, and
 *  the keyframes/options are copied because Svelte may reuse the arrays). */
async function installRecorder(page) {
  await page.evaluate(() => {
    if (window.__enterLog) {
      window.__enterLog.length = 0;
      return;
    }
    window.__enterLog = [];
    const original = Element.prototype.animate;
    Element.prototype.animate = function (keyframes, options) {
      const animation = original.call(this, keyframes, options);
      const frames = Array.isArray(keyframes) ? keyframes : [keyframes];
      window.__enterLog.push({
        el: this,
        animation,
        duration: Math.round(Number(options && options.duration) || 0),
        keys: frames.flatMap((frame) =>
          Object.keys(frame).filter((key) => !/^(offset|easing|composite|computedOffset)$/.test(key)),
        ),
      });
      return animation;
    };

    // Every <img> the app inserts loads immediately (a bench-only harness, like
    // the recorder above): a sheet that lists covers lazy-loads them by
    // visibility, and which slide of a scrolling list is visible is timing, not
    // behaviour. Without this the playlist picker's window asked for 8 covers
    // in one level and 16 in another.
    const eager = (root) => {
      if (root.nodeType === 1 && root.tagName === "IMG") root.loading = "eager";
      if (root.querySelectorAll) for (const img of root.querySelectorAll("img")) img.loading = "eager";
    };
    for (const img of document.querySelectorAll("img")) img.loading = "eager";
    new MutationObserver((records) => {
      for (const record of records) for (const node of record.addedNodes) eager(node);
    }).observe(document.body, { childList: true, subtree: true });
  });
}

/** A setup is over when the page is really quiet: the rows' covers, the
 *  ground's sleeve and the playlist cache must not land inside the act, or the
 *  "same requests in every level" check becomes a lie (measured before this
 *  guard: 12 /api/cover and 13 /images/cover in one act, 16 covers in the next
 *  level's act and none in the full one). `networkidle` is the browser's own
 *  "nothing asked for 500 ms" signal; the DOM images are then read once. */
async function quiet(page) {
  await page.waitForLoadState("networkidle", { timeout: 15000 }).catch(() => {});
  await settleImages(page);
  await page.waitForTimeout(200);
}

/** Escape out of whatever the previous scenario left open. */
async function cleanup(page) {
  // The app warms its playlist cache ten seconds after boot and then one
  // request every 1.5 s (lib/actions.js#warmPlaylists): those requests land in
  // an arbitrary scenario window and would make the "same requests in every
  // level" check a lie (measured: /api/playlist/<random> in three scenarios).
  // Fill the cache the warmer would leave, exactly as test/menu/run.mjs does,
  // so every path is skipped and an act is measured against a silent page.
  await page.evaluate(async () => {
    if (window.__t2bWarmFilled) return;
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
    window.__t2bWarmFilled = true;
  });
  for (let i = 0; i < 2; i++) {
    await page.keyboard.press("Escape").catch(() => {});
    await page.waitForTimeout(120);
  }
  await page.waitForTimeout(140);
}

/** Let the covers and rows the page mounts finish asking before the measured
 *  window opens, so an in-flight image never lands inside a level's act. */
async function settleImages(page) {
  await page.evaluate(async () => {
    const pending = [...document.images].filter((img) => !img.complete);
    await Promise.race([
      Promise.all(
        pending.map(
          (img) =>
            new Promise((resolve) => {
              img.addEventListener("load", resolve, { once: true });
              img.addEventListener("error", resolve, { once: true });
            }),
        ),
      ),
      new Promise((resolve) => setTimeout(resolve, 1200)),
    ]);
  });
}

/** A hash-only move: no document reload, so the bench's probe survives. */
async function go(page, hash, selector) {
  await page.evaluate((h) => {
    location.hash = h;
  }, hash);
  await page.waitForSelector(selector, { timeout: 40000 });
  await settleImages(page);
  await page.waitForTimeout(220);
  await quiet(page);
}

/** The track page every track-menu row starts from. */
const trackPage = (page) => go(page, "#/library/favorites", ".row.track");

/** Open a track row's menu: the ⋯ on a pointer, the long-press event the
 *  WebView emits on a phone (a phone row hides ⋯, see TrackRow's 640 px block). */
async function openTrackMenu(page) {
  if (await phoneNow(page)) {
    await page.evaluate(() => {
      const el = document.querySelector(".row.track");
      if (!el) throw new Error("no .row.track to long-press");
      el.scrollIntoView({ block: "center", inline: "nearest" });
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
    const more = page.locator(TRACK_MENU_BUTTON).first();
    await more.scrollIntoViewIfNeeded().catch(() => {});
    await more.click();
  }
  await page.waitForSelector(MENU, { timeout: 8000 });
  await page.waitForTimeout(120);
}

async function chooseMenuItem(page, label) {
  await page.locator(ITEM, { hasText: label }).first().click({ timeout: 8000 });
}

/** The album page, reached the way a person reaches it (a card click). */
async function albumPage(page) {
  await go(page, "#/library/albums", ".card");
  const card = page.locator(".card").first();
  await card.scrollIntoViewIfNeeded().catch(() => {});
  await card.click();
  await page.waitForSelector('.actions button[aria-label="Exporter"]', { timeout: 20000 });
  await settleImages(page);
  await page.waitForTimeout(200);
  await quiet(page);
}

/**
 * What the row promised: the animations Svelte actually created for this pane
 * and this overlay (durations and keyframes, read back from the recorder), and
 * the computed style of the glass (M8: no ancestor opacity, the pane's own
 * backdrop-filter intact).
 */
async function inspectEnter(page, { overlay, pane, enter }) {
  const seen = await page.evaluate(
    ({ overlay, pane }) => {
      const overlayEl = [...document.querySelectorAll(overlay)].pop();
      const paneEl = overlayEl && overlayEl.querySelector(pane);
      if (!overlayEl || !paneEl) return { present: false };
      const level =
        document.documentElement.dataset.eco === "1"
          ? "eco"
          : matchMedia("(prefers-reduced-motion: reduce)").matches
            ? "reduced"
            : "full";
      const phone = matchMedia("(max-width: 640px)").matches;
      const mine = (window.__enterLog || []).filter((entry) => entry.el === paneEl || entry.el === overlayEl);
      const animations = mine.map((entry) => ({
        target: entry.el === overlayEl ? "veil" : "pane",
        duration: entry.duration,
        keys: entry.keys,
      }));

      // M8: an opacity (or a filter) on ANY ancestor turns the pane's
      // backdrop-filter off. The overlay is the closest one.
      const ancestors = [];
      for (let el = overlayEl; el && el !== document.body; el = el.parentElement) {
        const style = getComputedStyle(el);
        ancestors.push({ opacity: style.opacity, filter: style.filter });
      }
      return {
        present: true,
        level,
        phone,
        animations,
        ancestors,
        paneBackdrop: getComputedStyle(paneEl).backdropFilter,
        overlayOpacity: getComputedStyle(overlayEl).opacity,
      };
    },
    { overlay, pane },
  );

  if (!seen.present) fail(`${overlay} never mounted its ${pane}`);
  if (seen.level === "eco") {
    const moving = seen.animations.filter((animation) => animation.duration > 0);
    if (moving.length) fail(`eco: ${overlay} still animates — ${JSON.stringify(moving)}`);
    return seen;
  }

  // Svelte creates a 0 ms probe animation first (it samples the `css(t)` curve
  // into keyframes) and then the real one: keep the longest per target.
  const longest = (target) =>
    seen.animations.filter((animation) => animation.target === target).sort((a, b) => b.duration - a.duration)[0];

  if (seen.level === "full") {
    const dur = seen.phone ? enter.phone : enter.desktop;
    const paneAnim = longest("pane");
    if (!paneAnim || !paneAnim.duration) {
      fail(`${overlay}: the pane has no enter animation — ${JSON.stringify(seen.animations)}`);
    }
    if (!paneAnim.keys.includes("transform")) {
      fail(`${overlay}: the pane does not move — ${JSON.stringify(seen.animations)}`);
    }
    if (paneAnim.duration !== dur) {
      fail(`${overlay}: pane enter is ${paneAnim.duration} ms, expected ${dur} — ${JSON.stringify(seen.animations)}`);
    }
    const veilAnim = longest("veil");
    if (!veilAnim || !veilAnim.duration) {
      fail(`${overlay}: the overlay has no veil animation — ${JSON.stringify(seen.animations)}`);
    }
    if (!veilAnim.keys.some((key) => /background/i.test(key))) {
      fail(`${overlay}: the overlay does not fade its own background — ${JSON.stringify(seen.animations)}`);
    }
    // M8, in the keyframes themselves: the veil must never animate `opacity`
    // on the glass pane's ancestor.
    if (veilAnim.keys.includes("opacity")) {
      fail(`${overlay}: the veil animates an ancestor opacity — the pane's blur is lost (M8)`);
    }
    if (veilAnim.duration !== dur) fail(`${overlay}: veil enter is ${veilAnim.duration} ms, expected ${dur}`);
  } else {
    for (const target of ["veil", "pane"]) {
      const animation = longest(target);
      if (!animation || !animation.duration) continue;
      if (animation.duration > 140) fail(`${overlay}: ${target} runs ${animation.duration} ms under reduced motion`);
      const moved = animation.keys.filter((key) => /^(transform|translate|scale|rotate)$/.test(key));
      if (moved.length) fail(`${overlay}: ${target} carries ${moved.join(",")} under reduced motion`);
    }
  }

  if (seen.overlayOpacity !== "1") fail(`${overlay}: the veil's own opacity is ${seen.overlayOpacity} (M8: the blur dies)`);
  for (const ancestor of seen.ancestors) {
    if (ancestor.opacity !== "1") fail(`${overlay}: an ancestor enters at opacity ${ancestor.opacity} — the pane's blur is gone`);
    if (ancestor.filter !== "none") fail(`${overlay}: an ancestor carries filter ${ancestor.filter} — the pane's backdrop is broken`);
  }
  if (seen.paneBackdrop === "none") fail(`${overlay}: the pane has no backdrop-filter while it enters`);
  return seen;
}

/** One sheet, opened by `act`, its enter asserted from the recorded animation. */
function scenario({ name, setup, act, overlay, pane = ".sheet", enter = { phone: 280, desktop: 200 } }) {
  return {
    name,
    midway: true,
    setup: async (page) => {
      await installRecorder(page);
      await cleanup(page);
      await setup(page);
      await quiet(page);
      // Only what this scenario creates: the recorder is global.
      await page.evaluate(() => {
        window.__enterLog.length = 0;
      });
    },
    act: async (page) => {
      await act(page);
      await page.waitForSelector(`${overlay} ${pane}`, { state: "attached", timeout: 30000 });
    },
    settle: async (page) => {
      // Two frames: Svelte creates the sampled enter on the frame after the
      // mount, and eco's 0.01 ms @starting-style ghost is gone by then.
      await frames(page, 2);
      // A sheet that lists covers (the playlist picker) lazy-loads them by
      // visibility; at rest the list is identical in every level, so ask every
      // image the pane renders for its URL now and wait for it — otherwise the
      // lazy boundary lands on a different row per run (measured: 16 covers
      // against 17), and the 350 ms fade of the ones that arrive (Cover.svelte,
      // another lot's file) starts AFTER the settle.
      await page.evaluate(
        async ({ overlay, pane }) => {
          const overlayEl = [...document.querySelectorAll(overlay)].pop();
          const paneEl = overlayEl && overlayEl.querySelector(pane);
          if (!paneEl) return;
          const images = [...paneEl.querySelectorAll("img")];
          for (const img of images) img.loading = "eager";
          await Promise.race([
            Promise.all(
              images.map((img) =>
                img.complete
                  ? null
                  : new Promise((resolve) => {
                      img.addEventListener("load", resolve, { once: true });
                      img.addEventListener("error", resolve, { once: true });
                    }),
              ),
            ),
            new Promise((resolve) => setTimeout(resolve, 2000)),
          ]);
        },
        { overlay, pane },
      );
      // Then let any foreign animation started in the same window (the cover
      // fade above) finish before the runner samples this scenario.
      await page
        .waitForFunction(
          ({ overlay, pane }) => {
            const overlayEl = [...document.querySelectorAll(overlay)].pop();
            const paneEl = overlayEl && overlayEl.querySelector(pane);
            if (!overlayEl || !paneEl) return true;
            return document.getAnimations().every((animation) => {
              const target = animation.effect && animation.effect.target;
              if (!target || !overlayEl.contains(target)) return true;
              return target === overlayEl || target === paneEl;
            });
          },
          { overlay, pane },
          { timeout: 4000, polling: 60 },
        )
        .catch(() => {});
      await inspectEnter(page, { overlay, pane, enter });
      // Let the enter land and the pane's own covers load, so the measured
      // window of every level ends on the same settled sheet (a half-slid one
      // exposes a different slice of its lazy covers: measured 3 cover
      // requests in full against 12 in reduced on the playlist picker).
      await page
        .evaluate(
          () =>
            Promise.race([
              Promise.all((window.__enterLog || []).map((entry) => entry.animation.finished.catch(() => {}))),
              new Promise((resolve) => setTimeout(resolve, 800)),
            ]),
        )
        .catch(() => {});
      await settleImages(page);
      await quiet(page);
      // Leave the page as the next scenario (in this file or another lot's)
      // expects it: no overlay on top. Escape closes it in the same handler
      // the click uses; the outro is inert and short.
      await page.keyboard.press("Escape").catch(() => {});
      await page.waitForSelector(`${overlay} ${pane}`, { state: "detached", timeout: 3000 }).catch(() => {});
      await page.waitForTimeout(120);
    },
  };
}

/** A sheet whose trigger is one of the track menu's entries. */
const fromMenu = (label, name, overlay = ".overlay") =>
  scenario({
    name,
    setup: trackPage,
    act: async (page) => {
      await openTrackMenu(page);
      await chooseMenuItem(page, label);
    },
    overlay,
  });

export const scenarios = [
  // O7 — CreditsSheet, from a track's menu. It used to be `fade` 150 + `scale`
  // 160; the veil now keeps its 22 px backdrop-filter for the whole enter.
  fromMenu("Voir les crédits", "credits"),

  // O8 — ExportSheet, from an album's « Exporter » button (its tab row is I3:
  // the colour walk is a CSS transition on a frequent control, 90 ms).
  scenario({
    name: "album-export",
    setup: albumPage,
    act: async (page) => {
      await page.locator('.actions button[aria-label="Exporter"]').first().click();
    },
    overlay: ".overlay",
  }),

  // O9 — FlashWarning, from Réglages › Animations › « Débridé ». A centred
  // confirm: `veil` + `dialog`, not a bottom sheet.
  scenario({
    name: "flash-warning",
    setup: (page) => go(page, "#/settings/viz", "button.seg-btn"),
    act: async (page) => {
      await page.locator("button.seg-btn", { hasText: "Débridé" }).first().click();
    },
    overlay: ".overlay",
    // A centred confirm: `dialog`, 200 ms on every viewport (not a bottom sheet).
    enter: { phone: 200, desktop: 200 },
  }),

  // O10 — GenreBulkSheet, from an album's genre button (admin). The component
  // is lazy: its chunk arrives after the click, and the mount-turn flag in the
  // component makes its first block change, so the intro plays on that mount.
  scenario({
    name: "album-genre-bulk",
    setup: albumPage,
    act: async (page) => {
      await page.locator('.actions button[aria-label="Donner un genre à l\'album"]').first().click();
    },
    overlay: ".overlay",
  }),

  // O11 — GenreTagSheet, from a track's menu.
  fromMenu("Étiqueter le genre…", "genre-tag"),

  // O12 — ReplaceSheet, from a track's menu. Its suggestion list enters with
  // `fadeIn` once (M7, one block, never a row intro), inside the sheet's own
  // enter — the sheet's animation is what this scenario measures.
  fromMenu("Remplacer le titre…", "replace"),

  // O14 — PlaylistPicker, from the row's « Ajouter à une playlist » button.
  // It had NO transition before this lot. The sheet is opened once in setup and
  // closed again: its covers are then known to the browser, so the measured
  // open does not race the lazy cover list (measured: 8 playlist covers in one
  // level's window, 16 in another).
  scenario({
    name: "playlist-picker",
    setup: async (page) => {
      await trackPage(page);
      await page.locator('.row.track button.add[aria-label="Ajouter à une playlist"]').first().click();
      await page.waitForSelector(".overlay .sheet", { timeout: 20000 });
      await quiet(page);
      await page.keyboard.press("Escape");
      await page.waitForTimeout(320);
      await quiet(page);
    },
    act: async (page) => {
      await page.locator('.row.track button.add[aria-label="Ajouter à une playlist"]').first().click();
    },
    overlay: ".overlay",
  }),

  // O15 — ShareSheet, from a track's menu. Its two tab rows enter with
  // `fadeIn`; the waveform canvas is drawn at mount, never per frame.
  fromMenu("Partager…", "share", ".backdrop"),
];
