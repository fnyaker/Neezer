#!/usr/bin/env node
// The theme changing with the sleeve, on both players.
//
//   npx vite build --outDir /tmp/neezer-theme && DIST=/tmp/neezer-theme node test/theme/run.mjs
//
// Two covers as different as a sleeve can be (a dark green and a red) are
// served for the first two titles of the 4 000-track playlist, and one « Suivant »
// is measured: `--cover-ac` (Pochette's whole accent, hence --ac/--btn/--hrt)
// must walk between the two colours instead of jumping, the blurred ground must
// never uncover the dark page under it (the opaque base used to be dropped the
// moment a second cover arrived), and nothing may poll after the skip.
//
// `prefers-reduced-motion` is a separate context: the colour is the new one at
// the first frame. Another context boots with a stored accent and checks the
// first frame already paints it, never the designed fallback.
import { resolve } from "node:path";
import { existsSync } from "node:fs";
import { loadPlaywright, startServer, login, checker } from "../harness.mjs";
import { accentFor, fromHex } from "../../src/lib/sillon.js";

const { chromium } = await loadPlaywright();
const { base, ids, stop } = await startServer(["--podcasts"]);
const { check, state } = checker();
const SHOTS = process.env.SHOTS;

const FADE = 600;
const say = (s) => console.log(s);

const WAV = (() => {
  const rate = 8000;
  const n = rate * 130;
  const h = Buffer.alloc(44);
  h.write("RIFF", 0);
  h.writeUInt32LE(36 + n, 4);
  h.write("WAVEfmt ", 8);
  h.writeUInt32LE(16, 16);
  h.writeUInt16LE(1, 20);
  h.writeUInt16LE(1, 22);
  h.writeUInt32LE(rate, 24);
  h.writeUInt32LE(rate, 28);
  h.writeUInt16LE(1, 32);
  h.writeUInt16LE(8, 34);
  h.write("data", 36);
  h.writeUInt32LE(n, 40);
  return Buffer.concat([h, Buffer.alloc(n, 128)]);
})();

const GREEN = "#0a6b3c";
const RED = "#c0182a";
const GREEN_AC = accentFor(fromHex(GREEN));
const RED_AC = accentFor(fromHex(RED));

// Real PNGs, not SVG: the app caches cover bytes as blobs and drops one
// `createImageBitmap` cannot decode (offline.js), which an SVG blob is — the
// sleeve would show its groove placeholder while the accent worked.
async function coverPngs(page) {
  const urls = await page.evaluate(([a, b]) => {
    const draw = (fill) => {
      const c = document.createElement("canvas");
      c.width = c.height = 64;
      const x = c.getContext("2d");
      x.fillStyle = fill;
      x.fillRect(0, 0, 64, 64);
      return c.toDataURL("image/png");
    };
    return [draw(a), draw(b), draw("#3a3a3a"), draw("#0f8a7a")];
  }, [GREEN, RED]);
  return urls.map((u) => Buffer.from(u.split(",")[1], "base64"));
}

const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM || (existsSync("/usr/bin/chromium") ? "/usr/bin/chromium" : undefined),
  args: ["--no-sandbox", "--autoplay-policy=no-user-gesture-required"],
});

const probe = await browser.newContext();
const probePage = await probe.newPage();
await probePage.goto(base + "/app/");
await probePage.waitForTimeout(300);
await login(probePage, base);
const payload = await probePage.evaluate(async (id) => (await fetch("/api/playlist/" + id)).json(), ids.big);
const [GREEN_PNG, RED_PNG, GREY_PNG, TEAL_PNG] = await coverPngs(probePage);
await probe.close();
const first = payload.tracks[0];
const second = payload.tracks.find((t, i) => i > 0 && t.album?.cover && t.album.cover !== first.album?.cover);
if (!first?.album?.cover || !second?.album?.cover) throw new Error("the bench playlist needs two different sleeves");
const pathOf = (u) => new URL(u, "http://x/").pathname;
const COVER = new Map([
  [pathOf(first.album.cover), GREEN_PNG],
  [pathOf(second.album.cover), RED_PNG],
  ["/api/cover/podcast-theme-bench", TEAL_PNG],
]);
// The play cache stores the art the server proxies for the TRACK id under the
// album cover's key (playcache.cacheCoverFor), and Cover prefers that blob.
for (const [track, png] of [
  [first, GREEN_PNG],
  [second, RED_PNG],
])
  for (const id of [track.deezer_id, track.id, track.uuid])
    if (id) COVER.set(`/api/cover/${id}`, png);
