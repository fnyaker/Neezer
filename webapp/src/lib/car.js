// The Android app's car mode, from the page's side.
//
// The overlay is native (android/.../CarOverlayRoot.kt): it draws its own
// buttons and playlists, and the WebView still owns the queue, the Flow and the
// favourites. The two hooks below are the only door between them:
//
//   window.__nsCarPlay(kind, id, title, payload) — launch what the overlay
//   tapped, through the SAME functions the SPA's own buttons use (playEntity,
//   startFlow, player.playQueue). No second playback path, no new endpoint.
//   "favorites" is the one asynchronous kind: the page reads the person's
//   starred tracks (/api/me/favorites, through the app's own api client and its
//   offline cache) and then queues them; the page says what happened, the native
//   service stays silent for that kind.
//
//   window.__nsCarFav() — present only while a track is loaded; toggles the
//   current favourite and returns its new state synchronously (the optimistic
//   store update happens before the request). The native heart can therefore
//   answer at once, and disappear when nothing is playing.
//
// The injection is a no-op in a browser: `installCarBridge` is called once by
// App.svelte with the app's own stores/actions, so the player itself is always
// injected and the launch path is testable on its own. Older APKs simply have
// no `NSNative.carAvailable`, and every entry point here stays hidden.

import { get } from "svelte/store";
import { api } from "./api.js";
import { toasts } from "./stores.js";

/**
 * Whether the installed APK carries the car mode. The bridge is injected
 * before this bundle runs, so one read at mount is enough for the settings
 * card; `carAvailable()` stays live for anything that asks later.
 */
export function carAvailable() {
  if (typeof window === "undefined") return false;
  const bridge = window.NSNative;
  if (!bridge || typeof bridge.carAvailable !== "function") return false;
  try {
    // A method that exists but says false means this install cannot show the
    // overlay (no permission, service unavailable): believe it. An APK that
    // returned nothing at all still counts as having the mode.
    return bridge.carAvailable() !== false;
  } catch {
    return false;
  }
}

/** Open the native full-screen car settings. Returns whether it was opened. */
export function openCarSettings() {
  if (!carAvailable()) return false;
  try {
    // Strictly true: an APK from before the bridge returned a boolean gives
    // `undefined`, and the page must say "indisponible" rather than pretend
    // the screen opened.
    return window.NSNative.openCarSettings() === true;
  } catch {
    return false;
  }
}

/**
 * Pin a playlist for the car (the action on a playlist's page). The native
 * side order is the display order; pinning an already pinned playlist is a
 * no-op there. `false` when this APK has no car mode or refused the call.
 *
 * `nbTracks` is optional: when the page knows it, the tile says "42 titres"
 * before the first `/api/me/playlists` read. The three-argument overload is
 * part of bridge 9; older APKs have no car mode at all, so nothing is lost.
 */
export function carPin(id, title, nbTracks) {
  if (!carAvailable() || id === null || id === undefined) return false;
  const key = String(id).trim().slice(0, 64);
  if (!key) return false;
  try {
    const label = String(title || "").slice(0, 200);
    const count = Number(nbTracks);
    const known = Number.isFinite(count) && count > 0 && count <= 1_000_000;
    const accepted = known
      ? window.NSNative.carPin(key, label, Math.floor(count))
      : window.NSNative.carPin(key, label);
    return accepted === true;
  } catch {
    return false;
  }
}

/** The kinds the overlay may launch; anything else is ignored. */
const KINDS = new Set(["flow", "favorites", "track", "playlist", "album", "artist", "mix"]);
/** One track's JSON from the page: a few kilobytes, never a whole queue. */
const MAX_PAYLOAD = 64 * 1024;

/** The built-in Favoris queue: one stable context for every launch. */
const FAVORITES_CONTEXT = { kind: "favorites", id: "favorites" };

/**
 * The "Favoris" tile: read the person's starred tracks, then play them through
 * the player's own queue. It is asynchronous (one request), so the hook answers
 * `true` at once — "accepted, loading" — and the PAGE says what really
 * happened. The native service shows no toast for this kind, which is what
 * keeps "Playlist lancée : Favoris" from appearing before there is a queue.
 *
 * `deps.favorites` is the app's own reader (`api.myFavorites`, injected by
 * installCarBridge); `deps.toast` is the app's toast store, so the messages are
 * visible in the player and testable without one.
 */
function playFavorites(deps) {
  if (!deps.player || typeof deps.player.playQueue !== "function") return false;
  if (typeof deps.favorites !== "function") return false;
  const say = typeof deps.toast === "function" ? deps.toast : () => {};
  say("Chargement des favoris…");
  void (async () => {
    try {
      const answer = await deps.favorites();
      const tracks = Array.isArray(answer)
        ? answer
        : answer && Array.isArray(answer.tracks)
          ? answer.tracks
          : [];
      if (!tracks.length) {
        say("Aucun favori pour l'instant");
        return;
      }
      deps.player.playQueue(tracks, 0, { ...FAVORITES_CONTEXT });
      say("Lecture des favoris");
    } catch (error) {
      if (error && error.status === 401) say("Session absente", "error");
      else if (error && error.offline) say("Hors ligne : favoris indisponibles", "error");
      else say("Favoris indisponibles", "error");
    }
  })();
  return true;
}

