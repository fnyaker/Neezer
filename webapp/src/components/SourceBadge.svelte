<script>
  // What a YouTube track REALLY plays at, next to the quality badge, when the
  // badge asks for more than YouTube has (FLAC, Opus 320…). The badge stays the
  // setting; this says the truth about the one track that cannot honour it.
  import { trackKey } from "../lib/trackkey.js";
  import { current, player, quality } from "../lib/stores.js";
  import { isYoutube, realBitrate, realBitrateBadge } from "../lib/youtube.js";

  let real = 0;
  // Keyed on the id as a STRING: `current` is handed over again on every
  // progress tick, and a statement reading the object would ask on each one.
  $: id = $current && isYoutube($current) ? trackKey($current) : null;
  // Asked once the audio is running: the first play is what archives the
  // file, so before that there is nothing to measure.
  $: playing = !!id && $player.currentTime > 0.5;
  let askedFor = null;
  $: if (id !== askedFor) {
    real = 0;
    if (!id) askedFor = null;
  }
  $: if (playing && askedFor !== id) {
    askedFor = id;
    const mine = id;
    realBitrate($current).then((k) => {
      if (mine === askedFor) real = k;
    });
  }
  $: badge = id ? realBitrateBadge($current, $quality, real) : null;
</script>

{#if badge}
  <span class="src mono" title={badge.title}>{badge.short}</span>
{/if}

<style>
  .src {
    display: inline-flex;
    align-items: center;
    flex: none;
    font-size: 10px;
    font-weight: 500;
    letter-spacing: 0.06em;
    line-height: 1;
    padding: 5px 7px;
    border-radius: 6px;
    color: var(--dim);
    border: 1px solid var(--ln);
    white-space: nowrap;
    cursor: help;
  }
</style>
