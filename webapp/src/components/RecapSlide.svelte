<script>
  // One card of the Récap story. The card's ID is NOT a prop: it is deduced
  // from `cardIds(payload)[index]`, so this component and RecapStory can never
  // disagree about the order. Every number, every title and every kicker comes
  // from lib/recap.js — this file chooses a LAYOUT, never a format.
  import { createEventDispatcher } from "svelte";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";
  import { artistLine } from "../lib/format.js";
  import {
    cardIds,
    slideTitle,
    slideKicker,
    formatNumber,
    formatMinutes,
    formatSpan,
    monthBars,
    busiestMonth,
    rankRows,
    genreRows,
    mosaicCovers,
    periodLabel,
    posterLines,
  } from "../lib/recap.js";

  export let payload;
  export let index = 0;

  const dispatch = createEventDispatcher();

  const ICONS = {
    year: "sparkles",
    minutes: "timer",
    artists: "user",
    tracks: "music",
    albums: "disc",
    genres: "tag",
    streak: "activity",
    firstlast: "music",
    share: "share",
  };

  $: ids = cardIds(payload);
  $: total = ids.length;
  $: id = ids[index] || null;
  $: title = id ? slideTitle(payload, id) : "";
  $: kicker = id ? slideKicker(payload, id) : "";
  $: icon = ICONS[id] || "sparkles";
  $: totals = (payload && payload.totals) || {};
  $: bars = monthBars(payload);
  $: busiest = busiestMonth(payload);
  $: artists = rankRows(payload, "artists");
  $: tracks = rankRows(payload, "tracks").filter((row) => row.track);
  $: albums = rankRows(payload, "albums");
  $: genres = genreRows(payload);
  $: genreInfo = (payload && payload.tops && payload.tops.genres) || null;
  $: covers = mosaicCovers(payload);
  $: streak = (payload && payload.streak) || null;
  $: biggest = (payload && payload.biggest_day) || null;
  $: first = (payload && payload.first) || null;
  $: last = (payload && payload.last) || null;
  $: poster = posterLines(payload);

  function count(n, one, many) {
    const value = Math.max(0, Math.trunc(Number(n) || 0));
    return `${formatNumber(value)} ${value > 1 ? many : one}`;
  }
  function percent(part) {
    const value = Number(part);
    if (!Number.isFinite(value)) return 0;
    return Math.round(Math.max(0, Math.min(1, value)) * 100);
  }

  // "le 2 janvier", "du 1ᵉʳ au 23 mars": the month NAMES are localised by the
  // browser's French locale rather than copied into this file. The day/month
  // shapes live here because the pure module formats the YEAR's period, not
  // these two card-specific lines.
  const monthFormat =
    typeof Intl !== "undefined" ? new Intl.DateTimeFormat("fr-FR", { month: "long" }) : null;
  function dayParts(iso) {
    const match = /^(\d{4})-(\d{2})-(\d{2})/.exec(String(iso == null ? "" : iso));
    if (!match) return null;
    const y = Number(match[1]);
    const mo = Number(match[2]);
    const d = Number(match[3]);
    if (mo < 1 || mo > 12 || d < 1) return null;
    return { y, mo, d };
  }
  function monthName(mo) {
    try {
      return monthFormat ? monthFormat.format(new Date(2020, mo - 1, 15)) : "";
    } catch {
      return "";
    }
  }
  function dayMonth(iso) {
    const day = dayParts(iso);
    if (!day) return "";
    return `${day.d === 1 ? "1ᵉʳ" : day.d} ${monthName(day.mo)}`.trim();
  }
  function streakRange(from, to) {
    const a = dayParts(from);
    const b = dayParts(to);
    if (!a || !b) return "";
    if (a.y === b.y && a.mo === b.mo)
      return `du ${a.d === 1 ? "1ᵉʳ" : a.d} au ${b.d} ${monthName(b.mo)}`.trim();
    return `du ${dayMonth(from)} au ${dayMonth(to)}`;
  }
