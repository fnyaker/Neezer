<script>
  // What a Flow is aimed at, in two decisions: the genres it rests on and how
  // far it may stray (Deezer's Flow wheel asks the same two questions; YouTube
  // Music's "Create a radio" asks for artists + Familiar→Discover). Shared by
  // the player's steering panel and, later, anything that starts a Flow: it
  // OWNS neither the dialog nor the launch — the caller wraps it and listens
  // for `launch` — so the same body can sit in a sheet, a page or a hero.
  //
  // The genres come from /flow/styles, already asked once per session by the
  // home (StylePicker.svelte#loadStyleRows): opening this panel must not cost
  // a second round-trip on a phone link. Nothing is sent to the server here —
  // the two choices are the device's (lib/flow.js), applied when the Flow
  // starts or is re-aimed.
  import { createEventDispatcher, onMount } from "svelte";
  import {
    startStyles,
    setStartStyles,
    STYLES_MAX,
    DIVERSITY_STEPS,
    diversityIndex,
    diversityStep,
  } from "../lib/flow.js";
  import { discoveryLevel, loadDiscovery, setDiscovery, DEFAULT_LEVEL } from "../lib/discovery.js";
  import { STYLE_FALLBACK } from "../lib/sillon.js";
  import { styleFamilies, loadStyleRows } from "./StylePicker.svelte";
  import Icon from "./Icon.svelte";

  // A button the caller drives while the Flow starts: the label stays, the
  // glyph spins, and a second press cannot start a second Flow.
  export let launching = false;

  const dispatch = createEventDispatcher();
  // Eight genres fit two short rows on a phone; the rest are one tap away
  // rather than behind a rail (M3: a chip set that does not fit REFLOWS, it
  // never scrolls sideways out of sight).
  const FIRST = 8;
  let rows = [];
  let expanded = false;

  $: families = styleFamilies(rows);
  // A fresh library (or an unreachable server) still gets a vocabulary: the
  // tiles on Explorer fall back to the same names, and both screens must offer
  // what they can rather than an empty section.
  $: allGenres = families.length ? families.map((f) => f.name) : STYLE_FALLBACK;
  $: shown = expanded ? allGenres : allGenres.slice(0, FIRST);
  $: chosen = $startStyles;
  $: full = chosen.length >= STYLES_MAX;
  $: index = diversityIndex($discoveryLevel);
  $: aim = diversityStep($discoveryLevel);
  // "Reset" is offered when there is something to undo — a disabled button that
  // never enables is furniture.
  $: dirty = chosen.length > 0 || index !== diversityIndex(DEFAULT_LEVEL);

  onMount(async () => {
    // The server's level, for a session that lands straight in the player (a
    // restored queue): the device copy paints first, the account's copy wins.
    loadDiscovery();
    try {
      rows = await loadStyleRows();
    } catch {
      /* the fallback list is the section */
    }
  });

  function toggle(name) {
    if (chosen.includes(name)) setStartStyles(chosen.filter((n) => n !== name));
    else if (!full) setStartStyles([...chosen, name]);
  }

  function reset() {
    setStartStyles([]);
    setDiscovery(DEFAULT_LEVEL);
  }
</script>

