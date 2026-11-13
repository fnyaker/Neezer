<script>
  // Two levels, one route. The library used to be a single row of eight chips:
  // on a phone it wrapped onto three rows and mixed music with podcasts. Here
  // the world is chosen first (Musique / Podcasts, a segmented control), then
  // the leaf inside it — and the leaf is still what the URL names, so every
  // deep link (`#/library/artists`, `#/library/podcasts`…) keeps working.
  import { onMount, tick } from "svelte";
  import Icon from "./Icon.svelte";

  export let category = "music"; // music | podcasts
  export let tab = "favorites"; // the leaf: favorites, artists, …, gone
  export let musicTabs = [];
  export let podcastTabs = [];
  export let goneCount = null; // the rare list, counted only once opened
  export let showGone = true;
  export let onpick = () => {};
  export let oncategory = () => {};

  const CATEGORIES = [
    { id: "music", label: "Musique", icon: "music" },
    { id: "podcasts", label: "Podcasts", icon: "mic" },
  ];

  $: pod = category === "podcasts";
  $: leaves = pod ? podcastTabs : musicTabs;
  $: selectedLeaf = pod ? podcastTabs[0]?.id : tab;

  // The leaf row scrolls on its own with faded edges (a phone holds two or
  // three labels). The fades only appear on the side that has more to show:
  // a permanent mask would dim the first label at rest.
  let scroller = null;
  let fadeL = false;
  let fadeR = false;
  function measure() {
    if (!scroller) return;
    const max = scroller.scrollWidth - scroller.clientWidth;
    fadeL = scroller.scrollLeft > 2;
    fadeR = max > 2 && scroller.scrollLeft < max - 2;
  }
  // A leaf coming back into view (Musique → Podcasts → Musique) is scrolled
  // back into the strip instead of being left off-screen. Offsets are measured
  // against the scroller's own box: `offsetLeft` would be relative to the
  // nearest positioned ancestor (the page), not to the strip's content.
  function revealSelected() {
    if (!scroller) return;
    const el = scroller.querySelector('[role="tab"][aria-selected="true"]');
    if (!el) return;
    const box = scroller.getBoundingClientRect();
    const rect = el.getBoundingClientRect();
    const left = rect.left - box.left + scroller.scrollLeft;
    const right = left + rect.width;
    // Keep the active leaf out of the mask's fade, not merely inside the box:
    // a tab parked against the edge would be half transparent.
    const EDGE = 42;
    if (left < scroller.scrollLeft + EDGE) scroller.scrollTo({ left: Math.max(0, left - EDGE), behavior: "instant" });
    else if (right > scroller.scrollLeft + scroller.clientWidth - EDGE)
      scroller.scrollTo({ left: right - scroller.clientWidth + EDGE, behavior: "instant" });
  }
  $: if (scroller) {
    category;
    tab;
    leaves;
    tick().then(() => {
      measure();
      revealSelected();
    });
  }

  onMount(() => {
    measure();
    const onResize = () => measure();
    window.addEventListener("resize", onResize, { passive: true });
    const ro = typeof ResizeObserver !== "undefined" ? new ResizeObserver(measure) : null;
    ro?.observe(scroller);
    // A webfont landing changes the strip's width (and the selected tab's
    // place) without a resize event.
    document.fonts?.ready?.then(() => {
      measure();
      revealSelected();
    });
    return () => {
      window.removeEventListener("resize", onResize);
      ro?.disconnect();
    };
  });

  // Roving tab index, the tab pattern's keyboard contract: arrows move between
  // the tabs of a row (and select, since the switch is local and instant),
  // Home/End go to the ends.
  function onKey(e) {
    let dir = 0;
    if (e.key === "ArrowRight") dir = 1;
    else if (e.key === "ArrowLeft") dir = -1;
    else if (e.key === "Home") dir = -2;
    else if (e.key === "End") dir = 2;
    else return;
    const list = e.currentTarget.closest('[role="tablist"]');
    if (!list) return;
    const tabs = [...list.querySelectorAll('[role="tab"]')];
    let i = tabs.indexOf(e.currentTarget);
    if (i < 0) return;
    e.preventDefault();
    if (dir === 1 || dir === -1) i = (i + dir + tabs.length) % tabs.length;
    else i = dir === -2 ? 0 : tabs.length - 1;
    tabs[i].focus();
    tabs[i].click();
  }
