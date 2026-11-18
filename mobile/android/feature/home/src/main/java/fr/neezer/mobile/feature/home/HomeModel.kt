package fr.neezer.mobile.feature.home

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
import uniffi.nz_ffi.FlowPress
import uniffi.nz_ffi.FlowStateData
import uniffi.nz_ffi.FlowStyleData
import uniffi.nz_ffi.HomeData
import uniffi.nz_ffi.TrackRow

/**
 * The home's state machine (lot MB5).
 *
 * Every rule the SPA taught is here, in the order the person meets it, and the
 * whole thing is plain Kotlin over [HomeService] and [PlayerPort] — `HomeModelTest`
 * proves each one on the JVM with a fake, no JNA, no `.so`, no device.
 *
 * - **The cache paints first, the network replaces it** (`screenFlow`): a warm
 *   open shows the device's copy in the first frame and refreshes behind it; a
 *   dead link keeps the copy and the screen says « Hors ligne · version
 *   mémorisée »; with nothing to show, the cause and « Réessayer ».
 * - **A click answers in the same frame.** The notch, the styles and the
 *   discovery level move in [state] BEFORE their request; a refused write rolls
 *   back and raises a toast, never a silent change.
 * - **One request per action**, never a storm: « Lancer le Flow » is one
 *   `POST /flow/next`, a mood card is one `/api/moods/<id>`, « Autres
 *   suggestions » is one `/recommend`, a pressed notch is one `PUT`.
 * - **Play is a command to the player**, through [PlayerPort]: the queue rules
 *   are MB3b's and are not repeated here.
 *
 * The two numbers the lot is held to (empty `home` rows are never asked):
 * a cold open costs the SPA's seven requests, a warm one zero.
 */
