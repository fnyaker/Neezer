<script>
  import { trackKey, trackRef } from "../lib/trackkey.js";
  import { flip } from "svelte/animate";
  import { dragHandleZone, dragHandle } from "svelte-dnd-action";
  import { push } from "svelte-spa-router";
  import { player, currentId, playing, openMenu, downloads } from "../lib/stores.js";
  import { buildTrackMenu } from "../lib/actions.js";
  import { duration as fmtDuration } from "../lib/format.js";
  import { playbackIdle } from "../lib/playback.js";
  import Icon from "./Icon.svelte";
  import { iconOf, offMain, sourceTitle } from "../lib/sources.js";
  import Cover from "./Cover.svelte";
  import ArtistLine from "./ArtistLine.svelte";

  // Editable, reorderable playlist list. Reordering uses svelte-dnd-action's
  // native drag-handle support (dragHandleZone + dragHandle), which handles
  // mouse, touch and keyboard reliably — dragging is initiated only from the
  // grip, so tapping/clicking a row still plays it and the list still scrolls.
  // `editing` is the dedicated mobile edit mode (big handles, taller rows).
  export let tracks = [];
  export let context = null;
  export let editing = false;
  export let onreorder = null; // (newTracks) => void
  export let onremove = null; // (index) => void

  // Row-shift animation duration. FLIP measures EVERY row's position on each
  // drag update / move — O(N) layout reads per event — so it's disabled past
  // ~120 rows: big playlists reorder instantly instead of animating sluggishly.
  const FLIP = 160;
  $: flipMs = rows.length > 120 ? 0 : FLIP;

  // Wrap each track with a stable id for keyed iteration + dnd. The id sticks
  // to the track OBJECT (WeakMap), so a reorder/remove keeps every surviving
  // row's id — Svelte's keyed {#each} then MOVES the DOM nodes instead of
  // destroying and recreating the whole list (which reloaded every cover and
  // froze the page on big playlists).
  let seq = 0;
  const rowIds = new WeakMap();
  function rowId(t) {
    let id = rowIds.get(t);
    if (id === undefined) {
      id = ++seq;
      rowIds.set(t, id);
    }
    return id;
  }
  let rows = [];
  $: syncRows(tracks);
  function syncRows(list) {
    // The parent echoes our own order back after a drag (data.tracks = ...):
    // same objects in the same order means the DOM is already right — skip.
    if (rows.length === list.length && rows.every((r, i) => r.track === list[i])) return;
    rows = list.map((t) => ({ id: rowId(t), track: t }));
  }

  function handleConsider(e) {
    rows = e.detail.items;
  }
  function handleFinalize(e) {
    rows = e.detail.items;
    onreorder?.(rows.map((r) => r.track));
  }

  // Tap-to-move fallback for edit mode: precise single-step moves that don't
  // require a drag gesture (essential on touch screens).
  function move(i, delta) {
    const j = i + delta;
    if (j < 0 || j >= rows.length) return;
    const next = rows.slice();
    const [r] = next.splice(i, 1);
    next.splice(j, 0, r);
    rows = next;
    onreorder?.(next.map((x) => x.track));
  }

  function playAt(i) {
    if (editing) return;
    const t = rows[i].track;
    if ($currentId === trackKey(t)) player.toggle();
    else player.playQueue(rows.map((r) => r.track), i, context);
  }

  async function menu(e, track) {
    e.preventDefault();
    e.stopPropagation();
    const coords = { clientX: e.clientX, clientY: e.clientY, preventDefault() {}, stopPropagation() {} };
    openMenu(coords, buildTrackMenu(track, push));
  }
</script>

<div
  class="list"
  class:editing
  use:dragHandleZone={{ items: rows, flipDurationMs: flipMs, dropTargetStyle: {} }}
  on:consider={handleConsider}
  on:finalize={handleFinalize}
