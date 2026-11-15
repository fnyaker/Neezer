// The Android app's player dressed as an <audio> element (lib/audio/nativeplayer.js
// ↔ NativeAudio.kt): what the page sees for what the native side reports, and
// that a report older than the page's last command never undoes it.
import test from "node:test";
import assert from "node:assert/strict";

const store = new Map();
globalThis.localStorage = {
  getItem: (k) => (store.has(k) ? store.get(k) : null),
  setItem: (k, v) => store.set(k, String(v)),
  removeItem: (k) => store.delete(k),
};
globalThis.window = { addEventListener() {}, location: { href: "http://srv/app/", hash: "" } };
globalThis.document = { addEventListener() {}, visibilityState: "visible" };
globalThis.location = globalThis.window.location;

// A fake page clock, so the interpolation is exact.
let clock = 1000;
globalThis.performance = { now: () => clock };

const { get } = await import("svelte/store");
const S = await import("../src/lib/stores.js");
const N = await import("../src/lib/audio/nativeplayer.js");
const O = await import("../src/lib/audio/output.js");

// The native side: records commands, and answers like NativeAudio.kt.
function app({ version = 4, available = true } = {}) {
  const sent = [];
  let next = 1;
  window.__cmdLens = [];
  window.NSNative = {
    bridgeVersion: () => version,
    nativeAudio: () => available,
    naCreate: () => next++,
    naCmd: (id, json) => {
      window.__cmdLens.push(json.length);
      sent.push({ id, ...JSON.parse(json) });
    },
  };
  N.resetNativeAudio();
  return sent;
}
const flush = () => new Promise((r) => setTimeout(r, 0));
function events(el) {
  const seen = [];
  for (const t of ["play", "pause", "playing", "waiting", "canplay", "loadedmetadata", "durationchange",
    "timeupdate", "seeking", "seeked", "ended", "error", "emptied", "loadstart"])
    el.addEventListener(t, () => seen.push(t));
  return seen;
}
// What NativeAudio.kt sends: `c` is the last command it has applied.
const report = (el, s) => window.__nsAudio(el._id, s);

test("offered only by an app that has the player", () => {
  app({ version: 3 });
  assert.equal(N.nativeAudioAvailable(), false);
  app({ available: false });
  assert.equal(N.nativeAudioAvailable(), false);
  delete window.NSNative;
  N.resetNativeAudio();
  assert.equal(N.nativeAudioAvailable(), false);
  assert.equal(N.createNativeElement(), null);
  app();
  assert.equal(N.nativeAudioAvailable(), true);
});

test("the setting makes the path direct, with its own reason", () => {
  app();
  S.nativePlayer.set(false);
  assert.notEqual(get(O.outputPlan).why, "native");
  S.nativePlayer.set(true);
  assert.equal(get(N.nativeAudioWanted), true);
  assert.deepEqual([get(O.outputPlan).direct, get(O.outputPlan).why], [true, "native"]);
  S.nativePlayer.set(false);
});

test("src + load is ONE load, with the absolute URL", async () => {
  const sent = app();
  const el = N.createNativeElement();
  el.src = "/api/stream/abc?q=opus";
  el.load();
  await flush();
  const loads = sent.filter((m) => m.op === "src");
  assert.equal(loads.length, 1);
  assert.equal(loads[0].url, "http://srv/api/stream/abc?q=opus");
  assert.equal(loads[0].pre, true);
});

