// The app's ONE source of motion (plan: docs/agents/transitions.md).
//
// Everything that moves, fades, slides or collapses in the SPA reads its
// numbers and its curves from here; the CSS half of the same table lives in
// `app.css`'s "motion" block, and `test/motion.test.mjs` reads that file and
// fails if the two tables disagree. No component imports `svelte/transition`
// or `svelte/animate` any more (T7 empties the allowlist the same test keeps),
// so this module is also the only place that has to know the rules:
//
//   M1  transform and opacity only (plus a veil's background-colour alpha and
//       `collapse`'s height, both bounded by the plan);
//   M2  the click answers first, the animation plays on top and is always
//       interruptible (a bidirectional transition reverses from where it is,
//       `animate()` cancels the one it started on the same element);
//   M3  no View Transitions, no route outro: pages only enter;
//   M4  nothing animates on mount of a long/windowed list row;
//   M5  exits are shorter than entrances, enter decelerates, exit accelerates;
//   M6  frequent things barely move (hover, press, selection);
//   M7  replace-in-place = intro only;
//   M8  never fade an ancestor of a glass pane (veils fade their own
//       background-colour alpha instead);
//   M9  a fixed overlay is portaled with `lib/portal.js#toBody`;
//   M10 `collapse` only for small blocks;
//   M11 no stagger, no spring overshoot, no parallax;
//   M12 hand-offs by event, never by clock;
//   M13 eco = no animation at all — except `[data-motion="status"]` (a running
//       job has to say so: CLAUDE.md rule 2);
//   M14 reduced motion = no displacement, short fades (≤ 140 ms);
//   M15 one source of truth, this file and the `app.css` tokens.
//
// THREE LEVELS, read once per run:
//
//   "off"      eco mode. Svelte's instant path (`duration: 0`, no Animation
//              object), `animate()` returns null, nothing is written.
//   "reduced"  the OS `prefers-reduced-motion: reduce`. Same motion with every
//              translate/scale removed and the duration capped at DUR.fast.
//   "full"     the motion the inventory describes.
//
// `createMotion({ eco, reduceMq })` builds the whole API around injected
// sources, so `node --test` can hand it a writable and a fake MediaQueryList;
// the module's default instance wires `stores.js#ecoMode` and
// `window.matchMedia` and is safe to import in Node (no window, no DOM).
//
// Researched (2026-11): NN/g "Executing UX Animations", Apple HIG "Motion",
// Material 3 motion tokens, Linear (Emil Kowalski), WebKit's UnitBezier
// solver — the numbers and the sources are in docs/agents/transitions.md.

import { writable, get } from "svelte/store";
import { ecoMode } from "./stores.js";
import { isMobile } from "./viewport.js";

// -- the table ---------------------------------------------------------------

/** Milliseconds. `sheet` is the phone bottom sheet, `screen` the full-screen
 *  player; `sheetOut`/`screenOut` are the (shorter) exits. */
export const DUR = {
  instant: 90,
  fast: 140,
  base: 200,
  sheet: 280,
  sheetOut: 200,
  screen: 320,
  screenOut: 240,
};

/** Pixels. `s` for a glyph/title swap, `m` for a page, toast or popover. */
export const MOVE = { s: 4, m: 8 };

/** Menus and popovers pop from 0.96, a centred dialog from 0.97, the desktop
 *  full-screen player from 0.98. */
export const SCALE = { pop: 0.96, dialog: 0.97, screen: 0.98 };

/** The curves as CSS writes them (Material 3's tokens). */
export const EASE_CSS = {
  enter: "cubic-bezier(0.05, 0.7, 0.1, 1)",
  exit: "cubic-bezier(0.3, 0, 0.8, 0.15)",
  standard: "cubic-bezier(0.2, 0, 0, 1)",
};

// -- the curves, as functions ------------------------------------------------

// WebKit's UnitBezier (Source/WebCore/platform/graphics/UnitBezier.h), the same
// solver as the `bezier-easing` package every CSS engine's timing function is
// modelled on: invert x → t with 8 Newton–Raphson iterations (epsilon 1e-6),
// then fall back to 20 bisection steps when the derivative is too flat to
// converge, and read y at that t. A CSS `cubic-bezier()` and this function
// therefore agree to well under 1e-3 (test/motion.test.mjs checks exactly
// that, against an independent bisection).
const NEWTON_ITERATIONS = 8;
const NEWTON_MIN_SLOPE = 1e-4;
const SOLVE_EPSILON = 1e-6;
const BISECTION_STEPS = 20;

