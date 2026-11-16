#!/usr/bin/env node
// The motion bench: what the transition plan's rules cost and do, on the real
// app, in headless Chromium.
//
//   npm run build && node test/motion/run.mjs
//   DIST=/tmp/build node test/motion/run.mjs        # a build made elsewhere
//   SHOTS=/tmp/motion node test/motion/run.mjs      # PNGs of every leg
//   ONLY=eco node test/motion/run.mjs               # one motion level
//   PYTHON=<venv python> …                          # a project venv
//
// The runner starts `tools/perf_api.py --serve` through test/harness.mjs (an
// 8 000-track library, Deezer off) and walks BOTH viewports the app is built
// for — 1440×900 and 390×844 (isMobile, hasTouch) — in the THREE motion levels
// of docs/agents/transitions.md: full, reduced (`emulateMedia`),
// eco (`viz.eco` in localStorage before boot, so `<html data-eco="1">`).
//
// LOT T0's OWN CHECKS (the foundation): the tokens resolve to the plan's
// numbers at full motion, to 0 ms/0 px/1 in eco and to ≤ 140 ms at reduced;
// and the central CSS guard really stops the cascade — after a real hover and
// after a class toggle on a `.btn`, `document.getAnimations()` filtered to
// targets outside `[data-motion="status"]` is EMPTY in eco, while the same two
// gestures leave running transitions at full motion. The `status` exception is
// proven both ways: a spinner marked `data-motion="status"` keeps its 1 s
// animation in eco, an unmarked one is stopped.
//
// EVERY LATER LOT adds `scenarios/<lot>.mjs` (see scenarios/README.md) with one
// scenario per inventory row that has a clickable trigger; this runner then
// holds each of them, in the three levels, to:
//   - identical /api and image requests across the three levels (a transition
//     never adds a request: CLAUDE.md rule 3);
//   - eco: no animation at all outside `[data-motion="status"]`;
//   - reduced: no running Animation whose keyframes carry a transform, and no
//     duration over 140 ms (M14);
// and takes the pictures the plan asks for (SHOTS=<dir>).

import { existsSync, mkdirSync, readdirSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import { join } from "node:path";
import { checker, loadPlaywright, login, startServer } from "../harness.mjs";

const LEVELS = (process.env.ONLY ? process.env.ONLY.split(",") : ["full", "reduced", "eco"]).filter((level) =>
  ["full", "reduced", "eco"].includes(level),
);
const SHOTS = process.env.SHOTS || null;
if (SHOTS) mkdirSync(SHOTS, { recursive: true });

const VIEWPORTS = [
  ["bureau 1440", { viewport: { width: 1440, height: 900 } }, false],
  [
    "téléphone 390",
    { viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true },
    true,
  ],
];

const { check, state } = checker();
const say = (text) => console.log(text);
const json = (value) => JSON.stringify(value);

/** A computed CSS time in seconds ("140ms" and "0.00001s" both live here). */
const secs = (value) => (String(value).endsWith("ms") ? parseFloat(value) / 1000 : parseFloat(value));

/** The alpha out of any serialisation Chromium picks for a colour:
 *  `rgba(0, 0, 0, 0.3)`, `color(srgb 0 0 0 / 0.3)`, `oklab(0 0 0 / 0)`. */
function alpha(value) {
  const text = String(value);
  const slash = text.match(/\/\s*([\d.]+)\s*\)$/);
  if (slash) return parseFloat(slash[1]);
  const rgba = text.match(/^rgba?\([^)]*?,\s*([\d.]+)\s*\)$/);
  if (rgba) return parseFloat(rgba[1]);
  return text === "transparent" ? 0 : NaN;
}

const server = await startServer();
process.on("exit", () => {
  try {
    server.stop();
  } catch {
    /* already gone */
  }
});

const { chromium } = await loadPlaywright();
const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM || (existsSync("/usr/bin/chromium") ? "/usr/bin/chromium" : undefined),
  args: ["--no-sandbox", "--autoplay-policy=no-user-gesture-required"],
});

// -- the page probes ---------------------------------------------------------