test("a play reaches playing, and the position runs on its own between ticks", async () => {
  const sent = app();
  const el = N.createNativeElement();
  const seen = events(el);
  el.src = "/api/stream/a";
  el.load();
  const c0 = sent.at(-1).c;
  report(el, { c: c0, st: 2, p: false, t: 0, d: 200, b: 3 });
  assert.equal(el.duration, 200);
  assert.equal(el.readyState, 1);
  const played = el.play();
  assert.equal(el.paused, false);
  const c1 = sent.at(-1).c;
  report(el, { c: c1, st: 3, p: true, t: 0, d: 200, b: 10 });
  await played;
  await flush();
  assert.ok(seen.includes("loadedmetadata") && seen.includes("canplay") && seen.includes("playing"));
  assert.equal(el.readyState, 4);
  clock += 500;
  assert.ok(Math.abs(el.currentTime - 0.5) < 1e-9);
  // A tick a little behind the interpolation does not move the playhead back.
  report(el, { c: c1, ev: "tick", st: 3, p: true, t: 0.48 });
  assert.ok(el.currentTime >= 0.5 - 1e-9);
  clock += 1000;
  assert.ok(Math.abs(el.currentTime - 1.48) < 1e-9);
});

test("a report sent before the pause reached the native side does not undo it", async () => {
  const sent = app();
  const el = N.createNativeElement();
  el.src = "/api/stream/a";
  el.load();
  el.play();
  const c1 = sent.at(-1).c;
  report(el, { c: c1, st: 3, p: true, t: 10, d: 200 });
  el.pause();
  report(el, { c: c1, ev: "tick", st: 3, p: true, t: 10.25 }); // in flight
  assert.equal(el.paused, true);
  // The native side pausing ON ITS OWN (focus lost) is adopted.
  el.play();
  const c2 = sent.at(-1).c;
  report(el, { c: c2, st: 3, p: true, t: 10.3 });
  const seen = events(el);
  report(el, { c: c2, st: 3, p: false, t: 11 });
  assert.equal(el.paused, true);
  assert.deepEqual(seen, ["pause"]);
});

test("a seek is reported as seeked only once the native side has done THAT seek", async () => {
  const sent = app();
  const el = N.createNativeElement();
  el.src = "/api/stream/a";
  el.load();
  const seen = events(el);
  el.currentTime = 30;
  const s1 = sent.at(-1).c;
  el.currentTime = 60;
  const s2 = sent.at(-1).c;
  assert.equal(el.seeking, true);
  report(el, { c: s1, ev: "seeked", st: 3, p: false, t: 30 });
  assert.equal(el.seeking, true);
  assert.equal(el.currentTime, 60);
  report(el, { c: s2, ev: "seeked", st: 3, p: false, t: 60 });
  assert.equal(el.seeking, false);
  assert.equal(el.currentTime, 60);
  await flush();
  assert.ok(seen.includes("seeking") && seen.includes("seeked"));
});

test("the end pauses then ends, like an element", () => {
  const sent = app();
  const el = N.createNativeElement();
  el.src = "/api/stream/a";
  el.load();
  el.play();
  const c = sent.at(-1).c;
  report(el, { c, st: 3, p: true, t: 199, d: 200 });
  const seen = events(el);
  report(el, { c, st: 4, p: true, t: 200, d: 200 });
  assert.deepEqual(seen, ["timeupdate", "pause", "ended"]);
  assert.equal(el.ended, true);
  assert.equal(el.paused, true);
  assert.equal(el.currentTime, 200);
});

test("reports about a replaced source are ignored, an error rejects the play", async () => {
  const sent = app();
  const el = N.createNativeElement();
  el.src = "/api/stream/a";
  el.load();
  const old = sent.at(-1).c;
  el.src = "/api/stream/b";
  el.load();
  report(el, { c: old, st: 3, p: false, t: 99, d: 300 });
  assert.equal(el.duration, NaN);
  const p = el.play();
  report(el, { c: sent.at(-1).c, ev: "error", code: 2, msg: "HTTP 503" });
  await assert.rejects(p);
  assert.equal(el.error.code, 2);
});

