<script context="module">
  import { api } from "../lib/api.js";

  // What counts as a "style family": the widest roots, with Electronics split
  // into its children (it is too wide to be one chip). One rule, read by the
  // home's chips and by Explorer's tiles, so both screens name the same styles.
  // The rows come from /flow/styles (GET).
  export function styleFamilies(rows) {
    const byParent = {};
    for (const r of rows || []) (byParent[r.parent || ""] ||= []).push(r);
    return [
      ...(byParent[""] || []).filter((r) => r.name !== "Électronique"),
      ...(byParent["Électronique"] || []),
    ]
      .filter((r) => r.branch > 0)
      .sort((a, b) => b.branch - a.branch)
      .slice(0, 16);
  }

  // The list itself, asked ONCE per session and shared: the home's chips and
  // Explorer's tiles mount on different screens, and a second screen must not
  // cost a second request on the link this app is built for. A failed load is
  // forgotten so the next visit tries again. The list moves slowly; a reload
  // is what refreshes it.
  let rowsPromise = null;
  export function loadStyleRows() {
    if (!rowsPromise) {
      rowsPromise = api.swr("/flow/styles", () => {}).then(
        (d) => d.styles || [],
        () => {
          rowsPromise = null;
          return [];
        }
      );
    }
    return rowsPromise;
  }
</script>

<script>
  // The style the Flow plays around: one family, several, or none ("Tout" = the
  // whole taste). Picking a family means ALL of it; its sub-genres open under the
  // row for a finer choice. The Flow then rests on what this person likes inside
  // the style, and the discovery slider decides how far from it the rest strays
  // (neezer/discover.py#style_taste). Kept on the device, like the energy notch.
  import { onMount } from "svelte";
  import { startStyles, setStartStyles, STYLES_MAX } from "../lib/flow.js";
  import Icon from "./Icon.svelte";

  let rows = [];
  let open = null; // the family whose sub-genres show
  let failed = false;

  $: byParent = rows.reduce((m, r) => ((m[r.parent || ""] ||= []).push(r), m), {});
  $: families = styleFamilies(rows);
  $: visibleFamilies = families.length ? families : STYLE_FALLBACK.map((name) => ({ name }));
  $: subs = open ? flat(open) : [];

  function flat(name, depth = 0) {
    const out = [];
    for (const r of byParent[name] || []) {
      if (r.branch > 0) {
        out.push(r);
        if (depth < 2) out.push(...flat(r.name, depth + 1));
      }
    }
    return out;
  }

  function toggle(name) {
    const cur = $startStyles;
    setStartStyles(cur.includes(name) ? cur.filter((n) => n !== name) : [...cur, name].slice(-STYLES_MAX));
  }

  // One tap on a chip IS the choice (that is what a filter chip promises):
  // selecting a family also opens its sub-genres to refine it. A second tap
  // removes it, as the old row did for its sub-chips.
  function pick(name) {
    if ($startStyles.includes(name)) {
      toggle(name);
      if (open === name) open = null;
    } else {
      toggle(name);
      open = name;
    }
  }

  onMount(async () => {
    try {
      rows = await loadStyleRows();
    } catch {
      failed = true;
    }
  });
</script>

{#if visibleFamilies.length}
  <section class="style-block fade-in">
    <div class="section-head">
      <h2 class="section-title">Par style</h2>
      <span class="hint">filtre Nouveautés et Pour toi</span>
    </div>
    <div class="chips" role="group" aria-label="Style du Flow">
      <button
        type="button"
        class="chip"
        class:chip-active={!$startStyles.length}
        aria-pressed={!$startStyles.length}
        on:click={() => setStartStyles([])}
      >
        {#if !$startStyles.length}<Icon name="check" size={13} />{/if}
        Tout
      </button>
      {#each visibleFamilies as f (f.name)}
        <button
          type="button"
          class="chip"
          class:chip-active={$startStyles.includes(f.name)}
          class:open={open === f.name}
          aria-pressed={$startStyles.includes(f.name)}
          aria-expanded={open === f.name}
          on:click={() => pick(f.name)}
        >
          {#if $startStyles.includes(f.name)}<Icon name="check" size={13} />{/if}
          {f.name}
        </button>
      {/each}
    </div>
    {#if open}
      <div class="subs">
        <button
          type="button"
          class="chip sub all"
          class:chip-active={$startStyles.includes(open)}
          on:click={() => toggle(open)}
        >
          <Icon name={$startStyles.includes(open) ? "check" : "play"} size={13} /> Tout {open}
        </button>
        {#each subs as g (g.name)}
          <button
            type="button"
            class="chip sub"
            class:chip-active={$startStyles.includes(g.name)}
            on:click={() => toggle(g.name)}
          >
            {g.name}
          </button>
        {/each}
      </div>
    {/if}
    {#if $startStyles.length}
      <p class="chosen">
        <span class="chosen-label">Flow autour de</span>
        {#each $startStyles as s}
          <button type="button" class="tag" on:click={() => pick(s)} title="Retirer {s}">
            {s} <Icon name="close" size={11} />
          </button>
        {/each}
      </p>
    {/if}
  </section>
{/if}

<style>
  .style-block {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
  }
  .hint {
    color: var(--dim);
    font-size: 13px;
  }
  /* A phone: one scrolling row, the standard chip rail of the app. A wide
     window gets the prototype's wrap: the whole vocabulary is worth seeing. */
  .chips {
    display: flex;
    gap: 8px;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
    padding: 3px;
    margin: -3px;
  }
  .chips::-webkit-scrollbar {
    display: none;
  }
  @media (min-width: 900px) {
    .chips {
      flex-wrap: wrap;
      overflow: visible;
    }
  }
  .chip.open:not(.chip-active) {
    border-color: var(--ac-text);
  }
  .subs {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    padding: 12px;
    border: 1px solid var(--ln);
    border-radius: 16px;
    background: var(--s1);
  }
  .sub {
    height: 36px;
    font-size: 12.5px;
    font-weight: 500;
  }
  .sub.all {
    font-weight: 700;
  }
  .chosen {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    margin: 0;
    font-size: 13px;
    color: var(--dim);
  }
  .chosen-label {
    margin-right: 2px;
  }
  .tag {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 28px;
    padding: 0 10px;
    border-radius: 999px;
    border: 1px solid var(--ln);
    background: var(--s2);
    color: var(--tx);
    font-size: 12.5px;
    font-weight: 500;
  }
  .tag:hover {
    border-color: var(--dim);
  }
</style>
