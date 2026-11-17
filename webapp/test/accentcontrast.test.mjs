// The ACCENT PAIR, held to WCAG AA everywhere it is painted.
//
// Two pairs carry every accent:
//   * text ON the accent surface — primary buttons, pills, badges, the selected
//     chip: `--onac` on `--ac`, `--onbtn` on `--btn`;
//   * the accent AS INK — `--ac-text` / `--ac-mark` / `--hrt` drawn on the page
//     (`--bg`), a card (`--s1`) and a chip (`--s2`).
//
// The second pair is the hole this test exists for. The SPA corrected a custom
// accent's ink against ONE surface (Papier's lightest, the easiest one for dark
// ink), which left #7C5CFF's ink at **4.18:1 on Papier's --bg and 3.74:1 on its
// --s2** — under AA on two of the three surfaces it paints. lib/theme.js now
// folds the ink over all three (the walk is monotone: a light theme only
// darkens, a dark theme only lightens), and public/theme-boot.js repeats the
// same fold before the first frame.
//
// The sweep covers every accent source — each theme's own accent, a custom
// colour over the whole hue circle (36 hues × 5 saturations × 9 lightnesses,
// greys included), and Pochette's sleeve lift — on all three themes.

import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import {
  ACCENT_FALLBACK,
  accentFor,
  contrast,
  fromHex,
  toHex,
} from "../src/lib/sillon.js";

// lib/theme.js reads localStorage at import and writes <html> through its
// store subscriptions. The same four-line DOM as test/sillon.test.mjs.
const fakeStorage = new Map();
const fakeProps = new Map();
const noop = () => {};
const fakeMeta = { attrs: {}, setAttribute(k, v) { this.attrs[k] = String(v); } };
globalThis.document = {
  addEventListener: noop,
  removeEventListener: noop,
  visibilityState: "visible",
  documentElement: {
    dataset: {},
    style: {
      backgroundColor: "",
      setProperty(k, v) {
        fakeProps.set(k, String(v));
      },
      removeProperty(k) {
        fakeProps.delete(k);
      },
    },
  },
  querySelector: (sel) => (String(sel).includes("theme-color") ? fakeMeta : null),
};
globalThis.location = { hash: "", pathname: "/" };
globalThis.window = {
  addEventListener: noop,
  removeEventListener: noop,
  location: globalThis.location,
  matchMedia: () => ({ matches: false, addEventListener: noop }),
  innerWidth: 1280,
  innerHeight: 900,
};
globalThis.localStorage = {
  getItem: (k) => (fakeStorage.has(k) ? fakeStorage.get(k) : null),
  setItem: (k, v) => fakeStorage.set(k, String(v)),
  removeItem: (k) => fakeStorage.delete(k),
  clear: () => fakeStorage.clear(),
};

const themeLib = await import("../src/lib/theme.js");

const THEMES = ["encre", "papier", "pochette"];
const MIN = 4.5;

// -- the theme's real tokens, read off app.css -------------------------------
//
// The test must hold the tokens that actually ship, not a second copy of them.
const APP_CSS = readFileSync(new URL("../src/app.css", import.meta.url), "utf8");

function tokensFor(theme) {
  const at = APP_CSS.indexOf(`[data-theme="${theme}"]`);
  assert.ok(at >= 0, `app.css has no [data-theme="${theme}"] block`);
  const open = APP_CSS.indexOf("{", at);
  const close = APP_CSS.indexOf("}", open);
  assert.ok(open > at && close > open, `app.css's ${theme} block has no body`);
  const tokens = {};
  for (const m of APP_CSS.slice(open + 1, close).matchAll(/--([a-z0-9-]+)\s*:\s*([^;]+);/gi)) {
    tokens[m[1]] = m[2].trim();
  }
  return tokens;
}

const TOKENS = Object.fromEntries(THEMES.map((t) => [t, tokensFor(t)]));

// One level of `var(--x)` / `var(--x, fallback)` resolution, which is all the
// theme blocks use (Pochette's --ac, --ac-text, --ac-mark and --hrt).
function resolve(tokens, value, depth = 0) {
  const m = /^var\(\s*--([\w-]+)\s*(?:,\s*([^)]+))?\)$/.exec(String(value).trim());
  if (!m || depth > 4) return String(value).trim();
  const inner = tokens[m[1]];
  if (inner == null) return m[2] ? m[2].trim() : String(value).trim();
  return resolve(tokens, inner, depth + 1);
}

