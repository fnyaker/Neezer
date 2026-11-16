#!/usr/bin/env node
// The motion bench: what the transition plan's rules cost and do, on the real
// app, in headless Chromium.
//
//   npm run build && node test/motion/run.mjs
//   DIST=/tmp/build node test/motion/run.mjs        # a build made elsewhere
//   SHOTS=/tmp/motion node test/motion/run.mjs      # PNGs of every leg
//   ONLY=eco node test/motion/run.mjs               # one motion level
//   VIEWPORT=390 FILES=home-search node test/motion/run.mjs   # one viewport, one file
//   SELFTEST=1 node test/motion/run.mjs             # the requests check's canary
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
// animation in eco, an unmarked one is stopped. The reduced-motion hover rule
// is judged on the PROBE's own animations: the app may legitimately still be
// moving elsewhere (a lot's page or shelf entrance is a 140 ms fade that lands
// right after boot), and holding that against the `.btn` rule reported the
// app's work as a broken guard — what matters is that a `.btn` hover starts
// nothing, which is what is checked.
//
// EVERY LATER LOT adds `scenarios/<lot>.mjs` (see scenarios/README.md) with one
// scenario per inventory row that has a clickable trigger; this runner then
// holds each of them, in the three levels, to:
//   - the same requests in the three levels (a transition never adds a request:
//     CLAUDE.md rule 3) — as SHAPES (a UUID is `<id>`), unique by URL, minus the
//     named background families below;
//   - eco: no animation at all outside `[data-motion="status"]`;
//   - reduced: no running Animation whose keyframes carry a transform, and no
//     duration over 140 ms (M14);
// and takes the pictures the plan asks for (SHOTS=<dir>).
//
// ONE FRESH PAGE PER SCENARIO FILE (and one for the foundation): a page is a
// session, and a session warms the cover cache and starts the shell's own
// background work. Sharing one page across files made a scenario's requests
// depend on the file that ran before it (measured: the same home scenario read
// 0 failure alone and 5 across the three levels once three files shared the
// page), so every file now boots its own. Before a window is read the page is
// also made QUIET — nothing new asked for three frames running, no picture
// still loading, no instant transition left — because the cover queue admits six
// pictures at a time from a horizon it re-scores every frame. The families of
// requests that are NOT a scenario's own (the shell's warm-up, playback, the
// sleeve pipeline) are named in `BACKGROUND_REQUESTS` below.
//
// SELFTEST=1 arms one extra scenario that adds a fetch while a transition runs
// (full motion only), so the requests check MUST fail: it is how the bench
// proves it has not gone blind. Run it after changing that check.

