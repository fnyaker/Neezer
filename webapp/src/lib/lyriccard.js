// Lyric-card rules, without a DOM.
//
// The card itself is drawn on a canvas (components/LyricCard.svelte), but the
// decisions a person can feel — which line is selected by default, what a tap
// does at the limit, where a long line breaks, what the file is called — are
// pure and live here so the node suite can hold them. Nothing in this module
// touches window, canvas or the network.
//
// The card is 4:5 (1080 x 1350). Instagram's documented feed range is 1.91:1 to
// 4:5 (1080 px wide, 566-1440 px tall), and 1080x1350 is the portrait size that
// uses the whole feed without a crop; a square loses about 12% of each side in
// the 3:4 profile grid (Instagram Help, "Image resolution of photos you share";
// Buffer's 2026 Instagram size guide). It also leaves room for three lyric
// lines plus the title/artist block at a readable size.

export const CARD_W = 1080;
export const CARD_H = 1350;
export const MAX_LINES = 3;

// Largest first: fitLyricLayout walks down until the block fits. The small end
// matters: three selected lines can each be a full lyric line, and the card
// must show them whole rather than clip the last one behind the title block.
export const LYRIC_SIZES = [96, 88, 80, 72, 64, 56, 48, 40, 34, 30, 26];

// The lines a card can offer, from either lyric shape:
//   { synced: [{ time, text }], text }  — the API's two forms
//   { text: "…" }                        — plain, unsynced
//
// A synced line with no words is an instrumental marker ("• • •" in the panel),
// not something anybody wants on a card: it is skipped, and `source` keeps the
// index in the ORIGINAL array so the active line still resolves (a blank marker
// between two verses must not shift the selection by one).
export function lyricLines(lyrics) {
  const out = [];
  const synced = lyrics && Array.isArray(lyrics.synced) ? lyrics.synced : null;
  if (synced && synced.length) {
    synced.forEach((line, i) => {
      const text = String((line && line.text) || "").trim();
      // A marker is punctuation or a note glyph, never words: the panel shows
      // "• • •" for it, and it must not become a selectable empty card.
      if (!text || !/[\p{L}\p{N}]/u.test(text)) return;
      const time = Number(line && line.time);
      out.push({ text, time: Number.isFinite(time) ? time : null, source: i });
    });
    return out;
  }
  const text = lyrics && typeof lyrics.text === "string" ? lyrics.text : "";
  text.split(/\r?\n/).forEach((line, i) => {
    const trimmed = line.trim();
    // Same marker rule as a synced list: "• • •" or "♪" is not a lyric.
    if (trimmed && /[\p{L}\p{N}]/u.test(trimmed)) out.push({ text: trimmed, time: null, source: i });
  });
  return out;
}

// Which line the card opens on: the one being heard, else the first.
//
// `activeSource` is the index in the original synced array (-1 while nothing
// plays or for plain lyrics). When it names a line we skipped (an instrumental
// marker) the nearest line BEFORE it is used — that is the words just sung.
export function defaultLineIndex(lines, activeSource = -1) {
  if (!lines || !lines.length) return -1;
  const at = Number(activeSource);
  if (!Number.isFinite(at) || at < 0) return 0;
  let before = 0;
  for (let i = 0; i < lines.length; i++) {
    if (lines[i].source === at) return i;
    if (lines[i].source < at) before = i;
    else break;
  }
  return before;
}

// One tap on a line. Returns the new selection (sorted in lyric order) and why
// nothing changed when the selection is at a limit.
//
//   min  the last selected line was tapped: a card always says something
//   max  a fourth line was tapped
//
// The card keeps any 1-3 lines, not only consecutive ones: a person building a
// card often wants two separate lines from a chorus, and the preview makes the
// result unambiguous.
export function toggleLine(selected, index, max = MAX_LINES) {
  const values = Array.isArray(selected) ? selected : [];
  const cur = [...new Set(values.filter((i) => Number.isInteger(i) && i >= 0))].sort((a, b) => a - b);
  if (!Number.isInteger(index) || index < 0) return { indices: cur, reason: null };
  if (cur.includes(index)) {
    if (cur.length <= 1) return { indices: cur, reason: "min" };
    return { indices: cur.filter((i) => i !== index), reason: null };
  }
  if (cur.length >= max) return { indices: cur, reason: "max" };
  return { indices: [...cur, index].sort((a, b) => a - b), reason: null };
}

// The text of the selected lines, in LYRIC order (never tap order), capped at
// three and ignoring blanks/out-of-range indices.
export function chooseLines(lines, selected, max = MAX_LINES) {
  if (!Array.isArray(lines) || !Array.isArray(selected)) return [];
  const picked = [...new Set(selected)]
    .filter((i) => Number.isInteger(i) && i >= 0 && i < lines.length)
    .sort((a, b) => a - b)
    .slice(0, max);
  return picked.map((i) => lines[i].text).filter(Boolean);
}

function widthOf(measure, text) {
  const m = measure && typeof measure.measureText === "function" ? measure.measureText(text) : null;
  return m && Number.isFinite(m.width) ? m.width : 0;
}