test("volume, rate and release go across; nothing is sent after release", () => {
  const sent = app();
  const el = N.createNativeElement();
  el.volume = 0.5;
  el.volume = 0.5; // unchanged: not sent again
  el.muted = true;
  el.playbackRate = 1.5;
  assert.deepEqual(
    sent.map((m) => [m.op, m.v ?? m.r]),
    [["volume", 0.5], ["volume", 0], ["rate", 1.5]]
  );
  el.release();
  const n = sent.length;
  el.volume = 0.1;
  el.play().catch(() => {});
  assert.equal(sent.at(-1).op === "release" ? sent.length : -1, n);
});

// Lot N6: on by default where the app's player carries everything the page's
// does, and the page's player back the moment the app's own fails.
function dspApp(version = 6, appVersion = "1.6.0") {
  const sent = app({ version });
  Object.assign(window.NSNative, {
    naFx() {},
    naAnalysis() {},
    naAnalysisAvailable: () => true,
    appVersion: () => appVersion,
  });
  N.resetNativeAudio();
  return sent;
}

test("nobody chose: on where the app's player has the effects and the animations, off where it has not", () => {
  S.nativeFault.set(null);
  S.nativePlayer.set(null);
  dspApp(6);
  assert.equal(get(N.nativeAudioWanted), true, "bridge 6: the app's player");
  app({ version: 5 });
  assert.equal(get(N.nativeAudioWanted), false, "bridge 5 (no EQ, no animations): the page's");
  S.nativePlayer.set(true);
  assert.equal(get(N.nativeAudioWanted), true, "unless the person asked for it");
  dspApp(6);
  S.nativePlayer.set(false);
  assert.equal(get(N.nativeAudioWanted), false, "and a choice of off is kept");
  S.nativePlayer.set(null);
});

test("the app's player failing hands the music to the page's, before anyone hears of the error", () => {
  S.nativeFault.set(null);
  S.nativePlayer.set(null);
  const sent = dspApp(6, "1.6.0");
  const el = N.createNativeElement();
  el.src = "/api/stream/x";
  el.load();
  let wantedAtError = null;
  el.addEventListener("error", () => (wantedAtError = get(N.nativeAudioWanted)));
  // A network error is not the player's: the page's would fail just the same.
  report(el, { c: sent.at(-1).c, ev: "error", code: 2, msg: "ERROR_CODE_IO_NETWORK_CONNECTION_FAILED" });
  assert.equal(wantedAtError, true);
  assert.equal(get(S.nativeFault), null);
  // The audio output could not be opened: that one is.
  report(el, { c: sent.at(-1).c, ev: "error", code: 3, msg: "ERROR_CODE_AUDIO_TRACK_INIT_FAILED" });
  assert.equal(wantedAtError, false, "Player.svelte's handler already reads the page's player as wanted");
  assert.equal(get(S.nativeFault).why, "ERROR_CODE_AUDIO_TRACK_INIT_FAILED");
  assert.equal(get(N.nativeAudioWanted), false);
});

test("a fault holds for a day and for this version of the app", () => {
  dspApp(6, "1.6.0");
  const at = Date.now();
  const f = { at, app: "1.6.0", why: "x" };
  assert.equal(N.faultHolds(f, at + 23 * 3600e3), true);
  assert.equal(N.faultHolds(f, at + 25 * 3600e3), false, "tried again the next day");
  dspApp(6, "1.7.0");
  assert.equal(N.faultHolds(f, at + 1000), false, "and by a new version of the app");
  S.nativeFault.set({ at, app: "1.7.0", why: "x" });
  assert.equal(get(N.nativeAudioWanted), false);
  N.clearNativeFault();
  assert.equal(get(N.nativeAudioWanted), true, "Réessayer maintenant");
});

test("an app that cannot make a player is not asked again on every element", () => {
  S.nativeFault.set(null);
  dspApp(6);
  window.NSNative.naCreate = () => 0;
  assert.equal(N.createNativeElement(), null);
  assert.equal(get(S.nativeFault).why, "no player");
  assert.equal(get(N.nativeAudioWanted), false);
  S.nativeFault.set(null);
});

