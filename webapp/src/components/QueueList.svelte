<script>
  // The queue, as one component shared by the three places that show it (the
  // desktop side panel, the desktop full-screen player, the phone's sheet).
  //
  // Everything that edits it lives here so the three cannot drift apart:
  // tap to play a row, remove an upcoming one, drag one to reorder, clear what
  // is left. Only what is AFTER the playing track can be moved or removed —
  // moving the track being heard, or one already played, is not something a
  // queue is for.
  //
  // The list is windowed (VirtualList), so a drag never depends on a row being
  // mounted: the target row comes from geometry (`indexAt`), the picture that
  // follows the finger is a separate floating copy, and the pointer is followed
  // on the window rather than on the grip — the row may scroll out of the DOM
  // while it is being dragged toward the end of a 4 000-track queue.
  import { trackRef } from "../lib/trackkey.js";
  import { onDestroy, tick } from "svelte";
  import { player, toasts, openPlaylistPicker, openQueueSave } from "../lib/stores.js";
  import { atLeast } from "../lib/remote/mode.js";
  import { duration as fmtDuration, artistLine } from "../lib/format.js";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";
  import VirtualList from "./VirtualList.svelte";

  export let items = [];
  export let idx = -1; // index of the playing track
  export let showDuration = false;
  // Called after a row is tapped (the phone's sheet closes itself).
  export let onPick = null;

  // Moving and removing are "read" level on a remote control; jumping is the
  // queue level and is always there.
  const editable = atLeast("read");

  let vl;
  let root;
  let drag = null; // { from, to, y, left, width, item }
  let raf = 0;
  let lastY = 0;

  $: upcoming = Math.max(0, items.length - idx - 1);

  export function scrollToIndex(...args) {
    return vl?.scrollToIndex(...args);
  }

  function pick(i) {
    player.jump(i);
    if (onPick) onPick(i);
  }

  function clearAll() {
    player.clearUpcoming();
    toasts.push("File vidée");
  }

  function buzz(ms) {
    try {
      navigator.vibrate?.(ms);
    } catch {
      /* not everywhere */
    }
  }

  // -- dragging ---------------------------------------------------------------

  function startDrag(e, from) {
    if (!editable || (e.button != null && e.button !== 0)) return;
    e.preventDefault();
    // A touch pointer is captured by the element it began on, which is removed
    // from the DOM if the list scrolls far enough: hit-test normally instead.
    try {
      e.currentTarget.releasePointerCapture?.(e.pointerId);
    } catch {
      /* not captured */
    }
    const box = root.getBoundingClientRect();
    drag = { from, to: from, y: e.clientY, left: box.left, width: box.width, item: items[from] };
    lastY = e.clientY;
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", cancel);
    window.addEventListener("keydown", onKey);
    raf = requestAnimationFrame(loop);
    buzz(10);
  }

  function retarget() {
    if (!drag) return;
    const at = vl ? vl.indexAt(lastY) : drag.from;
    // Nothing can land on or before the playing track.
    const to = Math.max(idx + 1, at < 0 ? drag.from : at);
    if (to !== drag.to) {
      drag = { ...drag, to };
      buzz(4);
    }
  }

  function onMove(e) {
    if (!drag) return;
    lastY = e.clientY;
    drag = { ...drag, y: e.clientY };
    retarget();
  }

  // Once a frame: the edge scroll, then the target again — scrolling moved the
  // rows under a pointer that itself did not move.
  function loop() {
    if (!drag) return;
    if (vl && vl.autoScroll(lastY)) retarget();
    raf = requestAnimationFrame(loop);
  }

  function stop() {
    cancelAnimationFrame(raf);
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", onUp);
    window.removeEventListener("pointercancel", cancel);
    window.removeEventListener("keydown", onKey);
    drag = null;
  }

  function onUp() {
    const d = drag;
    stop();
    if (d && d.to !== d.from) player.move(d.from, d.to);
  }

  function cancel() {
    stop();
  }

  function onKey(e) {
    if (e.key === "Escape") {
      e.preventDefault();
      stop();
    }
  }

  onDestroy(stop);

  // The same thing from the keyboard: the grip is focusable and the arrows move
  // the row one place, keeping the focus on it.
  async function gripKey(e, i) {
    const step = e.key === "ArrowUp" ? -1 : e.key === "ArrowDown" ? 1 : 0;
    if (!step) return;
    e.preventDefault();
    const to = i + step;
    if (to <= idx || to >= items.length) return;
    player.move(i, to);
    await tick();
    root?.querySelector(`[data-qi="${to}"] .grip`)?.focus();
  }
