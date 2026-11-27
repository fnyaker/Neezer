<script>
  import { onMount, tick } from "svelte";
  import { replace, push } from "svelte-spa-router";
  import { user } from "../lib/stores.js";
  import { api } from "../lib/api.js";
  import { online } from "../lib/net.js";
  import { nearEnd } from "../lib/grow.js";
  import Icon from "../components/Icon.svelte";
  import LoadFailed from "../components/LoadFailed.svelte";
  import OnboardingArtistCard from "../components/OnboardingArtistCard.svelte";
  import OnboardingAccount from "../components/OnboardingAccount.svelte";
  // The step changes like a page: it rises when going on, only fades when
  // coming back (docs/agents/transitions.md, I15 + M7). `rise` at y 0 IS that
  // fade — the level's own reduced variant already drops the transform.
  import { fadeIn, rise } from "../lib/motion.js";
  import {
    PAGE_FIRST,
    PAGE_NEXT,
    SEARCH_LIMIT,
    SIMILAR_TOTAL_MAX,
    artistCountLabel,
    budgetLeft,
    cacheList,
    canPassArtists,
    dropStore,
    enqueueLike,
    feedCause,
    finishBlocked,
    finishMessage,
    idOf,
    insertSimilar,
    normalizeQuery,
    progressHint,
    readStore,
    reconcilePage,
    removeSimilar,
    searchProblem,
    writeStore,
  } from "../lib/onboarding.js";

  export let step = "artists";

  const LIST_TTL = 30 * 60 * 1000;
  const CACHE_TTL = 30 * 60 * 1000;
  const RELATED_TRIES = 5;
  const BUILD_TRIES = 15;

  let accounts = false;
  // Which way the visible pane travels: forward to the accounts step rises,
  // the back button only fades (M7: the pane replaces itself in place).
  let stepBack = false;
  let loading = true;
  let record = null;
  let loadError = null;

  let artists = [];
  let next = "";
  let feed = null;
  let listError = null;
  let feedAttempts = 0;
  let loadingMore = false;
  let mounted = PAGE_FIRST;
  let listSeq = 0;
  let buildTimer = null;
  let buildTries = 0;

  let likedIds = new Set();
  let likedTotal = 0;
  let pendingIds = new Set();
  let slots = new Map();
  let queue = [];
  let flushing = false;
  let actionError = null;
  let insertNote = "";
  let similarNotice = "";
  let relatedTimers = new Map();
  let destroyed = false;

  let q = "";
  let query = "";
  let searching = false;
  let searchItems = null;
  let searchError = null;
  let searchNote = "";
  let searchSeq = 0;
  let debounce = null;
  let searchEl = null;

  let gridEl = null;
  let focusId = "";
  let scrolled = false;

  let finishStatus = "";
  let statusSeq = 0;
  let finishing = false;
  let finishWanted = false;
  let finishingAccount = false;
  let accountBusy = false;
  let accountFinishArmed = false;

  $: accounts = step === "accounts";
  $: items = query ? searchItems || [] : artists;
  $: visible = items.slice(0, Math.max(PAGE_FIRST, mounted));
  $: focusIndex = Math.min(
    Math.max(0, visible.findIndex((artist) => idOf(artist) === focusId)),
    Math.max(0, visible.length - 1)
  );
  $: unavailable = !!(feed && feed.state === "unavailable" && !feed.building);
  $: canPass = canPassArtists({ attempts: feedAttempts, feed, count: likedTotal });
  $: endOfList = !next && visible.length > 0 && visible.length >= items.length;
  $: finishLabel = finishing
    ? "Enregistrement…"
    : finishingAccount
      ? "Enregistrement…"
      : accounts
        ? "Terminer"
        : "Terminé";
  $: finishOff = !accounts && likedTotal < 1;
  $: railWidth = accounts ? 100 : 50;
  $: if ($online && queue.length) flushQueue();
  $: if (accounts) {
    clearTimeout(buildTimer);
    buildTimer = null;
    for (const timer of relatedTimers.values()) clearTimeout(timer);
    relatedTimers.clear();
  }
  $: if (finishWanted && !finishing && pendingIds.size === 0 && queue.length === 0) {
    finishWanted = false;
    finishArtists();
  }

  onMount(() => {
    const account = ($user && $user.name) || "";
    const cached = readStore("list", { maxAge: LIST_TTL });
    if (cached && (!cached.account || cached.account === account) && Array.isArray(cached.items) && cached.items.length) {
      artists = cached.items;
      next = typeof cached.next === "string" ? cached.next : "";
      if (Array.isArray(cached.liked)) likedIds = new Set(cached.liked);
    }
    const savedSearch = readStore("search", { maxAge: CACHE_TTL });
    if (savedSearch && savedSearch.account === account && savedSearch.q) {
      q = savedSearch.q;
      query = savedSearch.q;
      searchItems = Array.isArray(savedSearch.items) ? savedSearch.items : [];
    }
    const savedLiked = readStore("liked", { maxAge: null });
    if (savedLiked && (!savedLiked.account || savedLiked.account === account) && Array.isArray(savedLiked.ids)) {
      likedIds = new Set(savedLiked.ids);
    }
    const savedQueue = readStore("queue", { maxAge: null });
    if (savedQueue && (!savedQueue.account || savedQueue.account === account) && Array.isArray(savedQueue.entries)) {
      queue = savedQueue.entries;
      for (const entry of queue) if (entry.liked) likedIds.add(entry.id);
    } else {
      dropStore("queue");
    }
    const armGrowth = () => {
      scrolled = true;
    };
    const armIfNoScroll = setTimeout(() => {
      const doc = document.documentElement;
      if (!scrolled && doc.scrollHeight <= window.innerHeight + 200) scrolled = true;
    }, 1200);
    window.addEventListener("scroll", armGrowth, { passive: true, capture: true });
    load();
    return () => {
      destroyed = true;
      clearTimeout(debounce);
      clearTimeout(buildTimer);
      clearTimeout(armIfNoScroll);
      window.removeEventListener("scroll", armGrowth, { capture: true });
      for (const timer of relatedTimers.values()) clearTimeout(timer);
      relatedTimers.clear();
    };
  });

  async function load() {
    loading = true;
    loadError = null;
    try {
      const state = await api.onboardingState();
      if (destroyed) return;
      record = state || {};
      likedTotal = Number(record.liked) || 0;
      feed = record.feed || null;
      if (record.step === "done") {
        finishToHome();
        return;
      }
      if (accounts) {
        if (record.step !== "account") replace("/onboarding/artists");
        return;
      }
      if (query) {
        if (!searchItems || !searchItems.length) await runSearch(query);
        if (!artists.length) await loadPage({ first: true });
        return;
      }
      if (!artists.length) await loadPage({ first: true });
      else await revalidate();
    } catch (e) {
      if (!destroyed) loadError = e;
    } finally {
      if (!destroyed) loading = false;
    }
  }

  function rememberFeed(page) {
    feed = {
      ...(feed || {}),
      state: page.state || (page.items && page.items.length ? "ready" : "unavailable"),
      building: !!page.building,
      error: page.error || null,
      source: page.source || (feed && feed.source) || "",
    };
  }

  function accountName() {
    return ($user && $user.name) || "";
  }

  function seedLiked(items) {
    const next = new Set(likedIds);
    for (const item of items || []) if (item && item.liked) next.add(idOf(item));
    likedIds = next;
  }

  function persistList() {
    writeStore("list", {
      account: accountName(),
      items: cacheList(artists),
      next,
      liked: Array.from(likedIds),
    });
  }

  function persistQueue() {
    writeStore("queue", { account: accountName(), entries: queue });
  }

  function persistLiked() {
    writeStore("liked", { account: accountName(), ids: Array.from(likedIds) });
    persistList();
  }

  async function loadPage({ first = false } = {}) {
    if (loadingMore) return;
    loadingMore = true;
    listError = null;
    const seq = ++listSeq;
    try {
      const page = await api.onboardingArtists({
        after: first ? null : next,
        limit: first ? PAGE_FIRST : PAGE_NEXT,
      });
      if (destroyed || seq !== listSeq) return;
      const pageItems = Array.isArray(page.items) ? page.items : [];
      artists = first && !artists.length ? pageItems : reconcilePage(artists, pageItems);
      seedLiked(pageItems);
      next = page.next || "";
      rememberFeed(page);
      persistList();
      if (!artists.length) onEmpty(page);
    } catch (e) {
      if (destroyed || seq !== listSeq) return;
      listError = e;
    } finally {
      if (seq === listSeq) loadingMore = false;
    }
  }

  async function revalidate() {
    const before = artists.length;
    listError = null;
    try {
      const page = await api.onboardingArtists({ limit: PAGE_FIRST });
      if (destroyed) return;
      const pageItems = Array.isArray(page.items) ? page.items : [];
      artists = reconcilePage(artists, pageItems);
      seedLiked(pageItems);
      if (before <= pageItems.length) next = page.next || "";
      rememberFeed(page);
      persistList();
      if (!artists.length) onEmpty(page);
    } catch (e) {
      if (!destroyed) listError = e;
    }
  }

  function onEmpty(page) {
    if (page && page.building) startBuildPoll();
  }

  function startBuildPoll() {
    if (buildTimer || buildTries >= BUILD_TRIES) return;
    const tickBuild = async () => {
      buildTimer = null;
      if (destroyed || document.visibilityState === "hidden" || buildTries >= BUILD_TRIES) return;
      buildTries += 1;
      try {
        const state = await api.onboardingState();
        if (destroyed) return;
        if (state) {
          record = state;
          feed = state.feed || feed;
        }
        if (feed && feed.state === "ready") {
          await loadPage({ first: !artists.length });
          return;
        }
        if (feed && feed.state === "unavailable" && !feed.building) return;
      } catch {
      }
      if (!destroyed) buildTimer = setTimeout(tickBuild, 2000);
    };
    buildTimer = setTimeout(tickBuild, 2000);
  }

  async function retryFeed() {
    feedAttempts += 1;
    listError = null;
    const seq = ++listSeq;
    try {
      await api.onboardingFeedRetry();
    } catch (e) {
      if (destroyed || seq !== listSeq) return;
      if (e && e.status === 429) {
        actionError = {
          message: "Une reconstruction vient d'être demandée. Réessayez dans quelques minutes.",
          retry: retryFeed,
        };
        return;
      }
      if (e && e.body && e.body.code === "onboarding_done") {
        finishToHome();
        return;
      }
    }
    await loadPage({ first: !artists.length });
    if (!artists.length) startBuildPoll();
  }

  function seenIds() {
    return items
      .map(idOf)
      .filter(Boolean)
      .slice(0, 200);
  }

  function activeList() {
    return query ? searchItems || [] : artists;
  }

  function setActive(list) {
    if (query) {
      searchItems = list;
      return;
    }
    artists = list;
    persistList();
  }

  function setSlot(parent, state) {
    const copy = new Map(slots);
    copy.set(parent, state);
    slots = copy;
  }

  function clearSlot(parent) {
    if (!slots.has(parent)) return;
    const copy = new Map(slots);
    copy.delete(parent);
    slots = copy;
  }

  function toggle({ id, liked }) {
    const key = String(id == null ? "" : id);
    if (!key || pendingIds.has(key)) return;
    actionError = null;
    similarNotice = "";
    insertNote = "";
    if (liked) unlike(key);
    else like(key);
  }

  function like(key) {
    likedIds = new Set(likedIds).add(key);
    likedTotal += 1;
    persistLiked();
    if (!$online) {
      queue = enqueueLike(queue, key, true);
      persistQueue();
      return;
    }
    const list = activeList();
    const index = list.findIndex((artist) => idOf(artist) === key);
    const deeper = index >= 0 && list[index].via === "related";
    if (!deeper && budgetLeft(list, SIMILAR_TOTAL_MAX) > 0) setSlot(key, "pending");
    else if (!deeper) similarNotice = "Vous avez assez de suggestions pour l'instant. Continuez à parcourir.";
    pendingIds = new Set(pendingIds).add(key);
    api.onboardingLike(key, seenIds()).then(
      (result) => settleLike(key, true, result),
      (failure) => settleLike(key, false, failure)
    );
  }

  function unlike(key) {
    likedIds = new Set(likedIds);
    likedIds.delete(key);
    likedTotal = Math.max(0, likedTotal - 1);
    persistLiked();
    clearSlot(key);
    setActive(removeSimilar(activeList(), key, likedIds));
    if (!$online) {
      queue = enqueueLike(queue, key, false);
      persistQueue();
      return;
    }
    pendingIds = new Set(pendingIds).add(key);
    api.onboardingUnlike(key).then(
      () => settleUnlike(key, true),
      () => settleUnlike(key, false)
    );
  }

  function settleUnlike(key, ok) {
    pendingIds = new Set(pendingIds);
    pendingIds.delete(key);
    if (ok) return;
    finishWanted = false;
    likedIds = new Set(likedIds).add(key);
    likedTotal += 1;
    persistLiked();
    actionError = {
      message: "Votre choix n'a pas été enregistré.",
      retry: () => toggle({ id: key, liked: true }),
    };
  }

  function settleLike(key, ok, result) {
    pendingIds = new Set(pendingIds);
    pendingIds.delete(key);
    if (!ok) {
      finishWanted = false;
      likedIds = new Set(likedIds);
      likedIds.delete(key);
      likedTotal = Math.max(0, likedTotal - 1);
      persistLiked();
      clearSlot(key);
      actionError = {
        message: "Votre choix n'a pas été enregistré.",
        retry: () => {
          if (!likedIds.has(key)) like(key);
        },
      };
      return;
    }
    mergeRelated(key, result && result.related);
  }

  function mergeRelated(parent, related) {
    const state = related && related.state;
    if (state === "pending") {
      setSlot(parent, "pending");
      pollRelated(parent, 0);
      return;
    }
    if (state === "error") {
      setSlot(parent, "failed");
      return;
    }
    if (state === "ready" && Array.isArray(related.items) && related.items.length) {
      fillRelated(parent, related.items);
      return;
    }
    clearSlot(parent);
  }

  function fillRelated(parent, related) {
    const list = activeList();
    const index = list.findIndex((artist) => idOf(artist) === parent);
    clearSlot(parent);
    if (index < 0) return;
    const named = related.map((artist) => ({ ...artist, parent_name: list[index].name }));
    const result = insertSimilar(list, index, named, {
      budget: SIMILAR_TOTAL_MAX,
      liked: likedIds,
      diversity: record && record.diversity,
    });
    setActive(result.items);
    if (result.inserted) {
      insertNote = `${artistCountLabel(result.inserted)} proche${result.inserted > 1 ? "s" : ""} ajouté${result.inserted > 1 ? "s" : ""}`;
    }
  }

  function pollRelated(parent, attempt) {
    if (destroyed) return;
    const delay = attempt ? 1500 : 900;
    const timer = setTimeout(async () => {
      relatedTimers.delete(parent);
      if (destroyed || document.visibilityState === "hidden") {
        if (!destroyed) pollRelated(parent, attempt);
        return;
      }
      try {
        const result = await api.onboardingRelated(parent, seenIds());
        if (destroyed) return;
        const state = result && result.state;
        if (state === "ready") {
          fillRelated(parent, Array.isArray(result.items) ? result.items : []);
          return;
        }
        if (state === "pending" && attempt < RELATED_TRIES) {
          pollRelated(parent, attempt + 1);
          return;
        }
        if (state === "error") setSlot(parent, "failed");
        else clearSlot(parent);
      } catch {
        if (!destroyed) setSlot(parent, "failed");
      }
    }, delay);
    relatedTimers.set(parent, timer);
  }

  function retryRelated(parent) {
    if (destroyed) return;
    setSlot(parent, "pending");
    api.onboardingRelated(parent, seenIds()).then(
      (result) => {
        if (destroyed) return;
        const state = result && result.state;
        if (state === "ready") fillRelated(parent, Array.isArray(result.items) ? result.items : []);
        else if (state === "pending") pollRelated(parent, 1);
        else setSlot(parent, "failed");
      },
      () => {
        if (!destroyed) setSlot(parent, "failed");
      }
    );
  }

  async function flushQueue() {
    if (flushing || !queue.length) return;
    flushing = true;
    while (queue.length && !destroyed) {
      const entry = queue[0];
      try {
        if (entry.liked) await api.onboardingLike(entry.id, seenIds());
        else await api.onboardingUnlike(entry.id);
      } catch (e) {
        flushing = false;
        finishWanted = false;
        actionError = {
          message:
            "Votre choix n'a pas été enregistré. Vérifiez la connexion puis réessayez.",
          retry: flushQueue,
        };
        announce("");
        return;
      }
      queue = queue.slice(1);
      persistQueue();
    }
    flushing = false;
  }

  function grow() {
    if (query || !scrolled) return;
    if (visible.length < items.length) {
      mounted = Math.min(items.length, mounted + PAGE_FIRST);
      return;
    }
    if (next && !loadingMore) loadPage();
  }

  function columns() {
    if (!gridEl) return 1;
    try {
      const template = getComputedStyle(gridEl).gridTemplateColumns;
      return Math.max(1, template.split(" ").filter(Boolean).length);
    } catch {
      return 1;
    }
  }

  function onGridNav(key) {
    const keys = ["ArrowRight", "ArrowLeft", "ArrowDown", "ArrowUp", "Home", "End"];
    if (!keys.includes(key)) return;
    const total = visible.length;
    if (!total) return;
    const current = focusIndex;
    let target = current;
    if (key === "Home") target = 0;
    else if (key === "End") target = total - 1;
    else if (key === "ArrowRight") target = Math.min(total - 1, current + 1);
    else if (key === "ArrowLeft") target = Math.max(0, current - 1);
    else if (key === "ArrowDown") target = Math.min(total - 1, current + columns());
    else if (key === "ArrowUp") target = Math.max(0, current - columns());
    focusId = idOf(visible[target]);
    tick().then(() => {
      gridEl?.querySelector(`[data-index="${target}"]`)?.focus();
    });
  }

  function onSearchInput(event) {
    q = event.currentTarget.value;
    clearTimeout(debounce);
    if (!normalizeQuery(q)) {
      clearSearch();
      return;
    }
    if (searchProblem(q)) return;
    debounce = setTimeout(() => runSearch(q), 350);
  }

  function onSearchKey(event) {
    if (event.key === "Enter") {
      event.preventDefault();
      clearTimeout(debounce);
      runSearch(q);
    } else if (event.key === "Escape") {
      event.preventDefault();
      clearSearch();
    }
  }

  async function runSearch(raw) {
    const value = normalizeQuery(raw);
    if (searchProblem(value)) return;
    const seq = ++searchSeq;
    clearTimeout(debounce);
    searching = true;
    searchError = null;
    try {
      const result = await api.onboardingSearch(value, SEARCH_LIMIT);
      if (destroyed || seq !== searchSeq) return;
      query = value;
      q = value;
      searchItems = result && Array.isArray(result.items) ? result.items : [];
      searchNote = result && result.error ? feedCause(result.error) : "";
      writeStore("search", { account: accountName(), q: value, items: searchItems });
      mounted = PAGE_FIRST;
    } catch (e) {
      if (destroyed || seq !== searchSeq) return;
      searchError = e;
      searchItems = [];
    } finally {
      if (seq === searchSeq) searching = false;
    }
  }

  function clearSearch() {
    clearTimeout(debounce);
    searchSeq += 1;
    q = "";
    query = "";
    searchItems = null;
    searching = false;
    searchError = null;
    searchNote = "";
    dropStore("search");
    mounted = PAGE_FIRST;
    if (!artists.length) loadPage({ first: true });
    tick().then(() => searchEl?.focus());
  }

  function announce(message) {
    finishStatus = message;
    statusSeq += 1;
  }

  function onFinish() {
    if (accounts) {
      finishAccount();
      return;
    }
    const blocked = finishBlocked({
      liked: likedTotal,
      pending: pendingIds.size,
      queued: queue.length,
      online: $online,
    });
    if (blocked === "pending") {
      finishWanted = true;
      announce(finishMessage("pending"));
      return;
    }
    if (blocked) {
      announce(finishMessage(blocked));
      return;
    }
    finishArtists();
  }

  async function finishArtists() {
    if (finishing) return;
    finishing = true;
    finishStatus = "";
    try {
      const result = await api.onboardingArtistsDone();
      const reached = (result && result.step) || "account";
      user.update((current) =>
        current ? { ...current, onboarding: reached === "done" ? "done" : "account" } : current
      );
      if (reached === "done") {
        finishToHome();
        return;
      }
      stepBack = false;
      push("/onboarding/accounts");
    } catch (e) {
      const body = (e && e.body) || {};
      announce(
        body.error ||
          (e && e.offline
            ? "Hors ligne : reconnectez-vous pour terminer."
            : "L'étape n'a pas pu être enregistrée. Réessayez.")
      );
    } finally {
      finishing = false;
    }
  }

  async function passArtists() {
    if (finishing) return;
    finishing = true;
    finishStatus = "";
    try {
      const result = await api.onboardingArtistsDone({ skip: true });
      const reached = (result && result.step) || "done";
      user.update((current) => (current ? { ...current, onboarding: reached === "account" ? "account" : "done" } : current));
      if (reached === "account") {
        stepBack = false;
        push("/onboarding/accounts");
      } else finishToHome();
    } catch (e) {
      const body = (e && e.body) || {};
      announce(body.error || "Le serveur n'a pas pu terminer cette étape. Réessayez.");
    } finally {
      finishing = false;
    }
  }

  async function finishAccount() {
    if (finishingAccount) return;
    if (!$online) {
      announce("Hors ligne : terminez quand la connexion revient.");
      return;
    }
    if (accountBusy && !accountFinishArmed) {
      accountFinishArmed = true;
      announce(
        "Un import est en cours : il continuera sur le serveur. Cliquez sur Terminer pour quitter maintenant."
      );
      return;
    }
    finishingAccount = true;
    finishStatus = "";
    try {
      await api.onboardingAccountDone();
      user.update((current) => (current ? { ...current, onboarding: "done" } : current));
      finishToHome();
    } catch (e) {
      const body = (e && e.body) || {};
      if (body.code === "artists_not_done") {
        announce("Choisissez au moins un artiste avant de terminer.");
        actionError = { message: "Choisissez au moins un artiste avant de terminer.", retry: gotoArtists };
      } else {
        announce(body.error || "L'étape n'a pas pu être terminée. Réessayez.");
      }
    } finally {
      finishingAccount = false;
    }
  }

  function finishToHome() {
    user.update((current) => (current ? { ...current, onboarding: "done" } : current));
    try {
      history.replaceState(null, "", location.pathname + location.search + "#/");
    } catch {
    }
    location.reload();
  }

  function gotoArtists() {
    actionError = null;
    stepBack = true;
    push("/onboarding/artists");
  }
