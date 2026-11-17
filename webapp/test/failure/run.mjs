#!/usr/bin/env node
// A failure says why and offers Réessayer; it never reads as an empty screen or
// a verdict, the row menu works from the keyboard, and a row's controls are
// 44 px under a finger.
//
//   npm run build && node test/failure/run.mjs    (SHOTS=<dir> keeps pictures)
import { join } from "node:path";
import { loadPlaywright, startServer, login, checker } from "../harness.mjs";

const { chromium } = await loadPlaywright();
const { base, ids, stop } = await startServer(["--podcasts"]);
const { check, state } = checker();
const SHOTS = process.env.SHOTS || null;

const browser = await chromium.launch();

async function fresh({ width = 1440, height = 900, touch = false, setup = null, path = null } = {}) {
  const ctx = await browser.newContext(
    touch
      ? { viewport: { width, height }, deviceScaleFactor: 2, isMobile: true, hasTouch: true }
      : { viewport: { width, height } }
  );
  await ctx.route(/dzcdn\.net|api\.deezer\.com|scdn\.co/, (r) => r.abort());
  if (setup) await setup(ctx);
  const page = await ctx.newPage();
  const errors = [];
  page.on("pageerror", (e) => {
    errors.push(e.message);
    console.log("pageerror:", e.message);
  });
  await login(page, base);
  await page.reload();
  await page.waitForTimeout(1200);
  if (path) await page.goto(base + "/app/" + path);
  return { ctx, page, errors };
}

function endpoint(ctx, pattern, st = { broken: true, hits: 0 }) {
  ctx.route(pattern, async (r) => {
    st.hits++;
    if (!st.broken) return r.continue();
    await r.fulfill({
      status: 500,
      contentType: "application/json",
      body: JSON.stringify({ error: "boom" }),
    });
  });
  return st;
}

function refused(ctx, pattern, st = { broken: true, hits: 0 }) {
  ctx.route(pattern, async (r) => {
    st.hits++;
    if (!st.broken) return r.continue();
    await r.abort();
  });
  return st;
}

const body = (page) => page.evaluate(() => document.body.innerText || "");
const retryButton = (page) => page.locator('button:has-text("Réessayer")').first();

async function shot(page, name) {
  if (SHOTS) await page.screenshot({ path: join(SHOTS, name) });
}

async function failurePage({ name, path, pattern, wrong, cause, abort = false }) {
  const st = { broken: true, hits: 0 };
  const { ctx, page, errors } = await fresh({
    path,
    setup: (c) => (abort ? refused(c, pattern, st) : endpoint(c, pattern, st)),
  });
  await page.waitForSelector('button:has-text("Réessayer")', { timeout: 30000 });
  const text = await body(page);
  check(!text.includes(wrong), `${name}: no verdict on a failure ("${wrong}")`);
  check(text.includes("Réessayer"), `${name}: offers Réessayer`);
  if (cause) check(text.includes(cause), `${name}: names the cause`, cause);
  await shot(page, `${name}-500.png`);
  if (!abort) check(st.hits === 1, `${name}: the failed load is one request`, st.hits);
  check(errors.length === 0, `${name}: no page error`, errors.join(" | "));
  await ctx.close();
}

async function retryOnce({ name, path, pattern, settle }) {
  const st = { broken: true, hits: 0 };
  const { ctx, page } = await fresh({
    path,
    setup: (c) => endpoint(c, pattern, st),
  });
  await page.waitForSelector('button:has-text("Réessayer")', { timeout: 30000 });
  const before = st.hits;
  st.broken = false;
  await retryButton(page).click();
  if (settle) await page.waitForSelector(settle, { timeout: 30000 });
  else
    await page.waitForFunction(() => !document.body.innerText.includes("Le chargement a échoué"), null, {
      timeout: 30000,
    });
  check(st.hits === before + 1, `${name}: one click is one request`, `${before} -> ${st.hits}`);
  check(!(await body(page)).includes("Le chargement a échoué"), `${name}: the retry settles on the page`);
  await ctx.close();
}

