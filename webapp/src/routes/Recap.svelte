<script module>
  // One year request at a time, even across the route change from `#/recap` to
  // `#/recap/:year`: the router mounts a fresh component for the deep link
  // while the resolving instance is still alive, and both would otherwise ask
  // for the same payload. Concurrent identical requests share one promise; a
  // later visit still asks the server (its ETag makes it a 304).
  let inFlightYear = null;
</script>

<script>
  // The Récap screen: a year in review as a short story, on a phone first.
  //
  // Two URLs, one screen: `#/recap` resolves the most recent year that holds an
  // event and `replace`s itself onto `#/recap/:year`, which is a real deep link
  // (reloaded as-is, shareable inside the server). The whole year comes in ONE
  // bounded payload; the nine cards then cost zero requests.
  //
  // Personal listening data: nothing here is written to localStorage, and
  // `/recap*` is deliberately absent from lib/apicache.js — the server's ETag
  // does the revalidating.
  import { onMount } from "svelte";
  import { get } from "svelte/store";
  import { replace } from "svelte-spa-router";
  import { player } from "../lib/stores.js";
  import { online } from "../lib/net.js";
  import { api, failureCause } from "../lib/api.js";
  import { location } from "../lib/router.js";
  import { loadRecapYears, recapYears } from "../lib/recapyears.js";
  import { rememberScreen, recallScreen } from "../lib/nav.js";
  import {
    cardIds,
    emptyReason,
    formatNumber,
    formatMinutes,
    genreRows,
    periodLabel,
    rankRows,
    yearChips,
  } from "../lib/recap.js";
  import RecapStory from "../components/RecapStory.svelte";
  import RecapShare from "../components/RecapShare.svelte";
  import Icon from "../components/Icon.svelte";
  import LoadFailed from "../components/LoadFailed.svelte";
  // A branch of the screen fades in when its data arrives (I1); the deck dims
  // while a year is loading over 140 ms, from the tokens (M15).
  import { fadeIn } from "../lib/motion.js";

  export let params = {};

  const YEAR_RE = /^\d{4}$/;
  // The year this component was showing when the person left it — restored only
  // on a real traversal (lib/nav.js).
  const remembered = recallScreen();

  let mounted = false;
  let handled = "";
  let loading = false;
  let payload = null;
  let error = null;
  let invalid = false;
  let offline = false;
  let index = 0;
  let sharing = false;
  let activeChip = null;
  let seq = 0;
  let chipsEl = null;

  $: chipState = $recapYears;
  $: chips = yearChips(chipState || {});
  $: activeYear = activeChip != null ? activeChip : payload ? Number(payload.year) : null;
  $: nearby = nearestYear(chipState, activeYear);
  $: cards = payload ? cardIds(payload) : [];
  $: reason = payload ? emptyReason(payload) : null;
  $: noCards = !reason && payload != null && cards.length === 0;
  $: isEmpty = !!reason || noCards;
  $: noYears =
    !!chipState &&
    chipState.latest == null &&
    !payload &&
    !loading &&
    !error &&
    !offline &&
    !invalid;
  $: totals = (payload && payload.totals) || {};
  $: topArtist = payload ? rankRows(payload, "artists")[0] || null : null;
  $: topGenre = payload ? genreRows(payload)[0] || null : null;
  $: bigDay = (payload && payload.biggest_day) || null;

  // `#/recap` resolves a year; `#/recap/2026` is already one. The router keeps
  // this component mounted when only the parameter changes, so the LOCATION is
  // the single thing watched — and the last route we acted on is the gate (a
  // reactive block reading `params.year` would re-open on every unrelated
  // flush). The routed prop is the same fact for a deep link, and keeps the
  // prop meaningful without letting it drive the fetch on its own.
  $: routePath = params && params.year != null ? "/recap/" + encodeURIComponent(params.year) : $location;
  $: if (mounted) watchLocation(routePath);
  // Back to the screen lands where it was left; a fresh push starts at card 1.
  $: if (mounted && payload) rememberScreen({ recapYear: payload.year, recapIndex: index });

  onMount(() => {
    mounted = true;
  });

  function parseRoute(path) {
    if (path === "/recap") return { kind: "latest", raw: null };
    const match = /^\/recap\/([^/?#]+)\/?$/.exec(String(path || ""));
    if (!match) return null;
    let raw = match[1];
    try {
      raw = decodeURIComponent(raw);
    } catch {
      /* keep the raw form; the 4-digit test below refuses it without a call */
    }
    return { kind: "year", raw };
  }

  function watchLocation(path) {
    const route = parseRoute(path);
    if (!route) return;
    const key = route.kind === "year" ? "year:" + route.raw : "latest";
    if (key === handled) return;
    handled = key;
    open(route);
  }

  async function open(route, manual = false) {
    error = null;
    invalid = false;
    offline = false;
    // Nothing is fetched while the device knows it is offline: the sentence and
    // "Réessayer" are the answer, not a doomed request and its backoff. A
    // manual retry is the person asking again: when the DEVICE still has a
    // link (a server that just failed), one attempt is honest.
    const linkUp = typeof navigator === "undefined" || navigator.onLine !== false;
    if (!get(online) && !(manual && linkUp)) {
      loading = false;
      offline = true;
      return;
    }
    if (route.kind === "latest") {
      loading = true;
      payload = null;
      sharing = false;
      activeChip = null;
      let state = null;
      try {
        state = await loadRecapYears();
      } catch (e) {
        if (!mounted) return;
        loading = false;
        error = e;
        return;
      }
      if (!mounted) return;
      if (state && state.latest != null) {
        // Resolve, then let the deep-link branch do the one year request.
        replace("/recap/" + state.latest);
        return;
      }
      loading = false;
      return;
    }

    const raw = String(route.raw || "");
    if (!YEAR_RE.test(raw)) {
      // A malformed year NEVER reaches the server.
      loading = false;
      invalid = true;
      payload = null;
      return;
    }
    const year = Number(raw);
    const hadPayload = !!payload;
    loading = true;
    sharing = false;
    activeChip = year;
    // The chips are a side question: their failure must not condemn the year.
    loadRecapYears().catch(() => {});
    const mine = ++seq;
    try {
      const data = await fetchYear(year);
      if (!mounted || mine !== seq) return;
      const sameYear = hadPayload && payload && Number(payload.year) === Number(data.year);
      payload = data;
      activeChip = Number(data.year) || year;
      if (sameYear) {
        /* a retry of the year already on screen: keep the reader's card */
      } else if (!hadPayload && remembered && Number(remembered.recapYear) === Number(data.year)) {
        const max = Math.max(0, cardIds(data).length - 1);
        index = Math.max(0, Math.min(max, Number(remembered.recapIndex) || 0));
      } else {
        // Changing year always goes back to the first card.
        index = 0;
      }
    } catch (e) {
      if (!mounted || mine !== seq) return;
      payload = null;
      // A dead link while the device still has a carrier is a failure with a
      // "Réessayer"; a phone in airplane mode is a state, not a red error.
      const airplane = typeof navigator !== "undefined" && navigator.onLine === false;
      if (e && e.status === 0 && airplane) offline = true;
      else if (e && e.status === 400) invalid = true;
      else error = e;
    } finally {
      if (mounted && mine === seq) loading = false;
    }
  }

  function retry() {
    const route = parseRoute($location);
    if (route) open(route, true);
  }

  // The shared year fetch (module-level `inFlightYear`): see the module script.
  function fetchYear(year) {
    if (inFlightYear && inFlightYear.year === year) return inFlightYear.promise;
    const promise = api.recap(year).finally(() => {
      if (inFlightYear && inFlightYear.promise === promise) inFlightYear = null;
    });
    inFlightYear = { year, promise };
    return promise;
  }

  function chooseYear(value) {
    const year = Number(value);
    if (!Number.isInteger(year)) return;
    if (payload && Number(payload.year) === year) {
      // Already painted: drop a request in flight for another year and go back
      // to this one without asking the server again.
      seq += 1;
      loading = false;
      error = null;
      invalid = false;
      offline = false;
      activeChip = year;
      handled = "year:" + year;
      replace("/recap/" + year);
      return;
    }
    activeChip = year;
    replace("/recap/" + year);
  }

  function playTop() {
    const tracks = rankRows(payload, "tracks")
      .map((row) => row.track)
      .filter(Boolean);
    if (!tracks.length) return;
    player.playQueue(tracks, 0, {
      kind: "recap",
      id: String(payload.year),
      title: `Récap ${payload.year}`,
    });
  }

  function openShare() {
    if (payload && !isEmpty) sharing = true;
  }

  function revealYears() {
    const el = chipsEl;
    if (!el) return;
    const active = el.querySelector(".chip-active") || el.querySelector("button");
    if (!active) return;
    const still = typeof window !== "undefined" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    try {
      el.scrollTo({ left: Math.max(0, active.offsetLeft - 8), behavior: still ? "auto" : "smooth" });
    } catch {
      /* the chip may not be laid out yet; focusing still works */
    }
    active.focus({ preventScroll: true });
  }

  function nearestYear(state, current) {
    const rows = (state && Array.isArray(state.years) ? state.years : []).filter(
      (row) => row && Number(row.year) !== Number(current) && !(row.events != null && Number(row.events) <= 0)
    );
    const full = rows.filter((row) => row.enough);
    const pool = full.length ? full : rows;
    let best = null;
    for (const row of pool) {
      const distance = Math.abs(Number(row.year) - Number(current));
      if (!best || distance < best.distance) best = { year: Number(row.year), distance };
    }
    return best ? best.year : null;
  }

  function count(n, one, many) {
    const value = Math.max(0, Math.trunc(Number(n) || 0));
    return `${formatNumber(value)} ${value > 1 ? many : one}`;
  }

  $: emptyTitle =
    reason === "skips-only"
      ? `Aucune écoute en ${activeYear || ""}`.trim()
      : reason === "thin"
        ? `Trop peu d'écoutes en ${activeYear || ""}`.trim()
        : `Rien à raconter pour ${activeYear || ""}`.trim();
  $: emptyText =
    reason === "skips-only"
      ? "Seulement des titres passés cette année-là."
      : reason === "thin"
        ? `${count(totals.listens, "écoute", "écoutes")}, ${formatMinutes(totals.minutes)} — pas assez pour raconter une année.`
        : "Aucune écoute enregistrée cette année-là.";
</script>

<div class="recap fade-in">
  <header class="head">
    <div>
      <h1 class="page-title">Récap</h1>
      {#if payload && !isEmpty}
        <p class="head-sub muted">{periodLabel(payload)}</p>
      {/if}
    </div>
    {#if payload && !isEmpty && !loading}
      <button class="btn btn-ghost share-head" on:click={openShare}>
        <Icon name="share" size={16} /> Partager
      </button>
    {/if}
  </header>

  {#if !mounted || (loading && !payload)}
    <div class="recap-grid">
      <div class="deck sk-deck" aria-hidden="true">
        <div class="sk-bars">
          {#each Array(9) as _}<span></span>{/each}
        </div>
        <div class="sk-body">
          <span class="sk l w18"></span>
          <span class="sk l w72 big"></span>
          <span class="sk l w44"></span>
          <span class="sk l w60"></span>
          <span class="sk l w34"></span>
        </div>
      </div>
      <p class="sr" role="status">Chargement de ton récap…</p>
    </div>
  {:else if invalid}
    <div class="recap-grid" in:fadeIn>
      {#if chips.length}
        <div class="chips" bind:this={chipsEl} role="group" aria-label="Choisir une année">
          {#each chips as chip (chip.year)}
            <button
              class="chip"
              class:chip-active={chip.year === activeYear}
              aria-pressed={chip.year === activeYear}
              on:click={() => chooseYear(chip.year)}
            >{chip.label}</button>
          {/each}
        </div>
      {/if}
      <div class="deck panel">
        <span class="panel-icon"><Icon name="alert" size={26} /></span>
        <h2 class="panel-title">Cette année n'existe pas dans ton récap</h2>
        <p class="panel-text">Le lien est peut-être incomplet, ou l'année est hors de l'historique de ce serveur.</p>
        <div class="panel-actions">
          <a class="btn btn-ghost" href="#/"><Icon name="home" size={16} /> Retour à l'accueil</a>
        </div>
      </div>
    </div>
  {:else if offline}
    <div class="recap-grid" in:fadeIn>
      <div class="deck panel">
        <span class="panel-icon"><Icon name="cloudOff" size={26} /></span>
        <h2 class="panel-title">Ton récap a besoin du serveur</h2>
        <div class="panel-actions">
          <button class="btn btn-primary" on:click={retry}><Icon name="refresh" size={16} /> Réessayer</button>
        </div>
      </div>
    </div>
  {:else if error}
    <div class="recap-grid" in:fadeIn>
      <div class="panel card">
        <LoadFailed
          error={error}
          notFound={`Récap indisponible : ${failureCause(error) || "réessayez dans un instant"}.`}
          on:retry={retry}
        />
        <div class="panel-actions">
          <button class="btn btn-ghost" on:click={retry}><Icon name="refresh" size={16} /> Réessayer</button>
          <a class="btn btn-ghost" href="#/library/history">Retour à l'historique</a>
        </div>
      </div>
    </div>
  {:else if noYears}
    <div class="recap-grid" in:fadeIn>
      <div class="deck panel">
        <span class="panel-icon"><Icon name="music" size={26} /></span>
        <h2 class="panel-title">Ton récap apparaîtra quand tu auras écouté quelques titres.</h2>
        <div class="panel-actions">
          <a class="btn btn-primary" href="#/library"><Icon name="library" size={16} /> Ouvrir la bibliothèque</a>
        </div>
      </div>
    </div>
  {:else if isEmpty}
    <div class="recap-grid" in:fadeIn>
      {#if chips.length}
        <div class="chips" bind:this={chipsEl} role="group" aria-label="Choisir une année">
          {#each chips as chip (chip.year)}
            <button
              class="chip"
              class:chip-active={chip.year === activeYear}
              aria-pressed={chip.year === activeYear}
              on:click={() => chooseYear(chip.year)}
            >{chip.label}</button>
          {/each}
        </div>
      {/if}
      <div class="deck panel">
        <span class="panel-icon">
          <Icon name={reason === "skips-only" ? "next" : reason === "thin" ? "activity" : "timer"} size={26} />
        </span>
        <h2 class="panel-title">{emptyTitle}</h2>
        <p class="panel-text">{emptyText}</p>
        <div class="panel-actions">
          {#if nearby != null}
            <button class="btn btn-ghost" on:click={() => chooseYear(nearby)}>Voir {nearby}</button>
          {/if}
          <a class="btn btn-ghost" href="#/library/history">Retour à l'historique</a>
        </div>
      </div>
    </div>
  {:else if payload}
    <div class="recap-grid" in:fadeIn>
      <div class="chips" bind:this={chipsEl} role="group" aria-label="Choisir une année">
        {#each chips as chip (chip.year)}
          <button
            class="chip"
            class:chip-active={chip.year === activeYear}
            aria-pressed={chip.year === activeYear}
            on:click={() => chooseYear(chip.year)}
          >{chip.label}</button>
        {/each}
      </div>

      <div class="deck-col">
        <div class="deck" class:busy={loading} aria-busy={loading}>
          <RecapStory
            {payload}
            bind:index
            suspended={sharing || loading}
            on:play={playTop}
            on:share={openShare}
            on:changeyear={revealYears}
          />
        </div>
      </div>

      <section class="glance card" aria-labelledby="glance-title">
        <h2 class="label-mono" id="glance-title">En un coup d'œil</h2>
        <dl>
          <div><dt>Minutes</dt><dd>{formatNumber(totals.minutes)}</dd></div>
          <div><dt>Écoutes</dt><dd>{formatNumber(totals.listens)}</dd></div>
          <div><dt>Artiste</dt><dd title={topArtist ? topArtist.name : ""}>{topArtist ? topArtist.name : "—"}</dd></div>
          <div><dt>Style</dt><dd title={topGenre ? topGenre.name : ""}>{topGenre ? topGenre.name : "—"}</dd></div>
          <div>
            <dt>Plus grosse journée</dt>
            <dd>
              {#if bigDay}
                {count(bigDay.listens, "écoute", "écoutes")}
              {:else}
                —
              {/if}
            </dd>
          </div>
        </dl>
      </section>

      <button class="btn btn-primary share-btn" on:click={openShare} disabled={loading}>
        <Icon name="share" size={16} /> Partager
      </button>
    </div>
  {/if}
</div>

{#if sharing && payload}
  <RecapShare {payload} on:close={() => (sharing = false)} />
{/if}

<style>
  .recap {
    container-type: inline-size;
    min-width: 0;
  }
  .head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 16px;
    margin-bottom: 14px;
  }
  .page-title {
    margin: 0;
  }
  .head-sub {
    margin: 3px 0 0;
    font-size: 12.5px;
    line-height: 1.4;
  }

  .recap-grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 14px;
    min-width: 0;
  }
  .chips {
    display: flex;
    gap: 8px;
    overflow-x: auto;
    overscroll-behavior-x: contain;
    scrollbar-width: none;
    min-width: 0;
    padding: 2px;
    margin: -2px;
  }
  .chips::-webkit-scrollbar {
    display: none;
  }
  .chips .chip {
    min-height: 44px;
  }
  .deck-col {
    min-width: 0;
  }
  /* The deck is the content, one card at a time. Its height reserves the two
     fixed bars on a phone, the sticky top bar above it AND the screen's own
     heading, so the last line of a card is never parked under the mini player. */
  .deck {
    position: relative;
    width: 100%;
    max-width: 460px;
    margin-inline: auto;
    /* One-column: the header AND the year chips sit above the deck, so they
       are part of the height budget; 187 = header 121 + chips 58 + 8 px of
       breathing room below. 488 is the smallest deck a five-row card and its
       action still fit in; on a 360×640 phone the page scrolls the deck up
       over the mini player rather than clipping the fifth title. */
    height: clamp(
      488px,
      calc(100dvh - var(--topbar-h, 68px) - var(--player-h, 88px) - 187px),
      780px
    );
    min-height: 280px;
    border: 1px solid var(--ln);
    border-radius: var(--r-card);
    background: var(--s1);
    overflow: hidden;
    /* The cards inside adapt to the height the deck really has (the opening
       mosaic becomes a 2×2 block on a tall deck), not to the window's. */
    container-type: size;
    container-name: deck;
    transition: opacity var(--dur-fast) var(--ease-standard);
  }
  .deck.busy {
    opacity: 0.66;
    pointer-events: none;
  }
  .glance {
    display: none;
    padding: 16px 16px 6px;
  }
  .glance h2 {
    margin: 0 0 8px;
    font-size: 10.5px;
    color: var(--dim);
  }
  .glance dl {
    margin: 0;
  }
  .glance dl > div {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    padding: 8px 0;
    border-bottom: 1px solid var(--ln);
  }
  .glance dl > div:last-child {
    border-bottom: 0;
  }
  .glance dt {
    flex: none;
    color: var(--dim);
    font-size: 12.5px;
  }
  .glance dd {
    margin: 0;
    min-width: 0;
    max-width: 62%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
    font-weight: 600;
    text-align: right;
  }
  .share-btn {
    display: none;
    width: 100%;
    min-height: 44px;
  }
  .share-head {
    min-height: 44px;
    flex: none;
  }

  /* -- empty, invalid, offline, error ------------------------------------ */
  .panel {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 28px 22px;
    text-align: center;
    min-height: 300px;
  }
  .panel.card {
    height: auto;
    border: 1px solid var(--ln);
    border-radius: var(--r-card);
    background: var(--s1);
  }
  .panel-icon {
    display: grid;
    place-items: center;
    width: 56px;
    height: 56px;
    border-radius: 50%;
    background: var(--s2);
    color: var(--dim);
  }
  .panel-title {
    margin: 0;
    max-width: 22ch;
    font-family: var(--font-display);
    font-size: clamp(21px, 3.2vw, 27px);
    font-weight: 700;
    letter-spacing: -0.02em;
    line-height: 1.15;
  }
  .panel-text {
    margin: 0;
    max-width: 42ch;
    color: var(--dim);
    font-size: 13.5px;
    line-height: 1.5;
  }
  .panel-actions {
    display: flex;
    align-items: center;
    justify-content: center;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 6px;
  }
  .panel-actions .btn {
    min-height: 44px;
  }

  /* -- skeleton ----------------------------------------------------------- */
  .sk-deck {
    display: flex;
    flex-direction: column;
    gap: 20px;
    padding: 14px 16px 20px;
  }
  .sk-bars {
    display: flex;
    gap: 4px;
  }
  .sk-bars span {
    flex: 1;
    height: 3px;
    border-radius: 999px;
    background: var(--s2);
  }
  .sk-body {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 18px 0;
  }
  .sk {
    display: block;
    height: 12px;
    border-radius: 8px;
    background: linear-gradient(
      90deg,
      var(--s2) 25%,
      color-mix(in srgb, var(--s2) 82%, var(--tx)) 37%,
      var(--s2) 63%
    );
    background-size: 400% 100%;
    animation: recap-shimmer 1.4s ease infinite;
  }
  .sk.big {
    height: 34px;
  }
  .w18 {
    width: 18%;
  }
  .w34 {
    width: 34%;
  }
  .w44 {
    width: 44%;
  }
  .w60 {
    width: 60%;
  }
  .w72 {
    width: 72%;
  }
  @keyframes recap-shimmer {
    0% {
      background-position: 100% 0;
    }
    100% {
      background-position: -100% 0;
    }
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip: rect(0 0 0 0);
    clip-path: inset(50%);
    white-space: nowrap;
    border: 0;
  }

  /* -- wide content: deck + a reading column ----------------------------- */
  @container (min-width: 760px) {
    .recap-grid {
      grid-template-columns: minmax(360px, 460px) minmax(280px, 360px);
      grid-template-rows: auto auto 1fr;
      gap: 18px 24px;
      justify-content: center;
      align-items: start;
    }
    .chips {
      grid-column: 2;
      grid-row: 1;
    }
    .deck-col {
      grid-column: 1;
      grid-row: 1 / span 3;
    }
    .deck {
      max-width: none;
      /* The chips are in the right column here: only the header is above. */
      height: clamp(
        488px,
        calc(100dvh - var(--topbar-h, 68px) - var(--player-h, 88px) - 129px),
        780px
      );
    }
    .glance {
      display: block;
      grid-column: 2;
      grid-row: 2;
    }
    .share-btn {
      display: inline-flex;
      grid-column: 2;
      grid-row: 3;
      align-self: start;
    }
    .share-head {
      display: none;
    }
  }

  @media (max-width: 640px) {
    .head {
      margin-bottom: 10px;
    }
    .head-sub {
      display: none;
    }
    .deck {
      max-width: none;
      /* The two fixed bars, the top bar, the screen's title and the year chips
         all take their height; 488 px is the smallest deck a five-row card and
         its action still fit in, so a 640 px-tall phone scrolls the deck up
         over the mini player instead of clipping the fifth title. */
      height: clamp(
        488px,
        calc(
          100dvh - var(--topbar-h, 60px) - var(--mini-h, 64px) - var(--tabbar-h, 64px) -
            var(--safe-bottom, 0px) - 148px
        ),
        780px
      );
    }
    .panel {
      padding: 22px 16px;
    }
    .panel-title {
      max-width: 20ch;
    }
    .panel-actions {
      width: 100%;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .deck {
      transition: none;
    }
    .sk {
      animation: none;
    }
  }
</style>