import { existsSync, mkdirSync, readdirSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import { join } from "node:path";
import { checker, loadPlaywright, login, startServer } from "../harness.mjs";

const LEVELS = (process.env.ONLY ? process.env.ONLY.split(",") : ["full", "reduced", "eco"]).filter((level) =>
  ["full", "reduced", "eco"].includes(level),
);
const SHOTS = process.env.SHOTS || null;
if (SHOTS) mkdirSync(SHOTS, { recursive: true });

/** Two switches for a shared machine: one viewport (`VIEWPORT=390`) and one or
 *  several scenario files (`FILES=home-search,library`, the file name without
 *  `.mjs`). A full walk boots a page per file per level; narrowing it is what
 *  makes a run fit beside nineteen other workers' benches, and it is how a lot
 *  proves its own file alone AND beside the others. */
const ONLY_VIEWPORT = process.env.VIEWPORT || null;
const ONLY_FILES = process.env.FILES ? process.env.FILES.split(",").map((name) => name.trim()) : null;

const VIEWPORTS = [
  ["bureau 1440", { viewport: { width: 1440, height: 900 } }, false],
  [
    "téléphone 390",
    { viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true },
    true,
  ],
].filter(([, , phone]) => !ONLY_VIEWPORT || String(phone ? 390 : 1440) === ONLY_VIEWPORT);

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
  // Deezer's CDN and public API are never reached by a bench: matched on the
  // HOSTNAME with a predicate, not on the URL text — an unanchored pattern may
  // match anywhere in the URL (CodeQL: "missing regular expression anchor",
  // reported on this very line).
  await ctx.route(
    (url) => url.hostname === "api.deezer.com" || url.hostname === "dzcdn.net" || url.hostname.endsWith(".dzcdn.net"),
    (route) => route.abort(),
  );
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
  // The shell's own first-paint entrances (S2's `.layout` fade, a cover's fade)
  // are still running when the page first answers: let them finish, or the
  // foundation's "reduced motion starts no transition" probe measures THEM
  // instead of the `.btn` it hovers. Bounded — a page that never goes quiet is
  // not this line's business.
  await page.waitForFunction(() => window.__motion.running().length === 0, null, { timeout: 5000 }).catch(() => {});
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
    // What must be gone three frames later is MOTION. The guard's own 0.01 ms
    // is a remnant, not motion (`describe` rounds it to 0), and any surface of
    // the app can make one at any moment — a sleeve decoding, a row mounting.
    // Measured: a cover's `.loaded` remnant, on an eco page where nothing else
    // moved, failed this check while the guard itself was holding.
    const moving = hover.later.filter((animation) => animation.duration > 0);
    check(moving.length === 0, `${tag}: nothing moves three frames after a .btn hover in eco`, json(moving));
    const remnants = hover.later.length - moving.length;
    if (remnants) say(`  ${tag} : ${remnants} instant remnant(s) on the timeline (0 ms, not motion)`);
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
    // bench says so rather than trusting a comment. What is judged here is the
    // PROBE's own animations — the button this bench created and hovered. The
    // app may still be moving somewhere else at this instant (a shelf or page
    // entrance is a 140 ms fade that lands right after boot, which is a lot's
    // work and not a broken `.btn` rule); those are reported, never counted as
    // a failure. The intent is unchanged: a `.btn` hover starts NOTHING.
    const probe = hover.early.filter((animation) => animation.probe);
    check(probe.length === 0, `${tag}: a .btn hover starts no transition under reduced motion`, json(probe));
    const elsewhere = hover.early.filter((animation) => !animation.probe);
    if (elsewhere.length) {
      say(
        `  ${tag} : ${elsewhere.length} animation(s) of the app still running (not the probe, so not the .btn rule) — ` +
          json(elsewhere.map((animation) => [animation.name, animation.duration])),
      );
    }
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

/** One entry per scenario FILE: each file gets its own page per level (below). */
const scenarioDir = fileURLToPath(new URL("./scenarios/", import.meta.url));
const files = [];
for (const name of readdirSync(scenarioDir).filter((entry) => entry.endsWith(".mjs")).sort()) {
  const module = await import(pathToFileURL(join(scenarioDir, name)).href);
  if (!module.scenarios?.length) continue;
  const short = name.replace(/\.mjs$/, "");
  if (ONLY_FILES && !ONLY_FILES.includes(short)) continue;
  files.push({ name, scenarios: module.scenarios });
}

/** SELFTEST: the requests check's canary. One scenario that ADDS a fetch — at
 *  full motion only, where a transition runs — must make check (5) fail. It is
 *  never part of a normal run. */
if (process.env.SELFTEST) {
  files.push({
    name: "selftest",
    scenarios: [
      {
        name: "auto-test: un fetch ajouté pendant la transition",
        async act(page) {
          const level = await page.evaluate(() =>
            document.documentElement.dataset.eco === "1"
              ? "eco"
              : matchMedia("(prefers-reduced-motion: reduce)").matches
                ? "reduced"
                : "full",
          );
          // Only where motion is the difference between the levels.
          if (level === "full") await page.evaluate(() => fetch("/api/selftest-transition").catch(() => {}));
        },
      },
    ],
  });
  say("SELFTEST: a scenario that adds a fetch is armed — check (5) MUST fail on it.");
}

/** The /api and image requests a page asked for: the "nothing extra" check.
 *  `image` marks the pictures, which are compared by URL (see askedFor). */
function watch(page, requests) {
  page.on("request", (request) => {
    const url = new URL(request.url());
    const image = request.resourceType() === "image";
    if (url.pathname.startsWith("/api/") || image) {
      requests.push({ path: url.pathname.replace(/\?.*$/, ""), image });
    }
  });
}

/** Requests that are NOT a scenario's own, and therefore must not be held
 *  against a level: the shell's background work and whatever is playing. Both
 *  are driven by their own clocks, so they land in whichever window happens to
 *  be open — measured on the merged tree (T4: the warm-up drips once every
 *  1.5 s; a playing track pulls its bytes, gains, analysis and edges). A fetch
 *  a TRANSITION added is still caught: it happens at full motion only, so the
 *  three levels disagree on a path that is not in this list. */
const BACKGROUND_REQUESTS = [
  // warmPlaylists (lib/actions.js): ten seconds after a boot, `/me/favorites`
  // then up to 25 `/playlist/<id>` payloads, one every WARM_GAP (1.5 s), to
  // warm the offline cache. Every fresh page restarts it.
  /^\/api\/me\/favorites$/,
  /^\/api\/playlist\/[^/]+$/,
  // The sleeve pipeline (lib/coverqueue.js): six pictures in flight, admitted
  // from a horizon it re-scores every frame, each slot freed by a decode — so
  // WHICH sleeves a window asked for depends on decode timing, not on the
  // screen: the same home scenario asked for 76 `/api/cover/<id>` in full and
  // 80–87 in the other two levels. Named here, and measured by its own benches
  // (test/glass, test/backdrop, T4's list), never a verdict on a transition.
  /^\/api\/cover\//,
  /^\/images\/cover\//,
  // Playback: the bytes of the track and everything the player asks about it.
  /^\/api\/stream\//,
  /^\/api\/gains$/,
  /^\/api\/analyses$/,
  /^\/api\/audio\/edges\//,
  /^\/api\/lyrics\//,
];
const isBackground = (path) => BACKGROUND_REQUESTS.some((pattern) => pattern.test(path));

/** A path's SHAPE: its UUIDs and long numbers become `<id>`/`<n>`. A scenario's
 *  own setup may legitimately ask about a different entity in another level —
 *  the fixture's "first followed artist" drifts as the walks play things — and
 *  no transition changes WHICH entity is asked for; what a transition could add
 *  is a KIND of request, and that is what check (5) must see. */
const shape = (path) =>
  path
    .replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi, "<id>")
    .replace(/\/(\d{3,})(?=\/|$)/g, "/<n>");

