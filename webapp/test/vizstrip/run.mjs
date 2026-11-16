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
import { existsSync, writeFileSync } from "node:fs";
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

const browserArgs = ["--no-sandbox", "--autoplay-policy=no-user-gesture-required"];
const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM || (existsSync("/usr/bin/chromium") ? "/usr/bin/chromium" : undefined),
  args: browserArgs,
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

// --- the strip's picture: no background, one colour --------------------------
//
// What the person asked for, measured where it is true: on the CANVAS for the
// colour and the coverage (one solid theme accent on every capsule, alpha 0
// everywhere else — including the top and bottom rows, which the centred
// capsules cannot reach), and on the COMPOSITED strip for "the backdrop is
// exactly what shows outside the bars" and the contrast the eye actually gets.

const THEME_ACCENT = { encre: "#d2ff3c", papier: "#14130f", pochette: "#f4f1f5" };

function hex(rgb) {
  return "#" + rgb.map((v) => Math.max(0, Math.min(255, Math.round(v))).toString(16).padStart(2, "0")).join("");
}

/** The visible strip host's rectangle, in CSS px. */
function stripRect(page) {
  return page.evaluate(() => {
    const host = document.querySelector(".d .viz, .m .viz");
    if (!host || getComputedStyle(host).display === "none") return null;
    const r = host.getBoundingClientRect();
    return { x: r.x, y: r.y, w: r.width, h: r.height };
  });
}

/** Two screenshots of the same strip: as painted, and with the host hidden. */
async function shotPair(page, rect) {
  const clip = {
    x: Math.round(rect.x),
    y: Math.round(rect.y),
    width: Math.round(rect.w),
    height: Math.round(rect.h),
  };
  const shown = await page.screenshot({ clip });
  await page.evaluate(() => {
    const host = document.querySelector(".d .viz, .m .viz");
    if (host) host.style.visibility = "hidden";
  });
  await page.waitForTimeout(140);
  const hidden = await page.screenshot({ clip });
  await page.evaluate(() => {
    const host = document.querySelector(".d .viz, .m .viz");
    if (host) host.style.visibility = "";
  });
  await page.waitForTimeout(140);
  return { shown, hidden };
}

/** Diff + capsule colour + contrast, computed in the page (no image library). */
function compareShots(page, shown, hidden) {
  return page.evaluate(
    async ({ a, b }) => {
      const dec = async (b64) => {
        const img = new Image();
        await new Promise((res, rej) => {
          img.onload = res;
          img.onerror = rej;
          img.src = "data:image/png;base64," + b64;
        });
        const c = document.createElement("canvas");
        c.width = img.width;
        c.height = img.height;
        const g = c.getContext("2d", { willReadFrequently: true });
        g.drawImage(img, 0, 0);
        return { w: c.width, h: c.height, d: g.getImageData(0, 0, c.width, c.height).data };
      };
      const A = await dec(a);
      const B = await dec(b);
      const w = A.w;
      const h = A.h;
      const lin = (v) => {
        v /= 255;
        return v <= 0.04045 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
      };
      const lum = ([r, g, bb]) => 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(bb);
      const contrast = (x, y) => {
        const la = lum(x);
        const lb = lum(y);
        return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
      };
      let changed = 0;
      let edgeChanged = 0;
      let edgeN = 0;
      let edgeMax = 0;
      const cap = [0, 0, 0];
      let capN = 0;
      const back = [0, 0, 0];
      let backN = 0;
      for (let y = 0; y < h; y++) {
        for (let x = 0; x < w; x++) {
          const i = (y * w + x) * 4;
          const dr = Math.abs(A.d[i] - B.d[i]);
          const dg = Math.abs(A.d[i + 1] - B.d[i + 1]);
          const db = Math.abs(A.d[i + 2] - B.d[i + 2]);
          const d = Math.max(dr, dg, db);
          const edge = y < 2 || y >= h - 2;
          if (edge) {
            edgeN++;
            if (d > 2) edgeChanged++;
            if (d > edgeMax) edgeMax = d;
          }
          if (d > 2) changed++;
          // The capsules' own pixels: well away from the anti-aliased edge, so
          // the blend with the backdrop (the host is at 90% opacity) does not
          // smear the colour being read.
          if (d > 40) {
            cap[0] += A.d[i];
            cap[1] += A.d[i + 1];
            cap[2] += A.d[i + 2];
            back[0] += B.d[i];
            back[1] += B.d[i + 1];
            back[2] += B.d[i + 2];
            capN++;
            backN++;
          }
        }
      }
      const capMean = cap.map((v) => v / Math.max(1, capN));
      const backMean = back.map((v) => v / Math.max(1, backN));
      return {
        w,
        h,
        changed: changed / (w * h),
        edgeChanged: edgeN ? edgeChanged / edgeN : 0,
        edgeMax,
        capsule: capMean,
        backdrop: backMean,
        capsuleN: capN,
        contrast: capN ? contrast(capMean, backMean) : 0,
      };
    },
    { a: shown.toString("base64"), b: hidden.toString("base64") }
  );
}

