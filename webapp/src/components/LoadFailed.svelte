<script>
  import { createEventDispatcher } from "svelte";
  import { isDeezerDenied, isDeezerUnreachable, failureCause } from "../lib/api.js";
  import Icon from "./Icon.svelte";

  // F6: the block arrives with a 140 ms fade and a 4 px rise (see the `.failed`
  // rule below), and it replaces whatever was in the flow at once (M7: no
  // outro, or the page would keep the failed block's height while the retry's
  // content enters). It is a CSS animation with the motion TOKENS, not a
  // Svelte transition, and that is deliberate: this component is mounted with
  // its error already known, so the `{#if}` below is created during the
  // component's own first render — Svelte's local rule then never plays an
  // `in:` on it, measured here: `Element.prototype.animate` was not called once
  // for the block, on the first paint or on the retry. A keyframe runs on every
  // element creation, and the tokens give eco 0 ms and reduced motion a 0 px
  // move for free.

  export let error = null;
  export let notFound = "Introuvable.";
  export let inline = false;

  const dispatch = createEventDispatcher();
  $: status = error?.status ?? null;
  $: down = isDeezerUnreachable(error);
  $: denied = isDeezerDenied(error);
  $: offline = !down && (status === 0 || error?.offline === true);
  $: missing = !denied && !down && (status === 404 || !error);
  $: expired = !down && status === 401;
  $: refused = !down && status === 403;
  $: cause = failureCause(error);
  $: retry = () => dispatch("retry");
  $: verdict = /introuvable\.?$/i.test(String(notFound || "").trim());
  $: detail = verdict
    ? offline
      ? "Cette page n'est pas encore enregistrée sur cet appareil. Reconnecte-toi, puis réessaie."
      : `Cette page n'a pas pu être chargée : ${cause}.`
    : status !== null && status >= 500
      ? `${notFound} (${cause})`
      : notFound;
</script>

{#if denied}
  <div class="failed" class:inline role="status">
    <span class="icon"><Icon name="shield" size={inline ? 18 : 28} /></span>
    <div class="body">
      <h2>Réservé à d'autres comptes</h2>
      <p>
        Cette page n'existe que chez Deezer, et le compte Deezer de ce serveur ne t'est
        pas prêté. Tout ce qui est déjà sur le serveur reste à toi.
      </p>
    </div>
  </div>
{:else if down}
  <div class="failed" class:inline role="status">
    <span class="icon"><Icon name="cloudOff" size={inline ? 18 : 28} /></span>
    <div class="body">
      <h2>Deezer est injoignable</h2>
      <p>
        Cette page n'existe que chez Deezer et n'est pas encore dans ta bibliothèque.
        Tout ce que tu as téléchargé reste disponible.
      </p>
      <button class="pill ghost" on:click={retry}>
        <Icon name="refresh" size={16} /> Réessayer
      </button>
    </div>
  </div>
{:else if missing}
  <p class="muted">{notFound}</p>
{:else if offline}
  {#if verdict}
    <div class="failed" class:inline role="alert">
      <span class="icon"><Icon name="cloudOff" size={inline ? 18 : 28} /></span>
      <div class="body">
        <h2>Hors ligne</h2>
        <p>{detail}</p>
        <button class="pill ghost" on:click={retry}>
          <Icon name="refresh" size={16} /> Réessayer
        </button>
      </div>
    </div>
  {:else}
    <p class="muted">{notFound}</p>
  {/if}
{:else if expired}
  <div class="failed" class:inline role="alert">
    <span class="icon"><Icon name="shield" size={inline ? 18 : 28} /></span>
    <div class="body">
      <h2>Session expirée</h2>
      <p>Ta session ne répond plus : reconnecte-toi pour retrouver cette page.</p>
    </div>
  </div>
{:else if refused}
  <div class="failed" class:inline role="alert">
    <span class="icon"><Icon name="shield" size={inline ? 18 : 28} /></span>
    <div class="body">
      <h2>Accès refusé</h2>
      <p>Ce contenu ne t'est pas accessible depuis ce compte.</p>
    </div>
  </div>
{:else}
  {#if verdict}
    <div class="failed" class:inline role="alert">
      <span class="icon"><Icon name="alert" size={inline ? 18 : 28} /></span>
      <div class="body">
        <h2>Le chargement a échoué</h2>
        <p>{detail}</p>
        <button class="pill ghost" on:click={retry}>
          <Icon name="refresh" size={16} /> Réessayer
        </button>
      </div>
    </div>
  {:else}
    <p class="muted">{notFound}</p>
  {/if}
{/if}

<style>
  .failed {
    animation: blockIn var(--dur-fast) var(--ease-enter);
    max-width: 420px;
    margin: 12vh auto 0;
    padding: 0 16px;
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
  }
  @keyframes blockIn {
    from {
      opacity: 0;
      /* `--move-s` is 0px under reduced motion: the movement goes, the fade
         stays (M14). */
      transform: translateY(var(--move-s));
    }
    to {
      opacity: 1;
      transform: none;
    }
  }
  .failed.inline {
    max-width: none;
    margin: 0;
    padding: 14px 16px;
    flex-direction: row;
    align-items: flex-start;
    gap: 12px;
    text-align: left;
    border: 1px solid var(--ln);
    border-radius: var(--r-card);
    background: var(--s1);
  }
  .body {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }
  .inline .body {
    align-items: flex-start;
    gap: 6px;
  }
  .icon {
    display: grid;
    place-items: center;
    width: 64px;
    height: 64px;
    flex: none;
    border-radius: 50%;
    background: var(--bg-hover);
    color: var(--text-dim);
    margin-bottom: 4px;
  }
  .inline .icon {
    width: 32px;
    height: 32px;
    margin-bottom: 0;
  }
  h2 {
    margin: 0;
    font-size: 1.2rem;
  }
  .inline h2 {
    font-size: 1rem;
  }
  p {
    margin: 0 0 8px;
    color: var(--text-dim);
    line-height: 1.5;
  }
  .inline p {
    margin: 0;
    font-size: 0.9rem;
  }
</style>
