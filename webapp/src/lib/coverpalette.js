// The colours of a sleeve, read ONCE and shared by everything that wants them.
//
// Two consumers today, and they need the same thing for different reasons:
//   - the app's ground (`components/GlobalBackdrop.svelte`) paints a flat tint
//     under the blurred image while the sleeve decodes, so the page is never a
//     bare theme colour before the art arrives;
//   - the visualizer (`lib/viz/**`) colours its bars from the sleeve instead of
//     from an arbitrary palette.
// Reading a cover is a DECODE (a canvas draw plus a 64×64 readback), so it is
// done once per URL and kept: `paletteFor(url)` is memoised, concurrent callers
// share the one in-flight promise, and a failure is never stored (the Deezer
// image CDN drops requests; a sleeve must get its colours back when it answers).
//
// The canvas is 64×64 — well under the 128×128 ceiling — because everything
// here is an average over a region: the dominant colour of a 500px sleeve and
// of its 64px reduction are the same colour, and 4 096 pixels are a few
// hundred microseconds, not a frame.
//
// The shape returned by `paletteFor` (and by `analyse`, its pure half):
//
//   {
//     dominant: [r, g, b],   // the most PRESENT colour (what the ground tints with)
//     average:  [r, g, b],   // the mean of every opaque pixel
//     vivid:    [r, g, b],   // the most saturated well-lit colour (an accent)
//     colors:   [[r,g,b]…],  // up to 5 distinct dominant colours, most present first
//     zones:    { left, center, right },   // the three vertical bands (columns)
//     bands:    { top, middle, bottom },   // the three horizontal bands (rows)
//     middles:  [left, center, right],     // the MIDDLE row — its three cells
//     cells:    [[r,g,b] × 9],             // 3 columns × 3 rows, row-major
//     light:    true|false,                // the average reads as a light surface
//   }
//
// `zones` is what a horizontal gradient follows (left → right across the art),
// `bands` what a vertical one follows, and `middles` is the row the eye lands on
// when the sleeve is centred. Every entry is an opaque [r, g, b]; a region with
// no visible pixel is null.
//
// Nothing here throws and nothing rejects: `paletteFor` resolves null when the
// bytes could not be read (offline, a tainted canvas, a hostile image), and the
// caller keeps whatever it had.

export const PALETTE_SIZE = 64;
const CACHE_MAX = 16;
// Below this alpha a pixel is not part of the picture (a rounded PNG corner, a
// transparent sleeve): averaging it in would drag every zone toward black.
const ALPHA_MIN = 200;
// How far apart two "dominant" colours must be to be reported as distinct.
const DISTINCT = 26;
const MAX_COLORS = 5;

const clamp8 = (v) => (v < 0 ? 0 : v > 255 ? 255 : Math.round(v));

function mean(bin) {
  return bin && bin.n ? [clamp8(bin.r / bin.n), clamp8(bin.g / bin.n), clamp8(bin.b / bin.n)] : null;
}

function dist(a, b) {
  return Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
}

/**
 * The pure half: a grid of RGBA pixels in, the palette out.
 *
 * `pixels` is anything array-like with four bytes per pixel (`ImageData.data`),
 * `width` × `height` its shape. Kept free of the DOM so the rules below are
 * testable on a hand-drawn image (test/coverpalette.test.mjs).
 */
export function analyse(pixels, width, height) {
  const w = Math.max(1, Math.round(width) || 0);
  const h = Math.max(1, Math.round(height) || 0);
  if (!pixels || pixels.length < w * h * 4) return null;

  const cols = [0, 1, 2].map(() => ({ r: 0, g: 0, b: 0, n: 0 }));
  const rows = [0, 1, 2].map(() => ({ r: 0, g: 0, b: 0, n: 0 }));
  const cells = [0, 1, 2, 3, 4, 5, 6, 7, 8].map(() => ({ r: 0, g: 0, b: 0, n: 0 }));
  // 4 bits a channel: 4 096 buckets, which is a colour count a 64px sleeve can
  // actually fill — and the bucket's own mean is the colour reported, so the
  // quantisation decides the grouping, never the result.
  const hist = new Map();
  let tr = 0;
  let tg = 0;
  let tb = 0;
  let tn = 0;

  for (let y = 0; y < h; y++) {
    const row = Math.min(2, Math.floor((y * 3) / h));
    for (let x = 0; x < w; x++) {
      const i = (y * w + x) * 4;
      if (pixels[i + 3] < ALPHA_MIN) continue;
      const r = pixels[i];
      const g = pixels[i + 1];
      const b = pixels[i + 2];
      const col = Math.min(2, Math.floor((x * 3) / w));
      const add = (bin) => {
        bin.r += r;
        bin.g += g;
        bin.b += b;
        bin.n++;
      };
      add(cols[col]);
      add(rows[row]);
      add(cells[row * 3 + col]);
      tr += r;
      tg += g;
      tb += b;
      tn++;
      const key = ((r >> 4) << 8) | ((g >> 4) << 4) | (b >> 4);
      let bin = hist.get(key);
      if (!bin) {
        bin = { r: 0, g: 0, b: 0, n: 0 };
        hist.set(key, bin);
      }
      bin.r += r;
      bin.g += g;
      bin.b += b;
      bin.n++;
    }
  }
  if (!tn) return null;

  const average = [clamp8(tr / tn), clamp8(tg / tn), clamp8(tb / tn)];

  // Most present first; a colour too close to one already kept is dropped, so
  // five near-identical darks never stand in for five different colours.
  const colors = [];
  const bins = [...hist.values()].sort((a, b) => b.n - a.n);
  for (const bin of bins) {
    const c = mean(bin);
    if (!c) continue;
    if (colors.some((kept) => dist(kept, c) < DISTINCT)) continue;
    colors.push(c);
    if (colors.length >= MAX_COLORS) break;
  }

  // The colour a bar or a focus ring can use: saturated AND not at an extreme
  // of lightness, the rule color.js has always used for the header accent.
  let vivid = average;
  let best = 0.15;
  for (const bin of bins) {
    const c = mean(bin);
    if (!c) continue;
    const max = Math.max(c[0], c[1], c[2]);
    const min = Math.min(c[0], c[1], c[2]);
    const sat = max === 0 ? 0 : (max - min) / max;
    const lum = max / 255;
    const score = sat * (1 - Math.abs(lum - 0.55));
    if (score > best) {
      best = score;
      vivid = c;
    }
  }

  const zoneOf = (bin) => mean(bin);
  const middle = [3, 4, 5].map((k) => mean(cells[k]));

  return {
    dominant: colors[0] || average,
    average,
    vivid,
    colors,
    zones: { left: zoneOf(cols[0]), center: zoneOf(cols[1]), right: zoneOf(cols[2]) },
    bands: { top: zoneOf(rows[0]), middle: zoneOf(rows[1]), bottom: zoneOf(rows[2]) },
    middles: middle,
    cells: cells.map(mean),
    // A surface flag, not a WCAG pair: the caller chooses the ink. Weighted as
    // the eye weighs, on the sRGB values themselves.
    light: 0.2126 * average[0] + 0.7152 * average[1] + 0.0722 * average[2] > 140,
  };
}

