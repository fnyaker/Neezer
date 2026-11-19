package fr.neezer.mobile.feature.viz

import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.nz_ffi.PlayState
import uniffi.nz_ffi.VizSettingsData

/**
 * The visualiser's screen state: the settings, the track on the card, the
 * sleeve, and the two actions a person takes here.
 *
 * # Clicks are optimistic, and a refusal is a toast
 *
 * Every control writes its value into the state FIRST and calls Rust second
 * (`stores.js#persisted`'s own optimism: the value is the person's, the store is
 * local). Rust validates it against the store's whitelist; a refusal rolls the
 * state back to what it was and shows the cause with « Réessayer » in the toast
 * — never a silent no-op (rule 3).
 *
 * # One cover decode per track, and never a second one
 *
 * `$current` is handed over again on every progress tick in the SPA, and asking
 * for the artwork each time measured thirteen cover requests in three seconds of
 * the full-screen player (the SPA's own comment). Here the key is the track's
 * cover ID: the same sleeve is decoded ONCE and handed to Rust once, and a track
 * with no artwork CLEARS it so the row goes back to its own low → high walk
 * instead of keeping the last track's colours.
 *
 * # The card and the pause
 *
 * The player is read through [VizPlayer], never polled per frame: the visualiser
 * draws the sound, not the clock. `following` re-reads the snapshot when the
 * screen becomes visible and whenever the caller asks (the shell's change flow);
 * `playing` is what turns the notice on.
 */
class VizModel(
    private val service: VizService,
    private val player: VizPlayer,
    private val covers: VizCovers?,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val toast: (String) -> Unit = {},
) {
    data class State(
        /** The resolved settings, as Rust answered them (never a parallel copy). */
        val settings: VizSettingsData? = null,
        /** The track on the card. */
        val title: String = "",
        val artists: String = "",
        /** Whether the player is playing: the scene stops on pause. */
        val playing: Boolean = false,
        /** Something is loaded at all (`routes/Viz.svelte#loaded`). */
        val loaded: Boolean = false,
        /** The sleeve's cover id last handed to Rust (to ask only once). */
        val cover: String? = null,
        /** A setting write that failed, in French, with its key for « Réessayer ». */
        val failure: Failure? = null,
    ) {
        /** The mode actually drawn: eco means none. */
        val mode: String get() = settings?.mode ?: "bars"
        val eco: Boolean get() = settings?.eco == true
        val fps: Int get() = settings?.fps?.toInt() ?: 60
        val palette: String get() = settings?.palette ?: "cover"
        val intensity: Float get() = (settings?.intensity ?: 0.7).toFloat()
        val orientation: String get() = settings?.scopeOrientation ?: "horizontal"
        val scopeColour: String get() = settings?.scopeColour ?: "duo"
        val draws: Boolean get() = mode != "off"
    }

    /** A write that failed: the key, the sentence, and what to go back to. */
    data class Failure(val key: String, val cause: String, val retry: () -> Unit)

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    /** What the surface asks for its frame — the model owns the ONE service. */
    fun frame() = service.frame()

    fun setVisible(view: String, visible: Boolean) = service.setVisible(view, visible)

    fun setShape(bars: Int, cols: Int) = service.setShape(bars, cols)

    /** Reads the settings and the player: the screen's first paint, and a return. */
    fun refresh() {
        val settings = runCatching { service.settings() }.getOrNull()
        _state.value = _state.value.copy(settings = settings ?: _state.value.settings)
        follow()
    }

    /** Re-reads the player (a track change, the screen becoming visible). */
    fun follow() {
        val snapshot = player.snapshot() ?: return
        val current = snapshot.current
        val playing = snapshot.state == PlayState.PLAYING || snapshot.state == PlayState.BUFFERING
        val cover = current?.cover
        val previous = _state.value
        _state.value = previous.copy(
            title = current?.title ?: "",
            artists = current?.artists ?: "",
            playing = playing,
            loaded = current != null,
            cover = cover,
        )
        if (cover != previous.cover || current == null) {
            loadArtwork(cover)
        }
    }

    /**
     * The sleeve, once per track: the cache's file, decoded at 64 px, handed to
     * Rust. Off the main thread (`BitmapFactory` reads a file), and a failure is
     * not a screen failure: the row keeps its own palette.
     */
    private fun loadArtwork(cover: String?) {
        val source = covers
        if (source == null || cover.isNullOrBlank()) {
            service.clearArtwork()
            return
        }
        scope.launch {
            val path = runCatching { source.path(cover) }.getOrNull()
            val pixels = withContext(io) { VizArtwork.decode(path) }
            if (pixels == null) {
                service.clearArtwork()
            } else {
                service.setArtwork(pixels.rgba, pixels.width, pixels.height)
            }
        }
    }

    // --- the taps ------------------------------------------------------------

    fun setMode(mode: String) = write("viz.mode", { it.copy(mode = mode, chosenMode = mode) }) {
        runCatching { service.setMode(mode) }.getOrNull()
    }

    fun setPalette(palette: String) = write("viz.palette", { it.copy(palette = palette) }) {
        runCatching { service.setPalette(palette) }.getOrNull()
    }

    fun setIntensity(value: Float) = write("viz.intensity", { it.copy(intensity = value.toDouble()) }) {
        runCatching { service.setIntensity(value.toDouble().coerceIn(0.0, 1.0)) }.getOrNull()
    }

    fun setFps(fps: Int) = write("viz.fps", { it.copy(fps = fps.toUInt()) }) {
        runCatching { service.setFps(fps) }.getOrNull()
    }

    fun setEco(eco: Boolean) = write("viz.eco", {
        // Eco means NONE: the optimistic state is the same resolution Rust's
        // `settle` applies, so the canvas goes away on this very frame.
        it.copy(eco = eco, mode = if (eco) "off" else it.chosenMode)
    }) {
        runCatching { service.setEco(eco) }.getOrNull()
    }

    fun setScopeOrientation(orientation: String) =
        write("viz.scope.orientation", { it.copy(scopeOrientation = orientation) }) {
            runCatching { service.setScopeOrientation(orientation) }.getOrNull()
        }

    fun setScopeColour(colour: String) = write("viz.scope.colour", { it.copy(scopeColour = colour) }) {
        runCatching { service.setScopeColour(colour) }.getOrNull()
    }

    /** Eco, from the leaf button: the same write, and the SPA's two toasts. */
    fun toggleEco() {
        val on = !_state.value.eco
        setEco(on)
        toast(if (on) VizCopy.ECO_ON else VizCopy.ECO_OFF)
    }

    fun dismissFailure() {
        _state.value = _state.value.copy(failure = null)
    }

    /**
     * One optimistic write: the state first (the surface paints the new choice on
     * this frame), then Rust. `apply` builds the state the tap implies; the call
     * answers the resolved settings or `null` on failure.
     */
    private fun write(
        key: String,
        apply: (VizSettingsData) -> VizSettingsData,
        call: () -> VizSettingsData?,
    ) {
        val previous = _state.value.settings ?: service.settings()
        val optimistic = apply(previous)
        _state.value = _state.value.copy(settings = optimistic, failure = null)
        val resolved = call()
        if (resolved == null) {
            // Rolled back to the value the store really holds, and the person is
            // told why with the way back (rule 3).
            _state.value = _state.value.copy(
                settings = previous,
                failure = Failure(key, VizCopy.SETTING_FAILED) { write(key, apply, call) },
            )
            toast(VizCopy.SETTING_FAILED)
        } else {
            _state.value = _state.value.copy(settings = resolved)
        }
    }
}
