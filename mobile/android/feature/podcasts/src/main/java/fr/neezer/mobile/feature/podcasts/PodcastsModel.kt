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
import kotlinx.coroutines.launch
import uniffi.nz_ffi.ShowData

/**
 * « Podcasts » — the subscriptions, the catalogue search and the subscribe
 * gesture (lot MB13).
 *
 * `routes/Podcasts.svelte` ported: the shows the server holds, a search field
 * that waits **300 ms after the last keystroke** and then asks once
 * (`onInput`/`runSearch`), a card that subscribes by the provider's id
 * (`actions.js#subscribeToPodcast` wraps it in `https://www.deezer.com/show/…`),
 * and a pasted Deezer URL that subscribes on Enter.
 *
 * The first paint is the device's copy ([PodcastService.cachedShows]), then the
 * network replaces it: a warm open costs **0 blocking requests**, and a dead
 * link keeps the copy with its « Hors ligne · version mémorisée » note
 * ([ScreenState.Ready.stale]).
 */
class PodcastsModel(
    private val service: PodcastService,
    private val scope: CoroutineScope,
    /** The SPA's debounce; a test passes 0 and drives the search itself. */
    private val debounceMs: Long = 300,
) {
    /** Everything the screen draws, in one immutable value. */
    data class State(
        val shows: List<ShowData> = emptyList(),
        /** What is shown is the device's copy. */
        val stale: Boolean = false,
        val loading: Boolean = true,
        /** The list could not be read and there is no copy: the cause. */
        val failure: String? = null,
        /** The search field's own text. */
        val query: String = "",
        val searching: Boolean = false,
        /** The term the results belong to, so a stale answer never lands. */
        val searchingFor: String = "",
        val results: List<ShowData> = emptyList(),
        val searchFailure: String? = null,
        /** The card whose subscribe is in flight (its provider id). */
        val subscribing: String? = null,
        /** A one-line message to toast, consumed by the screen. */
        val message: String? = null,
        /** A show the screen must open (a successful subscribe). */
        val openShow: String? = null,
        /** Bumps on every change, so the screen coalesces two states. */
        val version: Long = 0,
    ) {
        val trimmedQuery: String get() = query.trim()

        /** The SPA hides the grid while a search is on. */
        val searchActive: Boolean get() = trimmedQuery.isNotEmpty()
    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    private var loadJob: Job? = null
    private var searchJob: Job? = null

    /** The first load: the copy first, then the server. */
    fun start() {
        if (loadJob?.isActive == true) return
        loadJob = scope.launch {
            _state.update { it.copy(loading = true, failure = null) }
            screenFlow(
                cached = { service.cachedShows() },
                fresh = { service.shows() },
            ).collect { value ->
                when (value) {
                    is ScreenState.Loading -> _state.update { it.copy(loading = true) }
                    is ScreenState.Ready -> _state.update {
                        it.copy(
                            shows = value.data,
                            stale = value.stale,
                            loading = false,
                            failure = null,
                        )
                    }
                    is ScreenState.Failed -> _state.update {
                        it.copy(
                            loading = false,
                            // A copy is already on screen: it stays, the screen
                            // says it is remembered (the standard's rule).
                            failure = if (it.shows.isEmpty()) value.cause else null,
                            stale = it.shows.isNotEmpty(),
                        )
                    }
                }
            }
        }
    }

    /** « Réessayer »: one click, one attempt (nothing retries by itself). */
    fun retry() {
        loadJob?.cancel()
        loadJob = null
        start()
    }

    /**
     * The search field changed. `onInput`'s rule: one request 300 ms after the
     * last key; a shorter term clears everything and asks nothing.
     */
    fun setQuery(query: String) {
        searchJob?.cancel()
        val term = query.trim()
        _state.update {
            it.copy(
                query = query,
                searchFailure = null,
                results = if (term.length < MIN_CHARACTERS) emptyList() else it.results,
                searching = term.length >= MIN_CHARACTERS,
                searchingFor = if (term.length < MIN_CHARACTERS) "" else it.searchingFor,
                version = it.version + 1,
            )
        }
        if (term.length < MIN_CHARACTERS) return
        searchJob = scope.launch {
            delay(debounceMs)
            runSearch(term)
        }
    }

    /** « Réessayer » of the search: the same term, at once. */
    fun retrySearch() {
        val term = _state.value.trimmedQuery
        if (term.length < MIN_CHARACTERS) return
        searchJob?.cancel()
        searchJob = scope.launch { runSearch(term) }
    }

    /** Enter on a pasted Deezer URL: subscribe it and open the show. */
    fun submitLink() {
        val term = _state.value.trimmedQuery
        if (!isDeezerUrl(term)) return
        subscribe(term, key = term)
    }

    /**
     * `PodcastCard`'s tap or « S'abonner »: the provider's id becomes the URL
     * `POST /api/podcasts` takes, and the screen opens the show that comes back.
     */
    fun subscribeCard(show: ShowData) {
        val url = show.subscribeUrl ?: return
        subscribe(url, key = show.deezerId ?: show.id)
    }

    private fun subscribe(url: String, key: String) {
        if (_state.value.subscribing != null) return
        _state.update {
            it.copy(subscribing = key, message = null, version = it.version + 1)
        }
        scope.launch {
            try {
                val card = service.subscribe(url)
                _state.update {
                    it.copy(
                        subscribing = null,
                        message = "Podcast ajouté",
                        // The SPA clears the field and the results, then opens
                        // the show it just subscribed.
                        query = "",
                        results = emptyList(),
                        searching = false,
                        searchingFor = "",
                        openShow = card.id.takeIf { id -> id.isNotEmpty() },
                        version = it.version + 1,
                    )
                }
                // A subscribe changes the list: re-read it so the grid is right
                // even if the person never opens the show.
                reloadAfterSubscribe()
            } catch (error: Throwable) {
                _state.update {
                    it.copy(
                        subscribing = null,
                        message = NzErrors.text(error),
                        version = it.version + 1,
                    )
                }
            }
        }
    }

    private suspend fun reloadAfterSubscribe() {
        runCatching { service.shows() }.onSuccess { shows ->
            _state.update { it.copy(shows = shows, stale = false) }
        }
    }

    /** The screen took the message (a toast is not a state). */
    fun consumeMessage() {
        _state.update { it.copy(message = null) }
    }

    /** The screen opened the show the subscribe answered. */
    fun consumeOpen() {
        _state.update { it.copy(openShow = null) }
    }

    fun stop() {
        loadJob?.cancel()
        searchJob?.cancel()
        loadJob = null
        searchJob = null
    }

    private suspend fun runSearch(term: String) {
        _state.update {
            it.copy(searching = true, searchFailure = null, version = it.version + 1)
        }
        try {
            val results = service.search(term)
            // An answer for a term the person has typed past is DROPPED: the
            // SPA's own `seq` guard, and why two keystrokes never flicker.
            if (_state.value.trimmedQuery != term) return
            _state.update {
                it.copy(
                    results = results,
                    searchingFor = term,
                    searching = false,
                    searchFailure = null,
                    version = it.version + 1,
                )
            }
        } catch (error: Throwable) {
            if (_state.value.trimmedQuery != term) return
            _state.update {
                it.copy(
                    results = emptyList(),
                    searchingFor = term,
                    searching = false,
                    searchFailure = NzErrors.text(error),
                    version = it.version + 1,
                )
            }
        }
    }

    private inline fun MutableStateFlow<State>.update(change: (State) -> State) {
        value = change(value)
    }

    companion object {
        /** `Podcasts.svelte#onInput`: a term under two characters asks nothing. */
        const val MIN_CHARACTERS = 2

        /** The SPA's own test for "this is a Deezer show link". */
        fun isDeezerUrl(term: String): Boolean =
            Regex("""deezer\.com/.*(show|episode)/""").containsMatchIn(term)
    }
}
