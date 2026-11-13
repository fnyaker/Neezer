// The car overlay's page side (lib/car.js): what the Android service can ask
// the SPA, and what it must NOT be able to ask. No player here — every function
// is injected, so a launch is observed as a call on the real entry point's
// shape (playEntity(kind, id), player.playQueue, startFlow()).
import test from "node:test";
import assert from "node:assert/strict";
import { get, writable } from "svelte/store";
import {
  carAvailable,
  carPin,
  carTrackPayload,
  installCarBridge,
  openCarSettings,
  playFromCar,
} from "../src/lib/car.js";

function freshWindow() {
  globalThis.window = {};
  return globalThis.window;
}

function deps(over = {}) {
  const calls = [];
  const player = {
    calls: [],
    playQueue(tracks, start, context) {
      this.calls.push({ tracks, start, context });
    },
  };
  const current = writable(null);
  const user = writable({ name: "nyaker" });
  let favourite = false;
  const toggled = [];
  const toastCalls = [];
  const base = {
    player,
    current,
    user,
    isFavorite: () => favourite,
    toggleFavorite: (track) => {
      toggled.push(track);
      favourite = !favourite;
    },
    playEntity: (kind, id) => calls.push({ kind, id }),
    startFlow: () => calls.push({ flow: true }),
    trackKey: (t) => (t && t.id ? String(t.id) : null),
    isSignedIn: () => !!get(user),
    favorites: async () => ({ tracks: [] }),
    toast: (message) => toastCalls.push(message),
    remote: false,
  };
  return {
    deps: { ...base, ...over },
    calls,
    player,
    current,
    user,
    toggled,
    toastCalls,
    setFavourite: (v) => (favourite = v),
  };
}

/** Let the favourites launch's request + playQueue settle. */
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

test("no bridge: nothing is available, nothing crosses", () => {
  freshWindow();
  assert.equal(carAvailable(), false);
  assert.equal(openCarSettings(), false);
  assert.equal(carPin("p1", "Voiture"), false);
  delete globalThis.window;
  assert.equal(carAvailable(), false);
});

test("an APK without carAvailable is an older APK, even with other methods", () => {
  const w = freshWindow();
  w.NSNative = { carPin() { throw new Error("must not be called"); } };
  assert.equal(carAvailable(), false);
  assert.equal(carPin("p1", "Voiture"), false);
});

test("carAvailable trusts the method, and survives it throwing", () => {
  const w = freshWindow();
  w.NSNative = { carAvailable: () => true };
  assert.equal(carAvailable(), true);
  w.NSNative.carAvailable = () => false;
  assert.equal(carAvailable(), false);
  w.NSNative.carAvailable = () => {
    throw new Error("service gone");
  };
  assert.equal(carAvailable(), false);
});

test("carPin trims, caps and reports refusal", () => {
  const w = freshWindow();
  let seen = null;
  w.NSNative = {
    carAvailable: () => true,
    carPin: (id, title) => {
      seen = { id, title };
      return id.length <= 64 && title.length <= 200;
    },
  };
  assert.equal(carPin(" p1 ", "x".repeat(400)), true);
  assert.equal(seen.id, "p1");
  assert.equal(seen.title.length, 200);
  assert.equal(carPin("x".repeat(100), "Voiture"), true);
  assert.equal(seen.id.length, 64);
  assert.equal(carPin("", "Voiture"), false);
  w.NSNative.carPin = () => false;
  assert.equal(carPin("p1", "Voiture"), false);
  w.NSNative.carPin = () => {
    throw new Error("bridge torn down");
  };
  assert.equal(carPin("p1", "Voiture"), false);
});

test("carPin hands the known track count over, and keeps the two-argument call otherwise", () => {
  const w = freshWindow();
  const calls = [];
  w.NSNative = {
    carAvailable: () => true,
    carPin: (...args) => {
      calls.push(args);
      return true;
    },
  };
  assert.equal(carPin("p1", "Voiture", 42), true);
  assert.deepEqual(calls.at(-1), ["p1", "Voiture", 42]);
  // The page often has no count (a Deezer playlist before its first read).
  assert.equal(carPin("p1", "Voiture"), true);
  assert.deepEqual(calls.at(-1), ["p1", "Voiture"]);
  assert.equal(carPin("p1", "Voiture", "n/a"), true);
  assert.deepEqual(calls.at(-1), ["p1", "Voiture"]);
  assert.equal(carPin("p1", "Voiture", 0), true);
  assert.deepEqual(calls.at(-1), ["p1", "Voiture"]);
  // An APK that returns nothing is a refusal, not a success.
  w.NSNative.carPin = () => undefined;
  assert.equal(carPin("p1", "Voiture", 42), false);
});

