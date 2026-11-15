#!/usr/bin/env node
// The Android app's player (lib/audio/nativeplayer.js ↔ NativeAudio.kt), driven
// by the real Player.svelte against the real server: the native side is a
// stand-in that answers like NativeAudio.kt does — commands applied in order,
// its state echoed with the last command's number, ticks four times a second,
// a delivery delay on every message — and plays the sound on a hidden <audio>
// of its own, so positions are real. Then: play, pause, seek, skip, a track
// ending into the next, the crossfade (two players at once), the setting
// switched off mid-song (handed back to the page's <audio>), the screen locked
// through two track ends (bridge 7: the app goes on by itself, the page frozen
// meanwhile — nothing it is sent is delivered — catches up when it wakes), the
// same with the page only throttled and the crossfade and trim on (bridge 8:
// the app holds the list before the screen locks and cuts each song itself;
// `--old` runs that scenario as bridge 7), bridge 10's long list through 36
// ends with the page frozen and its one refill request, a media button's
// play/pause/seek answered by the app behind a throttled page (A1), a 404 on one
// title of a locked queue costing that title alone (A2), a seek in a first-play
// stream with no length, and the Réglages card at the widths it lives in.
//
//   npm run build && node test/native/run.mjs [--shots <dir>] [--old]
import { writeFileSync, mkdtempSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadPlaywright, startServer, silence, login, checker, installSessionPrefix } from "../harness.mjs";

const { chromium } = await loadPlaywright();
const LEN = 8;
const wavPath = join(mkdtempSync(join(tmpdir(), "nsnative-")), "track.wav");
writeFileSync(wavPath, silence(LEN));
const { base, ids, stop } = await startServer(["--audio", wavPath]);
const { check, state } = checker();
const shotsAt = process.argv.includes("--shots") ? process.argv[process.argv.indexOf("--shots") + 1] : null;
if (shotsAt) mkdirSync(shotsAt, { recursive: true });

const browser = await chromium.launch({ args: ["--autoplay-policy=no-user-gesture-required"] });
const song = (id, k) => ({
  deezer_id: id, title: "Song " + k, duration: LEN,
  artist: { deezer_id: "a1", name: "Band" }, album: { deezer_id: "al1", title: "Album", cover: "" },
});

