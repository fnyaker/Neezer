<script>
  import { duration as fmtDuration } from "../lib/format.js";
  import { highlightParts } from "../lib/transcript.js";
  import Cover from "./Cover.svelte";
  // I10/I6: the passages are a block that arrives after a search — it enters
  // ONCE per visit, and `reveal` is what says so: Search re-answers the same
  // term on every settled keystroke, and fading the list each time is exactly
  // what M6 forbids.
  import { fadeIn } from "../lib/motion.js";

  export let hits = [];
  export let query = "";
  export let showChannel = false;
  export let onPick = null;
  /** Arm this block's entrance (Search arms it for the first answer of the visit). */
  export let reveal = true;

  function pickup(hit) {
    if (onPick) onPick(hit);
  }

  function spaceKeys(node) {
    const handler = (event) => {
      if (event.key === " " && event.target && event.target.closest && event.target.closest("button")) {
        event.stopPropagation();
      }
    };
    node.addEventListener("keydown", handler);
    return {
      destroy() {
        node.removeEventListener("keydown", handler);
      },
    };
  }
</script>

<div class="hits" use:spaceKeys in:fadeIn={reveal ? undefined : { duration: 0 }}>
  {#each hits as hit, i (hit.episode_id + ":" + hit.start + ":" + i)}
    {@const title = hit.episode?.title || hit.episode_title || "Épisode"}
    {@const channel = hit.channel_title || hit.episode?.channel_title || ""}
    <button
      type="button"
      class="hit"
      on:click={() => pickup(hit)}
    >
      <span class="sr">Lire {title} à {fmtDuration(hit.start)}.</span>
      <span class="art">
        <Cover src={hit.episode?.cover} alt="" kind="podcast" size={36} fallbackId={hit.episode_id} />
      </span>
      <time class="time">{fmtDuration(hit.start)}</time>
      <span class="body">
        <span class="title" title={title}>{title}</span>
        <span class="excerpt">
          {#each highlightParts(hit.text, query) as part, k (k)}
            {#if part.mark}<mark>{part.t}</mark>{:else}{part.t}{/if}
          {/each}
        </span>
        {#if showChannel && channel}
          <span class="channel">{channel}</span>
        {/if}
      </span>
    </button>
  {/each}
</div>

<style>
  .hits {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .hit {
    display: grid;
    grid-template-columns: 36px 48px minmax(0, 1fr);
    gap: 10px;
    align-items: start;
    width: 100%;
    min-height: 72px;
    padding: 10px 8px;
    border-radius: 12px;
    text-align: left;
    color: var(--tx);
  }
  .hit:hover {
    background: var(--s2);
  }
  .hit:focus-visible {
    outline: 2px solid var(--ac-text);
    outline-offset: 2px;
  }
  .art {
    display: block;
    width: 36px;
    height: 36px;
    overflow: hidden;
    border-radius: 7px;
    background: var(--s2);
  }
  .time {
    padding-top: 2px;
    color: var(--dim);
    font-family: var(--font-mono);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    line-height: 1.5;
    white-space: nowrap;
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .title {
    font-weight: 650;
    font-size: 0.9rem;
    line-height: 1.3;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .excerpt {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    overflow: hidden;
    color: var(--dim);
    font-size: 0.84rem;
    line-height: 1.45;
    overflow-wrap: anywhere;
  }
  .channel {
    color: var(--dim);
    font-size: 0.75rem;
    line-height: 1.4;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  mark {
    background: var(--ac);
    color: var(--onac);
    border-radius: 2px;
    padding: 0 1px;
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
    border: 0;
  }
  @media (pointer: coarse) {
    .hit {
      min-height: 72px;
    }
  }
</style>
