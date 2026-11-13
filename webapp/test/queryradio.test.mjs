import test from "node:test";
import assert from "node:assert/strict";
import {
  RADIO_MIN_RESULTS,
  countSearchResults,
  offerRadio,
  relevantTracks,
  radioSeed,
  radioContext,
  mergeRadioTracks,
  createRadioLauncher,
} from "../src/lib/queryradio.js";

// A search that names a person: every title credited to them is a strong
// match, a title by somebody else is not. The fixtures keep the fields the
// server really sends (artist + display_artist + deezer_id).
const Q = "patrick sebastien";

const dz = (i, extra = {}) => ({
  deezer_id: "dz-" + i,
  title: "Titre " + i,
  artist: { deezer_id: "42", name: "Patrick Sébastien" },
  display_artist: "Patrick Sébastien",
  local: false,
  ...extra,
});

const yt = (i, extra = {}) => ({
  deezer_id: "yt-" + i,
  source: "youtube",
  title: "Titre " + i,
  artist: { deezer_id: "42", name: "Patrick Sébastien" },
  display_artist: "Patrick Sébastien",
  local: true,
  ...extra,
});

const entity = (i, extra = {}) => ({ deezer_id: "e-" + i, name: "Entité " + i, ...extra });

const lots = (n, make) => Array.from({ length: n }, (_, i) => make(i));

// -- how many results the button needs --------------------------------------

test("29 results are a lookup, 30 are a radio", () => {
  const below = countSearchResults(Q, { tracks: lots(29, dz) });
  const at = countSearchResults(Q, { tracks: lots(30, dz) });
  assert.equal(below, 29);
  assert.equal(at, 30);
  assert.ok(below < RADIO_MIN_RESULTS);
  assert.ok(at >= RADIO_MIN_RESULTS);
  assert.equal(offerRadio(below), false, "no button under 30");
  assert.equal(offerRadio(at), true, "the button appears at 30");
});

test("every tab counts, and each entry once", () => {
  const results = {
    tracks: lots(10, dz),
    artists: lots(5, entity),
    albums: lots(8, entity),
    playlists: lots(4, entity),
    podcasts: lots(3, entity),
  };
  assert.equal(countSearchResults(Q, results), 30);
});

test("shows are counted only where the screen shows them", () => {
  const results = {
    tracks: lots(10, dz),
    artists: lots(5, entity),
    albums: lots(8, entity),
    playlists: lots(4, entity),
    podcasts: lots(3, entity),
  };
  assert.equal(countSearchResults(Q, results), 30);
  assert.equal(countSearchResults(Q, results, [], { withPodcasts: false }), 27);
});

test("a duplicate row is not a second result", () => {
  const tracks = lots(25, dz);
  // The same 25 recordings found again by the fallback, and the same artist
  // repeated under its UUID: the screen shows 25 results, not 50 or 26.
  const videos = tracks.map((t, i) =>
    yt(i, { title: t.title, artist: t.artist, display_artist: t.display_artist }),
  );
  const artists = [entity(1), entity(1, { id: "uuid-e-1" })];
  assert.equal(countSearchResults(Q, { tracks, artists }, videos), 26);
});

test("a fallback answer is counted only when it is passed in", () => {
  const tracks = lots(20, dz);
  const videos = lots(10, (i) => yt(i, { title: "Autre " + i, deezer_id: "yt-x-" + i }));
  assert.equal(countSearchResults(Q, { tracks }), 20);
  assert.equal(countSearchResults(Q, { tracks }, videos), 30);
});

// -- the queue the radio opens with -----------------------------------------

