<script>
  // The app's ground: ONE fixed, blurred, low-resolution sleeve behind every
  // screen — Accueil, Bibliothèque, Rechercher, Réglages, an album, a playlist,
  // an artist — so the shell reads as one room instead of a set of pages.
  //
  // THE RULES THIS FILE EXISTS FOR (they were expensive elsewhere):
  //
  //   - ONE image, ONE blur. The sleeve is a 500px source stretched to the
  //     viewport and blurred once. The blur lives on THIS layer, never on a
  //     card: `backdrop-filter` samples whatever is behind the element on every
  //     repaint, and a full-viewport one is the most expensive effect a page
  //     can carry. A `filter` on a static fixed layer is rasterised once per
  //     cover and composited afterwards.
  //   - ZERO requests that the app was not already making. The URL is the
  //     500px art the lists and the play cache already fetch, resolved through
  //     the offline cover blobs when the device holds them (lib/globalbackdrop.js).
  //   - ONE fade, 640 ms, and no flash. The layer fades out (220 ms), the new
  //     art and its tint go in at the invisible point, and the layer fades back
  //     (420 ms). The scrim never moves, so the ground is never the bare page
  //     colour — the old full-screen lesson: a layer dropped mid-fade is a
  //     flash, a layer swapped under a fixed scrim is a walk. `prefers-reduced-
  //     motion` swaps at once.
  //   - ECO MEANS NOTHING. No image element, no palette decode, no blur, no
  //     animation: the theme's flat ground (app.css's `html` background) is the
  //     whole backdrop, and `data-eco="1"` (App.svelte) tells the translucent
  //     surfaces to drop their own blur.
  //   - ONE LAYER AT A TIME. The full-screen player has its own blurred ground
  //     (DesktopNowPlaying / MobileNowPlaying); while it is open this one is
  //     removed from the DOM, so the compositor never holds two full-viewport
  //     blurred layers or two copies of the same art.
  //   - A TRACK WITH NO ART FALLS BACK TO THE THEME'S GROUND (faded out, not
  //     left showing the previous sleeve); a sleeve that FAILS to load keeps
  //     the previous ground, because a flaky CDN must not blank the page.
  //
  // The cover is keyed on `currentId` — a STRING — never on `current`, whose
  // object is re-emitted on every progress tick: a reactive statement reading
  // it asked for the credits four times a second once (docs/agents/webapp.md).
  import { onMount, onDestroy } from "svelte";
  import { get } from "svelte/store";
  import { current, currentId, offlineCovers, ecoMode, immersiveOpen } from "../lib/stores.js";
  import { paletteFor } from "../lib/coverpalette.js";
  import { backdropCover } from "../lib/globalbackdrop.js";
  import { cssUrl } from "../lib/format.js";
  import { rgb, levelFactor } from "../lib/color.js";
  import { api } from "../lib/api.js";
  import { trackRef } from "../lib/trackkey.js";

  // The fade's two halves. OUT + IN is the ~640 ms walk the brief asks for;
  // the CSS below carries the same two numbers, and IN must not exceed 600 ms
  // or the layer's own transition ends after the next skip can start.
  const OUT_MS = 220;
  const IN_MS = 420;

  let src = null; // the art actually painted
  let tint = null; // its dominant colour, or null when the palette could not be read
  let bright = 1; // its LEVEL, normalised: a white sleeve comes down, a black one up
  let fading = false; // the art group is at opacity 0 (fading out)
  let painted = false; // has this layer ever painted a sleeve?
  let wanted = null; // the URL being loaded, so a second call is a no-op
  let target = null; // [url, tint] the fade-out will paint, newest wins
  let failed = false; // the last attempt failed, so a later cached blob may rescue it
  let seq = 0; // invalidates every answer that is no longer the newest
  let timer = null;
  let disposed = false;
  let started = false; // the first paint has been scheduled
  let scheduled = false;

  const reducedMotion = () =>
    typeof window !== "undefined" && window.matchMedia
      ? window.matchMedia("(prefers-reduced-motion: reduce)").matches
      : false;

  function stopFade() {
    clearTimeout(timer);
    timer = null;
    target = null;
    fading = false;
  }

  function paint(url, colour, factor) {
    src = url;
    tint = colour;
    bright = factor || 1;
    painted = !!url;
  }

  // A track with NO art gets the theme's flat ground, not the previous sleeve:
  // the ground is a claim about what is playing, and a stale colour is a lie.
  // (An art that FAILS to load is different: the previous ground stays, so a
  // flaky CDN never blanks the page.)
  function clearArt() {
    seq++; // whatever was loading belongs to a track that is gone
    if (!painted && !src) {
      wanted = null;
      stopFade();
      return;
    }
    if (reduceMotion()) {
      stopFade();
      paint(null, null, 1);
      wanted = null;
      return;
    }
    target = [null, null, 1];
    if (fading) return;
    fading = true;
    timer = setTimeout(() => {
      timer = null;
      const next = target;
      target = null;
      if (next) paint(next[0], next[1], next[2]);
      wanted = null;
      fading = false;
    }, OUT_MS);
  }

  // The one place the fade's state machine lives. A target that arrives during
  // the fade-out REPLACES the pending one (the timer paints the newest); one
  // that arrives during the fade-in starts a fresh fade-out from where it is.
  function reveal(url, colour, factor) {
    if (!painted || reducedMotion()) {
      stopFade();
      paint(url, colour, factor);
      return;
    }
    target = [url, colour, factor];
    if (fading) return;
    fading = true;
    timer = setTimeout(() => {
      timer = null;
      const next = target;
      target = null;
      if (next) paint(next[0], next[1], next[2]);
      fading = false; // the CSS transition fades the new art back in
    }, OUT_MS);
  }

  // Which bytes to paint, and with what tint.
  //
  // The palette load IS the preload — one image, one readback — so the art can
  // never arrive twice for the same sleeve. When the direct URL cannot be read
  // (the CDN dropped it, or a host sends no CORS headers for a cross-origin
  // cover), the same-origin copy is tried: `/api/cover/<id>` is the server's
  // archived art, it costs one request ONLY when the CDN failed, and it is the
  // copy Cover.svelte and the play cache already fetch — so in the common case
  // it is an HTTP-cache hit, not a new load.
  async function resolveArt(track, direct) {
    const palette = await paletteFor(direct);
    if (palette) return { url: direct, tint: palette.dominant, bright: levelFactor(palette.average) };
    const ref = trackRef(track);
    const proxy = ref ? api.coverUrl(ref) : null;
    if (!proxy || proxy === direct) return null;
    const copy = await paletteFor(proxy);
    return copy ? { url: proxy, tint: copy.dominant, bright: levelFactor(copy.average) } : null;
  }

  async function point() {
    if (disposed || get(ecoMode)) return;
    const track = get(current);
    if (!track) return;
    const direct = backdropCover(track, get(offlineCovers));
    if (!direct) {
      // No sleeve: the theme's own ground is the honest answer.
      clearArt();
      return;
    }
    // `wanted` is the LOGICAL cover the track wants — the proxy that may paint
    // instead is a byte source, not another sleeve, so it must not make the
    // next call think the ground is out of date.
    if (direct === src || direct === wanted) return;
    const token = ++seq;
    wanted = direct;
    const art = await resolveArt(track, direct);
    if (disposed || token !== seq || get(ecoMode)) return;
    if (!art) {
      // Nothing readable: keep the previous ground and let a cached blob, a
      // later track or the same sleeve arriving offline try again.
      failed = true;
      wanted = null;
      return;
    }
    failed = false;
    reveal(art.url, art.tint, art.bright);
  }

  onMount(() => {
    // The FIRST ground is decoration and must not compete with the screen that
    // is painting: it waits for an idle moment (bounded, so it is never
    // missing). Every later track change points at once — that is the person's
    // action. Measured at 4× CPU on a cold boot, this keeps the ground's decode
    // and blur raster off the first screen's long tasks.
    const kick = () => {
      if (started || scheduled || disposed) return;
      scheduled = true;
      const go = () => {
        scheduled = false;
        if (started || disposed) return;
        started = true;
        point();
      };
      if (typeof requestIdleCallback === "function") requestIdleCallback(go, { timeout: 900 });
      else setTimeout(go, 350);
    };

    // Eco FIRST: its immediate value decides whether anything may be loaded at
    // all, and a phone that boots in eco must not start a decode before the
    // switch is read.
    const offEco = ecoMode.subscribe((on) => {
      seq++;
      stopFade();
      if (on) {
        // No image in the DOM, no palette in flight, no blurred layer to keep.
        src = null;
        tint = null;
        painted = false;
        wanted = null;
        failed = false;
      } else if (started) {
        point();
      } else {
        kick();
      }
    });
    const offTrack = currentId.subscribe(() => (started ? point() : kick()));
    // A sleeve that lands in the offline cache after the fact (an airplane-mode
    // boot, a download finishing) is the only reason to try again: the URL
    // itself changes to the blob, and a failed attempt left nothing painted.
    const offCovers = offlineCovers.subscribe(() => {
      if (started && (!painted || failed)) point();
    });
    return () => {
      disposed = true;
      offEco();
      offTrack();
      offCovers();
      clearTimeout(timer);
    };
  });
