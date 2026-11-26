#!/usr/bin/env node
// Les trois scénarios du propriétaire, en tête du rapport (plan-bughunt §8.6).
//
//   npm run build && node test/bughunt/stress.mjs            # les trois seuls
//   node test/bughunt/stress.mjs --only '#/search'           # sans lecteur → sort 1
//
// run.mjs les appelle après la passe (état chaud, comme une personne qui a
// navigué) ; ce fichier les rejoue seuls, avec le même serveur de banc.
import { pathToFileURL } from "node:url";
import { writeFileSync, unlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadPlaywright, startServer, login, silence, installSessionPrefix, deezerHost } from "../harness.mjs";
import { SCREENS, idsInto, openScreen, installObservation, shot, slug, REPO } from "./lib.mjs";

const GAP_MS = 100;
const TAPS = 20;

// --- reading what the player shows -----------------------------------------
async function readTitle(page) {
  return page
    .evaluate(() => {
      const el = document.querySelector(".m .info button.t, .m .info .t, footer.player .now .t");
      return el ? el.textContent.replace(/\s+/g, " ").trim() : "";
    })
    .catch(() => "");
}

async function readAudio(page) {
  return page
    .evaluate(() => {
      const all = [...(window.__bh?.els || []), ...document.querySelectorAll("audio")];
      const live = all.find((a) => !a.paused) || all[0];
      return {
        count: all.length,
        paused: live ? live.paused : null,
        t: live ? live.currentTime : null,
        ready: live ? live.readyState : null,
        src: live ? live.currentSrc : "",
      };
    })
    .catch(() => ({ count: 0 }));
}

/** page.evaluate that survives a navigation racing it. */
async function evalAfterNav(page, fn, arg) {
  for (let attempt = 0; attempt < 4; attempt++) {
    try {
      return await page.evaluate(fn, arg);
    } catch (e) {
      if (!/Execution context was destroyed|navigat/i.test(String(e.message || e))) throw e;
      await page.waitForTimeout(300);
    }
  }
  return null;
}

/** login() once more after a navigation raced it (context destroyed). */
async function signIn(page, base) {
  for (let attempt = 0; attempt < 3; attempt++) {
    try {
      await login(page, base);
      return true;
    } catch {
      await page.waitForTimeout(400);
    }
  }
  return false;
}

async function visible(page, selector) {
  return page.locator(selector).first().isVisible().catch(() => false);
}

/**
 * Click the control a finger reaches: the LAST visible one of that role+name.
 * The bar and the full-screen player both carry « Suivant »/« Lecture-Pause »;
 * the first in DOM order is the hidden one, and clicking it times out forever
 * (plan §5.1, rule 2: a click proves the element it touched).
 */
async function clickVisible(page, name, timeout = 2500) {
  const loc = page.getByRole("button", { name, exact: true });
  const n = await loc.count();
  for (let i = n - 1; i >= 0; i--) {
    if (!(await loc.nth(i).isVisible().catch(() => false))) continue;
    try {
      await loc.nth(i).click({ timeout });
      return true;
    } catch {
      /* covered or moving: try the next one */
    }
  }
  return false;
}

/** Start the playback if the element is paused (`.pp` is the play control). */
async function ensurePlaying(page) {
  let a = await readAudio(page);
  if (a.count && !a.paused) return true;
  const pps = page.locator('button[aria-label="Lecture/Pause"]');
  const visible = [];
  for (let i = 0; i < (await pps.count()); i++) if (await pps.nth(i).isVisible().catch(() => false)) visible.push(i);
  // The full-screen player is later in the DOM than the bar: when both are
  // "visible", the overlay's own control is the one a finger reaches.
  for (const idx of visible.reverse()) {
    await pps.nth(idx).click({ timeout: 2500 }).catch(() => {});
    for (let i = 0; i < 15; i++) {
      await page.waitForTimeout(100);
      a = await readAudio(page);
      if (a.count && !a.paused) return true;
    }
  }
  return !!a.count && !a.paused;
}

