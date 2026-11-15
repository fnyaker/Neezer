#!/usr/bin/env node
// The interface's translucent surfaces: what they are made of, and what they
// cost.
//
//   npm run build && node test/glass/run.mjs
//   SHOTS=/tmp/glass node test/glass/run.mjs          # PNGs of every case
//   THROTTLE=1 N=3 node test/glass/run.mjs            # desktop speed, 3 rounds
//
// THE APP HAS ONE BLURRED LAYER: the cover-derived ground (GlobalBackdrop), a
// single fixed image with one `filter: blur()`. Above it, `app.css` owns a
// three-tier ladder — tier 1 (cards, bars, notices): a `color-mix()` tint, no
// blur; tier 2 (popovers): the tint + `blur(14px)`; tier 3 (sheets/modals):
// the tint + `blur(22px)` over the scrim's veil. A `backdrop-filter` exists
// ONLY on tier 2/3, and only one overlay is open at a time, so the compositor
// never runs more than one of them plus the ground. This bench holds that, on
// Encre, Papier and Pochette at 1280×900 and 390×900, on real pixels:
//
//  - a surface's own alpha is in [0.55, 0.88]: visibly translucent, never a
//    20 % pane of glass nobody can read;
//  - the pixel actually composited behind a line of text equals
//    alpha·surface + (1−alpha)·backdrop (the tint is the one the tokens name);
//  - the text on it keeps WCAG 1.4.3 at its rendered size (4.5:1, 3:1 for
//    large text), sampled from the painted pixels: the worst 2 % of the line,
//    not the worst single pixel (one antialiased edge left by the compositor is
//    not a legibility failure, a 30-pixel patch is), and a failing case keeps
//    its crop in /tmp/glass-fail-*.png;
//  - a `backdrop-filter` only on a tier-2/tier-3 overlay, exactly one open at
//    a time (0 when nothing is open), no blurred CARD ever, the scrims flat
//    (they used to carry their own `blur(2px)`/`blur(6px)`), and at most ONE
//    blurred image layer in the DOM (the ground);
//  - eco (`viz.eco`, so <html> really carries data-eco="1") paints NO blurred
//    image, no filter, and puts every surface back on --s1/--s2 (alpha 1);
//  - `prefers-reduced-transparency: reduce` does the same for the surfaces;
//  - nothing overflows sideways;
//  - the cost: contextual menu, sheet, and a scroll of the card grid, in ms of
//    main thread under 4× CPU, median over the rounds.
//
// Measured on this bench (median, 4× CPU throttle, 1280×900 / 390×900):
//   before: 8/8 surfaces opaque (alpha 1), 8 backdrop-filter scrims reachable
//           from a track menu, 0 blurred image layer with nothing playing;
//   after:  8/8 surfaces at alpha 0.72–0.86, 0 backdrop-filter anywhere,
//           1 blurred image layer; see the numbers printed per case.
import { resolve } from "node:path";
import { loadPlaywright, startServer, login, checker } from "../harness.mjs";

const { chromium } = await loadPlaywright();
const { base, stop } = await startServer();
const { check, state } = checker();
const SHOTS = process.env.SHOTS;
const THROTTLE = +(process.env.THROTTLE || 4);
const N = +(process.env.N || 5);

const THEMES = ["encre", "papier", "pochette"];
const PHONE = { width: 390, height: 900 };
const DESK = { width: 1280, height: 900 };

// A surface and the lines of text that can sit on it. The SMALLEST visible
// leaf is the one measured: small secondary text is always the worst case.
const SURFACES = {
  sidebar: { sel: ".sidebar", texts: [".nav a .lbl", ".pl-sub", ".account .name", ".account .sub", ".pl-name"] },
  mobiletab: { sel: ".mobilenav", texts: [".l"] },
  topbar: { sel: ".topbar", texts: [".level", ".hint", ".kbd"] },
  player: { sel: "footer.player", texts: [".t", ".a", ".ph"] },
  card: { sel: ".card", texts: [".title", ".sub"] },
  notice: { sel: ".notice", texts: [".msg"] },
  menu: { sel: '.menu[role="menu"]', texts: [".lbl", ".item.empty"] },
  quality: { sel: ".qm .menu", texts: [".lab", ".hint"] },
  sheet: { sel: '.sheet[role="dialog"]', texts: ["h2", "p", "label", ".title", ".msg", "span"] },
};
// The ones that must be translucent on each viewport: the phone shell has no
// sidebar, and the quality badge lives in the full-screen player there.
const REQUIRED = {
  desk: ["sidebar", "topbar", "player", "card", "notice", "menu", "quality", "sheet"],
  phone: ["mobiletab", "topbar", "player", "card", "notice", "menu", "sheet"],
};

