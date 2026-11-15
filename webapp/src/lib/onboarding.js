export const PAGE_FIRST = 36;
export const PAGE_NEXT = 24;
export const SEARCH_MIN_CHARS = 2;
export const SEARCH_LIMIT = 25;
export const SIMILAR_PER_LIKE = 2;
export const SIMILAR_TOTAL_MAX = 12;
export const PASS_ATTEMPTS = 2;
export const CACHE_ITEMS_MAX = 120;
export const CACHE_TTL = 30 * 60 * 1000;
export const STORE_PREFIX = "nsOnboarding.";
export const DEFAULT_DIVERSITY = { window: 12, max_same_genre: 4, max_streak: 2 };

const NUMBER = new Intl.NumberFormat("fr-FR");

export function formatNumber(value) {
  const number = Number(value);
  return Number.isFinite(number) ? NUMBER.format(number) : "0";
}

export function plural(count, one, many) {
  return Number(count) > 1 ? many : one;
}

export function idOf(artist) {
  if (artist == null) return "";
  if (typeof artist !== "object") return String(artist);
  return String(artist.deezer_id || artist.id || artist.entity_id || "");
}

export function genreOf(artist) {
  if (!artist || typeof artist !== "object") return null;
  const list = Array.isArray(artist.genres) ? artist.genres : [];
  for (const value of list) {
    const genre = String(value == null ? "" : value).trim().toLowerCase();
    if (genre) return genre;
  }
  const single = String(artist.genre == null ? "" : artist.genre).trim().toLowerCase();
  return single || null;
}

function isInserted(item) {
  return !!item && item.via === "related";
}

export function countInserted(items) {
  return (Array.isArray(items) ? items : []).filter(isInserted).length;
}

export function budgetLeft(items, budget = SIMILAR_TOTAL_MAX) {
  return Math.max(0, Math.max(0, Number(budget) || 0) - countInserted(items));
}

function likedSet(value) {
  if (value instanceof Set) return value;
  if (Array.isArray(value)) return new Set(value.map(String));
  return new Set();
}

/** Places at most `perLike` related artists right after `likedIndex`, depth 1,
 *  never a duplicate of the list or a liked artist, never past `budget`
 *  inserted tiles in the list, and alternating main genres when it can. */
export function insertSimilar(items, likedIndex, similar, options = {}) {
  const list = Array.isArray(items) ? items : [];
  const index = Number.isInteger(likedIndex) ? likedIndex : -1;
  if (index < 0 || index >= list.length) return { items: list, inserted: 0 };
  const perLike = Math.max(0, options.perLike == null ? SIMILAR_PER_LIKE : options.perLike);
  const budget = Math.max(0, options.budget == null ? SIMILAR_TOTAL_MAX : options.budget);
  const diversity = { ...DEFAULT_DIVERSITY, ...(options.diversity || {}) };
  const room = Math.min(perLike, Math.max(0, budget - countInserted(list)));
  if (!room) return { items: list, inserted: 0 };
  const parent = idOf(list[index]);
  if (!parent) return { items: list, inserted: 0 };
  const liked = likedSet(options.liked);
  const taken = new Set(list.map(idOf).filter(Boolean));
  const candidates = [];
  for (const artist of Array.isArray(similar) ? similar : []) {
    const id = idOf(artist);
    if (!id || id === parent || taken.has(id) || liked.has(id)) continue;
    if (!artist || typeof artist !== "object" || artist.liked || artist.placeholder) continue;
    taken.add(id);
    candidates.push(artist);
  }
  if (!candidates.length) return { items: list, inserted: 0 };
  const windowSize = Math.max(1, Number(diversity.window) || 1);
  const cap = Math.max(1, Number(diversity.max_same_genre) || 1);
  const from = Math.max(0, index - windowSize + 1);
  const to = Math.min(list.length, index + windowSize);
  const counts = new Map();
  for (const artist of list.slice(from, to)) {
    const genre = genreOf(artist);
    if (genre) counts.set(genre, (counts.get(genre) || 0) + 1);
  }
  const fits = (artist) => {
    const genre = genreOf(artist);
    if (!genre) return true;
    return (counts.get(genre) || 0) < cap;
  };
  const picked = [];
  const deferred = [];
  for (const artist of candidates) {
    if (picked.length >= room) break;
    if (!fits(artist)) continue;
    const previous = picked.length ? genreOf(picked[picked.length - 1]) : null;
    if (previous && genreOf(artist) === previous) {
      deferred.push(artist);
      continue;
    }
    picked.push(artist);
    const genre = genreOf(artist);
    if (genre) counts.set(genre, (counts.get(genre) || 0) + 1);
  }
  for (const artist of deferred) {
    if (picked.length >= room) break;
    if (!fits(artist)) continue;
    picked.push(artist);
    const genre = genreOf(artist);
    if (genre) counts.set(genre, (counts.get(genre) || 0) + 1);
  }
  if (!picked.length) return { items: list, inserted: 0 };
  const block = picked.map((artist) => ({ ...artist, via: "related", parent }));
  return {
    items: [...list.slice(0, index + 1), ...block, ...list.slice(index + 1)],
    inserted: block.length,
  };
}

