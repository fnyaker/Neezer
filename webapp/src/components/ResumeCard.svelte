<script>
  // "Reprendre": the home's one answer to "what was I listening to?" — the
  // sleeve at 96 px, the title, the artist and where the playhead sits, on a
  // raised card beside the hero (DESIGN §6). It shows the player's own queue,
  // so it is true even after a reload (the session is restored by stores.js).
  import { player, current, nowPlayingOpen } from "../lib/stores.js";
  import { artistLine, duration } from "../lib/format.js";
  import { trackRef } from "../lib/trackkey.js";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";

  $: track = $current;
  $: total = $player.duration || 0;
  $: pct = track && total ? Math.min(100, Math.max(0, ($player.currentTime / total) * 100)) : 0;
  $: at = duration($player.currentTime || 0);
  $: of = duration(total);
</script>

<aside class="resume" aria-label="Reprendre l'écoute">
  <span class="label-mono">Reprendre</span>
  {#if track}
    <button class="body" on:click={() => nowPlayingOpen.set(true)} title="Ouvrir le lecteur">
      <span class="art">
        <Cover
          src={track.album?.cover}
          alt={track.title}
          kind={track.podcast ? "podcast" : "album"}
          fallbackId={trackRef(track)}
        />
      </span>
      <span class="txt">
        <span class="t">{track.title}</span>
        <span class="a">{artistLine(track)}</span>
        {#if total}<span class="when mono">{at} · {of}</span>{/if}
      </span>
      <span class="go"><Icon name="maximize" size={16} /></span>
    </button>
    {#if total}
      <span class="track" aria-hidden="true"><span class="fill" style={`width:${pct}%`}></span></span>
    {/if}
  {:else}
    <div class="idle">
      <span class="idle-title">Rien en cours de lecture.</span>
      <span class="idle-sub">Choisis une énergie et lance le Flow, ou ouvre un titre depuis Explorer.</span>
      <a class="btn btn-ghost btn-sm" href="#/search"><Icon name="search" size={15} /> Explorer</a>
    </div>
  {/if}
</aside>

<style>
  .resume {
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-width: 0;
    padding: 20px;
    border-radius: 24px;
    border: 1px solid var(--ln);
    background: var(--s1);
    color: var(--tx);
  }
  .body {
    display: flex;
    align-items: center;
    gap: 16px;
    min-width: 0;
    padding: 0;
    text-align: left;
    color: inherit;
  }
  .art {
    flex: none;
    display: block;
    width: 96px;
  }
  .txt {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }
  .t {
    font-family: var(--font-display);
    font-weight: 600;
    font-size: 20px;
    letter-spacing: -0.01em;
    line-height: 1.18;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .a,
  .when {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .a {
    color: var(--dim);
    font-size: 14px;
  }
  .when {
    margin-top: 4px;
    color: var(--dim);
    font-size: 12px;
  }
  .go {
    flex: none;
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    border-radius: 50%;
    border: 1px solid var(--ln);
    color: var(--dim);
  }
  .body:hover .go,
  .body:focus-visible .go {
    color: var(--tx);
    border-color: var(--dim);
  }
  .track {
    display: block;
    margin-top: auto;
    height: 3px;
    border-radius: 999px;
    background: var(--ln);
    overflow: hidden;
  }
  .fill {
    display: block;
    height: 100%;
    border-radius: inherit;
    background: var(--ac);
  }
  .idle {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
    min-width: 0;
    margin-block: auto;
  }
  .idle-title {
    font-family: var(--font-display);
    font-weight: 600;
    font-size: 20px;
    letter-spacing: -0.01em;
  }
  .idle-sub {
    color: var(--dim);
    font-size: 14px;
    line-height: 1.4;
  }
  .idle .btn {
    margin-top: 4px;
  }
  @media (max-width: 640px) {
    .resume {
      padding: 16px;
    }
    .art {
      width: 76px;
    }
    .t {
      font-size: 18px;
    }
  }
</style>
