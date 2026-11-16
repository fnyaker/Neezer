#!/usr/bin/env node
// The lyrics view, in its two shapes.
//
//   npm run build && node test/lyrics/run.mjs
//   SHOTS=/tmp/lyrics node test/lyrics/run.mjs          # PNGs of every case
//
// 1. SEPARATED — the wide desktop player's own column (≥1200 px). The words
//    float straight on the player's blurred sleeve, as Spotify, Deezer and
//    Apple Music's full-screen players show them: no ground, no frame, no
//    radius, no shadow of their own. This bench holds that on the computed
//    style AND on the painted pixels: the line being heard keeps WCAG AA
//    (4.5:1), and so do the past/future lines, over a WHITE and a BLACK
//    sleeve, in all three themes. The theme's own background colour is the
//    text shadow's ink (Lyrics.svelte), which is what lets the words read
//    without a surface; the bench samples the RAW backdrop with the glyphs and
//    their shadow removed, so it measures the worst case, not the help.
// 2. INTEGRATED — a narrow desktop's queue card ("Paroles" tab) and the
//    phone's lyrics sheet keep the glass of the cards around them (tier 1 for
//    the card, tier 3 for the sheet). Their surface is asserted, not changed.
//
// Measured on this bench (1440×900, 3 themes × 2 sleeves): the active line
// 11.06–17.19:1, past/future 4.66–7.80:1; worst pair Encre over a white
// sleeve at 4.66:1. The card-less column computes to `rgba(0, 0, 0, 0)`, a
// 0 px border, a 0 px radius and no shadow; the integrated card keeps its
// 64 % fill and 24 px radius; the phone sheet keeps its tier-3 backdrop.
import { join } from "node:path";
import { createRequire } from "node:module";
import { loadPlaywright, startServer, silence, login, checker } from "../harness.mjs";

const require = createRequire(import.meta.url);
const zlib = require("node:zlib");

const { chromium } = await loadPlaywright();
const { base, ids, stop } = await startServer();
const { check, state } = checker();
const SHOTS = process.env.SHOTS;
const WAV = silence(60);

// Two lines before 0:00 so a player a second in has an active line in the
// middle of the view, with past and future lines around it (the real case).
const LINES = [
  "On a laissé la ville derrière nous",
  "Les néons s'éteignent un par un",
  "Et le silence reprend la rue",
  "Tu dis que demain ne ressemble à rien",
  "On marche sans savoir où l'on va",
  "Les ombres dansent sur les murs",
  "Il reste un peu de lumière au bout",
  "Et nos pas résonnent encore",
  "Le vent nous parle à voix basse",
  "On écoute sans rien répondre",
  "Les étoiles tombent dans la mer",
  "Et le temps s'arrête un instant",
  "Tu ris comme si tout était simple",
  "Je garde ce rire dans mes mains",
  "La nuit nous appartient encore",
  "On n'a rien promis, c'est mieux",
  "Les heures passent sans nous compter",
  "Et l'aube arrive sans prévenir",
  "On rentrera quand la ville s'éveille",
  "Avec du sable dans les poches",
];
const SYNCED = LINES.map((text, i) => ({ time: (i - 2) * 4000, text }));

// ── WCAG 2.2, 1.4.3, on the painted pixels ──────────────────────────────────
const lum = ([r, g, b]) => {
  const f = (v) => {
    const c = v / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
};
const contrast = (a, b) => {
  const la = lum(a);
  const lb = lum(b);
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
};
const rgbOf = (s) => {
  s = String(s).trim();
  // Chromium serialises a plain colour as `rgb()`/`rgba()` and a color-mix()
  // result as `color(srgb r g b [/ a])`; both are the same 0..255 answer.
  let m = s.match(/^rgba?\(([^)]+)\)$/);
  if (m) {
    const p = m[1].split(/[ ,/]+/).filter(Boolean).map(Number);
    return [p[0], p[1], p[2]];
  }
  m = s.match(/^color\(srgb\s+([\d.eE+-]+)\s+([\d.eE+-]+)\s+([\d.eE+-]+)(?:\s*\/\s*[\d.eE+-]+)?\)$/);
  if (m) return [Number(m[1]) * 255, Number(m[2]) * 255, Number(m[3]) * 255];
  return null;
};
// The worst 2 % of the line's box, not the worst pixel: one composited edge is
// not a legibility failure, a patch is (the same rule as test/glass).
const worst = (px, color) => {
  const scored = px.map((p) => contrast(color, p)).sort((a, b) => a - b);
  return scored[Math.min(scored.length - 1, Math.floor(scored.length * 0.02))] || 0;
};

