<script>
  import { onMount, tick } from "svelte";
  import { trackLyrics, activeLyricIndex } from "../lib/lyrics.js";
  import { current } from "../lib/stores.js";
  import { playable } from "../lib/ladder.js";
  import { trackKey } from "../lib/trackkey.js";
  import { followScroll } from "../lib/scroll.js";
  import { lyricLines } from "../lib/lyriccard.js";
  import LyricCard from "./LyricCard.svelte";
  import Icon from "./Icon.svelte";

  $: lyrics = $trackLyrics;
  $: activeIdx = $activeLyricIndex;
  // The lyrics are asked for once the audio is under way (lib/lyrics.js): until
  // then "none" means still loading, not unavailable — the two states used to
  // read the same and the empty panel looked broken on every track change.
  $: ready = !!$current && $playable === trackKey($current);

  // -- the lyric card ---------------------------------------------------------
  // A card is made from what is on screen NOW: the sheet freezes the lyrics and
  // the active line, so playback moving on cannot rewrite the words under the
  // person's finger. A track change closes it: a card for the previous song
  // under the new song's header is worse than no card.
  let cardOpen = false;
  let cardTrack = null;
  let cardLyrics = null;
  let cardActive = -1;

  // The card's own rule decides whether there is anything to select (a line of
  // punctuation or a "♪" marker is not a lyric), so the button cannot open an
  // empty card on a track whose only "words" are markers.
  $: selectable = lyricLines(lyrics);
  $: canCreate = selectable.length > 0;
  $: reason = canCreate ? "" : ready ? "Paroles indisponibles pour ce titre." : "Chargement des paroles…";
  $: if (cardOpen && cardTrack && trackKey($current) !== trackKey(cardTrack)) cardOpen = false;

  function openCard() {
    if (!canCreate) return;
    cardTrack = $current;
    cardLyrics = lyrics;
    cardActive = $activeLyricIndex;
    cardOpen = true;
  }

  let box;
  let firstFollow = true;
  // Keep the active synced line in view as playback advances (centred-ish so
  // the upcoming lines are visible below). Jump without animation the first
  // time (on open), then smoothly afterwards.
  $: follow(activeIdx);
  async function follow(i) {
    if (i < 0 || !box) return;
    await tick();
    const el = box.querySelector("li.active");
    if (!el) return;
    followScroll(box, el, { ratio: 0.42, smooth: !firstFollow });
    firstFollow = false;
  }
  // Reset the "instant first jump" when the track (lyrics) changes.
  $: lyrics, (firstFollow = true);
  // A view mounted MID-TRACK (the desktop's left column appears when the window
  // grows, the phone's sheet opens late, the queue card switches back to
  // Paroles) already has an active line: scroll to it now instead of waiting
  // for the next line change. Without this the highlight was visible somewhere
  // far below the fold and the panel looked stuck at the top.
  onMount(() => tick().then(() => follow($activeLyricIndex)));
</script>

<div class="lyrics-wrap">
  <div class="lyrics" class:empty-state={!lyrics} bind:this={box}>
    {#if !lyrics}
      <p class="muted empty" role="status">
        {ready ? "Paroles indisponibles pour ce titre." : "Chargement des paroles…"}
      </p>
    {:else if lyrics.synced && lyrics.synced.length}
      <ul class="synced">
        {#each lyrics.synced as line, i}
          <li class:active={i === activeIdx}>{line.text || "• • •"}</li>
        {/each}
      </ul>
    {:else}
      <pre class="plain">{lyrics.text}</pre>
    {/if}
  </div>

  <!-- A named action, not a glyph hidden in the ⋯ menu: making a card is the
       reason the lyrics panel is open on a phone. The reason is visible when
       there is nothing to make a card from, so the disabled state is never a
       mystery. -->
  <div class="lyric-tools">
    <button class="card-btn" on:click={openCard} disabled={!canCreate} title={reason || "Choisir une à trois lignes"}>
      <Icon name="sparkles" size={16} /> Créer une carte
    </button>
    {#if reason}<span class="reason muted">{reason}</span>{/if}
  </div>
</div>

{#if cardOpen && cardTrack}
  <LyricCard track={cardTrack} lyrics={cardLyrics} activeIndex={cardActive} onclose={() => (cardOpen = false)} />
{/if}

<style>
  /* The wrapper is the panel: the lyrics scroll, the action stays put. */
  .lyrics-wrap {
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .lyrics {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 8px 4px;
    color: var(--tx);
  }
  /* No words: the message is a quiet, centred state, not a line stuck to the
     top of a tall column. */
  .lyrics.empty-state {
    display: grid;
    place-items: center;
    overflow: hidden;
  }
  .lyrics.empty-state > p {
    max-width: 26ch;
    margin: 0;
  }
  .synced {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .synced li {
    color: var(--dim);
    font-family: var(--font-display);
    font-size: 1.1rem;
    font-weight: 600;
    line-height: 1.35;
    transition: color 0.2s;
  }
  .synced li.active {
    color: var(--ac-text);
    font-size: 1.3rem;
  }
  .lyrics > p {
    margin: 28px 8px;
    text-align: center;
  }
  .plain {
    white-space: pre-wrap;
    font-family: inherit;
    color: var(--dim);
    line-height: 1.6;
  }
  .lyric-tools {
    flex: none;
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 5px;
    padding: 8px 4px 2px;
  }
  .card-btn {
    min-height: 44px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 0 14px;
    border: 1px solid var(--ln);
    border-radius: 999px;
    color: var(--tx);
    background: transparent;
    font-size: 0.84rem;
    font-weight: 600;
  }
  .card-btn:hover:not(:disabled) {
    background: var(--s2);
    border-color: var(--dim);
  }
  .card-btn:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .reason {
    font-size: 0.72rem;
    line-height: 1.3;
    text-align: center;
  }
</style>
