// Higher-level actions wiring the API to the stores (favorites, radio, toasts).
// Kept separate from api.js to avoid an import cycle (api.js imports stores).

import { migrateKey, trackKey, trackRef } from "./trackkey.js";
import { get, writable } from "svelte/store";

import { CAN_KEEP } from "./remote/mode.js";
import { api } from "./api.js";
import { onOwnerChange } from "./account.js";
import {
  favorites,
  disliked,
  favTracks,
  player,
  playlists,
  toasts,
  syncing,
  downloadQuality,
  lastPlaylist,
  playlistFolders,
  openFolders,
  notices,
} from "./stores.js";
import { downloadTrack, removeTrack, isDownloaded } from "./offline.js";
import { trackEntityRef } from "./entity.js";
import { reconcileList } from "./reconcile.js";
import { cacheAge } from "./apicache.js";
import { online } from "./net.js";
import { containerIds, idsOf, orderContainer, orderFolders } from "./playlistfolders.js";
import { createTrackMenus } from "./trackmenu.js";

// The contextual menus' spec lives in lib/trackmenu.js (one place, every
// surface); these two re-exports keep the existing importers — routes that
// build a favourite's metadata, and anything that named the download qualities.
export { DL_QUALITIES, entityMeta } from "./trackmenu.js";

// Run a manual Deezer sync end-to-end: kick off the background job, poll until
// it finishes, then refresh the playlist/favorite caches the UI reads. Shared by
// the sidebar (desktop) and the library page (mobile); guarded against overlap
// via the `syncing` store. Returns true on success.
export async function runDeezerSync() {
  if (get(syncing)) return false;
  syncing.set(true);
  toasts.push("Synchronisation Deezer lancée…");
  try {
    await api.sync();
    await new Promise((resolve) => {
      const tick = async () => {
        try {
          const s = await api.syncStatus();
          if (!s.running) return resolve();
        } catch {
          return resolve();
        }
        setTimeout(tick, 3000);
      };
      setTimeout(tick, 3000);
    });
    invalidatePlaylists();
    await loadFavorites(true);
    toasts.push("Bibliothèque à jour");
    return true;
  } catch (e) {
    toasts.push(e?.message || "Échec de la synchronisation", "error");
    return false;
  } finally {
    syncing.set(false);
  }
}

// The playlist list, into the shared `playlists` store. Callers that just need
// the array still get it back; callers that need to STAY current subscribe to
// the store instead (that is the whole point — see stores.js).
let playlistsInFlight = null;
// Why the last attempt failed, so a screen can say so instead of painting the
// `null` state as "no playlist at all".
export const playlistsError = writable(null);
// When the list was last asked for, successful or not. The focus refresh below
// is bounded by this: a flaky link must not turn "come back to the tab" into a
// request per visit.
let playlistsTouchedAt = 0;

export async function userPlaylists(force = false) {
  const cached = get(playlists);
  if (cached && !force) return cached;
  // Several views mount at once on a cold start; they must share one request.
  if (playlistsInFlight && !force) return playlistsInFlight;
  playlistsTouchedAt = Date.now();
  // This IS the refresh a pending invalidation was waiting for.
  cancelPlaylistRefresh();
  const p = api
    .myPlaylists()
    .then((r) => {
      const list = r.playlists || [];
      playlistsError.set(null);
      playlists.set(list);
      playlistFolders.set(r.folders || []);
      warmPlaylists(list);
      return list;
    })
    .catch((e) => {
      // Don't memoize a failure: storing [] here left the sidebar / picker / the
      // library tab permanently empty after a single failed load (offline boot,
      // server not up yet) until an unrelated edit invalidated the cache. Leave
      // the store as it is, so the next call refetches.
      playlistsError.set(e || { status: 0, message: "network" });
      return get(playlists) || [];
    })
    .finally(() => {
      if (playlistsInFlight === p) playlistsInFlight = null;
    });
  playlistsInFlight = p;
  return p;
}

// Something changed the playlists server-side: refetch so every mounted view
// updates itself. Debounced, because the edit paths call this per operation
// (adding ten tracks one by one used to mean ten refetches) and `/me/playlists`
// counts the tracks of every playlist — it is not a free call.
const PLAYLIST_REFRESH_DELAY = 600;
let playlistRefreshTimer = null;
function cancelPlaylistRefresh() {
  clearTimeout(playlistRefreshTimer);
  playlistRefreshTimer = null;
}

export function invalidatePlaylists() {
  cancelPlaylistRefresh();
  playlistRefreshTimer = setTimeout(() => {
    playlistRefreshTimer = null;
    userPlaylists(true).catch(() => {});
  }, PLAYLIST_REFRESH_DELAY);
}