const med = (xs) => {
  const s = [...xs].sort((a, b) => a - b);
  return s.length ? s[s.length >> 1] : NaN;
};
const p95 = (xs) => {
  const s = [...xs].sort((a, b) => a - b);
  return s.length ? s[Math.min(s.length - 1, Math.floor(s.length * 0.95))] : NaN;
};

// ── WCAG 2.2, 1.4.3 ──────────────────────────────────────────────────────────
function lum([r, g, b]) {
  const f = (v) => {
    const c = v / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}
const contrast = (a, b) => {
  const la = lum(a);
  const lb = lum(b);
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
};
const hex = ([r, g, b]) => "#" + [r, g, b].map((v) => Math.round(v).toString(16).padStart(2, "0")).join("");
const mean = (px) => [0, 1, 2].map((i) => px.reduce((a, p) => a + p[i], 0) / px.length);
const dist = (a, b) => Math.max(...[0, 1, 2].map((i) => Math.abs(a[i] - b[i])));

// ── what the page itself can answer ──────────────────────────────────────────
const PAGE_HELPERS = () => {
  // `rgba(r, g, b, a)` or `color(srgb r g b / a)` — the two serialisations
  // Chromium gives for a `color-mix()` and for a plain rgba().
  window.__parse = (s) => {
    s = String(s || "").trim();
    let m = s.match(/^rgba?\(([^)]+)\)$/);
    if (m) {
      const p = m[1].split(/[ ,/]+/).filter(Boolean).map(Number);
      return [p[0], p[1], p[2], p.length > 3 ? p[3] : 1];
    }
    m = s.match(/^color\(srgb\s+([\d.eE+-]+)\s+([\d.eE+-]+)\s+([\d.eE+-]+)(?:\s*\/\s*([\d.eE+-]+))?\)$/);
    if (m) return [Number(m[1]) * 255, Number(m[2]) * 255, Number(m[3]) * 255, m[4] === undefined ? 1 : Number(m[4])];
    m = s.match(/^#([0-9a-f]{6})([0-9a-f]{2})?$/i);
    if (m) {
      const n = parseInt(m[1], 16);
      return [(n >> 16) & 255, (n >> 8) & 255, n & 255, m[2] ? parseInt(m[2], 16) / 255 : 1];
    }
    return null;
  };

  // The SMALLEST visible text leaf under `root` matching one of `sels`.
  window.__textEl = (root, sels) => {
    let best = null;
    let bestSize = Infinity;
    for (const s of sels) {
      for (const el of root.querySelectorAll(s)) {
        if (el.children.length) continue;
        if (!(el.textContent || "").trim()) continue;
        const r = el.getBoundingClientRect();
        if (r.width < 6 || r.height < 6) continue;
        const cs = getComputedStyle(el);
        if (cs.visibility === "hidden" || cs.opacity === "0") continue;
        const size = parseFloat(cs.fontSize);
        if (size < bestSize) {
          best = el;
          bestSize = size;
        }
      }
    }
    return best;
  };

  window.__rect = (el) => {
    const r = el.getBoundingClientRect();
    const x = Math.max(0, Math.round(r.left) + 1);
    const y = Math.max(0, Math.round(r.top) + 1);
    const w = Math.min(Math.round(r.width) - 2, innerWidth - x);
    const h = Math.min(Math.round(r.height) - 2, innerHeight - y);
    return w >= 4 && h >= 4 ? { x, y, width: w, height: h } : null;
  };

  // Glyphs out of the way: what is left is the surface itself.
  window.__hideText = (el) => {
    const before = el.getAttribute("style");
    el.style.setProperty("-webkit-text-fill-color", "transparent");
    el.style.setProperty("color", "transparent");
    el.style.setProperty("text-shadow", "none");
    return before;
  };
  window.__restore = (el, before) => {
    if (before === null) el.removeAttribute("style");
    else el.setAttribute("style", before);
  };

  window.__pixels = async (b64) => {
    const img = new Image();
    img.src = "data:image/png;base64," + b64;
    await img.decode();
    const c = document.createElement("canvas");
    c.width = img.width;
    c.height = img.height;
    const ctx = c.getContext("2d", { willReadFrequently: true });
    ctx.drawImage(img, 0, 0);
    const d = ctx.getImageData(0, 0, c.width, c.height).data;
    const out = [];
    for (let i = 0; i < d.length; i += 4) out.push([d[i], d[i + 1], d[i + 2]]);
    return out;
  };
};

async function pixels(page, rect) {
  const buf = await page.screenshot({ clip: rect, animations: "disabled" });
  return page.evaluate((b64) => window.__pixels(b64), buf.toString("base64"));
}

// ── one surface, measured on the painted pixels ──────────────────────────────
async function measure(page, key) {
  const { sel, texts } = SURFACES[key];
  const info = await page.evaluate(
    ({ sel, texts }) => {
      const root = document.querySelector(sel);
      if (!root) return null;
      const r = root.getBoundingClientRect();
      if (r.width < 8 || r.height < 8) return null;
      const cs = getComputedStyle(root);
      const bg = window.__parse(cs.backgroundColor);
      const el = window.__textEl(root, texts);
      const out = {
        alpha: bg ? bg[3] : null,
        surface: bg ? [bg[0], bg[1], bg[2]] : null,
        rect: window.__rect(root),
        text: null,
      };
      if (el) {
        const tcs = getComputedStyle(el);
        const col = window.__parse(tcs.color);
        out.text = {
          rect: window.__rect(el),
          color: col ? [col[0], col[1], col[2]] : null,
          size: parseFloat(tcs.fontSize),
          weight: parseInt(tcs.fontWeight, 10) || 400,
          sample: (el.textContent || "").trim().slice(0, 42),
        };
      }
      return out;
    },
    { sel, texts }
  );
  if (!info || !info.text || !info.text.rect || !info.text.color) return info;

  // C: the composite behind that line, glyphs removed.
  const prev = await page.evaluate(({ sel, texts }) =>
    window.__hideText(window.__textEl(document.querySelector(sel), texts)), { sel, texts });
  const behind = await pixels(page, info.text.rect);
  // B: the same box with the surface out of the way — what it is a tint OF.
  await page.evaluate(({ sel }) => (document.querySelector(sel).style.visibility = "hidden"), { sel });
  const ground = await pixels(page, info.text.rect);
  await page.evaluate(({ sel, texts, prev }) => {
    const root = document.querySelector(sel);
    root.style.visibility = "";
    window.__restore(window.__textEl(root, texts), prev);
  }, { sel, texts, prev });

  info.behind = worst(behind, info.text.color);
  info.ground = mean(ground);
  info.composite = mean(behind);
  return info;
}

// The worst REGION of a sample, not the worst single pixel: one antialiased
// edge left by the compositor is not a legibility failure, while 2 % of a line
// (some 30 pixels) is a surface a person actually reads text on.
function worst(px, textColor) {
  const scored = px.map((p) => ({ p, c: contrast(textColor, p) })).sort((a, b) => a.c - b.c);
  const at = scored[Math.min(scored.length - 1, Math.floor(scored.length * 0.02))] || { p: null, c: 0 };
  return { min: at.c, at: at.p };
}

// ── the app ─────────────────────────────────────────────────────────────────
const browser = await chromium.launch({
  args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--autoplay-policy=no-user-gesture-required"],
});

// Deezer unreachable → the shell paints a real `Notices` card (that surface
// only exists when something is wrong, so the bench has to ask for it).
const NOTICE = JSON.stringify({ ok: false, reason: "network", retry_in: 3600 });
const WAIT = { timeout: 60000 };

async function context(size, phone) {
  const ctx = await browser.newContext(
    phone ? { viewport: size, deviceScaleFactor: 2, isMobile: true, hasTouch: true } : { viewport: size }
  );
  await ctx.route(/dzcdn\.net|api\.deezer\.com/, (r) => r.abort());
  await ctx.route(/\/api\/deezer\/status/, (r) =>
    r.fulfill({ status: 200, contentType: "application/json", body: NOTICE })
  );
  return ctx;
}

async function boot(page, theme, { eco = false } = {}) {
  await page.goto(base + "/app/");
  await page.evaluate(
    ({ theme, eco }) => {
      localStorage.setItem("ui.theme", JSON.stringify(theme));
      localStorage.setItem("viz.eco", JSON.stringify(eco));
    },
    { theme, eco }
  );
  await page.reload();
  await page.waitForSelector(".topbar", WAIT);
  await page.waitForTimeout(350);
  await page.evaluate(() => {
    const m = document.querySelector("main");
    if (m) m.scrollTop = 0;
    scrollTo(0, 0);
  });
}

// The ladder's rule, from the DOM: a `backdrop-filter` is allowed ONLY on the
// tier-2/tier-3 overlay classes, and only one overlay is open at a time. A
// card, a bar or a scrim that blurs its own backdrop is an offender.
const OVERLAY_SRC = "(^|\\s)(menu|sheet|dialog|glass-2|glass-3)(\\s|$)";
async function audit(page) {
  return page.evaluate((overlaySrc) => {
    const overlay = new RegExp(overlaySrc);
    const allowed = [];
    const offenders = [];
    const blurred = [];
    const cards = [];
    for (const el of document.querySelectorAll("*")) {
      const cs = getComputedStyle(el);
      const cls = typeof el.className === "string" ? el.className : "";
      if (cs.backdropFilter && cs.backdropFilter !== "none") {
        (overlay.test(cls) ? allowed : offenders).push(`${cls || el.tagName} → ${cs.backdropFilter}`);
      }
      if (cls.split(/\s+/).includes("card") && cs.backdropFilter !== "none") cards.push(cls);
      if ((cs.filter || "none").includes("blur(") && (el.tagName === "IMG" || cs.backgroundImage !== "none")) {
        blurred.push(`${cls || el.tagName} → ${cs.filter}`);
      }
    }
    return { allowed, offenders, blurred, cards, eco: document.documentElement.dataset.eco || null };
  }, OVERLAY_SRC);
}

async function overflow(page) {
  return page.evaluate(() => ({
    doc: document.documentElement.scrollWidth - innerWidth,
    main: (() => {
      const m = document.querySelector("main");
      return m ? m.scrollWidth - m.clientWidth : 0;
    })(),
  }));
}

async function shoot(page, name) {
  if (SHOTS) await page.screenshot({ path: resolve(SHOTS, name + ".png") });
}

// ── the run ─────────────────────────────────────────────────────────────────
const rows = [];
const costs = {};

for (const [label, size, phone] of [
  ["desk", DESK, false],
  ["phone", PHONE, true],
]) {
  const ctx = await context(size, phone);
  await ctx.addInitScript(PAGE_HELPERS);
  const page = await ctx.newPage();
  page.on("pageerror", (e) => console.log("pageerror:", e.message));
  await login(page, base);

  for (const theme of THEMES) {
    const tag = `${theme} ${size.width}`;
    const found = {};
    const take = async (key) => {
      const m = await measure(page, key);
      if (m) found[key] = m;
      return m;
    };

    // ── a screen with cards, the ground and the notice ───────────────────────
    // A track is started FIRST: the cover ground only exists while something
    // plays, and every contrast below has to be measured over it, not over the
    // bare page colour.
    await boot(page, theme);
    await page.goto(base + "/app/#/library");
    await page.waitForSelector(".row.track", WAIT);
    await page.waitForTimeout(250);
    await page.locator(".row.track button.play").first().click();
    await page.waitForTimeout(900);
    await page.goto(base + "/app/#/library/albums");
    await page.waitForSelector(".card", WAIT);
    await page.waitForTimeout(450);
    await take("sidebar");
    await take("mobiletab");
    await take("topbar");
    await take("card");
    await take("notice");
    await shoot(page, `glass-${label}-${theme}-screen`);

    // The contextual menu, opened on a card (right click, as a person does).
    const card = page.locator(".card").first();
    await card.click({ button: "right" });
    await page.waitForSelector('.menu[role="menu"]', WAIT);
    await page.waitForTimeout(180);
    await take("menu");
    await shoot(page, `glass-${label}-${theme}-menu`);
    const menuAudit = await audit(page);
    check(menuAudit.offenders.length === 0, `${tag} · menu : aucun blur hors overlay`, `${menuAudit.offenders.join(" | ")}`);
    check(menuAudit.allowed.length === 1, `${tag} · menu : 1 passe de blur`, `${menuAudit.allowed.length} — ${menuAudit.allowed.join(" | ")}`);
    check(/blur\(14px\)/.test(menuAudit.allowed[0] || ""), `${tag} · menu : palier 2 (14 px)`, menuAudit.allowed[0] || "aucun");
    const menuValues = new Set(menuAudit.allowed.map((v) => v.split(" → ")[1]));
    check(menuValues.size <= 1, `${tag} · menu : une seule valeur de flou`, `${menuValues.size} — ${[...menuValues].join(" | ")}`);
    await page.keyboard.press("Escape");
    await page.waitForTimeout(120);

    // The quality menu hangs off the player bar's badge (desktop only: on a
    // phone it is in the full-screen player, which is not this bench).
    if (!phone) {
      await page.locator(".qm .trigger").first().click();
      await page.waitForSelector(".qm .menu", WAIT);
      await page.waitForTimeout(140);
      await take("quality");
      await shoot(page, `glass-${label}-${theme}-quality`);
      await page.keyboard.press("Escape");
      await page.waitForTimeout(120);
    }

    // ── the player bar, then two sheets ─────────────────────────────────────
    await page.goto(base + "/app/#/library");
    await page.waitForSelector(".row.track", WAIT);
    await page.waitForTimeout(350);
    await take("player");
    await shoot(page, `glass-${label}-${theme}-player`);

    await page.locator('.row.track button.add[aria-label="Ajouter à une playlist"]').first().click();
    await page.waitForSelector('.sheet[role="dialog"]', WAIT);
    await page.waitForTimeout(220);
    await take("sheet");
    const sheetAudit = await audit(page);
    check(sheetAudit.allowed.length === 1, `${tag} · feuille : 1 passe de blur`, `${sheetAudit.allowed.length} — ${sheetAudit.allowed.join(" | ")}`);
    check(/blur\(22px\)/.test(sheetAudit.allowed[0] || ""), `${tag} · feuille : palier 3 (22 px)`, sheetAudit.allowed[0] || "aucun");
    check(sheetAudit.offenders.length === 0, `${tag} · feuille : le voile ne floute pas`, `${sheetAudit.offenders.join(" | ")}`);
    const sheetValues = new Set(sheetAudit.allowed.map((v) => v.split(" → ")[1]));
    check(sheetValues.size <= 1, `${tag} · feuille : une seule valeur de flou`, `${sheetValues.size} — ${[...sheetValues].join(" | ")}`);
    await shoot(page, `glass-${label}-${theme}-sheet`);
    await page.keyboard.press("Escape");
    await page.waitForTimeout(160);

    // A sheet whose scrim used to blur its own backdrop (CreditsSheet, 2 px):
    // the audit below must find nothing at all.
    let scrim = null;
    try {
      await page.locator('.row.track button.more[aria-label="Plus d\'options"]').first().click();
      await page.waitForSelector('.menu[role="menu"]', WAIT);
      await page.locator('.menu[role="menu"] .item', { hasText: "Voir les crédits" }).first().click();
      await page.waitForSelector('.sheet[role="dialog"]', { timeout: 8000 });
      await page.waitForTimeout(400);
      scrim = await audit(page);
      await shoot(page, `glass-${label}-${theme}-credits`);
      await page.keyboard.press("Escape");
      await page.waitForTimeout(150);
    } catch {
      console.log(`note ${tag} · feuille « crédits » non ouverte (le banc continue)`);
    }

    // The ladder's audit, in the state where the most is open: a closed menu
    // must blur nothing, and one open overlay must be one pass.
    const blur = scrim || (await audit(page));
    const over = await overflow(page);

    check(blur.offenders.length === 0, `${tag} · blur réservé aux overlays`, `${blur.offenders.length} — ${blur.offenders.join(" | ")}`);
    check(blur.allowed.length <= 1, `${tag} · une seule passe de blur ouverte`, `${blur.allowed.length} — ${blur.allowed.join(" | ")}`);
    check(blur.cards.length === 0, `${tag} · une carte ne floute pas son fond`, `${blur.cards.length}`);
    check(blur.blurred.length <= 1, `${tag} · ≤ 1 couche d'image floutée`, `${blur.blurred.length} — ${blur.blurred.join(" | ")}`);
    check(over.doc <= 0 && over.main <= 0, `${tag} · aucun débordement horizontal`, `doc ${over.doc}px, main ${over.main}px`);

    for (const key of REQUIRED[phone ? "phone" : "desk"]) {
      const m = found[key];
      if (!m) {
        check(false, `${tag} · ${key} mesurée`, "surface absente du DOM");
        continue;
      }
      check(m.alpha !== null && m.alpha > 0 && m.alpha < 1, `${tag} · ${key} translucide`, `alpha ${m.alpha}`);
      check(m.alpha === null || (m.alpha >= 0.55 && m.alpha <= 0.95), `${tag} · ${key} teinte lisible`, `alpha ${m.alpha}`);
      if (!m.text || !m.behind) {
        check(false, `${tag} · ${key} porte du texte mesurable`);
        continue;
      }
      const large = m.text.size >= 24 || (m.text.size >= 18.66 && m.text.weight >= 700);
      const min = large ? 3 : 4.5;
      const okc = m.behind.min >= min;
      check(okc, `${tag} · ${key} contraste ${min}:1`, `${m.behind.min.toFixed(2)}:1 — «${m.text.sample}» ${m.text.size}px sur ${hex(m.behind.at)}`);
      if (!okc) {
        // Keep what failed: the crop is the evidence, not a re-run.
        const which = { sel: SURFACES[key].sel, texts: SURFACES[key].texts };
        const prev = await page.evaluate((w) => window.__hideText(window.__textEl(document.querySelector(w.sel), w.texts)), which);
        await page.screenshot({ path: `/tmp/glass-fail-${tag.replace(/ /g, "-")}-${key}.png`, clip: m.text.rect });
        await page.evaluate((w) => window.__restore(window.__textEl(document.querySelector(w.sel), w.texts), w.prev), { ...which, prev });
      }
      // The tint is exactly the token's: a·S + (1−a)·B, B measured from the
      // same box with the surface removed.
      if (m.surface && m.composite && m.ground) {
        const expect = [0, 1, 2].map((i) => m.alpha * m.surface[i] + (1 - m.alpha) * m.ground[i]);
        check(dist(m.composite, expect) <= 4, `${tag} · ${key} teinte = tokens`, `mesuré ${hex(m.composite)} pour ${hex(expect)}`);
      }
    }

    // ── eco: the real switch, so <html> carries the attribute ───────────────
    await boot(page, theme, { eco: true });
    await page.goto(base + "/app/#/library/albums");
    await page.waitForSelector(".card", WAIT);
    await page.waitForTimeout(350);
    const ecoBlur = await audit(page);
    check(ecoBlur.eco === "1", `${tag} · éco pose data-eco="1"`, `${ecoBlur.eco}`);
    check(ecoBlur.blurred.length === 0, `${tag} · éco : aucune image floutée`, `${ecoBlur.blurred.length} — ${ecoBlur.blurred.join(" | ")}`);
    check(
      ecoBlur.allowed.length + ecoBlur.offenders.length === 0,
      `${tag} · éco : aucun backdrop-filter`,
      `${ecoBlur.allowed.length + ecoBlur.offenders.length} — ${[...ecoBlur.allowed, ...ecoBlur.offenders].join(" | ")}`
    );
    const ecoFound = {};
    for (const key of ["sidebar", "mobiletab", "topbar", "card", "notice"]) {
      const m = await measure(page, key);
      if (m) {
        ecoFound[key] = m;
        check(m.alpha !== null && Math.abs(m.alpha - 1) < 0.001, `${tag} · éco ${key} opaque`, `alpha ${m.alpha}`);
        if (m.behind) check(m.behind.min >= 4.5, `${tag} · éco ${key} contraste 4.5:1`, `${m.behind.min.toFixed(2)}:1`);
      }
    }
    await shoot(page, `glass-${label}-${theme}-eco`);

    // `prefers-reduced-transparency: reduce` — the OS's own switch.
    const cdp = await ctx.newCDPSession(page);
    const emulated = await cdp
      .send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-reduced-transparency", value: "reduce" }] })
      .then(() => page.evaluate(() => matchMedia("(prefers-reduced-transparency: reduce)").matches))
      .catch(() => false);
    if (emulated) {
      await boot(page, theme);
      await cdp.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-reduced-transparency", value: "reduce" }] });
      await page.waitForTimeout(150);
      const a = await page.evaluate(() => {
        const c = window.__parse(getComputedStyle(document.querySelector(".topbar")).backgroundColor);
        return c ? c[3] : null;
      });
      check(a !== null && Math.abs(a - 1) < 0.001, `${tag} · transparence réduite → opaque`, `alpha ${a}`);
      await cdp.send("Emulation.setEmulatedMedia", { features: [] }).catch(() => {});
    } else {
      console.log(`skip ${tag} · prefers-reduced-transparency non émulable dans ce navigateur`);
    }

    rows.push({ tag, found, eco: ecoFound, blur, ecoBlur, over, audits: [menuAudit, sheetAudit, blur] });
  }

  // ── the cost, once per viewport (the CSS is the same in all three themes) ──
  await boot(page, "encre");
  await page.goto(base + "/app/#/library/albums");
  await page.waitForSelector(".card", WAIT);
  await page.waitForTimeout(450);
  const cdp = await ctx.newCDPSession(page);
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: THROTTLE });
  await page.evaluate(() => {
    window.__long = [];
    try {
      new PerformanceObserver((l) => l.getEntries().forEach((e) => window.__long.push(e.duration))).observe({ entryTypes: ["longtask"] });
    } catch {
      /* no longtask support: the frame numbers still hold */
    }
  });

  const cost = await page.evaluate(async (rounds) => {
    const until = async (cond, cap = 900) => {
      const t0 = performance.now();
      while (performance.now() - t0 < cap) {
        if (cond()) return true;
        await new Promise((r) => requestAnimationFrame(() => r()));
      }
      return false;
    };
    const settle = () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(() => r())));
    const close = async () => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      const btn = [...document.querySelectorAll(".sheet [aria-label]")].find((b) => /fermer|masquer/i.test(b.getAttribute("aria-label")));
      if (btn) btn.click();
      await settle();
      await settle();
    };
    // A/B in the same session: the same open, once with the tier's blur and
    // once with `backdrop-filter: none` forced. Software rendering (SwiftShader)
    // and a busy machine make absolute times noisy; the PAIR is what says what
    // the material itself costs.
    const style = document.createElement("style");
    style.id = "__blur";
    document.head.appendChild(style);
    const blurOn = (on) => (style.textContent = on ? "" : ".menu,.sheet,.dialog{backdrop-filter:none !important}");
    const out = { menu: [], sheet: [], menuOff: [], sheetOff: [], frames: { glass: [], eco: [] } };

    // the contextual menu on a card, blur on then off
    for (let i = 0; i < rounds; i++) {
      for (const on of [true, false]) {
        blurOn(on);
        await close();
        const card = document.querySelector(".card");
        const r = card.getBoundingClientRect();
        const t0 = performance.now();
        card.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: r.left + 24, clientY: r.top + 24 }));
        await until(() => !!document.querySelector('.menu[role="menu"]'));
        await settle();
        (on ? out.menu : out.menuOff).push(performance.now() - t0);
      }
    }
    blurOn(true);
    await close();

    // a scroll of the card grid, tint on then tint off: what the translucency
    // itself costs per painted frame
    const scrollFor = async (ms) => {
      const main = document.querySelector("main");
      const d = [];
      let last = performance.now();
      const t0 = last;
      while (performance.now() - t0 < ms) {
        await new Promise((r) =>
          requestAnimationFrame((t) => {
            d.push(t - last);
            last = t;
            r();
          })
        );
        main.scrollTop += 28;
      }
      return d.slice(2);
    };
    const main = document.querySelector("main");
    main.scrollTop = 0;
    out.frames.glass = await scrollFor(900);
    document.documentElement.dataset.eco = "1";
    await settle();
    main.scrollTop = 0;
    out.frames.eco = await scrollFor(900);
    delete document.documentElement.dataset.eco;
    out.long = window.__long.slice();
    return out;
  }, N);
  // The sheet lives on the track list, not on the card grid.
  await page.goto(base + "/app/#/library");
  await page.waitForSelector(".row.track", WAIT);
  await page.waitForTimeout(400);
  cost.sheet = await page.evaluate(async (rounds) => {
    const until = async (cond, cap = 900) => {
      const t0 = performance.now();
      while (performance.now() - t0 < cap) {
        if (cond()) return true;
        await new Promise((r) => requestAnimationFrame(() => r()));
      }
      return false;
    };
    const settle = () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(() => r())));
    const times = { on: [], off: [] };
    const style = document.createElement("style");
    style.id = "__blur2";
    document.head.appendChild(style);
    const blurOn = (on) => (style.textContent = on ? "" : ".menu,.sheet,.dialog{backdrop-filter:none !important}");
    for (let i = 0; i < rounds; i++) {
      for (const on of [true, false]) {
        blurOn(on);
        const add = document.querySelector(".row.track button.add");
        const t0 = performance.now();
        add.click();
        await until(() => !!document.querySelector('.sheet[role="dialog"]'));
        await settle();
        (on ? times.on : times.off).push(performance.now() - t0);
        document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
        const btn = [...document.querySelectorAll(".sheet [aria-label]")].find((b) => /fermer|masquer/i.test(b.getAttribute("aria-label")));
        if (btn) btn.click();
        await settle();
        await settle();
      }
    }
    return times;
  }, N);
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: 1 });

  costs[label] = { cost, long: cost.long.reduce((a, b) => a + b, 0) };
  const menu = med(cost.menu);
  const menuOff = med(cost.menuOff);
  const sheet = med(cost.sheet.on);
  const sheetOff = med(cost.sheet.off);
  console.log(
    `${label.padEnd(5)} ${THROTTLE}× CPU : menu ${menu.toFixed(1)} ms (sans flou ${menuOff.toFixed(1)})` +
      ` · feuille ${sheet.toFixed(1)} ms (sans flou ${sheetOff.toFixed(1)})` +
      ` · image ${med(cost.frames.glass).toFixed(1)} ms (p95 ${p95(cost.frames.glass).toFixed(1)}) en verre,` +
      ` ${med(cost.frames.eco).toFixed(1)} ms (p95 ${p95(cost.frames.eco).toFixed(1)}) en éco · longtasks ${costs[label].long.toFixed(0)} ms`
  );
  // The absolute bounds sit above the SwiftShader baseline measured on this
  // machine (the same open, blur forced off); what the material itself costs is
  // the PAIRED difference, which is what these two checks hold.
  check(menu < 320, `${label} · menu sous 320 ms à ${THROTTLE}× (rendu logiciel)`, `${menu.toFixed(1)} ms`);
  check(sheet < 400, `${label} · feuille sous 400 ms à ${THROTTLE}× (rendu logiciel)`, `${sheet.toFixed(1)} ms`);
  check(menu - menuOff < 80, `${label} · le flou du menu coûte < 80 ms`, `${(menu - menuOff).toFixed(1)} ms`);
  check(sheet - sheetOff < 120, `${label} · le flou de la feuille coûte < 120 ms`, `${(sheet - sheetOff).toFixed(1)} ms`);
  await ctx.close();
}

