// The equalizer card's own rules, without a browser: the built-in curves, what
// makes a saved preset "the one I am on", what "Mettre à jour" is allowed to
// write, the meter's risk vocabulary, and the lifetime of the meter's one
// subscription. `components/AudioEffects.svelte` is the only consumer;
// `test/eqpresets.test.mjs` holds these rules on their own.
//
// WHY THE PRESET RULES MOVED OUT. They were inline in the card's markup, where
// none of them could be tested without a browser — and "update this preset"
// did not exist at all: re-saving under the same name was the only way to
// change one, which is a different gesture (it silently overwrites whatever
// else that name held) and cannot be offered on a built-in.
//
// THE UPDATE CONTRACT, and why it is narrow: "Mettre à jour" writes the ten
// band gains and the bass lift, and ONLY the fields the preset already carries
// (a preset stored by an older build without a curve keeps its shape rather
// than gaining a half-defined one). It leaves the preset's EQ on/off flag and
// its normalization level alone: those describe how the preset is applied, not
// the curve the person just shaped. The card shows "modifié" from exactly the
// same comparison, so what looks dirty is what gets written.
//
// A built-in is never writable. It has no stored row; "Enregistrer comme…"
// creates a user preset from the current chain instead.

/** The ten sliders, low to high: the count every rule below assumes. */
export const EQ_BAND_COUNT = 10;

/** The curves shipped with the app. `label` is the old field name kept as-is. */
export const BUILTIN_PRESETS = Object.freeze(
  [
    { id: "flat", label: "Plat", bands: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0] },
    { id: "bass", label: "Basses+", bands: [6, 5, 4, 2, 0, 0, 0, 0, 0, 0] },
    { id: "vocal", label: "Voix", bands: [-2, -1, 0, 2, 4, 4, 3, 1, 0, -1] },
    { id: "treble", label: "Aigus+", bands: [0, 0, 0, 0, 0, 1, 2, 4, 5, 6] },
    { id: "loud", label: "Loudness", bands: [6, 4, 2, 0, -1, -1, 0, 2, 4, 6] },
  ].map((p) => Object.freeze({ ...p, bands: Object.freeze(p.bands) }))
);

const BUILTIN_IDS = new Set(BUILTIN_PRESETS.map((p) => p.id));

const clamp = (v, lo, hi) => (v < lo ? lo : v > hi ? hi : v);
const bandArray = (v) =>
  Array.isArray(v) && v.length === EQ_BAND_COUNT && v.every((n) => Number.isFinite(+n)) ? v : null;
const sameBass = (a, b) => Math.abs((+a || 0) - (+b || 0)) < 0.005;

/**
 * The list the card renders: the five built-ins (flagged) then the person's
 * own, both with a `name` so the markup has one field to read. Malformed rows
 * from an old storage are dropped rather than painted as broken chips.
 */
export function presetItems(customPresets = []) {
  const builtins = BUILTIN_PRESETS.map((p) => ({
    ...p,
    bands: p.bands.slice(),
    name: p.label,
    builtin: true,
  }));
  const mine = (Array.isArray(customPresets) ? customPresets : [])
    .filter((p) => p && typeof p.id === "string" && p.id)
    .map((p) => ({ ...p, name: String(p.name || "Sans nom"), builtin: false }));
  return [...builtins, ...mine];
}

/** A built-in cannot be updated or deleted: it lives in the code, not storage. */
export function isBuiltinPreset(p) {
  return !!p && (p.builtin === true || BUILTIN_IDS.has(p.id));
}

/** A stored preset the person owns: the only kind "Mettre à jour" touches. */
export function isUserPreset(p) {
  return !!p && !isBuiltinPreset(p) && typeof p.id === "string" && p.id !== "";
}

/** Same ten gains, in order. False for a curve of the wrong length. */
export function bandsEqual(a, b) {
  const x = bandArray(a);
  const y = bandArray(b);
  if (!x || !y) return false;
  return x.every((v, i) => +v === +y[i]);
}

/**
 * A saved preset is the current chain when it agrees on everything it
 * restores: EQ on/off, the curve, the bass lift and the normalization level.
 */
export function presetMatches(p, chain) {
  if (!p || !chain) return false;
  if (!!p.eq !== !!chain.eq) return false;
  if (p.norm !== chain.norm) return false;
  if (!sameBass(p.bass, chain.bass)) return false;
  return bandsEqual(p.bands, chain.bands);
}

