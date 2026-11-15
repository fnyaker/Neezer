<script>
  import { onMount } from "svelte";
  import { api } from "../lib/api.js";
  import { user } from "../lib/stores.js";
  import Icon from "./Icon.svelte";
  import LoadFailed from "./LoadFailed.svelte";
  import {
    accountSummary,
    dropStore,
    importFailure,
    importNote,
    importProgress,
    isImportRunning,
    linkFailure,
    readStore,
    saveFailure,
    unmatchedCount,
    unmatchedItems,
    writeStore,
  } from "../lib/onboarding.js";

  export let options = null;
  export let onActivity = null;

  const POLL_MS = 1500;
  const POLL_MAX = 5 * 60 * 1000;

  let opts = options || null;
  let seen = options;
  let open = "";
  let arl = "";
  let headers = "";
  let url = "";
  let busy = "";
  let error = null;
  let notice = "";
  let skipped = [];
  let job = null;
  let lastUrl = "";
  let pollTimer = null;
  let accountTimer = null;
  let accountStart = 0;
  let pollStart = 0;
  let capped = false;
  let loadError = null;

  $: if (options !== seen) {
    seen = options;
    if (options) opts = options;
  }

  $: dz = opts ? opts.deezer || null : null;
  $: yt = opts ? opts.youtube || null : null;
  $: imports = opts && Array.isArray(opts.imports) ? opts.imports : [];
  $: playlistAvailable = imports.some((entry) => entry && entry.available);
  $: dzState = dz ? (!dz.available ? { text: "Indisponible sur ce serveur.", tone: "error" } : accountSummary(dz.account)) : null;
  $: ytState = yt
    ? !yt.available
      ? {
          text:
            yt.reason === "absent"
              ? "YouTube Music n'est pas installé sur ce serveur."
              : "YouTube Music est désactivé sur ce serveur.",
          tone: "error",
        }
      : yt.denied
        ? { text: "Votre compte n'a pas accès à ce service.", tone: "error" }
        : accountSummary(yt.account)
    : null;
  $: progress = importProgress(job);
  $: missed = unmatchedCount(job);
  $: missedList = unmatchedItems(job);
  $: note = importNote(job);
  $: runningAccount = !!((dz && dz.account && dz.account.running) || (yt && yt.account && yt.account.running));
  $: importInFlight = runningAccount || isImportRunning(job);
  $: if (runningAccount) watchAccounts();
  $: if (!runningAccount) stopAccountPoll();
  $: if (onActivity) onActivity(importInFlight);

  onMount(() => {
    const account = accountName();
    const saved = readStore("import", { maxAge: 3 * 60 * 60 * 1000 });
    const savedSkips = readStore("skipped", { maxAge: null });
    skipped = savedSkips && savedSkips.account === account && Array.isArray(savedSkips.ids) ? savedSkips.ids : [];
    if (!richOptions(opts)) loadOptions();
    if (saved && saved.account === account && saved.id) {
      job = { id: saved.id, state: "reading" };
      lastUrl = saved.url || "";
      startPolling();
    } else if (opts && opts.job && opts.job.id) {
      job = { ...opts.job };
      startPolling();
    }
    watchAccounts();
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      stopPolling();
      stopAccountPoll();
    };
  });

  function richOptions(value) {
    if (!value || !Array.isArray(value.imports)) return false;
    const deezer = value.deezer || {};
    return (
      "can_link" in deezer &&
      value.imports.every((entry) => entry && "available" in entry)
    );
  }

  async function loadOptions() {
    loadError = null;
    try {
      opts = await api.onboardingAccountOptions();
    } catch (e) {
      loadError = e;
    }
  }

  function accountName() {
    return ($user && $user.name) || "";
  }

  function memo(key, value) {
    if (value) writeStore(key, value);
    else dropStore(key);
  }

  function persistSkips() {
    memo("skipped", { account: accountName(), ids: skipped });
  }

  function skip(id) {
    if (skipped.includes(id)) return;
    skipped = [...skipped, id];
    persistSkips();
  }

  function unskip(id) {
    skipped = skipped.filter((value) => value !== id);
    persistSkips();
  }

  async function link(provider) {
    if (busy) return;
    const payload = { provider };
    if (provider === "deezer") {
      const secret = arl.trim();
      if (!secret) {
        error = { provider, message: "Collez votre ARL Deezer." };
        return;
      }
      payload.arl = secret;
    } else {
      const raw = headers.trim();
      if (!raw) {
        error = { provider, message: "Collez les en-têtes d'une session YouTube Music." };
        return;
      }
      payload.headers = raw;
    }
    busy = provider;
    error = null;
    notice = "";
    try {
      const result = await api.onboardingAccountLink(payload);
      if (result && result.options) opts = result.options;
      arl = "";
      headers = "";
      open = "";
      notice =
        provider === "deezer"
          ? "Compte Deezer relié. L'import de vos favoris a démarré sur le serveur."
          : "Compte YouTube Music relié. L'import de vos favoris a démarré sur le serveur.";
      watchAccounts(true);
    } catch (e) {
      error = { provider, message: linkFailure(e) };
    } finally {
      busy = "";
    }
  }

  async function startImport() {
    if (busy) return;
    const linkUrl = url.trim();
    if (!linkUrl) {
      error = { provider: "playlist", message: "Collez le lien d'une playlist." };
      return;
    }
    busy = "playlist";
    error = null;
    notice = "";
    try {
      const result = await api.onboardingAccountImport({ url: linkUrl });
      if (result && result.options) opts = result.options;
      job = result && result.job ? result.job : { id: result.id, state: "reading" };
      lastUrl = linkUrl;
      memo("import", { account: accountName(), id: job.id, url: linkUrl });
      url = "";
      open = "";
      startPolling();
    } catch (e) {
      error = { provider: "playlist", message: importFailure(e) };
    } finally {
      busy = "";
    }
  }

  async function save() {
    if (busy || !job) return;
    busy = "save";
    error = null;
    try {
      await api.importSave(job.id, job.name || "", null);
      startPolling(0);
    } catch (e) {
      error = { provider: "playlist", message: saveFailure(e) };
    } finally {
      busy = "";
    }
  }

  function retryImport() {
    if (!lastUrl) {
      job = null;
      memo("import", null);
      open = "playlist";
      return;
    }
    job = null;
    memo("import", null);
    url = lastUrl;
    open = "playlist";
  }

  function stopPolling() {
    if (pollTimer) clearTimeout(pollTimer);
    pollTimer = null;
  }

  function stopAccountPoll() {
    if (accountTimer) clearTimeout(accountTimer);
    accountTimer = null;
  }

  function watchAccounts(restart = false) {
    if (!runningAccount) {
      stopAccountPoll();
      return;
    }
    if (restart || !accountTimer) accountStart = Date.now();
    if (accountTimer) return;
    accountTimer = setTimeout(pollAccounts, POLL_MS);
  }

  async function pollAccounts() {
    accountTimer = null;
    if (typeof document !== "undefined" && document.visibilityState === "hidden") return;
    if (Date.now() - accountStart > POLL_MAX) return;
    await loadOptions();
    watchAccounts();
  }

  function startPolling(delay = POLL_MS) {
    stopPolling();
    pollStart = Date.now();
    capped = false;
    schedule(delay);
  }

  function schedule(delay) {
    if (pollTimer) clearTimeout(pollTimer);
    if (typeof document !== "undefined" && document.visibilityState === "hidden") {
      pollTimer = null;
      return;
    }
    pollTimer = setTimeout(pollOnce, delay);
  }

  async function pollOnce() {
    pollTimer = null;
    if (!job || !job.id) return;
    if (Date.now() - pollStart > POLL_MAX) {
      capped = true;
      return;
    }
    try {
      const next = await api.importStatus(job.id);
      if (next && next.id) job = next;
      if (isImportRunning(job)) {
        schedule(POLL_MS);
        return;
      }
      memo("import", job.state === "saved" ? null : { account: accountName(), id: job.id, url: lastUrl });
    } catch (e) {
      if (e && e.status === 404) {
        job = { ...job, state: "error", error: "L'analyse a expiré : relancez-la." };
        memo("import", null);
        return;
      }
      schedule(POLL_MS * 2);
    }
  }

  function refresh() {
    startPolling(0);
  }

  function onVisibility() {
    if (document.visibilityState === "hidden") stopPolling();
    else if (job && isImportRunning(job) && !capped) schedule(0);
  }

  function failureFor(id) {
    return error && error.provider === id ? error.message : "";
  }