// Line-break `text` to `maxWidth` with a canvas-like `measure` context.
//
// Pure in the sense that matters: it reads measureText and nothing else, so a
// four-line fake in the tests exercises the same code the canvas does. A word
// wider than the column is cut by character rather than allowed to overflow
// (a pasted URL, a long German compound, a French word the font renders wide);
// hard newlines are kept as hard breaks, blank paragraphs are dropped.
export function wrapText(measure, text, maxWidth) {
  const limit = Number(maxWidth);
  if (!Number.isFinite(limit) || limit <= 0) return [];
  const src = String(text == null ? "" : text);
  if (!src.trim()) return [];
  const out = [];
  for (const paragraph of src.split(/\r?\n/)) {
    const words = paragraph.trim().split(/\s+/).filter(Boolean);
    if (!words.length) continue;
    let line = "";
    for (let word of words) {
      // A single token wider than the column: emit the longest prefix that fits,
      // one piece at a time. A lone glyph wider than the column still goes out
      // on its own line (progress, not an infinite loop).
      while (widthOf(measure, word) > limit && word.length > 1) {
        let cut = word.length;
        while (cut > 1 && widthOf(measure, word.slice(0, cut)) > limit) cut--;
        if (cut < 1) cut = 1;
        if (line) {
          out.push(line);
          line = "";
        }
        out.push(word.slice(0, cut));
        word = word.slice(cut);
      }
      if (!word) continue;
      const next = line ? line + " " + word : word;
      if (!line || widthOf(measure, next) <= limit) {
        line = next;
      } else {
        out.push(line);
        line = word;
      }
    }
    if (line) out.push(line);
  }
  return out;
}

// `text` on one line, ellipsised when it does not fit.
export function fitOneLine(measure, text, maxWidth) {
  const limit = Number(maxWidth);
  const src = String(text == null ? "" : text).trim();
  if (!src) return "";
  if (!Number.isFinite(limit) || limit <= 0) return "";
  if (widthOf(measure, src) <= limit) return src;
  let cut = src.length;
  while (cut > 0 && widthOf(measure, src.slice(0, cut).trimEnd() + "…") > limit) cut--;
  return cut ? src.slice(0, cut).trimEnd() + "…" : "…";
}

// The largest lyric size whose wrapped rows fit the block.
//
// `measureAt(size)` returns a canvas-like context already set to that font —
// canvas text metrics depend on the font, so a single measure cannot answer for
// every size. `blocks` keeps each selected lyric line whole: the drawing code
// puts a small gap between blocks, so three chosen lines read as three lines and
// not as one long paragraph. Returns the smallest size when even that overflows
// (the drawing code clips to the block rather than pushing into the title).
export function fitLyricLayout(measureAt, texts, opts = {}) {
  const sizes = Array.isArray(opts.sizes) && opts.sizes.length ? opts.sizes : LYRIC_SIZES;
  const maxWidth = Number(opts.maxWidth) || CARD_W - 192;
  const maxHeight = Number(opts.maxHeight) || 650;
  const lineHeight = Number(opts.lineHeight) || 1.24;
  const paragraphGap = Number(opts.paragraphGap) || 0.14;
  const list = (Array.isArray(texts) ? texts : []).map((t) => String(t == null ? "" : t).trim()).filter(Boolean);
  if (!list.length) return { size: sizes[0], rows: [], blocks: [] };
  let fallback = null;
  for (const size of sizes) {
    const measure = measureAt(size);
    const blocks = [];
    for (const text of list) {
      const block = wrapText(measure, text, maxWidth);
      if (block.length) blocks.push(block);
    }
    const rows = blocks.flat();
    const height = rows.length * size * lineHeight + Math.max(0, blocks.length - 1) * size * paragraphGap;
    if (height <= maxHeight) return { size, rows, blocks };
    fallback = { size, rows, blocks };
  }
  return fallback || { size: sizes[sizes.length - 1], rows: [], blocks: [] };
}

// A filename anything can accept: `neezer-<artist>-<title>.png`.
//
// Neezer's own prefix, the two names as the person reads them, lowercased and
// with diacritics folded (Beyoncé -> beyonce) so the file survives every
// filesystem, mail client and messaging app. Path separators, reserved
// characters and control characters become hyphens; runs collapse; the parts
// are bounded so a 300-character title cannot produce an unopenable name.
export function cardFilename(artist, title) {
  const slug = (value, limit) =>
    String(value == null ? "" : value)
      .normalize("NFKD")
      .replace(/\p{M}+/gu, "")
      .replace(/[\\/:*?"<>|]+/g, " ")
      // Invisible control characters are removed, not turned into a word
      // boundary: a corrupted byte inside "Titre" must not become "t-itre".
      // eslint-disable-next-line no-control-regex
      .replace(/[\u0000-\u001f\u007f]+/g, "")
      .replace(/[^\p{L}\p{N}]+/gu, "-")
      .replace(/^-+|-+$/g, "")
      .toLowerCase()
      .slice(0, limit)
      .replace(/-+$/g, "");
  const a = slug(artist, 40);
  const t = slug(title, 64) || "titre";
  return `neezer-${a ? a + "-" : ""}${t}.png`;
}
