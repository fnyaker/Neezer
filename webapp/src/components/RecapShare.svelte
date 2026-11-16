<script>
  // The Réçap poster: a 1080×1350 (4:5) PNG drawn on a canvas, shared with the
  // Web Share API and never fetched.
  //
  // Why 4:5: the same frame as the lyric card — Instagram's documented feed
  // range is 1.91:1 to 4:5, and 1080×1350 is the portrait size that fills a
  // feed post without a crop (Instagram Help "Image resolution of photos you
  // share"; Buffer's 2026 Instagram size guide).
  //
  // The sleeves come ONLY from `offlineCovers` (the device's own cache of the
  // server's archived art). A missing one is Sillon's deterministic groove, so
  // the result is identical on Wi-Fi and on a dead mobile link: zero /api and
  // zero cover requests here.
  import { onMount, onDestroy, tick, createEventDispatcher } from "svelte";
  import { offlineCovers, toasts } from "../lib/stores.js";
  import { bytes as fmtBytes, coverKey } from "../lib/format.js";
  import { fitOneLine } from "../lib/lyriccard.js";
  import { grooveFor } from "../lib/sillon.js";
  import { mosaicCovers, posterLines, shareText } from "../lib/recap.js";
  import Icon from "./Icon.svelte";
  // The share sheet is an overlay like every other (M8): its veil fades its own
  // background alpha, its pane a `sheet`; and being `position:fixed` inside a
  // page it is portalled to `<body>` (M9). Both spinner marks are status.
  import { fadeIn, sheet, veil } from "../lib/motion.js";
  import { toBody } from "../lib/portal.js";

  export let payload;

  const dispatch = createEventDispatcher();

  const CARD_W = 1080;
  const CARD_H = 1350;
  const MARGIN = 96;
  const INK = "#0C0C0B";
  const CREAM = "#F7F4EC";
  const LEMON = "#D2FF3C";

  const year = payload && payload.year;
  const lines = posterLines(payload);
  const filename = `neezer-recap-${year || "annee"}.png`;
  const titleId = "recap-share-title";

  let canvasEl = null;
  let sheetEl = null;
  let closeEl = null;
  let lastFocus = null;
  let destroyed = false;
  let mounted = false;
  let preparing = false;
  let busy = false;
  let file = null;
  let fileBytes = 0;
  let renderError = "";
  let pendingPrepare = null;
  let prepareSeq = 0;
  let prepareTimer = 0;
  let renderRaf = 0;
  let images = [];
  let loadedKey = "";

  $: coverUrls = mosaicCovers(payload).map((url) => $offlineCovers[coverKey(url)] || null);
  $: if (mounted) syncCovers(coverUrls);
  $: sizeLabel = fileBytes ? fmtBytes(fileBytes) : "";
  $: previewLabel = `Aperçu de l’affiche du récap ${year || ""}`;
  $: shownText = shareText(payload);

  onMount(async () => {
    lastFocus = document.activeElement;
    mounted = true;
    await tick();
    (closeEl || sheetEl)?.focus();
    queueRender();
    await loadFonts();
    if (destroyed) return;
    queueRender();
  });

  onDestroy(() => {
    destroyed = true;
    clearTimeout(prepareTimer);
    cancelAnimationFrame(renderRaf);
    prepareSeq += 1;
    try {
      lastFocus?.focus?.();
    } catch {
      /* the opener may be gone */
    }
  });

  async function loadFonts() {
    try {
      if (typeof document === "undefined" || !document.fonts) return;
      const load = Promise.all([
        document.fonts.load('800 104px "Bricolage Grotesque"'),
        document.fonts.load('600 60px "Bricolage Grotesque"'),
        document.fonts.load('400 38px "Instrument Sans"'),
        document.fonts.load('500 26px "JetBrains Mono"'),
      ]);
      await Promise.race([load, new Promise((resolve) => setTimeout(resolve, 1400))]);
    } catch {
      /* the system stack is already usable */
    }
  }

  function close() {
    if (busy) return;
    dispatch("close");
  }

  function onKey(e) {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
      return;
    }
    if (e.key !== "Tab" || !sheetEl) return;
    if (!sheetEl.contains(document.activeElement)) return;
    const focusable = [
      ...sheetEl.querySelectorAll(
        'button:not([disabled]), [href], [tabindex]:not([tabindex="-1"])'
      ),
    ].filter((el) => el.getClientRects().length > 0);
    if (!focusable.length) {
      e.preventDefault();
      sheetEl.focus();
      return;
    }
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (e.shiftKey && (document.activeElement === first || document.activeElement === sheetEl)) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  }

  function onBackdrop(e) {
    if (e.target === e.currentTarget) close();
  }

  // -- sleeves, from the device only -----------------------------------------
  function syncCovers(urls) {
    const key = urls.join("|");
    if (key === loadedKey) return;
    loadedKey = key;
    images = [];
    for (let i = 0; i < urls.length; i++) {
      const url = urls[i];
      if (!url) continue;
      const img = new Image();
      img.decoding = "async";
      img.onload = () => {
        if (destroyed || loadedKey !== key) return;
        images[i] = img;
        queueRender();
      };
      img.onerror = () => {
        /* a missing tile stays a groove */
      };
      img.src = url;
    }
    queueRender();
  }

  // -- the poster -------------------------------------------------------------
  function queueRender() {
    if (destroyed || renderRaf) return;
    renderRaf = requestAnimationFrame(() => {
      renderRaf = 0;
      draw();
    });
  }

  function pathRoundRect(ctx, x, y, w, h, r) {
    const radius = Math.min(r, w / 2, h / 2);
    if (typeof ctx.roundRect === "function") {
      ctx.roundRect(x, y, w, h, radius);
      return;
    }
    ctx.moveTo(x + radius, y);
    ctx.arcTo(x + w, y, x + w, y + h, radius);
    ctx.arcTo(x + w, y + h, x, y + h, radius);
    ctx.arcTo(x, y + h, x, y, radius);
    ctx.arcTo(x, y, x + w, y, radius);
    ctx.closePath();
  }

  function draw() {
    const canvas = canvasEl;
    if (!canvas || destroyed) return;
    let ctx = null;
    try {
      ctx = canvas.getContext("2d");
    } catch {
      ctx = null;
    }
    if (!ctx) {
      renderError = "L'affiche ne peut pas être dessinée sur cet appareil.";
      return;
    }
    try {
      ctx.setTransform(1, 0, 0, 1, 0, 0);
      ctx.globalAlpha = 1;
      if (typeof ctx.filter === "string") ctx.filter = "none";
      ctx.clearRect(0, 0, CARD_W, CARD_H);

      // Ground: ink with the brand's lemon as light, never as the surface the
      // cream words are read on.
      ctx.fillStyle = INK;
      ctx.fillRect(0, 0, CARD_W, CARD_H);
      const glow = ctx.createRadialGradient(
        CARD_W * 0.88,
        CARD_H * 0.08,
        0,
        CARD_W * 0.88,
        CARD_H * 0.08,
        CARD_W
      );
      glow.addColorStop(0, "rgba(210,255,60,0.2)");
      glow.addColorStop(0.55, "rgba(210,255,60,0.05)");
      glow.addColorStop(1, "rgba(210,255,60,0)");
      ctx.fillStyle = glow;
      ctx.fillRect(0, 0, CARD_W, CARD_H);

      drawTop(ctx);
      drawWords(ctx);
      drawMosaic(ctx, 560);
      drawFacts(ctx, 840);
      drawBottom(ctx);
      renderError = "";
      invalidateFile();
      schedulePrepare();
    } catch {
      // A tainted canvas or a policy that refuses the drawing: say it, never a
      // preview that silently cannot be saved.
      renderError = "Le navigateur a refusé l'image (CSP). Enregistrement impossible.";
      file = null;
      fileBytes = 0;
    }
  }

  function drawTop(ctx) {
    ctx.textBaseline = "top";
    ctx.textAlign = "left";
    // The mark: Sillon's disc, lemon label, ink centre.
    const cx = MARGIN + 17;
    const cy = 114;
    ctx.beginPath();
    ctx.arc(cx, cy, 17, 0, Math.PI * 2);
    ctx.fillStyle = CREAM;
    ctx.fill();
    ctx.beginPath();
    ctx.arc(cx, cy, 9, 0, Math.PI * 2);
    ctx.strokeStyle = INK;
    ctx.lineWidth = 2;
    ctx.stroke();
    ctx.beginPath();
    ctx.arc(cx, cy, 3.5, 0, Math.PI * 2);
    ctx.fillStyle = LEMON;
    ctx.fill();

    ctx.fillStyle = CREAM;
    ctx.font = '600 32px "Bricolage Grotesque", "Instrument Sans", system-ui, sans-serif';
    ctx.fillText("neezer", MARGIN + 46, 96);

    ctx.textAlign = "right";
    ctx.fillStyle = "rgba(247,244,236,0.62)";
    ctx.font = '500 24px "JetBrains Mono", ui-monospace, monospace';
    ctx.fillText(`RÉCAP ${year || ""}`, CARD_W - MARGIN, 100);
    ctx.textAlign = "left";
  }

  function drawWords(ctx) {
    ctx.textAlign = "left";
    ctx.textBaseline = "top";
    const maxWidth = CARD_W - MARGIN * 2;
    // lines[0] is the year, lines[1] the minutes, lines[2] the listens.
    const title = (lines[0] || "").trim();
    const minutes = (lines[1] || "").trim();
    const listens = (lines[2] || "").trim();

    ctx.fillStyle = "rgba(247,244,236,0.84)";
    ctx.font = '600 56px "Bricolage Grotesque", "Instrument Sans", system-ui, sans-serif';
    ctx.fillText(fitOneLine(ctx, title, maxWidth), MARGIN, 196);

    ctx.fillStyle = LEMON;
    ctx.font = '800 102px "Bricolage Grotesque", "Instrument Sans", system-ui, sans-serif';
    ctx.fillText(fitOneLine(ctx, minutes, maxWidth), MARGIN, 286);

    ctx.fillStyle = "rgba(247,244,236,0.72)";
    ctx.font = '400 38px "Instrument Sans", system-ui, sans-serif';
    ctx.fillText(fitOneLine(ctx, listens, maxWidth), MARGIN, 432);
  }

  function drawMosaic(ctx, top) {
    const gap = 16;
    const size = (CARD_W - MARGIN * 2 - gap * 3) / 4;
    const urls = mosaicCovers(payload);
    for (let i = 0; i < 4; i++) {
      const x = MARGIN + i * (size + gap);
      ctx.save();
      ctx.beginPath();
      pathRoundRect(ctx, x, top, size, size, 18);
      ctx.clip();
      const img = images[i];
      if (img && img.naturalWidth && img.naturalHeight) {
        drawCoverFill(ctx, img, x, top, size, size);
      } else {
        drawGroove(ctx, x, top, size, urls[i] || `recap-${year}-${i}`);
      }
      ctx.restore();
      ctx.beginPath();
      pathRoundRect(ctx, x + 0.5, top + 0.5, size - 1, size - 1, 17.5);
      ctx.strokeStyle = "rgba(247,244,236,0.16)";
      ctx.lineWidth = 1;
      ctx.stroke();
    }
  }

  function drawCoverFill(ctx, img, x, y, w, h) {
    const iw = img.naturalWidth || img.width || 0;
    const ih = img.naturalHeight || img.height || 0;
    if (!iw || !ih) return;
    const ir = iw / ih;
    const br = w / h;
    let sx = 0;
    let sy = 0;
    let sw = iw;
    let sh = ih;
    if (ir > br) {
      sw = ih * br;
      sx = (iw - sw) / 2;
    } else {
      sh = iw / br;
      sy = (ih - sh) / 2;
    }
    ctx.drawImage(img, sx, sy, sw, sh, x, y, w, h);
  }

  // Sillon's record groove at tile scale: the same deterministic pair the
  // missing-cover tile uses (lib/sillon.js), so the poster looks deliberate.
  function drawGroove(ctx, x, y, size, seed) {
    const [a, b] = grooveFor(String(seed));
    ctx.fillStyle = a;
    ctx.fillRect(x, y, size, size);
    const cx = x + size / 2;
    const cy = y + size / 2;
    ctx.strokeStyle = b;
    ctx.globalAlpha = 0.62;
    ctx.lineWidth = 1.6;
    for (let r = size * 0.44; r > size * 0.12; r -= size * 0.07) {
      ctx.beginPath();
      ctx.arc(cx, cy, r, 0, Math.PI * 2);
      ctx.stroke();
    }
    ctx.globalAlpha = 1;
    ctx.beginPath();
    ctx.arc(cx, cy, size * 0.12, 0, Math.PI * 2);
    ctx.fillStyle = LEMON;
    ctx.fill();
    ctx.beginPath();
    ctx.arc(cx, cy, size * 0.035, 0, Math.PI * 2);
    ctx.fillStyle = INK;
    ctx.fill();
  }

  function drawFacts(ctx, top) {
    const rest = lines.slice(3);
    if (!rest.length) return;
    const maxWidth = CARD_W - MARGIN * 2;
    let y = top;
    rest.forEach((line, i) => {
      const last = i === rest.length - 1;
      ctx.fillStyle = last ? "rgba(247,244,236,0.48)" : "rgba(247,244,236,0.78)";
      ctx.font = last
        ? '500 26px "JetBrains Mono", ui-monospace, monospace'
        : '400 34px "Instrument Sans", system-ui, sans-serif';
      ctx.fillText(fitOneLine(ctx, line, maxWidth), MARGIN, y);
      y += last ? 40 : 54;
    });
  }

  function drawBottom(ctx) {
    const y = CARD_H - 132;
    ctx.strokeStyle = "rgba(247,244,236,0.22)";
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(MARGIN, y);
    ctx.lineTo(CARD_W - MARGIN, y);
    ctx.stroke();

    ctx.textBaseline = "top";
    ctx.fillStyle = "rgba(247,244,236,0.7)";
    ctx.font = '500 26px "JetBrains Mono", ui-monospace, monospace';
    ctx.fillText(String(year || ""), MARGIN, y + 26);

    ctx.textAlign = "right";
    ctx.fillStyle = "rgba(247,244,236,0.86)";
    ctx.font = '600 28px "Bricolage Grotesque", "Instrument Sans", system-ui, sans-serif';
    ctx.fillText("neezer", CARD_W - MARGIN, y + 22);
    ctx.textAlign = "left";
  }

  // -- PNG, share, save -------------------------------------------------------
  function invalidateFile() {
    file = null;
    fileBytes = 0;
    pendingPrepare = null;
    prepareSeq += 1;
  }
  function schedulePrepare() {
    if (destroyed) return;
    clearTimeout(prepareTimer);
    prepareTimer = setTimeout(() => prepareNow(), 110);
  }
  function prepareNow() {
    if (!canvasEl) {
      file = null;
      fileBytes = 0;
      return Promise.resolve(null);
    }
    const mine = ++prepareSeq;
    preparing = true;
    const p = new Promise((resolve) => {
      let settled = false;
      const done = (blob) => {
        if (settled) return;
        settled = true;
        if (destroyed || mine !== prepareSeq) {
          resolve(file);
          return;
        }
        preparing = false;
        if (blob && blob.size) {
          file = new File([blob], filename, { type: "image/png" });
          fileBytes = blob.size;
          renderError = "";
          resolve(file);
        } else {
          file = null;
          fileBytes = 0;
          renderError = "Le navigateur a refusé l'image (CSP). Enregistrement impossible.";
          resolve(null);
        }
      };
      try {
        canvasEl.toBlob(done, "image/png");
      } catch {
        done(null);
      }
    });
    pendingPrepare = p;
    p.finally(() => {
      if (pendingPrepare === p) pendingPrepare = null;
    });
    return p;
  }
  async function ensureFile() {
    for (let attempt = 0; attempt < 2; attempt++) {
      if (file) return file;
      const next = await (pendingPrepare || prepareNow());
      if (next) return next;
      if (renderError) return null;
    }
    return null;
  }

  function downloadBlob(f) {
    const url = URL.createObjectURL(f);
    const a = document.createElement("a");
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 30000);
  }

  const nativeShare =
    typeof window !== "undefined" && typeof window.NSNative?.shareFile === "function"
      ? window.NSNative.shareFile.bind(window.NSNative)
      : null;
  let canShareFiles = false;
  try {
    canShareFiles =
      typeof navigator !== "undefined" &&
      !!navigator.canShare &&
      navigator.canShare({ files: [new File([""], "recap.png", { type: "image/png" })] });
  } catch {
    canShareFiles = false;
  }
  // The Android bridge's shareFile() takes a SERVER url, and a canvas has none:
  // say it before the person presses, exactly as the lyric card does.
  $: nativeBlocked = !!nativeShare && !canShareFiles;

  async function share() {
    if (busy || renderError) return;
    busy = true;
    try {
      const f = await ensureFile();
      if (!f) {
        if (!renderError) toasts.push("L'affiche n'a pas pu être préparée.", "error");
        return;
      }
      const title = `Récap ${year || ""}`.trim();
      if (
        canShareFiles &&
        typeof navigator.share === "function" &&
        navigator.canShare &&
        navigator.canShare({ files: [f] })
      ) {
        await navigator.share({ files: [f], title });
        return;
      }
      if (typeof navigator.share === "function") {
        await navigator.share({ title, text: shareText(payload) });
        return;
      }
      if (nativeShare) {
        toasts.push("Partage d'images indisponible dans l'application.", "error");
        return;
      }
      downloadBlob(f);
    } catch (e) {
      if (e && (e.name === "AbortError" || e.name === "NotAllowedError")) {
        /* the person closed the OS share sheet — not a failure */
      } else {
        toasts.push("Échec du partage", "error");
      }
    } finally {
      busy = false;
    }
  }

  async function save() {
    if (busy || renderError) return;
    busy = true;
    try {
      const f = await ensureFile();
      if (!f) {
        if (!renderError) toasts.push("L'affiche n'a pas pu être préparée.", "error");
        return;
      }
      if (nativeShare && !canShareFiles) {
        toasts.push("Enregistrement indisponible dans l'application.", "error");
        return;
      }
      downloadBlob(f);
      toasts.push("Image enregistrée");
    } catch {
      toasts.push("Échec de l'enregistrement de l'image", "error");
    } finally {
      busy = false;
    }
  }
