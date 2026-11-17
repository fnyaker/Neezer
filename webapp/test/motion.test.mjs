// The motion foundation's rules, read off the SOURCE plus the pure module —
// the part a browser cannot prove cheaply (the eco guard itself is measured in
// test/motion/run.mjs, in Chromium).
//
// What this file holds (docs/agents/transitions.md, lot T0's "Done when"):
//   1. the CSS `cubic-bezier()`s and lib/motion.js's solver agree to 1e-3;
//   2. the token table in app.css and the JS table are the same numbers, and
//      eco/reduced really neutralise them;
//   3. `screenKey` collapses exactly the four in-screen routes and nothing
//      else, on EVERY route App.svelte declares (a new route fails here until
//      it is written down);
//   4. `createMotion`'s three levels: eco wins over reduced, the OS setting is
//      listened to live (`change`), and "off" is Svelte's instant path;
//   5. every transition returns duration 0 when off, and no transform at all
//      under reduced motion (M13/M14);
//   6. the ONE-source-of-truth lint (M15): no file imports
//      `svelte/transition`/`svelte/animate` at all (T7 emptied the frozen list),
//      no `transition:`/`animation:` declaration and no transition directive
//      carries a literal duration except the named walks and the
//      status/decorative keyframes M13/M14 keep running in eco, and
//      lib/motion.js exports every transition by name.

import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { get, writable } from "svelte/store";

import {
  DUR,
  MOVE,
  SCALE,
  EASE,
  EASE_CSS,
  bezier,
  createMotion,
  motionLevel,
} from "../src/lib/motion.js";

const SRC = fileURLToPath(new URL("../src/", import.meta.url));
const APP_CSS = readFileSync(new URL("../src/app.css", import.meta.url), "utf8");
const APP_SVELTE = readFileSync(new URL("../src/App.svelte", import.meta.url), "utf8");

// -- helpers -----------------------------------------------------------------

/** Every rule of app.css, with the at-rule it lives under (a tiny scanner:
 *  clear comments, then walk the braces and keep the custom properties). */
