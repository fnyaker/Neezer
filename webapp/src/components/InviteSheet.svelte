<script>
  import { onMount, tick, createEventDispatcher } from "svelte";
  import { api } from "../lib/api.js";
  import { toasts } from "../lib/stores.js";
  import {
    guardMessage,
    inviteUrl,
    copyText,
  } from "../lib/adminusers.js";
  import Icon from "./Icon.svelte";

  export let source = null;

  const dispatch = createEventDispatcher();
  const DURATIONS = [
    { value: 3600, label: "1\u00a0heure" },
    { value: 86400, label: "24\u00a0heures" },
    { value: 604800, label: "7\u00a0jours" },
    { value: 2592000, label: "30\u00a0jours" },
  ];
  const DEFAULT_TTL = 604800;

  let note = source?.note || "";
  let ttl = DEFAULT_TTL;
  let step = "form";
  let busy = false;
  let error = "";
  let result = null;
  let copied = false;
  let shared = false;
  let confirmClose = false;
  let confirmRevoke = false;
  let revokeBusy = false;
  let sheetEl = null;
  let noteEl = null;
  let linkEl = null;

  $: delivered = copied || shared;
  $: sourceTtl = source
    ? Math.max(
        3600,
        Math.min(
          2592000,
          Math.round(
            (Date.parse(source.expires) - Date.parse(source.created)) / 1000,
          ) || DEFAULT_TTL,
        ),
      )
    : DEFAULT_TTL;
  $: sourceDuration = durationLabel(sourceTtl);
  $: canShare = typeof navigator !== "undefined" && !!navigator.share;

  onMount(() => {
    tick().then(() => {
      if (step === "form" && !source) noteEl?.focus();
      else sheetEl?.focus();
    });
  });

  function durationLabel(seconds) {
    const known = DURATIONS.find((item) => item.value === seconds);
    if (known) return known.label;
    const days = Math.round(seconds / 86400);
    return `${days} jour${days > 1 ? "s" : ""}`;
  }

  async function submit() {
    if (busy) return;
    busy = true;
    error = "";
    try {
      const response = source
        ? await api.adminInviteRenew(source.id)
        : await api.adminInviteCreate({
            note: note.trim() || undefined,
            expires_in: ttl,
          });
      result = {
        invite: response.invite,
        token: response.token,
        url: inviteUrl(response.token),
      };
      step = "success";
      dispatch("created", {
        invite: response.invite,
        url: result.url,
        replaces: source?.id || null,
      });
      tick().then(() => {
        linkEl?.focus();
        linkEl?.select();
        if (linkEl) linkEl.scrollLeft = 0;
      });
    } catch (err) {
      error = guardMessage(err);
      const code = err?.body?.code;
      if (source && (code === "invite_used" || code === "invite_not_found")) {
        dispatch("stale", source.id);
      }
    } finally {
      busy = false;
    }
  }

  async function copy() {
    if (!result?.url) return;
    const done = await copyText(result.url);
    if (done) {
      copied = true;
      toasts.push("Lien copié");
      return;
    }
    toasts.push(
      "Copie impossible — copiez le lien manuellement.",
      "error",
      6000,
      { label: "Sélectionner le lien", run: selectLink },
    );
    selectLink();
  }

  function selectLink() {
    linkEl?.focus();
    linkEl?.select();
    if (linkEl) linkEl.scrollLeft = 0;
  }

  async function share() {
    if (!result?.url || !navigator.share) return;
    try {
      await navigator.share({
        title: "Invitation Neezer",
        text: "Créez votre compte Neezer avec ce lien.",
        url: result.url,
      });
      shared = true;
    } catch (err) {
      if (err?.name !== "AbortError") toasts.push("Le partage a échoué.", "error");
    }
  }

  function requestClose() {
    if (busy) return;
    if (step === "success" && !delivered) {
      confirmClose = true;
      return;
    }
    dispatch("close");
  }

  async function revoke() {
    if (revokeBusy) return;
    const id = step === "success" ? result?.invite?.id : source?.id;
    if (!id) return;
    revokeBusy = true;
    error = "";
    try {
      await api.adminInviteRevoke(id);
      dispatch("revoked", id);
      dispatch("close");
    } catch (err) {
      const code = err?.body?.code;
      if (code === "invite_used" || code === "invite_not_found") {
        dispatch("stale", id);
        toasts.push(guardMessage(err), "error");
        dispatch("close");
      } else {
        error = guardMessage(err);
      }
    } finally {
      revokeBusy = false;
    }
  }

  function onTtlKey(event, index) {
    const forward = event.key === "ArrowRight" || event.key === "ArrowDown";
    const backward = event.key === "ArrowLeft" || event.key === "ArrowUp";
    if (!forward && !backward) return;
    event.preventDefault();
    const nextIndex =
      (index + (forward ? 1 : -1) + DURATIONS.length) % DURATIONS.length;
    ttl = DURATIONS[nextIndex].value;
    tick().then(() =>
      sheetEl?.querySelector(`[data-ttl="${ttl}"]`)?.focus(),
    );
  }

  function onKey(event) {
    if (event.key === "Escape") {
      event.preventDefault();
      if (confirmClose) confirmClose = false;
      else if (confirmRevoke) confirmRevoke = false;
      else requestClose();
      return;
    }
    if (event.key !== "Tab" || !sheetEl) return;
    const focusable = [
      ...sheetEl.querySelectorAll(
        "button:not([disabled]), input:not([disabled]), [href], [tabindex]:not([tabindex='-1'])",
      ),
    ].filter((element) => element.offsetParent !== null);
    if (!focusable.length) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    const active = document.activeElement;
    if (
      event.shiftKey &&
      (active === first || active === sheetEl || !sheetEl.contains(active))
    ) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && active === last) {
      event.preventDefault();
      first.focus();
    }
  }

  function toBody(node) {
    document.body.appendChild(node);
    return {
      destroy() {
        node.parentNode?.removeChild(node);
      },
    };
  }
