#!/usr/bin/env node
// The desktop player's bar strip, at the height the column can pay for.
//
//   npx vite build && node test/vizstrip/run.mjs
//   DIST=/tmp/some-build node test/vizstrip/run.mjs
//
// The strip is 44 px where the column is already at its budget (a short window,
// the narrow side-panel layout), 56 px from 880 px of window height without a
// Flow and from 1060 with one. What must hold everywhere: the canvas fills the
// host (the visualizer sizes itself from the parent), the strip sits between
// the transport and the footer, the footer stays on the stage's padding, and
// nothing overflows. 641 × 900 is the narrow side-panel layout: the strip is
// not drawn there, and must stay that way.
import { existsSync } from "node:fs";
import { loadPlaywright, startServer, login, checker } from "../harness.mjs";

const { chromium } = await loadPlaywright();
const { base, ids, stop } = await startServer();
const { check, state } = checker();
const SHOTS = process.env.SHOTS;
const say = (s) => console.log(s);

const WAV = (() => {
  const rate = 8000;
  const n = rate * 120;
  const h = Buffer.alloc(44);
  h.write("RIFF", 0);
  h.writeUInt32LE(36 + n, 4);
  h.write("WAVEfmt ", 8);
  h.writeUInt32LE(16, 16);
  h.writeUInt16LE(1, 20);
  h.writeUInt16LE(1, 22);
  h.writeUInt32LE(rate, 24);
  h.writeUInt32LE(rate, 28);
  h.writeUInt16LE(1, 32);
  h.writeUInt16LE(8, 34);
  h.write("data", 36);
  h.writeUInt32LE(n, 40);
  return Buffer.concat([h, Buffer.alloc(n, 128)]);
})();

const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM || (existsSync("/usr/bin/chromium") ? "/usr/bin/chromium" : undefined),
  args: ["--no-sandbox", "--autoplay-policy=no-user-gesture-required"],
});

const measure = () => {
  const root = document.querySelector(".d") || document.querySelector(".m");
  if (!root) return null;
  const box = (s) => {
    const e = root.querySelector(s);
    if (!e) return null;
    if (getComputedStyle(e).display === "none") return { display: "none" };
    const r = e.getBoundingClientRect();
    return { top: +r.top.toFixed(1), bottom: +r.bottom.toFixed(1), h: +r.height.toFixed(1), w: +r.width.toFixed(1) };
  };
  const host = root.querySelector(".viz");
  const canvas = host ? [...host.querySelectorAll("canvas")].find((c) => getComputedStyle(c).display !== "none") : null;
  const main = root.querySelector(".main") || root.querySelector(".mid");
  const stage = root.querySelector(".stage");
  const sr = stage ? stage.getBoundingClientRect() : null;
  return {
    desktop: !!document.querySelector(".d"),
    vw: innerWidth,
    vh: innerHeight,
    mainW: main ? +main.getBoundingClientRect().width.toFixed(1) : null,
    mainOverflow: main ? main.scrollHeight - main.clientHeight : 0,
    stageBottom: sr ? +sr.bottom.toFixed(1) : null,
    viz: box(".viz"),
    canvas: canvas
      ? {
          css: [+canvas.getBoundingClientRect().width.toFixed(1), +canvas.getBoundingClientRect().height.toFixed(1)],
          buffer: [canvas.width, canvas.height],
        }
      : null,
    controls: box(".controls"),
    seek: box(".seek"),
    steer: box(".flow-steer"),
    footer: box(".footer"),
    docOverflow: document.documentElement.scrollWidth - document.documentElement.clientWidth,
  };
};

