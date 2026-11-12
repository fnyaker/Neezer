<script>
  // Explorer (the screen the nav used to call "Rechercher"). DESIGN §6: a title, the
  // sources a search reaches, the searches this device remembers, and the style
  // families as tiles — never an empty page when nothing is typed yet.
  //
  // The search itself is unchanged: lib/searchrank.js decides the best result,
  // the library answers first and the main provider joins it, and a pasted
  // playlist link becomes an import offer (lib/linkimport.js).
  import { onDestroy } from "svelte";
  import { api } from "../lib/api.js";
  import { isAdmin } from "../lib/stores.js";
  import { rememberScreen, recallScreen } from "../lib/nav.js";
  import Card from "../components/Card.svelte";
  import PodcastCard from "../components/PodcastCard.svelte";
  import TrackList from "../components/TrackList.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import Icon from "../components/Icon.svelte";
  import GenreTiles from "../components/GenreTiles.svelte";
  import { fallbackSources, mainSource, iconOf } from "../lib/sources.js";
  import { access } from "../lib/access.js";
  import { playlistLink, importRoute } from "../lib/linkimport.js";
  import { entityRef } from "../lib/entity.js";
  import { topResult, mixPodcasts } from "../lib/searchrank.js";
  import TopResult from "../components/TopResult.svelte";

  export let params = {};

  const EMPTY = { artists: [], albums: [], tracks: [], playlists: [], podcasts: [] };

  // Coming BACK here (from an album you opened out of the results, say) must
  // land you on the screen you left — query, tab, results and all — not on an
  // empty search page. lib/nav.js keys that on the history entry, so a fresh
  // tap on "Rechercher" in the nav still starts clean.
  const saved = recallScreen();

  let q = saved?.q ?? "";
  let results = saved?.results ?? EMPTY;
  let loading = false;
  let tab = saved?.tab ?? "all"; // all | tracks | artists | albums | playlists
  // library = what a search always reaches (the library + the main source);
  // podcasts = the shows only, for when that is what you are after.
  let scope = saved?.scope ?? "library";
  let timer;
  let seq = 0;
  let inputEl;
  // The term the last request was actually sent for. On leaving the screen,
  // only a term that WAS searched is remembered (never half a word typed).
  let searchedFor = saved?.q ?? "";
  // Don't pop the soft keyboard over a restored screen: you came back to read
  // the results, not to retype the query.
  const autofocusBox = !q;

  // -- recent searches --------------------------------------------------------
  // The last five terms this device actually searched, newest first, in
  // localStorage (`search.recent`, JSON) — a bare array of strings, deduped
  // case-insensitively. Length is capped on read too, so a hand-edited or
  // older value cannot flood the screen.
  const RECENT_KEY = "search.recent";
  const RECENT_MAX = 5;
  function readRecent() {
    try {
      const v = JSON.parse(localStorage.getItem(RECENT_KEY) || "[]");
      if (!Array.isArray(v)) return [];
      const out = [];
      for (const x of v) {
        if (typeof x !== "string") continue;
        const t = x.trim();
        if (!t || out.some((y) => y.toLowerCase() === t.toLowerCase())) continue;
        out.push(t);
        if (out.length >= RECENT_MAX) break;
      }
      return out;
    } catch {
      return [];
    }
  }
  let recent = readRecent();
  function noteSearch(term) {
    const t = String(term || "").trim();
    if (t.length < 2 || playlistLink(t)) return;
    const next = [t, ...recent.filter((x) => x.toLowerCase() !== t.toLowerCase())].slice(0, RECENT_MAX);
    recent = next;
    try {
      localStorage.setItem(RECENT_KEY, JSON.stringify(next));
    } catch {
      /* a convenience */
    }
  }
  function clearRecent() {
    recent = [];
    try {
      localStorage.removeItem(RECENT_KEY);
    } catch {
      /* a convenience */
    }
  }
  function useRecent(term) {
    clearTimeout(timer);
    q = term;
    scope = "library";
    // A tile or a remembered term is a search this person chose: keep it at
    // once, rather than only when they leave the screen.
    noteSearch(term);
    run(term);
    inputEl?.focus();
  }

  // The first fallback source that can be searched (Réglages → Sources, in
  // the admin's order): off unless ticked, on this screen only (a new search
  // starts with it off again). Asked on Enter or when the chip is pressed
  // — never per keystroke: every partial word typed would be a request there,
  // and the server keeps each answer a day, so a search repeated costs nothing.
  let yt = saved?.yt ?? false;
  let ytTracks = saved?.ytTracks ?? [];
  let ytFor = saved?.ytFor ?? "";
  let ytLoading = false;
  let ytError = "";
  let ytSeq = 0;
  // How each fallback source is searched from here; one without an entry is
  // not offered.
  const SEARCHERS = { youtube: (term) => api.youtubeSearch(term) };
  $: fb = $fallbackSources.find((f) => SEARCHERS[f.id] && $access.can(f.id, "search")) || null;
  $: ytAvailable = !!fb;
  $: ytStale = yt && ytAvailable && q.trim() !== ytFor;

  async function runYoutube() {
    const term = q.trim();
    // A pasted playlist link is an import, not something to search YouTube for.
    if (!yt || !ytAvailable || !term || term === ytFor || playlistLink(term)) return;
    const mine = ++ytSeq;
    ytLoading = true;
    ytError = "";
    try {
      const r = await SEARCHERS[fb.id](term);
      if (mine !== ytSeq) return;
      ytTracks = r.tracks || [];
      ytFor = term;
    } catch (e) {
      if (mine !== ytSeq) return;
      ytTracks = [];
      ytError =
        e?.status === 503 ? `${fb.name} ne répond pas pour le moment.` : `La recherche sur ${fb.name} a échoué.`;
    } finally {
      if (mine === ytSeq) ytLoading = false;
    }
  }

  function toggleYoutube() {
    yt = !yt;
    if (yt) runYoutube();
  }

  // The library is what every search reaches; the fallback chip adds a source
  // to it, the Podcasts chip narrows the screen to the shows. Pressing the
  // library chip comes back to the full answer.
  function resetScope() {
    scope = "library";
    inputEl?.focus();
  }
  function toggleShows() {
    scope = scope === "podcasts" ? "library" : "podcasts";
  }

  function onKey(e) {
    if (e.key !== "Enter") return;
    clearTimeout(timer);
    const term = q.trim();
    // Enter is "search now": the regular search at once, YouTube if ticked.
    run(term);
    runYoutube();
    noteSearch(term);
    e.currentTarget.blur(); // drop the phone keyboard over the results
  }

  // Whatever the screen is showing, ready for the next time we come back to it.
  $: rememberScreen({ q, tab, results, yt, ytTracks, ytFor, scope });

  // Deep link support (/search/:q from the sidebar).
  //
  // The gate is the last param we APPLIED — never the live input value. This
  // block re-runs whenever anything it reads changes, and `q` is one of those:
  // comparing against `q` meant every keystroke re-ran the block, found the
  // typed text different from the URL term and reset the box back to it. The
  // search field was effectively frozen on the deep-linked query.
  //
  // A restored screen has already applied its query: treat the URL term as
  // consumed, or the restore would be overwritten by a redundant refetch.
  let appliedParam = saved ? (params.q ?? null) : null;
  $: applyParam(params.q);
  function applyParam(raw) {
    if (raw === undefined || raw === appliedParam) return;
    appliedParam = raw;
    let decoded = raw;
    try {
      decoded = decodeURIComponent(raw); // a malformed %xx would throw
    } catch {
      /* keep the raw value */
    }
    q = decoded;
    run(q.trim());
  }

  // Live search as you type, debounced. We deliberately do NOT change the route
  // on each keystroke (that would tear down & rebuild this component).
  function onInput() {
    clearTimeout(timer);
    const term = q.trim();
    timer = setTimeout(() => run(term), 280);
  }

  async function run(term) {
    const mine = ++seq;
    if (!term || playlistLink(term)) {
      results = EMPTY;
      loading = false;
      searchedFor = "";
      return;
    }
    searchedFor = term;
    loading = true;
    try {
      const r = await api.search(term);
      if (mine !== seq) return; // a newer query already started
      results = { ...EMPTY, ...r };
    } catch {
      if (mine === seq) results = EMPTY;
    } finally {
      if (mine === seq) loading = false;
    }
  }

  onDestroy(() => {
    clearTimeout(timer);
    // A term the person actually searched is worth remembering; a half-typed
    // word the box still holds is not.
    if (searchedFor && searchedFor === q.trim()) noteSearch(searchedFor);
  });

  // A saved scope from an older session, or from an account that is no longer
  // the admin: the shows are not served to it, so the filter has nothing to
  // show — fall back to the library rather than an empty screen.
  $: if (scope === "podcasts" && !$isAdmin) scope = "library";

  $: hasResults =
    results.artists?.length ||
    results.albums?.length ||
    results.tracks?.length ||
    results.playlists?.length ||
    (results.podcasts?.length && $isAdmin);

  // A pasted playlist link is not a search: offer to import it instead.
  $: shared = playlistLink(q);

  // "Tout" leads with what a search is usually for: the artist the query
  // plainly names (only then — lib/searchrank.js), the titles, then the
  // artists, then the albums with the shows mixed in under a small "Podcast"
  // label, then the playlists. Ten titles is the most that still reads as a
  // shortlist on a phone; the rest is one tap away on the Titres tab. An old
  // saved "podcasts" tab lands on the albums, where the shows are now.
  $: if (tab === "podcasts") tab = "albums";
  $: podcasts = $isAdmin ? results.podcasts || [] : [];
  $: shows = scope === "podcasts" ? podcasts : [];
  $: top = tab === "all" && scope === "library" ? topResult(q, results, $isAdmin) : null;
  $: shelfArtists = (results.artists || []).filter((a) => !(top?.kind === "artist" && a === top.item));
  $: albumsAndShows = mixPodcasts(
    results.albums || [],
    podcasts.filter((p) => !(top?.kind === "podcast" && p === top.item)),
  );

  const TOP_TRACKS = 10;
  $: topTracks =
    tab === "all" ? (results.tracks || []).slice(0, TOP_TRACKS) : results.tracks || [];
  $: moreTracks = Math.max(0, (results.tracks?.length || 0) - TOP_TRACKS);
