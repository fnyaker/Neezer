<script>
  import { tick } from "svelte";
  import {
    moveToFolder,
    closeMoveToFolder,
    playlists,
    playlistFolders,
    openFolderSheet,
  } from "../lib/stores.js";
  import { userPlaylists, movePlaylistTo } from "../lib/actions.js";
  import { groupPlaylists } from "../lib/playlistfolders.js";
  import Icon from "./Icon.svelte";

  let opened = false;
  let q = "";
  let busy = false;
  let sheet = null;
  let searchEl = null;
  let returnFocus = null;

  $: target = $moveToFolder;
  $: if (target && !opened) enter();
  else if (!target && opened) leave();

  $: folders = $playlistFolders || [];
  $: grouped = groupPlaylists($playlists || [], folders);
  $: rootCount = grouped.unfiled.length + grouped.imported.length;
  $: targets = folders.length + 1;
  $: manyTargets = targets > 8;
  $: term = q.trim().toLowerCase();
  $: shownFolders = term ? grouped.groups.filter((f) => f.title.toLowerCase().includes(term)) : grouped.groups;
  $: current = target?.playlist?.folder_id ?? null;
  $: currentName = folders.find((f) => String(f.id) === String(current))?.title || null;

  function enter() {
    opened = true;
    q = "";
    busy = false;
    returnFocus = typeof document !== "undefined" ? document.activeElement : null;
    userPlaylists();
    tick().then(() => {
      const focus = manyTargets ? searchEl : sheet;
      focus?.focus?.({ preventScroll: true });
    });
  }

  function leave() {
    opened = false;
    const back = returnFocus;
    returnFocus = null;
    tick().then(() => {
      if (back && back.isConnected) back.focus?.({ preventScroll: true });
    });
  }

  function close() {
    if (!busy) closeMoveToFolder();
  }

  async function choose(folderId) {
    if (busy || !target) return;
    const wanted = folderId == null ? null : String(folderId);
    if (String(current ?? "") === String(wanted ?? "")) {
      closeMoveToFolder();
      return;
    }
    busy = true;
    try {
      if (await movePlaylistTo(target.playlist.id, wanted)) closeMoveToFolder();
    } finally {
      busy = false;
    }
  }

  function createAndMove() {
    if (!target) return;
    const playlist = target.playlist;
    openFolderSheet({
      mode: "create",
      after: async (folder) => {
        if (await movePlaylistTo(playlist.id, folder.id)) closeMoveToFolder();
      },
    });
  }

  function countOf(n) {
    return `${n} playlist${n > 1 ? "s" : ""}`;
  }

  function onKey(e) {
    if (!target) return;
    if (e.key === "Escape") {
      e.preventDefault();
      close();
      return;
    }
    if (e.key !== "Tab" || !sheet) return;
    const focusable = [...sheet.querySelectorAll("button, input, [href], [tabindex]:not([tabindex='-1'])")].filter(
      (el) => !el.disabled && el.offsetParent !== null
    );
    if (!focusable.length) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    const active = document.activeElement;
    if (e.shiftKey && (active === first || active === sheet || !sheet.contains(active))) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && active === last) {
      e.preventDefault();
      first.focus();
    }
  }

  function toBody(node) {
    document.body.appendChild(node);
    return {
      destroy() {
        node.parentNode?.removeChild(node);
      },
    };
  }
</script>

<svelte:window on:keydown={onKey} />