function rules(css) {
  const clean = css.replace(/\/\*[\s\S]*?\*\//g, "");
  const out = [];
  const stack = [];
  let buf = "";
  for (const ch of clean) {
    if (ch === "{") {
      stack.push(buf.trim());
      buf = "";
    } else if (ch === "}") {
      const selector = stack.pop() || "";
      const media = stack.find((entry) => entry.startsWith("@media")) || null;
      const declarations = new Map();
      if (selector && !selector.startsWith("@")) {
        for (const m of buf.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) declarations.set(m[1], m[2].trim());
        if (declarations.size) out.push({ selector, media, declarations });
      }
      buf = "";
    } else {
      buf += ch;
    }
  }
  return out;
}

const RULES = rules(APP_CSS);

/** The merged custom properties of the rules matching `selector`/`media`. */
function tokens(selector, media = null) {
  const merged = new Map();
  for (const rule of RULES) {
    if (rule.selector !== selector) continue;
    if ((rule.media || null) !== media) continue;
    for (const [name, value] of rule.declarations) merged.set(name, value);
  }
  assert.ok(merged.size, `app.css has no "${selector}" rule${media ? ` under ${media}` : ""}`);
  return merged;
}

/** `cubic-bezier(0.3, 0, 0.8, 0.15)` → [0.3, 0, 0.8, 0.15] */
function curve(value) {
  const m = String(value).match(/^cubic-bezier\(\s*([-\d.]+)\s*,\s*([-\d.]+)\s*,\s*([-\d.]+)\s*,\s*([-\d.]+)\s*\)$/);
  assert.ok(m, `not a cubic-bezier(): ${value}`);
  return m.slice(1, 5).map(Number);
}

const ms = (value) => Number(String(value).replace(/ms$/, ""));
const px = (value) => Number(String(value).replace(/px$/, ""));
const num = (value) => Number(String(value));
const squash = (value) => String(value).replace(/\s+/g, "");

// An INDEPENDENT solver for the same curve: bisect t until x(t) is the x we
// were asked for, then read y(t). Deliberately not the Newton path the module
// uses — the point is that two different methods agree.
function referenceBezier(x1, y1, x2, y2, x) {
  const cx = (t) => 3 * (1 - t) * (1 - t) * t * x1 + 3 * (1 - t) * t * t * x2 + t * t * t;
  const cy = (t) => 3 * (1 - t) * (1 - t) * t * y1 + 3 * (1 - t) * t * t * y2 + t * t * t;
  let lo = 0;
  let hi = 1;
  for (let i = 0; i < 200; i++) {
    const mid = (lo + hi) / 2;
    if (cx(mid) < x) lo = mid;
    else hi = mid;
  }
  return cy((lo + hi) / 2);
}

/** A Svelte transition's config(s): a function is the per-direction form. */
function configs(result) {
  return typeof result === "function" ? ["in", "out"].map((direction) => result({ direction })) : [result];
}

/** Every css string a config would emit, sampled the way Svelte samples it. */
function cssAt(config) {
  if (!config || typeof config.css !== "function") return "";
  let out = "";
  for (const t of [0, 0.25, 0.5, 0.75, 1]) out += config.css(t, 1 - t) || "";
  return out;
}

const NODE = { nodeType: 1, style: {}, clientWidth: 100, clientHeight: 50 };
const RECT = (left, top) => ({ left, top, width: 100, height: 40 });

/** A MediaQueryList stand-in that fires `change` like the real one. */
function fakeMq(matches) {
  const listeners = new Set();
  return {
    matches,
    addEventListener(type, fn) {
      if (type === "change") listeners.add(fn);
    },
    removeEventListener(type, fn) {
      listeners.delete(fn);
    },
    set(next) {
      this.matches = next;
      for (const fn of [...listeners]) fn({ matches: next });
    },
  };
}

/** Run `fn` with a phone-sized window (lib/viewport.js#isMobile reads it). */
function asPhone(fn) {
  const had = Object.prototype.hasOwnProperty.call(globalThis, "window");
  const previous = globalThis.window;
  globalThis.window = { matchMedia: () => ({ matches: true }) };
  try {
    return fn();
  } finally {
    if (had) globalThis.window = previous;
    else delete globalThis.window;
  }
}

// -- 1. the curves -----------------------------------------------------------

test("every EASE function is the CSS curve it names, to 1e-3", () => {
  // The number the plan asks for, spelled out: EASE.enter(0.5).
  const half = referenceBezier(0.05, 0.7, 0.1, 1, 0.5);
  assert.ok(Math.abs(EASE.enter(0.5) - half) < 1e-3, `EASE.enter(0.5) = ${EASE.enter(0.5)}, CSS ${half}`);
  console.log(`# EASE.enter(0.5) = ${EASE.enter(0.5).toFixed(6)} (CSS ${half.toFixed(6)})`);

  for (const name of ["enter", "exit", "standard"]) {
    const [x1, y1, x2, y2] = curve(EASE_CSS[name]);
    let worst = 0;
    for (let x = 0.05; x < 1; x += 0.05) {
      const expected = referenceBezier(x1, y1, x2, y2, x);
      worst = Math.max(worst, Math.abs(EASE[name](x) - expected));
    }
    assert.ok(worst < 1e-3, `${name}: worst |JS - CSS| = ${worst}`);
    console.log(`# ${name}: worst |JS - CSS| over 19 samples = ${worst.toExponential(2)}`);
    assert.equal(EASE[name](0), 0, `${name}(0) is 0`);
    assert.equal(EASE[name](1), 1, `${name}(1) is 1`);
  }

  // The solver is exported, so a new curve can be checked the same way.
  const custom = bezier(0.4, 0, 0.2, 1);
  assert.ok(Math.abs(custom(0.3) - referenceBezier(0.4, 0, 0.2, 1, 0.3)) < 1e-3);
});

// -- 2. the token table ------------------------------------------------------

test("the CSS motion tokens are the JS table", () => {
  const root = tokens(":root");
  const durations = {
    instant: "--dur-instant",
    fast: "--dur-fast",
    base: "--dur-base",
    sheet: "--dur-sheet",
    sheetOut: "--dur-sheet-out",
    screen: "--dur-screen",
    screenOut: "--dur-screen-out",
  };
  for (const [name, token] of Object.entries(durations)) {
    assert.ok(root.has(token), `app.css declares ${token}`);
    assert.equal(ms(root.get(token)), DUR[name], `${token} = DUR.${name}`);
  }
  assert.equal(Object.keys(durations).length, Object.keys(DUR).length, "every DUR entry has a token");

  assert.equal(px(root.get("--move-s")), MOVE.s);
  assert.equal(px(root.get("--move-m")), MOVE.m);

  assert.equal(num(root.get("--scale-pop")), SCALE.pop);
  assert.equal(num(root.get("--scale-dialog")), SCALE.dialog);
  assert.equal(num(root.get("--scale-screen")), SCALE.screen);

  for (const name of ["enter", "exit", "standard"]) {
    const value = root.get(`--ease-${name}`);
    assert.ok(value, `app.css declares --ease-${name}`);
    assert.equal(squash(value), squash(EASE_CSS[name]), `--ease-${name} = EASE_CSS.${name}`);
    assert.deepEqual(curve(value), curve(EASE_CSS[name]));
  }
});

test("eco zeroes every duration, move and scale — the tokens ARE the switch", () => {
  const base = tokens(":root");
  const eco = tokens('html[data-eco="1"]');
  const reduced = tokens(":root", "@media (prefers-reduced-motion: reduce)");

  for (const token of ["--dur-instant", "--dur-fast", "--dur-base", "--dur-sheet", "--dur-sheet-out", "--dur-screen", "--dur-screen-out"]) {
    assert.equal(ms(eco.get(token)), 0, `eco: ${token} is 0ms`);
    // Reduced only has to override what is longer than the 140 ms cap.
    const effective = reduced.has(token) ? ms(reduced.get(token)) : ms(base.get(token));
    assert.ok(effective <= DUR.fast, `reduced: effective ${token} = ${effective}ms ≤ ${DUR.fast}ms`);
  }
  for (const token of ["--move-s", "--move-m"]) {
    assert.equal(px(eco.get(token)), 0, `eco: ${token} is 0px`);
    assert.equal(px(reduced.get(token)), 0, `reduced: ${token} is 0px`);
  }
  for (const token of ["--scale-pop", "--scale-dialog", "--scale-screen"]) {
    assert.equal(num(eco.get(token)), 1, `eco: ${token} is 1`);
    assert.equal(num(reduced.get(token)), 1, `reduced: ${token} is 1`);
  }
  // Reduced keeps a fade that is still short enough to read as feedback.
  assert.equal(ms(reduced.get("--dur-base")), DUR.fast, "reduced: a 200 ms enter becomes 140 ms");
});

test("the eco guard stops every CSS transition and keyframe animation", () => {
  // The selector has to be the documented one: three `:not([data-motion="status"])`
  // arms (the element, ::before, ::after), because a frozen spinner reads as a hang.
  const arms = APP_CSS.match(/html\[data-eco="1"\] :not\(\[data-motion="status"\]\)(::before|::after)?/g) || [];
  assert.equal(arms.length, 3, "the guard covers the element and both pseudo-elements");

  const at = APP_CSS.indexOf('html[data-eco="1"] :not([data-motion="status"])');
  const open = APP_CSS.indexOf("{", at);
  const rule = APP_CSS.slice(open, APP_CSS.indexOf("}", open) + 1);
  for (const declaration of [
    "transition-duration: 0.01ms !important;",
    "transition-delay: 0s !important;",
    "animation-duration: 0.01ms !important;",
    "animation-delay: 0s !important;",
    "animation-iteration-count: 1 !important;",
    "scroll-behavior: auto !important;",
  ]) {
    assert.ok(rule.includes(declaration), `the guard declares ${declaration}`);
  }
  // 0.01ms, never 0s: a 0s transition fires no `transitionend`.
  assert.ok(!/transition-duration:\s*0s\s*!important/.test(rule));
});

// -- 3. screenKey ------------------------------------------------------------

test("screenKey collapses the four in-screen routes on every route the shell declares", () => {
  // Every route in `const routes = {...}` of App.svelte, with its page key. A
  // route added to the shell without an entry here fails the set comparison
  // below — that is the point: "/library…", "/settings…", "/search…" and
  // "/import…" write their sub-paths with `replace()`, so they are ONE screen,
  // and everything else is a page per path.
  const EXPECTED = {
    "/": "/",
    "/search": "/search",
    "/search/x": "/search",
    "/artist/x": "/artist/x",
    "/album/x": "/album/x",
    "/playlist/x": "/playlist/x",
    "/library": "/library",
    "/library/x": "/library",
    "/podcasts": "/podcasts",
    "/podcast/x": "/podcast/x",
    "/settings": "/settings",
    "/genres": "/genres",
    "/import": "/import",
    "/import/x": "/import",
    "/releases": "/releases",
    "/releases/artists": "/releases/artists",
    "/releases/discovery": "/releases/discovery",
    "/credits/x": "/credits/x",
    "/recap": "/recap",
    "/recap/x": "/recap/x",
  };

  const routes = APP_SVELTE.slice(APP_SVELTE.indexOf("const routes = {"));
  const declared = [...routes.match(/^ {4}"([^"]+)":/gm)].map((line) =>
    line.trim().replace(/^"/g, "").replace(/":$/g, ""),
  );
  assert.ok(declared.length >= 18, `parsed ${declared.length} routes from App.svelte`);

  // A declared route's concrete path: `:param` → x, `:param?` → nothing.
  const concrete = declared.map((route) => route.replace(/\/:\w+\?/g, "").replace(/\/:\w+/g, "/x"));
  assert.deepEqual(
    [...concrete].sort(),
    [...Object.keys(EXPECTED)].sort(),
    "every route App.svelte declares has a page key written down here",
  );

  const { screenKey } = createMotion({});
  for (const [path, key] of Object.entries(EXPECTED)) {
    assert.equal(screenKey(path), key, `screenKey(${JSON.stringify(path)})`);
  }

  // The rule itself, beyond the table: a prefix is a SEGMENT, and a sibling
  // leaf of a collapsed screen is still part of it.
  assert.equal(screenKey("/library/artists"), "/library");
  assert.equal(screenKey("/library/artists/2026"), "/library");
  assert.equal(screenKey("/settings/playback"), "/settings");
  assert.equal(screenKey("/search/daft%20punk"), "/search");
  assert.equal(screenKey("/import/https%3A%2F%2Fx"), "/import");
  assert.equal(screenKey("/libraryX"), "/libraryX", "a prefix is not a substring");
  assert.equal(screenKey("/album/1/"), "/album/1", "a trailing slash is the same page");
  assert.equal(screenKey("/album/1?x=1#y"), "/album/1", "query and hash are not the path");
  assert.equal(screenKey("album/1"), "/album/1", "a bare path is normalised");
  assert.notEqual(screenKey("/album/1"), screenKey("/album/2"), "two albums are two pages");
  assert.notEqual(screenKey("/playlist/1"), screenKey("/playlist/2"));
});

