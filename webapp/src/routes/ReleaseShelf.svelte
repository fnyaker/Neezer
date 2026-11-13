<script>
  // The full page behind a shelf card: the same payload Nouveautés already
  // holds, regrouped by artist proximity. It is a playlist-LIKE page — banner,
  // facts, Lire / Mélanger, the whole TrackList with headings and a
  // search/sort — but it is read-only: a shelf is not a playlist, so there is
  // nothing to rename, reorder, add or remove here.
  import { onDestroy, onMount } from "svelte";
  import { push } from "svelte-spa-router";
  import { failureCause, isDeezerDenied, isDeezerUnreachable } from "../lib/api.js";
  import { player } from "../lib/stores.js";
  import { duration as fmtDuration } from "../lib/format.js";
  import { SORTS, createProjector } from "../lib/tracklist.js";
  import {
    SHELF_COPY,
    buildReleaseShelves,
    loadReleasesFeed,
    readReleasesFeed,
  } from "./Releases.svelte";
  import Cover from "../components/Cover.svelte";
  import Icon from "../components/Icon.svelte";
  import TrackList from "../components/TrackList.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import LoadFailed from "../components/LoadFailed.svelte";

  export let shelf = "artists";
  // A route prop (App.svelte): "artists" or "discovery". Anything else is a
  // bad link, and the page falls back to the first shelf instead of a blank.
  $: key = SHELF_COPY[shelf] ? shelf : "artists";
  $: copy = SHELF_COPY[key];

  // The feed is module-scoped in Releases.svelte: a card click reaches this page
  // with the payload already in memory, so the first paint is immediate and
  // costs no request. Only a deep link (a reload, a shared URL) fetches.
  let feed = readReleasesFeed();
  let loading = !feed;
  let loadError = null;

  let query = "";
  let sort = "default";
  let dir = 1;
  const project = createProjector();
  onDestroy(() => project.free());

  $: shelves = feed ? buildReleaseShelves(feed) : null;
  $: view = shelves?.[key] || { tracks: [], releases: [], closeNames: [] };
  $: tracks = view.tracks;
  $: releases = view.releases;
  $: closeNames = view.closeNames || [];
  $: total = tracks.reduce((s, t) => s + (t.duration || 0), 0);
  $: facts = [
    `${tracks.length} titre${tracks.length > 1 ? "s" : ""}`,
    `${releases.length} sortie${releases.length > 1 ? "s" : ""}`,
  ]
    .filter(Boolean)
    .join(" · ");
  $: description = shelfDescription(key, closeNames, copy.description);
  $: failureText = loadError
    ? `Cette sélection n'a pas pu être chargée (${failureCause(loadError)}).`
    : "Cette sélection n'a pas pu être chargée.";
  $: shown = project(tracks, sort, dir, query);
  $: searching = !!query.trim();
  $: context = { kind: "releases", id: key };

  // The banner is a mosaic of what is in the shelf; one sleeve is shown whole,
  // two side by side, three or four as the familiar 2x2.
  $: sleeves = (releases.length ? releases : [null]).slice(0, 4);
  $: mosaic = sleeves.some((r) => r && r.cover) ? sleeves : sleeves.slice(0, 1);
  $: cells = mosaic.length;

  function shelfDescription(name, names, fallback) {
    if (name !== "discovery" || !names.length) return fallback;
    const shown = names.slice(0, 3);
    const more = names.length - shown.length;
    if (!more) return `Des artistes proches de ${listFr(shown)}.`;
    return `Des artistes proches de ${shown.join(", ")} et ${more} autre${more > 1 ? "s" : ""}.`;
  }
  function listFr(names) {
    if (names.length <= 1) return names[0] || "";
    return names.slice(0, -1).join(", ") + " et " + names[names.length - 1];
  }

  async function load() {
    loading = true;
    loadError = null;
    try {
      await loadReleasesFeed();
      feed = readReleasesFeed();
    } catch (e) {
      if (!feed) loadError = e;
    } finally {
      loading = false;
    }
  }
  onMount(() => {
    if (!feed) load();
  });

  function back() {
    if (window.history.length > 1) window.history.back();
    else push("/releases");
  }
  function playAll() {
    if (tracks.length) player.playQueue(tracks, 0, context);
  }
  function shufflePlay() {
    if (tracks.length) player.shufflePlay(tracks, context);
  }
</script>

