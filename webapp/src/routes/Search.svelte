<script>
  // Explorer (the screen the nav used to call "Rechercher"). DESIGN §6: a title, the
  // sources a search reaches, the searches this device remembers, and the style
  // families as tiles — never an empty page when nothing is typed yet.
  //
  // The search itself is unchanged: lib/searchrank.js decides the best result,
  // the library answers first and the main provider joins it, and a pasted
  // playlist link becomes an import offer (lib/linkimport.js).
  import { onDestroy, onMount } from "svelte";
  import { get } from "svelte/store";
  import { api } from "../lib/api.js";
  import { isAdmin, player, toasts, seekTo, immersiveOpen } from "../lib/stores.js";
  import { rememberScreen, recallScreen } from "../lib/nav.js";
  import Card from "../components/Card.svelte";
  import PodcastCard from "../components/PodcastCard.svelte";
  import TrackList from "../components/TrackList.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import LoadFailed from "../components/LoadFailed.svelte";
  import Icon from "../components/Icon.svelte";
  import ExplorerStyles from "../components/ExplorerStyles.svelte";
  import { fallbackSources, mainSource, iconOf } from "../lib/sources.js";
  import { access } from "../lib/access.js";
  import { playlistLink, importRoute } from "../lib/linkimport.js";
  import { entityRef } from "../lib/entity.js";
  import { compactNumber } from "../lib/format.js";
  import { topResult, mixPodcasts, rankSearchResults } from "../lib/searchrank.js";
  import {
    countSearchResults,
    createRadioLauncher,
    offerRadio,
  } from "../lib/queryradio.js";
  import TopResult from "../components/TopResult.svelte";
  import TranscriptHits from "../components/TranscriptHits.svelte";
  import { online } from "../lib/net.js";
  import { trackKey } from "../lib/trackkey.js";
  import { setResumePoint } from "../lib/podcastProgress.js";
  import {
    hitEpisode,
    loadTranscribeStatus,
    requestEpisodePanel,
    transcribeStatus,
  } from "../lib/transcript.js";
  import { cacheStats, searchCached } from "../lib/transcriptcache.js";
  // The plan's transitions (docs/agents/transitions.md, I10): the results area
  // enters once per visit, the query-radio offer rises, the tabs are I2/I3, and
  // nothing animates per keystroke (M6).
  import { fadeIn, rise } from "../lib/motion.js";

  export let params = {};

  const EMPTY = { artists: [], albums: [], tracks: [], playlists: [], podcasts: [] };

  // -- the entrance of the results area (I10, M6) -----------------------------
  // Svelte plays an intro again every time its block is RE-CREATED, and this
  // screen re-creates the results behind the Skeleton branch on every settled
  // search — unfiltered that is one fade per keystroke, which M6 forbids. So
  // the entrance is ARMED by the two changes that mean "something new is on
  // screen": the first answer of the visit, and a tab switch (I2). Every answer
  // re-states it, so a replacement INSIDE the area stays instant.
  let libraryIn = true; // read by `in:fadeIn` at the moment the block mounts
  let libraryShown = false;
  function noteLibraryReveal(payload) {
    const any =
      payload.artists?.length ||
      payload.albums?.length ||
      payload.tracks?.length ||
      payload.playlists?.length ||
      payload.podcasts?.length;
    libraryIn = !!any && !libraryShown;
    if (any) libraryShown = true;
  }
  // The transcribed passages are a second region with its own first answer.
  let hitsIn = true;
  let hitsShown = false;
  function noteHitsReveal(list) {
    const any = !!list?.length;
    hitsIn = any && !hitsShown;
    if (any) hitsShown = true;
  }
  // I2: the panel a tab switch brings in fades in (opacity only: panels differ
  // in height, so a translate would shift what is below them).
  function pickTab(id) {
    if (tab === id) return;
    libraryIn = true;
    tab = id;
  }

  // Coming BACK here (from an album you opened out of the results, say) must
  // land you on the screen you left — query, tab, results and all — not on an
  // empty search page. lib/nav.js keys that on the history entry, so a fresh
  // tap on "Rechercher" in the nav still starts clean.
  const saved = recallScreen();

  let q = saved?.q ?? "";
  let results = saved?.results ?? EMPTY;
  let loading = false;
  let loadError = null;
  let tab = saved?.tab ?? "all"; // all | tracks | artists | albums | playlists
  // library = what a search always reaches (the library + the main source);
  // podcasts = the shows only, for when that is what you are after.
  let scope = saved?.scope ?? "library";
  let podsTab = saved?.podsTab ?? "passages";
  let trHits = saved?.trHits ?? [];
  let trTotal = saved?.trTotal ?? 0;
  let trScope = saved?.trScope ?? null;
  let trMore = saved?.trMore ?? false;
  let trFor = saved?.trFor ?? "";
  let trAttempt = "";
  let trPending = "";
  let trLoading = false;
  let trError = saved?.trError ?? null;
  let trLocal = saved?.trLocal ?? false;
  let trTimer = null;
  let trSeq = 0;
  let trRetryLeft = 0;
  let trRetryTimer = null;
  let cachedPages = 0;
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
    // once, rather than only when they leave the screen, and ask the fallback
    // without waiting for the settle beat.
    noteSearch(term);
    run(term);
    runYoutube();
    inputEl?.focus();
  }

  // The first fallback source that can be searched (Réglages → Sources, in
  // the admin's order): ON by default, because a search that silently leaves
  // out what the server can find is not searching. The chip turns it off for
  // this screen, and Réglages → Sources turns the source itself off for good;
  // an account without `youtube:search` sees no chip and no request.
  //
  // Asked once the query stops moving (a longer beat than the library's
  // 280 ms) — never per keystroke: every partial word typed would be a
  // request there, and the server keeps each answer a day, so a search
  // repeated costs nothing. Enter, a recent term and a deep link ask at once.
  let yt = saved?.yt ?? true;
  let ytTracks = saved?.ytTracks ?? [];
  let ytFor = saved?.ytFor ?? "";
  let ytLoading = false;
  let ytError = saved?.ytError ?? "";
  let ytErrorFor = saved?.ytErrorFor ?? "";
  // The term a request was last SENT for. A failure is answered too: it must
  // not rearm the timer every time the error line changes (a retry storm).
  let ytAttemptFor = "";
  let ytSeq = 0;
  let ytTimer;
  const YT_DELAY = 800;
  // How each fallback source is searched from here; one without an entry is
  // not offered.
  const SEARCHERS = { youtube: (term) => api.youtubeSearch(term) };
  $: fb = $fallbackSources.find((f) => SEARCHERS[f.id] && $access.can(f.id, "search")) || null;
  $: ytAvailable = !!fb;
  $: ytTerm = q.trim();
  $: ytErrorHere = !!ytError && ytErrorFor === ytTerm;
  $: ytAnswerHere = !!ytTerm && ytFor === ytTerm;
  // Waiting for the fallback: the query is ahead of what it answered, or the
  // request is in flight. Both are states the screen says out loud.
  $: ytWaiting =
    !!yt && ytAvailable && !!ytTerm && !playlistLink(ytTerm) && !ytAnswerHere && !ytErrorHere;

  async function runYoutube(force = false) {
    if (scope !== "library") return;
    const term = q.trim();
    // A pasted playlist link is an import, not something to search YouTube for.
    if (!yt || !ytAvailable || !term || playlistLink(term)) return;
    if (!force && (term === ytFor || (term === ytAttemptFor && ytLoading))) return;
    const mine = ++ytSeq;
    ytAttemptFor = term;
    ytLoading = true;
    ytError = "";
    ytErrorFor = "";
    try {
      const r = await SEARCHERS[fb.id](term);
      if (mine !== ytSeq) return;
      ytTracks = r.tracks || [];
      ytFor = term;
    } catch (e) {
      if (mine !== ytSeq) return;
      ytTracks = [];
      // The term is answered (with a failure): the same query must not ask
      // again on its own, but the Retry button below can.
      ytFor = term;
      ytErrorFor = term;
      ytError =
        e?.status === 403
          ? `Vous n'avez pas accès à ${fb.name} sur ce serveur.`
          : e?.status === 503
            ? `${fb.name} ne répond pas pour le moment.`
            : `La recherche sur ${fb.name} a échoué.`;
    } finally {
      if (mine === ytSeq) ytLoading = false;
    }
  }

  // One settled query, one fallback request. The timer is cleared on every
  // change (including the query going empty), and a term already answered or
  // attempted is never re-asked here. The dependency on `ytAnswerHere` keeps
  // a late response from resetting a timer that is already armed for the
  // query now in the box.
  $: {
    clearTimeout(ytTimer);
    if (
      scope === "library" &&
      yt &&
      ytAvailable &&
      ytTerm &&
      !playlistLink(ytTerm) &&
      !ytAnswerHere &&
      !ytErrorHere &&
      ytTerm !== ytAttemptFor
    ) {
      ytTimer = setTimeout(() => runYoutube(), YT_DELAY);
    }
  }

  function toggleYoutube() {
    yt = !yt;
    // Turning it back on retries a failed term rather than showing the error
    // again; an already answered term is not re-requested.
    if (yt) runYoutube(ytErrorHere);
  }

  // The library is what every search reaches; the fallback chip adds a source
  // to it, the Podcasts chip narrows the screen to the shows. Pressing the
  // library chip comes back to the full answer.
  function resetScope() {
    const term = q.trim();
    if (scope === "podcasts" && term && term !== searchedFor) run(term);
    scope = "library";
    inputEl?.focus();
  }
  function toggleShows() {
    scope = scope === "podcasts" ? "library" : "podcasts";
  }

  function onKey(e) {
    if (e.key !== "Enter") return;
    clearTimeout(timer);
    clearTimeout(trTimer);
    const term = q.trim();
    if (scope !== "podcasts" || $isAdmin) run(term);
    runYoutube();
    if (scope === "podcasts") runTranscriptSearch(term, true);
    noteSearch(term);
    e.currentTarget.blur(); // drop the phone keyboard over the results
  }

  // Whatever the screen is showing, ready for the next time we come back to it.
  $: rememberScreen({
    q,
    tab,
    results,
    yt,
    ytTracks,
    ytFor,
    ytError,
    ytErrorFor,
    scope,
    podsTab,
    trHits,
    trTotal,
    trScope,
    trMore,
    trFor,
    trLocal,
    trError,
  });

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
    // A deep link is a search already chosen: ask the fallback at once too.
    runYoutube();
  }

  // Live search as you type, debounced. We deliberately do NOT change the route
  // on each keystroke (that would tear down & rebuild this component).
  function onInput() {
    clearTimeout(timer);
    const term = q.trim();
    timer = setTimeout(() => {
      if (scope !== "podcasts" || $isAdmin) run(term);
      else loading = false;
    }, 280);
  }

  function resetTranscriptSearch() {
    clearTimeout(trTimer);
    clearTimeout(trRetryTimer);
    trRetryTimer = null;
    trSeq++;
    trPending = "";
    trHits = [];
    trTotal = 0;
    trScope = null;
    trMore = false;
    trAttempt = "";
    trLoading = false;
    trError = null;
    trLocal = false;
    trRetryLeft = 0;
  }

  function startTranscriptRetry(seconds) {
    clearInterval(trRetryTimer);
    trRetryLeft = Math.max(1, Math.ceil(Number(seconds) || 0));
    trRetryTimer = setInterval(() => {
      trRetryLeft = Math.max(0, trRetryLeft - 1);
      if (trRetryLeft === 0) {
        clearInterval(trRetryTimer);
        trRetryTimer = null;
      }
    }, 1000);
  }

  async function localTranscriptSearch(term, mine) {
    const local = await searchCached(term, 20);
    if (mine !== trSeq) return;
    trHits = local.hits || [];
    trTotal = Number(local.total) || trHits.length;
    trScope = null;
    trMore = false;
    trError = null;
    trLocal = true;
    trFor = term;
    trLoading = false;
    noteHitsReveal(trHits);
  }

  async function runTranscriptSearch(term, force = false) {
    if (!term || term.length < 2) {
      resetTranscriptSearch();
      return;
    }
    if (!force && (term === trFor || (term === trAttempt && trLoading))) return;
    if (trRetryLeft > 0 && !force) return;
    clearTimeout(trTimer);
    const mine = ++trSeq;
    trAttempt = term;
    trLoading = true;
    trError = null;
    trLocal = false;
    if (!$online) {
      await localTranscriptSearch(term, mine);
      return;
    }
    try {
      const payload = await api.searchTranscripts(term, 20);
      if (mine !== trSeq) return;
      trHits = payload.hits || [];
      trTotal = Number(payload.total) || trHits.length;
      trScope = payload.scope || null;
      trMore = !!payload.more;
      trError = null;
      trLocal = false;
      trFor = term;
      noteHitsReveal(trHits);
    } catch (error) {
      if (mine !== trSeq) return;
      if (error?.status === 0 || !$online) {
        await localTranscriptSearch(term, mine);
        return;
      }
      trHits = [];
      trTotal = 0;
      trScope = null;
      trMore = false;
      trError = error;
      trFor = term;
      const retry = Number(error?.retryIn || error?.body?.retry_in || 0);
      if (error?.status === 429 && retry > 0) startTranscriptRetry(retry);
    } finally {
      if (mine === trSeq) trLoading = false;
    }
  }

  function pickPassage(hit) {
    const episode = hitEpisode(hit);
    if (!episode) return;
    const start = Number(hit.start) || 0;
    const current = get(player);
    const playing = current.queue[current.index];
    if (playing && trackKey(playing) === trackKey(episode)) {
      player.setProgress(start, episode.duration || 0);
      seekTo.set(start);
      player.play();
    } else {
      setResumePoint(trackKey(episode), start, episode.duration || 0);
      player.playQueue([episode], 0, { kind: "podcast", id: episode.channel_id });
    }
    requestEpisodePanel(episode.id, "transcript", start);
    immersiveOpen.set(true);
  }

  async function run(term) {
    const mine = ++seq;
    if (!term || playlistLink(term)) {
      results = EMPTY;
      loading = false;
      searchedFor = "";
      loadError = null;
      return;
    }
    searchedFor = term;
    loading = true;
    loadError = null;
    try {
      const r = await api.search(term);
      if (mine !== seq) return; // a newer query already started
      results = { ...EMPTY, ...r };
      noteLibraryReveal(results);
    } catch (e) {
      if (mine === seq) {
        results = EMPTY;
        loadError = e;
      }
    } finally {
      if (mine === seq) loading = false;
    }
  }

  onDestroy(() => {
    clearTimeout(timer);
    clearTimeout(ytTimer);
    clearTimeout(trTimer);
    clearTimeout(trRetryTimer);
    // A term the person actually searched is worth remembering; a half-typed
    // word the box still holds is not.
    if (searchedFor && searchedFor === q.trim()) noteSearch(searchedFor);
  });

  onMount(async () => {
    if (!$isAdmin && $transcribeStatus.state === "idle") loadTranscribeStatus();
    cachedPages = (await cacheStats()).pages;
  });

  $: statusReady =
    $transcribeStatus.state === "ready" || $transcribeStatus.state === "error";
  $: statusReadyCount = Number($transcribeStatus.data?.queue?.ready) || 0;
  $: canPodcastMode = $isAdmin || (statusReady && statusReadyCount > 0) || cachedPages > 0;
  $: if (scope === "podcasts" && statusReady && !canPodcastMode) scope = "library";

  $: trTerm = q.trim();
  $: trBusy = trLoading || (!!trPending && trPending === trTerm);
  $: {
    clearTimeout(trTimer);
    if (
      scope === "podcasts" &&
      canPodcastMode &&
      podsTab === "passages" &&
      trTerm.length >= 2 &&
      !playlistLink(trTerm) &&
      trFor !== trTerm &&
      trPending !== trTerm &&
      !(trAttempt === trTerm && trLoading)
    ) {
      trPending = trTerm;
      trTimer = setTimeout(() => {
        trPending = "";
        runTranscriptSearch(trTerm);
      }, 300);
    } else if (trPending) {
      trPending = "";
    }
  }

  $: if (podsTab === "shows" && !$isAdmin) podsTab = "passages";

  // A pasted playlist link is not a search: offer to import it instead.
  $: shared = playlistLink(q);

  // The list the screen shows: the catalogue's order kept, library rows
  // first, and the fallback's hits woven in only where they belong
  // (lib/searchrank.js — a video leads only when it matches the query better
  // than the best catalogue answer; an identical one is dropped). `ytFor`
  // gates them: a late answer to the previous query never lands under this one.
  $: mixed = rankSearchResults(ytTerm, results.tracks || [], yt && ytAnswerHere ? ytTracks : []);
  $: allTracks = mixed.tracks;
  $: mixedYt = allTracks.filter((t) => t.source === "youtube").length;

  $: hasResults =
    results.artists?.length ||
    results.albums?.length ||
    allTracks.length ||
    results.playlists?.length ||
    (results.podcasts?.length && $isAdmin);

  // -- the radio the search can start ----------------------------------------
  // A broad search is a mood, not a lookup: from RADIO_MIN_RESULTS distinct
  // answers the results can become a queue. The count is the whole screen's
  // (lib/queryradio.js), the fallback counted only once it has answered, and
  // the button waits for the results of the term in the box — never a stale
  // count under a new query. A pasted playlist link is an import, not a radio.
  $: radioTotal = countSearchResults(
    ytTerm,
    results,
    yt && ytAnswerHere ? ytTracks : [],
    { withPodcasts: $isAdmin },
  );
  $: showRadio =
    !!ytTerm &&
    !shared &&
    scope === "library" &&
    !loading &&
    searchedFor === ytTerm &&
    offerRadio(radioTotal);
  const radio = createRadioLauncher({
    play: (tracks, start, context) => player.playQueue(tracks, start, context),
    extend: (tracks) => player.extend(tracks),
    radioNext: (seed, opts) => api.radioNext(seed, opts),
    toast: (message, kind) => toasts.push(message, kind),
    state: () => {
      const s = get(player);
      return { context: s.context, queue: s.queue, autoplay: s.autoplay };
    },
    // Player.svelte tops a queue of three titles or fewer up at once; leaving
    // those to it is what keeps one press to ONE /api/radio/next.
    deferShortQueueToPlayer: true,
  });
  let radioBusyFor = "";
  $: radioBusy = radioBusyFor === ytTerm;
  async function launchRadio() {
    const term = ytTerm;
    // Synchronous, before the first await: a second press in the same burst
    // cannot start a second queue. Keyed by the term so refining the search
    // while a top-up is still in flight is a new launch, not a dead button.
    if (radioBusyFor === term) return;
    radioBusyFor = term;
    try {
      await radio.launch(term, results, yt && ytAnswerHere ? ytTracks : []);
    } finally {
      if (radioBusyFor === term) radioBusyFor = "";
    }
  }

  // "Tout" leads with what a search is usually for: the artist the query
  // plainly names (only then — lib/searchrank.js), the titles, then the
  // artists, then the albums with the shows mixed in under a small "Podcast"
  // label, then the playlists. Ten titles is the most that still reads as a
  // shortlist on a phone; the rest is one tap away on the Titres tab. An old
  // saved "podcasts" tab lands on the albums, where the shows are now.
  $: if (tab === "podcasts") tab = "albums";
  $: podcasts = $isAdmin ? results.podcasts || [] : [];
  $: shows = scope === "podcasts" ? podcasts : [];
  $: passages = trHits || [];
  $: top = tab === "all" && scope === "library" ? topResult(q, results, $isAdmin) : null;
  $: shelfArtists = (results.artists || []).filter((a) => !(top?.kind === "artist" && a === top.item));
  $: albumsAndShows = mixPodcasts(
    results.albums || [],
    podcasts.filter((p) => !(top?.kind === "podcast" && p === top.item)),
  );

  const TOP_TRACKS = 10;
  $: topTracks = tab === "all" ? allTracks.slice(0, TOP_TRACKS) : allTracks;
  $: moreTracks = Math.max(0, allTracks.length - TOP_TRACKS);
  // The three shelves below show only for the library's own answer (the shows
  // have their own branch). One condition, read once per section: each shelf
  // must be its OWN `{#if}` whose parent has already run, or Svelte skips its
  // entrance (see the note above the shelves).
  $: shelvesShown = scope === "library" && !loading && !!q.trim() && hasResults;
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
  {#if ytAvailable && scope !== "podcasts"}
    <button
      class="chip"
      class:chip-active={yt}
      aria-pressed={yt}
      title={yt
        ? `Interroger aussi ${fb.name} (décocher pour l'ignorer dans cette recherche)`
        : `Chercher aussi sur ${fb.name} (qualité Opus, plus basse que le catalogue)`}
      on:click={toggleYoutube}
    >
      <Icon name={iconOf(fb.id)} size={15} /> {fb.name}
    </button>
  {/if}
  {#if canPodcastMode}
    <button
      class="chip"
      class:chip-active={scope === "podcasts"}
      aria-pressed={scope === "podcasts"}
      on:click={toggleShows}
      title="Chercher dans les titres des émissions et le texte des épisodes transcrits"
      aria-label="Chercher dans les titres des émissions et le texte des épisodes transcrits"
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
    placeholder={scope === "podcasts" ? "Rechercher dans les podcasts…" : "Titres, artistes, albums, podcasts…"}
    aria-label={scope === "podcasts" ? "Rechercher dans les podcasts" : "Rechercher un titre, un artiste, un album ou un podcast"}
    bind:value={q}
    on:input={onInput}
    on:keydown={onKey}
    autofocus={autofocusBox}
  />
  {#if showRadio}
    <button
      class="btn btn-primary radio-launch"
      on:click={launchRadio}
      disabled={radioBusy}
      aria-busy={radioBusy}
      title="Écouter des titres qui ressemblent à cette recherche"
      in:rise
    >
      {#if radioBusy}
        <span class="radio-spin" aria-hidden="true" data-motion="status"></span>
        Lancement…
      {:else}
        <Icon name="radio" size={17} />
        Lancer la radio
      {/if}
    </button>
  {/if}
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
    {:else if scope === "library"}
      <p class="muted lead">
        Cherche un titre, un artiste, un album ou un podcast — les titres de ta bibliothèque
        d'abord, puis le catalogue, et YouTube quand il a ce que le catalogue n'a pas.
      </p>
    {/if}
    {#if scope === "podcasts"}
      <p class="muted lead">Tape un mot pour chercher dans le texte des épisodes transcrits.</p>
    {:else}
      <ExplorerStyles />
    {/if}
  </div>
{/if}

{#if q.trim() && !shared && scope === "library"}
  <div class="tabs" role="tablist">
    {#each [["all", "Tout"], ["tracks", "Titres"], ["artists", "Artistes"], ["albums", "Albums"], ["playlists", "Playlists"]] as [id, label]}
      <button role="tab" aria-selected={tab === id} class="chip" class:chip-active={tab === id} on:click={() => pickTab(id)}>
        {label}
      </button>
    {/each}
  </div>
{/if}

{#if scope === "podcasts" && !shared}
  {#if $isAdmin}
    <div class="pods-tabs" role="tablist" aria-label="Recherche dans les podcasts">
      <button
        type="button"
        role="tab"
        class="chip"
        class:chip-active={podsTab === "passages"}
        aria-selected={podsTab === "passages"}
        on:click={() => (podsTab = "passages")}
      >
        Passages
      </button>
      <button
        type="button"
        role="tab"
        class="chip"
        class:chip-active={podsTab === "shows"}
        aria-selected={podsTab === "shows"}
        on:click={() => (podsTab = "shows")}
      >
        Émissions
      </button>
    </div>
  {/if}

  {#if q.trim()}
  {#if podsTab === "passages" || !$isAdmin}
    {#if q.trim().length < 2}
      <p class="muted hint">2 caractères minimum pour chercher dans les transcriptions.</p>
    {:else if trError?.status === 404}
      <p class="muted hint">
        La recherche dans les transcriptions n'est pas disponible sur ce serveur.
      </p>
    {:else if trError?.offline || (!$online && !trHits.length)}
      <p class="muted hint">La recherche dans les transcriptions a besoin du serveur.</p>
    {:else if trError}
      <p class="muted hint">
        {trError?.status === 429
          ? `Trop de recherches. Réessayez dans ${trRetryLeft} s.`
          : "La recherche dans les transcriptions a échoué."}
        {#if trError?.status !== 404}
          <button
            type="button"
            class="tr-retry"
            disabled={trRetryLeft > 0}
            on:click={() => runTranscriptSearch(trTerm, true)}
          >
            Réessayer
          </button>
        {/if}
      </p>
    {:else}
      {#if trBusy || trLocal || trTotal > 0}
        <p class="tr-summary" role="status" aria-live="polite">
          {#if trBusy}
            Recherche dans les transcriptions…
          {:else if trLocal}
            Recherche locale — {trTotal} passage{trTotal > 1 ? "s" : ""} dans les pages déjà lues.
          {:else}
            {trTotal} passage{trTotal > 1 ? "s" : ""}{#if $isAdmin}{" "}dans {shows.length} émission{shows.length > 1 ? "s" : ""}{/if}
          {/if}
        </p>
      {/if}
      {#if trScope && !trBusy}
        <p class="tr-scope">
          {compactNumber(trScope.episodes)} épisode{trScope.episodes > 1 ? "s" : ""}
          {#if trScope.segments != null}{" "}· {compactNumber(trScope.segments)} segments{/if}
          {#if trScope.truncated}{" "}· les plus récents{/if}
        </p>
      {/if}
      {#if !trBusy && !trLocal && trMore}
        <p class="tr-scope">20 passages affichés — la recherche s'arrête là.</p>
      {/if}

      {#if trBusy}
        <Skeleton kind="list" />
      {:else if trHits.length}
        <TranscriptHits hits={trHits} query={trFor || trTerm} showChannel onPick={pickPassage} reveal={hitsIn} />
      {:else if !trBusy}
        {#if $isAdmin && shows.length}
          <p class="muted hint">Aucun passage parlé pour « {q.trim()} ».</p>
        {:else}
          <p class="muted hint">Rien pour « {q.trim()} » dans les podcasts.</p>
          <p class="muted hint">
            La recherche porte sur les titres des émissions et sur le texte des épisodes déjà
            transcrits.
          </p>
        {/if}
      {/if}
    {/if}
  {:else}
    {#if loading}
      <Skeleton kind="shelf" />
    {:else if !q.trim()}
      <p class="muted hint">Tape un mot pour chercher un podcast.</p>
    {:else if loadError}
      <LoadFailed error={loadError} inline on:retry={() => run(searchedFor || q.trim())} />
    {:else if shows.length}
      <p class="tr-scope">{shows.length} émission{shows.length > 1 ? "s" : ""}</p>
      <div class="grid" in:fadeIn={libraryIn ? undefined : { duration: 0 }}>
        {#each shows as p (entityRef(p))}<PodcastCard item={p} />{/each}
      </div>
    {:else}
      <p class="muted hint">Aucun podcast pour « {q} ».</p>
    {/if}
  {/if}
  {/if}
{:else if loading}
  <Skeleton kind="shelf" />
  <Skeleton kind="list" />
{:else if !q.trim()}
  <!-- the recent searches and the style tiles are above: never an empty page -->
{:else if loadError && !hasResults}
  <LoadFailed error={loadError} inline on:retry={() => run(searchedFor || q.trim())} />
{:else if !hasResults}
  {#if !(yt && ytAvailable && ytWaiting) && !shared}
    <p class="muted hint">Aucun résultat pour « {q} ».</p>
  {/if}
{:else}
  <div class="leadwrap" in:fadeIn={libraryIn ? undefined : { duration: 0 }}><div class="lead" class:withtop={!!top}>
    {#if top}
      <section class="topcol">
        <h2>Meilleur résultat</h2>
        <!-- The card fades on its own when another entity takes the slot: see
             TopResult.svelte (a `{#key}` there would not play the entrance). -->
        <TopResult kind={top.kind} item={top.item} />
      </section>
    {/if}
    {#if (tab === "all" || tab === "tracks") && topTracks.length}
      <section class="trackcol">
        <div class="sechead">
          <h2>Titres</h2>
          {#if tab === "all" && moreTracks}
            <button class="seeall" on:click={() => pickTab("tracks")}>
              Tout afficher<span class="count">{allTracks.length}</span>
            </button>
          {/if}
        </div>
        <TrackList tracks={topTracks} radio />
      </section>
    {/if}
  </div></div>
{/if}

<!-- The fallback is part of the SAME list above; this line only says what it
     is doing when it has nothing to add — searching, failed, or no closer
     match. It never becomes a second section of results. -->
{#if scope === "library" && !loading && !shared && yt && ytAvailable && ytTerm && (tab === "all" || tab === "tracks")}
  {#if ytErrorHere}
    <p class="ytstatus" role="alert">
      {ytError}
      <button class="ytretry" on:click={() => runYoutube(true)}>Réessayer</button>
    </p>
  {:else if ytWaiting}
    <p class="ytstatus" role="status" aria-live="polite">
      <span class="ytspin" aria-hidden="true" data-motion="status"></span> Recherche sur {fb.name}…
    </p>
  {:else if ytAnswerHere && !mixedYt && allTracks.length}
    <p class="ytstatus muted">Rien de plus sur {fb.name} pour « {ytTerm} ».</p>
  {/if}
{/if}

<!-- I2: a tab switch IS a change of panel — the shelf the new tab brings in is
     a NEW block, so its `in:fadeIn` plays, and the shelves it drops go in the
     same frame (M7). Each shelf carries the whole condition itself (rather than
     sitting one `{#if}` deeper, under a keyed block): Svelte runs a local
     intro only when the block that CREATED the element has already run, so a
     block nested under a branch that is running for the first time never
     animates — the shelves would then enter with the results but not with a
     tab, or the other way round. The titles panel is the same list before and
     after a switch to « Titres », so it keeps its one entrance. -->
{#if shelvesShown && (tab === "all" || tab === "artists") && shelfArtists.length}
  <section class="shelf-sec" in:fadeIn={libraryIn ? undefined : { duration: 0 }}>
    <div class="sechead">
      <h2>Artistes</h2>
      {#if tab === "all" && shelfArtists.length > 1}
        <button class="seeall" on:click={() => pickTab("artists")}>Tout afficher</button>
      {/if}
    </div>
    <div class={tab === "artists" ? "grid" : "shelf"}>
      {#each shelfArtists as a (entityRef(a))}<Card item={a} kind="artist" />{/each}
    </div>
  </section>
{/if}

{#if shelvesShown && (tab === "all" || tab === "albums") && albumsAndShows.length}
  <section class="shelf-sec" in:fadeIn={libraryIn ? undefined : { duration: 0 }}>
    <div class="sechead">
      <h2>Albums</h2>
      {#if tab === "all" && albumsAndShows.length > 1}
        <button class="seeall" on:click={() => pickTab("albums")}>Tout afficher</button>
      {/if}
    </div>
    <div class={tab === "albums" ? "grid" : "shelf"}>
      {#each albumsAndShows as a ((a.podcast ? "p" : "a") + entityRef(a))}
        {#if a.podcast}<PodcastCard item={a} />{:else}<Card item={a} kind="album" />{/if}
      {/each}
    </div>
  </section>
{/if}

{#if shelvesShown && (tab === "all" || tab === "playlists") && results.playlists?.length}
  <section class="shelf-sec" in:fadeIn={libraryIn ? undefined : { duration: 0 }}>
    <div class="sechead">
      <h2>Playlists</h2>
      {#if tab === "all" && results.playlists.length > 1}
        <button class="seeall" on:click={() => pickTab("playlists")}>Tout afficher</button>
      {/if}
    </div>
    <div class={tab === "playlists" ? "grid" : "shelf"}>
      {#each results.playlists as p (entityRef(p))}<Card item={p} kind="playlist" />{/each}
    </div>
  </section>
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
  /* The radio is a primary action riding the search field: same 44 px height,
     the pressed control itself says it is working (no separate spinner line,
     nothing jumps under it). */
  .radio-launch {
    height: 44px;
    min-width: 170px;
    padding: 0 20px;
  }
  /* A launch in progress is not a dead control: the label and the spinner stay
     at full contrast (the shared .btn:disabled fade made the pressed primary
     action unreadable). Clicks are still refused by the disabled attribute. */
  .radio-launch:disabled {
    opacity: 1;
    cursor: progress;
  }
  .radio-launch:active:not(:disabled) {
    transform: translateY(1px);
  }
  .radio-spin {
    width: 14px;
    height: 14px;
    flex: none;
    border-radius: 50%;
    border: 2px solid currentColor;
    border-right-color: transparent;
    opacity: 0.9;
    animation: radio-spin 0.8s linear infinite;
  }
  @keyframes radio-spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .radio-spin {
      animation: none;
      border-right-color: currentColor;
      opacity: 0.65;
    }
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
  /* What the fallback is doing when it has nothing to add. One quiet line
     under the SAME list, never a section of its own; the retry button is the
     only control, because a failed source is a state with a way out. */
  .ytstatus {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    margin: 10px 0 0;
    font-size: 0.8rem;
    line-height: 1.4;
    color: var(--dim);
  }
  .ytretry {
    padding: 4px 12px;
    min-height: 32px;
    border-radius: 999px;
    border: 1px solid var(--ln);
    background: var(--s1);
    color: var(--tx);
    font-weight: 600;
    font-size: 0.78rem;
  }
  .ytretry:hover {
    background: var(--s2);
  }
  @media (pointer: coarse) {
    .ytretry {
      min-height: 44px;
      padding: 0 16px;
    }
  }
  .ytspin {
    width: 12px;
    height: 12px;
    flex: none;
    border-radius: 50%;
    border: 2px solid var(--s2);
    border-top-color: var(--ac-text);
    animation: ytspin 0.8s linear infinite;
  }
  @keyframes ytspin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .ytspin {
      animation: none;
    }
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
  /* I3: these tablists select by COLOUR (the plan's fallback for a tablist with
     no sliding indicator), so the choice steps in one short beat instead of
     cutting — `--dur-instant`, and nothing moves, so a sideways-scrolled rail
     never has to be measured. The source chips above follow the same rule. */
  .tabs :global(.chip),
  .pods-tabs :global(.chip),
  .sources :global(.chip) {
    transition:
      background-color var(--dur-instant) var(--ease-standard),
      border-color var(--dur-instant) var(--ease-standard),
      color var(--dur-instant) var(--ease-standard);
  }
  /* One block per results section, so its entrance is ONE animation (I2/I6)
     rather than one per heading and one per grid. It is a plain block: no
     margin, no padding, nothing the layout can see. */
  .shelf-sec {
    min-width: 0;
  }
  .hint {
    margin-top: 20px;
  }
  .pods-tabs {
    display: flex;
    gap: 8px;
    margin: 4px 0 12px;
  }
  .tr-summary {
    margin: 8px 0 2px;
    color: var(--dim);
    font-family: var(--font-mono);
    font-size: 10.5px;
    letter-spacing: 0.04em;
    font-variant-numeric: tabular-nums;
    line-height: 1.5;
  }
  .tr-scope {
    margin: 0 0 8px;
    color: var(--dim);
    font-size: 0.74rem;
    line-height: 1.4;
  }
  .tr-retry {
    min-height: 44px;
    margin-left: 8px;
    padding: 0 12px;
    border-radius: 999px;
    border: 1px solid var(--ln);
    color: var(--ac-text);
    font-size: 0.8rem;
    font-weight: 600;
  }
  .tr-retry:disabled {
    opacity: 0.55;
    cursor: default;
  }
  .tr-retry:focus-visible {
    outline: 2px solid var(--ac-text);
    outline-offset: 2px;
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
    /* Two lines of source chips rather than a rail that cut YouTube off the
       right edge with no sign it existed: a search source is a state, and a
       hidden state is a state nobody turns off. At most four chips wrap onto
       two rows on a 360 px screen. */
    .sources {
      flex-wrap: wrap;
      gap: 6px;
      margin: 14px 0 12px;
    }
    /* The field occupies the page width: at 640 the desktop's 560 px cap left
       a 40 px hole beside the box. */
    .bar input {
      flex: 1 1 100%;
      max-width: none;
    }
    /* And the radio takes the line under it: a full-width 44 px target rather
       than a pill squeezed beside a field that already fills the screen. */
    .radio-launch {
      flex: 1 1 100%;
      width: 100%;
    }
  }
</style>
