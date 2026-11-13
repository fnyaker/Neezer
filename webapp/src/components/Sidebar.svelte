<script>
  // Sillon's shell, left column: the disc mark, the five destinations, the
  // playlists and the person. Desktop only — under 640 px the phone shell takes
  // over (MobileNav), so this whole column is display:none.
  //
  // The playlist list comes straight off the shared store (stores.js), so a
  // create / rename / delete from ANY screen lands here immediately. Only the
  // playlists the person owns are listed; the ones a Deezer sync created are
  // grouped behind ONE folded line, off by default — they are the noisy half
  // and the prototype puts them there.
  import { link } from "svelte-spa-router";
  import { tick } from "svelte";
  import { REMOTE, CAN_KEEP, CAN_ADMIN } from "../lib/remote/mode.js";
  import { location } from "../lib/router.js";
  import { user, isAdmin, syncing, playlists, openNewPlaylist } from "../lib/stores.js";
  import { online } from "../lib/net.js";
  import { api } from "../lib/api.js";
  import { userPlaylists, runDeezerSync } from "../lib/actions.js";
  import { isLocalId } from "../lib/format.js";
  import { grooveTint } from "../lib/sillon.js";
  import Icon from "./Icon.svelte";
  import Avatar from "./Avatar.svelte";
  import Cover from "./Cover.svelte";

  // Deezer playlists belong to the account owner — guests don't see them.
  // The load is kicked off once; nothing here re-reads it.
  $: if ($isAdmin) userPlaylists();
  $: shown = $playlists || [];
  $: own = shown.filter((p) => !p.deezer_id);
  $: auto = shown.filter((p) => !!p.deezer_id);
  // Collapsed by default: local disclosure state, no store, nothing persisted.
  let deezerOpen = false;
  let scrollEl;
  let foldEl;

  // Opening it must SHOW what it opened: the playlist box is a scroll area, and
  // the group sits at its end — without this the rows landed below the visible
  // part and the click looked like it did nothing.
  async function toggleDeezer() {
    deezerOpen = !deezerOpen;
    if (!deezerOpen) return;
    await tick();
    scrollEl.scrollTop = Math.max(0, foldEl.offsetTop - 4);
  }

  // The best state we can state without asking anyone: a sync is running, the
  // device is offline, or nothing is wrong that we know of.
  $: status = $syncing ? "Synchronisation…" : !$online ? "Hors ligne" : "Archive à jour";

  async function logout() {
    // On a remote control this account is lent: leaving the loan is the way
    // out, and it hands the browser back its own account (lib/remote/controller.js).
    if (REMOTE) {
      const { leaveRemote } = await import("../lib/remote/controller.js");
      return leaveRemote();
    }
    try {
      await api.logout();
    } catch {
      /* ignore */
    }
    user.set(null);
  }

  // Manual "refresh from Deezer" (shared action), then refresh the list.
  async function syncDeezer() {
    if (await runDeezerSync()) userPlaylists(true);
  }

  // Prefix match (except root) so e.g. /search/<query> keeps "Explorer" lit.
  $: isOn = (path) =>
    $location === path || (path !== "/" && $location.startsWith(path));

  // The list is walked, not windowed: a person's playlists number in the tens,
  // and the scroll box keeps the column's height honest when they do not.
  //
  // The thumbnail is the REAL sleeve when `/me/playlists` gives one (the cover
  // of the playlist's first track, same-origin) — it used to paint the groove
  // tint unconditionally, so the sidebar never showed a picture even when the
  // art existed. `Cover` brings the bounded loader with it (six requests in
  // flight, nearest first, off-screen rows wait), the offline blob, and the
  // groove fallback when the image fails. A playlist with no cover keeps the
  // deterministic tint: a missing sleeve is never an empty square.
  function thumb(p) {
    return grooveTint(p.id || p.title || "");
  }
</script>

