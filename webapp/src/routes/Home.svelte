<script>
  // The home, rebuilt from the design in /mnt/project-files/ui/refonte-navigation.md
  // and the Sillon prototype (DESIGN §6).
  //
  // It opens on ONE decision — today's energy — and then feeds it: the styles
  // the Flow rests on, the newest titles of the artists this person follows,
  // those artists, and titles picked for them. Every section paints from the
  // last copy this device saw and corrects itself quietly; none waits on a
  // provider (every route here reads the server's database).
  import { onMount } from "svelte";
  import { get } from "svelte/store";
  import { push } from "svelte-spa-router";
  import { api } from "../lib/api.js";
  import { entityRef, trackEntityRef } from "../lib/entity.js";
  import { player, recent, openMenu } from "../lib/stores.js";
  import { buildEntityMenu } from "../lib/actions.js";
  import Card from "../components/Card.svelte";
  import Cover from "../components/Cover.svelte";
  import Avatar from "../components/Avatar.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import Icon from "../components/Icon.svelte";
  import TrackList from "../components/TrackList.svelte";
  import HomeHero from "../components/HomeHero.svelte";
  import StylePicker from "../components/StylePicker.svelte";
  import { discoveryLevel, loadDiscovery, onDiscoverySettled } from "../lib/discovery.js";

  const NEW_ROWS = 6;
  const RECO_ROWS = 6;
  const ARTISTS_SHOWN = 12;

  // -- new releases: their TITLES --------------------------------------------
  let newTracks = [];
  let unseen = 0;
  let releasesLoaded = false;
  function takeReleases(d) {
    newTracks = d.tracks || [];
    unseen = d.unseen || 0;
    releasesLoaded = true;
  }
  // On the home, one card per release (its first title: the single itself, an
  // album's opener), newest release first: six sleeves at a glance rather than
  // six titles of the same album. The page behind "Tout afficher" lists every
  // title.
  $: newShown = (() => {
    const seen = new Set();
    const out = [];
    for (const t of newTracks) {
      const r = t.release?.deezer_id || t.album?.deezer_id || t.deezer_id;
      if (seen.has(r)) continue;
      seen.add(r);
      out.push(t);
      if (out.length >= NEW_ROWS) break;
    }
    return out;
  })();
  const releaseTitle = (t) => t.release?.title || t.album?.title || t.title || "";
  const releaseCover = (t) => t.release?.cover || t.album?.cover || null;
  function playAllNew() {
    if (newTracks.length) player.playQueue(newTracks, 0, { kind: "releases" });
  }
  // A card is a RELEASE: tapping it plays that release from its first title,
  // in the whole new-titles list (the same queue "Tout lire" uses).
  function playRelease(t) {
    const i = newTracks.indexOf(t);
    if (i >= 0) player.playQueue(newTracks, i, { kind: "releases" });
  }

  // -- favourite artists ------------------------------------------------------
  let artists = [];
  let artistsLoaded = false;

  // -- for you ----------------------------------------------------------------
  let reco = null;
  let recoLoaded = false;
  let recoLoading = false;
  async function loadReco(level, seed = "") {
    recoLoading = true;
    try {
      reco = await api.recommend(level, seed);
    } catch {
      /* the rows simply stay as they were */
    } finally {
      recoLoading = false;
      recoLoaded = true;
    }
  }
  function refreshReco() {
    loadReco(get(discoveryLevel), String(Date.now() % 1e9));
  }

  // -- recently played: the collections the last titles came from ------------
  $: recentAlbums = (() => {
    const seen = new Set();
    const out = [];
    for (const t of $recent) {
      const a = t.album;
      const ref = trackEntityRef(t, "album", a);
      if (!ref || seen.has(ref)) continue;
      seen.add(ref);
      out.push({ ref, title: a.title, cover: a.cover, artist: t.artist?.name || "", track: t });
      if (out.length >= 8) break;
    }
    return out;
  })();

  // -- mixes ------------------------------------------------------------------
  let mixes = [];
  let homeLoaded = false;

  // -- what could not be loaded ----------------------------------------------
  let failures = 0;
  function loadAll() {
    releasesLoaded = artistsLoaded = homeLoaded = recoLoaded = false;
    failures = 0;
    api.swr("/releases", takeReleases).catch(() => {
      failures += 1;
      releasesLoaded = true;
    });
    api
      .swr("/me/artists", (d) => {
        artists = d.artists || [];
        artistsLoaded = true;
      })
      .catch(() => {
        failures += 1;
        artistsLoaded = true;
      });
    api
      .swr("/home", (d) => {
        mixes = d.mixes || [];
        homeLoaded = true;
      })
      .catch(() => {
        failures += 1;
        homeLoaded = true;
      });
    loadDiscovery()
      .then((lv) => {
        const path = "/recommend?d=" + encodeURIComponent(Number(lv).toFixed(2));
        return api.swr(path, (d) => {
          reco = d;
          recoLoaded = true;
        });
      })
      .catch(() => {
        failures += 1;
        recoLoaded = true;
      });
  }

  onMount(() => {
    loadAll();
    return onDiscoverySettled((lv) => loadReco(lv));
  });

  $: everythingLoaded = releasesLoaded && artistsLoaded && homeLoaded && recoLoaded;
  $: empty =
    everythingLoaded &&
    !newShown.length &&
    !artists.length &&
    !reco?.tracks?.length &&
    !recentAlbums.length &&
    !mixes.length;
  $: allFailed = empty && failures >= 4;

  function artistMenu(e, artist) {
    openMenu(e, buildEntityMenu("artist", artist, push));
  }
