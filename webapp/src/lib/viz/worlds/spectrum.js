// SPECTRE — the spectrum as one groove of light.
//
// The most literal picture in the catalogue, and the one where fashion shows
// most. It used to be glass tubes: a rim light down each edge, a specular
// stripe, a hot top, hairline peak caps hanging above, a mirror-polished floor
// reflecting it all — every one of them a 2005 idea, and together they read as
// a media player from that year. The capsules that replaced them were honest
// but thin, evenly spaced and evenly coloured: a generic equaliser in a nicer
// dress. This is the Sillon version, and what it is made of is:
//
//   THE SHAPE. One capsule per band, fully rounded and mirrored about its axis
//   as ONE shape (two rounded bars meeting in the middle drew a dark pinch
//   along the axis). They are THICK now — one per 16 CSS px rather than one per
//   seven — so a band reads as a bead of light rather than a hairline, with a
//   gap you can see. The width follows the level: a quiet band is a dot, a loud
//   one swells into a lozenge, and the low end swells on the kick.
//   THE BREATH. At rest the row is never frozen: a slow wave, one cycle per bar,
//   travels along the bands so a breakdown is a groove breathing, not fifty
//   dead dashes. It is a clock in BEATS (uClock.x), and prefers-reduced-motion
//   holds it still.
//   THE COLOUR. The sleeve's OWN colours, walked left to right: `uZone` is the
//   artwork's vertical zones (lib/coverpalette.js through palette.js#zones), so
//   the bass end takes the colour of the cover under it and the treble takes
//   the cover's other end. No artwork, or a palette source that is not the
//   cover, falls back to the palette's low -> high walk (`pal`). Along each
//   capsule: a pale, hot core on the axis and the band's own colour warmed at
//   the tips — the warm end is the band's colour through a warm gel, not a
//   second palette. The bloom does the halo; nothing is painted on to fake one.
//   THE LEVEL. The three zone gains keep every register alive (one gain across
//   the spectrum parks the bass: the mids and highs drive it down), but the
//   heights are also scaled by the track's own DYNAMICS, so a breakdown is a
//   row of small beads instead of the drop renormalised to the same height.
//   THE WEIGHT. The low register is the subject: the sub's own envelope
//   (uPump.x) adds light behind the bass end and fattens those bands, so the
//   drop lands in the body before it is seen in the tips.
//   THE TRAIL. No peak caps. The highest each band has been in the last two
//   beats, falling linearly in sixteenths, is drawn as a faint extension of the
//   capsule — an afterglow that hangs and drops at the tempo of the track,
//   computed from the spectrum history with no per-bar state anywhere. The
//   strip does not draw it: over a player's artwork that ghost read as a smudge,
//   and dropping it saves the 8 history taps a pixel per picture.
//
// NO FIELD, NO FRAME. Full screen used to lay a deep palette-tinted room behind
// the row, with the light the row cast along its own axis. That field WAS the
// rectangle the critique saw sitting on the artwork: the app has one blurred
// backdrop and the bars are a row of light over it, nothing else. So the world
// now writes COLOUR AND ALPHA everywhere — the beads, their afterglow, and
// nothing in between — and the page's own backdrop shows through. There is no
// vignette, no border and no glow layer of this world's: the engine's bloom is
// the only halo, and it is off in eco.
//
// ON PAPER. Papier's player scrim is cream, not near-black, so a lemon row
// there is 1.02:1 and disappears. The transparent strip draws the band's own
// colour at an INK value with a pale axis core instead; a full-screen row stays
// the light one, because the screen it is on (the projector, the settings
// preview) is a dark field on every theme. The flag arrives in uCtl.w.
//
// Full screen the row runs along the middle of the frame; with the artwork in
// front, one row fills the band under it and one the band above, so they frame
// it instead of growing up behind it. In the player's strip (the `strip`
// layout) the row sits nearer the edges of its 40-64 px band.
//
// Parameters:
//   width    capsule width as a share of its pitch   trail  the afterglow
//   curve    loudness gamma (higher = more contrast)
//   pitch    CSS px per band in the strip (fewer, rounder capsules = larger)

