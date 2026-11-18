package fr.neezer.mobile.feature.podcasts

import fr.neezer.mobile.bridge.NzErrors
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.bridge.screenFlow
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import uniffi.nz_ffi.ChaptersData
import uniffi.nz_ffi.EpisodeData
import uniffi.nz_ffi.EpisodeMarkerData
import uniffi.nz_ffi.ListHandle
import uniffi.nz_ffi.ShowData
import uniffi.nz_ffi.TranscribeSettingsData
import uniffi.nz_ffi.TranscriptSearchData

/**
 * One show: its header, its episodes paged by cursor, the archive switch, the
 * transcription card, the search inside the transcripts, and the chapters and
 * markers of the rows (`routes/Show.svelte` ported, lot MB13).
 *
 * Three things this model owns that the SPA's screen does in its own script, and
 * that are the acceptance's rows:
 *
 * - **the 2 000-episode list** — the page opens with 50 rows and one request,
 *   and each further page is one request of 50 when the viewport reaches the
 *   end ([onVisible]); the rows live in Rust behind a `ListHandle`, so a screen
 *   never holds 2 000 of them;
 * - **the resume position** — read from the device's own map, so an episode
 *   resumes where it stopped with the link down (0 requests);
 * - **the transcription job** — started optimistically, then polled until it
 *   settles (`pollTranscript`), and the archive switch polled while its count
 *   moves (`watchArchive`, the SPA's 6 s and its five-stall stop).
 */
