<script>
  // Everything your artists released lately, as ONE list of titles read like a
  // playlist (newest release first, each release in its own order): a tap
  // plays from that title on, "Tout lire" plays it all. The server keeps it
  // fresh on its own; this page only reads it, and opening it marks
  // everything seen. A release whose titles the server has not read yet is
  // shown as an album to open, under the list.
  import { onMount } from "svelte";
  import { push } from "svelte-spa-router";
  import { api } from "../lib/api.js";
  import { player } from "../lib/stores.js";
  import { toggleEntityFavorite, entityMeta } from "../lib/actions.js";
  import { CAN_KEEP } from "../lib/remote/mode.js";
  import Card from "../components/Card.svelte";
  import Icon from "../components/Icon.svelte";
  import Cover from "../components/Cover.svelte";
  import TrackList from "../components/TrackList.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import BackButton from "../components/BackButton.svelte";

  let items = [];
  let tracks = [];
  let loading = true;
  let loadError = null;

  $: listed = new Set(tracks.map((t) => t.release?.deezer_id).filter(Boolean));
  $: albumsOnly = items.filter((r) => !listed.has(r.deezer_id));

  // §6 groups the page by RELEASE: the payload already carries the release of
  // every title, so the cards need no request and keep the server's own order
  // (newest first, each release's titles in its own order).
  const KIND = { album: "Album", single: "Single", ep: "EP", compile: "Compilation", compilation: "Compilation" };
  $: groups = groupReleases(tracks);
  $: allTracks = groups.flatMap((g) => g.tracks);
  // Titles are fetched on demand: 120 release cards must not mount 400 rows
  // (and their covers) before anyone asks for them.
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
  }

  function playAll() {
    if (allTracks.length) player.playQueue(allTracks, 0, { kind: "releases" });
  }

  onMount(async () => {
    try {
      const d = await api.swr("/releases", take);
      take(d);
      if (d.unseen) api.releasesSeen().catch(() => {});
    } catch (e) {
      loadError = e;
      /* an empty page says so below */
    } finally {
      loading = false;
    }
  });
</script>

<BackButton />
<div class="headline">
  <h1 class="page-title">Nouveautés</h1>
  <p class="lead muted">
    Les sorties récentes de tes artistes favoris, de ceux que tu écoutes le plus et de quelques
    artistes très proches d'eux.
  </p>
</div>

{#if loading && !tracks.length && !items.length}
  <Skeleton kind="list" />
{:else if !tracks.length && !items.length}
  <div class="empty">
    <svg class="rings" viewBox="0 0 64 64" width="64" height="64" aria-hidden="true">
      <circle cx="32" cy="32" r="28" /><circle cx="32" cy="32" r="18" /><circle cx="32" cy="32" r="6" />
    </svg>
    <p class="empty-title">Rien de neuf pour l'instant</p>
    <p class="empty-text">
      {#if loadError}
        Les nouveautés n'ont pas pu être chargées ({loadError.message || "réseau"}).
      {:else}
        Ajoute des artistes à tes favoris : leurs prochaines sorties apparaîtront ici.
      {/if}
    </p>
    <button class="btn btn-primary" on:click={() => push("/search")}><Icon name="search" size={16} /> Explorer</button>
  </div>
{:else}
  {#if tracks.length}
    <div class="bar">
      <button class="btn btn-primary" on:click={playAll}><Icon name="play" size={15} /> Tout lire · {allTracks.length}</button>
      <span class="chip static"><Icon name="sort" size={15} /> Groupé par sortie</span>
    </div>
    <div class="rels">
      {#each groups as g (g.id)}
        <section class="rel">
          <div class="rel-cover"><Cover src={g.meta.cover} alt={g.meta.title} kind="album" /></div>
          <div class="rel-body">
            <span class="label-mono kind">
              {KIND[g.meta.kind] || "Sortie"} · {g.tracks.length} titre{g.tracks.length > 1 ? "s" : ""}{#if g.isNew}
                · Nouveau{/if}
            </span>
            <h2 class="rel-title">{g.meta.title}</h2>
            <span class="muted rel-artist">{g.artist}</span>
            <div class="rel-actions">
              <button class="btn btn-primary btn-sm" on:click={() => playGroup(g)}><Icon name="play" size={14} /> Lire</button>
              {#if CAN_KEEP}
                <button
                  class="btn btn-ghost btn-sm"
                  on:click={() => favoriteGroup(g)}
                  disabled={saved.has(g.id)}>
                  <Icon name={saved.has(g.id) ? "heartFilled" : "heart"} size={15} />
                  {saved.has(g.id) ? "Dans tes favoris" : "Ajouter aux favoris"}
                </button>
              {/if}
              <button
                class="btn btn-ghost btn-sm"
                on:click={() => toggleGroup(g.id)}
                aria-expanded={opened.has(g.id)}>
                <Icon name={opened.has(g.id) ? "chevronUp" : "chevronDown"} size={15} />
                {opened.has(g.id) ? "Masquer les titres" : `Voir les ${g.tracks.length} titre${g.tracks.length > 1 ? "s" : ""}`}
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
    <h2 class="section-title more">{tracks.length ? "Autres sorties" : "Sorties"}</h2>
    <div class="grid">
      {#each albumsOnly as r (r.deezer_id)}
        <div class="cell" class:new={r.new}><Card item={card(r)} kind="album" /></div>
      {/each}
    </div>
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
  /* A caption, not a control: one grouping exists, so the chip states it
     rather than offering a menu with a single item. */
  .chip.static {
    cursor: default;
    color: var(--dim);
  }
  .chip.static:hover {
    background: none;
  }
  .rels {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .rel {
    display: flex;
    align-items: flex-start;
    gap: 20px;
    padding: 16px;
    background: var(--s1);
    border: 1px solid var(--ln);
    border-radius: var(--r-card);
    /* 120 release cards are cheap to keep in the DOM and free to skip. */
    content-visibility: auto;
    contain-intrinsic-size: auto 220px;
  }
  .rel-cover {
    flex: none;
    width: 132px;
  }
  .rel-body {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    flex: 1;
  }
  .kind {
    font-size: 11px;
  }
  .rel-title {
    margin: 0;
    font-size: 28px;
    font-weight: 700;
    letter-spacing: -0.02em;
    line-height: 1.1;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .rel-artist {
    font-size: 14px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .rel-actions {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    margin-top: 8px;
  }
  .rel-actions .btn:disabled {
    opacity: 0.55;
  }
  .rel-tracks {
    margin-top: 10px;
    padding-top: 8px;
    border-top: 1px solid var(--ln);
  }
  .more {
    margin: 28px 0 12px;
  }
  .cell {
    position: relative;
  }
  /* "Nouveau" is worn by the card, not over its sleeve: the badge sat on the
     cover and hid the art it was pointing at. */
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
  @media (max-width: 640px) {
    .rel {
      gap: 14px;
      padding: 12px;
    }
    .rel-cover {
      width: 92px;
    }
    .rel-title {
      font-size: 22px;
    }
    .rel-actions {
      gap: 6px;
    }
  }
</style>
