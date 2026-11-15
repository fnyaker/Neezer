// Where a scene's colours come from.
//
// The user picks the SOURCE, not the colours. Five sources, each answering a
// different want:
//
//   cover     the artwork's own colour — the scene belongs to the track
//   spectrum  hue follows what the music is doing (this is the "colours by
//             BPM and by the level of each frequency band" mode: the balance
//             between bass, mids and highs sets the hue, the tempo sets how
//             fast it drifts, and the level sets how saturated it gets)
//   neon / warm / ice   fixed schemes, for when the artwork is beige
//
// A palette exposes a base hue plus three accents (low / mid / high) and is
// updated once per analysis frame. Scenes read it; nothing allocates.
//
// AND, FOR THE BARS, THE COVER'S OWN ZONES. A spectrum drawn left to right is a
// picture of the whole width of the frame, and a real sleeve is not one colour:
// a sunset on the left and a black sea on the right, a band across the middle.
// So each band takes the colour of the artwork UNDER IT — `setZones` is handed
// the horizontal zones the app's one cover decoder extracted (its middle row,
// left -> right, resampled here to ZONE_COUNT stops) and `colourAt(x)` walks
// them. This is still the palette's SOURCE the user picked: with any source but
// "cover" the zones are ignored, and with no artwork the row falls back to its
// own low -> high walk, exactly as before. The extraction itself is NOT here:
// `lib/coverpalette.js` (shared with the blurred backdrop) decodes the sleeve
// once, and this file only colours with the answer, at 94 Hz over a handful of
// numbers.

import { approach, approachAngle, clamp, hslToRgb, mixAngle, rgbToHsl } from "./util.js";
// THE SKIN CATALOGUE IS LOADED ON DEMAND. It is 227 rows and it only affects
// how far the palette leans toward the genre's own temperament — a subtle
// thing, and one that cannot matter before a scene is drawing anyway, at which
// point the scene's own chunk has already pulled this in. Importing it
// statically made the palette (which every visualizer needs from its first
// frame) a static dependency of the catalogue, and Rollup then hoisted the
// catalogue into the entry chunk where everyone paid for it at launch.
let skinFor = null;
let skinLoading = null;
function loadSkins() {
  if (skinFor || skinLoading) return;
  skinLoading = import("./skins.js")
    .then((m) => {
      skinFor = m.skinFor;
    })
    .catch(() => {
      // Offline mid-deploy: the palette keeps the source the user picked,
      // un-leaned, which is exactly what it did before skins existed.
      skinLoading = null;
    });
}

export const PALETTES = [
  { id: "cover", label: "Pochette" },
  { id: "spectrum", label: "Spectre" },
  { id: "neon", label: "Néon" },
  { id: "warm", label: "Braise" },
  { id: "ice", label: "Glace" },
  { id: "mono", label: "Monochrome" },
];

const FIXED = {
  neon: { hue: 288, spread: 92, sat: 0.92, light: 0.58 },
  warm: { hue: 22, spread: 46, sat: 0.88, light: 0.55 },
  ice: { hue: 196, spread: 58, sat: 0.8, light: 0.6 },
  mono: { hue: 265, spread: 8, sat: 0.06, light: 0.72 },
};

// How many stops the row's colour walk has. Seven is enough that a 24-bead
// strip walks without a visible step, and small enough that the GL uniform it
// feeds is 7 vec4 — a handful of floats, once a picture.
export const ZONE_COUNT = 7;

/**
 * The horizontal zones to colour a row of bars from, taken from the palette the
 * app's one cover decoder answers with (lib/coverpalette.js#paletteFor).
 *
 * PER COLUMN, THE MOST SATURATED OF ITS THREE CELLS — not the column's mean.
 * A mean mixes the top of a sleeve with its bottom, and in colour a blue over
 * an amber average is olive: the zone ends up a colour that appears nowhere in
 * the artwork and that sits wrong on the blurred backdrop it is drawn over. The
 * decoder hands back its 3×3 grid (`cells`), so each vertical band can report a
 * colour it actually contains. `middles` and `zones` are the fallbacks for a
 * decoder that answers with rows or with column means only.
 *
 * Three anchors are as much positional truth as one 64 px decode holds;
 * `setZones` resamples them to ZONE_COUNT stops, so the "between them" colours a
 * row of beads shows are blends of the sleeve's own bands, not invented hues.
 */
