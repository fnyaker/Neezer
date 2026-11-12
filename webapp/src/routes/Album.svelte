<script>
  import { entityRef } from "../lib/entity.js";
  import { push } from "svelte-spa-router";
  import { REMOTE, CAN_KEEP, CAN_ADMIN, CAN_BROWSE } from "../lib/remote/mode.js";
  import { api } from "../lib/api.js";
  import { player, toasts, isAdmin, openExport, openGenreBulk } from "../lib/stores.js";
  import { toggleEntityFavorite, entityMeta, downloadTracks } from "../lib/actions.js";
  import { duration as fmtDuration } from "../lib/format.js";
  import Cover from "../components/Cover.svelte";
  import TrackList from "../components/TrackList.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import LoadFailed from "../components/LoadFailed.svelte";
  import Icon from "../components/Icon.svelte";

  export let params = {};

  let id = null;
  let data = null;
  let fav = false;
  let loading = true;
  let loadError = null; // why there is nothing to show (components/LoadFailed)
  // Tracks Deezer lists on this album that the library does not hold:
  // null = loading, [] = none (or Deezer unreachable).
  let onDeezer = null;
  let deezerDown = false;

  $: if (params.id && params.id !== id) {
    id = params.id;
    load(id);
  }

  // Sequence guard: navigating quickly between albums must not let an EARLIER
  // response (delayed by the network / offline retries) overwrite the page.
  let loadSeq = 0;
  async function load(albumId) {
    const mine = ++loadSeq;
    loading = true;
    loadError = null;
    data = null;
    onDeezer = null;
    deezerDown = false;
    try {
      // Stale-while-revalidate: an album you've opened before paints straight
      // from disk instead of showing a skeleton for the length of a round-trip.
      await api.swr("/album/" + albumId, (r) => {
        if (mine !== loadSeq) return;
        data = r;
        fav = !!r.album?.is_favorite; // correct heart state on open
        loading = false;
      });
      // After the page is on screen: the rest of the tracklist.
      if (mine === loadSeq && data?.library) {
        api
          .albumOnDeezer(albumId)
          .then((r) => {
            if (mine === loadSeq) onDeezer = r.tracks || [];
          })
          .catch(() => {
            if (mine !== loadSeq) return;
            onDeezer = [];
            deezerDown = true;
          });
      }
    } catch (e) {
      if (mine === loadSeq && !data) {
        data = null;
        loadError = e;
      }
    }
    if (mine === loadSeq) loading = false;
  }

  function playAll() {
    if (data?.tracks?.length) player.playQueue(data.tracks, 0, { kind: "album", id });
  }
  function shufflePlay() {
    if (data?.tracks?.length) player.shufflePlay(data.tracks, { kind: "album", id });
  }

  async function toggleFav() {
    const next = !fav;
    if (await toggleEntityFavorite("album", id, next, entityMeta("album", data?.album || {}))) fav = next;
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

  $: total = (data?.tracks || []).reduce((s, t) => s + (t.duration || 0), 0);
  $: facts = data?.album
    ? [
        "Album",
        data.album.year,
        `${data.tracks.length} titre${data.tracks.length > 1 ? "s" : ""}`,
        fmtDuration(total),
      ]
        .filter(Boolean)
        .join(" · ")
    : "";
</script>

{#if loading}
  <Skeleton kind="header" />
  <Skeleton kind="list" />
{:else if !data?.album}
  <LoadFailed error={loadError} notFound="Album introuvable." on:retry={() => load(id)} />
{:else}
  <div class="fade-in">
    <!-- §6: the same band as the artist and playlist pages — `--s2`, a
         hairline, a 200 px sleeve and the title at 72. -->
    <header class="banner gh">
      <div class="art"><Cover src={data.album.cover} alt={data.album.title} kind="album" eager /></div>
      <div class="meta">
        <span class="kind">{facts}</span>
        <h1>{data.album.title}</h1>
        <button class="artist" on:click={() => entityRef(data.album.artist) && push("/artist/" + entityRef(data.album.artist))}>
          {data.album.artist?.name}
        </button>
      </div>
    </header>

    <div class="actions">
      <button class="btn btn-primary" on:click={() => playAll()}><Icon name="play" size={15} /> Lire</button>
      <button class="btn btn-ghost" on:click={shufflePlay}><Icon name="shuffle" size={16} /> Mélanger</button>
      {#if !REMOTE}
        <button class="btn btn-ghost" on:click={downloadAll} disabled={dlBusy} aria-label="Télécharger l'album" title="Télécharger sur l'appareil (hors-ligne)"><Icon name="download" size={16} /> Télécharger</button>
      {/if}
      {#if CAN_KEEP}
        <button class="btn btn-ghost" on:click={() => openExport("album", id, data.album.title)} aria-label="Exporter" title="Exporter : vers Spotify, YouTube Music…, en ZIP ou en liste"><Icon name="archive" size={16} /> Exporter</button>
        <button class="btn btn-icon" class:on={fav} on:click={toggleFav} aria-label="Favori"><Icon name={fav ? "heartFilled" : "heart"} size={20} /></button>
      {/if}
      {#if $isAdmin && CAN_KEEP}
        <button
          class="btn btn-icon"
          on:click={() => openGenreBulk({ kind: "album", title: data.album.title, cover: data.album.cover, tracks: data.tracks })}
          aria-label="Donner un genre à l'album"
          title="Donner un genre à tout l'album"><Icon name="tag" size={20} /></button>
      {/if}
    </div>

    <TrackList tracks={data.tracks} numbered showAlbum={false} showCover={false} context={{ kind: "album", id }} />

    {#if data.library && onDeezer === null}
      <h2 class="section-title more">Disponible sur Deezer</h2>
      <Skeleton kind="list" count={3} />
    {:else if onDeezer?.length}
      <h2 class="section-title more">Disponible sur Deezer</h2>
      <TrackList tracks={onDeezer} showAlbum={false} showCover={false} context={{ kind: "album", id }} />
    {:else if deezerDown}
      <p class="muted down">Deezer est injoignable : la suite de l'album n'est pas affichée.</p>
    {/if}
  </div>
{/if}

<style>
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
  .artist {
    /* A link set as a button: without this the button's own padding indented
       the artist from the title and the label above it. */
    align-self: flex-start;
    padding: 0;
    font-family: var(--font-display);
    font-weight: 600;
    font-size: 16px;
    letter-spacing: -0.01em;
    color: var(--tx);
  }
  .artist:hover {
    text-decoration: underline;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    margin: 0 0 18px;
  }
  .btn-icon.on {
    color: var(--hrt);
    border-color: var(--hrt);
  }
  .more {
    margin: 28px 0 10px;
  }
  .down {
    margin: 18px 0 0;
    font-size: 0.9rem;
  }
  @media (max-width: 640px) {
    /* Same as the playlist page: one scrolling row of actions, not three. */
    .actions {
      flex-wrap: wrap;
      overflow: visible;
      padding-bottom: 2px;
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
      /* §9: the entity title is 32–40 under a finger. It was 25–27 (a 7vw
         clamp), which read as a caption beside the 96 px sleeve. The band
         stays short because the sleeve is; a long title wraps to two or three
         lines and grows the band by its own line-height, never sideways. */
      font-size: clamp(32px, 8.5vw, 40px);
    }
    .kind {
      line-height: 1.5;
    }
    .artist {
      font-size: 15px;
    }
  }
</style>
