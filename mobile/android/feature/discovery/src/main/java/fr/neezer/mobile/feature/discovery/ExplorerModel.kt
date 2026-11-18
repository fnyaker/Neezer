package fr.neezer.mobile.feature.discovery

import fr.neezer.mobile.bridge.NzErrors
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.bridge.screenFlow
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
import uniffi.nz_ffi.ExplorerLevelData
import uniffi.nz_ffi.ExplorerSettingsData
import uniffi.nz_ffi.ExplorerStylesData
import uniffi.nz_ffi.StationRowData
import uniffi.nz_ffi.StyleTileData

/**
 * Explorer's state machine (lot MB14), ported from `components/ExplorerStyles.svelte`.
 *
 * Three surfaces, three sets of rules:
 *
 * - **The tiles** are the ten parent styles of the person's taste, kept on the
 *   device: one `/explorer/styles` per visit, painted from the kept copy first.
 * - **The discovery sheet** is one `POST` on « Découvrir », then one `GET` every
 *   `pollMs` while the server works (the screen drives the delay; this model
 *   only answers `poll()`), at most `maxPolls` times. A `429`/`503` is a NAMED
 *   WAIT with its own retry, never a spinner (`phase = busy`).
 * - **The stations** are one `/radio/genre/<style>` per tile disc and one
 *   `/flow/styles` for the whole list (`stylesOf`), which never fans out per
 *   tile.
 *
 * A phase kept per (style, level) — the SPA's `results` Map, bounded to eight —
 * so reopening a panel paints what was found instead of asking again.
 */
