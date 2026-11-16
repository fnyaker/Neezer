<script>
  // The phone shell's four destinations (DESIGN §9) — the same four as the
  // sidebar, in Material 3 / Apple's tab-bar grammar: one word each, a glyph
  // over a label, the current one carried by a solid accent pill (colour is
  // never the only clue: `aria-current="page"` and the label's ink change too).
  //
  // 64 px EXACTLY, above the home indicator: --safe-bottom is added as padding
  // under the bar (content-box), so a notched phone gets a taller surface while
  // the touch row keeps its 64.
  //
  // Motion (docs/agents/transitions.md S9): the pill's colour arrives on
  // --dur-instant, and the glyph of a tab that just became current SWAPS in
  // (opacity + a 4 px rise, 140 ms) — a keyed block, because an intro only
  // plays for an element that is created after the first render. Nothing here
  // changes the bar's height: the move is a transform.
  import { link } from "svelte-spa-router";
  import { onMount } from "svelte";
  import { location } from "../lib/router.js";
  import { isMobile, onMobileChange } from "../lib/viewport.js";
  import { swap } from "../lib/motion.js";
  import Icon from "./Icon.svelte";

  $: active = (prefix) =>
    $location === prefix || (prefix !== "/" && $location.startsWith(prefix)) ? "active" : "";

  // The bar is `display: none` above 640 px, and remounting a glyph inside a
  // hidden subtree on every navigation would sample keyframes for pixels nobody
  // sees: one stable key there, the real one on a phone.
  let mobile = isMobile();
  onMount(() => onMobileChange((v) => (mobile = v)));
  $: glyphKey = (on) => (mobile ? on : "hors-ecran");
</script>

<nav class="mobilenav" aria-label="Navigation principale">
  <a use:link href="/" class={active("/")} aria-current={$location === "/" ? "page" : undefined}>
    <span class="ico">
      {#key glyphKey(active("/"))}<span class="glyph" in:swap><Icon name="home" size={22} /></span>{/key}
    </span><span class="l">Accueil</span>
  </a>
  <a use:link href="/search" class={active("/search")} aria-current={$location.startsWith("/search") ? "page" : undefined}>
    <span class="ico">
      {#key glyphKey(active("/search"))}<span class="glyph" in:swap><Icon name="compass" size={22} /></span>{/key}
    </span><span class="l">Explorer</span>
  </a>
  <a
    use:link
    href="/library"
    class={active("/library") || active("/podcast")}
    aria-current={$location.startsWith("/library") || $location.startsWith("/podcast") ? "page" : undefined}
  >
    <span class="ico">
      {#key glyphKey(active("/library") || active("/podcast"))}<span class="glyph" in:swap><Icon name="library" size={22} /></span>{/key}
    </span><span class="l">Biblio</span>
  </a>
  <a use:link href="/settings" class={active("/settings")} aria-current={$location.startsWith("/settings") ? "page" : undefined}>
    <span class="ico">
      {#key glyphKey(active("/settings"))}<span class="glyph" in:swap><Icon name="settings" size={22} /></span>{/key}
    </span><span class="l">Réglages</span>
  </a>
</nav>

<style>
  .mobilenav {
    display: none;
  }
  @media (max-width: 640px) {
    .mobilenav {
      position: fixed;
      left: 0;
      right: 0;
      bottom: 0;
      z-index: 40;
      /* content-box: the content row is 64 and the home indicator's inset is
         added UNDER it, so the bar never squashes its targets on a notched
         phone. */
      box-sizing: content-box;
      height: var(--tabbar-h, 64px);
      padding-bottom: var(--safe-bottom, env(safe-area-inset-bottom, 0px));
      display: flex;
      align-items: stretch;
      background: var(--s1);
      border-top: 1px solid var(--ln);
    }
    .mobilenav a {
      position: relative;
      flex: 1 1 0;
      min-width: 0;
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
      gap: 3px;
      padding: 4px 2px 6px;
      color: var(--dim);
      /* metadata type: the mono face is what labels a control in Sillon */
      font-family: var(--font-mono);
      font-size: 11px;
      font-weight: 600;
      line-height: 1;
      letter-spacing: 0.01em;
      text-decoration: none;
    }
    .ico {
      display: grid;
      place-items: center;
      width: 46px;
      height: 26px;
      border-radius: 999px;
      /* S9/M6: the pill's colour, on the instant token (it was a literal 0.16s
         ease). Its height is a contract — nothing here changes size. */
      transition:
        background-color var(--dur-instant) var(--ease-standard),
        color var(--dur-instant) var(--ease-standard);
    }
    .glyph {
      display: grid;
      place-items: center;
    }
    .l {
      max-width: 100%;
      overflow: hidden;
      white-space: nowrap;
      text-overflow: ellipsis;
    }
    .mobilenav a.active {
      color: var(--tx);
    }
    .mobilenav a.active .ico {
      background: var(--ac);
      color: var(--onac);
    }
    /* A press is answered on the pill, not by shrinking the whole tab (which
       made the label jump under the finger). */
    .mobilenav a:active {
      transform: none;
    }
    .mobilenav a:active .ico {
      background: color-mix(in srgb, var(--ac) 70%, var(--s2));
      color: var(--onac);
    }
    .mobilenav a:focus-visible {
      outline: 2px solid var(--ac-text);
      outline-offset: -2px;
      border-radius: 12px;
    }
    @media (prefers-reduced-motion: reduce) {
      .ico {
        transition: none;
      }
    }
  }
</style>
