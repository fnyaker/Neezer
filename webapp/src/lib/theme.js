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

import { writable, get } from "svelte/store";
import { current, currentId } from "./stores.js";
import { dominantColor } from "./color.js";
import { ACCENT_FALLBACK, accentFor } from "./sillon.js";

export const THEMES = ["encre", "papier", "pochette"];
export const DEFAULT_THEME = "encre";

// Labels and blurbs live with the theme so Réglages and any other surface show
// the same words (DESIGN, Réglages › Apparence).
export const THEME_INFO = {
  encre: { label: "Encre", hint: "Sombre, accent citron. Le défaut." },
  papier: { label: "Papier", hint: "Clair chaud, encre noire, citron en surlignage." },
  pochette: { label: "Pochette", hint: "Sombre, l'accent suit la pochette qui joue." },
};

// The colour the browser paints its own chrome with (Android's status bar, the
// address bar): the theme's page background, never the accent.
const BACKGROUND = { encre: "#0C0C0B", papier: "#F3F0E8", pochette: "#0F0E10" };

const KEY = "ui.theme";
// The last accent Pochette painted, kept so the reload's first frame already
// has the right colour instead of the orange fallback (index.html reads it).
const ACCENT_KEY = "ui.coverAc";

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

export const theme = writable(readTheme());

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
}

// The store is the one place the DOM hears about a theme: setting it (from
// Réglages tomorrow) repaints even if a caller forgets setTheme().
theme.subscribe(apply);

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
