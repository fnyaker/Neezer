<script>
  // Full-screen now-playing. Mobile and desktop are deliberately two separate
  // components — the experiences are different enough that one responsive layout
  // just fought itself.
  //
  // THE TRANSITION LIVES HERE, on `.player-host`, and not on the components'
  // own root elements: a Svelte local transition runs when the element sits in
  // the block that changed — for a component's root element it is registered
  // inside the component's own branch, which is created in the same flush, so
  // its intro never plays (measured: `.m`/`.d` came up with 0 animations and no
  // inline style). `.player-host` is an ordinary element of THIS template, the
  // `{#if}` block beneath PlayerBar has run since mount, so `screen` really
  // plays (transitions.md P1/P2).
  //
  // `dragY` is the phone sheet's drag offset, bound from MobileNowPlaying: the
  // host follows the finger and the EXIT starts from it (`fromY`), so a swipe
  // down continues into the closing slide instead of jumping to 0 first (P1).
  // The snap-back after a cancelled drag is the host's own CSS transition.
  import { onMount } from "svelte";
  import { immersiveOpen, immersiveShown, current, contextMenu } from "../lib/stores.js";
  import { screen } from "../lib/motion.js";
  import MobileNowPlaying from "./MobileNowPlaying.svelte";
  import DesktopNowPlaying from "./DesktopNowPlaying.svelte";

  let mobile = false;
  const mq = "(max-width: 640px)";
  if (typeof window !== "undefined") mobile = window.matchMedia(mq).matches;

  onMount(() => {
    const m = window.matchMedia(mq);
    const on = () => (mobile = m.matches);
    m.addEventListener("change", on);
    return () => m.removeEventListener("change", on);
  });

  let dragY = 0;
  let dragging = false;
  // A fresh open starts from the top: the previous dismissal left `dragY` at
  // the finger's position, and the outro needs it until the host is gone.
  let wasOpen = false;
  $: if ($immersiveOpen) {
    if (!wasOpen) {
      dragY = 0;
      dragging = false;
      wasOpen = true;
    }
  } else wasOpen = false;

  function onKey(e) {
    // Escape first belongs to whatever is on top: with a contextual menu open
    // it closes THAT (and gives the focus back to its row), never the player
    // underneath it. The menu marks the key handled; the store covers the case
    // where this listener happens to run before the menu's own window handler.
    if (e.key === "Escape" && $immersiveOpen && !e.defaultPrevented && !$contextMenu)
      immersiveOpen.set(false);
  }
</script>

<svelte:window on:keydown={onKey} />

{#if $immersiveOpen && $current}
  <!-- P1/P2: `screen` — the phone slides by its own height (no opacity: the
       panes inside are glass, M8), the desktop fades and scales 0.98. P3: the
       app's ground is handed over by event, on `introend`/`outrostart` (M12). -->
  <div
    class="player-host"
    class:dragging
    style={`transform:translateY(${dragY}px)`}
    transition:screen={{ fromY: dragY }}
    on:introend={() => immersiveShown.set(true)}
    on:outrostart={() => immersiveShown.set(false)}
  >
    {#if mobile}
      <MobileNowPlaying bind:dragY bind:dragging />
    {:else}
      <DesktopNowPlaying />
    {/if}
  </div>
{/if}

<style>
  /* The host is the full-screen box: its transform is what moves, and a fixed
     descendant (.m's sheets, .d's menus) is anchored to it — the same viewport
     box it filled before. */
  .player-host {
    position: fixed;
    inset: 0;
    z-index: 200;
    /* The snap-back after a cancelled drag (P1): an in-place move, M5/M15. */
    transition: transform var(--dur-base) var(--ease-standard);
  }
  .player-host.dragging {
    transition: none; /* follow the finger 1:1 while dragging */
  }
  @media (prefers-reduced-motion: reduce) {
    /* M14: no displacement — the snap-back lands at once. */
    .player-host {
      transition: none;
    }
  }
</style>
