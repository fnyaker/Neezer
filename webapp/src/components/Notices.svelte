<script>
  // Persistent, actionable notices — the things a toast is too fleeting for:
  // an expired Deezer credential, a downloaded update waiting to be applied, a
  // newer Android build. They sit above the content, stack quietly, and stay
  // until resolved or dismissed.
  //
  // Motion (F2): `rise` from −8 px — the strip lives under the top bar, so a
  // notice comes DOWN into its place — and `flipSmall` when the stack closes
  // the gap a dismissed one leaves. The transform is on the notice, never on
  // `.notices`: the strip is `position: sticky`, and a transform on it would
  // both shift the whole page's notices and hand the sticky box a new
  // containing block.
  import { rise, flipSmall, MOVE } from "../lib/motion.js";
  import { notices } from "../lib/stores.js";
  import Icon from "./Icon.svelte";

  const ICONS = { info: "info", warn: "alert", error: "alert" };

  function run(n) {
    try {
      n.action && n.action();
    } finally {
      // An acted-on notice has said what it had to say.
      if (n.action) notices.dismiss(n.id);
    }
  }
</script>

<!-- The strip is always rendered, and that is what makes the LAST notice's
     outro visible: an `{#if $notices.length}` around it destroyed the block the
     row's transition lived in, so dismissing the only notice removed it in one
     frame (measured: Element.prototype.animate was never called, the count went
     1 → 0 between two frames). `class:has` takes the empty strip's padding away,
     so an app with no notice looks exactly as before. -->
<div class="notices" class:has={$notices.length > 0} role="status" aria-live="polite">
  {#each $notices as n (n.id)}
    <div class="notice {n.kind}" transition:rise={{ y: -MOVE.m }} animate:flipSmall>
      <span class="ico"><Icon name={ICONS[n.kind] || "info"} size={17} /></span>
      <span class="msg">{n.message}</span>
      {#if n.action}
        <button class="act" on:click={() => run(n)}>{n.actionLabel || "Ouvrir"}</button>
      {/if}
      {#if n.dismissible !== false}
        <button class="x" on:click={() => notices.dismiss(n.id)} aria-label="Masquer">
          <Icon name="close" size={16} />
        </button>
      {/if}
    </div>
  {/each}
</div>

<style>
  /* Sticky, so a notice stays reachable when you're deep in a long list — with
     an opaque backdrop, otherwise the content scrolling underneath shows
     through the gaps around the cards. */
  .notices {
    position: sticky;
    top: 0;
    z-index: 60;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 6px 0 10px;
    /* NO fill of its own: the strip is only the cards' column. An opaque band
       here painted a rectangle over the cover ground (a hard edge under the
       cards, measured at 390 with a purple sleeve) — the notices are the
       surfaces, and a notice is the one material the HIG wants thick, so it
       reads as an alert even while rows pass between two of them. */
    background: none;
  }
  /* Nothing to say: the strip takes no room at all (its own 16 px of padding
     would push every screen down by a hair). */
  .notices:not(.has) {
    padding: 0;
    gap: 0;
  }
  .notice {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 11px 12px 11px 14px;
    /* Tier 2 without the blur: an alert carries a lot of text and floats over a
       list, so it takes the popover's tint — the strip behind it is already the
       page's own ground. */
    background: var(--glass-bg-2);
    border: 1px solid var(--glass-border);
    border-radius: var(--r-card);
    box-shadow: var(--glass-shadow-2);
    font-size: 0.9rem;
    line-height: 1.35;
  }
  .ico {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    flex: none;
    border-radius: 8px;
    background: var(--bg-hover);
    color: var(--text-dim);
  }
  .notice.warn .ico,
  .notice.error .ico {
    background: color-mix(in srgb, var(--ac) 18%, transparent);
    color: var(--ac-text);
  }
  .msg {
    flex: 1;
    min-width: 0;
  }
  .act {
    flex: none;
    padding: 7px 14px;
    border-radius: 999px;
    font-size: 0.83rem;
    font-weight: 700;
    background: var(--ac);
    color: var(--onac);
    white-space: nowrap;
  }
  .act:hover {
    filter: brightness(1.08);
  }
  .x {
    flex: none;
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: 50%;
    color: var(--text-dim);
  }
  @media (pointer: coarse) {
    .x {
      width: 44px;
      height: 44px;
    }
    .act {
      min-height: 44px;
      padding: 10px 16px;
    }
  }
  .x:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  @media (max-width: 700px) {
    .notice {
      flex-wrap: wrap;
      row-gap: 8px;
    }
    .msg {
      flex-basis: calc(100% - 84px);
    }
    .act {
      margin-left: 40px;
    }
  }
</style>