// -- 4. the levels -----------------------------------------------------------

test("createMotion: eco wins over reduced, and the OS setting is LIVE", () => {
  const eco = writable(false);
  const mq = fakeMq(false);
  const motion = createMotion({ eco, reduceMq: mq });

  assert.equal(motion.level(), "full");
  assert.equal(get(motion.motionLevel), "full");

  eco.set(true);
  assert.equal(motion.level(), "off", "eco switches everything off");

  mq.set(true);
  assert.equal(motion.level(), "off", "reduced motion does NOT override eco");

  eco.set(false);
  assert.equal(motion.level(), "reduced", "with eco off, the OS setting is what is left");

  mq.set(false);
  assert.equal(motion.level(), "full", "the change event is listened to, not read once");

  // A MediaQueryList that starts already reduced is read at creation.
  const alreadyReduced = createMotion({ eco: writable(false), reduceMq: fakeMq(true) });
  assert.equal(alreadyReduced.level(), "reduced");

  // No sources at all (SSR, a test, an old WebView): always full.
  assert.equal(createMotion({}).level(), "full");
  assert.equal(createMotion({ eco: writable(false) }).level(), "full");

  // The module's own instance is a readable store wired to stores.js#ecoMode:
  // in Node nothing is on, so it reports "full" without a window.
  assert.equal(get(motionLevel), "full");
});