say(`« ${first.title} » (${GREEN} → ${GREEN_AC}) → « ${second.title} » (${RED} → ${RED_AC})`);

async function context({ width, height, phone, reduced }) {
  const ctx = await browser.newContext({
    viewport: { width, height },
    deviceScaleFactor: phone ? 2 : 1,
    isMobile: phone,
    hasTouch: phone,
    reducedMotion: reduced ? "reduce" : "no-preference",
  });
  await ctx.route(/api\.deezer\.com|dzcdn\.net/, (r) => r.abort());
  await ctx.route(
    /\/api\/stream\//,
    (r) => r.fulfill({ status: 200, body: WAV, headers: { "Content-Type": "audio/wav", "Accept-Ranges": "none" } })
  );
  await ctx.route(/\/api\/cover\//, (r) => {
    const png = COVER.get(new URL(r.request().url()).pathname) || GREY_PNG;
    r.fulfill({ status: 200, contentType: "image/png", body: png });
  });
  await ctx.addInitScript(() => {
    try {
      localStorage.setItem("ui.theme", JSON.stringify("pochette"));
    } catch {
      /* no storage */
    }
  });
  return ctx;
}

async function openPlayer(ctx, { phone }) {
  const page = await ctx.newPage();
  page.on("pageerror", (e) => say("pageerror: " + e.message));
  await page.goto(base + "/app/");
  await login(page, base);
  await page.goto(base + "/app/");
  await page.waitForTimeout(600);
  await page.goto(base + "/app/#/playlist/" + ids.big);
  await page.waitForSelector(".row.track", { timeout: 60000 });
  await page.waitForTimeout(600);
  await page.locator(".row.track .titles").first().click();
  await page.waitForTimeout(1200);
  if (phone) {
    await page.locator("footer.player").click({ position: { x: 4, y: 4 } });
    await page.locator(".m").waitFor({ state: "visible", timeout: 20000 });
    await page.waitForSelector(".m .bg", { timeout: 20000 });
  } else {
    await page.locator('button[aria-label="Plein écran"]').first().click();
    await page.waitForSelector(".d .bg", { timeout: 20000 });
  }
  await page.waitForTimeout(1400);
  return page;
}

