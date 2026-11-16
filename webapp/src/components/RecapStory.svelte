<script>
  // The story's mechanics: which cards are mounted, the two tap zones, the
  // keyboard, the swipe, and the 7-second auto-advance that only exists on a
  // phone-sized screen.
  //
  // No request happens here. Covers come from the mounted slides, which pass
  // through Cover.svelte's bounded queue. The window is "active + next", never
  // the whole deck: opening costs one card's art, not nine cards'.
  import { onMount, onDestroy, createEventDispatcher } from "svelte";
  import RecapSlide from "./RecapSlide.svelte";
  import { cardIds, slideTitle } from "../lib/recap.js";

  export let payload;
  export let index = 0;
  // The screen pauses the story while it is not the thing being looked at (the
  // share sheet is open, a year is loading). The card id stays OUT of the
  // props — it is always `cardIds(payload)[index]`.
  export let suspended = false;

  const AUTO_MS = 7000;

  const dispatch = createEventDispatcher();

  let mounted = false;
  let narrow = false;
  let reduce = false;
  let away = false;
  let holding = false;
  let focused = true;
  let timer = 0;
  let barKey = 0;
  let rootEl = null;
  let swipe = null;
  let suppressTapUntil = 0;
  const queries = [];

  $: ids = cardIds(payload);
  $: if (ids.length && index > ids.length - 1) index = ids.length - 1;
  $: if (index < 0) index = 0;
  $: auto = mounted && narrow && !reduce;
  $: paused = !mounted || suspended || holding || !focused || away;
  $: arm(auto, paused, index, ids.length);
  $: live = ids.length ? `${index + 1} sur ${ids.length}. ${slideTitle(payload, ids[index])}` : "";
  $: visible = ids
    .map((_, i) => i)
    .filter((i) => i === index || i === index + 1);

  // A new card (or a resume) gets a full, honest 7 seconds: the bar and the
  // timeout are re-created together, so they can never disagree.
  function arm(enabled, isPaused, at, count) {
    clearTimeout(timer);
    timer = 0;
    if (!enabled || isPaused || at >= count - 1) return;
    barKey += 1;
    timer = setTimeout(() => {
      timer = 0;
      if (auto && !paused && index === at) go(1);
    }, AUTO_MS);
  }

  function go(delta) {
    const next = index + delta;
    if (next < 0 || next >= ids.length) return;
    index = next;
  }

  function tap(delta) {
    // A horizontal swipe also produces a click on a touch screen; the gesture
    // that just turned the card must not turn it twice.
    if (Date.now() < suppressTapUntil) return;
    go(delta);
  }

  function onKey(e) {
    // Ctrl/⌘/Alt + arrows belong to the player's own shortcuts (App.svelte):
    // only the plain reading keys are the deck's.
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    if (e.key === "ArrowRight" || e.key === "PageDown") {
      e.preventDefault();
      go(1);
    } else if (e.key === "ArrowLeft" || e.key === "PageUp") {
      e.preventDefault();
      go(-1);
    }
  }

  function onFocusOut(e) {
    // Focus moving to a control INSIDE the deck (a tap zone, "Lire ces 5
    // titres") is not leaving it.
    if (!rootEl || !e.relatedTarget || !rootEl.contains(e.relatedTarget)) focused = false;
  }
  function onFocusIn() {
    focused = true;
  }

  function onVisibility() {
    away = typeof document !== "undefined" && document.hidden;
  }

  function onPointerDown(e) {
    if (e.pointerType === "mouse") return;
    // A gesture that starts on a real control belongs to that control.
    const control = e.target && e.target.closest && e.target.closest(".slide button, .slide a, .slide input");
    if (control) return;
    holding = true;
    swipe = { x: e.clientX, y: e.clientY, id: e.pointerId };
  }
  function onPointerUp(e) {
    holding = false;
    if (!swipe || swipe.id !== e.pointerId) {
      swipe = null;
      return;
    }
    const dx = e.clientX - swipe.x;
    const dy = e.clientY - swipe.y;
    swipe = null;
    if (Math.abs(dx) > 48 && Math.abs(dx) > Math.abs(dy) * 1.4) {
      suppressTapUntil = Date.now() + 400;
      go(dx < 0 ? 1 : -1);
    }
  }
  function onPointerCancel() {
    holding = false;
    swipe = null;
  }

  onMount(() => {
    const watch = (query, apply) => {
      apply(query.matches);
      const handler = (e) => apply(e.matches);
      if (query.addEventListener) query.addEventListener("change", handler);
      queries.push([query, handler]);
    };
    watch(window.matchMedia("(max-width: 900px)"), (v) => (narrow = v));
    watch(window.matchMedia("(prefers-reduced-motion: reduce)"), (v) => (reduce = v));
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("pointerup", onPointerUp);
    window.addEventListener("pointercancel", onPointerCancel);
    onVisibility();
    mounted = true;
    return () => {
      for (const [query, handler] of queries) query.removeEventListener("change", handler);
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("pointerup", onPointerUp);
      window.removeEventListener("pointercancel", onPointerCancel);
    };
  });

  onDestroy(() => clearTimeout(timer));
</script>

<!-- The deck is a keyboard-operable reading region: the handler lives on the
     container (focus on it), not on window, so the page's own arrows and the
     year chips keep their meaning. `role="region"` is not a widget, hence the
     two a11y hints — the tabindex IS the point. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex a11y_no_noninteractive_element_interactions -->