{#if target}
  <div class="overlay" role="presentation" use:toBody on:click={(e) => e.target === e.currentTarget && close()}>
    <div
      class="sheet"
      role="dialog"
      aria-modal="true"
      aria-label="Déplacer vers"
      tabindex="-1"
      bind:this={sheet}
    >
      <header>
        <div class="ttl">
          <h2>Déplacer vers</h2>
          <p class="sub">{target.playlist.title}</p>
        </div>
        <button type="button" class="close" on:click={close} aria-label="Fermer">
          <Icon name="close" size={18} />
        </button>
      </header>

      {#if manyTargets}
        <label class="searchbox">
          <Icon name="search" size={16} />
          <input
            bind:this={searchEl}
            bind:value={q}
            placeholder="Filtrer les dossiers…"
            aria-label="Filtrer les dossiers"
          />
        </label>
      {/if}

      <div class="results">
        {#if $playlists === null}
          <p class="state">Chargement des playlists…</p>
        {:else}
          <button
            type="button"
            class="row"
            class:on={current == null}
            on:click={() => choose(null)}
            disabled={busy}
            aria-label={`Déplacer vers Sans dossier, ${countOf(rootCount)}`}
          >
            <span class="ico"><Icon name="folderMinus" size={20} /></span>
            <span class="meta">
              <span class="nm">Sans dossier</span>
              <span class="count mono">{countOf(rootCount)}</span>
            </span>
            {#if current == null}<span class="tick"><Icon name="check" size={18} /></span>{/if}
          </button>

          {#each shownFolders as f (f.id)}
            <button
              type="button"
              class="row"
              class:on={String(f.id) === String(current)}
              on:click={() => choose(f.id)}
              disabled={busy}
              aria-label={`Déplacer vers ${f.title}, ${countOf(f.count)}`}
            >
              <span class="ico"><Icon name="folder" size={20} /></span>
              <span class="meta">
                <span class="nm">{f.title}</span>
                <span class="count mono">{countOf(f.count)}</span>
              </span>
              {#if String(f.id) === String(current)}<span class="tick"><Icon name="check" size={18} /></span>{/if}
            </button>
          {/each}

          {#if term && !shownFolders.length}
            <p class="state">Aucun dossier ne correspond à « {q.trim()} ».</p>
          {/if}

          <button type="button" class="row new" on:click={createAndMove} disabled={busy}>
            <span class="ico"><Icon name="folderPlus" size={20} /></span>
            <span class="meta">
              <span class="nm">Nouveau dossier…</span>
            </span>
          </button>
        {/if}
      </div>

      {#if currentName}
        <p class="current">Dossier actuel · {currentName}</p>
      {/if}
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 420;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 20px;
    background: color-mix(in srgb, var(--onac) 60%, transparent);
    overscroll-behavior: contain;
    touch-action: none;
  }
  .sheet {
    width: min(440px, 100%);
    max-height: min(700px, calc(100dvh - 40px));
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 24px;
    background: var(--s1);
    border: 1px solid var(--ln);
    border-radius: 24px;
    color: var(--tx);
    box-shadow: 0 24px 64px color-mix(in srgb, var(--onac) 45%, transparent);
    touch-action: auto;
  }
  .sheet:focus {
    outline: none;
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
  h2 {
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
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .close {
    flex: none;
    width: 36px;
    height: 36px;
    display: grid;
    place-items: center;
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
    height: 40px;
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
  .results {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-height: 0;
    overflow-y: auto;
    margin: 0 -6px;
    padding: 0 6px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 48px;
    padding: 8px 10px;
    border-radius: 12px;
    text-align: left;
    color: var(--tx);
    font: inherit;
  }
  .row:hover:not(:disabled) {
    background: var(--s2);
  }
  .row:disabled {
    opacity: 0.6;
  }
  .row.on {
    background: var(--s2);
  }
  .ico {
    flex: none;
    width: 32px;
    height: 32px;
    display: grid;
    place-items: center;
    color: var(--dim);
  }
  .row.on .ico {
    color: var(--ac-text);
  }
  .meta {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .nm {
    font-size: 15px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .count {
    font-size: 11px;
    color: var(--dim);
  }
  .tick {
    flex: none;
    display: grid;
    place-items: center;
    color: var(--ac-text);
  }
  .new {
    margin-top: 4px;
    border: 1.5px dashed var(--dim);
  }
  .new .ico {
    color: var(--tx);
  }
  .state {
    margin: 0;
    padding: 18px 6px;
    text-align: center;
    color: var(--dim);
    font-size: 14px;
  }
  .current {
    margin: 0;
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--dim);
  }
  @media (pointer: coarse) {
    .close {
      width: 44px;
      height: 44px;
    }
    .searchbox {
      height: 44px;
    }
  }
  @media (max-width: 640px) {
    .overlay {
      padding: 0;
    }
    .sheet {
      width: 100%;
      height: 100dvh;
      max-height: 100dvh;
      border-radius: 0;
      border: none;
      padding: calc(16px + env(safe-area-inset-top, 0px)) 18px calc(16px + env(safe-area-inset-bottom, 0px));
    }
    .close {
      width: 44px;
      height: 44px;
    }
    .results {
      flex: 1;
    }
  }
</style>
