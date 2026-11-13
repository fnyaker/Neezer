// The Sillon themes: Encre (the default, dark with a lemon accent), Papier
// (warm light, ink accent) and Pochette (dark, and the accent IS the colour of
// the sleeve that is playing).
//
// The theme is an attribute on <html>, so a switch is one attribute write and
// no component has to know a theme exists. index.html carries the same read
// inline, BEFORE the first paint, so a reload in Papier never flashes black;
// this module then owns the rest of the session (localStorage, the browser's
// own chrome colour, and Pochette's accent).
//
// No new store library, no server round-trip: the preference is a `ui.*` key
// like the rest of this device's settings.
//
// A SECOND, OPTIONAL ACCENT: `ui.accent` holds a colour the person picked in
// Réglages › Apparence. It is a supplement to the theme, never a fourth theme:
// Encre and Papier paint it on <html> as inline custom properties, and Pochette
// keeps the sleeve's colour (its whole point). The inline properties are:
//   --ac / --btn   the exact chosen surface
//   --onac/--onbtn pure black or white, whichever carries >= 4.5:1 on it
//   --ac-text/--ac-mark/--hrt  a legible INK variant, because those tokens are
//                 consumed as foreground colours (focus rings, hearts,
//                 progress bars) and a dark custom accent would vanish on
//                 Encre. Papier already separates its lemon surface from its
//                 ink text/mark for the same reason.
// The correction is never silent: AppearanceSettings says when the ink had to
// be lifted/darkened. theme-boot.js repeats this maths before the first frame
// (a classic script cannot import this module); a test runs both against the
// same colours so they cannot drift.

import { writable, get } from "svelte/store";
import { current, currentId } from "./stores.js";
import { dominantColor } from "./color.js";
import {
  ACCENT_FALLBACK,
  accentFor,
  contrast,
  fromHex,
  luminance,
  readableAccent,
  toHex,
} from "./sillon.js";

export const THEMES = ["encre", "papier", "pochette"];
export const DEFAULT_THEME = "encre";

// Labels and blurbs live with the theme so Réglages and any other surface show
// the same words (DESIGN, Réglages › Apparence).
export const THEME_INFO = {
  encre: { label: "Encre", hint: "Sombre, accent citron. Le défaut." },
  papier: { label: "Papier", hint: "Clair chaud, encre noire, citron en surlignage." },
  pochette: { label: "Pochette", hint: "Sombre, l'accent suit la pochette qui joue." },
};

// The accent a theme paints before the person chooses one. This is the value
// the custom colour field starts on, and what "Réinitialiser" goes back to.
export const THEME_ACCENT = {
  encre: "#D2FF3C",
  papier: "#14130F",
  pochette: ACCENT_FALLBACK,
};

// The colour the browser paints its own chrome with (Android's status bar, the
// address bar): the theme's page background, never the accent.
const BACKGROUND = { encre: "#0C0C0B", papier: "#F3F0E8", pochette: "#0F0E10" };
// The surface the accent INK has to survive. --ac-text/--ac-mark/--hrt are
// drawn on the page AND on the raised cards/chips, so the ink is corrected
// against the most demanding one: Encre's lightest surface (--s2) and Papier's
// lightest (--s1). Using --bg alone let a mid violet pass at 4.50:1 on the page
// and fall to 3.7:1 on a card.
const INK_BG = { encre: "#1E1E1B", papier: "#FBF9F4", pochette: "#0F0E10" };

const KEY = "ui.theme";
// The last accent Pochette painted, kept so the reload's first frame already
// has the right colour instead of the orange fallback (index.html reads it).
const ACCENT_KEY = "ui.coverAc";
// The person's own accent. JSON like every other `ui.*` value, but a raw string
// from an older build or a hand-edited one is accepted too.
const CUSTOM_KEY = "ui.accent";

const WHITE = "#FFFFFF";
const BLACK = "#000000";
const MIN_CONTRAST = 4.5;
// Every property apply() owns. Clearing the whole list before painting is what
// makes "Réinitialiser", a theme switch and Pochette safe: no stale inline
// value can outlive the theme it belonged to.
const CUSTOM_PROPS = [
  "--ac",
  "--btn",
  "--hrt",
  "--onac",
  "--onbtn",
  "--ac-text",
  "--ac-mark",
];

function readTheme() {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return DEFAULT_THEME;
    // Written with JSON.stringify like every other `ui.*` preference; a raw
    // string from an older build (or a hand-edited one) is accepted too.
    let v = raw;
    try {
      v = JSON.parse(raw);
    } catch {
      v = raw.replace(/^"|"$/g, "");
    }
    return THEMES.includes(v) ? v : DEFAULT_THEME;
  } catch {
    return DEFAULT_THEME;
  }
}

