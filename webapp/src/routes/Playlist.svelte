<script>
  import { trackKey, trackRef } from "../lib/trackkey.js";
  import { onDestroy } from "svelte";
  import { REMOTE, CAN_KEEP, CAN_ADMIN, CAN_BROWSE } from "../lib/remote/mode.js";
  import { push } from "svelte-spa-router";
  import { api } from "../lib/api.js";
  import { player, isAdmin, toasts, lastPlaylist, openExport, openGenreBulk, openMenu } from "../lib/stores.js";
  import {
    toggleEntityFavorite,
    entityMeta,
    invalidatePlaylists,
    upsertPlaylistLocal,
    removePlaylistLocal,
    downloadTracks,
  } from "../lib/actions.js";
  import { duration as fmtDuration, isLocalId } from "../lib/format.js";
  import { carAvailable, carPin } from "../lib/car.js";
  import { SORTS, createProjector } from "../lib/tracklist.js";
  import { reconcilePayload } from "../lib/reconcile.js";
  import Cover from "../components/Cover.svelte";
  import TrackBrowser from "../components/TrackBrowser.svelte";
  import TrackList from "../components/TrackList.svelte";
  import PlaylistTracks from "../components/PlaylistTracks.svelte";
  import AddTracksSheet from "../components/AddTracksSheet.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import LoadFailed from "../components/LoadFailed.svelte";
  import Icon from "../components/Icon.svelte";

  export let params = {};

  let id = null;
  let data = null;
  let fav = false;
  let loading = true;
  let loadError = null; // why there is nothing to show (components/LoadFailed)

  let editing = false; // dedicated edit mode (drag handles + up/down + remove)
  let editingMeta = false; // header rename/description inputs open
  let showAdd = false; // AddTracksSheet open
  let draftTitle = "";
  let draftDesc = "";

  // View sort/search for editable playlists (like the favorites browser). A
  // non-default sort or a search shows a read-only, sorted projection; the
  // reorderable edit mode is only offered in the natural "Ordre d'origine".
  let plSort = "default";
  let plQuery = "";
  let plDir = 1;
  const PL_SORTS = SORTS;
  $: manualView = plSort === "default" && !plQuery.trim();
  // The same index as every other list (lib/tracklist.js).
  const projectTracks = createProjector();
  onDestroy(() => projectTracks.free());
  $: sortedTracks = projectTracks(data?.tracks || [], plSort, plDir, plQuery);

  $: if (params.id && params.id !== id) {
    id = params.id;
    load(id);
  }

  $: editable = !!data?.playlist?.editable && $isAdmin && CAN_KEEP;
  // A sort/search can't coexist with the drag-reorder edit mode.
  $: if (!manualView && editing) editing = false;
  $: existingIds = new Set((data?.tracks || []).map((t) => trackKey(t)));

  // Sequence guard: a delayed earlier response must not overwrite the page
  // after a quick navigation to another playlist.
  let loadSeq = 0;
  async function load(plId) {
    const mine = ++loadSeq;
    queueOrder(); // save any pending reorder of the playlist we're leaving
    loading = true;
    loadError = null;
    data = null;
    editing = false;
    editingMeta = false;
    showAdd = false;
    plSort = "default";
    plQuery = "";
    plDir = 1;
    try {
      // Cached copy first, fresh one right after — a big playlist you've opened
      // before is on screen immediately instead of after a full round-trip. The
      // fresh copy is then RECONCILED into what's on screen (see reconcile.js)
      // rather than replacing it: unchanged rows keep their component and their
      // decoded cover, so the second paint adds and removes exactly what
      // changed and nothing moves under the reader.
      await api.swr("/playlist/" + plId, (r) => {
        if (mine !== loadSeq) return;
        data = reconcilePayload(data, r);
        fav = !!r.playlist?.is_favorite;
        loading = false;
      });
    } catch (e) {
      if (mine === loadSeq && !data) {
        data = null;
        loadError = e;
      }
    }
    if (mine === loadSeq) loading = false;
  }

  function playAll() {
    if (data?.tracks?.length) player.playQueue(data.tracks, 0, { kind: "playlist", id });
  }
  let dlBusy = false;
  async function downloadAll() {
    if (dlBusy || !data?.tracks?.length) return;
    dlBusy = true;
    try {
      await downloadTracks(data.tracks);
    } finally {
      dlBusy = false;
    }
  }
  function shufflePlay() {
    if (data?.tracks?.length) player.shufflePlay(data.tracks, { kind: "playlist", id });
  }

  async function toggleFav() {
    const next = !fav;
    if (await toggleEntityFavorite("playlist", id, next, entityMeta("playlist", data?.playlist || {}))) fav = next;
  }

  // -- editing -------------------------------------------------------------

  function openMeta() {
    draftTitle = data.playlist.title;
    draftDesc = data.playlist.description || "";
    editingMeta = true;
  }

  async function saveMeta() {
    const title = draftTitle.trim();
    if (!title) return;
    editingMeta = false;
    data.playlist.title = title;
    data.playlist.description = draftDesc.trim();
    data = data;
    try {
      await api.editPlaylist(id, { title, description: draftDesc.trim() });
      upsertPlaylistLocal({ id, title }); // rename everywhere, at once
      invalidatePlaylists();
    } catch {
      toasts.push("Échec de l'enregistrement", "error");
    }
  }

  // -- edits: instant UI, serialized background saves -----------------------
  // Every mutation updates the page IMMEDIATELY; the matching API call is
  // pushed onto a FIFO chain, so at most one request is in flight and the
  // server applies operations in exactly the order the user made them (an
  // index-based remove racing a reorder would corrupt the playlist). Reorders
  // are coalesced: a burst of drags / up-down taps collapses into ONE PUT of
  // the latest order. The user never waits — worst case a save fails and the
  // list silently resyncs from the server.
  let chain = Promise.resolve();
  function enqueue(op) {
    chain = chain.then(op, op);
  }

  // Resync from the server after a failed save (source of truth) — without
  // resetting the edit-mode/scroll state the way a full load() would.
  async function resync(plId) {
    try {
      const r = await api.playlist(plId);
      if (r?.playlist && plId === id) {
        data = r;
        fav = !!r.playlist.is_favorite;
      }
    } catch {
      /* offline: the optimistic state stays; next load will settle it */
    }
  }

  let orderTimer = null;
  let pendingOrder = null;
  let orderQueued = false;
  function reorder(newTracks) {
    data.tracks = newTracks;
    data = data;
    // Capture the playlist id with the order: a quick navigation to another
    // playlist swaps `id` before the debounce fires.
    pendingOrder = { plId: id, order: newTracks.map((t) => trackRef(t)) };
    clearTimeout(orderTimer);
    orderTimer = setTimeout(queueOrder, 400);
  }
  function queueOrder() {
    clearTimeout(orderTimer);
    orderTimer = null;
    if (!pendingOrder || orderQueued) return; // the queued op sends the latest
    orderQueued = true;
    enqueue(async () => {
      orderQueued = false;
      if (!pendingOrder) return;
      const { plId, order } = pendingOrder;
      pendingOrder = null;
      try {
        await api.reorderPlaylist(plId, order);
      } catch {
        toasts.push("Échec du réordonnancement", "error");
        await resync(plId);
      }
    });
  }
  onDestroy(queueOrder);

  function removeAt(index) {
    const plId = id;
    data.tracks = data.tracks.filter((_, i) => i !== index);
    data = data;
    // Any pending reorder is queued FIRST: the server resolves the remove
    // index against its own order, which must match what the user saw.
    queueOrder();
    enqueue(async () => {
      try {
        await api.removePlaylistIndexes(plId, [index]);
        invalidatePlaylists(); // sidebar/menu track counts
      } catch {
        toasts.push("Échec de la suppression", "error");
        await resync(plId);
      }
    });
  }

  function onAdded(track) {
    const plId = id;
    data.tracks = [...data.tracks, track];
    data = data;
    enqueue(async () => {
      try {
        await api.addToPlaylist(plId, [trackRef(track)]);
        invalidatePlaylists();
      } catch {
        toasts.push(`Échec de l'ajout de « ${track.title} »`, "error");
        await resync(plId);
      }
    });
  }

  async function deletePlaylist() {
    if (!window.confirm(`Supprimer la playlist « ${data.playlist.title} » ?`)) return;
    try {
      await api.deletePlaylist(id);
      removePlaylistLocal(id); // gone from the sidebar/library before we leave
      invalidatePlaylists();
      // Drop the "add to last playlist" shortcut if it pointed here.
      if ($lastPlaylist?.id === id) lastPlaylist.set(null);
      toasts.push("Playlist supprimée");
      push("/library");
    } catch {
      toasts.push("Échec de la suppression", "error");
    }
  }

  $: total = (data?.tracks || []).reduce((s, t) => s + (t.duration || 0), 0);
  // The facts line, assembled as ONE string: separators written around an
  // {#if} get their surrounding whitespace trimmed by the compiler.
  $: facts = data?.playlist
    ? [
        "Playlist",
        data.playlist.owner,
        `${data.tracks.length} titre${data.tracks.length > 1 ? "s" : ""}`,
        fmtDuration(total),
      ]
        .filter(Boolean)
        .join(" · ")
    : "";

  // Everything that is not the two or three gestures a playlist is for goes
  // behind the ⋯ (DESIGN §6: deleting a playlist is not a bare icon any more).
  // The car entry only exists in an APK that carries the mode (lib/car.js),
  // and never on a remote control: those pins are this device's car, not the
  // driven player's.
  const canPinToCar = !REMOTE && carAvailable();
  function pinToCar() {
    if (!data?.playlist) return;
    if (carPin(id, data.playlist.title))
      toasts.push(`« ${data.playlist.title} » épinglée pour la voiture`);
    else toasts.push("Impossible d'épingler cette playlist", "error");
  }
  $: menuItems = data?.playlist ? playlistMenu() : [];
  function playlistMenu() {
    const items = [];
    if (CAN_KEEP)
      items.push({ label: "Exporter", icon: "archive", action: () => openExport("playlist", id, data.playlist.title) });
    if (canPinToCar)
      items.push({ label: "Épingler pour la voiture", icon: "pin", action: pinToCar });
    if ($isAdmin && CAN_KEEP && data.tracks.length)
      items.push({
        label: "Donner un genre",
        icon: "tag",
        action: () => openGenreBulk({ kind: "playlist", title: data.playlist.title, cover: data.playlist.cover, tracks: data.tracks }),
      });
    if (editable) {
      items.push({ label: "Renommer", icon: "edit", action: openMeta });
      items.push({ label: "Supprimer la playlist", icon: "trash", action: deletePlaylist });
    }
    return items;
  }
  function openActions(e) {
    const r = e.currentTarget.getBoundingClientRect();
    openMenu({ preventDefault() {}, stopPropagation() {}, clientX: r.left, clientY: r.bottom + 4 }, menuItems);
  }
