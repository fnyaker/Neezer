#!/usr/bin/env node
import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { startServer, checker } from "../harness.mjs";

async function loadPlaywright() {
  const specs = [
    process.env.PLAYWRIGHT,
    "playwright",
    "playwright-core",
    "/tmp/opencode/r1/node_modules/playwright-core/index.mjs",
  ].filter(Boolean);
  let last;
  for (const spec of specs) {
    try {
      return await import(spec);
    } catch (e) {
      last = e;
    }
  }
  throw last;
}

const { chromium } = await loadPlaywright();
const REPO = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const DIST = process.env.DIST || join(REPO, "neezer/webui/dist");
const executablePath =
  process.env.CHROMIUM || (existsSync("/usr/bin/chromium") ? "/usr/bin/chromium" : undefined);
const { check, state } = checker();
const note = (what, value) => console.log(`note ${what}: ${value}`);

const server = await startServer([
  "--tracks",
  "800",
  "--playlists",
  "3",
  "--favorites",
  "50",
  "--big-playlist",
  "100",
  "--runs",
  "1",
  "--onboarding",
  "--dist",
  DIST,
]);
const seed = server.ids && server.ids.onboarding;
if (!seed || !seed.tokens || seed.tokens.length < 2) {
  console.log("skip: tools/perf_api.py --onboarding did not seed invitation tokens");
  server.stop();
  process.exit(0);
}

const browser = await chromium.launch({
  executablePath,
  args: ["--no-sandbox", "--autoplay-policy=no-user-gesture-required"],
});

function track(page) {
  const st = {
    api: [],
    inspect: 0,
    redeem: 0,
    onbGet: 0,
    artistsGet: 0,
    searchGet: 0,
    likePost: 0,
    relatedGet: 0,
    donePost: 0,
    accountDonePost: 0,
    retryPost: 0,
  };
  page.on("request", (r) => {
    const url = new URL(r.url());
    if (!url.pathname.startsWith("/api/")) return;
    const method = r.method();
    const path = url.pathname;
    st.api.push(method + " " + path + (url.search || ""));
    if (path === "/api/invites/inspect") st.inspect += 1;
    if (path === "/api/invites/redeem" && method === "POST") st.redeem += 1;
    if (path === "/api/onboarding" && method === "GET") st.onbGet += 1;
    if (path === "/api/onboarding/artists" && method === "GET") st.artistsGet += 1;
    if (path === "/api/onboarding/search" && method === "GET") st.searchGet += 1;
    if (path === "/api/onboarding/artists/like" && method === "POST") st.likePost += 1;
    if (/\/api\/onboarding\/artists\/[^/]+\/related$/.test(path) && method === "GET") st.relatedGet += 1;
    if (path === "/api/onboarding/artists/done" && method === "POST") st.donePost += 1;
    if (path === "/api/onboarding/account/done" && method === "POST") st.accountDonePost += 1;
    if (path === "/api/onboarding/feed/retry" && method === "POST") st.retryPost += 1;
  });
  return st;
}

const overflow = (page) =>
  page.evaluate(() => ({
    doc: document.documentElement.scrollWidth,
    width: document.documentElement.clientWidth,
  }));

async function heightOf(page, selector) {
  return page.locator(selector).first().evaluate((el) => el.getBoundingClientRect().height);
}