async function pixels(page, rect) {
  const buf = await page.screenshot({ clip: rect, animations: "disabled" });
  return page.evaluate(async (b64) => {
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
  }, buf.toString("base64"));
}

async function lineIndexes(page, selector, boxSel) {
  return page.evaluate(
    ({ selector, boxSel }) => {
      const box = document.querySelector(boxSel)?.getBoundingClientRect();
      if (!box) return { active: -1, other: -1 };
      const lis = [...document.querySelectorAll(selector)];
      const inBox = lis
        .map((el, i) => ({ el, i }))
        .filter(({ el }) => {
          const r = el.getBoundingClientRect();
          // Inside the mask's edge fades, where the words are deliberately
          // half-dissolved, is not where a person reads a line.
          return r.top >= box.top + 40 && r.bottom <= box.bottom - 40;
        });
      return {
        active: inBox.find(({ el }) => el.classList.contains("active"))?.i ?? -1,
        other: inBox.find(({ el }) => !el.classList.contains("active"))?.i ?? -1,
      };
    },
    { selector, boxSel }
  );
}

async function measureLine(page, selector, index) {
  if (index < 0) return null;
  const info = await page.evaluate(
    ({ selector, index }) => {
      const el = [...document.querySelectorAll(selector)][index];
      if (!el) return null;
      const r = el.getBoundingClientRect();
      const cs = getComputedStyle(el);
      return {
        rect: {
          x: Math.round(r.left) + 1,
          y: Math.round(r.top) + 1,
          width: Math.round(r.width) - 2,
          height: Math.round(r.height) - 2,
        },
        color: cs.color,
        size: parseFloat(cs.fontSize),
        weight: parseInt(cs.fontWeight, 10) || 400,
      };
    },
    { selector, index }
  );
  if (!info || info.rect.width < 4 || info.rect.height < 4) return null;
  const color = rgbOf(info.color);
  if (!color) return null;
  // A class on <html>, not per-element inline styles: an element re-rendered
  // between the shot and the restore would keep its transparent fill forever
  // and the next screenshot would show a shadow-only line.
  await page.evaluate(() => document.documentElement.classList.add("bench-hide-lyrics"));
  const px = await pixels(page, info.rect);
  await page.evaluate(() => document.documentElement.classList.remove("bench-hide-lyrics"));
  // The line's `color` has a 200 ms transition: reading the NEXT line's colour
  // in the same tick would catch the start of this one coming back from
  // transparent (`oklab(0 0 0 / 0)`), not the colour a person sees.
  await page.waitForTimeout(240);
  const ratio = worst(px, color);
  return { ...info, contrast: +ratio.toFixed(2) };
}

