<script>
  // Persistent, actionable notices — the things a toast is too fleeting for:
  // an expired Deezer credential, a downloaded update waiting to be applied, a
  // newer Android build. They sit above the content, stack quietly, and stay
  // until resolved or dismissed.
  //
  // Motion (F2): `rise` from −8 px — the strip hangs under the top bar, so a
  // notice comes DOWN into its place — and `flipSmall` when the stack closes
  // the gap a dismissed one leaves. The transform is on the notice, never on
  // `.notices`: the strip is `position: fixed`, and a transform on it would
  // hand it a new containing block and move the whole stack with the card.
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

<!-- The strip floats over the page (see the style below), so it is always
     rendered: an `{#if $notices.length}` around it would destroy the block the
     row's outro transition lives in, and dismissing the only notice removed it
     in one frame (measured: Element.prototype.animate was never called, the
     count went 1 → 0 between two frames). `class:has` is what tells the empty
     strip from a live one: without it the fixed box would claim taps on the
     page under it. -->
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
  /* FLOATING, not in flow. A notice arrives on its own (an expired Deezer
     credential, an update waiting), and the owner's rule is that what appears
     on its own never moves the rest: in flow it pushed every screen down by
     its own height the moment it landed (measured at 412x915, one notice:
     the playlists page's top went 200 px -> 80 px when it was dismissed, and
     0 px after this rule). It now hangs under the top bar over the content it
     concerns, exactly like the network pill, and stays reachable when the
     person is deep in a long list. The offsets live in App.svelte, which owns
     the shell's sidebar and page margins. */
  .notices {
    position: fixed;
    top: calc(var(--topbar-h, 68px) + 6px);
    left: var(--page-x, 24px);
    right: var(--page-x, 24px);
    z-index: 60;
    display: flex;
    flex-direction: column;
    gap: 8px;
    /* A notification card, not a full-width bar: on a wide screen an alert
       that spans a 1 100 px column reads as a page section and covers the
       title it sits over. The phone keeps the full width (no room to lose). */
    max-width: 560px;
    /* NO fill of its own: the strip is only the cards' column. An opaque band
       here painted a rectangle over the cover ground (a hard edge under the
       cards, measured at 390 with a purple sleeve) — the notices are the
       surfaces, and a notice is the one material the HIG wants thick, so it
       reads as an alert even while rows pass between two of them. */
    background: none;
  }
  /* Nothing to say: no box and no taps. NOT `display: none` — the store empties
     when the LAST notice is dismissed but its outro still has to play, and a
     hidden strip would cut it (that is the bug this component was written
     around). With no padding and no children the fixed box is 0 px tall. */
  .notices:not(.has) {
    pointer-events: none;
  }
  .notice {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 11px 12px 11px 14px;
    /* Tier 2 IN FULL — tint AND its blur. The strip FLOATS over the page now
       (see above), so the alert must own its material: the 78 % tint alone let
       the page's own title read through the card (seen at 390: the playlist's
       name under the alert). Blurring what is behind is what the ladder's
       floating tier is for, and eco/reduced transparency already flatten
       `--glass-bg-2` to the opaque `--s1`. */
    background: var(--glass-bg-2);
    backdrop-filter: var(--glass-filter-2);
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
