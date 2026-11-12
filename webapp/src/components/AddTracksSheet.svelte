<script>
  import { trackKey, trackRef } from "../lib/trackkey.js";
  import { onDestroy } from "svelte";
  import { api } from "../lib/api.js";
  import { toasts } from "../lib/stores.js";
  import { duration as fmtDuration, artistLine } from "../lib/format.js";
  import Icon from "./Icon.svelte";
  import { iconOf, offMain, sourceTitle } from "../lib/sources.js";
  import Cover from "./Cover.svelte";

  // Modal (desktop) / bottom-sheet (mobile) to add tracks to a playlist. Search
  // covers both Deezer and the local library (the /search endpoint merges them),
  // so a single box adds either kind. Adds are fully optimistic: the box ticks
  // instantly and the parent page performs (and sequences) the actual API call.
  // Same grammar as PlaylistPicker: a ticked box IS the add, and adding twice
  // is impossible here (`added` + the playlist's own `existingIds`).
  export let existingIds = new Set();
  export let onclose = null;
  export let onadd = null; // (track) => void — owns the API call

  let q = "";
  let results = [];
  let loading = false;
  let added = new Set();
  let timer = null;

  function onInput() {
    clearTimeout(timer);
    timer = setTimeout(search, 280);
  }

  // Sequence guard: a slow earlier query must not overwrite a newer one's
  // results (typing "dire straits" could land back on the "dir" results).
  let seq = 0;
  async function search() {
    const term = q.trim();
    const mine = ++seq;
    if (!term) {
      results = [];
      loading = false;
      return;
    }
    loading = true;
    try {
      const r = await api.search(term);
      if (mine === seq) results = r.tracks || [];
    } catch {
      if (mine === seq) results = [];
    } finally {
      if (mine === seq) loading = false;
    }
  }

  function add(track) {
    const id = trackKey(track);
    if (added.has(id)) return;
    added = new Set(added).add(id);
    onadd?.(track);
    toasts.push("Ajouté à la playlist", "info", 2400);
  }

  function isIn(track) {
    const id = trackKey(track);
    return added.has(id) || existingIds.has(id);
  }

  function onKey(e) {
    if (e.key === "Escape") onclose?.();
  }

  // Hand the focus back to the button that opened the sheet (the page's +):
  // the router keeps that page mounted, so the element is still there. Read at
  // INIT — by mount time the autofocused search box already holds the focus.
  let returnFocus = typeof document !== "undefined" ? document.activeElement : null;
  onDestroy(() => {
    if (returnFocus && returnFocus.isConnected) returnFocus.focus?.({ preventScroll: true });
  });
</script>

<svelte:window on:keydown={onKey} />

