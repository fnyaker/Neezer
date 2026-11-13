<script>
  // Réglages › Apparence (DESIGN §6, prototype): the three Sillon themes, each
  // shown as a small picture of itself. Choosing one writes `ui.theme` through
  // lib/theme.js, which flips `data-theme` on <html> — one attribute, so the
  // whole app repaints at once and no screen reloads or refetches anything.
  import {
    theme,
    setTheme,
    THEMES,
    THEME_INFO,
    customAccent,
    setCustomAccent,
    clearCustomAccent,
    accentVars,
    normalizeAccent,
    themeAccentOf,
  } from "../lib/theme.js";
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

  // -- the person's own accent ----------------------------------------------
  // `hex` is what the FIELD shows (six characters, no #); `$customAccent` is
  // the committed preference. A partial or invalid entry never reaches
  // localStorage nor the app: the preview keeps the last valid colour and the
  // field says what is wrong. The person may type a colour faster than six
  // keystrokes; only the sixth one applies.
  let hex = "";
  let editing = false;
  let bad = false;
  let error = "";
  let synced; // the store value the field was last filled from
  let seededTheme; // the theme whose accent last seeded the field

  $: seed = $customAccent || themeAccentOf($theme);
  $: customActive = !!$customAccent && $theme !== "pochette";
  // Pochette keeps the sleeve's accent, so the preview shows the same colour
  // on the dark theme it WILL apply to (Encre) rather than pretending it is on.
  $: previewTheme = $theme === "pochette" ? "encre" : $theme;
  $: pair = accentVars(seed, previewTheme) || accentVars(themeAccentOf("encre"), "encre");
  $: correction = pair.tooDark
    ? "Trop sombre pour Encre\u00a0: les textes et icônes d'accent prennent une variante éclaircie (4,5:1). Le bouton garde votre couleur, son libellé reste noir ou blanc."
    : pair.tooLight
      ? "Trop claire pour Papier\u00a0: les textes et icônes d'accent prennent une variante assombrie (4,5:1). Le bouton garde votre couleur, son libellé reste noir ou blanc."
      : "";

  // Keep the field in step with the store and the theme — but never overwrite
  // what somebody is typing. A blur is handled by onBlur(), which either
  // commits or keeps the invalid text visible with its reason.
  $: if ($customAccent !== synced) {
    synced = $customAccent;
    if (!editing) {
      hex = seed.slice(1).toUpperCase();
      bad = false;
      error = "";
    }
  }
  $: if (!editing && !$customAccent && seededTheme !== $theme) {
    seededTheme = $theme;
    hex = themeAccentOf($theme).slice(1).toUpperCase();
    bad = false;
    error = "";
  }

  function commitHex() {
    hex = hex.replace(/^#/, "").trim().toUpperCase();
    const value = normalizeAccent(hex);
    if (!value) {
      bad = true;
      error = "Couleur invalide\u00a0: six chiffres hexadécimaux, par exemple D2FF3C.";
      return;
    }
    setCustomAccent(value);
    hex = value.slice(1);
    bad = false;
    error = "";
  }

  function onHexInput(e) {
    // The visible field carries the leading # as a separate glyph; strip one
    // from a pasted value so it never reads "##D2FF3C". Keep the case as typed
    // (CSS uppercases it): rewriting the DOM value on every keystroke would
    // move the caret when somebody edits the middle of the code.
    hex = e.currentTarget.value.replace(/^#/, "");
    bad = false;
    error = "";
    // Only a complete six-digit colour applies; anything shorter is simply
    // not a colour yet, not an error to shout about while typing.
    const m = /^([0-9a-f]{6})$/i.exec(hex.trim());
    if (m) setCustomAccent("#" + m[1]);
  }

  function onBlur() {
    editing = false;
    if (!hex.trim()) {
      revert();
      return;
    }
    commitHex();
  }

  function onKey(e) {
    if (e.key === "Enter") {
      e.preventDefault();
      commitHex();
    } else if (e.key === "Escape") {
      e.preventDefault();
      revert();
    }
  }

  function revert() {
    hex = seed.slice(1).toUpperCase();
    bad = false;
    error = "";
  }

  function onPick(e) {
    const value = normalizeAccent(e.currentTarget.value);
    if (!value) return;
    setCustomAccent(value);
    hex = value.slice(1);
    bad = false;
    error = "";
  }

  function reset() {
    clearCustomAccent();
    hex = themeAccentOf($theme).slice(1).toUpperCase();
    seededTheme = $theme;
    bad = false;
    error = "";
  }
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

  <div class="custom">
    <div class="custom-head">
      <h2 class="custom-title">Couleur personnalisée</h2>
      <span class="badge label-mono" class:on={customActive}>
        {$customAccent ? (customActive ? "Active" : "Mémorisée") : "Thème"}
      </span>
    </div>
    <p class="custom-intro" id="accent-help">
      Une couleur à vous pour les boutons, le cœur et les sélections. Elle s'applique à
      <strong>Encre</strong> et <strong>Papier</strong>&nbsp;; <strong>Pochette</strong> garde la
      couleur de la pochette qui joue. Le texte posé dessus passe en noir ou en blanc, et les
      textes d'accent sont ajustés pour tenir 4,5:1.
    </p>

    <div class="picker">
      <label class="swatch" title="Choisir une couleur">
        <input
          type="color"
          value={seed}
          on:input={onPick}
          aria-label="Choisir la couleur personnalisée"
          aria-describedby="accent-help"
        />
      </label>
      <div class="hexbox" class:bad>
        <span class="hash" aria-hidden="true">#</span>
        <input
          type="text"
          value={hex}
          spellcheck="false"
          autocomplete="off"
          inputmode="text"
          maxlength="7"
          aria-label="Code hexadécimal de la couleur, sans le dièse"
          aria-invalid={bad}
          aria-describedby={bad ? "accent-error accent-help" : "accent-help"}
          on:input={onHexInput}
          on:focus={() => (editing = true)}
          on:blur={onBlur}
          on:keydown={onKey}
        />
      </div>
      <button
        class="btn btn-ghost reset"
        on:click={reset}
        disabled={!$customAccent}
        aria-label="Réinitialiser la couleur du thème"
      >
        <Icon name="refresh" size={16} />
        Réinitialiser
      </button>
    </div>

    <div class="feedback">
      {#if bad}
        <p class="msg error" id="accent-error" role="alert">
          <Icon name="alert" size={15} />
          <span>{error}</span>
        </p>
      {:else if correction}
        <p class="msg" role="status">
          <Icon name="info" size={15} />
          <span>{correction}</span>
        </p>
      {:else}
        <p class="msg quiet">
          <Icon name={$customAccent ? "check" : "info"} size={15} />
          <span>
            {#if !$customAccent}
              Aucune couleur personnalisée&nbsp;: le thème garde son accent.
            {:else if customActive}
              Contraste vérifié&nbsp;: le texte tient 4,5:1 sur cette couleur.
            {:else}
              Mémorisée pour Encre et Papier&nbsp;; Pochette garde la couleur de la pochette.
            {/if}
          </span>
        </p>
      {/if}
    </div>

    <!-- Decorative: the field and the feedback already say the values. A real
         rendering of the pair -- surface, its on-text, and the ink that the
         heart and the marks use -- is what makes the correction visible. On
         Pochette the colour is dormant, so the preview names the theme it will
         apply to instead of pretending it is on screen. -->
    {#if $theme === "pochette"}
      <p class="pv-note label-mono">Aperçu sur Encre</p>
    {/if}
    <div
      class="preview"
      style={`--pv:${pair.surface};--pv-on:${pair.on};--pv-ink:${pair.ink}`}
      aria-hidden="true"
    >
      <span class="pv-btn">Lire</span>
      <span class="pv-heart"><Icon name="heartFilled" size={19} /></span>
      <span class="pv-dot"></span>
      <span class="pv-code mono">{pair.surface}</span>
    </div>
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

  /* -- Couleur personnalisée ------------------------------------------------ */
  /* Separated by a rule, not a second card: it is one more setting on the same
     screen, not another screen with its own frame. */
  .custom {
    margin-top: 20px;
    padding-top: 18px;
    border-top: 1px solid var(--ln);
  }
  .custom-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  /* The global h2 margin (1.5em above) would double the card's padding over
     this title; a card's title is interface text, not a masthead (DESIGN §6). */
  .custom-title {
    margin: 0;
    font-family: var(--font-body);
    font-size: 16px;
    font-weight: 600;
    letter-spacing: 0;
    line-height: 1.3;
  }
  .badge {
    flex: none;
    display: inline-flex;
    align-items: center;
    height: 22px;
    padding: 0 9px;
    border-radius: 999px;
    border: 1px solid var(--ln);
    background: var(--s2);
    color: var(--dim);
    font-size: 10px;
  }
  .badge.on {
    border-color: transparent;
    background: var(--ac);
    color: var(--onac);
  }
  .custom-intro {
    margin: 8px 0 0;
    color: var(--dim);
    font-size: 13.5px;
    line-height: 1.5;
    max-width: 62ch;
  }
  .custom-intro strong {
    color: var(--tx);
    font-weight: 600;
  }
  .picker {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 10px;
    margin-top: 14px;
  }
  /* 46 px, not the desktop's 40: this is a colour control a thumb has to hit,
     and the field beside it is a text target. Both keep 44 under a finger and
     here they keep it on a mouse too. */
  .swatch {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: none;
    width: 46px;
    height: 46px;
    padding: 0;
    overflow: hidden;
    border: 1px solid var(--ln);
    border-radius: 14px;
    background: var(--s2);
    cursor: pointer;
  }
  .swatch input[type="color"] {
    width: 100%;
    height: 100%;
    padding: 0;
    border: 0;
    border-radius: 13px;
    background: none;
    cursor: pointer;
    -webkit-appearance: none;
    appearance: none;
  }
  .swatch input[type="color"]::-webkit-color-swatch-wrapper {
    padding: 4px;
  }
  .swatch input[type="color"]::-webkit-color-swatch {
    border: 0;
    border-radius: 10px;
  }
  .swatch input[type="color"]::-moz-color-swatch {
    border: 0;
    border-radius: 10px;
  }
  .hexbox {
    display: flex;
    align-items: center;
    flex: 1 1 170px;
    min-width: 0;
    height: 46px;
    padding: 0 16px;
    border: 1px solid var(--ln);
    border-radius: 999px;
    background: var(--s1);
    font-family: var(--font-mono);
    font-size: 15px;
    letter-spacing: 0.05em;
  }
  .hexbox:focus-within {
    border-color: var(--ac-text);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--ac-text) 32%, transparent);
  }
  .hexbox.bad {
    border-color: var(--tx);
    box-shadow: 0 0 0 1px var(--tx);
  }
  .hexbox .hash {
    color: var(--dim);
  }
  .hexbox input {
    width: 100%;
    min-width: 0;
    padding: 0;
    border: 0;
    outline: none;
    background: none;
    color: var(--tx);
    font: inherit;
    letter-spacing: inherit;
    text-transform: uppercase;
  }
  /* The container draws the focus ring; the input's own outline would sit
     inside it and draw a second rounded box. */
  .hexbox input:focus-visible {
    outline: none;
  }
  .reset {
    flex: none;
    height: 46px;
    padding: 0 16px;
  }
  /* Below the width of the three cards the reset is a full-width action under
     the field rather than a chip squeezed beside it. */
  @container (max-width: 420px) {
    .reset {
      flex: 1 1 100%;
    }
  }
  .feedback {
    margin-top: 10px;
    min-height: 20px;
  }
  .pv-note {
    margin: 12px 0 0;
    font-size: 10px;
  }
  .msg {
    display: flex;
    align-items: flex-start;
    gap: 7px;
    margin: 0;
    color: var(--tx);
    font-size: 12.5px;
    line-height: 1.45;
    max-width: 66ch;
  }
  .msg :global(svg) {
    flex: none;
    margin-top: 1px;
  }
  .msg.quiet {
    color: var(--dim);
  }
  .msg.error {
    font-weight: 600;
  }
  /* The live pair: the button uses the surface and its on-text, the heart and
     the dot use the ink (what --hrt and --ac-mark actually paint in the app). */
  .preview {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 14px;
    margin-top: 10px;
    padding: 12px 14px;
    border: 1px solid var(--ln);
    border-radius: 16px;
    background: var(--s2);
  }
  .pv-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    height: 38px;
    padding: 0 18px;
    border: 1px solid var(--ln);
    border-radius: 999px;
    background: var(--pv);
    color: var(--pv-on);
    font-size: 14px;
    font-weight: 700;
  }
  .pv-heart {
    display: inline-flex;
    color: var(--pv-ink);
  }
  .pv-dot {
    width: 22px;
    height: 22px;
    border: 1px solid var(--ln);
    border-radius: 50%;
    background: var(--pv);
  }
  .pv-code {
    margin-left: auto;
    color: var(--dim);
    font-size: 12px;
    letter-spacing: 0.08em;
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
  @media (prefers-reduced-motion: reduce) {
    .tcard:active,
    .reset:active {
      transform: none;
    }
  }
</style>
