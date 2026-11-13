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
// and a slow-falling peak per band, drawn as a faint afterglow of the capsule
// rather than the hairline cap of a 2005 media player.
//
// THE LEVELLING IS RUST (webapp/rhythm/src/viz_bars.rs, through
// lib/viz/core.js): the grouping, the three zone gains, the gamma, the
// envelopes and the caps. This file draws what it computed.
//
// THE PICTURE IS THE SILLON ONE (`worlds/spectrum.js` is its GL twin): thick
// capsules at a constant pitch, the bass end wider than the treble, a slow
// breath in beats at rest, a pale core on the axis and the band's colour warmed
// toward the tips. The 2D path is what a device without WebGL2 gets, so it is
// drawn with flat fills — no gradient object, no shadowBlur per bar, no
// allocation in the loop — and its halo comes from post.js exactly as before.

import { clamp, hsl, roundRect } from "../util.js";
import { BarsCore, vizCore } from "../core.js";

// One cycle of breath every four beats — a bar at four-four.
const BREATH_BEATS = 4;
const TAU = Math.PI * 2;

export function createBarsScene(opts = {}) {
  const layout = opts.layout || "strip"; // "strip" (centred) | "full" (grounded)
  let reduced = !!opts.reducedMotion;
  // Papier's player scrim is cream: the transparent strip draws ink there so
  // the row keeps its contrast. The full scene is a dark room on every theme.
  let light = !!opts.light;
  const core = opts.core || vizCore();
  if (!core) throw new Error("bars: the animation core is not loaded");
  const rb = new BarsCore(core);
  let bars = 0;
  let width = 0;
  // Continuous beats, for the rest breath. The beat tracker already carries a
  // continuous position (`beatIndex + phase`), so this needs no clock of its
  // own and pauses with the music.
  let beats = 0;

  function setCount(n) {
    if (n === bars) return;
    bars = Math.min(n, rb.max);
    rb.count(bars);
  }

  function resize(w, h, preset) {
    width = w;
    // The same pitch as the GL world: one bead per 16 CSS px in the strip,
    // one per 20.5 full screen, inside the tier's ceiling. Thicker than the
    // old one-per-seven, and a constant physical width at any resolution.
    const want = clamp(Math.floor(w / (layout === "full" ? 20.5 : 16)), 18, preset.bars);
    setCount(want);
    void h;
  }

  function update(frame, dt) {
    if (!bars || !frame.bands) return;
    rb.update(frame.bands, frame.features?.level || 0, dt);
    const b = frame.beat;
    if (b && Number.isFinite(b.beatIndex)) beats = b.beatIndex + (b.phase || 0);
    else beats += dt / Math.max(0.2, b?.period || 0.5);
  }

  function draw(g, w, h, pal) {
    if (!bars || !w || !h) return;
    const full = layout === "full";
    const ink = !full && light;
    // The strip draws no background of its own — it sits over the blurred
    // cover — so the canvas has to be cleared rather than washed. Full screen
    // is a scene, and gets the same deep, palette-tinted field as the GL world.
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
    const smooth = rb.smooth;
    const peaks = rb.peaks;
    // Level is information: a breakdown is a row of small beads, not the drop
    // renormalised to the same height. `rb.level` is the one slow envelope the
    // core already keeps for every canvas caller.
    const loud = 0.3 + 0.7 * clamp(rb.level * 1.5, 0, 1);

    for (let i = 0; i < bars; i++) {
      const t = i / (bars - 1 || 1);
      const v = smooth[i] * loud;
      // The bass is a lozenge, the treble a needle; a loud band swells.
      const taper = 1.06 - 0.26 * t;
      const weight = Math.max(0, 1 - t * 2.5) * Math.min(1, v * 1.4);
      const halfW = Math.min(r * taper * (0.88 + 0.22 * Math.min(1, v / 0.9) + 0.18 * weight), bw * 0.46);
      const breath = reduced ? 0 : Math.sin(TAU * (beats / BREATH_BEATS + t * 0.55));
      const rest = 1 - 0.65 * clamp((v - 0.05) / 0.65, 0, 1);
      const minHalf = halfW * (1 + 0.46 * (0.5 + 0.5 * breath) * rest);
      const half = Math.max(minHalf, v * reach);
      const x = i * bw + (bw - halfW * 2) / 2;
      // Hue walks the palette's spread across the row, so the low end and the
      // top end are visibly different registers rather than one flat colour.
      const hue = pal.low + (pal.high - pal.low) * t;
      const ph = Math.max(minHalf, peaks[i] * reach * loud);
      if (full && ph > half + 1.5) {
        g.fillStyle = hsl(hue + 12, pal.sat * 0.9, pal.light, 0.16);
        roundRect(g, x, axis - ph, halfW * 2, ph * 2, halfW);
        g.fill();
      }
      // The capsule's own gradient, in two fills: a warm rim, then the lit
      // body inside it. Flat colours on purpose — a gradient object per bar
      // per frame is garbage the fallback cannot afford. On paper the same
      // two fills carry the band's hue at an ink value, with a pale axis.
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
      // A pale bead on the axis — the hot core, not a specular stripe.
      const coreH = Math.max(1, Math.min(half, halfW * 1.7));
      g.fillStyle = ink
        ? hsl(hue, pal.sat * 0.45, 0.76, 0.3 + 0.25 * Math.min(1, v / 0.6))
        : hsl(hue, pal.sat * 0.3, 0.9, 0.2 + 0.3 * Math.min(1, v / 0.6));
      roundRect(g, x + inset, axis - coreH, Math.max(0.6, halfW * 2 - inset * 2), coreH * 2, Math.max(0.5, halfW - inset));
      g.fill();
    }
  }

  // The OS setting can change while a scene is on screen; the Visualizer pushes
  // it into the live scene rather than rebuilding one.
  function setOptions(next = {}) {
    if (next.reducedMotion != null) reduced = !!next.reducedMotion;
    if (next.light != null) light = !!next.light;
  }

  function dispose() {
    rb.free();
  }

  return {
    resize, update, draw, setOptions, dispose,
    get bars() { return bars; },
    get level() { return rb.level; },
  };
}
