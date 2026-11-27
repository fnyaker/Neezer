<script>
  // The Flow's starting energy, in notches (Deezer's Flow moods are chosen the
  // same way, before the Flow starts: arXiv 2207.11229). One tap; the choice
  // is the person's, kept by the server and on the device (lib/flow.js).
  //
  // Drawn as Sillon chips ON the accent hero (DESIGN §6): an unselected notch
  // is an outline in --onac, the chosen one is filled with --onac and set in
  // --ac. That is the one colour pair the themes guarantee readable — a
  // --chip-active accent fill would vanish on the accent panel.
  import { ENERGY_STEPS, startEnergy, setStartEnergy } from "../lib/flow.js";
  import Icon from "./Icon.svelte";
</script>

<div class="energy" role="radiogroup" aria-label="Énergie du Flow">
  {#each ENERGY_STEPS as s}
    <button
      type="button"
      role="radio"
      aria-checked={$startEnergy === s.value}
      class:on={$startEnergy === s.value}
      on:click={() => setStartEnergy(s.value)}
    >
      {#if $startEnergy === s.value}<Icon name="check" size={13} />{/if}
      {s.name}
    </button>
  {/each}
</div>

<style>
  /* All six are a decision, so all six are visible: a rail hid the loud half
     behind a cut edge (a phone showed three and looked complete). Compact on
     the narrowest screens so the six still take two rows, not three. */
  .energy {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    min-width: 0;
    padding: 2px;
    margin: -2px;
  }
  @media (max-width: 460px) {
    /* Six equal cells, two rows: the row-by-row wrap left one chip alone on a
       third line and pushed the Flow button under the fold. */
    .energy {
      display: grid;
      grid-template-columns: repeat(3, minmax(0, 1fr));
    }
    button {
      justify-content: center;
      height: 40px;
      padding: 0 8px;
      font-size: 12.5px;
      gap: 5px;
    }
  }
  button {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    flex: none;
    height: 38px;
    padding: 0 16px;
    border-radius: 999px;
    border: 1.5px solid var(--onac);
    background: none;
    color: var(--onac);
    font-size: 14px;
    font-weight: 600;
    white-space: nowrap;
    /* A selection is colour only (M1/M6): the notch repaints, nothing moves. */
    transition:
      background-color var(--dur-instant) var(--ease-standard),
      color var(--dur-instant) var(--ease-standard);
  }
  button:hover {
    background: color-mix(in srgb, var(--onac) 12%, transparent);
  }
  button.on {
    background: var(--onac);
    color: var(--ac);
  }
  button.on:hover {
    background: var(--onac);
  }
  /* The global ring is --ac, which is the surface this row sits on: on the
     hero the ring has to be the ink colour or it is invisible. */
  button:focus-visible {
    outline-color: var(--onac);
    outline-offset: 2px;
  }
  @media (pointer: coarse) {
    button {
      height: 44px;
    }
  }
  /* app.css already neutralises .btn/.chip/.card/.pill under reduced motion:
     a change of colour is feedback, not a fade, and the OS asked for stillness
     (M14 — reduce, keep the meaning: the colour still changes, at once). */
  @media (prefers-reduced-motion: reduce) {
    button {
      transition: none;
    }
  }
</style>
