#!/usr/bin/env node
// The app's ground: one blurred sleeve behind every screen.
//
//   npm run build && node test/backdrop/run.mjs
//   DIST=/tmp/build node test/backdrop/run.mjs      # serve a build made elsewhere
//   SHOTS=/tmp/backdrop node test/backdrop/run.mjs  # PNGs of every case
//
// `components/GlobalBackdrop.svelte` puts the playing sleeve behind Accueil,
// Bibliothèque, Rechercher, Réglages and every entity page as ONE fixed
// 500 px image with ONE `filter: blur()` and a stable scrim. This bench holds
// that on real screens, at 1280×900 and 390×844, on a 8000-track library
// (`tools/perf_api.py --serve`), with the CDN answered by two flat sleeves:
//
//  - the ground is behind each of the seven routes, under the content and
//    under the player bar, and it SURVIVES a navigation (its painted URL does
//    not change);
//  - ONE visible blurred element outside the full-screen player, ZERO
//    `backdrop-filter` anywhere, and ONE `.gb-img` even mid-fade;
//  - it creates no stacking context and <body> stays transparent: the ground
//    is painted between the canvas and the pages, which is why the page title
//    is sampled — a title washed out by the layer is the bug that cost this;
//  - a skip is the 640 ms walk (out 220, swap, in 420) with no flash: the
//    scrim never moves, the new sleeve costs ONE load of the ground itself,
//    and nothing is asked for once it has settled;
//  - the full-screen player hides the ground (one blurred ground at a time);
//  - eco: no image, no blur, no scrim, `data-eco="1"`, ZERO cover requests,
//    and the attribute is REMOVED (not "0") when eco ends;
//  - contrast of --tx/--dim over a PURE-WHITE sleeve, sampled from the painted
//    pixels with the UI hidden (the worst ground a sleeve can make);
//  - `prefers-reduced-motion` swaps without fading;
//  - the cost: long tasks and frame gaps on a skip, on a cold boot at 4× CPU
//    (the ground's first paint is deferred to an idle moment), and the same
//    skip in eco as the baseline.
//
// Measured (1280×900 / 390×844): 1 blurred element, 0 backdrop-filter and 0
// cover requests on every route; a skip fades out at 30–65 ms, paints the new
// art at 228–245 ms and is full again at 578–603 ms, with 0 long tasks and a
// worst frame gap of 17–33 ms; over a white sleeve --dim keeps 5.00:1 (Encre)
// and 5.96:1 (Papier), --tx 11.8:1 and 16.3:1.
import { existsSync, mkdirSync } from "node:fs";
import { loadPlaywright, startServer, login, checker, deezerApi, dzcdn, pathOn, installSessionPrefix } from "../harness.mjs";

