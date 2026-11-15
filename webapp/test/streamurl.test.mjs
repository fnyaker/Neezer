import test from "node:test";
import assert from "node:assert/strict";

const store = new Map();
globalThis.localStorage = {
  getItem: (k) => (store.has(k) ? store.get(k) : null),
  setItem: (k, v) => store.set(k, String(v)),
  removeItem: (k) => store.delete(k),
};
globalThis.window = {
  addEventListener() {},
  location: { href: "http://srv/app/", hash: "" },
  matchMedia: () => ({ matches: false, addEventListener() {} }),
};
globalThis.document = { addEventListener() {}, visibilityState: "visible", hidden: false };
globalThis.location = globalThis.window.location;

const { api } = await import("../src/lib/api.js");

test("a stream URL carries the quality and, when asked, the offset", () => {
  assert.equal(api.streamUrl("abc", "FLAC"), "/api/stream/abc");
  assert.equal(api.streamUrl("abc", null), "/api/stream/abc");
  assert.equal(api.streamUrl("abc", "OPUS_128"), "/api/stream/abc?q=OPUS_128");
  assert.equal(api.streamUrl("abc", "OPUS_128", 123.456), "/api/stream/abc?q=OPUS_128&start=123.456");
  assert.equal(api.streamUrl("abc", "FLAC", 5), "/api/stream/abc?start=5.000");
  // A zero or unreadable offset is no offset: the URL the player always used.
  assert.equal(api.streamUrl("abc", "OPUS_128", 0), "/api/stream/abc?q=OPUS_128");
  assert.equal(api.streamUrl("abc", "OPUS_128", NaN), "/api/stream/abc?q=OPUS_128");
});
