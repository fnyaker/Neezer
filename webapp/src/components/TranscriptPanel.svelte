<script>
  import { onDestroy, tick } from "svelte";
  import { push } from "svelte-spa-router";
  import { get } from "svelte/store";
  import { online } from "../lib/net.js";
  import { api } from "../lib/api.js";
  import { isAdmin, immersiveOpen } from "../lib/stores.js";
  import { CAN_ADMIN } from "../lib/remote/mode.js";
  import { duration as fmtDuration } from "../lib/format.js";
  import { chaptersByEpisode, loadChapters } from "../lib/chapters.js";
  import {
    LANGUAGES,
    excerptAround,
    foldText,
    highlightParts,
    isActiveStatus,
    isTerminalStatus,
    languageLabel,
    loadTranscribeSettings,
    noteTranscript,
    pageInfo,
    reasonText,
    segmentAt,
    transcribeSettings,
  } from "../lib/transcript.js";
  import { firstFor, forget, readAt, readPage, writePage } from "../lib/transcriptcache.js";
  import { fadeIn, pop, scrollBehavior } from "../lib/motion.js";
  import Icon from "./Icon.svelte";

  export let episode = null;
  export let position = 0;
  export let isCurrent = false;
  export let initialPosition = null;
  export let active = true;
  export let onSeek = null;

  const PAGE = 200;
  const SEARCH_PAGE = 100;
  const POLL_MS = 10000;
  const SEARCH_DELAY = 300;

  let epId = "";
  let lastEp = "";
  let started = false;
  let limit = PAGE;
  let status = "none";
  let archived = null;
  let segments = [];
  let total = 0;
  let offset = 0;
  let language = null;
  let durationMs = 0;
  let doneMs = 0;
  let progress = 0;
  let percent = 0;
  let errorMessage = "";
  let errorCode = "";
  let loaded = false;
  let loading = false;
  let loadError = null;
  let offline = false;
  let fromCache = false;
  let pageSeq = 0;
  let pollTimer = null;
  let pollBusy = false;
  let announceText = "";
  let rowEls = [];
  let listEl = null;
  let keyboardIdx = -1;
  let follow = true;
  let lastActive = -1;
  let followEp = "";
  let flashStart = null;
  let flashTimer = null;

  let q = "";
  let searchTimer = null;
  let searchSeq = 0;
  let searchFor = "";
  let searchAttempt = "";
  let searchHits = [];
  let searchTotal = 0;
  let searchLoading = false;
  let searchError = null;
  let searchMode = false;
  let searchLocal = false;
  let afterHit = null;

  let menu = null;
  let menuOpen = false;
  let exportOpen = false;
  const EXPORT_FORMATS = [
    { id: "txt", label: "Texte (.txt)" },
    { id: "srt", label: "Sous-titres (.srt)" },
    { id: "vtt", label: "WebVTT (.vtt)" },
    { id: "md", label: "Markdown (.md)" },
  ];
  let actionBusy = false;
  let actionError = "";
  let retryLeft = 0;
  let retryTimer = null;
  let relaunchLanguage = "auto";

  $: admin = $isAdmin && CAN_ADMIN;
  $: epId = episode?.id == null ? "" : String(episode.id);
  $: if (epId !== lastEp) {
    lastEp = epId;
    resetEpisode();
  }

  $: chapters = epId ? $chaptersByEpisode[epId]?.chapters || [] : [];
  $: if (epId && episode?.podcast) loadChapters(epId);

  $: hasText = segments.length > 0;
  $: info = pageInfo(offset, limit, total);
  $: activeIdx = segmentAt(segments, position);
  $: tabIdx =
    keyboardIdx >= 0 && segments.length
      ? Math.min(keyboardIdx, segments.length - 1)
      : activeIdx >= 0
        ? activeIdx
        : 0;
  $: chapterMarks = buildChapterMarks(segments, chapters);
  $: canUseSettings = $transcribeSettings.data || null;

  $: if (active && epId && !started) {
    started = true;
    openPanel();
  }

  $: if (!active) stopPoll();

  $: if (active && started && epId && isActiveStatus(status) && !pollTimer && !pollBusy) {
    armPoll();
  }

  $: if (activeIdx !== lastActive) {
    lastActive = activeIdx;
    if (follow && active && loaded && segments.length && activeIdx >= 0) {
      tick().then(() => scrollToRow(activeIdx, 0.35));
    }
  }

  $: if (epId && followEp !== epId) {
    followEp = epId;
    follow = true;
  }

  function minutes(value) {
    return Math.max(0, Math.round((Number(value) || 0) / 60000));
  }

  function metaText(code) {
    return code
      ? `Langue : ${languageLabel(code)} · Transcription automatique — peut contenir des erreurs.`
      : "Transcription automatique — peut contenir des erreurs.";
  }

  function setProgress(value) {
    progress = Math.max(0, Math.min(1, Number(value) || 0));
    percent = Math.round(progress * 100);
  }

  function resetEpisode() {
    clearTimeout(pollTimer);
    clearTimeout(searchTimer);
    clearTimeout(flashTimer);
    stopRetryCountdown();
    pollTimer = null;
    pageSeq++;
    started = false;
    status = "none";
    archived = episode?.archived == null ? null : !!episode.archived;
    segments = [];
    total = 0;
    offset = 0;
    language = null;
    durationMs = Number(episode?.duration) > 0 ? Number(episode.duration) * 1000 : 0;
    doneMs = 0;
    setProgress(0);
    errorMessage = "";
    errorCode = "";
    loaded = false;
    loading = false;
    loadError = null;
    offline = false;
    fromCache = false;
    rowEls = [];
    keyboardIdx = -1;
    lastActive = -1;
    follow = true;
    flashStart = null;
    q = "";
    searchFor = "";
    searchAttempt = "";
    searchHits = [];
    searchTotal = 0;
    searchLoading = false;
    searchError = null;
    searchMode = false;
    searchLocal = false;
    afterHit = null;
    menu = null;
    menuOpen = false;
    actionError = "";
    retryLeft = 0;
  }

  function applyEpisodeState() {
    archived = episode?.archived == null ? null : !!episode.archived;
    const info = episode?.transcript;
    if (info && typeof info === "object") {
      if (info.status) status = info.status;
      if (info.progress != null) setProgress(info.progress);
      if (info.language) language = info.language;
      if (episode?.duration) durationMs = Number(episode.duration) * 1000;
    }
  }

  function announce(next) {
    if (next === "queued") announceText = "La transcription est en attente.";
    else if (next === "running" || next === "leased")
      announceText = "La transcription est en cours.";
    else if (next === "ready") announceText = "La transcription est prête.";
    else if (next === "stopped") announceText = "La transcription est arrêtée.";
    else if (next === "error") announceText = "La transcription a échoué.";
  }

  function applyMeta(payload) {
    if (!payload) return;
    const next = payload.status || status;
    if (next !== status) {
      status = next;
      announce(next);
    }
    if (payload.archived != null) archived = !!payload.archived;
    if (payload.progress != null) setProgress(payload.progress);
    if (payload.done_ms != null) doneMs = Number(payload.done_ms) || 0;
    if (payload.duration_ms) durationMs = Number(payload.duration_ms) || 0;
    if (payload.language) language = payload.language;
    errorMessage = payload.error?.message || "";
    errorCode = payload.error?.code || "";
    if (payload.status && isTerminalStatus(payload.status)) stopPoll();
    noteTranscript(epId, {
      status,
      progress,
      language,
      error: payload.error || null,
      segment_count: Number(payload.total) || total,
    });
  }

  function applySegments(payload, focus = null) {
    segments = Array.isArray(payload.segments) ? payload.segments : [];
    total = Math.max(0, Math.floor(Number(payload.total) || segments.length));
    offset = Math.max(0, Math.floor(Number(payload.offset) || 0));
    limit = Math.max(1, Math.floor(Number(payload.limit) || limit));
    loaded = true;
    rowEls = [];
    lastActive = -1;
    keyboardIdx = -1;
    if (payload.duration_ms) durationMs = Number(payload.duration_ms) || durationMs;
    if (payload.language) language = payload.language;
    tick().then(() => {
      const target = focus != null ? focus : position;
      const at = segmentAt(segments, target);
      if (follow && isCurrent && at >= 0) scrollToRow(at, 0.35);
      else if (listEl) scrollParentTo(listEl, 0);
    });
  }

  async function openPanel() {
    if (!epId) return;
    applyEpisodeState();
    if (episode?.transcript === undefined) {
      const meta = await fetchMeta();
      if (!meta) return;
    }
    if (status === "none") {
      const target = initialTarget();
      let cached = await readAt(epId, limit, target.at != null ? target.at : 0);
      if (!cached) cached = await firstFor(epId);
      if (cached) {
        applySegments(cached, target.at != null ? target.at : null);
        status = "ready";
        fromCache = true;
        offline = !get(online);
        loadError = null;
        if (get(online)) await loadPage({ offset: cached.offset });
        return;
      }
      if (admin) loadTranscribeSettings();
      return;
    }
    if (status === "queued") return;
    await loadPage(initialTarget());
  }

  function initialTarget() {
    if (initialPosition != null && Number.isFinite(Number(initialPosition))) {
      return { at: Number(initialPosition) };
    }
    if (isCurrent && Number(position) > 0) return { at: Number(position) };
    return { offset: 0 };
  }

  async function fetchMeta() {
    try {
      const payload = await api.episodeTranscript(epId, { offset: 0, limit: 1 });
      applyMeta(payload);
      return payload;
    } catch (error) {
      handleLoadFailure(error, false);
      return null;
    }
  }

  function handleLoadFailure(error, hadCache) {
    if ((error && error.status === 0) || !get(online)) {
      offline = true;
      if (!hadCache) loadError = { offline: true };
      return;
    }
    loadError = {
      message: (error && error.body && error.body.error) || "La transcription n'a pas pu être chargée.",
    };
  }

  async function loadPage(target = {}, force = false) {
    if (!epId) return;
    const mine = ++pageSeq;
    const at = target.at != null ? Number(target.at) : null;
    const wantedOffset = Math.max(0, Math.floor(Number(target.offset) || 0));
    loading = true;
    loadError = null;
    let cached = null;
    if (at != null) cached = await readAt(epId, limit, at);
    else cached = await readPage(epId, limit, wantedOffset);
    if (mine !== pageSeq) return;
    if (cached) {
      applySegments(cached, at);
      fromCache = true;
    }
    if (!get(online) && !force) {
      loading = false;
      if (cached) offline = true;
      else handleLoadFailure({ status: 0 }, false);
      return;
    }
    try {
      const payload = await api.episodeTranscript(
        epId,
        at != null ? { limit, at } : { limit, offset: wantedOffset },
      );
      if (mine !== pageSeq) return;
      applyMeta(payload);
      applySegments(payload, at);
      offline = false;
      fromCache = false;
      loadError = null;
      if (payload.status === "ready") writePage(payload, episode);
      else if (payload.status === "none") forget(epId);
    } catch (error) {
      if (mine !== pageSeq) return;
      handleLoadFailure(error, !!cached);
    } finally {
      if (mine === pageSeq) loading = false;
    }
  }

  function refreshPage() {
    if (status === "none" || status === "queued") {
      openPanel();
      return;
    }
    loadPage({ offset }, true);
  }

  function armPoll() {
    stopPoll();
    if (!active || !epId) return;
    pollTimer = setTimeout(poll, POLL_MS);
  }

  function stopPoll() {
    clearTimeout(pollTimer);
    pollTimer = null;
  }

  async function poll() {
    pollTimer = null;
    if (!active || !epId) return;
    if (typeof document !== "undefined" && document.visibilityState === "hidden") {
      armPoll();
      return;
    }
    if (!get(online)) {
      armPoll();
      return;
    }
    pollBusy = true;
    const before = status;
    try {
      const payload = await api.episodeTranscript(epId, { offset: 0, limit: 1 });
      if (!active) return;
      applyMeta(payload);
      if (payload.status === "ready" && before !== "ready") {
        await loadPage({ offset });
      } else if (
        (payload.status === "running" || payload.status === "leased") &&
        !loaded &&
        (Number(payload.done_ms) > 0 || Number(payload.total) > 0)
      ) {
        await loadPage({ offset: 0 });
      }
    } catch {
    } finally {
      pollBusy = false;
    }
  }

  function scrollParent() {
    let node = listEl?.parentElement || null;
    while (node) {
      if (typeof getComputedStyle === "function") {
        const style = getComputedStyle(node);
        if (/(auto|scroll)/.test(style.overflowY)) return node;
      }
      node = node.parentElement;
    }
    return null;
  }

  function scrollParentTo(node, ratio) {
    const parent = scrollParent();
    if (!parent || !node) return;
    const view = parent.getBoundingClientRect();
    const row = node.getBoundingClientRect();
    const top = Math.max(0, parent.scrollTop + row.top - view.top - parent.clientHeight * ratio);
    // P18/P20: the ONE owner of "smooth" is motion.js — reduced motion and eco
    // jump at once, and this file keeps no matchMedia of its own.
    parent.scrollTo({ top, behavior: scrollBehavior() });
  }

  function scrollToRow(index, ratio) {
    const node = rowEls[index];
    if (node) scrollParentTo(node, ratio);
  }

  function pauseFollow() {
    follow = false;
  }

  function rejoin() {
    follow = true;
    if (activeIdx >= 0) {
      keyboardIdx = activeIdx;
      tick().then(() => scrollToRow(activeIdx, 0.35));
    }
  }

  function seek(seconds, index) {
    if (index != null) keyboardIdx = index;
    follow = true;
    if (onSeek) onSeek(Number(seconds));
  }

  function panelKeys(node) {
    const handler = (event) => {
      if (event.key !== " ") return;
      const target = event.target;
      if (!target || !target.closest) return;
      if (target.closest("input, textarea, select")) return;
      if (target.closest("button")) event.stopPropagation();
    };
    node.addEventListener("keydown", handler);
    return {
      destroy() {
        node.removeEventListener("keydown", handler);
      },
    };
  }

  function listEvents(node) {
    node.addEventListener("keydown", onListKey);
    node.addEventListener("focusout", onListBlur);
    node.addEventListener("wheel", pauseFollow, { passive: true });
    node.addEventListener("touchmove", pauseFollow, { passive: true });
    return {
      destroy() {
        node.removeEventListener("keydown", onListKey);
        node.removeEventListener("focusout", onListBlur);
        node.removeEventListener("wheel", pauseFollow);
        node.removeEventListener("touchmove", pauseFollow);
      },
    };
  }

  function onListKey(event) {
    if (!segments.length) return;
    const key = event.key;
    let next = tabIdx;
    if (key === "ArrowDown") next = Math.min(segments.length - 1, tabIdx + 1);
    else if (key === "ArrowUp") next = Math.max(0, tabIdx - 1);
    else if (key === "Home") next = 0;
    else if (key === "End") next = segments.length - 1;
    else if (key === "PageDown" || key === "PageUp" || key === " ") {
      if (key === "PageDown" || key === "PageUp") follow = false;
      return;
    } else {
      return;
    }
    event.preventDefault();
    keyboardIdx = next;
    tick().then(() => rowEls[next]?.focus());
  }

  function onRowFocus(index) {
    keyboardIdx = index;
  }

  function onListBlur(event) {
    const next = event.relatedTarget;
    if (!listEl || !next || !listEl.contains(next)) keyboardIdx = -1;
  }

  function buildChapterMarks(list, chapters) {
    const marks = new Map();
    if (!Array.isArray(list) || !list.length || !Array.isArray(chapters) || !chapters.length) {
      return marks;
    }
    const first = Number(list[0].start);
    const last = Number(list[list.length - 1].start);
    for (let index = 0; index < chapters.length; index++) {
      const chapter = chapters[index];
      const start = Number(chapter.start);
      if (!Number.isFinite(start) || start < first || start > last) continue;
      let row = list.findIndex((segment) => Number(segment.start) >= start);
      if (row < 0) continue;
      if (!marks.has(row)) marks.set(row, { chapter, index });
    }
    return marks;
  }

  function onSearchInput() {
    clearTimeout(searchTimer);
    const term = q.trim();
    if (term.length < 2) {
      clearSearchResults();
      return;
    }
    searchTimer = setTimeout(() => runSearch(term), SEARCH_DELAY);
  }

  function onSearchKey(event) {
    if (event.key === "Enter") {
      clearTimeout(searchTimer);
      runSearch(q.trim());
      event.currentTarget.blur();
    } else if (event.key === "Escape" && q) {
      event.stopPropagation();
      clearSearch();
    }
  }

  function clearSearchResults() {
    searchSeq++;
    searchFor = "";
    searchAttempt = "";
    searchHits = [];
    searchTotal = 0;
    searchLoading = false;
    searchError = null;
    searchMode = false;
    searchLocal = false;
    afterHit = null;
  }

  function clearSearch() {
    clearTimeout(searchTimer);
    q = "";
    clearSearchResults();
    if (listEl) scrollParentTo(listEl, 0);
  }

  async function runSearch(term, force = false) {
    if (!epId || term.length < 2) {
      clearSearchResults();
      return;
    }
    if (!force && (term === searchFor || (term === searchAttempt && searchLoading))) return;
    clearTimeout(searchTimer);
    const mine = ++searchSeq;
    searchAttempt = term;
    searchLoading = true;
    searchError = null;
    searchMode = true;
    afterHit = null;
    if (!get(online)) {
      localSearch(term);
      searchLoading = false;
      return;
    }
    try {
      const payload = await api.episodeTranscript(epId, { q: term, limit: SEARCH_PAGE });
      if (mine !== searchSeq) return;
      searchHits = (payload.segments || []).map((segment) => ({
        start: Number(segment.start) || 0,
        end: Number(segment.end) || 0,
        text: String(segment.text || ""),
      }));
      searchTotal = Math.max(0, Number(payload.total) || searchHits.length);
      searchFor = term;
      searchLocal = false;
      searchMode = true;
    } catch (error) {
      if (mine !== searchSeq) return;
      if ((error && error.status === 0) || !get(online)) {
        localSearch(term);
      } else {
        searchFor = term;
        searchHits = [];
        searchTotal = 0;
        searchError = error;
      }
    } finally {
      if (mine === searchSeq) searchLoading = false;
    }
  }

  function localSearch(term) {
    const words = foldText(term).split(/\s+/).filter(Boolean);
    const hits = segments
      .filter((segment) => {
        const folded = foldText(segment.text);
        return words.every((word) => folded.includes(word));
      })
      .map((segment) => ({
        start: Number(segment.start) || 0,
        end: Number(segment.end) || 0,
        text: excerptAround(segment.text, term, 240),
      }));
    searchHits = hits;
    searchTotal = hits.length;
    searchFor = term;
    searchLocal = true;
    searchMode = true;
    searchError = null;
  }

  function pickHit(hit, index) {
    const start = Number(hit.start) || 0;
    afterHit = { index: index + 1, total: searchHits.length };
    searchMode = false;
    searchLocal = false;
    flashStart = start;
    clearTimeout(flashTimer);
    flashTimer = setTimeout(() => {
      flashStart = null;
    }, 1800);
    seek(start, null);
    loadPage({ at: start });
  }

  function backToResults() {
    if (!searchFor) return;
    searchMode = true;
    afterHit = null;
  }

  function goPage(delta) {
    if (loading) return;
    const next = (info.page - 1 + delta) * limit;
    if (next < 0 || next >= Math.max(total, 1)) return;
    searchMode = false;
    afterHit = null;
    loadPage({ offset: next });
  }

  function openSettings() {
    immersiveOpen.set(false);
    push("/settings/server");
  }

  function startRetryCountdown(seconds) {
    stopRetryCountdown();
    const value = Math.max(1, Math.ceil(Number(seconds) || 0));
    retryLeft = value;
    retryTimer = setInterval(() => {
      retryLeft = Math.max(0, retryLeft - 1);
      if (retryLeft === 0) stopRetryCountdown();
    }, 1000);
  }

  function stopRetryCountdown() {
    clearInterval(retryTimer);
    retryTimer = null;
  }

  function describeFailure(error) {
    const code = (error && error.body && error.body.code) || (error && error.code) || "";
    const reason = error && error.body && error.body.reason;
    const retry = Number(
      (error && error.retryIn) || (error && error.body && error.body.retry_in) || 0,
    );
    if (retry > 0) startRetryCountdown(retry);
    if (code === "queue_full") {
      return retry > 0
        ? `La file de transcription est pleine. Réessayez dans ${retry} s.`
        : "La file de transcription est pleine.";
    }
    if (code === "rate_limited") {
      return retry > 0 ? `Trop de demandes. Réessayez dans ${retry} s.` : "Trop de demandes.";
    }
    if (reason) return reasonText(reason);
    const message = (error && error.body && error.body.error) || (error && error.message) || "";
    return message || "La transcription n'a pas pu démarrer.";
  }

  function handleActionFailure(error) {
    actionError = describeFailure(error);
    if (error && error.status === 409 && error.body && error.body.code === "already_running") {
      poll();
    }
  }

  async function startEpisode() {
    if (actionBusy || retryLeft > 0) return;
    actionBusy = true;
    actionError = "";
    const before = { status, progress };
    status = "queued";
    setProgress(0);
    noteTranscript(epId, { status: "queued", progress: 0 });
    announce("queued");
    try {
      await api.startEpisodeTranscript(epId, {});
    } catch (error) {
      status = before.status;
      setProgress(before.progress);
      noteTranscript(epId, { status, progress });
      handleActionFailure(error);
    } finally {
      actionBusy = false;
    }
  }

  async function resumeEpisode() {
    if (actionBusy || retryLeft > 0) return;
    actionBusy = true;
    actionError = "";
    try {
      await api.startEpisodeTranscript(epId, {});
      status = "queued";
      announce("queued");
      noteTranscript(epId, { status: "queued", progress });
      armPoll();
    } catch (error) {
      handleActionFailure(error);
    } finally {
      actionBusy = false;
    }
  }

  async function stopEpisode() {
    if (actionBusy) return;
    actionBusy = true;
    actionError = "";
    try {
      const payload = await api.stopEpisodeTranscript(epId);
      status = payload?.status || "stopped";
      announce("stopped");
      noteTranscript(epId, { status, progress });
    } catch (error) {
      handleActionFailure(error);
    } finally {
      actionBusy = false;
    }
  }

  async function relaunch() {
    if (actionBusy || retryLeft > 0) return;
    actionBusy = true;
    actionError = "";
    menu = null;
    try {
      await api.startEpisodeTranscript(epId, {
        force: true,
        language: relaunchLanguage === "auto" ? "auto" : relaunchLanguage,
      });
      await forget(epId);
      segments = [];
      total = 0;
      offset = 0;
      doneMs = 0;
      setProgress(0);
      errorMessage = "";
      errorCode = "";
      loaded = false;
      searchMode = false;
      searchHits = [];
      status = "queued";
      announce("queued");
      noteTranscript(epId, { status: "queued", progress: 0, language: relaunchLanguage });
      armPoll();
    } catch (error) {
      handleActionFailure(error);
    } finally {
      actionBusy = false;
    }
  }

  async function removeTranscript() {
    if (actionBusy) return;
    actionBusy = true;
    actionError = "";
    menu = null;
    try {
      await api.deleteEpisodeTranscript(epId);
      await forget(epId);
      status = "none";
      segments = [];
      total = 0;
      offset = 0;
      doneMs = 0;
      setProgress(0);
      language = null;
      errorMessage = "";
      errorCode = "";
      loaded = false;
      searchMode = false;
      searchHits = [];
      afterHit = null;
      noteTranscript(epId, null);
    } catch (error) {
      handleActionFailure(error);
    } finally {
      actionBusy = false;
    }
  }

  // A plain download: the server renders the file and the browser's own
  // download manager takes it (works in the Android WebView, no blob).
  function exportTranscript(fmt) {
    exportOpen = false;
    if (!epId) return;
    window.location.href = api.transcriptExportUrl(epId, fmt);
    announceText = "Export de la transcription lancé.";
  }

  function onKeydown(event) {
    if (event.key !== "Escape") return;
    if (exportOpen) {
      exportOpen = false;
      event.stopPropagation();
    } else if (menuOpen) {
      menuOpen = false;
      event.stopPropagation();
    } else if (menu) {
      menu = null;
      event.stopPropagation();
    }
  }

  onDestroy(() => {
    pageSeq++;
    searchSeq++;
    clearTimeout(pollTimer);
    clearTimeout(searchTimer);
    clearTimeout(flashTimer);
    stopRetryCountdown();
    pollTimer = null;
  });
