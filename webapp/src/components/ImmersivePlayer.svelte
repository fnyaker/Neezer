<script>
  // Full-screen now-playing. Mobile and desktop are deliberately two separate
  // components — the experiences are different enough that one responsive layout
  // just fought itself.
  import { onMount } from "svelte";
  import { immersiveOpen, current, contextMenu } from "../lib/stores.js";
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
  {#if mobile}
    <MobileNowPlaying />
  {:else}
    <DesktopNowPlaying />
  {/if}
{/if}
