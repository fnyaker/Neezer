<script>
  import { trackKey, trackRef } from "../lib/trackkey.js";
  import { onDestroy, onMount } from "svelte";
  import { push, replace } from "svelte-spa-router";
  import { REMOTE, CAN_KEEP, CAN_ADMIN, CAN_BROWSE } from "../lib/remote/mode.js";
  import {
    player,
    favTracks,
    toasts,
    isAdmin,
    syncing,
    downloads,
    openExport,
    resolvedUnavailable,
    unavailableVersion,
    playlists,
    openMenu,
  } from "../lib/stores.js";
  import { userPlaylists, loadMyFavorites, runDeezerSync, upsertPlaylistLocal } from "../lib/actions.js";
  import { listDownloads } from "../lib/offline.js";
  import { api } from "../lib/api.js";
  import { rememberScreen, recallScreen } from "../lib/nav.js";
  import { bytes as fmtBytes } from "../lib/format.js";
  import Card from "../components/Card.svelte";
  import TrackBrowser from "../components/TrackBrowser.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import Icon from "../components/Icon.svelte";
  import UnavailableList from "../components/UnavailableList.svelte";
  import ArtistTile from "../components/ArtistTile.svelte";
  import LibraryNav from "../components/LibraryNav.svelte";
  import Podcasts from "./Podcasts.svelte";
  import TrackList from "../components/TrackList.svelte";
  import { SORTS, createProjector } from "../lib/tracklist.js";
  import { STEP, nearEnd } from "../lib/grow.js";

  // One library, two worlds. The old row of chips mixed music with podcasts
  // and wrapped onto three rows on a phone; now a segmented control chooses the
  // world (Musique / Podcasts), then a strip of leaves filters it. The leaf is
  // still what the URL names (#/library/artists, #/library/podcasts…), so every
  // deep link and "Tout afficher" keeps working.
  export let params = {};
  const MUSIC_TABS = [
    { id: "favorites", label: "Titres" },
    { id: "artists", label: "Artistes" },
    { id: "albums", label: "Albums" },
    { id: "playlists", label: "Playlists", admin: true },
    { id: "downloaded", label: "Téléchargés", local: true },
    { id: "local", label: "Mes fichiers" },
  ];
  const PODCAST_TABS = [{ id: "podcasts", label: "Abonnements" }];
  const ALL_TABS = [...MUSIC_TABS, ...PODCAST_TABS, { id: "gone" }];
  $: musicTabs = MUSIC_TABS.filter((c) => (!c.admin || $isAdmin) && (!c.local || !REMOTE));
  const saved = recallScreen();
  let tab = (ALL_TABS.some((c) => c.id === params.tab) && params.tab) || saved?.tab || "favorites";
  // A tab the current mode does not even mount (a remote control has no
  // downloads strip) still names its content; the panel falls back to the
  // world's own tab so it is never left unlabelled.
  $: panelLabel =
    tab === "gone" || tab === "podcasts" || musicTabs.some((t) => t.id === tab)
      ? `lib-leaf-${tab}`
      : "lib-cat-music";
  // The world is a function of the leaf, so the URL stays the single source of
  // truth and an old link needs no redirect. Musique remembers the leaf you
  // left it on: switching to Podcasts and back does not throw your filter away.
  $: category = tab === "podcasts" ? "podcasts" : "music";
  let lastMusic =
    saved?.lastMusic && ALL_TABS.some((c) => c.id === saved.lastMusic)
      ? saved.lastMusic
      : tab === "podcasts"
        ? "favorites"
        : tab;
  $: if (category === "music") lastMusic = tab;
  // The podcast grid is mounted once and then kept (hidden) across a world
  // switch: its own search box and its answers survive, and coming back costs
  // no request.
  let podcastsMounted = tab === "podcasts";
  $: if (tab === "podcasts") podcastsMounted = true;
  // A hash-only navigation (`#/library/artists` → `#/library/gone`) reuses this
  // component instead of remounting it: follow the URL when it moves, but not
  // when `choose()` itself is what moved it.
  let seenTab = params.tab;
  $: if (params.tab !== seenTab) {
    seenTab = params.tab;
    if (params.tab && ALL_TABS.some((c) => c.id === params.tab)) tab = params.tab;
  }
  function choose(id) {
    tab = id;
    replace("/library/" + id);
  }
  function chooseCategory(world) {
    if (world === category) return;
    choose(world === "podcasts" ? "podcasts" : lastMusic);
  }
  // Upload usage/cap for the current user (guests get a per-user quota; admins
  // are unlimited). Null until loaded.
  let usage = null;
  $: usagePct = usage && usage.quota ? Math.min(100, (usage.used / usage.quota) * 100) : 0;
  async function loadUsage() {
    try {
      usage = await api.uploadUsage();
    } catch {
      usage = null;
    }
  }
  // favTracks is a shared cache: instant on revisit, refreshed in background.
  $: favorites = $favTracks;
  // Straight off the shared store: a playlist created from the "add to
  // playlist" sheet (which opens OVER this screen, so nothing here remounts)
  // shows up in this tab the moment it exists.
  $: myPlaylists = $playlists;
  let plQuery = saved?.plQuery ?? "";
  $: rememberScreen({ tab, lastMusic, plQuery, arQuery, arSort });
  $: shownPlaylists = filterPlaylists(myPlaylists, plQuery);
  function filterPlaylists(list, term) {
    if (!list) return null;
    const t = term.trim().toLowerCase();
    if (!t) return list;
    return list.filter((p) => (p.title || "").toLowerCase().includes(t));
  }
  // -- artists and albums you starred -----------------------------------------
  let artists = null;
  let albums = null;
  let arQuery = saved?.arQuery ?? "";
  let arSort = saved?.arSort ?? "plays"; // plays | name | recent
  const AR_SORTS = [
    { id: "plays", label: "Les plus écoutés" },
    { id: "name", label: "A-Z" },
    { id: "recent", label: "Ajoutés récemment" },
  ];
  $: if (tab === "artists" && artists === null) loadArtists();
  $: if (tab === "albums" && albums === null) loadAlbums();
  async function loadArtists() {
    artists = [];
    try {
      await api.swr("/me/artists", (d) => (artists = d.artists || []));
    } catch {
      /* the list stays empty and says so */
    }
  }
  async function loadAlbums() {
    albums = [];
    try {
      await api.swr("/me/albums", (d) => (albums = d.albums || []));
    } catch {
      /* the list stays empty and says so */
    }
  }
  const fold = (x) => (x || "").normalize("NFD").replace(/[\u0300-\u036f]/g, "").toLowerCase();
  $: shownArtists = (() => {
    if (!artists) return [];
    const t = fold(arQuery.trim());
    const list = t ? artists.filter((a) => fold(a.name).includes(t)) : artists.slice();
    if (arSort === "name") list.sort((a, b) => fold(a.name).localeCompare(fold(b.name), "fr"));
    else if (arSort === "recent") list.sort((a, b) => (b.starred || "").localeCompare(a.starred || ""));
    return list; // "plays" is the server's own order
  })();
  // The grids mount one screenful at a time (lib/grow.js); a new filter or
  // query starts again from the top.
  let arLimit = STEP;
  let alLimit = STEP;
  let plLimit = STEP;
  let arKey, alKey, plKey;
  $: if (arKey !== (arKey = `${tab}\0${arQuery}\0${arSort}`)) arLimit = STEP;
  $: if (alKey !== (alKey = tab)) alLimit = STEP;
  $: if (plKey !== (plKey = `${tab}\0${plQuery}`)) plLimit = STEP;
  const titles = (n) => (n ? `${n} titre${n > 1 ? "s" : ""}` : "Aucun titre gardé");

  let local = null;
  let offline = null;
  // Tracks the server knows can no longer be played. Loaded when the tab is
  // first opened (and refreshed when a replacement lands), not on every page
  // view — it's a rare list, and a query nobody asked for is a query wasted.
  let gone = null;
  let goneLoaded = false;
  $: if (tab === "gone" && !goneLoaded) loadGone();
  async function loadGone() {
    goneLoaded = true;
    try {
      gone = (await api.unavailableTracks()).tracks || [];
    } catch {
      gone = [];
    }
  }
  // Dealing with a track (replaced, deleted, playable again) drops it from the
  // list at once — the server work runs in a worker thread, so refetching here
  // would just re-read the old state and the row would flicker back.
  $: if ($resolvedUnavailable && gone) {
    gone = gone.filter((t) => trackKey(t) !== $resolvedUnavailable);
  }
  // …then the truth, once the server says it's done.
  let seenVersion = 0;
  $: if ($unavailableVersion !== seenVersion) {
    seenVersion = $unavailableVersion;
    if (goneLoaded) loadGone();
  }

  let fileInput;
  let uploading = false;

  onMount(() => {
    loadMyFavorites();
    if ($isAdmin) userPlaylists();
    loadLocal();
    loadUsage();
  });

  // Downloaded tracks come straight from IndexedDB, so this tab works offline.
  // Refreshed whenever the downloaded set changes (add / remove / evict), but
  // COALESCED: downloading a 50-track album flips that store once per track,
  // and every flip used to re-read the entire IndexedDB meta store. The
  // sequence guard stops a slower earlier read from landing on a newer one.
  let offlineSeq = 0;
  let offlineTimer = null;
  $: $downloads, scheduleOffline();
  function scheduleOffline() {
    clearTimeout(offlineTimer);
    offlineTimer = setTimeout(loadOffline, 200);
  }
  async function loadOffline() {
    const mine = ++offlineSeq;
    const list = (await listDownloads()).map((m) => m.track).filter(Boolean);
    if (mine === offlineSeq) offline = list;
  }
  onDestroy(() => clearTimeout(offlineTimer));

  function playOffline() {
    if (offline?.length) player.playQueue(offline, 0, { kind: "downloads" });
  }

  async function loadLocal() {
    try {
      local = (await api.myLocal()).tracks || [];
    } catch {
      local = [];
    }
  }

  async function onFiles(e) {
    const files = [...(e.target.files || [])];
    e.target.value = ""; // allow re-selecting the same files later
    if (!files.length) return;
    uploading = true;
    try {
      const r = await api.upload(files);
      if (r.count) toasts.push(`${r.count} fichier(s) importé(s)`);
      if (r.quota_exceeded)
        toasts.push("Quota d'upload atteint : certains fichiers ont été refusés.", "error");
      else if (r.skipped && r.skipped.length)
        toasts.push(`${r.skipped.length} ignoré(s) (format non géré)`, "error");
      await loadLocal();
      await loadUsage();
      choose("local");
    } catch (e) {
      toasts.push(e?.status === 413 ? e.message : "Échec de l'import", "error");
    } finally {
      uploading = false;
    }
  }

  function playFavorites() {
    if (favorites?.length) player.playQueue(favorites, 0, { kind: "favorites" });
  }
  function shuffleFavorites() {
    if (favorites?.length) player.shufflePlay(favorites, { kind: "favorites" });
  }
  // The Titres table owns its action bar AND its column headings, so it cannot
  // be the shared TrackBrowser (which brings its own toolbar). Search and sort
  // still run on the same index (lib/tracklist.js) — one pass over folded
  // bytes, so a keystroke on 4 000 favourites stays cheap.
  let fQuery = "";
  let fSort = "default";
  let fDir = 1;
  const projectFavorites = createProjector();
  onDestroy(() => projectFavorites.free());
  $: shownFavorites = projectFavorites(favorites || [], fSort, fDir, fQuery);
  let dlBusy = false;
  async function archiveFavorites() {
    if (dlBusy) return;
    // Server-side pre-archive is for Deezer tracks: local/uploaded files are
    // already on the disk, and YouTube keeps its own Opus.
    const ids = (favorites || []).filter((t) => !t.local && !t.podcast && t.source !== "youtube").map(trackRef);
    if (!ids.length) {
      toasts.push("Rien à archiver (fichiers déjà locaux)");
      return;
    }
    dlBusy = true;
    try {
      const r = await api.download(ids);
      toasts.push(`Téléchargement de ${r.queued} titres lancé`);
    } catch {
      toasts.push("Téléchargement impossible", "error");
    } finally {
      dlBusy = false;
    }
  }
  // Header counters: only what is already on the device (the favourites and the
  // playlists are loaded on mount); a count nobody fetched is left out rather
  // than asked for on every open of the library.
  const plural = (n, one, many) => `${n} ${n > 1 ? many : one}`;
  $: counts = [
    favorites === null ? null : plural(favorites.length, "titre", "titres"),
    artists === null ? null : plural(artists.length, "artiste", "artistes"),
    myPlaylists === null ? null : plural(myPlaylists.length, "playlist", "playlists"),
  ]
    .filter(Boolean)
    .join(" · ");
  function playLocal() {
    if (local?.length) player.playQueue(local, 0, { kind: "local" });
  }

  // Manual "refresh from Deezer" (shared action), then refresh the playlists.
  async function syncDeezer() {
    if (await runDeezerSync()) userPlaylists(true);
  }

  async function newPlaylist() {
    const title = (window.prompt("Nom de la playlist ?") || "").trim();
    if (!title) return;
    try {
      const r = await api.createPlaylist(title, []);
      if (r?.id) upsertPlaylistLocal({ id: String(r.id), title, deezer_id: r.deezer_id });
      userPlaylists(true);
      if (r?.id) push("/playlist/" + r.id);
    } catch {
      toasts.push("Impossible de créer la playlist", "error");
    }
  }

  // The adding actions are ONE button opening a menu (a toolbar holds
  // actions, the platforms' rule): three pills of labels crowded the title on
  // a phone and read as navigation.
  function addMenu(e) {
    const items = [];
    if (CAN_KEEP && $isAdmin) items.push({ label: "Nouvelle playlist", icon: "plus", action: newPlaylist });
    if (CAN_BROWSE)
      items.push({ label: "Importer une playlist partagée", icon: "link", action: () => push("/import") });
    if (CAN_KEEP) items.push({ label: "Importer des fichiers audio", icon: "upload", action: () => fileInput.click() });
    if ($isAdmin && CAN_ADMIN)
      items.push({ label: "Synchroniser depuis Deezer", icon: "refresh", action: syncDeezer });
    const r = e.currentTarget.getBoundingClientRect();
    openMenu({ preventDefault() {}, stopPropagation() {}, clientX: r.right, clientY: r.bottom + 4 }, items);
  }
