<script>
  // While the Flow plays: steer it. "Moins / plus comme ça" (Pandora's thumbs,
  // for this session only — the global "Je n'aime pas" is beside the heart),
  // the energy one notch down or up, and ONE door onto what the Flow aims at:
  // its genres and its diversity (Deezer's Flow Tuner keeps both in the player
  // for the same reason — tuning should not mean leaving what is playing).
  //
  // The door's label carries the current aim, so nobody opens a panel to find
  // out what it would do; the panel itself is FlowPicker.svelte. It hangs off
  // <body> (a fixed overlay inside the player, which has a transform, would be
  // fixed to the player column, not the screen) and closes on Escape, on the
  // veil, and on a successful launch.
  import { tick } from "svelte";
  import {
    flowLive,
    flowSession,
    energyName,
    moreLikeThis,
    lessLikeThis,
    nudgeEnergy,
    startFlow,
    flowAimSummary,
    flowAimShort,
    startStyles,
  } from "../lib/flow.js";
  import { discoveryLevel } from "../lib/discovery.js";
  import { toasts } from "../lib/stores.js";
  import Icon from "./Icon.svelte";
  import FlowPicker from "./FlowPicker.svelte";

  let panelOpen = false;
  let launching = false;
  let panelEl = null;
  let openBtn = null;
  let returnFocus = null;

  $: aim = $flowSession?.energy ?? null;
  $: low = aim != null && aim <= 0.1;
  $: high = aim != null && aim >= 0.9;
  $: summary = flowAimSummary($startStyles, $discoveryLevel);
  // Two spans: the genre names give way first, the breadth never does.
  $: parts = flowAimShort($startStyles, $discoveryLevel).split(" · ");
  $: selectedGenres = $startStyles.length;

  async function openPanel() {
    if (panelOpen) return;
    returnFocus = typeof document !== "undefined" ? document.activeElement : null;
    panelOpen = true;
    await tick();
    panelEl?.focus?.({ preventScroll: true });
  }

  function closePanel() {
    if (!panelOpen) return;
    panelOpen = false;
    const back = returnFocus || openBtn;
    returnFocus = null;
    // After the panel is gone, so the browser does not refuse the focus.
    tick().then(() => {
      if (back && back.isConnected) back.focus?.({ preventScroll: true });
    });
  }

  // Escape closes; Tab stays inside the panel (a short cycle, enough for a
  // panel of chips, a six-notch choice and two buttons).
  function onKey(e) {
    if (!panelOpen) return;
    if (e.key === "Escape") {
      // The full-screen player has its own Escape handler on the window; the
      // panel is on top, so it takes the key first and stops it there — one
      // Escape closes the sheet, not the player behind it (capture listener).
      e.preventDefault();
      e.stopImmediatePropagation();
      closePanel();
      return;
    }
    if (e.key !== "Tab" || !panelEl) return;
    const f = [...panelEl.querySelectorAll("button, [href], input, [tabindex]:not([tabindex='-1'])")].filter(
      (el) => !el.disabled && el.offsetParent !== null
    );
    if (!f.length) return;
    const first = f[0];
    const last = f[f.length - 1];
    const active = document.activeElement;
    if (e.shiftKey && (active === first || active === panelEl || !panelEl.contains(active))) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && active === last) {
      e.preventDefault();
      first.focus();
    }
  }

  async function launch() {
    if (launching) return;
    launching = true;
    try {
      // Keep the panel open until the answer: a Flow that finds nothing says so
      // in a toast, and closing first would hide where the settings went.
      const ok = await startFlow();
      if (ok !== false) closePanel();
    } catch {
      toasts.push("Le Flow ne répond pas, réessayez", "error");
    } finally {
      launching = false;
    }
  }

  // A fixed overlay inside anything that has a transform is fixed to THAT, not
  // to the screen: hang the veil on <body> (the player column is one).
  function toBody(node) {
    document.body.appendChild(node);
    return {
      destroy() {
        node.parentNode?.removeChild(node);
      },
    };
  }
</script>

<svelte:window on:keydown|capture={onKey} />

