#!/usr/bin/env node
// Volume normalization, on the audio graph the browser really builds.
//
//   npm run build && node test/gain/run.mjs
//   DIST=/tmp/build node test/gain/run.mjs      # serve a build made elsewhere
//
// `lib/audio/graph.js` turns a track's ReplayGain (dB, `Track.gain` on the row,
// measured from the archived file) into a FIXED linear gain for the whole
// track: -(gain + 18.4) dB, plus the level's offset (low +2, medium +5,
// high +8), clamped to [-24, +12] dB. Two consequences this bench holds on the
// real player:
//
//  - the gain is applied at the SOURCE HANDOVER, so a track starts at its own
//    level with no audible "unity then correct" jump — and the FIRST track of a
//    session gets it too, not only the ones after a skip;
//  - it lands on the track being loaded, never the outgoing one: a skip from a
//    loud track to a quiet one moves the gain to the INCOMING value in one
//    step, with nothing of the previous track's level carried over.
//
// A track whose gain is unknown (null: never measured) plays at unity — the
// player never invents a level. That is also why "la normalisation ne marche
// plus" can be a DATA answer: a null `Track.gain` normalizes nothing, whatever
// the client does.
//
// Measured (medium level, gains -10 and -20 dB): 0.676 at the first play,
// 2.138 after the skip, exactly 10^(-3.4/20) and 10^(6.6/20); with
// normalization off, every gain node stays at 1.
import { loadPlaywright, startServer, silence, login, checker, installSessionPrefix, deezerHost, pathOn } from "../harness.mjs";

const { chromium } = await loadPlaywright();
const { base, stop } = await startServer();
const { check, state } = checker();
const say = (s) => console.log(s);
const wav = silence(60);

const browser = await chromium.launch({ args: ["--autoplay-policy=no-user-gesture-required"] });

/** 10^(db/20), the linear gain the graph must hold. */
const lin = (db) => Math.pow(10, db / 20);
// medium (+5 dB):  -(gain + 18.4) + 5
const MEDIUM = (gainDb) => lin(-(gainDb + 18.4) + 5);

const track = (id, gain) => ({
  deezer_id: id,
  title: "Song " + id,
  duration: 60,
  gain,
  artist: { deezer_id: "a1", name: "Band" },
  album: { deezer_id: "al1", title: "Album", cover: "" },
});
const session = (...queue) => ({
  queue,
  index: 0,
  start: 0,
  currentTime: 0,
  context: null,
  shuffle: false,
  repeat: "off",
  _orig: null,
  at: Date.now(),
});

