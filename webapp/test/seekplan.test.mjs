import test from "node:test";
import assert from "node:assert/strict";

const {
  seekPlan,
  seekableAt,
  bufferedAt,
  seekBounced,
  seekLost,
  readiness,
  trackTimeOf,
  elementTimeOf,
  seekVerdict,
  SEEK_LAND_MS,
} = await import("../src/lib/audio/seekplan.js");

const ranges = (list) => ({
  length: list.length,
  start: (i) => list[i][0],
  end: (i) => list[i][1],
});

const el = (o = {}) => ({
  duration: 0,
  readyState: 0,
  currentTime: 0,
  seekable: ranges([]),
  buffered: ranges([]),
  ...o,
});

test("an archived file is seeked at once, even past what is buffered", () => {
  // What the server answers for a file on disk: a length, and ranges over the
  // whole of it. The browser fetches the missing bytes itself.
  const flac = el({
    duration: 240,
    readyState: 4,
    buffered: ranges([[0, 30]]),
    seekable: ranges([[0, 240]]),
  });
  assert.equal(seekPlan(flac, 180), "direct");
  assert.equal(seekPlan(flac, 5), "direct");
  assert.equal(seekableAt(flac, 180), true);
});

test("a stream with no length is chased, never re-seeked", () => {
  // A first play generated as it is sent (or a transcode): no length, no
  // ranges. Setting currentTime there makes the browser re-request the URL and
  // start the track again.
  const live = el({ duration: Infinity, readyState: 3, buffered: ranges([[0, 8]]) });
  assert.equal(seekPlan(live, 60), "chase");
  assert.equal(seekPlan(live, 4), "direct", "inside what is already there");
  const unknown = el({ duration: NaN, readyState: 3, buffered: ranges([]) });
  assert.equal(seekPlan(unknown, 30), "chase");
});

test("no metadata yet: the seek waits for it", () => {
  const pending = el({ duration: NaN, readyState: 0 });
  assert.equal(seekPlan(pending, 42), "defer");
  assert.equal(seekPlan(null, 42), "chase");
});

test("an object URL and the app's own player are seekable everywhere", () => {
  const blob = el({ duration: 0, readyState: 0 });
  assert.equal(seekPlan(blob, 42, { direct: true }), "direct");
});

test("a seek the source undid is noticed, and so is one that never landed", () => {
  const bounced = el({ duration: 0, readyState: 3, currentTime: 0.2 });
  assert.equal(seekBounced(bounced, 60), true);
  assert.equal(seekBounced(el({ currentTime: 3 }), 60), false, "a fresh track near 0 is not a bounce");
  assert.equal(seekBounced(bounced, 0.5), false);

  const stuck = el({ duration: 0, readyState: 3, currentTime: 1.2 });
  const at = 1_000_000;
  assert.equal(seekLost(stuck, 60, at, at + SEEK_LAND_MS - 1), false, "still allowed to land");
  assert.equal(seekLost(stuck, 60, at, at + SEEK_LAND_MS + 1), true);
  const landed = el({ duration: 0, readyState: 3, currentTime: 60.4 });
  assert.equal(seekLost(landed, 60, at, at + SEEK_LAND_MS + 1), false, "landed within tolerance");
});

test("a source that can be seeked directly in Chrome is what 'direct' means", () => {
  const flac = el({ duration: 240, readyState: 4, seekable: ranges([[0, 240]]), buffered: ranges([[0, 30]]) });
  assert.equal(bufferedAt(flac, 180), false);
  assert.equal(seekableAt(flac, 180), true);
  assert.equal(seekPlan(flac, 239.9), "direct");
});


test("the seek readiness answer names one of the three ways in", () => {
  // ready: a finished FILE, a length and byte ranges — reload and seek.
  assert.deepEqual(readiness({ ready: true, ranges: "bytes", offset: true }), { mode: "ready", retryMs: 0 });
  // not ready but sliceable: the server begins encoding at the offset.
  assert.deepEqual(readiness({ ready: false, ranges: null, offset: true, streaming: false }), {
    mode: "offset",
    retryMs: 0,
  });
  // neither: keep the sound and ask again, at the pace the server set.
  assert.deepEqual(readiness({ ready: false, offset: false, streaming: false }), {
    mode: "wait",
    retryMs: 1000,
  });
  assert.deepEqual(readiness({ ready: false, offset: false, streaming: true }), {
    mode: "wait",
    retryMs: 1500,
  });
});

