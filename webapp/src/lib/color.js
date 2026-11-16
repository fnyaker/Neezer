// Extract a dominant/vibrant color from a cover image for Spotify-style
// gradient headers. Deezer CDN images are CORS-enabled, so the canvas isn't
// tainted; if extraction fails for any reason we fall back to a neutral purple.

import { loResCover } from "./format.js";

const cache = new Map();
const CACHE_MAX = 200; // bounded: a long browse session shouldn't grow forever
// Extractions already running, keyed by URL — two views asking for the same
// cover at once (the header and the now-playing backdrop) shared one decode
// instead of racing two.
const inFlight = new Map();
const FALLBACK = [60, 40, 90];

function remember(url, value) {
  if (cache.size >= CACHE_MAX) cache.delete(cache.keys().next().value);
  cache.set(url, value);
}

// A PICTURE'S LEVEL IS NORMALISED, NOT ITS COLOUR. The image behind the glass
// comes from album art, and sleeves are all over the place: a white cover made
// the ground (and everything seen through it) a pale wash, a near-black one a
// hole. The rule is a band, not a constant — the mood must still read as the
// sleeve's — so what sits between LOW and HIGH is left exactly as it is, a
// brighter picture is pulled down toward HIGH, a darker one lifted toward LOW,
// and both ends clamp before the art stops being itself.
//
// Measured on sRGB luma (0.2126 R + 0.7152 G + 0.0722 B, 0..255):
//   230 → 150 (×0.65, the floor), 180 → 120 (×0.67), 100 → 100 (untouched),
//   40 → 60 (×1.5, the cap), 10 → 15 (×1.5).  A 20 % grey keeps ×1, a white
//   sleeve lands at 150 and a black one at 15: a 10:1 span where the raw art
//   was 25:1, so the ambiance still varies while nothing blows out.
export const LEVEL_LOW = 64;
export const LEVEL_HIGH = 120;
export const LEVEL_MIN = 0.65;
export const LEVEL_MAX = 1.5;

export function levelFactor(rgb) {
  if (!rgb || rgb.length < 3) return 1;
  const l = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
  if (!(l > 0)) return LEVEL_MAX;
  if (l < LEVEL_LOW) return Math.min(LEVEL_MAX, Math.max(1, LEVEL_LOW / l));
  if (l > LEVEL_HIGH) return Math.min(1, Math.max(LEVEL_MIN, LEVEL_HIGH / l));
  return 1;
}

// The colour of an image ALREADY loaded in the page: drawn once into a 24 px
// canvas and scored for the most saturated, mid-bright pixel (a basically
// grayscale sleeve gets its average instead), plus that average's own level as
// `factor` — one draw, two answers, because the backdrop needs both. `null`
// when the canvas cannot read it at all — a cross-origin image without CORS, or
// no DOM.
export function sampleOfImage(img) {
  if (typeof document === "undefined" || !img) return null;
  try {
    const size = 24;
    const c = document.createElement("canvas");
    c.width = size;
    c.height = size;
    const ctx = c.getContext("2d", { willReadFrequently: true });
    ctx.drawImage(img, 0, 0, size, size);
    const { data } = ctx.getImageData(0, 0, size, size);
    let best = FALLBACK;
    let bestScore = -1;
    let r = 0;
    let g = 0;
    let b = 0;
    let n = 0;
    for (let i = 0; i < data.length; i += 4) {
      const a = data[i + 3];
      if (a < 200) continue;
      const cr = data[i];
      const cg = data[i + 1];
      const cb = data[i + 2];
      r += cr;
      g += cg;
      b += cb;
      n++;
      // Prefer saturated, mid-bright pixels for the accent.
      const max = Math.max(cr, cg, cb);
      const min = Math.min(cr, cg, cb);
      const sat = max === 0 ? 0 : (max - min) / max;
      const lum = max / 255;
      const score = sat * (1 - Math.abs(lum - 0.55));
      if (score > bestScore) {
        bestScore = score;
        best = [cr, cg, cb];
      }
    }
    if (!n) return { color: FALLBACK, factor: 1 };
    const average = [r / n, g / n, b / n].map(Math.round);
    // If the image is basically grayscale, use the average instead.
    return {
      color: bestScore > 0.15 ? best : average,
      factor: levelFactor(average),
    };
  } catch {
    return null;
  }
}

export function dominantOfImage(img) {
  return sampleOfImage(img)?.color ?? null;
}

export function dominantColor(url) {
  if (!url) return Promise.resolve(FALLBACK);
  if (cache.has(url)) return Promise.resolve(cache.get(url));
  const running = inFlight.get(url);
  if (running) return running;
  // The canvas below is 24px — the tiny Deezer variant (a few KB) is plenty.
  // Fetching the full 500px in CORS mode re-downloaded the whole cover just
  // for this. Non-Deezer URLs (local art, blobs) pass through unchanged.
  const fetchUrl = loResCover(url, 96) || url;

  const p = new Promise((resolve) => {
    const img = new Image();
    // crossOrigin is only needed to un-taint the canvas for a CROSS-origin
    // source — and it actively hurts on our own routes: it drops the session
    // cookie, so the login-protected /api/cover and /api/localcover art came
    // back 401 and every local file's header fell back to the default purple.
    if (!sameOrigin(fetchUrl)) img.crossOrigin = "anonymous";
    img.referrerPolicy = "no-referrer";
    img.onload = () => {
      const found = dominantOfImage(img);
      if (!found) {
        resolve(FALLBACK);
        return;
      }
      remember(url, found);
      resolve(found);
    };
    // No colour to extract (CDN drop, 404): DON'T memoize the fallback — the
    // next visit should get a real colour once the art is reachable again.
    img.onerror = () => resolve(FALLBACK);
    img.src = fetchUrl;
  }).finally(() => inFlight.delete(url));
  inFlight.set(url, p);
  return p;
}

function sameOrigin(u) {
  if (!u) return true;
  if (u.startsWith("blob:") || u.startsWith("data:")) return true;
  try {
    return new URL(u, window.location.href).origin === window.location.origin;
  } catch {
    return true;
  }
}

export function rgb([r, g, b], a = 1) {
  return `rgba(${r | 0}, ${g | 0}, ${b | 0}, ${a})`;
}

// Darken a color toward black by `f` (0..1) for readable gradients.
export function darken([r, g, b], f = 0.5) {
  return [r * (1 - f), g * (1 - f), b * (1 - f)];
}