test("a stream with no length is seekable: the seek goes to the app, which holds it", async () => {
  const sent = app({ version: 7 });
  const el = N.createNativeElement();
  el.src = "/api/stream/a?q=OPUS_128";
  el.load();
  el.play();
  let c = sent.at(-1).c;
  // A first play: no seek map, no duration.
  report(el, { c, st: 3, p: true, t: 5, b: 40, sk: false });
  assert.equal(el.seekable.length, 0, "an older app says nothing it cannot do");
  report(el, { c, st: 3, p: true, t: 5, b: 40, sk: false, sc: true });
  assert.equal(el.seekable.length, 1);
  assert.equal(el.seekable.end(0), Infinity);
  const seen = events(el);
  el.currentTime = 120;
  c = sent.at(-1).c;
  assert.deepEqual([sent.at(-1).op, sent.at(-1).t], ["seek", 120]);
  // Held: the app reports the target, the sound is still elsewhere.
  report(el, { c, ev: "tick", st: 3, p: true, t: 120, sk: false, sc: true });
  assert.equal(el.seeking, true);
  assert.ok(!seen.includes("timeupdate"), "nothing moves the bar while the seek is held");
  // Landed on the whole copy.
  report(el, { c, ev: "seeked", st: 3, p: true, t: 120, d: 250, sk: true, sc: true });
  await flush();
  assert.equal(el.seeking, false);
  assert.ok(seen.includes("seeked"));
  assert.ok(Math.abs(el.currentTime - 120) < 1e-6);
});

test("the next tracks go to the app only from bridge 7, and only when they change", () => {
  let sent = app({ version: 6 });
  let el = N.createNativeElement();
  el.setNext([{ url: "http://srv/api/stream/b", at: 0.5, v: 0.8, k: "b" }]);
  assert.equal(sent.filter((m) => m.op === "next").length, 0);
  sent = app({ version: 7 });
  el = N.createNativeElement();
  const list = [{ url: "http://srv/api/stream/b", at: 0.5, v: 0.8, k: "b", meta: { title: "B" } }];
  el.setNext(list);
  el.setNext(list.map((x) => ({ ...x })));
  const nexts = sent.filter((m) => m.op === "next");
  assert.equal(nexts.length, 1);
  assert.deepEqual(nexts[0].q, [{ url: "http://srv/api/stream/b", at: 0.5, v: 0.8, k: "b", meta: { title: "B" } }]);
  el.setNext([]);
  assert.deepEqual(sent.at(-1), { id: el._id, c: sent.at(-1).c, op: "next", q: [] });
  // A new source forgets what was sent (the app forgets it too).
  el.setNext(list);
  el.src = "/api/stream/c";
  el.load();
  el.setNext(list);
  assert.equal(sent.filter((m) => m.op === "next").length, 4);
});

test("bridge 8: where each track's audio ends goes with the list, and the app says when it is in the background", () => {
  let sent = app({ version: 7 });
  let el = N.createNativeElement();
  const list = [{ url: "http://srv/api/stream/b", at: 0.5, e: 181.25, v: 0.8, k: "b" }];
  el.setNext(list, 200.5);
  // Bridge 7 knows no trim: the list as it was, nothing more.
  assert.deepEqual(sent.filter((m) => m.op === "next")[0].q, [{ url: "http://srv/api/stream/b", at: 0.5, v: 0.8, k: "b", meta: null }]);
  assert.equal("end" in sent.filter((m) => m.op === "next")[0], false);
  assert.equal(el.ownsBackground, false);
  sent = app({ version: 8 });
  el = N.createNativeElement();
  assert.equal(el.ownsBackground, true);
  el.setNext(list, 200.5);
  let n = sent.filter((m) => m.op === "next");
  assert.deepEqual(n[0].q, [{ url: "http://srv/api/stream/b", at: 0.5, v: 0.8, k: "b", meta: null, e: 181.25 }]);
  assert.equal(n[0].end, 200.5);
  // The current track's end moving is a change, the same list again is not.
  el.setNext(list, 200.5);
  el.setNext(list, 199);
  n = sent.filter((m) => m.op === "next");
  assert.equal(n.length, 2);
  assert.equal(n[1].end, 199);
  // A nonsense end is no trim at all.
  el.setNext([{ ...list[0], e: -3 }], NaN);
  n = sent.filter((m) => m.op === "next");
  assert.equal(n[2].q[0].e, 0);
  assert.equal(n[2].end, 0);
  // Every state says whether the activity is stopped.
  assert.equal(el.appInBackground, false);
  report(el, { c: 0, st: 3, p: false, t: 0, sk: true, sc: true, bg: true });
  assert.equal(el.appInBackground, true);
  report(el, { c: 0, st: 3, p: false, t: 0, sk: true, sc: true, bg: false });
  assert.equal(el.appInBackground, false);
});

