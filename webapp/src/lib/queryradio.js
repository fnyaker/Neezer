// A radio started from a SEARCH, not from a title or an artist page: the
// results already found are the opening titles, and /api/radio/next fills the
// queue from the best one. Pure where it can be, so the count, the threshold,
// the ordering and the single request are provable without a browser.
//
// Why a threshold of results at all: a search that found two answers is a
// lookup, and a radio built from two titles is a shuffle of those two. Thirty
// results is the point where "play what this is about" replaces "open this".
// The count is what the screen shows across its tabs (titles, artists, albums,
// playlists, shows, and the fallback's hits), each entry once: a video that
// duplicates a catalogue row is not a second result.
//
// Why the queue is filtered by score: `rankSearchResults` answers "which order
// should the list show"; a radio needs "which titles are really about this".
// The catalogue's bar is CATALOGUE_MIN and the fallback's YOUTUBE_MIN, exactly
// the bars the search already applies to decide what deserves a line. A title
// under them is another result in the list, never a seed for the radio.
//
// Sources for the interaction: Spotify and YouTube Music name the action
// "Go to radio" / "Start radio" and start it from one row; Material 3's
// progress indicators and Apple's HIG ask for the busy state to live IN the
// control that was pressed, and WCAG 2.1 asks for the 44 px target the design
// system already gives a coarse pointer.

import {
  CATALOGUE_MIN,
  YOUTUBE_MIN,
  matchScore,
  rankSearchResults,
  recordingKey,
  topResult,
} from "./searchrank.js";
import { entityRef, trackEntityRef } from "./entity.js";
import { trackKey, trackRef } from "./trackkey.js";

/** How many results a search must show before it deserves a radio. */
export const RADIO_MIN_RESULTS = 30;
/** The queue length the launch aims for: below it, /api/radio/next tops it up. */
export const RADIO_TARGET = 30;

/** Whether a result count is broad enough for the button to appear. The screen
 *  keeps the other gates (a term in the box, results for THAT term, the
 *  library scope, no pasted playlist link) next to their own state. */
export function offerRadio(total) {
  return Number(total) >= RADIO_MIN_RESULTS;
}

// A key is added for every identity an item can be reached by, so the same
// row found under two forms (a UUID, a provider id, a re-upload of the same
// recording) is counted once. `null` keys are ignored.
function addKeys(seen, keys) {
  const kept = keys.filter(Boolean);
  if (!kept.length) return false;
  if (kept.some((k) => seen.has(k))) return true;
  for (const k of kept) seen.add(k);
  return false;
}

function trackKeys(t) {
  if (!t) return [];
  const key = trackKey(t);
  const ref = trackRef(t);
  const rec = recordingKey(t);
  return [
    key && "k:" + key,
    ref && "r:" + ref,
    // A deezer id is one row; a recording key catches the same song uploaded
    // twice or found on the fallback under another id.
    rec && "s:" + rec,
  ];
}

function entityKeys(x) {
  if (!x) return [];
  return [
    x.id && "i:" + x.id,
    x.deezer_id && "d:" + x.deezer_id,
    // Entities have no stable `name` key on purpose: two artists can share a
    // name and still be two rows.
  ];
}

function countUnique(items, keysOf) {
  const seen = new Set();
  let n = 0;
  for (const item of items || []) {
    if (!item) continue;
    const keys = keysOf(item);
    if (!keys.filter(Boolean).length) continue;
    if (addKeys(seen, keys)) continue;
    n++;
  }
  return n;
}

/**
 * How many distinct results the search found, across every tab. The fallback
 * hits are passed only when that source is ON and has answered for this term:
 * an unanswered source must not change the count under the person's eyes.
 *
 * @param {string} query
 * @param {{tracks?: any[], artists?: any[], albums?: any[], playlists?: any[], podcasts?: any[]}} results
 * @param {any[]} youtube the fallback's hits, when that source is active
 * @param {{withPodcasts?: boolean}} options shows are counted only for a screen
 *   that shows them (the admin's)
 * @returns {number}
 */
export function countSearchResults(
  query,
  results = {},
  youtube = [],
  { withPodcasts = true } = {},
) {
  const tracks = rankSearchResults(
    String(query || "").trim(),
    results?.tracks || [],
    youtube || [],
  ).tracks;
  return (
    countUnique(tracks, trackKeys) +
    countUnique(results?.artists, entityKeys) +
    countUnique(results?.albums, entityKeys) +
    countUnique(results?.playlists, entityKeys) +
    (withPodcasts ? countUnique(results?.podcasts, entityKeys) : 0)
  );
}

/**
 * The titles worth opening a radio with: above the search's own bar for their
 * source, de-duplicated by id AND by recording, strongest first. A library row
 * wins a tie (it plays from disk now); the catalogue's own order breaks the
 * rest, so two equally close titles keep the order the search showed.
 *
 * @param {string} query
 * @param {{tracks?: any[]}} results
 * @param {any[]} youtube
 * @returns {any[]} track objects, in play order
 */