// NativeAudio.kt, in the page: what the Kotlin side does, message for message.
function fakeApp() {
  const Real = window.Audio;
  // The stand-in's own clock; the page's is held while it is "frozen" (below).
  const setTimeout = window.setTimeout.bind(window);
  const setInterval = window.setInterval.bind(window);
  const DELAY = 15; // binder + evaluateJavascript
  const slots = new Map();
  let next = 1;
  window.__fake = { cmds: [], made: 0, released: 0, slots, frozen: false, held: [], lastPlayed: 0, bg: false, throttled: false, advAt: [], needCalls: 0, needsHeld: [], nextBytes: 0, pageCmds: 0, appCmds: 0, faults: [] };
  // The bench's queue can be longer than the twelve audio files the perf
  // server archives; a made-up stream URL still names a distinct track for
  // the page, but the real <audio> behind the stand-in must play a file.
  const playable = (u) => {
    if (!u.includes("/api/stream/x")) return u;
    return window.__fake.validUrl || u;
  };
  // A frozen page runs nothing it is sent: the messages wait, in order, for it to wake.
  const deliver = (id, s) => window.__nsAudio && window.__nsAudio(id, s);
  const emit = (id, s) =>
    setTimeout(() => (window.__fake.frozen ? window.__fake.held.push([id, s]) : deliver(id, s)), DELAY);
  // The page's timers: a frozen page runs none of them; they run on waking.
  // A THROTTLED page (what a hidden WebView really is: Chromium throttles a
  // hidden silent page's timers to once a minute, and every event the app sends
  // still runs) is the same for its timers and animation frames, harsher than
  // a minute but the same story: whatever the page starts there waits.
  const parked = [];
  const asleep = () => window.__fake.frozen || window.__fake.throttled;
  window.setTimeout = (fn, ms, ...a) =>
    setTimeout(() => (asleep() ? parked.push(() => fn(...a)) : fn(...a)), ms);
  window.setInterval = (fn, ms, ...a) => setInterval(() => !asleep() && fn(...a), ms);
  const raf = window.requestAnimationFrame.bind(window);
  window.requestAnimationFrame = (fn) => raf((t) => (asleep() ? parked.push(() => raf(fn)) : fn(t)));
  window.__fake.thaw = () => {
    window.__fake.frozen = false;
    window.__fake.throttled = false;
    for (const [id, s] of window.__fake.held.splice(0)) deliver(id, s);
    for (const f of parked.splice(0)) f();
    // The app's refill request waited in the renderer queue while frozen: let
    // the page's nsadvance coalescer and adopt run first (25 ms), then deliver.
    const needs = window.__fake.needsHeld.splice(0);
    if (needs.length) setTimeout(() => {
      for (const id of needs) if (window.__nsNeedNext) window.__nsNeedNext(id);
    }, 120);
  };
  // MainActivity.onStop / onStart: the app's players learn it and say it — and
  // the page is TOLD FIRST, while it can still run, which is the order the
  // activity uses (pushAppState, then setForeground): its one chance to name the
  // long list before Chromium throttles and then freezes the renderer.
  window.__fake.setBg = (bg) => {
    window.__fake.bg = bg;
    try {
      if (bg && window.__nsAppBackground) window.__nsAppBackground();
      else if (!bg && window.__nsAppForeground) window.__nsAppForeground();
    } catch (_) {
      /* the page hook is optional */
    }
    for (const sl of slots.values()) sl.send(null);
  };
  // The screen: a hidden page, as the WebView behind a locked phone is.
  window.__hidden = false;  Object.defineProperty(document, "visibilityState", { get: () => (window.__hidden ? "hidden" : "visible") });
  Object.defineProperty(document, "hidden", { get: () => window.__hidden });
  function slot(id) {
    const el = new Real();
    el.preload = "auto";
    const sl = { id, el, c: 0, item: false, prepared: false, pwr: false, seek: false, ended: false, tick: null, next: [], key: "", url: "", rep: "off", lastNeedAt: 0, faultMsg: null };
    // A first play: generated as it is sent, no seek map (NativeAudio.kt holds the seek).
    const unseekable = (sl.unseekable = () => localStorage.getItem("__unseekable") === "1" && !sl.whole);
    const st = () => {
      if (!sl.item) return 1;
      if (sl.ended) return 4;
      return el.readyState >= 3 ? 3 : 2;
    };
    sl.send = (ev) => {
      if (sl.dead) return;
      const bridge7 = version() >= 7;
      const s = { c: sl.c, st: st(), p: sl.pwr, t: el.currentTime, b: el.buffered.length ? el.buffered.end(el.buffered.length - 1) : 0, sk: sl.item && !unseekable() };
      if (bridge7) s.sc = true;
      if (version() >= 8) s.bg = window.__fake.bg;
      if (sl.chase != null) s.t = sl.chase;
      if (sl.key) Object.assign(s, { k: sl.key, u: sl.url });
      if (sl.item && isFinite(el.duration) && el.duration > 0 && !unseekable()) s.d = el.duration;
      // A title this player skipped (NativeRouting.fault): the page is told WHY
      // in the state that announces the next one, never as an `error`.
      if (sl.faultMsg) { s.fault = sl.faultMsg; sl.faultMsg = null; }
      if (ev) s.ev = ev;
      emit(id, s);
    };
    let lastSt = 1;
    const onChange = () => {
      const now = st();
      if (now === lastSt) return;
      lastSt = now;
      if (now === 3 && sl.seek) {
        sl.seek = false;
        sl.send("seeked");
      } else sl.send(null);
    };
    for (const e of ["canplay", "canplaythrough", "waiting", "loadedmetadata", "durationchange"]) el.addEventListener(e, onChange);
    el.addEventListener("seeked", () => {
      if (sl.seek && st() === 3) {
        sl.seek = false;
        lastSt = 3;
        sl.send("seeked");
      }
    });
    // NativeAudio.kt#followsList: bridge 8 follows the list only while the activity is stopped.
    const follows = () =>
      (sl.next.length || (version() >= 10 && sl.rep === "one")) &&
      sl.id === window.__fake.lastPlayed && sl.pwr && (version() < 8 || window.__fake.bg);
    // NativeAudio.kt#maybeNeedNext: bridge 10 asks the page when the list is
    // low while the activity is stopped, at most once every 30 s. A frozen
    // page queues the call; the bench holds it until thaw.
    const maybeNeed = () => {
      if (version() < 10 || !window.__fake.bg || sl.next.length > 4) return;
      const now = Date.now();
      if (now - sl.lastNeedAt < 30000) return;
      sl.lastNeedAt = now;
      window.__fake.needCalls++;
      if (window.__fake.frozen) window.__fake.needsHeld.push(id);
      else if (window.__nsNeedNext) window.__nsNeedNext(id);
    };
    // NativeAudio.kt#advance: on to the next one, by itself.
    const advance = () => {
      const n = sl.next[0];
      window.__fake.advAt.push(+el.currentTime.toFixed(3));
      sl.next = sl.next.slice(1);
      sl.end = n.e || 0;
      sl.key = n.k || "-";
      sl.url = n.url;
      window.__fake.advanced = (window.__fake.advanced || 0) + 1;
      if (!window.__fake.validUrl) window.__fake.validUrl = n.url;
      el.src = playable(n.url);
      el.volume = n.v;
      el.load();
      if (n.at > 0) el.addEventListener("loadedmetadata", () => (el.currentTime = n.at), { once: true });
      el.play().catch(() => {});
      lastSt = 2;
      sl.send("adv");
      maybeNeed();
    };
    sl.checkTrim = () => {
      if (version() >= 8 && sl.end > 0 && follows() && st() === 3 && el.currentTime >= sl.end - 0.015) advance();
    };
    // NativeAudio.transport: a media button's play/pause/seek applied to THIS
    // player while the activity is stopped. The page is told by the state.
    sl.transport = (cmd, value) => {
      const c = cmd === "toggle" ? (sl.pwr ? "pause" : "play") : cmd;
      if (c === "play") {
        if (sl.ended) {
          sl.ended = false;
          el.currentTime = 0;
        }
        sl.pwr = true;
        el.play().catch(() => {});
        sl.send(null);
        return true;
      }
      if (c === "pause") {
        sl.pwr = false;
        el.pause();
        sl.send(null);
        return true;
      }
      if (c === "seek") {
        sl.ended = false;
        sl.seek = true;
        el.currentTime = +value;
        sl.send(null);
        return true;
      }
      return false;
    };
    // NativeRouting.fault: a title that cannot be played costs that title, not
    // the list — while the activity is stopped and something follows.
    sl.fail = (msg) => {
      const canSkip = window.__fake.bg && version() >= 10 && sl.next.length &&
        sl.rep !== "one" && (window.__fake.faultSkips || 0) < 3;
      window.__fake.faults.push(msg);
      if (!canSkip) {
        emit(id, { c: sl.c, ev: "error", code: 2, msg });
        return false;
      }
      window.__fake.faultSkips = (window.__fake.faultSkips || 0) + 1;
      sl.faultMsg = msg;
      advance();
      return true;
    };
    el.addEventListener("ended", () => {
      if (follows()) {
        if (sl.next.length) return advance();
        // bridge 10 repeat one: the app loops the current source itself.
        sl.send("adv");
        el.currentTime = 0;
        el.play().catch(() => {});
        return;
      }
      sl.ended = true;
      onChange();
    });
    el.addEventListener("playing", () => {
      clearInterval(sl.tick);
      sl.tick = setInterval(() => !el.paused && sl.send("tick"), 250);
      // The trimmed end, timed on the player's own clock (NativeAudio.kt#checkTrim).
      clearInterval(sl.trim);
      sl.trim = setInterval(() => !el.paused && sl.checkTrim(), 10);
    });
    el.addEventListener("pause", () => clearInterval(sl.tick));
    el.addEventListener("error", () => emit(id, { c: sl.c, ev: "error", code: 2, msg: "fake" }));
    return sl;
  }
  // Fast-forward the sounding slot to its end, as if LEN seconds had passed:
  // the bench's native side is outside the page's frozen timers, exactly like
  // ExoPlayer reaching the end while the page runs nothing.
  window.__fake.endNow = () => {
    const sl = slots.get(window.__fake.lastPlayed) ||
      [...slots.values()].find((s) => !s.el.paused);
    if (!sl || sl.el.paused) return false;
    sl.el.dispatchEvent(new Event("ended"));
    return true;
  };
  // PlayerService.transport / MediaSessionCompat.Callback, message for message:
  // the app's own player answers while the activity is stopped
  // (NativeAudio.transport, NativeRouting.transport), the page otherwise.
  window.__fake.transport = (cmd, value) => {
    if (window.__fake.bg) {
      const sl = slots.get(window.__fake.lastPlayed);
      if (sl && sl.transport(cmd, value)) {
        window.__fake.appCmds++;
        return "app";
      }
    }
    window.__fake.pageCmds++;
    if (window.__nsNativeCmd) window.__nsNativeCmd(cmd, value);
    return "page";
  };
  // The track that is playing answers a verdict on itself (a 404, a decode
  // error): NativeAudio.onPlayerError, i.e. NativeRouting.fault.
  window.__fake.failTrack = (msg) => {
    const sl = slots.get(window.__fake.lastPlayed);
    return sl ? sl.fail(msg) : false;
  };
  // The APK under test: bridge 4 (the player alone) unless the bench says 6, 7, 8 or 10.
  const version = () => +(localStorage.getItem("__bridge") || 4);
  // The player itself breaking (the audio output refused, as ExoPlayer reports it).
  window.__fake.fail = () => {
    for (const sl of slots.values()) {
      if (sl.el.paused) continue;
      clearInterval(sl.tick);
      sl.el.pause();
      sl.pwr = false;
      emit(sl.id, { c: sl.c, ev: "error", code: 3, msg: "ERROR_CODE_AUDIO_TRACK_INIT_FAILED" });
    }
  };
  window.NSNative = {
    bridgeVersion: () => version(),
    appVersion: () => "1.6.0",
    naFx() {},
    naAnalysis() {},
    naAnalysisAvailable: () => version() >= 6,
    nativeAudio: () => true,
    naCreate(page) {
      const id = next++;
      window.__fake.made++;
      slots.set(id, slot(id));
      return id;
    },
    naCmd(id, json) {
      const o = JSON.parse(json);
      window.__fake.cmds.push(o.op);
      // Applied on the main looper: after the call returns.
      setTimeout(() => {
        const sl = slots.get(id);
        // NativeAudio.command: a player this side no longer has answers, it
        // does not swallow the command.
        if (!sl) {
          if (o.op !== "release") emit(id, { c: 0, ev: "error", code: 4, msg: "no player" });
          return;
        }
        if (sl.dead) return;
        const el = sl.el;
        sl.c = Math.max(sl.c, o.c);
        switch (o.op) {
          case "next":
            sl.next = o.q || [];
            sl.end = +o.end || 0;
            sl.rep = o.rep || "off";
            sl.lists = (sl.lists || 0) + 1;
            window.__fake.nextBytes = Math.max(window.__fake.nextBytes, json.length);
            break;
          case "src":
            sl.srcs = (sl.srcs || 0) + 1;
            sl.next = [];
            sl.end = 0;
            sl.key = "";
            sl.url = o.url;
            sl.whole = false;
            sl.chase = null;
            sl.seek = false;
            sl.ended = false;
            sl.item = true;
            sl.pwr = false;
            el.pause();
            if (!window.__fake.validUrl) window.__fake.validUrl = o.url;
            el.src = playable(o.url);
            el.volume = o.v;
            el.playbackRate = o.r;
            el.load();
            sl.send(null);
            break;
          case "clear":
            sl.item = false;
            el.pause();
            el.removeAttribute("src");
            el.load();
            sl.send(null);
            break;
          case "play":
            window.__fake.lastPlayed = id;
            if (sl.ended) {
              sl.ended = false;
              el.currentTime = 0;
            }
            sl.pwr = true;
            el.play().catch(() => {});
            sl.send(null);
            break;
          case "pause":
            sl.pwr = false;
            el.pause();
            sl.send(null);
            break;
          case "seek":
            sl.ended = false;
            window.__fake.seeks = (window.__fake.seeks || 0) + 1;
            if (sl.unseekable()) {
              // Held at its target while the stream is read whole, then landed
              // on the copy the server now has (here: a second later).
              sl.chase = o.t;
              sl.send(null);
              setTimeout(() => {
                sl.whole = true;
                sl.chase = null;
                sl.seek = true;
                el.currentTime = o.t;
              }, 1000);
              break;
            }
            sl.seek = true;
            el.currentTime = o.t;
            sl.send(null);
            break;
          case "volume":
            el.volume = o.v;
            break;
          case "rate":
            el.playbackRate = o.r;
            sl.send(null);
            break;
          case "state":
            sl.send(null);
            break;
          case "release":
            clearInterval(sl.tick);
            el.pause();
            el.removeAttribute("src");
            slots.delete(id);
            window.__fake.released++;
            break;
        }
      }, 2);
    },
  };
  // The app's player stops answering: the player is still there (the page may
  // make a new one) but this slot applies nothing and emits no state — a service
  // wedged on its main looper, or a slot the app released under the page.
  window.__fake.kill = () => {
    let n = 0;
    for (const sl of slots.values()) {
      sl.dead = true;
      clearInterval(sl.tick);
      clearInterval(sl.trim);
      sl.el.pause();
      n++;
    }
    return n;
  };
  // The page's own <audio> elements, counted (what the setting OFF uses).
  window.__pageEls = 0;
  // assets/nsshim.js, the half of the bridge that lives in the page: it replaces
  // navigator.mediaSession with one that CAPTURES the handlers Player.svelte
  // registers, and PlayerService's page-bound commands come back through
  // window.__nsNativeCmd. The bench models both, because "the command went to
  // the page" is exactly what a frozen renderer never runs — a route this fake
  // app reports must actually do something.
  const pageHandlers = {};
  const pageShim = {
    setActionHandler(name, fn) {
      pageHandlers[name] = fn;
    },
    setPositionState() {},
    metadata: null,
    playbackState: "none",
  };
  try {
    Object.defineProperty(Navigator.prototype, "mediaSession", {
      get: () => pageShim,
      configurable: true,
    });
  } catch (_) {
    /* the bench's Chromium allows it */
  }
  window.__nsNativeCmd = (cmd, value) => {
    const map = { play: "play", pause: "pause", next: "nexttrack", prev: "previoustrack", seek: "seekto" };
    const h = pageHandlers[map[cmd] || cmd];
    if (!h) return;
    if (cmd === "seek") h({ seekTime: Number(value) });
    else h({});
  };  window.Audio = function (...a) {
    window.__pageEls++;
    return new Real(...a);
  };
  window.Audio.prototype = Real.prototype;
}

