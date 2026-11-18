package fr.neezer.mobile.feature.discovery

import fr.neezer.mobile.bridge.NzErrors
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.nz_ffi.RecapYearsData
import uniffi.nz_ffi.TrackRow

/**
 * The Récap's state machine (lot MB14), ported from `routes/Recap.svelte` and
 * `lib/recap.js`.
 *
 * The screen has more states than a plain read because the year is the person's
 * choice: the SPA tells them apart and each has its own panel, so they are a
 * sealed type here rather than a single "error".
 *
 * - **`#/recap` resolves a year**, then opens it: the years are asked once and
 *   kept ten minutes (`recapyears.js`), so the home then the Récap cost one
 *   `/years`.
 * - **A year that is not four digits never reaches the server** (`YEAR_RE`), and
 *   the panel says so instead.
 * - **A known offline is a state, not a red error**: « Ton récap a besoin du
 *   serveur » with « Réessayer » (`Recap.svelte#offline`).
 * - **The deck is one card at a time**; the same year reopened keeps the card
 *   the person was reading, a new year starts at the first.
 */
class RecapModel(
    private val service: DiscoveryService,
    private val player: PlayPort,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val toast: (String) -> Unit = ::discToast,
) {
    /** What the screen draws when it has no deck. */
    sealed interface Failure {
        /** `#/recap/<not a year>`: the panel says the year does not exist. */
        data object Invalid : Failure

        /** The device knows it has no link: the server is needed, once. */
        data object Offline : Failure

        /** Anything else, with the cause and « Réessayer ». */
        data class Cause(val message: String) : Failure
    }

    data class State(
        /** The chips, from the kept years (a side question, never fatal). */
        val years: RecapYearsData? = null,
        /** The deck, when there is one. */
        val data: RecapDeck? = null,
        /** The chip the person is on, which may differ from the painted year. */
        val activeYear: Int? = null,
        val index: Int = 0,
        val loading: Boolean = false,
        /** The device's copy is what is shown (offline, or a refresh failed). */
        val stale: Boolean = false,
        val failure: Failure? = null,
        /** No year at all: « Ton récap apparaîtra quand tu auras écouté… ». */
        val noYears: Boolean = false,
        /** The share overlay is open. */
        val sharing: Boolean = false,
        /** A finger holds the story: the auto-advance is suspended. */
        val holding: Boolean = false,
    ) {
        val slides get() = data?.slides.orEmpty()

        val slide get() = slides.getOrNull(index)

        val chips get() = years?.chips.orEmpty()

        /** `isEmpty`: a reason, or a payload with no card. */
        val isEmpty: Boolean get() = data?.empty == true
    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    private var opening: Job? = null
    private var yearsJob: Job? = null

    /** `#/recap` (year `null`) or `#/recap/<year>`. */
    fun open(year: Int?) {
        if (year == null) {
            resolveLatest()
            return
        }
        if (year < 1000 || year > 9999) {
            _state.update {
                it.copy(data = null, failure = Failure.Invalid, loading = false, activeYear = year)
            }
            return
        }
        openYear(year)
    }

    /**
     * `#/recap`: the year list is the only request, and the latest year it names
     * is then opened. A list with no latest is the "appears when you listen"
     * panel (`noYears`).
     */
    private fun resolveLatest() {
        yearsJob?.cancel()
        yearsJob = scope.launch {
            _state.update { it.copy(loading = true, failure = null, noYears = false) }
            try {
                val years = withContext(io) { service.recapYears() }
                _state.update { it.copy(years = years) }
                val latest = years.latest
                if (latest == null) {
                    _state.update { it.copy(loading = false, noYears = true) }
                } else {
                    openYear(latest)
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(loading = false, failure = Failure.Cause(NzErrors.text(e))) }
            }
        }
    }

    /** `loadRecapYears`'s failure never condemns the year (`catch(() => {})`). */
    private fun loadChips() {
        if (_state.value.years != null) return
        scope.launch {
            runCatching { withContext(io) { service.recapYears() } }
                .onSuccess { years -> _state.update { it.copy(years = years) } }
        }
    }

    private fun openYear(year: Int) {
        opening?.cancel()
        loadChips()
        val kept = runCatching { service.cachedRecap(year) }.getOrNull()
        if (kept != null) {
            _state.update {
                it.copy(
                    data = kept,
                    activeYear = year,
                    index = if (it.data?.year == year) it.index else 0,
                    stale = true,
                    loading = true,
                    failure = null,
                    noYears = false,
                )
            }
        } else {
            _state.update {
                it.copy(
                    data = null,
                    activeYear = year,
                    index = 0,
                    loading = true,
                    failure = null,
                    noYears = false,
                )
            }
        }
        opening = scope.launch {
            try {
                val data = withContext(io) { service.recap(year) }
                _state.update { state ->
                    val same = state.data?.year == data.year && state.data != null
                    state.copy(
                        data = data,
                        activeYear = data.year,
                        index = if (same) state.index.coerceIn(0, data.slides.lastIndex.coerceAtLeast(0)) else 0,
                        loading = false,
                        stale = false,
                        failure = null,
                    )
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { state ->
                    if (state.data != null) {
                        // What the person already saw stays, flagged as kept.
                        state.copy(loading = false, stale = true)
                    } else {
                        state.copy(loading = false, failure = failureOf(e))
                    }
                }
            }
        }
    }

    private fun failureOf(e: Exception): Failure {
        val offline = runCatching { fr.neezer.mobile.bridge.NzErrors.text(e) }.getOrNull()
        return when {
            e is uniffi.nz_ffi.NzException.Offline -> Failure.Offline
            e is uniffi.nz_ffi.NzException.Invalid -> Failure.Invalid
            offline == "Hors ligne" -> Failure.Offline
            else -> Failure.Cause(NzErrors.text(e))
        }
    }

    /** « Réessayer »: the same year, one honest attempt (`manual`). */
    fun retry() {
        val year = _state.value.activeYear
        if (year == null) {
            resolveLatest()
        } else {
            openYear(year)
        }
    }

    /** A year chip. The painted year answers at once; another one is opened. */
    fun chooseYear(year: Int) {
        if (_state.value.data?.year == year) {
            _state.update { it.copy(activeYear = year) }
            return
        }
        openYear(year)
    }

    /** The empty panel's « Voir 2025 ». */
    fun openNearby() {
        _state.value.data?.nearbyYear?.let(::chooseYear)
    }

    /** A tap on the right (`delta > 0`) or the left of the card. */
    fun tap(right: Boolean) = advance(if (right) 1 else -1)

    fun advance(delta: Int) {
        val count = _state.value.slides.size
        if (count == 0) return
        _state.update { it.copy(index = (it.index + delta).coerceIn(0, count - 1)) }
    }

    /** A finger down pauses the auto-advance; up resumes it (`holding`). */
    fun hold(holding: Boolean) = _state.update { it.copy(holding = holding) }

    /** « Revoir mon récap ». */
    fun restart() = _state.update { it.copy(index = 0) }

    /** « Partager »: the overlay; the file is the screen's to draw and send. */
    fun share(open: Boolean) = _state.update { it.copy(sharing = open) }

    /** « Lire ces 5 titres »: the year's own top, in its order. */
    fun playTop() {
        val data = _state.value.data ?: return
        scope.launch {
            val top = withContext(io) { service.recapTop(data.year) }
            val rows: List<TrackRow> = withContext(io) { top.window(0, minOf(top.len(), 5)) }
            if (rows.isEmpty()) {
                toast("Cette année n'a pas de titre à lire.")
                return@launch
            }
            player.playList(rows, 0, CONTEXT_RECAP, data.year.toString())
        }
    }

    companion object {
        /** `RecapStory.svelte#AUTO_MS`: the phone's seven seconds per card. */
        const val AUTO_MS = 7_000L
        const val CONTEXT_RECAP = "recap"
    }
}

/**
 * The Récap's nine cards' layouts, chosen by the slide's own id — the same
 * `RecapSlide.svelte#ICONS` vocabulary travels in `glyph`.
 */
fun slideIsBig(id: String): Boolean = id == "year" || id == "share"