// Installed in every context: a spinner marked `data-motion="status"` and one
// that is not, plus the reader the guard is judged with. Everything here reads
// computed style and the Web Animations timeline, never a clock.
const HELPERS = () => {
  const style = document.createElement("style");
  style.textContent = "@keyframes motionProbeSpin { from { opacity: 0 } to { opacity: 1 } }";
  document.head.appendChild(style);

  // Two spinners for the guard's `status` exception: the marked one must keep
  // running in eco, the unmarked one must stop.
  const spin = document.createElement("div");
  spin.dataset.motion = "probe";
  spin.style.cssText =
    "position:fixed;left:-40px;top:0;width:8px;height:8px;animation:motionProbeSpin 1s linear infinite";
  document.body.appendChild(spin);
  const status = document.createElement("div");
  status.dataset.motion = "status";
  status.style.cssText =
    "position:fixed;left:-40px;top:12px;width:8px;height:8px;animation:motionProbeSpin 1s linear infinite";
  document.body.appendChild(status);

  // A real `.btn` with the app's own two classes, where a hover changes
  // `background-color` — i.e. one of the properties `button { transition: … }`
  // names, so a hover really starts a transition when motion is on.
  const button = document.createElement("button");
  button.type = "button";
  button.className = "btn btn-ghost";
  button.textContent = "Son";
  button.style.cssText = "position:fixed;left:8px;bottom:8px;z-index:2147483000";
  document.body.appendChild(button);

  window.__motionProbe = { spin, status, button };

  window.__motion = {
    frames: (n = 2) =>
      new Promise((resolve) => {
        const step = () => (--n <= 0 ? resolve() : requestAnimationFrame(step));
        requestAnimationFrame(step);
      }),
    /** Everything running that is not a status/probe indicator (the plan's
     *  filter, widened to the bench's own unmarked spinner). */
    running() {
      return document.getAnimations().filter((animation) => {
        const target = animation.effect && animation.effect.target;
        return !(target && target.closest && target.closest("[data-motion]"));
      });
    },
    describe(animation) {
      const effect = animation.effect;
      const target = effect && effect.target;
      const keyframes = effect && effect.getKeyframes ? effect.getKeyframes() : [];
      return {
        name: (target && target.className && target.className.toString()) || (target && target.tagName) || "?",
        probe: !!(target && target === window.__motionProbe.button),
        duration: Math.round(effect ? effect.getTiming().duration : 0),
        keys: keyframes.flatMap((frame) =>
          Object.keys(frame).filter((key) => !/^(offset|easing|composite|computedOffset)$/.test(key)),
        ),
      };
    },
    tokens() {
      const s = getComputedStyle(document.documentElement);
      const read = (name) => s.getPropertyValue(name).trim();
      return {
        "dur-instant": read("--dur-instant"),
        "dur-fast": read("--dur-fast"),
        "dur-base": read("--dur-base"),
        "dur-sheet": read("--dur-sheet"),
        "dur-screen": read("--dur-screen"),
        "move-s": read("--move-s"),
        "move-m": read("--move-m"),
        "scale-pop": read("--scale-pop"),
        "scale-screen": read("--scale-screen"),
        eco: document.documentElement.dataset.eco || null,
      };
    },
    /** The CSS expressions the JS emits, applied to a real element AT CREATION
     *  (a style set before insertion starts no transition — eco's guard gives
     *  every element a 0.01 ms transition, so a later write would not be
     *  readable in the same task): if the browser refuses one, the transition
     *  it belongs to would silently do nothing (a phone sheet that never
     *  slides). */
    expressions() {
      const sample = (css) => {
        const el = document.createElement("div");
        el.style.cssText = `position:fixed;left:-999px;top:0;width:100px;height:200px;${css}`;
        document.body.appendChild(el);
        const computed = getComputedStyle(el);
        const out = { transform: computed.transform, background: computed.backgroundColor };
        el.remove();
        return out;
      };
      return {
        slide: sample("transform:translateY(calc(1 * 100%))").transform,
        slideHalf: sample("transform:translateY(calc(0.5 * 100%))").transform,
        slideFromY: sample("transform:translateY(calc(0.25 * 120px))").transform,
        pop: sample("transform:translateY(-4px) scale(0.96)").transform,
        veil: sample("background-color:color-mix(in srgb, rgba(0, 0, 0, 0.6) 50%, transparent)").background,
        veilZero: sample("background-color:color-mix(in srgb, rgba(0, 0, 0, 0.6) 0%, transparent)").background,
      };
    },
    spinner() {
      const read = (el) => ({
        duration: getComputedStyle(el).animationDuration,
        running: el.getAnimations().length,
      });
      return { plain: read(window.__motionProbe.spin), status: read(window.__motionProbe.status) };
    },
  };
};