test("only titles above their source's bar open the radio", () => {
  const results = {
    tracks: [
      dz(1, { title: "Autre", artist: "Autre", display_artist: "Autre" }),
      dz(2),
    ],
  };
  const youtube = [
    // Another artist covering the title: under YOUTUBE_MIN, the search drops it.
    yt(3, { title: "Ivanov", artist: "The Cover Band", display_artist: "The Cover Band" }),
    // A real match on the fallback, kept.
    yt(4, { title: "Patrick Sébastien en tournée" }),
  ];
  const queue = relevantTracks(Q, results, youtube);
  assert.deepEqual(
    queue.map((t) => t.deezer_id),
    ["dz-2", "yt-4"],
  );
});

test("a title found twice is one row in the queue", () => {
  const results = { tracks: [dz(1), dz(1, { id: "uuid-dz-1" })] };
  assert.equal(relevantTracks(Q, results).length, 1);
  // And the same recording through the fallback is dropped by the ranking.
  const videos = [yt(9, { title: "Titre 1" })];
  assert.equal(relevantTracks(Q, { tracks: [dz(1)] }, videos).length, 1);
});

test("the strongest match leads, and a library row wins a tie", () => {
  const weak = dz(1, { title: "Patrick", artist: "Autre", display_artist: "Autre" });
  const strong = dz(2, { title: "Titre 2" });
  const local = dz(3, { title: "Titre 3", local: true });
  const queue = relevantTracks(Q, { tracks: [weak, strong, local] });
  assert.deepEqual(
    queue.map((t) => t.deezer_id),
    ["dz-3", "dz-2", "dz-1"],
  );
});

// -- the seed and the context ------------------------------------------------

test("the best title seeds the radio, with its artist for the not-yet-in-library case", () => {
  const tracks = [dz(1), dz(2)];
  const seed = radioSeed(Q, { tracks }, tracks);
  assert.deepEqual(seed, { kind: "track", id: "dz-1", artist: "42" });
  const ctx = radioContext(Q, seed);
  assert.equal(ctx.kind, "radio");
  assert.equal(ctx.id, "dz-1");
  assert.equal(ctx.artist, "42");
  assert.equal(ctx.title, "Radio de patrick sebastien");
});

test("with no title to start from, the artist the search names seeds an artist radio", () => {
  const results = {
    tracks: [],
    artists: [{ deezer_id: "a1", name: "Patrick Sébastien" }],
  };
  const seed = radioSeed(Q, results, []);
  assert.deepEqual(seed, { kind: "artist", id: "a1" });
  assert.equal(radioContext(Q, seed).kind, "artist-radio");
});

test("a search with neither a matching title nor an artist has no seed", () => {
  assert.equal(radioSeed(Q, { tracks: [], artists: [] }, []), null);
});

test("the top-up only adds what is not already queued", () => {
  const existing = [dz(1), dz(2)];
  const add = mergeRadioTracks(existing, [dz(1), dz(3), yt(1, { title: "Titre 2" })]);
  assert.deepEqual(add.map((t) => t.deezer_id), ["dz-3"]);
});

// -- one press, one launch ---------------------------------------------------

function bench(topUp = []) {
  const calls = { play: [], extend: [], next: [], toast: [], events: [] };
  const launcher = createRadioLauncher({
    play: (tracks, start, context) => {
      calls.events.push("play");
      calls.play.push({ tracks, start, context });
    },
    extend: (tracks) => {
      calls.events.push("extend");
      calls.extend.push(tracks);
    },
    radioNext: async (seed, opts) => {
      calls.events.push("next");
      calls.next.push({ seed, opts });
      return { tracks: topUp };
    },
    toast: (message, kind) => calls.toast.push({ message, kind }),
  });
  return { launcher, calls };
}