</script>

{#if id}
  <div
    class="slide"
    role="group"
    aria-roledescription="carte"
    aria-label={`${index + 1} sur ${total}`}
  >
    <header class="head">
      <span class="kicker mono">{kicker}</span>
      <span class="glyph" aria-hidden="true"><Icon name={icon} size={17} /></span>
    </header>

    <h2 class="title">{title}</h2>

    <div class="body">
      {#if id === "year"}
        <p class="lede">{count(totals.listens, "écoute", "écoutes")} · {formatMinutes(totals.minutes)}</p>
        {#if covers.length}
          <div class="mosaic" class:one={covers.length === 1} class:two={covers.length === 2}>
            {#each covers as url (url)}
              <span class="tile"><Cover src={url} alt="" kind="album" eager /></span>
            {/each}
          </div>
        {/if}
        <p class="period">{periodLabel(payload)}</p>
      {:else if id === "minutes"}
        <p class="lede">soit {formatSpan(totals.seconds)} de musique</p>
        <div class="chart" aria-hidden="true">
          {#each bars as bar (bar.month)}
            <span class="bar" style={`height:${bar.pct ? Math.max(4, bar.pct) : 2}%`}></span>
          {/each}
        </div>
        <div class="chart-foot">
          <span class="label-mono">écoutes par mois</span>
          {#if busiest}
            <span class="strong">Ton mois le plus fort : {busiest.label}</span>
          {/if}
        </div>
      {:else if id === "artists"}
        <ol class="ranks">
          {#each artists as row, i (row.id || row.name || i)}
            <li>
              <span class="rank mono">{i + 1}</span>
              <span class="thumb round"><Cover src={row.cover} alt="" kind="artist" round size={44} /></span>
              <span class="rowtext">
                <span class="rowname">{row.name}</span>
                <span class="rowsub">{count(row.listens, "écoute", "écoutes")}</span>
              </span>
            </li>
          {/each}
        </ol>
      {:else if id === "tracks"}
        <ol class="ranks">
          {#each tracks as row, i (row.track.id || i)}
            <li>
              <span class="rank mono">{i + 1}</span>
              <span class="thumb"><Cover src={row.track.album && row.track.album.cover} alt="" kind="track" size={40} fallbackId={row.track.id} /></span>
              <span class="rowtext">
                <span class="rowname">{row.track.title}</span>
                <span class="rowsub">{artistLine(row.track)}</span>
              </span>
              <span class="rowcount mono">{count(row.listens, "écoute", "écoutes")}</span>
            </li>
          {/each}
        </ol>
        <button class="btn btn-primary action" on:click|stopPropagation={() => dispatch("play")}>
          <Icon name="play" size={15} /> Lire ces 5 titres
        </button>
      {:else if id === "albums"}
        <ol class="ranks">
          {#each albums as row, i (row.id || row.title || i)}
            <li>
              <span class="rank mono">{i + 1}</span>
              <span class="thumb"><Cover src={row.cover} alt="" kind="album" size={40} /></span>
              <span class="rowtext">
                <span class="rowname">{row.title}</span>
                <span class="rowsub">{row.artist}</span>
              </span>
              <span class="rowcount mono">{count(row.listens, "écoute", "écoutes")}</span>
            </li>
          {/each}
        </ol>
      {:else if id === "genres"}
        <p class="lede">sur {percent(genreInfo && genreInfo.coverage)} % de tes écoutes analysées</p>
        <p class="muted explored">
          {count(genreInfo && genreInfo.explored, "style exploré", "styles explorés")}
        </p>
        <ol class="ranks genres">
          {#each genres as row, i (row.name || i)}
            <li>
              <span class="rank mono">{i + 1}</span>
              <span class="rowtext">
                <span class="rowname">{row.name}</span>
                <span class="sharebar" aria-hidden="true">
                  <span style={`width:${percent(row.share)}%`}></span>
                </span>
              </span>
              <span class="rowcount mono">{percent(row.share)} %</span>
            </li>
          {/each}
        </ol>
      {:else if id === "streak"}
        <div class="streak">
          {#if streak && streak.from && streak.to}
            <p class="dates">{streakRange(streak.from, streak.to)}</p>
          {/if}
          {#if biggest && biggest.date}
            <p class="big-day">
              Ta plus grosse journée : le {dayMonth(biggest.date)},
              <span class="nowrap">{count(biggest.listens, "écoute", "écoutes")}</span>
            </p>
          {/if}
        </div>
      {:else if id === "firstlast"}
        {#if first && first.track}
          <div class="fl">
            <span class="thumb big"><Cover src={first.track.album && first.track.album.cover} alt="" kind="track" size={52} fallbackId={first.track.id} /></span>
            <span class="rowtext">
              <span class="rowname">{first.track.title}</span>
              <span class="rowsub">{artistLine(first.track)}</span>
              <span class="date mono">le {dayMonth(first.at)}</span>
            </span>
          </div>
        {/if}
        {#if last && last.track}
          <div class="fl">
            <span class="thumb big"><Cover src={last.track.album && last.track.album.cover} alt="" kind="track" size={52} fallbackId={last.track.id} /></span>
            <span class="rowtext">
              <span class="fl-label label-mono">Ton dernier titre</span>
              <span class="rowname">{last.track.title}</span>
              <span class="rowsub">{artistLine(last.track)}</span>
              <span class="date mono">le {dayMonth(last.at)}</span>
            </span>
          </div>
        {/if}
      {:else if id === "share"}
        <div class="share-body">
          <div
            class="share-preview"
            role="img"
            aria-label={`Aperçu de l'affiche : ${poster.join(" · ")}`}
          >
            <span class="sp-kicker mono">Récap</span>
            {#each poster as line, i (i)}
              <span class="sp-line" class:big={i === 1}>{line}</span>
            {/each}
            <span class="sp-mark">neezer</span>
          </div>
          <div class="share-actions">
            <button class="btn btn-primary action" on:click|stopPropagation={() => dispatch("share")}>
              <Icon name="share" size={15} /> Partager l'image
            </button>
            <button class="btn btn-ghost action" on:click|stopPropagation={() => dispatch("restart")}>
              Revoir mon récap
            </button>
            <button class="btn btn-ghost action" on:click|stopPropagation={() => dispatch("changeyear")}>
              Changer d'année
            </button>
          </div>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .slide {
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 18px 20px 20px;
    /* A story is tapped, not selected: the whole surface belongs to the two
       tap zones underneath (RecapStory), and only real controls take a click
       back. */
    pointer-events: none;
  }
  /* Inside a pointer-events:none card, a real control is still a real target. */
  .slide button {
    pointer-events: auto;
  }
  .head {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    min-height: 22px;
  }
  .kicker {
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--dim);
  }
  .glyph {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--ac) 18%, transparent);
    color: var(--ac-text);
  }
  .title {
    flex: none;
    margin: 0;
    font-family: var(--font-display);
    /* The deck is a size container: the card's scale follows the card's width,
       not the window's (a 345 px deck in a 641 px window must not get a 36 px
       title). */
    font-size: clamp(24px, 7.4cqw, 38px);
    font-weight: 800;
    letter-spacing: -0.03em;
    line-height: 1.08;
    text-wrap: balance;
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 12px;
    /* Top-aligned: a story's title then its content, with the air below rather
       than a hole between a title pinned at the top and content floating in the
       middle of a tall card. */
    justify-content: flex-start;
  }
  .lede {
    margin: 0;
    font-family: var(--font-display);
    font-size: clamp(17px, 5.2cqw, 24px);
    font-weight: 600;
    letter-spacing: -0.015em;
    line-height: 1.25;
  }
  .period {
    margin: 0;
    color: var(--dim);
    font-size: 13.5px;
    line-height: 1.4;
  }
  .strong {
    font-weight: 600;
    color: var(--tx);
  }
  .muted {
    color: var(--dim);
  }

  /* -- the opening mosaic ------------------------------------------------ */
  .mosaic {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 8px;
  }
  /* A tall deck gives the four sleeves a 2×2 block instead of a thin strip —
     the deck is a size container (routes/Recap.svelte), so this follows the
     space the card actually has, not the window's. */
  @container deck (min-height: 580px) {
    .mosaic:not(.one):not(.two) {
      grid-template-columns: repeat(2, minmax(0, 1fr));
      max-width: 76%;
    }
  }
  @container deck (min-height: 700px) {
    .mosaic:not(.one):not(.two) {
      max-width: 86%;
    }
  }
  /* A phone's deck is shorter (the two fixed bars and the title take their
     height), but 518 px still gives the four sleeves a 2×2 block. */
  @media (max-width: 640px) {
    @container deck (min-height: 500px) {
      .mosaic:not(.one):not(.two) {
        grid-template-columns: repeat(2, minmax(0, 1fr));
        max-width: 76%;
      }
    }
  }
  .mosaic.two {
    grid-template-columns: repeat(2, minmax(0, 1fr));
    max-width: 66%;
  }
  .mosaic.one {
    grid-template-columns: 1fr;
    max-width: 33%;
  }
  .tile {
    display: block;
    overflow: hidden;
    border-radius: 14px;
    border: 1px solid var(--ln);
    aspect-ratio: 1;
    background: var(--s2);
  }

  /* -- minutes ------------------------------------------------------------ */
  .chart {
    flex: 1;
    min-height: 64px;
    max-height: 150px;
    display: flex;
    align-items: flex-end;
    gap: 4px;
    padding: 8px 0 0;
    border-bottom: 1px solid var(--ln);
  }
  .bar {
    flex: 1;
    min-width: 0;
    border-radius: 4px 4px 0 0;
    background: var(--ac);
    opacity: 0.9;
  }
  .bar:nth-child(2n) {
    opacity: 0.7;
  }
  .chart-foot {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 10px;
    flex-wrap: wrap;
    font-size: 12.5px;
    line-height: 1.4;
  }
  .chart-foot .label-mono {
    font-size: 10.5px;
  }

  /* -- tops --------------------------------------------------------------- */
  .ranks {
    flex: 1;
    min-height: 0;
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    justify-content: flex-start;
  }
  .ranks li {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 46px;
    padding: 4px 0;
    border-bottom: 1px solid var(--ln);
  }
  .ranks li:last-child {
    border-bottom: 0;
  }
  .rank {
    flex: none;
    width: 16px;
    color: var(--dim);
    font-size: 11px;
    text-align: right;
  }
  .thumb {
    flex: none;
    display: block;
    width: 40px;
    height: 40px;
    overflow: hidden;
    border-radius: 9px;
    background: var(--s2);
  }
  .thumb.round {
    width: 44px;
    height: 44px;
    border-radius: 50%;
  }
  .thumb.big {
    width: 52px;
    height: 52px;
  }
  .rowtext {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .rowname,
  .rowsub,
  .rowcount {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rowname {
    font-size: 14px;
    font-weight: 600;
    line-height: 1.25;
  }
  .rowsub {
    font-size: 12px;
    color: var(--dim);
    line-height: 1.25;
  }
  .rowcount {
    flex: none;
    max-width: 92px;
    color: var(--dim);
    font-size: 10.5px;
    text-align: right;
  }
  .action {
    flex: none;
    align-self: flex-start;
    min-height: 44px;
  }
  .action + .action {
    margin-top: 0;
  }

  /* -- styles ------------------------------------------------------------- */
  .explored {
    margin: -6px 0 0;
    font-size: 12.5px;
  }
  .genres .rowtext {
    gap: 6px;
  }
  .sharebar {
    display: block;
    width: 100%;
    height: 4px;
    border-radius: 999px;
    background: var(--s2);
    overflow: hidden;
  }
  .sharebar > span {
    display: block;
    height: 100%;
    border-radius: inherit;
    background: var(--ac);
  }
  .genres .rowcount {
    max-width: 48px;
  }

  /* -- streak ------------------------------------------------------------- */
  .streak {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .dates {
    margin: 0;
    font-family: var(--font-display);
    font-size: clamp(20px, 6cqw, 28px);
    font-weight: 600;
    letter-spacing: -0.02em;
  }
  .big-day {
    margin: 0;
    max-width: 26ch;
    font-size: 14.5px;
    line-height: 1.5;
    color: var(--dim);
  }
  .big-day .nowrap {
    color: var(--tx);
    font-weight: 600;
    white-space: nowrap;
  }

  /* -- first / last ------------------------------------------------------- */
  .fl {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 0;
    border-bottom: 1px solid var(--ln);
  }
  .fl:last-child {
    border-bottom: 0;
  }
  .fl-label {
    color: var(--dim);
    font-size: 10.5px;
  }
  .fl .date {
    color: var(--dim);
    font-size: 10.5px;
  }

  /* -- share -------------------------------------------------------------- */
  .share-body {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(0, 124px) minmax(0, 1fr);
    align-items: center;
    gap: 14px;
  }
  @container deck (min-height: 560px) {
    .share-body {
      grid-template-columns: minmax(0, 168px) minmax(0, 1fr);
      gap: 18px;
    }
  }
  .share-preview {
    aspect-ratio: 4 / 5;
    max-height: 100%;
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding: 12px 11px;
    border-radius: 12px;
    overflow: hidden;
    background:
      radial-gradient(90% 60% at 90% 8%, color-mix(in srgb, var(--ac) 38%, transparent), transparent 70%),
      #0c0c0b;
    color: #f7f4ec;
    box-shadow: 0 12px 30px rgba(0, 0, 0, 0.32);
  }
  .sp-kicker {
    color: rgba(247, 244, 236, 0.6);
    font-size: 6.5px;
    letter-spacing: 0.12em;
    text-transform: uppercase;
  }
  .sp-line {
    font-size: 7.5px;
    line-height: 1.25;
    color: rgba(247, 244, 236, 0.78);
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
  }
  .sp-line.big {
    margin: 2px 0;
    font-family: var(--font-display);
    font-size: 13px;
    font-weight: 800;
    line-height: 1.1;
    color: #d2ff3c;
  }
  .sp-mark {
    margin-top: auto;
    font-family: var(--font-display);
    font-size: 9px;
    font-weight: 700;
    letter-spacing: -0.02em;
  }
  .share-actions {
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 8px;
  }
  .share-actions .action {
    align-self: stretch;
    justify-content: flex-start;
    font-size: 12.5px;
    padding: 0 12px;
    white-space: normal;
    text-align: left;
    line-height: 1.15;
  }

  @media (max-width: 640px) {
    .slide {
      padding: 14px 16px 16px;
      gap: 8px;
    }
    .tile {
      border-radius: 10px;
    }
    .ranks li {
      min-height: 42px;
      gap: 8px;
    }
    .thumb {
      width: 36px;
      height: 36px;
    }
    .thumb.round {
      width: 40px;
      height: 40px;
    }
    .rowcount {
      max-width: 74px;
      font-size: 10px;
    }
    .chart {
      max-height: 110px;
    }
    .share-body {
      grid-template-columns: minmax(0, 108px) minmax(0, 1fr);
      gap: 10px;
    }
    .share-actions .action {
      font-size: 12px;
      padding: 0 10px;
    }
  }
  @media (max-width: 380px) {
    .share-body {
      grid-template-columns: minmax(0, 92px) minmax(0, 1fr);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .action {
      transition: none;
    }
  }
</style>
