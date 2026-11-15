// The app's ground rules (lib/globalbackdrop.js): which URL it paints, and the
// eco contract every translucent surface reads.
//
// Both are decisions, not DOM: `backdropCover` must point at the 500px art the
// app already fetched (never a new size, never a request when the device holds
// the blob), and `data-eco` must be exactly "1" while eco is on and ABSENT
// afterwards — a leftover attribute would keep every card's blur off forever.
import test from "node:test";
import assert from "node:assert/strict";
import { writable } from "svelte/store";

const { backdropCover, coverOf, applyEco, watchEco, BACKDROP_SIZE } = await import(
  "../src/lib/globalbackdrop.js"
);
const { coverKey } = await import("../src/lib/format.js");

const CDN_1000 = "https://cdns-images.dzcdn.net/images/cover/abc/1000x1000-000000-80-0-0.jpg";
const CDN_500 = "https://cdns-images.dzcdn.net/images/cover/abc/500x500-000000-80-0-0.jpg";

test("the ground asks for the 500px art, the size the app already fetched", () => {
  assert.equal(BACKDROP_SIZE, 500);
  assert.equal(
    backdropCover({ album: { cover: CDN_1000 } }, {}),
    CDN_500,
    "a sleeve listed at 1000px is asked for at 500 — the lists' own URL"
  );
  assert.equal(backdropCover({ album: { cover: CDN_500 } }, {}), CDN_500, "no second request");
});

test("a local cover is left exactly as it is", () => {
  const local = "/api/localcover/2f1a/art.png";
  assert.equal(backdropCover({ album: { cover: local } }, {}), local);
  const blob = "blob:http://127.0.0.1:1f11";
  assert.equal(backdropCover({ album: { cover: blob } }, {}), blob);
});

test("the blob already on the device wins over the network URL", () => {
  const covers = { [coverKey(CDN_1000)]: "blob:http://127.0.0.1/deadbeef" };
  assert.equal(
    backdropCover({ album: { cover: CDN_1000 } }, covers),
    "blob:http://127.0.0.1/deadbeef",
    "the offline copy answers the 500px URL — coverKey collapses the size"
  );
});

test("a track with no art asks for nothing", () => {
  assert.equal(backdropCover(null, {}), null);
  assert.equal(backdropCover({ title: "Sans pochette" }, {}), null);
  assert.equal(backdropCover({ album: {} }, {}), null);
});

test("a podcast episode carries its art on itself", () => {
  const url = "https://cdn/episode/500x500-x.jpg";
  assert.equal(coverOf({ cover: url }), url);
  assert.equal(coverOf({ album: { cover: CDN_500 }, cover: url }), CDN_500, "the album wins when both exist");
  assert.equal(backdropCover({ cover: url }, {}), url);
});

test("eco is exactly the presence of data-eco=1, and it is removed after", () => {
  const root = { dataset: {} };
  assert.equal(applyEco(true, root), true);
  assert.equal(root.dataset.eco, "1");
  assert.equal(applyEco(true, root), true, "idempotent");
  assert.equal(root.dataset.eco, "1");
  assert.equal(applyEco(false, root), false);
  assert.equal("eco" in root.dataset, false, "the attribute must be GONE, not 0 or false");
});

test("a store drives the contract and unsubscribing stops it", () => {
  const eco = writable(false);
  const root = { dataset: {} };
  const off = watchEco(eco, root);
  assert.equal("eco" in root.dataset, false);
  eco.set(true);
  assert.equal(root.dataset.eco, "1");
  eco.set(false);
  assert.equal("eco" in root.dataset, false);
  off();
  eco.set(true);
  assert.equal("eco" in root.dataset, false, "an unsubscribed store no longer paints");
});

test("no document and no root is a quiet no-op, never a throw", () => {
  assert.equal(applyEco(true), false);
  assert.equal(applyEco(true, {}), false, "a root without a dataset is ignored");
  assert.equal(applyEco(true, null), false);
});