/**
 * The preset the card highlights. A user preset wins over a built-in with the
 * same curve: it also carries the EQ flag and the normalization level, so it
 * is the more precise answer. A built-in matches on its curve alone, which is
 * all it prescribes.
 */
export function activePresetId(items, chain) {
  const custom = (items || []).find((p) => !isBuiltinPreset(p) && presetMatches(p, chain));
  if (custom) return custom.id;
  const built = (items || []).find((p) => isBuiltinPreset(p) && bandsEqual(p.bands, chain?.bands));
  return built ? built.id : null;
}

/** Does the preset carry anything "Mettre à jour" could write? */
export function presetUpdatable(p) {
  if (!isUserPreset(p)) return false;
  return !!bandArray(p.bands) || Number.isFinite(+p.bass);
}

/**
 * Would "Mettre à jour" change the preset? Only the two fields it writes are
 * compared — a preset whose normalization level differs from the live chain is
 * not "modified" for this button, since pressing it would not change that.
 */
export function presetNeedsUpdate(p, chain) {
  if (!isUserPreset(p) || !chain) return false;
  if (bandArray(p.bands) && !bandsEqual(p.bands, chain.bands)) return true;
  if (Number.isFinite(+p.bass) && !sameBass(p.bass, chain.bass)) return true;
  return false;
}

/**
 * The preset to store for "Mettre à jour", or null when it cannot be written
 * (a built-in, a missing preset, nothing updatable). Pure: the caller hands the
 * result back to the `fxPresets` store.
 */
export function updatedPreset(p, chain) {
  if (!presetUpdatable(p) || !chain) return null;
  const next = { ...p };
  if (bandArray(p.bands) && bandArray(chain.bands)) next.bands = chain.bands.map((v) => +v || 0);
  if (Number.isFinite(+p.bass) && Number.isFinite(+chain.bass))
    next.bass = clamp(+chain.bass, 0, 1);
  return next;
}

// -- the meter's vocabulary ---------------------------------------------------

/** The three states the engine reports; anything else is treated as safe. */
export const RISK_TONES = Object.freeze(["ok", "warn", "clip"]);

/**
 * The risk as a CSS tone. Unknown values fall back to "ok" rather than to a
 * warning: a bug in a reading must not paint the screen red.
 */
export function riskTone(risk) {
  return risk === "warn" || risk === "clip" ? risk : "ok";
}

// French decimals with a real minus sign, one decimal as the meter publishes
// them. "-0.0" is not a reading: anything under half a tenth is zero.
const MINUS = "\u2212";
export function fmtDb(value, { plus = false, unit = true } = {}) {
  const n = Number.isFinite(+value) ? +value : 0;
  const shown = Math.abs(n) < 0.05 ? 0 : n;
  const body = Math.abs(shown).toFixed(1).replace(".", ",");
  const sign = shown < 0 ? MINUS : plus && shown > 0 ? "+" : "";
  return unit ? `${sign}${body} dB` : `${sign}${body}`;
}

/**
 * How much to pull the level back to release the limiter: the engine's margin,
 * rounded to half a decibel (a whole number of clicks on a phone), never below
 * 0.5 so the button always does something.
 */
export function volumeCutDb(marginDb) {
  const m = Math.max(0, +marginDb || 0);
  return Math.max(0.5, Math.round(m * 2) / 2);
}

/** dB of attenuation as a linear gain (0.5 for -6 dB, 1 for 0). */
export function dbToGain(db) {
  return Math.pow(10, -Math.max(0, +db || 0) / 20);
}

// -- the meter's subscription, on a leash --------------------------------

/**
 * One subscription, startable and stoppable. The card starts it when the
 * equalizer is on screen and the page is visible, and stops it otherwise: the
 * engine's tap is cheap but not free (+0.46 s of main thread a minute), and a
 * settings screen left open behind another tab must not pay for a meter nobody
 * is looking at. `stop` is idempotent and `start` after a `stop` subscribes
 * again, which is what returning to the card does.
 */
export function createMeterSession(subscribe, onSnapshot) {
  let off = null;
  return {
    get running() {
      return !!off;
    },
    start() {
      if (off) return false;
      const stop = subscribe(onSnapshot);
      off = typeof stop === "function" ? stop : () => {};
      return true;
    },
    stop() {
      if (!off) return false;
      const stop = off;
      off = null;
      try {
        stop();
      } catch {
        /* a fake subscriber in a test may throw; the session still ends */
      }
      return true;
    },
  };
}