// A list shared while this tab was in the background must appear without a
// reload: one request at most a minute, never offline, never twice at once.
const PLAYLIST_FOCUS_AGE = 60 * 1000;

export function refreshPlaylistsIfStale() {
  if (typeof document !== "undefined" && document.hidden) return;
  if (!get(online) || get(playlists) === null) return;
  if (playlistsInFlight) return;
  if (Date.now() - playlistsTouchedAt < PLAYLIST_FOCUS_AGE) return;
  userPlaylists(true).catch(() => {});
}

if (typeof window !== "undefined") {
  window.addEventListener("focus", refreshPlaylistsIfStale);
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) refreshPlaylistsIfStale();
  });
}

// -- collaborative playlists: the shapes the UI decides on -------------------
// The server enforces the matrix; these helpers only read what it answered, once
// (lib/api.js attaches the parsed 409 body to `err.body`).

/** A list somebody else owns, from `/me/playlists` or the playlist payload. */
export function isSharedPlaylist(p) {
  return !!p && !!p.role && p.role !== "owner";
}

/** The words the collaborator controls use, in one place. */
export function roleLabel(role) {
  if (role === "owner") return "Propriétaire";
  if (role === "editor") return "Peut modifier";
  if (role === "viewer") return "Lecture seule";
  if (role === "admin") return "Administrateur";
  return "";
}

/** The 409 body of an optimistic-concurrency mismatch, or null (an "owner" 409
 *  is not a conflict: nothing was reloaded). */
export function conflictOf(err) {
  if (!err || err.status !== 409 || !err.body || err.body.error !== "conflict") return null;
  return err.body;
}

/** What a conflict says: who moved the playlist, and which half of it. */
export function conflictNotice(body) {
  if (!body) return "La playlist a changé entre-temps — rechargée.";
  if (body.scope === "members") return "La liste des collaborateurs a changé — rechargée.";
  return body.last_edited_by
    ? `Modifiée par ${body.last_edited_by} — rechargée.`
    : "La playlist a changé entre-temps — rechargée.";
}

/** Why a membership call failed, in words a person can act on. */
export function memberError(err, username = "") {
  if (!err) return "";
  if (err.offline || err.status === 0) return "Hors ligne : la modification n'a pas été enregistrée.";
  const code = (err.body && err.body.error) || "";
  if (err.status === 429) {
    const seconds = Math.max(1, Math.trunc(Number(err.body && err.body.retry_after) || 1));
    return `Trop d'invitations d'un coup — réessayez dans ${seconds} s.`;
  }
  if (err.status === 409) {
    if (code === "owner") return "Cette personne est déjà propriétaire de la playlist.";
    if (code === "member limit") return "Cette playlist a atteint sa limite de 100 collaborateurs.";
    if (code === "playlist limit") return "Ce compte a atteint sa limite de playlists.";
    if (code === "conflict") return conflictNotice(err.body);
    return "La playlist a changé entre-temps — réessayez.";
  }
  if (err.status === 404) {
    if (code === "unknown user") return `Aucun compte ne s'appelle « ${username} » sur ce serveur.`;
    return "Cette playlist ne vous est plus accessible.";
  }
  if (err.status === 403) return "Vous n'avez plus le droit de gérer les collaborateurs.";
  if (err.status === 400) {
    if (code === "self") return "C'est votre propre compte.";
    if (code === "invalid role") return "Rôle invalide.";
    if (code === "invalid username") return "Indiquez un nom d'utilisateur.";
    return "Demande invalide.";
  }
  return "Impossible de modifier les collaborateurs.";
}

// One resync per (playlist, scope) at a time: a second 409 arriving during the
// reload waits for the same GET instead of starting a loop. The entry is
// dropped once it settles, so a later conflict asks again.
const resyncs = new Map();

export function resyncOnce(key, run) {
  const k = String(key);
  const pending = resyncs.get(k);
  if (pending) return pending;
  const p = Promise.resolve().then(run);
  resyncs.set(k, p);
  const done = () => {
    if (resyncs.get(k) === p) resyncs.delete(k);
  };
  p.then(done, done);
  return p;
}

/** What the 30 s probe means: the members list moved, the content moved, and
 *  whether the page may take the truth NOW or must offer the pill instead. */
export function probeDecision(seen, next, { editing = false, busy = false } = {}) {
  const members = !!next && next.members_version !== seen?.members_version;
  const content = !!next && next.version !== seen?.version;
  return {
    members,
    reload: content && !editing && !busy,
    stale: content && (editing || busy),
  };
}