</script>

<svelte:window on:keydown={onKey} />

<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
<div class="rs-backdrop" use:toBody on:click={onBackdrop} transition:veil={{ for: "sheet" }}>
  <div
    class="rs-sheet"
    bind:this={sheetEl}
    role="dialog"
    aria-modal="true"
    aria-labelledby={titleId}
    tabindex="-1"
    transition:sheet
    on:click|stopPropagation
  >
    <header class="rs-head">
      <div>
        <h2 id={titleId}>Partage ton année</h2>
        <p class="rs-sub mono">PNG · 1080 × 1350{sizeLabel ? " · " + sizeLabel : ""}</p>
      </div>
      <button class="rs-close" bind:this={closeEl} on:click={close} aria-label="Fermer" disabled={busy}>
        <Icon name="close" size={22} />
      </button>
    </header>

    <div class="rs-body">
      <div class="rs-preview" role="img" aria-label={previewLabel}>
        <canvas bind:this={canvasEl} width={CARD_W} height={CARD_H} aria-hidden="true"></canvas>
        {#if preparing}
          <span class="rs-spin" aria-hidden="true" data-motion="status"></span>
        {/if}
      </div>

      <div class="rs-side">
        <p class="rs-text">{shownText}</p>
        {#if nativeBlocked}
          <p class="rs-note" role="status" in:fadeIn>
            Partage et enregistrement indisponibles dans l'application. Ouvrez le serveur dans un navigateur.
          </p>
        {/if}
        {#if renderError}
          <p class="rs-error" role="alert" in:fadeIn>{renderError}</p>
        {/if}
        <div class="rs-actions">
          <button class="rs-primary" on:click={share} disabled={busy || !!renderError}>
            {#if busy}
              <span class="rs-spin small" aria-hidden="true" data-motion="status"></span> Préparation…
            {:else}
              <Icon name="share" size={17} /> Partager l'image
            {/if}
          </button>
          <button class="rs-ghost" on:click={save} disabled={busy || !!renderError}>
            <Icon name="download" size={17} /> Enregistrer l'image
          </button>
        </div>
      </div>
    </div>
  </div>
</div>

<style>
  .rs-backdrop {
    position: fixed;
    inset: 0;
    z-index: 340;
    display: grid;
    place-items: center;
    padding: 20px;
    background: rgba(8, 6, 12, 0.62);
    touch-action: none;
    overscroll-behavior: contain;
  }
  .rs-sheet {
    width: min(880px, 94vw);
    max-height: 92dvh;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: var(--s1);
    color: var(--tx);
    border: 1px solid var(--ln);
    border-radius: 24px;
    box-shadow: 0 30px 90px rgba(0, 0, 0, 0.55);
    touch-action: auto;
  }
  .rs-head {
    flex: none;
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    padding: 20px 24px 14px;
    border-bottom: 1px solid var(--ln);
  }
  .rs-head h2 {
    margin: 0;
    font-family: var(--font-display);
    font-size: 1.35rem;
    font-weight: 800;
    letter-spacing: -0.03em;
    line-height: 1.15;
  }
  .rs-sub {
    margin: 4px 0 0;
    color: var(--dim);
    font-size: 0.7rem;
    letter-spacing: 0.04em;
    font-variant-numeric: tabular-nums;
  }
  .rs-close {
    flex: none;
    width: 44px;
    height: 44px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    color: var(--dim);
  }
  .rs-close:hover:not(:disabled) {
    color: var(--tx);
    background: var(--s2);
  }
  .rs-close:disabled {
    opacity: 0.4;
  }
  .rs-body {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(240px, 300px);
    gap: 26px;
    padding: 20px 24px 24px;
    overflow: hidden;
  }
  .rs-preview {
    position: relative;
    min-width: 0;
    display: grid;
    place-items: center;
  }
  .rs-preview canvas {
    display: block;
    width: auto;
    height: auto;
    max-width: 100%;
    max-height: min(64dvh, 620px);
    aspect-ratio: 4 / 5;
    border-radius: 14px;
    box-shadow: 0 14px 44px rgba(0, 0, 0, 0.38);
  }
  .rs-side {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 14px;
    justify-content: center;
  }
  .rs-text {
    margin: 0;
    font-size: 0.95rem;
    line-height: 1.55;
    color: var(--tx);
  }
  .rs-note,
  .rs-error {
    margin: 0;
    font-size: 0.78rem;
    line-height: 1.45;
  }
  .rs-note {
    color: var(--dim);
  }
  .rs-error {
    color: var(--tx);
    border: 1px solid color-mix(in srgb, var(--tx) 22%, transparent);
    background: color-mix(in srgb, var(--tx) 6%, transparent);
    border-radius: 10px;
    padding: 8px 12px;
  }
  .rs-actions {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 4px;
  }
  .rs-actions button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    min-height: 44px;
    padding: 0 18px;
    border-radius: 999px;
    font-weight: 700;
    font-size: 0.9rem;
    white-space: nowrap;
  }
  .rs-primary {
    background: var(--btn);
    color: var(--onbtn);
  }
  .rs-primary:hover:not(:disabled) {
    filter: brightness(1.08);
  }
  .rs-ghost {
    color: var(--tx);
    border: 1px solid var(--ln);
  }
  .rs-ghost:hover:not(:disabled) {
    background: var(--s2);
  }
  .rs-actions button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .rs-spin {
    position: absolute;
    right: 12px;
    bottom: 12px;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    border: 2px solid rgba(247, 244, 236, 0.35);
    border-top-color: var(--tx);
    animation: rs-spin 0.7s linear infinite;
  }
  .rs-spin.small {
    position: static;
    width: 14px;
    height: 14px;
    border-color: color-mix(in srgb, currentColor 28%, transparent);
    border-top-color: currentColor;
  }
  @keyframes rs-spin {
    to {
      transform: rotate(360deg);
    }
  }

  @media (max-width: 760px) {
    .rs-body {
      display: flex;
      flex-direction: column;
      gap: 16px;
      overflow-y: auto;
    }
    .rs-preview canvas {
      max-height: 46dvh;
    }
    .rs-side {
      justify-content: flex-start;
    }
  }
  @media (max-width: 640px) {
    .rs-backdrop {
      place-items: stretch;
      padding: 0;
    }
    .rs-sheet {
      width: 100%;
      height: 100dvh;
      max-height: none;
      border: none;
      border-radius: 0;
      box-shadow: none;
    }
    .rs-head {
      padding: max(14px, var(--safe-top, env(safe-area-inset-top, 0px))) 18px 12px;
    }
    .rs-head h2 {
      font-size: 1.15rem;
    }
    .rs-body {
      padding: 14px 18px calc(14px + var(--safe-bottom, env(safe-area-inset-bottom, 0px)));
    }
    .rs-preview canvas {
      max-height: 44dvh;
    }
    .rs-actions button {
      font-size: 0.85rem;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .rs-sheet,
    .rs-spin {
      animation: none;
    }
  }
</style>
