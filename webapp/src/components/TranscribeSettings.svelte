<script>
  import { onMount } from "svelte";
  import { api } from "../lib/api.js";
  import { bytes as fmtBytes } from "../lib/format.js";
  import {
    LANGUAGES,
    languageLabel,
    loadTranscribeSettings,
    reasonText,
    transcribeSettings,
  } from "../lib/transcript.js";
  import Icon from "./Icon.svelte";

  const MAX_MINUTES_MAX = 1440;
  const SHORTCUTS = [30, 60, 120, 180, 240, 360];

  let busy = null;
  let probing = false;
  let localError = "";
  let fieldError = "";

  $: data = $transcribeSettings.data;
  $: loading = $transcribeSettings.state === "loading" && !data;
  $: unavailable = $transcribeSettings.state === "error" && !$transcribeSettings.supported;
  $: failed = $transcribeSettings.state === "error" && $transcribeSettings.supported && !data;
  $: reason = data && data.available === false ? reasonText(data.reason) : "";

  onMount(() => {
    loadTranscribeSettings();
  });

  function publish(next) {
    transcribeSettings.set({
      state: "ready",
      data: next,
      error: null,
      supported: true,
    });
  }

  async function save(key, value) {
    if (!data || busy) return;
    const previous = data[key];
    publish({ ...data, [key]: value });
    busy = key;
    localError = "";
    fieldError = "";
    try {
      const fresh = await api.saveTranscribeSettings({ [key]: value });
      publish(fresh);
    } catch (error) {
      publish({ ...data, [key]: previous });
      const code = error && error.body && error.body.code;
      const why = error && error.body && error.body.reason;
      if (key === "enabled" && (code === "unavailable" || code === "disabled") && why) {
        const cause = reasonText(why);
        localError = `Impossible d'activer : ${cause.charAt(0).toLowerCase()}${cause.slice(1)}`;
      } else {
        localError = (error && error.body && error.body.error) || (error && error.message) || "Réglage non enregistré.";
      }
    } finally {
      busy = null;
    }
  }

  function toggleEnabled() {
    if (!data || busy) return;
    save("enabled", !data.enabled);
  }

  function numberField(key, raw, low, high) {
    const value = Number(raw);
    if (!Number.isFinite(value) || !Number.isInteger(value) || value < low || value > high) {
      fieldError = `Valeur attendue entre ${low} et ${high}.`;
      return;
    }
    fieldError = "";
    if (data && data[key] === value) return;
    save(key, value);
  }

  async function probe() {
    if (probing) return;
    probing = true;
    localError = "";
    try {
      const fresh = await loadTranscribeSettings(true);
      if (!fresh) localError = "Le moteur n'a pas répondu.";
    } finally {
      probing = false;
    }
  }

  function engineText(engine) {
    if (!engine) return "Moteur inconnu";
    const name = engine.name || "whisper.cpp";
    const version = engine.version ? ` ${engine.version}` : "";
    return `Moteur : ${name}${version} — ${engine.ok ? "trouvé" : "absent"}`;
  }

  function modelText(model) {
    if (!model) return "Modèle : inconnu";
    const name = model.name || "aucun modèle configuré";
    const size = model.size_mb ? ` · ${fmtBytes(model.size_mb * 1024 * 1024)}` : "";
    const state =
      !model.name ? " — introuvable" : model.ok ? " — valide" : " — invalide";
    return `Modèle : ${name}${size}${state}`;
  }

  function queueText(queue) {
    if (!queue) return "";
    const running = Number(queue.running) || 0;
    const queued = Number(queue.queued) || 0;
    const leased = Number(queue.leased) || 0;
    const active = running + leased;
    if (!active && !queued) return "";
    return `File : ${active} en cours, ${queued} en attente`;
  }
</script>

