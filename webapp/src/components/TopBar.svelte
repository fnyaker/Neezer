<script>
  // Sillon's top bar, sticky at the top of the scrolling <main>: back, the
  // search door, the Flow's level, and the one button that starts it.
  //
  // The search field here is a BUTTON, not an input: the real box lives on
  // Explorer (where the results are), so typing must not be duplicated in a
  // shell that has nowhere to show what you type. ⌘K / Ctrl+K opens it and
  // focuses the real field.
  import { link, push } from "svelte-spa-router";
  import { onMount } from "svelte";
  import { location } from "../lib/router.js";
  import { discoveryLevel, stepOf } from "../lib/discovery.js";
  import { startFlow } from "../lib/flow.js";
  import { toasts } from "../lib/stores.js";
  import { REMOTE } from "../lib/remote/mode.js";
  import BackButton from "./BackButton.svelte";
  import Icon from "./Icon.svelte";

  $: step = stepOf($discoveryLevel);

  let flowLoading = false;

  // The bar's own width, not the window's: with the File panel open at 1025 px
  // the shell's column is only ~450 px wide, while the same window with the
  // panel shut has room for the labelled button. Window media queries cannot
  // see that difference; a container query on <main> would make main a
  // containment root (and the containing block of every fixed overlay in it),
  // so the bar measures itself instead — one observer, fires on resize and on
  // the panel opening, no layout read in the click path.
  let barEl;
  let narrow = false; // no room for the Flow's level chip
  let compact = false; // no room for the Flow's label
  onMount(() => {
    const ro = new ResizeObserver(([e]) => {
      const w = e.contentRect.width;
      narrow = w < 660;
      compact = w < 520;
    });
    ro.observe(barEl);
    return () => ro.disconnect();
  });

  function focusSearch() {
    const box = document.querySelector('main input[type="search"]');
    if (box) box.focus();
    return !!box;
  }

  // The route's field is built after the hashchange has been handled, so it is
  // not there on the very next statement: keep asking for a few frames. ⌘K must
  // land the caret in the box, otherwise it is just a slower link.
  function focusWhenReady(tries = 12) {
    if (focusSearch() || !tries) return;
    setTimeout(() => focusWhenReady(tries - 1), 50);
  }

  function goSearch() {
    // Already there: the box is on screen, so answering means putting the
    // caret in it rather than pushing the same route again.
    if ($location.startsWith("/search")) focusWhenReady();
    else {
      push("/search");
      focusWhenReady();
    }
  }

  async function launchFlow() {
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

  // ⌘K / Ctrl+K, the shortcut the bar advertises. The global player shortcuts
  // in App.svelte ignore typing targets; this one is the opposite — it exists
  // to LAND in a typing target.
  function onKey(e) {
    if (!(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey) return;
    if (String(e.key).toLowerCase() !== "k") return;
    e.preventDefault();
    goSearch();
  }
</script>

<svelte:window on:keydown={onKey} />

<div class="topbar" class:narrow class:compact bind:this={barEl}>
  <!-- No reserved gutter: on a route with somewhere to go back to the button is
       the bar's first child (the bar's own gap spaces it); elsewhere nothing is
       left empty, because a phone cannot spare 48 px for a hole. -->
  <span class="back-slot"><BackButton /></span>

  {#if !$location.startsWith("/search")}
    <button class="field search" on:click={goSearch} aria-label="Rechercher un titre, un artiste, un album, un podcast">
      <Icon name="search" size={16} />
      <span class="hint">Titres, artistes, albums, podcasts…</span>
      <span class="kbd">⌘K</span>
    </button>
  {/if}

  <span class="grow"></span>

  <a
    use:link
    href="/settings/discovery"
    class="chip level"
    title="Niveau du Flow — le régler dans Réglages › Découverte"
  >
    {step.name} · Flow
  </a>

  {#if !REMOTE}
    <button
      class="btn btn-primary flow"
      on:click={launchFlow}
      disabled={flowLoading}
      aria-busy={flowLoading}
      aria-label="Lancer mon Flow"
    >
      <Icon name="play" size={14} />
      <span class="label">Lancer mon Flow</span>
    </button>
  {/if}
</div>

<style>
  .topbar {
    /* Sticky inside <main>, which is the scroll container. main's own top
       padding is handed to the bar (App.svelte): a sticky box keeps its
       padding-edge position and IGNORES a negative top margin, so bleeding it
       that way left it 32 px low and stuck 32 px down the viewport. The bar
       gives the page back the same gap underneath itself. */
    position: sticky;
    top: 0;
    z-index: 5;
    display: flex;
    align-items: center;
    gap: 12px;
    height: var(--topbar-h, 68px);
    margin: 0 calc(-1 * var(--page-x, 32px)) var(--page-y, 32px);
    padding: 0 var(--page-x, 32px);
    background: var(--bg);
    border-bottom: 1px solid var(--ln);
  }
  .back-slot {
    /* Laid out as if the button were the bar's own child: no reserved gutter
       when the route has nowhere to go back to (a phone cannot spare 48 px
       for an empty slot), and the bar's own gap when it has. */
    display: contents;
  }
  .search {
    flex: 0 1 460px;
    max-width: 45%;
    min-width: 0;
    text-align: left;
  }
  .hint {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    text-align: left;
  }
  .kbd {
    flex: none;
  }
  .grow {
    flex: 1;
  }
  .level {
    color: var(--tx);
    font-weight: 500;
  }

  /* The bar's column, measured (see the script): drop the level chip first,
     then the Flow's label — the search keeps a usable width and the bar never
     overflows its column. */
  .topbar.narrow .level {
    display: none;
  }
  .topbar.compact .flow {
    width: 40px;
    padding: 0;
  }
  .topbar.compact .flow .label {
    display: none;
  }
  /* With the label gone the right-hand cluster is narrow, so the field may use
     what is left instead of keeping the 45 % reserve for it. */
  .topbar.compact .search {
    max-width: 460px;
  }
  /* Phone: the shell G reworks the frame, but the bar must still fit 360 px. */
  @media (max-width: 640px) {
    .topbar {
      gap: 8px;
      padding-top: var(--safe-top, env(safe-area-inset-top, 0px));
    }
    .search {
      flex: 1 1 auto;
      max-width: none;
    }
    .kbd {
      display: none;
    }
    /* Later in the sheet than the compact rule above, and as specific: the
       glyph keeps §10's 44 px under a finger. */
    .topbar.compact .flow {
      width: 44px;
      height: 44px;
    }
  }
  @media (max-width: 420px) {
    .search .hint {
      font-size: 13px;
    }
  }
  /* §10: a target is 44 px under a finger — the field is a button here. */
  @media (pointer: coarse) {
    .search {
      min-height: 44px;
    }
  }
</style>