</script>

<svelte:window on:keydown={onKey} />

<div class="overlay" role="presentation" use:toBody>
  <div
    class="sheet"
    role="dialog"
    aria-modal="true"
    aria-labelledby="invite-title"
    tabindex="-1"
    bind:this={sheetEl}
  >
    <header>
      <div class="head-copy">
        <h2 id="invite-title">
          {#if step === "success"}Lien d'invitation créé{:else if source}Renvoyer l'invitation{:else}Inviter quelqu'un{/if}
        </h2>
        <p class="sub">
          {#if step === "success"}
            Ce lien ne sera plus affiché. Il ne sert qu'à un compte.
          {:else if source}
            L'ancien lien sera révoqué immédiatement. Seul le nouveau fonctionnera.
          {:else}
            Le lien crée un compte membre. Il ne sert qu'une fois.
          {/if}
        </p>
      </div>
      <button class="icon-btn" on:click={requestClose} aria-label="Fermer">
        <Icon name="close" size={18} />
      </button>
    </header>

    {#if step === "form"}
      {#if source}
        <dl class="facts">
          <div>
            <dt>Pour qui</dt>
            <dd>{source.note || "Sans note"}</dd>
          </div>
          <div>
            <dt>Durée</dt>
            <dd>{sourceDuration}</dd>
          </div>
        </dl>
      {:else}
        <label class="label" for="invite-note">Pour qui ? (facultatif)</label>
        <div class="note-wrap">
          <input
            id="invite-note"
            class="field"
            bind:this={noteEl}
            bind:value={note}
            maxlength="64"
            placeholder="Prénom ou note, pour vous y retrouver"
            autocomplete="off"
            spellcheck="false"
          />
          <span class="counter mono">{note.length}/64</span>
        </div>
        <p class="help">
          Ce nom ne crée pas de compte et n'est pas montré à la personne invitée :
          il vous sert seulement à reconnaître le lien.
        </p>

        <div class="label" id="invite-ttl-label">Durée de validité</div>
        <div class="segments" role="radiogroup" aria-labelledby="invite-ttl-label">
          {#each DURATIONS as duration, index (duration.value)}
            <button
              type="button"
              class="segment"
              class:on={ttl === duration.value}
              role="radio"
              aria-checked={ttl === duration.value}
              data-ttl={duration.value}
              on:click={() => (ttl = duration.value)}
              on:keydown={(event) => onTtlKey(event, index)}
            >
              {duration.label}
            </button>
          {/each}
        </div>
      {/if}

      <p class="help">
        Vous pourrez nommer un administrateur après la création du compte.
      </p>

      {#if error}
        <div class="inline-error" role="alert">
          <Icon name="alert" size={15} />
          <span>{error}</span>
          <button class="retry" on:click={submit}>Réessayer</button>
        </div>
      {/if}

      {#if source}
        {#if confirmRevoke}
          <div class="inline-confirm" role="alert">
            <p>Révoquer l'invitation ? Le lien actuel cessera de fonctionner.</p>
            <div>
              <button class="btn ghost" on:click={() => (confirmRevoke = false)}>
                Annuler
              </button>
              <button class="btn danger" on:click={revoke} disabled={revokeBusy}>
                <Icon name="trash" size={16} />
                {revokeBusy ? "Révocation…" : "Révoquer"}
              </button>
            </div>
          </div>
        {:else}
          <button class="danger-link" on:click={() => (confirmRevoke = true)}>
            Révoquer l'invitation
          </button>
        {/if}
      {/if}

      <footer>
        <button class="btn ghost" on:click={requestClose}>Annuler</button>
        <button class="btn primary" on:click={submit} disabled={busy}>
          {#if busy}
            {source ? "Renouvellement…" : "Création…"}
          {:else}
            <Icon name="link" size={16} />
            {source ? "Créer un nouveau lien" : "Créer le lien"}
          {/if}
        </button>
      </footer>
    {:else}
      <input
        class="link-field mono"
        readonly
        bind:this={linkEl}
        value={result?.url || ""}
        title={result?.url || ""}
        on:focus={(event) => {
          event.currentTarget.select();
          event.currentTarget.scrollLeft = 0;
        }}
      />

      <p class="warning">
        <Icon name="alert" size={15} />
        Copiez ce lien maintenant : il ne sera plus affiché.
      </p>

      <div class="success-actions">
        <button class="btn primary" on:click={copy}>
          <Icon name="copy" size={16} />
          {copied ? "Lien copié" : "Copier le lien"}
        </button>
        {#if canShare}
          <button class="btn ghost" on:click={share}>
            <Icon name="share" size={16} />
            Partager
          </button>
        {/if}
      </div>

      {#if error}
        <div class="inline-error" role="alert">
          <Icon name="alert" size={15} />
          <span>{error}</span>
        </div>
      {/if}

      {#if confirmRevoke}
        <div class="inline-confirm" role="alert">
          <p>Révoquer ce lien ? Il ne fonctionnera plus.</p>
          <div>
            <button class="btn ghost" on:click={() => (confirmRevoke = false)}>
              Annuler
            </button>
            <button class="btn danger" on:click={revoke} disabled={revokeBusy}>
              <Icon name="trash" size={16} />
              {revokeBusy ? "Révocation…" : "Révoquer"}
            </button>
          </div>
        </div>
      {:else}
        <button class="danger-link" on:click={() => (confirmRevoke = true)}>
          Révoquer ce lien
        </button>
      {/if}

      {#if confirmClose}
        <div class="inline-confirm" role="alert">
          <p>
            Fermer sans copier le lien ? Le lien ne sera plus affiché. Vous pourrez
            renvoyer une invitation, mais ce lien ne reviendra pas.
          </p>
          <div>
            <button class="btn ghost" on:click={() => (confirmClose = false)}>
              Annuler
            </button>
            <button class="btn danger" on:click={() => dispatch("close")}>
              Fermer quand même
            </button>
          </div>
        </div>
      {:else}
        <footer>
          <button class="btn ghost" on:click={requestClose}>Fermer</button>
        </footer>
      {/if}
    {/if}
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 440;
    display: grid;
    place-items: center;
    padding: 20px;
    background: color-mix(in srgb, var(--onac) 60%, transparent);
    overscroll-behavior: contain;
    touch-action: none;
  }
  .sheet {
    width: min(480px, 100%);
    max-height: min(720px, calc(100dvh - 40px));
    box-sizing: border-box;
    overflow-y: auto;
    padding: 24px;
    border: 1px solid var(--ln);
    border-radius: 24px;
    background: var(--s1);
    color: var(--tx);
    box-shadow: 0 24px 64px color-mix(in srgb, var(--onac) 45%, transparent);
    touch-action: auto;
  }
  .sheet:focus {
    outline: none;
  }
  header {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    margin-bottom: 20px;
  }
  .head-copy {
    flex: 1;
    min-width: 0;
  }
  h2 {
    margin: 0;
    font-family: var(--font-display);
    font-size: 23px;
    font-weight: 700;
    letter-spacing: -0.02em;
    line-height: 1.12;
  }
  .sub {
    margin: 6px 0 0;
    color: var(--dim);
    font-size: 13.5px;
    line-height: 1.45;
  }
  .icon-btn {
    display: grid;
    place-items: center;
    width: 44px;
    height: 44px;
    flex: none;
    margin: -8px -8px 0 0;
    border-radius: 50%;
    color: var(--dim);
  }
  .icon-btn:hover {
    background: var(--s2);
    color: var(--tx);
  }
  .label {
    display: block;
    margin: 0 0 7px;
    color: var(--dim);
    font-family: var(--font-mono);
    font-size: 10.5px;
    font-weight: 500;
    letter-spacing: 0.07em;
    text-transform: uppercase;
  }
  .note-wrap {
    position: relative;
  }
  .field {
    width: 100%;
    height: 48px;
    box-sizing: border-box;
    padding: 0 54px 0 14px;
    border: 1px solid var(--ln);
    border-radius: 12px;
    background: var(--bg);
    color: var(--tx);
    font: inherit;
    font-size: 15px;
  }
  .field::placeholder {
    color: var(--dim);
  }
  .field:focus-visible {
    outline: none;
    border-color: var(--ac-text);
  }
  .counter {
    position: absolute;
    right: 12px;
    top: 50%;
    transform: translateY(-50%);
    color: var(--dim);
    font-size: 10.5px;
    pointer-events: none;
  }
  .help {
    margin: 7px 0 18px;
    color: var(--dim);
    font-size: 12.5px;
    line-height: 1.45;
  }
  .segments {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 4px;
    padding: 4px;
    margin-bottom: 16px;
    border: 1px solid var(--ln);
    border-radius: 999px;
    background: var(--bg);
  }
  .segment {
    min-height: 44px;
    padding: 0 8px;
    border-radius: 999px;
    color: var(--dim);
    font-size: 12.5px;
    font-weight: 600;
    white-space: nowrap;
  }
  .segment:hover {
    color: var(--tx);
  }
  .segment.on {
    background: var(--ac);
    color: var(--onac);
    font-weight: 700;
  }
  .facts {
    display: grid;
    gap: 12px;
    margin: 0 0 18px;
    padding: 14px;
    border: 1px solid var(--ln);
    border-radius: 14px;
    background: var(--bg);
  }
  .facts dt {
    color: var(--dim);
    font-family: var(--font-mono);
    font-size: 10.5px;
    font-weight: 500;
    letter-spacing: 0.07em;
    text-transform: uppercase;
  }
  .facts dd {
    margin: 3px 0 0;
    font-size: 14px;
    line-height: 1.4;
    overflow-wrap: anywhere;
  }
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    min-height: 44px;
    padding: 0 16px;
    border: 1px solid transparent;
    border-radius: 999px;
    background: var(--s2);
    color: var(--tx);
    font-size: 14px;
    font-weight: 600;
    line-height: 1;
    white-space: nowrap;
  }
  .btn.primary {
    background: var(--btn);
    color: var(--onbtn);
  }
  .btn.ghost {
    background: transparent;
    border-color: var(--ln);
  }
  .btn.danger {
    background: transparent;
    border-color: var(--ln);
    color: var(--tx);
  }
  .btn:hover:not(:disabled) {
    border-color: var(--ac-text);
  }
  .btn:disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 22px;
    flex-wrap: wrap;
  }
  .danger-link {
    min-height: 44px;
    padding: 0 2px;
    color: var(--dim);
    font-size: 13px;
    font-weight: 600;
    text-decoration: underline;
  }
  .danger-link:hover {
    color: var(--tx);
  }
  .link-field {
    width: 100%;
    height: 48px;
    box-sizing: border-box;
    padding: 0 12px;
    border: 1px solid var(--ln);
    border-radius: 12px;
    background: var(--bg);
    color: var(--tx);
    font-size: 12.5px;
  }
  .link-field:focus-visible {
    outline: none;
    border-color: var(--ac-text);
  }
  .warning {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    margin: 12px 0 16px;
    padding: 10px 12px;
    border: 1px solid var(--ln);
    border-radius: 12px;
    color: var(--tx);
    font-size: 13px;
    line-height: 1.45;
  }
  .warning :global(svg) {
    color: var(--ac-text);
    flex: none;
    margin-top: 1px;
  }
  .success-actions {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    margin-bottom: 12px;
  }
  .inline-confirm {
    margin-top: 14px;
    padding: 12px;
    border: 1px solid var(--ln);
    border-radius: 12px;
    background: var(--bg);
  }
  .inline-confirm p {
    margin: 0 0 12px;
    color: var(--tx);
    font-size: 13px;
    line-height: 1.45;
  }
  .inline-confirm > div {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    flex-wrap: wrap;
  }
  .inline-error {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    margin: 0 0 14px;
    padding: 10px 12px;
    border: 1px solid var(--ln);
    border-radius: 12px;
    color: var(--tx);
    font-size: 13px;
    line-height: 1.45;
  }
  .inline-error :global(svg) {
    color: var(--ac-text);
    flex: none;
    margin-top: 1px;
  }
  .retry {
    margin-left: auto;
    min-height: 44px;
    padding: 0 8px;
    color: var(--ac-text);
    font-size: 13px;
    font-weight: 600;
    text-decoration: underline;
    white-space: nowrap;
  }
  @media (max-width: 640px) {
    .overlay {
      align-items: end;
      padding: 0;
    }
    .sheet {
      width: 100%;
      max-height: 92dvh;
      border-radius: 24px 24px 0 0;
      padding: 20px 20px calc(20px + env(safe-area-inset-bottom));
    }
    .segment {
      font-size: 12px;
      padding: 0 4px;
    }
  }
</style>
