<script>
  import { onMount, tick } from "svelte";
  import { push } from "svelte-spa-router";
  import {
    chaptersByEpisode,
    loadChapters,
    chapterAt,
    nextChapter,
    chapterEnd,
    dateLabel,
  } from "../lib/chapters.js";
  import { duration as fmtDuration } from "../lib/format.js";
  import { player, seekTo } from "../lib/stores.js";
  import ChapterList from "./ChapterList.svelte";
  import Icon from "./Icon.svelte";

  export let episode = null;
  export let position = 0;
  export let isCurrent = false;
  export let onSeek = null;

  let root = null;
  let descEl = null;
  let descOpen = false;
  let descCan = false;
  let lastEp = "";

  $: podcast = !!episode?.podcast;
  $: epId = episode?.id == null ? "" : String(episode.id);
  $: entry = epId ? $chaptersByEpisode[epId] || null : null;
  $: if (podcast && epId) loadChapters(epId);

  $: chapters = entry?.chapters || [];
  $: promised = (episode?.chapter_count || 0) > 0;
  $: unknownCount = episode?.chapter_count == null;
  $: showChapters =
    !entry ||
    entry.state !== "ready" ||
    chapters.length > 0 ||
    promised ||
    entry.archived === false;
  $: loading = !entry || entry.state === "loading" || entry.state === "idle";

  $: current = isCurrent ? chapterAt(chapters, position) : null;
  $: upcoming = isCurrent && !current ? nextChapter(chapters, position) : null;
  $: card = current
    ? { index: current.index, chapter: current.chapter, ahead: false }
    : upcoming
      ? { index: upcoming.index, chapter: upcoming.chapter, ahead: true }
      : null;
  $: currentIndex = current ? current.index : -1;
  $: prevIndex = card ? card.index - 1 : -1;
  $: nextIndex = card ? (card.ahead ? card.index : card.index + 1) : -1;

  $: duration = entry?.duration || episode?.duration || 0;
  $: chapterLength = card && !card.ahead ? chapterEnd(chapters, card.index, duration) - card.chapter.start : 0;
  $: chapterElapsed =
    card && !card.ahead && chapterLength > 0
      ? Math.max(0, Math.min(chapterLength, Number(position) - card.chapter.start))
      : 0;
  $: countdown = card && card.ahead ? Math.max(0, card.chapter.start - (Number(position) || 0)) : 0;

  $: descText = episode?.description ? String(episode.description) : "";
  $: descId = "pdesc-" + (epId || "none");
  $: if (epId && epId !== lastEp) {
    lastEp = epId;
    descOpen = false;
    descCan = false;
    tick().then(() => {
      measureDesc();
      root
        ?.closest(".under-b, .side-body, .about-body")
        ?.scrollTo({ top: 0, behavior: "auto" });
    });
  }

  onMount(() => {
    tick().then(measureDesc);
  });

  function measureDesc() {
    if (!descEl) return;
    descCan = descEl.scrollHeight > descEl.clientHeight + 1 || descText.length > 280;
  }

  function goTo(seconds) {
    if (onSeek) {
      onSeek(seconds);
      return;
    }
    player.setProgress(seconds, duration);
    seekTo.set(seconds);
  }

  function seekChapter(index) {
    const chapter = chapters[index];
    if (chapter) goTo(chapter.start);
  }

  function openShow() {
    if (episode?.channel_id) push("/podcast/" + episode.channel_id);
  }
</script>