class ExplorerModel(
    private val service: DiscoveryService,
    private val player: PlayPort,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val toast: (String) -> Unit = ::discToast,
) {
    /** Where one (style, level) stands. Mirrors `ExplorerStyles.svelte#phase`. */
    enum class Phase { Idle, Pending, Ready, Empty, Busy, Error }

    data class Found(
        val phase: Phase,
        val playlists: List<PlaylistTracks> = emptyList(),
        val cause: String = "",
        val busyStatus: Int = 0,
        val retryAfter: Int = 0,
        /** « Réessayer » must POST again, or poll a job already running. */
        val retryMode: String = RETRY_POST,
    )

    data class State(
        val screen: ScreenState<ExplorerStylesData> = ScreenState.Loading,
        /** The sheet is open on this style. */
        val picked: StyleTileData? = null,
        val level: Int = 3,
        val phase: Phase = Phase.Idle,
        val playlists: List<PlaylistTracks> = emptyList(),
        val cause: String = "",
        val busyStatus: Int = 0,
        /** The server's own wait, in seconds (`Retry-After`). */
        val retryAfter: Int = 0,
        /** The same, counting down each `tick()`. */
        val retryLeft: Int = 0,
        val retryMode: String = RETRY_POST,
        val polls: Int = 0,
        /** The style whose station is being started (one at a time). */
        val stationBusy: String? = null,
        /** « Toutes les stations » is open. */
        val allOpen: Boolean = false,
        val allRows: ScreenState<List<StationRowData>> = ScreenState.Loading,
        val allQuery: String = "",
        /** The five notches, and the poll's two numbers (from the core). */
        val levels: List<ExplorerLevelData> = emptyList(),
        val maxPolls: Int = 60,
        val pollMs: Long = 2_000L,
    ) {
        val data: ExplorerStylesData? get() = (screen as? ScreenState.Ready)?.data

        val tiles: List<StyleTileData> get() = data?.tiles.orEmpty()

        val stale: Boolean get() = (screen as? ScreenState.Ready)?.stale == true

        val aim: ExplorerLevelData? get() = levels.firstOrNull { it.level == level.toUInt() }

        /** "Réessaie dans 5 s." / "Tu peux réessayer." / "Réessaie dans un moment." */
        val busyWait: String
            get() = when {
                retryLeft > 0 -> "Réessaie dans $retryLeft s."
                retryAfter > 0 -> "Tu peux réessayer."
                else -> "Réessaie dans un moment."
            }

        /** The searched station rows, folded (`filterStyles`). */
        val shownStations: List<StationRowData>
            get() = when (val rows = allRows) {
                is ScreenState.Ready -> filterStations(rows.data, allQuery)
                else -> emptyList()
            }
    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    private var loading: Job? = null
    private var discovering: Job? = null
    private var stations: Job? = null

    /** The SPA's `results` Map: eight (style, level) answers at most. */
    private val remembered = LinkedHashMap<String, Found>()

    init {
        val settings = runCatching { service.explorerSettings() }.getOrNull()
        if (settings != null) {
            _state.update {
                it.copy(
                    levels = settings.levels,
                    maxPolls = settings.maxPolls.toInt(),
                    pollMs = settings.pollMs.toLong(),
                )
            }
        }
    }

    // -- the tiles -----------------------------------------------------------

    /** The kept tiles paint, then one `/explorer/styles` refreshes them. */
    fun load() {
        if (loading?.isActive == true) return
        loading = scope.launch {
            screenFlow(
                cached = { withContext(io) { service.cachedExplorerStyles() } },
                fresh = { withContext(io) { service.explorerStyles() } },
            ).collect { screen -> _state.update { it.copy(screen = screen) } }
        }
    }

    fun retry() {
        val failed = _state.value.screen as? ScreenState.Failed ?: return
        failed.retry()
    }

    // -- the discovery sheet -------------------------------------------------

    /** A tile's disc: one request, then the queue plays at once. */
    fun startStation(name: String, fromSheet: Boolean = false) {
        if (_state.value.stationBusy != null) return
        _state.update { it.copy(stationBusy = name) }
        scope.launch {
            try {
                val station = withContext(io) { service.genreStation(name) }
                if (station.trackCount == 0) {
                    toast("Cette station n'a encore rien à jouer.")
                } else {
                    player.play(station.rows, 0, station.kind, station.id)
                    if (fromSheet) closeStations()
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                toast("La station n'a pas pu démarrer (${NzErrors.text(e)})")
            } finally {
                _state.update { it.copy(stationBusy = null) }
            }
        }
    }

    /** The tile's card: the sheet opens on its remembered answer, if any. */
    fun chooseStyle(tile: StyleTileData) {
        val kept = remembered[key(tile.name, _state.value.level)]
        _state.update {
            it.copy(
                picked = tile,
                phase = kept?.phase ?: Phase.Idle,
                playlists = kept?.playlists.orEmpty(),
                cause = kept?.cause.orEmpty(),
                busyStatus = kept?.busyStatus ?: 0,
                retryAfter = kept?.retryAfter ?: 0,
                retryLeft = kept?.retryAfter ?: 0,
                retryMode = kept?.retryMode ?: RETRY_POST,
                polls = 0,
            )
        }
    }

    fun close() {
        discovering?.cancel()
        _state.update {
            it.copy(picked = null, phase = Phase.Idle, playlists = emptyList(), cause = "", polls = 0)
        }
    }

    /** The five notches: a level already discovered paints itself. */
    fun setLevel(level: Int) {
        val clamped = level.coerceIn(1, 5)
        val picked = _state.value.picked ?: return
        val kept = remembered[key(picked.name, clamped)]
        _state.update {
            it.copy(
                level = clamped,
                phase = kept?.phase ?: Phase.Idle,
                playlists = kept?.playlists.orEmpty(),
                cause = kept?.cause.orEmpty(),
                busyStatus = kept?.busyStatus ?: 0,
                retryAfter = kept?.retryAfter ?: 0,
                retryLeft = kept?.retryAfter ?: 0,
                retryMode = kept?.retryMode ?: RETRY_POST,
                polls = 0,
            )
        }
    }

    /** « Découvrir »: ONE POST. Nothing else is asked until the screen polls. */
    fun discover() {
        val picked = _state.value.picked ?: return
        if (_state.value.phase == Phase.Pending) return
        val level = _state.value.level
        discovering?.cancel()
        _state.update {
            it.copy(
                phase = Phase.Pending,
                playlists = emptyList(),
                cause = "",
                busyStatus = 0,
                retryAfter = 0,
                retryLeft = 0,
                retryMode = RETRY_POST,
                polls = 0,
            )
        }
        discovering = scope.launch {
            try {
                val answer = withContext(io) { service.explorerDiscover(picked.name, level) }
                apply(answer)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                fail(e, RETRY_POST)
            }
        }
    }

    /** One poll. The screen calls it `pollMs` after the last one, `maxPolls` at most. */
    fun poll() {
        val state = _state.value
        val picked = state.picked ?: return
        if (state.phase != Phase.Pending) return
        if (state.polls >= state.maxPolls) {
            _state.update {
                it.copy(
                    phase = Phase.Error,
                    cause = "La découverte prend plus de temps que prévu. Réessayez dans un moment.",
                    retryMode = RETRY_POST,
                )
            }
            return
        }
        _state.update { it.copy(polls = it.polls + 1) }
        scope.launch {
            try {
                val answer = withContext(io) { service.explorerPoll(picked.name, state.level) }
                apply(answer)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                fail(e, RETRY_POLL)
            }
        }
    }

    /** The busy countdown's one second: the screen's ticker, not a core timer. */
    fun tick() {
        val state = _state.value
        if (state.phase != Phase.Busy || state.retryLeft <= 0) return
        _state.update { it.copy(retryLeft = (it.retryLeft - 1).coerceAtLeast(0)) }
    }

    /**
     * « Réessayer » honours what failed: never a second POST for a job the server
     * already accepted, never a GET when nothing was launched.
     */
    fun retryDiscover() {
        if (_state.value.retryMode == RETRY_POLL) {
            _state.update {
                it.copy(phase = Phase.Pending, cause = "", busyStatus = 0, retryAfter = 0, retryLeft = 0)
            }
            poll()
        } else {
            discover()
        }
    }

    /** « Essayer un niveau plus large »: one notch, then discover if unknown. */
    fun widen() {
        if (_state.value.level >= 5) return
        setLevel(_state.value.level + 1)
        if (_state.value.phase == Phase.Idle || _state.value.phase == Phase.Empty || _state.value.phase == Phase.Error) {
            discover()
        }
    }

    /** A playlist's « Lire »: the checked titles ARE the answer, no request. */
    fun playPlaylist(playlist: PlaylistTracks) {
        if (playlist.trackCount == 0) {
            toast("Cette playlist n'a pas de titre vérifié.")
            return
        }
        player.play(playlist.rows, 0, CONTEXT_PLAYLIST, playlist.id)
        close()
    }

    private fun apply(answer: DiscoverResult) {
        val phase = when (answer.phase) {
            // A finished sweep with nothing in it is the "essayez un niveau plus
            // large" answer, never a ready panel with no row (`ready` with an
            // empty list is what both the server and this side may say).
            "ready" -> if (answer.playlists.isEmpty()) Phase.Empty else Phase.Ready
            "empty" -> Phase.Empty
            "busy" -> Phase.Busy
            "pending" -> Phase.Pending
            else -> Phase.Idle
        }
        _state.update {
            it.copy(
                phase = phase,
                playlists = answer.playlists,
                busyStatus = answer.busyStatus,
                retryAfter = answer.retryAfterS,
                retryLeft = answer.retryAfterS,
                cause = answer.busyCause,
                retryMode = if (phase == Phase.Busy) it.retryMode else RETRY_POST,
            )
        }
        remember(phase, answer)
    }

    private fun fail(e: Exception, mode: String) {
        _state.update {
            it.copy(
                phase = Phase.Error,
                cause = NzErrors.text(e),
                retryMode = mode,
                busyStatus = 0,
                retryAfter = 0,
                retryLeft = 0,
            )
        }
        remember(Phase.Error, null)
    }

    private fun remember(phase: Phase, answer: DiscoverResult?) {
        val picked = _state.value.picked ?: return
        val entry = Found(
            phase = phase,
            playlists = answer?.playlists ?: _state.value.playlists,
            cause = answer?.busyCause ?: _state.value.cause,
            busyStatus = answer?.busyStatus ?: 0,
            retryAfter = answer?.retryAfterS ?: 0,
            retryMode = _state.value.retryMode,
        )
        val key = key(picked.name, _state.value.level)
        remembered.remove(key)
        remembered[key] = entry
        while (remembered.size > 8) {
            remembered.remove(remembered.keys.first())
        }
    }

    // -- all the stations -----------------------------------------------------

    fun openStations() {
        _state.update { it.copy(allOpen = true, allQuery = "") }
        loadStations()
    }

    fun closeStations() = _state.update { it.copy(allOpen = false, allQuery = "") }

    fun searchStations(query: String) = _state.update { it.copy(allQuery = query) }

    /**
     * The sheet paints its kept copy first, then asks ONE `/flow/styles`:
     * every analysed genre, never a request per tile.
     */
    fun loadStations() {
        if (stations?.isActive == true) return
        val kept = runCatching { service.cachedStationStyles() }.getOrNull()
        if (kept != null && _state.value.allRows !is ScreenState.Ready) {
            _state.update { it.copy(allRows = ScreenState.Ready(kept, stale = true)) }
        }
        stations = scope.launch {
            try {
                val rows = withContext(io) { service.stationStyles() }
                _state.update { it.copy(allRows = ScreenState.Ready(rows)) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                val current = _state.value.allRows
                if (current is ScreenState.Ready) {
                    _state.update { it.copy(allRows = ScreenState.Ready(current.data, stale = true)) }
                } else {
                    _state.update { it.copy(allRows = ScreenState.Failed(NzErrors.text(e)) { loadStations() }) }
                }
            }
        }
        // A failed read answers its own retry: the sheet's button calls this
        // again, and `stations` is free by then.
        stations?.invokeOnCompletion { }
    }

    private fun key(style: String, level: Int): String = "$style|$level"

    companion object {
        const val RETRY_POST = "post"
        const val RETRY_POLL = "poll"
        const val CONTEXT_PLAYLIST = "playlist"
    }
}

/**
 * `genrestation.js#filterStyles`, on this side of the boundary for the sheet's
 * live search: accents and punctuation set aside, the same fold `flat()` does
 * (`Électro` ⇄ `electro`, `Disco / funk` ⇄ `disco funk`).
 */
fun filterStations(rows: List<StationRowData>, query: String): List<StationRowData> {
    val needle = foldGenre(query)
    if (needle.isEmpty()) return rows
    return rows.filter { row ->
        foldGenre(row.name).contains(needle) || foldGenre(row.parent ?: "").contains(needle)
    }
}