{#if $flowLive}
  <div class="flowctl" aria-label="Orienter le Flow" role="group">
    <div class="row">
      <button
        type="button"
        class="open"
        bind:this={openBtn}
        aria-haspopup="dialog"
        aria-expanded={panelOpen}
        aria-label={`Genres et diversité du Flow — ${summary}`}
        title="Genres et diversité du Flow"
        on:click={openPanel}
      >
        <Icon name="sliders" size={15} />
        <span class="txt">
          <span class="lbl">Genres<span class="amp">&nbsp;&amp; diversité</span></span>
          <span class="sum"><span class="sum-g">{parts[0]}</span><span class="sum-d">· {parts[1]}</span></span>
        </span>
        {#if selectedGenres}
          <span class="badge mono" aria-hidden="true">{selectedGenres}</span>
        {/if}
        <Icon name="chevronDown" size={14} />
      </button>

      <button
        type="button"
        class="steer"
        on:click={lessLikeThis}
        title="Passer et proposer moins de titres comme celui-ci"
        aria-label="Moins comme ça"
      >
        <Icon name="minus" size={16} /><span class="st">Moins comme ça</span><span class="st-short">Moins</span>
      </button>
      <button
        type="button"
        class="steer"
        on:click={moreLikeThis}
        title="Proposer plus de titres comme celui-ci"
        aria-label="Plus comme ça"
      >
        <Icon name="plus" size={16} /><span class="st">Plus comme ça</span><span class="st-short">Plus</span>
      </button>

      <div class="energy" aria-live="polite">
        <button type="button" class="step" on:click={() => nudgeEnergy(-1)} disabled={low} aria-label="Plus calme" title="Plus calme">
          <Icon name="chevronDown" size={18} />
        </button>
        <span class="lvl">
          <span class="k">Énergie</span>
          <span class="v">{energyName(aim)}</span>
        </span>
        <button type="button" class="step" on:click={() => nudgeEnergy(1)} disabled={high} aria-label="Plus énergique" title="Plus énergique">
          <Icon name="chevronUp" size={18} />
        </button>
      </div>
    </div>
  </div>
{/if}

{#if panelOpen}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div class="veil" use:toBody on:click={(e) => e.target === e.currentTarget && closePanel()}>
    <div
      class="panel"
      role="dialog"
      aria-modal="true"
      aria-labelledby="flow-panel-title"
      tabindex="-1"
      bind:this={panelEl}
    >
      <header>
        <div class="ttl">
          <h2 id="flow-panel-title">Orienter le Flow</h2>
          <p>Les genres sur lesquels il s'appuie, et jusqu'où il peut s'en écarter.</p>
        </div>
        <button type="button" class="x" on:click={closePanel} aria-label="Fermer">
          <Icon name="close" size={18} />
        </button>
      </header>
      <div class="body">
        <FlowPicker {launching} on:launch={launch} />
      </div>
    </div>
  </div>
{/if}

<style>
  /* The row answers to its OWN width, not the window's: the desktop player's
     side column is 520 px at 1440 and ~360 px at 700, and the phone's is 358.
     A viewport query cannot see that difference — the fullscreen bench sweeps
     both — so the compact arrangement is a container query. */
  .flowctl {
    container-type: inline-size;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: center;
    gap: 8px 10px;
  }
  .open,
  .steer,
  .step {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    min-height: 40px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--tx) 8%, transparent);
    color: var(--tx);
    font-weight: 600;
    font-size: 0.84rem;
    white-space: nowrap;
    transition: background 0.15s, color 0.15s, transform 0.1s;
  }
  /* The door: a chip, not a gear — it says what the Flow aims at. */
  .open {
    flex: 1 1 150px;
    min-width: 0;
    justify-content: flex-start;
    gap: 8px;
    height: 44px;
    padding: 0 12px 0 14px;
    text-align: left;
  }
  .open .txt {
    display: flex;
    flex-direction: column;
    gap: 0;
    min-width: 0;
    line-height: 1.12;
  }
  .open .lbl {
    font-size: 0.78rem;
    font-weight: 700;
  }
  .open .sum {
    display: flex;
    gap: 4px;
    min-width: 0;
    max-width: 100%;
    font-size: 0.68rem;
    font-weight: 500;
    color: var(--dim);
  }
  .open .sum-g {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .open .sum-d {
    flex: none;
    white-space: nowrap;
  }
  .open .badge {
    flex: none;
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    border-radius: 999px;
    display: grid;
    place-items: center;
    background: var(--ac);
    color: var(--onac);
    font-size: 10px;
    font-weight: 600;
  }
  .open:hover {
    background: color-mix(in srgb, var(--tx) 16%, transparent);
  }
  .open[aria-expanded="true"] {
    background: var(--ac);
    color: var(--onac);
  }
  .open[aria-expanded="true"] .sum {
    color: color-mix(in srgb, var(--onac) 80%, transparent);
  }
  .open[aria-expanded="true"] .badge {
    background: var(--onac);
    color: var(--ac);
  }
  .steer {
    padding: 0 15px;
    height: 40px;
  }
  /* Under a finger the two feedback buttons lose their sentence (the door
     needs the line): the glyph and a one-word label keep the action readable
     without a 44 px-tall tooltip nobody sees on a phone. */
  .st-short {
    display: none;
  }
  .step {
    width: 40px;
    padding: 0;
    flex: none;
  }
  .steer:hover,
  .step:hover:not(:disabled) {
    background: color-mix(in srgb, var(--tx) 16%, transparent);
    color: var(--tx);
  }
  .steer:active,
  .step:active:not(:disabled) {
    transform: scale(0.96);
  }
  .step:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .energy {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .lvl {
    display: inline-flex;
    flex-direction: column;
    align-items: center;
    min-width: 76px;
    line-height: 1.15;
  }
  .k {
    font-size: 0.66rem;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--dim);
    font-weight: 600;
  }
  .v {
    font-size: 0.86rem;
    font-weight: 700;
    color: var(--tx);
  }

  /* The panel: a wide card on a desktop, a sheet that rises from the thumb's
     edge on a phone (M3 modal bottom sheet / iOS sheet). It is the same body
     either way — one column is only what the width allows. */
  .veil {
    position: fixed;
    inset: 0;
    z-index: 400;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 20px;
    background: color-mix(in srgb, var(--onac) 60%, transparent);
    overscroll-behavior: contain;
    touch-action: none;
  }
  .panel {
    width: min(760px, 100%);
    min-width: 0;
    max-width: 100%;
    max-height: min(760px, calc(100dvh - 40px));
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 22px 24px 20px;
    background: var(--s1);
    border: 1px solid var(--ln);
    border-radius: 24px;
    color: var(--tx);
    box-shadow: 0 24px 64px color-mix(in srgb, var(--onac) 45%, transparent);
    touch-action: auto;
  }
  .panel:focus {
    outline: none;
  }
  header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
  }
  .ttl {
    min-width: 0;
  }
  .ttl h2 {
    margin: 0;
    font-family: var(--font-display);
    font-size: 24px;
    font-weight: 700;
    letter-spacing: -0.02em;
    line-height: 1.1;
  }
  .ttl p {
    margin: 4px 0 0;
    font-size: 13.5px;
    line-height: 1.4;
    color: var(--dim);
    max-width: 52ch;
  }
  .x {
    flex: none;
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    border-radius: 999px;
    background: var(--s2);
    color: var(--tx);
  }
  .x:hover {
    background: color-mix(in srgb, var(--tx) 12%, var(--s2));
  }
  .body {
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
    margin: 0 -6px;
    padding: 0 6px;
  }

  /* Phone: one labelled door and two 44 px feedback targets on the first line,
     the energy under them. The labels of the feedback buttons are their
     accessible names then (aria-label), so nothing is lost to a screen reader
     and the row grows by 8 px, not by 44. The same arrangement serves the
     desktop player's narrow side column, which has the same width. */
  @media (pointer: coarse) {
    .open,
    .steer,
    .step {
      min-height: 44px;
    }
    .step {
      width: 44px;
    }
  }
  @container (max-width: 400px) {
    .row {
      gap: 6px;
    }
    .open {
      flex: 1 1 0;
    }
    .open .amp {
      display: none;
    }
    .steer {
      width: 44px;
      height: 44px;
      padding: 0;
      flex-direction: column;
      gap: 0;
    }
    .st {
      display: none;
    }
    .st-short {
      display: block;
      font-size: 0.58rem;
      font-weight: 600;
      line-height: 1.1;
    }
    .energy {
      flex: 1 1 100%;
      justify-content: center;
    }
    .lvl {
      min-width: 64px;
    }
  }
  @media (max-width: 640px) {
    .veil {
      align-items: flex-end;
      padding: 0;
    }
    .panel {
      width: 100%;
      max-height: 92dvh;
      border-radius: 24px 24px 0 0;
      border-bottom: none;
      padding: 18px 16px calc(16px + env(safe-area-inset-bottom, 0px));
      gap: 12px;
    }
    .x {
      width: 44px;
      height: 44px;
    }
    .ttl h2 {
      font-size: 22px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .open,
    .steer,
    .step {
      transition: none;
    }
  }
</style>