function sameOrigin(url) {
  if (!url) return true;
  if (url.startsWith("blob:") || url.startsWith("data:")) return true;
  try {
    return new URL(url, window.location.href).origin === window.location.origin;
  } catch {
    return true;
  }
}

/**
 * The browser half: load `url`, draw it once on a 64×64 canvas, read it back.
 *
 * `crossOrigin` is set only for a CROSS-origin source — it un-taints the canvas
 * — and never for our own routes: it drops the session cookie, and a
 * login-protected `/api/cover` would answer 401 (the lesson color.js paid for).
 *
 * An image request can hang forever with no error, so a watchdog settles it:
 * nothing here may hold a pending palette for the rest of the session.
 */
export function decodeImage(url, { size = PALETTE_SIZE, timeoutMs = 8000 } = {}) {
  return new Promise((resolve, reject) => {
    if (typeof document === "undefined" || typeof Image === "undefined")
      throw new Error("no DOM to decode a cover");
    const img = new Image();
    if (!sameOrigin(url)) img.crossOrigin = "anonymous";
    img.referrerPolicy = "no-referrer";
    img.decoding = "async";
    let settled = false;
    const settle = (ok) => {
      if (settled) return;
      settled = true;
      clearTimeout(watch);
      img.onload = img.onerror = null;
      if (!ok) {
        try {
          img.removeAttribute("src");
        } catch {
          /* ignore */
        }
        reject(new Error("cover could not be read"));
      }
    };
    const watch = setTimeout(() => settle(img.complete && img.naturalWidth > 0), timeoutMs);
    img.onload = () => {
      try {
        const canvas = document.createElement("canvas");
        canvas.width = canvas.height = size;
        const ctx = canvas.getContext("2d", { willReadFrequently: true });
        if (!ctx) throw new Error("no 2d context");
        ctx.drawImage(img, 0, 0, size, size);
        const { data } = ctx.getImageData(0, 0, size, size);
        settle(true);
        resolve({ data, width: size, height: size });
      } catch (e) {
        settle(false);
        reject(e);
      }
    };
    img.onerror = () => settle(false);
    img.src = url;
  });
}

/**
 * A cache of palettes by URL. Exported as a factory so the cache rules
 * (one decode per URL, concurrent calls shared, failures not remembered, a
 * bounded size) are testable without a browser: pass your own `decode`.
 */
export function createPaletteCache({ decode = decodeImage, limit = CACHE_MAX } = {}) {
  const values = new Map(); // url -> palette (successes only)
  const running = new Map(); // url -> promise

  function recall(url, palette) {
    if (values.size >= limit) values.delete(values.keys().next().value);
    values.set(url, palette);
  }

  function paletteFor(url) {
    if (!url || typeof url !== "string") return Promise.resolve(null);
    if (values.has(url)) return Promise.resolve(values.get(url));
    const inFlight = running.get(url);
    if (inFlight) return inFlight;
    const p = Promise.resolve()
      .then(() => decode(url))
      .then((shot) => {
        const palette = shot ? analyse(shot.data, shot.width, shot.height) : null;
        // A failure is NOT remembered: the next visit must be able to ask again.
        if (palette) recall(url, palette);
        return palette;
      })
      .catch(() => null)
      .finally(() => running.delete(url));
    running.set(url, p);
    return p;
  }

  return {
    paletteFor,
    clear() {
      values.clear();
      running.clear();
    },
    size: () => values.size,
  };
}

const shared = createPaletteCache();

/** The app's one palette cache: `paletteFor(coverUrl)` → Promise<palette|null>. */
export const paletteFor = shared.paletteFor;
export const clearPalettes = shared.clear;
export const paletteCacheSize = shared.size;
