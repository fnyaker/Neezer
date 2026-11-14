<script context="module">
  // The bottom of a playlist asks the server for suggestions, which are the
  // library's own answer (no provider, see neezer/webui/playlistrecs.py).
  // Keeping them in module memory means coming back to a playlist paints its
  // suggestions with the playlist, with no second request; an add or a remove
  // drops the entry so the next open re-reads them.
  const recsCache = new Map(); // playlist id -> tracks
  const recsInflight = new Map(); // playlist id -> promise (one request per open)
  // A playlist whose suggestions changed while they were being fetched must
  // not have that older answer cached: the next open asks again.
  const recsStale = new Set();
  function forgetRecommendations(id) {
    const key = String(id);
    recsCache.delete(key);
    recsStale.add(key);
  }
</script>

<script>
  import { trackKey, trackRef } from "../lib/trackkey.js";
  import { onDestroy } from "svelte";
  import { REMOTE, CAN_KEEP, CAN_ADMIN, CAN_BROWSE } from "../lib/remote/mode.js";
  import { push } from "svelte-spa-router";
  import { api } from "../lib/api.js";
  import {
    player,
    isAdmin,
    user,
    toasts,
    lastPlaylist,
    openExport,
    openGenreBulk,
    openMenu,
    playlistFolders,
    playlists,
    openMoveToFolder,
  } from "../lib/stores.js";
  import {
    toggleEntityFavorite,
    entityMeta,
    invalidatePlaylists,
    upsertPlaylistLocal,
    removePlaylistLocal,
    downloadTracks,
    movePlaylistTo,
    conflictOf,
    conflictNotice,
    resyncOnce,
    probeDecision,
  } from "../lib/actions.js";
  import { duration as fmtDuration, artistLine, isLocalId } from "../lib/format.js";
  import { carAvailable, carPin } from "../lib/car.js";
  import { SORTS, createProjector } from "../lib/tracklist.js";
  import { reconcilePayload } from "../lib/reconcile.js";
  import { grooveTint, initialsOf } from "../lib/sillon.js";
  import Cover from "../components/Cover.svelte";
  import TrackBrowser from "../components/TrackBrowser.svelte";
  import TrackList from "../components/TrackList.svelte";
  import PlaylistTracks from "../components/PlaylistTracks.svelte";
  import AddTracksSheet from "../components/AddTracksSheet.svelte";
  import PlaylistMembersSheet from "../components/PlaylistMembersSheet.svelte";
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

  $: editable = !!data?.playlist?.editable && CAN_KEEP;
  $: canManage = !!data?.playlist?.can_manage && CAN_KEEP;
  // The caller's own role as the server named it (`owner | editor | viewer |
  // admin | public`), and whether this is a playlist somebody ELSE is on too —
  // the only case worth a live probe: a solo list cannot move under us.
  $: role = data?.playlist?.role || null;
  $: members = data?.playlist?.members || null;
  $: memberCount = members ? members.length : 0;
  $: collaborative = !!members && memberCount > 1;
  // A sort/search can't coexist with the drag-reorder edit mode.
  $: if (!manualView && editing) editing = false;
  $: existingIds = new Set((data?.tracks || []).map((t) => trackKey(t)));

  // -- collaboration: version, conflict machine, live probe -----------------
  // Every order-sensitive mutation carries `version` (content) or
  // `members_version`; a 409 drops the queue, makes ONE GET by scope and never
  // replays. The player's own queue is never touched here.
  let ready = false;
  let readyResolve = null;
  let readyPromise = Promise.resolve();
  let epoch = 0;
  let inflight = 0;
  let stale = null; // { by, at } — the probe saw a change we have not loaded
  let gone = false; // the playlist stopped being accessible while we were on it
  let showMembers = false;

  function beginReady() {
    ready = false;
    readyPromise = new Promise((resolve) => (readyResolve = resolve));
  }
  function settleReady() {
    if (!readyResolve) {
      ready = true;
      return;
    }
    ready = true;
    const resolve = readyResolve;
    readyResolve = null;
    resolve();
  }

  // One request at a time, in the person's order; an op carries the epoch it
  // was built against, so a conflict drops what no longer applies.
  let chain = Promise.resolve();
  function enqueue(op) {
    const mine = epoch;
    const readyAt = readyPromise;
    const run = async () => {
      await readyAt;
      if (mine !== epoch) return false;
      inflight++;
      try {
        return await op();
      } finally {
        inflight--;
      }
    };
    chain = chain.then(run, run);
    return chain;
  }

  function dropQueue() {
    epoch++;
    clearTimeout(orderTimer);
    orderTimer = null;
    pendingOrder = null;
    orderQueued = false;
  }

  function applyPayload(r) {
    if (!r?.playlist) return;
    data = reconcilePayload(data, r);
    fav = !!r.playlist.is_favorite;
    stale = null;
  }

  let freshInFlight = null;
  function pullFresh() {
    const plId = id;
    if (freshInFlight) return freshInFlight;
    const p = api
      .playlist(plId)
      .then((r) => {
        if (plId === id) applyPayload(r);
        return r;
      })
      .catch((e) => {
        if (e?.status === 404) markGone(plId);
        throw e;
      })
      .finally(() => {
        if (freshInFlight === p) freshInFlight = null;
      });
    freshInFlight = p;
    return p;
  }

  // Who is on it and what the caller may do, without the 4 000-track list.
  function pullMembers(announce = false) {
    const plId = id;
    return api.playlistMembers(plId).then((r) => {
      if (plId !== id || !data?.playlist) return r;
      const couldEdit = !!data.playlist.editable;
      data.playlist.members = r.members || [];
      data.playlist.members_version = r.members_version;
      data.playlist.role = r.role;
      data.playlist.admin = !!r.admin;
      data.playlist.editable = !!r.can_edit;
      data.playlist.can_manage = !!r.can_manage;
      data = data;
      if (couldEdit && !r.can_edit) {
        editing = false;
        editingMeta = false;
        if (announce)
          toasts.push("Vous n'avez plus le droit de modifier cette playlist.", "error", 6000);
      }
      return r;
    });
  }

  function markGone(plId) {
    if (plId != null && String(plId) !== String(id)) return;
    removePlaylistLocal(id);
    invalidatePlaylists();
    // The access went away under an open page (not a fresh 404).
    if (!data?.playlist || gone) return;
    gone = true;
    stopProbe();
    dropQueue();
    stale = null;
    toasts.push("Cette playlist ne vous est plus accessible.", "info", 6000);
    push("/library");
  }

  function onConflict(err, plId) {
    const body = conflictOf(err);
    if (!body) return false;
    if (String(plId) !== String(id)) return false;
    dropQueue();
    stale = null;
    const scope = body.scope === "members" ? "members" : "content";
    const reload = scope === "members" ? () => pullMembers(true) : () => pullFresh();
    resyncOnce(`${id}:${scope}`, reload).catch((e) => {
      if (e?.status === 404) markGone(id);
    });
    toasts.push(conflictNotice(body), "info", 6000);
    return true;
  }

  function onForbidden(err, plId, rollback = null) {
    if (!err || err.status !== 403) return false;
    if (String(plId) !== String(id)) return false;
    dropQueue();
    editing = false;
    editingMeta = false;
    rollback?.();
    resyncOnce(`${id}:members`, () => pullMembers()).catch(() => {});
    toasts.push("Vous n'avez plus le droit de modifier cette playlist.", "error", 6000);
    return true;
  }

  function onNetworkError(err, plId, rollback) {
    if (!err || !(err.offline || err.status === 0)) return false;
    if (String(plId) !== String(id)) return false;
    dropQueue();
    rollback?.();
    toasts.push("Hors ligne : la modification n'a pas été enregistrée.", "error", 6000);
    return true;
  }

  function failed(err, plId, message, rollback = null) {
    if (onConflict(err, plId)) return false;
    if (onForbidden(err, plId, rollback)) return false;
    if (err?.status === 404) {
      markGone(plId);
      return false;
    }
    if (onNetworkError(err, plId, rollback)) return false;
    if (String(plId) === String(id)) toasts.push(message, "error");
    return false;
  }

  // The next queued op must carry the version the server just returned.
  function adoptVersion(r) {
    if (!data?.playlist || !r) return;
    if (r.version != null) {
      data.playlist.version = r.version;
      data.playlist.last_edited_by = $user?.name || data.playlist.last_edited_by;
      data.playlist.last_edited_at = new Date().toISOString();
    }
    if (r.members_version != null) data.playlist.members_version = r.members_version;
    if (r.nb_tracks != null) data.playlist.nb_tracks = r.nb_tracks;
    data = data;
  }

  // Two counters (~200 B), only for a collaborative playlist and never while
  // the page is hidden.
  const PROBE_EVERY = 30000;
  const PROBE_FOCUS_AGE = 10000;
  let probeTimer = null;
  let lastProbeAt = 0;
  let probing = false;

  $: if (data?.playlist && collaborative && !gone) startProbe();
  else stopProbe();

  function startProbe() {
    if (probeTimer || typeof window === "undefined") return;
    lastProbeAt = Date.now();
    probeTimer = setTimeout(probeTick, PROBE_EVERY);
    window.addEventListener("focus", probeOnFocus);
    document.addEventListener("visibilitychange", probeOnVisibility);
  }
  function stopProbe() {
    if (probeTimer) clearTimeout(probeTimer);
    probeTimer = null;
    if (typeof window === "undefined") return;
    window.removeEventListener("focus", probeOnFocus);
    document.removeEventListener("visibilitychange", probeOnVisibility);
  }
  // Re-armed after each answer, so a focus probe is followed 30 s later.
  function scheduleProbe(delay = PROBE_EVERY) {
    if (probeTimer == null) return;
    clearTimeout(probeTimer);
    probeTimer = setTimeout(probeTick, delay);
  }
  function probeOnFocus() {
    if (!document.hidden) probe(PROBE_FOCUS_AGE);
  }
  function probeOnVisibility() {
    if (!document.hidden) probe(PROBE_FOCUS_AGE);
  }
  async function probeTick() {
    if (probeTimer == null) return;
    // Never under a save in flight or a drag about to be sent.
    if (document.hidden || inflight > 0 || pendingOrder || orderQueued) {
      scheduleProbe();
      return;
    }
    if (!(await probe(PROBE_EVERY - 1000))) scheduleProbe();
  }
  async function probe(minAge) {
    if (gone || !collaborative || probing || id == null) return false;
    if (Date.now() - lastProbeAt < minAge) return false;
    probing = true;
    const plId = id;
    try {
      const r = await api.playlistVersion(plId);
      lastProbeAt = Date.now();
      readProbe(r);
      return true;
    } catch (e) {
      lastProbeAt = Date.now();
      if (e?.status === 404) markGone(plId);
      return true;
    } finally {
      probing = false;
      scheduleProbe();
    }
  }
  function readProbe(r) {
    const p = data?.playlist;
    if (!p || !r) return;
    const decision = probeDecision(p, r, {
      editing,
      busy: inflight > 0 || !!pendingOrder || orderQueued,
    });
    if (decision.members)
      resyncOnce(`${id}:members`, () => pullMembers(true)).catch((e) => {
        if (e?.status === 404) markGone(id);
      });
    // Take the truth now, or offer the pill while the list is being edited.
    if (decision.reload) pullFresh().catch(() => {});
    else if (decision.stale) stale = { by: r.last_edited_by || "", at: r.last_edited_at || null };
  }
  function refreshStale() {
    if (!stale || inflight > 0) return;
    pullFresh().catch(() => {});
  }

  // -- recommendations at the bottom ----------------------------------------
  // Read-only suggestions from the playlist's own titles. Loaded AFTER the
  // playlist is on screen (the page never waits for them), one request per
  // playlist open, and hidden in the two modes where the list below is not the
  // playlist's own order (edit mode, search) and for a read-only/remote account.
  let recs = [];
  let recsState = "idle"; // idle | loading | ready | failed
  let recsFor = null; // the playlist the rows belong to
  let recBusy = false; // "Tout ajouter" in flight
  let recPending = new Set(); // track keys whose own add is in flight
  $: showRecommendations = editable && !editing && !plQuery.trim();

  // Sequence guard: a delayed earlier response must not overwrite the page
  // after a quick navigation to another playlist.
  let loadSeq = 0;
  async function load(plId) {
    const mine = ++loadSeq;
    queueOrder(); // save any pending reorder of the playlist we're leaving
    beginReady();
    loading = true;
    loadError = null;
    data = null;
    gone = false;
    stale = null;
    showMembers = false;
    editing = false;
    editingMeta = false;
    showAdd = false;
    plSort = "default";
    plQuery = "";
    plDir = 1;
    recs = [];
    recsFor = null;
    recsState = "idle";
    recPending = new Set();
    try {
      // Cached copy first, fresh one right after — a big playlist you've opened
      // before is on screen immediately instead of after a full round-trip. The
      // fresh copy is then RECONCILED into what's on screen (see reconcile.js)
      // rather than replacing it: unchanged rows keep their component and their
      // decoded cover, so the second paint adds and removes exactly what
      // changed and nothing moves under the reader.
      await api.swr("/playlist/" + plId, (r, isStale) => {
        if (mine !== loadSeq) return;
        data = reconcilePayload(data, r);
        fav = !!r.playlist?.is_favorite;
        loading = false;
        // The first edit waits for the fresh copy, never the cached one.
        if (!isStale) settleReady();
        // The playlist is on screen; only now does the bottom section ask.
        if (r.playlist?.editable && CAN_KEEP) loadRecommendations(plId);
      });
    } catch (e) {
      if (mine === loadSeq) {
        settleReady();
        if (!data) {
          data = null;
          loadError = e;
        }
      }
    }
    if (mine === loadSeq) {
      loading = false;
      settleReady();
    }
  }

  async function loadRecommendations(plId, { force = false } = {}) {
    const key = String(plId);
    if (force) recsCache.delete(key);
    else if (recsCache.has(key)) {
      recs = recsCache.get(key);
      recsFor = key;
      recsState = "ready";
      return;
    }
    recsStale.delete(key); // this answer will be the fresh one
    if (recsInflight.has(key)) {
      // Already on its way (a quick return to the same playlist): let its own
      // answer paint when it lands.
      if (recsFor !== key) {
        recs = [];
        recsFor = key;
        recsState = "loading";
      }
      return;
    }
    if (recsFor !== key) {
      recs = [];
      recsFor = key;
    }
    recsState = "loading";
    const p = api
      .playlistRecommendations(key, 12)
      .then((r) => {
        const list = (r?.tracks || []).filter((t) => t && !existingIds.has(trackKey(t)));
        if (!recsStale.has(key)) recsCache.set(key, list);
        if (recsFor === key) {
          recs = list;
          recsState = "ready";
        }
      })
      .catch(() => {
        if (recsFor === key) {
          recsState = "failed";
        }
      })
      .finally(() => recsInflight.delete(key));
    recsInflight.set(key, p);
  }

  // Optimistic, like onAdded itself: the row leaves the suggestions at once
  // (the playlist below already shows the track) and comes back at its place
  // if the save failed, which onAdded reports and resyncs.
  function retryRecommendations() {
    if (id != null) loadRecommendations(id, { force: true });
  }

  async function addRecommendation(track) {
    const key = trackKey(track);
    if (recBusy || recPending.has(key) || id == null) return;
    const plId = id;
    recPending = new Set(recPending).add(key);
    const at = recs.findIndex((t) => trackKey(t) === key);
    recs = recs.filter((t) => trackKey(t) !== key);
    forgetRecommendations(plId);
    const ok = await onAdded(track);
    if (ok && plId === id) toasts.push(`« ${track.title} » ajouté à la playlist`, "info", 2400);
    if (!ok && plId === id) {
      const restored = recs.filter((t) => trackKey(t) !== key);
      restored.splice(Math.min(Math.max(at, 0), restored.length), 0, track);
      recs = restored;
    }
    const next = new Set(recPending);
    next.delete(key);
    recPending = next;
  }

  // "Tout ajouter": the visible suggestions, in their order, one request. Never
  // more than what is on screen — the list is what the person is agreeing to.
  async function addAllRecommendations() {
    if (recBusy || !recs.length || id == null || !data?.playlist) return;
    const plId = id;
    const pl = data.playlist;
    const batch = recs.slice();
    recBusy = true;
    data.tracks = [...data.tracks, ...batch];
    data = data;
    recs = [];
    forgetRecommendations(plId);
    const ok = await enqueue(async () => {
      try {
        const r = await api.addToPlaylist(plId, batch.map(trackRef), pl.version ?? null);
        if (plId === id) adoptVersion(r);
        invalidatePlaylists();
        return true;
      } catch (e) {
        return failed(e, plId, "Échec de l'ajout des suggestions", () => {
          const gone = new Set(batch);
          data.tracks = data.tracks.filter((t) => !gone.has(t));
          data = data;
        });
      }
    });
    recBusy = false;
    if (ok) {
      toasts.push(
        batch.length > 1 ? `${batch.length} titres ajoutés à la playlist` : "Titre ajouté à la playlist",
        "info",
        2400,
      );
    } else if (plId === id) {
      // The failed batch may have landed partly: ask the server what is left
      // rather than trust the optimistic list.
      loadRecommendations(plId, { force: true });
    }
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
    if (!title || !data?.playlist) return;
    const plId = id;
    const pl = data.playlist;
    const description = draftDesc.trim();
    const previous = { title: pl.title, description: pl.description };
    editingMeta = false;
    pl.title = title;
    pl.description = description;
    data = data;
    await enqueue(async () => {
      if (plId !== id) return false;
      try {
        const r = await api.editPlaylist(plId, { title, description }, pl.version ?? null);
        adoptVersion(r);
        upsertPlaylistLocal({ id: plId, title }); // rename everywhere, at once
        invalidatePlaylists();
        return true;
      } catch (e) {
        const message =
          e?.status === 400
            ? "Nom ou description invalide (256 caractères au plus)."
            : "Échec de l'enregistrement";
        return failed(e, plId, message, () => {
          pl.title = previous.title;
          pl.description = previous.description;
          data = data;
        });
      }
    });
  }

  // -- edits: instant UI, serialized background saves -----------------------
  // Every mutation updates the page IMMEDIATELY; the matching API call is
  // pushed onto a FIFO chain (defined above), so at most one request is in
  // flight and the server applies operations in exactly the order the user made
  // them (an index-based remove racing a reorder would corrupt the playlist).
  // Each op carries the version it was built against; reorders are coalesced:
  // a burst of drags / up-down taps collapses into ONE PUT of the latest order.

  let orderTimer = null;
  let pendingOrder = null;
  let orderQueued = false;
  function reorder(newTracks) {
    if (!editable || !data?.playlist) return;
    data.tracks = newTracks;
    data = data;
    // Capture the playlist id and the payload object with the order: a quick
    // navigation swaps `data` before the debounce fires.
    pendingOrder = { plId: id, pl: data.playlist, order: newTracks.map((t) => trackRef(t)) };
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
      if (!pendingOrder) return false;
      const { plId, pl, order } = pendingOrder;
      pendingOrder = null;
      try {
        const r = await api.reorderPlaylist(plId, order, pl?.version ?? null);
        if (plId === id) adoptVersion(r);
        return true;
      } catch (e) {
        // Never replayed: the person sees the server's order and drags again.
        return failed(e, plId, "Échec du réordonnancement");
      }
    });
  }
  onDestroy(queueOrder);
  onDestroy(stopProbe);

  function toggleEditing() {
    if (!editable) return;
    editing = !editing;
    // Leaving edit mode is the moment the person is done choosing an order:
    // if a co-editor moved the list meanwhile, load it now.
    if (!editing && stale) pullFresh().catch(() => {});
  }

  function removeAt(index) {
    if (!editable || !data?.playlist) return;
    const plId = id;
    const pl = data.playlist;
    const removed = data.tracks[index];
    data.tracks = data.tracks.filter((_, i) => i !== index);
    data = data;
    forgetRecommendations(plId); // the suggestions were computed without it
    // Any pending reorder is queued FIRST: the server resolves the remove
    // index against its own order, which must match what the user saw.
    queueOrder();
    enqueue(async () => {
      try {
        const r = await api.removePlaylistIndexes(plId, [index], pl.version ?? null);
        if (plId === id) adoptVersion(r);
        invalidatePlaylists(); // sidebar/menu track counts
        return true;
      } catch (e) {
        return failed(e, plId, "Échec de la suppression", () => {
          if (!removed || !data?.tracks) return;
          const at = Math.min(index, data.tracks.length);
          data.tracks = [...data.tracks.slice(0, at), removed, ...data.tracks.slice(at)];
          data = data;
        });
      }
    });
  }

  // Returns a promise of whether the save went through, so the suggestions can
  // put a row back (and only say "added" once it really is).
  function onAdded(track) {
    if (!editable || !data?.playlist) return Promise.resolve(false);
    const plId = id;
    const pl = data.playlist;
    data.tracks = [...data.tracks, track];
    data = data;
    return enqueue(async () => {
      try {
        const r = await api.addToPlaylist(plId, [trackRef(track)], pl.version ?? null);
        if (plId === id) adoptVersion(r);
        invalidatePlaylists();
        forgetRecommendations(plId);
        return true;
      } catch (e) {
        return failed(e, plId, `Échec de l'ajout de « ${track.title} »`, () => {
          const at = data?.tracks ? data.tracks.indexOf(track) : -1;
          if (at < 0) return;
          data.tracks = [...data.tracks.slice(0, at), ...data.tracks.slice(at + 1)];
          data = data;
        });
      }
    });
  }

  async function deletePlaylist() {
    if (!data?.playlist) return;
    if (!window.confirm(`Supprimer la playlist « ${data.playlist.title} » ?`)) return;
    const plId = id;
    try {
      await api.deletePlaylist(plId);
      removePlaylistLocal(plId); // gone from the sidebar/library before we leave
      invalidatePlaylists();
      // Drop the "add to last playlist" shortcut if it pointed here.
      if ($lastPlaylist?.id === plId) lastPlaylist.set(null);
      toasts.push("Playlist supprimée");
      push("/library");
    } catch (e) {
      if (e?.status === 404) {
        removePlaylistLocal(plId);
        invalidatePlaylists();
        push("/library");
        return;
      }
      failed(e, plId, "Échec de la suppression");
    }
  }

  async function leavePlaylist() {
    if (!data?.playlist) return;
    if (!window.confirm(`Quitter la playlist « ${data.playlist.title} » ?`)) return;
    const plId = id;
    try {
      await api.leavePlaylist(plId);
      onLeft();
    } catch (e) {
      if (e?.status === 404) {
        removePlaylistLocal(plId);
        invalidatePlaylists();
        push("/library");
        return;
      }
      if (e?.status === 409) {
        toasts.push("Le propriétaire ne peut pas quitter sa playlist.", "error");
        return;
      }
      failed(e, plId, "Impossible de quitter la playlist");
    }
  }

  function onLeft() {
    showMembers = false;
    removePlaylistLocal(id);
    invalidatePlaylists();
    toasts.push("Vous avez quitté la playlist", "info", 2400);
    push("/library");
  }

  function onMembersChanged(r) {
    if (!data?.playlist || !r) return;
    if (r.members) data.playlist.members = r.members;
    if (r.members_version != null) data.playlist.members_version = r.members_version;
    // The sheet's own 403 path refreshes what the page may do, so a demotion
    // takes effect without a reload.
    if (r.role != null) data.playlist.role = r.role;
    if (r.admin != null) data.playlist.admin = !!r.admin;
    if (r.can_edit != null) {
      data.playlist.editable = !!r.can_edit;
      if (!r.can_edit) {
        editing = false;
        editingMeta = false;
      }
    }
    if (r.can_manage != null) data.playlist.can_manage = !!r.can_manage;
    data = data;
  }

  $: total = (data?.tracks || []).reduce((s, t) => s + (t.duration || 0), 0);
  $: myEntry = ($playlists || []).find((p) => String(p.id) === String(id)) || null;
  $: myFolder = myEntry?.folder_id
    ? ($playlistFolders || []).find((f) => String(f.id) === String(myEntry.folder_id)) || null
    : null;
  // Who is on the playlist: the owner first (the server orders it that way),
  // three faces at most, then a count.
  $: stack = (members || []).slice(0, 3);
  $: extraFaces = Math.max(0, memberCount - stack.length);
  // The facts line, assembled as ONE string: separators written around an
  // {#if} get their surrounding whitespace trimmed by the compiler.
  $: facts = data?.playlist
    ? [
        "Playlist",
        data.playlist.owner,
        `${data.tracks.length} titre${data.tracks.length > 1 ? "s" : ""}`,
        memberCount > 1 ? `${memberCount} collaborateurs` : null,
        fmtDuration(total),
        !editable && (role === "viewer" || role === "public") ? "Lecture seule" : null,
      ]
        .filter(Boolean)
        .join(" · ")
    : "";
  $: editedBy = data?.playlist?.last_edited_by || "";
  $: editedWhen = data?.playlist?.last_edited_at ? editMoment(data.playlist.last_edited_at) : "";
  $: editedTitle = editedBy
    ? `Modifié par ${editedBy}${editedWhen ? " · " + editedWhen : ""}`
    : "";

  function editMoment(iso) {
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return "";
    const time = d.toLocaleTimeString("fr-FR", { hour: "2-digit", minute: "2-digit" });
    const today = new Date();
    if (d.toDateString() === today.toDateString()) return `aujourd'hui à ${time}`;
    return `${d.toLocaleDateString("fr-FR", { day: "numeric", month: "short" })} à ${time}`;
  }

  // Everything that is not the two or three gestures a playlist is for goes
  // behind the ⋯ (DESIGN §6: deleting a playlist is not a bare icon any more).
  // The car entry only exists in an APK that carries the mode (lib/car.js),
  // and never on a remote control: those pins are this device's car, not the
  // driven player's.
  const canPinToCar = !REMOTE && carAvailable();
  function pinToCar() {
    if (!data?.playlist) return;
    // The count is already in the payload: the native tile can name it before
    // the first catalogue read (lib/car.js falls back to the two-argument
    // bridge call when the page does not know it).
    if (carPin(id, data.playlist.title, data.playlist.nb_tracks))
      toasts.push(`« ${data.playlist.title} » épinglée pour la voiture`);
    else toasts.push("Impossible d'épingler cette playlist", "error");
  }
  $: menuItems = data?.playlist ? playlistMenu() : [];
  function playlistMenu() {
    const items = [];
    if (CAN_KEEP)
      items.push({
        label: "Exporter",
        icon: "archive",
        action: () => openExport("playlist", id, data.playlist.title, { canManage }),
      });
    if (CAN_KEEP && myEntry) {
      items.push({
        label: "Déplacer vers un dossier…",
        icon: "folderMove",
        action: () =>
          openMoveToFolder({
            playlist: {
              id,
              title: data.playlist.title,
              deezer_id: data.playlist.deezer_id,
              folder_id: myEntry.folder_id ?? null,
            },
          }),
      });
      if (myEntry.folder_id)
        items.push({
          label: "Retirer du dossier",
          icon: "folderMinus",
          action: () => movePlaylistTo(id, null),
        });
    }
    if (canPinToCar)
      items.push({ label: "Épingler pour la voiture", icon: "pin", action: pinToCar });
    if ($isAdmin && CAN_KEEP && data.tracks.length)
      items.push({
        label: "Donner un genre",
        icon: "tag",
        action: () => openGenreBulk({ kind: "playlist", title: data.playlist.title, cover: data.playlist.cover, tracks: data.tracks }),
      });
    // Membership: every member may see who is on it; only the owner/admin gets
    // controls (the sheet hides them, and the server checks again).
    if (data.playlist.members)
      items.push({ label: "Collaborateurs", icon: "users", action: () => (showMembers = true) });
    if (canManage) {
      items.push({ label: "Renommer", icon: "edit", action: openMeta });
      items.push({ label: "Supprimer la playlist", icon: "trash", action: deletePlaylist });
    }
    if (!REMOTE && (role === "editor" || role === "viewer"))
      items.push({ label: "Quitter la playlist", icon: "logOut", action: leavePlaylist });
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
          <input class="title-input" bind:value={draftTitle} maxlength="256" placeholder="Nom de la playlist" aria-label="Nom de la playlist" />
          <textarea class="desc-input" bind:value={draftDesc} maxlength="256" rows="2" placeholder="Description (optionnel)"></textarea>
          <div class="meta-actions">
            <button class="btn btn-primary btn-sm" on:click={saveMeta}><Icon name="check" size={16} /> Enregistrer</button>
            <button class="btn btn-ghost btn-sm" on:click={() => (editingMeta = false)}>Annuler</button>
          </div>
        {:else}
          <span class="kind">{facts}</span>
          <h1>{data.playlist.title}</h1>
          {#if data.playlist.description}<p class="desc muted">{data.playlist.description}</p>{/if}
          {#if memberCount > 1}
            <button
              class="members-stack"
              on:click={() => (showMembers = true)}
              aria-label={`Voir les collaborateurs (${memberCount})`}
              title="Voir les collaborateurs"
            >
              <span class="faces" aria-hidden="true">
                {#each stack as m (m.id || m.name)}
                  <span class="face" style={`background-image:${grooveTint(m.name)}`}>{initialsOf(m.name)}</span>
                {/each}
                {#if extraFaces}<span class="face more mono">+{extraFaces}</span>{/if}
              </span>
            </button>
          {/if}
          {#if editedBy}
            <span class="edited mono" title={editedTitle}>
              Modifié par {editedBy}<span class="when">{editedWhen ? ` · ${editedWhen}` : ""}</span>
            </span>
          {/if}
          {#if myFolder}
            <a class="folder-link" href="#/library/playlists">Dossier · {myFolder.title}</a>
          {/if}
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
        <button
          class="btn btn-ghost"
          on:click={() => (showAdd = true)}
          disabled={!ready}
          aria-label="Ajouter des titres"
          title="Ajouter des titres"><Icon name="plus" size={16} /> Ajouter des titres</button>
        <button
          class="btn"
          class:btn-primary={editing}
          class:btn-ghost={!editing}
          class:on={editing}
          disabled={!manualView || !ready}
          title={!manualView ? "Repassez en « Ordre d'origine » pour réorganiser" : ""}
          on:click={toggleEditing}>
          <Icon name={editing ? "check" : "edit"} size={16} /> {editing ? "Terminé" : "Modifier"}
        </button>
      {:else if CAN_KEEP}
        <button class="btn btn-icon" class:on={fav} on:click={toggleFav} aria-label="Favori"><Icon name={fav ? "heartFilled" : "heart"} size={20} /></button>
      {/if}
      {#if stale}
        <button
          class="btn btn-ghost stale"
          on:click={refreshStale}
          title="Recharger la playlist"
        >
          <Icon name="refresh" size={16} />
          {stale.by ? `Modifiée par ${stale.by} — actualiser` : "La playlist a changé — actualiser"}
        </button>
      {/if}
      {#if menuItems.length}
        <button class="btn btn-icon" on:click={openActions} aria-label="Plus d'actions : exporter, collaborateurs, renommer, supprimer" title="Plus d'actions">
          <Icon name="more" size={20} />
        </button>
      {/if}
    </div>

    {#if role === "viewer" && !editable}
      <!-- A viewer sees the whole list and can play it; the sentence names the
           person who can change that, instead of leaving a dead screen. -->
      <p class="readnote">
        <Icon name="eye" size={15} />
        <span>Lecture seule — demandez à {data.playlist.owner} de vous donner l'accès en modification.</span>
      </p>
    {/if}

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

    {#if showRecommendations}
      <!-- Where the playlist stops: library titles near its own, loaded after
           the list is on screen (never in its way). -->
      <section class="recs" aria-labelledby="recs-title">
        <div class="rec-head">
          <h2 class="section-title" id="recs-title">Titres recommandés</h2>
          {#if recs.length > 1}
            <button class="btn btn-ghost btn-sm" on:click={addAllRecommendations} disabled={recBusy}>
              <Icon name="plus" size={15} /> Tout ajouter
            </button>
          {/if}
        </div>

        {#if recsState === "failed"}
          <div class="rec-state">
            <span>Impossible de charger les suggestions.</span>
            <button class="btn btn-ghost btn-sm" on:click={retryRecommendations}>
              <Icon name="refresh" size={15} /> Réessayer
            </button>
          </div>
        {:else if recs.length}
          <ul class="rec-list">
            {#each recs as t (trackKey(t))}
              <li class="rec">
                <span class="rec-cover">
                  <Cover src={t.album?.cover} alt={t.title} size={40} kind="track" fallbackId={trackRef(t)} />
                </span>
                <span class="rec-meta">
                  <span class="rec-title">{t.title}</span>
                  <span class="rec-artist">{artistLine(t)}</span>
                </span>
                <span class="rec-dur mono">{fmtDuration(t.duration)}</span>
                <button
                  class="rec-add"
                  on:click={() => addRecommendation(t)}
                  disabled={recBusy || recPending.has(trackKey(t))}
                  aria-label={`Ajouter ${t.title} à la playlist`}
                  title="Ajouter à la playlist">
                  <Icon name="plus" size={18} />
                </button>
              </li>
            {/each}
          </ul>
        {:else if recsState === "loading"}
          <p class="rec-state">Recherche de suggestions…</p>
        {:else if recsState === "ready"}
          <p class="rec-state">Pas de suggestion pour l'instant</p>
        {/if}
      </section>
    {/if}
  </div>

  {#if showAdd}
    <AddTracksSheet
      {existingIds}
      onadd={onAdded}
      onclose={() => (showAdd = false)}
    />
  {/if}

  {#if showMembers && data?.playlist?.members}
    <PlaylistMembersSheet
      id={id}
      title={data.playlist.title}
      members={data.playlist.members}
      membersVersion={data.playlist.members_version}
      role={data.playlist.role}
      canManage={canManage}
      admin={!!data.playlist.admin}
      privateTracks={data.playlist.private_tracks || 0}
      me={$user?.name || ""}
      onclose={() => (showMembers = false)}
      onchange={onMembersChanged}
      ongone={() => markGone(id)}
      onleave={onLeft}
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
  .folder-link {
    width: fit-content;
    font-family: var(--font-mono);
    font-size: 11px;
    letter-spacing: 0.04em;
    color: var(--ac-text);
    border-radius: 6px;
  }
  .folder-link:hover {
    text-decoration: underline;
  }
  /* The people on a playlist: three faces, then the count. It is one 44 px
     target (the whole row), because "who is on it" and "manage who is on it"
     are the same question. */
  .members-stack {
    display: inline-flex;
    align-items: center;
    gap: 10px;
    width: fit-content;
    min-height: 44px;
    padding: 0 12px 0 4px;
    margin-left: -4px;
    border-radius: 999px;
    color: var(--dim);
  }
  .members-stack:hover {
    color: var(--tx);
    background: var(--s2);
  }
  .faces {
    display: inline-flex;
    align-items: center;
  }
  .face {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    flex: none;
    border-radius: 50%;
    overflow: hidden;
    background-size: cover;
    background-position: center;
    box-shadow: 0 0 0 2px var(--s2);
    color: #f1efe8;
    font-family: var(--font-display);
    font-weight: 700;
    font-size: 10px;
    letter-spacing: -0.02em;
    line-height: 1;
    user-select: none;
  }
  /* Touching discs, not a pile: an initial must stay readable at 22 px, and a
     deep overlap cuts the middle of it out. */
  .face + .face {
    margin-left: -2px;
  }
  .face.more {
    background: var(--s1);
    border: 1px solid var(--ln);
    font-family: var(--font-mono);
    font-size: 9px;
    font-weight: 500;
    color: var(--dim);
    box-shadow: 0 0 0 2px var(--s2);
  }
  .edited {
    max-width: 100%;
    font-size: 11px;
    letter-spacing: 0.02em;
    color: var(--dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .stale {
    height: 44px;
  }
  .readnote {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    margin: 0 0 14px;
    padding: 10px 12px;
    border: 1px solid var(--ln);
    border-radius: 12px;
    background: var(--s1);
    color: var(--dim);
    font-size: 13px;
    line-height: 1.45;
  }
  .readnote :global(svg) {
    flex: none;
    margin-top: 1px;
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
  /* The bottom of a playlist: a section heading with a quiet "Tout ajouter",
     then compact rows built like AddTracksSheet's (cover 40, two lines, the
     add button a full 44 px target). It sits AFTER the list, so it never
     delays it and never moves it while loading. */
  .recs {
    margin: 30px 0 10px;
    padding-top: 18px;
    border-top: 1px solid var(--ln);
  }
  .rec-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 6px;
  }
  .rec-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .rec {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 56px;
    padding: 6px 8px;
    border-radius: 10px;
  }
  .rec:hover {
    background: var(--s2);
  }
  .rec-cover {
    flex: none;
    width: 40px;
    height: 40px;
  }
  .rec-meta {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .rec-title {
    font-size: 15px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .rec-artist {
    font-size: 13px;
    color: var(--dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .rec-dur {
    flex: none;
    font-size: 12px;
    color: var(--dim);
  }
  .rec-add {
    flex: none;
    display: grid;
    place-items: center;
    width: 44px;
    height: 44px;
    border-radius: 50%;
    color: var(--dim);
  }
  .rec-add:hover:not(:disabled) {
    color: var(--tx);
    background: var(--s2);
  }
  .rec-add:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .rec-state {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 0;
    padding: 6px 8px;
    color: var(--dim);
    font-size: 14px;
  }
  @media (max-width: 640px) {
    /* The action pills scroll on ONE line instead of stacking three rows of
       chrome between the header and the list. */
    .actions {
      flex-wrap: wrap;
      overflow: visible;
      padding-bottom: 2px;
    }
    /* The exact moment is a tooltip under a finger: the name is what fits on
       one line in a 230 px metadata column. */
    .edited .when {
      display: none;
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
