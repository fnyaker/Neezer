// The years a Récap can open — `GET /api/recap/years` — shared by the two
// entry points (the home's seasonal card and the Récap screen).
//
// It is a tiny answer (one SQL statement, a few hundred bytes) that barely
// changes, so it is kept 10 minutes and requested ONCE per session: opening
// the home then the Récap costs one `/years`, whatever the order. The screen
// and the card both consume the same store, so they cannot disagree about
// which year is "en cours" or which one is worth advertising.
//
// Personal listening data: this is never written to localStorage nor to the
// offline `/api` cache (see lib/apicache.js — `/recap*` is deliberately not a
// cache prefix).
import { writable, get } from "svelte/store";
import { api } from "./api.js";
import { online } from "./net.js";

export const recapYears = writable(null); // null | {years, latest, nowYear, minYear, at}

const TTL = 10 * 60 * 1000;
let at = 0;
let inFlight = null;

/**
 * Fetch the year list once, then serve the kept copy for ten minutes.
 * `force` asks again now (it still shares a request already in flight).
 *
 * Known offline: no request at all — the caller shows its own sentence.
 * Failures are left to the caller (the home ignores them, the screen says
 * why), and never replace a copy already on screen.
 */
export function loadRecapYears({ force = false } = {}) {
  if (!get(online)) return Promise.resolve(get(recapYears));
  if (!force && at && Date.now() - at < TTL) return Promise.resolve(get(recapYears));
  if (inFlight) return inFlight;
  inFlight = api
    .recapYears()
    .then((data) => {
      const state = {
        years: Array.isArray(data && data.years) ? data.years : [],
        latest: data && data.latest != null ? data.latest : null,
        nowYear: data && data.now_year != null ? data.now_year : null,
        minYear: data && data.min_year != null ? data.min_year : null,
        at: Date.now(),
      };
      at = state.at;
      recapYears.set(state);
      return state;
    })
    .finally(() => {
      inFlight = null;
    });
  return inFlight;
}
