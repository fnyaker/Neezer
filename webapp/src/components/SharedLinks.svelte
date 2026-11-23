<script>
  // Réglages → Compte → Liens partagés: every public link this account still
  // holds, newest first. A link is a capability (one track, no account) and it
  // is cut here; the public page lives in neezer/webui/trackshare.py.
  import { onMount } from "svelte";
  import { api } from "../lib/api.js";
  import { toasts } from "../lib/stores.js";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";

  let links = [];
  let loading = true;
  let failed = false;
  let busy = null;
  let confirm = null;

  onMount(load);

  async function load() {
    loading = true;
    failed = false;
    try {
      const r = await api.trackShares();
      links = r.links || [];
    } catch {
      failed = true;
    }
    loading = false;
  }

  function url(l) {
    return window.location.origin + l.url;
  }

  async function copy(l) {
    try {
      await navigator.clipboard.writeText(url(l));
      toasts.push("Lien copié");
    } catch {
      toasts.push("Copie impossible", "error");
    }
  }

  async function revoke(l) {
    if (busy) return;
    busy = l.id;
    try {
      await api.revokeTrackShareById(l.id);
      links = links.filter((x) => x.id !== l.id);
      confirm = null;
      toasts.push("Lien révoqué");
    } catch {
      toasts.push("Révocation impossible", "error");
    }
    busy = null;
  }

  function expiry(l) {
    if (!l.expires) return "Sans expiration";
    try {
      return `Expire le ${new Date(l.expires).toLocaleDateString("fr-FR", {
        day: "numeric",
        month: "long",
        year: "numeric",
      })}`;
    } catch {
      return "Expire bientôt";
    }
  }

  function listened(l) {
    if (!l.last_used) return "Pas encore écouté";
    try {
      return `Écouté le ${new Date(l.last_used).toLocaleDateString("fr-FR", {
        day: "numeric",
        month: "short",
      })}`;
    } catch {
      return "Écouté";
    }
  }
</script>

<section class="card">
  <h2>Liens partagés</h2>
  <p class="sub muted">
    Chaque lien ouvre une page publique qui joue <strong>ce titre seul</strong>,
    sans compte et sans téléchargement. Révoquer un lien le coupe immédiatement.
  </p>

  {#if loading}
    <div class="state">
      <span class="spinner" data-motion="status" aria-label="Chargement"></span>
    </div>
  {:else if failed}
    <div class="state">
      <span class="muted">Impossible de charger vos liens pour l'instant.</span>
      <button class="ghost" on:click={load}>Réessayer</button>
    </div>
  {:else if !links.length}
    <div class="state">
      <span class="glyph"><Icon name="link" size={18} /></span>
      <span class="muted">
        Aucun lien pour l'instant. Ouvrez le partage d'un titre et choisissez
        <strong>Lien</strong>.
      </span>
    </div>
  {:else}
    <ul class="links">
      {#each links as l (l.id)}
        <li>
          <span class="art">
            <Cover src={l.track.cover} alt="" size={42} kind="track" fallbackId={l.track.id} />
          </span>
          <span class="txt">
            <span class="t">{l.track.title}</span>
            <span class="a muted">{l.track.artist}</span>
            <span class="meta muted">{expiry(l)} · {listened(l)}</span>
          </span>
          {#if confirm === l.id}
            <span class="confirm">
              <button class="ghost" on:click={() => (confirm = null)} disabled={busy === l.id}>Annuler</button>
              <button class="danger" on:click={() => revoke(l)} disabled={busy === l.id}>
                {busy === l.id ? "Révocation…" : "Révoquer"}
              </button>
            </span>
          {:else}
            <span class="acts">
              <button class="icon" on:click={() => copy(l)} aria-label="Copier le lien" title="Copier le lien">
                <Icon name="copy" size={17} />
              </button>
              <button
                class="icon cut"
                on:click={() => (confirm = l.id)}
                aria-label="Révoquer le lien"
                title="Révoquer le lien"
              >
                <Icon name="trash" size={17} />
              </button>
            </span>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .card {
    background: var(--bg-card);
    border-radius: var(--radius);
    padding: 20px;
    margin-bottom: 18px;
  }
  .card h2 {
    margin-top: 0;
    font-size: 1.05rem;
  }
  .sub {
    font-size: 0.85rem;
    line-height: 1.5;
    margin: 4px 0 12px;
  }
  .sub strong {
    color: var(--text);
    font-weight: 600;
  }
  .state {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    flex-wrap: wrap;
    min-height: 74px;
    text-align: center;
    font-size: 0.86rem;
    line-height: 1.45;
  }
  .glyph {
    display: grid;
    place-items: center;
    width: 38px;
    height: 38px;
    border-radius: 12px;
    background: var(--bg-hover);
    color: var(--accent);
  }
  .spinner {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    border: 2px solid var(--bg-hover);
    border-top-color: var(--accent);
    animation: spin 0.7s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .links {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .links li {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 60px;
    padding: 8px 2px;
  }
  .links li + li {
    border-top: 1px solid var(--bg-hover);
  }
  .art {
    flex: none;
    width: 42px;
    height: 42px;
    border-radius: 8px;
    overflow: hidden;
  }
  .art :global(> *) {
    width: 100%;
    height: 100%;
  }
  .txt {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .t {
    font-weight: 600;
    font-size: 0.9rem;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .a,
  .meta {
    font-size: 0.78rem;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .meta {
    font-variant-numeric: tabular-nums;
  }
  .acts {
    flex: none;
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .icon {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border-radius: 10px;
    color: var(--text-dim);
  }
  .icon:hover {
    color: var(--text);
    background: var(--bg-hover);
  }
  .icon.cut:hover {
    color: #ffb4a8;
  }
  .confirm {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .ghost {
    padding: 8px 12px;
    border: 1px solid var(--bg-hover);
    border-radius: 999px;
    font-size: 0.8rem;
    font-weight: 600;
    color: var(--text);
  }
  .ghost:hover:not(:disabled) {
    background: var(--bg-hover);
  }
  .danger {
    padding: 8px 12px;
    border: 1px solid rgba(255, 120, 100, 0.4);
    border-radius: 999px;
    font-size: 0.8rem;
    font-weight: 700;
    color: #ffb4a8;
  }
  .danger:hover:not(:disabled) {
    background: rgba(255, 120, 100, 0.12);
  }
  @media (max-width: 560px) {
    .links li {
      align-items: flex-start;
      flex-wrap: wrap;
    }
    .txt {
      flex-basis: calc(100% - 54px);
    }
    .acts,
    .confirm {
      margin-left: 54px;
    }
  }
</style>
