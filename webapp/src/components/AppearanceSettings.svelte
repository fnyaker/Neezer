<script>
  // Réglages › Apparence (DESIGN §6, prototype): the three Sillon themes, each
  // shown as a small picture of itself. Choosing one writes `ui.theme` through
  // lib/theme.js, which flips `data-theme` on <html> — one attribute, so the
  // whole app repaints at once and no screen reloads or refetches anything.
  import { theme, setTheme, THEMES, THEME_INFO } from "../lib/theme.js";
  import Icon from "./Icon.svelte";

  // The swatches are LITERAL on purpose: each one shows a theme other than the
  // one on screen, so it cannot read the live tokens. The values are app.css's
  // (DESIGN §1); Pochette's accent is the live `--cover-ac` — the colour the
  // theme would paint this very second — with the designed #FF6B4A fallback.
  const PREVIEW = {
    encre: { bg: "#0C0C0B", s1: "#151513", tx: "#F1EFE8", ac: "#D2FF3C" },
    papier: { bg: "#F3F0E8", s1: "#FBF9F4", tx: "#14130F", ac: "#D2FF3C" },
    pochette: { bg: "#0F0E10", s1: "#18171A", tx: "#F4F1F5", ac: "var(--cover-ac, #FF6B4A)" },
  };
  $: cards = THEMES.map((id) => ({ id, ...THEME_INFO[id], ...PREVIEW[id] }));
</script>

<section class="card">
  <p class="intro">
    Choisissez l'ambiance. Le thème est retenu sur cet appareil et s'applique tout de
    suite&nbsp;: rien n'est rechargé, rien n'est renvoyé au serveur.
  </p>

  <div class="grid">
    {#each cards as c (c.id)}
      <button
        class="tcard"
        class:sel={$theme === c.id}
        aria-pressed={$theme === c.id}
        on:click={() => setTheme(c.id)}
        style={`--p-bg:${c.bg};--p-s1:${c.s1};--p-tx:${c.tx};--p-ac:${c.ac}`}
      >
        <span class="sw" aria-hidden="true">
          <span class="sheet">
            <span class="line l1"></span>
            <span class="line l2"></span>
          </span>
          <span class="dot"></span>
        </span>
        <span class="meta">
          <span class="name">{c.label}</span>
          {#if $theme === c.id}
            <span class="chosen"><Icon name="check" size={12} /> Choisi</span>
          {/if}
        </span>
        <span class="hint">{c.hint}</span>
      </button>
    {/each}
  </div>

  <p class="note">
    <strong>Pochette</strong> suit la pochette qui joue&nbsp;: l'accent — boutons, sélections,
    progression — reprend sa couleur dominante et change à chaque titre. Une pochette
    sans couleur franche laisse l'orange de repli.
  </p>
</section>

<style>
  .card {
    padding: 22px;
    margin-bottom: 16px;
    /* The theme row sizes itself on THIS card's width, not the window's: the
       desktop's 200 px menu leaves a 404 px content column at a 900 px window
       (three 96 px cards, the badge over the next name) and 752 at 1280. */
    container-type: inline-size;
  }
  .intro {
    margin: 0;
    color: var(--dim);
    font-size: 14px;
    line-height: 1.5;
    max-width: 56ch;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 14px;
    margin-top: 16px;
  }
  /* Three side by side need 3 x 145 + 28 = 463 px; below that the cards stack
     instead of shrinking to 96 px (404 px of content at a 900 px window, 484
     at a 1024 one — the two cases the fixed 200 px menu creates). */
  @container (max-width: 462px) {
    .grid {
      grid-template-columns: 1fr;
    }
    /* A stacked card is as wide as the page: a 96 px band under a 360 px card
       read as a progress bar, not as a screen. */
    .sw {
      height: 112px;
    }
  }
  /* No container queries: the same floor, applied by the grid itself. */
  @supports not (container-type: inline-size) {
    .grid {
      grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
    }
  }
  /* A real button, 2 px border on both states so selecting one never shifts
     the row. The border is --btn, not --ac: on Papier the accent is lemon on
     a near-white card and the selected one would be invisible. */
  .tcard {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    min-height: 44px;
    border-radius: 18px;
    border: 2px solid var(--ln);
    background: var(--s2);
    color: var(--tx);
    text-align: left;
  }
  .tcard:hover {
    border-color: var(--dim);
  }
  .tcard.sel {
    border-color: var(--btn);
  }
  /* The little screen: the theme's page, one of its cards, its ink, its accent
     (--bg / --s1 / --tx / --ac, DESIGN §6). */
  .sw {
    display: block;
    position: relative;
    height: 96px;
    padding: 12px;
    border-radius: 12px;
    background: var(--p-bg);
    color: var(--p-tx);
    overflow: hidden;
  }
  .sheet {
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 7px;
    height: 100%;
    padding: 10px;
    border-radius: 8px;
    background: var(--p-s1);
  }
  .line {
    display: block;
    border-radius: 4px;
    background: currentColor;
  }
  .l1 {
    width: 62%;
    height: 8px;
    opacity: 0.72;
  }
  .l2 {
    width: 42%;
    height: 6px;
    opacity: 0.36;
  }
  .dot {
    position: absolute;
    right: 22px;
    bottom: 22px;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: var(--p-ac);
  }
  /* The badge drops under the name rather than overflowing the card. */
  .meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 6px 8px;
    min-height: 24px;
  }
  .name {
    font-family: var(--font-display);
    font-weight: 700;
    font-size: 18px;
    letter-spacing: -0.02em;
  }
  .chosen {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex: none;
    height: 22px;
    padding: 0 8px;
    border-radius: 999px;
    background: var(--btn);
    color: var(--onbtn);
    font-family: var(--font-mono);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .hint {
    color: var(--dim);
    font-size: 13px;
    line-height: 1.35;
  }
  .note {
    margin: 16px 0 0;
    color: var(--dim);
    font-size: 13px;
    line-height: 1.5;
    max-width: 62ch;
  }
  .note strong {
    color: var(--tx);
    font-weight: 600;
  }
</style>
