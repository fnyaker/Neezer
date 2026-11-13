// MONO and LEFT/RIGHT BALANCE — for the AUDIBLE output only.
//
// WHERE IT LIVES. The processed path inserts this stage in graph.js, between
// the graph's output node and its look-ahead delay:
//
//   … → limiter → output ─┬─ [mono gain → stereo panner] → look-ahead → destination
//                         └─ analysisBus → analysers, scope, rhythm   (PRE-stage)
//
// The readers of the sound tap the OUTPUT node itself, so the rhythm analyser,
// the stereo scope and the visualizer keep the ORIGINAL stereo signal: a mono
// fold would erase exactly the two different channels the scope exists to draw,
// and a balance would move an image the animation is not supposed to inherit.
// Nothing here changes what they read. The two nodes sit in the path only while
// mono or balance is engaged; with both off the graph connects its output
// straight to the look-ahead, so the default path keeps its latency and its CPU
// cost.
//
// On the DIRECT path (lib/audio/output.js) and with the Android app's own
// player there is no graph to insert the stage into: the settings card disables
// these controls there and says « Disponible en lecture traitée. ».
//
// THE MATHS (W3C Web Audio API §6.3.3, the StereoPannerNode algorithm, and
// §4.5, the speaker down-mix):
//
//   mono   = 0.5 · (L + R)                  the spec's stereo→mono down-mix. A
//                                            centred signal keeps its amplitude;
//                                            an out-of-phase pair cancels, which
//                                            is the honest result of a mono fold.
//   pan    = clamp(balance, -1, 1)          -1 hard left … 0 centre … 1 hard right.
//   gains  = cos((pan + 1)·π/4), sin((pan + 1)·π/4)
//                                            the equal-power law of a
//                                            StereoPannerNode fed a MONO input.
//                                            At centre that is 1/√2 on each
//                                            side: gL² + gR² = 1 everywhere, so
//                                            the signal keeps its power while it
//                                            moves between the speakers instead
//                                            of dipping 3 dB in the middle —
//                                            the constant-power law every
//                                            console pan pot uses.
//
//     That is the law the graph gets when mono is on, because the down-mix node
//     feeds the panner. With mono off, the panner receives the original STEREO
//     signal and the spec's stereo branch applies instead (centre is unity; the
//     attenuated far channel is folded into the near one). That is what the
//     platform defines for a stereo balance, and it keeps both channels audible
//     at a hard pan.
//
// THE LEVEL. At centre the mono signal is played at 1/√2 per side: the
// constant-power centre, i.e. 3 dB below the same signal at full gain on one
// side, and 3 dB below the centred stereo original in total power. It is NOT
// compensated: the stage sits AFTER the safety limiter (which the analysers
// must keep reading), and a √2 make-up would put a full-scale mono fold 3 dB
// over the limiter's ceiling at a hard pan. A mono signal can never clip this
// stage as it stands. The volume control is right above it.
//
// The two settings are persisted under `audio.mono` (boolean) and
// `audio.balance` (number, -1..1), read and written HERE with plain
// localStorage — panning is a property of this output, not of the player, so it
// gets no entry in stores.js. `panSettings` is a small readable store (the
// `subscribe` contract Svelte needs) that the graph and the settings card share.

export const MONO_KEY = "audio.mono";
export const BALANCE_KEY = "audio.balance";

/** The spec's stereo→mono coefficient: mono = MONO_GAIN · (L + R). */
export const MONO_GAIN = 0.5;

/** Half a percent is "centre": below that the slider is at its detent. */
export const CENTRE_EPS = 0.005;

/** The spec's equal-power pair for a MONO source panned by `balance`. */
export function panGains(balance) {
  const x = ((clampBalance(balance) + 1) * Math.PI) / 4;
  return { left: Math.cos(x), right: Math.sin(x) };
}

/** The spec's stereo→mono down-mix, one sample: 0.5 · (L + R). */
export function monoDownmix(left, right) {
  return MONO_GAIN * ((+left || 0) + (+right || 0));
}

