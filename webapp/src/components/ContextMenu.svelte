<script>
  import { contextMenu, closeMenu } from "../lib/stores.js";
  import { tick } from "svelte";
  import Icon from "./Icon.svelte";
  import { DUR, collapse, pop, sheet, veil } from "../lib/motion.js";
  import { isMobile } from "../lib/viewport.js";

  // O1/O2: the same menu is a popover on a desktop and a bottom sheet under
  // 640 px (the breakpoint app.css's media query and viewport.js#isMobile share).
  // motion.js owns both shapes, their durations and their curves (M15); this
  // only picks which one starts.
  const PAD = 8;

  function menuMotion(node, { x, y }) {
    if (!isMobile()) {
      // The corner nearest the pointer, read off the menu it grows from, with
      // the same flip test place() applies below (pad 8). One rect, on an
      // explicit action and not on a list row: the scale then starts at the
      // finger even when the menu had to be pushed back on screen.
      const box = node && node.getBoundingClientRect ? node.getBoundingClientRect() : { width: 0, height: 0 };
      const right = box.width ? x + box.width + PAD > window.innerWidth : x > window.innerWidth / 2;
      const up = box.height ? y + box.height + PAD > window.innerHeight : y > window.innerHeight / 2;
      return pop(node, {
        origin: `${up ? "bottom" : "top"} ${right ? "right" : "left"}`,
        placement: up ? "up" : "down",
      });
    }
    // The phone sheet enters on DUR.sheet (280 ms) and leaves on DUR.instant.
    // M5 wants an exit shorter than its entrance, and `test/menu/run.mjs` — this
    // lot's own gate, 242 checks — asserts the menu is out of the DOM 180 ms
    // after Escape. Measured: at the 200 ms token it was still there 16 times
    // out of 16; at 140 ms it missed once (Svelte only starts the real outro on
    // the frame after the dummy); at 90 ms it clears in 105 ms. A dismissal the
    // person asked for has nothing to admire (NN/g: the more frequent, shorter).
    const shape = sheet(node, {});
    return ({ direction }) => {
      const config = shape({ direction });
      if (direction !== "out" || !(config.duration > DUR.instant)) return config;
      return { ...config, duration: DUR.instant };
    };
  }

  // The phone sheet's scrim walks its own tint in (M8: an opacity on this
  // wrapper would switch the sheet's backdrop-filter off). The desktop backdrop
  // is invisible and gets no transition at all. Its exit follows the sheet's
  // (DUR.instant, see menuMotion): the `{#if}` branch is only emptied once the
  // LONGEST transition in it has played, so a 200 ms scrim kept the menu in the
  // DOM 209 ms after Escape.
  function backdropMotion(node) {
    if (!isMobile()) return { duration: 0 };
    const shape = veil(node, { for: "sheet" });
    return ({ direction }) => {
      const config = shape({ direction });
      if (direction !== "out" || !(config.duration > DUR.instant)) return config;
      return { ...config, duration: DUR.instant };
    };
  }

  let el;
  let pos = { x: 0, y: 0 };
  let openSub = null;
  let trigger = null;
  let wasOpen = false;

  $: if ($contextMenu) {
    if (!wasOpen) {
      const active = typeof document !== "undefined" ? document.activeElement : null;
      trigger = active && active !== document.body ? active : null;
      wasOpen = true;
    }
    place($contextMenu);
  } else if (wasOpen) {
    wasOpen = false;
    restore();
  }

  async function place(menu) {
    openSub = null;
    pos = { x: menu.x, y: menu.y };
    await tick();
    if (!el) return;
    if (window.matchMedia("(max-width: 640px)").matches) {
      focusAt(0);
      return;
    }
    const r = el.getBoundingClientRect();
    let x = menu.x;
    let y = menu.y;
    if (x + r.width + PAD > window.innerWidth) x = window.innerWidth - r.width - PAD;
    if (y + r.height + PAD > window.innerHeight) y = window.innerHeight - r.height - PAD;
    pos = { x: Math.max(PAD, x), y: Math.max(PAD, y) };
    await tick();
    focusAt(0);
  }

  function entries() {
    if (!el) return [];
    return [...el.querySelectorAll('[role="menuitem"]')].filter(
      (b) => !b.disabled && b.getClientRects().length > 0
    );
  }

  function focusAt(i) {
    const list = entries();
    if (!list.length) {
      el?.focus?.();
      return;
    }
    const at = Math.max(0, Math.min(i, list.length - 1));
    list[at].focus({ preventScroll: true });
  }

  function restore() {
    const back = trigger;
    trigger = null;
    if (back && back.isConnected) back.focus?.({ preventScroll: true });
  }

  function dismiss() {
    const back = trigger;
    trigger = null;
    wasOpen = false;
    closeMenu();
    if (back && back.isConnected) back.focus?.({ preventScroll: true });
  }

  function onKeydown(e) {
    const list = entries();
    const i = list.indexOf(document.activeElement);
    if (e.key === "ArrowDown") {
      e.preventDefault();
      focusAt(i < 0 ? 0 : i + 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      focusAt(i < 0 ? list.length - 1 : i - 1);
    } else if (e.key === "Home") {
      e.preventDefault();
      focusAt(0);
    } else if (e.key === "End") {
      e.preventDefault();
      focusAt(list.length - 1);
    } else if (e.key === "Escape") {
      e.preventDefault();
      dismiss();
    } else if (e.key === "Tab") {
      dismiss();
    }
  }

  function toggleSub(item) {
    openSub = openSub === item ? null : item;
  }

  // A submenu opens on tap/click (works on touch, unlike the old hover); leaf
  // items run their action and close the menu.
  function run(item) {
    if (item.sub) {
      toggleSub(item);
      tick().then(() => {
        const at = entries().findIndex((b) => b.classList.contains("sub"));
        if (at >= 0) focusAt(at);
      });
      return;
    }
    dismiss();
    item.action?.();
  }
</script>

<svelte:window
  on:keydown={(e) => {
    if (e.key === "Escape" && !e.defaultPrevented && $contextMenu) dismiss();
  }}
/>

{#if $contextMenu}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div
    class="backdrop"
    transition:backdropMotion
    on:click={() => closeMenu()}
    on:contextmenu|preventDefault|stopPropagation={() => closeMenu()}
  ></div>
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div
    class="menu"
    role="menu"
    aria-label="Actions"
    tabindex="-1"
    bind:this={el}
    style="left:{pos.x}px; top:{pos.y}px"
    transition:menuMotion={{ x: pos.x, y: pos.y }}
    on:click|stopPropagation
    on:contextmenu|preventDefault|stopPropagation
    on:keydown={onKeydown}
  >
    {#each $contextMenu.items as item}
      {#if item === "divider"}
        <div class="divider"></div>
      {:else if item.sub}
        <button
          type="button"
          class="item has-sub"
          class:expanded={openSub === item}
          role="menuitem"
          on:click|stopPropagation={() => toggleSub(item)}
        >
          <span class="ic"><Icon name={item.icon} size={17} /></span>
          <span class="lbl">{item.label}</span>
          <span class="chev"><Icon name="chevronRight" size={15} /></span>
        </button>
        {#if openSub === item}
          <div class="submenu" role="menu" transition:collapse>
            {#if item.sub.length}
              {#each item.sub as sit}
                <button type="button" class="item sub" role="menuitem" on:click={() => run(sit)}>
                  <span class="ic"><Icon name={sit.icon} size={17} /></span>
                  <span class="lbl">{sit.label}</span>
                </button>
              {/each}
            {:else}
              <div class="item sub empty">Aucune playlist</div>
            {/if}
          </div>
        {/if}
      {:else}
        <button type="button" class="item" role="menuitem" class:danger={item.danger} on:click={() => run(item)}>
          <span class="ic"><Icon name={item.icon} size={17} /></span>
          <span class="lbl">{item.label}</span>
        </button>
      {/if}
    {/each}
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 290;
  }
  .menu {
    position: fixed;
    z-index: 300;
    min-width: 220px;
    max-height: 70vh;
    overflow-y: auto;
    /* Tier 2 of the glass ladder: app.css owns the material (fill, hairline,
       shadow and the one blur). This sheet only lays the menu out. */
    border-radius: 12px;
    padding: 6px;
    color: var(--tx);
    font-size: 0.9rem;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 9px 10px;
    border-radius: 7px;
    cursor: pointer;
    position: relative;
    white-space: nowrap;
    width: 100%;
    text-align: left;
    font: inherit;
    color: inherit;
  }
  .item:hover {
    background: var(--glass-hover);
  }
  .item.danger {
    color: var(--accent-2);
  }
  .item.empty {
    color: var(--text-dim);
    cursor: default;
  }
  .item.empty:hover {
    background: none;
  }
  .has-sub .chev {
    margin-left: auto;
    color: var(--text-dim);
    transition: transform var(--dur-base) var(--ease-standard);
  }
  .has-sub.expanded .chev {
    transform: rotate(90deg);
  }
  .ic {
    width: 18px;
    text-align: center;
    opacity: 0.85;
  }
  .lbl {
    flex: 1;
  }
  .divider {
    height: 1px;
    background: var(--ln);
    margin: 5px 8px;
  }
  /* Submenu expands inline (accordion) so it works the same with a mouse or a
     finger — the old side-positioned, hover-only flyout was unusable on touch.
     It is a step INTO the same material (4 % of the ink over the menu's fill),
     not a second surface: no border, no filter, no second tint to keep in
     step with the ladder. */
  .submenu {
    max-height: 40vh;
    overflow-y: auto;
    margin: 2px 2px 4px;
    border-radius: 9px;
    background: color-mix(in srgb, var(--tx) 4%, transparent);
  }
  .item.sub {
    padding-left: 34px;
  }

  /* Phone: render as a bottom sheet with large tap targets. The slide itself is
     motion.js#sheet (O2): no `sheet-up` keyframes here any more. */
  @media (max-width: 640px) {
    .backdrop {
      background: rgba(0, 0, 0, 0.5);
    }
    .menu {
      left: 0 !important;
      right: 0;
      top: auto !important;
      bottom: 0;
      width: 100%;
      min-width: 0;
      max-height: 78vh;
      border-radius: 16px 16px 0 0;
      padding: 8px 8px calc(10px + env(safe-area-inset-bottom));
    }
    .item {
      padding: 14px 14px;
      font-size: 1rem;
      border-radius: 10px;
    }
    .item.sub {
      padding-left: 40px;
    }
    .submenu {
      max-height: 42vh;
    }
  }
</style>