const { base, ids, stop } = await startServer(["--podcasts"]);
// An assertion that throws would otherwise leave the perf server running.
process.on("exit", () => {
  try {
    stop();
  } catch {
    /* already gone */
  }
});
const { chromium } = await loadPlaywright();
const { check, state } = checker();
const SHOTS = process.env.SHOTS || null;
if (SHOTS) mkdirSync(SHOTS, { recursive: true });
const say = (s) => console.log(s);
const snap = async (page, name) => {
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/${name}.png` });
};

const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM || (existsSync("/usr/bin/chromium") ? "/usr/bin/chromium" : undefined),
  args: ["--no-sandbox", "--autoplay-policy=no-user-gesture-required"],
});

// --- covers ---------------------------------------------------------------
const probe = await browser.newContext();
const probePage = await probe.newPage();
await probePage.goto(base + "/app/");
await probePage.waitForTimeout(400);
await login(probePage, base);
const payload = await probePage.evaluate(async (id) => (await fetch("/api/playlist/" + id)).json(), ids.big);
const pngs = await probePage.evaluate(() => {
  const draw = (fill, w = 500, h = 500) => {
    const c = document.createElement("canvas");
    c.width = w;
    c.height = h;
    const x = c.getContext("2d");
    x.fillStyle = fill;
    x.fillRect(0, 0, w, h);
    return c.toDataURL("image/png");
  };
  return {
    green: draw("#0a6b3c"),
    red: draw("#c0182a"),
    white: draw("#ffffff"),
    grey: draw("#3a3a3a"),
  };
});
await probe.close();
const PNG = Object.fromEntries(Object.entries(pngs).map(([k, u]) => [k, Buffer.from(u.split(",")[1], "base64")]));

const first = payload.tracks[0];
const second = payload.tracks.find((t, i) => i > 0 && t.album?.cover && t.album.cover !== first.album?.cover);
if (!first?.album?.cover || !second?.album?.cover) throw new Error("need two different sleeves");
const pathOf = (u) => new URL(u, "http://x/").pathname;
const ref = (t) => String(t.id || t.deezer_id || "");
const COVER = new Map([
  [pathOf(first.album.cover), PNG.green],
  [pathOf(second.album.cover), PNG.red],
]);
for (const [t, png] of [
  [first, PNG.green],
  [second, PNG.red],
]) {
  COVER.set(`/api/cover/${ref(t)}`, png);
}
const ALBUM_COVER = pathOf(first.album.cover);

// A real WAV: without it /api/stream 404s, the player errors and advances on
// its own, and the ground changes track under a navigation that did not.
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

async function context({ width, height, phone, cover = null }) {
  const ctx = await browser.newContext({
    viewport: { width, height },
    deviceScaleFactor: phone ? 2 : 1,
    isMobile: phone,
    hasTouch: phone,
  });
  await ctx.route(deezerApi, (r) => r.abort());
  await ctx.route(pathOn("/api/stream/"), (r) =>
    r.fulfill({ status: 200, body: WAV, headers: { "Content-Type": "audio/wav", "Accept-Ranges": "none" } })
  );
  const body = (path) => (cover === "white" ? PNG.white : COVER.get(path) || PNG.grey);
  await ctx.route(dzcdn, (r) => {
    r.fulfill({ status: 200, contentType: "image/png", body: body(new URL(r.request().url()).pathname) });
  });
  await ctx.route(pathOn("/api/cover/"), (r) => {
    r.fulfill({ status: 200, contentType: "image/png", body: body(new URL(r.request().url()).pathname) });
  });
  await ctx.addInitScript(
    () => {
      window.__long = [];
      try {
        new PerformanceObserver((l) => l.getEntries().forEach((e) => window.__long.push({ at: e.startTime, d: e.duration }))).observe(
          { entryTypes: ["longtask"] }
        );
      } catch {
        /* no longtask */
      }
    }
  );
  return ctx;
}

async function open(ctx) {
  const page = await ctx.newPage();
  page.on("pageerror", (e) => say("pageerror: " + e.message));
  // Every <img> the page creates, tagged with whether it is the palette's
  // loader (the only one setting referrerPolicy=no-referrer). This is how the
  // ground's OWN image load is counted apart from the player bar's Cover and
  // the play cache, which legitimately ask for the same sleeve.
  await page.addInitScript(() => {
    const Native = window.Image;
    window.__img = [];
    window.Image = class extends Native {
      set src(value) {
        try {
          window.__img.push({ v: String(value), ours: this.referrerPolicy === "no-referrer" });
        } catch {
          /* ignore */
        }
        super.src = value;
      }
      get src() {
        return super.src;
      }
    };
  });
  await page.goto(base + "/app/");
  await login(page, base);
  await page.goto(base + "/app/");
  await page.waitForTimeout(500);
  await page.goto(base + "/app/#/playlist/" + ids.big);
  await page.waitForSelector(".row.track", { timeout: 60000 });
  await page.waitForTimeout(500);
  const requests = [];
  page.on("request", (r) => requests.push({ at: Date.now(), url: r.url() }));
  await page.locator(".row.track .titles").first().click();
  await page.waitForTimeout(1500);
  return { page, requests };
}

const probeGround = (page) =>
  page.evaluate(() => {
    const vis = (el) => !!(el && el.checkVisibility && el.checkVisibility());
    const all = [...document.querySelectorAll("*")];
    const blurred = all.filter((el) => {
      if (!vis(el)) return false;
      const s = getComputedStyle(el);
      return (s.filter || "").includes("blur(") && !el.closest(".d, .m");
    });
    const backdrop = all.filter((el) => {
      if (!vis(el)) return false;
      const s = getComputedStyle(el);
      return (s.backdropFilter || s.webkitBackdropFilter || "").includes("blur(");
    });
    const gb = document.querySelector(".gb");
    const art = document.querySelector(".gb-art");
    const img = document.querySelector(".gb-img");
    const scrim = document.querySelector(".gb-scrim");
    const bar = document.querySelector("footer.player");
    const barBox = bar ? bar.getBoundingClientRect() : null;
    const at = (x, y) => {
      const el = document.elementFromPoint(x, y);
      return el ? { inGb: !!el.closest(".gb"), cls: el.className?.toString?.() || el.tagName } : null;
    };
    return {
      gb: !!gb && vis(gb),
      images: document.querySelectorAll(".gb-img").length,
      arts: document.querySelectorAll(".gb-art").length,
      scrims: document.querySelectorAll(".gb-scrim").length,
      blurred: blurred.length,
      blurredTags: blurred.map((e) => e.className?.toString?.() || e.tagName),
      backdrop: backdrop.length,
      backdropTags: backdrop.slice(0, 5).map((e) => e.className?.toString?.() || e.tagName),
      bgImage: img ? getComputedStyle(img).backgroundImage.slice(0, 120) : null,
      artOpacity: art ? +getComputedStyle(art).opacity : null,
      filter: img ? getComputedStyle(img).filter : null,
      z: img ? getComputedStyle(img).zIndex : null,
      eco: document.documentElement.dataset.eco ?? null,
      gbBox: gb
        ? { isolation: getComputedStyle(gb).isolation, position: getComputedStyle(gb).position }
        : null,
      bodyBg: getComputedStyle(document.body).backgroundColor,
      centre: at(Math.round(innerWidth / 2), Math.round(innerHeight / 2)),
      bottom: at(Math.round(innerWidth / 2), Math.round(innerHeight - 12)),
      barTop: barBox ? Math.round(barBox.top) : null,
    };
  });

const SCREENS = [
  ["accueil", "/", 900],
  ["bibliotheque", "/library", 1200],
  ["rechercher", "/search", 700],
  ["reglages", "/settings/playback", 900],
  ["album", "/album/" + ids.album, 900],
  ["playlist", "/playlist/" + ids.small, 900],
  ["artiste", "/artist/" + ids.artist, 900],
];

for (const [label, size, phone] of [
  ["bureau 1280", { width: 1280, height: 900 }, false],
  ["téléphone 390", { width: 390, height: 844 }, true],
]) {
  const ctx = await context({ ...size, phone });
  const { page, requests } = await open(ctx);
  say(`\n— ${label} : « ${first.title} » (${pathOf(first.album.cover)})`);

  let firstBg = null;
  for (const [name, hash, wait] of SCREENS) {
    const before = requests.length;
    await page.goto(base + "/app/#" + hash);
    await page.waitForTimeout(wait);
    const g = await probeGround(page);
    const mine = requests.slice(before).filter((r) => pathOf(r.url).startsWith("/api/") || /dzcdn/.test(r.url));
    const covers = mine.filter((r) => pathOf(r.url) === ALBUM_COVER || pathOf(r.url) === "/api/cover/" + ref(first));
    const shot = name === "bibliotheque" ? await page.screenshot() : null;
    await snap(page, `${phone ? "390" : "1280"}-${name}`);
    if (name === "bibliotheque") {
      // Text must sit ABOVE the ground: the page title's glyphs have to reach
      // the theme's ink, not the 86 % veil of a layer painted over them.
      const ink = await page.evaluate(async ({ b64 }) => {
        const el = document.querySelector(".page-title") || document.querySelector("main h1");
        if (!el) return null;
        const box = el.getBoundingClientRect();
        if (box.width < 20 || box.height < 10) return null;
        const cv = document.createElement("canvas");
        const toRgb = (v) => {
          cv.width = cv.height = 1;
          const x = cv.getContext("2d", { willReadFrequently: true });
          x.fillStyle = "#000000";
          x.fillStyle = v.trim();
          x.clearRect(0, 0, 1, 1);
          x.fillRect(0, 0, 1, 1);
          const d = x.getImageData(0, 0, 1, 1).data;
          return [d[0], d[1], d[2]];
        };
        const cs = getComputedStyle(document.documentElement);
        const bin = atob(b64);
        const bytes = new Uint8Array(bin.length);
        for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
        const bmp = await createImageBitmap(new Blob([bytes], { type: "image/png" }));
        const c = document.createElement("canvas");
        c.width = bmp.width;
        c.height = bmp.height;
        const x = c.getContext("2d", { willReadFrequently: true });
        x.drawImage(bmp, 0, 0);
        const dpr = bmp.width / innerWidth;
        const data = x.getImageData(
          Math.max(0, Math.round(box.x * dpr)),
          Math.max(0, Math.round(box.y * dpr)),
          Math.min(bmp.width - Math.round(box.x * dpr), Math.round(box.width * dpr)),
          Math.min(bmp.height - Math.round(box.y * dpr), Math.round(box.height * dpr))
        ).data;
        const bg = toRgb(cs.getPropertyValue("--bg"));
        const tx = toRgb(cs.getPropertyValue("--tx"));
        let best = null;
        let far = -1;
        for (let i = 0; i < data.length; i += 4) {
          const p = [data[i], data[i + 1], data[i + 2]];
          const d = Math.hypot(p[0] - bg[0], p[1] - bg[1], p[2] - bg[2]);
          if (d > far) {
            far = d;
            best = p;
          }
        }
        return { pixel: best, ink: tx, dist: best ? Math.hypot(best[0] - tx[0], best[1] - tx[1], best[2] - tx[2]) : null };
      }, { b64: shot.toString("base64") });
      check(!!ink && ink.dist < 70, `${label} · ${name} : le titre est peint AU-DESSUS du fond`, JSON.stringify(ink));
      say(`   titre : pixel ${ink?.pixel} pour une encre ${ink?.ink}`);
    }
    check(g.gb, `${label} · ${name} : le fond est monté`, JSON.stringify({ images: g.images, scrims: g.scrims }));
    check(g.images === 1, `${label} · ${name} : UNE image de fond`, `${g.images} image(s), ${g.arts} groupe(s)`);
    check(g.blurred <= 1, `${label} · ${name} : un seul élément flouté`, g.blurredTags.join(", "));
    check(g.backdrop === 0, `${label} · ${name} : aucun backdrop-filter`, g.backdropTags.join(", "));
    check(g.centre && !g.centre.inGb, `${label} · ${name} : le fond est DERRIÈRE le contenu`, JSON.stringify(g.centre));
    check(
      g.gbBox?.isolation === "auto" && g.gbBox?.position === "static",
      `${label} · ${name} : le fond ne crée pas de contexte d'empilement`,
      JSON.stringify(g.gbBox)
    );
    check(
      /rgba\(0, 0, 0, 0\)|transparent/.test(g.bodyBg),
      `${label} · ${name} : <body> laisse voir le fond`,
      g.bodyBg
    );
    check(g.bottom && !g.bottom.inGb, `${label} · ${name} : la barre du lecteur est au-dessus`, JSON.stringify(g.bottom));
    // The ground itself must SURVIVE a navigation: its painted URL is what the
    // component would have to fetch again if it remounted or re-decided.
    check(
      firstBg === null || g.bgImage === firstBg,
      `${label} · ${name} : le fond survit à la navigation (même image)`,
      `${g.bgImage?.slice(0, 60)}`
    );
    if (firstBg === null) firstBg = g.bgImage;
    say(
      `   ${name.padEnd(14)} images ${g.images} · flou ${g.blurred} · backdrop ${g.backdrop} · z ${g.z} · ` +
        `requêtes ${mine.length} (pochette ${covers.length}) · opacité ${g.artOpacity}`
    );
  }

  // ---- a skip: the fade, the requests, the jank ---------------------------
  const beforeSkip = requests.length;
  const imgMark = await page.evaluate(() => (window.__img || []).length);
  const run = await page.evaluate(async () => {
    const samples = [];
    let gaps = [];
    const btn = [...document.querySelectorAll("button")].find((b) => b.getAttribute("aria-label") === "Suivant");
    if (!btn) return { samples, gaps, error: "no next button" };
    const t0 = performance.now();
    let last = t0;
    btn.click();
    await new Promise((done) => {
      const loop = (now) => {
        gaps.push(now - last);
        last = now;
        const art = document.querySelector(".gb-art");
        const img = document.querySelector(".gb-img");
        samples.push({
          t: +(now - t0).toFixed(0),
          op: art ? +getComputedStyle(art).opacity : null,
          bg: img ? img.style.backgroundImage.slice(0, 80) : null,
          imgs: document.querySelectorAll(".gb-img").length,
        });
        if (now - t0 < 1100) requestAnimationFrame(loop);
        else done();
      };
      requestAnimationFrame(loop);
    });
    return { samples, gaps: gaps.slice(2), long: window.__long.filter((e) => e.at >= t0 && e.at <= t0 + 1500).reduce((a, e) => a + e.d, 0) };
  });
  const samples = run.samples || [];
  const start = samples.find((s) => s.op !== null && s.op < 0.99);
  const swapAt = samples.findIndex((s, i) => i > 0 && s.bg !== samples[i - 1].bg);
  const changed = swapAt >= 0 ? samples[swapAt] : null;
  const settled = samples.slice(swapAt + 1).find((s) => s.op !== null && s.op >= 0.99);
  const maxGap = run.gaps?.length ? Math.max(...run.gaps) : 0;
  const secondPaths = new Set([pathOf(second.album.cover), "/api/cover/" + ref(second)]);
  const isSecond = (u) => secondPaths.has(pathOf(u));
  const skipReqs = requests.slice(beforeSkip).filter((r) => isSecond(r.url));
  const imgSets = await page.evaluate((m) => (window.__img || []).slice(m), imgMark);
  const ourSets = imgSets.filter((x) => x.ours && isSecond(x.v));
  // A poll would keep asking after the sleeve is painted: count the covers
  // requested over the second and a half that follows the fade.
  const settledAt = requests.length;
  await page.waitForTimeout(1500);
  const polled = requests.slice(settledAt).filter((r) => isSecond(r.url));
  await snap(page, `${phone ? "390" : "1280"}-apres-saut`);
  check(!!start && start.t < 160, `${label} : le fondu démarre à ${start?.t ?? "?"} ms`, `${samples.length} cadres`);
  check(!!changed && changed.t > 150 && changed.t < 400, `${label} : la nouvelle pochette arrive pendant le creux (${changed?.t ?? "?"} ms)`, "");
  check(settled && settled.t < 950, `${label} : le fond est revenu plein à ${settled?.t} ms`, "");
  check(maxGap < 250, `${label} : aucun gel pendant le fondu (${maxGap.toFixed(0)} ms max)`, `longtasks ${(run.long || 0).toFixed(0)} ms`);
  check(
    samples.every((x) => (x.imgs ?? 0) <= 1),
    `${label} : UNE seule image floutée pendant le fondu`,
    `${Math.max(...samples.map((x) => x.imgs ?? 0))} au plus`
  );
  check(
    ourSets.length === 1,
    `${label} : le fond charge la nouvelle pochette UNE fois`,
    `${ourSets.length} chargement(s) du fond, ${skipReqs.length} au total (barre, Cover, cache de lecture)`
  );
  check(polled.length === 0, `${label} : aucune requête de pochette après le fondu`, `${polled.length} en 1,5 s`);
  check(new Set(skipReqs.map((r) => pathOf(r.url))).size <= 2, `${label} : au plus deux URL pour une pochette`, [...new Set(skipReqs.map((r) => pathOf(r.url)))].join(" ; "));
  say(
    `   saut : fondu ${start?.t} ms, nouvelle art ${changed?.t} ms, retour ${settled?.t} ms, pas max ${maxGap.toFixed(0)} ms, ` +
      `longtask ${(run.long || 0).toFixed(0)} ms, fond ${ourSets.length} chargement(s), requêtes ${skipReqs.length}, après fondu ${polled.length}`
  );

  // ---- full screen: ONE ground at a time ----------------------------------
  await page.goto(base + "/app/#/");
  await page.waitForTimeout(400);
  if (phone) {
    await page.locator("footer.player").click({ position: { x: 4, y: 4 } });
    await page.waitForSelector(".m", { timeout: 20000 });
  } else {
    await page.locator('button[aria-label="Plein écran"]').first().click();
    await page.waitForSelector(".d", { timeout: 20000 });
  }
  await page.waitForTimeout(800);
  const g = await probeGround(page);
  check(!g.gb, `${label} : la couche de fond quitte l'écran quand le plein écran est ouvert`, `gb ${g.gb}`);
  await snap(page, `${phone ? "390" : "1280"}-plein-ecran`);
  await page.locator('.d button[aria-label="Réduire"], .m button[aria-label="Réduire"]').first().click();
  await page.waitForTimeout(600);
  const back = await probeGround(page);
  check(back.gb && back.images === 1, `${label} : le fond revient en fermant le plein écran`, `${back.images} image(s)`);

  // Boot cost: a reload asks for the whole first screen, palette decode and
  // blur raster included. Measured as long-task ms over the 2.5 s after load.
  await page.reload();
  await page.waitForTimeout(2600);
  const boot = await page.evaluate(() => ({
    long: window.__long.reduce((a, e) => a + e.d, 0),
    worst: window.__long.reduce((a, e) => Math.max(a, e.d), 0),
  }));
  say(`   démarrage fond : longtasks ${boot.long.toFixed(0)} ms (pire ${boot.worst.toFixed(0)} ms)`);
  check(boot.worst < 400, `${label} : le premier écran ne gèle pas (pire tâche ${boot.worst.toFixed(0)} ms)`, "");
  // The same boot on a mid-range phone: 4× CPU throttle, the bench's other
  // standard. This is where a full-viewport blur would show up if it cost.
  const cdp4 = await ctx.newCDPSession(page);
  await cdp4.send("Emulation.setCPUThrottlingRate", { rate: 4 });
  await page.reload();
  await page.waitForTimeout(3200);
  const boot4 = await page.evaluate(() => ({
    long: window.__long.reduce((a, e) => a + e.d, 0),
    worst: window.__long.reduce((a, e) => Math.max(a, e.d), 0),
  }));
  say(`   démarrage fond ×4 : longtasks ${boot4.long.toFixed(0)} ms (pire ${boot4.worst.toFixed(0)} ms)`);

  await ctx.close();
}