class ShowModel(
    private val service: PodcastService,
    private val player: PodcastsPlayer,
    private val scope: CoroutineScope,
    /** The SPA's debounce for the show search (`SEARCH_DELAY`). */
    private val debounceMs: Long = 300,
    /** The archive poll's period (`POLL_MS`). Tests drive `pollArchive` directly. */
    private val archivePollMs: Long = 6_000,
    /** The transcript poll's period (`TRANSCRIPT_POLL_MS`). */
    private val transcriptPollMs: Long = 10_000,
) {
    /** Everything the show page draws. */
    data class State(
        val id: String = "",
        val show: ShowData? = null,
        val stale: Boolean = false,
        val loading: Boolean = true,
        val failure: String? = null,
        /** The windowed handle over the loaded episodes, `null` before the load. */
        val handle: ListHandle? = null,
        /** How many episodes the device holds for this show. */
        val loaded: Int = 0,
        /** The cursor of the next page, `null` on the last one. */
        val next: String? = null,
        /** The drawn rows of the visible window, by absolute index. */
        val cards: Map<Int, EpisodeData> = emptyMap(),
        val loadingMore: Boolean = false,
        val moreFailure: String? = null,
        /** The archive switch is in flight. */
        val archiveBusy: Boolean = false,
        /** The server is still archiving: the line says « Archivage en cours ». */
        val archiving: Boolean = false,
        val transcribeBusy: Boolean = false,
        val transcribeSettings: TranscribeSettingsData? = null,
        val transcribeFailure: String? = null,
        /** `false` when the server has no transcription routes at all (rule 7). */
        val transcribeSupported: Boolean = true,
        /** `all`, `recent` or `none` — the catch-up the switch will ask for. */
        val backlog: String = "all",
        /** How many episodes « Transcrire » will queue. */
        val batchCount: Int = 5,
        val searchQuery: String = "",
        val search: TranscriptSearchData? = null,
        val searchBusy: Boolean = false,
        val searchFailure: String? = null,
        val searchFor: String = "",
        val chapters: Map<String, ChaptersData> = emptyMap(),
        val chapterBusy: Set<String> = emptySet(),
        val chapterFailure: Map<String, String> = emptyMap(),
        val markers: Map<String, List<EpisodeMarkerData>> = emptyMap(),
        /** The sheet's episode id, when the chapter sheet is open. */
        val sheet: String? = null,
        val message: String? = null,
        val version: Long = 0,
    ) {
        /** The SPA's `readyCount`: transcripts of the loaded page that are ready. */
        val readyCount: Int
            get() = cards.values.count { it.transcript?.ready == true }

        /** The SPA's `archivedEpisodes`: the loaded rows that are on the server. */
        val archivedCount: Int
            get() = cards.values.count { it.archived }

        /**
         * The SPA's `eligible`: archived episodes with no usable transcript —
         * what « Transcrire » queues.
         */
        val eligible: List<EpisodeData>
            get() = cards.toSortedMap().values
                .filter { episode ->
                    episode.archived &&
                        (episode.transcript == null ||
                            episode.transcript?.status in setOf("none", "error", "stopped"))
                }
                .sortedByDescending { it.published }

        /** The transcription running or leased right now, if any. */
        val activeTranscript: EpisodeData?
            get() = cards.toSortedMap().values.firstOrNull {
                val status = it.transcript?.status
                status == "running" || status == "leased"
            }

        val queuedCount: Int
            get() = cards.values.count { it.transcript?.status == "queued" }

        /** « Transcrire » offers 5, 10 or 20 (`batchOptions`). */
        val batchChoices: List<Int>
            get() = PodcastsCopy.batchChoices(eligible.size)

        /** « 1,2 h d'audio » — the estimate above the batch button. */
        val audioSeconds: Long
            get() = eligible.sumOf { it.durationS.toLong() }

        /** The search panel exists once one transcript is ready (the SPA's rule). */
        val showSearch: Boolean
            get() = searchBusy || searchQuery.isNotEmpty() || cards.values.any { it.transcript?.ready == true }

        val isSearching: Boolean get() = searchQuery.trim().isNotEmpty()
    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    private var loadJob: Job? = null
    private var moreJob: Job? = null
    private var searchJob: Job? = null
    private var archiveJob: Job? = null
    private var transcribeWatchJob: Job? = null
    private var stalled = 0

    /** Opens the show: the copy first, then the first page (2 requests). */
    fun load(id: String) {
        if (_state.value.id == id && loadJob?.isActive == true) return
        loadJob?.cancel()
        archiveJob?.cancel()
        transcribeWatchJob?.cancel()
        _state.value = State(id = id)
        loadJob = scope.launch {
            screenFlow(
                cached = { service.cachedShow(id) },
                fresh = { service.show(id) },
            ).collect { value ->
                when (value) {
                    is ScreenState.Loading -> _state.update { it.copy(loading = true) }
                    is ScreenState.Ready -> apply(value.data, stale = value.stale)
                    is ScreenState.Failed -> _state.update {
                        it.copy(
                            loading = false,
                            failure = if (it.show == null) value.cause else null,
                            stale = it.show != null,
                            version = it.version + 1,
                        )
                    }
                }
            }
        }
    }

    fun retry() {
        val id = _state.value.id
        if (id.isNotEmpty()) load(id)
    }

    /** The page (or a further page) landed. */
    private fun apply(page: uniffi.nz_ffi.ShowPageData, stale: Boolean) {
        _state.update { current ->
            current.copy(
                show = page.show,
                handle = page.episodes,
                loaded = page.loaded.toInt(),
                next = page.next,
                loading = false,
                failure = null,
                stale = stale,
                loadingMore = false,
                moreFailure = null,
                version = current.version + 1,
            )
        }
        if (!stale) startWatching()
    }

    /** The SPA's `pollArchive`/`watchTranscript` start: a job still running. */
    private fun startWatching() {
        val state = _state.value
        if (state.transcribeSettings == null) loadTranscribeSettings()
        if (archiveJob?.isActive != true && state.archiving) {
            archiveJob = scope.launch { pollArchiveLoop() }
        }
    }

    // -- the windowed list ---------------------------------------------------

    /**
     * The viewport reported rows `first..last` (and whether it is still
     * scrolling).
     *
     * Two jobs: fetch the CARDS of the rows about to be drawn (one FFI call, no
     * request, only for rows not held yet), and ask the NEXT cursor once the
     * viewport is within a page of the end (`loadMoreEpisodes`'s job, one
     * request of 50 rows).
     */
    fun onVisible(first: Int, last: Int, scrolling: Boolean) {
        val state = _state.value
        if (state.loaded == 0) return
        val start = first.coerceAtLeast(0)
        val end = last.coerceIn(start, state.loaded - 1)
        val missing = (start..end).any { state.cards[it] == null }
        if (missing) {
            scope.launch {
                val cards = service.episodeCards(state.id, start, end - start + 1)
                if (cards.isEmpty()) return@launch
                val markers = markersOf(cards)
                _state.update { current ->
                    val next = HashMap(current.cards)
                    cards.forEachIndexed { offset, card ->
                        val at = start + offset
                        next[at] = card.copy(
                            markers = markers[card.id] ?: card.markers,
                        )
                    }
                    current.copy(cards = next, markers = current.markers + markers, version = current.version + 1)
                }
            }
        }
        // The next page: one request per 50 rows, and only when the finger is
        // really reading the end of the list (never during a fling).
        if (!scrolling && end >= state.loaded - LIST_AHEAD && state.next != null) {
            loadMore()
        }
    }

    /** `loadMoreEpisodes`: append the next cursor's page (one request). */
    fun loadMore() {
        val state = _state.value
        if (state.next == null || moreJob?.isActive == true) return
        _state.update { it.copy(loadingMore = true, moreFailure = null) }
        moreJob = scope.launch {
            try {
                val page = service.showMore(state.id)
                apply(page, stale = false)
            } catch (error: Throwable) {
                _state.update {
                    it.copy(
                        loadingMore = false,
                        moreFailure = NzErrors.text(error),
                        version = it.version + 1,
                    )
                }
            }
        }
    }

    // -- the archive switch --------------------------------------------------

    /**
     * `toggleAuto`: the switch moves FIRST, then the request; a refusal puts it
     * back and says why. What it queued is toasted (rule 2: a job says what it
     * did), and the count is polled while it moves.
     */
    fun toggleAuto() {
        val state = _state.value
        val show = state.show ?: return
        if (state.archiveBusy) return
        val on = !show.autoArchive
        _state.update {
            it.copy(
                show = show.copy(autoArchive = on),
                archiveBusy = true,
                message = null,
                version = it.version + 1,
            )
        }
        scope.launch {
            try {
                val answer = service.autoArchive(state.id, on)
                _state.update {
                    it.copy(
                        show = it.show?.copy(
                            autoArchive = answer.show.autoArchive,
                            archivedCount = answer.show.archivedCount,
                        ) ?: answer.show,
                        archiveBusy = false,
                        message = if (on && answer.queued > 0u) {
                            PodcastsCopy.archiveQueued(answer.queued.toInt())
                        } else {
                            null
                        },
                        version = it.version + 1,
                    )
                }
                if (on && answer.queued > 0u) {
                    stalled = 0
                    _state.update { it.copy(archiving = true) }
                    archiveJob?.cancel()
                    archiveJob = scope.launch { pollArchiveLoop() }
                }
            } catch (error: Throwable) {
                _state.update {
                    it.copy(
                        show = it.show?.copy(autoArchive = show.autoArchive),
                        archiveBusy = false,
                        message = NzErrors.text(error),
                        version = it.version + 1,
                    )
                }
            }
        }
    }

    /**
     * One archive poll: re-read the show, and stop as soon as it is complete,
     * the switch is off, or five polls moved nothing (`STALL_POLLS`).
     */
    suspend fun pollArchiveOnce() {
        val state = _state.value
        val show = state.show ?: return
        if (!state.archiving || !show.autoArchive) {
            _state.update { it.copy(archiving = false) }
            return
        }
        val before = show.archivedCount
        val fresh = runCatching { service.show(state.id) }.getOrNull()
        if (fresh != null) {
            val reachable = reachable(fresh.show, state.cards)
            val moved = fresh.show.archivedCount > before
            stalled = if (moved) 0 else stalled + 1
            _state.update {
                it.copy(
                    show = fresh.show,
                    archiving = fresh.show.archivedCount.toInt() < reachable && stalled < STALL_POLLS,
                    version = it.version + 1,
                )
            }
        } else {
            stalled += 1
            if (stalled >= STALL_POLLS) _state.update { it.copy(archiving = false) }
        }
    }

    private suspend fun pollArchiveLoop() {
        while (scope.isActive && _state.value.archiving) {
            delay(archivePollMs)
            pollArchiveOnce()
        }
    }

    /** Episodes the provider still serves: what « complete » is measured against. */
    private fun reachable(show: ShowData, cards: Map<Int, EpisodeData>): Int {
        val gone = cards.values.count { it.unavailable && !it.archived }
        return (show.episodeCount.toInt() - gone).coerceAtLeast(0)
    }

    // -- the transcription card ---------------------------------------------

    /** `loadTranscribeSettings`: once per show, and « Réessayer » re-probes. */
    fun loadTranscribeSettings(force: Boolean = false) {
        val state = _state.value
        if (!force && state.transcribeSettings != null) return
        _state.update { it.copy(transcribeFailure = null) }
        scope.launch {
            try {
                val settings = service.transcribeSettings(probe = force)
                _state.update {
                    it.copy(
                        transcribeSettings = settings,
                        transcribeSupported = true,
                        transcribeFailure = null,
                        version = it.version + 1,
                    )
                }
            } catch (error: Throwable) {
                val supported = service.transcriptionSupported()
                _state.update {
                    it.copy(
                        transcribeFailure = NzErrors.text(error),
                        transcribeSupported = supported ?: true,
                        version = it.version + 1,
                    )
                }
            }
        }
    }

    fun setBacklog(mode: String) {
        _state.update { it.copy(backlog = mode, version = it.version + 1) }
    }

    fun setBatchCount(count: Int) {
        _state.update { it.copy(batchCount = count, version = it.version + 1) }
    }

    /** `toggleShowTranscribe`: the switch first, the whole backlog it queued. */
    fun toggleShowTranscribe() {
        val state = _state.value
        val show = state.show ?: return
        if (state.transcribeBusy || state.transcribeSettings == null) return
        val on = !show.autoTranscribe
        _state.update {
            it.copy(
                show = show.copy(autoTranscribe = on),
                transcribeBusy = true,
                message = null,
                version = it.version + 1,
            )
        }
        scope.launch {
            try {
                val answer = service.setShowTranscribe(
                    state.id,
                    auto = on,
                    backlog = if (on) state.backlog else null,
                    language = null,
                )
                val queued = answer.queued.toInt()
                val skipped = answer.skipped.toInt()
                _state.update {
                    it.copy(
                        show = answer.show ?: it.show,
                        transcribeBusy = false,
                        message = when {
                            on && queued > 0 -> PodcastsCopy.transcribeQueued(queued)
                            on && skipped > 0 -> PodcastsCopy.transcribeSkipped(skipped, null)
                            on -> "Aucun épisode à transcrire pour le moment."
                            else -> null
                        },
                        version = it.version + 1,
                    )
                }
                // The rows' chips reflect the queue: re-read the page so a
                // « En attente » appears where the server accepted one.
                if (on && queued > 0) refreshRows()
            } catch (error: Throwable) {
                _state.update {
                    it.copy(
                        show = it.show?.copy(autoTranscribe = show.autoTranscribe),
                        transcribeBusy = false,
                        message = NzErrors.text(error),
                        version = it.version + 1,
                    )
                }
            }
        }
    }

    /** The show's language (`setShowLanguage`): optimistic, rolled back. */
    fun setLanguage(code: String) {
        val state = _state.value
        val show = state.show ?: return
        val previous = show.transcribeLanguage
        _state.update {
            it.copy(show = show.copy(transcribeLanguage = code), version = it.version + 1)
        }
        scope.launch {
            try {
                val answer = service.setShowTranscribe(
                    state.id,
                    auto = null,
                    backlog = null,
                    language = code,
                )
                _state.update {
                    it.copy(show = answer.show ?: it.show, version = it.version + 1)
                }
            } catch (error: Throwable) {
                _state.update {
                    it.copy(
                        show = it.show?.copy(transcribeLanguage = previous),
                        message = NzErrors.text(error),
                        version = it.version + 1,
                    )
                }
            }
        }
    }

    /** `startOne`: one episode queued, the chip optimistic. */
    fun startOne(episodeId: String) {
        val before = _state.value.cards.values.firstOrNull { it.id == episodeId }?.transcript
        _state.update { setTranscript(it, episodeId, QUEUED) }
        scope.launch {
            try {
                service.startTranscript(episodeId, language = null, force = false)
                refreshEpisode(episodeId)
            } catch (error: Throwable) {
                _state.update { setTranscript(it, episodeId, before) }
                _state.update {
                    it.copy(message = NzErrors.text(error), version = it.version + 1)
                }
            }
        }
    }

    /**
     * `startBatch`: the newest eligible episodes, ONE request each, in order.
     * A full queue (429) stops the batch — the SPA's rule — and the count that
     * was accepted is what the toast says.
     */
    fun startBatch() {
        val state = _state.value
        if (state.transcribeBusy || state.transcribeSettings == null) return
        val wanted = state.eligible.take(state.batchCount)
        if (wanted.isEmpty()) return
        val previous = wanted.associate { it.id to it.transcript }
        _state.update { current ->
            var next = current.copy(transcribeBusy = true, message = null)
            wanted.forEach { next = setTranscript(next, it.id, QUEUED) }
            next.copy(version = next.version + 1)
        }
        scope.launch {
            var accepted = 0
            var skipped = 0
            var cause: String? = null
            for (episode in wanted) {
                try {
                    service.startTranscript(episode.id, language = null, force = false)
                    accepted += 1
                } catch (error: Throwable) {
                    val already = error is uniffi.nz_ffi.NzException.Conflict &&
                        error.detail.contains("already_running")
                    if (already) {
                        accepted += 1
                        continue
                    }
                    skipped += 1
                    cause = cause ?: NzErrors.text(error)
                    _state.update { setTranscript(it, episode.id, previous[episode.id]) }
                    val full = error is uniffi.nz_ffi.NzException.NotReady ||
                        (error is uniffi.nz_ffi.NzException.Server && error.status.toInt() == 429)
                    if (full) break
                }
            }
            _state.update {
                it.copy(
                    transcribeBusy = false,
                    message = when {
                        accepted > 0 && skipped > 0 ->
                            "${PodcastsCopy.transcribeQueued(accepted)} · " +
                                PodcastsCopy.transcribeSkipped(skipped, cause)
                        accepted > 0 -> PodcastsCopy.transcribeQueued(accepted)
                        skipped > 0 -> PodcastsCopy.transcribeSkipped(skipped, cause)
                        else -> "Aucun épisode à transcrire."
                    },
                    version = it.version + 1,
                )
            }
            if (accepted > 0) refreshRows()
        }
    }

    /** `stopTranscript`: the server keeps what it wrote (`done_ms`). */
    fun stopTranscript(episodeId: String) {
        scope.launch {
            try {
                service.stopTranscript(episodeId)
                _state.update { setTranscript(it, episodeId, STOPPED) }
                refreshEpisode(episodeId)
            } catch (error: Throwable) {
                _state.update {
                    it.copy(message = NzErrors.text(error), version = it.version + 1)
                }
            }
        }
    }

    /** A forced restart, for the panel's menu (`force: true`, a language). */
    fun restartTranscript(episodeId: String, language: String) {
        scope.launch {
            try {
                service.startTranscript(episodeId, language, force = true)
                _state.update { setTranscript(it, episodeId, QUEUED) }
            } catch (error: Throwable) {
                _state.update {
                    it.copy(message = NzErrors.text(error), version = it.version + 1)
                }
            }
        }
    }

    /** `removeTranscript`: the server's text and this device's pages. */
    fun deleteTranscript(episodeId: String) {
        scope.launch {
            try {
                service.deleteTranscript(episodeId)
                _state.update { setTranscript(it, episodeId, null) }
            } catch (error: Throwable) {
                _state.update {
                    it.copy(message = NzErrors.text(error), version = it.version + 1)
                }
            }
        }
    }

    /**
     * The panel's `poll`: one `limit=1` read that only asks for the STATE of a
     * transcription in progress. Called by the panel while it is visible.
     */
    suspend fun pollTranscript(episodeId: String) {
        val page = runCatching {
            service.transcript(episodeId, limit = 1, offset = 0, at = null, q = null)
        }.getOrNull() ?: return
        _state.update { current ->
            val card = current.cards.values.firstOrNull { it.id == episodeId }
            if (card == null) {
                current
            } else {
                val info = card.transcript?.copy(
                    status = page.status,
                    progress = page.progress,
                    percent = page.percent,
                    language = page.language,
                ) ?: uniffi.nz_ffi.TranscriptInfoData(
                    status = page.status,
                    progress = page.progress,
                    percent = page.percent,
                    language = page.language,
                    source = null,
                    segmentCount = page.total,
                    errorCode = page.errorCode,
                    active = page.active,
                    ready = page.status == "ready",
                )
                current.copy(
                    cards = current.cards + (indexOf(current, episodeId) to card.copy(transcript = info)),
                    version = current.version + 1,
                )
            }
        }
    }

    /** Starts the panel's poll loop, once the model is watching a job. */
    fun watchTranscript(episodeId: String) {
        transcribeWatchJob?.cancel()
        transcribeWatchJob = scope.launch {
            while (scope.isActive) {
                val card = _state.value.cards.values.firstOrNull { it.id == episodeId } ?: break
                val status = card.transcript?.status
                if (status == "queued" || status == "running" || status == "leased") {
                    delay(transcriptPollMs)
                    pollTranscript(episodeId)
                } else {
                    break
                }
            }
        }
    }

    // -- the search inside the transcripts -----------------------------------

    fun setSearchQuery(query: String) {
        searchJob?.cancel()
        val term = query.trim()
        _state.update {
            it.copy(
                searchQuery = query,
                searchFailure = null,
                search = if (term.length < PodcastsModel.MIN_CHARACTERS) null else it.search,
                searchBusy = term.length >= PodcastsModel.MIN_CHARACTERS,
                version = it.version + 1,
            )
        }
        if (term.length < PodcastsModel.MIN_CHARACTERS) return
        searchJob = scope.launch {
            delay(debounceMs)
            runSearch(term)
        }
    }

    fun retrySearch() {
        val term = _state.value.searchQuery.trim()
        if (term.length < PodcastsModel.MIN_CHARACTERS) return
        searchJob?.cancel()
        searchJob = scope.launch { runSearch(term) }
    }

    fun clearSearch() {
        searchJob?.cancel()
        _state.update {
            it.copy(
                searchQuery = "",
                search = null,
                searchBusy = false,
                searchFailure = null,
                searchFor = "",
                version = it.version + 1,
            )
        }
    }

    private suspend fun runSearch(term: String) {
        _state.update { it.copy(searchBusy = true, searchFailure = null, version = it.version + 1) }
        try {
            val answer = service.showSearch(_state.value.id, term, SHOW_SEARCH_LIMIT)
            if (_state.value.searchQuery.trim() != term) return
            _state.update {
                it.copy(
                    search = answer,
                    searchBusy = false,
                    searchFor = term,
                    searchFailure = null,
                    version = it.version + 1,
                )
            }
        } catch (error: Throwable) {
            if (_state.value.searchQuery.trim() != term) return
            _state.update {
                it.copy(
                    search = null,
                    searchBusy = false,
                    searchFor = term,
                    searchFailure = NzErrors.text(error),
                    version = it.version + 1,
                )
            }
        }
    }

    // -- chapters ------------------------------------------------------------

    /**
     * `toggleChapters`/`loadChapters`: the copy first (0 requests), then one
     * request; a failure keeps the copy with « Hors ligne — dernière copie ».
     */
    fun loadChapters(episodeId: String, force: Boolean = false) {
        val state = _state.value
        if (!force && state.cachedChapter(episodeId) != null) return
        if (state.chapterBusy.contains(episodeId)) return
        _state.update {
            it.copy(
                chapterBusy = it.chapterBusy + episodeId,
                chapterFailure = it.chapterFailure - episodeId,
                version = it.version + 1,
            )
        }
        scope.launch {
            val cached = runCatching { service.cachedChapters(episodeId) }.getOrNull()
            if (cached != null) {
                _state.update {
                    it.copy(chapters = it.chapters + (episodeId to cached), version = it.version + 1)
                }
            }
            try {
                val fresh = service.chapters(episodeId)
                _state.update {
                    it.copy(
                        chapters = it.chapters + (episodeId to fresh),
                        chapterBusy = it.chapterBusy - episodeId,
                        version = it.version + 1,
                    )
                }
            } catch (error: Throwable) {
                _state.update {
                    it.copy(
                        chapterBusy = it.chapterBusy - episodeId,
                        chapterFailure = it.chapterFailure + (episodeId to NzErrors.text(error)),
                        version = it.version + 1,
                    )
                }
            }
        }
    }

    fun openSheet(episodeId: String) {
        loadChapters(episodeId)
        _state.update { it.copy(sheet = episodeId, version = it.version + 1) }
    }

    fun closeSheet() {
        _state.update { it.copy(sheet = null, version = it.version + 1) }
    }

    // -- markers -------------------------------------------------------------

    /** `addMarkerAt`: the chip appears first; a refusal removes it and says why. */
    fun addMarker(episodeId: String, seconds: Double, label: String? = null) {
        val positionMs = (seconds.coerceAtLeast(0.0) * 1000).toInt()
        scope.launch {
            try {
                val marker = service.addMarker(episodeId, positionMs, label)
                _state.update { current ->
                    val list = (current.markers[episodeId] ?: current.cards.values
                        .firstOrNull { it.id == episodeId }?.markers ?: emptyList()) + marker
                    current.copy(
                        markers = current.markers + (episodeId to list.sortedBy { it.positionMs }),
                        cards = current.cards.mapValues { (_, card) ->
                            if (card.id == episodeId) {
                                card.copy(markers = list.sortedBy { it.positionMs })
                            } else {
                                card
                            }
                        },
                        message = "Marqueur ajouté à ${PodcastsCopy.duration(marker.seconds.toInt())}",
                        version = current.version + 1,
                    )
                }
            } catch (error: Throwable) {
                _state.update {
                    it.copy(message = NzErrors.text(error), version = it.version + 1)
                }
            }
        }
    }

    /** `removeMarker`: the chip goes first and comes back on a refusal. */
    fun removeMarker(marker: EpisodeMarkerData) {
        val previous = _state.value.markers[marker.episodeId]
        _state.update { current ->
            val list = (current.markers[marker.episodeId] ?: emptyList()).filterNot { it.id == marker.id }
            current.copy(
                markers = current.markers + (marker.episodeId to list),
                cards = current.cards.mapValues { (_, card) ->
                    if (card.id == marker.episodeId) {
                        card.copy(markers = list)
                    } else {
                        card
                    }
                },
                version = current.version + 1,
            )
        }
        scope.launch {
            try {
                service.removeMarker(marker.id)
                _state.update {
                    it.copy(message = "Marqueur supprimé", version = it.version + 1)
                }
            } catch (error: Throwable) {
                _state.update { current ->
                    current.copy(
                        markers = previous?.let { list -> current.markers + (marker.episodeId to list) }
                            ?: current.markers,
                        message = NzErrors.text(error),
                        version = current.version + 1,
                    )
                }
            }
        }
    }

    // -- playing -------------------------------------------------------------

    /**
     * A tap on an episode's play button (`Show.svelte#toggle`/`#playFrom`): the
     * loaded page becomes the queue, starting at [index]; the same episode
     * toggles instead of restarting.
     */
    fun play(index: Int) {
        val state = _state.value
        val card = state.cards[index] ?: return
        val snapshot = player.snapshot()
        val current = snapshot?.current
        if (current != null && current.key == card.id) {
            player.toggle()
            return
        }
        scope.launch {
            val episodes = service.episodeCards(state.id, 0, state.loaded.coerceAtLeast(index + 1))
            if (episodes.isEmpty()) return@launch
            player.play(episodes.map { it.row }, index, state.id)
        }
    }

    /** « Lire »: the whole loaded page, from the top. */
    fun playAll() {
        play(0)
    }

    /** A marker tap or a resume: seek (or start there and seek). */
    fun playAt(episodeId: String, seconds: Double) {
        val state = _state.value
        val index = indexOf(state, episodeId)
        if (index < 0) return
        val card = state.cards[index] ?: return
        val snapshot = player.snapshot()
        if (snapshot?.current?.key == card.id) {
            player.seek(seconds)
            return
        }
        scope.launch {
            val episodes = service.episodeCards(state.id, 0, state.loaded.coerceAtLeast(index + 1))
            if (episodes.isEmpty()) return@launch
            player.play(episodes.map { it.row }, index, state.id)
            if (seconds > 0) player.seek(seconds)
        }
    }

    // -- the messages --------------------------------------------------------

    fun consumeMessage() {
        _state.update { it.copy(message = null) }
    }

    fun stop() {
        loadJob?.cancel()
        moreJob?.cancel()
        searchJob?.cancel()
        archiveJob?.cancel()
        transcribeWatchJob?.cancel()
    }

    // -- helpers -------------------------------------------------------------

    private fun State.cachedChapter(episodeId: String): ChaptersData? = chapters[episodeId]

    private fun indexOf(state: State, episodeId: String): Int =
        state.cards.entries.firstOrNull { it.value.id == episodeId }?.key ?: -1

    private suspend fun refreshRows() {
        val state = _state.value
        if (state.loaded == 0) return
        val cards = runCatching {
            service.episodeCards(state.id, 0, minOf(state.loaded, ROWS_TO_REFRESH))
        }.getOrNull() ?: return
        _state.update { current ->
            val next = HashMap(current.cards)
            cards.forEachIndexed { at, card -> next[at] = card }
            current.copy(cards = next, version = current.version + 1)
        }
    }

    private suspend fun refreshEpisode(episodeId: String) {
        val state = _state.value
        val index = indexOf(state, episodeId)
        if (index < 0) return
        val card = runCatching { service.episodeCards(state.id, index, 1) }.getOrNull()?.firstOrNull()
            ?: return
        _state.update {
            it.copy(cards = it.cards + (index to card), version = it.version + 1)
        }
    }

    /** The markers the store holds for these rows (disk only, no request). */
    private suspend fun markersOf(cards: List<EpisodeData>): Map<String, List<EpisodeMarkerData>> {
        val out = HashMap<String, List<EpisodeMarkerData>>()
        for (card in cards) {
            val markers = runCatching { service.episodeMarkers(card.id) }.getOrNull()
            if (!markers.isNullOrEmpty()) out[card.id] = markers
        }
        return out
    }

    private fun setTranscript(
        state: State,
        episodeId: String,
        transcript: uniffi.nz_ffi.TranscriptInfoData?,
    ): State {
        val index = indexOf(state, episodeId)
        if (index < 0) return state
        val card = state.cards[index] ?: return state
        return state.copy(
            cards = state.cards + (index to card.copy(transcript = transcript)),
            version = state.version + 1,
        )
    }

    companion object {
        /** The SPA's `runShowSearch` limit. */
        const val SHOW_SEARCH_LIMIT = 50

        /** « Charger plus » starts this close to the end of the loaded pages. */
        const val LIST_AHEAD = 10

        /** The archive poll stops after this many polls moved nothing. */
        const val STALL_POLLS = 5

        /** How many rows a refresh re-reads (the first page is 50). */
        const val ROWS_TO_REFRESH = 50

        /** The chip the model writes while a start is in flight. */
        val QUEUED = uniffi.nz_ffi.TranscriptInfoData(
            status = "queued",
            progress = 0.0,
            percent = 0u,
            language = null,
            source = null,
            segmentCount = 0u,
            errorCode = null,
            active = true,
            ready = false,
        )

        private val STOPPED = QUEUED.copy(status = "stopped", active = false)
    }
}
