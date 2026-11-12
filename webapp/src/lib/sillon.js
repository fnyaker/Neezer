// Sillon's palette, as pure rules: the missing-cover grooves, the avatar
// initials and the WCAG maths the Pochette theme uses to keep its accent
// readable. No imports, no DOM — so the browser, the build and the node tests
// all read the same numbers.

// DESIGN §4: a cover we don't have is a record groove, not an empty square.
// Eight pairs, dark enough that a light glyph and light initials read on them.
export const GROOVES = [
  ["#2b2b29", "#3b3b37"],
  ["#7a1d12", "#9b2a1b"],
  ["#14304a", "#1c4466"],
  ["#4a2a62", "#5e3a7b"],
  ["#1f4a3a", "#2a634e"],
  ["#6b5a14", "#8a7420"],
  ["#5a1f3a", "#7a2c50"],
  ["#3a3a33", "#55554a"],
];

// What Pochette shows when the cover has no colour to give (or there is no
// cover at all). Also the value index.html puts on <html> before the app runs.
export const ACCENT_FALLBACK = "#FF6B4A";

// The Pochette theme sets --onac to this: every accent has to be legible ON it.
export const ON_ACCENT = "#0C0C0B";

// FNV-1a, 32-bit. Chosen because it is four lines, needs no table, and spreads
// adjacent ids ("album-217", "album-218") across the eight grooves instead of
// parking a whole page on one — which is exactly what a modulo over a naive
// char-sum used to do.
export function hashId(seed) {
  let h = 0x811c9dc5;
  const s = String(seed ?? "");
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

// The colour pair for a given seed. Same seed, same pair, on every device and
// every launch: a placeholder that changed colour on reload would read as a
// loading state.
export function grooveFor(seed) {
  return GROOVES[hashId(seed) % GROOVES.length];
}

// The repeating radial gradient itself (DESIGN §4), for a missing cover.
export function grooveBackground(seed) {
  const [a, b] = grooveFor(seed);
  return `repeating-radial-gradient(circle at 68% 32%, ${a} 0 5px, ${b} 5px 6px)`;
}

// A solid/soft tile for an artist without a photo: the darker of the pair,
// lifted by the lighter one at the top-left corner (where no initials sit).
export function grooveTint(seed) {
  const [a, b] = grooveFor(seed);
  return `radial-gradient(120% 120% at 28% 18%, ${b} 0%, ${a} 62%)`;
}

// Up to two initials, for a face we don't have. "Dr. Peacock" → "DP",
// "DJ TÖRKE" → "DT", "Angerfist" → "AN" (a single word still gives two
// letters, which is what makes a grid of circles look deliberate).
export function initialsOf(name) {
  const words = String(name ?? "")
    .replace(/[^\p{L}\p{N} ]/gu, "")
    .split(/\s+/)
    .filter(Boolean);
  if (!words.length) return "";
  if (words.length === 1) return words[0].slice(0, 2).toUpperCase();
  return (words[0][0] + words[1][0]).toUpperCase();
}

// -- colour maths -----------------------------------------------------------

const clamp = (v) => Math.max(0, Math.min(255, Math.round(v)));

export function toHex(rgb) {
  return (
    "#" +
    rgb
      .slice(0, 3)
      .map((v) => clamp(v).toString(16).padStart(2, "0"))
      .join("")
      .toUpperCase()
  );
}

export function fromHex(hex) {
  const s = String(hex || "").replace("#", "");
  if (s.length !== 6) return [0, 0, 0];
  return [0, 2, 4].map((i) => parseInt(s.slice(i, i + 2), 16) || 0);
}

// WCAG 2.1 relative luminance.
export function luminance(rgb) {
  const [r, g, b] = rgb.slice(0, 3).map((v) => {
    const c = clamp(v) / 255;
    return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

// WCAG 2.1 contrast ratio, 1…21.
export function contrast(a, b) {
  const la = luminance(a);
  const lb = luminance(b);
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

// Brighten a colour until it carries 4.5:1 against `on`.
//
// It scales the channels (a MULTIPLICATION, which keeps the hue and most of the
// saturation) rather than blending toward white — the white blend turned a deep
// cover blue into grey-lilac, and an accent that no longer resembles the sleeve
// is worse than no theme at all. Colours with a dead channel (a near-black
// sleeve) cannot be scaled into range, so those finish by mixing white in.
export function readableAccent(rgb, on = ON_ACCENT, ratio = 4.5) {
  const base = [0, 1, 2].map((i) => clamp(rgb[i] ?? 0));
  const bg = fromHex(on);
  if (contrast(base, bg) >= ratio) return base;
  let last = base;
  for (let s = 1.05; s <= 8.0001; s += 0.05) {
    last = base.map((v) => clamp(v * s));
    if (contrast(last, bg) >= ratio) return last;
  }
  for (let k = 0.05; k <= 1.0001; k += 0.05) {
    const mixed = last.map((v) => clamp(v + (255 - v) * k));
    if (contrast(mixed, bg) >= ratio) return mixed;
  }
  return [255, 255, 255];
}

// The accent the Pochette theme actually paints, from whatever the canvas read.
export function accentFor(rgb) {
  return toHex(readableAccent(rgb));
}