async function sample(page, { twice }) {
  const requests = [];
  const onReq = (r) => {
    if (r.url().includes("/api/")) requests.push({ at: Date.now(), url: r.url().replace(base, "") });
  };
  page.on("request", onReq);
  const out = await page.evaluate(
    async ({ twice, fade }) => {
      const rows = [];
      const gaps = [];
      const canvas = document.createElement("canvas");
      canvas.width = canvas.height = 1;
      const ix = canvas.getContext("2d", { willReadFrequently: true });
      const toRgb = (css) => {
        ix.fillStyle = "#000000";
        ix.fillStyle = css;
        ix.clearRect(0, 0, 1, 1);
        ix.fillRect(0, 0, 1, 1);
        const d = ix.getImageData(0, 0, 1, 1).data;
        return [d[0], d[1], d[2]];
      };
      let long = 0;
      let observer;
      let clicked = false;
      try {
        observer = new PerformanceObserver((list) => {
          for (const e of list.getEntries()) long += e.duration;
        });
        observer.observe({ type: "longtask", buffered: false });
      } catch {
        /* no longtask */
      }
      const next = () => {
        const b = [...document.querySelectorAll(".d button, .m button")].find(
          (x) => x.getAttribute("aria-label") === "Suivant"
        );
        b?.click();
      };
      const t0 = performance.now();
      let last = t0;
      next();
      const loop = () => {
        const now = performance.now();
        gaps.push(now - last);
        last = now;
        const raw = getComputedStyle(document.documentElement).getPropertyValue("--cover-ac").trim();
        rows.push({
          t: +(now - t0).toFixed(1),
          ac: toRgb(raw),
          raw,
          op: [...document.querySelectorAll(".d .bg, .m .bg")].map((e) => +getComputedStyle(e).opacity),
        });
        if (twice && !clicked && now - t0 >= 240) {
          clicked = true;
          next();
        }
        if (now - t0 < fade + 900) requestAnimationFrame(loop);
      };
      requestAnimationFrame(loop);
      await new Promise((r) => setTimeout(r, fade + 1000));
      observer?.disconnect();
      return { rows, gaps: gaps.slice(2), long };
    },
    { twice, fade: FADE }
  );
  page.off("request", onReq);
  return { ...out, requests };
}

const readAccent = () => {
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = 1;
  const ix = canvas.getContext("2d", { willReadFrequently: true });
  const css = getComputedStyle(document.documentElement).getPropertyValue("--cover-ac").trim();
  ix.fillStyle = "#000000";
  ix.fillStyle = css;
  ix.clearRect(0, 0, 1, 1);
  ix.fillRect(0, 0, 1, 1);
  const d = ix.getImageData(0, 0, 1, 1).data;
  return [d[0], d[1], d[2]];
};
const rgb = (v) => {
  if (Array.isArray(v)) return v;
  const s = String(v ?? "").trim();
  if (s.startsWith("#")) return fromHex(s);
  const m = s.match(/-?\d+(\.\d+)?/g);
  return m ? m.slice(0, 3).map(Number) : null;
};
const dist3 = (a, b) => Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
const near = (a, b, e = 3) => !!a && !!b && dist3(a, b) <= e;

