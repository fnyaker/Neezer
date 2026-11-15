// The offline API cache's records (lib/apicache.js). It used to store the
// PARSED response — a structured clone of every object in it, synchronously,
// on each visit to a big list (17-24 ms for 4000 tracks, measured) — and now
// stores the response's text. Records written by an older build must still
// read, since the cache outlives an update.
import test from "node:test";
import assert from "node:assert/strict";
import { fakeIndexedDB } from "./fakeidb.mjs";
import { decode } from "../src/lib/apicache.js";

function memoryStorage() {
  const rows = new Map();
  return {
    getItem: (k) => (rows.has(k) ? rows.get(k) : null),
    setItem: (k, v) => rows.set(k, String(v)),
    removeItem: (k) => rows.delete(k),
    key: (i) => [...rows.keys()][i] ?? null,
    get length() {
      return rows.size;
    },
  };
}

test("a record written as text reads back as the response", () => {
  const data = { tracks: [{ deezer_id: "1", title: "Été" }] };
  assert.deepEqual(decode({ path: "/me/favorites", text: JSON.stringify(data), ts: 1 }), data);
});

test("a record an older build wrote as an object still reads", () => {
  const data = { playlists: [{ id: "x" }] };
  assert.deepEqual(decode({ path: "/me/playlists", data, ts: 1 }), data);
});

test("a missing or damaged record is no answer, not an error", () => {
  assert.equal(decode(null), null);
  assert.equal(decode(undefined), null);
  assert.equal(decode({ path: "/x", text: "{not json", ts: 1 }), null);
  assert.equal(decode({ path: "/x", ts: 1 }), null);
});

test("a cached answer belongs to the account that fetched it", async () => {
  globalThis.localStorage = memoryStorage();
  globalThis.sessionStorage = memoryStorage();
  globalThis.indexedDB = fakeIndexedDB();
  const { cacheGet, cachePut, cacheDelete, cachePurgeOwner, ownerKey } = await import(
    "../src/lib/apicache.js"
  );
  const { switchAccount, accountOwner } = await import("../src/lib/account.js");
  const path = "/playlist/private";
  const a = accountOwner({ name: "a" });
  const b = accountOwner({ name: "b" });
  assert.notEqual(ownerKey(a, path), ownerKey(b, path));

  assert.equal(await cacheGet(path), null);
  await cachePut(path, { playlist: { title: "A" } });
  assert.equal(await cacheGet(path), null);

  switchAccount({ name: "a" });
  await cachePut(path, { playlist: { title: "A" } });
  assert.equal((await cacheGet(path)).playlist.title, "A");

  switchAccount({ name: "b" });
  assert.equal(await cacheGet(path), null);
  await cachePut(path, { playlist: { title: "B" } });
  assert.equal((await cacheGet(path)).playlist.title, "B");

  switchAccount({ name: "a" });
  assert.equal((await cacheGet(path)).playlist.title, "A");

  switchAccount({ name: "b" });
  await cacheDelete(path);
  assert.equal(await cacheGet(path), null);

  switchAccount({ name: "a" });
  await cachePut(path, { playlist: { title: "A2" } });
  await cachePurgeOwner(a);
  assert.equal(await cacheGet(path), null);

  switchAccount({ name: "b" });
  await cachePut(path, { playlist: { title: "B2" } });
  await cachePurgeOwner(a);
  assert.equal((await cacheGet(path)).playlist.title, "B2");
});