test("openCarSettings only calls an installed bridge", () => {
  const w = freshWindow();
  let opened = 0;
  w.NSNative = { carAvailable: () => true, openCarSettings: () => (opened++, true) };
  assert.equal(openCarSettings(), true);
  assert.equal(opened, 1);
  // An APK from before the boolean return (or any void implementation) must
  // not be read as "the screen opened".
  w.NSNative.openCarSettings = () => undefined;
  assert.equal(openCarSettings(), false);
  w.NSNative.openCarSettings = () => false;
  assert.equal(openCarSettings(), false);
  w.NSNative.openCarSettings = () => {
    throw new Error("no activity");
  };
  assert.equal(openCarSettings(), false);
});

test("a payload is one track object, never a queue or a giant string", () => {
  assert.deepEqual(carTrackPayload('{"id":"t1"}'), { id: "t1" });
  assert.equal(carTrackPayload("[1,2]"), null);
  assert.equal(carTrackPayload("not json"), null);
  assert.equal(carTrackPayload("null"), null);
  assert.equal(carTrackPayload("x".repeat(65 * 1024)), null);
});

test("playFromCar: flow uses startFlow, playlist uses playEntity(kind, id)", () => {
  const { deps: d, calls } = deps();
  assert.equal(playFromCar(d, "flow", "", "Mon Flow", null), true);
  assert.deepEqual(calls, [{ flow: true }]);
  calls.length = 0;
  assert.equal(playFromCar(d, "playlist", "p1", "Voiture", null), true);
  assert.equal(playFromCar(d, "album", "a1", "Album", null), true);
  assert.deepEqual(calls, [
    { kind: "playlist", id: "p1" },
    { kind: "album", id: "a1" },
  ]);
});

test("playFromCar: a track goes through player.playQueue with its own context", () => {
  const { deps: d, player } = deps();
  const track = { id: "t1", title: "No Fucking Soul", artist: { name: "Angerfist" } };
  assert.equal(playFromCar(d, "track", "t1", "", JSON.stringify(track)), true);
  assert.equal(player.calls.length, 1);
  assert.deepEqual(player.calls[0].tracks, [track]);
  assert.equal(player.calls[0].start, 0);
  assert.deepEqual(player.calls[0].context, { kind: "track", id: "t1" });
});

test("playFromCar: favorites reads /api/me/favorites, then plays the queue once", async () => {
  const list = [{ id: "t1", title: "A" }, { id: "t2", title: "B" }];
  let asked = 0;
  const { deps: d, player, toastCalls } = deps({
    favorites: async () => {
      asked++;
      return { tracks: list };
    },
  });
  assert.equal(playFromCar(d, "favorites", "", "Favoris", null), true);
  assert.equal(asked, 1);
  assert.deepEqual(toastCalls, ["Chargement des favoris…"]);
  await tick();
  assert.equal(player.calls.length, 1);
  assert.deepEqual(player.calls[0], {
    tracks: list,
    start: 0,
    context: { kind: "favorites", id: "favorites" },
  });
  // The page owns the wording; the native service shows no toast for this kind.
  assert.deepEqual(toastCalls, ["Chargement des favoris…", "Lecture des favoris"]);
});

test("playFromCar: no favourite starts nothing and says so", async () => {
  const { deps: d, player, toastCalls } = deps({ favorites: async () => ({ tracks: [] }) });
  assert.equal(playFromCar(d, "favorites", "", "Favoris", null), true);
  await tick();
  assert.deepEqual(player.calls, []);
  assert.deepEqual(toastCalls, ["Chargement des favoris…", "Aucun favori pour l'instant"]);
});

test("playFromCar: a 401 on the favourites says Session absente", async () => {
  const { deps: d, player, toastCalls } = deps({
    favorites: async () => {
      throw { status: 401, message: "unauthorized" };
    },
  });
  assert.equal(playFromCar(d, "favorites", "", "Favoris", null), true);
  await tick();
  assert.deepEqual(player.calls, []);
  assert.deepEqual(toastCalls, ["Chargement des favoris…", "Session absente"]);
});

