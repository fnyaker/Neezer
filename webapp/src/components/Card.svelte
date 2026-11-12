<script>
  import { push } from "svelte-spa-router";
  import { openMenu } from "../lib/stores.js";
  import { playEntity, buildEntityMenu } from "../lib/actions.js";
  import Cover from "./Cover.svelte";
  import { entityRef } from "../lib/entity.js";
  import { isLocalId } from "../lib/format.js";
  import Icon from "./Icon.svelte";

  // item: {id, deezer_id, title|name, cover|picture}, kind: album|artist|playlist|mix
  export let item;
  export let kind = "album";

  $: title = item.title || item.name || "";
  $: image = item.cover || item.picture || null;
  $: subtitle =
    item.subtitle ||
    (kind === "album"
      ? item.artist?.name || item.year || ""
      : kind === "artist"
        ? "Artiste"
        : kind === "mix"
          ? "Mix"
          : item.owner || "Playlist");

  // A card opens its entity by the row's `id`; a catalogue result the library
  // does not hold yet has only the id its provider lists it under.
  $: routeId = entityRef(item);

  function open() {
    push((kind === "mix" ? "/mix/" : "/" + kind + "/") + routeId);
  }
  function play(e) {
    e.stopPropagation();
    playEntity(kind, routeId);
  }
  function menu(e) {
    openMenu(e, buildEntityMenu(kind, item, push));
  }
</script>

<div
  class="card"
  on:click={open}
  on:contextmenu={menu}
  on:keydown={(e) => e.key === "Enter" && open()}
  role="button"
  tabindex="0"
  title={title}
>
  <div class="cv">
    <Cover src={image} alt={title} {kind} round={kind === "artist"} fallbackId={isLocalId(item.id) ? item.id : null} />
    <button class="play" on:click={play} aria-label="Lire"><Icon name="play" size={18} /></button>
  </div>
  <div class="meta">
    <div class="title">{title}</div>
    {#if subtitle}<div class="sub">{subtitle}</div>{/if}
  </div>
</div>

<style>
  /* Sillon's card: a raised surface with a hairline, a 20 px corner, and the
     title set in the display face. `content-visibility` stays — it is what
     keeps a 300-card grid cheap (webapp.md, test/clicks/run.mjs). */
  .card {
    display: block;
    text-align: left;
    padding: 12px;
    border-radius: var(--r-card);
    border: 1px solid var(--ln);
    background: var(--s1);
    color: var(--tx);
    transition:
      background-color 0.14s ease,
      border-color 0.14s ease;
    width: 100%;
    cursor: pointer;
    /* Skip rendering off-screen cards, but with `auto` on the placeholder size:
       the browser then REMEMBERS each card's last rendered height instead of
       assuming a flat 220px. The fixed guess was wrong for every grid whose
       columns are wider than a shelf card, so cards entering and leaving the
       viewport kept resizing the page under the scrollbar. */
    content-visibility: auto;
    contain-intrinsic-size: 232px; /* fallback: engines without the `auto` keyword */
    contain-intrinsic-size: auto 232px;
  }
  .card:hover,
  .card:focus-visible {
    background: var(--s2);
    border-color: var(--s2);
  }
  .cv {
    position: relative;
  }
  .play {
    position: absolute;
    right: 8px;
    bottom: 8px;
    width: 40px;
    height: 40px;
    border-radius: 50%;
    background: var(--btn);
    color: var(--onbtn);
    display: grid;
    place-items: center;
    box-shadow: 0 8px 20px rgba(0, 0, 0, 0.32);
    opacity: 0;
    transform: translateY(8px);
    transition:
      opacity 0.14s ease,
      transform 0.14s ease;
  }
  .card:hover .play,
  .card:focus-within .play {
    opacity: 1;
    transform: none;
  }
  .play:hover {
    transform: scale(1.06);
  }
  /* A finger has no hover: the only way to discover the play button is to see
     it, so on a touch screen it is simply there. */
  @media (hover: none) {
    .play {
      opacity: 1;
      transform: none;
    }
  }
  .meta {
    margin-top: 12px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .title {
    font-family: var(--font-display);
    font-weight: 600;
    font-size: 14.5px;
    letter-spacing: -0.01em;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sub {
    font-size: 13px;
    color: var(--dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
