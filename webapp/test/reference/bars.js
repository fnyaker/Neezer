// THE ORACLE. lib/viz/scenes/bars.js as it was in JavaScript, before its
// levelling moved to Rust (webapp/rhythm/src/viz_bars.rs); test/vizcore.test.mjs
// holds the Rust core to the same heights and caps. Not imported by the app.
// What it pins is the LEVELLING: `draw` follows the app's drawing (the row of
// capsules that replaced the glass bars), so the comparison stays about the
// arithmetic and not about a picture the app no longer draws.

// The spectrum bars, rebuilt on the precise band data.
//
// The levelling is the part worth reading. One global auto-gain across the whole
// spectrum does not work on music: the mids and highs carry most of the energy,
// so they drive the gain down and the bass ends up parked at the bottom of the
// display whatever it is doing. Instead the strip is split into three zones —
// bass, mid, high — each with its own slow gain envelope, and the per-bar gain
// is interpolated between the zone centres so there is no seam where they meet.
// Every register then keeps its own life.
//
// On top of that: a gamma curve for quiet/loud contrast, a fast-attack /
// slow-release envelope so bars are lively rather than parked in the middle,
// and peak caps that fall slowly — which is what makes a spectrum readable
// rather than just busy.

import { approach, clamp, groupBands, hsl, roundRect } from "../../src/lib/viz/util.js";

const CENTRES = [1 / 6, 0.5, 5 / 6];
const BREATH_BEATS = 4;
const TAU = Math.PI * 2;

