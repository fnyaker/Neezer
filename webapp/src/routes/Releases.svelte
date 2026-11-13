<script context="module">
  // The releases feed is ONE payload, read by TWO screens: Nouveautés paints it
  // and the shelves opened from its cards regroup it. It lives at module scope
  // (the same pattern as the Flow's style rows) so a card click costs no
  // request, the back button repaints in the first frame, and two quick
  // navigations cannot race two copies of the same list. A reload, or a visit
  // after FEED_FRESH_MS, asks the server again; the releases watcher keeps the
  // copy it serves current.
  import { api } from "../lib/api.js";
  import { get } from "svelte/store";
  import { user } from "../lib/stores.js";

  const FEED_FRESH_MS = 5 * 60_000;
  let feed = null;
  let feedUser = null;
  let feedAt = 0;
  let feedPromise = null;

  // The feed is personal. Logging out and back in as somebody else does not
  // reload the SPA (the login screen replaces the shell), so a cache keyed on
  // nothing would hand the next account the previous one's releases.
  const who = () => get(user)?.name || "";

  export function readReleasesFeed() {
    return feed && feedUser === who() ? feed : null;
  }

  export function writeReleasesFeed(data) {
    feed = data;
    feedUser = who();
    feedAt = Date.now();
    return feed;
  }

  export function releasesFeedStale(ms = FEED_FRESH_MS) {
    return !feed || feedUser !== who() || Date.now() - feedAt > ms;
  }

  // Opening or deep-linking a shelf also means those releases have been seen.
  // The DISPLAYED count stays this visit's (`take` reads it before this runs);
  // the shared feed is what a later back navigation reads, and it must not
  // announce the same releases as new twice.
  export function clearReleasesUnseen() {
    if (feed) feed = { ...feed, unseen: 0 };
    return feed;
  }

  export function loadReleasesFeed() {
    if (feedPromise) return feedPromise;
    const requester = who();
    feedPromise = api.swr("/releases", () => {}).then(
      (d) => {
        feedPromise = null;
        // The account changed while the request was in flight: answer the
        // caller that asked, never stamp those releases as the new account's.
        if (who() !== requester) return d;
        writeReleasesFeed(d);
        if (d.unseen) {
          clearReleasesUnseen();
          api.releasesSeen().catch(() => {});
        }
        return d;
      },
      (e) => {
        feedPromise = null;
        throw e;
      }
    );
    return feedPromise;
  }

  export const SHELF_COPY = {
    artists: {
      title: "Tes artistes",
      short: "Les sorties de tes favoris et des artistes que tu écoutes.",
      description:
        "Les sorties récentes des artistes que tu suis et de ceux que tu écoutes le plus.",
    },
    discovery: {
      title: "Découverte",
      short: "Des artistes proches de tes favoris, et d'autres à découvrir.",
      description:
        "Des artistes proches de tes favoris, et des sorties que tu n'as pas encore écoutées.",
    },
  };

  /**
   * Split the flat `tracks` list on each release card's `close_to`:
   *   artists   — favourites and the artists this person listens to (no close_to)
   *   discovery — artists close to a favourite (close_to set)
   * A release is counted once and its titles keep the release's own order;
   * both sides keep the API's newest-release-first order. The cards of the
   * payload are the authority on close_to, not the track's release stub.
   */
  export function buildReleaseShelves(payload) {
    const cards = new Map();
    for (const r of payload?.releases || []) {
      if (r?.deezer_id != null) cards.set(String(r.deezer_id), r);
    }
    const make = (key) => ({ key, releases: [], tracks: [], seenReleases: new Set(), seenTracks: new Set() });
    const buckets = { artists: make("artists"), discovery: make("discovery") };

    for (const t of payload?.tracks || []) {
      const rel = t?.release;
      if (!rel?.deezer_id) continue;
      const rid = String(rel.deezer_id);
      const card = cards.get(rid) || null;
      const bucket = card?.close_to ? buckets.discovery : buckets.artists;
      const tid = t?.deezer_id != null ? String(t.deezer_id) : "";
      if (tid && bucket.seenTracks.has(tid)) continue;
      if (tid) bucket.seenTracks.add(tid);
      bucket.tracks.push(t);
      if (!bucket.seenReleases.has(rid)) {
        bucket.seenReleases.add(rid);
        bucket.releases.push(
          card || {
            deezer_id: rel.deezer_id,
            title: rel.title || "",
            cover: rel.cover || null,
            kind: rel.kind || "",
            released: rel.released || null,
            nb_tracks: rel.nb_tracks || 0,
            close_to: null,
          }
        );
      }
    }

    const closeNames = [
      ...new Set(buckets.discovery.releases.map((r) => r.close_to).filter(Boolean)),
    ];
    return {
      artists: { key: "artists", releases: buckets.artists.releases, tracks: buckets.artists.tracks },
      discovery: {
        key: "discovery",
        releases: buckets.discovery.releases,
        tracks: buckets.discovery.tracks,
        closeNames,
      },
    };
  }