/**
 * The canvas' own pixels: alpha coverage outside the capsules, and the exact
 * colour of their interior. WebGL is read after the scene's own frame (the
 * default framebuffer is cleared when the compositor takes it), a 2D canvas
 * with getImageData.
 */
function canvasStats(page) {
  return page.evaluate(async () => {
    const host = document.querySelector(".d .viz, .m .viz");
    const canv = host && [...host.querySelectorAll("canvas")].find((c) => getComputedStyle(c).display !== "none");
    if (!canv) return null;
    const fold = (px, w, h, rgba) => {
      let nz = 0;
      let edgeNz = 0;
      let edgeN = 0;
      const colours = new Map();
      let lo = [255, 255, 255];
      let hi = [0, 0, 0];
      for (let y = 0; y < h; y++) {
        for (let x = 0; x < w; x++) {
          const i = (y * w + x) * 4;
          const a = px[i + 3];
          if (a > 0) nz++;
          if (y < 2 || y >= h - 2) {
            edgeN++;
            if (a > 0) edgeNz++;
          }
          // Full coverage only: one or two code values under it is the
          // canvas' un-premultiply rounding, not another colour (measured: the
          // row read a 4-5/255 spread with the threshold at 250, which is the
          // anti-aliased rim).
          if (a >= 254) {
            const k = `${px[i]},${px[i + 1]},${px[i + 2]}`;
            colours.set(k, (colours.get(k) || 0) + 1);
            for (let c = 0; c < 3; c++) {
              if (px[i + c] < lo[c]) lo[c] = px[i + c];
              if (px[i + c] > hi[c]) hi[c] = px[i + c];
            }
          }
        }
      }
      const top = [...colours.entries()].sort((a, b) => b[1] - a[1]).slice(0, 3);
      let mode = null;
      let best = 0;
      for (const [k, n] of colours) if (n > best) { best = n; mode = k; }
      // The row's geometry, on the line through its axis: the beads are the
      // runs of coverage, so their count and their widths are measured, not
      // assumed.
      const row = h >> 1;
      const widths = [];
      let start = -1;
      for (let x = 0; x < w; x++) {
        const a = px[(row * w + x) * 4 + 3];
        if (a > 0 && start < 0) start = x;
        else if (a === 0 && start >= 0) {
          widths.push(x - start);
          start = -1;
        }
      }
      if (start >= 0) widths.push(w - start);
      widths.sort((a, b) => a - b);
      return {
        w, h, rgba,
        nonZero: nz / (w * h),
        edge: edgeN ? edgeNz / edgeN : 0,
        colours: colours.size,
        spread: Math.max(hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]),
        top,
        mode: mode ? mode.split(",").map(Number) : null,
        covered: best / (w * h),
        beads: widths.length,
        beadW: widths.length ? [widths[0], widths[widths.length >> 1], widths[widths.length - 1]] : null,
      };
    };
    const gl = canv.getContext("webgl2", { alpha: true, premultipliedAlpha: true });
    if (gl) {
      // The default framebuffer is cleared once the compositor takes it, and
      // the host's frame cap may skip a rAF: read in the frame the scene drew
      // (and retry a few frames rather than reading an empty buffer as "no
      // background"). 20 frames is a third of a second.
      return await new Promise((res) => {
        let tries = 0;
        const tick = () => {
          const w = canv.width;
          const h = canv.height;
          const px = new Uint8Array(w * h * 4);
          gl.bindFramebuffer(gl.FRAMEBUFFER, null);
          gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, px);
          const stats = fold(px, w, h, true);
          if (stats.nonZero > 0 || ++tries > 20) res(stats);
          else requestAnimationFrame(tick);
        };
        requestAnimationFrame(tick);
      });
    }
    const g = canv.getContext("2d");
    const d = g.getImageData(0, 0, canv.width, canv.height).data;
    return fold(d, canv.width, canv.height, false);
  });
}