test("level decides scrolling: smooth only at full motion", () => {
  const eco = writable(false);
  const mq = fakeMq(false);
  const motion = createMotion({ eco, reduceMq: mq });
  assert.equal(motion.scrollBehavior(), "smooth");
  mq.set(true);
  assert.equal(motion.scrollBehavior(), "auto");
  eco.set(true);
  assert.equal(motion.scrollBehavior(), "auto");
});

// -- 5. every transition, off and reduced ------------------------------------

const CASES = [
  ["fadeIn", (m) => m.fadeIn(NODE, {})],
  ["fade", (m) => m.fade(NODE, {})],
  ["rise", (m) => m.rise(NODE, {})],
  ["swap", (m) => m.swap(NODE, {})],
  ["pop", (m) => m.pop(NODE, { origin: "top right", placement: "down" })],
  ["dialog", (m) => m.dialog(NODE, {})],
  ["sheet", (m) => m.sheet(NODE, {})],
  ["veil", (m) => m.veil(NODE, { for: "sheet", color: "rgba(0, 0, 0, 0.6)" })],
  ["panel", (m) => m.panel(NODE, {})],
  ["screen", (m) => m.screen(NODE, {})],
  ["collapse", (m) => m.collapse(NODE, {})],
  ["flipSmall", (m) => m.flipSmall(NODE, { from: RECT(0, 0), to: RECT(0, 40) })],
];