export function removeSimilar(items, parentId, liked = null) {
  const parent = String(parentId == null ? "" : parentId);
  const kept = likedSet(liked);
  const out = [];
  for (const item of Array.isArray(items) ? items : []) {
    if (!isInserted(item) || String(item.parent == null ? "" : item.parent) !== parent) {
      out.push(item);
      continue;
    }
    if (kept.has(idOf(item)) || item.liked) out.push({ ...item, via: "grid", parent: null });
  }
  return out;
}

export function reconcilePage(items, page) {
  const list = Array.isArray(items) ? [...items] : [];
  const places = new Map();
  list.forEach((item, index) => places.set(idOf(item), index));
  const seen = new Set();
  for (const fresh of Array.isArray(page) ? page : []) {
    const id = idOf(fresh);
    if (!id || seen.has(id)) continue;
    seen.add(id);
    if (places.has(id)) {
      const index = places.get(id);
      const current = list[index];
      list[index] = { ...current, ...fresh, via: current.via, parent: current.parent };
    } else {
      places.set(id, list.length);
      list.push(fresh);
    }
  }
  return list;
}

export function cacheList(items, max = CACHE_ITEMS_MAX) {
  const list = (Array.isArray(items) ? items : []).filter((item) => item && !item.placeholder);
  return max > 0 && list.length > max ? list.slice(0, max) : list;
}

export function normalizeQuery(raw) {
  return String(raw == null ? "" : raw)
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 80);
}

export function searchProblem(raw) {
  return normalizeQuery(raw).length >= SEARCH_MIN_CHARS ? null : "too_short";
}

export function artistCountLabel(count) {
  const n = Math.max(0, Number(count) || 0);
  return `${formatNumber(n)} ${plural(n, "artiste", "artistes")}`;
}

export function progressHint(count) {
  const n = Math.max(0, Number(count) || 0);
  if (n < 1) return "Choisissez-en au moins un.";
  if (n < 3) return `Encore ${3 - n} pour un bon départ.`;
  if (n < 5) return "C'est un bon départ.";
  return "Vous pouvez continuer ou terminer.";
}

export function finishBlocked({ liked = 0, pending = 0, queued = 0, online = true, min = 1 } = {}) {
  if ((Number(liked) || 0) < min) return "min";
  if (!online) return "offline";
  if ((Number(pending) || 0) > 0 || (Number(queued) || 0) > 0) return "pending";
  return null;
}

export function finishMessage(reason) {
  if (reason === "min") return "Choisissez au moins un artiste pour terminer.";
  if (reason === "offline") return "Hors ligne : reconnectez-vous pour terminer.";
  if (reason === "pending") return "Enregistrement de vos choix…";
  return "";
}

export function feedCause(code) {
  switch (String(code || "")) {
    case "deezer-unreachable":
      return "Deezer ne répond pas.";
    case "disabled":
      return "Le catalogue Deezer est désactivé sur ce serveur.";
    case "held":
      return "Le catalogue est en pause après plusieurs échecs.";
    case "absent":
      return "Aucun catalogue n'est configuré sur ce serveur.";
    case "busy":
      return "Le serveur prépare déjà la sélection.";
    case "error":
      return "Le serveur n'a pas pu préparer la sélection.";
    default:
      return "La sélection n'a pas pu être préparée.";
  }
}

