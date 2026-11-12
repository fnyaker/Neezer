<script>
  // One face, no photo? A constructor's initial disc — deterministic from the
  // name, from Sillon's eight groove pairs (lib/sillon.js, DESIGN §4). Same
  // artist, same colour, on every device: a placeholder that changed colour on
  // reload would read as a loading state.
  //
  // Used where a round portrait is the subject (the home's "Tes artistes", the
  // library's artist grid, the artist page), and square for a show.
  import { initialsOf, grooveTint } from "../lib/sillon.js";

  export let name = "";
  export let src = null;
  // Optional explicit alt; the name is the honest default for a portrait.
  export let alt = null;
  export let size = 104;
  export let round = true;

  let failed = false;
  // A new source gets a new chance (a recycled tile in a virtualised grid).
  $: if (src) failed = false;

  $: seed = String(name || src || "");
  $: tint = grooveTint(seed);
  $: initials = initialsOf(name);
  $: withImage = !!src && !failed;
</script>

<span
  class="av"
  class:round
  style={`width:${size}px;height:${size}px;font-size:${Math.max(11, Math.round(size * 0.29))}px;${
    withImage ? "" : `background-image:${tint}`
  }`}
  role={withImage ? undefined : name ? "img" : undefined}
  aria-label={!withImage && name ? name : undefined}
  aria-hidden={!withImage && !name ? "true" : undefined}
>
  {#if withImage}
    <img {src} alt={alt ?? name} loading="lazy" decoding="async" on:error={() => (failed = true)} />
  {:else}
    <span class="ini" aria-hidden="true">{initials}</span>
  {/if}
</span>

<style>
  .av {
    position: relative;
    display: grid;
    place-items: center;
    flex: none;
    overflow: hidden;
    border-radius: var(--r-cover);
    /* The eight groove pairs are dark by design, so the initials are always
       light — in Papier too, where --tx is ink and would vanish here. */
    color: #f1efe8;
    background-color: var(--s2);
    font-family: var(--font-display);
    font-weight: 700;
    line-height: 1;
    letter-spacing: -0.01em;
    user-select: none;
  }
  .av.round {
    border-radius: 50%;
  }
  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .ini {
    display: block;
  }
</style>