</script>

<div class="ql" bind:this={root}>
  {#if editable && upcoming > 0}
    <div class="qhead">
      <div class="qbar">
        <span class="qcount">À suivre · {upcoming} titre{upcoming > 1 ? "s" : ""}</span>
        <button class="qclear" on:click={clearAll} aria-label="Vider la file" title="Vider la file">
          <Icon name="trash" size={15} />
        </button>
      </div>
      <!-- Sillon §7: what is left of the queue is a list one keeps — the same
           sheet that saves a track saves the whole run. -->
      <button class="qsave" on:click={openQueueSave}>
        <Icon name="plus" size={14} />
        Enregistrer en playlist
      </button>
    </div>
  {/if}

  <VirtualList {items} bind:this={vl} estimateHeight={56} let:item let:index>
    <div
      class="qitem"
      class:now={index === idx}
      class:past={index < idx}
      class:edit={editable && index > idx}
      class:dragging={drag && drag.from === index}
      class:before={drag && drag.to === index && drag.to < drag.from}
      class:after={drag && drag.to === index && drag.to > drag.from}
      data-qi={index}
    >
      <button class="qrow" on:click={() => pick(index)}>
        <Cover src={item.album?.cover} alt="" size={40} kind={item.podcast ? "podcast" : "track"} fallbackId={trackRef(item)} />
        <span class="qm">
          <span class="qt">{item.title}</span>
          <span class="qa">{artistLine(item)}</span>
        </span>
        {#if showDuration}<span class="qd mono">{fmtDuration(item.duration)}</span>{/if}
      </button>
      {#if editable && index > idx}
        <button class="qadd" on:click={() => openPlaylistPicker(item)} aria-label="Ajouter « {item.title} » à une playlist" title="Ajouter à une playlist">
          <Icon name="plus" size={15} />
        </button>
        <button class="qx" on:click={() => player.removeAt(index)} aria-label="Retirer « {item.title} » de la file" title="Retirer">
          <Icon name="close" size={15} />
        </button>
        <span
          class="grip"
          role="button"
          tabindex="0"
          aria-label="Déplacer « {item.title} » (glisser, ou flèches haut et bas)"
          title="Glisser pour déplacer"
          on:pointerdown={(e) => startDrag(e, index)}
          on:keydown={(e) => gripKey(e, index)}
        >
          <Icon name="grip" size={18} />
        </span>
      {:else if editable}
        <!-- The playing and played rows have no controls: an empty slot keeps
             their duration in the same column as everyone else's. -->
        <span class="qslot" aria-hidden="true"></span>
      {/if}
    </div>
  </VirtualList>
</div>

{#if drag}
  <!-- The row as it is carried: a copy, so it survives the real one leaving the
       window when the list scrolls under the finger. -->
  <div class="ghost" style="top:{drag.y}px; left:{drag.left}px; width:{drag.width}px" aria-hidden="true">
    <Cover src={drag.item.album?.cover} alt="" size={40} kind={drag.item.podcast ? "podcast" : "track"} fallbackId={trackRef(drag.item)} />
    <span class="qm">
      <span class="qt">{drag.item.title}</span>
      <span class="qa">{artistLine(drag.item)}</span>
    </span>
  </div>
{/if}

<style>
  /* The parent sets the palette: --ql-dim (secondary text), --ql-hover (row
     under the pointer), --ql-bg (the sticky header's ground) and --ql-ghost.
     Everything default to the theme's own tokens, so a panel that says nothing
     is already in Sillon. */
  .ql {
    color: inherit;
  }
  .qhead {
    position: sticky;
    top: 0;
    z-index: 2;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 4px 4px 8px;
    background: var(--ql-bg, transparent);
  }
  .qbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    min-width: 0;
  }
  .qcount {
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--ql-dim, var(--dim));
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .qclear {
    flex: none;
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: 8px;
    color: var(--ql-dim, var(--dim));
  }
  .qclear:hover {
    color: var(--tx);
    background: var(--ql-hover, var(--s2));
  }
  /* The accent action of the header: what a finished listen becomes. */
  .qsave {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    height: 32px;
    padding: 0 14px;
    border-radius: 999px;
    background: var(--ac);
    color: var(--onac);
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.01em;
    white-space: nowrap;
  }
  .qsave:hover {
    filter: brightness(1.06);
  }

  .qitem {
    position: relative;
    display: flex;
    align-items: center;
    box-sizing: border-box;
    height: 56px;
    border-radius: 10px;
  }
  .qitem:hover {
    background: var(--ql-hover, var(--s2));
  }
  .qitem.now {
    background: color-mix(in srgb, var(--ac) 10%, transparent);
  }
  /* The playing row is told apart by its tinted ground and its weight, not by
     the accent AS TEXT: on the light theme the accent is a fill, and lemon on
     paper is unreadable (the theme pair rule in docs/agents/webapp.md). */
  .qitem.now .qt {
    color: var(--tx);
    font-weight: 700;
  }
  .qitem.past {
    opacity: 0.5;
  }
  .qitem.dragging {
    opacity: 0.3;
  }
  /* Where the row will land: a line on the side it arrives from. */
  .qitem.before::before,
  .qitem.after::after {
    content: "";
    position: absolute;
    left: 6px;
    right: 6px;
    height: 2px;
    border-radius: 2px;
    background: var(--ac-mark);
    z-index: 1;
  }
  .qitem.before::before {
    top: -1px;
  }
  .qitem.after::after {
    bottom: -1px;
  }

  .qrow {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0 5px;
    border-radius: 10px;
    text-align: left;
    color: inherit;
  }
  .qm {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
    gap: 1px;
  }
  .qt,
  .qa {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .qt {
    font-weight: 600;
    font-size: 0.9rem;
    line-height: 1.25;
  }
  .qa {
    font-size: 0.76rem;
    color: var(--ql-dim, var(--dim));
    line-height: 1.25;
  }
  .qd {
    font-size: 11px;
    color: var(--ql-dim, var(--dim));
    font-variant-numeric: tabular-nums;
    flex: none;
  }

  .qadd,
  .qx,
  .grip {
    display: grid;
    place-items: center;
    flex: none;
    width: 28px;
    height: 32px;
    border-radius: 8px;
    color: var(--ql-dim, var(--dim));
  }
  .qadd:hover,
  .qx:hover,
  .grip:hover {
    color: var(--tx);
  }
  .grip {
    cursor: grab;
    touch-action: none;
    user-select: none;
    -webkit-user-select: none;
  }
  .grip:active {
    cursor: grabbing;
  }
  .grip:focus-visible {
    outline: 2px solid var(--ac-text);
    outline-offset: -2px;
  }
  /* The empty column of the playing and played rows: three controls' width, so
     every duration lands on the same x. */
  .qslot {
    flex: none;
    width: 84px;
  }
  /* §10: a finger gets 44 px on each — but three of them on a 340 px panel would
     leave the title a couple of letters, so the width stays, the height grows. */
  @media (pointer: coarse) {
    .qadd,
    .qx,
    .grip {
      width: 36px;
      height: 44px;
    }
    .qslot {
      width: 108px;
    }
  }

  .ghost {
    position: fixed;
    z-index: 400;
    display: flex;
    align-items: center;
    gap: 10px;
    box-sizing: border-box;
    height: 56px;
    padding: 0 6px;
    margin-top: -28px; /* centred on the pointer */
    border-radius: 12px;
    background: var(--ql-ghost, var(--s2));
    color: var(--tx);
    border: 1px solid var(--ln);
    box-shadow: 0 14px 40px color-mix(in srgb, var(--bg) 70%, transparent);
    pointer-events: none;
  }
</style>