{#if loading}
  <Skeleton kind="header" />
  <Skeleton kind="list" />
{:else if !feed}
  <LoadFailed
    error={loadError}
    notFound={failureText}
    on:retry={load}
  />
  {#if !isDeezerUnreachable(loadError) && !isDeezerDenied(loadError)}
    <div class="retry">
      <button class="btn btn-primary" on:click={load}><Icon name="refresh" size={15} /> Réessayer</button>
    </div>
  {/if}
{:else}
  <div class="fade-in">
    <header class="banner">
      <div
        class="art"
        class:one={cells === 1}
        class:two={cells === 2}
        class:three={cells === 3}
        class:four={cells === 4}
      >
        {#each mosaic as r, i (r?.deezer_id || i)}
          <span class="cell">
            <Cover src={r?.cover || null} alt={r?.title || copy.title} kind="album" eager />
          </span>
        {/each}
      </div>
      <div class="meta">
        <div class="kicker">
          <button class="back" on:click={back} aria-label="Retour aux nouveautés" title="Retour aux nouveautés">
            <Icon name="chevronLeft" size={18} />
          </button>
          <span class="kind"><span class="prefix">Nouveautés · </span>{facts}{#if total}<span class="total"> · {fmtDuration(total)}</span>{/if}</span>
        </div>
        <h1>{copy.title}</h1>
        <p class="desc muted">{description}</p>
      </div>
    </header>

    <div class="actions">
      <button class="btn btn-primary" on:click={playAll} disabled={!tracks.length}>
        <Icon name="play" size={15} /> Lire
      </button>
      <button class="btn btn-ghost" on:click={shufflePlay} disabled={!tracks.length}>
        <Icon name="shuffle" size={16} /> Mélanger
      </button>
    </div>

    {#if tracks.length}
      <div class="toolbar">
        <label class="field searchbox">
          <Icon name="search" size={16} />
          <input
            placeholder="Rechercher dans cette sélection…"
            bind:value={query}
            aria-label="Rechercher dans cette sélection"
          />
        </label>
        <span class="spacer"></span>
        <span class="selwrap">
          <select class="sortsel" bind:value={sort} aria-label="Trier par">
            {#each SORTS as s}<option value={s.key}>{s.label}</option>{/each}
          </select>
          <Icon name="chevronDown" size={15} />
        </span>
        <button
          class="btn btn-icon"
          class:rev={dir === -1}
          on:click={() => (dir = -dir)}
          aria-label="Inverser l'ordre"
          title="Inverser l'ordre"
        >
          <Icon name="sort" size={17} />
        </button>
      </div>
      {#if searching && !shown.length}
        <p class="muted empty-hint">Aucun titre ne correspond à « {query} ».</p>
      {:else}
        <TrackList tracks={shown} {context} numbered headers />
      {/if}
    {:else}
      <div class="empty">
        <p class="empty-title">Rien par ici pour l'instant</p>
        <p class="empty-text">
          {key === "artists"
            ? "Ajoute des artistes à tes favoris : leurs prochaines sorties apparaîtront ici."
            : "Le serveur proposera des artistes proches de tes favoris dès qu'il en trouvera."}
        </p>
        <a class="btn btn-ghost" href="#/releases"><Icon name="sparkles" size={15} /> Voir les nouveautés</a>
      </div>
    {/if}
  </div>
{/if}

<style>
  /* The playlist page's band, copied rather than shared: this page is a
     read-only sibling, and the playlist file owns its own editing rules. */
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
    display: grid;
    gap: 2px;
    width: 200px;
    flex: none;
    aspect-ratio: 1 / 1;
    border-radius: var(--r-cover);
    overflow: hidden;
    background: var(--s2);
  }
  .art.one {
    grid-template-columns: 1fr;
  }
  .art.two {
    grid-template-columns: 1fr 1fr;
  }
  .art.three,
  .art.four {
    grid-template-columns: 1fr 1fr;
    grid-template-rows: 1fr 1fr;
  }
  .art.three .cell:last-child {
    grid-column: 1 / -1;
  }
  .cell {
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }
  .cell :global(.cover) {
    border-radius: 0;
  }
  .meta {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
    max-width: 62ch;
  }
  .kicker {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 36px;
  }
  .back {
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    flex: none;
    border-radius: 50%;
    border: 1px solid var(--ln);
    background: var(--s1);
    color: var(--tx);
  }
  .back:hover {
    background: var(--s2);
  }
  .kind {
    font-family: var(--font-mono);
    font-size: 12px;
    letter-spacing: 0.04em;
    color: var(--dim);
    min-width: 0;
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
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    margin: 0 0 16px;
  }
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
  .empty {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
    margin-top: 20px;
    padding: 24px;
    border: 1px dashed var(--ln);
    border-radius: var(--r-card);
  }
  .empty-title {
    margin: 0;
    font-family: var(--font-display);
    font-size: 20px;
    font-weight: 600;
  }
  .empty-text {
    margin: 0;
    color: var(--dim);
    font-size: 14px;
    line-height: 1.5;
    max-width: 52ch;
  }
  .empty .btn {
    margin-top: 6px;
  }
  .retry {
    display: flex;
    justify-content: center;
    margin-top: 16px;
  }

  @media (max-width: 640px) {
    .actions {
      flex-wrap: wrap;
      overflow: visible;
      padding-bottom: 2px;
    }
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
    .kicker {
      min-height: 44px;
      gap: 8px;
    }
    .back {
      width: 44px;
      height: 44px;
    }
    h1 {
      font-size: clamp(32px, 8.5vw, 40px);
    }
    .kind {
      line-height: 1.5;
    }
    .kind .prefix,
    .kind .total {
      /* The sleeve and the title already carry the band; on a phone those are
         the two facts that push the line to a second row. */
      display: none;
    }
    /* Same as the playlist: the search on a line of its own, then the sort. */
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