<nav class="sidebar" aria-label="Navigation principale">
  <a class="brand" use:link href="/">
    <svg width="28" height="28" viewBox="0 0 32 32" aria-hidden="true">
      <circle cx="16" cy="16" r="15" fill="var(--ac)" />
      <circle cx="16" cy="16" r="10" fill="none" stroke="var(--onac)" stroke-width="1.6" />
      <circle cx="16" cy="16" r="6" fill="none" stroke="var(--onac)" stroke-width="1.6" />
      <circle cx="16" cy="16" r="2" fill="var(--onac)" />
    </svg>
    <span class="word">Neezer</span>
  </a>

  <div class="nav">
    <a use:link href="/" class:active={isOn("/")} aria-current={isOn("/") ? "page" : undefined}>
      <Icon name="home" size={20} /><span class="lbl">Accueil</span>
    </a>
    <a use:link href="/search" class:active={isOn("/search")} aria-current={isOn("/search") ? "page" : undefined}>
      <Icon name="search" size={20} /><span class="lbl">Explorer</span>
    </a>
    <a
      use:link
      href="/library"
      class:active={isOn("/library") || isOn("/podcast")}
      aria-current={isOn("/library") || isOn("/podcast") ? "page" : undefined}
    >
      <Icon name="library" size={20} /><span class="lbl">Bibliothèque</span>
    </a>
    <a use:link href="/releases" class:active={isOn("/releases")} aria-current={isOn("/releases") ? "page" : undefined}>
      <Icon name="sparkles" size={20} /><span class="lbl">Nouveautés</span>
    </a>
    <a use:link href="/settings" class:active={isOn("/settings")} aria-current={isOn("/settings") ? "page" : undefined}>
      <Icon name="settings" size={20} /><span class="lbl">Réglages</span>
    </a>
  </div>

  {#if $isAdmin}
    <div class="pl">
      <div class="pl-head">
        <span class="label-mono">Playlists</span>
        <span class="pl-actions">
          {#if CAN_ADMIN}
            <button
              class="iconbtn"
              class:spin={$syncing}
              on:click={syncDeezer}
              disabled={$syncing}
              title="Synchroniser depuis Deezer"
              aria-label="Synchroniser depuis Deezer"
            >
              <Icon name="refresh" size={16} />
            </button>
          {/if}
          {#if CAN_KEEP}
            <button class="iconbtn" on:click={openNewPlaylist} title="Nouvelle playlist" aria-label="Nouvelle playlist">
              <Icon name="plus" size={16} />
            </button>
          {/if}
        </span>
      </div>

      <div class="pl-scroll" bind:this={scrollEl}>
        {#if own.length}
          <ul class="pl-list">
            {#each own as p (p.id)}
              <li>
                <a use:link href={"/playlist/" + p.id} class="pl-row" class:active={isOn("/playlist/" + p.id)}>
                  <span
                    class="thumb"
                    style={p.cover ? undefined : `background-image:${thumb(p)}`}
                    aria-hidden="true"
                  >
                    {#if p.cover}
                      <Cover
                        src={p.cover}
                        alt=""
                        size={26}
                        kind="playlist"
                        fallbackId={isLocalId(p.id) ? p.id : null}
                      />
                    {/if}
                  </span>
                  <span class="pl-name">{p.title}</span>
                  <span class="pl-n mono">{p.nb_tracks || 0}</span>
                </a>
              </li>
            {/each}
          </ul>
        {:else if $playlists && !auto.length}
          <!-- An empty answer, said plainly; while it is still null the list is
               simply not there yet (a failure leaves it null, not empty). -->
          <p class="pl-empty">Aucune playlist pour l'instant</p>
        {/if}

        {#if auto.length}
          <button
            class="pl-row fold"
            bind:this={foldEl}
            on:click={toggleDeezer}
            aria-expanded={deezerOpen}
            aria-controls={deezerOpen ? "deezer-playlists" : undefined}
          >
            <Icon name={deezerOpen ? "chevronDown" : "chevronRight"} size={14} />
            <span class="pl-name">Deezer · {auto.length} playlist{auto.length > 1 ? "s" : ""} auto</span>
          </button>
          {#if deezerOpen}
            <ul class="pl-list auto" id="deezer-playlists">
              {#each auto as p (p.id)}
                <li>
                  <a use:link href={"/playlist/" + p.id} class="pl-row" class:active={isOn("/playlist/" + p.id)}>
                    <span
                      class="thumb"
                      style={p.cover ? undefined : `background-image:${thumb(p)}`}
                      aria-hidden="true"
                    >
                      {#if p.cover}
                        <Cover
                          src={p.cover}
                          alt=""
                          size={26}
                          kind="playlist"
                          fallbackId={isLocalId(p.id) ? p.id : null}
                        />
                      {/if}
                    </span>
                    <span class="pl-name">{p.title}</span>
                    <span class="pl-n mono">{p.nb_tracks || 0}</span>
                  </a>
                </li>
              {/each}
            </ul>
          {/if}
        {/if}
      </div>
    </div>
  {:else}
    <div class="spacer"></div>
  {/if}

  <div class="account">
    {#if REMOTE}
      <Avatar name={REMOTE.owner} size={32} />
      <span class="who">
        <span class="name">{REMOTE.owner}</span>
        <span class="sub">Contrôle à distance</span>
      </span>
      <button class="out" on:click={logout} title="Quitter le contrôle" aria-label="Quitter le contrôle">
        <Icon name="logOut" size={16} />
      </button>
    {:else}
      <Avatar name={$user?.name || ""} size={32} />
      <span class="who">
        <span class="name">{$user?.name}</span>
        <span class="sub">{status}</span>
      </span>
      <button class="out" on:click={logout} title="Se déconnecter" aria-label="Se déconnecter">
        <Icon name="logOut" size={16} />
      </button>
    {/if}
  </div>
</nav>

<style>
  .sidebar {
    background: var(--s1);
    border-right: 1px solid var(--ln);
    padding: 20px 12px 12px;
    display: flex;
    flex-direction: column;
    gap: 20px;
    overflow: hidden;
    height: 100%;
    min-height: 0;
  }
  /* The phone shell (MobileNav) owns ≤ 640: this column is not there at all,
     so nothing in it requests anything either. */
  @media (max-width: 640px) {
    .sidebar {
      display: none;
    }
  }

  /* -- brand --------------------------------------------------------------- */
  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 10px;
    color: var(--tx);
    border-radius: 10px;
  }
  .brand svg {
    flex: none;
    display: block;
  }
  .word {
    font-family: var(--font-display);
    font-weight: 700;
    font-size: 22px;
    letter-spacing: -0.02em;
    line-height: 1;
  }

  /* -- destinations -------------------------------------------------------- */
  .nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .nav a {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 42px;
    padding: 0 12px;
    border-radius: 12px;
    color: var(--tx);
    font-size: 15px;
    font-weight: 500;
  }
  .nav a:hover {
    background: var(--s2);
  }
  .nav a.active {
    background: var(--s2);
    font-weight: 700;
  }
  .lbl {
    flex: 1;
    text-align: left;
  }

  /* -- playlists ----------------------------------------------------------- */
  .pl {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1 1 auto;
    min-height: 0;
  }
  .pl-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 0 10px 6px;
  }
  .pl-actions {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .iconbtn {
    width: 28px;
    height: 28px;
    display: grid;
    place-items: center;
    border-radius: 8px;
    color: var(--dim);
  }
  .iconbtn:hover:not(:disabled) {
    color: var(--tx);
    background: var(--s2);
  }
  .iconbtn:disabled {
    opacity: 0.55;
    cursor: default;
  }
  .iconbtn.spin :global(svg) {
    animation: spin 1s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .iconbtn.spin :global(svg) {
      animation: none;
    }
  }

  .pl-scroll {
    position: relative;
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
    padding-bottom: 4px;
  }
  .pl-list {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .pl-row {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    height: 40px;
    padding: 0 10px;
    border-radius: 10px;
    color: var(--tx);
    font-size: 14px;
    text-align: left;
  }
  .pl-row:hover {
    background: var(--s2);
  }
  .pl-row.active {
    background: var(--s2);
  }
  .thumb {
    width: 26px;
    height: 26px;
    flex: none;
    border-radius: 6px;
    overflow: hidden;
    background-size: cover;
    background-position: center;
  }
  /* The tile is 26 px: Cover's 12 px corner would read as a circle, so keep the
     sidebar's own radius on the art it renders. */
  .thumb :global(.cover) {
    border-radius: 6px;
  }
  .pl-name {
    flex: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .pl-n {
    font-size: 11px;
    color: var(--dim);
    flex: none;
  }
  .fold {
    gap: 8px;
    color: var(--dim);
    font-size: 13px;
  }
  .fold:hover {
    color: var(--tx);
  }
  .auto .thumb {
    margin-left: 6px;
  }
  .pl-empty {
    margin: 2px 10px 8px;
    font-size: 13px;
    color: var(--dim);
  }
  .spacer {
    flex: 1;
  }

  /* -- the person ---------------------------------------------------------- */
  .account {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 2px 4px 10px;
    border-top: 1px solid var(--ln);
    flex: none;
  }
  .who {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
    line-height: 1.2;
  }
  .name {
    font-size: 14px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sub {
    font-size: 12px;
    color: var(--dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .out {
    width: 32px;
    height: 32px;
    flex: none;
    display: grid;
    place-items: center;
    border-radius: 50%;
    color: var(--dim);
  }
  .out:hover {
    color: var(--tx);
    background: var(--s2);
  }
</style>