</script>

<div class="library-nav">
  <div class="cat-group" role="tablist" aria-label="Section de la bibliothèque" aria-orientation="horizontal">
    {#each CATEGORIES as c (c.id)}
      <button
        type="button"
        role="tab"
        id={`lib-cat-${c.id}`}
        class="cat"
        class:on={category === c.id}
        aria-selected={category === c.id}
        aria-controls="library-view"
        tabindex={category === c.id ? 0 : -1}
        on:click={() => oncategory(c.id)}
        on:keydown={onKey}
      >
        <Icon name={c.icon} size={18} />
        <span>{c.label}</span>
        {#if category === c.id}<span class="mark" aria-hidden="true"><Icon name="check" size={15} /></span>{/if}
      </button>
    {/each}
  </div>

  <div class="subbar" role="tablist" aria-label={pod ? "Podcasts" : "Musique"} aria-orientation="horizontal">
    <div
      class="subtabs"
      class:fade-l={fadeL}
      class:fade-r={fadeR}
      bind:this={scroller}
      on:scroll={measure}
    >
      {#each leaves as t (t.id)}
        <button
          type="button"
          role="tab"
          id={`lib-leaf-${t.id}`}
          class="subtab"
          class:on={selectedLeaf === t.id}
          aria-selected={selectedLeaf === t.id}
          aria-controls="library-view"
          tabindex={selectedLeaf === t.id ? 0 : -1}
          on:click={() => onpick(t.id)}
          on:keydown={onKey}
        >{t.label}</button>
      {/each}
    </div>
    {#if showGone && !pod}
      <button
        type="button"
        role="tab"
        id="lib-leaf-gone"
        class="gone"
        class:on={tab === "gone"}
        aria-selected={tab === "gone"}
        aria-controls="library-view"
        tabindex={tab === "gone" ? 0 : -1}
        on:click={() => onpick("gone")}
        on:keydown={onKey}
      >
        <span class="icn" aria-hidden="true"><Icon name="alert" size={14} /></span>
        <span>Indisponibles</span>
        {#if goneCount}<span class="badge">{goneCount}</span>{/if}
      </button>
    {/if}
  </div>
</div>

<style>
  .library-nav {
    margin-bottom: 20px;
  }
  /* Under the page title the row needs its own air; a phone already has the
     counters line between them. */
  @media (min-width: 641px) {
    .library-nav {
      margin-top: 6px;
    }
  }

  /* The segmented control: one track, two equal segments, the active one is a
     solid accent surface with a check mark — not colour alone (§10). 54 px
     outer for a 44 px target inside. */
  .cat-group {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 4px;
    width: 100%;
    max-width: 480px;
    height: 54px;
    padding: 4px;
    border: 1px solid var(--ln);
    border-radius: 999px;
    background: var(--s1);
  }
  .cat {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    min-width: 0;
    height: 44px;
    padding: 0 14px;
    border: 0;
    border-radius: 999px;
    background: none;
    color: var(--dim);
    font: inherit;
    font-size: 15px;
    font-weight: 600;
    line-height: 1;
    white-space: nowrap;
    cursor: pointer;
    transition:
      background-color 0.16s ease,
      color 0.16s ease;
  }
  .cat:hover:not(.on) {
    background: var(--s2);
    color: var(--tx);
  }
  .cat.on {
    background: var(--ac);
    color: var(--onac);
  }
  .cat :global(svg) {
    flex: none;
  }
  .mark {
    display: grid;
    place-items: center;
    margin-left: 1px;
    opacity: 0.9;
  }

  /* The leaves: text tabs on a hairline, the active one underlined in the
     accent-as-ink token (a lemon line would vanish on Papier). */
  .subbar {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    margin-top: 12px;
    border-bottom: 1px solid var(--ln);
  }
  .subtabs {
    --fade-l: 0px;
    --fade-r: 0px;
    display: flex;
    align-items: center;
    gap: 2px;
    flex: 1 1 auto;
    min-width: 0;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
    -webkit-overflow-scrolling: touch;
    scroll-snap-type: x proximity;
    scroll-padding-inline: 4px;
    -webkit-mask-image: linear-gradient(
      to right,
      transparent 0,
      #000 var(--fade-l),
      #000 calc(100% - var(--fade-r)),
      transparent 100%
    );
    mask-image: linear-gradient(
      to right,
      transparent 0,
      #000 var(--fade-l),
      #000 calc(100% - var(--fade-r)),
      transparent 100%
    );
  }
  .subtabs.fade-l {
    --fade-l: 40px;
  }
  .subtabs.fade-r {
    --fade-r: 40px;
  }
  .subtabs::-webkit-scrollbar {
    display: none;
  }
  .subtab {
    position: relative;
    display: inline-flex;
    align-items: center;
    flex: none;
    height: 46px;
    padding: 0 12px;
    border: 0;
    border-radius: 8px;
    background: none;
    color: var(--dim);
    font: inherit;
    font-size: 14px;
    font-weight: 600;
    line-height: 1;
    white-space: nowrap;
    scroll-snap-align: start;
    cursor: pointer;
    transition: color 0.14s ease;
  }
  .subtab::after {
    content: "";
    position: absolute;
    left: 12px;
    right: 12px;
    bottom: 0;
    height: 2px;
    border-radius: 2px;
    background: var(--ac-text);
    transform: scaleX(0);
    transform-origin: left center;
    transition: transform 0.18s ease;
  }
  .subtab:hover,
  .subtab.on {
    color: var(--tx);
  }
  .subtab.on::after {
    transform: scaleX(1);
  }

  /* The rare list stays a filter on the right, outside the scroller so it is
     always reachable; it is never one of the destinations. */
  .gone {
    display: inline-flex;
    align-items: center;
    flex: none;
    gap: 6px;
    height: 34px;
    padding: 0 11px;
    border: 1px solid var(--ln);
    border-radius: 999px;
    background: none;
    color: var(--dim);
    font: inherit;
    font-size: 13px;
    font-weight: 600;
    line-height: 1;
    white-space: nowrap;
    cursor: pointer;
  }
  .gone:hover {
    background: var(--s2);
    color: var(--tx);
  }
  .gone.on {
    border-color: transparent;
    background: var(--ac);
    color: var(--onac);
  }
  .gone .icn {
    display: grid;
    place-items: center;
  }
  .badge {
    display: inline-block;
    padding: 1px 7px;
    border-radius: 999px;
    background: var(--s2);
    color: var(--tx);
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 500;
    line-height: 1.4;
  }
  .gone.on .badge {
    background: var(--onac);
    color: var(--ac);
  }

  .cat:focus-visible,
  .subtab:focus-visible,
  .gone:focus-visible {
    outline: 2px solid var(--ac-text);
    outline-offset: 2px;
  }

  @media (max-width: 640px) {
    .library-nav {
      margin-bottom: 18px;
    }
    .cat-group {
      max-width: none;
    }
    .cat {
      padding: 0 10px;
      font-size: 15px;
    }
    .subbar {
      margin-top: 10px;
      gap: 6px;
    }
    .subtabs {
      gap: 0;
    }
    .subtab {
      height: 45px;
      padding: 0 10px;
    }
    .subtab::after {
      left: 10px;
      right: 10px;
    }
    .gone {
      height: 40px;
      padding: 0 10px;
      font-size: 12.5px;
    }
    /* The alert glyph costs a whole label's width on a 390 px screen; the
       word and its count already say what the filter is. */
    .gone .icn {
      display: none;
    }
  }

  @media (pointer: coarse) {
    .gone {
      min-height: 44px;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .cat,
    .subtab,
    .subtab::after {
      transition: none;
    }
  }
</style>
