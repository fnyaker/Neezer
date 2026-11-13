<script>
  // Settings UI for the opt-in DSP chain (see lib/audio/graph.js): volume
  // normalization, bass enhancement and a 10-band equalizer. Everything writes
  // to the persisted fx.* stores, which the audio graph subscribes to live.
  import { tick, onMount, onDestroy } from "svelte";
  import { get } from "svelte/store";
  import {
    player,
    eqEnabled,
    eqBands,
    bassBoost,
    normalization,
    fxPresets,
    crossfadeEnabled,
    crossfadeSeconds,
    crossfadeOnSkip,
    crossfadeSkipSeconds,
    trimSilence,
    trimThresholdDb,
    audioOutput,
    nativePlayer,
    nativeFault,
    glitchRoutes,
    saveFxPreset,
    applyFxPreset,
    deleteFxPreset,
    toasts,
  } from "../lib/stores.js";
  import { EQ_FREQS, EQ_MIN_DB, EQ_MAX_DB } from "../lib/audio/graph.js";
  import { subscribeSaturation } from "../lib/audio/saturation.js";
  import {
    BUILTIN_PRESETS,
    EQ_BAND_COUNT,
    activePresetId,
    createMeterSession,
    dbToGain,
    fmtDb,
    presetItems,
    presetNeedsUpdate,
    riskTone,
    updatedPreset,
    volumeCutDb,
  } from "../lib/eqpresets.js";
  import { outputPlan, retryNow } from "../lib/audio/output.js";
  import {
    nativeAudioAvailable,
    nativeDspAvailable,
    nativeChoiceOn,
    faultHolds,
    clearNativeFault,
  } from "../lib/audio/nativeplayer.js";
  import Icon from "./Icon.svelte";

  // -- where the sound goes (lib/audio/output.js) ------------------------------
  const OUTPUT_MODES = [
    { id: "auto", label: "Automatique", short: "Auto" },
    { id: "direct", label: "Directe" },
    { id: "graph", label: "Traitée" },
  ];
  const OUTPUT_HINTS = {
    auto:
      "Traitée, et directe seulement quand le lecteur mesure que le son se hache (un autoradio Bluetooth, parfois), le temps que ça dure.",
    direct:
      "Le son part tel quel vers le système, comme dans n'importe quelle application de musique : la lecture la plus robuste. Les animations suivent une copie du son ; l'égaliseur et les basses sont inactifs, et la normalisation ne fait que baisser les titres trop forts.",
    graph:
      "Tout passe par le moteur audio du navigateur : égaliseur, basses, normalisation complète. Déconseillé en Bluetooth sur Android, où ce moteur peut couper des morceaux du son.",
  };
  const KIND_LABEL = {
    bluetooth: "Bluetooth",
    wired: "Filaire",
    usb: "USB",
    speaker: "Haut-parleur",
    hdmi: "HDMI",
    other: "Autre sortie",
  };
  $: plan = $outputPlan;
  $: route = plan.route;
  $: routeText =
    route.kind === "unknown"
      ? ""
      : route.source === "latency"
        ? "Bluetooth probable (latence de sortie)"
        : `${KIND_LABEL[route.kind] || "Sortie"}${route.name ? " · " + route.name : ""}`;
  // The verdict on the output playing now (lib/audio/output.js), if any.
  $: verdict = $glitchRoutes[route.key] || null;
  $: whyText = {
    setting: "choisie ici",
    glitch: "le son se hachait en lecture traitée sur cette sortie",
    probing: "nouvel essai après un son haché — le lecteur écoute",
    default: routeText || "",
    native: routeText || "",
  }[plan.why];
  // The Android app's own player (lib/audio/nativeplayer.js): only offered
  // where the app has one.
  const nativeOffered = nativeAudioAvailable();
  // On by default where the app's player carries the effects too; what the
  // switch shows is the setting, a failure below it says what plays instead.
  $: nativeOn = nativeChoiceOn($nativePlayer);
  $: nativeFailed = nativeOn && faultHolds($nativeFault);
  $: native = nativeOffered && nativeOn && !nativeFailed;
  $: retryText =
    plan.why === "glitch" && plan.retryAt
      ? `Nouvel essai vers ${fmtTime(plan.retryAt)}, ou dès que la sortie se reconnecte.`
      : "Nouvel essai au prochain titre.";
  // The equalizer and the bass lift change the sound on its way THROUGH the
  // processor, which the direct path does not go through.
  // The app's own player runs them itself (bridge 6: the Rust chain).
  const nativeDsp = nativeDspAvailable();
  $: dspNote = plan.direct && !(plan.why === "native" && nativeDsp) ? "Inactif en lecture directe." : "";
  function fmtTime(ts) {
    try {
      return new Date(ts).toLocaleTimeString("fr-FR", { hour: "2-digit", minute: "2-digit" });
    } catch {
      return "";
    }
  }

  const NORM_LEVELS = [
    { id: "off", label: "Désactivée", hint: "Volume d'origine" },
    { id: "low", label: "Basse", hint: "Cible plus calme" },
    { id: "medium", label: "Normale", hint: "Cible standard" },
    { id: "high", label: "Élevée", hint: "Cible plus forte" },
  ];

  // Master output volume — the same store the desktop player bar drives, exposed
  // here so it's reachable on mobile (where the player bar is hidden) and so the
  // user can pull the level down to offset the EQ / bass boost.
  $: volPct = Math.round(($player.muted ? 0 : $player.volume) * 100);

  function freqLabel(hz) {
    return hz >= 1000 ? hz / 1000 + "k" : String(hz);
  }

  function setBand(i, v) {
    const next = $eqBands.slice();
    next[i] = Math.max(EQ_MIN_DB, Math.min(EQ_MAX_DB, +v || 0));
    eqBands.set(next);
  }

  $: bassPct = Math.round($bassBoost * 100);

  // -- transitions -----------------------------------------------------------
  $: fadeLabel =
    $crossfadeSeconds <= 0 ? "enchaîné direct" : `${(+$crossfadeSeconds).toFixed(1)} s`;

  $: skipFadeLabel =
    $crossfadeSkipSeconds <= 0 ? "coupe douce" : `${(+$crossfadeSkipSeconds).toFixed(2).replace(/0$/, "")} s`;

  // -- the equalizer's presets (the rules live in lib/eqpresets.js) ----------
  // A saved preset captures the whole chain (EQ + bass + normalisation), so it
  // restores a complete listening setup — "casque", "voiture", "nuit" — not just
  // a curve. Naming happens in an inline field rather than window.prompt: the
  // native dialog is blocked outright in some embedded webviews (including the
  // Android app's), and it looks nothing like the rest of the app.
  let pickedId = null;
  let naming = false;
  let draftName = "";
  let nameInput;
  let builtinList = null;
  let mineList = null;

  $: chain = { eq: $eqEnabled, bands: $eqBands, bass: $bassBoost, norm: $normalization };
  $: items = presetItems($fxPresets);
  $: builtins = items.filter((p) => p.builtin);
  $: mine = items.filter((p) => !p.builtin);
  $: activeId = activePresetId(items, chain);
  // What the person clicked (or last applied), even after they move a slider:
  // that is the preset "Mettre à jour" writes. Without a click, the preset the
  // chain matches is the natural selection; with neither, a custom setting.
  $: picked = items.find((p) => p.id === pickedId) || null;
  $: selected = picked || items.find((p) => p.id === activeId) || null;
  $: dirty = !!selected && !selected.builtin && presetNeedsUpdate(selected, chain);

  function usePreset(p) {
    if (!p) return;
    if (p.builtin) {
      eqBands.set(p.bands.slice());
      eqEnabled.set(true);
    } else {
      applyFxPreset(p);
    }
    pickedId = p.id;
    // No toast: the chip highlights, the state line names the preset, and a
    // second toast would only stack under the next action's.
  }

  async function openNaming() {
    naming = true;
    draftName = "";
    await tick();
    nameInput?.focus();
  }
  function confirmSave() {
    const saved = saveFxPreset(draftName);
    if (!saved) {
      nameInput?.focus();
      return;
    }
    naming = false;
    draftName = "";
    // saveFxPreset answers with the snapshot it BUILT; re-saving an existing
    // name keeps the stored row's id, so select what storage actually holds.
    const stored = get(fxPresets).find(
      (p) => String(p.name).toLowerCase() === saved.name.toLowerCase()
    );
    pickedId = (stored || saved).id;
    toasts.push(`Préréglage « ${saved.name} » enregistré`);
    revealSelected();
  }
  function cancelSave() {
    naming = false;
    draftName = "";
  }
  function onNameKey(e) {
    if (e.key === "Enter") confirmSave();
    else if (e.key === "Escape") cancelSave();
  }

  function restorePreset(p) {
    if (!p) return;
    fxPresets.update((list) => {
      const i = list.findIndex((x) => x.id === p.id);
      if (i < 0) return [...list, p];
      const next = list.slice();
      next[i] = p;
      return next;
    });
  }

  // "Mettre à jour" writes the current curve and bass lift into the selected
  // user preset. The previous row rides in the toast, so undo is one tap and
  // there is no confirmation dialog in the way.
  function updateCurrent() {
    if (!selected || selected.builtin) return;
    const before = $fxPresets.find((p) => p.id === selected.id);
    const next = updatedPreset(before, chain);
    if (!next) return;
    fxPresets.update((list) => list.map((p) => (p.id === next.id ? next : p)));
    pickedId = next.id;
    toasts.push(`Préréglage « ${next.name} » mis à jour`, "info", 2600, {
      label: "Annuler",
      run: () => restorePreset(before),
    });
  }

  function removeCustom(p) {
    if (!p || p.builtin) return;
    const at = $fxPresets.findIndex((x) => x.id === p.id);
    if (at < 0) return;
    const removed = $fxPresets[at];
    deleteFxPreset(p.id);
    if (pickedId === p.id) pickedId = null;
    revealSelected();
    toasts.push(`Préréglage « ${p.name} » supprimé`, "info", 2600, {
      label: "Annuler",
      run: () =>
        fxPresets.update((list) => {
          if (list.some((x) => x.id === removed.id)) return list;
          const next = list.slice();
          next.splice(Math.min(at, next.length), 0, removed);
          return next;
        }),
    });
  }

  // A preset's curve at a glance, so two presets named alike (or a built-in
  // and its copy) are told apart before applying them. Static per preset.
  function sparkHeight(db) {
    const v = Math.max(EQ_MIN_DB, Math.min(EQ_MAX_DB, +db || 0));
    return Math.round(10 + ((v - EQ_MIN_DB) / (EQ_MAX_DB - EQ_MIN_DB)) * 80);
  }

  // Bring the selected preset into its rail's window without scrolling the
  // page: with ten saved curves, the one the chain matches can mount past the
  // right edge on a phone (or below the fold of the desktop list), and a
  // highlight nobody can see is not a highlight.
  async function revealSelected() {
    await tick();
    for (const list of [builtinList, mineList]) {
      if (!list) continue;
      const el = list.querySelector('.pitem[data-selected="true"]');
      if (!el) continue;
      const lr = list.getBoundingClientRect();
      const er = el.getBoundingClientRect();
      if (list.scrollWidth > list.clientWidth) {
        if (er.left < lr.left) list.scrollLeft += er.left - lr.left - 4;
        else if (er.right > lr.right) list.scrollLeft += er.right - lr.right + 4;
      }
      if (list.scrollHeight > list.clientHeight) {
        if (er.top < lr.top) list.scrollTop += er.top - lr.top - 4;
        else if (er.bottom > lr.bottom) list.scrollTop += er.bottom - lr.bottom + 4;
      }
    }
  }

  // -- the live level meter (lib/audio/saturation.js) -----------------------
  // One snapshot at 25 Hz. The BARS are written straight to the DOM: ten
  // Svelte updates per tick would run the component's dirty checks 250 times a
  // second for ten 10 px rectangles. The readout and the warning are reactive,
  // but at a human pace (5 Hz / 2 Hz) and only when a rounded value moves.
  let eqCard = null;
  let cardVisible = false;
  const fills = new Array(EQ_BAND_COUNT).fill(null);
  const lastBands = new Array(EQ_BAND_COUNT).fill(0);
  let meterAvailable = false;
  let meterReason = "idle";
  let meterRisk = "off";
  let meterMeasured = false;
  // The readout is written straight to its three spans, not through Svelte: a
  // `stats = {...}` at 5 Hz re-ran the component's update pass 5 times a
  // second over fifteen preset chips and ten faders for three numbers, which
  // measured 98 ms of main thread a second in the phone profile. The engine's
  // own tick costs 8.
  const readValues = { peak: -96, headroom: 96, reduction: 0 };
  const readEls = { peak: null, headroom: null, reduction: null };
  let lastReadAt = 0;
  // The ENGINE publishes at 25 Hz; the BARS are painted at 13 Hz and glide
  // between readings (90 ms of CSS). Measured on this screen at 390 px: a
  // write per reading cost 124 ms of main thread a second (321 style recalcs
  // in 5 s) for ten 6 px lines, half the rate costs 45 ms/s of which the
  // engine's own tap is 32. The risk and the warning still see every reading.
  const BAR_PAINT_MS = 75;
  let lastBarsAt = 0;
  const nowMs = () =>
    typeof performance !== "undefined" && performance.now ? performance.now() : Date.now();
  let warn = null;
  let warnAnnounce = "";
  let holdReduction = 0;
  let holdMargin = 0;
  let holdAfter = 0;
  let warnedTone = "ok";
  let lastWarnAt = 0;

  $: meterState = !meterAvailable ? "off" : meterMeasured ? "live" : "pending";

  function paintReadout() {
    if (readEls.peak) readEls.peak.textContent = fmtDb(readValues.peak);
    if (readEls.headroom) readEls.headroom.textContent = fmtDb(readValues.headroom);
    if (readEls.reduction) {
      readEls.reduction.textContent = fmtDb(readValues.reduction);
      readEls.reduction.classList.toggle("lit", readValues.reduction >= 0.1);
    }
  }
  function readout(node, which) {
    readEls[which] = node;
    paintReadout();
    return {
      destroy() {
        if (readEls[which] === node) readEls[which] = null;
      },
    };
  }

  // `use:levelBar={i}`: the node exists only while the bars are rendered, so
  // registration follows mount/unmount instead of a query after every change.
  function levelBar(node, i) {
    fills[i] = node;
    node.style.setProperty("--lvl", String(lastBands[i] || 0));
    return {
      destroy() {
        if (fills[i] === node) fills[i] = null;
      },
    };
  }

  function paintBars(bands) {
    for (let i = 0; i < EQ_BAND_COUNT; i++) {
      const v = Math.max(0, Math.min(1, +bands?.[i] || 0));
      lastBands[i] = v;
      const el = fills[i];
      if (el) el.style.setProperty("--lvl", v.toFixed(3));
    }
  }

  function onSaturation(snap) {
    if (!snap) return;
    const now = nowMs();
    if (!snap.available) {
      if (meterAvailable || readValues.peak !== -96) {
        meterAvailable = false;
        meterMeasured = false;
        readValues.peak = -96;
        readValues.headroom = 96;
        readValues.reduction = 0;
        paintBars(null);
        warn = null;
        warnedTone = "ok";
        holdReduction = 0;
        holdMargin = 0;
        if (warnAnnounce) warnAnnounce = "";
      }
      if (snap.reason !== meterReason) meterReason = snap.reason || "idle";
      if (meterRisk !== "off") meterRisk = "off";
      return;
    }
    if (!meterAvailable) {
      meterAvailable = true;
      meterReason = "";
    }
    if (snap.measured !== meterMeasured) meterMeasured = snap.measured;
    if (now - lastBarsAt >= BAR_PAINT_MS) {
      lastBarsAt = now;
      paintBars(snap.bands);
    }

    const tone = riskTone(snap.risk);
    if (tone !== meterRisk) meterRisk = tone;

    if (now - lastReadAt >= 250) {
      lastReadAt = now;
      const peak = Math.round((+snap.peakDb || 0) * 10) / 10;
      const headroom = Math.round((+snap.headroomDb || 0) * 10) / 10;
      const reduction = Math.round((+snap.reductionDb || 0) * 10) / 10;
      if (
        peak !== readValues.peak ||
        headroom !== readValues.headroom ||
        reduction !== readValues.reduction
      ) {
        readValues.peak = peak;
        readValues.headroom = headroom;
        readValues.reduction = reduction;
        paintReadout();
      }
    }

    if (tone === "ok") {
      holdReduction = 0;
      holdMargin = 0;
      warnedTone = "ok";
      if (warnAnnounce) warnAnnounce = "";
      if (warn) warn = null;
      return;
    }
    // The analyser's 85 ms window still holds the signal from before a volume
    // change. A peak-hold started inside it would keep the old reduction for
    // the whole episode (seen: the button lowered the level, the visible peak
    // fell to 0.8 dB, and the banner insisted on the previous 7.0).
    if (now < holdAfter) return;
    // A peak-hold over the warning: the offer must not flap between 0.6 and 2.1
    // dB at 25 Hz while the person reads it, and the largest reduction of the
    // episode is what the ear actually heard.
    holdReduction = Math.max(holdReduction, +snap.reductionDb || 0);
    holdMargin = Math.max(holdMargin, +snap.marginDb || 0);
    if (tone !== warnedTone) {
      warnedTone = tone;
      warnAnnounce = `${tone === "clip" ? "Saturation" : "Risque de saturation"} : le limiteur réduit de ${fmtDb(holdReduction)}.`;
      lastWarnAt = 0; // paint the banner now, not after the throttle
    }
    if (now - lastWarnAt >= 750) {
      lastWarnAt = now;
      const next = {
        tone,
        reduction: Math.round(holdReduction * 10) / 10,
        margin: Math.round(holdMargin * 10) / 10,
        cut: volumeCutDb(holdMargin),
      };
      if (
        !warn ||
        warn.tone !== next.tone ||
        warn.reduction !== next.reduction ||
        warn.margin !== next.margin ||
        warn.cut !== next.cut
      )
        warn = next;
    }
  }

  // The engine's tap costs about 0.46 s of main thread a minute; it is worth
  // paying only while this card is on screen in a visible page. Scrolling the
  // equalizer out of view, switching tab, or leaving Réglages stops it.
  const meter = createMeterSession(subscribeSaturation, onSaturation);
  $: canReduce = !!warn && !$player.muted && $player.volume > 0.005;

  // A deliberate volume move (the slider, the mute button, or the warning's own
  // "Réduire le volume") invalidates the peak-hold: keeping the old 9.7 dB next
  // to a quieter chain read as "the button did nothing". The next reading
  // rebuilds the banner from the level that is actually playing now.
  let lastVolume = null;
  $: if ($player.volume !== lastVolume) {
    lastVolume = $player.volume;
    holdReduction = 0;
    holdMargin = 0;
    // Let the meter's 85 ms window turn over before the next reading becomes
    // the new hold; see the guard in onSaturation.
    holdAfter = nowMs() + 160;
    lastWarnAt = 0;
    warnedTone = "ok";
    if (warnAnnounce) warnAnnounce = "";
  }

  onMount(() => {
    let io = null;
    const sync = () => {
      if (cardVisible && document.visibilityState !== "hidden") meter.start();
      else meter.stop();
    };
    const onVisibility = () => sync();
    document.addEventListener("visibilitychange", onVisibility);
    if (typeof IntersectionObserver === "function") {
      io = new IntersectionObserver(
        (entries) => {
          cardVisible = !!entries[entries.length - 1]?.isIntersecting;
          sync();
        },
        { rootMargin: "160px 0px" }
      );
      io.observe(eqCard);
    } else {
      cardVisible = true;
      sync();
    }
    revealSelected();
    return () => {
      io?.disconnect();
      document.removeEventListener("visibilitychange", onVisibility);
    };
  });
  onDestroy(() => meter.stop());

  $: unavailableText = native
    ? "Mesure indisponible avec le lecteur de l'application."
    : meterReason === "direct"
      ? "Mesure indisponible en lecture directe : le son ne traverse pas l'égaliseur."
      : "La mesure démarre avec la lecture.";

  // The requested mitigation: the player's volume, optimistically, one toast
  // with undo. A quiet confirmation that the chain is quieter now — no engine
  // write, no automatic change of the sound.
  function reduceVolume() {
    if (!warn) return;
    const cut = warn.cut;
    const before = { volume: $player.volume, muted: $player.muted };
    const target = Math.max(0, before.volume * dbToGain(cut));
    if (target >= before.volume - 0.0005) return;
    player.setVolume(target);
    toasts.push(`Volume réduit de ${fmtDb(cut)}`, "info", 2600, {
      label: "Annuler",
      run: () => {
        player.setVolume(before.volume);
        if (before.muted && !$player.muted) player.toggleMute();
      },
    });
  }