// ---- eco ----------------------------------------------------------------
for (const [label, size, phone] of [
  ["bureau 1280", { width: 1280, height: 900 }, false],
  ["téléphone 390", { width: 390, height: 844 }, true],
]) {
  const ctx = await context({ ...size, phone });
  const { page, requests } = await open(ctx);
  await page.evaluate(() => localStorage.setItem("viz.eco", JSON.stringify(true)));
  await page.reload();
  await page.waitForTimeout(1600);
  await page.goto(base + "/app/#/library");
  await page.waitForTimeout(1200);
  const g = await probeGround(page);
  const before = requests.length;
  await page.waitForTimeout(3000);
  const afterEco = requests.slice(before).filter((r) => pathOf(r.url) === ALBUM_COVER || pathOf(r.url) === "/api/cover/" + ref(first));
  await snap(page, `${phone ? "390" : "1280"}-eco`);
  check(g.eco === "1", `${label} · éco : data-eco="1" sur <html>`, `"${g.eco}"`);
  check(g.images === 0 && g.arts === 0 && g.scrims === 0, `${label} · éco : aucune image, aucun scrim`, JSON.stringify({ images: g.images, arts: g.arts, scrims: g.scrims }));
  check(g.blurred === 0, `${label} · éco : aucun filter:blur`, g.blurredTags.join(", "));
  check(g.backdrop === 0, `${label} · éco : aucun backdrop-filter`, g.backdropTags.join(", "));
  check(afterEco.length === 0, `${label} · éco : aucune requête de pochette pendant 3 s`, `${afterEco.length}`);

  // The same skip WITHOUT the ground, for the cost comparison: no layer, no
  // palette, no blur — the number the with-ground run is measured against.
  const ecoSkip = await page.evaluate(async () => {
    const gaps = [];
    const btn = [...document.querySelectorAll("button")].find((b) => b.getAttribute("aria-label") === "Suivant");
    const t0 = performance.now();
    let last = t0;
    let imgs = 0;
    btn.click();
    await new Promise((done) => {
      const loop = (now) => {
        gaps.push(now - last);
        last = now;
        imgs = Math.max(imgs, document.querySelectorAll(".gb-img").length);
        if (now - t0 < 1100) requestAnimationFrame(loop);
        else done();
      };
      requestAnimationFrame(loop);
    });
    return { gaps: gaps.slice(2), long: window.__long.filter((e) => e.at >= t0 && e.at <= t0 + 1500).reduce((a, e) => a + e.d, 0), imgs };
  });
  const ecoGap = ecoSkip.gaps.length ? Math.max(...ecoSkip.gaps) : 0;
  say(`   éco saut : pas max ${ecoGap.toFixed(0)} ms, longtasks ${ecoSkip.long.toFixed(0)} ms, images ${ecoSkip.imgs}`);
  check(ecoSkip.imgs === 0, `${label} · éco : aucun saut ne monte d'image`, `${ecoSkip.imgs}`);

  // Leaving eco: the ground comes back, the attribute goes away.
  await page.evaluate(() => localStorage.setItem("viz.eco", JSON.stringify(false)));
  const beforeOff = requests.length;
  await page.reload();
  await page.waitForTimeout(1800);
  const back = await probeGround(page);
  const cdp = await ctx.newCDPSession(page);
  const measure = async () => {
    await cdp.send("Performance.enable").catch(() => {});
    const m = await cdp.send("Performance.getMetrics").catch(() => null);
    const dom = await cdp.send("Memory.getDOMCounters").catch(() => null);
    const heap = m?.metrics?.find((x) => x.name === "JSHeapUsedSize")?.value ?? null;
    return { heap: heap === null ? null : Math.round(heap / 1024), nodes: dom?.nodes ?? null };
  };
  const ecoMem = await measure();
  await page.reload();
  await page.waitForTimeout(2600);
  const ecoBoot = await page.evaluate(() => ({
    long: window.__long.reduce((a, e) => a + e.d, 0),
    worst: window.__long.reduce((a, e) => Math.max(a, e.d), 0),
  }));
  say(`   démarrage éco : longtasks ${ecoBoot.long.toFixed(0)} ms (pire ${ecoBoot.worst.toFixed(0)} ms)`);
  const cdpEco = await ctx.newCDPSession(page);
  await cdpEco.send("Emulation.setCPUThrottlingRate", { rate: 4 });
  await page.reload();
  await page.waitForTimeout(3200);
  const ecoBoot4 = await page.evaluate(() => ({
    long: window.__long.reduce((a, e) => a + e.d, 0),
    worst: window.__long.reduce((a, e) => Math.max(a, e.d), 0),
  }));
  say(`   démarrage éco ×4 : longtasks ${ecoBoot4.long.toFixed(0)} ms (pire ${ecoBoot4.worst.toFixed(0)} ms)`);
  const offReq = requests.slice(beforeOff).filter((r) => pathOf(r.url) === ALBUM_COVER || pathOf(r.url) === "/api/cover/" + ref(first));
  const liveMem = await measure();
  check(back.eco === null && back.images === 1, `${label} · sortie d'éco : l'image revient, data-eco disparaît`, JSON.stringify({ eco: back.eco, images: back.images }));
  say(
    `   mémoire : éco ${ecoMem.heap} Ko / ${ecoMem.nodes} nœuds → fond ${liveMem.heap} Ko / ${liveMem.nodes} nœuds ` +
      `(+${liveMem.nodes - ecoMem.nodes} nœuds)`
  );
  check(offReq.length <= 1, `${label} · sortie d'éco : une seule requête de pochette`, `${offReq.length}`);
  await snap(page, `${phone ? "390" : "1280"}-hors-eco`);
  await ctx.close();
}



