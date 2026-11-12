<script>
  // The player's bar and its full-screen views — the PICTURE of the player,
  // with no engine in it. Player.svelte renders it over the audio it drives;
  // a remote control (lib/remote/controller.js) renders exactly the same one
  // over somebody else's player, which is what "same app, another player"
  // means: every control here goes through the player store and `seekTo`,
  // never through an <audio> element.
  //
  // Sillon §7: three columns — what is playing (open the full-screen view),
  // the transport over a 4 px bar, and the room's controls with the quality
  // badge at their head.
  import { trackKey, trackRef } from "../lib/trackkey.js";
  import { push } from "svelte-spa-router";
  import {
    player,
    current,
    favorites,
    nowPlayingOpen,
    immersiveOpen,
    seekTo,
    buffered,
    openMenu,
    openPlaylistPicker,
  } from "../lib/stores.js";
  import { playbackLabel, playbackBusy } from "../lib/playback.js";
  import { toggleFavorite, buildTrackMenu } from "../lib/actions.js";
  import { duration as fmtDuration, artistLine } from "../lib/format.js";
  import { REMOTE, atLeast } from "../lib/remote/mode.js";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";
  import ImmersivePlayer from "./ImmersivePlayer.svelte";
  import PartyButton from "./PartyButton.svelte";
  import RemoteButton from "./RemoteButton.svelte";
  import SleepButton from "./SleepButton.svelte";
  import SpeedButton from "./SpeedButton.svelte";
  import QualityMenu from "./QualityMenu.svelte";

  $: fav = $current && $favorites.has(trackKey($current));

  function trackMenu(e) {
    e.preventDefault();
    e.stopPropagation();
    if (!$current) return;
    const coords = { clientX: e.clientX, clientY: e.clientY, preventDefault() {}, stopPropagation() {} };
    openMenu(coords, buildTrackMenu($current, push));
  }

  // Through `seekTo`, like every other view: the engine chases it on a live
  // stream (performSeek), a remote control sends it to the player it drives.
  function seek(e) {
    seekTo.set(+e.target.value);
  }

  // What a remote control may touch: a narrower level sees the bar without the
  // controls it could not use (the server would refuse them anyway).
  const canKeep = atLeast("full");
  const canQueue = atLeast("read");

  $: progress = $player.duration ? ($player.currentTime / $player.duration) * 100 : 0;
  // Buffered region (lighter fill), clamped to never read below the playhead.
  $: bufferedPct = $player.duration
    ? Math.min(100, Math.max(progress, ($buffered / $player.duration) * 100))
    : 0;
  $: repeatIconName = $player.repeat === "one" ? "repeat1" : "repeat";
</script>

<!-- the <audio> elements are created and managed in JS (see makeEl/switchQuality) -->

