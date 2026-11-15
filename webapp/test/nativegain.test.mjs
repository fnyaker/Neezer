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
globalThis.performance = { now: () => 1000 };

const S = await import("../src/lib/stores.js");
const N = await import("../src/lib/audio/nativeplayer.js");
const G = await import("../src/lib/audio/graph.js");

window.NSNative = {
  bridgeVersion: () => 11,
  nativeAudio: () => true,
  naCreate: (() => {
    let n = 1;
    return () => n++;
  })(),
  naCmd() {},
  naFx() {},
  naAnalysis() {},
  naAnalysisAvailable: () => true,
  appVersion: () => "1.6.0",
};
N.resetNativeAudio();
S.nativeFault.set(null);
S.nativePlayer.set(true);
S.normalization.set("high");

const wanted = G.directVolumeFor(-6);

test("the first track of a session is normalized, like the next one", () => {
  assert.ok(wanted < 0.95, `a loud track is turned down (${wanted.toFixed(3)})`);
  const first = N.createNativeElement();
  G.setElementGain(first, -6);
  G.registerSource(first);
  G.setPlayerVolume(1, [first]);
  assert.equal(first.volume, wanted, "the session's first track gets its ReplayGain");
  const second = N.createNativeElement();
  G.setElementGain(second, -6);
  G.registerSource(second);
  G.setPlayerVolume(1, [second]);
  assert.equal(second.volume, wanted, "and the next one the same way");
  assert.equal(first.volume, second.volume);
});

test("a gain named through setTrackGain before any play names no element", () => {
  G.setPlayerVolume(1, []);
  const orphan = N.createNativeElement();
  G.setTrackGain(-6);
  G.registerSource(orphan);
  G.setPlayerVolume(1, [orphan]);
  assert.equal(orphan.volume, 1, "which is why the player names the element at the handover");
});
