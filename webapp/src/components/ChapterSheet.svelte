<script>
  import { onDestroy, onMount, tick } from "svelte";
  import { fade } from "svelte/transition";
  import { loadChapters } from "../lib/chapters.js";
  import ChapterList from "./ChapterList.svelte";
  import Icon from "./Icon.svelte";

  const reduceMotion =
    typeof window !== "undefined" &&
    window.matchMedia &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  export let episode = null;
  export let chapters = [];
  export let currentIndex = -1;
  export let state = "ready";
  export let error = null;
  export let onSeek = null;
  export let onClose = null;
  export let onRetry = null;

  let sheetEl = null;
  let closeBtn = null;
  let opener = null;

  $: loading = state === "loading" || state === "idle";
  $: title = episode?.title || "";

  onMount(() => {
    opener = document.activeElement;
    tick().then(() => closeBtn?.focus({ preventScroll: true }));
    if (state === "idle" && !chapters.length && episode?.id) loadChapters(episode.id);
  });

  onDestroy(() => {
    if (opener && opener.isConnected) opener.focus?.({ preventScroll: true });
  });

  function onKey(e) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onClose?.();
      return;
    }
    if (e.key !== "Tab" || !sheetEl) return;
    const focusable = [
      ...sheetEl.querySelectorAll(
        'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'
      ),
    ].filter((el) => el.getClientRects().length > 0);
    if (!focusable.length) {
      e.preventDefault();
      sheetEl.focus();
      return;
    }
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (e.shiftKey && (document.activeElement === first || document.activeElement === sheetEl)) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  }

  function seek(start) {
    onSeek?.(start);
    onClose?.();
  }

  function backdrop(e) {
    if (e.target === e.currentTarget) onClose?.();
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

<div
  class="chsheet"
  use:toBody
  in:fade={{ duration: reduceMotion ? 0 : 150 }}
  role="dialog"
  tabindex="-1"
  aria-modal="true"
  aria-label={"Chapitres · " + title}
  on:click={backdrop}
  on:keydown={onKey}
>
  <div class="chsheet-p" bind:this={sheetEl} tabindex="-1">
    <header class="chsheet-h">
      <div class="chsheet-meta">
        <span class="chsheet-k">Chapitres</span>
        <span class="chsheet-t" title={title}>{title}</span>
      </div>
      <button
        type="button"
        class="chsheet-close"
        bind:this={closeBtn}
        aria-label="Fermer les chapitres"
        on:click={() => onClose?.()}
      >
        <Icon name="close" size={22} />
      </button>
    </header>
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div class="chsheet-b" tabindex="0">
      {#if loading}
        <p class="note status" role="status">Chargement des chapitres…</p>
        <ChapterList loading />
      {:else if state === "error" && !chapters.length}
        <p class="note">
          {error?.offline
            ? "Hors ligne : les chapitres n'ont pas encore été chargés."
            : "Chapitres indisponibles."}
        </p>
        <button type="button" class="retry" on:click={() => onRetry?.()}>
          <Icon name="refresh" size={15} /> Réessayer
        </button>
      {:else if !chapters.length}
        <p class="note">Aucun chapitre pour cet épisode.</p>
      {:else}
        {#if error}
          <p class="note offline"><Icon name="cloudOff" size={14} /> Hors ligne — dernière copie</p>
        {/if}
        <ChapterList {chapters} {currentIndex} onSeek={seek} variant="panel" />
      {/if}
    </div>
  </div>
</div>

<style>
  .chsheet {
    position: fixed;
    inset: 0;
    z-index: 300;
    display: grid;
    place-items: center;
    padding: 24px;
    background: color-mix(in srgb, var(--bg) 60%, transparent);
  }
  .chsheet-p {
    display: flex;
    flex-direction: column;
    width: min(520px, 100%);
    max-height: 80vh;
    min-height: 0;
    border: 1px solid var(--ln);
    border-radius: 20px;
    background: var(--s1);
    color: var(--tx);
    overflow: hidden;
  }
  .chsheet-p:focus {
    outline: none;
  }
  .chsheet-h {
    flex: none;
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 64px;
    padding: 8px 10px 8px 18px;
    border-bottom: 1px solid var(--ln);
  }
  .chsheet-meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }
  .chsheet-k {
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--dim);
  }
  .chsheet-t {
    font-family: var(--font-display);
    font-size: 1.15rem;
    font-weight: 700;
    line-height: 1.2;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .chsheet-close {
    display: grid;
    place-items: center;
    width: 44px;
    height: 44px;
    border-radius: 50%;
    color: var(--tx);
    flex: none;
  }
  .chsheet-close:hover {
    background: var(--s2);
  }
  .chsheet-close:focus-visible {
    outline: 2px solid var(--ac-text);
    outline-offset: 2px;
  }
  .chsheet-b {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
    padding: 4px 12px calc(24px + var(--safe-bottom, env(safe-area-inset-bottom, 0px)));
  }
  .chsheet-b:focus-visible {
    outline: 2px solid var(--ac-text);
    outline-offset: -2px;
  }
  .note {
    margin: 10px 2px 12px;
    font-size: 0.9rem;
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
  @media (max-width: 640px) {
    .chsheet {
      display: block;
      padding: 0;
      background: var(--s1);
    }
    .chsheet-p {
      width: 100%;
      height: 100%;
      max-height: none;
      border: 0;
      border-radius: 0;
    }
    .chsheet-h {
      min-height: 64px;
      padding: calc(var(--safe-top, env(safe-area-inset-top, 0px)) + 8px) 10px 8px 18px;
    }
    .chsheet-b {
      padding: 4px 14px calc(24px + var(--safe-bottom, env(safe-area-inset-bottom, 0px)));
    }
  }
</style>