test("off (eco) is Svelte's instant path for every transition", () => {
  const motion = createMotion({ eco: writable(true), reduceMq: fakeMq(false) });
  for (const [name, call] of CASES) {
    for (const config of configs(call(motion))) {
      assert.equal(config.duration, 0, `${name}: duration 0 when eco is on`);
      assert.equal(config.css, undefined, `${name}: nothing is written when eco is on`);
    }
  }

  // The help the shell calls directly is silent too.
  assert.equal(motion.animate({ animate: () => assert.fail("no Animation in eco") }, [{ opacity: 0 }, { opacity: 1 }]), null);
  const el = { animate: () => assert.fail("no Animation in eco"), children: [{ animate: () => assert.fail("no row shift in eco") }] };
  assert.deepEqual(motion.shiftRows(el, 0, -1, 56), []);
  assert.equal(motion.pageEnter({ animate: () => assert.fail("no page enter in eco") }, {}), null);
});

test("reduced motion removes every displacement and caps the fade at 140 ms", () => {
  const motion = createMotion({ eco: writable(false), reduceMq: fakeMq(true) });
  for (const [name, call] of CASES) {
    for (const config of configs(call(motion))) {
      assert.ok(config.duration <= DUR.fast, `${name}: ${config.duration} ms ≤ ${DUR.fast}`);
      const css = cssAt(config);
      assert.ok(!/(^|[;:\s])(transform|translate|scale|rotate)\s*:/i.test(css), `${name}: no transform under reduced motion — got "${css}"`);
    }
  }

  // The imperative half drops the movement and the duration, and keeps the fade.
  const seen = [];
  const el = { animate: (frames, options) => (seen.push({ frames, options }), { cancel() {} }) };
  motion.animate(el, [{ opacity: 0, transform: "translateY(8px)" }, { opacity: 1, transform: "none" }], { duration: 320 });
  assert.deepEqual(seen.at(-1).frames, [{ opacity: 0 }, { opacity: 1 }]);
  assert.equal(seen.at(-1).options.duration, DUR.fast);

  motion.pageEnter(el, { traversal: false });
  assert.deepEqual(seen.at(-1).frames, [{ opacity: 0 }, { opacity: 1 }], "a page enter is a fade when reduced");
  assert.equal(seen.at(-1).options.duration, DUR.fast);

  // A row shift has nothing left to show: it is cancelled, not translated.
  assert.equal(motion.shiftRows({ children: [{ animate: () => assert.fail("no row shift when reduced") }] }, 0, -1, 56).length, 0);
});

test("full motion keeps the plan's numbers and moves by transform only", () => {
  const motion = createMotion({ eco: writable(false), reduceMq: fakeMq(false) });

  const fadeIn = motion.fadeIn(NODE, {});
  assert.equal(fadeIn.duration, DUR.fast);
  assert.equal(fadeIn.easing, EASE.enter);

  const [fadeInCfg, fadeOutCfg] = configs(motion.fade(NODE, {}));
  assert.equal(fadeInCfg.duration, DUR.base, "fade enters over 200 ms");
  assert.equal(fadeOutCfg.duration, DUR.fast, "fade leaves over 140 ms (M5: exits are shorter)");
  assert.equal(fadeOutCfg.easing, EASE.exit);

  const rise = configs(motion.rise(NODE, { y: -MOVE.m }))[0];
  assert.match(rise.css(0, 1), /translateY\(-8px\)/, "rise starts where it was asked to");
  assert.match(rise.css(1, 0), /opacity: 1/);

  const pop = configs(motion.pop(NODE, { origin: "top right", placement: "down" }))[0];
  assert.match(pop.css(0, 1), /transform-origin: top right/);
  assert.match(pop.css(0, 1), /scale\(0\.96\)/);
  assert.match(pop.css(0, 1), /translateY\(-4px\)/, "a menu below its anchor leans from above");

  const dialog = configs(motion.dialog(NODE, {}))[0];
  assert.match(dialog.css(0, 1), /scale\(0\.97\)/);

  // The phone shapes, which are the ones a desktop viewport never exercises.
  asPhone(() => {
    const sheetIn = configs(motion.sheet(NODE, {}))[0];
    assert.equal(sheetIn.duration, DUR.sheet);
    assert.match(sheetIn.css(0, 1), /translateY\(calc\(1 \* 100%\)\)/);
    assert.ok(!/opacity/.test(sheetIn.css(0, 1)), "a phone sheet carries glass: no opacity (M8)");
    assert.equal(configs(motion.sheet(NODE, {}))[1].duration, DUR.sheetOut);

    // A dragged sheet resumes from the finger, not from off-screen.
    assert.match(configs(motion.sheet(NODE, { fromY: 120 }))[0].css(0, 1), /translateY\(calc\(1 \* 120px\)\)/);

    assert.equal(configs(motion.screen(NODE, {}))[0].duration, DUR.screen);
    assert.equal(configs(motion.screen(NODE, {}))[1].duration, DUR.screenOut);
    assert.match(configs(motion.screen(NODE, { fromY: 90 }))[1].css(1, 0), /translateY\(calc\(0 \* 90px\)\)/);

    // And on a phone, reduced motion is "it appears": no slide is left to play.
    const reduced = createMotion({ eco: writable(false), reduceMq: fakeMq(true) });
    assert.equal(configs(reduced.sheet(NODE, {}))[0].duration, 0);
    assert.equal(configs(reduced.screen(NODE, {}))[0].duration, 0);
  });

  // The desktop full-screen player is a fade and a 0.98 scale.
  const desktopScreen = configs(motion.screen(NODE, {}))[0];
  assert.equal(desktopScreen.duration, DUR.base);
  assert.match(desktopScreen.css(0, 1), /scale\(0\.98\)/);

  // The veil fades its own background alpha, never its opacity (M8).
  const veil = configs(motion.veil(NODE, { for: "sheet", color: "rgba(0, 0, 0, 0.6)" }))[0];
  assert.match(veil.css(0, 1), /background-color: color-mix\(in srgb, rgba\(0, 0, 0, 0\.6\) 0\.0%, transparent\)/);
  assert.match(veil.css(1, 0), /100\.0%, transparent/);
  assert.ok(!/opacity/.test(cssAt(veil)), "a veil never animates opacity");

  // flipSmall translates from the old rect to the new one.
  const flip = motion.flipSmall(NODE, { from: RECT(0, 0), to: RECT(0, 40) });
  assert.equal(flip.duration, DUR.base);
  assert.match(flip.css(0, 1), /translate\(0px, -40px\)/);
});