export function relevantTracks(query, results = {}, youtube = []) {
  const term = String(query || "").trim();
  if (!term) return [];
  const ranked = rankSearchResults(term, results?.tracks || [], youtube || []).tracks;
  const seen = new Set();
  const kept = [];
  ranked.forEach((track, order) => {
    if (!track) return;
    const source = track.source === "youtube" ? "youtube" : "catalogue";
    const score = matchScore(term, track);
    if (score < (source === "youtube" ? YOUTUBE_MIN : CATALOGUE_MIN)) return;
    const keys = trackKeys(track);
    if (!keys.filter(Boolean).length) return; // nothing playable
    if (addKeys(seen, keys)) return;
    kept.push({
      track,
      score,
      local: track.local === true && source !== "youtube",
      order,
    });
  });
  kept.sort(
    (a, b) =>
      b.score - a.score ||
      Number(b.local) - Number(a.local) ||
      a.order - b.order,
  );
  return kept.map((x) => x.track);
}

/** The artist a search names, for a radio that has no title to start from:
 *  `topResult`'s exact/prefix match first, then the catalogue's own first
 *  answer. Null when the search found no artist at all. */
function bestArtist(query, results) {
  const top = topResult(query, results || {}, false);
  if (top?.kind === "artist" && entityRef(top.item)) return top.item;
  return (results?.artists || []).find((a) => entityRef(a)) || null;
}

/**
 * The radio's seed: the best title when there is one, so the radio sounds like
 * what was typed; otherwise the artist the search names. A catalogue title
 * absent from the library still radios on its artist — /api/radio/next does
 * that fallback, and only when the seed carries `artist`.
 *
 * @returns {{kind: "track", id: string, artist?: string}|{kind: "artist", id: string}|null}
 */
export function radioSeed(query, results, tracks = []) {
  const term = String(query || "").trim();
  const best = tracks[0] || null;
  if (best) {
    const id = trackRef(best);
    if (id) {
      const artist =
        trackEntityRef(best, "artist", best.artist) ||
        entityRef(bestArtist(term, results));
      const seed = { kind: "track", id };
      if (artist) seed.artist = String(artist);
      return seed;
    }
  }
  const artist = bestArtist(term, results);
  const id = entityRef(artist);
  return id ? { kind: "artist", id: String(id) } : null;
}

/**
 * What the player carries while this radio plays. `title` is the honest name
 * of the context ("Radio de patrick sébastien") and the field the remote
 * control already carries; the kind stays the one Player.svelte knows: `radio`
 * for a title seed (it can top up on the title or its artist), `artist-radio`
 * for an artist seed.
 */
export function radioContext(query, seed) {
  const title = `Radio de ${String(query || "").trim()}`;
  if (!seed) return null;
  if (seed.kind === "artist") return { kind: "artist-radio", id: seed.id, title };
  const ctx = { kind: "radio", id: seed.id, title };
  if (seed.artist) ctx.artist = seed.artist;
  return ctx;
}

/**
 * The titles of `incoming` that are not in `existing` yet, by id or by
 * recording. The server already excludes what the caller sends, but it also
 * caps that list, and a fallback answer can repeat a row the search gave.
 */
export function mergeRadioTracks(existing = [], incoming = []) {
  const seen = new Set();
  const has = (t) => {
    const keys = trackKeys(t);
    return !keys.filter(Boolean).length || addKeys(seen, keys);
  };
  for (const t of existing) has(t);
  const out = [];
  for (const t of incoming) {
    if (!t) continue;
    const keys = trackKeys(t);
    if (!keys.filter(Boolean).length) continue;
    if (has(t)) continue;
    out.push(t);
  }
  return out;
}

// Player.svelte asks /api/radio/next at once for a queue of three titles or
// fewer (its buffer rule is `index < length - 3`), so the launch must leave
// those to it: two calls for one press would be exactly the request fan-out
// this project treats as a bug. `deferShortQueueToPlayer` is how the screen
// says that Player is mounted; the pure tests and any other caller keep the
// launcher self-sufficient.
const PLAYER_EAGER_MAX = 3;
// A double click can land after the first launch already answered (a local
// server answers in a few milliseconds). The radio is playing by then, so a
// second press inside this window is the same press: it returns the first
// result instead of replacing the queue.
const CLICK_COOLDOWN = 600;
// The default player state for a caller that does not wire one (the pure
// tests): a stable, idle player, so a launch is never seen as superseded.
const DEFAULT_STATE = { context: null, queue: [], autoplay: true };

