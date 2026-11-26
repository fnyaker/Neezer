// How a gesture becomes a movement (docs/agents/transitions.md, plan §3).
//
// One small module, no dependency, shared by every SPA surface a finger drags:
// the phone player's dismissal and its « À propos » panel, the track row's
// "lire ensuite" pull, the cover carousel's re-centre and the recap deck.
// Before it, each of them repeated its own threshold-and-snap code and three of
// them jumped to 0 the instant the finger left (`dragY = 0`, `dx = 0`), which
// reads as the sheet/row being yanked out from under the finger.
//
// The model is Apple's "project momentum" release (WWDC 2018, "Designing Fluid
// Interfaces"): a release is not the end of the movement, it is a new movement
// that starts at the finger's position WITH the finger's velocity, and a damped
// spring carries it to the endpoint it was heading for. The numbers on our side
// are in docs/agents/transitions.md (S4), and they are the same ones the native
// app's `Spring.StiffnessMediumLow` anchors give, so the two apps settle alike.
//
//   - the velocity is the TRAILING 80 ms only (`tracker`) — an average seeded
//     with the finger planting stays low for a whole short flick;
//   - what a release is heading for is the finger's position plus its velocity
//     over PROJECT_MS (160 ms), so a fast 40 px flick passes a threshold a slow
//     90 px drag does not;
//   - `spring` is the closed-form solution of the damped oscillator, sampled by
//     `t`, so a 60 Hz and a 120 Hz display settle identically and a dropped
//     frame never changes the trajectory.

// -- velocity ----------------------------------------------------------------

/** How much of a gesture's tail describes how the finger left (plan §3). */
export const VEL_WINDOW_MS = 80;

/**
 * The moment an input event happened, on the page's monotonic clock. The
 * `timeStamp` is the input device's own, so a queued or coalesced event still
 * carries when the finger was there; a browser that reports 0 gets the handler's
 * clock instead.
 * @param {{timeStamp?: number}} e
 */
export function eventTime(e) {
  return e && Number.isFinite(e.timeStamp) && e.timeStamp > 0 ? e.timeStamp : performance.now();
}

/**
 * A trailing-window velocity tracker. `sample(x, t)` in the same units and clock
 * as the input events (px, and a monotonic ms clock — `performance.now()` at the
 * handler, because coalesced touch moves can share a `timeStamp`); `velocity()`
 * is px/ms, 0 while there is nothing to compare. Only the last `windowMs` are
 * used, and at least two points are always kept so a two-event flick still has a
 * velocity.
 * @param {number} [windowMs]
 */
export function tracker(windowMs = VEL_WINDOW_MS) {
  /** @type {[number, number][]} [t, x] */
  let samples = [];
  return {
    sample(x, t) {
      const now = Number.isFinite(t) ? t : performance.now();
      samples.push([now, x]);
      while (samples.length > 2 && now - samples[0][0] > windowMs) samples.shift();
    },
    /** px/ms, `(last - oldest in the window) / dt`; positive with x growing. */
    velocity() {
      if (samples.length < 2) return 0;
      const [t1, x1] = samples[samples.length - 1];
      // The window start is the baseline — unless it shares the newest sample's
      // timestamp (coalesced moves land in one task with one clock reading, and
      // a zero-length baseline would read as "no velocity" on a real flick).
      let ref = samples[0];
      if (t1 - ref[0] < 1) {
        ref = null;
        for (let i = 1; i < samples.length; i++) {
          if (t1 - samples[i][0] >= 1) {
            ref = samples[i];
            break;
          }
        }
        if (!ref) return 0;
      }
      const dt = t1 - ref[0];
      return dt > 0 ? (x1 - ref[1]) / dt : 0;
    },
    reset() {
      samples = [];
    },
  };
}

// -- projection --------------------------------------------------------------

/** How long a release is projected along its velocity (WWDC 2018). */
export const PROJECT_MS = 160;

/** Where a release at `v` px/ms is heading, in px from where it let go. */
export function projected(v, ms = PROJECT_MS) {
  return v * ms;
}

// -- the spring --------------------------------------------------------------

// A damped harmonic oscillator, x'' + 2ζω₀x' + ω₀²x = 0, solved in closed form:
//   x(t) = e^(-ζω₀t) · (x₀·cos(ω_d t) + ((v₀ + ζω₀x₀) / ω_d)·sin(ω_d t))
// with ω₀ = 2π / response, ω_d = ω₀√(1 - ζ²), x₀ = from - to, v₀ the release
// velocity in px/s. Closed form (not numeric integration) means a 60 Hz and a
// 120 Hz display agree, and a dropped frame is only a coarser sample of the
// same curve — never a different trajectory.
//
// ζ = 1 (critical damping) has its own solution and ζ > 1 (overdamped) too;
// `damping: 1` is what a snap-back uses precisely because it never crosses the
// target.
const STOP_PX = 0.5; // |x| below this…
const STOP_V = 5; // …and |x'| below this (px/s) = arrived