</script>

<!--
  THE GROUND PAINTS BETWEEN THE CANVAS AND THE PAGES.
  `html` carries the theme's ground (app.css), `<body>` is transparent, and this
  element takes no room in the shell's layout. Its two fixed layers carry
  `z-index: -1`, so they are painted in the root stacking context's negative
  step: above the canvas, below every card, row, title and bar of the app.
  That is the whole trick, and it has one condition — this element must NOT
  create a stacking context. `isolation: isolate` here put the ground ABOVE the
  in-flow text of every page and washed the titles out (seen in the screenshots;
  the bench's title probe now reads 228/255 against an ink of 241); a
  `position` or a `z-index` would do the same.
-->
<div class="gb" class:hidden={$immersiveOpen} aria-hidden="true" style={`--gb-in:${IN_MS}ms;--gb-out:${OUT_MS}ms`}>
  {#if !$ecoMode && src}
    <!-- ONE image element, one blur. The sleeve's own colour is a pseudo-element
         of the same group, so it fades in and out with the art and the layer
         count stays one image + one scrim. -->
    <div
      class="gb-art"
      class:fading
      style={`--gb-tint:${tint ? rgb(tint, 0.32) : "transparent"}`}
    >
      <div class="gb-img" style={`background-image:${cssUrl(src)}; --gb-bright:${bright}`}></div>
    </div>
  {/if}
  {#if !$ecoMode}
    <div class="gb-scrim"></div>
  {/if}
</div>

<style>
  .gb {
    /* No box of its own: it only owns the fixed layers below. */
    position: static;
    height: 0;
    pointer-events: none;
    /* NO stacking context, no position, no z-index: see the markup comment. */
  }
  /* The full-screen player brings its own ground: this one leaves the DOM so
     the compositor never holds two full-viewport blurred layers. */
  .gb.hidden {
    display: none;
  }

  .gb-art,
  .gb-scrim {
    position: fixed;
    inset: 0;
    z-index: -1;
    pointer-events: none;
    /* It is decoration: never a selection target. */
    user-select: none;
  }

  .gb-art {
    overflow: hidden;
    opacity: 1;
    /* The duration comes from the script (IN_MS), so the JS fade-out and the
       CSS fade-in can never disagree. */
    transition: opacity var(--gb-in, 420ms) cubic-bezier(0.4, 0, 0.2, 1);
    /* The first ground arrives after an idle moment, so it is a short fade-in
       rather than a colour appearing under the reader's eyes. `backwards` (not
       `both`) so the animation stops overriding the transition once it ends. */
    animation: gb-intro 600ms cubic-bezier(0.4, 0, 0.2, 1) backwards;
  }
  @keyframes gb-intro {
    from {
      opacity: 0;
    }
  }
  .gb-art.fading {
    opacity: 0;
    transition-duration: var(--gb-out, 220ms);
  }

  .gb-img {
    position: absolute;
    inset: 0;
    background-size: cover;
    background-position: center;
    /* THE one blur of the shell. It sits on a fixed layer whose content only
       changes when a sleeve does, so it is rasterised once and composited —
       not re-run on scroll, not re-run per frame. Scale pushes the blur's
       transparent halo (the edge of the element) outside the viewport. */
    /* The blur and the tint are the ground; `--gb-bright` is the sleeve's
       LEVEL (lib/color.js#levelFactor): a white cover is pulled down so the
       page never becomes a wash, a black one lifted so it is not a hole, and
       art between the two bands is left alone. One pass, no extra filter. */
    filter: blur(44px) saturate(1.3) brightness(var(--gb-bright, 1));
    transform: scale(1.14);
    transform-origin: center;
  }
  /* The sleeve's own colour, over the blurred image: the ground is tinted even
     where the art is nearly monochrome, and a sleeve's colour is on screen
     while its bytes are still arriving. NOT blurred (it is already soft) and
     not a second image — a flat gradient of the same layer group. */
  .gb-art::after {
    content: "";
    position: absolute;
    inset: 0;
    pointer-events: none;
    background: radial-gradient(
      90% 70% at 50% 0%,
      var(--gb-tint, transparent) 0%,
      transparent 100%
    );
  }

  /* The stable half: it never changes with the track, so the ground cannot
     flash between two sleeves. 86 % of the theme's ground in the reading area
     keeps `--dim` above 4.5:1 even over a pure-white sleeve (measured in the
     bench); the edges close to ~95 %, which is the vignette that makes the
     centre read as lit by the cover. */
  .gb-scrim {
    background:
      radial-gradient(
        125% 100% at 50% 40%,
        transparent 32%,
        color-mix(in srgb, var(--bg) 65%, transparent) 100%
      ),
      linear-gradient(
        180deg,
        color-mix(in srgb, var(--bg) 88%, transparent) 0%,
        color-mix(in srgb, var(--bg) 86%, transparent) 55%,
        color-mix(in srgb, var(--bg) 91%, transparent) 100%
      );
  }

  @media (prefers-reduced-motion: reduce) {
    .gb-art,
    .gb-art.fading {
      transition: none;
      animation: none;
    }
  }
</style>