async function list({ name, path, pattern, wrong }) {
  const { ctx, page, errors } = await fresh({ path, setup: (c) => endpoint(c, pattern) });
  await page.waitForSelector('button:has-text("Réessayer")', { timeout: 30000 });
  const text = await body(page);
  check(!text.includes(wrong), `${name}: a failure is not an empty list ("${wrong}")`);
  check(text.includes("Réessayer"), `${name}: offers Réessayer`);
  await shot(page, `${name}-500.png`);
  check(errors.length === 0, `${name}: no page error`, errors.join(" | "));
  await ctx.close();
}

await failurePage({
  name: "album",
  path: "#/album/" + ids.album,
  pattern: /\/api\/album\/[0-9a-f-]+$/,
  wrong: "Album introuvable",
  cause: "HTTP 500",
});
await retryOnce({
  name: "album",
  path: "#/album/" + ids.album,
  pattern: /\/api\/album\/[0-9a-f-]+$/,
  settle: "header.banner h1",
});
await failurePage({
  name: "artist",
  path: "#/artist/" + ids.artist,
  pattern: /\/api\/artist\/[0-9a-f-]+$/,
  wrong: "Artiste introuvable",
});
await retryOnce({
  name: "artist",
  path: "#/artist/" + ids.artist,
  pattern: /\/api\/artist\/[0-9a-f-]+$/,
  settle: "header.banner h1",
});
await failurePage({
  name: "playlist",
  path: "#/playlist/" + ids.small,
  pattern: /\/api\/playlist\/[0-9a-f-]+$/,
  wrong: "Playlist introuvable",
});
await retryOnce({
  name: "playlist",
  path: "#/playlist/" + ids.small,
  pattern: /\/api\/playlist\/[0-9a-f-]+$/,
  settle: "header.banner h1",
});
await failurePage({
  name: "album-offline",
  path: "#/album/" + ids.album,
  pattern: /\/api\/album\/[0-9a-f-]+$/,
  wrong: "Album introuvable",
  abort: true,
});

await list({
  name: "library-favorites",
  path: "#/library/favorites",
  pattern: /\/api\/me\/favorites$/,
  wrong: "Rien ici pour l'instant",
});
await (async () => {
  const st = { broken: true, hits: 0 };
  const { ctx, page } = await fresh({
    path: "#/library/favorites",
    setup: (c) => endpoint(c, /\/api\/me\/favorites$/, st),
  });
  await page.waitForSelector('button:has-text("Réessayer")', { timeout: 30000 });
  const counts = await page.evaluate(() => document.querySelector(".counts")?.textContent || "");
  check(!/\b0 titre/.test(counts), "library-favorites: the header count is not 0 titre", counts);
  st.broken = false;
  await retryButton(page).click();
  await page.waitForSelector(".row.track", { timeout: 30000 });
  check(st.hits === 2, "library-favorites: one retry is one request", st.hits);
  await ctx.close();
})();

await list({
  name: "library-playlists",
  path: "#/library/playlists",
  pattern: /\/api\/me\/playlists$/,
  wrong: "Aucune playlist",
});
await list({
  name: "library-artists",
  path: "#/library/artists",
  pattern: /\/api\/me\/artists$/,
  wrong: "Aucun artiste suivi",
});
await list({
  name: "library-albums",
  path: "#/library/albums",
  pattern: /\/api\/me\/albums$/,
  wrong: "Aucun album gardé",
});
await list({
  name: "library-local",
  path: "#/library/local",
  pattern: /\/api\/me\/local$/,
  wrong: "Aucun fichier à toi",
});
await list({
  name: "podcasts",
  path: "#/library/podcasts",
  pattern: /\/api\/podcasts$/,
  wrong: "Aucun abonnement",
});
await list({
  name: "show",
  path: "#/podcast/" + ids.podcast.show,
  pattern: /\/api\/podcast\/[0-9a-f-]+\?/,
  wrong: "Podcast introuvable",
});
await list({
  name: "settings-sources",
  path: "#/settings/sources",
  pattern: /\/api\/settings$/,
  wrong: "Aucun ARL configuré",
});

