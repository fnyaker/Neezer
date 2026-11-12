<script>
  import { entityRef } from "../lib/entity.js";
  import { api } from "../lib/api.js";
  import { REMOTE, CAN_KEEP, CAN_ADMIN, CAN_BROWSE } from "../lib/remote/mode.js";
  import { player, isAdmin, openGenreBulk } from "../lib/stores.js";
  import { toggleEntityFavorite, entityMeta } from "../lib/actions.js";
  import Cover from "../components/Cover.svelte";
  import Card from "../components/Card.svelte";
  import TrackList from "../components/TrackList.svelte";
  import PersonCredits from "../components/PersonCredits.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import LoadFailed from "../components/LoadFailed.svelte";
  import Icon from "../components/Icon.svelte";
  import { rememberScreen, recallScreen } from "../lib/nav.js";

  export let params = {};

  let id = null;
  let data = null;
  let disco = null;
  // Albums Deezer has and the library does not: null = loading, [] = none (or
  // Deezer unreachable — the library page above is complete on its own).
  let onDeezer = null;
  let deezerDown = false;
  let fav = false;
  let loading = true;
  let loadError = null; // why there is nothing to show (components/LoadFailed)

  // Two views on the artist page: the card-based overview and a flat, full
  // "Titres" list (all of the artist's tracks, loaded lazily on first open).
  let tab = "overview";
  let allTracks = null; // null = not loaded yet, [] = loaded/empty
  let tracksLoading = false;
  // Which tab we were on when we left this history entry. load() clears the tab
  // on every artist change, so it is consumed THERE (on the first load) rather
  // than applied here, where it would be overwritten a moment later.
  let restoredTab = recallScreen()?.tab || null;
  $: rememberScreen({ tab });

  $: if (params.id && params.id !== id) {
    id = params.id;
    load(id);
  }

  // Lazily fetch the full track list the first time the "Titres" tab is opened.
  $: if (tab === "tracks" && allTracks === null && !tracksLoading) loadTracks(id);

  async function loadTracks(artistId) {
    tracksLoading = true;
    try {
      const r = await api.artistTracks(artistId);
      if (artistId === id) allTracks = r.tracks || [];
    } catch {
      if (artistId === id) allTracks = [];
    }
    if (artistId === id) tracksLoading = false;
  }

  function playAllTracks() {
    if (allTracks?.length) player.playQueue(allTracks, 0, { kind: "artist", id });
  }

  // Sequence guard: a delayed earlier response (network / offline retries)
  // must not overwrite the page after a quick navigation to another artist.
  let loadSeq = 0;
  async function load(artistId) {
    const mine = ++loadSeq;
    loading = true;
    loadError = null;
    data = null;
    disco = null;
    onDeezer = null;
    deezerDown = false;
    fav = false;
    tab = restoredTab || "overview";
    restoredTab = null;
    allTracks = null;
    tracksLoading = false;
    try {
      // Cached copy first, fresh one right after — no skeleton on a revisit.
      await api.swr("/artist/" + artistId, (r) => {
        if (mine !== loadSeq) return;
        data = r;
        fav = !!r.artist?.is_favorite; // correct when the source reports it
        loading = false;
      });
      // After the page is on screen: what only Deezer has.
      if (mine === loadSeq && data?.library) loadOnDeezer(artistId, mine);
    } catch (e) {
      if (mine === loadSeq && !data) {
        data = null;
        loadError = e;
      }
    }
    if (mine === loadSeq) loading = false;
    api
      .discography(artistId)
      .then((r) => {
        if (mine === loadSeq) disco = r.discography;
      })
      .catch(() => {
        if (mine === loadSeq) disco = null;
      });
  }

  function loadOnDeezer(artistId, mine) {
    api
      .artistOnDeezer(artistId)
      .then((r) => {
        if (mine === loadSeq) onDeezer = r.albums || [];
      })
      .catch(() => {
        if (mine !== loadSeq) return;
        onDeezer = [];
        deezerDown = true;
      });
  }

  function playTop() {
    if (data?.top?.length) player.playQueue(data.top, 0, { kind: "artist", id });
  }

  async function startRadio() {
    try {
      const r = await api.artistRadio(id);
      player.playQueue(r.tracks, 0, { kind: "artist-radio", id });
    } catch {
      /* ignore */
    }
  }

  async function toggleFav() {
    const next = !fav;
    if (await toggleEntityFavorite("artist", id, next, entityMeta("artist", data?.artist || {}))) fav = next;
  }

  const TYPE_LABEL = { album: "Album", single: "Single", ep: "EP", compile: "Compilation" };
  function releaseSubtitle(r) {
    const type = TYPE_LABEL[r.record_type] || "Sortie";
    return r.year ? `${type} · ${r.year}` : type;
  }

  $: albums = disco?.album || data?.albums || [];
  $: singles = disco?.single || [];
  $: featured = disco?.featured || [];
  // The 5 most recent official releases (backend already sorts `all` by date).
  $: latest = ((disco?.all && disco.all.length ? disco.all : albums) || [])
    .slice(0, 5)
    .map((r) => ({ ...r, subtitle: releaseSubtitle(r) }));
  // One string, not a sentence split around {#if}s (the compiler trims the
  // spaces at a block's edges).
  $: facts = data?.artist
    ? [
        "Artiste",
        allTracks !== null
          ? `${allTracks.length} titre${allTracks.length > 1 ? "s" : ""}`
          : data.artist.nb_fan
            ? `${data.artist.nb_fan.toLocaleString("fr-FR")} fans`
            : null,
      ]
        .filter(Boolean)
        .join(" · ")
    : "";