export function zonesOfPalette(p) {
  if (!p) return null;
  const cells = Array.isArray(p.cells) && p.cells.length >= 9 ? p.cells : null;
  const fill = p.average || p.dominant || [128, 128, 128];
  let z = null;
  if (cells) {
    // Row-major 3×3: column `c` is cells[c], cells[3 + c], cells[6 + c].
    z = [0, 1, 2].map((c) => bestOf(cells[c], cells[3 + c], cells[6 + c]) || fill);
  } else if (p.zones) z = [p.zones.left, p.zones.center, p.zones.right];
  else if (Array.isArray(p.middles) && p.middles.length) z = p.middles;
  if (!z || !z.length) return null;
  return z.map((c) => c || fill);
}

// The cell a zone is best represented by: the saturated one that is not at an
// extreme of lightness — the same score lib/color.js uses for an accent, and
// the reason this is a choice rather than an average.
function bestOf(...list) {
  let best = null;
  let scoreMax = -1;
  let sum = [0, 0, 0];
  let n = 0;
  for (const c of list) {
    if (!c) continue;
    sum = [sum[0] + c[0], sum[1] + c[1], sum[2] + c[2]];
    n++;
    const max = Math.max(c[0], c[1], c[2]);
    const min = Math.min(c[0], c[1], c[2]);
    const sat = max ? (max - min) / max : 0;
    const score = sat * (1 - Math.abs(max / 255 - 0.55));
    if (score > scoreMax) {
      scoreMax = score;
      best = c;
    }
  }
  if (!n) return null;
  // A column of greys keeps its mean: "the most saturated grey" is noise.
  if (scoreMax < 0.06) return [Math.round(sum[0] / n), Math.round(sum[1] / n), Math.round(sum[2] / n)];
  return best;
}

// The shapes the shared decoder may answer with, normalised to a plain list.
function zoneList(list) {
  if (!list) return [];
  if (Array.isArray(list)) return list;
  if (Array.isArray(list.zones)) return list.zones;
  if (Array.isArray(list.palette)) return list.palette;
  return [];
}

// [r,g,b] 0..255, {r,g,b} or {rgb:[…]}, and 0..1 floats tolerated — one cover
// decoder exists in this app and this is the edge where its answer is read.
function zoneRgbOf(zone) {
  if (!zone) return null;
  const a = Array.isArray(zone) ? zone : Array.isArray(zone.rgb) ? zone.rgb : [zone.r, zone.g, zone.b];
  if (a.length < 3) return null;
  let [r, g, b] = [+a[0] || 0, +a[1] || 0, +a[2] || 0];
  if (r <= 1 && g <= 1 && b <= 1) {
    r *= 255;
    g *= 255;
    b *= 255;
  }
  return [clamp(r, 0, 255), clamp(g, 0, 255), clamp(b, 0, 255)];
}

