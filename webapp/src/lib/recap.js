// Récap rules, without a DOM.
//
// The screen (routes/Recap.svelte and its three components) reads its CARD
// ORDER, its copy, its numbers and its poster lines from HERE. One year's story
// must not be told two ways by two files: a component formats nothing itself,
// it calls one of these functions. Nothing in this module touches window, a
// canvas, a store or the network, so `test/recap.test.mjs` holds it without a
// browser.
//
// Sources of the decisions (plan `recap.md` §2, §6, §7, §11):
// - Spotify Wrapped (Newsroom, "The Simple Truth About How Your Wrapped Comes
//   to Life", 2025-12) for the one-idea-per-card story, minutes-heard
//   accounting and a shareable picture per card;
// - Apple Music Replay for an ever-running, explicitly PARTIAL year;
// - the plan's §7 copy is the contract: the strings below are tested to the
//   character and must not be reworded in a component.

const TOP = 5;
const MOSAIC = 4;
const POSTER_LINE_MAX = 38;

// The cards a payload may name, in the fixed order of the plan (§2.3). A card
// absent from `payload.cards` is NOT rendered: the client never invents one.
const KNOWN_CARDS = [
  "year",
  "minutes",
  "artists",
  "tracks",
  "albums",
  "genres",
  "streak",
  "firstlast",
];

const MONTHS = [
  "janvier",
  "février",
  "mars",
  "avril",
  "mai",
  "juin",
  "juillet",
  "août",
  "septembre",
  "octobre",
  "novembre",
  "décembre",
];

// 12 bars on a 360 px phone: three letters is the widest that stays legible
// under a 20 px column, and "juil." is the French typographic abbreviation.
const MONTHS_SHORT = [
  "janv.",
  "févr.",
  "mars",
  "avr.",
  "mai",
  "juin",
  "juil.",
  "août",
  "sept.",
  "oct.",
  "nov.",
  "déc.",
];

// A narrow no-break space (U+202F) is the French thousands separator, and it
// must not break across a line: `Intl` would be right on a full ICU, but the
// output of this function is compared to literals in the node suite and drawn
// on a canvas, so the separator is written here once, explicitly.
export function formatNumber(n) {
  const num = Number(n);
  const value = Number.isFinite(num) ? Math.trunc(num) : 0;
  const sign = value < 0 ? "-" : "";
  return sign + String(Math.abs(value)).replace(/\B(?=(\d{3})+(?!\d))/g, "\u202f");
}

// "8 540 minutes" / "1 minute" / "0 minute" — French counts zero as singular.
export function formatMinutes(n) {
  const num = Number(n);
  const value = Number.isFinite(num) ? Math.trunc(num) : 0;
  return `${formatNumber(value)} minute${value > 1 ? "s" : ""}`;
}

// A span of listened time, at the precision that reads: minutes under an hour,
// hours and minutes under a day, days and hours above. "5 j 22 h" is the
// plan's literal; abbreviations are invariant in French (never "5 js").
export function formatSpan(seconds) {
  const num = Number(seconds);
  const total = Number.isFinite(num) ? Math.max(0, Math.round(num)) : 0;
  if (total < 60) return "0 min";
  if (total < 3600) return `${Math.floor(total / 60)} min`;
  if (total < 86400) {
    const h = Math.floor(total / 3600);
    const m = Math.floor((total % 3600) / 60);
    return m ? `${h} h ${String(m).padStart(2, "0")}` : `${h} h`;
  }
  const d = Math.floor(total / 86400);
  const h = Math.floor((total % 86400) / 3600);
  return h ? `${d} j ${h} h` : `${d} j`;
}

// The server's order, filtered to the ids this screen knows, then the client's
// own `share` card. A deck with no card at all is not a deck: `share` is only
// appended to a real story, never shown alone on an empty year.
export function cardIds(payload) {
  if (!payload) return [];
  const out = [];
  const seen = new Set();
  const list = Array.isArray(payload.cards) ? payload.cards : [];
  for (const id of list) {
    if (!KNOWN_CARDS.includes(id) || seen.has(id)) continue;
    seen.add(id);
    out.push(id);
  }
  if (payload.enough && out.length) out.push("share");
  return out;
}

// The big line of a card. Exact §7 copy.
export function slideTitle(payload, id) {
  const year = Number(payload && payload.year);
  switch (id) {
    case "year":
      return `Ton année ${year}`;
    case "minutes":
      return formatMinutes(payload && payload.totals && payload.totals.minutes);
    case "artists":
      return "Tes 5 artistes";
    case "tracks":
      return "Tes 5 titres";
    case "albums":
      return "Tes 5 albums";
    case "genres":
      return "Tes styles";
    case "streak": {
      const days = Math.max(0, Math.trunc(Number(payload && payload.streak && payload.streak.days) || 0));
      return `${formatNumber(days)} jour${days > 1 ? "s" : ""} d'affilée`;
    }
    case "firstlast":
      return "Ton premier titre de l'année";
    case "share":
      return "Partage ton année";
    default:
      return "";
  }
}

