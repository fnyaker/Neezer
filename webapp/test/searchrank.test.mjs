import test from "node:test";
import assert from "node:assert/strict";
import {
  topResult,
  mixPodcasts,
  matchScore,
  rankSearchResults,
  sameRecording,
  CATALOGUE_MIN,
  YOUTUBE_MIN,
} from "../src/lib/searchrank.js";

const art = (name) => ({ deezer_id: name, name });

test("an artist named exactly leads, accents and case aside", () => {
  const r = { artists: [art("Stromae Tribute"), art("Stromaé")] };
  assert.equal(topResult("stromae", r).item.name, "Stromaé");
});

test("the first artist leads on the start of its name, never from inside a word", () => {
  const r = { artists: [art("Daft Punk"), art("Daftside")] };
  assert.equal(topResult("daft", r).item.name, "Daft Punk");
  assert.equal(topResult("daft p", r).item.name, "Daft Punk");
  assert.equal(topResult("aft", r), null);
  assert.equal(topResult("da", r), null); // two letters name nothing yet
});

test("a title search does not get an artist on top", () => {
  const r = { artists: [art("Indila")], tracks: [{ title: "Dernière danse" }] };
  assert.equal(topResult("derniere danse", r), null);
});

test("a show named exactly leads only where podcasts are shown", () => {
  const r = { artists: [], podcasts: [{ deezer_id: "1", title: "Les Pieds sur terre" }] };
  assert.equal(topResult("les pieds sur terre", r), null);
  assert.equal(topResult("les pieds sur terre", r, true).kind, "podcast");
});

test("shows are mixed in after the first two albums, labelled", () => {
  const out = mixPodcasts([{ id: "a" }, { id: "b" }, { id: "c" }], [{ deezer_id: "p" }]);
  assert.deepEqual(out.map((x) => x.id || x.deezer_id), ["a", "b", "p", "c"]);
  assert.equal(out[2].podcast, true);
  assert.equal(mixPodcasts([{ id: "a" }], []).length, 1);
});

// -- how close a YouTube hit is (lib/searchrank.js#matchScore) ---------------------

const yt = (title, artist = "Indila", extra = {}) => ({
  deezer_id: "yt-" + title,
  title,
  artist: { deezer_id: "", name: artist },
  display_artist: artist,
  source: "youtube",
  local: true,
  ...extra,
});
const deezer = (title, artist = "Indila", extra = {}) => ({
  deezer_id: "dz-" + title,
  title,
  artist: { deezer_id: String(title.length), name: artist },
  local: false,
  ...extra,
});
const Q = "derniere danse indila";

test("YouTube must clear a higher bar than a catalogue result", () => {
  assert.ok(YOUTUBE_MIN > CATALOGUE_MIN);
});

test("a title exactly as typed is a full match, accents and case aside", () => {
  assert.equal(matchScore("Dernière danse", yt("Dernière danse")), 1);
  assert.equal(matchScore("DERNIERE DANSE", yt("Dernière danse")), 1);
  // With the artist typed too, the title half is not diluted by its words.
  assert.equal(matchScore(Q, yt("Dernière danse")), 1);
});

test("a partial title scores above zero but below the exact one", () => {
  const exact = matchScore("derniere danse", yt("Dernière danse"));
  const short = matchScore("derniere danse", yt("Dernière"));
  assert.ok(short > 0);
  assert.ok(short < exact);
  // "Dernière danse" is wholly in the query: typing only the start of a title
  // still finds it.
  assert.ok(matchScore("derniere", yt("Dernière danse")) > 0.6);
});

test("another artist does not clear YouTube's bar", () => {
  const right = matchScore(Q, yt("Dernière danse", "Indila"));
  const wrong = matchScore(Q, yt("Dernière danse", "The Cover Band"));
  assert.ok(wrong < right);
  assert.ok(wrong < YOUTUBE_MIN, `wrong artist scored ${wrong}`);
});

test("cover, live, remix and other versions are demoted below the plain title", () => {
  const plain = matchScore(Q, yt("Dernière danse"));
  for (const title of [
    "Dernière danse (Live)",
    "Dernière danse (Remix)",
    "Dernière danse (Cover)",
    "Dernière danse (Reprise)",
    "Dernière danse (Instrumental)",
  ]) {
    const score = matchScore(Q, yt(title));
    assert.ok(score < plain, title);
    assert.ok(score < YOUTUBE_MIN, `${title} scored ${score}`);
  }
  // Typing "live" makes it the recording that was asked for, not a version.
  assert.ok(matchScore("derniere danse live indila", yt("Dernière danse (Live)")) >= YOUTUBE_MIN);
});

test("the cosmetic decorations of an upload do not make it another recording", () => {
  const d = deezer("Dernière danse");
  assert.ok(sameRecording(d, yt("Dernière danse (Clip officiel)")));
  assert.ok(sameRecording(d, yt("Dernière danse (Official Audio)")));
  // A live version is another recording on purpose.
  assert.ok(!sameRecording(d, yt("Dernière danse (Live)")));
});

// -- the mixed list (lib/searchrank.js#rankSearchResults) --------------------------

test("an exact YouTube duplicate is never a second line", () => {
  const d = deezer("Dernière danse");
  const out = rankSearchResults(Q, [d], [yt("Dernière danse")]);
  assert.deepEqual(out.tracks, [d]);
  assert.equal(out.promoted, 0);
  assert.equal(out.dropped, 1, "the identical video is dropped, not appended");

  const clip = rankSearchResults(Q, [d], [yt("Dernière danse (Clip officiel)")]);
  assert.deepEqual(clip.tracks, [d]);
  assert.equal(clip.dropped, 1);
});

test("a closer video is promoted after the library, before the catalogue", () => {
  const local = { title: "Autre chose", artist: { name: "Indila" }, local: true };
  const cat = deezer("Dernière danse (Live)");
  const out = rankSearchResults(Q, [local, cat], [yt("Dernière danse")]);
  assert.deepEqual(
    out.tracks.map((t) => t.title),
    ["Autre chose", "Dernière danse", "Dernière danse (Live)"],
  );
  assert.equal(out.promoted, 1);
  assert.equal(out.appended, 0);
});

test("a tie keeps the Deezer result in front", () => {
  const d = deezer("Dernière danse 2024");
  const v = yt("Dernière danse 2025");
  assert.equal(matchScore(Q, d), matchScore(Q, v)); // the same match, two rows
  const out = rankSearchResults(Q, [d], [v]);
  assert.deepEqual(out.tracks, [d, v]);
  assert.equal(out.promoted, 0, "equal is not closer: Deezer wins");
  assert.equal(out.appended, 1);
});

test("a weak or wrong-artist video is dropped, not appended", () => {
  const d = deezer("Dernière danse");
  const out = rankSearchResults(Q, [d], [
    yt("Dernière danse (Live)"),
    yt("Dernière danse", "The Cover Band"),
  ]);
  assert.deepEqual(out.tracks, [d]);
  assert.equal(out.dropped, 2);
});
