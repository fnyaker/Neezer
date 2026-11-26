#!/usr/bin/env node
// La chasse aux bugs web (plan-bughunt, lot B1).
//
//   npm run build && node test/bughunt/run.mjs             # the whole pass (~20 min)
//   node test/bughunt/run.mjs --only '#/settings/apparence' --out /tmp/bh
//   node test/bughunt/run.mjs --max-minutes 5 --max-clicks 100   # smoke
//
// It logs in to tools/perf_api.py --serve, walks EVERY screen of lib.mjs
// SCREENS, clicks every element the accessibility tree names (3 samples per
// list, §4.4), classifies what happened, runs the owner's three scenarios, and
// writes bughunt-web.json + bughunt-web.md + screens/ into --out.
//
// Exit 0: no bloquant bug.  Exit 1: at least one bloquant.  Exit 2: tool error
// (an undeclared route, a missing id, a sound setup failure).
import { writeFileSync, mkdirSync, rmSync, mkdtempSync } from "node:fs";
import { execSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadPlaywright, startServer, login, silence, installSessionPrefix, deezerHost } from "../harness.mjs";
import {
  SCREENS,
  OUT_OF_PASS,
  driftProblems,
  idsInto,
  installObservation,
  walkScreen,
  severity,
  collectBugs,
  writeJson,
  writeMarkdown,
  summaryLine,
  slug,
  REPO,
} from "./lib.mjs";
import { skipBurst, settingsApply, sessionRoundTrip } from "./stress.mjs";

// --- arguments -------------------------------------------------------------
const argv = process.argv.slice(2);
const arg = (name, fallback = "") => {
  const i = argv.indexOf(name);
  return i >= 0 ? argv[i + 1] ?? "" : fallback;
};
const flag = (name) => argv.includes(name);
const OUT = arg("--out", join(REPO, "webapp/test/bughunt/out"));
const ONLY = arg("--only", "");
const MAX_CLICKS = +(arg("--max-clicks", "1200") || 1200);
const MAX_MINUTES = +(arg("--max-minutes", "25") || 25);
const DESKTOP = flag("--desktop");
const SHOTS = !flag("--no-shots");

// --- §8.2 anti-drift guard, before any click -------------------------------
try {
  const problems = driftProblems();
  if (problems.length) {
    for (const p of problems) console.error(`bughunt: ${p}`);
    console.error("bughunt: la table SCREENS ne couvre pas le routeur — aucun clic n'a été fait.");
    process.exit(2);
  }
} catch (e) {
  console.error(`bughunt: ${e.message}`);
  process.exit(2);
}

// --- the bench -------------------------------------------------------------
// A private 0700 directory, not a predictable name in the shared tmp dir
// (CodeQL js/insecure-temporary-file): another user could pre-create or
// symlink `bughunt-<pid>.wav` before the pass writes it.
const wavDir = mkdtempSync(join(tmpdir(), "bughunt-wav-"));
const wavPath = join(wavDir, "silence.wav");
writeFileSync(wavPath, silence(120), { mode: 0o600 });
const { base, ids, stop } = await startServer([
  "--podcasts",
  "--recap",
  "--covers",
  join(REPO, "tests/assets/cover.jpg"),
  "--audio",
  wavPath,
]);
let browser;
let page;
const startedAt = Date.now();

function finish(code) {
  try {
    rmSync(wavDir, { recursive: true, force: true });
  } catch {
    /* already gone */
  }
  stop();
  process.exit(code);
}