export function createPalette(mode = "cover") {
  // The cover source's FALLBACK is the Sillon accent — the lemon of Encre and
  // Papier — not an arbitrary violet. It is what a track with no artwork (a
  // local file, a sleeve the CDN cannot serve) draws with, and the colour the
  // first frames have before the artwork's own dominant colour arrives. Once a
  // cover answers, it takes over: this is still the "cover" source.
  let hue = 73.8;
  let spread = 64;
  let sat = 1;
  let light = 0.62;
  let drift = 0; // continuous rotation, advanced at the tempo
  let coverHue = 73.8;
  let coverSat = 1;

  // The sleeve's zones, as `setZones` normalised them: HSL per zone, plus the
  // two views a scene draws from. Allocated once for the life of the palette,
  // so a frame allocates nothing.
  const zoneH = new Float32Array(ZONE_COUNT);
  const zoneS = new Float32Array(ZONE_COUNT);
  const zoneL = new Float32Array(ZONE_COUNT);
  const zoneGrey = new Uint8Array(ZONE_COUNT);
  const zones = new Float32Array(ZONE_COUNT * 3); // h (degrees), s, l — the canvas walk
  const zoneRgb = new Float32Array(ZONE_COUNT * 4); // r, g, b, 1 in sRGB — the GL walk
  let hasZones = false;

  const out = {
    mode,
    hue,
    spread,
    sat,
    light,
    // Accent hues for the three broad registers, so a scene can colour a bass
    // element and a treble element differently without inventing its own scheme.
    low: hue - spread / 2,
    mid: hue,
    high: hue + spread / 2,
    energy: 0,
    // The cover's own palette, left -> right, when the palette's source is the
    // cover and there is artwork. `zones` is [h, s, l] per zone (what a canvas
    // scene draws with), `zoneRgb` is [r, g, b, 1] per zone in sRGB (what the
    // GL engine uploads). Both are the same colour, in the space each consumer
    // works in; `hasZones` is false when there is nothing to walk.
    zones,
    zoneRgb,
    hasZones,
    colourAt,
  };

  function setMode(m) {
    if (m && m !== mode) {
      mode = m;
      out.mode = m;
    }
  }

  // The genre's own colour temperament (lib/viz/skins.js). The `warm` lean
  // below is one axis blended across families; this is the specific thing a
  // named genre asks for — acid is not simply "warm", it is acid green, and
  // black metal is not "cold", it is bleached. Like `warm` it is a LEAN on the
  // source the user picked, never a replacement: the cover still decides which
  // colour is being leaned.
  let skinHue = 0;
  let skinSat = 1;
  let skinLight = 1;
  function setSkin(s) {
    skinHue = +(s?.hue ?? 0) || 0;
    skinSat = +(s?.sat ?? 1) || 1;
    skinLight = +(s?.light ?? 1) || 1;
  }
  // Resolved from the frame, and MEMOISED on the name: this runs ninety times a
  // second and the answer only changes when the classifier renames the family.
  let lastName = null;
  function followSkin(style) {
    const name = style ? `${style.dominant}|${style.archetype}` : "";
    if (name === lastName) return;
    lastName = name;
    if (!skinFor) {
      loadSkins();
      setSkin(null);
    } else setSkin(style ? skinFor(style.dominant, style.archetype) : null);
  }

  // The cover's dominant colour, as [r,g,b]. Called when the track changes;
  // `null` is "there is no artwork", which goes back to the accent rather than
  // keeping the last sleeve's colour.
  function setCover(rgb) {
    if (!rgb) {
      coverHue = 73.8; // the Sillon lemon, the same accent a missing cover gets
      coverSat = 1;
      return;
    }
    const [h, s] = rgbToHsl(rgb);
    // A sleeve with no hue at all — a monochrome sleeve, the grey placeholder
    // disc of a missing file — gets the Sillon accent rather than hue 0. Red is
    // a colour the artwork does not contain, and the accent is exactly what
    // this palette already draws with when there is no artwork at all.
    coverHue = s < 0.08 ? 73.8 : h;
    // A washed-out cover would give a grey scene; floor the saturation so the
    // animation still has colour in it, and cap it so a neon sleeve does not
    // produce something painful on a big screen.
    coverSat = clamp(s * 0.85 + 0.25, 0.35, 0.92);
  }

  // The cover's vertical zones, left to right — the share of the palette this
  // file owns. Called once per track by the host with what the app's one cover
  // decoder answered; `null` forgets them, and the row goes back to its own
  // low -> high walk.
  function setZones(list) {
    const src = zoneList(list);
    if (!src.length) {
      hasZones = false;
      out.hasZones = false;
      return;
    }
    const n = src.length;
    for (let i = 0; i < ZONE_COUNT; i++) {
      // Resampled to our count, so the decoder may answer with 5, 7 or 12.
      const rgb = zoneRgbOf(n === 1 ? src[0] : src[Math.round((i * (n - 1)) / (ZONE_COUNT - 1))]);
      if (!rgb) continue;
      const [h, sat, l] = rgbToHsl(rgb);
      zoneH[i] = h;
      // The same floors setCover applies to the dominant colour: a washed-out
      // zone would draw a grey bead, a neon sleeve could burn.
      zoneS[i] = clamp(sat * 0.85 + 0.25, 0.35, 0.92);
      zoneL[i] = l;
      // A zone with no hue of its own (a grey band, a black border) borrows the
      // sleeve's: a row of grey beads on a coloured sleeve reads as a bug.
      zoneGrey[i] = sat < 0.08 ? 1 : 0;
    }
    hasZones = true;
  }

  // The row's colour at `t` (0 left, 1 right), written into `dst` as
  // [hue degrees, saturation, lightness] — no allocation, called per band. The
  // hue takes the short way round between two zones, so 350° -> 10° crosses red
  // instead of sweeping the wheel backwards across the row. False when there is
  // no zone palette to walk.
  function colourAt(t, dst) {
    if (!out.hasZones) return false;
    const x = clamp(t, 0, 1) * (ZONE_COUNT - 1);
    const i = Math.min(ZONE_COUNT - 2, Math.floor(x));
    const k = x - i;
    const a = i * 3;
    const b = a + 3;
    const d = (((zones[b] - zones[a]) % 360) + 540) % 360 - 180;
    dst[0] = (((zones[a] + d * k) % 360) + 360) % 360;
    dst[1] = zones[a + 1] + (zones[b + 1] - zones[a + 1]) * k;
    dst[2] = zones[a + 2] + (zones[b + 2] - zones[a + 2]) * k;
    return true;
  }

  function update(frame, dt) {
    const f = frame.features;
    const beat = frame.beat;
    const e = frame.energy;
    const total = e.sub + e.bass + e.lowMid + e.mid + e.high + e.air + 1e-12;
    const lowShare = (e.sub + e.bass) / total;
    const highShare = (e.high + e.air) / total;
    out.energy = f ? f.level : 0;

    // Drift: a full turn every 16 bars at the detected tempo, so the colour
    // cycle is locked to the music rather than to wall-clock time. With no
    // tempo yet it creeps, which reads as "waiting" rather than "broken".
    const barLen = beat?.locked ? beat.period * beat.beatsPerBar : 2.4;
    drift = (drift + (dt / (barLen * 16)) * 360) % 360;

    let targetHue;
    let targetSat;
    let targetLight;
    let targetSpread;
    if (mode === "cover") {
      targetHue = coverHue + drift * 0.28;
      targetSat = coverSat;
      targetLight = 0.56;
      targetSpread = 64;
    } else if (mode === "spectrum") {
      // Bass-heavy → the warm end, treble-heavy → the cool end, and the whole
      // mapping rotates with the tempo so a track never sits on one colour.
      const balance = clamp(0.5 + (highShare - lowShare) * 1.6, 0, 1);
      targetHue = 8 + balance * 250 + drift;
      targetSat = clamp(0.55 + (f ? f.level : 0) * 0.45, 0.5, 1);
      targetLight = clamp(0.44 + (f ? f.level : 0) * 0.2, 0.4, 0.68);
      targetSpread = 60 + (f ? f.percussivity : 0) * 70;
    } else {
      const p = FIXED[mode] || FIXED.neon;
      targetHue = p.hue + drift * 0.16;
      targetSat = p.sat;
      targetLight = p.light;
      targetSpread = p.spread;
    }

    // The genre's own temperature, as a pull rather than an override: the user
    // picked the palette SOURCE and that choice stands, but frenchcore and
    // psytrance do not want the same end of it. `warm` is 0 for cold and 1 for
    // hot, so this leans the hue a third of the way toward red or toward cyan
    // and leaves the rest of the palette alone. See style.js's LOOK_KEYS.
    const look = frame.style?.look;
    if (look) {
      const towards = look.warm > 0.5 ? 14 : 196; // red, or cyan
      const pull = Math.abs(look.warm - 0.5) * 0.62;
      targetHue = mixAngle(targetHue, towards, pull);
      // A harmony that is moving earns a little more colour; a held drone does
      // not. This is the melodic channel showing up in the palette at all.
      targetSat = clamp(targetSat + (f?.chordChange || 0) * 0.12, 0, 1);
    }

    followSkin(frame.style);
    targetHue += skinHue;
    targetSat = clamp(targetSat * skinSat, 0.02, 1);
    targetLight = clamp(targetLight * skinLight, 0.12, 0.85);

    // Ease everything: an instant hue change on a full-screen background is a
    // flash, and this runs at ~94 Hz.
    hue = approachAngle(hue, targetHue, 0.5, dt);
    sat = approach(sat, targetSat, 0.4, dt);
    light = approach(light, targetLight, 0.4, dt);
    spread = approach(spread, targetSpread, 0.7, dt);

    out.hue = hue;
    out.sat = sat;
    out.light = light;
    out.spread = spread;
    out.low = hue - spread * 0.5;
    out.mid = hue;
    out.high = hue + spread * 0.5;

    // The sleeve's own zones: the hue lean the rest of the palette just earned
    // (the tempo's drift, the genre's temperature) applied to each of them, and
    // the value folded into the row's own band so a dark sleeve reads as a
    // deeper bead rather than as a black one. Recomputed here, once per
    // analysis frame, so both consumers see the same numbers and nothing is
    // read per picture.
    const walk = hasZones && mode === "cover";
    out.hasZones = walk;
    if (walk) {
      for (let i = 0; i < ZONE_COUNT; i++) {
        const base = zoneGrey[i] ? coverHue : zoneH[i];
        const k = i * 3;
        // NO TEMPO DRIFT HERE, on purpose. The drift exists so a palette built
        // on ONE colour does not sit still for a whole track; the zones are the
        // artwork's own colours, and the blurred backdrop behind the row is that
        // same artwork. Rotating them away from it would put a violet row on a
        // blue sleeve — the one thing a zoned palette must not do. The genre's
        // lean stays: it is a deliberate tint of the source, not a rotation.
        const h = (((base + skinHue) % 360) + 360) % 360;
        const s = clamp(zoneS[i] * skinSat, 0.3, 1);
        const l = clamp((light * 0.6 + zoneL[i] * 0.4) * skinLight, 0.5, 0.72);
        zones[k] = h;
        zones[k + 1] = s;
        zones[k + 2] = l;
        hslToRgb(h / 360, s, l, zoneRgb, i * 4);
        zoneRgb[i * 4 + 3] = 1;
      }
    }
    return out;
  }

  return { update, setMode, setCover, setSkin, setZones, colourAt, out };
}
