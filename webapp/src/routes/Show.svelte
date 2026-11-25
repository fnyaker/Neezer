<script>
  import { trackKey } from "../lib/trackkey.js";
  import { push } from "svelte-spa-router";
  import { REMOTE, CAN_KEEP, CAN_ADMIN, CAN_BROWSE } from "../lib/remote/mode.js";
  import { api } from "../lib/api.js";
  import { online } from "../lib/net.js";
  import { get } from "svelte/store";
  import { player, currentId, playing, isAdmin, toasts, seekTo, openExport, openTranscriptExport, immersiveOpen } from "../lib/stores.js";
  import { podcastProgress, setResumePoint } from "../lib/podcastProgress.js";
  import { episodeMarkers, loadShowMarkers, removeMarker } from "../lib/markers.js";
  import { chaptersByEpisode, loadChapters, chapterAt, chapterCountLabel } from "../lib/chapters.js";
  import { buildTrackMenu } from "../lib/actions.js";
  import { openMenuAt, keepNativeMenu, menuKey } from "../lib/trackmenu.js";
  import { downloadTrack } from "../lib/offline.js";
  import { isMobile } from "../lib/viewport.js";
  import { duration as fmtDuration } from "../lib/format.js";
  import { onMount, onDestroy, tick } from "svelte";
  import { animate, fadeIn, DUR, EASE_CSS } from "../lib/motion.js";
  import {
    batchOptions,
    isActiveStatus,
    loadTranscribeSettings,
    reasonText,
    requestEpisodePanel,
    transcribeSettings,
    transcriptNote,
  } from "../lib/transcript.js";
  import Cover from "../components/Cover.svelte";
  import GradientHeader from "../components/GradientHeader.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import Icon from "../components/Icon.svelte";
  import LoadFailed from "../components/LoadFailed.svelte";
  import ChapterList from "../components/ChapterList.svelte";
  import ChapterSheet from "../components/ChapterSheet.svelte";
  import TranscriptHits from "../components/TranscriptHits.svelte";

  export let params = {};

  let id = null;
  let data = null;
  let loading = true;
  let loadError = null;
  let expanded = new Set();
  let chapterOpen = new Set();
  let chapterPos = {};
  let chapterCap = {};
  let sheet = null;
  let backlogMode = "all";
  let batchCount = 5;
  let batchBusy = false;
  let transcribeBusy = {};
  let searching = false;
  let searchQ = "";
  let searchTimer = null;
  let searchSeq = 0;
  let searchHits = [];
  let searchTotal = 0;
  let searchScope = null;
  let searchFor = "";
  let searchAttempt = "";
  let searchLoading = false;
  let searchError = null;
  let searchRetryLeft = 0;
  let searchRetryTimer = null;
  let seenNote = 0;
  let nextEpisodes = null;
  let loadingEpisodes = false;

  $: if (params.id && params.id !== id) {
    id = params.id;
    load(id);
  }

  onMount(() => {
    if ($isAdmin && CAN_ADMIN) loadTranscribeSettings();
  });

  let loadSeq = 0;
  async function load(pid) {
    const mine = ++loadSeq;
    archiving = false; // another show: its own count, not this one's
    clearTimeout(pollTimer);
    pollTimer = null;
    backlogMode = "all";
    batchBusy = false;
    transcribeBusy = {};
    searchQ = "";
    resetSearchState();
    loading = true;
    data = null;
    loadError = null;
    chapterOpen = new Set();
    chapterPos = {};
    chapterCap = {};
    sheet = null;
    loadShowMarkers(pid); // fail-soft, fills the markers store as it lands
    try {
      const r = await api.podcast(pid);
      if (mine === loadSeq) {
        data = r;
        nextEpisodes = r.next || null;
        if (hasActiveTranscript(r)) watchTranscript();
      }
    } catch (e) {
      if (mine === loadSeq) {
        data = null;
        loadError = e;
      }
    }
    if (mine === loadSeq) loading = false;
  }

  async function loadMoreEpisodes() {
    if (!nextEpisodes || loadingEpisodes) return;
    loadingEpisodes = true;
    try {
      const r = await api.podcast(id, { cursor: nextEpisodes, limit: 50 });
      data = {
        ...data,
        ...r,
        episodes: [...(data?.episodes || []), ...(r.episodes || [])],
      };
      nextEpisodes = r.next || null;
    } catch {
      toasts.push("Les épisodes suivants n'ont pas pu être chargés.", "error");
    } finally {
      loadingEpisodes = false;
    }
  }

  function hasActiveTranscript(show) {
    return (show?.episodes || []).some(
      (ep) => ep.transcript && isActiveStatus(ep.transcript.status),
    );
  }

  function setEpisodeTranscript(episodeId, transcript) {
    if (!data) return;
    data = {
      ...data,
      episodes: (data.episodes || []).map((ep) =>
        String(ep.id) === String(episodeId) ? { ...ep, transcript: transcript || null } : ep,
      ),
    };
  }

  function transcribeError(error) {
    const code = (error && error.body && error.body.code) || (error && error.code) || "";
    const why = error && error.body && error.body.reason;
    if (why) return reasonText(why);
    if (code === "too_long") return "L'épisode dépasse la durée maximale de transcription.";
    if (code === "not_archived") return "L'épisode doit d'abord être archivé sur le serveur.";
    if (code === "no_duration") return "La durée de l'épisode est inconnue ; impossible de la borner.";
    if (code === "queue_full") return "La file de transcription est pleine.";
    if (code === "already_running") return "Cet épisode est déjà en cours de transcription.";
    if (code === "no_worker") {
      return "Aucun ordinateur n'est enrôlé pour transcrire, et le serveur n'a pas de moteur.";
    }
    return (error && error.body && error.body.error) || (error && error.message) || "La transcription n'a pas pu démarrer.";
  }

  function openSettings() {
    push("/settings/server");
  }

  async function toggleShowTranscribe() {
    if (!data || batchBusy || !transcribe) return;
    const on = !data.auto_transcribe;
    const before = data;
    data = { ...data, auto_transcribe: on };
    batchBusy = true;
    try {
      const payload = on
        ? { auto: true, backlog: backlogMode, language: data.transcribe_language || "auto" }
        : { auto: false };
      const r = await api.setShowTranscribe(data.id, payload);
      if (data?.id === r.id) {
        data = {
          ...data,
          auto_transcribe: r.auto_transcribe,
          transcribe_language: r.transcribe_language,
        };
      }
      if (r.backlog) backlogMode = r.backlog;
      if (on) {
        const skipped = r.skipped && typeof r.skipped === "object"
          ? Object.values(r.skipped).reduce((a, b) => a + (Number(b) || 0), 0)
          : Number(r.skipped) || 0;
        if (r.queued) toasts.push(`${r.queued} épisode${r.queued > 1 ? "s" : ""} en file de transcription.`);
        else if (skipped) toasts.push(`${skipped} épisode${skipped > 1 ? "s" : ""} ignoré${skipped > 1 ? "s" : ""} (trop longs ou hors budget).`, "error");
        else toasts.push("Aucun épisode à transcrire pour le moment.");
        watchTranscript();
      }
    } catch (error) {
      data = before;
      toasts.push(transcribeError(error), "error");
    } finally {
      batchBusy = false;
    }
  }

  async function setShowLanguage(value) {
    const previous = data?.transcribe_language || "auto";
    data = { ...data, transcribe_language: value };
    try {
      const r = await api.setShowTranscribe(data.id, { language: value });
      if (data?.id === r.id) data = { ...data, transcribe_language: r.transcribe_language };
    } catch (error) {
      data = { ...data, transcribe_language: previous };
      toasts.push(transcribeError(error), "error");
    }
  }

  async function startOne(ep) {
    if (!transcribeAvailable || transcribeBusy[ep.id]) return;
    const previous = ep.transcript || null;
    setEpisodeTranscript(ep.id, { status: "queued", progress: 0 });
    transcribeBusy = { ...transcribeBusy, [ep.id]: true };
    try {
      await api.startEpisodeTranscript(ep.id, {});
      toasts.push("Épisode en file de transcription.");
      watchTranscript();
    } catch (error) {
      setEpisodeTranscript(ep.id, previous);
      toasts.push(transcribeError(error), "error");
    } finally {
      const next = { ...transcribeBusy };
      delete next[ep.id];
      transcribeBusy = next;
    }
  }

  async function startBatch() {
    if (!transcribeAvailable || batchBusy || !data) return;
    const wanted = eligible.slice(0, batchCount);
    if (!wanted.length) return;
    batchBusy = true;
    const previous = new Map(wanted.map((ep) => [ep.id, ep.transcript || null]));
    for (const ep of wanted) setEpisodeTranscript(ep.id, { status: "queued", progress: 0 });
    let accepted = 0;
    let skipped = 0;
    let cause = null;
    try {
      for (let index = 0; index < wanted.length; index++) {
        const ep = wanted[index];
        try {
          await api.startEpisodeTranscript(ep.id, {});
          accepted++;
        } catch (error) {
          const code = error?.body?.code || error?.code || "";
          setEpisodeTranscript(ep.id, previous.get(ep.id));
          if (code === "already_running") {
            accepted++;
            setEpisodeTranscript(ep.id, { status: "queued", progress: 0 });
          } else {
            skipped++;
            cause = cause || transcribeError(error);
            if (code === "queue_full" || error?.status === 429) {
              for (const rest of wanted.slice(index + 1)) {
                setEpisodeTranscript(rest.id, previous.get(rest.id));
              }
              break;
            }
          }
        }
      }
      if (accepted) toasts.push(`${accepted} épisode${accepted > 1 ? "s" : ""} en file de transcription.`);
      if (skipped) toasts.push(`${skipped} ignoré${skipped > 1 ? "s" : ""}${cause ? ` : ${cause}` : ""}`, "error");
      if (!accepted && !skipped) toasts.push("Aucun épisode à transcrire.");
      if (accepted) watchTranscript();
    } finally {
      batchBusy = false;
    }
  }

  async function stopTranscript(ep) {
    try {
      await api.stopEpisodeTranscript(ep.id);
      setEpisodeTranscript(ep.id, { ...(ep.transcript || {}), status: "stopped" });
      toasts.push("Transcription arrêtée.");
    } catch (error) {
      toasts.push(transcribeError(error), "error");
    }
  }

  async function openTranscript(ep, at = null) {
    const index = episodes.findIndex((item) => String(item.id) === String(ep.id));
    const start = at == null ? ($currentId === trackKey(ep) ? get(player).currentTime : ($podcastProgress[trackKey(ep)]?.t || 0)) : at;
    if ($currentId === trackKey(ep)) {
      if (at != null) {
        player.setProgress(at, ep.duration || 0);
        seekTo.set(at);
        player.play();
      }
    } else if (index >= 0) {
      playAt(index, ep, start);
    }
    requestEpisodePanel(ep.id, "transcript", start);
    immersiveOpen.set(true);
  }

  function pickTranscriptHit(hit) {
    const ep = episodes.find((item) => String(item.id) === String(hit.episode_id));
    if (!ep) return;
    const index = episodes.findIndex((item) => String(item.id) === String(ep.id));
    playAt(index, ep, hit.start);
    requestEpisodePanel(ep.id, "transcript", hit.start);
    immersiveOpen.set(true);
  }

  function resetSearchState() {
    clearTimeout(searchTimer);
    clearInterval(searchRetryTimer);
    searchRetryTimer = null;
    searchSeq++;
    searching = false;
    searchHits = [];
    searchTotal = 0;
    searchScope = null;
    searchFor = "";
    searchAttempt = "";
    searchLoading = false;
    searchError = null;
    searchRetryLeft = 0;
  }

  function clearShowSearch() {
    searchQ = "";
    resetSearchState();
  }

  function onSearchInput() {
    const term = searchQ.trim();
    if (term.length < 2) {
      resetSearchState();
      return;
    }
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => runShowSearch(term), 300);
  }

  function onSearchKey(event) {
    if (event.key === "Enter") {
      event.preventDefault();
      clearTimeout(searchTimer);
      runShowSearch(searchQ.trim(), true);
      event.currentTarget.blur();
    }
  }

  function startSearchRetry(seconds) {
    clearInterval(searchRetryTimer);
    searchRetryLeft = Math.max(1, Math.ceil(Number(seconds) || 0));
    searchRetryTimer = setInterval(() => {
      searchRetryLeft = Math.max(0, searchRetryLeft - 1);
      if (searchRetryLeft === 0) {
        clearInterval(searchRetryTimer);
        searchRetryTimer = null;
      }
    }, 1000);
  }

  async function runShowSearch(term, force = false) {
    if (!term || term.length < 2) {
      resetSearchState();
      return;
    }
    if (!force && (term === searchFor || (term === searchAttempt && searchLoading))) return;
    if (searchRetryLeft > 0) return;
    const mine = ++searchSeq;
    searchAttempt = term;
    searchLoading = true;
    searchError = null;
    searching = true;
    if (!$online) {
      searchError = { offline: true };
      searchLoading = false;
      searching = false;
      return;
    }
    try {
      const payload = await api.showTranscriptSearch(id, term, 50);
      if (mine !== searchSeq) return;
      searchHits = payload.hits || [];
      searchTotal = Number(payload.total) || searchHits.length;
      searchScope = payload.scope || null;
      searchFor = term;
    } catch (error) {
      if (mine !== searchSeq) return;
      searchHits = [];
      searchTotal = 0;
      searchScope = null;
      searchFor = term;
      searching = false;
      if (error?.status === 0 || !$online) searchError = { offline: true };
      else {
        searchError = error;
        const retry = Number(error?.retryIn || error?.body?.retry_in || 0);
        if (error?.status === 429 && retry > 0) startSearchRetry(retry);
      }
    } finally {
      if (mine === searchSeq) searchLoading = false;
    }
  }

  $: if ($transcriptNote && $transcriptNote.seq !== seenNote) {
    seenNote = $transcriptNote.seq;
    if (data && $transcriptNote.episodeId) {
      setEpisodeTranscript($transcriptNote.episodeId, $transcriptNote.transcript);
      if ($transcriptNote.transcript && isActiveStatus($transcriptNote.transcript.status)) {
        watchTranscript();
      }
    }
  }

  $: episodes = data?.episodes || [];

  $: archivedEpisodes = episodes.filter((ep) => ep.archived);
  $: readyCount = episodes.filter((ep) => ep.transcript?.status === "ready").length;
  $: eligible = episodes.filter(
    (ep) =>
      ep.archived &&
      (!ep.transcript ||
        ep.transcript.status === "none" ||
        ep.transcript.status === "error" ||
        ep.transcript.status === "stopped"),
  );
  $: choices = batchOptions(eligible.length);
  $: if (choices.length && !choices.includes(batchCount)) batchCount = choices[0];
  $: transcribe = $transcribeSettings.state === "ready" ? $transcribeSettings.data : null;
  $: transcribeAvailable = !!transcribe && transcribe.available !== false;
  $: activeTranscript = episodes.find(
    (ep) => ep.transcript && (ep.transcript.status === "running" || ep.transcript.status === "leased"),
  );
  $: queuedCount = episodes.filter((ep) => ep.transcript?.status === "queued").length;
  $: showSearch = episodes.some((ep) => ep.transcript?.status === "ready") || searching;
  $: audioSeconds = eligible.reduce((sum, ep) => sum + (Number(ep.duration) || 0), 0);
  $: audioHours = Math.round((audioSeconds / 3600) * 10) / 10;
  $: cpuHours = Math.round((audioHours / 2) * 10) / 10;

  // -- "Archiver automatiquement les nouveaux épisodes" ------------------------
  // The show's own switch (Pocket Casts' per-podcast auto-download, design S16).
  // The click answers at once; the server then queues what this show is missing
  // on its background archiver (one file at a time, behind anything played), and
  // the line under the switch counts it in: "12 sur 40 épisodes archivés". While
  // that count moves, the page re-reads the show every few seconds — and stops
  // as soon as it is complete, stalls, or the page is hidden.
  let autoBusy = false;
  async function toggleAuto() {
    if (autoBusy || !data) return;
    const on = !data.auto_archive;
    const before = data;
    data = { ...data, auto_archive: on };
    autoBusy = true;
    try {
      const r = await api.setPodcastAutoArchive(data.id, on);
      if (data?.id === r.id) data = { ...data, auto_archive: r.auto_archive, archived_count: r.archived_count };
      if (on && r.queued) {
        toasts.push(`${r.queued} épisode${r.queued > 1 ? "s" : ""} en file d'archivage`);
        watchArchive();
      }
    } catch (e) {
      data = before;
      toasts.push(e?.message || "Réglage non enregistré", "error");
    } finally {
      autoBusy = false;
    }
  }

  const POLL_MS = 6000;
  const TRANSCRIPT_POLL_MS = 15000;
  const STALL_POLLS = 5; // ~30 s without a new file: stop asking
  let pollTimer = null;
  let stalled = 0;
  let archiving = false;
  function watchArchive() {
    clearTimeout(pollTimer);
    stalled = 0;
    archiving = true;
    pollTimer = setTimeout(pollArchive, POLL_MS);
  }
  function watchTranscript() {
    if (pollTimer) return;
    pollTimer = setTimeout(pollArchive, TRANSCRIPT_POLL_MS);
  }
  function pollDelay() {
    return archiving ? POLL_MS : TRANSCRIPT_POLL_MS;
  }
  async function pollArchive() {
    pollTimer = null;
    if (!data) return;
    if (!archiving && !hasActiveTranscript(data)) return;
    if (document.visibilityState === "hidden") {
      pollTimer = setTimeout(pollArchive, pollDelay());
      return;
    }
    const pid = data.id;
    const was = data.archived_count;
    try {
      const r = await api.podcast(pid);
      if (data?.id !== pid) return;
      data = r;
      stalled = r.archived_count > was ? 0 : stalled + 1;
    } catch {
      stalled++;
    }
    const archiveDone =
      !archiving ||
      !data.auto_archive ||
      data.archived_count >= reachable(data) ||
      stalled >= STALL_POLLS;
    if (archiveDone) archiving = false;
    if (archiving || hasActiveTranscript(data)) {
      pollTimer = setTimeout(pollArchive, pollDelay());
    }
  }
  // Episodes Deezer no longer serves cannot be archived: they don't count
  // against "complete".
  function reachable(d) {
    const gone = (d?.episodes || []).filter((e) => e.unavailable && !e.archived).length;
    return Math.max(0, (d?.episode_count || 0) - gone);
  }
  onDestroy(() => {
    clearTimeout(pollTimer);
    clearTimeout(searchTimer);
    clearInterval(searchRetryTimer);
  });

  function playFrom(i) {
    if (episodes.length) player.playQueue(episodes, i, { kind: "podcast", id });
  }
  function playAll() {
    playFrom(0);
  }
  function toggle(i, ep) {
    if ($currentId === trackKey(ep)) player.toggle();
    else playFrom(i);
  }

  // Jump straight to a position inside an episode (a marker tap): the playing
  // episode just seeks; another episode records the target as its resume point
  // first, so the player's load lands exactly there.
  function playAt(i, ep, position) {
    if ($currentId === trackKey(ep)) {
      player.setProgress(position, ep.duration || 0);
      seekTo.set(position);
      player.play();
      return;
    }
    setResumePoint(trackKey(ep), position, ep.duration || 0);
    playFrom(i);
  }

  function episodeMenu(e, ep) {
    if (keepNativeMenu(e)) return;
    openMenuAt(e, buildTrackMenu(ep, push), e.currentTarget);
  }

  // Download every episode to the device, one at a time. Sequential on purpose:
  // an episode is tens of megabytes, and a show can hold hundreds of them —
  // firing them all at once would saturate the connection and the server's
  // archiver both. Already-downloaded episodes are skipped by downloadTrack.
  let dlBusy = false;
  let dlDone = 0;
  async function downloadAll() {
    if (dlBusy || !episodes.length) return;
    dlBusy = true;
    dlDone = 0;
    let failed = 0;
    try {
      for (const ep of episodes) {
        try {
          await downloadTrack(ep, null);
        } catch {
          failed++;
        }
        dlDone++;
      }
      toasts.push(
        failed
          ? `${episodes.length - failed} épisode(s) téléchargé(s), ${failed} en échec`
          : `${episodes.length} épisode(s) disponibles hors-ligne`,
        failed ? "error" : undefined
      );
    } finally {
      dlBusy = false;
    }
  }

  function toggleExpand(eid, el) {
    const n = new Set(expanded);
    const opening = !n.has(eid);
    if (opening) n.add(eid);
    else n.delete(eid);
    expanded = n;
    // I4: the clamp lifts at once (a description can be taller than M10's
    // 400 px) and the text fades back in; no height walk above a long list.
    if (opening) reveal(el);
  }

  /** A `fadeIn` on an element already in the DOM (M7, I4). */
  function reveal(el) {
    if (!el) return;
    animate(el, [{ opacity: 0 }, { opacity: 1 }], { duration: DUR.fast, easing: EASE_CSS.enter });
  }

  async function toggleChapters(ep) {
    const key = trackKey(ep);
    const next = new Set(chapterOpen);
    if (next.has(key)) {
      next.delete(key);
      chapterOpen = next;
      return;
    }
    next.add(key);
    chapterCap = { ...chapterCap, [key]: isMobile() ? 4 : 8 };
    chapterPos = { ...chapterPos, [key]: $currentId === key ? get(player).currentTime : 0 };
    chapterOpen = next;
    loadChapters(ep.id);
    // I4: the chapter list appears instantly (M10: up to 8 rows above a long
    // episode list is not a small block) and fades in; the chevron turns.
    await tick();
    reveal(document.getElementById("echapters-" + key));
  }

  function seekChapter(ep, i, start) {
    chapterPos = { ...chapterPos, [trackKey(ep)]: start };
    playAt(i, ep, start);
  }

  function openChapters(ep, i) {
    sheet = { ep, i };
  }

  async function unsubscribe() {
    if (!window.confirm("Se désabonner de ce podcast ?")) return;
    try {
      await api.unsubscribePodcast(id);
      toasts.push("Désabonné");
      push("/library/podcasts");
    } catch (e) {
      toasts.push(e?.message || "Échec", "error");
    }
  }

  // Resume state for an episode: fraction played (0..1), seconds remaining and
  // whether it was heard to the end. null when there's nothing to show.
  function resumeOf(ep) {
    const p = $podcastProgress[trackKey(ep)];
    if (!p || (!p.t && !p.done)) return null;
    if (p.done) return { done: true, pct: 1, left: 0 };
    const dur = p.d || ep.duration || 0;
    if (!dur) return { done: false, pct: 0, left: 0 };
    return {
      done: false,
      pct: Math.min(1, p.t / dur),
      left: Math.max(0, Math.round(dur - p.t)),
    };
  }

  function fmtDate(ts) {
    if (!ts) return "";
    try {
      return new Date(ts * 1000).toLocaleDateString("fr-FR", {
        day: "numeric",
        month: "short",
        year: "numeric",
      });
    } catch {
      return "";
    }
  }