// ---- contrast over a pure-white sleeve ----------------------------------
for (const [name, size, phone] of [
  ["bureau 1280", { width: 1280, height: 900 }, false],
  ["téléphone 390", { width: 390, height: 844 }, true],
]) {
  for (const theme of ["encre", "papier"]) {
    const ctx = await context({ ...size, phone, cover: "white" });
    await ctx.addInitScript((t) => localStorage.setItem("ui.theme", JSON.stringify(t)), theme);
    const page = await ctx.newPage();
    page.on("pageerror", (e) => say(`contraste ${name}/${theme} pageerror: ` + e.message));
    page.on("console", (m) => m.type() === "error" && say(`contraste ${name}/${theme} console: ` + m.text().slice(0, 160)));
    await page.goto(base + "/app/");
    await login(page, base);
    // A reload after the login: a goto that only changes the fragment is a
    // same-document navigation, and the SPA stays on the login screen.
    await page.goto(base + "/app/");
    await page.goto(base + "/app/#/playlist/" + ids.big);
    try {
      await page.waitForSelector(".row.track", { timeout: 90000 });
    } catch (e) {
      await snap(page, `contraste-echec-${name.replace(/\W/g, "")}-${theme}`);
      say(`contraste ${name}/${theme} : la playlist n'a pas monté — ` + (await page.evaluate(() => document.body.innerText.slice(0, 300))));
      throw e;
    }
    await page.locator(".row.track .titles").first().click();
    await page.waitForTimeout(1600);
    await page.goto(base + "/app/#/library");
    await page.waitForTimeout(1400);
    const tokens = await page.evaluate(() => {
      const cs = getComputedStyle(document.documentElement);
      const toRgb = (value) => {
        const c = document.createElement("canvas");
        c.width = c.height = 1;
        const x = c.getContext("2d", { willReadFrequently: true });
        x.fillStyle = "#000000";
        x.fillStyle = value.trim();
        x.clearRect(0, 0, 1, 1);
        x.fillRect(0, 0, 1, 1);
        const d = x.getImageData(0, 0, 1, 1).data;
        return [d[0], d[1], d[2]];
      };
      return { tx: toRgb(cs.getPropertyValue("--tx")), dim: toRgb(cs.getPropertyValue("--dim")), bg: toRgb(cs.getPropertyValue("--bg")) };
    });
    await snap(page, `${phone ? "390" : "1280"}-${theme}-blanc-ui`);
    // The UI is hidden for the probe: what is measured is the GROUND itself,
    // the surface page titles and muted lines sit on — not a lemon chip that
    // happens to be under a sample point.
    await page.evaluate(() => {
      for (const el of document.querySelectorAll("#app > *")) {
        if (!el.classList || !el.classList.contains("gb")) el.style.visibility = "hidden";
      }
    });
    await page.waitForTimeout(150);
    const shot = await page.screenshot();
    const pixels = await page.evaluate(
      async ({ b64 }) => {
        // atob, not fetch(data:): the app's CSP refuses data: URLs, and a
        // createImageBitmap decodes bytes the policy does not govern.
        const bin = atob(b64);
        const bytes = new Uint8Array(bin.length);
        for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
        const bmp = await createImageBitmap(new Blob([bytes], { type: "image/png" }));
        const c = document.createElement("canvas");
        c.width = bmp.width;
        c.height = bmp.height;
        const x = c.getContext("2d", { willReadFrequently: true });
        x.drawImage(bmp, 0, 0);
        const col = x.getImageData(0, 0, bmp.width, bmp.height).data;
        const out = [];
        // A 24 × 14 grid over the whole viewport: the worst ground pixel, not
        // the one that happens to sit under the middle of the page.
        for (let gy = 0; gy < 14; gy++)
          for (let gx = 0; gx < 24; gx++) {
            const px = Math.min(bmp.width - 1, Math.round(((gx + 0.5) / 24) * bmp.width));
            const py = Math.min(bmp.height - 1, Math.round(((gy + 0.5) / 14) * bmp.height));
            const i = (py * bmp.width + px) * 4;
            out.push([col[i], col[i + 1], col[i + 2]]);
          }
        return out;
      },
      { b64: shot.toString("base64") }
    );
    const lin = (v) => {
      const c = v / 255;
      return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
    };
    const lum = ([r, g, b]) => 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
    const ratio = (a, b) => {
      const [l1, l2] = [lum(a), lum(b)].sort((x, y) => y - x);
      return (l1 + 0.05) / (l2 + 0.05);
    };
    const tx = tokens.tx;
    const dim = tokens.dim;
    const worstTx = Math.min(...pixels.map((p) => ratio(tx, p)));
    const worstDim = Math.min(...pixels.map((p) => ratio(dim, p)));
    const lightest = pixels.reduce((a, p) => (lum(p) > lum(a) ? p : a), pixels[0]);
    const darkest = pixels.reduce((a, p) => (lum(p) < lum(a) ? p : a), pixels[0]);
    check(worstTx >= 4.5, `${name} · ${theme} : --tx tient 4.5:1 sur une pochette blanche`, `${worstTx.toFixed(2)}:1`);
    check(worstDim >= 4.5, `${name} · ${theme} : --dim tient 4.5:1 sur une pochette blanche`, `${worstDim.toFixed(2)}:1`);
    say(
      `   contraste ${name} · ${theme} : --tx ${worstTx.toFixed(2)}:1, --dim ${worstDim.toFixed(2)}:1, ` +
        `sol ${lightest} → ${darkest} (${pixels.length} points)`
    );
    await snap(page, `${phone ? "390" : "1280"}-${theme}-blanc`);
    await ctx.close();
  }
}