</script>

<div class="account">
  <header class="intro">
    <h1>Reliez ce que vous écoutez ailleurs</h1>
    <p class="sub">Chaque option est facultative. Rien n'est publié ni modifié sur vos comptes.</p>
  </header>

  {#if notice}
    <p class="notice" role="status">
      <Icon name="check" size={16} /><span>{notice}</span>
    </p>
  {/if}

  {#if loadError}
    <div class="failed">
      <LoadFailed
        notFound="Impossible de lire les options de compte. Réessayez dans un instant."
        on:retry={loadOptions}
      />
      <button class="ghost" type="button" on:click={loadOptions}>
        <Icon name="refresh" size={16} /> Réessayer
      </button>
    </div>
  {:else if !opts}
    <div class="cards" aria-busy="true">
      {#each [1, 2, 3] as _}
        <div class="card wait">
          <span class="sk sq"></span>
          <span class="sk l"></span>
          <span class="sk l short"></span>
          <span class="sk button"></span>
        </div>
      {/each}
    </div>
  {:else}
    {#if importInFlight}
      <p class="queued">
        <Icon name="refresh" size={14} />
        <span>Un import est déjà en cours. Attendez qu'il finisse pour en lancer un autre.</span>
      </p>
    {/if}
    <div class="cards">
      <article class="card" class:done={skipped.includes("deezer")}>
        {#if skipped.includes("deezer")}
          <div class="skipped">
            <span class="who"><Icon name="music" size={16} /> Deezer — Ignoré</span>
            <button class="link" type="button" on:click={() => unskip("deezer")}>
              Relier quand même
            </button>
          </div>
        {:else}
          <div class="head">
            <span class="glyph"><Icon name="music" size={20} /></span>
            <div class="titles">
              <h2>Deezer</h2>
              <p>
                Retrouvez vos artistes suivis, vos titres aimés et vos playlists. Le compte est lu,
                jamais modifié.
              </p>
            </div>
          </div>
          {#if dzState}
            <p class="state" class:ok={dzState.tone === "ok"} class:err={dzState.tone === "error"}>
              {#if dzState.tone === "ok"}<Icon name="check" size={14} />
              {:else if dzState.tone === "error"}<Icon name="alert" size={14} />
              {:else if dzState.tone === "busy"}<Icon name="refresh" size={14} />{/if}
              <span>{dzState.text}</span>
            </p>
          {/if}
          {#if dz && dz.account && dz.available}
            <p class="hint">
              {dz.account.running && dz.account.running.running
                ? "L'import de votre compte continue sur le serveur. Vous pouvez quitter cette page."
                : "Votre compte est relié : ses nouveaux éléments sont importés automatiquement."}
            </p>
          {:else if dz && open !== "deezer" && dz.can_link}
            <button
              class="primary"
              type="button"
              disabled={importInFlight}
              on:click={() => (open = "deezer")}
            >
              Relier mon compte
            </button>
          {:else if dz && dz.can_link}
            <label for="ob-arl">ARL Deezer</label>
            <input
              id="ob-arl"
              type="password"
              autocomplete="off"
              autocapitalize="none"
              spellcheck="false"
              bind:value={arl}
              aria-describedby="ob-arl-help"
              placeholder="Collez votre ARL"
            />
            <p class="hint" id="ob-arl-help">
              Votre ARL vaut une session : il est chiffré sur le serveur et ne sert qu'à lire vos
              favoris.
            </p>
            <button
              class="primary"
              type="button"
              disabled={busy === "deezer"}
              on:click={() => link("deezer")}
            >
              {busy === "deezer" ? "Connexion…" : "Relier mon compte"}
            </button>
          {/if}
          {#if failureFor("deezer")}
            <p class="err" role="alert"><Icon name="alert" size={14} /><span>{failureFor("deezer")}</span></p>
          {/if}
          <div class="cardfoot">
            <button class="ghost small" type="button" on:click={() => skip("deezer")}>Ignorer</button>
          </div>
        {/if}
      </article>

      <article class="card" class:done={skipped.includes("youtube")}>
        {#if skipped.includes("youtube")}
          <div class="skipped">
            <span class="who"><Icon name="youtube" size={16} /> YouTube Music — Ignoré</span>
            <button class="link" type="button" on:click={() => unskip("youtube")}>
              Relier quand même
            </button>
          </div>
        {:else}
          <div class="head">
            <span class="glyph"><Icon name="youtube" size={20} /></span>
            <div class="titles">
              <h2>YouTube Music</h2>
              <p>
                Importez vos playlists et vos favoris. La connexion se fait depuis un ordinateur,
                avec les en-têtes d'une session ouverte.
              </p>
            </div>
          </div>
          {#if ytState}
            <p class="state" class:ok={ytState.tone === "ok"} class:err={ytState.tone === "error"}>
              {#if ytState.tone === "ok"}<Icon name="check" size={14} />
              {:else if ytState.tone === "error"}<Icon name="alert" size={14} />
              {:else if ytState.tone === "busy"}<Icon name="refresh" size={14} />{/if}
              <span>{ytState.text}</span>
            </p>
          {/if}
          {#if yt && yt.account && yt.available}
            <p class="hint">
              {yt.account.running && yt.account.running.running
                ? "L'import de votre compte continue sur le serveur. Vous pouvez quitter cette page."
                : "Votre compte est relié : ses nouveaux éléments sont importés automatiquement."}
            </p>
          {:else if yt && yt.can_link && open !== "youtube"}
            <button
              class="primary"
              type="button"
              disabled={importInFlight}
              on:click={() => (open = "youtube")}
            >
              Relier YouTube Music
            </button>
          {:else if yt && yt.can_link}
            <label for="ob-yt">En-têtes de la requête browse</label>
            <textarea
              id="ob-yt"
              rows="4"
              spellcheck="false"
              autocapitalize="none"
              bind:value={headers}
              aria-describedby="ob-yt-help"
              placeholder="cookie: …&#10;authorization: …"
            ></textarea>
            <p class="hint" id="ob-yt-help">
              Copiez les en-têtes d'une requête browse de music.youtube.com, connecté. Ils valent une
              session : ils sont chiffrés sur le serveur.
            </p>
            <p class="hint">Cette connexion se fait depuis un ordinateur. Sur téléphone, importez plutôt une playlist.</p>
            <button
              class="primary"
              type="button"
              disabled={busy === "youtube"}
              on:click={() => link("youtube")}
            >
              {busy === "youtube" ? "Connexion…" : "Relier YouTube Music"}
            </button>
          {/if}
          {#if failureFor("youtube")}
            <p class="err" role="alert"><Icon name="alert" size={14} /><span>{failureFor("youtube")}</span></p>
          {/if}
          <div class="cardfoot">
            <button class="ghost small" type="button" on:click={() => skip("youtube")}>Ignorer</button>
          </div>
        {/if}
      </article>

      <article class="card playlist" class:done={skipped.includes("playlist")}>
        {#if skipped.includes("playlist")}
          <div class="skipped">
            <span class="who"><Icon name="link" size={16} /> Playlist publique — Ignoré</span>
            <button class="link" type="button" on:click={() => unskip("playlist")}>
              Importer quand même
            </button>
          </div>
        {:else}
          <div class="head">
            <span class="glyph"><Icon name="link" size={20} /></span>
            <div class="titles">
              <h2>Playlist publique</h2>
              <p>
                Importez une playlist publique par son lien. Les favoris d'un compte Spotify ne sont
                pas accessibles sans application développeur.
              </p>
            </div>
          </div>

          {#if job}
            <p class="state" class:ok={progress.saved || progress.done} class:err={progress.failed} role="status">
              {#if progress.failed}<Icon name="alert" size={14} />
              {:else if progress.busy}<Icon name="refresh" size={14} />
              {:else}<Icon name="check" size={14} />{/if}
              <span>{progress.label}</span>
            </p>
            {#if progress.busy}
              {#if progress.percent == null}
                <div class="bar indet" aria-hidden="true"><span></span></div>
              {:else}
                <div
                  class="bar"
                  role="progressbar"
                  aria-label="Progression de l'import"
                  aria-valuemin="0"
                  aria-valuemax="100"
                  aria-valuenow={progress.percent}
                >
                  <span style={`width:${progress.percent}%`}></span>
                </div>
              {/if}
              <p class="hint">L'import continue sur le serveur si vous quittez cette page.</p>
            {:else if progress.done}
              {#if missed}
                <details class="missed">
                  <summary>{missed} {missed > 1 ? "titres n'ont pas été trouvés" : "titre n'a pas été trouvé"}</summary>
                  <ul>
                    {#each missedList as item}
                      <li>
                        <span class="t">{item.line.title}</span>
                        <span class="a">{Array.isArray(item.line.artists) ? item.line.artists.join(", ") : ""}</span>
                      </li>
                    {/each}
                  </ul>
                  {#if missed > missedList.length}
                    <p class="hint">et {missed - missedList.length} autres.</p>
                  {/if}
                </details>
              {/if}
              {#if note}<p class="hint">{note}</p>{/if}
              {#if job.state === "done"}
                <button
                  class="primary"
                  type="button"
                  disabled={busy === "save"}
                  on:click={save}
                >
                  {busy === "save" ? "Ajout…" : "Ajouter à ma bibliothèque"}
                </button>
              {/if}
            {:else if progress.failed}
              <button class="ghost" type="button" on:click={retryImport}>Relancer</button>
            {/if}
            {#if capped && isImportRunning(job)}
              <button class="ghost small" type="button" on:click={refresh}>Actualiser</button>
            {/if}
          {:else if !playlistAvailable}
            <p class="hint">Aucune playlist publique n'est disponible sur ce serveur.</p>
          {:else if open !== "playlist"}
            <button
              class="primary"
              type="button"
              disabled={importInFlight}
              on:click={() => (open = "playlist")}
            >
              Importer une playlist
            </button>
          {:else}
            <label for="ob-url">Lien de la playlist</label>
            <input
              id="ob-url"
              type="url"
              inputmode="url"
              autocomplete="off"
              autocapitalize="none"
              spellcheck="false"
              bind:value={url}
              aria-describedby="ob-url-help"
              placeholder="https://open.spotify.com/playlist/…"
            />
            <p class="hint" id="ob-url-help">
              Deezer, Spotify, Apple Music ou YouTube. Les titres sont rapprochés de votre
              bibliothèque ; rien n'est enregistré avant votre accord.
            </p>
            <button class="primary" type="button" disabled={busy === "playlist"} on:click={startImport}>
              {busy === "playlist" ? "Analyse…" : "Importer"}
            </button>
          {/if}
          {#if failureFor("playlist")}
            <p class="err" role="alert"><Icon name="alert" size={14} /><span>{failureFor("playlist")}</span></p>
          {/if}
          <div class="cardfoot">
            <button class="ghost small" type="button" on:click={() => skip("playlist")}>Ignorer</button>
          </div>
        {/if}
      </article>
    </div>

    <p class="foot">Vous pourrez relier ces comptes plus tard dans Réglages, puis Sources.</p>
  {/if}
</div>

<style>
  .account {
    width: min(960px, 100%);
    margin: 0 auto;
    display: flex;
    flex-direction: column;
    gap: 24px;
  }
  .intro {
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
  .sub {
    margin: 0;
    max-width: 62ch;
    color: var(--dim);
    font-size: 14px;
    line-height: 1.45;
  }
  .notice {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    margin: 0;
    padding: 12px 14px;
    border: 1px solid var(--ln);
    border-radius: 12px;
    background: var(--s1);
    font-size: 13.5px;
    line-height: 1.45;
  }
  .notice :global(svg) {
    flex: none;
    margin-top: 1px;
    color: var(--ac-text);
  }
  .queued {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    margin: 0;
    color: var(--dim);
    font-size: 12.5px;
    line-height: 1.45;
  }
  .queued :global(svg) {
    flex: none;
    margin-top: 1px;
  }
  .cards {
    display: grid;
    grid-template-columns: 1fr;
    gap: 16px;
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 20px;
    border: 1px solid var(--ln);
    border-radius: var(--r-card);
    background: var(--s1);
  }
  .card.done {
    background: transparent;
  }
  .head {
    display: flex;
    align-items: flex-start;
    gap: 14px;
  }
  .glyph {
    flex: none;
    width: 44px;
    height: 44px;
    display: grid;
    place-items: center;
    border-radius: 12px;
    background: var(--s2);
    color: var(--tx);
  }
  .titles {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  h2 {
    margin: 0;
    font-family: var(--font-display);
    font-size: 18px;
    font-weight: 700;
    letter-spacing: -0.02em;
  }
  .titles p {
    margin: 0;
    color: var(--dim);
    font-size: 13.5px;
    line-height: 1.45;
  }
  .state {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    margin: 0;
    color: var(--dim);
    font-family: var(--font-mono);
    font-size: 12.5px;
    line-height: 1.45;
  }
  .state :global(svg) {
    flex: none;
    margin-top: 1px;
  }
  .state.ok {
    color: var(--tx);
  }
  .state.err {
    color: var(--ac-text);
  }
  label {
    font-size: 13px;
    font-weight: 600;
  }
  input,
  textarea {
    width: 100%;
    min-height: 48px;
    padding: 12px 14px;
    border: 1px solid var(--ln);
    border-radius: 12px;
    background: var(--bg);
    color: var(--tx);
    font-size: 15px;
    line-height: 1.4;
  }
  textarea {
    resize: vertical;
    font-family: var(--font-mono);
    font-size: 12.5px;
  }
  input:focus,
  textarea:focus {
    border-color: var(--ac-text);
  }
  .hint {
    margin: 0;
    color: var(--dim);
    font-size: 12.5px;
    line-height: 1.45;
  }
  .err {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    margin: 0;
    color: var(--ac-text);
    font-size: 12.5px;
    line-height: 1.45;
  }
  .err :global(svg) {
    flex: none;
    margin-top: 1px;
  }
  .primary,
  .ghost {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    min-height: 44px;
    padding: 0 18px;
    border-radius: 999px;
    font-size: 14px;
    font-weight: 700;
    align-self: flex-start;
  }
  .primary {
    background: var(--btn);
    color: var(--onbtn);
  }
  .primary:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .ghost {
    border: 1px solid var(--ln);
    color: var(--tx);
  }
  .ghost:hover {
    background: var(--s2);
  }
  .small {
    min-height: 44px;
    padding: 0 12px;
    border: 0;
    color: var(--dim);
    font-size: 13px;
    font-weight: 600;
  }
  .small:hover {
    color: var(--tx);
  }
  .cardfoot {
    display: flex;
    align-items: center;
    gap: 12px;
    justify-content: flex-end;
    flex-wrap: wrap;
    margin-top: auto;
  }
  .skipped {
    display: flex;
    align-items: center;
    gap: 12px;
    justify-content: space-between;
    min-height: 44px;
    flex-wrap: wrap;
  }
  .who {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    color: var(--dim);
    font-size: 14px;
  }
  .link {
    min-height: 44px;
    padding: 0 6px;
    color: var(--tx);
    font-size: 13.5px;
    font-weight: 600;
    text-decoration: underline;
    text-underline-offset: 3px;
  }
  .link:hover {
    color: var(--ac-text);
  }
  .bar {
    height: 3px;
    border-radius: 2px;
    background: var(--ln);
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--ac-text);
  }
  .bar.indet span {
    width: 36%;
    animation: slide 1.2s ease-in-out infinite;
  }
  @keyframes slide {
    0% {
      transform: translateX(-110%);
    }
    100% {
      transform: translateX(320%);
    }
  }
  .missed {
    font-size: 12.5px;
    color: var(--dim);
  }
  .missed summary {
    min-height: 44px;
    display: flex;
    align-items: center;
    cursor: pointer;
    color: var(--tx);
  }
  .missed ul {
    margin: 0;
    padding: 0 0 0 18px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 260px;
    overflow-y: auto;
  }
  .missed li {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .missed .t {
    color: var(--tx);
  }
  .missed .a {
    font-size: 11.5px;
  }
  .wait {
    gap: 10px;
  }
  .sk {
    display: block;
    border-radius: 8px;
    background: var(--s2);
  }
  .sk.sq {
    width: 44px;
    height: 44px;
    border-radius: 12px;
  }
  .sk.l {
    height: 12px;
    width: 72%;
  }
  .sk.l.short {
    width: 48%;
  }
  .sk.button {
    height: 44px;
    width: 160px;
    border-radius: 999px;
  }
  .failed {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 16px;
  }
  .failed :global(.failed) {
    margin: 0;
    padding: 0;
  }
  .foot {
    margin: 0 0 8px;
    color: var(--dim);
    font-size: 12.5px;
    line-height: 1.45;
  }
  @media (min-width: 900px) {
    .cards {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
    .card.playlist {
      grid-column: span 2;
    }
    h1 {
      font-size: 40px;
      line-height: 1.1;
    }
    .sub {
      font-size: 15px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .bar.indet span {
      animation: none;
      width: 100%;
      opacity: 0.5;
    }
    .primary,
    .ghost {
      transition: none;
    }
  }
</style>
