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
  // A number is a fixed px size; a CSS length ("100%") lets a responsive grid
  // drive it. The initials scale from the fixed size only.
  export let size = 104;
  export let round = true;

  let failed = false;
  // A new source gets a new chance (a recycled tile in a virtualised grid).
  $: if (src) failed = false;

  $: seed = String(name || src || "");
  $: tint = grooveTint(seed);
  $: initials = initialsOf(name);
  $: withImage = !!src && !failed;
  $: dim = typeof size === "number" ? `${size}px` : size;
  $: fontSize = Math.max(11, Math.round((typeof size === "number" ? size : 104) * 0.29));
</script>

<span
  class="av"
  class:round
  style={`width:${dim};height:${dim};font-size:${fontSize}px;background-image:${tint}`}
  role={name ? "img" : undefined}
  aria-label={name || undefined}
>
  <!-- The discs and initials are UNDER the photo, never a state that waits for
       a 404: on a slow link the artist is still an initialled disc while the
       image is in flight. A loaded photo simply covers it. -->
  <span class="ini" aria-hidden="true">{initials}</span>
  {#if withImage}
    <img {src} alt={alt ?? name} loading="lazy" decoding="async" on:error={() => (failed = true)} />
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
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    z-index: 0;
  }
  img {
    position: relative;
    z-index: 1;
  }
</style>
