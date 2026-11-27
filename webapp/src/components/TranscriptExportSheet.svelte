<script>
  // The show's transcript export: pick a season (or the whole show) and a text
  // format, then one ZIP downloads. The season list comes from the server (it
  // derives seasons from the titles — Deezer has no season field); this sheet
  // reads it once on open and never retries in a storm.
  import { onDestroy } from "svelte";
  import { sheet, veil } from "../lib/motion.js";
  import { api } from "../lib/api.js";
  import { transcriptExportSheet, closeTranscriptExport, toasts } from "../lib/stores.js";
  import { modalFocus } from "../lib/modal.js";
  import Icon from "./Icon.svelte";
  import LoadFailed from "./LoadFailed.svelte";

  const FORMATS = [
    { id: "txt", label: "Texte", ext: "txt" },
    { id: "srt", label: "Sous-titres (SubRip)", ext: "srt" },
    { id: "vtt", label: "WebVTT", ext: "vtt" },
    { id: "md", label: "Markdown", ext: "md" },
  ];

  let target = null;
  let loadedFor = null;
  let loading = false;
  let error = null;
  let all = { episodes: 0, transcribed: 0 };
  let groups = [];
  let season = "all";
  let fmt = "txt";
  let started = false;
  let timer = null;

  $: target = $transcriptExportSheet;
  $: if (target && loadedFor !== target) open(target);
  $: if (!target) {
    loadedFor = null;
    started = false;
  }

  async function open(entry) {
    loadedFor = entry;
    started = false;
    loading = true;
    error = null;
    groups = [];
    all = { episodes: 0, transcribed: 0 };
    season = "all";
    fmt = "txt";
    await load();
  }

  async function load() {
    if (!target) return;
    loading = true;
    error = null;
    try {
      const body = await api.transcriptSeasons(target.id);
      groups = body?.seasons || [];
      all = body?.all || { episodes: 0, transcribed: 0 };
      if (season !== "all" && !groups.some((g) => g.key === season && g.transcribed > 0)) {
        season = all.transcribed > 0 ? "all" : groups.find((g) => g.transcribed > 0)?.key || "all";
      }
    } catch (e) {
      error = e;
    }
    loading = false;
  }

  $: canDownload = !loading && !error && readyFor(season) > 0;

  function readyFor(key) {
    if (key === "all") return all.transcribed || 0;
    return groups.find((g) => g.key === key)?.transcribed || 0;
  }

  // window.location hands the ZIP to the browser's download manager, the same
  // way the audio export does: no blob, no archive held in this tab, and it
  // works in the Android WebView.
  function start() {
    if (!canDownload || !target) return;
    started = true;
    window.location.href = api.showTranscriptsExportUrl(target.id, season, fmt);
    toasts.push("Export lancé — voir les téléchargements du navigateur");
    timer = setTimeout(closeTranscriptExport, 1200);
  }

  function onKey(event) {
    if (event.key === "Escape") closeTranscriptExport();
  }

  onDestroy(() => clearTimeout(timer));
</script>

<svelte:window on:keydown={onKey} />

