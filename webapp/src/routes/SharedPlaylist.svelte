<script>
  // The public, read-only view of a shared playlist link: whoever holds the
  // token — no account, no session — sees the playlist and can play the tracks
  // the owner has archived. There is no app shell, no global player, no edit
  // action: the page owns its <audio> element and its queue, and streams from
  // the capability URLs under /api/shared/playlist/<token>/.
  import { onMount } from "svelte";
  import { api } from "../lib/api.js";
  import { duration as fmtDuration } from "../lib/format.js";
  import Icon from "../components/Icon.svelte";
  import Cover from "../components/Cover.svelte";

  export let token = "";

  let phase = "loading"; // loading | ready | missing | failed
  let data = null; // { playlist, tracks }
  let queue = []; // the streamable tracks, in list order
  // A public link may hold thousands of titles: paint a first slice and let the
  // person ask for more, so the DOM and the cover queue never mount 4 000 rows
  // on a phone. The queue still uses ALL streamable titles.
  const SLICE = 100;
  let shown = SLICE;
  $: visibleTracks = data ? data.tracks.slice(0, shown) : [];
  $: moreTracks = !!(data && data.tracks.length > shown);
  let seq = []; // playback order: indices into `queue`
  let pos = -1; // position in `seq`
  let playing = false;
  let shuffleOn = false;
  let posTime = 0;
  let durSec = 0;
  let seekVal = 0;
  let dragging = false;
  let volume = 1;
  let failedTrack = "";
  let audio;

  $: queueMap = new Map(queue.map((t, i) => [t, i]));
  $: if (data) document.title = `${data.playlist.title} · Neezer`;
  $: if (!dragging && durSec) seekVal = Math.min(1000, Math.round((1000 * posTime) / durSec));
  $: current = pos >= 0 && seq[pos] !== undefined ? queue[seq[pos]] : null;
  $: currentTitle = current ? current.title : "";
  $: currentArtist = current ? current.display_artist || current.artist?.name || "" : "";
  $: nbLabel = data
    ? `${data.playlist.nb_tracks} titre${data.playlist.nb_tracks > 1 ? "s" : ""}`
    : "";

  onMount(async () => {
    try {
      data = await api.sharedPlaylist(token);
      queue = data.tracks.filter((t) => t.streamable);
      shown = SLICE;
      phase = "ready";
    } catch (e) {
      phase = e?.status === 404 ? "missing" : "failed";
    }
  });

  function streamUrl(mid) {
    return `/api/shared/playlist/${encodeURIComponent(token)}/stream/${encodeURIComponent(mid)}`;
  }

  function playSeq(p) {
    if (!queue.length || p < 0 || p >= seq.length) return;
    pos = p;
    playing = true;
    failedTrack = "";
    audio.src = streamUrl(queue[seq[p]].mid);
    audio.play().catch(() => {});
  }

  function playAll() {
    shuffleOn = false;
    seq = queue.map((_, i) => i);
    playSeq(0);
  }

  function shufflePlay() {
    if (!queue.length) return;
    shuffleOn = true;
    seq = queue.map((_, i) => i).sort(() => Math.random() - 0.5);
    playSeq(0);
  }

  function toggleRow(t) {
    if (!t.streamable) return;
    const qi = queueMap.get(t);
    if (qi === undefined) return;
    if (pos >= 0 && seq[pos] === qi) {
      if (playing) {
        audio.pause();
        return;
      }
      audio.play().catch(() => {});
      return;
    }
    let p = seq.indexOf(qi);
    if (p === -1) {
      // Started from a plain list order, then shuffled: keep the shuffle.
      shuffleOn = true;
      seq = queue.map((_, i) => i).sort(() => Math.random() - 0.5);
      p = seq.indexOf(qi);
    }
    playSeq(p);
  }

  function togglePlay() {
    if (!current) return;
    if (playing) audio.pause();
    else audio.play().catch(() => {});
  }

  function next() {
    if (pos >= 0 && pos < seq.length - 1) {
      playSeq(pos + 1);
      return;
    }
    audio.pause();
    audio.removeAttribute("src");
    playing = false;
    pos = -1;
  }

  function prev() {
    if (!current) return;
    if (audio.currentTime > 3) {
      audio.currentTime = 0;
      return;
    }
    if (pos > 0) playSeq(pos - 1);
  }

  function tick() {
    posTime = audio.currentTime || 0;
    durSec = audio.duration || 0;
  }

  function onSeekInput(e) {
    dragging = true;
    seekVal = Number(e.target.value);
  }

  function onSeekChange(e) {
    dragging = false;
    const d = audio.duration;
    if (d) audio.currentTime = (Number(e.target.value) / 1000) * d;
  }

  function onVolume(e) {
    volume = Number(e.target.value);
    audio.volume = volume;
  }

  function onAudioError() {
    playing = false;
    failedTrack = currentTitle;
  }

  function retry() {
    phase = "loading";
    api.sharedPlaylist(token)
      .then((d) => {
        data = d;
        queue = d.tracks.filter((t) => t.streamable);
        shown = SLICE;
        phase = "ready";
      })
      .catch((e) => (phase = e?.status === 404 ? "missing" : "failed"));
  }

  function isCurrent(t) {
    return current === t;
  }
