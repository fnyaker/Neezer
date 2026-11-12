// How the search's "Tout" tab is put together (design: /mnt/project-files/ui/
// refonte-navigation.md, *Recherche*). Pure functions, so the order is tested
// without a browser.
//
//   topResult(q, results, withPodcasts)  the ONE artist (or show) the query
//       plainly names, shown first and large — or null. Only a clear match
//       earns the place: a top card that is not what was typed pushes the
//       titles down for nothing.
//   mixPodcasts(albums, podcasts)        the shows among the albums, with
//       `podcast: true`: no category of their own, a small label instead.
//   matchScore(q, track)                 how much of what was typed a title and
//       its artist account for, 0..1. The YouTube fallback uses it with a
//       HIGHER bar than the catalogue (YOUTUBE_MIN > CATALOGUE_MIN): a video
//       is only worth promoting when it is closer to the query than the best
//       Deezer answer, never because it is simply another result.
//   rankSearchResults(q, catalogue, youtube)  the catalogue's results (library
//       rows first, in the order the server gave) with the fallback's hits
//       woven into the SAME list: promoted ones just after the library, the
//       rest after the catalogue, exact duplicates dropped.

import { fold } from "./genre/rank.js";

export { fold };

/** @returns {{kind: "artist"|"podcast", item}|null} */
export function topResult(q, results, withPodcasts = false) {
  const artists = results?.artists || [];
  const exact = fold(q);
  if (!exact) return null;
  // An artist named exactly, among the first few (the catalogue lists them
  // by popularity: the third "Indila" is a tribute act, the first is not).
  const same = artists.slice(0, 3).find((a) => fold(a.name) === exact);
  if (same) return { kind: "artist", item: same };
  if (withPodcasts) {
    const show = (results?.podcasts || []).slice(0, 3).find((p) => fold(p.title) === exact);
    if (show) return { kind: "podcast", item: show };
  }
  // Else only the FIRST artist, and only when the query is the start of its
  // name: what somebody types before they have finished typing it.
  const first = artists[0];
  if (first && exact.length >= 3 && startsName(exact, fold(first.name))) {
    return { kind: "artist", item: first };
  }
  return null;
}

function startsName(a, b) {
  // Folded names are words joined by single spaces, so a prefix of the whole
  // name starts on a word: "daft" and "daft p" name Daft Punk, "aft" does not.
  return b.startsWith(a);
}

/** The albums with the shows mixed in after the first two: what matched best
 *  stays first, and a show the query names is not buried at the end. */
export function mixPodcasts(albums, podcasts, after = 2) {
  const a = albums || [];
  const p = (podcasts || []).map((x) => ({ ...x, podcast: true }));
  if (!p.length) return a;
  return [...a.slice(0, after), ...p, ...a.slice(after)];
}

// -- how close a hit is to what was typed -------------------------------------------
//
// A score is a number, not a verdict: rankSearchResults is where the two bars
// live. The catalogue's answer is judged at CATALOGUE_MIN (a one-word
// coincidence is not a "comparable result" that should stop a better video);
// a YouTube hit must clear YOUTUBE_MIN, and only LEADS when it is strictly
// closer than the best catalogue answer. Equal is not enough: the catalogue
// result is the one the person expects, and a duplicate video is noise.

/** The main source's bar: below it, a catalogue row is not a comparable match. */
export const CATALOGUE_MIN = 0.5;
/** YouTube's bar: a video must be a real match, not just a word in its title. */
export const YOUTUBE_MIN = 0.75;

// Words that name a DIFFERENT recording of a title: a plain query does not
// ask for them, so a video carrying one is demoted below the studio version.
const VARIANT = new Set([
  "cover", "live", "remix", "remixed", "mix", "edit", "version", "instrumental",
  "karaoke", "tribute", "acoustic", "demo", "reprise", "extended", "radio",
  "sped", "slowed", "nightcore", "bootleg", "mashup", "reaction", "8d",
  "orchestral", "piano", "a-capella", "acapella",
]);
const VARIANT_RE = /\b(cover|live|remix|remixed|instrumental|karaoke|tribute|acoustic|demo|reprise|extended|sped|slowed|nightcore|bootleg|mashup|reaction|orchestral|acapella|a-capella)\b/i;
// Decorations that do NOT change which recording it is: an official clip, a
// lyric video, an audio upload. They are stripped before comparing two rows,
// so the same song on Deezer and on YouTube is ONE line.
const COSMETIC_RE = /\b(official|officiel|officielle|clip|video|vid[ée]o|audio|lyrics?|paroles|visuali[sz]er|hd|hq|4k|mv)\b/i;

/** The name a track is credited to (the full credit line when there is one). */
export function artistOf(track) {
  const t = track || {};
  const listed = Array.isArray(t.artists)
    ? t.artists.map((a) => a?.name).filter(Boolean).join(", ")
    : "";
  return String(t.display_artist || t.artist?.name || listed || "");
}