/**
 * The whole strip claim for the theme on screen: canvas alpha 0 outside the
 * capsules, one exact colour inside, the backdrop untouched between them, and
 * the contrast the eye gets. `accent` is the colour --ac-text resolves to.
 */
async function assertStripColour(page, label, accent) {
  const rect = await stripRect(page);
  check(!!rect, `${label} : la bande est dessinée`, "");
  if (!rect) return null;
  const cs = await canvasStats(page);
  check(!!cs, `${label} : le canvas est lisible`, "");
  if (!cs) return null;
  const edge = (cs.edge * 100).toFixed(1);
  check(cs.nonZero < 0.6, `${label} : le canvas est transparent hors capsules (${(cs.nonZero * 100).toFixed(1)} % couverts)`, "");
  check(cs.edge === 0, `${label} : aucune alpha sur les rangées du haut et du bas (${edge} %)`, "");
  check(
    cs.spread <= 3,
    `${label} : une seule couleur sur les capsules (écart ${cs.spread}/255, ${cs.colours} valeurs de bord)`,
    cs.top.map(([k]) => k).join(" ")
  );
  check(
    !!cs.mode && hex(cs.mode) === accent,
    `${label} : la couleur est l'accent du thème ${accent} (${cs.mode ? hex(cs.mode) : "—"})`,
    ""
  );
  check(
    cs.beads >= 18 && cs.beads <= 40 && !!cs.beadW,
    `${label} : ${cs.beads} capsules sur l'axe, ${cs.beadW?.[0]}-${cs.beadW?.[2]} px de large (médiane ${cs.beadW?.[1]})`,
    ""
  );
  const { shown, hidden } = await shotPair(page, rect);
  if (SHOTS) {
    const slug = label.replace(/[^a-z0-9]+/gi, "-").toLowerCase();
    writeFileSync(`${SHOTS}/strip-${slug}.png`, shown);
    writeFileSync(`${SHOTS}/strip-${slug}-backdrop.png`, hidden);
  }
  const cmp = await compareShots(page, shown, hidden);
  check(cmp.edgeMax <= 1, `${label} : le fond est identique au backdrop hors capsules (écart max ${cmp.edgeMax}/255)`, "");
  check(cmp.changed < 0.6, `${label} : rien d'autre que les barres n'est peint (${(cmp.changed * 100).toFixed(1)} %)`, "");
  check(
    cmp.contrast >= 3,
    `${label} : contraste ${cmp.contrast.toFixed(2)}:1 sur le scrim (barres ${hex(cmp.capsule)} sur ${hex(cmp.backdrop)})`,
    ""
  );
  say(
    `${label} : ${cs.w}x${cs.h}, alpha hors capsules ${(cs.nonZero * 100).toFixed(1)} % (bords ${edge} %), ` +
      `couleur ${cs.mode ? hex(cs.mode) : "—"} (écart ${cs.spread}/255, ${cs.colours} valeurs), ` +
      `${cs.beads} capsules de ${cs.beadW?.[0]}-${cs.beadW?.[2]} px, ` +
      `contraste ${cmp.contrast.toFixed(2)}:1, fond identique ${(100 - cmp.changed * 100).toFixed(1)} %`
  );
  return { cs, cmp };
}

