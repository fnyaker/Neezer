package fr.neezer.mobile.feature.onboarding

import fr.neezer.mobile.bridge.NzErrors
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import uniffi.nz_ffi.AccountOptions
import uniffi.nz_ffi.ArtistCard
import uniffi.nz_ffi.DiversityBounds
import uniffi.nz_ffi.FeedState
import uniffi.nz_ffi.FinishBlock
import uniffi.nz_ffi.ImportJob
import uniffi.nz_ffi.ImportState
import uniffi.nz_ffi.OnboardingStep
import uniffi.nz_ffi.RelatedAnswer
import uniffi.nz_ffi.RelatedState

/**
 * The onboarding's state machine (lot MB5b), `routes/Onboarding.svelte` and
 * `components/OnboardingAccount.svelte` ported.
 *
 * Plain Kotlin over [OnboardingService] and [OnboardingRules], so
 * `OnboardingModelTest` proves every rule on the JVM with a fake, on a VIRTUAL
 * clock. What the page taught, in the order a person meets it:
 *
 * - **Two requests cold, none warm.** The kept copy paints first; the visit's
 *   two reads replace it; a dead link keeps the copy with « Hors ligne · version
 *   mémorisée », and with nothing to paint the cause and « Réessayer ».
 * - **A like is optimistic and settles.** The heart and the count change in the
 *   same frame; ONE request carries the neighbours; a refusal rolls both back
 *   and offers « Réessayer » (rule 4). A neighbour request the server says is
 *   still `pending` is polled (900 ms then 1 500 ms, five tries), and one it says
 *   failed shows its own line — never a silent gap.
 * - **A search is debounced** (350 ms, the page's own), cancelled by a new key,
 *   and refuses under two characters.
 * - **« Terminé » obeys the page's own gate**: one kept artist, a live link, and
 *   nothing still being written; « Passer pour l'instant » appears only after two
 *   attempts at a feed the server itself calls unavailable.
 * - **The account step is optional and never blocks the exit.** Each card paints
 *   what the server says (even a provider switched off), the import is a SERVER
 *   job resumed by [loadOptions] after the screen was left, and its poll shows
 *   the phase, the count and the CAUSE of every line nobody found (rule 2).
 */
