import test from "node:test";
import assert from "node:assert/strict";
import { get } from "svelte/store";
import { fakeIndexedDB } from "./fakeidb.mjs";

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

globalThis.localStorage = memoryStorage();
globalThis.sessionStorage = memoryStorage();
globalThis.window = { addEventListener() {}, removeEventListener() {}, location: { hash: "" } };
globalThis.indexedDB = fakeIndexedDB();
globalThis.localStorage.setItem("auth.user", JSON.stringify({ name: "a" }));

const { switchAccount, accountOwner, activeOwner } = await import("../src/lib/account.js");
const {
  loadOfflineIndex,
  listDownloads,
  getMeta,
  getObjectURL,
  removeTrack,
  clearAll,
} = await import("../src/lib/offline.js");
const { downloads, downloadsSize } = await import("../src/lib/stores.js");

const A = accountOwner({ name: "a" });
const B = accountOwner({ name: "b" });

async function put(store, row) {
  const db = globalThis.indexedDB.dbs.get("nsupy-offline");
  const t = db.transaction(store, "readwrite");
  t.objectStore(store).put(row);
  await new Promise((resolve, reject) => {
    t.oncomplete = resolve;
    t.onerror = () => reject(t.error);
  });
}

async function rows(store) {
  const db = globalThis.indexedDB.dbs.get("nsupy-offline");
  return new Promise((resolve, reject) => {
    const t = db.transaction(store, "readonly");
    const req = t.objectStore(store).getAll();
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

test("a download belongs to the account that asked for it", async () => {
  assert.equal(activeOwner(), A);
  await loadOfflineIndex();

  await put("meta", { id: "old1", size: 10, quality: "FLAC", track: { id: "old1" } });
  await put("meta", { id: "a1", owner: A, size: 100, quality: "FLAC", track: { id: "a1" } });
  await put("audio", { id: "a1", blob: new Blob(["aa"]) });
  await put("meta", { id: "b1", owner: B, size: 200, quality: "FLAC", track: { id: "b1" } });
  await put("audio", { id: "b1", blob: new Blob(["bb"]) });

  await loadOfflineIndex();
  assert.deepEqual([...get(downloads)].sort(), ["a1", "old1"]);
  assert.equal(get(downloadsSize), 110);

  const claimed = (await rows("meta")).find((m) => m.id === "old1");
  assert.equal(claimed.owner, A);

  const mine = await listDownloads();
  assert.deepEqual(mine.map((m) => m.id).sort(), ["a1", "old1"]);
  assert.equal(await getMeta("b1"), null);
  assert.equal(await getMeta("a1").then((m) => m.owner), A);

  switchAccount({ name: "b" });
  assert.equal(activeOwner(), B);
  await loadOfflineIndex();
  assert.deepEqual([...get(downloads)], ["b1"]);
  assert.equal(get(downloadsSize), 200);
  assert.equal(await getMeta("a1"), null);
  assert.equal(await getMeta("b1").then((m) => m.id), "b1");

  assert.equal(await removeTrack("a1"), false);
  assert.equal(await getObjectURL("a1"), null);
  const aAudio = (await rows("audio")).filter((r) => r.id === "a1");
  assert.equal(aAudio.length, 1);

  assert.equal(await clearAll(), true);
  assert.deepEqual([...get(downloads)], []);
  assert.equal(get(downloadsSize), 0);
  const left = (await rows("meta")).map((m) => m.id).sort();
  assert.deepEqual(left, ["a1", "old1"]);
  const audio = (await rows("audio")).map((r) => r.id).sort();
  assert.deepEqual(audio, ["a1"]);

  switchAccount({ name: "a" });
  await loadOfflineIndex();
  assert.deepEqual([...get(downloads)].sort(), ["a1", "old1"]);
  assert.equal(await removeTrack("a1"), true);
  assert.equal(await getMeta("a1"), null);
});
