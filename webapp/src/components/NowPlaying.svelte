<script>
  import { trackEntityRef } from "../lib/entity.js";
  import { trackKey, trackRef } from "../lib/trackkey.js";
  import { push } from "svelte-spa-router";
  import { slide } from "svelte/transition";
  import {
    player,
    current,
    favorites,
    nowPlayingOpen,
  } from "../lib/stores.js";
  import { toggleFavorite } from "../lib/actions.js";
  import Cover from "./Cover.svelte";
  import Lyrics from "./Lyrics.svelte";
  import Icon from "./Icon.svelte";
  import ArtistLine from "./ArtistLine.svelte";
  import QueueList from "./QueueList.svelte";

  let tab = "queue"; // queue | lyrics

  $: fav = $current && $favorites.has(trackKey($current));
  $: queue = $player.queue;
  $: idx = $player.index;

  function go(path) {
    push(path);
  }
</script>

<aside class="np" transition:slide={{ axis: "x", duration: 200 }}>
  <header class="np-head">
    <span class="label-mono">En lecture</span>
    <button class="close" on:click={() => nowPlayingOpen.set(false)} aria-label="Fermer"><Icon name="close" size={18} /></button>
  </header>

  {#if $current}
    <div class="art">
      <Cover src={$current.album?.cover} alt={$current.title} kind={$current.podcast ? "podcast" : "album"} fallbackId={trackRef($current)} eager />
    </div>
    <div class="meta">
      <div class="info">
        <button class="t" on:click={() => trackEntityRef($current, "album", $current.album) && go("/album/" + trackEntityRef($current, "album", $current.album))}>
          {$current.title}
        </button>
        <span class="a"><ArtistLine track={$current} navigate={go} /></span>
      </div>
      <button class="fav" class:on={fav} on:click={() => toggleFavorite($current)} aria-label="Favori">
        <Icon name={fav ? "heartFilled" : "heart"} size={20} />
      </button>
    </div>

    <div class="tabs">
      <button class:active={tab === "queue"} on:click={() => (tab = "queue")}>File d'attente</button>
      <button class:active={tab === "lyrics"} on:click={() => (tab = "lyrics")}>Paroles</button>
    </div>

    <div class="body">
      {#if tab === "queue"}
        <div class="queue">
          <QueueList items={queue} {idx} showDuration />
        </div>
      {:else}
        <Lyrics />
      {/if}
    </div>
  {:else}
    <p class="muted empty">Aucune lecture en cours.</p>
  {/if}
</aside>

<style>
  .np {
    background: var(--s1);
    border-left: 1px solid var(--ln);
    color: var(--tx);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    height: 100%;
  }
  /* Narrow desktop: float over the content instead of taking a column. */
  @media (max-width: 1024px) {
    .np {
      position: fixed;
      inset: 0;
      bottom: var(--player-h);
      z-index: 110;
      border-left: none;
    }
  }
  /* Phone: clear the mini player instead of the full desktop player bar. */
  @media (max-width: 640px) {
    .np {
      bottom: 60px;
    }
  }
  .np-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 18px 20px 10px;
  }
  .close {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border-radius: 50%;
    color: var(--dim);
  }
  .close:hover {
    color: var(--tx);
    background: var(--s2);
  }
  .art {
    width: 72px;
    padding: 12px 20px 0;
  }
  .meta {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 20px 14px;
  }
  .info {
    min-width: 0;
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .t {
    font-family: var(--font-display);
    font-weight: 700;
    font-size: 1.3rem;
    letter-spacing: -0.02em;
    line-height: 1.15;
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .t:hover {
    text-decoration: underline;
  }
  /* `display: block` is load-bearing: this used to be a <button> and now wraps
     one button per credited artist (each with its own hover), and ellipsis
     doesn't apply to an inline. */
  .a {
    display: block;
    text-align: left;
    color: var(--dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .fav {
    flex: none;
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    border-radius: 50%;
    color: var(--dim);
  }
  .fav:hover {
    color: var(--tx);
  }
  .fav.on {
    color: var(--hrt);
  }
  .tabs {
    display: flex;
    gap: 6px;
    padding: 0 16px 6px;
  }
  .tabs button {
    height: 32px;
    padding: 0 14px;
    border-radius: 999px;
    color: var(--dim);
    font-weight: 600;
    font-size: 0.84rem;
  }
  .tabs button:hover {
    color: var(--tx);
  }
  .tabs button.active {
    background: var(--ac);
    color: var(--onac);
  }
  .body {
    flex: 1;
    overflow-y: auto;
    padding: 8px 14px 18px;
    min-height: 0;
  }
  .queue {
    margin: 0;
    padding: 0;
    color: var(--tx);
  }
  .empty {
    padding: 24px 20px;
  }
</style>