/** What one scenario asked for, as check (5) compares it: the SHAPE of every
 *  /api path and every image URL, UNIQUE (the same picture requested twice is
 *  one picture — the cover queue is frame-sliced and may re-ask), without the
 *  background families above. */
function askedFor(requests) {
  const paths = new Set();
  for (const request of requests) {
    if (isBackground(request.path)) continue;
    paths.add(shape(request.path));
  }
  return [...paths].sort();
}

/** A window is read on a QUIET page, and "quiet" is about what the page ASKS
 *  FOR, not about the pictures we happen to know: the cover queue admits six
 *  pictures at a time and re-scores what is mounted on every frame, so a window
 *  that ends mid-admission has asked for a different set of art at each level —
 *  measured on the merged tree as ~45 `/api/cover/<id>` against ~44.
 *  `PerformanceObserver` sees every resource the page loads (a fetch, an XHR, an
 *  `<img>`), so quiescence is "three frames in a row with nothing new asked
 *  for", plus no image still loading so a decode that lands on the last frame
 *  is not missed, plus no instant transition left (eco's guard is 0.01 ms, and
 *  the animation checks would read that remnant as "something still runs").
 *  Frames, not a clock, with a 240-frame budget so a page that never settles
 *  cannot hang the bench. */
async function quiet(page) {
  await page.evaluate(async () => {
    if (!window.__quietSeen) {
      window.__quietSeen = 0;
      const watch = new PerformanceObserver((list) => {
        window.__quietSeen += list.getEntries().length;
      });
      try {
        watch.observe({ type: "resource", buffered: false });
      } catch {
        /* no resource timing: the image and animation waits below still hold */
      }
      window.__quietWatch = watch;
    }
    const frame = () => new Promise((resolve) => requestAnimationFrame(resolve));
    let still = 0;
    let last = -1;
    for (let i = 0; i < 240; i++) {
      const seen = window.__quietSeen;
      const loading = [...document.images].some((img) => img.isConnected && !img.complete);
      const instant = document
        .getAnimations()
        .some((animation) => animation.effect && animation.effect.getTiming().duration < 0.1);
      still = !loading && !instant && seen === last ? still + 1 : 0;
      last = seen;
      if (still >= 3) return;
      await frame();
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
  await quiet(page);

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
  return askedFor(requests.slice(mark));
}

// -- the walk ----------------------------------------------------------------

/** Each scenario's window, per viewport and level: keyed with its FILE, so two
 *  files may name a scenario the same. */
const requestSets = new Map();

/** (5) A transition never adds a request: the three levels ask for the same
 *  /api paths and pictures. `full` is the reference and runs first. */
function compareRequests(file, scenarios, label, level) {
  if (level === "full") return;
  for (const scenario of scenarios) {
    const mine = requestSets.get(`${file}|${scenario.name}|${label}|${level}`);
    const reference = requestSets.get(`${file}|${scenario.name}|${label}|full`);
    if (!reference) continue;
    check(
      json(mine) === json(reference),
      `${label} ${file} ${scenario.name}: requests unchanged in ${level}`,
      `${json(mine)} vs ${json(reference)}`,
    );
  }
}

for (const [label, viewport, phone] of VIEWPORTS) {
  const tagOf = (level) => `${phone ? "390" : "1440"}-${level}`;

  // The foundation (tokens, guard, probe), on a page of its own: it answers for
  // the whole app, not for a file.
  for (const level of LEVELS) {
    say(`\n— ${label} · ${level} · fondation`);
    const { ctx, page, errors, authed } = await open(level, viewport);
    check(authed, `${label} · ${level}: signed in (the shell, not the login screen)`);
    await foundation(page, level, phone);
    await shot(page, `${tagOf(level)}-accueil`);
    check(errors.length === 0, `${label} · ${level}: no page error`, errors.join(" | "));
    await ctx.close();
  }

  // Then a FRESH page per file and per level: a page is a session, and a
  // session's warm caches and background work must not decide what the next
  // file's windows ask for (measured: one home scenario read 0 failure alone
  // and 5 across levels once three files shared a page).
  for (const level of LEVELS) {
    for (const file of files) {
      say(`\n— ${label} · ${level} · ${file.name}`);
      const { ctx, page, errors, authed } = await open(level, viewport);
      const requests = [];
      watch(page, requests);
      check(authed, `${label} · ${level} ${file.name}: signed in (the shell, not the login screen)`);

      for (const scenario of file.scenarios) {
        const tag = tagOf(level);
        requestSets.set(
          `${file.name}|${scenario.name}|${label}|${level}`,
          await runScenario(page, requests, scenario, level, tag),
        );
        await shot(page, `${tag}-${scenario.name}`);
      }

      compareRequests(file.name, file.scenarios, label, level);
      check(errors.length === 0, `${label} · ${level} ${file.name}: no page error`, errors.join(" | "));
      await ctx.close();
    }
  }
}

await browser.close();
server.stop();

const total = files.reduce((count, file) => count + file.scenarios.length, 0);
say(
  `\n${state.failed === 0 ? "OK" : "ÉCHEC"} — ${total} scénario(s) ` +
    `(${files.length} fichier(s)) × ${VIEWPORTS.length} viewports × ${LEVELS.length} niveaux ` +
    `(${LEVELS.join(", ")}), ${state.failed} échec(s).`,
);
process.exit(state.failed === 0 ? 0 : 1);