test("the app going on by itself: one nsadvance, nothing else until the page adopts", async () => {
  const sent = app({ version: 7 });
  const el = N.createNativeElement();
  el.src = "/api/stream/a";
  el.load();
  el.play();
  const c = sent.at(-1).c;
  report(el, { c, st: 3, p: true, t: 199, d: 200, sk: true, sc: true });
  await flush();
  const seen = [];
  for (const t of ["nsadvance", "ended", "pause", "canplay", "playing", "loadedmetadata", "durationchange", "timeupdate", "loadstart"])
    el.addEventListener(t, () => seen.push(t));
  // A page frozen through the end of the track: the first thing it hears is the app already on "b".
  report(el, { c, st: 3, p: true, t: 12, d: 180, sk: true, sc: true, k: "b", u: "http://srv/api/stream/b" });
  report(el, { c, ev: "tick", st: 3, p: true, t: 12.25, d: 180, sk: true, sc: true, k: "b", u: "http://srv/api/stream/b" });
  await new Promise((r) => setTimeout(r, 40)); // the advance burst is coalesced
  assert.deepEqual(seen, ["nsadvance"], "not an end, and nothing about a track the page does not know yet");
  assert.equal(el.advancedKey, "b");
  assert.equal(el.src, "http://srv/api/stream/b");
  assert.equal(el.paused, false);
  assert.equal(el.duration, 180);
  assert.equal(el.adopt("x"), false);
  assert.equal(el.adopt("b"), true);
  await flush();
  assert.deepEqual(seen.slice(1), ["loadstart", "durationchange", "loadedmetadata", "canplay", "playing"]);
  assert.equal(el.advancedKey, "");
  // Its position is the app's, not a restart.
  assert.ok(el.currentTime >= 12.25 - 1e-9);
  // Going on again (the next one on the list) is news again.
  report(el, { c, st: 2, p: true, t: 0, sk: true, sc: true, k: "c", u: "http://srv/api/stream/c" });
  await new Promise((r) => setTimeout(r, 40));
  assert.equal(seen.filter((t) => t === "nsadvance").length, 2);
  // A page that loads its own source instead takes the element back entirely.
  el.dropAdvance();
  el.src = "/api/stream/d";
  el.load();
  await flush();
  assert.ok(seen.includes("loadstart"));
});

