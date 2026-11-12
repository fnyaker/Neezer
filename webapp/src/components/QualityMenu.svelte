<script>
  // The streaming quality, as one clickable mono badge and its drop-up list.
  // Sillon §7: the badge REPLACES the old segmented FLAC/320/128/64 strip and
  // the separate quality button — it names the setting, opens the six choices
  // with their plain-French hint, and sits wherever the player bar does.
  //
  // No colour of its own: the badge is the accent's outline, the menu is the
  // raised surface (--s2), so all three themes repaint without touching it.
  import { quality } from "../lib/stores.js";
  import Icon from "./Icon.svelte";
  import SourceBadge from "./SourceBadge.svelte";

  // Which edge the list hangs from, and on which side: a badge in a footer opens
  // upwards, one in a header opens downwards; at the left of a row it opens
  // rightwards.
  export let placement = "up"; // up | down
  export let align = "right"; // right | left

  const QUALITIES = ["FLAC", "OPUS_320", "OPUS_256", "OPUS_192", "OPUS_128", "OPUS_64"];
  const LABEL = {
    FLAC: "FLAC",
    OPUS_320: "Opus 320",
    OPUS_256: "Opus 256",
    OPUS_192: "Opus 192",
    OPUS_128: "Opus 128",
    OPUS_64: "Opus 64",
  };
  const SHORT = { FLAC: "FLAC", OPUS_320: "320", OPUS_256: "256", OPUS_192: "192", OPUS_128: "128", OPUS_64: "64" };
  const HINT = {
    FLAC: "Sans perte",
    OPUS_320: "Haute qualité",
    OPUS_256: "Haute qualité",
    OPUS_192: "Bon compromis",
    OPUS_128: "Standard",
    OPUS_64: "Données réduites",
  };

  let open = false;
  function pick(q) {
    quality.set(q);
    open = false;
  }
</script>

<svelte:window on:click={() => (open = false)} on:keydown={(e) => e.key === "Escape" && (open = false)} />

<div class="qm">
  <SourceBadge />
  <div class="wrap">
    <button
      class="trigger"
      class:open
      on:click|stopPropagation={() => (open = !open)}
      aria-haspopup="listbox"
      aria-expanded={open}
      aria-label="Qualité de streaming : {LABEL[$quality]}"
      title="Qualité de streaming"
    >
      <span class="val">{SHORT[$quality]}</span>
      <Icon name="chevronDown" size={11} />
    </button>
    {#if open}
      <ul class="menu {placement} {align}" role="listbox" aria-label="Qualité de streaming">
        {#each QUALITIES as q}
          <li>
            <button role="option" aria-selected={$quality === q} class:sel={$quality === q} on:click|stopPropagation={() => pick(q)}>
              <span class="ql">
                <span class="lab">{LABEL[q]}</span>
                <span class="hint mono">{HINT[q]}</span>
              </span>
              {#if $quality === q}<Icon name="check" size={15} />{/if}
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</div>

<style>
  .qm {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .wrap {
    position: relative;
    display: flex;
  }
  .trigger {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 26px;
    padding: 0 8px;
    border: 1px solid var(--ac-text);
    border-radius: 6px;
    background: none;
    color: var(--tx);
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.06em;
    line-height: 1;
    white-space: nowrap;
  }
  .trigger:hover {
    background: color-mix(in srgb, var(--ac) 14%, transparent);
  }
  .trigger:focus-visible {
    outline: 2px solid var(--ac-text);
    outline-offset: 2px;
  }
  .trigger :global(svg) {
    transition: transform 0.15s ease;
  }
  .trigger.open :global(svg) {
    transform: rotate(180deg);
  }
  .menu {
    position: absolute;
    min-width: 208px;
    list-style: none;
    margin: 0;
    padding: 6px;
    background: var(--s2);
    border: 1px solid var(--ln);
    border-radius: 16px;
    box-shadow: 0 16px 44px color-mix(in srgb, var(--bg) 65%, transparent);
    z-index: 40;
  }
  .menu.up {
    bottom: calc(100% + 8px);
  }
  .menu.down {
    top: calc(100% + 8px);
  }
  .menu.right {
    right: 0;
  }
  .menu.left {
    left: 0;
  }
  .menu button {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    width: 100%;
    padding: 8px 10px;
    border-radius: 11px;
    color: var(--tx);
    text-align: left;
  }
  .menu button:hover {
    background: var(--s1);
  }
  .menu button.sel {
    color: var(--ac-text);
  }
  .ql {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .lab {
    font-size: 0.86rem;
    font-weight: 600;
  }
  .hint {
    font-size: 11px;
    color: var(--dim);
  }
  .menu button.sel .hint {
    color: inherit;
    opacity: 0.75;
  }

  /* §10: 44 px under a finger. The badge keeps its small mono shape, its
     box grows. */
  @media (pointer: coarse) {
    .trigger {
      min-height: 34px;
      padding: 0 10px;
    }
    .menu {
      min-width: 224px;
      padding: 8px;
    }
    .menu button {
      padding: 12px 12px;
      border-radius: 12px;
    }
  }
</style>