// The small mono line above the title. The opening card names its year (and
// says when it is still running) because the position adds nothing there; every
// other card carries the reader's place in the story, which is also what the
// screen reader hears. `state.nowYear` is never compared here: this is the
// payload's own `partial` flag.
export function slideKicker(payload, id) {
  const ids = cardIds(payload);
  const at = ids.indexOf(id);
  if (at < 0) return "";
  if (id === "year") {
    const year = Number(payload && payload.year);
    return payload && payload.partial ? `${year} · en cours` : String(year);
  }
  return `${at + 1} sur ${ids.length}`;
}

function isoDay(value) {
  const match = /^(\d{4})-(\d{2})-(\d{2})/.exec(String(value == null ? "" : value));
  if (!match) return null;
  const y = Number(match[1]);
  const mo = Number(match[2]);
  const d = Number(match[3]);
  if (mo < 1 || mo > 12 || d < 1 || d > 31) return null;
  return { y, mo, d };
}

// "1ᵉʳ janvier", "13 novembre": the article-less day a French date uses.
function dayMonth(day) {
  return `${day.d === 1 ? "1ᵉʳ" : day.d} ${MONTHS[day.mo - 1]}`;
}

// The year's display period: a finished year is the whole year; the running
// one is what has actually happened so far. The year is carried once, at the
// end, so both forms stay unambiguous.
export function periodLabel(payload) {
  const year = Number(payload && payload.year);
  const label = Number.isInteger(year) ? String(year) : "";
  const from = isoDay(payload && payload.period && payload.period.from);
  const to = isoDay(payload && payload.period && payload.period.to);
  if (!payload || !payload.partial || !from || !to) return `toute l'année ${label}`.trim();
  return `du ${dayMonth(from)} au ${dayMonth(to)} ${to.y}`;
}

// 12 bars, always, zero-filled and in calendar order. `pct` is normalised to
// the busiest month so the tallest bar is exactly 100; an empty year is 12
// flat bars (never NaN, never a division by zero).
export function monthBars(payload) {
  const listens = new Map();
  const rows = payload && Array.isArray(payload.months) ? payload.months : [];
  for (const row of rows) {
    const month = Number(row && row.month);
    if (!Number.isInteger(month) || month < 1 || month > 12) continue;
    listens.set(month, Math.max(0, Math.trunc(Number(row.listens) || 0)));
  }
  const values = Array.from({ length: 12 }, (_, i) => listens.get(i + 1) || 0);
  const max = Math.max(0, ...values);
  return values.map((value, i) => ({
    month: i + 1,
    label: MONTHS_SHORT[i],
    listens: value,
    pct: max > 0 ? Math.round((value / max) * 100) : 0,
  }));
}

// The busiest month, named in full: "Ton mois le plus fort : juillet". Ties go
// to the earliest month, which is the one a person saw first in the year.
export function busiestMonth(payload) {
  const bars = monthBars(payload);
  let best = null;
  for (const bar of bars) {
    if (bar.listens > 0 && (!best || bar.listens > best.listens)) best = bar;
  }
  return best ? { label: MONTHS[best.month - 1], listens: best.listens } : null;
}

// The rows of a top, in the server's order, at most five. No row is ever
// invented, no missing field is filled in: `[]` is an honest answer.
export function rankRows(payload, kind) {
  if (kind !== "tracks" && kind !== "artists" && kind !== "albums") return [];
  const list = payload && payload.tops && payload.tops[kind];
  if (!Array.isArray(list)) return [];
  const out = [];
  for (const row of list) {
    if (!row || typeof row !== "object") continue;
    out.push(row);
    if (out.length >= TOP) break;
  }
  return out;
}

// The styles card's lines: nothing at all when the server could not vouch for
// the measurement (`available` false), so a weak year never gets an invented
// "éclectique".
export function genreRows(payload) {
  const genres = payload && payload.tops && payload.tops.genres;
  if (!genres || !genres.available || !Array.isArray(genres.rows)) return [];
  return genres.rows.filter((row) => row && row.name).slice(0, TOP);
}

// Up to four sleeves for the opening card and the poster: albums first (they
// read as records), then the top titles' album art. Unique URLs only, and
// nothing when the year holds no picture — a missing tile is drawn as Sillon's
// groove by the caller, never as an empty square.
export function mosaicCovers(payload) {
  const out = [];
  const seen = new Set();
  const push = (url) => {
    if (typeof url !== "string" || !url || seen.has(url) || out.length >= MOSAIC) return;
    seen.add(url);
    out.push(url);
  };
  for (const row of (payload && payload.tops && payload.tops.albums) || []) {
    if (row && typeof row === "object") push(row.cover);
  }
  for (const row of (payload && payload.tops && payload.tops.tracks) || []) {
    const album = row && row.track && row.track.album;
    if (album) push(album.cover);
  }
  return out;
}