class HomeModel(
    private val service: HomeService,
    private val player: PlayerPort,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val toast: (String) -> Unit = { fr.neezer.mobile.designsystem.NzToasts.show(it) },
) {
    data class State(
        /** What to draw: loading, the answer (with the cache flag), or the cause. */
        val screen: ScreenState<HomeData> = ScreenState.Loading,
        /** The style chips' genres, kept for the session. */
        val styles: List<FlowStyleData> = emptyList(),
        /** The genres the Flow rests on (`[]`: the whole taste). */
        val chosen: List<String> = emptyList(),
        /** The Flow's steering, refreshed after every change. */
        val flow: FlowStateData? = null,
        /** The discovery panel under the hero is open. */
        val tuneOpen: Boolean = false,
        /** The Flow's picker panel is open. */
        val flowPanel: Boolean = false,
        /** « Lancer le Flow » is in flight (the glyph spins, a second press is ignored). */
        val starting: Boolean = false,
        /** « Lancer le Flow » from the picker is in flight. */
        val launching: Boolean = false,
        /** The mood whose mix is being fetched. */
        val moodBusy: String? = null,
        /** « Autres suggestions » is in flight. */
        val recoBusy: Boolean = false,
    ) {
        /** The answer, when there is one to draw. */
        val data: HomeData? get() = (screen as? ScreenState.Ready)?.data

        /** What is drawn came from the device and the refresh failed. */
        val stale: Boolean get() = (screen as? ScreenState.Ready)?.stale == true

        /** The notch the hero draws as chosen: the device's, before any answer. */
        val notch: Double? get() = flow?.energy ?: data?.hero?.energy

        /** The discovery level the chip and the panel draw. */
        val level: Double get() = flow?.level ?: data?.discovery?.level ?: 0.35

        /** The four names the hero's tuning panel offers. */
        val discoverySteps get() = data?.discovery?.steps.orEmpty()

        /** The six names the Flow's picker offers. */
        val flowSteps get() = data?.discovery?.flowSteps.orEmpty()
    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    private var loading: Job? = null
    private var stylesJob: Job? = null

    /** Reads the home: the device's copy first, then the SPA's seven requests. */
    fun load() {
        if (loading?.isActive == true) return
        loading = scope.launch {
            // The device's own Flow state, before anything: the hero's notches
            // and the chip are the person's on the first frame. Off the main
            // thread: it reads the kv store and the saved queue.
            withContext(io) { publishFlow() }
            screenFlow(
                cached = { withContext(io) { service.cachedHome() } },
                fresh = { withContext(io) { service.home() } },
            ).collect { screen ->
                _state.update { it.copy(screen = screen) }
                if (screen is ScreenState.Ready) {
                    publishFlow()
                    loadStyles()
                }
            }
        }
    }

    /** « Réessayer », from the failure block. */
    fun retry() {
        val failed = _state.value.screen as? ScreenState.Failed ?: return
        failed.retry()
    }

    // -- the Flow's steering ---------------------------------------------------

    /** Opens or closes the hero's discovery panel. */
    fun tune(open: Boolean) = _state.update { it.copy(tuneOpen = open) }

    /** Opens or closes the Flow's picker. */
    fun flowPanel(open: Boolean) = _state.update { it.copy(flowPanel = open) }

    /** A style chip: the Flow rests on it, or stops doing so (`StylePicker`). */
    fun pickStyle(name: String) {
        val current = _state.value.chosen
        val next = if (current.contains(name)) current - name else (current + name).takeLast(STYLES_MAX)
        // Optimistic: the chip repaints now, the device writes at once (no request).
        _state.update { it.copy(chosen = next) }
        service.setFlowStyles(next)
        publishFlow(styles = next)
    }

    /** « Tout » : the whole taste, no style at all. */
    fun clearStyles() {
        _state.update { it.copy(chosen = emptyList()) }
        service.setFlowStyles(emptyList())
        publishFlow(styles = emptyList())
    }

    /** An energy notch of the hero, or one of the player's controls (+/-). */
    fun chooseNotch(notch: Double?) {
        val before = _state.value.notch
        _state.update { it.copy(flow = it.flow?.copy(energy = notch, energyName = energyName(it, notch))) }
        scope.launch {
            try {
                withContext(io) { service.setEnergy(notch) }
                publishFlow()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(flow = it.flow?.copy(energy = before, energyName = energyName(it, before))) }
                toast("Impossible d'enregistrer l'énergie : ${NzErrors.text(e)}. Réessayer.")
            }
        }
    }

    /** One notch up ([direction] > 0) or down, from the player's controls. */
    fun nudge(direction: Int) {
        val next = service.nudgeEnergy(direction) ?: return
        _state.update { it.copy(flow = it.flow?.copy(energy = next, energyName = energyName(it, next))) }
        reaimIfLive()
    }

    /** A stop of the discovery ladder: the chip, the Flow's picker, the hero's panel. */
    fun chooseDiscovery(level: Double) {
        val before = _state.value.level
        _state.update { it.copy(flow = it.flow?.copy(level = level), tuneOpen = false) }
        scope.launch {
            try {
                withContext(io) { service.setDiscoveryLevel(level) }
                publishFlow()
                reaimIfLive()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(flow = it.flow?.copy(level = before)) }
                toast("Impossible d'enregistrer la découverte : ${NzErrors.text(e)}. Réessayer.")
            }
        }
    }

    /** « Réinitialiser »: no style, the default breadth — the picker's own rule. */
    fun resetFlow() {
        clearStyles()
        chooseDiscovery(DEFAULT_LEVEL)
    }

    /**
     * « Lancer le Flow » — the hero's button and the picker's.
     *
     * One request carrying the notch, the level and the styles; the queue then
     * holds the playing title and AT MOST the next (the player's own refill asks
     * for one more at the end of each row).
     */
    fun launchFlow() {
        if (_state.value.starting || _state.value.launching) return
        _state.update { it.copy(starting = !it.flowPanel, launching = it.flowPanel) }
        scope.launch {
            try {
                val start = withContext(io) { service.startFlow() }
                if (start.tracks.isEmpty()) {
                    toast("Rien à proposer pour l'instant")
                } else {
                    player.play(start.tracks, 0, CONTEXT_FLOW, start.sid)
                    _state.update { it.copy(flowPanel = false) }
                }
                publishFlow()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                toast("Le Flow ne répond pas, réessayez : ${NzErrors.text(e)}")
            } finally {
                _state.update { it.copy(starting = false, launching = false) }
            }
        }
    }

    /** « Plus comme ça » / « Moins comme ça » (the Flow's controls). */
    fun pressFlow(press: FlowPress) {
        scope.launch {
            try {
                val next = withContext(io) { service.flowFeedback(press) }
                when (press) {
                    FlowPress.MORE -> {
                        next?.let {
                            player.clearUpcoming()
                            player.playNext(it)
                        }
                        toast("Plus de titres comme celui-ci")
                    }
                    FlowPress.LESS -> {
                        next?.let {
                            player.clearUpcoming()
                            player.playNext(it)
                        }
                        player.skipToNext()
                        toast("Moins de titres comme celui-ci")
                    }
                    FlowPress.REAIM -> next?.let {
                        player.clearUpcoming()
                        player.playNext(it)
                    }
                }
                publishFlow()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                toast("Le Flow ne répond pas, réessayez : ${NzErrors.text(e)}")
            }
        }
    }

    /** The notch or the breadth moved while a Flow plays: the next is chosen again. */
    private fun reaimIfLive() {
        if (_state.value.flow?.live == true) pressFlow(FlowPress.REAIM)
    }

    // -- what the person taps --------------------------------------------------

    /** A sleeve of « Nouveautés »: that release, from its first title. */
    fun playRelease(index: Int) {
        val tracks = _state.value.data?.releases?.tracks.orEmpty()
        if (tracks.isEmpty()) return
        val start = index.coerceIn(0, tracks.lastIndex)
        player.play(tracks, start, CONTEXT_RELEASES, "")
    }

    /** « Tout lire »: every new title, from the first. */
    fun playAllNew() {
        val tracks = _state.value.data?.releases?.tracks.orEmpty()
        if (tracks.isNotEmpty()) player.play(tracks, 0, CONTEXT_RELEASES, "")
    }

    /** A « Pour toi » row: the title alone, then its radio (`playThenRadio`). */
    fun playReco(row: TrackRow) {
        player.play(listOf(row), 0, CONTEXT_RADIO, row.id)
    }

    /** Swipe right on a row: « Lire ensuite » (the queue is the player's). */
    fun queueNext(row: TrackRow) = player.playNext(row)

    /** « Autres suggestions »: one request, the rows dim while it runs. */
    fun refreshReco() {
        if (_state.value.recoBusy) return
        _state.update { it.copy(recoBusy = true) }
        scope.launch {
            try {
                val rows = withContext(io) { service.moreSuggestions() }
                _state.update { state ->
                    val data = state.data
                    if (data == null || rows.isEmpty()) state else state.copy(screen = ScreenState.Ready(data.copy(reco = rows), state.stale))
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                toast("Pas d'autres suggestions : ${NzErrors.text(e)}. Réessayer.")
            } finally {
                _state.update { it.copy(recoBusy = false) }
            }
        }
    }

    /**
     * A « Mix d'ambiance »: ONE request for its titles, then a FINITE queue —
     * nothing is appended when the last title ends (`playMood`'s `autoplay:
     * false`, which the player already does for a context it does not top up).
     */
    fun playMood(id: String) {
        if (_state.value.moodBusy == id) return
        _state.update { it.copy(moodBusy = id) }
        scope.launch {
            try {
                val tracks = withContext(io) { service.moodTracks(id) }
                if (tracks.isEmpty()) {
                    toast("Ce mix est vide pour l'instant.")
                } else {
                    player.play(tracks, 0, CONTEXT_MOOD, id)
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                toast("Impossible de lancer ce mix : ${NzErrors.text(e)}. Réessaie.")
            } finally {
                _state.update { it.copy(moodBusy = null) }
            }
        }
    }

    // -- internals -------------------------------------------------------------

    /** Re-reads the Flow's steering (no request: the device and the saved queue). */
    private fun publishFlow(styles: List<String>? = null) {
        val flow = runCatching { service.flowState() }.getOrNull() ?: _state.value.flow
        _state.update { state ->
            state.copy(
                flow = flow?.let { if (styles == null) it else it.copy(styles = styles) },
                chosen = styles ?: flow?.styles ?: state.chosen,
            )
        }
    }

    /**
     * The genre chips: the device's copy paints, then the session's ONE request.
     *
     * Guarded on a job, not only on the list: the flow hands its cached answer
     * and then its fresh one, and two launches would each ask `/flow/styles` —
     * the eighth request of a cold home, for nothing.
     */
    private fun loadStyles() {
        if (_state.value.styles.isNotEmpty() || stylesJob?.isActive == true) return
        stylesJob = scope.launch {
            val kept = runCatching { withContext(io) { service.cachedFlowStyles() } }.getOrDefault(emptyList())
            if (kept.isNotEmpty()) _state.update { it.copy(styles = kept) }
            runCatching { withContext(io) { service.flowStyles() } }
                .onSuccess { rows -> if (rows.isNotEmpty()) _state.update { it.copy(styles = rows) } }
        }
    }

    /** The notch's French name, from the hero's own ladder (never a number). */
    private fun energyName(state: State, notch: Double?): String {
        val steps = state.data?.hero?.energySteps.orEmpty()
        if (steps.isEmpty()) return if (notch == null) "Libre" else ""
        return steps.minByOrNull { step ->
            val value = step.value ?: return@minByOrNull if (notch == null) 0.0 else Double.MAX_VALUE
            if (notch == null) Double.MAX_VALUE else kotlin.math.abs(value - notch)
        }?.name ?: "Libre"
    }

    companion object {
        /** `flow.js#STYLES_MAX`: six genres is what two short rows hold. */
        const val STYLES_MAX = 6

        /** `discovery.js#DEFAULT_LEVEL`. */
        const val DEFAULT_LEVEL = 0.35

        const val CONTEXT_FLOW = "flow"
        const val CONTEXT_RELEASES = "releases"
        const val CONTEXT_RADIO = "radio"
        const val CONTEXT_MOOD = "mood"
    }
}
