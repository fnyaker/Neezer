<script>
  // Explorer's way in when nothing is typed: the style families as coloured
  // tiles, four to a row. A tile's colour IS its name hashed onto Sillon's
  // eight grooves (lib/sillon.js, DESIGN §4), so "Hardcore" is the same tile on
  // every launch and every device — a palette that shuffled would read as a
  // loading state. The names come from the same rule as the home's chips
  // (styleFamilies in StylePicker.svelte), so both screens say the same thing.
  import { onMount, createEventDispatcher } from "svelte";
  import { grooveBackground, STYLE_FALLBACK } from "../lib/sillon.js";
  import { styleFamilies, loadStyleRows } from "./StylePicker.svelte";

  const TILES = 8;
  // DESIGN §6 names the styles Explorer offers. The live list comes from the
  // Flow's own vocabulary; on a fresh library, an unreachable server or a
  // response with no branch roots, this list is what keeps Explorer from being
  // an empty page — the tiles are a way in, not a report.
  const dispatch = createEventDispatcher();

  let rows = [];
  let loaded = false;
  $: families = styleFamilies(rows);
  $: tiles = (families.length ? families : loaded ? STYLE_FALLBACK.map((name) => ({ name })) : []).slice(0, TILES);

  onMount(async () => {
    try {
      // The home already asked for this list; asking again from Explorer would
      // be one round-trip per open on the link this is built for.
      rows = await loadStyleRows();
    } catch {
      /* the tiles are a way in, not the page: the search field still works */
    } finally {
      loaded = true;
    }
  });
</script>

{#if tiles.length}
  <section class="genres">
    <h2 class="section-title">Parcourir par style</h2>
    <div class="tiles">
      {#each tiles as g (g.name)}
        <button
          class="tile"
          style={`--art:${grooveBackground(g.name)}`}
          title={`Chercher ${g.name}`}
          on:click={() => dispatch("pick", g.name)}
        >
          <span class="art" aria-hidden="true"></span>
          <span class="cap">
            <span class="name">{g.name}</span>
          </span>
        </button>
      {/each}
    </div>
  </section>
{:else if !loaded}
  <section class="genres" aria-hidden="true">
    <h2 class="section-title">Parcourir par style</h2>
    <div class="tiles">
      {#each Array(TILES) as _}<span class="tile sk"></span>{/each}
    </div>
  </section>
{/if}

<style>
  .genres {
    container-type: inline-size;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .tiles {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 14px;
  }
  .tile {
    position: relative;
    display: flex;
    align-items: flex-end;
    height: 124px;
    padding: 0;
    border-radius: 18px;
    border: 1px solid var(--ln);
    overflow: hidden;
    background: var(--s1);
    color: var(--tx);
    text-align: left;
    content-visibility: auto;
    contain-intrinsic-size: auto 124px;
  }
  .art {
    position: absolute;
    inset: 0 0 40px 0;
    background-image: var(--art);
  }
  .cap {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
    padding: 10px 14px 12px;
    background: var(--s1);
  }
  .name {
    font-family: var(--font-display);
    font-size: 22px;
    font-weight: 700;
    letter-spacing: -0.02em;
    line-height: 1.05;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tile:hover {
    border-color: var(--s2);
  }
  .tile:hover .cap {
    background: var(--s2);
  }
  .sk {
    display: block;
    background: var(--s2);
  }
  @container (max-width: 720px) {
    .tiles {
      grid-template-columns: repeat(3, minmax(0, 1fr));
    }
  }
  @container (max-width: 480px) {
    .tiles {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
    .name {
      font-size: 19px;
    }
  }
</style>
