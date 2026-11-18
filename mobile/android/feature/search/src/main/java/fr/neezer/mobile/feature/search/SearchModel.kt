package fr.neezer.mobile.feature.search

import fr.neezer.mobile.bridge.NzErrors
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.nz_ffi.ImportOffer
import uniffi.nz_ffi.NzException
import uniffi.nz_ffi.SearchFallback
import uniffi.nz_ffi.SearchScope

/** The tabs of the library scope (`Search.svelte`'s `all | tracks | artists | albums | playlists`). */
enum class SearchTab { All, Tracks, Artists, Albums, Playlists }

/**
 * The search's state machine (lot MB6), `routes/Search.svelte` ported.
 *
 * Every rule the SPA taught is here, in the order the person meets it, and the
 * whole thing is plain Kotlin over [SearchService] and [SearchPlayer] —
 * `SearchModelTest` proves each one on the JVM with a fake, on a VIRTUAL clock.
 *
 * - **One settled query, two requests.** The list is debounced
 *   ([DEBOUNCE_MS], the SPA's 280 ms rounded to the mobile contract's 250) and
 *   the fallback has its own, longer beat ([YOUTUBE_MS], the SPA's 800): a person
 *   who pauses mid-word pays for one search, not for one search plus one YouTube
 *   query per pause. **A new key cancels the previous request** — the FFI future
 *   is dropped, so one query is in flight at a time.
 * - **Two characters or nothing** ([SearchService.minTerm]), and a pasted
 *   playlist link is an IMPORT offer, never a query (0 requests).
 * - **The kept answer paints first** (`cachedSearch`, disk only): a warm visit
 *   shows what it saw, then the network replaces it; a dead link keeps it with
 *   « Hors ligne · version mémorisée »; with nothing to show, the cause and
 *   « Réessayer ».
 * - **A failed fallback never condemns the list**: its own line, with
 *   « Réessayer », under the same list (rule 3).
 * - **The radio plays from what is on screen** (`searchAsRadio`): the first title
 *   before any request, and the player's own refill tops the queue up.
 *
 * The numbers the lot is held to: a settled query = 2 requests cold, 0 blocking
 * warm (the kept copy), and « daft punk » typed fast = 2 requests (`onQuery` nine
 * times, one debounce).
 */
