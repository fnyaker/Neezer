<script>
  import { onMount, tick } from "svelte";
  import { trackLyrics, activeLyricIndex } from "../lib/lyrics.js";
  import { current } from "../lib/stores.js";
  import { playable } from "../lib/ladder.js";
  import { trackKey } from "../lib/trackkey.js";
  import { followScroll } from "../lib/scroll.js";
  import { lyricLines } from "../lib/lyriccard.js";
  import { fadeIn, scrollBehavior } from "../lib/motion.js";
  import LyricCard from "./LyricCard.svelte";
  import Icon from "./Icon.svelte";

  // Floating: the desktop player's own lyrics column (≥1200 px), where the
  // words sit directly on the blurred sleeve with no card around them, the way
  // Spotify, Deezer and Apple Music's full-screen players do it. Everything
  // else — the queue card's tab, the phone's lyrics sheet, the side panel —
  // is a card among the others' cards, and keeps its glass surface.
  export let floating = false;

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
  // time (on open), then smoothly afterwards — and `scrollBehavior()` (P18) is
  // the one owner of "smooth": reduced motion and eco jump at once.
  $: follow(activeIdx);
  async function follow(i) {
    if (i < 0 || !box) return;
    await tick();
    const el = box.querySelector("li.active");
    if (!el) return;
    followScroll(box, el, { ratio: 0.42, smooth: !firstFollow && scrollBehavior() === "smooth" });
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

<div class="lyrics-wrap" class:floating in:fadeIn>
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
    /* The line being heard takes the accent: a colour change on a frequent
       event, so it barely moves (M6) and reads from a token (M15). */
    transition: color var(--dur-fast);
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
  /* -- the separated (desktop wide) column --------------------------------
     No card here: the player's blurred sleeve IS the ground, and the words
     float on it. What keeps them legible is a shadow painted in the theme's
     own background colour — a local, theme-aware scrim around the glyphs, so
     a bright sleeve cannot wash the letters out and a dark one cannot swallow
     them. Nothing else changes: same type, same accent for the line being
     heard, same scroll-follow. */
  .floating {
    /* The secondary lines are lifted toward the theme's own text colour: on a
       light theme `--dim` over the brightest part of a blurred sleeve measured
       4.14:1, and a line being read along with is not decoration. 24 % of
       `--tx` puts Papier's worst pair back above AA while the current line —
       bigger, bolder, in `--ac-text` — stays plainly the one being heard. */
    --floating-dim: color-mix(in srgb, var(--dim) 76%, var(--tx));
  }
  .floating .lyrics {
    padding: 10px 6px;
    /* The words dissolve at the top and bottom edges instead of being cut by
       the column, as the full-screen players do (Spotify, Apple Music). */
    -webkit-mask-image: linear-gradient(
      180deg,
      transparent 0,
      #000 28px,
      #000 calc(100% - 30px),
      transparent 100%
    );
    mask-image: linear-gradient(180deg, transparent 0, #000 28px, #000 calc(100% - 30px), transparent 100%);
  }
  .floating .synced {
    gap: 14px;
    /* The fade is 28/30 px; this padding lets the FIRST and the LAST line sit
       outside it instead of dissolving while it is the line being heard (a
       track's opening line used to land in the top fade every time). */
    padding: 34px 0;
  }
  .floating .synced li {
    color: var(--floating-dim);
    font-size: 1.08rem;
    line-height: 1.32;
    text-shadow:
      0 1px 2px color-mix(in srgb, var(--bg) 72%, transparent),
      0 0 18px color-mix(in srgb, var(--bg) 55%, transparent);
  }
  .floating .synced li.active {
    /* The base rule (.synced li.active) is LESS specific than the line rule
       above, so the accent has to be restated here or the current line loses
       its colour — the bench caught exactly that. */
    color: var(--ac-text);
    font-size: 1.34rem;
    font-weight: 800;
    line-height: 1.28;
    text-shadow:
      0 1px 2px color-mix(in srgb, var(--bg) 80%, transparent),
      0 0 22px color-mix(in srgb, var(--bg) 62%, transparent);
  }
  .floating .lyrics.empty-state > p,
  .floating .reason,
  .floating .plain {
    color: var(--floating-dim);
    text-shadow:
      0 1px 2px color-mix(in srgb, var(--bg) 72%, transparent),
      0 0 18px color-mix(in srgb, var(--bg) 55%, transparent);
  }
  .floating .lyric-tools {
    padding: 10px 6px 2px;
  }
  /* The one control in the column wears the player's clear glass (the pill
     row beside it), not the card's outline: an outlined pill with no card
     behind it reads as a leftover from the panel that was removed. */
  .floating .card-btn {
    border-color: transparent;
    background-color: color-mix(in srgb, var(--s1) 28%, transparent);
    background-image: var(--glass-beam-x);
    background-size: 100% 1px;
    background-position: top left;
    background-repeat: no-repeat;
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--tx) 10%, transparent);
  }
  .floating .card-btn:hover:not(:disabled) {
    background-color: var(--glass-hover);
    border-color: transparent;
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