test("animate() cancels the animation it started on the same element", () => {
  const motion = createMotion({ eco: writable(false), reduceMq: fakeMq(false) });
  let cancelled = 0;
  const el = {
    animate: () => ({
      cancel() {
        cancelled++;
      },
    }),
  };
  const first = motion.animate(el, [{ opacity: 0 }, { opacity: 1 }], { duration: 200 });
  assert.ok(first);
  const second = motion.animate(el, [{ opacity: 0 }, { opacity: 1 }], { duration: 200 });
  assert.ok(second);
  assert.notEqual(first, second);
  assert.equal(cancelled, 1, "the first animation was cancelled, not stacked");
});

// -- 6. one source of truth --------------------------------------------------

test("no file imports svelte/transition or svelte/animate", () => {
  // M15: lib/motion.js is the ONE home of a transition. T0 froze the 19 files
  // that still imported Svelte's built-ins so the sweep could be done lot by
  // lot; T7 emptied that list, so an import is now a failure anywhere — a new
  // file that reaches for `svelte/transition` fails HERE instead of drifting.
  const files = [];
  const walk = (dir) => {
    for (const name of readdirSync(dir)) {
      const path = join(dir, name);
      if (statSync(path).isDirectory()) walk(path);
      else if (/\.(svelte|js|mjs)$/.test(name)) files.push(path);
    }
  };
  walk(SRC);

  const importers = new Set();
  for (const path of files) {
    const text = readFileSync(path, "utf8");
    if (/from\s+["']svelte\/(transition|animate)["']/.test(text)) {
      importers.add(relative(SRC, path).replaceAll("\\", "/"));
    }
  }

  assert.deepEqual([...importers].sort(), [], "nothing imports svelte/transition or svelte/animate");
  assert.ok(!importers.has("lib/motion.js"), "the one motion module implements its transitions itself");

  // And the module really exports the transitions the plan lists, by name.
  for (const name of ["fadeIn", "fade", "rise", "swap", "pop", "dialog", "sheet", "veil", "panel", "screen", "collapse", "flipSmall"]) {
    assert.equal(typeof createMotion({})[name], "function", `createMotion() exposes ${name}`);
  }
  for (const name of ["motionLevel", "level", "scrollBehavior", "animate", "screenKey", "pageEnter", "shiftRows"]) {
    assert.ok(createMotion({})[name] !== undefined, `createMotion() exposes ${name}`);
  }
});

// -- 7. no literal durations -------------------------------------------------

/** Every `transition:`/`animation:` declaration in a file, with its value.
 *  Splitting on what ENDS a declaration (`;`, `{`, `}`) rather than on lines is
 *  what catches app.css's global `button` rule, whose five properties are
 *  written on five lines. Comments are cleared first: a rule explained in prose
 *  is not a rule. */
function declarations(text) {
  const clean = text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "");
  const out = [];
  for (const chunk of clean.split(/[;{}]/)) {
    const m = chunk.match(/(?:^|\s)(transition|animation)(-[a-z-]+)?\s*:\s*([\s\S]+)$/);
    if (!m) continue;
    out.push({ property: m[1] + (m[2] || ""), value: m[3].trim() });
  }
  return out;
}

