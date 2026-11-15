// Offline metadata cache for GET responses (playlists, their tracks, albums,
// artists, mixes…). It is a pure fallback: online, requests always hit the
// network and refresh the cache; only when the network fails do we serve the
// last-seen response, so a playlist and its tracklist stay browsable offline
// (and whatever was downloaded from it plays). Stored in IndexedDB (larger,
// quota-safe) rather than localStorage — track lists can be big.

import { REMOTE } from "./remote/mode.js";
import { activeOwner, ownerPrefix } from "./account.js";

const DB_NAME = "nsupy-apicache";
const DB_VERSION = 2;
const MAX_ENTRIES = 400;
let dbPromise = null;

function openDB() {
  if (dbPromise) return dbPromise;
  dbPromise = new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, DB_VERSION);
    req.onupgradeneeded = () => {
      const db = req.result;
      if (db.objectStoreNames.contains("responses")) db.deleteObjectStore("responses");
      db.createObjectStore("responses", { keyPath: "key" });
    };
    req.onsuccess = () => {
      const db = req.result;
      db.onversionchange = () => db.close();
      resolve(db);
    };
    req.onerror = () => reject(req.error);
    req.onblocked = () => {
      dbPromise = null;
      reject(new Error("indexeddb blocked"));
    };
  });
  return dbPromise;
}

