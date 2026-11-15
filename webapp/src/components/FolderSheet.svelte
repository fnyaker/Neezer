<script>
  import { tick } from "svelte";
  import { folderSheet, closeFolderSheet } from "../lib/stores.js";
  import { createFolder, renameFolder } from "../lib/actions.js";
  import Icon from "./Icon.svelte";

  let opened = false;
  let mode = "create";
  let name = "";
  let error = "";
  let busy = false;
  let sheet = null;
  let inputEl = null;
  let returnFocus = null;

  $: if ($folderSheet && !opened) enter();
  else if (!$folderSheet && opened) leave();

  function enter() {
    opened = true;
    mode = $folderSheet.mode === "rename" ? "rename" : "create";
    name = mode === "rename" ? $folderSheet.folder?.title || "" : "";
    error = "";
    busy = false;
    returnFocus = typeof document !== "undefined" ? document.activeElement : null;
    tick().then(() => {
      inputEl?.focus({ preventScroll: true });
      if (mode === "rename") inputEl?.select();
    });
  }

  function leave() {
    opened = false;
    const back = returnFocus;
    returnFocus = null;
    tick().then(() => {
      if (back && back.isConnected) back.focus?.({ preventScroll: true });
    });
  }

  function close() {
    if (!busy) closeFolderSheet();
  }

  async function submit() {
    if (busy) return;
    const title = name.trim();
    if (!title) {
      error = "Le nom du dossier ne peut pas être vide.";
      tick().then(() => inputEl?.focus({ preventScroll: true }));
      return;
    }
    busy = true;
    try {
      if (mode === "rename") {
        const ok = await renameFolder($folderSheet.folder.id, title);
        if (ok) closeFolderSheet();
      } else {
        const payload = $folderSheet;
        const folder = await createFolder(title);
        if (folder) {
          closeFolderSheet();
          payload.after?.(folder);
        }
      }
    } finally {
      busy = false;
    }
  }

  function onKey(e) {
    if (!$folderSheet) return;
    if (e.key === "Escape") {
      e.preventDefault();
      close();
      return;
    }
    if (e.key !== "Tab" || !sheet) return;
    const focusable = [...sheet.querySelectorAll("button, input, [href], [tabindex]:not([tabindex='-1'])")].filter(
      (el) => !el.disabled && el.offsetParent !== null
    );
    if (!focusable.length) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    const active = document.activeElement;
    if (e.shiftKey && (active === first || active === sheet || !sheet.contains(active))) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && active === last) {
      e.preventDefault();
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

{#if $folderSheet}
  <div class="overlay" role="presentation" use:toBody on:click={(e) => e.target === e.currentTarget && close()}>
    <div
      class="sheet"
      role="dialog"
      aria-modal="true"
      aria-label={mode === "rename" ? "Renommer le dossier" : "Nouveau dossier"}
      tabindex="-1"
      bind:this={sheet}
    >
      <header>
        <h2>{mode === "rename" ? "Renommer le dossier" : "Nouveau dossier"}</h2>
        <button type="button" class="close" on:click={close} aria-label="Fermer">
          <Icon name="close" size={18} />
        </button>
      </header>
      <form on:submit|preventDefault={submit}>
        <label class="label-mono" for="folder-name">Nom du dossier</label>
        <input
          id="folder-name"
          class="nameinput"
          type="text"
          bind:value={name}
          bind:this={inputEl}
          maxlength="80"
          autocomplete="off"
          aria-invalid={!!error}
          aria-describedby={error ? "folder-name-error" : undefined}
          on:input={() => (error = "")}
        />
        {#if error}
          <p class="error" id="folder-name-error" role="alert">{error}</p>
        {/if}
        <div class="actions">
          <button type="button" class="btn btn-ghost" on:click={close} disabled={busy}>Annuler</button>
          <button type="submit" class="btn btn-primary" disabled={busy}>
            {mode === "rename" ? "Renommer" : "Créer"}
          </button>
        </div>
      </form>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 430;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 20px;
    background: color-mix(in srgb, var(--onac) 60%, transparent);
    overscroll-behavior: contain;
    touch-action: none;
  }
  .sheet {
    width: min(440px, 100%);
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: 18px;
    padding: 24px;
    background: var(--s1);
    border: 1px solid var(--ln);
    border-radius: 24px;
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
    justify-content: space-between;
    gap: 12px;
  }
  h2 {
    margin: 0;
    font-family: var(--font-display);
    font-size: 24px;
    font-weight: 700;
    letter-spacing: -0.02em;
    line-height: 1.1;
  }
  .close {
    flex: none;
    width: 36px;
    height: 36px;
    display: grid;
    place-items: center;
    border-radius: 999px;
    background: var(--s2);
    color: var(--tx);
  }
  .close:hover {
    background: color-mix(in srgb, var(--tx) 12%, var(--s2));
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .nameinput {
    width: 100%;
    height: 44px;
    box-sizing: border-box;
    padding: 0 14px;
    border-radius: 12px;
    border: 1px solid var(--ln);
    background: var(--bg);
    color: var(--tx);
    font-family: var(--font-body);
    font-size: 15px;
  }
  .nameinput:focus {
    outline: none;
    border-color: var(--ac-text);
  }
  .nameinput[aria-invalid="true"] {
    border-color: var(--ac-text);
  }
  .error {
    margin: 0;
    color: var(--ac-text);
    font-size: 13px;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 8px;
  }
  @media (pointer: coarse) {
    .close {
      width: 44px;
      height: 44px;
    }
  }
  @media (max-width: 640px) {
    .overlay {
      align-items: flex-end;
      padding: 0;
    }
    .sheet {
      width: 100%;
      border-radius: 24px 24px 0 0;
      border-bottom: none;
      padding: 20px 18px calc(18px + env(safe-area-inset-bottom, 0px));
    }
    .close {
      width: 44px;
      height: 44px;
    }
  }
</style>