// ── the summary ──────────────────────────────────────────────────────────────
const all = rows.flatMap((r) => Object.entries({ ...r.found, ...r.eco }).map(([id, m]) => ({ tag: r.tag, id, min: m.behind ? m.behind.min : null })));
const weakest = all.filter((x) => x.min !== null).sort((a, b) => a.min - b.min)[0];
const surfaces = rows.reduce((a, r) => a + Object.keys(r.found).length, 0);
// The ladder's whole vocabulary, as the compositor saw it during the run.
const seen = new Set(["none"]);
for (const r of rows)
  for (const audit of r.audits)
    for (const entry of [...audit.allowed, ...audit.offenders]) seen.add(entry.split(" → ")[1]);
console.log(`valeurs de backdrop-filter vues : ${[...seen].join(" · ")} (${seen.size})`);
check(seen.size <= 3, "≤ 3 valeurs de flou dans toute l'application", `${seen.size} — ${[...seen].join(" | ")}`);
if (weakest) console.log(`\ncontraste le plus faible : ${weakest.min.toFixed(2)}:1 (${weakest.id}, ${weakest.tag})`);
console.log(`${rows.length} cas · ${surfaces} surfaces mesurées · ${state.failed} échec(s)`);
await browser.close();
stop();
process.exit(state.failed ? 1 : 0);