/**
 * The settle curve from `from` to `to`, seeded by `velocityPxPerS` (WITH the
 * direction of travel: positive = toward increasing px).
 * @param {number} from
 * @param {number} to
 * @param {number} velocityPxPerS
 * @param {{response?: number, damping?: number}} [opts]
 * @returns {{value: (t: number) => number, velocity: (t: number) => number, done: (t: number) => boolean}}
 */
export function spring(from, to, velocityPxPerS, { response = 0.32, damping = 0.82 } = {}) {
  const w0 = (2 * Math.PI) / Math.max(response, 1e-3);
  const z = Math.max(damping, 1e-3);
  const x0 = from - to;
  const v0 = velocityPxPerS;

  let value;
  let speed;

  if (Math.abs(z - 1) < 1e-6) {
    // critically damped: x(t) = e^(-ω₀t)·(x₀ + (v₀ + ω₀x₀)·t)
    const b = v0 + w0 * x0;
    value = (t) => {
      const e = Math.exp(-w0 * t);
      return to + e * (x0 + b * t);
    };
    speed = (t) => {
      const e = Math.exp(-w0 * t);
      return e * (b - w0 * (x0 + b * t));
    };
  } else if (z < 1) {
    // underdamped: the one that overshoots a little, which is what makes a
    // dismissal feel like it was thrown rather than placed.
    const wd = w0 * Math.sqrt(1 - z * z);
    const a = x0;
    const b = (v0 + z * w0 * x0) / wd;
    value = (t) => {
      const e = Math.exp(-z * w0 * t);
      return to + e * (a * Math.cos(wd * t) + b * Math.sin(wd * t));
    };
    speed = (t) => {
      const e = Math.exp(-z * w0 * t);
      return (
        e *
        (-z * w0 * (a * Math.cos(wd * t) + b * Math.sin(wd * t)) +
          wd * (b * Math.cos(wd * t) - a * Math.sin(wd * t)))
      );
    };
  } else {
    // overdamped: two real roots, no crossing either.
    const s = w0 * Math.sqrt(z * z - 1);
    const r1 = -z * w0 + s;
    const r2 = -z * w0 - s;
    const c2 = (v0 - r1 * x0) / (r2 - r1);
    const c1 = x0 - c2;
    value = (t) => to + c1 * Math.exp(r1 * t) + c2 * Math.exp(r2 * t);
    speed = (t) => c1 * r1 * Math.exp(r1 * t) + c2 * r2 * Math.exp(r2 * t);
  }

  return {
    value,
    velocity: speed,
    done: (t) => Math.abs(value(t) - to) < STOP_PX && Math.abs(speed(t)) < STOP_V,
  };
}

// -- who owns Escape ---------------------------------------------------------

// Two drags can be live at once (the phone sheet's dismissal and the queue's
// reorder, one inside the other), but there is ONE Escape: the finger that is
// down owns it. A claim replaces the previous owner (the row drag inside the
// sheet is the topmost layer), and a cancel never decides: it only puts the
// gesture back (S8, docs/agents/transitions.md).
let owner = null;

/** Make `cancel` the drag Escape answers to (the last claim wins). */
export function claimDrag(cancel) {
  owner = cancel;
}

/** The finger left (or the drag ended on its own): Escape has no owner again. */
export function releaseDrag() {
  owner = null;
}

/** Cancel whatever holds Escape; true when something was actually cancelled. */
export function cancelDragIfAny() {
  const c = owner;
  owner = null;
  if (c) {
    c();
    return true;
  }
  return false;
}

// -- settle ------------------------------------------------------------------

/** Hard stop: a spring that has not landed by then lands anyway. */
const MAX_SETTLE_MS = 3000;

/**
 * Play `spring(from, to, velocity)` on rAF and write each frame through `apply`,
 * then `done`. Returns a cancel function; call it on a new pointer down so a
 * new gesture interrupts the settle instead of fighting it (M2/S4).
 *
 * Under reduced motion or eco (`reduce`) the settle lands at once: `apply(to)`
 * then `done()` in the same tick, no rAF, no displacement (transitions.md M13/M14).
 *
 * @param {{from: number, to: number, velocity?: number, response?: number,
 *          damping?: number, reduce?: boolean,
 *          apply?: (v: number) => void, done?: () => void}} opts
 * @returns {() => void} cancel
 */
export function settle({
  from,
  to,
  velocity = 0,
  response = 0.32,
  damping = 0.82,
  reduce = false,
  apply,
  done,
}) {
  if (reduce) {
    apply?.(to);
    done?.();
    return () => {};
  }
  const s = spring(from, to, velocity, { response, damping });
  const origin = typeof performance !== "undefined" ? performance.now() : Date.now();
  let frame = null;
  let cancelled = false;
  const step = (now) => {
    if (cancelled) return;
    const t = Math.max(0, now - origin) / 1000;
    if (s.done(t) || now - origin > MAX_SETTLE_MS) {
      frame = null;
      apply?.(to);
      done?.();
      return;
    }
    apply?.(s.value(t));
    frame = requestAnimationFrame(step);
  };
  frame = requestAnimationFrame(step);
  return () => {
    cancelled = true;
    if (frame !== null) cancelAnimationFrame(frame);
    frame = null;
  };
}
