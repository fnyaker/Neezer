// The one definition of "a phone", shared by the shell and the player.
//
// The breakpoint is a SINGLE number: 640 px, the width under which the shell
// drops the sidebar and the desktop player for the tab bar + mini player
// (DESIGN §9). CSS media queries cannot import a constant, so the same number
// is written in `src/mobile.css` and in the components' own `@media` blocks;
// this module is what JavaScript reads, so a screen never invents a second
// breakpoint of its own.
//
// Zero dependency, safe to import before the app mounts (a bench, a test in
// node without a DOM): `isMobile()` answers false when there is no window.

export const MOBILE_MQ = "(max-width: 640px)";

/** Is this a phone-sized viewport (CSS pixels)? */
export function isMobile() {
  if (typeof window === "undefined" || !window.matchMedia) return false;
  return window.matchMedia(MOBILE_MQ).matches;
}

/**
 * Call `fn` with the current match state, then on every change.
 * Returns the unsubscribe function (never throws when there is no window).
 */
export function onMobileChange(fn) {
  if (typeof window === "undefined" || !window.matchMedia) return () => {};
  const mq = window.matchMedia(MOBILE_MQ);
  const handler = () => fn(mq.matches);
  handler(); // the caller gets the state it is subscribing to, not only changes
  if (mq.addEventListener) {
    mq.addEventListener("change", handler);
    return () => mq.removeEventListener("change", handler);
  }
  // iOS Safari < 14 and the Android WebView still only have addListener.
  mq.addListener(handler);
  return () => mq.removeListener(handler);
}
