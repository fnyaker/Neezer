<script>
  import { entityRef, trackEntityRef } from "../lib/entity.js";
  import { trackKey, trackRef } from "../lib/trackkey.js";
  import { push } from "svelte-spa-router";
  import {
    player,
    currentId,
    playing,
    favorites,
    openMenu,
    downloads,
    unavailableIds,
    openReplace,
    openPlaylistPicker,
  } from "../lib/stores.js";
  import { CAN_KEEP } from "../lib/remote/mode.js";
  import { toggleFavorite, buildTrackMenu } from "../lib/actions.js";
  import { duration as fmtDuration } from "../lib/format.js";
  import { playbackIdle } from "../lib/playback.js";
  import Icon from "./Icon.svelte";
  import { iconOf, offMain, sourceTitle } from "../lib/sources.js";
  import Cover from "./Cover.svelte";
  import ArtistLine from "./ArtistLine.svelte";

  export let track;
  export let index = null; // optional track number
  export let onplay = null; // optional override: () => void
  export let showAlbum = true;
  export let showCover = true; // album thumbnail per row (off on album pages)

  $: isCurrent = $currentId === trackKey(track);
  $: isPlaying = isCurrent && $playing;
  // A row may only claim a quality it was TOLD. The API carries a track's own
  // quality on some payloads and YouTube's archived bitrate, and nothing else;
  // anything else would be the quality the user ASKED for, not the file they
  // hear. An empty tag keeps the cell silent rather than inventing "FLAC".
  $: quality = qualityTag(track);
  $: fav = $favorites.has(trackKey(track));
  $: downloaded = $downloads.has(trackKey(track));
  // Flagged by the API when the list was fetched, or discovered mid-session by
  // the player's probe. A track you have downloaded plays from the device, so it
  // is never "unavailable" to YOU whatever Deezer decided.
  $: unavailable =
    !downloaded &&
    (track.unavailable === true || $unavailableIds.has(trackKey(track)));

  function qualityTag(t) {
    const q = t?.quality;
    if (q) return String(q).replace(/^OPUS[_-](\d+)$/i, "OPUS $1");
    // YouTube's file really is Opus at the archived bitrate (lib/youtube.js).
    if (t?.source === "youtube" && t.bitrate) return `YT ${t.bitrate}k`;
    return "";
  }

  function play() {
    if (isCurrent) player.toggle();
    else if (onplay) onplay();
    else player.playTrack(track);
  }

  function menu(e) {
    e.preventDefault();
    e.stopPropagation();
    const coords = { clientX: e.clientX, clientY: e.clientY, preventDefault() {}, stopPropagation() {} };
    openMenu(coords, buildTrackMenu(track, push));
  }

  // -- swipe right: "lire ensuite" -------------------------------------------
  // Touch-only, and deliberately the SAME gesture language as the now-playing
  // sheet: arm on touchstart, commit to an axis after a few pixels, and never
  // fight the scroller — a gesture that locks to "y" is left entirely alone, so
  // flicking through a 4000-track list feels exactly as it did before.
  //
  // The row slides right and the strip it vacates on the LEFT is what shows the
  // action. Drawing it there (rather than behind the row) is what keeps this
  // cheap and correct: the row itself stays transparent, so it needs no opaque
  // background to hide a layer underneath and nothing has to match whatever
  // page background — gradient header included — it happens to sit on.
  const AXIS_LOCK = 10; // px before a gesture commits to an axis
  const COMMIT = 64; // px of pull that actually queues the track
  const MAX_PULL = 104; // hard stop, rubber-banded past COMMIT
  // iOS reserves the very left edge for its own back gesture; starting there
  // means the system wins mid-drag and we would be left holding a translated
  // row. Cheaper to never arm than to recover from it.
  const EDGE_GUARD = 24;

  let dx = 0;
  let dragging = false; // finger down AND locked to the horizontal axis
  let armed = false;
  let axis = null;
  let startX = 0;
  let startY = 0;
  let swiped = false; // this gesture ended in a commit -> swallow the click

  $: committed = dx >= COMMIT;

  // A recycled row (TrackList keys rows by position): a gesture begun on the
  // previous track must never commit on this one.
  let swipeFor = null;
  $: if (track !== swipeFor) {
    swipeFor = track;
    resetSwipe();
    swiped = false;
  }

  function resetSwipe() {
    armed = false;
    axis = null;
    dragging = false;
    dx = 0;
  }

  function onTouchStart(e) {
    swiped = false;
    // Multi-touch, or a gesture that starts on a control, belongs to that
    // control (or to a pinch) — not to us.
    if (e.touches.length !== 1 || e.target.closest("button")) {
      armed = false;
      return;
    }
    const t = e.touches[0];
    if (t.clientX < EDGE_GUARD) {
      armed = false;
      return;
    }
    armed = true;
    axis = null;
    startX = t.clientX;
    startY = t.clientY;
  }

  function onTouchMove(e) {
    if (!armed) return;
    const t = e.touches[0];
    const mx = t.clientX - startX;
    const my = t.clientY - startY;
    if (axis === null && (Math.abs(mx) > AXIS_LOCK || Math.abs(my) > AXIS_LOCK)) {
      axis = Math.abs(mx) > Math.abs(my) ? "x" : "y";
      // A vertical gesture is the scroller's: disarm so we never look at it again.
      if (axis === "y") armed = false;
    }
    if (axis !== "x") return;
    // Right only: a left pull has no action behind it, so it must not move.
    if (mx <= 0) {
      dragging = false;
      dx = 0;
      return;
    }
    // Hold the horizontal gesture so the list doesn't scroll under the finger.
    if (e.cancelable) e.preventDefault();
    dragging = true;
    dx = mx <= COMMIT ? mx : COMMIT + Math.min(MAX_PULL - COMMIT, (mx - COMMIT) * 0.35);
  }

  // The click the browser synthesises after a drag would play the track (or hit
  // whatever control the finger happens to be over) on release. preventDefault
  // on touchend normally stops it; this capture-phase guard is what makes that
  // certain on the engines that emit it anyway.
  function onClickCapture(e) {
    if (!swiped) return;
    swiped = false;
    e.preventDefault();
    e.stopPropagation();
  }

  function onTouchEnd(e) {
    if (!armed) return;
    armed = false;
    const go = axis === "x" && dx >= COMMIT;
    // Any horizontal drag has already moved the row under the finger: the click
    // the browser would synthesise from it would play the track (or hit a
    // control) on release. Swallow it.
    if (axis === "x" && dx > 0) {
      swiped = true;
      if (e.cancelable) e.preventDefault();
    }
    axis = null;
    dragging = false;
    dx = 0;
    if (go) {
      player.playNext([track]);
      try {
        navigator.vibrate?.(12);
      } catch {
        /* no haptics here — the queue still got the track */
      }
    }
  }