// A keyframe name a literal `animation:` duration is allowed on: the indicators
// M13 keeps running in eco (a frozen spinner reads as a hang) and the skeletons
// and decorative loops M13/M14 stop. They CANNOT ride a `--dur-*` token: the
// tokens are 0 ms in eco, which is exactly what these must not become.
const STATUS_KEYFRAMES = new Map([
  ["spin", "a spinner says a job is running (M13)"],
  ["pp-spin", "the player bar's play/pause spinner (M13)"],
  ["hero-spin", "HomeHero's loader (M13)"],
  ["lc-spin", "the lyric card's share spinner (M13)"],
  ["rs-spin", "the recap share poster's spinner (M13)"],
  ["radio-spin", "the query radio's spinner (M13)"],
  ["ytspin", "the YouTube fallback's spinner (M13)"],
  ["rowspin", "the row that is being fetched (L7, M13)"],
  ["shimmer", "a skeleton (F7): its shimmer is the loading state itself"],
  ["ch-shimmer", "the chapter list's skeleton (F7)"],
  ["recap-shimmer", "the recap's skeleton (F7)"],
  ["slide", "an indeterminate job bar (I8, M13)"],
  ["bar-slide", "an indeterminate job bar (I8, M13)"],
  ["pulse", "the party/remote link's live indicator (M13)"],
  ["blink", "the remote banner's live indicator (M13)"],
  ["eq", "the playing row's decorative equalizer (L7): eco stops it"],
  ["flash", "a found transcript segment's highlight (P20): eco stops it"],
  ["grain-drift", "the visualizer's film grain (P22): eco has no canvas at all"],
  ["bg-in", "the sleeve backdrop's own entrance (P9): lib/backdrop.js owns its 600 ms"],
  ["gb-intro", "the shell ground's first arrival (P9): the same walk"],
]);

// A literal time that IS the plan's contract, not a UI motion the token table
// covers. Each entry is `file`, the declaration it lives in, and why it stays a
// number; the list is asserted to be USED, so it cannot rot into decoration.
const WALKS = [
  ["app.css", "transition", "--cover-ac 600ms",
    "the registered --cover-ac colour walk (webapp.md): one timerless 600 ms custom-property transition"],
  ["app.css", "transition-duration", "0.01ms",
    "the eco guard's remnant (M13): 0.01 ms, never 0 s, so a transitionend can still fire"],
  ["app.css", "animation-duration", "0.01ms",
    "the same guard, for keyframes"],
  ["components/GradientHeader.svelte", "transition", "--gh-c 600ms",
    "the header tint's 600 ms walk (webapp.md)"],
  ["components/GlobalBackdrop.svelte", "transition", "var(--gb-in, 420ms)",
    "the ground's fade-in duration comes from the script (IN_MS); the literal is its CSS fallback (webapp.md: 420/220 ms)"],
  ["components/GlobalBackdrop.svelte", "transition-duration", "var(--gb-out, 220ms)",
    "the same walk's fade-out fallback"],
  ["components/DesktopNowPlaying.svelte", "transition", "background-color 600ms",
    "the sleeve halo's colour walk (P9): lib/backdrop.js owns the 600 ms"],
  ["components/Visualizer.svelte", "transition", "opacity 0.6s",
    "the visualizer's own canvas fade (animations.md, P22: unchanged), on the sleeve walk's 600 ms"],
  ["components/Visualizer.svelte", "transition-duration", "0.2s",
    "its reduced-motion fallback (animations.md), not a UI token"],
];

// The same rule, one syntax away: a Svelte transition DIRECTIVE takes its
// duration from lib/motion.js too. `duration: 0` is the plan's own instant path
// (a cache hit, a row that must not animate on mount — M4/M6/M7) and is allowed;
// any other number is a literal.
const WALK_DIRECTIVES = [
  ["routes/Party.svelte", "in:fade", 420,
    "the party guest's ground is the shell ground's own 220 + 420 ms walk (webapp.md): this is the walk's second half, and it must match it"],
];