</script>

{#if loading}
  <Skeleton kind="header" />
  <Skeleton kind="list" />
{:else if !data?.playlist}
  <LoadFailed error={loadError} notFound="Playlist introuvable." on:retry={() => load(id)} />
{:else}
  <div class="fade-in">
    <header class="banner gh">
      <div class="art"><Cover src={data.playlist.cover} alt={data.playlist.title} kind="playlist" fallbackId={isLocalId(id) ? id : null} eager /></div>
      <div class="meta">
        {#if editingMeta}
          <input class="title-input" bind:value={draftTitle} placeholder="Nom de la playlist" aria-label="Nom de la playlist" />
          <textarea class="desc-input" bind:value={draftDesc} rows="2" placeholder="Description (optionnel)"></textarea>
          <div class="meta-actions">
            <button class="btn btn-primary btn-sm" on:click={saveMeta}><Icon name="check" size={16} /> Enregistrer</button>
            <button class="btn btn-ghost btn-sm" on:click={() => (editingMeta = false)}>Annuler</button>
          </div>
        {:else}
          <span class="kind">{facts}</span>
          <h1>{data.playlist.title}</h1>
          {#if data.playlist.description}<p class="desc muted">{data.playlist.description}</p>{/if}
        {/if}
      </div>
    </header>

    <div class="actions">
      <button class="btn btn-primary" on:click={playAll}><Icon name="play" size={15} /> Lire</button>
      <button class="btn btn-ghost" on:click={shufflePlay}><Icon name="shuffle" size={16} /> Mélanger</button>
      {#if !REMOTE}
        <button
          class="btn btn-ghost"
          on:click={downloadAll}
          disabled={dlBusy}
          aria-label="Télécharger la playlist"
          title="Télécharger sur l'appareil (hors-ligne)"><Icon name="download" size={16} /> Télécharger</button>
      {/if}
      {#if editable}
        <button class="btn btn-ghost" on:click={() => (showAdd = true)} aria-label="Ajouter des titres" title="Ajouter des titres"><Icon name="plus" size={16} /> Ajouter des titres</button>
        <button
          class="btn"
          class:btn-primary={editing}
          class:btn-ghost={!editing}
          class:on={editing}
          disabled={!manualView}
          title={!manualView ? "Repassez en « Ordre d'origine » pour réorganiser" : ""}
          on:click={() => (editing = !editing)}>
          <Icon name={editing ? "check" : "edit"} size={16} /> {editing ? "Terminé" : "Modifier"}
        </button>
      {:else if CAN_KEEP}
        <button class="btn btn-icon" class:on={fav} on:click={toggleFav} aria-label="Favori"><Icon name={fav ? "heartFilled" : "heart"} size={20} /></button>
      {/if}
      {#if menuItems.length}
        <button class="btn btn-icon" on:click={openActions} aria-label="Plus d'actions : exporter, étiqueter, renommer, supprimer" title="Plus d'actions">
          <Icon name="more" size={20} />
        </button>
      {/if}
    </div>

    {#if editable}
      <!-- Sort/search toolbar (like the favourites browser). -->
      <div class="toolbar">
        <label class="field searchbox">
          <Icon name="search" size={16} />
          <input placeholder="Rechercher dans la playlist…" bind:value={plQuery} aria-label="Rechercher dans la playlist" />
        </label>
        <span class="spacer"></span>
        <span class="selwrap">
          <select class="sortsel" bind:value={plSort} aria-label="Trier par">
            {#each PL_SORTS as s}<option value={s.key}>{s.label}</option>{/each}
          </select>
          <Icon name="chevronDown" size={15} />
        </span>
        <button class="btn btn-icon" class:rev={plDir === -1} on:click={() => (plDir = -plDir)} aria-label="Inverser l'ordre" title="Inverser l'ordre">
          <Icon name="sort" size={17} />
        </button>
      </div>
      {#if editing}
        <!-- Reorderable edit mode (drag + up/down + remove). Only in the manual
             order; the toggle is disabled otherwise. -->
        <PlaylistTracks
          tracks={data.tracks}
          context={{ kind: "playlist", id }}
          {editing}
          onreorder={reorder}
          onremove={removeAt}
        />
      {:else if plQuery.trim() && !sortedTracks.length}
        <p class="muted empty-hint">Aucun titre ne correspond à « {plQuery} ».</p>
      {:else}
        <!-- View mode: the virtualized list — smooth scrolling on huge playlists. -->
        <TrackList tracks={sortedTracks} context={{ kind: "playlist", id }} numbered={true} headers />
      {/if}
    {:else}
      <TrackBrowser tracks={data.tracks} context={{ kind: "playlist", id }} />
    {/if}
  </div>

  {#if showAdd}
    <AddTracksSheet
      {existingIds}
      onadd={onAdded}
      onclose={() => (showAdd = false)}
    />
  {/if}
{/if}

<style>
  /* §6: playlist header — 200 px sleeve, title at 72, the facts in mono. */
  .banner {
    display: flex;
    align-items: flex-end;
    gap: 28px;
    margin: calc(-1 * var(--page-y)) calc(-1 * var(--page-x)) 20px;
    padding: 40px var(--page-x) 28px;
    background: var(--s2);
    border-bottom: 1px solid var(--ln);
  }
  .art {
    width: 200px;
    flex: none;
    border-radius: var(--r-cover);
    overflow: hidden;
  }
  .meta {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
    max-width: 62ch;
  }
  .kind {
    font-family: var(--font-mono);
    font-size: 12px;
    letter-spacing: 0.04em;
    color: var(--dim);
  }
  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-weight: 800;
    text-wrap: balance;
    font-size: clamp(34px, 6vw, 72px);
    line-height: 0.95;
    letter-spacing: -0.04em;
    overflow-wrap: anywhere;
  }
  .desc {
    margin: 0;
    font-size: 14px;
    line-height: 1.45;
  }
  .title-input {
    font-family: var(--font-display);
    font-size: clamp(26px, 4vw, 44px);
    font-weight: 800;
    letter-spacing: -0.03em;
    background: var(--s1);
    border: 1px solid var(--ln);
    border-radius: 10px;
    color: var(--tx);
    padding: 6px 10px;
    outline: none;
    max-width: 100%;
  }
  .desc-input {
    max-width: 100%;
    background: var(--s1);
    border: 1px solid var(--ln);
    border-radius: 10px;
    color: var(--tx);
    padding: 8px 10px;
    outline: none;
    resize: vertical;
    font: inherit;
  }
  .title-input:focus,
  .desc-input:focus {
    border-color: var(--ac-text);
  }
  .meta-actions {
    display: flex;
    gap: 8px;
    margin-top: 2px;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    margin: 0 0 16px;
  }
  .btn-icon.on {
    color: var(--hrt);
    border-color: var(--hrt);
  }
  .btn.on {
    color: var(--onac);
    background: var(--ac);
    border-color: transparent;
  }
  /* Sort/search toolbar for the editable playlist view (mirrors TrackBrowser). */
  .toolbar {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 6px 0 12px;
  }
  .searchbox {
    width: 260px;
    flex: 0 1 260px;
    min-width: 160px;
  }
  .searchbox input {
    border: none;
    background: none;
    outline: none;
    color: var(--tx);
    font: inherit;
    width: 100%;
    min-width: 0;
    padding: 0;
  }
  .searchbox:focus-within {
    border-color: var(--ac-text);
    color: var(--tx);
  }
  .toolbar .spacer {
    flex: 1;
  }
  .selwrap {
    position: relative;
    display: inline-flex;
    align-items: center;
  }
  .selwrap :global(svg) {
    position: absolute;
    right: 12px;
    color: var(--dim);
    pointer-events: none;
  }
  .sortsel {
    height: 40px;
    background: var(--s1);
    color: var(--tx);
    border: 1px solid var(--ln);
    border-radius: 999px;
    padding: 0 36px 0 14px;
    font: inherit;
    font-size: 13px;
    outline: none;
    cursor: pointer;
    appearance: none;
    -webkit-appearance: none;
  }
  .btn-icon.rev {
    color: var(--ac-text);
    border-color: var(--ac-text);
  }
  .empty-hint {
    margin-top: 18px;
  }
  @media (max-width: 640px) {
    /* The action pills scroll on ONE line instead of stacking three rows of
       chrome between the header and the list. */
    .actions {
      flex-wrap: wrap;
      overflow: visible;
      padding-bottom: 2px;
    }
    /* Spotify's phone header: the sleeve and the facts share one short band,
       so the tracklist is not pushed a screen and a half down. */
    .banner {
      flex-direction: row;
      align-items: center;
      gap: 14px;
      padding: 16px var(--page-x) 16px;
    }
    .art {
      width: 96px;
    }
    .meta {
      gap: 6px;
    }
    h1 {
      /* Same rule as the album's band (§9): 32–40 under a finger, not 25–27. */
      font-size: clamp(32px, 8.5vw, 40px);
    }
    .kind {
      line-height: 1.5;
    }
    /* Same as the favourites' toolbar: the search on a line of its own. Beside
       the sort it was squeezed to a 90 px box showing "Rech". */
    .toolbar {
      flex-wrap: wrap;
    }
    .searchbox {
      flex: 1 1 100%;
      max-width: none;
      width: auto;
    }
    .selwrap {
      flex: 1 1 auto;
      min-width: 0;
    }
    .selwrap .sortsel {
      width: 100%;
      min-width: 0;
    }
    .toolbar .spacer {
      display: none;
    }
  }
</style>