export function createBarsScene(opts = {}) {
  const layout = opts.layout || "strip"; // "strip" (centred) | "full" (grounded)
  let bars = 0;
  let grouped = null;
  let smooth = null;
  let peaks = null;
  const agc = [0.15, 0.15, 0.15];
  let width = 0;
  let level = 0;
  let beats = 0;

  function gainAt(gain, t) {
    if (t <= CENTRES[0]) return gain[0];
    if (t >= CENTRES[2]) return gain[2];
    const z = t < CENTRES[1] ? 0 : 1;
    const f = (t - CENTRES[z]) / (CENTRES[z + 1] - CENTRES[z]);
    return gain[z] * (1 - f) + gain[z + 1] * f;
  }

  function setCount(n) {
    if (n === bars) return;
    bars = n;
    grouped = new Float32Array(n);
    smooth = new Float32Array(n);
    peaks = new Float32Array(n);
  }

  function resize(w, h, preset) {
    width = w;
    // The same pitch as the app's canvas scene and its GL twin.
    const want = clamp(Math.floor(w / (layout === "full" ? 20.5 : 16)), 18, preset.bars);
    setCount(want);
    void h;
  }

  function update(frame, dt) {
    if (!bars) return;
    groupBands(frame.bands, grouped);
    level = approach(level, frame.features?.level || 0, 0.15, dt);
    const b = frame.beat;
    if (b && Number.isFinite(b.beatIndex)) beats = b.beatIndex + (b.phase || 0);
    else beats += dt / Math.max(0.2, b?.period || 0.5);

    // Per-zone slow level → per-zone gain.
    const zoneSum = [0, 0, 0];
    const zoneN = [0, 0, 0];
    for (let i = 0; i < bars; i++) {
      const z = i < bars / 3 ? 0 : i < (2 * bars) / 3 ? 1 : 2;
      zoneSum[z] += grouped[i];
      zoneN[z] += 1;
    }
    const gain = [0, 0, 0];
    for (let z = 0; z < 3; z++) {
      const mean = zoneN[z] ? zoneSum[z] / zoneN[z] : 0;
      agc[z] = approach(agc[z], mean, 0.9, dt);
      gain[z] = 0.62 / Math.max(0.1, agc[z]);
    }

    for (let i = 0; i < bars; i++) {
      const t = i / bars;
      let v = Math.min(1, grouped[i] * gainAt(gain, t));
      v = Math.pow(v, 1.7); // gamma → quiet/loud contrast
      smooth[i] =
        v > smooth[i] ? approach(smooth[i], v, 0.035, dt) : approach(smooth[i], v, 0.16, dt);
      // Peak caps: instant up, then a slow, accelerating fall.
      if (smooth[i] > peaks[i]) peaks[i] = smooth[i];
      else peaks[i] = Math.max(smooth[i], peaks[i] - dt * (0.22 + peaks[i] * 0.35));
    }
  }

  function draw(g, w, h, pal) {
    if (!bars || !w || !h) return;
    const full = layout === "full";
    const ink = false; // the oracle's frames never carry the Papier option
    if (full) {
      g.fillStyle = hsl(pal.hue, pal.sat * 0.4, 0.045, 0.72);
      g.fillRect(0, 0, w, h);
    } else {
      g.clearRect(0, 0, w, h);
    }
    const bw = w / bars;
    const body = Math.max(2, Math.min(bw * 0.64, h * 0.55));
    const r = body / 2;
    const axis = h / 2;
    const reach = full ? h * 0.31 : h / 2 - 1;

    const loud = 0.3 + 0.7 * clamp(level * 1.5, 0, 1);

    for (let i = 0; i < bars; i++) {
      const t = i / (bars - 1 || 1);
      const v = smooth[i] * loud;
      const taper = 1.06 - 0.26 * t;
      const weight = Math.max(0, 1 - t * 2.5) * Math.min(1, v * 1.4);
      const halfW = Math.min(r * taper * (0.88 + 0.22 * Math.min(1, v / 0.9) + 0.18 * weight), bw * 0.46);
      const breath = Math.sin(TAU * (beats / BREATH_BEATS + t * 0.55));
      const rest = 1 - 0.65 * clamp((v - 0.05) / 0.65, 0, 1);
      const minHalf = halfW * (1 + 0.46 * (0.5 + 0.5 * breath) * rest);
      const half = Math.max(minHalf, v * reach);
      const x = i * bw + (bw - halfW * 2) / 2;
      const hue = pal.low + (pal.high - pal.low) * t;
      const ph = Math.max(minHalf, peaks[i] * reach * loud);
      if (full && ph > half + 1.5) {
        g.fillStyle = hsl(hue + 12, pal.sat * 0.9, pal.light, 0.16);
        roundRect(g, x, axis - ph, halfW * 2, ph * 2, halfW);
        g.fill();
      }
      g.fillStyle = ink
        ? hsl(hue + 8, pal.sat, 0.3, 0.95)
        : hsl(hue + 14, pal.sat, Math.min(0.9, pal.light - 0.01), 0.92);
      roundRect(g, x, axis - half, halfW * 2, half * 2, halfW);
      g.fill();
      const inset = Math.min(1.4, halfW * 0.3);
      const iw = halfW * 2 - inset * 2;
      const ih = half * 2 - inset * 2;
      if (iw > 0.6 && ih > 0.6) {
        g.fillStyle = ink
          ? hsl(hue, pal.sat * 0.75, 0.44, 0.92)
          : hsl(hue, pal.sat * 0.85, Math.min(0.92, pal.light + 0.1 + 0.22 * v), 0.74 + 0.26 * Math.min(1, v / 0.6));
        roundRect(g, x + inset, axis - half + inset, iw, ih, Math.max(0.5, halfW - inset));
        g.fill();
      }
      const coreH = Math.max(1, Math.min(half, halfW * 1.7));
      g.fillStyle = ink
        ? hsl(hue, pal.sat * 0.45, 0.76, 0.3 + 0.25 * Math.min(1, v / 0.6))
        : hsl(hue, pal.sat * 0.3, 0.9, 0.2 + 0.3 * Math.min(1, v / 0.6));
      roundRect(g, x + inset, axis - coreH, Math.max(0.6, halfW * 2 - inset * 2), coreH * 2, Math.max(0.5, halfW - inset));
      g.fill();
    }
  }

  return {
    resize, update, draw,
    get smooth() { return smooth; },
    get peaks() { return peaks; },
    get bars() { return bars; },
    get level() { return level; },
  };
}