/** Parse the track object the native side carried along, or null. */
export function carTrackPayload(payload) {
  if (payload === null || payload === undefined) return null;
  if (typeof payload === "object") return payload;
  if (typeof payload !== "string" || payload.length > MAX_PAYLOAD) return null;
  try {
    const value = JSON.parse(payload);
    return value && typeof value === "object" && !Array.isArray(value) ? value : null;
  } catch {
    return null;
  }
}

/**
 * What `window.__nsCarPlay` does, with the player's own functions passed in.
 * Exported for the tests; the installed hook is the same call.
 *
 * Returns whether a launch was started. `false` is what lets the overlay say
 * "impossible" instead of pretending a playlist is playing.
 */
export function playFromCar(deps, kind, id, title, payload) {
  if (!deps || typeof kind !== "string" || !KINDS.has(kind)) return false;
  if (deps.remote) return false;
  // Without a session the API answers 401: refuse before the request, so the
  // overlay can say "connecte-toi" rather than spin a dead launch.
  if (typeof deps.isSignedIn === "function" && !deps.isSignedIn()) return false;

  if (kind === "favorites") return playFavorites(deps);

  if (kind === "flow") {
    if (typeof deps.startFlow !== "function") return false;
    try {
      deps.startFlow();
      return true;
    } catch {
      return false;
    }
  }

  if (kind === "track") {
    const track = carTrackPayload(payload);
    if (!track || typeof deps.trackKey !== "function") return false;
    let key = null;
    try {
      key = deps.trackKey(track);
    } catch {
      return false;
    }
    if (!key) return false;
    const raw = String(id || key).trim();
    if (raw.length > 128) return false;
    const context = { kind: "track", id: raw || String(key) };
    try {
      deps.player.playQueue([track], 0, context);
      return true;
    } catch {
      return false;
    }
  }

  const ref = id === null || id === undefined ? "" : String(id).trim();
  if (!ref || ref.length > 128) return false;
  if (typeof deps.playEntity !== "function") return false;
  try {
    // playEntity's third parameter is a playback CONTEXT, not a title: the
    // overlay's title is only for its own display and must not leak here.
    deps.playEntity(kind, ref);
    return true;
  } catch {
    return false;
  }
}

/**
 * Install the two hooks once. `deps` are App.svelte's own imports:
 * `{ player, current, user, isFavorite, toggleFavorite, playEntity, startFlow,
 *    trackKey, remote }` (the stores are Svelte readable stores; `user` is read
 * with `get`). Returns false when called twice or with an incomplete set —
 * a second install must not wrap the first and double a favourite toggle.
 */
export function installCarBridge(deps) {
  if (typeof window === "undefined") return false;
  if (!deps || !deps.player || !deps.current || !deps.user) return false;
  if (typeof deps.playEntity !== "function" || typeof deps.startFlow !== "function") return false;
  const missing = ["isFavorite", "toggleFavorite", "trackKey"].filter(
    (key) => typeof deps[key] !== "function"
  );
  if (missing.length) return false;
  if (window.__nsCarInstalled) return false;
  window.__nsCarInstalled = true;

  window.__nsCarPlay = function (kind, id, title, payload) {
    return playFromCar(
      {
        player: deps.player,
        remote: !!deps.remote,
        isSignedIn: () => !!get(deps.user),
        playEntity: deps.playEntity,
        startFlow: deps.startFlow,
        trackKey: deps.trackKey,
        // The one kind that needs a request: the app's own cached client, not
        // a second fetch of its own. A test (or a display) may inject one.
        favorites: deps.favorites || (() => api.myFavorites()),
        toast: deps.toast || ((message, tone) => toasts.push(message, tone)),
      },
      kind,
      id,
      title,
      payload
    );
  };

  // The heart exists only while a track is loaded. The native side checks
  // `typeof window.__nsCarFav === "function"`; on a track change the hook is
  // replaced with one closing over the new track, and removed at the end of
  // the queue.
  const syncFavourite = (track) => {
    if (!track) {
      try {
        delete window.__nsCarFav;
      } catch {
        window.__nsCarFav = undefined;
      }
      return;
    }
    window.__nsCarFav = function () {
      if (!get(deps.user)) return false;
      let was = false;
      try {
        was = deps.isFavorite(deps.trackKey(track));
      } catch {
        was = false;
      }
      try {
        deps.toggleFavorite(track);
      } catch {
        return was;
      }
      // toggleFavorite is optimistic: the new state is in the store before its
      // first await, so this is already the truth the heart should show.
      return !was;
    };
  };
  deps.current.subscribe(syncFavourite);

  return true;
}
