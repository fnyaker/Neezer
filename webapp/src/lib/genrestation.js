// Genre stations, the pure part: the playback context a started station gets,
// which rows of the answer may be handed to the player, and the shape of the
// "all stations" list.
//
// Kept out of the Svelte component so the rules (what a station is called,
// what is playable, how a style search folds accents and spaces) are pinned by
// a plain `node --test`, with no browser, no store and no request.

import { flat } from "./genretree.js";

export const STATION_KIND = "genre-radio";
export const STATION_PREFIX = "Station ";

/** The queue context of a station: its OWN kind, the style as the server's
 *  canonical node (`id`), and a title a person reads. `null` when neither the
 *  asked style nor the answer names one — the caller then has nothing to play. */
export function stationContext(style, payload) {
  const asked = String(style || "").trim();
  const canonical =
    typeof payload?.style === "string" && payload.style.trim() ? payload.style.trim() : asked;
  const name =
    typeof payload?.name === "string" && payload.name.trim() ? payload.name.trim() : canonical;
  if (!canonical) return null;
  return { kind: STATION_KIND, id: canonical, title: STATION_PREFIX + name };
}

/** The rows the player can stream: a catalogue row has a provider id, a
 *  library row its own. A station from a local library is all library rows. */
export function stationTracks(payload) {
  const rows = Array.isArray(payload?.tracks) ? payload.tracks : [];
  return rows.filter((t) => t && (t.deezer_id || t.id));
}

/** `GET /api/flow/styles` -> `[{name, parent, branch, tracks}]`, junk dropped.
 *  `branch` is the genre and everything under it, `tracks` its own titles. */
export function stylesOf(payload) {
  const rows = Array.isArray(payload?.styles) ? payload.styles : [];
  const out = [];
  const seen = new Set();
  for (const s of rows) {
    if (!s || typeof s.name !== "string" || !s.name.trim()) continue;
    const name = s.name.trim();
    // A kept copy from an older build can hold a row twice; a keyed list
    // cannot, so the first reading wins.
    if (seen.has(name)) continue;
    seen.add(name);
    const branch = Number(s.branch);
    const tracks = Number(s.tracks);
    out.push({
      name,
      parent: typeof s.parent === "string" && s.parent.trim() ? s.parent.trim() : null,
      branch: Number.isFinite(branch) ? branch : 0,
      tracks: Number.isFinite(tracks) ? tracks : 0,
    });
  }
  return out;
}

/** The styles whose name or parent holds what was typed, accents and spacing
 *  set aside: "electro" finds "Électro", "disco funk" finds "Disco / funk". */
export function filterStyles(rows, query) {
  const q = flat(query);
  if (!q) return rows;
  return rows.filter((r) => flat(r.name).includes(q) || flat(r.parent || "").includes(q));
}