function assertFrame(label, m, want) {
  check(!!m, `${label} : le lecteur est monté`, "");
  check(m.viz && m.viz.display !== "none", `${label} : la barre du visualiseur est dessinée`, m.viz?.display);
  if (!m.viz || m.viz.display === "none") return;
  check(Math.abs(m.viz.h - want) < 0.6, `${label} : barre de ${want} px (${m.viz.h} mesurés)`, "");
  check(
    !!m.canvas && Math.abs(m.canvas.css[1] - m.viz.h) < 0.6 && Math.abs(m.canvas.css[0] - m.viz.w) < 0.6,
    `${label} : le canvas remplit la barre (${m.canvas?.css})`,
    ""
  );
  check(
    !!m.controls && m.viz.top >= m.controls.bottom - 0.5,
    `${label} : la barre est sous la transport`,
    `transport bas ${m.controls?.bottom}, barre haut ${m.viz.top}`
  );
  check(
    !!m.footer && m.viz.bottom <= m.footer.top + 0.5,
    `${label} : la barre ne touche pas le pied (volume/outils)`,
    `barre bas ${m.viz.bottom}, pied haut ${m.footer?.top}`
  );
  check(m.mainOverflow === 0, `${label} : la colonne ne déborde pas (${m.mainOverflow} px)`, "");
  check(m.docOverflow === 0, `${label} : aucun débordement de page`, `${m.docOverflow}`);
}

async function open(page, { playlist = ids.big, flow = false } = {}) {
  page.on("pageerror", (e) => say("pageerror: " + e.message));
  await page.goto(base + "/app/");
  await login(page, base);
  await page.goto(base + "/app/");
  await page.waitForTimeout(700);
  if (flow) {
    await page.goto(base + "/app/#/");
    await page.waitForTimeout(900);
    await page.locator('button:has-text("Lancer mon Flow")').click();
    await page.waitForTimeout(1200);
    await page.locator('button[aria-label="Plein écran"]').first().click();
    await page.waitForSelector(".d .flow-steer", { timeout: 20000 });
  } else {
    await page.goto(base + "/app/#/playlist/" + playlist);
    await page.waitForSelector(".row.track", { timeout: 60000 });
    await page.waitForTimeout(400);
    await page.locator(".row.track .titles").first().click();
    await page.waitForTimeout(1000);
    await page.locator('button[aria-label="Plein écran"]').first().click();
    await page.waitForSelector(".d .controls .pp", { timeout: 20000 });
  }
  await page.waitForTimeout(900);
}

// One ordinary player: the height query, the budget, the narrow layout.
{
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  await ctx.route(/dzcdn\.net|api\.deezer\.com/, (r) => r.abort());
  await ctx.route(
    /\/api\/stream\//,
    (r) => r.fulfill({ status: 200, body: WAV, headers: { "Content-Type": "audio/wav", "Accept-Ranges": "none" } })
  );
  const page = await ctx.newPage();
  await open(page);

  for (const [w, h, want] of [
    [1280, 900, 56],
    [1440, 900, 56],
    [1920, 1080, 56],
    [1280, 880, 56],
    [1280, 870, 44],
  ]) {
    await page.setViewportSize({ width: w, height: h });
    await page.waitForTimeout(350);
    const m = await page.evaluate(measure);
    assertFrame(`${w}x${h}`, m, want);
    check(
      !!m.footer && m.footer.bottom <= (m.stageBottom ?? h) + 0.5,
      `${w}x${h} : le pied reste dans la scène`,
      `pied bas ${m.footer?.bottom}, scène ${m.stageBottom}`
    );
    if (want === 56) {
      check(
        Math.abs(m.footer.bottom - ((m.stageBottom ?? h) - 24)) < 1,
        `${w}x${h} : les 12 px sortent de la bande libre, pas du pied`,
        `pied bas ${m.footer.bottom}`
      );
    }
    say(`${w}x${h} : barre ${m.viz?.h} px, colonne ${m.mainW} px, pied ${m.footer?.top}–${m.footer?.bottom}`);
    if (SHOTS && w === 1280 && h === 900) await page.locator(".d .main").screenshot({ path: `${SHOTS}/vizstrip-1280x900.png` });
  }

  // The narrow side-panel layout: the strip is not drawn at all.
  await page.setViewportSize({ width: 641, height: 900 });
  await page.waitForTimeout(350);
  const narrow = await page.evaluate(measure);
  check(narrow.viz?.display === "none", "641x900 : la barre n'est pas dessinée (colonne étroite)", narrow.viz?.display);
  check(narrow.mainOverflow === 0, "641x900 : la colonne ne déborde pas", `${narrow.mainOverflow}`);
  check(narrow.docOverflow === 0, "641x900 : aucun débordement", `${narrow.docOverflow}`);
  check(
    !!narrow.footer && narrow.footer.bottom <= (narrow.stageBottom ?? 900) + 0.5,
    "641x900 : le pied reste dans la scène",
    ""
  );
  if (SHOTS) await page.locator(".d").screenshot({ path: `${SHOTS}/vizstrip-641x900.png` });
  await ctx.close();
}