function reqp(request) {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

function done(transaction) {
  return new Promise((resolve, reject) => {
    transaction.oncomplete = () => resolve();
    transaction.onerror = () => reject(transaction.error);
    transaction.onabort = () => reject(transaction.error);
  });
}

const CACHE_PREFIXES = [
  "/me/playlists",
  "/me/favorites",
  "/me/favorite-ids",
  "/me/local",
  "/playlist/",
  "/album/",
  "/artist/",
  "/home/mix/",
  "/home",
  // The mood mixes' catalog and their track lists: they are built from the
  // measured library and change slowly, so the Home row and a mood page paint
  // offline and skip a doomed refetch on a bad link.
  "/moods",
  // Lyrics are fetched on every track change — caching them makes the Paroles
  // panel work offline and skips the doomed refetch for known tracks.
  "/lyrics/",
  // A person's credits page (who wrote, composed, produced what).
  "/credits/person",
];

// A track's credits (the full-screen player's panel, the track menu's sheet):
// small, asked again on every visit, and worth having on the device offline.
const CREDITS = /^\/track\/[^/]+\/credits$/;

// Live or personal endpoints that only look content-shaped: the playlist
// version probe, the account/administration surfaces and an episode's chapters
// (they change when the episode is archived). Never served off disk.
const LIVE_PATHS = [
  /^\/playlist\/[^/]+\/version$/,
  /^\/(?:admin|invites|onboarding)(?:\/|$)/,
  /^\/podcast\/episode\/[^/]+\/chapters$/,
];

export function isCacheable(path) {
  // A remote control reads somebody else's library: never into this device's
  // offline copy of its own.
  if (REMOTE) return false;
  const p = String(path).split("?")[0];
  if (LIVE_PATHS.some((re) => re.test(p))) return false;
  return CREDITS.test(p) || CACHE_PREFIXES.some((pre) => p === pre || p.startsWith(pre));
}

// A record holds the response's JSON TEXT (`text`), or — written by an older
// build — the parsed object (`data`). See cachePut for why text.
export function decode(rec) {
  if (!rec) return null;
  if (typeof rec.text === "string") {
    try {
      return JSON.parse(rec.text);
    } catch {
      return null;
    }
  }
  return rec.data ?? null;
}

export function ownerKey(owner, path) {
  return owner + "\u0000" + path;
}

function scopeOwner(path) {
  if (!isCacheable(path)) return null;
  return activeOwner();
}

export async function cacheGet(path) {
  const owner = scopeOwner(path);
  if (!owner) return null;
  try {
    const db = await openDB();
    const rec = await reqp(
      db.transaction("responses", "readonly").objectStore("responses").get(ownerKey(owner, path))
    );
    return decode(rec);
  } catch {
    return null;
  }
}

// Age in ms of a cached entry, or Infinity when there isn't one. Used by the
// background warmer to skip what it refreshed recently.
export async function cacheAge(path) {
  const owner = scopeOwner(path);
  if (!owner) return Infinity;
  try {
    const db = await openDB();
    const rec = await reqp(
      db.transaction("responses", "readonly").objectStore("responses").get(ownerKey(owner, path))
    );
    return rec && rec.ts ? Date.now() - rec.ts : Infinity;
  } catch {
    return Infinity;
  }
}

// `text` is the response body as it came off the wire. Storing it, rather than
// the parsed object, is what keeps this off the main thread's bill: IndexedDB
// structured-clones what it is given, SYNCHRONOUSLY, and a 4000-track list is
// 4000 nested objects — 17-24 ms of cloning on every visit to the favourites
// or a big playlist, measured, against a single string copy. Reading pays one
// JSON.parse instead of the clone it replaces.
//
// A page of the library read under a provider id (`/album/2000000`: an older
// build's link, a catalogue card) is also kept under its row's own id, which is
// what every link opens it by now — so what was browsed before still opens
// offline.
const BY_PROVIDER_ID = /^\/(album|artist)\/[0-9]+$/;
export function aliasPath(path, body) {
  const m = BY_PROVIDER_ID.exec(String(path).split("?")[0]);
  if (!m) return null;
  let data = body;
  if (typeof body === "string") {
    try {
      data = JSON.parse(body);
    } catch {
      return null;
    }
  }
  const id = data && data[m[1]] && data[m[1]].id;
  return id ? `/${m[1]}/${id}` : null;
}

export async function cachePut(path, data, text = null) {
  const owner = scopeOwner(path);
  if (!owner) return;
  try {
    const db = await openDB();
    const t = db.transaction("responses", "readwrite");
    const store = t.objectStore("responses");
    const body = typeof text === "string" ? text : JSON.stringify(data ?? null);
    const ts = Date.now();
    store.put({ key: ownerKey(owner, path), owner, path, text: body, ts });
    const alias = aliasPath(path, body);
    if (alias)
      store.put({ key: ownerKey(owner, alias), owner, path: alias, text: body, ts });
    await done(t);
    prune().catch(() => {});
  } catch {
    /* best effort */
  }
}

export async function cacheDelete(path) {
  const owner = scopeOwner(path);
  if (!owner) return;
  try {
    const db = await openDB();
    const t = db.transaction("responses", "readwrite");
    t.objectStore("responses").delete(ownerKey(owner, path));
    await done(t);
  } catch {
    /* best effort */
  }
}

export async function cachePurgeOwner(owner) {
  if (!owner) return;
  try {
    const db = await openDB();
    const rows = await reqp(
      db.transaction("responses", "readonly").objectStore("responses").getAll()
    );
    const prefix = ownerPrefix(owner);
    const doomed = rows.filter(
      (r) => r && (r.owner === owner || String(r.key || "").startsWith(prefix))
    );
    if (!doomed.length) return;
    const t = db.transaction("responses", "readwrite");
    const store = t.objectStore("responses");
    for (const r of doomed) store.delete(r.key);
    await done(t);
  } catch {
    /* best effort */
  }
}

// Keep the cache bounded: drop the oldest entries past MAX_ENTRIES. Guarded by
// a cheap count() so the (heavy) getAll only runs when actually over the cap —
// this runs after every cached GET, so it must cost ~nothing in the usual case.
async function prune() {
  const db = await openDB();
  const count = await reqp(
    db.transaction("responses", "readonly").objectStore("responses").count()
  );
  if (count <= MAX_ENTRIES) return;
  const all = await reqp(
    db.transaction("responses", "readonly").objectStore("responses").getAll()
  );
  const doomed = all
    .sort((a, b) => (a.ts || 0) - (b.ts || 0))
    .slice(0, all.length - MAX_ENTRIES);
  const t = db.transaction("responses", "readwrite");
  const store = t.objectStore("responses");
  for (const r of doomed) store.delete(r.key);
}