test("playFromCar: favorites without a session never asks the server", async () => {
  const { deps: d, user, player } = deps();
  let asked = 0;
  user.set(null);
  const withReader = {
    ...d,
    favorites: async () => {
      asked++;
      return { tracks: [{ id: "t1" }] };
    },
  };
  assert.equal(playFromCar(withReader, "favorites", "", "Favoris", null), false);
  await tick();
  assert.equal(asked, 0);
  assert.deepEqual(player.calls, []);
});

test("playFromCar: an offline favourites read fails honestly, no queue", async () => {
  const { deps: d, player, toastCalls } = deps({
    favorites: async () => {
      throw { status: 0, offline: true };
    },
  });
  assert.equal(playFromCar(d, "favorites", "", "Favoris", null), true);
  await tick();
  assert.deepEqual(player.calls, []);
  assert.deepEqual(toastCalls, ["Chargement des favoris…", "Hors ligne : favoris indisponibles"]);
});

test("playFromCar refuses an unknown kind, a bad track and a missing id", () => {
  const { deps: d, calls, player } = deps();
  assert.equal(playFromCar(d, "radio", "r1", "Radio", null), false);
  assert.equal(playFromCar(d, "track", "t1", "", "{broken"), false);
  assert.equal(playFromCar(d, "track", "t1", "", JSON.stringify({ title: "no id" })), false);
  assert.equal(playFromCar(d, "playlist", "", "Empty", null), false);
  assert.equal(playFromCar(d, "playlist", "x".repeat(200), "Long", null), false);
  assert.deepEqual(calls, []);
  assert.deepEqual(player.calls, []);
});

test("playFromCar does nothing without a session or on a remote control", () => {
  const { deps: d, calls, user } = deps();
  user.set(null);
  assert.equal(playFromCar(d, "flow", "", "Mon Flow", null), false);
  assert.equal(playFromCar(d, "playlist", "p1", "Voiture", null), false);
  assert.deepEqual(calls, []);
  const remote = deps({ remote: true });
  assert.equal(playFromCar(remote.deps, "flow", "", "Mon Flow", null), false);
  assert.deepEqual(remote.calls, []);
});

test("installCarBridge installs once, for the page's lifetime", () => {
  freshWindow();
  const { deps: d } = deps();
  assert.equal(installCarBridge(d), true);
  assert.equal(typeof window.__nsCarPlay, "function");
  const first = window.__nsCarPlay;
  // A second install must not wrap the first (a double favourite toggle).
  assert.equal(installCarBridge(d), false);
  assert.equal(window.__nsCarPlay, first);
  // An incomplete set is refused and does not mark the page installed.
  freshWindow();
  assert.equal(installCarBridge({ player: d.player }), false);
  assert.equal(window.__nsCarInstalled, undefined);
});

test("__nsCarPlay is the same launch, reachable from the native side", () => {
  freshWindow();
  const { deps: d, calls, player, user } = deps();
  installCarBridge(d);
  assert.equal(window.__nsCarPlay("playlist", "p1", "Voiture"), true);
  assert.deepEqual(calls, [{ kind: "playlist", id: "p1" }]);
  assert.equal(
    window.__nsCarPlay("track", "t1", "", JSON.stringify({ id: "t1", title: "T" })),
    true
  );
  assert.equal(player.calls.length, 1);
  user.set(null);
  assert.equal(window.__nsCarPlay("playlist", "p1", "Voiture"), false);
  assert.equal(calls.length, 1);
});

test("__nsCarFav exists only while a track is current, and answers the new state", () => {
  freshWindow();
  const { deps: d, current, user, toggled, setFavourite } = deps();
  installCarBridge(d);
  assert.equal(window.__nsCarFav, undefined);
  const track = { id: "t1", title: "T" };
  current.set(track);
  assert.equal(typeof window.__nsCarFav, "function");
  assert.equal(window.__nsCarFav(), true); // was false -> now a favourite
  assert.equal(window.__nsCarFav(), false); // was true -> removed
  assert.deepEqual(toggled, [track, track]);
  // Logged out: the heart stops rather than firing an unauthenticated request.
  setFavourite(false);
  user.set(null);
  assert.equal(window.__nsCarFav(), false);
  assert.equal(toggled.length, 2);
  current.set(null);
  assert.equal(window.__nsCarFav, undefined);
});