<div
  class="story"
  class:paused
  class:auto
  bind:this={rootEl}
  tabindex="0"
  role="region"
  aria-label={`Récap ${payload && payload.year ? payload.year : ""}`}
  style={`--fill-ms:${AUTO_MS}ms`}
  on:keydown={onKey}
  on:focusin={onFocusIn}
  on:focusout={onFocusOut}
  on:pointerdown={onPointerDown}
>
  <div class="bars" aria-hidden="true">
    {#each ids as id, i (id)}
      <span class="bar" class:done={i < index}>
        {#if i === index}
          {#key barKey}
            <!-- The countdown bar says a 7 s timer is running: it is a status
                 indicator (M13), so eco keeps it moving and everything else
                 stops. Its duration is the JS clock's, not a motion token. -->
            <span
              class="fill"
              class:run={auto}
              class:full={!auto}
              data-motion={auto ? "status" : undefined}
            ></span>
          {/key}
        {:else if i < index}
          <span class="fill full"></span>
        {/if}
      </span>
    {/each}
  </div>

  <p class="sr" aria-live="polite" aria-atomic="true">{live}</p>

  <div class="stage">
    {#each visible as i (i)}
      <div class="slide-wrap" class:active={i === index} aria-hidden={i !== index}>
        <RecapSlide
          {payload}
          index={i}
          on:play={() => dispatch("play")}
          on:share={() => dispatch("share")}
          on:changeyear={() => dispatch("changeyear")}
          on:restart={() => (index = 0)}
        />
      </div>
    {/each}

    <button class="tap prev" aria-label="Carte précédente" on:click={() => tap(-1)}></button>
    <button class="tap next" aria-label="Carte suivante" on:click={() => tap(1)}></button>
  </div>

  <div class="foot">
    <span class="count mono">{index + 1} / {ids.length}</span>
    <span class="hint mono" aria-hidden="true">← →</span>
  </div>
</div>

<style>
  .story {
    position: relative;
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
    outline: none;
    /* Vertical drags scroll the page (the 360×640 deck needs it); horizontal
       drags are the card swipe. */
    touch-action: pan-y;
  }
  .story:focus-visible {
    box-shadow: inset 0 0 0 2px var(--ac-text);
  }
  .bars {
    flex: none;
    display: flex;
    gap: 4px;
    padding: 12px 12px 0;
  }
  .bar {
    flex: 1;
    min-width: 0;
    height: 3px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--tx) 16%, transparent);
    overflow: hidden;
  }
  .fill {
    display: block;
    width: 100%;
    height: 100%;
    border-radius: inherit;
    background: var(--tx);
    /* The bar grows by transform, never by width (M1): the same 7 s, no layout
       on a phone that is already decoding a card. */
    transform: scaleX(0);
    transform-origin: 0 50%;
  }
  .fill.full,
  .fill.run {
    transition: none;
  }
  .fill.run {
    animation: recap-fill var(--fill-ms) linear forwards;
  }
  .fill.full {
    transform: none;
  }
  .story.paused .fill.run {
    animation-play-state: paused;
  }
  @keyframes recap-fill {
    from {
      transform: scaleX(0);
    }
    to {
      transform: scaleX(1);
    }
  }

  .stage {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
  .slide-wrap {
    position: absolute;
    inset: 0;
    z-index: 2;
    visibility: hidden;
    opacity: 0;
    /* A story slide FADES IN over 200 ms (I15, M7): the bridge mounts the
       active card and the next one and nothing else (webapp.md's sleeve
       budget; `test/recap/run.mjs` requires exactly one `.active`), so the card
       being left is removed in the same frame and there is no one to fade out.
       Both cards are absolutely positioned, so neither costs a layout, and a
       fast next/prev reverses from where it is. */
    transition: opacity var(--dur-base) var(--ease-standard);
    /* The next card is mounted (and its art loading) but not seen and not
       focusable — `visibility` also takes its buttons out of the tab order.
       `pointer-events:none` is what lets a tap on the card's words fall through
       to the two zones UNDER it; the card's own controls take their click back
       (RecapSlide re-enables them). */
    pointer-events: none;
  }
  .slide-wrap.active {
    visibility: visible;
    opacity: 1;
  }

  /* The two real tap targets. They sit UNDER the card: the card itself is
     pointer-events:none, so a tap on its words falls through here, while its
     own buttons ("Lire ces 5 titres", "Partager") take their click back. */
  .tap {
    position: absolute;
    top: 0;
    bottom: 0;
    z-index: 1;
    min-height: 44px;
    background: transparent;
    border: 0;
    border-radius: 0;
    color: transparent;
  }
  .tap.prev {
    left: 0;
    width: 40%;
  }
  .tap.next {
    right: 0;
    width: 60%;
  }
  .tap:focus-visible {
    outline: none;
    background: color-mix(in srgb, var(--ac) 9%, transparent);
    box-shadow: inset 0 0 0 2px var(--ac-text);
  }

  .foot {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 8px 18px 12px;
    color: var(--dim);
    font-size: 10.5px;
    letter-spacing: 0.06em;
  }
  .count {
    font-variant-numeric: tabular-nums;
  }
  .hint {
    opacity: 0.7;
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip: rect(0 0 0 0);
    clip-path: inset(50%);
    white-space: nowrap;
    border: 0;
  }

  @media (max-width: 640px) {
    .bars {
      padding: 10px 10px 0;
    }
    .foot {
      padding: 6px 14px 10px;
    }
    .hint {
      display: none;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .fill.run {
      animation: none;
      transform: none;
    }
  }
</style>