</script>

<svelte:window on:keydown={onKeydown} />

<div class="tp" class:quiet={!active} use:panelKeys>
  <p class="sr" role="status" aria-live="polite">{announceText}</p>

  {#if loadError && !hasText}
    <div class="state-card" role="status">
      <span class="state-ico"><Icon name={loadError.offline ? "cloudOff" : "alert"} size={22} /></span>
      <h3>{loadError.offline ? "Transcription indisponible hors ligne." : "Transcription indisponible."}</h3>
      {#if loadError.message}<p>{loadError.message}</p>{/if}
      {#if !loadError.offline}<p>Le serveur n'a pas répondu ; le texte reste sur le serveur.</p>{/if}
      <button type="button" class="btn ghost" on:click={refreshPage}>
        <Icon name="refresh" size={16} /> Réessayer
      </button>
    </div>
  {:else if status === "none"}
    {#if archived === false}
      <div class="state-card">
        <span class="state-ico"><Icon name="cloudOff" size={22} /></span>
        <h3>Épisode non archivé</h3>
        <p>L'épisode doit d'abord être archivé sur le serveur.</p>
      </div>
    {:else if !admin}
      <div class="state-card">
        <span class="state-ico"><Icon name="transcript" size={22} /></span>
        <h3>Aucune transcription</h3>
        <p>Aucune transcription pour cet épisode.</p>
      </div>
    {:else if $transcribeSettings.state === "loading" && !canUseSettings}
      <div class="state-card" aria-busy="true">
        <p class="status" role="status">Vérification de la transcription…</p>
      </div>
    {:else if $transcribeSettings.state === "error" && !$transcribeSettings.supported}
      <div class="state-card">
        <span class="state-ico"><Icon name="transcript" size={22} /></span>
        <h3>Transcription indisponible</h3>
        <p>La transcription n'est pas disponible sur ce serveur.</p>
      </div>
    {:else if $transcribeSettings.state === "error" && !canUseSettings}
      <div class="state-card">
        <span class="state-ico"><Icon name="alert" size={22} /></span>
        <h3>Impossible de vérifier la transcription.</h3>
        <p>Le serveur n'a pas répondu. Rien n'a été lancé.</p>
        <button type="button" class="btn ghost" on:click={() => loadTranscribeSettings(true)}>
          <Icon name="refresh" size={16} /> Réessayer
        </button>
      </div>
    {:else if !canUseSettings}
      <div class="state-card" aria-busy="true">
        <p class="status" role="status">Vérification de la transcription…</p>
      </div>
    {:else if canUseSettings.available === false}
      <div class="state-card">
        <span class="state-ico"><Icon name="transcript" size={22} /></span>
        <h3>Transcription non configurée</h3>
        <p>{reasonText(canUseSettings.reason)}</p>
        <button type="button" class="btn ghost" on:click={openSettings}>
          <Icon name="settings" size={16} /> Ouvrir Réglages
        </button>
      </div>
    {:else}
      <div class="state-card">
        <span class="state-ico"><Icon name="transcript" size={22} /></span>
        <h3>Transcrire cet épisode</h3>
        <p>Le texte est produit sur ce serveur à partir du fichier archivé. Rien n'est envoyé à un tiers.</p>
        <p class="small">
          Un épisode d'une heure demande souvent plus d'une heure de calcul ; le serveur continue si
          vous fermez cet écran.
        </p>
        <button type="button" class="btn primary" on:click={startEpisode} disabled={actionBusy || retryLeft > 0}>
          <Icon name="transcript" size={16} />
          {actionBusy ? "Envoi…" : retryLeft > 0 ? `Réessayez dans ${retryLeft} s` : "Transcrire cet épisode"}
        </button>
        {#if actionError}<p class="alert" role="alert">{actionError}</p>{/if}
      </div>
    {/if}
  {:else if status === "queued"}
    <div class="state-card">
      <span class="state-ico"><Icon name="transcript" size={22} /></span>
      <h3>En attente de transcription…</h3>
      <p>La file avance ; le serveur continue si vous fermez cet écran.</p>
      <span class="bar indeterminate" role="progressbar" aria-label="Progression de la transcription">
        <span data-motion="status"></span>
      </span>
      {#if admin}
        <div class="state-actions">
          <button type="button" class="btn ghost" on:click={stopEpisode} disabled={actionBusy}>Arrêter</button>
        </div>
      {/if}
      {#if actionError}<p class="alert" role="alert">{actionError}</p>{/if}
    </div>
  {:else if status === "running" || status === "leased"}
    {#if !hasText}
      <div class="state-card">
        <span class="state-ico"><Icon name="transcript" size={22} /></span>
        <h3>Transcription en cours — {percent} %</h3>
        <p>{minutes(doneMs)} min transcrites sur {minutes(durationMs) || "?"}</p>
        <p class="small">Elle se met en pause pendant que vous écoutez.</p>
        <span
          class="bar"
          role="progressbar"
          aria-label="Progression de la transcription"
          aria-valuemin="0"
          aria-valuemax="100"
          aria-valuenow={percent}
        >
          <span style={`width:${percent}%`}></span>
        </span>
        {#if admin}
          <div class="state-actions">
            <button type="button" class="btn ghost" on:click={stopEpisode} disabled={actionBusy}>Arrêter</button>
          </div>
        {/if}
        {#if actionError}<p class="alert" role="alert">{actionError}</p>{/if}
      </div>
    {:else}
      <div class="banner live">
        <div class="banner-line">
          <strong>Transcription en cours — {percent} %</strong>
          <span class="small">{minutes(doneMs)} min sur {minutes(durationMs) || "?"}</span>
          {#if admin}
            <button type="button" class="btn tiny" on:click={stopEpisode} disabled={actionBusy}>Arrêter</button>
          {/if}
        </div>
        <span
          class="bar"
          role="progressbar"
          aria-label="Progression de la transcription"
          aria-valuemin="0"
          aria-valuemax="100"
          aria-valuenow={percent}
        >
          <span style={`width:${percent}%`}></span>
        </span>
        <p class="small">Elle se met en pause pendant que vous écoutez.</p>
      </div>
    {/if}
  {:else if status === "stopped"}
    {#if !hasText}
      <div class="state-card">
        <span class="state-ico"><Icon name="transcript" size={22} /></span>
        <h3>Transcription arrêtée à {percent} %.</h3>
        <p>Le texte déjà transcrit reste sur le serveur.</p>
        {#if admin}
          <button type="button" class="btn primary" on:click={resumeEpisode} disabled={actionBusy || retryLeft > 0}>
            {actionBusy ? "Envoi…" : retryLeft > 0 ? `Réessayez dans ${retryLeft} s` : "Reprendre"}
          </button>
        {/if}
        {#if actionError}<p class="alert" role="alert">{actionError}</p>{/if}
      </div>
    {:else}
      <div class="banner stopped">
        <div class="banner-line">
          <strong>Transcription arrêtée à {percent} %.</strong>
          {#if admin}
            <button type="button" class="btn tiny" on:click={resumeEpisode} disabled={actionBusy || retryLeft > 0}>
              {retryLeft > 0 ? `Réessayez dans ${retryLeft} s` : "Reprendre"}
            </button>
          {/if}
        </div>
      </div>
    {/if}
  {:else if status === "error"}
    <div class="banner error" role="alert">
      <div class="banner-line">
        <Icon name="alert" size={16} />
        <strong>{admin && errorMessage ? errorMessage : "La transcription s'est arrêtée avant la fin."}</strong>
        {#if admin}
          <button type="button" class="btn tiny" on:click={resumeEpisode} disabled={actionBusy || retryLeft > 0}>
            {retryLeft > 0 ? `Réessayez dans ${retryLeft} s` : "Réessayer"}
          </button>
        {/if}
      </div>
      <p class="small">Le texte déjà transcrit reste lisible.</p>
      {#if admin && errorCode && !errorMessage}<p class="small">{errorCode}</p>{/if}
      {#if actionError}<p class="alert" role="alert">{actionError}</p>{/if}
    </div>
  {:else if status === "ready" && !hasText && !loading}
    <div class="state-card">
      <span class="state-ico"><Icon name="transcript" size={22} /></span>
      <h3>Transcription vide</h3>
      <p>Aucune parole n'a été reconnue dans cet épisode.</p>
      {#if admin}
        <div class="state-actions">
          <button type="button" class="btn ghost" on:click={openSettings}>
            <Icon name="settings" size={16} /> Ouvrir Réglages
          </button>
        </div>
      {/if}
    </div>
  {:else if loading && !hasText}
    <div class="state-card" aria-busy="true">
      <p class="status" role="status">Chargement de la transcription…</p>
      <!-- P20: the job's own pulse is a status indicator — a frozen shimmer
           reads as a hang (M13), so eco keeps it. -->
      <span class="skeleton-line" data-motion="status"></span>
      <span class="skeleton-line short" data-motion="status"></span>
    </div>
  {/if}

  {#if hasText}
    {#if offline}
      <p class="offline-line">
        <Icon name="cloudOff" size={14} /> Hors ligne — dernière copie.
        <button type="button" class="offline-retry" on:click={refreshPage}>Réessayer</button>
      </p>
    {/if}
    {#if fromCache && loading}
      <p class="cache-line" role="status">Dernière copie affichée, mise à jour…</p>
    {/if}

    <div class="tools">
      <div class="searchbox">
        <Icon name="search" size={15} />
        <input
          type="search"
          bind:value={q}
          placeholder="Rechercher dans la transcription"
          aria-label="Rechercher dans la transcription"
          enterkeyhint="search"
          on:input={onSearchInput}
          on:keydown={onSearchKey}
        />
        {#if q}
          <button
            type="button"
            class="clear"
            aria-label="Effacer la recherche"
            title="Effacer la recherche"
            on:click={clearSearch}
          >
            <Icon name="close" size={14} />
          </button>
        {/if}
      </div>
      {#if status === "ready"}
        <div class="menu-wrap">
          <button
            type="button"
            class="btn ghost"
            aria-haspopup="menu"
            aria-expanded={exportOpen}
            title="Exporter la transcription"
            on:click={() => (exportOpen = !exportOpen)}
          >
            <Icon name="download" size={16} /> Exporter
          </button>
          {#if exportOpen}
            <div class="menu" role="menu" transition:pop={{ origin: "top right", placement: "down" }}>
              {#each EXPORT_FORMATS as entry (entry.id)}
                <button type="button" role="menuitem" on:click={() => exportTranscript(entry.id)}>
                  {entry.label}
                </button>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
      {#if admin && isTerminalStatus(status)}
        <div class="menu-wrap">
          <button
            type="button"
            class="icon-btn"
            aria-label="Options de la transcription"
            aria-expanded={menuOpen}
            title="Options de la transcription"
            on:click={() => (menuOpen = !menuOpen)}
          >
            <Icon name="more" size={18} />
          </button>
          {#if menuOpen}
            <div class="menu" role="menu" transition:pop={{ origin: "top right", placement: "down" }}>
              <button type="button" role="menuitem" on:click={() => { menu = "relaunch"; menuOpen = false; }}>
                Changer la langue et relancer…
              </button>
              <button type="button" role="menuitem" on:click={() => { menu = "delete"; menuOpen = false; }}>
                Supprimer la transcription
              </button>
            </div>
          {/if}
        </div>
      {/if}
    </div>

    {#if actionError}<p class="alert" role="alert">{actionError}</p>{/if}

    {#if menu === "relaunch"}
      <div class="confirm" role="group" aria-label="Relancer la transcription">
        <p>Relancer la transcription ? Le texte actuel sera remplacé.</p>
        <div class="langs">
          {#each LANGUAGES as code (code)}
            <button
              type="button"
              class="lang"
              class:on={relaunchLanguage === code}
              aria-pressed={relaunchLanguage === code}
              on:click={() => (relaunchLanguage = code)}
            >
              {languageLabel(code)}
            </button>
          {/each}
        </div>
        <div class="confirm-actions">
          <button type="button" class="btn primary" on:click={relaunch} disabled={actionBusy || retryLeft > 0}>
            {actionBusy ? "Envoi…" : retryLeft > 0 ? `Réessayez dans ${retryLeft} s` : "Relancer"}
          </button>
          <button type="button" class="btn ghost" on:click={() => (menu = null)}>Annuler</button>
        </div>
      </div>
    {:else if menu === "delete"}
      <div class="confirm" role="group" aria-label="Supprimer la transcription">
        <p>Supprimer la transcription ? Le texte sera perdu ; l'audio reste intact.</p>
        <div class="confirm-actions">
          <button type="button" class="btn danger" on:click={removeTranscript} disabled={actionBusy}>
            {actionBusy ? "Suppression…" : "Supprimer"}
          </button>
          <button type="button" class="btn ghost" on:click={() => (menu = null)}>Annuler</button>
        </div>
      </div>
    {/if}

    {#if searchMode}
      <div class="meta" role="status" aria-live="polite">
        {#if searchLoading}
          Recherche…
        {:else if searchError}
          La recherche dans cet épisode a échoué.
          <button type="button" class="link" on:click={() => runSearch(searchFor || q.trim(), true)}>Réessayer</button>
        {:else if searchHits.length}
          {#if searchTotal > searchHits.length}
            {searchHits.length} résultats sur {searchTotal}
          {:else}
            {searchTotal} résultat{searchTotal > 1 ? "s" : ""}
          {/if}
          <button type="button" class="link" on:click={clearSearch}>Effacer</button>
        {:else}
          Aucun résultat dans cet épisode.
          <button type="button" class="link" on:click={clearSearch}>Effacer</button>
        {/if}
        {#if searchLocal && !searchLoading}<span class="small">Recherche limitée à la page affichée.</span>{/if}
      </div>
      {#if searchLoading && !searchHits.length}
        <p class="status-line" role="status">Recherche dans l'épisode…</p>
      {:else if searchHits.length}
        <div class="hits" in:fadeIn>
          {#each searchHits as hit, i (i)}
            <button type="button" class="hit" on:click={() => pickHit(hit, i)}>
              <span class="sr">Lire à {fmtDuration(hit.start)}.</span>
              <time>{fmtDuration(hit.start)}</time>
              <span class="hit-text">
                {#each highlightParts(hit.text, searchFor) as part, k (k)}
                  {#if part.mark}<mark>{part.t}</mark>{:else}{part.t}{/if}
                {/each}
              </span>
            </button>
          {/each}
        </div>
      {/if}
    {:else}
      <div class="meta">
        {#if afterHit}
          Résultat {afterHit.index} sur {afterHit.total} ·
          <button type="button" class="link" on:click={backToResults}>Retour aux résultats</button>
        {:else if loading}
          <span class="small">Mise à jour…</span>
        {:else}
          <span class="small">{metaText(language)}</span>
        {/if}
      </div>
      {#if loadError && hasText}
        <p class="cache-line" role="status">
          Dernière copie — mise à jour impossible.
          <button type="button" class="link" on:click={refreshPage}>Réessayer</button>
        </p>
      {/if}
      {#if follow === false && isCurrent}
        <div class="rejoin-wrap">
          <button type="button" class="rejoin" on:click={rejoin}>
            <Icon name="play" size={14} /> Revenir à la lecture
          </button>
        </div>
      {/if}
      {#if !follow && !isCurrent}
        <div class="rejoin-wrap">
          <button type="button" class="rejoin" on:click={() => (follow = true)}>
            Suivre la lecture
          </button>
        </div>
      {/if}

      <div
        class="list"
        class:following={follow}
        bind:this={listEl}
        role="group"
        aria-label="Transcription de l'épisode"
        use:listEvents
        in:fadeIn
      >
        {#each segments as segment, i (i)}
          {@const mark = chapterMarks.get(i)}
          {#if mark}
            <div class="chapter-mark">
              <span class="chapter-time">{fmtDuration(mark.chapter.start)}</span>
              <span>{mark.chapter.title || `Chapitre ${mark.index + 1}`}</span>
            </div>
          {/if}
          <button
            type="button"
            class="seg"
            class:on={i === activeIdx}
            class:flash={flashStart != null && Number(segment.start) === Number(flashStart)}
            aria-current={i === activeIdx ? "true" : undefined}
            tabindex={i === tabIdx ? 0 : -1}
            bind:this={rowEls[i]}
            on:focus={() => onRowFocus(i)}
            on:click={() => seek(segment.start, i)}
          >
            <span class="sr">Lire à {fmtDuration(segment.start)}.</span>
            <time>{fmtDuration(segment.start)}</time>
            <span class="seg-text">{segment.text}</span>
          </button>
        {/each}
      </div>

      {#if total > limit}
        <div class="pager">
          <button
            type="button"
            class="btn ghost"
            disabled={info.page <= 1 || loading}
            on:click={() => goPage(-1)}
          >
            <Icon name="chevronLeft" size={15} /> Page précédente
          </button>
          <span class="page-label">Page {info.page} sur {info.pages}</span>
          <button
            type="button"
            class="btn ghost"
            disabled={info.page >= info.pages || loading}
            on:click={() => goPage(1)}
          >
            Page suivante <Icon name="chevronRight" size={15} />
          </button>
        </div>
      {/if}
    {/if}
  {/if}
</div>

<style>
  .tp {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
    color: var(--tx);
  }
  .tp.quiet {
    pointer-events: none;
  }
  .sr {
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
  .state-card {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
    padding: 18px 16px;
    border-radius: 16px;
    background: var(--s2);
    min-width: 0;
  }
  .state-ico {
    display: grid;
    place-items: center;
    width: 44px;
    height: 44px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--ac) 16%, var(--s1));
    color: var(--ac-text);
  }
  .state-card h3 {
    margin: 0;
    font-size: 1rem;
    font-weight: 700;
    line-height: 1.3;
  }
  .state-card p {
    margin: 0;
    color: var(--dim);
    font-size: 0.86rem;
    line-height: 1.5;
    overflow-wrap: anywhere;
  }
  .state-card p.small,
  .small {
    font-size: 0.78rem;
  }
  .state-actions {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    margin-top: 4px;
  }
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 7px;
    min-height: 44px;
    padding: 0 16px;
    border-radius: 999px;
    font-size: 0.88rem;
    font-weight: 650;
    color: var(--tx);
    border: 1px solid transparent;
    white-space: nowrap;
  }
  .btn.primary {
    background: var(--btn);
    color: var(--onbtn);
  }
  .btn.ghost {
    border-color: var(--ln);
    color: var(--ac-text);
  }
  .btn.ghost:hover {
    background: var(--s2);
  }
  .btn.danger {
    background: var(--accent-2, #e05252);
    color: var(--onac);
  }
  .btn.tiny {
    min-height: 44px;
    padding: 0 12px;
    border: 1px solid var(--ln);
    color: var(--ac-text);
    font-size: 0.8rem;
  }
  .btn:disabled {
    opacity: 0.55;
    cursor: default;
  }
  .btn:focus-visible,
  .icon-btn:focus-visible,
  .seg:focus-visible,
  .hit:focus-visible,
  .lang:focus-visible,
  .rejoin:focus-visible,
  .link:focus-visible {
    outline: 2px solid var(--ac-text);
    outline-offset: 2px;
  }
  .bar {
    display: block;
    width: 100%;
    height: 5px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--dim) 30%, transparent);
    overflow: hidden;
  }
  .bar > span {
    display: block;
    height: 100%;
    border-radius: 999px;
    background: var(--ac-mark);
    /* The job's progress is a continuous measurement, so its own duration is a
       token and its curve stays linear (I8/M15: no literal). */
    transition: width var(--dur-base) linear;
  }
  .bar.indeterminate > span {
    width: 35%;
    animation: slide 1.4s ease-in-out infinite;
  }
  @keyframes slide {
    0% {
      transform: translateX(-100%);
    }
    100% {
      transform: translateX(300%);
    }
  }
  .banner {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    border-radius: 12px;
    background: var(--s2);
    min-width: 0;
  }
  .banner.live {
    border-left: 3px solid var(--ac-mark);
  }
  .banner.stopped {
    border-left: 3px solid var(--dim);
  }
  .banner.error {
    border-left: 3px solid var(--accent-2, #e05252);
  }
  .banner-line {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    font-size: 0.84rem;
    line-height: 1.4;
  }
  .banner p {
    margin: 0;
    color: var(--dim);
  }
  .tools {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .searchbox {
    display: flex;
    align-items: center;
    gap: 7px;
    flex: 1;
    min-width: 0;
    height: 44px;
    padding: 0 12px;
    border: 1px solid var(--ln);
    border-radius: 999px;
    background: var(--s1);
    color: var(--dim);
  }
  .searchbox input {
    flex: 1;
    min-width: 0;
    height: 100%;
    border: 0;
    background: transparent;
    color: var(--tx);
    font-size: 0.86rem;
    text-overflow: ellipsis;
  }
  .searchbox input::placeholder {
    color: var(--dim);
    opacity: 1;
  }
  .searchbox input::-webkit-search-cancel-button {
    -webkit-appearance: none;
  }
  .searchbox input:focus-visible {
    outline: none;
  }
  .searchbox:focus-within {
    outline: 2px solid var(--ac-text);
    outline-offset: 2px;
  }
  .clear {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    flex: none;
    border-radius: 50%;
    color: var(--dim);
  }
  .clear:hover {
    color: var(--tx);
    background: var(--s2);
  }
  .icon-btn {
    display: grid;
    place-items: center;
    width: 44px;
    height: 44px;
    flex: none;
    border-radius: 50%;
    color: var(--tx);
    border: 1px solid var(--ln);
  }
  .icon-btn:hover {
    background: var(--s2);
  }
  .menu-wrap {
    position: relative;
    flex: none;
  }
  .menu {
    position: absolute;
    right: 0;
    top: calc(100% + 6px);
    z-index: 5;
    display: flex;
    flex-direction: column;
    min-width: 230px;
    padding: 4px;
    border-radius: 12px;
    border: 1px solid var(--ln);
    background: var(--s1);
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.35);
  }
  .menu button {
    min-height: 44px;
    padding: 0 12px;
    border-radius: 8px;
    text-align: left;
    color: var(--tx);
    font-size: 0.84rem;
  }
  .menu button:hover {
    background: var(--s2);
  }
  .confirm {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    border-radius: 12px;
    border: 1px solid var(--ln);
    background: var(--s1);
  }
  .confirm p {
    margin: 0;
    font-size: 0.86rem;
    line-height: 1.45;
  }
  .langs {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .lang {
    min-height: 44px;
    padding: 0 12px;
    border-radius: 999px;
    border: 1px solid var(--ln);
    color: var(--dim);
    font-size: 0.8rem;
    font-weight: 600;
  }
  .lang.on {
    background: var(--ac);
    color: var(--onac);
    border-color: var(--ac);
  }
  .confirm-actions {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }
  .meta {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    min-height: 18px;
    color: var(--dim);
    font-size: 0.78rem;
    line-height: 1.4;
  }
  .link {
    padding: 0;
    color: var(--ac-text);
    font-weight: 650;
    font-size: inherit;
  }
  .link:hover {
    text-decoration: underline;
  }
  .offline-line,
  .cache-line {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    margin: 0;
    color: var(--ac-text);
    font-size: 0.78rem;
    line-height: 1.4;
  }
  .cache-line {
    color: var(--dim);
  }
  .offline-retry {
    min-height: 44px;
    padding: 0 12px;
    border-radius: 999px;
    border: 1px solid var(--ln);
    color: var(--ac-text);
    font-size: 0.78rem;
    font-weight: 600;
  }
  .offline-retry:focus-visible {
    outline: 2px solid var(--ac-text);
    outline-offset: 2px;
  }
  .status-line,
  .status {
    margin: 0;
    color: var(--dim);
    font-size: 0.84rem;
  }
  .skeleton-line {
    display: block;
    width: 100%;
    height: 12px;
    margin-top: 8px;
    border-radius: 6px;
    background: color-mix(in srgb, var(--dim) 18%, transparent);
    animation: pulse 1.4s ease-in-out infinite;
  }
  .skeleton-line.short {
    width: 64%;
  }
  @keyframes pulse {
    50% {
      opacity: 0.45;
    }
  }
  .list {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .chapter-mark {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 14px 4px 4px;
    color: var(--dim);
    font-family: var(--font-mono);
    font-size: 10.5px;
    letter-spacing: 0.04em;
    border-top: 1px solid var(--ln);
    margin-top: 8px;
    min-width: 0;
  }
  .chapter-mark span:last-child {
    font-family: var(--font-body);
    font-size: 0.82rem;
    font-weight: 650;
    color: var(--tx);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .seg {
    display: grid;
    grid-template-columns: 48px minmax(0, 1fr);
    gap: 10px;
    align-items: start;
    width: 100%;
    min-height: 44px;
    padding: 8px 8px 8px 6px;
    border-radius: 10px;
    text-align: left;
    color: var(--tx);
    border-left: 3px solid transparent;
  }
  .seg:hover {
    background: var(--s2);
  }
  .seg.on {
    background: color-mix(in srgb, var(--ac) 16%, var(--s1));
    border-left-color: var(--ac-mark);
  }
  .seg.on time {
    color: var(--ac-text);
    font-weight: 600;
  }
  .seg.flash {
    animation: flash 1.6s ease-out;
  }
  @keyframes flash {
    0% {
      background: color-mix(in srgb, var(--ac) 34%, var(--s1));
    }
    100% {
      background: color-mix(in srgb, var(--ac) 16%, var(--s1));
    }
  }
  .seg time,
  .hit time {
    padding-top: 1px;
    flex: none;
    color: var(--dim);
    font-family: var(--font-mono);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    line-height: 1.55;
    white-space: nowrap;
  }
  .seg-text {
    min-width: 0;
    font-size: 0.88rem;
    line-height: 1.55;
    overflow-wrap: anywhere;
  }
  .hits {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .hit {
    display: grid;
    grid-template-columns: 48px minmax(0, 1fr);
    gap: 10px;
    align-items: start;
    width: 100%;
    min-height: 52px;
    padding: 8px;
    border-radius: 10px;
    text-align: left;
    color: var(--tx);
  }
  .hit:hover {
    background: var(--s2);
  }
  .hit-text {
    min-width: 0;
    font-size: 0.84rem;
    line-height: 1.5;
    overflow-wrap: anywhere;
  }
  mark {
    background: var(--ac);
    color: var(--onac);
    border-radius: 2px;
    padding: 0 1px;
  }
  .rejoin-wrap {
    position: sticky;
    top: 0;
    z-index: 4;
    display: flex;
    justify-content: center;
    pointer-events: none;
  }
  .rejoin {
    pointer-events: auto;
    display: inline-flex;
    align-items: center;
    gap: 7px;
    min-height: 44px;
    padding: 0 16px;
    border-radius: 999px;
    background: var(--btn);
    color: var(--onbtn);
    font-size: 0.84rem;
    font-weight: 700;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.35);
  }
  .pager {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    flex-wrap: wrap;
    padding: 8px 0;
  }
  .page-label {
    color: var(--dim);
    font-family: var(--font-mono);
    font-size: 10.5px;
    letter-spacing: 0.04em;
    white-space: nowrap;
  }
  .pager .btn {
    min-height: 44px;
    padding: 0 12px;
    font-size: 0.8rem;
  }
  .alert {
    margin: 0;
    padding: 8px 10px;
    border-radius: 8px;
    background: color-mix(in srgb, var(--accent-2, #e05252) 12%, transparent);
    color: var(--tx);
    font-size: 0.82rem;
    line-height: 1.4;
  }
  @media (pointer: coarse) {
    .clear {
      width: 44px;
      height: 44px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .bar > span,
    .bar.indeterminate > span,
    .skeleton-line {
      animation: none;
      transition: none;
    }
    .seg.flash {
      animation: none;
    }
  }
</style>
