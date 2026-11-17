#!/usr/bin/env node
import { existsSync, readFileSync } from "node:fs";
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
  "400",
  "--playlists",
  "3",
  "--favorites",
  "50",
  "--big-playlist",
  "100",
  "--runs",
  "1",
  "--admin-users",
  "200",
  "--admin-invites",
  "20",
  "--dist",
  DIST,
]);
const seed = server.ids && server.ids.admin;
if (!seed) {
  console.log("skip: tools/perf_api.py --admin-users did not seed accounts");
  server.stop();
  process.exit(0);
}

const browser = await chromium.launch({ executablePath, args: ["--no-sandbox"] });

function trackAdmin(page) {
  const requests = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (!url.pathname.startsWith("/api/admin/")) return;
    requests.push({
      method: request.method(),
      path: url.pathname,
      query: url.search,
      at: requests.length,
    });
  });
  return requests;
}

async function loginVia(ctx, username, password) {
  const response = await ctx.request.post(server.base + "/api/login", {
    data: { username, password },
  });
  if (!response.ok()) throw new Error(`login ${username}: ${response.status()}`);
}

async function openUsers(page) {
  await page.goto(server.base + "/app/#/settings/users");
  await page.waitForSelector('button[aria-label^="Ouvrir le compte de"]', { timeout: 30000 });
  await page.waitForFunction(
    () => document.querySelectorAll('button[aria-label^="Ouvrir le compte de"]').length >= 25,
    null,
    { timeout: 30000 }
  );
  await page.waitForTimeout(400);
}

const userCount = (page) =>
  page.locator('button[aria-label^="Ouvrir le compte de"]').count();

async function measure(requests, action) {
  const start = requests.length;
  await action();
  await new Promise((resolve) => setTimeout(resolve, 500));
  return requests.slice(start);
}

const mutations = (entries, method) => entries.filter((entry) => entry.method === method);

const overflow = (page) =>
  page.evaluate(() => ({
    doc: document.documentElement.scrollWidth,
    width: document.documentElement.clientWidth,
  }));

async function heightOf(page, selector) {
  return page.locator(selector).first().evaluate((el) => el.getBoundingClientRect().height);
}