/**
 * A CSS `cubic-bezier(x1, y1, x2, y2)` as a `t => t` easing function.
 * @param {number} x1
 * @param {number} y1
 * @param {number} x2
 * @param {number} y2
 * @returns {(x: number) => number}
 */
export function bezier(x1, y1, x2, y2) {
  const coefA = (a1, a2) => 1 - 3 * a2 + 3 * a1;
  const coefB = (a1, a2) => 3 * a2 - 6 * a1;
  const coefC = (a1) => 3 * a1;
  const calc = (t, a1, a2) => ((coefA(a1, a2) * t + coefB(a1, a2)) * t + coefC(a1)) * t;
  const slope = (t, a1, a2) => 3 * coefA(a1, a2) * t * t + 2 * coefB(a1, a2) * t + coefC(a1);
  const sampleX = (t) => calc(t, x1, x2);
  const sampleY = (t) => calc(t, y1, y2);
  const sampleDX = (t) => slope(t, x1, x2);

  /** x → t (the curve's parameter), never the y. */
  function solveX(x) {
    let t = x;
    let dx;
    // Newton–Raphson first: it converges in three or four iterations on the
    // curves we use.
    for (let i = 0; i < NEWTON_ITERATIONS; i++) {
      dx = sampleX(t) - x;
      if (Math.abs(dx) < SOLVE_EPSILON) return t;
      const d = sampleDX(t);
      if (Math.abs(d) < NEWTON_MIN_SLOPE) break;
      t -= dx / d;
    }
    // Bisection for reliability: 20 steps take the bracket under 1e-6.
    let lo = 0;
    let hi = 1;
    t = x < 0 ? 0 : x > 1 ? 1 : x;
    for (let i = 0; i < BISECTION_STEPS; i++) {
      const at = sampleX(t);
      if (Math.abs(at - x) < SOLVE_EPSILON) return t;
      if (x > at) lo = t;
      else hi = t;
      t = (hi - lo) * 0.5 + lo;
    }
    return t;
  }

  // The curve the browser draws for `cubic-bezier(x1, y1, x2, y2)`.
  return (x) => {
    if (x <= 0) return 0;
    if (x >= 1) return 1;
    return sampleY(solveX(x));
  };
}

/** The curves as easing functions, for `easing:` and `el.animate()`. */
export const EASE = {
  enter: bezier(0.05, 0.7, 0.1, 1),
  exit: bezier(0.3, 0, 0.8, 0.15),
  standard: bezier(0.2, 0, 0, 1),
};

// -- levels ------------------------------------------------------------------

/** Svelte's instant path: no Animation object, no css sampling (measured in
 *  node_modules/svelte/internal/client/dom/elements/transitions.js: `if
 *  (!options?.duration && !options?.delay)` returns before any css call). */
const OFF = { duration: 0 };

const NUMBER = (value, fallback) => (Number.isFinite(+value) ? +value : fallback);
const PX = (n) => `${Math.round(n * 100) / 100}px`;

/** One computed style per run (never per frame): transitions that need the
 *  element's own value read it here. */
function styleOf(node) {
  if (!node || typeof getComputedStyle !== "function") return {};
  return getComputedStyle(node);
}

/**
 * Build the whole motion API around injected level sources.
 *
 * @param {{ eco?: { subscribe: Function } | null, reduceMq?: any }} [sources]
 *   `eco` is any readable/writable store of a boolean (the app's `ecoMode`),
 *   `reduceMq` a MediaQueryList-like object (`{ matches, addEventListener }`).
 *   Either may be omitted: `node --test` passes a `writable` and a fake.
 */