>
  {#each rows as row, i (row.id)}
    {@const track = row.track}
    {@const isCurrent = $currentId === trackKey(track)}
    <!-- svelte-ignore a11y-no-static-element-interactions -->
    <div class="row" class:active={isCurrent} animate:flip={{ duration: flipMs }} on:contextmenu={(e) => menu(e, track)}>
      <span class="grip" use:dragHandle aria-label="Déplacer le titre" title="Glisser pour réordonner">
        <Icon name="grip" size={editing ? 22 : 18} />
      </span>

      {#if editing}
        <span class="updown">
          <button on:click={() => move(i, -1)} disabled={i === 0} aria-label="Monter le titre" title="Monter">
            <Icon name="chevronUp" size={20} />
          </button>
          <button on:click={() => move(i, 1)} disabled={i === rows.length - 1} aria-label="Descendre le titre" title="Descendre">
            <Icon name="chevronDown" size={20} />
          </button>
        </span>
      {:else}
        <button class="play" on:click={() => playAt(i)} aria-label="Lire">
          {#if isCurrent && $playing && $playbackIdle}
            <span class="eq" aria-hidden="true"><i></i><i></i><i></i></span>
          {:else if isCurrent && $playing}
            <span class="rowspin" aria-hidden="true"></span>
          {:else}
            {#if isCurrent}
              <span class="curplay" aria-hidden="true"><Icon name="play" size={13} /></span>
            {:else}
              <span class="num">{i + 1}</span>
              <span class="ic"><Icon name="play" size={14} /></span>
            {/if}
          {/if}
        </button>
      {/if}

      <div class="titles">
        <div class="thumb"><Cover src={track.album?.cover} alt={track.title} size={44} kind="track" fallbackId={trackRef(track)} /></div>
        <div class="meta">
          <div class="t">
            {#if offMain(track.source)}<span class="local" title={sourceTitle(track.source)}><Icon name={iconOf(track.source)} size={13} /></span>{:else if track.local}<span class="local" title={sourceTitle("local")}><Icon name="cloudOff" size={13} /></span>{/if}
            {#if $downloads.has(trackKey(track))}<span class="dlbadge" title="Disponible hors-ligne"><Icon name="downloaded" size={13} /></span>{/if}
            {track.title}
            {#if track.explicit}<span class="explicit">E</span>{/if}
          </div>
          <span class="a muted"><ArtistLine {track} /></span>
        </div>
      </div>

      <span class="alb muted">{track.album?.title || ""}</span>

      <span class="dur muted">{fmtDuration(track.duration)}</span>

      <button class="remove" on:click|stopPropagation={() => onremove?.(i)} aria-label="Retirer de la playlist" title="Retirer">
        <Icon name="minusCircle" size={editing ? 24 : 20} />
      </button>
      <button class="more" on:click|stopPropagation={(e) => menu(e, track)} aria-label="Plus d'options"><Icon name="more" size={18} /></button>
    </div>
  {/each}
</div>

{#if !rows.length}
  <p class="muted empty">Cette playlist est vide. Ajoutez des titres pour commencer.</p>
{/if}

<style>
  .list {
    display: flex;
    flex-direction: column;
  }
  .row {
    display: grid;
    grid-template-columns: 28px 36px minmax(0, 1fr) minmax(0, 0.7fr) 56px 34px 34px;
    align-items: center;
    gap: 12px;
    padding: 6px 8px;
    border-radius: 10px;
    user-select: none;
    background: var(--bg);
    /* This list can't be windowed — svelte-dnd-action needs every drop target
       in the DOM — so instead we make the off-screen rows nearly free: the
       browser skips their style/layout/paint entirely (and never fetches their
       lazily-loaded cover). The row BOX itself is still laid out at its
       intrinsic height, so drag hit-testing and scroll geometry stay exact.
       Rows are uniform, so the placeholder height below is the real one. */
    content-visibility: auto;
    contain-intrinsic-size: 56px; /* fallback: engines without the `auto` keyword */
    contain-intrinsic-size: auto 56px;
  }
  .row:hover {
    background: var(--s2);
  }
  .row.active .t {
    color: var(--ac);
  }
  .grip {
    color: var(--dim);
    cursor: grab;
    opacity: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
    /* let the handle own the gesture (no browser scroll/zoom while dragging) */
    touch-action: none;
  }
  .grip:active {
    cursor: grabbing;
    color: var(--tx);
  }
  .row:hover .grip,
  .list.editing .grip {
    opacity: 1;
  }
  .play {
    width: 36px;
    height: 36px;
    display: grid;
    place-items: center;
    position: relative;
    border-radius: 50%;
  }
  .play .num {
    font-family: var(--font-mono);
    font-size: 13px;
    color: var(--dim);
  }
  .play .curplay {
    display: flex;
    color: var(--ac);
  }
  .play .ic {
    display: none;
  }
  .row:hover .play .num {
    display: none;
  }
  .row:hover .play .ic {
    display: block;
  }
  /* Up/down tap-to-move buttons (edit mode) — the drag handle's precise,
     gesture-free sibling. */
  .updown {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
  }
  .updown button {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 34px;
    height: 26px;
    color: var(--dim);
    border-radius: 6px;
  }
  .updown button:hover:not(:disabled) {
    color: var(--tx);
    background: var(--s2);
  }
  .updown button:disabled {
    opacity: 0.3;
  }
  .titles {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
  }
  .thumb {
    flex: none;
    width: 44px;
  }
  .meta {
    min-width: 0;
  }
  .t {
    font-size: 15px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .local {
    display: inline-flex;
    vertical-align: -2px;
    color: var(--dim);
    margin-right: 3px;
  }
  .dlbadge {
    display: inline-flex;
    vertical-align: -2px;
    color: var(--ac);
    margin-right: 3px;
  }
  .explicit {
    font-size: 0.6rem;
    background: var(--s2);
    padding: 1px 3px;
    border-radius: 3px;
    vertical-align: 1px;
  }
  /* One hoverable button per credited artist lives inside (ArtistLine), so the
     hover treatment is applied there, per name, not on the whole line. */
  .a,
  .alb {
    display: block;
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 100%;
    font-size: 13px;
    color: var(--dim);
  }
  .dur {
    text-align: right;
    font-family: var(--font-mono);
    font-size: 13px;
    font-variant-numeric: tabular-nums;
    color: var(--dim);
  }
  .remove {
    color: var(--dim);
    opacity: 0;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .remove:hover {
    color: var(--ac);
  }
  .more {
    color: var(--dim);
    opacity: 0;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .row:hover .remove,
  .row:hover .more,
  .list.editing .remove {
    opacity: 1;
  }
  .more:hover {
    color: var(--tx);
  }
  .empty {
    margin-top: 18px;
  }
  .rowspin {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 2px solid var(--s2);
    border-top-color: var(--ac);
    animation: rowspin 0.8s linear infinite;
  }
  @keyframes rowspin {
    to {
      transform: rotate(360deg);
    }
  }
  /* equalizer animation on the playing row */
  .eq {
    display: flex;
    align-items: flex-end;
    gap: 2px;
    height: 16px;
  }
  .eq i {
    width: 3px;
    background: var(--ac);
    animation: eq 0.9s ease-in-out infinite;
  }
  .eq i:nth-child(1) {
    animation-delay: -0.2s;
  }
  .eq i:nth-child(2) {
    animation-delay: -0.5s;
  }
  .eq i:nth-child(3) {
    animation-delay: -0.8s;
  }
  @keyframes eq {
    0%,
    100% {
      height: 30%;
    }
    50% {
      height: 100%;
    }
  }

  /* ----- Touch edit mode: a dedicated, finger-friendly layout ----- */
  @media (max-width: 640px) {
    /* Three visible columns: play · title · more. The grip, album, duration and
       remove button are out of the flow here (grip/remove only return in edit
       mode), so the column count must match or the title gets squeezed. */
    .row {
      grid-template-columns: 36px minmax(0, 1fr) 34px;
      gap: 10px;
    }
    .alb,
    .dur {
      display: none;
    }
    .more {
      opacity: 1;
    }
    /* Remove is an edit-mode-only action on touch; fully out of the flow
       otherwise (opacity:0 alone would still steal a grid column). */
    .remove {
      display: none;
    }
    /* Hide the grip entirely when not editing (no hover on touch). */
    .grip {
      display: none;
    }
    .list.editing .grip {
      display: flex;
      opacity: 1;
      width: 44px;
      /* big, comfortable drag target */
      margin-left: -4px;
    }
    .list.editing .row {
      grid-template-columns: 44px 40px minmax(0, 1fr) 44px;
      gap: 10px;
      padding: 12px 6px;
      align-items: center;
      contain-intrinsic-size: 76px; /* taller edit rows on phones */
      contain-intrinsic-size: auto 76px;
    }
    .list.editing .updown button {
      width: 40px;
      height: 32px;
    }
    .list.editing .more {
      display: none;
    }
    .list.editing .remove {
      display: flex;
      opacity: 1;
      width: 44px;
      height: 44px;
    }
    .list.editing .row:nth-child(odd) {
      background: var(--s2);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .eq i {
      animation: none;
      height: 70%;
    }
  }
  /* The floating clone svelte-dnd-action renders while dragging: lift it. */
  :global(#dnd-action-dragged-el) {
    outline: 2px solid var(--ac);
    border-radius: 10px;
    background: var(--s1);
    cursor: grabbing;
  }
</style>
