#!/usr/bin/env node
// The contextual menu, on EVERY surface that manipulates a title, an album, a
// playlist, an artist or a show — and nowhere it does not belong.
//
// What this holds, at 1280×900 and 390×844, on the built SPA and a real server:
//   · a right-click (desktop) / the WebView's long-press event (phone) opens the
//     menu on each wired surface, with the items that surface is entitled to;
//   · Shift+F10, the keyboard's Menu key and the focus give the menu back;
//   · opening a menu costs ZERO /api request (measured, not assumed);
//   · the queue's own row does not offer "queue this again" and removes only
//     what the queue itself allows; the "Lire ensuite" action still works.
//
//   npm run build && node test/menu/run.mjs            (SHOTS=<dir> keeps pictures)
//
// Runs tools/perf_api.py --serve (8 000 tracks, Deezer off) with a show, a
// listening history and the two search answers the bench needs (Deezer is off,
// so the catalogue search has nothing to say; the shows are stubbed like
// test/explore/run.mjs does with Deezer's own answers).
import { join } from "node:path";
import { loadPlaywright, startServer, login, silence, checker } from "../harness.mjs";

const { chromium } = await loadPlaywright();
const { base, ids, stop } = await startServer(["--podcasts", "--recap"]);
const WAV = silence();
const SHOTS = process.env.SHOTS || null;
const { check, state } = checker();

const MENU = ".menu[role=menu]";
const ITEMS = `${MENU} [role=menuitem]`;
const STUB_ARTIST = "Amour 123";
const STUB_SHOW = "Les Histoires du banc";

// What a track's menu must carry (the same list as the row's "⋯"): the tests
// name the ones a surface must not lose, not every last entry.
const TRACK = [
  "Lire ensuite",
  "Ajouter à la file",
  "Ajouter à une playlist…",
  "Remplacer le titre…",
  "Télécharger",
  "Télécharger en…",
  "Partager…",
  "Lancer la radio",
  "Voir les crédits",
  "Aller à l'artiste",
  "Aller à l'album",
];
const ENTITY_ALBUM = ["Lire", "Ouvrir", "Télécharger"];
const ENTITY_PLAYLIST = ["Lire", "Ouvrir", "Télécharger"];
const ENTITY_ARTIST = ["Lire", "Ouvrir", "Suivre"];
const EPISODE = ["Lire ensuite", "Ajouter à la file", "Partager…", "Télécharger", "Ouvrir le podcast"];
const PODCAST = ["S'abonner et ouvrir", "S'abonner"];

const browser = await chromium.launch();

