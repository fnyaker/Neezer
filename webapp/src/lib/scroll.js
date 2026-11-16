// Scroll a child element into view within a scroll container, keeping it at a
// fixed fraction down the viewport (default: the first quarter). Used so the
// active queue track / lyric line follows playback instead of scrolling off.
//
// The behaviour is the motion module's (lib/motion.js#scrollBehavior): smooth at
// full motion, an instant jump when eco is on or the OS asks for reduced motion
// — a glide is movement, and the plan's M14 removes it. `smooth: false` stays
// "now" whatever the level.
import { scrollBehavior } from "./motion.js";

export function followScroll(container, el, { ratio = 0.25, smooth = true } = {}) {
  if (!container || !el) return;
  const cr = container.getBoundingClientRect();
  const er = el.getBoundingClientRect();
  const top = container.scrollTop + (er.top - cr.top) - container.clientHeight * ratio;
  container.scrollTo({ top: Math.max(0, top), behavior: smooth ? scrollBehavior() : "auto" });
}