<footer class="player">
  <!-- now playing (left / tap to open the immersive view) -->
  <div class="left">
    <button class="now" on:click={() => immersiveOpen.set(true)} aria-label="Ouvrir le lecteur plein écran">
      {#if $current}
        <Cover src={$current.album?.cover} alt={$current.title} size={56} kind={$current.podcast ? "podcast" : "album"} fallbackId={trackRef($current)} eager />
        <span class="info">
          <span class="t">{$current.title}</span>
          <!-- While the player is working toward playback, the subtitle line says
               WHAT is happening (Chargement…, Nouvel essai…) instead of the artist,
               so silence under a "playing" icon is never unexplained. -->
          <span class="a" class:status={$playbackLabel}>{$playbackLabel || artistLine($current)}</span>
        </span>
      {:else}
        <span class="ph">Rien en lecture</span>
      {/if}
    </button>

    {#if $current && canKeep}
      <button class="fav desk" class:on={fav} on:click={() => toggleFavorite($current)} aria-label="Favori" title="Favori">
        <Icon name={fav ? "heartFilled" : "heart"} size={18} />
      </button>
      <button class="sm add" on:click={() => openPlaylistPicker($current)} aria-label="Ajouter à une playlist" title="Ajouter à une playlist">
        <Icon name="plus" size={18} />
      </button>
    {/if}
  </div>

  <!-- transport, over its own 4 px bar (the prototype's one line, centred) -->
  <div class="controls">
    <div class="transport">
      {#if canQueue}
        <button class="sm shuf" class:on={$player.shuffle} on:click={() => player.toggleShuffle()} aria-label="Aléatoire"><Icon name="shuffle" size={18} /></button>
      {/if}
      <button class="prev" on:click={() => player.prev()} aria-label="Précédent"><Icon name="prev" size={22} /></button>
      <button class="pp" class:busy={$playbackBusy} on:click={() => player.toggle()} aria-label="Lecture/Pause">
        <Icon name={$player.playing ? "pause" : "play"} size={18} />
      </button>
      <button class="next" on:click={() => player.next()} aria-label="Suivant"><Icon name="next" size={22} /></button>
      {#if canQueue}
        <button class="sm rep" class:on={$player.repeat !== "off"} on:click={() => player.cycleRepeat()} aria-label="Répéter">
          <Icon name={repeatIconName} size={18} />
        </button>
      {/if}
    </div>
    <div class="seek">
      <span class="time">{fmtDuration($player.currentTime)}</span>
      <input
        type="range"
        min="0"
        max={$player.duration || 0}
        value={$player.currentTime}
        on:input={seek}
        style={`--p:${progress}%; --b:${bufferedPct}%`}
        aria-label="Position dans le titre"
      />
      <span class="time">{fmtDuration($player.duration)}</span>
    </div>
  </div>

  <!-- the room: quality, the devices, the queue, the level, then the views -->
  <div class="extra">
    {#if canKeep}<QualityMenu />{/if}
    {#if canKeep}<SpeedButton />{/if}
    <!-- A remote control lends neither: the party plays from THIS device's
         engine, and links are the owner's to make (and the sleep timer runs
         where the audio is). -->
    {#if !REMOTE}
      <SleepButton size={17} />
      <PartyButton size={17} />
      <RemoteButton size={17} />
    {/if}
    {#if canQueue}
      <button class="sm" class:on={$nowPlayingOpen} on:click={() => nowPlayingOpen.update((v) => !v)} title="File d'attente" aria-label="File d'attente">
        <Icon name="queue" size={18} />
      </button>
    {/if}
    <button class="sm vol-ic" on:click={() => player.toggleMute()} aria-label={$player.muted || $player.volume === 0 ? "Rétablir le son" : "Couper le son"}>
      <Icon name={$player.muted || $player.volume === 0 ? "mute" : "volume"} size={18} />
    </button>
    <input
      class="vol"
      type="range"
      min="0"
      max="1"
      step="0.01"
      value={$player.muted ? 0 : $player.volume}
      on:input={(e) => player.setVolume(+e.target.value)}
      aria-label="Volume"
      style={`--p:${($player.muted ? 0 : $player.volume) * 100}%`}
    />
    {#if canQueue}
      <button class="sm" on:click={trackMenu} title="Plus d'options" aria-label="Plus d'options"><Icon name="more" size={18} /></button>
    {/if}
    <button class="sm max" on:click={() => immersiveOpen.set(true)} title="Plein écran" aria-label="Plein écran"><Icon name="maximize" size={17} /></button>
  </div>
</footer>

<!-- mobile full-screen now playing -->
<ImmersivePlayer />

<style>
  .player {
    position: fixed;
    bottom: 0;
    left: 0;
    right: 0;
    height: var(--player-h);
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1.3fr) minmax(0, 1fr);
    align-items: center;
    gap: 16px;
    padding: 0 20px;
    background: var(--s1);
    border-top: 1px solid var(--ln);
    z-index: 50;
  }

  /* left: the sleeve, the names, and the two quiet actions */
  .left {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
  }
  .now {
    display: flex;
    align-items: center;
    gap: 14px;
    min-width: 0;
    padding: 0;
    text-align: left;
    color: var(--tx);
    border-radius: 10px;
  }
  .now:hover .t {
    text-decoration: underline;
  }
  .now .info {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .t,
  .a {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .t {
    font-size: 15px;
    font-weight: 600;
    line-height: 1.2;
  }
  .a {
    font-size: 13px;
    color: var(--dim);
    line-height: 1.2;
  }
  .a.status {
    color: var(--ac);
  }
  .ph {
    color: var(--dim);
    padding-left: 4px;
  }
  .fav {
    flex: none;
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    border-radius: 50%;
    color: var(--dim);
  }
  .fav:hover {
    color: var(--tx);
  }
  .fav.on {
    color: var(--hrt);
  }
  .add {
    flex: none;
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    border-radius: 50%;
    color: var(--dim);
  }
  .add:hover {
    color: var(--tx);
    background: var(--s2);
  }

  /* centre: transport over the bar */
  .controls {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .transport {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 16px;
  }
  .transport button {
    color: var(--tx);
  }
  .transport .sm {
    color: var(--dim);
  }
  .transport .sm:hover {
    color: var(--tx);
  }
  .transport .sm.on {
    color: var(--ac);
  }
  .pp {
    width: 46px;
    height: 46px;
    border-radius: 50%;
    background: var(--btn);
    color: var(--onbtn);
    display: grid;
    place-items: center;
    position: relative;
    flex: none;
  }
  .pp:hover {
    filter: brightness(1.06);
  }
  /* Discreet spinner ring around the play/pause button while the player is
     working toward playback (loading / buffering / archiving / retrying). */
  .pp.busy::after {
    content: "";
    position: absolute;
    inset: -4px;
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
  .seek {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    max-width: 520px;
  }
  .time {
    font-family: var(--font-mono);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    color: var(--dim);
    min-width: 34px;
    text-align: center;
  }
  input[type="range"] {
    -webkit-appearance: none;
    appearance: none;
    flex: 1;
    height: 4px;
    border-radius: 2px;
    background: var(--ln);
    cursor: pointer;
  }
  .seek input[type="range"] {
    background: linear-gradient(
      90deg,
      var(--ac) var(--p, 0%),
      color-mix(in srgb, var(--tx) 32%, transparent) var(--p, 0%),
      color-mix(in srgb, var(--tx) 32%, transparent) var(--b, 0%),
      var(--ln) var(--b, 0%)
    );
  }
  .vol {
    background: linear-gradient(90deg, var(--ac) var(--p, 0%), var(--ln) var(--p, 0%));
  }
  input[type="range"]::-webkit-slider-thumb {
    -webkit-appearance: none;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--tx);
  }
  input[type="range"]::-moz-range-thumb {
    width: 10px;
    height: 10px;
    border: none;
    border-radius: 50%;
    background: var(--tx);
  }
  input.vol {
    width: 84px;
    flex: none;
  }

  /* right: the room's controls */
  .extra {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 4px;
    min-width: 0;
  }
  .extra .sm {
    color: var(--dim);
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    border-radius: 8px;
    flex: none;
  }
  .extra .sm:hover {
    color: var(--tx);
    background: var(--s2);
  }
  .extra .sm.on {
    color: var(--ac);
  }
  .extra .max {
    color: var(--tx);
  }

  /* Not every right-hand column has the room for nine controls: the level
     goes first (it is also in the full-screen view), then the slider. */
  @media (max-width: 1440px) {
    .extra {
      gap: 2px;
    }
    input.vol {
      width: 72px;
    }
  }
  @media (max-width: 1180px) {
    input.vol {
      display: none;
    }
  }

  /* narrow desktop: drop shuffle/repeat to the second row flanking the seek
     bar instead of cramming everything on one line and squeezing the play
     button. */
  @media (min-width: 641px) and (max-width: 1024px) {
    .player {
      gap: 12px;
      padding: 0 16px;
    }
    .transport {
      gap: 12px;
    }
    .extra {
      gap: 0;
    }
    .extra .max {
      display: none; /* tap the track to open the full-screen player */
    }
  }

  /* mobile (phone-sized): just the now-playing + play/next, tap to expand */
  @media (max-width: 640px) {
    .player {
      grid-template-columns: minmax(0, 1fr) auto auto;
      height: 60px;
      gap: 8px;
      padding: 0 12px;
    }
    .controls .seek,
    .fav.desk,
    .add {
      display: none;
    }
    /* Keep only the fullscreen toggle from the extras cluster — the quality
       badge, the volume, etc. don't fit a narrow bar, but the fullscreen
       button (class "max") must stay reachable. */
    .extra > :not(.max) {
      display: none;
    }
    .extra {
      gap: 0;
    }
    .transport {
      gap: 10px;
    }
    .transport .shuf,
    .transport .rep {
      display: none;
    }
    .t,
    .a {
      max-width: 46vw;
    }
  }
</style>