export function canPassArtists({ attempts = 0, feed = null, count = 0, min = 1 } = {}) {
  if ((Number(count) || 0) >= min) return false;
  if ((Number(attempts) || 0) < PASS_ATTEMPTS) return false;
  return !!(feed && feed.state === "unavailable" && !feed.building);
}

export function stepFor(onboarding) {
  return onboarding === "artists" || onboarding === "account" ? onboarding : "done";
}

export function pathForStep(step) {
  if (step === "artists") return "/onboarding/artists";
  if (step === "account") return "/onboarding/accounts";
  return "/";
}

export function stepFromPath(path) {
  const value = String(path || "");
  if (value === "/onboarding" || value === "/onboarding/") return "root";
  if (!value.startsWith("/onboarding/")) return null;
  const leaf = value.slice("/onboarding/".length).split("/")[0];
  if (leaf === "artists") return "artists";
  if (leaf === "accounts") return "accounts";
  return "unknown";
}

export function onboardingTarget(onboarding, path) {
  const step = stepFor(onboarding);
  const leaf = stepFromPath(path);
  if (step === "done") return leaf ? "/" : null;
  const wanted = pathForStep(step);
  if (!leaf) return wanted;
  if (leaf === "root" || leaf === "unknown") return wanted;
  if (step === "artists") return leaf === "artists" ? null : wanted;
  return leaf === "artists" || leaf === "accounts" ? null : wanted;
}

export function isImportRunning(job) {
  return !!job && (job.state === "reading" || job.state === "matching" || job.state === "saving");
}

export function importProgress(job) {
  if (!job || !job.state) return { label: "", percent: null, busy: false, failed: false };
  const total = Math.max(0, Number(job.total) || 0);
  const done = Math.max(0, Number(job.done) || 0);
  const matched = Math.max(0, Number(job.matched) || 0);
  const percent = total > 0 ? Math.min(100, Math.round((100 * done) / total)) : null;
  if (job.state === "reading") {
    return { label: "Lecture de la playlist…", percent: null, busy: true, failed: false };
  }
  if (job.state === "matching") {
    return {
      label:
        total > 0
          ? `Recherche des titres : ${formatNumber(done)} sur ${formatNumber(total)}`
          : "Recherche des titres…",
      percent,
      busy: true,
      failed: false,
    };
  }
  if (job.state === "saving") {
    return { label: "Ajout à votre bibliothèque…", percent: null, busy: true, failed: false };
  }
  if (job.state === "saved") {
    return {
      label: `Ajouté : ${formatNumber(matched)} ${plural(matched, "titre", "titres")}.`,
      percent: 100,
      busy: false,
      failed: false,
      done: true,
      saved: true,
    };
  }
  if (job.state === "error") {
    return {
      label: `L'import a échoué : ${job.error || "cause inconnue"}.`,
      percent: null,
      busy: false,
      failed: true,
    };
  }
  const found = ` ${plural(matched, "titre trouvé", "titres trouvés")} sur ${formatNumber(total)}.`;
  return {
    label: `${formatNumber(matched)}${found}`,
    percent: 100,
    busy: false,
    failed: false,
    done: true,
  };
}

export function unmatchedItems(job, max = 50) {
  const items = job && Array.isArray(job.items) ? job.items : [];
  return items.filter((item) => !item.track).slice(0, Math.max(0, max));
}

export function unmatchedCount(job) {
  const items = job && Array.isArray(job.items) ? job.items : [];
  return items.filter((item) => !item.track).length;
}

export function importNote(job) {
  if (!job || job.state !== "done" || !job.error) return "";
  return String(job.error);
}