async function open(ctx, settings, queue) {
  // Deezer's CDN and API are never reached from the bench.
  const offline = (u) => {
    const h = new URL(u).hostname;
    return h === "api.deezer.com" || h === "dzcdn.net" || h.endsWith(".dzcdn.net");
  };
  await ctx.route(offline, (r) => r.abort());
  await ctx.addInitScript(fakeApp);
  await ctx.addInitScript(installSessionPrefix);
  await ctx.addInitScript(() => {
    const pkey = window.__nsPkey;
    const j = sessionStorage.getItem("__inject");
    if (!j) return;
    const { session, settings } = JSON.parse(j);
    localStorage.setItem(pkey("player.session"), JSON.stringify(session));
    localStorage.removeItem(pkey("player.pos"));
    for (const [k, v] of Object.entries(settings)) localStorage.setItem(k, JSON.stringify(v));
    sessionStorage.removeItem("__inject");
  });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => console.log("pageerror:", e.message));
  await login(page, base);
  const session = { queue, index: 0, start: 0, currentTime: 0, context: null, shuffle: false, repeat: "off", _orig: null, at: Date.now() };
  await page.evaluate((j) => sessionStorage.setItem("__inject", j), JSON.stringify({ session, settings }));
  await page.reload();
  await page.waitForFunction(() => document.querySelector("footer.player .pp"), null, { timeout: 30000 });
  await page.waitForTimeout(400);
  return page;
}
const where = (page) =>
  page.evaluate(() => {
    const p = window.__nsAudioPath();
    return {
      t: p.t, paused: p.paused, why: p.plan.why,
      title: document.querySelector("footer.player .info .t")?.textContent,
      playing: [...window.__fake.slots.values()].filter((s) => !s.el.paused).length,
      made: window.__fake.made, released: window.__fake.released, pageEls: window.__pageEls,
    };
  });