</script>

<!-- The click handler here only SWALLOWS the click a drag synthesises; the row
     itself keeps every one of its real controls. -->
<!-- svelte-ignore a11y-no-static-element-interactions a11y-click-events-have-key-events -->
<div
  class="swipe"
  class:dragging
  on:touchstart|passive={onTouchStart}
  on:touchmove={onTouchMove}
  on:touchend={onTouchEnd}
  on:touchcancel={resetSwipe}
  on:click|capture={onClickCapture}
>
  <!-- The strip the row vacates: it only ever exists to the LEFT of the row,
       so it is clipped to exactly the pulled distance and never paints under
       the row itself. -->
  <div class="action" class:committed style="width:{dx}px" aria-hidden="true">
    <span class="act-in"><Icon name="next" size={20} /></span>
  </div>

  <div
    class="row track"
    class:active={isCurrent}
    class:noalb={!(showAlbum && track.album)}
    class:over={index === null && showCover}
    style={dx ? `transform:translate3d(${dx}px,0,0)` : ""}
    on:dblclick={play}
    on:contextmenu={menu}
  >
    <button class="play" on:click={play} aria-label="Lire">
      {#if isPlaying && $playbackIdle}
        <span class="eq" aria-hidden="true"><i></i><i></i><i></i></span>
      {:else if isPlaying}
        <!-- current row but audio isn't flowing yet (loading/buffering): a spinner
             tells the truth instead of an equalizer dancing over silence. -->
        <span class="rowspin" aria-hidden="true"></span>
      {:else if isCurrent}
        <!-- The playing row wears the accent and a triangle where a number
             would be (DESIGN §6), so "current" is a shape, not only a colour. -->
        <span class="curplay" aria-hidden="true"><Icon name="play" size={13} /></span>
      {:else}
        {#if index !== null}<span class="num">{index}</span>{/if}
        <span class="ic"><Icon name="play" size={15} /></span>
      {/if}
    </button>

    <!-- The title and artist are the row's main target, as in every music app:
         a tap there plays, the controls around it keep their own job. -->
    <!-- svelte-ignore a11y-no-static-element-interactions a11y-click-events-have-key-events -->
    <div class="titles" on:click={play}>
      {#if showCover}
        <div class="thumb"><Cover src={track.album?.cover} alt={track.title} size={44} kind="track" fallbackId={trackRef(track)} /></div>
      {/if}
      <div class="meta">
        <div class="t" class:gone={unavailable}>
          {#if unavailable}
            <button
              class="gonebadge"
              title="Titre indisponible — cliquez pour le remplacer"
              aria-label="Titre indisponible, remplacer"
              on:click|stopPropagation={() => openReplace(track)}><Icon name="alert" size={13} /></button>
          {/if}
          {#if offMain(track.source)}<span class="local" title={sourceTitle(track.source)}><Icon name={iconOf(track.source)} size={13} /></span>{:else if track.local}<span class="local" title={sourceTitle("local")}><Icon name="cloudOff" size={13} /></span>{/if}
          {#if downloaded}<span class="dlbadge" title="Disponible hors-ligne"><Icon name="downloaded" size={13} /></span>{/if}
          {track.title}
          {#if track.explicit}<span class="explicit">E</span>{/if}
        </div>
        <span class="a muted"><ArtistLine {track} /></span>
      </div>
    </div>

    {#if showAlbum && track.album}
      {#if entityRef(track.album)}
        <button class="alb muted" on:click|stopPropagation={() => push("/album/" + trackEntityRef(track, "album", track.album))}>
          {track.album.title}
        </button>
      {:else}
        <!-- No page to open (a YouTube track's album is only a label). -->
        <span class="alb muted plain">{track.album.title}</span>
      {/if}
    {/if}

    <span class="qcell">
      {#if quality}<span class="qbadge" title={`Qualité du fichier : ${quality}`}>{quality}</span>{/if}
    </span>

    <span class="acts">
      {#if CAN_KEEP}
        <button class="add" on:click|stopPropagation={() => openPlaylistPicker(track)} aria-label="Ajouter à une playlist" title="Ajouter à une playlist">
          <Icon name="plus" size={17} />
        </button>
      {/if}
      <button class="fav" class:on={fav} on:click|stopPropagation={() => toggleFavorite(track)} aria-label="Favori">
        <Icon name={fav ? "heartFilled" : "heart"} size={17} />
      </button>
    </span>
    <span class="dur muted">{fmtDuration(track.duration)}</span>
    <!-- The row menu takes the duration's place on hover (Spotify's move): the
         row keeps ONE action column and the prototype's 56 px time column. -->
    <button class="more" on:click|stopPropagation={menu} aria-label="Plus d'options"><Icon name="more" size={18} /></button>
  </div>
</div>

<style>
  /* Clips the pulled row and hosts the reveal strip. `position: relative` only
     — no transform, no stacking context beyond that, so the virtual list keeps
     measuring rows exactly as before. */
  .swipe {
    position: relative;
    overflow: hidden;
    border-radius: 10px;
    /* The row lays itself out on ITS width, not the screen's: a list in the
       full-screen player's side panel is phone-narrow on a desktop. */
    container: trackrow / inline-size;
  }
  /* Released: glide home. Held: follow the finger with no transition at all.
     The strip and the row share the SAME curve and duration — they are two
     halves of one movement, and letting the strip snap shut while the row was
     still travelling left a gap of bare page behind it for a fifth of a
     second. */
  .swipe .track,
  .swipe .action {
    transition: transform 0.22s cubic-bezier(0.22, 1, 0.36, 1),
      width 0.22s cubic-bezier(0.22, 1, 0.36, 1);
  }
  .swipe.dragging .track,
  .swipe.dragging .action {
    transition: none;
  }
  .swipe.dragging .track {
    will-change: transform;
  }
  .action {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    overflow: hidden;
    display: flex;
    align-items: center;
    border-radius: 10px;
    background: var(--s2);
    color: var(--dim);
    pointer-events: none;
  }
  .action.committed {
    background: var(--ac);
    color: var(--onac);
  }
  /* Pinned to the left of the strip so the glyph stays put while the strip
     grows around it — the icon is revealed like a curtain rather than drifting
     with the finger. Deliberately WORDLESS: the strip is only ~64 px wide at
     the point it commits, and any label long enough to be honest ("Lire
     ensuite") gets cut down to a word that names a different action ("Lire").
     The toast on release says it in full. */
  .act-in {
    display: flex;
    align-items: center;
    padding-left: 18px;
    transition: transform 0.14s ease;
  }
  .action.committed .act-in {
    transform: scale(1.12);
  }
  /* DESIGN §6, one grid for every list: number · title+artist (cover 44) ·
     album · quality · + and heart · duration. Without an album the column is
     dropped rather than left empty (its `1fr` used to swallow the heart). */
  .track {
    display: grid;
    grid-template-columns: 36px minmax(0, 1fr) minmax(0, 0.8fr) 64px 76px 56px;
    align-items: center;
    gap: 12px;
    padding: 6px 8px;
    border-radius: 10px;
    user-select: none;
    position: relative;
  }
  .track.noalb {
    grid-template-columns: 36px minmax(0, 1fr) 64px 76px 56px;
  }
  /* No track number but a cover: the play glyph (and the playing indicator)
     sits ON the cover instead of in a column of its own — the list then starts
     at the cover, as Spotify's and Apple Music's do. */
  .track.over {
    grid-template-columns: minmax(0, 1fr) minmax(0, 0.8fr) 64px 76px 56px;
  }
  .track.over.noalb {
    grid-template-columns: minmax(0, 1fr) 64px 76px 56px;
  }
  .track:hover {
    background: var(--s2);
  }
  .track.active .t {
    color: var(--ac);
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
  .track:hover .play .num {
    display: none;
  }
  .track:hover .play .ic {
    display: block;
  }
  .titles {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
    cursor: pointer;
  }
  .track.over .play {
    position: absolute;
    left: 15px;
    top: 50%;
    transform: translateY(-50%);
    z-index: 1;
    border-radius: 8px;
    color: var(--tx);
    pointer-events: none;
  }
  .track.over:hover .play,
  .track.over.active .play {
    background: var(--s2);
  }
  .track.over:not(:hover) .play .ic {
    display: none;
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
  /* A dead track stays readable but visibly demoted, and its badge is the way
     in to replacing it — the icon IS the button, so there's no hunting. */
  .t.gone {
    color: var(--dim);
  }
  .gonebadge {
    display: inline-flex;
    vertical-align: -2px;
    color: var(--ac);
    margin-right: 4px;
    padding: 0;
  }
  .gonebadge:hover {
    filter: brightness(1.25);
  }
  .dlbadge {
    display: inline-flex;
    vertical-align: -2px;
    color: var(--ac);
    margin-right: 3px;
  }
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
  /* `.a` holds one hoverable button per credited artist (ArtistLine), so the
     hover treatment belongs on each name, not on the whole line. */
  .alb:hover:not(.plain) {
    color: var(--tx);
    text-decoration: underline;
  }
  /* The file's real quality, in the metadata face: a badge, not a claim the
     row made up (see `quality` above). An empty cell is silent. */
  .qcell {
    display: flex;
    justify-content: center;
    min-width: 0;
  }
  .qbadge {
    max-width: 100%;
    padding: 2px 6px;
    border: 1px solid var(--ln);
    border-radius: 6px;
    font-family: var(--font-mono);
    font-size: 10px;
    letter-spacing: 0.02em;
    color: var(--dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .acts {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 2px;
    min-width: 0;
  }
  .add,
  .fav {
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    border-radius: 50%;
    color: var(--dim);
    opacity: 0;
  }
  .add:hover,
  .fav:hover {
    color: var(--tx);
    background: var(--s2);
  }
  .track:hover .add,
  .track:hover .fav,
  .fav.on {
    opacity: 1;
  }
  .fav.on {
    color: var(--hrt);
  }
  .dur {
    text-align: right;
    font-family: var(--font-mono);
    font-size: 13px;
    font-variant-numeric: tabular-nums;
    color: var(--dim);
  }
  /* The row menu borrows the duration's place on hover/focus (Spotify's move):
     one action column, the prototype's 56 px time column, no third icon. */
  .more {
    position: absolute;
    right: 6px;
    top: 50%;
    transform: translateY(-50%);
    width: 30px;
    height: 30px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    color: var(--dim);
    opacity: 0;
  }
  .more:hover {
    color: var(--tx);
    background: var(--s2);
  }
  .track:hover .more,
  .track:focus-within .more {
    opacity: 1;
  }
  .track:hover .dur,
  .track:focus-within .dur {
    opacity: 0;
  }
  /* small spinner shown on the current row while audio isn't flowing yet */
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
  /* A finger has no hover: the row's own controls are simply there. */
  @media (hover: none) {
    .add,
    .fav {
      opacity: 1;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .eq i {
      animation: none;
      height: 70%;
    }
    .swipe .track,
    .swipe .action,
    .act-in {
      transition: none;
    }
  }
  /* A phone keeps cover, title and actions; album, quality and duration are
     metadata that a 390 px row cannot carry honestly. The row menu has no
     column of its own here and is reached by long-press (contextmenu). */
  @media (max-width: 640px) {
    .track,
    .track.noalb {
      grid-template-columns: 36px minmax(0, 1fr) 68px;
      gap: 8px;
    }
    .track.over,
    .track.over.noalb {
      grid-template-columns: minmax(0, 1fr) 68px;
    }
    .alb,
    .qcell,
    .dur,
    .more {
      display: none;
    }
    .add,
    .fav {
      opacity: 1;
    }
  }
  /* The same, for a narrow LIST on a wide screen (the player's side panel):
     phone rules, plus the heart first in the action cell so it stays glued to
     the title instead of floating after the `+`. */
  @container trackrow (max-width: 520px) {
    .track,
    .track.noalb {
      grid-template-columns: 36px minmax(0, 1fr) 68px;
      gap: 8px;
    }
    .track.over,
    .track.over.noalb {
      grid-template-columns: minmax(0, 1fr) 68px;
    }
    .acts {
      gap: 0;
    }
    .alb,
    .qcell,
    .dur,
    .more {
      display: none;
    }
    .acts {
      flex-direction: row-reverse;
      justify-content: flex-end;
    }
    .add,
    .fav {
      opacity: 1;
    }
  }
</style>
