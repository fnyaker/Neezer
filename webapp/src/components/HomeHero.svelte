<script>
  // The home's opening: the day's greeting, one question ("Quelle énergie ce
  // matin ?"), the six Flow notches in one tap, and the person's own listening
  // beside it (DESIGN §6). Full accent because it is the one decision the home
  // asks for; everything below is what that decision feeds.
  import { user, toasts } from "../lib/stores.js";
  import { startFlow } from "../lib/flow.js";
  import { discoveryLevel, stepOf } from "../lib/discovery.js";
  import Icon from "./Icon.svelte";
  import EnergyPicker from "./EnergyPicker.svelte";
  import ResumeCard from "./ResumeCard.svelte";
  import DiscoverySlider from "./DiscoverySlider.svelte";

  // The prototype's copy, kept word for word: it is the home's masthead, not a
  // clock. The date under it is the real one.
  $: weekday = new Date().toLocaleDateString("fr-FR", { weekday: "long" });
  $: who = $user?.name || "";
  $: hello = `Bonjour${who ? `, ${who}` : ""} · ${weekday}`;
  // The discovery level is set once in a while, not on every visit: one chip
  // that opens it (progressive disclosure, NN/g), beside the Flow it steers.
  let tuneOpen = false;
  $: step = stepOf($discoveryLevel);

  let flowLoading = false;
  async function playFlow() {
    if (flowLoading) return;
    flowLoading = true;
    try {
      await startFlow();
    } catch {
      toasts.push("Le Flow ne répond pas, réessayez", "error");
    } finally {
      flowLoading = false;
    }
  }
</script>

<div class="hero-wrap">
  <div class="hero-row">
    <section class="hero" aria-labelledby="hero-title">
      <svg class="rings" viewBox="0 0 360 360" aria-hidden="true">
        <g fill="none" stroke="currentColor" stroke-width="2">
          <circle cx="180" cy="180" r="170" />
          <circle cx="180" cy="180" r="140" />
          <circle cx="180" cy="180" r="110" />
          <circle cx="180" cy="180" r="80" />
          <circle cx="180" cy="180" r="50" />
        </g>
      </svg>
      <p class="hello">{hello}</p>
      <h1 id="hero-title">Quelle énergie ce matin&nbsp;?</h1>
      <EnergyPicker />
      <div class="go">
        <button class="flow" on:click={playFlow} disabled={flowLoading}>
          {#if flowLoading}
            <Icon name="refresh" size={15} />
          {:else}
            <Icon name="play" size={15} />
          {/if}
          Lancer le Flow
        </button>
        <button
          class="tune"
          aria-expanded={tuneOpen}
          on:click={() => (tuneOpen = !tuneOpen)}
          title="Régler la découverte du Flow et des suggestions"
        >
          <Icon name="sliders" size={13} /> {step.name}
        </button>
        <span class="note">Le Flow suit ton écoute, au niveau choisi.</span>
      </div>
    </section>
    <ResumeCard />
  </div>
  {#if tuneOpen}
    <div class="tune-panel">
      <DiscoverySlider />
    </div>
  {/if}
</div>

<style>
  .hero-wrap {
    container-type: inline-size;
  }
  .hero-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 16px;
    align-items: stretch;
  }
  .hero {
    position: relative;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    gap: 20px;
    min-width: 0;
    padding: 32px;
    border-radius: 24px;
    background: var(--ac);
    color: var(--onac);
  }
  .rings {
    position: absolute;
    right: -90px;
    top: -70px;
    width: 360px;
    height: 360px;
    opacity: 0.22;
    pointer-events: none;
  }
  .hello {
    position: relative;
    margin: 0;
    font-family: var(--font-mono);
    font-size: 12px;
    font-weight: 500;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  h1 {
    position: relative;
    max-width: 520px;
    margin: 0;
    font-family: var(--font-display);
    font-size: 44px;
    font-weight: 700;
    line-height: 1;
    letter-spacing: -0.03em;
  }
  .go {
    position: relative;
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  .flow {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: 48px;
    padding: 0 24px;
    border-radius: 24px;
    background: var(--onac);
    /* --onac is the ink in every theme; the colour that reads on it is the
       accent surface, not --ac-text (which is the same ink on Papier). */
    color: var(--ac);
    font-size: 15px;
    font-weight: 700;
    white-space: nowrap;
  }
  .flow:hover:not(:disabled) {
    background: color-mix(in srgb, var(--onac) 88%, var(--ac));
  }
  .flow:disabled {
    opacity: 0.7;
  }
  .note {
    font-size: 14px;
    line-height: 1.35;
  }
  /* The discovery level, on the hero's accent: the same outline language as
     the energy chips (--ac on --ac is invisible). */
  .tune {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 34px;
    padding: 0 13px;
    border-radius: 999px;
    border: 1px solid var(--onac);
    background: none;
    color: var(--onac);
    font-size: 12.5px;
    font-weight: 600;
    white-space: nowrap;
  }
  .tune:hover {
    background: color-mix(in srgb, var(--onac) 12%, transparent);
  }
  .tune[aria-expanded="true"] {
    background: var(--onac);
    color: var(--ac);
  }
  .tune:focus-visible {
    outline-color: var(--onac);
  }
  .tune-panel {
    display: flex;
    margin-top: 14px;
  }
  .tune-panel :global(.disc) {
    max-width: 420px;
  }
  @media (pointer: coarse) {
    .tune {
      height: 44px;
    }
  }
  /* The global ring is the accent, which is this panel's own surface. */
  .flow:focus-visible {
    outline-color: var(--onac);
  }
  @container (min-width: 860px) {
    .hero-row {
      grid-template-columns: minmax(0, 1.9fr) minmax(280px, 1fr);
    }
    h1 {
      font-size: 56px;
    }
  }
  @media (max-width: 640px) {
    .hero {
      padding: 24px 20px;
      gap: 16px;
    }
    h1 {
      font-size: 34px;
    }
    .note {
      font-size: 13px;
    }
  }
</style>
