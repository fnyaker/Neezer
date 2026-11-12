<script>
  // "Back" for detail pages (album / artist / playlist / show…): mounted by the
  // TopBar, so it sits in the shell's flow instead of floating over the page.
  // Hash routing keeps real browser history, so history.back() does the right
  // thing; if there's nothing to go back to (deep link / first load), fall back
  // to Home.
  //
  // Only on detail pages (the ones you reach by tapping into something and from
  // which there's no nav entry to get back). Top-level routes stay clean — they
  // are one tap away in the nav, and a back arrow there would have nowhere to
  // go but Home, where you already are.
  import { push } from "svelte-spa-router";
  import { location } from "../lib/router.js";
  import Icon from "./Icon.svelte";

  function back() {
    if (window.history.length > 1) window.history.back();
    else push("/");
  }

  const DETAIL = ["/album/", "/artist/", "/playlist/", "/mix/", "/podcast/"];
  $: show = DETAIL.some((p) => $location.startsWith(p));
</script>

{#if show}
  <button class="back" on:click={back} aria-label="Retour" title="Retour">
    <Icon name="chevronLeft" size={18} />
  </button>
{/if}

<style>
  .back {
    width: 36px;
    height: 36px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    border: 1px solid var(--ln);
    background: var(--s1);
    color: var(--tx);
  }
  .back:hover {
    background: var(--s2);
  }
  /* §10: a target is 44 px under a finger. */
  @media (pointer: coarse) {
    .back {
      width: 44px;
      height: 44px;
    }
  }
</style>
