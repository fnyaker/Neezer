<script>
  import { createEventDispatcher } from "svelte";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";
  // The heart is a glyph swap (M7): the old one goes at once, the new one
  // fades and settles 4 px from where it lands (docs/agents/transitions.md, I15).
  import { swap } from "../lib/motion.js";

  export let artist = null;
  export let liked = false;
  export let pending = false;
  export let placeholder = false;
  export let similarOf = "";
  export let index = 0;
  export let tabindex = -1;

  const dispatch = createEventDispatcher();
  const NAV = ["ArrowRight", "ArrowLeft", "ArrowDown", "ArrowUp", "Home", "End"];

  $: name = artist ? artist.name || "" : "";
  $: image = artist ? artist.picture || artist.picture_small || null : null;
  $: label = placeholder ? "" : `${liked ? "Retirer" : "Ajouter"} ${name} à mes artistes`;
  $: full = similarOf ? `${label}, artiste proche de ${similarOf}` : label;

  function onKey(event) {
    if (!NAV.includes(event.key)) return;
    event.preventDefault();
    dispatch("nav", { key: event.key });
  }
</script>

{#if placeholder}
  <div class="tile ph" aria-hidden="true">
    <div class="art sk"></div>
    <div class="lines">
      <span class="line sk"></span>
      <span class="line sk short"></span>
    </div>
  </div>
{:else}
  <button
    type="button"
    class="tile"
    class:liked
    class:pending
    {tabindex}
    data-index={index}
    aria-pressed={liked}
    aria-busy={pending}
    aria-label={full}
    on:click={() => dispatch("toggle", { id: artist?.deezer_id || artist?.id, liked })}
    on:focus={() => dispatch("focused", { index })}
    on:keydown={onKey}
  >
    <span class="art">
      <Cover src={image} alt={name} kind="artist" round />
      <span class="heart" class:on={liked} aria-hidden="true">
        {#key liked}
          <span class="glyphswap" in:swap><Icon name={liked ? "heartFilled" : "heart"} size={17} /></span>
        {/key}
      </span>
      {#if similarOf}
        <span class="near" aria-hidden="true">Proche</span>
      {/if}
    </span>
    <span class="name">{name}</span>
  </button>
{/if}

<style>
  .tile {
    display: flex;
    flex-direction: column;
    gap: 9px;
    width: 100%;
    padding: 8px;
    border-radius: 16px;
    text-align: left;
    color: var(--tx);
    content-visibility: auto;
    contain-intrinsic-size: auto 240px;
    /* Hover and press are frequent things: 90 ms, tokens only (M6/M15). */
    transition:
      background-color var(--dur-instant) var(--ease-standard),
      transform var(--dur-instant) var(--ease-standard);
  }
  .tile:not(.ph):hover {
    background: var(--s2);
  }
  .tile:not(.ph):active {
    transform: scale(0.985);
  }
  .art {
    position: relative;
    display: block;
    width: 100%;
    aspect-ratio: 1 / 1;
  }
  .heart {
    position: absolute;
    z-index: 2;
    top: 6px;
    right: 6px;
    width: 30px;
    height: 30px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: var(--s1);
    color: var(--tx);
    box-shadow: 0 0 0 1px var(--ln);
  }
  /* The entered glyph is the only thing the swap moves: the disc keeps its
     30 px and its place, so a like never nudges the grid. */
  .glyphswap {
    display: grid;
    place-items: center;
  }
  .heart.on {
    background: var(--ac);
    color: var(--onac);
    box-shadow: 0 0 0 2px var(--ac-text);
  }
  .near {
    position: absolute;
    z-index: 2;
    top: 6px;
    left: 6px;
    padding: 3px 6px;
    border-radius: 6px;
    border: 1px solid var(--ln);
    background: var(--s1);
    color: var(--dim);
    font-family: var(--font-mono);
    font-size: 10px;
    font-weight: 500;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .name {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    min-height: 2.5em;
    padding: 0 2px;
    font-family: var(--font-display);
    font-size: 14.5px;
    font-weight: 600;
    line-height: 1.25;
    letter-spacing: -0.01em;
  }
  .pending {
    opacity: 0.72;
  }
  .ph {
    cursor: default;
    pointer-events: none;
  }
  .sk {
    border-radius: var(--r-cover);
    background: linear-gradient(
      90deg,
      var(--s2) 25%,
      color-mix(in srgb, var(--s2) 82%, var(--tx)) 37%,
      var(--s2) 63%
    );
    background-size: 400% 100%;
    animation: shimmer 1.4s ease infinite;
  }
  .ph .lines {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-height: 2.5em;
    padding: 3px 2px 0;
  }
  .ph .line {
    display: block;
    height: 11px;
    border-radius: 6px;
  }
  .ph .line.short {
    width: 62%;
  }
  @keyframes shimmer {
    0% {
      background-position: 100% 0;
    }
    100% {
      background-position: -100% 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .tile {
      transition: none;
    }
    .sk {
      animation: none;
    }
    .tile:not(.ph):active {
      transform: none;
    }
  }
</style>