class OnboardingModel(
    private val service: OnboardingService,
    private val rules: OnboardingRules,
    private val scope: CoroutineScope,
    private val describe: (Throwable) -> String = NzErrors::text,
    private val onDone: () -> Unit = {},
) {
    data class ActionError(val message: String, val retry: (() -> Unit)? = null)

    /** Where one liked tile's neighbours stand (`slots`). */
    enum class RelatedSlot { Pending, Failed }

    data class State(
        val step: OnboardingStep = OnboardingStep.ARTISTS,
        /** The link, as the last answer taught it: a failure says so, a success restores it. */
        val online: Boolean = true,
        /** Nothing painted yet (the skeleton, never a spinner over a painted page). */
        val loading: Boolean = true,
        /** What is painted is the copy this device kept. */
        val stale: Boolean = false,
        /** The cause of a read that failed with nothing to paint. */
        val failure: String? = null,
        val tiles: List<ArtistCard> = emptyList(),
        /** The cursor of the next page, `null` on the last one. */
        val next: String? = null,
        val feed: FeedState = FeedState(
            state = "",
            building = false,
            error = null,
            source = "",
            ready = false,
            stale = false,
            progress = null,
        ),
        val liked: Int = 0,
        val minArtists: Int = 1,
        val diversity: DiversityBounds = DiversityBounds(
            window = 12u,
            maxSameGenre = 4u,
            maxStreak = 2u,
        ),
        /** The tiles the person kept — the page's `likedIds`. */
        val likedIds: Set<String> = emptySet(),
        /** A like being written: the tile says so and a second tap is ignored. */
        val pendingIds: Set<String> = emptySet(),
        val relatedSlots: Map<String, RelatedSlot> = emptyMap(),
        val loadingMore: Boolean = false,
        /** What the box holds, exactly as typed. */
        val query: String = "",
        /** The search's answer, when one is on screen. */
        val searchItems: List<ArtistCard>? = null,
        val searching: Boolean = false,
        val searchFailure: String? = null,
        /** The PROVIDER's cause beside a library-only answer. */
        val searchNote: String? = null,
        val actionError: ActionError? = null,
        /** « 2 artistes proches ajoutés », « Vous avez fait le tour… » */
        val notice: String? = null,
        /** The finish button's own line (why it waits). */
        val status: String? = null,
        val finishing: Boolean = false,
        val feedAttempts: Int = 0,
        /** « Passer pour l'instant »: the page's two-attempt gate, set by the model. */
        val canPass: Boolean = false,
        // -- the account step ------------------------------------------------
        val options: AccountOptions? = null,
        val optionsLoading: Boolean = false,
        val optionsFailure: String? = null,
        val importJob: ImportJob? = null,
        /** The import's own action in flight (`"save"` or empty). */
        val importBusy: String = "",
        /** The five-minute poll cap was reached: the screen offers « Actualiser ». */
        val importCapped: Boolean = false,
        /** The provider whose link form is open (`"deezer"`, `"youtube"`, `"playlist"`). */
        val open: String = "",
        val linkBusy: String = "",
        val linkError: Map<String, String> = emptyMap(),
        val accountNotice: String? = null,
        val skipped: Set<String> = emptySet(),
        /** A first « Terminer » during an import only arms the second one. */
        val finishArmed: Boolean = false,
    ) {
        /** What the list draws: the search's answer or the feed. */
        val items: List<ArtistCard> get() = searchItems ?: tiles

        /** Whether the feed itself has nothing left to offer. */
        val endOfList: Boolean get() = next == null && items.isNotEmpty()

        val searchingNow: Boolean get() = query.isNotBlank() && searchItems != null

        /** Whether the feed's own reason for being empty can be named. */
        val feedUnavailable: Boolean get() = feed.state == "unavailable" && !feed.building

        /** Whether an import (playlist or a linked account's) is still running. */
        val importRunning: Boolean
            get() {
                val job = importJob
                if (job != null && (job.state == ImportState.READING ||
                        job.state == ImportState.MATCHING ||
                        job.state == ImportState.SAVING)
                ) {
                    return true
                }
                val options = options ?: return false
                return (options.deezer.account?.running != null) ||
                    (options.youtube.account?.running != null)
            }
    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    private val jobs = mutableListOf<Job>()
    private var loadJob: Job? = null
    private var searchJob: Job? = null
    private var debounceJob: Job? = null
    private var buildJob: Job? = null
    private var accountPollJob: Job? = null
    private var importPollJob: Job? = null
    private var relatedJobs = mutableMapOf<String, Job>()

    // -- the visit ----------------------------------------------------------

    /** Paints the kept copy, then the visit's two reads. */
    fun start() {
        loadJob?.cancel()
        loadJob = track(scope.launch {
            val kept = runCatching { service.cached() }.getOrNull()
            if (kept != null) apply(kept, stale = true)
            try {
                val fresh = service.load()
                apply(fresh, stale = false)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                val online = e !is uniffi.nz_ffi.NzException.Offline
                if (kept == null) {
                    _state.update { it.copy(loading = false, failure = describe(e), online = online) }
                } else {
                    _state.update { it.copy(loading = false, stale = true, online = online) }
                }
            }
        })
    }

    private fun apply(data: uniffi.nz_ffi.OnboardingData, stale: Boolean) {
        val liked = data.page.items.filter { it.liked }.map { it.key }
        val canPass = rules.canPass(
            _state.value.feedAttempts,
            data.state.feed.state,
            data.state.feed.building,
            data.state.liked.toInt(),
            data.state.minArtists.toInt().coerceAtLeast(1),
        )
        _state.update {
            it.copy(
                canPass = canPass,
                step = data.state.step,
                loading = false,
                stale = stale,
                failure = null,
                tiles = data.page.items,
                next = data.page.next,
                feed = data.state.feed,
                liked = data.state.liked.toInt(),
                minArtists = data.state.minArtists.toInt().coerceAtLeast(1),
                diversity = data.state.diversity,
                likedIds = it.likedIds + liked,
            )
        }
        when (data.state.step) {
            OnboardingStep.DONE -> done()
            OnboardingStep.ACCOUNT -> enterAccount()
            OnboardingStep.ARTISTS -> if (data.page.items.isEmpty() && data.page.building) pollBuild()
        }
    }

    /** The page's list is the feed, unless a search is on screen. */
    private fun activeList(): List<ArtistCard> = _state.value.items

    private fun setActive(list: List<ArtistCard>) {
        _state.update { current ->
            if (current.searchItems != null) current.copy(searchItems = list) else current.copy(tiles = list)
        }
    }

    /** The first 200 ids on screen, what a like tells the server to avoid. */
    private fun seen(): List<String> = activeList()
        .map { it.key }
        .filter { it.isNotEmpty() }
        .take(200)

    // -- likes --------------------------------------------------------------

    /** The heart: optimistic in the same frame, then ONE request. */
    fun toggle(card: ArtistCard) {
        val id = card.key
        if (id.isEmpty() || _state.value.pendingIds.contains(id)) return
        _state.update { it.copy(actionError = null, notice = null) }
        if (_state.value.likedIds.contains(id)) unlike(id) else like(id)
    }

    private fun like(id: String) {
        _state.update {
            it.copy(likedIds = it.likedIds + id, liked = it.liked + 1, pendingIds = it.pendingIds + id)
        }
        track(scope.launch {
            try {
                val answer = service.like(id, seen())
                settleLike(id, answer)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                rollbackLike(id)
                _state.update {
                    it.copy(
                        actionError = ActionError(
                            "Votre choix n'a pas été enregistré.",
                            retry = { like(id) },
                        ),
                    )
                }
            }
        })
    }

    private fun settleLike(id: String, answer: RelatedAnswer) {
        _state.update { it.copy(pendingIds = it.pendingIds - id) }
        when (answer.state) {
            RelatedState.READY -> fillRelated(id, answer.items)
            RelatedState.PENDING -> {
                _state.update { it.copy(relatedSlots = it.relatedSlots + (id to RelatedSlot.Pending)) }
                pollRelated(id, 0)
            }
            RelatedState.ERROR -> {
                _state.update { it.copy(relatedSlots = it.relatedSlots + (id to RelatedSlot.Failed)) }
            }
            RelatedState.NONE -> _state.update { it.copy(relatedSlots = it.relatedSlots - id) }
        }
    }

    private fun fillRelated(parent: String, related: List<ArtistCard>) {
        val current = activeList()
        val index = current.indexOfFirst { it.key == parent }
        _state.update { it.copy(relatedSlots = it.relatedSlots - parent) }
        if (index < 0 || related.isEmpty()) return
        val merged = rules.insertSimilar(
            items = current,
            index = index,
            similar = related,
            liked = _state.value.likedIds,
            perLike = rules.similarPerLike,
            budget = rules.similarTotalMax,
            diversity = _state.value.diversity,
        )
        setActive(merged.items)
        val inserted = merged.inserted.toInt()
        if (inserted > 0) {
            val plural = if (inserted > 1) "s" else ""
            _state.update {
                it.copy(notice = "$inserted proche$plural ajouté$plural")
            }
        }
    }

    private fun pollRelated(parent: String, attempt: Int) {
        relatedJobs[parent]?.cancel()
        relatedJobs[parent] = track(scope.launch {
            delay(if (attempt == 0) RELATED_FIRST_MS else RELATED_NEXT_MS)
            try {
                val answer = service.related(parent, seen())
                when (answer.state) {
                    RelatedState.READY -> fillRelated(parent, answer.items)
                    RelatedState.PENDING -> if (attempt + 1 < RELATED_TRIES) {
                        pollRelated(parent, attempt + 1)
                    } else {
                        _state.update {
                            it.copy(relatedSlots = it.relatedSlots + (parent to RelatedSlot.Failed))
                        }
                    }
                    else -> _state.update {
                        it.copy(relatedSlots = it.relatedSlots - parent)
                    }
                }
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                _state.update {
                    it.copy(relatedSlots = it.relatedSlots + (parent to RelatedSlot.Failed))
                }
            }
        })
    }

    /** The tile's own « Réessayer » under « Les artistes proches n'ont pas pu… ». */
    fun retryRelated(parent: String) {
        _state.update { it.copy(relatedSlots = it.relatedSlots + (parent to RelatedSlot.Pending)) }
        track(scope.launch {
            try {
                val answer = service.related(parent, seen())
                when (answer.state) {
                    RelatedState.READY -> fillRelated(parent, answer.items)
                    RelatedState.PENDING -> pollRelated(parent, 1)
                    else -> _state.update {
                        it.copy(relatedSlots = it.relatedSlots + (parent to RelatedSlot.Failed))
                    }
                }
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                _state.update {
                    it.copy(relatedSlots = it.relatedSlots + (parent to RelatedSlot.Failed))
                }
            }
        })
    }

    private fun unlike(id: String) {
        val before = _state.value
        _state.update {
            it.copy(
                likedIds = it.likedIds - id,
                liked = (it.liked - 1).coerceAtLeast(0),
                pendingIds = it.pendingIds + id,
            )
        }
        setActive(rules.removeSimilar(activeList(), id, _state.value.likedIds))
        track(scope.launch {
            try {
                service.unlike(id)
                _state.update { it.copy(pendingIds = it.pendingIds - id) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update {
                    before.copy(
                        pendingIds = before.pendingIds - id,
                        actionError = ActionError(
                            "Votre choix n'a pas été enregistré.",
                            retry = { toggle(artistById(id)) },
                        ),
                    )
                }
            }
        })
    }

    private fun rollbackLike(id: String) {
        _state.update {
            it.copy(
                likedIds = it.likedIds - id,
                liked = (it.liked - 1).coerceAtLeast(0),
                pendingIds = it.pendingIds - id,
                relatedSlots = it.relatedSlots - id,
            )
        }
    }

    private fun artistById(id: String): ArtistCard =
        activeList().firstOrNull { it.key == id } ?: ArtistCard(
            id = id,
            deezerId = id,
            name = "",
            picture = null,
            pictureSmall = null,
            genres = emptyList(),
            provider = "deezer",
            via = null,
            parent = null,
            liked = true,
            openable = true,
            hint = null,
        )

    // -- the search box -----------------------------------------------------

    fun onQuery(raw: String) {
        _state.update { it.copy(query = raw) }
        debounceJob?.cancel()
        val normalized = rules.normalizeQuery(raw)
        if (normalized.length < rules.searchMin) {
            clearSearch()
            return
        }
        debounceJob = track(scope.launch {
            delay(SEARCH_DEBOUNCE_MS)
            runSearch(normalized)
        })
    }

    /** Enter: the page re-runs the term at once (it does not wait the debounce). */
    fun submitSearch() {
        debounceJob?.cancel()
        val normalized = rules.normalizeQuery(_state.value.query)
        if (normalized.length < rules.searchMin) {
            clearSearch()
            return
        }
        track(scope.launch { runSearch(normalized) })
    }

    private suspend fun runSearch(term: String) {
        searchJob?.cancel()
        _state.update { it.copy(searching = true, searchFailure = null) }
        searchJob = track(scope.launch {
            try {
                val found = service.search(term, rules.pageFirst)
                _state.update {
                    it.copy(
                        query = found.query.ifEmpty { term },
                        searchItems = found.items,
                        searching = false,
                        searchFailure = null,
                        searchNote = found.error?.let { code ->
                            OnboardingCopy.feedCause(rules.feedCause(code))
                        },
                        actionError = null,
                        notice = null,
                    )
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update {
                    it.copy(searching = false, searchItems = emptyList(), searchFailure = describe(e))
                }
            }
        })
    }

    fun retrySearch() {
        val term = rules.normalizeQuery(_state.value.query)
        if (term.length >= rules.searchMin) track(scope.launch { runSearch(term) })
    }

    fun clearSearch() {
        debounceJob?.cancel()
        searchJob?.cancel()
        _state.update {
            it.copy(
                query = "",
                searchItems = null,
                searching = false,
                searchFailure = null,
                searchNote = null,
            )
        }
    }

    // -- more tiles, and a feed the server is building ----------------------

    fun loadMore() {
        val current = _state.value
        if (current.searchingNow || current.loadingMore) return
        val after = current.next ?: return
        _state.update { it.copy(loadingMore = true) }
        track(scope.launch {
            try {
                val page = service.page(after, rules.pageNext)
                _state.update {
                    it.copy(
                        tiles = rules.reconcile(it.tiles, page.items),
                        next = page.next,
                        loadingMore = false,
                    )
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(loadingMore = false, notice = describe(e)) }
            }
        })
    }

    /** « Réessayer » when the server said it had no selection but was building. */
    fun retryFeed() {
        val attempts = _state.value.feedAttempts + 1
        _state.update { it.copy(feedAttempts = attempts, notice = null) }
        track(scope.launch {
            try {
                service.feedRetry()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update {
                    it.copy(
                        notice = if (e is uniffi.nz_ffi.NzException) {
                            describe(e)
                        } else {
                            "Une reconstruction vient d'être demandée. Réessayez dans quelques minutes."
                        },
                    )
                }
            }
            try {
                val page = service.page(null, rules.pageFirst)
                _state.update {
                    it.copy(
                        tiles = if (it.tiles.isEmpty()) page.items else rules.reconcile(it.tiles, page.items),
                        next = page.next,
                        feedAttempts = attempts,
                    )
                }
                if (page.items.isEmpty() && page.building) pollBuild()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(notice = describe(e)) }
            }
        })
    }

    private fun pollBuild() {
        if (buildJob?.isActive == true) return
        buildJob = track(scope.launch {
            var tries = 0
            while (tries < BUILD_TRIES) {
                delay(BUILD_POLL_MS)
                tries += 1
                val state = try {
                    service.state()
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    continue
                }
                _state.update {
                    it.copy(
                        feed = state.feed,
                        liked = maxOf(it.liked, state.liked.toInt()),
                        canPass = rules.canPass(
                            it.feedAttempts,
                            state.feed.state,
                            state.feed.building,
                            it.liked,
                            it.minArtists,
                        ),
                    )
                }
                if (state.feed.state == "ready") {
                    val page = runCatching { service.page(null, rules.pageFirst) }.getOrNull() ?: return@launch
                    _state.update {
                        it.copy(
                            tiles = if (it.tiles.isEmpty()) page.items else rules.reconcile(it.tiles, page.items),
                            next = page.next,
                            feed = state.feed,
                        )
                    }
                    return@launch
                }
                if (state.feed.state == "unavailable" && !state.feed.building) return@launch
            }
        })
    }

    // -- finishing the artists step -----------------------------------------

    /** « Terminé »: the page's own gate, then ONE request. */
    fun finish() {
        val current = _state.value
        val blocked = rules.finishBlocked(
            current.liked,
            current.pendingIds.size,
            0,
            current.online,
            current.minArtists,
        )
        if (blocked != null) {
            _state.update { it.copy(status = OnboardingCopy.finishMessage(blocked)) }
            return
        }
        _state.update { it.copy(finishing = true, status = "") }
        track(scope.launch {
            try {
                val done = service.artistsDone(false)
                _state.update { it.copy(finishing = false) }
                when (done.step) {
                    OnboardingStep.ACCOUNT -> enterAccount()
                    OnboardingStep.DONE -> done()
                    OnboardingStep.ARTISTS -> Unit
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update {
                    it.copy(finishing = false, status = describe(e))
                }
            }
        })
    }

    /** « Passer pour l'instant »: one request, `{skip: true}`. */
    fun pass() {
        if (_state.value.finishing) return
        _state.update { it.copy(finishing = true, status = "") }
        track(scope.launch {
            try {
                val done = service.artistsDone(true)
                _state.update { it.copy(finishing = false) }
                when (done.step) {
                    OnboardingStep.ACCOUNT -> enterAccount()
                    else -> done()
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(finishing = false, status = describe(e)) }
            }
        })
    }

    private fun done() {
        _state.update { it.copy(step = OnboardingStep.DONE) }
        onDone()
    }

    private fun enterAccount() {
        // The artists step's own notice ("2 proches ajoutés") belongs to the step
        // it was about; the account pane starts clean.
        _state.update { it.copy(step = OnboardingStep.ACCOUNT, notice = null, status = null) }
        loadOptions()
    }

    // -- the account step ---------------------------------------------------

    /** One request; also the RESUME: the options carry the import still running. */
    fun loadOptions() {
        if (_state.value.optionsLoading) return
        _state.update { it.copy(optionsLoading = true, optionsFailure = null) }
        track(scope.launch {
            try {
                val options = service.options()
                _state.update {
                    it.copy(
                        options = options,
                        optionsLoading = false,
                        optionsFailure = null,
                        importJob = options.job ?: it.importJob,
                    )
                }
                if (options.job != null) pollImport()
                watchAccounts()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(optionsLoading = false, optionsFailure = describe(e)) }
            }
        })
    }

    fun open(provider: String) {
        _state.update { it.copy(open = provider, linkError = it.linkError - provider) }
    }

    fun closeForm() {
        _state.update { it.copy(open = "") }
    }

    /** Links the person's own Deezer (an ARL) or YouTube Music (browse headers). */
    fun link(provider: String, secret: String) {
        val current = _state.value
        if (current.linkBusy.isNotEmpty()) return
        if (secret.isBlank()) {
            _state.update {
                it.copy(
                    linkError = it.linkError + (provider to if (provider == "deezer") {
                        "Collez votre ARL Deezer."
                    } else {
                        "Collez les en-têtes d'une session YouTube Music."
                    }),
                )
            }
            return
        }
        _state.update { it.copy(linkBusy = provider, linkError = it.linkError - provider, accountNotice = null) }
        track(scope.launch {
            try {
                val options = service.link(provider, secret)
                _state.update {
                    it.copy(
                        options = options,
                        linkBusy = "",
                        open = "",
                        accountNotice = if (provider == "deezer") {
                            "Compte Deezer relié. L'import de vos favoris a démarré sur le serveur."
                        } else {
                            "Compte YouTube Music relié. L'import de vos favoris a démarré sur le serveur."
                        },
                    )
                }
                watchAccounts(restart = true)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update {
                    it.copy(linkBusy = "", linkError = it.linkError + (provider to describe(e)))
                }
            }
        })
    }

    /** Starts a public playlist import: ONE request, then the job's poll. */
    fun startImport(url: String) {
        val current = _state.value
        if (current.linkBusy.isNotEmpty()) return
        if (url.isBlank()) {
            _state.update { it.copy(linkError = it.linkError + ("playlist" to "Collez le lien d'une playlist.")) }
            return
        }
        _state.update { it.copy(linkBusy = "playlist", linkError = it.linkError - "playlist") }
        track(scope.launch {
            try {
                val started = service.startImport(url)
                _state.update {
                    it.copy(
                        options = started.options,
                        importJob = started.job,
                        linkBusy = "",
                        open = "",
                        importCapped = false,
                    )
                }
                pollImport(0)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update {
                    it.copy(linkBusy = "", linkError = it.linkError + ("playlist" to describe(e)))
                }
            }
        })
    }

    /** « Ajouter à ma bibliothèque »: keeps the import as a LOCAL playlist. */
    fun saveImport() {
        val job = _state.value.importJob ?: return
        if (_state.value.importBusy.isNotEmpty()) return
        _state.update { it.copy(importBusy = "save") }
        track(scope.launch {
            try {
                val saved = service.importSave(job.id, job.name)
                _state.update { it.copy(importJob = saved, importBusy = "") }
                pollImport(0)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update {
                    it.copy(importBusy = "", linkError = it.linkError + ("playlist" to describe(e)))
                }
            }
        })
    }

    /** « Relancer » after a failed import: the URL field is reopened. */
    fun retryImport() {
        _state.update { it.copy(importJob = null, open = "playlist", importCapped = false) }
    }

    /** « Actualiser » once the poll cap was reached. */
    fun refreshImport() {
        _state.update { it.copy(importCapped = false) }
        pollImport(0)
    }

    private fun pollImport(from: Int = 0) {
        if (importPollJob?.isActive == true && from == 0) return
        importPollJob?.cancel()
        importPollJob = track(scope.launch {
            var polls = from
            while (polls < IMPORT_POLL_MAX) {
                val job = _state.value.importJob ?: return@launch
                if (!job.running()) return@launch
                delay(IMPORT_POLL_MS)
                polls += 1
                val next = try {
                    service.importStatus(job.id)
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    if (describe(e).contains("expiré")) {
                        _state.update {
                            it.copy(importJob = job.copy(state = ImportState.ERROR, error = describe(e)))
                        }
                        return@launch
                    }
                    delay(IMPORT_POLL_BACKOFF_MS)
                    continue
                }
                _state.update { it.copy(importJob = next, importCapped = false) }
            }
            _state.update { it.copy(importCapped = true) }
        })
    }

    /** The linked accounts' own imports, polled while one runs (1.5 s). */
    private fun watchAccounts(restart: Boolean = false) {
        val running = _state.value.options?.let { options ->
            options.deezer.account?.running != null || options.youtube.account?.running != null
        } ?: false
        if (!running) {
            accountPollJob?.cancel()
            accountPollJob = null
            return
        }
        if (accountPollJob?.isActive == true && !restart) return
        accountPollJob?.cancel()
        accountPollJob = track(scope.launch {
            var polls = 0
            while (polls < ACCOUNT_POLL_MAX) {
                delay(ACCOUNT_POLL_MS)
                polls += 1
                val options = runCatching { service.options() }.getOrNull() ?: continue
                _state.update { it.copy(options = options) }
                val still = options.deezer.account?.running != null ||
                    options.youtube.account?.running != null
                if (!still) return@launch
            }
        })
    }

    /** « Terminer »: a first press during an import only arms the second. */
    fun accountDone() {
        if (!_state.value.online) {
            _state.update { it.copy(accountNotice = "Hors ligne : terminez quand la connexion revient.") }
            return
        }
        val current = _state.value
        if (current.importRunning && !current.finishArmed) {
            _state.update {
                it.copy(
                    finishArmed = true,
                    accountNotice = "Un import est en cours : il continuera sur le serveur. Cliquez sur Terminer pour quitter maintenant.",
                )
            }
            return
        }
        _state.update { it.copy(finishing = true, accountNotice = null) }
        track(scope.launch {
            try {
                service.accountDone()
                _state.update { it.copy(finishing = false) }
                done()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update {
                    it.copy(
                        finishing = false,
                        accountNotice = describe(e),
                        actionError = if (describe(e).contains("artiste")) {
                            ActionError("Choisissez au moins un artiste avant de terminer.", retry = { gotoArtists() })
                        } else {
                            null
                        },
                    )
                }
            }
        })
    }

    /**
     * The step a redeemed account said is next, painted BEFORE the first read:
     * an account whose onboarding is at `account` must not flash the artists page
     * while the visit loads.
     */
    fun gotoAccountForStart() {
        if (_state.value.step != OnboardingStep.ACCOUNT) {
            _state.update { it.copy(step = OnboardingStep.ACCOUNT) }
        }
    }

    /** Back to the artists step when the server says it is not finished. */
    fun gotoArtists() {
        _state.update { it.copy(actionError = null, step = OnboardingStep.ARTISTS) }
    }

    fun skip(provider: String) {
        _state.update { it.copy(skipped = it.skipped + provider) }
    }

    fun unskip(provider: String) {
        _state.update { it.copy(skipped = it.skipped - provider) }
    }

    fun dismissActionError() {
        _state.update { it.copy(actionError = null) }
    }

    /** Leaving the screen cancels what it started; the SERVER's job goes on. */
    fun leave() {
        jobs.forEach { it.cancel() }
        jobs.clear()
        relatedJobs.values.forEach { it.cancel() }
        relatedJobs.clear()
    }

    private fun track(job: Job): Job {
        jobs.removeAll { it.isCompleted }
        jobs.add(job)
        return job
    }

    private fun ImportJob.running(): Boolean =
        state == ImportState.READING || state == ImportState.MATCHING || state == ImportState.SAVING

    companion object {
        /** The page's 350 ms debounce. */
        const val SEARCH_DEBOUNCE_MS = 350L
        /** The page's first wait for a tile's neighbours, then its 1 500 ms. */
        const val RELATED_FIRST_MS = 900L
        const val RELATED_NEXT_MS = 1_500L
        const val RELATED_TRIES = 5
        /** The feed's build poll: 2 s, at most 15 tries (`BUILD_TRIES`). */
        const val BUILD_POLL_MS = 2_000L
        const val BUILD_TRIES = 15
        /** The import's poll: 1.5 s, at most 200 (five minutes) — a COUNT, not a clock. */
        const val IMPORT_POLL_MS = 1_500L
        const val IMPORT_POLL_BACKOFF_MS = 3_000L
        const val IMPORT_POLL_MAX = 200
        /** The linked accounts' poll. */
        const val ACCOUNT_POLL_MS = 1_500L
        const val ACCOUNT_POLL_MAX = 200
    }
}