/** Put the app on a theme (the attribute lib/theme.js paints from). */
async function setTheme(page, name) {
  await page.evaluate((t) => {
    document.documentElement.dataset.theme = t;
  }, name);
  await page.waitForTimeout(700);
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
  await ctx.route((u) => u.hostname === "api.deezer.com" || u.hostname === "dzcdn.net" || u.hostname.endsWith(".dzcdn.net"), (r) => r.abort());
  await ctx.route(
    (u) => u.pathname.startsWith("/api/stream/"),
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

  // THE PICTURE: zero background, one solid colour from the theme. Measured on
  // the canvas (coverage + exact colour) and on the composited strip (backdrop
  // identity + contrast with the eye's own pixels), on all three themes.
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.waitForTimeout(450);
  for (const theme of ["encre", "papier", "pochette"]) {
    await setTheme(page, theme);
    await assertStripColour(page, `${theme} 1280x900`, THEME_ACCENT[theme]);
    if (SHOTS) await page.locator(".d").screenshot({ path: `${SHOTS}/player-${theme}-1280x900.png` });
  }

  // A CUSTOM ACCENT is written INLINE on <html> by lib/theme.js, so the strip
  // has to watch the style attribute as well as data-theme: switching it must
  // repaint the live scene, not wait for a reload.
  await setTheme(page, "encre");
  await page.evaluate(() => document.documentElement.style.setProperty("--ac-text", "#00e5ff"));
  await page.waitForTimeout(500);
  const custom = await canvasStats(page);
  check(
    !!custom && !!custom.mode && hex(custom.mode) === "#00e5ff" && custom.spread <= 3,
    "accent personnalisé : la bande suit l'inline --ac-text sans rechargement",
    custom?.mode ? `${hex(custom.mode)} (écart ${custom.spread})` : "—"
  );
  await page.evaluate(() => document.documentElement.style.removeProperty("--ac-text"));
  await page.waitForTimeout(300);

  // ECO. The app's rule is "no canvas anywhere", so in eco the strip is not
  // drawn at all — the strongest transparency there is. (The scene's own eco
  // grade, for the paths that can still draw one, is photographed by
  // `node test/render/run.mjs --strip --eco`.)
  await page.locator('button[aria-label="Mode éco (aucune animation)"]').first().click();
  await page.waitForTimeout(500);
  const ecoGone = await page.evaluate(() => !document.querySelector(".d .viz"));
  check(ecoGone, "éco : la bande n'est plus dessinée du tout", "");
  await page.locator('button[aria-label="Mode éco (aucune animation)"]').first().click();
  await page.waitForTimeout(500);
  const ecoBack = await page.evaluate(() => !!document.querySelector(".d .viz"));
  check(ecoBack, "éco désactivé : la bande revient", "");
  await ctx.close();
}

// The OS setting: prefers-reduced-motion still draws the same row, in the same
// one colour, with the same transparent background (the breath holds still).
{
  const ctx = await browser.newContext({
    viewport: { width: 1280, height: 900 },
    reducedMotion: "reduce",
  });
  await ctx.route((u) => u.hostname === "api.deezer.com" || u.hostname === "dzcdn.net" || u.hostname.endsWith(".dzcdn.net"), (r) => r.abort());
  await ctx.route(
    (u) => u.pathname.startsWith("/api/stream/"),
    (r) => r.fulfill({ status: 200, body: WAV, headers: { "Content-Type": "audio/wav", "Accept-Ranges": "none" } })
  );
  const page = await ctx.newPage();
  await open(page);
  await assertStripColour(page, "reduced-motion 1280x900", THEME_ACCENT.encre);
  await setTheme(page, "papier");
  await assertStripColour(page, "reduced-motion papier", THEME_ACCENT.papier);
  await ctx.close();
}

// NO WEBGL2: the strip is the canvas fallback. Its old veil was the host's own
// post pass — a bloom canvas blended over the page and a grain div over the
// whole strip — so beyond the pixels, that pass must not exist for this layout.
{
  const noGl = await chromium.launch({
    executablePath: process.env.CHROMIUM || (existsSync("/usr/bin/chromium") ? "/usr/bin/chromium" : undefined),
    args: [...browserArgs, "--disable-webgl", "--disable-webgl2"],
  });
  const ctx = await noGl.newContext({ viewport: { width: 1280, height: 900 } });
  await ctx.route((u) => u.hostname === "api.deezer.com" || u.hostname === "dzcdn.net" || u.hostname.endsWith(".dzcdn.net"), (r) => r.abort());
  await ctx.route(
    (u) => u.pathname.startsWith("/api/stream/"),
    (r) => r.fulfill({ status: 200, body: WAV, headers: { "Content-Type": "audio/wav", "Accept-Ranges": "none" } })
  );
  const page = await ctx.newPage();
  await open(page);
  const is2d = await page.evaluate(() => {
    const host = document.querySelector(".d .viz");
    const c = host && [...host.querySelectorAll("canvas")].find((x) => getComputedStyle(x).display !== "none");
    return !!c && !c.getContext("webgl2");
  });
  check(is2d, "sans WebGL2 : la bande est le canvas de secours", "");
  await assertStripColour(page, "sans WebGL2 encre", THEME_ACCENT.encre);
  const post = await page.evaluate(() => {
    const host = document.querySelector(".d .viz");
    const bloom = host ? [...host.querySelectorAll("canvas")].filter((c) => c.classList.contains("bloom")) : [];
    return { grain: !!host?.querySelector(".grain"), bloom: bloom.length, off: bloom.every((c) => c.classList.contains("off")) };
  });
  check(!post.grain, "sans WebGL2 : aucun grain au-dessus de la bande", JSON.stringify(post));
  check(post.off, "sans WebGL2 : le bloom du host est éteint", JSON.stringify(post));
  await setTheme(page, "papier");
  await assertStripColour(page, "sans WebGL2 papier", THEME_ACCENT.papier);
  await ctx.close();
  await noGl.close();
}

// The Flow's steering row takes the band first: the taller strip waits longer.
{
  const ctx = await browser.newContext({ viewport: { width: 1920, height: 1080 } });
  await ctx.route((u) => u.hostname === "api.deezer.com" || u.hostname === "dzcdn.net" || u.hostname.endsWith(".dzcdn.net"), (r) => r.abort());
  await ctx.route(
    (u) => u.pathname.startsWith("/api/stream/"),
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
  await ctx.route((u) => u.hostname === "api.deezer.com" || u.hostname === "dzcdn.net" || u.hostname.endsWith(".dzcdn.net"), (r) => r.abort());
  await ctx.route(
    (u) => u.pathname.startsWith("/api/stream/"),
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
  await assertStripColour(page, "encre 390x844", THEME_ACCENT.encre);
  await setTheme(page, "papier");
  await assertStripColour(page, "papier 390x844", THEME_ACCENT.papier);
  await setTheme(page, "encre");
  if (SHOTS) await page.locator(".m").screenshot({ path: `${SHOTS}/vizstrip-390x844.png` });
  await ctx.close();
}

await browser.close();
stop();
console.log(state.failed ? `\n${state.failed} check(s) failed` : "\nall viz strip checks passed");
process.exit(state.failed ? 1 : 0);