test("no transition or animation declaration carries a literal duration", () => {
  // M15, the second half: a duration or a distance comes from lib/motion.js or
  // from the `--dur-*`/`--ease-*`/`--move-*` tokens. A literal is invisible to
  // the token table, so it keeps running at 120 ms in eco where the guard is
  // the only thing left to stop it — and it is how `button { transition:
  // ... 0.12s }` survived the whole sweep (T7 tokenised it).
  const files = [];
  const walk = (dir) => {
    for (const name of readdirSync(dir)) {
      const path = join(dir, name);
      if (statSync(path).isDirectory()) walk(path);
      else if (/\.(svelte|css|js|mjs)$/.test(name)) files.push(path);
    }
  };
  walk(SRC);

  const used = new Set();
  const offenders = [];
  for (const path of files) {
    const name = relative(SRC, path).replaceAll("\\", "/");
    for (const { property, value } of declarations(readFileSync(path, "utf8"))) {
      for (const m of value.matchAll(/(?<![\w-])(-?\d*\.?\d+)(m?s)\b/g)) {
        const time = Number(m[1]);
        if (time === 0) continue; // `0s`/`0ms` is "no duration", the guard's own `0s` delay aside
        const literal = m[1] + m[2];

        // A walk whose number IS the contract.
        const walkIndex = WALKS.findIndex(
          ([file, prop, needle]) => file === name && prop === property && value.includes(needle),
        );
        if (walkIndex >= 0) {
          used.add(walkIndex);
          continue;
        }

        // A negative delay phases an infinite loop; it is not a duration.
        if (property === "animation-delay" && time < 0) continue;

        // A status/skeleton/decorative keyframe's own loop (M13/M14).
        if (property === "animation" || property === "animation-duration") {
          const keyframe = value
            .split(/[\s,]+/)
            .find((token) => /^[a-z][\w-]*$/i.test(token) && !/^(infinite|linear|ease|ease-in|ease-out|ease-in-out|steps|normal|reverse|alternate|alternate-reverse|forwards|backwards|both|none|running|paused)$/i.test(token));
          if (keyframe && STATUS_KEYFRAMES.has(keyframe)) continue;
        }

        offenders.push(`${name}: ${property}: ${value} (${literal})`);
      }
    }
  }

  assert.deepEqual(
    [...new Set(offenders)].sort(),
    [],
    "a duration comes from lib/motion.js or the --dur-* tokens: use `var(--dur-…) var(--ease-…)`, " +
      "or name the walk/status keyframe in this test with the number it carries and why",
  );

  // The directive form of the same rule (`in:fade={{ duration: 420 }}`).
  const directives = /\b(transition|in|out|animate):([A-Za-z][\w]*)\s*=\s*\{\{[^}]*?\bduration\s*:\s*(-?\d*\.?\d+)\b/g;
  const usedDirectives = new Set();
  const literalDirectives = [];
  for (const path of files) {
    if (!path.endsWith(".svelte")) continue;
    const name = relative(SRC, path).replaceAll("\\", "/");
    for (const m of readFileSync(path, "utf8").matchAll(directives)) {
      const time = Number(m[3]);
      if (time === 0) continue;
      const index = WALK_DIRECTIVES.findIndex(([file, key, ms]) => file === name && key === `${m[1]}:${m[2]}` && ms === time);
      if (index >= 0) {
        usedDirectives.add(index);
        continue;
      }
      literalDirectives.push(`${name}: ${m[1]}:${m[2]}={{ duration: ${m[3]} }}`);
    }
  }
  assert.deepEqual(
    [...new Set(literalDirectives)].sort(),
    [],
    "a transition directive's duration is DUR.* from lib/motion.js (0 is the instant path)",
  );

  // Keep the exemption tables honest: an unused entry is a stale claim.
  const staleWalks = WALKS.filter((_, index) => !used.has(index)).map(([file, prop, needle]) => `${file}: ${prop}: ${needle}`);
  assert.deepEqual(staleWalks, [], "every walk exemption is still a declaration in the source");
  const staleDirectives = WALK_DIRECTIVES.filter((_, index) => !usedDirectives.has(index)).map(
    ([file, key, ms]) => `${file}: ${key} {{ duration: ${ms} }}`,
  );
  assert.deepEqual(staleDirectives, [], "every directive exemption is still a directive in the source");
  for (const [keyframe, why] of STATUS_KEYFRAMES) {
    assert.ok(why.trim().length > 10, `${keyframe} says why it is not a token`);
  }
});