export default {
  id: "spectrum",
  uses: ["sdf"],
  params: { width: 0.64, curve: 1.8, trail: 1, pitch: 16 },
  // No lens fringe: on thin light lines it reads as a colour-fringed LCD, and
  // on a 40 px strip it split every capsule into three. `transparent` says this
  // world writes its own coverage instead of an opaque field: the engine grades
  // it as premultiplied light, so whatever the page puts behind the canvas is
  // what the row sits on.
  look: { exposure: 1, bloom: 1.0, threshold: 0.68, saturation: 1.08, ca: 0, transparent: true },

  fragment: `
// The zone gains (uS0.xyz: bass, mid, high), interpolated between the zone
// centres so the three never show a seam.
float gainAt(float f) {
  if (f <= 1.0 / 6.0) return uS0.x;
  if (f >= 5.0 / 6.0) return uS0.z;
  if (f < 0.5) return mix(uS0.x, uS0.y, (f - 1.0 / 6.0) * 3.0);
  return mix(uS0.y, uS0.z, (f - 0.5) * 3.0);
}

float level(float f, float row) {
  return texture(uSpec, vec2(f, row)).r;
}

float shape(float v, float f) {
  return pow(clamp(v * gainAt(f), 0.0, 1.0), P_CURVE);
}

// A history row \`k\` sixteenths ago.
float histAt(float f, float k) {
  float y = uHistHead - k / 64.0;
  return texture(uHist, vec2(f, fract(y) + 0.5 / 64.0)).r;
}

// A capsule of half width w and half height h (h >= w), centred on the origin.
float capsule(vec2 q, float w, float h) {
  return sdRound2(q, vec2(w, max(h, w)), w);
}

// The warm end of a band's colour: the band's own hue pushed through a warm
// gel, not a second palette. It is what makes the tips read as light that went
// through something, the way the canvas post pass grades its halo.
vec3 warmTip(vec3 c) {
  return mix(c, c * vec3(1.16, 1.0, 0.72) * 1.12, 0.45);
}

// The sleeve's zones arrive as sRGB; the engine's light is linear, and the
// palette's own colours arrive luminance-normalised so a pale sleeve and a dark
// one expose the same. The zones do the same, to the same 0.26 the block uses.
vec3 coverLight(vec3 c) {
  vec3 l = vec3(
    c.r <= 0.04045 ? c.r / 12.92 : pow((c.r + 0.055) / 1.055, 2.4),
    c.g <= 0.04045 ? c.g / 12.92 : pow((c.g + 0.055) / 1.055, 2.4),
    c.b <= 0.04045 ? c.b / 12.92 : pow((c.b + 0.055) / 1.055, 2.4));
  return l * clamp(0.26 / max(luma(l), 1e-4), 0.5, 3.5);
}

// The colour at a band's position across the row: the cover's own zone under
// it when there is one, else the palette's walk. Seven zones, six spans.
vec3 zoneAt(float t) {
  if (uZoneOK < 0.5) return pal(t);
  float x = clamp(t, 0.0, 1.0) * 6.0;
  float i = min(floor(x), 5.0);
  vec3 a = uZone[int(i)].rgb;
  vec3 b = uZone[int(i) + 1].rgb;
  return coverLight(mix(a, b, x - i));
}

void main() {
  vec2 frag = gl_FragCoord.xy;
  bool strip = uCtl.z > 0.5;
  float N = max(8.0, uS0.w);
  float pitchPx = uRes.x / N;
  float bi = clamp(floor(frag.x / pitchPx), 0.0, N - 1.0);
  float f = (bi + 0.5) / N;
  // The band's value: the peak of three taps across its span, so a narrow
  // partial inside a wide band is still seen (averaging it away is what makes
  // a spectrum look limp).
  float span = 0.5 / N;
  float vS = max(level(f, 0.25), max(level(f - span * 0.6, 0.25), level(f + span * 0.6, 0.25)));
  // Level is information: a breakdown is a row of small beads, not the drop's
  // wall renormalised to the same height.
  float dyn = clamp(uMood.z, 0.0, 1.0);
  float loud = mix(0.3, 1.0, smoothstep(0.05, 0.9, dyn));
  float v = shape(vS, f) * loud;
  // Where the row runs: its axis and how far a capsule may reach from it, in
  // pixels, so the edges are anti-aliased at any size.
  float axis = 0.5 * uRes.y;
  float reach = strip ? 0.5 * uRes.y - 1.5 : 0.31 * uRes.y;
  if (!strip && uHole.z > 0.0) {
    // With the artwork in front: the band under it and the band above it,
    // each with its own row, when both have room for one.
    float below = (0.5 + 0.5 * uHoleR.z) * uRes.y;
    float above = (0.5 + 0.5 * (uHole.y + uHole.w + 0.03)) * uRes.y;
    float lo = below - 0.04 * uRes.y;
    float hi = 0.96 * uRes.y - above;
    if (min(lo, hi) > 0.1 * uRes.y) {
      bool upper = frag.y > 0.5 * (below + above);
      axis = upper ? above + 0.5 * hi : 0.04 * uRes.y + 0.5 * lo;
      reach = 0.46 * (upper ? hi : lo);
    }
  }

  // The low end is the subject. The sub's own cone (uPump.x) and any kick
  // weight the bands near the bottom of the spectrum...
  float low = pow(1.0 - f, 2.2);
  float kick = uHit.x * (0.55 + 0.45 * uHit.y);
  float sub = 0.6 * uPump.x + 0.4 * uHit.y;
  float weight = clamp(0.65 * kick + 0.7 * sub, 0.0, 1.0) * low;
  float vv = clamp(v + weight * 0.22 * (0.4 + 0.6 * v), 0.0, 1.0);

  // A band's width follows its level and its register: the bass is a lozenge,
  // the treble a needle, and a loud band swells — a quiet one is a dot. The low
  // end fattens on the kick, which is the difference between an equaliser and
  // a body.
  float taper = mix(1.06, 0.8, smoothstep(0.0, 1.0, f));
  float wBase = pitchPx * P_WIDTH * 0.5;
  float halfW = min(wBase * taper * (0.88 + 0.22 * smoothstep(0.0, 0.9, vv) + 0.18 * weight), pitchPx * 0.46);
  // At rest, a slow wave — one cycle per bar — travels along the row, so a
  // quiet passage breathes rather than freezing into dashes. In BEATS, and
  // held still under prefers-reduced-motion.
  float breath = uCtl.y > 0.5 ? 0.0 : sin(TAU * (uClock.x * 0.25 + f * 0.55));
  float rest = 1.0 - 0.65 * smoothstep(0.05, 0.7, vv);
  float minHalf = halfW * (1.0 + 0.46 * (0.5 + 0.5 * breath) * rest);
  float hh = max(minHalf, vv * reach);

  float cx = (bi + 0.5) * pitchPx;
  vec2 q = vec2(frag.x - cx, frag.y - axis);
  float sd = capsule(q, halfW, hh);
  float cover = clamp(0.5 - sd, 0.0, 1.0);
  // The band's own colour at this position across the row, and along each
  // capsule: hot core at the axis, the band's colour warmed at the tips.
  vec3 base = zoneAt(f);
  vec3 core = mix(base, vec3(1.0), 0.45);
  vec3 tip = warmTip(base);
  float along = clamp(abs(q.y) / max(hh, 1.0), 0.0, 1.0);
  vec3 c = mix(core, tip, smoothstep(0.0, 1.0, along));
  c *= (0.62 + 0.68 * vv + 0.6 * weight + 0.18 * uHit2.w);
  // The tips melt into the bloom instead of ending on a hard edge.
  c *= 1.0 - 0.14 * smoothstep(0.72, 1.0, along);
  // Papier is the one theme whose player scrim is paper, not a dark room, so
  // the transparent STRIP becomes ink there. A full-screen row stays the light
  // one: the screen behind it is the projector's own dark page on every theme.
  bool pale = strip && uCtl.w > 0.5;

  vec3 body = c;
  if (pale) {
    // A deep, SATURATED version of the band's colour — not a grey — with a
    // pale core on the axis. On cream it reads as ink that still belongs to
    // the sleeve. This is a change of VALUE, not of palette: the source the
    // user picked still decides the hue.
    vec3 ink = base * 0.38;
    vec3 inkCore = mix(ink, vec3(1.0), 0.32);
    body = mix(inkCore, warmTip(ink) * 1.05, smoothstep(0.0, 1.0, along));
    body *= 0.9 + 0.25 * vv;
  }
  // A quiet band is a little translucent, a loud one solid: the row reads as
  // light over the artwork rather than stickers on it. On paper the ink goes
  // out nearly opaque, because contrast is the whole point there.
  float aBody = cover * (pale ? 0.96 : (0.82 + 0.18 * smoothstep(0.0, 0.5, vv)));

  // The afterglow, full screen: the highest this band has been in the last two
  // beats, falling 0.045 per sixteenth. Stateless, and it falls in musical
  // time. Not computed for the strip, which draws no ghost — and not in eco,
  // where P_TRAIL is zeroed.
  vec3 outC = body;
  float outA = aBody;
  if (!strip) {
    float cap = v;
    for (int k = 1; k < 8; k++) {
      float fk = float(k);
      cap = max(cap, shape(histAt(f, fk), f) * loud - fk * 0.045);
    }
    float hg = max(minHalf, cap * reach);
    float ghost = clamp(0.5 - capsule(q, halfW, hg), 0.0, 1.0) * (1.0 - cover);
    vec3 g = mix(tip, base, 0.5) * (0.45 + 0.55 * cap);
    float ga = ghost * 0.2 * P_TRAIL;
    // The ghost hangs BEHIND the bead and never inside it, so the two layers
    // resolve as one straight colour and one coverage — what the engine's final
    // pass expects of a transparent world.
    float k2 = ga * (1.0 - aBody);
    outA = aBody + k2;
    outC = (body * aBody + g * k2) / max(outA, 1e-4);
  }

  // No field, no spill, no halo behind the row: the page's backdrop is the
  // picture this light is drawn on. Alpha is the row's own coverage.
  emitA(outC * uEnergy, outA);
}
`,

  create({ preset, state, flash, params, opts }) {
    // Three slow zone gains: each zone brought to the same working level, so
    // the bass keeps its own life instead of sitting under the mids.
    const zone = [0.2, 0.2, 0.2];
    const sums = [0, 0, 0];
    const cnt = [0, 0, 0];
    let drop = -1e9;
    const ceiling = Math.max(24, preset.bars || 64);
    const pitch = Math.max(6, params?.pitch ?? 16);
    return {
      step(dt, m, c) {
        const s = c.spec;
        if (s) {
          const n = s.length;
          sums[0] = sums[1] = sums[2] = 0;
          cnt[0] = cnt[1] = cnt[2] = 0;
          for (let i = 0; i < n; i++) {
            const z = i < n / 3 ? 0 : i < (2 * n) / 3 ? 1 : 2;
            sums[z] += s[i];
            cnt[z]++;
          }
          for (let z = 0; z < 3; z++) {
            const mean = cnt[z] ? sums[z] / cnt[z] : 0;
            zone[z] = m.ease(zone[z], mean, 2, dt);
            state[z] = 0.7 / Math.max(0.1, zone[z]);
          }
        }
        // As many bands as the frame has room for, at a CONSTANT pitch in CSS
        // pixels: one per `pitch` in the strip, one per 20.5 full screen. A
        // row of beads is a physical width on any display, and the count is
        // stable while the window does not change. Always under the tier's
        // ceiling, so a smaller device draws fewer beads, not thinner ones.
        const aspect = c.aspect || 16 / 9;
        const width = c.width || aspect * 100;
        const want = opts?.layout === "strip" ? width / pitch : width / 20.5;
        state[3] = Math.round(Math.min(ceiling, Math.max(18, want)));
        if (m.stamp.drop !== drop) {
          drop = m.stamp.drop;
          flash(0.5);
        }
      },
    };
  },
};