// ── one page, the real player ───────────────────────────────────────────────
async function makePage(theme, sleeve, viewport) {
  const ctx = await browser.newContext({ viewport, deviceScaleFactor: 1 });
  await ctx.addInitScript((theme) => {
    try {
      localStorage.setItem("ui.theme", JSON.stringify(theme));
    } catch {
      /* a locked-down browser: the default theme is fine */
    }
  }, theme);
  // The mask that hides the glyphs while their backdrop is sampled. An init
  // script, not a style tag: a later navigation must not lose it.
  await ctx.addInitScript(() => {
    const css =
      ".bench-hide-lyrics .lyrics .synced li{-webkit-text-fill-color:transparent!important;color:transparent!important;text-shadow:none!important}";
    const add = () => {
      const style = document.createElement("style");
      style.textContent = css;
      document.head.append(style);
    };
    if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", add, { once: true });
    else add();
  });
  const png = SLEEVES[sleeve];
  await ctx.route(/dzcdn\.net|api\.deezer\.com/, (r) => {
    if (/images\/cover\//.test(r.request().url())) {
      return r.fulfill({
        status: 200,
        body: png,
        headers: { "Content-Type": "image/png", "Access-Control-Allow-Origin": "*" },
      });
    }
    return r.abort();
  });
  await ctx.route(/\/api\/stream\//, (r) =>
    r.fulfill({ status: 200, body: WAV, headers: { "Content-Type": "audio/wav", "Accept-Ranges": "none" } })
  );
  await ctx.route(/\/api\/lyrics\//, (r) =>
    r.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ lyrics: { synced: SYNCED } }) })
  );
  const cover = `https://e-cdns-images.dzcdn.net/images/cover/${sleeve}/1000x1000-000000-80-0-0.jpg`;
  await ctx.route(/\/api\/playlist\//, async (r) => {
    const res = await r.fetch();
    const body = await res.json();
    for (const t of body.tracks || []) t.album = { ...(t.album || {}), cover };
    await r.fulfill({ response: res, body: JSON.stringify(body), headers: { "Content-Type": "application/json" } });
  });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => console.log("pageerror:", e.message));
  await login(page, base);
  await page.reload();
  return page;
}

async function openPlayer(page, width) {
  await page.goto(base + "/app/#/playlist/" + ids.big);
  await page.waitForFunction(() => document.body.innerText.includes("4000 titres"), null, { timeout: 60000 });
  await page.locator('button.play[aria-label="Lire"]').first().click();
  await page.waitForTimeout(1200);
  if (width <= 640) {
    await page.locator('button[aria-label="Ouvrir le lecteur plein écran"]').first().click();
    await page.waitForSelector(".m .fb");
  } else {
    await page.locator('button[aria-label="Plein écran"]').first().click();
    await page.waitForSelector(".d .controls .pp");
  }
  await page.waitForTimeout(1400);
}

