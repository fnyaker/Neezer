export const SEEK_LAND_MS = 4000;
export const SEEK_TOLERANCE = 1.5;

export function seekableAt(el, t) {
  try {
    const s = el.seekable;
    for (let i = 0; i < s.length; i++) {
      // A range that ends at Infinity is a live stream saying "anywhere": it can
      // seek nowhere, and taking it for a real range is what makes a page seek
      // restart the stream instead of chasing it.
      if (isFinite(s.end(i)) && t >= s.start(i) && t <= s.end(i)) return true;
    }
  } catch {
    /* an element without ranges yet */
  }
  return false;
}

export function bufferedAt(el, t) {
  try {
    const b = el.buffered;
    for (let i = 0; i < b.length; i++) {
      if (t >= b.start(i) && t <= b.end(i)) return true;
    }
  } catch {
    /* not ready */
  }
  return false;
}

export function finiteDuration(el) {
  const d = el && el.duration;
  return typeof d === "number" && isFinite(d) && d > 0 ? d : 0;
}

// "direct": the browser will fetch the rest of the file for that position.
// "defer": no metadata yet; the seek lands on loadedmetadata (safeSeek).
// "chase": the source cannot be seeked there without restarting it.
export function seekPlan(el, t, { direct = false } = {}) {
  if (!el) return "chase";
  const target = Math.max(0, +t || 0);
  if (direct) return "direct";
  if (seekableAt(el, target)) return "direct";
  if (bufferedAt(el, target)) return "direct";
  const d = finiteDuration(el);
  const ready = typeof el.readyState === "number" ? el.readyState : 0;
  if (ready < 1) return "defer";
  if (d && target <= d) return "direct";
  return "chase";
}

// A seek that went backwards to the start is a source that re-fetched itself:
// the stream had no ranges, so the browser gave up its position for byte 0.
export function seekBounced(el, target) {
  const t = typeof el?.currentTime === "number" ? el.currentTime : 0;
  return target > 2 && t < 1;
}

export function seekLost(el, target, startedAt, now = Date.now()) {
  if (now - startedAt < SEEK_LAND_MS) return false;
  const t = typeof el?.currentTime === "number" ? el.currentTime : 0;
  if (Math.abs(t - target) <= SEEK_TOLERANCE) return false;
  const ready = typeof el?.readyState === "number" ? el.readyState : 0;
  return ready >= 2 || seekBounced(el, target);
}

export const SEEK_WAIT_MS = 1000;
export const SEEK_WAIT_STREAMING_MS = 1500;

export function readiness(body, status = 200) {
  if (status === 409 && body && body.ranges === "bytes") return { mode: "ready", retryMs: 0 };
  if (status === 400) return { mode: "stop", retryMs: 0 };
  if (status >= 500) return { mode: "wait", retryMs: SEEK_WAIT_STREAMING_MS };
  if (!body || typeof body !== "object") return { mode: "wait", retryMs: SEEK_WAIT_MS };
  if (body.ready === true) return { mode: "ready", retryMs: 0 };
  if (body.offset === true) return { mode: "offset", retryMs: 0 };
  return { mode: "wait", retryMs: body.streaming ? SEEK_WAIT_STREAMING_MS : SEEK_WAIT_MS };
}

export function trackTimeOf(position, offset) {
  const p = +position;
  return (isFinite(p) ? p : 0) + (offset > 0.05 ? +offset : 0);
}

export function elementTimeOf(position, offset) {
  const p = +position;
  return Math.max(0, (isFinite(p) ? p : 0) - (offset > 0.05 ? +offset : 0));
}