/** Open the full-screen player; false when it cannot open (that is a bug). */
async function openPlayer(page, ctx) {
  if (await visible(page, 'button[aria-label="Réduire"]')) return true;
  const disabled = await page
    .getByRole("button", { name: "Ouvrir le lecteur plein écran", exact: false })
    .evaluateAll((els) => els.every((e) => e.disabled))
    .catch(() => true);
  if (disabled) return false;
  if (!(await clickVisible(page, "Ouvrir le lecteur plein écran", 4000))) return false;
  return page
    .locator('button[aria-label="Réduire"]')
    .last()
    .waitFor({ state: "visible", timeout: 6000 })
    .then(() => true)
    .catch(() => false);
}

/** The player's clock, watched for 1.5 s: does it advance? */
async function clockDelta(page) {
  const a0 = await readAudio(page);
  await page.waitForTimeout(1500);
  const a1 = await readAudio(page);
  if (a0.t == null || a1.t == null) return { delta: null, playing: false };
  return { delta: +(a1.t - a0.t).toFixed(2), playing: !a1.paused };
}

/** The index of a title in the injected queue (titles are the visible key). */
function queueIndex(queue, title) {
  return queue.findIndex((t) => t.title === title);
}

/**
 * Put the 12-track bench queue back, the way the app boots from it. The pass
 * clicks « Lancer mon Flow », a playlist's « Lire »… and the queue it leaves is
 * not the one the burst needs (measured: the current title was not in it any
 * more, and the scenario failed for the wrong reason).
 */
async function resetQueue(page, queue) {
  // Through sessionStorage, not localStorage: the app saves its own session as
  // the page unloads, so a direct write is overwritten by the reload itself
  // (harness.mjs's `__inject` runs at document start, after that save).
  await page.evaluate((songs) => {
    sessionStorage.setItem(
      "__inject",
      JSON.stringify({ queue: songs, index: 0, start: 0, currentTime: 0, context: null, shuffle: false, repeat: "all", _orig: null, at: Date.now() })
    );
  }, queue);
  await page.reload();
  await page.waitForSelector('nav a[href="#/"], .mobilenav a[href="#/"], [role=navigation] a[href="#/"]', { timeout: 20000, state: "attached" }).catch(() => {});
  await page.waitForTimeout(600);
}

/**
 * §8.6 — the forward burst, then the alternating one. Returns TWO verdicts
 * (the report's table has one row each): the contract is `bughunt: …` lines,
 * not a single object.
 */
