<script>
  // A shelf is an APERÇU, not a tracklist: the header says how much there is,
  // six compact rows say what it sounds like, and the whole card is ONE link
  // to the full page (never a play button hidden in a link, never an editable
  // list). The rows are deliberately not TrackList: no album column, no
  // quality, no per-row menu — 40 px of sleeve, the names, the duration.
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";
  import { artistLine, duration as fmtDuration } from "../lib/format.js";

  export let shelf; // { key, title, subtitle, tracks, releases }
  export let limit = 6;

  $: shown = shelf.tracks.slice(0, limit);
  $: trackCount = shelf.tracks.length;
  $: releaseCount = shelf.releases.length;
  $: count = `${trackCount} titre${trackCount > 1 ? "s" : ""} · ${releaseCount} sortie${releaseCount > 1 ? "s" : ""}`;
  $: titleId = `shelf-title-${shelf.key}`;
  $: countId = `shelf-count-${shelf.key}`;
  // Four sleeves make a mosaic; grooves alone would read as "no artwork", so a
  // shelf without a single cover shows one placeholder instead of four.
  $: sleeves = (shelf.releases.length ? shelf.releases : [null]).slice(0, 4);
  $: mosaic = sleeves.some((r) => r && r.cover) ? sleeves : sleeves.slice(0, 1);
  $: cells = mosaic.length;
</script>

<a
  class="rc card"
  href={"#/releases/" + shelf.key}
  aria-labelledby="{titleId} {countId}"
>
  <header class="rc-head">
    <span
      class="rc-mosaic"
      class:one={cells === 1}
      class:two={cells === 2}
      class:three={cells === 3}
      class:four={cells === 4}
      aria-hidden="true"
    >
      {#each mosaic as r, i (r?.deezer_id || i)}
        <span class="rc-cell">
          <Cover src={r?.cover || null} alt={r?.title || ""} kind="album" />
        </span>
      {/each}
    </span>
    <span class="rc-heading">
      <h2 class="rc-title" id={titleId}>{shelf.title}</h2>
      <span class="rc-sub">{shelf.subtitle}</span>
      <span class="rc-count mono" id={countId}>{count}</span>
    </span>
  </header>

  <ol class="rc-list">
    {#each shown as t (t.deezer_id || t.title)}
      <li class="rc-row">
        <span class="rc-art">
          <Cover src={t.release?.cover || null} alt="" kind="album" />
        </span>
        <span class="rc-lines">
          <span class="rc-t">{t.title}</span>
          <span class="rc-a">{artistLine(t)}</span>
        </span>
        <span class="rc-dur mono">{fmtDuration(t.duration)}</span>
      </li>
    {/each}
  </ol>

  <!-- The cue, not a second control: the card itself is the target. -->
  <span class="rc-foot">
    <span>Tout voir</span>
    <Icon name="chevronRight" size={15} />
  </span>
</a>

<style>
  .rc {
    display: flex;
    flex-direction: column;
    min-width: 0;
    padding: 18px 18px 12px;
    color: var(--tx);
    text-decoration: none;
    transition: background-color 0.14s ease, border-color 0.14s ease;
  }
  .rc:hover,
  .rc:focus-visible {
    background: var(--s2);
    border-color: var(--dim);
  }
  .rc-head {
    display: flex;
    align-items: flex-start;
    gap: 16px;
    min-width: 0;
  }
  .rc-mosaic {
    flex: none;
    display: grid;
    gap: 2px;
    width: 84px;
    aspect-ratio: 1 / 1;
    overflow: hidden;
    border-radius: 14px;
    background: var(--s2);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--tx) 8%, transparent);
  }
  .rc-mosaic.one {
    grid-template-columns: 1fr;
  }
  .rc-mosaic.two {
    grid-template-columns: 1fr 1fr;
  }
  .rc-mosaic.three,
  .rc-mosaic.four {
    grid-template-columns: 1fr 1fr;
    grid-template-rows: 1fr 1fr;
  }
  .rc-mosaic.three .rc-cell:last-child {
    grid-column: 1 / -1;
  }
  .rc-cell {
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }
  .rc-cell :global(.cover) {
    border-radius: 0;
  }
  .rc-heading {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    flex: 1;
  }
  .rc-title {
    margin: 0;
    font-family: var(--font-display);
    font-size: 24px;
    font-weight: 700;
    letter-spacing: -0.02em;
    line-height: 1.05;
    overflow-wrap: anywhere;
  }
  .rc-sub {
    color: var(--dim);
    font-size: 13px;
    line-height: 1.4;
    /* Two lines keep the two cards' headers level without truncating a real
       sentence to a fragment. */
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .rc-count {
    color: var(--dim);
    font-size: 11.5px;
    letter-spacing: 0.02em;
  }
  .rc-list {
    display: flex;
    flex-direction: column;
    margin: 14px 0 0;
    padding: 0;
    list-style: none;
    min-width: 0;
  }
  .rc-row {
    display: flex;
    align-items: center;
    gap: 11px;
    min-width: 0;
    min-height: 50px;
    padding: 5px 0;
    border-top: 1px solid var(--ln);
  }
  .rc-row:first-child {
    border-top: none;
  }
  .rc-art {
    flex: none;
    width: 40px;
  }
  .rc-art :global(.cover) {
    border-radius: 8px;
  }
  .rc-lines {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    flex: 1;
  }
  .rc-t,
  .rc-a {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rc-t {
    font-size: 14px;
    font-weight: 600;
    letter-spacing: -0.005em;
  }
  .rc-a {
    color: var(--dim);
    font-size: 12.5px;
  }
  .rc-dur {
    flex: none;
    color: var(--dim);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  .rc-foot {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 2px;
    min-height: 32px;
    margin-top: 4px;
    padding-top: 6px;
    border-top: 1px solid var(--ln);
    color: var(--dim);
    font-size: 12.5px;
    font-weight: 600;
  }
  .rc:hover .rc-foot {
    color: var(--ac-text);
  }

  /* One card alone gets the whole row, so its six titles read in two columns
     instead of six half-empty lines across a metre of desktop. */
  @container shelves (min-width: 900px) {
    .rc:only-child .rc-list {
      display: grid;
      grid-auto-flow: column;
      grid-template-rows: repeat(3, auto);
      column-gap: 28px;
    }
    .rc:only-child .rc-row:nth-child(4) {
      border-top: none;
    }
  }

  @media (max-width: 640px) {
    .rc {
      padding: 14px 14px 8px;
    }
    .rc-head {
      gap: 12px;
    }
    .rc-mosaic {
      width: 64px;
      border-radius: 12px;
    }
    .rc-title {
      font-size: 22px;
    }
    .rc-count {
      font-size: 11px;
    }
    .rc-list {
      margin-top: 10px;
    }
    .rc-row {
      min-height: 48px;
      gap: 10px;
    }
    .rc-art {
      width: 40px;
    }
    .rc-foot {
      min-height: 36px;
    }
  }
</style>