<section class="card">
  <h2>Transcription des podcasts</h2>
  <p class="sub">
    Le texte est produit sur ce serveur, à partir des fichiers archivés. Aucun audio n'est envoyé
    à un tiers. Rien n'est transcrit automatiquement : chaque épisode se demande depuis sa page ou
    depuis le lecteur.
  </p>

  {#if loading}
    <p class="state" role="status" aria-busy="true">Chargement des réglages…</p>
  {:else if unavailable}
    <p class="state">La transcription n'est pas disponible sur ce serveur.</p>
  {:else if failed}
    <p class="state" role="alert">Les réglages de transcription n'ont pas pu être lus.</p>
    <button type="button" class="ghost" on:click={() => loadTranscribeSettings(true)}>
      <Icon name="refresh" size={16} /> Réessayer
    </button>
  {:else if data}
    {#if localError}
      <p class="alert" role="alert">{localError}</p>
    {/if}
    {#if fieldError}
      <p class="alert" role="alert">{fieldError}</p>
    {/if}

    <button
      type="button"
      class="switch-row"
      role="switch"
      aria-checked={!!data.enabled}
      disabled={busy === "enabled"}
      on:click={toggleEnabled}
    >
      <span class="switch-text">
        <span class="switch-title">Transcrire les podcasts</span>
        <span class="switch-hint">Le moteur reste désactivé tant que cette option est fermée.</span>
      </span>
      <span class="sw" class:on={data.enabled}><span class="knob"></span></span>
    </button>

    {#if reason}
      <p class="state warn" role="status">{reason}</p>
    {/if}

    <div class="ts-fields">
      <label class="ts-field">
        <span class="ts-field-label">Langue des prochaines transcriptions</span>
        <select
          value={data.language}
          disabled={busy === "language"}
          on:change={(event) => save("language", event.currentTarget.value)}
        >
          {#each LANGUAGES as code (code)}
            <option value={code}>{languageLabel(code)}</option>
          {/each}
        </select>
      </label>

      <label class="ts-field">
        <span class="ts-field-label">Durée maximale d'un épisode</span>
        <span class="ts-inline">
          <input
            type="number"
            min="5"
            max={MAX_MINUTES_MAX}
            step="1"
            value={data.max_minutes}
            disabled={busy === "max_minutes"}
            on:change={(event) => numberField("max_minutes", event.currentTarget.value, 5, MAX_MINUTES_MAX)}
          />
          <span class="unit">min</span>
        </span>
      </label>
    </div>

    <div class="shortcuts" aria-label="Durées courantes">
      {#each SHORTCUTS as minutes (minutes)}
        <button
          type="button"
          class="chip"
          class:on={data.max_minutes === minutes}
          aria-pressed={data.max_minutes === minutes}
          disabled={busy === "max_minutes"}
          on:click={() => save("max_minutes", minutes)}
        >
          {minutes} min
        </button>
      {/each}
    </div>

    <div class="ts-fields">
      <label class="ts-field">
        <span class="ts-field-label">Budget automatique par jour</span>
        <span class="ts-inline">
          <input
            type="number"
            min="0"
            max="1440"
            step="1"
            value={data.daily_minutes}
            disabled={busy === "daily_minutes"}
            on:change={(event) => numberField("daily_minutes", event.currentTarget.value, 0, 1440)}
          />
          <span class="unit">min</span>
        </span>
        <span class="ts-field-hint">0 = illimité. La transcription s'arrête d'elle-même au budget.</span>
      </label>

      <label class="ts-field">
        <span class="ts-field-label">Recherche dans les transcriptions</span>
        <span class="ts-inline">
          <input
            type="number"
            min="0"
            max="10000000"
            step="1000"
            value={data.search_segments}
            disabled={busy === "search_segments"}
            on:change={(event) => numberField("search_segments", event.currentTarget.value, 0, 10000000)}
          />
          <span class="unit">segments</span>
        </span>
        <span class="ts-field-hint">
          {data.search_segments ? `${data.search_segments.toLocaleString("fr-FR")} segments parcourus au plus.` : "Toute la bibliothèque transcrite."}
        </span>
      </label>
    </div>

    <dl class="facts">
      <dt>Moteur</dt>
      <dd>{engineText(data.engine)}</dd>
      <dt>Modèle</dt>
      <dd>{modelText(data.model)}</dd>
      {#if data.threads}
        <dt>Calcul</dt>
        <dd>{data.threads} fils de calcul sur ce serveur</dd>
      {/if}
      {#if queueText(data.queue)}
        <dt>File</dt>
        <dd>{queueText(data.queue)}</dd>
      {/if}
    </dl>

    <div class="foot">
      <button type="button" class="ghost" on:click={probe} disabled={probing}>
        <Icon name="refresh" size={16} /> {probing ? "Vérification…" : "Tester le moteur"}
      </button>
      {#if data.available !== false}
        <span class="state ok"><Icon name="check" size={14} /> Prêt à transcrire</span>
      {/if}
    </div>
  {/if}
</section>

<style>
  .card {
    background: var(--s1);
    border: 1px solid var(--ln);
    border-radius: var(--r-card);
    padding: 22px;
    margin-bottom: 16px;
  }
  .card h2 {
    margin: 0 0 8px;
    font-family: var(--font-body);
    font-size: 16px;
    font-weight: 600;
    line-height: 1.3;
  }
  .sub {
    margin: 4px 0 16px;
    color: var(--dim);
    font-size: 0.85rem;
    line-height: 1.5;
    max-width: 70ch;
  }
  .switch-row {
    display: flex;
    align-items: center;
    gap: 14px;
    width: 100%;
    min-height: 56px;
    padding: 10px 12px;
    margin: 0 0 8px;
    border-radius: 12px;
    background: var(--s2);
    text-align: left;
    color: var(--tx);
  }
  .switch-row:hover:not(:disabled) {
    background: var(--bg-hover, var(--s2));
  }
  .switch-row:disabled {
    opacity: 0.65;
    cursor: default;
  }
  .switch-row:focus-visible {
    outline: 2px solid var(--ac-text);
    outline-offset: 2px;
  }
  .switch-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
  }
  .switch-title {
    font-weight: 650;
    font-size: 0.92rem;
  }
  .switch-hint {
    color: var(--dim);
    font-size: 0.78rem;
    line-height: 1.4;
  }
  .sw {
    flex: none;
    width: 44px;
    height: 26px;
    border-radius: 999px;
    background: var(--s1);
    border: 1px solid var(--dim);
    position: relative;
    transition: background 0.15s ease, border-color 0.15s ease;
  }
  .sw.on {
    background: var(--btn);
    border-color: var(--btn);
  }
  .knob {
    position: absolute;
    top: 3px;
    left: 3px;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: var(--tx);
    transition: transform 0.15s ease, background 0.15s ease;
  }
  .sw.on .knob {
    background: var(--onbtn);
    transform: translateX(18px);
  }
  .ts-fields {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 14px 20px;
    margin: 16px 0 6px;
  }
  .ts-field {
    display: flex;
    flex-direction: column;
    gap: 7px;
    min-width: 0;
  }
  .ts-field-label {
    color: var(--dim);
    font-size: 0.78rem;
    font-weight: 600;
    line-height: 1.4;
  }
  .ts-field select,
  .ts-inline input {
    min-height: 44px;
    padding: 0 12px;
    border-radius: 10px;
    border: 1px solid var(--ln);
    background: var(--bg);
    color: var(--tx);
    font-size: 0.9rem;
    font-weight: 600;
  }
  .ts-field select {
    max-width: 100%;
  }
  .ts-inline {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .ts-inline input {
    width: 110px;
    font-variant-numeric: tabular-nums;
  }
  .unit {
    color: var(--dim);
    font-size: 0.82rem;
  }
  .ts-field-hint {
    color: var(--dim);
    font-size: 0.74rem;
    line-height: 1.4;
  }
  .shortcuts {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin: 8px 0 0;
  }
  .chip {
    height: auto;
    min-height: 36px;
    padding: 0 11px;
    border-radius: 999px;
    border: 1px solid var(--ln);
    color: var(--dim);
    font-size: 0.78rem;
    font-weight: 600;
  }
  .chip.on {
    background: var(--ac);
    border-color: var(--ac);
    color: var(--onac);
  }
  .chip:focus-visible,
  .ghost:focus-visible {
    outline: 2px solid var(--ac-text);
    outline-offset: 2px;
  }
  @media (pointer: coarse) {
    .chip {
      min-height: 44px;
    }
  }
  .facts {
    display: grid;
    grid-template-columns: 92px minmax(0, 1fr);
    gap: 6px 12px;
    margin: 18px 0 0;
    padding: 12px;
    border-radius: 12px;
    background: var(--s2);
  }
  .facts dt {
    color: var(--dim);
    font-family: var(--font-mono);
    font-size: 10.5px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    line-height: 1.5;
  }
  .facts dd {
    margin: 0;
    min-width: 0;
    font-size: 0.82rem;
    line-height: 1.5;
    overflow-wrap: anywhere;
  }
  .foot {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
    margin-top: 16px;
  }
  .ghost {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    min-height: 44px;
    padding: 0 14px;
    border-radius: 999px;
    border: 1px solid var(--ln);
    color: var(--ac-text);
    font-size: 0.88rem;
    font-weight: 600;
  }
  .ghost:hover:not(:disabled) {
    background: var(--s2);
  }
  .ghost:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .state {
    margin: 0;
    color: var(--dim);
    font-size: 0.82rem;
    line-height: 1.45;
  }
  .state.ok {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    color: var(--ac-text);
  }
  .state.warn {
    color: var(--accent-2, #e05252);
  }
  .alert {
    margin: 0 0 12px;
    padding: 9px 11px;
    border-radius: 9px;
    background: color-mix(in srgb, var(--accent-2, #e05252) 13%, transparent);
    color: var(--tx);
    font-size: 0.82rem;
    line-height: 1.45;
  }
  @media (prefers-reduced-motion: reduce) {
    .sw,
    .knob {
      transition: none;
    }
  }
</style>
