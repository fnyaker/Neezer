<script>
  import { dominantColor, rgb } from "../lib/color.js";
  import { resolveCover } from "../lib/format.js";
  import { offlineCovers } from "../lib/stores.js";

  export let cover = null;

  let tint = null;
  let colorSeq = 0;

  // Resolve through the offline cover cache so the gradient still gets a real
  // color in airplane mode (a downloaded blob canvas isn't CORS-tainted).
  $: updateColor(resolveCover($offlineCovers, cover));

  async function updateColor(url) {
    // Sequence guard: on a cover change, a slower earlier extraction must not
    // overwrite the newer color.
    const mine = ++colorSeq;
    if (!url) {
      tint = null;
      return;
    }
    const c = await dominantColor(url);
    if (mine !== colorSeq) return;
    tint = rgb(c);
  }
</script>

<div class="gh" style={tint ? `--gh-c:${tint}` : ""}>
  <div class="gh-inner">
    <slot />
  </div>
</div>

<style>
  /* The header bleeds the page's own margins away, so it takes them from <main>
     (--page-y / --page-x, app.css) instead of repeating the numbers: they were
     written twice, and the day the page padding moved the header left a band
     of background above itself. */
  .gh {
    margin: calc(-1 * var(--page-y, 28px)) calc(-1 * var(--page-x, 32px)) 8px;
    padding: 56px var(--page-x, 32px) 24px;
    background: linear-gradient(
      180deg,
      color-mix(in srgb, var(--gh-c) 90%, transparent) 0%,
      color-mix(in srgb, var(--gh-c) 45%, black) 55%,
      var(--bg) 100%
    );
    transition: --gh-c 600ms cubic-bezier(0.4, 0, 0.2, 1);
  }
  .gh-inner {
    display: flex;
    gap: 28px;
    align-items: flex-end;
  }
  @media (prefers-reduced-motion: reduce) {
    .gh {
      transition: none;
    }
  }
  @media (max-width: 640px) {
    .gh {
      padding: 40px var(--page-x, 20px) 18px;
    }
    .gh-inner {
      flex-direction: column;
      align-items: flex-start;
      gap: 16px;
    }
  }
</style>