/**
 * Build the one launch action of the screen. All effects are injected so the
 * request count, the single play, and the double-click guard are testable:
 *
 *   play(tracks, start, context)  replace the queue now (player.playQueue)
 *   extend(tracks)                append what the top-up returned
 *   radioNext(seed, opts)         POST /api/radio/next
 *   toast(message, kind)          a verdict the person can read
 *   state()                       { context, queue, autoplay } from the player,
 *                                 to drop an answer that a later playback replaced
 *
 * A promise guard, not a boolean read at the next render: two click events in
 * the same tick must share one launch, and they do because `pending` is set
 * before any await.
 */
export function createRadioLauncher({
  play,
  extend,
  radioNext,
  toast = () => {},
  state = () => DEFAULT_STATE,
  target = RADIO_TARGET,
  deferShortQueueToPlayer = false,
} = {}) {
  let pending = null; // { term, promise } — the launch the button is waiting on
  let generation = 0; // a newer search's radio supersedes the previous one
  let last = null; // { at, query, result }: what the click that is still "now" produced

  async function run(query, results, youtube, alive = () => true) {
    const term = String(query || "").trim();
    if (!term) {
      toast("Tape une recherche avant de lancer la radio.", "error");
      return { ok: false, reason: "empty-query", queue: [], requests: 0 };
    }
    const tracks = relevantTracks(term, results, youtube);
    const seed = radioSeed(term, results, tracks);
    if (!seed) {
      toast("La radio n'a rien à jouer pour cette recherche.", "error");
      return { ok: false, reason: "no-seed", queue: [], requests: 0 };
    }
    const context = radioContext(term, seed);
    const before = state() || DEFAULT_STATE;
    let queue = tracks.slice();
    let started = false;
    if (queue.length) {
      play(queue, 0, context); // the first title plays NOW, no request before it
      started = true;
    }
    // What the player must still BE when the answer lands: a started radio is
    // identified by its own context (a new playback replaces it); a launch
    // that did not play yet is identified by the queue that was playing, so a
    // tap elsewhere in the meantime is not overwritten by a late answer.
    const token = started
      ? { started: true, context: (state() || DEFAULT_STATE).context }
      : { started: false, queue: before.queue, context: before.context };
    const stillMine = () => {
      const now = state() || DEFAULT_STATE;
      if (!now.autoplay) return false;
      if (token.started) return now.context === token.context;
      return now.queue === token.queue && now.context === token.context;
    };

    const shortForPlayer =
      deferShortQueueToPlayer && queue.length > 0 && queue.length <= PLAYER_EAGER_MAX;
    if (queue.length >= target || shortForPlayer) {
      return { ok: true, started, queue, requests: 0, added: 0 };
    }

    try {
      const r = await radioNext(seed, {
        exclude: queue.map(trackRef).filter(Boolean),
      });
      // A newer search started a radio while this answer was on its way, or
      // another playback replaced this one: its queue is the one on screen, so
      // this answer must not extend it (or toast about it) behind the
      // person's back.
      if (!alive() || !stillMine()) {
        return { ok: false, cancelled: true, queue, requests: 1, added: 0 };
      }
      const add = mergeRadioTracks(queue, r?.tracks || []);
      if (!started) {
        if (!add.length) {
          toast("La radio n'a trouvé aucun titre à jouer.", "error");
          return { ok: false, reason: "empty-radio", queue: [], requests: 1, added: 0 };
        }
        play(add, 0, context);
        queue = add;
        started = true;
      } else if (add.length) {
        extend(add);
        queue = [...queue, ...add];
      } else {
        toast("La radio n'a pas trouvé d'autres titres.", "info");
      }
      return { ok: started, started, queue, requests: 1, added: add.length };
    } catch {
      if (!alive() || !stillMine()) {
        return { ok: false, cancelled: true, queue, requests: 1, added: 0 };
      }
      if (started) {
        toast("La radio a démarré, mais la suite n'a pas pu être chargée.", "error");
      } else {
        toast("La radio n'a pas pu démarrer. Réessayer.", "error");
      }
      return { ok: started, started, queue, requests: 1, added: 0, error: true };
    }
  }

  return {
    /** One launch per click burst; the second click joins the first promise. */
    launch(query, results, youtube = []) {
      const term = String(query || "").trim();
      if (pending && pending.term === term) return pending.promise;
      // The first launch already answered and the radio is playing: a click
      // that lands inside the double-click window is the same press. A new
      // search is a new intent and is never held back.
      if (last && last.query === term && Date.now() - last.at < CLICK_COOLDOWN) {
        return Promise.resolve(last.result);
      }
      const mine = ++generation;
      const promise = run(query, results, youtube, () => mine === generation)
        .then((result) => {
          if (result.ok) last = { at: Date.now(), query: term, result };
          return result;
        })
        .finally(() => {
          if (pending && pending.promise === promise) pending = null;
        });
      pending = { term, promise };
      return promise;
    },
    get busy() {
      return !!pending;
    },
  };
}
