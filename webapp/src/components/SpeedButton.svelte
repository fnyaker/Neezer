<script>
  // Playback speed of the podcast that is playing, as a chip and a drop-up.
  // Only an episode has one: nobody wants a song at 1.5x.
  import { current, toasts } from "../lib/stores.js";
  import { partyHost } from "../lib/party/hostbridge.js";
  import { SPEEDS, currentSpeed, setSpeed, speedLabel } from "../lib/speed.js";
  import Icon from "./Icon.svelte";

  export let align = "right";
  // Where the list hangs from: "up" from a footer, "down" from a header.
  export let placement = "up";

  let open = false;

  $: visible = !!$current && !!$current.podcast;
  // A party's guests play the host's audio in real time: a faster host would
  // pull away from them, so the speed waits until the party is over.
  $: locked = !!$partyHost;

  function pick(r) {
    setSpeed($current, r);
    open = false;
  }
  function toggle() {
    if (locked) {
      toasts.push("La vitesse est fixée à 1× pendant une party", "info");
      return;
    }
    open = !open;
  }
</script>

<svelte:window on:click={() => (open = false)} on:keydown={(e) => e.key === "Escape" && (open = false)} />

{#if visible}
  <div class="sp">
    <button
      class="trigger"
      class:on={$currentSpeed !== 1 && !locked}
      class:locked
      on:click|stopPropagation={toggle}
      aria-haspopup="listbox"
      aria-expanded={open}
      aria-label="Vitesse de lecture : {speedLabel(locked ? 1 : $currentSpeed)}"
      title="Vitesse de lecture"
    >
      <span>{speedLabel(locked ? 1 : $currentSpeed)}</span>
    </button>
    {#if open}
      <ul class="menu {placement} {align}" role="listbox" on:click|stopPropagation>
        <li class="mh">Vitesse de lecture</li>
        {#each SPEEDS as r}
          <li>
            <button role="option" aria-selected={$currentSpeed === r} class:sel={$currentSpeed === r} on:click={() => pick(r)}>
              <span>{r === 1 ? "Normale" : speedLabel(r)}</span>
              {#if $currentSpeed === r}<Icon name="check" size={15} />{/if}
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
{/if}

<style>
  .sp {
    position: relative;
    display: inline-flex;
  }
  .trigger {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    height: 34px;
    min-width: 46px;
    padding: 0 10px;
    font-size: 0.78rem;
    font-weight: 800;
    font-variant-numeric: tabular-nums;
    color: var(--tx);
    background: var(--s2);
    border: 1px solid var(--ln);
    border-radius: 999px;
    cursor: pointer;
  }
  .trigger:hover {
    background: var(--ln);
  }
  .trigger.on {
    color: var(--ac);
    border-color: var(--ac);
  }
  .trigger.locked {
    opacity: 0.5;
  }
  .menu {
    position: absolute;
    min-width: 176px;
    list-style: none;
    margin: 0;
    padding: 6px;
    background: var(--s2);
    border: 1px solid var(--ln);
    border-radius: 16px;
    box-shadow: 0 16px 44px color-mix(in srgb, var(--bg) 65%, transparent);
    z-index: 40;
  }
  .menu.up {
    bottom: calc(100% + 8px);
  }
  .menu.down {
    top: calc(100% + 8px);
  }
  .menu.right {
    right: 0;
  }
  .menu.left {
    left: 0;
  }
  .mh {
    padding: 8px 10px 6px;
    font-size: 0.7rem;
    font-weight: 800;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--dim);
  }
  .menu button {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    width: 100%;
    padding: 9px 10px;
    border-radius: 8px;
    font-size: 0.88rem;
    color: var(--tx);
    text-align: left;
  }
  .menu button:hover {
    background: var(--s1);
  }
  .menu button.sel {
    color: var(--ac);
  }
  @media (max-width: 640px) {
    .menu {
      min-width: 210px;
      padding: 8px;
    }
    .menu button {
      padding: 13px 14px;
      font-size: 0.95rem;
    }
  }
</style>