export function createMotion({ eco = null, reduceMq = null } = {}) {
  const motionLevel = writable("full");
  let ecoOn = false;
  let reduced = false;

  const compute = () => (ecoOn ? "off" : reduced ? "reduced" : "full");
  const publish = () => motionLevel.set(compute());

  // The person's own switch. `subscribe` hands its current value over at once,
  // so the level is right before the first frame.
  if (eco && typeof eco.subscribe === "function") {
    eco.subscribe((value) => {
      ecoOn = value === true || value === "1" || value === 1;
      publish();
    });
  }

  // The OS setting, listened to LIVE (`change`): reduced motion is toggled
  // while the app runs, and a read-once match is what made the old per-file
  // checks drift.
  if (reduceMq) {
    reduced = !!reduceMq.matches;
    const onChange = (event) => {
      reduced = event && typeof event.matches === "boolean" ? event.matches : !!reduceMq.matches;
      publish();
    };
    if (typeof reduceMq.addEventListener === "function") reduceMq.addEventListener("change", onChange);
    else if (typeof reduceMq.addListener === "function") reduceMq.addListener(onChange);
  }
  publish();

  const level = () => get(motionLevel);

  /** "smooth" only at full motion: a reduced or eco device jumps. */
  const scrollBehavior = () => (level() === "full" ? "smooth" : "auto");

  // The last animation this module started on an element: the interruption
  // rule (M2). A second page enter, or a row shift landing on a row that is
  // already gliding, cancels the first instead of stacking two.
  const running = new WeakMap();

  // Transform-ish properties, dropped under reduced motion (M14).
  const MOVEMENT = new Set(["transform", "translate", "scale", "rotate"]);

  function stripMovement(keyframes) {
    const list = Array.isArray(keyframes) ? keyframes : [keyframes];
    return list.map((frame) => {
      const out = {};
      for (const [key, value] of Object.entries(frame)) {
        if (!MOVEMENT.has(key)) out[key] = value;
      }
      return out;
    });
  }

  /**
   * The ONE wrapper for imperative WAAPI (page enter, row shift).
   * Returns the Animation, or null when eco is on. In reduced motion the
   * movement is dropped and the duration capped; the previous animation this
   * module started on the same element is always cancelled first.
   */
  function animate(el, keyframes, options = {}) {
    if (!el || typeof el.animate !== "function") return null;
    const previous = running.get(el);
    if (previous) {
      running.delete(el);
      try {
        previous.cancel();
      } catch {
        /* an already-cancelled animation */
      }
    }
    const now = level();
    if (now === "off") return null;
    const frames = now === "reduced" ? stripMovement(keyframes) : keyframes;
    const opts = { fill: "none", ...options };
    if (now === "reduced") opts.duration = Math.min(NUMBER(opts.duration, DUR.base), DUR.fast);
    const animation = el.animate(frames, opts);
    running.set(el, animation);
    return animation;
  }

  /** A run's config: the level is read HERE, once, at the start (M2/M13/M14). */
  const run = (direction, { enter, exit, css, reduced: reducedCss }) => {
    const now = level();
    if (now === "off") return OFF;
    const isIn = direction === "in";
    const base = isIn ? NUMBER(enter, DUR.base) : NUMBER(exit, DUR.fast);
    const duration = now === "reduced" ? Math.min(base, DUR.fast) : base;
    return {
      duration,
      easing: isIn ? EASE.enter : EASE.exit,
      css: now === "reduced" && reducedCss ? reducedCss : css,
    };
  };

  // -- the transitions (Svelte's `(node, params) => config | ({direction}) => config`)

  /** opacity 0 → 1, enter only (M7: the thing it replaces goes at once). */
  function fadeIn(node, params = {}) {
    const now = level();
    if (now === "off") return OFF;
    const duration = NUMBER(params.duration, DUR.fast);
    return {
      duration: now === "reduced" ? Math.min(duration, DUR.fast) : duration,
      easing: EASE.enter,
      css: (t) => `opacity: ${t}`,
    };
  }

  /** opacity, both ways: base to enter, fast to leave (M5). */
  function fade(node, params = {}) {
    return ({ direction }) =>
      run(direction, {
        enter: params.duration,
        exit: params.out,
        css: (t) => `opacity: ${t}`,
      });
  }

  /** opacity + translateY(y → 0). Negative y comes from above. */
  function rise(node, params = {}) {
    const y = NUMBER(params.y, MOVE.m);
    return ({ direction }) =>
      run(direction, {
        enter: params.duration,
        exit: params.out,
        css: (t, u) => `opacity: ${t}; transform: translateY(${PX(u * y)})`,
        reduced: (t) => `opacity: ${t}`,
      });
  }

  /** opacity + translateY(MOVE.s → 0) and/or scale → 1, enter only: glyphs,
   *  titles, counters, the player bar's play/pause glyph (P13). */
  function swap(node, params = {}) {
    const now = level();
    if (now === "off") return OFF;
    const y = NUMBER(params.y, MOVE.s);
    const scale = NUMBER(params.scale, 1);
    const duration = NUMBER(params.duration, DUR.fast);
    const capped = now === "reduced" ? Math.min(duration, DUR.fast) : duration;
    if (now === "reduced") return { duration: capped, easing: EASE.enter, css: (t) => `opacity: ${t}` };
    return {
      duration: capped,
      easing: EASE.enter,
      css: (t, u) =>
        `opacity: ${t}` +
        (y !== 0 || scale !== 1
          ? `; transform:${y !== 0 ? ` translateY(${PX(u * y)})` : ""}${scale !== 1 ? ` scale(${1 - u * (1 - scale)})` : ""}`
          : ""),
    };
  }

  /** opacity + scale + a 4 px lean toward its anchor (menus, popovers). */
  function pop(node, params = {}) {
    const origin = params.origin || "center";
    const placement = params.placement === "up" ? 1 : -1;
    return ({ direction }) =>
      run(direction, {
        enter: params.duration,
        exit: params.out,
        css: (t, u) =>
          `opacity: ${t}; transform-origin: ${origin}; transform: translateY(${PX(u * placement * MOVE.s)}) scale(${1 - u * (1 - SCALE.pop)})`,
        reduced: (t) => `opacity: ${t}`,
      });
  }

  /** opacity + scale(SCALE.dialog → 1): a centred confirm. */
  function dialog(node, params = {}) {
    return ({ direction }) =>
      run(direction, {
        enter: params.duration,
        exit: params.out,
        css: (t, u) => `opacity: ${t}; transform: scale(${1 - u * (1 - SCALE.dialog)})`,
        reduced: (t) => `opacity: ${t}`,
      });
  }

  /** A bottom sheet: slides up on a phone, fades+scales on the desktop (M8:
   *  a phone sheet carries glass panes, so it never gets an opacity). */
  function sheet(node, params = {}) {
    const fromY = params.fromY;
    return ({ direction }) => {
      const now = level();
      if (now === "off") return OFF;
      const isIn = direction === "in";
      const phone = isMobile();
      if (phone) {
        // No displacement under reduced motion, and no opacity either (M8):
        // the sheet appears, it does not slide.
        if (now === "reduced") return OFF;
        const duration = isIn ? DUR.sheet : DUR.sheetOut;
        const start = NUMBER(fromY, 0) > 0 ? PX(fromY) : "100%";
        return {
          duration,
          easing: isIn ? EASE.enter : EASE.exit,
          css: (t, u) => `transform: translateY(calc(${u} * ${start}))`,
        };
      }
      return run(direction, {
        enter: params.duration,
        exit: params.out,
        css: (t, u) => `opacity: ${t}; transform: scale(${1 - u * (1 - SCALE.dialog)})`,
        reduced: (t) => `opacity: ${t}`,
      });
    };
  }

  /** A veil fades its OWN background-colour alpha: opacity on this ancestor
   *  would switch its sheet's `backdrop-filter` off (M8, measured 5.0 → 127.4). */
  function veil(node, params = {}) {
    const kind = params.for || "sheet";
    return ({ direction }) => {
      const now = level();
      if (now === "off") return OFF;
      const isIn = direction === "in";
      const phone = isMobile();
      let enter = DUR.base;
      let exit = DUR.fast;
      if (kind === "sheet" && phone) {
        enter = DUR.sheet;
        exit = DUR.sheetOut;
      }
      const duration = isIn ? enter : exit;
      const capped = now === "reduced" ? Math.min(duration, DUR.fast) : duration;
      // Read once per run, from the element itself, so the fade lands exactly
      // on the tint its own stylesheet asked for.
      const colour = params.color || styleOf(node).backgroundColor || "";
      return {
        duration: capped,
        easing: isIn ? EASE.enter : EASE.exit,
        css: (t) =>
          colour
            ? `background-color: color-mix(in srgb, ${colour} ${(t * 100).toFixed(1)}%, transparent)`
            : `background-color: rgba(0, 0, 0, ${(t * 0.6).toFixed(3)})`,
      };
    };
  }

  /** The desktop Now-Playing column: opacity + a 16 px slide from its side. */
  function panel(node, params = {}) {
    const x = NUMBER(params.x, 16);
    return ({ direction }) =>
      run(direction, {
        enter: params.duration,
        exit: params.out,
        css: (t, u) => `opacity: ${t}; transform: translateX(${PX(u * x)})`,
        reduced: (t) => `opacity: ${t}`,
      });
  }

  /** The full-screen player: phone slides up, desktop fades and scales. */
  function screen(node, params = {}) {
    const fromY = params.fromY;
    return ({ direction }) => {
      const now = level();
      if (now === "off") return OFF;
      const isIn = direction === "in";
      if (isMobile()) {
        // The phone player is glass on a photograph: no opacity (M8), and
        // nothing moves at all under reduced motion.
        if (now === "reduced") return OFF;
        const duration = isIn ? DUR.screen : DUR.screenOut;
        const start = NUMBER(fromY, 0) > 0 ? PX(fromY) : "100%";
        return {
          duration,
          easing: isIn ? EASE.enter : EASE.exit,
          css: (t, u) => `transform: translateY(calc(${u} * ${start}))`,
        };
      }
      return run(direction, {
        enter: params.duration,
        exit: params.out,
        css: (t, u) => `opacity: ${t}; transform: scale(${1 - u * (1 - SCALE.screen)})`,
        reduced: (t) => `opacity: ${t}`,
      });
    };
  }

  /** Svelte `slide`'s semantics (height + padding + margins + borders, one
   *  layout per frame), bounded by M10: small blocks only. Under reduced
   *  motion the height goes and a short fade stays (no displacement). */
  function collapse(node, params = {}) {
    return ({ direction }) => {
      const now = level();
      if (now === "off") return OFF;
      const isIn = direction === "in";
      const base = isIn ? NUMBER(params.duration, DUR.base) : NUMBER(params.out, DUR.fast);
      const duration = now === "reduced" ? Math.min(base, DUR.fast) : base;
      if (now === "reduced") return { duration, easing: EASE.standard, css: (t) => `opacity: ${t}` };
      const style = styleOf(node);
      const value = (name) => (Number.isFinite(parseFloat(style[name])) ? parseFloat(style[name]) : 0);
      const height = value("height");
      const padTop = value("paddingTop");
      const padBottom = value("paddingBottom");
      const marginTop = value("marginTop");
      const marginBottom = value("marginBottom");
      const borderTop = value("borderTopWidth");
      const borderBottom = value("borderBottomWidth");
      const opacity = Number.isFinite(parseFloat(style.opacity)) ? parseFloat(style.opacity) : 1;
      return {
        duration,
        easing: EASE.standard,
        css: (t) =>
          "overflow: hidden;" +
          `opacity: ${Math.min(t * 20, 1) * opacity};` +
          `height: ${t * height}px;` +
          `min-height: 0;` +
          `padding-top: ${t * padTop}px;` +
          `padding-bottom: ${t * padBottom}px;` +
          `margin-top: ${t * marginTop}px;` +
          `margin-bottom: ${t * marginBottom}px;` +
          `border-top-width: ${t * borderTop}px;` +
          `border-bottom-width: ${t * borderBottom}px;`,
      };
    };
  }

  /** Svelte `flip`, for keyed lists that can never exceed 50 items (M4). */
  function flipSmall(node, { from, to } = {}, params = {}) {
    const now = level();
    if (now === "off" || now === "reduced") return OFF;
    if (!from || !to) return OFF;
    const style = styleOf(node);
    const transform = !style.transform || style.transform === "none" ? "" : style.transform;
    const [originX = 0, originY = 0] = String(style.transformOrigin || "0 0").split(" ").map(parseFloat);
    const width = (node && node.clientWidth) || to.width || 1;
    const height = (node && node.clientHeight) || to.height || 1;
    const ox = (originX || 0) / width;
    const oy = (originY || 0) / height;
    const dx = from.left + from.width * ox - (to.left + to.width * ox);
    const dy = from.top + from.height * oy - (to.top + to.height * oy);
    if (!dx && !dy) return OFF;
    const duration = NUMBER(params.duration, DUR.base);
    return {
      duration,
      easing: EASE.standard,
      css: (t, u) => `transform: ${transform} translate(${PX(u * dx)}, ${PX(u * dy)})`.trim(),
    };
  }

  // -- helpers the shell uses --------------------------------------------------

  // The screens whose sub-paths are IN-SCREEN state written with `replace()`
  // (a library tab, a settings section, a search query, an import URL): moving
  // between them is not a page change, so the page does not enter again.
  const SCREEN_PREFIXES = ["/library", "/settings", "/search", "/import"];

  /**
   * The page-transition key of a route. "/library/artists" and "/library" are
   * the same screen; "/album/1" and "/album/2" are two.
   */
  function screenKey(path) {
    let p = String(path ?? "/");
    const hash = p.indexOf("#");
    if (hash >= 0) p = p.slice(0, hash);
    const query = p.indexOf("?");
    if (query >= 0) p = p.slice(0, query);
    if (!p.startsWith("/")) p = "/" + p;
    if (p.length > 1 && p.endsWith("/")) p = p.slice(0, -1);
    if (!p) return "/";
    for (const prefix of SCREEN_PREFIXES) {
      if (p === prefix || p.startsWith(prefix + "/")) return prefix;
    }
    return p;
  }

  /**
   * The page enter, called on `onRouteLoaded` when `screenKey` changed:
   * a push rises 8 px over DUR.base, a back/forward only fades over DUR.fast
   * (nothing moves, so a restored scroll offset is not seen to slide).
   */
  function pageEnter(node, { traversal = false } = {}) {
    const keyframes = traversal
      ? [{ opacity: 0 }, { opacity: 1 }]
      : [{ opacity: 0, transform: `translateY(${MOVE.m}px)` }, { opacity: 1, transform: "none" }];
    return animate(node, keyframes, { duration: traversal ? DUR.fast : DUR.base, easing: EASE_CSS.enter });
  }

  /**
   * The queue's rows make room: every mounted row at or after `fromIndex`
   * starts one row off and glides back, with no geometry read at all (M4/L2).
   * Call it right after the store mutation, after `await tick()`.
   * A row shift is pure displacement: eco AND reduced motion do nothing.
   */
  function shiftRows(container, fromIndex, deltaRows, rowH, startIndex = 0) {
    if (!container || !container.children || !deltaRows || !rowH) return [];
    if (level() !== "full") return [];
    const dy = -deltaRows * rowH;
    const out = [];
    for (let i = 0; i < container.children.length; i++) {
      if (startIndex + i < fromIndex) continue;
      const animation = animate(container.children[i], [{ transform: `translateY(${PX(dy)})` }, { transform: "none" }], {
        duration: DUR.base,
        easing: EASE_CSS.standard,
      });
      if (animation) out.push(animation);
    }
    return out;
  }

  return {
    motionLevel,
    level,
    scrollBehavior,
    animate,
    fadeIn,
    fade,
    rise,
    swap,
    pop,
    dialog,
    sheet,
    veil,
    panel,
    screen,
    collapse,
    flipSmall,
    screenKey,
    pageEnter,
    shiftRows,
  };
}

// -- the app's instance ------------------------------------------------------

/** The OS setting, or null where there is none (Node, an old WebView). */
function systemReduce() {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return null;
  try {
    return window.matchMedia("(prefers-reduced-motion: reduce)");
  } catch {
    return null;
  }
}

const app = createMotion({ eco: ecoMode, reduceMq: systemReduce() });

/** "off" | "reduced" | "full", as a readable store. */
export const motionLevel = app.motionLevel;
export const level = app.level;
export const scrollBehavior = app.scrollBehavior;
export const animate = app.animate;
export const { fadeIn, fade, rise, swap, pop, dialog, sheet, veil, panel, screen, collapse, flipSmall } = app;
export const { screenKey, pageEnter, shiftRows } = app;
