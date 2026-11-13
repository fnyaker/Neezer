<script>
  // Explorer's way in when nothing is typed: the styles this person actually
  // LISTENS to, as parents of the genre tree — never a terminal leaf, whose
  // label alone says nothing about the shelf behind it. A tap opens one
  // style's discovery: a 1–5 breadth slider ("Prudent" → "Large"), server-side
  // discovery on Deezer, then playlists whose titles were checked against the
  // style. The search box above keeps doing what it alone does: titles,
  // artists, albums, podcasts. A style is no longer a search term.
  //
  // Requests, in order: ONE for the styles when the landing paints (kept on the
  // device and painted from there first), nothing at all until "Découvrir", the
  // discovery POST, then one GET every 2 s while the server works. The answers
  // are kept for the session, per style and level: reopening a panel paints
  // what was found instead of asking again.
  import { onDestroy, onMount, tick } from "svelte";
  import { api, failureCause } from "../lib/api.js";
  import { player, toasts } from "../lib/stores.js";
  import { playEntity } from "../lib/actions.js";
  import { grooveBackground } from "../lib/sillon.js";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";

  // The discovery's breadth, in five notches: the number the server takes, the
  // words the person reads. "Prudent" stays inside the style; "Large" asks for
  // everything Deezer relates to it.
  const LEVELS = [
    { n: 1, name: "Prudent", hint: "Des playlists qui restent dans le style, sans surprise." },
    { n: 2, name: "Mesuré", hint: "Le style, avec quelques voisins immédiats." },
    { n: 3, name: "Équilibré", hint: "Le style et ce qui l'entoure." },
    { n: 4, name: "Curieux", hint: "Le style, ses voisins et des passerelles." },
    { n: 5, name: "Large", hint: "Tout ce que Deezer rapproche de ce style." },
  ];
  const LEVEL_KEY = "explorer.discover.level";
  const STYLES_KEY = "explorer.styles.v1";
  const POLL_MS = 2000;
  // Two minutes of polling covers a real Deezer sweep on a slow link; past it
  // the panel stops asking on its own and hands the person a way out rather
  // than an endless skeleton (rule 3).
  const MAX_POLLS = 60;
  const TILES_MAX = 10;

  // Discovery results by `style|level`, for this page session only: the
  // payload carries playlists and their checked titles, too big for
  // localStorage, and re-asking a finished sweep would be one more cost the
  // person did not ask for.
  const results = new Map();

  // -- pure helpers ----------------------------------------------------------

  // The parent a row is DISPLAYED as. The server promises parents only; if a
  // leaf ever comes through anyway, its parent (or family) is the label, and
  // rows that name the same parent merge — two leaves must never look like two
  // main entries. A leaf with no parent at all is not a way in: dropped.
  function parentStyles(raw) {
    const byName = new Map();
    for (const s of Array.isArray(raw) ? raw : []) {
      if (!s || typeof s.name !== "string" || !s.name.trim()) continue;
      const parent = typeof s.parent === "string" && s.parent.trim() ? s.parent : null;
      const family = typeof s.family === "string" && s.family.trim() ? s.family : null;
      if (s.leaf && !parent && !family) continue;
      const name = s.leaf ? parent || family : s.name;
      const row = byName.get(name) || { name, tracks: 0, weight: 0, leaf: false, children: [] };
      row.tracks += Number(s.tracks) || 0;
      row.weight += Number(s.weight) || 0;
      if (s.leaf) row.leaf = true;
      if (s.name !== name) row.children.push(s.name);
      byName.set(name, row);
    }
    // Most listened first: the server already sorts, but a merge above can
    // reorder what it sent.
    return [...byName.values()].sort((a, b) => b.tracks - a.tracks);
  }

  const countLabel = (n) => `${Number(n || 0).toLocaleString("fr-FR")} titre${Number(n) === 1 ? "" : "s"}`;

  function verifiedLabel(p) {
    const matched = Number.isFinite(Number(p.matched)) ? Number(p.matched) : null;
    if (matched == null) return p.nb_tracks ? countLabel(p.nb_tracks) : "Titres vérifiés";
    if (p.nb_tracks && p.nb_tracks > matched) return `${countLabel(matched)} vérifiés sur ${Number(p.nb_tracks).toLocaleString("fr-FR")}`;
    return `${countLabel(matched)} vérifiés`;
  }

  function playlistOf(p) {
    return {
      id: p?.id,
      title: typeof p?.title === "string" && p.title.trim() ? p.title : "Playlist sans titre",
      cover: p?.cover || null,
      nb_tracks: Number(p?.nb_tracks) || 0,
      matched: Number.isFinite(Number(p?.matched)) ? Number(p.matched) : null,
      tracks: Array.isArray(p?.tracks) ? p.tracks.filter(Boolean) : [],
    };
  }

  // The discovery POST is the one endpoint lib/api.js does not expose yet; the
  // contract is Explorer's own. A POST is never retried here: "Découvrir" is
  // the person's action, and a failed one offers "Réessayer".
  async function postDiscover(style, level) {
    let res;
    try {
      res = await fetch("/api/explorer/discover", {
        method: "POST",
        credentials: "include",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ style, level }),
      });
    } catch {
      throw { status: 0, message: "network", offline: true };
    }
    let data = {};
    try {
      data = (await res.json()) || {};
    } catch {
      /* the status says enough */
    }
    if (!res.ok) {
      throw {
        status: res.status,
        code: res.headers.get("X-NS-Error") || undefined,
        message: data.error || res.statusText,
      };
    }
    return data;
  }

  function readStylesCache() {
    try {
      const v = JSON.parse(localStorage.getItem(STYLES_KEY) || "null");
      // RAW, as the server sent it: normalising on the way in would leave a
      // merged parent flagged as a leaf, and the second pass would drop it.
      return Array.isArray(v?.styles) && v.styles.length ? v.styles : null;
    } catch {
      return null;
    }
  }

  function writeStylesCache(raw) {
    try {
      localStorage.setItem(STYLES_KEY, JSON.stringify({ at: Date.now(), styles: raw }));
    } catch {
      /* a convenience: the screen still works */
    }
  }

  function readLevel() {
    try {
      const v = parseInt(localStorage.getItem(LEVEL_KEY) || "3", 10);
      return v >= 1 && v <= 5 ? v : 3;
    } catch {
      return 3;
    }
  }

  // -- state -----------------------------------------------------------------

  let rows = []; // raw, as the server sent them
  let loaded = false; // an answer (or a cached one) has painted
  let loading = false;
  let loadFailed = false;
  let loadCause = "";

  let picked = null;
  let level = readLevel();
  let phase = "idle"; // idle | pending | ready | empty | error
  let playlists = [];
  let errText = "";
  let errCause = "";

  let polls = 0;
  let timer = null;
  let seq = 0; // bumped by every change of aim: an older answer is stale

  let dialog = null;
  let returnFocus = null;

  $: styles = parentStyles(rows).slice(0, TILES_MAX);
  $: maxTracks = Math.max(1, ...styles.map((s) => s.tracks));
  $: aim = LEVELS[level - 1] || LEVELS[2];
  $: pending = phase === "pending";
  // What is already on the device, when the network could not confirm it.
  $: stale = loaded && loadFailed && styles.length > 0;

  onMount(() => {
    const cached = readStylesCache();
    if (cached) {
      rows = cached;
      loaded = true;
    }
    loadStyles();
  });

  onDestroy(() => {
    stopPoll();
    seq++;
  });

  async function loadStyles(force = false) {
    if (loading) return;
    loading = true;
    loadFailed = false;
    loadCause = "";
    try {
      const r = await api.get("/explorer/styles");
      rows = Array.isArray(r?.styles) ? r.styles : [];
      loaded = true;
      writeStylesCache(rows);
    } catch (e) {
      loadFailed = true;
      loadCause = failureCause(e);
      // Nothing on screen and nothing on disk: say so, with a way to retry.
      if (!styles.length) loaded = false;
      if (force) toasts.push("Impossible d'actualiser vos styles", "error");
    } finally {
      loading = false;
    }
  }

  // -- the discovery ---------------------------------------------------------

  function stopPoll() {
    clearTimeout(timer);
    timer = null;
  }

  function remember() {
    if (!picked) return;
    const key = `${picked.name}|${level}`;
    // Bounded: five breadths on ten styles is fifty payloads of checked
    // titles. Eight is more than a session explores, and the least recently
    // written goes first (Map keeps insertion order; re-set to refresh).
    results.delete(key);
    results.set(key, {
      phase,
      playlists: playlists.slice(),
      errText,
      errCause,
    });
    while (results.size > 8) results.delete(results.keys().next().value);
  }

  // Paint whatever this style + level already knows, and resume a sweep that
  // was still running when the panel was last closed.
  function restore() {
    stopPoll();
    seq++;
    const kept = results.get(`${picked.name}|${level}`);
    phase = kept?.phase || "idle";
    playlists = kept?.playlists || [];
    errText = kept?.errText || "";
    errCause = kept?.errCause || "";
    if (phase === "pending") {
      polls = 0;
      schedule();
    }
  }

  function chooseStyle(s) {
    picked = s;
    returnFocus = typeof document !== "undefined" ? document.activeElement : null;
    restore();
    tick().then(() => dialog?.focus?.({ preventScroll: true }));
  }

  function setLevel(v) {
    const n = Math.max(1, Math.min(5, Number(v) || 3));
    if (n === level) return;
    level = n;
    try {
      localStorage.setItem(LEVEL_KEY, String(n));
    } catch {
      /* a convenience */
    }
    restore();
  }

  function close() {
    stopPoll();
    seq++;
    picked = null;
    const back = returnFocus;
    returnFocus = null;
    tick().then(() => {
      if (back && back.isConnected) back.focus?.({ preventScroll: true });
    });
  }

  function fail(e, mine) {
    if (mine !== seq) return;
    stopPoll();
    phase = "error";
    const msg = typeof e === "string" ? e : e?.message;
    // The server's own French when it has one; otherwise the HTTP/network cause
    // `failureCause` names. Never both: "HTTP 503 : Deezer ne répond pas"
    // doubled the same sentence back at the reader.
    errText =
      typeof msg === "string" && msg && msg !== "network" ? msg : failureCause(e) || "La découverte n'a pas abouti.";
    errCause = "";
    remember();
  }

  // One answer from the server, whatever produced it. A ready sweep paints,
  // an error names its cause, a pending one waits two more seconds.
  function handle(r, mine) {
    if (mine !== seq) return;
    if (r?.state === "error" || r?.error) {
      fail(r?.error ? String(r.error) : "La découverte n'a pas abouti.", mine);
      return;
    }
    if (r?.state === "ready") {
      stopPoll();
      playlists = (Array.isArray(r.playlists) ? r.playlists : []).map(playlistOf);
      phase = playlists.length ? "ready" : "empty";
      errText = "";
      errCause = "";
      remember();
      return;
    }
    phase = "pending";
    remember();
    if (polls >= MAX_POLLS) {
      fail("La découverte prend plus de temps que prévu. Réessayez dans un moment.", mine);
      return;
    }
    schedule();
  }

  function schedule() {
    stopPoll();
    const mine = seq;
    timer = setTimeout(async () => {
      timer = null;
      polls++;
      try {
        const r = await api.get(
          `/explorer/discover?style=${encodeURIComponent(picked?.name || "")}&level=${level}`
        );
        handle(r, mine);
      } catch (e) {
        fail(e, mine);
      }
    }, POLL_MS);
  }

  async function discover() {
    if (!picked || pending) return;
    stopPoll();
    const mine = ++seq;
    phase = "pending";
    playlists = [];
    errText = "";
    errCause = "";
    polls = 0;
    remember();
    try {
      const r = await postDiscover(picked.name, level);
      handle(r, mine);
    } catch (e) {
      fail(e, mine);
    }
  }

  // A ready sweep with nothing in it is not a failure: the style may be too
  // narrow for Deezer's playlists. Offer the one move that can find something.
  function widen() {
    if (level >= 5) return;
    setLevel(level + 1);
    // A level already discovered paints itself; only an unknown one asks.
    if (phase === "idle" || phase === "empty" || phase === "error") discover();
  }

  function play(p) {
    const tracks = (p.tracks || []).filter((t) => t && (t.deezer_id || t.id));
    if (tracks.length) {
      // The checked titles are already here: no second request, and what plays
      // is exactly what the panel counted.
      player.playQueue(tracks, 0, { kind: "playlist", id: p.id });
    } else {
      playEntity("playlist", p.id);
    }
    close();
  }

  // -- keyboard --------------------------------------------------------------

  function onKey(e) {
    if (!picked) return;
    if (e.key === "Escape") {
      // Capture: if this sheet were opened over something with its own Escape
      // (a player), the key closes THIS layer only.
      e.preventDefault();
      e.stopImmediatePropagation();
      close();
      return;
    }
    if (e.key !== "Tab" || !dialog) return;
    const f = [...dialog.querySelectorAll("button, [href], input, [tabindex]:not([tabindex='-1'])")].filter(
      (el) => !el.disabled && el.offsetParent !== null
    );
    if (!f.length) return;
    const first = f[0];
    const last = f[f.length - 1];
    const active = document.activeElement;
    if (e.shiftKey && (active === first || active === dialog || !dialog.contains(active))) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && active === last) {
      e.preventDefault();
      first.focus();
    }
  }

  // A fixed overlay inside a transformed ancestor is fixed to THAT: hang the
  // veil on <body> (a route can sit inside one).
  function toBody(node) {
    document.body.appendChild(node);
    return {
      destroy() {
        node.parentNode?.removeChild(node);
      },
    };
  }