export function accountSummary(account) {
  if (!account) return { text: "Non relié", tone: "idle" };
  const running = account.running;
  if (running && running.running) {
    if (running.phase === "connect") return { text: "Connexion au compte…", tone: "busy" };
    if (running.phase === "list") return { text: "Lecture de la liste…", tone: "busy" };
    if (running.phase === "import" && running.total) {
      return {
        text: `Import en cours : ${formatNumber(running.done || 0)} sur ${formatNumber(running.total)}`,
        tone: "busy",
      };
    }
    return { text: "Import en cours…", tone: "busy" };
  }
  if (account.status === "auth") {
    return { text: "Le compte refuse la connexion : reliez-le à nouveau.", tone: "error" };
  }
  if (account.status === "network") {
    return { text: "Le fournisseur ne répond pas : l'import reprendra plus tard.", tone: "error" };
  }
  const last = account.last;
  if (last && last.reason === "auth") {
    return { text: "Le dernier import a échoué : le compte refuse la connexion.", tone: "error" };
  }
  if (last && last.reason === "network") {
    return { text: "Le dernier import a échoué : le fournisseur ne répondait pas.", tone: "error" };
  }
  const added = (last && last.added) || {};
  const artists = Math.max(0, Number(added.artists) || 0);
  const tracks = Math.max(0, Number(added.tracks) || 0);
  const parts = [];
  if (artists) parts.push(`${formatNumber(artists)} ${plural(artists, "artiste", "artistes")}`);
  if (tracks) parts.push(`${formatNumber(tracks)} ${plural(tracks, "titre", "titres")}`);
  if (parts.length) return { text: `Dernier import : ${parts.join(", ")}.`, tone: "ok" };
  if (last) return { text: "Dernier import : rien de nouveau.", tone: "ok" };
  return { text: "Compte relié", tone: "ok" };
}

export function linkFailure(error) {
  const body = (error && error.body) || {};
  if (body.error) return String(body.error);
  if (error && error.status === 429) return "Trop de tentatives. Réessayez dans quelques minutes.";
  if (error && error.offline) return "Vous êtes hors ligne : la liaison attend le retour du réseau.";
  if (error && error.status === 503) return "Le fournisseur ne répond pas. Réessayez dans un instant.";
  return "La liaison a échoué. Réessayez dans un instant.";
}

export function importFailure(error) {
  const body = (error && error.body) || {};
  if (body.error) return String(body.error);
  if (error && error.status === 429) return "Un import est déjà en cours. Attendez qu'il finisse.";
  if (error && error.offline) return "Vous êtes hors ligne : l'import attend le retour du réseau.";
  return "L'import n'a pas pu démarrer. Réessayez dans un instant.";
}

export function saveFailure(error) {
  const body = (error && error.body) || {};
  if (body.error) return String(body.error);
  if (error && error.status === 400) return "Aucun titre de cet import n'est disponible.";
  if (error && error.offline) return "Vous êtes hors ligne : l'ajout attend le retour du réseau.";
  return "L'ajout à votre bibliothèque a échoué. Réessayez.";
}

function storageOf(storage) {
  if (storage) return storage;
  try {
    return globalThis.sessionStorage || null;
  } catch {
    return null;
  }
}

export function storeKey(name) {
  return STORE_PREFIX + String(name);
}

export function readStore(name, { storage = null, maxAge = CACHE_TTL, now = Date.now() } = {}) {
  const box = storageOf(storage);
  if (!box) return null;
  try {
    const raw = JSON.parse(box.getItem(storeKey(name)) || "null");
    if (!raw || raw.v !== 1 || typeof raw.at !== "number") return null;
    if (maxAge != null && now - raw.at > maxAge) return null;
    return raw.value == null ? null : raw.value;
  } catch {
    return null;
  }
}

export function writeStore(name, value, { storage = null, now = Date.now() } = {}) {
  const box = storageOf(storage);
  if (!box) return false;
  try {
    box.setItem(storeKey(name), JSON.stringify({ v: 1, at: now, value }));
    return true;
  } catch {
    return false;
  }
}

export function dropStore(name, { storage = null } = {}) {
  const box = storageOf(storage);
  if (!box) return;
  try {
    box.removeItem(storeKey(name));
  } catch {
  }
}

export function enqueueLike(queue, id, liked) {
  const key = String(id == null ? "" : id);
  if (!key) return Array.isArray(queue) ? queue : [];
  const list = (Array.isArray(queue) ? queue : []).filter((entry) => String(entry.id) !== key);
  list.push({ id: key, liked: !!liked });
  return list;
}
