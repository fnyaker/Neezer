<script>
  // The home's seasonal door into the Récap. ONE target, no request of its
  // own: it paints from the `/api/recap/years` answer the home already fetched
  // for the session (lib/recapyears.js).
  //
  // No cover is asked for: the year list does not carry any, and spending a
  // year request (or four cover requests) just to decorate the home would
  // break the one-request budget. The 2×2 record-groove tile is Sillon's
  // deterministic fallback, drawn from the year itself — no network, and it
  // reads as a sleeve mosaic rather than an empty square.
  import Icon from "./Icon.svelte";
  import { formatNumber, formatMinutes } from "../lib/recap.js";
  import { grooveBackground } from "../lib/sillon.js";

  export let year = null;

  $: tiles = year && year.year ? [0, 1, 2, 3] : [];
</script>

{#if year && year.year}
  <a
    class="recap-home card"
    href={`#/recap/${year.year}`}
    aria-label={`Voir mon récap ${year.year}`}
  >
    <span class="art" aria-hidden="true">
      {#each tiles as i (i)}
        <span style={`background:${grooveBackground(`recap-${year.year}-${i}`)}`}></span>
      {/each}
    </span>
    <span class="txt">
      <span class="label-mono kicker">Récap</span>
      <span class="ttl">Ton année {year.year}{year.partial ? ", en cours" : ""}</span>
      <span class="stats">
        {formatNumber(year.listens)} écoute{Number(year.listens) > 1 ? "s" : ""} · {formatMinutes(year.minutes)}
      </span>
    </span>
    <span class="go">
      <span class="go-label">Voir mon récap</span>
      <Icon name="chevronRight" size={16} />
    </span>
  </a>
{/if}

<style>
  .recap-home {
    display: flex;
    align-items: center;
    gap: 14px;
    width: 100%;
    min-height: 88px;
    padding: 12px 14px 12px 12px;
    color: var(--tx);
    text-decoration: none;
    transition:
      background-color 0.14s ease,
      border-color 0.14s ease;
  }
  .recap-home:hover,
  .recap-home:focus-visible {
    background: var(--s2);
    border-color: var(--s2);
  }
  .art {
    flex: none;
    width: 62px;
    height: 62px;
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 3px;
    padding: 3px;
    border-radius: 14px;
    overflow: hidden;
    background: var(--s2);
  }
  .art span {
    display: block;
    min-width: 0;
    min-height: 0;
    border-radius: 4px;
  }
  .txt {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .kicker {
    color: var(--dim);
    font-size: 10px;
  }
  .ttl {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-display);
    font-size: 16.5px;
    font-weight: 700;
    letter-spacing: -0.015em;
    line-height: 1.2;
  }
  .stats {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--dim);
    font-size: 12.5px;
    line-height: 1.3;
  }
  .go {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--dim);
    font-size: 12.5px;
    font-weight: 600;
    white-space: nowrap;
  }
  .recap-home:hover .go,
  .recap-home:focus-visible .go {
    color: var(--tx);
  }

  @media (max-width: 420px) {
    /* The words tell the story; the chevron still says "this opens". */
    .go-label {
      display: none;
    }
    .recap-home {
      gap: 12px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .recap-home {
      transition: none;
    }
  }
</style>