const surfaceOf = (theme, name) => resolve(TOKENS[theme], TOKENS[theme][name]);

// -- the accent grid ---------------------------------------------------------

// The whole hue circle, at every lightness that changes which ink wins.
function hslToHex(h, s, l) {
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = l - c / 2;
  const [r, g, b] =
    h < 60 ? [c, x, 0]
    : h < 120 ? [x, c, 0]
    : h < 180 ? [0, c, x]
    : h < 240 ? [0, x, c]
    : h < 300 ? [x, 0, c]
    : [c, 0, x];
  return toHex([(r + m) * 255, (g + m) * 255, (b + m) * 255]);
}

const HUE_CIRCLE = [
  ...new Set(
    Array.from({ length: 36 }, (_, i) => i * 10).flatMap((hue) =>
      [0, 0.25, 0.5, 0.75, 1].flatMap((sat) =>
        [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9].map((light) =>
          hslToHex(hue, sat, light)
        )
      )
    )
  ),
];

// Every grey is its own worst case for pure black/white text; the sweep above
// only reaches nine of them.
const GREYS = Array.from({ length: 52 }, (_, i) => toHex([i * 5, i * 5, i * 5]));
const ACCENTS = [...new Set([...HUE_CIRCLE, ...GREYS])];

// -- text ON the accent ------------------------------------------------------

test("a custom accent's label clears 4.5:1 at every hue, on Encre and Papier", () => {
  let worst = Infinity;
  let worstWhat = "";
  for (const theme of ["encre", "papier"]) {
    for (const hex of ACCENTS) {
      const pair = themeLib.accentVars(hex, theme);
      assert.ok(pair, `${hex} on ${theme} is not a colour`);
      assert.equal(pair.surface, hex, `${hex} on ${theme}: --ac must stay the chosen colour`);
      // The rule the task states: pick the light or the dark ink by WCAG
      // luminance, never a themed near-black that would miss on a mid grey.
      const black = contrast(fromHex(hex), [0, 0, 0]);
      const white = contrast(fromHex(hex), [255, 255, 255]);
      assert.equal(pair.on, black >= white ? "#000000" : "#FFFFFF", `${hex} on ${theme}`);
      const ratio = contrast(fromHex(pair.surface), fromHex(pair.on));
      if (ratio < worst) {
        worst = ratio;
        worstWhat = `${hex} on ${theme} with ${pair.on}`;
      }
      assert.ok(
        ratio >= MIN,
        `${hex} on ${theme}: label ${pair.on} on the accent only ${ratio.toFixed(2)}:1`
      );
      // And the same pair backs --btn/--onbtn.
      assert.ok(
        ratio >= MIN,
        `${hex} on ${theme}: the primary button's label is the same pair`
      );
    }
  }
  console.log(`# ${ACCENTS.length * 2} custom labels, worst ${worst.toFixed(2)}:1 (${worstWhat})`);
});

test("the accent's own ink clears 4.5:1 on bg, s1 AND s2, for every hue and theme", () => {
  let worst = Infinity;
  let worstWhat = "";
  let pairs = 0;
  const offenders = [];
  for (const theme of ["encre", "papier"]) {
    const surfaces = ["bg", "s1", "s2"].map((name) => ({ name, hex: surfaceOf(theme, name) }));
    for (const hex of ACCENTS) {
      const pair = themeLib.accentVars(hex, theme);
      for (const surface of surfaces) {
        const ratio = contrast(fromHex(pair.ink), fromHex(surface.hex));
        pairs++;
        if (ratio < worst) {
          worst = ratio;
          worstWhat = `${hex} on ${theme} → ${pair.ink} on --${surface.name} ${surface.hex}`;
        }
        if (ratio < MIN) {
          offenders.push({
            ratio,
            msg: `${hex} on ${theme}: accent ink ${pair.ink} on --${surface.name} (${surface.hex}) only ${ratio.toFixed(2)}:1`,
          });
        }
      }
    }
  }
  offenders.sort((a, b) => a.ratio - b.ratio);
  assert.equal(
    offenders.length,
    0,
    `${offenders.length} accent-ink pairs are under 4.5:1; worst:\n  ` +
      offenders.slice(0, 8).map((o) => o.msg).join("\n  ")
  );
  console.log(`# ${pairs} accent-ink pairs, worst ${worst.toFixed(2)}:1 (${worstWhat})`);
});