test("bridge 10: a long list, repeat flags, and an old APK never gets more than ten", () => {
  let sent = app({ version: 9 });
  let el = N.createNativeElement();
  assert.equal(el.longNext, false);
  const many = Array.from({ length: 200 }, (_, i) => ({
    url: "http://srv/api/stream/" + i, at: 0, e: 0, v: 1, k: "k" + i, meta: { title: "T" + i },
  }));
  el.setNext(many, 0, "all");
  let next = sent.filter((m) => m.op === "next");
  assert.equal(next.length, 1);
  assert.equal(next[0].q.length, 10, "a bridge 9 APK would drop a longer command whole");
  assert.equal("rep" in next[0], false);

  sent = app({ version: 10 });
  el = N.createNativeElement();
  assert.equal(el.longNext, true);
  el.setNext(many, 200.5, "all");
  next = sent.filter((m) => m.op === "next");
  assert.equal(next.length, 1);
  assert.equal(next[0].q.length, 200);
  assert.equal(next[0].end, 200.5);
  assert.equal(next[0].rep, "all");
  assert.ok(Math.max(...window.__cmdLens) < 128 * 1024, "a 200-entry command stays under MAX_AUDIO_CMD");
  // The same list is not re-sent, but a refill request forces it.
  el.setNext(many, 200.5, "all");
  assert.equal(sent.filter((m) => m.op === "next").length, 1);
  el.setNext(many, 200.5, "all", true);
  assert.equal(sent.filter((m) => m.op === "next").length, 2);
  // One RE-NAMED list, even with no next track (repeat one: the app loops).
  el.setNext([], 0, "one", true);
  assert.deepEqual(sent.at(-1).q, []);
  assert.equal(sent.at(-1).rep, "one");
});

test("bridge 10: the app asks for more through __nsNeedNext, and a burst is one nsadvance", async () => {  const sent = app({ version: 10 });
  const el = N.createNativeElement();
  el.src = "/api/stream/a";
  el.load();
  el.play();
  let asked = 0;
  el.addEventListener("nsneednext", () => asked++);
  assert.equal(typeof window.__nsNeedNext, "function");
  window.__nsNeedNext(el._id);
  assert.equal(asked, 1);

  const seen = [];
  el.addEventListener("nsadvance", () => seen.push(el.advancedKey));
  // Three advances delivered before the page runs: only the last matters.
  const c = sent.at(-1).c;
  report(el, { c, st: 3, p: true, t: 1, k: "b", u: "http://srv/api/stream/b" });
  report(el, { c, st: 3, p: true, t: 1, k: "c", u: "http://srv/api/stream/c" });
  report(el, { c, st: 3, p: true, t: 1, k: "d", u: "http://srv/api/stream/d" });
  assert.deepEqual(seen, [], "nothing fires synchronously");
  await new Promise((r) => setTimeout(r, 40));
  assert.deepEqual(seen, ["d"], "one nsadvance, carrying the last key");
  assert.equal(el.advancedKey, "d");
});

// A1: a media button's pause / play / seek while the page is frozen is applied
// by the app itself (NativeAudio.transport) and reaches the page as a state —
// never as a page command, which is exactly what a frozen renderer never runs.
test("a pause the app applied behind a frozen page is adopted, not undone", async () => {
  const sent = app({ version: 10 });
  const el = N.createNativeElement();
  el.src = "/api/stream/a";
  el.load();
  el.play();
  const c = sent.at(-1).c;
  report(el, { c, st: 3, p: true, t: 10, d: 200, sk: true, sc: true });
  // The person presses pause on the headset. The renderer ran nothing, so the
  // app paused the player itself and the state it sends says so.
  report(el, { c, ev: "tick", st: 3, p: false, t: 10.25, d: 200, sk: true, sc: true });
  assert.equal(el.paused, true, "the page reads the app's pause");
  const seen = events(el);
  // Nothing about the seek/pause went back to the app: it is already applied.
  assert.equal(sent.filter((m) => m.op === "pause").length, 0);
  // A play the page issues later is the person's: it goes through.
  el.play();
  assert.equal(sent.at(-1).op, "play");
  await flush();
  assert.ok(seen.includes("play"));
});

