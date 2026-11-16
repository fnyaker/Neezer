<script>
  // One mood of the home's "Mixes d'ambiance" row. The whole tile is the
  // target (44 px at the very least, far more here); the trailing glyph is a
  // cue, never a second control, so a tap anywhere does the same thing.
  import { createEventDispatcher } from "svelte";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";

  export let mood; // {id, name, subtitle, tracks, ready, cover}
  export let loading = false; // its detail request is in flight

  const dispatch = createEventDispatcher();
  $: count = `${mood.tracks} titre${mood.tracks > 1 ? "s" : ""}`;
  $: label = loading
    ? `Préparation du mix ${mood.name}`
    : `Lancer le mix ${mood.name} : ${mood.subtitle}, ${count}`;

  function play() {
    if (!loading) dispatch("play", mood);
  }
</script>

<button
  type="button"
  class="mood"
  class:loading
  aria-label={label}
  aria-busy={loading}
  title={label}
  on:click={play}
>
  <span class="art">
    <Cover src={mood.cover} alt={mood.name} kind="mix" />
  </span>
  <span class="meta">
    <span class="name">{mood.name}</span>
    <span class="sub">{mood.subtitle}</span>
    <span class="foot">
      <span class="count mono">{count}</span>
      <span class="go">
        {#if loading}
          <!-- I9/M13: a job that runs has to SAY so, so the spinner itself is
               marked `status`: it is the one animation eco keeps, and it spins
               on the marked element (a child's animation is what the guard
               stops). -->
          <span class="spin" data-motion="status"><Icon name="refresh" size={15} /></span>
        {:else}
          <Icon name="play" size={15} />
        {/if}
      </span>
    </span>
  </span>
</button>

<style>
  .mood {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: 100%;
    min-width: 0;
    min-height: 44px;
    padding: 0;
    border: 0;
    border-radius: var(--r-card);
    background: none;
    color: var(--tx);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .art {
    display: block;
    width: 100%;
    border-radius: var(--r-cover);
    transition: transform var(--dur-instant) var(--ease-standard);
  }
  .mood:hover .art,
  .mood:focus-visible .art {
    transform: translateY(-2px);
  }
  .meta {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }
  .name,
  .sub {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .name {
    font-family: var(--font-display);
    font-weight: 600;
    font-size: 15px;
    letter-spacing: -0.005em;
    white-space: nowrap;
  }
  /* Two lines, then an ellipsis: the copy is informative, not decorative. */
  .sub {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    color: var(--dim);
    font-size: 12.5px;
    line-height: 1.3;
  }
  .foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-top: 3px;
  }
  .count {
    color: var(--dim);
    font-size: 11px;
  }
  .go {
    display: grid;
    place-items: center;
    flex: none;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    background: var(--btn);
    color: var(--onbtn);
    transition:
      transform var(--dur-instant) var(--ease-standard),
      background-color var(--dur-instant) var(--ease-standard);
  }
  .mood:hover .go,
  .mood:focus-visible .go {
    transform: scale(1.06);
  }
  .loading {
    cursor: progress;
  }
  .loading .art,
  .loading .name,
  .loading .sub {
    opacity: 0.6;
  }
  .loading .go {
    background: var(--s2);
    color: var(--dim);
  }
  .spin {
    display: grid;
    place-items: center;
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .art,
    .go {
      transition: none;
    }
    .mood:hover .art,
    .mood:focus-visible .art,
    .mood:hover .go,
    .mood:focus-visible .go {
      transform: none;
    }
    .spin {
      animation: none;
    }
  }
</style>