// A 64 px solid PNG, made here: the real backdrop pipeline (lib/backdrop.js)
// takes it through sampleOfImage/levelFactor exactly like a sleeve.
function solidPng([r, g, b]) {
  const w = 64;
  const raw = Buffer.concat(Array.from({ length: w }, () => Buffer.concat([Buffer.from([0]), Buffer.from(Array(w).fill([r, g, b]).flat())])));
  const crcTable = Array.from({ length: 256 }, (_, n) => {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    return c >>> 0;
  });
  const crc = (buf) => {
    let c = 0xffffffff;
    for (const byte of buf) c = crcTable[(c ^ byte) & 0xff] ^ (c >>> 8);
    return (c ^ 0xffffffff) >>> 0;
  };
  const chunk = (type, data) => {
    const len = Buffer.alloc(4);
    len.writeUInt32BE(data.length);
    const td = Buffer.concat([Buffer.from(type), data]);
    const sum = Buffer.alloc(4);
    sum.writeUInt32BE(crc(td));
    return Buffer.concat([len, td, sum]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(w, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 2; // truecolour
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", zlib.deflateSync(raw)),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

const SLEEVES = {
  white: solidPng([245, 245, 245]),
  black: solidPng([8, 8, 9]),
};

const browser = await chromium.launch({ args: ["--autoplay-policy=no-user-gesture-required"] });

const CASES = process.env.CASES
  ? process.env.CASES.split(";").map((s) => s.split("/"))
  : [
  ["encre", "white"],
  ["encre", "black"],
  ["papier", "white"],
  ["papier", "black"],
  ["pochette", "white"],
  ["pochette", "black"],
];

for (const [theme, sleeve] of CASES) {
  const page = await makePage(theme, sleeve, { width: 1440, height: 900 });
  await openPlayer(page, 1440);
  await page.waitForSelector(".lyrics-col .synced li", { timeout: 15000 });

  const surface = await page.evaluate(() => {
    const cs = getComputedStyle(document.querySelector(".lyrics-col"));
    return {
      background: cs.backgroundColor,
      border: cs.borderTopWidth,
      radius: cs.borderRadius,
      shadow: cs.boxShadow,
    };
  });
  check(
    surface.background === "rgba(0, 0, 0, 0)" && surface.border === "0px" && surface.radius === "0px" && surface.shadow === "none",
    `${theme}/${sleeve}: the separated column is not a card`,
    JSON.stringify(surface)
  );

  const indexes = await lineIndexes(page, ".lyrics-col .synced li", ".lyrics-col .lyrics");
  const active = await measureLine(page, ".lyrics-col .synced li", indexes.active);
  const other = await measureLine(page, ".lyrics-col .synced li", indexes.other);
  check(
    !!active && active.contrast >= 4.5,
    `${theme}/${sleeve}: the line being heard is 4.5:1 on the painted backdrop`,
    active ? `${active.contrast}:1 at ${active.size}px/${active.weight}` : "no active line"
  );
  check(
    !!other && other.contrast >= 4.5,
    `${theme}/${sleeve}: past/future lines are 4.5:1 on the painted backdrop`,
    other ? `${other.contrast}:1 at ${other.size}px/${other.weight}` : "no other line"
  );
  if (SHOTS) {
    await page.waitForTimeout(400);
    await page.screenshot({ path: join(SHOTS, `lyrics-${theme}-${sleeve}-1440x900.jpg`), type: "jpeg", quality: 86 });
  }

  // The integrated shape: the narrow desktop's queue card, Paroles tab.
  await page.setViewportSize({ width: 1024, height: 768 });
  await page.waitForTimeout(500);
  await page.locator('.d .tabs button:has-text("Paroles")').first().click();
  await page.waitForSelector(".side .synced li", { timeout: 15000 });
  const card = await page.evaluate(() => {
    const cs = getComputedStyle(document.querySelector(".side"));
    // Chromium serialises a color-mix() as `color(srgb … / 0.64)` and a plain
    // rgba() the other way; the alpha is the last slash anywhere in both.
    const s = String(cs.backgroundColor);
    const rgba = s.match(/^rgba?\(([^)]+)\)$/);
    let alpha = 1;
    if (rgba) {
      const p = rgba[1].split(/[ ,/]+/).filter(Boolean).map(Number);
      alpha = p.length > 3 ? p[3] : 1;
    } else {
      const slashed = s.match(/\/\s*([\d.]+)\s*\)\s*$/);
      alpha = slashed ? Number(slashed[1]) : 1;
    }
    return { alpha, border: cs.borderTopWidth, radius: cs.borderRadius };
  });
  check(
    card.alpha > 0.55 && card.alpha <= 0.88 && card.border === "1px" && card.radius === "24px",
    `${theme}/${sleeve}: the integrated lyrics keep their glass card`,
    JSON.stringify(card)
  );
  if (SHOTS) {
    await page.waitForTimeout(400);
    await page.screenshot({ path: join(SHOTS, `lyrics-${theme}-${sleeve}-1024x768.jpg`), type: "jpeg", quality: 86 });
  }
  await page.context().close();
}

// The phone's lyrics sheet: unchanged (tier 3 over the sleeve).
{
  const page = await makePage("encre", "white", { width: 390, height: 844 });
  await openPlayer(page, 390);
  await page.locator('.m button[aria-label="Paroles"]').first().click();
  await page.waitForSelector(".lyrics-view .synced li", { timeout: 15000 });
  const sheet = await page.evaluate(() => {
    const el = document.querySelector(".lyrics-view");
    const cs = getComputedStyle(el);
    return { filter: cs.backdropFilter || cs.webkitBackdropFilter, radius: cs.borderRadius };
  });
  check(
    /blur\(/.test(sheet.filter || ""),
    "phone: the lyrics sheet keeps the tier-3 glass",
    JSON.stringify(sheet)
  );
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - innerWidth);
  check(overflow <= 0, "phone: no sideways overflow", `${overflow}px`);
  if (SHOTS) {
    await page.waitForTimeout(600);
    await page.screenshot({ path: join(SHOTS, "lyrics-phone-390x844.jpg"), type: "jpeg", quality: 86 });
  }
  await page.context().close();
}

await browser.close();
stop();
process.exit(state.failed ? 1 : 0);