const queue = ids.ids.slice(0, 3).map((id, k) => song(id, k + 1));

{
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const page = await open(ctx, { "audio.native": true }, queue);
  let w = await where(page);
  check(w.why === "native", "the plan says the app's player", w.why);
  check(w.made >= 2 && w.pageEls === 0, "both elements are the app's players, no page <audio>", `${w.made} made, ${w.pageEls} page`);
  await page.locator("footer.player .pp").click();
  await page.waitForTimeout(1500);
  w = await where(page);
  check(!w.paused && w.t > 0.9 && w.playing === 1, "play: one player runs and the position moves", `t ${w.t?.toFixed(2)}, ${w.playing} playing`);
  const a = await where(page);
  await page.waitForTimeout(600);
  const b = await where(page);
  check(Math.abs(b.t - a.t - 0.6) < 0.15, "the page's clock follows the player between ticks", `${(b.t - a.t).toFixed(3)} s in 0.6 s`);
  await page.locator("footer.player .pp").click();
  await page.waitForTimeout(700);
  const p1 = await where(page);
  await page.waitForTimeout(700);
  const p2 = await where(page);
  check(p1.paused && p2.playing === 0 && Math.abs(p2.t - p1.t) < 0.01, "pause: nothing plays, the position holds", `${p1.t.toFixed(2)} → ${p2.t.toFixed(2)}`);
  await page.locator("footer.player .pp").click();
  await page.waitForTimeout(400);
  const nextBtn = page.locator('footer.player button[aria-label="Suivant"]').first();
  await nextBtn.click();
  await page.waitForTimeout(1200);
  w = await where(page);
  check(w.title === "Song 2" && w.t < 1.5 && !w.paused && w.playing === 1, "skip: the next track plays from its start on one player", `${w.title} at ${w.t?.toFixed(2)}, ${w.playing} playing`);
  // Let it run out into track 3.
  await page.waitForTimeout((LEN - w.t + 1.5) * 1000);
  w = await where(page);
  check(w.title === "Song 3" && w.t > 0.2 && w.t < 2.5 && !w.paused, "a track ending moves on to the next by itself", `${w.title} at ${w.t?.toFixed(2)}`);
  // The setting off, mid-song: handed back to a page <audio> at the same place.
  const before = await where(page);
  const tb = Date.now();
  await page.evaluate(() => (location.hash = "#/settings/playback"));
  await page.locator(".native-tg").click();
  await page.waitForTimeout(2500);
  w = await where(page);
  const ran = (Date.now() - tb) / 1000;
  check(w.why !== "native" && w.pageEls >= 1 && w.playing === 0 && !w.paused && Math.abs(w.t - before.t - ran) < 0.8,
    "the setting off mid-song: back on the page's <audio>, same place, no app player left playing",
    `${w.why}, ${w.pageEls} page el, ${w.playing} native playing, t ${before.t.toFixed(2)} → ${w.t?.toFixed(2)}, ${w.released} released`);
  await ctx.close();
}

{
  // The crossfade on the volumes: two players at once, then one.
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const page = await open(ctx, { "audio.native": true, "fade.enabled": true, "fade.seconds": 3 }, queue);
  await page.locator("footer.player .pp").click();
  let both = 0;
  const t0 = Date.now();
  let w;
  while (Date.now() - t0 < (LEN + 0.7) * 1000) {
    await page.waitForTimeout(200);
    w = await where(page);
    if (w.playing === 2) both++;
  }
  check(both >= 5 && w.title === "Song 2" && w.playing === 1, "crossfade: both players heard together, then the next alone",
    `${(both * 0.2).toFixed(1)} s overlap, now ${w.title} on ${w.playing}`);
  await ctx.close();
}

{
  // Lot N6: nobody chose, and the app's player has the effects and the
  // animations (bridge 6): it plays. Then it fails under the song.
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const page = await open(ctx, { __bridge: 6 }, queue);
  let w = await where(page);
  check(w.why === "native" && w.pageEls === 0, "bridge 6, no choice made: the app's player by default", `${w.why}, ${w.pageEls} page el`);
  await page.locator("footer.player .pp").click();
  await page.waitForTimeout(2500);
  const before = await where(page);
  const tb = Date.now();
  await page.evaluate(() => window.__fake.fail());
  await page.waitForTimeout(1500);
  w = await where(page);
  const ran = (Date.now() - tb) / 1000;
  check(w.why !== "native" && w.pageEls >= 1 && !w.paused && w.title === before.title && w.t > before.t - 0.3 && w.t < before.t + ran + 0.3,
    "the app's player fails: the page's carries the same song on from where it was",
    `${w.why}, ${w.pageEls} page el, ${before.title} ${before.t.toFixed(2)} → ${w.title} ${w.t?.toFixed(2)} in ${ran.toFixed(1)} s`);
  await page.waitForTimeout(1000);
  const w2 = await where(page);
  check(w2.t > w.t + 0.7 && w2.playing === 0, "and keeps playing, with no app player left running", `${w.t.toFixed(2)} → ${w2.t.toFixed(2)}, ${w2.playing} native`);
  // Réglages says so, with a way back.
  for (const [name, vp] of [["desktop", { width: 1280, height: 900 }], ["phone", { width: 390, height: 844 }]]) {
    await page.setViewportSize(vp);
    await page.evaluate(() => (location.hash = "#/settings/playback"));
    await page.waitForSelector(".native-fail");
    const card = page.locator("section.card", { has: page.locator(".out-seg") });
    const box = await card.boundingBox();
    const btn = await page.locator(".native-fail .g-retry").boundingBox();
    check(btn && btn.x >= box.x && btn.x + btn.width <= box.x + box.width + 0.5 && btn.height >= 36,
      `${name}: the failure is said under the switch, with its button inside the card`, btn ? `${btn.width.toFixed(0)}×${btn.height.toFixed(0)}` : "missing");
    const on = await page.locator(".native-tg").getAttribute("aria-checked");
    check(on === "true", `${name}: the switch still shows the setting (on)`, on);
    if (shotsAt) await card.screenshot({ path: join(shotsAt, `native-fail-${name}.png`) });
  }
  await page.locator(".native-fail .g-retry").click();
  await page.waitForTimeout(2500);
  w = await where(page);
  check(w.why === "native" && w.playing === 1 && !w.paused, "Réessayer: back on the app's player, still playing", `${w.why}, ${w.playing} native playing`);
  await ctx.close();
}