test("the app seeking while the page is frozen moves the page's playhead", async () => {
  const sent = app({ version: 10 });
  const el = N.createNativeElement();
  el.src = "/api/stream/a";
  el.load();
  el.play();
  let c = sent.at(-1).c;
  report(el, { c, st: 3, p: true, t: 10, d: 200, sk: true, sc: true });
  const seen = events(el);
  // The car card's bar dragged: the app seeks its own player and says `seeked`.
  report(el, { c, ev: "seeked", st: 3, p: true, t: 90, d: 200, sk: true, sc: true });
  await flush();
  assert.ok(Math.abs(el.currentTime - 90) < 1e-6, `playhead at ${el.currentTime}`);
  assert.equal(el.seeking, false);
  assert.ok(seen.includes("seeked"), "the page learns the discontinuity");
  assert.equal(sent.filter((m) => m.op === "seek").length, 0, "the app did it, not the page");
});

test("a title the app skipped behind a frozen page moves the page on without faulting its player", async () => {
  const sent = app({ version: 10 });
  const el = N.createNativeElement();
  el.src = "/api/stream/a";
  el.load();
  el.play();
  const seen = events(el);
  el.addEventListener("nsadvance", () => seen.push("nsadvance"));
  const c = sent.at(-1).c;
  // Track "b" answered 404: the app consumed it, started "c" and said why in
  // the state that announces it (`fault`), NOT as an error event.
  report(el, { c, st: 2, p: true, t: 0, k: "c", u: "http://srv/api/stream/c", fault: "ERROR_CODE_IO_BAD_HTTP_STATUS", d: 180, sk: true, sc: true });
  await new Promise((r) => setTimeout(r, 40));
  assert.deepEqual(seen, ["nsadvance"], "no error event reached the page");
  assert.equal(el.error, null, "the app's player is not marked broken for one bad file");
  assert.equal(get(S.nativeFault), null);
  assert.equal(el.advancedKey, "c");
});

test("a title the app paused while the page was frozen is not resumed by the adopt", async () => {
  const sent = app({ version: 10 });
  const el = N.createNativeElement();
  el.src = "/api/stream/a";
  el.load();
  el.play();
  const c = sent.at(-1).c;
  report(el, { c, st: 3, p: true, t: 10, d: 200, k: "b", u: "http://srv/api/stream/b", sk: true, sc: true });
  // ...then the person paused from the lock screen, still frozen: the page
  // never ran the advance and never heard the pause.
  report(el, { c, ev: "tick", st: 3, p: false, t: 10.2, d: 200, k: "b", u: "http://srv/api/stream/b", sk: true, sc: true });
  await new Promise((r) => setTimeout(r, 40));
  const seen = [];
  for (const t of ["pause", "playing", "canplay", "loadstart"]) el.addEventListener(t, () => seen.push(t));
  assert.equal(el.adopt("b"), true);
  const before = sent.filter((m) => m.op === "play").length;
  // What Player.svelte#loadTrack does next, from a store that still says playing.
  await el.play();
  assert.equal(sent.filter((m) => m.op === "play").length, before, "the stale resume did not reach the app");
  await flush();
  assert.ok(seen.includes("pause"), "the page is told it is paused");
  assert.equal(el.paused, true);
  // A real tap afterwards still plays.
  el.play();
  assert.equal(sent.at(-1).op, "play");
});

test("a player the app could not make says so, whatever source the page has since sent", async () => {
  const sent = app({ version: 10 });
  const el = N.createNativeElement();
  const seen = events(el);
  el.src = "/api/stream/a";
  el.load();
  const c = sent.at(-1).c;
  // NativeAudio.refuse: the sixth slot, an ExoPlayer that threw.
  report(el, { c: 0, ev: "error", code: 4, msg: "no player" });
  await flush();
  assert.ok(seen.includes("error"), "the refusal reached the page");
  assert.equal(el.error?.message, "no player");
  assert.notEqual(get(S.nativeFault), null, "and the page's own player takes over");
  assert.ok(c > 0);
  S.nativePlayer.set(false);
});