</script>

{#if loading}
  <Skeleton kind="header" />
  <Skeleton kind="list" />
{:else if !data}
  <LoadFailed error={loadError} notFound="Podcast introuvable." on:retry={() => load(id)} />
{:else}
  <div in:fadeIn>
    <GradientHeader cover={data.cover}>
      <div class="art"><Cover src={data.cover} alt={data.title} kind="podcast" fallbackId={data.id} eager /></div>
      <div class="meta">
        <span class="kind">Podcast{#if data.local} · Local{/if}</span>
        <h1>{data.title}</h1>
        <div class="sub muted">
          {episodes.length} épisode{episodes.length > 1 ? "s" : ""}
          {#if data.local}
            · {data.archived_count} archivé{data.archived_count > 1 ? "s" : ""} sur le serveur
          {/if}
        </div>
        {#if data.local}
          <p class="localnote">
            Ce podcast n'existe plus sur Deezer. Tout ce qui a été archivé reste
            ici, jouable et téléchargeable comme avant — c'est désormais un
            podcast local.
          </p>
        {/if}
        {#if data.description}<p class="desc">{data.description}</p>{/if}
      </div>
    </GradientHeader>

    <div class="row actions">
      <button class="pill" on:click={playAll} disabled={!episodes.length}>
        <Icon name="play" size={18} /> Lire
      </button>
      <button class="ghost-btn" on:click={downloadAll} disabled={!episodes.length || dlBusy}
              title="Télécharger tous les épisodes sur cet appareil (écoute hors-ligne)">
        <Icon name={dlBusy ? "downloaded" : "download"} size={17} />
        {dlBusy ? `${dlDone}/${episodes.length}…` : "Tout télécharger"}
      </button>
      {#if CAN_KEEP}
        <button class="ghost-btn" on:click={() => openExport("podcast", id, data.title)}
                disabled={!episodes.length} title="Exporter en ZIP (clé USB, autre lecteur…)">
          <Icon name="archive" size={17} /> Exporter
        </button>
      {/if}
      {#if $isAdmin && CAN_KEEP}
        <button class="icon-btn" on:click={unsubscribe} aria-label="Se désabonner" title="Se désabonner">
          <Icon name="trash" size={22} />
        </button>
      {/if}
    </div>

    {#if $isAdmin && CAN_ADMIN && !data.local}
      <button
        class="auto"
        role="switch"
        aria-checked={!!data.auto_archive}
        disabled={autoBusy}
        on:click={toggleAuto}
      >
        <span class="auto-ico"><Icon name="archive" size={18} /></span>
        <span class="auto-txt">
          <span class="auto-title">Archiver automatiquement les nouveaux épisodes</span>
          <span class="auto-hint muted">
            {data.auto_archive ? (archiving ? "Archivage en cours" : "Activé") : "Désactivé"}
            · {Math.min(data.archived_count, data.episode_count)} sur {data.episode_count} épisode{data.episode_count > 1 ? "s" : ""} archivé{data.archived_count > 1 ? "s" : ""} sur le serveur
          </span>
          {#if data.auto_archive && data.episode_count}
            <span class="auto-bar" aria-hidden="true">
              <span style={`transform:scaleX(${Math.min(1, data.archived_count / data.episode_count)})`}></span>
            </span>
          {/if}
        </span>
        <span class="sw" class:on={data.auto_archive}><span class="knob"></span></span>
      </button>
    {/if}

    {#if $isAdmin && CAN_ADMIN}
      <section class="tr-card" aria-labelledby="tr-card-title">
        <div class="tr-head">
          <h2 id="tr-card-title">Transcription des épisodes</h2>
          <span class="tr-count">{readyCount} sur {archivedEpisodes.length}</span>
        </div>
        <p class="tr-sub">
          Le texte est produit sur ce serveur, à partir des fichiers archivés. Aucun audio n'est
          envoyé à un tiers.
        </p>

        {#if $transcribeSettings.state === "loading" && !transcribe}
          <p class="tr-state" role="status" aria-busy="true">Vérification de la transcription…</p>
        {:else if $transcribeSettings.state === "error" && !transcribe}
          <p class="tr-state" role="alert">Impossible de vérifier la transcription.</p>
          <button type="button" class="tr-btn ghost" on:click={() => loadTranscribeSettings(true)}>
            <Icon name="refresh" size={15} /> Réessayer
          </button>
        {:else if !transcribe}
          <p class="tr-state" role="status" aria-busy="true">Vérification de la transcription…</p>
        {:else if transcribe.available === false}
          <p class="tr-state warn">{reasonText(transcribe.reason)}</p>
          <button type="button" class="tr-btn ghost" on:click={openSettings}>
            <Icon name="settings" size={15} /> Ouvrir Réglages
          </button>
        {:else}
          <button
            type="button"
            class="tr-switch"
            role="switch"
            aria-checked={!!data.auto_transcribe}
            disabled={batchBusy}
            on:click={toggleShowTranscribe}
          >
            <span class="tr-switch-text">
              <span class="tr-switch-title">Transcrire automatiquement les nouveaux épisodes</span>
              <span class="tr-switch-hint">
                {data.auto_transcribe ? "Activé" : "Désactivé"} · {readyCount} sur {archivedEpisodes.length} épisode{archivedEpisodes.length > 1 ? "s" : ""} archivé{archivedEpisodes.length > 1 ? "s" : ""} transcrit{readyCount > 1 ? "s" : ""}
              </span>
            </span>
            <span class="sw" class:on={data.auto_transcribe}><span class="knob"></span></span>
          </button>

          <div class="tr-fields">
            <label class="tr-field">
              <span>Rattrapage à l'activation</span>
              <select value={backlogMode} on:change={(e) => (backlogMode = e.currentTarget.value)}>
                <option value="all">Tout l'historique archivé</option>
                <option value="recent">Les 10 épisodes archivés les plus récents</option>
                <option value="none">Aucun rattrapage</option>
              </select>
            </label>
            <label class="tr-field">
              <span>Langue de l'émission</span>
              <select
                value={data.transcribe_language || "auto"}
                on:change={(e) => setShowLanguage(e.currentTarget.value)}
              >
                {#each ["auto", "fr", "en", "es", "de", "it", "pt"] as code (code)}
                  <option value={code}>
                    {code === "auto" ? "Détection automatique" : code === "fr" ? "Français" : code === "en" ? "Anglais" : code === "es" ? "Espagnol" : code === "de" ? "Allemand" : code === "it" ? "Italien" : "Portugais"}
                  </option>
                {/each}
              </select>
            </label>
          </div>

          {#if eligible.length}
            <p class="tr-estimate">
              {eligible.length} épisode{eligible.length > 1 ? "s" : ""} · {audioHours} h d'audio,
              environ {cpuHours} h de calcul.
            </p>
            <div class="tr-batch">
              <label class="tr-batch-label">
                Transcrire les
                <select bind:value={batchCount}>
                  {#each choices as count (count)}
                    <option value={count}>{count}</option>
                  {/each}
                </select>
                épisodes archivés les plus récents
              </label>
              <button type="button" class="tr-btn primary" on:click={startBatch} disabled={batchBusy}>
                {batchBusy ? "Envoi…" : "Transcrire"}
              </button>
            </div>
            <p class="tr-help">
              Un seul épisode à la fois, en arrière-plan. La transcription se met en pause pendant
              l'écoute.
            </p>
          {:else}
            <p class="tr-state">Aucun épisode archivé à transcrire.</p>
          {/if}
        {/if}

        {#if activeTranscript}
          {@const pct = Math.round((Number(activeTranscript.transcript?.progress) || 0) * 100)}
          <div class="tr-progress">
            <span class="tr-progress-title">{activeTranscript.title}</span>
            <span
              class="tr-bar"
              role="progressbar"
              aria-label="Progression de la transcription"
              aria-valuemin="0"
              aria-valuemax="100"
              aria-valuenow={pct}
            >
              <span style={`transform:scaleX(${pct / 100})`}></span>
            </span>
            <span class="tr-pct">{pct} %</span>
            <button type="button" class="tr-btn tiny" on:click={() => stopTranscript(activeTranscript)}>
              Arrêter
            </button>
          </div>
        {/if}
        {#if queuedCount}
          <p class="tr-help">{queuedCount} épisode{queuedCount > 1 ? "s" : ""} en attente de transcription.</p>
        {/if}
        <button
          type="button"
          class="tr-btn ghost"
          on:click={() => openTranscriptExport(id, data.title)}
          title="Télécharger le texte des épisodes prêts (TXT, SRT, WebVTT, Markdown)"
        >
          <Icon name="download" size={15} /> Exporter les transcriptions
        </button>
      </section>
    {/if}

    {#if showSearch}
      <form
        class="tr-search"
        on:submit|preventDefault={() => runShowSearch(searchQ.trim(), true)}
      >
        <Icon name="search" size={16} />
        <input
          type="search"
          bind:value={searchQ}
          placeholder="Rechercher dans le texte des épisodes…"
          aria-label="Rechercher dans les transcriptions de cette émission"
          enterkeyhint="search"
          on:input={onSearchInput}
          on:keydown={onSearchKey}
        />
        {#if searchQ}
          <button type="button" class="tr-clear" on:click={clearShowSearch} aria-label="Effacer la recherche">
            Effacer
          </button>
        {/if}
      </form>
      {#if searchQ.trim().length === 1}
        <p class="tr-help">2 caractères minimum.</p>
      {/if}
    {/if}

    {#if searchError}
      <p class="tr-search-error" role="alert">
        {#if searchError.offline}
          La recherche dans les transcriptions a besoin du serveur.
        {:else if searchError.status === 429}
          Trop de recherches. Réessayez dans {searchRetryLeft} s.
        {:else}
          La recherche a échoué.
          <button type="button" class="tr-link" on:click={() => runShowSearch(searchFor || searchQ.trim(), true)}>
            Réessayer
          </button>
        {/if}
      </p>
    {/if}

    {#if searching}
      {#if searchLoading}
        <p class="tr-state" role="status">Recherche dans les épisodes…</p>
      {:else}
        <div class="tr-results-head">
          <span>{searchTotal} passage{searchTotal > 1 ? "s" : ""}</span>
          <button type="button" class="tr-link" on:click={clearShowSearch}>Effacer</button>
        </div>
        {#if searchScope?.truncated}
          <p class="tr-help">
            Recherche dans les {searchScope.episodes} épisodes transcrits les plus récents.
          </p>
        {/if}
        {#if searchHits.length}
          <TranscriptHits hits={searchHits} query={searchFor} showChannel={false} onPick={pickTranscriptHit} />
        {:else}
          <p class="tr-state">Aucun passage pour « {searchQ.trim()} » dans cette émission.</p>
        {/if}
      {/if}
    {:else if !episodes.length}
      <p class="muted">Aucun épisode disponible.</p>
    {:else}
      <ul class="episodes">
        {#each episodes as ep, i (trackKey(ep))}
          {@const active = $currentId === trackKey(ep)}
          {@const resume = resumeOf(ep)}
          {@const marks = $episodeMarkers[trackKey(ep)] || []}
          {@const key = trackKey(ep)}
          {@const cOpen = chapterOpen.has(key)}
          {@const cEntry = $chaptersByEpisode[String(ep.id)] || null}
          {@const cList = cEntry?.chapters || []}
          {@const cCap = chapterCap[key] || 8}
          {@const cPos = chapterPos[key] || 0}
          {@const cAt = $currentId === key ? chapterAt(cList, cPos) : null}
          {@const tr = ep.transcript === undefined ? undefined : ep.transcript || null}
          {@const trPct = Math.round((Number(tr?.progress) || 0) * 100)}
          <!-- svelte-ignore a11y-no-static-element-interactions a11y-no-noninteractive-element-interactions -->
          <li
            class:active
            tabindex="-1"
            on:contextmenu={(e) => episodeMenu(e, ep)}
            on:keydown={(e) => menuKey(e, (ev) => episodeMenu(ev, ep))}
          >
            <button class="pl" on:click={() => toggle(i, ep)} aria-label="Lire l'épisode">
              <Icon name={active && $playing ? "pause" : "play"} size={18} />
            </button>
            <div class="body">
              <div class="top">
                <span class="etitle" class:on={active}>{ep.title}</span>
                <span class="side-r">
                  <span class="when muted">{fmtDate(ep.published)}</span>
                  <button class="emenu" on:click={(e) => episodeMenu(e, ep)} aria-label="Options de l'épisode" aria-haspopup="menu">
                    <Icon name="moreVertical" size={17} />
                  </button>
                </span>
              </div>
              {#if ep.description}
                <button
                  type="button"
                  class="edesc"
                  class:clamped={!expanded.has(trackKey(ep))}
                  on:click={(e) => toggleExpand(trackKey(ep), e.currentTarget)}
                >
                  {ep.description}
                </button>
              {/if}
              <div class="foot muted">
                {#if resume?.done}
                  <span class="doneb"><Icon name="check" size={13} /> Terminé</span>
                {:else if resume}
                  <span class="resume">Reprendre · il reste {fmtDuration(resume.left)}</span>
                {:else}
                  {fmtDuration(ep.duration)}
                {/if}
                {#if ep.status === "completed"}· <Icon name="downloaded" size={14} />{/if}
                {#if ep.unavailable}
                  · <span class="gone"><Icon name="alert" size={13} /> non archivé</span>
                {/if}
                {#if (ep.chapter_count || 0) > 0}
                  <button
                    type="button"
                    class="chapters-chip"
                    aria-expanded={cOpen}
                    aria-controls={"echapters-" + key}
                    title={cOpen ? "Masquer les chapitres" : "Afficher les chapitres"}
                    on:click={() => toggleChapters(ep)}
                  >
                    <Icon name="chapters" size={14} />
                    <span>{chapterCountLabel(ep.chapter_count)}</span>
                    <span class="chev" class:rot={cOpen}><Icon name="chevronDown" size={14} /></span>
                  </button>
                {/if}
                {#if tr !== undefined}
                  {#if tr?.status === "ready"}
                    <button
                      type="button"
                      class="tr-chip"
                      title="Ouvrir la transcription dans le lecteur"
                      aria-label={"Lire la transcription de " + ep.title}
                      on:click={() => openTranscript(ep)}
                    >
                      <Icon name="transcript" size={14} /> Transcription
                    </button>
                  {:else if tr && (tr.status === "running" || tr.status === "leased" || tr.status === "queued")}
                    {#if $isAdmin && CAN_ADMIN}
                      <button
                        type="button"
                        class="tr-chip stop"
                        aria-label="Arrêter la transcription"
                        on:click={() => stopTranscript(ep)}
                      >
                        <Icon name="transcript" size={14} />
                        {tr.status === "queued" ? "En attente" : `${trPct} %`} · Arrêter
                      </button>
                    {:else}
                      <span class="tr-chip static">
                        <Icon name="transcript" size={14} />
                        {tr.status === "queued" ? "En attente" : `${trPct} %`}
                      </span>
                    {/if}
                  {:else if tr?.status === "stopped"}
                    <span class="tr-chip static">
                      <Icon name="transcript" size={14} />
                      {$isAdmin && CAN_ADMIN ? `Arrêtée · ${trPct} %` : "Arrêtée"}
                    </span>
                  {:else if tr?.status === "error"}
                    <span class="tr-chip static">
                      <Icon name="alert" size={14} /> Échec
                    </span>
                  {:else if (!tr || tr.status === "none") && $isAdmin && CAN_ADMIN && ep.archived && transcribeAvailable}
                    <button
                      type="button"
                      class="tr-chip"
                      disabled={!!transcribeBusy[ep.id]}
                      aria-label={"Transcrire " + ep.title}
                      on:click={() => startOne(ep)}
                    >
                      {transcribeBusy[ep.id] ? "Envoi…" : "Transcrire"}
                    </button>
                  {/if}
                {/if}
              </div>
              {#if resume && !resume.done}
                <div class="ebar"><span style={`width:${resume.pct * 100}%`}></span></div>
              {/if}
              <div
                class="chblock"
                id={"echapters-" + key}
                role="region"
                aria-label="Chapitres"
                hidden={!cOpen}
              >
                {#if cOpen}
                  {#if !cEntry || cEntry.state === "loading" || cEntry.state === "idle"}
                    <p class="ch-note" role="status">Chargement des chapitres…</p>
                    <ChapterList loading />
                  {:else if cEntry.state === "error"}
                    <p class="ch-note">
                      {cEntry.error?.offline
                        ? "Hors ligne : les chapitres n'ont pas encore été chargés."
                        : "Chapitres indisponibles."}
                    </p>
                    <button type="button" class="ch-retry" on:click={() => loadChapters(ep.id, { force: true })}>
                      <Icon name="refresh" size={15} /> Réessayer
                    </button>
                  {:else if cEntry.archived === false}
                    <p class="ch-note">Les chapitres seront lus à la fin de l'archivage.</p>
                  {:else if !cList.length}
                    <p class="ch-note">Aucun chapitre pour cet épisode.</p>
                  {:else}
                    {#if cEntry.offline}
                      <p class="ch-note ch-offline"><Icon name="cloudOff" size={14} /> Hors ligne — dernière copie</p>
                    {/if}
                    <ChapterList
                      chapters={cList.slice(0, cCap)}
                      currentIndex={cAt ? cAt.index : -1}
                      variant="inline"
                      onSeek={(start) => seekChapter(ep, i, start)}
                    />
                    {#if cList.length > cCap}
                      <button type="button" class="ch-see" aria-haspopup="dialog" on:click={() => openChapters(ep, i)}>
                        Voir les {cList.length} chapitres <Icon name="chevronRight" size={15} />
                      </button>
                    {/if}
                  {/if}
                {/if}
              </div>
              {#if marks.length}
                <div class="marks">
                  {#each marks as m (m.id)}
                    <span class="mark">
                      <button
                        class="mjump"
                        on:click={() => playAt(i, ep, m.position)}
                        title={"Lire à " + fmtDuration(m.position)}
                      >
                        <Icon name="bookmarkFilled" size={12} />
                        <span class="mt">{fmtDuration(m.position)}</span>
                        {#if m.label}<span class="ml">{m.label}</span>{/if}
                      </button>
                      <button class="mdel" on:click={() => removeMarker(m)} aria-label="Supprimer le marqueur">
                        <Icon name="close" size={12} />
                      </button>
                    </span>
                  {/each}
                </div>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
      {#if nextEpisodes}
        <div class="ep-more">
          <button
            type="button"
            class="tr-btn ghost"
            disabled={loadingEpisodes}
            on:click={loadMoreEpisodes}
          >
            {loadingEpisodes ? "Chargement…" : "Charger plus d'épisodes"}
          </button>
        </div>
      {/if}
    {/if}
  </div>
{/if}

{#if sheet}
  {@const skey = trackKey(sheet.ep)}
  {@const sEntry = $chaptersByEpisode[String(sheet.ep.id)] || null}
  {@const sList = sEntry?.chapters || []}
  {@const sPos = chapterPos[skey] || 0}
  {@const sAt = $currentId === skey ? chapterAt(sList, sPos) : null}
  <ChapterSheet
    episode={sheet.ep}
    chapters={sList}
    currentIndex={sAt ? sAt.index : -1}
    state={sEntry?.state || "idle"}
    error={sEntry?.error || null}
    onSeek={(start) => seekChapter(sheet.ep, sheet.i, start)}
    onClose={() => (sheet = null)}
    onRetry={() => loadChapters(sheet.ep.id, { force: true })}
  />
{/if}

<style>
  /* The show's archive switch: a row of its own under the actions, the same
     switch as Réglages, with what it has done said under its name. */
  .auto {
    display: flex;
    align-items: center;
    gap: 14px;
    width: 100%;
    margin: 4px 0 18px;
    padding: 12px 16px;
    border-radius: var(--radius);
    background: var(--bg-card);
    text-align: left;
    color: var(--text);
  }
  .auto:hover:not(:disabled) {
    background: var(--bg-hover);
  }
  .auto-ico {
    flex: none;
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    border-radius: 10px;
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
  }
  .auto-txt {
    display: flex;
    flex-direction: column;
    gap: 3px;
    flex: 1;
    min-width: 0;
  }
  .auto-title {
    font-weight: 650;
  }
  .auto-hint {
    font-size: 0.82rem;
  }
  .auto-bar {
    display: block;
    height: 3px;
    margin-top: 4px;
    border-radius: 2px;
    background: var(--bg-hover);
    overflow: hidden;
  }
  .auto-bar span {
    display: block;
    height: 100%;
    background: var(--accent);
    transform-origin: left;
    /* I8: a job's progress is a scaleX on --dur-base linear, never a width. */
    transition: transform var(--dur-base) linear;
  }
  .sw {
    flex: none;
    width: 44px;
    height: 26px;
    border-radius: 999px;
    background: var(--s2);
    border: 1px solid var(--dim);
    position: relative;
    transition:
      background-color var(--dur-instant) var(--ease-standard),
      border-color var(--dur-instant) var(--ease-standard);
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
    transition:
      transform var(--dur-instant) var(--ease-standard),
      background-color var(--dur-instant) var(--ease-standard);
  }
  .sw.on .knob {
    background: var(--onbtn);
    transform: translateX(16px);
  }

  .localnote {
    margin: 10px 0 0;
    max-width: 60ch;
    font-size: 0.85rem;
    line-height: 1.45;
    color: var(--text-dim);
  }
  .gone {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--accent-2);
  }
  .art {
    width: 200px;
    flex: none;
    box-shadow: 0 16px 40px rgba(0, 0, 0, 0.5);
    border-radius: var(--radius);
    overflow: hidden;
  }
  .meta {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }
  .kind {
    font-size: 0.8rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  h1 {
    font-size: clamp(1.6rem, 4.5vw, 2.6rem);
  }
  .desc {
    color: var(--text-dim);
    font-size: 0.9rem;
    max-width: 60ch;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .actions {
    margin: 16px 0 18px;
    gap: 12px;
    align-items: center;
    flex-wrap: wrap;
  }
  /* Same secondary action as the library/playlist pages, so "Tout télécharger"
     and "Exporter" read identically wherever they appear. */
  .ghost-btn {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    padding: 9px 15px;
    border-radius: 999px;
    border: 1px solid var(--bg-hover);
    color: var(--text-dim);
    font-weight: 600;
    font-size: 0.9rem;
    white-space: nowrap;
  }
  .ghost-btn:hover:not(:disabled) {
    color: var(--text);
    border-color: var(--text-dim);
  }
  .ghost-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .icon-btn {
    color: var(--text-dim);
    margin-left: auto;
  }
  .icon-btn:hover {
    color: var(--text);
  }
  .episodes {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }
  .episodes li {
    display: flex;
    gap: 14px;
    padding: 14px 10px;
    border-top: 1px solid var(--bg-hover);
    align-items: flex-start;
  }
  .episodes li.active {
    background: var(--bg-hover);
    border-radius: var(--radius);
  }
  .pl {
    width: 40px;
    height: 40px;
    flex: none;
    border-radius: 50%;
    background: var(--bg-card);
    color: var(--text);
    display: grid;
    place-items: center;
    margin-top: 2px;
  }
  .pl:hover {
    background: var(--btn);
    color: var(--onbtn);
  }
  .body {
    min-width: 0;
    flex: 1;
  }
  .top {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    align-items: baseline;
  }
  .etitle {
    font-weight: 700;
  }
  .etitle.on {
    color: var(--accent-2);
  }
  .side-r {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: none;
  }
  .when {
    font-size: 0.8rem;
    white-space: nowrap;
    flex: none;
  }
  .emenu {
    color: var(--text-dim);
    display: grid;
    place-items: center;
    padding: 2px;
    border-radius: 6px;
  }
  .emenu:hover {
    color: var(--text);
    background: var(--bg-hover);
  }
  .edesc {
    font-size: 0.88rem;
    color: var(--text-dim);
    margin: 6px 0;
    cursor: pointer;
    text-align: left;
    display: block;
    width: 100%;
    background: none;
    border: none;
    padding: 0;
    font-family: inherit;
  }
  .edesc.clamped {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .foot {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    row-gap: 4px;
    font-size: 0.82rem;
  }
  .chapters-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 32px;
    padding: 0 10px;
    border-radius: 999px;
    border: 1px solid var(--ln);
    color: var(--dim);
    font-size: 0.78rem;
    font-weight: 600;
    white-space: nowrap;
  }
  .chapters-chip > :global(svg) {
    color: var(--ac-text);
  }
  .chapters-chip:hover {
    background: var(--s2);
    color: var(--tx);
  }
  .chapters-chip:focus-visible {
    outline: 2px solid var(--ac-text);
    outline-offset: 2px;
  }
  .chev {
    display: inline-flex;
    transition: transform var(--dur-base) var(--ease-standard);
  }
  .chev.rot {
    transform: rotate(180deg);
  }
  @media (pointer: coarse) {
    .chapters-chip {
      height: 44px;
      padding: 0 14px;
    }
  }
  .chblock {
    margin-top: 10px;
    border-top: 1px solid var(--ln);
    padding-top: 8px;
  }
  .chblock[hidden] {
    display: none;
  }
  .ch-note {
    margin: 6px 2px 8px;
    font-size: 0.86rem;
    line-height: 1.5;
    color: var(--text-dim);
  }
  .ch-offline {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--accent);
  }
  .ch-retry {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    min-height: 44px;
    margin: 0 0 6px 2px;
    padding: 0 14px;
    border-radius: 999px;
    border: 1px solid var(--ln);
    color: var(--ac-text);
    font-size: 0.88rem;
    font-weight: 600;
  }
  .ch-retry:hover {
    background: var(--s2);
  }
  .ch-retry:focus-visible {
    outline: 2px solid var(--ac-text);
    outline-offset: 2px;
  }
  .ch-see {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    width: 100%;
    min-height: 44px;
    margin-top: 2px;
    color: var(--ac-text);
    font-size: 0.88rem;
    font-weight: 600;
  }
  .ch-see:hover {
    background: var(--s2);
  }
  .ch-see:focus-visible {
    outline: 2px solid var(--ac-text);
    outline-offset: -2px;
  }
  @media (prefers-reduced-motion: reduce) {
    .chev {
      transition: none;
    }
    /* M14: the switch's thumb is a displacement, and a job's bar grows by a
       scale — both land as cuts under reduced motion. */
    .sw,
    .knob,
    .auto-bar span {
      transition: none;
    }
  }
  .resume {
    color: var(--accent);
    font-weight: 600;
  }
  .doneb {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--text-dim);
    font-weight: 600;
  }
  .ebar {
    margin-top: 8px;
    height: 3px;
    border-radius: 2px;
    background: var(--bg-hover);
    overflow: hidden;
    max-width: 320px;
  }
  .ebar span {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .marks {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 10px;
  }
  .mark {
    display: inline-flex;
    align-items: center;
    background: var(--bg-card);
    border: 1px solid var(--bg-hover);
    border-radius: 999px;
    overflow: hidden;
  }
  .mjump {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px 4px 10px;
    color: var(--text-dim);
    font-size: 0.78rem;
    max-width: 240px;
  }
  .mjump:hover {
    color: var(--text);
    background: var(--bg-hover);
  }
  .mjump :global(svg) {
    color: var(--accent);
    flex: none;
  }
  .mt {
    font-variant-numeric: tabular-nums;
    font-weight: 600;
  }
  .ml {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .mdel {
    display: grid;
    place-items: center;
    padding: 4px 8px 4px 4px;
    color: var(--text-dim);
  }
  .mdel:hover {
    color: var(--text);
  }
  .tr-card {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin: 4px 0 18px;
    padding: 16px;
    border-radius: var(--radius);
    background: var(--bg-card);
    min-width: 0;
  }
  .tr-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
  }
  .tr-head h2 {
    margin: 0;
    font-family: var(--font-body);
    font-size: 1rem;
    font-weight: 700;
  }
  .tr-count {
    flex: none;
    color: var(--dim);
    font-family: var(--font-mono);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }
  .tr-sub,
  .tr-help {
    margin: 0;
    color: var(--dim);
    font-size: 0.82rem;
    line-height: 1.45;
    max-width: 70ch;
  }
  .tr-state {
    margin: 0;
    color: var(--dim);
    font-size: 0.84rem;
    line-height: 1.45;
  }
  .tr-state.warn {
    color: var(--accent-2, #e05252);
  }
  .tr-switch {
    display: flex;
    align-items: center;
    gap: 14px;
    width: 100%;
    min-height: 56px;
    padding: 8px 10px;
    border-radius: 12px;
    background: var(--s1);
    text-align: left;
    color: var(--text);
  }
  .tr-switch:hover:not(:disabled) {
    background: var(--s2);
  }
  .tr-switch:disabled {
    opacity: 0.65;
    cursor: default;
  }
  .tr-switch:focus-visible {
    outline: 2px solid var(--ac-text);
    outline-offset: 2px;
  }
  .tr-switch-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
  }
  .tr-switch-title {
    font-weight: 650;
    font-size: 0.9rem;
  }
  .tr-switch-hint {
    color: var(--dim);
    font-size: 0.78rem;
  }
  .tr-fields {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 10px 16px;
  }
  .tr-field {
    display: flex;
    flex-direction: column;
    gap: 5px;
    min-width: 0;
    color: var(--dim);
    font-size: 0.78rem;
    font-weight: 600;
  }
  .tr-field select {
    min-height: 44px;
    padding: 0 10px;
    border-radius: 10px;
    border: 1px solid var(--bg-hover);
    background: var(--bg);
    color: var(--text);
    font-size: 0.86rem;
    font-weight: 600;
    max-width: 100%;
  }
  .tr-estimate {
    margin: 0;
    color: var(--text-dim);
    font-family: var(--font-mono);
    font-size: 10.5px;
    letter-spacing: 0.03em;
    line-height: 1.5;
  }
  .tr-batch {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  .tr-batch-label {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    color: var(--text-dim);
    font-size: 0.86rem;
  }
  .tr-batch-label select {
    min-height: 44px;
    padding: 0 10px;
    border-radius: 10px;
    border: 1px solid var(--bg-hover);
    background: var(--bg);
    color: var(--text);
    font-weight: 650;
  }
  .ep-more {
    display: flex;
    justify-content: center;
    padding: 16px 0 8px;
  }

  .tr-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    min-height: 44px;
    padding: 0 15px;
    border-radius: 999px;
    border: 1px solid transparent;
    font-size: 0.86rem;
    font-weight: 650;
    white-space: nowrap;
  }
  .tr-btn.primary {
    background: var(--btn);
    color: var(--onbtn);
  }
  .tr-btn.ghost {
    border-color: var(--ln);
    color: var(--ac-text);
  }
  .tr-btn.ghost:hover {
    background: var(--s2);
  }
  .tr-btn.tiny {
    min-height: 44px;
    padding: 0 12px;
    border-color: var(--ln);
    color: var(--ac-text);
    font-size: 0.8rem;
  }
  .tr-btn:disabled {
    opacity: 0.55;
    cursor: default;
  }
  .tr-btn:focus-visible,
  .tr-chip:focus-visible,
  .tr-clear:focus-visible,
  .tr-link:focus-visible,
  .tr-search:focus-within {
    outline: 2px solid var(--ac-text);
    outline-offset: 2px;
  }
  .tr-progress {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto auto;
    align-items: center;
    gap: 8px;
  }
  .tr-progress-title {
    grid-column: 1 / -1;
    min-width: 0;
    color: var(--text-dim);
    font-size: 0.8rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tr-bar {
    display: block;
    height: 4px;
    border-radius: 999px;
    background: var(--bg-hover);
    overflow: hidden;
  }
  .tr-bar span {
    display: block;
    height: 100%;
    border-radius: 999px;
    background: var(--accent);
    transform-origin: left;
    /* I8: the transcription job's progress, on --dur-base linear. */
    transition: transform var(--dur-base) linear;
  }
  .tr-pct {
    flex: none;
    color: var(--text-dim);
    font-family: var(--font-mono);
    font-size: 10.5px;
    font-variant-numeric: tabular-nums;
  }
  .tr-search {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 44px;
    margin: 4px 0 10px;
    padding: 0 12px;
    border: 1px solid var(--ln);
    border-radius: 999px;
    background: var(--s1);
    color: var(--dim);
  }
  .tr-search input {
    flex: 1;
    min-width: 0;
    height: 100%;
    border: 0;
    background: transparent;
    color: var(--text);
    font-size: 0.9rem;
    text-overflow: ellipsis;
  }
  .tr-search input::placeholder {
    color: var(--dim);
    opacity: 1;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tr-search input::-webkit-search-cancel-button {
    -webkit-appearance: none;
  }
  .tr-search input:focus-visible {
    outline: none;
  }
  .tr-clear {
    flex: none;
    min-height: 36px;
    padding: 0 8px;
    color: var(--ac-text);
    font-size: 0.8rem;
    font-weight: 650;
  }
  .tr-search-error {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    margin: 0 0 10px;
    color: var(--text-dim);
    font-size: 0.8rem;
    line-height: 1.45;
  }
  .tr-results-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    margin: 8px 0;
    color: var(--dim);
    font-size: 0.8rem;
  }
  .tr-link {
    padding: 0;
    color: var(--ac-text);
    font-size: inherit;
    font-weight: 650;
  }
  .tr-link:hover {
    text-decoration: underline;
  }
  .tr-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 32px;
    padding: 0 10px;
    border-radius: 999px;
    border: 1px solid var(--ln);
    color: var(--dim);
    font-size: 0.78rem;
    font-weight: 600;
    white-space: nowrap;
  }
  .tr-chip > :global(svg) {
    color: var(--ac-text);
  }
  .tr-chip:hover:not(:disabled) {
    background: var(--s2);
    color: var(--tx);
  }
  .tr-chip.stop {
    color: var(--ac-text);
    border-color: var(--ac-text);
  }
  .tr-chip.static {
    cursor: default;
  }
  .tr-chip:disabled {
    opacity: 0.6;
    cursor: default;
  }
  @media (pointer: coarse) {
    .tr-chip {
      height: 44px;
      padding: 0 14px;
    }
    .tr-clear {
      min-height: 44px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .tr-bar span {
      transition: none;
    }
  }
  @media (max-width: 640px) {
    .art {
      width: 150px;
    }
    .tr-card {
      padding: 14px;
    }
  }
</style>