// Merge a playlist the user just created or renamed into the store NOW. The
// refetch above confirms it a moment later; this is what makes a playlist
// created from the "add to playlist" sheet appear in the library and the
// sidebar the instant it exists, instead of only after a navigation.
export function upsertPlaylistLocal(p) {
  if (!p || !p.id) return;
  playlists.update((list) => {
    const cur = list || [];
    const i = cur.findIndex((x) => String(x.id) === String(p.id));
    // New: the server orders by creation date, newest first — so does this.
    if (i < 0) return [{ nb_tracks: 0, editable: true, cover: null, ...p }, ...cur];
    const next = cur.slice();
    next[i] = { ...cur[i], ...p };
    return next;
  });
}

export function removePlaylistLocal(id) {
  if (!id) return;
  playlists.update((list) =>
    list ? list.filter((p) => String(p.id) !== String(id)) : list
  );
}

// Keep a playlist's track count honest between an edit and its refetch.
function bumpPlaylistCount(id, delta) {
  if (!id || !delta) return;
  playlists.update((list) => {
    if (!list) return list;
    const i = list.findIndex((p) => String(p.id) === String(id));
    if (i < 0) return list;
    const next = list.slice();
    next[i] = { ...next[i], nb_tracks: Math.max(0, (next[i].nb_tracks || 0) + delta) };
    return next;
  });
}

const OFFLINE_FOLDERS = "Hors ligne : impossible de modifier les dossiers";
const FOLDER_ORDER_DELAY = 400;
const folderOrderTimers = new Map();

function folderIdOf(value) {
  return value == null ? null : String(value);
}

function openFolderLocally(id) {
  const fid = folderIdOf(id);
  if (!fid) return;
  openFolders.update((ids) => {
    const list = ids || [];
    return list.some((x) => String(x) === fid) ? list : [...list, fid];
  });
}

function closeFolderLocally(id) {
  const fid = folderIdOf(id);
  if (!fid) return;
  openFolders.update((ids) => (ids || []).filter((x) => String(x) !== fid));
}

function offlineFolders() {
  toasts.push(OFFLINE_FOLDERS, "error");
}

