<script>
  import { duration as fmtDuration } from "../lib/format.js";
  import VirtualList from "./VirtualList.svelte";

  export let chapters = [];
  export let currentIndex = -1;
  export let onSeek = null;
  export let variant = "panel";
  export let loading = false;

  const VIRTUAL = 60;
  let coarse = false;
  if (typeof window !== "undefined" && window.matchMedia) {
    coarse = window.matchMedia("(pointer: coarse)").matches;
  }
  $: rowHeight = coarse ? 60 : 56;
  $: virtual = chapters.length > VIRTUAL;

  function pick(chapter) {
    onSeek?.(chapter.start);
  }
</script>

<div class="chapters {variant}" role="list" aria-label="Chapitres">
  {#if loading}
    {#each [0, 1, 2] as _}
      <div class="chapter-row sk-row" aria-hidden="true">
        <span class="sk sk-time"></span>
        <span class="sk sk-title"></span>
      </div>
    {/each}
  {:else if virtual}
    <VirtualList
      items={chapters}
      estimateHeight={rowHeight}
      buffer={8}
      key={(item, index) => item.start + ":" + index}
      let:item
      let:index
    >
      <div class="chapter-row" role="listitem">
        <button
          type="button"
          class="chapter-btn"
          class:is-current={index === currentIndex}
          aria-current={index === currentIndex ? "step" : undefined}
          title={item.title || "Chapitre " + (index + 1)}
          on:click={() => pick(item)}
        >
          <span class="chapter-time">{fmtDuration(item.start)}</span>
          <span class="chapter-title">{item.title || "Chapitre " + (index + 1)}</span>
          {#if index === currentIndex}<span class="chapter-now">En cours</span>{/if}
        </button>
      </div>
    </VirtualList>
  {:else}
    {#each chapters as chapter, index (chapter.start + ":" + index)}
      <div class="chapter-row" role="listitem">
        <button
          type="button"
          class="chapter-btn"
          class:is-current={index === currentIndex}
          aria-current={index === currentIndex ? "step" : undefined}
          title={chapter.title || "Chapitre " + (index + 1)}
          on:click={() => pick(chapter)}
        >
          <span class="chapter-time">{fmtDuration(chapter.start)}</span>
          <span class="chapter-title">{chapter.title || "Chapitre " + (index + 1)}</span>
          {#if index === currentIndex}<span class="chapter-now">En cours</span>{/if}
        </button>
      </div>
    {/each}
  {/if}
</div>

<style>
  .chapters {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    min-width: 0;
  }
  .chapter-row {
    height: 56px;
    min-width: 0;
  }
  @media (pointer: coarse) {
    .chapter-row {
      height: 60px;
    }
  }
  .chapter-btn {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    height: 100%;
    min-width: 0;
    padding: 0 8px 0 10px;
    border-radius: 10px;
    color: var(--tx);
    text-align: left;
    transition: background 0.14s ease;
  }
  .chapter-btn:hover {
    background: color-mix(in srgb, var(--tx) 6%, transparent);
  }
  .chapter-btn:focus-visible {
    outline: 2px solid var(--ac-text);
    outline-offset: -2px;
  }
  .chapter-time {
    flex: none;
    width: 52px;
    font-family: var(--font-mono);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    color: var(--dim);
    white-space: nowrap;
  }
  .chapter-title {
    flex: 1;
    min-width: 0;
    font-size: 0.9rem;
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .chapter-now {
    flex: none;
    font-family: var(--font-mono);
    font-size: 9.5px;
    font-weight: 500;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--ac-text);
  }
  .chapter-btn.is-current {
    background: color-mix(in srgb, var(--ac) 12%, transparent);
    box-shadow: inset 3px 0 0 var(--ac-mark);
  }
  .chapter-btn.is-current .chapter-title {
    font-weight: 700;
  }
  .inline .chapter-btn {
    padding-left: 0;
  }
  .inline .chapter-time {
    width: 46px;
  }
  @media (max-width: 640px) {
    .inline .chapter-now {
      display: none;
    }
  }
  .sk-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 8px 0 10px;
  }
  .sk {
    background: linear-gradient(
      90deg,
      var(--s2) 25%,
      color-mix(in srgb, var(--s2) 82%, var(--tx)) 37%,
      var(--s2) 63%
    );
    background-size: 400% 100%;
    animation: ch-shimmer 1.4s ease infinite;
    border-radius: 7px;
  }
  .sk-time {
    flex: none;
    width: 44px;
    height: 10px;
  }
  .sk-title {
    width: 70%;
    max-width: 220px;
    height: 12px;
  }
  @keyframes ch-shimmer {
    0% {
      background-position: 100% 0;
    }
    100% {
      background-position: -100% 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .sk {
      animation: none;
    }
    .chapter-btn {
      transition: none;
    }
  }
</style>