for (const [label, size, phone] of [
  ["bureau 1280", { width: 1280, height: 800, phone: false }, false],
  ["téléphone 390", { width: 390, height: 844, phone: true }, true],
]) {
  const ctx = await context({ ...size, reduced: false });
  const page = await openPlayer(ctx, { phone });

  const start = await page.evaluate(readAccent);
  const run = await sample(page, { twice: false });
  await page.waitForTimeout(FADE + 300);
  const end = await page.evaluate(readAccent);
  const { rows, gaps, long, requests } = run;

  check(near(start, fromHex(GREEN_AC), 4), `${label} : l'accent part de la pochette en cours`, `${start} pour ${GREEN_AC}`);
  check(near(end, fromHex(RED_AC), 4), `${label} : l'accent arrive sur la nouvelle pochette`, `${end} pour ${RED_AC}`);

  const firstChanged = rows.find((r) => !near(rgb(r.ac), start, 2));
  const settled = rows.find(
    (r) => near(rgb(r.ac), end, 2) && rows.slice(rows.indexOf(r)).every((x) => near(rgb(x.ac), end, 3))
  );
  const mid = rows.find((r) => r.t >= FADE / 2);
  const midColour = mid ? rgb(mid.ac) : null;
  check(
    settled && settled.t >= 450 && settled.t <= 900,
    `${label} : la couleur est posée ${FADE} ms après le clic (${Math.round(settled?.t || 0)} ms mesurés)`,
    `premier mouvement ${firstChanged?.t} ms, posé ${settled?.t} ms`
  );
  check(
    !!midColour && !near(midColour, start, 12) && !near(midColour, end, 12),
    `${label} : à mi-course la couleur est ENTRE les deux (pas un saut)`,
    `milieu ${midColour}`
  );
  const total = dist3(start || [0, 0, 0], end || [0, 0, 0]);
  const jump = rows.reduce((m, r, i) => (i ? Math.max(m, dist3(rgb(r.ac), rgb(rows[i - 1].ac))) : m), 0);
  check(jump < total * 0.6, `${label} : aucun saut d'un cadre à l'autre (${jump.toFixed(1)} sur ${total.toFixed(1)})`, "");

  const hole = rows.filter((r) => Math.max(...r.op, 0) < 0.99);
  check(
    hole.length === 0,
    `${label} : le fond ne découvre jamais la page (${rows.length} cadres)`,
    hole.slice(0, 3).map((r) => `${r.t}ms ${r.op}`).join(" ; ")
  );
  const maxLayers = Math.max(...rows.map((r) => r.op.length));
  check(maxLayers <= 8, `${label} : au plus 8 couches de fond (${maxLayers} montées)`, "");
  const gap = Math.max(...gaps);
  check(gap < 250, `${label} : aucun gel de l'interface (${gap.toFixed(1)} ms max)`, `longtasks ${long.toFixed(1)} ms`);
  say(
    `${label} : accent ${start} → ${end}, posé à ${Math.round(settled?.t || 0)} ms, milieu ${midColour}, ` +
      `cadres ${rows.length}, couches ≤ ${maxLayers}, pas max ${gap.toFixed(1)} ms, longtask ${long.toFixed(1)} ms`
  );

  const mine = [pathOf(first.album.cover), pathOf(second.album.cover)];
  const covers = requests.filter((r) => mine.includes(pathOf(r.url))).length;
  const tail = requests.filter((r) => r.at > Date.now() - 700);
  check(covers <= 3, `${label} : ${covers} requête(s) pour les deux pochettes du saut`, requests.map((r) => r.url).join(" ; "));
  check(
    tail.length === 0,
    `${label} : plus aucune requête une fois posé (${tail.length} en fin de fenêtre)`,
    tail.map((r) => r.url).join(" ; ")
  );

  if (SHOTS) await page.screenshot({ path: resolve(SHOTS, `theme-${phone ? "phone" : "desktop"}.png`) });
  await ctx.close();

  const rapid = await context({ ...size, reduced: false });
  const rapidPage = await openPlayer(rapid, { phone });
  if (SHOTS) {
    await rapidPage.evaluate(() => {
      const b = [...document.querySelectorAll(".d button, .m button")].find((x) => x.getAttribute("aria-label") === "Suivant");
      b?.click();
    });
    await rapidPage.waitForTimeout(300);
    await rapidPage.screenshot({ path: resolve(SHOTS, `theme-${phone ? "phone" : "desktop"}-mid.png`) });
    await rapidPage.waitForTimeout(800);
  }
  const twice = await sample(rapidPage, { twice: true });
  const deepest = Math.min(...twice.rows.map((r) => Math.max(...r.op, 0)));
  check(
    deepest >= 0.9,
    `${label} : deux sauts rapprochés ne découvrent pas le fond (${(100 - deepest * 100).toFixed(1)} % au pire)`,
    twice.rows
      .filter((r) => Math.max(...r.op, 0) < 0.99)
      .slice(0, 3)
      .map((r) => `${r.t}ms ${r.op}`)
      .join(" ; ")
  );
  check(
    Math.max(...twice.rows.map((r) => r.op.length)) <= 8,
    `${label} : la rafale reste bornée à 8 couches`,
    `${Math.max(...twice.rows.map((r) => r.op.length))}`
  );
  await rapid.close();
}

