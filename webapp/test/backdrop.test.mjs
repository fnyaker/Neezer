// The full-screen backdrop (lib/backdrop.js): a cover that fails to load, and
// the ONE blurred image at rest / TWO during a crossfade the app is held to.
//
// A CDN image that fails is worth retrying: the Deezer image CDN drops
// requests. A blob: that fails is not — its bytes are already in the page, so
// whatever refused it (a page policy, a revoked URL) refuses it again, and the
// two retries cost 1.2 s + 2.4 s of the PREVIOUS track's backdrop. That was
// every track change on the Android app while the CSP refused blob: images.
import test from "node:test";
import assert from "node:assert/strict";

globalThis.window = { addEventListener() {}, location: { href: "http://x/" } };
globalThis.localStorage = { getItem: () => null, setItem() {}, removeItem() {} };

// An <img> preloader that refuses blob: URLs and anything named "dead".
const requested = [];
globalThis.Image = class {
  constructor() {
    this.onload = this.onerror = null;
    this.complete = false;
    this.naturalWidth = 0;
  }
  removeAttribute() {}
  set src(u) {
    requested.push(u);
    setTimeout(() => {
      const ok = !u.startsWith("blob:") && !u.includes("dead");
      this.complete = true;
      this.naturalWidth = ok ? 500 : 0;
      (ok ? this.onload : this.onerror)?.();
    }, 1);
  }
};

const { createBackdrop } = await import("../src/lib/backdrop.js");
const { get } = await import("svelte/store");
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

test("a refused blob: goes straight to the same-origin copy", async () => {
  const bg = createBackdrop({ fadeMs: 5 });
  requested.length = 0;
  bg.set("blob:http://x/1f11969a", "/api/cover/689582032");
  await wait(50);
  assert.deepEqual(requested, ["blob:http://x/1f11969a", "/api/cover/689582032"]);
  const top = get(bg).at(-1);
  assert.equal(top.src, "/api/cover/689582032");
  assert.equal(top.url, "blob:http://x/1f11969a", "painted on behalf of the cover the track wants");
  bg.destroy();
});

test("a failing CDN image is still retried before falling back", async () => {
  const bg = createBackdrop({ fadeMs: 5 });
  requested.length = 0;
  bg.set("https://cdn/dead/500x500.jpg", "/api/cover/1");
  await wait(50);
  assert.deepEqual(requested, ["https://cdn/dead/500x500.jpg"], "the retry waits its back-off");
  bg.destroy();
});

test("a skip during a fade queues the cover instead of stacking a third layer", async () => {
  const bg = createBackdrop({ fadeMs: 300 });
  bg.set("https://cdn/cover/a.jpg");
  await wait(15);
  bg.set("https://cdn/cover/b.jpg");
  await wait(15);
  bg.set("https://cdn/cover/c.jpg");
  await wait(15);
  assert.deepEqual(
    get(bg).map((l) => l.url),
    ["https://cdn/cover/a.jpg", "https://cdn/cover/b.jpg"],
    "the outgoing and the incoming cover, and nothing else"
  );
  await wait(340);
  assert.deepEqual(
    get(bg).map((l) => l.url),
    ["https://cdn/cover/b.jpg", "https://cdn/cover/c.jpg"],
    "once B is opaque, A goes and the cover that waited is painted"
  );
  await wait(340);
  assert.deepEqual(get(bg).map((l) => l.url), ["https://cdn/cover/c.jpg"]);
  bg.destroy();
});

test("a burst never mounts more than two layers and settles on the newest", async () => {
  const bg = createBackdrop({ fadeMs: 120 });
  const most = [];
  for (const c of ["a", "b", "c", "d", "e"]) {
    bg.set(`https://cdn/cover/${c}.jpg`);
    await wait(10);
    most.push(get(bg).length);
  }
  assert.ok(Math.max(...most) <= 2, `at most two blurred images (saw ${most.join(", ")})`);
  await wait(400);
  assert.deepEqual(get(bg).map((l) => l.url), ["https://cdn/cover/e.jpg"]);
  bg.destroy();
});

test("the stack settles on the newest cover once it is opaque", async () => {
  const bg = createBackdrop({ fadeMs: 30 });
  bg.set("https://cdn/cover/a.jpg");
  await wait(5);
  bg.set("https://cdn/cover/b.jpg");
  await wait(80);
  assert.deepEqual(get(bg).map((l) => l.url), ["https://cdn/cover/b.jpg"]);
  bg.destroy();
});

test("the same art through the offline cache is one fade, not two", async () => {
  const bg = createBackdrop({ fadeMs: 20 });
  requested.length = 0;
  bg.set("https://cdn/cover/a.jpg", "/api/cover/track-a");
  await wait(20);
  const painted = get(bg).length;
  bg.set("blob:http://x/abc", "/api/cover/track-a");
  await wait(30);
  assert.equal(painted, 1);
  assert.equal(get(bg).length, 1, "the blob is the same cover: no second fade");
  assert.deepEqual(requested, ["https://cdn/cover/a.jpg"], "and no extra request");
  bg.destroy();
});

test("destroy leaves no timer behind", async () => {
  const bg = createBackdrop({ fadeMs: 10 });
  bg.set("https://cdn/cover/a.jpg");
  await wait(5);
  bg.set("https://cdn/cover/b.jpg");
  await wait(5);
  bg.destroy();
  const after = get(bg);
  await wait(40);
  assert.equal(get(bg), after, "the hold timer was cleared with the owner");
});