</script>

<h1 class="page-title">Explorer</h1>

<div class="sources" role="group" aria-label="Source de la recherche">
  <button
    class="chip"
    class:chip-active={scope === "library"}
    aria-pressed={scope === "library"}
    on:click={resetScope}
    title={$mainSource ? `La bibliothèque et ${$mainSource.name}` : "Toute ta bibliothèque"}
  >
    Ma bibliothèque
  </button>
  {#if ytAvailable}
    <button
      class="chip"
      class:chip-active={yt}
      aria-pressed={yt}
      title="Chercher aussi sur {fb.name} (source de secours)"
      on:click={toggleYoutube}
    >
      <Icon name={iconOf(fb.id)} size={15} /> {fb.name}
    </button>
  {/if}
  {#if $isAdmin}
    <button
      class="chip"
      class:chip-active={scope === "podcasts"}
      aria-pressed={scope === "podcasts"}
      on:click={toggleShows}
      title="Ne montrer que les podcasts"
    >
      Podcasts
    </button>
  {/if}
</div>

<div class="bar">
  <!-- svelte-ignore a11y-autofocus -->
  <input
    bind:this={inputEl}
    type="search"
    enterkeyhint="search"
    placeholder="Titres, artistes, albums, podcasts…"
    aria-label="Rechercher un titre, un artiste, un album ou un podcast"
    bind:value={q}
    on:input={onInput}
    on:keydown={onKey}
    autofocus={autofocusBox}
  />
</div>

{#if shared}
  <a class="shared fade-in" href={importRoute(shared.url)}>
    <span class="s-icon"><Icon name="link" size={20} /></span>
    <span class="s-text">
      <strong>Importer cette playlist {shared.label}</strong>
      <span class="muted">Retrouver ses titres pour les écouter ici, et la garder si tu veux.</span>
    </span>
  </a>
{/if}

{#if !q.trim() && !shared}
  <div class="landing">
    {#if recent.length}
      <section class="recents">
        <div class="section-head">
          <h2 class="section-title">Récents</h2>
          <button class="see-all" on:click={clearRecent}>Effacer</button>
        </div>
        <div class="recent-chips">
          {#each recent as term (term)}
            <button class="chip" on:click={() => useRecent(term)} title="Rechercher {term}">
              <Icon name="search" size={14} /> {term}
            </button>
          {/each}
        </div>
      </section>
    {:else}
      <p class="muted lead">
        Cherche un titre, un artiste, un album ou un podcast — les titres de ta bibliothèque
        d'abord, puis le catalogue.
      </p>
    {/if}
    <GenreTiles on:pick={(e) => useRecent(e.detail)} />
  </div>
{/if}

{#if q.trim() && !shared && scope === "library"}
  <div class="tabs" role="tablist">
    {#each [["all", "Tout"], ["tracks", "Titres"], ["artists", "Artistes"], ["albums", "Albums"], ["playlists", "Playlists"]] as [id, label]}
      <button role="tab" aria-selected={tab === id} class="chip" class:chip-active={tab === id} on:click={() => (tab = id)}>
        {label}
      </button>
    {/each}
  </div>
{/if}

{#if scope === "podcasts"}
  {#if loading}
    <Skeleton kind="shelf" />
  {:else if q.trim() && !shared}
    {#if shows.length}
      <div class="grid">
        {#each shows as p (entityRef(p))}<PodcastCard item={p} />{/each}
      </div>
    {:else}
      <p class="muted hint">Aucun podcast pour « {q} ».</p>
    {/if}
  {/if}
{:else if loading}
  <Skeleton kind="shelf" />
  <Skeleton kind="list" />
{:else if !q.trim()}
  <!-- the recent searches and the style tiles are above: never an empty page -->
{:else if !hasResults}
  {#if !(yt && ytAvailable) && !shared}
    <p class="muted hint">Aucun résultat pour « {q} ».</p>
  {/if}
{:else}
  <div class="leadwrap"><div class="lead" class:withtop={!!top}>
    {#if top}
      <section class="topcol">
        <h2>Meilleur résultat</h2>
        <TopResult kind={top.kind} item={top.item} />
      </section>
    {/if}
    {#if (tab === "all" || tab === "tracks") && results.tracks?.length}
      <section class="trackcol">
        <div class="sechead">
          <h2>Titres</h2>
          {#if tab === "all" && moreTracks}
            <button class="seeall" on:click={() => (tab = "tracks")}>
              Tout afficher<span class="count">{results.tracks.length}</span>
            </button>
          {/if}
        </div>
        <TrackList tracks={topTracks} radio />
      </section>
    {/if}
  </div></div>
{/if}

{#if scope === "library" && !loading && q.trim() && !shared && yt && ytAvailable && (tab === "all" || tab === "tracks")}
  <h2 class="ythead"><Icon name={iconOf(fb.id)} size={20} /> Sur {fb.name}</h2>
  {#if ytTracks.length && !ytStale && !ytLoading}
    <p class="ytnote muted">Archivé à la première écoute{fb.quality ? `, en ${fb.quality} au mieux (ce que ${fb.name} propose de mieux)` : ""}.</p>
  {/if}
  {#if ytLoading}
    <Skeleton kind="list" />
  {:else if ytStale}
    <button class="ytgo" on:click={runYoutube}>
      <Icon name="search" size={15} /> Chercher « {q.trim()} » sur {fb.name}
    </button>
  {:else if ytError}
    <p class="muted hint small">{ytError}</p>
  {:else if !ytTracks.length}
    <p class="muted hint small">Rien sur {fb.name} pour « {ytFor} ».</p>
  {:else}
    <TrackList tracks={ytTracks} />
  {/if}
{/if}

{#if scope === "library" && !loading && q.trim() && hasResults}

  {#if (tab === "all" || tab === "artists") && shelfArtists.length}
    <div class="sechead">
      <h2>Artistes</h2>
      {#if tab === "all" && shelfArtists.length > 1}
        <button class="seeall" on:click={() => (tab = "artists")}>Tout afficher</button>
      {/if}
    </div>
    <div class={tab === "artists" ? "grid" : "shelf"}>
      {#each shelfArtists as a (entityRef(a))}<Card item={a} kind="artist" />{/each}
    </div>
  {/if}

  {#if (tab === "all" || tab === "albums") && albumsAndShows.length}
    <div class="sechead">
      <h2>Albums</h2>
      {#if tab === "all" && albumsAndShows.length > 1}
        <button class="seeall" on:click={() => (tab = "albums")}>Tout afficher</button>
      {/if}
    </div>
    <div class={tab === "albums" ? "grid" : "shelf"}>
      {#each albumsAndShows as a ((a.podcast ? "p" : "a") + entityRef(a))}
        {#if a.podcast}<PodcastCard item={a} />{:else}<Card item={a} kind="album" />{/if}
      {/each}
    </div>
  {/if}

  {#if (tab === "all" || tab === "playlists") && results.playlists?.length}
    <div class="sechead">
      <h2>Playlists</h2>
      {#if tab === "all" && results.playlists.length > 1}
        <button class="seeall" on:click={() => (tab = "playlists")}>Tout afficher</button>
      {/if}
    </div>
    <div class={tab === "playlists" ? "grid" : "shelf"}>
      {#each results.playlists as p (entityRef(p))}<Card item={p} kind="playlist" />{/each}
    </div>
  {/if}
{/if}

<style>
  .sources {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin: 16px 0 14px;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .bar input {
    flex: 1 1 180px;
    min-width: 0;
    max-width: 560px;
    height: 44px;
    padding: 0 18px;
    border-radius: 999px;
    border: 1px solid var(--ln);
    background: var(--s1);
    color: var(--tx);
    font-size: 15px;
  }
  .bar input::placeholder {
    color: var(--dim);
    opacity: 1;
  }
  /* One ring, the app's: the accent outline from app.css, hugging the border
     (a 2 px outline offset from a 1 px border read as a double ring). */
  .bar input:focus-visible {
    outline-offset: 0;
  }
  /* Hide the browser's own clear cross: the box is the same pill everywhere. */
  .bar input::-webkit-search-cancel-button {
    -webkit-appearance: none;
  }
  .lead {
    margin: 4px 0 0;
    max-width: 62ch;
    line-height: 1.45;
  }
  .landing {
    display: flex;
    flex-direction: column;
    gap: 26px;
    margin-top: 24px;
  }
  .recents {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .recent-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .see-all {
    padding: 0;
    border: none;
    background: none;
    color: var(--dim);
    font-size: 13px;
    font-weight: 600;
  }
  .see-all:hover {
    color: var(--tx);
  }
  .ythead {
    display: flex;
    align-items: center;
    gap: 9px;
  }
  /* The note sits under the heading, not beside it: beside it, a phone broke
     "Sur YouTube" over two lines to make room. */
  .ytnote {
    margin: 0 0 10px;
    font-size: 0.8rem;
  }
  .ytgo {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    margin: 4px 0 8px;
    padding: 10px 18px;
    border-radius: 999px;
    border: 1px solid var(--ln);
    background: var(--s1);
    color: var(--tx);
    font-weight: 600;
    font-size: 0.9rem;
  }
  .ytgo:hover {
    background: var(--s2);
  }
  .hint.small {
    margin-top: 6px;
  }
  .shared {
    display: flex;
    align-items: center;
    gap: 14px;
    max-width: 560px;
    margin: 16px 0 4px;
    padding: 14px 18px;
    border-radius: var(--r-card);
    border: 1px solid var(--ln);
    background: var(--s1);
    color: var(--tx);
    text-decoration: none;
  }
  .shared:hover {
    background: var(--s2);
  }
  .s-icon {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    flex: none;
    border-radius: 50%;
    background: var(--ac);
    color: var(--onac);
  }
  .s-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
    font-size: 0.9rem;
  }
  /* One row of chips that scrolls sideways on a phone rather than wrapping
     to a second line (Material 3's filter chips). */
  .tabs {
    display: flex;
    gap: 8px;
    margin: 18px 0 4px;
    padding: 2px;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .tabs::-webkit-scrollbar {
    display: none;
  }
  .hint {
    margin-top: 20px;
  }
  /* The best result beside the titles where there is room for both (the way
     Spotify and YouTube Music lay it out), above them on a phone. */
  .leadwrap {
    container-type: inline-size;
  }
  .lead.withtop {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 4px 28px;
  }
  @container (min-width: 860px) {
    .lead.withtop {
      grid-template-columns: minmax(260px, 340px) minmax(0, 1fr);
      align-items: start;
    }
  }
  .lead section {
    min-width: 0;
  }
  /* The section title and its "tout afficher" share a baseline: the link sits
     at the end of the row rather than under the heading, so the eye reads
     "Titres … tout afficher" as one line and the shelves below stay flush. */
  .sechead {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 16px;
  }
  .seeall {
    display: inline-flex;
    align-items: baseline;
    gap: 7px;
    flex: none;
    padding: 0;
    color: var(--dim);
    font-size: 13px;
    font-weight: 600;
  }
  .seeall:hover {
    color: var(--tx);
  }
  .seeall .count {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
    font-size: 11px;
    font-weight: 500;
    color: var(--dim);
    background: var(--s2);
    border-radius: 999px;
    padding: 2px 7px;
  }
  .seeall:hover .count {
    color: var(--tx);
  }
  @media (max-width: 640px) {
    .sources {
      flex-wrap: nowrap;
      overflow-x: auto;
      scrollbar-width: none;
      margin-right: calc(-1 * var(--page-x, 20px));
      padding-right: var(--page-x, 20px);
    }
    .sources::-webkit-scrollbar {
      display: none;
    }
  }
</style>