test("the first title plays before any request, and one top-up fills the queue", async () => {
  const tracks = lots(5, dz);
  const { launcher, calls } = bench([dz(100), dz(101)]);
  const res = await launcher.launch(Q, { tracks }, []);

  assert.deepEqual(calls.events, ["play", "next", "extend"]);
  assert.equal(calls.play.length, 1);
  assert.equal(calls.play[0].start, 0);
  assert.deepEqual(calls.play[0].tracks.map((t) => t.deezer_id), tracks.map((t) => t.deezer_id));
  assert.equal(calls.play[0].context.kind, "radio");
  assert.equal(calls.next.length, 1);
  assert.deepEqual(calls.next[0].seed, { kind: "track", id: "dz-0", artist: "42" });
  assert.deepEqual(calls.next[0].opts.exclude, tracks.map((t) => t.deezer_id));
  assert.equal(calls.extend.length, 1);
  assert.equal(calls.extend[0].length, 2);
  assert.equal(res.ok, true);
  assert.equal(res.requests, 1);
  assert.equal(res.queue.length, 7);
});

test("30 relevant titles ask the radio for nothing", async () => {
  const { launcher, calls } = bench([dz(100)]);
  const res = await launcher.launch(Q, { tracks: lots(30, dz) }, []);
  assert.equal(calls.play.length, 1);
  assert.equal(calls.next.length, 0);
  assert.equal(calls.extend.length, 0);
  assert.equal(res.requests, 0);
});

test("two clicks in one burst start ONE queue and ONE request", async () => {
  const { launcher, calls } = bench([dz(100)]);
  const first = launcher.launch(Q, { tracks: lots(5, dz) }, []);
  const second = launcher.launch(Q, { tracks: lots(5, dz) }, []);
  assert.equal(first, second);
  await Promise.all([first, second]);
  assert.equal(calls.play.length, 1);
  assert.equal(calls.next.length, 1);
  assert.equal(calls.extend.length, 1);
});

test("a click just after the first launch answered does not restart the radio", async () => {
  const { launcher, calls } = bench([dz(100)]);
  const first = await launcher.launch(Q, { tracks: lots(5, dz) }, []);
  const second = await launcher.launch(Q, { tracks: lots(5, dz) }, []);
  assert.equal(second, first); // the same result, not a second queue
  assert.equal(calls.play.length, 1);
  assert.equal(calls.next.length, 1);
});

test("a new search is a new intent, even right after a launch", async () => {
  const { launcher, calls } = bench([dz(100)]);
  await launcher.launch(Q, { tracks: lots(5, dz) }, []);
  await launcher.launch("patrick", { tracks: lots(5, dz) }, []);
  assert.equal(calls.play.length, 2);
  assert.equal(calls.next.length, 2);
});

test("a queue too short for Player's buffer is topped up by the launch itself", async () => {
  const { launcher, calls } = bench([dz(100), dz(101)]);
  const res = await launcher.launch(Q, { tracks: [dz(1), dz(2)] }, []);
  assert.equal(calls.next.length, 1);
  assert.equal(res.queue.length, 4);
});

test("the screen lets Player top a queue of three or fewer (no second request)", async () => {
  const calls = { play: [], extend: [], next: [] };
  const launcher = createRadioLauncher({
    play: (tracks) => calls.play.push(tracks),
    extend: (tracks) => calls.extend.push(tracks),
    radioNext: async () => {
      calls.next.push(true);
      return { tracks: [dz(100)] };
    },
    toast: () => {},
    deferShortQueueToPlayer: true,
  });
  const res = await launcher.launch(Q, { tracks: [dz(1), dz(2)] }, []);
  assert.equal(calls.next.length, 0);
  assert.equal(calls.play.length, 1);
  assert.equal(res.requests, 0);
});

test("no seed is an honest failure, not a silent no-op", async () => {
  const { launcher, calls } = bench([]);
  const res = await launcher.launch(Q, { tracks: [], artists: [] }, []);
  assert.equal(res.ok, false);
  assert.equal(calls.play.length, 0);
  assert.equal(calls.next.length, 0);
  assert.equal(calls.toast.length, 1);
  assert.equal(calls.toast[0].kind, "error");
});