{
  const ctx = await context({ width: 1280, height: 800, phone: false, reduced: true });
  const page = await openPlayer(ctx, { phone: false });
  const before = await page.evaluate(readAccent);
  const motion = await page.evaluate(() => ({
    rootTransition: getComputedStyle(document.documentElement).transitionProperty,
    bgAnimation: getComputedStyle(document.querySelector(".d .bg")).animationName,
  }));
  const { rows } = await sample(page, { twice: false });
  const firstFrame = rows.find((r) => r.t > 16) || rows[0];
  check(motion.rootTransition === "none", "reduced motion : aucune transition sur --cover-ac", motion.rootTransition);
  check(motion.bgAnimation === "none", "reduced motion : aucune animation de fond", motion.bgAnimation);
  check(
    !near(rgb(firstFrame.ac), before, 2),
    "reduced motion : la couleur est la nouvelle dès le premier cadre",
    `${before} → ${firstFrame.ac} à ${firstFrame.t} ms`
  );
  const hole = rows.filter((r) => Math.max(...r.op, 0) < 0.99);
  check(hole.length === 0, "reduced motion : le fond reste opaque", `${hole.length}`);
  await ctx.close();
}

{
  const ctx = await context({ width: 1280, height: 800, phone: false, reduced: false });
  await ctx.addInitScript(() => {
    try {
      localStorage.setItem("ui.coverAc", JSON.stringify("#123456"));
    } catch {
      /* no storage */
    }
    window.__firstFrames = [];
    const read = () => {
      const canvas = document.createElement("canvas");
      canvas.width = canvas.height = 1;
      const ix = canvas.getContext("2d");
      ix.fillStyle = "#000000";
      ix.fillStyle = getComputedStyle(document.documentElement).getPropertyValue("--cover-ac").trim();
      ix.fillRect(0, 0, 1, 1);
      const d = ix.getImageData(0, 0, 1, 1).data;
      window.__firstFrames.push([d[0], d[1], d[2]]);
    };
    requestAnimationFrame(() => {
      read();
      requestAnimationFrame(read);
    });
  });
  const page = await ctx.newPage();
  await page.goto(base + "/app/");
  await page.waitForTimeout(200);
  const early = await page.evaluate(() => window.__firstFrames);
  check(near(early[0], fromHex("#123456"), 1), "démarrage : la première image peint l'accent mémorisé", early[0]);
  check(near(early[1], fromHex("#123456"), 1), "démarrage : aucune transition depuis le repli", early[1]);
  await ctx.close();
}

