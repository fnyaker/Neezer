<script>
  // What this person actually played, read from their own log (the rows the
  // player reports to /api/listen; neezer/listening.py). A history screen is
  // read-only: nothing here edits, deletes or reports anything.
  //
  // Pagination is a keyset cursor (the server returns `next`), not a page
  // number: a play recorded while somebody is reading page one can never push
  // an event off page two. The last page is kept in memory while the screen is
  // mounted, and "Charger plus" APPENDS to it — the rows already read never
  // move. Personal history is never written to localStorage.
  //
  // The rows are windowed: only the day headers and events on screen are
  // mounted, however many pages were loaded. Day headers are 44 px and event
  // rows 72 px, so the virtual offsets are exact without measuring the DOM.
  import { onMount, tick } from "svelte";
  import { api, failureCause, isDeezerDenied, isDeezerUnreachable } from "../lib/api.js";
  import { playThenRadio } from "../lib/actions.js";
  import { artistLine, duration as fmtDuration } from "../lib/format.js";
  import Cover from "../components/Cover.svelte";
  import Icon from "../components/Icon.svelte";
  import LoadFailed from "../components/LoadFailed.svelte";
  import Skeleton from "../components/Skeleton.svelte";

  const PAGE = 50;
  const DAY_H = 44;
  const EVENT_H = 72;
  const BUFFER = 6;

  const FILTERS = [
    { id: "", label: "Tout" },
    { id: "listen", label: "Écoutes" },
    { id: "skip", label: "Sauts" },
  ];

  let events = null; // null until the first answer
  let summary = null;
  let next = null;
  let filter = "";
  let loading = false;
  let error = null;
  let moreError = null;
  let seq = 0;

  let pageEl;
  let listEl;
  let scroller = null;
  let vStart = 0;
  let vEnd = 24;

  // -- days and rows ---------------------------------------------------------

  function pad(n) {
    return String(n).padStart(2, "0");
  }
  function dayKey(d) {
    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
  }
  // The server sends a naive local ISO time ("2026-11-13T20:31:00"); a
  // fractional part, if a row ever has one, is display-only noise.
  function parseAt(value) {
    if (!value) return null;
    const d = new Date(String(value).replace(/\.\d+/, "").replace(" ", "T"));
    return isNaN(d.getTime()) ? null : d;
  }
  function dayLabel(d, today, yesterday) {
    const key = dayKey(d);
    if (key === dayKey(today)) return "Aujourd'hui";
    if (key === dayKey(yesterday)) return "Hier";
    const opts = { weekday: "long", day: "numeric", month: "long" };
    if (d.getFullYear() !== today.getFullYear()) opts.year = "numeric";
    const text = d.toLocaleDateString("fr-FR", opts);
    return text.charAt(0).toUpperCase() + text.slice(1);
  }
  function plural(n, one, many) {
    return `${n} ${n > 1 ? many : one}`;
  }
  function groupByDay(list) {
    const today = new Date();
    const yesterday = new Date(today);
    yesterday.setDate(today.getDate() - 1);
    const out = [];
    for (const ev of list) {
      const d = parseAt(ev.at);
      if (!d) continue;
      const key = dayKey(d);
      let group = out[out.length - 1];
      if (!group || group.key !== key) {
        group = {
          key,
          date: d,
          label: dayLabel(d, today, yesterday),
          listens: 0,
          skips: 0,
          seconds: 0,
          events: [],
        };
        out.push(group);
      }
      group.events.push(ev);
      if (ev.outcome > 0) group.listens += 1;
      else if (ev.outcome < 0) group.skips += 1;
      group.seconds += Math.max(0, Number(ev.listened) || 0);
    }
    return out;
  }
  function flatten(groups) {
    const out = [];
    for (const group of groups) {
      out.push({ kind: "day", key: "d:" + group.key, group });
      for (const ev of group.events) out.push({ kind: "event", key: "e:" + ev.id, ev });
    }
    return out;
  }

  $: groups = groupByDay(events || []);
  $: rows = flatten(groups);
  // A history with nothing at all does not need a 30-day card full of zeros:
  // the empty state is the answer, right under the filters.
  $: emptyHistory =
    events !== null &&
    events.length === 0 &&
    !!summary &&
    !summary.listens &&
    !summary.skips &&
    !summary.minutes &&
    !summary.top_artists.length &&
    !summary.top_tracks.length;
  $: offsets = buildOffsets(rows);
  $: totalH = offsets.length ? offsets[offsets.length - 1] : 0;
  $: visible = rows.slice(vStart, Math.max(vStart, vEnd));
  $: vTop = offsets[vStart] || 0;
  $: if (rows) tick().then(recompute);

  function buildOffsets(list) {
    const out = [0];
    for (const row of list) out.push(out[out.length - 1] + (row.kind === "day" ? DAY_H : EVENT_H));
    return out;
  }

  // -- virtual window --------------------------------------------------------

  function findScroller(node) {
    let el = node && node.parentElement;
    while (el) {
      const oy = getComputedStyle(el).overflowY;
      if (oy === "auto" || oy === "scroll") return el;
      el = el.parentElement;
    }
    return document.scrollingElement || document.documentElement;
  }
  function viewport(el) {
    if (el === document.scrollingElement || el === document.documentElement || el === document.body)
      return { top: 0, height: window.innerHeight };
    const r = el.getBoundingClientRect();
    return { top: r.top, height: r.height };
  }
  function rowAt(pixels) {
    let lo = 0;
    let hi = rows.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (offsets[mid] <= pixels) lo = mid + 1;
      else hi = mid;
    }
    return Math.max(0, Math.min(rows.length - 1, lo - 1));
  }
  function recompute() {
    if (!rows.length) {
      vStart = 0;
      vEnd = 0;
      return;
    }
    if (!listEl) {
      vStart = 0;
      vEnd = Math.min(rows.length, 24);
      return;
    }
    if (!scroller) scroller = findScroller(listEl);
    const list = listEl.getBoundingClientRect();
    const screen = viewport(scroller);
    const above = screen.top - list.top; // list content scrolled above the viewport
    const low = Math.max(0, above - BUFFER * EVENT_H);
    const high = Math.max(0, above + screen.height + BUFFER * EVENT_H);
    const start = rowAt(low);
    let end = rowAt(high) + 1;
    // A day header just above the viewport is cheap and keeps the first
    // visible event under its date.
    const from = start > 0 && rows[start - 1].kind === "day" ? start - 1 : start;
    end = Math.max(from + 1, Math.min(end, rows.length));
    if (from !== vStart || end !== vEnd) {
      vStart = from;
      vEnd = end;
    }
  }
  let ticking = false;
  function onScroll() {
    if (ticking) return;
    ticking = true;
    requestAnimationFrame(() => {
      ticking = false;
      recompute();
    });
  }
  onMount(loadFirst);
  onMount(() => {
    scroller = findScroller(pageEl);
    scroller?.addEventListener("scroll", onScroll, { passive: true });
    window.addEventListener("resize", onScroll, { passive: true });
    let ro = null;
    if (typeof ResizeObserver !== "undefined" && scroller) {
      ro = new ResizeObserver(onScroll);
      ro.observe(scroller);
    }
    tick().then(recompute);
    return () => {
      scroller?.removeEventListener("scroll", onScroll);
      window.removeEventListener("resize", onScroll);
      ro?.disconnect();
    };
  });

  // -- reading ---------------------------------------------------------------

  async function loadFirst() {
    const mine = ++seq;
    loading = true;
    error = null;
    moreError = null;
    try {
      const data = await api.history({
        limit: PAGE,
        outcome: filter || null,
      });
      if (mine !== seq) return;
      events = data.events || [];
      summary = data.summary || summary;
      next = data.next || null;
    } catch (e) {
      if (mine === seq) {
        events = null;
        error = e;
      }
    } finally {
      if (mine === seq) loading = false;
    }
  }
  async function loadMore() {
    if (!next || loading) return;
    const mine = seq;
    loading = true;
    moreError = null;
    try {
      const data = await api.history({
        before: next.before,
        beforeId: next.before_id,
        limit: PAGE,
        outcome: filter || null,
      });
      if (mine !== seq) return;
      // A cursor is exact, but a defensive id check makes a duplicate
      // impossible to show even if a page overlaps.
      const have = new Set((events || []).map((e) => e.id));
      const fresh = (data.events || []).filter((e) => !have.has(e.id));
      events = (events || []).concat(fresh);
      next = data.next || null;
    } catch (e) {
      if (mine === seq) moreError = e;
    } finally {
      if (mine === seq) loading = false;
    }
  }
  function setFilter(value) {
    if (value === filter) return;
    filter = value;
    events = null;
    next = null;
    error = null;
    moreError = null;
    loadFirst();
  }
  function retry() {
    if (events === null) loadFirst();
    else loadMore();
  }

  $: failText = error
    ? `Historique indisponible : ${failureCause(error) || "réessayez dans un instant"}.`
    : "";
  $: genericFailure = !!error && !isDeezerUnreachable(error) && !isDeezerDenied(error);

  function when(ev) {
    const d = parseAt(ev.at);
    if (!d) return "";
    return d.toLocaleTimeString("fr-FR", { hour: "2-digit", minute: "2-digit", hour12: false });
  }
  const OUTCOME = {
    listen: { icon: "check", label: "Écouté", title: "Au moins la moitié du titre (ou 4 min)" },
    skip: { icon: "next", label: "Passé", title: "Moins de 30 secondes" },
    neutral: { icon: "minus", label: "Partiel", title: "Écoute partielle, ni comptée ni sautée" },
  };
  function verdict(ev) {
    return ev.outcome > 0 ? "listen" : ev.outcome < 0 ? "skip" : "neutral";
  }
  function play(ev) {
    if (ev && ev.track) playThenRadio(ev.track);
  }