test("a command no state ever echoes asks for a resync, once", async () => {
  const sent = app({ version: 10 });
  const el = N.createNativeElement();
  const said = [];
  el.addEventListener("nsdesync", () => said.push(el.desyncReason));
  el.src = "/api/stream/a";
  el.load();
  await new Promise((r) => setTimeout(r, 1400));
  assert.deepEqual(said, [], "the app is allowed 1.5 s to answer");
  await new Promise((r) => setTimeout(r, 300));
  assert.deepEqual(said, ["no answer to src"]);
  // A state that has seen the command settles it: nothing more is said.
  report(el, { c: sent.at(-1).c, st: 2, p: false, t: 0, sk: true, sc: true });
  await new Promise((r) => setTimeout(r, 1600));
  assert.equal(said.length, 1);
});

test("an advance whose coalescing timer was lost is announced again by the next state", async () => {
  app({ version: 7 });
  const el = N.createNativeElement();
  el.src = "/api/stream/a";
  el.load();
  el.play();
  const seen = [];
  el.addEventListener("nsadvance", () => seen.push(el.advancedKey));
  const c = el._seq;
  report(el, { c, st: 3, p: true, t: 4, d: 180, k: "b", u: "http://srv/api/stream/b", sk: true, sc: true });
  // A throttled page: the 25 ms timer never ran. The app's next tick re-arms it.
  clearTimeout(el._advTimer);
  el._advTimer = null;
  report(el, { c, ev: "tick", st: 3, p: true, t: 4.25, d: 180, k: "b", u: "http://srv/api/stream/b", sk: true, sc: true });
  await new Promise((r) => setTimeout(r, 40));
  assert.deepEqual(seen, ["b"]);
});

test("an advance nobody takes over asks for a resync instead of leaving the element deaf", async () => {
  app({ version: 7 });
  const el = N.createNativeElement();
  const seen = events(el);
  const said = [];
  el.addEventListener("nsdesync", () => said.push(el.desyncReason));
  el.src = "/api/stream/a";
  el.load();
  el.play();
  const c = el._seq;
  report(el, { c, st: 3, p: true, t: 4, d: 180, k: "b", u: "http://srv/api/stream/b", sk: true, sc: true });
  // Three seconds later the page still has not adopted it (a load in flight,
  // a frozen page that woke on another source): the element says so.
  el._advSince -= 3100; // the page has had three seconds to take it over
  report(el, { c, ev: "tick", st: 3, p: true, t: 4.25, d: 180, k: "b", u: "http://srv/api/stream/b", sk: true, sc: true });
  assert.deepEqual(said, ["advance not adopted"]);
  assert.equal(el.advancedKey, "b");
  // What the page does with it: drop the advance and load its own truth — and
  // the element is not deaf any more (an error reaches it).
  el.dropAdvance();
  assert.equal(el.advancedKey, "");
  report(el, { c, ev: "error", code: 2, msg: "gone" });
  assert.ok(seen.includes("error"));
});

test("a probe resolves on the state that answers it, and rejects when nothing does", async () => {
  const sent = app({ version: 11 });
  const el = N.createNativeElement();
  el.src = "/api/stream/a";
  el.load();
  const asked = el.probe(300);
  const last = sent.at(-1);
  assert.equal(last.op, "state", "bridge 11: a state on demand, not a side effect");
  report(el, { c: last.c, st: 3, p: true, t: 7, d: 180, k: "", u: "", sk: true, sc: true });
  const got = await asked;
  assert.equal(got, el);
  assert.equal(el.currentTime >= 7 - 1e-9, true, "the page learned where the app is");
  await assert.rejects(el.probe(120), /did not answer/);
});

test("the volume a store hands over can never throw inside a flush", () => {
  app({ version: 10 });
  const el = N.createNativeElement();
  el.volume = "0.4";
  assert.equal(el.volume, 0.4);
  el.volume = NaN;
  assert.equal(el.volume, 0.4, "a NaN is ignored, not a TypeError");
  el.volume = undefined;
  assert.equal(el.volume, 0.4);
  el.volume = 3;
  assert.equal(el.volume, 1);
});