async function play(level, queue) {
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  await ctx.route(deezerHost, (r) => r.abort());
  await ctx.route(pathOn("/api/stream/"), (r) =>
    r.fulfill({ status: 200, body: wav, headers: { "Content-Type": "audio/wav", "Accept-Ranges": "none" } })
  );
  // Every GainNode the graph creates, so the normalization can be read off the
  // audio path itself (the element's own volume is only the player's level).
  await ctx.addInitScript(() => {
    window.__gains = [];
    for (const name of ["AudioContext", "webkitAudioContext"]) {
      const C = window[name];
      if (!C) continue;
      const orig = C.prototype.createGain;
      C.prototype.createGain = function (...a) {
        const n = orig.apply(this, a);
        window.__gains.push(n);
        return n;
      };
    }
  });
  await ctx.addInitScript(() => {
    const Native = window.Audio;
    window.__els = [];
    window.Audio = function (...a) {
      const el = new Native(...a);
      window.__els.push(el);
      return el;
    };
    window.Audio.prototype = Native.prototype;
  });
  await ctx.addInitScript(installSessionPrefix);
  await ctx.addInitScript((level) => {
    try {
      const j = sessionStorage.getItem("__inject");
      if (j) {
        localStorage.setItem(window.__nsPkey("player.session"), j);
        localStorage.removeItem(window.__nsPkey("player.pos"));
        sessionStorage.removeItem("__inject");
      }
      // `fx.normalize` is a DEVICE setting: the plain key, not an account one.
      localStorage.setItem("fx.normalize", JSON.stringify(level));
    } catch {
      /* no storage */
    }
  }, level);

  const page = await ctx.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await login(page, base);
  await page.evaluate((j) => sessionStorage.setItem("__inject", j), JSON.stringify(session(...queue)));
  await page.reload();
  await page.waitForFunction(() => document.querySelector("footer.player .pp"), null, { timeout: 30000 });
  await page.waitForTimeout(600);

  const read = () =>
    page.evaluate(() => ({
      title: document.querySelector("footer.player .info .t")?.textContent,
      wired: !!window.__nsAudioPath?.().wired,
      direct: !!window.__nsAudioPath?.().direct,
      norm: window.__gains.map((n) => +n.gain.value.toFixed(4)).filter((v) => Math.abs(v - 1) > 1e-3),
      vols: window.__els.map((e) => e.volume),
    }));

  await page.locator("footer.player .pp").click();
  await page.waitForFunction(() => window.__els.some((e) => e.src && !e.paused), null, { timeout: 15000 });
  await page.waitForTimeout(900);
  const first = await read();

  await page.locator("footer.player .next").click();
  await page.waitForFunction(
    () => document.querySelector("footer.player .info .t")?.textContent === "Song s2",
    null,
    { timeout: 15000 }
  );
  await page.waitForTimeout(900);
  const second = await read();
  await ctx.close();
  return { first, second, errors };
}

// -- two loudness levels, on the shipped gain ---------------------------------
{
  const { first, second, errors } = await play("medium", [track("s1", -10), track("s2", -20)]);
  say(`\nnormalisation « moyenne » : ${JSON.stringify([first.norm, second.norm])}`);
  // The processed path is where the gain node lives; a direct element would
  // carry it as its volume instead, and that is pinned by nativegain.test.mjs.
  check(first.wired && !first.direct, "le son passe par le graphe (sinon rien à lire)", JSON.stringify({ w: first.wired, d: first.direct }));
  check(first.title === "Song s1", "le premier titre joue", first.title);
  check(
    first.norm.length === 1 && Math.abs(first.norm[0] - MEDIUM(-10)) < 0.002,
    `le PREMIER titre porte son gain (-10 dB → ${MEDIUM(-10).toFixed(3)})`,
    JSON.stringify(first.norm)
  );
  check(
    second.norm.length === 1 && Math.abs(second.norm[0] - MEDIUM(-20)) < 0.002,
    `après un saut, le gain est celui du nouveau titre (-20 dB → ${MEDIUM(-20).toFixed(3)})`,
    JSON.stringify(second.norm)
  );
  // 6.6 dB apart, and nothing of the outgoing value left: one node, one value.
  check(
    first.norm.length === 1 && second.norm.length === 1
      ? Math.abs(20 * Math.log10(second.norm[0] / first.norm[0]) - 10) < 0.05
      : false,
    "deux titres de 10 dB d'écart sortent 10 dB d'écart (le nivellement est le gain)",
    `${first.norm[0]} → ${second.norm[0]}`
  );
  check(errors.length === 0, "aucune erreur de page", errors.join(" | "));
}

// -- off, and a gain we do not know: unity ------------------------------------
{
  const { first } = await play("off", [track("s1", -10), track("s2", -20)]);
  check(first.norm.length === 0, "normalisation désactivée : aucun gain n'est appliqué", JSON.stringify(first.norm));
}
{
  const { first, second } = await play("medium", [track("s1", null), track("s2", null)]);
  check(
    first.norm.length === 0 && second.norm.length === 0,
    "un gain inconnu (null) ne s'invente pas : le titre joue à l'unité",
    JSON.stringify([first.norm, second.norm])
  );
}

await browser.close();
await stop();
console.log(`\n${state.failed ? state.failed + " FAIL" : "tout est vert"}`);
process.exit(state.failed ? 1 : 0);