{
  const ctx = await context({ width: 1280, height: 900, phone: false, reduced: false });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => say("pageerror: " + e.message));
  await page.goto(base + "/app/");
  await login(page, base);
  await page.goto(base + "/app/");
  await page.waitForTimeout(700);
  const show = ids.podcast?.show;
  check(!!show, "en-tête de pochette : une émission du banc", String(show));
  const real = await page.evaluate(async (id) => (await fetch("/api/podcast/" + id)).json(), show);
  real.cover = "/api/cover/podcast-theme-bench";
  await ctx.route(new RegExp("/api/podcast/" + show + "(\\?|$)"), (r) =>
    r.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(real) })
  );
  await page.goto(base + "/app/#/podcasts");
  await page.waitForTimeout(600);

  const sampler = () => {
    window.__gh = [];
    const t0 = performance.now();
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = 1;
    const ix = canvas.getContext("2d", { willReadFrequently: true });
    const loop = () => {
      const gh = document.querySelector(".gh");
      if (gh) {
        const cs = getComputedStyle(gh);
        const css = cs.getPropertyValue("--gh-c").trim();
        ix.fillStyle = "#000000";
        ix.fillStyle = css;
        ix.clearRect(0, 0, 1, 1);
        ix.fillRect(0, 0, 1, 1);
        const d = ix.getImageData(0, 0, 1, 1).data;
        window.__gh.push({
          t: +(performance.now() - t0).toFixed(0),
          c: [d[0], d[1], d[2]],
          bg: cs.backgroundImage,
        });
      }
      if (performance.now() - t0 < 2600) requestAnimationFrame(loop);
    };
    requestAnimationFrame(loop);
  };
  const run = async () => {
    const rows = await page.evaluate(() => window.__gh);
    const fallback = fromHex("#3a2a55");
    const teal = fromHex("#0f8a7a");
    const changed = rows.findIndex((r) => dist3(r.c, fallback) > 2);
    const first = rows[changed];
    const flat = changed > 0 ? rows[changed - 1] : null;
    const settled = rows.find(
      (r) => near(r.c, teal, 2) && rows.slice(rows.indexOf(r)).every((x) => near(x.c, teal, 3))
    );
    return { rows, fallback, teal, first, flat, settled };
  };

  await page.evaluate(sampler);
  await page.evaluate((id) => (location.hash = "#/podcast/" + id), show);
  await page.waitForTimeout(2800);
  const normal = await run();
  const { rows, fallback, teal, first, flat, settled } = normal;
  check(rows.length > 30, `en-tête : ${rows.length} cadres échantillonnés`, "");
  check(!!first && near(first.c, fallback, 3), "en-tête : la teinte de repli peint d'abord", `${first?.c}`);
  check(!!settled && near(settled.c, teal, 3), "en-tête : la teinte de la pochette arrive", `${settled?.c} pour ${teal}`);
  const duration = settled && flat ? settled.t - flat.t : 0;
  check(
    duration >= 400 && duration <= 900,
    `en-tête : la teinte met 600 ms à arriver (${Math.round(duration)} ms mesurés)`,
    `dernier cadre au repli ${flat?.t} ms, posé ${settled?.t} ms`
  );
  const midRow = flat && settled ? rows.find((r) => r.t >= flat.t + (settled.t - flat.t) / 2) : null;
  check(
    !!midRow && dist3(midRow.c, fallback) > 12 && dist3(midRow.c, teal) > 12,
    "en-tête : à mi-course la teinte est ENTRE les deux",
    `${midRow?.c}`
  );
  const shades = new Set(
    (first && settled ? rows.filter((r) => r.t >= first.t && r.t <= settled.t) : []).map((r) => r.bg)
  );
  check(shades.size >= 5, `en-tête : le dégradé lui-même suit (${shades.size} valeurs)`, "");
  const gaps = rows.slice(1).map((r, i) => r.t - rows[i].t).filter((g) => g > 0);
  const worstGap = Math.max(...gaps);
  check(worstGap < 120, `en-tête : aucun gel pendant le fondu (${worstGap.toFixed(1)} ms max)`, "");
  const prop = await page.evaluate(() => getComputedStyle(document.querySelector(".gh")).transitionProperty);
  check(prop.includes("--gh-c"), "en-tête : la transition porte sur --gh-c", prop);
  say(
    `en-tête : ${fallback} → ${teal}, arrivée ${Math.round(duration)} ms, milieu ${midRow?.c}, ` +
      `${shades.size} nuances de dégradé`
  );
  if (SHOTS) {
    await page.screenshot({ path: resolve(SHOTS, "header-settled.png") });
  }

  await page.emulateMedia({ reducedMotion: "reduce" });
  const propReduce = await page.evaluate(() => getComputedStyle(document.querySelector(".gh")).transitionProperty);
  await page.evaluate(() => (location.hash = "#/podcasts"));
  await page.waitForTimeout(400);
  await page.evaluate(sampler);
  await page.evaluate((id) => (location.hash = "#/podcast/" + id), show);
  await page.waitForTimeout(1200);
  const reduced = await run();
  check(propReduce === "none", "en-tête : reduced motion coupe la transition", propReduce);
  const between = reduced.rows.filter(
    (r) => dist3(r.c, reduced.fallback) > 8 && dist3(r.c, reduced.teal) > 8
  );
  check(between.length === 0, "en-tête : reduced motion, aucune teinte intermédiaire", `${between.length}`);
  await ctx.close();
}

await browser.close();
stop();
console.log(state.failed ? `\n${state.failed} check(s) failed` : "\nall theme checks passed");
process.exit(state.failed ? 1 : 0);
