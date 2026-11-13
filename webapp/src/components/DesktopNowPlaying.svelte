<script>
  import { trackEntityRef } from "../lib/entity.js";
  // Desktop now-playing: Sillon §7 — the sleeve and its named actions on the
  // left, one raised card on the right holding the queue, the lyrics and what
  // the track is. The blurred backdrop and the animation stay as they were.
  import { trackKey, trackRef } from "../lib/trackkey.js";
  import { onMount, onDestroy, tick } from "svelte";
  import { REMOTE, CAN_KEEP, CAN_BROWSE } from "../lib/remote/mode.js";
  import { push } from "svelte-spa-router";
  import { fade } from "svelte/transition";
  import {
    player,
    current,
    playing,
    favorites,
    disliked,
    immersiveOpen,
    seekTo,
    buffered,
    openMenu,
    openPlaylistPicker,
    offlineCovers,
    openShare,
    quality,
  } from "../lib/stores.js";
  import { toggleFavorite, toggleDislike, buildTrackMenu, startTrackRadio } from "../lib/actions.js";
  import { addMarkerAt } from "../lib/markers.js";
  import { duration as fmtDuration, hiResCover, resolveCover, cssUrl } from "../lib/format.js";
  import { nameOf } from "../lib/sources.js";
  import { playbackLabel, playbackBusy } from "../lib/playback.js";
  import { createBackdrop } from "../lib/backdrop.js";
  import { api } from "../lib/api.js";

  // Blurred backdrop — crossfading layers driven by lib/backdrop.js (shared
  // with the mobile view). Resolved through the offline cache so it also works
  // in airplane mode.
  const bg = createBackdrop();
  // The second argument is the same-origin, server-cached copy of the same art,
  // used only if the (Deezer CDN) URL turns out to be unreachable — the backdrop
  // used to just keep the previous track's blur forever in that case.
  $: bg.set(
    resolveCover($offlineCovers, $current?.album?.cover),
    trackRef($current) ? api.coverUrl(trackRef($current)) : ""
  );
  // The cover glow reuses the last *decoded* backdrop (never a loading URL).
  $: glowSrc = $bg.length ? $bg[$bg.length - 1].src : "";
  import {
    vizMode,
    vizWorld,
    flashLevel,
    vizQuality,
    vizPalette,
    vizIntensity,
    vizFps,
    vizBeatDetect,
    vizFullBleed,
    vizScopeOrientation,
    vizScopeColour,
    vizShowStyle,
    ecoMode,
  } from "../lib/stores.js";
  import { effectiveMode, MODE_BY_ID } from "../lib/viz/modes.js";
  import { readout } from "../lib/audio/engine.js";
  import { servedStyleLabel } from "../lib/trackverdict.js";
  import Visualizer from "./Visualizer.svelte";
  import EcoToggle from "./EcoToggle.svelte";
  import { currentLyricLine } from "../lib/lyrics.js";
  import Cover from "./Cover.svelte";
  import Lyrics from "./Lyrics.svelte";
  import PartyButton from "./PartyButton.svelte";
  import RemoteButton from "./RemoteButton.svelte";
  import Icon from "./Icon.svelte";
  import FlowControls from "./FlowControls.svelte";
  import FlowUpNext from "./FlowUpNext.svelte";
  import { flowLive } from "../lib/flow.js";
  import ArtistLine from "./ArtistLine.svelte";
  import QualityMenu from "./QualityMenu.svelte";
  import QueueList from "./QueueList.svelte";
  import NowPlayingMore from "./NowPlayingMore.svelte";
  import SleepButton from "./SleepButton.svelte";
  import SpeedButton from "./SpeedButton.svelte";

  let tab = "queue";
  // How much of the stage goes to reading, decided by what the columns need.
  // 1200 px is where the sleeve column, the player and the queue still hold
  // their comfortable widths; 1600 px adds a fourth column (the facts) without
  // taking width from the other three. Both are Material's large / extra-large
  // window classes, and both are measured in test/fullscreen/run.mjs.
  // Below ROOMY, the lyrics return to the queue card's tabs; below WIDE, so do
  // the facts. That order is the whole point: the queue never gives way first.
  const ROOMY_MQ = "(min-width: 1200px)";
  const WIDE_MQ = "(min-width: 1600px)";
  let roomy = false;
  let wide = false;
  if (typeof window !== "undefined") {
    roomy = window.matchMedia(ROOMY_MQ).matches;
    wide = window.matchMedia(WIDE_MQ).matches;
  }
  onMount(() => {
    const r = window.matchMedia(ROOMY_MQ);
    const w = window.matchMedia(WIDE_MQ);
    const on = () => {
      roomy = r.matches;
      wide = w.matches;
    };
    r.addEventListener("change", on);
    w.addEventListener("change", on);
    return () => {
      r.removeEventListener("change", on);
      w.removeEventListener("change", on);
    };
  });
  // A tab that gained a column of its own must not leave a duplicate view
  // behind in the queue card.
  $: if (roomy && tab === "lyrics") tab = "queue";
  $: if (wide && tab === "more") tab = "queue";

  // The artwork's box, handed to the animation so its scenes lay themselves out
  // AROUND the cover instead of behind it (lib/viz/geometry.js). An element
  // rather than a measurement: the cover is min(330px, 46vh, 100%) wide and
  // centred in a column that is itself centred, and none of that belongs in a
  // scene.
  let coverBox;

  const QUALITY_LABEL = {
    FLAC: "FLAC",
    OPUS_320: "Opus 320",
    OPUS_256: "Opus 256",
    OPUS_192: "Opus 192",
    OPUS_128: "Opus 128",
    OPUS_64: "Opus 64",
  };
  // "Plus ▾": the same menu the rows and the bar's ⋯ open — everything that can
  // be done to this title, minus what the named pills already say.
  function plusMenu(e) {
    if (!$current) return;
    const r = e.currentTarget.getBoundingClientRect();
    const coords = {
      clientX: r.left,
      clientY: r.bottom + 6,
      preventDefault() {},
      stopPropagation() {},
    };
    openMenu(coords, buildTrackMenu($current, go));
  }

  function trackMenu(e) {
    e.preventDefault();
    e.stopPropagation();
    if (!$current) return;
    const coords = { clientX: e.clientX, clientY: e.clientY, preventDefault() {}, stopPropagation() {} };
    openMenu(coords, buildTrackMenu($current, go));
  }

  $: q = $player.queue;
  $: idx = $player.index;

  // Keep the playing track in view in the queue list (first quarter): jump
  // instantly when the queue tab (re)opens, then follow smoothly as it plays.
  let queueVL;
  let firstQueueFollow = true;
  $: followQueue(tab, idx, $immersiveOpen);
  async function followQueue() {
    if (tab !== "queue" || !$immersiveOpen || idx < 0) return;
    await tick();
    // The queue is windowed, so the playing row may not be in the DOM — scroll
    // by index instead of querying for it.
    queueVL?.scrollToIndex(idx, { ratio: 0.25, smooth: !firstQueueFollow });
    firstQueueFollow = false;
  }
  $: if (tab !== "queue" || !$immersiveOpen) firstQueueFollow = true;

  $: fav = $current && $favorites.has(trackKey($current));
  $: no = !!($current && $disliked.has(trackKey($current)));
  $: progress = $player.duration ? ($player.currentTime / $player.duration) * 100 : 0;
  $: bufferedPct = $player.duration
    ? Math.min(100, Math.max(progress, ($buffered / $player.duration) * 100))
    : 0;
  $: repeatIcon = $player.repeat === "one" ? "repeat1" : "repeat";

  function close() {
    immersiveOpen.set(false);
  }
  function go(p) {
    // A queue-only remote control has no screens to go to.
    if (!CAN_BROWSE) return;
    close();
    push(p);
  }
  function seek(e) {
    const t = +e.target.value;
    player.setProgress(t, $player.duration);
    seekTo.set(t);
  }

  // -- what the track is (the À propos tab) ---------------------------------
  // Only what the app already holds: no request of its own (the credits and the
  // similar tracks below ask theirs, keyed on the id).
  $: albumTitle = $current?.album?.title || "";
  $: sourceLabel = $current?.podcast ? "Podcast" : nameOf($current?.source) || "Archive du serveur";
  $: qualityText = QUALITY_LABEL[$quality];
  $: hasFacts = !!$current && !$current.podcast;

  // -- animation -------------------------------------------------------------
  // Settings own the scene, its effort and its palette; Visualizer.svelte owns
  // the canvas and the loop. `effectiveMode` degrades a rhythm-driven scene
  // when the analysis is switched off, so nothing ever renders as a dead canvas.
  $: vmode = effectiveMode($vizMode, $vizBeatDetect, $ecoMode);
  $: fullBleed = $vizFullBleed && (MODE_BY_ID.get(vmode)?.fullBleed ?? false);
  $: stripViz = vmode === "bars";
  // The genre is the SERVER's answer and nothing else (the analyser pins it,
  // audio/engine.js adoptVerdict, and runs no classifier of its own): a track
  // the server has no genre for shows none.
  $: styleLabel = $vizShowStyle ? $servedStyleLabel : "";

  onDestroy(() => {
    bg.destroy();
  });