{
  // Bridge 7, the screen locked: the page is hidden, then frozen (it hears
  // nothing), through the ends of two tracks.
  const q5 = ids.ids.slice(0, 4).map((id, k) => song(id, k + 1));
  const ctx = await browser.newContext({ viewport: { width: 390, height: 844 } });
  const page = await open(ctx, { __bridge: 7 }, q5);
  await page.locator("footer.player .pp").click();
  await page.waitForTimeout(1500);
  await page.evaluate(() => {
    window.__hidden = true;
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await page.waitForTimeout(300);
  const told = await page.evaluate(() => [...window.__fake.slots.values()].map((s) => s.next.length));
  check(told.some((n) => n === 3), "hidden: the app is told the three tracks that follow", JSON.stringify(told));
  const srcBefore = await page.evaluate(() => window.__fake.cmds.filter((c) => c === "src").length);
  // Frozen: nothing it is sent is delivered and its own timers stop too, so
  // nothing of the page runs but the app's stand-in.
  await page.evaluate(() => (window.__fake.frozen = true));
  // Song 1 runs out, then Song 2: ~6.5 + 8 s, then 2 s into Song 3.
  const t0 = Date.now();
  await page.waitForTimeout((LEN - 1.8 + LEN + 2) * 1000);
  const sounding = await page.evaluate(() =>
    [...window.__fake.slots.values()].filter((s) => !s.el.paused).map((s) => ({ src: s.el.src, t: s.el.currentTime, adv: window.__fake.advanced })));
  check(sounding.length === 1 && sounding[0].adv === 2 && sounding[0].src.includes(String(ids.ids[2])),
    "frozen: the app went on by itself, twice, and plays Song 3", JSON.stringify(sounding));
  await page.evaluate(() => {
    window.__hidden = false;
    window.__fake.thaw();
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await page.waitForTimeout(1200);
  const w = await where(page);
  const real = await page.evaluate(() => [...window.__fake.slots.values()].find((s) => !s.el.paused)?.el.currentTime);
  const srcAfter = await page.evaluate(() => window.__fake.cmds.filter((c) => c === "src").length);
  check(w.title === "Song 3" && !w.paused && w.playing === 1 && Math.abs(w.t - real) < 0.4,
    "awake: the page is on Song 3, where the app is, with one player sounding", `${w.title} at ${w.t?.toFixed(2)} (app ${real?.toFixed(2)}), ${w.playing} playing, ${((Date.now() - t0) / 1000).toFixed(1)} s`);
  check(srcAfter === srcBefore, "and took its source over instead of loading the track again", `${srcAfter - srcBefore} new loads`);
  const nexts = await page.evaluate(() => [...window.__fake.slots.values()].map((s) => s.next.length));
  check(nexts.every((n) => n === 0), "visible again: the app has nothing left to go on to by itself", JSON.stringify(nexts));
  await ctx.close();
}

{
  // Bridge 8, what a locked phone really does: the activity stops, the page is
  // hidden and THROTTLED — its timers and frames wait, every event the app
  // sends still runs — with the crossfade and the silence trim on (each song's
  // audio ends at 5.5 s of 8). The page used to start the crossfade itself on
  // a timeupdate and stall in it. `--old` runs it as bridge 7 did, to see that.
  const BR = process.argv.includes("--old") ? 7 : 8;
  const END = 5.5;
  const q5 = ids.ids.slice(0, 4).map((id, k) => song(id, k + 1));
  const ctx = await browser.newContext({ viewport: { width: 390, height: 844 } });
  await ctx.route("**/api/audio/edges/**", (r) =>
    r.fulfill({ contentType: "application/json", body: JSON.stringify({ ready: true, start: 0, end: END, duration: LEN }) }));
  const page = await open(ctx, { __bridge: BR, "fade.enabled": true, "fade.seconds": 3 }, q5);
  await page.locator("footer.player .pp").click();
  await page.waitForTimeout(1500);
  if (BR >= 8) {
    const known = await page.evaluate(() =>
      [...window.__fake.slots.values()].filter((s) => !s.el.paused).map((s) => ({ n: s.next.length, end: s.end, e: s.next.map((x) => x.e), lists: s.lists || 0 })));
    // A song further on carries its trim once its bounds have been asked
    // (EDGES_AHEAD of them, as the list is made).
    check(known.length === 1 && known[0].n === 3 && known[0].end === END && known[0].e[0] === END,
      "visible: the app already holds what follows, with where the songs' audio ends", JSON.stringify(known));
  }
  const srcBefore = await page.evaluate(() => window.__fake.cmds.filter((c) => c === "src").length);
  const srcsBefore = await page.evaluate(() => [...window.__fake.slots.values()].find((s) => !s.el.paused)?.srcs || 0);
  await page.evaluate(() => {
    window.__fake.setBg(true);
    window.__hidden = true;
    window.__fake.throttled = true;
    document.dispatchEvent(new Event("visibilitychange"));
  });
  let both = 0;
  const t0 = Date.now();
  while (Date.now() - t0 < (END - 1.5 + END + 1) * 1000) {
    await page.waitForTimeout(200);
    both += await page.evaluate(() => ([...window.__fake.slots.values()].filter((s) => !s.el.paused).length > 1 ? 1 : 0));
  }
  const r = await page.evaluate(() => ({
    sounding: [...window.__fake.slots.values()].filter((s) => !s.el.paused).map((s) => s.el.src),
    adv: window.__fake.advanced || 0,
    at: window.__fake.advAt,
    src: window.__fake.cmds.filter((c) => c === "src").length,
  }));
  check(r.sounding.length === 1 && r.sounding[0].includes(String(ids.ids[2])) && r.adv === 2,
    "locked screen, throttled page: the app went on by itself, twice, and plays Song 3", JSON.stringify({ ...r, both }));
  check(r.at.length === 2 && r.at.every((t) => Math.abs(t - END) < 0.1),
    "each song cut where its audio ends, by the app", JSON.stringify(r.at));
  check(r.src === srcBefore && both === 0, "and the page started no transition of its own (no load, no overlap)",
    `${r.src - srcBefore} loads, ${(both * 0.2).toFixed(1)} s with two players`);
  await page.evaluate(() => {
    window.__fake.setBg(false);
    window.__hidden = false;
    window.__fake.thaw();
    document.dispatchEvent(new Event("visibilitychange"));
  });
  // Read at once: with songs this short the page arms its crossfade into Song 4
  // a moment later, as it should when it is shown.
  await page.waitForTimeout(300);
  const w = await where(page);
  // The player sounding was given no source since the screen locked (the
  // crossfade into Song 4 may already be loading on the other one).
  const { real, srcs } = await page.evaluate(() => {
    const sl = [...window.__fake.slots.values()].find((s) => !s.el.paused);
    return { real: sl?.el.currentTime, srcs: sl?.srcs || 0 };
  });
  check(w.title === "Song 3" && !w.paused && Math.abs(w.t - real) < 0.4 && srcs === srcsBefore,
    "back in front of it: the page is on Song 3 where the app is, its source taken over",
    `${w.title} at ${w.t?.toFixed(2)} (app ${real?.toFixed(2)}), ${srcs - srcsBefore} loads on it`);
  // Shown again, the page does its own transitions as it always has: the
  // crossfade into Song 4, two players heard together.
  let both2 = 0;
  const t1 = Date.now();
  let w4;
  while (Date.now() - t1 < (END - (w.t || 0) + 3.5) * 1000) {
    await page.waitForTimeout(200);
    w4 = await where(page);
    if (w4.playing === 2) both2++;
  }
  check(both2 >= 3 && w4.title === "Song 4", "visible again: the page crossfades into Song 4 itself",
    `${(both2 * 0.2).toFixed(1)} s overlap, now ${w4.title}`);
  await ctx.close();
}

{
  // Bridge 10: a 40-title queue, a hidden page and then a FROZEN one. The app
  // must go on past the old ten-title lease with no page at all, ask the page
  // for more once when its list runs low, and the page must catch up on the
  // LAST advance (not load every intermediate track) when it wakes.
  const queueLong = Array.from({ length: 40 }, (_, k) =>
    song(k < ids.ids.length ? ids.ids[k] : "x" + (k + 1), k + 1));
  const ctx = await browser.newContext({ viewport: { width: 390, height: 844 } });
  const page = await open(ctx, { __bridge: 10 }, queueLong);
  await page.locator("footer.player .pp").click();
  await page.waitForTimeout(1200);
  const nextLen = () => page.evaluate(() => {
    // The active player is the one the page told to play last — the same rule
    // NativeAudio.followsList uses. Never "the first not paused": a second
    // slot can be alive for a moment and would hide the active list.
    const id = window.__fake.lastPlayed;
    const sl = window.__fake.slots.get(id) ||
      [...window.__fake.slots.values()].find((s) => !s.el.paused);
    const playing = [...window.__fake.slots.values()].filter((s) => !s.el.paused).length;
    return sl ? { id, n: sl.next.length, lists: sl.lists || 0, srcs: sl.srcs || 0, playing } : null;
  });
  let n = await nextLen();
  check(n && n.n === 32 && n.playing === 1,
    "bridge 10, visible: the app holds 32 titles, not 10", JSON.stringify(n));

  // The activity stops: the page names the long background list before it can
  // be throttled/frozen.
  await page.evaluate(() => {
    window.__hidden = true;
    window.__fake.setBg(true);
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await page.waitForTimeout(900);
  n = await nextLen();
  check(n && n.n === 39 && n.playing === 1,
    "background: the app holds the whole remaining queue (39)", JSON.stringify(n));
  const srcsBefore = n.srcs;

  // Frozen for 36 track ends: the page runs nothing and is told nothing; the
  // app alone must go on, well past the old ten-title limit.
  await page.evaluate(() => {
    window.__fake.frozen = true;
    for (let i = 0; i < 36; i++) window.__fake.endNow();
  });
  await page.waitForTimeout(300);
  const frozen = await page.evaluate(() => ({
    advanced: window.__fake.advanced || 0,
    needCalls: window.__fake.needCalls || 0,
    held: window.__fake.needsHeld.length,
    next: window.__fake.slots.get(window.__fake.lastPlayed)?.next.length ?? -1,
  }));
  check(frozen.advanced === 36, "frozen: the app went on 36 times without the page", JSON.stringify(frozen));
  check(frozen.needCalls === 1 && frozen.held === 1,
    "frozen: the refill was asked once when the list ran low, and waits for the page", JSON.stringify(frozen));

  // The person comes back: the queued advances and the queued refill run; the
  // page must land on the track the app is on, then name what follows it.
  await page.evaluate(() => {
    window.__hidden = false;
    window.__fake.setBg(false);
    window.__fake.thaw();
    document.dispatchEvent(new Event("visibilitychange"));
  });
  const thawAt = Date.now();
  await page.waitForFunction(
    () => document.querySelector("footer.player .info .t")?.textContent === "Song 37",
    null,
    { timeout: 5000 }
  );
  const catchUpMs = Date.now() - thawAt;
  await page.waitForTimeout(400);
  const w = await where(page);
  n = await nextLen();
  check(w.title === "Song 37" && !w.paused && w.playing === 1,
    "awake: the page caught up on the LAST advance (Song 37), one player sounding",
    `${w.title} at ${w.t?.toFixed(2)}, ${w.playing} playing, caught up in ${catchUpMs} ms`);
  check(catchUpMs < 1500, "the catch-up after the freeze is bounded", `${catchUpMs} ms`);
  check(n && n.lists >= 3 && n.n >= 3,
    "awake: one refill named what follows the app's current track", JSON.stringify(n));
  check(n && n.srcs === srcsBefore, "and the page adopted the app's source instead of reloading it",
    `${n ? n.srcs - srcsBefore : "?"} new loads`);
  const bytes = await page.evaluate(() => window.__fake.nextBytes || 0);
  check(bytes > 0 && bytes < 128 * 1024, "the longest bridge `next` payload stays under MAX_AUDIO_CMD",
    `${bytes} bytes`);
  await ctx.close();
}

{
  // A1: pause / play / seek from the notification, a headset, the lock screen
  // or the car card, with the activity stopped and the page THROTTLED (its own
  // timers wait; what the app sends it still runs). The app's player answers
  // them itself — a command sent to the page is a task a frozen renderer never
  // runs, and even a throttled one answers late.
  const ctx = await browser.newContext({ viewport: { width: 390, height: 844 } });
  const page = await open(ctx, { __bridge: 10 }, queue);
  await page.locator("footer.player .pp").click();
  await page.waitForTimeout(1200);
  await page.evaluate(() => {
    window.__hidden = true;
    window.__fake.setBg(true);
    window.__fake.throttled = true;
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await page.waitForTimeout(300);
  const pause = await page.evaluate(() => {
    const route = window.__fake.transport("pause", null);
    const sl = window.__fake.slots.get(window.__fake.lastPlayed);
    return {
      route, pwr: sl.pwr, playing: [...window.__fake.slots.values()].filter((s) => !s.el.paused).length,
      pageCmds: window.__fake.pageCmds, appCmds: window.__fake.appCmds,
    };
  });
  check(pause.route === "app" && pause.pwr === false && pause.playing === 0 && pause.pageCmds === 0,
    "a headset pause behind a throttled page stops the app's player itself, with no page command",
    JSON.stringify(pause));
  const seek = await page.evaluate(() => {
    const route = window.__fake.transport("seek", 4);
    const sl = window.__fake.slots.get(window.__fake.lastPlayed);
    return { route, t: sl.el.currentTime, pageCmds: window.__fake.pageCmds };
  });
  check(seek.route === "app" && Math.abs(seek.t - 4) < 0.05 && seek.pageCmds === 0,
    "a seek from the car card lands on the app's player", JSON.stringify(seek));
  const resumed = await page.evaluate(() => {
    const route = window.__fake.transport("play", null);
    const sl = window.__fake.slots.get(window.__fake.lastPlayed);
    return { route, pwr: sl.pwr, pageCmds: window.__fake.pageCmds };
  });
  check(resumed.route === "app" && resumed.pwr === true && resumed.pageCmds === 0,
    "and so does the play button", JSON.stringify(resumed));
  // Awake: the page adopts every one of them from the states it was sent.
  await page.evaluate(() => {
    window.__hidden = false;
    window.__fake.setBg(false);
    window.__fake.thaw();
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await page.waitForTimeout(900);
  const w = await where(page);
  const store = await page.evaluate(() => navigator.mediaSession.playbackState);
  check(w.title === "Song 1" && !w.paused && w.playing === 1 && store === "playing",
    "awake: the page is on the app's player, position taken over, store playing",
    `${w.title} at ${w.t?.toFixed(2)}, ${w.playing} playing, store ${store}`);
  check(w.t > 3.5, "and the seek it never performed itself is where the sound is", `${w.t?.toFixed(2)} s`);
  // In front of the page, nothing changed: the command goes to the page, which
  // owns the transport there.
  const visible = await page.evaluate(() => {
    const route = window.__fake.transport("pause", null);
    return { route, pageCmds: window.__fake.pageCmds };
  });
  await page.waitForTimeout(700);
  const w2 = await where(page);
  check(visible.route === "page" && w2.paused && w2.playing === 0,
    "in front of the page, a pause still goes to the page", JSON.stringify(visible));
  await ctx.close();
}

{
  // A2: a title that cannot be played (the fifth of eight answers 404) while the
  // screen is locked. It used to stop the WHOLE list the app had been given; it
  // must cost that title and nothing else.
  const q8 = Array.from({ length: 8 }, (_, k) =>
    song(k < ids.ids.length ? ids.ids[k] : "x" + (k + 1), k + 1));
  const ctx = await browser.newContext({ viewport: { width: 390, height: 844 } });
  const page = await open(ctx, { __bridge: 10 }, q8);
  await page.locator("footer.player .pp").click();
  await page.waitForTimeout(1200);
  const r = await page.evaluate(() => {
    window.__hidden = true;
    window.__fake.setBg(true);
    window.__fake.frozen = true;
    for (let i = 0; i < 4; i++) window.__fake.endNow(); // four titles played
    const before = window.__fake.advanced || 0;
    const skipped = window.__fake.failTrack("ERROR_CODE_IO_BAD_HTTP_STATUS");
    for (let i = 0; i < 2; i++) window.__fake.endNow(); // and the list goes on
    return {
      before, skipped, advanced: window.__fake.advanced || 0,
      faults: window.__fake.faults.length,
      sounding: [...window.__fake.slots.values()].filter((s) => !s.el.paused).length,
    };
  });
  check(r.before === 4 && r.skipped === true && r.advanced === 7 && r.sounding === 1,
    "404 on the fifth of eight, screen locked: the app consumes that title and plays the rest",
    JSON.stringify(r));
  await page.evaluate(() => {
    window.__hidden = false;
    window.__fake.setBg(false);
    window.__fake.thaw();
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await page.waitForFunction(
    () => document.querySelector("footer.player .info .t")?.textContent === "Song 8",
    null,
    { timeout: 5000 }
  );
  await page.waitForTimeout(400);
  const w = await where(page);
  check(w.title === "Song 8" && !w.paused && w.playing === 1,
    "awake: the page caught up on the last title the app played, nothing stalled",
    `${w.title} at ${w.t?.toFixed(2)}, ${w.playing} playing`);
  await ctx.close();
}

{
  // B1: the app's player stops answering in the middle of a queue (a service
  // wedged on its looper, a slot it no longer has). Every command then dies in
  // the void: the store moves and the sound does not, and nothing used to put
  // them back together short of restarting the app.
  const q5 = Array.from({ length: 5 }, (_, k) =>
    song(k < ids.ids.length ? ids.ids[k] : "x" + (k + 1), k + 1));
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const page = await open(ctx, { "audio.native": true }, q5);
  const made0 = await page.evaluate(() => window.__fake.made);
  await page.locator("footer.player .pp").click();
  await page.waitForTimeout(1200);
  const killed = await page.evaluate(() => window.__fake.kill());
  const t0 = Date.now();
  await page.locator('footer.player button[aria-label="Suivant"]').first().click();
  let recovered = true;
  try {
    await page.waitForFunction(
      () => {
        const live = [...window.__fake.slots.values()].filter((s) => !s.dead && !s.el.paused);
        const title = document.querySelector("footer.player .info .t")?.textContent;
        return live.length === 1 && title === "Song 2";
      },
      null,
      { timeout: 9000 }
    );
  } catch {
    recovered = false;
  }
  const elapsed = Date.now() - t0;
  const w = await where(page);
  const made1 = await page.evaluate(() => window.__fake.made);
  check(killed >= 1 && recovered, "a dead app player: the skip still reaches a player and the sound follows",
    `${killed} slot(s) silent, recovered in ${elapsed} ms`);
  check(elapsed < 7000, "and it takes the ack deadline plus one probe, not a restart", `${elapsed} ms`);
  check(w.title === "Song 2" && w.playing === 1 && !w.paused, "the title the person asked for is the one sounding",
    `${w.title} at ${w.t?.toFixed(2)}, ${w.playing} playing`);
  check(made1 === made0 + 1, "one fresh player, not a loop of them", `${made1 - made0} made`);
  check(w.pageEls === 0, "still the app's own player, not the page's", `${w.pageEls} page el`);
  await page.locator('footer.player button[aria-label="Suivant"]').first().click();
  await page.waitForTimeout(1200);
  const w2 = await where(page);
  check(w2.title === "Song 3" && w2.playing === 1 && !w2.paused, "and the queue goes on: the next skip lands too",
    `${w2.title} at ${w2.t?.toFixed(2)}, ${w2.playing} playing`);
  await ctx.close();
}

{
  // A seek in a first play: no length, no ranges. The page used to wait for a
  // `seekable` that never came (the bar moved, the sound did not).
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const page = await open(ctx, { __bridge: 7, __unseekable: 1 }, queue);
  await page.locator("footer.player .pp").click();
  await page.waitForTimeout(1500);
  const bar = page.locator("footer.player input[type=range]").first();
  const box = await bar.boundingBox();
  const sk0 = Date.now();
  await page.mouse.click(box.x + box.width * 0.75, box.y + box.height / 2);
  await page.waitForTimeout(300);
  const asked = await page.evaluate(() => window.__fake.seeks || 0);
  check(asked >= 1, "the seek goes to the app at once (no page-side chase)", `${asked} seek(s) in ${Date.now() - sk0} ms`);
  await page.waitForTimeout(1500);
  const real = await page.evaluate(() => [...window.__fake.slots.values()].find((s) => !s.el.paused)?.el.currentTime);
  const w = await where(page);
  check(real > LEN * 0.6 && Math.abs(w.t - real) < 0.4 && !w.paused, "and the sound is where the bar is", `app ${real?.toFixed(2)}, bar ${w.t?.toFixed(2)}`);
  await ctx.close();
}

{
  // B2: seeking in a stream that has no ranges. The server answers three shapes
  // (/stream/<id>/ready) and can begin encoding at an offset (?start=); the page
  // must take the right one and show the track's position while the element's
  // own clock starts at 0.
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const page = await open(ctx, { "player.quality": "OPUS_128", "audio.native": false }, queue);
  const shapes = await page.evaluate(async (one) => {
    const json = async (u) => {
      const r = await fetch(u, { credentials: "include" });
      return { status: r.status, body: await r.json() };
    };
    const slice = await fetch(`/api/stream/${one}?q=OPUS_128&start=4`, { credentials: "include" });
    return {
      flac: (await json(`/api/stream/${one}/ready`)).body,
      opus: (await json(`/api/stream/${one}/ready?q=OPUS_320`)).body,
      slice: {
        status: slice.status,
        accept: slice.headers.get("accept-ranges"),
        length: slice.headers.get("content-length"),
        start: slice.headers.get("x-ns-start"),
      },
    };
  }, ids.ids[0]);
  check(shapes.flac.ready === true && shapes.flac.ranges === "bytes",
    "an archived file answers ready, with byte ranges", JSON.stringify(shapes.flac));
  check(shapes.opus.ready === false && shapes.opus.offset === true,
    "a quality with no transcode yet: not ready, but the offset is honoured", JSON.stringify(shapes.opus));
  check(shapes.slice.status === 200 && shapes.slice.accept === "none" && !shapes.slice.length &&
    Math.abs(+shapes.slice.start - 4) < 0.01,
    "and ?start= answers the audio from there, without a length or ranges", JSON.stringify(shapes.slice));

  await page.route("**/api/stream/*/ready*", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ ready: false, ranges: null, offset: true, streaming: false, bytes: 0, cached: false }),
    })
  );
  const started = [];
  page.on("request", (r) => {
    if (r.url().includes("/api/stream/") && r.url().includes("start=")) started.push(r.url());
  });
  await page.evaluate(() => {
    document.querySelector("footer.player .pp").click();
    const bar = document.querySelector("footer.player input[type=range]");
    bar.value = "5";
    bar.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await page.waitForTimeout(2000);
  const at = await page.evaluate(() => {
    const p = window.__nsAudioPath();
    return {
      el: p.t,
      paused: p.paused,
      title: document.querySelector("footer.player .info .t")?.textContent,
    };
  });
  const shown = await page.evaluate(
    () => document.querySelector("footer.player .seek .time")?.textContent.trim()
  );
  const seconds = (() => {
    const m = /^(\d+):(\d+)$/.exec(shown || "");
    return m ? +m[1] * 60 + +m[2] : NaN;
  })();
  if (started.length) {
    check(/[?&]start=5\./.test(started[0]), "a stream with no ranges is reopened at the offset",
      started[0]);
    check(at.el < 2.5, "and the element's own clock starts at 0 (the offset is not forced on it)",
      `element ${at.el?.toFixed(2)} s`);
  } else {
    console.log("note the offset branch: not taken here (the element could seek there itself at that moment)");
  }
  check(seconds >= 5 && seconds <= 8 && !at.paused && at.title === "Song 1",
    "the bar shows the position asked for, and the sound is there",
    `${shown} (element ${at.el?.toFixed(2)}, ${at.title})`);
  await ctx.close();
}

// The card in Réglages, where it lives.
for (const [name, vp] of [["desktop", { width: 1280, height: 900 }], ["phone", { width: 390, height: 844 }]]) {
  const ctx = await browser.newContext({ viewport: vp, deviceScaleFactor: 2 });
  const page = await open(ctx, { "audio.native": true }, queue);
  await page.evaluate(() => (location.hash = "#/settings/playback"));
  await page.waitForSelector(".out-seg");
  const card = page.locator("section.card", { has: page.locator(".out-seg") });
  const box = await card.boundingBox();
  const sw = await page.locator(".native-tg").boundingBox();
  check(sw && box && sw.x >= box.x && sw.x + sw.width <= box.x + box.width + 0.5 && sw.height >= 40,
    `${name}: the switch sits inside its card, a finger's height`, sw ? `${sw.width.toFixed(0)}×${sw.height.toFixed(0)}` : "missing");
  const status = await page.locator(".out-mode").textContent();
  check(status === "Lecteur Android", `${name}: the status names the app's player`, status);
  if (shotsAt) await card.screenshot({ path: join(shotsAt, `native-${name}.png`) });
  await ctx.close();
}

await browser.close();
stop();
console.log(state.failed ? `${state.failed} FAILED` : "all ok");
process.exit(state.failed ? 1 : 0);