</script>

<svelte:window on:keydown|capture={onKey} />

<section class="styles" aria-labelledby="explorer-styles-title">
  <div class="head">
    <h2 class="section-title" id="explorer-styles-title">Parcourir par style</h2>
    {#if styles.length}
      <span class="lead">Vos styles les plus écoutés</span>
    {/if}
  </div>

  {#if styles.length}
    <div class="tiles">
      {#each styles as s (s.name)}
        <button
          type="button"
          class="tile"
          style={`--art:${grooveBackground(s.name)}`}
          aria-label={`Découvrir ${s.name} — ${countLabel(s.tracks)}`}
          title={`Découvrir des playlists ${s.name}`}
          on:click={() => chooseStyle(s)}
        >
          <span class="art" aria-hidden="true"></span>
          <span class="cap">
            <span class="name">{s.name}</span>
            <span class="n">{countLabel(s.tracks)}</span>
            <span class="bar" aria-hidden="true" style={`--w:${Math.round((s.tracks / maxTracks) * 100)}%`}></span>
          </span>
        </button>
      {/each}
    </div>
    {#if stale}
      <p class="state">
        Liste enregistrée sur cet appareil — l'actualisation a échoué.
        <button type="button" class="linkbtn" on:click={() => loadStyles(true)}>Réessayer</button>
      </p>
    {/if}
  {:else if !loaded && loading}
    <div class="tiles" aria-hidden="true">
      {#each Array(6) as _}<span class="tile sk"></span>{/each}
    </div>
    <p class="srx" role="status">Chargement de vos styles…</p>
  {:else if loadFailed}
    <div class="state">
      <p>Impossible de charger vos styles{loadCause ? ` (${loadCause})` : ""}.</p>
      <button type="button" class="btn btn-ghost" on:click={() => loadStyles(true)}>Réessayer</button>
    </div>
  {:else}
    <p class="state">
      Votre historique d'écoute est encore vide : les styles que vous écoutez le plus apparaîtront
      ici. La recherche et la bibliothèque restent ouvertes au-dessus.
    </p>
  {/if}
</section>

{#if picked}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div class="veil" use:toBody on:click={(e) => e.target === e.currentTarget && close()}>
    <div
      class="panel"
      role="dialog"
      aria-modal="true"
      aria-labelledby="discover-title"
      tabindex="-1"
      bind:this={dialog}
    >
      <header>
        <div class="ttl">
          <p class="kicker">Découverte par style</p>
          <h2 id="discover-title">{picked.name}</h2>
          <p class="sub">
            {#if picked.tracks}
              {countLabel(picked.tracks)} dans votre historique
            {:else}
              Un style de votre historique
            {/if}
            · playlists et titres vérifiés chez Deezer
          </p>
        </div>
        <button type="button" class="x" on:click={close} aria-label="Fermer">
          <Icon name="close" size={18} />
        </button>
      </header>

      <div class="aim">
        <div class="ahead">
          <label for="discover-level">Niveau de découverte</label>
          <span class="aname">{aim.name}</span>
        </div>
        <input
          id="discover-level"
          type="range"
          min="1"
          max="5"
          step="1"
          value={level}
          style={`--p:${((level - 1) / 4) * 100}%`}
          aria-label="Niveau de découverte"
          aria-valuetext={`${aim.name}, ${level} sur 5 : ${aim.hint}`}
          on:input={(e) => setLevel(+e.target.value)}
        />
        <div class="ends"><span>Prudent</span><span>Large</span></div>
        <p class="ahint">{aim.hint}</p>
      </div>

      <button type="button" class="btn btn-primary go" on:click={discover} disabled={pending} aria-busy={pending}>
        {#if pending}
          <span class="spin" aria-hidden="true"></span> Recherche…
        {:else}
          <Icon name="search" size={15} /> Découvrir
        {/if}
      </button>

      <div class="out" aria-live="polite">
        {#if pending}
          <div class="rows" aria-hidden="true">
            {#each Array(3) as _}
              <div class="row sk">
                <span class="thumb"></span>
                <span class="meta"><span class="sk-line"></span><span class="sk-line short"></span></span>
              </div>
            {/each}
          </div>
          <p class="state pending-note">
            Recherche de playlists Deezer pour {picked.name}… La vérification des titres prend
            quelques secondes.
          </p>
        {:else if phase === "error"}
          <div class="state fail" role="alert">
            <p>{errText}{errCause ? ` (${errCause})` : ""}</p>
            <button type="button" class="btn btn-ghost" on:click={discover}>Réessayer</button>
          </div>
        {:else if phase === "empty"}
          <div class="state">
            <p>Aucune playlist Deezer ne correspond à {picked.name} à ce niveau.</p>
            {#if level < 5}
              <button type="button" class="btn btn-ghost" on:click={widen}>Essayer un niveau plus large</button>
            {:else}
              <p class="muted">Essayez un autre style.</p>
            {/if}
          </div>
        {:else if phase === "ready"}
          <div class="rows">
            {#each playlists as p (p.id)}
              <div class="row">
                <span class="thumb">
                  <Cover src={p.cover} alt={p.title} size={52} kind="playlist" />
                </span>
                <span class="meta">
                  <span class="ptitle">{p.title}</span>
                  <span class="pcount">{verifiedLabel(p)}</span>
                </span>
                <button
                  type="button"
                  class="btn btn-primary play"
                  aria-label={`Lire ${p.title}`}
                  on:click={() => play(p)}
                >
                  <Icon name="play" size={13} /> Lire
                </button>
              </div>
            {/each}
          </div>
        {:else}
          <p class="state hint">
            Choisissez un niveau, puis « Découvrir » : le serveur cherche des playlists Deezer
            dans ce style et vérifie leurs titres.
          </p>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .styles {
    container-type: inline-size;
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
  }
  .head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 16px;
  }
  .head .section-title {
    margin: 0;
  }
  .lead {
    flex: none;
    font-size: 12.5px;
    color: var(--dim);
  }
  /* Two columns on the narrowest phone, five at a wide window: ten parents read
     as one shelf, never as a wall. */
  .tiles {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 12px;
  }
  .tile {
    position: relative;
    display: flex;
    align-items: flex-end;
    min-width: 0;
    height: 112px;
    padding: 0;
    border-radius: 18px;
    border: 1px solid var(--ln);
    overflow: hidden;
    background: var(--s1);
    color: var(--tx);
    text-align: left;
    content-visibility: auto;
    contain-intrinsic-size: auto 112px;
    transition: border-color 0.15s ease;
  }
  .art {
    position: absolute;
    inset: 0 0 54px 0;
    background-image: var(--art);
  }
  .cap {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 1px;
    width: 100%;
    min-width: 0;
    padding: 9px 12px 10px;
    background: var(--s1);
  }
  .name {
    display: block;
    font-family: var(--font-display);
    font-size: 19px;
    font-weight: 700;
    letter-spacing: -0.02em;
    line-height: 1.05;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .n {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
    font-size: 10.5px;
    letter-spacing: 0.02em;
    color: var(--dim);
  }
  .bar {
    display: block;
    width: 100%;
    height: 2px;
    margin-top: 5px;
    border-radius: 2px;
    background: var(--s2);
    background-image: linear-gradient(90deg, var(--ac-text) 0, var(--ac-text) var(--w), transparent var(--w));
  }
  .tile:hover {
    border-color: var(--s2);
  }
  .tile:hover .cap {
    background: var(--s2);
  }
  .tile:focus-visible {
    outline-offset: 2px;
  }
  .sk {
    display: block;
    background: var(--s2);
  }
  .state {
    margin: 0;
    color: var(--dim);
    font-size: 14px;
    line-height: 1.5;
  }
  .state p {
    margin: 0 0 10px;
  }
  .linkbtn {
    padding: 0;
    border: none;
    background: none;
    color: var(--tx);
    font-size: 13px;
    font-weight: 600;
    text-decoration: underline;
  }
  .linkbtn:hover {
    color: var(--ac-text);
  }
  .srx {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
    border: 0;
  }
  @container (min-width: 600px) {
    .tiles {
      grid-template-columns: repeat(3, minmax(0, 1fr));
    }
  }
  @container (min-width: 840px) {
    .tiles {
      grid-template-columns: repeat(4, minmax(0, 1fr));
    }
  }
  @container (min-width: 1040px) {
    .tiles {
      grid-template-columns: repeat(5, minmax(0, 1fr));
    }
  }

  /* -- the discovery panel ------------------------------------------------- */

  /* The theme's own ink at 60 %, never a hardcoded black (DESIGN §8). */
  .veil {
    position: fixed;
    inset: 0;
    z-index: 400;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 20px;
    background: color-mix(in srgb, var(--onac) 60%, transparent);
    overscroll-behavior: contain;
    touch-action: none;
  }
  .panel {
    width: min(640px, 100%);
    min-width: 0;
    max-width: 100%;
    max-height: min(760px, calc(100dvh - 40px));
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 22px 24px 20px;
    background: var(--s1);
    border: 1px solid var(--ln);
    border-radius: 24px;
    color: var(--tx);
    box-shadow: 0 24px 64px color-mix(in srgb, var(--onac) 45%, transparent);
    touch-action: auto;
  }
  .panel:focus {
    outline: none;
  }
  header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
  }
  .ttl {
    min-width: 0;
  }
  .kicker {
    margin: 0;
    font-family: var(--font-mono);
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--dim);
  }
  .ttl h2 {
    margin: 2px 0 0;
    font-family: var(--font-display);
    font-size: 28px;
    font-weight: 700;
    letter-spacing: -0.02em;
    line-height: 1.05;
    overflow-wrap: anywhere;
  }
  .sub {
    margin: 4px 0 0;
    font-size: 13px;
    line-height: 1.4;
    color: var(--dim);
  }
  .x {
    flex: none;
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    border-radius: 999px;
    background: var(--s2);
    color: var(--tx);
  }
  .x:hover {
    background: color-mix(in srgb, var(--tx) 12%, var(--s2));
  }
  .aim {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 12px 14px 10px;
    border-radius: 16px;
    background: var(--bg);
    border: 1px solid var(--ln);
  }
  .ahead {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
  }
  .ahead label {
    font-size: 0.78rem;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--dim);
    font-weight: 600;
  }
  .aname {
    font-weight: 700;
    font-size: 0.95rem;
  }
  input[type="range"] {
    -webkit-appearance: none;
    appearance: none;
    width: 100%;
    height: 40px; /* the touch target; the track itself is drawn thin */
    margin: 2px 0 0;
    background: transparent;
    cursor: pointer;
  }
  input[type="range"]::-webkit-slider-runnable-track {
    height: 6px;
    border-radius: 999px;
    background: linear-gradient(90deg, var(--btn) 0, var(--btn) var(--p), var(--s2) var(--p));
  }
  input[type="range"]::-moz-range-track {
    height: 6px;
    border-radius: 999px;
    background: var(--s2);
  }
  input[type="range"]::-moz-range-progress {
    height: 6px;
    border-radius: 999px;
    background: var(--btn);
  }
  input[type="range"]::-webkit-slider-thumb {
    -webkit-appearance: none;
    width: 20px;
    height: 20px;
    margin-top: -9px;
    border-radius: 50%;
    background: var(--tx);
    border: 2px solid var(--onbtn);
    box-shadow: 0 2px 8px color-mix(in srgb, var(--onac) 45%, transparent);
  }
  input[type="range"]::-moz-range-thumb {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: var(--tx);
    border: 2px solid var(--onbtn);
    box-shadow: 0 2px 8px color-mix(in srgb, var(--onac) 45%, transparent);
  }
  input[type="range"]:focus-visible {
    outline: none;
  }
  input[type="range"]:focus-visible::-webkit-slider-thumb {
    box-shadow: 0 0 0 4px color-mix(in srgb, var(--ac-text) 45%, transparent);
  }
  input[type="range"]:focus-visible::-moz-range-thumb {
    box-shadow: 0 0 0 4px color-mix(in srgb, var(--ac-text) 45%, transparent);
  }
  .ends {
    display: flex;
    justify-content: space-between;
    margin-top: -6px;
    font-size: 12px;
    color: var(--dim);
  }
  .ahint {
    margin: 4px 0 0;
    min-height: 1.25em;
    font-size: 12.5px;
    line-height: 1.35;
    color: var(--dim);
  }
  .go {
    width: 100%;
    height: 44px;
    flex: none;
  }
  .spin {
    width: 13px;
    height: 13px;
    border-radius: 50%;
    border: 2px solid currentColor;
    border-top-color: transparent;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .out {
    min-height: 84px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .state.fail {
    padding: 12px 14px;
    border-radius: 14px;
    border: 1px solid var(--ln);
    background: var(--bg);
  }
  .pending-note {
    margin: 2px 0 0;
    text-align: center;
    font-size: 13px;
  }
  .hint {
    margin: 0;
    font-size: 13px;
  }
  .rows {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
    min-height: 52px;
    padding: 6px;
    border-radius: 14px;
  }
  .row:not(.sk):hover {
    background: var(--s2);
  }
  .thumb {
    flex: none;
    width: 52px;
    height: 52px;
    border-radius: 8px;
    overflow: hidden;
    background: var(--s2);
  }
  .meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
  }
  .ptitle {
    font-weight: 600;
    font-size: 14.5px;
    line-height: 1.2;
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow-wrap: anywhere;
  }
  .pcount {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
    font-size: 11px;
    color: var(--dim);
  }
  .play {
    flex: none;
    height: 40px;
  }
  .row.sk {
    background: color-mix(in srgb, var(--tx) 4%, transparent);
  }
  .row.sk .thumb,
  .row.sk .sk-line {
    background: color-mix(in srgb, var(--tx) 12%, transparent);
  }
  .sk-line {
    display: block;
    height: 10px;
    width: 70%;
    border-radius: 6px;
    background: var(--s2);
  }
  .sk-line.short {
    width: 40%;
    height: 8px;
  }

  @media (pointer: coarse) {
    .x {
      width: 44px;
      height: 44px;
    }
    .play {
      min-height: 44px;
    }
    input[type="range"] {
      height: 44px;
    }
  }
  /* A phone: the sheet rises from the bottom (thumb reach), and nothing may
     scroll sideways inside it — a long playlist title wraps instead. */
  @media (max-width: 640px) {
    .tiles {
      gap: 10px;
    }
    .tile {
      height: 104px;
    }
    .name {
      font-size: 17px;
    }
    .lead {
      display: none;
    }
    .veil {
      align-items: flex-end;
      padding: 0;
    }
    .panel {
      width: 100%;
      max-height: 92dvh;
      border-radius: 24px 24px 0 0;
      border-bottom: none;
      padding: 18px 16px calc(16px + env(safe-area-inset-bottom, 0px));
      gap: 12px;
    }
    .ttl h2 {
      font-size: 24px;
    }
    .row {
      gap: 10px;
      padding: 6px 2px;
    }
    .play {
      padding: 0 14px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .tile {
      transition: none;
    }
    .spin {
      animation: none;
    }
  }
</style>