async function runFlow({ width, height, phone, token, name }) {
  const tag = `${width}x${height}`;
  const ctx = await browser.newContext({
    viewport: { width, height },
    deviceScaleFactor: phone ? 2 : 1,
    isMobile: phone,
    hasTouch: phone,
  });
  await ctx.route(/dzcdn\.net|api\.deezer\.com|lrclib/, (r) => r.abort());
  const page = await ctx.newPage();
  const st = track(page);
  page.on("pageerror", (e) => console.log("pageerror:", e.message));

  await page.goto(server.base + "/app/#/invite/" + encodeURIComponent(token));
  await page.waitForSelector("#invite-name", { timeout: 30000 });
  await page.waitForTimeout(300);
  check(
    st.inspect === 1 && st.api.length <= 2,
    `${tag}: the invitation page asks 1 inspect and no more than 2 API calls (measured ${st.api.length}: ${st.api.join(", ")})`
  );
  const ov = await overflow(page);
  check(ov.doc <= ov.width + 1, `${tag}: the invitation does not overflow (${ov.doc}/${ov.width})`);
  const card = await page.locator("main.card").boundingBox();
  check(card && card.width <= 441, `${tag}: the invitation card is at most 440 px (${card && card.width.toFixed(1)})`);
  check((await heightOf(page, ".submit")) >= 47.5, `${tag}: the invite submit is 48 px`);
  check((await heightOf(page, ".reveal")) >= 43.5, `${tag}: the password reveal is 44 px`);
  check((await heightOf(page, "#invite-name")) >= 47.5, `${tag}: the name field is 48 px`);

  await page.locator("#invite-name").fill(name);
  await page.locator("#invite-password").fill("court");
  await page.locator("#invite-confirm").fill("court");
  await page.locator(".submit").click();
  await page.waitForTimeout(400);
  check(st.redeem === 0, `${tag}: an invalid form sends no POST (measured ${st.redeem})`);
  check(
    (await page.locator("#invite-pass-err").innerText()).includes("12 caractères minimum"),
    `${tag}: the short password says its cause`
  );

  await page.locator("#invite-password").fill(seed.password);
  await page.locator("#invite-confirm").fill(seed.password);
  await page.locator(".submit").click();
  await page.waitForFunction(() => location.hash === "#/onboarding/artists", null, { timeout: 30000 });
  await page.waitForSelector("button.tile", { timeout: 30000 });
  await page.waitForTimeout(400);
  check(st.redeem === 1, `${tag}: the valid form sends exactly 1 redeem POST (measured ${st.redeem})`);
  check(
    st.onbGet === 1 && st.artistsGet === 1,
    `${tag}: onboarding artists opens on 2 API calls (measured ${st.onbGet}+${st.artistsGet})`
  );
  check(
    (await page.locator(".finish").getAttribute("aria-disabled")) === "true",
    `${tag}: Terminé is disabled before a like`
  );
  check((await heightOf(page, ".finish")) >= 43.5, `${tag}: Terminé is 44 px`);
  check((await heightOf(page, "#ob-search")) >= 43.5, `${tag}: the artist search is 44 px`);

  const tiles = await page.locator("button.tile").count();
  check(tiles >= 36, `${tag}: the first grid page holds at least 36 artists (measured ${tiles})`);
  const cols = await page.evaluate(
    () => getComputedStyle(document.querySelector(".grid")).gridTemplateColumns.split(/\s+/).filter(Boolean).length
  );
  check(cols === (width >= 1280 ? 6 : 2), `${tag}: the grid has ${width >= 1280 ? 6 : 2} columns (measured ${cols})`);
  const ov2 = await overflow(page);
  check(ov2.doc <= ov2.width + 1, `${tag}: the grid does not overflow (${ov2.doc}/${ov2.width})`);

  const beforeSearch = st.api.length;
  await page.locator("#ob-search").pressSequentially("camille", { delay: 40 });
  await page.waitForFunction(
    () => [...document.querySelectorAll("button.tile .name")].some((el) => el.textContent.trim() === "Camille Fictive"),
    null,
    { timeout: 15000 }
  );
  await page.waitForTimeout(500);
  const searched = st.api.slice(beforeSearch).filter((entry) => entry.includes("/api/onboarding/search"));
  check(searched.length === 1, `${tag}: typing a search asks 1 GET, not one per letter (measured ${searched.length})`);
  check((await page.locator("button.tile").count()) === 1, `${tag}: the search shows the one matching artist`);
  check((await heightOf(page, ".clear")) >= 43.5, `${tag}: the clear button is 44 px`);
  const beforeClear = st.api.length;
  await page.locator(".clear").click();
  await page.waitForTimeout(500);
  const cleared = st.api.slice(beforeClear);
  const clearedSearches = cleared.filter((entry) => entry.includes("/api/onboarding/search")).length;
  check(
    clearedSearches === 0 && cleared.length <= 1,
    `${tag}: clearing the search asks no search, at most the next page (measured ${cleared.length}: ${cleared.join(", ")})`
  );

  const beforeLike = {
    like: st.likePost,
    related: st.relatedGet,
    api: st.api.length,
  };
  await page.locator("button.tile").first().click();
  await page.waitForFunction(
    () => document.querySelector(".progressrow .count")?.textContent.trim() === "1 artiste",
    null,
    { timeout: 15000 }
  );
  await page.waitForFunction(
    () => document.querySelectorAll("button.tile .near").length === 2,
    null,
    { timeout: 15000 }
  );
  check(st.likePost - beforeLike.like === 1, `${tag}: a like sends 1 POST (measured ${st.likePost - beforeLike.like})`);
  check(st.relatedGet - beforeLike.related === 0, `${tag}: the 2 neighbours come in the same answer (measured ${st.relatedGet - beforeLike.related} GET)`);
  const near = await page.evaluate(() => {
    const all = [...document.querySelectorAll("button.tile")];
    return all
      .map((el, index) => (el.querySelector(".near") ? index : -1))
      .filter((index) => index >= 0);
  });
  check(
    near.length === 2 && near[1] === near[0] + 1,
    `${tag}: the 2 neighbours sit together right after the liked tile (measured ${JSON.stringify(near)})`
  );
  check(
    (await page.locator(".finish").getAttribute("aria-disabled")) === "false",
    `${tag}: Terminé unlocks after a like`
  );
  const likedName = (await page.locator('button.tile[aria-pressed="true"] .name').innerText()).trim();

  const beforeReload = { done: st.donePost, like: st.likePost };
  await page.reload({ waitUntil: "domcontentloaded" });
  await page.waitForSelector("button.tile", { timeout: 30000 });
  await page.waitForFunction(
    () => document.querySelector(".progressrow .count")?.textContent.trim() === "1 artiste",
    null,
    { timeout: 30000 }
  );
  await page.waitForTimeout(800);
  check((await page.evaluate(() => location.hash)) === "#/onboarding/artists", `${tag}: a refresh stays on the artists step`);
  check(st.donePost === beforeReload.done, `${tag}: the refresh sends no step POST`);
  check(st.likePost === beforeReload.like, `${tag}: the refresh sends no like POST`);
  check(
    (await page.locator("button.tile .near").count()) === 2,
    `${tag}: the 2 neighbours are not rebuilt or duplicated by the refresh (measured ${await page.locator("button.tile .near").count()})`
  );
  const pressed = await page.locator('button.tile[aria-pressed="true"]').count();
  const stillLiked = await page
    .locator(`button.tile[aria-label="Retirer ${likedName} à mes artistes"]`)
    .count();
  check(
    pressed === 1 && stillLiked === 1,
    `${tag}: the liked tile is still marked liked after the refresh (measured ${pressed} pressed, ${stillLiked} for ${likedName})`
  );
  const ov3 = await overflow(page);
  check(ov3.doc <= ov3.width + 1, `${tag}: the refreshed artists page does not overflow`);

  const beforeFinish = {
    done: st.donePost,
    api: st.api.length,
  };
  await page.locator(".finish").click();
  await page.waitForFunction(() => location.hash === "#/onboarding/accounts", null, { timeout: 30000 });
  await page.waitForSelector(".account .card", { timeout: 30000 });
  await page.waitForTimeout(400);
  check(st.donePost - beforeFinish.done === 1, `${tag}: finishing the artists step sends 1 POST (measured ${st.donePost - beforeFinish.done})`);
  const accountCalls = st.api.slice(beforeFinish.api);
  const accountPosts = accountCalls.filter((entry) => entry.startsWith("POST /api/onboarding/account/")).length;
  const accountGets = accountCalls.filter((entry) => entry.startsWith("GET ")).length;
  check(accountPosts === 0, `${tag}: the account step opens with 0 POST (measured ${accountPosts})`);
  check(accountGets === 0, `${tag}: the account step opens without a request, the payload is kept (measured ${accountGets} GET)`);
  note(`${tag}: account step request(s)`, JSON.stringify(accountCalls));

  const cards = await page.locator(".account .cards > .card").count();
  check(cards === 3, `${tag}: the account step shows 3 optional cards (measured ${cards})`);
  const skips = await page.locator(".account .card .ghost.small").count();
  check(skips === 3, `${tag}: each card can be skipped (measured ${skips})`);
  const skipHeights = await page.locator(".account .card .ghost.small").evaluateAll((els) =>
    els.map((el) => el.getBoundingClientRect().height)
  );
  check(skipHeights.every((value) => value >= 43.5), `${tag}: the skip controls are 44 px (measured ${JSON.stringify(skipHeights.map((v) => Math.round(v)))})`);
  check((await heightOf(page, ".account .glyph")) >= 43.5, `${tag}: the option glyph is 44 px`);

  const beforeSkip = st.api.length;
  await page.locator(".account .card .ghost.small").first().click();
  await page.waitForTimeout(400);
  check(st.api.length === beforeSkip, `${tag}: skipping an option asks nothing (measured ${st.api.length - beforeSkip})`);
  check((await page.locator(".account .skipped").count()) === 1, `${tag}: the skipped card says it was ignored`);

  const beforeDone = st.accountDonePost;
  await page.locator(".finish").click();
  await page.waitForFunction(() => location.hash === "#/", null, { timeout: 30000 });
  await page.waitForSelector(phone ? "nav.mobilenav" : "nav.sidebar", { timeout: 30000 });
  check(st.accountDonePost - beforeDone === 1, `${tag}: Terminer sends exactly 1 POST (measured ${st.accountDonePost - beforeDone})`);
  const final = await overflow(page);
  check(final.doc <= final.width + 1, `${tag}: the home after onboarding does not overflow`);

  await ctx.close();
  return { st, tiles, cols };
}

