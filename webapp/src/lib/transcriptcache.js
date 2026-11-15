import { REMOTE } from "./remote/mode.js";
import { episodeCard, excerptAround, foldText, pageKey } from "./transcript.js";

export const DB_NAME = "neezer-transcripts";
export const DB_VERSION = 1;
export const STORE = "pages";
export const MAX_PAGES = 50;
export const MAX_BYTES = 5 * 1024 * 1024;
export const MAX_PER_EPISODE = 4;

let dbPromise = null;
let lastStamp = 0;

export function cacheWriteAllowed(remote = REMOTE) {
  return !remote;
}

export function cacheReadAllowed(remote = REMOTE) {
  return !remote;
}

function openDB() {
  if (dbPromise) return dbPromise;
  dbPromise = new Promise((resolve, reject) => {
    const factory = globalThis.indexedDB;
    if (!factory) {
      reject(new Error("indexedDB is unavailable"));
      return;
    }
    const request = factory.open(DB_NAME, DB_VERSION);
    request.onupgradeneeded = () => {
      const db = request.result;
      if (!db.objectStoreNames.contains(STORE)) {
        const store = db.createObjectStore(STORE, { keyPath: "k" });
        if (store && typeof store.createIndex === "function") {
          try {
            store.createIndex("at", "at");
          } catch {}
        }
      }
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  return dbPromise;
}

export function resetTranscriptCache() {
  dbPromise = null;
}

function requestp(request) {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

async function allRecords() {
  const db = await openDB();
  const store = db.transaction(STORE, "readonly").objectStore(STORE);
  const rows = await requestp(store.getAll());
  return Array.isArray(rows) ? rows : [];
}

async function putRecord(record) {
  const db = await openDB();
  const store = db.transaction(STORE, "readwrite").objectStore(STORE);
  await requestp(store.put(record));
}

async function deleteKeys(keys) {
  if (!keys.length) return;
  const db = await openDB();
  const transaction = db.transaction(STORE, "readwrite");
  const store = transaction.objectStore(STORE);
  for (const key of keys) store.delete(key);
  await new Promise((resolve, reject) => {
    transaction.oncomplete = resolve;
    transaction.onerror = () => reject(transaction.error);
    transaction.onabort = () => reject(transaction.error);
  });
}

function measurable(records) {
  return records.reduce((sum, row) => sum + (Number(row.bytes) || 0), 0);
}

async function prune() {
  const rows = await allRecords();
  const keep = new Set();
  const byEpisode = new Map();
  for (const row of rows) {
    const list = byEpisode.get(row.episodeId) || [];
    list.push(row);
    byEpisode.set(row.episodeId, list);
  }
  for (const list of byEpisode.values()) {
    list.sort((a, b) => (Number(b.at) || 0) - (Number(a.at) || 0));
    for (const row of list.slice(0, MAX_PER_EPISODE)) keep.add(row.k);
  }
  const kept = rows.filter((row) => keep.has(row.k));
  kept.sort((a, b) => (Number(b.at) || 0) - (Number(a.at) || 0));
  let pages = 0;
  let bytes = 0;
  const survivors = new Set();
  for (const row of kept) {
    const size = Number(row.bytes) || 0;
    if (pages >= MAX_PAGES) break;
    if (bytes + size > MAX_BYTES && survivors.size > 0) break;
    survivors.add(row.k);
    pages++;
    bytes += size;
  }
  const stale = rows.filter((row) => !survivors.has(row.k)).map((row) => row.k);
  await deleteKeys(stale);
}

export async function readPage(episodeId, limit, offset) {
  if (!cacheReadAllowed()) return null;
  try {
    const id = String(episodeId);
    const db = await openDB();
    const store = db.transaction(STORE, "readonly").objectStore(STORE);
    const row = await requestp(store.get(pageKey(id, limit, offset)));
    return row && Array.isArray(row.segments) ? row : null;
  } catch {
    return null;
  }
}

export async function readAt(episodeId, limit, seconds) {
  if (!cacheReadAllowed()) return null;
  try {
    const id = String(episodeId);
    const at = Number(seconds);
    const rows = (await allRecords())
      .filter(
        (row) => row.episodeId === id && Number(row.limit) === Number(limit),
      )
      .sort((a, b) => Number(a.offset) - Number(b.offset));
    if (!rows.length) return null;
    if (!Number.isFinite(at)) return rows[0];
    let found = null;
    for (const row of rows) {
      const first = row.segments && row.segments[0];
      if (first && Number(first.start) <= at) found = row;
    }
    return found || rows[0];
  } catch {
    return null;
  }
}

export async function firstFor(episodeId) {
  if (!cacheReadAllowed()) return null;
  try {
    const id = String(episodeId);
    const rows = (await allRecords())
      .filter((row) => row.episodeId === id)
      .sort((a, b) => Number(a.offset) - Number(b.offset));
    return rows[0] || null;
  } catch {
    return null;
  }
}

export async function writePage(payload, episode) {
  if (!cacheWriteAllowed()) return false;
  if (!payload || payload.status !== "ready") return false;
  const segments = Array.isArray(payload.segments) ? payload.segments : [];
  if (!segments.length) return false;
  const episodeId = String(payload.episode_id || (episode && episode.id) || "");
  if (!episodeId) return false;
  const limit = Math.max(1, Math.floor(Number(payload.limit) || segments.length));
  const offset = Math.max(0, Math.floor(Number(payload.offset) || 0));
  const clean = segments.map((segment) => ({
    start: Number(segment.start) || 0,
    end: Number(segment.end) || 0,
    text: String(segment.text || ""),
  }));
  lastStamp = Math.max(Date.now(), lastStamp + 1);
  const record = {
    k: pageKey(episodeId, limit, offset),
    episodeId,
    offset,
    limit,
    total: Math.max(0, Math.floor(Number(payload.total) || clean.length)),
    at: lastStamp,
    language: payload.language || null,
    duration: Number(payload.duration_ms) ? Math.round(Number(payload.duration_ms) / 1000) : 0,
    engine: payload.engine || null,
    model: payload.model || null,
    segments: clean,
    episode: episodeCard(episode),
    bytes: 0,
  };
  record.bytes = JSON.stringify(record).length;
  try {
    let rows = await allRecords();
    const changed = rows.some(
      (row) => row.episodeId === episodeId && Number(row.total) !== record.total,
    );
    if (changed) {
      await deleteKeys(rows.filter((row) => row.episodeId === episodeId).map((row) => row.k));
      rows = rows.filter((row) => row.episodeId !== episodeId);
    }
    await putRecord(record);
    await prune();
    return true;
  } catch {
    return false;
  }
}

export async function forget(episodeId) {
  if (!cacheWriteAllowed()) return false;
  try {
    const id = String(episodeId);
    const rows = (await allRecords()).filter((row) => row.episodeId === id);
    await deleteKeys(rows.map((row) => row.k));
    return true;
  } catch {
    return false;
  }
}

export async function clearTranscriptCache() {
  try {
    const db = await openDB();
    const store = db.transaction(STORE, "readwrite").objectStore(STORE);
    await requestp(store.clear());
    return true;
  } catch {
    return false;
  }
}

export async function searchCached(query, limit = 20) {
  if (!cacheReadAllowed()) return { hits: [], total: 0, local: true };
  const words = foldText(query).split(/\s+/).filter(Boolean);
  if (!words.length) return { hits: [], total: 0, local: true };
  try {
    const rows = await allRecords();
    const seen = new Set();
    const hits = [];
    for (const row of rows) {
      for (const segment of row.segments || []) {
        const folded = foldText(segment.text);
        if (!words.every((word) => folded.includes(word))) continue;
        const key = `${row.episodeId}:${segment.start}`;
        if (seen.has(key)) continue;
        seen.add(key);
        hits.push({
          episode_id: row.episodeId,
          episode: row.episode || null,
          idx: 0,
          start: Number(segment.start) || 0,
          end: Number(segment.end) || 0,
          text: excerptAround(segment.text, query, 240),
          channel_id: row.episode && row.episode.channel_id,
          channel_title: row.episode && row.episode.channel_title,
        });
      }
    }
    hits.sort((a, b) => {
      const left = Number(a.episode && a.episode.published) || 0;
      const right = Number(b.episode && b.episode.published) || 0;
      return right - left || a.start - b.start;
    });
    return {
      hits: hits.slice(0, Math.max(1, Math.floor(Number(limit) || 20))),
      total: hits.length,
      local: true,
    };
  } catch {
    return { hits: [], total: 0, local: true };
  }
}

export async function cacheStats() {
  if (!cacheReadAllowed()) return { pages: 0, bytes: 0, episodes: 0 };
  try {
    const rows = await allRecords();
    return {
      pages: rows.length,
      bytes: measurable(rows),
      episodes: new Set(rows.map((row) => row.episodeId)).size,
    };
  } catch {
    return { pages: 0, bytes: 0, episodes: 0 };
  }
}
