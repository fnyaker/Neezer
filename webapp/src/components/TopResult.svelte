<script>
  // The search's best result: the one artist (or show) the query plainly
  // names, large, with what it is said in a small label and its own play
  // button — the "Top result" card Spotify and YouTube Music both open their
  // results on. Which result earns the place is lib/searchrank.js's call.
  import { push } from "svelte-spa-router";
  import { openMenu } from "../lib/stores.js";
  import { playEntity, buildEntityMenu, subscribeToPodcast } from "../lib/actions.js";
  import { entityRef } from "../lib/entity.js";
  import { isLocalId } from "../lib/format.js";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";

  export let kind = "artist"; // artist | podcast
  export let item;

  let busy = false;

  $: title = item.name || item.title || "";
  $: image = item.picture || item.cover || null;
  $: ref = entityRef(item);

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
    if (kind === "artist") openMenu(e, buildEntityMenu("artist", item, push));
  }
</script>

<div
  class="top"
  role="button"
  tabindex="0"
  title={title}
  on:click={open}
  on:contextmenu={menu}
  on:keydown={(e) => e.key === "Enter" && open()}
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
    class:spin={busy}
    on:click={play}
    disabled={busy}
    aria-label={kind === "artist" ? "Lire" : "S'abonner"}
  >
    <Icon name={kind === "artist" ? "play" : busy ? "refresh" : "plus"} size={22} />
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
      background-color 0.15s ease,
      border-color 0.15s ease;
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
    transition: transform 0.15s ease;
  }
  .play:hover {
    transform: scale(1.05);
  }
  .play.spin :global(.icon) {
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .play,
    .play.spin :global(.icon) {
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