test("the seek refusals: a byte range is a way in, a bad start is the end", () => {
  assert.deepEqual(readiness({ error: "seek with a byte range", ranges: "bytes" }, 409), {
    mode: "ready",
    retryMs: 0,
  });
  assert.deepEqual(readiness({ error: "not archived yet", ready: false, streaming: true }, 409), {
    mode: "wait",
    retryMs: 1500,
  });
  assert.deepEqual(readiness({ error: "invalid start" }, 400), { mode: "stop", retryMs: 0 });
  assert.deepEqual(readiness({ error: "boom" }, 503), { mode: "wait", retryMs: 1500 });
  assert.deepEqual(readiness(null, 0), { mode: "wait", retryMs: 1000 });
});

test("the position a slice shows is the track's, not the element's", () => {
  // A stream opened at 120 s: the element starts at 0, the person is at 120.
  assert.equal(trackTimeOf(0, 120), 120);
  assert.equal(trackTimeOf(3.5, 120), 123.5);
  // A seek to 130 s is element 10 s; a seek before the offset is clamped to 0.
  assert.equal(elementTimeOf(130, 120), 10);
  assert.equal(elementTimeOf(100, 120), 0);
  // No offset at all leaves both alone, and junk is not a position.
  assert.equal(trackTimeOf(42, 0), 42);
  assert.equal(elementTimeOf(42, 0), 42);
  assert.equal(trackTimeOf(NaN, 120), 120);
  assert.equal(elementTimeOf(undefined, 120), 0);
});

test("a live stream that claims to be seekable everywhere is not", () => {
  // Chromium reports seekable = [0, Infinity] for a source with no length: taken
  // for real ranges, every seek "lands" there and the chase never asks the
  // server — the seek then restarts the stream from 0.
  const live = el({ duration: Infinity, readyState: 4, seekable: ranges([[0, Infinity]]) });
  assert.equal(seekableAt(live, 42), false);
  assert.equal(seekPlan(live, 42), "chase");
  const file = el({ duration: 240, readyState: 4, seekable: ranges([[0, 240]]) });
  assert.equal(seekableAt(file, 42), true);
});


// A model of Player.svelte#watchSeek: one watch at a time, disarmed the moment
// the seek lands, one chase when it is lost.
function watcher() {
  let w = null;
  const chases = [];
  return {
    seek(target, at) {
      w = { target, at };
    },
    tick(position, now, readyState = 4) {
      if (!w) return;
      const v = seekVerdict({ position, target: w.target, startedAt: w.at, now, readyState });
      if (v === "landed") {
        w = null;
        return;
      }
      if (v !== "lost") return;
      chases.push(w.target);
      w = null;
    },
    get armed() {
      return w !== null;
    },
    chases,
  };
}

test("a seek that arrived is disarmed, and never chased back to later", () => {
  const m = watcher();
  m.seek(30, 0);
  m.tick(30.2, 120); // it landed
  assert.equal(m.armed, false, "the watch ends at the landing");
  m.tick(32, 5_000); // five seconds later the person is simply 2 s past it
  assert.deepEqual(m.chases, [], "no chase, no jump back to 30");
  // The trap this replaces: the verdict alone still says "lost" long after the
  // seek arrived, which is why the caller MUST disarm on "landed".
  assert.equal(seekVerdict({ position: 32, target: 30, startedAt: 0, now: 5_000, readyState: 4 }), "lost");
});

test("a seek that never lands is chased once, then the bar is the real position", () => {
  const m = watcher();
  m.seek(30, 0);
  m.tick(2, 1_000); // still where it was, deadline not reached
  assert.deepEqual(m.chases, []);
  assert.equal(m.armed, true);
  m.tick(2, 4_200); // past the deadline
  assert.deepEqual(m.chases, [30], "one chase");
  m.tick(2.5, 9_000); // the chase failed: nothing is armed any more
  assert.deepEqual(m.chases, [30], "no second chase, no loop");
  assert.equal(m.armed, false);
});

test("a source that re-fetched itself from 0 is chased at once", () => {
  const m = watcher();
  m.seek(30, 0);
  m.tick(0.2, 300); // a rangeless stream restarted from the first byte
  assert.deepEqual(m.chases, [30]);
});

test("a seek the element has not applied yet is only waited for", () => {
  const m = watcher();
  m.seek(30, 0);
  m.tick(2, 2_000, 1); // metadata only: not a miss, not a landing
  assert.deepEqual(m.chases, []);
  assert.equal(m.armed, true);
  assert.equal(seekVerdict({ position: 30, target: 30, startedAt: 0, now: 0, readyState: 4 }), "landed");
  assert.equal(seekVerdict({ position: 0, target: 30, startedAt: 0, now: 4_100, readyState: 1 }), "waiting");
});