async function runView(width, height, phone) {
  const tag = `${width}×${height}`;
  const ctx = await browser.newContext(
    phone
      ? { viewport: { width, height }, deviceScaleFactor: 2, isMobile: true, hasTouch: true }
      : { viewport: { width, height } }
  );
  await ctx.route(/dzcdn\.net|api\.deezer\.com|scdn\.co/, (r) => r.abort());
  // The synthetic library has no audio files: without this the player fails the
  // first load and skips to the next title mid-test, moving the queue's
  // playhead under the rows. The same one-minute silence test/queue/run.mjs uses.
  await ctx.route(/\/api\/stream\//, (r) =>
    r.fulfill({ status: 200, body: WAV, headers: { "Content-Type": "audio/wav", "Accept-Ranges": "none" } })
  );
  // The two answers Deezer would give: the bench asks the menu and the layout,
  // not the provider. (A show card only exists as a search result.)
  await ctx.route(/\/api\/search\/podcasts/, (r) =>
    r.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        podcasts: [{ deezer_id: "9400001", title: STUB_SHOW, cover: null, description: "" }],
      }),
    })
  );
  await ctx.route(/\/api\/search\?q=/, (r) =>
    r.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        artists: [{ id: "1000000", deezer_id: "1000000", name: STUB_ARTIST, picture: null }],
        albums: [],
        tracks: [],
        playlists: [],
        podcasts: [],
      }),
    })
  );

  const page = await ctx.newPage();
  const errors = [];
  page.on("pageerror", (e) => {
    errors.push(e.message);
    console.log(`pageerror ${tag}:`, e.message);
  });

  // Every /api request the app makes that is not audio or artwork. A menu opens
  // from stores already in memory: the delta around an open must be exactly 0.
  const api = [];
  page.on("request", (r) => {
    const u = r.url();
    if (!u.startsWith(base) || !/\/api\//.test(u)) return;
    if (/\/api\/(stream|cover)\//.test(u)) return;
    // The player prefetches the next title (rule 1b, Player.svelte): it can
    // land in any window and is not the menu. A bulk download stays counted.
    if (/\/api\/download$/.test(u) && (r.postData() || "").includes("prefetch")) return;
    api.push(u.slice(base.length));
  });
  const quiet = async () => {
    let last = api.length;
    for (let i = 0; i < 16; i++) {
      await page.waitForTimeout(250);
      if (api.length === last) return;
      last = api.length;
    }
  };

  await login(page, base);
  await page.reload();
  await page.waitForTimeout(1400);

  // The app warms its playlist cache ten seconds after boot, then one request
  // every 1.5 s for twenty-five playlists (lib/actions.js#warmPlaylists). That
  // is the app preparing — never a menu — but it would land inside any 300 ms
  // window and make the request count a lie. The cache is filled here exactly
  // as the warmer would leave it, so the background pass skips every path and
  // opening a menu is measured against a silent page.
  await page.evaluate(async () => {
    const me = await fetch("/api/me").then((r) => r.json());
    const uid = me?.user?.id;
    if (uid == null) return;
    const owner = location.origin + "/id:" + String(uid);
    const lists = await fetch("/api/me/playlists").then((r) => r.json());
    const paths = [
      "/me/favorites",
      ...(lists.playlists || []).slice(0, 25).map((p) => "/playlist/" + p.id),
    ];
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
  await page.waitForTimeout(300);

  const goto = async (path, wait) => {
    await page.goto(base + "/app/" + path);
    if (wait) await page.waitForSelector(wait, { timeout: 40000 });
    await page.waitForTimeout(650);
    await quiet();
  };

  const texts = () => page.$$eval(ITEMS, (els) => els.map((e) => e.textContent.trim()));
  const active = () =>
    page.evaluate(() => {
      const a = document.activeElement;
      return { tag: a?.tagName || "", cls: String(a?.className || "") };
    });

  // Open the menu the way a person does: a real right-click on the desktop, the
  // contextmenu event a WebView emits after a long press on the phone.
  async function open(selector, index = 0) {
    const loc = page.locator(selector).nth(index);
    // Centre it: a sticky top bar would otherwise swallow the click on a row
    // that scrollIntoViewIfNeeded parked under it.
    await loc.evaluate((el) => el.scrollIntoView({ block: "center", inline: "nearest" })).catch(() => {});
    await page.waitForTimeout(80);
    if (phone) {
      await page.evaluate(
        ({ selector, index }) => {
          const el = document.querySelectorAll(selector)[index];
          if (!el) throw new Error("missing " + selector + " #" + index);
          const r = el.getBoundingClientRect();
          el.dispatchEvent(
            new MouseEvent("contextmenu", {
              bubbles: true,
              cancelable: true,
              clientX: Math.round(r.left + Math.min(24, r.width / 2)),
              clientY: Math.round(r.top + Math.min(20, r.height / 2)),
            })
          );
        },
        { selector, index }
      );
    } else {
      const box = await loc.boundingBox();
      if (!box) throw new Error("no box for " + selector);
      await page.mouse.click(box.x + Math.min(24, box.width / 2), box.y + Math.min(20, box.height / 2), {
        button: "right",
      });
    }
    await page.waitForSelector(MENU, { timeout: 5000 });
    await page.waitForTimeout(180);
  }

  // Open, judge the items, check the API stayed silent, close with Escape and
  // check the focus went back to the trigger.
  async function surface({ name, selector, index = 0, want = [], not = [], focus = null, aria = null }) {
    await quiet();
    const before = api.length;
    try {
      await open(selector, index);
    } catch (e) {
      check(false, `${tag} ${name}: the menu opens`, e.message);
      return;
    }
    const items = await texts();
    const missing = want.filter((w) => !items.some((i) => i === w || i.startsWith(w)));
    const wrong = not.filter((w) => items.some((i) => i === w || i.startsWith(w)));
    check(missing.length === 0, `${tag} ${name}: carries its actions`, missing.join(" | "));
    check(wrong.length === 0, `${tag} ${name}: drops what is absurd there`, wrong.join(" | "));
    if (aria) {
      const has = await page.evaluate((s) => {
        const el = document.querySelector(s);
        return !!el && (el.hasAttribute("aria-haspopup") || !!el.querySelector('[aria-haspopup="menu"]'));
      }, aria);
      check(has, `${tag} ${name}: says it opens a menu (aria-haspopup)`);
    }
    await page.waitForTimeout(300);
    const extra = api.slice(before);
    check(extra.length === 0, `${tag} ${name}: opening it costs no request`, extra.join(" "));
    if (SHOTS) await page.screenshot({ path: join(SHOTS, `menu-${tag}-${name.replace(/\W+/g, "-")}.png`) });

    await page.keyboard.press("Escape");
    await page.waitForTimeout(180);
    check((await page.locator(MENU).count()) === 0, `${tag} ${name}: Escape closes it`);
    const back = await page.evaluate((s) => {
      const el = document.querySelector(s);
      const a = document.activeElement;
      return !!el && (el === a || el.contains(a));
    }, focus || selector);
    const where = await active();
    check(back, `${tag} ${name}: Escape gives the focus back`, `${where.tag}.${where.cls}`);
  }

  async function keyboard({ name, selector, index = 0, first, focus = null }) {
    await quiet();
    const el = page.locator(selector).nth(index);
    await el.scrollIntoViewIfNeeded().catch(() => {});
    await page.evaluate(({ s, i }) => document.querySelectorAll(s)[i]?.focus(), { s: selector, i: index });
    await page.keyboard.press("Shift+F10");
    let opened = true;
    try {
      await page.waitForSelector(MENU, { timeout: 3000 });
    } catch {
      opened = false;
    }
    check(opened, `${tag} ${name}: Shift+F10 opens it`);
    if (!opened) return;
    const items = await texts();
    check(items[0] === first, `${tag} ${name}: the first item is the expected one`, `${items[0]} / ${first}`);
    await page.keyboard.press("Escape");
    await page.waitForTimeout(180);
    const back = await page.evaluate((s) => {
      const el = document.querySelector(s);
      const a = document.activeElement;
      return !!el && (el === a || el.contains(a));
    }, focus || selector);
    check(back, `${tag} ${name}: keyboard Escape gives the focus back`);
  }

  // -- the pages ------------------------------------------------------------

  // 1. A title row and the artist credited on it (two menus, one line).
  await goto("#/album/" + ids.album, ".row.track");
  await surface({
    name: "titre (rangée)",
    selector: ".row.track",
    want: TRACK,
    not: ["Retirer de la file"],
    aria: ".row.track",
  });
  await surface({
    name: "artiste (ligne de crédit)",
    selector: ".row.track .a .one",
    want: ENTITY_ARTIST,
    not: ["Lire ensuite"],
    aria: ".row.track .a .one",
  });
  await keyboard({ name: "titre (rangée)", selector: ".row.track", first: "Lire ensuite" });
  await keyboard({
    name: "artiste (ligne de crédit)",
    selector: ".row.track .a .one",
    first: "Lire",
  });
  // The other key the ARIA practices name: the Menu key (Chromium reports it
  // as "ContextMenu"). Dispatched on the focused row, as the OS would.
  {
    await page.evaluate(() => {
      const el = document.querySelector(".row.track");
      el.focus();
      el.dispatchEvent(new KeyboardEvent("keydown", { key: "ContextMenu", bubbles: true, cancelable: true }));
    });
    let ok = true;
    try {
      await page.waitForSelector(MENU, { timeout: 3000 });
    } catch {
      ok = false;
    }
    check(ok, `${tag} titre (rangée): the Menu key opens it`);
    if (ok) {
      await page.keyboard.press("Escape");
      await page.waitForTimeout(150);
    }
  }

  // 2. The card grids: album/playlist cards and artist tiles.
  await goto("#/library/playlists", ".card");
  // Card.svelte is the one surface this lot does not own (another agent is on
  // it): the menu is there, its aria-haspopup is not — noted in the report.
  await surface({ name: "carte playlist", selector: ".card", want: ENTITY_PLAYLIST });
  await goto("#/library/artists", ".tile");
  await surface({ name: "tuile artiste", selector: ".tile", want: ENTITY_ARTIST, aria: ".tile" });
  await keyboard({ name: "tuile artiste", selector: ".tile", first: "Lire" });

  // 3. The search's best result (an artist; the show top result uses the same
  //    builder as a show card).
  await goto("#/search/" + encodeURIComponent(STUB_ARTIST), ".top");
  await surface({ name: "meilleur résultat", selector: ".top", want: ENTITY_ARTIST, aria: ".top" });

  // 4. A show card from the results (Deezer's answer stubbed) and its episodes.
  await goto("#/podcasts", ".search input");
  await page.locator(".search input").first().fill("histoires");
  await page.waitForSelector(".grid:not(.hidden) .card[role=button]", { timeout: 8000 });
  await page.waitForTimeout(400);
  await surface({
    name: "carte podcast",
    selector: ".grid:not(.hidden) .card[role=button]",
    want: PODCAST,
    aria: ".grid:not(.hidden) .card[role=button]",
  });
  await keyboard({
    name: "carte podcast",
    selector: ".grid:not(.hidden) .card[role=button]",
    first: "S'abonner et ouvrir",
  });

  await goto("#/podcast/" + ids.podcast.show, ".episodes li");
  await surface({
    name: "épisode",
    selector: ".episodes li",
    want: EPISODE,
    not: ["Aller à l'album"],
    // The row is a listitem (aria-haspopup is not valid there); its own "⋯"
    // button carries the contract, and is where a keyboard reaches it.
    aria: ".episodes li .emenu",
  });
  await keyboard({ name: "épisode", selector: ".episodes li", first: "Lire ensuite" });

  // 5. A release card's preview rows (a title each) and the full list behind.
  await goto("#/releases", ".rc-row");
  await surface({ name: "carte de sortie", selector: ".rc-row", want: TRACK, not: ["Retirer de la file"] });

  // 6. History: one event row and the summary's top titles. On a phone the
  //    windowed list starts below the summary, so it mounts its rows once the
  //    page is scrolled to it.
  await goto("#/library/history", phone ? ".vlist" : ".ev");
  if (phone) {
    await page.locator(".vlist").scrollIntoViewIfNeeded();
    await page.waitForSelector(".ev", { timeout: 12000 });
    await page.waitForTimeout(400);
  }
  await surface({ name: "historique (événement)", selector: ".ev", want: TRACK, aria: ".ev" });
  // A right-click inside a text the person had selected is a Copy request, not
  // ours: no menu, and the selection is still there.
  {
    await quiet();
    const before = await page.evaluate(() => {
      const title = document.querySelector(".ev-title");
      if (!title) return null;
      const range = document.createRange();
      range.selectNodeContents(title);
      const sel = window.getSelection();
      sel.removeAllRanges();
      sel.addRange(range);
      return sel.toString();
    });
    if (before) {
      await page.evaluate(() => {
        const el = document.querySelector(".ev-title");
        const r = el.getBoundingClientRect();
        el.dispatchEvent(
          new MouseEvent("contextmenu", {
            bubbles: true,
            cancelable: true,
            clientX: r.left + r.width / 2,
            clientY: r.top + r.height / 2,
          })
        );
      });
      await page.waitForTimeout(250);
      check(
        (await page.locator(MENU).count()) === 0,
        `${tag} historique: a selected text keeps the browser's own menu`
      );
      const after = await page.evaluate(() => window.getSelection().toString());
      check(after === before, `${tag} historique: the selection is not taken away`);
      await page.evaluate(() => window.getSelection().removeAllRanges());
    }
  }
  await surface({ name: "historique (top titre)", selector: ".top-track", want: TRACK, aria: ".top-track" });
  await keyboard({ name: "historique (événement)", selector: ".ev", first: "Lire ensuite" });

  // 7. Réglages → Découverte: a disliked title (the seed is idempotent).
  await page.evaluate(async (tid) => {
    await fetch("/api/dislike", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ id: tid, on: true }),
    });
  }, ids.track);
  await goto("#/settings/discovery", "section.card .list li");
  await surface({
    name: "titre « je n'aime pas »",
    selector: "section.card .list li",
    want: TRACK,
    not: ["Retirer de la file"],
  });

  // 8. The editable playlist list (PlaylistTracks, drag and drop included).
  await goto("#/playlist/" + ids.small, "button:has-text('Modifier')");
  await page.locator("button:has-text('Modifier')").first().click();
  await page.waitForSelector(".list > .row", { timeout: 8000 });
  await page.waitForTimeout(400);
  await surface({
    name: "playlist éditable",
    selector: ".list > .row",
    want: TRACK,
    aria: ".list > .row",
  });

  // 9. The queue: play a finite playlist, pause nothing (the queue is a store),
  //    then open the three panels that share QueueList. Leave the edit mode the
  //    previous surface opened first: its rows have no play button.
  {
    const done = page.locator("button:has-text('Terminé')").first();
    if (await done.count()) {
      await done.click();
      await page.waitForTimeout(350);
    }
  }
  await goto("#/playlist/" + ids.small, "button.play[aria-label='Lire']");
  await page.locator("button.play[aria-label='Lire']").first().click();
  await page.waitForTimeout(900);
  if (phone) {
    await page.locator("footer.player .now").first().click();
    await page.waitForTimeout(500);
    await page.locator('.m button[aria-label="File d\'attente"]').first().click();
  } else {
    await page.locator('button[aria-label="File d\'attente"]').first().click();
  }
  await page.waitForSelector(".qitem", { timeout: 8000 });
  await page.waitForTimeout(500);
  await quiet();
  const queueRow = ".qitem[data-qi='1']";
  await surface({
    name: "file d'attente (à venir)",
    selector: queueRow,
    want: ["Retirer de la file", "Ajouter à une playlist…", "Voir les crédits"],
    not: ["Lire ensuite", "Ajouter à la file"],
    aria: queueRow,
  });
  await surface({
    name: "file d'attente (titre en cours)",
    selector: ".qitem.now",
    want: ["Ajouter à une playlist…", "Lancer la radio"],
    not: ["Retirer de la file", "Lire ensuite", "Ajouter à la file"],
    aria: ".qitem.now",
  });
  await keyboard({ name: "file d'attente", selector: queueRow, first: "Retirer de la file" });

  // A drag in progress owns the row: a contextmenu that arrives mid-gesture
  // (a long press that turned into a drag) must not open a menu over it.
  {
    await quiet();
    const grip = page.locator(".qitem .grip").first();
    await grip.scrollIntoViewIfNeeded().catch(() => {});
    const gb = await grip.boundingBox();
    if (gb) {
      await page.mouse.move(gb.x + gb.width / 2, gb.y + gb.height / 2);
      await page.mouse.down();
      await page.evaluate(() => {
        const el = document.querySelector(".qitem");
        if (!el) return;
        const r = el.getBoundingClientRect();
        el.dispatchEvent(
          new MouseEvent("contextmenu", {
            bubbles: true,
            cancelable: true,
            clientX: r.left + 30,
            clientY: r.top + 20,
          })
        );
      });
      await page.waitForTimeout(250);
      check((await page.locator(MENU).count()) === 0, `${tag} file d'attente: a drag does not open a menu`);
      await page.mouse.up();
      await page.waitForTimeout(250);
    }
  }

  // The queue's own action: remove the upcoming row that was just inspected.
  {
    const titles = await page.$$eval(".qitem .qt", (els) => els.map((e) => e.textContent.trim()));
    const victim = titles[1];
    await open(queueRow);
    await page.waitForTimeout(250);
    const remove = page.locator(`${MENU} [role=menuitem]`, { hasText: "Retirer de la file" }).first();
    if ((await remove.count()) === 0) console.log("menu at action:", await texts());
    await remove.click({ timeout: 5000 });
    await page.waitForTimeout(400);
    const after = await page.$$eval(".qitem .qt", (els) => els.map((e) => e.textContent.trim()));
    check(!after.includes(victim), `${tag} file d'attente: « Retirer de la file » removes that title`, victim);
  }

  // A phone's full-screen player is a global overlay: close the sheet and the
  // player before asking the home for its own rows.
  if (phone) {
    const closeQueue = page.locator('button[aria-label="Fermer la file d\'attente"]');
    if (await closeQueue.count()) await closeQueue.first().click();
    const reduce = page.locator('button[aria-label="Réduire"]');
    if (await reduce.count()) await reduce.first().click();
    await page.waitForTimeout(350);
  }

  // 10. The home: a followed artist and a recent album (the listen just made).
  await goto("#/", ".artists a.artist");
  await surface({ name: "accueil (artiste)", selector: ".artists a.artist", want: ENTITY_ARTIST, aria: ".artists a.artist" });
  await page.waitForSelector(".shortcut", { timeout: 8000 });
  await surface({ name: "accueil (album récent)", selector: ".shortcut", want: ENTITY_ALBUM, aria: ".shortcut" });

  // The row's own action, last: it starts playback (and its own requests).
  await goto("#/album/" + ids.album, ".row.track");
  await open(".row.track");
  await page.locator(`${MENU} [role=menuitem]`, { hasText: "Lire ensuite" }).first().click();
  let toasted = true;
  try {
    await page.waitForFunction(() => document.body.innerText.includes("Sera lu ensuite"), null, {
      timeout: 3000,
    });
  } catch {
    toasted = false;
  }
  check(toasted, `${tag} titre: « Lire ensuite » answers`);

  check(errors.length === 0, `${tag}: no page error`, errors.join(" | "));
  await ctx.close();
}

await runView(1280, 900, false);
await runView(390, 844, true);

await browser.close();
stop();

console.log(state.failed ? `\n${state.failed} check(s) failed` : "\nall menu checks passed");
process.exit(state.failed ? 1 : 0);