// ---- reduced motion ------------------------------------------------------
{
  const ctx = await browser.newContext({
    viewport: { width: 390, height: 844 },
    deviceScaleFactor: 2,
    isMobile: true,
    hasTouch: true,
    reducedMotion: "reduce",
  });
  await ctx.route(deezerApi, (r) => r.abort());
  await ctx.route((u) => dzcdn(u) || u.pathname.startsWith("/api/cover/"), (r) => {
    const path = new URL(r.request().url()).pathname;
    r.fulfill({ status: 200, contentType: "image/png", body: COVER.get(path) || PNG.grey });
  });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => say("reduced-motion pageerror: " + e.message));
  await page.goto(base + "/app/");
  await login(page, base);
  await page.goto(base + "/app/");
  await page.goto(base + "/app/#/playlist/" + ids.big);
  await page.waitForSelector(".row.track", { timeout: 90000 });
  await page.locator(".row.track .titles").first().click();
  await page.waitForTimeout(1500);
  const run = await page.evaluate(async () => {
    const samples = [];
    const btn = [...document.querySelectorAll("button")].find((b) => b.getAttribute("aria-label") === "Suivant");
    const t0 = performance.now();
    btn.click();
    await new Promise((done) => {
      const loop = (now) => {
        const art = document.querySelector(".gb-art");
        const img = document.querySelector(".gb-img");
        samples.push({ t: +(now - t0).toFixed(0), op: art ? +getComputedStyle(art).opacity : null, bg: img?.style.backgroundImage.slice(0, 80) });
        if (now - t0 < 700) requestAnimationFrame(loop);
        else done();
      };
      requestAnimationFrame(loop);
    });
    return samples;
  });
  const changed = run.find((s) => s.bg && !s.bg.includes(pathOf(first.album.cover)));
  const minOpacity = Math.min(...run.map((s) => (s.op === null ? 1 : s.op)));
  check(changed && changed.t < 300, `reduced-motion : la pochette change vite (${changed?.t ?? "?"} ms)`, "");
  check(minOpacity >= 0.99, `reduced-motion : aucun fondu d'opacité (min ${minOpacity})`, "");
  await ctx.close();
}