{#if podcast}
  <div class="podcast-panel" bind:this={root}>
    <div class="podcast-seg">
      {#if showChapters}
        <div class="podcast-block" aria-busy={loading}>
          <div class="podcast-h">
            <h2>Chapitres</h2>
            {#if entry?.state === "ready" && chapters.length}
              <span class="podcast-h-n mono" aria-hidden="true">{chapters.length}</span>
            {/if}
          </div>

          {#if loading}
            <p class="note status" role="status">Chargement des chapitres…</p>
            {#if promised || unknownCount}
              <ChapterList loading />
            {/if}
          {:else if entry?.state === "error"}
            <p class="note">
              {entry.error?.offline
                ? "Hors ligne : les chapitres n'ont pas encore été chargés."
                : "Chapitres indisponibles."}
            </p>
            <button class="retry" type="button" on:click={() => loadChapters(epId, { force: true })}>
              <Icon name="refresh" size={15} /> Réessayer
            </button>
          {:else if entry.archived === false}
            <p class="note">Les chapitres seront lus à la fin de l'archivage.</p>
          {:else if chapters.length === 0}
            <p class="note">Aucun chapitre pour cet épisode.</p>
          {:else}
            {#if entry.offline}
              <p class="note offline">
                <Icon name="cloudOff" size={14} /> Hors ligne — dernière copie
              </p>
            {/if}

            {#if card}
              <div class="card">
                <div class="card-top">
                  <span class="card-label">{card.ahead ? "Prochain chapitre" : chapters.length === 1 ? "Chapitre 1" : "Chapitre " + (card.index + 1) + " sur " + chapters.length}</span>
                  <span class="card-nav">
                    <button
                      type="button"
                      class="nav"
                      disabled={prevIndex < 0}
                      aria-label="Chapitre précédent"
                      title={prevIndex < 0 ? "Premier chapitre" : "Chapitre précédent · " + (chapters[prevIndex]?.title || "Chapitre " + (prevIndex + 1))}
                      on:click={() => seekChapter(prevIndex)}
                    >
                      <Icon name="prev" size={18} />
                    </button>
                    <button
                      type="button"
                      class="nav"
                      disabled={nextIndex < 0 || nextIndex >= chapters.length}
                      aria-label="Chapitre suivant"
                      title={nextIndex >= 0 && nextIndex < chapters.length ? "Chapitre suivant · " + (chapters[nextIndex]?.title || "Chapitre " + (nextIndex + 1)) : "Dernier chapitre"}
                      on:click={() => seekChapter(nextIndex)}
                    >
                      <Icon name="next" size={18} />
                    </button>
                  </span>
                </div>
                <h3 class="card-title" title={card.chapter.title}>{card.chapter.title || "Chapitre " + (card.index + 1)}</h3>
                {#if card.ahead}
                  <span class="card-time">dans {fmtDuration(countdown)}</span>
                {:else if chapterLength > 0}
                  <div class="card-progress">
                    <span class="card-bar" aria-hidden="true">
                      <span style={`width:${Math.min(100, (chapterElapsed / chapterLength) * 100)}%`}></span>
                    </span>
                    <span class="card-time">{fmtDuration(chapterElapsed)} / {fmtDuration(chapterLength)}</span>
                  </div>
                {/if}
              </div>
            {/if}

            <ChapterList {chapters} {currentIndex} onSeek={goTo} />
          {/if}
        </div>
      {/if}
    </div>

    {#if descText}
      <section class="podcast-block">
        <div class="podcast-h"><h2>Description</h2></div>
        <div
          id={descId}
          class="desc-text"
          class:clamp={!descOpen}
          bind:this={descEl}
        >{descText}</div>
        {#if descCan}
          <button
            type="button"
            class="more-btn"
            aria-expanded={descOpen}
            aria-controls={descId}
            on:click={() => (descOpen = !descOpen)}
          >
            {descOpen ? "Réduire" : "Lire la suite"}
            <Icon name={descOpen ? "chevronUp" : "chevronDown"} size={15} />
          </button>
        {/if}
      </section>
    {/if}

    <section class="podcast-block">
      <div class="podcast-h"><h2>Informations</h2></div>
      <dl class="infos">
        <dt>Podcast</dt>
        <dd>
          {#if episode.channel_id}
            <button
              type="button"
              class="show-link"
              title="Ouvrir la page du podcast"
              on:click={openShow}
            >
              <span>{episode.album?.title || episode.artist?.name || "Podcast"}</span>
              <Icon name="chevronRight" size={14} />
            </button>
          {:else}
            <span class="ellipsis" title={episode.album?.title || episode.artist?.name || ""}>{episode.album?.title || episode.artist?.name || "Podcast"}</span>
          {/if}
        </dd>
        {#if episode.published}
          <dt>Publié le</dt>
          <dd><span class="ellipsis" title={dateLabel(episode.published)}>{dateLabel(episode.published)}</span></dd>
        {/if}
        {#if episode.duration}
          <dt>Durée</dt>
          <dd><span class="mono">{fmtDuration(episode.duration)}</span></dd>
        {/if}
        {#if chapters.length}
          <dt>Chapitres</dt>
          <dd><span class="mono">{chapters.length}</span></dd>
        {/if}
        <dt>Archivé</dt>
        <dd>{episode.archived ? "Oui" : "Non"}</dd>
      </dl>
    </section>
  </div>
{/if}

<style>
  .podcast-panel {
    min-width: 0;
    padding: 0 0 8px;
    color: var(--tx);
  }
  .podcast-seg {
    min-width: 0;
  }
  .podcast-block {
    min-width: 0;
  }
  .podcast-block + .podcast-block {
    margin-top: 24px;
  }
  .podcast-h {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    margin: 4px 0 8px;
  }
  .podcast-h h2 {
    margin: 0;
    font-family: var(--font-display);
    font-size: 1.1rem;
    font-weight: 700;
    letter-spacing: -0.01em;
    color: var(--tx);
  }
  .podcast-h-n {
    font-size: 11px;
    color: var(--dim);
    font-variant-numeric: tabular-nums;
  }
  .note {
    margin: 4px 2px 10px;
    font-size: 0.88rem;
    line-height: 1.5;
    color: var(--dim);
  }
  .note.offline {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--ac-text);
  }
  .retry {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    min-height: 44px;
    margin: 0 0 6px 2px;
    padding: 0 14px;
    border-radius: 999px;
    border: 1px solid var(--ln);
    color: var(--ac-text);
    font-size: 0.88rem;
    font-weight: 600;
  }
  .retry:hover {
    background: var(--s2);
  }
  .retry:focus-visible {
    outline: 2px solid var(--ac-text);
    outline-offset: 2px;
  }

  .card {
    margin: 0 0 10px;
    padding: 12px;
    border-radius: 14px;
    background: var(--s2);
    min-width: 0;
  }
  .card-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    min-width: 0;
  }
  .card-label {
    min-width: 0;
    font-family: var(--font-mono);
    font-size: 10.5px;
    font-weight: 500;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .card-nav {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex: none;
  }
  .nav {
    display: grid;
    place-items: center;
    width: 44px;
    height: 44px;
    border-radius: 50%;
    color: var(--tx);
    flex: none;
  }
  .nav:hover:not(:disabled) {
    background: color-mix(in srgb, var(--tx) 8%, transparent);
  }
  .nav:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .nav:focus-visible {
    outline: 2px solid var(--ac-text);
    outline-offset: -2px;
  }
  .card-title {
    margin: 2px 0 8px;
    font-size: 1.02rem;
    font-weight: 700;
    line-height: 1.25;
    min-height: 2.5em;
    overflow-wrap: anywhere;
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    overflow: hidden;
  }
  .card-progress {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }
  .card-bar {
    flex: 1;
    min-width: 0;
    height: 3px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--dim) 30%, transparent);
    overflow: hidden;
  }
  .card-bar span {
    display: block;
    height: 100%;
    border-radius: 999px;
    background: var(--ac-mark);
  }
  .card-time {
    flex: none;
    font-family: var(--font-mono);
    font-size: 10.5px;
    color: var(--dim);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .desc-text {
    font-size: 0.9rem;
    line-height: 1.55;
    color: var(--dim);
    white-space: pre-line;
    overflow-wrap: anywhere;
  }
  .desc-text.clamp {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 6;
    overflow: hidden;
  }
  @media (min-width: 641px) {
    .desc-text.clamp {
      -webkit-line-clamp: 8;
    }
  }
  .more-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-height: 44px;
    padding: 0;
    color: var(--ac-text);
    font-size: 0.88rem;
    font-weight: 600;
  }
  .more-btn:focus-visible {
    outline: 2px solid var(--ac-text);
    outline-offset: 2px;
  }

  .infos {
    display: grid;
    grid-template-columns: 92px minmax(0, 1fr);
    gap: 10px 12px;
    margin: 0;
    align-items: center;
  }
  .infos dt {
    font-family: var(--font-mono);
    font-size: 10.5px;
    font-weight: 500;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--dim);
    line-height: 1.5;
  }
  .infos dd {
    margin: 0;
    min-width: 0;
    font-size: 0.88rem;
    color: var(--tx);
    line-height: 1.5;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .infos dd .ellipsis,
  .infos dd .mono {
    display: block;
  }
  .show-link {
    display: flex;
    align-items: center;
    gap: 4px;
    width: 100%;
    min-width: 0;
    min-height: 44px;
    padding: 0;
    color: var(--tx);
    text-align: left;
  }
  .show-link span {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .show-link :global(svg) {
    color: var(--dim);
  }
  .show-link:hover {
    color: var(--ac-text);
  }
  .show-link:focus-visible {
    outline: 2px solid var(--ac-text);
    outline-offset: 2px;
  }
</style>