</script>

<svelte:head>
  <title>{accounts ? "Compte" : "Artistes"} · Neezer</title>
  <meta name="robots" content="noindex" />
</svelte:head>

<div class="page">
  <header class="bar">
    <div class="left">
      {#if accounts}
        <button class="iconbtn" type="button" aria-label="Retour aux artistes" on:click={gotoArtists}>
          <Icon name="chevronLeft" size={20} />
        </button>
      {/if}
      <span class="brand">
        <span class="mark"><Icon name="disc" size={15} /></span>
        <span class="brandname">Neezer</span>
      </span>
    </div>
    <span class="label-mono steplabel">Étape {accounts ? 2 : 1} sur 2</span>
    <button
      class="finish"
      type="button"
      aria-disabled={finishOff ? "true" : "false"}
      aria-describedby="ob-status"
      on:click={onFinish}
    >
      {finishLabel}
    </button>
    <span class="rail" aria-hidden="true"><span style={`transform:scaleX(${railWidth / 100})`}></span></span>
  </header>

  {#if finishStatus}
    {#key statusSeq}
      <p class="status" id="ob-status" role="status" aria-live="polite" in:fadeIn>{finishStatus}</p>
    {/key}
  {:else}
    <span class="sr" id="ob-status"></span>
  {/if}

  {#if !$online}
    <p class="banner" role="status" in:fadeIn>
      <Icon name="cloudOff" size={16} />
      <span>
        {accounts
          ? "Hors ligne : les liaisons et les imports attendent le retour du réseau."
          : "Hors ligne : vos choix seront enregistrés au retour du réseau."}
      </span>
    </p>
  {/if}

  {#if loading && !record}
    <main class="pane">
      <div class="skeleton">
        <span class="sk title"></span>
        <span class="sk line"></span>
        <span class="sk search"></span>
        <div class="grid" aria-busy="true">
          {#each [1, 2, 3, 4, 5, 6, 7, 8] as _}
            <OnboardingArtistCard placeholder />
          {/each}
        </div>
      </div>
    </main>
  {:else if loadError}
    <main class="pane" in:fadeIn>
      <div class="failure">
        <span class="failure-icon"><Icon name="cloudOff" size={24} /></span>
        <h1>Impossible de charger cette étape.</h1>
        <div class="failedslot">
          <LoadFailed
            error={loadError}
            notFound="Le serveur n'a pas répondu. Votre compte est créé : rien n'est perdu."
            on:retry={load}
          />
        </div>
        <button class="ghost" type="button" on:click={load}>
          <Icon name="refresh" size={16} /> Réessayer
        </button>
      </div>
    </main>
  {:else if accounts}
    <main class="pane" in:rise={{ y: stepBack ? 0 : 8 }}>
      <OnboardingAccount
        options={record ? record.account || null : null}
        onActivity={(busy) => (accountBusy = busy)}
      />
    </main>
  {:else}
    <main class="pane" in:rise={{ y: stepBack ? 0 : 8 }}>
      <div class="flow">
        <header class="head">
          <div class="titles">
            <h1>Quels artistes écoutez-vous ?</h1>
          </div>
          <div class="searchwrap">
            <label class="sr" for="ob-search">Chercher un artiste</label>
            <span class="sicon" aria-hidden="true"><Icon name="search" size={17} /></span>
            <input
              id="ob-search"
              bind:this={searchEl}
              class="search"
              type="search"
              placeholder="Chercher un artiste…"
              bind:value={q}
              on:input={onSearchInput}
              on:keydown={onSearchKey}
              autocomplete="off"
              autocapitalize="none"
              spellcheck="false"
            />
            {#if q}
              <button class="clear" type="button" aria-label="Effacer la recherche" on:click={clearSearch}>
                <Icon name="close" size={16} />
              </button>
            {/if}
          </div>
        </header>

        {#if !loadError && !(listError && !artists.length)}
          <div class="progressrow">
            <span class="dots" aria-hidden="true">
              {#each [0, 1, 2, 3, 4] as i}
                <span class="dot" class:on={i < likedTotal}></span>
              {/each}
            </span>
            <span class="count">{artistCountLabel(likedTotal)}</span>
          </div>
          <p class="sr" role="status" aria-live="polite">
            {artistCountLabel(likedTotal)}. {progressHint(likedTotal)}{insertNote ? ` ${insertNote}.` : ""}
          </p>
        {/if}

        {#if actionError}
          <p class="alert" role="alert" in:fadeIn>
            <Icon name="alert" size={16} />
            <span>{actionError.message}</span>
            {#if actionError.retry}
              <button class="retry" type="button" on:click={actionError.retry}>Réessayer</button>
            {/if}
          </p>
        {/if}
        {#if similarNotice}
          <p class="note" in:fadeIn>{similarNotice}</p>
        {/if}

        {#if searching && !searchItems}
          <div class="grid" aria-busy="true">
            {#each [1, 2, 3, 4, 5, 6, 7, 8] as _}
              <OnboardingArtistCard placeholder />
            {/each}
          </div>
        {:else if searchError}
          <div class="failure inline" in:fadeIn>
            <span class="failure-icon"><Icon name="alert" size={22} /></span>
            <h2>La recherche n'a pas abouti.</h2>
            <div class="failedslot">
              <LoadFailed
                error={searchError}
                notFound="Le serveur n'a pas répondu. Votre saisie est conservée."
                on:retry={() => runSearch(q)}
              />
            </div>
            <div class="actions">
              <button class="ghost" type="button" on:click={() => runSearch(q)}>
                <Icon name="refresh" size={16} /> Réessayer
              </button>
              <button class="ghost" type="button" on:click={clearSearch}>Effacer la recherche</button>
            </div>
          </div>
        {:else if query && !searchItems.length}
          <div class="failure inline" in:fadeIn>
            <p class="empty-title">Aucun artiste ne porte ce nom.</p>
            <div class="actions">
              <button class="ghost" type="button" on:click={clearSearch}>Effacer la recherche</button>
            </div>
          </div>
        {:else if !items.length && listError}
          <div class="failure inline" in:fadeIn>
            <span class="failure-icon"><Icon name="cloudOff" size={22} /></span>
            <h2>Impossible de charger les artistes.</h2>
            <div class="failedslot">
              <LoadFailed
                error={listError}
                notFound="Le serveur n'a pas répondu. Vous pourrez réessayer sans perdre votre compte."
                on:retry={retryFeed}
              />
            </div>
            <div class="actions">
              <button class="ghost" type="button" on:click={retryFeed}>
                <Icon name="refresh" size={16} /> Réessayer
              </button>
            </div>
          </div>
        {:else if !items.length && unavailable}
          <div class="failure inline" in:fadeIn>
            <span class="failure-icon"><Icon name="cloudOff" size={22} /></span>
            <h2>Aucun artiste n'est disponible pour l'instant.</h2>
            <p class="cause">{feedCause(feed && feed.error)}</p>
            <div class="actions">
              <button class="ghost" type="button" on:click={retryFeed}>
                <Icon name="refresh" size={16} /> Réessayer
              </button>
            </div>
            {#if canPass}
              <div class="escape">
                <button class="ghost" type="button" on:click={passArtists}>Passer pour l'instant</button>
              </div>
            {/if}
          </div>
        {:else if !items.length}
          <p class="hint preparing">Nous préparons une sélection…</p>
          <div class="grid" aria-busy="true">
            {#each [1, 2, 3, 4, 5, 6, 7, 8] as _}
              <OnboardingArtistCard placeholder />
            {/each}
          </div>
        {:else}
          {#if searchNote}
            <p class="note">{searchNote} Résultats de votre bibliothèque.</p>
          {/if}
          <div
            class="grid"
            role="group"
            aria-label="Artistes suggérés"
            aria-busy={searching}
            bind:this={gridEl}
            in:fadeIn
          >
            {#each visible as artist, i (idOf(artist))}
              <OnboardingArtistCard
                {artist}
                index={i}
                liked={likedIds.has(idOf(artist))}
                pending={pendingIds.has(idOf(artist))}
                similarOf={artist.parent_name || ""}
                tabindex={i === focusIndex ? 0 : -1}
                on:toggle={(event) => toggle(event.detail)}
                on:nav={(event) => onGridNav(event.detail.key)}
                on:focused={(event) => (focusId = idOf(visible[event.detail.index]))}
              />
              {#if slots.get(idOf(artist)) === "pending"}
                <OnboardingArtistCard placeholder />
                <OnboardingArtistCard placeholder />
              {:else if slots.get(idOf(artist)) === "failed"}
                <div class="simfail" role="status">
                  <span>Les artistes proches n'ont pas pu être chargés.</span>
                  <button class="link" type="button" on:click={() => retryRelated(idOf(artist))}>
                    Réessayer
                  </button>
                </div>
              {/if}
            {/each}
          </div>
          {#if visible.length < items.length || next}
            {#if !query}
              <div class="sentinel" use:nearEnd={grow} aria-hidden="true"></div>
            {/if}
          {/if}
          {#if endOfList}
            <p class="note end">Vous avez fait le tour des suggestions.</p>
          {/if}
          {#if loadingMore}
            <p class="label-mono loadingmore">Chargement…</p>
          {/if}
        {/if}
      </div>
    </main>
  {/if}
</div>

<style>
  .page {
    min-height: 100dvh;
    display: flex;
    flex-direction: column;
    background: var(--bg);
    padding-bottom: calc(24px + var(--safe-bottom, env(safe-area-inset-bottom, 0px)));
  }
  .bar {
    position: sticky;
    top: 0;
    z-index: 6;
    display: flex;
    align-items: center;
    gap: 12px;
    height: 56px;
    padding: 0 20px;
    padding-top: var(--safe-top, env(safe-area-inset-top, 0px));
    background: var(--bg);
  }
  .left {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .iconbtn {
    width: 44px;
    height: 44px;
    display: grid;
    place-items: center;
    margin-left: -10px;
    border-radius: 50%;
    color: var(--tx);
  }
  .iconbtn:hover {
    background: var(--s2);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 9px;
    min-width: 0;
  }
  .mark {
    width: 26px;
    height: 26px;
    flex: none;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: var(--ac);
    color: var(--onac);
  }
  .brandname {
    font-family: var(--font-display);
    font-size: 16px;
    font-weight: 700;
    letter-spacing: -0.02em;
  }
  .steplabel {
    margin: 0 auto;
    white-space: nowrap;
  }
  .finish {
    min-height: 44px;
    padding: 0 18px;
    border-radius: 999px;
    background: var(--btn);
    color: var(--onbtn);
    font-size: 14px;
    font-weight: 700;
  }
  .finish[aria-disabled="true"] {
    opacity: 0.55;
  }
  .rail {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 2px;
    background: var(--ln);
  }
  .rail span {
    display: block;
    height: 100%;
    width: 100%;
    background: var(--ac-text);
    transform-origin: 0 50%;
    /* The step's progress: a transform, never a width (M1) — the same 200 ms
       in-place move as every other indicator. */
    transition: transform var(--dur-base) var(--ease-standard);
  }
  .status {
    margin: 0;
    padding: 8px 20px;
    font-size: 13px;
    line-height: 1.4;
    color: var(--ac-text);
  }
  .banner {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    margin: 8px 20px 0;
    padding: 10px 12px;
    border: 1px solid var(--ln);
    border-radius: 12px;
    background: var(--s1);
    color: var(--dim);
    font-size: 12.5px;
    line-height: 1.45;
  }
  .banner :global(svg) {
    flex: none;
    margin-top: 1px;
  }
  .pane {
    width: 100%;
    max-width: 1220px;
    margin: 0 auto;
    padding: 4px 20px 0;
  }
  .flow {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .head {
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding-top: 8px;
  }
  .titles {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: 26px;
    font-weight: 700;
    line-height: 1.15;
    letter-spacing: -0.03em;
  }
  .searchwrap {
    position: relative;
    width: 100%;
    max-width: 460px;
  }
  .search {
    width: 100%;
    height: 44px;
    padding: 0 44px 0 42px;
    border: 1px solid var(--ln);
    border-radius: 999px;
    background: var(--s1);
    color: var(--tx);
    font-size: 14.5px;
  }
  .search:focus {
    border-color: var(--ac-text);
  }
  .search::-webkit-search-cancel-button {
    display: none;
  }
  .sicon {
    position: absolute;
    left: 14px;
    top: 50%;
    transform: translateY(-50%);
    display: flex;
    color: var(--dim);
    pointer-events: none;
  }
  .clear {
    position: absolute;
    right: 0;
    top: 0;
    width: 44px;
    height: 44px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    color: var(--dim);
  }
  .clear:hover {
    color: var(--tx);
    background: var(--s2);
  }
  .progressrow {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 24px;
    flex-wrap: wrap;
  }
  .dots {
    display: inline-flex;
    gap: 5px;
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--ln);
  }
  .dot.on {
    background: var(--ac-text);
  }
  .count {
    font-family: var(--font-mono);
    font-size: 12px;
    font-weight: 500;
    letter-spacing: 0.02em;
  }
  .hint {
    margin: 0;
    color: var(--dim);
    font-size: 12.5px;
    line-height: 1.45;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 12px;
    margin-top: 2px;
  }
  .sentinel {
    height: 1px;
  }
  .simfail {
    grid-column: span 2;
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    padding: 0 8px;
    color: var(--dim);
    font-size: 12.5px;
  }
  .link {
    min-height: 44px;
    padding: 0 6px;
    color: var(--ac-text);
    font-size: 12.5px;
    font-weight: 600;
    text-decoration: underline;
    text-underline-offset: 3px;
  }
  .alert {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    margin: 0;
    padding: 12px;
    border: 1px solid var(--ln);
    border-radius: 12px;
    background: var(--s1);
    font-size: 13px;
    line-height: 1.45;
  }
  .alert :global(svg) {
    flex: none;
    margin-top: 1px;
    color: var(--ac-text);
  }
  .retry {
    margin-left: auto;
    min-height: 44px;
    padding: 0 10px;
    color: var(--ac-text);
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
  }
  .note {
    margin: 0;
    color: var(--dim);
    font-size: 12.5px;
    line-height: 1.45;
  }
  .end {
    text-align: center;
    padding: 10px 0 4px;
  }
  .loadingmore {
    margin: 0;
    text-align: center;
  }
  .failure {
    max-width: 520px;
    margin: 24px auto 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    text-align: center;
  }
  .failure.inline {
    margin: 12px auto 0;
    padding: 24px 16px;
    border: 1px solid var(--ln);
    border-radius: var(--r-card);
    background: var(--s1);
  }
  .failure-icon {
    width: 56px;
    height: 56px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: var(--s2);
    color: var(--dim);
  }
  .failure h1,
  .failure h2 {
    margin: 0;
    font-family: var(--font-display);
    font-size: 20px;
    font-weight: 700;
    line-height: 1.2;
  }
  .cause {
    margin: 0;
    color: var(--dim);
    font-size: 13px;
    line-height: 1.45;
  }
  .empty-title {
    margin: 0;
    font-family: var(--font-display);
    font-size: 18px;
    font-weight: 600;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
    justify-content: center;
  }
  .ghost {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    min-height: 44px;
    padding: 0 18px;
    border: 1px solid var(--ln);
    border-radius: 999px;
    color: var(--tx);
    font-size: 14px;
    font-weight: 700;
  }
  .ghost:hover {
    background: var(--s2);
  }
  .escape {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    margin-top: 8px;
    padding-top: 16px;
    border-top: 1px solid var(--ln);
    width: 100%;
  }
  .preparing {
    text-align: center;
  }
  .skeleton {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding-top: 10px;
  }
  .sk {
    display: block;
    border-radius: 8px;
    background: var(--s2);
  }
  .sk.title {
    width: 60%;
    height: 30px;
  }
  .sk.line {
    width: 42%;
    height: 12px;
  }
  .sk.search {
    width: min(460px, 100%);
    height: 44px;
    border-radius: 999px;
  }
  .failedslot {
    width: 100%;
  }
  .failedslot :global(.failed) {
    margin: 0;
    padding: 0;
    gap: 8px;
  }
  .failedslot :global(.failed p) {
    margin: 0;
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
    border: 0;
  }
  @media (min-width: 641px) {
    .bar {
      height: 64px;
      padding: 0 32px;
      padding-top: var(--safe-top, env(safe-area-inset-top, 0px));
      gap: 16px;
    }
    .pane {
      padding: 4px 32px 0;
      max-width: 1244px;
    }
    .head {
      flex-direction: row;
      align-items: flex-end;
      justify-content: space-between;
      gap: 24px;
      padding-top: 12px;
    }
    .titles {
      flex: 1;
      min-width: 0;
    }
    .searchwrap {
      width: min(420px, 42%);
      flex: none;
    }
    .grid {
      grid-template-columns: repeat(3, minmax(0, 1fr));
      gap: 16px;
    }
    .simfail {
      grid-column: span 2;
    }
    .status,
    .banner {
      margin-left: 32px;
      margin-right: 32px;
    }
    .banner {
      margin-top: 12px;
    }
    h1 {
      font-size: 34px;
    }
  }
  @media (min-width: 900px) {
    .grid {
      grid-template-columns: repeat(4, minmax(0, 1fr));
    }
    h1 {
      font-size: 40px;
      line-height: 1.1;
    }
  }
  @media (min-width: 1100px) {
    .grid {
      grid-template-columns: repeat(5, minmax(0, 1fr));
    }
  }
  @media (min-width: 1280px) {
    .bar {
      height: 72px;
    }
    .grid {
      grid-template-columns: repeat(6, minmax(0, 1fr));
    }
  }
  @media (max-width: 380px) {
    .brandname {
      display: none;
    }
    h1 {
      font-size: 23px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .rail span {
      transition: none;
    }
    .finish,
    .ghost {
      transition: none;
    }
  }
</style>