/** -1 … 1, with anything unreadable landing at the centre. */
export function clampBalance(value) {
  const n = typeof value === "number" ? value : parseFloat(String(value ?? "").replace(",", "."));
  if (!Number.isFinite(n)) return 0;
  return n <= -1 ? -1 : n >= 1 ? 1 : n;
}

/** A stored/boolean-ish value → the mono switch. Unknown means off. */
export function parseMono(raw) {
  if (raw === true || raw === 1) return true;
  if (raw == null) return false;
  const s = String(raw).trim().toLowerCase().replace(/^"|"$/g, "");
  return s === "true" || s === "1" || s === "on" || s === "yes";
}

/** A stored value → a clamped balance; malformed or missing means centre. */
export function parseBalance(raw) {
  if (raw == null || raw === "") return 0;
  return clampBalance(raw);
}

/** "Centre", "30 % à gauche", "À droite" — the readout under the slider. */
export function describeBalance(balance) {
  const b = clampBalance(balance);
  if (Math.abs(b) < CENTRE_EPS) return "Centre";
  if (b <= -0.995) return "À gauche";
  if (b >= 0.995) return "À droite";
  const pct = Math.round(Math.abs(b) * 100);
  return `${pct} % à ${b < 0 ? "gauche" : "droite"}`;
}

/** The whole stage in one line, for a status sentence. */
export function describePan(settings) {
  const s = settings || {};
  return `${s.mono ? "Mono" : "Stéréo"} · ${describeBalance(s.balance)}`;
}

function defaultStorage() {
  try {
    return typeof localStorage !== "undefined" ? localStorage : null;
  } catch {
    return null;
  }
}

/**
 * The settings as stored. `storage` is injectable so the rules can be tested
 * without a browser; omit it to use this device's localStorage. A storage that
 * throws (private mode, a full quota) reads as the defaults.
 */
export function readPanSettings(storage = defaultStorage()) {
  if (!storage) return { mono: false, balance: 0 };
  let mono = null;
  let balance = null;
  try {
    mono = storage.getItem(MONO_KEY);
    balance = storage.getItem(BALANCE_KEY);
  } catch {
    /* unreadable storage: the defaults */
  }
  return { mono: parseMono(mono), balance: parseBalance(balance) };
}

/**
 * Write the settings, clamped and normalised, and answer what was written.
 * `storage` is injectable for the tests. A failed write (private mode) still
 * answers the normalised value, so the live setting works for this session.
 */
export function writePanSettings(settings, storage = defaultStorage()) {
  const s = settings || {};
  const clean = { mono: !!s.mono, balance: clampBalance(s.balance) };
  if (storage) {
    try {
      storage.setItem(MONO_KEY, clean.mono ? "true" : "false");
      storage.setItem(BALANCE_KEY, String(Math.round(clean.balance * 1000) / 1000));
    } catch {
      /* the setting still applies for this session */
    }
  }
  return clean;
}

// -- the live setting --------------------------------------------------------

let current = null;
const listeners = new Set();

function emit() {
  for (const fn of [...listeners]) {
    try {
      fn(current);
    } catch {
      /* a listener never breaks the write */
    }
  }
}

/** The settings this page is using right now. */
export function getPanSettings() {
  if (!current) current = readPanSettings();
  return current;
}

/**
 * A Svelte-readable store over the persisted settings. Deliberately not in
 * stores.js: panning belongs to the audible output of this device, and a
 * remote control must not carry it to the player it drives.
 */
export const panSettings = {
  subscribe(run) {
    listeners.add(run);
    run(getPanSettings());
    return () => listeners.delete(run);
  },
};

/** Replace (a patch of) the settings, persist them, and tell every reader. */
export function applyPanSettings(patch) {
  const now = getPanSettings();
  const p = patch && typeof patch === "object" ? patch : {};
  current = writePanSettings({
    mono: "mono" in p ? p.mono : now.mono,
    balance: "balance" in p ? p.balance : now.balance,
  });
  emit();
  return current;
}

/** The mono switch. */
export function setMono(mono) {
  return applyPanSettings({ mono: !!mono });
}

/** The balance slider (-1 … 1). */
export function setBalance(balance) {
  return applyPanSettings({ balance });
}

/** The "Centre" reset. */
export function resetBalance() {
  return applyPanSettings({ balance: 0 });
}