{#if target}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div class="overlay" transition:veil={{ for: "sheet" }} on:click|self={closeTranscriptExport}>
    <div
      class="sheet"
      role="dialog"
      aria-modal="true"
      aria-label="Exporter les transcriptions"
      tabindex="-1"
      transition:sheet
      use:modalFocus
    >
      <header>
        <div class="ttl">
          <h2>Exporter les transcriptions</h2>
          <p class="muted">{target.title || "Cette émission"}</p>
        </div>
        <button class="close" on:click={closeTranscriptExport} aria-label="Fermer">
          <Icon name="close" size={20} />
        </button>
      </header>

      <div class="body">
        {#if loading}
          <div class="pad" aria-busy="true" role="status">
            <p class="status">Chargement des saisons…</p>
            <span class="sk" data-motion="status"></span>
            <span class="sk short" data-motion="status"></span>
          </div>
        {:else if error}
          <div class="pad">
            <LoadFailed
              error={error}
              notFound="La liste des saisons n'a pas pu être chargée."
              inline
              on:retry={load}
            />
          </div>
        {:else if !all.episodes}
          <p class="muted pad">Cette émission n'a pas encore d'épisode.</p>
        {:else}
          <p class="label">Épisodes</p>
          <div class="choices" role="radiogroup" aria-label="Épisodes à exporter">
            <button
              type="button"
              class="fmt"
              role="radio"
              aria-checked={season === "all"}
              class:sel={season === "all"}
              disabled={!all.transcribed}
              on:click={() => (season = "all")}
            >
              <span class="fname">Toute l'émission</span>
              <span class="count muted">{all.transcribed}/{all.episodes} transcrits</span>
              {#if season === "all"}<span class="tick"><Icon name="check" size={15} /></span>{/if}
            </button>
            {#each groups as group (group.key)}
              <button
                type="button"
                class="fmt"
                role="radio"
                aria-checked={season === group.key}
                class:sel={season === group.key}
                disabled={!group.transcribed}
                on:click={() => (season = group.key)}
              >
                <span class="fname">{group.label}</span>
                <span class="count muted">{group.transcribed}/{group.episodes} transcrits</span>
                {#if season === group.key}<span class="tick"><Icon name="check" size={15} /></span>{/if}
              </button>
            {/each}
          </div>

          <p class="label">Format</p>
          <div class="formats" role="radiogroup" aria-label="Format du texte">
            {#each FORMATS as entry (entry.id)}
              <button
                type="button"
                class="fmt"
                role="radio"
                aria-checked={fmt === entry.id}
                class:sel={fmt === entry.id}
                on:click={() => (fmt = entry.id)}
              >
                <span class="fname">{entry.label}</span>
                <span class="count muted">.{entry.ext}</span>
                {#if fmt === entry.id}<span class="tick"><Icon name="check" size={15} /></span>{/if}
              </button>
            {/each}
          </div>
        {/if}
      </div>

      <footer>
        <button class="ghost" on:click={closeTranscriptExport}>Annuler</button>
        <button class="pill" on:click={start} disabled={!canDownload || started}>
          <Icon name="download" size={17} />
          {started ? "Démarré…" : "Télécharger le ZIP"}
        </button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: grid;
    place-items: center;
    z-index: 300;
    padding: 20px;
  }
  .sheet {
    background: var(--bg-elev);
    border-radius: 16px;
    width: min(460px, 100%);
    max-height: 86vh;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    box-shadow: 0 30px 80px rgba(0, 0, 0, 0.55);
  }
  header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
    padding: 18px 20px 12px;
  }
  .ttl h2 {
    margin: 0;
    font-size: 1.15rem;
  }
  .ttl p {
    margin: 4px 0 0;
    font-size: 0.85rem;
  }
  .close {
    flex: none;
    color: var(--text-dim);
  }
  .close:hover {
    color: var(--text);
  }
  .body {
    overflow-y: auto;
    min-height: 0;
  }
  .pad {
    padding: 8px 20px 20px;
  }
  .label {
    margin: 10px 20px 6px;
    font-size: 0.78rem;
    font-weight: 700;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    color: var(--dim);
  }
  .choices,
  .formats {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 0 14px;
  }
  .fmt {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    min-height: 46px;
    text-align: left;
    padding: 10px 14px;
    border-radius: 10px;
    border: 1px solid transparent;
    background: var(--bg);
    color: var(--text);
  }
  .fmt:hover:not(:disabled) {
    background: var(--bg-hover);
  }
  .fmt.sel {
    border-color: var(--accent);
    background: var(--bg-hover);
  }
  .fmt:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .fname {
    font-weight: 600;
  }
  .count {
    margin-left: auto;
    font-size: 0.8rem;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .tick {
    color: var(--accent);
    display: flex;
  }
  .fmt .count + .tick {
    margin-left: 4px;
  }
  .status {
    margin: 0;
    color: var(--dim);
    font-size: 0.84rem;
  }
  .sk {
    display: block;
    width: 100%;
    height: 12px;
    margin-top: 10px;
    border-radius: 6px;
    background: color-mix(in srgb, var(--dim) 18%, transparent);
    animation: pulse 1.4s ease-in-out infinite;
  }
  .sk.short {
    width: 64%;
  }
  @keyframes pulse {
    50% {
      opacity: 0.45;
    }
  }
  footer {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 10px;
    padding: 16px 20px;
    margin-top: 6px;
    border-top: 1px solid var(--bg-hover);
  }
  .ghost {
    color: var(--text-dim);
    padding: 8px 12px;
  }
  .ghost:hover {
    color: var(--text);
  }
  .pill {
    gap: 8px;
    white-space: nowrap;
  }
  .pill:disabled {
    opacity: 0.5;
    cursor: default;
    transform: none;
  }
  @media (prefers-reduced-motion: reduce) {
    .sk {
      animation: none;
    }
  }
</style>