async function open(level, viewport) {
  const options = { ...viewport };
  if (level === "reduced") options.reducedMotion = "reduce";
  const ctx = await browser.newContext(options);
  if (level === "eco") {
    // Before boot: `<html data-eco="1">` is written by the shell's watchEco,
    // and the tokens have to be zero from the first paint.
    await ctx.addInitScript(() => localStorage.setItem("viz.eco", JSON.stringify(true)));
  }
  await ctx.route(/api\.deezer\.com|dzcdn\.net/, (route) => route.abort());
  const page = await ctx.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(server.base + "/app/");
  await login(page, server.base);
  // A REAL document load after the login: `goto` to a hash-only change is a
  // same-document navigation, so the shell would never be re-booted with the
  // new session and the bench would silently measure the login screen.
  await page.reload({ timeout: 60000 });
  await page.waitForSelector("footer.player, .mobilenav, main", { timeout: 60000 });
  await page.evaluate(HELPERS);
  if (level === "eco") {
    await page.waitForFunction(() => document.documentElement.dataset.eco === "1", null, { timeout: 15000 });
  }
  await page.evaluate(() => window.__motion.frames(3));
  const authed = await page.evaluate(() => !document.querySelector('input[type="password"]'));
  return { ctx, page, errors, authed };
}

/** A REAL pointer hover on the bench's own `.btn.btn-ghost`, then what is on
 *  the timeline one frame later (a 120 ms background-colour transition is
 *  still running) and three frames after that, still hovered. */
async function realHover(page) {
  const box = await page.evaluate(() => {
    const el = window.__motionProbe.button;
    const r = el.getBoundingClientRect();
    return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
  });
  const duration = await page.evaluate(
    () => getComputedStyle(window.__motionProbe.button).transitionDuration,
  );
  await page.mouse.move(box.x, box.y);
  const early = await page.evaluate(async () => {
    await window.__motion.frames(1);
    return window.__motion.running().map(window.__motion.describe);
  });
  const later = await page.evaluate(async () => {
    await window.__motion.frames(3);
    return window.__motion.running().map(window.__motion.describe);
  });
  await page.mouse.move(2, 2);
  await page.evaluate(() => window.__motion.frames(4));
  return { duration, early, later };
}

// -- T0: the tokens and the guard -------------------------------------------

