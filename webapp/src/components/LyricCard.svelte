<script module>
  // One id per mounted card: two Lyrics panels can be in the DOM (the desktop
  // tab and the now-playing column), and duplicate aria ids would point a
  // screen reader at the wrong heading.
  let cardSeq = 0;
</script>

<script>
  // The lyric card: a canvas poster made from one to three lines of the
  // playing track's words, shared as a PNG.
  //
  // Why 4:5 (1080x1350): Instagram's documented feed range is 1.91:1 to 4:5 and
  // 1080x1350 is the portrait frame that keeps the whole image in a feed post
  // (a square loses about 12% of each side in the 3:4 profile grid); it also
  // leaves room for three lyric lines plus the title/artist block. Sources:
  // Instagram Help "Image resolution of photos you share", Buffer's 2026
  // Instagram size guide. The interaction follows the established apps:
  // Spotify selects up to five CONSECUTIVE lines with a tap-to-toggle card and
  // album-coloured backgrounds; Apple Music selects first/last lines in a share
  // sheet preview. This card keeps 1-3 lines (any of them, previewed live),
  // takes the sleeve's dominant colour, and reuses the existing share chain.
  //
  // Nothing here is fetched: the sleeve comes from the on-device cover cache,
  // and a missing sleeve is drawn as Sillon's deterministic groove. Opening the
  // card on a dead mobile link produces the same image as on Wi-Fi.
  import { onMount, onDestroy, tick } from "svelte";
  import { offlineCovers, toasts } from "../lib/stores.js";
  import { artistLine, bytes, coverKey, duration as fmtDuration } from "../lib/format.js";
  import { trackKey, trackRef } from "../lib/trackkey.js";
  import { dominantColor, rgb } from "../lib/color.js";
  import { grooveFor, fromHex, readableAccent } from "../lib/sillon.js";
  import {
    CARD_W,
    CARD_H,
    MAX_LINES,
    lyricLines,
    defaultLineIndex,
    toggleLine,
    chooseLines,
    wrapText,
    fitOneLine,
    fitLyricLayout,
    cardFilename,
  } from "../lib/lyriccard.js";
  import Icon from "./Icon.svelte";

  export let track = null;
  export let lyrics = null;
  // The line being heard when the card was opened. Frozen on purpose: a
  // selection that moves under the person's finger is not a selection.
  export let activeIndex = -1;
  export let onclose = () => {};

  // -- the card's geometry ----------------------------------------------------
  const MARGIN = 96;
  const LYRIC_TOP = 250;
  // The accent rule above the title sits at META_TOP - 42 (1064); stopping at
  // 1010 leaves the words room without ever touching the metadata.
  const LYRIC_MAX_H = 760;
  const META_H = 148;
  const META_TOP = CARD_H - MARGIN - META_H;
  const CREAM = "#F7F4EC";
  const INK = "#0C0C0B";
  const TITLE_SIZE = 42;
  const TITLE_LH = 48;
  const ARTIST_SIZE = 30;

  const SEED = trackKey(track) || trackRef(track) || "neezer";
  const GROOVE = grooveFor(SEED);

  let lines = lyricLines(lyrics);
  let selected = lines.length ? [defaultLineIndex(lines, activeIndex)] : [];
  let notice = "";
  let renderError = "";
  let preparing = false;
  let busy = false;
  let file = null;
  let fileBytes = 0;
  let canvasEl = null;
  let sheetEl = null;
  let closeEl = null;
  let lastFocus = null;
  let coverImg = null;
  let coverUrl = null;
  let loadedCoverUrl = null;
  // The sleeve's colour, lifted where needed so the small marks read; the card
  // itself is always dark with cream text, so a bright sleeve can never make
  // the words illegible (the veil beneath the text is the guarantee, the
  // dominant colour is the atmosphere).
  let dominant = fromHex(GROOVE[1]);
  let destroyed = false;
  let prepareSeq = 0;
  let prepareTimer = 0;
  let pendingPrepare = null;
  let renderRaf = 0;
  const uid = ++cardSeq;
  const titleId = "lyric-card-title-" + uid;
  const hintId = "lyric-card-hint-" + uid;

  $: selectedTexts = chooseLines(lines, selected);
  $: filename = cardFilename(artistLine(track), track && track.title);
  $: unsynced = lines.length > 0 && !(lyrics && Array.isArray(lyrics.synced) && lyrics.synced.length);
  $: hasTimes = lines.some((l) => l.time != null);
  $: previewLabel = selectedTexts.length
    ? "Aperçu de la carte : " + selectedTexts.join(" · ")
    : "Aperçu de la carte";
  $: sizeLabel = fileBytes ? bytes(fileBytes) : "";
  $: coverUrl = track && track.album && track.album.cover ? $offlineCovers[coverKey(track.album.cover)] || null : null;
  $: if (coverUrl !== loadedCoverUrl) {
    loadedCoverUrl = coverUrl;
    loadCover(coverUrl);
  }
  // The sleeve cache fills in a moment after playback starts; the moment it
  // does, the card gains its art without a second open.
  $: if (coverImg && dominant) queueRender();

  // -- opening, focus and closing --------------------------------------------
  onMount(async () => {
    lastFocus = document.activeElement;
    await tick();
    (closeEl || sheetEl)?.focus();
    // Paint at once with the face the page already has, then refine when the
    // self-hosted fonts confirm: a slow or missing font must not leave the
    // preview blank for the whole 1.4 s budget.
    queueRender();
    await loadFonts();
    if (destroyed) return;
    queueRender();
  });

  onDestroy(() => {
    destroyed = true;
    clearTimeout(prepareTimer);
    cancelAnimationFrame(renderRaf);
    prepareSeq++;
    // The button that opened this sheet gets the focus back: a keyboard user
    // must not land at the top of the player after Échap.
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
        document.fonts.load('800 96px "Bricolage Grotesque"'),
        document.fonts.load('600 42px "Bricolage Grotesque"'),
        document.fonts.load('400 30px "Instrument Sans"'),
        document.fonts.load('500 24px "JetBrains Mono"'),
      ]);
      // A font that never resolves must not keep the preview on its fallback
      // face for the whole session; the stack is already usable.
      await Promise.race([load, new Promise((resolve) => setTimeout(resolve, 1400))]);
    } catch {
      /* the system stack is the fallback */
    }
  }

  function close() {
    if (busy) return;
    onclose();
  }

  function onKey(e) {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
      return;
    }
    if (e.key !== "Tab" || !sheetEl) return;
    if (!sheetEl.contains(document.activeElement)) return;
    const focusable = [...sheetEl.querySelectorAll('button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])')]
      .filter((el) => el.getClientRects().length > 0);
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

  function onBackdropClick(e) {
    if (e.target === e.currentTarget) close();
  }

  // Keep the player / page behind the veil still, without touching its
  // scrollbar (a width change there would shift the screen under the sheet).
  function onWheel(e) {
    const target = e.target;
    if (!target || !target.closest || !target.closest(".lc-scroll")) e.preventDefault();
  }

  // -- selection --------------------------------------------------------------
  function toggle(i) {
    const r = toggleLine(selected, i);
    if (r.reason === "max") notice = "Trois lignes au maximum : retirez-en une.";
    else if (r.reason === "min") notice = "Gardez au moins une ligne.";
    else notice = "";
    if (r.reason) return;
    selected = r.indices;
    invalidateFile();
    // Synchronous: the canvas is the preview, and the tap must show its result
    // before the eye leaves the line. The list itself has already been updated
    // by Svelte's microtask.
    draw();
    schedulePrepare();
  }

  // The canvas is about to change: whatever PNG was ready names the previous
  // selection, so it must not be what "Partager" sends.
  function invalidateFile() {
    file = null;
    fileBytes = 0;
    pendingPrepare = null;
    prepareSeq++;
  }

  function timeLabel(ms) {
    if (ms == null || !Number.isFinite(Number(ms))) return "";
    // The app's one duration format: h:mm:ss past an hour, never "92:15".
    return fmtDuration(Math.round(Number(ms) / 1000));
  }

  // -- the sleeve -------------------------------------------------------------
  // The cached blob is the ONLY source: it is the server's own 1000px art,
  // already on the device, and using it means the card opens offline and costs
  // no request. Missing art is Sillon's groove, never an empty square.
  function loadCover(url) {
    coverImg = null;
    if (!url) {
      dominant = fromHex(GROOVE[1]);
      queueRender();
      return;
    }
    const img = new Image();
    img.decoding = "async";
    img.onload = () => {
      if (destroyed || coverUrl !== url) return;
      if (!img.naturalWidth || !img.naturalHeight) {
        dominant = fromHex(GROOVE[1]);
        queueRender();
        return;
      }
      coverImg = img;
      queueRender();
      dominantColor(url)
        .then((c) => {
          if (destroyed || coverUrl !== url) return;
          if (Array.isArray(c) && c.length >= 3) {
            dominant = c;
            queueRender();
          }
        })
        .catch(() => {});
    };
    img.onerror = () => {
      if (destroyed || coverUrl !== url) return;
      coverImg = null;
      dominant = fromHex(GROOVE[1]);
      queueRender();
    };
    img.src = url;
  }

  // -- drawing ----------------------------------------------------------------
  function queueRender() {
    if (destroyed || renderRaf) return;
    renderRaf = requestAnimationFrame(() => {
      renderRaf = 0;
      draw();
    });
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
      renderError = "La carte ne peut pas être dessinée sur cet appareil.";
      return;
    }
    try {
      ctx.setTransform(1, 0, 0, 1, 0, 0);
      ctx.globalAlpha = 1;
      if (typeof ctx.filter === "string") ctx.filter = "none";
      ctx.clearRect(0, 0, CARD_W, CARD_H);
      const accent = readableAccent(dominant, INK, 3);
      paintGround(ctx, accent);
      drawTopLine(ctx, accent);
      drawWords(ctx);
      drawMeta(ctx, accent);
      renderError = "";
      invalidateFile();
      schedulePrepare();
    } catch {
      // Tainted canvas / a policy that refuses the drawing: say it, never a
      // preview that silently cannot be saved.
      renderError = "Le navigateur a refusé l'image (CSP). Enregistrement impossible.";
      file = null;
      fileBytes = 0;
    }
  }

  function paintGround(ctx, accent) {
    if (coverImg) {
      const canFilter = typeof ctx.filter === "string";
      ctx.save();
      if (canFilter) ctx.filter = "blur(32px) saturate(1.12)";
      drawCoverFill(ctx, coverImg, -90, -90, CARD_W + 180, CARD_H + 180);
      ctx.restore();
      if (canFilter) ctx.filter = "none";
      // Even a white sleeve keeps cream text above 6:1 here; the colour sits
      // on top as light, not as the surface the words are read on.
      ctx.fillStyle = "rgba(8,8,10,0.66)";
      ctx.fillRect(0, 0, CARD_W, CARD_H);
    } else {
      paintGroove(ctx);
      ctx.fillStyle = "rgba(8,8,10,0.26)";
      ctx.fillRect(0, 0, CARD_W, CARD_H);
    }

    // The sleeve's own colour, as a glow in the top-right corner.
    const glow = ctx.createRadialGradient(CARD_W * 0.84, CARD_H * 0.12, 0, CARD_W * 0.84, CARD_H * 0.12, CARD_W * 1.05);
    glow.addColorStop(0, rgb(dominant, 0.52));
    glow.addColorStop(0.55, rgb(dominant, 0.15));
    glow.addColorStop(1, rgb(dominant, 0));
    ctx.fillStyle = glow;
    ctx.fillRect(0, 0, CARD_W, CARD_H);

    // Bottom veil: the title and artist are always legible, whatever the art.
    const bottom = ctx.createLinearGradient(0, CARD_H * 0.55, 0, CARD_H);
    bottom.addColorStop(0, "rgba(6,6,8,0)");
    bottom.addColorStop(1, "rgba(6,6,8,0.88)");
    ctx.fillStyle = bottom;
    ctx.fillRect(0, CARD_H * 0.55, CARD_W, CARD_H * 0.45);

    // Top veil for the label and the mark.
    const top = ctx.createLinearGradient(0, 0, 0, 190);
    top.addColorStop(0, "rgba(6,6,8,0.55)");
    top.addColorStop(1, "rgba(6,6,8,0)");
    ctx.fillStyle = top;
    ctx.fillRect(0, 0, CARD_W, 190);

    // A soft band behind the words, wide enough to cover a bright chorus.
    const scrim = ctx.createLinearGradient(0, LYRIC_TOP - 90, 0, LYRIC_TOP + LYRIC_MAX_H + 80);
    scrim.addColorStop(0, "rgba(6,6,8,0)");
    scrim.addColorStop(0.16, "rgba(6,6,8,0.3)");
    scrim.addColorStop(0.84, "rgba(6,6,8,0.3)");
    scrim.addColorStop(1, "rgba(6,6,8,0)");
    ctx.fillStyle = scrim;
    ctx.fillRect(0, LYRIC_TOP - 90, CARD_W, LYRIC_MAX_H + 170);

    // A short mark of the cover's own colour, in case a theme has made the
    // accent invisible on the card. Decorative only.
    ctx.fillStyle = rgb(accent, 0.85);
    ctx.fillRect(MARGIN, META_TOP - 42, 64, 5);
  }

  // Sillon's record groove, at card scale: the same deterministic pair the
  // missing-cover tile uses (lib/sillon.js), drawn as a real disc.
  function paintGroove(ctx) {
    const [a, b] = GROOVE;
    ctx.fillStyle = a;
    ctx.fillRect(0, 0, CARD_W, CARD_H);
    const cx = CARD_W * 0.7;
    const cy = CARD_H * 0.29;
    ctx.strokeStyle = b;
    ctx.globalAlpha = 0.6;
    ctx.lineWidth = 3;
    for (let r = 10; r < 1900; r += 18) {
      ctx.beginPath();
      ctx.arc(cx, cy, r, 0, Math.PI * 2);
      ctx.stroke();
    }
    ctx.globalAlpha = 0.35;
    ctx.fillStyle = b;
    ctx.beginPath();
    ctx.arc(cx, cy, 118, 0, Math.PI * 2);
    ctx.fill();
    ctx.globalAlpha = 1;
    ctx.fillStyle = INK;
    ctx.beginPath();
    ctx.arc(cx, cy, 13, 0, Math.PI * 2);
    ctx.fill();
  }

  function drawCoverFill(ctx, img, x, y, w, h) {
    const iw = img.naturalWidth || img.width || 0;
    const ih = img.naturalHeight || img.height || 0;
    if (!iw || !ih || !w || !h) return;
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

  function drawTopLine(ctx, accent) {
    ctx.textBaseline = "top";
    ctx.textAlign = "left";
    ctx.fillStyle = "rgba(247,244,236,0.66)";
    ctx.font = '500 24px "JetBrains Mono", ui-monospace, monospace';
    ctx.fillText("PAROLES", MARGIN, 96);

    const label = "Neezer";
    ctx.textAlign = "right";
    ctx.font = '600 28px "Bricolage Grotesque", "Instrument Sans", system-ui, sans-serif';
    ctx.fillStyle = "rgba(247,244,236,0.8)";
    ctx.fillText(label, CARD_W - MARGIN, 92);
    const labelW = ctx.measureText(label).width;
    drawBrandMark(ctx, CARD_W - MARGIN - labelW - 22 - 30, 92, 30, accent);
    ctx.textAlign = "left";
  }

  // The discreet brand mark: Sillon's disc in paper ink, its label in the
  // sleeve's own colour. It stays quiet next to the groove and never picks a
  // fight with a coloured cover the way the groove pair would.
  function drawBrandMark(ctx, x, y, size, accent) {
    const cx = x + size / 2;
    const cy = y + size / 2;
    const R = size / 2;
    ctx.save();
    ctx.beginPath();
    ctx.arc(cx, cy, R, 0, Math.PI * 2);
    ctx.fillStyle = "rgba(12,12,11,0.32)";
    ctx.fill();
    ctx.strokeStyle = "rgba(247,244,236,0.72)";
    ctx.lineWidth = Math.max(1, size * 0.045);
    ctx.stroke();
    ctx.beginPath();
    ctx.arc(cx, cy, R * 0.62, 0, Math.PI * 2);
    ctx.strokeStyle = "rgba(247,244,236,0.4)";
    ctx.lineWidth = Math.max(1, size * 0.03);
    ctx.stroke();
    ctx.beginPath();
    ctx.arc(cx, cy, R * 0.24, 0, Math.PI * 2);
    ctx.fillStyle = rgb(accent, 1);
    ctx.fill();
    ctx.beginPath();
    ctx.arc(cx, cy, R * 0.07, 0, Math.PI * 2);
    ctx.fillStyle = "rgba(12,12,11,0.85)";
    ctx.fill();
    ctx.restore();
  }

  function drawDisc(ctx, x, y, size, accent) {
    const cx = x + size / 2;
    const cy = y + size / 2;
    const R = size / 2;
    ctx.save();
    ctx.beginPath();
    ctx.arc(cx, cy, R, 0, Math.PI * 2);
    ctx.fillStyle = GROOVE[0];
    ctx.fill();
    ctx.strokeStyle = "rgba(247,244,236,0.34)";
    ctx.lineWidth = Math.max(1, size * 0.018);
    ctx.stroke();
    ctx.strokeStyle = GROOVE[1];
    ctx.globalAlpha = 0.85;
    ctx.lineWidth = Math.max(1, size * 0.032);
    for (let r = R - size * 0.08; r > R * 0.3; r -= Math.max(1.6, size * 0.05)) {
      ctx.beginPath();
      ctx.arc(cx, cy, r, 0, Math.PI * 2);
      ctx.stroke();
    }
    ctx.globalAlpha = 1;
    // Sillon's mark: a lemon (accent) label and a black centre.
    ctx.beginPath();
    ctx.arc(cx, cy, R * 0.2, 0, Math.PI * 2);
    ctx.fillStyle = rgb(accent, 1);
    ctx.fill();
    ctx.beginPath();
    ctx.arc(cx, cy, R * 0.055, 0, Math.PI * 2);
    ctx.fillStyle = INK;
    ctx.fill();
    ctx.restore();
  }

  function lyricFont(size) {
    return `800 ${size}px "Bricolage Grotesque", "Instrument Sans", system-ui, sans-serif`;
  }

  function drawWords(ctx) {
    // Read the selection DIRECTLY: `selectedTexts` is a reactive statement and
    // Svelte runs it after this synchronous call, so using it here made the
    // preview (and the PNG) one tap behind — the last selected line was
    // missing until the next redraw.
    const texts = chooseLines(lines, selected);
    if (!texts.length) return;
    const layout = fitLyricLayout(
      (size) => {
        ctx.font = lyricFont(size);
        return ctx;
      },
      texts,
      { maxWidth: CARD_W - MARGIN * 2, maxHeight: LYRIC_MAX_H, lineHeight: 1.24 },
    );
    if (!layout.rows.length) return;
    ctx.save();
    ctx.beginPath();
    ctx.rect(MARGIN, LYRIC_TOP, CARD_W - MARGIN * 2, LYRIC_MAX_H);
    ctx.clip();
    ctx.font = lyricFont(layout.size);
    ctx.textAlign = "left";
    ctx.textBaseline = "top";
    ctx.fillStyle = CREAM;
    // A shadow, not a second colour: the words read on a busy sleeve too.
    ctx.shadowColor = "rgba(0,0,0,0.42)";
    ctx.shadowBlur = 18;
    ctx.shadowOffsetY = 3;
    const lh = layout.size * 1.24;
    const gap = layout.size * 0.14;
    const total = layout.rows.length * lh + Math.max(0, layout.blocks.length - 1) * gap;
    let y = LYRIC_TOP + Math.max(0, (LYRIC_MAX_H - total) / 2);
    for (const block of layout.blocks) {
      for (const row of block) {
        ctx.fillText(row, MARGIN, y);
        y += lh;
      }
      y += gap;
    }
    ctx.restore();
  }

  function drawMeta(ctx, accent) {
    // The sleeve (or the groove disc) at the left, the names beside it.
    if (coverImg) {
      ctx.save();
      ctx.beginPath();
      pathRoundRect(ctx, MARGIN, META_TOP, META_H, META_H, 14);
      ctx.clip();
      drawCoverFill(ctx, coverImg, MARGIN, META_TOP, META_H, META_H);
      ctx.restore();
      ctx.beginPath();
      pathRoundRect(ctx, MARGIN + 0.75, META_TOP + 0.75, META_H - 1.5, META_H - 1.5, 13.5);
      ctx.strokeStyle = "rgba(247,244,236,0.24)";
      ctx.lineWidth = 1.5;
      ctx.stroke();
    } else {
      drawDisc(ctx, MARGIN, META_TOP, META_H, accent);
    }

    const x = MARGIN + META_H + 34;
    const w = CARD_W - x - MARGIN;
    ctx.textAlign = "left";
    ctx.textBaseline = "top";

    ctx.font = `600 ${TITLE_SIZE}px "Bricolage Grotesque", "Instrument Sans", system-ui, sans-serif`;
    ctx.fillStyle = CREAM;
    const title = String((track && track.title) || "Titre inconnu").trim() || "Titre inconnu";
    let rows = wrapText(ctx, title, w);
    if (rows.length > 2) rows = [rows[0], fitOneLine(ctx, rows.slice(1).join(" "), w)];
    let y = META_TOP + 2;
    for (const row of rows) {
      ctx.fillText(row, x, y);
      y += TITLE_LH;
    }

    ctx.font = `400 ${ARTIST_SIZE}px "Instrument Sans", system-ui, sans-serif`;
    ctx.fillStyle = "rgba(247,244,236,0.76)";
    const artist = fitOneLine(ctx, artistLine(track) || "", w);
    if (artist) ctx.fillText(artist, x, y + 6);
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

  // -- PNG, share, save -------------------------------------------------------
  function schedulePrepare() {
    if (destroyed) return;
    clearTimeout(prepareTimer);
    prepareTimer = setTimeout(() => prepareNow(), 110);
  }

  function prepareNow() {
    if (!canvasEl || !chooseLines(lines, selected).length) {
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
          // toBlob returns null / throws when the canvas is tainted or a policy
          // refuses it. That must be a visible reason, never a dead button.
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
    // The preview may be redrawn by a late cover while a share is being
    // prepared; a stale prepare resolves null, so ask once more from the
    // canvas as it is now rather than doing nothing when the button is pressed.
    for (let attempt = 0; attempt < 2; attempt++) {
      if (file) return file;
      const f = await (pendingPrepare || prepareNow());
      if (f) return f;
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

  // The same capability test and preference order as ShareSheet.svelte. A plain
  // Android WebView has no file plumbing for navigator.share, hence the bridge.
  const nativeShare =
    typeof window !== "undefined" && typeof window.NSNative?.shareFile === "function"
      ? window.NSNative.shareFile.bind(window.NSNative)
      : null;
  let canShareFiles = false;
  // Asked once, with a real (empty) PNG File, exactly as ShareSheet does. Some
  // WebViews do implement file sharing; where they do not, the native bridge
  // below cannot help (see share()) and the reason is said out loud.
  try {
    canShareFiles =
      typeof navigator !== "undefined" &&
      !!navigator.canShare &&
      navigator.canShare({ files: [new File([""], "carte.png", { type: "image/png" })] });
  } catch {
    canShareFiles = false;
  }
  // The Android bridge's shareFile() fetches a SERVER url; a generated canvas
  // has none. Say it in the sheet, not only in a 2.6 s toast, so the person
  // knows before pressing and can open the server in a browser instead.
  $: nativeBlocked = !!nativeShare && !canShareFiles;

  async function share() {
    if (busy || renderError || !lines.length) return;
    busy = true;
    try {
      const f = await ensureFile();
      if (!f) {
        if (!renderError) toasts.push("L'image n'a pas pu être préparée.", "error");
        return;
      }
      const artist = artistLine(track);
      const title = `${(track && track.title) || "Neezer"}${artist ? " — " + artist : ""}`;
      if (
        canShareFiles &&
        typeof navigator.share === "function" &&
        navigator.canShare &&
        navigator.canShare({ files: [f] })
      ) {
        await navigator.share({ files: [f], title });
        return;
      }
      if (nativeShare) {
        // MainActivity.shareFile takes a SERVER url and fetches it itself; a
        // canvas has no such url, and handing it a blob: would be refused with
        // only a native toast. Say what is true and leave the file ready.
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
    if (busy || renderError || !lines.length) return;
    busy = true;
    try {
      const f = await ensureFile();
      if (!f) {
        if (!renderError) toasts.push("L'image n'a pas pu être préparée.", "error");
        return;
      }
      if (nativeShare && !canShareFiles) {
        // A blob: download reaches Android's DownloadListener, which only speaks
        // http(s): the file would vanish with a generic failure. Be explicit.
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
<div class="lc-backdrop" on:click={onBackdropClick} on:wheel|nonpassive={onWheel}>
  <div
    class="lc-sheet"
    bind:this={sheetEl}
    role="dialog"
    aria-modal="true"
    aria-labelledby={titleId}
    aria-describedby={lines.length ? hintId : undefined}
    aria-busy={preparing || busy}
    tabindex="-1"
    on:click|stopPropagation
  >
    <header class="lc-head">
      <div class="lc-who">
        <h2 id={titleId}>Créer une carte</h2>
        <p class="lc-sub">
          <span class="lc-track">{(track && track.title) || "Titre inconnu"}</span>
          {#if artistLine(track)}<span class="lc-dot" aria-hidden="true">·</span><span class="lc-artist">{artistLine(track)}</span>{/if}
        </p>
      </div>
      <button class="lc-close" bind:this={closeEl} on:click={close} aria-label="Fermer" disabled={busy}>
        <Icon name="close" size={22} />
      </button>
    </header>

    <div class="lc-body">
      <div class="lc-preview-col">
        <!-- The card is a picture: the wrapper names it for assistive tech, the
             canvas itself is decorative (Svelte refuses role="img" on canvas,
             and the wrapper is the correct place for the accessible name). -->
        <div class="lc-preview" role="img" aria-label={previewLabel}>
          <canvas bind:this={canvasEl} width={CARD_W} height={CARD_H} aria-hidden="true"></canvas>
          {#if preparing}
            <span class="lc-spin" aria-hidden="true"></span>
          {/if}
        </div>
        <p class="lc-format mono">
          PNG · {CARD_W} × {CARD_H}{sizeLabel ? " · " + sizeLabel : ""}
        </p>
        {#if unsynced}
          <p class="lc-unsynced" role="status">Aucune parole synchronisée pour ce titre.</p>
        {/if}
        {#if nativeBlocked}
          <p class="lc-note" role="status">
            Partage et enregistrement indisponibles dans l'application. Ouvrez le serveur dans un navigateur.
          </p>
        {/if}
        {#if renderError}
          <p class="lc-error" role="alert">{renderError}</p>
        {/if}
      </div>

      <div class="lc-choose">
        {#if lines.length}
          <div class="lc-choose-head">
            <h3 id={hintId}>Choisissez une à trois lignes</h3>
            <span class="lc-count mono" aria-hidden="true">{selected.length} / {MAX_LINES}</span>
          </div>
          <div class="lc-lines lc-scroll" class:timed={hasTimes}>
            {#each lines as line, i (i)}
              <button
                type="button"
                class="lc-line"
                class:on={selected.includes(i)}
                aria-pressed={selected.includes(i)}
                on:click={() => toggle(i)}
              >
                {#if hasTimes}<span class="lc-time mono">{timeLabel(line.time)}</span>{/if}
                <span class="lc-text">{line.text}</span>
                <span class="lc-tick" aria-hidden="true">
                  {#if selected.includes(i)}<Icon name="check" size={15} />{/if}
                </span>
              </button>
            {/each}
          </div>
          <p class="lc-notice" role="status" aria-live="polite">{notice}</p>
        {:else}
          <div class="lc-none">
            <p role="status">Aucune parole pour ce titre.</p>
          </div>
        {/if}
      </div>
    </div>

    <footer class="lc-foot">
      <button class="lc-ghost" on:click={save} disabled={busy || !!renderError || !lines.length}>
        <Icon name="download" size={17} /> Enregistrer l'image
      </button>
      <button class="lc-primary" on:click={share} disabled={busy || !!renderError || !lines.length}>
        {#if busy}
          <span class="lc-spin small" aria-hidden="true"></span> Préparation…
        {:else}
          <Icon name="share" size={17} /> Partager
        {/if}
      </button>
    </footer>
  </div>
</div>

<style>
  .lc-backdrop {
    position: fixed;
    inset: 0;
    z-index: 320;
    display: grid;
    place-items: center;
    padding: 20px;
    background: rgba(8, 6, 12, 0.6);
    backdrop-filter: blur(6px);
    touch-action: none;
    overscroll-behavior: contain;
  }
  .lc-sheet {
    width: min(1000px, 94vw);
    max-height: 92dvh;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: var(--s1);
    color: var(--tx);
    border: 1px solid var(--ln);
    border-radius: 24px;
    box-shadow: 0 30px 90px rgba(0, 0, 0, 0.55);
    animation: lc-in 0.18s ease;
    touch-action: auto;
  }
  @keyframes lc-in {
    from {
      opacity: 0;
      transform: translateY(10px) scale(0.99);
    }
  }

  .lc-head {
    flex: none;
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    padding: 20px 24px 16px;
    border-bottom: 1px solid var(--ln);
  }
  .lc-who {
    min-width: 0;
  }
  .lc-head h2 {
    margin: 0;
    font-family: var(--font-display);
    font-size: 1.35rem;
    font-weight: 800;
    letter-spacing: -0.03em;
    line-height: 1.15;
  }
  .lc-sub {
    margin: 4px 0 0;
    display: flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
    font-size: 0.85rem;
    color: var(--dim);
  }
  .lc-track,
  .lc-artist {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .lc-track {
    color: var(--tx);
    font-weight: 600;
  }
  .lc-dot {
    flex: none;
    opacity: 0.6;
  }
  .lc-close {
    flex: none;
    width: 44px;
    height: 44px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    color: var(--dim);
  }
  .lc-close:hover:not(:disabled) {
    color: var(--tx);
    background: var(--s2);
  }
  .lc-close:disabled {
    opacity: 0.4;
  }

  .lc-body {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(280px, 400px) minmax(0, 1fr);
    gap: 26px;
    padding: 20px 24px;
    overflow: hidden;
  }
  .lc-preview-col {
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
  }
  /* Width derives from the height budget so the 4:5 frame can never be
     stretched by a max-width: the preview scales with the viewport, never
     crops or distorts the card. */
  .lc-preview {
    position: relative;
    width: min(100%, 400px, calc(54dvh * 0.8));
    aspect-ratio: 4 / 5;
    border-radius: 16px;
    overflow: hidden;
    background: var(--s2);
    box-shadow: 0 14px 44px rgba(0, 0, 0, 0.38);
  }
  .lc-preview canvas {
    display: block;
    width: 100%;
    height: 100%;
  }
  .lc-format {
    margin: 0;
    font-size: 0.68rem;
    color: var(--dim);
    letter-spacing: 0.04em;
    font-variant-numeric: tabular-nums;
  }
  .lc-unsynced,
  .lc-note,
  .lc-error {
    margin: 0;
    max-width: 34ch;
    text-align: center;
    font-size: 0.78rem;
    line-height: 1.4;
  }
  .lc-unsynced,
  .lc-note {
    color: var(--dim);
  }
  .lc-note {
    max-width: 38ch;
  }
  .lc-error {
    color: var(--tx);
    border: 1px solid color-mix(in srgb, var(--tx) 22%, transparent);
    background: color-mix(in srgb, var(--tx) 6%, transparent);
    border-radius: 10px;
    padding: 8px 12px;
  }

  .lc-choose {
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .lc-choose-head {
    flex: none;
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 10px;
  }
  .lc-choose-head h3 {
    margin: 0;
    font-size: 0.95rem;
    font-weight: 700;
    letter-spacing: -0.01em;
  }
  .lc-count {
    flex: none;
    font-size: 0.72rem;
    color: var(--dim);
    font-variant-numeric: tabular-nums;
  }
  .lc-lines {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 2px 4px 2px 2px;
    margin: 0 -4px;
    scrollbar-gutter: stable;
  }
  .lc-line {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 24px;
    align-items: center;
    gap: 12px;
    min-height: 48px;
    width: 100%;
    padding: 9px 12px;
    border-radius: 12px;
    border: 1px solid transparent;
    background: transparent;
    color: var(--tx);
    text-align: left;
    line-height: 1.35;
  }
  .lc-lines.timed .lc-line {
    grid-template-columns: 56px minmax(0, 1fr) 24px;
    gap: 10px;
  }
  .lc-line:hover:not(.on) {
    background: var(--s2);
  }
  /* Selected: a real accent surface, with --onac ink — never a colour alone
     (the check mark and aria-pressed carry the state too). */
  .lc-line.on {
    background: var(--ac);
    color: var(--onac);
    border-color: transparent;
  }
  .lc-time {
    color: var(--dim);
    font-size: 0.7rem;
    font-variant-numeric: tabular-nums;
  }
  .lc-line.on .lc-time {
    color: var(--onac);
    opacity: 0.72;
  }
  .lc-text {
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .lc-tick {
    width: 22px;
    height: 22px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    border: 1.5px solid color-mix(in srgb, var(--dim) 55%, transparent);
    color: var(--onac);
  }
  .lc-line.on .lc-tick {
    border-color: transparent;
  }
  .lc-notice {
    flex: none;
    min-height: 1.2em;
    margin: 8px 2px 0;
    font-size: 0.78rem;
    line-height: 1.3;
    color: var(--dim);
  }
  .lc-none {
    flex: 1;
    display: grid;
    place-items: center;
    text-align: center;
    color: var(--dim);
    font-size: 0.9rem;
  }
  .lc-none p {
    max-width: 30ch;
    margin: 0;
  }

  .lc-foot {
    flex: none;
    display: flex;
    justify-content: flex-end;
    gap: 10px;
    padding: 16px 24px calc(16px + var(--safe-bottom, env(safe-area-inset-bottom, 0px)));
    border-top: 1px solid var(--ln);
  }
  .lc-foot button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    min-height: 44px;
    padding: 0 20px;
    border-radius: 999px;
    font-weight: 700;
    font-size: 0.9rem;
    white-space: nowrap;
  }
  .lc-ghost {
    color: var(--tx);
    border: 1px solid var(--ln);
  }
  .lc-ghost:hover:not(:disabled) {
    background: var(--s2);
  }
  .lc-primary {
    background: var(--btn);
    color: var(--onbtn);
  }
  .lc-primary:hover:not(:disabled) {
    filter: brightness(1.08);
  }
  .lc-foot button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .lc-spin {
    position: absolute;
    right: 12px;
    bottom: 12px;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    border: 2px solid rgba(247, 244, 236, 0.35);
    border-top-color: var(--tx);
    animation: lc-spin 0.7s linear infinite;
  }
  .lc-spin.small {
    position: static;
    width: 14px;
    height: 14px;
    border-color: color-mix(in srgb, currentColor 28%, transparent);
    border-top-color: currentColor;
  }
  @keyframes lc-spin {
    to {
      transform: rotate(360deg);
    }
  }

  /* Tablet and narrow desktop: one column, the preview above the lines. */
  @media (max-width: 860px) {
    .lc-body {
      display: flex;
      flex-direction: column;
      gap: 16px;
    }
    .lc-preview-col {
      flex: none;
    }
    .lc-preview {
      width: min(100%, 340px, calc(36dvh * 0.8));
    }
    .lc-choose {
      flex: 1;
    }
    .lc-lines {
      min-height: 130px;
    }
  }

  /* Phone: the sheet IS the screen, actions under the thumb. */
  @media (max-width: 640px) {
    .lc-backdrop {
      place-items: stretch;
      padding: 0;
    }
    .lc-sheet {
      width: 100%;
      height: 100vh;
      height: 100dvh;
      max-height: none;
      border: none;
      border-radius: 0;
      box-shadow: none;
    }
    .lc-head {
      padding: max(14px, var(--safe-top, env(safe-area-inset-top, 0px))) 18px 12px;
    }
    .lc-head h2 {
      font-size: 1.15rem;
    }
    .lc-body {
      padding: 12px 18px 0;
      gap: 14px;
    }
    .lc-preview {
      width: min(100%, 300px, calc(40dvh * 0.8));
    }
    .lc-format {
      font-size: 0.64rem;
    }
    .lc-lines {
      min-height: 0;
    }
    .lc-foot {
      padding: 12px 18px calc(12px + var(--safe-bottom, env(safe-area-inset-bottom, 0px)));
    }
    /* One 44 px row, both actions under the thumb. Tighter padding and type
       are what let "Enregistrer l'image" fit at 360 px — a stacked pair cost
       123 px of a 640 px screen, leaving the line list one and a half rows. */
    .lc-foot button {
      flex: 1;
      gap: 6px;
      padding: 0 10px;
      font-size: 0.8rem;
    }
  }

  /* A 640 px-tall phone cannot afford a 256 px preview and four readable
     lines: give the preview back to the selection, which is the thing the
     person came here to do. The exported card is unchanged. */
  @media (max-width: 640px) and (max-height: 700px) {
    .lc-preview {
      width: min(100%, 300px, calc(32dvh * 0.8));
    }
    .lc-head {
      padding-top: 10px;
      padding-bottom: 8px;
    }
    .lc-head h2 {
      font-size: 1.05rem;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .lc-sheet,
    .lc-spin {
      animation: none;
    }
  }
</style>