const wordsOf = (s) => fold(s).split(" ").filter(Boolean);
const setOf = (s) => new Set(wordsOf(s));

/** F1 of two word sets: both "what was typed is there" and "nothing extra". */
function overlapF1(a, b) {
  if (!a.size || !b.size) return 0;
  let hit = 0;
  for (const w of a) if (b.has(w)) hit++;
  if (!hit) return 0;
  const precision = hit / b.size;
  const recall = hit / a.size;
  return (2 * precision * recall) / (precision + recall);
}

/** A version marker the query did not ask for (live, remix, cover…) demotes a
 *  title. Capped at three steps so a pile of suffixes cannot reach zero. */
function variantPenalty(titleWords, asked) {
  let extra = 0;
  for (const w of titleWords) if (VARIANT.has(w) && !asked.has(w)) extra++;
  return extra ? Math.pow(0.72, Math.min(extra, 3)) : 1;
}

/**
 * How much of `query` a track's title and artist account for, 0..1.
 *
 * Half is coverage (every word typed is found somewhere in the credit), half
 * is the title's F1 against the words the artist's name does not explain —
 * so "Daft Punk" does not count against a title query, and a title with words
 * nobody typed ("(Live)", "Remix") is not as close as the plain one.
 */
export function matchScore(query, track) {
  const asked = setOf(query);
  if (!asked.size) return 0;
  const titleWords = wordsOf(track?.title);
  const title = new Set(titleWords);
  const artist = setOf(artistOf(track));
  // When the artist's name explains every word typed (an artist query), the
  // title is not expected to match it; otherwise judge the remaining words.
  const titleAsked = new Set([...asked].filter((w) => !artist.has(w)));
  const titleScore = titleAsked.size ? overlapF1(titleAsked, title) : 1;
  let covered = 0;
  for (const w of asked) if (title.has(w) || artist.has(w)) covered++;
  const coverage = covered / asked.size;
  const score = (coverage + titleScore) / 2;
  return Math.max(0, Math.min(1, score * variantPenalty(titleWords, asked)));
}

/** A title without its cosmetic decorations ("(Clip officiel)", "(Audio)"),
 *  so two uploads of the same recording compare equal. A LIVE or REMIX marker
 *  is NOT removed: it names another recording on purpose. */
function canonicalTitle(track) {
  const raw = String(track?.title || "");
  const kept = raw.replace(/[\(\[]([^\)\]]*)[\)\]]/g, (all, inside) =>
    COSMETIC_RE.test(inside) && !VARIANT_RE.test(inside) ? " " : all,
  );
  return fold(kept);
}

/** The identity of a recording for de-duplication ("" when there is no title). */
export function recordingKey(track) {
  const title = canonicalTitle(track);
  return title ? `${title}\u0000${fold(artistOf(track))}` : "";
}

export function sameRecording(a, b) {
  const ka = recordingKey(a);
  return !!ka && ka === recordingKey(b);
}

/**
 * The catalogue's results with the YouTube hits in the SAME list.
 *
 * The catalogue's own order is preserved and library rows (`local`) stay
 * first. A YouTube hit is dropped when it duplicates a catalogue row (same
 * id, or the same recording once decorations are stripped) or when its score
 * is under YOUTUBE_MIN. A surviving hit that is STRICTLY closer than the best
 * comparable catalogue answer is promoted to just after the library rows; the
 * others follow the catalogue (Deezer wins a tie).
 *
 * @returns {{tracks: any[], promoted: number, appended: number, dropped: number}}
 */
export function rankSearchResults(query, catalogue = [], youtube = []) {
  const cat = Array.isArray(catalogue) ? catalogue : [];
  const videos = Array.isArray(youtube) ? youtube : [];
  const library = cat.filter((t) => t && t.local === true);
  const rest = cat.filter((t) => t && t.local !== true);

  let best = 0;
  const seen = new Set();
  const ids = new Set();
  for (const t of cat) {
    const score = matchScore(query, t);
    if (score >= CATALOGUE_MIN && score > best) best = score;
    const key = recordingKey(t);
    if (key) seen.add(key);
    if (t?.deezer_id) ids.add(String(t.deezer_id));
  }

  const kept = [];
  let dropped = 0;
  for (const t of videos) {
    if (t?.deezer_id && ids.has(String(t.deezer_id))) {
      dropped++;
      continue;
    }
    const key = recordingKey(t);
    if (key && seen.has(key)) {
      dropped++;
      continue;
    }
    const score = matchScore(query, t);
    if (score < YOUTUBE_MIN) {
      dropped++;
      continue;
    }
    if (key) seen.add(key);
    kept.push({ track: t, score });
  }
  kept.sort((a, b) => b.score - a.score);

  const promoted = [];
  const appended = [];
  for (const hit of kept) (hit.score > best ? promoted : appended).push(hit.track);
  return {
    tracks: [...library, ...promoted, ...rest, ...appended],
    promoted: promoted.length,
    appended: appended.length,
    dropped,
  };
}