// The year selector: one chip per year that holds at least one event, newest
// first. Only the server's `nowYear` may say "en cours" — a phone whose clock
// is a day ahead must not relabel the year.
export function yearChips(state) {
  const rows = state && Array.isArray(state.years) ? state.years : [];
  const nowYear = Number(state && (state.nowYear != null ? state.nowYear : state.now_year));
  const current = Number.isInteger(nowYear) ? nowYear : null;
  const seen = new Set();
  const out = [];
  for (const row of rows) {
    const year = Number(row && row.year);
    if (!Number.isInteger(year) || seen.has(year)) continue;
    if (row.events != null && !(Number(row.events) > 0)) continue;
    seen.add(year);
    out.push({
      year,
      label: current != null && year === current ? `${year} · en cours` : String(year),
    });
  }
  return out.sort((a, b) => b.year - a.year);
}

// The season's home card: the most recent year the server judged substantial,
// and only while it is this year or the one before (`nowYear` comes from the
// server). In 2030 there is no eternal "Ton année 2024" banner. Returns the
// year ROW (the card shows its listens and minutes), or null.
export function advertisedYear(state) {
  const rows = state && Array.isArray(state.years) ? state.years : [];
  const nowYear = Number(state && (state.nowYear != null ? state.nowYear : state.now_year));
  let best = null;
  for (const row of rows) {
    const year = Number(row && row.year);
    if (!Number.isInteger(year) || !row.enough) continue;
    if (row.events != null && !(Number(row.events) > 0)) continue;
    if (!best || year > Number(best.year)) best = row;
  }
  if (!best || !Number.isInteger(nowYear)) return null;
  const year = Number(best.year);
  return year === nowYear || year === nowYear - 1 ? best : null;
}

// Why a year has no story. The rule is the person's own data, in this order:
// no event at all; events but not one listen; or listens the server judged too
// few. `null` means the deck has cards.
export function emptyReason(payload) {
  if (!payload) return "none";
  const totals = payload.totals || {};
  const events = Math.max(0, Math.trunc(Number(totals.events) || 0));
  const listens = Math.max(0, Math.trunc(Number(totals.listens) || 0));
  if (events === 0) return "none";
  if (listens === 0) return "skips-only";
  if (payload.enough === false) return "thin";
  return null;
}

// One sentence for the Web Share text fallback. Tested to the character.
export function shareText(payload) {
  const year = Number(payload && payload.year);
  const totals = (payload && payload.totals) || {};
  const listens = Math.max(0, Math.trunc(Number(totals.listens) || 0));
  const artist = rankRows(payload, "artists")[0];
  const who = artist && artist.name ? `, avec ${artist.name} en tête` : "";
  const label = payload && payload.partial ? `Mon année ${year} (en cours)` : `Mon année ${year}`;
  return `${label} en musique : ${formatNumber(listens)} écoute${listens > 1 ? "s" : ""}, ${formatMinutes(
    totals.minutes
  )}${who}.`;
}

// Cut a string to `max` characters, ellipsis included. Whitespace is folded so
// a canvas line never wraps on a newline it was not told about.
export function ellipsis(text, max = POSTER_LINE_MAX) {
  // Only ASCII layout whitespace is folded: the French thousands separator is
  // a narrow no-break space (U+202F) and turning it into a plain space would
  // break "8 540" across two lines on the poster.
  const source = String(text == null ? "" : text)
    .replace(/[ \t\n\r\f\v]+/g, " ")
    .trim();
  const limit = Math.max(1, Math.trunc(Number(max) || POSTER_LINE_MAX));
  if (source.length <= limit) return source;
  if (limit === 1) return "…";
  return source.slice(0, limit - 1).trimEnd() + "…";
}

// The six lines of the 1080×1350 poster, in reading order: the year, the
// minutes, the listens, then the strongest facts the year actually holds. A
// thin year produces fewer lines rather than a padded one. Every line is
// bounded: the canvas draws them one line each.
export function posterLines(payload) {
  const year = Number(payload && payload.year);
  const totals = (payload && payload.totals) || {};
  const lines = [];
  if (Number.isInteger(year)) {
    lines.push(payload && payload.partial ? `Ton année ${year}, en cours` : `Ton année ${year}`);
  }
  lines.push(formatMinutes(totals.minutes));
  const listens = Math.max(0, Math.trunc(Number(totals.listens) || 0));
  lines.push(`${formatNumber(listens)} écoute${listens > 1 ? "s" : ""}`);
  const artist = rankRows(payload, "artists")[0];
  if (artist && artist.name) lines.push(`Artiste : ${artist.name}`);
  const track = rankRows(payload, "tracks")[0];
  if (track && track.track && track.track.title) lines.push(`Titre : ${track.track.title}`);
  const month = busiestMonth(payload);
  if (month) lines.push(`Ton mois le plus fort : ${month.label}`);
  else if (payload) lines.push(periodLabel(payload));
  return lines.slice(0, 6).map((line) => ellipsis(line, POSTER_LINE_MAX));
}
