<script>
  // Transient confirmations ("done"), as a pill of --tx on --bg, bottom-centred
  // above the player (DESIGN §8). They say what happened and disappear; anything
  // the person must still act on belongs in `notices` (Notices.svelte), not here.
  //
  // A mutation that failed carries a "Réessayer" action (built by the action
  // helper that made the call), so the retry is one tap away instead of a
  // reopened sheet. `role="status"` + aria-live announce it politely.
  //
  // The motion is `lib/motion.js#rise` (F1): 8 px from the side it belongs to,
  // 200 ms in and 140 ms out, and never anything at all in eco mode — the
  // module's level decides, so this file has no matchMedia of its own. The
  // stack reorders through `flipSmall`, which is allowed here because a screen
  // never holds more than a handful of toasts (M4: ≤ 5).
  import { rise, flipSmall, MOVE } from "../lib/motion.js";
  import { toasts, playlistPicker } from "../lib/stores.js";
  import Icon from "./Icon.svelte";

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

  // Where the toast comes from: the side it sits on (`anchorTop` is the top of
  // the screen, so −8 px), and never a `duration: 0` branch of its own — the
  // level in lib/motion.js already gives eco the instant path.
  const slide = () => ({ y: anchorTop ? -MOVE.m : MOVE.m });

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
    <!-- The slot carries BOTH directives, and that is load-bearing: Svelte's
         local rule only plays an intro whose own block has already run, so a
         `transition:` on one of the two pills below (inside their `{#if}`) would
         never play at all — measured: `Element.prototype.animate` was never
         called. On the each item's own root (this div, the only child of the
         keyed block) the rise plays, and `flipSmall` still moves the slot when
         the stack closes a gap. The pill inside is unchanged and unstyled
         here. -->
    <div class="tslot" transition:rise={slide()} animate:flipSmall>
      {#if t.action}
        <div class="toast {t.kind} with-action">
          {#if t.kind === "error"}
            <span class="lead" aria-hidden="true"><Icon name="alert" size={14} /></span>
          {/if}
          <span class="msg">{t.message}</span>
          <button type="button" class="act" on:click={() => act(t)}>{t.action.label}</button>
        </div>
      {:else}
        <button type="button" class="toast {t.kind}" on:click={() => toasts.dismiss(t.id)}>
          {#if t.kind === "error"}
            <span class="lead" aria-hidden="true"><Icon name="alert" size={14} /></span>
          {/if}
          <span class="msg">{t.message}</span>
        </button>
      {/if}
    </div>
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
  /* The flip's box: no width, height, gap or margin of its own, so the stack
     lays out exactly as the pills did. */
  .tslot {
    display: flex;
    justify-content: center;
    max-width: 100%;
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