</script>

<div class="page">
  <header class="top">
    <span class="brand"><Icon name="disc" size={18} /> Lecture publique · Neezer</span>
  </header>

  {#if phase === "loading"}
    <div class="center">
      <span class="spinner" aria-label="Chargement"></span>
    </div>
  {:else if phase === "missing"}
    <div class="center msg">
      <span class="mark"><Icon name="disc" size={34} /></span>
      <h1>Ce lien n'est plus valide.</h1>
      <p class="muted">
        Il a été révoqué ou a expiré. Demandez à son propriétaire d'en créer un nouveau.
      </p>
    </div>
  {:else if phase === "failed"}
    <div class="center msg">
      <span class="mark"><Icon name="cloudOff" size={34} /></span>
      <h1>Chargement impossible.</h1>
      <p class="muted">Le serveur ne répond pas pour l'instant.</p>
      <button class="btn btn-primary" on:click={retry}>Réessayer</button>
    </div>
  {:else}
    <main class="body">
      <section class="hero">
        <div class="cover">
          <Cover src={data.playlist.cover} kind="playlist" eager />
        </div>
        <div class="meta">
          <p class="kicker label-mono">Playlist publique</p>
          <h1>{data.playlist.title}</h1>
          {#if data.playlist.description}
            <p class="desc">{data.playlist.description}</p>
          {/if}
          <p class="by muted">{nbLabel} · par {data.playlist.owner}</p>
          <div class="actions">
            <button class="btn btn-primary" on:click={playAll} disabled={!queue.length}>
              <Icon name="play" size={18} />
              Lire
            </button>
            <button class="btn btn-ghost" on:click={shufflePlay} disabled={!queue.length}>
              <Icon name="shuffle" size={18} />
              Mélanger
            </button>
          </div>
          {#if !queue.length && data.tracks.length}
            <p class="note muted">
              Aucun titre archivé pour l'instant&nbsp;: le propriétaire doit d'abord les
              archiver pour qu'ils soient écoutables ici.
            </p>
          {/if}
        </div>
      </section>

      {#if data.tracks.length}
        <ol class="tracks">
          {#each visibleTracks as t (t.mid)}
            <li class:current={isCurrent(t)} class:off={!t.streamable}>
              {#if t.streamable}
                <button
                  class="row"
                  on:click={() => toggleRow(t)}
                  aria-label={isCurrent(t) && playing ? `Pause : ${t.title}` : `Lire : ${t.title}`}
                >
                  <span class="tcover">
                    <Cover src={t.cover} kind="track" />
                    <span class="glyph">
                      {#if isCurrent(t) && playing}
                        <Icon name="pause" size={16} />
                      {:else}
                        <Icon name="play" size={16} />
                      {/if}
                    </span>
                  </span>
                  <span class="names">
                    <span class="tt">{t.title}</span>
                    <span class="aa">{t.display_artist || t.artist?.name || ""}</span>
                  </span>
                  <span class="dur mono">{fmtDuration(t.duration)}</span>
                </button>
              {:else}
                <div class="row off">
                  <span class="tcover">
                    <Cover src={t.cover} kind="track" />
                  </span>
                  <span class="names">
                    <span class="tt">{t.title}</span>
                    <span class="aa">{t.display_artist || t.artist?.name || ""}</span>
                    <span class="why">Non archivé — le propriétaire doit l'archiver pour qu'il soit écoutable</span>
                  </span>
                  <span class="chip">Non archivé</span>
                  <span class="dur mono">{fmtDuration(t.duration)}</span>
                </div>
              {/if}
            </li>
          {/each}
        </ol>
        {#if moreTracks}
          <div class="more">
            <button class="btn btn-ghost" on:click={() => (shown += SLICE)}>
              Afficher les {Math.min(SLICE, data.tracks.length - shown)} titres suivants
            </button>
            <span class="muted count">{shown} sur {data.tracks.length}</span>
          </div>
        {/if}
      {:else}
        <p class="empty muted">Cette playlist ne contient aucun titre.</p>
      {/if}

      <footer class="bar" class:on={current}>
        <div class="now">
          <span class="nt">{currentTitle}</span>
          <span class="na muted">{failedTrack ? `Lecture impossible — réessayez` : currentArtist}</span>
        </div>
        <div class="ctrl">
          <button class="btn-icon" on:click={prev} aria-label="Titre précédent">
            <Icon name="prev" size={20} />
          </button>
          <button class="btn-icon play" on:click={togglePlay} aria-label={playing ? "Pause" : "Lecture"}>
            {#if playing}
              <Icon name="pause" size={22} />
            {:else}
              <Icon name="play" size={22} />
            {/if}
          </button>
          <button class="btn-icon" on:click={next} aria-label="Titre suivant">
            <Icon name="next" size={20} />
          </button>
        </div>
        <div class="prog">
          <span class="mono t">{fmtDuration(posTime)}</span>
          <input
            type="range"
            min="0"
            max="1000"
            step="1"
            value={seekVal}
            on:input={onSeekInput}
            on:change={onSeekChange}
            aria-label="Position"
          />
          <span class="mono t">{fmtDuration(durSec)}</span>
        </div>
        <div class="vol">
          <Icon name="volume" size={18} />
          <input
            type="range"
            min="0"
            max="1"
            step="0.01"
            value={volume}
            on:input={onVolume}
            aria-label="Volume"
          />
        </div>
      </footer>
    </main>
  {/if}

  <!-- svelte-ignore a11y-media-has-caption -->
  <audio
    bind:this={audio}
    preload="none"
    on:ended={next}
    on:timeupdate={tick}
    on:play={() => (playing = true)}
    on:pause={() => (playing = false)}
    on:error={onAudioError}
  ></audio>
</div>

<style>
  .page {
    position: fixed;
    inset: 0;
    overflow-y: auto;
    background: var(--bg);
    color: var(--tx);
    font-family: var(--font-body);
  }
  .top {
    position: sticky;
    top: 0;
    z-index: 5;
    display: flex;
    align-items: center;
    padding: 14px var(--page-x, 24px);
    background: color-mix(in srgb, var(--bg) 86%, transparent);
    backdrop-filter: blur(14px);
    border-bottom: 1px solid var(--ln);
  }
  .brand {
    display: inline-flex;
    align-items: center;
    gap: 9px;
    font-weight: 600;
    font-size: 14px;
  }
  .brand :global(svg) {
    color: var(--ac-mark);
  }
  .center {
    min-height: calc(100vh - 60px);
    display: grid;
    place-items: center;
    align-content: center;
    gap: 14px;
    padding: 24px;
    text-align: center;
  }
  .spinner {
    width: 34px;
    height: 34px;
    border-radius: 50%;
    border: 3px solid var(--s2);
    border-top-color: var(--ac-mark);
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .msg {
    max-width: 420px;
    margin: 0 auto;
  }
  .msg h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: 1.6rem;
    letter-spacing: -0.03em;
  }
  .msg p {
    margin: 6px 0 0;
    line-height: 1.5;
  }
  .mark {
    display: grid;
    place-items: center;
    width: 64px;
    height: 64px;
    border-radius: 50%;
    background: var(--s2);
    color: var(--ac-mark);
  }
  .body {
    max-width: 760px;
    margin: 0 auto;
    padding: 28px var(--page-x, 24px) 140px;
  }
  .hero {
    display: flex;
    gap: 24px;
    align-items: center;
  }
  .cover {
    flex: none;
    width: 200px;
    height: 200px;
    border-radius: 14px;
    overflow: hidden;
    box-shadow: 0 16px 40px rgba(0, 0, 0, 0.35);
  }
  .meta {
    min-width: 0;
  }
  .kicker {
    margin: 0 0 6px;
    color: var(--ac-text);
    font-size: 12px;
  }
  .meta h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: clamp(1.7rem, 4vw, 2.6rem);
    line-height: 1.05;
    letter-spacing: -0.03em;
    overflow-wrap: anywhere;
  }
  .desc {
    margin: 10px 0 0;
    font-size: 0.95rem;
    line-height: 1.5;
    color: var(--dim);
    display: -webkit-box;
    -webkit-line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .by {
    margin: 10px 0 0;
    font-size: 0.9rem;
  }
  .actions {
    display: flex;
    gap: 10px;
    margin-top: 18px;
  }
  .note {
    margin: 12px 0 0;
    font-size: 0.82rem;
    line-height: 1.45;
  }
  .tracks {
    list-style: none;
    margin: 32px 0 0;
    padding: 0;
  }
  .tracks li {
    border-radius: 12px;
  }
  .tracks li.current {
    background: var(--s1);
  }
  .more {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin: 18px 0 0;
    flex-wrap: wrap;
  }
  .more .count {
    font-size: 0.78rem;
    font-variant-numeric: tabular-nums;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 14px;
    width: 100%;
    min-height: 64px;
    padding: 9px 14px;
    border: 0;
    border-radius: 12px;
    background: none;
    color: var(--tx);
    text-align: left;
    font-family: var(--font-body);
    cursor: pointer;
  }
  .row:hover:not(.off) {
    background: var(--s2);
  }
  .tcover {
    position: relative;
    flex: none;
    width: 46px;
    height: 46px;
    border-radius: 8px;
    overflow: hidden;
  }
  .tcover :global(> *) {
    width: 100%;
    height: 100%;
  }
  .glyph {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.45);
    color: #fff;
    opacity: 0;
    transition: opacity 0.12s ease;
  }
  .row:hover .glyph,
  .current .glyph {
    opacity: 1;
  }
  .names {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .tt {
    font-weight: 600;
    font-size: 0.95rem;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .current .tt {
    color: var(--ac-text);
  }
  .aa {
    font-size: 0.82rem;
    color: var(--dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .why {
    font-size: 0.76rem;
    color: var(--dim);
    line-height: 1.35;
  }
  .chip {
    flex: none;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.05em;
    color: var(--dim);
    border: 1px solid var(--ln);
    border-radius: 999px;
    padding: 5px 10px;
  }
  .dur {
    flex: none;
    font-size: 0.8rem;
    color: var(--dim);
    font-variant-numeric: tabular-nums;
  }
  .empty {
    margin: 32px 0 0;
    text-align: center;
  }
  .bar {
    position: fixed;
    left: 0;
    right: 0;
    bottom: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto minmax(0, 1.4fr) auto;
    align-items: center;
    gap: 18px;
    padding: 12px max(var(--page-x, 24px), 12px) calc(12px + env(safe-area-inset-bottom, 0px));
    background: var(--s1);
    border-top: 1px solid var(--ln);
    transform: translateY(102%);
    transition: transform 0.22s ease;
  }
  .bar.on {
    transform: none;
  }
  .now {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .nt {
    font-weight: 600;
    font-size: 0.9rem;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .na {
    font-size: 0.78rem;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ctrl {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .ctrl .btn-icon.play {
    width: 52px;
    height: 52px;
    background: var(--btn);
    border-color: transparent;
    color: var(--onbtn);
  }
  .prog {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }
  .prog .t {
    font-size: 0.76rem;
    color: var(--dim);
    font-variant-numeric: tabular-nums;
  }
  .prog input {
    flex: 1;
    min-width: 0;
  }
  .vol {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--dim);
  }
  .vol input {
    width: 90px;
  }
  input[type="range"] {
    appearance: none;
    height: 28px;
    background: transparent;
    cursor: pointer;
  }
  input[type="range"]::-webkit-slider-runnable-track {
    height: 4px;
    border-radius: 2px;
    background: var(--s2);
  }
  input[type="range"]::-webkit-slider-thumb {
    appearance: none;
    width: 14px;
    height: 14px;
    margin-top: -5px;
    border-radius: 50%;
    background: var(--ac-text);
    border: 0;
  }
  input[type="range"]::-moz-range-track {
    height: 4px;
    border-radius: 2px;
    background: var(--s2);
  }
  input[type="range"]::-moz-range-thumb {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--ac-text);
    border: 0;
  }
  @media (pointer: coarse) {
    .row {
      min-height: 72px;
    }
    .btn-icon {
      width: 44px;
      height: 44px;
    }
    .ctrl .btn-icon.play {
      width: 56px;
      height: 56px;
    }
    .prog input,
    .vol input {
      height: 44px;
    }
  }
  @media (max-width: 640px) {
    .hero {
      flex-direction: column;
      align-items: stretch;
      gap: 18px;
    }
    .cover {
      width: 148px;
      height: 148px;
    }
    .actions {
      width: 100%;
    }
    .actions .btn {
      flex: 1;
      justify-content: center;
    }
    .tracks {
      margin-top: 24px;
    }
    .chip {
      display: none; /* the row's own line says it, and there is no room */
    }
    .bar {
      grid-template-columns: 1fr auto;
      grid-template-rows: auto auto;
      gap: 4px 14px;
    }
    .now {
      display: none;
    }
    .ctrl {
      justify-self: start;
      grid-column: 1;
      grid-row: 2;
    }
    .prog {
      grid-column: 1 / -1;
      grid-row: 1;
    }
    .vol {
      grid-column: 2;
      grid-row: 2;
      justify-self: end;
    }
    .vol input {
      width: 70px;
    }
  }
</style>