export async function skipBurst(page, { base, ctx, queue }) {
  const verdicts = [];
  const errorsFrom = ctx.events.pageerrors.length;
  const fail = (title, reason, numbers) => ({
    title,
    verdict: "fail",
    numbers: numbers || "",
    reason,
    outcome: reason === "pageerror" ? "exception" : "nothing",
  });

  // A known queue first: the current title must be one of the 12 the burst
  // counts through.
  let startTitle = "";
  let startIndex = -1;
  for (let attempt = 0; attempt < 2; attempt++) {
    startTitle = await readTitle(page);
    startIndex = queueIndex(queue, startTitle);
    if (startIndex >= 0) break;
    if (attempt === 0) await resetQueue(page, queue);
  }
  const openTitle = "Rafale Suivant ×20 à 100 ms";
  const early = async (reason, numbers) => {
    const evidence = ctx.dir ? await shot(page, ctx, { name: "scenario-rafale" }, 0, slug(openTitle, 24), "fail") : null;
    return [
      { ...fail(openTitle, reason, `${numbers} reason=${reason}`), evidence },
      { ...fail("Rafale alternée ×20", reason, `taps=0 reason=${reason}`), evidence },
    ];
  };
  if (!(await openPlayer(page, ctx))) return early("player-not-open", "taps=0 gap_ms=100 playing=0");
  if (!(await ensurePlaying(page))) return early("not-playing", "taps=0 playing=0");
  startTitle = await readTitle(page);
  startIndex = queueIndex(queue, startTitle);
  if (startIndex < 0) return early("title-not-found", `title=${JSON.stringify(startTitle)}`);

  let tapped = 0;
  for (let i = 0; i < TAPS; i++) {
    if (await clickVisible(page, "Suivant")) tapped += 1;
    await page.waitForTimeout(GAP_MS);
  }
  await page.waitForTimeout(2000);
  const expected = (startIndex + TAPS) % queue.length;
  const expectedTitle = queue[expected].title;
  const shown = await readTitle(page);
  const clock = await clockDelta(page);
  let titleChanged = false;
  if (shown === expectedTitle) {
    await clickVisible(page, "Suivant");
    await page.waitForTimeout(1200);
    titleChanged = (await readTitle(page)) !== shown;
  }
  const pageerror = ctx.events.pageerrors.length > errorsFrom;
  const numbers = `taps=${TAPS} tapped=${tapped} gap_ms=${GAP_MS} playing=${clock.playing ? 1 : 0} clock_delta=${clock.delta == null ? "n/a" : clock.delta.toFixed(2) + "s"} title_changed=${titleChanged ? 1 : 0} expected=${JSON.stringify(expectedTitle)} shown=${JSON.stringify(shown)}`;
  const withReason = (v) => (v.verdict === "fail" ? { ...v, numbers: `${v.numbers} reason=${v.reason}` } : v);
  if (pageerror) verdicts.push(fail(openTitle, "pageerror", numbers));
  else if (!clock.playing) verdicts.push(fail(openTitle, "not-playing", numbers));
  else if (clock.delta == null || clock.delta < 0.5) verdicts.push(fail(openTitle, "clock-frozen", numbers));
  else if (shown !== expectedTitle) verdicts.push(fail(openTitle, "title-mismatch", numbers));
  else if (!titleChanged) verdicts.push(fail(openTitle, "title-stuck", numbers));
  else verdicts.push({ title: openTitle, verdict: "pass", numbers });

  // Alternating: 20 taps end on the same index (each pair returns), then one
  // more forward tap must change the title.
  const altTitle = "Rafale alternée ×20";
  const before = await readTitle(page);
  const beforeIndex = queueIndex(queue, before);
  let altTapped = 0;
  for (let i = 0; i < TAPS / 2; i++) {
    if (await clickVisible(page, "Suivant")) altTapped += 1;
    await page.waitForTimeout(GAP_MS);
    if (await clickVisible(page, "Précédent")) altTapped += 1;
    await page.waitForTimeout(GAP_MS);
  }
  await page.waitForTimeout(2000);
  const altShown = await readTitle(page);
  const altClock = await clockDelta(page);
  let altChanged = false;
  if (beforeIndex >= 0 && altShown === before) {
    await clickVisible(page, "Suivant");
    await page.waitForTimeout(1200);
    altChanged = (await readTitle(page)) !== altShown;
  }
  const altNumbers = `taps=${TAPS} tapped=${altTapped} gap_ms=${GAP_MS} playing=${altClock.playing ? 1 : 0} clock_delta=${altClock.delta == null ? "n/a" : altClock.delta.toFixed(2) + "s"} title_changed=${altChanged ? 1 : 0} expected=${JSON.stringify(before)} shown=${JSON.stringify(altShown)}`;
  const altPageerror = ctx.events.pageerrors.length > errorsFrom + (pageerror ? 1 : 0);
  if (altPageerror) verdicts.push(fail(altTitle, "pageerror", altNumbers));
  else if (!altClock.playing) verdicts.push(fail(altTitle, "not-playing", altNumbers));
  else if (altClock.delta == null || altClock.delta < 0.5) verdicts.push(fail(altTitle, "clock-frozen", altNumbers));
  else if (beforeIndex < 0 || altShown !== before) verdicts.push(fail(altTitle, "title-mismatch", altNumbers));
  else if (!altChanged) verdicts.push(fail(altTitle, "title-stuck", altNumbers));
  else verdicts.push({ title: altTitle, verdict: "pass", numbers: altNumbers });

  for (const v of verdicts) {
    if (v.verdict === "fail" && ctx.dir) {
      v.evidence = await shot(page, ctx, { name: "scenario-rafale" }, 0, slug(v.title, 24), "fail");
    }
  }
  return verdicts.map(withReason);
}

/** A control's aria value (slider/switch), for the change-then-return check. */
async function ariaValue(loc) {
  return loc.getAttribute("aria-valuenow").catch(() => null);
}

/**
 * §8.5 — every control of every Settings section, applied for real: a switch,
 * a checkbox, the second option of a select, the next radio, a slider by
 * ArrowRight then ArrowLeft, a text box by typing zzz then Escape. After each:
 * the main content is non-empty, no exception, and a server write is followed
 * by a reload that repaints the section.
 */
