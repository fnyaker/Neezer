<script module>
  // I10: one card per best result, and a card that DISPLACES another entity
  // fades in (M7). The memory of what the column showed last lives at MODULE
  // scope because a new answer replaces the whole results area — so the card is
  // a new instance every time and could never see the change itself. Svelte
  // cannot help here with `{#key}` either: a transition registered while a key
  // block's branch is still initialising is skipped (`transitions.js`:
  // `run = !block || (block.f & REACTION_RAN) !== 0`), which is why this is the
  // module's own `animate()` — the same wrapper the page enter uses, so eco and
  // reduced motion apply without a second implementation.
  import { animate, DUR, EASE_CSS } from "../lib/motion.js";

  // An object, not a bare `let`: reassigning a module-level declaration from a
  // component's reactive statement is what Svelte warns about (and the update
  // it asks for is not one we want — this memory is not a dependency).
  const memory = { shown: "" };
</script>

<script>
  // The search's best result: the one artist (or show) the query plainly
  // names, large, with what it is said in a small label and its own play
  // button — the "Top result" card Spotify and YouTube Music both open their
  // results on. Which result earns the place is lib/searchrank.js's call.
  import { push } from "svelte-spa-router";
  import { playEntity, buildEntityMenu, buildPodcastMenu, subscribeToPodcast } from "../lib/actions.js";
  import { openMenuAt, keepNativeMenu, menuKey } from "../lib/trackmenu.js";
  import { entityRef } from "../lib/entity.js";
  import { isLocalId } from "../lib/format.js";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";
  import { fadeIn } from "../lib/motion.js";

  export let kind = "artist"; // artist | podcast
  export let item;

  let busy = false;

  $: title = item.name || item.title || "";
  $: image = item.picture || item.cover || null;
  $: ref = entityRef(item);

  let card = null;
  $: if (card) {
    const id = `${kind}:${ref}`;
    // The same entity settled again (the same term re-searched) does not move;
    // the first card of a session does not either — the results area it arrives
    // in already fades (M11: no double entrance).
    if (memory.shown && memory.shown !== id) {
      animate(card, [{ opacity: 0 }, { opacity: 1 }], { duration: DUR.fast, easing: EASE_CSS.enter });
    }
    memory.shown = id;
  }

  async function open() {
    if (kind === "artist") return push("/artist/" + ref);
    if (busy) return;
    busy = true;
    try {
      const c = await subscribeToPodcast(item.deezer_id);
      if (c?.id) push("/podcast/" + c.id);
    } finally {
      busy = false;
    }
  }
  function play(e) {
    e.stopPropagation();
    if (kind === "artist") playEntity("artist", ref);
    else open();
  }
  function menu(e) {
    if (keepNativeMenu(e)) return;
    const items =
      kind === "artist" ? buildEntityMenu("artist", item, push) : buildPodcastMenu(item, push);
    openMenuAt(e, items, e.currentTarget);
  }
</script>

<div
  class="top"
  role="button"
  tabindex="0"
  bind:this={card}
  title={title}
  aria-haspopup="menu"
  on:click={open}
  on:contextmenu={menu}
  on:keydown={(e) => {
    if (menuKey(e, menu)) return;
    if (e.key === "Enter") open();
  }}
  in:fadeIn
>
  <div class="art" class:round={kind === "artist"}>
    <Cover src={image} alt={title} {kind} round={kind === "artist"} fallbackId={isLocalId(item.id) ? item.id : null} />
  </div>
  <div class="text">
    <div class="name">{title}</div>
    <span class="label">{kind === "artist" ? "Artiste" : "Podcast"}</span>
  </div>
  <button
    class="play"
    on:click={play}
    disabled={busy}
    aria-label={kind === "artist" ? "Lire" : "S'abonner"}
  >
    {#if busy}
      <span class="status" data-motion="status"><Icon name="refresh" size={22} /></span>
    {:else}
      <Icon name={kind === "artist" ? "play" : "plus"} size={22} />
    {/if}
  </button>
</div>

<style>
  /* Sillon's card grammar: a raised surface with a hairline, a 24 px corner,
     the name in the display face, one accent control. */
  .top {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 12px;
    padding: 20px;
    border-radius: 24px;
    border: 1px solid var(--ln);
    background: var(--s1);
    color: var(--tx);
    cursor: pointer;
    transition:
      background-color var(--dur-instant) var(--ease-standard),
      border-color var(--dur-instant) var(--ease-standard);
    min-width: 0;
  }
  .top:hover {
    background: var(--s2);
    border-color: var(--s2);
  }
  .art {
    width: 128px;
    border-radius: var(--r-cover);
    overflow: hidden;
  }
  .art.round {
    border-radius: 50%;
  }
  .text {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
    min-width: 0;
    max-width: 100%;
    padding-right: 64px;
  }
  .name {
    max-width: 100%;
    font-family: var(--font-display);
    font-size: 30px;
    font-weight: 700;
    letter-spacing: -0.03em;
    line-height: 1.1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .label {
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--dim);
  }
  .play {
    position: absolute;
    right: 18px;
    bottom: 18px;
    width: 52px;
    height: 52px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: var(--btn);
    color: var(--onbtn);
    transition: transform var(--dur-instant) var(--ease-standard);
  }
  .play:hover {
    transform: scale(1.05);
  }
  .status {
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
    .play,
    .status {
      animation: none;
      transition: none;
    }
  }
  /* A phone: the card is the row it sits in, not a poster. */
  @media (max-width: 640px) {
    .top {
      flex-direction: row;
      align-items: center;
      gap: 14px;
      padding: 12px;
      border-radius: var(--r-card);
    }
    .art {
      width: 64px;
      flex: none;
    }
    .text {
      flex: 1 1 0;
      gap: 4px;
      padding-right: 0;
    }
    .name {
      font-size: 20px;
    }
    .play {
      position: static;
      flex: none;
      width: 46px;
      height: 46px;
    }
  }
</style>