</script>

<div class="history fade-in" bind:this={pageEl}>
  {#if summary && !emptyHistory}
    <section class="summary card" aria-labelledby="sum-title">
      <div class="sum-head">
        <h2 class="section-title" id="sum-title">{summary.days} derniers jours</h2>
        <span class="label-mono sum-note">Ton écoute</span>
      </div>
      <div class="stats">
        <div class="stat">
          <strong>{summary.listens}</strong>
          <span>écoute{summary.listens > 1 ? "s" : ""}</span>
        </div>
        <div class="stat">
          <strong>{summary.skips}</strong>
          <span>saut{summary.skips > 1 ? "s" : ""}</span>
        </div>
        <div class="stat">
          <strong>{summary.minutes}</strong>
          <span>minute{summary.minutes > 1 ? "s" : ""}</span>
        </div>
      </div>
      {#if summary.top_artists.length || summary.top_tracks.length}
        <div class="tops">
          {#if summary.top_artists.length}
            <div class="top">
              <h3 class="label-mono">Artistes les plus écoutés</h3>
              <div class="artists">
                {#each summary.top_artists as a (a.id)}
                  <a class="artist" href={`#/artist/${a.id}`} title={`Ouvrir ${a.name}`}>
                    <Cover src={a.cover} alt={a.name} kind="artist" round size={44} />
                    <span class="artist-name">{a.name}</span>
                    <span class="artist-count">{plural(a.count, "écoute", "écoutes")}</span>
                  </a>
                {/each}
              </div>
            </div>
          {/if}
          {#if summary.top_tracks.length}
            <div class="top">
              <h3 class="label-mono">Titres les plus écoutés</h3>
              <ol class="tracks">
                {#each summary.top_tracks as item (item.track.id)}
                  <li>
                    <button class="top-track" on:click={() => play({ track: item.track })} title={`Lire ${item.track.title}`}>
                      <Cover src={item.track.album?.cover} alt="" kind="track" size={36} fallbackId={item.track.id} />
                      <span class="top-track-text">
                        <span class="top-track-title">{item.track.title}</span>
                        <span class="top-track-artist muted">{artistLine(item.track)}</span>
                      </span>
                      <span class="top-track-count">{plural(item.count, "écoute", "écoutes")}</span>
                      <span class="top-track-play" aria-hidden="true"><Icon name="play" size={14} /></span>
                    </button>
                  </li>
                {/each}
              </ol>
            </div>
          {/if}
        </div>
      {/if}
    </section>
  {:else if !error && !emptyHistory}
    <div class="summary card sum-skeleton" aria-hidden="true">
      <span class="sk w30"></span>
      <span class="sk w55"></span>
      <span class="sk w40"></span>
    </div>
  {/if}

  <div class="filters" role="tablist" aria-label="Filtrer l'historique">
    {#each FILTERS as f (f.id)}
      <button
        type="button"
        role="tab"
        class="chip"
        class:chip-active={filter === f.id}
        aria-selected={filter === f.id}
        on:click={() => setFilter(f.id)}
      >{f.label}</button>
    {/each}
  </div>

  {#if error}
    <LoadFailed error={error} notFound={failText} on:retry={retry} />
    {#if genericFailure}
      <div class="retry-row">
        <button class="btn btn-ghost" on:click={retry}><Icon name="refresh" size={16} /> Réessayer</button>
      </div>
    {/if}
  {:else if events === null}
    <Skeleton kind="list" count={8} />
  {:else if !events.length}
    <div class="empty">
      <span class="empty-icon"><Icon name="timer" size={34} /></span>
      <p class="empty-title">
        {filter === "listen" ? "Aucune écoute dans l'historique" : filter === "skip" ? "Aucun saut dans l'historique" : "Aucune écoute pour l'instant"}
      </p>
      <p class="empty-text">
        {filter
          ? "Change de filtre pour revoir tout ton historique."
          : "Lance un titre : ce que tu écoutes et ce que tu passes apparaîtra ici, jour par jour."}
      </p>
      {#if filter}
        <button class="btn btn-ghost" on:click={() => setFilter("")}><Icon name="queue" size={16} /> Voir tout l'historique</button>
      {:else}
        <a class="btn btn-primary" href="#/search"><Icon name="search" size={16} /> Explorer</a>
      {/if}
    </div>
  {:else}
    <div class="vlist" bind:this={listEl} style={`height:${totalH}px`}>
      <div class="vwin" style={`transform:translateY(${vTop}px)`}>
        {#each visible as row (row.key)}
          {#if row.kind === "day"}
            <div class="day">
              <h2>{row.group.label}</h2>
              <span class="day-meta">
                {plural(row.group.listens, "écoute", "écoutes")}
                {#if row.group.skips}· {plural(row.group.skips, "saut", "sauts")}{/if}
                {#if row.group.seconds}· {Math.floor(row.group.seconds / 60)} min{/if}
              </span>
            </div>
          {:else}
            <div class="ev">
              <button class="ev-cover" on:click={() => play(row.ev)} aria-label={`Lire ${row.ev.track.title}`} title="Lire ce titre">
                <Cover src={row.ev.track.album?.cover} alt="" kind="track" size={48} fallbackId={row.ev.track.id} />
                <span class="ev-play" aria-hidden="true"><Icon name="play" size={13} /></span>
              </button>
              <div class="ev-main">
                <span class="ev-title" title={row.ev.track.title}>{row.ev.track.title}</span>
                <span class="ev-artist muted">{artistLine(row.ev.track)}</span>
                <span class="ev-meta mono">
                  {when(row.ev)} · {row.ev.listened > 0 ? `${fmtDuration(row.ev.listened)} écoutés` : "Pas écouté"}
                </span>
              </div>
              <span class={`verdict ${verdict(row.ev)}`} title={OUTCOME[verdict(row.ev)].title}>
                <Icon name={OUTCOME[verdict(row.ev)].icon} size={14} />
                <span>{OUTCOME[verdict(row.ev)].label}</span>
              </span>
            </div>
          {/if}
        {/each}
      </div>
    </div>

    <div class="more">
      {#if next}
        <button class="btn btn-ghost" on:click={loadMore} disabled={loading}>
          {#if loading}
            <span class="spin"><Icon name="refresh" size={16} /></span> Chargement…
          {:else}
            <Icon name="plus" size={16} /> Charger plus
          {/if}
        </button>
      {:else}
        <p class="muted end">Début de ton historique.</p>
      {/if}
      {#if moreError}
        <p class="load-error" role="alert">Impossible de charger la suite : {failureCause(moreError)}.</p>
        <button class="btn btn-ghost" on:click={loadMore} disabled={loading}><Icon name="refresh" size={16} /> Réessayer</button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .history {
    max-width: 1080px;
    /* The Now-Playing column takes ~340 px off the content on a wide window:
       the summary must answer to the width it actually has, not the viewport's. */
    container-type: inline-size;
  }

  /* -- summary ------------------------------------------------------------- */
  .summary {
    padding: 18px 20px 20px;
    margin-bottom: 18px;
  }
  .sum-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
  }
  .sum-head .section-title {
    margin: 0;
  }
  .sum-note {
    color: var(--dim);
    font-size: 11px;
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 10px;
    margin: 16px 0 4px;
  }
  .stat {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 12px 14px;
    border-radius: var(--r-card);
    background: var(--s2);
  }
  .stat strong {
    font-family: var(--font-display);
    font-size: 30px;
    line-height: 1;
    letter-spacing: -0.03em;
    font-weight: 700;
  }
  .stat span {
    color: var(--dim);
    font-size: 12px;
    font-weight: 600;
  }
  .tops {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 22px;
    margin-top: 18px;
  }
  .tops h3 {
    margin: 0 0 10px;
    color: var(--dim);
    font-size: 11px;
  }
  .top {
    /* A grid item's default min-width is its content: the artist rail's
       min-content width pushed the whole summary wider than a phone. */
    min-width: 0;
  }
  .artists {
    display: flex;
    gap: 8px;
    overflow-x: auto;
    padding: 2px 2px 4px;
    scrollbar-width: none;
  }
  .artists::-webkit-scrollbar {
    display: none;
  }
  .artist {
    flex: 0 0 96px;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 5px;
    padding: 8px 6px;
    border-radius: var(--r-card);
    text-align: center;
    color: var(--tx);
    text-decoration: none;
  }
  .artist:hover {
    background: var(--s2);
  }
  .artist-name {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 12.5px;
    font-weight: 600;
  }
  .artist-count {
    color: var(--dim);
    font-family: var(--font-mono);
    font-size: 10.5px;
  }
  .tracks {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .top-track {
    width: 100%;
    min-height: 46px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 5px 6px;
    border: 0;
    border-radius: 10px;
    background: none;
    color: var(--tx);
    text-align: left;
  }
  .top-track:hover {
    background: var(--s2);
  }
  .top-track-text {
    min-width: 0;
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .top-track-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13.5px;
    font-weight: 600;
  }
  .top-track-artist {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 12px;
  }
  .top-track-count {
    flex: none;
    color: var(--dim);
    font-family: var(--font-mono);
    font-size: 10.5px;
  }
  .top-track-play {
    flex: none;
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    background: var(--s2);
    color: var(--tx);
  }

  /* -- filters ------------------------------------------------------------- */
  .filters {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0 0 12px;
  }

  /* -- read failure -------------------------------------------------------- */
  .retry-row {
    display: flex;
    justify-content: center;
    margin-top: 14px;
  }
  .load-error {
    margin: 8px 0 4px;
    color: var(--dim);
    font-size: 13px;
  }
  .more {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 22px 0 10px;
  }
  .more .end {
    margin: 0;
    font-size: 12.5px;
    color: var(--dim);
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .spin {
    display: grid;
    animation: spin 1s linear infinite;
  }

  /* -- empty --------------------------------------------------------------- */
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 48px 20px;
    text-align: center;
  }
  .empty-icon {
    display: grid;
    place-items: center;
    width: 68px;
    height: 68px;
    margin-bottom: 4px;
    border-radius: 50%;
    background: var(--s2);
    color: var(--dim);
  }
  .empty-title {
    margin: 0;
    font-family: var(--font-display);
    font-size: 21px;
    font-weight: 600;
  }
  .empty-text {
    max-width: 380px;
    margin: 0 0 10px;
    color: var(--dim);
    font-size: 14px;
    line-height: 1.5;
  }

  /* -- windowed rows ------------------------------------------------------- */
  .vlist {
    position: relative;
  }
  .vwin {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
  }
  .day {
    height: 44px;
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 12px;
    padding: 0 2px 6px;
    border-bottom: 1px solid var(--ln);
  }
  .day h2 {
    margin: 0;
    font-family: var(--font-body);
    font-size: 13px;
    font-weight: 700;
    letter-spacing: 0.01em;
  }
  .day-meta {
    color: var(--dim);
    font-family: var(--font-mono);
    font-size: 10.5px;
    white-space: nowrap;
  }
  .ev {
    height: 72px;
    display: grid;
    grid-template-columns: 48px minmax(0, 1fr) auto;
    align-items: center;
    gap: 12px;
    padding: 0 2px;
    border-bottom: 1px solid var(--ln);
  }
  .ev-cover {
    position: relative;
    width: 48px;
    height: 48px;
    padding: 0;
    border: 0;
    border-radius: 10px;
    overflow: hidden;
    background: var(--s2);
    color: var(--tx);
  }
  .ev-play {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.42);
    color: #fff;
    opacity: 0;
    transition: opacity 0.15s ease;
  }
  .ev-cover:hover .ev-play,
  .ev-cover:focus-visible .ev-play {
    opacity: 1;
  }
  .ev-main {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .ev-title,
  .ev-artist,
  .ev-meta {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ev-title {
    color: var(--tx);
    font-size: 14px;
    font-weight: 600;
  }
  .ev-artist {
    font-size: 12.5px;
  }
  .ev-meta {
    font-size: 10.5px;
    color: var(--dim);
  }
  .verdict {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 24px;
    padding: 0 9px;
    border-radius: 999px;
    background: var(--s2);
    color: var(--dim);
    font-size: 11px;
    font-weight: 700;
    white-space: nowrap;
  }
  .verdict.listen {
    color: var(--tx);
  }
  .verdict.skip {
    color: var(--tx);
  }
  .verdict.neutral {
    color: var(--dim);
  }

  /* -- phone --------------------------------------------------------------- */
  @container (min-width: 980px) {
    /* Five titles over two columns: the summary is a header, not the screen.
       At this width the row keeps its title readable and the count; the play
       target stays the whole row (the circle was only a hint). */
    .tracks {
      display: grid;
      grid-template-columns: repeat(2, minmax(0, 1fr));
      column-gap: 6px;
    }
    .top-track-play {
      display: none;
    }
  }
  @container (max-width: 720px) {
    .tops {
      grid-template-columns: 1fr;
      gap: 18px;
    }
  }
  @media (max-width: 900px) {
    .tops {
      grid-template-columns: 1fr;
      gap: 18px;
    }
  }
  @media (max-width: 640px) {
    .summary {
      padding: 16px;
    }
    .stats {
      gap: 6px;
      margin-top: 12px;
    }
    .stat {
      padding: 10px 11px;
    }
    .stat strong {
      font-size: 25px;
    }
    .stat span {
      font-size: 11px;
    }
    .tops {
      margin-top: 14px;
    }
    .artist {
      flex-basis: 92px;
    }
    .top-track-count {
      display: none;
    }
    .ev {
      grid-template-columns: 44px minmax(0, 1fr) auto;
      gap: 10px;
    }
    .ev-cover {
      width: 44px;
      height: 44px;
    }
    .ev-title {
      font-size: 13.5px;
    }
    .verdict span {
      /* A phone keeps the glyph and the word: it is the difference between
         "listened" and "skipped", never carried by colour alone. */
      display: inline;
    }
    .day-meta {
      font-size: 10px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .ev-play {
      transition: none;
    }
    .spin {
      animation: none;
    }
  }
</style>