function readAccent() {
  try {
    const raw = localStorage.getItem(ACCENT_KEY);
    if (!raw) return null;
    let v = raw;
    try {
      v = JSON.parse(raw);
    } catch {
      v = raw.replace(/^"|"$/g, "");
    }
    return /^#[0-9a-f]{6}$/i.test(v) ? v : null;
  } catch {
    return null;
  }
}

// Accepts "#D2FF3C", "d2ff3c", "#d2f" (the browser's own shorthand) and
// returns the canonical "#D2FF3C"; anything else is null. Shared by the field,
// the store and the tests, so a value that reaches localStorage has already
// passed the only shape the theme can paint.
export function normalizeAccent(raw) {
  if (typeof raw !== "string") return null;
  let s = raw.trim().replace(/^#/, "");
  if (/^[0-9a-f]{3}$/i.test(s)) s = s[0] + s[0] + s[1] + s[1] + s[2] + s[2];
  return /^[0-9a-f]{6}$/i.test(s) ? "#" + s.toUpperCase() : null;
}

function readCustomAccent() {
  try {
    const raw = localStorage.getItem(CUSTOM_KEY);
    if (!raw) return null;
    let v = raw;
    try {
      v = JSON.parse(raw);
    } catch {
      v = raw.replace(/^"|"$/g, "");
    }
    return normalizeAccent(v);
  } catch {
    return null;
  }
}

const clamp8 = (v) => Math.max(0, Math.min(255, Math.round(v)));

// Make a colour legible as INK on `background`: lighten it on a dark theme
// (the same multiplication-readableAccent() uses, which keeps the hue), darken
// it on a light one. Returns the adjusted channels and whether it moved.
function inkFor(rgb, background) {
  const bg = fromHex(background);
  if (contrast(rgb, bg) >= MIN_CONTRAST) return { rgb, adjusted: false };
  let last = rgb;
  if (luminance(bg) > 0.5) {
    // Light theme: scale down toward black, then mix black in for a colour
    // whose channels cannot be scaled far enough (pure white, a pale pastel).
    for (let s = 0.95; s >= 0.0499; s -= 0.05) {
      last = rgb.map((v) => clamp8(v * s));
      if (contrast(last, bg) >= MIN_CONTRAST) return { rgb: last, adjusted: true };
    }
    for (let k = 0.05; k <= 1.0001; k += 0.05) {
      const mixed = last.map((v) => clamp8(v * (1 - k)));
      if (contrast(mixed, bg) >= MIN_CONTRAST) return { rgb: mixed, adjusted: true };
    }
    return { rgb: [0, 0, 0], adjusted: true };
  }
  // Dark theme: readableAccent already walks the scale then the white blend.
  return { rgb: readableAccent(rgb, background, MIN_CONTRAST), adjusted: true };
}

// The whole pair a custom accent needs on a theme, as pure numbers. `null` for
// an invalid colour. The UI, apply() and the tests all read this one rule.
export function accentVars(raw, name = DEFAULT_THEME) {
  const surface = normalizeAccent(raw);
  if (!surface) return null;
  const theme = BACKGROUND[name] ? name : DEFAULT_THEME;
  // The ink is corrected against the most demanding surface it is drawn on,
  // not the page background (see INK_BG).
  const background = INK_BG[theme];
  const rgb = fromHex(surface);
  // Pure black or pure white, whichever wins: the midpoint of the WCAG range
  // still gives 4.58:1 to one of them, so text on the surface can never be
  // illegible (a near-black #0C0C0B would miss 4.5 on a mid grey).
  const on = contrast(rgb, [0, 0, 0]) >= contrast(rgb, [255, 255, 255]) ? BLACK : WHITE;
  const ink = inkFor(rgb, background);
  return {
    surface,
    on,
    ink: toHex(ink.rgb),
    background,
    adjusted: ink.adjusted,
    tooDark: ink.adjusted && luminance(fromHex(background)) < 0.5,
    tooLight: ink.adjusted && luminance(fromHex(background)) > 0.5,
  };
}

export function themeAccentOf(name) {
  return THEME_ACCENT[name] || THEME_ACCENT[DEFAULT_THEME];
}

export const theme = writable(readTheme());
// Read once at module load: theme-boot.js has already painted it, and initTheme
// re-applies it through the subscription below as soon as the app owns the page.
export const customAccent = writable(readCustomAccent());

function apply(name) {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  const bg = BACKGROUND[name] || BACKGROUND[DEFAULT_THEME];
  root.dataset.theme = name;
  // Also inline: it is what shows in the overscroll edges and behind a
  // translucent status bar, and index.html has already painted it (see its
  // pre-paint script) — the two must not disagree after a switch.
  root.style.backgroundColor = bg;
  const meta = document.querySelector('meta[name="theme-color"]');
  if (meta) meta.setAttribute("content", bg);

  // A custom accent is a property of the FIXED-accent themes only. Pochette's
  // --ac is var(--cover-ac); writing an inline --ac there would freeze the
  // sleeve's colour forever, so the whole block is cleared instead. --cover-ac
  // is set by setAccent() below and never touched here.
  for (const prop of CUSTOM_PROPS) root.style.removeProperty(prop);
  if (name === "pochette") return;
  const vars = accentVars(get(customAccent), name);
  if (!vars) return;
  root.style.setProperty("--ac", vars.surface);
  root.style.setProperty("--btn", vars.surface);
  // --hrt is a FOREGROUND (the heart is `color: var(--hrt)`), so it takes the
  // legible ink variant, exactly as Papier's ink heart already does.
  root.style.setProperty("--hrt", vars.ink);
  root.style.setProperty("--onac", vars.on);
  root.style.setProperty("--onbtn", vars.on);
  root.style.setProperty("--ac-text", vars.ink);
  root.style.setProperty("--ac-mark", vars.ink);
}

// The store is the one place the DOM hears about a theme: setting it (from
// Réglages tomorrow) repaints even if a caller forgets setTheme().
theme.subscribe(apply);
// The custom accent rides on top of the current theme; its own subscription
// also covers a reset (null) and a value written by another surface.
customAccent.subscribe(() => apply(get(theme)));

export function setTheme(name) {
  const next = THEMES.includes(name) ? name : DEFAULT_THEME;
  try {
    localStorage.setItem(KEY, JSON.stringify(next));
  } catch {
    /* private mode: the theme still applies to this session */
  }
  theme.set(next);
  if (next === "pochette") paintAccent();
}

// -- the person's accent -----------------------------------------------------

// Returns the canonical colour (or null when the input is not a colour). The
// store update repaints through the subscription above; the localStorage write
// is what theme-boot.js finds on the next launch.
export function setCustomAccent(raw) {
  const hex = normalizeAccent(raw);
  if (!hex) return null;
  try {
    localStorage.setItem(CUSTOM_KEY, JSON.stringify(hex));
  } catch {
    /* private mode: the colour still applies to this session */
  }
  customAccent.set(hex);
  return hex;
}

// "Réinitialiser": back to the theme's own accent. Removes the key rather than
// storing a copy of the theme colour, so a future theme change carries.
export function clearCustomAccent() {
  try {
    localStorage.removeItem(CUSTOM_KEY);
  } catch {
    /* ignore */
  }
  customAccent.set(null);
}

// -- Pochette: the accent is the sleeve's colour -----------------------------

// A track carries its art on its album (`album.cover`), a podcast episode on
// itself; a catalogue result may bring either.
function coverOf(track) {
  return track?.album?.cover || track?.cover || track?.coverArt || track?.picture || null;
}

let paintedAccent = null;
// The sleeve currently asked for (a URL, or null). Guarding on the URL rather
// than on a counter is what makes the queue's own churn harmless: replacing a
// queue can emit current = A, then null, then A again, and a counter would
// throw away A's colour and leave the fallback up with nothing to re-ask.
let wantedSleeve = Symbol("none");

function setAccent(hex) {
  if (typeof document === "undefined" || hex === paintedAccent) return;
  paintedAccent = hex;
  document.documentElement.style.setProperty("--cover-ac", hex);
  try {
    localStorage.setItem(ACCENT_KEY, JSON.stringify(hex));
  } catch {
    /* ignore */
  }
}

function paintAccent() {
  const url = coverOf(get(current));
  if (url === wantedSleeve) return;
  wantedSleeve = url;
  // A sleeve-less title (a local file without art) costs NO request: the
  // designed fallback is the answer.
  if (!url) {
    setAccent(ACCENT_FALLBACK);
    return;
  }
  dominantColor(url)
    .then((rgb) => {
      // A slower extraction must never overwrite a newer sleeve's colour, or a
      // theme switch made while it was in flight.
      if (url !== wantedSleeve || get(theme) !== "pochette") return;
      setAccent(accentFor(rgb));
    })
    .catch(() => {
      if (url === wantedSleeve && get(theme) === "pochette") setAccent(ACCENT_FALLBACK);
    });
}

let watching = false;
let lastTrackKey = Symbol("unset");

export function initTheme() {
  if (typeof document === "undefined") return;
  const name = readTheme();
  theme.set(name); // apply() runs through the subscription, including the meta
  // The stored accent colours the first frame; the current track refines it.
  const stored = readAccent();
  if (stored) setAccent(stored);
  if (watching) return;
  watching = true;
  // ONE listener for the app's lifetime (initTheme is idempotent), keyed on the
  // track's ID — a STRING. `current` hands over the store's object again on
  // every progress tick, and a listener reading it directly asked for the
  // sleeve's colour four times a second (the credits panel's lesson, webapp.md).
  currentId.subscribe((id) => {
    if (id === lastTrackKey) return;
    lastTrackKey = id;
    if (get(theme) === "pochette") paintAccent();
  });
}