export async function settingsApply(page, ctx) {
  const title = "Appliquer chaque réglage";
  let controls = 0;
  let blanks = 0;
  let reloadsOk = 0;
  let reloadsBroken = 0;
  let skipped = 0;
  const failures = [];
  const errorsFrom = ctx.events.pageerrors.length;
  const sections = SCREENS.filter((s) => s.section).map((s) => ({ ...s, concreteHash: idsInto(s, ctx.ids) }));

  for (const section of sections) {
    if (!(await openScreen(page, ctx.base, section, ctx.ids))) continue;
    const host = page.locator("main .route-host");
    const snap = await host.ariaSnapshot().catch(() => "");
    const wanted = [];
    const unnamed = new Map();
    for (const line of snap.split("\n")) {
      const m = line.match(/^\s*-\s+(switch|checkbox|combobox|radio|slider|textbox|searchbox)(?:\s+"((?:[^"\\]|\\.)*)")?/);
      if (!m) continue;
      const name = (m[2] || "").replace(/\s+/g, " ").trim();
      const occ = name ? 0 : unnamed.get(m[1]) || 0;
      if (!name) unnamed.set(m[1], occ + 1);
      wanted.push({ role: m[1], name, occurrence: occ });
    }
    for (const want of wanted) {
      // Only the [role, name] pair is kept from the first snapshot: a control
      // changed earlier in the section can re-render the rest, so the locator
      // is looked up again right before it is used.
      let el;
      if (!want.name) {
        const byRole = host.getByRole(want.role);
        if ((await byRole.count()) <= want.occurrence) continue;
        el = { ...want, loc: byRole.nth(want.occurrence) };
      } else {
        el = { ...want, loc: host.getByRole(want.role, { name: want.name, exact: true }).first() };
        if ((await el.loc.count()) === 0) {
          const loose = host.getByRole(want.role, { name: want.name, exact: false }).first();
          if ((await loose.count()) > 0) el.loc = loose;
          else continue; // gone: the section re-rendered, this control no longer exists
        }
      }
      if (await el.loc.isDisabled().catch(() => false)) continue;
      if (!(await el.loc.isVisible().catch(() => false))) continue;
      controls += 1;
      const writesFrom = ctx.events.writes.length;
      const errFrom = ctx.events.pageerrors.length;
      try {
        if (el.role === "combobox") {
          await el.loc.selectOption({ index: 1 }, { timeout: 3000 });
        } else if (el.role === "slider") {
          const before = await ariaValue(el.loc);
          await el.loc.focus();
          await page.keyboard.press("ArrowRight");
          await page.waitForTimeout(200);
          const after = await ariaValue(el.loc);
          await page.keyboard.press("ArrowLeft");
          await page.waitForTimeout(200);
          const back = await ariaValue(el.loc);
          if (before != null && after == null) failures.push(`${section.name} « ${el.name} » slider sans aria-valuenow`);
          else if (before != null && after === before) failures.push(`${section.name} « ${el.name} » slider ne change pas`);
          else if (before != null && back !== before) failures.push(`${section.name} « ${el.name} » slider ne revient pas`);
        } else if (el.role === "textbox" || el.role === "searchbox") {
          // A colour field rejects anything that is not a colour: `zzz` is the
          // plan's hostile text, a hex value is the same test on this input.
          const type = await el.loc.evaluate((n) => n.getAttribute("type") || "").catch(() => "");
          await el.loc.fill(type === "color" ? "#123456" : "zzz", { timeout: 3000 });
          await page.keyboard.press("Escape");
        } else {
          // A custom switch/radio is a hidden input under its label: an
          // intercepted click is the label's to take, not a dead control.
          await el.loc.click({ timeout: 2500 }).catch(async (e) => {
            if (!/intercepts pointer|not visible/i.test(String(e.message || e))) throw e;
            await el.loc.click({ force: true, timeout: 1500 });
          });
        }
      } catch (e) {
        // A visually hidden input (a custom radio drawn by its label) cannot be
        // clicked itself: the pass clicks the labelled control, and this
        // scenario skips it rather than calling the app broken.
        if (/not visible|intercepts pointer|Timeout/i.test(String(e.message || e))) {
          skipped += 1;
          continue;
        }
        failures.push(`${section.name} « ${el.name} » ${el.role} : ${String(e.message || e).slice(0, 100)}`);
        continue;
      }
      await page.waitForTimeout(450);
      const mainText = await page.locator("main").innerText().catch(() => "");
      if (mainText.trim().length < 20) {
        blanks += 1;
        failures.push(`${section.name} « ${el.name} » : contenu principal vide (${mainText.trim().length} caractères)`);
        await shot(page, ctx, { name: section.name }, 0, slug(el.name, 24), "blank");
      }
      if (ctx.events.pageerrors.length > errFrom) {
        failures.push(`${section.name} « ${el.name} » : exception ${ctx.events.pageerrors.at(-1).msg}`);
      }
      const wrote = ctx.events.writes.slice(writesFrom).some((w) => w.url.includes("/api/"));
      if (wrote) {
        await page.reload();
        await page.goto(ctx.base + "/app/" + section.concreteHash);
        // The section, NOT the section's own marker: turning eco on removes the
        // world picker from Réglages → Animations, and a marker that names it
        // would call a correct repaint broken (measured).
        const ok = await page.waitForSelector("main h1", { timeout: 8000, state: "attached" }).then(() => true).catch(() => false);
        const painted = ok && (await page.locator("main").innerText()).trim().length >= 20;
        if (painted) reloadsOk += 1;
        else {
          reloadsBroken += 1;
          failures.push(`${section.name} « ${el.name} » : rechargement ne repeint pas la section`);
          await shot(page, ctx, { name: section.name }, 0, slug(el.name, 24), "reload-broken");
        }
      }
    }
  }

  // §8.5 — `Recharger l'application` is out of the pass; clicked once here.
  let hardReload = "skipped";
  const help = { ...SCREENS.find((s) => s.section === "help"), concreteHash: "#/settings/help" };
  if (await openScreen(page, ctx.base, help, ctx.ids)) {
    let btn = page.getByRole("button", { name: /Recharger l'application/i }).first();
    if ((await btn.count()) > 0 && !(await btn.isVisible().catch(() => false))) btn = page.locator('button:has-text("Recharger l\'application")').first();
    if ((await btn.count()) === 0) btn = page.locator('button:has-text("Recharger l\'application")').first();
    if ((await btn.count()) > 0) {
      const themeBefore = await page.evaluate(() => document.documentElement.dataset.theme || "");
      const titleBefore = await readTitle(page);
      await btn.click({ timeout: 4000 }).catch(() => {});
      await page.waitForLoadState("load", { timeout: 15000 }).catch(() => {});
      const shell = await page
        .waitForSelector('nav a[href="#/"], .mobilenav a[href="#/"], [role=navigation] a[href="#/"]', { timeout: 20000, state: "attached" })
        .then(() => true)
        .catch(() => false);
      const themeAfter = await evalAfterNav(page, () => document.documentElement.dataset.theme || "");
      await page.waitForTimeout(600);
      const titleAfter = await readTitle(page);
      if (!shell) failures.push("Recharger l'application : la coquille ne revient pas");
      else if (themeBefore && themeAfter !== themeBefore) failures.push(`Recharger l'application : thème ${themeBefore} → ${themeAfter}`);
      else if (titleBefore && titleAfter !== titleBefore) failures.push(`Recharger l'application : file ${titleBefore} → ${titleAfter}`);
      else hardReload = "ok";
    }
  }

  const pageerror = ctx.events.pageerrors.length > errorsFrom;
  const ok = !failures.length && !blanks && !reloadsBroken && !pageerror;
  const verdict = {
    title,
    verdict: ok ? "pass" : "fail",
    numbers: `settings_apply=${ok ? "ok" : "fail"} controls=${controls} skipped=${skipped} blanks=${blanks} reloads_ok=${reloadsOk} reloads_broken=${reloadsBroken} hard_reload=${hardReload}${failures.length ? ` first=${JSON.stringify(failures[0]).slice(0, 120)}` : ""}`,
    reason: failures[0] || (pageerror ? "pageerror" : undefined),
    outcome: blanks ? "blank" : "state-changed",
  };
  if (!ok && ctx.dir) verdict.evidence = await shot(page, ctx, { name: "scenario-reglages" }, 0, "settings-apply", "fail");
  return verdict;
}

/**
 * §4.5/§8.6 — the third symptom: log out, the app must come back to the login
 * screen with no blank page and no exception, then log back in and the shell
 * must repaint.
 */
export async function sessionRoundTrip(page, { base, ids, ctx }) {
  const title = "Déconnexion puis reconnexion";
  const errorsFrom = ctx.events.pageerrors.length;
  const account = { ...SCREENS.find((s) => s.section === "account"), concreteHash: "#/settings/account" };
  if (!(await openScreen(page, base, account, ids))) {
    return { title, verdict: "fail", numbers: "session_roundtrip=fail login_painted=0 resignin=0", reason: "account-screen-unreachable", outcome: "blank" };
  }
  const out = page.getByRole("button", { name: /^(Déconnexion|Se déconnecter)$/i }).first();
  if ((await out.count()) === 0) {
    return { title, verdict: "fail", numbers: "session_roundtrip=fail login_painted=0 resignin=0", reason: "logout-not-found", outcome: "nothing" };
  }
  await out.click({ timeout: 4000 }).catch(() => {});
  const loginPainted = await page
    .waitForSelector('main h1:has-text("Connexion")', { timeout: 10000 })
    .then(() => true)
    .catch(() => false);
  const textLen = (await page.locator("main").innerText().catch(() => "")).trim().length;
  const pageerror = ctx.events.pageerrors.length > errorsFrom;
  if (!loginPainted || textLen < 20 || pageerror) {
    const evidence = ctx.dir ? await shot(page, ctx, { name: "scenario-session" }, 0, "logout", "fail") : null;
    return {
      title,
      verdict: "fail",
      numbers: `session_roundtrip=fail login_painted=${loginPainted ? 1 : 0} resignin=0 text_len=${textLen}`,
      reason: pageerror ? "pageerror" : loginPainted ? "login-blank" : "logout-stuck",
      outcome: pageerror ? "exception" : "blank",
      evidence,
    };
  }
  // Back in: the same login the pass used, then a reload so the shell reboots
  // with the new cookie (a hash-only goto does not re-boot the SPA).
  await signIn(page, base);
  await page.reload();
  const shell = await page
    .waitForSelector('nav a[href="#/"], .mobilenav a[href="#/"], [role=navigation] a[href="#/"]', { timeout: 20000, state: "attached" })
    .then(() => true)
    .catch(() => false);
  const me = await page.evaluate(() => fetch("/api/me").then((r) => r.status).catch(() => 0));
  const ok = shell && me === 200;
  const verdict = {
    title,
    verdict: ok ? "pass" : "fail",
    numbers: `session_roundtrip=${ok ? "ok" : "fail"} login_painted=1 resignin=${shell ? 1 : 0} me=${me}`,
    reason: ok ? undefined : "resignin-failed",
    outcome: "navigated",
  };
  if (!ok && ctx.dir) verdict.evidence = await shot(page, ctx, { name: "scenario-session" }, 0, "resignin", "fail");
  return verdict;
}

// --- standalone runner ------------------------------------------------------
async function main() {
  const argv = process.argv.slice(2);
  const arg = (name, fallback = "") => {
    const i = argv.indexOf(name);
    return i >= 0 ? argv[i + 1] ?? "" : fallback;
  };
  const ONLY = arg("--only", "");
  const OUT = arg("--out", join(REPO, "webapp/test/bughunt/out"));
  const wavPath = join(tmpdir(), `bughunt-stress-${process.pid}.wav`);
  writeFileSync(wavPath, silence(120));
  const { base, ids, stop } = await startServer([
    "--podcasts",
    "--recap",
    "--covers",
    join(REPO, "tests/assets/cover.jpg"),
    "--audio",
    wavPath,
  ]);
  const { chromium } = await loadPlaywright();
  const browser = await chromium.launch({
    args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--autoplay-policy=no-user-gesture-required"],
  });
  const context = await browser.newContext({ viewport: { width: 412, height: 900 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true });
  const wav = silence(120);
  await context.route(deezerHost, (r) => r.abort());
  await context.route(/\/api\/stream\//, (r) =>
    r.fulfill({ status: 200, body: wav, headers: { "Content-Type": "audio/wav", "Content-Length": String(wav.length) } })
  );
  await context.addInitScript(installSessionPrefix);
  await context.addInitScript(() => {
    const pkey = window.__nsPkey;
    if (!pkey) return;
    const j = sessionStorage.getItem("__inject");
    if (!j) return;
    localStorage.setItem(pkey("player.session"), j);
    localStorage.removeItem(pkey("player.pos"));
    // Repeat all, so 20 forward taps wrap instead of dying at the last track:
    // the burst measures the player, not the queue's end. `player.repeat` is a
    // DEVICE key (lib/stores.js), not a personal one.
    localStorage.setItem("player.repeat", JSON.stringify("all"));
    sessionStorage.removeItem("__inject");
  });
  const page = await context.newPage();
  const events = await installObservation(page);
  await signIn(page, base);
  await page.reload();
  await page.waitForSelector('nav a[href="#/"], .mobilenav a[href="#/"], [role=navigation] a[href="#/"]', { timeout: 20000, state: "attached" }).catch(() => {});

  const q = await page.evaluate(async (album) => {
    const a = await fetch("/api/album/" + album).then((r) => r.json());
    return (a.tracks?.[0]?.title || "").split(/\s+/).slice(0, 3).join(" ");
  }, ids.album);
  const benchIds = { ...ids, q };
  const ctx = { dir: OUT, base, ids: benchIds, page, events, shots: true, maxShots: 40, maxScreenshots: 40, screenShots: 0, anomalyShots: 0, screenshotsCapped: false, clicks: 0, reloads: 0, stopped: null, counts: { duplicates: 0 }, layersSeen: new Set(), layersOutOfDepth: [] };

  let queue = [];
  if (!ONLY) {
    queue = await page.evaluate(async (big) => {
      const pl = await fetch("/api/playlist/" + big).then((r) => r.json());
      return pl.tracks.slice(0, 12).map((t) => ({
        deezer_id: t.deezer_id,
        title: t.title,
        duration: t.duration || 120,
        artist: t.artist ? { deezer_id: t.artist.deezer_id, name: t.artist.name } : { deezer_id: "a", name: "Artiste" },
        album: t.album ? { deezer_id: t.album.deezer_id, title: t.album.title, cover: t.album.cover } : null,
      }));
    }, ids.big);
    await page.evaluate((songs) => {
      sessionStorage.setItem("__inject", JSON.stringify({ queue: songs, index: 0, start: 0, currentTime: 0, context: null, shuffle: false, repeat: "all", _orig: null, at: Date.now() }));
    }, queue);
    await page.reload();
    await page.waitForSelector('nav a[href="#/"], .mobilenav a[href="#/"], [role=navigation] a[href="#/"]', { timeout: 20000, state: "attached" }).catch(() => {});
  } else {
    const concrete = SCREENS.map((s) => ({ ...s, concreteHash: idsInto(s, benchIds) }));
    const screen = concrete.find((s) => s.hash === ONLY || s.concreteHash === ONLY);
    if (!screen) {
      console.error(`bughunt: unknown screen ${ONLY}`);
      process.exit(2);
    }
    await openScreen(page, base, screen, benchIds);
  }

  const verdicts = [];
  if (ONLY) verdicts.push(...(await skipBurst(page, { base, ctx, queue })));
  else {
    verdicts.push(...(await skipBurst(page, { base, ctx, queue })));
    verdicts.push(await settingsApply(page, ctx));
    verdicts.push(await sessionRoundTrip(page, { base, ids: benchIds, ctx }));
  }
  for (const v of verdicts) console.log(`bughunt: ${v.title}: ${v.verdict.toUpperCase()} ${v.numbers}${v.reason ? " reason=" + v.reason : ""}`);
  const failed = verdicts.filter((v) => v.verdict === "fail").length;
  console.log(`stress: verdicts=${verdicts.length} failed=${failed}`);
  await browser.close();
  try {
    unlinkSync(wavPath);
  } catch {
    /* gone */
  }
  stop();
  process.exit(failed ? 1 : 0);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((e) => {
    console.error(`bughunt: tool error — ${e && e.stack ? e.stack : e}`);
    process.exit(2);
  });
}
