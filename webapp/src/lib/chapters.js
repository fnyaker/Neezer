import { get, writable } from "svelte/store";
import { api } from "./api.js";

/**
 * Per-episode chapter state, keyed by episode id as a STRING:
 * { state: "idle"|"loading"|"ready"|"error", chapters, source, archived,
 *   scanned, duration, cached, offline, error, at }
 */
export const chaptersByEpisode = writable({});

const MAX_ENTRIES = 40;
const inflight = new Map();

function blank() {
  return {
    state: "idle",
    chapters: [],
    source: null,
    archived: null,
    scanned: false,
    duration: 0,
    cached: false,
    offline: false,
    error: null,
    at: 0,
  };
}

function integer(value) {
  if (value == null || value === "") return null;
  const n = Number(value);
  return Number.isFinite(n) ? Math.round(n) : null;
}

export function normalizeChapters(raw) {
  if (!Array.isArray(raw)) return [];
  const out = [];
  for (const item of raw) {
    if (!item || typeof item !== "object") continue;
    const start = integer(item.start);
    if (start === null || start < 0) continue;
    const endRaw = integer(item.end);
    const end = endRaw !== null && endRaw >= start ? endRaw : null;
    const title = typeof item.title === "string" ? item.title.trim() : "";
    out.push({ start, end, title });
  }
  out.sort((a, b) => a.start - b.start);
  const seen = new Set();
  return out.filter((c) => {
    if (seen.has(c.start)) return false;
    seen.add(c.start);
    return true;
  });
}

function put(id, patch) {
  chaptersByEpisode.update((all) => {
    const next = { ...all, [id]: { ...(all[id] || blank()), ...patch, at: Date.now() } };
    const keys = Object.keys(next);
    if (keys.length > MAX_ENTRIES) {
      keys.sort((a, b) => (next[a].at || 0) - (next[b].at || 0));
      for (const key of keys.slice(0, keys.length - MAX_ENTRIES)) {
        if (key !== id) delete next[key];
      }
    }
    return next;
  });
}

function apply(id, data, cached) {
  const chapters = normalizeChapters(data && data.chapters);
  put(id, {
    state: "ready",
    chapters,
    source: data && typeof data.source === "string" ? data.source : null,
    archived: data && typeof data.archived === "boolean" ? data.archived : null,
    scanned: !!(data && data.scanned),
    duration: Math.max(0, integer(data && data.duration) ?? 0),
    cached: !!cached,
    offline: false,
    error: null,
  });
}

export function loadChapters(episodeId, { force = false } = {}) {
  const id = episodeId == null ? "" : String(episodeId);
  if (!id) return Promise.resolve(null);
  const current = get(chaptersByEpisode)[id];
  if (!force && current && current.state === "ready") return Promise.resolve(current);
  if (inflight.has(id)) return inflight.get(id);
  put(id, { state: "loading", error: null, offline: false });
  const path = "/podcast/episode/" + encodeURIComponent(id) + "/chapters";
  const p = api
    .swr(path, (data, stale) => {
      if (stale) apply(id, data, true);
    })
    .then((data) => {
      if (data) apply(id, data, false);
      return get(chaptersByEpisode)[id] || null;
    })
    .catch((err) => {
      const entry = get(chaptersByEpisode)[id] || blank();
      if (entry.chapters.length) {
        put(id, { state: "ready", cached: true, offline: true, error: err });
      } else {
        put(id, { state: "error", chapters: [], error: err });
      }
      return get(chaptersByEpisode)[id] || null;
    })
    .finally(() => inflight.delete(id));
  inflight.set(id, p);
  return p;
}

export function chapterAt(chapters, seconds) {
  const list = chapters || [];
  if (!list.length) return null;
  const t = Number(seconds) || 0;
  if (t < list[0].start) return null;
  let lo = 0;
  let hi = list.length - 1;
  let best = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (list[mid].start <= t) {
      best = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  if (best < 0) return null;
  const chapter = list[best];
  if (chapter.end != null && t >= chapter.end) return null;
  return { index: best, chapter };
}

export function nextChapter(chapters, seconds) {
  const list = chapters || [];
  const t = Number(seconds) || 0;
  let lo = 0;
  let hi = list.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (list[mid].start <= t) lo = mid + 1;
    else hi = mid;
  }
  if (lo >= list.length) return null;
  return { index: lo, chapter: list[lo] };
}

export function chapterEnd(chapters, index, fallback = 0) {
  const list = chapters || [];
  const chapter = list[index];
  if (!chapter) return 0;
  if (chapter.end != null) return chapter.end;
  const after = list[index + 1];
  if (after) return after.start;
  const end = Number(fallback) || 0;
  return end > chapter.start ? end : chapter.start;
}

export function chapterLabel(chapters, index) {
  const chapter = (chapters || [])[index];
  if (!chapter) return "";
  const base = "Chapitre " + (index + 1);
  return chapter.title ? base + " · " + chapter.title : base;
}

export function chapterCountLabel(count) {
  const n = Math.max(0, Math.floor(Number(count) || 0));
  return n > 1 ? n + " chapitres" : n + " chapitre";
}

export function podcastAbout(episode) {
  return (
    !!episode?.podcast &&
    (!!String(episode.description || "").trim() ||
      (episode.chapter_count || 0) > 0 ||
      !!episode.archived)
  );
}

export function dateLabel(ts) {
  const n = Number(ts);
  if (!Number.isFinite(n) || n <= 0) return "";
  try {
    return new Date(n * 1000).toLocaleDateString("fr-FR", {
      day: "numeric",
      month: "short",
      year: "numeric",
    });
  } catch {
    return "";
  }
}