console.log("\n-- 1440x900: the list, search, pagination, invitations, accounts");
{
  const ctx = await browser.newContext({
    viewport: { width: 1440, height: 900 },
    permissions: ["clipboard-read", "clipboard-write"],
  });
  await ctx.route(/dzcdn\.net|api\.deezer\.com|lrclib/, (r) => r.abort());
  await loginVia(ctx, "bench", "Bench1");
  const page = await ctx.newPage();
  const requests = trackAdmin(page);
  const allPaths = [];
  page.on("request", (request) => {
    try {
      allPaths.push(new URL(request.url()).pathname);
    } catch {}
  });
  page.on("pageerror", (e) => console.log("pageerror:", e.message));

  await openUsers(page);
  const serverCards = allPaths.filter((path) =>
    /^\/api\/(settings|archive\/|storage|releases\/settings)/.test(path)
  );
  check(
    serverCards.length === 0,
    `1440: opening the users category loads no server card (measured ${serverCards.length}: ${serverCards.join(", ")})`
  );
  const initial = requests.slice();
  check(
    initial.length === 2 &&
      initial.filter((entry) => entry.method === "GET").length === 2 &&
      initial.some((entry) => entry.path === "/api/admin/users") &&
      initial.some((entry) => entry.path === "/api/admin/invites"),
    `1440: opening the list asks 1 users GET + 1 invites GET (measured ${JSON.stringify(initial.map((e) => e.method + " " + e.path))})`
  );
  check((await userCount(page)) === 25, `1440: the first page holds 25 accounts (measured ${await userCount(page)})`);
  check(await page.locator("table").isVisible(), "1440: the 1280+ layout is the table");
  const tableBox = await page.locator(".board table").boundingBox();
  check(tableBox && tableBox.width >= 700, `1440: the table is at least 700 px wide (measured ${tableBox && tableBox.width.toFixed(1)})`);
  const first = await overflow(page);
  check(first.doc <= first.width + 1, `1440: the list does not overflow (${first.doc}/${first.width})`);

  const search = await measure(requests, async () => {
    await page.locator('input[aria-label="Rechercher un compte"]').fill("cam");
    await page.waitForFunction(
      () => document.querySelectorAll('button[aria-label^="Ouvrir le compte de"]').length === 20,
      null,
      { timeout: 15000 }
    );
  });
  check(
    search.filter((entry) => entry.path === "/api/admin/users").length === 1,
    `1440: a search asks 1 users GET (measured ${JSON.stringify(search.map((e) => e.method + " " + e.path + e.query))})`
  );
  check((await userCount(page)) === 20, `1440: the search shows its 20 accounts (measured ${await userCount(page)})`);

  const clear = await measure(requests, async () => {
    await page.locator('.clear[aria-label="Effacer la recherche"]').click();
    await page.waitForFunction(
      () => document.querySelectorAll('button[aria-label^="Ouvrir le compte de"]').length === 25,
      null,
      { timeout: 15000 }
    );
  });
  check(
    clear.filter((entry) => entry.path === "/api/admin/users").length === 1,
    `1440: clearing the search asks 1 users GET (measured ${clear.length})`
  );

  const more = await measure(requests, async () => {
    await page.getByRole("button", { name: "Charger plus" }).click();
    await page.waitForFunction(
      () => document.querySelectorAll('button[aria-label^="Ouvrir le compte de"]').length === 50,
      null,
      { timeout: 15000 }
    );
  });
  check(
    more.filter((entry) => entry.path === "/api/admin/users").length === 1,
    `1440: Charger plus asks 1 users GET (measured ${more.length})`
  );
  const labels = await page
    .locator('button[aria-label^="Ouvrir le compte de"]')
    .evaluateAll((els) => els.map((el) => el.getAttribute("aria-label")));
  check(labels.length === 50 && new Set(labels).size === 50, `1440: the second page adds no duplicate (measured ${labels.length}/${new Set(labels).size})`);

  const disabledSearch = await measure(requests, async () => {
    await page.locator('input[aria-label="Rechercher un compte"]').fill("deja");
    await page.waitForFunction(
      () => document.querySelectorAll('button[aria-label^="Ouvrir le compte de"]').length === 1,
      null,
      { timeout: 15000 }
    );
  });
  check(
    disabledSearch.filter((entry) => entry.path === "/api/admin/users").length === 1,
    `1440: searching a disabled account asks 1 users GET (measured ${disabledSearch.length})`
  );
  const filter = await measure(requests, async () => {
    await page.getByRole("button", { name: /^Désactivés/ }).click();
    await page.waitForTimeout(300);
  });
  check(
    filter.length === 0,
    `1440: an état filter reads the loaded rows and asks nothing (measured ${filter.length})`
  );
  check((await userCount(page)) === 1, "1440: the Désactivés filter keeps the one disabled account");
  await page.getByRole("button", { name: /^Tous/ }).click();
  await page.locator('.clear[aria-label="Effacer la recherche"]').click();
  await page.waitForFunction(
    () => document.querySelectorAll('button[aria-label^="Ouvrir le compte de"]').length === 25,
    null,
    { timeout: 15000 }
  );

  const invite = await measure(requests, async () => {
    await page.getByRole("button", { name: /Inviter/ }).first().click();
    await page.locator("#invite-note").fill("Banc invitation");
    await page.getByRole("button", { name: "Créer le lien" }).click();
    await page.locator("input.link-field").waitFor({ timeout: 15000 });
  });
  check(
    mutations(invite, "POST").filter((entry) => entry.path === "/api/admin/invites").length === 1,
    `1440: creating an invitation sends 1 POST (measured ${JSON.stringify(invite.map((e) => e.method + " " + e.path))})`
  );
  const link = await page.locator("input.link-field").inputValue();
  check(
    link.includes("/app/#/invite/") && /#\/invite\/[A-Za-z0-9_.-]{40,}/.test(link),
    `1440: the link is shown once, with a token (${link})`
  );
  check((await page.locator("input.link-field").count()) === 1, "1440: exactly one link field is shown");
  await page.locator(".sheet").getByRole("button", { name: "Copier le lien" }).click();
  await page.locator(".sheet").getByRole("button", { name: "Lien copié" }).waitFor({ timeout: 5000 });
  check(true, "1440: copying the link answers in place");
  await page.locator('.sheet button[aria-label="Fermer"]').click();
  await page.locator(".sheet").waitFor({ state: "detached", timeout: 10000 });

  await page.getByRole("button", { name: /Ouvrir l'invitation Banc invitation/ }).first().click();
  await page.locator(".dialog[role='dialog']").waitFor({ timeout: 10000 });
  check(
    (await page.locator(".dialog input.link-field").count()) === 0,
    "1440: reopening the created invitation shows no link field"
  );
  check(
    (await page.getByText("Le lien n'est affiché qu'à sa création").count()) === 1,
    "1440: the created invitation keeps only the copy action, with its explanation"
  );
  await page.locator('.dialog[role="dialog"] button[aria-label="Fermer"]').click();

  await page.getByRole("button", { name: /Ouvrir l'invitation Invitation 02/ }).first().click();
  await page.locator(".dialog[role='dialog']").waitFor({ timeout: 10000 });
  check(
    (await page.getByText("Lien non disponible").count()) === 1,
    "1440: a seeded invitation whose link this session never saw says it is unavailable"
  );
  await page.locator('.dialog[role="dialog"] button[aria-label="Fermer"]').click();

  const revoke = await measure(requests, async () => {
    await page.getByRole("button", { name: /Ouvrir l'invitation Invitation 01/ }).first().click();
    await page.getByRole("button", { name: /Révoquer l'invitation/ }).click();
    await page.locator(".confirm-dialog").getByRole("button", { name: "Révoquer l'invitation" }).click();
    await page.waitForFunction(
      () =>
        ![...document.querySelectorAll("button[aria-label]")].some((el) =>
          el.getAttribute("aria-label").includes("Invitation 01")
        ),
      null,
      { timeout: 15000 }
    );
  });
  check(
    mutations(revoke, "POST").filter((entry) => /\/api\/admin\/invites\/[^/]+\/revoke$/.test(entry.path)).length === 1,
    `1440: revoking an invitation sends 1 POST (measured ${JSON.stringify(revoke.map((e) => e.method + " " + e.path))})`
  );

  await page.locator('input[aria-label="Rechercher un compte"]').fill("desactive");
  await page.waitForFunction(
    () => document.querySelectorAll('button[aria-label^="Ouvrir le compte de"]').length === 2,
    null,
    { timeout: 15000 }
  );
  const disable = await measure(requests, async () => {
    await page.getByRole("button", { name: "Ouvrir le compte de desactive_moi" }).click();
    await page.getByRole("button", { name: /Désactiver le compte/ }).click();
    await page.locator(".confirm-dialog").getByRole("button", { name: "Désactiver le compte" }).click();
    await page.waitForFunction(
      () => !document.querySelector('.dialog[role="dialog"]'),
      null,
      { timeout: 15000 }
    );
  });
  check(
    mutations(disable, "PATCH").length === 1,
    `1440: disabling an account sends 1 PATCH (measured ${JSON.stringify(disable.map((e) => e.method + " " + e.path))})`
  );
  const disabledRow = await page.locator("tr", { hasText: "desactive_moi" }).first().innerText();
  check(disabledRow.includes("Désactivé"), `1440: the disabled row says Désactivé (${disabledRow.replace(/\s+/g, " ").trim()})`);

  await page.locator('input[aria-label="Rechercher un compte"]').fill("bench");
  await page.waitForFunction(
    () => document.querySelectorAll('button[aria-label^="Ouvrir le compte de"]').length === 1,
    null,
    { timeout: 15000 }
  );
  await page.getByRole("button", { name: "Ouvrir le compte de bench" }).click();
  await page.locator(".dialog[role='dialog']").waitFor({ timeout: 10000 });
  check(
    (await page.getByText("Vous ne pouvez pas modifier votre propre compte ici.").count()) === 1,
    "1440: the self account is guarded in the UI"
  );
  const selfActions = await page.locator(".dialog button:disabled").count();
  check(selfActions >= 3, `1440: the self actions are disabled (measured ${selfActions})`);
  await page.locator('.dialog[role="dialog"] button[aria-label="Fermer"]').click();

  const own = await (await ctx.request.get(server.base + "/api/admin/users?q=bench")).json();
  const me = own.users.find((entry) => entry.name === "bench");
  const guard = await ctx.request.patch(server.base + "/api/admin/users/" + me.id, {
    data: { admin: false },
  });
  const guardBody = await guard.json();
  check(
    guard.status() === 409 && guardBody.code === "last_admin",
    `1440: demoting the last active admin is a real 409 last_admin (measured ${guard.status()} ${guardBody.code})`
  );

  await page.route("**/api/admin/users/*", async (route) => {
    if (route.request().method() === "PATCH") {
      await route.fulfill({
        status: 409,
        contentType: "application/json",
        body: JSON.stringify({
          error: "Impossible : c'est le dernier administrateur actif.",
          code: "last_admin",
        }),
      });
    } else {
      await route.continue();
    }
  });
  await page.locator('input[aria-label="Rechercher un compte"]').fill("supprime");
  await page.waitForFunction(
    () => document.querySelectorAll('button[aria-label^="Ouvrir le compte de"]').length === 1,
    null,
    { timeout: 15000 }
  );
  await page.getByRole("button", { name: "Ouvrir le compte de supprime_moi" }).click();
  await page.getByRole("button", { name: /Nommer administrateur/ }).click();
  await page.locator(".confirm-dialog").getByRole("button", { name: "Nommer administrateur" }).click();
  await page.getByText("Impossible : c'est le dernier administrateur actif.").waitFor({ timeout: 10000 });
  check(true, "1440: a 409 last_admin is shown in French");
  await page.locator(".confirm-dialog").getByRole("button", { name: "Annuler" }).click();
  await page.locator('.dialog[role="dialog"] button[aria-label="Fermer"]').click();
  await page.unroute("**/api/admin/users/*");

  const remove = await measure(requests, async () => {
    await page.getByRole("button", { name: "Ouvrir le compte de supprime_moi" }).click();
    await page.getByRole("button", { name: /Supprimer le compte/ }).click();
    await page.locator("#admin-confirm-typed").waitFor({ timeout: 15000 });
    await page.locator("#admin-confirm-typed").fill("supprime_moi");
    await page.getByRole("button", { name: "Supprimer définitivement" }).click();
    await page.waitForFunction(
      () => !document.querySelector('button[aria-label="Ouvrir le compte de supprime_moi"]'),
      null,
      { timeout: 15000 }
    );
  });
  check(
    mutations(remove, "DELETE").length === 1,
    `1440: deleting an account sends 1 DELETE (measured ${JSON.stringify(remove.map((e) => e.method + " " + e.path))})`
  );
  check(
    remove.filter((entry) => entry.method === "GET").length === 1,
    `1440: the delete dialog reads the uploads once (measured ${remove.filter((e) => e.method === "GET").length})`
  );

  const last = await overflow(page);
  check(last.doc <= last.width + 1, `1440: the list still does not overflow (${last.doc}/${last.width})`);
  note("1440: admin requests over the scenario", requests.length);
  await ctx.close();
}

console.log("\n-- a member: no category, no admin request, a bare 403");
{
  const ctx = await browser.newContext({ viewport: { width: 390, height: 844 } });
  await ctx.route(/dzcdn\.net|api\.deezer\.com|lrclib/, (r) => r.abort());
  // The seeded member's password is a fixture the server hands over in an
  // owner-only file, never in the READY line it prints (tools/perf_api.py).
  await loginVia(ctx, seed.member, readFileSync(seed.member_password_file, "utf8"));
  const page = await ctx.newPage();
  const requests = trackAdmin(page);
  await page.goto(server.base + "/app/#/settings/users");
  await page.waitForTimeout(1200);
  check(requests.length === 0, `member: opening #/settings/users asks no admin route (measured ${requests.length})`);
  check(
    (await page.getByText("Utilisateurs", { exact: true }).count()) === 0,
    "member: the Utilisateurs category is not offered"
  );
  const users = await ctx.request.get(server.base + "/api/admin/users");
  const usersBody = await users.text();
  const invites = await ctx.request.get(server.base + "/api/admin/invites");
  check(users.status() === 403 && invites.status() === 403, `member: the admin API answers 403 (measured ${users.status()}/${invites.status()})`);
  check(!usersBody.includes("camille"), "member: the 403 body names no account");
  await ctx.close();
}

console.log("\n-- 390 / 641 / 1280: cards below 1280, the table at 1280, nothing overflowing");
for (const [width, height, phone] of [
  [390, 844, true],
  [641, 900, false],
  [1280, 900, false],
]) {
  const ctx = await browser.newContext({
    viewport: { width, height },
    deviceScaleFactor: phone ? 2 : 1,
    isMobile: phone,
    hasTouch: phone,
  });
  await ctx.route(/dzcdn\.net|api\.deezer\.com|lrclib/, (r) => r.abort());
  await loginVia(ctx, "bench", "Bench1");
  const page = await ctx.newPage();
  await openUsers(page);
  const ov = await overflow(page);
  check(ov.doc <= ov.width + 1, `${width}: the list does not overflow (${ov.doc}/${ov.width})`);
  const table = await page.locator(".board table").count();
  const cards = await page.locator("button.row-card").count();
  if (width >= 1280) {
    check(table === 1 && cards === 0, `${width}: the table is used (measured table ${table}, cards ${cards})`);
  } else {
    check(table === 0 && cards > 0, `${width}: cards are used (measured table ${table}, cards ${cards})`);
    const heights = await page
      .locator("button.row-card")
      .evaluateAll((els) => els.slice(0, 10).map((el) => el.getBoundingClientRect().height));
    check(heights.every((value) => value >= 43.5), `${width}: every card is at least 44 px (measured ${JSON.stringify(heights.map((v) => Math.round(v)))})`);
  }
  check((await heightOf(page, ".admin .search")) >= 43.5, `${width}: the search field is 44 px`);
  const inviteButton = await page.locator(".head-actions .invite").boundingBox();
  check(inviteButton && inviteButton.height >= 43.5, `${width}: Inviter is 44 px`);
  note(`${width}: table/cards`, `${table}/${cards}`);
  await ctx.close();
}

await browser.close();
server.stop();
console.log("");
if (state.failed) {
  console.log(`FAIL: ${state.failed} check(s) failed`);
  process.exit(1);
}
console.log("ok — accounts list, search, pagination, invitations, guarded mutations, member 403 and 390/641/1280");
