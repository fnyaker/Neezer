<script>
  import { dominantColor, rgb, darken } from "../lib/color.js";
  import { resolveCover } from "../lib/format.js";
  import { offlineCovers } from "../lib/stores.js";

  export let cover = null;

  let bg = "linear-gradient(180deg, #3a2a55, var(--bg) 70%)";
  let colorSeq = 0;

  // Resolve through the offline cover cache so the gradient still gets a real
  // color in airplane mode (a downloaded blob canvas isn't CORS-tainted).
  $: updateColor(resolveCover($offlineCovers, cover));

  async function updateColor(url) {
    // Sequence guard: on a cover change, a slower earlier extraction must not
    // overwrite the newer color.
    const mine = ++colorSeq;
    const c = await dominantColor(url);
    if (mine !== colorSeq) return;
    bg = `linear-gradient(180deg, ${rgb(c, 0.9)} 0%, ${rgb(darken(c, 0.55), 1)} 55%, var(--bg) 100%)`;
  }
</script>

<div class="gh" style="background:{bg}">
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
    transition: background 0.4s ease;
  }
  .gh-inner {
    display: flex;
    gap: 28px;
    align-items: flex-end;
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