await (async () => {
  const st = { broken: true, hits: 0 };
  const { ctx, page } = await fresh({
    path: "#/search",
    setup: (c) =>
      c.route(/\/api\/search\?/, async (r) => {
        st.hits++;
        if (!st.broken) return r.continue();
        await r.fulfill({ status: 500, contentType: "application/json", body: '{"error":"boom"}' });
      }),
  });
  await page.fill('input[type="search"]', "storm");
  await page.press('input[type="search"]', "Enter");
  await page.waitForSelector('button:has-text("Réessayer")', { timeout: 30000 });
  const text = await body(page);
  check(!text.includes("Aucun résultat pour"), "search: a failure is not 'Aucun résultat'");
  check(text.includes("Réessayer"), "search: offers Réessayer");
  const before = st.hits;
  st.broken = false;
  await retryButton(page).click();
  await page.waitForFunction(() => !document.body.innerText.includes("Le chargement a échoué"), null, {
    timeout: 30000,
  });
  check(st.hits === before + 1, "search: one retry is one request", `${before} -> ${st.hits}`);
  await ctx.close();
})();

await (async () => {
  const { ctx, page, errors } = await fresh({ path: "#/library/favorites" });
  await page.waitForSelector(".row.track .more", { timeout: 30000 });
  await page.evaluate(() => {
    const b = document.querySelectorAll(".row.track .more")[0];
    b.dataset.kb = "1";
    b.focus();
  });
  await page.keyboard.press("Enter");
  await page.waitForSelector(".menu[role=menu]", { timeout: 5000 });
  check(
    await page.evaluate(() => {
      const a = document.activeElement;
      return a?.getAttribute("role") === "menuitem" && !!a.closest(".menu");
    }),
    "menu: Enter opens it with the focus on the first item"
  );
  const first = await page.evaluate(() => document.activeElement?.textContent?.trim() || "");
  await page.keyboard.press("ArrowDown");
  const second = await page.evaluate(() => document.activeElement?.textContent?.trim() || "");
  check(second && second !== first, "menu: ArrowDown moves to the next item", `${first} -> ${second}`);
  await page.keyboard.press("End");
  const last = await page.evaluate(() => document.activeElement?.textContent?.trim() || "");
  check(last && last !== second, "menu: End goes to the last item", last);
  await page.keyboard.press("Home");
  check(
    (await page.evaluate(() => document.activeElement?.textContent?.trim() || "")) === first,
    "menu: Home goes back to the first item"
  );
  await page.keyboard.press("Escape");
  // The pane closes AT ONCE and plays its outro (transitions.md O1: 140 ms,
  // inert), so the element leaves the DOM a moment later. Wait for it to
  // detach, then assert on what the person is left with.
  await page.locator(".menu[role=menu]").waitFor({ state: "detached", timeout: 3000 }).catch(() => {});
  check((await page.locator(".menu[role=menu]").count()) === 0, "menu: Escape closes it");
  check(
    await page.evaluate(() => document.activeElement?.dataset?.kb === "1"),
    "menu: Escape gives the focus back to the trigger"
  );
  await page.keyboard.press("Enter");
  await page.waitForSelector(".menu[role=menu]", { timeout: 5000 });
  await page.keyboard.press("Tab");
  await page.locator(".menu[role=menu]").waitFor({ state: "detached", timeout: 3000 }).catch(() => {});
  check((await page.locator(".menu[role=menu]").count()) === 0, "menu: Tab leaves the menu");
  check(
    await page.evaluate(() => !document.activeElement?.closest(".menu")),
    "menu: Tab does not leave the focus in a closed menu"
  );
  check(errors.length === 0, "menu: no page error", errors.join(" | "));
  await ctx.close();
})();