<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
<div class="backdrop" on:click={() => onclose?.()}>
  <div class="sheet" on:click|stopPropagation role="dialog" aria-modal="true" tabindex="-1" aria-label="Ajouter des titres">
    <header>
      <div class="ttl">
        <h2>Ajouter des titres</h2>
        <p class="sub">Cherche dans Deezer ou dans ta bibliothèque locale.</p>
      </div>
      <button type="button" class="close" on:click={() => onclose?.()} aria-label="Fermer">
        <Icon name="close" size={18} />
      </button>
    </header>

    <label class="searchbox">
      <Icon name="search" size={16} />
      <!-- svelte-ignore a11y-autofocus -->
      <input placeholder="Rechercher un titre…" aria-label="Rechercher un titre" bind:value={q} on:input={onInput} autofocus />
    </label>

    <div class="results">
      {#if loading}
        <p class="state">Recherche…</p>
      {:else if q.trim() && !results.length}
        <p class="state">Aucun résultat pour « {q.trim()} ».</p>
      {:else if !q.trim()}
        <p class="state">Tape le nom d'un titre, d'un artiste ou d'un album.</p>
      {/if}

      {#each results as t (trackKey(t))}
        <div class="res">
          <span class="thumb"><Cover src={t.album?.cover} alt={t.title} size={40} kind="track" fallbackId={trackRef(t)} /></span>
          <span class="meta">
            <span class="t">
              {#if offMain(t.source)}<span class="src" title={sourceTitle(t.source)}><Icon name={iconOf(t.source)} size={12} /></span>{:else if t.local}<span class="src" title="Fichier local"><Icon name="cloudOff" size={12} /></span>{/if}
              <span class="tt">{t.title}</span>
            </span>
            <span class="sub2">{artistLine(t)}{t.album?.title ? " · " + t.album.title : ""}</span>
          </span>
          <span class="dur mono">{fmtDuration(t.duration)}</span>
          <button
            type="button"
            class="box"
            class:on={isIn(t)}
            role="checkbox"
            aria-checked={isIn(t)}
            aria-label={isIn(t) ? `« ${t.title} » déjà dans la playlist` : `Ajouter « ${t.title} »`}
            disabled={isIn(t)}
            on:click={() => add(t)}
          >
            {#if isIn(t)}<Icon name="check" size={14} />{/if}
          </button>
        </div>
      {/each}
    </div>
  </div>
</div>

<style>
  /* The veil is the theme's ink at 60 % — the same rule as PlaylistPicker. */
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 400;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 20px;
    background: color-mix(in srgb, var(--onac) 60%, transparent);
    overscroll-behavior: contain;
    touch-action: none;
  }
  .sheet {
    width: min(520px, 100%);
    min-width: 0;
    max-width: 100%;
    max-height: min(720px, calc(100vh - 40px));
    max-height: min(720px, calc(100dvh - 40px));
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 24px;
    background: var(--s1);
    border: 1px solid var(--ln);
    border-radius: 24px;
    color: var(--tx);
    box-shadow: 0 24px 64px color-mix(in srgb, var(--onac) 45%, transparent);
    touch-action: auto;
  }
  header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
  }
  .ttl {
    min-width: 0;
  }
  .ttl h2 {
    margin: 0;
    font-family: var(--font-display);
    font-size: 24px;
    font-weight: 700;
    letter-spacing: -0.02em;
    line-height: 1.1;
  }
  .sub {
    margin: 4px 0 0;
    font-size: 14px;
    color: var(--dim);
  }
  .close {
    flex: none;
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    border-radius: 999px;
    background: var(--s2);
    color: var(--tx);
  }
  .close:hover {
    background: color-mix(in srgb, var(--tx) 12%, var(--s2));
  }
  .searchbox {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 44px;
    flex: none;
    padding: 0 14px;
    border-radius: 999px;
    border: 1px solid var(--ln);
    background: var(--bg);
    color: var(--dim);
    font-size: 14px;
  }
  .searchbox:focus-within {
    border-color: var(--ac-text);
    color: var(--tx);
  }
  .searchbox input {
    flex: 1;
    min-width: 0;
    border: none;
    background: none;
    outline: none;
    color: var(--tx);
    font: inherit;
  }
  .searchbox input::placeholder {
    color: var(--dim);
    opacity: 1;
  }
  .results {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-height: 0;
    overflow-y: auto;
    margin: 0 -6px;
    padding: 0 6px;
  }
  .state {
    margin: 0;
    padding: 18px 6px;
    text-align: center;
    color: var(--dim);
    font-size: 14px;
  }
  .res {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 56px;
    padding: 8px;
    border-radius: 12px;
  }
  .res:hover {
    background: var(--s2);
  }
  .thumb {
    flex: none;
    width: 40px;
    height: 40px;
  }
  .meta {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .t {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    font-weight: 600;
    font-size: 15px;
  }
  .tt {
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .src {
    flex: none;
    display: inline-flex;
    color: var(--dim);
  }
  .sub2 {
    font-size: 12px;
    color: var(--dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dur {
    flex: none;
    font-size: 12px;
    color: var(--dim);
  }
  .box {
    flex: none;
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: 7px;
    border: 2px solid var(--dim);
    color: var(--onac);
  }
  .box:hover:not(:disabled) {
    border-color: var(--tx);
  }
  .box.on {
    background: var(--ac);
    border-color: var(--ac-text);
  }
  .box:disabled {
    cursor: default;
  }
  @media (pointer: coarse) {
    .close {
      width: 44px;
      height: 44px;
    }
    .box {
      width: 30px;
      height: 30px;
    }
  }
  @media (max-width: 640px) {
    .backdrop {
      align-items: flex-end;
      padding: 0;
    }
    .sheet {
      width: 100%;
      max-width: 100%;
      max-height: 92vh;
      max-height: 92dvh;
      border-radius: 24px 24px 0 0;
      border-bottom: none;
      padding: 20px 18px calc(18px + env(safe-area-inset-bottom, 0px));
    }
    .close {
      width: 44px;
      height: 44px;
    }
  }
</style>