async function foundation(page, level, phone) {
  const tag = `${level}${phone ? " 390" : " 1440"}`;
  const tokens = await page.evaluate(() => window.__motion.tokens());
  const hover = await realHover(page);
  const spinner = await page.evaluate(() => window.__motion.spinner());
  const expressions = await page.evaluate(() => window.__motion.expressions());

  // The transforms and the veil's colour, as the browser computes them: a
  // refused expression would make a transition silently do nothing.
  check(expressions.slide === "matrix(1, 0, 0, 1, 0, 200)", `${tag}: translateY(calc(1 * 100%)) resolves`, expressions.slide);
  check(expressions.slideHalf === "matrix(1, 0, 0, 1, 0, 100)", `${tag}: … and at half way`, expressions.slideHalf);
  check(expressions.slideFromY === "matrix(1, 0, 0, 1, 0, 30)", `${tag}: … and from a dragged fromY`, expressions.slideFromY);
  check(expressions.pop === "matrix(0.96, 0, 0, 0.96, 0, -4)", `${tag}: a pop's scale+lean resolves`, expressions.pop);
  check(alpha(expressions.veil) === 0.3, `${tag}: a veil at 50 % is half its alpha`, expressions.veil);
  check(alpha(expressions.veilZero) === 0, `${tag}: … and at 0 % it is clear`, expressions.veilZero);

  say(
    `  ${tag} : --dur-base ${tokens["dur-base"]}, --dur-fast ${tokens["dur-fast"]}, ` +
      `--move-m ${tokens["move-m"]}, --scale-pop ${tokens["scale-pop"]}, data-eco=${tokens.eco}`,
  );

  // 1. the tokens resolve to the plan's numbers, per level (the browser
  //    serialises a time as seconds: ".2s" is 200 ms).
  if (level === "full") {
    check(secs(tokens["dur-instant"]) === 0.09, `${tag}: --dur-instant is 90ms`, tokens["dur-instant"]);
    check(secs(tokens["dur-fast"]) === 0.14, `${tag}: --dur-fast is 140ms`, tokens["dur-fast"]);
    check(secs(tokens["dur-base"]) === 0.2, `${tag}: --dur-base is 200ms`, tokens["dur-base"]);
    check(secs(tokens["dur-sheet"]) === 0.28, `${tag}: --dur-sheet is 280ms`, tokens["dur-sheet"]);
    check(secs(tokens["dur-screen"]) === 0.32, `${tag}: --dur-screen is 320ms`, tokens["dur-screen"]);
    check(tokens["move-m"] === "8px" && tokens["move-s"] === "4px", `${tag}: --move-m 8px, --move-s 4px`);
    check(parseFloat(tokens["scale-pop"]) === 0.96, `${tag}: --scale-pop is 0.96`, tokens["scale-pop"]);
    check(parseFloat(tokens["scale-screen"]) === 0.98, `${tag}: --scale-screen is 0.98`, tokens["scale-screen"]);
    check(tokens.eco === null, `${tag}: no data-eco attribute at full motion`);
  }
  if (level === "reduced") {
    check(secs(tokens["dur-base"]) === 0.14, `${tag}: --dur-base is capped at 140ms`, tokens["dur-base"]);
    check(secs(tokens["dur-sheet"]) === 0.14, `${tag}: --dur-sheet is capped at 140ms`, tokens["dur-sheet"]);
    check(secs(tokens["dur-screen"]) === 0.14, `${tag}: --dur-screen is capped at 140ms`, tokens["dur-screen"]);
    check(tokens["move-m"] === "0px" && tokens["move-s"] === "0px", `${tag}: the moves are 0px`);
    check(parseFloat(tokens["scale-pop"]) === 1 && parseFloat(tokens["scale-screen"]) === 1, `${tag}: the scales are 1`);
  }

  // 2. eco: the tokens are zero, no gesture starts an animation outside a
  //    status indicator, and both halves of the guard hold.
  if (level === "eco") {
    check(tokens.eco === "1", `${tag}: <html data-eco="1">`, String(tokens.eco));
    for (const name of ["dur-instant", "dur-fast", "dur-base", "dur-sheet", "dur-screen"]) {
      check(secs(tokens[name]) === 0, `${tag}: eco --${name} is 0ms`, tokens[name]);
    }
    check(tokens["move-m"] === "0px" && tokens["move-s"] === "0px", `${tag}: eco moves are 0px`);
    check(parseFloat(tokens["scale-pop"]) === 1, `${tag}: eco scales are 1`, tokens["scale-pop"]);

    check(secs(hover.duration) <= 0.00001, `${tag}: the .btn's own transition-duration is 0.01ms`, hover.duration);
    check(
      hover.early.every((animation) => animation.duration === 0),
      `${tag}: nothing but a 0 ms remnant starts on a .btn hover in eco`,
      json(hover.early),
    );
    check(hover.later.length === 0, `${tag}: getAnimations() empty after a .btn hover in eco`, json(hover.later));
    check(secs(spinner.plain.duration) <= 0.00001, `${tag}: an unmarked 1 s animation is stopped`, spinner.plain.duration);
    check(spinner.plain.running === 0, `${tag}: … and it is not on the timeline`);
    check(secs(spinner.status.duration) === 1, `${tag}: a [data-motion="status"] spinner keeps its 1 s`, spinner.status.duration);
    check(spinner.status.running === 1, `${tag}: … and keeps running`);
  } else if (level === "full") {
    // 3. At full motion the SAME gesture must leave something running, or the
    //    eco check above would pass on a page where nothing animates at all.
    const mine = hover.early.filter((animation) => animation.probe);
    check(mine.length > 0, `${tag}: a real pointer hover on a .btn animates`, json(hover.early));
    check(
      mine.some((animation) => animation.keys.includes("backgroundColor")),
      `${tag}: … on background-color, one of the properties M1 allows`,
      json(mine),
    );
    check(
      mine.every((animation) => animation.duration <= 140),
      `${tag}: … for at most 140 ms (M6: a frequent thing barely moves)`,
      json(mine.map((a) => a.duration)),
    );
    check(
      hover.later.some((animation) => animation.probe),
      `${tag}: … and it is still running three frames later`,
      json(hover.later),
    );
    const rest = hover.early.filter((animation) => !animation.probe);
    say(
      `  ${tag} : ${mine.length} transition(s) on the hovered .btn — ${json(mine.map((a) => a.duration))} ms` +
        (rest.length ? ` ; ${rest.length} other animation(s) on the page — ${json(rest.map((a) => [a.name, a.duration]))}` : ""),
    );
  } else {
    // Reduced motion: the app's own `.btn` rules are off (app.css), and the
    // bench says so rather than trusting a comment.
    check(hover.early.length === 0, `${tag}: a .btn hover starts no transition under reduced motion`, json(hover.early));
  }

  return tokens;
}

// -- SHOTS ------------------------------------------------------------------

