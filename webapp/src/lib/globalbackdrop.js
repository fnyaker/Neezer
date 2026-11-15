// The rules of the app's ground: which sleeve, at what size, and what the eco
// switch means for the whole document.
//
// The DOM half lives in `components/GlobalBackdrop.svelte` (one fixed, blurred,
// low-resolution layer behind every screen); this file holds the two decisions
// that can be made — and tested — without a browser.
//
// WHY 500 AND NOT 1000. The blurred layer is a ground, not a picture: a 64px
// blur destroys every detail a 1000px sleeve would carry, while the bytes and
// the decoded bitmap are four times the 500px ones. 500 is also the exact size
// the lists and the play cache already fetch (`baseCover`), so pointing the
// ground at it costs ZERO extra requests — the browser serves it from its own
// cache, and when the device holds the art as a blob (`offlineCovers`) that
// local copy wins, which is what makes the ground work in airplane mode.
//
// WHY THE ECO ATTRIBUTE LIVES HERE. `data-eco="1"` on <html> is the contract
// between this layer and the translucent surfaces (cards, menus, the top bar):
// when it is set there is NO blurred image anywhere in the app, so a
// `backdrop-filter` on a card would have nothing to blur but a flat colour and
// would only cost GPU. The surfaces read the attribute and drop their own blur;
// the ground reads it and paints nothing at all.

import { baseCover, resolveCover } from "./format.js";

// The canonical size the app already downloads (≤ 512 px, as the ground requires).
export const BACKDROP_SIZE = 500;

/** The art a track carries: its album's, or its own (a podcast episode). */
export function coverOf(track) {
  if (!track) return null;
  return track.album?.cover || track.cover || track.coverArt || track.picture || null;
}

/**
 * The URL the ground should paint for `track`, or null when it has no art.
 *
 * `covers` is the `offlineCovers` map (`coverKey` → object URL). Resolving
 * through it is not a fallback: a sleeve already on the device is the BEST
 * source, because it costs no request and works offline. `coverKey` collapses
 * the embedded WxH, so a blob cached at 1000px still answers a 500px URL.
 */
export function backdropCover(track, covers) {
  const raw = coverOf(track);
  if (!raw || typeof raw !== "string") return null;
  const low = baseCover(raw, BACKDROP_SIZE) || raw;
  return resolveCover(covers, low) || low;
}

/**
 * Put (or remove) the eco contract on the document element.
 *
 * Idempotent, returns the state it applied, and tolerant of a missing document
 * so the boot path can call it before anything is mounted. `root` is injectable
 * for the test (a plain `{ dataset: {} }` is enough).
 */
export function applyEco(on, root) {
  const el = root || (typeof document !== "undefined" ? document.documentElement : null);
  if (!el || !el.dataset) return false;
  if (on) el.dataset.eco = "1";
  else delete el.dataset.eco;
  return !!on;
}

/** Subscribe a store to the eco contract; returns the unsubscribe function. */
export function watchEco(store, root) {
  return store.subscribe((on) => applyEco(on, root));
}
