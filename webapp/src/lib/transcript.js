import { get, writable } from "svelte/store";
import { api } from "./api.js";

export const LANGUAGES = ["auto", "fr", "en", "es", "de", "it", "pt"];
export const ACTIVE_STATUSES = ["queued", "leased", "running"];
export const TERMINAL_STATUSES = ["ready", "error", "stopped"];

const LANGUAGE_LABELS = {
  auto: "Détection automatique",
  fr: "Français",
  en: "Anglais",
  es: "Espagnol",
  de: "Allemand",
  it: "Italien",
  pt: "Portugais",
};

export function foldText(value) {
  return String(value == null ? "" : value)
    .normalize("NFD")
    .toLowerCase()
    .replace(/\p{M}+/gu, "");
}

export function languageLabel(code) {
  if (code == null || code === "") return "";
  const key = String(code).toLowerCase();
  if (LANGUAGE_LABELS[key]) return LANGUAGE_LABELS[key];
  return String(code).toUpperCase();
}

export function reasonText(reason) {
  switch (reason) {
    case "disabled":
      return "La transcription est désactivée sur ce serveur.";
    case "engine_missing":
      return "Le moteur de transcription n'est pas installé sur le serveur.";
    case "model_missing":
      return "Le modèle de transcription est introuvable sur le serveur.";
    case "model_invalid":
      return "Le fichier de modèle n'est pas un modèle whisper.cpp valide.";
    default:
      return "La transcription n'est pas configurée sur ce serveur.";
  }
}

export function isActiveStatus(status) {
  return ACTIVE_STATUSES.includes(status);
}

export function isTerminalStatus(status) {
  return TERMINAL_STATUSES.includes(status);
}

export function segmentAt(segments, seconds) {
  if (!Array.isArray(segments) || !segments.length) return -1;
  const at = Number(seconds);
  if (!Number.isFinite(at)) return -1;
  let low = 0;
  let high = segments.length - 1;
  let found = -1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    const start = Number(segments[middle] && segments[middle].start);
    if (Number.isFinite(start) && start <= at) {
      found = middle;
      low = middle + 1;
    } else {
      high = middle - 1;
    }
  }
  return found;
}

export function pageInfo(offset, limit, total) {
  const step = Math.max(1, Math.floor(Number(limit) || 1));
  const count = Math.max(0, Math.floor(Number(total) || 0));
  const pages = Math.max(1, Math.ceil(count / step));
  const raw = Math.floor(Math.max(0, Number(offset) || 0) / step) + 1;
  const page = Math.min(pages, Math.max(1, raw));
  const from = count === 0 ? 0 : (page - 1) * step;
  const to = Math.min(count, from + step);
  return { page, pages, from, to };
}

export function batchOptions(eligible) {
  const count = Math.max(0, Math.floor(Number(eligible) || 0));
  if (count === 0) return [];
  const out = [Math.min(5, count)];
  if (count >= 10) out.push(10);
  if (count >= 20) out.push(20);
  return out;
}

export function highlightParts(text, query) {
  const source = String(text == null ? "" : text);
  const words = String(query == null ? "" : query)
    .trim()
    .split(/\s+/)
    .filter(Boolean);
  if (!source || !words.length) return [{ t: source, mark: false }];
  const lower = source.toLowerCase();
  const occurrences = (needle) => {
    const found = [];
    if (!needle) return found;
    let from = 0;
    while (from <= lower.length - needle.length) {
      const at = lower.indexOf(needle, from);
      if (at < 0) break;
      found.push([at, at + needle.length]);
      from = at + needle.length;
    }
    return found;
  };
  let ranges = occurrences(String(query == null ? "" : query).trim().toLowerCase());
  if (!ranges.length) {
    for (const word of words) ranges.push(...occurrences(word.toLowerCase()));
  }
  if (!ranges.length) return [{ t: source, mark: false }];
  ranges.sort((a, b) => a[0] - b[0] || b[1] - a[1]);
  const merged = [];
  for (const range of ranges) {
    const last = merged[merged.length - 1];
    if (last && range[0] < last[1]) last[1] = Math.max(last[1], range[1]);
    else merged.push([range[0], range[1]]);
  }
  const parts = [];
  let at = 0;
  for (const [start, end] of merged) {
    if (start > at) parts.push({ t: source.slice(at, start), mark: false });
    parts.push({ t: source.slice(start, end), mark: true });
    at = end;
  }
  if (at < source.length) parts.push({ t: source.slice(at), mark: false });
  return parts;
}