function newFolderId() {
  if (globalThis.crypto?.randomUUID) return globalThis.crypto.randomUUID();
  const bytes = globalThis.crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = [...bytes].map((b) => b.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

async function putOrder(key, run) {
  const noticeId = "folder-order:" + key;
  try {
    await run();
    notices.dismiss(noticeId);
  } catch {
    notices.push({
      id: noticeId,
      kind: "error",
      message: "Ordre non enregistré",
      actionLabel: "Réessayer",
      action: () => putOrder(key, run),
    });
  }
}

function scheduleOrder(key, run) {
  const previous = folderOrderTimers.get(key);
  if (previous) clearTimeout(previous.timer);
  const timer = setTimeout(() => {
    folderOrderTimers.delete(key);
    putOrder(key, run);
  }, FOLDER_ORDER_DELAY);
  folderOrderTimers.set(key, { timer, run });
}

export async function flushPlaylistFolderOrders() {
  const pending = [...folderOrderTimers.entries()];
  folderOrderTimers.clear();
  await Promise.all(
    pending.map(([key, { timer, run }]) => {
      clearTimeout(timer);
      return putOrder(key, run);
    })
  );
}

export async function createFolder(title) {
  const name = (title || "").trim();
  if (!name) return null;
  if (!get(online)) {
    offlineFolders();
    return null;
  }
  const id = newFolderId();
  const folders = get(playlistFolders) || [];
  const folder = {
    id,
    title: name,
    position: folders.reduce((max, f) => Math.max(max, Number(f.position) || 0), -1) + 1,
    nb_playlists: 0,
  };
  playlistFolders.set([...folders, folder]);
  openFolderLocally(id);
  try {
    const r = await api.createPlaylistFolder(name, id);
    if (r?.position != null) {
      playlistFolders.update((list) =>
        (list || []).map((f) => (String(f.id) === id ? { ...f, position: r.position } : f))
      );
    }
    return { ...folder, position: r?.position ?? folder.position };
  } catch (e) {
    playlistFolders.update((list) => (list || []).filter((f) => String(f.id) !== id));
    closeFolderLocally(id);
    toasts.push(e?.offline ? OFFLINE_FOLDERS : "Impossible de créer le dossier", "error");
    return null;
  }
}

export async function renameFolder(id, title) {
  const fid = folderIdOf(id);
  const name = (title || "").trim();
  const folders = get(playlistFolders) || [];
  const folder = folders.find((f) => String(f.id) === fid);
  if (!folder || !name) return false;
  if (!get(online)) {
    offlineFolders();
    return false;
  }
  const previous = folder.title;
  playlistFolders.update((list) =>
    (list || []).map((f) => (String(f.id) === fid ? { ...f, title: name } : f))
  );
  try {
    await api.renamePlaylistFolder(fid, name);
    return true;
  } catch (e) {
    playlistFolders.update((list) =>
      (list || []).map((f) => (String(f.id) === fid ? { ...f, title: previous } : f))
    );
    toasts.push(e?.offline ? OFFLINE_FOLDERS : "Impossible de renommer le dossier", "error");
    return false;
  }
}

export async function deleteFolder(id) {
  const fid = folderIdOf(id);
  const folders = get(playlistFolders) || [];
  const folder = folders.find((f) => String(f.id) === fid);
  if (!folder) return false;
  if (!get(online)) {
    offlineFolders();
    return false;
  }
  const list = get(playlists) || [];
  const placed = list.filter((p) => folderIdOf(p.folder_id) === fid);
  const order = idsOf(placed);
  const wasOpen = (get(openFolders) || []).some((x) => String(x) === fid);
  const remaining = folders.filter((f) => String(f.id) !== fid);
  const rootOrder = [...containerIds(list, folders, null), ...order];
  const optimisticallyUnfiled = list.map((p) =>
    folderIdOf(p.folder_id) === fid ? { ...p, folder_id: null, position: null } : p
  );
  playlistFolders.set(remaining);
  playlists.set(orderContainer(optimisticallyUnfiled, remaining, null, rootOrder));
  closeFolderLocally(fid);
  try {
    const r = await api.deletePlaylistFolder(fid);
    const restoredOrder = r?.playlists || order;
    const count = restoredOrder.length;
    const moved = count
      ? ` · ${count} playlist${count > 1 ? "s" : ""} déplacée${count > 1 ? "s" : ""} dans « Sans dossier »`
      : "";
    toasts.push(`Dossier supprimé${moved} · Annuler`, "info", 8000, {
      label: "Annuler",
      run: () => restoreFolder(folder, restoredOrder),
    });
    return true;
  } catch (e) {
    playlistFolders.set(folders);
    playlists.set(list);
    if (wasOpen) openFolderLocally(fid);
    toasts.push(e?.offline ? OFFLINE_FOLDERS : "Impossible de supprimer le dossier", "error");
    return false;
  }
}

async function restoreFolder(folder, order) {
  if (!get(online)) {
    offlineFolders();
    return;
  }
  try {
    await api.createPlaylistFolder(folder.title, folder.id);
    playlistFolders.update((list) => [...(list || []), { ...folder, nb_playlists: order.length }]);
    openFolderLocally(folder.id);
    const folders = get(playlistFolders) || [];
    playlists.update((list) => {
      const replaced = (list || []).map((p) =>
        order.includes(String(p.id)) ? { ...p, folder_id: String(folder.id) } : p
      );
      return orderContainer(replaced, folders, folder.id, order);
    });
    if (order.length) await api.reorderPlaylistContainer(folder.id, order);
    toasts.push("Dossier restauré");
  } catch {
    toasts.push("Impossible de restaurer le dossier", "error");
  }
}

export function reorderFolders(ids) {
  const folders = get(playlistFolders) || [];
  if (!folders.length) return;
  const next = orderFolders(folders, ids);
  playlistFolders.set(next);
  playlists.update((list) => orderContainer(list || [], next, null, []));
  scheduleOrder("folders", () => api.reorderPlaylistFolders(idsOf(next)));
}

export function reorderContainer(folderId, ids) {
  const fid = folderIdOf(folderId);
  const folders = get(playlistFolders) || [];
  playlists.update((list) => orderContainer(list || [], folders, fid, ids));
  scheduleOrder(fid || "root", () => api.reorderPlaylistContainer(fid, ids));
}

export async function movePlaylistTo(playlistId, folderId) {
  const pid = folderIdOf(playlistId);
  const target = folderIdOf(folderId);
  const folders = get(playlistFolders) || [];
  if (target && !folders.some((f) => String(f.id) === target)) return false;
  const list = get(playlists) || [];
  const item = list.find((p) => String(p.id) === pid);
  if (!item) return false;
  if (folderIdOf(item.folder_id) === target) return true;
  if (!get(online)) {
    offlineFolders();
    return false;
  }
  const nextIds = [...containerIds(list, folders, target).filter((id) => id !== pid), pid];
  const moved = list.map((p) =>
    String(p.id) === pid ? { ...p, folder_id: target, position: null } : p
  );
  playlists.set(orderContainer(moved, folders, target, nextIds));
  if (target) openFolderLocally(target);
  try {
    await api.reorderPlaylistContainer(target, nextIds);
    return true;
  } catch (e) {
    playlists.set(list);
    toasts.push(e?.offline ? OFFLINE_FOLDERS : "Impossible de déplacer la playlist", "error");
    return false;
  }
}


// -- offline warming ---------------------------------------------------------
// Knowing your playlists exist is useless offline if their track lists aren't
// on the device. So once the list is loaded, quietly pull each playlist's
// tracks into the offline cache — ONE at a time, spaced out, skipping anything
// refreshed recently: this must never compete with what the user is actually
// doing (or with the audio stream sharing the link).

const WARM_MAX_AGE = 24 * 60 * 60 * 1000; // don't re-warm a fresh entry
const WARM_GAP = 1500; // ms between two warms
// Each of these costs the server live Deezer calls, so a session warms a
// bounded number of lists; the rest are picked up on later runs (and any list
// you actually open refreshes itself anyway).
const WARM_MAX = 25;
// Let the screens the user is actually looking at load first.
const WARM_START_DELAY = 10000;
let warming = false;

export async function warmPlaylists(list) {
  if (warming || !Array.isArray(list) || !list.length) return;
  if (!get(online)) return;
  warming = true;
  try {
    await new Promise((r) => setTimeout(r, WARM_START_DELAY));
    // Favourites first: it's the list people open offline most.
    const paths = [
      "/me/favorites",
      ...list.slice(0, WARM_MAX).map((p) => "/playlist/" + p.id),
    ];
    for (const path of paths) {
      if (!get(online)) break;
      if ((await cacheAge(path)) < WARM_MAX_AGE) continue;
      try {
        await api.get(path); // caches on success (see api.js)
      } catch {
        /* one unreachable playlist must not stop the rest */
      }
      await new Promise((r) => setTimeout(r, WARM_GAP));
    }
  } finally {
    warming = false;
  }
}

let favoritesLoaded = false;
// Two screens mounting at launch both asked, and since `favoritesLoaded` is only
// set once the answer is in, both went to the server: the same 40 kB list twice
// on every launch. A caller arriving while it is on its way shares it.
let favoritesInFlight = null;

export function loadFavorites(force = false) {
  if (favoritesLoaded && !force) return Promise.resolve();
  // A forced reload follows a change: it must not be handed an answer that
  // left before it.
  if (favoritesInFlight && !force) return favoritesInFlight;
  const mine = (async () => {
    try {
      const r = await api.favoriteIds();
      // The server may still name a track by its Deezer id: a key is a key.
      favorites.set(new Set((r.ids || []).map(migrateKey)));
      disliked.set(new Set((r.disliked || []).map(migrateKey)));
      favoritesLoaded = true;
    } catch {
      /* ignore */
    } finally {
      if (favoritesInFlight === mine) favoritesInFlight = null;
    }
  })();
  favoritesInFlight = mine;
  return mine;
}

export function isFavorite(id) {
  return get(favorites).has(migrateKey(id));
}

// Full favorite tracks, cached. First call fetches; later calls return the
// cache immediately and refresh in the background (the server response is cheap
// when the favorites haven't changed, thanks to its checksum cache).
let favTracksInFlight = null;
export const favTracksError = writable(null);
export async function loadMyFavorites(force = false) {
  const cached = get(favTracks);
  if (cached && !force) {
    refreshFavTracks();
    return cached;
  }
  // Nothing in memory (first library open of the session): paint the last-seen
  // list off disk while the network call runs. /me/favorites costs the server
  // two live Deezer round-trips, so on a slow link that's the difference
  // between an instant list and several seconds of skeleton.
  primeFavTracksFromCache();
  return refreshFavTracks();
}

async function primeFavTracksFromCache() {
  if (get(favTracks) !== null) return;
  const cached = await api.peek("/me/favorites");
  // Only fill the gap — never clobber a fresh response that landed meanwhile.
  if (cached && get(favTracks) === null) favTracks.set(cached.tracks || []);
}
function refreshFavTracks() {
  if (favTracksInFlight) return favTracksInFlight;
  favTracksError.set(null);
  favTracksInFlight = api
    .myFavorites()
    .then((r) => {
      // Reconcile rather than replace: the favourites are shown from the cached
      // copy first, and the fresh one is usually the same list plus or minus a
      // track. Merging in place means the browser re-renders those few rows
      // instead of all four thousand — no flash, no lost scroll, no re-decoded
      // artwork.
      favTracksError.set(null);
      favTracks.set(reconcileList(get(favTracks), r.tracks || []));
      return get(favTracks);
    })
    .catch((e) => {
      favTracksError.set(e || { status: 0, message: "network" });
      return get(favTracks) || [];
    })
    .finally(() => {
      favTracksInFlight = null;
    });
  return favTracksInFlight;
}

export async function toggleFavorite(track) {
  // A read-only remote control shows the owner's hearts but cannot move them.
  if (!CAN_KEEP) {
    toasts.push("Ce lien est en lecture seule", "info");
    return;
  }
  const id = trackKey(track);
  const on = !get(favorites).has(id);
  // optimistic
  if (on) favorites.add(id);
  else favorites.remove(id);
  updateFavTracksCache(track, on);
  try {
    await api.favorite(trackRef(track), on);
    toasts.push(on ? "Ajouté aux favoris" : "Retiré des favoris");
  } catch {
    // rollback
    if (on) favorites.remove(id);
    else favorites.add(id);
    updateFavTracksCache(track, !on);
    toasts.push("Échec de la mise à jour du favori", "error");
  }
}

// The queues the server generates: disliking the title playing there means
// "not this one", so it is skipped at once (Pandora's thumbs down). In an album
// or a playlist the person chose it: the dislike only concerns what is
// recommended (Deezer, Spotify), and the title plays on.
const GENERATED = new Set(["flow", "radio", "artist-radio", "reco", "genre-radio"]);

// "Je n'aime pas" / back. Optimistic, with "Annuler" in the toast; a third
// title of one artist offers to stop recommending the artist (Deezer does).
export async function toggleDislike(track, { quiet = false } = {}) {
  if (!CAN_KEEP) {
    toasts.push("Ce lien est en lecture seule", "info");
    return;
  }
  const id = trackKey(track);
  if (!id) return;
  const on = !get(disliked).has(id);
  if (on) disliked.add(id);
  else disliked.remove(id);
  let skipped = false;
  if (on) {
    const s = get(player);
    const cur = s.index >= 0 ? s.queue[s.index] : null;
    if (cur && trackKey(cur) === id && GENERATED.has(s.context?.kind)) {
      player.next();
      skipped = true;
    }
  }
  try {
    const r = await api.dislike(trackRef(track), on);
    if (quiet) return;
    if (on && r && r.artist_hint) {
      const h = r.artist_hint;
      toasts.push(`${h.count} titres de ${h.name} que vous n'aimez pas`, "info", 8000, {
        label: "Ne plus le recommander",
        run: () =>
          api
            .muteArtist(h.provider, h.id)
            .then(() => toasts.push(`${h.name} ne sera plus recommandé`))
            .catch(() => toasts.push("Échec, réessayez", "error")),
      });
    } else if (on) {
      toasts.push(
        skipped ? "Titre passé, il ne sera plus proposé" : "Ce titre ne sera plus proposé",
        "info",
        6000,
        { label: "Annuler", run: () => toggleDislike(track, { quiet: true }) }
      );
    } else {
      toasts.push("Ce titre peut de nouveau être proposé");
    }
  } catch {
    if (on) disliked.remove(id);
    else disliked.add(id);
    toasts.push("Échec, réessayez", "error");
  }
}

// Keep the cached favorites list consistent with a toggle, so the library view
// updates instantly without a refetch.
function updateFavTracksCache(track, on) {
  const list = get(favTracks);
  if (list === null) return; // not loaded yet — nothing to keep in sync
  const id = trackKey(track);
  if (on) {
    if (!list.some((t) => trackKey(t) === id)) {
      favTracks.set([{ ...track, added: Math.floor(Date.now() / 1000) }, ...list]);
    }
  } else {
    favTracks.set(list.filter((t) => trackKey(t) !== id));
  }
}

export async function toggleEntityFavorite(kind, id, on, meta = {}) {
  if (!CAN_KEEP) {
    toasts.push("Ce lien est en lecture seule", "info");
    return false;
  }
  try {
    await api.favoriteEntity(kind, id, on, meta);
    toasts.push(on ? "Ajouté à vos favoris" : "Retiré de vos favoris");
    return true;
  } catch {
    toasts.push("Échec de la mise à jour", "error");
    return false;
  }
}

// Subscribe to a podcast from a search result (its Deezer show id): imports the
// show + episodes on the server (archived on first play) and returns the local
// channel, so the caller can open it.
export async function subscribeToPodcast(showDeezerId) {
  try {
    const c = await api.subscribePodcast(
      "https://www.deezer.com/show/" + showDeezerId
    );
    toasts.push("Podcast ajouté");
    return c;
  } catch (e) {
    toasts.push(e?.message || "Échec de l'ajout du podcast", "error");
    return null;
  }
}

// A title on its own — on the home, in the search results, among the
// suggestions — plays at once and then turns into its radio: the queue starts
// with that title alone and the player tops it up from the radio anchored on
// it (Player.svelte#ensureUpcoming), the way Spotify, YouTube Music and Apple
// Music's Autoplay continue from a single song. No request stands between the
// tap and the sound. `artist` lets the server radio on the artist when the
// title is a catalogue one whose row its first play has not made yet.
export function playThenRadio(track) {
  if (!track) return;
  const key = trackRef(track);
  const artist = trackEntityRef(track, "artist", track.artist) || "";
  player.playQueue([track], 0, { kind: "radio", id: key, artist });
}

export async function startTrackRadio(track) {
  try {
    const key = trackRef(track);
    const r = await api.trackRadio(key);
    if (r.tracks && r.tracks.length) {
      // A radio starts with its own seed, then what sounds like it.
      const rest = r.tracks.filter((t) => trackRef(t) !== key);
      player.playQueue([track, ...rest], 0, { kind: "radio", id: key });
      toasts.push("Radio lancée");
    }
  } catch {
    toasts.push("Impossible de lancer la radio", "error");
  }
}

// Optimistic: the toast and the "last playlist" shortcut update INSTANTLY —
// the server call (which may include a Deezer round-trip) runs in the
// background, and only a failure surfaces afterwards. `version` is the content
// counter of the playlist the caller has on screen; the cross-screen picker
// sends none (an append is commutative, plan §8.5).
export function addTrackToPlaylist(playlistId, trackId, playlistTitle, version = null) {
  lastPlaylist.set({ id: String(playlistId), title: playlistTitle });
  // 2,4 s: the duration the design system gives a confirmation pill.
  toasts.push(`Ajouté à « ${playlistTitle} »`, "info", 2400);
  bumpPlaylistCount(playlistId, 1);
  return api
    .addToPlaylist(playlistId, [String(trackId)], version)
    .then(() => {
      // The lists show track counts — refresh them (debounced).
      invalidatePlaylists();
      return true;
    })
    .catch((e) => {
      bumpPlaylistCount(playlistId, -1); // the optimistic count was wrong
      // The playlist moved under the caller: say who, reload the list, and let
      // the person ask again — the retry retries WITHOUT the stale counter, so
      // it can land instead of conflicting a second time.
      const conflict = conflictOf(e);
      if (conflict) {
        invalidatePlaylists();
        toasts.push(conflictNotice(conflict), "error", 6000, {
          label: "Réessayer",
          run: () => addTrackToPlaylist(playlistId, trackId, playlistTitle),
        });
        return false;
      }
      // A mutation that failed can be retried from the toast itself: the
      // person does not have to reopen the sheet and find the playlist again.
      toasts.push(`Échec de l'ajout à « ${playlistTitle} »`, "error", 6000, {
        label: "Réessayer",
        run: () => addTrackToPlaylist(playlistId, trackId, playlistTitle),
      });
      return false;
    });
}

// -- playlist creation (one place, so every form says the same thing) --------

// POST the new playlist, then show it EVERYWHERE at once (sidebar, library,
// the picker that is currently open over a screen that will not remount), and
// let the debounced refetch confirm the server's copy. Throws on failure: the
// caller owns the toast, because only it knows what the playlist was for.
async function postNewPlaylist(title, refs, folder = null) {
  const r = await api.createPlaylist(title, refs, folder);
  const list = refs || [];
  if (r?.id) {
    upsertPlaylistLocal({
      id: String(r.id),
      title,
      deezer_id: r.deezer_id,
      nb_tracks: list.length,
      folder_id: r.folder_id ?? folder,
    });
    if (list.length) lastPlaylist.set({ id: String(r.id), title });
  }
  invalidatePlaylists();
  return r || null;
}

const countLabel = (n) => `${n} titre${n > 1 ? "s" : ""}`;

// Create a playlist from the picker (a title, or a title + the track being
// added). Optimistic on the LIST, not on the tracks: the server is the one
// that decides the rows, and the answer is what closes the form.
export async function createPlaylistFrom(title, refs = [], folder = null) {
  const name = (title || "").trim();
  const list = (refs || []).filter(Boolean);
  if (!name) return null;
  try {
    const r = await postNewPlaylist(name, list, folder);
    if (list.length === 1) toasts.push(`Ajouté à « ${name} »`, "info", 2400);
    else if (list.length > 1) toasts.push(`${countLabel(list.length)} ajoutés à « ${name} »`, "info", 2400);
    else toasts.push(`Playlist « ${name} » créée`, "info", 2400);
    return r;
  } catch {
    toasts.push(`Impossible de créer « ${name} »`, "error", 6000, {
      label: "Réessayer",
      run: () => createPlaylistFrom(name, list, folder),
    });
    return null;
  }
}

// "Enregistrer la file en playlist": the queue's titles, IN ORDER, become a
// new playlist. One request, no new route — api.createPlaylist takes the refs.
export async function saveQueueAsPlaylist(title, refs = []) {
  const name = (title || "").trim();
  const list = (refs || []).filter(Boolean);
  if (!name) return null;
  try {
    const r = await postNewPlaylist(name, list);
    toasts.push(`File enregistrée dans « ${name} » · ${countLabel(list.length)}`, "info", 2400);
    return r;
  } catch {
    toasts.push(`Impossible d'enregistrer la file dans « ${name} »`, "error", 6000, {
      label: "Réessayer",
      run: () => saveQueueAsPlaylist(name, list),
    });
    return null;
  }
}

// -- offline downloads (device-local; available to every user) --------------

export async function downloadTrackTo(track, quality = null) {
  const q = quality || get(downloadQuality);
  if (isDownloaded(trackKey(track))) return;
  toasts.push(`Téléchargement de « ${track.title} »…`);
  const ok = await downloadTrack(track, q);
  toasts.push(
    ok ? `« ${track.title} » disponible hors-ligne` : "Échec du téléchargement",
    ok ? "info" : "error"
  );
}

export async function undownloadTrack(track) {
  await removeTrack(trackKey(track));
  toasts.push("Téléchargement retiré");
}

// How many track downloads to run in parallel for a whole album/playlist. The
// server archives each in its own worker (see download_workers), so a few
// concurrent requests cut the wait dramatically without swamping the device.
const DL_CONCURRENCY = 4;

// Download a set of tracks (fail-soft), a few at a time. Local-only and already
// cached tracks are skipped. Used for whole albums / playlists.
export async function downloadTracks(tracks, quality = null) {
  const q = quality || get(downloadQuality);
  const list = (tracks || []).filter(
    (t) => t && trackKey(t) && !isDownloaded(trackKey(t))
  );
  if (!list.length) {
    toasts.push("Déjà disponible hors-ligne");
    return;
  }
  toasts.push(`Téléchargement de ${list.length} titre(s)…`);
  // Bounded worker pool: DL_CONCURRENCY consumers pull from a shared cursor so
  // several tracks archive + download at once instead of strictly one by one.
  let ok = 0;
  let cursor = 0;
  async function worker() {
    for (;;) {
      const i = cursor++;
      if (i >= list.length) return;
      if (await downloadTrack(list[i], q)) ok++;
    }
  }
  await Promise.all(
    Array.from({ length: Math.min(DL_CONCURRENCY, list.length) }, worker)
  );
  toasts.push(
    ok === list.length ? `${ok} titre(s) téléchargé(s)` : `${ok}/${list.length} téléchargé(s)`,
    ok ? "info" : "error"
  );
}

// Fetch an album/playlist's tracks, then download them all.
export async function downloadEntity(kind, id, quality = null) {
  try {
    let tracks = [];
    if (kind === "album") tracks = (await api.album(id)).tracks;
    else if (kind === "playlist") tracks = (await api.playlist(id)).tracks;
    await downloadTracks(tracks, quality);
  } catch {
    toasts.push("Téléchargement impossible", "error");
  }
}

// Load and play an album/playlist/artist as a queue.
export async function playEntity(kind, id, context = null) {
  try {
    let tracks = [];
    if (kind === "album") tracks = (await api.album(id)).tracks;
    else if (kind === "playlist") tracks = (await api.playlist(id)).tracks;
    else if (kind === "artist") tracks = (await api.artistRadio(id)).tracks;
    if (tracks && tracks.length) {
      player.playQueue(tracks, 0, context || { kind, id });
    } else {
      toasts.push("Rien à lire ici", "error");
    }
  } catch {
    toasts.push("Lecture impossible", "error");
  }
}

// The menu's ITEMS live in lib/trackmenu.js; this file binds them to the
// operations that own the API and the optimistic stores, and re-exports the
// builders so every existing import ("../lib/actions.js") keeps working.
const MENUS = createTrackMenus({
  toggleFavorite,
  toggleDislike,
  startTrackRadio,
  addTrackToPlaylist,
  downloadTrackTo,
  undownloadTrack,
  playEntity,
  downloadEntity,
  toggleEntityFavorite,
  movePlaylistTo,
  subscribeToPodcast,
});

export const buildTrackMenu = (track, nav, options = {}) => MENUS.track(track, nav, options);
export const buildEntityMenu = (kind, item, nav) => MENUS.entity(kind, item, nav);
export const buildPodcastMenu = (item, nav, options = {}) => MENUS.podcast(item, nav, options);

onOwnerChange(() => {
  cancelPlaylistRefresh();
  playlistsInFlight = null;
  playlistsTouchedAt = 0;
  playlists.set(null);
  playlistFolders.set(null);
  warming = false;
  favoritesInFlight = null;
  favoritesLoaded = false;
  favorites.set(new Set());
  disliked.set(new Set());
  favTracksInFlight = null;
  favTracks.set(null);
});