</script>

<!-- One source for the facts, rendered either in the queue card's À propos tab
     or in its own right-hand column when the stage can afford it. Exactly one
     of the two is mounted, so the credits and the mix are never asked twice. -->
{#snippet aboutContent()}
  {#if hasFacts}
    <div class="facts">
      <div class="fact">
        <span class="label-mono">Artiste</span>
        <span class="v"><ArtistLine track={$current} navigate={go} /></span>
      </div>
      <div class="fact">
        <span class="label-mono">Album</span>
        <span class="v ellipsis" title={albumTitle}>{albumTitle || "—"}</span>
      </div>
      <div class="fact">
        <span class="label-mono">Genre</span>
        <span class="v ellipsis" title={styleLabel}>{styleLabel || "—"}</span>
      </div>
      <div class="fact">
        <span class="label-mono">Qualité</span>
        <span class="v mono">{qualityText}</span>
      </div>
      <div class="fact">
        <span class="label-mono">Durée</span>
        <span class="v mono">{fmtDuration($current.duration)}</span>
      </div>
      <div class="fact">
        <span class="label-mono">Source</span>
        <span class="v ellipsis" title={sourceLabel}>{sourceLabel}</span>
      </div>
    </div>
  {/if}
  <NowPlayingMore track={$current} />
{/snippet}

<div class="d" transition:fade={{ duration: 150 }}>
  {#if $ecoMode}
    {#if $bg.length}
      <div class="bg" style={`background-image:${cssUrl($bg[$bg.length - 1].src)}`}></div>
    {/if}
  {:else}
    {#each $bg as layer (layer.id)}
      <div class="bg" style={`background-image:${cssUrl(layer.src)}`} in:fade={{ duration: 350 }}></div>
    {/each}
  {/if}
  <div class="scrim"></div>

  {#if fullBleed}
    <div class="viz-full" aria-hidden="true">
      <Visualizer
        mode={vmode}
        quality={$vizQuality}
        palette={$vizPalette}
        intensity={$vizIntensity}
        fps={$vizFps}
        layout="full"
        active={$immersiveOpen}
        paused={!$playing}
        scopeOrientation={$vizScopeOrientation}
        scopeColour={$vizScopeColour}
        world={$vizWorld}
        flash={$flashLevel}
        occluder={coverBox}
      />
    </div>
  {/if}

  <header>
    {#if CAN_BROWSE}
      <button class="ic" on:click={close} aria-label="Réduire"><Icon name="chevronDown" size={24} /></button>
    {:else}
      <!-- A queue-only remote control has nothing behind this view. -->
      <span class="ic" aria-hidden="true"></span>
    {/if}
    <span class="ctx">
      <span class="label-mono where">{$player.context?.kind === "flow" ? "Flow" : "En lecture"}</span>
      {#if styleLabel}<span class="genre">{styleLabel}{#if $readout.bpm} · {$readout.bpm} BPM{/if}</span>{/if}
    </span>
    {#if CAN_KEEP}
      <QualityMenu placement="down" />
    {:else}
      <span class="ic" aria-hidden="true"></span>
    {/if}
  </header>

  <div class="stage">
    {#if roomy && CAN_BROWSE}
      <aside class="lyrics-col" aria-label="Paroles">
        <header class="col-h"><span class="label-mono">Paroles</span></header>
        <div class="lyrics-body"><Lyrics /></div>
      </aside>
    {/if}
    <section class="main" class:steer={CAN_KEEP && $flowLive} class:strip={stripViz}>
      <div class="cur-lyric" aria-hidden="true">
        {#if $currentLyricLine && !$ecoMode}
          {#key $currentLyricLine}
            <span in:fade={{ duration: 220 }}>{$currentLyricLine}</span>
          {/key}
        {/if}
      </div>

      <div class="cover" bind:this={coverBox}>
        <div class="glow" style={`background-image:${cssUrl(glowSrc)}`}></div>
        {#key trackKey($current)}
          <div class="cover-fade" in:fade={{ duration: 260 }} out:fade={{ duration: 260 }}>
            <!-- 1000px: 1500 is 2-3× the bytes for a marginal gain, and the CDN
                 generates big sizes on demand (slow). Cover.svelte shows the
                 cached 500px instantly and upgrades when this one is decoded. -->
            <Cover src={hiResCover($current.album?.cover, 1000)} alt={$current.title} kind={$current.podcast ? "podcast" : "album"} fallbackId={trackRef($current)} eager />
          </div>
        {/key}
      </div>

      <div class="info">
        <div class="txt">
          <button class="t" on:click={() => trackEntityRef($current, "album", $current.album) && go("/album/" + trackEntityRef($current, "album", $current.album))}>{$current.title}</button>
          <span class="a" class:status={$playbackLabel}>{#if $playbackLabel}{$playbackLabel}{:else}<ArtistLine track={$current} navigate={go} />{/if}</span>
        </div>
        {#if CAN_KEEP && !$current.podcast}
          <!-- "not this one" is said less often than "more of this", and must
               never be hit for the other: quieter, and apart from the heart. -->
          <button class="fav no" class:on={no} on:click={() => toggleDislike($current)} aria-label={no ? "Retirer de « Je n'aime pas »" : "Je n'aime pas"} aria-pressed={no} title={no ? "Retirer de « Je n'aime pas »" : "Je n'aime pas"}>
            <Icon name={no ? "thumbsDownFilled" : "thumbsDown"} size={19} />
          </button>
        {/if}
        {#if CAN_KEEP}
          <button class="fav" class:on={fav} on:click={() => toggleFavorite($current)} aria-label="Favori" title="Favori">
            <Icon name={fav ? "heartFilled" : "heart"} size={24} />
          </button>
        {/if}
      </div>

      <!-- Named actions: what you came here to do, spelled out. -->
      <div class="pills">
        {#if CAN_KEEP}
          <button class="pill" on:click={() => openPlaylistPicker($current)}><Icon name="plus" size={15} /> Playlist</button>
          <button class="pill" on:click={() => startTrackRadio($current)}><Icon name="radio" size={15} /> Radio du titre</button>
          <button class="pill" on:click={() => openShare($current)}><Icon name="share" size={15} /> Partager</button>
        {/if}
        {#if CAN_BROWSE}
          <button class="pill" on:click={plusMenu} aria-haspopup="menu">Plus <Icon name="chevronDown" size={14} /></button>
        {/if}
      </div>

      <div class="seek">
        <span class="time">{fmtDuration($player.currentTime)}</span>
        <input type="range" min="0" max={$player.duration || 0} value={$player.currentTime} on:input={seek} style={`--p:${progress}%; --b:${bufferedPct}%`} aria-label="Position dans le titre" />
        <span class="time">{fmtDuration($player.duration)}</span>
      </div>

      <div class="controls">
        <button class="sm" class:on={$player.shuffle} class:gone={!CAN_BROWSE} disabled={!CAN_BROWSE} on:click={() => player.toggleShuffle()} aria-label="Aléatoire"><Icon name="shuffle" size={22} /></button>
        <button on:click={() => player.prev()} aria-label="Précédent"><Icon name="prev" size={30} /></button>
        <button class="pp" class:busy={$playbackBusy} on:click={() => player.toggle()} aria-label="Lecture/Pause"><Icon name={$playing ? "pause" : "play"} size={28} /></button>
        <button on:click={() => player.next()} aria-label="Suivant"><Icon name="next" size={30} /></button>
        <button class="sm" class:on={$player.repeat !== "off"} class:gone={!CAN_BROWSE} disabled={!CAN_BROWSE} on:click={() => player.cycleRepeat()} aria-label="Répéter"><Icon name={repeatIcon} size={22} /></button>
      </div>

      {#if CAN_KEEP && $flowLive}<div class="flow-steer"><FlowControls /></div>{/if}

      {#if stripViz}
        <div class="viz">
          <Visualizer
            mode="bars"
            quality={$vizQuality}
            palette={$vizPalette}
            intensity={$vizIntensity}
            fps={$vizFps}
            layout="strip"
            active={$immersiveOpen}
            paused={!$playing}
          />
        </div>
      {/if}

      <div class="footer">
        <div class="vol">
          <button class="sm" on:click={() => player.toggleMute()} aria-label={$player.muted || $player.volume === 0 ? "Rétablir le son" : "Couper le son"}>
            <Icon name={$player.muted || $player.volume === 0 ? "mute" : "volume"} size={19} />
          </button>
          <input class="vol-range" type="range" min="0" max="1" step="0.01" value={$player.muted ? 0 : $player.volume} on:input={(e) => player.setVolume(+e.target.value)} style={`--p:${($player.muted ? 0 : $player.volume) * 100}%`} aria-label="Volume" />
        </div>
        <div class="tools">
          {#if !REMOTE || CAN_KEEP}<SpeedButton />{/if}
          {#if $current.podcast && CAN_KEEP}
            <button
              class="sm"
              on:click={() => addMarkerAt($current, $player.currentTime)}
              title="Marquer cette position"
              aria-label="Marquer cette position"
            >
              <Icon name="bookmarkPlus" size={20} />
            </button>
          {/if}
          {#if !REMOTE}
            <SleepButton size={19} />
            <PartyButton size={19} />
            <RemoteButton size={19} />
          {/if}
          {#if CAN_BROWSE}
            <button class="sm" on:click={trackMenu} aria-label="Plus d'options"><Icon name="more" size={20} /></button>
            <EcoToggle />
          {/if}
        </div>
      </div>
    </section>

    <aside class="side">
      {#if wide}
        <!-- Four columns: the queue's tab row would be a single dead pill.
             Its header aligns with the two reading columns. -->
        <header class="col-h"><span class="label-mono">File d'attente</span></header>
      {:else}
        <div class="tabs">
          <button class:active={tab === "queue"} on:click={() => (tab = "queue")}>File d'attente</button>
          <!-- A queue-only link lends the transport and the queue, not the
               library's reading (the lyrics are served at the "read" level).
               Each tab disappears when its column has its own room, so there is
               never a second copy of the same view. -->
          {#if CAN_BROWSE && !roomy}<button class:active={tab === "lyrics"} on:click={() => (tab = "lyrics")}>Paroles</button>{/if}
          {#if CAN_BROWSE && !$current.podcast && !wide}<button class:active={tab === "more"} on:click={() => (tab = "more")}>À propos</button>{/if}
        </div>
      {/if}
      <div class="side-body">
        {#if tab === "queue"}
          <div class="queue">
            <QueueList items={q} {idx} bind:this={queueVL} showDuration />
            <FlowUpNext />
          </div>
        {:else if tab === "more"}
          {@render aboutContent()}
        {:else}
          <Lyrics />
        {/if}
      </div>
    </aside>

    {#if wide && CAN_BROWSE && !$current.podcast}
      <!-- The facts get their own card only once the stage can hold four
           columns; below that they are the queue card's third tab. -->
      <aside class="about" aria-label="À propos">
        <header class="col-h"><span class="label-mono">À propos</span></header>
        <div class="side-body about-body">{@render aboutContent()}</div>
      </aside>
    {/if}
  </div>
</div>

<style>
  /* A control this remote-control link does not lend: gone, its place kept so
     the transport stays centred. */
  .gone {
    visibility: hidden;
  }
  /* The full-screen player is a room built over the sleeve: its own ground,
     its scrim, and the theme's own text colours — so Papier reads as paper and
     Encre as ink instead of every theme forcing white on a dark blur. */
  .d {
    position: fixed;
    inset: 0;
    z-index: 200;
    color: var(--tx);
    overflow: hidden;
    background: var(--bg);
  }
  .bg {
    position: absolute;
    inset: 0;
    background-size: cover;
    background-position: center;
    filter: blur(64px) saturate(1.35);
    transform: scale(1.25);
  }
  .scrim {
    position: absolute;
    inset: 0;
    background:
      radial-gradient(ellipse 85% 75% at 50% 45%, transparent 0%, color-mix(in srgb, var(--bg) 78%, transparent) 100%),
      linear-gradient(180deg, color-mix(in srgb, var(--bg) 70%, transparent) 0%, color-mix(in srgb, var(--bg) 92%, transparent) 100%);
  }
  header,
  .stage {
    position: relative;
    z-index: 1;
  }
  /* The header's quality badge opens its list DOWN over the stage: the header
     must paint above the card, or the menu slides under it. */
  header {
    z-index: 3;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 16px 24px;
  }
  .ctx {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    min-width: 0;
    text-align: center;
  }
  .ctx .where {
    color: var(--dim);
  }
  .ctx .genre {
    font-family: var(--font-display);
    font-size: 14px;
    font-weight: 600;
    color: var(--tx);
    max-width: 40vw;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ic {
    display: grid;
    place-items: center;
    width: 44px;
    height: 44px;
    border-radius: 50%;
    border: 1px solid var(--ln);
    color: var(--tx);
    flex: none;
  }
  span.ic {
    border-color: transparent;
  }
  .ic:hover {
    background: var(--s2);
  }

  .stage {
    height: calc(100% - 76px);
    display: flex;
    align-items: stretch;
    gap: 32px;
    padding: 0 32px 24px;
    max-width: 1240px;
    margin: 0 auto;
  }
  .main {
    flex: 1;
    min-width: 0;
    max-width: 520px;
    margin: 0 auto;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 18px;
  }

  /* The reading columns (Sillon's raised card, same as the queue's). They only
     exist when their media query holds; the queue keeps its own width so the
     player never has to give up more than the room left over. */
  .lyrics-col,
  .about {
    flex: none;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--s1);
    border: 1px solid var(--ln);
    border-radius: 24px;
    overflow: hidden;
  }
  .lyrics-col {
    width: clamp(250px, 21vw, 300px);
  }
  .about {
    width: clamp(300px, 19vw, 360px);
  }
  .col-h {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    min-height: 56px;
    padding: 16px 18px 8px;
    color: var(--dim);
  }
  .lyrics-body {
    flex: 1;
    min-height: 0;
    padding: 0 18px 18px;
  }
  .about .side-body {
    flex: 1;
    min-height: 0;
    padding: 6px 16px 18px;
  }
  .about-body .facts {
    margin: 6px 0 10px;
  }

  .cover {
    position: relative;
    width: min(330px, 46vh, 100%);
    aspect-ratio: 1 / 1;
    margin: 0 auto;
    flex: none;
  }
  .glow {
    position: absolute;
    inset: -6%;
    background-size: cover;
    background-position: center;
    filter: blur(40px) saturate(1.5);
    opacity: 0.55;
    border-radius: 28%;
    z-index: -1;
  }
  /* keyed crossfade: old + new overlap during the transition */
  .cover-fade {
    position: absolute;
    inset: 0;
  }
  .cover :global(.cover) {
    box-shadow: 0 30px 70px color-mix(in srgb, var(--bg) 70%, transparent);
  }

  /* current synced lyric line, above the cover (up to 3 lines) */
  .cur-lyric {
    min-height: 26px;
    /* grid stack: the crossfading lines share one cell, so the box grows to
       fit the tallest line instead of clipping long ones to a single row */
    display: grid;
    justify-items: center;
    align-items: center;
  }
  .cur-lyric span {
    grid-area: 1 / 1;
    text-align: center;
    font-family: var(--font-display);
    font-size: 1.1rem;
    line-height: 1.3;
    font-weight: 700;
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 3;
    overflow: hidden;
    color: var(--ac-text);
  }

  .info {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .txt {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .t {
    /* A button's default inline padding set the title 6 px right of the
       artist line under it. */
    padding: 0;
    font-family: var(--font-display);
    font-size: 44px;
    line-height: 1;
    letter-spacing: -0.03em;
    font-weight: 800;
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  /* `display: block` is load-bearing: this used to be a <button> and now wraps
     one button per credited artist (each with its own hover), and ellipsis
     doesn't apply to an inline. */
  .a {
    display: block;
    font-size: 18px;
    text-align: left;
    color: var(--dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .t:hover {
    text-decoration: underline;
  }
  .fav {
    display: grid;
    place-items: center;
    width: 48px;
    height: 48px;
    border-radius: 50%;
    color: var(--dim);
    flex: none;
  }
  .fav:hover {
    background: var(--s2);
    color: var(--tx);
  }
  .fav.on {
    color: var(--hrt);
  }
  .fav.no {
    color: var(--dim);
  }
  .fav.no.on {
    color: var(--tx);
  }

  .pills {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .pills .pill {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    height: 38px;
    padding: 0 16px;
    border-radius: 999px;
    border: 1px solid var(--ln);
    background: none;
    color: var(--tx);
    font-size: 14px;
    font-weight: 600;
    white-space: nowrap;
  }
  .pills .pill:hover {
    background: var(--s2);
  }

  .seek {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .time {
    font-family: var(--font-mono);
    font-size: 12px;
    color: var(--dim);
    width: 38px;
    text-align: center;
    font-variant-numeric: tabular-nums;
    flex: none;
  }
  input[type="range"] {
    -webkit-appearance: none;
    appearance: none;
    height: 6px;
    border-radius: 3px;
    flex: 1;
    cursor: pointer;
    background: var(--ln);
  }
  /* Seek bar only (not volume): show the buffered region as a lighter fill. */
  .seek input[type="range"] {
    background: linear-gradient(
      90deg,
      var(--ac) var(--p, 0%),
      color-mix(in srgb, var(--tx) 42%, transparent) var(--p, 0%),
      color-mix(in srgb, var(--tx) 42%, transparent) var(--b, 0%),
      var(--ln) var(--b, 0%)
    );
  }
  .vol-range {
    width: 96px;
  }
  .vol input[type="range"] {
    flex: none;
    height: 4px;
    background: linear-gradient(90deg, var(--ac) var(--p, 0%), var(--ln) var(--p, 0%));
  }
  input[type="range"]::-webkit-slider-thumb {
    -webkit-appearance: none;
    width: 13px;
    height: 13px;
    border-radius: 50%;
    background: var(--tx);
  }
  input[type="range"]::-moz-range-thumb {
    width: 13px;
    height: 13px;
    border: none;
    border-radius: 50%;
    background: var(--tx);
  }

  /* The Flow's steering row (one line in a full column, two when narrow):
     the cover gives up that height so nothing below is pushed off screen
     (test/fullscreen/run.mjs, "Flow" sizes). */
  .flow-steer {
    margin-top: -4px;
  }
  .main.steer .cover {
    width: min(calc(46vh - 72px), 100%);
  }
  .controls {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 26px;
  }
  .controls button {
    color: var(--tx);
    flex: none;
  }
  .controls .sm {
    color: var(--dim);
  }
  .controls .sm.on {
    color: var(--ac-text);
  }
  .controls .pp {
    width: 68px;
    height: 68px;
    border-radius: 50%;
    background: var(--btn);
    color: var(--onbtn);
    display: grid;
    place-items: center;
    position: relative;
  }
  .controls .pp:hover {
    filter: brightness(1.06);
  }
  .controls .pp.busy::after {
    content: "";
    position: absolute;
    inset: -5px;
    border-radius: 50%;
    border: 2px solid transparent;
    border-top-color: var(--tx);
    animation: pp-spin 0.8s linear infinite;
  }
  @keyframes pp-spin {
    to {
      transform: rotate(360deg);
    }
  }
  .a.status {
    color: var(--ac-text);
  }

  .viz {
    position: relative;
    width: 100%;
    height: 44px;
    flex: none;
    opacity: 0.9;
  }
  /* The strip lives in the empty band between the transport and the footer
     instead of hugging the controls: the flexible auto margins put it in the
     middle of that band, footer on the bottom edge. Only when the strip is
     actually drawn (the ± and the bars are one setting) and only while the
     height query leaves the band room — below that the strip is hidden. */
  @media (min-height: 821px) {
    .main.strip > .cur-lyric,
    .main.strip .viz {
      margin-top: auto;
    }
    .main.strip .viz {
      margin-bottom: auto;
    }
  }
  /* Above the blurred cover and its scrim, below the stage: the backdrop
     becomes the animation instead of competing with it. */
  .viz-full {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }

  .footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
  }
  .vol {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .footer .sm {
    color: var(--dim);
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
  }
  .footer .sm:hover {
    color: var(--tx);
  }
  .tools {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  /* right: the raised card (Sillon §7) */
  .side {
    width: clamp(300px, 34vw, 360px);
    flex: none;
    display: flex;
    flex-direction: column;
    background: var(--s1);
    border: 1px solid var(--ln);
    border-radius: 24px;
    overflow: hidden;
  }
  .tabs {
    display: flex;
    gap: 6px;
    padding: 16px 16px 8px;
  }
  .tabs button {
    white-space: nowrap;
    height: 32px;
    padding: 0 14px;
    border-radius: 999px;
    color: var(--dim);
    font-weight: 600;
    font-size: 0.84rem;
  }
  .tabs button:hover {
    color: var(--tx);
  }
  .tabs button.active {
    background: var(--ac);
    color: var(--onac);
  }
  .side-body {
    flex: 1;
    overflow-y: auto;
    padding: 6px 12px 16px;
    min-height: 0;
  }
  .queue {
    margin: 0;
    padding: 0;
    color: var(--tx);
  }

  /* À propos: what the track IS, in six labelled cells (three across, two
     down — a 340 px panel has no room for six stacked lines), then the one
     action it invites, then the long reading under it (credits, the artist,
     the similar titles). */
  .facts {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 6px 14px;
    margin: 6px 4px 10px;
  }
  .fact {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    min-width: 0;
    line-height: 1.5;
  }
  .fact .label-mono {
    flex: none;
    width: 52px;
    font-size: 10px;
    letter-spacing: 0.06em;
  }
  .fact .v {
    display: block;
    font-size: 12.5px;
    color: var(--tx);
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .fact .v.mono {
    font-size: 12px;
  }
  .ellipsis {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  @media (min-width: 1200px) {
    /* Three columns: the sleeve column, the player, the queue. The row is
       centred in a wider stage; the queue card keeps its full width so its
       rows stay readable, and the player takes what is left (still > 470 px
       at 1200, its full 520 from 1244). */
    .stage {
      max-width: 1440px;
      gap: 24px;
      justify-content: center;
    }
  }
  @media (min-width: 1600px) {
    /* Four columns: the facts get their own card to the right of the queue. */
    .stage {
      max-width: 1720px;
    }
    .lyrics-col {
      width: clamp(280px, 18vw, 340px);
    }
  }
  @media (max-width: 900px) {
    .stage {
      gap: 20px;
      padding: 0 20px 20px;
    }
    .controls {
      gap: 18px;
    }
    .tabs {
      gap: 2px;
      padding: 12px 12px 6px;
    }
    .tabs button {
      padding: 0 10px;
    }
  }
  /* The Flow's steering row is 92 px high in the player (the Genres door, the
     two feedback chips and the energy wrap onto two lines), against 40 px for
     the old row. At 900 px high the row and the 44 px strip together push the
     footer off the screen — measured at 1440x900: footer 879-913 in a 900 px
     window. The strip is decoration, the row and the footer are not: the bars
     yield whenever the Flow is live and the window is under 950 px high.
     1920x1080 keeps them; 1440x900 without Flow keeps them too. */
  @media (max-height: 949px) {
    .main.steer .viz {
      display: none;
    }
  }
  /* A narrow window keeps the full layout (the programs this runs in have both
     dimensions): the sleeve gives up width before the named pills wrap into a
     third line. */
  @media (max-width: 1100px) {
    .cover,
    .main.steer .cover {
      width: min(280px, 46vh, 100%);
    }
  }
  /* Short viewports: the fixed-height rows below the sleeve (pills, controls,
     visualizer, volume/tools footer) can still overflow and get clipped.
     Degrade gracefully — the visualizer first, then spacing, then the cover —
     so the transport is never the thing that disappears (swept at nine sizes
     by test/fullscreen/run.mjs). */
  @media (max-height: 820px) {
    .main {
      gap: 14px;
    }
    .viz {
      display: none;
    }
    .cover {
      width: min(330px, 38vh, 100%);
    }
    .main.steer .cover {
      width: min(calc(38vh - 72px), 100%);
    }
  }
  @media (max-height: 700px) {
    .stage {
      padding: 0 24px 16px;
    }
    .main {
      gap: 10px;
    }
    .cover,
    .main.steer .cover {
      width: min(30vh, 100%);
    }
    .t {
      font-size: 32px;
    }
    .a {
      font-size: 16px;
    }
    .controls {
      gap: 18px;
    }
    .controls .pp {
      width: 56px;
      height: 56px;
    }
    .pills .pill {
      height: 34px;
      font-size: 13px;
    }
    .footer {
      gap: 8px;
    }
  }
  @media (max-height: 600px) {
    header {
      padding: 10px 20px;
    }
    .stage {
      height: calc(100% - 64px);
      padding: 0 20px 12px;
    }
    .main {
      gap: 8px;
    }
    .cover,
    .main.steer .cover {
      width: min(24vh, 100%);
    }
    .cur-lyric {
      min-height: 20px;
    }
    .t {
      font-size: 26px;
    }
    .a {
      font-size: 15px;
    }
    .vol-range {
      width: 72px;
    }
  }
</style>