<div class="picker">
  <div class="sections">
    <section class="block" aria-labelledby="flow-genres-title">
      <div class="head">
        <h3 id="flow-genres-title">Genres</h3>
        <span class="sub">{chosen.length ? `${chosen.length} sur ${STYLES_MAX}` : "Tous les styles"}</span>
      </div>
      <div class="chips" role="group" aria-label="Genres du Flow">
        <button
          type="button"
          class="chip"
          class:chip-active={!chosen.length}
          aria-pressed={!chosen.length}
          on:click={() => setStartStyles([])}
        >
          {#if !chosen.length}<Icon name="check" size={13} />{/if}
          Tous
        </button>
        {#each shown as name (name)}
          <button
            type="button"
            class="chip"
            class:chip-active={chosen.includes(name)}
            aria-pressed={chosen.includes(name)}
            disabled={full && !chosen.includes(name)}
            title={full && !chosen.includes(name) ? `${STYLES_MAX} genres au maximum — retirez-en un` : name}
            on:click={() => toggle(name)}
          >
            {#if chosen.includes(name)}<Icon name="check" size={13} />{/if}
            {name}
          </button>
        {/each}
        {#if allGenres.length > FIRST}
          <button
            type="button"
            class="chip more"
            aria-expanded={expanded}
            on:click={() => (expanded = !expanded)}
          >
            {expanded ? "Moins de genres" : `Tous les genres (${allGenres.length})`}
          </button>
        {/if}
      </div>
      {#if full}<p class="note">Six genres au maximum : retirez-en un pour en ajouter un autre.</p>{/if}
    </section>

    <section class="block" aria-labelledby="flow-diversity-title">
      <div class="head">
        <h3 id="flow-diversity-title">Diversité</h3>
        <span class="sub">{aim.name}</span>
      </div>
      <div class="steps" role="radiogroup" aria-label="Diversité du Flow">
        {#each DIVERSITY_STEPS as s, i}
          <button
            type="button"
            role="radio"
            aria-checked={i === index}
            class:on={i === index}
            title={s.hint}
            on:click={() => setDiscovery(s.value)}
          >
            {s.name}
          </button>
        {/each}
      </div>
      <p class="note" aria-live="polite">{aim.hint}</p>
    </section>
  </div>

  <div class="actions">
    <button type="button" class="btn btn-ghost" on:click={reset} disabled={!dirty}>
      Réinitialiser
    </button>
    <button
      type="button"
      class="btn btn-primary go"
      on:click={() => dispatch("launch")}
      disabled={launching}
      aria-busy={launching}
    >
      <Icon name={launching ? "refresh" : "play"} size={15} />
      Lancer le Flow
    </button>
  </div>
</div>

<style>
  .picker {
    display: flex;
    flex-direction: column;
    gap: 22px;
    min-width: 0;
    container-type: inline-size;
  }
  /* Two decisions side by side once there is width for both: genres on the
     left (the list that grows), breadth on the right. Under it, one column —
     a phone never gets a squeezed pair of columns. */
  .sections {
    display: flex;
    flex-direction: column;
    gap: 22px;
    min-width: 0;
  }
  .block {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
  }
  .head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    min-width: 0;
  }
  h3 {
    margin: 0;
    font-family: var(--font-mono);
    font-size: 12px;
    font-weight: 500;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--dim);
  }
  .sub {
    flex: none;
    font-size: 13px;
    font-weight: 600;
    color: var(--tx);
    max-width: 60%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* Wraps, never scrolls sideways: a genre hidden past the edge is a genre
     nobody knows is selectable. Each chip is a 44 px target under a finger. */
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    min-width: 0;
  }
  .chips .chip {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .chip:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .chip.more {
    border-style: dashed;
    color: var(--dim);
  }
  .chip.more:hover {
    color: var(--tx);
  }
  .chip.sk {
    width: 74px;
    height: 36px;
    background: var(--s2);
    border-color: transparent;
  }
  .steps {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 8px;
  }
  .steps button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 0;
    height: 40px;
    padding: 0 8px;
    border-radius: 12px;
    border: 1px solid var(--ln);
    background: var(--s1);
    color: var(--tx);
    font-size: 13px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    transition:
      background-color 0.15s ease,
      color 0.15s ease,
      border-color 0.15s ease;
  }
  .steps button:hover {
    border-color: var(--dim);
  }
  /* A selection is an --ac surface with --onac ink, the app's pair: never an
     accent as text on it. */
  .steps button.on {
    background: var(--ac);
    border-color: transparent;
    color: var(--onac);
  }
  .note {
    margin: -2px 0 0;
    min-height: 1.2em;
    font-size: 13px;
    line-height: 1.35;
    color: var(--dim);
  }
  .actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    flex-wrap: wrap;
    gap: 10px;
    padding-top: 4px;
  }
  .go {
    min-width: 170px;
  }
  @media (min-width: 760px) {
    .go {
      margin-left: auto;
    }
  }
  @container (min-width: 620px) {
    .sections {
      display: grid;
      grid-template-columns: minmax(0, 1.35fr) minmax(250px, 1fr);
      gap: 24px;
      align-items: start;
    }
    .steps {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }
  @media (pointer: coarse) {
    .steps button {
      height: 44px;
    }
  }
  @media (max-width: 400px) {
    .steps button {
      font-size: 12px;
      padding: 0 6px;
    }
    .actions {
      justify-content: stretch;
    }
    .actions .btn {
      flex: 1 1 auto;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .steps button {
      transition: none;
    }
  }
</style>
