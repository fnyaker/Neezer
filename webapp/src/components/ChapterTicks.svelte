<script>
  import { onMount } from "svelte";
  import { fadeIn } from "../lib/motion.js";

  export let chapters = [];
  export let duration = 0;
  export let thumb = 15;

  let el = null;
  let width = 0;
  let ro = null;

  $: marks = (chapters || []).filter((c) => c.start > 0 && c.start < duration);
  $: usable = Math.max(0, width - thumb);
  $: visible = usable > 0 && marks.length > 0 && usable / marks.length >= 7;
  $: total = Number(duration) || 0;

  onMount(() => {
    const parent = el?.parentElement;
    if (!parent) return;
    const read = () => (width = parent.clientWidth);
    read();
    if (typeof ResizeObserver !== "undefined") {
      ro = new ResizeObserver(read);
      ro.observe(parent);
    }
    window.addEventListener("resize", read);
    return () => {
      ro?.disconnect();
      window.removeEventListener("resize", read);
    };
  });
</script>

<div class="chapter-ticks" bind:this={el} aria-hidden="true" style={`--thumb:${thumb}px`}>
  {#if visible}
    <!-- P21: the marks appear as ONE block (opacity only — the ticks are
         positioned on the seek bar, so no transform on them, M1). -->
    <span class="tick-layer" in:fadeIn>
      {#each marks as mark (mark.start)}
        <span
          class="chapter-tick"
          style={`--t:${total > 0 ? mark.start / total : 0}`}
        ></span>
      {/each}
    </span>
  {/if}
</div>

<style>
  .chapter-ticks {
    position: absolute;
    left: 0;
    right: 0;
    top: 50%;
    height: 6px;
    transform: translateY(-50%);
    pointer-events: none;
    overflow: visible;
  }
  .tick-layer {
    /* The marks' positioning box is exactly the rail's: the ticks still resolve
       `left`/`top` against the same geometry as before. */
    position: absolute;
    inset: 0;
  }
  .chapter-tick {
    position: absolute;
    top: 50%;
    left: calc(var(--thumb) / 2 + (100% - var(--thumb)) * var(--t));
    width: 1px;
    height: 6px;
    transform: translate(-50%, -50%);
    background: var(--dim);
    opacity: 0.7;
    box-shadow: 0 0 0 1px var(--bg);
    pointer-events: none;
  }
  @media (pointer: coarse) {
    .chapter-tick {
      width: 2px;
    }
  }
</style>