</script>

<div class="head">
  <h1 class="page-title">Bibliothèque</h1>
  {#if counts}<span class="counts mono">{counts}</span>{/if}
  <span class="head-actions">
    <a class="btn btn-icon" href="#/import" title="Importer une playlist partagée" aria-label="Importer une playlist partagée">
      <Icon name="link" size={18} />
    </a>
    {#if CAN_KEEP || CAN_BROWSE}
    <button class="btn btn-icon add" on:click={addMenu} disabled={uploading || $syncing} aria-label="Ajouter à la bibliothèque" title="Ajouter">
      {#if uploading || $syncing}<span class="spin"><Icon name="refresh" size={20} /></span>{:else}<Icon name="plus" size={20} />{/if}
    </button>
    {/if}
  </span>
  <input
    bind:this={fileInput}
    type="file"
    accept="audio/*,.flac,.opus,.m4a,.ogg,.wav,.aac,.wma"
    multiple
    on:change={onFiles}
    hidden
  />
</div>

{#if usage && !usage.unlimited && usage.quota}
  <div class="quota">
    <div class="quota-bar"><span style={`width:${usagePct}%`} class:warn={usagePct > 90}></span></div>
    <span class="muted quota-txt label-mono">{fmtBytes(usage.used)} / {fmtBytes(usage.quota)} d'espace d'upload utilisé</span>
  </div>
{/if}

<LibraryNav
  {category}
  {tab}
  musicTabs={musicTabs}
  podcastTabs={PODCAST_TABS}
  goneCount={gone?.length || null}
  onpick={choose}
  oncategory={chooseCategory}
/>

<div id="library-view" role="tabpanel" aria-labelledby={panelLabel}>
{#if tab === "favorites"}
  {#if favorites === null}
    <Skeleton kind="list" />
  {:else if !favorites.length}
    <div class="empty">
      <svg class="rings" viewBox="0 0 64 64" width="64" height="64" aria-hidden="true">
        <circle cx="32" cy="32" r="28" /><circle cx="32" cy="32" r="18" /><circle cx="32" cy="32" r="6" />
      </svg>
      <p class="empty-title">Rien ici pour l'instant</p>
      <p class="empty-text">Touche le cœur d'un titre pour le garder ici.</p>
      <button class="btn btn-primary" on:click={() => push("/search")}><Icon name="search" size={16} /> Explorer</button>
    </div>
  {:else}
    <div class="bar">
      <div class="bar-actions">
        <button class="btn btn-primary" on:click={playFavorites}><Icon name="play" size={15} /> Tout lire</button>
        <button class="btn btn-ghost" on:click={shuffleFavorites}><Icon name="shuffle" size={16} /> Mélanger</button>
        <button
          class="btn btn-ghost"
          on:click={() => openExport("favorites", "me", "Favoris")}
          title="Exporter : vers un service, en ZIP ou en liste"><Icon name="archive" size={16} /> Exporter</button>
      </div>
      <span class="spacer"></span>
      <label class="field search">
        <Icon name="search" size={16} />
        <input placeholder="Rechercher dans cette liste…" bind:value={fQuery} aria-label="Filtrer dans cette liste" />
      </label>
      <span class="selwrap">
        <select class="sortsel" bind:value={fSort} aria-label="Trier par">
          {#each SORTS as s}<option value={s.key}>{s.label}</option>{/each}
        </select>
        <Icon name="chevronDown" size={15} />
      </span>
      <button
        class="btn btn-icon"
        class:rev={fDir === -1}
        on:click={() => (fDir = -fDir)}
        aria-label="Inverser l'ordre"
        title="Inverser l'ordre"><Icon name="sort" size={17} /></button>
      <button
        class="btn btn-icon"
        on:click={archiveFavorites}
        disabled={dlBusy}
        aria-label="Télécharger la liste"
        title="Télécharger (archiver) toute la liste"><Icon name="download" size={17} /></button>
    </div>
    {#if fQuery.trim() && !shownFavorites.length}
      <p class="muted hint">Aucun titre ne correspond à « {fQuery} ».</p>
    {:else}
      <TrackList tracks={shownFavorites} context={{ kind: "favorites" }} numbered headers />
    {/if}
  {/if}
{:else if tab === "artists"}
  {#if artists === null || (!artists.length && !arQuery)}
    {#if artists === null}
      <Skeleton kind="shelf" />
    {:else}
      <div class="empty">
        <svg class="rings" viewBox="0 0 64 64" width="64" height="64" aria-hidden="true">
          <circle cx="32" cy="32" r="28" /><circle cx="32" cy="32" r="18" /><circle cx="32" cy="32" r="6" />
        </svg>
        <p class="empty-title">Aucun artiste suivi</p>
        <p class="empty-text">
          Ouvre la page d'un artiste et touche le cœur pour le suivre : ses nouveautés arriveront
          sur l'accueil.
        </p>
        <button class="btn btn-primary" on:click={() => push("/search")}><Icon name="search" size={16} /> Explorer</button>
      </div>
    {/if}
  {:else}
    <div class="tools">
      <label class="field search">
        <Icon name="search" size={16} />
        <input placeholder="Rechercher un artiste…" bind:value={arQuery} aria-label="Rechercher un artiste" />
      </label>
      <label class="sort">
        <span class="selwrap">
          <select class="sortsel" bind:value={arSort} aria-label="Trier les artistes">
            {#each AR_SORTS as o}<option value={o.id}>{o.label}</option>{/each}
          </select>
          <Icon name="chevronDown" size={15} />
        </span>
      </label>
      <span class="spacer"></span>
      <span class="count">{artists.length} artiste{artists.length > 1 ? "s" : ""}</span>
    </div>
    {#if !shownArtists.length}
      <p class="muted hint">Aucun artiste ne correspond à « {arQuery} ».</p>
    {:else}
      <div class="artist-grid">
        {#each shownArtists.slice(0, arLimit) as a (a.id)}<ArtistTile artist={a} detail={titles(a.tracks)} />{/each}
      </div>
      {#if shownArtists.length > arLimit}<div class="grow" use:nearEnd={() => (arLimit += STEP)}></div>{/if}
    {/if}
  {/if}
{:else if tab === "albums"}
  {#if albums === null}
    <Skeleton kind="shelf" />
  {:else if !albums.length}
    <div class="empty">
      <svg class="rings" viewBox="0 0 64 64" width="64" height="64" aria-hidden="true">
        <circle cx="32" cy="32" r="28" /><circle cx="32" cy="32" r="18" /><circle cx="32" cy="32" r="6" />
      </svg>
      <p class="empty-title">Aucun album gardé</p>
      <p class="empty-text">Touche le cœur sur la page d'un album pour le garder ici.</p>
      <button class="btn btn-primary" on:click={() => push("/search")}><Icon name="search" size={16} /> Parcourir les albums</button>
    </div>
  {:else}
    <div class="grid">
      {#each albums.slice(0, alLimit) as a (a.id)}<Card item={a} kind="album" />{/each}
    </div>
    {#if albums.length > alLimit}<div class="grow" use:nearEnd={() => (alLimit += STEP)}></div>{/if}
  {/if}
{:else if tab === "playlists"}
  {#if myPlaylists === null}
    <Skeleton kind="shelf" />
  {:else if !myPlaylists.length}
    <div class="empty">
      <svg class="rings" viewBox="0 0 64 64" width="64" height="64" aria-hidden="true">
        <circle cx="32" cy="32" r="28" /><circle cx="32" cy="32" r="18" /><circle cx="32" cy="32" r="6" />
      </svg>
      <p class="empty-title">Aucune playlist</p>
      <p class="empty-text">Une playlist pour un moment, une humeur, une route : crée la première.</p>
      {#if CAN_KEEP && $isAdmin}
        <button class="btn btn-primary" on:click={newPlaylist}><Icon name="plus" size={16} /> Nouvelle playlist</button>
      {:else}
        <button class="btn btn-primary" on:click={() => push("/search")}><Icon name="search" size={16} /> Explorer</button>
      {/if}
    </div>
  {:else}
    <div class="tools">
      <label class="field search">
        <Icon name="search" size={16} />
        <input placeholder="Rechercher dans mes playlists…" bind:value={plQuery} aria-label="Rechercher une playlist" />
      </label>
      <span class="spacer"></span>
      <span class="count">{myPlaylists.length} playlist{myPlaylists.length > 1 ? "s" : ""}</span>
    </div>
    {#if !shownPlaylists.length}
      <p class="muted hint">Aucune playlist ne correspond à « {plQuery} ».</p>
    {:else}
      <div class="pl-grid">
        {#each shownPlaylists.slice(0, plLimit) as p (p.id)}
          <Card item={{ ...p, subtitle: `${p.nb_tracks ?? 0} titre${p.nb_tracks > 1 ? "s" : ""}` }} kind="playlist" />
        {/each}
      </div>
      {#if shownPlaylists.length > plLimit}<div class="grow" use:nearEnd={() => (plLimit += STEP)}></div>{/if}
    {/if}
  {/if}
{:else if tab === "downloaded"}
  {#if offline === null}
    <Skeleton kind="list" />
  {:else if !offline.length}
    <div class="empty">
      <svg class="rings" viewBox="0 0 64 64" width="64" height="64" aria-hidden="true">
        <circle cx="32" cy="32" r="28" /><circle cx="32" cy="32" r="18" /><circle cx="32" cy="32" r="6" />
      </svg>
      <p class="empty-title">Rien hors-ligne</p>
      <p class="empty-text">
        Télécharge un titre, un album ou une playlist pour l'écouter sans réseau, en avion comme
        dans le métro.
      </p>
      <button class="btn btn-primary" on:click={() => push("/search")}><Icon name="search" size={16} /> Explorer</button>
    </div>
  {:else}
    <div class="bar">
      <button class="btn btn-primary" on:click={playOffline}><Icon name="play" size={15} /> Tout lire</button>
      <span class="count-line label-mono">{offline.length} titres · hors-ligne</span>
    </div>
    <TrackBrowser tracks={offline} context={{ kind: "downloads" }} downloadable={false} />
  {/if}
{:else if tab === "gone"}
  {#if gone === null}
    <Skeleton kind="list" />
  {:else if !gone.length}
    <div class="empty">
      <svg class="rings" viewBox="0 0 64 64" width="64" height="64" aria-hidden="true">
        <circle cx="32" cy="32" r="28" /><circle cx="32" cy="32" r="18" /><circle cx="32" cy="32" r="6" />
      </svg>
      <p class="empty-title">Aucun titre indisponible</p>
      <p class="empty-text">
        Quand Deezer retire un titre de son catalogue, il apparaît ici pour que tu puisses lui
        trouver un remplaçant. Ceux que tu as déjà écoutés sont archivés sur le serveur&nbsp;:
        ceux-là restent lisibles quoi qu'il arrive.
      </p>
      <button class="btn btn-primary" on:click={() => choose("favorites")}><Icon name="play" size={15} /> Écouter mes favoris</button>
    </div>
  {:else}
    <p class="muted hint gone-hint">
      Ces titres ne peuvent plus être lus. Choisissez un remplaçant&nbsp;: il
      prendra leur place dans toutes vos playlists et vos favoris. Ou cochez
      ceux dont vous ne voulez plus et supprimez-les d'un coup.
    </p>
    <UnavailableList bind:tracks={gone} />
  {/if}
{:else if tab === "local"}
  {#if local === null}
    <Skeleton kind="list" />
  {:else if !local.length}
    <div class="empty">
      <svg class="rings" viewBox="0 0 64 64" width="64" height="64" aria-hidden="true">
        <circle cx="32" cy="32" r="28" /><circle cx="32" cy="32" r="18" /><circle cx="32" cy="32" r="6" />
      </svg>
      <p class="empty-title">Aucun fichier à toi</p>
      <p class="empty-text">Ajoute ta propre musique : n'importe quel format, elle rejoint la bibliothèque.</p>
      {#if CAN_KEEP}
        <button class="btn btn-primary" on:click={() => fileInput.click()}><Icon name="upload" size={16} /> Importer des fichiers</button>
      {:else}
        <button class="btn btn-primary" on:click={() => push("/search")}><Icon name="search" size={16} /> Explorer</button>
      {/if}
    </div>
  {:else}
    <div class="bar">
      <button class="btn btn-primary" on:click={playLocal}><Icon name="play" size={15} /> Tout lire</button>
      <span class="count-line label-mono">{local.length} fichiers</span>
    </div>
    <TrackBrowser tracks={local} context={{ kind: "local" }} downloadable={false} />
  {/if}
{/if}

{#if podcastsMounted}
  <div hidden={tab !== "podcasts"}><Podcasts embedded /></div>
{/if}
</div>

<style>
  .grow {
    height: 1px;
  }
  .head-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 12px 20px;
    flex-wrap: wrap;
  }
  .head .page-title {
    margin: 0;
  }
  .counts {
    margin-left: auto;
    padding-bottom: 4px;
    font-size: 12px;
    color: var(--dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .add {
    flex: none;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .add .spin {
    display: grid;
    animation: spin 1s linear infinite;
  }
  .quota {
    margin-top: 14px;
  }
  .quota-bar {
    height: 6px;
    border-radius: 3px;
    background: var(--s2);
    overflow: hidden;
  }
  .quota-bar span {
    display: block;
    height: 100%;
    background: var(--ac);
    transition: width 0.25s ease;
  }
  .quota-bar span.warn {
    background: var(--tx);
  }
  .quota-txt {
    display: inline-block;
    margin-top: 6px;
    font-size: 11px;
  }
  /* With the File panel open the content column can be ~650 px while the
     window is still wide: wrapping is the container-aware answer (a window
     media query would miss it). */
  :global(.layout.np-open) .bar {
    flex-wrap: wrap;
    overflow: visible;
  }
  /* One action bar above a table: the primary on the left, the list's own
     tools on the right, everything on the 40 px rhythm (§6). */
  .bar,
  .tools {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
    margin: 0 0 10px;
  }
  .bar-actions {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .bar .spacer,
  .tools .spacer {
    flex: 1;
  }
  .field.search {
    width: 260px;
    flex: 0 1 260px;
    min-width: 160px;
  }
  .field.search input {
    border: 0;
    background: none;
    outline: none;
    color: var(--tx);
    font: inherit;
    width: 100%;
    min-width: 0;
    padding: 0;
  }
  .field:focus-within {
    border-color: var(--ac-text);
    color: var(--tx);
  }
  /* A native select on Linux paints its own grey widget: the theme owns the
     shape and the glyph instead (appearance:none + the chevron beside it). */
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
    padding: 0 36px 0 14px;
    border-radius: 999px;
    border: 1px solid var(--ln);
    background: var(--s1);
    color: var(--tx);
    font: inherit;
    font-size: 13px;
    outline: none;
    appearance: none;
    -webkit-appearance: none;
  }
  .sortsel:focus-visible,
  .btn:focus-visible {
    outline: 2px solid var(--ac-text);
    outline-offset: 2px;
  }
  .btn-icon.rev {
    color: var(--ac-text);
    border-color: var(--ac-text);
  }
  .count-line {
    display: inline-block;
    margin: 0 0 10px;
    font-size: 11px;
  }
  .tools .search {
    flex: 1 1 240px;
    width: auto;
  }
  .sort {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .sort .label-mono {
    font-size: 11px;
  }
  .count {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--dim);
    white-space: nowrap;
  }
  /* §6: seven artist portraits across, the fallback disc capped so the name
     stays the thing you read (ArtistTile's own tile is full width). */
  .artist-grid {
    display: grid;
    grid-template-columns: repeat(7, minmax(0, 1fr));
    gap: 8px 12px;
  }
  .artist-grid :global(.pic) {
    max-width: 104px;
    margin-inline: auto;
  }
  .pl-grid {
    display: grid;
    grid-template-columns: repeat(6, minmax(0, 1fr));
    gap: 16px;
  }
  /* An empty tab is a place with an action, not a grey sentence: rings (the
     record), what happened, what to do now. */
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    text-align: center;
    padding: 56px 24px;
    border: 1px dashed var(--ln);
    border-radius: var(--r-card);
    margin-top: 4px;
  }
  .rings {
    fill: none;
    stroke: var(--dim);
    stroke-width: 2;
    opacity: 0.8;
  }
  .empty-title {
    margin: 0;
    font-family: var(--font-display);
    font-size: 22px;
    font-weight: 600;
    letter-spacing: -0.015em;
  }
  .empty-text {
    margin: 0;
    max-width: 420px;
    color: var(--dim);
    font-size: 14px;
    line-height: 1.5;
  }
  .empty .btn {
    margin-top: 4px;
  }
  .hint {
    margin-top: 20px;
  }
  .gone-hint {
    margin-bottom: 14px;
  }
  @media (max-width: 1200px) {
    .artist-grid {
      grid-template-columns: repeat(6, minmax(0, 1fr));
    }
    .pl-grid {
      grid-template-columns: repeat(5, minmax(0, 1fr));
    }
  }
  @media (max-width: 1000px) {
    .artist-grid {
      grid-template-columns: repeat(5, minmax(0, 1fr));
    }
    .pl-grid {
      grid-template-columns: repeat(4, minmax(0, 1fr));
    }
  }
  @media (max-width: 820px) {
    .artist-grid {
      grid-template-columns: repeat(4, minmax(0, 1fr));
    }
    .pl-grid {
      grid-template-columns: repeat(3, minmax(0, 1fr));
    }
  }
  /* Tablets and narrow desktop windows: the sidebar leaves ~500 px, where the
     title, the three counts and the + no longer share one line — the counts
     take their own line instead of pushing the button onto a third. */
  @media (max-width: 1024px) {
    .head {
      display: grid;
      grid-template-columns: minmax(0, 1fr) auto;
      align-items: center;
      gap: 4px 12px;
    }
    .head .page-title {
      grid-column: 1;
      grid-row: 1;
    }
    .add {
      grid-column: 2;
      grid-row: 1;
      margin-left: 0;
    }
    .counts {
      grid-column: 1 / -1;
      grid-row: 2;
      margin-left: 0;
      padding: 0;
      font-size: 11px;
    }
    /* The three actions wrap rather than scrolling the last one out of
       reach; the filter and the sort share the next row. */
    .bar-actions {
      width: 100%;
      gap: 6px;
      flex-wrap: wrap;
      overflow: visible;
      padding-bottom: 2px;
    }
    .field.search,
    .tools .search {
      flex: 1 1 200px;
      width: auto;
      max-width: none;
      min-width: 150px;
    }
    .selwrap {
      flex: 0 0 auto;
    }
  }
  @media (max-width: 640px) {
    .head {
      align-items: flex-start;
    }
    /* The counters are a sentence, not a label: on a phone a big library
       wraps them under the title instead of clipping them to "4000 titres · 3…"
       (the ellipsis was the old one-line rule). */
    .counts {
      white-space: normal;
      overflow: visible;
      line-height: 1.4;
    }
    .bar .spacer,
    .tools .spacer {
      display: none;
    }
    /* A phone cannot stack five rows of controls above the list: the three
       actions wrap (never a clipped "Exporter"), the filter takes the next
       row, and the sort shares the last with its two icons. */
    .bar,
    .tools {
      gap: 8px;
    }
    .bar-actions {
      width: 100%;
      gap: 6px;
      flex-wrap: wrap;
      overflow: visible;
      padding-bottom: 2px;
    }
    .field.search,
    .tools .search {
      flex: 1 1 100%;
      width: auto;
      max-width: none;
    }
    .selwrap {
      flex: 1 1 auto;
      min-width: 0;
    }
    .selwrap .sortsel {
      width: 100%;
      min-width: 0;
    }
    .artist-grid {
      grid-template-columns: repeat(3, minmax(0, 1fr));
      gap: 4px;
    }
    .artist-grid :global(.pic) {
      max-width: 88px;
    }
    .pl-grid {
      grid-template-columns: repeat(2, minmax(0, 1fr));
      gap: 12px;
    }
    .empty {
      padding: 40px 18px;
    }
    .empty-title {
      font-size: 20px;
    }
  }
</style>
