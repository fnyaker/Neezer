<script>
  // The one playlist modal (DESIGN §8). Three entries, one sheet:
  //   { track }        → add that title to a playlist (checkboxes, "Terminé")
  //   { mode: "new" }  → create an empty playlist (sidebar's +)
  //   { mode: "queue" }→ save the queue, in order, as a new playlist
  //
  // A checked box is an ACTION, not a stored membership: the API adds a track
  // but a playlist's tracklist is not on this screen (fetching one per playlist
  // on a phone link would be the fan-out rule 3 forbids), so the box shows what
  // THIS opening has already added. Ticking twice never adds twice.
  import { tick } from "svelte";
  import { get } from "svelte/store";
  import { trackRef } from "../lib/trackkey.js";
  import {
    playlistPicker,
    closePlaylistPicker,
    lastPlaylist,
    playlists,
    player,
  } from "../lib/stores.js";
  import {
    userPlaylists,
    addTrackToPlaylist,
    createPlaylistFrom,
    saveQueueAsPlaylist,
  } from "../lib/actions.js";
  import Icon from "./Icon.svelte";
  import Cover from "./Cover.svelte";
  import { isLocalId, artistLine } from "../lib/format.js";

  let q = ""; // playlist filter
  let name = ""; // create form
  let creating = false; // inline create form, in track mode
  let busy = false; // a create is in flight
  let added = new Set(); // playlist ids added during THIS opening
  let listFailed = false;
  const LIST_MAX = 200; // rows mounted at most (a sheet, not a library page)

  let opened = false;
  let sheet = null;
  let nameEl = null;
  let returnFocus = null;
  // Read ONCE per opening, never from `$player`: that store re-emits on every
  // progress tick, and a sheet that recomputes its queue four times a second is
  // the per-tick bug the now-playing panel already paid for.
  let queueRefs = [];
  // The pin too: re-reading `lastPlaylist` on add would move the row that was
  // just ticked to the top, under the finger.
  let pinned = null;

  $: picker = $playlistPicker;
  $: track = picker?.track || null;
  // A bare { track } is the "add a title" mode; the store's modes are the rest.
  $: mode = picker?.mode || (track ? "track" : null);
  // The store can move straight from one mode to another (no null in between):
  // that is a different sheet, so its fields start clean.
  let current = null;
  $: if (picker && !opened) enter();
  else if (picker && opened && picker !== current) refresh();
  else if (!picker && opened) leave();

  $: all = $playlists;
  $: shown = filterList(all, q, pinned);
  // Bounded: a person with hundreds of playlists still gets a sheet that opens
  // at once; the filter (shown above six) is how they reach the rest.
  $: visible = shown ? shown.slice(0, LIST_MAX) : shown;
  $: truncated = !!shown && shown.length > LIST_MAX;
  $: manyPlaylists = (all?.length || 0) > 6;
  $: suggestion = suggestName(all);
  $: formTitle = mode === "track" ? "Créer et ajouter" : "Créer";

  function enter() {
    opened = true;
    current = picker;
    q = "";
    name = "";
    creating = mode !== "track";
    busy = false;
    added = new Set();
    queueRefs = (get(player).queue || []).map(trackRef).filter(Boolean);
    pinned = get(lastPlaylist);
    // The trigger, to hand the focus back to on the way out.
    returnFocus = typeof document !== "undefined" ? document.activeElement : null;
    loadList();
    tick().then(() => {
      const target = creating ? nameEl : sheet;
      target?.focus?.({ preventScroll: true });
    });
  }

  // Same sheet, new mode: reset what the old one was holding, keep the trigger
  // we must hand the focus back to.
  function refresh() {
    current = picker;
    q = "";
    name = "";
    creating = mode !== "track";
    busy = false;
    added = new Set();
    queueRefs = (get(player).queue || []).map(trackRef).filter(Boolean);
    pinned = get(lastPlaylist);
    loadList();
    tick().then(() => (creating ? nameEl : sheet)?.focus?.({ preventScroll: true }));
  }

  function leave() {
    opened = false;
    current = null;
    const back = returnFocus;
    returnFocus = null;
    // After the sheet is gone, so the browser does not refuse the focus.
    tick().then(() => {
      if (back && back.isConnected) back.focus?.({ preventScroll: true });
    });
  }

  // One shared fetch (`playlists` store). A failure keeps the store as it was,
  // so "never loaded" stays `null` — that is the only case worth a retry line.
  function loadList(force = false) {
    listFailed = false;
    userPlaylists(force)
      .then(() => {
        if ($playlists === null) listFailed = true;
      })
      .catch(() => (listFailed = true));
  }

  function filterList(list, term, last) {
    if (!list) return null;
    let out = list;
    const t = (term || "").trim().toLowerCase();
    if (t) out = out.filter((p) => (p.title || "").toLowerCase().includes(t));
    // Pin the last-used playlist first — it is the most likely target.
    if (last?.id) {
      const i = out.findIndex((p) => String(p.id) === String(last.id));
      if (i > 0) out = [out[i], ...out.slice(0, i), ...out.slice(i + 1)];
    }
    return out;
  }

  // "Ma playlist 8" for eight playlists: the next number, never an existing name.
  function suggestName(list) {
    const used = new Set((list || []).map((p) => (p.title || "").trim().toLowerCase()));
    let n = (list?.length || 0) + 1;
    while (used.has(`ma playlist ${n}`)) n++;
    return `Ma playlist ${n}`;
  }

  function countOf(p) {
    const n = p.nb_tracks || 0;
    return `${n} titre${n > 1 ? "s" : ""}`;
  }

  function isAdded(p) {
    return added.has(String(p.id));
  }

  // Ticking adds NOW (the action helper toasts and optimistically bumps the
  // count; a failure comes back as its own toast with "Réessayer").
  function toggle(p) {
    if (!track || busy || isAdded(p)) return;
    const id = String(p.id);
    added = new Set(added).add(id);
    // Optimistic box; the action helper toasts at once and rolls the shared
    // count back if the server refuses — then the box does not stay ticked.
    addTrackToPlaylist(p.id, trackRef(track), p.title).then((ok) => {
      if (ok) return;
      const next = new Set(added);
      next.delete(id);
      added = next;
    });
  }

  function openCreating() {
    creating = true;
    name = "";
    tick().then(() => nameEl?.focus({ preventScroll: true }));
  }

  function cancel() {
    if (mode === "track") {
      // Back to the list, nothing lost.
      creating = false;
      name = "";
      tick().then(() => sheet?.focus({ preventScroll: true }));
    } else {
      closePlaylistPicker();
    }
  }

  async function submit() {
    if (busy) return;
    const title = (name.trim() || suggestion).trim();
    if (!title) return;
    if (mode === "queue" && !queueRefs.length) return;
    busy = true;
    try {
      let r = null;
      if (mode === "queue") r = await saveQueueAsPlaylist(title, queueRefs);
      else if (mode === "track" && track) r = await createPlaylistFrom(title, [trackRef(track)]);
      else r = await createPlaylistFrom(title, []);
      if (r) closePlaylistPicker();
    } finally {
      busy = false;
    }
  }

  // Escape closes; Tab stays inside the sheet (a simple cycle, enough for a
  // four-control dialog and honest for a keyboard user).
  function onKey(e) {
    if (!picker) return;
    if (e.key === "Escape") {
      e.preventDefault();
      closePlaylistPicker();
      return;
    }
    if (e.key !== "Tab" || !sheet) return;
    const f = [...sheet.querySelectorAll("button, input, [href], [tabindex]:not([tabindex='-1'])")].filter(
      (el) => !el.disabled && el.offsetParent !== null
    );
    if (!f.length) return;
    const first = f[0];
    const last = f[f.length - 1];
    const active = document.activeElement;
    if (e.shiftKey && (active === first || active === sheet || !sheet.contains(active))) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && active === last) {
      e.preventDefault();
      first.focus();
    }
  }

  // A fixed overlay inside anything that has a transform is fixed to THAT, not
  // to the screen: hang the veil on <body> (same reason as FlashWarning).
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