class SearchModel(
    private val service: SearchService,
    private val player: SearchPlayer,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val toast: (String) -> Unit = { fr.neezer.mobile.designsystem.NzToasts.show(it) },
) {
    data class State(
        /** What the box holds, exactly as typed. */
        val query: String = "",
        /** Which source the answer comes from. */
        val scope: SearchScope = SearchScope.LIBRARY,
        val tab: SearchTab = SearchTab.All,
        /** The answer being drawn (the kept copy, then the fresh one). */
        val answer: SearchAnswer? = null,
        /** What is drawn is the copy this device kept (offline, or a refresh failed). */
        val stale: Boolean = false,
        /** A read is in flight (the skeleton, never a spinner over a painted list). */
        val loading: Boolean = false,
        /** The cause of a read that failed with nothing to draw (with « Réessayer »). */
        val failure: String? = null,
        /** Where the fallback stands for the term on screen. */
        val youtube: YoutubeUi = YoutubeUi.Idle,
        /** The screen's YouTube chip (`Search.svelte`'s `yt`): ON by default. */
        val youtubeOn: Boolean = true,
        /**
         * Whether this account can ask the fallback at all — read from the
         * session when the screen opens, so the chip never appears and then
         * vanishes when the first answer lands.
         */
        val youtubeAvailable: Boolean = false,
        /** The shows only exist for an admin, and only where the server has them. */
        val showsAvailable: Boolean = false,
        /** The transcripts route answered `Unsupported` (rule 7): the scope is hidden. */
        val podcastsAvailable: Boolean = true,
        /** « Récents » — the terms this account searched, newest first. */
        val recent: List<String> = emptyList(),
        /** A pasted playlist link: an import offer, not a query. */
        val importOffer: ImportOffer? = null,
        /** « Lancer la radio » is in flight (the button says so, a second press is ignored). */
        val radioBusy: Boolean = false,
    ) {
        /** The term the answer is about, trimmed ("" when nothing was searched). */
        val term: String get() = query.trim()

        /** What was actually searched — never half a word typed. */
        val searchedTerm: String? get() = answer?.term?.takeIf { it.isNotEmpty() }

        /** The tabs of the current scope. */
        val podcasts: Boolean get() = scope == SearchScope.PODCASTS
    }

    /** Where the fallback stands (`Search.svelte`'s `ytWaiting`/`ytErrorHere`/`ytAnswerHere`). */
    sealed interface YoutubeUi {
        data object Idle : YoutubeUi
        data object Waiting : YoutubeUi
        data class Added(val rows: Int) : YoutubeUi
        data class Failed(val failure: SearchFallback) : YoutubeUi
    }

    private val _state = MutableStateFlow(
        State(
            recent = runCatching { service.recentSearches() }.getOrDefault(emptyList()),
        ),
    )
    val state: StateFlow<State> = _state.asStateFlow()

    /** The debounced read (the list and, after it, the fallback). */
    private var loadJob: Job? = null

    /** The fallback's own beat, armed by an answer. */
    private var youtubeJob: Job? = null

    /**
     * Reads the screen's first answer: the kept copy of the term the shell handed
     * over (a deep link, `AppLink.Search(q)`), then its network read.
     */
    fun start(term: String? = null) {
        // What the SESSION already knows, before any search: the device's terms,
        // whether the shows exist for this account, and whether the fallback can
        // be asked at all. Read once, so nothing on the screen appears and then
        // vanishes under the first answer.
        _state.update {
            it.copy(
                recent = runCatching { service.recentSearches() }.getOrDefault(emptyList()),
                showsAvailable = runCatching { service.isAdmin() }.getOrDefault(false),
                youtubeAvailable = runCatching { service.youtubeAvailable() }.getOrDefault(false),
            )
        }
        val initial = term?.trim().orEmpty()
        if (initial.length >= service.minTerm) {
            _state.update { it.copy(query = initial) }
            submit()
        }
    }

    // -- the box ---------------------------------------------------------------

    /** A key was typed or deleted. The debounce is restarted on every key. */
    fun onQuery(text: String) {
        val offer = runCatching { service.importOffer(text) }.getOrNull()
        _state.update { it.copy(query = text, importOffer = offer) }
        loadJob?.cancel()
        youtubeJob?.cancel()
        val term = text.trim()
        if (offer != null || term.length < service.minTerm) {
            // Nothing to search: a link is an import, and a one-letter term names
            // nothing (`Search.svelte#run`'s own two guards).
            _state.update {
                it.copy(
                    answer = null,
                    failure = null,
                    loading = false,
                    stale = false,
                    youtube = YoutubeUi.Idle,
                )
            }
            return
        }
        loadJob = scope.launch {
            delay(DEBOUNCE_MS)
            load(term, askFallbackNow = false)
        }
    }

    /**
     * Enter: the search at once, the fallback at once, the term remembered.
     *
     * **A term the list on screen already answers is not asked again** — Enter
     * then only closes the keyboard and remembers the term. The SPA re-runs its
     * `run(term)` here; on a phone link that is a second identical request for a
     * list the person is already reading ("same result, cheaper", CLAUDE.md), and
     * the fallback is still asked if it never answered.
     */
    fun submit() {
        val term = _state.value.query.trim()
        if (term.length < service.minTerm || _state.value.importOffer != null) return
        val current = _state.value
        val already = current.answer?.takeIf { it.term == term && !current.stale }
        runCatching { service.noteSearch(term) }
        _state.update { it.copy(recent = runCatching { service.recentSearches() }.getOrDefault(emptyList())) }
        if (already != null) {
            if (current.youtube !is YoutubeUi.Waiting && !already.youtubeAnswered) {
                armFallback(term, already)
            }
            return
        }
        loadJob?.cancel()
        youtubeJob?.cancel()
        loadJob = scope.launch { load(term, askFallbackNow = true) }
    }

    /** A chip of « Récents »: the term is a search this person chose. */
    fun useRecent(term: String) {
        _state.update { it.copy(query = term, importOffer = null) }
        submit()
    }

    /**
     * The screen is going away: a term that was really SEARCHED is remembered, a
     * half-typed word is not (`Search.svelte#onDestroy`: `searchedFor === q`).
     */
    fun leave() {
        val state = _state.value
        val searched = state.searchedTerm
        if (searched != null && searched == state.term) {
            runCatching { service.noteSearch(searched) }
        }
    }

    /** « Effacer » — the device forgets them (one write, no request). */
    fun clearRecent() {
        runCatching { service.clearRecentSearches() }
        _state.update { it.copy(recent = emptyList()) }
    }

    /**
     * A source chip: the library, the transcripts, or the shows.
     *
     * The parameter is [next] and not `scope`: the model's own
     * `scope: CoroutineScope` would be shadowed, and `scope.launch` would stop
     * compiling (measured).
     */
    fun chooseScope(next: SearchScope) {
        if (next == _state.value.scope) return
        if (next != SearchScope.LIBRARY) {
            _state.update { it.copy(showsAvailable = service.isAdmin()) }
        }
        _state.update {
            it.copy(
                scope = next,
                answer = null,
                failure = null,
                stale = false,
                loading = false,
            )
        }
        val term = _state.value.term
        if (term.length >= service.minTerm) {
            loadJob?.cancel()
            loadJob = scope.launch { load(term, askFallbackNow = false) }
        }
    }

    /** A tab: switching costs NOTHING (the answer already holds every family). */
    fun chooseTab(tab: SearchTab) = _state.update { it.copy(tab = tab) }

    /** « Réessayer » on a failed read. */
    fun retry() {
        val term = _state.value.term
        if (term.length < service.minTerm) return
        loadJob?.cancel()
        loadJob = scope.launch { load(term, askFallbackNow = false) }
    }

    /**
     * « Réessayer » on the fallback's own line: ONE more request, for the term on
     * screen (`Search.svelte#runYoutube(true)`).
     */
    fun retryFallback() {
        val term = _state.value.term
        if (term.length < service.minTerm) return
        youtubeJob?.cancel()
        youtubeJob = scope.launch { askFallback(term, wait = false) }
    }

    /**
     * The YouTube chip: it turns the fallback off for this screen, or back on.
     *
     * Off, the video rows leave the list at once — a re-read of the answer this
     * device kept, **0 requests** (`Search.svelte` re-ranks with `yt ?` its hits
     * `: []`). On, a term the fallback already answered is not asked again; a
     * term that failed is.
     */
    fun toggleYoutube() {
        val on = !_state.value.youtubeOn
        _state.update { it.copy(youtubeOn = on) }
        youtubeJob?.cancel()
        val term = _state.value.term
        youtubeJob = scope.launch {
            if (term.length < service.minTerm) {
                _state.update { it.copy(youtube = YoutubeUi.Idle) }
                return@launch
            }
            val kept = runCatching {
                withContext(io) { service.cachedSearch(term, _state.value.scope, on) }
            }.getOrNull()
            if (kept != null && term == _state.value.term) paint(kept, stale = _state.value.stale)
            if (!on) {
                _state.update { it.copy(youtube = YoutubeUi.Idle) }
                return@launch
            }
            val answer = _state.value.answer ?: return@launch
            if (answer.youtubeAnswered) {
                _state.update { it.copy(youtube = YoutubeUi.Added(answer.youtube)) }
            } else {
                askFallback(term, wait = false)
            }
        }
    }

    // -- the radio -------------------------------------------------------------

    /**
     * « Lancer la radio »: the titles already on screen open the queue, the first
     * one before any request, and the player's own refill asks for the rest one
     * at a time. Two presses in one burst are one launch.
     */
    fun launchRadio() {
        if (_state.value.radioBusy) return
        val term = _state.value.term
        if (term.length < service.minTerm) return
        _state.update { it.copy(radioBusy = true) }
        scope.launch {
            try {
                val plan = withContext(io) { service.searchAsRadio(term) }
                if (plan.tracks.isEmpty() || plan.contextId.isEmpty()) {
                    toast("La radio n'a rien à jouer pour cette recherche.")
                } else {
                    player.play(plan.tracks, plan.contextKind, plan.contextId)
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                toast("La radio n'a pas pu démarrer : ${NzErrors.text(e)}. Réessayer.")
            } finally {
                _state.update { it.copy(radioBusy = false) }
            }
        }
    }

    // -- internals -------------------------------------------------------------

    /** The kept copy, the fresh answer, then the fallback's beat. */
    private suspend fun load(term: String, askFallbackNow: Boolean) {
        // A newer key already started: this read is not the one on screen.
        if (term != _state.value.term) return
        _state.update {
            it.copy(
                loading = it.answer?.term != term,
                failure = null,
                youtube = if (it.answer?.term == term) it.youtube else YoutubeUi.Idle,
            )
        }
        // 1. What this device kept paints at once (0 requests).
        val kept = runCatching {
            withContext(io) { service.cachedSearch(term, _state.value.scope, _state.value.youtubeOn) }
        }.getOrNull()
        if (kept != null && term == _state.value.term) {
            paint(kept, stale = true)
        }
        // 2. The network replaces it.
        try {
            val fresh = withContext(io) {
                service.search(term, _state.value.scope, _state.value.youtubeOn)
            }
            if (term != _state.value.term || fresh.scope != _state.value.scope) return
            paint(fresh, stale = false)
            _state.update { it.copy(loading = false, failure = null) }
            if (askFallbackNow) {
                youtubeJob?.cancel()
                youtubeJob = scope.launch { askFallback(term, wait = false) }
            } else {
                armFallback(term, fresh)
            }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            if (term != _state.value.term) return
            // `NzError::Unsupported` (rule 7): the server predates the route.
            val unsupported = e is NzException.Unsupported
            if (unsupported && _state.value.scope == SearchScope.PODCASTS) {
                // Rule 7: an older server hides the entry instead of failing every
                // visit. The chip goes away and the library comes back.
                _state.update {
                    it.copy(
                        podcastsAvailable = false,
                        scope = SearchScope.LIBRARY,
                        loading = false,
                        failure = null,
                    )
                }
                toast("La recherche dans les transcriptions n'est pas disponible sur ce serveur.")
                return
            }
            if (_state.value.answer?.term == term) {
                // The copy stays, and says it is one ("Hors ligne · version mémorisée").
                _state.update { it.copy(stale = true, loading = false) }
            } else {
                _state.update { it.copy(failure = NzErrors.text(e), loading = false) }
            }
        }
    }

    /** Arms the fallback's own beat once an answer is on screen. */
    private fun armFallback(term: String, answer: SearchAnswer) {
        if (_state.value.scope != SearchScope.LIBRARY) return
        if (!_state.value.youtubeOn) return
        if (!answer.youtubeAvailable) return
        // A term the fallback already answered is never re-asked on its own
        // (`Search.svelte`'s `ytAnswerHere`).
        if (answer.youtubeAnswered) {
            _state.update { it.copy(youtube = YoutubeUi.Added(answer.youtube)) }
            return
        }
        youtubeJob?.cancel()
        youtubeJob = scope.launch { askFallback(term, wait = true) }
    }

    /**
     * Asks the fallback, once, and weaves its rows into the SAME list.
     *
     * A failure is a line under the list, never an empty screen: the catalogue's
     * rows stay, and « Réessayer » asks once more.
     */
    private suspend fun askFallback(term: String, wait: Boolean) {
        if (term != _state.value.term) return
        if (wait) {
            _state.update { it.copy(youtube = YoutubeUi.Waiting) }
            delay(YOUTUBE_MS)
            if (term != _state.value.term) return
        } else {
            _state.update { it.copy(youtube = YoutubeUi.Waiting) }
        }
        try {
            val woven = withContext(io) { service.youtubeHits(term) }
            if (term != _state.value.term) return
            paint(woven, stale = _state.value.stale)
            _state.update {
                it.copy(
                    youtube = woven.youtubeFailure
                        ?.let { failure -> YoutubeUi.Failed(failure) }
                        ?: YoutubeUi.Added(woven.youtube),
                )
            }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            if (term != _state.value.term) return
            _state.update { it.copy(youtube = YoutubeUi.Failed(SearchFallback.FAILED)) }
        }
    }

    /** One answer on screen; the fallback's standing comes with it. */
    private fun paint(answer: SearchAnswer, stale: Boolean) {
        // Read OUTSIDE the update: its lambda may run twice, and an FFI call is
        // not something to repeat for a state change.
        val admin = runCatching { service.isAdmin() }.getOrDefault(false)
        _state.update {
            it.copy(
                answer = answer,
                stale = stale,
                loading = false,
                failure = null,
                showsAvailable = it.showsAvailable || admin,
                youtubeAvailable = it.youtubeAvailable || answer.youtubeAvailable,
                youtube = when {
                    answer.youtubeFailure != null -> YoutubeUi.Failed(answer.youtubeFailure)
                    answer.youtubeAnswered -> YoutubeUi.Added(answer.youtube)
                    else -> it.youtube
                },
            )
        }
    }

    companion object {
        /** `Search.svelte`'s debounce, rounded to this lot's contract (250 ms). */
        const val DEBOUNCE_MS = 250L

        /** The SPA's fallback beat: 800 ms after the last keystroke. */
        const val YOUTUBE_MS = 800L

        /** The SPA's `RECENT_MAX`. */
        const val RECENT_MAX = 5
    }
}