</script>

<section class="card">
  <h2><Icon name="speaker" size={18} /> Sortie audio</h2>
  <p class="muted sub">
    Le chemin du son jusqu'à vos écouteurs, votre enceinte ou l'autoradio.
  </p>
  {#if nativeOffered}
    <button class="toggle native-tg" role="switch" aria-checked={nativeOn} on:click={() => nativePlayer.set(!nativeOn)}>
      <span class="tg-txt">
        <span class="tg-title">Lecteur de l'application</span>
        <span class="tg-hint muted">
          Le son est lu par Android lui-même, hors de la page : il continue écran
          éteint et en Bluetooth comme dans une application de musique.
          {#if nativeDsp}L'égaliseur, les basses et les animations le suivent.{:else}En essai :
          l'égaliseur, les basses et les animations ne le suivent pas encore (mettez l'application à jour).{/if}
        </span>
      </span>
      <span class="sw" class:on={nativeOn}><span class="knob"></span></span>
    </button>
    {#if nativeFailed}
      <div class="out-next native-fail">
        <span class="muted">
          Le lecteur de l'application a échoué sur cet appareil ; la page lit la musique à sa place
          jusqu'à demain ou jusqu'à la prochaine mise à jour de l'application.
        </span>
        <button class="g-retry" on:click={clearNativeFault}>Réessayer maintenant</button>
      </div>
    {/if}
  {/if}
  <div class="seg out-seg" class:dimmed={native} role="radiogroup" aria-label="Sortie audio">
    {#each OUTPUT_MODES as m}
      <button
        class="seg-btn"
        class:sel={$audioOutput === m.id}
        role="radio"
        aria-checked={$audioOutput === m.id}
        aria-label={m.label}
        disabled={native}
        on:click={() => audioOutput.set(m.id)}
      >
        {#if m.short}<span class="long">{m.label}</span><span class="short">{m.short}</span>{:else}{m.label}{/if}
      </button>
    {/each}
  </div>
  {#if !native}<p class="muted out-hint">{OUTPUT_HINTS[$audioOutput] || OUTPUT_HINTS.auto}</p>{/if}
  <div class="out-status" class:direct={plan.direct}>
    <span class="out-ico"><Icon name={route.kind === "bluetooth" ? "bluetooth" : "speaker"} size={16} /></span>
    <span class="out-txt">
      <span class="out-mode">{plan.why === "native" ? "Lecteur Android" : plan.direct ? "Lecture directe" : "Lecture traitée"}</span>
      {#if whyText}<span class="muted out-why">{whyText}</span>{/if}
      {#if plan.why === "glitch" && routeText}<span class="muted out-why">{routeText}</span>{/if}
    </span>
  </div>
  {#if plan.why === "glitch" && verdict}
    <div class="out-next">
      <span class="muted">{retryText}</span>
      <button class="g-retry" on:click={() => retryNow(route.key)}>Réessayer maintenant</button>
    </div>
  {/if}
</section>

<section class="card">
  <h2><Icon name="sliders" size={18} /> Audio</h2>
  <p class="muted sub">
    Ces effets traitent le son dans le navigateur. Ils sont désactivés par
    défaut ; une fois activés, la lecture passe par le processeur audio (ce qui
    peut, sur certains mobiles, affecter la lecture en arrière-plan).
  </p>

  <!-- Master volume ------------------------------------------------------- -->
  <div class="block">
    <div class="block-head">
      <span class="block-title">Volume</span>
      <span class="muted block-hint">
        Volume de sortie général (identique à la barre du lecteur) — accessible
        sur mobile, et pratique pour compenser l'égaliseur ou l'amplification
        des basses.
      </span>
    </div>
    <div class="slider-row">
      <button class="vbtn" on:click={() => player.toggleMute()} aria-label="Muet" title="Muet">
        <Icon name={$player.muted || $player.volume === 0 ? "mute" : "volume"} size={18} />
      </button>
      <input type="range" min="0" max="1" step="0.01" value={$player.muted ? 0 : $player.volume} on:input={(e) => player.setVolume(+e.target.value)} />
      <span class="val">{volPct}%</span>
    </div>
  </div>

  <!-- Volume normalization ------------------------------------------------ -->
  <div class="block">
    <div class="block-head">
      <span class="block-title">Normalisation du volume</span>
      <span class="muted block-hint">
        Gain fixe par titre (selon sa loudness ReplayGain analysée par Deezer)
        pour que tous jouent au même niveau — sans compression ni ajustement
        pendant la lecture.
        {#if plan.direct}En lecture directe, elle ne fait que baisser les titres trop forts.{/if}
      </span>
    </div>
    <div class="seg">
      {#each NORM_LEVELS as n}
        <button class="seg-btn" class:sel={$normalization === n.id} on:click={() => normalization.set(n.id)} title={n.hint}>
          {n.label}
        </button>
      {/each}
    </div>
  </div>

  <!-- Bass boost ---------------------------------------------------------- -->
  <div class="block">
    <div class="block-head">
      <span class="block-title">Amélioration des basses</span>
      <span class="muted block-hint">
        Renforce le grave. Un limiteur automatique évite la saturation.
        {#if dspNote}<span class="off-note">{dspNote}</span>{/if}
      </span>
    </div>
    <div class="slider-row">
      <input type="range" min="0" max="1" step="0.01" value={$bassBoost} on:input={(e) => bassBoost.set(+e.target.value)} />
      <span class="val">{bassPct}%</span>
    </div>
  </div>
</section>

<!-- The equaliser: its presets first (a shortcut to a curve), then the ten
     faders with the live level of each band above them. The meter is read from
     lib/audio/saturation.js, pre-limiter, so it can say how hard the safety
     limiter is working — which is the whole point of the warning. -->
<section class="card eq-card" bind:this={eqCard}>
  <h2><Icon name="sliders" size={18} /> Égaliseur</h2>
  <p class="muted sub">
    Dix bandes de 31 Hz à 16 kHz (±16 dB). Un limiteur de sécurité rattrape les
    pics en fin de chaîne ; le mètre montre le niveau réel de chaque bande et
    l'effort du limiteur.
    {#if dspNote}<span class="off-note">{dspNote}</span>{/if}
  </p>

  <button class="toggle" role="switch" aria-checked={$eqEnabled} on:click={() => eqEnabled.set(!$eqEnabled)}>
    <span class="tg-txt">
      <span class="tg-title">Égaliseur 10 bandes</span>
      <span class="tg-hint muted">
        Ajustez chaque bande (±16 dB), ou partez d'un préréglage.
      </span>
    </span>
    <span class="sw" class:on={$eqEnabled}><span class="knob"></span></span>
  </button>

  <!-- Presets: built-ins and the person's own, all of them reachable --------- -->
  <div class="picker">
    <div class="pgroup">
      <div class="pgroup-head">
        <span class="pgroup-title">Intégrés</span>
        <span class="pcount mono">{BUILTIN_PRESETS.length}</span>
      </div>
      <div class="plist" role="group" aria-label="Préréglages intégrés" bind:this={builtinList}>
        {#each builtins as p (p.id)}
          <button
            class="pitem"
            class:active={activeId === p.id}
            class:picked={pickedId === p.id && activeId !== p.id}
            data-selected={selected?.id === p.id ? "true" : "false"}
            aria-pressed={activeId === p.id}
            title="Appliquer « {p.name} »"
            on:click={() => usePreset(p)}
          >
            <span class="pname">{p.name}</span>
            <span class="pspark" aria-hidden="true">
              {#each p.bands as g}<i style={`height:${sparkHeight(g)}%`}></i>{/each}
            </span>
            {#if activeId === p.id}<span class="pcheck"><Icon name="check" size={14} /></span>{/if}
          </button>
        {/each}
      </div>
    </div>
    <div class="pgroup">
      <div class="pgroup-head">
        <span class="pgroup-title">Mes préréglages</span>
        <span class="pcount mono">{mine.length}</span>
        <button class="padd" on:click={openNaming} aria-label="Nouveau préréglage" title="Nouveau préréglage">
          <Icon name="plus" size={16} />
        </button>
      </div>
      {#if mine.length}
        <div class="plist" role="group" aria-label="Mes préréglages" bind:this={mineList}>
          {#each mine as p (p.id)}
            <button
              class="pitem"
              class:active={activeId === p.id}
              class:picked={pickedId === p.id && activeId !== p.id}
              data-selected={selected?.id === p.id ? "true" : "false"}
              aria-pressed={activeId === p.id}
              title="Appliquer « {p.name} »"
              on:click={() => usePreset(p)}
            >
              <span class="pname">{p.name}</span>
              <span class="pspark" aria-hidden="true">
                {#each p.bands as g}<i style={`height:${sparkHeight(g)}%`}></i>{/each}
              </span>
              {#if activeId === p.id}<span class="pcheck"><Icon name="check" size={14} /></span>{/if}
            </button>
          {/each}
        </div>
      {:else}
        <p class="pempty muted">Aucun préréglage enregistré.</p>
      {/if}
    </div>
  </div>

  <p class="pstate" aria-live="polite">
    {#if selected}
      {#if selected.builtin}
        «&nbsp;{selected.name}&nbsp;» · préréglage intégré, non modifiable.
      {:else if dirty}
        «&nbsp;{selected.name}&nbsp;» · <strong>modifié</strong> — «&nbsp;Mettre à jour&nbsp;» garde ce
        réglage.
      {:else}
        «&nbsp;{selected.name}&nbsp;» · à jour.
      {/if}
    {:else}
      Réglage personnalisé — enregistrez-le pour le retrouver.
    {/if}
  </p>

  {#if naming}
    <div class="naming">
      <input
        bind:this={nameInput}
        bind:value={draftName}
        on:keydown={onNameKey}
        placeholder="Nom du préréglage (ex. Casque)"
        aria-label="Nom du préréglage"
        maxlength="40"
      />
      <button class="pill sm" on:click={confirmSave} disabled={!draftName.trim()}>
        <Icon name="check" size={15} /> Enregistrer
      </button>
      <button class="ghost sm" on:click={cancelSave}>Annuler</button>
    </div>
  {:else}
    <div class="pactions">
      {#if selected?.builtin}
        <button class="save" on:click={openNaming}>
          <Icon name="plus" size={16} /> Enregistrer comme…
        </button>
      {:else if selected}
        <button class="upd" on:click={updateCurrent} disabled={!dirty}>
          <Icon name="check" size={16} /> Mettre à jour
        </button>
        <button class="del" on:click={() => removeCustom(selected)}>
          <Icon name="trash" size={15} /> Supprimer
        </button>
      {:else}
        <button class="save" on:click={openNaming}>
          <Icon name="plus" size={16} /> Enregistrer comme…
        </button>
      {/if}
    </div>
  {/if}

  <!-- The live meter: one snapshot at 25 Hz, bars painted by hand ----------- -->
  <div class="meter-block" class:off={!meterAvailable}>
    <div class="meter-head">
      <span class="meter-title"><Icon name="activity" size={14} /> Niveau par bande</span>
      {#if meterState === "live"}
        <span class="meter-read">
          <span>Pic <span class="mv" use:readout={"peak"}></span></span>
          <span>Marge <span class="mv" use:readout={"headroom"}></span></span>
          <span>Limiteur <span class="mv" use:readout={"reduction"}></span></span>
        </span>
      {:else if meterState === "pending"}
        <span class="meter-read">Mesure en cours…</span>
      {:else}
        <span class="meter-read meter-offtxt">{unavailableText}</span>
      {/if}
    </div>

    {#if warn}
      <div class="warn" class:clip={warn.tone === "clip"}>
        <span class="warn-ico"><Icon name="alert" size={18} /></span>
        <span class="warn-txt">
          <strong>{warn.tone === "clip" ? "Saturation" : "Risque de saturation"}</strong>
          <span class="muted">
            {#if warn.reduction >= 0.1}Le limiteur réduit de {fmtDb(warn.reduction)}.{:else}Le signal touche
              0 dBFS.{/if}
            Marge recommandée : {fmtDb(warn.margin)}.
          </span>
        </span>
        <button class="warn-btn" on:click={reduceVolume} disabled={!canReduce}>
          Réduire le volume de {fmtDb(warn.cut)}
        </button>
      </div>
    {/if}
    <span class="sr-live" aria-live="polite">{warnAnnounce}</span>

    <div class="eq" class:dim={!$eqEnabled} data-risk={meterRisk}>
      {#each EQ_FREQS as hz, i}
        <div class="band">
          <span class="gain">{$eqBands[i] > 0 ? "+" : ""}{$eqBands[i]}</span>
          <div class="band-mid">
            <span class="lvl" aria-hidden="true">
              <span class="lvl-fill" use:levelBar={i}></span>
            </span>
            <input
              class="vert"
              type="range"
              min={EQ_MIN_DB}
              max={EQ_MAX_DB}
              step="1"
              value={$eqBands[i]}
              disabled={!$eqEnabled}
              on:input={(e) => setBand(i, e.target.value)}
              aria-label={freqLabel(hz) + " Hz"}
            />
          </div>
          <span class="freq">{freqLabel(hz)}</span>
        </div>
      {/each}
    </div>
  </div>
</section>

<!-- Transitions ------------------------------------------------------------
     Crossfading needs the Web Audio graph (each track fades through its own
     normalization gain), so it carries the same caveat as the effects above
     and lives in the same tab. -->
<section class="card">
  <h2><Icon name="shuffle" size={18} /> Transitions</h2>
  <p class="muted sub">
    Comment un titre laisse la place au suivant. Le fondu respecte la
    normalisation : chaque titre garde son propre niveau pendant le
    chevauchement, aucun des deux ne saute.
  </p>

  <div class="block">
    <button
      class="toggle"
      role="switch"
      aria-checked={$crossfadeEnabled}
      on:click={() => crossfadeEnabled.set(!$crossfadeEnabled)}
    >
      <span class="tg-txt">
        <span class="tg-title">Fondu enchaîné</span>
        <span class="tg-hint muted">
          Les deux titres se superposent en fin de morceau.
          {plan.direct
            ? "En lecture directe, le fondu se fait sur le volume de chacun."
            : "Comme les effets, cela fait passer la lecture par le moteur audio."}
        </span>
      </span>
      <span class="sw" class:on={$crossfadeEnabled}><span class="knob"></span></span>
    </button>

    <div class="slider-row" class:dim={!$crossfadeEnabled}>
      <span class="s-label">Durée</span>
      <input
        type="range"
        min="0"
        max="12"
        step="0.5"
        value={$crossfadeSeconds}
        disabled={!$crossfadeEnabled}
        on:input={(e) => crossfadeSeconds.set(+e.target.value)}
        style={`--p:${($crossfadeSeconds / 12) * 100}%`}
        aria-label="Durée du fondu"
      />
      <span class="s-val">{fadeLabel}</span>
    </div>
  </div>

  <div class="block" class:dim={!$crossfadeEnabled}>
    <button
      class="toggle"
      role="switch"
      aria-checked={$crossfadeOnSkip}
      disabled={!$crossfadeEnabled}
      on:click={() => crossfadeOnSkip.set(!$crossfadeOnSkip)}
    >
      <span class="tg-txt">
        <span class="tg-title">Adoucir les changements manuels</span>
        <span class="tg-hint muted">
          Suivant et précédent s'enchaînent avec un fondu plus court que celui de
          fin de titre, sans la coupure sèche au milieu de la forme d'onde.
        </span>
      </span>
      <span class="sw" class:on={$crossfadeOnSkip}><span class="knob"></span></span>
    </button>

    <div class="slider-row" class:dim={!$crossfadeEnabled || !$crossfadeOnSkip}>
      <span class="s-label">Durée</span>
      <input
        type="range"
        min="0"
        max="5"
        step="0.25"
        value={$crossfadeSkipSeconds}
        disabled={!$crossfadeEnabled || !$crossfadeOnSkip}
        on:input={(e) => crossfadeSkipSeconds.set(+e.target.value)}
        style={`--p:${($crossfadeSkipSeconds / 5) * 100}%`}
        aria-label="Durée du fondu sur suivant et précédent"
      />
      <span class="s-val">{skipFadeLabel}</span>
    </div>
  </div>

  <div class="block">
    <button
      class="toggle"
      role="switch"
      aria-checked={$trimSilence}
      on:click={() => trimSilence.set(!$trimSilence)}
    >
      <span class="tg-txt">
        <span class="tg-title">Tronquer les silences</span>
        <span class="tg-hint muted">
          Démarre et termine chaque titre là où le son commence et s'arrête
          vraiment. Le serveur mesure le fichier une fois, après son archivage :
          un titre est donc ajusté à partir de la lecture suivante.
        </span>
      </span>
      <span class="sw" class:on={$trimSilence}><span class="knob"></span></span>
    </button>

    <div class="slider-row" class:dim={!$trimSilence}>
      <span class="s-label">Seuil</span>
      <input
        type="range"
        min="-70"
        max="-25"
        step="1"
        value={$trimThresholdDb}
        disabled={!$trimSilence}
        on:input={(e) => trimThresholdDb.set(+e.target.value)}
        style={`--p:${(($trimThresholdDb + 70) / 45) * 100}%`}
        aria-label="Seuil de silence"
      />
      <span class="s-val">{$trimThresholdDb} dB</span>
    </div>
  </div>
</section>

<style>
  /* One row of three, whatever the width: a mode switch that wraps reads as
     two unrelated groups. */
  .out-seg {
    flex-wrap: nowrap;
    max-width: 480px;
  }
  .out-seg .seg-btn {
    flex: 1 1 0;
    min-width: 0;
    min-height: 40px;
    white-space: nowrap;
  }
  .out-seg .short {
    display: none;
  }
  .out-seg.dimmed {
    opacity: 0.45;
  }
  .native-tg {
    padding-bottom: 16px;
  }
  .native-fail {
    margin: -6px 0 16px;
  }
  .native-tg .tg-hint {
    max-width: 62ch;
    line-height: 1.45;
  }
  @media (max-width: 420px) {
    .out-seg .long {
      display: none;
    }
    .out-seg .short {
      display: inline;
    }
  }
  .out-hint {
    font-size: 0.8rem;
    line-height: 1.45;
    margin: 12px 0 0;
  }
  .out-status {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 14px;
    padding: 11px 14px;
    border-radius: 12px;
    background: var(--bg);
    border: 1px solid var(--bg-hover);
  }
  .out-status.direct {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--bg-hover));
  }
  .out-ico {
    flex: none;
    display: flex;
    color: var(--text-dim);
  }
  .out-status.direct .out-ico {
    color: var(--accent);
  }
  .out-txt {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .out-mode {
    font-weight: 600;
    font-size: 0.9rem;
  }
  .out-why {
    font-size: 0.78rem;
    overflow-wrap: anywhere;
  }
  .out-next {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px 14px;
    margin-top: 10px;
    font-size: 0.78rem;
    line-height: 1.4;
  }
  .out-next span {
    flex: 1 1 220px;
  }
  .g-retry {
    flex: none;
    margin-left: auto;
    padding: 8px 14px;
    min-height: 36px;
    border-radius: 999px;
    border: 1px solid var(--bg-hover);
    color: var(--text-dim);
    font-weight: 600;
    font-size: 0.8rem;
  }
  .g-retry:hover {
    color: var(--text);
    border-color: var(--text-dim);
  }
  .off-note {
    color: var(--accent);
    font-weight: 600;
  }
  .s-label {
    min-width: 58px;
    font-size: 0.82rem;
    color: var(--text-dim);
  }
  .s-val {
    min-width: 96px;
    text-align: right;
    font-size: 0.82rem;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .dim {
    opacity: 0.45;
  }
  .card {
    background: var(--bg-card);
    border-radius: var(--radius);
    padding: 20px;
    margin-bottom: 18px;
  }
  .card h2 {
    /* The page's h2 margin (1.7em above) doubled the card's own padding over
       the title: 46 px above it against 20 at the sides. */
    margin-top: 0;
    font-size: 1.05rem;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .sub {
    font-size: 0.85rem;
    margin: 4px 0 14px;
  }
  .block {
    padding: 14px 0;
    border-top: 1px solid var(--bg-hover);
  }
  .block:first-of-type {
    border-top: none;
    padding-top: 4px;
  }
  .block-head {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-bottom: 10px;
  }
  .block-title {
    font-weight: 600;
  }
  .block-hint {
    font-size: 0.78rem;
  }
  .seg {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .seg-btn {
    padding: 8px 14px;
    border-radius: 999px;
    background: var(--bg);
    border: 1px solid var(--bg-hover);
    color: var(--text-dim);
    font-weight: 600;
    font-size: 0.85rem;
  }
  .seg-btn.sel {
    background: var(--ac);
    border-color: var(--ac);
    color: var(--onac);
  }
  .slider-row {
    display: flex;
    align-items: center;
    gap: 14px;
  }
  .vbtn {
    flex: none;
    color: var(--text-dim);
    display: flex;
    align-items: center;
  }
  .vbtn:hover {
    color: var(--text);
  }
  .slider-row input[type="range"] {
    -webkit-appearance: none;
    appearance: none;
    height: 4px;
    border-radius: 2px;
    background: var(--bg-hover);
    flex: 1;
    cursor: pointer;
  }
  .slider-row input[type="range"]::-webkit-slider-thumb {
    -webkit-appearance: none;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--accent);
    cursor: pointer;
  }
  .slider-row input[type="range"]::-moz-range-thumb {
    width: 16px;
    height: 16px;
    border: none;
    border-radius: 50%;
    background: var(--accent);
    cursor: pointer;
  }
  .val {
    font-variant-numeric: tabular-nums;
    font-weight: 600;
    width: 46px;
    text-align: right;
  }
  .toggle {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    width: 100%;
    text-align: left;
    padding: 0 0 12px;
  }
  .tg-txt {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .tg-title {
    font-weight: 600;
  }
  .tg-hint {
    font-size: 0.78rem;
  }
  .sw {
    flex: none;
    width: 44px;
    height: 26px;
    border-radius: 999px;
    background: var(--s2);
    border: 1px solid var(--dim);
    position: relative;
    transition: background 0.15s ease, border-color 0.15s ease;
  }
  .sw.on {
    background: var(--btn);
    border-color: var(--btn);
  }
  .knob {
    position: absolute;
    top: 3px;
    left: 3px;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: var(--tx);
    transition: transform 0.15s ease, background 0.15s ease;
  }
  .sw.on .knob {
    background: var(--onbtn);
    transform: translateX(16px);
  }
  /* -- the equalizer's presets -------------------------------------------- */
  .eq-card {
    /* Semantic marks, not brand colours: the meter has to be readable on all
       three themes, and "safe" must not be the cover's colour on Pochette.
       Lemon on Encre, ink on Papier, the text colour on Pochette; the amber
       and red are dark enough for Papier's cream card. */
    --m-ok: var(--tx);
    --m-warn: #f5b041;
    --m-clip: #ff6b6b;
  }
  :global([data-theme="encre"]) .eq-card {
    --m-ok: var(--ac);
  }
  :global([data-theme="papier"]) .eq-card {
    --m-warn: #8a5200;
    --m-clip: #b3261e;
  }
  .picker {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: 16px;
    margin: 2px 0 10px;
  }
  .pgroup {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  .pgroup-head {
    display: flex;
    align-items: center;
    gap: 7px;
    min-height: 28px;
  }
  .pgroup-title {
    font-size: 0.78rem;
    font-weight: 600;
    color: var(--text-dim);
  }
  .pcount {
    font-size: 0.66rem;
    color: var(--text-dim);
    background: var(--bg);
    border-radius: 999px;
    padding: 1px 7px;
    font-variant-numeric: tabular-nums;
  }
  .padd {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border-radius: 50%;
    border: 1px solid var(--bg-hover);
    color: var(--text-dim);
  }
  .padd:hover {
    color: var(--text);
    border-color: var(--text-dim);
  }
  .plist {
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 236px;
    overflow-y: auto;
    overscroll-behavior: contain;
    padding: 2px;
    margin: -2px;
  }
  .pitem {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    min-height: 44px;
    padding: 0 10px 0 12px;
    border-radius: 10px;
    border: 1px solid var(--bg-hover);
    background: var(--bg);
    color: var(--text);
    text-align: left;
    font-size: 0.85rem;
    font-weight: 600;
  }
  .pitem:hover {
    border-color: var(--text-dim);
  }
  .pitem.active {
    border-color: var(--ac-text);
    background: color-mix(in srgb, var(--ac-text) 8%, var(--bg));
  }
  .pitem.active .pname {
    color: var(--ac-text);
  }
  .pitem.picked:not(.active) {
    border-style: dashed;
    border-color: var(--text-dim);
  }
  .pname {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pspark {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 1px;
    height: 20px;
    width: 52px;
  }
  .pspark i {
    display: block;
    flex: 1;
    min-height: 2px;
    border-radius: 1px;
    background: var(--text-dim);
  }
  .pitem.active .pspark i {
    background: var(--ac-text);
  }
  .pcheck {
    flex: none;
    display: flex;
    color: var(--ac-text);
  }
  .pempty {
    font-size: 0.8rem;
    line-height: 1.4;
    margin: 0;
    padding: 10px 0;
  }
  .pstate {
    min-height: 1.3em;
    margin: 0 0 10px;
    font-size: 0.78rem;
    line-height: 1.4;
    color: var(--text-dim);
  }
  .pstate strong {
    color: var(--text);
  }
  .pactions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    margin-bottom: 8px;
  }
  .upd {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    min-height: 40px;
    padding: 0 16px;
    border-radius: 999px;
    background: var(--ac);
    color: var(--onac);
    font-weight: 700;
    font-size: 0.85rem;
  }
  .upd:disabled {
    background: var(--bg);
    border: 1px solid var(--bg-hover);
    color: var(--text-dim);
    cursor: default;
  }
  .del {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-height: 40px;
    padding: 0 12px;
    border-radius: 999px;
    color: var(--text-dim);
    font-weight: 600;
    font-size: 0.82rem;
  }
  .del:hover {
    color: var(--text);
  }
  .save {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    min-height: 40px;
    padding: 0 15px;
    border-radius: 999px;
    border: 1px dashed var(--bg-hover);
    color: var(--text-dim);
    font-weight: 600;
    font-size: 0.85rem;
  }
  .save:hover {
    color: var(--text);
    border-color: var(--text-dim);
  }
  .naming {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }
  .naming input {
    flex: 1;
    min-width: 180px;
    padding: 9px 14px;
    border-radius: 999px;
    border: 1px solid var(--accent);
    background: var(--bg);
    color: var(--text);
    outline: none;
    font-size: 0.9rem;
  }
  .pill.sm,
  .ghost.sm {
    font-size: 0.85rem;
    padding: 8px 14px;
    gap: 6px;
  }
  .pill.sm:disabled {
    opacity: 0.5;
    cursor: default;
    transform: none;
  }
  .ghost.sm {
    color: var(--text-dim);
  }
  .ghost.sm:hover {
    color: var(--text);
  }
  /* -- the live meter: peak, margin, limiter work, and the warning ---------- */
  .meter-block {
    margin-top: 4px;
  }
  .meter-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 4px 14px;
    margin-bottom: 8px;
  }
  .meter-title {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 0.72rem;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .meter-read {
    display: flex;
    flex-wrap: wrap;
    gap: 2px 14px;
    font-family: var(--font-mono);
    font-size: 0.72rem;
    line-height: 1.4;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .meter-read .lit {
    color: var(--m-warn);
  }
  /* Fixed slots: the numbers change five times a second, the line must not. */
  .meter-read .mv {
    display: inline-block;
    min-width: 7.5ch;
    font-variant-numeric: tabular-nums;
  }
  .meter-block.off .meter-read {
    font-family: inherit;
  }
  .warn {
    display: flex;
    align-items: flex-start;
    flex-wrap: wrap;
    gap: 8px 12px;
    margin: 0 0 12px;
    padding: 12px 14px;
    border-radius: 12px;
    background: color-mix(in srgb, var(--m-warn) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--m-warn) 45%, transparent);
  }
  .warn.clip {
    background: color-mix(in srgb, var(--m-clip) 13%, transparent);
    border-color: color-mix(in srgb, var(--m-clip) 48%, transparent);
  }
  .warn-ico {
    display: flex;
    flex: none;
    padding-top: 1px;
    color: var(--m-warn);
  }
  .warn.clip .warn-ico {
    color: var(--m-clip);
  }
  .warn-txt {
    flex: 1 1 220px;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: 0.8rem;
    line-height: 1.4;
  }
  .warn-txt strong {
    font-size: 0.9rem;
  }
  .warn-btn {
    flex: none;
    margin-left: auto;
    min-height: 40px;
    padding: 0 14px;
    border-radius: 999px;
    border: 1px solid color-mix(in srgb, var(--m-warn) 60%, var(--ln));
    background: var(--bg);
    color: var(--text);
    font-weight: 600;
    font-size: 0.8rem;
  }
  .warn.clip .warn-btn {
    border-color: color-mix(in srgb, var(--m-clip) 60%, var(--ln));
  }
  .warn-btn:hover:not(:disabled) {
    background: var(--bg-card);
  }
  .warn-btn:disabled {
    opacity: 0.55;
    cursor: default;
  }
  .sr-live {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
    border: 0;
  }

  /* -- the ten faders, each with its band's level above it ----------------- */
  .eq {
    display: flex;
    justify-content: space-between;
    gap: 6px;
    overflow-x: auto;
    padding: 4px 2px 0;
  }
  .band {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    flex: 1;
    min-width: 44px;
  }
  .band-mid {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
  }
  .lvl {
    display: flex;
    align-items: flex-end;
    width: 12px;
    height: 48px;
    border-radius: 6px;
    overflow: hidden;
    background: var(--bg);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--tx) 15%, transparent);
  }
  .lvl-fill {
    display: block;
    width: 100%;
    height: 100%;
    border-radius: 6px;
    background: var(--m-ok);
    transform: scaleY(var(--lvl, 0));
    transform-origin: 50% 100%;
    transition: transform 90ms linear, background-color 160ms ease;
  }
  .eq[data-risk="warn"] .lvl-fill {
    background: var(--m-warn);
  }
  .eq[data-risk="clip"] .lvl-fill {
    background: var(--m-clip);
  }
  .gain {
    font-size: 0.74rem;
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
    min-height: 1em;
  }
  .freq {
    font-size: 0.72rem;
    color: var(--text-dim);
  }
  .eq.dim .gain,
  .eq.dim input.vert {
    opacity: 0.45;
  }
  input.vert {
    writing-mode: vertical-lr;
    direction: rtl;
    width: 10px;
    height: 170px;
    -webkit-appearance: none;
    appearance: none;
    background: var(--bg);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--tx) 10%, transparent);
    border-radius: 5px;
    cursor: pointer;
  }
  input.vert:disabled {
    cursor: default;
  }
  input.vert::-webkit-slider-thumb {
    -webkit-appearance: none;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: var(--accent);
    cursor: pointer;
  }
  input.vert::-moz-range-thumb {
    width: 20px;
    height: 20px;
    border: none;
    border-radius: 50%;
    background: var(--accent);
    cursor: pointer;
  }

  /* On phones a horizontal row of ten vertical faders forces horizontal
     scrolling, and dragging to scroll grabs whichever fader is under the
     finger. Rotate the whole EQ for touch: one horizontal fader per row,
     stacked vertically. Page scroll (up/down) and fader drag (left/right)
     are now on perpendicular axes, so they never fight — `touch-action:
     pan-y` makes the browser hand vertical swipes to the page and horizontal
     drags to the fader. The band's meter becomes the thin line above its
     fader, so the level and the slider share the same frequency label. */
  @media (max-width: 640px) {
    .picker {
      grid-template-columns: 1fr;
      gap: 14px;
    }
    .plist {
      flex-direction: row;
      max-height: none;
      overflow-x: auto;
      overflow-y: hidden;
      scroll-snap-type: x mandatory;
      padding: 2px 2px 8px;
      margin: -2px -2px 0;
      overscroll-behavior-x: contain;
      scrollbar-width: none;
    }
    .plist::-webkit-scrollbar {
      display: none;
    }
    .pitem {
      flex: 0 0 auto;
      width: auto;
      max-width: 74vw;
      min-height: 44px;
      padding: 0 14px;
      scroll-snap-align: start;
    }
    .pspark {
      display: none;
    }
    .pname {
      flex: 0 1 auto;
    }
    .eq {
      flex-direction: column;
      justify-content: flex-start;
      gap: 4px;
      overflow-x: visible;
      padding: 4px 0 2px;
    }
    .band {
      flex-direction: row;
      align-items: center;
      gap: 12px;
      width: 100%;
      min-width: 0;
      /* The 26px thumbs draw outside the thin track's box, so give each row a
         generous height — otherwise adjacent thumbs sit on top of each other
         and the whole thing reads as a cramped grid. This is the breathing
         room that makes it feel considered rather than crammed. */
      min-height: 56px;
    }
    .band-mid {
      flex: 1;
      min-width: 0;
      gap: 6px;
      align-items: stretch;
    }
    .lvl {
      width: 100%;
      height: 6px;
      border-radius: 3px;
    }
    .lvl-fill {
      border-radius: 3px;
      transform: scaleX(var(--lvl, 0));
      transform-origin: 0 50%;
    }
    .freq {
      order: -1;
      flex: none;
      width: 40px;
      text-align: left;
      font-size: 0.78rem;
      font-weight: 600;
      color: var(--text);
    }
    .gain {
      flex: none;
      width: 44px;
      text-align: right;
      min-height: 0;
      font-size: 0.78rem;
    }
    input.vert {
      writing-mode: horizontal-tb;
      direction: ltr;
      width: 100%;
      /* 6 px of visible track, 19 px of transparent hit area above and below:
         the thumb is 26 px and the row must stay a 44 px target. The track is
         drawn by the runnable-track pseudo-element so the padding stays empty
         (the input's own background would paint a second line at its top). */
      box-sizing: content-box;
      height: 6px;
      padding: 19px 0;
      background: transparent;
      box-shadow: none;
      border-radius: 0;
      touch-action: pan-y;
    }
    input.vert::-webkit-slider-runnable-track {
      height: 6px;
      border-radius: 3px;
      background: var(--bg);
      box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--tx) 10%, transparent);
    }
    input.vert::-moz-range-track {
      height: 6px;
      border-radius: 3px;
      background: var(--bg);
      box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--tx) 10%, transparent);
    }
    input.vert::-webkit-slider-thumb {
      width: 26px;
      height: 26px;
    }
    input.vert::-moz-range-thumb {
      width: 26px;
      height: 26px;
    }
    .warn-btn {
      flex: 1 1 100%;
      margin-left: 0;
      min-height: 44px;
    }
  }
  @media (pointer: coarse) {
    .seg-btn,
    .padd,
    .save,
    .del,
    .upd,
    .warn-btn,
    .naming input,
    .naming button {
      min-height: 44px;
    }
    .padd {
      width: 44px;
      height: 44px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .lvl-fill {
      transition: none;
    }
  }
</style>