export function excerptAround(text, query, max = 240) {
  const value = String(text == null ? "" : text);
  const size = Math.max(8, Math.floor(Number(max) || 240));
  if (value.length <= size) return value;
  const words = String(query == null ? "" : query)
    .trim()
    .split(/\s+/)
    .filter(Boolean);
  const lower = value.toLowerCase();
  let found = -1;
  for (const word of words) {
    const at = lower.indexOf(word.toLowerCase());
    if (at >= 0 && (found < 0 || at < found)) found = at;
  }
  if (found < 0) return value.slice(0, size - 1) + "…";
  let start = Math.max(0, found - Math.floor(size / 2));
  let end = Math.min(value.length, start + size);
  start = Math.max(0, end - size);
  const ellipses = (start > 0 ? 1 : 0) + (end < value.length ? 1 : 0);
  if (ellipses) {
    const room = size - ellipses;
    end = Math.min(value.length, start + room);
    start = Math.max(0, end - room);
  }
  return (
    (start > 0 ? "…" : "") +
    value.slice(start, end) +
    (end < value.length ? "…" : "")
  );
}

export function episodeCard(episode) {
  if (!episode) return null;
  return {
    id: String(episode.id == null ? "" : episode.id),
    title: episode.title || "",
    cover: episode.cover || "",
    channel_id: episode.channel_id ? String(episode.channel_id) : "",
    channel_title: episode.album?.title || episode.artist?.name || episode.channel_title || "",
    published: Number(episode.published) || 0,
    duration: Number(episode.duration) || 0,
  };
}

export function hitEpisode(hit) {
  const card = (hit && hit.episode) || {};
  const id = String((hit && hit.episode_id) || card.id || "");
  if (!id) return null;
  const channelId = String((hit && hit.channel_id) || card.channel_id || "");
  const channelTitle = (hit && hit.channel_title) || card.channel_title || "";
  const cover = card.cover || (hit && hit.cover) || "";
  return {
    id,
    deezer_id: id,
    podcast: true,
    title: card.title || (hit && hit.episode_title) || "",
    duration: Number(card.duration) || 0,
    published: Number(card.published) || 0,
    channel_id: channelId,
    artist: { deezer_id: channelId, name: channelTitle },
    album: { deezer_id: channelId, title: channelTitle, cover },
    cover,
    archived: true,
    chapter_count: 0,
    transcript: { status: "ready", progress: 1 },
  };
}

export function pageKey(episodeId, limit, offset) {
  return `${String(episodeId)}:${Number(limit) || 0}:${Number(offset) || 0}`;
}

export const transcribeSettings = writable({
  state: "idle",
  data: null,
  error: null,
  supported: true,
});

let settingsPromise = null;

export async function loadTranscribeSettings(force = false) {
  const current = get(transcribeSettings);
  if (!force) {
    if (current.state === "ready") return current.data;
    if (settingsPromise) return settingsPromise;
  }
  transcribeSettings.set({
    state: "loading",
    data: current.data,
    error: null,
    supported: current.supported,
  });
  settingsPromise = api
    .transcribeSettings(force ? { probe: true } : {})
    .then((data) => {
      transcribeSettings.set({ state: "ready", data, error: null, supported: true });
      return data;
    })
    .catch((error) => {
      const supported = !(error && (error.status === 404 || error.status === 403));
      transcribeSettings.set({
        state: "error",
        data: current.data,
        error,
        supported,
      });
      return null;
    })
    .finally(() => {
      settingsPromise = null;
    });
  return settingsPromise;
}

export function resetTranscribeSettings() {
  settingsPromise = null;
  transcribeSettings.set({ state: "idle", data: null, error: null, supported: true });
}

export const transcribeStatus = writable({ state: "idle", data: null, error: null });

let statusPromise = null;

export async function loadTranscribeStatus(force = false) {
  const current = get(transcribeStatus);
  if (!force) {
    if (current.state === "ready") return current.data;
    if (statusPromise) return statusPromise;
  }
  transcribeStatus.set({ ...current, state: "loading", error: null });
  statusPromise = api
    .transcribeStatus()
    .then((data) => {
      transcribeStatus.set({ state: "ready", data, error: null });
      return data;
    })
    .catch((error) => {
      transcribeStatus.set({ state: "error", data: current.data, error });
      return null;
    })
    .finally(() => {
      statusPromise = null;
    });
  return statusPromise;
}

export function resetTranscribeStatus() {
  statusPromise = null;
  transcribeStatus.set({ state: "idle", data: null, error: null });
}

export const panelRequest = writable(null);
let requestSeq = 0;

export function requestEpisodePanel(episodeId, seg = "transcript", position = null) {
  const id = episodeId == null ? "" : String(episodeId);
  if (!id) return;
  panelRequest.set({
    episodeId: id,
    seg,
    position: Number.isFinite(Number(position)) ? Number(position) : null,
    at: Date.now(),
    seq: ++requestSeq,
  });
}

export function takePanelRequest() {
  let found = null;
  panelRequest.update((request) => {
    if (request && Date.now() - request.at <= 3000) found = request;
    return null;
  });
  return found;
}

export const transcriptNote = writable(null);
let noteSeq = 0;

export function noteTranscript(episodeId, transcript) {
  const id = episodeId == null ? "" : String(episodeId);
  if (!id) return;
  transcriptNote.set({
    episodeId: id,
    transcript: transcript || null,
    at: Date.now(),
    seq: ++noteSeq,
  });
}