</script>

<div class="home">
  <HomeHero />
  <StylePicker />

  {#if newShown.length}
    <section class="releases">
      <div class="section-head">
        <h2 class="section-title">
          Nouveautés
          <span class="mono count">{newTracks.length} titre{newTracks.length > 1 ? "s" : ""}</span>
          {#if unseen}
            <span class="mono fresh" aria-label="{unseen} nouvelles sorties">{unseen}</span>
          {/if}
        </h2>
        <div class="actions">
          <button class="btn btn-ghost btn-sm" on:click={playAllNew} aria-label="Tout lire">
            <Icon name="play" size={14} /> <span class="lbl">Tout lire</span>
          </button>
          <a class="see-all" href="#/releases">Tout afficher</a>
        </div>
      </div>
      <div class="rel-grid">
        {#each newShown as t, i (t.deezer_id || i)}
          <button class="rel" on:click={() => playRelease(t)} title={releaseTitle(t)}>
            <span class="rel-art">
              <Cover src={releaseCover(t)} alt={releaseTitle(t)} kind="album" />
            </span>
            <span class="rel-title">{releaseTitle(t)}</span>
            <span class="rel-sub">{t.artist?.name}</span>
          </button>
        {/each}
      </div>
    </section>
  {:else if !releasesLoaded}
    <Skeleton kind="shelf" count={NEW_ROWS} />
  {/if}

  {#if artists.length}
    <section>
      <div class="section-head">
        <h2 class="section-title">Tes artistes</h2>
        <a class="see-all" href="#/library/artists">Tout afficher</a>
      </div>
      <div class="artists bleed-x">
        {#each artists.slice(0, ARTISTS_SHOWN) as a (a.id || a.name)}
          <a
            class="artist"
            href={"#/artist/" + entityRef(a)}
            title={a.name}
            on:contextmenu={(e) => artistMenu(e, a)}
          >
            <Avatar name={a.name} src={a.picture} size={104} />
            <span class="artist-name">{a.name}</span>
          </a>
        {/each}
      </div>
    </section>
  {/if}

  {#if reco?.tracks?.length}
    <section>
      <div class="section-head">
        <h2 class="section-title">Pour toi</h2>
        <button
          class="btn btn-ghost btn-sm"
          on:click={refreshReco}
          disabled={recoLoading}
          aria-label="Autres suggestions"
        >
          <Icon name="refresh" size={14} /> Autres suggestions
        </button>
      </div>
      <div class="reco" class:dim={recoLoading}>
        <TrackList tracks={reco.tracks.slice(0, RECO_ROWS)} radio context={{ kind: "reco" }} />
      </div>
    </section>
  {:else if !recoLoaded}
    <Skeleton kind="list" count={RECO_ROWS} />
  {/if}

  {#if recentAlbums.length}
    <section>
      <div class="section-head">
        <h2 class="section-title">Écoutés récemment</h2>
      </div>
      <div class="shortcuts">
        {#each recentAlbums as a (a.ref)}
          <button class="shortcut" on:click={() => push("/album/" + a.ref)} title={a.title}>
            <span class="sc-cover"><Cover src={a.cover} alt={a.title} kind="album" /></span>
            <span class="sc-txt">
              <span class="sc-title">{a.title}</span>
              <span class="sc-sub">{a.artist}</span>
            </span>
          </button>
        {/each}
      </div>
    </section>
  {/if}

  {#if mixes.length}
    <section>
      <div class="section-head">
        <h2 class="section-title">Tes mixes</h2>
      </div>
      <div class="shelf">
        {#each mixes as m (m.id)}<Card item={{ ...m, deezer_id: m.id }} kind="mix" />{/each}
      </div>
    </section>
  {:else if !homeLoaded}
    <Skeleton kind="shelf" />
  {/if}

  {#if empty}
    <section class="empty card">
      <span class="empty-icon"><Icon name={allFailed ? "server" : "compass"} size={26} /></span>
      {#if allFailed}
        <h2>L'accueil n'a pas pu se charger</h2>
        <p class="muted">
          Le serveur n'a pas répondu. Ta musique reste sur l'appareil : la bibliothèque et tes
          téléchargements s'ouvrent normalement.
        </p>
        <button class="btn btn-primary" on:click={loadAll}><Icon name="refresh" size={15} /> Réessayer</button>
      {:else}
        <h2>Ta bibliothèque est vide pour l'instant</h2>
        <p class="muted">
          Cherche un titre ou un artiste et lance la lecture. Ajoute des artistes à tes favoris
          pour voir ici leurs nouveautés.
        </p>
        <a class="btn btn-primary" href="#/search"><Icon name="search" size={15} /> Explorer</a>
      {/if}
    </section>
  {/if}
</div>

<style>
  .home {
    display: flex;
    flex-direction: column;
    gap: 34px;
  }
  section {
    min-width: 0;
  }
  .section-title {
    display: inline-flex;
    align-items: baseline;
    gap: 10px;
    flex-wrap: wrap;
  }
  .count {
    font-family: var(--font-mono);
    font-size: 13px;
    font-weight: 400;
    letter-spacing: 0;
    color: var(--dim);
  }
  .fresh {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 20px;
    height: 20px;
    padding: 0 6px;
    border-radius: 999px;
    background: var(--ac);
    color: var(--onac);
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 10px;
    flex: none;
  }
  /* New releases: sleeves, not rows — six at a glance. Container queries, not
     media ones: the now-playing panel and the sidebar narrow the CONTENT, and
     six columns of 110 px in that width was a squashed row. */
  .releases {
    container-type: inline-size;
  }
  .rel-grid {
    display: grid;
    grid-template-columns: repeat(6, minmax(0, 1fr));
    gap: 16px;
  }
  .rel {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
    padding: 0;
    color: var(--tx);
    text-align: left;
  }
  .rel-art {
    display: block;
    width: 100%;
    transition: transform 0.14s ease;
  }
  .rel:hover .rel-art {
    transform: translateY(-2px);
  }
  .rel-title,
  .rel-sub {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rel-title {
    font-family: var(--font-display);
    font-weight: 600;
    font-size: 14px;
    letter-spacing: -0.005em;
  }
  .rel-sub {
    margin-top: -8px;
    color: var(--dim);
    font-size: 13px;
  }
  /* Artists: the portrait is the subject, the name the caption (Avatar 104). */
  .artists {
    display: flex;
    gap: 22px;
    overflow-x: auto;
    scrollbar-width: none;
    padding-block: 2px 6px;
  }
  .artists::-webkit-scrollbar {
    display: none;
  }
  .artist {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    flex: none;
    width: 116px;
    padding: 6px 6px 10px;
    border-radius: var(--r-card);
    color: var(--tx);
    text-align: center;
    transition: background-color 0.14s ease;
  }
  .artist:hover,
  .artist:focus-visible {
    background: var(--s2);
  }
  .artist-name {
    max-width: 104px;
    font-family: var(--font-display);
    font-weight: 600;
    font-size: 13.5px;
    letter-spacing: -0.005em;
    line-height: 1.25;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .reco {
    transition: opacity 0.2s ease;
  }
  .reco.dim {
    opacity: 0.55;
  }
  /* Recently played: compact shortcuts, the whole tile the target. */
  .shortcuts {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: 8px;
  }
  .shortcut {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
    padding: 6px 12px 6px 6px;
    border-radius: 12px;
    border: 1px solid var(--ln);
    background: var(--s1);
    color: var(--tx);
    text-align: left;
    overflow: hidden;
    transition: background-color 0.12s ease, border-color 0.12s ease;
  }
  .shortcut:hover,
  .shortcut:focus-visible {
    background: var(--s2);
    border-color: var(--s2);
  }
  .sc-cover {
    flex: none;
    width: 48px;
  }
  .sc-txt {
    display: flex;
    flex-direction: column;
    min-width: 0;
    gap: 2px;
  }
  .sc-title,
  .sc-sub {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sc-title {
    font-family: var(--font-display);
    font-weight: 600;
    font-size: 14px;
    letter-spacing: -0.005em;
  }
  .sc-sub {
    font-size: 12.5px;
    color: var(--dim);
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 10px;
    padding: 28px;
    max-width: 620px;
  }
  .empty h2 {
    margin: 0;
    font-size: 22px;
  }
  .empty p {
    margin: 0;
    max-width: 54ch;
    line-height: 1.45;
  }
  .empty-icon {
    display: grid;
    place-items: center;
    width: 52px;
    height: 52px;
    margin-bottom: 2px;
    border-radius: 50%;
    background: var(--s2);
    color: var(--dim);
  }
  .empty .btn {
    margin-top: 6px;
  }
  @container (max-width: 900px) {
    .rel-grid {
      grid-template-columns: repeat(4, minmax(0, 1fr));
    }
  }
  @container (max-width: 640px) {
    .rel-grid {
      grid-template-columns: repeat(3, minmax(0, 1fr));
    }
    /* Three sleeves across a phone leave ~100 px beside each caption: 13/12
       keeps the title one line where 14/13 clipped it. */
    .rel-title {
      font-size: 13px;
    }
    .rel-sub {
      font-size: 12px;
    }
  }
  @container (max-width: 440px) {
    .rel-grid {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
    /* Two columns leave the sleeve ~150 px and the caption the whole width
       under it: the 13/12 px sizes that suit a 3-up row read as fine print
       here. */
    .rel-title {
      font-size: 14px;
    }
    .rel-sub {
      font-size: 13px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .rel-art {
      transition: none;
    }
    .rel:hover .rel-art {
      transform: none;
    }
  }
  @media (max-width: 640px) {
    .home {
      gap: 28px;
    }
    .count {
      font-size: 12px;
    }
    .actions .lbl {
      display: none;
    }
    /* §10: an action is 44 px under a finger — .btn-sm's 40 is the desktop
       rhythm, and "Tout lire" / "Autres suggestions" sit alone on the row. */
    .actions .btn-sm,
    .section-head .btn-sm {
      min-height: 44px;
    }
    /* Caption sizes follow the column count, not the window (the container
       queries above); the gap is the only window rule. */
    .rel-grid {
      gap: 12px;
    }
    /* §6's artists were a rail on a phone, and a rail hides most of them
       behind a cut edge. A 3-up grid shows nine at a glance, all on the page
       margin, and no sideways scroller competes with the tab bar's swipe.
       The avatar is 104 px inline (Avatar's `size`); on a 360 px screen the
       cells are ~100 px, so the route makes it fluid rather than letting it
       overhang the last column. */
    .artists {
      display: grid;
      grid-template-columns: repeat(3, minmax(0, 1fr));
      gap: 4px 8px;
      overflow: visible;
      padding-block: 0;
    }
    .artist {
      width: 100%;
      gap: 8px;
      padding: 4px 2px 8px;
    }
    .artist :global(.av) {
      width: 100% !important;
      height: auto !important;
      aspect-ratio: 1;
      font-size: 26px !important;
    }
    .artist-name {
      max-width: 100%;
      font-size: 13px;
    }
    .shortcuts {
      grid-template-columns: 1fr 1fr;
    }
    .shortcut {
      gap: 8px;
      padding-right: 8px;
    }
    .sc-cover {
      width: 40px;
    }
  }
  @media (max-width: 380px) {
    .shortcuts {
      grid-template-columns: 1fr;
    }
  }
</style>