// The Flow's steering row takes the band first: the taller strip waits longer.
{
  const ctx = await browser.newContext({ viewport: { width: 1920, height: 1080 } });
  await ctx.route(/dzcdn\.net|api\.deezer\.com/, (r) => r.abort());
  await ctx.route(
    /\/api\/stream\//,
    (r) => r.fulfill({ status: 200, body: WAV, headers: { "Content-Type": "audio/wav", "Accept-Ranges": "none" } })
  );
  const page = await ctx.newPage();
  const pool = await (async () => {
    await page.goto(base + "/app/");
    await login(page, base);
    await page.goto(base + "/app/");
    await page.waitForTimeout(500);
    return page.evaluate(async (id) => (await fetch("/api/playlist/" + id)).json(), ids.big);
  })();
  const tracks = pool.tracks.slice(0, 60);
  let k = 0;
  await ctx.route(/\/api\/flow\/next/, (r) => {
    const n = JSON.parse(r.request().postData() || "{}").n || 1;
    r.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ tracks: Array.from({ length: n }, () => tracks[k++ % tracks.length]) }),
    });
  });
  await page.close();
  const flowPage = await ctx.newPage();
  await open(flowPage, { flow: true });
  for (const [w, h, want] of [
    [1440, 1040, 44],
    [1440, 1060, 56],
    [1920, 1080, 56],
  ]) {
    await flowPage.setViewportSize({ width: w, height: h });
    await flowPage.waitForTimeout(350);
    const m = await flowPage.evaluate(measure);
    assertFrame(`Flow ${w}x${h}`, m, want);
    check(
      !!m.footer && m.footer.bottom <= h,
      `Flow ${w}x${h} : le pied reste à l'écran`,
      `pied bas ${m.footer?.bottom} / ${h}`
    );
    say(`Flow ${w}x${h} : barre ${m.viz?.h} px, pied ${m.footer?.top}–${m.footer?.bottom}`);
  }
  await ctx.close();
}

// The phone: the desktop player is not mounted, and its own 40 px strip keeps
// its place between the transport and the thumb row.
{
  const ctx = await browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
  await ctx.route(/dzcdn\.net|api\.deezer\.com/, (r) => r.abort());
  await ctx.route(
    /\/api\/stream\//,
    (r) => r.fulfill({ status: 200, body: WAV, headers: { "Content-Type": "audio/wav", "Accept-Ranges": "none" } })
  );
  const page = await ctx.newPage();
  await page.goto(base + "/app/");
  await login(page, base);
  await page.goto(base + "/app/");
  await page.waitForTimeout(500);
  await page.goto(base + "/app/#/playlist/" + ids.big);
  await page.waitForSelector(".row.track", { timeout: 60000 });
  await page.waitForTimeout(400);
  await page.locator(".row.track .titles").first().click();
  await page.waitForTimeout(900);
  await page.locator("footer.player").click({ position: { x: 4, y: 4 } });
  await page.locator(".m").waitFor({ state: "visible", timeout: 20000 });
  await page.waitForTimeout(900);
  const m = await page.evaluate(measure);
  check(!m.desktop, "390x844 : le lecteur desktop n'est pas monté", "");
  check(!!m.viz && m.viz.display !== "none" && Math.abs(m.viz.h - 40) < 0.6, "390x844 : barre mobile de 40 px", `${m.viz?.h}`);
  check(
    !!m.footer && m.viz.bottom <= m.footer.top + 0.5,
    "390x844 : la barre ne touche pas la rangée de pouces",
    `barre bas ${m.viz?.bottom}, pied haut ${m.footer?.top}`
  );
  check(m.docOverflow === 0, "390x844 : aucun débordement", `${m.docOverflow}`);
  if (SHOTS) await page.locator(".m").screenshot({ path: `${SHOTS}/vizstrip-390x844.png` });
  await ctx.close();
}

await browser.close();
stop();
console.log(state.failed ? `\n${state.failed} check(s) failed` : "\nall viz strip checks passed");
process.exit(state.failed ? 1 : 0);