// --- a title with NO art takes the ground away ----------------------------
// From a phone's log (2026-11-27): on a track change to a title that has no
// sleeve, `GlobalBackdrop`'s clearArt() called `reduceMotion()` — a name that
// file never declares (`reducedMotion` is the helper) — so every such change
// threw a ReferenceError out of an async subscriber: an unhandled rejection on
// the console, and the PREVIOUS sleeve left painted, claiming a ground for a
// track that has none. Pinned here because nothing else can see it: Svelte and
// Rollup both build a free variable silently (no warning in `npm run build`).
{
  const ctx = await context({ width: 390, height: 844, phone: true });
  await ctx.addInitScript(installSessionPrefix);
  await ctx.addInitScript(() => {
    try {
      const j = sessionStorage.getItem("__inject");
      if (j) {
        localStorage.setItem(window.__nsPkey("player.session"), j);
        localStorage.removeItem(window.__nsPkey("player.pos"));
        sessionStorage.removeItem("__inject");
      }
    } catch {
      /* no storage */
    }
  });
  const ART = "https://e-cdns-images.dzcdn.net/images/cover/aaaa/500x500-000000-80-0-0.jpg";
  const artUrl = new URL(ART).pathname;
  const track = (id, cover) => ({
    deezer_id: id,
    title: "Titre " + id,
    duration: 60,
    artist: { deezer_id: "a1", name: "Band" },
    album: { deezer_id: "al1", title: "Album", cover },
  });
  const session = {
    queue: [track("avec", ART), track("sans", "")],
    index: 0,
    start: 0,
    currentTime: 0,
    context: null,
    shuffle: false,
    repeat: "off",
    _orig: null,
    at: Date.now(),
  };
  const page = await ctx.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await login(page, base);
  await page.evaluate((j) => sessionStorage.setItem("__inject", j), JSON.stringify(session));
  await page.reload();
  await page.waitForFunction(() => document.querySelector("footer.player .pp"), null, { timeout: 30000 });
  await page.locator("footer.player .pp").click();
  // The ground paints after an idle moment on the very first track.
  const painted = await page
    .waitForFunction(
      (u) => [...document.querySelectorAll(".gb-img")].some((e) => e.style.backgroundImage.includes(u.slice(0, 40))),
      artUrl,
      { timeout: 8000 }
    )
    .then(() => true)
    .catch(() => false);
  check(painted, "fond : la pochette du premier titre est peinte", "");
  await page.locator("footer.player .next").click();
  await page.waitForFunction(
    () => /Titre sans/.test(document.querySelector("footer.player .info .t")?.textContent || ""),
    null,
    { timeout: 15000 }
  );
  await page.waitForTimeout(900); // the 220 ms fade-out plus a margin
  const g = await page.evaluate(() => ({
    art: document.querySelectorAll(".gb-art").length,
    img: document.querySelectorAll(".gb-img").length,
    scrim: document.querySelectorAll(".gb-scrim").length,
  }));
  check(errors.length === 0, "fond : aucun pageerror sur un titre sans pochette", errors.join(" | "));
  check(g.art === 0 && g.img === 0, `fond : l'ancienne pochette est retirée (${g.art} art, ${g.img} image)`, "");
  check(g.scrim === 1, "fond : le scrim, lui, reste", `${g.scrim}`);
  await ctx.close();
}

await browser.close();
stop();
console.log(`\n${state.failed ? state.failed + " FAIL" : "tout est vert"}`);
process.exit(state.failed ? 1 : 0);