test("the measured hole: a mid accent's ink on all three Papier surfaces", () => {
  // The numbers the SPA shipped with: #7C5CFF's ink was corrected against
  // Papier's --s1 (the easiest surface for dark ink) and measured 4.18:1 on
  // --bg and 3.74:1 on --s2. A near-grey and a pale yellow failed too.
  const probes = [
    ["#7C5CFF", "violet moyen"],
    ["#808080", "gris moyen"],
    ["#FFF176", "jaune clair"],
  ];
  const offenders = [];
  for (const [hex, kind] of probes) {
    const pair = themeLib.accentVars(hex, "papier");
    for (const name of ["bg", "s1", "s2"]) {
      const surface = surfaceOf("papier", name);
      const ratio = contrast(fromHex(pair.ink), fromHex(surface));
      if (ratio < MIN) {
        offenders.push(
          `${kind} ${hex}: ink ${pair.ink} on Papier --${name} (${surface}) only ${ratio.toFixed(2)}:1`
        );
      }
    }
  }
  assert.deepEqual(
    offenders,
    [],
    `the accent ink still falls under 4.5:1 on Papier:\n  ${offenders.join("\n  ")}`
  );
});

test("the accent's own colours clear 4.5:1, on every theme", () => {
  // The tokens app.css ships for the default accent of each theme: text on the
  // accent pill, text on the primary button, and the accent ink on the three
  // surfaces it paints.
  for (const theme of THEMES) {
    const tokens = TOKENS[theme];
    const pairs = [
      ["--onac on --ac", "onac", "ac"],
      ["--onbtn on --btn", "onbtn", "btn"],
    ];
    for (const [what, inkName, surfaceName] of pairs) {
      const ink = surfaceOf(theme, inkName);
      const surface = surfaceOf(theme, surfaceName);
      const ratio = contrast(fromHex(ink), fromHex(surface));
      assert.ok(
        ratio >= MIN,
        `${theme}: ${what} is ${ink} on ${surface}, only ${ratio.toFixed(2)}:1`
      );
    }
    // --ac-text / --ac-mark / --hrt are the accent as a FOREGROUND; the ones
    // that resolve to a plain colour (Pochette's --ac-mark is `var(--ac)`, a
    // surface-derived mark with its own lift) are held on the three surfaces.
    for (const inkName of ["ac-text", "ac-mark", "hrt"]) {
      const ink = surfaceOf(theme, inkName);
      if (!/^#[0-9a-f]{6}$/i.test(ink)) continue;
      for (const surfaceName of ["bg", "s1", "s2"]) {
        const surface = surfaceOf(theme, surfaceName);
        const ratio = contrast(fromHex(ink), fromHex(surface));
        assert.ok(
          ratio >= MIN,
          `${theme}: --${inkName} ${ink} on --${surfaceName} (${surface}) only ${ratio.toFixed(2)}:1`
        );
      }
    }
  }
});

test("Pochette's sleeve accent keeps its dark label at every hue", () => {
  // --onac on Pochette is the fixed near-black (sillon.js#ON_ACCENT), so the
  // accent itself is what accentFor lifts until that label clears 4.5:1.
  const onac = surfaceOf("pochette", "onac");
  let worst = Infinity;
  let worstWhat = "";
  for (const hex of [ACCENT_FALLBACK, ...HUE_CIRCLE]) {
    const accent = accentFor(fromHex(hex));
    const ratio = contrast(fromHex(accent), fromHex(onac));
    if (ratio < worst) {
      worst = ratio;
      worstWhat = `${hex} → ${accent}`;
    }
    assert.ok(
      ratio >= MIN,
      `sleeve ${hex} → ${accent}: label ${onac} only ${ratio.toFixed(2)}:1`
    );
  }
  console.log(`# ${HUE_CIRCLE.length + 1} sleeve accents, worst ${worst.toFixed(2)}:1 (${worstWhat})`);
});
