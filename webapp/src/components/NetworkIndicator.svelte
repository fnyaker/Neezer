<script>
  // Transient connectivity notices. Going offline shows a compact pill for a
  // few seconds, then it fades away — the app keeps working offline (downloads,
  // cached pages), so a permanent alarming banner is wrong; state is conveyed
  // contextually (toasts, offline queue filtering). Recovery flashes briefly.
  //
  // Sillon's pill: --s2 on --tx, one glyph and the sentence. The glyph carries
  // the meaning as much as the words do — nothing here is said by colour alone.
  import { fly } from "svelte/transition";
  import { online, reconnectedAt } from "../lib/net.js";
  import { REMOTE } from "../lib/remote/mode.js";
  import Icon from "./Icon.svelte";

  // Read once: §10 asks the pill not to animate itself for someone who asked
  // for less motion, and a change of system setting lands with the next launch
  // anyway. (Svelte 5's prefersReducedMotion is a MediaQuery, not a store — the
  // `$…` auto-subscription threw "e.subscribe is not a function".)
  const reduced =
    typeof window !== "undefined" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const slide = { y: -16, duration: reduced ? 0 : 200 };

  let showOffline = false;
  let offTimer = null;
  let justReconnected = false;
  let reTimer = null;

  // Fires only on transitions of the store value, not continuously.
  $: if (!$online) {
    showOffline = true;
    clearTimeout(offTimer);
    offTimer = setTimeout(() => (showOffline = false), 4000);
  } else {
    clearTimeout(offTimer);
    showOffline = false;
  }

  $: if ($reconnectedAt) {
    justReconnected = true;
    clearTimeout(reTimer);
    reTimer = setTimeout(() => (justReconnected = false), 2200);
  }
</script>

{#if !$online && showOffline}
  <div class="net chip" role="status" aria-live="polite" transition:fly={slide}>
    <Icon name="cloudOff" size={15} />
    <!-- A remote control plays nothing here: what it can promise is to pick
         the player back up by itself. -->
    {REMOTE ? "Hors ligne — reprise automatique" : "Hors ligne — lecture locale disponible"}
  </div>
{:else if justReconnected}
  <div class="net chip" role="status" aria-live="polite" transition:fly={slide}>
    <Icon name="check" size={15} />
    Reconnecté
  </div>
{/if}

<style>
  /* Centred by the box, not by a transform: the fly transition animates
     `transform` itself and would drop a translateX(-50%) for its duration —
     the pill used to jump to the middle-left while it appeared. */
  .net {
    position: fixed;
    top: calc(var(--topbar-h, 68px) + 12px);
    left: 0;
    right: 0;
    z-index: 250;
    width: max-content;
    margin-inline: auto;
    height: auto;
    min-height: 34px;
    padding: 7px 16px;
    gap: 8px;
    background: var(--s2);
    color: var(--tx);
    border: 1px solid var(--ln);
    font-size: 13px;
    font-weight: 600;
    max-width: calc(100vw - 32px);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    pointer-events: none;
  }
</style>