</script>

<script>
  // Everything newly released by the artists this person follows or listens
  // to — as two shelves (their artists, and what is close to those artists)
  // plus the per-release list underneath. The server keeps it fresh on its
  // own; this page only reads it, and opening it marks it seen.
  import { onMount } from "svelte";
  import { push } from "svelte-spa-router";
  import { failureCause, isDeezerDenied, isDeezerUnreachable } from "../lib/api.js";
  import { player } from "../lib/stores.js";
  import { toggleEntityFavorite, entityMeta } from "../lib/actions.js";
  import { CAN_KEEP } from "../lib/remote/mode.js";
  import { rememberScreen, recallScreen } from "../lib/nav.js";
  import Card from "../components/Card.svelte";
  import Icon from "../components/Icon.svelte";
  import Cover from "../components/Cover.svelte";
  import TrackList from "../components/TrackList.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import LoadFailed from "../components/LoadFailed.svelte";
  import ReleaseCard from "../components/ReleaseCard.svelte";

  let items = [];
  let tracks = [];
  let pending = 0;
  let newCount = 0;
  let loading = true;
  let loadError = null;
  // "Toutes les sorties" is a view state, and the card click is a traversal
  // away from this entry: remember it (lib/nav.js) so Back returns to the page
  // as it was left — and so the scroll restore is not clamped by a list that
  // collapsed in the meantime.
  const left = recallScreen();
  let showAll = !!(left && left.releasesAll);

  const KIND = { album: "Album", single: "Single", ep: "EP", compile: "Compilation", compilation: "Compilation" };

  $: shelves = buildReleaseShelves({ releases: items, tracks });
  $: cards = ["artists", "discovery"]
    .map((key) => ({
      ...shelves[key],
      title: SHELF_COPY[key].title,
      subtitle: SHELF_COPY[key].short,
    }))
    .filter((s) => s.tracks.length);
  $: listed = new Set(tracks.map((t) => t.release?.deezer_id).filter(Boolean));
  $: albumsOnly = items.filter((r) => !listed.has(r.deezer_id));
  $: groups = groupReleases(tracks);
  $: allTracks = groups.flatMap((g) => g.tracks);
  $: releaseTotal = groups.length + albumsOnly.length;
  $: freshLabel = `${newCount} nouvelle${newCount > 1 ? "s" : ""} sortie${newCount > 1 ? "s" : ""}`;
  // When there is nothing to put in a shelf (the server has not read the
  // titles yet), the only thing worth showing is the per-release list: open it
  // instead of leaving the page on a collapsed heading.
  $: if (!loading && !cards.length && releaseTotal && !showAll) showAll = true;

  function toggleAll() {
    showAll = !showAll;
    rememberScreen({ releasesAll: showAll });
  }

  // The flat payload already carries each title's release, so both the shelves
  // and this page's cards need no request and keep the server's order (newest
  // release first, each release's titles in its own order).
  let opened = new Set();
  function toggleGroup(id) {
    const next = new Set(opened);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    opened = next;
  }

  function groupReleases(list) {
    const out = [];
    const byId = new Map();
    for (const t of list) {
      const r = t.release;
      if (!r?.deezer_id) continue;
      let g = byId.get(r.deezer_id);
      if (!g) {
        g = {
          id: r.deezer_id,
          meta: r,
          artist: t.display_artist || t.artist?.name || "",
          tracks: [],
          isNew: false,
        };
        byId.set(r.deezer_id, g);
        out.push(g);
      }
      g.tracks.push(t);
      if (t.new) g.isNew = true;
    }
    return out;
  }

  function playGroup(g) {
    if (g.tracks.length) player.playQueue(g.tracks, 0, { kind: "album", id: g.id });
  }

  let saved = new Set();
  async function favoriteGroup(g) {
    const meta = {
      title: g.meta.title,
      cover: g.meta.cover,
      artist: { deezer_id: g.tracks[0]?.artist?.deezer_id, name: g.artist },
    };
    if (await toggleEntityFavorite("album", g.id, true, entityMeta("album", meta))) {
      const next = new Set(saved);
      next.add(g.id);
      saved = next;
    }
  }

  function card(r) {
    const when = r.released ? new Date(r.released + "T12:00:00") : null;
    const date = when && !isNaN(when) ? when.toLocaleDateString("fr-FR", { day: "numeric", month: "short" }) : "";
    const kind = r.kind === "single" ? "Single" : r.kind === "ep" ? "EP" : "";
    const who = r.close_to ? `${r.artist.name} · proche de ${r.close_to}` : r.artist.name;
    return { ...r, subtitle: [who, kind, date].filter(Boolean).join(" · ") };
  }

  function take(d) {
    items = d.releases || [];
    tracks = d.tracks || [];
    pending = d.pending || 0;
    // The count this visit can announce. The shared feed's own `unseen` is
    // cleared once /releases/seen has been posted, so coming back here does
    // not call the same releases new a second time.
    newCount = d.unseen || 0;
    loading = false;
  }

  function playAll() {
    if (allTracks.length) player.playQueue(allTracks, 0, { kind: "releases" });
  }

  async function load() {
    loading = !items.length && !tracks.length;
    loadError = null;
    try {
      take(await loadReleasesFeed());
    } catch (e) {
      if (!items.length && !tracks.length) loadError = e;
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    const cached = readReleasesFeed();
    if (cached) {
      take(cached);
      if (releasesFeedStale()) load(); // repaint from memory, correct quietly
    } else {
      load();
    }
    // The next click on this screen is a card, i.e. the shelf route. Fetch its
    // (6.5 kB) chunk while the cards are being read, during idle, so the click
    // itself has no code to wait for — the data is already here either way.
    const warm = () => import("./ReleaseShelf.svelte").catch(() => {});
    if ("requestIdleCallback" in window) requestIdleCallback(warm, { timeout: 2500 });
    else setTimeout(warm, 1200);
  });
