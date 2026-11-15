// The shared sleeve palette (lib/coverpalette.js): the maths, the cache, and
// the one rule that matters — a sleeve is decoded ONCE per URL.
//
// `analyse` is the pure half, so these tests hand it a drawn image instead of
// a browser: three vertical bands with known colours prove which pixels each
// zone averages, and a solid square proves which colour is called dominant.
// The cache is tested with an injected decoder, so "one decode" is a count, not
// a timing.
import test from "node:test";
import assert from "node:assert/strict";

// `decodeImage` is the only DOM-touching export and it is never called here;
// importing the module must still be safe without a window (it is imported by
// `npm test` in node).
const { analyse, createPaletteCache, PALETTE_SIZE, paletteFor } = await import(
  "../src/lib/coverpalette.js"
);

const RED = [200, 20, 20];
const GREEN = [20, 200, 20];
const BLUE = [20, 20, 200];
const WHITE = [255, 255, 255];

/** A width×height RGBA grid from `fn(x, y)`; alpha is the fourth byte. */
function grid(width, height, fn) {
  const data = new Uint8ClampedArray(width * height * 4);
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const [r, g, b, a = 255] = fn(x, y);
      const i = (y * width + x) * 4;
      data[i] = r;
      data[i + 1] = g;
      data[i + 2] = b;
      data[i + 3] = a;
    }
  }
  return { data, width, height };
}

test("the three vertical zones are the three columns, averaged", () => {
  const shot = grid(30, 30, (x) => (x < 10 ? RED : x < 20 ? GREEN : BLUE));
  const p = analyse(shot.data, shot.width, shot.height);
  assert.deepEqual(p.zones.left, RED);
  assert.deepEqual(p.zones.center, GREEN);
  assert.deepEqual(p.zones.right, BLUE, "a colour present only on the right stays on the right");
  // The middle row of the 3×3 grid is what a centred sleeve reads as.
  assert.deepEqual(p.middles, [RED, GREEN, BLUE]);
});

test("the three bands are the three rows, averaged", () => {
  const shot = grid(30, 30, (_x, y) => (y < 10 ? RED : y < 20 ? GREEN : BLUE));
  const p = analyse(shot.data, shot.width, shot.height);
  assert.deepEqual(p.bands.top, RED);
  assert.deepEqual(p.bands.middle, GREEN);
  assert.deepEqual(p.bands.bottom, BLUE);
});

test("dominant is the colour that COVERS the most, not the brightest", () => {
  // A white square on a red field: the white is the loudest pixel and must not
  // become the ground's tint.
  const shot = grid(40, 40, (x, y) =>
    x >= 5 && x < 15 && y >= 5 && y < 15 ? WHITE : RED
  );
  const p = analyse(shot.data, shot.width, shot.height);
  assert.deepEqual(p.dominant, RED);
  assert.ok(
    p.colors.some((c) => c[0] > 240 && c[1] > 240 && c[2] > 240),
    "the white is still reported as one of the colours"
  );
  assert.ok(p.colors.length <= 5, "at most five colours");
});

test("a transparent pixel is not part of the picture", () => {
  // A rounded PNG's corners are transparent; averaging them in would drag every
  // zone toward black.
  const shot = grid(30, 30, (x, y) => (x < 10 ? [...RED, 0] : y < 15 ? GREEN : BLUE));
  const p = analyse(shot.data, shot.width, shot.height);
  assert.deepEqual(p.zones.left, null, "a fully transparent band has no colour");
  assert.notDeepEqual(p.average, [0, 0, 0]);
});

test("an empty image is null, never a black palette", () => {
  const shot = grid(8, 8, () => [0, 0, 0, 0]);
  assert.equal(analyse(shot.data, shot.width, shot.height), null);
  assert.equal(analyse(new Uint8ClampedArray(4), 2, 2), null, "short buffer");
  assert.equal(analyse(null, 8, 8), null);
});

test("light says whether the sleeve reads as a light surface", () => {
  const whiteShot = grid(8, 8, () => WHITE);
  const blackShot = grid(8, 8, () => [0, 0, 0]);
  assert.equal(analyse(whiteShot.data, whiteShot.width, whiteShot.height).light, true);
  assert.equal(analyse(blackShot.data, blackShot.width, blackShot.height).light, false);
});

test("one decode per URL, shared by concurrent callers", async () => {
  const reference = grid(4, 4, () => GREEN);
  const palette = analyse(reference.data, reference.width, reference.height);
  let calls = 0;
  const cache = createPaletteCache({
    decode: async () => {
      calls++;
      await new Promise((r) => setTimeout(r, 5));
      return grid(4, 4, () => GREEN);
    },
  });
  const [a, b] = await Promise.all([cache.paletteFor("https://cdn/a.jpg"), cache.paletteFor("https://cdn/a.jpg")]);
  assert.equal(calls, 1, "two views asking at once share one decode");
  assert.deepEqual(a.zones.center, GREEN);
  assert.deepEqual(b.dominant, palette.dominant);
  await cache.paletteFor("https://cdn/a.jpg");
  assert.equal(calls, 1, "a second visit costs no decode");
});

test("a failed cover is not remembered", async () => {
  let calls = 0;
  const cache = createPaletteCache({
    decode: async () => {
      calls++;
      throw new Error("CDN dropped it");
    },
  });
  assert.equal(await cache.paletteFor("https://cdn/dead.jpg"), null);
  assert.equal(await cache.paletteFor("https://cdn/dead.jpg"), null);
  assert.equal(calls, 2, "the sleeve must get its colours back when the CDN answers");
});

test("the cache is bounded and evicts the oldest", async () => {
  let calls = 0;
  const cache = createPaletteCache({
    limit: 2,
    decode: async () => {
      calls++;
      return grid(3, 3, () => RED);
    },
  });
  await cache.paletteFor("a");
  await cache.paletteFor("b");
  await cache.paletteFor("c");
  assert.equal(cache.size(), 2);
  await cache.paletteFor("a");
  assert.equal(calls, 4, "the evicted one is decoded again");
});

test("the shared cache answers a non-URL with null instead of throwing", async () => {
  assert.equal(await paletteFor(null), null);
  assert.equal(await paletteFor(""), null);
  assert.equal(await paletteFor(42), null);
});

test("the canvas edge is within the 128 px ceiling", () => {
  assert.ok(PALETTE_SIZE <= 128);
});
