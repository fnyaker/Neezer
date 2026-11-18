package fr.neezer.mobile.feature.audiofx

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import fr.neezer.mobile.designsystem.NzToasts
import uniffi.nz_ffi.FxMeterFrame
import uniffi.nz_ffi.FxPreset
import uniffi.nz_ffi.FxState

/**
 * The effects card's state machine: one record of what to draw, the person's
 * controls, and the meter's lifetime.
 *
 * **Every control is optimistic and answered by the store.** The card does not
 * wait for anything: `fn setBand` writes the value through the player (which
 * applies it to the engine's atomics at once, so the sound changes in the next
 * buffer) and the state the call RETURNS is the state drawn. A failure is the
 * device's own (the core is gone): the value is put back and the toast says why.
 *
 * **The meter belongs to the card.** [openMeter] on the card's first composition,
 * [closeMeter] when it leaves: the Rust side starts a 25 Hz thread for it and
 * stops AND JOINS it on close, so a closed card takes no reading at all
 * (`docs/agents/mobile.md`, MB12's acceptance). There is no timer in Kotlin.
 */
internal class AudioFxModel(private val backend: AudioFxBackend) {
    /** The card's state, or `null` until the first read lands. */
    var state: FxState? by mutableStateOf(null)
        private set

    /** The latest reading, or `null` while nothing has been measured. */
    var meter: FxMeterFrame? by mutableStateOf(null)
        private set

    /** The preset the person clicked, until the chain moves away from it. */
    var selected: String? by mutableStateOf(null)
        private set

    /** The naming field's text; `null` when the field is not open. */
    var draftName: String? by mutableStateOf(null)
        private set

    /** The cause of the last write the device refused, in French. */
    var failure: String? by mutableStateOf(null)
        private set

    /** A running write, for a control that must not be tapped twice. */
    var busy: Boolean by mutableStateOf(false)
        private set

    private var meterOpen = false

    /** The card appeared. Cheap and request-free: `kv` only. */
    fun start() {
        if (state != null) return
        runCatching { backend.state() }
            .onSuccess { state = it; failure = null }
            .onFailure { failure = it.message ?: it::class.java.simpleName }
    }

    /** The card is on screen: the meter may run. */
    fun openMeter() {
        if (meterOpen) return
        meterOpen = true
        // A reading from a previous visit is not this visit's: the card waits for
        // its own first frame instead of painting a stale bar.
        meter = null
        runCatching {
            backend.subscribeMeter { frame -> meter = frame }
        }.onFailure { failure = it.message }
    }

    /** The card left: the meter is stopped AND joined on the Rust side. */
    fun closeMeter() {
        if (!meterOpen) return
        meterOpen = false
        runCatching { backend.unsubscribeMeter() }
        meter = null
    }

    // -- the controls ---------------------------------------------------------

    fun setEqEnabled(on: Boolean) = write("eq") { backend.setEqEnabled(on) }

    fun setBand(index: Int, db: Float) = write("band:$index") { backend.setBand(index, db) }

    fun setBass(lift: Float) = write("bass") { backend.setBass(lift.coerceIn(0f, 1f)) }

    fun setNormalization(level: String) = write("norm") { backend.setNormalization(level) }

    fun setMono(mono: Boolean) = write("mono") { backend.setMono(mono) }

    fun setBalance(balance: Float) = write("balance") { backend.setBalance(balance.coerceIn(-1f, 1f)) }

    fun resetBalance() = write("balance") { backend.resetBalance() }

    fun setCrossfade(enabled: Boolean, seconds: Float) =
        write("fade") { backend.setCrossfade(enabled, seconds.coerceIn(0f, 12f)) }

    fun setSkipFade(enabled: Boolean, seconds: Float) =
        write("fade") { backend.setSkipFade(enabled, seconds.coerceIn(0f, 5f)) }

    fun setTrim(on: Boolean) = write("trim") { backend.setTrim(on) }

    fun setOutput(mode: String) = write("output") { backend.setOutput(mode) }

    /** The preset chip. A built-in replaces the curve; one of mine restores all. */
    fun usePreset(preset: FxPreset) {
        selected = preset.id
        write("preset") { backend.applyPreset(preset.id)?.let { it } ?: backend.state() }
    }

    /** The card's own selection, when the chain matches a preset. */
    fun selectedId(): String? = selected ?: state?.activePreset

    fun openNaming() {
        draftName = ""
    }

    fun cancelNaming() {
        draftName = null
    }

    fun editName(text: String) {
        draftName = text.take(40)
    }

    /** « Enregistrer »: the whole chain, under the name in the field. */
    fun confirmSave() {
        val name = draftName?.trim().orEmpty()
        if (name.isEmpty()) return
        val saved = runCatching { backend.savePreset(name) }.getOrNull() ?: return
        draftName = null
        selected = saved.id
        state = runCatching { backend.state() }.getOrNull() ?: state
        NzToasts.show(FxCopy.saved(saved.name))
    }

    fun updateSelected() {
        val id = selectedId() ?: return
        val updated = runCatching { backend.updatePreset(id) }.getOrNull() ?: return
        selected = updated.id
        state = runCatching { backend.state() }.getOrNull() ?: state
        NzToasts.show(FxCopy.updated(updated.name))
    }

    fun deleteSelected() {
        val id = selectedId() ?: return
        val preset = state?.presets?.firstOrNull { it.id == id } ?: return
        val removed = runCatching { backend.deletePreset(id) }.getOrDefault(false)
        if (!removed) return
        if (selected == id) selected = null
        state = runCatching { backend.state() }.getOrNull() ?: state
        NzToasts.show(FxCopy.deleted(preset.name))
    }

    /** « Réessayer maintenant » under a verdict. */
    fun retryNow() = write("output") {
        backend.retryNow()
        backend.state()
    }

    fun forgetGlitch(key: String) = write("output") {
        backend.forgetGlitch(key)
        backend.state()
    }

    /**
     * The warning's own button: the player's volume, pulled back by the measured
     * cut, with one toast to undo it. No engine write, no automatic change of the
     * sound — the SPA's own rule.
     */
    fun reduceVolume(cutDb: Double) {
        val before = runCatching { backend.volume() to backend.isMuted() }.getOrNull() ?: return
        val gain = Math.pow(10.0, -cutDb.coerceAtLeast(0.0) / 20.0).toFloat()
        val target = (before.first * gain).coerceIn(0f, 1f)
        if (target >= before.first - 0.0005f) return
        runCatching {
            backend.setVolume(target)
            backend.noteChange("volume")
        }.onFailure { failure = it.message }
        NzToasts.show(
            FxCopy.volumeReduced(FxFormat.db(cutDb)),
            FxCopy.undo(),
        ) {
            runCatching {
                backend.setVolume(before.first)
                if (before.second && !backend.isMuted()) backend.toggleMute()
            }
        }
    }

    /**
     * One write, answered by the store. `key` names the control for the warning's
     * new episode: a band, a preset, the switch, the level — each starts one
     * (`resetRiskHold`).
     */
    private fun write(key: String, block: () -> FxState) {
        val before = state
        busy = true
        runCatching {
            val next = block()
            state = next
            failure = null
            backend.noteChange(key)
        }.onFailure { error ->
            state = before
            failure = error.message ?: error::class.java.simpleName
            NzToasts.show(failure ?: "Réglage impossible")
        }
        busy = false
    }
}