async function shot(page, name) {
  if (!SHOTS) return;
  // The bench's own probes are not the app's UI: hide them for the picture.
  await page.evaluate(() => {
    for (const el of document.querySelectorAll("[data-motion], #motion-probe-btn")) el.style.visibility = "hidden";
    const probe = window.__motionProbe && window.__motionProbe.button;
    if (probe) probe.style.visibility = "hidden";
  });
  await page.screenshot({ path: join(SHOTS, `${name}.png`) });
  await page.evaluate(() => {
    for (const el of document.querySelectorAll("[data-motion]")) el.style.visibility = "visible";
    const probe = window.__motionProbe && window.__motionProbe.button;
    if (probe) probe.style.visibility = "visible";
  });
}

// -- scenarios ---------------------------------------------------------------

const scenarioDir = fileURLToPath(new URL("./scenarios/", import.meta.url));
const scenarioFiles = readdirSync(scenarioDir).filter((name) => name.endsWith(".mjs"));
const scenarios = [];
for (const name of scenarioFiles) {
  const module = await import(pathToFileURL(join(scenarioDir, name)).href);
  for (const scenario of module.scenarios || []) scenarios.push({ file: name, ...scenario });
}

/** The /api and image requests a page asked for: the "nothing extra" check. */
function watch(page, requests) {
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname.startsWith("/api/") || request.resourceType() === "image") {
      requests.push(url.pathname.replace(/\?.*$/, ""));
    }
  });
}

async function runScenario(page, requests, scenario, level, tag) {
  if (scenario.setup) await scenario.setup(page);
  await page.evaluate(() => window.__motion.frames(2));
  const mark = requests.length;
  if (scenario.act) await scenario.act(page);
  if (SHOTS && scenario.midway) await shot(page, `${tag}-${scenario.name}-50`);
  if (scenario.settle) await scenario.settle(page);

  // (6) eco = nothing moves, outside the status indicators.
  if (level === "eco") {
    const running = await page.evaluate(() => window.__motion.running().map(window.__motion.describe));
    check(running.length === 0, `${tag} ${scenario.name}: nothing animates in eco`, json(running));
  }
  // (7) reduced = no displacement, nothing over 140 ms.
  if (level === "reduced") {
    const running = await page.evaluate(() => window.__motion.running().map(window.__motion.describe));
    for (const animation of running) {
      check(animation.duration <= 140, `${tag} ${scenario.name}: ${animation.name} ≤ 140 ms`, `${animation.duration} ms`);
      const moved = animation.keys.filter((key) => /^(transform|translate|scale|rotate)$/.test(key));
      check(moved.length === 0, `${tag} ${scenario.name}: ${animation.name} carries no transform`, json(moved));
    }
  }
  return requests.slice(mark).sort();
}

// -- the walk ----------------------------------------------------------------

const requestSets = new Map();

for (const [label, viewport, phone] of VIEWPORTS) {
  for (const level of LEVELS) {
    say(`\n— ${label} · ${level}`);
    const { ctx, page, errors, authed } = await open(level, viewport);
    const requests = [];
    watch(page, requests);

    check(authed, `${label} · ${level}: signed in (the shell, not the login screen)`);
    await foundation(page, level, phone);
    await shot(page, `${phone ? "390" : "1440"}-${level}-accueil`);

    for (const scenario of scenarios) {
      const tag = `${phone ? "390" : "1440"}-${level}`;
      const seen = await runScenario(page, requests, scenario, level, tag);
      requestSets.set(`${scenario.name}|${label}|${level}`, seen);
      await shot(page, `${tag}-${scenario.name}`);
    }

    // (5) A transition never adds a request: the three levels ask for the same
    // /api paths and images, in the same multiset.
    for (const scenario of scenarios) {
      const mine = requestSets.get(`${scenario.name}|${label}|${level}`);
      const reference = requestSets.get(`${scenario.name}|${label}|full`);
      if (level !== "full" && reference) {
        check(
          json(mine) === json(reference),
          `${label} ${scenario.name}: requests unchanged in ${level}`,
          `${json(mine)} vs ${json(reference)}`,
        );
      }
    }

    check(errors.length === 0, `${label} · ${level}: no page error`, errors.join(" | "));
    await ctx.close();
  }
}

await browser.close();
server.stop();

say(
  `\n${state.failed === 0 ? "OK" : "ÉCHEC"} — ${scenarios.length} scénario(s) ` +
    `(${scenarioFiles.length} fichier(s)) × ${VIEWPORTS.length} viewports × ${LEVELS.length} niveaux ` +
    `(${LEVELS.join(", ")}), ${state.failed} échec(s).`,
);
process.exit(state.failed === 0 ? 0 : 1);