</script>

<div class="headline">
  <h1 class="page-title">Nouveautés</h1>
  <p class="lead muted">
    Les sorties récentes de tes artistes favoris, de ceux que tu écoutes le plus et de quelques
    artistes très proches d'eux.
  </p>
</div>

{#if loading && !items.length && !tracks.length}
  <Skeleton kind="list" />
{:else if loadError && !items.length && !tracks.length}
  <div class="failure">
    <LoadFailed
      error={loadError}
      notFound={`Les nouveautés n'ont pas pu être chargées (${failureCause(loadError)}).`}
      on:retry={load}
    />
    {#if !isDeezerUnreachable(loadError) && !isDeezerDenied(loadError)}
      <div class="retry">
        <button class="btn btn-primary" on:click={load}><Icon name="refresh" size={15} /> Réessayer</button>
      </div>
    {/if}
  </div>
{:else if !items.length && !tracks.length}
  <div class="empty">
    <svg class="rings" viewBox="0 0 64 64" width="64" height="64" aria-hidden="true">
      <circle cx="32" cy="32" r="28" /><circle cx="32" cy="32" r="18" /><circle cx="32" cy="32" r="6" />
    </svg>
    <p class="empty-title">Rien de neuf pour l'instant</p>
    <p class="empty-text">
      Ajoute des artistes à tes favoris : leurs prochaines sorties apparaîtront ici.
    </p>
    <button class="btn btn-primary" on:click={() => push("/search")}><Icon name="search" size={16} /> Explorer</button>
  </div>
{:else}
  <div class="bar">
    {#if allTracks.length}
      <button class="btn btn-primary" on:click={playAll}><Icon name="play" size={15} /> Tout lire · {allTracks.length}</button>
    {/if}
    {#if newCount}
      <span class="fresh" aria-label={freshLabel}>{freshLabel}</span>
    {/if}
  </div>

  {#if cards.length}
    <div class="shelves-wrap">
      <div class="shelves" class:two={cards.length > 1}>
        {#each cards as s (s.key)}
          <ReleaseCard shelf={s} />
        {/each}
      </div>
    </div>
  {/if}

  {#if releaseTotal}
    <section class="all">
      <div class="section-head all-head">
        <h2 class="section-title">Toutes les sorties <span class="mono count">{releaseTotal}</span></h2>
        <button class="btn btn-ghost btn-sm" on:click={toggleAll} aria-expanded={showAll}>
          <Icon name={showAll ? "chevronUp" : "chevronDown"} size={15} />
          {showAll ? "Replier" : "Afficher"}
        </button>
      </div>
      {#if showAll}
        <p class="muted all-hint">Regroupées par sortie, la plus récente d'abord.</p>
        {#if pending}
          <p class="muted all-hint">
            {pending} sortie{pending > 1 ? "s" : ""} n'{pending > 1 ? "ont" : "a"} pas encore
            {pending > 1 ? "leurs" : "son"} titre{pending > 1 ? "s" : ""} : le serveur
            {pending > 1 ? "les" : "le"} lit en arrière-plan.
          </p>
        {/if}
        {#if groups.length}
          <div class="rels">
            {#each groups as g (g.id)}
              <section class="rel">
                <div class="rel-cover"><Cover src={g.meta.cover} alt={g.meta.title} kind="album" /></div>
                <div class="rel-body">
                  <span class="label-mono kind">
                    {KIND[g.meta.kind] || "Sortie"} · {g.tracks.length} titre{g.tracks.length > 1 ? "s" : ""}{#if g.isNew}
                      · Nouveau{/if}
                  </span>
                  <h3 class="rel-title">{g.meta.title}</h3>
                  <span class="muted rel-artist">{g.artist}</span>
                  <div class="rel-actions">
                    <button class="btn btn-primary btn-sm" on:click={() => playGroup(g)}><Icon name="play" size={14} /> Lire</button>
                    {#if CAN_KEEP}
                      <button
                        class="btn btn-ghost btn-sm fav"
                        on:click={() => favoriteGroup(g)}
                        aria-label={saved.has(g.id) ? "Dans tes favoris" : "Ajouter aux favoris"}
                        disabled={saved.has(g.id)}>
                        <Icon name={saved.has(g.id) ? "heartFilled" : "heart"} size={15} />
                        <span class="lbl">{saved.has(g.id) ? "Dans tes favoris" : "Ajouter aux favoris"}</span>
                      </button>
                    {/if}
                    <button
                      class="btn btn-ghost btn-sm"
                      on:click={() => toggleGroup(g.id)}
                      aria-expanded={opened.has(g.id)}>
                      <Icon name={opened.has(g.id) ? "chevronUp" : "chevronDown"} size={15} />
                      {opened.has(g.id) ? "Masquer les titres" : g.tracks.length > 1 ? `Voir les ${g.tracks.length} titres` : "Voir le titre"}
                    </button>
                  </div>
                  {#if opened.has(g.id)}
                    <div class="rel-tracks">
                      <TrackList tracks={g.tracks} numbered showAlbum={false} showCover={false} context={{ kind: "album", id: g.id }} />
                    </div>
                  {/if}
                </div>
              </section>
            {/each}
          </div>
        {/if}
        {#if albumsOnly.length}
          <h3 class="sub-title">{groups.length ? "Autres sorties" : "Sorties"}</h3>
          <div class="grid">
            {#each albumsOnly as r (r.deezer_id)}
              <div class="cell" class:new={r.new}><Card item={card(r)} kind="album" /></div>
            {/each}
          </div>
        {/if}
      {/if}
    </section>
  {/if}
{/if}

<style>
  .headline {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-bottom: 8px;
  }
  .headline .page-title {
    margin: 0;
  }
  .lead {
    margin: 0;
    max-width: 560px;
    font-size: 14px;
    line-height: 1.5;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
    margin: 18px 0 20px;
  }
  .fresh {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 11px;
    border-radius: 999px;
    background: var(--ac);
    color: var(--onac);
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.02em;
    white-space: nowrap;
  }
  .shelves-wrap {
    container-type: inline-size;
    container-name: shelves;
  }
  .shelves {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 18px;
  }
  @container shelves (min-width: 860px) {
    .shelves.two {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }
  .all {
    margin-top: 34px;
  }
  .all-head {
    align-items: center;
  }
  .all-head .section-title {
    display: inline-flex;
    align-items: baseline;
    gap: 9px;
  }
  .count {
    font-family: var(--font-mono);
    font-size: 13px;
    font-weight: 400;
    letter-spacing: 0;
    color: var(--dim);
  }
  .all-hint {
    margin: 8px 0 0;
    font-size: 13px;
    line-height: 1.45;
  }
  .rels {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin-top: 14px;
  }
  .rel {
    display: flex;
    align-items: flex-start;
    gap: 14px;
    padding: 12px;
    background: var(--s1);
    border: 1px solid var(--ln);
    border-radius: var(--r-card);
    /* A long list of release cards is cheap to keep and free to skip. */
    content-visibility: auto;
    contain-intrinsic-size: auto 132px;
  }
  .rel-cover {
    flex: none;
    width: 84px;
  }
  .rel-body {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
    flex: 1;
  }
  .kind {
    font-size: 11px;
  }
  .rel-title {
    margin: 0;
    font-family: var(--font-display);
    font-size: 20px;
    font-weight: 700;
    letter-spacing: -0.02em;
    line-height: 1.1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rel-artist {
    font-size: 13.5px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .rel-actions {
    display: flex;
    gap: 7px;
    flex-wrap: wrap;
    margin-top: 6px;
  }
  .rel-actions .btn:disabled {
    opacity: 0.55;
  }
  .rel-tracks {
    margin-top: 10px;
    padding-top: 8px;
    border-top: 1px solid var(--ln);
  }
  .sub-title {
    margin: 24px 0 12px;
    font-family: var(--font-display);
    font-size: 19px;
    font-weight: 600;
    letter-spacing: -0.015em;
  }
  .cell {
    position: relative;
  }
  /* "Nouveau" is worn by the card, not over its sleeve. */
  .cell.new::after {
    content: "Nouveau";
    position: absolute;
    bottom: 68px;
    left: 20px;
    padding: 2px 8px;
    border-radius: 999px;
    background: var(--ac);
    color: var(--onac);
    font-family: var(--font-mono);
    font-size: 10px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    pointer-events: none;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    text-align: center;
    padding: 56px 24px;
    border: 1px dashed var(--ln);
    border-radius: var(--r-card);
    margin-top: 20px;
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
  .failure {
    text-align: center;
  }
  .retry {
    display: flex;
    justify-content: center;
    margin-top: 16px;
  }
  @media (max-width: 640px) {
    .bar {
      gap: 8px;
    }
    .fresh {
      height: 26px;
      padding-inline: 9px;
    }
    .all {
      margin-top: 28px;
    }
    /* §9: a release is a short band — a 72 px sleeve with the label, title and
       artist beside it, then the actions on ONE row across the full card. */
    .rel {
      display: grid;
      grid-template-columns: 72px minmax(0, 1fr);
      column-gap: 12px;
      row-gap: 3px;
      padding: 10px;
      align-items: start;
    }
    .rel-cover {
      grid-column: 1;
      grid-row: 1 / span 3;
      width: 72px;
    }
    .rel-body {
      display: contents;
    }
    .rel-body > * {
      min-width: 0;
    }
    .rel-title {
      font-size: 18px;
    }
    .rel-actions {
      grid-column: 1 / -1;
      gap: 6px;
      flex-wrap: nowrap;
      overflow-x: auto;
      scrollbar-width: none;
      margin-top: 6px;
      padding-bottom: 2px;
    }
    .rel-actions::-webkit-scrollbar {
      display: none;
    }
    .rel-actions .btn {
      flex: none;
      min-height: 44px;
    }
    /* Three pills do not fit one phone row with their full labels: the
       favourite becomes its heart (the label stays in the accessible name),
       and the row still scrolls if a release title is unusually long. */
    .rel-actions .fav {
      width: 44px;
      padding: 0;
    }
    .rel-actions .fav .lbl {
      display: none;
    }
    .rel-tracks {
      grid-column: 1 / -1;
    }
  }
</style>
