// The end of the queue cannot be a dead end (stores.js).
//
// The owner's report: after enough "next" taps the web app "cannot change the
// track at all". The one deterministic client-side form of that was the last
// title — `next()` set `playing: false` and touched nothing else, so the tap
// read as a silent pause with no message. `skipNext()` is the button's verb: it
// moves when it can and, at the end, leaves the audio alone and bumps
// `atEndSeq`, which the audio owner answers with a continuation and a toast.
// Pure store simulation: no browser, no audio.

import test from "node:test";
import assert from "node:assert/strict";

const mem = new Map();
globalThis.localStorage = {
  getItem: (k) => (mem.has(k) ? mem.get(k) : null),
  setItem: (k, v) => mem.set(k, String(v)),
  removeItem: (k) => mem.delete(k),
  key: (i) => [...mem.keys()][i] ?? null,
  get length() {
    return mem.size;
  },
};
globalThis.localStorage.setItem("auth.user", JSON.stringify({ name: "a", admin: true }));
globalThis.window = { addEventListener() {}, location: { href: "http://x/app/", hash: "" } };
globalThis.document = { addEventListener() {}, visibilityState: "visible" };
globalThis.location = globalThis.window.location;

const { get } = await import("svelte/store");
const { player } = await import("../src/lib/stores.js");

const track = (id) => ({ deezer_id: String(id), title: `T${id}`, duration: 200 });

function fresh(n = 6, index = 0, extra = {}) {
  player.set({
    ...get(player),
    queue: Array.from({ length: n }, (_, i) => track(i + 1)),
    index,
    playing: true,
    currentTime: 42,
    seq: 10,
    shuffle: false,
    repeat: "off",
    autoplay: true,
    _orig: null,
    atEndSeq: 0,
    ...extra,
  });
}

test("60 manual skips circle a 20-title queue three times without losing a seq", () => {
  fresh(20, 0, { repeat: "all", seq: 10 });
  for (let i = 0; i < 60; i++) {
    const r = player.skipNext();
    assert.equal(r.moved, true, `skip ${i + 1} moved`);
    assert.equal(r.atEnd, false);
  }
  const s = get(player);
  assert.equal(s.index, 0, "three exact turns of 20");
  assert.equal(s.seq, 70, "one deliberate navigation per move, none lost");
  assert.equal(s.playing, true);
  assert.equal(s.atEndSeq, 0, "never at the end, never an atEnd event");
});

test("a skip on the last title is an event, not a silent pause", () => {
  fresh(20, 19, { repeat: "off", seq: 33 });
  const r = player.skipNext();
  assert.deepEqual(r, { moved: false, atEnd: true });
  let s = get(player);
  assert.equal(s.index, 19, "the last title stays put");
  assert.equal(s.playing, true, "the decision to pause belongs to the caller, not the store");
  assert.equal(s.seq, 33, "no navigation happened");
  assert.equal(s.atEndSeq, 1);

  // A second tap is a second event: the UI tries again (or re-says it).
  assert.deepEqual(player.skipNext(), { moved: false, atEnd: true });
  s = get(player);
  assert.equal(s.atEndSeq, 2);
  assert.equal(s.index, 19);
  assert.equal(s.playing, true);
});

test("next() at the last title still stops — the natural-end contract", () => {
  fresh(3, 2, { repeat: "off" });
  const r = player.next();
  assert.deepEqual(r, { moved: false, atEnd: true });
  assert.equal(get(player).index, 2);
  assert.equal(get(player).playing, false);
});

test("atEndSeq is a runtime event counter, never written to the session", async () => {
  fresh(3, 1, { repeat: "off" });
  player.skipNext(); // a move, then the end event on the same store
  player.skipNext();
  assert.equal(get(player).atEndSeq, 1);
  // The session blob is written on an idle callback (a 1 ms timer in Node).
  await new Promise((r) => setTimeout(r, 30));
  let raw = null;
  for (let i = 0; i < globalThis.localStorage.length; i++) {
    const k = globalThis.localStorage.key(i);
    if (k && k.endsWith("player.session")) raw = globalThis.localStorage.getItem(k);
  }
  assert.ok(raw, "the session was written");
  assert.equal(JSON.parse(raw).atEndSeq, undefined, "a reload must not restore a stale end event");
});