{#if picker}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div class="overlay" use:toBody on:click={(e) => e.target === e.currentTarget && closePlaylistPicker()}>
    <div
      class="sheet"
      role="dialog"
      aria-modal="true"
      aria-label={mode === "queue" ? "Enregistrer la file en playlist" : mode === "new" ? "Nouvelle playlist" : "Ajouter à une playlist"}
      tabindex="-1"
      bind:this={sheet}
    >
      <header>
        <div class="ttl">
          <h2>
            {#if mode === "queue"}Enregistrer la file en playlist{:else if mode === "new"}Nouvelle playlist{:else}Ajouter à une playlist{/if}
          </h2>
          <p class="sub">
            {#if track}
              {track.title}{artistLine(track) ? " · " + artistLine(track) : ""}
            {:else if mode === "queue"}
              Les titres de la file d'attente, dans l'ordre.
            {:else}
              Tu pourras y ajouter des titres ensuite.
            {/if}
          </p>
        </div>
        <button type="button" class="close" on:click={closePlaylistPicker} aria-label="Fermer">
          <Icon name="close" size={18} />
        </button>
      </header>

      {#if mode === "queue"}
        <p class="queuecount mono">{queueRefs.length} titre{queueRefs.length > 1 ? "s" : ""} dans la file</p>
      {/if}

      {#if mode === "track" && !creating}
        <button type="button" class="newbtn" on:click={openCreating}>
          <Icon name="plus" size={18} />
          Nouvelle playlist
        </button>

        {#if manyPlaylists}
          <label class="searchbox">
            <Icon name="search" size={16} />
            <input bind:value={q} placeholder="Filtrer les playlists…" aria-label="Filtrer les playlists" />
          </label>
        {/if}

        <div class="results">
          {#if shown === null}
            {#if listFailed}
              <p class="state">
                Impossible de charger tes playlists.
                <button type="button" class="btn btn-sm btn-ghost" on:click={() => loadList(true)}>Réessayer</button>
              </p>
            {:else}
              <p class="state">Chargement des playlists…</p>
            {/if}
          {:else if !shown.length}
            <p class="state">
              {#if q.trim()}Aucune playlist ne correspond à « {q.trim()} ».{:else}Aucune playlist pour l'instant.{/if}
            </p>
          {:else}
            {#each visible as p (p.id)}
              <button
                type="button"
                class="row"
                role="checkbox"
                aria-checked={isAdded(p)}
                aria-label={isAdded(p) ? `Ajouté à « ${p.title} »` : `Ajouter à « ${p.title} »`}
                on:click={() => toggle(p)}
                disabled={busy}
              >
                <span class="thumb">
                  <Cover src={p.cover} alt={p.title} size={40} kind="playlist" fallbackId={isLocalId(p.id) ? p.id : null} />
                </span>
                <span class="meta">
                  <span class="name">
                    <span class="nm">{p.title}</span>
                    {#if String(pinned?.id) === String(p.id)}<span class="recent">récente</span>{/if}
                  </span>
                  <span class="count mono">{countOf(p)}</span>
                </span>
                <span class="box" class:on={isAdded(p)} aria-hidden="true">
                  {#if isAdded(p)}<Icon name="check" size={14} />{/if}
                </span>
              </button>
            {/each}
            {#if truncated}
              <p class="state">Filtre pour trouver les autres playlists.</p>
            {/if}
          {/if}
        </div>

        <button type="button" class="btn btn-primary done" on:click={closePlaylistPicker}>Terminé</button>
      {:else}
        <form class="form" on:submit|preventDefault={submit}>
          <label class="label-mono" for="pl-name">Nom de la playlist</label>
          <input
            id="pl-name"
            class="nameinput"
            type="text"
            bind:value={name}
            bind:this={nameEl}
            placeholder={suggestion}
            maxlength="100"
            autocomplete="off"
          />
          <div class="formactions">
            <button type="button" class="btn btn-ghost" on:click={cancel}>Annuler</button>
            <button type="submit" class="btn btn-primary" disabled={busy || (mode === "queue" && !queueRefs.length)}>
              {formTitle}
            </button>
          </div>
        </form>

        {#if mode === "queue" && !queueRefs.length}
          <p class="hint">La file est vide : il n'y a rien à enregistrer.</p>
        {/if}
      {/if}
    </div>
  </div>
{/if}

<style>
  /* The veil is the theme's own ink at 60 % (DESIGN §8): dark in Encre and
     Pochette, a dark warm grey in Papier — never a hardcoded black. */
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 400;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 20px;
    background: color-mix(in srgb, var(--onac) 60%, transparent);
    overscroll-behavior: contain;
    /* A swipe on the veil must not scroll the page behind it; the sheet and its
       list re-enable their own gestures. */
    touch-action: none;
  }
  .sheet {
    width: min(440px, 100%);
    min-width: 0;
    max-width: 100%;
    max-height: min(700px, calc(100vh - 40px));
    max-height: min(700px, calc(100dvh - 40px));
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
  .sheet:focus {
    outline: none;
  }
  header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
    min-width: 0;
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
    overflow-wrap: anywhere;
    /* "title · artist" can be long: two lines carry it, the rest is ellipsis. */
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
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
    color: var(--tx);
    background: color-mix(in srgb, var(--tx) 12%, var(--s2));
  }
  .newbtn {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    height: 52px;
    flex: none;
    border: 1.5px dashed var(--dim);
    border-radius: 14px;
    color: var(--tx);
    font-size: 15px;
    font-weight: 600;
  }
  .newbtn:hover {
    border-color: var(--tx);
    background: var(--s2);
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
  .state button {
    margin-left: 8px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 48px;
    padding: 8px;
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
  .name {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .nm {
    display: block;
    min-width: 0;
    font-weight: 600;
    font-size: 15px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .recent {
    flex: none;
    font-family: var(--font-mono);
    font-size: 10px;
    font-weight: 500;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--dim);
    border: 1px solid var(--ln);
    border-radius: 999px;
    padding: 2px 6px;
  }
  .count {
    font-size: 11px;
    color: var(--dim);
  }
  .box {
    flex: none;
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 7px;
    border: 2px solid var(--dim);
    color: var(--onac);
  }
  .box.on {
    background: var(--ac);
    border-color: var(--ac-text);
  }
  .done {
    width: 100%;
    height: 44px;
    flex: none;
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px;
    border-radius: 16px;
    background: var(--s2);
  }
  .nameinput {
    width: 100%;
    height: 44px;
    box-sizing: border-box;
    padding: 0 14px;
    border-radius: 12px;
    border: 1px solid var(--ln);
    background: var(--bg);
    color: var(--tx);
    font-family: var(--font-body);
    font-size: 15px;
  }
  .nameinput::placeholder {
    color: var(--dim);
    opacity: 1;
  }
  .formactions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 2px;
  }
  .queuecount {
    margin: -6px 0 0;
    font-size: 11px;
    color: var(--dim);
  }
  .hint {
    margin: -4px 0 0;
    font-size: 13px;
    color: var(--dim);
  }

  /* A finger: every hit target is at least 44 px, whatever the width. */
  @media (pointer: coarse) {
    .close {
      width: 44px;
      height: 44px;
    }
    .searchbox {
      height: 44px;
    }
  }

  /* A finger: the sheet rises from the bottom (thumb reach) and every control
     is 44 px. The 440 px card stays for the desktop. */
  @media (max-width: 640px) {
    .overlay {
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
    .row {
      padding: 10px 8px;
    }
  }
</style>