await (async () => {
  const { ctx, page } = await fresh({ path: "#/playlist/" + ids.small });
  await page.waitForSelector('button:has-text("Ajouter des titres")', { timeout: 30000 });
  await page.click('button:has-text("Ajouter des titres")');
  await page.waitForSelector("[role=dialog][aria-modal=true]", { timeout: 5000 });
  check(
    await page.evaluate(() => {
      const d = document.querySelector("[role=dialog][aria-modal=true]");
      return !!d && d.contains(document.activeElement);
    }),
    "sheet: the focus moves inside the dialog"
  );
  for (let i = 0; i < 12; i++) await page.keyboard.press("Tab");
  check(
    await page.evaluate(() => {
      const d = document.querySelector("[role=dialog][aria-modal=true]");
      return !!d && d.contains(document.activeElement);
    }),
    "sheet: Tab stays inside the dialog"
  );
  await page.keyboard.press("Escape");
  // Same shape as the menu: the dialog closes at once, its outro plays for
  // 140–200 ms and only then leaves the DOM (transitions.md O9/O23).
  await page
    .locator("[role=dialog][aria-modal=true]")
    .waitFor({ state: "detached", timeout: 3000 })
    .catch(() => {});
  check(
    (await page.locator("[role=dialog][aria-modal=true]").count()) === 0,
    "sheet: Escape closes it"
  );
  await ctx.close();
})();

const rowTargets = async (width, touch, min) => {
  const { ctx, page } = await fresh({ width, height: 844, touch, path: "#/library/favorites" });
  await page.waitForSelector(".row.track", { timeout: 30000 });
  const small = await page.evaluate((low) => {
    const out = [];
    for (const row of [...document.querySelectorAll(".row.track")].slice(0, 6)) {
      for (const b of row.querySelectorAll("button.play, button.add, button.fav, button.more")) {
        const r = b.getBoundingClientRect();
        if (!r.width || !r.height) continue;
        if (r.width < low - 0.5 || r.height < low - 0.5) {
          out.push(`${b.className || b.getAttribute("aria-label")}:${Math.round(r.width)}x${Math.round(r.height)}`);
        }
      }
    }
    return out;
  }, min);
  check(small.length === 0, `${width}${touch ? " touch" : ""}: every row control is ${min} px`, small.join(" "));
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth
  );
  check(overflow <= 1, `${width}: no sideways overflow on the favourites`, overflow);
  await shot(page, `rows-${width}.png`);
  await ctx.close();
};

await rowTargets(390, true, 44);
await rowTargets(641, false, 40);
await rowTargets(1440, false, 40);

await (async () => {
  const { ctx, page } = await fresh({ width: 390, height: 844, touch: true, path: "#/library/albums" });
  await page.waitForSelector(".card .play", { timeout: 30000 });
  const size = await page.evaluate(() => {
    const r = document.querySelector(".card .play").getBoundingClientRect();
    return [Math.round(r.width), Math.round(r.height)];
  });
  check(size[0] >= 44 && size[1] >= 44, "touch 390: the card's play is 44 px", size.join("x"));
  await ctx.close();
})();

const overflowAt = async (width, touch, path, name) => {
  const { ctx, page } = await fresh({ width, height: 900, touch, path });
  await page.waitForTimeout(700);
  const d = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth
  );
  check(d <= 1, `${width}: no sideways overflow on ${name}`, d);
  await ctx.close();
};

for (const [w, touch] of [
  [390, true],
  [641, false],
  [1440, false],
]) {
  await overflowAt(w, touch, "#/library/favorites", "the favourites");
  await overflowAt(w, touch, "#/library/artists", "the artists");
  await overflowAt(w, touch, "#/album/" + ids.album, "an album");
  await overflowAt(w, touch, "#/artist/" + ids.artist, "an artist");
  await overflowAt(w, touch, "#/playlist/" + ids.small, "a playlist");
  await overflowAt(w, touch, "#/library/podcasts", "the podcasts");
  await overflowAt(w, touch, "#/settings/sources", "the sources");
}

await browser.close();
stop();

console.log(state.failed ? `\n${state.failed} check(s) failed` : "\nall failure checks passed");
process.exit(state.failed ? 1 : 0);