test("a new search's radio is not extended by the previous one's late answer", async () => {
  const resolvers = [];
  const calls = { play: [], extend: [] };
  const launcher = createRadioLauncher({
    play: (tracks, start, context) => calls.play.push({ tracks, start, context }),
    extend: (tracks) => calls.extend.push(tracks),
    radioNext: (seed) =>
      new Promise((res) => resolvers.push({ seed, res })),
    toast: () => {},
  });
  const old = launcher.launch(Q, { tracks: lots(5, dz) }, []);
  const fresh = launcher.launch("patrick", { tracks: lots(5, dz) }, []);
  assert.equal(calls.play.length, 2, "both clicks started what they were asked to");

  resolvers[0].res({ tracks: [dz(100)] });
  await old;
  assert.equal(calls.extend.length, 0, "the superseded answer extends nothing");

  resolvers[1].res({ tracks: [dz(101)] });
  await fresh;
  assert.equal(calls.extend.length, 1);
  assert.equal(calls.extend[0][0].deezer_id, "dz-101");
});

test("a playback started during the top-up is not extended by a late answer", async () => {
  const resolvers = [];
  const calls = { play: [], extend: [], toast: [] };
  let st = { context: null, queue: [], autoplay: true };
  const launcher = createRadioLauncher({
    play: (tracks, start, context) => {
      calls.play.push({ tracks, start, context });
      st = { context, queue: tracks.slice(), autoplay: true };
    },
    extend: (tracks) => calls.extend.push(tracks),
    radioNext: () => new Promise((res) => resolvers.push(res)),
    toast: (message, kind) => calls.toast.push({ message, kind }),
    state: () => st,
  });
  const pending = launcher.launch(Q, { tracks: lots(5, dz) }, []);
  // The person plays a playlist before the radio's answer lands.
  st = { context: { kind: "playlist", id: "p1" }, queue: [dz(999)], autoplay: true };
  resolvers[0]({ tracks: [dz(100)] });
  const res = await pending;
  assert.equal(calls.extend.length, 0, "the new queue is left alone");
  assert.equal(res.cancelled, true);
  assert.equal(calls.toast.length, 0, "and the slow answer does not toast over it");
});

test("a playback started while a seedless launch waits is not hijacked", async () => {
  const resolvers = [];
  const calls = { play: [], extend: [] };
  let st = { context: { kind: "playlist", id: "p0" }, queue: [dz(50)], autoplay: true };
  const launcher = createRadioLauncher({
    play: (tracks, start, context) => calls.play.push({ tracks, start, context }),
    extend: (tracks) => calls.extend.push(tracks),
    radioNext: () => new Promise((res) => resolvers.push(res)),
    toast: () => {},
    state: () => st,
  });
  const pending = launcher.launch(
    Q,
    { tracks: [], artists: [{ deezer_id: "a1", name: "Patrick Sébastien" }] },
    [],
  );
  st = { context: { kind: "playlist", id: "p1" }, queue: [dz(999)], autoplay: true };
  resolvers[0]({ tracks: [dz(100)] });
  const res = await pending;
  assert.equal(calls.play.length, 0, "the radio does not replace what the person started");
  assert.equal(res.cancelled, true);
});

test("a failed top-up says so and keeps the titles that did start", async () => {
  const calls = { play: [], extend: [], toast: [] };
  const launcher = createRadioLauncher({
    play: (tracks, start, context) => calls.play.push({ tracks, start, context }),
    extend: (tracks) => calls.extend.push(tracks),
    radioNext: async () => {
      throw new Error("offline");
    },
    toast: (message, kind) => calls.toast.push({ message, kind }),
  });
  const res = await launcher.launch(Q, { tracks: lots(5, dz) }, []);
  assert.equal(res.ok, true);
  assert.equal(calls.play.length, 1);
  assert.equal(calls.toast.length, 1);
  assert.equal(calls.toast[0].kind, "error");
  assert.match(calls.toast[0].message, /démarré/);
});