</script>

{#if loading}
  <Skeleton kind="header" />
  <Skeleton kind="list" />
{:else if !data?.artist}
  <LoadFailed error={loadError} notFound="Artiste introuvable." on:retry={() => load(id)} />
{:else}
  <div class="fade-in">
    <!-- §6: a plain `--s2` band under a hairline, not a gradient. The photo is
         round and 168 px, the name set at 88 in Bricolage 800. -->
    <header class="banner gh">
      <div class="art"><Cover src={data.artist.picture} alt={data.artist.name} kind="artist" round eager /></div>
      <div class="meta">
        <span class="kind">{facts}</span>
        <h1>{data.artist.name}</h1>
        <div class="actions">
          <button class="btn btn-primary" on:click={playTop}><Icon name="play" size={15} /> Lire</button>
          <button class="btn btn-ghost" on:click={startRadio}><Icon name="radio" size={16} /> Radio</button>
          {#if CAN_KEEP}
            <button
              class="btn btn-icon"
              class:on={fav}
              on:click={toggleFav}
              aria-label="Suivre"
              title={fav ? "Ne plus suivre" : "Suivre"}><Icon name={fav ? "heartFilled" : "heart"} size={20} /></button>
          {/if}
          {#if $isAdmin && CAN_KEEP}
            <!-- Every track of this artist the library holds, reviewed first. -->
            <button
              class="btn btn-icon"
              on:click={() => openGenreBulk({ kind: "artist", artist: id, title: data.artist.name, cover: data.artist.picture })}
              aria-label="Donner un genre à l'artiste"
              title="Donner un genre à tous ses titres de la bibliothèque"><Icon name="tag" size={20} /></button>
          {/if}
        </div>
      </div>
    </header>

    <div class="tabs" role="tablist" aria-label="Sections de l'artiste">
      <button
        class="chip"
        class:chip-active={tab === "overview"}
        role="tab"
        aria-selected={tab === "overview"}
        on:click={() => (tab = "overview")}>Aperçu</button>
      <button
        class="chip"
        class:chip-active={tab === "tracks"}
        role="tab"
        aria-selected={tab === "tracks"}
        on:click={() => (tab = "tracks")}>Titres</button>
      <!-- What this artist wrote, composed or produced for other titles of the
           library: shown only when there is some. -->
      {#if data.credited}
        <button
          class="chip"
          class:chip-active={tab === "credits"}
          role="tab"
          aria-selected={tab === "credits"}
          on:click={() => (tab = "credits")}>Crédits</button>
      {/if}
    </div>

    {#if tab === "credits" && data.credited}
      <PersonCredits name={data.artist.name} />
    {:else if tab === "tracks"}
      {#if tracksLoading && allTracks === null}
        <Skeleton kind="list" />
      {:else if !allTracks?.length}
        <p class="muted">Aucun titre trouvé pour cet artiste.</p>
      {:else}
        <div class="bar">
          <button class="btn btn-primary" on:click={playAllTracks}><Icon name="play" size={15} /> Tout lire</button>
          <span class="count label-mono">{allTracks.length} titres</span>
        </div>
        <TrackList tracks={allTracks} context={{ kind: "artist", id }} numbered />
      {/if}
    {:else}
      {#if latest.length}
        <h2 class="section-title">Dernières sorties</h2>
        <div class="latest">{#each latest as a (entityRef(a))}<Card item={a} kind="album" />{/each}</div>
      {/if}

      {#if data.top?.length}
        <h2 class="section-title">Titres populaires</h2>
        <TrackList tracks={data.top.slice(0, 10)} numbered showAlbum={false} />
      {/if}

      {#if albums.length}
        <h2 class="section-title">Albums</h2>
        <div class="grid">{#each albums as a (entityRef(a))}<Card item={a} kind="album" />{/each}</div>
      {/if}

      {#if data.library && onDeezer === null}
        <h2 class="section-title">Disponible sur Deezer</h2>
        <Skeleton kind="shelf" count={5} />
      {:else if onDeezer?.length}
        <h2 class="section-title">Disponible sur Deezer</h2>
        <div class="grid">{#each onDeezer as a (entityRef(a))}<Card item={a} kind="album" />{/each}</div>
      {:else if deezerDown}
        <p class="muted down">Deezer est injoignable : le reste du catalogue n'est pas affiché.</p>
      {/if}

      {#if singles.length}
        <h2 class="section-title">Singles &amp; EP</h2>
        <div class="grid">{#each singles as a (entityRef(a))}<Card item={a} kind="album" />{/each}</div>
      {/if}

      {#if featured.length}
        <h2 class="section-title">Apparaît dans</h2>
        <div class="grid">{#each featured as a (entityRef(a))}<Card item={a} kind="album" />{/each}</div>
      {/if}

      {#if data.related?.length}
        <h2 class="section-title">Artistes similaires</h2>
        <div class="shelf">{#each data.related as a (entityRef(a))}<Card item={a} kind="artist" />{/each}</div>
      {/if}

      {#if data.bio}
        <h2 class="section-title">Biographie</h2>
        <p class="bio muted">{data.bio}</p>
      {/if}
    {/if}
  </div>
{/if}

<style>
  /* The band bleeds the page's own margins away (--page-y / --page-x), exactly
     as the old gradient header did, so the colour reaches both edges. */
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
    width: 168px;
    flex: none;
    border-radius: 50%;
    overflow: hidden;
  }
  .meta {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
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
    font-size: clamp(40px, 7.5vw, 88px);
    line-height: 0.95;
    letter-spacing: -0.04em;
    overflow-wrap: anywhere;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    margin-top: 2px;
  }
  .btn-icon.on {
    color: var(--hrt);
    border-color: var(--hrt);
  }
  .tabs {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    margin: 4px 0 24px;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 14px;
    flex-wrap: wrap;
    margin-bottom: 14px;
  }
  .count {
    font-size: 11px;
  }
  /* §6's "Dernières sorties" rail: six across, the caption under the sleeve. */
  .latest {
    display: grid;
    grid-template-columns: repeat(6, minmax(0, 1fr));
    gap: 16px;
  }
  .down {
    margin: 18px 0 0;
    font-size: 0.9rem;
  }
  .bio {
    line-height: 1.6;
    max-width: 70ch;
    white-space: pre-line;
  }
  @media (max-width: 1100px) {
    .latest {
      grid-template-columns: repeat(4, minmax(0, 1fr));
    }
  }
  @media (max-width: 820px) {
    .latest {
      grid-template-columns: repeat(3, minmax(0, 1fr));
    }
  }
  @media (max-width: 640px) {
    .actions {
      flex-wrap: nowrap;
      overflow-x: auto;
      scrollbar-width: none;
      padding-bottom: 2px;
    }
    .actions::-webkit-scrollbar {
      display: none;
    }
    /* §9: a short band, like the album's and the playlist's — a 104 px disc
       and the name beside it, not a portrait stacked above three lines of
       chrome. The name is 32–40 at a finger, the actions scroll on one row. */
    .banner {
      flex-direction: row;
      align-items: center;
      gap: 14px;
      padding: 16px var(--page-x);
    }
    .art {
      width: 104px;
    }
    .meta {
      gap: 6px;
    }
    h1 {
      font-size: clamp(32px, 8.5vw, 40px);
    }
    .kind {
      line-height: 1.5;
    }
    .latest {
      display: flex;
      gap: 12px;
      overflow-x: auto;
      padding-bottom: 4px;
      scroll-snap-type: x proximity;
    }
    .latest :global(.card) {
      flex: none;
      width: 148px;
      scroll-snap-align: start;
    }
  }
</style>