try {
  const { chromium } = await loadPlaywright();
  browser = await chromium.launch({
    args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--autoplay-policy=no-user-gesture-required"],
  });
  const context = await browser.newContext(
    DESKTOP
      ? { viewport: { width: 1440, height: 900 } }
      : { viewport: { width: 412, height: 900 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true }
  );
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
  page = await context.newPage();
  const events = await installObservation(page);

  // §4.2 measures a 401 on /api/me in the first seconds of boot: the shell asks
  // before the cookie exists. Login, then a real reload — a hash-only goto
  // would never re-boot the app (measured on the prototype).
  await login(page, base);
  await page.reload();
  await page.waitForSelector('nav a[href="#/"], .mobilenav a[href="#/"], [role=navigation] a[href="#/"]', { timeout: 20000, state: "attached" }).catch(() => {});

  // A 12-track queue from the bench library, so the player and its scenarios
  // have real titles and the archive answers /api/stream (the route above).
  const queue = await page.evaluate(async (big) => {
    const pl = await fetch("/api/playlist/" + big).then((r) => r.json());
    return pl.tracks.slice(0, 12).map((t) => ({
      deezer_id: t.deezer_id,
      title: t.title,
      duration: t.duration || 120,
      artist: t.artist ? { deezer_id: t.artist.deezer_id, name: t.artist.name } : { deezer_id: "a", name: "Artiste" },
      album: t.album ? { deezer_id: t.album.deezer_id, title: t.album.title, cover: t.album.cover } : null,
    }));
  }, ids.big);
  if (queue.length < 12) throw new Error(`bughunt: the bench playlist gave ${queue.length} tracks, 12 needed`);
  await page.evaluate((songs) => {
    sessionStorage.setItem(
      "__inject",
      JSON.stringify({ queue: songs, index: 0, start: 0, currentTime: 0, context: null, shuffle: false, repeat: "all", _orig: null, at: Date.now() })
    );
  }, queue);
  await page.reload();
  await page.waitForSelector('nav a[href="#/"], .mobilenav a[href="#/"], [role=navigation] a[href="#/"]', { timeout: 20000, state: "attached" }).catch(() => {});
  await page.waitForTimeout(800);

  // §8.2: {q} is three words of a real title. The READY line carries ids, not
  // titles; the bench album's first track is a real, searchable title.
  const q = await page.evaluate(async (album) => {
    const a = await fetch("/api/album/" + album).then((r) => r.json());
    return (a.tracks?.[0]?.title || "").split(/\s+/).slice(0, 3).join(" ");
  }, ids.album);
  if (!q) throw new Error("bughunt: the bench album gave no track title for {q}");
  const benchIds = { ...ids, q };

  const head = (() => {
    try {
      return execSync("git rev-parse --short HEAD", { cwd: REPO }).toString().trim();
    } catch {
      return "0000000";
    }
  })();

  // --- the walk ------------------------------------------------------------
  const all = SCREENS.map((s) => ({ ...s, concreteHash: idsInto(s, benchIds) }));
  // --only takes the table's own hash (`#/settings/apparence`) OR its concrete
  // form with the bench ids (`#/album/2000000`), which is what a person copies
  // out of the report.
  const screens = ONLY ? all.filter((s) => s.hash === ONLY || s.concreteHash === ONLY) : all;
  if (ONLY && !screens.length) {
    console.error(`bughunt: unknown screen ${ONLY}`);
    process.exit(2);
  }

  const ctx = {
    dir: OUT,
    base,
    ids: benchIds,
    page,
    events,
    shots: SHOTS,
    maxShots: 40,
    maxScreenshots: 40,
    screenShots: 0,
    anomalyShots: 0,
    screenshotsCapped: false,
    clicks: 0,
    reloads: 0,
    stopped: null,
    shellSeen: new Set(),
    layersSeen: new Set(),
    layersOutOfDepth: [],
    failedScreens: [],
    deadline: Date.now() + MAX_MINUTES * 60000,
    budgetReached() {
      return this.clicks >= MAX_CLICKS || Date.now() > this.deadline;
    },
    counts: { duplicates: 0 },
  };
  mkdirSync(OUT, { recursive: true });

  const rows = [];
  const screenRows = [];
  let elementsFound = 0;
  const t0 = Date.now();
  for (let i = 0; i < screens.length; i++) {
    const screen = screens[i];
    const s0 = Date.now();
    const walked = await walkScreen(page, screen, ctx, i + 1);
    rows.push(...walked.rows);
    elementsFound += walked.elementsFound;
    screenRows.push({ name: screen.name, hash: screen.hash, elements: walked.elementsFound, screenshot: walked.screenshot });
    console.log(
      `bughunt: screen ${String(i + 1).padStart(2, "0")}/${screens.length} ${screen.name} elements=${walked.elementsFound} clicks=${walked.rows.filter((r) => !r.inLayer && !["duplicate", "unreachable", "skipped-session"].includes(r.outcome)).length} ms=${Date.now() - s0}`
    );
    if (ctx.stopped) break;
  }

  // --- the owner's scenarios (§8.6), after the pass ------------------------
  const scenarios = [];
  if (!ONLY) {
    const burst = await skipBurst(page, { ids: benchIds, base, ctx, queue });
    scenarios.push(...(Array.isArray(burst) ? burst : [burst]));
    scenarios.push(await settingsApply(page, ctx));
    scenarios.push(await sessionRoundTrip(page, { base, ids: benchIds, ctx }));
  }

  // --- the report ----------------------------------------------------------
  for (const row of rows) row.severity = severity(row);
  const scenarioRows = [];
  for (const s of scenarios) {
    if (s.verdict !== "fail") continue;
    scenarioRows.push({
      run_id: "",
      screen: { name: "Scénarios", path: "—" },
      element: { role: "scénario", name: s.title, selector: "", via: "aria", signature: slug(s.title, 40), duplicate_of: null },
      action: "scénario",
      outcome: s.outcome || "freeze",
      detail: s.reason || s.numbers,
      ms: s.ms || 0,
      severity: "bloquant",
      bug_id: null,
      evidence: s.evidence || null,
      confirm: null,
    });
  }
  const allRows = [...rows, ...scenarioRows];
  const bugs = collectBugs(allRows, screens);
  const notDuplicate = rows.filter((r) => !r.inLayer && r.outcome !== "duplicate");
  const clicked = notDuplicate.filter((r) => !["unreachable", "skipped-session"].includes(r.outcome)).length;
  const counts = {
    screens: screens.length - (ONLY ? 0 : ctx.failedScreens.length),
    elementsFound,
    clicked,
    duplicates: ctx.counts.duplicates,
    nothing: rows.filter((r) => r.outcome === "nothing").length,
    namedNothing: rows.filter((r) => r.outcome === "nothing" && /^(Lire|Ouvrir|Ajouter|Retirer|Supprimer|Appliquer|Choisir|Enregistrer|Fermer|Chercher|Télécharger|Suivant|Précédent|Lecture|Réduire|Partager|Importer|Créer|Renommer|Déplacer|Vider|Recharger|Se connecter|Se déconnecter|Lancer|Reprendre|Mettre|Délier|Déconnexion|Répéter|Aléatoire|Favori|Paroles|Exporter|Inviter|Copier|Coller|Envoyer|Valider|Confirmer|Annuler|Retour|Trier|Filtrer|Basculer|Activer|Désactiver|Modifier|Éditer|Monter|Descendre)/i.test(r.element.name)).length,
    blank: rows.filter((r) => r.outcome === "blank").length,
    exceptions: rows.filter((r) => r.outcome === "exception").length,
    freezes: rows.filter((r) => r.outcome === "freeze").length,
    unreachable: rows.filter((r) => r.outcome === "unreachable").length,
    seconds: Math.round((Date.now() - startedAt) / 1000),
    stopped: ctx.stopped,
    screenshotsCapped: ctx.screenshotsCapped || ctx.screenShots >= 40 || ctx.anomalyShots >= 40,
    reloads: ctx.reloads,
  };
  const meta = {
    date: new Date().toLocaleString("sv-SE", { timeZone: "Europe/Paris" }).replace(" ", "T") + "+01:00",
    head,
    server: "serveur de banc perf_api — 8 000 titres, 60 playlists, Deezer éteint",
    viewport: DESKTOP ? "1440×900" : "412×900",
    tool: `webapp/test/bughunt/run.mjs@${head}`,
  };
  const report = {
    app: "web",
    run_id: String(process.env.GITHUB_RUN_ID || Date.now()),
    tool: meta.tool,
    meta,
    counts,
    scenarios,
    bugs,
    screens: screenRows,
    outOfPass: OUT_OF_PASS,
    layersOutOfDepth: [...new Set(ctx.layersOutOfDepth)],
    rows: allRows,
  };

  for (const row of allRows) row.run_id = report.run_id;
  const jsonPath = writeJson(OUT, report);
  const md = writeMarkdown(report);
  writeFileSync(join(OUT, "bughunt-web.md"), md);
  writeFileSync(join(OUT, "run.txt"), summaryLine(report) + `\ntool ${meta.tool}\nhead ${head}\n`);
  const consoleLines = [
    ...events.pageerrors.map((e) => `${new Date(e.at).toISOString()} PAGEERROR ${e.msg}`),
    ...events.consoleErrors.map((e) => `${new Date(e.at).toISOString()} CONSOLE ${e.msg}`),
    ...events.httpErrors.map((e) => `${new Date(e.at).toISOString()} HTTP ${e.status} ${e.url}`),
    ...events.failed.map((e) => `${new Date(e.at).toISOString()} FAILED ${e.url} ${e.err}`),
    ...events.dialogs.map((e) => `${new Date(e.at).toISOString()} DIALOG ${e.type} ${e.msg}`),
  ];
  writeFileSync(join(OUT, "console.log"), consoleLines.join("\n") + (consoleLines.length ? "\n" : ""));

  console.log(summaryLine(report));
  console.log(`bughunt: report ${jsonPath}`);
  if (ctx.reloads) console.log(`bughunt: reloads=${ctx.reloads}`);
  await browser.close();
  finish(bugs.some((b) => b.severity === "bloquant") ? 1 : 0);
} catch (e) {
  console.error(`bughunt: tool error — ${e && e.stack ? e.stack : e}`);
  try {
    if (browser) await browser.close();
  } catch {
    /* ignore */
  }
  finish(2);
}