console.log("\n-- 390x844: invitation, artists, a like, a refresh, the account step");
const phone = await runFlow({ width: 390, height: 844, phone: true, token: seed.tokens[0].token, name: "bancphone" });
note("390: API calls by the whole flow", phone.st.api.length);
note("390: grid", `${phone.tiles} tiles, ${phone.cols} columns`);

console.log("\n-- 1440x900: the same flow at the desktop grid");
const desktop = await runFlow({ width: 1440, height: 900, phone: false, token: seed.tokens[1].token, name: "bancdesk" });
note("1440: API calls by the whole flow", desktop.st.api.length);
note("1440: grid", `${desktop.tiles} tiles, ${desktop.cols} columns`);

console.log("\n-- a server with no artists: two retries, Passer pour l'instant");
const empty = await startServer([
  "--tracks",
  "0",
  "--playlists",
  "3",
  "--favorites",
  "50",
  "--big-playlist",
  "100",
  "--runs",
  "1",
  "--onboarding",
  "--dist",
  DIST,
]);
const emptySeed = empty.ids && empty.ids.onboarding;
if (!emptySeed || !emptySeed.tokens || !emptySeed.tokens.length) {
  check(false, "the empty server seed has no invitation token");
} else {
  const ctx = await browser.newContext({
    viewport: { width: 390, height: 844 },
    deviceScaleFactor: 2,
    isMobile: true,
    hasTouch: true,
  });
  await ctx.route(/dzcdn\.net|api\.deezer\.com|lrclib/, (r) => r.abort());
  const page = await ctx.newPage();
  const st = track(page);
  await page.goto(empty.base + "/app/#/invite/" + encodeURIComponent(emptySeed.tokens[0].token));
  await page.waitForSelector("#invite-name", { timeout: 30000 });
  await page.locator("#invite-name").fill("sansartiste");
  await page.locator("#invite-password").fill(emptySeed.password);
  await page.locator("#invite-confirm").fill(emptySeed.password);
  await page.locator(".submit").click();
  await page.waitForFunction(() => location.hash === "#/onboarding/artists", null, { timeout: 30000 });
  await page.waitForFunction(
    () => document.body.innerText.includes("Aucun artiste n'est disponible pour l'instant."),
    null,
    { timeout: 30000 }
  );
  check((await page.locator("button.tile").count()) === 0, "empty server: no artist tile");
  check(
    (await page.locator(".finish").getAttribute("aria-disabled")) === "true",
    "empty server: Terminé is disabled"
  );
  const cause = (await page.locator(".failure .cause").innerText()).trim();
  check(cause.length > 0, `empty server: the cause is named ("${cause}")`);

  await page.getByRole("button", { name: /Réessayer/ }).first().click();
  await page.waitForTimeout(500);
  check(
    (await page.getByRole("button", { name: "Passer pour l'instant" }).count()) === 0,
    "empty server: the escape hatch waits for a second attempt"
  );
  await page.getByRole("button", { name: /Réessayer/ }).first().click();
  await page.getByRole("button", { name: "Passer pour l'instant" }).waitFor({ timeout: 15000 });
  check(st.retryPost === 2, `empty server: 2 retries, 2 POST (measured ${st.retryPost})`);
  const beforeSkip = st.donePost;
  await page.getByRole("button", { name: "Passer pour l'instant" }).click();
  await page.waitForFunction(() => location.hash === "#/", null, { timeout: 30000 });
  await page.waitForSelector("nav.mobilenav", { timeout: 30000 });
  check(st.donePost - beforeSkip === 1, `empty server: Passer sends 1 skip POST (measured ${st.donePost - beforeSkip})`);
  check(st.accountDonePost === 0, "empty server: the account step is skipped");
  const ov = await overflow(page);
  check(ov.doc <= ov.width + 1, "empty server: the home does not overflow");
  await ctx.close();
}
empty.stop();

await browser.close();
server.stop();
console.log("");
if (state.failed) {
  console.log(`FAIL: ${state.failed} check(s) failed`);
  process.exit(1);
}
console.log("ok — invitation, artists, similar tiles, refresh, account step and the no-artist escape");
