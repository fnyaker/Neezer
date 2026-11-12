<script>
  // Transient confirmations ("done"), as a pill of --tx on --bg, bottom-centred
  // above the player (DESIGN §8). They say what happened and disappear; anything
  // the person must still act on belongs in `notices` (Notices.svelte), not here.
  //
  // A mutation that failed carries a "Réessayer" action (built by the action
  // helper that made the call), so the retry is one tap away instead of a
  // reopened sheet. `role="status"` + aria-live announce it politely.
  import { onMount } from "svelte";
  import { fly } from "svelte/transition";
  import { toasts, playlistPicker } from "../lib/stores.js";
  import Icon from "./Icon.svelte";

  let reduce = false;
  onMount(() => {
    if (typeof window === "undefined" || !window.matchMedia) return;
    const mq = window.matchMedia("(prefers-reduced-motion: reduce)");
    reduce = mq.matches;
    const onChange = () => (reduce = mq.matches);
    mq.addEventListener?.("change", onChange);
    return () => mq.removeEventListener?.("change", onChange);
  });

  // A toast born over the playlist sheet goes to the TOP: at the bottom it
  // landed on "Terminé", the very button the person presses next (seen at
  // 1440x900). Its anchor is recorded once and kept, so closing the sheet does
  // not make a visible toast jump across the screen.
  let anchored = {};
  $: pruneAnchors($toasts, $playlistPicker);
  $: anchorTop = $toasts.length > 0 && !!anchored[$toasts[$toasts.length - 1].id];

  function pruneAnchors(list, picker) {
    const ids = new Set(list.map((t) => t.id));
    for (const k of Object.keys(anchored)) if (!ids.has(Number(k))) delete anchored[k];
    for (const t of list) if (!(t.id in anchored)) anchored[t.id] = !!picker;
  }

  const slide = () => (reduce ? { y: 0, duration: 0 } : { y: 16, duration: 180 });

  function act(t) {
    toasts.dismiss(t.id);
    try {
      t.action.run();
    } catch {
      /* the toast is gone either way */
    }
  }
</script>

<div class="toasts" class:top={anchorTop} role="status" aria-live="polite">
  {#each $toasts as t (t.id)}
    {#if t.action}
      <div class="toast {t.kind} with-action" transition:fly={slide()}>
        {#if t.kind === "error"}
          <span class="lead" aria-hidden="true"><Icon name="alert" size={14} /></span>
        {/if}
        <span class="msg">{t.message}</span>
        <button type="button" class="act" on:click={() => act(t)}>{t.action.label}</button>
      </div>
    {:else}
      <button type="button" class="toast {t.kind}" transition:fly={slide()} on:click={() => toasts.dismiss(t.id)}>
        {#if t.kind === "error"}
          <span class="lead" aria-hidden="true"><Icon name="alert" size={14} /></span>
        {/if}
        <span class="msg">{t.message}</span>
      </button>
    {/if}
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    left: 50%;
    bottom: calc(var(--player-h, 88px) + 16px + env(safe-area-inset-bottom, 0px));
    transform: translateX(-50%);
    z-index: 500;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    width: max-content;
    max-width: min(92vw, 520px);
    pointer-events: none;
  }
  /* Anchored at the top: it must not cover the sheet's own bottom action. */
  .toasts.top {
    top: calc(env(safe-area-inset-top, 0px) + 16px);
    bottom: auto;
  }
  .toast {
    pointer-events: auto;
    display: flex;
    align-items: center;
    gap: 10px;
    max-width: 100%;
    padding: 12px 20px;
    border-radius: 999px;
    background: var(--tx);
    color: var(--bg);
    font-size: 14px;
    font-weight: 600;
    line-height: 1.3;
    text-align: left;
    box-shadow: 0 10px 30px color-mix(in srgb, var(--onac) 35%, transparent);
  }
  button.toast:hover {
    background: color-mix(in srgb, var(--tx) 92%, var(--bg));
  }
  .msg {
    min-width: 0;
    overflow-wrap: anywhere;
  }
  /* A failure is told by its glyph as much as by the words — never by a colour
     alone (§10). The circle is the pill's own ink, inverted. */
  .lead {
    flex: none;
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: 999px;
    background: var(--bg);
    color: var(--tx);
  }
  .with-action {
    padding: 6px 6px 6px 16px;
    gap: 12px;
  }
  .act {
    flex: none;
    min-height: 34px;
    padding: 0 14px;
    border-radius: 999px;
    background: var(--bg);
    color: var(--tx);
    font-weight: 700;
    font-size: 13px;
    white-space: nowrap;
  }
  .act:hover {
    background: color-mix(in srgb, var(--bg) 86%, var(--tx));
  }
  @media (pointer: coarse) {
    .act {
      min-height: 40px;
    }
  }
  /* Phone: the mini player (64, floating 8 above the tab bar) and the tab bar
     (64) below it — a toast clears both, plus the home indicator, so it never
     covers the transport or the destinations it is talking about. */
  @media (max-width: 640px) {
    .toasts {
      bottom: calc(var(--mini-h) + var(--tabbar-h) + var(--safe-bottom) + 16px);
      max-width: calc(100vw - 24px);
    }
    .toast {
      padding: 11px 18px;
    }
  }
</style>
